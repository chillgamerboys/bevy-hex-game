//! Contract tests for the damage loop: cast, decide, disable, go down.
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
use std::time::Duration;

use bevy::prelude::*;

use hex_combat::{
    CombatEvent, CommandRefusal, FactionLatticeKnowledge, Initiative, KnownCell,
    RestorationRefusal, TurnOrder,
};
use hex_core::{
    CommandQueue, ControlOwner, ElementId, GameCommand, Headroom, HexCoord, HexSpan, IssuedCommand,
    KnowledgeExpiry, KnowledgeSource, LatticeCoord, LightDomain, Mode, PendingDecision, PlayerSeat,
    Screen, SubstanceId, TilePos, TraversalProfile, UnitId,
};
use hex_lattice::{apply_disables, CellKind, LatticeSpec, LatticeState, LatticeStats};
use hex_perception::{
    apply_observations, FactionMapKnowledge, FactionObservation, FactionObservations, ObservedUnit,
    SurfaceSnapshot, SurfaceSnapshots,
};
use hex_test_support::{SyntheticArena, TestAppBuilder};
use hex_units::{Body, Downed, Faction, Party, Player, Standing, StandsOn, UnitRegistry};

#[expect(
    clippy::expect_used,
    reason = "invalid shared deterministic fixture data must fail during construction"
)]
fn test_app() -> App {
    let mut builder = TestAppBuilder::new()
        .with_fixed_step(Duration::ZERO)
        .with_arena(SyntheticArena::flat_radius(10, 1))
        .expect("the shared synthetic arena must be valid");
    let app = builder.app_mut();
    app.insert_resource(hex_assets::CombatSettings::default());
    app.add_plugins((
        hex_units::authored_object_occupancy::plugin,
        hex_combat::plugin,
    ));
    app.init_resource::<UnitRegistry>();
    let mut app = builder.build();
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
            Body::new(TraversalProfile::WALKER),
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

#[expect(
    clippy::expect_used,
    reason = "duplicate test identities or surfaces invalidate the fixture"
)]
fn publish_spatial_knowledge(app: &mut App) {
    let rows: Vec<(UnitId, Faction, TilePos, HexSpan)> = {
        let world = app.world_mut();
        let mut query = world.query::<(&UnitId, &Faction, &StandsOn)>();
        query
            .iter(world)
            .map(|(id, faction, standing)| (*id, *faction, standing.0.pos, standing.0.span))
            .collect()
    };
    let current =
        SurfaceSnapshots::try_from_iter(rows.iter().map(|&(_, _, pos, span)| SurfaceSnapshot {
            pos,
            span,
            substance: SubstanceId(0),
            headroom: Headroom(2),
            is_solid: true,
            blocked: false,
            domain: LightDomain::Exterior,
        }))
        .expect("test units occupy unique surfaces");
    let observe_all = || {
        let mut observation = FactionObservation::new();
        for &(id, faction, pos, _) in &rows {
            observation.insert_surface(pos);
            observation
                .try_insert_unit(ObservedUnit {
                    id,
                    faction,
                    pos,
                    provides_sight: true,
                })
                .expect("test unit ids are unique");
        }
        observation
    };
    let observations = FactionObservations::from_factions(observe_all(), observe_all());
    let mut spatial = FactionMapKnowledge::new();
    apply_observations(&mut spatial, &current, &observations);
    app.insert_resource(spatial);
}

fn take_events(app: &mut App) -> Vec<CombatEvent> {
    app.world_mut()
        .resource_mut::<Messages<CombatEvent>>()
        .drain()
        .collect()
}

#[expect(
    clippy::expect_used,
    reason = "fixture facts must be accepted by the active combat authority"
)]
fn publish_adapter_facts(app: &mut App) {
    hex_combat::publish_combat_adapter_facts(app.world_mut())
        .expect("the fixture projection must be valid");
}

#[expect(
    clippy::expect_used,
    reason = "a missing lattice means the test fixture itself is invalid"
)]
fn disable(app: &mut App, entity: Entity, cells: &[LatticeCoord]) {
    let mut entity = app.world_mut().entity_mut(entity);
    let mut state = entity
        .get_mut::<LatticeState>()
        .expect("fixture unit has a lattice");
    apply_disables(&mut state, cells);
}

