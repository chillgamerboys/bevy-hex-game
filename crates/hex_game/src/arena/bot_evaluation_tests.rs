//! Paired scripted opponents against the real world/gameplay composition.
//!
//! Scenario seeds vary the route, opening phase, and mirrored starting side; they
//! do not change either brain's private random seed. These are controlled policy
//! comparisons, not estimates of beginner or trained human win rates.

#![cfg(feature = "test-support")]

use super::*;
use hex_arena::{Actor, BotDebugSnapshot, RoundSummary};
use hex_core::arena::ArenaMaterials;
use hex_core::{HexCoord, TerrainEdit, TerrainImpact, TilePos};
use serde::Serialize;
use std::time::Instant;

const BASELINE_SOURCE_HEAD: &str = "8d20e13ad7521b1cd6294b14eeb56fbbce17f55b";
const CHARGE_SECONDS: f32 = 0.75;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
enum ScriptKind {
    StationaryTarget,
    Peeker,
    Rusher,
    Strafer,
}

struct Opponent {
    kind: ScriptKind,
    seed: u16,
    anchor: Vec3,
    forward: Vec3,
    last_target: Vec3,
    target_velocity: Vec3,
    next_cast: u64,
    armed_at: Option<u64>,
    recovery: PeekerRecovery,
    recovery_goal: Option<Vec3>,
    next_recovery_plan: u64,
    next_recovery_jump: u64,
}

#[derive(Default, Serialize)]
struct PeekerRecovery {
    active: bool,
    episodes: u32,
    completed: u32,
    jump_requests: u32,
}

impl Opponent {
    fn new(kind: ScriptKind, seed: u16, human: &Actor, bot: &Actor) -> Self {
        Self {
            kind,
            seed,
            anchor: human.feet,
            forward: (bot.feet - human.feet).with_y(0.0).normalize_or_zero(),
            // Both duelists start on the authored firing lane. This opening
            // waypoint is fixed at setup, never refreshed through opaque cover.
            last_target: bot.center(),
            target_velocity: Vec3::ZERO,
            next_cast: u64::from(seed) * 19,
            armed_at: None,
            recovery: PeekerRecovery::default(),
            recovery_goal: None,
            next_recovery_plan: 0,
            next_recovery_jump: 0,
        }
    }

    fn intent(
        &mut self,
        elapsed: u64,
        session: &ArenaSession,
        tuning: &ArenaTuning,
        terrain: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
    ) -> (ActorIntent, bool) {
        let human = session
            .actors
            .iter()
            .find(|actor| actor.id == 0)
            .expect("scripted actor");
        let bot = session
            .actors
            .iter()
            .find(|actor| actor.id == 1)
            .expect("opponent actor");
        let visible = actor_visible(session, human, bot);
        if visible {
            self.last_target = bot.center();
            self.target_velocity = ((bot.feet - bot.previous_feet) / hex_arena::STEP)
                .with_y(0.0)
                .clamp_length_max(7.0);
        }
        let phase = elapsed + u64::from(self.seed) * 19;
        let side = self.forward.cross(Vec3::Y);
        let to_target = (self.last_target - human.center()).with_y(0.0);
        let distance = to_target.length();
        let mut recovery_jump = false;
        let desired = match self.kind {
            ScriptKind::StationaryTarget => Vec3::ZERO,
            ScriptKind::Peeker => {
                let exposed = phase % 240 >= 150;
                let direction = if self.seed.is_multiple_of(2) {
                    1.0
                } else {
                    -1.0
                };
                let goal = self.anchor + side * if exposed { direction * 2.6 } else { 0.0 };
                let (goal, jump) = self.peeker_goal(elapsed, human, goal, terrain, geometry);
                recovery_jump = jump;
                let delta = (goal - human.feet).with_y(0.0);
                if delta.length() > 0.15 {
                    delta.normalize_or_zero()
                } else {
                    Vec3::ZERO
                }
            }
            ScriptKind::Rusher => to_target.normalize_or_zero(),
            ScriptKind::Strafer => {
                let strafe = if phase % 192 < 96 { 1.0 } else { -1.0 };
                let approach = if distance > 16.0 {
                    0.65
                } else if distance < 9.0 {
                    -0.65
                } else {
                    0.0
                };
                (side * strafe + to_target.normalize_or_zero() * approach).normalize_or_zero()
            }
        };
        let charge = human.charge();
        let speed =
            tuning.launch_speed(charge.map_or(tuning.charge_seconds, |charge| charge.elapsed));
        let aim = scripted_aim(
            human.eye(),
            self.last_target,
            self.target_velocity,
            speed,
            tuning.projectile_gravity,
        );
        let forward = aim.with_y(0.0).normalize_or(self.forward);
        let mut input = ActorIntent {
            aim,
            movement: Vec2::new(desired.dot(forward.cross(Vec3::Y)), desired.dot(forward)),
            run: matches!(self.kind, ScriptKind::Rusher),
            jump: recovery_jump
                || (matches!(self.kind, ScriptKind::Rusher) && phase.is_multiple_of(120)),
            ..default()
        };
        if matches!(self.kind, ScriptKind::StationaryTarget) {
            return (input, visible);
        }
        if let Some(charge) = charge {
            let expired = self
                .armed_at
                .is_some_and(|start| elapsed.saturating_sub(start) > 480);
            let too_close = distance < tuning.fireball_radius() + 0.8;
            if expired || too_close {
                self.armed_at = None; // Neutral held state cancels through authority.
            } else if visible && charge.elapsed >= tuning.charge_seconds {
                input.cast_released = true;
                self.armed_at = None;
                self.next_cast = elapsed + 42;
            } else {
                input.cast_held = true;
            }
            return (input, visible);
        }
        if elapsed < self.next_cast {
            return (input, visible);
        }
        let ready = |spell: Spell| {
            human
                .cooldowns
                .get(spell.index())
                .is_some_and(|cooldown| *cooldown <= hex_arena::STEP)
        };
        if visible && distance < tuning.blast_radius() * 0.8 && ready(Spell::AreaBlast) {
            input.selected = Some(Spell::AreaBlast);
            input.cast_pressed = true;
            input.cast_released = true;
            self.next_cast = elapsed + 42;
        } else if (visible || matches!(self.kind, ScriptKind::Peeker))
            && distance >= tuning.fireball_radius() + 0.8
            && ready(Spell::Fireball)
        {
            input.selected = Some(Spell::Fireball);
            input.cast_pressed = true;
            input.cast_held = true;
            self.armed_at = Some(elapsed);
        }
        (input, visible)
    }

