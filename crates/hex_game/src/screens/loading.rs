//! Waits for assets before letting gameplay spawn.
//!
//! This screen is what makes `OnEnter(Screen::Gameplay)` a safe place to build the
//! world. Before it existed, the grid spawned in `PreStartup` and the player in
//! `Startup`, and the only thing stopping the player system from reading a
//! resource that did not exist yet was the gap between those two schedules —
//! undocumented, unenforced, and one refactor away from a panic.

use bevy::prelude::*;
use hex_assets::GameAssets;
use hex_core::Screen;

use super::{despawn_screen, screen_root};

pub(super) fn plugin(app: &mut App) {
    app.add_systems(OnEnter(Screen::Loading), spawn_loading);
    app.add_systems(
        Update,
        enter_gameplay_when_ready.run_if(in_state(Screen::Loading)),
    );
    app.add_systems(OnExit(Screen::Loading), despawn_screen(Screen::Loading));
}

fn spawn_loading(mut commands: Commands) {
    commands
        .spawn(screen_root(Screen::Loading, "Loading Screen"))
        .with_children(|parent| {
            parent.spawn((
                Text::new("loading..."),
                TextFont::from_font_size(28.0),
                TextColor(Color::srgb(0.8, 0.8, 0.8)),
            ));
        });
}

fn enter_gameplay_when_ready(
    assets: Res<GameAssets>,
    asset_server: Res<AssetServer>,
    mut next: ResMut<NextState<Screen>>,
) {
    if assets.is_ready(&asset_server) {
        next.set(Screen::Gameplay);
    }
}
