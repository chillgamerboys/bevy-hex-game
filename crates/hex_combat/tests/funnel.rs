//! Integration tests for the command funnel: one applier, replayable input.
//!
//! Commands are pushed straight into the [`CommandQueue`] — the same resource
//! every emitter writes — so these tests cover the applier's contract without
//! caring which input produced the command. Emission itself is covered where
//! the emitters live: the click observer in `hex_units`' tests, the end-turn
//! key in `loop.rs`, the AI in `enemy.rs`.
//!
//! The heart of the file is the replay test: the funnel exists so that the
//! sim's entire input is an ordered command sequence, and a sequence applied
//! twice from the same spawn state must land the same world twice.

use std::time::Duration;

use bevy::app::PluginsState;
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;

use hex_assets::{PlayerSettings, Substance, SubstanceFile, SubstanceTable};
use hex_combat::{Initiative, TurnOrder};
use hex_core::{
    Busy, CommandQueue, GameCommand, Headroom, HexCoord, HexSpan, HexTile, IssuedCommand, Mode,
    PlayerSeat, Screen, SubstanceId, TilePos, Turn, UnitId, MAX_HEADROOM,
};
use hex_units::{Body, Faction, Standing, StandsOn, UnitRegistry};

const GROUND: f32 = 2.0;
const GROUND_LEVEL: hex_core::Level = 1;
const STONE: SubstanceId = SubstanceId(1);

fn test_app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin, bevy::input::InputPlugin));
    app.init_state::<Screen>();
    // The shipped combat.ron values; production loads the file instead.
    app.insert_resource(hex_assets::CombatSettings::default());
    app.add_sub_state::<Mode>();
    app.insert_resource(substance_table());
    app.insert_resource(PlayerSettings {
        scale: 0.25,
        speed: 5.0,
        color: (1.0, 0.2, 0.2),
    });
    // A fixed tick makes every run take the same frames through the same
    // animations — which the replay test depends on to mean anything.
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        Duration::from_millis(100),
    ));
    app.add_systems(OnEnter(Screen::Gameplay), spawn_terrain);
    app.add_plugins((
        hex_anim::plugin,
        hex_units::movement::plugin,
        hex_combat::plugin,
    ));

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

/// A unit with an explicit, pre-registered id.
///
/// Deliberately no `Enemy` marker on hostiles: the AI emits for marked enemies,
/// and these tests script every command themselves. `begin_combat` upserts the
/// carried ids, so the explicit registration and the dealt order agree.
fn spawn_unit(
    app: &mut App,
    faction: Faction,
    coord: HexCoord,
    initiative: u32,
    id: u64,
) -> Entity {
    let standing = Standing {
        pos: TilePos::new(coord, GROUND_LEVEL),
        span: HexSpan::new(GROUND - 1.0, GROUND),
    };
    let entity = app
        .world_mut()
        .spawn((
            faction,
            StandsOn(standing),
            Body::new(hex_core::TraversalProfile::WALKER),
            Initiative(initiative),
            UnitId(id),
            Transform::from_translation(standing.world_position()),
        ))
        .id();
    app.world_mut()
        .resource_mut::<UnitRegistry>()
        .register(UnitId(id), entity);
    entity
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

fn push(app: &mut App, command: GameCommand) {
    app.world_mut()
        .resource_mut::<CommandQueue>()
        .push(IssuedCommand {
            seat: PlayerSeat(0),
            command,
        });
}

/// Runs frames until the queue is drained and nothing is mid-presentation.
///
/// Bounded so a regression stalls the test with a message instead of hanging
/// the suite.
fn settle(app: &mut App) {
    for _ in 0..300 {
        let busy = {
            let mut busy = app.world_mut().query_filtered::<Entity, With<Busy>>();
            busy.iter(app.world()).next().is_some()
        };
        if !busy && app.world().resource::<CommandQueue>().is_empty() {
            app.update();
            return;
        }
        app.update();
    }
}

/// A surface path along adjacent coordinates at ground level.
fn path(coords: &[HexCoord]) -> Vec<TilePos> {
    coords
        .iter()
        .map(|coord| TilePos::new(*coord, GROUND_LEVEL))
        .collect()
}

fn standing_of(app: &mut App, entity: Entity) -> Option<TilePos> {
    app.world().get::<StandsOn>(entity).map(|s| s.0.pos)
}

fn budget_of(app: &App, entity: Entity) -> Option<u32> {
    app.world()
        .get::<Turn>(entity)
        .map(|turn| turn.movement_left)
}

/// Out of combat there is no turn and no budget: a valid move command simply
/// starts the walk. The funnel is the write path in both tempos.
#[test]
fn an_exploring_move_flows_through_the_funnel() {
    let mut app = test_app();
    let player = spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20, 1);
    enter_gameplay(&mut app);
    assert_eq!(mode(&app), Mode::Exploring, "precondition: nobody to fight");

    let destination = HexCoord::new_cubic(1, -1, 0);
    push(
        &mut app,
        GameCommand::MoveAlong {
            unit: UnitId(1),
            path: path(&[HexCoord::ORIGIN, destination]),
        },
    );
    settle(&mut app);

    assert_eq!(
        standing_of(&mut app, player),
        Some(TilePos::new(destination, GROUND_LEVEL)),
        "the commanded walk should land"
    );
}

