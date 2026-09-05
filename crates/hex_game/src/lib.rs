//! Game application composition and domain-to-presentation adapters.
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
use bevy::diagnostic::{EntityCountDiagnosticsPlugin, FrameTimeDiagnosticsPlugin};
use bevy::log::{BoxedLayer, LogPlugin};
use bevy::picking::mesh_picking::MeshPickingPlugin;
use bevy::prelude::*;
use bevy::render::settings::{InstanceFlags, WgpuSettings};
use bevy::render::RenderPlugin;
use bevy::window::WindowMode;
use bevy::winit::WinitPlugin;
use hex_core::{
    AppSystems, GameplayPhase, GameplaySetup, InputBindings, PausableSystems, Pause,
    PerceptionSystems, Screen,
};

pub mod campaign_authority;
#[cfg(any(feature = "map-review", feature = "visual-walk", feature = "v4-world"))]
mod capture;
mod casting;
#[cfg(feature = "dev")]
mod content_debug;
mod creation_store;
#[cfg(feature = "dev-time-preview")]
mod dev_time_controls;
mod fog;
mod menus;
mod multiplayer_gameplay;
mod preferences;
mod readouts;
#[cfg(feature = "map-review")]
mod review;
mod save;
mod scenarios;
mod screens;
mod spell_vfx;
mod storage;
mod terrain_health_bars;
#[cfg(feature = "test-support")]
pub mod test_support;
/// Isolated V4 world-package explorer and windowless integration harness.
#[cfg(feature = "v4-world")]
pub mod v4;
#[cfg(feature = "visual-walk")]
mod walk;

