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

use hex_anim::Transformation;
use hex_assets::{CubeCoord, GameAssets, PlayerSettings, ScenarioSettings};
use hex_assets::{Substance, SubstanceFile, SubstanceTable};
use hex_core::{
    GameplaySetup, Headroom, HexCoord, HexSpan, HexTile, Mode, Pause, Screen, SubstanceId, TilePos,
    Turn, MAX_HEADROOM,
};
use hex_units::{
    Enemy, Faction, HoveredSurface, MovingTo, PathOverlay, Player, RangeOverlay, StandsOn, UnitRing,
};

/// World height of the fake ground these tests stand things on.
const GROUND: f32 = 2.0;

/// The level of that ground's surface.
const GROUND_LEVEL: hex_core::Level = 1;

/// The one solid substance the fake terrain is made of. Id 1, since sorted names put
/// `air` at 0 and `stone` next.
const STONE: SubstanceId = SubstanceId(1);

/// Not solid, but a real voxel all the same — id 2, after `air` and `stone`.
const WATER: SubstanceId = SubstanceId(2);

/// A coordinate whose surface run is water rather than stone.
///
/// Three hexes out, on the rim of the fixture. Placing it at two put it inside the
/// budget of `combat_tints_exactly_what_this_turn_can_reach`, whose count is spelled
/// out by hand — so flooding one tile silently changed an unrelated test's expected
/// answer, and that test caught it. Fixtures stay out of each other's way here for the
/// same reason `CRAWLSPACE` does.
const POOL: HexCoord = HexCoord::new_cubic(0, 3, -3);

/// How tall the test player is, matching what the game ships.
const BODY_LEVELS: hex_core::Level = 2;

/// A coordinate roofed over with only one clear voxel — too low for the player.
///
/// Deliberately on the opposite side of the origin from the destination used by
/// `clicking_a_tile_moves_the_player`. That mattered more when `route` walked a
/// straight line and this would have blocked it outright; the search now goes around
/// obstacles, so the worst it could do is lengthen an unrelated test's path. Keeping
/// the fixtures apart still means neither test can fail for the other's reason.
const CRAWLSPACE: HexCoord = HexCoord::new_cubic(-2, 2, 0);

/// Where the enemy starts. Off both the crawlspace and the route the click-to-move
/// test walks, so neither test can fail for the other's reason.
const ENEMY_START: HexCoord = HexCoord::new_cubic(1, 1, -2);

/// A headless app with gameplay wired up, and a stand-in for the map.
///
/// `hex_units` cannot depend on `hex_map` — that is the boundary this whole
/// structure exists to enforce — so the tiles here are spawned by the test itself.
/// That is not a workaround: it is the point. Gameplay consumes `HexTile` entities
/// carrying `TilePos`, `HexSpan`, `SubstanceId` and `Headroom`, and anything
/// producing that contract will do.
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
    // The real app registers this in `screens/gameplay.rs` too. Without it there is no
    // `State<Pause>` at all, and a test for "a paused click does nothing" would pass
    // whether or not the observer checks — the resource simply would not exist.
    app.add_sub_state::<Pause>();

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
/// workaround: gameplay queries `With<HexTile>` for `TilePos`, `HexSpan`,
/// `SubstanceId` and [`Headroom`], and anything producing that contract will do.
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
            if coord == POOL { WATER } else { STONE },
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
    // Rendered, but never footing. Added when the showcase map introduced a river:
    // the map publishes a water run as an ordinary tile entity, and the *only* thing
    // stopping a piece walking onto it is gameplay checking `solid`.
    substances.insert(
        "water".to_owned(),
        Substance {
            color: (0.1, 0.3, 0.65),
            solid: false,
            diggable: true,
        },
    );
    SubstanceTable::from_file(&SubstanceFile { substances })
}

/// Fires a click at `entity`, as the picking backend would.
///
/// The pointer's screen location is irrelevant here — picking has already resolved
/// which entity was hit by the time this event exists, which is exactly why a click
/// identifies one specific surface rather than a coordinate.
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

