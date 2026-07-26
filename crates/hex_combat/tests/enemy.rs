//! Integration tests for what an enemy does with its turn.
//!
//! The AI is a placeholder, but "closes the distance and swings" is still a claim
//! that can be wrong in specific ways — walking onto the player, overshooting its
//! movement budget, or never ending its turn and stalling the fight. Those are what
//! these check.
//!
//! Terrain is spawned by the test, because `hex_combat` cannot see `hex_map` and does
//! not need to: it consumes `TilePos`, `HexSpan`, `SubstanceId` and `Headroom`.

use bevy::app::PluginsState;
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;

use hex_anim::Transformation;
use hex_assets::{PlayerSettings, Substance, SubstanceFile, SubstanceTable};
use hex_combat::{Initiative, TurnOrder};
use hex_core::{
    Headroom, HexCoord, HexSpan, HexTile, Mode, Screen, SubstanceId, TilePos, Turn, MAX_HEADROOM,
};
use hex_units::{Body, Faction, Standing, StandsOn};

const GROUND: f32 = 2.0;
const GROUND_LEVEL: hex_core::Level = 1;
const STONE: SubstanceId = SubstanceId(1);

fn test_app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin, bevy::input::InputPlugin));
    app.init_state::<Screen>();
    app.add_sub_state::<Mode>();
    app.insert_resource(substance_table());
    app.insert_resource(PlayerSettings {
        scale: 0.25,
        speed: 5.0,
        color: (1.0, 0.2, 0.2),
        levels_tall: 2,
    });
    app.add_systems(OnEnter(Screen::Gameplay), spawn_terrain);
    app.add_plugins(hex_combat::plugin);

    while app.plugins_state() != PluginsState::Cleaned {
        app.finish();
        app.cleanup();
    }
    app
}

/// Flat, walkable ground with plenty of headroom.
fn spawn_terrain(mut commands: Commands) {
    for coord in HexCoord::ORIGIN.within_radius(10) {
        commands.spawn((
            HexTile,
            coord,
            TilePos::new(coord, GROUND_LEVEL),
            HexSpan::new(GROUND - 1.0, GROUND),
            STONE,
            Headroom(MAX_HEADROOM),
        ));
    }
}

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

fn spawn_unit(app: &mut App, faction: Faction, coord: HexCoord, initiative: u32) -> Entity {
    let standing = Standing {
        pos: TilePos::new(coord, GROUND_LEVEL),
        span: HexSpan::new(GROUND - 1.0, GROUND),
    };
    let mut unit = app.world_mut().spawn((
        faction,
        StandsOn(standing),
        Body { levels_tall: 2 },
        Initiative(initiative),
        Transform::from_translation(standing.world_position()),
    ));
    if faction == Faction::Hostile {
        unit.insert(hex_units::Enemy);
    } else {
        unit.insert(hex_units::Player);
    }
    unit.id()
}

fn enter_gameplay(app: &mut App) {
    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Gameplay);
    app.update();
    app.update();
    app.update();
}

/// Where a unit currently stands.
///
/// Returns an `Option` rather than unwrapping: the restriction lints are only relaxed
/// inside `#[test]` functions, not in helpers they call, so a panic belongs at the
/// call site where it is also more informative.
fn coord_of(app: &App, entity: Entity) -> Option<HexCoord> {
    Some(app.world().get::<StandsOn>(entity)?.0.pos.coord)
}

/// The enemy has a lower initiative here, so the player acts first. Ending the
/// player's turn hands over, and the enemy closes the distance on its own.
#[test]
fn an_enemy_closes_the_distance_on_its_turn() {
    let mut app = test_app();
    spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20);
    let enemy = spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(4, -4, 0),
        10,
    );
    enter_gameplay(&mut app);

    let before = coord_of(&app, enemy).expect("the enemy exists");
    assert_eq!(before.distance(HexCoord::ORIGIN), 4, "precondition");

    end_turn(&mut app);
    app.update();

    let after = coord_of(&app, enemy).expect("the enemy exists");
    assert!(
        after.distance(HexCoord::ORIGIN) < before.distance(HexCoord::ORIGIN),
        "the enemy should have moved closer; it is at {after:?}"
    );
}

