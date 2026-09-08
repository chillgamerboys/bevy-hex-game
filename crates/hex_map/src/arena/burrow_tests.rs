//! Independent world authority tests for the queued conversion protocol.

use hex_core::arena::{ArenaBurrowRejection as Reject, ArenaStaticSpan};
use hex_core::{TerrainBatchId, TerrainDamageKind, TerrainVoxelHealth};

use super::*;

fn app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins).add_plugins(plugin);
    app.update();
    app
}

fn tile(q: i32, level: i32) -> TilePos {
    TilePos::new(HexCoord::from_axial(q, 0), level)
}

fn request(app: &mut App, sequence: u64, volume: Vec<TilePos>) -> ArenaBurrowOutcome {
    let generation = app.world().resource::<ArenaReset>().generation;
    app.world_mut().write_message(ArenaBurrowRequest {
        generation,
        actor: 7,
        sequence,
        volume,
    });
    app.world_mut().run_schedule(ArenaTick);
    app.world_mut()
        .resource_mut::<Messages<ArenaBurrowOutcome>>()
        .drain()
        .next()
        .expect("one correlated outcome")
}

fn damage(app: &mut App, batch: u64, position: TilePos, power: u8) {
    app.world_mut().write_message(TerrainImpact {
        batch: TerrainBatchId(batch),
        kind: TerrainDamageKind::Physical,
        power,
        volume: vec![position],
    });
}

fn health(app: &App, position: TilePos) -> Option<TerrainVoxelHealth> {
    app.world().resource::<DamagedVoxels>().get(position)
}

