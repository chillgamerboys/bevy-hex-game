//! Development-only cyclic time controls.

use bevy::prelude::*;
use hex_core::Screen;

use crate::{
    blurb, fine, heading, layout::is_ultra_constrained, panel, row_button, DevTimeIntent,
    DevTimeView, GameplayChromeView, HudElement, ResolvedUiMetrics, UiAssets, UiHudSetup, UiIntent,
    UiRegionRole, UiSystems, UiViewportClass,
};

const CONTROLS: [(&str, &str, DevTimeIntent); 6] = [
    (
        "Dev Time Previous Half Hour",
        "−30m",
        DevTimeIntent::PreviousHalfHour,
    ),
    (
        "Dev Time Next Half Hour",
        "+30m",
        DevTimeIntent::NextHalfHour,
    ),
    ("Dev Time Midnight", "Midnight", DevTimeIntent::Midnight),
    ("Dev Time Dawn", "Dawn", DevTimeIntent::Dawn),
    ("Dev Time Noon", "Noon", DevTimeIntent::Noon),
    ("Dev Time Dusk", "Dusk", DevTimeIntent::Dusk),
];

#[derive(Component)]
struct DevTimePanel;

#[derive(Component)]
struct DevTimeHeading;

#[derive(Component)]
struct DevTimeStatus;

#[derive(Component, Default)]
struct DevTimeControls {
    available: bool,
}

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
struct DevTimeControl(DevTimeIntent);

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        OnEnter(Screen::Gameplay),
        spawn_panel.in_set(UiHudSetup::Panels),
    )
    .add_systems(
        Update,
        (
            rebuild.in_set(UiSystems::Render),
            reconcile_layout.in_set(UiSystems::Render),
            emit_intents.in_set(UiSystems::EmitIntents),
        )
            .run_if(in_state(Screen::Gameplay)),
    );
}

fn spawn_panel(
    mut commands: Commands,
    assets: Res<UiAssets>,
    metrics: Res<ResolvedUiMetrics>,
    chrome: Res<GameplayChromeView>,
    regions: Query<(Entity, &UiRegionRole, Option<&ChildOf>)>,
) {
    let Some((inspector, frame)) = regions.iter().find_map(|(entity, role, parent)| {
        (*role == UiRegionRole::Inspector).then_some((entity, parent.map(ChildOf::parent)))
    }) else {
        return;
    };
    let parent = panel_parent(metrics.viewport, inspector, frame);
    let panel = commands
        .spawn((
            Name::new("Dev Time Panel"),
            DevTimePanel,
            HudElement,
            panel(),
            Pickable::IGNORE,
            GlobalZIndex(3),
        ))
        .insert(panel_node(*metrics, chrome.decision_required))
        .with_children(|panel| {
            panel.spawn((DevTimeHeading, heading(&assets, "DEV · TIME")));
            panel.spawn((DevTimeStatus, blurb(&assets, "Checking cyclic time…")));
            panel.spawn((
                Name::new("Dev Time Controls"),
                DevTimeControls::default(),
                controls_node(*metrics),
                Pickable::IGNORE,
            ));
        })
        .id();
    commands.entity(parent).add_child(panel);
}

fn panel_parent(viewport: UiViewportClass, inspector: Entity, frame: Option<Entity>) -> Entity {
    if viewport == UiViewportClass::Compact {
        frame.unwrap_or(inspector)
    } else {
        inspector
    }
}

