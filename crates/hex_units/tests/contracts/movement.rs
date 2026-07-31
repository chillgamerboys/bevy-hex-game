//! Contract tests for the player and movement.
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

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use bevy::asset::AssetPlugin;
use bevy::camera::NormalizedRenderTarget;
use bevy::picking::backend::HitData;
use bevy::picking::events::{Click, Pointer};
use bevy::picking::pointer::{Location, PointerButton, PointerId};
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;

use hex_anim::Transformation;
use hex_assets::{
    ArtPalette, CubeCoord, Encounter, EncounterFaction, EncounterPlacement, FormationCenter,
    GameAssets, PaletteSwatch, PlayerSettings, Roster, RosterEntry, SrgbColor, SwatchId,
};
use hex_assets::{Substance, SubstanceFile, SubstanceTable};
use hex_core::{
    Busy, CommandQueue, GameCommand, GameplayPhase, GameplaySetup, GameplaySetupFailure, Headroom,
    HexCoord, HexSpan, HexTile, MapAnchorId, MapAnchors, Mode, PartyFormation, PartyMovementMode,
    Pause, Screen, SubstanceId, TerrainReady, TilePos, TraversalBlockers, TraversalProfile, Turn,
    MAX_HEADROOM,
};
use hex_test_support::TestAppBuilder;
use hex_units::{
    Body, Enemy, Faction, Footing, HexPathingLine, HoveredSurface, MovementSystems, MovingTo,
    Party, PathOverlay, Player, RangeOverlay, Selected, StandsOn, UnitRegistry, UnitRing,
};

/// World height of the fake ground these tests stand things on.
const GROUND: f32 = 2.0;

/// The level of that ground's surface.
const GROUND_LEVEL: hex_core::Level = 1;

/// Non-solid lava sorts first after air in the fixture's substance table.
const LAVA: SubstanceId = SubstanceId(1);

/// The one solid substance the fake terrain is made of.
const STONE: SubstanceId = SubstanceId(2);

/// Non-solid water sorts after stone.
const WATER: SubstanceId = SubstanceId(3);

/// A coordinate whose surface run is water rather than stone.
///
/// Three hexes out, on the rim of the fixture. Placing it at two put it inside the
/// budget of `combat_tints_exactly_what_this_turn_can_reach`, whose count is spelled
/// out by hand — so flooding one tile silently changed an unrelated test's expected
/// answer, and that test caught it. Fixtures stay out of each other's way here for the
/// same reason `CRAWLSPACE` does.
const POOL: HexCoord = HexCoord::new_cubic(0, 3, -3);

/// A second non-solid hazard used to keep live volcanic movement in parity.
const LAVA_POOL: HexCoord = HexCoord::new_cubic(3, 0, -3);

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

/// A solid surface occupied by a generated tree feature.
const TREE_ROOT: HexCoord = HexCoord::new_cubic(0, 1, -1);

/// Where the enemy starts. Off both the crawlspace and the route the click-to-move
/// test walks, so neither test can fail for the other's reason.
const ENEMY_START: HexCoord = HexCoord::new_cubic(1, 1, -2);

/// An encounter placement at an authored coordinate.
fn fixed(coord: HexCoord) -> EncounterPlacement {
    EncounterPlacement::Fixed(CubeCoord {
        x: coord.x(),
        y: coord.y(),
        z: coord.z(),
    })
}

/// A roster of `archetypes` on one side, all coming in through one placement.
fn roster(faction: EncounterFaction, placement: EncounterPlacement, archetypes: &[&str]) -> Roster {
    Roster {
        faction,
        placement,
        units: archetypes
            .iter()
            .map(|archetype| RosterEntry {
                archetype: (*archetype).to_owned(),
                placement: None,
                ai_profile: None,
                ai_group: None,
            })
            .collect(),
    }
}

/// One player against one hostile — the shape every test here used before rosters,
/// so the placement behaviour these tests cover is unchanged by the schema.
fn duel(player: EncounterPlacement, hostile: EncounterPlacement) -> Encounter {
    Encounter {
        name: "Test Duel".to_owned(),
        rosters: vec![
            roster(EncounterFaction::Player, player, &["hedge-mage"]),
            roster(EncounterFaction::Hostile, hostile, &["raider"]),
        ],
    }
}

/// A headless app with gameplay wired up, and a stand-in for the map.
///
/// `hex_units` cannot depend on `hex_map` — that is the boundary this whole
/// structure exists to enforce — so the tiles here are spawned by the test itself.
/// That is not a workaround: it is the point. Gameplay consumes `HexTile` entities
/// carrying `TilePos`, `HexSpan`, `SubstanceId` and `Headroom`, and anything
/// producing that contract will do.
fn test_app() -> App {
    let mut builder = TestAppBuilder::new().with_fixed_step(Duration::ZERO);
    let app = builder.app_mut();

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
    let palette = art_palette();
    app.insert_resource(substance_table(&palette));
    app.insert_resource(PlayerSettings {
        scale: 0.25,
        speed: 5.0,
    });
    app.insert_resource(palette);
    app.insert_resource(duel(fixed(HexCoord::ORIGIN), fixed(ENEMY_START)));

    app.add_plugins(hex_units::plugin);

    builder.build()
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
            if coord == POOL {
                WATER
            } else if coord == LAVA_POOL {
                LAVA
            } else {
                STONE
            },
            Headroom(headroom),
        ));
    }
    commands.insert_resource(TerrainReady);
}

/// A substance table with one solid substance, matching `STONE`.
#[expect(
    clippy::expect_used,
    reason = "invalid compile-time fixture data should fail the test immediately"
)]
fn fixture_swatch_id(id: &str) -> SwatchId {
    SwatchId::new(id).expect("fixture swatch ids should be valid")
}

