//! Party and formation presentation from an immutable gameplay projection.

use bevy::input_focus::tab_navigation::TabIndex;
use bevy::prelude::*;
use hex_core::Screen;
use hex_gameplay_model::MainViewDestination;

use crate::{
    blurb, hud_heading, hud_text_role, owner_resolved_control_role, responsive_control_role,
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
        (
            rebuild.in_set(UiSystems::Render),
            emit_intents.in_set(UiSystems::EmitIntents),
        )
            .run_if(in_state(Screen::Gameplay)),
    );
}

fn spawn_panels(
    mut commands: Commands,
    assets: Res<UiAssets>,
    regions: Query<(Entity, &UiRegionRole)>,
) {
    let party = commands
        .spawn((
            Name::new("Party Panel"),
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
            root.spawn(hud_heading(&assets, "party"));
            root.spawn(blurb(&assets, "ALLIES"));
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
            root.spawn(hud_heading(&assets, "formation"));
            root.spawn(blurb(
                &assets,
                "Select an ally, then choose a slot. Occupied slots swap.",
            ));
            root.spawn((
                FormationBody,
                Node {
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(8.0),
                    ..default()
                },
                Pickable::IGNORE,
            ));
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
    review: Option<Res<crate::review::UiReviewPresentation>>,
    chrome: Res<crate::GameplayChromeView>,
    party_bodies: Query<Entity, With<PartyBody>>,
    formation_bodies: Query<Entity, With<FormationBody>>,
    mut formation_panels: Query<&mut Node, With<FormationPanel>>,
    metrics: Res<ResolvedUiMetrics>,
    assets: Res<UiAssets>,
) {
    let review_changed = review.as_ref().is_some_and(|review| review.is_changed());
    if !view.is_changed() && !review_changed && !metrics.is_changed() && !chrome.is_changed() {
        return;
    }
    let view = review
        .as_ref()
        .and_then(|review| review.party.as_ref())
        .unwrap_or(view.as_ref());
    if let Ok(mut panel) = formation_panels.single_mut() {
        panel.display = if view.formation_visible
            && matches!(chrome.main_view, MainViewDestination::Formation)
        {
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
                            crate::UiVisibilityRequirement::Immediate,
                        ),
                        PartyControl(PartyIntent::ActivateMember(member.slot)),
                        BorderColor::all(border),
                        BackgroundColor(background),
                    ))
                    .with_children(|card| {
                        card.spawn(body_text(&assets, member.label.clone()));
                        if !member.cells.is_empty() {
                            crate::sandbox::spawn_mini_lattice(card, &assets, &member.cells);
                        }
                    });
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
                        crate::UiVisibilityRequirement::Scrollable,
                    ),
                    PartyControl(PartyIntent::ToggleMovementMode),
                    BorderColor::all(ACCENT_EDGE),
                    BackgroundColor(Color::srgba(0.93, 0.79, 0.46, 0.16)),
                ))
                .with_child(body_text(&assets, view.movement_mode.clone()));
            formation
                .spawn((
                    control_button(
                        "Party Rest",
                        "REST PARTY",
                        Val::Percent(100.0),
                        crate::UiVisibilityRequirement::Scrollable,
                    ),
                    PartyControl(PartyIntent::Rest),
                    BorderColor::all(EDGE),
                    BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.07)),
                ))
                .with_child(body_text(&assets, "REST PARTY"));
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
                                    crate::UiVisibilityRequirement::Scrollable,
                                ),
                                PartyControl(PartyIntent::SelectPreset(preset.clone())),
                                BorderColor::all(EDGE),
                                BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.07)),
                            ))
                            .with_child(body_text(&assets, preset.clone()));
                    }
                });
            formation.spawn(blurb(&assets, "ASSIGNMENT GRID · ◆ anchor"));
            spawn_slot_grid(formation, view, &assets, metrics.control_scale.max(1.0));
        });
    }
}

