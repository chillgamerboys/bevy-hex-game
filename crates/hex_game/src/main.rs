// Lets `bevy_lint`'s attributes be written in source without breaking a normal build.
#![cfg_attr(bevy_lint, feature(register_tool), register_tool(bevy))]
// Without this, launching the shipped game on Windows opens a console window
// behind it. Kept off for dev builds, where stdout is the log.
#![cfg_attr(not(feature = "dev"), windows_subsystem = "windows")]

use bevy::diagnostic::{
    EntityCountDiagnosticsPlugin, FrameTimeDiagnosticsPlugin, LogDiagnosticsPlugin,
};
use bevy::picking::mesh_picking::MeshPickingPlugin;
use bevy::prelude::*;
use bevy::render::settings::{InstanceFlags, WgpuSettings};
use bevy::render::RenderPlugin;

fn main() -> AppExit {
    App::new().add_plugins(AppPlugin).run()
}

/// The root plugin. Everything the game does hangs off this one place, so the
/// composition of the app is readable end to end without chasing plugin groups.
pub struct AppPlugin;

impl Plugin for AppPlugin {
    fn build(&self, app: &mut App) {
        // Linux/WSL2 only, inert everywhere else (notably macOS/Metal, which has
        // no non-conformant adapters to filter). Allowing non-conformant Vulkan
        // adapters is what lets wgpu pick Mesa Dozen — the D3D12 translation layer
        // that reaches the host GPU through /usr/lib/wsl/lib/libd3d12.so — instead
        // of falling back to llvmpipe software rendering. Without the flag, wgpu
        // filters Dozen out and renders on CPU: single-digit FPS even on a discrete
        // NVIDIA card. Don't remove it; it costs nothing on other platforms.
        app.add_plugins(
            DefaultPlugins.set(RenderPlugin {
                render_creation: WgpuSettings {
                    instance_flags: InstanceFlags::default()
                        | InstanceFlags::ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER,
                    ..default()
                }
                .into(),
                ..default()
            }),
        );

        app.add_plugins(MeshPickingPlugin);

        // Frame-time + entity-count diagnostics are cheap and logged to stdout
        // once per second by LogDiagnosticsPlugin. Always on so release-build
        // perf is observable without rebuilding.
        app.add_plugins((
            FrameTimeDiagnosticsPlugin::default(),
            EntityCountDiagnosticsPlugin::default(),
            LogDiagnosticsPlugin::default(),
        ));

        app.add_plugins((hex_assets::plugin, hex_world::plugin, hex_gameplay::plugin));

        #[cfg(feature = "dev")]
        app.add_plugins(hex_dev::plugin);
    }
}
