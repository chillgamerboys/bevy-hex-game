use super::*;

#[test]
fn liquid_presentation_is_additive_non_pickable_and_tracks_grid_lifecycle() {
    let mut app = procedural_app();
    enter_gameplay(&mut app);

    let expected_tiles: usize = app
        .world()
        .resource::<VoxelMap>()
        .columns()
        .map(|(_coord, column)| hex_map::runs(column).len())
        .sum();
    assert_eq!(
        tile_count(&mut app),
        expected_tiles,
        "presentation geometry changed the authoritative tile count"
    );

    let first_grid = app
        .world_mut()
        .query_filtered::<Entity, With<HexGrid>>()
        .single(app.world())
        .expect("the first grid should exist");
    let first_chunks = terrain_chunk_roots(&mut app);
    let first_presentations = liquid_presentations(&mut app);
    assert!(
        !first_presentations.is_empty(),
        "the procedural river should produce presentation caps"
    );
    assert!(first_presentations
        .iter()
        .all(|(_entity, parent, pickable)| *parent == first_grid && *pickable == Pickable::IGNORE));

    let solid_edit = {
        let world = app.world();
        let table = world.resource::<SubstanceTable>();
        world
            .resource::<VoxelMap>()
            .columns()
            .find_map(|(coord, column)| {
                hex_map::runs(column)
                    .into_iter()
                    .find(|run| table.is_solid(run.substance) && table.is_diggable(run.substance))
                    .map(|run| TilePos::new(coord, run.top - 1))
            })
            .expect("the generated map should contain diggable solid terrain")
    };
    app.world_mut()
        .write_message(TerrainEdit::Clear { pos: solid_edit });
    app.update();
    app.update();

    let second_grid = app
        .world_mut()
        .query_filtered::<Entity, With<HexGrid>>()
        .single(app.world())
        .expect("the edited grid should exist");
    assert_eq!(second_grid, first_grid);
    let second_chunks = terrain_chunk_roots(&mut app);
    let affected = terrain_chunk_key(solid_edit.coord);
    assert_ne!(second_chunks.get(&affected), first_chunks.get(&affected));
    let retired_root = first_chunks
        .get(&affected)
        .copied()
        .expect("the edited column should have an original chunk root");
    assert!(
        app.world().get_entity(retired_root).is_err(),
        "the replaced chunk root remained alive"
    );
    assert!(first_chunks
        .iter()
        .all(|(chunk, entity)| { *chunk == affected || second_chunks.get(chunk) == Some(entity) }));
    let second_presentations = liquid_presentations(&mut app);
    assert_eq!(second_presentations, first_presentations);

    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Title);
    app.update();
    app.update();
    assert!(liquid_presentations(&mut app).is_empty());
}

#[test]
fn the_grid_has_a_single_parent() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let grids = app
        .world_mut()
        .query_filtered::<Entity, With<HexGrid>>()
        .iter(app.world())
        .count();
    assert_eq!(grids, 1, "tiles should hang off exactly one grid entity");
}