#[test]
fn restoration_validates_and_revives_through_the_command_funnel() {
    let mut app = test_app();
    let caster = spawn(&mut app, UnitId(0), Faction::Player, HexCoord::ORIGIN);
    app.world_mut()
        .entity_mut(caster)
        .insert((Player, ControlOwner::default()));
    let target = spawn(
        &mut app,
        UnitId(1),
        Faction::Player,
        HexCoord::new_cubic(1, -1, 0),
    );
    disable(
        &mut app,
        target,
        &[LatticeCoord::ORIGIN, LatticeCoord::new(1, 0)],
    );
    app.world_mut().entity_mut(target).insert(Downed);

    *app.world_mut().resource_mut::<PendingDecision>() = PendingDecision::ChooseRestores {
        decider: UnitId(0),
        target: UnitId(1),
        count: 1,
    };
    app.world_mut()
        .resource_mut::<CommandQueue>()
        .push(IssuedCommand {
            seat: PlayerSeat::default(),
            command: GameCommand::ChooseRestores {
                unit: UnitId(0),
                target: UnitId(1),
                cells: vec![LatticeCoord::ORIGIN],
            },
        });
    app.update();

    let state = app
        .world()
        .entity(target)
        .get::<LatticeState>()
        .expect("target keeps its lattice");
    assert!(!state.is_disabled(LatticeCoord::ORIGIN));
    assert!(state.is_disabled(LatticeCoord::new(1, 0)));
    assert!(!app.world().entity(target).contains::<Downed>());
    assert!(!app.world().resource::<PendingDecision>().is_open());
    assert_eq!(
        take_events(&mut app),
        vec![
            CombatEvent::HexesRestored {
                caster: UnitId(0),
                target: UnitId(1),
                cells: vec![LatticeCoord::ORIGIN],
            },
            CombatEvent::Revived {
                unit: UnitId(1),
                reenters_round: 1,
            },
        ]
    );
}

#[test]
fn restoration_rejects_the_wrong_target_quota_and_cells_without_mutating() {
    let mut app = test_app();
    spawn(&mut app, UnitId(0), Faction::Player, HexCoord::ORIGIN);
    let target = spawn(
        &mut app,
        UnitId(1),
        Faction::Player,
        HexCoord::new_cubic(1, -1, 0),
    );
    spawn(
        &mut app,
        UnitId(2),
        Faction::Player,
        HexCoord::new_cubic(2, -2, 0),
    );
    disable(&mut app, target, &[LatticeCoord::ORIGIN]);

    let cases = [
        (
            UnitId(2),
            vec![LatticeCoord::ORIGIN],
            CommandRefusal::Restoration {
                reason: RestorationRefusal::WrongTarget {
                    expected: UnitId(1),
                },
            },
        ),
        (
            UnitId(1),
            Vec::new(),
            CommandRefusal::Restoration {
                reason: RestorationRefusal::WrongCount {
                    expected: 1,
                    actual: 0,
                },
            },
        ),
        (
            UnitId(1),
            vec![LatticeCoord::new(1, 0)],
            CommandRefusal::Restoration {
                reason: RestorationRefusal::CellNotDisabled {
                    cell: LatticeCoord::new(1, 0),
                },
            },
        ),
    ];
    for (named_target, cells, refusal) in cases {
        *app.world_mut().resource_mut::<PendingDecision>() = PendingDecision::ChooseRestores {
            decider: UnitId(0),
            target: UnitId(1),
            count: 1,
        };
        let command = GameCommand::ChooseRestores {
            unit: UnitId(0),
            target: named_target,
            cells,
        };
        app.world_mut()
            .resource_mut::<CommandQueue>()
            .push(IssuedCommand {
                seat: PlayerSeat::default(),
                command: command.clone(),
            });
        app.update();
        assert_eq!(
            take_events(&mut app),
            vec![CombatEvent::CommandRefused { command, refusal }]
        );
        assert!(
            app.world()
                .entity(target)
                .get::<LatticeState>()
                .is_some_and(|state| state.is_disabled(LatticeCoord::ORIGIN)),
            "a refused restoration mutated the target"
        );
    }
}

