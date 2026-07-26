//! Integration tests for the player and movement.
//!
//! These exist because of two bugs that a person found by clicking, both green
//! across every automated check at the time:
//!
//! - Clicking the title screen **panicked**, because a global observer took a
//!   resource that only exists during gameplay. Bevy validates system parameters
//!   before the body runs, so the observer's own guard never got the chance.
//! - The player spawned at ground level and **sank into the terrain**, because it
//!   read tile entities in the schedule that created them.
//!
//! Headless, so nothing visual is covered — see the note in `hex_map`'s tests.

use bevy::app::PluginsState;
use bevy::asset::AssetPlugin;
use bevy::camera::NormalizedRenderTarget;
use bevy::picking::backend::HitData;
use bevy::picking::events::{Click, Pointer};
use bevy::picking::pointer::{Location, PointerButton, PointerId};
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;

use hex_assets::{CubeCoord, GameAssets, PlayerSettings, ScenarioSettings};
use hex_assets::{Substance, SubstanceFile, SubstanceTable};
use hex_core::{
    GameplaySetup, Headroom, HexCoord, HexSpan, HexTile, Mode, Screen, SubstanceId, TilePos,
    MAX_HEADROOM,
};
use hex_units::{Enemy, Faction, Player, StandsOn};

/// World height of the fake ground these tests stand things on.
const GROUND: f32 = 2.0;

/// The level of that ground's surface.
const GROUND_LEVEL: hex_core::Level = 1;

/// The one solid substance the fake terrain is made of. Id 1, since sorted names put
/// `air` at 0 and `stone` next.
const STONE: SubstanceId = SubstanceId(1);

/// How tall the test player is, matching what the game ships.
const BODY_LEVELS: hex_core::Level = 2;

/// A coordinate roofed over with only one clear voxel — too low for the player.
///
/// Deliberately on the opposite side of the origin from the destination used by
/// `clicking_a_tile_moves_the_player`: a straight-line route through here would be
/// blocked, so putting it on that line would make an unrelated test fail for a
/// reason that has nothing to do with what it checks.
const CRAWLSPACE: HexCoord = HexCoord::new_cubic(-2, 2, 0);

/// Where the enemy starts. Off both the crawlspace and the line the click-to-move
/// test walks, so neither test can fail for the other's reason.
const ENEMY_START: HexCoord = HexCoord::new_cubic(1, 1, -2);

/// A headless app with gameplay wired up, and a stand-in for the map.
///
/// `hex_units` cannot depend on `hex_map` — that is the boundary this whole
/// structure exists to enforce — so the tiles here are spawned by the test itself.
/// That is not a workaround: it is the point. Gameplay consumes `HexCoord` and
/// `HexSpan` components, and anything producing those will do.
fn test_app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default(), StatesPlugin));
    app.init_asset::<Mesh>();
    app.init_asset::<StandardMaterial>();
    app.init_state::<Screen>();
    // The real app registers this in `screens/gameplay.rs`. Without it there is no
    // `State<Mode>` at all, and click-to-move correctly refuses to act — which looks
    // exactly like a movement bug from inside a test.
    app.add_sub_state::<Mode>();

    app.configure_sets(
        OnEnter(Screen::Gameplay),
        (
            GameplaySetup::Resources,
            GameplaySetup::Terrain,
            GameplaySetup::Actors,
        )
            .chain(),
    );

    // Stand-in terrain: flat ground across a small patch, spawned in `Terrain` so it
    // is visible to anything in `Actors`, exactly as the real map is.
    app.add_systems(
        OnEnter(Screen::Gameplay),
        spawn_fake_terrain.in_set(GameplaySetup::Terrain),
    );

    // Stand-ins for what the real app loads. Default handles are fine: these tests
    // check placement and bookkeeping, neither of which needs a mesh to have loaded.
    app.insert_resource(GameAssets {
        hex_tile: Handle::default(),
        player_pieces: [Handle::default(), Handle::default()],
        skybox: Handle::default(),
    });
    app.insert_resource(substance_table());
    app.insert_resource(PlayerSettings {
        scale: 0.25,
        speed: 5.0,
        color: (1.0, 0.2, 0.2),
        levels_tall: BODY_LEVELS,
    });
    app.insert_resource(ScenarioSettings {
        player: CubeCoord { x: 0, y: 0, z: 0 },
        enemy: CubeCoord {
            x: ENEMY_START.x(),
            y: ENEMY_START.y(),
            z: ENEMY_START.z(),
        },
    });

    app.add_plugins(hex_units::plugin);

    while app.plugins_state() != PluginsState::Cleaned {
        app.finish();
        app.cleanup();
    }
    app
}