#[expect(
    clippy::expect_used,
    reason = "invalid compile-time fixture data should fail the test immediately"
)]
fn art_palette() -> ArtPalette {
    let swatches = [
        ("liquid/lava", "Lava", [0.9, 0.2, 0.05]),
        ("liquid/water", "Water", [0.1, 0.3, 0.65]),
        ("terrain/stone", "Stone", [0.5, 0.5, 0.5]),
        ("unit/hostile", "Hostile", [0.25, 0.45, 0.9]),
        ("unit/player", "Player", [1.0, 0.2, 0.2]),
    ]
    .into_iter()
    .map(|(id, name, [red, green, blue])| {
        let color =
            SrgbColor::new(red, green, blue).expect("fixture swatch colors should be valid");
        let swatch = PaletteSwatch::new(name, color, BTreeSet::from(["test".to_owned()]))
            .expect("fixture palette entries should be valid");
        (fixture_swatch_id(id), swatch)
    })
    .collect::<BTreeMap<_, _>>();
    ArtPalette::new(swatches).expect("fixture palette should be valid")
}

#[expect(
    clippy::expect_used,
    reason = "invalid compile-time fixture data should fail the test immediately"
)]
fn substance_table(palette: &ArtPalette) -> SubstanceTable {
    let mut substances = bevy::platform::collections::HashMap::default();
    substances.insert("air".to_owned(), Substance::invisible(false, false));
    substances.insert(
        "lava".to_owned(),
        Substance::from_swatch(fixture_swatch_id("liquid/lava"), false, true),
    );
    substances.insert(
        "stone".to_owned(),
        Substance::from_swatch(fixture_swatch_id("terrain/stone"), true, true),
    );
    // Rendered, but never footing. Added when the showcase map introduced a river:
    // the map publishes a water run as an ordinary tile entity, and the *only* thing
    // stopping a piece walking onto it is gameplay checking `solid`.
    substances.insert(
        "water".to_owned(),
        Substance::from_swatch(fixture_swatch_id("liquid/water"), false, true),
    );
    SubstanceTable::from_file(&SubstanceFile { substances }, palette)
        .expect("fixture substances should resolve through the fixture palette")
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

/// Starts the walk the click emitted, exactly as `hex_combat`'s applier would.
///
/// The observer is an emitter: a click resolves a route and pushes a command,
/// and the one applier — in `hex_combat`, across a boundary this crate cannot
/// see — grounds the path and starts the animation. These tests exercise the
/// movement mechanics *behind* that applier, so this helper plays its part,
/// The full click-to-walk pipeline is covered by `hex_combat`'s funnel tests.
///
/// Returns [`None`] when no move was emitted or it cannot be grounded — only
/// the `#[test]` itself may unwrap.
fn commit_move(app: &mut App) -> Option<()> {
    let issued = app.world_mut().resource_mut::<CommandQueue>().pop()?;
    let GameCommand::MoveAlong { unit, path } = issued.command else {
        return None;
    };
    let entity = app.world().resource::<UnitRegistry>().entity_of(unit)?;
    let speed = app.world().resource::<PlayerSettings>().speed;
    let body = *app.world().get::<Body>(entity)?;

    let mut tiles = app
        .world_mut()
        .query_filtered::<(&TilePos, &HexSpan, &SubstanceId, &Headroom), With<HexTile>>();
    let steps = {
        let world = app.world();
        let footing = Footing::from_tiles(
            tiles.iter(world),
            world.resource::<SubstanceTable>(),
            body,
            world.get_resource::<TraversalBlockers>(),
        );
        path.iter()
            .map(|pos| footing.at(*pos))
            .collect::<Option<Vec<_>>>()?
    };

    let animation: Transformation = HexPathingLine::new(&steps, speed).into();
    app.world_mut()
        .entity_mut(entity)
        .insert((animation, MovingTo::new(steps, speed), Busy));
    Some(())
}

/// Settles one domain route without using presentation-component lifetime as an oracle.
fn settle_movement(app: &mut App, entity: Entity) {
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        core::time::Duration::from_millis(100),
    ));
    for _ in 0..32 {
        if app.world().get::<MovingTo>(entity).is_none() {
            break;
        }
        app.update();
    }
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        core::time::Duration::ZERO,
    ));
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

#[test]
fn shipped_unit_swatches_preserve_the_pre_migration_colors() {
    let palette: ArtPalette = ron::from_str(include_str!("../../../../assets/art/palette.ron"))
        .expect("the shipped art palette should parse");

    for (id, expected) in [
        ("unit/player", [1.0_f32, 0.2, 0.2]),
        ("unit/hostile", [0.25_f32, 0.45, 0.9]),
    ] {
        let actual = palette
            .get_str(id)
            .unwrap_or_else(|| panic!("the shipped palette should contain {id}"))
            .color()
            .to_array();
        assert_eq!(
            actual.map(f32::to_bits),
            expected.map(f32::to_bits),
            "{id} changed during the zero-delta palette migration"
        );
    }
}

#[test]
fn unit_materials_use_the_exact_authored_palette_swatches() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    let player_color = unit_material_color::<With<Player>>(&mut app)
        .expect("the player mesh should have a material");
    let hostile_color = unit_material_color::<With<Enemy>>(&mut app)
        .expect("the hostile mesh should have a material");

    assert_eq!(
        player_color.to_srgba(),
        Color::srgb(1.0, 0.2, 0.2).to_srgba(),
        "the player material did not resolve unit/player"
    );
    assert_eq!(
        hostile_color.to_srgba(),
        Color::srgb(0.25, 0.45, 0.9).to_srgba(),
        "the hostile material did not resolve unit/hostile"
    );
}

