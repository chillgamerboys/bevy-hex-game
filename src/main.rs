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
        // Linux/WSL2 only, inert everywhere else (notably macOS/Metal, which has
        // no non-conformant adapters to filter). Allowing non-conformant Vulkan
        // adapters is what lets wgpu pick Mesa Dozen — the D3D12 translation layer
        // that reaches the host GPU through /usr/lib/wsl/lib/libd3d12.so — instead
        // of falling back to llvmpipe software rendering. Without the flag, wgpu
        // filters Dozen out and renders on CPU: single-digit FPS even on a discrete
        // NVIDIA card. Don't remove it; it costs nothing on other platforms.
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