/// Logical terrain runs are authoritative facts, not scene or picking entities.
///
/// World placement remains exactly reconstructible from the public coordinate/span
/// tuple, while the bounded render batch owns the actual transform and visibility.
/// This prevents large worlds from feeding every material run through Bevy's scene
/// propagation and culling systems.
#[test]
fn logical_tiles_are_scene_free_and_retain_exact_world_geometry() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let mut query = app.world_mut().query_filtered::<(
        &TilePos,
        &HexSpan,
        Option<&Transform>,
        Option<&GlobalTransform>,
        Option<&Visibility>,
        Option<&InheritedVisibility>,
        Option<&ViewVisibility>,
        Option<&Pickable>,
        Option<&Mesh3d>,
        Option<&MeshMaterial3d<StandardMaterial>>,
        &ChildOf,
    ), With<HexTile>>();

    let mut checked = 0;
    let mut logical_roots = BTreeSet::new();
    for (
        position,
        span,
        transform,
        global,
        visibility,
        inherited,
        view,
        pickable,
        mesh,
        material,
        parent,
    ) in query.iter(app.world())
    {
        assert!(
            transform.is_none(),
            "logical run entered transform propagation"
        );
        assert!(
            global.is_none(),
            "logical run entered global-transform propagation"
        );
        assert!(
            visibility.is_none() && inherited.is_none() && view.is_none(),
            "logical run entered visibility propagation or culling"
        );
        assert!(
            pickable.is_none(),
            "logical run entered the picking backend"
        );
        assert!(mesh.is_none(), "logical run still owns a draw mesh");
        assert!(material.is_none(), "logical run still owns a PBR material");
        let centre = position.coord.to_world(span.centre());
        assert!(
            centre.is_finite() && span.height().is_finite() && span.height() > 0.0,
            "logical run no longer reconstructs finite positive world geometry"
        );
        logical_roots.insert(parent.parent());
        checked += 1;
    }
    assert!(checked > 0, "no tiles were checked");
    assert!(!logical_roots.is_empty());
    for root in logical_roots {
        let owner = app.world().entity(root);
        assert!(owner.get::<Transform>().is_none());
        assert!(owner.get::<GlobalTransform>().is_none());
        assert!(owner.get::<Visibility>().is_none());
        assert!(owner.get::<InheritedVisibility>().is_none());
        assert!(owner.get::<ViewVisibility>().is_none());
        let chunk = owner
            .get::<ChildOf>()
            .expect("logical-run owner should preserve recursive chunk lifecycle")
            .parent();
        assert!(app.world().get::<TerrainChunkRoot>(chunk).is_some());
    }
}

/// Exact run entities remain gameplay's stable projection but no longer each own a
/// PBR draw. Every run appears in exactly one bounded, pickable chunk mesh instead.
#[test]
fn terrain_runs_are_lightweight_and_render_batches_cover_them_exactly_once() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let logical = {
        let world = app.world_mut();
        let mut tiles = world.query_filtered::<(
            Entity,
            &SubstanceId,
            Option<&Transform>,
            Option<&Visibility>,
            Option<&Pickable>,
            Option<&Mesh3d>,
            Option<&MeshMaterial3d<StandardMaterial>>,
        ), With<HexTile>>();
        tiles
            .iter(world)
            .map(
                |(entity, substance, transform, visibility, pickable, mesh, material)| {
                    assert!(transform.is_none());
                    assert!(visibility.is_none());
                    assert!(pickable.is_none());
                    assert!(mesh.is_none(), "logical terrain run still owns a draw mesh");
                    assert!(
                        material.is_none(),
                        "logical terrain run still owns a PBR material"
                    );
                    (entity, *substance)
                },
            )
            .collect::<BTreeMap<_, _>>()
    };
    assert!(!logical.is_empty());

    let mut represented = BTreeSet::new();
    let mut batch_count = 0usize;
    {
        let world = app.world_mut();
        let mut batches = world.query::<(
            &TerrainRenderBatch,
            &Mesh3d,
            &MeshMaterial3d<StandardMaterial>,
            &Pickable,
            &ChildOf,
        )>();
        for (batch, _mesh, _material, pickable, parent) in batches.iter(world) {
            assert_eq!(*pickable, Pickable::default());
            assert!(
                batch.runs().len() <= 512,
                "terrain batch exceeded its bound"
            );
            let chunk = world
                .get::<TerrainChunkRoot>(parent.parent())
                .expect("every terrain batch should belong to one chunk root");
            assert_eq!(batch.chunk(), *chunk);
            for run in batch.runs() {
                assert_eq!(
                    logical.get(&run.entity()),
                    Some(&batch.substance()),
                    "a combined batch mixed logical runs from another substance"
                );
                assert!(
                    represented.insert(run.entity()),
                    "one logical run appeared in multiple terrain batches"
                );
            }
            batch_count += 1;
        }
    }

    assert_eq!(represented, logical.keys().copied().collect());
    assert!(batch_count > 0);
    assert!(
        batch_count < logical.len(),
        "batching did not reduce terrain draw cardinality"
    );
}

