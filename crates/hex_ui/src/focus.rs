use bevy::input_focus::{
    tab_navigation::{TabGroup, TabIndex, TabNavigationPlugin},
    FocusCause, InputFocus, InputFocusVisible,
};
use bevy::math::Affine2;
use bevy::prelude::*;
use bevy::ui::InteractionDisabled;
use bevy::ui_widgets::ScrollIntoView;
use std::collections::HashMap;

const FOCUS_COLOR: Color = Color::srgb(0.98, 0.86, 0.56);

#[derive(Component)]
struct LogicalTabIndex(i32);

/// Marks a blocking tab scope whose highest visible instance must own keyboard focus.
///
/// Bevy confines Tab navigation to a modal [`TabGroup`] only after focus is already
/// inside that group. Runtime overlays use this marker so showing or rebuilding one
/// explicitly hands focus to its first (or previously focused) enabled control.
#[derive(Component, Debug, Clone, Copy)]
pub(crate) struct ModalFocusScope;

#[derive(Default, Resource)]
struct ModalFocusMemory {
    active_scope_name: Option<String>,
    preferred_controls: HashMap<String, String>,
    return_focus: Option<Entity>,
}

#[derive(Debug)]
struct FocusRefreshRequest {
    root: Entity,
    preferred_name: Option<String>,
}

#[derive(Default, Resource)]
pub(crate) struct FocusRefreshRequests(Vec<FocusRefreshRequest>);

pub(super) fn plugin(app: &mut App) {
    app.add_plugins(TabNavigationPlugin)
        .init_resource::<FocusRefreshRequests>()
        .init_resource::<ModalFocusMemory>()
        .add_systems(
            PreUpdate,
            activate_focused_button
                .after(bevy::input::InputSystems)
                .after(bevy::ui::UiSystems::Focus),
        )
        .add_systems(
            PostUpdate,
            (
                prepare_buttons,
                sync_focusability,
                restore_focus_after_refresh,
                retain_topmost_modal_focus,
                scroll_focused_into_view,
                paint_keyboard_focus,
            )
                .chain(),
        );
}

/// Clears focus before a route replaces its descendants and records where to
/// restore it after deferred hierarchy commands have been applied.
pub(crate) fn begin_route_refresh(
    root: Entity,
    focus: &mut InputFocus,
    parents: &Query<&ChildOf>,
    names: &Query<&Name>,
    requests: &mut FocusRefreshRequests,
) {
    let Some(focused) = focus.get() else { return };
    if !is_within_root(focused, root, parents) {
        return;
    }

    requests.0.push(FocusRefreshRequest {
        root,
        preferred_name: names.get(focused).ok().map(|name| name.as_str().to_owned()),
    });
    // This must be immediate. The old descendants are removed through deferred
    // Commands, and no later focus system may target one for ScrollIntoView.
    focus.clear();
}

fn is_within_root(mut entity: Entity, root: Entity, parents: &Query<&ChildOf>) -> bool {
    loop {
        if entity == root {
            return true;
        }
        let Ok(parent) = parents.get(entity) else {
            return false;
        };
        entity = parent.parent();
    }
}

fn prepare_buttons(world: &mut World) {
    // Apply these components immediately. Screen transitions may despawn a freshly
    // added button later in this frame; queuing an EntityCommand here would then try
    // to mutate the stale entity when deferred commands flush.
    let buttons = {
        let mut query =
            world.query_filtered::<(Entity, Option<&Name>, Option<&TabIndex>), Added<Button>>();
        query
            .iter(world)
            .map(|(entity, name, tab_index)| {
                let label =
                    name.map_or_else(|| "Button".to_owned(), |name| name.as_str().to_owned());
                (entity, label, tab_index.map_or(0, |index| index.0))
            })
            .collect::<Vec<_>>()
    };
    for (entity, label, logical_index) in buttons {
        let Ok(mut entity) = world.get_entity_mut(entity) else {
            continue;
        };
        if !entity.contains::<TabIndex>() {
            entity.insert(TabIndex(logical_index));
        }
        entity.insert(LogicalTabIndex(logical_index));
        if !entity.contains::<AccessibleLabel>() {
            entity.insert(AccessibleLabel::new(label));
        }
    }
}

