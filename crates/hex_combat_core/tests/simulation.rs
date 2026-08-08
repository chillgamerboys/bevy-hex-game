//! Canonical multi-turn gameplay simulations.
//!
//! This target intentionally imports no Bevy App, renderer, viewport, clock, map
//! generator, ECS entity, or asset server. Focused verb/effect/AI contracts remain
//! beside their owners; these cases own deterministic composition claims.

#![expect(
    clippy::expect_used,
    reason = "invalid canonical fixture construction must fail at the fixture seam"
)]

use std::collections::{BTreeMap, BTreeSet};

use hex_combat_core::{
    ArenaSnapshot, CombatCase, CombatEvent, CombatState, CombatTermination, CombatUnit,
    ControllerInput, ElementNames, FrozenCombatContent, RulesProfile, RunBounds,
};
use hex_core::{
    deterministic_fixture, ElementId, Faction, GameCommand, HexCoord, IssuedCommand, LatticeCoord,
    PlayerSeat, SpellId, TilePos, UnitId,
};
use hex_lattice::{
    apply_cast, castable, Casting, CellKind, FusionTable, LatticeSpec, LatticeState, LatticeStats,
    Requirement, SpellTable,
};

const LEVEL: i32 = 1;

fn position(q: i32, r: i32) -> TilePos {
    TilePos::new(HexCoord::from_axial(q, r), LEVEL)
}

fn profile(name: &str, movement_per_turn: u32) -> RulesProfile {
    RulesProfile::new(name, movement_per_turn).expect("fixture profile is valid")
}

fn arena_for(positions: impl IntoIterator<Item = TilePos>) -> ArenaSnapshot {
    let surfaces: BTreeSet<_> = positions.into_iter().collect();
    let links = surfaces
        .iter()
        .flat_map(|from| {
            surfaces
                .iter()
                .filter(move |to| from.coord.distance(to.coord) == 1)
                .map(move |to| (*from, *to))
        })
        .collect::<Vec<_>>();
    ArenaSnapshot::new(surfaces.iter().copied(), links)
        .expect("fixture arena is valid")
        .with_observation(Faction::Player, surfaces.iter().copied())
        .with_observation(Faction::Hostile, surfaces)
}

fn combatant(id: UnitId, faction: Faction, position: TilePos, initiative: u32) -> CombatUnit {
    let spec = LatticeSpec::default().with(LatticeCoord::ORIGIN, CellKind::Blank);
    let stats = LatticeStats::default();
    let state = LatticeState::new(&spec, &stats);
    CombatUnit::new(id, PlayerSeat(0), faction, position, initiative)
        .with_lattice(spec, state, stats)
}

fn roster_case(name: &str, rules: RulesProfile, side_size: usize) -> CombatCase {
    let mut units = Vec::with_capacity(side_size.saturating_mul(2));
    for index in 0..side_size {
        let lane = i32::try_from(index).unwrap_or(i32::MAX).saturating_mul(2);
        units.push(combatant(
            UnitId(u64::try_from(index).unwrap_or(u64::MAX)),
            Faction::Player,
            position(0, lane),
            100_u32.saturating_sub(u32::try_from(index).unwrap_or(u32::MAX)),
        ));
    }
    for index in 0..side_size {
        let lane = i32::try_from(index).unwrap_or(i32::MAX).saturating_mul(2);
        units.push(combatant(
            UnitId(
                u64::try_from(side_size)
                    .unwrap_or(u64::MAX)
                    .saturating_add(u64::try_from(index).unwrap_or(u64::MAX)),
            ),
            Faction::Hostile,
            position(2, lane),
            50_u32.saturating_sub(u32::try_from(index).unwrap_or(u32::MAX)),
        ));
    }
    let surfaces = (0..side_size).flat_map(|index| {
        let lane = i32::try_from(index).unwrap_or(i32::MAX).saturating_mul(2);
        (0..=2).map(move |q| position(q, lane))
    });
    CombatCase {
        name: name.to_owned(),
        rules,
        arena: arena_for(surfaces),
        elements: ElementNames::default(),
        content: FrozenCombatContent::default(),
        controllers: units
            .iter()
            .map(|unit| (unit.id, ControllerInput::Baseline { seat: unit.seat }))
            .collect(),
        units,
        bounds: RunBounds::new(256, 128, 32).expect("fixture bounds"),
    }
}

