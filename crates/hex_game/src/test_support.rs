//! Default-off observations for headless game-layer integration tests.
//!
//! The API deliberately returns immutable snapshots and formatted projections. It
//! does not expose mutable screen resources or provide a second way to apply game
//! behavior.

use bevy::prelude::*;
use hex_assets::{CombatRulesPreset, CombatSettings};
use hex_combat::{CombatSummary, EncounterResolution};
use hex_core::GameplayPhase;

use crate::combat_reports::{
    CombatLabReport, CombatLabReportHistory, CombatLabReportId, SavedCombatLabReport,
    COMBAT_LAB_REPORT_HISTORY_VERSION,
};
use crate::screens::{combat_lab, gameplay};

/// Functional post-combat report modes exposed to headless app tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

impl From<ReportMode> for gameplay::OutcomeReportMode {
    fn from(mode: ReportMode) -> Self {
        match mode {
            ReportMode::Overview => Self::Overview,
            ReportMode::Units => Self::Units,
            ReportMode::SpellsEffects => Self::SpellsEffects,
            ReportMode::Timeline => Self::Timeline,
            ReportMode::Compare => Self::Compare,
        }
    }
}

/// Projects one report mode from canonical report data.
#[must_use]
pub fn report_text(
    report: &CombatLabReport,
    mode: ReportMode,
    saved: &[SavedCombatLabReport],
    selected: Option<CombatLabReportId>,
) -> String {
    let store = combat_lab_report_store(saved);
    gameplay::outcome_report_text(report, mode.into(), Some(&store), selected)
}

fn combat_lab_report_store(
    saved: &[SavedCombatLabReport],
) -> crate::combat_reports::CombatLabReportStore {
    let next_id = saved
        .iter()
        .map(|entry| entry.id.0)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    crate::combat_reports::CombatLabReportStore {
        history: CombatLabReportHistory {
            version: COMBAT_LAB_REPORT_HISTORY_VERSION,
            next_id,
            reports: saved.to_vec(),
        },
        error: None,
    }
}

/// Projects the live drawer from the same canonical summary used by reports.
#[must_use]
pub fn live_statistics_text(summary: &CombatSummary) -> String {
    gameplay::live_statistics_label(summary)
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
    let (visible, visible_after_clear, input_survived) = combat_lab::observe_fixture_filter(query);
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