fn panel_node(metrics: ResolvedUiMetrics, decision_required: bool) -> Node {
    let mut node = Node {
        width: Val::Percent(100.0),
        flex_direction: FlexDirection::Column,
        row_gap: Val::Px(8.0),
        padding: UiRect::all(Val::Px(12.0)),
        border: UiRect::all(Val::Px(1.0)),
        border_radius: BorderRadius::all(Val::Px(10.0)),
        ..default()
    };
    if metrics.viewport == UiViewportClass::Compact {
        node.position_type = PositionType::Absolute;
        node.row_gap = Val::Px(4.0);
        if uses_ultra_middle_band(metrics) {
            node.top = Val::Px(138.0);
            node.right = Val::Px(12.0);
            node.left = Val::Px(196.0);
            node.width = Val::Px(metrics.effective_size.x - 208.0);
            node.height = Val::Px(82.0);
            node.row_gap = Val::Px(2.0);
            node.padding = UiRect::all(Val::Px(4.0));
        } else if is_ultra_constrained(metrics) {
            node.top = Val::Px(8.0);
            node.left = Val::Px(8.0);
            node.width = Val::Px(180.0);
            node.height = Val::Px(128.0);
            node.padding = UiRect::all(Val::Px(4.0));
        } else {
            node.top = Val::Px(8.0);
            node.right = Val::Px(8.0);
            node.width = Val::Px(256.0);
            node.height = Val::Px(120.0);
            node.padding = UiRect::all(Val::Px(6.0));
        }
    }
    if is_ultra_constrained(metrics) && decision_required {
        node.display = Display::None;
    }
    node
}

fn uses_ultra_middle_band(metrics: ResolvedUiMetrics) -> bool {
    is_ultra_constrained(metrics)
        && metrics.effective_size.x >= 620.0
        && metrics.effective_size.y >= 352.0
}

fn controls_node(metrics: ResolvedUiMetrics) -> Node {
    let gap = if uses_ultra_middle_band(metrics) {
        2.0
    } else if metrics.viewport == UiViewportClass::Compact {
        4.0
    } else {
        6.0
    };
    Node {
        width: Val::Percent(100.0),
        flex_direction: FlexDirection::Row,
        flex_wrap: FlexWrap::Wrap,
        column_gap: Val::Px(gap),
        row_gap: Val::Px(gap),
        ..default()
    }
}

fn reconcile_layout(
    mut commands: Commands,
    metrics: Res<ResolvedUiMetrics>,
    chrome: Res<GameplayChromeView>,
    added_panels: Query<(), Added<DevTimePanel>>,
    added_roots: Query<(), Added<DevTimeControls>>,
    added_controls: Query<(), Added<DevTimeControl>>,
    regions: Query<(Entity, &UiRegionRole, &ChildOf)>,
    mut panels: Query<(Entity, &ChildOf, &mut Node), (With<DevTimePanel>, Without<DevTimeHeading>)>,
    mut roots: Query<
        &mut Node,
        (
            With<DevTimeControls>,
            Without<DevTimePanel>,
            Without<DevTimeHeading>,
            Without<DevTimeControl>,
        ),
    >,
    mut headings: Query<
        &mut Node,
        (
            With<DevTimeHeading>,
            Without<DevTimePanel>,
            Without<DevTimeControls>,
            Without<DevTimeControl>,
        ),
    >,
    mut controls: Query<
        &mut Node,
        (
            With<DevTimeControl>,
            Without<DevTimePanel>,
            Without<DevTimeHeading>,
            Without<DevTimeControls>,
        ),
    >,
) {
    if !metrics.is_changed()
        && !chrome.is_changed()
        && added_panels.is_empty()
        && added_roots.is_empty()
        && added_controls.is_empty()
    {
        return;
    }

    let Some((inspector, frame)) = regions.iter().find_map(|(entity, role, parent)| {
        (*role == UiRegionRole::Inspector).then_some((entity, parent.parent()))
    }) else {
        return;
    };
    let wanted_parent = panel_parent(metrics.viewport, inspector, Some(frame));
    if let Ok((panel, current_parent, mut node)) = panels.single_mut() {
        if current_parent.parent() != wanted_parent {
            commands.entity(wanted_parent).add_child(panel);
        }
        *node = panel_node(*metrics, chrome.decision_required);
    }
    if let Ok(mut node) = roots.single_mut() {
        *node = controls_node(*metrics);
    }
    if let Ok(mut node) = headings.single_mut() {
        node.display = if metrics.viewport == UiViewportClass::Compact {
            Display::None
        } else {
            Display::Flex
        };
    }
    let (width, height) = if uses_ultra_middle_band(*metrics) {
        (132.0, 22.0)
    } else if is_ultra_constrained(*metrics) {
        (82.0, 28.0)
    } else if metrics.viewport == UiViewportClass::Compact {
        (76.0, 36.0)
    } else {
        (96.0, 48.0)
    };
    for mut node in &mut controls {
        node.width = Val::Px(width);
        node.height = Val::Px(height);
        node.padding = if is_ultra_constrained(*metrics) {
            UiRect::axes(Val::Px(3.0), Val::Px(1.0))
        } else if metrics.viewport == UiViewportClass::Compact {
            UiRect::axes(Val::Px(4.0), Val::Px(2.0))
        } else {
            UiRect::axes(Val::Px(10.0), Val::Px(4.0))
        };
    }
}

