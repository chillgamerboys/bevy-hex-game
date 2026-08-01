//! Default-off observations for headless game-layer integration tests.
//!
//! The API deliberately returns immutable snapshots and formatted projections. It
//! does not expose mutable screen resources or provide a second way to apply game
//! behavior.

use bevy::prelude::*;
use hex_assets::{CombatRulesPreset, CombatSettings};
use hex_combat::{CombatSummary, EncounterResolution};
use hex_core::{GameplayPhase, Mode, PendingDecision, Screen, TilePos, Turn, UnitId};
use hex_lattice::{LatticeSpec, LatticeState};
use hex_units::{Downed, StandsOn};

use crate::combat_reports::{
    CombatLabReport, CombatLabReportId, CurrentCombatLabReport, SavedCombatLabReport,
};
use crate::screens::{combat_lab, gameplay};

pub use hex_ui::test_support::HeadlessUiPlugin;

/// Functional post-combat report modes exposed to headless app tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReportMode {
    /// Aggregate run metrics.
    Overview,
    /// Stable roster and per-unit metrics.
    Units,
    /// Cast, effect, Channel, and disable metrics.
    SpellsEffects,
    /// Bounded canonical event detail.
    Timeline,
    /// Signed deltas against one saved report.
    Compare,
}

/// Typed report-selection observation; rendered text is deliberately absent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportSnapshot {
    /// Selected presentation mode.
    pub mode: ReportMode,
    /// Canonical identity of the current report.
    pub current_fingerprint: Option<u64>,
    /// Independently selected comparison report and its canonical identity.
    pub comparison: Option<(CombatLabReportId, Option<u64>)>,
    /// Stable current roster identities.
    pub units: Vec<UnitId>,
}

/// Observes report identity and selection without parsing rendered labels.
#[must_use]
pub fn report_snapshot(
    report: &CombatLabReport,
    mode: ReportMode,
    saved: &[SavedCombatLabReport],
    selected: Option<CombatLabReportId>,
) -> ReportSnapshot {
    let comparison = selected.and_then(|selected| {
        saved
            .iter()
            .find(|entry| entry.id == selected)
            .map(|entry| (entry.id, entry.report.fingerprint().ok()))
    });
    let mut units = report
        .rosters
        .players
        .iter()
        .chain(&report.rosters.hostiles)
        .map(|entry| UnitId(entry.unit_id))
        .collect::<Vec<_>>();
    units.sort();
    ReportSnapshot {
        mode,
        current_fingerprint: report.fingerprint().ok(),
        comparison,
        units,
    }
}

/// Typed live-drawer facts read from the canonical combat summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveStatisticsSnapshot {
    /// Completed rounds.
    pub rounds: u32,
    /// Completed turns.
    pub turns: u32,
    /// Accepted commands.
    pub successful_commands: u32,
    /// Refused commands.
    pub refused_commands: u32,
    /// Exact movement distance.
    pub movement_distance: u32,
    /// Exact movement budget consumed.
    pub movement_budget_used: u32,
    /// Successful Channel actions.
    pub channels: u32,
    /// Raw, prevented, and applied disable totals.
    pub disables: (u32, u32, u32),
    /// Mana restored by stable element name.
    pub channelled_mana: std::collections::BTreeMap<String, u32>,
    /// Stable per-unit identities.
    pub units: Vec<UnitId>,
}

/// Observes the live drawer's canonical source without parsing its rendered text.
#[must_use]
pub fn live_statistics_snapshot(summary: &CombatSummary) -> LiveStatisticsSnapshot {
    LiveStatisticsSnapshot {
        rounds: summary.rounds,
        turns: summary.turns,
        successful_commands: summary.successful_commands,
        refused_commands: summary.refused_commands,
        movement_distance: summary.movement_distance,
        movement_budget_used: summary.movement_budget_used,
        channels: summary.channels,
        disables: (
            summary.raw_disables,
            summary.prevented_disables,
            summary.applied_disables,
        ),
        channelled_mana: summary.channelled_mana.clone(),
        units: summary.units.keys().copied().collect(),
    }
}

/// Reports whether the live drawer belongs on the current gameplay surface.
#[must_use]
pub fn live_statistics_visible(
    phase: GameplayPhase,
    resolution: Option<EncounterResolution>,
) -> bool {
    gameplay::lab_statistics_should_be_visible(phase, resolution.as_ref())
}

/// Immutable result of restoring a report into Sandbox tuning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxReentrySnapshot {
    /// Stable map catalog id.
    pub map: String,
    /// Selected rules preset.
    pub preset: CombatRulesPreset,
    /// Ordered player archetypes.
    pub players: Vec<String>,
    /// Ordered hostile archetypes.
    pub hostiles: Vec<String>,
    /// Exact player deployment.
    pub player_positions: Vec<hex_core::TilePos>,
    /// Exact hostile deployment.
    pub hostile_positions: Vec<hex_core::TilePos>,
    /// Whether the handoff was consumed once.
    pub request_consumed: bool,
}

