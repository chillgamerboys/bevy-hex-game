//! Scripted visual walk: drive the real game through its screens and
//! photograph each step, so an agent (or a human) can look at the frames.
//!
//! Compiled only with the default-off `visual-walk` feature. Setting
//! `HEX_WALK_SCRIPT` to a RON step list and `HEX_WALK_OUT` to an output
//! directory runs the walk on launch: the runner advances one step at a time
//! (waiting for screens, settling frames, injecting clicks and keys, capturing
//! PNGs) and exits with success only if every step completed. A per-step
//! watchdog turns a stall into a diagnostic and a failing exit instead of a
//! hang.
//!
//! # Why clicks are injected as `Interaction::Pressed`
//!
//! `bevy_ui`'s focus system only resets a node's `Interaction` when it is not
//! `Pressed` — an injected press on a button the real cursor is nowhere near
//! is deliberately left alone ("press sticks until release"). Every handler in
//! this game reads `Changed<Interaction>` + `== Pressed`, so one injected
//! insert is exactly one activation, exercised through the real button wiring
//! rather than a state-bypass. The runner clears the press to
//! `Interaction::None` on the following step for buttons that outlive their
//! click. Keys go through `ButtonInput::press` from `PreUpdate`, after the
//! input plugin's frame clear, so `just_pressed` is visible to every `Update`
//! reader in the same frame.

use std::env;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use bevy::camera::RenderTarget;
use bevy::input::InputSystems;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use hex_assets::ScenarioLibrary;
use hex_core::{GameplaySetupFailure, ResolvedMapSeed, Screen, TerrainReady};
use serde::Deserialize;

use crate::capture::write_png;
use crate::scenarios::ScenarioToLoad;

const SCRIPT_ENV: &str = "HEX_WALK_SCRIPT";
const OUT_ENV: &str = "HEX_WALK_OUT";
const STEP_TIMEOUT: Duration = Duration::from_secs(60);
const WALK_WIDTH: u32 = 1920;
const WALK_HEIGHT: u32 = 1080;

/// Installs the walk runner only when its environment is present.
pub(super) fn plugin(app: &mut App) {
    let script = env::var(SCRIPT_ENV).ok();
    let out = env::var(OUT_ENV).ok();
    let (script, out) = match (script, out) {
        (None, None) => return,
        (Some(script), Some(out)) => (script, out),
        (Some(_), None) => {
            install_config_error(app, format!("{SCRIPT_ENV} requires {OUT_ENV}"));
            return;
        }
        (None, Some(_)) => {
            install_config_error(app, format!("{OUT_ENV} requires {SCRIPT_ENV}"));
            return;
        }
    };

    let steps = match load_script(&script) {
        Ok(steps) => steps,
        Err(error) => {
            install_config_error(app, error);
            return;
        }
    };

    info!(
        "visual walk: {} steps from {script}, output to {out}",
        steps.len()
    );
    app.insert_resource(WalkState::new(steps, PathBuf::from(out)))
        .add_systems(PreUpdate, run_walk.after(InputSystems));
}

fn install_config_error(app: &mut App, error: String) {
    app.insert_resource(WalkConfigurationError(error))
        .add_systems(Startup, reject_invalid_configuration);
}

#[derive(Resource, Debug)]
struct WalkConfigurationError(String);

fn reject_invalid_configuration(
    error: Res<WalkConfigurationError>,
    mut exit: MessageWriter<AppExit>,
) {
    error!("invalid visual-walk configuration: {}", error.0);
    exit.write(AppExit::error());
}

/// One scripted action. The RON script is a `Vec<WalkStep>`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
enum WalkStep {
    /// Wait until the app is in the named screen.
    AwaitScreen(String),
    /// Wait until validated terrain is ready (gameplay only).
    AwaitTerrain,
    /// Let this many frames pass before the next step.
    Settle(u32),
    /// Photograph the primary window into `<out>/<name>.png`.
    Capture(String),
    /// Press the `index`-th button whose `Name` starts with `name`.
    Click {
        name: String,
        #[serde(default)]
        index: usize,
    },
    /// Press and release a key: `Backspace`, `Escape`, `Space`, or `Enter`.
    Key(String),
    /// Launch a scenario by exact name, bypassing the menu UI.
    StartScenario {
        name: String,
        #[serde(default)]
        seed: Option<u64>,
    },
}

