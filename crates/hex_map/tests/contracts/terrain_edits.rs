use super::*;

#[derive(Clone, Copy)]
enum MalformedChunkTopology {
    Missing,
    Orphan,
    WrongParent,
    Duplicate,
    Unexpected,
}

fn assert_chunk_topology_fails_closed(kind: MalformedChunkTopology) {
    let mut app = test_app();
    enter_gameplay(&mut app);
    let target = {
        let (coord, run) =
            diggable_run(&app, 1).expect("the authored map should expose one diggable edit target");
        TilePos::new(coord, run.top - 1)
    };
    let grid = app
        .world_mut()
        .query_filtered::<Entity, With<HexGrid>>()
        .single(app.world())
        .expect("the active terrain grid should be unique");
    let roots = terrain_chunk_roots(&mut app);
    let (&key, &existing) = roots
        .iter()
        .next()
        .expect("the generated map should publish chunk roots");

    match kind {
        MalformedChunkTopology::Missing => {
            app.world_mut().entity_mut(existing).despawn();
        }
        MalformedChunkTopology::Orphan => {
            app.world_mut().spawn(TerrainChunkRoot { q: 99, r: 99 });
        }
        MalformedChunkTopology::WrongParent => {
            let parent = app.world_mut().spawn_empty().id();
            let root = app
                .world_mut()
                .spawn(TerrainChunkRoot { q: 99, r: 99 })
                .id();
            app.world_mut().entity_mut(parent).add_child(root);
        }
        MalformedChunkTopology::Duplicate => {
            let root = app
                .world_mut()
                .spawn(TerrainChunkRoot { q: key.0, r: key.1 })
                .id();
            app.world_mut().entity_mut(grid).add_child(root);
        }
        MalformedChunkTopology::Unexpected => {
            let root = app
                .world_mut()
                .spawn(TerrainChunkRoot { q: 99, r: 99 })
                .id();
            app.world_mut().entity_mut(grid).add_child(root);
        }
    }

    app.world_mut()
        .write_message(TerrainEdit::Clear { pos: target });
    app.update();

    assert!(!app.world().contains_resource::<TerrainReady>());
    let failure = app
        .world()
        .resource::<GameplaySetupFailure>()
        .reason
        .as_str();
    assert!(
        failure.contains("chunk"),
        "malformed chunk topology should publish an exact failure: {failure}"
    );
}

#[test]
fn terrain_edits_reject_every_malformed_chunk_root_topology() {
    for kind in [
        MalformedChunkTopology::Missing,
        MalformedChunkTopology::Orphan,
        MalformedChunkTopology::WrongParent,
        MalformedChunkTopology::Duplicate,
        MalformedChunkTopology::Unexpected,
    ] {
        assert_chunk_topology_fails_closed(kind);
    }
}

#[test]
fn terrain_edit_replaces_only_the_affected_chunk_mesh_batches() {
    let mut app = test_app();
    enter_gameplay(&mut app);
    let (coord, run) = {
        let world = app.world();
        let map = world.resource::<VoxelMap>();
        let table = world.resource::<SubstanceTable>();
        map.columns()
            .find_map(|(coord, column)| {
                let crosses_chunk_seam = coord.neighbors().into_iter().any(|neighbour| {
                    map.column(neighbour).is_some()
                        && terrain_chunk_key(neighbour) != terrain_chunk_key(coord)
                });
                if !crosses_chunk_seam {
                    return None;
                }
                hex_map::runs(column)
                    .into_iter()
                    .find(|run| table.is_diggable(run.substance))
                    .map(|run| (coord, run))
            })
            .expect("the map should expose a diggable run on a chunk seam")
    };
    let target = TilePos::new(coord, run.top - 1);
    let roots_before = terrain_chunk_roots(&mut app);
    let before = terrain_render_batches_by_chunk(&mut app);
    let meshes_before = terrain_render_meshes_by_chunk(&mut app);
    let affected = terrain_chunk_key(coord);
    assert!(
        coord.neighbors().into_iter().any(|neighbour| {
            before.contains_key(&terrain_chunk_key(neighbour))
                && terrain_chunk_key(neighbour) != affected
        }),
        "the fixture should exercise a cross-chunk mesh dependency"
    );
    let mesh_asset_count_before = app.world().resource::<Assets<Mesh>>().len();

    app.world_mut()
        .write_message(TerrainEdit::Clear { pos: target });
    app.update();
    app.update();

    let roots_after = terrain_chunk_roots(&mut app);
    assert_ne!(roots_before.get(&affected), roots_after.get(&affected));
    for (chunk, root) in &roots_before {
        if *chunk != affected {
            assert_eq!(roots_after.get(chunk), Some(root));
        }
    }

    let after = terrain_render_batches_by_chunk(&mut app);
    let meshes_after = terrain_render_meshes_by_chunk(&mut app);
    assert_ne!(before.get(&affected), after.get(&affected));
    for (chunk, entities) in &before {
        if *chunk != affected {
            assert_eq!(after.get(chunk), Some(entities));
            assert_eq!(meshes_after.get(chunk), meshes_before.get(chunk));
        }
    }
    for retired in meshes_before.get(&affected).into_iter().flatten() {
        assert!(
            app.world()
                .resource::<Assets<Mesh>>()
                .get(*retired)
                .is_none(),
            "the affected chunk retained a retired combined mesh asset"
        );
    }
    let mesh_asset_count_after = app.world().resource::<Assets<Mesh>>().len();
    assert!(
        mesh_asset_count_after <= mesh_asset_count_before.saturating_add(1),
        "retired chunk meshes leaked instead of being replaced: before={mesh_asset_count_before}, after={mesh_asset_count_after}"
    );
}