/// Restores a frozen report through the real Combat Lab initialization system.
#[must_use]
pub fn sandbox_reentry_snapshot(report: CombatLabReport) -> SandboxReentrySnapshot {
    let mut app = App::new();
    app.init_resource::<combat_lab::CombatLabState>()
        .init_resource::<combat_lab::FrozenSandboxOverlay>()
        .insert_resource(combat_lab::CombatLabSandboxRequest {
            report,
            overlay: None,
        })
        .add_systems(Update, combat_lab::initialize_lab);
    app.update();

    let state = app.world().resource::<combat_lab::CombatLabState>();
    let players = state
        .players
        .iter()
        .map(combat_lab::roster_choice_key)
        .collect();
    let hostiles = state
        .hostiles
        .iter()
        .map(combat_lab::roster_choice_key)
        .collect();
    let (player_positions, hostile_positions) = state.preserved_deployment.as_ref().map_or_else(
        || (Vec::new(), Vec::new()),
        |deployment| (deployment.players.clone(), deployment.hostiles.clone()),
    );
    SandboxReentrySnapshot {
        map: state.map.clone(),
        preset: state
            .rules
            .as_ref()
            .map_or(CombatRulesPreset::Shipped, |profile| profile.preset),
        players,
        hostiles,
        player_positions,
        hostile_positions,
        request_consumed: !app
            .world()
            .contains_resource::<combat_lab::CombatLabSandboxRequest>(),
    }
}

/// Immutable contract facts for one Wave 7 fixture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixtureSnapshot {
    /// Stable machine id.
    pub id: &'static str,
    /// Number of player units in the owned encounter.
    pub players: usize,
    /// Number of hostile units in the owned encounter.
    pub hostiles: usize,
}

/// Returns the fixture-owned Wave 7 encounter matrix.
#[must_use]
pub fn wave_seven_fixtures() -> Vec<FixtureSnapshot> {
    ["occupancy-matrix", "channel-attrition", "tempo-matrix"]
        .into_iter()
        .filter_map(|id| {
            let encounter = combat_lab::fixed_fixture_encounter(id)?;
            Some(FixtureSnapshot {
                id,
                players: encounter
                    .rosters
                    .iter()
                    .find(|roster| roster.faction == hex_assets::EncounterFaction::Player)
                    .map_or(0, |roster| roster.units.len()),
                hostiles: encounter
                    .rosters
                    .iter()
                    .find(|roster| roster.faction == hex_assets::EncounterFaction::Hostile)
                    .map_or(0, |roster| roster.units.len()),
            })
        })
        .collect()
}

/// Returns the three movement budgets admitted by the Tempo Matrix fixture.
#[must_use]
pub fn tempo_movement_matrix(shipped: &CombatSettings) -> [u32; 3] {
    [
        combat_lab::fixture_profile(combat_lab::FixtureRulesVariant::Shipped, shipped)
            .movement_per_turn,
        combat_lab::fixture_profile(combat_lab::FixtureRulesVariant::TacticalTwoStep, shipped)
            .movement_per_turn,
        combat_lab::fixture_profile(combat_lab::FixtureRulesVariant::CustomThreeStep, shipped)
            .movement_per_turn,
    ]
}

/// Immutable observation of one real fixture-filter update and clear cycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixtureFilterSnapshot {
    /// Stable fixture ids visible for the requested query.
    pub visible: Vec<String>,
    /// Stable fixture ids visible after clearing the same input entity.
    pub visible_after_clear: Vec<String>,
    /// Whether the original editable entity survived both updates.
    pub input_survived: bool,
}

/// Exercises the production fixture-filter system without exposing screen state.
#[must_use]
pub fn fixture_filter_snapshot(query: &str) -> FixtureFilterSnapshot {
    let (visible, visible_after_clear, input_survived) =
        hex_ui::test_support::combat_lab_fixture_filter_cycle(query);
    FixtureFilterSnapshot {
        visible,
        visible_after_clear,
        input_survived,
    }
}

/// Production gameplay UI registration exposed only to headless integration tests.
pub struct HeadlessGameplayUiPlugin;

impl Plugin for HeadlessGameplayUiPlugin {
    fn build(&self, app: &mut App) {
        gameplay::plugin(app);
    }
}

/// Exact turn facts for the active unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnStateSnapshot {
    /// Movement remaining in canonical hex steps.
    pub movement_left: u32,
    /// Whether the action has been consumed.
    pub acted: bool,
}

