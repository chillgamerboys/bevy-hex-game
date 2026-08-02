//! The battle-mutable half of a lattice, and the per-owner mana stats.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use bevy_ecs::prelude::*;
use bevy_reflect::prelude::*;
use hex_core::{ElementId, EnchantId, LatticeCoord, SpellId};
use serde::{Deserialize, Serialize};

use crate::cast::castable;
use crate::spec::{CellKind, LatticeSpec};
use crate::tables::{Casting, Tables};

/// Why deserialized battle state cannot belong to its immutable lattice inscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LatticeStateError {
    /// The state does not contain exactly one mana entry for every gem cell.
    ManaShape,
    /// A gem carries more mana than the archetype permits.
    ManaCapacity,
    /// A disabled coordinate is not present in the inscription.
    DisabledCell,
    /// A lock is not on a live gem or names no active enchantment.
    InvalidLock,
    /// An active enchantment has no funding-gem lock.
    OrphanEnchantment,
    /// The saved spell cell or spell identity differs from the inscription.
    EnchantmentCell,
    /// The saved enchantment metadata differs from current spell rules.
    EnchantmentDefinition,
    /// The locked gems cannot reproduce a legal cast of the saved enchantment.
    EnchantmentFunding,
    /// Locked mana is zero or differs from the funding cast's exact cost.
    EnchantmentMana,
    /// An active enchantment id is not below the next monotonic id.
    EnchantmentSequence,
}

impl fmt::Display for LatticeStateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ManaShape => "gem coordinates do not match the lattice inscription",
            Self::ManaCapacity => "gem mana exceeds the archetype capacity",
            Self::DisabledCell => "a disabled coordinate is outside the lattice inscription",
            Self::InvalidLock => "an enchantment lock is internally inconsistent",
            Self::OrphanEnchantment => "an active enchantment has no funding gem",
            Self::EnchantmentCell => "an active enchantment does not match its spell cell",
            Self::EnchantmentDefinition => {
                "an active enchantment does not match the current spell definition"
            }
            Self::EnchantmentFunding => {
                "an active enchantment's funding gems cannot legally cast its spell"
            }
            Self::EnchantmentMana => "an active enchantment has impossible locked mana",
            Self::EnchantmentSequence => "an active enchantment id is outside the saved sequence",
        })
    }
}

/// A lattice owner's per-element mana rules: its Attunement and Channelling.
///
/// Both are keyed by element. `capacity` is how much mana a single gem of that
/// element can hold (Attunement); `channelling` is how much a channel action
/// restores across that element's gems (Channelling). Unattuned elements resolve
/// to zero.
///
/// A `Component` alongside [`LatticeSpec`] and [`LatticeState`], because all three are
/// per-unit and a unit that carries a lattice carries its own mana rules — an enemy's
/// lattice is its entire stat block, and Attunement is part of that block rather than a
/// global constant.
///
/// # Save compatibility
///
/// Serde matches [`LatticeState`], but the map is **keyed by [`ElementId`], which is
/// session-local** — ids are dealt from sorted element names at load. Sorting means a
/// reorder is harmless and an *insertion* is not: shipping a content patch that adds an
/// element shifts every id after it, and a save written before the patch would read one
/// element's attunement as its neighbour's. Nothing persists this yet. Whoever lands
/// saves resolves it the way the command log already did — by storing stable names and
/// re-resolving on load — rather than by trusting these ids across versions.
#[derive(Component, Reflect, Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct LatticeStats {
    capacity: BTreeMap<ElementId, u16>,
    channelling: BTreeMap<ElementId, u16>,
}

impl LatticeStats {
    /// Builds stats from per-element capacity and channelling maps.
    #[must_use]
    pub fn new(capacity: BTreeMap<ElementId, u16>, channelling: BTreeMap<ElementId, u16>) -> Self {
        Self {
            capacity,
            channelling,
        }
    }

    /// The mana capacity of a single gem of `element` (zero if unattuned).
    #[must_use]
    pub fn capacity(&self, element: ElementId) -> u16 {
        self.capacity.get(&element).copied().unwrap_or(0)
    }

    /// The mana a channel action restores to `element`'s gems (zero if unattuned).
    #[must_use]
    pub fn channelling(&self, element: ElementId) -> u16 {
        self.channelling.get(&element).copied().unwrap_or(0)
    }
}

