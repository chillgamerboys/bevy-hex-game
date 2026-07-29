//! The defender's answer: which hexes a landed hit takes down.
//!
//! The design's one mid-resolution decision. Damage names a **count**; the defender
//! picks **which**, except for the rare abilities that target hexes directly. That makes
//! early damage nearly free — you give up junk hexes — and late damage catastrophic,
//! because everything still standing is load-bearing.
//!
//! # Why it is a command rather than a function call
//!
//! The applier could pick for the defender and be done in one frame. It must not,
//! because **the choice has to be in the replay log**. A fight replays by re-running its
//! commands; a choice made inside the applier and never written down would be re-derived
//! on replay, and any change to the policy — or a human answering in co-op — would make
//! the same log produce a different fight.
//!
//! So the applier parks a [`PendingDecision`], something answers it by pushing a
//! `ChooseDisables`, and this handler applies that answer. The lattice UI answers for
//! a player defender; the policy in [`crate::ai`] answers for everyone else.

use bevy::prelude::*;

use hex_core::{LatticeCoord, PendingDecision, UnitId};
use hex_lattice::apply_disables;

use crate::{CombatEvent, CommandRefusal, UnitData};

use super::{cast::LatticeQuery, Verb};

/// Applies the defender's answer, or returns the reason it was refused.
pub(super) fn apply(
    ctx: &mut Verb,
    lattices: &mut LatticeQuery,
    unit: UnitId,
    entity: Entity,
    cells: &[LatticeCoord],
) -> Result<(), CommandRefusal> {
    let PendingDecision::ChooseDisables {
        decider,
        count,
        source,
    } = *ctx.pending
    else {
        return Err(CommandRefusal::NoPendingDecision);
    };
    // The answer must be the one that was asked for. A replayed or forged log must not
    // be able to disable a bystander's hexes by naming somebody else's unit.
    if decider != unit {
        return Err(CommandRefusal::WrongDecisionUnit { expected: decider });
    }
    let Ok((spec, mut state)) = lattices.get_mut(entity) else {
        return Err(CommandRefusal::MissingUnitData {
            unit,
            data: UnitData::Lattice,
        });
    };

    // The count is what the hit *asked* for; a lattice with less left than that gives
    // everything it has. Demanding an exact match would deadlock precisely at the moment
    // a unit is about to go down — the answer could never be satisfied, and resolution
    // would park forever on a decision nobody can meet.
    let live = spec
        .cells()
        .filter(|&(coord, _)| !state.is_disabled(coord))
        .count();
    let owed = usize::from(count).min(live);
    if cells.len() != owed {
        return Err(CommandRefusal::WrongDisableCount {
            expected: u32::try_from(owed).unwrap_or(u32::MAX),
            actual: u32::try_from(cells.len()).unwrap_or(u32::MAX),
        });
    }
    // Every cell has to be one of this lattice's own, and distinct. Without the first
    // check an answer could name coordinates that are not in the drawing at all, which
    // `apply_disables` would treat as no-ops — turning a hit into nothing. Without the
    // second, naming one hex twice would satisfy the count while taking down one.
    let mut seen: Vec<LatticeCoord> = Vec::with_capacity(cells.len());
    for &cell in cells {
        if spec.get(cell).is_none() {
            return Err(CommandRefusal::CellOutsideLattice { cell });
        }
        if seen.contains(&cell) {
            return Err(CommandRefusal::DuplicateCell { cell });
        }
        // And it has to still be standing. `apply_disables` treats an already-dead cell
        // as a no-op, so naming two corpses would satisfy the count and absorb the hit
        // for free — the same hole the membership check above closes, one step along.
        if state.is_disabled(cell) {
            return Err(CommandRefusal::CellAlreadyDisabled { cell });
        }
        seen.push(cell);
    }

    let broken = apply_disables(&mut state, cells);
    *ctx.pending = PendingDecision::None;

    ctx.events.push(CombatEvent::HexesDisabled {
        source,
        target: unit,
        cells: cells.to_vec(),
    });
    for record in &broken {
        let spell = ctx
            .spells
            .and_then(|spells| spells.name(record.spell))
            .map(str::to_owned);
        ctx.events.push(CombatEvent::EnchantmentBroken {
            unit,
            spell,
            burned_mana: record.burned_mana,
            trigger: record.trigger,
        });
        info!(
            "damage: {unit:?} loses an enchantment — {} mana burned with it",
            record.burned_mana
        );
    }
    // What fell, not what was asked for: a spent lattice gives fewer than the hit
    // demanded, and a log that reported the demand would overstate every killing blow.
    info!(
        "damage: {source:?} disables {} of {unit:?}'s hexes",
        cells.len()
    );
    Ok(())
}
