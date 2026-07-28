//! The battle-mutable half of a lattice, and the per-owner mana stats.

use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::prelude::*;
use bevy_reflect::prelude::*;
use hex_core::{ElementId, EnchantId, LatticeCoord, SpellId};
use serde::{Deserialize, Serialize};

use crate::spec::{CellKind, LatticeSpec};

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