/// An enchantment currently held in the lattice.
///
/// Its `locked_mana` was drawn from the funding gems when it was cast and is held
/// here until the enchantment ends. If it *breaks* (a funding gem is disabled) the
/// locked mana is consumed — lost, not returned.
#[derive(Reflect, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ActiveEnchantment {
    /// The spell that created this enchantment.
    pub spell: SpellId,
    /// The spell cell it was cast from.
    pub cell: LatticeCoord,
    /// The mana tied up in this enchantment.
    pub locked_mana: u16,
    /// The flat reduction it applies to incoming disable counts while active.
    pub defense: u16,
}

/// The record of an enchantment that broke because a funding gem was disabled.
///
/// Returned by [`apply_disables`](crate::apply_disables) so the caller can log the
/// loss and present it; the mana is already gone from the lattice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrokenEnchantment {
    /// The enchantment that broke.
    pub enchant: EnchantId,
    /// The spell that had created it.
    pub spell: SpellId,
    /// The locked mana consumed by the break.
    pub burned_mana: u16,
    /// The disabled gem that triggered the break.
    pub trigger: LatticeCoord,
}

/// The battle-mutable half of a lattice: mana, disabled cells, and enchantment locks.
///
/// **Only those.** Lasting *effects* on a unit — burn above all — belong to
/// `hex_combat`'s ledger, not here: they carry a source and a tick point, and a rules
/// engine with no turn order and no notion of who cast what can represent neither.
///
/// Small and integer-valued by construction, so cloning it is cheap — that clone
/// is the AI's forward-simulation primitive. Every collection is ordered, so
/// iteration and serialization are deterministic.
#[derive(Component, Reflect, Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct LatticeState {
    mana: BTreeMap<LatticeCoord, u16>,
    disabled: BTreeSet<LatticeCoord>,
    locks: BTreeMap<LatticeCoord, EnchantId>,
    enchantments: BTreeMap<EnchantId, ActiveEnchantment>,
    next_enchant: u32,
}

impl LatticeState {
    /// The opening state for `spec`: every gem full to its element's attunement
    /// capacity, nothing disabled and no enchantments.
    #[must_use]
    pub fn new(spec: &LatticeSpec, stats: &LatticeStats) -> Self {
        let mut mana = BTreeMap::new();
        for (coord, kind) in spec.cells() {
            if let CellKind::Gem { element } = kind {
                mana.insert(coord, stats.capacity(element));
            }
        }
        Self {
            mana,
            ..Self::default()
        }
    }

    // --- reads -------------------------------------------------------------

    /// The mana held by the gem at `coord` (zero if there is no gem there).
    #[must_use]
    pub fn mana(&self, coord: LatticeCoord) -> u16 {
        self.mana.get(&coord).copied().unwrap_or(0)
    }