/// Exact lattice cell state observed from the owning components.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LatticeCellStateSnapshot {
    /// Stable lattice coordinate.
    pub coord: hex_core::LatticeCoord,
    /// Whether the cell is disabled.
    pub disabled: bool,
    /// Current mana, zero for non-gem cells.
    pub mana: u16,
    /// Whether an enchantment currently locks the cell.
    pub locked: bool,
}

/// Exact authoritative state for one live unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitStateSnapshot {
    /// Stable unit identity.
    pub id: UnitId,
    /// Exact occupied surface, including level.
    pub position: TilePos,
    /// Active turn budget, when this is the acting unit.
    pub turn: Option<TurnStateSnapshot>,
    /// Whether the unit is downed.
    pub downed: bool,
    /// Canonically ordered lattice cells.
    pub lattice: Vec<LatticeCellStateSnapshot>,
}

/// Immutable gameplay oracle for headless application tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameplayStateSnapshot {
    /// Current top-level screen.
    pub screen: Option<Screen>,
    /// Current gameplay lifecycle phase.
    pub phase: Option<GameplayPhase>,
    /// Current exploration/combat mode.
    pub mode: Option<Mode>,
    /// Stable initiative order.
    pub turn_order: Vec<UnitId>,
    /// Current actor named by the turn authority.
    pub acting: Option<UnitId>,
    /// Completed round count.
    pub round: u32,
    /// Open canonical decision, if any.
    pub pending: PendingDecision,
    /// Commands currently waiting at the authoritative funnel.
    pub queued_commands: usize,
    /// Application-authorized actions and their canonical refusal reasons.
    pub legal_actions: Vec<hex_ui::ActionAffordance>,
    /// Exact unit state in stable id order.
    pub units: Vec<UnitStateSnapshot>,
    /// Current encounter outcome.
    pub outcome: Option<hex_combat::EncounterOutcome>,
    /// Full saved-report fingerprint, when a Lab result exists.
    pub report_fingerprint: Option<u64>,
}

/// Reads canonical components and resources instead of inferring behavior from pixels.
#[must_use]
pub fn gameplay_state_snapshot(world: &mut World) -> GameplayStateSnapshot {
    let screen = world
        .get_resource::<State<Screen>>()
        .map(|state| *state.get());
    let phase = world.get_resource::<GameplayPhase>().copied();
    let mode = world
        .get_resource::<State<Mode>>()
        .map(|state| *state.get());
    let (turn_order, acting, round) = world.get_resource::<hex_combat::TurnOrder>().map_or_else(
        || (Vec::new(), None, 0),
        |order| (order.order().to_vec(), order.current(), order.round),
    );
    let pending = world
        .get_resource::<PendingDecision>()
        .cloned()
        .unwrap_or_default();
    let queued_commands = world
        .get_resource::<hex_core::CommandQueue>()
        .map_or(0, hex_core::CommandQueue::len);
    let legal_actions = world
        .get_resource::<hex_ui::GameplayHudView>()
        .map_or_else(Vec::new, |view| view.actions.clone());
    let outcome = world
        .get_resource::<EncounterResolution>()
        .and_then(EncounterResolution::outcome);
    let report_fingerprint = world
        .get_resource::<CurrentCombatLabReport>()
        .and_then(|report| report.0.fingerprint().ok());

    let mut query = world.query::<(
        &UnitId,
        &StandsOn,
        Option<&Turn>,
        Option<&LatticeSpec>,
        Option<&LatticeState>,
        Has<Downed>,
    )>();
    let mut units = query
        .iter(world)
        .map(|(id, standing, turn, spec, state, downed)| {
            let lattice = spec.zip(state).map_or_else(Vec::new, |(spec, state)| {
                let mut cells = spec
                    .cells()
                    .map(|(coord, _)| LatticeCellStateSnapshot {
                        coord,
                        disabled: state.is_disabled(coord),
                        mana: state.mana(coord),
                        locked: state.is_locked(coord),
                    })
                    .collect::<Vec<_>>();
                cells.sort_by_key(|cell| cell.coord);
                cells
            });
            UnitStateSnapshot {
                id: *id,
                position: standing.0.pos,
                turn: turn.map(|turn| TurnStateSnapshot {
                    movement_left: turn.movement_left,
                    acted: turn.acted,
                }),
                downed,
                lattice,
            }
        })
        .collect::<Vec<_>>();
    units.sort_by_key(|unit| unit.id);
    GameplayStateSnapshot {
        screen,
        phase,
        mode,
        turn_order,
        acting,
        round,
        pending,
        queued_commands,
        legal_actions,
        units,
        outcome,
        report_fingerprint,
    }
}
