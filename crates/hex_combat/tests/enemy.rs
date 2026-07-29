//! Integration tests for what an enemy does with its turn.
//!
//! The AI is a placeholder, but "closes the distance and swings" is still a claim
//! that can be wrong in specific ways — walking onto the player, overshooting its
//! movement budget, or never ending its turn and stalling the fight. Those are what
//! these check.
//!
//! Terrain is spawned by the test, because `hex_combat` cannot see `hex_map` and does
//! not need to: it consumes `TilePos`, `HexSpan`, `SubstanceId` and `Headroom`.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use bevy::app::PluginsState;
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;

use hex_anim::Transformation;
use hex_assets::{
    ArtPalette, PaletteSwatch, PlayerSettings, SrgbColor, Substance, SubstanceFile, SubstanceTable,
    SwatchId,
};
use hex_combat::{Initiative, TurnOrder};
use hex_core::{
    CommandQueue, ControlOwner, GameCommand, Headroom, HexCoord, HexSpan, HexTile, IssuedCommand,
    LatticeCoord, Mode, PendingDecision, PlayerSeat, Screen, SubstanceId, TilePos, Turn, UnitId,
    MAX_HEADROOM,
};
use hex_lattice::{CellKind, LatticeSpec, LatticeState, LatticeStats};
use hex_units::{Body, Faction, HexPathingLine, MovingTo, Standing, StandsOn};

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
    });
    app.add_systems(OnEnter(Screen::Gameplay), spawn_terrain);
    // `hex_units::movement::plugin`, not the whole of `hex_units::plugin`: this is what
    // keeps `StandsOn` honest as a unit walks, and combat is meaningless without it.
    // The full plugin would also read the active scenario placements and spawn its own
    // pieces on top of the ones these tests place by hand.
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