    fn peeker_goal(
        &mut self,
        elapsed: u64,
        human: &Actor,
        ordinary_goal: Vec3,
        terrain: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
    ) -> (Vec3, bool) {
        if human.feet.y < self.anchor.y - 0.3 && !self.recovery.active {
            self.recovery.active = true;
            self.recovery.episodes += 1;
            self.next_recovery_plan = 0;
        }
        if !self.recovery.active {
            return (ordinary_goal, false);
        }
        if human.grounded && human.feet.y >= self.anchor.y - 0.1 {
            // Reattach the scripted route to ground reached by the real body.
            // Returning to a destroyed old anchor would immediately fall again.
            self.anchor = human.feet;
            self.recovery.active = false;
            self.recovery.completed += 1;
            self.recovery_goal = None;
            return (human.feet, false);
        }
        if human.grounded && elapsed >= self.next_recovery_plan {
            self.next_recovery_plan = elapsed + 60;
            let columns = HexCoord::from_world(human.feet).within_radius(2);
            self.recovery_goal = terrain
                .voxels
                .keys()
                .filter_map(|pos| {
                    if !columns.contains(&pos.coord) {
                        return None;
                    }
                    let top = geometry.top(*pos);
                    // A modest supported ledge, within the ordinary jump apex.
                    if top < human.feet.y - 0.1
                        || top > human.feet.y + 1.2
                        || top > self.anchor.y + 0.1
                    {
                        return None;
                    }
                    let goal = pos.coord.to_world(top);
                    let head = geometry
                        .voxel_at(goal + Vec3::Y * hex_arena::BODY_HEIGHT)?
                        .level;
                    if (pos.level + 1..=head)
                        .any(|level| terrain.voxels.contains_key(&TilePos::new(pos.coord, level)))
                    {
                        return None;
                    }
                    let progress = top - human.feet.y;
                    let distance = (goal - human.feet).with_y(0.0).length();
                    if distance < 0.25 {
                        return None;
                    }
                    let score = progress * 5.0
                        - distance * 0.15
                        - (goal - ordinary_goal).with_y(0.0).length() * 0.1;
                    Some((goal, score))
                })
                .max_by(|(_, a), (_, b)| a.total_cmp(b))
                .map(|(goal, _)| goal);
        }
        let jump = human.grounded && elapsed >= self.next_recovery_jump;
        if jump {
            self.next_recovery_jump = elapsed + 84;
            self.recovery.jump_requests += 1;
        }
        (self.recovery_goal.unwrap_or(ordinary_goal), jump)
    }
}

fn actor_visible(session: &ArenaSession, observer: &Actor, target: &Actor) -> bool {
    // A conservative public terrain sweep, including the camera's 0.1 radius,
    // keeps the fixture independent of private bot/collision helpers.
    [target.center(), target.eye()].into_iter().any(|point| {
        session
            .camera_position(observer.eye(), point)
            .distance(point)
            < 0.04
    })
}

fn script_visible_to_bot(session: &ArenaSession) -> bool {
    let human = session
        .actors
        .iter()
        .find(|actor| actor.id == 0)
        .expect("scripted actor");
    let bot = session
        .actors
        .iter()
        .find(|actor| actor.id == 1)
        .expect("opponent actor");
    actor_visible(session, bot, human)
}

