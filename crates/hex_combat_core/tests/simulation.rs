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
    ControllerInput, ElementNames, RulesProfile, RunBounds,
};
use hex_core::{
    ElementId, Faction, GameCommand, HexCoord, IssuedCommand, LatticeCoord, PlayerSeat, SpellId,
    TilePos, UnitId,
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
    ArenaSnapshot::new(surfaces.iter().copied(), [])
        .expect("fixture arena is valid")
        .with_observation(Faction::Player, surfaces.iter().copied())
        .with_observation(Faction::Hostile, surfaces)
}

fn roster_case(name: &str, rules: RulesProfile, side_size: usize, turns: u32) -> CombatCase {
    let player_positions = [
        position(-2, 0),
        position(-2, 1),
        position(-1, -1),
        position(-3, 0),
        position(-3, 1),
        position(-2, -1),
    ];
    let hostile_positions = [
        position(0, 0),
        position(0, 1),
        position(1, -1),
        position(1, 0),
        position(1, 1),
        position(0, -1),
    ];
    let mut units = Vec::with_capacity(side_size.saturating_mul(2));
    for (index, &position) in player_positions.iter().take(side_size).enumerate() {
        units.push(CombatUnit::new(
            UnitId(u64::try_from(index).unwrap_or(u64::MAX)),
            PlayerSeat(0),
            Faction::Player,
            position,
            100_u32.saturating_sub(u32::try_from(index).unwrap_or(u32::MAX)),
        ));
    }
    for (index, &position) in hostile_positions.iter().take(side_size).enumerate() {
        units.push(CombatUnit::new(
            UnitId(
                u64::try_from(side_size)
                    .unwrap_or(u64::MAX)
                    .saturating_add(u64::try_from(index).unwrap_or(u64::MAX)),
            ),
            PlayerSeat(0),
            Faction::Hostile,
            position,
            50_u32.saturating_sub(u32::try_from(index).unwrap_or(u32::MAX)),
        ));
    }
    CombatCase {
        name: name.to_owned(),
        rules,
        arena: arena_for(units.iter().map(|unit| unit.position)),
        elements: ElementNames::default(),
        controllers: units
            .iter()
            .map(|unit| (unit.id, ControllerInput::Scripted(unit.seat)))
            .collect(),
        units,
        bounds: RunBounds {
            max_commands: turns,
        },
    }
}

#[test]
fn shipped_tactical_and_custom_three_step_profiles_are_deterministic() {
    for (name, rules, expected_budget) in [
        ("shipped-3v3", profile("Shipped", 4), 4),
        ("tactical-3v3", profile("Tactical", 2), 2),
        ("custom-3v3", profile("Custom", 3), 3),
    ] {
        let case = roster_case(name, rules, 3, 12);
        let first = case.run().expect("first canonical run");
        let second = case.run().expect("second canonical run");
        assert_eq!(first, second, "{name} diverged across identical runs");
        assert_eq!(
            first
                .turn
                .active
                .map(|turn| turn.movement_left)
                .unwrap_or_default(),
            expected_budget
        );
        assert_eq!(first.summary.turns, case.bounds.max_commands);
        assert_eq!(first.summary.successful_commands, case.bounds.max_commands);
        assert_eq!(first.summary.idle_turns, case.bounds.max_commands);
        assert_eq!(first.positions.len(), 6);
        assert!(matches!(
            first.termination,
            CombatTermination::BoundedNoProgress {
                completed_turns,
                no_progress_streak
            } if completed_turns == case.bounds.max_commands
                && no_progress_streak == case.bounds.max_commands
        ));
    }
}

#[test]
fn deterministic_six_by_six_run_has_exact_unique_occupancy_and_bounded_telemetry() {
    let case = roster_case("shipped-6v6", profile("Shipped", 4), 6, 24);
    let first = case.run().expect("first canonical run");
    let second = case.run().expect("second canonical run");
    assert_eq!(first, second, "the 6v6 canonical snapshots diverged");
    assert_eq!(first.turn.order.len(), 12);
    assert_eq!(first.positions.len(), 12);
    let unique = first.positions.values().collect::<BTreeSet<_>>();
    assert_eq!(unique.len(), 12, "two bodies share an exact surface");
    assert_eq!(first.summary.turns, 24);
    assert_ne!(first.state_fingerprint, 0);
    assert_ne!(first.command_fingerprint, 0);
    assert_ne!(first.transcript_fingerprint, 0);
    assert_eq!(first.transcript_event_count, 24);
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
