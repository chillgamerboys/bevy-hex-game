//! Integration tests for the combat loop.
//!
//! Headless, with stand-in units rather than real ones — `hex_combat` consumes
//! `Faction`, `StandsOn` and `Initiative`, and anything producing those will do. That
//! is the same trick `hex_units`' tests play on the map, and it is the clearest
//! available proof that the crate boundary is real rather than decorative.
//!
//! Nothing here is visual. Whether a fight *reads* as a fight needs a person.

use bevy::app::PluginsState;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input::ButtonState;
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;

use hex_combat::{Initiative, TurnOrder};
use hex_core::{HexCoord, HexSpan, Mode, Screen, TilePos, Turn, UnitId};
use hex_units::{Faction, Standing, StandsOn};

/// Far enough apart that no fight starts on its own.
const FAR: i32 = 12;

fn test_app() -> App {
    let mut app = App::new();
    // `InputPlugin` is not optional here: ending a turn reads the keyboard, and
    // `MinimalPlugins` provides no `ButtonInput<KeyCode>` at all. Without it every
    // test fails with "Resource does not exist" from inside the scheduler, which
    // names neither the system nor the resource.
    app.add_plugins((MinimalPlugins, StatesPlugin, bevy::input::InputPlugin));
    app.init_state::<Screen>();
    app.add_sub_state::<Mode>();
    app.add_plugins(hex_combat::plugin);

    while app.plugins_state() != PluginsState::Cleaned {
        app.finish();
        app.cleanup();
    }
    app
}

/// The stable id combat dealt this entity when the fight began.
#[expect(
    clippy::expect_used,
    reason = "test helper outside a #[test] fn; a missing id IS the failure"
)]
fn unit_id(app: &App, entity: Entity) -> UnitId {
    *app.world()
        .entity(entity)
        .get::<UnitId>()
        .expect("combat should have dealt this unit a stable id")
}

/// A unit at a coordinate. Only the components `hex_combat` actually reads.
fn spawn_unit(app: &mut App, faction: Faction, coord: HexCoord, initiative: u32) -> Entity {
    app.world_mut()
        .spawn((
            faction,
            StandsOn(Standing {
                pos: TilePos::new(coord, 1),
                span: HexSpan::new(0.0, 1.0),
            }),
            Initiative(initiative),
        ))
        .id()
}

fn enter_gameplay(app: &mut App) {
    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Gameplay);
    app.update();
    app.update();
}

fn mode(app: &App) -> Mode {
    *app.world().resource::<State<Mode>>().get()
}

/// Two units within engage range start a fight, and everyone present joins the order.
#[test]
fn closing_the_distance_starts_a_fight() {
    let mut app = test_app();
    let player = spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20);
    let enemy = spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(2, -2, 0),
        10,
    );
    enter_gameplay(&mut app);
    app.update();

    assert_eq!(
        mode(&app),
        Mode::Combat,
        "a nearby hostile should start a fight"
    );

    let order = app.world().resource::<TurnOrder>();
    assert_eq!(
        order.order().len(),
        2,
        "both units should be in the turn order"
    );
    assert_eq!(
        order.position_of(unit_id(&app, player)),
        Some(0),
        "the higher initiative acts first"
    );
    assert_eq!(order.position_of(unit_id(&app, enemy)), Some(1));
}

/// Units far apart stay in real time. Without this the game would open in combat.
#[test]
fn distant_units_stay_in_real_time() {
    let mut app = test_app();
    spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20);
    spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(FAR, -FAR, 0),
        10,
    );
    enter_gameplay(&mut app);
    app.update();

    assert_eq!(mode(&app), Mode::Exploring);
    assert!(app.world().resource::<TurnOrder>().is_empty());
}

/// One side alone is not a fight. `None` for "nobody to fight" has to be distinct
/// from "far away", or a lone player would flip into combat with nothing.
#[test]
fn a_lone_unit_never_fights() {
    let mut app = test_app();
    spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20);
    enter_gameplay(&mut app);
    app.update();

    assert_eq!(mode(&app), Mode::Exploring);
}

/// Exactly one unit holds the turn at a time.
#[test]
fn exactly_one_unit_acts_at_a_time() {
    let mut app = test_app();
    spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20);
    spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(2, -2, 0),
        10,
    );
    enter_gameplay(&mut app);
    app.update();

    let acting = app
        .world_mut()
        .query_filtered::<Entity, With<Turn>>()
        .iter(app.world())
        .count();
    assert_eq!(acting, 1);
}

