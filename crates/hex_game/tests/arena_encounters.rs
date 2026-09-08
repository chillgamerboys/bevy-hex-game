//! Combined authored-world and continuous-combat contracts. No renderer or window.

#![expect(
    clippy::expect_used,
    reason = "invalid deterministic fixtures must fail these contracts"
)]

use bevy::prelude::*;
use hex_arena::{ArenaInput, ArenaOutcome, ArenaSession, Spell};
use hex_core::arena::{
    ArenaEncounter, ArenaMap, ArenaReset, ArenaSelection, ArenaTerrainView, ArenaTick,
    ArenaVoxelGeometry,
};
use hex_core::{HexCoord, TerrainEdit, TilePos};

fn app(selection: ArenaSelection) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(selection)
        .add_plugins((hex_map::arena::plugin, hex_arena::plugin));
    app.world_mut().resource_mut::<ArenaSession>().bot_enabled = false;
    app.update();
    tick(&mut app);
    app
}

fn tick(app: &mut App) {
    app.world_mut().run_schedule(ArenaTick);
}

fn select(app: &mut App, selection: ArenaSelection) {
    *app.world_mut().resource_mut::<ArenaSelection>() = selection;
    app.world_mut().resource_mut::<ArenaReset>().generation += 1;
    tick(app);
}

fn fort(encounter: ArenaEncounter) -> ArenaSelection {
    ArenaSelection {
        map: ArenaMap::Fort,
        encounter,
    }
}

#[test]
fn every_recipe_publishes_the_expected_living_roster_and_safe_player_start() {
    for (selection, count) in [
        (ArenaSelection::default(), 2),
        (fort(ArenaEncounter::Dragon), 2),
        (fort(ArenaEncounter::Goblins), 6),
        (fort(ArenaEncounter::ShamanParty), 5),
        (fort(ArenaEncounter::Shadow), 2),
        (
            ArenaSelection {
                map: ArenaMap::SevenRegions,
                ..Default::default()
            },
            11,
        ),
    ] {
        let fixture = app(selection);
        let world = fixture.world().resource::<ArenaTerrainView>();
        let geometry = fixture.world().resource::<ArenaVoxelGeometry>();
        let session = fixture.world().resource::<ArenaSession>();
        assert_eq!(world.selection, selection);
        assert_eq!(session.actors.len(), count, "{selection:?}");
        let human = session.actors.first().expect("human");
        assert_eq!(human.id, 0);
        let mut ids = std::collections::BTreeSet::new();
        for actor in &session.actors {
            assert!(ids.insert(actor.id), "duplicate stable ID");
            assert!(actor.hp > 0.0 && actor.feet.is_finite());
            assert!(geometry.contains_column(HexCoord::from_world(actor.feet)));
            assert!(
                session.actor_pose_valid(actor.id, world, *geometry),
                "{selection:?} actor {} lacks full-body clearance or dry support at {:?}",
                actor.id,
                actor.feet,
            );
            if selection.map != ArenaMap::Duel && actor.id != 0 {
                assert!(
                    actor.feet.distance(human.feet) > 12.0,
                    "{selection:?} starts inside enemy activation range"
                );
            }
        }
        assert!(session.outcome.is_none());
    }
}