#[derive(Default, Serialize)]
struct VisibilityCoverage {
    visible_ticks: u64,
    hidden_ticks: u64,
    reveals: u64,
    reconceals: u64,
    initial_visible: Option<bool>,
    last_visible: Option<bool>,
}

impl VisibilityCoverage {
    fn record(&mut self, visible: bool) {
        self.initial_visible.get_or_insert(visible);
        self.visible_ticks += u64::from(visible);
        self.hidden_ticks += u64::from(!visible);
        self.reveals += u64::from(self.last_visible == Some(false) && visible);
        self.reconceals += u64::from(self.last_visible == Some(true) && !visible);
        self.last_visible = Some(visible);
    }

    fn observed_peek_cycle(&self) -> bool {
        self.reveals > 0 && self.reconceals > 0
    }
}

// An intentionally modest opponent policy: three lead/gravity refinements, no
// private bot aiming or planner helpers and no counterfactual hit search.
fn scripted_aim(eye: Vec3, target: Vec3, velocity: Vec3, speed: f32, gravity: f32) -> Vec3 {
    let mut time = (eye.distance(target) / speed).min(1.5);
    let mut direction = (target - eye).normalize_or(Vec3::X);
    for _ in 0..3 {
        let point = target + velocity * time.min(0.6);
        direction = (point - eye + Vec3::Y * (0.5 * gravity * time * time)).normalize_or(direction);
        let horizontal_speed = (direction.with_y(0.0).length() * speed).max(0.1);
        time = ((point - eye).with_y(0.0).length() / horizontal_speed).min(1.5);
    }
    direction
}

#[derive(Serialize)]
struct ReleasedCharge {
    simulation_tick: u64,
    actor: usize,
    spell_slot: usize,
    held_seconds: f32,
}

#[derive(Serialize)]
struct SlowTick {
    simulation_tick: u64,
    cpu_ms: f64,
    terrain_revision_changed: bool,
    forecasts: u64,
    route_rollout_ticks: u64,
    movement_rollout_ticks: u64,
}

#[derive(Serialize)]
struct ReappearanceResponse {
    appeared_tick: u64,
    charge_seconds_at_reappearance: Option<f32>,
    release_tick: Option<u64>,
    release_spell_slot: Option<usize>,
    latency_seconds: Option<f64>,
    status: &'static str,
}

#[derive(Serialize)]
struct EvaluationRow {
    script: ScriptKind,
    scenario_seed: u16,
    baseline: bool,
    max_seconds: u32,
    setup_ticks: u64,
    simulation_ticks: u64,
    opponent_visible_ticks: u64,
    script_visibility_to_bot: VisibilityCoverage,
    peeker_cycle_observed: Option<bool>,
    ending_hp: [f32; 2],
    ending_feet: [[f32; 3]; 2],
    final_bot_debug: Option<BotDebugSnapshot>,
    peeker_recovery: Option<PeekerRecovery>,
    reappearance_responses: Vec<ReappearanceResponse>,
    bot_useful_fireball_rate: Option<f64>,
    final_voxels: usize,
    terrain_revisions: u64,
    terrain_publication_ms: f64,
    summary: RoundSummary,
    release_charges: Vec<ReleasedCharge>,
    cpu_total_ms: f64,
    cpu_tick_p50_ms: f64,
    cpu_tick_p95_ms: f64,
    cpu_tick_p99_ms: f64,
    cpu_tick_max_ms: f64,
    slowest_ticks: Vec<SlowTick>,
}

fn prepare(kind: ScriptKind, seed: u16, baseline: bool) -> (App, Opponent) {
    let mut app = app(120);
    app.world_mut().resource_mut::<ArenaTuning>().charge_seconds = CHARGE_SECONDS;
    assert!(app.world().resource::<ArenaTuning>().validate().is_ok());
    if !seed.is_multiple_of(2) {
        for actor in &mut app.world_mut().resource_mut::<ArenaSession>().actors {
            actor.feet.x = -actor.feet.x;
            actor.feet.z = -actor.feet.z;
            actor.previous_feet = actor.feet;
            actor.aim.x = -actor.aim.x;
            actor.aim.z = -actor.aim.z;
        }
    }
    let session = app.world().resource::<ArenaSession>();
    let human = session
        .actors
        .iter()
        .find(|actor| actor.id == 0)
        .expect("human spawn");
    let bot = session
        .actors
        .iter()
        .find(|actor| actor.id == 1)
        .expect("bot spawn");
    let script = Opponent::new(kind, seed, human, bot);
    if matches!(kind, ScriptKind::Peeker) {
        let geometry = *app.world().resource::<ArenaVoxelGeometry>();
        let ground = geometry
            .voxel_at(script.anchor)
            .expect("spawn surface")
            .level;
        let coord = HexCoord::from_world(script.anchor + script.forward * 2.1);
        let stone = app.world().resource::<ArenaMaterials>().stone;
        for level in ground + 1..=ground + 4 {
            app.world_mut().write_message(TerrainEdit::Set {
                pos: TilePos::new(coord, level),
                substance: stone,
            });
        }
        tick(&mut app); // Publication, collision, and material HP are world-owned.
        assert!(app
            .world()
            .resource::<ArenaTerrainView>()
            .voxels
            .contains_key(&TilePos::new(coord, ground + 4)));
        let session = app.world().resource::<ArenaSession>();
        assert!(
            !script_visible_to_bot(session),
            "peeker seed {seed} must begin hidden from the bot"
        );
        let human = session
            .actors
            .iter()
            .find(|actor| actor.id == 0)
            .expect("human");
        let bot = session
            .actors
            .iter()
            .find(|actor| actor.id == 1)
            .expect("bot");
        assert!(
            !actor_visible(session, human, bot),
            "peeker seed {seed} must begin behind opaque cover"
        );
    }
    app.world_mut()
        .resource_mut::<ArenaSession>()
        .use_baseline_bot(baseline);
    assert!(app.world().resource::<ArenaSession>().bot_enabled);
    (app, script)
}

