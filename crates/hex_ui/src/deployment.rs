//! Combat Lab deployment presentation.

use bevy::input_focus::tab_navigation::TabGroup;
use bevy::prelude::*;
use hex_core::Screen;

use crate::{
    action_rail_clearance, blurb, fine, heading, label, layout::is_ultra_constrained, row_button,
    stacked_row_button, DeploymentIntent, DeploymentRosterEntryView, DeploymentView,
    ResolvedUiMetrics, UiAssets, UiIntent, UiSystems, UiViewportClass, DANGER,
};

#[derive(Component)]
struct DeploymentRoot;

#[derive(Component)]
struct DeploymentSummary;

#[derive(Component)]
struct DeploymentSide;

#[derive(Component)]
struct DeploymentActions;

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        (
            render,
            apply_layout,
            emit_actions.in_set(UiSystems::EmitIntents),
        )
            .run_if(in_state(Screen::Gameplay)),
    );
}

fn render(
    mut commands: Commands,
    roots: Query<Entity, With<DeploymentRoot>>,
    view: Res<DeploymentView>,
    assets: Res<UiAssets>,
) {
    if !view.is_changed() {
        return;
    }
    for root in &roots {
        commands.entity(root).despawn();
    }
    if !view.active {
        return;
    }

    commands
        .spawn((
            Name::new("Combat Lab Deployment HUD"),
            DeploymentRoot,
            TabGroup {
                order: 20,
                modal: true,
            },
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(22.0),
                right: Val::Px(22.0),
                top: Val::Px(18.0),
                min_height: Val::Px(126.0),
                padding: UiRect::all(Val::Px(13.0)),
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(18.0),
                border_radius: BorderRadius::all(Val::Px(7.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.015, 0.022, 0.035, 0.94)),
            BorderColor::all(Color::srgba(0.93, 0.79, 0.46, 0.52)),
            GlobalZIndex(11),
        ))
        .with_children(|hud| {
            hud.spawn((
                DeploymentSummary,
                Node {
                    width: Val::Px(300.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(3.0),
                    ..default()
                },
            ))
            .with_children(|summary| {
                summary.spawn(heading(&assets, format!("DEPLOY · {}", view.map_name)));
                summary.spawn(blurb(&assets, view.notice.clone()));
                summary.spawn(fine(
                    &assets,
                    "CLICK BLUE for Player · CLICK RED for Hostile · solid tokens show placements",
                ));
            });
            spawn_side(hud, &assets, "PLAYER", true, &view.players);
            spawn_side(hud, &assets, "HOSTILE", false, &view.hostiles);
            hud.spawn((
                DeploymentActions,
                Node {
                    width: Val::Px(340.0),
                    flex_direction: FlexDirection::Row,
                    flex_wrap: FlexWrap::Wrap,
                    align_content: AlignContent::FlexStart,
                    column_gap: Val::Px(5.0),
                    row_gap: Val::Px(5.0),
                    ..default()
                },
            ))
            .with_children(|actions| {
                deployment_button(actions, &assets, "Undo", DeploymentIntent::Undo);
                deployment_button(
                    actions,
                    &assets,
                    "Clear Player",
                    DeploymentIntent::ClearPlayer,
                );
                deployment_button(
                    actions,
                    &assets,
                    "Clear Hostile",
                    DeploymentIntent::ClearHostile,
                );
                deployment_button(
                    actions,
                    &assets,
                    "Deterministic Auto-place",
                    DeploymentIntent::AutoPlace,
                );
                deployment_button(actions, &assets, "Back to Rules", DeploymentIntent::Back);
                if view.complete {
                    deployment_button(
                        actions,
                        &assets,
                        "Start Combat",
                        DeploymentIntent::StartCombat,
                    );
                } else {
                    actions
                        .spawn(fine(
                            &assets,
                            "START COMBAT · DISABLED — place every roster entry",
                        ))
                        .insert(TextColor(DANGER));
                }
            });
        });
}

fn spawn_side(
    hud: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    title: &'static str,
    player: bool,
    roster: &[DeploymentRosterEntryView],
) {
    hud.spawn((
        DeploymentSide,
        Node {
            width: Val::Px(245.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(2.0),
            ..default()
        },
    ))
    .with_children(|side| {
        side.spawn(fine(assets, title));
        for entry in roster {
            let text = format!(
                "{} [{}{}] {}\n{}",
                if entry.selected { "SELECTED" } else { "SELECT" },
                if player { "P" } else { "H" },
                entry.index + 1,
                entry.name,
                entry.position.map_or_else(
                    || "choose surface".to_owned(),
                    |pos| format!(
                        "({},{},{}) · elevation {}",
                        pos.coord.x(),
                        pos.coord.y(),
                        pos.coord.z(),
                        pos.level
                    )
                )
            );
            side.spawn((
                stacked_row_button(text.clone(), 235.0),
                DeploymentIntent::Select {
                    player,
                    index: entry.index,
                },
            ))
            .insert(BorderColor::all(if entry.selected {
                Color::srgba(0.93, 0.79, 0.46, 0.95)
            } else {
                Color::srgba(0.26, 0.29, 0.34, 0.9)
            }))
            .with_child(fine(assets, text));
        }
    });
}

fn deployment_button(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    text: &'static str,
    action: DeploymentIntent,
) {
    parent
        .spawn((row_button(text, 166.0), action))
        .with_child(label(assets, text));
}

fn emit_actions(
    clicked: Query<(&Interaction, &DeploymentIntent), Changed<Interaction>>,
    mut intents: MessageWriter<UiIntent>,
) {
    for (interaction, action) in &clicked {
        if *interaction == Interaction::Pressed {
            intents.write(UiIntent::Deployment(*action));
        }
    }
}

fn apply_layout(
    metrics: Res<ResolvedUiMetrics>,
    added_roots: Query<(), Added<DeploymentRoot>>,
    mut roots: Query<&mut Node, With<DeploymentRoot>>,
    mut summary: Query<
        &mut Node,
        (
            With<DeploymentSummary>,
            Without<DeploymentRoot>,
            Without<DeploymentSide>,
            Without<DeploymentActions>,
        ),
    >,
    mut sides: Query<
        &mut Node,
        (
            With<DeploymentSide>,
            Without<DeploymentRoot>,
            Without<DeploymentSummary>,
            Without<DeploymentActions>,
        ),
    >,
    mut actions: Query<
        &mut Node,
        (
            With<DeploymentActions>,
            Without<DeploymentRoot>,
            Without<DeploymentSummary>,
            Without<DeploymentSide>,
        ),
    >,
) {
    if !metrics.is_changed() && added_roots.is_empty() {
        return;
    }
    let compact = metrics.viewport == UiViewportClass::Compact;
    let ultra_constrained = is_ultra_constrained(*metrics);
    for mut node in &mut roots {
        (node.left, node.right) = match metrics.viewport {
            UiViewportClass::Compact if ultra_constrained => (Val::Px(8.0), Val::Px(8.0)),
            UiViewportClass::Compact => (Val::Px(196.0), Val::Px(12.0)),
            UiViewportClass::Standard => (Val::Px(244.0), Val::Px(320.0)),
            UiViewportClass::Wide => (Val::Px(288.0), Val::Px(360.0)),
        };
        node.top = Val::Px(if ultra_constrained {
            68.0
        } else if compact {
            12.0
        } else {
            18.0
        });
        node.bottom = if ultra_constrained {
            Val::Px(8.0)
        } else if compact {
            Val::Px(action_rail_clearance(metrics.viewport))
        } else {
            Val::Auto
        };
        node.min_height = if compact {
            Val::Px(0.0)
        } else {
            Val::Px(126.0)
        };
        node.flex_direction = if compact {
            FlexDirection::Column
        } else {
            FlexDirection::Row
        };
        node.overflow = if compact {
            Overflow::scroll_y()
        } else {
            Overflow::default()
        };
    }
    for mut node in &mut summary {
        node.width = if compact {
            Val::Percent(100.0)
        } else {
            Val::Px(300.0)
        };
    }
    for mut node in &mut sides {
        node.width = if compact {
            Val::Percent(100.0)
        } else {
            Val::Px(245.0)
        };
    }
    for mut node in &mut actions {
        node.width = if compact {
            Val::Percent(100.0)
        } else {
            Val::Px(340.0)
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assets() -> UiAssets {
        UiAssets {
            display: Handle::default(),
            body: Handle::default(),
            hex_cell: Handle::default(),
        }
    }

    #[test]
    fn start_combat_exists_only_for_a_canonically_complete_projection() {
        let mut app = App::new();
        app.insert_resource(assets())
            .insert_resource(DeploymentView {
                active: true,
                map_name: "Flat Arena".to_owned(),
                notice: "Choose a surface".to_owned(),
                players: vec![DeploymentRosterEntryView {
                    index: 0,
                    name: "Hedge Mage".to_owned(),
                    selected: true,
                    position: None,
                }],
                hostiles: Vec::new(),
                complete: false,
            })
            .add_systems(Update, render);
        app.update();
        let mut actions = app.world_mut().query::<&DeploymentIntent>();
        assert!(!actions
            .iter(app.world())
            .any(|action| *action == DeploymentIntent::StartCombat));

        app.world_mut().resource_mut::<DeploymentView>().complete = true;
        app.update();
        let mut actions = app.world_mut().query::<&DeploymentIntent>();
        assert!(actions
            .iter(app.world())
            .any(|action| *action == DeploymentIntent::StartCombat));
    }
}