#[test]
fn cutaway_runs_and_their_render_batches_share_exact_ownership() {
    let mut app = test_app();
    enter_gameplay(&mut app);
    let (coord, roof) =
        diggable_run(&app, 1).expect("the fixture should contain one diggable terrain run");
    let (edit_target, replacement) = {
        let world = app.world();
        let map = world.resource::<VoxelMap>();
        let table = world.resource::<SubstanceTable>();
        let metal = table
            .id("metal")
            .expect("the fixture substance table should contain metal");
        map.columns()
            .filter(|(candidate, _column)| {
                *candidate != coord && terrain_chunk_key(*candidate) == terrain_chunk_key(coord)
            })
            .find_map(|(candidate, column)| {
                hex_map::runs(column)
                    .into_iter()
                    .find(|run| table.is_diggable(run.substance) && run.substance != metal)
                    .map(|run| (TilePos::new(candidate, run.top - 1), metal))
            })
            .expect("the roof chunk should contain another editable terrain run")
    };
    let region = InteriorRegionId(17);
    install_roof_metadata(&mut app, coord, roof, region);
    // Interior metadata is normally present before initial presentation. This
    // focused fixture installs it after generation, then exercises the same exact
    // chunk replacement path used when roof ownership changes after an edit.
    app.world_mut().write_message(TerrainEdit::Set {
        pos: edit_target,
        substance: replacement,
    });
    app.update();
    app.update();

    let world = app.world_mut();
    let logical_ownership = {
        let mut tiles = world.query_filtered::<(Entity, &CutawayOccluder), With<HexTile>>();
        tiles
            .iter(world)
            .map(|(entity, cutaway)| (entity, *cutaway))
            .collect::<BTreeMap<_, _>>()
    };
    assert!(!logical_ownership.is_empty());

    let mut covered = BTreeSet::new();
    let mut batches = world.query::<(&TerrainRenderBatch, Option<&CutawayOccluder>)>();
    for (batch, cutaway) in batches.iter(world) {
        for run in batch.runs() {
            if let Some(expected) = logical_ownership.get(&run.entity()) {
                assert_eq!(cutaway.copied(), Some(*expected));
                covered.insert(run.entity());
            }
        }
    }
    assert_eq!(covered, logical_ownership.keys().copied().collect());
}

#[test]
fn terrain_batch_mesh_assets_are_released_on_teardown_and_rebuilt_on_reentry() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let first_meshes = {
        let world = app.world_mut();
        let mut batches = world.query_filtered::<&Mesh3d, With<TerrainRenderBatch>>();
        batches
            .iter(world)
            .map(|mesh| mesh.0.id())
            .collect::<BTreeSet<_>>()
    };
    assert!(!first_meshes.is_empty());
    assert!(first_meshes.iter().all(|id| app
        .world()
        .resource::<Assets<Mesh>>()
        .get(*id)
        .is_some()));

    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Title);
    app.update();
    app.update();

    assert_eq!(
        app.world_mut()
            .query_filtered::<Entity, With<TerrainRenderBatch>>()
            .iter(app.world())
            .count(),
        0,
        "teardown retained terrain batch entities"
    );
    assert!(first_meshes.iter().all(|id| app
        .world()
        .resource::<Assets<Mesh>>()
        .get(*id)
        .is_none()));

    enter_gameplay(&mut app);
    let second_meshes = {
        let world = app.world_mut();
        let mut batches = world.query_filtered::<&Mesh3d, With<TerrainRenderBatch>>();
        batches
            .iter(world)
            .map(|mesh| mesh.0.id())
            .collect::<BTreeSet<_>>()
    };
    assert_eq!(second_meshes.len(), first_meshes.len());
    assert!(second_meshes.iter().all(|id| app
        .world()
        .resource::<Assets<Mesh>>()
        .get(*id)
        .is_some()));
}