fn settle_terrain(app: &mut App) -> f64 {
    // This App is discarded after reporting. Stop only combat authority, then
    // let the production world sets publish the last admitted tick's mutations.
    // This works for timeouts too, without inventing an outcome or another turn.
    app.configure_sets(ArenaTick, ArenaSystems::Simulate.run_if(|| false));
    let before = combat_snapshot(app);
    let start = Instant::now();
    tick(app);
    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    let after = combat_snapshot(app);
    assert_eq!(
        before.tick, after.tick,
        "terrain settlement cannot advance combat"
    );
    assert_eq!(before.actors, after.actors);
    assert_eq!(before.projectiles, after.projectiles);
    assert!(app.world().resource::<Messages<TerrainEdit>>().is_empty());
    assert!(app.world().resource::<Messages<TerrainImpact>>().is_empty());
    elapsed_ms
}

fn run_round(kind: ScriptKind, seed: u16, baseline: bool, max_seconds: u32) -> EvaluationRow {
    let (mut app, mut opponent) = prepare(kind, seed, baseline);
    let setup_ticks = app.world().resource::<ArenaSession>().tick;
    let initial_revision = app.world().resource::<ArenaTerrainView>().revision;
    let mut visible_ticks = 0;
    let mut script_visibility = VisibilityCoverage::default();
    let mut elapsed_ticks = 0;
    let mut cpu_ticks = Vec::new();
    let mut slowest_ticks: Vec<SlowTick> = Vec::new();
    let mut release_charges = Vec::new();
    let mut responses: Vec<ReappearanceResponse> = Vec::new();
    let mut pending_response: Option<usize> = None;
    for elapsed in 0..u64::from(max_seconds) * 120 {
        let session = app.world().resource::<ArenaSession>();
        if session.outcome.is_some() {
            break;
        }
        let debug_before = session.bot_debug();
        let revision_before = app.world().resource::<ArenaTerrainView>().revision;
        let start = Instant::now();
        let script_visible = script_visible_to_bot(session);
        if !script_visible {
            if let Some(index) = pending_response.take() {
                responses.get_mut(index).expect("open response").status =
                    "no_release_before_concealment";
            }
        } else if script_visibility.last_visible == Some(false) {
            let bot = session
                .actors
                .iter()
                .find(|actor| actor.id == 1)
                .expect("bot");
            let eligible = bot.hp > 0.0
                && (bot.charge().is_some()
                    || bot
                        .cooldowns
                        .get(Spell::Fireball.index())
                        .is_some_and(|cooldown| *cooldown <= hex_arena::STEP));
            if eligible {
                pending_response = Some(responses.len());
                responses.push(ReappearanceResponse {
                    appeared_tick: elapsed_ticks,
                    charge_seconds_at_reappearance: bot.charge().map(|charge| charge.elapsed),
                    release_tick: None,
                    release_spell_slot: None,
                    latency_seconds: None,
                    status: "pending",
                });
            }
        }
        script_visibility.record(script_visible);
        let before = session.round_summary();
        let charges = session
            .actors
            .iter()
            .map(|actor| (usize::from(actor.id), actor.charge()))
            .collect::<Vec<_>>();
        let (input, visible) = opponent.intent(
            elapsed,
            session,
            app.world().resource::<ArenaTuning>(),
            app.world().resource::<ArenaTerrainView>(),
            *app.world().resource::<ArenaVoxelGeometry>(),
        );
        visible_ticks += u64::from(visible);
        app.world_mut().resource_mut::<ArenaInput>().human = input;
        tick(&mut app);
        elapsed_ticks += 1;
        let cpu_ms = start.elapsed().as_secs_f64() * 1000.0;
        cpu_ticks.push(cpu_ms);
        let session = app.world().resource::<ArenaSession>();
        let debug_after = session.bot_debug();
        if slowest_ticks.len() < 3
            || slowest_ticks
                .last()
                .is_some_and(|tick| cpu_ms > tick.cpu_ms)
        {
            slowest_ticks.push(SlowTick {
                simulation_tick: elapsed_ticks,
                cpu_ms,
                terrain_revision_changed: app.world().resource::<ArenaTerrainView>().revision
                    != revision_before,
                forecasts: debug_after.forecasts - debug_before.forecasts,
                route_rollout_ticks: debug_after.route_rollout_ticks
                    - debug_before.route_rollout_ticks,
                movement_rollout_ticks: debug_after.movement_rollout_ticks
                    - debug_before.movement_rollout_ticks,
            });
            slowest_ticks.sort_by(|a, b| b.cpu_ms.total_cmp(&a.cpu_ms));
            slowest_ticks.truncate(3);
        }
        let after = session.round_summary();
        for (actor, (prior, current)) in before.actors.iter().zip(&after.actors).enumerate() {
            for (spell_slot, (old, new)) in prior.casts.iter().zip(&current.casts).enumerate() {
                assert!(*new >= *old && *new - *old <= 1);
                if new > old {
                    if actor == 1 {
                        if let Some(index) = pending_response.take() {
                            let response = responses.get_mut(index).expect("open response");
                            response.release_tick = Some(elapsed_ticks);
                            response.release_spell_slot = Some(spell_slot);
                            response.latency_seconds = Some(
                                f64::from(
                                    u32::try_from(elapsed_ticks - response.appeared_tick)
                                        .expect("bounded response time"),
                                ) / 120.0,
                            );
                            response.status = "released";
                        }
                    }
                    let held_seconds = charges
                        .iter()
                        .find(|(id, _)| *id == actor)
                        .and_then(|(_, charge)| *charge)
                        .map_or(0.0, |charge| charge.elapsed);
                    release_charges.push(ReleasedCharge {
                        simulation_tick: session.tick - setup_ticks,
                        actor,
                        spell_slot,
                        held_seconds,
                    });
                }
            }
        }
        assert!(session.actors.iter().all(|actor| actor.feet.is_finite()
            && actor.hp.is_finite()
            && (0.0..=100.0).contains(&actor.hp)));
    }
    assert!(!cpu_ticks.is_empty());
    let cpu_total_ms = cpu_ticks.iter().sum();
    cpu_ticks.sort_by(f64::total_cmp);
    let terrain_publication_ms = settle_terrain(&mut app);
    let session = app.world().resource::<ArenaSession>();
    if let Some(index) = pending_response {
        responses.get_mut(index).expect("open response").status = if session.outcome.is_some() {
            "censored_round_end"
        } else {
            "censored_timeout"
        };
    }
    let bot_stats = session
        .round_summary()
        .actors
        .get(1)
        .copied()
        .expect("bot stats");
    let hp = |id| {
        session
            .actors
            .iter()
            .find(|actor| actor.id == id)
            .expect("round actor")
            .hp
    };
    let feet = |id| {
        session
            .actors
            .iter()
            .find(|actor| actor.id == id)
            .expect("round actor")
            .feet
            .to_array()
    };
    let row = EvaluationRow {
        script: kind,
        scenario_seed: seed,
        baseline,
        max_seconds,
        setup_ticks,
        simulation_ticks: elapsed_ticks,
        opponent_visible_ticks: visible_ticks,
        peeker_cycle_observed: matches!(kind, ScriptKind::Peeker)
            .then(|| script_visibility.observed_peek_cycle()),
        script_visibility_to_bot: script_visibility,
        ending_hp: [hp(0), hp(1)],
        ending_feet: [feet(0), feet(1)],
        // The frozen baseline intentionally never runs the instrumented brain.
        final_bot_debug: (!baseline).then(|| session.bot_debug()),
        peeker_recovery: matches!(kind, ScriptKind::Peeker).then_some(opponent.recovery),
        reappearance_responses: responses,
        bot_useful_fireball_rate: (bot_stats.fireballs_resolved > 0).then(|| {
            f64::from(bot_stats.useful_fireballs) / f64::from(bot_stats.fireballs_resolved)
        }),
        final_voxels: app.world().resource::<ArenaTerrainView>().voxels.len(),
        terrain_revisions: app.world().resource::<ArenaTerrainView>().revision - initial_revision,
        terrain_publication_ms,
        summary: session.round_summary(),
        release_charges,
        cpu_total_ms,
        cpu_tick_p50_ms: *cpu_ticks.get(cpu_ticks.len() / 2).expect("median tick"),
        cpu_tick_p95_ms: *cpu_ticks.get(cpu_ticks.len() * 95 / 100).expect("p95 tick"),
        cpu_tick_p99_ms: *cpu_ticks.get(cpu_ticks.len() * 99 / 100).expect("p99 tick"),
        cpu_tick_max_ms: *cpu_ticks.last().expect("slowest tick"),
        slowest_ticks,
    };
    assert_eq!(
        row.summary
            .actors
            .iter()
            .map(|stats| stats.casts.iter().sum::<u32>())
            .sum::<u32>(),
        u32::try_from(row.release_charges.len()).expect("bounded cast count")
    );
    for response in &row.reappearance_responses {
        assert_ne!(response.status, "pending");
        if let Some(release_tick) = response.release_tick {
            assert!(row.release_charges.iter().any(|release| release.actor == 1
                && release.simulation_tick == release_tick
                && Some(release.spell_slot) == response.release_spell_slot));
            assert!(response
                .latency_seconds
                .is_some_and(|latency| latency > 0.0));
        }
    }
    if matches!(kind, ScriptKind::StationaryTarget) {
        assert!(
            row.summary
                .actors
                .get(1)
                .expect("bot stats")
                .casts
                .iter()
                .sum::<u32>()
                > 0,
            "an enabled brain must release a spell at the open stationary target"
        );
    }
    row
}

