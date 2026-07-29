//! The applier: the one place a command becomes a change to the sim.
//!
//! Emitters — the click handler, the end-turn key, the AI — resolve *intent*
//! and push [`IssuedCommand`]s. This module drains the [`CommandQueue`] in
//! issue order, validates each command against the rules, and either applies
//! it or drops it with a logged reason. Nothing else mutates turn budgets,
//! starts walks, or lands strikes.
//!
//! # Where it runs
//!
//! [`CombatSystems::Apply`](crate::CombatSystems), between the AI's decision
//! and the turn advancing, inside `PausableSystems` — so a command issued just
//! before a pause waits in the queue rather than being lost, which is the whole
//! reason the funnel is a queue and not a `Message` (see
//! [`hex_core::commands`]).
//!
//! Despite the set's name, the applier is not combat-only: exploring moves are
//! commands too. Validation is what differs by [`Mode`] — free movement in
//! real time, turn ownership and budgets in combat.

use bevy::prelude::*;

use hex_anim::Transformation;
use hex_assets::{PlayerSettings, SubstanceTable};
use hex_core::{
    AppSystems, Busy, CommandQueue, ControlOwner, GameCommand, IssuedCommand, Mode,
    PausableSystems, PendingDecision, Screen, TilePos, TraversalBlockers, Turn,
};
use hex_units::{
    Body, Faction, Footing, HexPathingLine, MovingTo, Standing, StandsOn, UnitRegistry,
};

use crate::turns::TurnOrder;

/// How far the attacker leans toward its target, as a fraction of the distance.
///
/// Small on purpose: it must read as a swing rather than as a move, or the
/// player cannot tell an attack from a step.
const LUNGE_FRACTION: f32 = 0.35;

/// Seconds for the lunge out and the return. Slow enough to notice, short
/// enough not to make a turn feel like waiting.
const LUNGE_SECONDS: f32 = 0.18;

/// Tiles, as the applier needs them to ground a commanded path.
type TileQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static TilePos,
        &'static hex_core::HexSpan,
        &'static hex_core::SubstanceId,
        &'static hex_core::Headroom,
    ),
    With<hex_core::HexTile>,
>;

/// Everything the applier reads or writes about one unit.
///
/// Standing and body are `Option` because not every verb needs them — ending a
/// turn moves nothing — and a test harness spawns only what its verbs read.
/// Each verb demands what it actually uses and drops the command otherwise.
type ActorQuery<'w, 's> = Query<
    'w,
    's,
    (
        Option<&'static StandsOn>,
        Option<&'static Body>,
        Option<&'static mut Turn>,
        Has<Busy>,
        Option<&'static ControlOwner>,
        Option<&'static Faction>,
    ),
>;

pub(crate) fn plugin(app: &mut App) {
    app.init_resource::<CommandQueue>()
        .register_type::<GameCommand>()
        .register_type::<IssuedCommand>()
        .register_type::<Busy>()
        .register_type::<PendingDecision>();
    app.add_systems(
        Update,
        (
            // Frees units whose walk or swing landed this frame, before anyone
            // decides or applies anything on their behalf.
            sync_busy
                .in_set(AppSystems::Update)
                .in_set(PausableSystems)
                .after(hex_units::MovementSystems::Reconcile)
                .before(crate::CombatSystems::Act)
                .run_if(in_state(Screen::Gameplay)),
            apply_commands
                .in_set(crate::CombatSystems::Apply)
                .in_set(PausableSystems)
                .run_if(in_state(Screen::Gameplay)),
        ),
    );
    // Unit ids reset between sessions, so a held-over command would name
    // somebody else's unit next launch.
    app.add_systems(OnExit(Screen::Gameplay), clear_queue);
}

fn clear_queue(mut queue: ResMut<CommandQueue>) {
    queue.clear();
}

/// Keeps [`Busy`] equal to "presentation in flight".
///
/// The applier inserts [`Busy`] eagerly when it commits an animation; this
/// system is the other half, removing it once both the [`Transformation`] and
/// the [`MovingTo`] it stood for are gone — and re-asserting it for any
/// presentation that arrived outside the funnel, so the marker can be trusted
/// wherever it is read.
fn sync_busy(
    mut commands: Commands,
    units: Query<
        (Entity, Has<Transformation>, Has<MovingTo>, Has<Busy>),
        Or<(With<Busy>, With<Transformation>, With<MovingTo>)>,
    >,
) {
    for (entity, animating, walking, busy) in &units {
        let active = animating || walking;
        if active && !busy {
            commands.entity(entity).insert(Busy);
        } else if !active && busy {
            commands.entity(entity).remove::<Busy>();
        }
    }
}