/// The player records which surface it occupies, not merely which hex.
#[test]
fn the_player_knows_which_surface_it_is_on() {
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

/// Clicking a tile starts a move, and the player arrives **when the walk finishes**.
///
/// This test used to assert the arrival one frame after the click, which passed only
/// because `StandsOn` was written the moment the move was *ordered*. That is the bug
/// the review found: everything asking where a unit is — engagement most of all — was
/// reading the destination rather than the position, so a click across the map started
/// a fight instantly at the far end of the route.
///
/// So the assertion is now in two halves, and the first half is the one that matters:
/// **immediately after the click the player has not moved.**
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

    let player = single::<With<Player>>(&mut app).expect("a player should exist");
    assert_eq!(
        standing_of(&mut app).map(|s| s.pos.coord),
        Some(HexCoord::ORIGIN),
        "the click committed a route; the piece has not walked it yet"
    );
    assert!(
        app.world().get::<MovingTo>(player).is_some(),
        "the committed route should be recorded while the walk runs"
    );

    // Stand in for the animation finishing, which is what `hex_anim`'s driver does.
    app.world_mut()
        .entity_mut(player)
        .remove::<Transformation>();
    app.update();

    let standing = standing_of(&mut app).expect("a player should exist");
    assert_eq!(
        standing.pos.coord, destination,
        "the player should be on the destination once the walk lands"
    );
    assert_eq!(
        standing.pos.level, GROUND_LEVEL,
        "the player should arrive on the surface, not inside the column"
    );
    assert!(
        app.world().get::<MovingTo>(player).is_none(),
        "an arrived piece should no longer be carrying a route"
    );
}