#[test]
fn v3_forest_protects_feature_roots_and_rebuilds_them_deterministically() {
    let mut app = v3_forest_app();
    enter_gameplay(&mut app);

    let initial_roots: BTreeMap<_, _> = feature_roots(&mut app)
        .into_iter()
        .map(|(entity, _kind, position, _parent)| (position, entity))
        .collect();
    let tree_root = app
        .world()
        .resource::<TraversalBlockers>()
        .iter()
        .next()
        .expect("Forest should publish a tree blocker");
    let original_substance = app.world().resource::<VoxelMap>().get(tree_root);
    let first_grid = app
        .world_mut()
        .query_filtered::<Entity, With<HexGrid>>()
        .single(app.world())
        .expect("Forest grid should exist");
    let initial_chunks = terrain_chunk_roots(&mut app);

    app.world_mut()
        .write_message(TerrainEdit::Clear { pos: tree_root });
    app.update();
    app.update();

    assert_eq!(
        app.world().resource::<VoxelMap>().get(tree_root),
        original_substance,
        "static feature support was edited without feature reprojection"
    );
    let unchanged_grid = app
        .world_mut()
        .query_filtered::<Entity, With<HexGrid>>()
        .single(app.world())
        .expect("Forest grid should remain");
    assert_eq!(unchanged_grid, first_grid, "rejected edit rebuilt the grid");
    let unchanged_roots: BTreeMap<_, _> = feature_roots(&mut app)
        .into_iter()
        .map(|(entity, _kind, position, _parent)| (position, entity))
        .collect();
    assert_eq!(unchanged_roots, initial_roots);

    let blocker_coords: BTreeSet<_> = app
        .world()
        .resource::<TraversalBlockers>()
        .iter()
        .map(|position| position.coord)
        .collect();
    let unrelated = {
        let world = app.world();
        let table = world.resource::<SubstanceTable>();
        world
            .resource::<VoxelMap>()
            .columns()
            .filter(|(coord, _column)| !blocker_coords.contains(coord))
            .find_map(|(coord, column)| {
                hex_map::runs(column)
                    .into_iter()
                    .rev()
                    .find(|run| table.is_solid(run.substance) && table.is_diggable(run.substance))
                    .map(|run| TilePos::new(coord, run.top - 1))
            })
            .expect("Forest should have unrelated diggable terrain")
    };
    app.world_mut()
        .write_message(TerrainEdit::Clear { pos: unrelated });
    app.update();
    app.update();

    let edited_grid = app
        .world_mut()
        .query_filtered::<Entity, With<HexGrid>>()
        .single(app.world())
        .expect("edited Forest grid should exist");
    assert_eq!(edited_grid, first_grid);
    let edited_roots: BTreeMap<_, _> = feature_roots(&mut app)
        .into_iter()
        .map(|(entity, _kind, position, parent)| {
            assert_eq!(parent, edited_grid);
            (position, entity)
        })
        .collect();
    assert_eq!(edited_roots, initial_roots);
    let edited_chunks = terrain_chunk_roots(&mut app);
    let affected = terrain_chunk_key(unrelated.coord);
    assert_ne!(edited_chunks.get(&affected), initial_chunks.get(&affected));
    let retired_root = initial_chunks
        .get(&affected)
        .copied()
        .expect("the edited column should have an original chunk root");
    assert!(
        app.world().get_entity(retired_root).is_err(),
        "the replaced Forest chunk root remained alive"
    );
    assert!(initial_chunks
        .iter()
        .all(|(chunk, entity)| { *chunk == affected || edited_chunks.get(chunk) == Some(entity) }));

    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Title);
    app.update();
    app.update();
    assert!(feature_roots(&mut app).is_empty());
}

