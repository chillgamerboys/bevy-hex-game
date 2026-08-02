//! The game itself.
//!
//! Owns the pause toggle, the route back to Main Menu, and the HUD. Terrain,
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
use hex_assets::{CombatSettings, ElementCatalog, FixedSettingsFreeze, FormationCatalog};
use hex_combat::{
    ChannelReadiness, CommandRefusal, EncounterOutcome, EncounterResolution, Turn, TurnOrder,
};
use hex_core::{
    AppSystems, Busy, CommandQueue, ControlOwner, GameCommand, GameplayPhase, GameplaySystems,
    InputAction, InputBindings, IssuedCommand, Mode, PartyFormation, PartyMovementMode, Pause,
    PendingDecision, Screen, UnitId,
};
use hex_gameplay_model::{HudActionResult, HudState, MainMenuModel, MainMenuRoute};
use hex_lattice::{CellKind, LatticeSpec, LatticeState, LatticeStats};
use hex_units::{Archetype, Downed, Party, Player, Selected, UnitRegistry};

use super::despawn_screen;
use super::sandbox::{CreatorDisplayName, GameplaySessionOrigin, SandboxSession};
use crate::readouts::{ActivityNotice, DisableSelection, GameplayUiContext, UiUnitIdentity};
use crate::scenarios::ActiveScenario;
use hex_ui::{
    ActionAffordance, ActionAvailability, ActionPriority, GameplayAction, GameplayHudView,
    OutcomeAction, OutcomeActionView, OutcomeView,
};

pub(crate) fn plugin(app: &mut App) {
    app.init_resource::<InputBindings>()
        .init_resource::<MainMenuModel>();
    app.add_sub_state::<Pause>();
    app.register_type::<Pause>();
    // A second sub-state of `Screen::Gameplay`, independent of `Pause`. Both are
    // computed from the screen, so pausing does not destroy the mode.
    app.add_sub_state::<Mode>();
    app.register_type::<Mode>();

    app.add_systems(
        Update,
        handle_input
            .after(AppSystems::Update)
            .run_if(in_state(Screen::Gameplay))
            .run_if(resource_equals(GameplayPhase::Active))
            .run_if(hex_combat::encounter_unresolved),
    );
    app.add_systems(
        Update,
        publish_hud_view
            .in_set(AppSystems::Update)
            .after(GameplaySystems::UiContext)
            .before(hex_ui::UiSystems::Render)
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
        (sync_outcome_view, handle_outcome_actions)
            .chain()
            .after(hex_ui::UiSystems::EmitIntents)
            .before(hex_ui::UiSystems::Render)
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
        (reset_pause, reset_mode, reset_outcome_view),
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
    pause: Res<State<Pause>>,
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
            GameplayAction::Pause => next_pause.set(toggled_pause(*pause.get())),
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
            GameplayAction::ConfirmDecision => {
                // The decision input system independently reads this same typed
                // intent and reduces it through the exact ChooseDisables/
                // ChooseRestores command funnel.
            }
            GameplayAction::Rest => {}
        }
    }
}

fn reset_outcome_view(mut view: ResMut<OutcomeView>) {
    *view = OutcomeView::default();
}

