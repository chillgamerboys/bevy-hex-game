//! Actual world/120 Hz arena composition and explicit local calibration.
#![expect(
    clippy::expect_used,
    reason = "invalid deterministic fixtures fail the contract"
)]

use bevy::prelude::*;
use hex_arena::{
    ArenaBattleSetup, ArenaSession, ArenaTuning, BattlePreset, BattleResult, BattleSummary,
};
use hex_core::arena::{
    ArenaMap, ArenaReset, ArenaSelection, ArenaTerrainView, ArenaTick, ArenaVoxelGeometry,
};
use hex_core::TerrainEdit;
use std::time::Instant;

fn app(map: ArenaMap, setup: ArenaBattleSetup) -> App {
    configured_app(map, setup, authored_tuning())
}

fn authored_tuning() -> ArenaTuning {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/config/arena.ron");
    let source = std::fs::read_to_string(path).expect("same arena configuration as the native app");
    let tuning: ArenaTuning = ron::from_str(&source).expect("valid authored arena configuration");
    tuning.validate().expect("admitted authored tuning");
    tuning
}

fn configured_app(map: ArenaMap, setup: ArenaBattleSetup, tuning: ArenaTuning) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(tuning)
        .insert_resource(ArenaSelection {
            map,
            ..Default::default()
        })
        .insert_resource(setup)
        .add_plugins((hex_map::arena::plugin, hex_arena::plugin));
    app.update();
    app.world_mut().run_schedule(ArenaTick);
    app
}

fn battle(app: &App) -> BattleSummary {
    app.world()
        .resource::<ArenaSession>()
        .battle_summary()
        .expect("accepted spectator setup")
}

/// Publish edits from the terminal combat tick without simulating another living tick.
/// Its full ArenaTick CPU cost is separate from ordinary combat-tick distributions.
fn flush_terminal_publication(app: &mut App) -> serde_json::Value {
    let before = app.world().resource::<ArenaSession>();
    assert!(before.is_finished(), "only terminal battles may be flushed");
    let tick_before = before.tick;
    let summary_before = serde_json::to_value(battle(app)).expect("terminal summary");
    let hp_before: Vec<_> = before
        .actors
        .iter()
        .map(|actor| (actor.id, actor.hp))
        .collect();
    let revision_before = app.world().resource::<ArenaTerrainView>().revision;
    let began = Instant::now();
    app.world_mut().run_schedule(ArenaTick);
    let cpu_ms = began.elapsed().as_secs_f64() * 1000.0;
    let after = app.world().resource::<ArenaSession>();
    let revision_after = app.world().resource::<ArenaTerrainView>().revision;
    assert_eq!(
        after.tick, tick_before,
        "terminal publication advanced simulation"
    );
    assert_eq!(
        serde_json::to_value(battle(app)).expect("terminal summary"),
        summary_before
    );
    assert_eq!(
        after
            .actors
            .iter()
            .map(|actor| (actor.id, actor.hp))
            .collect::<Vec<_>>(),
        hp_before
    );
    serde_json::json!({
        "cpu_ms":cpu_ms,"revision_before":revision_before,"revision_after":revision_after,
        "terrain_published":revision_after!=revision_before,"tick_before":tick_before,
        "tick_after":after.tick,"battle_state_unchanged":true,
        "measurement":"One terminal ArenaTick, including any queued terrain publication; excluded from living combat-tick distributions."
    })
}