#[test]
fn selecting_and_restarting_restores_world_and_roster_without_old_actor_ids() {
    let mut fixture = app(fort(ArenaEncounter::Goblins));
    let original = fixture
        .world()
        .resource::<ArenaTerrainView>()
        .voxels
        .clone();
    let original_actors: Vec<_> = fixture
        .world()
        .resource::<ArenaSession>()
        .actors
        .iter()
        .map(|actor| (actor.id, actor.hp, actor.feet))
        .collect();
    let removable = {
        let world = fixture.world().resource::<ArenaTerrainView>();
        world
            .voxels
            .keys()
            .rev()
            .find(|voxel| {
                voxel.level > 0
                    && !world.edit_protected.get(&voxel.coord).is_some_and(|spans| {
                        spans
                            .iter()
                            .any(|(low, high)| (*low..=*high).contains(&voxel.level))
                    })
            })
            .copied()
            .expect("destructible unprotected terrain")
    };
    fixture
        .world_mut()
        .write_message(TerrainEdit::Clear { pos: removable });
    tick(&mut fixture);
    assert!(!fixture
        .world()
        .resource::<ArenaTerrainView>()
        .voxels
        .contains_key(&removable));
    fixture
        .world_mut()
        .resource_mut::<ArenaSession>()
        .actors
        .first_mut()
        .expect("human")
        .hp = 17.0;
    select(&mut fixture, fort(ArenaEncounter::Goblins));
    assert_eq!(
        fixture.world().resource::<ArenaTerrainView>().voxels,
        original
    );
    let reset_actors: Vec<_> = fixture
        .world()
        .resource::<ArenaSession>()
        .actors
        .iter()
        .map(|actor| (actor.id, actor.hp, actor.feet))
        .collect();
    assert_eq!(reset_actors, original_actors);
    for selection in [
        fort(ArenaEncounter::Dragon),
        ArenaSelection {
            map: ArenaMap::SevenRegions,
            ..Default::default()
        },
        ArenaSelection::default(),
    ] {
        select(&mut fixture, selection);
        let count = if selection.map == ArenaMap::SevenRegions {
            11
        } else {
            2
        };
        let session = fixture.world().resource::<ArenaSession>();
        assert_eq!(session.actors.len(), count);
        assert!(session.projectiles.is_empty() && session.effects.is_empty());
        assert!(session.actors.iter().all(|actor| actor.charge().is_none()));
        assert!(session.outcome.is_none());
    }
}

#[test]
fn one_surviving_enemy_does_not_win_until_the_player_dies() {
    let selection = ArenaSelection {
        map: ArenaMap::SevenRegions,
        ..Default::default()
    };
    let mut fixture = app(selection);
    {
        let mut session = fixture.world_mut().resource_mut::<ArenaSession>();
        for actor in session.actors.iter_mut().skip(2) {
            actor.hp = 0.0;
        }
    }
    tick(&mut fixture);
    assert!(fixture.world().resource::<ArenaSession>().outcome.is_none());
    fixture
        .world_mut()
        .resource_mut::<ArenaSession>()
        .actors
        .get_mut(1)
        .expect("enemy")
        .hp = 0.0;
    tick(&mut fixture);
    assert_eq!(
        fixture.world().resource::<ArenaSession>().outcome,
        Some(ArenaOutcome::Winner(0))
    );

    select(&mut fixture, selection);
    fixture
        .world_mut()
        .resource_mut::<ArenaSession>()
        .actors
        .first_mut()
        .expect("human")
        .hp = 0.0;
    tick(&mut fixture);
    let session = fixture.world().resource::<ArenaSession>();
    assert!(session.outcome.is_some());
    assert_ne!(session.outcome, Some(ArenaOutcome::Winner(0)));
    assert_eq!(
        session.actors.iter().filter(|actor| actor.hp > 0.0).count(),
        10
    );
}

