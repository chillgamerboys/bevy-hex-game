use hex_core::{TerrainBatchId, TerrainImpactDisposition};

use super::*;

fn app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins).add_plugins(plugin);
    app.update();
    app
}

fn tick(app: &mut App) {
    app.world_mut().run_schedule(ArenaTick);
}

fn impact(app: &mut App, batch: u64, volume: Vec<TilePos>, power: u8) -> TerrainImpactOutcome {
    let element = app.world().resource::<ArenaMaterials>().fire;
    app.world_mut().write_message(TerrainImpact {
        batch: TerrainBatchId(batch),
        volume,
        element,
        power,
    });
    tick(app);
    let outcomes: Vec<_> = app
        .world_mut()
        .resource_mut::<Messages<TerrainImpactOutcome>>()
        .drain()
        .collect();
    assert_eq!(outcomes.len(), 1);
    outcomes.into_iter().next().expect("one outcome")
}

#[test]
fn authored_world_is_repeatable_bounded_and_has_two_supported_spawns() {
    let app = app();
    let geometry = *app.world().resource::<ArenaVoxelGeometry>();
    let materials = *app.world().resource::<ArenaMaterials>();
    let view = app.world().resource::<ArenaTerrainView>();
    let map = app.world().resource::<VoxelMap>();
    assert_eq!(map.len(), 469);
    assert!(view
        .voxels
        .keys()
        .all(|pos| geometry.contains_column(pos.coord)));
    for feet in view.spawns {
        let support = geometry
            .voxel_at(feet - Vec3::Y * 0.001)
            .expect("spawn in arena");
        assert_eq!(view.voxels.get(&support), Some(&materials.grass));
        assert!(!view.voxels.contains_key(&support.above()));
    }
    let repeated = build_arena(geometry, materials);
    for (coord, column) in map.columns() {
        let other = repeated.column(coord).expect("same footprint");
        assert_eq!(
            column.iter().collect::<Vec<_>>(),
            other.iter().collect::<Vec<_>>()
        );
        assert_eq!(column.get(0), materials.bedrock);
    }
    for side in [-1, 1] {
        for step in 1..=5 {
            let coord = HexCoord::from_axial((3 - step) * side, 6 * side);
            assert_eq!(map.surface(coord), Some(GROUND_LEVEL + step));
        }
        let platform = HexCoord::from_axial(-5 * side, 6 * side);
        assert_eq!(map.surface(platform), Some(GROUND_LEVEL + 5));
    }
    for q in -8..=8 {
        assert_eq!(map.surface(HexCoord::from_axial(q, 0)), Some(GROUND_LEVEL));
    }
}

#[test]
fn damage_uses_current_toughness_and_publishes_before_same_tick_consumers() {
    let mut app = app();
    let stone = TilePos::new(HexCoord::ORIGIN, 3);
    let revision = app.world().resource::<ArenaTerrainView>().revision;
    let first = impact(&mut app, 1, vec![stone], 2);
    let TerrainImpactResult::Applied(voxels) = first.result else {
        panic!("valid impact rejected");
    };
    assert_eq!(
        voxels.first().expect("voxel").disposition,
        TerrainImpactDisposition::Damaged
    );
    assert_eq!(
        app.world().resource::<ArenaTerrainView>().revision,
        revision
    );
    assert!(app
        .world()
        .resource::<ArenaTerrainView>()
        .voxels
        .contains_key(&stone));
    let second = impact(&mut app, 2, vec![stone], 2);
    let TerrainImpactResult::Applied(voxels) = second.result else {
        panic!("valid impact rejected");
    };
    assert_eq!(
        voxels.first().expect("voxel").disposition,
        TerrainImpactDisposition::Destroyed
    );
    assert_eq!(
        app.world().resource::<ArenaTerrainView>().revision,
        revision + 1
    );
    assert!(!app
        .world()
        .resource::<ArenaTerrainView>()
        .voxels
        .contains_key(&stone));
    // The run above the destroyed voxel remains a separate physical stack.
    assert!(app
        .world()
        .resource::<ArenaTerrainView>()
        .voxels
        .contains_key(&stone.above()));
}

