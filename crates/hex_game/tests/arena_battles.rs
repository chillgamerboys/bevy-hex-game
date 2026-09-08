//! Actual world/120 Hz arena composition and explicit local calibration.
#![expect(
    clippy::expect_used,
    reason = "invalid deterministic fixtures fail the contract"
)]

use bevy::prelude::*;
use hex_arena::{ArenaBattleSetup, ArenaSession, BattlePreset, BattleResult, BattleSummary};
use hex_core::arena::{
    ArenaMap, ArenaReset, ArenaSelection, ArenaTerrainView, ArenaTick, ArenaVoxelGeometry,
};
use std::time::Instant;

fn app(map: ArenaMap, setup: ArenaBattleSetup) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
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
                let mut fixture = app(map, setup.clone());
                let setup_ms = began.elapsed().as_secs_f64() * 1000.0;
                let mut timings = Vec::new();
                let mut publications = Vec::new();
                loop {
                    if fixture.world().resource::<ArenaSession>().is_finished() {
                        break;
                    }
                    let revision = fixture.world().resource::<ArenaTerrainView>().revision;
                    let began = Instant::now();
                    fixture.world_mut().run_schedule(ArenaTick);
                    let ms = began.elapsed().as_secs_f64() * 1000.0;
                    timings.push(ms);
                    if fixture.world().resource::<ArenaTerrainView>().revision != revision {
                        publications.push(ms);
                    }
                    assert!(
                        timings.len()
                            <= usize::try_from(seconds * 120 + 2).expect("small round bound"),
                        "simulation failed to honor bound"
                    );
                }
                let summary = battle(&fixture);
                let session = fixture.world().resource::<ArenaSession>();
                assert!(
                    !matches!(summary.result, Some(BattleResult::InvalidSetup(_))),
                    "deployment failure {map:?}/{left:?}/{right:?}: {:?}",
                    summary.result
                );
                println!(
                    "ARENA_BATTLE {}",
                    serde_json::json!({"map":format!("{map:?}"),"first":first.slug(),"second":second.slug(),"side_and_initiative_swapped":swap,"left":left.slug(),"right":right.slug(),"setup":setup,"setup_ms":setup_ms,"summary":summary,"actors":session.actors.iter().map(|a|serde_json::json!({"id":a.id,"team":a.team,"species":a.species,"hp":a.hp,"feet":[a.feet.x,a.feet.y,a.feet.z]})).collect::<Vec<_>>(),"stats":session.encounter_stats(),"tick_cpu":distribution(&timings),"publication_cpu":distribution(&publications)})
                );
            }
        }
    }
}