/// Flat ground across a small patch — as **two stacked runs per column**, which is
/// what the real map produces.
///
/// `hex_units` cannot depend on `hex_map` — that is the boundary this structure
/// exists to enforce — so the tiles are spawned by the test itself. That is not a
/// workaround: gameplay consumes `TilePos`, `HexSpan`, `SubstanceId` and [`Headroom`],
/// and anything producing those will do.
///
/// The layering is the whole point. An earlier version of this fixture spawned **one**
/// tile per coordinate, so every tile was trivially the surface and a bug that
/// confused a buried run for a surface could not show up. It shipped: the player stood
/// on the bedrock at the bottom of the column and every route walked underground and
/// arrived nowhere. Terrain in this test has to be layered or it is not terrain.
fn spawn_fake_terrain(mut commands: Commands) {
    for coord in HexCoord::ORIGIN.within_radius(3) {
        // Buried: solid, and deliberately with no room above it. Nothing may stand
        // here, however solid it is.
        commands.spawn((
            HexTile,
            coord,
            TilePos::new(coord, GROUND_LEVEL - 1),
            HexSpan::new(0.0, GROUND - 1.0),
            STONE,
            Headroom(0),
        ));
        // The surface. Open sky everywhere except the crawlspace, which is roofed so
        // low that the player cannot fit even though the ground is perfectly good.
        let headroom = if coord == CRAWLSPACE {
            BODY_LEVELS - 1
        } else {
            MAX_HEADROOM
        };
        commands.spawn((
            HexTile,
            coord,
            TilePos::new(coord, GROUND_LEVEL),
            HexSpan::new(GROUND - 1.0, GROUND),
            STONE,
            Headroom(headroom),
        ));
    }
}

/// A substance table with one solid substance, matching `STONE`.
fn substance_table() -> SubstanceTable {
    let mut substances = bevy::platform::collections::HashMap::default();
    substances.insert(
        "air".to_owned(),
        Substance {
            color: (0.0, 0.0, 0.0),
            solid: false,
            diggable: false,
        },
    );
    substances.insert(
        "stone".to_owned(),
        Substance {
            color: (0.5, 0.5, 0.5),
            solid: true,
            diggable: true,
        },
    );
    SubstanceTable::from_file(&SubstanceFile { substances })
}

/// Fires a click at `entity`, as the picking backend would.
///
/// The pointer's screen location is irrelevant here — picking has already resolved
/// which entity was hit by the time this event exists, which is exactly why a click
/// identifies one specific column rather than a coordinate.
fn click(app: &mut App, entity: Entity, window: Entity) {
    let Some(target) = bevy::window::WindowRef::Entity(window).normalize(Some(window)) else {
        unreachable!("an explicit window entity always normalizes")
    };
    let location = Location {
        target: NormalizedRenderTarget::Window(target),
        position: Vec2::ZERO,
    };
    let click = Click {
        button: PointerButton::Primary,
        hit: HitData::new(entity, 0.0, None, None),
        duration: core::time::Duration::from_millis(1),
        count: 1,
    };
    app.world_mut()
        .trigger(Pointer::new(PointerId::Mouse, location, click, entity));
}

fn enter_gameplay(app: &mut App) {
    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Gameplay);
    app.update();
    app.update();
}

/// Regression test for the sunken player.
///
/// The player must stand *on* the surface, not at the world origin. Getting this
/// wrong looked like a rendering bug and was actually a scheduling one: the spawn
/// read tiles that had not been flushed from the command queue yet, found nothing,
/// and fell back to ground level.
#[test]
fn the_player_spawns_on_the_surface() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let mut query = app.world_mut().query_filtered::<&Transform, With<Player>>();
    let transform = query
        .iter(app.world())
        .next()
        .expect("a player should exist during gameplay");

    assert!(
        (transform.translation.y - GROUND).abs() < 1e-4,
        "player is at y={} but the ground is at {GROUND}",
        transform.translation.y
    );
}

