//! The game itself.
//!
//! Owns the pause toggle, the route back to the title screen, and the HUD. Terrain,
//! environment, and the units are spawned by `hex_map`, `hex_world`, and `hex_units`
//! on `OnEnter(Screen::Gameplay)`; this module deliberately does not reach into any
//! of them. It reads `Mode` and `TurnOrder` to describe what is happening, and
//! writes neither.
//!
//! Casting is [`crate::casting`]'s — the spell panel, the shape preview and the cast
//! command all live there. This module used to carry a placeholder that cast the first
//! damaging spell at the nearest hostile on `1`, so the damage loop could be played
//! before an interface for it existed; nothing here emits a command any more.

use bevy::prelude::*;
use bevy::ui_widgets::ScrollArea;
use hex_assets::{CombatSettings, FormationCatalog};
use hex_combat::{
    CombatSummary, EncounterOutcome, EncounterResolution, Turn, TurnOrder, UnitCombatSummary,
};
use hex_core::{
    CommandQueue, ControlOwner, GameCommand, GameplayPhase, GameplaySystems, InputAction,
    InputBindings, IssuedCommand, Mode, PartyFormation, PartyMovementMode, Pause, PendingDecision,
    Screen, UnitId,
};
use hex_lattice::{LatticeSpec, LatticeState};
use hex_units::{Archetype, Downed, Party, Player, Selected, UnitRegistry};

#[cfg(test)]
use hex_core::HexCoord;

use super::combat_lab::{
    CombatLabReportLaunch, CombatLabSandboxRequest, CombatLabSession, CreatorContentOverlay,
    CreatorDisplayName,
};
use super::{despawn_screen, DespawnOnExit};
use crate::combat_reports::{
    CombatLabReport, CombatLabReportStore, CombatLabReportTermination, CurrentCombatLabReport,
};
use crate::readouts::{GameplayUiContext, UiUnitIdentity};
use crate::scenarios::ActiveScenario;
use crate::storage::StoragePaths;
use hex_ui::{
    blurb, fine, heading, row_button, ActionAffordance, ActionAvailability, ActionPriority,
    GameplayAction, GameplayHudView, UiAssets,
};

pub(crate) fn plugin(app: &mut App) {
    app.init_resource::<InputBindings>();
    app.init_resource::<OutcomeReportState>();
    app.add_sub_state::<Pause>();
    app.register_type::<Pause>();
    // A second sub-state of `Screen::Gameplay`, independent of `Pause`. Both are
    // computed from the screen, so pausing does not destroy the mode.
    app.add_sub_state::<Mode>();
    app.register_type::<Mode>();

    app.add_systems(
        Update,
        handle_input
            .run_if(in_state(Screen::Gameplay))
            .run_if(resource_equals(GameplayPhase::Active))
            .run_if(hex_combat::encounter_unresolved),
    );
    app.add_systems(
        Update,
        publish_hud_view
            .after(GameplaySystems::UiContext)
            .run_if(in_state(Screen::Gameplay)),
    );
    app.add_systems(
        Update,
        handle_gameplay_ui_intents
            .after(publish_hud_view)
            .after(hex_ui::UiSystems::EmitIntents)
            .run_if(in_state(Screen::Gameplay))
            .run_if(resource_equals(GameplayPhase::Active)),
    );
    app.add_systems(
        Update,
        (
            handle_party_strip.after(hex_ui::UiSystems::EmitIntents),
            publish_party_view,
        )
            .chain()
            .run_if(in_state(Screen::Gameplay))
            .run_if(resource_equals(GameplayPhase::Active)),
    );
    app.add_systems(
        Update,
        (
            handle_outcome_report_controls,
            sync_outcome_modal,
            update_outcome_report,
            handle_outcome_actions,
        )
            .chain()
            .run_if(in_state(Screen::Gameplay))
            .run_if(resource_equals(GameplayPhase::Active)),
    );
    app.add_systems(
        Update,
        (
            handle_lab_statistics_intents.after(hex_ui::UiSystems::EmitIntents),
            publish_lab_statistics_view,
        )
            .chain()
            .run_if(in_state(Screen::Gameplay))
            .run_if(resource_equals(GameplayPhase::Active)),
    );
    // Pausable, because the system that acts on the flag is. `mirror_truth` runs in
    // `PausableSystems`, so a toggle that kept firing while paused would set the
    // resource with nothing to carry it out — leaving the store holding a full reveal
    // the flag says is off, which is the stale-and-authoritative state its own doc
    // calls worse than no reveal at all.
    #[cfg(feature = "dev")]
    app.add_systems(
        Update,
        toggle_reveal_all
            .in_set(hex_core::PausableSystems)
            .run_if(in_state(Screen::Gameplay)),
    );
    app.add_systems(
        OnEnter(Screen::Gameplay),
        (
            reset_pause,
            reset_mode,
            reset_outcome_report,
            reset_lab_statistics_view,
        ),
    );
    app.add_systems(OnExit(Screen::Gameplay), despawn_screen(Screen::Gameplay));
}

fn handle_gameplay_ui_intents(
    mut intents: MessageReader<hex_ui::UiIntent>,
    mode: Res<State<Mode>>,
    order: Res<TurnOrder>,
    pending: Res<PendingDecision>,
    registry: Res<UnitRegistry>,
    owners: Query<(Option<&ControlOwner>, &hex_units::Faction)>,
    mut queue: ResMut<CommandQueue>,
    mut next_pause: ResMut<NextState<Pause>>,
) {
    for intent in intents.read() {
        let hex_ui::UiIntent::Gameplay(action) = intent else {
            continue;
        };
        match action {
            GameplayAction::Channel => crate::casting::panel::queue_current_player_command(
                true,
                &order,
                &pending,
                &registry,
                &owners,
                &mut queue,
                |unit| GameCommand::Channel { unit },
            ),
            GameplayAction::EndTurn => crate::casting::panel::queue_current_player_command(
                true,
                &order,
                &pending,
                &registry,
                &owners,
                &mut queue,
                |unit| GameCommand::EndTurn { unit },
            ),
            GameplayAction::Pause => next_pause.set(Pause(true)),
            GameplayAction::Rest if *mode.get() == Mode::Exploring => {
                let player = registry.iter().find_map(|(unit, entity)| {
                    owners.get(entity).ok().and_then(|(owner, faction)| {
                        (*faction == hex_units::Faction::Player)
                            .then(|| (unit, owner.copied().unwrap_or_default().0))
                    })
                });
                if let Some((unit, seat)) = player {
                    queue.push(IssuedCommand {
                        seat,
                        command: GameCommand::Rest { unit },
                    });
                }
            }
            GameplayAction::Rest | GameplayAction::ConfirmDecision => {}
        }
    }
}

#[derive(Component)]
struct OutcomeModal;