#[test]
fn clearing_a_tagged_surface_prunes_its_exact_membership() {
    let mut app = sky_islands_app();
    enter_gameplay(&mut app);
    let target = app
        .world()
        .resource::<SpecialMovementRegions>()
        .iter()
        .map(|(position, _)| position)
        .next()
        .expect("sky islands should publish optional surfaces");

    app.world_mut()
        .write_message(TerrainEdit::Clear { pos: target });
    app.update();
    app.update();

    assert_eq!(
        app.world().resource::<SpecialMovementRegions>().get(target),
        None
    );
}

#[test]
fn terrain_edits_prune_stale_interior_floor_and_roof_voxel_metadata() {
    let mut app = test_app();
    enter_gameplay(&mut app);
    let target = {
        let world = app.world_mut();
        let table = world.resource::<SubstanceTable>().clone();
        let mut tiles = world.query::<(&TilePos, &SubstanceId, &Headroom)>();
        tiles
            .iter(world)
            .find(|(_, substance, headroom)| table.is_diggable(**substance) && headroom.0 >= 2)
            .map(|(position, _, _)| *position)
            .expect("the authored map should have a clearable exposed surface")
    };
    let region = InteriorRegionId(4);
    let mut interiors = InteriorRegions::new();
    interiors.insert_surface(target, region);
    interiors.insert_roof_voxel(target, region);
    app.insert_resource(interiors);

    app.world_mut()
        .write_message(TerrainEdit::Clear { pos: target });
    app.update();
    app.update();

    let interiors = app.world().resource::<InteriorRegions>();
    assert_eq!(interiors.get(target), None);
    assert_eq!(interiors.roof_region(target), None);
}

#[test]
fn splitting_a_roof_reprojects_cutaway_onto_both_remaining_runs() {
    let mut app = test_app();
    enter_gameplay(&mut app);
    let (coord, roof) =
        diggable_run(&app, 3).expect("the authored map should contain a tall diggable run");
    let region = InteriorRegionId(8);
    install_roof_metadata(&mut app, coord, roof, region);
    let split_level = roof.bottom + roof.levels() / 2;

    app.world_mut().write_message(TerrainEdit::Clear {
        pos: TilePos::new(coord, split_level),
    });
    app.update();
    app.update();

    let interiors = app.world().resource::<InteriorRegions>();
    assert_eq!(
        interiors.roof_region(TilePos::new(coord, split_level)),
        None
    );
    assert_eq!(
        interiors.roof_region(TilePos::new(coord, split_level - 1)),
        Some(region)
    );
    assert_eq!(
        interiors.roof_region(TilePos::new(coord, roof.top - 1)),
        Some(region)
    );

    let world = app.world_mut();
    let mut tiles = world.query::<(&HexCoord, &TilePos, Option<&CutawayOccluder>)>();
    let projected: HashMap<TilePos, Option<InteriorRegionId>> = tiles
        .iter(world)
        .filter(|(tile_coord, _, _)| **tile_coord == coord)
        .map(|(_, position, cutaway)| (*position, cutaway.map(|tag| tag.0)))
        .collect();
    assert_eq!(
        projected.get(&TilePos::new(coord, split_level - 1)),
        Some(&Some(region)),
        "the lower roof fragment lost its cutaway projection"
    );
    assert_eq!(
        projected.get(&TilePos::new(coord, roof.top - 1)),
        Some(&Some(region)),
        "the upper roof fragment lost its cutaway projection"
    );
}