#[expect(
    clippy::expect_used,
    reason = "invalid compile-time fixture data should fail the test immediately"
)]
fn substance_table() -> SubstanceTable {
    let stone_id = SwatchId::new("terrain/stone").expect("the fixture swatch id should be valid");
    let stone = PaletteSwatch::new(
        "Stone",
        SrgbColor::new(0.5, 0.5, 0.5).expect("the fixture color should be valid"),
        BTreeSet::from(["test".to_owned()]),
    )
    .expect("the fixture swatch should be valid");
    let palette = ArtPalette::new(BTreeMap::from([(stone_id.clone(), stone)]))
        .expect("the fixture palette should be valid");
    let mut substances = bevy::platform::collections::HashMap::default();
    substances.insert("air".to_owned(), Substance::invisible(false, false));
    substances.insert(
        "stone".to_owned(),
        Substance::from_swatch(stone_id, true, true),
    );
    SubstanceTable::from_file(&SubstanceFile { substances }, &palette)
        .expect("the fixture substance should resolve through its palette")
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

fn spawn_unit(app: &mut App, faction: Faction, coord: HexCoord, initiative: u32) -> Entity {
    let standing = Standing {
        pos: TilePos::new(coord, GROUND_LEVEL),
        span: HexSpan::new(GROUND - 1.0, GROUND),
    };
    let mut unit = app.world_mut().spawn((
        faction,
        StandsOn(standing),
        Body::new(hex_core::TraversalProfile::WALKER),
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

/// Stands in for the walk animation finishing.
///
/// Removing the component is exactly what `hex_anim`'s driver does once a transformer
/// reports itself done, and movement reconciliation commits the final route step.
/// `StandsOn` advances only across whole completed legs, so any test that reads the
/// final position after ordering a move has to let the move land first.
fn finish_moving(app: &mut App, entity: Entity) {
    app.world_mut()
        .entity_mut(entity)
        .remove::<Transformation>();
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
    finish_moving(&mut app, enemy);

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
    finish_moving(&mut app, enemy);

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
    finish_moving(&mut app, enemy);
    let after = coord_of(&app, enemy).expect("the enemy exists");

    let travelled = before.distance(after);
    assert!(
        travelled <= 4,
        "the enemy moved {travelled} hexes on a four-hex budget"
    );
    assert!(travelled > 0, "the enemy should have moved at all");
}

/// Equal tactical choices resolve by the stable unit id rather than query
/// iteration order or entity bits — neither of which survives a save or a
/// second run.
#[test]
fn equally_routable_foes_have_a_deterministic_tie_break() {
    let mut app = test_app();
    let enemy = spawn_unit(&mut app, Faction::Hostile, HexCoord::ORIGIN, 30);
    let first = HexCoord::new_cubic(3, -3, 0);
    let second = HexCoord::new_cubic(0, 3, -3);
    let first_entity = spawn_unit(&mut app, Faction::Player, first, 20);
    let second_entity = spawn_unit(&mut app, Faction::Player, second, 10);
    // Explicit ids, with the LOWER one on the later spawn: the winner must
    // follow the stable id, not spawn order and not entity allocation.
    app.world_mut().entity_mut(first_entity).insert(UnitId(2));
    app.world_mut().entity_mut(second_entity).insert(UnitId(1));
    let (expected, other) = (second, first);
    enter_gameplay(&mut app);

    let destination = app
        .world()
        .get::<MovingTo>(enemy)
        .and_then(|moving| moving.path.last())
        .expect("the enemy should commit an approach")
        .pos
        .coord;
    assert_eq!(
        destination.distance(expected),
        1,
        "an exact tie did not choose the lower unit id"
    );
    assert!(
        destination.distance(other) > 1,
        "the route headed toward the other equally distant foe"
    );
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
        Some(unit_id(&app, enemy)),
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
        Some(unit_id(&app, player)),
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
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        Duration::from_millis(100),
    ));
    let player = spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20);
    let adjacent = HexCoord::new_cubic(1, -1, 0);
    let enemy = spawn_unit(&mut app, Faction::Hostile, adjacent, 10);
    enter_gameplay(&mut app);

    end_turn(&mut app);

    assert!(
        app.world().get::<Transformation>(enemy).is_some(),
        "an adjacent enemy should commit an attack animation"
    );
    assert_eq!(
        app.world().resource::<TurnOrder>().current(),
        Some(unit_id(&app, enemy)),
        "the attacker should keep its turn while the lunge is running"
    );

    for _ in 0..10 {
        app.update();
        let attack_finished = app.world().get::<Transformation>(enemy).is_none();
        let turn_advanced =
            app.world().resource::<TurnOrder>().current() == Some(unit_id(&app, player));
        if attack_finished && turn_advanced {
            break;
        }
    }

    assert!(
        app.world().get::<Transformation>(enemy).is_none(),
        "the deterministic clock did not finish the enemy's attack"
    );
    assert!(
        app.world().get::<Transformation>(player).is_none(),
        "the target's recoil outlived the completed attack"
    );
    assert_eq!(
        app.world().resource::<TurnOrder>().current(),
        Some(unit_id(&app, player)),
        "the turn should advance after the attack animation completes"
    );
    assert!(
        app.world().get::<Turn>(player).is_some(),
        "the player should receive the next turn"
    );
    assert_eq!(
        coord_of(&app, enemy),
        Some(adjacent),
        "an attack should not change which surface the enemy stands on"
    );
}

/// A hostile strike can park resolution on a human defender choice. The AI must not
/// prequeue its end-turn beside that strike — the modal gate correctly refuses every
/// command except the matching answer — but it must still end the spent turn once the
/// answer and presentation have both finished.
#[test]
fn a_player_defence_choice_does_not_strand_the_enemy_turn() {
    let mut app = test_app();
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        Duration::from_millis(100),
    ));
    let player = spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20);
    let enemy = spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(1, -1, 0),
        10,
    );
    let spec = LatticeSpec::default()
        .with(LatticeCoord::ORIGIN, CellKind::Blank)
        .with(LatticeCoord::new(1, 0), CellKind::Blank);
    let stats = LatticeStats::default();
    let state = LatticeState::new(&spec, &stats);
    app.world_mut()
        .entity_mut(player)
        .insert((ControlOwner::default(), spec, state, stats));
    app.world_mut()
        .entity_mut(enemy)
        .insert(ControlOwner::default());
    enter_gameplay(&mut app);

    let player_id = unit_id(&app, player);
    let enemy_id = unit_id(&app, enemy);
    end_turn(&mut app);

    assert_eq!(
        *app.world().resource::<PendingDecision>(),
        PendingDecision::ChooseDisables {
            decider: player_id,
            count: 1,
            source: enemy_id,
        },
        "the adjacent strike should wait for the player to name its disabled cell"
    );
    assert_eq!(
        app.world().resource::<TurnOrder>().current(),
        Some(enemy_id),
        "the attacker owns the turn while its damage is unresolved"
    );

    app.world_mut()
        .resource_mut::<CommandQueue>()
        .push(IssuedCommand {
            seat: PlayerSeat::default(),
            command: GameCommand::ChooseDisables {
                unit: player_id,
                cells: vec![LatticeCoord::ORIGIN],
            },
        });
    app.update();

    for _ in 0..20 {
        app.update();
        if app.world().resource::<TurnOrder>().current() == Some(player_id) {
            break;
        }
    }

    assert!(
        !app.world().resource::<PendingDecision>().is_open(),
        "the matching player answer should resolve the parked damage"
    );
    assert_eq!(
        app.world().resource::<TurnOrder>().current(),
        Some(player_id),
        "the AI should end its already-spent turn after the decision and lunge resolve"
    );
    assert!(
        app.world().get::<Turn>(player).is_some(),
        "the player should receive the next turn"
    );
}