pub(crate) use hex_gameplay_model::ReportMode as OutcomeReportMode;
use hex_gameplay_model::{
    resolve_lab_run, LabRunAction, LabRunFailure, LabRunTransition, ReportViewModel,
};

type OutcomeReportState = ReportViewModel<crate::combat_reports::CombatLabReportId>;

#[derive(Component, Debug, Clone, Copy)]
enum OutcomeReportControl {
    Mode(OutcomeReportMode),
    CompareWith(crate::combat_reports::CombatLabReportId),
}

#[derive(Component)]
struct OutcomeReportTab(OutcomeReportMode);

#[derive(Component)]
struct OutcomeReportBody;

#[derive(Component)]
struct OutcomeCompareControls;

#[derive(Component, Clone, Copy)]
enum OutcomeAction {
    Continue,
    Retry,
    RetryExact,
    TuneAgain,
    CopyToSandbox,
    SaveReport,
    ReturnTitle,
}

fn reset_outcome_report(mut state: ResMut<OutcomeReportState>) {
    *state = OutcomeReportState::default();
}

fn reset_lab_statistics_view(mut view: ResMut<hex_ui::LabStatisticsView>) {
    *view = hex_ui::LabStatisticsView::default();
}

#[expect(
    clippy::too_many_arguments,
    reason = "manual stop freezes the same independent launch facts as outcome reporting"
)]
fn handle_lab_statistics_intents(
    mut commands: Commands,
    mut intents: MessageReader<hex_ui::UiIntent>,
    mut view: ResMut<hex_ui::LabStatisticsView>,
    lab: Option<Res<CombatLabSession>>,
    launch: Option<Res<CombatLabReportLaunch>>,
    summary: Option<Res<CombatSummary>>,
    shipped: Option<Res<CombatSettings>>,
    paths: Res<StoragePaths>,
    mut reports: ResMut<CombatLabReportStore>,
    mut next_screen: ResMut<NextState<Screen>>,
) {
    let mut end_experiment = false;
    for intent in intents.read() {
        match intent {
            hex_ui::UiIntent::LabStatistics(hex_ui::LabStatisticsIntent::Toggle) => {
                view.expanded = !view.expanded;
            }
            hex_ui::UiIntent::LabStatistics(hex_ui::LabStatisticsIntent::EndExperiment) => {
                end_experiment = true;
            }
            _ => {}
        }
    }
    if !end_experiment {
        return;
    }
    let (Some(lab), Some(launch), Some(summary), Some(shipped)) = (
        lab.as_deref(),
        launch.as_deref(),
        summary.as_deref(),
        shipped.as_deref(),
    ) else {
        error!("Combat Lab manual stop is missing frozen launch or summary facts");
        return;
    };
    let report = match CombatLabReport::new(
        lab.profile.clone(),
        launch.origin.clone(),
        launch.map.clone(),
        launch.content_revision,
        launch.rosters.clone(),
        launch.deployment.clone(),
        CombatLabReportTermination::ManualStop,
        summary.clone(),
    ) {
        Ok(report) => report,
        Err(error) => {
            reports.error = Some(format!("manual-stop report failed closed: {error}"));
            return;
        }
    };
    commands.insert_resource(CurrentCombatLabReport(report.clone()));
    if let Err(error) = reports.save(report, shipped, &paths) {
        reports.error = Some(error);
        return;
    }
    next_screen.set(Screen::CombatLab);
}

pub(crate) fn lab_statistics_should_be_visible(
    phase: GameplayPhase,
    resolution: Option<&EncounterResolution>,
) -> bool {
    phase == GameplayPhase::Active && resolution.is_none_or(|resolution| resolution.0.is_none())
}

fn publish_lab_statistics_view(
    phase: Res<GameplayPhase>,
    resolution: Option<Res<EncounterResolution>>,
    lab: Option<Res<CombatLabSession>>,
    summary: Option<Res<CombatSummary>>,
    mut view: ResMut<hex_ui::LabStatisticsView>,
) {
    let next = hex_ui::LabStatisticsView {
        present: lab.is_some(),
        visible: lab.is_some() && lab_statistics_should_be_visible(*phase, resolution.as_deref()),
        expanded: view.expanded,
        text: summary.as_deref().map_or_else(
            || "Waiting for canonical combat statistics…".to_owned(),
            live_statistics_label,
        ),
    };
    if *view != next {
        *view = next;
    }
}

pub(crate) fn live_statistics_label(summary: &CombatSummary) -> String {
    let mana = if summary.channelled_mana.is_empty() {
        "none".to_owned()
    } else {
        summary
            .channelled_mana
            .iter()
            .map(|(element, amount)| format!("{element} {amount}"))
            .collect::<Vec<_>>()
            .join(" · ")
    };
    let outcome = summary
        .outcome
        .map_or("IN PROGRESS", |outcome| match outcome {
            EncounterOutcome::Victory => "VICTORY",
            EncounterOutcome::Defeat => "DEFEAT",
        });
    let units = summary
        .units
        .iter()
        .map(|(unit, summary)| format_unit_statistics(*unit, summary))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "Round {} · Turns {} · Outcome {outcome} · No-progress {}/{} current/max\n\
         Commands {} successful / {} refused · AI choices {}\n\
         Move {} actions · {} distance / {} budget used\n\
         Casts {} · Channel {} · Strikes {} · Idle turns {}\n\
         Disables {} raw / {} prevented / {} applied\n\
         Restored {} · Downed {} · Revived {}\n\
         Mana restored · {mana}\n\
         PER UNIT\n{units}",
        summary.rounds,
        summary.turns,
        summary.no_progress_current,
        summary.no_progress_max,
        summary.successful_commands,
        summary.refused_commands,
        summary.ai_selection_count,
        summary.moves,
        summary.movement_distance,
        summary.movement_budget_used,
        summary.casts,
        summary.channels,
        summary.strikes,
        summary.idle_turns,
        summary.raw_disables,
        summary.prevented_disables,
        summary.applied_disables,
        summary.restored_cells,
        summary.downings,
        summary.revivals,
    )
}

fn format_unit_statistics(unit: UnitId, summary: &UnitCombatSummary) -> String {
    let mana = summary
        .channelled_mana
        .iter()
        .map(|(element, amount)| format!("{element} {amount}"))
        .collect::<Vec<_>>()
        .join("/");
    format!(
        "#{} turns {} · no-progress {}/{} · cmd {}/{} · move {}/{} · cast {} · channel {} [{}] · strike {} · disable {}/{}/{} · restore {} · down/revive {}/{} · idle {} · AI {}",
        unit.0,
        summary.turns,
        summary.no_progress_current,
        summary.no_progress_max,
        summary.successful_commands,
        summary.refused_commands,
        summary.movement_distance,
        summary.movement_budget_used,
        summary.casts_by_spell.values().sum::<u32>(),
        summary.channels,
        if mana.is_empty() { "none" } else { &mana },
        summary.strikes,
        summary.raw_disables,
        summary.prevented_disables,
        summary.applied_disables,
        summary.restored_cells,
        summary.downings,
        summary.revivals,
        summary.idle_turns,
        summary.ai_choices,
    )
}