fn sync_focusability(world: &mut World) {
    let controls = {
        let mut query = world.query::<(Entity, &LogicalTabIndex, &TabIndex)>();
        query
            .iter(world)
            .map(|(entity, logical, actual)| (entity, logical.0, actual.0))
            .collect::<Vec<_>>()
    };
    for (entity, logical, actual) in controls {
        let wanted = if is_reachable(world, entity) {
            logical
        } else {
            -1
        };
        if wanted != actual {
            world.entity_mut(entity).insert(TabIndex(wanted));
        }
    }

    let focused = world.resource::<InputFocus>().get();
    if focused.is_some_and(|entity| {
        world.get_entity(entity).is_err()
            || world
                .get::<TabIndex>(entity)
                .is_none_or(|index| index.0 < 0)
            || !is_reachable(world, entity)
    }) {
        world.resource_mut::<InputFocus>().clear();
    }
}

fn restore_focus_after_refresh(
    mut focus: ResMut<InputFocus>,
    mut requests: ResMut<FocusRefreshRequests>,
    children: Query<&Children>,
    controls: Query<(&TabIndex, Option<&Name>), With<Button>>,
) {
    for request in requests.0.drain(..) {
        let Some(entity) = first_reachable_control(
            request.root,
            request.preferred_name.as_deref(),
            &children,
            &controls,
        ) else {
            continue;
        };
        focus.set(entity, FocusCause::Navigated);
    }
}

/// Gives the visually highest presented modal scope explicit ownership of keyboard
/// focus and remembers the exact named control across deferred hierarchy rebuilds.
fn retain_topmost_modal_focus(world: &mut World) {
    let candidates = {
        let mut query = world.query_filtered::<(
            Entity,
            &TabGroup,
            Option<&GlobalZIndex>,
            Option<&ZIndex>,
            Option<&Name>,
        ), With<ModalFocusScope>>();
        query
            .iter(world)
            .filter(|(_, group, _, _, _)| group.modal)
            .map(|(entity, _, global, local, name)| {
                (
                    entity,
                    global.map_or(0, |index| index.0),
                    local.map_or(0, |index| index.0),
                    name.map(|name| name.as_str().to_owned()),
                )
            })
            .collect::<Vec<_>>()
    };
    let topmost = candidates
        .into_iter()
        .filter(|(entity, _, _, _)| is_reachable(world, *entity))
        .max_by_key(|(entity, global, local, _)| (*global, *local, entity.to_bits()));

    let current_focus = world.resource::<InputFocus>().get();
    let Some((root, _, _, scope_name)) = topmost else {
        let (was_active, return_focus) = {
            let mut memory = world.resource_mut::<ModalFocusMemory>();
            let was_active = memory.active_scope_name.take().is_some();
            memory.preferred_controls.clear();
            (was_active, memory.return_focus.take())
        };
        if !was_active {
            return;
        }

        // A route refresh may already have supplied a better live target while the
        // modal closed. Restore the pre-modal target only when focus is otherwise
        // empty or stale.
        if current_focus
            .is_some_and(|entity| world.get_entity(entity).is_ok() && is_reachable(world, entity))
        {
            return;
        }
        if let Some(return_focus) = return_focus {
            if world
                .get::<TabIndex>(return_focus)
                .is_some_and(|index| index.0 >= 0)
                && is_reachable(world, return_focus)
            {
                world
                    .resource_mut::<InputFocus>()
                    .set(return_focus, FocusCause::Navigated);
            }
        }
        return;
    };

    let scope_name = scope_name.unwrap_or_else(|| format!("Modal Scope {}", root.to_bits()));
    let same_scope = world
        .resource::<ModalFocusMemory>()
        .active_scope_name
        .as_deref()
        == Some(scope_name.as_str());
    if !same_scope {
        let return_focus = current_focus.filter(|entity| {
            !is_descendant_or_self(world, *entity, root)
                && world.get_entity(*entity).is_ok()
                && is_reachable(world, *entity)
        });
        let mut memory = world.resource_mut::<ModalFocusMemory>();
        if memory.active_scope_name.is_none() {
            memory.return_focus = return_focus;
        }
        memory.active_scope_name = Some(scope_name.clone());
    }

    if current_focus.is_some_and(|entity| {
        is_descendant_or_self(world, entity, root)
            && world
                .get::<TabIndex>(entity)
                .is_some_and(|index| index.0 >= 0)
            && is_reachable(world, entity)
    }) {
        let focused_name = current_focus
            .and_then(|entity| world.get::<Name>(entity))
            .map(|name| name.as_str().to_owned());
        if let Some(focused_name) = focused_name {
            world
                .resource_mut::<ModalFocusMemory>()
                .preferred_controls
                .insert(scope_name, focused_name);
        }
        return;
    }

    let preferred = world
        .resource::<ModalFocusMemory>()
        .preferred_controls
        .get(&scope_name)
        .cloned();
    let Some(target) = first_reachable_control_in_world(world, root, preferred.as_deref()) else {
        // A visible blocking modal with no enabled action must not leave a hidden
        // gameplay control focused underneath it.
        world.resource_mut::<InputFocus>().clear();
        return;
    };
    let target_name = world
        .get::<Name>(target)
        .map(|name| name.as_str().to_owned());
    world
        .resource_mut::<InputFocus>()
        .set(target, FocusCause::Navigated);
    if let Some(target_name) = target_name {
        world
            .resource_mut::<ModalFocusMemory>()
            .preferred_controls
            .insert(scope_name, target_name);
    }
}

