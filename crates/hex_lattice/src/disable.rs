//! Damage bookkeeping: net incoming disables, applying chosen disables, and burns.
//!
//! This module owns the *bookkeeping* half of damage. Who chooses which hexes are
//! disabled — the defender-chooses suspension — belongs to the command funnel that
//! consumes this crate, not here. These functions take an already-chosen cell list
//! and apply its consequences deterministically.

use hex_core::LatticeCoord;

use crate::state::{BrokenEnchantment, LatticeState};

/// The net number of hexes a raw incoming disable count actually disables, after
/// the defender's active defensive enchantments subtract their flat reductions.
///
/// Flat subtraction gives threshold behaviour for free: a metal shield reducing by
/// one turns a fireball's 3 into a 2 and an ember's 1 into nothing. Burn ignores
/// armour, so burn-driven disables bypass this and are applied directly.
#[must_use]
pub fn resolve_incoming(state: &LatticeState, raw: u16) -> u16 {
    let defense = state
        .active_enchantments()
        .map(|(_, enchantment)| enchantment.defense)
        .fold(0u16, u16::saturating_add);
    raw.saturating_sub(defense)
}

/// Applies the chosen disables to the lattice and breaks any enchantment whose
/// funding gem just went down, consuming its locked mana.
///
/// Returns one [`BrokenEnchantment`] per enchantment broken this call. Disabling an
/// already-disabled cell is a no-op, so applying a cell list is idempotent.
pub fn apply_disables(state: &mut LatticeState, cells: &[LatticeCoord]) -> Vec<BrokenEnchantment> {
    let mut broken = Vec::new();
    for &coord in cells {
        if !state.disable(coord) {
            continue;
        }
        let Some(id) = state.locked_by(coord) else {
            continue;
        };
        if let Some(enchantment) = state.break_enchant(id) {
            broken.push(BrokenEnchantment {
                enchant: id,
                spell: enchantment.spell,
                burned_mana: enchantment.locked_mana,
                trigger: coord,
            });
        }
    }
    broken
}

/// Re-enables the chosen cells, returning how many were actually restored.
///
/// The inverse of [`apply_disables`] for the hexes themselves, and **deliberately not
/// its inverse for anything else**. An enchantment broken when its funding gem went
/// down stays broken and its locked mana stays spent: breaking is what the design
/// charges for a hit that cracks a shield, and undoing it here would make a restoring
/// spell quietly refund mana it never paid for. Restoring a live cell is a no-op, so a
/// cell list is idempotent the same way disabling one is.
///
/// The count is the caller's honesty check — `RestoreHexes { count: 2 }` against a
/// lattice with one hex down restores one, and the caller needs to be able to say so
/// rather than report two. **It does not enforce a count**: this takes an
/// already-chosen list, exactly as [`apply_disables`] does, and how that list was
/// bounded is the caller's business.
///
/// A cell that is live, that holds no gem, or that is not in the lattice at all are
/// alike here — all three are `false`, and none is an error. For a restoring spell that
/// is right: healing a hex nobody hurt is a waste, not a fault.
pub fn restore(state: &mut LatticeState, cells: &[LatticeCoord]) -> usize {
    // A plain loop rather than `filter().count()`: the mutation is the point, and
    // hiding it in a lazy adaptor makes a future `.take(n)` or `.any()` silently
    // change how many cells actually come back.
    let mut restored = 0;
    for &coord in cells {
        if state.restore(coord) {
            restored += 1;
        }
    }
    restored
}

/// Advances every burn one of the target's turns and returns how many hexes the
/// burns disable this turn.
///
/// The caller applies those disables — burn ignores armour, so it does *not* run
/// them through [`resolve_incoming`] — via [`apply_disables`] once it (or the
/// defender) has chosen which hexes. Expired burns are dropped.
pub fn tick_burns(state: &mut LatticeState) -> u16 {
    state.advance_burns()
}