/// The player records which column it occupies, not merely which hex.
#[test]
fn the_player_knows_which_column_it_is_on() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let mut query = app.world_mut().query_filtered::<&StandsOn, With<Player>>();
    let standing = query
        .iter(app.world())
        .next()
        .copied()
        .expect("a player should exist during gameplay");

    assert_eq!(standing.0.pos.coord, HexCoord::ORIGIN);
    assert!((standing.0.span.top - GROUND).abs() < 1e-4);
}

/// Regression test for the title-screen crash.
///
/// The click observer is global: it fires in every state, including menus, and
/// including **before settings have finished loading**. Bevy validates system
/// parameters *before* the body runs, so a plain `Res<T>` on a resource that does
/// not exist yet panics regardless of any guard inside the observer.
///
/// The app here deliberately omits `PlayerSettings`. An earlier version of this test
/// used the full harness, which inserts it — so the observer's parameters always
/// validated and the test passed even with the bug reintroduced. It verified
/// nothing. Reproducing the crash requires reproducing the *absence*.
#[test]
fn clicking_before_settings_load_does_not_panic() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default(), StatesPlugin));
    app.init_asset::<Mesh>();
    app.init_asset::<StandardMaterial>();
    app.init_state::<Screen>();
    // No GameAssets, no PlayerSettings: the state the game is in on the title
    // screen, before the loading screen has run.
    app.add_plugins(hex_units::plugin);
    app.update();

    let window = app.world_mut().spawn(Window::default()).id();
    let target = app.world_mut().spawn_empty().id();
    click(&mut app, target, window);
    app.update();
}

/// Clicking a tile starts a move, and updates which column the player is on.
#[test]
fn clicking_a_tile_moves_the_player() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let destination = HexCoord::new_cubic(2, -2, 0);
    // The surface run, not the buried one under it — that is the face a click would
    // actually land on.
    let mut tiles = app
        .world_mut()
        .query_filtered::<(Entity, &HexCoord, &Headroom), With<HexTile>>();
    let target = tiles
        .iter(app.world())
        .find(|(_, coord, headroom)| **coord == destination && headroom.0 > 0)
        .map(|(entity, _, _)| entity)
        .expect("the fake terrain covers this coordinate");

    let window = app.world_mut().spawn(Window::default()).id();
    click(&mut app, target, window);
    app.update();

    let mut players = app.world_mut().query_filtered::<&StandsOn, With<Player>>();
    let standing = players
        .iter(app.world())
        .next()
        .copied()
        .expect("a player should exist");

    assert_eq!(
        standing.0.pos.coord, destination,
        "clicking a tile should move the player onto that column"
    );
    assert_eq!(
        standing.0.pos.level, GROUND_LEVEL,
        "the player should arrive on the surface, not inside the column"
    );
}

/// Regression test for the buried-run bug.
///
/// A column is several stacked runs, and only the top one is a surface. Treating
/// every run as standable made the bedrock at the bottom look exactly as good as the
/// grass on top — so the player spawned inside the terrain, and routes walked the
/// buried layer, never arrived at the clicked tile, and returned "no route". Both
/// visible symptoms, one cause.
///
/// The fix is [`Headroom`]: the map reports how much space sits above each run,
/// because a run knows its own extent but nothing about what is stacked on it. Zero
/// means buried.
#[test]
fn buried_runs_are_not_standable() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let mut buried = app.world_mut().query_filtered::<&Headroom, With<HexTile>>();
    assert!(
        buried.iter(app.world()).any(|headroom| headroom.0 == 0),
        "the fixture must contain buried runs or this test proves nothing"
    );

    let mut players = app.world_mut().query_filtered::<&StandsOn, With<Player>>();
    let standing = players
        .iter(app.world())
        .next()
        .copied()
        .expect("a player should exist");

    assert_eq!(
        standing.0.pos.level, GROUND_LEVEL,
        "the player stood on a buried run instead of the surface"
    );
}