fn handle_party_strip(
    mut commands: Commands,
    mode: Res<State<Mode>>,
    party: Res<Party>,
    registry: Res<UnitRegistry>,
    formations: Res<FormationCatalog>,
    mut formation: ResMut<PartyFormation>,
    mut intents: MessageReader<hex_ui::UiIntent>,
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<InputBindings>,
    mut queue: ResMut<CommandQueue>,
    selected_units: Query<(Entity, &UnitId), (With<Player>, With<Selected>)>,
    owners: Query<&ControlOwner>,
) {
    if *mode.get() != Mode::Exploring {
        return;
    }
    let selected = selected_units.iter().next().map(|(_, unit)| *unit);
    let mut rest_requested = bindings.just_pressed(&keys, InputAction::Rest);
    for intent in intents.read() {
        let hex_ui::UiIntent::Party(intent) = intent else {
            continue;
        };
        match intent {
            hex_ui::PartyIntent::SelectMember(slot) => {
                if let Some(entity) = party
                    .members
                    .get(*slot)
                    .and_then(|unit| registry.entity_of(*unit))
                {
                    for (old, _) in &selected_units {
                        if old != entity {
                            commands.entity(old).remove::<Selected>();
                        }
                    }
                    commands.entity(entity).insert(Selected);
                }
            }
            hex_ui::PartyIntent::ToggleMovementMode => {
                formation.mode = match formation.mode {
                    PartyMovementMode::Group => PartyMovementMode::Solo,
                    PartyMovementMode::Solo => PartyMovementMode::Group,
                };
            }
            hex_ui::PartyIntent::SelectPreset(name) => {
                if let Some(preset) = formations.get(name) {
                    formation.select_preset(preset, &party.members);
                }
            }
            hex_ui::PartyIntent::AssignSlot(offset) => {
                let Some(member) = selected else { continue };
                let Some(preset) = formations.get(&formation.preset) else {
                    continue;
                };
                if preset.slots.iter().any(|slot| slot.offset == *offset) {
                    let _ = formation.assign(member, *offset);
                    formation.fill_unassigned(preset, &party.members);
                }
            }
            hex_ui::PartyIntent::Rest => rest_requested = true,
        }
    }
    if rest_requested {
        let issuer = selected
            .or_else(|| party.members.first().copied())
            .filter(|unit| registry.entity_of(*unit).is_some());
        if let Some(unit) = issuer {
            let seat = registry
                .entity_of(unit)
                .and_then(|entity| owners.get(entity).ok())
                .copied()
                .unwrap_or_default()
                .0;
            queue.push(IssuedCommand {
                seat,
                command: GameCommand::Rest { unit },
            });
        }
    }
}

