//! Sandbox deployment presentation.

use bevy::input_focus::tab_navigation::TabGroup;
use bevy::prelude::*;
use bevy::ui_widgets::ScrollArea;
use hex_core::Screen;
use hex_gameplay_model::SandboxSide;

use crate::{
    blurb, fine, heading, label, layout::is_ultra_constrained, row_button, stacked_row_button,
    DeploymentIntent, DeploymentRosterEntryView, DeploymentView, ResolvedUiMetrics, UiAssets,
    UiIntent, UiSystems, UiViewportClass, DANGER,
};

#[derive(Component)]
struct DeploymentRoot;

#[derive(Component)]
struct DeploymentSummary;

#[derive(Component)]
struct DeploymentSide;

#[derive(Component)]
struct DeploymentSides;

#[derive(Component)]
struct DeploymentActions;

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        (
            (render, apply_layout).chain().in_set(UiSystems::Render),
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
            Name::new("Sandbox Deployment HUD"),
            DeploymentRoot,
            TabGroup {
                order: 20,
                modal: true,
            },
            // Empty drawer space must not consume world picks. Interactive
            // descendants still own their ordinary button hit targets.
            Pickable::IGNORE,
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
                    "CLICK BLUE for Party · CLICK RED for Enemies · solid tokens show placements",
                ));
            });
            hud.spawn((
                Name::new("Deployment Roster Scroll"),
                DeploymentSides,
                Node {
                    min_width: Val::Px(0.0),
                    min_height: Val::Px(0.0),
                    flex_grow: 1.0,
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(18.0),
                    ..default()
                },
            ))
            .with_children(|sides| {
                spawn_side(sides, &assets, "PARTY", SandboxSide::Party, &view.party);
                spawn_side(
                    sides,
                    &assets,
                    "ENEMIES",
                    SandboxSide::Enemies,
                    &view.enemies,
                );
            });
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
                    "Clear Party",
                    DeploymentIntent::ClearParty,
                );
                deployment_button(
                    actions,
                    &assets,
                    "Clear Enemies",
                    DeploymentIntent::ClearEnemies,
                );
                deployment_button(
                    actions,
                    &assets,
                    "Deterministic Auto-place",
                    DeploymentIntent::AutoPlace,
                );
                deployment_button(
                    actions,
                    &assets,
                    "Return to Sandbox",
                    DeploymentIntent::Back,
                );
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
    roster_side: SandboxSide,
    roster: &[DeploymentRosterEntryView],
) {
    hud.spawn((
        DeploymentSide,
        Node {
            width: Val::Px(245.0),
            // In stacked layouts `DeploymentSides` is the sole scroll owner.
            // Keep each side at its natural roster height so Yoga reports the
            // complete final card in that owner's attainable content range.
            flex_shrink: 0.0,
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
                if roster_side == SandboxSide::Party {
                    "P"
                } else {
                    "E"
                },
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
                crate::UiVisibilityRequirement::Scrollable,
                DeploymentIntent::Select {
                    side: roster_side,
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
    let width = match action {
        DeploymentIntent::Undo => 90.0,
        DeploymentIntent::ClearParty | DeploymentIntent::ClearEnemies => 150.0,
        DeploymentIntent::AutoPlace => 270.0,
        DeploymentIntent::Back => 170.0,
        DeploymentIntent::StartCombat => 160.0,
        DeploymentIntent::Select { .. } => unreachable!("roster selection uses its own card"),
    };
    parent
        .spawn((row_button(text, width), action))
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
    mut commands: Commands,
    metrics: Res<ResolvedUiMetrics>,
    added_roots: Query<(), Added<DeploymentRoot>>,
    mut roots: Query<(Entity, &mut Node), With<DeploymentRoot>>,
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
    mut side_groups: Query<
        (Entity, &mut Node),
        (
            With<DeploymentSides>,
            Without<DeploymentRoot>,
            Without<DeploymentSummary>,
            Without<DeploymentSide>,
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
    // The 1512-wide logical canvas produced by a Retina fullscreen window is
    // nominally Standard, but cannot fit the three deployment columns and the
    // persistent action region side-by-side. Treat deployment as a denser
    // composition with its own content breakpoint.
    let stacked = compact || metrics.logical_size.x < 1900.0 || metrics.content_scale >= 1.5;
    let ultra_constrained = is_ultra_constrained(*metrics);
    for (entity, mut node) in &mut roots {
        (node.left, node.right, node.width) = if stacked {
            (
                Val::Auto,
                Val::Px(if ultra_constrained { 8.0 } else { 12.0 }),
                Val::Px(stacked_drawer_width(*metrics)),
            )
        } else {
            let (left, right) = match metrics.viewport {
                UiViewportClass::Compact => (196.0, 12.0),
                UiViewportClass::Standard => (244.0, 320.0),
                UiViewportClass::Wide => (288.0, 360.0),
            };
            (Val::Px(left), Val::Px(right), Val::Auto)
        };
        node.top = Val::Px(if ultra_constrained || compact {
            12.0
        } else if stacked {
            68.0
        } else {
            18.0
        });
        node.bottom = if ultra_constrained {
            Val::Px(8.0)
        } else if compact {
            // Deployment projects an intentionally minimal 48px rail. Reserving
            // the ordinary gameplay rail's full height would collapse the 6v6
            // roster viewport to zero on short Compact canvases.
            Val::Px(68.0)
        } else if stacked {
            Val::Px(68.0)
        } else {
            Val::Auto
        };
        node.min_height = if stacked {
            Val::Px(0.0)
        } else {
            Val::Px(126.0)
        };
        node.flex_direction = if stacked {
            FlexDirection::Column
        } else {
            FlexDirection::Row
        };
        node.overflow = Overflow::clip();
        commands.entity(entity).remove::<ScrollArea>();
    }
    for mut node in &mut summary {
        node.display = if ultra_constrained {
            Display::None
        } else {
            Display::Flex
        };
        node.width = if stacked {
            Val::Percent(100.0)
        } else {
            Val::Px(300.0)
        };
    }
    for mut node in &mut sides {
        node.width = if stacked {
            Val::Percent(100.0)
        } else {
            Val::Px(245.0 * metrics.control_scale.max(1.0))
        };
    }
    for (entity, mut node) in &mut side_groups {
        node.width = if stacked {
            Val::Percent(100.0)
        } else {
            Val::Auto
        };
        node.min_height = Val::Px(0.0);
        node.flex_grow = 1.0;
        node.height = if stacked { Val::Px(0.0) } else { Val::Auto };
        node.flex_basis = if stacked { Val::Px(0.0) } else { Val::Auto };
        node.flex_direction = if stacked {
            FlexDirection::Column
        } else {
            FlexDirection::Row
        };
        node.row_gap = if stacked { Val::Px(8.0) } else { Val::ZERO };
        // Leave semantic tail room inside the sole roster scroll owner. Bevy's
        // fractional text/control rounding can otherwise place the final card's
        // border one logical pixel beyond the reported content extent.
        node.padding.bottom = if stacked {
            Val::Px(8.0 * metrics.spacing_scale)
        } else {
            Val::ZERO
        };
        node.overflow = if stacked {
            Overflow::scroll_y()
        } else {
            Overflow::default()
        };
        if stacked {
            commands.entity(entity).insert(ScrollArea);
        } else {
            commands.entity(entity).remove::<ScrollArea>();
        }
    }
    for mut node in &mut actions {
        node.width = if stacked {
            Val::Percent(100.0)
        } else {
            Val::Px(340.0)
        };
        node.flex_shrink = 0.0;
    }
}

fn stacked_drawer_width(metrics: ResolvedUiMetrics) -> f32 {
    // Deployment is an overlay on an interactive map, so its stacked form must
    // preserve a substantial world lane. Grow enough for semantic controls, but
    // never return to the almost-full-canvas slab this drawer replaced.
    (450.0 * metrics.control_scale.max(1.0)).min(metrics.logical_size.x * 0.66)
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
                party: vec![DeploymentRosterEntryView {
                    index: 0,
                    name: "Hedge Mage".to_owned(),
                    selected: true,
                    position: None,
                }],
                enemies: Vec::new(),
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

    #[cfg(feature = "test-support")]
    #[test]
    fn retina_compact_deployment_preserves_a_clickable_world_lane() {
        let mut app = App::new();
        // The reported native window was 2566x1494 physical pixels on Retina,
        // or approximately this logical UI canvas after window chrome.
        app.add_plugins(crate::test_support::HeadlessUiPlugin::new(1284, 744));
        app.world_mut().insert_resource(DeploymentView {
            active: true,
            map_name: "Two Rings".to_owned(),
            notice: "Choose a surface".to_owned(),
            party: vec![
                DeploymentRosterEntryView {
                    index: 0,
                    name: "Hedge Mage".to_owned(),
                    selected: true,
                    position: None,
                },
                DeploymentRosterEntryView {
                    index: 1,
                    name: "Raider".to_owned(),
                    selected: false,
                    position: None,
                },
            ],
            enemies: vec![DeploymentRosterEntryView {
                index: 0,
                name: "Raider".to_owned(),
                selected: false,
                position: None,
            }],
            complete: false,
        });
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Gameplay);
        for _ in 0..8 {
            app.update();
        }

        let root = app
            .world_mut()
            .query_filtered::<Entity, With<DeploymentRoot>>()
            .single(app.world())
            .expect("deployment must have one root");
        let root_size = app
            .world()
            .get::<ComputedNode>(root)
            .map(|node| node.size() * node.inverse_scale_factor)
            .expect("deployment root must be laid out");
        let canvas = Vec2::new(1284.0, 744.0);
        assert!(
            root_size.x <= canvas.x * 0.5,
            "the compact drawer must leave at least half the map horizontally visible: {root_size:?}"
        );
        assert!(
            root_size.x * root_size.y <= canvas.x * canvas.y * 0.45,
            "the deployment overlay must not cover most of the interactive map: {root_size:?}"
        );
        assert_eq!(
            app.world().get::<Pickable>(root),
            Some(&Pickable::IGNORE),
            "empty drawer space must pass world picks through"
        );

        let snapshot = crate::test_support::ui_tree_snapshot(app.world_mut());
        let issues = snapshot.task_issues(crate::test_support::UiTaskCase::DeploymentIncomplete);
        assert!(
            issues.is_empty(),
            "the bounded drawer must retain its controls and scroll contract: {issues:#?}"
        );
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn enlarged_stacked_roster_can_focus_and_reveal_its_final_card() {
        use bevy::input_focus::InputFocus;

        let mut app = App::new();
        app.add_plugins(crate::test_support::HeadlessUiPlugin::new(960, 540));
        app.world_mut()
            .insert_resource(crate::UiScalePreference(crate::UiScaleMode::Percent175));
        let roster = |side: &str| {
            (0..6)
                .map(|index| DeploymentRosterEntryView {
                    index,
                    name: format!("{side} Unit {}", index + 1),
                    selected: index == 0,
                    position: None,
                })
                .collect::<Vec<_>>()
        };
        app.world_mut().insert_resource(DeploymentView {
            active: true,
            map_name: "Stacked Surface Arena".to_owned(),
            notice: "Place every Party and Enemy character on an exact legal surface.".to_owned(),
            party: roster("Party"),
            enemies: roster("Enemy"),
            complete: false,
        });
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Gameplay);
        for _ in 0..8 {
            app.update();
        }

        let entity_named = |app: &mut App, wanted: &str| {
            app.world_mut()
                .query::<(Entity, &Name)>()
                .iter(app.world())
                .find_map(|(entity, name)| (name.as_str() == wanted).then_some(entity))
                .unwrap_or_else(|| panic!("missing {wanted:?}"))
        };
        let roster_scroll = entity_named(&mut app, "Deployment Roster Scroll");
        let final_card_name = "SELECT [E6] Enemy Unit 6\nchoose surface";
        let final_card_entity = entity_named(&mut app, final_card_name);
        let roster_scroll_geometry = app
            .world()
            .get::<ComputedNode>(roster_scroll)
            .map(|node| {
                (
                    node.size() * node.inverse_scale_factor,
                    node.content_size() * node.inverse_scale_factor,
                )
            })
            .expect("the deployment scroll owner must be laid out");
        let side_geometry = app
            .world_mut()
            .query_filtered::<&ComputedNode, With<DeploymentSide>>()
            .iter(app.world())
            .map(|node| {
                (
                    node.size() * node.inverse_scale_factor,
                    node.content_size() * node.inverse_scale_factor,
                )
            })
            .collect::<Vec<_>>();
        let initial = crate::test_support::ui_tree_snapshot(app.world_mut());
        let final_card = initial
            .nodes
            .iter()
            .find(|node| node.name == final_card_name)
            .expect("the populated Enemy roster must expose its sixth card");
        assert_eq!(
            final_card.visibility_requirement,
            Some(crate::UiVisibilityRequirement::Scrollable)
        );
        assert!(
            !final_card.fully_visible && final_card.scroll_reachable,
            "the exact regression must start below the fold but have an attainable range: {final_card:?}; owner={roster_scroll_geometry:?}; sides={side_geometry:?}"
        );

        assert!(
            final_card.in_focus_order && final_card.keyboard_reachable == Some(true),
            "the sixth Enemy deployment card must belong to the active keyboard scope: {final_card:?}; order={:?}",
            initial.focus_order,
        );
        app.insert_resource(InputFocus::from_entity(final_card_entity));
        for _ in 0..3 {
            app.update();
        }
        assert!(
            app.world()
                .get::<ScrollPosition>(roster_scroll)
                .is_some_and(|position| position.y > 0.0),
            "focusing the final card must move the sole deployment scroll owner"
        );
        let focused = crate::test_support::ui_tree_snapshot(app.world_mut());
        let final_card = focused
            .nodes
            .iter()
            .find(|node| node.name == final_card_name)
            .expect("the focused card remains presented");
        assert!(
            final_card.fully_visible && final_card.focused,
            "the complete card and focus ring must be visible after keyboard navigation: {final_card:?}"
        );
    }
}