/// Clicking ground the player is too tall to stand on does nothing.
///
/// The terrain is perfectly solid and one flat step away — the only thing wrong with
/// it is a ceiling one voxel up. A shorter piece would walk straight in. This is the
/// end-to-end version of the size rule, through a real click on a real tile entity.
#[test]
fn clicking_a_space_too_low_to_fit_does_not_move_the_player() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let mut tiles = app
        .world_mut()
        .query_filtered::<(Entity, &HexCoord, &Headroom), With<HexTile>>();
    let target = tiles
        .iter(app.world())
        .find(|(_, coord, headroom)| **coord == CRAWLSPACE && headroom.0 > 0)
        .map(|(entity, _, _)| entity)
        .expect("the crawlspace is part of the fake terrain");

    let window = app.world_mut().spawn(Window::default()).id();
    click(&mut app, target, window);
    app.update();

    let mut players = app.world_mut().query_filtered::<&StandsOn, With<Player>>();
    let standing = players
        .iter(app.world())
        .next()
        .copied()
        .expect("a player should exist");

    assert_eq!(
        standing.0.pos.coord,
        HexCoord::ORIGIN,
        "the player squeezed into a space too low for it"
    );
}

/// Leaving gameplay removes the player; re-entering brings back exactly one.
#[test]
fn the_player_does_not_leak_across_screens() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Title);
    app.update();
    app.update();

    let count = app
        .world_mut()
        .query_filtered::<Entity, With<Player>>()
        .iter(app.world())
        .count();
    assert_eq!(count, 0, "the player outlived the gameplay screen");

    enter_gameplay(&mut app);
    let count = app
        .world_mut()
        .query_filtered::<Entity, With<Player>>()
        .iter(app.world())
        .count();
    assert_eq!(count, 1, "re-entering should give exactly one player");
}

/// Teardown is keyed on [`Faction`], not on `Player`, so every unit is covered by the
/// same system. An enemy that outlived the screen would accumulate one per visit —
/// invisible until the fourth or fifth time somebody re-entered.
#[test]
fn no_unit_leaks_across_screens() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Title);
    app.update();
    app.update();

    let count = app
        .world_mut()
        .query_filtered::<Entity, With<Faction>>()
        .iter(app.world())
        .count();
    assert_eq!(count, 0, "a unit outlived the gameplay screen");

    enter_gameplay(&mut app);
    let count = app
        .world_mut()
        .query_filtered::<Entity, With<Faction>>()
        .iter(app.world())
        .count();
    assert_eq!(
        count, 2,
        "re-entering should give exactly the player and one enemy"
    );
}

/// The enemy stands *on* its scenario coordinate, not at the world origin and not
/// inside the terrain — the same regression the player already has a test for, which
/// a second unit is perfectly capable of reproducing independently.
#[test]
fn the_enemy_spawns_where_the_scenario_says() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let mut query = app
        .world_mut()
        .query_filtered::<(&StandsOn, &Transform), With<Enemy>>();
    let (standing, transform) = query
        .iter(app.world())
        .next()
        .expect("an enemy should exist during gameplay");

    assert_eq!(
        standing.0.pos.coord, ENEMY_START,
        "the enemy ignored its scenario coordinate"
    );
    assert_eq!(
        standing.0.pos.level, GROUND_LEVEL,
        "the enemy stood on a buried run instead of the surface"
    );
    assert!(
        (transform.translation.y - GROUND).abs() < 1e-4,
        "enemy is at y={} but the ground is at {GROUND}",
        transform.translation.y
    );
}

/// A coordinate whose components do not sum to zero is a designer's typo, not a
/// crash. It falls back to the centre of the map and says so.
#[test]
fn an_impossible_scenario_coordinate_falls_back_to_the_centre() {
    let mut app = test_app();
    app.insert_resource(ScenarioSettings {
        player: CubeCoord { x: 0, y: 0, z: 0 },
        // 1 + 1 + 1 is not 0, so this is not a hex.
        enemy: CubeCoord { x: 1, y: 1, z: 1 },
    });
    enter_gameplay(&mut app);

    let mut query = app.world_mut().query_filtered::<&StandsOn, With<Enemy>>();
    let standing = query
        .iter(app.world())
        .next()
        .copied()
        .expect("a bad coordinate should still produce an enemy");

    assert_eq!(
        standing.0.pos.coord,
        HexCoord::ORIGIN,
        "an impossible coordinate should fall back to the centre"
    );
}
