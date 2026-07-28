//! Melee: validate a swing and play it.
//!
//! **Nothing deals damage yet.** Damage disables lattice hexes, and units do
//! not carry lattices — so a strike is an animation and a log line, exactly as
//! the crate docs promise. The seam where damage lands is marked below.

use bevy::prelude::*;

use hex_core::{Busy, UnitId};
use hex_units::Footing;

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
) -> Result<(), &'static str> {
    if !ctx.in_combat {
        return Err("strikes only happen in combat");
    }
    if ctx.turn_order.current() != Some(unit) {
        return Err("not this unit's turn");
    }
    let Some(target_entity) = ctx.registry.entity_of(target) else {
        return Err("no such target");
    };
    let Some((target_standing, target_faction)) = actors
        .get(target_entity)
        .ok()
        .and_then(|(standing, _, _, _, _, faction)| Some((standing.copied()?, faction.copied()?)))
    else {
        return Err("target has no standing or faction to be struck at");
    };
    let target_standing = target_standing.0;
    let Ok((standing, body, turn, busy, _, faction)) = actors.get_mut(entity) else {
        return Err("unit no longer exists");
    };
    let (Some(standing), Some(body), Some(faction)) = (standing, body, faction) else {
        return Err("unit has no standing, body, or faction to strike with");
    };
    if busy || ctx.committed.contains(&entity) {
        return Err("unit is still finishing its last action");
    }
    // The rules live here, not in the emitters: today's only strike emitter
    // already filters hostiles, but a replayed or forged log must not be able
    // to make allies swing at each other.
    if !faction.is_hostile_to(target_faction) {
        return Err("target is not hostile to this unit");
    }
    let Some(table) = ctx.table else {
        return Err("no substance table to judge reach against");
    };
    // **Reach, not range.** Melee is the step rule both ways: an attacker five
    // levels up must not acquire a two-hex punch.
    let footing = Footing::from_tiles(tiles.iter(), table, *body);
    if !(footing.admits_step(standing.0.pos, target_standing.pos)
        && footing.admits_step(target_standing.pos, standing.0.pos))
    {
        return Err("target is out of melee reach");
    }
    let Some(mut turn) = turn else {
        return Err("no turn to take the action from");
    };
    if turn.acted {
        return Err("unit already took its action");
    }
    turn.acted = true;

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