fn publish_party_view(
    mode: Res<State<Mode>>,
    context: Res<GameplayUiContext>,
    party: Res<Party>,
    registry: Res<UnitRegistry>,
    formations: Res<FormationCatalog>,
    formation: Res<PartyFormation>,
    units: Query<(
        &UnitId,
        &Archetype,
        Option<&CreatorDisplayName>,
        Option<&LatticeSpec>,
        Option<&LatticeState>,
        Has<Downed>,
        Has<Selected>,
    )>,
    mut view: ResMut<hex_ui::PartyView>,
) {
    let anchor = formations
        .get(&formation.preset)
        .and_then(|preset| formation.anchor_member(preset));
    let members = party
        .members
        .iter()
        .enumerate()
        .filter_map(|(slot, member)| {
            let entity = registry.entity_of(*member)?;
            let (id, archetype, display_name, spec, state, downed, selected) =
                units.get(entity).ok()?;
            let condition = spec.zip(state).map_or_else(String::new, |(spec, state)| {
                let total = spec.cells().count();
                let live = spec
                    .cells()
                    .filter(|(coord, _)| !state.is_disabled(*coord))
                    .count();
                format!("{live}/{total}")
            });
            let active = context
                .acting
                .as_ref()
                .is_some_and(|unit| unit.unit == *id && unit.faction == hex_units::Faction::Player);
            Some(hex_ui::PartyMemberView {
                slot,
                label: format!(
                    "{}ALLY {} · {} #{} · {}{}{}",
                    if active { "▶ " } else { "" },
                    slot + 1,
                    display_name.map_or(archetype.0.as_str(), |name| name.0.as_str()),
                    id.0,
                    condition,
                    if downed { " · DOWN" } else { "" },
                    if anchor == Some(*id) {
                        " · ANCHOR ◆"
                    } else {
                        ""
                    }
                ),
                active,
                selected,
            })
        })
        .collect();
    let active_preset = formations.get(&formation.preset);
    let next = hex_ui::PartyView {
        members,
        formation_visible: *mode.get() == Mode::Exploring,
        movement_mode: format!("{:?} MOVEMENT", formation.mode).to_uppercase(),
        presets: formations
            .presets
            .iter()
            .map(|preset| preset.name.clone())
            .collect(),
        slots: active_preset
            .into_iter()
            .flat_map(|preset| preset.slots.iter())
            .map(|slot| hex_ui::FormationSlotView {
                offset: slot.offset,
                anchor: slot.anchor,
            })
            .collect(),
    };
    if *view != next {
        *view = next;
    }
}
fn sync_outcome_modal(
    mut commands: Commands,
    resolution: Res<EncounterResolution>,
    existing: Query<Entity, With<OutcomeModal>>,
    assets: Res<UiAssets>,
    lab: Option<Res<CombatLabSession>>,
    launch: Option<Res<CombatLabReportLaunch>>,
    summary: Option<Res<CombatSummary>>,
    reports: Option<Res<CombatLabReportStore>>,
    report_state: Res<OutcomeReportState>,
) {
    let Some(outcome) = resolution.outcome() else {
        for entity in &existing {
            commands.entity(entity).despawn();
        }
        return;
    };
    if !existing.is_empty() {
        return;
    }
    let report = lab
        .as_deref()
        .zip(launch.as_deref())
        .zip(summary.as_deref())
        .and_then(|((lab, launch), summary)| {
            match CombatLabReport::new(
                lab.profile.clone(),
                launch.origin.clone(),
                launch.map.clone(),
                launch.content_revision,
                launch.rosters.clone(),
                launch.deployment.clone(),
                CombatLabReportTermination::Outcome(outcome),
                summary.clone(),
            ) {
                Ok(report) => Some(report),
                Err(error) => {
                    error!("Combat Lab report evidence failed closed: {error}");
                    None
                }
            }
        });
    if let Some(report) = &report {
        commands.insert_resource(CurrentCombatLabReport(report.clone()));
    }
    let return_label = lab
        .as_deref()
        .map(|session| match session.return_to {
            Screen::CharacterCreator => "Return to Creator",
            Screen::SpellCreator => "Return to Spell Creator",
            Screen::CombatLab => "Return to Combat Lab",
            _ => "Return to Title",
        })
        .unwrap_or("Return to Title");
    commands
        .spawn((
            Name::new("Encounter Outcome Modal"),
            OutcomeModal,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.62)),
            GlobalZIndex(20),
            Pickable::default(),
            DespawnOnExit(Screen::Gameplay),
        ))
        .with_children(|overlay| {
            overlay
                .spawn((
                    Node {
                        width: if report.is_some() {
                            Val::Percent(88.0)
                        } else {
                            Val::Px(430.0)
                        },
                        max_width: Val::Px(1500.0),
                        max_height: Val::Percent(90.0),
                        padding: UiRect::all(Val::Px(28.0)),
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        row_gap: Val::Px(16.0),
                        border: UiRect::all(Val::Px(1.0)),
                        border_radius: BorderRadius::all(Val::Px(10.0)),
                        ..default()
                    },
                    BorderColor::all(Color::srgba(0.93, 0.79, 0.46, 0.5)),
                    BackgroundColor(Color::srgba(0.02, 0.03, 0.045, 0.97)),
                ))
                .with_children(|panel| {
                    let (title, detail) = match outcome {
                        EncounterOutcome::Victory => {
                            ("Victory", "The battlefield remains as the encounter ended.")
                        }
                        EncounterOutcome::Defeat => (
                            "Defeat",
                            "Retry replays this scenario with the same resolved seed.",
                        ),
                    };
                    panel.spawn(heading(&assets, title));
                    panel.spawn(blurb(&assets, detail));
                    if let (Some(report), Some(lab)) = (&report, lab.as_deref()) {
                        let changes = report
                            .profile
                            .changed_from_shipped(&lab.shipped_combat);
                        panel.spawn(fine(
                            &assets,
                            format!(
                                "{:?} · {} · seed {} · Player {} / Hostile {} · {} rule change{} · fingerprint {:016X}",
                                report.profile.preset,
                                report.map.scenario,
                                report
                                    .map
                                    .resolved_seed
                                    .map_or_else(|| "authored".to_owned(), |seed| seed.to_string()),
                                report.rosters.players.len(),
                                report.rosters.hostiles.len(),
                                changes.len(),
                                if changes.len() == 1 { "" } else { "s" },
                                report.summary_fingerprint,
                            ),
                        ));
                        panel
                            .spawn(Node {
                                flex_direction: FlexDirection::Row,
                                column_gap: Val::Px(7.0),
                                ..default()
                            })
                            .with_children(|tabs| {
                                for (mode, label) in OutcomeReportMode::ALL {
                                    let text = if report_state.mode == mode {
                                        format!("{label} · ACTIVE")
                                    } else {
                                        label.to_owned()
                                    };
                                    tabs.spawn((
                                        row_button(label, 155.0),
                                        OutcomeReportControl::Mode(mode),
                                        OutcomeReportTab(mode),
                                    ))
                                    .with_child(blurb(&assets, text));
                                }
                            });
                        panel
                            .spawn((
                                ScrollArea,
                                Node {
                                    width: Val::Percent(100.0),
                                    min_height: Val::Px(0.0),
                                    flex_grow: 1.0,
                                    overflow: Overflow::scroll_y(),
                                    flex_direction: FlexDirection::Column,
                                    ..default()
                                },
                            ))
                            .with_child((
                                OutcomeReportBody,
                                blurb(
                                    &assets,
                                    outcome_report_text(
                                        report,
                                        report_state.mode,
                                        reports.as_deref(),
                                        report_state.compare_report,
                                    ),
                                ),
                            ));
                        panel
                            .spawn((
                                OutcomeCompareControls,
                                Node {
                                    flex_direction: FlexDirection::Row,
                                    column_gap: Val::Px(6.0),
                                    flex_wrap: FlexWrap::Wrap,
                                    ..default()
                                },
                                if report_state.mode == OutcomeReportMode::Compare {
                                    Visibility::Inherited
                                } else {
                                    Visibility::Hidden
                                },
                            ))
                            .with_children(|selectors| {
                                if let Some(reports) = reports.as_deref() {
                                    for saved in &reports.history.reports {
                                        let selected =
                                            report_state.compare_report == Some(saved.id);
                                        let label = if selected {
                                            format!("COMPARE · REPORT {}", saved.id.0)
                                        } else {
                                            format!("Report {}", saved.id.0)
                                        };
                                        selectors
                                            .spawn((
                                                row_button(label.clone(), 150.0),
                                                OutcomeReportControl::CompareWith(saved.id),
                                            ))
                                            .with_child(blurb(&assets, label));
                                    }
                                }
                            });
                    }
                    panel
                        .spawn(Node {
                            flex_direction: FlexDirection::Row,
                            column_gap: Val::Px(10.0),
                            ..default()
                        })
                        .with_children(|buttons| {
                            if report.is_some() {
                                for (action, text) in [
                                    (OutcomeAction::SaveReport, "Save Report"),
                                    (OutcomeAction::RetryExact, "Retry Exact"),
                                    (OutcomeAction::TuneAgain, "Tune & Run Again"),
                                ] {
                                    buttons
                                        .spawn((row_button(text, 170.0), action))
                                        .with_child(blurb(&assets, text));
                                }
                                if matches!(
                                    report.as_ref().map(|report| &report.origin),
                                    Some(crate::combat_reports::CombatLabReportOrigin::FixedFixture {
                                        ..
                                    })
                                ) {
                                    let text = "Copy to Sandbox";
                                    buttons
                                        .spawn((
                                            row_button(text, 170.0),
                                            OutcomeAction::CopyToSandbox,
                                        ))
                                        .with_child(blurb(&assets, text));
                                }
                            } else {
                                let primary = match outcome {
                                    EncounterOutcome::Victory => {
                                        (OutcomeAction::Continue, "Continue")
                                    }
                                    EncounterOutcome::Defeat => (OutcomeAction::Retry, "Retry"),
                                };
                                buttons
                                    .spawn((row_button(primary.1, 150.0), primary.0))
                                    .with_child(blurb(&assets, primary.1));
                            }
                            buttons
                                .spawn((
                                    row_button(return_label, 170.0),
                                    OutcomeAction::ReturnTitle,
                                ))
                                .with_child(blurb(&assets, return_label));
                        });
                });
        });
}

fn handle_outcome_report_controls(
    clicked: Query<(&Interaction, &OutcomeReportControl), Changed<Interaction>>,
    mut state: ResMut<OutcomeReportState>,
) {
    for (interaction, control) in &clicked {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match *control {
            OutcomeReportControl::Mode(mode) => state.select_mode(mode),
            OutcomeReportControl::CompareWith(id) => state.compare_with(id),
        }
    }
}

