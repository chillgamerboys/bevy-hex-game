//! Party and formation presentation from an immutable gameplay projection.

use bevy::input_focus::tab_navigation::TabIndex;
use bevy::prelude::*;
use hex_core::Screen;

use crate::{
    blurb, body_text_role, heading, owner_resolved_control_role, responsive_control_role,
    HudElement, PartyIntent, PartyView, ResolvedUiMetrics, UiAssets, UiHudSetup, UiIntent,
    UiRegionRole, UiSystems, ACCENT, ACCENT_EDGE, BLURB_SIZE, EDGE, LABEL, PANEL_BG,
};

#[derive(Component)]
struct PartyBody;

#[derive(Component)]
struct FormationPanel;

#[derive(Component)]
struct FormationBody;

#[derive(Component, Debug, Clone, PartialEq, Eq)]
struct PartyControl(PartyIntent);

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        OnEnter(Screen::Gameplay),
        spawn_panels.in_set(UiHudSetup::Panels),
    )
    .add_systems(
        Update,
        (rebuild, emit_intents.in_set(UiSystems::EmitIntents)).run_if(in_state(Screen::Gameplay)),
    );
}

fn spawn_panels(
    mut commands: Commands,
    assets: Res<UiAssets>,
    regions: Query<(Entity, &UiRegionRole)>,
) {
    let party = commands
        .spawn((
            Name::new("Party Strip"),
            HudElement,
            Node {
                width: Val::Percent(100.0),
                padding: UiRect::all(Val::Px(10.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(10.0)),
                ..default()
            },
            BorderColor::all(EDGE),
            BackgroundColor(PANEL_BG),
        ))
        .with_children(|root| {
            root.spawn(heading(&assets, "party"));
            root.spawn(blurb(&assets, "ALLIES · keys 1–6"));
            root.spawn((
                PartyBody,
                Node {
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(6.0),
                    ..default()
                },
                Pickable::IGNORE,
            ));
        })
        .id();
    let formation = commands
        .spawn((
            Name::new("Formation Panel"),
            FormationPanel,
            HudElement,
            Node {
                display: Display::None,
                width: Val::Percent(100.0),
                padding: UiRect::all(Val::Px(12.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(10.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(10.0),
                ..default()
            },
            BorderColor::all(EDGE),
            BackgroundColor(PANEL_BG),
        ))
        .with_children(|root| {
            root.spawn(heading(&assets, "formation"));
            root.spawn(blurb(
                &assets,
                "Select an ally, then choose a slot. Occupied slots swap.",
            ));
            root.spawn((FormationBody, Node::default(), Pickable::IGNORE));
        })
        .id();
    if let Some(region) = region(UiRegionRole::Party, &regions) {
        commands.entity(region).add_child(party);
    }
    if let Some(region) = region(UiRegionRole::Inspector, &regions) {
        commands.entity(region).add_child(formation);
    }
}

fn region(wanted: UiRegionRole, regions: &Query<(Entity, &UiRegionRole)>) -> Option<Entity> {
    regions
        .iter()
        .find_map(|(entity, role)| (*role == wanted).then_some(entity))
}

fn rebuild(
    mut commands: Commands,
    view: Res<PartyView>,
    party_bodies: Query<Entity, With<PartyBody>>,
    formation_bodies: Query<Entity, With<FormationBody>>,
    mut formation_panels: Query<&mut Node, With<FormationPanel>>,
    metrics: Res<ResolvedUiMetrics>,
    assets: Res<UiAssets>,
) {
    if !view.is_changed() && !metrics.is_changed() {
        return;
    }
    if let Ok(mut panel) = formation_panels.single_mut() {
        panel.display = if view.formation_visible {
            Display::Flex
        } else {
            Display::None
        };
    }
    if let Ok(body) = party_bodies.single() {
        commands.entity(body).despawn_related::<Children>();
        commands.entity(body).with_children(|members| {
            for member in &view.members {
                let border = if member.active || member.selected {
                    ACCENT
                } else {
                    EDGE
                };
                let background = if member.active {
                    Color::srgba(0.93, 0.79, 0.46, 0.28)
                } else if member.selected {
                    Color::srgba(0.93, 0.79, 0.46, 0.16)
                } else {
                    Color::srgba(1.0, 1.0, 1.0, 0.07)
                };
                members
                    .spawn((
                        control_button(
                            format!("Party Member {}", member.slot + 1),
                            member.label.clone(),
                            Val::Percent(100.0),
                        ),
                        PartyControl(PartyIntent::SelectMember(member.slot)),
                        BorderColor::all(border),
                        BackgroundColor(background),
                    ))
                    .with_child(body_text(&assets, member.label.clone()));
            }
        });
    }
    if let Ok(body) = formation_bodies.single() {
        commands.entity(body).despawn_related::<Children>();
        commands.entity(body).with_children(|formation| {
            formation
                .spawn((
                    control_button(
                        "Party Movement Mode",
                        view.movement_mode.clone(),
                        Val::Percent(100.0),
                    ),
                    PartyControl(PartyIntent::ToggleMovementMode),
                    BorderColor::all(ACCENT_EDGE),
                    BackgroundColor(Color::srgba(0.93, 0.79, 0.46, 0.16)),
                ))
                .with_child(body_text(&assets, view.movement_mode.clone()));
            formation
                .spawn((
                    control_button("Party Rest", "REST PARTY · R", Val::Percent(100.0)),
                    PartyControl(PartyIntent::Rest),
                    BorderColor::all(EDGE),
                    BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.07)),
                ))
                .with_child(body_text(&assets, "REST PARTY · R"));
            formation
                .spawn((
                    Node {
                        width: Val::Percent(100.0),
                        flex_direction: FlexDirection::Row,
                        flex_wrap: FlexWrap::Wrap,
                        column_gap: Val::Px(6.0),
                        row_gap: Val::Px(6.0),
                        ..default()
                    },
                    Pickable::IGNORE,
                ))
                .with_children(|presets| {
                    for preset in &view.presets {
                        presets
                            .spawn((
                                control_button(
                                    format!("Formation Preset {preset}"),
                                    preset.clone(),
                                    Val::Auto,
                                ),
                                PartyControl(PartyIntent::SelectPreset(preset.clone())),
                                BorderColor::all(EDGE),
                                BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.07)),
                            ))
                            .with_child(body_text(&assets, preset.clone()));
                    }
                });
            formation.spawn(blurb(&assets, "ASSIGNMENT GRID · ◆ anchor"));
            spawn_slot_grid(formation, &view, &assets, metrics.control_scale.max(1.0));
        });
    }
}

