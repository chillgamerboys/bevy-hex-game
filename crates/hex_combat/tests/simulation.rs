//! Deterministic multi-turn gameplay simulations.
//!
//! Focused command, effect, and AI tests remain beside their owning contracts.
//! This target owns composition claims: profile matrices, roster-size runs,
//! canonical snapshots, exact occupancy, Channel accounting, and typed bounded
//! no-progress termination.

use std::collections::BTreeMap;

use bevy::prelude::*;
use hex_assets::{
    CombatRulesPreset, CombatRulesProfile, CombatSettings, ElementCatalog, ElementFile,
    PlayerSettings,
};
use hex_combat::{
    CombatEvent, CombatSummary, CombatTranscriptRecorder, EncounterOutcome, EncounterResolution,
    Initiative, TurnOrder,
};
use hex_core::{
    CommandQueue, ControlOwner, ElementId, GameCommand, HexCoord, HexSpan, IssuedCommand,
    LatticeCoord, Mode, PlayerSeat, SpellId, TilePos, Turn, UnitId,
};
use hex_lattice::{
    apply_cast, castable, Casting, CellKind, FusionTable, LatticeSpec, LatticeState, LatticeStats,
    Requirement, SpellTable,
};
use hex_test_support::{enter_gameplay, SyntheticArena, TestAppBuilder};
use hex_units::{Body, Faction, Standing, StandsOn, UnitRegistry};
use serde::Serialize;
use xxhash_rust::xxh3::xxh3_64;