fn load_script(path: &str) -> Result<Vec<WalkStep>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read {SCRIPT_ENV} {path}: {error}"))?;
    let steps: Vec<WalkStep> =
        ron::from_str(&text).map_err(|error| format!("cannot parse {path}: {error}"))?;
    if steps.is_empty() {
        return Err(format!("{path} contains no steps"));
    }
    for (index, step) in steps.iter().enumerate() {
        validate_step(step).map_err(|error| format!("{path} step {index}: {error}"))?;
    }
    Ok(steps)
}

fn validate_step(step: &WalkStep) -> Result<(), String> {
    match step {
        WalkStep::AwaitScreen(name) => parse_screen(name).map(|_| ()),
        WalkStep::Key(name) => parse_key(name).map(|_| ()),
        WalkStep::Capture(name) if name.trim().is_empty() => {
            Err("capture name must not be empty".to_owned())
        }
        WalkStep::Click { name, .. } if name.trim().is_empty() => {
            Err("click name must not be empty".to_owned())
        }
        WalkStep::StartScenario { name, .. } if name.trim().is_empty() => {
            Err("scenario name must not be empty".to_owned())
        }
        _ => Ok(()),
    }
}

fn parse_screen(name: &str) -> Result<Screen, String> {
    match name {
        "Splash" => Ok(Screen::Splash),
        "Title" => Ok(Screen::Title),
        "LatticeDemo" => Ok(Screen::LatticeDemo),
        "Loading" => Ok(Screen::Loading),
        "Gameplay" => Ok(Screen::Gameplay),
        _ => Err(format!(
            "unknown screen {name:?}; expected Splash, Title, LatticeDemo, Loading, or Gameplay"
        )),
    }
}

fn parse_key(name: &str) -> Result<KeyCode, String> {
    match name {
        "Backspace" => Ok(KeyCode::Backspace),
        "Escape" => Ok(KeyCode::Escape),
        "Space" => Ok(KeyCode::Space),
        "Enter" => Ok(KeyCode::Enter),
        _ => Err(format!(
            "unknown key {name:?}; expected Backspace, Escape, Space, or Enter"
        )),
    }
}

/// What the capture observer reports back to the runner.
#[derive(Debug)]
enum CaptureOutcome {
    Written { brightest: u8, coverage: bool },
    Failed(String),
}

#[derive(Resource)]
struct WalkState {
    steps: Vec<WalkStep>,
    cursor: usize,
    out_dir: PathBuf,
    settled: u32,
    step_started: Instant,
    capture_requested: bool,
    capture_outcome: Option<CaptureOutcome>,
    /// A button pressed by the previous step, to be reset to `None`.
    pressed: Option<Entity>,
    /// A key pressed by the previous step, to be released.
    held_key: Option<KeyCode>,
    /// The offscreen image the camera renders into for capture.
    ///
    /// The window surface is not readable on every backend — on macOS/Metal a
    /// `Screenshot::primary_window()` comes back black — so the walk redirects
    /// the game's single camera into a target image exactly as the map-review
    /// harness does. `bevy_ui` only follows a *window*-targeting camera by
    /// default, so the runner also tags every UI root with `UiTargetCamera`
    /// pointing at the redirected camera; frames then show everything a
    /// player would see.
    target: Option<Handle<Image>>,
    /// The camera entity the UI roots must be pointed at.
    camera: Option<Entity>,
    failed: bool,
}

impl WalkState {
    fn new(steps: Vec<WalkStep>, out_dir: PathBuf) -> Self {
        Self {
            steps,
            cursor: 0,
            out_dir,
            settled: 0,
            step_started: Instant::now(),
            capture_requested: false,
            capture_outcome: None,
            pressed: None,
            held_key: None,
            target: None,
            camera: None,
            failed: false,
        }
    }

