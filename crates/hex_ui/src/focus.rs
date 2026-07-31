use bevy::input_focus::{
    tab_navigation::{TabIndex, TabNavigationPlugin},
    InputFocus, InputFocusVisible,
};
use bevy::prelude::*;

const FOCUS_COLOR: Color = Color::srgb(0.98, 0.86, 0.56);

pub(super) fn plugin(app: &mut App) {
    app.add_plugins(TabNavigationPlugin)
        .add_systems(PreUpdate, activate_focused_button)
        .add_systems(Update, (prepare_buttons, paint_keyboard_focus));
}

fn prepare_buttons(world: &mut World) {
    // Apply these components immediately. Screen transitions may despawn a freshly
    // added button later in this frame; queuing an EntityCommand here would then try
    // to mutate the stale entity when deferred commands flush.
    let buttons = {
        let mut query = world.query_filtered::<(Entity, Option<&Name>), Added<Button>>();
        query
            .iter(world)
            .map(|(entity, name)| {
                let label =
                    name.map_or_else(|| "Button".to_owned(), |name| name.as_str().to_owned());
                (entity, label)
            })
            .collect::<Vec<_>>()
    };
    for (entity, label) in buttons {
        let Ok(mut entity) = world.get_entity_mut(entity) else {
            continue;
        };
        if !entity.contains::<TabIndex>() {
            entity.insert(TabIndex(0));
        }
        if !entity.contains::<AccessibleLabel>() {
            entity.insert(AccessibleLabel::new(label));
        }
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
    buttons: Query<Entity, With<Button>>,
    mut commands: Commands,
) {
    if !focus.is_changed() && !visible.is_changed() {
        return;
    }
    for entity in &buttons {
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