fn aggregate(rows: &[EvaluationRow], baseline: bool) -> serde_json::Value {
    let selected = rows
        .iter()
        .filter(|row| row.baseline == baseline)
        .collect::<Vec<_>>();
    let bot_damage: f32 = selected
        .iter()
        .map(|row| row.summary.actors.get(1).expect("bot stats").damage_dealt)
        .sum();
    let self_damage: f32 = selected
        .iter()
        .map(|row| row.summary.actors.get(1).expect("bot stats").self_damage)
        .sum();
    let hp_margin: f32 = selected
        .iter()
        .map(|row| row.ending_hp.get(1).expect("bot HP") - row.ending_hp.first().expect("human HP"))
        .sum();
    let resolved: u32 = selected
        .iter()
        .map(|row| {
            row.summary
                .actors
                .get(1)
                .expect("bot stats")
                .fireballs_resolved
        })
        .sum();
    let useful: u32 = selected
        .iter()
        .map(|row| {
            row.summary
                .actors
                .get(1)
                .expect("bot stats")
                .useful_fireballs
        })
        .sum();
    let responses = selected
        .iter()
        .flat_map(|row| &row.reappearance_responses)
        .collect::<Vec<_>>();
    let mut latencies = responses
        .iter()
        .filter_map(|response| response.latency_seconds)
        .collect::<Vec<_>>();
    latencies.sort_by(f64::total_cmp);
    serde_json::json!({
        "rounds": selected.len(),
        "bot_wins": selected.iter().filter(|row| row.summary.winner == Some(1)).count(),
        "script_wins": selected.iter().filter(|row| row.summary.winner == Some(0)).count(),
        "draws": selected.iter().filter(|row| row.summary.complete && row.summary.winner.is_none()).count(),
        "timeouts": selected.iter().filter(|row| !row.summary.complete).count(),
        "bot_damage_dealt": bot_damage,
        "bot_self_damage": self_damage,
        "sum_bot_minus_script_remaining_hp": hp_margin,
        "simulated_ticks": selected.iter().map(|row| row.simulation_ticks).sum::<u64>(),
        "cpu_total_ms": selected.iter().map(|row| row.cpu_total_ms).sum::<f64>(),
        "terrain_publication_ms": selected.iter().map(|row| row.terrain_publication_ms).sum::<f64>(),
        "resolved_fireballs": resolved,
        "useful_fireballs": useful,
        "useful_resolved_fireball_rate": (resolved > 0).then(|| f64::from(useful) / f64::from(resolved)),
        "reappearance_episodes": responses.len(),
        "reappearance_releases": latencies.len(),
        "reappearance_no_response": responses.iter().filter(|response| response.status == "no_release_before_concealment").count(),
        "reappearance_censored": responses.iter().filter(|response| response.status.starts_with("censored_")).count(),
        "reappearance_latency_p50_seconds": latencies.get(latencies.len() / 2),
        "reappearance_latency_p95_seconds": latencies.get(latencies.len() * 95 / 100),
    })
}

