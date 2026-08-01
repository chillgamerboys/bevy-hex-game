//! Live Combat Lab statistics presentation.

use bevy::input_focus::tab_navigation::TabGroup;
use bevy::prelude::*;
use bevy::ui_widgets::ScrollArea;
use hex_core::Screen;

use crate::{
    blurb, heading, layout::is_ultra_constrained, row_button, supporting_text_role, DespawnOnExit,
    LabStatisticsIntent, LabStatisticsView, ResolvedUiMetrics, UiAssets, UiIntent, UiSystems,
    UiViewportClass, ACCENT_EDGE, LABEL,
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
    app.add_systems(OnEnter(Screen::Gameplay), spawn)
        .add_systems(
            Update,
            (
                (render, apply_layout).chain().in_set(UiSystems::Render),
                emit_intents.in_set(UiSystems::EmitIntents),
            )
                .run_if(in_state(Screen::Gameplay)),
        );
}

fn spawn(mut commands: Commands, assets: Res<UiAssets>) {
    commands
        .spawn((
            Name::new("Combat Lab Live Statistics Drawer"),
            Drawer,
            TabGroup::new(15),
            DespawnOnExit(Screen::Gameplay),
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
                    row_button("Expand or collapse live Combat Lab statistics", 250.0),
                    Control(LabStatisticsIntent::Toggle),
                ))
                .with_child(blurb(&assets, "Statistics · Collapse"));
            drawer
                .spawn((
                    Name::new("Combat Lab Statistics Body"),
                    Body,
                    ScrollArea,
                    ScrollPosition::default(),
                    Node {
                        min_height: Val::Px(0.0),
                        flex_grow: 1.0,
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(5.0),
                        overflow: Overflow::scroll_y(),
                        ..default()
                    },
                ))
                .with_children(|body| {
                    body.spawn(heading(&assets, "LIVE COMBAT LAB STATISTICS"));
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
                });
            drawer
                .spawn((
                    row_button(
                        "End experiment and save the current Combat Lab report",
                        250.0,
                    ),
                    Control(LabStatisticsIntent::EndExperiment),
                ))
                .with_child(blurb(&assets, "End Experiment"));
        });
}

fn render(
    view: Res<LabStatisticsView>,
    review: Option<Res<crate::review::UiReviewPresentation>>,
    mut drawers: Query<&mut Visibility, With<Drawer>>,
    mut bodies: Query<&mut Visibility, (With<Body>, Without<Drawer>)>,
    mut summaries: Query<&mut Text, With<Summary>>,
    buttons: Query<(&Control, &Children)>,
    mut labels: Query<&mut Text, (Without<Summary>, Without<Drawer>)>,
) {
    let review_changed = review.as_ref().is_some_and(|review| review.is_changed());
    if !view.is_changed() && !review_changed {
        return;
    }
    let view = review
        .as_ref()
        .and_then(|review| review.statistics.as_ref())
        .unwrap_or(view.as_ref());
    for mut visibility in &mut drawers {
        *visibility = if view.present && view.visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
    for mut visibility in &mut bodies {
        *visibility = if view.expanded {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
    for mut text in &mut summaries {
        **text = view.text.clone();
    }
    for (control, children) in &buttons {
        if control.0 != LabStatisticsIntent::Toggle {
            continue;
        }
        if let Some(child) = children.first() {
            if let Ok(mut text) = labels.get_mut(*child) {
                **text = if view.expanded {
                    "Statistics · Collapse".to_owned()
                } else {
                    "Statistics · Expand".to_owned()
                };
            }
        }
    }
}

fn apply_layout(
    metrics: Res<ResolvedUiMetrics>,
    view: Res<LabStatisticsView>,
    review: Option<Res<crate::review::UiReviewPresentation>>,
    added: Query<(), Added<Drawer>>,
    mut drawers: Query<&mut Node, (With<Drawer>, Without<Body>)>,
    mut bodies: Query<(&mut Node, &mut Visibility), (With<Body>, Without<Drawer>)>,
) {
    if !metrics.is_changed() && added.is_empty() {
        return;
    }
    let expanded = review
        .as_ref()
        .and_then(|review| review.statistics.as_ref())
        .map_or(view.expanded, |view| view.expanded);
    for (mut body, mut visibility) in &mut bodies {
        body.display = Display::Flex;
        *visibility = if expanded {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
    for mut node in &mut drawers {
        match metrics.viewport {
            UiViewportClass::Compact => {
                node.display = Display::Flex;
                let ultra = is_ultra_constrained(*metrics);
                let top = if ultra {
                    crate::layout::ultra_action_rail_height(*metrics) + 16.0
                } else {
                    92.0
                };
                node.top = Val::Px(top);
                let bottom = if ultra {
                    12.0
                } else {
                    crate::layout::semantic_action_rail_clearance(*metrics)
                };
                node.bottom = Val::Px(bottom);
                if ultra {
                    node.left = Val::Px(12.0);
                    node.right = Val::Px(12.0);
                    node.width = Val::Auto;
                    node.flex_direction = FlexDirection::Row;
                    node.align_items = AlignItems::Center;
                    node.column_gap = Val::Px(7.0);
                    for (mut body, mut visibility) in &mut bodies {
                        body.display = Display::None;
                        *visibility = Visibility::Hidden;
                    }
                } else {
                    node.left = Val::Auto;
                    node.right = Val::Px(12.0);
                    node.width = Val::Px(250.0 * metrics.control_scale.max(1.0) + 20.0);
                    node.flex_direction = FlexDirection::Column;
                    node.align_items = AlignItems::Stretch;
                    node.column_gap = Val::ZERO;
                }
                node.max_height = Val::Px((metrics.logical_size.y - top - bottom).max(0.0));
            }
            UiViewportClass::Standard => {
                node.display = Display::Flex;
                node.left = Val::Auto;
                node.right = Val::Px(12.0);
                node.top = Val::Px(12.0);
                node.bottom = Val::Auto;
                // The expanded drawer owns the inspector region. Matching its
                // complete width prevents the read-only lattice beneath it from
                // peeking around the opaque replacement surface.
                node.width = Val::Px((250.0 * metrics.control_scale.max(1.0) + 20.0).max(300.0));
                node.max_height = Val::Px(520.0);
                node.flex_direction = FlexDirection::Column;
                node.align_items = AlignItems::Stretch;
                node.column_gap = Val::ZERO;
            }
            UiViewportClass::Wide => {
                node.display = Display::Flex;
                node.left = Val::Auto;
                node.right = Val::Px(16.0);
                node.top = Val::Px(16.0);
                node.bottom = Val::Auto;
                node.width = Val::Px((250.0 * metrics.control_scale.max(1.0) + 20.0).max(332.0));
                node.max_height = Val::Px(560.0);
                node.flex_direction = FlexDirection::Column;
                node.align_items = AlignItems::Stretch;
                node.column_gap = Val::ZERO;
            }
        }
    }
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
    use crate::action_rail_clearance;

    #[test]
    fn drawer_is_secondary_on_compact_canvases() {
        let metrics = ResolvedUiMetrics {
            viewport: UiViewportClass::Compact,
            ..default()
        };
        assert!(action_rail_clearance(metrics.viewport) > 0.0);
    }
}