/// The same command sequence from the same spawn state lands the same world.
///
/// This is the funnel's reason to exist: every sim mutation flows through the
/// drained queue, so the sequence *is* the input, and applying it twice must
/// be indistinguishable — same turn order, same positions, same budgets.
#[test]
fn a_replayed_sequence_lands_identically() {
    let script = [
        GameCommand::MoveAlong {
            unit: UnitId(1),
            path: path(&[HexCoord::ORIGIN, HexCoord::new_cubic(1, -1, 0)]),
        },
        GameCommand::EndTurn { unit: UnitId(1) },
        GameCommand::EndTurn { unit: UnitId(2) },
        GameCommand::MoveAlong {
            unit: UnitId(1),
            path: path(&[HexCoord::new_cubic(1, -1, 0), HexCoord::new_cubic(1, 0, -1)]),
        },
    ];

    let run = || {
        let mut app = test_app();
        let player = spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20, 1);
        let hostile = spawn_unit(
            &mut app,
            Faction::Hostile,
            HexCoord::new_cubic(2, -2, 0),
            10,
            2,
        );
        enter_gameplay(&mut app);
        assert_eq!(mode(&app), Mode::Combat, "precondition: fighting");

        for command in script.clone() {
            push(&mut app, command);
            settle(&mut app);
        }

        let order = app.world().resource::<TurnOrder>();
        (
            order.order().to_vec(),
            order.current(),
            order.round,
            standing_of(&mut app, player),
            standing_of(&mut app, hostile),
            budget_of(&app, player),
        )
    };

    assert_eq!(run(), run(), "a replay must not diverge");
}

/// A command from a unit that is not acting is refused, not deferred.
#[test]
fn an_end_turn_from_the_wrong_unit_is_dropped() {
    let mut app = test_app();
    let player = spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20, 1);
    spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(2, -2, 0),
        10,
        2,
    );
    enter_gameplay(&mut app);
    assert_eq!(mode(&app), Mode::Combat, "precondition: fighting");
    assert_eq!(
        app.world().resource::<TurnOrder>().current(),
        Some(UnitId(1)),
        "precondition: the player acts first"
    );

    push(&mut app, GameCommand::EndTurn { unit: UnitId(2) });
    app.update();
    app.update();

    assert_eq!(
        app.world().resource::<TurnOrder>().current(),
        Some(UnitId(1)),
        "somebody else's end-turn must not pass the player's turn"
    );
    assert!(
        app.world().get::<Turn>(player).is_some(),
        "the player should still hold the turn marker"
    );
}