#[test]
fn shield_edits_publish_whole_columns_and_cannot_replace_bedrock_or_extend_arena() {
    let mut app = app();
    let materials = *app.world().resource::<ArenaMaterials>();
    let wall = TilePos::new(HexCoord::ORIGIN, GROUND_LEVEL + 1);
    let bedrock = TilePos::new(HexCoord::ORIGIN, 0);
    let outside = TilePos::new(HexCoord::from_axial(13, 0), 9);
    for pos in [wall, wall.above(), bedrock, outside] {
        app.world_mut().write_message(TerrainEdit::Set {
            pos,
            substance: materials.stone,
        });
    }
    tick(&mut app);
    let view = app.world().resource::<ArenaTerrainView>();
    assert_eq!(view.voxels.get(&wall), Some(&materials.stone));
    assert_eq!(view.voxels.get(&wall.above()), Some(&materials.stone));
    assert_eq!(view.voxels.get(&bedrock), Some(&materials.bedrock));
    assert!(!view.voxels.contains_key(&outside));
    app.world_mut()
        .write_message(TerrainEdit::Clear { pos: wall });
    app.world_mut()
        .write_message(TerrainEdit::Clear { pos: bedrock });
    tick(&mut app);
    assert!(!app
        .world()
        .resource::<ArenaTerrainView>()
        .voxels
        .contains_key(&wall));
    assert_eq!(
        app.world()
            .resource::<ArenaTerrainView>()
            .voxels
            .get(&bedrock),
        Some(&materials.bedrock)
    );
}

#[test]
fn invalid_and_duplicate_impacts_have_exactly_one_rejection_without_mutation() {
    let mut app = app();
    let pos = TilePos::new(HexCoord::ORIGIN, 3);
    let revision = app.world().resource::<ArenaTerrainView>().revision;
    let malformed = impact(&mut app, 7, vec![pos, pos], 2);
    assert_eq!(
        malformed.result,
        TerrainImpactResult::Rejected(TerrainImpactRejection::NonCanonicalVolume)
    );
    let repeated = impact(&mut app, 7, vec![pos], 2);
    assert_eq!(
        repeated.result,
        TerrainImpactResult::Rejected(TerrainImpactRejection::ReusedBatch)
    );
    assert_eq!(
        app.world().resource::<ArenaTerrainView>().revision,
        revision
    );
}

#[test]
fn reset_restores_material_health_and_batch_ledger_and_drops_queued_old_effects() {
    let mut app = app();
    let initial = app.world().resource::<ArenaTerrainView>().voxels.clone();
    let stone = TilePos::new(HexCoord::ORIGIN, 3);
    impact(&mut app, 1, vec![stone], 2);
    let materials = *app.world().resource::<ArenaMaterials>();
    app.world_mut().write_message(TerrainEdit::Set {
        pos: TilePos::new(HexCoord::ORIGIN, 12),
        substance: materials.stone,
    });
    app.world_mut().write_message(TerrainImpact {
        batch: TerrainBatchId(2),
        volume: vec![stone],
        element: materials.fire,
        power: 8,
    });
    app.world_mut().resource_mut::<ArenaReset>().generation += 1;
    tick(&mut app);
    assert_eq!(app.world().resource::<ArenaTerrainView>().voxels, initial);
    assert!(app
        .world()
        .resource::<Messages<TerrainImpactOutcome>>()
        .is_empty());
    assert!(app.world().resource::<Messages<TerrainEdit>>().is_empty());
    assert!(app.world().resource::<Messages<TerrainImpact>>().is_empty());
    let fresh = impact(&mut app, 1, vec![stone], 2);
    let TerrainImpactResult::Applied(voxels) = fresh.result else {
        panic!("batch ledger retained across reset");
    };
    let health = voxels
        .first()
        .expect("voxel")
        .health_before
        .expect("health");
    assert_eq!(health.remaining, 4);
    assert_eq!(health.maximum, 4);
}

#[test]
fn announcements_survive_paused_frames_without_mutating_until_the_next_tick() {
    let mut app = app();
    let materials = *app.world().resource::<ArenaMaterials>();
    let pos = TilePos::new(HexCoord::ORIGIN, GROUND_LEVEL);
    app.world_mut().write_message(TerrainImpact {
        batch: TerrainBatchId(9),
        volume: vec![pos],
        element: materials.fire,
        power: 2,
    });
    for _ in 0..8 {
        app.update();
    }
    assert!(app
        .world()
        .resource::<ArenaTerrainView>()
        .voxels
        .contains_key(&pos));
    tick(&mut app);
    assert!(!app
        .world()
        .resource::<ArenaTerrainView>()
        .voxels
        .contains_key(&pos));
}