fn paired_report(max_seconds: u32) {
    let start = Instant::now();
    let mut rows = Vec::new();
    for kind in [
        ScriptKind::StationaryTarget,
        ScriptKind::Peeker,
        ScriptKind::Rusher,
        ScriptKind::Strafer,
    ] {
        for seed in 0_u16..3 {
            // Alternate execution order to avoid always favoring the second
            // implementation with warmed allocator/cache state.
            for baseline in if seed.is_multiple_of(2) {
                [true, false]
            } else {
                [false, true]
            } {
                rows.push(run_round(kind, seed, baseline, max_seconds));
            }
        }
    }
    assert_eq!(rows.len(), 24);
    let pairs = rows.chunks_exact(2).map(|pair| {
        let baseline = pair.iter().find(|row| row.baseline).expect("paired baseline");
        let candidate = pair.iter().find(|row| !row.baseline).expect("paired candidate");
        assert_eq!(baseline.script, candidate.script);
        assert_eq!(baseline.scenario_seed, candidate.scenario_seed);
        let margin = |row: &EvaluationRow| row.ending_hp.get(1).expect("bot HP") - row.ending_hp.first().expect("human HP");
        serde_json::json!({
            "script": baseline.script,
            "scenario_seed": baseline.scenario_seed,
            "candidate_minus_baseline_hp_margin": margin(candidate) - margin(baseline),
            "candidate_minus_baseline_bot_damage": candidate.summary.actors.get(1).expect("candidate stats").damage_dealt - baseline.summary.actors.get(1).expect("baseline stats").damage_dealt,
        })
    }).collect::<Vec<_>>();
    let report = serde_json::json!({
        "schema": "arena-paired-bot-v1",
        "baseline_source_head": BASELINE_SOURCE_HEAD,
        "charge_seconds": CHARGE_SECONDS,
        "baseline_charge_policy": "unchanged reference-charge policy; actual release holds recorded",
        "scenario_seed_meaning": "opponent route/phase and mirrored side; both brains retain their default RNG seed",
        "method": "real authored arena, world-owned edits/HP/destruction, 120Hz ArenaTick, ActorIntent scripts, no renderer",
        "human_win_rate_evidence": "NOT_ESTABLISHED: scripted policies are not beginner/trained human cohorts",
        "performance_scope": "cpu_total_ms and tick percentiles time input policy, visibility sampling, and the full fixed tick; they exclude fixture setup, diagnostic counter snapshots, post-tick assertions, final terrain publication, and rendering. terrain_publication_ms times the final world-only pass. wall_seconds includes fixture setup, diagnostics, assertions, reporting preparation, and publication for all rounds, but not JSON serialization or log output",
        "diagnostic_scope": "slowest_ticks retains three measured ticks with per-tick counter deltas; baseline counters remain zero and final_bot_debug is null because the new brain does not run in baseline rounds",
        "terrain_scope": "final_voxels and terrain_revisions include all queued edits/impacts from measured ticks, published once with Simulate disabled; future projectile flight and pending wall emergence are not advanced",
        "peeker_adequacy": "initial cover is asserted and movement is checked separately with combat disabled; row coverage records conservative public terrain sweeps, not the bot's sensing cadence. A missing cycle in a short KO or destroyed-cover round is reported, not a test failure",
        "peeker_recovery": "both brains face the same script: below its prior surface, seek a nearby supported ledge and request ordinary grounded jumps, then re-anchor only on physically reaching that height. Deep vertical craters can remain unrecoverable; active/episodes/completed/jump_requests are reported. Earlier no-recovery stress results remain in arena-strong-bot-peeker-diagnostic-01",
        "response_scope": "public terrain-sweep false-to-true visibility, with bot alive and charging or Fireball cooldown ready. Latency is measured to the next actual spell release, in120Hz tick intervals. Concealment before release is no response; round end/timeout while exposed is censored. This approximation does not read the brain's private reaction gate",
        "useful_shot_scope": "actual resolved Fireballs whose opponent capsule falloff is at least40% of maximum uncapped damage, divided by resolved Fireballs; unresolved in-flight projectiles are excluded",
        "wall_seconds": start.elapsed().as_secs_f64(),
        "baseline": aggregate(&rows, true),
        "candidate": aggregate(&rows, false),
        "paired_deltas": pairs,
        "rows": rows,
    });
    eprintln!("ARENA_BOT_EVALUATION {report}");
}