#[test]
fn conversion_reads_real_same_tick_hp_and_preserves_already_damaged_dirt() {
    let mut app = app();
    let [stone_one, stone_three, grass, dirt] = [tile(-3, 3), tile(-2, 3), tile(-1, 8), tile(0, 5)];
    damage(&mut app, 1, stone_one, 3);
    damage(&mut app, 2, stone_three, 1);
    damage(&mut app, 3, dirt, 1);
    let before = app.world().resource::<ArenaTerrainView>().revision;
    let outcome = request(&mut app, 1, vec![stone_one, stone_three, grass, dirt]);
    let ArenaBurrowResult::Accepted { changed } = outcome.result else {
        panic!("eligible earth");
    };
    assert_eq!(
        (outcome.actor, outcome.sequence, outcome.generation),
        (7, 1, 0)
    );
    assert_eq!(
        changed
            .iter()
            .map(|change| change.position)
            .collect::<Vec<_>>(),
        vec![stone_one, stone_three, grass]
    );
    let records = changed
        .iter()
        .map(|change| {
            (
                change.health_before.remaining,
                change.health_before.maximum,
                change.health_after.remaining,
                change.health_after.maximum,
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(records, [(1, 4, 1, 2), (3, 4, 2, 2), (1, 1, 1, 2)]);
    for pos in [stone_one, grass, dirt] {
        assert_eq!(health(&app, pos), TerrainVoxelHealth::new(1, 2));
    }
    assert_eq!(health(&app, stone_three), None);
    let material = app.world().resource::<ArenaMaterials>().dirt;
    let view = app.world().resource::<ArenaTerrainView>();
    assert_eq!(view.revision, before + 1);
    assert_eq!(
        view.dirty_columns,
        [stone_one.coord, stone_three.coord, grass.coord]
            .into_iter()
            .collect()
    );
    assert!(!view.full_rebuild);
    for pos in [stone_one, stone_three, grass, dirt] {
        assert_eq!(view.voxels.get(&pos), Some(&material));
    }
    // A second conversion is a genuine no-op, not a health reset. Ordinary damage
    // must still destroy the one-HP dirt in the same subsequent tick.
    damage(&mut app, 4, stone_one, 1);
    let ArenaBurrowResult::Accepted { changed } =
        request(&mut app, 2, vec![stone_one, dirt]).result
    else {
        panic!("air and dirt");
    };
    assert!(changed.is_empty());
    assert!(app.world().resource::<VoxelMap>().get(stone_one).is_air());
    assert_eq!(health(&app, dirt), TerrainVoxelHealth::new(1, 2));
}

#[test]
fn direct_edits_and_destruction_resolve_before_conversion_without_refilling_air() {
    let mut app = app();
    let new_shield = tile(-2, 15);
    let removed = tile(2, 3);
    let stone = app.world().resource::<ArenaMaterials>().stone;
    app.world_mut().write_message(TerrainEdit::Set {
        pos: new_shield,
        substance: stone,
    });
    damage(&mut app, 1, new_shield, 3);
    damage(&mut app, 2, removed, 4);
    let ArenaBurrowResult::Accepted { changed } =
        request(&mut app, 1, vec![new_shield, removed]).result
    else {
        panic!("live material admission");
    };
    assert_eq!(changed.len(), 1);
    assert_eq!(changed.first().expect("shield change").position, new_shield);
    assert_eq!(health(&app, new_shield), TerrainVoxelHealth::new(1, 2));
    assert!(app.world().resource::<VoxelMap>().get(removed).is_air());
}

#[test]
fn atomic_mixed_volumes_reject_bounds_bedrock_protected_air_liquid_and_unmasked_objects() {
    for case in 0..9 {
        let mut app = app();
        let candidate = tile(-3, 8);
        let mut rejected = tile(2, 12);
        let reason = match case {
            0 => {
                rejected = tile(2, 0);
                Reject::IneligibleMaterial
            }
            1 => {
                rejected = tile(100, 12);
                Reject::OutsideWorld
            }
            2 => {
                rejected = tile(2, -1);
                Reject::OutsideWorld
            }
            3 => {
                rejected = tile(2, 129);
                Reject::OutsideWorld
            }
            4 => {
                app.world_mut()
                    .resource_mut::<ArenaWorldState>()
                    .original
                    .as_mut()
                    .expect("recipe")
                    .view
                    .edit_protected
                    .insert(rejected.coord, vec![(rejected.level, rejected.level)]);
                Reject::Protected
            }
            5 => {
                let water = app
                    .world()
                    .resource::<SubstanceTable>()
                    .id("water")
                    .expect("water");
                app.world_mut()
                    .resource_mut::<ArenaWorldState>()
                    .original
                    .as_mut()
                    .expect("recipe")
                    .view
                    .liquids
                    .push(ArenaSolidSpan {
                        bottom: rejected,
                        top_level: rejected.level,
                        substance: water,
                    });
                Reject::Liquid
            }
            7 => {
                rejected = TilePos::new(HexCoord::from_axial(i32::MAX, i32::MAX), 12);
                Reject::OutsideWorld
            }
            8 => {
                rejected = TilePos::new(HexCoord::from_axial(i32::MIN, 0), 12);
                Reject::OutsideWorld
            }
            _ => {
                app.world_mut()
                    .resource_mut::<ArenaWorldState>()
                    .original
                    .as_mut()
                    .expect("recipe")
                    .view
                    .static_spans
                    .push(ArenaStaticSpan {
                        bottom: rejected,
                        top_level: rejected.level,
                        blocks_movement: false,
                        blocks_projectiles: false,
                        blocks_sight: false,
                    });
                Reject::StaticObject
            }
        };
        let material = app.world().resource::<VoxelMap>().get(candidate);
        let revision = app.world().resource::<ArenaTerrainView>().revision;
        let mut volume = vec![candidate, rejected];
        volume.sort_unstable();
        assert_eq!(
            request(&mut app, 1, volume).result,
            ArenaBurrowResult::Rejected {
                position: Some(rejected),
                reason
            }
        );
        assert_eq!(app.world().resource::<VoxelMap>().get(candidate), material);
        assert_eq!(health(&app, candidate), None);
        assert_eq!(
            app.world().resource::<ArenaTerrainView>().revision,
            revision
        );
    }
}

#[test]
fn interval_guards_do_not_block_unrelated_levels_in_the_same_column() {
    let mut app = app();
    let candidate = tile(2, 8);
    let high = tile(2, 12);
    {
        let mut state = app.world_mut().resource_mut::<ArenaWorldState>();
        let view = &mut state.original.as_mut().expect("recipe").view;
        view.edit_protected.insert(high.coord, vec![(12, 14)]);
        view.static_spans.push(ArenaStaticSpan {
            bottom: high,
            top_level: 14,
            blocks_movement: false,
            blocks_projectiles: false,
            blocks_sight: false,
        });
    }
    assert!(matches!(
        request(&mut app, 1, vec![candidate]).result,
        ArenaBurrowResult::Accepted { .. }
    ));
}

#[test]
fn envelope_failures_consume_sequences_and_stale_generations_do_not_poison_current_sources() {
    let mut app = app();
    let candidate = tile(0, 8);
    for (sequence, volume, reason) in [
        (1, vec![], Reject::EmptyVolume),
        (2, vec![candidate, candidate], Reject::NonCanonicalVolume),
        (3, vec![tile(1, 8), candidate], Reject::NonCanonicalVolume),
        (
            4,
            (0..65).map(|level| tile(0, level)).collect(),
            Reject::TooManyCells,
        ),
    ] {
        assert_eq!(
            request(&mut app, sequence, volume).result,
            ArenaBurrowResult::Rejected {
                position: None,
                reason
            }
        );
        assert_eq!(
            request(&mut app, sequence, vec![candidate]).result,
            ArenaBurrowResult::Rejected {
                position: None,
                reason: Reject::ReusedSequence
            }
        );
    }
    app.world_mut().write_message(ArenaBurrowRequest {
        generation: 99,
        actor: 7,
        sequence: u64::MAX,
        volume: vec![candidate],
    });
    app.world_mut().run_schedule(ArenaTick);
    let stale = app
        .world_mut()
        .resource_mut::<Messages<ArenaBurrowOutcome>>()
        .drain()
        .next()
        .expect("stale outcome");
    assert_eq!(
        stale.result,
        ArenaBurrowResult::Rejected {
            position: None,
            reason: Reject::StaleGeneration
        }
    );
    assert!(matches!(
        request(&mut app, 5, vec![candidate]).result,
        ArenaBurrowResult::Accepted { .. }
    ));
}

#[test]
fn exactly_sixty_four_noop_cells_do_not_change_publication() {
    let mut app = app();
    let revision = app.world().resource::<ArenaTerrainView>().revision;
    let volume = (40..104).map(|level| tile(0, level)).collect();
    assert_eq!(
        request(&mut app, 1, volume).result,
        ArenaBurrowResult::Accepted { changed: vec![] }
    );
    assert_eq!(
        app.world().resource::<ArenaTerrainView>().revision,
        revision
    );
}

#[test]
fn paused_requests_survive_and_reset_clears_outcomes_inbox_sequences_and_converted_hp() {
    let mut app = app();
    let candidate = tile(0, 8);
    let original = app.world().resource::<VoxelMap>().get(candidate);
    app.world_mut().write_message(ArenaBurrowRequest {
        generation: 0,
        actor: 7,
        sequence: 1,
        volume: vec![candidate],
    });
    for _ in 0..5 {
        app.update();
    }
    assert_eq!(app.world().resource::<VoxelMap>().get(candidate), original);
    app.world_mut().run_schedule(ArenaTick);
    assert_ne!(app.world().resource::<VoxelMap>().get(candidate), original);
    assert_eq!(health(&app, candidate), TerrainVoxelHealth::new(1, 2));
    app.world_mut().write_message(ArenaBurrowRequest {
        generation: 0,
        actor: 7,
        sequence: 2,
        volume: vec![tile(1, 8)],
    });
    app.world_mut().run_schedule(PreUpdate);
    app.world_mut().resource_mut::<ArenaReset>().generation = 1;
    app.world_mut().run_schedule(ArenaTick);
    assert_eq!(app.world().resource::<VoxelMap>().get(candidate), original);
    assert_eq!(health(&app, candidate), None);
    assert!(app
        .world()
        .resource::<ArenaWorldState>()
        .burrow_sequences
        .is_empty());
    assert!(app.world().resource::<ArenaInbox>().burrows.is_empty());
    assert!(app
        .world()
        .resource::<Messages<ArenaBurrowRequest>>()
        .is_empty());
    assert!(app
        .world()
        .resource::<Messages<ArenaBurrowOutcome>>()
        .is_empty());
    assert!(matches!(
        request(&mut app, 1, vec![candidate]).result,
        ArenaBurrowResult::Accepted { .. }
    ));
}

#[test]
fn invalid_destination_rejects_atomically_and_material_policy_is_world_authored() {
    let mut app = app();
    let materials = *app.world().resource::<ArenaMaterials>();
    let policy = app.world().resource::<ArenaBurrowMaterials>();
    for substance in [materials.grass, materials.dirt, materials.stone] {
        assert!(policy.eligible.contains(&substance));
    }
    assert!(!policy.eligible.contains(&materials.bedrock));
    assert!(!policy.eligible.contains(&SubstanceId::AIR));
    app.world_mut()
        .resource_mut::<ArenaBurrowMaterials>()
        .eligible
        .remove(&materials.dirt);
    assert_eq!(
        request(&mut app, 1, vec![tile(0, 8)]).result,
        ArenaBurrowResult::Rejected {
            position: None,
            reason: Reject::InvalidDestination
        }
    );
    assert_eq!(
        app.world().resource::<VoxelMap>().get(tile(0, 8)),
        materials.grass
    );
}

#[test]
fn distinct_actors_can_reuse_a_sequence_and_overlapping_requests_never_repair_dirt() {
    let mut app = app();
    let position = tile(0, 8);
    let revision = app.world().resource::<ArenaTerrainView>().revision;
    for actor in 0..24 {
        app.world_mut().write_message(ArenaBurrowRequest {
            generation: 0,
            actor,
            sequence: 1,
            volume: vec![position],
        });
    }
    app.world_mut().run_schedule(ArenaTick);
    let outcomes = app
        .world_mut()
        .resource_mut::<Messages<ArenaBurrowOutcome>>()
        .drain()
        .collect::<Vec<_>>();
    assert_eq!(outcomes.len(), 24);
    for (expected_actor, outcome) in (0..24).zip(outcomes) {
        assert_eq!(outcome.actor, expected_actor);
        let ArenaBurrowResult::Accepted { changed } = outcome.result else {
            panic!("independent actor sequence must be admitted");
        };
        assert_eq!(changed.len(), usize::from(expected_actor == 0));
    }
    assert_eq!(health(&app, position), TerrainVoxelHealth::new(1, 2));
    assert_eq!(
        app.world()
            .resource::<ArenaWorldState>()
            .burrow_sequences
            .len(),
        24
    );
    let view = app.world().resource::<ArenaTerrainView>();
    assert_eq!(view.revision, revision + 1);
    assert_eq!(view.dirty_columns, BTreeSet::from([position.coord]));
}
