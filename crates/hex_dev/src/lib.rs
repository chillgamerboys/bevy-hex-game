//! Development tooling: world inspection and Bevy-native diagnostic overlays.
//!
//! Only compiled when `hex_game`'s `dev` feature is on, so `bevy-inspector-egui`
//! and egui stay out of shipping builds entirely. The previous version gated on a
//! runtime `if cfg!(debug_assertions)`, which reads like it excludes the inspector
//! but still linked it into every release binary.
//!
//! Depends on no other workspace crate. It used to re-register `HexCoord`,
//! `HexGrid`, `HexTile`, `Player`, and `PanOrbitCamera` — all five of which their
//! owning plugins already register — so adding a reflected component anywhere
//! meant editing this file too.

use bevy::dev_tools::diagnostics_overlay::{DiagnosticsOverlay, DiagnosticsOverlayPlugin};
use bevy::prelude::*;
use bevy::ui_render::prelude::GlobalUiDebugOptions;
use bevy_inspector_egui::bevy_egui::EguiPlugin;
use bevy_inspector_egui::quick::WorldInspectorPlugin;

/// Keyboard ownership from the developer inspector, separate from Bevy UI focus.
pub use bevy_inspector_egui::bevy_egui::input::EguiWantsInput as DevUiInputCapture;

#[derive(Component)]
struct DevDiagnostics;

/// Adds the world inspector and opt-in Bevy-native overlays.
pub fn plugin(app: &mut App) {
    if !app.is_plugin_added::<EguiPlugin>() {
        app.add_plugins(EguiPlugin::default());
    }
    app.add_plugins((WorldInspectorPlugin::new(), DiagnosticsOverlayPlugin))
        .add_systems(Update, toggle_overlays);
}

fn toggle_overlays(
    keys: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
    diagnostics: Query<Entity, With<DevDiagnostics>>,
    mut ui_debug: ResMut<GlobalUiDebugOptions>,
) {
    if keys.just_pressed(KeyCode::F2) {
        if diagnostics.is_empty() {
            commands.spawn((DevDiagnostics, DiagnosticsOverlay::fps()));
        } else {
            for entity in &diagnostics {
                commands.entity(entity).despawn();
            }
        }
    }
    if keys.just_pressed(KeyCode::F3) {
        ui_debug.toggle();
        ui_debug.show_hidden = true;
        ui_debug.show_clipped = true;
        ui_debug.outline_padding_box = true;
        ui_debug.outline_content_box = true;
        ui_debug.outline_scrollbars = true;
    }
}