fn control_button(
    name: impl Into<String>,
    accessible: impl Into<String>,
    width: Val,
) -> impl Bundle {
    (
        Name::new(name.into()),
        AccessibleLabel::new(accessible),
        Button,
        TabIndex(0),
        responsive_control_role(),
        Node {
            width,
            min_height: Val::Px(48.0),
            padding: UiRect::axes(Val::Px(9.0), Val::Px(7.0)),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(6.0)),
            ..default()
        },
    )
}

fn body_text(assets: &UiAssets, text: impl Into<String>) -> impl Bundle {
    (
        Text::new(text),
        body_text_role(),
        TextFont {
            font: assets.body.clone().into(),
            ..TextFont::from_font_size(BLURB_SIZE)
        },
        TextColor(LABEL),
        Pickable::IGNORE,
    )
}

#[expect(
    clippy::cast_precision_loss,
    reason = "formation offsets are content-limited to a six-cell miniature"
)]
fn spawn_slot_grid(
    parent: &mut ChildSpawnerCommands,
    view: &PartyView,
    assets: &UiAssets,
    semantic_control_scale: f32,
) {
    let positions: Vec<_> = view
        .slots
        .iter()
        .map(|slot| {
            (
                slot,
                (slot.offset.x() * 20 + slot.offset.y() * 10) as f32 * semantic_control_scale,
                (slot.offset.y() * 18) as f32 * semantic_control_scale,
            )
        })
        .collect();
    let bounds: Option<(f32, f32, f32, f32)> = positions.iter().fold(None, |bounds, (_, x, y)| {
        Some(match bounds {
            None => (*x, *x, *y, *y),
            Some((min_x, max_x, min_y, max_y)) => {
                (min_x.min(*x), max_x.max(*x), min_y.min(*y), max_y.max(*y))
            }
        })
    });
    let Some((min_x, max_x, min_y, max_y)) = bounds else {
        return;
    };
    parent
        .spawn((
            Name::new("Formation mini-grid"),
            Node {
                width: Val::Px(max_x - min_x + 44.0 * semantic_control_scale),
                height: Val::Px(max_y - min_y + 44.0 * semantic_control_scale),
                position_type: PositionType::Relative,
                align_self: AlignSelf::Center,
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|grid| {
            for (slot, x, y) in positions {
                let label = if slot.anchor { "◆" } else { "⬡" };
                grid.spawn((
                    Name::new(format!(
                        "Formation Slot ({}, {})",
                        slot.offset.x(),
                        slot.offset.y()
                    )),
                    AccessibleLabel::new(format!(
                        "Formation slot {}, {}{}",
                        slot.offset.x(),
                        slot.offset.y(),
                        if slot.anchor { " · anchor" } else { "" }
                    )),
                    Button,
                    TabIndex(0),
                    owner_resolved_control_role(),
                    PartyControl(PartyIntent::AssignSlot(slot.offset)),
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(x - min_x),
                        top: Val::Px(y - min_y),
                        width: Val::Px(44.0 * semantic_control_scale),
                        height: Val::Px(44.0 * semantic_control_scale),
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        border: UiRect::all(Val::Px(1.0)),
                        border_radius: BorderRadius::all(Val::Px(4.0)),
                        ..default()
                    },
                    BorderColor::all(EDGE),
                    BackgroundColor(if slot.anchor {
                        Color::srgba(0.93, 0.79, 0.46, 0.45)
                    } else {
                        Color::srgba(1.0, 1.0, 1.0, 0.1)
                    }),
                ))
                .with_child(body_text(assets, label));
            }
        });
}

fn emit_intents(
    controls: Query<(&Interaction, &PartyControl), Changed<Interaction>>,
    mut intents: MessageWriter<UiIntent>,
) {
    for (interaction, control) in &controls {
        if *interaction == Interaction::Pressed {
            intents.write(UiIntent::Party(control.0.clone()));
        }
    }
}