    /// Every gem coordinate retained by this battle state, in canonical order.
    ///
    /// The coordinates remain present when a gem is empty or disabled, so persistence
    /// adapters can identify the immutable lattice inscription without treating the
    /// current mana total as character identity.
    pub fn mana_cells(&self) -> impl Iterator<Item = (LatticeCoord, u16)> + '_ {
        self.mana.iter().map(|(&coord, &mana)| (coord, mana))
    }

    /// Whether the cell at `coord` is disabled.
    #[must_use]
    pub fn is_disabled(&self, coord: LatticeCoord) -> bool {
        self.disabled.contains(&coord)
    }

    /// Whether the gem at `coord` is currently funding an enchantment.
    #[must_use]
    pub fn is_locked(&self, coord: LatticeCoord) -> bool {
        self.locks.contains_key(&coord)
    }

    /// Total mana currently held across all gems.
    #[must_use]
    pub fn total_gem_mana(&self) -> u32 {
        self.mana.values().map(|&mana| u32::from(mana)).sum()
    }

    /// Total mana tied up across all active enchantments.
    #[must_use]
    pub fn total_locked_mana(&self) -> u32 {
        self.enchantments
            .values()
            .map(|enchantment| u32::from(enchantment.locked_mana))
            .sum()
    }

    /// The active enchantment with this id, if any.
    #[must_use]
    pub fn enchantment(&self, id: EnchantId) -> Option<&ActiveEnchantment> {
        self.enchantments.get(&id)
    }

    /// Every active enchantment, in id order.
    pub fn active_enchantments(
        &self,
    ) -> impl Iterator<Item = (EnchantId, &ActiveEnchantment)> + '_ {
        self.enchantments.iter().map(|(&id, e)| (id, e))
    }

    /// How many enchantments are active.
    #[must_use]
    pub fn enchantment_count(&self) -> usize {
        self.enchantments.len()
    }

    /// Validates deserialized mutable state against its immutable archetype and rules.
    ///
    /// This is intentionally owned by the rules crate: persistence adapters may carry
    /// bytes, but they must not reconstruct lattice invariants from private fields.
    pub fn validate_against(
        &self,
        spec: &LatticeSpec,
        stats: &LatticeStats,
        tables: &impl Tables,
    ) -> Result<(), LatticeStateError> {
        let gems = spec
            .cells()
            .filter_map(|(coord, kind)| match kind {
                CellKind::Gem { element } => Some((coord, element)),
                _ => None,
            })
            .collect::<BTreeMap<_, _>>();
        if self.mana.len() != gems.len() || self.mana.keys().any(|coord| !gems.contains_key(coord))
        {
            return Err(LatticeStateError::ManaShape);
        }
        for (&coord, &mana) in &self.mana {
            let Some(element) = gems.get(&coord).copied() else {
                return Err(LatticeStateError::ManaShape);
            };
            if mana > stats.capacity(element) {
                return Err(LatticeStateError::ManaCapacity);
            }
        }
        if self.disabled.iter().any(|coord| spec.get(*coord).is_none()) {
            return Err(LatticeStateError::DisabledCell);
        }
        for (&coord, &enchantment) in &self.locks {
            if !matches!(spec.get(coord), Some(CellKind::Gem { .. }))
                || self.disabled.contains(&coord)
                || !self.enchantments.contains_key(&enchantment)
            {
                return Err(LatticeStateError::InvalidLock);
            }
        }
        for (&id, enchantment) in &self.enchantments {
            let funding = self
                .locks
                .iter()
                .filter_map(|(&coord, &locked)| (locked == id).then_some(coord))
                .collect::<Vec<_>>();
            if funding.is_empty() {
                return Err(LatticeStateError::OrphanEnchantment);
            }
            if !matches!(
                spec.get(enchantment.cell),
                Some(CellKind::Spell { spell }) if spell == enchantment.spell
            ) {
                return Err(LatticeStateError::EnchantmentCell);
            }
            if !matches!(
                tables.casting(enchantment.spell),
                Casting::Enchantment { defense } if defense == enchantment.defense
            ) {
                return Err(LatticeStateError::EnchantmentDefinition);
            }
            if enchantment.locked_mana == 0 {
                return Err(LatticeStateError::EnchantmentMana);
            }
            let funding = funding.into_iter().collect::<BTreeSet<_>>();

            // Recreate only the facts the cast could have consumed. A locked gem cannot
            // be drained or refilled later, so its deficit from capacity is an upper
            // bound on the mana this enchantment drew. Disabling every other gem forces
            // the canonical resolver to prove that these exact leaves satisfy the
            // spell's adjacency, element, fusion, and per-requirement cost topology.
            // Spell and fusion disables are intentionally absent: either can be disabled
            // after a persistent enchantment was cast without breaking its gem locks.
            let mut cast_state = Self::new(spec, stats);
            for (&coord, &element) in &gems {
                if funding.contains(&coord) {
                    cast_state.set_mana(
                        coord,
                        stats.capacity(element).saturating_sub(self.mana(coord)),
                    );
                } else {
                    cast_state.disable(coord);
                }
            }
            let exact_funding =
                |plan: &crate::CastPlan| plan.drains.keys().copied().eq(funding.iter().copied());
            let plan = match castable(spec, &cast_state, enchantment.cell, tables) {
                Ok(plan) if exact_funding(&plan) => plan,
                _ => {
                    // A second pass at full capacity distinguishes a structurally valid
                    // funding topology with insufficient saved deficit from locks that
                    // could never have formed this enchantment at all.
                    for &coord in &funding {
                        let Some(CellKind::Gem { element }) = spec.get(coord) else {
                            return Err(LatticeStateError::InvalidLock);
                        };
                        cast_state.set_mana(coord, stats.capacity(element));
                    }
                    if matches!(
                        castable(spec, &cast_state, enchantment.cell, tables),
                        Ok(plan) if exact_funding(&plan)
                    ) {
                        return Err(LatticeStateError::EnchantmentMana);
                    }
                    return Err(LatticeStateError::EnchantmentFunding);
                }
            };
            let planned_mana = plan
                .drains
                .values()
                .copied()
                .fold(0_u16, u16::saturating_add);
            if enchantment.locked_mana != planned_mana {
                return Err(LatticeStateError::EnchantmentMana);
            }
            if id.0 >= self.next_enchant && !(id.0 == u32::MAX && self.next_enchant == u32::MAX) {
                return Err(LatticeStateError::EnchantmentSequence);
            }
        }
        Ok(())
    }

    // --- writes, for the operation modules ---------------------------------

    /// Removes up to `amount` mana from the gem at `coord`.
    pub(crate) fn drain(&mut self, coord: LatticeCoord, amount: u16) {
        if let Some(mana) = self.mana.get_mut(&coord) {
            *mana = mana.saturating_sub(amount);
        }
    }

    /// Sets the mana of the gem at `coord` (used by channelling).
    pub(crate) fn set_mana(&mut self, coord: LatticeCoord, value: u16) {
        if let Some(mana) = self.mana.get_mut(&coord) {
            *mana = value;
        }
    }

    /// Allocates the next enchantment id from the monotonic counter.
    pub(crate) fn allocate_enchant(&mut self) -> EnchantId {
        let id = EnchantId(self.next_enchant);
        self.next_enchant = self.next_enchant.saturating_add(1);
        id
    }

    /// Registers an active enchantment under `id`.
    pub(crate) fn insert_enchantment(&mut self, id: EnchantId, enchantment: ActiveEnchantment) {
        self.enchantments.insert(id, enchantment);
    }

    /// Marks the gem at `coord` as funding the enchantment `id`.
    pub(crate) fn lock(&mut self, coord: LatticeCoord, id: EnchantId) {
        self.locks.insert(coord, id);
    }

    /// The enchantment the gem at `coord` funds, if any.
    pub(crate) fn locked_by(&self, coord: LatticeCoord) -> Option<EnchantId> {
        self.locks.get(&coord).copied()
    }

    /// Disables the cell at `coord`, returning whether it was newly disabled.
    pub(crate) fn disable(&mut self, coord: LatticeCoord) -> bool {
        self.disabled.insert(coord)
    }

    /// Re-enables the cell at `coord`, returning whether it had been disabled.
    ///
    /// **Mana is untouched.** Disabling never spent a gem's mana — `disabled` and
    /// `mana` are separate stores — so restoring cannot hand any back. A recovered gem
    /// holds exactly what it held when it went down, and refilling it is
    /// [`channel`](crate::channel)'s job.
    pub(crate) fn restore(&mut self, coord: LatticeCoord) -> bool {
        self.disabled.remove(&coord)
    }

    /// Removes the enchantment `id` and all of its gem locks, returning it.
    pub(crate) fn break_enchant(&mut self, id: EnchantId) -> Option<ActiveEnchantment> {
        let enchantment = self.enchantments.remove(&id)?;
        self.locks.retain(|_, &mut locked| locked != id);
        Some(enchantment)
    }
}

