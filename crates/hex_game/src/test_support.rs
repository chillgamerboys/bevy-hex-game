//! Default-off observations for headless game-layer integration tests.
//!
//! The API deliberately returns immutable snapshots and formatted projections. It
//! does not expose mutable screen resources or provide a second way to apply game
//! behavior.

use bevy::prelude::*;
use hex_assets::{
    CombatRulesPreset, CombatRulesProfile, CombatSettings, ElementCatalog, ElementFile, SpellBook,
};
use hex_combat::{CombatSummary, EncounterResolution, FactionLatticeKnowledge, TurnOrder};
use hex_core::{
    ElementId, GameplayPhase, LatticeCoord, Mode, PendingDecision, Screen, TilePos, Turn, UnitId,
};
use hex_lattice::{CellKind, LatticeSpec, LatticeState, LatticeStats};
use hex_units::{Downed, Faction, Party, Player, Selected, StandsOn, UnitRegistry};

use crate::casting::{Aiming, CastReadout};
use crate::combat_reports::{
    CombatLabReport, CombatLabReportHistory, CombatLabReportId, CombatLabReportMap,
    CombatLabReportStore, CurrentCombatLabReport, SavedCombatLabReport,
};
use crate::readouts::{
    refresh_lattice_readouts, refresh_ui_context, DisableSelection, GameplayUiContext,
    RetainedTarget,
};
use crate::screens::{combat_lab, gameplay};

pub use hex_gameplay_model::ReportMode;
pub use hex_ui::test_support::HeadlessUiPlugin;

/// Typed report-selection observation; rendered text is deliberately absent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportSnapshot {
    /// Report mode projected by the application adapter after settling.
    pub mode: ReportMode,
    /// Canonical identity read from the retained current-report resource.
    pub current_fingerprint: u64,
    /// Comparison selected in the immutable presentation projection and the
    /// matching canonical saved-report identity.
    pub comparison: Option<(CombatLabReportId, u64)>,
    /// Stable current roster identities read from the retained report.
    pub units: Vec<UnitId>,
}

/// Bounded before/after observations from one real application transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportTransitionSnapshot {
    /// Initial Overview followed by each typed report-mode transition.
    pub modes: Vec<ReportSnapshot>,
    /// Observation after leaving Compare; its comparison identity must persist.
    pub after_compare: ReportSnapshot,
}

/// Drives typed report intents through the shipping application adapter.
///
/// The mode sequence is intentionally owned here instead of supplied by the
/// caller: each output mode comes from `ReportViewModel` through
/// `sync_outcome_report_view`. Current and comparison fingerprints are read from
/// canonical retained-report resources and joined to the view's typed selected
/// id. No rendered text or caller-provided output projection is consulted.
pub fn report_transition_snapshot(
    report: &CombatLabReport,
    saved: &[SavedCombatLabReport],
    compare_with: CombatLabReportId,
) -> Result<ReportTransitionSnapshot, String> {
    let outcome = report
        .outcome
        .ok_or_else(|| "report transition requires a completed encounter".to_owned())?;
    let shipped = CombatSettings::default();
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    gameplay::install_outcome_report_test_adapter(&mut app);

    let history = CombatLabReportHistory {
        next_id: saved
            .iter()
            .map(|entry| entry.id.0)
            .max()
            .unwrap_or(0)
            .saturating_add(1),
        reports: saved.to_vec(),
        ..default()
    };
    app.insert_resource(EncounterResolution(Some(outcome)))
        .insert_resource(CurrentCombatLabReport(report.clone()))
        .insert_resource(CombatLabReportStore {
            history,
            error: None,
        })
        .insert_resource(combat_lab::CombatLabSession {
            kind: combat_lab::CombatLabSessionKind::Sandbox,
            return_to: Screen::CombatLab,
            profile: report.profile.clone(),
            shipped_combat: shipped,
            report_map: report.map.clone(),
            initial_state: None,
        });

    app.update();
    let mut modes = vec![observe_report_presentation(app.world())?];
    for mode in [
        ReportMode::Units,
        ReportMode::SpellsEffects,
        ReportMode::Timeline,
    ] {
        app.world_mut().write_message(hex_ui::UiIntent::Outcome(
            hex_ui::OutcomeIntent::SelectMode(mode),
        ));
        app.update();
        modes.push(observe_report_presentation(app.world())?);
    }

    app.world_mut().write_message(hex_ui::UiIntent::Outcome(
        hex_ui::OutcomeIntent::CompareWith(compare_with),
    ));
    app.update();
    modes.push(observe_report_presentation(app.world())?);

    app.world_mut().write_message(hex_ui::UiIntent::Outcome(
        hex_ui::OutcomeIntent::SelectMode(ReportMode::Overview),
    ));
    app.update();
    let after_compare = observe_report_presentation(app.world())?;

    Ok(ReportTransitionSnapshot {
        modes,
        after_compare,
    })
}