#[test]
fn replacing_roof_material_does_not_transfer_its_cutaway_tag() {
    let mut app = test_app();
    enter_gameplay(&mut app);
    let (coord, roof) =
        diggable_run(&app, 2).expect("the authored map should contain a tall diggable run");
    let region = InteriorRegionId(9);
    install_roof_metadata(&mut app, coord, roof, region);
    let replaced = TilePos::new(coord, roof.top - 1);
    let replacement = app
        .world()
        .resource::<SubstanceTable>()
        .id("metal")
        .expect("the test substance table should contain metal");
    assert_ne!(replacement, roof.substance);

    app.world_mut().write_message(TerrainEdit::Set {
        pos: replaced,
        substance: replacement,
    });
    app.update();
    app.update();

    let interiors = app.world().resource::<InteriorRegions>();
    assert_eq!(interiors.roof_region(replaced), None);
    assert_eq!(
        interiors.roof_region(TilePos::new(coord, roof.top - 2)),
        Some(region)
    );

    let world = app.world_mut();
    let mut tiles = world.query::<(&HexCoord, &TilePos, &SubstanceId, Option<&CutawayOccluder>)>();
    let replacement_run = tiles
        .iter(world)
        .find(|(tile_coord, position, _, _)| **tile_coord == coord && **position == replaced)
        .expect("the replacement material should render as its own run");
    assert_eq!(*replacement_run.2, replacement);
    assert_eq!(
        replacement_run.3, None,
        "replacement material inherited a stale cutaway tag"
    );

    let remaining_roof = TilePos::new(coord, roof.top - 2);
    let original_run = tiles
        .iter(world)
        .find(|(tile_coord, position, _, _)| **tile_coord == coord && **position == remaining_roof)
        .expect("the original roof material should remain rendered");
    assert_eq!(*original_run.2, roof.substance);
    assert_eq!(
        original_run.3.map(|tag| tag.0),
        Some(region),
        "the remaining roof run lost its cutaway tag"
    );
}

#[test]
fn v3_waterfall_rejects_liquid_and_support_edits_but_rebuilds_dry_terrain() {
    let mut app = v3_waterfall_app();
    enter_gameplay(&mut app);

    let (water_position, support_position, dry_position, water, support) = {
        let world = app.world();
        let map = world.resource::<VoxelMap>();
        let table = world.resource::<SubstanceTable>();
        let water = table.id("water").expect("water should exist");
        let water_position = map
            .columns()
            .find_map(|(coord, column)| {
                column.iter().enumerate().find_map(|(index, substance)| {
                    (substance == water).then(|| {
                        TilePos::new(coord, i32::try_from(index).expect("test levels fit in i32"))
                    })
                })
            })
            .expect("Waterfall should contain authored water");
        let support_position =
            TilePos::new(water_position.coord, water_position.level.saturating_sub(1));
        let support = map.get(support_position);
        let dry_position = world
            .resource::<BiomeRegions>()
            .iter()
            .map(|(position, _region)| position)
            .find(|position| {
                position.coord.y().abs() > 3
                    && table.is_solid(map.get(*position))
                    && table.is_diggable(map.get(*position))
            })
            .expect("Waterfall should contain a classified dry diggable surface");
        (
            water_position,
            support_position,
            dry_position,
            water,
            support,
        )
    };
    assert!(!support.is_air());

    let original_grid = app
        .world_mut()
        .query_filtered::<Entity, With<HexGrid>>()
        .single(app.world())
        .expect("Waterfall grid should exist");
    for protected in [water_position, support_position] {
        app.world_mut()
            .write_message(TerrainEdit::Clear { pos: protected });
        app.update();
        app.update();
        let current_grid = app
            .world_mut()
            .query_filtered::<Entity, With<HexGrid>>()
            .single(app.world())
            .expect("ignored edit should preserve the grid");
        assert_eq!(current_grid, original_grid);
    }
    assert_eq!(
        app.world().resource::<VoxelMap>().get(water_position),
        water
    );
    assert_eq!(
        app.world().resource::<VoxelMap>().get(support_position),
        support
    );

    app.world_mut()
        .write_message(TerrainEdit::Clear { pos: dry_position });
    app.update();
    app.update();
    let edited_grid = app
        .world_mut()
        .query_filtered::<Entity, With<HexGrid>>()
        .single(app.world())
        .expect("dry edit should retain the grid");
    assert_eq!(edited_grid, original_grid);
    assert!(app
        .world()
        .resource::<VoxelMap>()
        .get(dry_position)
        .is_air());
    assert!(
        app.world()
            .resource::<BiomeRegions>()
            .get(dry_position)
            .is_none(),
        "clearing a generated surface must remove its stale exact biome membership"
    );
}