#[test]
fn authored_world_edits_refresh_camera_collision_in_the_same_tick() {
    for selection in [
        ArenaSelection::default(),
        fort(ArenaEncounter::Dragon),
        ArenaSelection {
            map: ArenaMap::SevenRegions,
            ..Default::default()
        },
    ] {
        let mut fixture = app(selection);
        let geometry = *fixture.world().resource::<ArenaVoxelGeometry>();
        let pos = {
            let world = fixture.world().resource::<ArenaTerrainView>();
            let level = world
                .voxels
                .keys()
                .map(|pos| pos.level)
                .max()
                .expect("terrain")
                + 12;
            let coord = world
                .columns
                .keys()
                .find(|coord| !world.edit_protected.contains_key(coord))
                .copied()
                .expect("unprotected column");
            TilePos::new(coord, level)
        };
        assert!(pos.level <= geometry.max_level);
        let center = geometry.center(pos);
        let eye = center - Vec3::X * 3.0;
        let desired = center + Vec3::X * 3.0;
        let stone = fixture
            .world()
            .resource::<hex_core::arena::ArenaMaterials>()
            .stone;
        fixture.world_mut().write_message(TerrainEdit::Set {
            pos,
            substance: stone,
        });
        tick(&mut fixture);
        let blocked = fixture
            .world()
            .resource::<ArenaSession>()
            .camera_position(eye, desired);
        assert!(
            blocked.distance(desired) > 1.0,
            "new wall must already block {selection:?}"
        );
        fixture
            .world_mut()
            .write_message(TerrainEdit::Clear { pos });
        tick(&mut fixture);
        let clear = fixture
            .world()
            .resource::<ArenaSession>()
            .camera_position(eye, desired);
        assert!(
            clear.distance(desired) < 0.001,
            "removed wall must already clear {selection:?}"
        );
    }
}

/// A synthetic stress fixture, not a human balance or ordinary travel scenario.
/// The player visits each party repeatedly and receives extra life to keep every
/// decision loop running. Execute separately in the native launcher build profile.
#[test]
#[ignore = "explicit native-profile timing measurement; emits a performance receipt"]
fn profile_simultaneous_seven_region_combat() {
    profile_combat(ArenaSelection {
        map: ArenaMap::SevenRegions,
        ..Default::default()
    });
}

#[test]
#[ignore = "explicit native-profile timing measurement; emits four performance receipts"]
fn profile_fort_encounter_presets() {
    for encounter in [
        ArenaEncounter::Dragon,
        ArenaEncounter::Goblins,
        ArenaEncounter::ShamanParty,
        ArenaEncounter::Shadow,
    ] {
        profile_combat(fort(encounter));
    }
}

