//! Main menu.
//!
//! Scaffolding: it transitions correctly and is laid out for real buttons, but
//! the options themselves wait for the design doc. Keyboard-driven for now so
//! there is no button styling to throw away later.

use bevy::prelude::*;
use hex_core::Screen;

use super::{despawn_screen, screen_root};

pub(super) fn plugin(app: &mut App) {
    app.add_systems(OnEnter(Screen::Title), spawn_title);
    app.add_systems(Update, handle_input.run_if(in_state(Screen::Title)));
    app.add_systems(OnExit(Screen::Title), despawn_screen(Screen::Title));
}

fn spawn_title(mut commands: Commands) {
    commands
        .spawn(screen_root(Screen::Title, "Title Screen"))
        .with_children(|parent| {
            parent.spawn((
                Text::new("hex game"),
                TextFont::from_font_size(56.0),
                TextColor(Color::srgb(0.95, 0.95, 0.95)),
            ));
            parent.spawn((
                Text::new("press ENTER to play    -    ESC to quit"),
                TextFont::from_font_size(20.0),
                TextColor(Color::srgb(0.7, 0.7, 0.7)),
            ));
        });
}

fn handle_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut next: ResMut<NextState<Screen>>,
    mut exit: MessageWriter<AppExit>,
) {
    if keys.just_pressed(KeyCode::Enter) {
        next.set(Screen::Loading);
    }
    if keys.just_pressed(KeyCode::Escape) {
        exit.write(AppExit::Success);
    }
}
