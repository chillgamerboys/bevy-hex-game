//! Application composition for the standalone workshop.

use bevy::prelude::*;
use bevy::window::{PresentMode, WindowResolution};
use bevy_egui::EguiPlugin;

/// Starts the Asset Workshop.
pub fn run() {
    App::new()
        .insert_resource(ClearColor(Color::srgb(0.055, 0.06, 0.07)))
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Bevy Hex Asset Workshop".to_owned(),
                name: Some("hex-editor".to_owned()),
                resolution: WindowResolution::new(1440, 900),
                present_mode: PresentMode::AutoVsync,
                ..default()
            }),
            ..default()
        }))
        .add_plugins(EguiPlugin::default())
        .run();
}