#[cfg(test)]
mod tests {
    use hex_core::{ElementId, LatticeCoord, SpellId};

    use super::*;
    use crate::tables::{FusionTable, Requirement, SpellTable};

    struct TestSpells;

    const FIRE: ElementId = ElementId(0);
    const WATER: ElementId = ElementId(1);
    const STEAM: ElementId = ElementId(2);
    const DIRECT_SPELL: SpellId = SpellId(0);
    const FUSION_SPELL: SpellId = SpellId(1);

    impl FusionTable for TestSpells {
        fn recipe(&self, output: ElementId) -> Option<Vec<Requirement>> {
            (output == STEAM).then(|| {
                vec![
                    Requirement {
                        element: FIRE,
                        mana: 1,
                    },
                    Requirement {
                        element: WATER,
                        mana: 1,
                    },
                ]
            })
        }
    }

    impl SpellTable for TestSpells {
        fn requirements(&self, spell: SpellId) -> Vec<Requirement> {
            vec![Requirement {
                element: if spell == FUSION_SPELL { STEAM } else { FIRE },
                mana: 1,
            }]
        }

        fn casting(&self, _spell: SpellId) -> Casting {
            Casting::Enchantment { defense: 1 }
        }
    }

    fn fixture() -> (LatticeSpec, LatticeStats) {
        let gem = LatticeCoord::ORIGIN;
        let spell = LatticeCoord::new(1, 0);
        (
            LatticeSpec::new(BTreeMap::from([
                (gem, CellKind::Gem { element: FIRE }),
                (
                    spell,
                    CellKind::Spell {
                        spell: DIRECT_SPELL,
                    },
                ),
            ])),
            LatticeStats::new(BTreeMap::from([(FIRE, 3)]), BTreeMap::new()),
        )
    }

    #[test]
    fn fresh_state_validates_against_its_inscription() {
        let (spec, stats) = fixture();
        let state = LatticeState::new(&spec, &stats);
        assert_eq!(state.validate_against(&spec, &stats, &TestSpells), Ok(()));
    }

