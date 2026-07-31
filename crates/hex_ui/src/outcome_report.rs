//! Encounter outcome and Combat Lab report presentation.

use bevy::prelude::*;
use bevy::ui_widgets::ScrollArea;
use hex_core::Screen;

use crate::{
    blurb, fine, heading, overlay_root, row_button, DespawnOnExit, OutcomeAction, OutcomeIntent,
    OutcomeReportView, UiAssets, UiIntent, UiSystems,
};

const OUTCOME_PANEL_BG: Color = Color::srgb(0.02, 0.03, 0.045);

#[derive(Component)]
struct OutcomeRoot;

#[derive(Component, Debug, Clone, Copy)]
struct Control(OutcomeIntent);

pub(super) fn plugin(app: &mut App) {
    app.add_systems(OnEnter(Screen::Gameplay), spawn)
        .add_systems(
            Update,
            (render, emit_intents.in_set(UiSystems::EmitIntents))
                .run_if(in_state(Screen::Gameplay)),
        );
}

fn spawn(mut commands: Commands) {
    commands.spawn((
        overlay_root("Encounter Outcome Modal"),
        OutcomeRoot,
        DespawnOnExit(Screen::Gameplay),
        Visibility::Hidden,
    ));
}

fn render(
    mut commands: Commands,
    view: Res<OutcomeReportView>,
    review: Option<Res<crate::review::UiReviewPresentation>>,
    roots: Query<Entity, With<OutcomeRoot>>,
    assets: Res<UiAssets>,
) {
    let review_changed = review.as_ref().is_some_and(|review| review.is_changed());
    if !view.is_changed() && !review_changed {
        return;
    }
    let view = review
        .as_ref()
        .and_then(|review| review.outcome.as_ref())
        .unwrap_or(view.as_ref());
    for root in &roots {
        commands.entity(root).despawn_related::<Children>();
        commands.entity(root).insert(if view.visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        if !view.visible {
            continue;
        }
        commands.entity(root).with_children(|overlay| {
            overlay
                .spawn((
                    Name::new("Encounter Outcome Panel"),
                    Node {
                        width: if view.body.is_some() {
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
                    BackgroundColor(OUTCOME_PANEL_BG),
                ))
                .with_children(|panel| {
                    panel.spawn(heading(&assets, view.title.clone()));
                    panel.spawn(blurb(&assets, view.detail.clone()));
                    if let Some(metadata) = &view.metadata {
                        panel.spawn(fine(&assets, metadata.clone()));
                        spawn_tabs(panel, &assets, view);
                        panel
                            .spawn((
                                Name::new("Outcome Report Body Scroll"),
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
                            .with_child(blurb(&assets, view.body.clone().unwrap_or_default()));
                        if view.mode == hex_gameplay_model::ReportMode::Compare {
                            spawn_comparisons(panel, &assets, view);
                        }
                    }
                    spawn_actions(panel, &assets, &view.actions);
                });
        });
    }
}

fn spawn_tabs(parent: &mut ChildSpawnerCommands, assets: &UiAssets, view: &OutcomeReportView) {
    parent
        .spawn((
            Name::new("Outcome Report Modes"),
            Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(7.0),
                row_gap: Val::Px(7.0),
                flex_wrap: FlexWrap::Wrap,
                ..default()
            },
        ))
        .with_children(|tabs| {
            for (mode, label) in hex_gameplay_model::ReportMode::ALL {
                let text = if view.mode == mode {
                    format!("{label} · ACTIVE")
                } else {
                    label.to_owned()
                };
                tabs.spawn((
                    row_button(label, 155.0),
                    Control(OutcomeIntent::SelectMode(mode)),
                ))
                .with_child(blurb(assets, text));
            }
        });
}

fn spawn_comparisons(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    view: &OutcomeReportView,
) {
    parent
        .spawn((
            Name::new("Outcome Compare Choices"),
            Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(6.0),
                row_gap: Val::Px(6.0),
                flex_wrap: FlexWrap::Wrap,
                ..default()
            },
        ))
        .with_children(|selectors| {
            for choice in &view.comparisons {
                selectors
                    .spawn((
                        row_button(choice.label.clone(), 150.0),
                        Control(OutcomeIntent::CompareWith(choice.id)),
                    ))
                    .with_child(blurb(assets, choice.label.clone()));
            }
        });
}

fn spawn_actions(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    actions: &[crate::OutcomeActionView],
) {
    parent
        .spawn((
            Name::new("Outcome Actions"),
            Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(10.0),
                row_gap: Val::Px(8.0),
                flex_wrap: FlexWrap::Wrap,
                justify_content: JustifyContent::Center,
                ..default()
            },
        ))
        .with_children(|buttons| {
            for action in actions {
                buttons
                    .spawn((
                        row_button(action.label.clone(), action_width(action.action)),
                        Control(OutcomeIntent::Activate(action.action)),
                    ))
                    .with_child(blurb(assets, action.label.clone()));
            }
        });
}

fn action_width(action: OutcomeAction) -> f32 {
    match action {
        OutcomeAction::Continue | OutcomeAction::Retry => 150.0,
        _ => 170.0,
    }
}

fn emit_intents(
    clicked: Query<(&Interaction, &Control), Changed<Interaction>>,
    mut intents: MessageWriter<UiIntent>,
) {
    for (interaction, control) in &clicked {
        if *interaction == Interaction::Pressed {
            intents.write(UiIntent::Outcome(control.0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_outcomes_keep_a_single_primary_transition_plus_return() {
        assert!((action_width(OutcomeAction::Continue) - 150.0).abs() < f32::EPSILON);
        assert!((action_width(OutcomeAction::Return) - 170.0).abs() < f32::EPSILON);
    }

    #[test]
    fn report_content_surface_is_opaque_over_live_gameplay() {
        assert!((OUTCOME_PANEL_BG.to_srgba().alpha - 1.0).abs() < f32::EPSILON);
    }
}
