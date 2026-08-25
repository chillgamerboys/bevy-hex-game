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

/// The contract between the map and everything else: a tile carries its rendered
/// run's span, and its transform agrees with that span.
///
/// This is the invariant gameplay leans on to place a piece on a surface, and the
/// one a run-meshing change is most likely to break silently — the tiles would still
/// render, just in the wrong place.
#[test]
fn every_tile_transform_matches_its_span() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let mut query = app
        .world_mut()
        .query_filtered::<(&HexSpan, &Transform), With<HexTile>>();

    let mut checked = 0;
    for (span, transform) in query.iter(app.world()) {
        assert!(
            (transform.translation.y - span.centre()).abs() < 1e-4,
            "tile sits at {} but its span centre is {}",
            transform.translation.y,
            span.centre()
        );
        assert!(
            (transform.scale.y - span.height()).abs() < 1e-4,
            "tile is {} tall but its span is {}",
            transform.scale.y,
            span.height()
        );
        checked += 1;
    }
    assert!(checked > 0, "no tiles were checked");
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
