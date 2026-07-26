//! Integration tests for map construction.
//!
//! # Why these exist
//!
//! Every bug found in this codebase so far was found by a person clicking, not by
//! CI. Two are worth naming, because both were green across `cargo check`, clippy,
//! the unit tests, and every CI check:
//!
//! - The player spawned at ground level and **sank into the terrain**, because it
//!   read tile entities in the same schedule that created them. `Commands`-spawned
//!   entities are not queryable until the queue is applied, so ordering alone was
//!   not enough — the sets needed a sync point between them.
//! - A global observer **panicked on a menu click**, because it took a resource that
//!   only exists during gameplay.
//!
//! Neither is visible to a compiler. Both are visible to a test that runs the
//! schedule and looks at the world afterwards.
//!
//! # What these deliberately do not cover
//!
//! They run headless, with no renderer. A black skybox, a wrong colour, or a mesh at
//! the wrong scale still will not be caught here — only by looking at the window.
//! These raise the floor; they do not replace running the game.

use bevy::app::PluginsState;
use bevy::asset::AssetPlugin;
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;

use std::collections::HashMap;

use hex_assets::GameAssets;
use hex_assets::{Substance, SubstanceFile, SubstanceTable};
use hex_core::{
    GameplaySetup, Headroom, HexCoord, HexGrid, HexSpan, HexTile, Level, Screen, SubstanceId,
    TerrainEdit, TilePos, MAX_HEADROOM,
};
use hex_map::{MapSettings, PerlinStepSettings, TerrainSettings, VoxelMap};

/// Radius used by the tests. Small enough to stay fast, large enough that the
/// tile-count formula is a meaningful check.
const TEST_RADIUS: u32 = 4;

/// Builds a headless app with the map wired up and settings already present.
///
/// Settings are inserted directly rather than loaded from RON: this is testing
/// terrain construction, not the asset pipeline, and a test that depends on file IO
/// fails for reasons that have nothing to do with what it is checking.
fn test_app() -> App {
    let mut app = App::new();

    app.add_plugins((MinimalPlugins, AssetPlugin::default(), StatesPlugin));
    app.init_asset::<Mesh>();
    app.init_asset::<StandardMaterial>();
    app.init_state::<Screen>();

    // The real binary configures these in `AppPlugin`. Repeating it here rather than
    // depending on `hex_game` keeps the test on the crate it is about — and means
    // this test would still catch a missing sync point if the binary's wiring were
    // deleted entirely.
    app.configure_sets(
        OnEnter(Screen::Gameplay),
        (
            GameplaySetup::Resources,
            GameplaySetup::Terrain,
            GameplaySetup::Actors,
        )
            .chain(),
    );

    // Default handles rather than real assets. Tile *placement* is what these tests
    // check, and it does not depend on a mesh having loaded — dragging in file IO
    // would only add a way for them to fail for unrelated reasons.
    app.insert_resource(GameAssets {
        hex_tile: Handle::default(),
        player_pieces: [Handle::default(), Handle::default()],
    });

    app.insert_resource(substance_table());

    app.insert_resource(MapSettings {
        grid_radius: TEST_RADIUS,
        level_height: 0.4,
        terrain: TerrainSettings {
            seed: Some(20_260_725),
            // Taller than the shipped default. The banding puts dirt in the top two
            // levels, so shallow terrain produces nothing but one-voxel runs — and a
            // one-voxel run cannot be split, only removed. Digging needs depth to be
            // worth testing.
            steps: vec![PerlinStepSettings {
                x_freq: 0.035,
                y_freq: 0.05,
                magnitude: 14.0,
            }],
        },
    });

    app.add_plugins(hex_map::grid::plugin);

    while app.plugins_state() != PluginsState::Cleaned {
        app.finish();
        app.cleanup();
    }
    app
}

/// The substances the generator expects, built directly rather than loaded from RON.
///
/// The asset pipeline has its own tests; depending on it here would only add a way
/// for these to fail for unrelated reasons.
fn substance_table() -> SubstanceTable {
    let mut substances = bevy::platform::collections::HashMap::default();
    for (name, solid, diggable) in [
        ("air", false, false),
        ("bedrock", true, false),
        ("dirt", true, true),
        ("grass", true, true),
        ("stone", true, true),
    ] {
        substances.insert(
            name.to_owned(),
            Substance {
                color: (0.5, 0.5, 0.5),
                solid,
                diggable,
            },
        );
    }
    SubstanceTable::from_file(&SubstanceFile { substances })
}

/// Runs the app until it has entered gameplay and the world has settled.
fn enter_gameplay(app: &mut App) {
    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Gameplay);
    // Two updates: one to apply the transition and run `OnEnter`, one to flush the
    // commands it queued so the entities are queryable.
    app.update();
    app.update();
}

fn tile_count(app: &mut App) -> usize {
    app.world_mut()
        .query_filtered::<Entity, With<HexTile>>()
        .iter(app.world())
        .count()
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

/// Every tile carries what it is made of and where it sits, so gameplay can ask
/// whether it is solid or diggable without knowing how the map is stored.
#[test]
fn tiles_carry_their_substance_and_position() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let mut query = app
        .world_mut()
        .query_filtered::<(&SubstanceId, &TilePos, &HexCoord), With<HexTile>>();

    let mut checked = 0;
    for (substance, pos, coord) in query.iter(app.world()) {
        assert!(!substance.is_air(), "air should not be spawned as a prism");
        assert_eq!(pos.coord, *coord, "a tile's position must match its column");
        checked += 1;
    }
    assert!(checked > 0, "no tiles were checked");
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
    let target = {
        let map = app
            .world()
            .get_resource::<VoxelMap>()
            .expect("a world should exist");
        map.columns()
            .find_map(|(coord, column)| {
                hex_map::runs(column)
                    .into_iter()
                    .find(|run| run.levels() >= 3)
                    .map(|run| TilePos::new(coord, run.bottom + 1))
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

/// Leaving gameplay must remove everything the map built. A leak here would grow
/// the world every time the player returned to the title screen and started again.
#[test]
fn leaving_gameplay_removes_the_map() {
    let mut app = test_app();
    enter_gameplay(&mut app);
    assert!(tile_count(&mut app) > 0, "precondition: tiles exist");

    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Title);
    app.update();
    app.update();

    assert_eq!(
        tile_count(&mut app),
        0,
        "tiles outlived the gameplay screen"
    );
    let grids = app
        .world_mut()
        .query_filtered::<Entity, With<HexGrid>>()
        .iter(app.world())
        .count();
    assert_eq!(grids, 0, "the grid parent outlived the gameplay screen");
    assert!(
        app.world().get_resource::<VoxelMap>().is_none(),
        "voxel storage outlived the gameplay screen"
    );
}

/// Re-entering rebuilds a complete grid rather than doubling it or leaving gaps.
#[test]
fn gameplay_can_be_re_entered() {
    let mut app = test_app();
    enter_gameplay(&mut app);
    let first = tile_count(&mut app);

    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Title);
    app.update();
    app.update();
    enter_gameplay(&mut app);

    assert_eq!(
        tile_count(&mut app),
        first,
        "rebuild should match the first"
    );
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
