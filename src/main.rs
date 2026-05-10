use bevy::diagnostic::{
    EntityCountDiagnosticsPlugin, FrameTimeDiagnosticsPlugin, LogDiagnosticsPlugin,
};
use bevy::picking::mesh_picking::MeshPickingPlugin;
use bevy::prelude::*;

use magic_game::plugins::world_3d::World3dPlugins;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(MeshPickingPlugin)
        // Frame-time + entity-count diagnostics are cheap and logged to stdout
        // once per second by LogDiagnosticsPlugin. Always on so release-build
        // perf is observable without rebuilding.
        .add_plugins((
            FrameTimeDiagnosticsPlugin::default(),
            EntityCountDiagnosticsPlugin::default(),
            LogDiagnosticsPlugin::default(),
        ))
        .add_plugins(World3dPlugins)
        .run();
}