fn first_reachable_control_in_world(
    world: &World,
    root: Entity,
    preferred_name: Option<&str>,
) -> Option<Entity> {
    let mut stack = vec![root];
    let mut first = None;
    while let Some(entity) = stack.pop() {
        if world.get::<Button>(entity).is_some()
            && world
                .get::<TabIndex>(entity)
                .is_some_and(|index| index.0 >= 0)
            && is_reachable(world, entity)
        {
            first.get_or_insert(entity);
            if preferred_name.is_some_and(|preferred| {
                world
                    .get::<Name>(entity)
                    .is_some_and(|name| name.as_str() == preferred)
            }) {
                return Some(entity);
            }
        }
        if let Some(children) = world.get::<Children>(entity) {
            stack.extend(children.iter().rev());
        }
    }
    first
}

fn is_descendant_or_self(world: &World, mut entity: Entity, ancestor: Entity) -> bool {
    loop {
        if entity == ancestor {
            return true;
        }
        let Some(parent) = world.get::<ChildOf>(entity) else {
            return false;
        };
        entity = parent.parent();
    }
}

fn first_reachable_control(
    root: Entity,
    preferred_name: Option<&str>,
    children: &Query<&Children>,
    controls: &Query<(&TabIndex, Option<&Name>), With<Button>>,
) -> Option<Entity> {
    let mut stack = vec![root];
    let mut first = None;
    while let Some(entity) = stack.pop() {
        if let Ok((tab_index, name)) = controls.get(entity) {
            if tab_index.0 >= 0 {
                first.get_or_insert(entity);
                if preferred_name
                    .is_some_and(|preferred| name.is_some_and(|name| name.as_str() == preferred))
                {
                    return Some(entity);
                }
            }
        }
        if let Ok(entity_children) = children.get(entity) {
            stack.extend(entity_children.iter().rev());
        }
    }
    first
}

fn is_reachable(world: &World, mut entity: Entity) -> bool {
    loop {
        if world.get::<InteractionDisabled>(entity).is_some()
            || world
                .get::<Visibility>(entity)
                .is_some_and(|visibility| *visibility == Visibility::Hidden)
            || world
                .get::<Node>(entity)
                .is_some_and(|node| node.display == Display::None)
        {
            return false;
        }
        let Some(parent) = world.get::<ChildOf>(entity) else {
            return true;
        };
        entity = parent.parent();
    }
}

fn activate_focused_button(
    keys: Res<ButtonInput<KeyCode>>,
    focus: Res<InputFocus>,
    mut buttons: Query<&mut Interaction, With<Button>>,
    mut previously_pressed: Local<Option<Entity>>,
) {
    if let Some(entity) = previously_pressed.take() {
        if let Ok(mut interaction) = buttons.get_mut(entity) {
            *interaction = Interaction::None;
        }
    }
    if !keys.any_just_pressed([KeyCode::Enter, KeyCode::Space]) {
        return;
    }
    let Some(entity) = focus.get() else { return };
    let Ok(mut interaction) = buttons.get_mut(entity) else {
        return;
    };
    *interaction = Interaction::Pressed;
    *previously_pressed = Some(entity);
}

