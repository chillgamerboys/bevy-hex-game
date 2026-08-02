//! Development-only cyclic time controls.

use bevy::prelude::*;
use hex_core::Screen;

use crate::{
    blurb, fine, heading, layout::is_ultra_constrained, panel, theme::fixed_row_button,
    DevTimeIntent, DevTimeView, GameplayChromeView, HudElement, ResolvedUiMetrics, UiAssets,
    UiHudSetup, UiIntent, UiRegionRole, UiSystems, UiViewportClass,
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
        spawn_panel.in_set(UiHudSetup::Tooling),
    )
    .add_systems(
        Update,
        (rebuild, reconcile_layout)
            .chain()
            .in_set(UiSystems::Render)
            .run_if(in_state(Screen::Gameplay)),
    )
    .add_systems(
        Update,
        emit_intents
            .in_set(UiSystems::EmitIntents)
            .run_if(in_state(Screen::Gameplay)),
    );
}

fn spawn_panel(
    mut commands: Commands,
    assets: Res<UiAssets>,
    metrics: Res<ResolvedUiMetrics>,
    chrome: Res<GameplayChromeView>,
    review: Option<Res<crate::review::UiReviewPresentation>>,
    regions: Query<(Entity, &UiRegionRole)>,
) {
    let Some(inspector) = regions
        .iter()
        .find_map(|(entity, role)| (*role == UiRegionRole::Inspector).then_some(entity))
    else {
        return;
    };
    let chrome = review
        .as_ref()
        .map_or(*chrome, |review| review.effective_chrome(*chrome));
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
            panel.spawn((
                DevTimeStatus,
                Node {
                    width: Val::Percent(100.0),
                    ..default()
                },
                blurb(&assets, "Checking cyclic time…"),
            ));
            panel.spawn((
                Name::new("Dev Time Controls"),
                DevTimeControls::default(),
                controls_node(*metrics),
                Pickable::IGNORE,
            ));
        })
        .id();
    commands.entity(inspector).add_child(panel);
}

fn panel_is_collapsed(metrics: ResolvedUiMetrics, decision_required: bool) -> bool {
    is_ultra_constrained(metrics) && (decision_required || metrics.effective_size.y < 400.0)
}

fn panel_node(metrics: ResolvedUiMetrics, decision_required: bool) -> Node {
    let mut node = Node {
        width: Val::Percent(100.0),
        flex_shrink: 0.0,
        flex_direction: FlexDirection::Column,
        row_gap: Val::Px(8.0),
        padding: UiRect::all(Val::Px(12.0)),
        border: UiRect::all(Val::Px(1.0)),
        border_radius: BorderRadius::all(Val::Px(10.0)),
        ..default()
    };
    if metrics.viewport == UiViewportClass::Compact {
        node.row_gap = Val::Px(4.0);
        if is_ultra_constrained(metrics) {
            node.row_gap = Val::Px(2.0);
            node.padding = UiRect::all(Val::Px(4.0));
        } else {
            node.padding = UiRect::all(Val::Px(6.0));
        }
    }
    // Development tooling is secondary. At extreme semantic density there is
    // not enough room for six legible 44px controls beside the action rail, so
    // collapse the panel instead of clipping it or covering player actions.
    if panel_is_collapsed(metrics, decision_required) {
        node.display = Display::None;
    }
    node
}