fn profile_combat(selection: ArenaSelection) {
    let load_start = std::time::Instant::now();
    let mut fixture = app(selection);
    let headless_setup_ms = load_start.elapsed().as_secs_f64() * 1000.0;
    let representatives = {
        let mut session = fixture.world_mut().resource_mut::<ArenaSession>();
        session.bot_enabled = true;
        for actor in &mut session.actors {
            actor.max_hp = 100_000.0;
            actor.hp = actor.max_hp;
        }
        session
            .parties()
            .iter()
            .map(|party| {
                session
                    .actors
                    .iter()
                    .find(|actor| actor.party == Some(party.id))
                    .expect("party representative")
                    .id
            })
            .collect::<Vec<_>>()
    };
    let enemy_count = fixture.world().resource::<ArenaSession>().actors.len() - 1;
    let party_count = representatives.len();
    let mut samples = Vec::new();
    let mut all_active_samples = Vec::new();
    let mut publication_samples = Vec::new();
    let mut damage_samples = Vec::new();
    let mut destroyed_voxels = 0;
    let mut peak_active = 0;
    let mut peak_projectiles = 0;
    let mut peak_barriers = 0;
    for step in 0..3600 {
        let index = (step / 12) % representatives.len();
        let (feet, aim) = {
            let session = fixture.world().resource::<ArenaSession>();
            let target = session
                .actors
                .iter()
                .find(|actor| actor.id == *representatives.get(index).expect("bounded party index"))
                .expect("representative remains present");
            let offset =
                target.body_rotation() * Vec3::X * (target.body_dimensions().x * 0.5 + 0.4);
            (target.feet + offset, -offset.normalize_or_zero())
        };
        {
            let mut session = fixture.world_mut().resource_mut::<ArenaSession>();
            let human = session.actors.first_mut().expect("human");
            human.feet = feet;
            human.aim = aim;
        }
        {
            let mut input = fixture.world_mut().resource_mut::<ArenaInput>();
            input.human.aim = aim;
            input.human.selected = Some(Spell::AreaBlast);
            input.human.cast_pressed = step % 240 == 0;
            input.human.cast_released = step % 240 == 0;
        }
        let revision_before = fixture.world().resource::<ArenaTerrainView>().revision;
        let voxels_before = fixture.world().resource::<ArenaTerrainView>().voxels.len();
        let outcomes_before = fixture.world().resource::<ArenaSession>().terrain_outcomes;
        let start = std::time::Instant::now();
        tick(&mut fixture);
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        let session = fixture.world().resource::<ArenaSession>();
        assert!(session.outcome.is_none(), "stress actors must remain alive");
        peak_active = peak_active.max(session.encounter_summary().active_parties);
        peak_projectiles = peak_projectiles.max(session.projectiles.len());
        peak_barriers = peak_barriers.max(session.barriers().len());
        let world = fixture.world().resource::<ArenaTerrainView>();
        destroyed_voxels += voxels_before.saturating_sub(world.voxels.len());
        if step >= 120 {
            samples.push(elapsed);
            if session.encounter_summary().active_parties == party_count
                && session.encounter_summary().living_enemies == enemy_count
            {
                all_active_samples.push(elapsed);
            }
            if world.revision != revision_before {
                publication_samples.push(elapsed);
            }
            if session.terrain_outcomes > outcomes_before {
                damage_samples.push(elapsed);
            }
        }
    }
    let distribution = |mut values: Vec<f64>| {
        values.sort_by(f64::total_cmp);
        if values.is_empty() {
            return serde_json::json!({"ticks": 0});
        }
        let percentile = |percent| {
            *values
                .get((values.len() - 1) * percent / 100)
                .expect("percentile of nonempty samples")
        };
        serde_json::json!({
            "ticks": values.len(), "p50_ms": percentile(50), "p95_ms": percentile(95),
            "p99_ms": percentile(99), "max_ms": percentile(100),
        })
    };
    let all_active_ticks = all_active_samples.len();
    let publication_ticks = publication_samples.len();
    let damage_ticks = damage_samples.len();
    let session = fixture.world().resource::<ArenaSession>();
    let receipt = serde_json::json!({
        "fixture": "synthetic-party-visits-extra-life",
        "map": format!("{:?}", selection.map),
        "encounter": format!("{:?}", selection.encounter),
        "measurement": "headless ArenaTick wall time; excludes renderer and app frame cost",
        "headless_setup_ms": headless_setup_ms,
        "ticks_measured": samples.len(),
        "all_active_ticks": all_active_ticks,
        "all_active_fraction": f64::from(u32::try_from(all_active_ticks).expect("bounded timing sample count"))
            / f64::from(u32::try_from(samples.len()).expect("bounded timing sample count")),
        "all_ticks": distribution(samples),
        "all_enemies_active": distribution(all_active_samples),
        "terrain_publication_ticks": distribution(publication_samples),
        "terrain_damage_outcome_ticks": distribution(damage_samples),
        "destroyed_voxels": destroyed_voxels,
        "peak_active_parties": peak_active, "peak_projectiles": peak_projectiles,
        "peak_barriers": peak_barriers,
        "living_enemies": session.encounter_summary().living_enemies,
        "terrain_outcomes": session.terrain_outcomes,
    });
    #[expect(
        clippy::print_stdout,
        reason = "explicit ignored benchmark emits its machine-readable receipt"
    )]
    {
        println!("ENCOUNTER_PERFORMANCE {receipt}");
    }
    assert_eq!(peak_active, party_count, "measure all parties concurrently");
    assert!(
        all_active_ticks >= 2400,
        "need sustained all-party combat, not only a peak"
    );
    assert!(
        publication_ticks > 0 && damage_ticks > 0 && destroyed_voxels > 0,
        "require measured damage and destruction publication pressure"
    );
    assert_eq!(session.encounter_summary().living_enemies, enemy_count);
}