fn observe_report_presentation(world: &World) -> Result<ReportSnapshot, String> {
    let view = world
        .get_resource::<hex_ui::OutcomeReportView>()
        .ok_or_else(|| "outcome report presentation was not published".to_owned())?;
    if !view.visible {
        return Err("outcome report presentation remained hidden".to_owned());
    }
    let current = world
        .get_resource::<CurrentCombatLabReport>()
        .ok_or_else(|| "canonical current report is missing".to_owned())?;
    let store = world
        .get_resource::<CombatLabReportStore>()
        .ok_or_else(|| "canonical saved report store is missing".to_owned())?;

    let selected = view
        .comparisons
        .iter()
        .filter(|choice| choice.selected)
        .map(|choice| choice.id)
        .collect::<Vec<_>>();
    if selected.len() > 1 {
        return Err("report presentation selected more than one comparison".to_owned());
    }
    let comparison = selected.first().copied().map(|id| {
        let entry = store
            .history
            .reports
            .iter()
            .find(|entry| entry.id == id)
            .ok_or_else(|| format!("selected report {} is absent from canonical history", id.0))?;
        Ok::<_, String>((id, entry.report.fingerprint()?))
    });
    let comparison = comparison.transpose()?;

    let mut units = current
        .0
        .rosters
        .players
        .iter()
        .chain(&current.0.rosters.hostiles)
        .map(|entry| UnitId(entry.unit_id))
        .collect::<Vec<_>>();
    units.sort();
    Ok(ReportSnapshot {
        mode: view.mode,
        current_fingerprint: current.0.fingerprint()?,
        comparison,
        units,
    })
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

/// Immutable before/after observations of the shipping lattice/statistics
/// adapters and Lab-statistics intent path.
#[derive(Debug, Clone, PartialEq)]
pub struct ProductionLabUiTransitionSnapshot {
    /// UI tree before the typed toggle.
    pub collapsed: hex_ui::test_support::UiTreeSnapshot,
    /// UI tree after the typed toggle.
    pub expanded: hex_ui::test_support::UiTreeSnapshot,
    /// Final application-owned expansion state.
    pub expanded_state: bool,
    /// Cells emitted for the player by the production lattice publisher.
    pub published_own_lattice_cells: usize,
}

/// Drives the real Lab-statistics toggle at the reported Retina client size.
///
/// This deliberately installs only the production context, lattice, and
/// statistics adapter systems needed by the transition. The player lattice comes
/// from canonical ECS components and the unit registry; no authored UI fixture or
/// review override supplies it. Installing the full gameplay plugin would
/// fabricate unrelated combat, storage, and world authority in this focused
/// adapter contract.
#[must_use]
pub fn production_lab_ui_transition_snapshot() -> ProductionLabUiTransitionSnapshot {
    let mut app = App::new();
    app.add_plugins(HeadlessUiPlugin::with_scale_factor(2582, 1442, 2.0))
        .init_resource::<TurnOrder>()
        .init_resource::<PendingDecision>()
        .init_resource::<UnitRegistry>()
        .init_resource::<Party>()
        .init_resource::<CastReadout>()
        .init_resource::<Aiming>()
        .init_resource::<RetainedTarget>()
        .init_resource::<DisableSelection>()
        .init_resource::<GameplayUiContext>()
        .init_resource::<FactionLatticeKnowledge>()
        .add_systems(
            Update,
            (
                refresh_ui_context,
                refresh_lattice_readouts,
                gameplay::toggle_lab_statistics_from_intents,
                gameplay::publish_lab_statistics_view,
            )
                .chain()
                .after(hex_ui::UiSystems::EmitIntents)
                .before(hex_ui::UiSystems::Render)
                .run_if(in_state(Screen::Gameplay)),
        );

    let shipped = CombatSettings::default();
    app.world_mut()
        .insert_resource(hex_ui::UiScalePreference(hex_ui::UiScaleMode::Auto));
    app.world_mut()
        .insert_resource(hex_ui::GameplayChromeView::default());
    app.world_mut().insert_resource(hex_ui::GameplayHudView {
        phase: GameplayPhase::Active,
        actor: Some(UnitId(0)),
        actor_label: "Hedge Mage · Player".to_owned(),
        round: "Round 4".to_owned(),
        movement_remaining: 2,
        action_remaining: true,
        required_prompt: None,
        actions: vec![
            hex_ui::ActionAffordance {
                action: hex_ui::GameplayAction::Channel,
                label: "Channel".to_owned(),
                shortcut: None,
                availability: hex_ui::ActionAvailability::Enabled,
                priority: hex_ui::ActionPriority::Primary,
            },
            hex_ui::ActionAffordance {
                action: hex_ui::GameplayAction::EndTurn,
                label: "End turn".to_owned(),
                shortcut: Some("Space".to_owned()),
                availability: hex_ui::ActionAvailability::Enabled,
                priority: hex_ui::ActionPriority::Primary,
            },
        ],
    });

    let elements = ElementCatalog::from_file(&ElementFile {
        wheel: vec!["Ember".to_owned(), "Tide".to_owned()],
        fusions: default(),
    });
    app.world_mut().insert_resource(elements);
    app.world_mut().insert_resource(SpellBook::default());

    let unit = UnitId(0);
    let stats = LatticeStats::new(
        std::collections::BTreeMap::from([(ElementId(0), 3)]),
        std::collections::BTreeMap::from([(ElementId(0), 1)]),
    );
    let spec = [
        LatticeCoord::ORIGIN,
        LatticeCoord::new(1, 0),
        LatticeCoord::new(1, -1),
        LatticeCoord::new(0, -1),
        LatticeCoord::new(-1, 0),
        LatticeCoord::new(-1, 1),
        LatticeCoord::new(0, 1),
    ]
    .into_iter()
    .fold(LatticeSpec::default(), |spec, coord| {
        spec.with(
            coord,
            CellKind::Gem {
                element: ElementId(0),
            },
        )
    });
    let state = LatticeState::new(&spec, &stats);
    let unit_entity = app
        .world_mut()
        .spawn((
            Player,
            Selected,
            unit,
            Name::new("Hedge Mage"),
            Faction::Player,
            spec,
            state,
            stats,
        ))
        .id();
    app.world_mut()
        .resource_mut::<UnitRegistry>()
        .register(unit, unit_entity);
    app.world_mut().resource_mut::<Party>().members.push(unit);

    app.world_mut().insert_resource(GameplayPhase::Active);
    let mut summary = CombatSummary::default();
    summary.rounds = 4;
    summary.turns = 11;
    summary.successful_commands = 27;
    summary.refused_commands = 3;
    app.world_mut().insert_resource(summary);
    app.world_mut()
        .insert_resource(combat_lab::CombatLabSession {
            kind: combat_lab::CombatLabSessionKind::Sandbox,
            return_to: Screen::CombatLab,
            profile: CombatRulesProfile::shipped(&shipped),
            shipped_combat: shipped,
            report_map: CombatLabReportMap {
                catalog_id: "flat-arena".to_owned(),
                scenario: "Production UI contract".to_owned(),
                resolved_seed: None,
            },
            initial_state: None,
        });
    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Gameplay);
    for _ in 0..8 {
        app.update();
    }
    let published_own_lattice_cells = app
        .world()
        .resource::<hex_ui::GameplayLatticesView>()
        .own
        .as_ref()
        .map_or(0, |lattice| lattice.cells.len());
    let collapsed = hex_ui::test_support::ui_tree_snapshot(app.world_mut());

    app.world_mut()
        .write_message(hex_ui::UiIntent::LabStatistics(
            hex_ui::LabStatisticsIntent::Toggle,
        ));
    for _ in 0..8 {
        app.update();
    }
    let expanded = hex_ui::test_support::ui_tree_snapshot(app.world_mut());
    let expanded_state = app.world().resource::<hex_ui::LabStatisticsView>().expanded;
    ProductionLabUiTransitionSnapshot {
        collapsed,
        expanded,
        expanded_state,
        published_own_lattice_cells,
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

/// Immutable gameplay observation for headless application tests.
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
    /// Action affordances currently projected to presentation.
    ///
    /// These are useful for adapter-parity assertions, but are not a canonical
    /// legality oracle. Command authority remains in the owning gameplay systems.
    pub presented_actions: Vec<hex_ui::ActionAffordance>,
    /// Exact unit state in stable id order.
    pub units: Vec<UnitStateSnapshot>,
    /// Current encounter outcome.
    pub outcome: Option<hex_combat::EncounterOutcome>,
    /// Full saved-report fingerprint, when a Lab result exists.
    pub report_fingerprint: Option<u64>,
}

/// Reads authority resources/components and explicitly named presentation projections.
///
/// No value is inferred from rendered text or pixels.
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
    let presented_actions = world
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
        presented_actions,
        units,
        outcome,
        report_fingerprint,
    }
}