/// Ending the last unit's turn wraps to the first and counts a round.
#[test]
fn ending_the_last_turn_wraps_and_counts_a_round() {
    let mut app = test_app();
    let player = spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20);
    let enemy = spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(2, -2, 0),
        10,
    );
    enter_gameplay(&mut app);
    app.update();

    assert_eq!(
        app.world().resource::<TurnOrder>().current(),
        Some(unit_id(&app, player))
    );

    end_turn(&mut app);
    assert_eq!(
        app.world().resource::<TurnOrder>().current(),
        Some(unit_id(&app, enemy)),
        "the turn should pass to the next unit"
    );
    assert_eq!(app.world().resource::<TurnOrder>().round, 0);

    end_turn(&mut app);
    assert_eq!(
        app.world().resource::<TurnOrder>().current(),
        Some(unit_id(&app, player)),
        "the order should wrap to the front"
    );
    assert_eq!(
        app.world().resource::<TurnOrder>().round,
        1,
        "a full cycle is one round"
    );
}

/// Walking away ends the fight and clears the order, so nothing keeps a turn.
#[test]
fn retreating_ends_the_fight() {
    let mut app = test_app();
    let player = spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20);
    spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(2, -2, 0),
        10,
    );
    enter_gameplay(&mut app);
    app.update();
    assert_eq!(mode(&app), Mode::Combat, "precondition: fighting");

    // Teleport the player well clear of the disengage margin.
    let far = HexCoord::new_cubic(FAR, -FAR, 0);
    if let Some(mut standing) = app.world_mut().get_mut::<StandsOn>(player) {
        standing.0.pos = TilePos::new(far, 1);
    }
    app.update();
    app.update();

    assert_eq!(mode(&app), Mode::Exploring);
    assert!(
        app.world().resource::<TurnOrder>().is_empty(),
        "the order should be cleared when the fight ends"
    );
    let acting = app
        .world_mut()
        .query_filtered::<Entity, With<Turn>>()
        .iter(app.world())
        .count();
    assert_eq!(acting, 0, "nobody should still hold a turn");
}

/// A hostile just past engage range but inside the margin must **not** restart the
/// fight it just left. Without the margin a unit on the boundary flips every frame.
#[test]
fn the_disengage_margin_stops_combat_flapping() {
    let mut app = test_app();
    let player = spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20);
    spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(2, -2, 0),
        10,
    );
    enter_gameplay(&mut app);
    app.update();
    assert_eq!(mode(&app), Mode::Combat, "precondition: fighting");

    // Distance 5: past the engage range of 4, but inside the margin of 2 beyond it.
    if let Some(mut standing) = app.world_mut().get_mut::<StandsOn>(player) {
        standing.0.pos = TilePos::new(HexCoord::new_cubic(5, -5, 0), 1);
    }
    app.update();
    app.update();

    assert_eq!(
        mode(&app),
        Mode::Combat,
        "stepping just outside engage range should not end the fight"
    );
}

/// Leaving gameplay clears the order, so a new session cannot inherit entities that
/// no longer exist.
#[test]
fn leaving_gameplay_clears_the_order() {
    let mut app = test_app();
    spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20);
    spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(2, -2, 0),
        10,
    );
    enter_gameplay(&mut app);
    app.update();
    assert!(!app.world().resource::<TurnOrder>().is_empty());

    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Title);
    app.update();
    app.update();

    assert!(app.world().resource::<TurnOrder>().is_empty());
}

/// Presses the end-turn key for one frame.
///
/// Sends a real `KeyboardInput` rather than calling `ButtonInput::press` directly.
/// Bevy's `keyboard_input_system` clears the button state at the start of *every*
/// frame before processing events, so a press written straight into the resource is
/// wiped before any `Update` system sees it — the key looks stuck down and nothing
/// ever reports `just_pressed`.
fn end_turn(app: &mut App) {
    let window = app.world_mut().spawn(()).id();
    app.world_mut().write_message(KeyboardInput {
        key_code: KeyCode::Space,
        logical_key: Key::Space,
        state: ButtonState::Pressed,
        text: None,
        repeat: false,
        window,
    });
    app.update();
    app.world_mut().write_message(KeyboardInput {
        key_code: KeyCode::Space,
        logical_key: Key::Space,
        state: ButtonState::Released,
        text: None,
        repeat: false,
        window,
    });
    app.update();
}

/// A unit that already carries an explicit id (a test's, or a future load
/// path's) must still resolve through the registry — deleting the upsert in
/// `begin_combat` makes the wrap back to it silently stall.
#[test]
fn a_carried_id_still_receives_its_turn_on_the_wrap() {
    let mut app = test_app();
    let player = spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20);
    spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(2, -2, 0),
        10,
    );
    app.world_mut().entity_mut(player).insert(UnitId(5));
    enter_gameplay(&mut app);
    app.update();

    assert_eq!(
        app.world().resource::<TurnOrder>().current(),
        Some(UnitId(5)),
        "the carried id should appear in the order untouched"
    );

    end_turn(&mut app);
    end_turn(&mut app);

    assert_eq!(
        app.world().resource::<TurnOrder>().current(),
        Some(UnitId(5)),
        "the wrap should return to the carried id"
    );
    assert!(
        app.world().get::<Turn>(player).is_some(),
        "the registry must resolve a carried id back to its entity"
    );
}