#[test]
fn mountain_range_protects_the_shared_sea_and_republishes_it_after_a_dry_edit() {
    let mut app = v3_mountain_range_app();
    enter_gameplay(&mut app);

    let (water_position, support_position, cap_position, dry_position, water, sand, stone) = {
        let world = app.world();
        let map = world.resource::<VoxelMap>();
        let table = world.resource::<SubstanceTable>();
        let water = table.id("water").expect("water should exist");
        let sand = table.id("sand").expect("sand should exist");
        let stone = table.id("stone").expect("stone should exist");
        let blockers = world.resource::<TraversalBlockers>();
        let anchors = world.resource::<MapAnchors>();
        let water_position = map
            .columns()
            .filter_map(|(coord, column)| {
                (column.get(4) == sand && (5..=8).all(|level| column.get(level) == water))
                    .then_some(TilePos::new(coord, 8))
            })
            .min()
            .expect("Mountain Range should contain exact Shallow Sea strata");
        let dry_position = world
            .resource::<BiomeRegions>()
            .iter()
            .map(|(position, _region)| position)
            .filter(|position| position.level >= 60)
            .filter(|position| {
                table.is_solid(map.get(*position))
                    && table.is_diggable(map.get(*position))
                    && table.is_solid(map.get(position.below()))
                    && map.get(position.above()).is_air()
                    && map
                        .column(position.coord)
                        .is_some_and(|column| column.iter().all(|substance| substance != water))
                    && !blockers.contains(*position)
                    && anchors
                        .iter()
                        .all(|(_anchor, anchor_position)| anchor_position != *position)
            })
            .max()
            .expect("Mountain Range should expose editable high-massif terrain");
        (
            water_position,
            TilePos::new(water_position.coord, 4),
            water_position.above(),
            dry_position,
            water,
            sand,
            stone,
        )
    };
    let original_dry_region = app
        .world()
        .resource::<BiomeRegions>()
        .get(dry_position)
        .expect("the selected massif surface should publish a biome identity");
    let original_grid = app
        .world_mut()
        .query_filtered::<Entity, With<HexGrid>>()
        .single(app.world())
        .expect("Mountain Range grid should exist");
    let original_chunks = terrain_chunk_roots(&mut app);
    let original_presentations = liquid_presentations(&mut app);
    assert!(
        !original_presentations.is_empty(),
        "the shared watershed should publish liquid presentation"
    );

    for (edit, position, expected) in [
        (
            TerrainEdit::Clear {
                pos: water_position,
            },
            water_position,
            water,
        ),
        (
            TerrainEdit::Clear {
                pos: support_position,
            },
            support_position,
            sand,
        ),
        (
            TerrainEdit::Set {
                pos: cap_position,
                substance: stone,
            },
            cap_position,
            SubstanceId::AIR,
        ),
    ] {
        app.world_mut().write_message(edit);
        app.update();
        app.update();

        assert_eq!(
            app.world().resource::<VoxelMap>().get(position),
            expected,
            "the conservative shared-sea guard admitted an edit at {position:?}"
        );
        let unchanged_grid = app
            .world_mut()
            .query_filtered::<Entity, With<HexGrid>>()
            .single(app.world())
            .expect("a rejected sea edit should retain the grid");
        assert_eq!(
            unchanged_grid, original_grid,
            "a rejected sea edit rebuilt the grid"
        );
    }
    assert_eq!(
        liquid_presentations(&mut app),
        original_presentations,
        "rejected sea edits should not disturb runtime liquid publication"
    );

    app.world_mut()
        .write_message(TerrainEdit::Clear { pos: dry_position });
    app.update();
    app.update();

    let edited_grid = app
        .world_mut()
        .query_filtered::<Entity, With<HexGrid>>()
        .single(app.world())
        .expect("the edited Mountain Range grid should remain");
    assert_eq!(edited_grid, original_grid);
    let edited_chunks = terrain_chunk_roots(&mut app);
    let affected = terrain_chunk_key(dry_position.coord);
    assert_ne!(edited_chunks.get(&affected), original_chunks.get(&affected));
    let retired_root = original_chunks
        .get(&affected)
        .copied()
        .expect("the edited column should have an original chunk root");
    assert!(
        app.world().get_entity(retired_root).is_err(),
        "the replaced Mountain Range chunk root remained alive"
    );
    assert!(original_chunks
        .iter()
        .all(|(chunk, entity)| { *chunk == affected || edited_chunks.get(chunk) == Some(entity) }));
    assert!(
        app.world()
            .resource::<VoxelMap>()
            .get(dry_position)
            .is_air(),
        "the unrelated massif edit should be accepted"
    );
    assert_eq!(
        app.world().resource::<BiomeRegions>().get(dry_position),
        None,
        "the cleared exact surface retained stale biome membership"
    );
    assert_eq!(
        app.world()
            .resource::<BiomeRegions>()
            .get(dry_position.below()),
        Some(original_dry_region),
        "the newly exposed massif surface did not inherit its biome identity"
    );
    assert_column_run_publication(&mut app, dry_position.coord);

    assert_eq!(liquid_presentations(&mut app), original_presentations);
    assert!(app.world().contains_resource::<TerrainReady>());
    assert!(!app.world().contains_resource::<GameplaySetupFailure>());
}

