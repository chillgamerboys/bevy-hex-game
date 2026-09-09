//! Native Battle Mode process ownership, independent of tactical application state.

use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy::winit::{UpdateMode, WinitSettings};
use hex_core::Screen;

/// One explicit Main Menu request, consumed even when launching is unavailable.
#[derive(Resource)]
pub(crate) struct BattleLaunchRequest;

/// Process lifetime and immutable launch status projected into the Main Menu.
#[derive(Resource, Default)]
pub(crate) struct BattleLauncher {
    child: Option<RunningChild>,
    presentation: Option<SuspendedPresentation>,
    error: Option<String>,
}

impl BattleLauncher {
    /// Whether this application still owns a running Battle Mode child.
    pub(crate) fn running(&self) -> bool {
        self.child.is_some()
    }

    /// Most recent launch or child failure, cleared by the next launch attempt.
    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

/// Installs the sole child owner; combat plugins remain in the separate process.
pub(crate) fn plugin(app: &mut App) {
    app.init_resource::<BattleLauncher>()
        .init_resource::<LaunchBackend>()
        .init_resource::<ParentWindowPolicy>()
        .add_systems(PostUpdate, supervise);
}

/// Hiding is known to work on our Windows/macOS backends. Keep Linux's parent
/// rendered because Wayland cannot hide it; its disabled menu explains the child.
#[derive(Resource)]
struct ParentWindowPolicy {
    hide_while_running: bool,
}

// Linux's false happens to match bool::default; macOS/Windows must default true.
#[cfg_attr(
    not(any(target_os = "macos", target_os = "windows")),
    expect(
        clippy::derivable_impls,
        reason = "The false default on this target must remain true on macOS and Windows."
    )
)]
impl Default for ParentWindowPolicy {
    fn default() -> Self {
        Self {
            hide_while_running: cfg!(any(target_os = "macos", target_os = "windows")),
        }
    }
}

struct ChildExit {
    success: bool,
    description: String,
}

// This narrow OS boundary lets lifecycle tests drive ordinary Bevy schedules
// without recursively starting the test executable or opening a native window.
trait ChildProcess: Send + Sync {
    fn poll(&mut self) -> io::Result<Option<ChildExit>>;
    fn terminate(&mut self);
}

struct NativeChild(Child);

impl ChildProcess for NativeChild {
    fn poll(&mut self) -> io::Result<Option<ChildExit>> {
        self.0.try_wait().map(|status| {
            status.map(|status| ChildExit {
                success: status.success(),
                description: status.to_string(),
            })
        })
    }

    fn terminate(&mut self) {
        if matches!(self.0.try_wait(), Ok(Some(_))) {
            return;
        }
        // Child does not kill or reap on Drop. This owns normal application
        // shutdown; force-killing the parent is outside this portable contract.
        if let Err(error) = self.0.kill() {
            warn!(%error, "Could not stop Battle Mode child during shutdown");
            return;
        }
        if let Err(error) = self.0.wait() {
            warn!(%error, "Could not reap Battle Mode child during shutdown");
        }
    }
}

struct RunningChild {
    process: Box<dyn ChildProcess>,
    finished: bool,
}

impl RunningChild {
    fn poll(&mut self) -> io::Result<Option<ChildExit>> {
        let status = self.process.poll()?;
        self.finished = status.is_some();
        Ok(status)
    }
}

impl Drop for RunningChild {
    fn drop(&mut self) {
        if !self.finished {
            self.process.terminate();
        }
    }
}

#[derive(Resource)]
struct LaunchBackend(Box<dyn FnMut() -> io::Result<RunningChild> + Send + Sync>);

impl Default for LaunchBackend {
    fn default() -> Self {
        Self(Box::new(|| {
            let executable = std::env::current_exe()?;
            let child =
                launch_command(&executable, std::env::vars_os().map(|(key, _)| key)).spawn()?;
            Ok(RunningChild {
                process: Box::new(NativeChild(child)),
                finished: false,
            })
        }))
    }
}

fn launch_command(executable: &Path, environment_keys: impl Iterator<Item = OsString>) -> Command {
    let mut command = Command::new(executable);
    command.arg("--arena").stdin(Stdio::null());
    // Preserve the parent's cwd, asset root, library paths and ordinary OS
    // environment. Only automation and previous arena selection are discarded.
    for key in environment_keys {
        let normalized = key.to_string_lossy().to_ascii_uppercase();
        if ["HEX_ARENA_", "HEX_REVIEW_", "HEX_WALK_"]
            .iter()
            .any(|prefix| normalized.starts_with(prefix))
        {
            command.env_remove(key);
        }
    }
    // An ordinary visible start always lands on the existing Fort/Dragon menu.
    command
        .env("HEX_ARENA_MAP", "fort")
        .env("HEX_ARENA_ENCOUNTER", "dragon")
        .env("HEX_ARENA_CONTROL", "player");
    command
}