#[test]
fn terrain_settlement_publishes_queued_edits_without_advancing_live_combat() {
    let (mut app, _) = prepare(ScriptKind::StationaryTarget, 0, false);
    app.world_mut().resource_mut::<ArenaInput>().human = ActorIntent {
        selected: Some(Spell::AreaBlast),
        cast_pressed: true,
        cast_released: true,
        ..default()
    };
    tick(&mut app);
    assert!(
        !app.world().resource::<Messages<TerrainImpact>>().is_empty(),
        "the measured tick must leave a real explosion queued for the world"
    );
    let pos = TilePos::new(HexCoord::ORIGIN, 16);
    assert!(!app
        .world()
        .resource::<ArenaTerrainView>()
        .voxels
        .contains_key(&pos));
    let stone = app.world().resource::<ArenaMaterials>().stone;
    app.world_mut().write_message(TerrainEdit::Set {
        pos,
        substance: stone,
    });
    assert!(app.world().resource::<ArenaSession>().outcome.is_none());
    let before = serde_json::to_value(app.world().resource::<ArenaSession>().round_summary())
        .expect("summary");
    let _ = settle_terrain(&mut app);
    assert_eq!(
        app.world().resource::<ArenaTerrainView>().voxels.get(&pos),
        Some(&stone)
    );
    assert_eq!(
        before,
        serde_json::to_value(app.world().resource::<ArenaSession>().round_summary())
            .expect("summary")
    );
}