/// Where the player is standing right now.
fn standing_of(app: &mut App) -> Option<hex_units::Standing> {
    let mut players = app.world_mut().query_filtered::<&StandsOn, With<Player>>();
    players.iter(app.world()).next().map(|s| s.0)
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

// ---------------------------------------------------------------------------
// Selection, the turn ring, and the movement overlays.
//
// The bug these exist for: clicking a tile either moved the player or did
// nothing, and "nothing" had five different causes that looked identical.
// Drawing the reachable set is what makes a refusal visible before the click,
// so a test that the tint is *absent* matters as much as one that it is there.
// ---------------------------------------------------------------------------

/// Puts the game in combat with the player holding a turn worth `movement` hexes.
///
/// Returns [`None`] rather than expecting, because the restriction lint fires in test
/// *helpers* as well as in `#[test]` functions — only the test itself may unwrap.
fn take_a_turn(app: &mut App, movement: u32) -> Option<()> {
    app.world_mut()
        .resource_mut::<NextState<Mode>>()
        .set(Mode::Combat);
    let player = single::<With<Player>>(app)?;
    app.world_mut().entity_mut(player).insert(Turn {
        movement_left: movement,
        acted: false,
    });
    app.update();
    Some(())
}

/// Which unit the ring is currently under, or [`None`] if nothing is ringed.
fn ring_owner(app: &mut App) -> Option<Entity> {
    let mut rings = app.world_mut().query_filtered::<&ChildOf, With<UnitRing>>();
    rings
        .iter(app.world())
        .next()
        .map(bevy::prelude::ChildOf::parent)
}

/// The one entity matching a filter, or [`None`].
fn single<Q: bevy::ecs::query::QueryFilter>(app: &mut App) -> Option<Entity> {
    let mut query = app.world_mut().query_filtered::<Entity, Q>();
    query.iter(app.world()).next()
}

/// Points the cursor at the standable surface of a coordinate.
///
/// The *surface* run, filtered by headroom — not the first tile at the coordinate.
/// The fixture stacks a buried run under every surface, and a search can never reach
/// the buried one, so taking the first match would draw no path and blame the feature.
fn hover(app: &mut App, coord: HexCoord) -> Option<()> {
    let mut tiles = app
        .world_mut()
        .query_filtered::<(&TilePos, &Headroom), With<HexTile>>();
    let pos = tiles
        .iter(app.world())
        .find(|(pos, headroom)| pos.coord == coord && headroom.0 > 0)
        .map(|(pos, _)| *pos)?;
    app.world_mut().resource_mut::<HoveredSurface>().0 = Some(pos);
    app.update();
    Some(())
}

fn count<Q: bevy::ecs::query::QueryFilter>(app: &mut App) -> usize {
    let mut query = app.world_mut().query_filtered::<Entity, Q>();
    query.iter(app.world()).count()
}

/// Hovering draws the way there, and the way there is the length the search says.
///
/// Two hexes away is two tinted tiles, not three: the surface the piece already
/// stands on is not part of the journey, and tinting it would read as a move
/// starting one hex early.
#[test]
fn hovering_a_tile_draws_the_way_to_it() {
    let mut app = test_app();
    enter_gameplay(&mut app);
    hover(&mut app, HexCoord::new_cubic(2, -2, 0)).expect("the fixture covers this coordinate");

    assert_eq!(
        count::<With<PathOverlay>>(&mut app),
        2,
        "a tile two steps away should be two tinted steps"
    );
}

/// Exploring has no movement budget, so every connected surface is reachable and a
/// range tint would cover the entire map — which says nothing at all.
///
/// The path still draws. That is the half of the feature exploring actually needs.
#[test]
fn exploring_draws_the_path_but_not_a_range() {
    let mut app = test_app();
    enter_gameplay(&mut app);
    hover(&mut app, HexCoord::new_cubic(2, -2, 0)).expect("the fixture covers this coordinate");

    assert_eq!(
        count::<With<RangeOverlay>>(&mut app),
        0,
        "unlimited movement must not tint the whole map as 'in range'"
    );
    assert!(
        count::<With<PathOverlay>>(&mut app) > 0,
        "the path is the part of the preview exploring still needs"
    );
}

/// In combat the tint covers exactly what this turn's movement can pay for.
///
/// The count is spelled out rather than compared to a formula, because a formula
/// would reproduce whatever mistake the implementation made. Nineteen coordinates lie
/// within two steps of the origin; the crawlspace is one of them and is too low for
/// this body; the piece is standing on another. Seventeen remain.
#[test]
fn combat_tints_exactly_what_this_turn_can_reach() {
    let mut app = test_app();
    enter_gameplay(&mut app);
    take_a_turn(&mut app, 2).expect("a player should exist during gameplay");

    let mut tinted = app
        .world_mut()
        .query_filtered::<&TilePos, With<RangeOverlay>>();
    let positions: Vec<TilePos> = tinted.iter(app.world()).copied().collect();

    assert_eq!(
        positions.len(),
        17,
        "two hexes of movement should reach seventeen other surfaces, got {positions:?}"
    );
    assert!(
        positions
            .iter()
            .all(|pos| pos.coord.distance(HexCoord::ORIGIN) <= 2),
        "something outside the budget was tinted as reachable"
    );
    assert!(
        !positions.iter().any(|pos| pos.coord == CRAWLSPACE),
        "the crawlspace is too low for this body and must not be offered"
    );
}

/// Nothing is tinted on somebody else's turn.
///
/// Regression guard for the promise this feature makes: a lit tile is one the piece
/// can be sent to *now*. Leaving the range up during the enemy's turn would light
/// tiles that any click would refuse, which is the exact confusion being fixed.
#[test]
fn no_tint_while_it_is_not_your_turn() {
    let mut app = test_app();
    enter_gameplay(&mut app);
    take_a_turn(&mut app, 2).expect("a player should exist during gameplay");
    assert!(count::<With<RangeOverlay>>(&mut app) > 0, "setup failed");

    let player = single::<With<Player>>(&mut app).expect("a player should exist");
    app.world_mut().entity_mut(player).remove::<Turn>();
    app.update();

    assert_eq!(
        count::<With<RangeOverlay>>(&mut app),
        0,
        "the range outlived the turn it belonged to"
    );
}

/// A ring marks whoever is acting, and moves on when they stop.
///
/// Reconciled from who holds a `Turn` rather than from `Added`/`RemovedComponents`,
/// because the real turn system takes the marker off one unit and puts it on the next
/// in the same system on the same frame. This test passes the turn the same way.
#[test]
fn the_ring_follows_whoever_is_acting() {
    let mut app = test_app();
    enter_gameplay(&mut app);
    take_a_turn(&mut app, 2).expect("a player should exist during gameplay");

    assert_eq!(
        count::<With<UnitRing>>(&mut app),
        1,
        "the acting unit should be ringed"
    );

    // Hand the turn over in one frame, exactly as `advance_turn` does.
    let player = single::<With<Player>>(&mut app).expect("a player should exist");
    let enemy = single::<With<Enemy>>(&mut app).expect("an enemy should exist");
    app.world_mut().entity_mut(player).remove::<Turn>();
    app.world_mut().entity_mut(enemy).insert(Turn {
        movement_left: 4,
        acted: false,
    });
    app.update();
    app.update();

    assert_eq!(
        count::<With<UnitRing>>(&mut app),
        1,
        "handing the turn over in one frame should leave exactly one ring"
    );

    let owner = ring_owner(&mut app).expect("the ring should be a child of the acting unit");
    assert_eq!(
        owner, enemy,
        "the ring stayed on the unit that stopped acting"
    );
}

/// Overlays are plain world entities, so nothing else tears them down.
///
/// Mirrors `no_unit_leaks_across_screens`, including the two updates: the state
/// transition and the `OnExit` schedule it triggers do not both land in one.
#[test]
fn no_overlay_leaks_across_screens() {
    let mut app = test_app();
    enter_gameplay(&mut app);
    take_a_turn(&mut app, 2).expect("a player should exist during gameplay");
    hover(&mut app, HexCoord::new_cubic(2, -2, 0)).expect("the fixture covers this coordinate");
    assert!(count::<With<RangeOverlay>>(&mut app) > 0, "setup failed");

    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Title);
    app.update();
    app.update();

    assert_eq!(
        count::<Or<(With<RangeOverlay>, With<PathOverlay>)>>(&mut app),
        0,
        "tints from a finished game are still on the title screen"
    );
}