fn rebuild(
    mut commands: Commands,
    view: Res<DevTimeView>,
    mut statuses: Query<(&mut Text, Ref<DevTimeStatus>)>,
    mut controls: Query<(Entity, &mut DevTimeControls)>,
    assets: Res<UiAssets>,
    metrics: Res<ResolvedUiMetrics>,
) {
    let Ok((mut status, status_marker)) = statuses.single_mut() else {
        return;
    };
    let Ok((controls_entity, mut controls)) = controls.single_mut() else {
        return;
    };
    let controls_added = controls.is_added();
    if !view.is_changed() && !metrics.is_changed() && !status_marker.is_added() && !controls_added {
        return;
    }

    let available = matches!(view.as_ref(), DevTimeView::Available { .. });
    if controls_added || controls.available != available {
        commands
            .entity(controls_entity)
            .despawn_related::<Children>();
        if available {
            commands.entity(controls_entity).with_children(|controls| {
                for (name, label, intent) in CONTROLS {
                    controls
                        .spawn((row_button(name, 96.0), DevTimeControl(intent)))
                        .with_child(fine(&assets, label));
                }
            });
        }
        controls.available = available;
    }

    match view.as_ref() {
        DevTimeView::Available { hours } => {
            **status = if metrics.viewport == UiViewportClass::Compact {
                format!("DEV · TIME · {hours:.1} h")
            } else {
                format!("CURRENT · {hours:.1} h")
            };
        }
        DevTimeView::Unavailable { reason } => {
            **status = if metrics.viewport == UiViewportClass::Compact {
                format!("DEV · TIME · UNAVAILABLE\n{reason}")
            } else {
                format!("UNAVAILABLE · {reason}")
            };
        }
    }
}