#[test]
fn shipped_tactical_and_custom_three_step_profiles_are_deterministic() {
    let fixture = deterministic_fixture("tempo-matrix").expect("shared deterministic fixture");
    assert!(fixture.simulated);
    assert!(fixture.profile_matrix);
    assert_eq!(fixture.party.len(), fixture.enemies.len());
    let side_size = fixture.party.len();
    for (profile_name, rules, expected_budget) in [
        ("shipped", profile("Shipped", 4), 4),
        ("tactical", profile("Tactical", 2), 2),
        ("custom", profile("Custom", 3), 3),
    ] {
        let name = format!("{}-{profile_name}", fixture.id);
        let case = roster_case(&name, rules, side_size);
        let first = case.run().expect("first canonical run");
        let second = case.run().expect("second canonical run");
        assert_eq!(first, second, "{name} diverged across identical runs");
        assert_eq!(first.state.rules.movement_per_turn, expected_budget);
        assert_eq!(
            first.termination,
            CombatTermination::Outcome(hex_combat_core::EncounterOutcome::Victory)
        );
        assert_eq!(
            first
                .state
                .commands
                .iter()
                .filter(|issued| matches!(issued.command, GameCommand::Strike { .. }))
                .count(),
            3
        );
        assert_eq!(
            first
                .state
                .events
                .iter()
                .filter(|event| matches!(event, CombatEvent::Downed { .. }))
                .count(),
            3
        );
        assert!(first
            .state
            .commands
            .iter()
            .any(|issued| matches!(issued.command, GameCommand::MoveAlong { .. })));
        assert_eq!(first.positions.len(), 6);
    }
}

#[test]
fn deterministic_fixture_manifest_preserves_stable_review_identities() {
    for id in [
        "ability-lab",
        "raider-mirror",
        "creator-spell-matrix",
        "creator-roster-matrix",
        "occupancy-matrix",
        "channel-attrition",
        "tempo-matrix",
    ] {
        assert_eq!(
            deterministic_fixture(id).map(|fixture| fixture.id),
            Some(id)
        );
    }
}

#[test]
fn deterministic_six_by_six_run_has_exact_unique_occupancy_and_bounded_telemetry() {
    let case = roster_case("shipped-6v6", profile("Shipped", 4), 6);
    let first = case.run().expect("first canonical run");
    let second = case.run().expect("second canonical run");
    assert_eq!(first, second, "the 6v6 canonical snapshots diverged");
    assert_eq!(
        first.termination,
        CombatTermination::Outcome(hex_combat_core::EncounterOutcome::Victory)
    );
    assert!(
        first
            .state
            .commands
            .iter()
            .filter(|issued| matches!(issued.command, GameCommand::Strike { .. }))
            .count()
            >= 6
    );
    assert!((6..12).map(UnitId).all(|unit| first
        .state
        .units
        .get(&unit)
        .is_some_and(|actor| actor.downed)));
    assert!(first
        .state
        .commands
        .iter()
        .any(|issued| matches!(issued.command, GameCommand::MoveAlong { .. })));
    assert_eq!(first.positions.len(), 12);
    let unique = first.positions.values().collect::<BTreeSet<_>>();
    assert_eq!(unique.len(), 12, "two bodies share an exact surface");
    assert_ne!(first.state_fingerprint, 0);
    assert_ne!(first.command_fingerprint, 0);
    assert_ne!(first.transcript_fingerprint, 0);
    assert!(first.transcript_event_count > 24);
}