    fn advance(&mut self) {
        self.cursor += 1;
        self.settled = 0;
        self.capture_requested = false;
        self.capture_outcome = None;
        self.step_started = Instant::now();
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the runner is one sequential state machine over the app's surfaces; \
              splitting it would scatter the step semantics across systems"
)]
fn run_walk(
    mut commands: Commands,
    mut state: ResMut<WalkState>,
    screen: Res<State<Screen>>,
    mut next: ResMut<NextState<Screen>>,
    terrain: Option<Res<TerrainReady>>,
    failure: Option<Res<GameplaySetupFailure>>,
    library: Option<Res<ScenarioLibrary>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    buttons: Query<(Entity, &Name), With<Button>>,
    mut images: ResMut<Assets<Image>>,
    mut camera_targets: Query<(Entity, &mut RenderTarget), With<Camera>>,
    ui_roots: Query<Entity, (With<Node>, Without<ChildOf>, Without<UiTargetCamera>)>,
    mut exit: MessageWriter<AppExit>,
) {
    if state.failed {
        return;
    }

    // Redirect the game's single camera into a readable offscreen image before
    // anything is photographed; see `WalkState::target`.
    if state.target.is_none() {
        let Ok((camera, mut render_target)) = camera_targets.single_mut() else {
            return;
        };
        let image =
            Image::new_target_texture(WALK_WIDTH, WALK_HEIGHT, TextureFormat::Rgba8UnormSrgb, None);
        let handle = images.add(image);
        *render_target = RenderTarget::Image(handle.clone().into());
        state.target = Some(handle);
        state.camera = Some(camera);
    }

    // UI roots spawn and despawn with every screen; keep pointing new ones at
    // the redirected camera or their screens render into nothing.
    if let Some(camera) = state.camera {
        for root in &ui_roots {
            commands.entity(root).insert(UiTargetCamera(camera));
        }
    }
    if let Some(failure) = failure {
        error!(
            "visual walk aborted: gameplay setup failed: {}",
            failure.reason
        );
        state.failed = true;
        exit.write(AppExit::error());
        return;
    }

    // Cleanups owed from the previous step, regardless of what runs now.
    if let Some(entity) = state.pressed.take() {
        if buttons.contains(entity) {
            commands.entity(entity).insert(Interaction::None);
        }
    }
    if let Some(key) = state.held_key.take() {
        keys.release(key);
    }

    let Some(step) = state.steps.get(state.cursor).cloned() else {
        info!("visual walk complete: {} steps", state.steps.len());
        exit.write(AppExit::Success);
        state.failed = true;
        return;
    };

    if state.step_started.elapsed() > STEP_TIMEOUT {
        error!(
            "visual walk timed out on step {} ({step:?}) after {:.0}s on screen {:?}",
            state.cursor,
            STEP_TIMEOUT.as_secs_f32(),
            screen.get()
        );
        state.failed = true;
        exit.write(AppExit::error());
        return;
    }

    match step {
        WalkStep::AwaitScreen(ref name) => {
            let wanted = parse_screen(name).unwrap_or(Screen::Title);
            if *screen.get() == wanted {
                state.advance();
            }
        }
        WalkStep::AwaitTerrain => {
            if terrain.is_some() {
                state.advance();
            }
        }
        WalkStep::Settle(frames) => {
            state.settled += 1;
            if state.settled >= frames {
                state.advance();
            }
        }
        WalkStep::Capture(ref name) => {
            if !state.capture_requested {
                let Some(target) = state.target.clone() else {
                    return;
                };
                let path = state.out_dir.join(format!("{name}.png"));
                info!("visual walk capturing {}", path.display());
                commands.spawn(Screenshot::image(target)).observe(
                    move |captured: On<ScreenshotCaptured>, mut state: ResMut<WalkState>| {
                        let outcome = match write_png(&captured.image, &path) {
                            Ok(stats) => CaptureOutcome::Written {
                                brightest: stats.brightest,
                                coverage: stats.has_coverage,
                            },
                            Err(error) => CaptureOutcome::Failed(error),
                        };
                        state.capture_outcome = Some(outcome);
                    },
                );
                state.capture_requested = true;
                return;
            }
            match state.capture_outcome.take() {
                None => {}
                Some(CaptureOutcome::Written {
                    brightest,
                    coverage,
                }) if brightest > 8 => {
                    info!("visual walk captured {name} (coverage: {coverage})");
                    state.advance();
                }
                Some(CaptureOutcome::Written { .. }) => {
                    error!("visual walk capture {name} came back black; failing the walk");
                    state.failed = true;
                    exit.write(AppExit::error());
                }
                Some(CaptureOutcome::Failed(error)) => {
                    error!("visual walk capture {name} failed: {error}");
                    state.failed = true;
                    exit.write(AppExit::error());
                }
            }
        }
        WalkStep::Click { ref name, index } => {
            let mut matches: Vec<(Entity, String)> = buttons
                .iter()
                .filter(|(_, button_name)| button_name.as_str().starts_with(name.as_str()))
                .map(|(entity, button_name)| (entity, button_name.as_str().to_owned()))
                .collect();
            // Query iteration order is arbitrary; entity order tracks spawn
            // order within one screen build, which is deterministic.
            matches.sort_by_key(|&(entity, _)| entity);
            let Some(&(entity, ref matched)) = matches.get(index) else {
                return; // keep waiting; the watchdog reports if it never appears
            };
            info!(
                "visual walk clicking {matched:?} ({} of {})",
                index,
                matches.len()
            );
            commands.entity(entity).insert(Interaction::Pressed);
            state.pressed = Some(entity);
            state.advance();
        }
        WalkStep::Key(ref name) => {
            let key = parse_key(name).unwrap_or(KeyCode::Escape);
            info!("visual walk pressing {name}");
            keys.press(key);
            state.held_key = Some(key);
            state.advance();
        }
        WalkStep::StartScenario { ref name, seed } => {
            let Some(library) = library.as_deref() else {
                return;
            };
            let Some(scenario) = library
                .scenarios
                .iter()
                .find(|scenario| scenario.name == *name)
                .cloned()
            else {
                error!("visual walk: scenario {name:?} is not in scenarios.ron");
                state.failed = true;
                exit.write(AppExit::error());
                return;
            };
            let resolved_seed = seed.or(scenario.generation_seed).map(ResolvedMapSeed);
            info!("visual walk launching scenario {name:?}");
            commands.insert_resource(ScenarioToLoad {
                scenario,
                resolved_seed,
            });
            next.set(Screen::Loading);
            state.advance();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL_SCRIPT: &str = r#"[
        AwaitScreen("Title"),
        Settle(30),
        Capture("01-title"),
        Click(name: "Lattice Demo"),
        AwaitScreen("LatticeDemo"),
        Key("Backspace"),
        StartScenario(name: "The Crossing"),
        AwaitTerrain,
        Capture("02-crossing"),
    ]"#;

    #[test]
    fn a_full_script_parses_with_every_step_kind() {
        let steps: Vec<WalkStep> = ron::from_str(FULL_SCRIPT).expect("script parses");
        assert_eq!(steps.len(), 9);
        assert_eq!(steps.first(), Some(&WalkStep::AwaitScreen("Title".into())));
        assert_eq!(
            steps.get(3),
            Some(&WalkStep::Click {
                name: "Lattice Demo".into(),
                index: 0
            })
        );
        assert_eq!(
            steps.get(6),
            Some(&WalkStep::StartScenario {
                name: "The Crossing".into(),
                seed: None
            })
        );
        for step in &steps {
            validate_step(step).expect("every step validates");
        }
    }

    #[test]
    fn unknown_screens_and_keys_are_rejected_at_load() {
        assert!(validate_step(&WalkStep::AwaitScreen("Menu".into())).is_err());
        assert!(validate_step(&WalkStep::Key("F13".into())).is_err());
        assert!(validate_step(&WalkStep::Capture(" ".into())).is_err());
        assert!(validate_step(&WalkStep::Click {
            name: String::new(),
            index: 0
        })
        .is_err());
    }

    #[test]
    fn every_screen_name_round_trips() {
        for name in ["Splash", "Title", "LatticeDemo", "Loading", "Gameplay"] {
            parse_screen(name).expect("known screen parses");
        }
        assert!(parse_screen("Gameplay ").is_err());
    }

    #[test]
    fn the_shipped_walk_scripts_parse_and_validate() {
        for script in ["../../walks/menus.ron", "../../walks/gameplay.ron"] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(script);
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
            let steps: Vec<WalkStep> = ron::from_str(&text)
                .unwrap_or_else(|error| panic!("cannot parse {}: {error}", path.display()));
            assert!(!steps.is_empty());
            for step in &steps {
                validate_step(step)
                    .unwrap_or_else(|error| panic!("{} invalid: {error}", path.display()));
            }
        }
    }
}