fn emit_intents(
    view: Res<DevTimeView>,
    controls: Query<(&Interaction, &DevTimeControl), Changed<Interaction>>,
    mut intents: MessageWriter<UiIntent>,
) {
    if !matches!(view.as_ref(), DevTimeView::Available { .. }) {
        return;
    }
    for (interaction, control) in &controls {
        if *interaction == Interaction::Pressed {
            intents.write(UiIntent::DevTime(control.0));
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::input_focus::tab_navigation::{TabGroup, TabIndex};
    use bevy::input_focus::InputFocus;

    use super::*;

    #[derive(Resource, Default)]
    struct Received(Vec<DevTimeIntent>);

    fn receive(mut intents: MessageReader<UiIntent>, mut received: ResMut<Received>) {
        for intent in intents.read() {
            if let UiIntent::DevTime(intent) = intent {
                received.0.push(*intent);
            }
        }
    }

    #[test]
    fn controls_cover_the_exact_six_intents_with_stable_names() {
        assert_eq!(
            CONTROLS,
            [
                (
                    "Dev Time Previous Half Hour",
                    "−30m",
                    DevTimeIntent::PreviousHalfHour,
                ),
                (
                    "Dev Time Next Half Hour",
                    "+30m",
                    DevTimeIntent::NextHalfHour,
                ),
                ("Dev Time Midnight", "Midnight", DevTimeIntent::Midnight),
                ("Dev Time Dawn", "Dawn", DevTimeIntent::Dawn),
                ("Dev Time Noon", "Noon", DevTimeIntent::Noon),
                ("Dev Time Dusk", "Dusk", DevTimeIntent::Dusk),
            ]
        );
    }

    #[test]
    fn one_press_emits_one_typed_intent() {
        let mut app = App::new();
        app.add_message::<UiIntent>()
            .init_resource::<Received>()
            .insert_resource(DevTimeView::Available { hours: 12.0 })
            .add_systems(Update, (emit_intents, receive).chain());
        let button = app
            .world_mut()
            .spawn((
                Interaction::None,
                DevTimeControl(DevTimeIntent::NextHalfHour),
            ))
            .id();

        app.update();
        assert!(app.world().resource::<Received>().0.is_empty());
        *app.world_mut().get_mut::<Interaction>(button).unwrap() = Interaction::Pressed;
        app.update();
        app.update();

        assert_eq!(
            app.world().resource::<Received>().0,
            [DevTimeIntent::NextHalfHour]
        );
    }

    #[test]
    fn unavailable_removes_controls_and_blocks_a_stale_press() {
        let mut app = App::new();
        app.add_message::<UiIntent>()
            .init_resource::<Received>()
            .init_resource::<ResolvedUiMetrics>()
            .insert_resource(DevTimeView::Available { hours: 12.0 })
            .insert_resource(UiAssets {
                display: Handle::default(),
                body: Handle::default(),
                hex_cell: Handle::default(),
            })
            .add_systems(Update, (rebuild, emit_intents, receive).chain());
        app.world_mut()
            .spawn((DevTimeStatus, Text::new("Waiting…")));
        app.world_mut()
            .spawn((DevTimeControls::default(), Node::default()));

        app.update();
        let controls = {
            let world = app.world_mut();
            let mut query = world.query_filtered::<Entity, With<DevTimeControl>>();
            query.iter(world).collect::<Vec<_>>()
        };
        assert_eq!(controls.len(), 6);
        let stale = controls
            .first()
            .copied()
            .expect("available time must create controls");
        *app.world_mut().get_mut::<Interaction>(stale).unwrap() = Interaction::Pressed;
        *app.world_mut().resource_mut::<DevTimeView>() = DevTimeView::Unavailable {
            reason: "Static lighting profile".to_owned(),
        };

        app.update();

        let control_count = {
            let world = app.world_mut();
            let mut query = world.query_filtered::<Entity, With<DevTimeControl>>();
            query.iter(world).count()
        };
        assert_eq!(control_count, 0);
        assert!(app.world().resource::<Received>().0.is_empty());
        let view = DevTimeView::default();
        assert!(matches!(
            view,
            DevTimeView::Unavailable { ref reason } if !reason.is_empty()
        ));
    }

    #[test]
    fn controls_are_focusable_accessible_inspector_descendants() {
        let mut app = App::new();
        app.insert_resource(DevTimeView::Available { hours: 12.0 })
            .init_resource::<GameplayChromeView>()
            .insert_resource(crate::resolve_ui_metrics(
                Vec2::new(1920.0, 1080.0),
                crate::UiScaleMode::Auto,
            ))
            .insert_resource(UiAssets {
                display: Handle::default(),
                body: Handle::default(),
                hex_cell: Handle::default(),
            })
            .add_systems(Update, (spawn_panel, rebuild).chain());
        let gameplay_group = app
            .world_mut()
            .spawn((Name::new("Gameplay HUD Safe Frame"), TabGroup::new(0)))
            .id();
        let inspector = app
            .world_mut()
            .spawn((Name::new("Inspector HUD Region"), UiRegionRole::Inspector))
            .id();
        app.world_mut()
            .entity_mut(gameplay_group)
            .add_child(inspector);

        app.update();

        let controls = {
            let world = app.world_mut();
            let mut query = world.query_filtered::<
                (Entity, &Name, &TabIndex, &AccessibleLabel),
                With<DevTimeControl>,
            >();
            query
                .iter(world)
                .map(|(entity, name, index, label)| {
                    (entity, name.as_str().to_owned(), index.0, label.0.clone())
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(controls.len(), 6);
        for (entity, name, index, label) in controls {
            assert!(name.starts_with("Dev Time "));
            assert_eq!(index, 0);
            assert!(!label.is_empty());
            assert!(has_ancestor(app.world(), entity, inspector));
            assert!(has_ancestor(app.world(), entity, gameplay_group));
        }
    }

    #[test]
    fn compact_panel_leaves_the_hidden_inspector_without_recreating_controls() {
        let mut app = App::new();
        app.insert_resource(DevTimeView::Available { hours: 12.0 })
            .init_resource::<GameplayChromeView>()
            .insert_resource(crate::resolve_ui_metrics(
                Vec2::new(1280.0, 720.0),
                crate::UiScaleMode::Auto,
            ))
            .insert_resource(UiAssets {
                display: Handle::default(),
                body: Handle::default(),
                hex_cell: Handle::default(),
            })
            .add_systems(Startup, spawn_panel)
            .add_systems(Update, (rebuild, reconcile_layout).chain());
        let frame = app
            .world_mut()
            .spawn((Name::new("Gameplay HUD Safe Frame"), TabGroup::new(0)))
            .id();
        let inspector = app
            .world_mut()
            .spawn((Name::new("Inspector HUD Region"), UiRegionRole::Inspector))
            .id();
        app.world_mut().entity_mut(frame).add_child(inspector);

        app.update();
        app.update();

        let (panel, parent, position, width) = {
            let world = app.world_mut();
            let mut query = world.query_filtered::<(Entity, &ChildOf, &Node), With<DevTimePanel>>();
            query
                .iter(world)
                .next()
                .map(|(entity, parent, node)| {
                    (entity, parent.parent(), node.position_type, node.width)
                })
                .expect("the compact development panel must exist")
        };
        assert_eq!(parent, frame);
        assert_eq!(position, PositionType::Absolute);
        assert_eq!(width, Val::Px(256.0));

        let mut before = control_entities(app.world_mut());
        before.sort_by_key(|entity| entity.to_bits());
        assert_eq!(before.len(), 6);
        let focused = before
            .first()
            .copied()
            .expect("compact time controls must be reachable");
        app.insert_resource(InputFocus::from_entity(focused));

        *app.world_mut().resource_mut::<ResolvedUiMetrics>() =
            crate::resolve_ui_metrics(Vec2::new(1920.0, 1080.0), crate::UiScaleMode::Auto);
        app.update();

        assert_eq!(
            app.world().get::<ChildOf>(panel).map(ChildOf::parent),
            Some(inspector)
        );
        let mut after = control_entities(app.world_mut());
        after.sort_by_key(|entity| entity.to_bits());
        assert_eq!(after, before);
        assert_eq!(app.world().resource::<InputFocus>().get(), Some(focused));
    }

    #[test]
    fn ultra_constrained_controls_use_the_free_left_column() {
        let metrics =
            crate::resolve_ui_metrics(Vec2::new(960.0, 540.0), crate::UiScaleMode::Percent200);
        assert_eq!(metrics.viewport, UiViewportClass::Compact);
        assert!(!uses_ultra_middle_band(metrics));
        let panel = panel_node(metrics, false);
        let Val::Px(width) = panel.width else {
            panic!("the compact panel must have a bounded width");
        };
        let Val::Px(left) = panel.left else {
            panic!("the ultra-constrained panel must have a bounded left inset");
        };
        let Val::Px(top) = panel.top else {
            panic!("the compact panel must have a bounded top inset");
        };
        let Val::Px(height) = panel.height else {
            panic!("the compact panel must have a bounded height");
        };
        let mut actions = Node::default();
        crate::layout::constrain_region_to_canvas(metrics, UiRegionRole::Actions, &mut actions);
        let Val::Px(actions_left) = actions.left else {
            panic!("the action region must have a bounded left inset");
        };
        let action_rail_top = metrics.effective_size.y - 12.0 - 116.0;
        assert!(left + width < actions_left);
        assert!(top + height <= action_rail_top);
        assert!(2.0 * 82.0 + 4.0 <= width - 8.0);
    }

    #[test]
    fn common_two_hundred_percent_canvas_uses_the_gap_between_actions_and_rail() {
        let metrics =
            crate::resolve_ui_metrics(Vec2::new(1280.0, 720.0), crate::UiScaleMode::Percent200);
        assert!(uses_ultra_middle_band(metrics));
        let panel = panel_node(metrics, false);
        assert_eq!(panel.left, Val::Px(196.0));
        assert_eq!(panel.right, Val::Px(12.0));
        assert_eq!(panel.top, Val::Px(138.0));
        assert_eq!(panel.height, Val::Px(82.0));
        let panel_width = metrics.effective_size.x - 196.0 - 12.0;
        assert!(3.0 * 132.0 + 2.0 * 2.0 <= panel_width - 8.0);
        assert!(138.0 + 82.0 <= metrics.effective_size.y - 12.0 - 116.0);
    }

    #[test]
    fn compact_panel_and_controls_fit_without_covering_primary_gameplay_surfaces() {
        for logical_size in [
            UVec2::new(960, 540),
            UVec2::new(1280, 720),
            UVec2::new(1920, 1080),
        ] {
            for mode in [crate::UiScaleMode::Auto, crate::UiScaleMode::Percent200] {
                let mut app = App::new();
                app.add_plugins(crate::test_support::HeadlessUiPlugin::new(
                    logical_size.x,
                    logical_size.y,
                ));
                app.world_mut()
                    .insert_resource(crate::UiScalePreference(mode));
                app.world_mut()
                    .insert_resource(DevTimeView::Available { hours: 12.0 });
                app.world_mut().insert_resource(crate::GameplayChromeView {
                    shown: true,
                    decision_required: false,
                    encounter_complete: false,
                });
                app.world_mut()
                    .resource_mut::<NextState<Screen>>()
                    .set(Screen::Gameplay);
                for _ in 0..8 {
                    app.update();
                }

                let snapshot = crate::test_support::ui_tree_snapshot(app.world_mut());
                if snapshot.metrics.viewport != UiViewportClass::Compact {
                    continue;
                }
                let panel = snapshot
                    .nodes
                    .iter()
                    .find(|node| node.name == "Dev Time Panel")
                    .unwrap_or_else(|| {
                        panic!(
                            "the development time panel must be visible at {logical_size:?} in {mode:?}"
                        )
                    });
                assert!(panel.size.cmpgt(Vec2::ZERO).all());
                assert!(
                    !panel.overflows,
                    "the panel must fit at {logical_size:?} in {mode:?}: {panel:?}"
                );
                let panel_min = panel.center - panel.size * 0.5;
                let panel_max = panel.center + panel.size * 0.5;
                assert!(panel_min.cmpge(Vec2::ZERO).all());
                assert!(
                    panel_max.cmple(snapshot.metrics.effective_size).all(),
                    "the panel must remain on canvas at {logical_size:?} in {mode:?}: panel={panel:?}, metrics={:?}",
                    snapshot.metrics
                );

                for control_name in CONTROLS.map(|(name, _, _)| name) {
                    let control = snapshot
                        .nodes
                        .iter()
                        .find(|node| node.name == control_name)
                        .unwrap_or_else(|| {
                            panic!(
                                "{control_name:?} must be visible at {logical_size:?} in {mode:?}"
                            )
                        });
                    assert!(control.size.cmpgt(Vec2::ZERO).all());
                    assert!(
                        !control.overflows,
                        "{control_name:?} must fit at {logical_size:?} in {mode:?}: {control:?}"
                    );
                    let control_min = control.center - control.size * 0.5;
                    let control_max = control.center + control.size * 0.5;
                    assert!(control_min.cmpge(panel_min).all());
                    assert!(control_max.cmple(panel_max).all());
                }

                for primary_name in [
                    "Party HUD Region",
                    "Turn HUD Region",
                    "Actions HUD Region",
                    "Casting Panel",
                    "Primary Action Rail",
                ] {
                    let Some(primary) =
                        snapshot.nodes.iter().find(|node| node.name == primary_name)
                    else {
                        continue;
                    };
                    assert!(
                        !overlaps(panel, primary),
                        "the development panel must not cover {primary_name:?} at {logical_size:?} in {mode:?}: panel={panel:?}, primary={primary:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn required_choice_hides_ultra_constrained_controls_and_preserves_their_entities() {
        let mut app = App::new();
        app.add_plugins(crate::test_support::HeadlessUiPlugin::new(1280, 720));
        app.world_mut()
            .insert_resource(crate::UiScalePreference(crate::UiScaleMode::Percent200));
        app.world_mut()
            .insert_resource(DevTimeView::Available { hours: 12.0 });
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Gameplay);
        for _ in 0..8 {
            app.update();
        }

        let mut before = control_entities(app.world_mut());
        before.sort_by_key(|entity| entity.to_bits());
        assert_eq!(before.len(), 6);
        let focused = before
            .first()
            .copied()
            .expect("available time controls must be focusable");
        app.insert_resource(InputFocus::from_entity(focused));
        app.insert_resource(required_hud());
        *app.world_mut().resource_mut::<GameplayChromeView>() = GameplayChromeView {
            shown: true,
            decision_required: true,
            encounter_complete: false,
        };
        for _ in 0..4 {
            app.update();
        }

        let required = crate::test_support::ui_tree_snapshot(app.world_mut());
        assert!(required
            .nodes
            .iter()
            .all(|node| node.name != "Dev Time Panel"));
        assert!(CONTROLS
            .iter()
            .all(|(name, _, _)| required.nodes.iter().all(|node| node.name != *name)));
        assert_eq!(app.world().resource::<InputFocus>().get(), None);
        let rail = required
            .nodes
            .iter()
            .find(|node| node.name == "Primary Action Rail")
            .expect("the required action rail must remain visible");
        assert!(rail.size.cmpgt(Vec2::ZERO).all());

        *app.world_mut().resource_mut::<GameplayChromeView>() = GameplayChromeView {
            shown: true,
            decision_required: false,
            encounter_complete: false,
        };
        app.insert_resource(crate::GameplayHudView::default());
        for _ in 0..4 {
            app.update();
        }

        let restored = crate::test_support::ui_tree_snapshot(app.world_mut());
        assert!(restored
            .nodes
            .iter()
            .any(|node| node.name == "Dev Time Panel"));
        let mut after = control_entities(app.world_mut());
        after.sort_by_key(|entity| entity.to_bits());
        assert_eq!(after, before);
    }

    #[test]
    fn unchanged_available_view_rebuilds_recreated_gameplay_panel() {
        let mut app = App::new();
        app.insert_resource(DevTimeView::Available { hours: 6.0 })
            .init_resource::<ResolvedUiMetrics>()
            .insert_resource(UiAssets {
                display: Handle::default(),
                body: Handle::default(),
                hex_cell: Handle::default(),
            })
            .add_systems(Update, rebuild);
        let first_status = app
            .world_mut()
            .spawn((DevTimeStatus, Text::new("Checking cyclic time…")))
            .id();
        let first_controls = app
            .world_mut()
            .spawn((DevTimeControls::default(), Node::default()))
            .id();

        app.update();
        assert_eq!(control_entities(app.world_mut()).len(), 6);

        for entity in control_entities(app.world_mut()) {
            assert!(app.world_mut().despawn(entity));
        }
        assert!(app.world_mut().despawn(first_controls));
        assert!(app.world_mut().despawn(first_status));
        let second_status = app
            .world_mut()
            .spawn((DevTimeStatus, Text::new("Checking cyclic time…")))
            .id();
        app.world_mut()
            .spawn((DevTimeControls::default(), Node::default()));

        app.update();

        assert_eq!(control_entities(app.world_mut()).len(), 6);
        assert_eq!(
            app.world()
                .get::<Text>(second_status)
                .map(|text| text.as_str()),
            Some("CURRENT · 6.0 h")
        );
    }

    #[test]
    fn available_hour_updates_preserve_control_entities_and_focus() {
        let mut app = App::new();
        app.insert_resource(DevTimeView::Available { hours: 12.0 })
            .init_resource::<ResolvedUiMetrics>()
            .insert_resource(UiAssets {
                display: Handle::default(),
                body: Handle::default(),
                hex_cell: Handle::default(),
            })
            .add_systems(Update, rebuild);
        app.world_mut()
            .spawn((DevTimeStatus, Text::new("Checking cyclic time…")));
        app.world_mut()
            .spawn((DevTimeControls::default(), Node::default()));
        app.update();
        let mut before = control_entities(app.world_mut());
        before.sort_by_key(|entity| entity.to_bits());
        let focused = before
            .first()
            .copied()
            .expect("available time must create controls");
        app.insert_resource(InputFocus::from_entity(focused));

        *app.world_mut().resource_mut::<DevTimeView>() = DevTimeView::Available { hours: 12.5 };
        app.update();

        let mut after = control_entities(app.world_mut());
        after.sort_by_key(|entity| entity.to_bits());
        assert_eq!(after, before);
        assert!(app.world().get_entity(focused).is_ok());
        assert_eq!(
            app.world().resource::<InputFocus>().get(),
            Some(focused),
            "updating the time must preserve keyboard focus"
        );
    }

    fn control_entities(world: &mut World) -> Vec<Entity> {
        let mut query = world.query_filtered::<Entity, With<DevTimeControl>>();
        query.iter(world).collect()
    }

    fn has_ancestor(world: &World, mut entity: Entity, wanted: Entity) -> bool {
        while let Some(parent) = world.get::<ChildOf>(entity) {
            entity = parent.parent();
            if entity == wanted {
                return true;
            }
        }
        false
    }

    fn overlaps(
        left: &crate::test_support::UiNodeObservation,
        right: &crate::test_support::UiNodeObservation,
    ) -> bool {
        let left_min = left.center - left.size * 0.5;
        let left_max = left.center + left.size * 0.5;
        let right_min = right.center - right.size * 0.5;
        let right_max = right.center + right.size * 0.5;
        left_min.x < right_max.x
            && left_max.x > right_min.x
            && left_min.y < right_max.y
            && left_max.y > right_min.y
    }

    fn required_hud() -> crate::GameplayHudView {
        let disabled = |reason: &str| crate::ActionAvailability::Disabled {
            reason: reason.to_owned(),
        };
        crate::GameplayHudView {
            phase: hex_core::GameplayPhase::Active,
            actor: Some(hex_core::UnitId(0)),
            actor_label: "Hedge Mage".to_owned(),
            round: "Round 1".to_owned(),
            movement_remaining: 2,
            action_remaining: true,
            required_prompt: Some("Choose the required cells in the lattice".to_owned()),
            actions: vec![
                crate::ActionAffordance {
                    action: crate::GameplayAction::ConfirmDecision,
                    label: "Confirm choice".to_owned(),
                    shortcut: Some("Enter".to_owned()),
                    availability: disabled("Choose the required cells in the lattice"),
                    priority: crate::ActionPriority::Required,
                },
                crate::ActionAffordance {
                    action: crate::GameplayAction::Channel,
                    label: "Channel".to_owned(),
                    shortcut: None,
                    availability: disabled("Resolve the required choice first"),
                    priority: crate::ActionPriority::Primary,
                },
                crate::ActionAffordance {
                    action: crate::GameplayAction::EndTurn,
                    label: "End turn".to_owned(),
                    shortcut: Some("Space".to_owned()),
                    availability: disabled("Resolve the required choice first"),
                    priority: crate::ActionPriority::Primary,
                },
            ],
        }
    }
}