#[test]
fn a_missing_required_unit_swatch_fails_before_spawning_or_allocating_materials() {
    let mut app = test_app();
    let hostile_id = SwatchId::new("unit/hostile").expect("the shipped id should be valid");
    let hostile_color = app
        .world()
        .resource::<ArtPalette>()
        .get(&hostile_id)
        .expect("the fixture should contain unit/hostile")
        .color()
        .to_bevy_color();
    let player_color = app
        .world()
        .resource::<ArtPalette>()
        .get_str("unit/player")
        .expect("the fixture should contain unit/player")
        .color()
        .to_bevy_color();
    app.world_mut()
        .resource_mut::<ArtPalette>()
        .remove(&hostile_id)
        .expect("removing one swatch should preserve palette validity")
        .expect("the fixture should contain unit/hostile");

    enter_gameplay(&mut app);

    let spawned = app
        .world_mut()
        .query_filtered::<Entity, With<Faction>>()
        .iter(app.world())
        .count();
    assert_eq!(spawned, 0, "a palette failure left partial actors behind");
    assert!(
        app.world()
            .resource::<GameplaySetupFailure>()
            .reason
            .contains("unit/hostile"),
        "the setup failure did not identify the missing required swatch"
    );

    let materials = app.world().resource::<Assets<StandardMaterial>>();
    assert_eq!(
        materials.len(),
        4,
        "only the range, path, player-ring, and hostile-ring overlay materials should exist"
    );
    assert!(
        materials.iter().all(|(_, material)| {
            material.base_color != player_color && material.base_color != hostile_color
        }),
        "unit material allocation began before all required swatches resolved"
    );
}

/// Map validation and live movement both use the canonical walker.
#[test]
fn gameplay_units_use_the_canonical_walker() {
    let mut app = test_app();
    enter_gameplay(&mut app);

    assert_eq!(TraversalProfile::WALKER.levels_tall, 2);
    assert_eq!(TraversalProfile::WALKER.max_climb, 1);
    assert_eq!(TraversalProfile::WALKER.max_drop, 1);

    let mut bodies = app.world_mut().query_filtered::<&Body, With<Player>>();
    let body = bodies
        .iter(app.world())
        .next()
        .expect("the spawned player should carry a body");
    assert_eq!(
        body.traversal_profile(),
        TraversalProfile::WALKER,
        "live movement did not use the profile used by map validation"
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

    // The click emitted intent, nothing more: the piece has not moved and no
    // route is committed until the applier speaks.
    let player = single::<With<Player>>(&mut app).expect("a player should exist");
    assert_eq!(
        standing_of(&mut app).map(|s| s.pos.coord),
        Some(HexCoord::ORIGIN),
        "a click alone must not move the piece"
    );
    assert!(
        app.world().get::<MovingTo>(player).is_none(),
        "the observer must emit a command, not commit a route itself"
    );
    assert_eq!(
        app.world().resource::<CommandQueue>().len(),
        1,
        "one click on a routable tile is exactly one move command"
    );

    commit_move(&mut app).expect("the emitted move should ground and start");
    app.update();
    assert!(
        app.world().get::<MovingTo>(player).is_some(),
        "the committed route should be recorded while the walk runs"
    );

    settle_movement(&mut app, player);

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

#[test]
fn solo_mode_emits_for_only_the_selected_party_member() {
    let mut app = test_app();
    app.insert_resource(Encounter {
        name: "Test Party".to_owned(),
        rosters: vec![
            roster(
                EncounterFaction::Player,
                EncounterPlacement::Formation {
                    center: FormationCenter::Fixed(CubeCoord { x: 0, y: 0, z: 0 }),
                    spread: 2,
                },
                &["hedge-mage", "raider"],
            ),
            roster(EncounterFaction::Hostile, fixed(ENEMY_START), &["wolf"]),
        ],
    });
    enter_gameplay(&mut app);
    app.world_mut().resource_mut::<PartyFormation>().mode = PartyMovementMode::Solo;
    let selected = app
        .world()
        .resource::<Party>()
        .members
        .first()
        .copied()
        .expect("the party should have a first member");
    let selected_entity = app
        .world()
        .resource::<UnitRegistry>()
        .entity_of(selected)
        .expect("the first party member should be registered");
    app.world_mut().entity_mut(selected_entity).insert(Selected);
    let destination = HexCoord::new_cubic(2, -2, 0);
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

    let queued = app
        .world_mut()
        .resource_mut::<CommandQueue>()
        .pop()
        .expect("Solo click should emit one command");
    assert_eq!(queued.command.unit(), selected);
    assert!(matches!(queued.command, GameCommand::MoveAlong { .. }));
    assert!(
        app.world().resource::<CommandQueue>().is_empty(),
        "Solo mode must not emit a second member's movement"
    );
}

/// A completed domain route reaches its bound and presentation also settles.
#[test]
fn a_completed_domain_route_reaches_its_destination() {
    let mut app = test_app();
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        core::time::Duration::from_millis(100),
    ));
    enter_gameplay(&mut app);

    let destination = HexCoord::new_cubic(2, -2, 0);
    let target =
        surface_at(&mut app, destination).expect("the fake terrain covers this coordinate");
    let window = app.world_mut().spawn(Window::default()).id();
    click(&mut app, target, window);
    commit_move(&mut app).expect("the emitted move should ground and start");

    for _ in 0..20 {
        app.update();
        let player = single::<With<Player>>(&mut app).expect("a player should exist");
        if app.world().get::<MovingTo>(player).is_none() {
            break;
        }
    }

    let player = single::<With<Player>>(&mut app).expect("a player should exist");
    assert!(
        app.world().get::<Transformation>(player).is_none(),
        "the route animation did not finish"
    );
    assert!(
        app.world().get::<MovingTo>(player).is_none(),
        "the finished route was not reconciled"
    );
    assert_eq!(
        standing_of(&mut app).map(|standing| standing.pos.coord),
        Some(destination)
    );
}

#[derive(Resource, Default)]
struct StandingChangeCount(usize);

fn count_player_standing_changes(
    changed: Query<(), (With<Player>, Changed<StandsOn>)>,
    mut count: ResMut<StandingChangeCount>,
) {
    count.0 += changed.iter().count();
}