#[test]
fn peeker_fixture_exposes_and_reconceals_with_combat_disabled() {
    for seed in 0_u16..3 {
        let (mut app, mut opponent) = prepare(ScriptKind::Peeker, seed, false);
        app.world_mut().resource_mut::<ArenaSession>().bot_enabled = false;
        let mut coverage = VisibilityCoverage::default();
        for elapsed in 0..720 {
            let session = app.world().resource::<ArenaSession>();
            coverage.record(script_visible_to_bot(session));
            let (mut input, _) = opponent.intent(
                elapsed,
                session,
                app.world().resource::<ArenaTuning>(),
                app.world().resource::<ArenaTerrainView>(),
                *app.world().resource::<ArenaVoxelGeometry>(),
            );
            input.cast_pressed = false;
            input.cast_released = false;
            input.cast_held = false;
            input.selected = None;
            app.world_mut().resource_mut::<ArenaInput>().human = input;
            tick(&mut app);
        }
        assert_eq!(coverage.initial_visible, Some(false));
        assert!(
            coverage.reveals >= 2 && coverage.reconceals >= 2,
            "peeker seed {seed} must repeatedly expose and return to cover"
        );
        assert!(app.world().resource::<ArenaSession>().outcome.is_none());
        assert!(app
            .world()
            .resource::<ArenaSession>()
            .round_summary()
            .actors
            .iter()
            .all(|stats| stats.casts == [0; 3]));
    }
}

#[test]
fn peeker_recovers_from_reachable_crater_and_resumes_releasing() {
    let (mut app, mut opponent) = prepare(ScriptKind::Peeker, 0, false);
    app.world_mut().resource_mut::<ArenaSession>().bot_enabled = false;
    let original_height = opponent.anchor.y;
    let coord = HexCoord::from_world(opponent.anchor);
    let geometry = *app.world().resource::<ArenaVoxelGeometry>();
    let ground = geometry
        .voxel_at(opponent.anchor - Vec3::Y * 0.01)
        .expect("ground")
        .level;
    // Remove three real ground layers; the surrounding ledge is within the
    // ordinary jump apex. No actor position or movement state is assigned.
    for level in ground - 2..=ground {
        app.world_mut().write_message(TerrainEdit::Clear {
            pos: TilePos::new(coord, level),
        });
    }
    let mut saw_crater_floor = false;
    let mut saw_recovered_visibility = false;
    for elapsed in 0..1200 {
        let session = app.world().resource::<ArenaSession>();
        let human = session
            .actors
            .iter()
            .find(|actor| actor.id == 0)
            .expect("human");
        saw_crater_floor |= human.feet.y < original_height - 0.8;
        saw_recovered_visibility |=
            opponent.recovery.completed > 0 && script_visible_to_bot(session);
        let (mut input, _) = opponent.intent(
            elapsed,
            session,
            app.world().resource::<ArenaTuning>(),
            app.world().resource::<ArenaTerrainView>(),
            geometry,
        );
        // First verify the walk/jump recovery independently of fresh destruction.
        if opponent.recovery.completed == 0 {
            input.cast_pressed = false;
            input.cast_released = false;
            input.cast_held = false;
            input.selected = None;
        }
        app.world_mut().resource_mut::<ArenaInput>().human = input;
        tick(&mut app);
        let session = app.world().resource::<ArenaSession>();
        if saw_recovered_visibility
            && session
                .round_summary()
                .actors
                .first()
                .expect("human stats")
                .casts
                .get(1)
                .is_some_and(|casts| *casts > 0)
        {
            break;
        }
    }
    assert!(saw_crater_floor);
    assert!(opponent.recovery.jump_requests > 0 && opponent.recovery.completed > 0);
    assert!(saw_recovered_visibility);
    assert!(app
        .world()
        .resource::<ArenaSession>()
        .round_summary()
        .actors
        .first()
        .expect("human stats")
        .casts
        .get(1)
        .is_some_and(|casts| *casts > 0));
}

#[test]
fn paired_scripted_bot_evaluation() {
    paired_report(10);
}

#[test]
#[ignore = "Explicit longer paired report: 24 rounds, up to 45 seconds each"]
fn paired_scripted_bot_evaluation_long() {
    paired_report(45);
}

#[test]
#[ignore = "Focused Peeker diagnostics: six paired rounds, up to 45 seconds each"]
fn paired_peeker_diagnostic() {
    let mut rows = Vec::new();
    for seed in 0_u16..3 {
        for baseline in [true, false] {
            rows.push(run_round(ScriptKind::Peeker, seed, baseline, 45));
        }
    }
    let report = serde_json::json!({
        "schema": "arena-peeker-diagnostic-v1",
        "baseline_source_head": BASELINE_SOURCE_HEAD,
        "charge_seconds": CHARGE_SECONDS,
        "method": "same real-world paired evaluator, Peeker only, 45 second limit",
        "slow_tick_counter_scope": "per-tick delta, baseline counters zero because the new brain does not run",
        "rows": rows,
    });
    eprintln!("ARENA_PEEKER_DIAGNOSTIC {report}");
}