fn control_button(
    name: impl Into<String>,
    accessible: impl Into<String>,
    width: Val,
    visibility_requirement: crate::UiVisibilityRequirement,
) -> impl Bundle {
    (
        Name::new(name.into()),
        AccessibleLabel::new(accessible),
        Button,
        TabIndex(0),
        visibility_requirement,
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
        hud_text_role(),
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
    const SLOT_STEP: i32 = 48;
    const ROW_OFFSET: i32 = SLOT_STEP / 2;

    let positions: Vec<_> = view
        .slots
        .iter()
        .map(|slot| {
            (
                slot,
                (slot.offset.x() * SLOT_STEP + slot.offset.y() * ROW_OFFSET) as f32
                    * semantic_control_scale,
                (slot.offset.y() * SLOT_STEP) as f32 * semantic_control_scale,
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
                let label = if slot.anchor { "◆" } else { "◇" };
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
                    crate::UiVisibilityRequirement::Scrollable,
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

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::*;

    fn formation_app(width: u32, height: u32, mode: crate::UiScaleMode) -> App {
        let mut app = App::new();
        app.add_plugins(crate::test_support::HeadlessUiPlugin::new(width, height));
        app.world_mut()
            .insert_resource(crate::UiScalePreference(mode));
        app.world_mut().insert_resource(PartyView {
            members: (0..6)
                .map(|slot| crate::PartyMemberView {
                    slot,
                    label: format!("ALLY {} · formation member", slot + 1),
                    cells: Vec::new(),
                    active: slot == 0,
                    selected: slot == 0,
                })
                .collect(),
            formation_visible: true,
            movement_mode: "GROUP MOVE · G".to_owned(),
            presets: ["Column", "Compact", "Wedge"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            slots: [(-1, 1), (0, -1), (0, 0), (0, 1), (1, -1), (1, 0)]
                .into_iter()
                .map(|(q, r)| crate::FormationSlotView {
                    offset: hex_core::HexCoord::from_axial(q, r),
                    anchor: q == 0 && r == 0,
                })
                .collect(),
        });
        app.world_mut().insert_resource(crate::GameplayChromeView {
            party_shown: false,
            initiative_shown: false,
            activity_shown: false,
            action_bar_shown: false,
            main_view: MainViewDestination::Formation,
            terrain_health_shown: true,
            encounter_complete: false,
        });
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Gameplay);

        for _ in 0..8 {
            app.update();
        }
        app
    }

    #[test]
    fn live_six_member_formation_controls_fit_the_standard_inspector() {
        let mut app = formation_app(1920, 1080, crate::UiScaleMode::Auto);
        let snapshot = crate::test_support::ui_tree_snapshot(app.world_mut());
        assert!(
            snapshot.layout_issues().is_empty(),
            "live formation controls must remain reachable: {:?}",
            snapshot.layout_issues()
        );
        let grid = snapshot
            .nodes
            .iter()
            .find(|node| node.name == "Formation mini-grid")
            .expect("the live formation grid must be presented");
        assert!(grid.fully_visible, "formation grid must fit: {grid:?}");
    }

    #[test]
    fn enlarged_compact_formation_keeps_the_final_slot_keyboard_reachable() {
        use bevy::input_focus::InputFocus;

        let mut app = formation_app(1280, 720, crate::UiScaleMode::Percent200);
        let entity_named = |app: &mut App, wanted: &str| {
            app.world_mut()
                .query::<(Entity, &Name)>()
                .iter(app.world())
                .find_map(|(entity, name)| (name.as_str() == wanted).then_some(entity))
                .unwrap_or_else(|| panic!("missing {wanted:?}"))
        };
        let final_slot_name = "Formation Slot (1, 0)";
        let final_slot_entity = entity_named(&mut app, final_slot_name);
        let initial = crate::test_support::ui_tree_snapshot(app.world_mut());
        let formation_controls = initial
            .nodes
            .iter()
            .filter(|node| {
                matches!(node.name.as_str(), "Party Movement Mode" | "Party Rest")
                    || node.name.starts_with("Formation Preset ")
                    || node.name.starts_with("Formation Slot (")
            })
            .collect::<Vec<_>>();
        assert_eq!(
            formation_controls.len(),
            11,
            "the populated fixture must cover every secondary formation control"
        );
        for control in formation_controls {
            assert_eq!(
                control.visibility_requirement,
                Some(crate::UiVisibilityRequirement::Scrollable),
                "secondary formation controls must opt into the Inspector's scroll contract: {control:?}"
            );
            assert!(
                control.scroll_reachable,
                "secondary formation controls must have a complete Inspector scroll route: {control:?}"
            );
        }
        let final_slot = initial
            .nodes
            .iter()
            .find(|node| node.name == final_slot_name)
            .expect("the populated formation must expose its final slot");
        assert!(
            final_slot.fully_visible
                && final_slot.in_focus_order
                && final_slot.keyboard_reachable == Some(true),
            "the full-screen Compact Main View must expose the final formation slot without clipping: {final_slot:?}"
        );

        app.insert_resource(InputFocus::from_entity(final_slot_entity));
        for _ in 0..3 {
            app.update();
        }
        let focused = crate::test_support::ui_tree_snapshot(app.world_mut());
        let final_slot = focused
            .nodes
            .iter()
            .find(|node| node.name == final_slot_name)
            .expect("the focused formation slot remains presented");
        assert!(
            final_slot.fully_visible && final_slot.focused,
            "the complete slot and focus ring must be visible after keyboard navigation: {final_slot:?}"
        );
    }
}