/// A logical position changes only when the walk completes another whole leg.
///
/// Reassigning the same `StandsOn` on every animation tick makes Bevy's change
/// detection report movement while the piece is still between the same two waypoints.
#[test]
fn an_unfinished_leg_does_not_mark_stands_on_changed() {
    let mut app = test_app();
    app.init_resource::<StandingChangeCount>();
    app.add_systems(
        Update,
        count_player_standing_changes.after(MovementSystems::Reconcile),
    );
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        core::time::Duration::from_millis(100),
    ));
    enter_gameplay(&mut app);
    app.world_mut().resource_mut::<StandingChangeCount>().0 = 0;

    let target = surface_at(&mut app, HexCoord::new_cubic(2, -2, 0))
        .expect("the fake terrain covers this coordinate");
    let window = app.world_mut().spawn(Window::default()).id();
    click(&mut app, target, window);
    commit_move(&mut app).expect("the emitted move should ground and start");

    // The first driven frame anchors at zero; the next three active ticks are still
    // short of one 1.73-unit leg at five world units per second.
    for _ in 0..4 {
        app.update();
    }
    assert_eq!(
        app.world().resource::<StandingChangeCount>().0,
        0,
        "StandsOn changed before a whole route leg completed"
    );

    app.update();
    assert_eq!(
        app.world().resource::<StandingChangeCount>().0,
        1,
        "completing the first route leg should publish exactly one logical change"
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

/// A coordinate whose components do not sum to zero is not a hex.
///
/// **This assertion was inverted deliberately.** It used to require the unit to appear
/// at the centre of the map with a warning, which is the failure this codebase is worst
/// at seeing: the piece exists, the frame renders, and it is simply not where the
/// designer put it. With a roster the same fallback also silently collides with whoever
/// is standing at the centre. Every rostered unit is now either placed where it was
/// asked for or setup fails naming the entry — and an encounter *file* carrying this
/// coordinate no longer parses at all.
#[test]
fn an_impossible_encounter_coordinate_fails_setup_with_a_reason() {
    let mut app = test_app();
    // 1 + 1 + 1 is not 0, so this is not a hex.
    app.insert_resource(duel(
        fixed(HexCoord::ORIGIN),
        EncounterPlacement::Fixed(CubeCoord { x: 1, y: 1, z: 1 }),
    ));
    enter_gameplay(&mut app);

    assert!(
        single::<With<Enemy>>(&mut app).is_none(),
        "an impossible coordinate must not put a unit somewhere nobody asked for"
    );
    let reason = &app.world().resource::<GameplaySetupFailure>().reason;
    assert!(
        reason.contains("sum to zero") && reason.contains("raider"),
        "the failure should name the entry and what is wrong with it: {reason}"
    );
}

/// An authored coordinate with nothing standable on it is the same class of typo, and
/// gets the same answer: a named failure rather than a unit at the centre of the map.
#[test]
fn an_authored_coordinate_with_no_footing_fails_setup_with_a_reason() {
    let mut app = test_app();
    // Well outside the radius the fake terrain covers, so there is no surface at all.
    app.insert_resource(duel(
        fixed(HexCoord::ORIGIN),
        fixed(HexCoord::new_cubic(9, -9, 0)),
    ));
    enter_gameplay(&mut app);

    assert!(single::<With<Enemy>>(&mut app).is_none());
    assert!(app
        .world()
        .resource::<GameplaySetupFailure>()
        .reason
        .contains("nothing can be stood on"));
}

/// Generated placement is a contract with the active map. A missing anchor must not
/// quietly put the unit at the origin, where it could make an invalid map appear to
/// have loaded correctly.
#[test]
fn a_missing_generated_anchor_fails_required_actor_setup() {
    let mut app = test_app();
    app.insert_resource(duel(
        EncounterPlacement::Anchor("missing_party_start".to_owned()),
        fixed(ENEMY_START),
    ));
    enter_gameplay(&mut app);

    assert!(
        single::<With<Player>>(&mut app).is_none(),
        "a missing anchor must not fall back to an arbitrary surface"
    );
    assert!(
        single::<With<Enemy>>(&mut app).is_none(),
        "actor setup should stop after a required placement fails"
    );
    assert!(app
        .world()
        .resource::<GameplaySetupFailure>()
        .reason
        .contains("missing_party_start"));
}

const DECK_ANCHOR: &str = "test_deck";
const DECK_COORD: HexCoord = HexCoord::new_cubic(3, -3, 0);
const DECK_LEVEL: hex_core::Level = 6;

fn spawn_anchored_deck(mut commands: Commands) {
    commands.spawn((
        HexTile,
        DECK_COORD,
        TilePos::new(DECK_COORD, DECK_LEVEL),
        HexSpan::new(6.0, 7.0),
        STONE,
        Headroom(MAX_HEADROOM),
    ));
    let mut anchors = MapAnchors::new();
    let _previous = anchors.insert(
        MapAnchorId::from(DECK_ANCHOR),
        TilePos::new(DECK_COORD, DECK_LEVEL),
    );
    commands.insert_resource(anchors);
}

/// An anchor names a `TilePos`, not merely a coordinate. On a stacked column the
/// generated placement must stay on the published deck rather than choosing the
/// ordinary ground underneath it.
#[test]
fn a_generated_anchor_uses_its_exact_surface() {
    let mut app = test_app();
    app.add_systems(
        OnEnter(Screen::Gameplay),
        spawn_anchored_deck.in_set(GameplaySetup::Terrain),
    );
    app.insert_resource(duel(
        EncounterPlacement::Anchor(DECK_ANCHOR.to_owned()),
        fixed(ENEMY_START),
    ));
    enter_gameplay(&mut app);

    let standing = standing_of(&mut app).expect("the anchored player should spawn");
    assert_eq!(standing.pos, TilePos::new(DECK_COORD, DECK_LEVEL));
}

// ---------------------------------------------------------------------------
// Rosters and formations.
//
// The scaffold these replace could only say "one player here, one enemy there".
// What is worth testing about a roster is everything the old shape could not be
// wrong about: that a group of units gets *distinct* surfaces, that those
// surfaces are ones the body can actually walk to, that the same encounter deals
// the same ones every run, and that a unit with nowhere to stand is a reported
// failure rather than a piece that quietly does not appear.
// ---------------------------------------------------------------------------

/// Every spawned unit's stable id and surface, in the order the ids were dealt.
fn placed_units(app: &mut App) -> Vec<(hex_core::UnitId, TilePos)> {
    let world = app.world_mut();
    let mut query = world.query::<(&hex_core::UnitId, &StandsOn)>();
    let mut placed: Vec<(hex_core::UnitId, TilePos)> = query
        .iter(world)
        .map(|(id, standing)| (*id, standing.0.pos))
        .collect();
    placed.sort_unstable();
    placed
}

/// A party of four at one anchor is the case the two-coordinate scaffold could not
/// express at all: they must end up on four *different* surfaces, each one the body
/// could walk to from the anchor.
#[test]
fn a_formation_spreads_a_party_over_distinct_walkable_surfaces() {
    let mut app = test_app();
    app.insert_resource(Encounter {
        name: "Warband".to_owned(),
        rosters: vec![
            roster(
                EncounterFaction::Player,
                EncounterPlacement::Formation {
                    center: FormationCenter::Fixed(CubeCoord { x: 0, y: 0, z: 0 }),
                    spread: 1,
                },
                &["hedge-mage", "raider", "raider", "wolf"],
            ),
            roster(EncounterFaction::Hostile, fixed(ENEMY_START), &["raider"]),
        ],
    });
    enter_gameplay(&mut app);

    let placed = placed_units(&mut app);
    assert_eq!(placed.len(), 5, "every rostered unit should be placed");

    let party: Vec<TilePos> = placed
        .iter()
        .take(4)
        .map(|(_, pos)| *pos)
        .collect::<Vec<_>>();
    let mut distinct = party.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(
        distinct.len(),
        4,
        "a formation stacked two units on one voxel: {party:?}"
    );
    assert_eq!(
        party.first().copied(),
        Some(TilePos::new(HexCoord::ORIGIN, GROUND_LEVEL)),
        "the first unit of a formation should stand on its centre"
    );
    for pos in &party {
        assert!(
            pos.coord.distance(HexCoord::ORIGIN) <= 1,
            "{pos:?} is outside the formation's spread of one step"
        );
        assert_eq!(
            pos.level, GROUND_LEVEL,
            "a formation put a unit on a buried run rather than a surface"
        );
    }
    assert_eq!(
        app.world().resource::<Party>().members.len(),
        4,
        "the party should hold every player-rostered unit"
    );
}

/// The same encounter on the same terrain deals the same surfaces. `Reach` is a hash
/// map, so nothing about its iteration order is a promise — a formation that sorted its
/// candidates by accident would make one launch's fight different from the next.
#[test]
fn a_formation_deals_the_same_surfaces_every_run() {
    let party_of_three = || Encounter {
        name: "Warband".to_owned(),
        rosters: vec![roster(
            EncounterFaction::Player,
            EncounterPlacement::Formation {
                center: FormationCenter::Fixed(CubeCoord { x: 0, y: 0, z: 0 }),
                spread: 2,
            },
            &["hedge-mage", "raider", "wolf"],
        )],
    };

    let mut first = test_app();
    first.insert_resource(party_of_three());
    enter_gameplay(&mut first);

    let mut second = test_app();
    second.insert_resource(party_of_three());
    enter_gameplay(&mut second);

    assert_eq!(placed_units(&mut first), placed_units(&mut second));
}

/// Exact placements are resolved before formations, so the unit that has to be
/// somewhere precise keeps its surface and the crowd flows around it.
#[test]
fn a_formation_flows_around_an_exact_placement() {
    let mut app = test_app();
    app.insert_resource(Encounter {
        name: "Sentry".to_owned(),
        rosters: vec![
            // Declared *second*, and centred on the surface the sentry holds.
            roster(
                EncounterFaction::Hostile,
                EncounterPlacement::Formation {
                    center: FormationCenter::Fixed(CubeCoord { x: 0, y: 0, z: 0 }),
                    spread: 1,
                },
                &["wolf", "wolf"],
            ),
            roster(
                EncounterFaction::Player,
                fixed(HexCoord::ORIGIN),
                &["sentry"],
            ),
        ],
    });
    enter_gameplay(&mut app);

    let centre = TilePos::new(HexCoord::ORIGIN, GROUND_LEVEL);
    assert_eq!(
        standing_of(&mut app).map(|standing| standing.pos),
        Some(centre),
        "the exact placement lost its surface to a formation"
    );
    let hostiles: Vec<TilePos> = {
        let world = app.world_mut();
        let mut query = world.query_filtered::<&StandsOn, With<Enemy>>();
        query.iter(world).map(|standing| standing.0.pos).collect()
    };
    assert_eq!(hostiles.len(), 2);
    assert!(
        !hostiles.contains(&centre),
        "a formation stood on top of an exact placement"
    );
}

/// A roster with more units than its formation has room for is a setup failure naming
/// the entry — never a unit that silently does not appear.
#[test]
fn a_formation_with_no_room_fails_setup_with_a_reason() {
    let mut app = test_app();
    app.insert_resource(Encounter {
        name: "Crowded".to_owned(),
        rosters: vec![roster(
            EncounterFaction::Player,
            // Spread zero is the centre and nothing else, so the second unit has
            // nowhere to go.
            EncounterPlacement::Formation {
                center: FormationCenter::Fixed(CubeCoord { x: 0, y: 0, z: 0 }),
                spread: 0,
            },
            &["hedge-mage", "raider"],
        )],
    });
    enter_gameplay(&mut app);

    assert!(
        placed_units(&mut app).is_empty(),
        "a roster that cannot be placed must not half-spawn"
    );
    let reason = &app.world().resource::<GameplaySetupFailure>().reason;
    assert!(
        reason.contains("no free surface") && reason.contains("raider"),
        "the failure should name the entry that had nowhere to stand: {reason}"
    );
}

/// Deployment preparation needs stable actor identities before the tester assigns
/// exact surfaces, but its authored regions are not actor placements. A deliberately
/// impossible spread-zero 6v6 must therefore stage on canonical live footing rather
/// than fail before the Deployment screen can report region capacity.
#[test]
fn deployment_preparation_stages_a_full_roster_without_using_formation_capacity() {
    let encounter = || Encounter {
        name: "Crowded Deployment".to_owned(),
        rosters: vec![
            roster(
                EncounterFaction::Player,
                EncounterPlacement::Formation {
                    center: FormationCenter::Fixed(CubeCoord { x: 0, y: 0, z: 0 }),
                    spread: 0,
                },
                &[
                    "hedge-mage",
                    "raider",
                    "wolf",
                    "hedge-mage",
                    "raider",
                    "wolf",
                ],
            ),
            roster(
                EncounterFaction::Hostile,
                EncounterPlacement::Formation {
                    center: FormationCenter::Fixed(CubeCoord {
                        x: ENEMY_START.x(),
                        y: ENEMY_START.y(),
                        z: ENEMY_START.z(),
                    }),
                    spread: 0,
                },
                &[
                    "hedge-mage",
                    "raider",
                    "wolf",
                    "hedge-mage",
                    "raider",
                    "wolf",
                ],
            ),
        ],
    };
    let run = || {
        let mut app = test_app();
        app.insert_resource(GameplayPhase::Preparing);
        app.insert_resource(encounter());
        enter_gameplay(&mut app);
        assert!(
            !app.world().contains_resource::<GameplaySetupFailure>(),
            "deployment staging must reach the exact-placement screen"
        );
        let placed = placed_units(&mut app);
        assert_eq!(placed.len(), 12);
        assert_eq!(
            placed
                .iter()
                .map(|(_, position)| *position)
                .collect::<BTreeSet<_>>()
                .len(),
            12,
            "staging may not overlap bodies"
        );
        assert_eq!(app.world().resource::<Party>().members.len(), 6);
        assert_eq!(
            placed.first().map(|(_, position)| *position),
            Some(TilePos::new(HexCoord::ORIGIN, GROUND_LEVEL)),
            "the first player must retain its authored formation centre"
        );
        assert_eq!(
            placed.get(6).map(|(_, position)| *position),
            Some(TilePos::new(ENEMY_START, GROUND_LEVEL)),
            "the first hostile must retain its authored formation centre"
        );
        placed
    };

    assert_eq!(
        run(),
        run(),
        "hidden staging must be deterministic across fresh launches"
    );
}

/// Every unit carries the archetype its roster named. Nothing resolves it yet — this is
/// the key an archetype's lattice will be looked up by, and the test that it survives
/// the spawn path at all.
#[test]
fn a_spawned_unit_carries_its_archetype_and_a_stable_id() {
    let mut app = test_app();
    app.insert_resource(Encounter {
        name: "Named".to_owned(),
        rosters: vec![
            roster(
                EncounterFaction::Player,
                EncounterPlacement::Formation {
                    center: FormationCenter::Fixed(CubeCoord { x: 0, y: 0, z: 0 }),
                    spread: 1,
                },
                &["hedge-mage", "raider"],
            ),
            roster(EncounterFaction::Hostile, fixed(ENEMY_START), &["wolf"]),
        ],
    });
    enter_gameplay(&mut app);

    let world = app.world_mut();
    let mut query = world.query::<(&hex_core::UnitId, &hex_units::Archetype)>();
    let mut named: Vec<(u64, String)> = query
        .iter(world)
        .map(|(id, archetype)| (id.0, archetype.0.clone()))
        .collect();
    named.sort_unstable();

    // Ids are dealt in declaration order, so the roster order *is* the id order.
    assert_eq!(
        named,
        vec![
            (0, "hedge-mage".to_owned()),
            (1, "raider".to_owned()),
            (2, "wolf".to_owned()),
        ]
    );
}

fn remove_terrain_ready(mut commands: Commands) {
    commands.remove_resource::<TerrainReady>();
}

/// Failed terrain setup leaves `TerrainReady` absent. Actors are gated on that marker
/// rather than accepting the tile entities a failed or partial generator may have
/// emitted before it stopped.
#[test]
fn units_do_not_spawn_until_terrain_is_ready() {
    let mut app = test_app();
    app.add_systems(
        OnEnter(Screen::Gameplay),
        remove_terrain_ready
            .after(spawn_fake_terrain)
            .in_set(GameplaySetup::Terrain),
    );
    enter_gameplay(&mut app);

    assert!(single::<With<Player>>(&mut app).is_none());
    assert!(single::<With<Enemy>>(&mut app).is_none());
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

fn unit_material_color<Q: bevy::ecs::query::QueryFilter>(app: &mut App) -> Option<Color> {
    let unit = single::<Q>(app)?;
    let handle = {
        let mut pieces = app
            .world_mut()
            .query_filtered::<(&ChildOf, &MeshMaterial3d<StandardMaterial>), Without<UnitRing>>();
        pieces
            .iter(app.world())
            .find(|(parent, _)| parent.parent() == unit)
            .map(|(_, material)| material.0.clone())?
    };
    app.world()
        .resource::<Assets<StandardMaterial>>()
        .get(&handle)
        .map(|material| material.base_color)
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

#[test]
fn one_hundred_idle_frames_retain_the_exact_selection_overlays() {
    let mut app = test_app();
    enter_gameplay(&mut app);
    take_a_turn(&mut app, 2).expect("a player should exist during gameplay");
    hover(&mut app, HexCoord::new_cubic(2, -2, 0)).expect("the fixture covers this coordinate");

    let before: BTreeSet<_> = {
        let mut overlays = app.world_mut().query_filtered::<Entity, Or<(
            With<RangeOverlay>,
            With<PathOverlay>,
            With<UnitRing>,
        )>>();
        overlays.iter(app.world()).collect()
    };
    assert!(
        !before.is_empty(),
        "setup failed to draw selection overlays"
    );

    for _ in 0..100 {
        app.update();
    }

    let after: BTreeSet<_> = {
        let mut overlays = app.world_mut().query_filtered::<Entity, Or<(
            With<RangeOverlay>,
            With<PathOverlay>,
            With<UnitRing>,
        )>>();
        overlays.iter(app.world()).collect()
    };
    assert_eq!(
        after, before,
        "idle selection reconciliation despawned and rebuilt unchanged overlays"
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
/// this body; the piece is standing on another; and the enemy occupies one exact
/// surface. Sixteen remain.
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
        16,
        "two hexes of movement should reach sixteen unoccupied surfaces, got {positions:?}"
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
    assert!(
        !positions.iter().any(|pos| pos.coord == ENEMY_START),
        "another body's exact surface must not be offered"
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

#[test]
fn lava_is_drawn_but_is_not_footing() {
    let mut app = test_app();
    enter_gameplay(&mut app);
    take_a_turn(&mut app, 4).expect("a player should exist during gameplay");

    let mut tinted = app
        .world_mut()
        .query_filtered::<&TilePos, With<RangeOverlay>>();
    let reachable: Vec<TilePos> = tinted.iter(app.world()).copied().collect();

    assert!(
        !reachable.iter().any(|pos| pos.coord == LAVA_POOL),
        "lava was offered as somewhere to walk to"
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

#[test]
fn clicking_a_generated_feature_blocker_emits_no_move() {
    let mut app = test_app();
    let mut blockers = TraversalBlockers::new();
    assert!(blockers.insert(TilePos::new(TREE_ROOT, GROUND_LEVEL)));
    app.insert_resource(blockers);
    enter_gameplay(&mut app);

    let mut tiles = app
        .world_mut()
        .query_filtered::<(Entity, &TilePos), With<HexTile>>();
    let tree_root = tiles
        .iter(app.world())
        .find(|(_, pos)| pos.coord == TREE_ROOT && pos.level == GROUND_LEVEL)
        .map(|(entity, _)| entity)
        .expect("the fixture has solid terrain under the generated tree");

    let window = app.world_mut().spawn(Window::default()).id();
    click(&mut app, tree_root, window);
    app.update();

    assert!(
        app.world().resource::<CommandQueue>().is_empty(),
        "a click must not route the selected unit into a generated tree root"
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

/// A second click while the piece is still walking emits nothing at all.
///
/// An unguarded second click would route from a stale `StandsOn` and queue a
/// second move on top of the one still playing. The observer's mid-walk
/// suppression kills it at the source; the applier's `Busy` gate is the
/// authoritative copy of the same rule, and double-billing specifically is
/// covered by `hex_combat`'s funnel tests where the budget actually lives.
#[test]
fn a_second_click_while_moving_emits_nothing() {
    let mut app = test_app();
    enter_gameplay(&mut app);
    take_a_turn(&mut app, 4).expect("a player should exist during gameplay");

    let target = surface_at(&mut app, HexCoord::new_cubic(2, -2, 0))
        .expect("the fake terrain covers this coordinate");
    let window = app.world_mut().spawn(Window::default()).id();

    click(&mut app, target, window);
    commit_move(&mut app).expect("the emitted move should ground and start");
    app.update();

    // **A different tile**, and that detail is the test. Clicking the same one again
    // routes from the destination to itself for a cost of zero, so nothing would be
    // emitted whether or not the guard exists and the test proves nothing. The
    // double emission only appears when the second click has somewhere new to go.
    let elsewhere = surface_at(&mut app, HexCoord::new_cubic(2, 0, -2))
        .expect("the fake terrain covers this coordinate");
    click(&mut app, elsewhere, window);

    assert!(
        app.world().resource::<CommandQueue>().is_empty(),
        "a click during a walk was emitted as a second command"
    );
}

/// Removing presentation cannot unlock or complete a domain route.
///
/// `MovingTo` and `Busy`, not animation component presence, protect the exact
/// logical route. A presentation teardown while the route is active must neither
/// emit another command nor teleport to the destination.
#[test]
fn presentation_teardown_during_a_domain_route_is_ignored() {
    let mut app = test_app();
    enter_gameplay(&mut app);
    take_a_turn(&mut app, 4).expect("a player should exist during gameplay");

    let destination = HexCoord::new_cubic(2, -2, 0);
    let target =
        surface_at(&mut app, destination).expect("the fake terrain covers this coordinate");
    let window = app.world_mut().spawn(Window::default()).id();
    click(&mut app, target, window);
    commit_move(&mut app).expect("the emitted move should ground and start");
    app.update();

    let player = single::<With<Player>>(&mut app).expect("a player should exist");
    assert!(app.world().get::<MovingTo>(player).is_some());

    let before = standing_of(&mut app).expect("the player has an exact surface");
    app.world_mut()
        .entity_mut(player)
        .remove::<Transformation>();
    let elsewhere = surface_at(&mut app, HexCoord::new_cubic(2, 0, -2))
        .expect("the fake terrain covers this coordinate");
    click(&mut app, elsewhere, window);

    assert!(
        app.world().resource::<CommandQueue>().is_empty(),
        "the landing-frame click was emitted as a command"
    );
    assert_eq!(
        standing_of(&mut app),
        Some(before),
        "presentation teardown changed gameplay position"
    );
    assert!(
        app.world().get::<MovingTo>(player).is_some(),
        "presentation teardown completed the domain route"
    );

    settle_movement(&mut app, player);

    assert_eq!(
        standing_of(&mut app).map(|standing| standing.pos.coord),
        Some(destination),
        "the pending arrival should land untouched"
    );
    assert!(app.world().get::<MovingTo>(player).is_none());
    assert!(
        app.world().get::<Busy>(player).is_none(),
        "the completed domain route retained its legality gate"
    );
    assert!(app.world().get::<Transformation>(player).is_none());
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
    commit_move(&mut app).expect("the emitted move should ground and start");
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

/// Stable ids are dealt in scenario spawn order — player first — recorded in
/// the registry and the party, and reset with the session so the same launch
/// always deals the same ids. That determinism is what lets a save or replay
/// name units without caring which entities they landed on this run.
#[test]
fn unit_ids_follow_spawn_order_and_reset_between_sessions() {
    use hex_core::UnitId;

    let mut app = test_app();
    enter_gameplay(&mut app);

    let ids = |app: &mut App| {
        let mut players = app
            .world_mut()
            .query_filtered::<(Entity, &UnitId), With<Player>>();
        let (player_entity, player_id) = players
            .single(app.world())
            .map(|(entity, id)| (entity, *id))
            .expect("one player with an id");
        let mut enemies = app.world_mut().query_filtered::<&UnitId, With<Enemy>>();
        let enemy_id = *enemies.single(app.world()).expect("one enemy with an id");
        (player_entity, player_id, enemy_id)
    };

    let (player_entity, player_id, enemy_id) = ids(&mut app);
    assert_eq!(player_id, UnitId(0), "the player spawns first");
    assert_eq!(enemy_id, UnitId(1), "the enemy spawns second");

    let registry = app.world().resource::<UnitRegistry>();
    assert_eq!(registry.entity_of(player_id), Some(player_entity));
    assert_eq!(registry.id_of(player_entity), Some(player_id));
    assert_eq!(
        app.world()
            .entity(player_entity)
            .get::<hex_core::ControlOwner>(),
        Some(&hex_core::ControlOwner::default()),
        "spawned units carry the seat-0 ownership marker"
    );
    assert_eq!(
        app.world().resource::<Party>().members,
        vec![player_id],
        "only player-faction units enrol in the party"
    );

    // Leaving tears identity down; re-entering deals the same ids again.
    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Title);
    app.update();
    app.update();
    assert!(app.world().resource::<Party>().members.is_empty());
    assert_eq!(
        app.world().resource::<UnitRegistry>().entity_of(UnitId(0)),
        None,
        "the registry must not outlive its units"
    );

    enter_gameplay(&mut app);
    let (_, player_again, enemy_again) = ids(&mut app);
    assert_eq!(
        player_again,
        UnitId(0),
        "a fresh session re-deals from zero"
    );
    assert_eq!(enemy_again, UnitId(1));
}

/// A unit spawns carrying the lattice its archetype names, and one whose archetype has
/// no lattice spawns without one.
///
/// The content-level tests prove `lattices.ron` resolves and that the drawings work;
/// **none of them touches the spawn path**, so deleting the insert in `spawn_unit`
/// would leave every one of them green. This runs the real `GameplaySetup` chain with a
/// real `LatticeLibrary` resource and asserts on the components that came out.
///
/// The negative half matters as much: an archetype the library does not define must
/// still spawn a unit — walkable, strikeable, and merely inert — rather than failing
/// setup, because that is the behaviour the loud CI check is allowed to rely on.
#[test]
fn a_unit_spawns_with_its_archetypes_lattice_and_without_one_it_lacks() {
    let mut app = test_app();

    // Two archetypes, one defined and one not. `known` is a single gem, which is the
    // smallest thing that is still recognisably a lattice.
    let mut cells = std::collections::BTreeMap::new();
    cells.insert(
        hex_core::LatticeCoord::ORIGIN,
        hex_lattice::CellKind::Gem {
            element: hex_core::ElementId(0),
        },
    );
    let mut capacity = std::collections::BTreeMap::new();
    capacity.insert(hex_core::ElementId(0), 3);
    let mut library = hex_assets::LatticeLibrary::default();
    library.insert(
        "known".to_owned(),
        hex_assets::Archetype {
            spec: hex_lattice::LatticeSpec::new(cells),
            stats: hex_lattice::LatticeStats::new(capacity, std::collections::BTreeMap::new()),
            ai_profile: None,
        },
    );
    app.insert_resource(library);

    app.insert_resource(Encounter {
        name: "Lattices".to_owned(),
        rosters: vec![
            roster(
                EncounterFaction::Player,
                fixed(HexCoord::ORIGIN),
                &["known"],
            ),
            roster(
                EncounterFaction::Hostile,
                fixed(ENEMY_START),
                &["undefined"],
            ),
        ],
    });
    enter_gameplay(&mut app);

    let world = app.world_mut();
    let mut query = world.query::<(
        &hex_units::Archetype,
        Option<&hex_lattice::LatticeSpec>,
        Option<&hex_lattice::LatticeState>,
        Option<&hex_lattice::LatticeStats>,
    )>();
    let mut seen: Vec<(String, bool, bool, bool)> = query
        .iter(world)
        .map(|(archetype, spec, state, stats)| {
            (
                archetype.0.clone(),
                spec.is_some(),
                state.is_some(),
                stats.is_some(),
            )
        })
        .collect();
    seen.sort();

    assert_eq!(
        seen,
        vec![
            ("known".to_owned(), true, true, true),
            ("undefined".to_owned(), false, false, false),
        ],
        "the defined archetype should carry all three components and the undefined one none"
    );

    // And the state was built against *this* spec: one gem, filled to its attunement.
    let mut states = world.query::<&hex_lattice::LatticeState>();
    let state = states.iter(world).next().expect("one unit has a lattice");
    assert_eq!(
        state.mana(hex_core::LatticeCoord::ORIGIN),
        3,
        "a fresh gem should hold its element's attunement, not zero"
    );
}
