//! What an enemy does with its turn.
//!
//! **This is a placeholder and should read as one.** It closes the distance and
//! swings; that is the whole repertoire. It exists so a turn visibly passes and so
//! the colleague testing terrain has something that walks over it, not because it is
//! the enemy behaviour the game wants.
//!
//! Real behaviour needs things that do not exist yet — lattices to decide what an
//! enemy *can* cast, hidden information to decide what it knows, and a rout threshold
//! to decide when it stops. All three are in the design and none are built.
//!
//! # One thing per turn
//!
//! An enemy either moves or attacks, then ends its turn. Not "move and attack", even
//! though a player gets both, because a placeholder that spends a full turn's economy
//! invites being tuned rather than replaced.

use bevy::prelude::*;

use hex_anim::Transformation;
use hex_assets::{PlayerSettings, SubstanceTable};
use hex_core::{Headroom, HexSpan, HexTile, Mode, PausableSystems, SubstanceId, TilePos, Turn};
use hex_units::{route, Body, Enemy, Faction, Footing, HexPathingLine, Standing, StandsOn};

use crate::turns::TurnOrder;

/// How far the attacker leans toward its target, as a fraction of the distance.
///
/// Small on purpose: it must read as a swing rather than as a move, or the player
/// cannot tell an attack from a step.
const LUNGE_FRACTION: f32 = 0.35;

/// Seconds for the lunge out and the return. Slow enough to notice, short enough not
/// to make a turn feel like waiting.
const LUNGE_SECONDS: f32 = 0.18;

/// Tiles, as the AI needs them to work out where it can walk.
type TileQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static TilePos,
        &'static HexSpan,
        &'static SubstanceId,
        &'static Headroom,
    ),
    With<HexTile>,
>;

pub(crate) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        take_enemy_turn
            .in_set(PausableSystems)
            .run_if(in_state(Mode::Combat)),
    );
}

/// Moves toward the nearest enemy, or swings if already next to one.
///
/// Runs only for the unit whose turn it is, and only once that unit has stopped
/// moving — a second decision taken mid-animation would queue a new path on top of
/// the one still playing.
fn take_enemy_turn(
    mut commands: Commands,
    turn_order: Res<TurnOrder>,
    mut acting: Query<
        (Entity, &mut Turn, &StandsOn, &Body, &Faction),
        (With<Enemy>, Without<Transformation>),
    >,
    others: Query<(Entity, &Faction, &StandsOn)>,
    tiles: TileQuery,
    table: Option<Res<SubstanceTable>>,
    settings: Option<Res<PlayerSettings>>,
) {
    let (Some(table), Some(settings)) = (table, settings) else {
        return;
    };
    let Some(current) = turn_order.current() else {
        return;
    };
    let Ok((entity, mut turn, standing, body, faction)) = acting.get_mut(current) else {
        // Not an enemy's turn, or it is still mid-stride.
        return;
    };
    if turn.acted {
        return;
    }

    let Some((target_entity, target)) = nearest_foe(&others, *faction, standing.0) else {
        // Nothing to fight. Spend the turn so the order keeps moving rather than
        // stalling on a unit with nothing to do.
        spend(&mut turn);
        return;
    };

    // Adjacent already: swing.
    if standing.0.pos.coord.distance(target.pos.coord) == 1 {
        lunge(&mut commands, entity, standing.0, target, settings.speed);
        recoil(
            &mut commands,
            target_entity,
            target,
            standing.0,
            settings.speed,
        );
        info!("enemy attacks");
        spend(&mut turn);
        return;
    }

    // Otherwise close the distance as far as this turn allows.
    let footing = Footing::from_tiles(tiles.iter(), &table, *body);
    if let Some(steps) = approach(standing.0, target, &footing, turn.movement_left) {
        let animation: Transformation = HexPathingLine::new(&steps, settings.speed).into();
        if let Some(destination) = steps.last() {
            commands
                .entity(entity)
                .insert((animation, StandsOn(*destination)));
        }
    }
    spend(&mut turn);
}

