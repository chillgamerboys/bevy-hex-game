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
use hex_assets::FormationCatalog;
use hex_combat::{EncounterOutcome, EncounterResolution, Turn, TurnOrder};
use hex_core::{
    CommandQueue, ControlOwner, GameCommand, GameplaySetup, HexCoord, IssuedCommand, Mode,
    PartyFormation, PartyMovementMode, Pause, PendingDecision, Screen, UnitId,
};
use hex_lattice::{LatticeSpec, LatticeState};
use hex_units::{Archetype, Downed, Party, Player, Selected, UnitRegistry};

use super::{despawn_screen, DespawnOnExit};
use crate::menus::widgets::{blurb, heading, row_button, UiAssets};
use crate::readouts::HudElement;
use crate::scenarios::ActiveScenario;

pub(super) fn plugin(app: &mut App) {
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
            .run_if(hex_combat::encounter_unresolved),
    );
    app.add_systems(Update, update_hud.run_if(in_state(Screen::Gameplay)));
    app.add_systems(
        Update,
        (handle_party_strip, update_party_strip)
            .chain()
            .run_if(in_state(Screen::Gameplay)),
    );
    app.add_systems(
        Update,
        (sync_outcome_modal, handle_outcome_actions)
            .chain()
            .run_if(in_state(Screen::Gameplay)),
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
            spawn_hud,
            spawn_party_strip.in_set(GameplaySetup::View),
        ),
    );
    app.add_systems(OnExit(Screen::Gameplay), despawn_screen(Screen::Gameplay));
}

/// Marks the HUD line so it can be rewritten as the mode changes.
#[derive(Component)]
struct HudText;

#[derive(Component)]
struct PartyStrip;

#[derive(Component)]
struct PartyMemberButton(usize);

#[derive(Component)]
struct PartyPresetButton(String);

#[derive(Component)]
struct PartySlotButton(HexCoord);

#[derive(Component)]
struct PartyModeButton;

#[derive(Component)]
struct PartyModeText;

#[derive(Component)]
struct PartyRestButton;

#[derive(Component)]
struct OutcomeModal;

#[derive(Component, Clone, Copy)]
enum OutcomeAction {
    Continue,
    Retry,
    ReturnTitle,
}

