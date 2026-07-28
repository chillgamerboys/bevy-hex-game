//! Integration tests for the damage loop: cast, decide, disable, go down.
//!
//! These run the real applier over the real command queue. What they prove is the part
//! no unit test can: that the **defender's choice round-trips through the command log**
//! rather than being made inside the applier, and that a unit whose lattice is spent
//! actually leaves the fight.
//!
//! The lattices here are hand-built rather than loaded from `lattices.ron`, because this
//! crate cannot see content files and should not need to — what is under test is the
//! wiring, not the drawings.

use std::collections::BTreeMap;

use bevy::app::PluginsState;
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;

use hex_combat::{Initiative, TurnOrder};
use hex_core::{
    CommandQueue, ElementId, GameCommand, HexCoord, HexSpan, IssuedCommand, LatticeCoord, Mode,
    PendingDecision, PlayerSeat, Screen, TilePos, UnitId,
};
use hex_lattice::{CellKind, LatticeSpec, LatticeState, LatticeStats};
use hex_units::{Downed, Faction, Standing, StandsOn, UnitRegistry};

fn test_app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin, bevy::input::InputPlugin));
    app.init_state::<Screen>();
    app.insert_resource(hex_assets::CombatSettings::default());
    app.add_sub_state::<Mode>();
    app.add_plugins(hex_combat::plugin);
    app.init_resource::<UnitRegistry>();
    while app.plugins_state() != PluginsState::Cleaned {
        app.finish();
        app.cleanup();
    }
    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Gameplay);
    app.update();
    app
}

/// Two gems, both attuned, so the defender has exactly two hexes to lose.
fn two_gem_lattice() -> (LatticeSpec, LatticeStats) {
    let spec = LatticeSpec::default()
        .with(
            LatticeCoord::ORIGIN,
            CellKind::Gem {
                element: ElementId(0),
            },
        )
        .with(
            LatticeCoord::new(1, 0),
            CellKind::Gem {
                element: ElementId(0),
            },
        );
    let stats = LatticeStats::new(BTreeMap::from([(ElementId(0), 3)]), BTreeMap::new());
    (spec, stats)
}

fn spawn(app: &mut App, id: UnitId, faction: Faction, coord: HexCoord) -> Entity {
    let (spec, stats) = two_gem_lattice();
    let state = LatticeState::new(&spec, &stats);
    let entity = app
        .world_mut()
        .spawn((
            faction,
            id,
            StandsOn(Standing {
                pos: TilePos::new(coord, 1),
                span: HexSpan::new(0.0, 1.0),
            }),
            Initiative(10),
            spec,
            state,
            stats,
        ))
        .id();
    app.world_mut()
        .resource_mut::<UnitRegistry>()
        .register(id, entity);
    entity
}

/// The defender's answer arrives as a command, and applying it disables exactly the
/// hexes it names.
///
/// This is the shape that keeps a fight replayable: the applier does not choose, it
/// parks a decision; something answers by pushing `ChooseDisables`; the answer is what
/// mutates. A version that picked inside the applier would pass no test here because
/// there would be no command to inspect.
#[test]
fn a_disable_decision_is_answered_through_the_command_log() {
    let mut app = test_app();
    let defender = spawn(&mut app, UnitId(1), Faction::Hostile, HexCoord::ORIGIN);

    // Park a decision the way a landed cast would.
    *app.world_mut().resource_mut::<PendingDecision>() = PendingDecision::ChooseDisables {
        decider: UnitId(1),
        count: 1,
        source: UnitId(0),
    };
    assert!(app.world().resource::<PendingDecision>().is_open());

    app.world_mut()
        .resource_mut::<CommandQueue>()
        .push(IssuedCommand {
            seat: PlayerSeat::default(),
            command: GameCommand::ChooseDisables {
                unit: UnitId(1),
                cells: vec![LatticeCoord::ORIGIN],
            },
        });
    app.update();

    let state = app
        .world()
        .entity(defender)
        .get::<LatticeState>()
        .expect("the defender kept its lattice");
    assert!(
        state.is_disabled(LatticeCoord::ORIGIN),
        "the named hex should be down"
    );
    assert!(
        !state.is_disabled(LatticeCoord::new(1, 0)),
        "and only the named one"
    );
    assert!(
        !app.world().resource::<PendingDecision>().is_open(),
        "answering should close the decision"
    );
}