/// Out of combat there is no turn to key a ring on, so it follows the selection.
///
/// Reported from play: "the circle didn't display in explore mode". The first version
/// keyed the ring on `Turn` alone, which does not exist while exploring — so the piece
/// you control looked no different from anything else on the map, in the mode you
/// spend most of your time in.
#[test]
fn a_ring_marks_the_selection_while_exploring() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    assert_eq!(
        count::<With<UnitRing>>(&mut app),
        1,
        "exploring has no turn, so the ring must follow the selection instead"
    );

    let owner = ring_owner(&mut app).expect("the ring should be a child of a unit");
    let player = single::<With<Player>>(&mut app).expect("a player should exist");
    assert_eq!(
        owner, player,
        "the ring is under something that is not yours"
    );
}

/// Entering combat moves the ring from the selection onto whoever is acting.
///
/// Both rules are live at once out of combat and in it, so the handover between them
/// is its own case: one ring, on the acting unit, never two.
#[test]
fn combat_moves_the_ring_onto_the_acting_unit() {
    let mut app = test_app();
    enter_gameplay(&mut app);
    take_a_turn(&mut app, 2).expect("a player should exist during gameplay");

    assert_eq!(
        count::<With<UnitRing>>(&mut app),
        1,
        "the selection ring and the turn ring should never both be drawn"
    );
}

/// Water is rendered as an ordinary tile but is never somewhere to stand.
///
/// The showcase map added a river, and the map publishes a water run exactly like a
/// stone one: same components, same `HexTile`, a `TilePos` at its topmost **material**
/// voxel. The only thing between a piece and walking onto the river is `Footing`
/// checking the substance's `solid` flag — so that check is a gameplay contract, not
/// an implementation detail, and it belongs in a test that would fail without it.
#[test]
fn water_is_drawn_but_is_not_footing() {
    let mut app = test_app();
    enter_gameplay(&mut app);
    take_a_turn(&mut app, 4).expect("a player should exist during gameplay");

    let mut tinted = app
        .world_mut()
        .query_filtered::<&TilePos, With<RangeOverlay>>();
    let reachable: Vec<TilePos> = tinted.iter(app.world()).copied().collect();

    assert!(
        !reachable.is_empty(),
        "setup failed — nothing was reachable at all"
    );
    assert!(
        !reachable.iter().any(|pos| pos.coord == POOL),
        "the river was offered as somewhere to walk to"
    );
}