/// It must stop *next to* the player, never on top of it. Walking onto the target
/// would put two units on one surface, which the model has no way to express.
#[test]
fn an_enemy_stops_adjacent_rather_than_on_top() {
    let mut app = test_app();
    spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20);
    let enemy = spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(3, -3, 0),
        10,
    );
    enter_gameplay(&mut app);

    end_turn(&mut app);
    app.update();

    let after = coord_of(&app, enemy).expect("the enemy exists");
    assert_ne!(after, HexCoord::ORIGIN, "the enemy walked onto the player");
    assert!(
        after.distance(HexCoord::ORIGIN) >= 1,
        "the enemy should stop adjacent at the closest"
    );
}

/// A turn's movement budget is a limit, not a suggestion. An enemy far away must not
/// cross the whole map in one go.
#[test]
fn an_enemy_cannot_outrun_its_movement_budget() {
    let mut app = test_app();
    spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20);
    // Distance 6, inside the disengage margin so the fight holds, but further than
    // one turn's movement.
    let enemy = spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(6, -6, 0),
        10,
    );
    enter_gameplay(&mut app);

    // Force the fight: this distance is outside engage range on its own.
    app.world_mut()
        .resource_mut::<NextState<Mode>>()
        .set(Mode::Combat);
    app.update();
    app.update();

    let before = coord_of(&app, enemy).expect("the enemy exists");
    end_turn(&mut app);
    app.update();
    let after = coord_of(&app, enemy).expect("the enemy exists");

    let travelled = before.distance(after);
    assert!(
        travelled <= 4,
        "the enemy moved {travelled} hexes on a four-hex budget"
    );
    assert!(travelled > 0, "the enemy should have moved at all");
}

/// A turn must not pass while its unit is still mid-stride. Advancing early cuts the
/// animation off and strands the piece between two hexes.
///
/// Note the test does **not** wait for the animation to play: `app.update()` in a
/// tight loop advances the clock by microseconds, so a real 0.36s lunge would never
/// finish however many frames were run. The contract is expressed in terms of the
/// component, so the test is too.
#[test]
fn a_turn_does_not_pass_while_a_unit_is_still_moving() {
    let mut app = test_app();
    spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20);
    let enemy = spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(3, -3, 0),
        10,
    );
    enter_gameplay(&mut app);

    end_turn(&mut app);

    assert!(
        app.world().get::<Transformation>(enemy).is_some(),
        "precondition: the enemy should be moving"
    );
    assert_eq!(
        app.world().resource::<TurnOrder>().current(),
        Some(enemy),
        "the turn must stay with a unit that is still animating"
    );
}

/// Once the animation is done, the turn comes back. An enemy that never finishes
/// stalls the fight with no way for the player to recover.
#[test]
fn an_enemy_turn_ends_once_its_animation_finishes() {
    let mut app = test_app();
    let player = spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20);
    let enemy = spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(3, -3, 0),
        10,
    );
    enter_gameplay(&mut app);

    end_turn(&mut app);

    // Stand in for the animation completing. Removing the component is exactly what
    // `hex_anim`'s driver does when a transformer reports itself finished.
    app.world_mut().entity_mut(enemy).remove::<Transformation>();
    app.update();
    app.update();

    assert_eq!(
        app.world().resource::<TurnOrder>().current(),
        Some(player),
        "the turn should have come back to the player"
    );
    assert!(
        app.world().get::<Turn>(player).is_some(),
        "the player should hold the turn marker again"
    );
}

/// An enemy already next to the player attacks instead of moving — and an attack
/// leaves it exactly where it started.
#[test]
fn an_adjacent_enemy_attacks_without_moving() {
    let mut app = test_app();
    spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20);
    let adjacent = HexCoord::new_cubic(1, -1, 0);
    let enemy = spawn_unit(&mut app, Faction::Hostile, adjacent, 10);
    enter_gameplay(&mut app);

    end_turn(&mut app);
    // Long enough for the lunge to play out and be removed.
    for _ in 0..240 {
        app.update();
    }

    assert_eq!(
        coord_of(&app, enemy),
        Some(adjacent),
        "an attack should not change which surface the enemy stands on"
    );
}

/// Presses the end-turn key for one frame.
///
/// A real `KeyboardInput`, not a direct `ButtonInput::press`: Bevy clears the button
/// state at the start of every frame before processing events, so a direct press
/// never reaches an `Update` system.
fn end_turn(app: &mut App) {
    use bevy::input::keyboard::{Key, KeyboardInput};
    use bevy::input::ButtonState;

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
