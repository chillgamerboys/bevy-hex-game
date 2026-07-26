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
use hex_units::{
    route, Body, Enemy, Faction, Footing, HexPathingLine, MovingTo, Standing, StandsOn,
};

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
            .in_set(crate::CombatSystems::Act)
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

    let footing = Footing::from_tiles(tiles.iter(), &table, *body);
    let Some(plan) = best_foe(&others, *faction, standing.0, &footing, turn.movement_left) else {
        // Nothing to fight. Spend the turn so the order keeps moving rather than
        // stalling on a unit with nothing to do.
        spend(&mut turn);
        return;
    };

    match plan.action {
        FoeAction::Attack => {
            lunge(
                &mut commands,
                entity,
                standing.0,
                plan.target,
                settings.speed,
            );
            recoil(
                &mut commands,
                plan.entity,
                plan.target,
                standing.0,
                settings.speed,
            );
            info!("enemy attacks");
        }
        FoeAction::Move(approach) => {
            let animation: Transformation =
                HexPathingLine::new(&approach.steps, settings.speed).into();
            commands
                .entity(entity)
                .insert((animation, MovingTo::new(approach.steps, settings.speed)));
        }
        FoeAction::Wait => {}
    }
    spend(&mut turn);
}

/// Marks a turn as finished. `advance_turn` picks it up from here.
fn spend(turn: &mut Turn) {
    turn.acted = true;
    turn.movement_left = 0;
}

/// What the enemy can do about one foe this turn.
enum FoeAction {
    /// Already within melee reach.
    Attack,
    /// A terrain route exists and this is the affordable prefix of it.
    Move(Approach),
    /// No terrain route reaches this foe.
    Wait,
}

/// One candidate target and the action available against it.
struct FoePlan {
    entity: Entity,
    target: Standing,
    action: FoeAction,
}

impl FoePlan {
    /// Deterministic target priority: attack, routable approach, unreachable.
    ///
    /// Route cost decides between two approachable foes, horizontal distance is a
    /// stable secondary signal, and entity id resolves exact ties. Query iteration
    /// order is deliberately absent from the decision.
    fn priority(&self, from: Standing) -> (u8, usize, u32, u64) {
        let (kind, route_cost) = match &self.action {
            FoeAction::Attack => (0, 0),
            FoeAction::Move(approach) => (1, approach.route_cost),
            FoeAction::Wait => (2, usize::MAX),
        };
        (
            kind,
            route_cost,
            from.pos.coord.distance(self.target.pos.coord),
            self.entity.to_bits(),
        )
    }
}

/// The hostile unit that offers the best action from this terrain position.
///
/// Horizontal nearness is not routability on stacked terrain: a target on a bridge
/// directly overhead may be impossible to approach while another target two hexes
/// away has open ground all the way to it. Every candidate is planned before it is
/// ranked so the unreachable one cannot consume the turn merely by looking nearer on
/// the map.
fn best_foe(
    others: &Query<(Entity, &Faction, &StandsOn)>,
    faction: Faction,
    from: Standing,
    footing: &Footing,
    budget: u32,
) -> Option<FoePlan> {
    others
        .iter()
        .filter(|(_, other, _)| faction.is_hostile_to(**other))
        .map(|(entity, _, standing)| {
            let target = standing.0;
            let action = if footing.admits_step(from.pos, target.pos)
                && footing.admits_step(target.pos, from.pos)
            {
                // **Reach, not range.** Melee gets no high-ground bonus: an attacker
                // five levels up must not acquire a two-hex punch.
                FoeAction::Attack
            } else {
                approach(from, target, footing, budget).map_or(FoeAction::Wait, FoeAction::Move)
            };
            FoePlan {
                entity,
                target,
                action,
            }
        })
        .min_by_key(|plan| plan.priority(from))
}

/// A full route's tactical distance and the prefix affordable this turn.
struct Approach {
    steps: Vec<Standing>,
    route_cost: usize,
}

/// The steps to take toward `target`, stopping adjacent to it and within `budget`.
///
/// [`None`] when there is nowhere to go — no route, already adjacent, or no movement
/// left. `route` searches the whole standable graph, so an enemy behind a wall walks
/// around it rather than standing there, and the clamp to `budget` below is the
/// ordinary case rather than the rare one: closing a long distance simply takes
/// several turns.
fn approach(from: Standing, target: Standing, footing: &Footing, budget: u32) -> Option<Approach> {
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
    full.get(..=reachable).map(|steps| Approach {
        steps: steps.to_vec(),
        route_cost: adjacent_index,
    })
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
