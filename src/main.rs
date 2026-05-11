use bevy::diagnostic::{
    EntityCountDiagnosticsPlugin, FrameTimeDiagnosticsPlugin, LogDiagnosticsPlugin,
};
use bevy::picking::mesh_picking::MeshPickingPlugin;
use bevy::prelude::*;
use bevy::render::settings::{InstanceFlags, WgpuSettings};
use bevy::render::RenderPlugin;

use magic_game::plugins::world_3d::World3dPlugins;

fn main() {
    App::new()
        // Allow non-conformant Vulkan adapters so wgpu picks Mesa Dozen (D3D12
        // translation layer) on WSL2 + Mesa instead of falling back to llvmpipe
        // software rendering. Dozen targets the host GPU via /usr/lib/wsl/lib/
        // libd3d12.so. Without this flag, wgpu filters Dozen out and renders on
        // CPU — single-digit FPS even on a discrete NVIDIA card.
        .add_plugins(DefaultPlugins.set(RenderPlugin {
            render_creation: WgpuSettings {
                instance_flags: InstanceFlags::default()
                    | InstanceFlags::ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER,
                ..default()
            }
            .into(),
            ..default()
        }))
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
