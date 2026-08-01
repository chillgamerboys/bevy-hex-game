use super::*;

#[test]
fn nonprocedural_maps_publish_an_empty_region_registry() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    assert!(app.world().resource::<SpecialMovementRegions>().is_empty());
    assert!(app.world().resource::<InteriorRegions>().is_empty());
}

/// Every column produces at least one entity, and typically several — one per
/// substance run.
#[test]
fn entering_gameplay_spawns_a_full_grid() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    // A hexagon of radius r holds 3r² + 3r + 1 columns.
    let columns = (3 * TEST_RADIUS * TEST_RADIUS + 3 * TEST_RADIUS + 1) as usize;
    assert!(
        tile_count(&mut app) >= columns,
        "every column should spawn at least one prism"
    );
}

/// Generated terrain is solid from the bedrock floor upward, with no gaps.
///
/// Digging needs something to dig through, so a column starting above ground would be
/// a hole nothing could stand in. Floating spans are legal in general — that is what
/// `HexSpan` is for — but the *generator* must not produce them.
#[test]
fn generated_terrain_is_solid_to_the_floor() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let map = app
        .world()
        .get_resource::<VoxelMap>()
        .expect("gameplay should have generated a world");

    for (coord, column) in map.columns() {
        assert!(!column.is_empty(), "{coord:?} has no ground at all");
        for level in 0..column.top() {
            assert!(
                !column.get(level).is_air(),
                "{coord:?} has a gap at level {level}; generated terrain should be solid"
            );
        }
    }
}

/// Every column has at least one level above bedrock.
///
/// Bedrock is deliberately not diggable, so a column of nothing but bedrock would be a
/// permanent hole in the world.
#[test]
fn no_column_is_bare_bedrock() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let map = app
        .world()
        .get_resource::<VoxelMap>()
        .expect("gameplay should have generated a world");

    for (coord, column) in map.columns() {
        assert!(
            column.top() >= 2,
            "{coord:?} is bare bedrock at height {}",
            column.top()
        );
    }
}

/// Entity count scales with substance *variety*, not with depth.
///
/// This is what makes voxel storage affordable. Without run-merging, a radius-20
/// world with bedrock depth would be tens of thousands of entities.
#[test]
fn entities_scale_with_runs_not_voxels() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let voxels: usize = {
        let map = app
            .world()
            .get_resource::<VoxelMap>()
            .expect("gameplay should have generated a world");
        map.columns()
            .map(|(_, column)| usize::try_from(column.top()).unwrap_or(0))
            .sum()
    };

    let tiles = tile_count(&mut app);
    assert!(
        tiles < voxels,
        "{tiles} entities for {voxels} voxels — runs are not being merged"
    );
}

#[test]
fn one_hundred_idle_frames_do_not_rebuild_or_republish_terrain() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let grid_before = app
        .world_mut()
        .query_filtered::<Entity, With<HexGrid>>()
        .single(app.world())
        .expect("the grid should exist");
    let tiles_before: BTreeSet<_> = {
        let mut tiles = app.world_mut().query_filtered::<Entity, With<HexTile>>();
        tiles.iter(app.world()).collect()
    };
    app.world_mut().clear_trackers();

    for _ in 0..100 {
        app.update();
    }

    let grid_after = app
        .world_mut()
        .query_filtered::<Entity, With<HexGrid>>()
        .single(app.world())
        .expect("the grid should still exist");
    let tiles_after: BTreeSet<_> = {
        let mut tiles = app.world_mut().query_filtered::<Entity, With<HexTile>>();
        tiles.iter(app.world()).collect()
    };
    assert_eq!(grid_after, grid_before);
    assert_eq!(
        tiles_after, tiles_before,
        "idle terrain reconciliation replaced unchanged run entities"
    );
    assert!(
        !app.world().resource_ref::<VoxelMap>().is_changed(),
        "an empty terrain-edit stream marked voxel storage changed"
    );
    assert!(
        !app.world()
            .resource_ref::<SpecialMovementRegions>()
            .is_changed(),
        "an empty terrain-edit stream marked special regions changed"
    );
}

#[test]
fn building_into_non_root_visual_cell_retires_the_complete_cave_vegetation_feature() {
    let mut app = v3_caves_app();
    enter_gameplay(&mut app);
    let before = cave_vegetation_instances(&mut app);
    let (root, visual) = cave_vegetation_non_root_visual(&mut app);
    let stone = app
        .world()
        .resource::<SubstanceTable>()
        .id("stone")
        .expect("the fixture substance table should contain stone");
    assert!(app.world().resource::<VoxelMap>().get(visual).is_air());

    app.world_mut().write_message(TerrainEdit::Set {
        pos: visual,
        substance: stone,
    });
    app.update();
    app.update();

    assert_eq!(app.world().resource::<VoxelMap>().get(visual), stone);
    let after = cave_vegetation_instances(&mut app);
    assert_eq!(after.len(), before.len().saturating_sub(1));
    assert!(!after.contains_key(&root));
}
