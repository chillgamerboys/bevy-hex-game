//! Integration tests for map construction.
//!
//! # Why these exist
//!
//! Every bug found in this codebase so far was found by a person clicking, not by
//! CI. Two are worth naming, because both were green across `cargo check`, clippy,
//! the unit tests, and five CI jobs:
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

use hex_assets::GameAssets;
use hex_core::{GameplaySetup, HexCoord, HexGrid, HexSpan, HexTile, Screen};
use hex_map::{HeightMap, MapSettings, PerlinStepSettings, TerrainSettings};

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
        skybox: Handle::default(),
    });

    app.insert_resource(MapSettings {
        grid_radius: TEST_RADIUS,
        height_scale: 0.4,
        tile_color: (1.0, 0.8, 0.8),
        terrain: TerrainSettings {
            seed: Some(20_260_725),
            steps: vec![PerlinStepSettings {
                x_freq: 0.035,
                y_freq: 0.05,
                magnitude: 3.0,
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

#[test]
fn entering_gameplay_spawns_a_full_grid() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    // A hexagon of radius r holds 3r² + 3r + 1 tiles.
    let expected = (3 * TEST_RADIUS * TEST_RADIUS + 3 * TEST_RADIUS + 1) as usize;
    assert_eq!(tile_count(&mut app), expected);
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

/// The contract between the map and everything else: a tile carries the column it
/// occupies, and its transform agrees with that column.
///
/// This is the invariant gameplay leans on to place a piece on a surface, and the
/// one a multi-span rewrite is most likely to break silently — the tiles would still
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

/// Today's terrain is a height field, so every column starts at ground level.
///
/// Not a rule the map has to keep — floating platforms are the point of `HexSpan`.
/// It is here so that when it *does* change, the change is deliberate and shows up
/// in a diff rather than passing unnoticed.
#[test]
fn todays_terrain_rests_on_the_ground() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let mut query = app.world_mut().query_filtered::<&HexSpan, With<HexTile>>();
    for span in query.iter(app.world()) {
        assert!(
            span.bottom.abs() < f32::EPSILON,
            "expected ground-resting columns, found one starting at {}",
            span.bottom
        );
        assert!(span.height() > 0.0, "a column must have positive height");
    }
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

/// The height map has to exist before the tiles that read it.
///
/// Directly guards `GameplaySetup::Resources` running before `::Terrain`. Systems in
/// one `OnEnter` schedule run in unspecified order unless a set says otherwise, and
/// the two live in different crates, so `.chain()` cannot express it.
#[test]
fn the_height_map_exists_once_gameplay_starts() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let height_map = app.world().get_resource::<HeightMap>();
    assert!(
        height_map.is_some(),
        "terrain spawned without its height map"
    );
}

/// Every tile is at a distinct coordinate, and all of them lie within the radius.
///
/// Stacked columns are legal — that is what `HexSpan` is for — but today's generator
/// makes none, so a duplicate coordinate means the range iteration went wrong.
#[test]
fn tiles_cover_the_radius_without_duplicates() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let mut query = app.world_mut().query_filtered::<&HexCoord, With<HexTile>>();
    let coords: Vec<HexCoord> = query.iter(app.world()).copied().collect();

    let mut unique = coords.clone();
    unique.sort_by_key(|c| (c.x(), c.y()));
    unique.dedup();
    assert_eq!(unique.len(), coords.len(), "a coordinate was spawned twice");

    for coord in &coords {
        assert!(
            HexCoord::ORIGIN.distance(*coord) <= TEST_RADIUS,
            "{coord:?} lies outside the configured radius"
        );
    }
}
