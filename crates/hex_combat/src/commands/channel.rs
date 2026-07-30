//! Spending one combat action to refill the acting unit's lattice.

use std::collections::BTreeMap;

use bevy::prelude::*;
use hex_core::UnitId;
use hex_lattice::{channel, CellKind, LatticeStats};

use crate::{CombatData, CombatEvent, CommandRefusal, UnitData};

use super::{cast::LatticeQuery, ActorQuery, Verb};

/// Applies Channel, or returns the exact reason it was refused.
pub(super) fn apply(
    ctx: &mut Verb,
    actors: &mut ActorQuery,
    lattices: &mut LatticeQuery,
    stats: &Query<&LatticeStats>,
    unit: UnitId,
    entity: Entity,
) -> Result<(), CommandRefusal> {
    if !ctx.in_combat {
        return Err(CommandRefusal::CombatOnly);
    }
    let Ok((_, _, turn, busy, _, _, downed)) = actors.get_mut(entity) else {
        return Err(CommandRefusal::MissingUnitData {
            unit,
            data: UnitData::EntityRecord,
        });
    };
    if downed {
        return Err(CommandRefusal::ActingUnitDowned { unit });
    }
    if ctx.turn_order.current() != Some(unit) {
        return Err(CommandRefusal::NotCurrentTurn {
            current: ctx.turn_order.current(),
        });
    }
    if busy || ctx.committed.contains(&entity) {
        return Err(CommandRefusal::Busy);
    }
    let Some(mut turn) = turn else {
        return Err(CommandRefusal::NoTurn);
    };
    if turn.acted {
        return Err(CommandRefusal::ActionAlreadySpent);
    }
    let Ok((spec, mut state)) = lattices.get_mut(entity) else {
        return Err(CommandRefusal::MissingUnitData {
            unit,
            data: UnitData::Lattice,
        });
    };
    let Ok(stats) = stats.get(entity) else {
        return Err(CommandRefusal::MissingUnitData {
            unit,
            data: UnitData::Lattice,
        });
    };
    let Some(elements) = ctx.elements else {
        return Err(CommandRefusal::MissingCombatData {
            data: CombatData::ElementCatalog,
        });
    };
    if spec.cells().any(
        |(_, kind)| matches!(kind, CellKind::Gem { element } if elements.name(element).is_none()),
    ) {
        return Err(CommandRefusal::MissingCombatData {
            data: CombatData::ElementCatalog,
        });
    }

    let mut restored = BTreeMap::new();
    for (element, amount) in channel(&mut state, spec, stats) {
        let Some(name) = elements.name(element) else {
            unreachable!("every gem element was validated before Channel mutated the lattice")
        };
        restored.insert(name.to_owned(), amount);
    }
    turn.acted = true;
    ctx.events.push(CombatEvent::Channelled { unit, restored });
    Ok(())
}
