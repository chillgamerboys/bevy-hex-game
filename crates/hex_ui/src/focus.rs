use bevy::input_focus::{
    tab_navigation::{TabIndex, TabNavigationPlugin},
    InputFocus, InputFocusVisible,
};
use bevy::math::Affine2;
use bevy::prelude::*;
use bevy::ui::InteractionDisabled;
use bevy::ui_widgets::ScrollIntoView;

const FOCUS_COLOR: Color = Color::srgb(0.98, 0.86, 0.56);

#[derive(Component)]
struct LogicalTabIndex(i32);

pub(super) fn plugin(app: &mut App) {
    app.add_plugins(TabNavigationPlugin)
        .add_systems(PreUpdate, activate_focused_button)
        .add_systems(
            PostUpdate,
            (
                prepare_buttons,
                sync_focusability,
                scroll_focused_into_view,
                paint_keyboard_focus,
            )
                .chain(),
        );
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
}