fn controls_node(metrics: ResolvedUiMetrics) -> Node {
    let gap = if is_ultra_constrained(metrics) {
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
    metrics: Res<ResolvedUiMetrics>,
    chrome: Res<GameplayChromeView>,
    review: Option<Res<crate::review::UiReviewPresentation>>,
    added_panels: Query<(), Added<DevTimePanel>>,
    added_roots: Query<(), Added<DevTimeControls>>,
    added_controls: Query<(), Added<DevTimeControl>>,
    mut panels: Query<&mut Node, (With<DevTimePanel>, Without<DevTimeHeading>)>,
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
        && review.as_ref().is_none_or(|review| !review.is_changed())
        && added_panels.is_empty()
        && added_roots.is_empty()
        && added_controls.is_empty()
    {
        return;
    }

    let chrome = review
        .as_ref()
        .map_or(*chrome, |review| review.effective_chrome(*chrome));
    if let Ok(mut node) = panels.single_mut() {
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
    let (width, height) = control_size(*metrics);
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

fn control_size(metrics: ResolvedUiMetrics) -> (f32, f32) {
    if is_ultra_constrained(metrics) && metrics.content_scale >= 1.5 {
        (172.0, 44.0)
    } else if is_ultra_constrained(metrics) {
        (82.0, 44.0)
    } else if metrics.viewport == UiViewportClass::Compact {
        (76.0, 36.0)
    } else {
        (96.0, 48.0)
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
            let (width, height) = control_size(*metrics);
            commands.entity(controls_entity).with_children(|controls| {
                for (name, label, intent) in CONTROLS {
                    controls
                        .spawn((
                            fixed_row_button(name, width, height),
                            DevTimeControl(intent),
                        ))
                        .with_child(fine(&assets, label));
                }
            });
        }
        controls.available = available;
    }

    match view.as_ref() {
        DevTimeView::Available { hours } => {
            **status = if is_ultra_constrained(*metrics) {
                format!("TIME · {hours:.1} h")
            } else if metrics.viewport == UiViewportClass::Compact {
                format!("DEV · TIME · {hours:.1} h")
            } else {
                format!("CURRENT · {hours:.1} h")
            };
        }
        DevTimeView::Unavailable { reason } => {
            **status = if is_ultra_constrained(*metrics) {
                format!("TIME · UNAVAILABLE\n{reason}")
            } else if metrics.viewport == UiViewportClass::Compact {
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
    #[cfg(feature = "test-support")]
    use hex_core::AppSystems;

    use super::*;

    #[cfg(feature = "test-support")]
    #[derive(Resource, Default)]
    struct RequiredChoiceTransition(bool);

    #[cfg(feature = "test-support")]
    fn activate_required_choice_for_render(
        mut transition: ResMut<RequiredChoiceTransition>,
        mut hud: ResMut<crate::GameplayHudView>,
        mut chrome: ResMut<GameplayChromeView>,
    ) {
        if !transition.0 {
            return;
        }
        transition.0 = false;
        *hud = required_hud();
        *chrome = GameplayChromeView {
            shown: true,
            decision_required: true,
            encounter_complete: false,
        };
    }

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
    fn compact_panel_stays_in_the_scrollable_inspector_without_recreating_controls() {
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
        assert_eq!(parent, inspector);
        assert_eq!(position, PositionType::Relative);
        assert_eq!(width, Val::Percent(100.0));
        let control_sizes = {
            let world = app.world_mut();
            let mut query = world.query_filtered::<&Node, With<DevTimeControl>>();
            query
                .iter(world)
                .map(|node| (node.width, node.height))
                .collect::<Vec<_>>()
        };
        assert_eq!(control_sizes.len(), 6);
        assert!(
            control_sizes
                .iter()
                .all(|size| *size == (Val::Px(76.0), Val::Px(36.0))),
            "new compact controls must use their final size in the first rendered frame"
        );

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
    fn ultra_constrained_controls_reflow_inside_the_inspector() {
        let metrics = crate::resolve_ui_metrics(Vec2::new(960.0, 540.0), crate::UiScaleMode::Auto);
        assert_eq!(metrics.viewport, UiViewportClass::Compact);
        let panel = panel_node(metrics, false);
        let (control_width, control_height) = control_size(metrics);
        assert_eq!(panel.position_type, PositionType::Relative);
        assert_eq!(panel.width, Val::Percent(100.0));
        assert!(control_height >= 44.0);
        assert!(control_width * 3.0 + 16.0 <= crate::layout::inspector_width(metrics));
    }

    #[test]
    fn common_two_hundred_percent_canvas_collapses_secondary_controls() {
        let metrics =
            crate::resolve_ui_metrics(Vec2::new(1280.0, 720.0), crate::UiScaleMode::Percent200);
        let panel = panel_node(metrics, false);
        assert_eq!(panel.position_type, PositionType::Relative);
        assert_eq!(panel.width, Val::Percent(100.0));
        assert_eq!(panel.display, Display::None);
        let (control_width, control_height) = control_size(metrics);
        assert!(control_width * 2.0 + 12.0 <= crate::layout::inspector_width(metrics));
        assert!(control_height >= 44.0);
    }

    #[cfg(feature = "test-support")]
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
                app.update();
                let expected_size = control_size(*app.world().resource::<ResolvedUiMetrics>());
                let first_frame_sizes = {
                    let world = app.world_mut();
                    let mut query = world.query_filtered::<&Node, With<DevTimeControl>>();
                    query
                        .iter(world)
                        .map(|node| (node.width, node.height))
                        .collect::<Vec<_>>()
                };
                assert_eq!(first_frame_sizes.len(), 6);
                assert!(first_frame_sizes
                    .iter()
                    .all(|size| { *size == (Val::Px(expected_size.0), Val::Px(expected_size.1)) }));
                for _ in 0..7 {
                    app.update();
                }

                let snapshot = crate::test_support::ui_tree_snapshot(app.world_mut());
                if snapshot.metrics.viewport != UiViewportClass::Compact {
                    continue;
                }
                let Some(panel) = snapshot
                    .nodes
                    .iter()
                    .find(|node| node.name == "Dev Time Panel")
                else {
                    assert!(
                        is_ultra_constrained(snapshot.metrics)
                            && snapshot.metrics.effective_size.y < 400.0,
                        "the development panel may collapse only when secondary tooling cannot fit at {logical_size:?} in {mode:?}: {:?}",
                        snapshot.metrics
                    );
                    continue;
                };
                assert!(panel.size.cmpgt(Vec2::ZERO).all());
                assert!(
                    !panel.overflows,
                    "the panel must fit at {logical_size:?} in {mode:?}: {panel:?}"
                );
                let panel_min = panel.center - panel.size * 0.5;
                let panel_max = panel.center + panel.size * 0.5;
                assert!(panel_min.cmpge(Vec2::ZERO).all());
                assert!(
                    panel_max.cmple(snapshot.metrics.logical_size).all(),
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

    #[cfg(feature = "test-support")]
    #[test]
    fn required_choice_hides_ultra_constrained_controls_and_preserves_their_entities() {
        let mut app = App::new();
        app.add_plugins(crate::test_support::HeadlessUiPlugin::new(1280, 720))
            .init_resource::<RequiredChoiceTransition>()
            .add_systems(
                Update,
                activate_required_choice_for_render.in_set(AppSystems::Update),
            );
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
        app.world_mut().resource_mut::<RequiredChoiceTransition>().0 = true;
        app.update();

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
        app.world_mut()
            .insert_resource(crate::UiScalePreference(crate::UiScaleMode::Auto));
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

    #[cfg(feature = "test-support")]
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

    #[cfg(feature = "test-support")]
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
