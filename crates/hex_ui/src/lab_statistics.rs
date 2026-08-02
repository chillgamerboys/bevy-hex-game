//! Live Combat Lab statistics presentation.

use bevy::input_focus::tab_navigation::TabGroup;
use bevy::prelude::*;
use hex_core::Screen;

use crate::{
    blurb, fixed_row_button, hud_heading, layout::is_ultra_constrained, supporting_text_role,
    GameplayChromeView, LabStatisticsIntent, LabStatisticsView, ResolvedUiMetrics, UiAssets,
    UiHudSetup, UiIntent, UiRegionRole, UiSystems, ACCENT_EDGE, LABEL,
};

#[derive(Component)]
struct Drawer;

#[derive(Component)]
struct Body;

#[derive(Component)]
struct Summary;

#[derive(Component, Clone, Copy)]
struct Control(LabStatisticsIntent);

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        OnEnter(Screen::Gameplay),
        spawn.in_set(UiHudSetup::Secondary),
    )
    .add_systems(
        Update,
        (
            (apply_layout, render).chain().in_set(UiSystems::Render),
            emit_intents.in_set(UiSystems::EmitIntents),
        )
            .run_if(in_state(Screen::Gameplay)),
    );
}

fn spawn(mut commands: Commands, assets: Res<UiAssets>, regions: Query<(Entity, &UiRegionRole)>) {
    let Some(inspector) = regions
        .iter()
        .find_map(|(entity, role)| (*role == UiRegionRole::Inspector).then_some(entity))
    else {
        error!(
            "UiHudSetup::Frame did not create the gameplay Inspector; refusing to mount Combat Lab statistics"
        );
        return;
    };
    let drawer = commands
        .spawn((
            Name::new("Combat Lab Live Statistics Drawer"),
            Drawer,
            crate::UiVisibilityRequirement::Scrollable,
            TabGroup::new(15),
            Node {
                position_type: PositionType::Absolute,
                padding: UiRect::all(Val::Px(10.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(7.0),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(7.0)),
                ..default()
            },
            BorderColor::all(ACCENT_EDGE),
            BackgroundColor(Color::srgba(0.02, 0.03, 0.045, 1.0)),
            GlobalZIndex(12),
            Visibility::Hidden,
        ))
        .with_children(|drawer| {
            drawer
                .spawn((
                    fixed_row_button(
                        "Expand or collapse live Combat Lab statistics",
                        250.0,
                        48.0,
                    ),
                    crate::UiVisibilityRequirement::Scrollable,
                    Control(LabStatisticsIntent::Toggle),
                ))
                .with_child(blurb(&assets, "Statistics · Collapse"));
            drawer
                .spawn((
                    Name::new("Combat Lab Statistics Body"),
                    Body,
                    crate::UiVisibilityRequirement::Scrollable,
                    Node {
                        min_height: Val::Auto,
                        flex_grow: 0.0,
                        flex_shrink: 0.0,
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(5.0),
                        ..default()
                    },
                ))
                .with_children(|body| {
                    body.spawn(hud_heading(&assets, "LIVE COMBAT LAB STATISTICS"));
                    body.spawn((
                        Name::new("Combat Lab Statistics Scroll Cue"),
                        AccessibleLabel::new(
                            "More live Combat Lab run details are available below",
                        ),
                        crate::UiVisibilityRequirement::Scrollable,
                        blurb(&assets, "Scroll for full run details ↓"),
                    ));
                    body.spawn((
                        Summary,
                        Text::new("Waiting for canonical combat statistics…"),
                        supporting_text_role(),
                        TextFont {
                            font: assets.body.clone().into(),
                            ..TextFont::from_font_size(18.0)
                        },
                        TextColor(LABEL),
                    ));
                    body.spawn(blurb(
                        &assets,
                        "Totals are gameplay-owned · per-unit and timeline details open in the outcome report.",
                    ));
                    body.spawn((
                        Name::new("Combat Lab Statistics Detail End"),
                        crate::UiVisibilityRequirement::Scrollable,
                        Node {
                            width: Val::Px(1.0),
                            height: Val::Px(1.0),
                            min_height: Val::Px(1.0),
                            flex_shrink: 0.0,
                            ..default()
                        },
                        Pickable::IGNORE,
                    ));
                });
            drawer
                .spawn((
                    fixed_row_button(
                        "End experiment and save the current Combat Lab report",
                        250.0,
                        48.0,
                    ),
                    crate::UiVisibilityRequirement::Scrollable,
                    Control(LabStatisticsIntent::EndExperiment),
                ))
                .with_child(blurb(&assets, "End Experiment"));
        })
        .id();
    commands.entity(inspector).add_child(drawer);
}

fn render(
    view: Res<LabStatisticsView>,
    lattices: Res<crate::GameplayLatticesView>,
    chrome: Res<GameplayChromeView>,
    metrics: Res<ResolvedUiMetrics>,
    review: Option<Res<crate::review::UiReviewPresentation>>,
    added_drawers: Query<(), Added<Drawer>>,
    mut drawers: Query<(&mut Visibility, &mut Node), With<Drawer>>,
    mut bodies: Query<(&mut Visibility, &mut Node), (With<Body>, Without<Drawer>)>,
    mut summaries: Query<&mut Text, With<Summary>>,
    buttons: Query<(&Control, &Children)>,
    mut labels: Query<&mut Text, (Without<Summary>, Without<Drawer>)>,
) {
    let review_changed = review.as_ref().is_some_and(|review| review.is_changed());
    if !view.is_changed()
        && !lattices.is_changed()
        && !chrome.is_changed()
        && !metrics.is_changed()
        && !review_changed
        && added_drawers.is_empty()
    {
        return;
    }
    let view = review
        .as_ref()
        .and_then(|review| review.statistics.as_ref())
        .unwrap_or(view.as_ref());
    let lattices = review
        .as_ref()
        .and_then(|review| review.lattices.as_ref())
        .unwrap_or(lattices.as_ref());
    let chrome = review
        .as_ref()
        .map_or(*chrome, |review| review.effective_chrome(*chrome));
    let show_drawer = view.present
        && view.visible
        && lattices.own.is_some()
        && chrome.shown
        && !chrome.decision_required
        && !chrome.encounter_complete;
    for (mut visibility, mut node) in &mut drawers {
        *visibility = if show_drawer {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        node.display = if show_drawer {
            Display::Flex
        } else {
            Display::None
        };
    }
    for (mut visibility, mut node) in &mut bodies {
        *visibility = if show_drawer && view.expanded {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        node.display = if show_drawer && view.expanded {
            Display::Flex
        } else {
            Display::None
        };
    }
    for mut text in &mut summaries {
        **text = view.text.clone();
    }
    let abbreviated = is_ultra_constrained(*metrics) && view.expanded;
    for (control, children) in &buttons {
        if let Some(child) = children.first() {
            if let Ok(mut text) = labels.get_mut(*child) {
                **text = match control.0 {
                    LabStatisticsIntent::Toggle if abbreviated => "Collapse".to_owned(),
                    LabStatisticsIntent::Toggle if view.expanded => {
                        "Statistics · Collapse".to_owned()
                    }
                    LabStatisticsIntent::Toggle => "Statistics · Expand".to_owned(),
                    LabStatisticsIntent::EndExperiment if abbreviated => "End Lab".to_owned(),
                    LabStatisticsIntent::EndExperiment => "End Experiment".to_owned(),
                };
            }
        }
    }
}

fn apply_layout(
    metrics: Res<ResolvedUiMetrics>,
    added: Query<(), Added<Drawer>>,
    mut drawers: Query<&mut Node, (With<Drawer>, Without<Body>)>,
    mut bodies: Query<&mut Node, (With<Body>, Without<Drawer>)>,
    mut controls: Query<&mut Node, (With<Control>, Without<Drawer>, Without<Body>)>,
) {
    if !metrics.is_changed() && added.is_empty() {
        return;
    }
    let semantic_target = 44.0 * metrics.control_scale.max(1.0);
    for mut control in &mut controls {
        control.height = Val::Px(48.0 * metrics.control_scale.max(1.0));
        control.min_height = Val::Px(semantic_target);
        control.width = Val::Px(250.0 * metrics.control_scale.max(1.0));
        control.min_width = Val::Px(250.0 * metrics.control_scale.max(1.0));
        control.max_width = Val::Auto;
        control.flex_shrink = 0.0;
    }
    for mut body in &mut bodies {
        body.position_type = PositionType::Relative;
        body.top = Val::Auto;
        body.right = Val::Auto;
        body.bottom = Val::Auto;
        body.left = Val::Auto;
        body.width = Val::Auto;
        body.height = Val::Auto;
        body.min_height = Val::Auto;
        body.max_height = Val::Auto;
        body.flex_grow = 0.0;
        body.flex_shrink = 0.0;
    }
    for mut node in &mut drawers {
        apply_inspector_drawer_layout(&mut node);
    }
}

fn apply_inspector_drawer_layout(node: &mut Node) {
    node.position_type = PositionType::Relative;
    node.left = Val::Auto;
    node.right = Val::Auto;
    node.top = Val::Auto;
    node.bottom = Val::Auto;
    node.width = Val::Percent(100.0);
    node.flex_grow = 0.0;
    node.flex_shrink = 0.0;
    node.min_height = Val::Auto;
    node.max_height = Val::Auto;
    node.flex_direction = FlexDirection::Column;
    node.align_items = AlignItems::Stretch;
    node.column_gap = Val::ZERO;
}

fn emit_intents(
    clicked: Query<(&Interaction, &Control), Changed<Interaction>>,
    mut intents: MessageWriter<UiIntent>,
) {
    for (interaction, control) in &clicked {
        if *interaction == Interaction::Pressed {
            intents.write(UiIntent::LabStatistics(control.0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{action_rail_clearance, UiViewportClass};

    #[test]
    fn drawer_is_secondary_on_compact_canvases() {
        let metrics = ResolvedUiMetrics {
            viewport: UiViewportClass::Compact,
            ..default()
        };
        assert!(action_rail_clearance(metrics.viewport) > 0.0);
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn gameplay_reentry_recreates_exactly_one_inspector_owned_drawer() {
        let mut app = App::new();
        app.add_plugins(crate::test_support::HeadlessUiPlugin::default())
            .add_systems(
                OnExit(Screen::Gameplay),
                crate::despawn_screen(Screen::Gameplay),
            );
        *app.world_mut().resource_mut::<LabStatisticsView>() = LabStatisticsView {
            present: true,
            visible: true,
            expanded: false,
            text: "Re-entry statistics".to_owned(),
        };

        let enter = |app: &mut App, screen| {
            app.world_mut()
                .resource_mut::<NextState<Screen>>()
                .set(screen);
            for _ in 0..8 {
                app.update();
            }
        };
        let drawer_count = |world: &mut World| {
            world
                .query_filtered::<Entity, With<Drawer>>()
                .iter(world)
                .count()
        };

        enter(&mut app, Screen::Gameplay);
        assert_eq!(drawer_count(app.world_mut()), 1);
        enter(&mut app, Screen::Title);
        assert_eq!(drawer_count(app.world_mut()), 0);
        enter(&mut app, Screen::Gameplay);
        assert_eq!(drawer_count(app.world_mut()), 1);
    }
}