fn update_outcome_report(
    state: Res<OutcomeReportState>,
    report: Option<Res<CurrentCombatLabReport>>,
    reports: Option<Res<CombatLabReportStore>>,
    mut body: Query<&mut Text, With<OutcomeReportBody>>,
    tabs: Query<(&OutcomeReportTab, &Children)>,
    mut tab_text: Query<&mut Text, Without<OutcomeReportBody>>,
    mut compare_controls: Query<&mut Visibility, With<OutcomeCompareControls>>,
) {
    let Some(report) = report.as_deref() else {
        return;
    };
    if let Ok(mut text) = body.single_mut() {
        **text = outcome_report_text(
            &report.0,
            state.mode,
            reports.as_deref(),
            state.compare_report,
        );
    }
    for (tab, children) in &tabs {
        let Some((_, label)) = OutcomeReportMode::ALL
            .iter()
            .find(|(mode, _)| *mode == tab.0)
        else {
            continue;
        };
        if let Some(child) = children.first() {
            if let Ok(mut text) = tab_text.get_mut(*child) {
                **text = if state.mode == tab.0 {
                    format!("{label} · ACTIVE")
                } else {
                    (*label).to_owned()
                };
            }
        }
    }
    if let Ok(mut visibility) = compare_controls.single_mut() {
        *visibility = if state.mode == OutcomeReportMode::Compare {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}

pub(crate) fn outcome_report_text(
    report: &CombatLabReport,
    mode: OutcomeReportMode,
    store: Option<&CombatLabReportStore>,
    selected: Option<crate::combat_reports::CombatLabReportId>,
) -> String {
    match mode {
        OutcomeReportMode::Overview => format!(
            "OVERVIEW\nRounds {} · Turns {} · Commands {} successful / {} refused · AI choices {}\n\
             Movement {} distance / {} budget · Casts {} · Channel {} · Strikes {} · Idle {}\n\
             Disables {} raw / {} prevented / {} applied · Restored {} · Downed {} · Revived {}\n\
             No-progress stretch {} current / {} maximum · Timeline {} of {} canonical events retained",
            report.summary.rounds,
            report.summary.turns,
            report.summary.successful_commands,
            report.summary.refused_commands,
            report.summary.ai_selection_count,
            report.summary.movement_distance,
            report.summary.movement_budget_used,
            report.summary.casts,
            report.summary.channels,
            report.summary.strikes,
            report.summary.idle_turns,
            report.summary.raw_disables,
            report.summary.prevented_disables,
            report.summary.applied_disables,
            report.summary.restored_cells,
            report.summary.downings,
            report.summary.revivals,
            report.summary.no_progress_current,
            report.summary.no_progress_max,
            report.summary.events.len(),
            report.summary.event_count,
        ),
        OutcomeReportMode::Units => {
            let mut lines = vec!["UNITS · stable frozen roster order".to_owned()];
            for (side, roster) in [
                ("PLAYER", report.rosters.players.as_slice()),
                ("HOSTILE", report.rosters.hostiles.as_slice()),
            ] {
                for entry in roster {
                    let unit = UnitId(entry.unit_id);
                    let summary = report.summary.units.get(&unit).cloned().unwrap_or_default();
                    lines.push(format!(
                        "{side} · {} · {}",
                        entry.display_name,
                        format_unit_statistics(unit, &summary)
                    ));
                }
            }
            lines.join("\n")
        }
        OutcomeReportMode::SpellsEffects => {
            let casts = report
                .summary
                .casts_by_spell
                .iter()
                .map(|(spell, count)| format!("{spell} {count}"))
                .collect::<Vec<_>>()
                .join(" · ");
            let effects = report
                .summary
                .delivered_effects
                .iter()
                .map(|(effect, count)| format!("{effect:?} {count}"))
                .collect::<Vec<_>>()
                .join(" · ");
            let mana = report
                .summary
                .channelled_mana
                .iter()
                .map(|(element, amount)| format!("{element} {amount}"))
                .collect::<Vec<_>>()
                .join(" · ");
            format!(
                "SPELLS & EFFECTS\nCasts · {}\nDelivered · {}\nChannel mana · {}\nDisable flow · {} raw / {} prevented / {} applied · restorations {}",
                if casts.is_empty() { "none" } else { &casts },
                if effects.is_empty() { "none" } else { &effects },
                if mana.is_empty() { "none" } else { &mana },
                report.summary.raw_disables,
                report.summary.prevented_disables,
                report.summary.applied_disables,
                report.summary.restored_cells,
            )
        }
        OutcomeReportMode::Timeline => {
            let retained = report.summary.events.len();
            let shown = retained.min(18);
            let skipped = retained.saturating_sub(shown);
            let mut lines = vec![format!(
                "TIMELINE · showing final {shown} of {} retained / {} total{}",
                retained,
                report.summary.event_count,
                if u64::try_from(retained).unwrap_or(u64::MAX) < report.summary.event_count {
                    " · OLDER EVENTS TRUNCATED"
                } else {
                    ""
                }
            )];
            for (index, event) in report.summary.events.iter().skip(skipped).enumerate() {
                lines.push(format!("{:04} · {event:?}", skipped + index + 1));
            }
            lines.join("\n")
        }
        OutcomeReportMode::Compare => {
            let Some(store) = store else {
                return "COMPARE\nSaved report history is unavailable.".to_owned();
            };
            let comparison = selected
                .and_then(|id| store.history.reports.iter().find(|saved| saved.id == id))
                .or_else(|| store.history.reports.last());
            let Some(saved) = comparison else {
                return "COMPARE\nSave a report, then select it here. Fixed fixtures do not read history until Compare is explicitly opened.".to_owned();
            };
            format_report_comparison(report, &saved.report, saved.id.0)
        }
    }
}

fn format_report_comparison(
    current: &CombatLabReport,
    saved: &CombatLabReport,
    saved_id: u64,
) -> String {
    format!(
        "COMPARE · THIS RUN ↔ REPORT {saved_id}\n\
         THIS · {}\n\
         SAVED · {}\n\
         DELTAS (this − saved) · rounds {:+} · turns {:+} · successful {:+} · refused {:+}\n\
         movement {:+} · Channel {:+} · applied disables {:+} · no-progress max {:+}",
        outcome_frozen_header(current),
        outcome_frozen_header(saved),
        signed_report_delta(current.summary.rounds, saved.summary.rounds),
        signed_report_delta(current.summary.turns, saved.summary.turns),
        signed_report_delta(
            current.summary.successful_commands,
            saved.summary.successful_commands,
        ),
        signed_report_delta(
            current.summary.refused_commands,
            saved.summary.refused_commands,
        ),
        signed_report_delta(
            current.summary.movement_distance,
            saved.summary.movement_distance,
        ),
        signed_report_delta(current.summary.channels, saved.summary.channels),
        signed_report_delta(
            current.summary.applied_disables,
            saved.summary.applied_disables,
        ),
        signed_report_delta(
            current.summary.no_progress_max,
            saved.summary.no_progress_max,
        ),
    )
}

fn outcome_frozen_header(report: &CombatLabReport) -> String {
    let roster = |entries: &[crate::combat_reports::CombatLabReportRosterEntry]| {
        entries
            .iter()
            .map(|entry| entry.display_name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "{:?} · {} · rules [{}/{}/{}/{}/{}/{}] · P [{}] · H [{}]",
        report.profile.preset,
        report.map.scenario,
        report.profile.movement_per_turn,
        report.profile.strike_disables,
        report.profile.engage_range,
        report.profile.disengage_margin,
        report.profile.levels_per_bonus_range,
        report.profile.reveal_duration,
        roster(&report.rosters.players),
        roster(&report.rosters.hostiles),
    )
}

fn signed_report_delta(left: u32, right: u32) -> i64 {
    i64::from(left) - i64::from(right)
}

fn handle_outcome_actions(
    clicked: Query<(&Interaction, &OutcomeAction), Changed<Interaction>>,
    resolution: Res<EncounterResolution>,
    active: Option<Res<ActiveScenario>>,
    lab: Option<Res<CombatLabSession>>,
    overlay: Option<Res<CreatorContentOverlay>>,
    current_report: Option<Res<CurrentCombatLabReport>>,
    mut report_store: Option<ResMut<CombatLabReportStore>>,
    paths: Option<Res<StoragePaths>>,
    mut commands: Commands,
    mut next_mode: ResMut<NextState<Mode>>,
    mut next_screen: ResMut<NextState<Screen>>,
) {
    let Some(outcome) = resolution.outcome() else {
        return;
    };
    for (interaction, action) in &clicked {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match (*action, outcome) {
            (OutcomeAction::Continue, EncounterOutcome::Victory) => {
                if let Some(lab) = lab.as_deref() {
                    next_screen.set(lab.return_to);
                } else {
                    next_mode.set(Mode::Exploring);
                }
            }
            (OutcomeAction::Retry, EncounterOutcome::Defeat) => {
                let Some(active) = active.as_deref() else {
                    error!("cannot retry: active scenario launch input was not retained");
                    continue;
                };
                commands.insert_resource(active.0.clone());
                next_screen.set(Screen::Loading);
            }
            (
                action @ (OutcomeAction::RetryExact
                | OutcomeAction::TuneAgain
                | OutcomeAction::CopyToSandbox),
                _,
            ) => {
                let action = match action {
                    OutcomeAction::RetryExact => LabRunAction::RetryExact,
                    OutcomeAction::TuneAgain => LabRunAction::TuneAgain,
                    OutcomeAction::CopyToSandbox => LabRunAction::CopyToSandbox,
                    _ => unreachable!("match arm limits outcome actions"),
                };
                let transition = resolve_lab_run(
                    action,
                    active.as_deref().map(|active| &active.0),
                    current_report.as_deref().map(|report| &report.0),
                    |report| {
                        matches!(
                            report.origin,
                            crate::combat_reports::CombatLabReportOrigin::FixedFixture { .. }
                        )
                    },
                );
                match transition {
                    Ok(LabRunTransition::RetryExact(scenario)) => {
                        commands.insert_resource(scenario);
                        next_screen.set(Screen::Loading);
                    }
                    Ok(LabRunTransition::RestoreSandbox(report)) => {
                        commands.insert_resource(CombatLabSandboxRequest {
                            report,
                            overlay: overlay.as_deref().cloned(),
                        });
                        next_screen.set(Screen::CombatLab);
                    }
                    Err(failure) => {
                        let message = match failure {
                            LabRunFailure::MissingScenario => {
                                "cannot retry exact Lab run: frozen scenario input was not retained"
                            }
                            LabRunFailure::MissingReport => {
                                "cannot restore Lab run: frozen report state is unavailable"
                            }
                            LabRunFailure::CopyRequiresFixture => {
                                "cannot copy non-fixture report to Sandbox"
                            }
                        };
                        error!("{message}");
                    }
                }
            }
            (OutcomeAction::SaveReport, _) => {
                let (Some(report), Some(store), Some(lab), Some(paths)) = (
                    current_report.as_deref(),
                    report_store.as_deref_mut(),
                    lab.as_deref(),
                    paths.as_deref(),
                ) else {
                    error!("cannot save Lab report: frozen report state is unavailable");
                    continue;
                };
                match store.save(report.0.clone(), &lab.shipped_combat, paths) {
                    Ok(id) => info!("saved Combat Lab report {}", id.0),
                    Err(error) => error!("could not save Combat Lab report: {error}"),
                }
            }
            (OutcomeAction::ReturnTitle, _) => {
                next_screen.set(
                    lab.as_deref()
                        .map_or(Screen::Title, |session| session.return_to),
                );
            }
            _ => {}
        }
    }
}

/// Publishes authoritative game facts to the presentation-only action rail.
fn publish_hud_view(
    phase: Res<GameplayPhase>,
    resolution: Res<EncounterResolution>,
    mode: Res<State<Mode>>,
    order: Res<TurnOrder>,
    pending: Res<PendingDecision>,
    context: Res<GameplayUiContext>,
    acting: Query<(Has<Player>, &Turn)>,
    mut view: ResMut<GameplayHudView>,
) {
    if *phase == GameplayPhase::Deployment {
        let next = GameplayHudView {
            phase: GameplayPhase::Deployment,
            actor: None,
            actor_label: "Combat Lab deployment".to_owned(),
            round: "Setup".to_owned(),
            movement_remaining: 0,
            action_remaining: false,
            required_prompt: Some(
                "Choose each roster entry, place it on a matching surface, then confirm Start Combat."
                    .to_owned(),
            ),
            actions: Vec::new(),
        };
        if *view != next {
            *view = next;
        }
        return;
    }
    if let Some(outcome) = resolution.outcome() {
        let outcome = format!("{outcome:?}");
        let next = GameplayHudView {
            phase: GameplayPhase::Active,
            actor: None,
            actor_label: "Encounter complete".to_owned(),
            round: outcome,
            movement_remaining: 0,
            action_remaining: false,
            required_prompt: Some(
                "Review the encounter report, then retry, tune, copy, or return to Combat Lab."
                    .to_owned(),
            ),
            actions: Vec::new(),
        };
        if *view != next {
            *view = next;
        }
        return;
    }
    let next = match mode.get() {
        Mode::Exploring => GameplayHudView {
            phase: GameplayPhase::Active,
            actor: context.acting.as_ref().map(|actor| actor.unit),
            actor_label: context
                .acting
                .as_ref()
                .map_or_else(|| "Party".to_owned(), UiUnitIdentity::label),
            round: "Exploring".to_owned(),
            movement_remaining: 0,
            action_remaining: true,
            required_prompt: Some(
                "Choose: click a reachable surface to move, or use a party action.".to_owned(),
            ),
            actions: vec![
                ActionAffordance {
                    action: GameplayAction::Rest,
                    label: "Rest party".to_owned(),
                    shortcut: Some("R".to_owned()),
                    availability: ActionAvailability::Enabled,
                    priority: ActionPriority::Primary,
                },
                ActionAffordance {
                    action: GameplayAction::Pause,
                    label: "Pause".to_owned(),
                    shortcut: Some("Esc".to_owned()),
                    availability: ActionAvailability::Enabled,
                    priority: ActionPriority::Secondary,
                },
            ],
        },
        Mode::Combat => {
            // How much movement is left, spelled out. Without it a click refused for
            // being out of range is indistinguishable from a click that did not
            // register — which is precisely the complaint the tinted range answers,
            // and the number is what confirms the tint rather than merely repeating it.
            let (player_turn, movement_remaining, action_remaining) = match acting.single() {
                Ok((true, turn)) => (true, turn.movement_left, !turn.acted),
                Ok((false, _)) | Err(_) => (false, 0, false),
            };
            let actor = context
                .acting
                .as_ref()
                .map_or_else(|| "No active unit".to_owned(), UiUnitIdentity::label);
            let required_prompt = Some(
                decision_context_hint(&context, &pending)
                    .unwrap_or_else(|| combat_action_hint(player_turn, &pending).to_owned()),
            );
            let availability = if player_turn && !pending.is_open() {
                ActionAvailability::Enabled
            } else {
                ActionAvailability::Disabled {
                    reason: if pending.is_open() {
                        "Resolve the required lattice choice first".to_owned()
                    } else {
                        "Enemy turn".to_owned()
                    },
                }
            };
            let mut actions = vec![
                ActionAffordance {
                    action: GameplayAction::Channel,
                    label: "Channel".to_owned(),
                    shortcut: None,
                    availability: if action_remaining {
                        availability.clone()
                    } else {
                        ActionAvailability::Disabled {
                            reason: "Action already spent".to_owned(),
                        }
                    },
                    priority: ActionPriority::Primary,
                },
                ActionAffordance {
                    action: GameplayAction::EndTurn,
                    label: "End turn".to_owned(),
                    shortcut: Some("Space".to_owned()),
                    availability,
                    priority: ActionPriority::Primary,
                },
            ];
            if pending.is_open() {
                actions.insert(
                    0,
                    ActionAffordance {
                        action: GameplayAction::ConfirmDecision,
                        label: "Confirm choice".to_owned(),
                        shortcut: Some("Enter".to_owned()),
                        availability: ActionAvailability::Disabled {
                            reason: "Choose the required cells in the lattice".to_owned(),
                        },
                        priority: ActionPriority::Required,
                    },
                );
            }
            GameplayHudView {
                phase: GameplayPhase::Active,
                actor: context.acting.as_ref().map(|actor| actor.unit),
                actor_label: actor,
                round: format!("Round {}", order.round + 1),
                movement_remaining,
                action_remaining,
                required_prompt,
                actions,
            }
        }
    };
    if *view != next {
        *view = next;
    }
}

fn decision_context_hint(context: &GameplayUiContext, pending: &PendingDecision) -> Option<String> {
    let owner = context
        .decision_owner
        .as_ref()
        .map(UiUnitIdentity::label)
        .unwrap_or_else(|| "UNKNOWN ALLY".to_owned());
    let target = context
        .decision_target
        .as_ref()
        .map(UiUnitIdentity::label)
        .unwrap_or_else(|| "UNKNOWN TARGET".to_owned());
    match pending {
        PendingDecision::ChooseDisables { .. } => {
            Some(format!("DAMAGE CHOICE · {owner} · CHOOSE LIVE CELLS"))
        }
        PendingDecision::ChooseRestores { .. } => {
            Some(format!("CASTER {owner} · RESTORE TARGET {target}"))
        }
        PendingDecision::None => None,
    }
}

/// The action hint must agree with the command emitters: Space and casting are
/// player-turn controls, while an open defender choice replaces every ordinary
/// simulation command.
fn combat_action_hint(player_turn: bool, pending: &PendingDecision) -> &'static str {
    match pending {
        PendingDecision::ChooseDisables { .. } => "choose a live cell above, then ENTER to confirm",
        PendingDecision::ChooseRestores { .. } => {
            "choose a disabled target cell above, then ENTER to confirm"
        }
        PendingDecision::None if player_turn => "cast from the panel   ·   SPACE to end turn",
        PendingDecision::None => "waiting for the enemy",
    }
}

/// Flips the dev reveal-all toggle, so a designer can see the truth behind the
/// fog while playing.
///
/// Behind the `dev` feature deliberately: the shipped build has no key that
/// exposes hidden information, and hidden information is the game's source of
/// uncertainty rather than dice. `K` for knowledge — `Escape`, `Backspace`,
/// `Space`, `Tab`, `Q`, `H`, `C`, `Enter` and `WASD` are all taken.
///
/// The resource is initialised by `hex_combat`'s plugin, which the binary always
/// adds, so this cannot be the observer-on-the-title-screen crash: it is a
/// system, it is gated on the gameplay screen, and its parameter always resolves.
///
/// Logs the new state as well as updating the hostile lattice panel. The line is
/// useful when a designer is validating disclosure and the HUD itself is hidden.
#[cfg(feature = "dev")]
fn toggle_reveal_all(
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<InputBindings>,
    mut reveal: ResMut<hex_combat::RevealAll>,
) {
    if bindings.just_pressed(&keys, InputAction::RevealAll) {
        reveal.0 = !reveal.0;
        info!("reveal-all {}", if reveal.0 { "on" } else { "off" });
    }
}

/// Entering gameplay always starts unpaused, so a pause left set from a previous
/// session cannot leak into a new one.
fn reset_pause(mut next: ResMut<NextState<Pause>>) {
    next.set(Pause(false));
}

/// And always starts out of combat, for the same reason.
fn reset_mode(mut next: ResMut<NextState<Mode>>) {
    next.set(Mode::Exploring);
}

fn handle_input(
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<InputBindings>,
    pause: Res<State<Pause>>,
    mut next_pause: ResMut<NextState<Pause>>,
    mut next_screen: ResMut<NextState<Screen>>,
    lab: Option<Res<CombatLabSession>>,
) {
    if bindings.just_pressed(&keys, InputAction::Pause) {
        next_pause.set(Pause(!pause.get().0));
    }
    // Backspace rather than Escape, which is taken by pause.
    if bindings.just_pressed(&keys, InputAction::ReturnTitle) {
        next_screen.set(
            lab.as_deref()
                .map_or(Screen::Title, |session| session.return_to),
        );
    }
}

#[cfg(test)]
mod tests {
    use bevy::state::app::StatesPlugin;
    use bevy::MinimalPlugins;
    use hex_assets::ScenarioLibrary;
    use hex_core::{ResolvedMapSeed, TilePos};

    use super::*;

    fn sample_report(rounds: u32) -> CombatLabReport {
        let shipped = hex_assets::CombatSettings::default();
        let mut summary = CombatSummary::default();
        summary.rounds = rounds;
        summary.turns = rounds.saturating_mul(3);
        summary.no_progress_max = 2;
        summary.outcome = Some(EncounterOutcome::Victory);
        summary.units.insert(
            UnitId(1),
            UnitCombatSummary {
                movement_distance: 3,
                channels: 1,
                ..default()
            },
        );
        CombatLabReport::new(
            hex_assets::CombatRulesProfile::shipped(&shipped),
            crate::combat_reports::CombatLabReportOrigin::Sandbox,
            crate::combat_reports::CombatLabReportMap {
                catalog_id: "flat-arena".to_owned(),
                scenario: "Ability Lab".to_owned(),
                resolved_seed: None,
            },
            77,
            crate::combat_reports::CombatLabReportRosters {
                players: vec![crate::combat_reports::CombatLabReportRosterEntry {
                    unit_id: 1,
                    archetype: "hedge-mage".to_owned(),
                    display_name: "Hedge Mage".to_owned(),
                    controller: crate::combat_reports::CombatLabReportController::Human,
                }],
                hostiles: vec![crate::combat_reports::CombatLabReportRosterEntry {
                    unit_id: 2,
                    archetype: "raider".to_owned(),
                    display_name: "Raider".to_owned(),
                    controller: crate::combat_reports::CombatLabReportController::BaselineAi,
                }],
            },
            crate::combat_reports::CombatLabReportDeployment {
                players: vec![TilePos::new(HexCoord::ORIGIN, 1)],
                hostiles: vec![TilePos::new(HexCoord::from_axial(1, 0), 1)],
            },
            CombatLabReportTermination::Outcome(EncounterOutcome::Victory),
            summary,
        )
        .expect("fixture report evidence")
    }

    #[test]
    fn combat_hints_never_offer_player_commands_during_an_enemy_turn() {
        assert_eq!(
            combat_action_hint(true, &PendingDecision::None),
            "cast from the panel   ·   SPACE to end turn"
        );
        assert_eq!(
            combat_action_hint(false, &PendingDecision::None),
            "waiting for the enemy"
        );
        assert_eq!(
            combat_action_hint(
                false,
                &PendingDecision::ChooseDisables {
                    decider: UnitId(1),
                    count: 1,
                    source: UnitId(2),
                }
            ),
            "choose a live cell above, then ENTER to confirm"
        );
    }

    #[test]
    fn decision_hints_name_owner_and_affected_target() {
        let owner = UiUnitIdentity {
            unit: UnitId(1),
            name: "hedge-mage #1".to_owned(),
            faction: hex_units::Faction::Player,
            party_slot: Some(0),
        };
        let target = UiUnitIdentity {
            unit: UnitId(2),
            name: "raider #2".to_owned(),
            faction: hex_units::Faction::Player,
            party_slot: Some(1),
        };
        let context = GameplayUiContext {
            decision_owner: Some(owner),
            decision_target: Some(target),
            ..default()
        };

        let restore = decision_context_hint(
            &context,
            &PendingDecision::ChooseRestores {
                decider: UnitId(1),
                target: UnitId(2),
                count: 1,
            },
        )
        .expect("restoration is a decision");
        assert!(restore.contains("CASTER ALLY 1 · HEDGE-MAGE #1"));
        assert!(restore.contains("RESTORE TARGET ALLY 2 · RAIDER #2"));
    }

    #[test]
    fn outcome_report_tabs_are_wired_controls_not_inert_buttons() {
        let shipped = hex_assets::CombatSettings::default();
        let report = sample_report(4);
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(UiAssets {
                display: Handle::default(),
                body: Handle::default(),
                hex_cell: Handle::default(),
            })
            .insert_resource(EncounterResolution(Some(EncounterOutcome::Victory)))
            .insert_resource(report.summary.clone())
            .insert_resource(CombatLabSession {
                kind: super::super::combat_lab::CombatLabSessionKind::Sandbox,
                return_to: Screen::CombatLab,
                profile: report.profile.clone(),
                shipped_combat: shipped,
                report_map: report.map.clone(),
                initial_state: None,
            })
            .insert_resource(CombatLabReportLaunch {
                origin: report.origin.clone(),
                map: report.map.clone(),
                content_revision: report.content_revision,
                rosters: report.rosters.clone(),
                deployment: report.deployment.clone(),
            })
            .init_resource::<CombatLabReportStore>()
            .init_resource::<OutcomeReportState>()
            .add_systems(Update, sync_outcome_modal);
        app.update();

        let mut controls = app.world_mut().query::<&OutcomeReportControl>();
        let modes = controls
            .iter(app.world())
            .filter(|control| matches!(control, OutcomeReportControl::Mode(_)))
            .count();
        assert_eq!(modes, OutcomeReportMode::ALL.len());
        let mut body = app
            .world_mut()
            .query_filtered::<Entity, With<OutcomeReportBody>>();
        assert_eq!(body.iter(app.world()).count(), 1);
    }

    #[test]
    fn retry_requeues_the_exact_active_scenario_and_seed() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin))
            .init_state::<Screen>()
            .add_sub_state::<Mode>();
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Gameplay);
        app.update();
        app.update();

        let library: ScenarioLibrary =
            ron::from_str(include_str!("../../../../assets/config/scenarios.ron"))
                .expect("shipped scenarios parse");
        let scenario = library
            .scenarios
            .into_iter()
            .find(|scenario| scenario.generation_seed.is_some())
            .expect("a generated scenario exists");
        let seed = ResolvedMapSeed(9_001);
        app.insert_resource(EncounterResolution(Some(EncounterOutcome::Defeat)));
        app.insert_resource(ActiveScenario(crate::scenarios::ScenarioToLoad {
            scenario: scenario.clone(),
            resolved_seed: Some(seed),
            encounter_override: None,
        }));
        app.world_mut()
            .spawn((Interaction::Pressed, OutcomeAction::Retry));
        app.add_systems(Update, handle_outcome_actions);
        app.update();

        let retry = app.world().resource::<crate::scenarios::ScenarioToLoad>();
        assert_eq!(retry.scenario.name, scenario.name);
        assert_eq!(retry.scenario.world, scenario.world);
        assert_eq!(retry.scenario.encounter, scenario.encounter);
        assert_eq!(retry.resolved_seed, Some(seed));
    }
}