#[expect(
    clippy::cast_precision_loss,
    reason = "formation offsets are content-limited to a six-cell miniature"
)]
fn spawn_party_strip(
    mut commands: Commands,
    assets: Res<UiAssets>,
    formations: Res<FormationCatalog>,
) {
    let mut offered_slots: Vec<HexCoord> = formations
        .presets
        .iter()
        .flat_map(|preset| preset.slots.iter().map(|slot| slot.offset))
        .collect();
    offered_slots.sort_unstable();
    offered_slots.dedup();
    let slot_pixels: Vec<(HexCoord, f32, f32)> = offered_slots
        .iter()
        .map(|offset| {
            (
                *offset,
                (offset.x() * 20 + offset.y() * 10) as f32,
                (offset.y() * 18) as f32,
            )
        })
        .collect();
    let min_slot_x = slot_pixels
        .iter()
        .map(|(_, x, _)| *x)
        .fold(f32::INFINITY, f32::min);
    let max_slot_x = slot_pixels
        .iter()
        .map(|(_, x, _)| *x)
        .fold(f32::NEG_INFINITY, f32::max);
    let min_slot_y = slot_pixels
        .iter()
        .map(|(_, _, y)| *y)
        .fold(f32::INFINITY, f32::min);
    let max_slot_y = slot_pixels
        .iter()
        .map(|(_, _, y)| *y)
        .fold(f32::NEG_INFINITY, f32::max);
    commands
        .spawn((
            Name::new("Party Strip"),
            PartyStrip,
            HudElement,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(12.0),
                left: Val::Px(190.0),
                right: Val::Px(190.0),
                min_height: Val::Px(112.0),
                padding: UiRect::all(Val::Px(8.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(5.0),
                border_radius: BorderRadius::all(Val::Px(5.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.03, 0.045, 0.82)),
            DespawnOnExit(Screen::Gameplay),
        ))
        .with_children(|root| {
            root.spawn((
                Text::new("PARTY"),
                TextFont {
                    font: assets.display.clone().into(),
                    ..TextFont::from_font_size(15.0)
                },
                TextColor(Color::srgb(0.93, 0.79, 0.46)),
            ));
            root.spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(4.0),
                    ..default()
                },
                BackgroundColor(Color::NONE),
            ))
            .with_children(|members| {
                for index in 0..6 {
                    members
                        .spawn((
                            Name::new(format!("Party Member {}", index + 1)),
                            Button,
                            PartyMemberButton(index),
                            Node {
                                width: Val::Percent(16.0),
                                flex_grow: 1.0,
                                padding: UiRect::axes(Val::Px(7.0), Val::Px(4.0)),
                                border: UiRect::all(Val::Px(1.0)),
                                ..default()
                            },
                            BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.13)),
                            BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.07)),
                        ))
                        .with_child((
                            Text::new(format!("{}  —", index + 1)),
                            TextFont {
                                font: assets.body.clone().into(),
                                ..TextFont::from_font_size(11.0)
                            },
                            TextColor(Color::srgb(0.94, 0.94, 0.95)),
                        ));
                }
            });
            root.spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(5.0),
                    ..default()
                },
                BackgroundColor(Color::NONE),
            ))
            .with_children(|controls| {
                controls
                    .spawn((
                        Name::new("Party Movement Mode"),
                        Button,
                        PartyModeButton,
                        Node {
                            padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.93, 0.79, 0.46, 0.2)),
                    ))
                    .with_child((
                        PartyModeText,
                        Text::new("Movement: Group"),
                        TextFont {
                            font: assets.body.clone().into(),
                            ..TextFont::from_font_size(11.0)
                        },
                    ));
                controls
                    .spawn((
                        Name::new("Party Rest"),
                        Button,
                        PartyRestButton,
                        Node {
                            padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.07)),
                    ))
                    .with_child((
                        Text::new("Rest  R"),
                        TextFont {
                            font: assets.body.clone().into(),
                            ..TextFont::from_font_size(11.0)
                        },
                    ));
                for preset in &formations.presets {
                    controls
                        .spawn((
                            Name::new(format!("Formation Preset {}", preset.name)),
                            Button,
                            PartyPresetButton(preset.name.clone()),
                            Node {
                                padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.07)),
                        ))
                        .with_child((
                            Text::new(preset.name.clone()),
                            TextFont {
                                font: assets.body.clone().into(),
                                ..TextFont::from_font_size(11.0)
                            },
                        ));
                }
                controls
                    .spawn((
                        Name::new("Formation mini-grid"),
                        Node {
                            width: Val::Px(max_slot_x - min_slot_x + 24.0),
                            height: Val::Px(max_slot_y - min_slot_y + 24.0),
                            position_type: PositionType::Relative,
                            ..default()
                        },
                        BackgroundColor(Color::NONE),
                    ))
                    .with_children(|grid| {
                        for (offset, x, y) in &slot_pixels {
                            grid.spawn((
                                Name::new(format!(
                                    "Formation Slot ({}, {})",
                                    offset.x(),
                                    offset.y()
                                )),
                                Button,
                                PartySlotButton(*offset),
                                Node {
                                    position_type: PositionType::Absolute,
                                    left: Val::Px(x - min_slot_x),
                                    top: Val::Px(y - min_slot_y),
                                    width: Val::Px(24.0),
                                    height: Val::Px(22.0),
                                    align_items: AlignItems::Center,
                                    justify_content: JustifyContent::Center,
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.1)),
                            ))
                            .with_child((
                                Text::new("⬡"),
                                TextFont {
                                    font: assets.body.clone().into(),
                                    ..TextFont::from_font_size(12.0)
                                },
                            ));
                        }
                    });
            });
        });
}