fn scroll_focused_into_view(
    focus: Res<InputFocus>,
    parents: Query<&ChildOf>,
    nodes: Query<
        (&UiGlobalTransform, &ComputedNode),
        Without<crate::creator::CompactCreatorCanvasScroll>,
    >,
    mut canvases: Query<
        (
            &Node,
            &UiGlobalTransform,
            &ComputedNode,
            &mut ScrollPosition,
        ),
        With<crate::creator::CompactCreatorCanvasScroll>,
    >,
    mut commands: Commands,
) {
    if !focus.is_changed() {
        return;
    }
    if let Some(entity) = focus.get() {
        if let Some(canvas) = parents
            .iter_ancestors(entity)
            .find(|ancestor| canvases.contains(*ancestor))
        {
            let (
                Ok((target_transform, target_computed)),
                Ok((canvas_node, canvas_transform, canvas_computed, mut canvas_scroll)),
            ) = (nodes.get(entity), canvases.get_mut(canvas))
            else {
                commands.trigger(ScrollIntoView { entity });
                return;
            };
            let target_size = target_computed.size() * target_computed.inverse_scale_factor;
            let target_affine: Affine2 = target_transform.into();
            let target_pos = target_affine.translation * target_computed.inverse_scale_factor
                - target_size * 0.5;
            let canvas_size = canvas_computed.size() * canvas_computed.inverse_scale_factor;
            let canvas_affine: Affine2 = canvas_transform.into();
            let canvas_pos = canvas_affine.translation * canvas_computed.inverse_scale_factor
                - canvas_size * 0.5;
            let target_local_top_left = target_pos - canvas_pos + canvas_scroll.0;
            let target_local_bottom_right = target_local_top_left + target_size;
            let content_size =
                canvas_computed.content_size() * canvas_computed.inverse_scale_factor;
            let max_range = (content_size - canvas_size).max(Vec2::ZERO);

            if canvas_node.overflow.x == OverflowAxis::Scroll {
                if target_local_top_left.x < canvas_scroll.x {
                    canvas_scroll.x = target_local_top_left.x.clamp(0.0, max_range.x);
                } else if target_local_bottom_right.x > canvas_scroll.x + canvas_size.x {
                    canvas_scroll.x =
                        (target_local_bottom_right.x - canvas_size.x).clamp(0.0, max_range.x);
                }
            }
            if canvas_node.overflow.y == OverflowAxis::Scroll {
                if target_local_top_left.y < canvas_scroll.y {
                    canvas_scroll.y = target_local_top_left.y.clamp(0.0, max_range.y);
                } else if target_local_bottom_right.y > canvas_scroll.y + canvas_size.y {
                    canvas_scroll.y =
                        (target_local_bottom_right.y - canvas_size.y).clamp(0.0, max_range.y);
                }
            }
            // The generic owner now receives the canvas rather than the still
            // clipped cell, revealing this nested viewport in the outer page.
            commands.trigger(ScrollIntoView { entity: canvas });
            return;
        }
        // Bevy's ScrollArea observer walks to the nearest scroll owner and updates
        // it just enough to expose this control. This keeps keyboard focus and its
        // visible ring together without duplicating scroll geometry here.
        commands.trigger(ScrollIntoView { entity });
    }
}