/// Drains the queue: validate, apply, project.
///
/// Commands apply in issue order, whole-queue-per-run. A dropped command is a
/// `warn!` with its reason — a drop is an emitter bug, a verb that is not
/// built yet, or input that lost a race with the frame it landed on (a key or
/// click arriving exactly as the mode flipped or a turn passed). All of them
/// deserve a line: the first two are defects, and the third explains itself.
fn apply_commands(
    mut commands: Commands,
    mut queue: ResMut<CommandQueue>,
    mode: Res<State<Mode>>,
    turn_order: Res<TurnOrder>,
    registry: Res<UnitRegistry>,
    settings: Option<Res<PlayerSettings>>,
    table: Option<Res<SubstanceTable>>,
    blockers: Option<Res<TraversalBlockers>>,
    tiles: TileQuery,
    mut actors: ActorQuery,
) {
    // Units this drain already committed presentation for. `Busy` lands via
    // `Commands` and is not queryable until the next sync point, so within one
    // drain this set is the truth.
    let mut committed: Vec<Entity> = Vec::new();
    let in_combat = *mode.get() == Mode::Combat;

    while let Some(issued) = queue.pop() {
        let unit = issued.command.unit();
        let Some(entity) = registry.entity_of(unit) else {
            drop_command(&issued, "no such unit");
            continue;
        };

        // Seat validation. Units without an owner belong to seat 0 — "the only
        // session there is" — matching how they are spawned; the check grows
        // teeth the moment a second seat exists.
        let owner = actors
            .get(entity)
            .ok()
            .and_then(|(_, _, _, _, owner, _)| owner.copied())
            .unwrap_or_default();
        if owner.0 != issued.seat {
            drop_command(&issued, "issued by a seat that does not own the unit");
            continue;
        }

        match issued.command {
            GameCommand::MoveAlong { ref path, .. } => {
                // `in_combat` is the mode at application. A click emitted in
                // the last exploring frame can therefore apply as the first
                // combat move, billed like any other — accepted: it is
                // validated against the same rules as a move ordered a frame
                // later, and the one-frame window cannot be closed without
                // stamping commands with the mode they were issued under.
                if in_combat && turn_order.current() != Some(unit) {
                    drop_command(&issued, "not this unit's turn");
                    continue;
                }
                let Ok((standing, body, turn, busy, _, _)) = actors.get_mut(entity) else {
                    drop_command(&issued, "unit no longer exists");
                    continue;
                };
                let (Some(standing), Some(body)) = (standing, body) else {
                    drop_command(&issued, "unit has no standing or body to walk with");
                    continue;
                };
                if busy || committed.contains(&entity) {
                    drop_command(&issued, "unit is still finishing its last action");
                    continue;
                }
                let Some(table) = table.as_deref() else {
                    drop_command(&issued, "no substance table to ground the path against");
                    continue;
                };
                let footing = Footing::from_tiles(tiles.iter(), table, *body, blockers.as_deref());
                let Some(steps) = ground_path(path, standing.0, &footing) else {
                    drop_command(&issued, "path is not walkable from where the unit stands");
                    continue;
                };

                // A route of N surfaces costs N-1 steps.
                let cost = u32::try_from(steps.len().saturating_sub(1)).unwrap_or(u32::MAX);
                if in_combat {
                    let Some(mut turn) = turn else {
                        drop_command(&issued, "no turn to spend movement from");
                        continue;
                    };
                    if cost > turn.movement_left {
                        drop_command(&issued, "path costs more movement than remains");
                        continue;
                    }
                    turn.movement_left -= cost;
                }

                let mut unit_commands = commands.entity(entity);
                if let Some(settings) = settings.as_deref() {
                    let animation: Transformation =
                        HexPathingLine::new(&steps, settings.speed).into();
                    unit_commands.insert((animation, MovingTo::new(steps, settings.speed), Busy));
                } else {
                    // Headless: no speed to animate with. The route still
                    // commits, and reconciliation lands it immediately.
                    unit_commands.insert((MovingTo::new(steps, 0.0), Busy));
                }
                committed.push(entity);
            }
            GameCommand::Strike { target, .. } => {
                if !in_combat {
                    drop_command(&issued, "strikes only happen in combat");
                    continue;
                }
                if turn_order.current() != Some(unit) {
                    drop_command(&issued, "not this unit's turn");
                    continue;
                }
                let Some(target_entity) = registry.entity_of(target) else {
                    drop_command(&issued, "no such target");
                    continue;
                };
                let Some((target_standing, target_faction)) = actors
                    .get(target_entity)
                    .ok()
                    .and_then(|(standing, _, _, _, _, faction)| {
                        Some((standing.copied()?, faction.copied()?))
                    })
                else {
                    drop_command(&issued, "target has no standing or faction to be struck at");
                    continue;
                };
                let target_standing = target_standing.0;
                let Ok((standing, body, turn, busy, _, faction)) = actors.get_mut(entity) else {
                    drop_command(&issued, "unit no longer exists");
                    continue;
                };
                let (Some(standing), Some(body), Some(faction)) = (standing, body, faction) else {
                    drop_command(
                        &issued,
                        "unit has no standing, body, or faction to strike with",
                    );
                    continue;
                };
                if busy || committed.contains(&entity) {
                    drop_command(&issued, "unit is still finishing its last action");
                    continue;
                }
                // The rules live here, not in the emitters: today's only
                // strike emitter already filters hostiles, but a replayed or
                // forged log must not be able to make allies swing at each
                // other.
                if !faction.is_hostile_to(target_faction) {
                    drop_command(&issued, "target is not hostile to this unit");
                    continue;
                }
                let Some(table) = table.as_deref() else {
                    drop_command(&issued, "no substance table to judge reach against");
                    continue;
                };
                // **Reach, not range.** Melee is the step rule both ways: an
                // attacker five levels up must not acquire a two-hex punch.
                let footing = Footing::from_tiles(tiles.iter(), table, *body, blockers.as_deref());
                if !(footing.admits_step(standing.0.pos, target_standing.pos)
                    && footing.admits_step(target_standing.pos, standing.0.pos))
                {
                    drop_command(&issued, "target is out of melee reach");
                    continue;
                }
                let Some(mut turn) = turn else {
                    drop_command(&issued, "no turn to take the action from");
                    continue;
                };
                if turn.acted {
                    drop_command(&issued, "unit already took its action");
                    continue;
                }
                turn.acted = true;

                let striker_standing = standing.0;
                if let Some(settings) = settings.as_deref() {
                    lunge(
                        &mut commands,
                        entity,
                        striker_standing,
                        target_standing,
                        settings.speed,
                    );
                    recoil(
                        &mut commands,
                        target_entity,
                        target_standing,
                        striker_standing,
                        settings.speed,
                    );
                    commands.entity(entity).insert(Busy);
                    committed.push(entity);
                }
                // Nothing deals damage yet: damage disables lattice hexes, and
                // there are no lattices. The strike is an animation and this
                // log line, exactly as the crate docs promise.
                info!("strike: {unit:?} hits {target:?}");
            }
            GameCommand::EndTurn { .. } => {
                if !in_combat {
                    drop_command(&issued, "no turns to end outside combat");
                    continue;
                }
                if turn_order.current() != Some(unit) {
                    drop_command(&issued, "not this unit's turn");
                    continue;
                }
                let Ok((_, _, turn, _, _, _)) = actors.get_mut(entity) else {
                    drop_command(&issued, "unit no longer exists");
                    continue;
                };
                let Some(mut turn) = turn else {
                    drop_command(&issued, "current unit carries no turn to end");
                    continue;
                };
                // Yield everything. Deliberately legal while the unit is still
                // moving: ending a turn is a declaration, not presentation, and
                // `advance_turn` already waits for the walk to land.
                turn.acted = true;
                turn.movement_left = 0;
            }
            GameCommand::Cast { .. } | GameCommand::Channel { .. } => {
                drop_command(&issued, "not built yet — waits on lattices (HEX-12)");
            }
            GameCommand::ChooseDisables { .. } => {
                drop_command(&issued, "not built yet — waits on the damage model");
            }
        }
    }
}

/// Says exactly what was refused and why, once per drop.
fn drop_command(issued: &IssuedCommand, reason: &str) {
    warn!("command dropped ({reason}): {issued:?}");
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

/// A short lean toward the target and back, as the visible half of an attack.
///
/// Built from the same primitives as walking, which is a useful check that the
/// `hex_anim` split holds: the animation engine needed nothing added to express
/// a swing it was never designed for.
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
/// Returning to `start` matters: the animation ends and the component is
/// removed, and anything that ended somewhere else would leave the piece off
/// its tile with nothing to correct it.
fn there_and_back(start: Vec3, tip: Vec3, speed: f32) -> Transformation {
    // Speed is derived from the distance so both legs take `LUNGE_SECONDS`,
    // whatever the lunge length works out to. Guard the degenerate case: a
    // zero-length leg makes `LinearMovement` produce NaN.
    let distance = start.distance(tip);
    if distance <= f32::EPSILON {
        // Nothing to animate. A stationary "swing" is better than a NaN
        // transform, which would put the piece somewhere unrenderable and
        // never come back.
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