#[test]
fn scripted_no_progress_replay_preserves_every_canonical_projection() {
    let mut case = roster_case("scripted-no-progress-3v3", profile("Shipped", 4), 3);
    case.controllers = case
        .units
        .iter()
        .map(|unit| {
            (
                unit.id,
                ControllerInput::Scripted {
                    seat: unit.seat,
                    commands: (0..12)
                        .map(|_| GameCommand::EndTurn { unit: unit.id })
                        .collect(),
                },
            )
        })
        .collect();
    case.bounds = RunBounds::new(13, 13, 12).expect("no-progress bounds");

    let first = case.run().expect("first exact replay");
    let second = case.run().expect("second exact replay");
    assert_eq!(first, second, "complete scripted snapshots diverged");
    assert_eq!(first.state.commands, second.state.commands);
    assert_eq!(first.state.events, second.state.events);
    assert_eq!(first.positions, second.positions);
    assert_eq!(first.lattices, second.lattices);
    assert_eq!(first.turn, second.turn);
    assert_eq!(first.termination, second.termination);
    assert_eq!(
        first.termination,
        CombatTermination::NoProgressBoundReached {
            completed_turns: 12,
            no_progress_streak: 12,
        }
    );
}

#[test]
fn baseline_controller_resolves_defender_choice_to_a_real_outcome() {
    let left = position(0, 0);
    let right = position(1, 0);
    let arena = ArenaSnapshot::new([left, right], [(left, right), (right, left)])
        .expect("fixture arena")
        .with_observation(Faction::Player, [left, right])
        .with_observation(Faction::Hostile, [left, right]);
    let lattice = || {
        let spec = LatticeSpec::default().with(LatticeCoord::ORIGIN, CellKind::Blank);
        let stats = LatticeStats::default();
        let state = LatticeState::new(&spec, &stats);
        (spec, state, stats)
    };
    let (player_spec, player_state, player_stats) = lattice();
    let (hostile_spec, hostile_state, hostile_stats) = lattice();
    let units = vec![
        CombatUnit::new(UnitId(0), PlayerSeat(0), Faction::Player, left, 20).with_lattice(
            player_spec,
            player_state,
            player_stats,
        ),
        CombatUnit::new(UnitId(1), PlayerSeat(0), Faction::Hostile, right, 10).with_lattice(
            hostile_spec,
            hostile_state,
            hostile_stats,
        ),
    ];
    let case = CombatCase {
        name: "baseline-outcome".to_owned(),
        rules: profile("Shipped", 4),
        arena,
        elements: ElementNames::default(),
        content: FrozenCombatContent::default(),
        controllers: units
            .iter()
            .map(|unit| (unit.id, ControllerInput::Baseline { seat: unit.seat }))
            .collect(),
        units,
        bounds: RunBounds::new(12, 8, 4).expect("bounds"),
    };
    let snapshot = case.run().expect("baseline run");
    assert_eq!(
        snapshot.termination,
        CombatTermination::Outcome(hex_combat_core::EncounterOutcome::Victory)
    );
    assert_eq!(snapshot.summary.successful_commands, 2);
    assert!(snapshot
        .state
        .events
        .iter()
        .any(|event| matches!(event, CombatEvent::Downed { unit: UnitId(1) })));
}

fn chokepoint() -> ArenaSnapshot {
    let left = position(-1, 0);
    let middle = position(0, 0);
    let right = position(1, 0);
    ArenaSnapshot::new(
        [left, middle, right],
        [
            (left, middle),
            (middle, left),
            (middle, right),
            (right, middle),
        ],
    )
    .expect("fixture chokepoint is valid")
    .with_observation(Faction::Player, [left, middle, right])
    .with_observation(Faction::Hostile, [left, middle, right])
}

#[test]
fn chokepoint_occupancy_refusal_is_canonical_and_position_preserving() {
    let run = || {
        let mut state = CombatState::start(
            profile("Shipped", 4),
            chokepoint(),
            ElementNames::default(),
            [
                CombatUnit::new(
                    UnitId(0),
                    PlayerSeat(0),
                    Faction::Player,
                    position(-1, 0),
                    20,
                ),
                CombatUnit::new(
                    UnitId(1),
                    PlayerSeat(0),
                    Faction::Player,
                    position(0, 0),
                    10,
                ),
                CombatUnit::new(
                    UnitId(2),
                    PlayerSeat(0),
                    Faction::Hostile,
                    position(1, 0),
                    5,
                ),
            ],
        )
        .expect("fixture state");
        let refusal = state.apply(IssuedCommand {
            seat: PlayerSeat(0),
            command: GameCommand::MoveAlong {
                unit: UnitId(0),
                path: vec![position(-1, 0), position(0, 0)],
            },
        });
        (state, refusal)
    };

    let first = run();
    let second = run();
    assert_eq!(first, second);
    assert!(first.1.is_err());
    assert_eq!(first.0.metrics.successful_commands, 0);
    assert_eq!(first.0.metrics.refused_commands, 1);
    assert_eq!(
        first.0.units.get(&UnitId(0)).map(|unit| unit.position),
        Some(position(-1, 0))
    );
    assert!(first.0.events.iter().any(|event| matches!(
        event,
        CombatEvent::CommandRefused {
            command: GameCommand::MoveAlong {
                unit: UnitId(0),
                ..
            },
            ..
        }
    )));
}