/// Builds and runs the shipping application.
pub fn run() -> AppExit {
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

/// Native play starts fullscreen before the first rendered frame.
///
/// Preferences still project immediately after startup, so an explicit persisted
/// windowed choice wins. Review-shaped builds start windowed when used manually;
/// automated captures later replace Winit with a windowless schedule runner.
fn initial_window_mode() -> WindowMode {
    #[cfg(any(feature = "map-review", feature = "visual-walk"))]
    {
        WindowMode::Windowed
    }
    #[cfg(not(any(feature = "map-review", feature = "visual-walk")))]
    {
        WindowMode::BorderlessFullscreen(bevy::window::MonitorSelection::Primary)
    }
}

/// Whether this launch will write automated visual evidence rather than be played.
///
/// Both automation paths render into an explicit image target. They retain a logical
/// `PrimaryWindow` entity for UI layout but replace Winit's native event loop, so no
/// operating-system window should appear or take focus. A review-feature build
/// without either automation request remains an ordinary, visible game launch.
fn automated_capture_requested() -> bool {
    #[cfg(any(feature = "map-review", feature = "visual-walk"))]
    {
        automated_capture_requested_for(
            cfg!(feature = "map-review"),
            std::env::var_os("HEX_REVIEW_CAPTURE").is_some(),
            cfg!(feature = "visual-walk"),
            std::env::var_os("HEX_WALK_SCRIPT").is_some(),
            std::env::var_os("HEX_WALK_OUT").is_some(),
        )
    }
    #[cfg(not(any(feature = "map-review", feature = "visual-walk")))]
    {
        false
    }
}

#[cfg(any(feature = "map-review", feature = "visual-walk", test))]
const fn automated_capture_requested_for(
    map_review_enabled: bool,
    review_capture_present: bool,
    visual_walk_enabled: bool,
    walk_script_present: bool,
    walk_output_present: bool,
) -> bool {
    (map_review_enabled && review_capture_present)
        || (visual_walk_enabled && (walk_script_present || walk_output_present))
}

fn initial_window(automated_capture: bool) -> Window {
    Window {
        title: storage::APP_NAME.to_owned(),
        mode: initial_window_mode(),
        visible: !automated_capture,
        focused: !automated_capture,
        ..default()
    }
}

/// The root plugin. Everything the game does hangs off this one place, so the
/// composition of the app is readable end to end without chasing plugin groups.
pub struct AppPlugin;

impl Plugin for AppPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<InputBindings>()
            .init_resource::<GameplayPhase>();
        // Linux/WSL2 only, inert everywhere else (notably macOS/Metal, which has
        // no non-conformant adapters to filter). Allowing non-conformant Vulkan
        // adapters is what lets wgpu pick Mesa Dozen — the D3D12 translation layer
        // that reaches the host GPU through /usr/lib/wsl/lib/libd3d12.so — instead
        // of falling back to llvmpipe software rendering. Without the flag, wgpu
        // filters Dozen out and renders on CPU: single-digit FPS even on a discrete
        // NVIDIA card. Don't remove it; it costs nothing on other platforms.
        let automated_capture = automated_capture_requested();
        let default_plugins = DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(initial_window(automated_capture)),
                ..default()
            })
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
            });
        if automated_capture {
            // Winit activates the macOS application during startup even if its
            // window is hidden. Automated captures already render into an Image,
            // so use Bevy's windowless runner and avoid creating or activating a
            // native application window at all.
            app.add_plugins(default_plugins.disable::<WinitPlugin>());
            app.add_plugins(bevy::app::ScheduleRunnerPlugin::run_loop(
                std::time::Duration::from_secs_f64(1.0 / 60.0),
            ));
        } else {
            app.add_plugins(default_plugins);
        }

        app.add_plugins(MeshPickingPlugin);
        app.add_plugins(bevy_hanabi::HanabiPlugin);
        app.add_plugins(hex_ui::UiPlugin);
        app.add_systems(Startup, log_app_identity);

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

        // A scripted walk must never inherit the operator's preferences, campaigns,
        // or creations. Install its disposable root before any
        // persistence-owning plugin initializes `StoragePaths`.
        #[cfg(feature = "visual-walk")]
        walk::isolate_storage(app);

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
        app.configure_sets(
            Update,
            PausableSystems
                .run_if(in_state(Pause(false)))
                .run_if(resource_equals(GameplayPhase::Active)),
        );
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
        // `hex_units` spawns actors, `hex_perception` publishes initial knowledge,
        // and `hex_world` frames the result. Systems in the same `OnEnter` schedule
        // otherwise run in unspecified order. Chaining also gives each step a sync
        // point, so entities spawned by one set are queryable by the next.
        app.configure_sets(
            OnEnter(Screen::Gameplay),
            (
                GameplaySetup::Resources,
                GameplaySetup::Terrain,
                GameplaySetup::Actors,
                GameplaySetup::Restore,
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
            creation_store::plugin,
            preferences::plugin,
            hex_objects::plugin,
            hex_map::plugin,
            hex_world::plugin,
            hex_units::plugin,
            hex_perception::plugin,
            hex_combat::plugin,
            scenarios::plugin,
            save::plugin,
            screens::plugin,
            menus::plugin,
            // After `screens`, which owns the sub-states the casting systems are gated
            // on, and after `menus`, which inserts the fonts its panel is built from.
            casting::plugin,
            readouts::plugin,
        ));
        app.add_plugins(spell_vfx::plugin);
        app.add_plugins((fog::plugin, terrain_health_bars::plugin));

        #[cfg(feature = "test-support")]
        app.add_plugins(test_support::plugin);

        #[cfg(feature = "map-review")]
        app.add_plugins(review::plugin);

        #[cfg(feature = "visual-walk")]
        app.add_plugins(walk::plugin);

        #[cfg(feature = "dev")]
        app.add_plugins((hex_dev::plugin, content_debug::plugin));

        #[cfg(feature = "dev-time-preview")]
        app.add_plugins(dev_time_controls::plugin);
    }
}

fn log_app_identity() {
    info!("starting {} ({})", storage::APP_NAME, storage::APP_ID);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_window_mode_matches_the_build_shape() {
        #[cfg(any(feature = "map-review", feature = "visual-walk"))]
        assert_eq!(initial_window_mode(), WindowMode::Windowed);
        #[cfg(not(any(feature = "map-review", feature = "visual-walk")))]
        assert_eq!(
            initial_window_mode(),
            WindowMode::BorderlessFullscreen(bevy::window::MonitorSelection::Primary)
        );
    }

    #[test]
    fn automated_capture_host_window_is_hidden_and_unfocused() {
        let automated = initial_window(true);
        assert!(!automated.visible);
        assert!(!automated.focused);

        let interactive = initial_window(false);
        assert!(interactive.visible);
        assert!(interactive.focused);
    }

    #[test]
    fn only_required_automation_environment_starts_the_windowless_runner() {
        assert!(automated_capture_requested_for(
            true, true, false, false, false
        ));
        assert!(automated_capture_requested_for(
            false, false, true, true, false
        ));
        assert!(automated_capture_requested_for(
            false, false, true, false, true
        ));
        assert!(!automated_capture_requested_for(
            true, false, true, false, false
        ));
        assert!(!automated_capture_requested_for(
            false, true, false, true, true
        ));
    }
}