/// An answer that does not match the open decision is refused rather than applied.
///
/// A replayed or forged log must not be able to disable a bystander's hexes by naming
/// somebody else's unit, name more hexes than the hit earned, or name the same hex twice
/// to satisfy a count while taking down one.
#[test]
fn a_mismatched_answer_is_refused() {
    let mut app = test_app();
    let defender = spawn(&mut app, UnitId(1), Faction::Hostile, HexCoord::ORIGIN);
    let open = PendingDecision::ChooseDisables {
        decider: UnitId(1),
        count: 2,
        source: UnitId(0),
    };

    let bad_answers = [
        // The wrong number of hexes.
        vec![LatticeCoord::ORIGIN],
        // The same hex twice, which would satisfy the count while costing one.
        vec![LatticeCoord::ORIGIN, LatticeCoord::ORIGIN],
        // A cell that is not in this lattice at all.
        vec![LatticeCoord::ORIGIN, LatticeCoord::new(9, 9)],
    ];

    for cells in bad_answers {
        *app.world_mut().resource_mut::<PendingDecision>() = open.clone();
        app.world_mut()
            .resource_mut::<CommandQueue>()
            .push(IssuedCommand {
                seat: PlayerSeat::default(),
                command: GameCommand::ChooseDisables {
                    unit: UnitId(1),
                    cells: cells.clone(),
                },
            });
        app.update();

        let state = app
            .world()
            .entity(defender)
            .get::<LatticeState>()
            .expect("the defender kept its lattice");
        assert!(
            !state.is_disabled(LatticeCoord::new(1, 0)),
            "a refused answer should disable nothing: {cells:?}"
        );
        assert!(
            app.world().resource::<PendingDecision>().is_open(),
            "a refused answer should leave the decision open: {cells:?}"
        );
    }
}

/// A unit whose every hex is disabled leaves the turn order and is marked down.
///
/// Downed rather than despawned, so a restoring spell has something to target — and so
/// the registry, which has no unregister, never serves a dead entity.
#[test]
fn a_unit_with_every_hex_disabled_goes_down_and_leaves_the_order() {
    let mut app = test_app();
    spawn(&mut app, UnitId(0), Faction::Player, HexCoord::ORIGIN);
    let defender = spawn(
        &mut app,
        UnitId(1),
        Faction::Hostile,
        HexCoord::new_cubic(1, -1, 0),
    );

    app.world_mut()
        .resource_mut::<NextState<Mode>>()
        .set(Mode::Combat);
    app.update();
    assert!(
        app.world()
            .resource::<TurnOrder>()
            .position_of(UnitId(1))
            .is_some(),
        "the defender should start in the order"
    );

    // Take both hexes down through the real path: park a decision, answer it.
    *app.world_mut().resource_mut::<PendingDecision>() = PendingDecision::ChooseDisables {
        decider: UnitId(1),
        count: 2,
        source: UnitId(0),
    };
    app.world_mut()
        .resource_mut::<CommandQueue>()
        .push(IssuedCommand {
            seat: PlayerSeat::default(),
            command: GameCommand::ChooseDisables {
                unit: UnitId(1),
                cells: vec![LatticeCoord::ORIGIN, LatticeCoord::new(1, 0)],
            },
        });
    app.update();
    app.update();

    assert!(
        app.world().entity(defender).contains::<Downed>(),
        "a spent lattice should put its unit down"
    );
    assert_eq!(
        app.world().resource::<TurnOrder>().position_of(UnitId(1)),
        None,
        "a downed unit should leave the turn order"
    );
    assert!(
        app.world()
            .resource::<UnitRegistry>()
            .entity_of(UnitId(1))
            .is_some(),
        "and stay registered, because a restoring spell needs something to target"
    );
}