/// And a click on it does nothing, rather than walking the piece onto the water.
///
/// The tint and the click have to agree. A tile lit as reachable that then refuses —
/// or one left dark that accepts — is worse than either rule alone, because it teaches
/// the player that the highlight cannot be trusted.
#[test]
fn clicking_water_does_not_move_the_player() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let mut tiles = app
        .world_mut()
        .query_filtered::<(Entity, &TilePos, &SubstanceId), With<HexTile>>();
    let pool = tiles
        .iter(app.world())
        .find(|(_, pos, substance)| pos.coord == POOL && **substance == WATER)
        .map(|(entity, _, _)| entity)
        .expect("the fixture floods this coordinate");

    let before = single::<With<Player>>(&mut app).expect("a player should exist");
    let mut standing = app.world_mut().query_filtered::<&StandsOn, With<Player>>();
    let start = standing
        .iter(app.world())
        .next()
        .copied()
        .expect("a player should exist")
        .0
        .pos;

    let window = app.world_mut().spawn(Window::default()).id();
    click(&mut app, pool, window);
    app.update();

    let mut after = app.world_mut().query_filtered::<&StandsOn, With<Player>>();
    let ended = after
        .iter(app.world())
        .next()
        .copied()
        .expect("the player should still exist")
        .0
        .pos;

    assert_eq!(ended, start, "the player waded into the river");
    assert!(
        app.world().get_entity(before).is_ok(),
        "the player should not have been despawned"
    );
}

/// The entity id of a tile's standable surface at a coordinate.
fn surface_at(app: &mut App, coord: HexCoord) -> Option<Entity> {
    let mut tiles = app
        .world_mut()
        .query_filtered::<(Entity, &HexCoord, &Headroom), With<HexTile>>();
    tiles
        .iter(app.world())
        .find(|(_, at, headroom)| **at == coord && headroom.0 > 0)
        .map(|(entity, _, _)| entity)
}

/// How much movement the player has left, if it holds a turn.
fn movement_left(app: &mut App) -> Option<u32> {
    let mut turns = app.world_mut().query_filtered::<&Turn, With<Player>>();
    turns
        .iter(app.world())
        .next()
        .map(|turn| turn.movement_left)
}

/// A second click while the piece is still walking is ignored, not charged.
///
/// `StandsOn` names the *committed destination* from the moment a move starts, so an
/// unguarded second click routes from where the piece is going rather than where it
/// is, queues a second animation over the first, and bills `movement_left` twice for
/// one turn. Two clicks two hexes away would leave a four-hex budget empty having
/// moved two.
#[test]
fn a_second_click_while_moving_is_not_charged_again() {
    let mut app = test_app();
    enter_gameplay(&mut app);
    take_a_turn(&mut app, 4).expect("a player should exist during gameplay");

    let target = surface_at(&mut app, HexCoord::new_cubic(2, -2, 0))
        .expect("the fake terrain covers this coordinate");
    let window = app.world_mut().spawn(Window::default()).id();

    click(&mut app, target, window);
    app.update();
    let after_first = movement_left(&mut app);
    assert_eq!(after_first, Some(2), "two hexes should cost two");

    // **A different tile**, and that detail is the test. Clicking the same one again
    // routes from the destination to itself for a cost of zero, so the bill is
    // unchanged whether or not the guard exists and the test proves nothing. The
    // double charge only appears when the second click has somewhere new to go.
    let elsewhere = surface_at(&mut app, HexCoord::new_cubic(2, 0, -2))
        .expect("the fake terrain covers this coordinate");
    click(&mut app, elsewhere, window);
    app.update();

    assert_eq!(
        movement_left(&mut app),
        after_first,
        "the second click was billed to a turn that had already paid"
    );
}