/// Headroom under a platform is the size of the gap, not open sky.
///
/// This is what makes a body's size mean anything: build a roof two levels up and the
/// ground below reports 2, so a three-level body no longer fits there. Without this,
/// every surface would look infinitely tall and overhangs would be free to walk under.
#[test]
fn a_platform_overhead_reduces_the_headroom_below() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let coord = HexCoord::ORIGIN;
    let (surface, stone) = {
        let world = app.world();
        let map = world
            .get_resource::<VoxelMap>()
            .expect("a world should exist");
        let table = world
            .get_resource::<SubstanceTable>()
            .expect("a substance table should exist");
        (
            map.surface(coord).expect("the origin should have ground"),
            table.id("stone").expect("stone should be defined"),
        )
    };

    // A roof three levels above the surface leaves exactly two clear voxels between.
    let gap = 2;
    app.world_mut().write_message(TerrainEdit::Set {
        pos: TilePos::new(coord, surface + gap + 1),
        substance: stone,
    });
    app.update();
    app.update();

    assert_column_run_publication(&mut app, coord);
    let published = published_run_bounds(&mut app, coord);
    assert!(
        published.contains(&(surface + gap + 1, surface + gap + 1, stone)),
        "the one-voxel platform must publish its exact bottom and inclusive top"
    );
    assert!(
        published.iter().any(|(_, top, _)| *top == surface),
        "the ground run below the platform must retain its own published bounds"
    );

    let mut query = app
        .world_mut()
        .query_filtered::<(&TilePos, &Headroom), With<HexTile>>();
    let headroom = query
        .iter(app.world())
        .find(|(pos, _)| pos.coord == coord && pos.level == surface)
        .map(|(_, headroom)| headroom.0)
        .expect("the original surface should still be a tile");

    assert_eq!(
        headroom, gap,
        "the ground under a platform should report the gap, not open sky"
    );
}

/// Digging a voxel out of the middle of a run splits it in two, which is what makes
/// caves and tunnels fall out of the same mechanism as everything else.
///
/// The run has to be at least three levels deep. Clearing a run that is only one
/// voxel tall *removes* it rather than splitting it, so entity count goes down — a
/// first version of this test picked an arbitrary level, hit a single-voxel dirt
/// band, and failed with 156 -> 155.
#[test]
fn clearing_a_voxel_splits_a_run() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let before = tile_count(&mut app);

    // Find a run thick enough that hollowing its middle leaves material either side.
    let (target, original_run) = {
        let map = app
            .world()
            .get_resource::<VoxelMap>()
            .expect("a world should exist");
        map.columns()
            .find_map(|(coord, column)| {
                hex_map::runs(column)
                    .into_iter()
                    .find(|run| run.levels() >= 3)
                    .map(|run| (TilePos::new(coord, run.bottom + 1), run))
            })
            .expect("generated terrain should contain at least one run three levels deep")
    };

    app.world_mut()
        .write_message(TerrainEdit::Clear { pos: target });
    app.update();
    app.update();

    let after = tile_count(&mut app);
    let map = app
        .world()
        .get_resource::<VoxelMap>()
        .expect("a world should exist");

    assert!(map.get(target).is_air(), "the dug voxel should be air");
    assert!(
        !map.get(target.below()).is_air(),
        "material below the hole should survive"
    );
    assert!(
        !map.get(target.above()).is_air(),
        "material above the hole should survive"
    );
    assert_eq!(
        after,
        before + 1,
        "splitting one run into two should add exactly one entity"
    );

    assert_column_run_publication(&mut app, target.coord);
    let published = published_run_bounds(&mut app, target.coord);
    assert!(
        published.contains(&(
            original_run.bottom,
            target.level - 1,
            original_run.substance
        )),
        "the lower cave wall fragment must publish the original run bottom"
    );
    assert!(
        published.contains(&(
            target.level + 1,
            original_run.top - 1,
            original_run.substance
        )),
        "the overhanging fragment must publish the first material voxel above the cave"
    );
}