/// A path that teleports is refused whole; a command applies entirely or not
/// at all.
#[test]
fn an_unwalkable_path_is_dropped() {
    let mut app = test_app();
    let player = spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20, 1);
    spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(2, -2, 0),
        10,
        2,
    );
    enter_gameplay(&mut app);
    assert_eq!(mode(&app), Mode::Combat, "precondition: fighting");

    // Origin to three hexes out in one "step".
    push(
        &mut app,
        GameCommand::MoveAlong {
            unit: UnitId(1),
            path: path(&[HexCoord::ORIGIN, HexCoord::new_cubic(3, -3, 0)]),
        },
    );
    app.update();
    app.update();

    assert_eq!(
        standing_of(&mut app, player),
        Some(TilePos::new(HexCoord::ORIGIN, GROUND_LEVEL)),
        "an unwalkable path must not move the piece"
    );
    assert_eq!(
        budget_of(&app, player),
        Some(4),
        "a refused path must not be billed"
    );
}

/// A path longer than the remaining budget is refused before anything moves.
#[test]
fn an_over_budget_path_is_dropped() {
    let mut app = test_app();
    let player = spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20, 1);
    spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(2, -2, 0),
        10,
        2,
    );
    enter_gameplay(&mut app);
    assert_eq!(mode(&app), Mode::Combat, "precondition: fighting");

    // Five adjacent steps against a budget of four.
    push(
        &mut app,
        GameCommand::MoveAlong {
            unit: UnitId(1),
            path: path(&[
                HexCoord::ORIGIN,
                HexCoord::new_cubic(0, 1, -1),
                HexCoord::new_cubic(0, 2, -2),
                HexCoord::new_cubic(0, 3, -3),
                HexCoord::new_cubic(0, 4, -4),
                HexCoord::new_cubic(0, 5, -5),
            ]),
        },
    );
    app.update();
    app.update();

    assert_eq!(
        standing_of(&mut app, player),
        Some(TilePos::new(HexCoord::ORIGIN, GROUND_LEVEL)),
        "an over-budget path must not move the piece"
    );
    assert_eq!(
        budget_of(&app, player),
        Some(4),
        "a refused path must not be billed"
    );
}

/// The future verbs parse, queue, and die in validation — never silently.
#[test]
fn an_unbuilt_verb_is_dropped_and_changes_nothing() {
    let mut app = test_app();
    let player = spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20, 1);
    spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(2, -2, 0),
        10,
        2,
    );
    enter_gameplay(&mut app);
    assert_eq!(mode(&app), Mode::Combat, "precondition: fighting");

    push(&mut app, GameCommand::Cast { unit: UnitId(1) });
    app.update();
    app.update();

    assert!(
        app.world().resource::<CommandQueue>().is_empty(),
        "the unbuilt verb should have been drained"
    );
    assert_eq!(
        budget_of(&app, player),
        Some(4),
        "an unbuilt verb must change nothing"
    );
    assert_eq!(
        app.world().resource::<TurnOrder>().current(),
        Some(UnitId(1)),
        "an unbuilt verb must not consume the turn"
    );
}

/// One command per unit per presentation: the second move in a single drain is
/// refused and, above all, never billed.
///
/// This is the applier-side half of the old double-charge bug. The click
/// emitter also suppresses mid-walk clicks, but the budget lives here, so the
/// authoritative guard has to hold even for emitters that forget.
#[test]
fn a_busy_unit_cannot_start_a_second_move() {
    let mut app = test_app();
    let player = spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20, 1);
    spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(2, -2, 0),
        10,
        2,
    );
    enter_gameplay(&mut app);
    assert_eq!(mode(&app), Mode::Combat, "precondition: fighting");

    let first = HexCoord::new_cubic(1, -1, 0);
    push(
        &mut app,
        GameCommand::MoveAlong {
            unit: UnitId(1),
            path: path(&[HexCoord::ORIGIN, first]),
        },
    );
    push(
        &mut app,
        GameCommand::MoveAlong {
            unit: UnitId(1),
            path: path(&[HexCoord::ORIGIN, HexCoord::new_cubic(0, 1, -1)]),
        },
    );
    settle(&mut app);

    assert_eq!(
        standing_of(&mut app, player),
        Some(TilePos::new(first, GROUND_LEVEL)),
        "only the first move should have been committed"
    );
    assert_eq!(
        budget_of(&app, player),
        Some(3),
        "exactly one step should have been billed"
    );
}
