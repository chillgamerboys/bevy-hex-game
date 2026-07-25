//! Development tooling: the world inspector.
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

use bevy::prelude::*;
use bevy_inspector_egui::bevy_egui::EguiPlugin;
use bevy_inspector_egui::quick::WorldInspectorPlugin;

/// Adds the world inspector.
pub fn plugin(app: &mut App) {
    if !app.is_plugin_added::<EguiPlugin>() {
        app.add_plugins(EguiPlugin::default());
    }
    app.add_plugins(WorldInspectorPlugin::new());
}