/// Neither digging nor replacement may remove the world's non-diggable floor.
#[test]
fn terrain_edits_preserve_non_diggable_bedrock() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let pos = TilePos::ORIGIN;
    let (bedrock, stone) = {
        let table = app
            .world()
            .get_resource::<SubstanceTable>()
            .expect("a substance table should exist");
        (
            table.id("bedrock").expect("bedrock should be defined"),
            table.id("stone").expect("stone should be defined"),
        )
    };
    assert_eq!(
        app.world()
            .get_resource::<VoxelMap>()
            .expect("a world should exist")
            .get(pos),
        bedrock,
        "the test target should begin as bedrock"
    );

    app.world_mut().write_message(TerrainEdit::Clear { pos });
    app.update();
    app.update();
    assert_eq!(
        app.world()
            .get_resource::<VoxelMap>()
            .expect("a world should exist")
            .get(pos),
        bedrock,
        "clearing must not remove non-diggable bedrock"
    );

    app.world_mut().write_message(TerrainEdit::Set {
        pos,
        substance: SubstanceId::AIR,
    });
    app.update();
    app.update();
    assert_eq!(
        app.world()
            .get_resource::<VoxelMap>()
            .expect("a world should exist")
            .get(pos),
        bedrock,
        "setting air must not remove non-diggable bedrock"
    );

    app.world_mut().write_message(TerrainEdit::Set {
        pos,
        substance: stone,
    });
    app.update();
    app.update();
    assert_eq!(
        app.world()
            .get_resource::<VoxelMap>()
            .expect("a world should exist")
            .get(pos),
        bedrock,
        "replacement must not overwrite non-diggable bedrock"
    );
}

/// Positions below the bedrock floor are outside the map and must not trigger work.
#[test]
fn terrain_edits_below_the_floor_are_ignored() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let grid_before = app
        .world_mut()
        .query_filtered::<Entity, With<HexGrid>>()
        .single(app.world())
        .expect("the grid should exist");
    let stone = app
        .world()
        .get_resource::<SubstanceTable>()
        .and_then(|table| table.id("stone"))
        .expect("stone should be defined");

    app.world_mut().write_message(TerrainEdit::Set {
        pos: TilePos::new(HexCoord::ORIGIN, -1),
        substance: stone,
    });
    app.update();
    app.update();

    let grid_after = app
        .world_mut()
        .query_filtered::<Entity, With<HexGrid>>()
        .single(app.world())
        .expect("the grid should still exist");
    assert_eq!(
        grid_after, grid_before,
        "an ignored edit should not rebuild the grid"
    );
}

