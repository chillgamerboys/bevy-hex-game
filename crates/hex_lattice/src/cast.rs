//! Casting: the single legality function, its plan, and the applier.

use std::collections::{BTreeMap, BTreeSet};

use hex_core::{LatticeCoord, SpellId};

use crate::spec::{CellKind, LatticeSpec};
use crate::state::{ActiveEnchantment, LatticeState};
use crate::tables::{Casting, Requirement, Tables};

/// The exact plan for a cast: which gems supply how much mana.
///
/// Produced by [`castable`] and consumed by [`apply_cast`], so preview,
/// application, and AI forward-simulation all agree on the mana to the point. The
/// `drains` map records the leaf gems (a fusion resolves to *its* feeder gems), so
/// its values sum to the cast's total mana cost — the basis of conservation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CastPlan {
    /// The spell being cast.
    pub spell: SpellId,
    /// The spell cell it is cast from.
    pub cell: LatticeCoord,
    /// Leaf gem drains: coordinate to mana removed.
    pub drains: BTreeMap<LatticeCoord, u16>,
}

/// Why a cast is illegal — the vocabulary for saying no out loud.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CastBlocked {
    /// The chosen cell does not hold a spell.
    NotASpell,
    /// The spell's own cell is disabled.
    SpellDisabled,
    /// The spell's adjacent element and mana requirements cannot all be met — a
    /// missing element, a disabled feeder, or too little mana. Casting is binary,
    /// so there is no degraded cast.
    Unsatisfiable,
}

/// Decides whether the spell at `cell` can be cast, and if so exactly how.
///
/// Binary: it returns a complete [`CastPlan`] or a [`CastBlocked`] reason, never a
/// partial cast. A requirement is met by a distinct adjacent live gem of the right
/// element with enough mana (and not already committed to an enchantment), or by an
/// adjacent live *fusion* whose output matches
/// and whose own recipe resolves the same way — recursively, and cycle-safe. The
/// assignment is deterministic: candidates are tried in [`LatticeCoord`] order and
/// the first complete one wins.
pub fn castable(
    spec: &LatticeSpec,
    state: &LatticeState,
    cell: LatticeCoord,
    tables: &impl Tables,
) -> Result<CastPlan, CastBlocked> {
    let spell = match spec.get(cell) {
        Some(CellKind::Spell { spell }) => spell,
        _ => return Err(CastBlocked::NotASpell),
    };
    if state.is_disabled(cell) {
        return Err(CastBlocked::SpellDisabled);
    }

    let requirements = tables.requirements(spell);
    let mut used = BTreeSet::new();
    let mut drains = BTreeMap::new();
    if satisfy(
        &requirements,
        cell,
        spec,
        state,
        tables,
        &mut used,
        &mut drains,
    ) {
        Ok(CastPlan {
            spell,
            cell,
            drains,
        })
    } else {
        Err(CastBlocked::Unsatisfiable)
    }
}

/// Applies a cast's plan to the lattice: drains the leaf gems, and for an
/// enchantment ties the drawn mana up and locks its funding gems.
///
/// The plan must have come from [`castable`] on this same state; it is not
/// re-validated. Evocations simply consume; enchantments record their locked mana
/// and mark each funding gem so that disabling one later breaks the enchantment.
pub fn apply_cast(state: &mut LatticeState, plan: &CastPlan, tables: &impl Tables) {
    for (&coord, &amount) in &plan.drains {
        state.drain(coord, amount);
    }
    if let Casting::Enchantment { defense } = tables.casting(plan.spell) {
        let locked = plan
            .drains
            .values()
            .copied()
            .fold(0u16, u16::saturating_add);
        let id = state.allocate_enchant();
        state.insert_enchantment(
            id,
            ActiveEnchantment {
                spell: plan.spell,
                cell: plan.cell,
                locked_mana: locked,
                defense,
            },
        );
        for &coord in plan.drains.keys() {
            state.lock(coord, id);
        }
    }
}

/// Tries to satisfy `reqs` from the neighbours of `around`, extending the `used`
/// cell set and the `drains` map. Returns whether a complete assignment was found.
///
/// Each requirement claims a *distinct* adjacent cell — a spell's tier is a count
/// of adjacent gems, not an amount of mana from fewer of them. A fusion source
/// claims its own cell and then resolves its recipe from *its* neighbours; because
/// a claimed cell cannot be reused, `used` doubles as the cycle guard for fusion
/// chains.
fn satisfy(
    reqs: &[Requirement],
    around: LatticeCoord,
    spec: &LatticeSpec,
    state: &LatticeState,
    tables: &impl Tables,
    used: &mut BTreeSet<LatticeCoord>,
    drains: &mut BTreeMap<LatticeCoord, u16>,
) -> bool {
    let Some((req, rest)) = reqs.split_first() else {
        return true;
    };

    // A locked gem is spoken for by the enchantment it already hosts — its capacity
    // is committed, so it can fund no further cast. Excluding it keeps `locks`
    // one-enchantment-per-gem: without this, a second cast drawing on the same gem
    // would overwrite its lock and orphan the first enchantment, which could then
    // never break (its mana stranded, the disable→break invariant violated).
    let candidates: Vec<(LatticeCoord, CellKind)> = spec
        .present_neighbors(around)
        .into_iter()
        .filter(|(coord, _)| {
            !state.is_disabled(*coord) && !used.contains(coord) && !state.is_locked(*coord)
        })
        .collect();

    for (coord, kind) in candidates {
        match kind {
            CellKind::Gem { element }
                if element == req.element && state.mana(coord) >= req.mana =>
            {
                // `used` filters this coord out of any later candidate list, so it
                // can appear in `drains` at most once — a plain insert/remove pair.
                used.insert(coord);
                drains.insert(coord, req.mana);
                if satisfy(rest, around, spec, state, tables, used, drains) {
                    return true;
                }
                used.remove(&coord);
                drains.remove(&coord);
            }
            CellKind::Fusion { output } if output == req.element => {
                let Some(recipe) = tables.recipe(output) else {
                    continue;
                };
                let used_snapshot = used.clone();
                let drains_snapshot = drains.clone();
                used.insert(coord);
                if satisfy(&recipe, coord, spec, state, tables, used, drains)
                    && satisfy(rest, around, spec, state, tables, used, drains)
                {
                    return true;
                }
                *used = used_snapshot;
                *drains = drains_snapshot;
            }
            _ => {}
        }
    }

    false
}