fn paint_keyboard_focus(
    focus: Res<InputFocus>,
    visible: Res<InputFocusVisible>,
    focusable: Query<Entity, With<TabIndex>>,
    mut commands: Commands,
) {
    if !focus.is_changed() && !visible.is_changed() {
        return;
    }
    for entity in &focusable {
        if visible.0 && focus.get() == Some(entity) {
            commands.entity(entity).insert(Outline {
                color: FOCUS_COLOR,
                width: Val::Px(3.0),
                offset: Val::Px(2.0),
            });
        } else {
            commands.entity(entity).remove::<Outline>();
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::input_focus::tab_navigation::TabGroup;

    use super::*;

    #[derive(Component)]
    struct RouteRefreshReplacement;

    #[derive(Resource)]
    struct RouteRefreshFixture {
        root: Entity,
        requested: bool,
    }

    #[derive(Resource)]
    struct ScrollObservation {
        targets: Vec<Entity>,
        all_targets_were_live: bool,
    }

    impl Default for ScrollObservation {
        fn default() -> Self {
            Self {
                targets: Vec::new(),
                all_targets_were_live: true,
            }
        }
    }

    fn rebuild_fixture_route(
        mut fixture: ResMut<RouteRefreshFixture>,
        mut focus: ResMut<InputFocus>,
        mut requests: ResMut<FocusRefreshRequests>,
        parents: Query<&ChildOf>,
        names: Query<&Name>,
        mut commands: Commands,
    ) {
        if !fixture.requested {
            return;
        }
        fixture.requested = false;
        let root = fixture.root;
        begin_route_refresh(root, &mut focus, &parents, &names, &mut requests);
        commands.entity(root).despawn_related::<Children>();
        commands.entity(root).with_children(|route| {
            route.spawn((Name::new("Earlier Action"), Button, TabIndex(0)));
            route.spawn((
                Name::new("Persistent Action"),
                Button,
                TabIndex(0),
                RouteRefreshReplacement,
            ));
        });
    }

    fn record_scroll_target(
        scroll: On<ScrollIntoView>,
        entities: Query<Entity>,
        mut observation: ResMut<ScrollObservation>,
    ) {
        observation.all_targets_were_live &= entities.contains(scroll.entity);
        observation.targets.push(scroll.entity);
    }

    #[test]
    fn hidden_subtrees_leave_and_reenter_the_tab_order() {
        let mut app = App::new();
        app.init_resource::<InputFocus>()
            .add_systems(PostUpdate, (prepare_buttons, sync_focusability).chain());
        let root = app
            .world_mut()
            .spawn((TabGroup::new(0), Node::default(), Visibility::Inherited))
            .id();
        let button = app
            .world_mut()
            .spawn((Name::new("Action"), Button, TabIndex(0)))
            .id();
        app.world_mut().entity_mut(root).add_child(button);

        app.update();
        assert_eq!(app.world().get::<TabIndex>(button), Some(&TabIndex(0)));

        app.world_mut().get_mut::<Node>(root).unwrap().display = Display::None;
        app.update();
        assert_eq!(app.world().get::<TabIndex>(button), Some(&TabIndex(-1)));

        app.world_mut().get_mut::<Node>(root).unwrap().display = Display::Flex;
        app.update();
        assert_eq!(app.world().get::<TabIndex>(button), Some(&TabIndex(0)));
    }

    #[test]
    fn despawned_focus_is_cleared_before_a_rebuilt_control_can_receive_activation() {
        let mut app = App::new();
        app.init_resource::<InputFocus>()
            .init_resource::<ButtonInput<KeyCode>>()
            .add_systems(PreUpdate, activate_focused_button)
            .add_systems(PostUpdate, (prepare_buttons, sync_focusability).chain());
        let button = app
            .world_mut()
            .spawn((Name::new("Old Action"), Button, TabIndex(0)))
            .id();
        app.update();
        app.insert_resource(InputFocus::from_entity(button));

        app.world_mut().despawn(button);
        app.update();
        assert_eq!(app.world().resource::<InputFocus>().get(), None);

        let replacement = app
            .world_mut()
            .spawn((Name::new("New Action"), Button, TabIndex(0)))
            .id();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Enter);
        app.update();

        assert_ne!(
            app.world().get::<Interaction>(replacement),
            Some(&Interaction::Pressed),
            "a key meant for a despawned focused action must not activate its replacement"
        );
    }

    #[test]
    fn route_refresh_retargets_focus_before_scrolling_the_rebuilt_hierarchy() {
        let mut app = App::new();
        app.init_resource::<InputFocus>()
            .insert_resource(InputFocusVisible(true))
            .init_resource::<FocusRefreshRequests>()
            .init_resource::<ScrollObservation>()
            .add_observer(record_scroll_target)
            .add_systems(Update, rebuild_fixture_route)
            .add_systems(
                PostUpdate,
                (
                    prepare_buttons,
                    sync_focusability,
                    restore_focus_after_refresh,
                    scroll_focused_into_view,
                    paint_keyboard_focus,
                )
                    .chain(),
            );
        let root = app.world_mut().spawn_empty().id();
        let old = app
            .world_mut()
            .spawn((Name::new("Persistent Action"), Button, TabIndex(0)))
            .id();
        app.world_mut().entity_mut(root).add_child(old);
        app.insert_resource(RouteRefreshFixture {
            root,
            requested: false,
        });
        app.update();

        app.world_mut()
            .resource_mut::<InputFocus>()
            .set(old, FocusCause::Navigated);
        app.world_mut()
            .resource_mut::<RouteRefreshFixture>()
            .requested = true;
        app.update();

        assert!(app.world().get_entity(old).is_err());
        let replacement = app
            .world_mut()
            .query_filtered::<Entity, With<RouteRefreshReplacement>>()
            .single(app.world())
            .unwrap();
        assert_eq!(
            app.world().resource::<InputFocus>().get(),
            Some(replacement)
        );
        assert_eq!(app.world().get::<TabIndex>(replacement), Some(&TabIndex(0)));
        assert!(app.world().get::<Outline>(replacement).is_some());
        let observation = app.world().resource::<ScrollObservation>();
        assert!(observation.all_targets_were_live);
        assert_eq!(observation.targets.last(), Some(&replacement));
    }
}