fn handle_party_strip(
    mut commands: Commands,
    mode: Res<State<Mode>>,
    party: Res<Party>,
    registry: Res<UnitRegistry>,
    formations: Res<FormationCatalog>,
    mut formation: ResMut<PartyFormation>,
    member_clicks: Query<(&Interaction, &PartyMemberButton), Changed<Interaction>>,
    mode_clicks: Query<&Interaction, (Changed<Interaction>, With<PartyModeButton>)>,
    preset_clicks: Query<(&Interaction, &PartyPresetButton), Changed<Interaction>>,
    slot_clicks: Query<(&Interaction, &PartySlotButton), Changed<Interaction>>,
    rest_clicks: Query<&Interaction, (Changed<Interaction>, With<PartyRestButton>)>,
    keys: Res<ButtonInput<KeyCode>>,
    mut queue: ResMut<CommandQueue>,
    selected: Query<(Entity, &UnitId), (With<Player>, With<Selected>)>,
    owners: Query<&ControlOwner>,
) {
    if *mode.get() != Mode::Exploring {
        return;
    }
    for (interaction, button) in &member_clicks {
        if *interaction != Interaction::Pressed {
            continue;
        }
        if let Some(entity) = party
            .members
            .get(button.0)
            .and_then(|unit| registry.entity_of(*unit))
        {
            for (old, _) in &selected {
                if old != entity {
                    commands.entity(old).remove::<Selected>();
                }
            }
            commands.entity(entity).insert(Selected);
        }
    }
    if mode_clicks
        .iter()
        .any(|interaction| *interaction == Interaction::Pressed)
    {
        formation.mode = match formation.mode {
            PartyMovementMode::Group => PartyMovementMode::Solo,
            PartyMovementMode::Solo => PartyMovementMode::Group,
        };
    }
    for (interaction, button) in &preset_clicks {
        if *interaction == Interaction::Pressed {
            if let Some(preset) = formations.get(&button.0) {
                formation.select_preset(preset, &party.members);
            }
        }
    }
    let selected_info = selected.iter().next().map(|(_, unit)| *unit);
    let selected = selected_info;
    for (interaction, button) in &slot_clicks {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let Some(member) = selected else {
            continue;
        };
        let Some(preset) = formations.get(&formation.preset) else {
            continue;
        };
        if preset.slots.iter().any(|slot| slot.offset == button.0) {
            let _ = formation.assign(member, button.0);
            formation.fill_unassigned(preset, &party.members);
        }
    }
    let rest_requested = keys.just_pressed(KeyCode::KeyR)
        || rest_clicks
            .iter()
            .any(|interaction| *interaction == Interaction::Pressed);
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

fn update_party_strip(
    party: Res<Party>,
    registry: Res<UnitRegistry>,
    formations: Res<FormationCatalog>,
    formation: Res<PartyFormation>,
    units: Query<(
        &UnitId,
        &Archetype,
        Option<&LatticeSpec>,
        Option<&LatticeState>,
        Has<Downed>,
        Has<Selected>,
    )>,
    mut members: Query<(&PartyMemberButton, &Children, &mut BackgroundColor)>,
    mut slots: Query<
        (
            &PartySlotButton,
            &Children,
            &mut Visibility,
            &mut BackgroundColor,
        ),
        Without<PartyMemberButton>,
    >,
    mut modes: Query<&mut Text, With<PartyModeText>>,
    mut texts: Query<&mut Text, Without<PartyModeText>>,
) {
    let anchor = formations
        .get(&formation.preset)
        .and_then(|preset| formation.anchor_member(preset));
    for (button, children, mut color) in &mut members {
        let Some(&member) = party.members.get(button.0) else {
            continue;
        };
        let Some(entity) = registry.entity_of(member) else {
            continue;
        };
        let Ok((id, archetype, spec, state, downed, selected)) = units.get(entity) else {
            continue;
        };
        let condition = spec.zip(state).map_or_else(String::new, |(spec, state)| {
            let total = spec.cells().count();
            let live = spec
                .cells()
                .filter(|(coord, _)| !state.is_disabled(*coord))
                .count();
            format!(" {live}/{total}")
        });
        let status = format!(
            "{}  {} #{}{}{}{}",
            button.0 + 1,
            archetype.0,
            id.0,
            condition,
            if downed { " DOWN" } else { "" },
            if anchor == Some(*id) { " ◆" } else { "" }
        );
        if let Some(child) = children.first() {
            if let Ok(mut text) = texts.get_mut(*child) {
                text.0 = status;
            }
        }
        color.0 = if selected {
            Color::srgba(0.93, 0.79, 0.46, 0.25)
        } else {
            Color::srgba(1.0, 1.0, 1.0, 0.07)
        };
    }
    for mut text in &mut modes {
        text.0 = format!("Movement: {:?}", formation.mode);
    }
    let active_preset = formations.get(&formation.preset);
    for (slot, children, mut visibility, mut color) in &mut slots {
        let authored = active_preset.and_then(|preset| {
            preset
                .slots
                .iter()
                .find(|authored| authored.offset == slot.0)
        });
        *visibility = if authored.is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        color.0 = if authored.is_some_and(|authored| authored.anchor) {
            Color::srgba(0.93, 0.79, 0.46, 0.45)
        } else {
            Color::srgba(1.0, 1.0, 1.0, 0.1)
        };
        if let Some(child) = children.first() {
            if let Ok(mut text) = texts.get_mut(*child) {
                text.0 = if authored.is_some_and(|authored| authored.anchor) {
                    "◆".to_owned()
                } else {
                    "⬡".to_owned()
                };
            }
        }
    }
}

/// Controls are otherwise undiscoverable — there is no manual and no tutorial.
fn spawn_hud(mut commands: Commands, assets: Res<UiAssets>) {
    commands
        .spawn((
            Name::new("Gameplay HUD"),
            HudElement,
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(12.0),
                left: Val::Px(12.0),
                right: Val::Px(12.0),
                padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                border_radius: BorderRadius::all(Val::Px(4.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.03, 0.04, 0.05, 0.78)),
            // Without this the HUD swallows clicks on any tile behind it, and
            // click-to-move silently stops working along the bottom edge.
            Pickable::IGNORE,
            DespawnOnExit(Screen::Gameplay),
        ))
        .with_children(|parent| {
            parent.spawn((
                HudText,
                Text::new(exploring_hint()),
                TextFont {
                    font: assets.body.clone().into(),
                    ..TextFont::from_font_size(14.0)
                },
                TextColor(Color::srgb(0.94, 0.94, 0.94)),
                Pickable::IGNORE,
            ));
        });
}

fn sync_outcome_modal(
    mut commands: Commands,
    resolution: Res<EncounterResolution>,
    existing: Query<Entity, With<OutcomeModal>>,
    assets: Res<UiAssets>,
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
                        width: Val::Px(430.0),
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
                    panel
                        .spawn(Node {
                            flex_direction: FlexDirection::Row,
                            column_gap: Val::Px(10.0),
                            ..default()
                        })
                        .with_children(|buttons| {
                            let primary = match outcome {
                                EncounterOutcome::Victory => (OutcomeAction::Continue, "Continue"),
                                EncounterOutcome::Defeat => (OutcomeAction::Retry, "Retry"),
                            };
                            buttons
                                .spawn((row_button(primary.1, 150.0), primary.0))
                                .with_child(blurb(&assets, primary.1));
                            buttons
                                .spawn((
                                    row_button("Return to Title", 150.0),
                                    OutcomeAction::ReturnTitle,
                                ))
                                .with_child(blurb(&assets, "Return to Title"));
                        });
                });
        });
}