/// Advances the simulation without coupling enemy-policy tests to an input binding.
#[expect(
    clippy::expect_used,
    reason = "an active combatant is a precondition for this integration-test helper"
)]
fn end_turn(app: &mut App) {
    let unit = app
        .world()
        .resource::<TurnOrder>()
        .current()
        .expect("combat should have a current unit");
    app.world_mut()
        .resource_mut::<CommandQueue>()
        .push(IssuedCommand {
            seat: PlayerSeat::default(),
            command: GameCommand::EndTurn { unit },
        });
    // The command advances to the hostile during Apply/Advance. Its policy runs
    // in Act on the following frame, preserving the production schedule boundary.
    app.update();
    app.update();
}

/// The turn cannot pass while the unit holding it is still walking.
///
/// Acting is half immediate and half deferred: `spend` mutates `Turn` in place, but the
/// walk animation is inserted through `Commands`. Until `CombatSystems` existed,
/// `take_enemy_turn` and `advance_turn` were unordered — so `advance_turn` could see a
/// turn already marked finished with no `Transformation` yet attached to say the unit
/// was moving, and hand the turn on before the enemy had taken a step.
///
/// The observable consequence is this: while a `Transformation` is running, the order
/// must still be pointing at its owner. Anything else means somebody else can act
/// while the enemy is mid-stride.
#[test]
fn the_turn_does_not_pass_while_its_unit_is_still_walking() {
    let mut app = test_app();
    spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20);
    let enemy = spawn_unit(
        &mut app,
        Faction::Hostile,
        HexCoord::new_cubic(4, -4, 0),
        10,
    );
    enter_gameplay(&mut app);

    end_turn(&mut app);
    app.update();

    assert!(
        app.world().get::<Transformation>(enemy).is_some(),
        "precondition: the enemy should be walking by now"
    );
    assert_eq!(
        app.world().resource::<TurnOrder>().current(),
        Some(unit_id(&app, enemy)),
        "the turn was handed on while the enemy was still mid-walk"
    );
}

