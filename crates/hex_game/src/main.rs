//! The game binary: app setup, plugin wiring, screens, and menus.
//!
//! Everything the game does hangs off `AppPlugin` here, so the composition of the
//! app is readable end to end without chasing plugin groups. This is also the only
//! crate that can see every other one — it is the wiring, and deliberately holds no
//! game logic of its own.

// Lets `bevy_lint`'s attributes be written in source without breaking a normal build.
#![cfg_attr(bevy_lint, feature(register_tool), register_tool(bevy))]
// Without this, launching the shipped game on Windows opens a console window
// behind it. Dev and map-review builds keep the console because their diagnostics
// are part of the workflow.
#![cfg_attr(
    not(any(feature = "dev", feature = "map-review")),
    windows_subsystem = "windows"
)]

use bevy::diagnostic::{EntityCountDiagnosticsPlugin, FrameTimeDiagnosticsPlugin};
use bevy::log::{BoxedLayer, LogPlugin};
use bevy::picking::mesh_picking::MeshPickingPlugin;
use bevy::prelude::*;
use bevy::render::settings::{InstanceFlags, WgpuSettings};
use bevy::render::RenderPlugin;
use hex_assets::DisplaySettings;
use hex_core::{AppSystems, GameplaySetup, PausableSystems, Pause, PerceptionSystems, Screen};

#[cfg(any(feature = "map-review", feature = "visual-walk"))]
mod capture;
#[cfg(feature = "dev")]
mod content_debug;
mod menus;
#[cfg(feature = "map-review")]
mod review;
mod scenarios;
mod screens;
#[cfg(feature = "visual-walk")]
mod walk;

fn main() -> AppExit {
    // Chain rather than replace: console builds keep the default stderr report,
    // and the windowed Windows release gets the panic into the log file — which
    // otherwise records a session that just stops with no last line.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        bevy::log::error!("panic: {info}");
        default_hook(info);
    }));
    App::new().add_plugins(AppPlugin).run()
}

/// A plain-text log file beside the executable, fresh each launch.
///
/// The shipped Windows build hides its console, so without this a crash in the
/// field leaves nothing to attach to a bug report. `Arc<File>` already
/// satisfies `MakeWriter`, so no logging dependency is added, and
/// `File::create` truncates — the file is *this* session, which is the version
/// someone actually wants when the game just died.
///
/// Failure to create the file downgrades to stderr-and-carry-on: a read-only
/// install directory must not stop the game from launching.
fn file_log_layer(_app: &mut App) -> Option<BoxedLayer> {
    use bevy::log::tracing_subscriber::{fmt, Layer};

    let path = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join("hex_game.log")))
        .unwrap_or_else(|| std::path::PathBuf::from("hex_game.log"));
    match std::fs::File::create(&path) {
        Ok(file) => Some(
            fmt::layer()
                .with_writer(std::sync::Arc::new(file))
                .with_ansi(false)
                .boxed(),
        ),
        Err(error) => {
            #[expect(
                clippy::print_stderr,
                reason = "the log subscriber does not exist yet while its own layer is being built; stderr is the only channel there is"
            )]
            {
                eprintln!("cannot create log file at {}: {error}", path.display());
            }
            None
        }
    }
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
            DefaultPlugins
                .set(RenderPlugin {
                    render_creation: WgpuSettings {
                        instance_flags: InstanceFlags::default()
                            | InstanceFlags::ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER,
                        ..default()
                    }
                    .into(),
                    ..default()
                })
                .set(LogPlugin {
                    custom_layer: file_log_layer,
                    ..default()
                }),
        );

        app.add_plugins(MeshPickingPlugin);

        // Frame-time + entity-count collectors are cheap and stay on in every
        // build: dev tooling reads them. The once-per-second printout belongs
        // to dev-shaped builds, where someone is watching a console — in the
        // shipped profile (windowed on Windows, and with the session log file
        // everywhere) it would mostly churn `hex_game.log` at a megabyte an
        // hour for nobody.
        app.add_plugins((
            FrameTimeDiagnosticsPlugin::default(),
            EntityCountDiagnosticsPlugin::default(),
        ));
        #[cfg(any(debug_assertions, feature = "dev", feature = "map-review"))]
        app.add_plugins(bevy::diagnostic::LogDiagnosticsPlugin::default());

        // Order the shared `Update` phases once, here. Systems that participate in
        // cross-crate timing opt into these sets; self-contained state, UI and
        // presentation systems can run outside them.
        app.configure_sets(
            Update,
            (
                AppSystems::TickTimers,
                AppSystems::RecordInput,
                AppSystems::Update,
            )
                .chain(),
        );

        // One gate for everything that must stop while paused. `Pause` is a
        // sub-state of `Screen::Gameplay`, so it does not exist in menus and this
        // condition is false there too.
        app.configure_sets(Update, PausableSystems.run_if(in_state(Pause(false))));
        app.configure_sets(
            Update,
            (
                PerceptionSystems::PublishAmbient,
                PerceptionSystems::ResolveIllumination,
                PerceptionSystems::ResolveObservation,
                PerceptionSystems::PublishKnowledge,
                PerceptionSystems::ApplyPresentation,
            )
                .chain()
                .in_set(AppSystems::Update),
        );

        // World construction is split across crates — `hex_map` builds terrain,
        // `hex_units` spawns actors, future perception publishes initial knowledge,
        // and `hex_world` frames the result. Systems in the same `OnEnter` schedule
        // otherwise run in unspecified order. Chaining also gives each step a sync
        // point, so entities spawned by one set are queryable by the next.
        app.configure_sets(
            OnEnter(Screen::Gameplay),
            (
                GameplaySetup::Resources,
                GameplaySetup::Terrain,
                GameplaySetup::Actors,
                GameplaySetup::Perception,
                GameplaySetup::View,
                GameplaySetup::Finalize,
            )
                .chain(),
        );
        app.configure_sets(
            OnEnter(Screen::Gameplay),
            (
                PerceptionSystems::PublishAmbient,
                PerceptionSystems::ResolveIllumination,
                PerceptionSystems::ResolveObservation,
                PerceptionSystems::PublishKnowledge,
                PerceptionSystems::ApplyPresentation,
            )
                .chain()
                .in_set(GameplaySetup::Perception),
        );

        app.add_plugins((
            hex_assets::plugin,
            hex_map::plugin,
            hex_world::plugin,
            hex_units::plugin,
            hex_perception::plugin,
            hex_combat::plugin,
            scenarios::plugin,
            screens::plugin,
            menus::plugin,
        ));

        #[cfg(feature = "map-review")]
        app.add_plugins(review::plugin);

        #[cfg(feature = "visual-walk")]
        app.add_plugins(walk::plugin);

        app.add_systems(Update, apply_display_settings);

        #[cfg(feature = "dev")]
        app.add_plugins((hex_dev::plugin, content_debug::plugin));
    }
}

/// Applies presentation settings to the window.
///
/// Runs continuously rather than once because the file can be edited while the
/// game is running; `is_changed` keeps it to actual changes. Vsync is left as the
/// default: it caps the frame rate to the display without capping it *below* the
/// display, and on an adaptive-refresh panel the driver already drops the rate
/// when nothing is moving. A fixed cap would cost input latency to save power the
/// hardware is already saving.
fn apply_display_settings(settings: Option<Res<DisplaySettings>>, mut windows: Query<&mut Window>) {
    let Some(settings) = settings else { return };
    if !settings.is_changed() {
        return;
    }
    for mut window in &mut windows {
        window.present_mode = settings.present_mode.into();
    }
}
