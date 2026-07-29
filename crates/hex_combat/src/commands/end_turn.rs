//! Yielding the rest of a turn.

use bevy::prelude::*;

use hex_core::UnitId;

use crate::{CommandRefusal, UnitData};

use super::{ActorQuery, Verb};

/// Ends a turn, or returns the reason it was refused.
pub(super) fn apply(
    ctx: &mut Verb,
    actors: &mut ActorQuery,
    unit: UnitId,
    entity: Entity,
) -> Result<(), CommandRefusal> {
    if !ctx.in_combat {
        return Err(CommandRefusal::CombatOnly);
    }
    if ctx.turn_order.current() != Some(unit) {
        return Err(CommandRefusal::NotCurrentTurn {
            current: ctx.turn_order.current(),
        });
    }
    let Ok((_, _, turn, _, _, _, _)) = actors.get_mut(entity) else {
        return Err(CommandRefusal::MissingUnitData {
            unit,
            data: UnitData::EntityRecord,
        });
    };
    let Some(mut turn) = turn else {
        return Err(CommandRefusal::NoTurn);
    };
    // Yield everything. Deliberately legal while the unit is still moving:
    // ending a turn is a declaration, not presentation, and `advance_turn`
    // already waits for the walk to land.
    turn.acted = true;
    turn.movement_left = 0;
    Ok(())
}