const LEVEL: i32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ControllerInput {
    Scripted(PlayerSeat),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UnitInput {
    id: UnitId,
    faction: Faction,
    position: TilePos,
    initiative: u32,
    controller: ControllerInput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RunBounds {
    turns: u32,
    frames_per_command: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CombatCase {
    name: &'static str,
    profile: CombatRulesProfile,
    units: Vec<UnitInput>,
    bounds: RunBounds,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CombatTermination {
    Outcome(EncounterOutcome),
    BoundedNoProgress {
        completed_turns: u32,
        no_progress_streak: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TurnStateSnapshot {
    order: Vec<UnitId>,
    current: Option<UnitId>,
    round: u32,
    opening_movement_budget: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LatticeSnapshot {
    unit: UnitId,
    total_mana: u32,
    locked_mana: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CombatRunSnapshot {
    case: &'static str,
    summary: CombatSummary,
    summary_fingerprint: u64,
    command_fingerprint: u64,
    transcript_event_count: usize,
    transcript_fingerprint: u64,
    termination: CombatTermination,
    turn: TurnStateSnapshot,
    lattices: Vec<LatticeSnapshot>,
    positions: BTreeMap<UnitId, TilePos>,
}

fn stable_fingerprint(domain: &[u8], value: &impl Serialize) -> u64 {
    let mut bytes = domain.to_vec();
    assert!(
        serde_json::to_writer(&mut bytes, value).is_ok(),
        "canonical simulation data must serialize"
    );
    xxh3_64(&bytes)
}

fn position(q: i32, r: i32) -> TilePos {
    TilePos::new(HexCoord::from_axial(q, r), LEVEL)
}

fn roster_case(name: &'static str, profile: CombatRulesProfile, side_size: usize) -> CombatCase {
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
        let id = u64::try_from(index).unwrap_or(u64::MAX);
        units.push(UnitInput {
            id: UnitId(id),
            faction: Faction::Player,
            position,
            initiative: 100_u32.saturating_sub(u32::try_from(index).unwrap_or(u32::MAX)),
            controller: ControllerInput::Scripted(PlayerSeat(0)),
        });
    }
    for (index, &position) in hostile_positions.iter().take(side_size).enumerate() {
        let offset = u64::try_from(side_size).unwrap_or(u64::MAX);
        let id = offset.saturating_add(u64::try_from(index).unwrap_or(u64::MAX));
        units.push(UnitInput {
            id: UnitId(id),
            faction: Faction::Hostile,
            position,
            initiative: 50_u32.saturating_sub(u32::try_from(index).unwrap_or(u32::MAX)),
            controller: ControllerInput::Scripted(PlayerSeat(0)),
        });
    }
    CombatCase {
        name,
        profile,
        units,
        bounds: RunBounds {
            turns: u32::try_from(side_size.saturating_mul(4)).unwrap_or(u32::MAX),
            frames_per_command: 4,
        },
    }
}

fn spawn_unit(app: &mut App, input: UnitInput) -> Entity {
    let span = HexSpan::new(1.0, 2.0);
    let standing = Standing {
        pos: input.position,
        span,
    };
    let ControllerInput::Scripted(seat) = input.controller;
    let entity = app
        .world_mut()
        .spawn((
            input.id,
            input.faction,
            ControlOwner(seat),
            StandsOn(standing),
            Body::new(hex_core::TraversalProfile::WALKER),
            Initiative(input.initiative),
            Transform::from_translation(standing.world_position()),
        ))
        .id();
    app.world_mut()
        .resource_mut::<UnitRegistry>()
        .register(input.id, entity);
    entity
}

#[expect(
    clippy::expect_used,
    reason = "invalid deterministic fixture inputs must fail at their construction seam"
)]
fn simulation_app(profile: &CombatRulesProfile) -> App {
    let shipped = CombatSettings::default();
    let settings = profile
        .effective_settings(&shipped)
        .expect("a test case profile must be valid against shipped settings");
    let mut builder = TestAppBuilder::new()
        .with_arena(SyntheticArena::chokepoint(LEVEL))
        .expect("the synthetic arena fixture must be valid");
    builder
        .app_mut()
        .insert_resource(settings)
        .insert_resource(ElementCatalog::from_file(&ElementFile {
            wheel: vec!["Fire".to_owned(), "Water".to_owned()],
            fusions: bevy::platform::collections::HashMap::default(),
        }))
        .insert_resource(PlayerSettings {
            scale: 0.25,
            speed: 5.0,
        })
        .add_plugins((
            hex_anim::plugin,
            hex_units::movement::plugin,
            hex_combat::plugin,
        ));
    builder.build()
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

#[expect(
    clippy::expect_used,
    reason = "invalid deterministic fixture content must fail at its construction seam"
)]
fn insert_depleted_channel_lattice(app: &mut App, entity: Entity) {
    let fire = app
        .world()
        .resource::<ElementCatalog>()
        .id("Fire")
        .expect("the deterministic catalog defines Fire");
    let spell = LatticeCoord::ORIGIN;
    let [gem, ..] = spell.neighbors();
    let spec = LatticeSpec::default()
        .with(spell, CellKind::Spell { spell: SpellId(0) })
        .with(gem, CellKind::Gem { element: fire });
    let stats = LatticeStats::new(BTreeMap::from([(fire, 3)]), BTreeMap::from([(fire, 2)]));
    let mut state = LatticeState::new(&spec, &stats);
    let tables = ChannelTables { fire };
    let plan = castable(&spec, &state, spell, &tables)
        .expect("the deterministic spell drains its one gem");
    assert!(apply_cast(&mut state, &plan, &tables));
    app.world_mut()
        .entity_mut(entity)
        .insert((spec, state, stats));
}

#[expect(
    clippy::expect_used,
    reason = "a missing canonical turn fact is the simulation failure this helper reports"
)]
fn run_case(case: &CombatCase) -> CombatRunSnapshot {
    let mut app = simulation_app(&case.profile);
    for input in &case.units {
        spawn_unit(&mut app, *input);
    }
    app.world_mut()
        .resource_mut::<CombatTranscriptRecorder>()
        .enable();
    enter_gameplay(&mut app);
    app.update();
    assert_eq!(
        *app.world().resource::<State<Mode>>().get(),
        Mode::Combat,
        "{} must enter combat",
        case.name
    );

    let opening_movement_budget = {
        let current = app
            .world()
            .resource::<TurnOrder>()
            .current()
            .expect("the simulation has a current actor");
        let registry = app.world().resource::<UnitRegistry>();
        let entity = registry
            .entity_of(current)
            .expect("the current stable id is registered");
        app.world()
            .get::<Turn>(entity)
            .expect("the current actor has a turn")
            .movement_left
    };

    for command_index in 0..case.bounds.turns {
        let current = app
            .world()
            .resource::<TurnOrder>()
            .current()
            .expect("bounded no-progress simulation keeps a current actor");
        app.world_mut()
            .resource_mut::<CommandQueue>()
            .push(IssuedCommand {
                seat: PlayerSeat(0),
                command: GameCommand::EndTurn { unit: current },
            });
        for _ in 0..case.bounds.frames_per_command {
            app.update();
            if app.world().resource::<CommandQueue>().is_empty() {
                break;
            }
        }
        assert!(
            app.world().resource::<CommandQueue>().is_empty(),
            "{} command {command_index} exceeded its deterministic frame bound",
            case.name
        );
    }

    let order = app.world().resource::<TurnOrder>();
    let turn = TurnStateSnapshot {
        order: order.order().to_vec(),
        current: order.current(),
        round: order.round,
        opening_movement_budget,
    };
    let summary = app.world().resource::<CombatSummary>().clone();
    let resolution = app.world().resource::<EncounterResolution>().outcome();
    let termination = resolution.map_or(
        CombatTermination::BoundedNoProgress {
            completed_turns: summary.turns,
            no_progress_streak: summary.no_progress_current,
        },
        CombatTermination::Outcome,
    );
    let transcript = app.world().resource::<CombatTranscriptRecorder>();
    let transcript_event_count = transcript.events().len();
    let command_fingerprint = stable_fingerprint(
        b"combat-run-commands-v1",
        &(&summary.commands, &summary.refusals),
    );
    let transcript_fingerprint =
        stable_fingerprint(b"combat-run-transcript-v1", &transcript.events());
    let positions = {
        let world = app.world_mut();
        let mut query = world.query::<(&UnitId, &StandsOn)>();
        query
            .iter(world)
            .map(|(&unit, standing)| (unit, standing.0.pos))
            .collect()
    };
    let lattices = {
        let world = app.world_mut();
        let mut query = world.query::<(&UnitId, &hex_lattice::LatticeState)>();
        let mut snapshots = query
            .iter(world)
            .map(|(&unit, state)| LatticeSnapshot {
                unit,
                total_mana: state.total_gem_mana(),
                locked_mana: state.total_locked_mana(),
            })
            .collect::<Vec<_>>();
        snapshots.sort_by_key(|snapshot| snapshot.unit);
        snapshots
    };
    CombatRunSnapshot {
        case: case.name,
        summary_fingerprint: summary.fingerprint(),
        command_fingerprint,
        summary,
        transcript_event_count,
        transcript_fingerprint,
        termination,
        turn,
        lattices,
        positions,
    }
}

#[test]
fn shipped_tactical_and_custom_three_step_profiles_are_deterministic() {
    let shipped = CombatSettings::default();
    let mut custom = CombatRulesProfile::custom_from(&CombatRulesProfile::shipped(&shipped));
    custom.movement_per_turn = 3;
    let profiles = [
        ("shipped-3v3", CombatRulesProfile::shipped(&shipped), 4),
        (
            "tactical-3v3",
            CombatRulesProfile::tactical_two_step(&shipped),
            2,
        ),
        ("custom-3v3", custom, 3),
    ];

    for (name, profile, expected_budget) in profiles {
        let case = roster_case(name, profile, 3);
        let first = run_case(&case);
        let second = run_case(&case);
        assert_eq!(first, second, "{name} diverged across identical runs");
        assert_eq!(first.turn.opening_movement_budget, expected_budget);
        assert_eq!(first.summary.turns, case.bounds.turns);
        assert_eq!(first.summary.successful_commands, case.bounds.turns);
        assert_eq!(first.summary.idle_turns, case.bounds.turns);
        assert_eq!(first.positions.len(), 6);
        assert!(matches!(
            first.termination,
            CombatTermination::BoundedNoProgress {
                completed_turns,
                no_progress_streak
            } if completed_turns == case.bounds.turns && no_progress_streak == case.bounds.turns
        ));
    }
}

#[test]
fn deterministic_six_by_six_run_has_exact_unique_occupancy_and_bounded_telemetry() {
    let shipped = CombatSettings::default();
    let case = roster_case("shipped-6v6", CombatRulesProfile::shipped(&shipped), 6);
    let first = run_case(&case);
    let second = run_case(&case);
    assert_eq!(first, second, "the 6v6 canonical snapshots diverged");
    assert_eq!(first.turn.order.len(), 12);
    assert_eq!(first.positions.len(), 12);
    let unique = first
        .positions
        .values()
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(unique.len(), 12, "two bodies share an exact surface");
    assert_eq!(first.summary.turns, 24);
    assert_ne!(first.summary_fingerprint, 0);
    assert_ne!(first.command_fingerprint, 0);
    assert_ne!(first.transcript_fingerprint, 0);
    assert_eq!(
        first.transcript_event_count,
        usize::try_from(first.summary.event_count).unwrap_or(usize::MAX)
    );
}

#[test]
fn chokepoint_occupancy_refusal_is_canonical_and_position_preserving() {
    let shipped = CombatSettings::default();
    let profile = CombatRulesProfile::shipped(&shipped);
    let run = || {
        let mut app = simulation_app(&profile);
        for input in [
            UnitInput {
                id: UnitId(0),
                faction: Faction::Player,
                position: position(-1, 0),
                initiative: 20,
                controller: ControllerInput::Scripted(PlayerSeat(0)),
            },
            UnitInput {
                id: UnitId(1),
                faction: Faction::Player,
                position: position(0, 0),
                initiative: 10,
                controller: ControllerInput::Scripted(PlayerSeat(0)),
            },
            UnitInput {
                id: UnitId(2),
                faction: Faction::Hostile,
                position: position(1, 0),
                initiative: 5,
                controller: ControllerInput::Scripted(PlayerSeat(0)),
            },
        ] {
            spawn_unit(&mut app, input);
        }
        app.world_mut()
            .resource_mut::<CombatTranscriptRecorder>()
            .enable();
        enter_gameplay(&mut app);
        app.update();
        app.world_mut()
            .resource_mut::<CommandQueue>()
            .push(IssuedCommand {
                seat: PlayerSeat(0),
                command: GameCommand::MoveAlong {
                    unit: UnitId(0),
                    path: vec![position(-1, 0), position(0, 0)],
                },
            });
        app.update();
        app.update();
        (
            app.world().resource::<CombatSummary>().clone(),
            app.world()
                .resource::<UnitRegistry>()
                .entity_of(UnitId(0))
                .and_then(|entity| app.world().get::<StandsOn>(entity))
                .map(|standing| standing.0.pos),
        )
    };

    let first = run();
    let second = run();
    assert_eq!(first, second);
    assert_eq!(first.0.successful_commands, 0);
    assert_eq!(first.0.refused_commands, 1);
    assert_eq!(first.1, Some(position(-1, 0)));
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

#[test]
fn channel_restores_exact_state_and_spends_only_one_action() {
    let run = || {
        let shipped = CombatSettings::default();
        let profile = CombatRulesProfile::shipped(&shipped);
        let mut app = simulation_app(&profile);
        let player = spawn_unit(
            &mut app,
            UnitInput {
                id: UnitId(0),
                faction: Faction::Player,
                position: position(-1, 0),
                initiative: 20,
                controller: ControllerInput::Scripted(PlayerSeat(0)),
            },
        );
        spawn_unit(
            &mut app,
            UnitInput {
                id: UnitId(1),
                faction: Faction::Hostile,
                position: position(1, 0),
                initiative: 10,
                controller: ControllerInput::Scripted(PlayerSeat(0)),
            },
        );
        insert_depleted_channel_lattice(&mut app, player);
        app.world_mut()
            .resource_mut::<CombatTranscriptRecorder>()
            .enable();
        enter_gameplay(&mut app);
        app.update();

        let command = GameCommand::Channel { unit: UnitId(0) };
        for _ in 0..2 {
            app.world_mut()
                .resource_mut::<CommandQueue>()
                .push(IssuedCommand {
                    seat: PlayerSeat(0),
                    command: command.clone(),
                });
            app.update();
        }

        (
            app.world().resource::<CombatSummary>().clone(),
            app.world()
                .get::<LatticeState>(player)
                .map(LatticeState::total_gem_mana),
            app.world().get::<Turn>(player).map(|turn| turn.acted),
            app.world()
                .resource::<CombatTranscriptRecorder>()
                .events()
                .to_vec(),
        )
    };

    let first = run();
    let second = run();
    assert_eq!(first, second, "identical Channel cases diverged");
    assert_eq!(first.0.channels, 1);
    assert_eq!(first.0.channelled_mana.get("Fire"), Some(&2));
    assert_eq!(first.0.successful_commands, 1);
    assert_eq!(first.0.refused_commands, 1);
    assert_eq!(first.1, Some(3));
    assert_eq!(first.2, Some(true));
    assert!(first.3.iter().any(|event| matches!(
        event,
        CombatEvent::CommandRefused {
            command: GameCommand::Channel { unit: UnitId(0) },
            ..
        }
    )));
}

#[test]
fn profiles_keep_stable_named_identity() {
    let shipped = CombatSettings::default();
    let mut custom = CombatRulesProfile::custom_from(&CombatRulesProfile::shipped(&shipped));
    custom.movement_per_turn = 3;
    assert_eq!(
        [
            CombatRulesProfile::shipped(&shipped).preset,
            CombatRulesProfile::tactical_two_step(&shipped).preset,
            custom.preset,
        ],
        [
            CombatRulesPreset::Shipped,
            CombatRulesPreset::TacticalTwoStep,
            CombatRulesPreset::Custom,
        ]
    );
}