/// A click that lands while paused does nothing at all.
///
/// `PausableSystems` gates systems, and this is a global observer that was never in
/// that set. Without an explicit check, a click behind the pause overlay spends the
/// turn immediately and the walk plays out the instant the game resumes.
#[test]
fn a_click_while_paused_neither_moves_nor_spends() {
    let mut app = test_app();
    enter_gameplay(&mut app);
    take_a_turn(&mut app, 4).expect("a player should exist during gameplay");

    app.world_mut()
        .resource_mut::<NextState<Pause>>()
        .set(Pause(true));
    app.update();

    let before = movement_left(&mut app);
    let mut standing = app.world_mut().query_filtered::<&StandsOn, With<Player>>();
    let start = standing
        .iter(app.world())
        .next()
        .copied()
        .expect("a player should exist")
        .0
        .pos;

    let target = surface_at(&mut app, HexCoord::new_cubic(2, -2, 0))
        .expect("the fake terrain covers this coordinate");
    let window = app.world_mut().spawn(Window::default()).id();
    click(&mut app, target, window);
    app.update();

    let mut after = app.world_mut().query_filtered::<&StandsOn, With<Player>>();
    let ended = after
        .iter(app.world())
        .next()
        .copied()
        .expect("a player should exist")
        .0
        .pos;

    assert_eq!(ended, start, "the piece moved while the game was paused");
    assert_eq!(
        movement_left(&mut app),
        before,
        "a paused click spent a turn"
    );
}

/// An overlay must not outlive the ground it describes.
///
/// `apply_terrain_edits` despawns the **entire** grid and respawns it on any accepted
/// edit. Nothing about the unit changes when that happens — same piece, same surface,
/// same budget — so a preview keyed only on the unit keeps drawing a route across
/// terrain that no longer exists, while a click computes fresh footing and refuses it.
/// The tint and the click have to agree, or the highlight teaches the player to
/// distrust it.
#[test]
fn a_route_stops_being_drawn_when_its_ground_goes() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let destination = HexCoord::new_cubic(2, -2, 0);
    hover(&mut app, destination).expect("the fixture covers this coordinate");
    assert!(
        count::<With<PathOverlay>>(&mut app) > 0,
        "setup failed — no route was drawn to begin with"
    );

    // Take the destination's surface away, exactly as a rebuilt grid would.
    let mut tiles = app
        .world_mut()
        .query_filtered::<(Entity, &HexCoord), With<HexTile>>();
    let doomed: Vec<Entity> = tiles
        .iter(app.world())
        .filter(|(_, coord)| **coord == destination)
        .map(|(entity, _)| entity)
        .collect();
    for entity in doomed {
        app.world_mut().entity_mut(entity).despawn();
    }
    app.update();

    assert_eq!(
        count::<With<PathOverlay>>(&mut app),
        0,
        "the route is still lit across ground that has been deleted"
    );
}

/// A fight starting mid-walk puts the piece down where it is.
///
/// Committing to a long walk and then being ambushed halfway should leave the piece
/// where the ambush happened, not deliver it to a destination chosen before anybody
/// knew there was a fight.
///
/// It lands on a **whole step** of the route, never between two: a piece standing
/// between hexes is not a position the rest of the game can express, since every rule
/// here is written in terms of a surface.
#[test]
fn a_fight_stops_the_walk_where_it_started() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let destination = HexCoord::new_cubic(3, -3, 0);
    let target = surface_at(&mut app, destination).expect("the fixture covers this coordinate");
    let window = app.world_mut().spawn(Window::default()).id();
    click(&mut app, target, window);
    app.update();

    let player = single::<With<Player>>(&mut app).expect("a player should exist");
    let route: Vec<TilePos> = app
        .world()
        .get::<MovingTo>(player)
        .expect("the click should have committed a route")
        .path
        .iter()
        .map(|standing| standing.pos)
        .collect();
    assert!(route.len() > 1, "setup failed — the route is not a walk");

    app.world_mut()
        .resource_mut::<NextState<Mode>>()
        .set(Mode::Combat);
    app.update();

    assert!(
        app.world().get::<Transformation>(player).is_none(),
        "the walk carried on after the fight began"
    );
    assert!(
        app.world().get::<MovingTo>(player).is_none(),
        "the piece is still holding a route it is no longer walking"
    );

    let ended = standing_of(&mut app).expect("a player should exist").pos;
    assert!(
        route.contains(&ended),
        "the piece was put down at {ended:?}, which is not on the route it was walking"
    );
    assert_ne!(
        ended,
        TilePos::new(destination, GROUND_LEVEL),
        "the piece was delivered to a destination chosen before the fight existed"
    );
}