#[derive(Default)]
struct SuspendedPresentation {
    windows: Vec<(Entity, bool)>,
    cameras: Vec<(Entity, bool)>,
    winit: Option<WinitSettings>,
}

impl SuspendedPresentation {
    fn suspend(
        windows: &mut Query<(Entity, &mut Window), With<PrimaryWindow>>,
        cameras: &mut Query<(Entity, &mut Camera)>,
        winit: Option<&mut WinitSettings>,
        hide_while_running: bool,
    ) -> Self {
        let mut saved = Self::default();
        for (entity, mut window) in windows.iter_mut() {
            saved.windows.push((entity, window.visible));
            if hide_while_running {
                window.visible = false;
            }
        }
        for (entity, mut camera) in cameras.iter_mut() {
            saved.cameras.push((entity, camera.is_active));
            if hide_while_running {
                camera.is_active = false;
            }
        }
        if let Some(winit) = winit {
            saved.winit = Some(winit.clone());
            let mode = UpdateMode::reactive_low_power(Duration::from_millis(250));
            winit.focused_mode = mode;
            winit.unfocused_mode = mode;
        }
        saved
    }

    fn restore(
        self,
        windows: &mut Query<(Entity, &mut Window), With<PrimaryWindow>>,
        cameras: &mut Query<(Entity, &mut Camera)>,
        winit: Option<&mut WinitSettings>,
    ) {
        for (entity, visible) in self.windows {
            if let Ok((_, mut window)) = windows.get_mut(entity) {
                window.visible = visible;
                if visible {
                    // Best effort: compositor policy may refuse focus (Wayland
                    // also does not implement hiding), but the menu is restored.
                    window.focused = true;
                }
            }
        }
        for (entity, active) in self.cameras {
            if let Ok((_, mut camera)) = cameras.get_mut(entity) {
                camera.is_active = active;
            }
        }
        if let Some((saved, current)) = self.winit.zip(winit) {
            *current = saved;
        }
    }
}

fn supervise(
    mut commands: Commands,
    request: Option<Res<BattleLaunchRequest>>,
    screen: Option<Res<State<Screen>>>,
    mut launcher: ResMut<BattleLauncher>,
    mut backend: ResMut<LaunchBackend>,
    window_policy: Res<ParentWindowPolicy>,
    mut windows: Query<(Entity, &mut Window), With<PrimaryWindow>>,
    mut cameras: Query<(Entity, &mut Camera)>,
    mut winit: Option<ResMut<WinitSettings>>,
) {
    let was_running = launcher.running();
    let completed = launcher
        .child
        .as_mut()
        .and_then(|child| match child.poll() {
            Ok(None) => None,
            Ok(Some(status)) => Some((!status.success).then(|| {
                warn!(status = %status.description, "Battle Mode child failed");
                "Battle Mode closed unexpectedly. Please try again.".to_owned()
            })),
            Err(error) => {
                warn!(%error, "Could not monitor Battle Mode child");
                Some(Some(
                    "Battle Mode could not stay open. Please try again.".to_owned(),
                ))
            }
        });
    if let Some(error) = completed {
        launcher.child = None;
        if let Some(saved) = launcher.presentation.take() {
            saved.restore(&mut windows, &mut cameras, winit.as_deref_mut());
        }
        launcher.error = error;
    }
    if request.is_none() {
        return;
    }
    commands.remove_resource::<BattleLaunchRequest>();
    // A duplicate arriving on the same frame as child exit is consumed too.
    if was_running || !screen.is_some_and(|screen| *screen.get() == Screen::Title) {
        return;
    }
    launcher.error = None;
    match (backend.0)() {
        Ok(child) => {
            launcher.presentation = Some(SuspendedPresentation::suspend(
                &mut windows,
                &mut cameras,
                winit.as_deref_mut(),
                window_policy.hide_while_running,
            ));
            launcher.child = Some(child);
        }
        Err(error) => {
            warn!(%error, "Could not open Battle Mode child");
            launcher.error = Some("Battle Mode could not open. Please try again.".to_owned());
        }
    }
}

#[cfg(test)]
#[path = "battle_launcher_tests.rs"]
mod tests;