#[test]
#[ignore = "manual release-mode localized terrain-edit stress gate"]
fn one_hundred_localized_terrain_edits_stay_within_the_interactive_budget() {
    let mut app = procedural_app();
    enter_gameplay(&mut app);

    let (target, original) = {
        let map = app.world().resource::<VoxelMap>();
        let table = app.world().resource::<SubstanceTable>();
        map.columns()
            .find_map(|(coord, column)| {
                let level = column.surface()?;
                let substance = column.get(level);
                table
                    .is_diggable(substance)
                    .then_some((TilePos::new(coord, level), substance))
            })
            .expect("radius-12 Hills should expose diggable surface terrain")
    };
    let meshes_before = app.world().resource::<Assets<Mesh>>().len();
    let materials_before = app.world().resource::<Assets<StandardMaterial>>().len();
    let mut samples = Vec::with_capacity(100);
    let mut created_entities = 0_usize;
    let mut maximum_created = 0_usize;
    let mut assets_after_warmup = None;

    for index in 0..100 {
        let before: BTreeSet<_> = {
            let mut tiles = app.world_mut().query_filtered::<Entity, With<HexTile>>();
            tiles.iter(app.world()).collect()
        };
        if index % 2 == 0 {
            app.world_mut()
                .write_message(TerrainEdit::Clear { pos: target });
        } else {
            app.world_mut().write_message(TerrainEdit::Set {
                pos: target,
                substance: original,
            });
        }

        let started = Instant::now();
        app.update();
        samples.push(started.elapsed());

        let after: BTreeSet<_> = {
            let mut tiles = app.world_mut().query_filtered::<Entity, With<HexTile>>();
            tiles.iter(app.world()).collect()
        };
        let created = after.difference(&before).count();
        created_entities = created_entities.saturating_add(created);
        maximum_created = maximum_created.max(created);
        let expected = if index % 2 == 0 {
            SubstanceId::AIR
        } else {
            original
        };
        assert_eq!(
            app.world().resource::<VoxelMap>().get(target),
            expected,
            "localized edit {index} did not settle in one update"
        );
        if index == 3 {
            assets_after_warmup = Some((
                app.world().resource::<Assets<Mesh>>().len(),
                app.world().resource::<Assets<StandardMaterial>>().len(),
            ));
        }
    }

    samples.sort_unstable();
    let p95 = samples
        .get(94)
        .copied()
        .expect("the terrain benchmark records exactly 100 samples");
    let worst = samples
        .get(99)
        .copied()
        .expect("the terrain benchmark records exactly 100 samples");
    let (meshes_after_warmup, materials_after_warmup) =
        assets_after_warmup.expect("the terrain benchmark completes its four-edit warmup");
    let meshes_after = app.world().resource::<Assets<Mesh>>().len();
    let materials_after = app.world().resource::<Assets<StandardMaterial>>().len();
    eprintln!(
        "radius-12 localized terrain edits: p95={p95:?}, worst={worst:?}, \
         created_total={created_entities}, max_created_per_edit={maximum_created}, \
         meshes={meshes_before}->{meshes_after_warmup}->{meshes_after}, \
         materials={materials_before}->{materials_after_warmup}->{materials_after}"
    );
    assert_eq!(
        meshes_after, meshes_after_warmup,
        "localized terrain edits kept allocating mesh assets after the warmup"
    );
    assert_eq!(
        materials_after, materials_after_warmup,
        "localized terrain edits kept allocating material assets after the warmup"
    );

    let (p95_budget, worst_budget) = if cfg!(debug_assertions) {
        (Duration::from_millis(100), Duration::from_millis(250))
    } else {
        (Duration::from_micros(16_700), Duration::from_millis(50))
    };
    assert!(
        p95 < p95_budget && worst < worst_budget,
        "localized terrain edits exceeded the interaction budget: p95={p95:?}, worst={worst:?}"
    );
}

/// Building above the surface leaves the space between as air — a floating platform.
#[test]
fn setting_a_voxel_above_the_surface_builds_a_platform() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let coord = HexCoord::ORIGIN;
    let (surface, stone) = {
        let world = app.world();
        let map = world
            .get_resource::<VoxelMap>()
            .expect("a world should exist");
        let table = world
            .get_resource::<SubstanceTable>()
            .expect("a substance table should exist");
        (
            map.surface(coord).expect("the origin should have ground"),
            table.id("stone").expect("stone should be defined"),
        )
    };

    let platform = TilePos::new(coord, surface + 4);
    app.world_mut().write_message(TerrainEdit::Set {
        pos: platform,
        substance: stone,
    });
    app.update();
    app.update();

    let map = app
        .world()
        .get_resource::<VoxelMap>()
        .expect("a world should exist");
    assert_eq!(map.get(platform), stone, "the platform should exist");
    assert!(
        map.get(TilePos::new(coord, surface + 2)).is_air(),
        "the space beneath a floating platform stays empty"
    );
}

#[test]
fn terrain_edits_retire_cave_vegetation_with_invalidated_support() {
    let mut app = v3_caves_app();
    enter_gameplay(&mut app);
    let before = cave_vegetation_instances(&mut app);
    let target = *before
        .keys()
        .next()
        .expect("Caves should publish sparse vegetation");

    app.world_mut()
        .write_message(TerrainEdit::Clear { pos: target });
    app.update();
    app.update();

    let after = cave_vegetation_instances(&mut app);
    assert_eq!(after.len(), before.len().saturating_sub(1));
    assert!(!after.contains_key(&target));
}

#[test]
fn clearing_non_root_support_retires_the_complete_cave_vegetation_feature() {
    let mut app = v3_caves_app();
    enter_gameplay(&mut app);
    let before = cave_vegetation_instances(&mut app);
    let (root, visual) = cave_vegetation_non_root_visual(&mut app);
    let support = TilePos::new(visual.coord, root.level);
    assert!(app
        .world()
        .resource::<SubstanceTable>()
        .is_diggable(app.world().resource::<VoxelMap>().get(support)));

    app.world_mut()
        .write_message(TerrainEdit::Clear { pos: support });
    app.update();
    app.update();

    assert!(app.world().resource::<VoxelMap>().get(support).is_air());
    let after = cave_vegetation_instances(&mut app);
    assert_eq!(after.len(), before.len().saturating_sub(1));
    assert!(!after.contains_key(&root));
}