fn handle_outcome_actions(
    clicked: Query<(&Interaction, &OutcomeAction), Changed<Interaction>>,
    resolution: Res<EncounterResolution>,
    active: Option<Res<ActiveScenario>>,
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
                next_mode.set(Mode::Exploring);
            }
            (OutcomeAction::Retry, EncounterOutcome::Defeat) => {
                let Some(active) = active.as_deref() else {
                    error!("cannot retry: active scenario launch input was not retained");
                    continue;
                };
                commands.insert_resource(active.0.clone());
                next_screen.set(Screen::Loading);
            }
            (OutcomeAction::ReturnTitle, _) => next_screen.set(Screen::Title),
            _ => {}
        }
    }
}

fn exploring_hint() -> String {
    "EXPLORING   ·   click a tile to move   ·   right-drag to orbit   ·   \
     WASD to pan   ·   scroll to zoom   ·   H hides HUD   ·   ESC to pause"
        .to_owned()
}

/// Rewrites the hint line to say what the game is doing and whose turn it is.
///
/// This compact summary complements the lattice panels, initiative list, and combat
/// log by keeping the current mode, round, and action budget visible at a glance.
fn update_hud(
    mode: Res<State<Mode>>,
    order: Res<TurnOrder>,
    pending: Res<PendingDecision>,
    acting: Query<(Has<Player>, &Turn)>,
    party: Query<(&LatticeSpec, &LatticeState), With<Player>>,
    mut hud: Query<&mut Text, With<HudText>>,
) {
    let Ok(mut text) = hud.single_mut() else {
        return;
    };

    let wanted = match mode.get() {
        Mode::Exploring => exploring_hint(),
        Mode::Combat => {
            // How much movement is left, spelled out. Without it a click refused for
            // being out of range is indistinguishable from a click that did not
            // register — which is precisely the complaint the tinted range answers,
            // and the number is what confirms the tint rather than merely repeating it.
            let (whose, player_turn) = match acting.single() {
                Ok((true, turn)) => (format!("your turn, {} to move", turn.movement_left), true),
                Ok((false, _)) => ("enemy turn".to_owned(), false),
                Err(_) => ("…".to_owned(), false),
            };
            format!(
                "COMBAT   ·   round {}   ·   {}{}   ·   {}   \
                 ·   H hides HUD   ·   ESC to pause",
                order.round + 1,
                whose,
                lattice_readout(&party),
                combat_action_hint(player_turn, &pending)
            )
        }
    };

    if text.0 != wanted {
        text.0 = wanted;
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
fn toggle_reveal_all(keys: Res<ButtonInput<KeyCode>>, mut reveal: ResMut<hex_combat::RevealAll>) {
    if keys.just_pressed(KeyCode::KeyK) {
        reveal.0 = !reveal.0;
        info!("reveal-all {}", if reveal.0 { "on" } else { "off" });
    }
}

/// Your party's hexes still standing, out of the hexes it started with.
///
/// A **sum across the party**, not one unit's lattice, and the label says "party" for
/// that reason: every shipped encounter fields one player unit today, so an unqualified
/// count would read correctly by accident and then quietly become an aggregate the first
/// time somebody adds a second line to a roster — which the encounter files openly invite.
/// Per-unit readouts are the casting-UX ticket's, not this one's.
///
/// Read straight off the components rather than through
/// [`FactionKnowledge`](hex_combat::FactionKnowledge): a faction's knowledge of *itself*
/// is not the question that store answers. It exists to gate what you know about a
/// **hostile** lattice, where seeing a unit reveals nothing about its contents — and
/// routing your own hexes through it would either need a self-view nothing publishes, or
/// teach the next reader that `view()` is how you look at anything, which is exactly the
/// confusion the two-channel split exists to prevent.
///
/// Empty while nothing carries a lattice, so a party of one inert unit reads exactly as
/// it did before this — no readout rather than a zero.
fn lattice_readout(party: &Query<(&LatticeSpec, &LatticeState), With<Player>>) -> String {
    let mut live = 0_usize;
    let mut total = 0_usize;
    for (spec, state) in party {
        // The spec is what says which cells exist; the state only says which of them
        // are down. Counting the state alone would miss every cell that has never been
        // touched, which early in a fight is all of them.
        for (coord, _) in spec.cells() {
            total += 1;
            if !state.is_disabled(coord) {
                live += 1;
            }
        }
    }
    if total == 0 {
        return String::new();
    }
    format!("   ·   party {live}/{total} hexes")
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
    pause: Res<State<Pause>>,
    mut next_pause: ResMut<NextState<Pause>>,
    mut next_screen: ResMut<NextState<Screen>>,
) {
    if keys.just_pressed(KeyCode::Escape) {
        next_pause.set(Pause(!pause.get().0));
    }
    // Backspace rather than Escape, which is taken by pause.
    if keys.just_pressed(KeyCode::Backspace) {
        next_screen.set(Screen::Title);
    }
}

#[cfg(test)]
mod tests {
    use bevy::state::app::StatesPlugin;
    use bevy::MinimalPlugins;
    use hex_assets::ScenarioLibrary;
    use hex_core::ResolvedMapSeed;

    use super::*;

    /// Every layer of the full-width HUD must let world picks pass through.
    ///
    /// Pickability is per entity, so ignoring only the backing node still leaves its
    /// text able to swallow tile clicks.
    #[test]
    fn gameplay_hud_does_not_block_tile_clicks() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(UiAssets {
            display: Handle::default(),
            body: Handle::default(),
            hex_cell: Handle::default(),
        });
        app.add_systems(Startup, spawn_hud);
        app.update();

        let mut roots = app
            .world_mut()
            .query_filtered::<&Pickable, With<BackgroundColor>>();
        assert!(
            roots
                .iter(app.world())
                .any(|pickable| *pickable == Pickable::IGNORE),
            "the HUD backing node blocks world picks"
        );

        let mut labels = app.world_mut().query_filtered::<&Pickable, With<HudText>>();
        assert_eq!(
            labels.iter(app.world()).next(),
            Some(&Pickable::IGNORE),
            "the HUD text blocks world picks"
        );
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
