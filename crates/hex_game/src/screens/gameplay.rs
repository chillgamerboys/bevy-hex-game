//! The game itself.
//!
//! Owns only the pause toggle and the route back to the title screen. The world
//! and the player are spawned by `hex_world` and `hex_gameplay` on
//! `OnEnter(Screen::Gameplay)`; this module deliberately does not reach into
//! either of them.

use bevy::prelude::*;
use hex_core::{Pause, Screen};

use super::{despawn_screen, DespawnOnExit};

pub(super) fn plugin(app: &mut App) {
    app.add_sub_state::<Pause>();
    app.register_type::<Pause>();

    app.add_systems(Update, handle_input.run_if(in_state(Screen::Gameplay)));
    app.add_systems(OnEnter(Screen::Gameplay), (reset_pause, spawn_hud));
    app.add_systems(OnExit(Screen::Gameplay), despawn_screen(Screen::Gameplay));
}

/// Controls are otherwise undiscoverable — there is no manual and no tutorial.
fn spawn_hud(mut commands: Commands) {
    commands
        .spawn((
            Name::new("Gameplay HUD"),
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(12.0),
                left: Val::Px(12.0),
                ..default()
            },
            DespawnOnExit(Screen::Gameplay),
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new(
                    "click a tile to move    -    right-drag to orbit    -    WASD to pan    \
                     -    scroll to zoom    -    ESC to pause",
                ),
                TextFont::from_font_size(14.0),
                TextColor(Color::srgba(0.9, 0.9, 0.9, 0.7)),
            ));
        });
}

/// Entering gameplay always starts unpaused, so a pause left set from a previous
/// session cannot leak into a new one.
fn reset_pause(mut next: ResMut<NextState<Pause>>) {
    next.set(Pause(false));
}

fn handle_input(
    keys: Res<ButtonInput<KeyCode>>,
    pause: Res<State<Pause>>,
    mut next_pause: ResMut<NextState<Pause>>,
    mut next_screen: ResMut<NextState<Screen>>,
) {
    if keys.just_pressed(KeyCode::Escape) {
        next_pause.set(Pause(!pause.get().0));
    }
    // Backspace rather than Escape, which is taken by pause.
    if keys.just_pressed(KeyCode::Backspace) {
        next_screen.set(Screen::Title);
    }
}