#[test]
fn terminal_publication_applies_queued_terrain_without_changing_outcome_tick_or_hp() {
    let mut setup = ArenaBattleSetup::spectator(BattlePreset::Shadow, BattlePreset::Dragon, 3);
    setup.tick_limit = Some(1);
    let mut fixture = app(ArenaMap::Duel, setup);
    assert_eq!(battle(&fixture).result, Some(BattleResult::Timeout));
    let removable = fixture
        .world()
        .resource::<ArenaTerrainView>()
        .voxels
        .keys()
        .find(|voxel| voxel.level > 0)
        .copied()
        .expect("destructible Duel terrain");
    fixture
        .world_mut()
        .write_message(TerrainEdit::Clear { pos: removable });
    let receipt = flush_terminal_publication(&mut fixture);
    assert_eq!(
        receipt
            .get("terrain_published")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert!(!fixture
        .world()
        .resource::<ArenaTerrainView>()
        .voxels
        .contains_key(&removable));
    // A second flush has no queued publication and still cannot advance the match.
    let quiet = flush_terminal_publication(&mut fixture);
    assert_eq!(
        quiet
            .get("terrain_published")
            .and_then(serde_json::Value::as_bool),
        Some(false)
    );
}

#[test]
fn original_rosters_admit_on_published_deployment_without_a_human_body() {
    for map in [ArenaMap::Duel, ArenaMap::Fort] {
        for left in BattlePreset::ALL {
            for right in BattlePreset::ALL {
                let fixture = app(map, ArenaBattleSetup::spectator(left, right, 5));
                let session = fixture.world().resource::<ArenaSession>();
                let world = fixture.world().resource::<ArenaTerrainView>();
                let geometry = *fixture.world().resource::<ArenaVoxelGeometry>();
                assert!(world.battle_deployment.is_some());
                assert_eq!(session.human_actor_id(), None);
                assert!(
                    battle(&fixture).result.is_none(),
                    "{map:?} {left:?}/{right:?} {:?}",
                    battle(&fixture).result
                );
                assert_eq!(
                    session.actors.len(),
                    left.members().len() + right.members().len()
                );
                for actor in &session.actors {
                    assert!(
                        session.actor_pose_valid(actor.id, world, geometry),
                        "{map:?} {left:?}/{right:?} actor{} {:?}",
                        actor.id,
                        actor.feet
                    );
                }
            }
        }
    }
}

#[test]
fn spectator_setup_edits_wait_for_reset_then_return_to_ordinary_duel() {
    let mut fixture = app(
        ArenaMap::Duel,
        ArenaBattleSetup::spectator(BattlePreset::Shadow, BattlePreset::Dragon, 7),
    );
    *fixture.world_mut().resource_mut::<ArenaBattleSetup>() =
        ArenaBattleSetup::spectator(BattlePreset::Goblins, BattlePreset::Goblins, 9);
    fixture.world_mut().run_schedule(ArenaTick);
    assert_eq!(battle(&fixture).seed, 7);
    fixture.world_mut().resource_mut::<ArenaReset>().generation += 1;
    fixture.world_mut().run_schedule(ArenaTick);
    assert_eq!(battle(&fixture).seed, 9);
    assert_eq!(fixture.world().resource::<ArenaSession>().actors.len(), 10);
    *fixture.world_mut().resource_mut::<ArenaBattleSetup>() = ArenaBattleSetup::default();
    fixture.world_mut().resource_mut::<ArenaReset>().generation += 1;
    fixture.world_mut().run_schedule(ArenaTick);
    let session = fixture.world().resource::<ArenaSession>();
    assert_eq!(session.human_actor_id(), Some(0));
    assert!(session.battle_summary().is_none());
    assert_eq!(session.actors.len(), 2);
}

fn distribution(values: &[f64]) -> serde_json::Value {
    let mut values = values.to_vec();
    values.sort_by(f64::total_cmp);
    let percentile = |percent: usize| {
        values
            .get(values.len().saturating_sub(1) * percent / 100)
            .copied()
            .unwrap_or(0.0)
    };
    serde_json::json!({"samples":values.len(),"p50_ms":percentile(50),"p95_ms":percentile(95),"p99_ms":percentile(99),"max_ms":percentile(100),"over_tick_budget":values.iter().filter(|x|**x>1000.0/120.0).count()})
}

/// Seed/recipe choices are external test inputs, never adaptive gameplay settings.
#[test]
#[ignore = "explicit native-profile seeded monster calibration; prints local per-round receipts"]
fn calibrate_original_monster_groups() {
    let seed_count = std::env::var("HEX_BATTLE_SEED_COUNT")
        .ok()
        .map_or(8, |v| v.parse::<u64>().expect("positive seed count"));
    assert!((1..=1000).contains(&seed_count));
    let first_seed = std::env::var("HEX_BATTLE_FIRST_SEED")
        .ok()
        .map_or(1, |v| v.parse::<u64>().expect("first seed"));
    let seconds = std::env::var("HEX_BATTLE_SECONDS")
        .ok()
        .map_or(90, |v| v.parse::<u64>().expect("seconds"));
    assert!((1..=300).contains(&seconds));
    let map_name = std::env::var("HEX_BATTLE_MAP").unwrap_or_else(|_| "duel".into());
    let map = [("duel", ArenaMap::Duel), ("fort", ArenaMap::Fort)]
        .into_iter()
        .find_map(|(name, map)| (name == map_name).then_some(map))
        .expect("battle map must be duel or fort");
    let selected = std::env::var("HEX_BATTLE_MATCHUPS").ok();
    let matchups = selected.map_or_else(
        || {
            vec![
                (BattlePreset::Shadow, BattlePreset::Dragon),
                (BattlePreset::Shadow, BattlePreset::Goblins),
                (BattlePreset::Shadow, BattlePreset::ShamanParty),
                (BattlePreset::Dragon, BattlePreset::Goblins),
                (BattlePreset::Dragon, BattlePreset::ShamanParty),
                (BattlePreset::Goblins, BattlePreset::ShamanParty),
            ]
        },
        |text| {
            text.split(',')
                .map(|pair| {
                    let (a, b) = pair.split_once(':').expect("matchup left:right");
                    (
                        BattlePreset::from_slug(a).expect("left preset"),
                        BattlePreset::from_slug(b).expect("right preset"),
                    )
                })
                .collect()
        },
    );
    let trace = std::env::var("HEX_BATTLE_TRACE").is_ok_and(|value| value == "1");
    let tuning = authored_tuning();
    println!(
        "ARENA_TUNING {}",
        serde_json::json!({"tuning":tuning,"matches_defaults":serde_json::to_value(&tuning).expect("serialize authored tuning") == serde_json::to_value(ArenaTuning::default()).expect("serialize defaults")})
    );
    for seed_offset in 0..seed_count {
        let seed = first_seed.checked_add(seed_offset).expect("seed bound");
        for &(first, second) in &matchups {
            for swap in [false, true] {
                let (left, right) = if swap {
                    (second, first)
                } else {
                    (first, second)
                };
                let mut setup = ArenaBattleSetup::spectator(left, right, seed);
                setup.tick_limit = Some(seconds * 120);
                let began = Instant::now();
                let mut fixture = configured_app(map, setup.clone(), tuning.clone());
                let setup_ms = began.elapsed().as_secs_f64() * 1000.0;
                let mut timings = Vec::new();
                let mut publications = Vec::new();
                let mut previous_self_damage = 0.0;
                loop {
                    if fixture.world().resource::<ArenaSession>().is_finished() {
                        break;
                    }
                    let revision = fixture.world().resource::<ArenaTerrainView>().revision;
                    let before = trace.then(|| {
                        let session = fixture.world().resource::<ArenaSession>();
                        serde_json::json!({
                            "actors":session.actors.iter().map(|actor|serde_json::json!({
                                "id":actor.id,"hp":actor.hp,"feet":actor.feet.to_array(),"eye":actor.eye().to_array(),
                                "aim":actor.aim.to_array(),"impulse":actor.impulse_velocity().to_array()
                            })).collect::<Vec<_>>(),
                            "projectiles":session.projectiles.iter().map(|shot|serde_json::json!({
                                "id":shot.id,"owner":shot.owner,"position":shot.position.to_array(),"velocity":shot.velocity.to_array(),"age":shot.age
                            })).collect::<Vec<_>>()
                        })
                    });
                    let began = Instant::now();
                    fixture.world_mut().run_schedule(ArenaTick);
                    let ms = began.elapsed().as_secs_f64() * 1000.0;
                    timings.push(ms);
                    if fixture.world().resource::<ArenaTerrainView>().revision != revision {
                        publications.push(ms);
                    }
                    let session = fixture.world().resource::<ArenaSession>();
                    let self_damage: f32 = if trace {
                        session
                            .encounter_stats()
                            .iter()
                            .map(|stats| stats.combat.self_damage)
                            .sum()
                    } else {
                        0.0
                    };
                    let self_hit = self_damage > previous_self_damage + 0.001;
                    previous_self_damage = self_damage;
                    let release = trace && session.projectiles.iter().any(|shot| shot.age < 0.001);
                    if trace
                        && (session.tick.is_multiple_of(60)
                            || session.is_finished()
                            || self_hit
                            || release)
                    {
                        let terrain = fixture.world().resource::<ArenaTerrainView>();
                        let geometry = *fixture.world().resource::<ArenaVoxelGeometry>();
                        println!(
                            "ARENA_BATTLE_TRACE {}",
                            serde_json::json!({
                                "seed":seed,"map":format!("{map:?}"),"left":left.slug(),"right":right.slug(),"tick":session.tick,
                                "self_hit":self_hit,"release":release,"before":before,
                                "projectiles":session.projectiles.iter().map(|shot|serde_json::json!({"id":shot.id,"owner":shot.owner,"position":shot.position.to_array(),"velocity":shot.velocity.to_array(),"age":shot.age})).collect::<Vec<_>>(),
                                "effects":session.effects.iter().filter(|effect|effect.age<0.02).map(|effect|serde_json::json!({"kind":effect.kind,"center":effect.center.to_array(),"radius":effect.radius,"age":effect.age})).collect::<Vec<_>>(),
                            "revision":terrain.revision,"knowledge":session.party_knowledge(),"decisions":session.creature_decisions(),
                                "actors":session.actors.iter().filter(|actor|actor.hp>0.0).map(|actor|serde_json::json!({
                                    "id":actor.id,"team":actor.team,"species":actor.species,"hp":actor.hp,"feet":actor.feet.to_array(),
                                    "eye":actor.eye().to_array(),"aim":actor.aim.to_array(),"body_rotation":actor.body_rotation().to_array(),
                                    "velocity":((actor.feet-actor.previous_feet)*120.0).to_array(),"impulse":actor.impulse_velocity().to_array(),"flying":actor.flying,"grounded":actor.grounded,
                                    "cooldowns":actor.cooldowns,"charge":actor.charge().map(|charge|charge.elapsed),
                                    "attack":actor.attack_state().map(|attack|serde_json::json!({"kind":attack.kind,"phase":attack.phase,"direction":attack.direction.to_array(),"progress":attack.progress})),
                                    "volume_valid":session.actor_volume_valid(actor.id,terrain,geometry)
                                })).collect::<Vec<_>>(),"stats":session.encounter_stats()
                            })
                        );
                    }
                    assert!(
                        timings.len()
                            <= usize::try_from(seconds * 120 + 2).expect("small round bound"),
                        "simulation failed to honor bound"
                    );
                }
                let terminal_publication = flush_terminal_publication(&mut fixture);
                let summary = battle(&fixture);
                let session = fixture.world().resource::<ArenaSession>();
                assert_eq!(
                    session.accepted_battle_setup(),
                    &setup,
                    "requested setup was not accepted exactly"
                );
                let published_map = fixture.world().resource::<ArenaTerrainView>().selection.map;
                assert_eq!(published_map, map, "requested map was not published");
                assert!(
                    !matches!(summary.result, Some(BattleResult::InvalidSetup(_))),
                    "deployment failure {map:?}/{left:?}/{right:?}: {:?}",
                    summary.result
                );
                println!(
                    "ARENA_BATTLE {}",
                    serde_json::json!({"map":format!("{published_map:?}"),"first":first.slug(),"second":second.slug(),"side_and_initiative_swapped":swap,"left":left.slug(),"right":right.slug(),"setup":session.accepted_battle_setup(),"setup_source":"accepted_battle_setup","setup_ms":setup_ms,"summary":summary,"actors":session.actors.iter().map(|a|serde_json::json!({"id":a.id,"team":a.team,"species":a.species,"hp":a.hp,"feet":[a.feet.x,a.feet.y,a.feet.z]})).collect::<Vec<_>>(),"stats":session.encounter_stats(),"tick_cpu":distribution(&timings),"publication_cpu":distribution(&publications),"terminal_publication":terminal_publication})
                );
            }
        }
    }
}