struct ChannelTables {
    fire: ElementId,
}

impl FusionTable for ChannelTables {
    fn recipe(&self, _output: ElementId) -> Option<Vec<Requirement>> {
        None
    }
}

impl SpellTable for ChannelTables {
    fn requirements(&self, _spell: SpellId) -> Vec<Requirement> {
        vec![Requirement {
            element: self.fire,
            mana: 2,
        }]
    }

    fn casting(&self, _spell: SpellId) -> Casting {
        Casting::Evocation
    }
}

fn depleted_channel_lattice() -> (ElementId, LatticeSpec, LatticeState, LatticeStats) {
    let fire = ElementId(0);
    let spell = LatticeCoord::ORIGIN;
    let [gem, ..] = spell.neighbors();
    let spec = LatticeSpec::default()
        .with(spell, CellKind::Spell { spell: SpellId(0) })
        .with(gem, CellKind::Gem { element: fire });
    let stats = LatticeStats::new(BTreeMap::from([(fire, 3)]), BTreeMap::from([(fire, 2)]));
    let mut state = LatticeState::new(&spec, &stats);
    let tables = ChannelTables { fire };
    let plan = castable(&spec, &state, spell, &tables).expect("fixture cast is legal");
    assert!(apply_cast(&mut state, &plan, &tables));
    (fire, spec, state, stats)
}

#[test]
fn channel_restores_exact_state_and_spends_only_one_action() {
    let run = || {
        let (fire, spec, lattice, stats) = depleted_channel_lattice();
        let mut state = CombatState::start(
            profile("Shipped", 4),
            chokepoint(),
            ElementNames::new(BTreeMap::from([(fire, "Fire".to_owned())])),
            [
                CombatUnit::new(
                    UnitId(0),
                    PlayerSeat(0),
                    Faction::Player,
                    position(-1, 0),
                    20,
                )
                .with_lattice(spec, lattice, stats),
                CombatUnit::new(
                    UnitId(1),
                    PlayerSeat(0),
                    Faction::Hostile,
                    position(1, 0),
                    10,
                ),
            ],
        )
        .expect("fixture state");
        let command = IssuedCommand {
            seat: PlayerSeat(0),
            command: GameCommand::Channel { unit: UnitId(0) },
        };
        let first = state.apply(command.clone());
        let second = state.apply(command);
        (state, first, second)
    };

    let first = run();
    let second = run();
    assert_eq!(first, second, "identical Channel cases diverged");
    assert!(first.1.is_ok());
    assert!(first.2.is_err());
    assert_eq!(first.0.metrics.channels, 1);
    assert_eq!(first.0.metrics.channelled_mana.get("Fire"), Some(&2));
    assert_eq!(first.0.metrics.successful_commands, 1);
    assert_eq!(first.0.metrics.refused_commands, 1);
    assert_eq!(
        first
            .0
            .units
            .get(&UnitId(0))
            .and_then(|unit| unit.lattice.as_ref())
            .map(|lattice| lattice.state.total_gem_mana()),
        Some(3)
    );
    assert_eq!(
        first
            .0
            .units
            .get(&UnitId(0))
            .and_then(|unit| unit.turn)
            .map(|turn| turn.acted),
        Some(true)
    );
}

#[test]
fn profiles_keep_stable_named_identity() {
    assert_eq!(
        [
            profile("Shipped", 4).name,
            profile("Tactical", 2).name,
            profile("Custom", 3).name,
        ],
        ["Shipped", "Tactical", "Custom"]
    );
}