/// Every tile carries the complete map/gameplay component contract.
#[test]
fn tiles_carry_the_complete_component_contract() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    {
        let registry = app.world().resource::<AppTypeRegistry>().read();
        assert!(
            registry.get(TypeId::of::<RunBottom>()).is_some(),
            "the map plugin must register the shared RunBottom component"
        );
    }

    let mut query = app.world_mut().query_filtered::<(
        &HexCoord,
        &TilePos,
        &RunBottom,
        &HexSpan,
        &SubstanceId,
        &Headroom,
    ), With<HexTile>>();

    let mut checked = 0;
    for (coord, pos, bottom, span, substance, headroom) in query.iter(app.world()) {
        assert!(!substance.is_air(), "air should not be spawned as a prism");
        assert_eq!(pos.coord, *coord, "a tile's position must match its column");
        assert!(
            bottom.0 <= pos.level,
            "a run's inclusive bottom cannot exceed its inclusive top"
        );
        assert!(span.height() > 0.0, "a tile span must have positive height");
        assert!(
            (0..=MAX_HEADROOM).contains(&headroom.0),
            "headroom must remain bounded"
        );
        checked += 1;
    }
    assert!(checked > 0, "no tiles were checked");

    let coords: Vec<_> = app
        .world()
        .resource::<VoxelMap>()
        .columns()
        .map(|(coord, _)| coord)
        .collect();
    for coord in coords {
        assert_column_run_publication(&mut app, coord);
    }
}

/// In gap-free generated terrain, only the top run of each column has headroom, and
/// under open sky it saturates.
///
/// This is the map's half of a contract gameplay cannot check for itself: a run knows
/// its own extent but nothing about what is stacked on it, so only the map can measure
/// the space above. Getting it wrong is what put the player inside the terrain and
/// left every route walking through the bedrock.
///
/// Generated terrain has no caves or overhangs, so exactly one run per column has room
/// above it and that room is open sky. A column with a bridge over it would report the
/// gap instead, which the platform test below covers.
#[test]
fn only_the_top_of_each_column_has_headroom() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let mut query = app
        .world_mut()
        .query_filtered::<(&TilePos, &Headroom), With<HexTile>>();

    let mut tops: HashMap<HexCoord, Level> = HashMap::new();
    let mut clear_per_column: HashMap<HexCoord, usize> = HashMap::new();
    let mut clear_levels: HashMap<HexCoord, Level> = HashMap::new();

    for (pos, headroom) in query.iter(app.world()) {
        let top = tops.entry(pos.coord).or_insert(pos.level);
        *top = (*top).max(pos.level);
        if headroom.0 > 0 {
            *clear_per_column.entry(pos.coord).or_insert(0) += 1;
            clear_levels.insert(pos.coord, pos.level);
            assert_eq!(
                headroom.0, MAX_HEADROOM,
                "the surface of column {:?} is under open sky and should saturate",
                pos.coord
            );
        }
    }

    assert!(!tops.is_empty(), "no tiles were checked");
    for (coord, top) in &tops {
        assert_eq!(
            clear_per_column.get(coord).copied().unwrap_or(0),
            1,
            "column {coord:?} should have exactly one run with room above it"
        );
        assert_eq!(
            clear_levels.get(coord).copied(),
            Some(*top),
            "the run with room above it in column {coord:?} should be its topmost"
        );
    }
}

/// The world has to exist before the tiles built from it.
///
/// Directly guards `GameplaySetup::Resources` running before `::Terrain`. Systems in
/// one `OnEnter` schedule run in unspecified order unless a set says otherwise, and
/// the two live in different crates, so `.chain()` cannot express it.
#[test]
fn the_world_exists_once_gameplay_starts() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let map = app.world().get_resource::<VoxelMap>();
    assert!(map.is_some(), "tiles spawned without a world to build from");
}

/// Every column within the radius is represented, and nothing outside it is.
///
/// Coordinates now repeat — one entity per substance run — so this checks coverage
/// rather than uniqueness.
#[test]
fn tiles_cover_the_radius_and_nothing_beyond() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let mut query = app.world_mut().query_filtered::<&HexCoord, With<HexTile>>();
    let coords: Vec<HexCoord> = query.iter(app.world()).copied().collect();

    for coord in &coords {
        assert!(
            HexCoord::ORIGIN.distance(*coord) <= TEST_RADIUS,
            "{coord:?} lies outside the configured radius"
        );
    }

    let mut unique = coords;
    unique.sort_by_key(|c| (c.x(), c.y()));
    unique.dedup();
    let expected = (3 * TEST_RADIUS * TEST_RADIUS + 3 * TEST_RADIUS + 1) as usize;
    assert_eq!(unique.len(), expected, "some columns were not spawned");
}