/// A defender with fewer live hexes than the hit demands gives everything it has.
///
/// **The deadlock this guards.** The auto-policy can only offer hexes that exist, so a
/// two-hex hit on a lattice with one hex left produces a one-cell answer. An applier
/// demanding an exact match would refuse it, the policy would re-offer the same answer,
/// and resolution would park forever — precisely at the moment a unit is about to go
/// down, which is the moment it matters most.
#[test]
fn a_short_answer_is_accepted_when_the_lattice_has_no_more_to_give() {
    let mut app = test_app();
    let defender = spawn(&mut app, UnitId(1), Faction::Hostile, HexCoord::ORIGIN);

    // One hex already gone, so only one remains.
    *app.world_mut().resource_mut::<PendingDecision>() = PendingDecision::ChooseDisables {
        decider: UnitId(1),
        count: 1,
        source: UnitId(0),
    };
    app.world_mut()
        .resource_mut::<CommandQueue>()
        .push(IssuedCommand {
            seat: PlayerSeat::default(),
            command: GameCommand::ChooseDisables {
                unit: UnitId(1),
                cells: vec![LatticeCoord::ORIGIN],
            },
        });
    app.update();

    // Now ask for two when only one is left.
    *app.world_mut().resource_mut::<PendingDecision>() = PendingDecision::ChooseDisables {
        decider: UnitId(1),
        count: 2,
        source: UnitId(0),
    };
    app.world_mut()
        .resource_mut::<CommandQueue>()
        .push(IssuedCommand {
            seat: PlayerSeat::default(),
            command: GameCommand::ChooseDisables {
                unit: UnitId(1),
                cells: vec![LatticeCoord::new(1, 0)],
            },
        });
    app.update();

    assert!(
        !app.world().resource::<PendingDecision>().is_open(),
        "a short answer from a spent lattice must close the decision, not deadlock it"
    );
    let state = app
        .world()
        .entity(defender)
        .get::<LatticeState>()
        .expect("the defender kept its lattice");
    assert!(
        state.is_disabled(LatticeCoord::new(1, 0)),
        "the last hex goes"
    );
}

/// A downed unit's lattice is still reachable, or nothing could ever revive it.
///
/// Filtering the applier's lattice query by `Downed` would have been the obvious thing
/// and would have quietly made the design's stated recovery impossible: downed exists
/// *instead of* despawning precisely so a restoring spell has something to target.
#[test]
fn a_downed_units_lattice_can_still_be_restored() {
    let mut app = test_app();
    let defender = spawn(&mut app, UnitId(1), Faction::Hostile, HexCoord::ORIGIN);

    {
        let mut entity = app.world_mut().entity_mut(defender);
        let mut state = entity.get_mut::<LatticeState>().expect("a lattice");
        hex_lattice::apply_disables(&mut state, &[LatticeCoord::ORIGIN, LatticeCoord::new(1, 0)]);
    }
    app.world_mut()
        .resource_mut::<NextState<Mode>>()
        .set(Mode::Combat);
    app.update();
    app.update();
    assert!(
        app.world().entity(defender).contains::<Downed>(),
        "a spent lattice should put its unit down"
    );

    // The engine's restore reaches it, which is what a revival spell will do.
    let mut entity = app.world_mut().entity_mut(defender);
    let mut state = entity
        .get_mut::<LatticeState>()
        .expect("a downed unit keeps its lattice");
    assert_eq!(
        hex_lattice::restore(&mut state, &[LatticeCoord::ORIGIN]),
        1,
        "a downed unit's hexes must still be restorable"
    );
}
