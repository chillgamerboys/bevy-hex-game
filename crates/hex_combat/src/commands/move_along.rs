//! Walking: validate a commanded route and commit it.

use bevy::prelude::*;

use hex_anim::Transformation;
use hex_core::{Busy, TilePos, UnitId};
use hex_units::{Footing, HexPathingLine, MovingTo, Standing};

use crate::{CombatData, CommandRefusal, UnitData};

use super::{ActorQuery, TileQuery, Verb};

/// Applies a move, or returns the reason it was refused.
pub(super) fn apply(
    ctx: &mut Verb,
    commands: &mut Commands,
    tiles: &TileQuery,
    actors: &mut ActorQuery,
    unit: UnitId,
    entity: Entity,
    path: &[TilePos],
) -> Result<(), CommandRefusal> {
    // `in_combat` is the mode at application. A click emitted in the last
    // exploring frame can therefore apply as the first combat move, billed
    // like any other — accepted: it is validated against the same rules as a
    // move ordered a frame later, and the one-frame window cannot be closed
    // without stamping commands with the mode they were issued under.
    if ctx.in_combat && ctx.turn_order.current() != Some(unit) {
        return Err(CommandRefusal::NotCurrentTurn {
            current: ctx.turn_order.current(),
        });
    }
    let Ok((standing, body, turn, busy, _, _, _)) = actors.get_mut(entity) else {
        return Err(CommandRefusal::MissingUnitData {
            unit,
            data: UnitData::EntityRecord,
        });
    };
    let Some(standing) = standing else {
        return Err(CommandRefusal::MissingUnitData {
            unit,
            data: UnitData::Standing,
        });
    };
    let Some(body) = body else {
        return Err(CommandRefusal::MissingUnitData {
            unit,
            data: UnitData::Body,
        });
    };
    if busy || ctx.committed.contains(&entity) {
        return Err(CommandRefusal::Busy);
    }
    let Some(table) = ctx.table else {
        return Err(CommandRefusal::MissingCombatData {
            data: CombatData::SubstanceTable,
        });
    };
    let footing = Footing::from_tiles(tiles.iter(), table, *body);
    let Some(steps) = ground_path(path, standing.0, &footing) else {
        return Err(CommandRefusal::InvalidPath);
    };

    // A route of N surfaces costs N-1 steps.
    let cost = u32::try_from(steps.len().saturating_sub(1)).unwrap_or(u32::MAX);
    if ctx.in_combat {
        let Some(mut turn) = turn else {
            return Err(CommandRefusal::NoTurn);
        };
        if cost > turn.movement_left {
            return Err(CommandRefusal::MovementBudgetExceeded {
                cost,
                remaining: turn.movement_left,
            });
        }
        turn.movement_left -= cost;
    }

    let mut unit_commands = commands.entity(entity);
    if let Some(settings) = ctx.settings {
        let animation: Transformation = HexPathingLine::new(&steps, settings.speed).into();
        unit_commands.insert((animation, MovingTo::new(steps, settings.speed), Busy));
    } else {
        // Headless: no speed to animate with. The route still commits, and
        // reconciliation lands it immediately.
        unit_commands.insert((MovingTo::new(steps, 0.0), Busy));
    }
    ctx.committed.push(entity);
    Ok(())
}

/// Grounds a commanded path against the live terrain.
///
/// Returns the path as standings when it starts where the unit stands, every
/// consecutive pair is a legal step for this body, and it actually goes
/// somewhere. [`None`] is a validation failure, never a partial path — a
/// command applies whole or not at all.
fn ground_path(path: &[TilePos], from: Standing, footing: &Footing) -> Option<Vec<Standing>> {
    if path.len() < 2 || path.first() != Some(&from.pos) {
        return None;
    }
    let mut steps = Vec::with_capacity(path.len());
    for pair in path.windows(2) {
        let (&a, &b) = match pair {
            [a, b] => (a, b),
            _ => return None,
        };
        if !footing.admits_step(a, b) {
            return None;
        }
        if steps.is_empty() {
            steps.push(footing.at(a)?);
        }
        steps.push(footing.at(b)?);
    }
    Some(steps)
}