#[test]
fn exploring_rest_recovers_only_the_party() {
    let mut app = test_app();
    let first = spawn(&mut app, UnitId(0), Faction::Player, HexCoord::ORIGIN);
    let second = spawn(
        &mut app,
        UnitId(1),
        Faction::Player,
        HexCoord::new_cubic(1, -1, 0),
    );
    let hostile = spawn(
        &mut app,
        UnitId(2),
        Faction::Hostile,
        HexCoord::new_cubic(3, -3, 0),
    );
    for entity in [first, second, hostile] {
        disable(&mut app, entity, &[LatticeCoord::ORIGIN]);
        app.world_mut().entity_mut(entity).insert(Downed);
    }
    app.world_mut()
        .entity_mut(second)
        .insert(ControlOwner(PlayerSeat(5)));
    app.world_mut().resource_mut::<Party>().members = vec![UnitId(0), UnitId(1)];

    app.world_mut()
        .resource_mut::<CommandQueue>()
        .push(IssuedCommand {
            seat: PlayerSeat(5),
            command: GameCommand::Rest { unit: UnitId(1) },
        });
    app.update();

    for entity in [first, second] {
        assert!(!app.world().entity(entity).contains::<Downed>());
        assert!(app
            .world()
            .entity(entity)
            .get::<LatticeState>()
            .is_some_and(|state| !state.is_disabled(LatticeCoord::ORIGIN)));
    }
    assert!(app.world().entity(hostile).contains::<Downed>());
    assert!(app
        .world()
        .entity(hostile)
        .get::<LatticeState>()
        .is_some_and(|state| state.is_disabled(LatticeCoord::ORIGIN)));
    assert_eq!(
        take_events(&mut app)
            .into_iter()
            .filter(|event| matches!(event, CombatEvent::Rested { .. }))
            .count(),
        2
    );
}

#[test]
fn a_revived_unit_rejoins_only_when_the_round_wraps() {
    let mut app = test_app();
    let caster = spawn(&mut app, UnitId(0), Faction::Player, HexCoord::ORIGIN);
    app.world_mut()
        .entity_mut(caster)
        .insert((Player, ControlOwner::default(), Initiative(20)));
    let revived = spawn(
        &mut app,
        UnitId(1),
        Faction::Player,
        HexCoord::new_cubic(1, -1, 0),
    );
    app.world_mut()
        .entity_mut(revived)
        .insert((Player, Downed, Initiative(15)));
    disable(
        &mut app,
        revived,
        &[LatticeCoord::ORIGIN, LatticeCoord::new(1, 0)],
    );
    spawn(
        &mut app,
        UnitId(2),
        Faction::Hostile,
        HexCoord::new_cubic(2, -2, 0),
    );
    *app.world_mut().resource_mut::<PendingDecision>() = PendingDecision::ChooseRestores {
        decider: UnitId(0),
        target: UnitId(1),
        count: 1,
    };
    app.world_mut()
        .resource_mut::<NextState<Mode>>()
        .set(Mode::Combat);
    app.update();
    app.update();
    assert_eq!(
        app.world().resource::<TurnOrder>().order(),
        &[UnitId(0), UnitId(2)]
    );

    app.world_mut()
        .resource_mut::<CommandQueue>()
        .push(IssuedCommand {
            seat: PlayerSeat::default(),
            command: GameCommand::ChooseRestores {
                unit: UnitId(0),
                target: UnitId(1),
                cells: vec![LatticeCoord::ORIGIN],
            },
        });
    app.update();
    let authority = hex_combat::authority_snapshot(app.world())
        .expect("the combat contract must run through the renderer-free authority");
    assert_eq!(
        authority.pending_revivals.get(&UnitId(1)),
        Some(&1),
        "the restoration adapter must publish its delayed initiative fact"
    );
    assert!(
        authority
            .units
            .get(&UnitId(1))
            .is_some_and(|unit| !unit.downed),
        "the restoration adapter must publish the revived unit"
    );
    assert_eq!(
        app.world().resource::<TurnOrder>().position_of(UnitId(1)),
        None,
        "revival must not splice into the current round"
    );

    for unit in [UnitId(0), UnitId(2)] {
        app.world_mut()
            .resource_mut::<CommandQueue>()
            .push(IssuedCommand {
                seat: PlayerSeat::default(),
                command: GameCommand::EndTurn { unit },
            });
        app.update();
    }
    assert_eq!(app.world().resource::<TurnOrder>().round, 1);
    assert_eq!(
        app.world().resource::<TurnOrder>().order(),
        &[UnitId(0), UnitId(1), UnitId(2)],
        "the revived unit should rejoin sorted initiative at the boundary"
    );
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
    assert_eq!(
        take_events(&mut app),
        vec![CombatEvent::HexesDisabled {
            source: UnitId(0),
            target: UnitId(1),
            cells: vec![LatticeCoord::ORIGIN],
        }]
    );
}