/// Marks a turn as finished. `advance_turn` picks it up from here.
fn spend(turn: &mut Turn) {
    turn.acted = true;
    turn.movement_left = 0;
}

/// The closest unit hostile to `faction`.
fn nearest_foe(
    others: &Query<(Entity, &Faction, &StandsOn)>,
    faction: Faction,
    from: Standing,
) -> Option<(Entity, Standing)> {
    others
        .iter()
        .filter(|(_, other, _)| faction.is_hostile_to(**other))
        .min_by_key(|(_, _, standing)| from.pos.coord.distance(standing.0.pos.coord))
        .map(|(entity, _, standing)| (entity, standing.0))
}

/// The steps to take toward `target`, stopping adjacent to it and within `budget`.
///
/// [`None`] when there is nowhere to go — no route, already adjacent, or no movement
/// left. `route` searches the whole standable graph, so an enemy behind a wall walks
/// around it rather than standing there, and the clamp to `budget` below is the
/// ordinary case rather than the rare one: closing a long distance simply takes
/// several turns.
fn approach(
    from: Standing,
    target: Standing,
    footing: &Footing,
    budget: u32,
) -> Option<Vec<Standing>> {
    if budget == 0 {
        return None;
    }
    let full = route(from, target, footing)?;

    // `full` runs from where we stand to the target's own surface. Stopping one short
    // leaves the attacker adjacent, which is where it wants to be anyway.
    let adjacent_index = full.len().checked_sub(2)?;
    let reachable = adjacent_index.min(budget as usize);
    if reachable == 0 {
        return None;
    }
    full.get(..=reachable).map(<[Standing]>::to_vec)
}

/// A short lean toward the target and back, as the visible half of an attack.
///
/// Built from the same primitives as walking, which is a useful check that the
/// `hex_anim` split holds: the animation engine needed nothing added to express a
/// swing it was never designed for.
fn lunge(commands: &mut Commands, entity: Entity, from: Standing, toward: Standing, speed: f32) {
    let start = from.world_position();
    let tip = start + (toward.world_position() - start) * LUNGE_FRACTION;
    commands
        .entity(entity)
        .insert(there_and_back(start, tip, speed));
}

/// The target's half: a smaller flinch directly away from the attacker.
fn recoil(
    commands: &mut Commands,
    entity: Entity,
    target: Standing,
    attacker: Standing,
    speed: f32,
) {
    let start = target.world_position();
    let away = start + (start - attacker.world_position()) * (LUNGE_FRACTION * 0.5);
    commands
        .entity(entity)
        .insert(there_and_back(start, away, speed));
}

/// An out-and-back movement that finishes exactly where it started.
///
/// Returning to `start` matters: the animation ends and the component is removed, and
/// anything that ended somewhere else would leave the piece off its tile with nothing
/// to correct it.
fn there_and_back(start: Vec3, tip: Vec3, speed: f32) -> Transformation {
    // Speed is derived from the distance so both legs take `LUNGE_SECONDS`, whatever
    // the lunge length works out to. Guard the degenerate case: a zero-length leg
    // makes `LinearMovement` produce NaN.
    let distance = start.distance(tip);
    if distance <= f32::EPSILON {
        // Nothing to animate. A stationary "swing" is better than a NaN transform,
        // which would put the piece somewhere unrenderable and never come back.
        return HexPathingLine::new(&[], speed).into();
    }
    let leg_speed = distance / LUNGE_SECONDS;

    let mut series = hex_anim::TransformerSeries::new();
    series.push(hex_anim::LinearMovement::new(start, tip, leg_speed, 0.0));
    series.push(hex_anim::LinearMovement::new(
        tip,
        start,
        leg_speed,
        f64::from(LUNGE_SECONDS),
    ));
    Transformation::new(series)
}
