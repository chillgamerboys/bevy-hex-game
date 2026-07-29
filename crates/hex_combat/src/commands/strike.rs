//! Melee: validate a swing and play it.
//!
//! A strike commits presentation, then names an incoming disable count. Defences
//! subtract from it and the defender chooses the exact lattice cells through the
//! same replayable decision seam used by spell damage and Burn.

use bevy::prelude::*;

use hex_core::{Busy, UnitId};
use hex_units::Footing;

use crate::{CombatData, CombatEvent, CommandRefusal, UnitData};

use super::cast::{open_disable_decision, LatticeQuery};
use super::{presentation, ActorQuery, TileQuery, Verb};

/// Applies a strike, or returns the reason it was refused.
pub(super) fn apply(
    ctx: &mut Verb,
    commands: &mut Commands,
    tiles: &TileQuery,
    actors: &mut ActorQuery,
    lattices: &mut LatticeQuery,
    unit: UnitId,
    entity: Entity,
    target: UnitId,
) -> Result<(), CommandRefusal> {
    if !ctx.in_combat {
        return Err(CommandRefusal::CombatOnly);
    }
    if ctx.turn_order.current() != Some(unit) {
        return Err(CommandRefusal::NotCurrentTurn {
            current: ctx.turn_order.current(),
        });
    }
    // Same rule as casting: a second hit landing while a defender still owes an answer
    // would overwrite the open decision and silently erase the first one's damage.
    if ctx.pending.is_open() {
        let decider = match *ctx.pending {
            hex_core::PendingDecision::ChooseDisables { decider, .. } => decider,
            hex_core::PendingDecision::None => unit,
        };
        return Err(CommandRefusal::DecisionPending { decider });
    }
    let Some(target_entity) = ctx.registry.entity_of(target) else {
        return Err(CommandRefusal::UnknownTarget { target });
    };
    let Ok((target_standing, _, _, _, _, target_faction, target_downed)) =
        actors.get(target_entity)
    else {
        return Err(CommandRefusal::MissingUnitData {
            unit: target,
            data: UnitData::EntityRecord,
        });
    };
    if target_downed {
        return Err(CommandRefusal::TargetDowned { target });
    }
    let Some(target_standing) = target_standing.copied() else {
        return Err(CommandRefusal::MissingUnitData {
            unit: target,
            data: UnitData::Standing,
        });
    };
    let Some(target_faction) = target_faction.copied() else {
        return Err(CommandRefusal::MissingUnitData {
            unit: target,
            data: UnitData::Faction,
        });
    };
    let target_standing = target_standing.0;
    let Ok((standing, body, turn, busy, _, faction, _)) = actors.get_mut(entity) else {
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
    let Some(faction) = faction else {
        return Err(CommandRefusal::MissingUnitData {
            unit,
            data: UnitData::Faction,
        });
    };
    if busy || ctx.committed.contains(&entity) {
        return Err(CommandRefusal::Busy);
    }
    // The rules live here, not in the emitters: today's only strike emitter
    // already filters hostiles, but a replayed or forged log must not be able
    // to make allies swing at each other.
    if !faction.is_hostile_to(target_faction) {
        return Err(CommandRefusal::TargetNotHostile { target });
    }
    let Some(table) = ctx.table else {
        return Err(CommandRefusal::MissingCombatData {
            data: CombatData::SubstanceTable,
        });
    };
    // **Reach, not range.** Melee is the step rule both ways: an attacker five
    // levels up must not acquire a two-hex punch.
    let footing = Footing::from_tiles(tiles.iter(), table, *body, ctx.blockers);
    if !(footing.admits_step(standing.0.pos, target_standing.pos)
        && footing.admits_step(target_standing.pos, standing.0.pos))
    {
        return Err(CommandRefusal::TargetOutOfMeleeReach { target });
    }
    let Some(mut turn) = turn else {
        return Err(CommandRefusal::NoTurn);
    };
    if turn.acted {
        return Err(CommandRefusal::ActionAlreadySpent);
    }
    turn.acted = true;
    ctx.events.push(CombatEvent::Strike {
        attacker: unit,
        target,
    });

    let striker_standing = standing.0;
    if let Some(settings) = ctx.settings {
        presentation::lunge(
            commands,
            entity,
            striker_standing,
            target_standing,
            settings.speed,
        );
        presentation::recoil(
            commands,
            target_entity,
            target_standing,
            striker_standing,
            settings.speed,
        );
        commands.entity(entity).insert(Busy);
        ctx.committed.push(entity);
    }
    // The damage seam, now wired. A strike is the one attack every unit has — a wolf
    // is four hexes and a bite — so it deals damage the same way a spell does: it names
    // a count, the defender's defences subtract from it, and the defender chooses which
    // hexes go down. Nothing about melee is special except where the number comes from.
    let count = ctx.combat.map_or(0, |settings| settings.strike_disables);
    if count > 0 {
        open_disable_decision(ctx, lattices, target, target_entity, unit, count);
    }
    info!("strike: {unit:?} hits {target:?}");
    Ok(())
}