/// A fight starts when a unit **arrives**, not when it sets off.
///
/// The whole point of splitting `StandsOn` from `MovingTo`. Engagement asks where units
/// are; while the two were one component that answer was the destination, so committing
/// to a walk started the fight instantly at the far end of the route — and a walk whose
/// endpoint happened to be out of range could pass straight through engaging distance
/// without anything noticing.
///
/// Covered here rather than in `hex_units` because `engagement` lives in this crate and
/// the claim is about the two halves agreeing.
#[test]
fn a_fight_starts_on_arrival_not_on_departure() {
    let mut app = test_app();
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        Duration::from_millis(100),
    ));
    spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20);
    let start = HexCoord::new_cubic(5, -5, 0);
    let destination = HexCoord::new_cubic(4, -4, 0);
    let enemy = spawn_unit(&mut app, Faction::Hostile, start, 10);
    enter_gameplay(&mut app);

    assert_eq!(
        *app.world().resource::<State<Mode>>().get(),
        Mode::Exploring,
        "precondition: five hexes apart is no fight"
    );

    // One real leg crosses exactly from outside engagement range to its boundary.
    // Making its speed equal to its world-space length gives the leg a one-second
    // duration under the deterministic 100 ms test clock.
    let path: Vec<Standing> = start
        .line_between(destination)
        .into_iter()
        .map(|coord| Standing {
            pos: TilePos::new(coord, GROUND_LEVEL),
            span: HexSpan::new(GROUND - 1.0, GROUND),
        })
        .collect();
    let [from, to] = path.as_slice() else {
        panic!("an adjacent coordinate pair should produce a two-step route");
    };
    let speed = from.world_position().distance(to.world_position());
    app.world_mut().entity_mut(enemy).insert((
        MovingTo::new(path.clone(), speed),
        Transformation::from(HexPathingLine::new(&path, speed)),
    ));

    // Several active ticks, but less than the leg's one-second duration.
    for _ in 0..5 {
        app.update();
    }
    assert_eq!(
        coord_of(&app, enemy),
        Some(start),
        "logical position advanced before the route reached its first waypoint"
    );
    assert!(
        app.world().get::<Transformation>(enemy).is_some(),
        "the route finished before the arrival assertion"
    );
    assert!(matches!(
        app.world().resource::<NextState<Mode>>(),
        &NextState::Unchanged
    ));

    for _ in 0..10 {
        app.update();
        if coord_of(&app, enemy) == Some(destination) {
            break;
        }
    }

    assert_eq!(
        coord_of(&app, enemy),
        Some(destination),
        "the real route never arrived at the engagement boundary"
    );
    assert_eq!(
        *app.world().resource::<State<Mode>>().get(),
        Mode::Exploring,
        "the queued combat transition should apply on the next frame"
    );
    assert!(
        matches!(
            app.world().resource::<NextState<Mode>>(),
            &NextState::Pending(Mode::Combat)
        ),
        "arrival at engagement range did not queue combat"
    );

    app.update();

    assert_eq!(
        *app.world().resource::<State<Mode>>().get(),
        Mode::Combat,
        "arriving next to the player should start the fight"
    );
}

/// Intermediate whole steps are positions too, even when one frame crosses several.
///
/// Both endpoints are outside engage range, so an implementation that updates
/// `StandsOn` only at final arrival misses the fight completely even though the route
/// passes directly through the player's tile. The deliberately high speed makes the
/// first active tick jump from distance eight to distance six on the other side, so
/// sampling only one `StandsOn` per frame also misses every in-range waypoint.
#[test]
fn crossing_engage_range_mid_route_starts_fight_and_stops_walk() {
    let mut app = test_app();
    spawn_unit(&mut app, Faction::Player, HexCoord::ORIGIN, 20);
    let start = HexCoord::new_cubic(-8, 8, 0);
    let end = HexCoord::new_cubic(8, -8, 0);
    let enemy = spawn_unit(&mut app, Faction::Hostile, start, 10);
    enter_gameplay(&mut app);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        Duration::from_millis(250),
    ));

    let path: Vec<Standing> = start
        .line_between(end)
        .into_iter()
        .map(|coord| Standing {
            pos: TilePos::new(coord, GROUND_LEVEL),
            span: HexSpan::new(GROUND - 1.0, GROUND),
        })
        .collect();
    let speed = 100.0;
    app.world_mut().entity_mut(enemy).insert((
        MovingTo::new(path.clone(), speed),
        Transformation::from(HexPathingLine::new(&path, speed)),
    ));

    for _ in 0..100 {
        app.update();
        if *app.world().resource::<State<Mode>>().get() == Mode::Combat {
            break;
        }
    }

    assert_eq!(
        *app.world().resource::<State<Mode>>().get(),
        Mode::Combat,
        "the route crossed engage range without starting a fight"
    );
    assert!(
        app.world().get::<Transformation>(enemy).is_none(),
        "the walk continued after combat began"
    );
    assert!(
        app.world().get::<MovingTo>(enemy).is_none(),
        "the interrupted unit kept its obsolete route"
    );

    let stopped = app
        .world()
        .get::<StandsOn>(enemy)
        .expect("the enemy should still have a logical position")
        .0
        .pos;
    assert!(path.iter().any(|step| step.pos == stopped));
    assert!(
        stopped.coord.distance(HexCoord::ORIGIN) <= 4,
        "combat began before the route reached engagement distance"
    );
    assert_ne!(
        stopped.coord, end,
        "the interrupted walk was delivered to its out-of-range endpoint"
    );
}