#[test]
fn the_auto_policy_waits_for_a_player_decider() {
    let mut app = test_app();
    let player = spawn(&mut app, UnitId(0), Faction::Player, HexCoord::ORIGIN);
    app.world_mut()
        .entity_mut(player)
        .insert((Player, ControlOwner::default()));
    spawn(
        &mut app,
        UnitId(1),
        Faction::Hostile,
        HexCoord::new_cubic(1, -1, 0),
    );
    app.world_mut()
        .resource_mut::<NextState<Mode>>()
        .set(Mode::Combat);
    app.update();

    *app.world_mut().resource_mut::<PendingDecision>() = PendingDecision::ChooseDisables {
        decider: UnitId(0),
        count: 1,
        source: UnitId(1),
    };
    publish_adapter_facts(&mut app);
    app.update();

    assert!(
        app.world().resource::<PendingDecision>().is_open(),
        "the AI must leave a player choice open for the UI"
    );
    assert!(
        app.world().resource::<CommandQueue>().is_empty(),
        "the AI must not queue a player answer"
    );
    assert!(
        !app.world()
            .entity(player)
            .get::<LatticeState>()
            .is_some_and(|state| state.is_disabled(LatticeCoord::ORIGIN)),
        "waiting for the player must not mutate the lattice"
    );
}

