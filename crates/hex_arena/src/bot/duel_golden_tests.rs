//! Frozen before spectator targeting, from accepted Duel runtime at 27338de.
//!
//! Reuses the existing stationary, sprinting, and covered-charge unit fixtures.
//! Terrain is their fixed published view: emitted world requests are counted but
//! no replacement map HP/damage consumer is invented here. Real-world settlement
//! remains the responsibility of the existing application integration tests.
//! Integers quantize physical values to 0.001 units and seconds to milliseconds.
//! There is deliberately no environment flag to regenerate expected results.

use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy)]
enum Scenario {
    Stationary,
    Sprinting,
    Covered,
}

impl Scenario {
    fn name(self) -> &'static str {
        match self {
            Self::Stationary => "stationary_default_seed",
            Self::Sprinting => "sprinting_seed_10203040",
            Self::Covered => "covered_charge_seed_a53c9e21",
        }
    }

    fn fixture(self) -> Fixture {
        let mut fixture = Fixture::new(match self {
            Self::Stationary => 14.0,
            Self::Sprinting => 12.0,
            Self::Covered => 28.0,
        });
        match self {
            Self::Stationary => fixture.session.bot = Bot::default(),
            Self::Sprinting => {
                fixture.session.bot.seed = 0x1020_3040;
                fixture.actor_mut(1).cooldowns = [600.0; 3];
            }
            Self::Covered => {
                fixture.session.bot.seed = 0xA53C_9E21;
                fixture.tuning.bot.blind_fire_enabled = false;
                fixture.tuning.bot.flank_enabled = false;
                fixture.tuning.bot.ambush_chance = 0.0;
            }
        }
        fixture
    }

    fn before_tick(self, fixture: &mut Fixture, step: u64) -> ActorIntent {
        match self {
            Self::Sprinting => {
                if step == 15 {
                    fixture.actor_mut(1).cooldowns = [0.0; 3];
                    fixture.session.bot.think_ticks = 0;
                }
                ActorIntent {
                    aim: Vec3::X,
                    movement: Vec2::X,
                    run: true,
                    ..Default::default()
                }
            }
            Self::Covered => {
                if step == 2 {
                    for coord in HexCoord::ORIGIN.within_radius(2) {
                        for level in 1..=8 {
                            fixture
                                .world
                                .voxels
                                .insert(TilePos::new(coord, level), fixture.materials.stone);
                        }
                    }
                    fixture.refresh();
                } else if step == 102 {
                    fixture.world.voxels.retain(|pos, _| pos.level == 0);
                    fixture.refresh();
                }
                ActorIntent::default()
            }
            Self::Stationary => ActorIntent::default(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct FrozenActor {
    id: u8,
    feet_milli: [i32; 3],
    previous_feet_milli: [i32; 3],
    aim_milli: [i32; 3],
    dimensions_milli: [i32; 3],
    hp_milli: i32,
    grounded: bool,
    impulse_milli: [i32; 3],
    vertical_velocity_milli: i32,
    selected: String,
    cooldown_ms: [i32; 3],
    charge: Option<(String, i32)>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct FrozenShot {
    id: u64,
    owner: u8,
    spell: String,
    position_milli: [i32; 3],
    previous_position_milli: [i32; 3],
    velocity_milli: [i32; 3],
    age_ms: i32,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct FrozenStats {
    casts: [u32; 3],
    damage_dealt_milli: i32,
    damage_received_milli: i32,
    self_damage_milli: i32,
    resolved_fireballs: u32,
    useful_fireballs: u32,
    first_damage_tick: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Snapshot {
    input_step: u64,
    tick: u64,
    actors: Vec<FrozenActor>,
    shots: Vec<FrozenShot>,
    winner: Option<u8>,
    complete: bool,
    stats: [FrozenStats; 2],
    bot_rng: u32,
    bot_mode: String,
    last_release_reason: String,
    planned_charge_ms: i32,
    memory_milli: Option<[i32; 3]>,
    memory_age_ms: Option<i32>,
    terrain_impacts: usize,
    terrain_impact_voxels: usize,
    terrain_edits: usize,
    shields_raised: u64,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct FrozenRun {
    name: String,
    initial_seed: u32,
    snapshots: Vec<Snapshot>,
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "short bounded arena runs quantize finite physical values to millimetres"
)]
fn quantized(value: f32) -> i32 {
    assert!(value.is_finite() && value.abs() < 1_000_000.0);
    (value * 1000.0).round() as i32
}

fn vector(value: Vec3) -> [i32; 3] {
    value.to_array().map(quantized)
}

fn freeze_actor(actor: &Actor) -> FrozenActor {
    FrozenActor {
        id: actor.id,
        feet_milli: vector(actor.feet),
        previous_feet_milli: vector(actor.previous_feet),
        aim_milli: vector(actor.aim),
        dimensions_milli: vector(actor.body_dimensions()),
        hp_milli: quantized(actor.hp),
        grounded: actor.grounded,
        impulse_milli: vector(actor.body.impulse_velocity),
        vertical_velocity_milli: quantized(actor.body.vertical_velocity),
        selected: actor.selected.name().into(),
        cooldown_ms: actor.cooldowns.map(quantized),
        charge: actor
            .charge()
            .map(|charge| (charge.spell.name().into(), quantized(charge.elapsed))),
    }
}

fn freeze_shot(shot: &Projectile) -> FrozenShot {
    FrozenShot {
        id: shot.id,
        owner: shot.owner,
        spell: shot.spell.name().into(),
        position_milli: vector(shot.position),
        previous_position_milli: vector(shot.previous_position),
        velocity_milli: vector(shot.velocity),
        age_ms: quantized(shot.age),
    }
}

fn freeze_stats(stats: crate::ActorCombatStats) -> FrozenStats {
    FrozenStats {
        casts: stats.casts,
        damage_dealt_milli: quantized(stats.damage_dealt),
        damage_received_milli: quantized(stats.damage_received),
        self_damage_milli: quantized(stats.self_damage),
        resolved_fireballs: stats.fireballs_resolved,
        useful_fireballs: stats.useful_fireballs,
        first_damage_tick: stats.first_damage_tick,
    }
}

fn run(scenario: Scenario) -> FrozenRun {
    let mut fixture = scenario.fixture();
    let initial_seed = fixture.session.bot.seed;
    let mut snapshots = Vec::new();
    let mut terrain_impacts = 0;
    let mut terrain_impact_voxels = 0;
    let mut terrain_edits = 0;
    for input_step in 1..=1800 {
        let intent = scenario.before_tick(&mut fixture, input_step);
        let commands = fixture.session.advance(
            intent,
            &fixture.world,
            fixture.geometry,
            fixture.materials,
            &fixture.tuning,
        );
        terrain_impacts += commands.impacts.len();
        terrain_impact_voxels += commands
            .impacts
            .iter()
            .map(|i| i.volume.len())
            .sum::<usize>();
        terrain_edits += commands.edits.len();
        if [1, 13, 90, 113, 240, 720, 1800].contains(&input_step) {
            let session = &fixture.session;
            let summary = session.round_summary();
            let debug = session.bot_debug();
            snapshots.push(Snapshot {
                input_step,
                tick: session.tick,
                actors: session.actors.iter().map(freeze_actor).collect(),
                shots: session.projectiles.iter().map(freeze_shot).collect(),
                winner: summary.winner,
                complete: summary.complete,
                stats: summary.actors.map(freeze_stats),
                bot_rng: session.bot.seed,
                bot_mode: debug.mode.into(),
                last_release_reason: debug.last_release_reason.into(),
                planned_charge_ms: quantized(debug.planned_charge),
                memory_milli: debug.last_seen.map(|point| point.map(quantized)),
                memory_age_ms: debug.memory_age.map(quantized),
                terrain_impacts,
                terrain_impact_voxels,
                terrain_edits,
                shields_raised: session.shields_raised,
            });
        }
    }
    FrozenRun {
        name: scenario.name().into(),
        initial_seed,
        snapshots,
    }
}

fn verify(scenario: Scenario) {
    let expected: Vec<FrozenRun> =
        ron::from_str(include_str!("duel_goldens.ron")).expect("frozen Duel fixtures parse");
    let accepted = expected
        .into_iter()
        .find(|expected| expected.name == scenario.name())
        .expect("one frozen run per scenario");
    let actual = run(scenario);
    assert_eq!(actual, accepted, "accepted Duel behavior changed");
}

#[test]
fn accepted_duel_stationary_default_seed_matches_frozen_runtime() {
    verify(Scenario::Stationary);
}

#[test]
fn accepted_duel_sprinting_target_matches_frozen_runtime() {
    verify(Scenario::Sprinting);
}

#[test]
fn accepted_duel_covered_charge_matches_frozen_runtime() {
    verify(Scenario::Covered);
}
