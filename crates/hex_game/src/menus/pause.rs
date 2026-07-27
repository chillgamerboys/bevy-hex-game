//! Pause overlay.
//!
//! Exists because a pause with no visible state is indistinguishable from a
//! crash: the world simply stops responding and nothing explains why. Anything
//! that suspends the game needs to say so on screen.

use bevy::prelude::*;
use hex_core::Pause;

use super::overlay_root;
use super::widgets::{blurb, display, UiAssets};

pub(super) fn plugin(app: &mut App) {
    app.add_systems(OnEnter(Pause(true)), spawn_pause_menu);
    app.add_systems(OnExit(Pause(true)), despawn_pause_menu);
}

#[derive(Component)]
struct PauseMenu;

fn spawn_pause_menu(mut commands: Commands, assets: Res<UiAssets>) {
    commands
        .spawn((overlay_root("Pause Menu"), PauseMenu))
        .with_children(|parent| {
            parent.spawn(display(&assets, "Paused"));
            parent.spawn(blurb(
                &assets,
                "ESC to resume   ·   BACKSPACE to quit to title",
            ));
        });
}

fn despawn_pause_menu(mut commands: Commands, menus: Query<Entity, With<PauseMenu>>) {
    for entity in &menus {
        commands.entity(entity).despawn();
    }
}