#[test]
fn the_auto_policy_uses_the_hostile_deciders_control_owner() {
    let mut app = test_app();
    let player = spawn(&mut app, UnitId(0), Faction::Player, HexCoord::ORIGIN);
    app.world_mut().entity_mut(player).insert(Player);
    let hostile = spawn(
        &mut app,
        UnitId(1),
        Faction::Hostile,
        HexCoord::new_cubic(1, -1, 0),
    );
    app.world_mut()
        .entity_mut(hostile)
        .insert(ControlOwner(PlayerSeat(7)));
    app.world_mut()
        .resource_mut::<NextState<Mode>>()
        .set(Mode::Combat);
    app.update();

    *app.world_mut().resource_mut::<PendingDecision>() = PendingDecision::ChooseDisables {
        decider: UnitId(1),
        count: 1,
        source: UnitId(0),
    };
    publish_adapter_facts(&mut app);
    app.update();

    assert!(
        !app.world().resource::<PendingDecision>().is_open(),
        "the correctly owned AI answer should apply"
    );
    assert!(
        app.world()
            .entity(hostile)
            .get::<LatticeState>()
            .is_some_and(|state| state.is_disabled(LatticeCoord::ORIGIN)),
        "the hostile's deterministic first choice should be disabled"
    );
    assert!(
        take_events(&mut app).iter().any(|event| matches!(
            event,
            CombatEvent::HexesDisabled {
                target: UnitId(1),
                ..
            }
        )),
        "the answer should resolve rather than being refused for seat zero"
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
        (
            vec![LatticeCoord::ORIGIN],
            CommandRefusal::WrongDisableCount {
                expected: 2,
                actual: 1,
            },
        ),
        // The same hex twice, which would satisfy the count while costing one.
        (
            vec![LatticeCoord::ORIGIN, LatticeCoord::ORIGIN],
            CommandRefusal::DuplicateCell {
                cell: LatticeCoord::ORIGIN,
            },
        ),
        // A cell that is not in this lattice at all.
        (
            vec![LatticeCoord::ORIGIN, LatticeCoord::new(9, 9)],
            CommandRefusal::CellOutsideLattice {
                cell: LatticeCoord::new(9, 9),
            },
        ),
    ];

    for (cells, refusal) in bad_answers {
        *app.world_mut().resource_mut::<PendingDecision>() = open.clone();
        let command = GameCommand::ChooseDisables {
            unit: UnitId(1),
            cells: cells.clone(),
        };
        app.world_mut()
            .resource_mut::<CommandQueue>()
            .push(IssuedCommand {
                seat: PlayerSeat::default(),
                command: command.clone(),
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
        assert_eq!(
            take_events(&mut app),
            vec![CombatEvent::CommandRefused { command, refusal }]
        );
    }
}

#[test]
fn a_wrong_decider_and_an_already_disabled_cell_are_refused() {
    let mut app = test_app();
    spawn(&mut app, UnitId(1), Faction::Hostile, HexCoord::ORIGIN);
    spawn(
        &mut app,
        UnitId(2),
        Faction::Hostile,
        HexCoord::new_cubic(1, -1, 0),
    );

    *app.world_mut().resource_mut::<PendingDecision>() = PendingDecision::ChooseDisables {
        decider: UnitId(1),
        count: 1,
        source: UnitId(0),
    };
    let wrong = GameCommand::ChooseDisables {
        unit: UnitId(2),
        cells: vec![LatticeCoord::ORIGIN],
    };
    app.world_mut()
        .resource_mut::<CommandQueue>()
        .push(IssuedCommand {
            seat: PlayerSeat::default(),
            command: wrong.clone(),
        });
    app.update();
    assert_eq!(
        take_events(&mut app),
        vec![CombatEvent::CommandRefused {
            command: wrong,
            refusal: CommandRefusal::WrongDecisionUnit {
                expected: UnitId(1),
            },
        }]
    );

    let first = GameCommand::ChooseDisables {
        unit: UnitId(1),
        cells: vec![LatticeCoord::ORIGIN],
    };
    app.world_mut()
        .resource_mut::<CommandQueue>()
        .push(IssuedCommand {
            seat: PlayerSeat::default(),
            command: first,
        });
    app.update();
    take_events(&mut app);

    *app.world_mut().resource_mut::<PendingDecision>() = PendingDecision::ChooseDisables {
        decider: UnitId(1),
        count: 1,
        source: UnitId(0),
    };
    let corpse = GameCommand::ChooseDisables {
        unit: UnitId(1),
        cells: vec![LatticeCoord::ORIGIN],
    };
    app.world_mut()
        .resource_mut::<CommandQueue>()
        .push(IssuedCommand {
            seat: PlayerSeat::default(),
            command: corpse.clone(),
        });
    app.update();
    assert_eq!(
        take_events(&mut app),
        vec![CombatEvent::CommandRefused {
            command: corpse,
            refusal: CommandRefusal::CellAlreadyDisabled {
                cell: LatticeCoord::ORIGIN,
            },
        }]
    );
    assert!(
        app.world().resource::<PendingDecision>().is_open(),
        "a refused corpse choice keeps the real decision open"
    );
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
    publish_spatial_knowledge(&mut app);

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
    let learned = app
        .world_mut()
        .resource_mut::<FactionLatticeKnowledge>()
        .learn(
            Faction::Player,
            UnitId(1),
            LatticeCoord::ORIGIN,
            KnownCell {
                kind: CellKind::Gem {
                    element: ElementId(0),
                },
                mana: Some(3),
                disabled: false,
                source: KnowledgeSource::Divination,
                expiry: KnowledgeExpiry::Sustained,
            },
        );
    assert!(learned, "precondition: the hostile lattice is known");

    // Take both hexes down through the real path: park a decision, answer it.
    *app.world_mut().resource_mut::<PendingDecision>() = PendingDecision::ChooseDisables {
        decider: UnitId(1),
        count: 2,
        source: UnitId(0),
    };
    publish_adapter_facts(&mut app);
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
    assert!(
        app.world()
            .resource::<FactionLatticeKnowledge>()
            .view(Faction::Player, UnitId(1))
            .and_then(|known| known.cell(LatticeCoord::ORIGIN))
            .is_some(),
        "knowledge of a retained downed unit must survive until actual despawn"
    );
    assert_eq!(
        take_events(&mut app),
        vec![
            CombatEvent::HexesDisabled {
                source: UnitId(0),
                target: UnitId(1),
                cells: vec![LatticeCoord::ORIGIN, LatticeCoord::new(1, 0)],
            },
            CombatEvent::Downed { unit: UnitId(1) },
            CombatEvent::EncounterResolved {
                outcome: hex_combat::EncounterOutcome::Victory,
            },
        ],
        "exact disables precede the downing they caused"
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

/// A downed unit's lattice remains reachable for the restoration flow.
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
    app.world_mut().entity_mut(defender).insert(Downed);
    app.world_mut()
        .resource_mut::<NextState<Mode>>()
        .set(Mode::Combat);
    app.update();
    app.update();
    assert!(
        app.world().entity(defender).contains::<Downed>(),
        "a spent lattice should put its unit down"
    );

    // The engine primitive reaches the retained lattice. Command-level tests cover
    // removing Downed and scheduling the unit's initiative re-entry.
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
