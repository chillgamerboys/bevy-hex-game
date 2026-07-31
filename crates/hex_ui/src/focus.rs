use bevy::input_focus::{
    tab_navigation::{TabIndex, TabNavigationPlugin},
    InputFocus, InputFocusVisible,
};
use bevy::prelude::*;
use bevy::ui::InteractionDisabled;

const FOCUS_COLOR: Color = Color::srgb(0.98, 0.86, 0.56);

#[derive(Component)]
struct LogicalTabIndex(i32);

pub(super) fn plugin(app: &mut App) {
    app.add_plugins(TabNavigationPlugin)
        .add_systems(PreUpdate, activate_focused_button)
        .add_systems(
            PostUpdate,
            (prepare_buttons, sync_focusability, paint_keyboard_focus).chain(),
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
        world
            .get::<TabIndex>(entity)
            .is_some_and(|index| index.0 < 0)
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
}