fn handle_party_strip(
    mode: Res<State<Mode>>,
    party: Res<Party>,
    registry: Res<UnitRegistry>,
    formations: Res<FormationCatalog>,
    mut formation: ResMut<PartyFormation>,
    mut intents: MessageReader<hex_ui::UiIntent>,
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<InputBindings>,
    mut queue: ResMut<CommandQueue>,
    mut activity: MessageWriter<ActivityNotice>,
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
            // Inspection is presentation-only and handled by `readouts`; it must
            // never rewrite gameplay `Selected` or command authority.
            hex_ui::PartyIntent::ActivateMember(_) => {}
            hex_ui::PartyIntent::ToggleMovementMode => {
                formation.mode = match formation.mode {
                    PartyMovementMode::Group => PartyMovementMode::Solo,
                    PartyMovementMode::Solo => PartyMovementMode::Group,
                };
                activity.write(ActivityNotice(format!(
                    "Party movement changed to {:?}.",
                    formation.mode
                )));
            }
            hex_ui::PartyIntent::SelectPreset(name) => {
                if let Some(preset) = formations.get(name) {
                    formation.select_preset(preset, &party.members);
                    activity.write(ActivityNotice(format!("Formation changed to {name}.")));
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
                    activity.write(ActivityNotice("Formation assignment changed.".to_owned()));
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
            activity.write(ActivityNotice("Party rest requested.".to_owned()));
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
    elements: Option<Res<ElementCatalog>>,
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
                cells: spec
                    .map(|spec| party_lattice_cells(spec, elements.as_deref()))
                    .unwrap_or_default(),
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

fn party_lattice_cells(
    spec: &LatticeSpec,
    elements: Option<&ElementCatalog>,
) -> Vec<hex_ui::SandboxLatticeCellView> {
    spec.cells()
        .map(|(coord, kind)| {
            let (label, kind) = match kind {
                CellKind::Gem { element } => (
                    elements
                        .and_then(|elements| elements.name(element))
                        .map_or_else(|| "G".to_owned(), compact_lattice_label),
                    hex_ui::SandboxLatticeCellKind::Gem,
                ),
                CellKind::Fusion { output } => (
                    elements
                        .and_then(|elements| elements.name(output))
                        .map_or_else(|| "F".to_owned(), compact_lattice_label),
                    hex_ui::SandboxLatticeCellKind::Fusion,
                ),
                CellKind::Spell { .. } => ("S".to_owned(), hex_ui::SandboxLatticeCellKind::Spell),
                CellKind::Blank => ("·".to_owned(), hex_ui::SandboxLatticeCellKind::Blank),
            };
            hex_ui::SandboxLatticeCellView {
                q: coord.q(),
                r: coord.r(),
                label,
                kind,
            }
        })
        .collect()
}

fn compact_lattice_label(name: &str) -> String {
    name.chars()
        .filter(|character| character.is_alphanumeric())
        .take(2)
        .collect::<String>()
        .to_uppercase()
}
fn sync_outcome_view(
    resolution: Res<EncounterResolution>,
    sandbox: Option<Res<SandboxSession>>,
    mut view: ResMut<OutcomeView>,
) {
    let Some(outcome) = resolution.outcome() else {
        if view.visible {
            *view = OutcomeView::default();
        }
        return;
    };
    let actions = if sandbox.is_some() {
        vec![
            OutcomeActionView {
                action: OutcomeAction::RetryExact,
                label: "Retry Exact".to_owned(),
            },
            OutcomeActionView {
                action: OutcomeAction::Return,
                label: "Return to Sandbox".to_owned(),
            },
        ]
    } else {
        vec![
            OutcomeActionView {
                action: match outcome {
                    EncounterOutcome::Victory => OutcomeAction::Continue,
                    EncounterOutcome::Defeat => OutcomeAction::Retry,
                },
                label: match outcome {
                    EncounterOutcome::Victory => "Continue",
                    EncounterOutcome::Defeat => "Retry",
                }
                .to_owned(),
            },
            OutcomeActionView {
                action: OutcomeAction::Return,
                label: "Return to Main Menu".to_owned(),
            },
        ]
    };
    let next = OutcomeView {
        visible: true,
        title: match outcome {
            EncounterOutcome::Victory => "Victory",
            EncounterOutcome::Defeat => "Defeat",
        }
        .to_owned(),
        detail: match (sandbox.is_some(), outcome) {
            (true, EncounterOutcome::Victory) => "The Enemy roster can no longer continue.",
            (true, EncounterOutcome::Defeat) => "The Party roster can no longer continue.",
            (false, EncounterOutcome::Victory) => {
                "The encounter is complete. Continue the Campaign when ready."
            }
            (false, EncounterOutcome::Defeat) => {
                "Retry replays this scenario with the same resolved seed."
            }
        }
        .to_owned(),
        actions,
    };
    if *view != next {
        *view = next;
    }
}

fn handle_outcome_actions(
    mut intents: MessageReader<hex_ui::UiIntent>,
    resolution: Res<EncounterResolution>,
    active: Option<Res<ActiveScenario>>,
    sandbox: Option<Res<SandboxSession>>,
    origin: Option<Res<GameplaySessionOrigin>>,
    mut commands: Commands,
    mut next_mode: ResMut<NextState<Mode>>,
    mut next_screen: ResMut<NextState<Screen>>,
    mut main_menu: ResMut<MainMenuModel>,
) {
    let Some(outcome) = resolution.outcome() else {
        return;
    };
    for intent in intents.read() {
        let hex_ui::UiIntent::Outcome(hex_ui::OutcomeIntent::Activate(action)) = intent else {
            continue;
        };
        match (*action, outcome) {
            (OutcomeAction::Continue, EncounterOutcome::Victory) if sandbox.is_none() => {
                next_mode.set(Mode::Exploring);
            }
            (OutcomeAction::Retry, EncounterOutcome::Defeat) if sandbox.is_none() => {
                let Some(active) = active.as_deref() else {
                    error!("cannot retry: active scenario launch input was not retained");
                    continue;
                };
                commands.insert_resource(active.0.clone());
                next_screen.set(Screen::Loading);
            }
            (OutcomeAction::RetryExact, _) if sandbox.is_some() => {
                let Some(sandbox) = sandbox.as_deref() else {
                    error!(
                        "cannot retry exact Sandbox run: frozen launch snapshot was not retained"
                    );
                    continue;
                };
                // `SandboxSession` and `CreatorContentOverlay` deliberately remain
                // installed. Loading consumes the same frozen scenario identity,
                // deployment, roster, and content snapshot as the completed run.
                commands.insert_resource(FixedSettingsFreeze::<CombatSettings>::default());
                commands.insert_resource(FixedSettingsFreeze::<hex_assets::SpellFile>::default());
                commands.insert_resource(FixedSettingsFreeze::<hex_assets::LatticeFile>::default());
                commands.insert_resource(sandbox.launch.loading_input());
                commands.insert_resource(sandbox.launch.rules.clone());
                next_screen.set(Screen::Loading);
            }
            (OutcomeAction::Return, _) => {
                let destination = gameplay_return_screen(origin.as_deref(), sandbox.is_some());
                prepare_main_menu_for_gameplay_return(destination, &mut main_menu);
                next_screen.set(destination);
            }
            _ => {}
        }
    }
}

fn gameplay_return_screen(
    origin: Option<&GameplaySessionOrigin>,
    has_sandbox_session: bool,
) -> Screen {
    if has_sandbox_session || matches!(origin, Some(GameplaySessionOrigin::Sandbox)) {
        Screen::Sandbox
    } else {
        Screen::Title
    }
}

fn prepare_main_menu_for_gameplay_return(destination: Screen, main_menu: &mut MainMenuModel) {
    if destination == Screen::Title {
        main_menu.show(MainMenuRoute::Root);
    }
}

/// Publishes authoritative game facts to the presentation-only Action Bar.
fn publish_hud_view(
    phase: Res<GameplayPhase>,
    resolution: Res<EncounterResolution>,
    mode: Res<State<Mode>>,
    order: Res<TurnOrder>,
    pending: Res<PendingDecision>,
    selection: Res<DisableSelection>,
    bindings: Res<InputBindings>,
    context: Res<GameplayUiContext>,
    elements: Option<Res<ElementCatalog>>,
    acting: Query<(
        &UnitId,
        Has<Player>,
        &Turn,
        Has<Busy>,
        Has<Downed>,
        Option<&LatticeSpec>,
        Option<&LatticeStats>,
    )>,
    mut view: ResMut<GameplayHudView>,
) {
    if *phase == GameplayPhase::Deployment {
        let next = GameplayHudView {
            phase: GameplayPhase::Deployment,
            actor: None,
            actor_label: "Sandbox deployment".to_owned(),
            round: "Setup".to_owned(),
            movement_remaining: 0,
            action_remaining: false,
            required_prompt: Some(
                "Place each character on any open legal surface, then review and start combat."
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
            required_prompt: Some("Choose Retry Exact or return to the session setup.".to_owned()),
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
                    shortcut: Some(bindings.chord(InputAction::Rest).label()),
                    availability: ActionAvailability::Enabled,
                    priority: ActionPriority::Primary,
                },
                ActionAffordance {
                    action: GameplayAction::Pause,
                    label: "Pause".to_owned(),
                    shortcut: Some(bindings.chord(InputAction::Pause).label()),
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
            let actor_facts = acting.single().ok();
            let (player_turn, movement_remaining, action_remaining) = match actor_facts {
                Some((_, true, turn, _, _, _, _)) => (true, turn.movement_left, !turn.acted),
                Some((_, false, _, _, _, _, _)) | None => (false, 0, false),
            };
            let actor = context
                .acting
                .as_ref()
                .map_or_else(|| "No active unit".to_owned(), UiUnitIdentity::label);
            let required_prompt = Some(
                decision_context_hint(&context, &pending)
                    .unwrap_or_else(|| combat_action_hint(player_turn, &pending, &bindings)),
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
            let channel = match actor_facts {
                Some((unit, true, turn, busy, downed, lattice, stats)) if !pending.is_open() => {
                    channel_availability(hex_combat::channel_refusal(ChannelReadiness {
                        in_combat: true,
                        unit: *unit,
                        current: order.current(),
                        downed,
                        busy,
                        turn: Some(turn),
                        lattice,
                        stats,
                        elements: elements.as_deref(),
                    }))
                }
                Some((_, true, _, _, _, _, _)) => ActionAvailability::Disabled {
                    reason: "Resolve the required lattice choice first".to_owned(),
                },
                Some((_, false, _, _, _, _, _)) | None => ActionAvailability::Disabled {
                    reason: "Enemy turn".to_owned(),
                },
            };
            let mut actions = vec![
                ActionAffordance {
                    action: GameplayAction::Channel,
                    label: "Channel".to_owned(),
                    shortcut: None,
                    availability: channel,
                    priority: ActionPriority::Primary,
                },
                ActionAffordance {
                    action: GameplayAction::EndTurn,
                    label: "End turn".to_owned(),
                    shortcut: Some(bindings.chord(InputAction::EndTurn).label()),
                    availability,
                    priority: ActionPriority::Primary,
                },
            ];
            if pending.is_open() && selection.remaining_choices().is_some() {
                actions.insert(
                    0,
                    ActionAffordance {
                        action: GameplayAction::ConfirmDecision,
                        label: "Confirm choice".to_owned(),
                        shortcut: Some(bindings.chord(InputAction::Confirm).label()),
                        availability: decision_confirmation_availability(
                            selection.remaining_choices(),
                        ),
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

fn decision_confirmation_availability(remaining: Option<usize>) -> ActionAvailability {
    match remaining {
        Some(0) => ActionAvailability::Enabled,
        Some(1) => ActionAvailability::Disabled {
            reason: "Choose 1 more lattice cell".to_owned(),
        },
        Some(remaining) => ActionAvailability::Disabled {
            reason: format!("Choose {remaining} more lattice cells"),
        },
        None => ActionAvailability::Disabled {
            reason: "Waiting for the decision owner".to_owned(),
        },
    }
}

fn channel_availability(refusal: Option<CommandRefusal>) -> ActionAvailability {
    let Some(refusal) = refusal else {
        return ActionAvailability::Enabled;
    };
    let reason = match refusal {
        CommandRefusal::Busy => "Still finishing movement",
        CommandRefusal::ActionAlreadySpent => "Action already spent",
        CommandRefusal::ActingUnitDowned { .. } => "This unit is downed",
        CommandRefusal::NoTurn => "No active turn",
        CommandRefusal::MissingUnitData {
            data: hex_combat::UnitData::Lattice,
            ..
        } => "This unit has no channel lattice",
        CommandRefusal::MissingCombatData {
            data: hex_combat::CombatData::ElementCatalog,
        } => "Element content is unavailable",
        CommandRefusal::NotCurrentTurn { .. } => "Another unit is acting",
        CommandRefusal::CombatOnly => "Channel is combat-only",
        _ => "Command unavailable",
    };
    ActionAvailability::Disabled {
        reason: reason.to_owned(),
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

/// The action hint must agree with the configured command bindings. An open
/// defender choice replaces every ordinary simulation command.
fn combat_action_hint(
    player_turn: bool,
    pending: &PendingDecision,
    bindings: &InputBindings,
) -> String {
    let confirm = bindings.chord(InputAction::Confirm).label();
    match pending {
        PendingDecision::ChooseDisables { .. } => {
            format!("choose a live cell above, then {confirm} to confirm")
        }
        PendingDecision::ChooseRestores { .. } => {
            format!("choose a disabled target cell above, then {confirm} to confirm")
        }
        PendingDecision::None if player_turn => format!(
            "cast from the panel   ·   {} to end turn",
            bindings.chord(InputAction::EndTurn).label()
        ),
        PendingDecision::None => "waiting for the enemy".to_owned(),
    }
}

/// Flips the dev reveal-all toggle, so a designer can see the truth behind the
/// fog while playing.
///
/// Behind the `dev` feature deliberately: the shipped build has no key that
/// exposes hidden information, and hidden information is the game's source of
/// uncertainty rather than dice. The default remains `K`; Settings owns any
/// collision-safe customization.
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
    sandbox: Option<Res<SandboxSession>>,
    origin: Option<Res<GameplaySessionOrigin>>,
    mut main_menu: ResMut<MainMenuModel>,
    mut hud: ResMut<HudState>,
) {
    let escape_closed_surface = keys.just_pressed(KeyCode::Escape)
        && hud.close_active_surface() != HudActionResult::NoChange;
    if bindings.just_pressed(&keys, InputAction::Pause) && !escape_closed_surface {
        next_pause.set(toggled_pause(*pause.get()));
    }
    // The return action stays distinct from Escape, which always dismisses an
    // ordinary HUD task before the configured pause action may run.
    if bindings.just_pressed(&keys, InputAction::ReturnTitle) {
        let destination = gameplay_return_screen(origin.as_deref(), sandbox.is_some());
        prepare_main_menu_for_gameplay_return(destination, &mut main_menu);
        next_screen.set(destination);
    }
}

fn toggled_pause(pause: Pause) -> Pause {
    Pause(!pause.0)
}

#[cfg(test)]
mod tests {
    use bevy::state::app::StatesPlugin;
    use bevy::MinimalPlugins;
    use hex_assets::{
        CombatSettings, Encounter, EncounterFaction, EncounterPlacement, Roster, RosterEntry,
        ScenarioLibrary,
    };
    use hex_core::{HexCoord, ResolvedMapSeed, TilePos};
    use hex_gameplay_model::{CampaignSlotId, SandboxCharacter, SandboxMapSelection};

    use super::super::sandbox::{SandboxDeploymentSnapshot, SandboxLaunchSnapshot};

    #[test]
    fn typed_and_keyboard_pause_paths_share_the_same_toggle() {
        assert_eq!(toggled_pause(Pause(false)), Pause(true));
        assert_eq!(toggled_pause(Pause(true)), Pause(false));
    }

    #[test]
    fn required_confirmation_enables_exactly_when_the_choice_is_complete() {
        assert_eq!(
            decision_confirmation_availability(Some(0)),
            ActionAvailability::Enabled
        );
        assert_eq!(
            decision_confirmation_availability(Some(2)),
            ActionAvailability::Disabled {
                reason: "Choose 2 more lattice cells".to_owned(),
            }
        );
    }

    #[test]
    fn channel_affordance_preserves_the_combat_owned_refusal() {
        assert_eq!(
            channel_availability(Some(CommandRefusal::Busy)),
            ActionAvailability::Disabled {
                reason: "Still finishing movement".to_owned(),
            }
        );
        assert_eq!(channel_availability(None), ActionAvailability::Enabled);
    }

    use super::*;

    fn sample_sandbox_session(seed: u64) -> SandboxSession {
        let library: ScenarioLibrary =
            ron::from_str(include_str!("../../../../assets/config/scenarios.ron"))
                .expect("shipped scenarios parse");
        let scenario = library
            .scenarios
            .into_iter()
            .find(|scenario| scenario.name == "Procedural Hills")
            .expect("Sandbox scenario exists");
        let party = vec![
            SandboxCharacter::Template("hedge-mage".to_owned()),
            SandboxCharacter::Template("hedge-mage".to_owned()),
        ];
        let enemies = vec![
            SandboxCharacter::Template("raider".to_owned()),
            SandboxCharacter::Template("raider".to_owned()),
        ];
        let party_surfaces = vec![
            TilePos::new(HexCoord::from_axial(-2, 1), 1),
            TilePos::new(HexCoord::from_axial(-1, 1), 1),
        ];
        let enemy_surfaces = vec![
            TilePos::new(HexCoord::from_axial(2, -1), 1),
            TilePos::new(HexCoord::from_axial(3, -1), 1),
        ];
        let initial = Encounter {
            name: "Sandbox test".to_owned(),
            rosters: Vec::new(),
        };
        let rules = CombatSettings {
            movement_per_turn: 3,
            ..Default::default()
        };
        let mut launch = SandboxLaunchSnapshot::new(
            SandboxMapSelection::new("procedural-hills", Some(seed)),
            "Procedural Hills".to_owned(),
            party.clone(),
            enemies.clone(),
            Some(77),
            rules,
            scenario,
            initial,
        );
        let exact = Encounter {
            name: "Sandbox test · exact".to_owned(),
            rosters: [
                (EncounterFaction::Player, &party, &party_surfaces),
                (EncounterFaction::Hostile, &enemies, &enemy_surfaces),
            ]
            .into_iter()
            .map(|(faction, choices, surfaces)| Roster {
                faction,
                placement: EncounterPlacement::Surface(
                    surfaces.first().copied().unwrap_or(TilePos::ORIGIN),
                ),
                units: choices
                    .iter()
                    .zip(surfaces)
                    .map(|(choice, surface)| RosterEntry {
                        archetype: match choice {
                            SandboxCharacter::Template(key) => key.clone(),
                            SandboxCharacter::Custom(id) => format!("custom-character-{}", id.0),
                        },
                        placement: Some(EncounterPlacement::Surface(*surface)),
                        ai_profile: None,
                        ai_group: None,
                    })
                    .collect(),
            })
            .collect(),
        };
        launch.freeze_deployment(
            SandboxDeploymentSnapshot {
                party: party_surfaces,
                enemies: enemy_surfaces,
            },
            exact,
            Some(77),
        );
        SandboxSession { launch }
    }

    #[test]
    fn combat_hints_never_offer_player_commands_during_an_enemy_turn() {
        let bindings = InputBindings::default();
        assert_eq!(
            combat_action_hint(true, &PendingDecision::None, &bindings),
            "cast from the panel   ·   Space to end turn"
        );
        assert_eq!(
            combat_action_hint(false, &PendingDecision::None, &bindings),
            "waiting for the enemy"
        );
        assert_eq!(
            combat_action_hint(
                false,
                &PendingDecision::ChooseDisables {
                    decider: UnitId(1),
                    count: 1,
                    source: UnitId(2),
                },
                &bindings,
            ),
            "choose a live cell above, then Enter to confirm"
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
    fn sandbox_outcome_projects_only_exact_retry_and_return() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(EncounterResolution(Some(EncounterOutcome::Victory)))
            .insert_resource(sample_sandbox_session(9_001))
            .init_resource::<OutcomeView>()
            .add_systems(Update, sync_outcome_view);
        app.update();

        let view = app.world().resource::<OutcomeView>();
        assert!(view.visible);
        assert_eq!(
            view.actions
                .iter()
                .map(|action| action.action)
                .collect::<Vec<_>>(),
            vec![OutcomeAction::RetryExact, OutcomeAction::Return]
        );
        assert_eq!(
            view.actions.get(1).map(|action| action.label.as_str()),
            Some("Return to Sandbox")
        );
    }

    #[test]
    fn sandbox_retry_requeues_exact_identity_and_preserves_session() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin))
            .init_state::<Screen>()
            .add_sub_state::<Mode>()
            .init_resource::<MainMenuModel>();
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
        let session = sample_sandbox_session(seed.0);
        let expected_retry = session.launch.loading_input();
        let expected_rules = session.launch.rules.clone();
        app.insert_resource(session.clone());
        app.insert_resource(GameplaySessionOrigin::Sandbox);
        app.add_message::<hex_ui::UiIntent>();
        app.add_systems(Update, handle_outcome_actions);
        app.world_mut()
            .write_message(hex_ui::UiIntent::Outcome(hex_ui::OutcomeIntent::Activate(
                OutcomeAction::RetryExact,
            )));
        app.update();

        let retry = app.world().resource::<crate::scenarios::ScenarioToLoad>();
        assert_eq!(retry, &expected_retry);
        assert_ne!(retry.encounter_override, None);
        let exact = retry
            .encounter_override
            .as_ref()
            .expect("Retry Exact carries deployed encounter");
        let retried_party = exact
            .rosters
            .first()
            .map(|roster| {
                roster
                    .units
                    .iter()
                    .map(|unit| unit.archetype.as_str())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        assert_eq!(retried_party, vec!["hedge-mage", "hedge-mage"]);
        assert_eq!(app.world().resource::<CombatSettings>(), &expected_rules);
        assert!(app
            .world()
            .contains_resource::<FixedSettingsFreeze<CombatSettings>>());
        assert!(app
            .world()
            .contains_resource::<FixedSettingsFreeze<hex_assets::SpellFile>>());
        assert!(app
            .world()
            .contains_resource::<FixedSettingsFreeze<hex_assets::LatticeFile>>());
        assert_eq!(
            app.world().resource::<SandboxSession>().launch,
            session.launch
        );
        assert_eq!(
            app.world().resource::<GameplaySessionOrigin>(),
            &GameplaySessionOrigin::Sandbox
        );
    }

    #[test]
    fn gameplay_returns_to_its_typed_owner() {
        assert_eq!(
            gameplay_return_screen(Some(&GameplaySessionOrigin::Sandbox), true),
            Screen::Sandbox
        );
        assert_eq!(
            gameplay_return_screen(
                Some(&GameplaySessionOrigin::Campaign(CampaignSlotId::Two)),
                false,
            ),
            Screen::Title
        );
    }

    #[test]
    fn terminal_sandbox_return_intent_transitions_the_real_app_back_to_sandbox() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin))
            .insert_state(Screen::Gameplay)
            .add_sub_state::<Mode>()
            .add_message::<hex_ui::UiIntent>()
            .init_resource::<MainMenuModel>()
            .insert_resource(EncounterResolution(Some(EncounterOutcome::Victory)))
            .insert_resource(sample_sandbox_session(9_001))
            .insert_resource(GameplaySessionOrigin::Sandbox)
            .add_systems(Update, handle_outcome_actions);
        app.world_mut()
            .write_message(hex_ui::UiIntent::Outcome(hex_ui::OutcomeIntent::Activate(
                OutcomeAction::Return,
            )));
        app.update();
        app.update();
        assert_eq!(
            *app.world().resource::<State<Screen>>().get(),
            Screen::Sandbox
        );
    }

    #[test]
    fn campaign_return_intent_opens_the_main_menu_root() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin))
            .insert_state(Screen::Gameplay)
            .add_sub_state::<Mode>()
            .add_message::<hex_ui::UiIntent>()
            .init_resource::<MainMenuModel>()
            .insert_resource(EncounterResolution(Some(EncounterOutcome::Victory)))
            .insert_resource(GameplaySessionOrigin::Campaign(CampaignSlotId::Two))
            .add_systems(Update, handle_outcome_actions);
        app.world_mut()
            .resource_mut::<MainMenuModel>()
            .show(MainMenuRoute::Campaign);
        app.world_mut()
            .write_message(hex_ui::UiIntent::Outcome(hex_ui::OutcomeIntent::Activate(
                OutcomeAction::Return,
            )));

        app.update();
        app.update();

        assert_eq!(
            *app.world().resource::<State<Screen>>().get(),
            Screen::Title
        );
        assert_eq!(
            app.world().resource::<MainMenuModel>().route,
            MainMenuRoute::Root
        );
    }

    #[test]
    fn campaign_keyboard_return_opens_the_main_menu_root() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin))
            .insert_state(Screen::Gameplay)
            .insert_state(Pause(false))
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<InputBindings>()
            .init_resource::<HudState>()
            .init_resource::<MainMenuModel>()
            .add_systems(Update, handle_input);
        app.world_mut()
            .resource_mut::<MainMenuModel>()
            .show(MainMenuRoute::Campaign);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Backspace);

        app.update();
        app.update();

        assert_eq!(
            *app.world().resource::<State<Screen>>().get(),
            Screen::Title
        );
        assert_eq!(
            app.world().resource::<MainMenuModel>().route,
            MainMenuRoute::Root
        );
    }

    #[test]
    fn escape_closes_an_ordinary_main_view_before_default_pause() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin))
            .insert_state(Screen::Gameplay)
            .insert_state(Pause(false))
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<InputBindings>()
            .init_resource::<HudState>()
            .init_resource::<MainMenuModel>()
            .add_systems(Update, handle_input);
        let result = app
            .world_mut()
            .resource_mut::<HudState>()
            .open_formation(hex_gameplay_model::HudContext::default());
        assert_eq!(result, HudActionResult::RuntimeChanged);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Escape);

        app.update();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear();
        app.update();

        assert_eq!(
            app.world().resource::<HudState>().stored_main_view(),
            hex_gameplay_model::MainViewDestination::Closed
        );
        assert_eq!(*app.world().resource::<State<Pause>>().get(), Pause(false));
    }
}
