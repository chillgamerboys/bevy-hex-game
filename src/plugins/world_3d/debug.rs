use bevy::prelude::*;
use bevy_inspector_egui::bevy_egui::EguiPlugin;
use bevy_inspector_egui::quick::WorldInspectorPlugin;

use crate::plugins::world_3d::{
    camera::PanOrbitCamera,
    hex::{HexCoord, HexGrid, HexTile},
    player::Player,
};

pub struct DebugPlugin;

impl Plugin for DebugPlugin {
    fn build(&self, app: &mut App) {
        if cfg!(debug_assertions) {
            if !app.is_plugin_added::<EguiPlugin>() {
                app.add_plugins(EguiPlugin::default());
            }
            app.add_plugins(WorldInspectorPlugin::new())
                .register_type::<HexCoord>()
                .register_type::<HexGrid>()
                .register_type::<HexTile>()
                .register_type::<Player>()
                .register_type::<PanOrbitCamera>();
        }
    }
}