    #[test]
    fn state_from_another_inscription_is_rejected() {
        let (spec, stats) = fixture();
        let other_spec = LatticeSpec::new(BTreeMap::from([(
            LatticeCoord::new(-1, 0),
            CellKind::Gem {
                element: ElementId(0),
            },
        )]));
        let state = LatticeState::new(&other_spec, &stats);
        assert_eq!(
            state.validate_against(&spec, &stats, &TestSpells),
            Err(LatticeStateError::ManaShape)
        );
    }

    #[test]
    fn an_enchantment_cannot_claim_mana_that_its_funding_gem_never_spent() {
        let (spec, stats) = fixture();
        let mut state = LatticeState::new(&spec, &stats);
        let id = state.allocate_enchant();
        state.insert_enchantment(
            id,
            ActiveEnchantment {
                spell: DIRECT_SPELL,
                cell: LatticeCoord::new(1, 0),
                locked_mana: 1,
                defense: 1,
            },
        );
        state.lock(LatticeCoord::ORIGIN, id);
        assert_eq!(
            state.validate_against(&spec, &stats, &TestSpells),
            Err(LatticeStateError::EnchantmentMana)
        );

        state.drain(LatticeCoord::ORIGIN, 1);
        assert_eq!(state.validate_against(&spec, &stats, &TestSpells), Ok(()));
    }

    #[test]
    fn an_enchantment_cannot_be_funded_by_a_wrong_element_gem() {
        let spell = LatticeCoord::ORIGIN;
        let fire = LatticeCoord::new(1, 0);
        let water = LatticeCoord::new(-1, 0);
        let spec = LatticeSpec::new(BTreeMap::from([
            (
                spell,
                CellKind::Spell {
                    spell: DIRECT_SPELL,
                },
            ),
            (fire, CellKind::Gem { element: FIRE }),
            (water, CellKind::Gem { element: WATER }),
        ]));
        let stats = LatticeStats::new(BTreeMap::from([(FIRE, 3), (WATER, 3)]), BTreeMap::new());
        let mut state = LatticeState::new(&spec, &stats);
        let id = state.allocate_enchant();
        state.insert_enchantment(
            id,
            ActiveEnchantment {
                spell: DIRECT_SPELL,
                cell: spell,
                locked_mana: 1,
                defense: 1,
            },
        );
        state.lock(water, id);
        state.drain(water, 1);

        assert_eq!(
            state.validate_against(&spec, &stats, &TestSpells),
            Err(LatticeStateError::EnchantmentFunding)
        );
    }

    #[test]
    fn an_enchantment_cannot_be_funded_by_a_remote_gem() {
        let spell = LatticeCoord::ORIGIN;
        let bridge = LatticeCoord::new(1, 0);
        let remote = LatticeCoord::new(2, 0);
        let spec = LatticeSpec::new(BTreeMap::from([
            (
                spell,
                CellKind::Spell {
                    spell: DIRECT_SPELL,
                },
            ),
            (bridge, CellKind::Blank),
            (remote, CellKind::Gem { element: FIRE }),
        ]));
        let stats = LatticeStats::new(BTreeMap::from([(FIRE, 3)]), BTreeMap::new());
        let mut state = LatticeState::new(&spec, &stats);
        let id = state.allocate_enchant();
        state.insert_enchantment(
            id,
            ActiveEnchantment {
                spell: DIRECT_SPELL,
                cell: spell,
                locked_mana: 1,
                defense: 1,
            },
        );
        state.lock(remote, id);
        state.drain(remote, 1);

        assert_eq!(
            state.validate_against(&spec, &stats, &TestSpells),
            Err(LatticeStateError::EnchantmentFunding)
        );
    }

    #[test]
    fn a_fusion_casts_from_its_exact_locked_leaf_gems() {
        let spell = LatticeCoord::ORIGIN;
        let fusion = LatticeCoord::new(1, 0);
        let fire = LatticeCoord::new(2, 0);
        let water = LatticeCoord::new(1, -1);
        let spec = LatticeSpec::new(BTreeMap::from([
            (
                spell,
                CellKind::Spell {
                    spell: FUSION_SPELL,
                },
            ),
            (fusion, CellKind::Fusion { output: STEAM }),
            (fire, CellKind::Gem { element: FIRE }),
            (water, CellKind::Gem { element: WATER }),
        ]));
        let stats = LatticeStats::new(BTreeMap::from([(FIRE, 3), (WATER, 3)]), BTreeMap::new());
        let mut state = LatticeState::new(&spec, &stats);
        let plan = castable(&spec, &state, spell, &TestSpells)
            .expect("the intact fusion should fund the enchantment");
        assert!(crate::apply_cast(&mut state, &plan, &TestSpells));

        assert_eq!(state.validate_against(&spec, &stats, &TestSpells), Ok(()));
    }
}
