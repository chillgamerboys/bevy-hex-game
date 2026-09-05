//! Scripted visual walk: drive the real game through its screens and
//! photograph each step, so an agent (or a human) can look at the frames.
//!
//! Compiled only with the default-off `visual-walk` feature. Setting
//! `HEX_WALK_SCRIPT` to a RON step list and `HEX_WALK_OUT` to an output
//! directory runs the walk on launch: the runner advances one step at a time
//! (waiting for screens, settling frames, injecting UI or exact terrain clicks and
//! keys, waiting for bounded party movement, capturing PNGs) and exits with success
//! only if every step completed. A per-step
//! watchdog turns a stall into a diagnostic and a failing exit instead of a
//! hang. `HEX_WALK_VIEWPORT=1280x720@2` optionally selects an exact logical
//! canvas and device scale; the default is 1920×1080@1.
//!
//! # How clicks are injected
//!
//! Named UI clicks use `Interaction::Pressed`. `bevy_ui`'s focus system only resets
//! a node's `Interaction` when it is not
//! `Pressed` — an injected press on a button the real cursor is nowhere near
//! is deliberately left alone ("press sticks until release"). Every handler in
//! this game reads `Changed<Interaction>` + `== Pressed`, so one injected
//! insert is exactly one activation, exercised through the real button wiring
//! rather than a state-bypass. Exact terrain clicks emit the ordinary primary
//! `Pointer<Click>` after stack-safe surface resolution. The runner clears each
//! named-button press to
//! `Interaction::None` on the following step for buttons that outlive their
//! click. Keys go through `ButtonInput::press` from `PreUpdate`, after the
//! input plugin's frame clear, so `just_pressed` is visible to every `Update`
//! reader in the same frame.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bevy::camera::{ClearColorConfig, ImageRenderTarget, NormalizedRenderTarget, RenderTarget};
use bevy::ecs::system::SystemParam;
use bevy::input::InputSystems;
use bevy::picking::backend::HitData;
use bevy::picking::events::{Click, Over, Pointer};
use bevy::picking::pointer::{Location, PointerButton, PointerId};
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::window::CursorMoved;
#[cfg(test)]
use hex_assets::SandboxMapCatalog;
use hex_assets::{Encounter, EncounterFaction, ScenarioLibrary};
use hex_core::{
    Busy, CameraFocusTarget, CommandQueue, GameplaySetup, GameplaySetupFailure, Headroom, HexCoord,
    HexTile, MapAnchorId, MapAnchors, ResolvedMapSeed, Screen, TilePos,
};
use hex_units::{MovingTo, Party, Selected, StandsOn, UnitRegistry};
use hex_world::CameraMode;
use serde::Deserialize;

use crate::capture::{install_capture, prepare_capture_path, temporary_capture_path, write_png};
use crate::scenarios::ScenarioToLoad;

/// Marks an explicitly configured windowless script; never present in native play.
#[derive(Resource)]
pub(crate) struct AutomatedWalk;

/// Orders synthetic input before consumers that capture edges for exploration.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum WalkSystems {
    InjectInput,
}

const SCRIPT_ENV: &str = "HEX_WALK_SCRIPT";
const OUT_ENV: &str = "HEX_WALK_OUT";
const VIEWPORT_ENV: &str = "HEX_WALK_VIEWPORT";
const UI_DEBUG_ENV: &str = "HEX_WALK_UI_DEBUG";
const DATA_ENV: &str = "HEX_GAME_DATA_DIR";
const REVIEW_INDEX_FILE: &str = "review-index.md";
const STEP_TIMEOUT: Duration = Duration::from_secs(60);
/// The windowless walk owns deterministic simulation time. Renderer readback and
/// PNG encoding may take arbitrarily long wall-clock time without advancing a
/// character farther between two requested evidence frames.
const WALK_FRAME_DURATION: Duration = Duration::from_nanos(16_666_667);
const WALK_TIME_SCALE: f32 = 12.0;
/// Temporal evidence runs at shipped simulation speed. Ordinary setup and the
/// uncaptured remainder of long routes retain the accelerated walk speed.
const TEMPORAL_CAPTURE_TIME_SCALE: f32 = 1.0;
const MAX_ORBIT_YAW_TURNS: f32 = 0.5;
const MAX_ORBIT_PITCH_FRACTION: f32 = 1.0;
/// A temporal diagnostic is intentionally a small bounded image sequence, not
/// an unbounded video recorder.
const MAX_MOVEMENT_CAPTURE_FILES: u16 = 48;
const MAX_MOVEMENT_CAPTURE_FILES_PER_WALK: usize = 192;
const MAX_MOVEMENT_CAPTURE_FRAMES: u32 = 900;
/// A terrain click can take a few schedules to publish its command and route.
const MOVEMENT_START_GRACE_FRAMES: u8 = 8;
/// Full render frames allowed after both cameras move to a fresh image target.
///
/// Four frames let the asynchronous UI glyph atlas settle on Metal. Two frames
/// occasionally captured a complete 3D pass with only part of the UI text uploaded.
const CAPTURE_TARGET_SETTLE_FRAMES: u8 = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReviewScenarioProvenance {
    name: String,
    seed: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReviewRunProvenance {
    run_id: String,
    script_path: String,
    expected_captures: usize,
    planned_scenarios: Vec<ReviewScenarioProvenance>,
}

impl ReviewRunProvenance {
    fn from_steps(script_path: String, run_id: String, steps: &[WalkStep]) -> Self {
        let expected_captures = steps.iter().map(step_capture_count).sum();
        let planned_scenarios = steps
            .iter()
            .filter_map(|step| match step {
                WalkStep::StartScenario { name, seed, .. } => Some(ReviewScenarioProvenance {
                    name: name.clone(),
                    seed: *seed,
                }),
                _ => None,
            })
            .collect();
        Self {
            run_id,
            script_path,
            expected_captures,
            planned_scenarios,
        }
    }
}

/// Gives every configured walk a fresh storage root unless the caller explicitly
/// supplied one. This runs before persistence plugins initialize `StoragePaths`.
pub(super) fn isolate_storage(app: &mut App) {
    if env::var_os(DATA_ENV).is_some() {
        return;
    }
    let script = env::var_os(SCRIPT_ENV);
    let out = env::var_os(OUT_ENV);
    if script.is_none() && out.is_none() {
        return;
    }

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let parent = out.map_or_else(env::temp_dir, PathBuf::from);
    let root = isolated_storage_root(parent, std::process::id(), nonce);
    info!(
        "visual walk: isolating disposable application data at {}",
        root.display()
    );
    app.insert_resource(crate::storage::StoragePaths::under(root));
}

fn isolated_storage_root(out: PathBuf, process_id: u32, nonce: u128) -> PathBuf {
    out.join(format!(".game-data-{process_id}-{nonce}"))
}

fn review_run_id() -> String {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    format!("{}-{nonce}", std::process::id())
}

fn walk_time_update_strategy() -> bevy::time::TimeUpdateStrategy {
    bevy::time::TimeUpdateStrategy::ManualDuration(WALK_FRAME_DURATION)
}

fn walk_environment_value(name: &str) -> Result<Option<String>, String> {
    normalize_walk_environment_value(name, env::var(name))
}

fn normalize_walk_environment_value(
    name: &str,
    value: Result<String, env::VarError>,
) -> Result<Option<String>, String> {
    match value {
        Ok(value) => Ok(Some(value)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(_)) => Err(format!("{name} is not valid Unicode")),
    }
}

/// Installs the walk runner only when its environment is present.
pub(super) fn plugin(app: &mut App) {
    let script = match walk_environment_value(SCRIPT_ENV) {
        Ok(value) => value,
        Err(error) => {
            install_config_error(app, error);
            return;
        }
    };
    let out = match walk_environment_value(OUT_ENV) {
        Ok(value) => value,
        Err(error) => {
            install_config_error(app, error);
            return;
        }
    };
    let run_id = review_run_id();
    if let Some(out) = out.as_deref() {
        let script_label = script.as_deref().unwrap_or("<missing HEX_WALK_SCRIPT>");
        if let Err(error) = write_starting_review_index(&PathBuf::from(out), script_label, &run_id)
        {
            install_config_error(app, error);
            return;
        }
    }
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
    let out_dir = PathBuf::from(&out);

    let steps = match load_script(&script) {
        Ok(steps) => steps,
        Err(error) => {
            install_config_error(app, error);
            return;
        }
    };
    let viewport = match env::var(VIEWPORT_ENV) {
        Ok(viewport) => match parse_viewport(&viewport) {
            Ok(viewport) => viewport,
            Err(error) => {
                install_config_error(app, error);
                return;
            }
        },
        Err(env::VarError::NotPresent) => hex_ui::ReviewViewport::DEFAULT,
        Err(error) => {
            install_config_error(app, format!("cannot read {VIEWPORT_ENV}: {error}"));
            return;
        }
    };
    let review = ReviewRunProvenance::from_steps(script.clone(), run_id, &steps);
    if let Err(error) = write_incomplete_review_index(&out_dir, &review) {
        install_config_error(app, error);
        return;
    }

    info!(
        "visual walk: {} steps from {script}, output to {out} at {}x{}@{}",
        steps.len(),
        viewport.logical_size.x,
        viewport.logical_size.y,
        viewport.device_scale,
    );
    let diagnostic_overlays = env::var_os(UI_DEBUG_ENV).is_some();
    if diagnostic_overlays {
        let mut options = app
            .world_mut()
            .resource_mut::<bevy::ui_render::prelude::GlobalUiDebugOptions>();
        options.enabled = true;
        options.show_hidden = true;
        options.show_clipped = true;
        options.outline_padding_box = true;
        options.outline_content_box = true;
        options.outline_scrollbars = true;
    }
    app.insert_resource(AutomatedWalk)
        .insert_resource(walk_time_update_strategy())
        .insert_resource(WalkState::new(
            steps,
            out_dir,
            viewport,
            diagnostic_overlays,
            review,
        ))
        .add_systems(Startup, accelerate_walk_time)
        .add_systems(
            OnEnter(Screen::Gameplay),
            suppress_hostiles_for_map_review.in_set(GameplaySetup::Resources),
        )
        .add_systems(
            PreUpdate,
            run_walk
                .after(InputSystems)
                .in_set(WalkSystems::InjectInput),
        );
}

fn accelerate_walk_time(mut time: ResMut<Time<Virtual>>) {
    // Walks exercise the ordinary animation/timer systems, but should not spend
    // wall-clock minutes waiting through every combatant in a 6v6 matrix.
    time.set_relative_speed(WALK_TIME_SCALE);
}

fn install_config_error(app: &mut App, error: String) {
    app.insert_resource(WalkConfigurationError(error))
        .add_systems(Startup, reject_invalid_configuration);
}

#[derive(Resource, Debug)]
struct WalkConfigurationError(String);

/// Feature-only launch policy for terrain and camera evidence.
#[derive(Resource)]
struct SuppressHostilesForMapReview;

fn suppress_hostiles_for_map_review(
    policy: Option<Res<SuppressHostilesForMapReview>>,
    mut encounter: ResMut<Encounter>,
    mut commands: Commands,
) {
    if policy.is_some() {
        retain_non_hostile_rosters(&mut encounter);
        commands.remove_resource::<SuppressHostilesForMapReview>();
    }
}

fn retain_non_hostile_rosters(encounter: &mut Encounter) {
    encounter
        .rosters
        .retain(|roster| roster.faction != EncounterFaction::Hostile);
}

fn reject_invalid_configuration(
    error: Res<WalkConfigurationError>,
    mut exit: MessageWriter<AppExit>,
) {
    error!("invalid visual-walk configuration: {}", error.0);
    exit.write(AppExit::error());
}

/// One scripted action. The RON script is a `Vec<WalkStep>`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
enum WalkStep {
    /// Wait until the app is in the named screen.
    AwaitScreen(String),
    /// Explicit bounded startup allowance for large worlds in unoptimized builds.
    AwaitGameplay { max_seconds: u32 },
    /// Wait until validated terrain is ready (gameplay only).
    AwaitTerrain,
    /// Let this many frames pass before the next step.
    Settle(u32),
    /// Photograph the current Bevy image target into `<out>/<name>.png`.
    Capture(String),
    /// Capture one gameplay-owned review task through its named structural contract.
    ///
    /// Map-owner walks retain the generic `Capture` variant unchanged. Gameplay
    /// UI acceptance uses this fail-closed variant so a correct screen root with
    /// the wrong task contents cannot produce evidence.
    ReviewCapture {
        name: String,
        task: hex_ui::test_support::UiTaskCase,
    },
    /// Press the `index`-th button whose `Name` starts with `name`.
    Click {
        name: String,
        #[serde(default)]
        index: usize,
    },
    /// Click one exact exposed terrain entity through the ordinary picking observer path.
    ///
    /// Omitting `level` is accepted only when the coordinate has one exposed
    /// surface. Stacked terrain must name its exact surface instead of letting
    /// the runner guess which entity a real pointer would have hit.
    ClickTile {
        q: i32,
        r: i32,
        #[serde(default)]
        level: Option<hex_core::Level>,
    },
    /// Hover one exact exposed terrain entity through the ordinary picking observer path.
    ///
    /// This is separate from [`Self::ClickTile`] so presentation walks can inspect
    /// pre-commit movement and targeting feedback without authorizing a command.
    HoverTile {
        q: i32,
        r: i32,
        #[serde(default)]
        level: Option<hex_core::Level>,
    },
    /// Click one generated anchor through the same stack-safe picking path.
    ///
    /// `expected` deliberately duplicates the current hero-seed projection. The
    /// anchor remains the authority, while a moved anchor makes old visual evidence
    /// fail stale instead of silently reviewing a different place.
    ClickAnchor {
        name: String,
        expected: CameraRouteTile,
    },
    /// Record an exact, bounded PNG sequence while the movement started by the
    /// immediately preceding [`Self::ClickAnchor`] remains authoritative.
    ///
    /// Frames are requested every `every_frames` deterministic 60 Hz movement
    /// updates at 1x simulation speed and written as `<prefix>-0001.png`,
    /// `<prefix>-0002.png`, and so on. The exact count keeps
    /// the completed review index exhaustive; ending movement early fails and
    /// removes this step's partial sequence instead of publishing stale evidence.
    CaptureWhileMoving {
        prefix: String,
        every_frames: u32,
        capture_count: u16,
    },
    /// Wait for every registered party member's domain movement to finish.
    ///
    /// The script owns the frame limit so a stalled route fails deterministically
    /// instead of relying only on the runner's wall-clock watchdog.
    AwaitPartyIdle { max_frames: u32 },
    /// Prove that the authoritative selection and its camera projection reached
    /// one exact stack-safe surface before accepting visual evidence.
    ///
    /// This is deliberately separate from [`Self::AwaitPartyIdle`]: an ignored
    /// click also leaves the party idle, so idleness alone cannot prove a route
    /// was accepted or completed.
    AssertSelectedAt { expected: CameraRouteTile },
    /// Prove that ordinary input selected the expected camera perspective.
    AssertCameraMode(WalkCameraMode),
    /// Wait until a named button exists without activating it.
    AwaitButton(String),
    /// Install an authored immutable UI presentation state without solving combat.
    PresentUi(String),
    /// Select one semantic UI scale for responsive presentation review.
    SetUiScale(hex_ui::UiScaleMode),
    /// Change the logical canvas and device scale for later presentation frames.
    SetViewport {
        width: u32,
        height: u32,
        device_scale: f32,
    },
    /// Perform one bounded right-mouse drag through ordinary camera input.
    ///
    /// Positive yaw is a counter-clockwise fraction of one turn. Positive pitch is
    /// a downward fraction of a quarter turn. Each relative gesture is bounded so
    /// both synthetic cursor positions describe one plausible drag rather than a
    /// direct camera-state mutation.
    OrbitCamera {
        yaw_turns: f32,
        #[serde(default)]
        pitch_fraction: f32,
    },
    /// Press and release a supported gameplay or menu key.
    Key(String),
    /// Hold a bounded chord through ordinary input before releasing every key.
    HoldKeys { keys: Vec<String>, frames: u32 },
    /// Validate exploration state separately from screenshot interpretation.
    #[cfg(feature = "dev")]
    AssertExplorer {
        mode: String,
        grounded: bool,
        /// Distance from the preceding successful explorer observation.
        #[serde(default)]
        minimum_displacement: Option<f32>,
    },
    /// Install an exact internal scenario as a review-only launch input.
    StartScenario {
        name: String,
        #[serde(default)]
        seed: Option<u64>,
        /// Remove hostile rosters before actor setup for this feature-only launch.
        ///
        /// Map-owner presentation walks use this when combat is unrelated to the
        /// terrain and camera evidence under review. Shipped scenario tests still
        /// exercise the authored encounter independently.
        #[serde(default)]
        suppress_hostiles: bool,
    },
}

/// Stable review vocabulary kept separate from the runtime resource's Rust names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
enum WalkCameraMode {
    Map,
    Character,
    FirstPerson,
}

impl WalkCameraMode {
    fn matches(self, actual: CameraMode) -> bool {
        matches!(
            (self, actual),
            (Self::Map, CameraMode::Map)
                | (Self::Character, CameraMode::Character)
                | (Self::FirstPerson, CameraMode::FirstPerson)
        )
    }
}

/// Stack-safe position serialized by camera-route evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(deny_unknown_fields)]
struct CameraRouteTile {
    q: i32,
    r: i32,
    level: hex_core::Level,
}

impl CameraRouteTile {
    fn position(self) -> TilePos {
        TilePos::new(HexCoord::from_axial(self.q, self.r), self.level)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OrbitGesturePhase {
    Delta,
    Release,
}

#[derive(Debug, Clone, Copy)]
struct OrbitGesture {
    window: Entity,
    baseline: Vec2,
    destination: Vec2,
    phase: OrbitGesturePhase,
}

fn load_script(path: &str) -> Result<Vec<WalkStep>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read {SCRIPT_ENV} {path}: {error}"))?;
    let steps: Vec<WalkStep> =
        ron::from_str(&text).map_err(|error| format!("cannot parse {path}: {error}"))?;
    if steps.is_empty() {
        return Err(format!("{path} contains no steps"));
    }
    validate_script_steps(path, &steps)?;
    Ok(steps)
}

fn step_capture_count(step: &WalkStep) -> usize {
    match step {
        WalkStep::Capture(_) | WalkStep::ReviewCapture { .. } => 1,
        WalkStep::CaptureWhileMoving { capture_count, .. } => usize::from(*capture_count),
        _ => 0,
    }
}

fn movement_capture_name(prefix: &str, index: u16) -> String {
    format!("{prefix}-{:04}", u32::from(index) + 1)
}

fn movement_capture_span(every_frames: u32, capture_count: u16) -> Result<u32, String> {
    if every_frames == 0 {
        return Err("CaptureWhileMoving every_frames must be positive".to_owned());
    }
    if capture_count == 0 {
        return Err("CaptureWhileMoving capture_count must be positive".to_owned());
    }
    if capture_count > MAX_MOVEMENT_CAPTURE_FILES {
        return Err(format!(
            "CaptureWhileMoving capture_count {capture_count} exceeds the per-step limit {MAX_MOVEMENT_CAPTURE_FILES}"
        ));
    }
    let span = every_frames
        .checked_mul(u32::from(capture_count))
        .ok_or_else(|| "CaptureWhileMoving frame span overflowed".to_owned())?;
    if span > MAX_MOVEMENT_CAPTURE_FRAMES {
        return Err(format!(
            "CaptureWhileMoving spans {span} frames, exceeding the per-step limit {MAX_MOVEMENT_CAPTURE_FRAMES}"
        ));
    }
    Ok(span)
}

/// Wall-clock watchdog for one step under the windowless 60 Hz runner.
///
/// `AwaitPartyIdle` already owns an exact update bound. Its wall watchdog must
/// leave enough time for that bound to run instead of silently replacing an
/// authored 18,000-frame allowance with the generic sixty-second limit.
fn step_timeout(step: &WalkStep) -> Duration {
    if let WalkStep::AwaitGameplay { max_seconds } = step {
        return Duration::from_secs(u64::from(*max_seconds));
    }
    let WalkStep::AwaitPartyIdle { max_frames } = step else {
        return STEP_TIMEOUT;
    };
    let frame_budget = WALK_FRAME_DURATION
        .checked_mul(*max_frames)
        .unwrap_or(Duration::MAX);
    STEP_TIMEOUT.saturating_add(frame_budget)
}

fn validate_script_steps(path: &str, steps: &[WalkStep]) -> Result<(), String> {
    let mut capture_names = BTreeSet::<String>::new();
    let mut movement_capture_files = 0_usize;
    for (index, step) in steps.iter().enumerate() {
        validate_step(step).map_err(|error| format!("{path} step {index}: {error}"))?;
        let capture_names_for_step = match step {
            WalkStep::Capture(name) | WalkStep::ReviewCapture { name, .. } => vec![name.clone()],
            WalkStep::CaptureWhileMoving {
                prefix,
                capture_count,
                ..
            } => {
                let clicked_destination = match index
                    .checked_sub(1)
                    .and_then(|prior| steps.get(prior))
                {
                    Some(WalkStep::ClickAnchor { expected, .. }) => *expected,
                    _ => {
                        return Err(format!(
                            "{path} step {index}: CaptureWhileMoving must immediately follow ClickAnchor"
                        ));
                    }
                };
                if !matches!(steps.get(index + 1), Some(WalkStep::AwaitPartyIdle { .. })) {
                    return Err(format!(
                        "{path} step {index}: CaptureWhileMoving must be followed by AwaitPartyIdle"
                    ));
                }
                match steps.get(index + 2) {
                    Some(WalkStep::AssertSelectedAt { expected })
                        if *expected == clicked_destination => {}
                    Some(WalkStep::AssertSelectedAt { expected }) => {
                        return Err(format!(
                            "{path} step {index}: CaptureWhileMoving arrival proof {expected:?} does not match ClickAnchor destination {clicked_destination:?}"
                        ));
                    }
                    _ => {
                        return Err(format!(
                            "{path} step {index}: CaptureWhileMoving must be followed by AwaitPartyIdle and matching AssertSelectedAt"
                        ));
                    }
                }
                movement_capture_files = movement_capture_files
                    .checked_add(usize::from(*capture_count))
                    .ok_or_else(|| {
                        format!("{path} step {index}: movement capture count overflowed")
                    })?;
                if movement_capture_files > MAX_MOVEMENT_CAPTURE_FILES_PER_WALK {
                    return Err(format!(
                        "{path} step {index}: movement sequences request {movement_capture_files} files, exceeding the per-walk limit {MAX_MOVEMENT_CAPTURE_FILES_PER_WALK}"
                    ));
                }
                (0..*capture_count)
                    .map(|frame| movement_capture_name(prefix, frame))
                    .collect()
            }
            _ => Vec::new(),
        };
        for name in capture_names_for_step {
            if !capture_names.insert(name.clone()) {
                return Err(format!(
                    "{path} step {index}: duplicate capture name {name:?} would overwrite evidence"
                ));
            }
        }
    }
    Ok(())
}

fn validate_step(step: &WalkStep) -> Result<(), String> {
    match step {
        WalkStep::AwaitScreen(name) => parse_screen(name).map(|_| ()),
        WalkStep::AwaitGameplay { max_seconds } => {
            if (1..=300).contains(max_seconds) {
                Ok(())
            } else {
                Err("AwaitGameplay requires a timeout in 1..300 seconds".into())
            }
        }
        WalkStep::Key(name) => parse_key(name).map(|_| ()),
        WalkStep::HoldKeys { keys, frames } => {
            if keys.is_empty() || keys.len() > 4 || *frames == 0 || *frames > 600 {
                return Err("HoldKeys requires 1..4 keys and 1..600 frames".into());
            }
            for key in keys {
                parse_key(key)?;
            }
            Ok(())
        }
        #[cfg(feature = "dev")]
        WalkStep::AssertExplorer {
            mode,
            minimum_displacement,
            ..
        } => {
            if matches!(mode.as_str(), "walk" | "fly")
                && minimum_displacement
                    .is_none_or(|distance| distance.is_finite() && distance > 0.0)
            {
                Ok(())
            } else {
                Err("Explorer mode must be walk or fly; minimum displacement must be finite and positive".into())
            }
        }
        WalkStep::Capture(name) | WalkStep::ReviewCapture { name, .. } => {
            validate_capture_name(name)
        }
        WalkStep::CaptureWhileMoving {
            prefix,
            every_frames,
            capture_count,
        } => {
            validate_capture_name(prefix)?;
            movement_capture_span(*every_frames, *capture_count).map(|_| ())
        }
        WalkStep::Click { name, .. } if name.trim().is_empty() => {
            Err("click name must not be empty".to_owned())
        }
        WalkStep::ClickAnchor { name, .. } if name.trim().is_empty() => {
            Err("anchor name must not be empty".to_owned())
        }
        WalkStep::AwaitPartyIdle { max_frames: 0 } => {
            Err("AwaitPartyIdle max_frames must be positive".to_owned())
        }
        WalkStep::AwaitButton(name) if name.trim().is_empty() => {
            Err("awaited button name must not be empty".to_owned())
        }
        WalkStep::PresentUi(name)
            if !matches!(
                name.as_str(),
                "clear"
                    | "normal-gameplay"
                    | "player-turn-max"
                    | "hostile-turn"
                    | "casting-list"
                    | "required-decision"
                    | "restore-decision"
                    | "aiming-disabled"
                    | "multiplayer-lobby"
                    | "multiplayer-lan-browser"
                    | "multiplayer-lan-host"
                    | "multiplayer-campaign"
                    | "multiplayer-campaign-refusal"
                    | "multiplayer-campaign-lobby"
                    | "multiplayer-campaign-save"
                    | "multiplayer-mismatch"
                    | "multiplayer-reconnect"
                    | "multiplayer-host"
                    | "multiplayer-client-menu"
                    | "sandbox-outcome"
            ) =>
        {
            Err(format!("unknown presentation-only UI fixture {name:?}"))
        }
        WalkStep::SetViewport {
            width,
            height,
            device_scale,
        } => hex_ui::ReviewViewport::new(*width, *height, *device_scale).map(|_| ()),
        WalkStep::OrbitCamera {
            yaw_turns,
            pitch_fraction,
        } => validate_orbit_drag(*yaw_turns, *pitch_fraction),
        WalkStep::StartScenario { name, .. } if name.trim().is_empty() => {
            Err("scenario name must not be empty".to_owned())
        }
        _ => Ok(()),
    }
}

fn validate_capture_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("capture name must not be empty".to_owned());
    }
    if name.chars().all(|character| {
        character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || matches!(character, '-' | '_')
    }) {
        Ok(())
    } else {
        Err(format!(
            "capture name {name:?} must use only lowercase ASCII letters, digits, '-' or '_'"
        ))
    }
}

fn markdown_html_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn scenario_provenance_markdown(label: &str, scenarios: &[ReviewScenarioProvenance]) -> String {
    if scenarios.is_empty() {
        return format!("- {label}: none\n");
    }
    scenarios
        .iter()
        .map(|scenario| {
            let seed = scenario.seed.map_or_else(
                || "none/catalog default".to_owned(),
                |seed| seed.to_string(),
            );
            format!(
                "- {label}: <code>{}</code> — seed <code>{seed}</code>\n",
                markdown_html_text(&scenario.name)
            )
        })
        .collect()
}

fn incomplete_review_index_markdown(review: &ReviewRunProvenance) -> String {
    let mut markdown = format!(
        "# Visual walk review index — INCOMPLETE\n\n\
         **Run status: INCOMPLETE — NOT REVIEWABLE.** A visual walk has started but has not\n\
         atomically published a complete capture pack. PNGs in this directory may mix stale and\n\
         current-run output; do not classify or curate them.\n\n\
         ## Run provenance\n\n\
         - Run ID: <code>{}</code>\n\
         - Script: <code>{}</code>\n\
         - Captures persisted by a completed run: **0 of {} expected**\n",
        markdown_html_text(&review.run_id),
        markdown_html_text(&review.script_path),
        review.expected_captures,
    );
    markdown.push_str(&scenario_provenance_markdown(
        "Planned scenario",
        &review.planned_scenarios,
    ));
    markdown
}

fn starting_review_index_markdown(script_path: &str, run_id: &str) -> String {
    format!(
        "# Visual walk review index — INCOMPLETE\n\n\
         **Run status: INCOMPLETE — NOT REVIEWABLE.** A visual walk invocation has started, but\n\
         its script and capture plan have not finished validation. PNGs in this directory may\n\
         mix stale and current-run output; do not classify or curate them.\n\n\
         ## Run provenance\n\n\
         - Run ID: <code>{}</code>\n\
         - Script: <code>{}</code>\n\
         - Expected captures: unknown until script validation completes\n",
        markdown_html_text(run_id),
        markdown_html_text(script_path),
    )
}

fn completed_review_index_markdown(
    review: &ReviewRunProvenance,
    launched_scenarios: &[ReviewScenarioProvenance],
    captures: &[String],
) -> Result<String, String> {
    if captures.len() != review.expected_captures {
        return Err(format!(
            "visual walk completed with {} persisted captures, expected {}; the review index remains incomplete",
            captures.len(),
            review.expected_captures
        ));
    }
    let mut markdown = format!(
        "# Visual walk review index\n\n\
         **Capture status: COMPLETE. Human review status: UNREVIEWED.** This index includes every\n\
         persisted frame from the completed run in script order. Set each frame's result to PASS\n\
         or FAIL and record notes before curating a smaller approval report. Dense frame sequences\n\
         can clear discrete renderer defects; they do not replace a required native control-feel or\n\
         camera-comfort check.\n\n\
         ## Run provenance\n\n\
         - Run ID: <code>{}</code>\n\
         - Script: <code>{}</code>\n\
         - Captures persisted: **{} of {} expected**\n",
        markdown_html_text(&review.run_id),
        markdown_html_text(&review.script_path),
        captures.len(),
        review.expected_captures,
    );
    markdown.push_str(&scenario_provenance_markdown(
        "Planned scenario",
        &review.planned_scenarios,
    ));
    markdown.push_str(&scenario_provenance_markdown(
        "Launched scenario",
        launched_scenarios,
    ));
    markdown.push_str(
        "\n## Per-frame classification\n\n\
         Allowed results are **PASS** and **FAIL**. `UNREVIEWED` is never approval evidence.\n\n",
    );
    for name in captures {
        markdown.push_str("### `");
        markdown.push_str(name);
        markdown.push_str("`\n\n- Result: **UNREVIEWED** — replace with **PASS** or **FAIL**\n- Notes: _record the defect for FAIL; optional for PASS_\n\n![");
        markdown.push_str(name);
        markdown.push_str("](<./");
        markdown.push_str(name);
        markdown.push_str(".png>)\n\n");
    }
    Ok(markdown)
}

fn write_review_index_atomically(
    out_dir: &std::path::Path,
    markdown: &str,
) -> Result<PathBuf, String> {
    let path = out_dir.join(REVIEW_INDEX_FILE);
    prepare_capture_path(&path)
        .map_err(|error| format!("cannot prepare staged review index: {error}"))?;
    let temporary = temporary_capture_path(&path)
        .map_err(|error| format!("cannot prepare staged review index: {error}"))?;
    if let Err(error) = std::fs::write(&temporary, markdown) {
        let _cleanup = std::fs::remove_file(&temporary);
        return Err(format!("cannot write temporary review index: {error}"));
    }
    if let Err(error) = install_capture(&temporary, &path) {
        let _cleanup = std::fs::remove_file(&temporary);
        return Err(format!("cannot atomically install review index: {error}"));
    }
    Ok(path)
}

fn write_incomplete_review_index(
    out_dir: &std::path::Path,
    review: &ReviewRunProvenance,
) -> Result<PathBuf, String> {
    write_review_index_atomically(out_dir, &incomplete_review_index_markdown(review))
}

fn write_starting_review_index(
    out_dir: &std::path::Path,
    script_path: &str,
    run_id: &str,
) -> Result<PathBuf, String> {
    write_review_index_atomically(
        out_dir,
        &starting_review_index_markdown(script_path, run_id),
    )
}

fn write_completed_review_index(
    out_dir: &std::path::Path,
    review: &ReviewRunProvenance,
    launched_scenarios: &[ReviewScenarioProvenance],
    captures: &[String],
) -> Result<PathBuf, String> {
    let markdown = completed_review_index_markdown(review, launched_scenarios, captures)?;
    write_review_index_atomically(out_dir, &markdown)
}

fn validate_orbit_drag(yaw_turns: f32, pitch_fraction: f32) -> Result<(), String> {
    if !yaw_turns.is_finite() || !pitch_fraction.is_finite() {
        return Err("camera orbit values must be finite".to_owned());
    }
    if yaw_turns.abs() > MAX_ORBIT_YAW_TURNS {
        return Err(format!(
            "camera yaw must be within ±{MAX_ORBIT_YAW_TURNS} turns per gesture"
        ));
    }
    if pitch_fraction.abs() > MAX_ORBIT_PITCH_FRACTION {
        return Err(format!(
            "camera pitch must be within ±{MAX_ORBIT_PITCH_FRACTION} quarter turns per gesture"
        ));
    }
    if yaw_turns == 0.0 && pitch_fraction == 0.0 {
        return Err("camera orbit gesture must move yaw or pitch".to_owned());
    }
    Ok(())
}

fn orbit_cursor_positions(
    window_size: Vec2,
    cursor_position: Option<Vec2>,
    yaw_turns: f32,
    pitch_fraction: f32,
) -> Result<(Vec2, Vec2), String> {
    validate_orbit_drag(yaw_turns, pitch_fraction)?;
    if !window_size.is_finite() || window_size.min_element() <= 0.0 {
        return Err("camera orbit requires a finite positive window".to_owned());
    }
    let baseline = cursor_position.unwrap_or(window_size * 0.5);
    let delta = Vec2::new(
        -yaw_turns * window_size.x,
        pitch_fraction * window_size.y * 0.5,
    );
    Ok((baseline, baseline + delta))
}

#[cfg(test)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CameraRouteManifest {
    schema_version: u16,
    routes: Vec<CameraRouteCase>,
}

#[cfg(test)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CameraRouteCase {
    scenario: String,
    seed: Option<u64>,
    points: Vec<CameraRoutePoint>,
}

#[cfg(test)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CameraRoutePoint {
    label: String,
    destination: CameraRouteDestination,
    azimuth_turns: Vec<f32>,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
enum CameraRouteDestination {
    Anchor {
        name: String,
        expected: CameraRouteTile,
    },
    Exact(CameraRouteTile),
}

fn parse_screen(name: &str) -> Result<Screen, String> {
    match name {
        "Splash" => Ok(Screen::Splash),
        "Title" => Ok(Screen::Title),
        "Multiplayer" => Ok(Screen::Multiplayer),
        "Sandbox" => Ok(Screen::Sandbox),
        "Settings" => Ok(Screen::Settings),
        "LatticeDemo" => Ok(Screen::LatticeDemo),
        "CharacterCreator" => Ok(Screen::CharacterCreator),
        "SpellCreator" => Ok(Screen::SpellCreator),
        "VfxTuner" => Ok(Screen::VfxTuner),
        "Loading" => Ok(Screen::Loading),
        "Gameplay" => Ok(Screen::Gameplay),
        _ => Err(format!(
            "unknown screen {name:?}; expected Splash, Title, Multiplayer, Sandbox, Settings, CharacterCreator, SpellCreator, LatticeDemo, VfxTuner, Loading, or Gameplay"
        )),
    }
}

fn parse_key(name: &str) -> Result<KeyCode, String> {
    match name {
        "W" => Ok(KeyCode::KeyW),
        "A" => Ok(KeyCode::KeyA),
        "S" => Ok(KeyCode::KeyS),
        "D" => Ok(KeyCode::KeyD),
        "Space" => Ok(KeyCode::Space),
        "Shift" => Ok(KeyCode::ShiftLeft),
        "Backspace" => Ok(KeyCode::Backspace),
        "Escape" => Ok(KeyCode::Escape),
        "B" => Ok(KeyCode::KeyB),
        "C" => Ok(KeyCode::KeyC),
        "F" => Ok(KeyCode::KeyF),
        "H" => Ok(KeyCode::KeyH),
        "I" => Ok(KeyCode::KeyI),
        "L" => Ok(KeyCode::KeyL),
        "P" => Ok(KeyCode::KeyP),
        "V" => Ok(KeyCode::KeyV),
        _ => Err(format!(
            "unknown key {name:?}; expected Backspace, Escape, or a configured HUD/camera review key"
        )),
    }
}

fn parse_viewport(viewport: &str) -> Result<hex_ui::ReviewViewport, String> {
    let invalid = || format!("{VIEWPORT_ENV} must be WIDTHxHEIGHT@SCALE");
    let Some((size, device_scale)) = viewport.split_once('@') else {
        return Err(invalid());
    };
    let Some((width, height)) = size.split_once('x') else {
        return Err(invalid());
    };
    if height.contains('x') || device_scale.contains('@') {
        return Err(invalid());
    }
    let width = width
        .parse::<u32>()
        .map_err(|error| format!("{VIEWPORT_ENV} width is invalid: {error}"))?;
    let height = height
        .parse::<u32>()
        .map_err(|error| format!("{VIEWPORT_ENV} height is invalid: {error}"))?;
    let device_scale = device_scale
        .parse::<f32>()
        .map_err(|error| format!("{VIEWPORT_ENV} device scale is invalid: {error}"))?;
    hex_ui::ReviewViewport::new(width, height, device_scale)
}

/// Resolves a script coordinate to the same exact entity the picking backend would
/// have reported. `None` means terrain has not published any tile yet, so the step
/// may continue waiting.
fn resolve_tile_click_target<'a>(
    tiles: impl Iterator<Item = (Entity, &'a TilePos, &'a Headroom)>,
    coord: HexCoord,
    level: Option<hex_core::Level>,
) -> Result<Option<(Entity, TilePos)>, String> {
    let mut saw_tile = false;
    let mut at_coord = Vec::new();
    for (entity, pos, headroom) in tiles {
        saw_tile = true;
        if pos.coord == coord && headroom.0 > 0 {
            at_coord.push((entity, *pos));
        }
    }
    if !saw_tile {
        return Ok(None);
    }

    at_coord.sort_by_key(|&(entity, pos)| (pos, entity));
    let available_levels: Vec<hex_core::Level> =
        at_coord.iter().map(|(_, pos)| pos.level).collect();
    let matches: Vec<(Entity, TilePos)> = at_coord
        .into_iter()
        .filter(|(_, pos)| level.is_none_or(|level| pos.level == level))
        .collect();

    match matches.as_slice() {
        [(entity, pos)] => Ok(Some((*entity, *pos))),
        [] if available_levels.is_empty() => Err(format!(
            "ClickTile(q: {}, r: {}) names no published terrain coordinate",
            coord.x(),
            coord.y()
        )),
        [] => Err(format!(
            "ClickTile(q: {}, r: {}, level: {level:?}) names no published run; available levels are {available_levels:?}",
            coord.x(),
            coord.y()
        )),
        _ if level.is_none() => Err(format!(
            "ClickTile(q: {}, r: {}) is ambiguous across stacked levels {available_levels:?}; specify an exact level",
            coord.x(),
            coord.y()
        )),
        _ => Err(format!(
            "ClickTile(q: {}, r: {}, level: {level:?}) matched duplicate published runs",
            coord.x(),
            coord.y()
        )),
    }
}

fn primary_tile_click(target: Entity, window: Entity) -> Option<Pointer<Click>> {
    let target_window = bevy::window::WindowRef::Entity(window).normalize(Some(window))?;
    let location = Location {
        target: NormalizedRenderTarget::Window(target_window),
        position: Vec2::ZERO,
    };
    let click = Click {
        button: PointerButton::Primary,
        hit: HitData::new(target, 0.0, None, None),
        duration: Duration::from_millis(1),
        count: 1,
    };
    Some(Pointer::new(PointerId::Mouse, location, click, target))
}

fn primary_tile_hover(target: Entity, window: Entity) -> Option<Pointer<Over>> {
    let target_window = bevy::window::WindowRef::Entity(window).normalize(Some(window))?;
    let location = Location {
        target: NormalizedRenderTarget::Window(target_window),
        position: Vec2::ZERO,
    };
    Some(Pointer::new(
        PointerId::Mouse,
        location,
        Over {
            hit: HitData::new(target, 0.0, None, None),
        },
        target,
    ))
}

/// What the capture observer reports back to the runner.
#[derive(Debug)]
enum CaptureOutcome {
    Written { brightest: u8, coverage: bool },
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MovementCaptureTick {
    WaitingForMovement,
    WaitingForInterval,
    Capture(u16),
}

#[derive(Debug)]
struct MovementCaptureState {
    prefix: String,
    every_frames: u32,
    capture_count: u16,
    start_grace_frames: u8,
    observed_pending_movement: bool,
    pending_frames: u32,
    requested: u16,
    outcomes: BTreeMap<u16, CaptureOutcome>,
    failure: Option<String>,
    /// Prevent late renderer callbacks from writing after fail-closed cleanup.
    aborted: bool,
}

/// A fully written temporal sequence that is not evidence until the contiguous
/// `AwaitPartyIdle` and exact `AssertSelectedAt` proof also succeed.
///
/// Keeping this small tombstone prevents a route that stopped early, timed out,
/// or arrived on the wrong surface from leaving apparently complete PNGs behind.
#[derive(Debug)]
struct PendingMovementCaptureProof {
    prefix: String,
    capture_count: u16,
}

impl MovementCaptureState {
    fn new(prefix: String, every_frames: u32, capture_count: u16) -> Self {
        Self {
            prefix,
            every_frames,
            capture_count,
            start_grace_frames: 0,
            observed_pending_movement: false,
            pending_frames: 0,
            requested: 0,
            outcomes: BTreeMap::new(),
            failure: None,
            aborted: false,
        }
    }

    fn observe_movement_frame(
        &mut self,
        selected_movement_is_pending: bool,
    ) -> Result<MovementCaptureTick, String> {
        if !selected_movement_is_pending {
            if self.observed_pending_movement {
                return Err(format!(
                    "movement ended after {} of {} temporal frames were requested",
                    self.requested, self.capture_count
                ));
            }
            self.start_grace_frames = self.start_grace_frames.saturating_add(1);
            if self.start_grace_frames > MOVEMENT_START_GRACE_FRAMES {
                return Err(format!(
                    "the preceding ClickAnchor did not start movement within {MOVEMENT_START_GRACE_FRAMES} frames"
                ));
            }
            return Ok(MovementCaptureTick::WaitingForMovement);
        }

        self.observed_pending_movement = true;
        self.pending_frames = self.pending_frames.saturating_add(1);
        if !self.pending_frames.is_multiple_of(self.every_frames) {
            return Ok(MovementCaptureTick::WaitingForInterval);
        }
        if self.requested >= self.capture_count {
            return Err(format!(
                "movement capture tried to exceed its exact {}-file plan",
                self.capture_count
            ));
        }
        let index = self.requested;
        self.requested += 1;
        Ok(MovementCaptureTick::Capture(index))
    }

    fn scheduled_all(&self) -> bool {
        self.requested == self.capture_count
    }

    fn all_requested_finished(&self) -> bool {
        self.outcomes.len() == usize::from(self.requested)
    }

    fn first_outcome_failure(&self) -> Option<String> {
        self.outcomes.iter().find_map(|(index, outcome)| {
            let name = movement_capture_name(&self.prefix, *index);
            match outcome {
                CaptureOutcome::Written { brightest, .. } if *brightest > 8 => None,
                CaptureOutcome::Written { .. } => {
                    Some(format!("temporal frame {name:?} came back black"))
                }
                CaptureOutcome::Failed(error) => {
                    Some(format!("temporal frame {name:?} failed: {error}"))
                }
            }
        })
    }

    fn callback_issue(&self, prefix: &str, frame_index: u16) -> Option<String> {
        if self.aborted {
            return Some(format!(
                "temporal frame {frame_index} for {prefix:?} arrived after abort"
            ));
        }
        if self.prefix != prefix {
            return Some(format!(
                "temporal frame for {prefix:?} arrived during {:?}",
                self.prefix
            ));
        }
        if frame_index >= self.requested {
            return Some(format!(
                "temporal frame {frame_index} arrived before its request was recorded"
            ));
        }
        self.outcomes
            .contains_key(&frame_index)
            .then(|| format!("temporal frame {frame_index} completed more than once"))
    }
}

fn movement_capture_path(out_dir: &std::path::Path, prefix: &str, index: u16) -> PathBuf {
    out_dir.join(format!("{}.png", movement_capture_name(prefix, index)))
}

fn remove_capture_file_if_present(path: &std::path::Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("cannot remove {}: {error}", path.display())),
    }
}

fn cleanup_movement_capture_outputs(
    out_dir: &std::path::Path,
    prefix: &str,
    capture_count: u16,
) -> Result<(), String> {
    let mut failures = Vec::new();
    for index in 0..capture_count {
        let path = movement_capture_path(out_dir, prefix, index);
        if let Err(error) = remove_capture_file_if_present(&path) {
            failures.push(error);
        }
        match temporary_capture_path(&path) {
            Ok(temporary) => {
                if let Err(error) = remove_capture_file_if_present(&temporary) {
                    failures.push(error);
                }
            }
            Err(error) => failures.push(format!(
                "cannot resolve temporary sequence path for {}: {error}",
                path.display()
            )),
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaptureTargetPreparation {
    /// The next capture must first detach and replace the shared target.
    Refresh,
    /// Both cameras must render into this exact new generation before screenshotting.
    Settling {
        expected_generation: u64,
        rendered_frames: u8,
    },
}

#[derive(Resource)]
struct WalkState {
    steps: Vec<WalkStep>,
    cursor: usize,
    out_dir: PathBuf,
    review: ReviewRunProvenance,
    launched_scenarios: Vec<ReviewScenarioProvenance>,
    settled: u32,
    step_started: Instant,
    capture_requested: bool,
    capture_outcome: Option<CaptureOutcome>,
    /// Bounded temporal recorder owned by one `CaptureWhileMoving` step.
    movement_capture: Option<MovementCaptureState>,
    /// Written frames awaiting the exact arrival proof required by the script.
    pending_movement_capture_proof: Option<PendingMovementCaptureProof>,
    /// Every successfully persisted frame, in script order, for exhaustive review.
    completed_captures: Vec<String>,
    /// A button pressed by the previous step, to be reset to `None`.
    pressed: Option<Entity>,
    /// A key pressed by the previous step, to be released.
    held_key: Option<KeyCode>,
    #[cfg(feature = "dev")]
    last_explorer_position: Option<Vec3>,
    /// Multi-frame ordinary right-drag currently being injected.
    orbit_gesture: Option<OrbitGesture>,
    /// The Bevy image target the game and UI render into for capture.
    target: Option<Handle<Image>>,
    /// Previous target retained until its replacement receives a distinct asset ID.
    retired_target: Option<Handle<Image>>,
    /// Monotonic identity for the shared game/UI image target.
    target_generation: u64,
    /// Per-capture refresh and render-settling state.
    capture_target: CaptureTargetPreparation,
    /// The camera entity the UI roots must be pointed at.
    camera: Option<Entity>,
    /// Exact logical canvas and raster density under review.
    viewport: hex_ui::ReviewViewport,
    /// Authored presentation fixture whose named composition contract is active.
    presentation: Option<String>,
    /// Debug outlines are useful diagnostics but invalidate acceptance evidence.
    diagnostic_overlays: bool,
    failed: bool,
}

#[derive(Component)]
struct WalkUiCamera;

#[derive(SystemParam)]
struct WalkContent<'w, 's> {
    failure: Option<Res<'w, GameplaySetupFailure>>,
    camera_mode: Option<Res<'w, CameraMode>>,
    library: Option<Res<'w, ScenarioLibrary>>,
    anchors: Option<Res<'w, MapAnchors>>,
    party: Option<Res<'w, Party>>,
    registry: Option<Res<'w, UnitRegistry>>,
    queue: Option<Res<'w, CommandQueue>>,
    movement: Query<'w, 's, (Has<Busy>, Has<MovingTo>, Option<&'static StandsOn>)>,
    selected: Query<
        'w,
        's,
        (
            Entity,
            &'static StandsOn,
            Option<&'static CameraFocusTarget>,
        ),
        With<Selected>,
    >,
    buttons: Query<'w, 's, (Entity, &'static Name), With<Button>>,
    tiles: Query<'w, 's, (Entity, &'static TilePos, &'static Headroom), With<HexTile>>,
}

#[derive(SystemParam)]
struct WalkInput<'w> {
    keys: ResMut<'w, ButtonInput<KeyCode>>,
    mouse: ResMut<'w, ButtonInput<MouseButton>>,
    cursor_moved: MessageWriter<'w, CursorMoved>,
}

impl WalkContent<'_, '_> {
    /// `None` means party facts are not ready yet. `Some(false)` means at least one
    /// stable party member still has a queued command or live domain route.
    fn party_is_idle(&self) -> Option<bool> {
        let (Some(party), Some(registry), Some(queue)) = (
            self.party.as_deref(),
            self.registry.as_deref(),
            self.queue.as_deref(),
        ) else {
            return None;
        };
        if party.members.is_empty() {
            return None;
        }
        for member in &party.members {
            if queue.holds_command_for(*member) {
                return Some(false);
            }
            let entity = registry.entity_of(*member)?;
            let Ok((busy, moving, _)) = self.movement.get(entity) else {
                return None;
            };
            if busy || moving {
                return Some(false);
            }
        }
        Some(true)
    }

    /// Whether the exact selected actor targeted by `ClickAnchor` owns a live
    /// domain route.
    ///
    /// Temporal evidence must not borrow an unrelated party member's movement:
    /// that could make an ignored click look successful until a later assertion.
    fn selected_movement_is_pending(&self) -> Option<bool> {
        let (entity, _, _) = self.selected.single().ok()?;
        let (_, moving, _) = self.movement.get(entity).ok()?;
        Some(moving)
    }

    fn assert_selected_at(&self, expected: TilePos) -> Result<(), String> {
        let (entity, standing, focus) = self.selected.single().map_err(|error| {
            format!("visual walk needs exactly one selected unit before position proof: {error}")
        })?;
        if standing.0.pos != expected {
            let party_positions = self
                .party
                .as_deref()
                .zip(self.registry.as_deref())
                .map(|(party, registry)| {
                    party
                        .members
                        .iter()
                        .filter_map(|unit| {
                            let entity = registry.entity_of(*unit)?;
                            let (_, _, standing) = self.movement.get(entity).ok()?;
                            Some((*unit, standing.map(|standing| standing.0.pos)))
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            return Err(format!(
                "selected unit {entity:?} stands at {:?}, not expected {expected:?}; party positions: {party_positions:?}",
                standing.0.pos,
            ));
        }
        let Some(focus) = focus else {
            return Err(format!(
                "selected unit {entity:?} reached {expected:?} without a camera focus projection"
            ));
        };
        if focus.surface != expected {
            return Err(format!(
                "selected unit {entity:?} reached {expected:?}, but camera focus remains at {:?}",
                focus.surface
            ));
        }
        Ok(())
    }
}

impl WalkState {
    fn new(
        steps: Vec<WalkStep>,
        out_dir: PathBuf,
        viewport: hex_ui::ReviewViewport,
        diagnostic_overlays: bool,
        review: ReviewRunProvenance,
    ) -> Self {
        Self {
            steps,
            cursor: 0,
            out_dir,
            review,
            launched_scenarios: Vec::new(),
            settled: 0,
            step_started: Instant::now(),
            capture_requested: false,
            capture_outcome: None,
            movement_capture: None,
            pending_movement_capture_proof: None,
            completed_captures: Vec::new(),
            pressed: None,
            held_key: None,
            #[cfg(feature = "dev")]
            last_explorer_position: None,
            orbit_gesture: None,
            target: None,
            retired_target: None,
            target_generation: 0,
            capture_target: CaptureTargetPreparation::Refresh,
            camera: None,
            viewport,
            presentation: None,
            diagnostic_overlays,
            failed: false,
        }
    }

    fn advance(&mut self) {
        self.cursor += 1;
        self.settled = 0;
        self.capture_requested = false;
        self.capture_outcome = None;
        self.capture_target = CaptureTargetPreparation::Refresh;
        self.step_started = Instant::now();
    }
}

fn begin_temporal_capture_time(time: &mut Time<Virtual>) {
    time.set_relative_speed(TEMPORAL_CAPTURE_TIME_SCALE);
}

fn restore_walk_time(time: &mut Time<Virtual>) {
    time.set_relative_speed(WALK_TIME_SCALE);
}

/// Stops one temporal sequence immediately and makes any late screenshot
/// callback harmless before removing its exact partial outputs.
fn abort_movement_capture(
    state: &mut WalkState,
    time: &mut Time<Virtual>,
    mut reason: String,
) -> String {
    restore_walk_time(time);
    if let Some(mut recording) = state.movement_capture.take() {
        recording.aborted = true;
        if let Err(cleanup) = cleanup_movement_capture_outputs(
            &state.out_dir,
            &recording.prefix,
            recording.capture_count,
        ) {
            reason.push_str(&format!("; active-sequence cleanup also failed: {cleanup}"));
            // Keep the aborted tombstone so `Drop` retries exact cleanup and late
            // renderer callbacks still cannot republish a partial frame.
            state.movement_capture = Some(recording);
        }
    }
    if let Some(proof) = state.pending_movement_capture_proof.take() {
        if let Err(cleanup) =
            cleanup_movement_capture_outputs(&state.out_dir, &proof.prefix, proof.capture_count)
        {
            reason.push_str(&format!("; arrival-proof cleanup also failed: {cleanup}"));
            // Retain the tombstone so `Drop` gets one final exact cleanup attempt.
            state.pending_movement_capture_proof = Some(proof);
        }
    }
    reason
}

impl Drop for WalkState {
    fn drop(&mut self) {
        if let Some(recording) = self.movement_capture.as_ref() {
            if let Err(error) = cleanup_movement_capture_outputs(
                &self.out_dir,
                &recording.prefix,
                recording.capture_count,
            ) {
                error!(
                    "visual walk could not clean aborted temporal capture {:?}: {error}",
                    recording.prefix
                );
            }
        }
        if let Some(proof) = self.pending_movement_capture_proof.as_ref() {
            if let Err(error) =
                cleanup_movement_capture_outputs(&self.out_dir, &proof.prefix, proof.capture_count)
            {
                error!(
                    "visual walk could not clean temporal capture {:?} after a missing arrival proof: {error}",
                    proof.prefix
                );
            }
        }
    }
}

fn install_walk_target(
    state: &mut WalkState,
    images: &mut Assets<Image>,
    target: Handle<Image>,
) -> Result<(), String> {
    if state
        .retired_target
        .as_ref()
        .is_some_and(|retired| retired.id() == target.id())
    {
        return Err("visual-walk replacement reused the retired render-target ID".to_owned());
    }
    state.target_generation = state
        .target_generation
        .checked_add(1)
        .ok_or_else(|| "visual-walk render-target generation overflowed".to_owned())?;
    state.target = Some(target);
    if let Some(retired) = state.retired_target.take() {
        images.remove(retired.id());
    }
    Ok(())
}

/// Makes each screenshot generation-owning instead of reusing an image whose 3D
/// render pass may have gone stale while UI composition continued to update.
fn prepare_capture_target(state: &mut WalkState) -> Result<bool, String> {
    match state.capture_target {
        CaptureTargetPreparation::Refresh => {
            let expected_generation = state
                .target_generation
                .checked_add(1)
                .ok_or_else(|| "visual-walk render-target generation overflowed".to_owned())?;
            if state.retired_target.is_some() {
                return Err("visual-walk still holds an unretired render target".to_owned());
            }
            state.retired_target = state.target.take();
            state.capture_target = CaptureTargetPreparation::Settling {
                expected_generation,
                rendered_frames: 0,
            };
            Ok(false)
        }
        CaptureTargetPreparation::Settling {
            expected_generation,
            rendered_frames,
        } => {
            if state.target_generation > expected_generation {
                return Err(format!(
                    "visual-walk render target advanced past capture generation \
                     {expected_generation} to {}",
                    state.target_generation
                ));
            }
            if state.target_generation < expected_generation || state.target.is_none() {
                return Ok(false);
            }
            if rendered_frames < CAPTURE_TARGET_SETTLE_FRAMES {
                state.capture_target = CaptureTargetPreparation::Settling {
                    expected_generation,
                    rendered_frames: rendered_frames + 1,
                };
                return Ok(false);
            }
            Ok(true)
        }
    }
}

fn shared_target_msaa_update(source: Msaa, target: Msaa) -> Option<Msaa> {
    (target != source).then_some(source)
}

fn capture_structural_issues(
    snapshot: &hex_ui::test_support::UiTreeSnapshot,
    presentation: Option<&str>,
    task: Option<hex_ui::test_support::UiTaskCase>,
    screen: Option<Screen>,
) -> Vec<String> {
    let mut issues = task.map_or_else(
        || {
            presentation.map_or_else(
                || snapshot.layout_issues(),
                |fixture| snapshot.review_fixture_issues(fixture),
            )
        },
        |task| snapshot.task_issues(task),
    );
    if let (Some(fixture), Some(task)) = (presentation, task) {
        let compatible = match fixture {
            "normal-gameplay" => matches!(
                task,
                hex_ui::test_support::UiTaskCase::Exploration
                    | hex_ui::test_support::UiTaskCase::CharacterMainView
                    | hex_ui::test_support::UiTaskCase::ActivityTabs
                    | hex_ui::test_support::UiTaskCase::CustomHudVisibility
                    | hex_ui::test_support::UiTaskCase::CompactTemporarySurface
            ),
            "player-turn-max" => task == hex_ui::test_support::UiTaskCase::PlayerTurnMaxActions,
            "hostile-turn" => task == hex_ui::test_support::UiTaskCase::HostileTurn,
            "casting-list" => task == hex_ui::test_support::UiTaskCase::Casting,
            "aiming-disabled" => task == hex_ui::test_support::UiTaskCase::AimingBlocked,
            "required-decision" => matches!(
                task,
                hex_ui::test_support::UiTaskCase::DisableDecision
                    | hex_ui::test_support::UiTaskCase::HudHiddenRequired
            ),
            "restore-decision" => task == hex_ui::test_support::UiTaskCase::RestoreDecision,
            "sandbox-outcome" => task == hex_ui::test_support::UiTaskCase::SandboxOutcome,
            _ => false,
        };
        if !compatible {
            issues.push(format!(
                "presentation fixture {fixture:?} cannot satisfy task {:?}",
                task.contract().id
            ));
        }
    }
    if let Some(screen) = screen {
        if let Some(task) = task {
            let expected = task.contract().screen;
            if expected != screen {
                issues.push(format!(
                    "capture task {:?} belongs to {expected:?}, not active screen {screen:?}",
                    task.contract().id
                ));
            }
        }
        let expected_roots: &[&str] = match screen {
            Screen::Splash => &["Splash Screen"],
            Screen::Title => &["Main Menu"],
            Screen::Settings => &["Settings Screen"],
            Screen::LatticeDemo => &["Lattice Demo Screen"],
            Screen::VfxTuner => &["VFX Tuner Screen"],
            Screen::CharacterCreator | Screen::SpellCreator => &["Creator Screen"],
            Screen::Sandbox => &["Sandbox"],
            Screen::Multiplayer => &["Multiplayer"],
            Screen::Loading => &["Loading Screen"],
            Screen::Gameplay => &["Gameplay HUD Safe Frame"],
        };
        if !snapshot
            .nodes
            .iter()
            .any(|node| expected_roots.iter().any(|expected| node.name == *expected))
        {
            issues.push(format!(
                "capture snapshot for {screen:?} is stale or incomplete; expected one of {expected_roots:?}"
            ));
        }
    }
    issues
}

#[expect(
    clippy::too_many_arguments,
    reason = "the runner is one sequential state machine over the app's surfaces; \
              splitting it would scatter the step semantics across systems"
)]
fn run_walk(
    mut commands: Commands,
    #[cfg(feature = "dev")] explorer: Option<Res<crate::fly::Explorer>>,
    mut state: ResMut<WalkState>,
    screen: Res<State<Screen>>,
    mut next: ResMut<NextState<Screen>>,
    content: WalkContent,
    mut walk_time: ResMut<Time<Virtual>>,
    mut ui_scale: ResMut<hex_ui::UiScalePreference>,
    mut primary_window: Query<(Entity, &mut Window), With<bevy::window::PrimaryWindow>>,
    mut input: WalkInput,
    mut images: ResMut<Assets<Image>>,
    mut game_camera: Query<(&mut RenderTarget, &Msaa), (With<Camera3d>, Without<WalkUiCamera>)>,
    mut review_camera: Query<
        (Entity, &mut RenderTarget, &mut Msaa),
        (With<WalkUiCamera>, Without<Camera3d>),
    >,
    ui_roots: Query<(Entity, Option<&UiTargetCamera>), (With<Node>, Without<ChildOf>)>,
    latest_ui_tree: Res<hex_ui::test_support::LatestUiTreeSnapshot>,
    mut exit: MessageWriter<AppExit>,
) {
    if state.failed {
        return;
    }

    // The Bevy image target owns capture pixels. Keep the ordinary window's
    // requested logical size aligned so the runtime semantic-metrics system sees
    // the same canvas without overriding the operating system's scale factor.
    if let Ok((_, mut window)) = primary_window.single_mut() {
        let logical = state.viewport.logical_size.as_vec2();
        if (window.width() - logical.x).abs() > 0.5 || (window.height() - logical.y).abs() > 0.5 {
            window.resolution.set(logical.x, logical.y);
        }
    }

    // Redirect the game's single camera into an explicitly scaled Bevy image.
    if state.target.is_none() {
        let Ok((mut game_target, game_msaa)) = game_camera.single_mut() else {
            if state.movement_capture.is_some() {
                let reason = abort_movement_capture(
                    &mut state,
                    &mut walk_time,
                    "the game camera disappeared during temporal capture".to_owned(),
                );
                error!("visual walk temporal capture failed: {reason}");
                state.failed = true;
                exit.write(AppExit::error());
            }
            return;
        };
        let Ok(physical_size) = state.viewport.physical_size() else {
            let reason = abort_movement_capture(
                &mut state,
                &mut walk_time,
                "visual walk viewport became invalid".to_owned(),
            );
            error!("{reason}");
            state.failed = true;
            exit.write(AppExit::error());
            return;
        };
        let image = Image::new_target_texture(
            physical_size.x,
            physical_size.y,
            TextureFormat::Bgra8UnormSrgb,
            None,
        );
        let handle = images.add(image);
        let render_target = RenderTarget::Image(ImageRenderTarget {
            handle: handle.clone(),
            scale_factor: state.viewport.device_scale,
        });
        *game_target = render_target.clone();
        let ui_camera = if let Ok((camera, mut target, mut ui_msaa)) = review_camera.single_mut() {
            *target = render_target.clone();
            if let Some(wanted) = shared_target_msaa_update(*game_msaa, *ui_msaa) {
                *ui_msaa = wanted;
            }
            camera
        } else {
            commands
                .spawn((
                    Name::new("Visual Walk UI Camera"),
                    WalkUiCamera,
                    Camera2d,
                    Camera {
                        order: 1,
                        clear_color: ClearColorConfig::None,
                        ..default()
                    },
                    *game_msaa,
                    render_target,
                ))
                .id()
        };
        if let Err(reason) = install_walk_target(&mut state, &mut images, handle) {
            let reason = abort_movement_capture(&mut state, &mut walk_time, reason);
            error!("{reason}");
            state.failed = true;
            exit.write(AppExit::error());
            return;
        }
        state.camera = Some(ui_camera);
    }

    // A shared render target requires compatible sampling across every camera.
    // Character tree fading enables OIT and therefore turns the 3D camera's MSAA
    // off; mirror that exact setting onto the tooling-only UI camera and restore it
    // change-driven when OIT leaves. Otherwise Bevy may keep presenting the last
    // compatible 3D pass while the UI pass alone continues to update.
    if let (Ok((_, game_msaa)), Ok((_, _, mut ui_msaa))) =
        (game_camera.single(), review_camera.single_mut())
    {
        if let Some(wanted) = shared_target_msaa_update(*game_msaa, *ui_msaa) {
            *ui_msaa = wanted;
        }
    }

    // UI roots spawn and despawn with every screen; keep pointing new ones at
    // the redirected camera or their screens render into nothing.
    if let Some(camera) = state.camera {
        let mut retargeted = false;
        for (root, target) in &ui_roots {
            if target.is_none_or(|target| target.entity() != camera) {
                commands.entity(root).insert(UiTargetCamera(camera));
                retargeted = true;
            }
        }
        if retargeted {
            if state.movement_capture.is_some() || state.pending_movement_capture_proof.is_some() {
                let reason = abort_movement_capture(
                    &mut state,
                    &mut walk_time,
                    "UI camera roots changed before temporal capture received its exact arrival proof"
                        .to_owned(),
                );
                error!("visual walk temporal capture failed: {reason}");
                state.failed = true;
                exit.write(AppExit::error());
            }
            return;
        }
    }
    if let Some(failure) = content.failure.as_deref() {
        let reason = abort_movement_capture(
            &mut state,
            &mut walk_time,
            format!("gameplay setup failed: {}", failure.reason),
        );
        error!("visual walk aborted: {reason}");
        state.failed = true;
        exit.write(AppExit::error());
        return;
    }

    // Cleanups owed from the previous step, regardless of what runs now.
    if let Some(entity) = state.pressed.take() {
        if content.buttons.contains(entity) {
            commands.entity(entity).insert(Interaction::None);
        }
    }
    if let Some(key) = state.held_key.take() {
        input.keys.release(key);
    }

    let Some(step) = state.steps.get(state.cursor).cloned() else {
        if state.movement_capture.is_some() || state.pending_movement_capture_proof.is_some() {
            let reason = abort_movement_capture(
                &mut state,
                &mut walk_time,
                "visual walk ended before temporal movement received its exact arrival proof"
                    .to_owned(),
            );
            error!("visual walk refused an incomplete temporal contract: {reason}");
            state.failed = true;
            exit.write(AppExit::error());
            return;
        }
        let review_index = match write_completed_review_index(
            &state.out_dir,
            &state.review,
            &state.launched_scenarios,
            &state.completed_captures,
        ) {
            Ok(path) => path,
            Err(reason) => {
                error!("visual walk could not publish its exhaustive review index: {reason}");
                state.failed = true;
                exit.write(AppExit::error());
                return;
            }
        };
        info!(
            "visual walk complete: {} steps, {} captures indexed at {}",
            state.steps.len(),
            state.completed_captures.len(),
            review_index.display()
        );
        restore_walk_time(&mut walk_time);
        input.mouse.release(MouseButton::Right);
        exit.write(AppExit::Success);
        state.failed = true;
        return;
    };
    let review_task = match &step {
        WalkStep::ReviewCapture { task, .. } => Some(*task),
        _ => None,
    };

    let timeout = step_timeout(&step);
    if state.step_started.elapsed() > timeout {
        let timeout_reason = format!(
            "step {} ({step:?}) timed out after {:.0}s on screen {:?}",
            state.cursor,
            timeout.as_secs_f32(),
            screen.get()
        );
        let reason = abort_movement_capture(&mut state, &mut walk_time, timeout_reason);
        error!("visual walk timed out: {reason}");
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
        WalkStep::AwaitGameplay { .. } => {
            if *screen.get() == Screen::Gameplay {
                state.advance();
            }
        }
        WalkStep::AwaitTerrain => {
            if !content.tiles.is_empty() {
                state.advance();
            }
        }
        WalkStep::CaptureWhileMoving {
            ref prefix,
            every_frames,
            capture_count,
        } => {
            if state.movement_capture.is_none() {
                if state.pending_movement_capture_proof.is_some() {
                    let reason = abort_movement_capture(
                        &mut state,
                        &mut walk_time,
                        "a new temporal sequence started before the prior arrival proof completed"
                            .to_owned(),
                    );
                    error!("visual walk temporal capture {prefix:?} failed: {reason}");
                    state.failed = true;
                    exit.write(AppExit::error());
                    return;
                }
                if let Err(reason) =
                    cleanup_movement_capture_outputs(&state.out_dir, prefix, capture_count)
                {
                    restore_walk_time(&mut walk_time);
                    error!("visual walk could not prepare temporal capture {prefix:?}: {reason}");
                    state.failed = true;
                    exit.write(AppExit::error());
                    return;
                }
                begin_temporal_capture_time(&mut walk_time);
                state.movement_capture = Some(MovementCaptureState::new(
                    prefix.clone(),
                    every_frames,
                    capture_count,
                ));
            }

            let ready_failure = state.movement_capture.as_ref().and_then(|recording| {
                recording
                    .failure
                    .clone()
                    .or_else(|| recording.first_outcome_failure())
            });
            if let Some(reason) = ready_failure {
                let reason = abort_movement_capture(&mut state, &mut walk_time, reason);
                error!("visual walk temporal capture {prefix:?} failed: {reason}");
                state.failed = true;
                exit.write(AppExit::error());
                return;
            }

            let completed = state.movement_capture.as_ref().is_some_and(|recording| {
                recording.scheduled_all() && recording.all_requested_finished()
            });
            if completed {
                let Some(recording) = state.movement_capture.take() else {
                    restore_walk_time(&mut walk_time);
                    error!("visual walk lost its completed temporal-capture state");
                    state.failed = true;
                    exit.write(AppExit::error());
                    return;
                };
                info!(
                    "visual walk wrote {} temporal frames with prefix {:?}; awaiting exact arrival proof",
                    recording.capture_count, recording.prefix
                );
                state.pending_movement_capture_proof = Some(PendingMovementCaptureProof {
                    prefix: recording.prefix,
                    capture_count: recording.capture_count,
                });
                restore_walk_time(&mut walk_time);
                state.advance();
                return;
            }
            if state
                .movement_capture
                .as_ref()
                .is_some_and(MovementCaptureState::scheduled_all)
            {
                return;
            }

            let Some(snapshot) = latest_ui_tree.0.as_ref() else {
                let reason = abort_movement_capture(
                    &mut state,
                    &mut walk_time,
                    "the UI structural snapshot disappeared during temporal capture".to_owned(),
                );
                error!("visual walk temporal capture {prefix:?} failed: {reason}");
                state.failed = true;
                exit.write(AppExit::error());
                return;
            };
            let issues = capture_structural_issues(
                snapshot,
                state.presentation.as_deref(),
                None,
                Some(*screen.get()),
            );
            if !issues.is_empty() {
                let reason = abort_movement_capture(
                    &mut state,
                    &mut walk_time,
                    format!(
                        "structural oracle rejected the temporal sequence:\n{}",
                        issues.join("\n")
                    ),
                );
                error!("visual walk temporal capture {prefix:?} failed: {reason}");
                state.failed = true;
                exit.write(AppExit::error());
                return;
            }
            let expected_logical = state.viewport.logical_size.as_vec2();
            let logical_error = (snapshot.metrics.logical_size - expected_logical)
                .abs()
                .max_element();
            if logical_error > 0.5 {
                let reason = abort_movement_capture(
                    &mut state,
                    &mut walk_time,
                    format!(
                        "temporal snapshot logical size {:?} did not match {:?}",
                        snapshot.metrics.logical_size, expected_logical
                    ),
                );
                error!("visual walk temporal capture {prefix:?} failed: {reason}");
                state.failed = true;
                exit.write(AppExit::error());
                return;
            }

            let Some(selected_movement_is_pending) = content.selected_movement_is_pending() else {
                let reason = abort_movement_capture(
                    &mut state,
                    &mut walk_time,
                    "the exact selected actor disappeared during temporal capture".to_owned(),
                );
                error!("visual walk temporal capture {prefix:?} failed: {reason}");
                state.failed = true;
                exit.write(AppExit::error());
                return;
            };
            let Some(target) = state.target.clone() else {
                let reason = abort_movement_capture(
                    &mut state,
                    &mut walk_time,
                    "the shared render target disappeared during temporal capture".to_owned(),
                );
                error!("visual walk temporal capture {prefix:?} failed: {reason}");
                state.failed = true;
                exit.write(AppExit::error());
                return;
            };
            let Ok(expected_physical) = state.viewport.physical_size() else {
                let reason = abort_movement_capture(
                    &mut state,
                    &mut walk_time,
                    "review viewport physical size became invalid during temporal capture"
                        .to_owned(),
                );
                error!("visual walk temporal capture {prefix:?} failed: {reason}");
                state.failed = true;
                exit.write(AppExit::error());
                return;
            };
            let tick = if let Some(recording) = state.movement_capture.as_mut() {
                recording.observe_movement_frame(selected_movement_is_pending)
            } else {
                restore_walk_time(&mut walk_time);
                error!("visual walk lost its temporal-capture state");
                state.failed = true;
                exit.write(AppExit::error());
                return;
            };
            let frame_index = match tick {
                Ok(MovementCaptureTick::Capture(index)) => index,
                Ok(
                    MovementCaptureTick::WaitingForMovement
                    | MovementCaptureTick::WaitingForInterval,
                ) => return,
                Err(reason) => {
                    let reason = abort_movement_capture(&mut state, &mut walk_time, reason);
                    error!("visual walk temporal capture {prefix:?} failed: {reason}");
                    state.failed = true;
                    exit.write(AppExit::error());
                    return;
                }
            };

            let path = movement_capture_path(&state.out_dir, prefix, frame_index);
            let callback_prefix = prefix.clone();
            info!(
                "visual walk requesting temporal frame {} at pending movement frame {}",
                path.display(),
                state
                    .movement_capture
                    .as_ref()
                    .map_or(0, |recording| recording.pending_frames)
            );
            // Do not call `prepare_capture_target` here: replacing and settling
            // the target would disrupt the fixed-interval temporal sample. This
            // diagnostic reads the continuously rendered walk target. Advancing
            // the step resets `capture_target`, so the next ordinary acceptance
            // Capture still owns a fresh generation and its full stale guard.
            let mut screenshot = Screenshot::image(target.clone());
            screenshot.0 = RenderTarget::Image(ImageRenderTarget {
                handle: target,
                scale_factor: state.viewport.device_scale,
            });
            commands.spawn(screenshot).observe(
                move |captured: On<ScreenshotCaptured>, mut state: ResMut<WalkState>| {
                    let callback_error = match state.movement_capture.as_ref() {
                        None => Some(format!(
                            "temporal frame {frame_index} for {callback_prefix:?} arrived without an active recorder"
                        )),
                        Some(recording) => {
                            recording.callback_issue(&callback_prefix, frame_index)
                        }
                    };
                    if let Some(error) = callback_error {
                        if let Some(recording) = state.movement_capture.as_mut() {
                            if recording.failure.is_none() {
                                recording.failure = Some(error.clone());
                            }
                        }
                        error!("visual walk rejected renderer callback: {error}");
                        return;
                    }

                    let outcome = if captured.image.size() != expected_physical {
                        CaptureOutcome::Failed(format!(
                            "capture size {:?} did not match review target {expected_physical:?}",
                            captured.image.size()
                        ))
                    } else {
                        match write_png(&captured.image, &path) {
                            Ok(stats) => CaptureOutcome::Written {
                                brightest: stats.brightest,
                                coverage: stats.has_coverage,
                            },
                            Err(error) => CaptureOutcome::Failed(error),
                        }
                    };
                    let Some(recording) = state.movement_capture.as_mut() else {
                        error!("visual walk lost the recorder after validating frame {frame_index}");
                        return;
                    };
                    if recording.outcomes.insert(frame_index, outcome).is_some()
                        && recording.failure.is_none()
                    {
                        recording.failure =
                            Some(format!("temporal frame {frame_index} completed more than once"));
                    }
                },
            );
        }
        WalkStep::AwaitPartyIdle { max_frames } => {
            state.settled = state.settled.saturating_add(1);
            if content.party_is_idle() == Some(true) {
                state.advance();
            } else if state.settled >= max_frames {
                let reason = abort_movement_capture(
                    &mut state,
                    &mut walk_time,
                    format!(
                        "visual walk exhausted AwaitPartyIdle after {max_frames} frames; party facts: {:?}",
                        content.party_is_idle()
                    ),
                );
                error!("{reason}");
                state.failed = true;
                exit.write(AppExit::error());
            }
        }
        WalkStep::AssertSelectedAt { expected } => {
            let expected = expected.position();
            match content.assert_selected_at(expected) {
                Ok(()) => {
                    info!("visual walk proved selected unit and camera focus at {expected:?}");
                    if let Some(proof) = state.pending_movement_capture_proof.take() {
                        for index in 0..proof.capture_count {
                            state
                                .completed_captures
                                .push(movement_capture_name(&proof.prefix, index));
                        }
                        info!(
                            "visual walk accepted {} temporal frames with prefix {:?}",
                            proof.capture_count, proof.prefix
                        );
                    }
                    state.advance();
                }
                Err(reason) => {
                    let reason = abort_movement_capture(&mut state, &mut walk_time, reason);
                    error!("visual walk rejected position evidence: {reason}");
                    state.failed = true;
                    exit.write(AppExit::error());
                }
            }
        }
        WalkStep::AssertCameraMode(expected) => {
            let Some(actual) = content.camera_mode.as_deref() else {
                error!("visual walk cannot assert camera mode before CameraMode exists");
                state.failed = true;
                exit.write(AppExit::error());
                return;
            };
            if expected.matches(*actual) {
                info!("visual walk proved camera mode {expected:?}");
                state.advance();
            } else {
                error!(
                    "visual walk expected camera mode {expected:?}, found {:?}",
                    *actual
                );
                state.failed = true;
                exit.write(AppExit::error());
            }
        }
        WalkStep::Settle(frames) => {
            state.settled += 1;
            if state.settled >= frames {
                state.advance();
            }
        }
        WalkStep::Capture(ref name) | WalkStep::ReviewCapture { ref name, .. } => {
            if review_task.is_some() && state.diagnostic_overlays {
                error!(
                    "visual walk rejected acceptance capture {name:?}: {UI_DEBUG_ENV} enables diagnostic overlays"
                );
                state.failed = true;
                exit.write(AppExit::error());
                return;
            }
            match prepare_capture_target(&mut state) {
                Ok(true) => {}
                Ok(false) => return,
                Err(reason) => {
                    error!("visual walk capture {name} failed: {reason}");
                    state.failed = true;
                    exit.write(AppExit::error());
                    return;
                }
            }
            if !state.capture_requested {
                let Some(snapshot) = latest_ui_tree.0.as_ref() else {
                    return;
                };
                let issues = capture_structural_issues(
                    snapshot,
                    state.presentation.as_deref(),
                    review_task,
                    Some(*screen.get()),
                );
                if !issues.is_empty() {
                    error!(
                        "visual walk structural oracle rejected {name}:\n{}",
                        issues.join("\n")
                    );
                    state.failed = true;
                    exit.write(AppExit::error());
                    return;
                }
                let expected_logical = state.viewport.logical_size.as_vec2();
                if (snapshot.metrics.logical_size - expected_logical)
                    .abs()
                    .max_element()
                    > 0.5
                {
                    return;
                }
                let Some(target) = state.target.clone() else {
                    return;
                };
                let Ok(expected_physical) = state.viewport.physical_size() else {
                    state.capture_outcome = Some(CaptureOutcome::Failed(
                        "review viewport physical size is invalid".to_owned(),
                    ));
                    state.capture_requested = true;
                    return;
                };
                let path = state.out_dir.join(format!("{name}.png"));
                info!("visual walk capturing {}", path.display());
                let mut screenshot = Screenshot::image(target.clone());
                // Bevy's convenience constructor defaults image targets to 1×.
                // Preserve the reviewed target's device scale so the screenshot
                // render-graph key matches the cameras' ImageRenderTarget.
                screenshot.0 = RenderTarget::Image(ImageRenderTarget {
                    handle: target,
                    scale_factor: state.viewport.device_scale,
                });
                commands.spawn(screenshot).observe(
                    move |captured: On<ScreenshotCaptured>, mut state: ResMut<WalkState>| {
                        let outcome = if captured.image.size() != expected_physical {
                            CaptureOutcome::Failed(format!(
                                "capture size {:?} did not match review target {expected_physical:?}",
                                captured.image.size()
                            ))
                        } else {
                            match write_png(&captured.image, &path) {
                                Ok(stats) => CaptureOutcome::Written {
                                    brightest: stats.brightest,
                                    coverage: stats.has_coverage,
                                },
                                Err(error) => CaptureOutcome::Failed(error),
                            }
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
                    state.completed_captures.push(name.clone());
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
            let mut matches: Vec<(Entity, String)> = content
                .buttons
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
        WalkStep::ClickTile { q, r, level } => {
            let coord = HexCoord::from_axial(q, r);
            match resolve_tile_click_target(content.tiles.iter(), coord, level) {
                Ok(None) => {}
                Ok(Some((target, pos))) => {
                    let Ok((window, _)) = primary_window.single() else {
                        return;
                    };
                    let Some(click) = primary_tile_click(target, window) else {
                        error!("visual walk could not normalize the primary window for {step:?}");
                        state.failed = true;
                        exit.write(AppExit::error());
                        return;
                    };
                    info!("visual walk clicking terrain {pos:?} through pointer picking");
                    commands.trigger(click);
                    state.advance();
                }
                Err(reason) => {
                    error!("visual walk refused {step:?}: {reason}");
                    state.failed = true;
                    exit.write(AppExit::error());
                }
            }
        }
        WalkStep::HoverTile { q, r, level } => {
            let coord = HexCoord::from_axial(q, r);
            match resolve_tile_click_target(content.tiles.iter(), coord, level) {
                Ok(None) => {}
                Ok(Some((target, pos))) => {
                    let Ok((window, _)) = primary_window.single() else {
                        return;
                    };
                    let Some(over) = primary_tile_hover(target, window) else {
                        error!("visual walk could not normalize the primary window for {step:?}");
                        state.failed = true;
                        exit.write(AppExit::error());
                        return;
                    };
                    info!("visual walk hovering terrain {pos:?} through pointer picking");
                    commands.trigger(over);
                    state.advance();
                }
                Err(reason) => {
                    error!("visual walk refused {step:?}: {reason}");
                    state.failed = true;
                    exit.write(AppExit::error());
                }
            }
        }
        WalkStep::ClickAnchor { ref name, expected } => {
            let starts_temporal_capture = matches!(
                state.steps.get(state.cursor + 1),
                Some(WalkStep::CaptureWhileMoving { .. })
            );
            if starts_temporal_capture && state.settled == 0 {
                // `Time<Virtual>` advances before PreUpdate. Arm 1x speed for
                // one complete update before issuing the click so movement
                // cannot consume a leftover accelerated delta on its first tick.
                begin_temporal_capture_time(&mut walk_time);
                state.settled = 1;
                return;
            }
            let Some(anchors) = content.anchors.as_deref() else {
                return;
            };
            let id = MapAnchorId::from(name.as_str());
            let Some(actual) = anchors.get(&id) else {
                if starts_temporal_capture {
                    restore_walk_time(&mut walk_time);
                }
                error!("visual walk anchor {name:?} is not published by this map");
                state.failed = true;
                exit.write(AppExit::error());
                return;
            };
            let expected = expected.position();
            if actual != expected {
                if starts_temporal_capture {
                    restore_walk_time(&mut walk_time);
                }
                error!(
                    "visual walk anchor {name:?} moved from expected {expected:?} to {actual:?}; \
                     recapture and review the route before updating its stale detector"
                );
                state.failed = true;
                exit.write(AppExit::error());
                return;
            }
            match resolve_tile_click_target(content.tiles.iter(), actual.coord, Some(actual.level))
            {
                Ok(None) => {}
                Ok(Some((target, pos))) => {
                    let Ok((window, _)) = primary_window.single() else {
                        return;
                    };
                    let Some(click) = primary_tile_click(target, window) else {
                        if starts_temporal_capture {
                            restore_walk_time(&mut walk_time);
                        }
                        error!("visual walk could not normalize the primary window for {step:?}");
                        state.failed = true;
                        exit.write(AppExit::error());
                        return;
                    };
                    info!(
                        "visual walk clicking anchor {name:?} at {pos:?} through pointer picking"
                    );
                    if starts_temporal_capture {
                        begin_temporal_capture_time(&mut walk_time);
                    }
                    commands.trigger(click);
                    state.advance();
                }
                Err(reason) => {
                    if starts_temporal_capture {
                        restore_walk_time(&mut walk_time);
                    }
                    error!("visual walk refused {step:?}: {reason}");
                    state.failed = true;
                    exit.write(AppExit::error());
                }
            }
        }
        WalkStep::AwaitButton(ref name) => {
            if content
                .buttons
                .iter()
                .any(|(_, button_name)| button_name.as_str().starts_with(name.as_str()))
            {
                state.advance();
            }
        }
        WalkStep::PresentUi(ref name) => {
            if let Err(reason) = hex_ui::apply_ui_review_fixture(&mut commands, name) {
                error!("visual walk: {reason}");
                state.failed = true;
                exit.write(AppExit::error());
                return;
            }
            state.presentation = (name != "clear").then(|| name.clone());
            state.advance();
        }
        WalkStep::SetUiScale(mode) => {
            ui_scale.0 = mode;
            state.advance();
        }
        WalkStep::SetViewport {
            width,
            height,
            device_scale,
        } => match hex_ui::ReviewViewport::new(width, height, device_scale) {
            Ok(viewport) => {
                state.viewport = viewport;
                state.retired_target = state.target.take();
                state.advance();
            }
            Err(error) => {
                error!("visual walk viewport is invalid: {error}");
                state.failed = true;
                exit.write(AppExit::error());
            }
        },
        WalkStep::OrbitCamera {
            yaw_turns,
            pitch_fraction,
        } => {
            if *screen.get() != Screen::Gameplay {
                error!("visual walk camera orbit is only valid during Gameplay");
                state.failed = true;
                exit.write(AppExit::error());
                return;
            }
            let Ok((window_entity, window)) = primary_window.single_mut() else {
                return;
            };
            match state.orbit_gesture.as_mut() {
                None => {
                    let size = Vec2::new(window.width(), window.height());
                    let (baseline, destination) = match orbit_cursor_positions(
                        size,
                        window.cursor_position(),
                        yaw_turns,
                        pitch_fraction,
                    ) {
                        Ok(positions) => positions,
                        Err(error) => {
                            error!("visual walk camera orbit is invalid: {error}");
                            state.failed = true;
                            exit.write(AppExit::error());
                            return;
                        }
                    };
                    input.mouse.press(MouseButton::Right);
                    input.cursor_moved.write(CursorMoved {
                        window: window_entity,
                        position: baseline,
                        delta: None,
                    });
                    state.orbit_gesture = Some(OrbitGesture {
                        window: window_entity,
                        baseline,
                        destination,
                        phase: OrbitGesturePhase::Delta,
                    });
                }
                Some(gesture) if gesture.phase == OrbitGesturePhase::Delta => {
                    input.cursor_moved.write(CursorMoved {
                        window: gesture.window,
                        position: gesture.destination,
                        delta: Some(gesture.destination - gesture.baseline),
                    });
                    gesture.phase = OrbitGesturePhase::Release;
                }
                Some(_) => {
                    input.mouse.release(MouseButton::Right);
                    state.orbit_gesture = None;
                    state.advance();
                }
            }
        }
        WalkStep::HoldKeys { ref keys, frames } => {
            for name in keys {
                if let Ok(key) = parse_key(name) {
                    if state.settled < frames {
                        input.keys.press(key);
                    } else {
                        input.keys.release(key);
                    }
                }
            }
            if state.settled < frames {
                state.settled += 1;
            } else {
                state.advance();
            }
        }
        #[cfg(feature = "dev")]
        WalkStep::AssertExplorer {
            ref mode,
            grounded,
            minimum_displacement,
        } => {
            let observation = explorer.as_deref().map(crate::fly::Explorer::observation);
            if observation.is_some_and(|(actual, on_ground, position)| {
                actual == mode
                    && on_ground == grounded
                    && minimum_displacement.is_none_or(|distance| {
                        state
                            .last_explorer_position
                            .is_some_and(|previous| previous.distance(position) >= distance)
                    })
            }) {
                info!("exploration observation: {observation:?}");
                state.last_explorer_position = observation.map(|(_, _, position)| position);
                state.advance();
            } else {
                error!("exploration assertion failed: wanted {mode} grounded={grounded}, minimum displacement={minimum_displacement:?}, got {observation:?}");
                state.failed = true;
                exit.write(AppExit::error());
            }
        }
        WalkStep::Key(ref name) => {
            let key = parse_key(name).unwrap_or(KeyCode::Escape);
            info!("visual walk pressing {name}");
            input.keys.press(key);
            state.held_key = Some(key);
            state.advance();
        }
        WalkStep::StartScenario {
            ref name,
            seed,
            suppress_hostiles,
        } => {
            let Some(library) = content.library.as_deref() else {
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
            state.launched_scenarios.push(ReviewScenarioProvenance {
                name: name.clone(),
                seed: resolved_seed.map(|resolved| resolved.0),
            });
            if suppress_hostiles {
                commands.insert_resource(SuppressHostilesForMapReview);
            } else {
                commands.remove_resource::<SuppressHostilesForMapReview>();
            }
            commands.insert_resource(ScenarioToLoad {
                scenario,
                resolved_seed,
                encounter_override: None,
            });
            next.set(Screen::Loading);
            state.advance();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIRST_PERSON_CAMERA_SCRIPT: &str = "../../walks/camera_first_person.ron";
    const CRYSTAL_ASCENT_CAMERA_SCRIPT: &str = "../../walks/camera_crystal_ascent.ron";
    const CRYSTAL_MOUNTAIN_CAMERA_SCRIPT: &str = "../../walks/camera_crystal_mountain.ron";
    const DESERT_TRANSITION_CAMERA_SCRIPT: &str = "../../walks/camera_desert_transition.ron";
    const DESERT_PLAIN_CAMERA_SCRIPT: &str = "../../walks/camera_desert_plain.ron";
    const DUNES_CAMERA_SCRIPT: &str = "../../walks/camera_dunes.ron";
    const DESERT_OASIS_RINGS_CAMERA_SCRIPT: &str = "../../walks/camera_desert_oasis_rings.ron";
    const SANDY_ISLETS_CAMERA_SCRIPT: &str = "../../walks/camera_sandy_islets.ron";
    const WOODED_ISLAND_CAMERA_SCRIPT: &str = "../../walks/camera_wooded_island.ron";
    const OCEAN_ARCHIPELAGOES_CAMERA_SCRIPT: &str = "../../walks/camera_ocean_archipelagoes.ron";
    const GRAND_V3_BASELINE_CAMERA_SCRIPT: &str = "../../walks/camera_grand_v3_baseline.ron";
    const GRAND_V3_CORRECTIVE_MOTION_SCRIPT: &str =
        "../../walks/camera_grand_v3_corrective_motion.ron";

    /// Supplemental temporal gates deliberately stay outside the one-static-route-
    /// per-scenario manifest contract enforced by `CAMERA_ROUTE_SCRIPTS`.
    const AUXILIARY_CAMERA_REVIEW_SCRIPTS: &[(&str, &str)] =
        &[(GRAND_V3_CORRECTIVE_MOTION_SCRIPT, "Grand V3 Baseline")];

    const CAMERA_ROUTE_SCRIPTS: &[(&str, &str)] = &[
        ("../../walks/camera_crossing.ron", "The Crossing"),
        (
            "../../walks/camera_procedural_hills.ron",
            "Procedural Hills",
        ),
        ("../../walks/camera_rolling_hills.ron", "Rolling Hills"),
        ("../../walks/camera_frozen_hills.ron", "Frozen Hills"),
        ("../../walks/camera_volcanic_hills.ron", "Volcanic Hills"),
        ("../../walks/camera_sky_islands.ron", "Sky Islands"),
        ("../../walks/camera_mountains.ron", "Mountains"),
        ("../../walks/camera_caves.ron", "Caves"),
        ("../../walks/camera_waterfall.ron", "Waterfall"),
        ("../../walks/camera_forest.ron", "Forest"),
        ("../../walks/camera_deep_forest.ron", "Deep Forest"),
        ("../../walks/camera_prairie.ron", "Prairie"),
        (
            "../../walks/camera_desert_transition.ron",
            "Desert Transition",
        ),
        ("../../walks/camera_desert_plain.ron", "Desert Plain"),
        ("../../walks/camera_dunes.ron", "Dunes"),
        (
            "../../walks/camera_desert_oasis_rings.ron",
            "Desert Oasis Rings",
        ),
        (SANDY_ISLETS_CAMERA_SCRIPT, "Sandy Islets"),
        (WOODED_ISLAND_CAMERA_SCRIPT, "Wooded Island"),
        (OCEAN_ARCHIPELAGOES_CAMERA_SCRIPT, "Ocean Archipelagoes"),
        ("../../walks/camera_fort.ron", "Fort"),
        ("../../walks/camera_crystal_ascent.ron", "Crystal Ascent"),
        (
            "../../walks/camera_crystal_mountain.ron",
            "Crystal Mountain",
        ),
        ("../../walks/camera_seven_regions.ron", "Seven Regions"),
        ("../../walks/camera_two_rings.ron", "Two Rings"),
        ("../../walks/camera_mountain_range.ron", "Mountain Range"),
        (GRAND_V3_BASELINE_CAMERA_SCRIPT, "Grand V3 Baseline"),
    ];

    const TWO_RINGS_ROUTE_SCRIPTS: &[&str] = &[
        "../../walks/camera_two_rings.ron",
        "../../walks/camera_two_rings_mountains.ron",
        "../../walks/camera_two_rings_woodlands.ron",
        "../../walks/camera_two_rings_prairies.ron",
        "../../walks/camera_two_rings_west.ron",
    ];

    /// Sandbox maps reviewed through deployment rather than terrain traversal.
    ///
    /// Flat Arena has no meaningful camera route; the README Sandbox deployment
    /// walk and scoped map-selection frames exercise it through its shipping path.
    const DEPLOYMENT_ONLY_SANDBOX_MAP_IDS: &[&str] = &["flat-arena"];

    #[derive(Resource, Default)]
    struct PointerRecord {
        target: Option<Entity>,
        primary: bool,
        hovered: bool,
    }

    #[derive(Resource, Clone, Copy)]
    struct PointerRequest {
        target: Entity,
        window: Entity,
    }

    fn issue_requested_pointer_click(mut commands: Commands, request: Res<PointerRequest>) {
        if let Some(click) = primary_tile_click(request.target, request.window) {
            commands.trigger(click);
        }
    }

    fn issue_requested_pointer_hover(mut commands: Commands, request: Res<PointerRequest>) {
        if let Some(over) = primary_tile_hover(request.target, request.window) {
            commands.trigger(over);
        }
    }

    fn record_pointer_click(click: On<Pointer<Click>>, mut record: ResMut<PointerRecord>) {
        record.target = Some(click.event_target());
        record.primary = click.button == PointerButton::Primary;
    }

    fn record_pointer_hover(over: On<Pointer<Over>>, mut record: ResMut<PointerRecord>) {
        record.target = Some(over.event_target());
        record.hovered = true;
    }

    #[derive(Resource, Default, Debug, PartialEq, Eq)]
    struct PartyIdleRecord(Option<bool>);

    fn record_party_idle(content: WalkContent, mut record: ResMut<PartyIdleRecord>) {
        record.0 = content.party_is_idle();
    }

    #[derive(Resource, Default, Debug, PartialEq, Eq)]
    struct SelectedMovementRecord(Option<bool>);

    fn record_selected_movement(content: WalkContent, mut record: ResMut<SelectedMovementRecord>) {
        record.0 = content.selected_movement_is_pending();
    }

    #[derive(Resource, Default, Debug)]
    struct SelectedAtRecord(Option<Result<(), String>>);

    fn record_selected_at(content: WalkContent, mut record: ResMut<SelectedAtRecord>) {
        record.0 = Some(content.assert_selected_at(TilePos::ORIGIN));
    }

    const FULL_SCRIPT: &str = r#"[
        AwaitScreen("Title"),
        Settle(30),
        Capture("01-title"),
        Click(name: "Sandbox"),
        AwaitScreen("Sandbox"),
        Key("Backspace"),
        StartScenario(name: "The Crossing"),
        AwaitTerrain,
        ClickTile(q: 2, r: -2),
        ClickTile(q: 2, r: -2, level: Some(7)),
        HoverTile(q: 3, r: -2, level: Some(7)),
        ClickAnchor(name: "bridge", expected: (q: 0, r: 0, level: 16)),
        CaptureWhileMoving(prefix: "bridge-motion", every_frames: 3, capture_count: 4),
        AwaitPartyIdle(max_frames: 600),
        AssertSelectedAt(expected: (q: 0, r: 0, level: 16)),
        OrbitCamera(yaw_turns: 0.33333334, pitch_fraction: -0.1),
        AwaitButton("Cast Ember"),
        SetViewport(width: 3840, height: 2160, device_scale: 1.0),
        SetUiScale(Percent200),
        PresentUi("required-decision"),
        Capture("02-crossing"),
    ]"#;

    #[test]
    fn disposable_storage_root_is_unique_to_one_walk_process() {
        let out = PathBuf::from("captures");
        let first = isolated_storage_root(out.clone(), 42, 100);
        let second = isolated_storage_root(out.clone(), 42, 101);

        assert_eq!(first.parent(), Some(out.as_path()));
        assert_ne!(first, second);
        assert_eq!(first, out.join(".game-data-42-100"));
    }

    #[test]
    fn required_walk_environment_distinguishes_absent_and_present_values() {
        assert_eq!(
            normalize_walk_environment_value(SCRIPT_ENV, Err(env::VarError::NotPresent)),
            Ok(None)
        );
        assert_eq!(
            normalize_walk_environment_value(SCRIPT_ENV, Ok("walk.ron".to_owned())),
            Ok(Some("walk.ron".to_owned()))
        );
    }

    #[cfg(unix)]
    #[test]
    fn required_walk_environment_rejects_non_unicode_values() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let error = normalize_walk_environment_value(
            SCRIPT_ENV,
            Err(env::VarError::NotUnicode(OsString::from_vec(vec![0xff]))),
        )
        .expect_err("non-Unicode automation paths must fail closed");
        assert!(error.contains(SCRIPT_ENV));
        assert!(error.contains("Unicode"));
    }

    #[test]
    fn a_full_script_parses_with_every_step_kind() {
        let steps: Vec<WalkStep> = ron::from_str(FULL_SCRIPT).expect("script parses");
        assert_eq!(steps.len(), 21);
        assert_eq!(steps.first(), Some(&WalkStep::AwaitScreen("Title".into())));
        assert_eq!(
            steps.get(3),
            Some(&WalkStep::Click {
                name: "Sandbox".into(),
                index: 0
            })
        );
        assert_eq!(
            steps.get(6),
            Some(&WalkStep::StartScenario {
                name: "The Crossing".into(),
                seed: None,
                suppress_hostiles: false,
            })
        );
        assert_eq!(
            steps.get(8),
            Some(&WalkStep::ClickTile {
                q: 2,
                r: -2,
                level: None,
            })
        );
        assert_eq!(
            steps.get(9),
            Some(&WalkStep::ClickTile {
                q: 2,
                r: -2,
                level: Some(7),
            })
        );
        assert_eq!(
            steps.get(10),
            Some(&WalkStep::HoverTile {
                q: 3,
                r: -2,
                level: Some(7),
            })
        );
        assert_eq!(
            steps.get(11),
            Some(&WalkStep::ClickAnchor {
                name: "bridge".to_owned(),
                expected: CameraRouteTile {
                    q: 0,
                    r: 0,
                    level: 16,
                },
            })
        );
        assert_eq!(
            steps.get(12),
            Some(&WalkStep::CaptureWhileMoving {
                prefix: "bridge-motion".to_owned(),
                every_frames: 3,
                capture_count: 4,
            })
        );
        assert_eq!(
            steps.get(13),
            Some(&WalkStep::AwaitPartyIdle { max_frames: 600 })
        );
        assert_eq!(
            steps.get(14),
            Some(&WalkStep::AssertSelectedAt {
                expected: CameraRouteTile {
                    q: 0,
                    r: 0,
                    level: 16,
                },
            })
        );
        assert_eq!(
            steps.get(15),
            Some(&WalkStep::OrbitCamera {
                yaw_turns: 0.33333334,
                pitch_fraction: -0.1,
            })
        );
        for step in &steps {
            validate_step(step).expect("every step validates");
        }
        validate_script_steps("full-script.ron", &steps)
            .expect("cross-step capture constraints should validate");
        assert_eq!(
            ReviewRunProvenance::from_steps(
                "full-script.ron".to_owned(),
                "test-run".to_owned(),
                &steps,
            )
            .expected_captures,
            6,
            "two static frames plus four temporal frames must be indexed"
        );
    }

    #[test]
    fn walk_steps_reject_unknown_fields_in_temporal_capture_configuration() {
        let error = ron::from_str::<Vec<WalkStep>>(
            r#"[
                CaptureWhileMoving(
                    prefix: "motion",
                    every_frames: 1,
                    capture_count: 1,
                    capture_counts: 1,
                ),
            ]"#,
        )
        .expect_err("a misspelled temporal-capture field must fail closed");
        assert!(error.to_string().contains("capture_counts"));
    }

    #[test]
    fn map_review_hostile_suppression_preserves_the_authored_player_roster() {
        let authored = || {
            ron::from_str::<Encounter>(include_str!(
                "../../../assets/config/encounters/anchored-skirmish.ron"
            ))
            .expect("the shipped anchored skirmish parses")
        };
        let mut encounter = authored();
        let players = encounter
            .rosters
            .iter()
            .filter(|roster| roster.faction == EncounterFaction::Player)
            .cloned()
            .collect::<Vec<_>>();
        assert!(!players.is_empty());
        assert!(encounter.unit_count(EncounterFaction::Hostile) > 0);

        retain_non_hostile_rosters(&mut encounter);

        assert_eq!(encounter.rosters, players);
        assert_eq!(encounter.unit_count(EncounterFaction::Hostile), 0);
        encounter
            .validate()
            .expect("removing hostiles must leave a valid player-owned review encounter");

        let mut app = App::new();
        app.insert_resource(authored())
            .insert_resource(SuppressHostilesForMapReview)
            .add_systems(Update, suppress_hostiles_for_map_review);
        app.update();
        assert_eq!(
            app.world()
                .resource::<Encounter>()
                .unit_count(EncounterFaction::Hostile),
            0
        );
        assert!(
            !app.world()
                .contains_resource::<SuppressHostilesForMapReview>(),
            "the presentation-only policy must be consumed by exactly one launch"
        );

        app.insert_resource(authored());
        app.update();
        assert!(
            app.world()
                .resource::<Encounter>()
                .unit_count(EncounterFaction::Hostile)
                > 0,
            "a later launch must retain its authored hostile roster"
        );
    }

    #[test]
    fn capture_uses_the_active_fixture_composition_contract() {
        let snapshot = hex_ui::test_support::UiTreeSnapshot {
            metrics: hex_ui::ResolvedUiMetrics {
                viewport: hex_ui::UiViewportClass::Standard,
                logical_size: Vec2::new(1920.0, 1080.0),
                ..default()
            },
            nodes: Vec::new(),
            focus_order: Vec::new(),
            action_priority: None,
        };
        assert!(capture_structural_issues(&snapshot, None, None, None).is_empty());
        let issues = capture_structural_issues(
            &snapshot,
            Some("sandbox-outcome"),
            Some(hex_ui::test_support::UiTaskCase::SandboxOutcome),
            Some(Screen::Gameplay),
        );
        assert!(
            issues.iter().any(|issue| issue.contains("Retry Exact")),
            "the capture path must reject an incomplete Sandbox outcome: {issues:?}"
        );
        let issues = capture_structural_issues(&snapshot, None, None, Some(Screen::Sandbox));
        assert!(
            issues
                .iter()
                .any(|issue| issue.contains("stale or incomplete")),
            "a stale screen snapshot must not authorize a capture: {issues:?}"
        );
        let issues = capture_structural_issues(
            &snapshot,
            Some("aiming-disabled"),
            Some(hex_ui::test_support::UiTaskCase::AimingBlocked),
            Some(Screen::Gameplay),
        );
        assert!(
            issues.iter().any(|issue| issue.contains("Cancel Aim")),
            "a named review capture must reject the right screen with the wrong task contents: {issues:?}"
        );
        for task in [
            hex_ui::test_support::UiTaskCase::CharacterMainView,
            hex_ui::test_support::UiTaskCase::ActivityTabs,
            hex_ui::test_support::UiTaskCase::CustomHudVisibility,
            hex_ui::test_support::UiTaskCase::CompactTemporarySurface,
        ] {
            let issues = capture_structural_issues(
                &snapshot,
                Some("normal-gameplay"),
                Some(task),
                Some(Screen::Gameplay),
            );
            assert!(
                issues
                    .iter()
                    .all(|issue| !issue.contains("cannot satisfy task")),
                "normal gameplay fixture must be compatible with {}: {issues:?}",
                task.contract().id
            );
        }
    }

    #[test]
    fn unknown_screens_and_keys_are_rejected_at_load() {
        assert_eq!(parse_key("C"), Ok(KeyCode::KeyC));
        assert_eq!(parse_key("H"), Ok(KeyCode::KeyH));
        assert_eq!(parse_key("P"), Ok(KeyCode::KeyP));
        assert_eq!(parse_key("I"), Ok(KeyCode::KeyI));
        assert_eq!(parse_key("L"), Ok(KeyCode::KeyL));
        assert_eq!(parse_key("B"), Ok(KeyCode::KeyB));
        assert_eq!(parse_key("V"), Ok(KeyCode::KeyV));
        assert_eq!(parse_key("F"), Ok(KeyCode::KeyF));
        assert_eq!(parse_key("Escape"), Ok(KeyCode::Escape));
        assert!(validate_step(&WalkStep::AwaitScreen("Menu".into())).is_err());
        assert!(validate_step(&WalkStep::AwaitGameplay { max_seconds: 0 }).is_err());
        assert!(validate_step(&WalkStep::AwaitGameplay { max_seconds: 301 }).is_err());
        assert_eq!(
            step_timeout(&WalkStep::AwaitGameplay { max_seconds: 180 }),
            Duration::from_secs(180)
        );
        assert!(validate_step(&WalkStep::Key("F13".into())).is_err());
        assert!(validate_step(&WalkStep::Capture(" ".into())).is_err());
        assert!(validate_step(&WalkStep::Capture("../overwrite".into())).is_err());
        assert!(validate_step(&WalkStep::Capture("review frame".into())).is_err());
        assert!(validate_step(&WalkStep::Capture("01-Case-Collision".into())).is_err());
        validate_step(&WalkStep::Capture("01-safe_review-frame".into()))
            .expect("a stable slug is a safe capture name");
        assert!(validate_step(&WalkStep::ReviewCapture {
            name: " ".into(),
            task: hex_ui::test_support::UiTaskCase::MainMenu,
        })
        .is_err());
        assert!(validate_step(&WalkStep::Click {
            name: String::new(),
            index: 0
        })
        .is_err());
        assert!(validate_step(&WalkStep::AwaitButton(" ".into())).is_err());
        assert!(validate_step(&WalkStep::AwaitPartyIdle { max_frames: 0 }).is_err());
        validate_step(&WalkStep::AwaitPartyIdle { max_frames: 1 })
            .expect("a positive frame bound is valid");
        assert!(validate_step(&WalkStep::CaptureWhileMoving {
            prefix: "motion".to_owned(),
            every_frames: 0,
            capture_count: 1,
        })
        .is_err());
        assert!(validate_step(&WalkStep::CaptureWhileMoving {
            prefix: "motion".to_owned(),
            every_frames: 1,
            capture_count: 0,
        })
        .is_err());
        assert!(validate_step(&WalkStep::CaptureWhileMoving {
            prefix: "../motion".to_owned(),
            every_frames: 1,
            capture_count: 1,
        })
        .is_err());
        assert!(validate_step(&WalkStep::CaptureWhileMoving {
            prefix: "motion".to_owned(),
            every_frames: 1,
            capture_count: MAX_MOVEMENT_CAPTURE_FILES + 1,
        })
        .is_err());
        assert!(validate_step(&WalkStep::CaptureWhileMoving {
            prefix: "motion".to_owned(),
            every_frames: MAX_MOVEMENT_CAPTURE_FRAMES,
            capture_count: 2,
        })
        .is_err());
        validate_step(&WalkStep::CaptureWhileMoving {
            prefix: "motion".to_owned(),
            every_frames: MAX_MOVEMENT_CAPTURE_FRAMES / u32::from(MAX_MOVEMENT_CAPTURE_FILES),
            capture_count: MAX_MOVEMENT_CAPTURE_FILES,
        })
        .expect("the exact bounded temporal capture limit is valid");
        assert!(validate_step(&WalkStep::ClickAnchor {
            name: " ".to_owned(),
            expected: CameraRouteTile {
                q: 0,
                r: 0,
                level: 1,
            },
        })
        .is_err());
        validate_step(&WalkStep::AssertCameraMode(WalkCameraMode::FirstPerson))
            .expect("every typed camera mode should be a valid assertion");
    }

    #[test]
    fn movement_sequence_requires_click_anchor_and_owns_unique_files() {
        let expected = CameraRouteTile {
            q: 0,
            r: 0,
            level: 1,
        };
        let recorder = WalkStep::CaptureWhileMoving {
            prefix: "route-motion".to_owned(),
            every_frames: 3,
            capture_count: 4,
        };
        let valid = vec![
            WalkStep::ClickAnchor {
                name: "route".to_owned(),
                expected,
            },
            recorder.clone(),
            WalkStep::AwaitPartyIdle { max_frames: 600 },
            WalkStep::AssertSelectedAt { expected },
        ];
        validate_script_steps("valid.ron", &valid)
            .expect("an immediately stale-checked movement recorder should validate");

        let misplaced = vec![WalkStep::Settle(1), recorder.clone()];
        let error = validate_script_steps("misplaced.ron", &misplaced)
            .expect_err("a temporal recorder without ClickAnchor must fail closed");
        assert!(error.contains("immediately follow ClickAnchor"));

        let collision = vec![
            WalkStep::ClickAnchor {
                name: "route".to_owned(),
                expected,
            },
            recorder.clone(),
            WalkStep::AwaitPartyIdle { max_frames: 600 },
            WalkStep::AssertSelectedAt { expected },
            WalkStep::Capture("route-motion-0002".to_owned()),
        ];
        let error = validate_script_steps("collision.ron", &collision)
            .expect_err("generated temporal names must not overwrite static evidence");
        assert!(error.contains("duplicate capture name"));

        let missing_arrival = vec![
            WalkStep::ClickAnchor {
                name: "route".to_owned(),
                expected,
            },
            recorder.clone(),
            WalkStep::AwaitPartyIdle { max_frames: 600 },
        ];
        let error = validate_script_steps("missing-arrival.ron", &missing_arrival)
            .expect_err("a temporal sequence without an arrival proof must fail closed");
        assert!(error.contains("matching AssertSelectedAt"));

        let wrong_destination = CameraRouteTile {
            q: 1,
            r: 0,
            level: 1,
        };
        let mismatched_arrival = vec![
            WalkStep::ClickAnchor {
                name: "route".to_owned(),
                expected,
            },
            recorder,
            WalkStep::AwaitPartyIdle { max_frames: 600 },
            WalkStep::AssertSelectedAt {
                expected: wrong_destination,
            },
        ];
        let error = validate_script_steps("mismatched-arrival.ron", &mismatched_arrival)
            .expect_err("a temporal sequence must prove its own clicked destination");
        assert!(error.contains("does not match ClickAnchor destination"));

        let mut excessive = Vec::new();
        for sequence in
            0..=MAX_MOVEMENT_CAPTURE_FILES_PER_WALK / usize::from(MAX_MOVEMENT_CAPTURE_FILES)
        {
            excessive.push(WalkStep::ClickAnchor {
                name: format!("route-{sequence}"),
                expected,
            });
            excessive.push(WalkStep::CaptureWhileMoving {
                prefix: format!("route-{sequence}-motion"),
                every_frames: 1,
                capture_count: MAX_MOVEMENT_CAPTURE_FILES,
            });
            excessive.push(WalkStep::AwaitPartyIdle { max_frames: 600 });
            excessive.push(WalkStep::AssertSelectedAt { expected });
        }
        let error = validate_script_steps("too-many-frames.ron", &excessive)
            .expect_err("one walk must not expand temporal sequences without bound");
        assert!(error.contains("per-walk limit"));
    }

    #[test]
    fn movement_sequence_schedules_exact_fixed_intervals_and_fails_early_idle() {
        let mut recording = MovementCaptureState::new("motion".to_owned(), 3, 4);
        let mut scheduled = Vec::new();
        for frame in 1..=12 {
            let tick = recording
                .observe_movement_frame(true)
                .expect("pending movement should remain recordable");
            if let MovementCaptureTick::Capture(index) = tick {
                scheduled.push((frame, index));
            }
        }
        assert_eq!(scheduled, [(3, 0), (6, 1), (9, 2), (12, 3)]);
        assert!(recording.scheduled_all());

        let mut ended = MovementCaptureState::new("short".to_owned(), 2, 3);
        assert_eq!(
            ended.observe_movement_frame(true),
            Ok(MovementCaptureTick::WaitingForInterval)
        );
        let error = ended
            .observe_movement_frame(false)
            .expect_err("movement ending before the exact sequence must fail");
        assert!(error.contains("movement ended after 0 of 3"));

        let mut never_started = MovementCaptureState::new("ignored".to_owned(), 1, 1);
        for _ in 0..MOVEMENT_START_GRACE_FRAMES {
            assert_eq!(
                never_started.observe_movement_frame(false),
                Ok(MovementCaptureTick::WaitingForMovement)
            );
        }
        assert!(never_started.observe_movement_frame(false).is_err());
    }

    #[test]
    fn temporal_callback_gate_rejects_late_unrequested_and_duplicate_frames() {
        let mut recording = MovementCaptureState::new("motion".to_owned(), 1, 2);
        assert_eq!(
            recording.observe_movement_frame(true),
            Ok(MovementCaptureTick::Capture(0))
        );
        assert_eq!(recording.callback_issue("motion", 0), None);
        assert!(recording
            .callback_issue("other-motion", 0)
            .is_some_and(|reason| reason.contains("arrived during")));
        assert!(recording
            .callback_issue("motion", 1)
            .is_some_and(|reason| reason.contains("before its request")));

        recording.outcomes.insert(
            0,
            CaptureOutcome::Written {
                brightest: 255,
                coverage: true,
            },
        );
        assert!(recording
            .callback_issue("motion", 0)
            .is_some_and(|reason| reason.contains("more than once")));
        recording.aborted = true;
        assert!(recording
            .callback_issue("motion", 1)
            .is_some_and(|reason| reason.contains("after abort")));
    }

    fn review_index_test_directory(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        std::env::temp_dir().join(format!(
            "hex-walk-review-index-{label}-{}-{nonce}",
            std::process::id()
        ))
    }

    #[test]
    fn aborted_movement_sequence_cleans_only_its_exact_outputs() {
        let directory = review_index_test_directory("motion-cleanup");
        let _cleanup = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("capture directory should be creatable");
        for index in 0..2 {
            let path = movement_capture_path(&directory, "route", index);
            std::fs::write(&path, b"stale frame").expect("stale frame fixture should write");
            let temporary =
                temporary_capture_path(&path).expect("temporary capture path should resolve");
            std::fs::write(&temporary, b"partial frame")
                .expect("partial frame fixture should write");
        }
        let unrelated = directory.join("route-not-this-sequence.png");
        std::fs::write(&unrelated, b"keep").expect("unrelated fixture should write");

        cleanup_movement_capture_outputs(&directory, "route", 2)
            .expect("exact movement outputs should be removable");

        for index in 0..2 {
            let path = movement_capture_path(&directory, "route", index);
            assert!(!path.exists());
            assert!(!temporary_capture_path(&path)
                .expect("temporary capture path should resolve")
                .exists());
        }
        assert!(
            unrelated.exists(),
            "cleanup must not use a broad prefix glob"
        );
        let _cleanup = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn movement_cleanup_attempts_every_exact_output_after_one_removal_error() {
        let directory = review_index_test_directory("motion-cleanup-error");
        let _cleanup = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("capture directory should be creatable");

        let invalid_file = movement_capture_path(&directory, "route", 0);
        std::fs::create_dir(&invalid_file).expect("directory fixture should block remove_file");
        let later_file = movement_capture_path(&directory, "route", 1);
        std::fs::write(&later_file, b"stale frame").expect("later frame fixture should write");
        let later_temporary =
            temporary_capture_path(&later_file).expect("temporary capture path should resolve");
        std::fs::write(&later_temporary, b"partial frame")
            .expect("later partial frame fixture should write");

        let error = cleanup_movement_capture_outputs(&directory, "route", 2)
            .expect_err("a directory at an exact file path must be reported");
        assert!(error.contains(&invalid_file.display().to_string()));
        assert!(invalid_file.is_dir());
        assert!(
            !later_file.exists() && !later_temporary.exists(),
            "one removal error must not leave later exact outputs behind"
        );

        let _cleanup = std::fs::remove_dir_all(directory);
    }

    fn review_provenance(expected_captures: usize) -> ReviewRunProvenance {
        ReviewRunProvenance {
            run_id: "test-run-42".to_owned(),
            script_path: "walks/camera_test.ron".to_owned(),
            expected_captures,
            planned_scenarios: vec![ReviewScenarioProvenance {
                name: "Test Scenario".to_owned(),
                seed: Some(77),
            }],
        }
    }

    #[test]
    fn visual_walk_uses_fixed_sixty_hz_time_and_step_aware_idle_watchdogs() {
        assert!(matches!(
            walk_time_update_strategy(),
            bevy::time::TimeUpdateStrategy::ManualDuration(duration)
                if duration == WALK_FRAME_DURATION
        ));
        assert_eq!(step_timeout(&WalkStep::Settle(1)), STEP_TIMEOUT);

        let idle = WalkStep::AwaitPartyIdle { max_frames: 18_000 };
        assert_eq!(
            step_timeout(&idle),
            STEP_TIMEOUT
                + WALK_FRAME_DURATION
                    .checked_mul(18_000)
                    .expect("the authored frame budget fits Duration")
        );
        assert!(step_timeout(&idle) > Duration::from_secs(300));
    }

    #[test]
    fn aborting_temporal_capture_restores_speed_and_removes_partial_evidence() {
        let directory = review_index_test_directory("motion-abort-state");
        let _cleanup = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("capture directory should be creatable");
        let partial = movement_capture_path(&directory, "route", 0);
        std::fs::write(&partial, b"partial frame").expect("partial frame fixture should write");

        let mut state = WalkState::new(
            Vec::new(),
            directory.clone(),
            hex_ui::ReviewViewport::DEFAULT,
            false,
            review_provenance(1),
        );
        state.movement_capture = Some(MovementCaptureState::new("route".to_owned(), 1, 1));
        let mut time = Time::<Virtual>::default();
        begin_temporal_capture_time(&mut time);
        assert_eq!(
            time.relative_speed().to_bits(),
            TEMPORAL_CAPTURE_TIME_SCALE.to_bits()
        );

        let reason = abort_movement_capture(&mut state, &mut time, "fixture failure".to_owned());
        assert_eq!(reason, "fixture failure");
        assert_eq!(time.relative_speed().to_bits(), WALK_TIME_SCALE.to_bits());
        assert!(state.movement_capture.is_none());
        assert!(!partial.exists());

        let _cleanup = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn temporal_frames_remain_fail_closed_until_the_exact_arrival_proof() {
        let directory = review_index_test_directory("motion-arrival-proof");
        let _cleanup = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("capture directory should be creatable");
        for index in 0..2 {
            std::fs::write(
                movement_capture_path(&directory, "route", index),
                b"completed frame awaiting route proof",
            )
            .expect("temporal fixture should write");
        }

        let mut state = WalkState::new(
            Vec::new(),
            directory.clone(),
            hex_ui::ReviewViewport::DEFAULT,
            false,
            review_provenance(2),
        );
        state.pending_movement_capture_proof = Some(PendingMovementCaptureProof {
            prefix: "route".to_owned(),
            capture_count: 2,
        });
        let mut time = Time::<Virtual>::default();

        let reason = abort_movement_capture(
            &mut state,
            &mut time,
            "selected actor arrived on the wrong surface".to_owned(),
        );
        assert_eq!(reason, "selected actor arrived on the wrong surface");
        assert!(state.pending_movement_capture_proof.is_none());
        assert!(state.completed_captures.is_empty());
        for index in 0..2 {
            assert!(!movement_capture_path(&directory, "route", index).exists());
        }

        let _cleanup = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn generated_review_index_exposes_every_capture_and_explicit_classification() {
        let captures = vec![
            "01-overview".to_owned(),
            "02-seam-stress".to_owned(),
            "03-emissive-closeup".to_owned(),
        ];
        let review = review_provenance(captures.len());
        let launched = vec![ReviewScenarioProvenance {
            name: "Test Scenario".to_owned(),
            seed: Some(91),
        }];
        let markdown = completed_review_index_markdown(&review, &launched, &captures)
            .expect("a complete capture list should render its review index");

        assert!(markdown.contains("Capture status: COMPLETE"));
        assert!(markdown.contains("Human review status: UNREVIEWED"));
        assert!(markdown.contains("native control-feel"));
        assert!(markdown.contains("walks/camera_test.ron"));
        assert!(markdown.contains("Test Scenario"));
        assert!(markdown.contains("seed <code>91</code>"));
        assert!(markdown.contains("3 of 3 expected"));
        for name in &captures {
            assert!(markdown.contains(&format!("### `{name}`")));
            assert!(markdown.contains(&format!("](<./{name}.png>)")));
        }
        assert_eq!(
            markdown.matches("- Result: **UNREVIEWED**").count(),
            captures.len()
        );
        assert_eq!(markdown.matches("- Notes:").count(), captures.len());
        assert_eq!(markdown.matches(".png>)").count(), captures.len());
        assert!(!markdown.contains("- [ ]"));
    }

    #[test]
    fn walk_start_replaces_a_stale_checked_index_with_incomplete_marker() {
        let directory = review_index_test_directory("stale");
        let _cleanup = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("review test directory should be creatable");
        let path = directory.join(REVIEW_INDEX_FILE);
        std::fs::write(&path, "# Old review\n\n- [x] PASS\n")
            .expect("stale checked index should be writable");

        write_starting_review_index(&directory, "walks/camera_test.ron", "startup-run-43")
            .expect("walk startup should invalidate stale review evidence");
        let markdown = std::fs::read_to_string(&path)
            .expect("the incomplete review marker should remain readable");
        assert!(markdown.contains("INCOMPLETE — NOT REVIEWABLE"));
        assert!(markdown.contains("Expected captures: unknown until script validation completes"));
        assert!(markdown.contains("walks/camera_test.ron"));
        assert!(!markdown.contains("[x] PASS"));
        assert!(!temporary_capture_path(&path)
            .expect("review index should have a valid staging path")
            .exists());

        let _cleanup = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn completed_index_is_installed_only_for_the_exact_capture_count() {
        let directory = review_index_test_directory("complete");
        let _cleanup = std::fs::remove_dir_all(&directory);
        let review = review_provenance(2);
        let path = write_incomplete_review_index(&directory, &review)
            .expect("walk startup should publish an incomplete marker");
        let incomplete = std::fs::read_to_string(&path)
            .expect("the incomplete review marker should be readable");

        let error = write_completed_review_index(
            &directory,
            &review,
            &[ReviewScenarioProvenance {
                name: "Test Scenario".to_owned(),
                seed: Some(77),
            }],
            &["01-overview".to_owned()],
        )
        .expect_err("a partial capture pack must not replace its incomplete marker");
        assert!(error.contains("1 persisted captures, expected 2"));
        assert_eq!(
            std::fs::read_to_string(&path)
                .expect("the rejected completion should leave its marker readable"),
            incomplete
        );

        write_completed_review_index(
            &directory,
            &review,
            &[ReviewScenarioProvenance {
                name: "Test Scenario".to_owned(),
                seed: Some(77),
            }],
            &["01-overview".to_owned(), "02-detail".to_owned()],
        )
        .expect("an exact capture pack should atomically replace its marker");
        let complete =
            std::fs::read_to_string(&path).expect("the completed review index should be readable");
        assert!(complete.contains("Capture status: COMPLETE"));
        assert!(complete.contains("2 of 2 expected"));
        assert!(!complete.contains("INCOMPLETE — NOT REVIEWABLE"));
        assert!(!temporary_capture_path(&path)
            .expect("review index should have a valid staging path")
            .exists());

        let _cleanup = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn first_person_walk_proves_the_three_state_cycle_and_tactical_controls() {
        let steps: Vec<WalkStep> =
            ron::from_str(include_str!("../../../walks/camera_first_person.ron"))
                .expect("the focused first-person walk should parse");
        for step in &steps {
            validate_step(step).expect("the focused first-person walk should validate");
        }

        let asserted_modes = steps
            .iter()
            .filter_map(|step| match step {
                WalkStep::AssertCameraMode(mode) => Some(*mode),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            asserted_modes,
            vec![
                WalkCameraMode::Map,
                WalkCameraMode::Character,
                WalkCameraMode::FirstPerson,
                WalkCameraMode::Map,
            ],
            "the walk must prove Map -> Character -> First Person -> Map in order"
        );
        let cycles_to = |expected| {
            steps.windows(2).any(|pair| {
                matches!(
                    pair,
                    [WalkStep::Key(key), WalkStep::AssertCameraMode(actual)]
                        if key == "C" && *actual == expected
                )
            })
        };
        assert!(cycles_to(WalkCameraMode::Character));
        assert!(cycles_to(WalkCameraMode::FirstPerson));
        assert!(cycles_to(WalkCameraMode::Map));
        assert_eq!(
            steps
                .iter()
                .filter(|step| matches!(step, WalkStep::Capture(_)))
                .count(),
            4,
            "the walk should retain before-move, after-move, look, and restored-Map evidence"
        );
        assert!(
            steps
                .iter()
                .any(|step| matches!(step, WalkStep::OrbitCamera { .. })),
            "right-drag look must be exercised through the ordinary input adapter"
        );
        assert!(
            steps
                .iter()
                .any(|step| matches!(step, WalkStep::AssertSelectedAt { .. })),
            "click-to-move must be proved at an exact stack-safe destination"
        );
    }

    #[test]
    fn orbit_gesture_is_finite_bounded_and_converts_to_an_ordinary_drag() {
        let size = Vec2::new(1_200.0, 800.0);
        let (baseline, destination) = orbit_cursor_positions(size, None, 1.0 / 3.0, 0.5)
            .expect("a bounded multi-azimuth gesture should resolve");
        assert_eq!(baseline, Vec2::new(600.0, 400.0));
        assert!((destination.x - 200.0).abs() < 1e-4);
        assert!((destination.y - 600.0).abs() < 1e-4);

        let authored_cursor = Vec2::new(175.0, 90.0);
        let (baseline, destination) =
            orbit_cursor_positions(size, Some(authored_cursor), -0.25, -1.0)
                .expect("the real cursor should become the ordinary drag baseline");
        assert_eq!(baseline, authored_cursor);
        assert_eq!(destination, Vec2::new(475.0, -310.0));

        for (yaw, pitch) in [
            (0.0, 0.0),
            (0.500_1, 0.0),
            (0.0, 1.001),
            (f32::NAN, 0.0),
            (0.0, f32::INFINITY),
        ] {
            assert!(
                orbit_cursor_positions(size, None, yaw, pitch).is_err(),
                "{yaw:?}/{pitch:?} should be rejected"
            );
        }
        assert!(orbit_cursor_positions(Vec2::ZERO, None, 0.25, 0.0).is_err());
    }

    #[test]
    fn every_capture_owns_a_fresh_settled_render_target_generation() {
        let steps = vec![
            WalkStep::Capture("first".to_owned()),
            WalkStep::Capture("second".to_owned()),
        ];
        let review = ReviewRunProvenance::from_steps(
            "test-camera-walk.ron".to_owned(),
            "test-render-target-run".to_owned(),
            &steps,
        );
        let mut state = WalkState::new(
            steps,
            PathBuf::from("captures"),
            hex_ui::ReviewViewport::DEFAULT,
            false,
            review,
        );
        let mut images = Assets::<Image>::default();

        let initial = images.add(Image::default());
        let initial_id = initial.id();
        install_walk_target(&mut state, &mut images, initial).expect("the initial target installs");
        assert_eq!(state.target_generation, 1);
        assert!(!prepare_capture_target(&mut state).expect("refresh starts"));
        assert!(state.target.is_none());
        assert!(
            images.get(initial_id).is_some(),
            "the old asset must stay allocated until the replacement gets a distinct ID"
        );

        let first = images.add(Image::default());
        assert_ne!(first.id(), initial_id);
        install_walk_target(&mut state, &mut images, first)
            .expect("the first capture target installs");
        assert!(
            images.get(initial_id).is_none(),
            "the retired image must be removed after replacement"
        );
        for _ in 0..CAPTURE_TARGET_SETTLE_FRAMES {
            assert!(
                !prepare_capture_target(&mut state).expect("settling succeeds"),
                "a fresh target must render before capture"
            );
        }
        assert!(prepare_capture_target(&mut state).expect("first target is ready"));
        let first_generation = state.target_generation;

        state.advance();
        assert!(!prepare_capture_target(&mut state).expect("next refresh starts"));
        let second = images.add(Image::default());
        install_walk_target(&mut state, &mut images, second)
            .expect("the second capture target installs");
        assert_eq!(state.target_generation, first_generation + 1);
        for _ in 0..CAPTURE_TARGET_SETTLE_FRAMES {
            assert!(!prepare_capture_target(&mut state).expect("settling succeeds"));
        }
        assert!(prepare_capture_target(&mut state).expect("second target is ready"));
    }

    #[test]
    fn shared_target_ui_sampling_tracks_oit_and_restores_change_driven() {
        assert_eq!(
            shared_target_msaa_update(Msaa::Off, Msaa::Sample4),
            Some(Msaa::Off)
        );
        assert_eq!(
            shared_target_msaa_update(Msaa::Off, Msaa::Off),
            None,
            "a stable OIT frame must not republish the sampling component"
        );
        assert_eq!(
            shared_target_msaa_update(Msaa::Sample4, Msaa::Off),
            Some(Msaa::Sample4)
        );
    }

    #[test]
    fn camera_route_manifest_is_seed_exact_for_traversed_sandbox_maps() {
        let catalog: SandboxMapCatalog =
            ron::from_str(include_str!("../../../assets/config/sandbox_maps.ron"))
                .expect("the shipped Sandbox map catalog parses");
        let manifest: CameraRouteManifest =
            ron::from_str(include_str!("../../../walks/camera_routes.ron"))
                .expect("the camera route manifest parses");
        assert_eq!(manifest.schema_version, 1);

        let maps = catalog
            .maps
            .iter()
            .filter(|map| !DEPLOYMENT_ONLY_SANDBOX_MAP_IDS.contains(&map.id.as_str()))
            .map(|map| (map.scenario.as_str(), map.fixed_seed))
            .collect::<std::collections::BTreeMap<_, _>>();
        let routes = manifest
            .routes
            .iter()
            .map(|route| (route.scenario.as_str(), route.seed))
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(manifest.routes.len(), maps.len());
        assert_eq!(routes.len(), manifest.routes.len(), "route names repeat");
        assert_eq!(
            routes, maps,
            "traversed Sandbox maps and camera routes must be a seed-exact bijection"
        );
        assert_eq!(
            catalog.maps.len(),
            routes.len() + DEPLOYMENT_ONLY_SANDBOX_MAP_IDS.len(),
            "every Sandbox map needs either camera traversal or explicit deployment-only review"
        );
        for id in DEPLOYMENT_ONLY_SANDBOX_MAP_IDS {
            assert!(
                catalog.get(id).is_some(),
                "deployment-only Sandbox map {id:?} must remain in the shipping catalog"
            );
        }
        assert_eq!(routes.len(), 26);

        for route in &manifest.routes {
            assert!(
                !route.points.is_empty(),
                "{} has no review point",
                route.scenario
            );
            let labels = route
                .points
                .iter()
                .map(|point| point.label.as_str())
                .collect::<std::collections::BTreeSet<_>>();
            assert_eq!(
                labels.len(),
                route.points.len(),
                "{} repeats a point label",
                route.scenario
            );
            for point in &route.points {
                assert!(!point.label.trim().is_empty());
                assert!(!point.azimuth_turns.is_empty());
                for &azimuth in &point.azimuth_turns {
                    assert!(azimuth.is_finite());
                    assert!(azimuth.abs() <= MAX_ORBIT_YAW_TURNS);
                }
                match &point.destination {
                    CameraRouteDestination::Anchor { name, expected } => {
                        assert!(!name.trim().is_empty());
                        let _ = expected.position();
                    }
                    CameraRouteDestination::Exact(position) => {
                        let _ = position.position();
                    }
                }
            }
        }

        let sky = manifest
            .routes
            .iter()
            .find(|route| route.scenario == "Sky Islands")
            .expect("Sky Islands has a route");
        assert!(sky.points.iter().all(|point| {
            matches!(
                &point.destination,
                CameraRouteDestination::Anchor { name, .. } if name == "bridge"
            )
        }));
    }

    #[test]
    fn crystal_ascent_walk_proves_the_vertical_route_and_both_close_cameras() {
        let steps: Vec<WalkStep> =
            ron::from_str(include_str!("../../../walks/camera_crystal_ascent.ron"))
                .expect("the Crystal Ascent camera walk should parse");
        for step in &steps {
            validate_step(step).expect("the Crystal Ascent camera walk should validate");
        }

        assert!(steps.contains(&WalkStep::StartScenario {
            name: "Crystal Ascent".to_owned(),
            seed: Some(1_592_598_566),
            suppress_hostiles: false,
        }));
        assert!(steps.windows(4).any(|steps| matches!(
            steps,
            [
                WalkStep::Key(open),
                WalkStep::AwaitButton(name),
                WalkStep::Click {
                    name: clicked,
                    index: 0
                },
                WalkStep::Key(key),
            ] if open == "F"
                && name == "Party Movement Mode"
                && clicked == "Party Movement Mode"
                && key == "Escape"
        )));
        let clicks = steps
            .iter()
            .filter_map(|step| match step {
                WalkStep::ClickAnchor { name, expected } => Some((name.as_str(), *expected)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            clicks,
            vec![
                (
                    "crystal_ascent.bottom_chamber",
                    CameraRouteTile {
                        q: -8,
                        r: -8,
                        level: 6,
                    },
                ),
                (
                    "crystal_ascent.mid_flight",
                    CameraRouteTile {
                        q: 22,
                        r: 0,
                        level: 74,
                    },
                ),
                (
                    "crystal_ascent.corner_landing",
                    CameraRouteTile {
                        q: -10,
                        r: 21,
                        level: 134,
                    },
                ),
                (
                    "crystal_ascent.upper_contraction",
                    CameraRouteTile {
                        q: -19,
                        r: 19,
                        level: 138,
                    },
                ),
                (
                    "crystal_ascent.upper_exit",
                    CameraRouteTile {
                        q: 16,
                        r: 15,
                        level: 150,
                    },
                ),
            ]
        );
        assert!(steps.contains(&WalkStep::AssertSelectedAt {
            expected: CameraRouteTile {
                q: -17,
                r: -15,
                level: 6,
            },
        }));
        assert!(!steps
            .iter()
            .any(|step| matches!(step, WalkStep::ClickTile { .. })));
        assert!(steps.windows(2).all(|pair| {
            !matches!(pair.first(), Some(WalkStep::ClickAnchor { .. }))
                || matches!(pair.get(1), Some(WalkStep::Settle(frames)) if *frames >= 5)
        }));
        let captures = steps
            .iter()
            .filter_map(|step| match step {
                WalkStep::Capture(name) => Some(name.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            captures,
            vec![
                "01-crystal-ascent-lower-exterior-entrance-character",
                "02-crystal-ascent-bottom-chamber-heart-character",
                "03-crystal-ascent-bottom-chamber-heart-first-person",
                "04-crystal-ascent-mid-flight-first-person",
                "05-crystal-ascent-mid-flight-character",
                "06-crystal-ascent-corner-landing-character",
                "07-crystal-ascent-corner-landing-first-person",
                "08-crystal-ascent-upper-contraction-first-person",
                "09-crystal-ascent-upper-contraction-character",
                "10-crystal-ascent-summit-oculus-clearing-character",
                "11-crystal-ascent-summit-clearing-first-person",
            ]
        );
        assert_eq!(
            steps
                .iter()
                .filter(|step| matches!(step, WalkStep::OrbitCamera { .. }))
                .count(),
            8
        );
        assert!(steps.windows(2).any(|pair| matches!(
            pair,
            [WalkStep::Key(key), WalkStep::AssertCameraMode(WalkCameraMode::Character)]
                if key == "C"
        )));
        assert!(steps.windows(2).any(|pair| matches!(
            pair,
            [WalkStep::Key(key), WalkStep::AssertCameraMode(WalkCameraMode::FirstPerson)]
                if key == "C"
        )));
        assert!(steps.ends_with(&[
            WalkStep::Key("Backspace".to_owned()),
            WalkStep::AwaitScreen("Title".to_owned()),
        ]));
        assert!(CAMERA_ROUTE_SCRIPTS.contains(&(CRYSTAL_ASCENT_CAMERA_SCRIPT, "Crystal Ascent")));
    }

    #[test]
    fn crystal_mountain_walk_proves_the_spanning_route_and_all_camera_modes() {
        let steps: Vec<WalkStep> =
            ron::from_str(include_str!("../../../walks/camera_crystal_mountain.ron"))
                .expect("the Crystal Mountain camera walk should parse");
        for step in &steps {
            validate_step(step).expect("the Crystal Mountain camera walk should validate");
        }

        assert!(steps.contains(&WalkStep::StartScenario {
            name: "Crystal Mountain".to_owned(),
            seed: Some(1_592_598_566),
            suppress_hostiles: false,
        }));
        let anchor_clicks = steps
            .iter()
            .filter_map(|step| match step {
                WalkStep::ClickAnchor { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            anchor_clicks,
            vec![
                "crystal_mountain.tunnel_mouth",
                "crystal_mountain.midpoint",
                "crystal_mountain.gothic_transition",
                "crystal_mountain.ascent_threshold",
                "crystal_ascent.bottom_chamber",
                "crystal_ascent.mid_flight",
                "crystal_mountain.summit_exit",
                "crystal_mountain.basin_clearing",
            ]
        );
        assert!(steps
            .windows(3)
            .filter(|window| { matches!(window.first(), Some(WalkStep::ClickAnchor { .. })) })
            .all(|window| matches!(
                window,
                [
                    WalkStep::ClickAnchor { .. },
                    WalkStep::Settle(5),
                    WalkStep::AwaitPartyIdle { .. },
                ]
            )));
        let captures = steps
            .iter()
            .filter_map(|step| match step {
                WalkStep::Capture(name) => Some(name.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            captures,
            vec![
                "01-crystal-mountain-opaque-massif-map",
                "02-crystal-mountain-rear-ridge-basin-map",
                "03-crystal-mountain-foot-portal-map",
                "04-crystal-mountain-foot-portal-character",
                "05-crystal-mountain-foot-portal-first-person",
                "06-crystal-mountain-natural-tunnel-map",
                "07-crystal-mountain-natural-tunnel-character",
                "08-crystal-mountain-natural-tunnel-first-person",
                "09-crystal-mountain-gothic-transition-map",
                "10-crystal-mountain-gothic-transition-character",
                "11-crystal-mountain-gothic-transition-first-person",
                "12-crystal-mountain-ascent-base-plinth-character",
                "13-crystal-mountain-ascent-base-plinth-first-person",
                "14-crystal-mountain-crystal-chamber-map",
                "15-crystal-mountain-crystal-chamber-character",
                "16-crystal-mountain-crystal-chamber-first-person",
                "17-crystal-mountain-mid-ascent-map",
                "18-crystal-mountain-mid-ascent-character",
                "19-crystal-mountain-mid-ascent-first-person",
                "20-crystal-mountain-summit-exit-map",
                "21-crystal-mountain-summit-exit-character",
                "22-crystal-mountain-summit-exit-first-person",
                "23-crystal-mountain-wooded-basin-map",
                "24-crystal-mountain-wooded-basin-character",
                "25-crystal-mountain-wooded-basin-first-person",
            ]
        );
        for camera in [
            WalkCameraMode::Map,
            WalkCameraMode::Character,
            WalkCameraMode::FirstPerson,
        ] {
            assert!(
                steps.contains(&WalkStep::AssertCameraMode(camera)),
                "Crystal Mountain must capture {camera:?} evidence"
            );
        }
        assert!(steps.ends_with(&[
            WalkStep::Settle(5),
            WalkStep::Key("Backspace".to_owned()),
            WalkStep::AwaitScreen("Title".to_owned()),
        ]));
        assert!(
            CAMERA_ROUTE_SCRIPTS.contains(&(CRYSTAL_MOUNTAIN_CAMERA_SCRIPT, "Crystal Mountain"))
        );
    }

    #[test]
    fn desert_camera_walks_pin_every_authored_review_landmark() {
        struct DesertWalkCase<'a> {
            script_path: &'a str,
            scenario: &'a str,
            party_start: CameraRouteTile,
            graph_steps: &'a [u8],
            anchors: &'a [&'a str],
            captures: &'a [&'a str],
        }

        let cases = [
            DesertWalkCase {
                script_path: DESERT_TRANSITION_CAMERA_SCRIPT,
                scenario: "Desert Transition",
                party_start: CameraRouteTile {
                    q: -12,
                    r: 0,
                    level: 15,
                },
                graph_steps: &[4, 4, 3, 1, 4, 4],
                anchors: &["grass_overlook", "transition_center", "sand_overlook"],
                captures: &[
                    "01-desert-transition-bands-map",
                    "02-desert-transition-grass-front-character",
                    "03-desert-transition-grass-reverse-character",
                    "04-desert-transition-ecotone-character",
                    "05-desert-transition-sand-front-character",
                    "06-desert-transition-sand-reverse-character",
                ],
            },
            DesertWalkCase {
                script_path: DESERT_PLAIN_CAMERA_SCRIPT,
                scenario: "Desert Plain",
                party_start: CameraRouteTile {
                    q: -12,
                    r: 0,
                    level: 15,
                },
                graph_steps: &[4, 4, 4, 4, 1],
                anchors: &["desert_plain_overlook"],
                captures: &[
                    "01-desert-plain-relief-map",
                    "02-desert-plain-overlook-front-character",
                    "03-desert-plain-overlook-side-character",
                    "04-desert-plain-overlook-rear-character",
                ],
            },
            DesertWalkCase {
                script_path: DUNES_CAMERA_SCRIPT,
                scenario: "Dunes",
                party_start: CameraRouteTile {
                    q: -12,
                    r: 0,
                    level: 21,
                },
                graph_steps: &[4, 4, 4, 4, 2],
                anchors: &["dune_crest", "dune_trough"],
                captures: &[
                    "01-dunes-ridge-field-map",
                    "02-dunes-crest-front-character",
                    "03-dunes-crest-side-character",
                    "04-dunes-trough-front-character",
                    "05-dunes-trough-ridge-wall-character",
                ],
            },
            DesertWalkCase {
                script_path: DESERT_OASIS_RINGS_CAMERA_SCRIPT,
                scenario: "Desert Oasis Rings",
                party_start: CameraRouteTile {
                    q: -13,
                    r: 6,
                    level: 15,
                },
                graph_steps: &[4, 3, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 2, 4, 4, 4, 4, 2],
                anchors: &[
                    "oasis_overlook",
                    "inner_dune_crest",
                    "outer_dune_crest",
                    "desert_plain_overlook",
                ],
                captures: &[
                    "01-desert-oasis-rings-oasis-overview-map",
                    "02-desert-oasis-rings-oasis-overlook-character",
                    "03-desert-oasis-rings-oasis-palms-character",
                    "04-desert-oasis-rings-inner-dune-crest-character",
                    "05-desert-oasis-rings-inner-dune-oasis-reverse-character",
                    "06-desert-oasis-rings-outer-dune-crest-character",
                    "07-desert-oasis-rings-open-desert-character",
                ],
            },
        ];

        for case in cases {
            let script_path = case.script_path;
            let scenario = case.scenario;
            let steps: Vec<WalkStep> = ron::from_str(
                &std::fs::read_to_string(
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(script_path),
                )
                .unwrap_or_else(|error| panic!("cannot read {script_path}: {error}")),
            )
            .unwrap_or_else(|error| panic!("cannot parse {script_path}: {error}"));
            for step in &steps {
                validate_step(step)
                    .unwrap_or_else(|error| panic!("{script_path} is invalid: {error}"));
            }

            assert!(steps.contains(&WalkStep::StartScenario {
                name: scenario.to_owned(),
                seed: Some(1_592_598_566),
                suppress_hostiles: true,
            }));
            assert!(steps.contains(&WalkStep::AssertCameraMode(WalkCameraMode::Map)));
            assert!(steps.contains(&WalkStep::AssertCameraMode(WalkCameraMode::Character)));
            let anchors = steps
                .iter()
                .filter_map(|step| match step {
                    WalkStep::ClickAnchor { name, .. } => Some(name.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(anchors, case.anchors, "{scenario}");
            let captures = steps
                .iter()
                .filter_map(|step| match step {
                    WalkStep::Capture(name) => Some(name.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(captures, case.captures, "{scenario}");
            assert!(steps
                .windows(4)
                .filter(|window| matches!(
                    window.first(),
                    Some(WalkStep::ClickAnchor { .. } | WalkStep::ClickTile { .. })
                ))
                .all(|window| matches!(
                    window,
                    [
                        WalkStep::ClickAnchor { .. } | WalkStep::ClickTile { .. },
                        WalkStep::Settle(5),
                        WalkStep::AwaitPartyIdle { .. },
                        WalkStep::AssertSelectedAt { .. },
                    ]
                )));

            // These edge counts are emitted by the production Footing router over
            // the seed-exact published tile graph (including Oasis palm blockers).
            // The horizontal and vertical lower bounds make accidental coordinate
            // edits fail even before a visual walk reaches the tactical range gate.
            let destinations = steps
                .iter()
                .filter_map(|step| match step {
                    WalkStep::ClickAnchor { expected, .. } => Some(*expected),
                    WalkStep::ClickTile {
                        q,
                        r,
                        level: Some(level),
                    } => Some(CameraRouteTile {
                        q: *q,
                        r: *r,
                        level: *level,
                    }),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(destinations.len(), case.graph_steps.len(), "{scenario}");
            let mut previous = case.party_start;
            for (destination, &graph_steps) in destinations.iter().zip(case.graph_steps) {
                assert!((1..=4).contains(&graph_steps), "{scenario}");
                assert!(
                    previous
                        .position()
                        .coord
                        .distance(destination.position().coord)
                        <= u32::from(graph_steps),
                    "{scenario} movement leg {previous:?} -> {destination:?} cannot fit its \
                     recorded {graph_steps}-step ordinary path"
                );
                assert!(
                    previous.level.abs_diff(destination.level) <= u32::from(graph_steps),
                    "{scenario} movement leg {previous:?} -> {destination:?} changes too many \
                     levels for {graph_steps} ordinary steps"
                );
                previous = *destination;
            }
            assert!(destinations.len() > case.anchors.len(), "{scenario}");
            assert!(CAMERA_ROUTE_SCRIPTS.contains(&(script_path, scenario)));
        }
    }

    #[test]
    fn island_camera_walks_pin_coasts_canopies_and_all_camera_modes() {
        struct IslandWalkCase<'a> {
            script_path: &'a str,
            scenario: &'a str,
            anchors: &'a [&'a str],
            captures: &'a [&'a str],
        }

        let cases = [
            IslandWalkCase {
                script_path: SANDY_ISLETS_CAMERA_SCRIPT,
                scenario: "Sandy Islets",
                anchors: &["sandy_islets_primary_overlook"],
                captures: &[
                    "01-sandy-islets-five-islet-map",
                    "02-sandy-islets-primary-overlook-character",
                    "03-sandy-islets-satellite-channel-character",
                    "04-sandy-islets-primary-overlook-first-person",
                ],
            },
            IslandWalkCase {
                script_path: WOODED_ISLAND_CAMERA_SCRIPT,
                scenario: "Wooded Island",
                anchors: &["wooded_island_clearing"],
                captures: &[
                    "01-wooded-island-coast-and-canopy-map",
                    "02-wooded-island-beach-character",
                    "03-wooded-island-beach-coast-character",
                    "04-wooded-island-high-clearing-character",
                    "05-wooded-island-ridge-and-canopy-character",
                    "06-wooded-island-high-clearing-first-person",
                ],
            },
            IslandWalkCase {
                script_path: OCEAN_ARCHIPELAGOES_CAMERA_SCRIPT,
                scenario: "Ocean Archipelagoes",
                anchors: &["archipelago.home_beach", "archipelago.home_ridge"],
                captures: &[
                    "01-ocean-archipelagoes-complete-map",
                    "02-ocean-archipelagoes-home-channel-character",
                    "03-ocean-archipelagoes-home-channel-first-person",
                    "04-ocean-archipelagoes-causeway-character",
                    "05-ocean-archipelagoes-wooded-heart-character",
                    "06-ocean-archipelagoes-satellite-coast-character",
                    "07-ocean-archipelagoes-wooded-heart-first-person",
                ],
            },
        ];

        for case in cases {
            let text = std::fs::read_to_string(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(case.script_path),
            )
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", case.script_path));
            let steps: Vec<WalkStep> = ron::from_str(&text)
                .unwrap_or_else(|error| panic!("cannot parse {}: {error}", case.script_path));
            for step in &steps {
                validate_step(step)
                    .unwrap_or_else(|error| panic!("{} is invalid: {error}", case.script_path));
            }

            assert!(steps.contains(&WalkStep::StartScenario {
                name: case.scenario.to_owned(),
                seed: Some(1_592_598_566),
                suppress_hostiles: false,
            }));
            for camera in [
                WalkCameraMode::Map,
                WalkCameraMode::Character,
                WalkCameraMode::FirstPerson,
            ] {
                assert!(
                    steps.contains(&WalkStep::AssertCameraMode(camera)),
                    "{} omits {camera:?} review evidence",
                    case.scenario
                );
            }
            assert_eq!(
                steps
                    .iter()
                    .filter_map(|step| match step {
                        WalkStep::ClickAnchor { name, .. } => Some(name.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
                case.anchors,
                "{}",
                case.scenario
            );
            assert_eq!(
                steps
                    .iter()
                    .filter_map(|step| match step {
                        WalkStep::Capture(name) => Some(name.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
                case.captures,
                "{}",
                case.scenario
            );
            assert!(
                steps
                    .windows(4)
                    .filter(|window| matches!(
                        window.first(),
                        Some(WalkStep::ClickAnchor { .. } | WalkStep::ClickTile { .. })
                    ))
                    .all(|window| matches!(
                        window,
                        [
                            WalkStep::ClickAnchor { .. } | WalkStep::ClickTile { .. },
                            WalkStep::Settle(5),
                            WalkStep::AwaitPartyIdle { .. },
                            WalkStep::AssertSelectedAt { .. },
                        ]
                    )),
                "{} movement legs need exact arrival proofs",
                case.scenario
            );
            assert!(CAMERA_ROUTE_SCRIPTS.contains(&(case.script_path, case.scenario)));
            assert!(steps.ends_with(&[
                WalkStep::Key("Backspace".to_owned()),
                WalkStep::AwaitScreen("Title".to_owned()),
            ]));
        }
    }

    #[test]
    fn mountain_range_walk_pins_review_route_and_rear_silhouette() {
        let steps: Vec<WalkStep> =
            ron::from_str(include_str!("../../../walks/camera_mountain_range.ron"))
                .expect("the Mountain Range camera walk parses");
        let manifest: CameraRouteManifest =
            ron::from_str(include_str!("../../../walks/camera_routes.ron"))
                .expect("the camera route manifest parses");

        assert!(steps.contains(&WalkStep::StartScenario {
            name: "Mountain Range".to_owned(),
            seed: Some(129_704_046),
            suppress_hostiles: true,
        }));

        let captures = steps
            .iter()
            .filter_map(|step| match step {
                WalkStep::Capture(name) => Some(name.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            captures,
            vec![
                "01-mountain-range-front-massif",
                "02-mountain-range-rear-silhouette",
                "03-mountain-range-coast",
                "04-mountain-range-watershed",
                "05-mountain-range-foothills",
                "06-mountain-range-mountain-tiers-front",
                "07-mountain-range-mountain-tiers-rear",
                "08-mountain-range-deep-mountain-base",
            ]
        );

        let orbits = steps
            .iter()
            .filter_map(|step| match step {
                WalkStep::OrbitCamera { yaw_turns, .. } => Some(yaw_turns.to_bits()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            orbits,
            vec![
                0.5_f32.to_bits(),
                (-0.5_f32).to_bits(),
                0.5_f32.to_bits(),
                (-0.5_f32).to_bits(),
            ]
        );

        let clicks = steps
            .iter()
            .filter_map(|step| match step {
                WalkStep::ClickAnchor { name, expected } => Some((name.as_str(), *expected)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            clicks,
            vec![
                (
                    "coast_review",
                    CameraRouteTile {
                        q: -52,
                        r: 25,
                        level: 12,
                    },
                ),
                (
                    "inland_review",
                    CameraRouteTile {
                        q: -9,
                        r: 14,
                        level: 20,
                    },
                ),
                (
                    "foothill_review",
                    CameraRouteTile {
                        q: -7,
                        r: 13,
                        level: 20,
                    },
                ),
                (
                    "massif_front_review",
                    CameraRouteTile {
                        q: 31,
                        r: 5,
                        level: 34,
                    },
                ),
                (
                    "deep_mountain_base",
                    CameraRouteTile {
                        q: 53,
                        r: 6,
                        level: 48,
                    },
                ),
            ]
        );

        let route = manifest
            .routes
            .iter()
            .find(|route| route.scenario == "Mountain Range")
            .expect("Mountain Range is present in the route manifest");
        assert_eq!(route.seed, Some(129704046));
        let manifested_clicks = route
            .points
            .iter()
            .filter_map(|point| match &point.destination {
                CameraRouteDestination::Anchor { name, expected } => {
                    Some((name.as_str(), *expected))
                }
                CameraRouteDestination::Exact(_) => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(manifested_clicks.len(), route.points.len());
        assert_eq!(manifested_clicks, clicks);
        assert!(route.points.iter().any(|point| {
            point.label == "massif front and rear silhouette"
                && point
                    .azimuth_turns
                    .iter()
                    .map(|azimuth| azimuth.to_bits())
                    .eq([0.0_f32.to_bits(), 0.5_f32.to_bits()])
        }));

        let map_front = steps
            .iter()
            .position(|step| step == &WalkStep::Capture("01-mountain-range-front-massif".into()))
            .expect("the front massif capture exists");
        let map_turn_rear = steps
            .iter()
            .position(|step| {
                matches!(
                    step,
                    WalkStep::OrbitCamera { yaw_turns, .. }
                        if yaw_turns.to_bits() == 0.5_f32.to_bits()
                )
            })
            .expect("the Map-mode rear orbit exists");
        let map_rear = steps
            .iter()
            .position(|step| step == &WalkStep::Capture("02-mountain-range-rear-silhouette".into()))
            .expect("the rear silhouette capture exists");
        let map_restore_front = steps
            .iter()
            .position(|step| {
                matches!(
                    step,
                    WalkStep::OrbitCamera { yaw_turns, .. }
                        if yaw_turns.to_bits() == (-0.5_f32).to_bits()
                )
            })
            .expect("the Map-mode front orbit restoration exists");
        let character_mode = steps
            .iter()
            .position(|step| step == &WalkStep::Key("C".into()))
            .expect("the walk enters Character camera mode");
        let massif_click = steps
            .iter()
            .position(|step| {
                matches!(
                    step,
                    WalkStep::ClickAnchor { name, .. } if name == "massif_front_review"
                )
            })
            .expect("the walk clicks the massif-front destination");
        let massif_proof = steps
            .iter()
            .position(|step| {
                matches!(
                    step,
                    WalkStep::AssertSelectedAt { expected }
                        if *expected
                            == CameraRouteTile {
                                q: 31,
                                r: 5,
                                level: 34,
                            }
                )
            })
            .expect("the walk proves exact arrival at the massif front");
        let character_front = steps
            .iter()
            .position(|step| {
                step == &WalkStep::Capture("06-mountain-range-mountain-tiers-front".into())
            })
            .expect("the Character front-massif capture exists");
        let character_turn_rear = steps
            .iter()
            .enumerate()
            .filter_map(|(index, step)| {
                matches!(
                    step,
                    WalkStep::OrbitCamera { yaw_turns, .. }
                        if yaw_turns.to_bits() == 0.5_f32.to_bits()
                )
                .then_some(index)
            })
            .nth(1)
            .expect("the Character rear orbit exists");
        let character_rear = steps
            .iter()
            .position(|step| {
                step == &WalkStep::Capture("07-mountain-range-mountain-tiers-rear".into())
            })
            .expect("the Character rear-massif capture exists");
        let character_restore_front = steps
            .iter()
            .enumerate()
            .filter_map(|(index, step)| {
                matches!(
                    step,
                    WalkStep::OrbitCamera { yaw_turns, .. }
                        if yaw_turns.to_bits() == (-0.5_f32).to_bits()
                )
                .then_some(index)
            })
            .nth(1)
            .expect("the Character front orbit restoration exists");
        let deep_mountain_click = steps
            .iter()
            .position(|step| {
                matches!(
                    step,
                    WalkStep::ClickAnchor { name, .. } if name == "deep_mountain_base"
                )
            })
            .expect("the walk continues to the Deep Mountain base");

        assert!(map_front < map_turn_rear);
        assert!(map_turn_rear < map_rear);
        assert!(map_rear < map_restore_front);
        assert!(map_restore_front < character_mode);
        assert!(character_mode < massif_click);
        assert!(massif_click < massif_proof);
        assert!(massif_proof < character_front);
        assert!(character_front < character_turn_rear);
        assert!(character_turn_rear < character_rear);
        assert!(character_rear < character_restore_front);
        assert!(character_restore_front < deep_mountain_click);
    }

    #[test]
    fn grand_v3_walk_pins_the_complete_surface_and_crystal_itinerary() {
        let steps: Vec<WalkStep> =
            ron::from_str(include_str!("../../../walks/camera_grand_v3_baseline.ron"))
                .expect("the Grand V3 Baseline camera walk parses");

        assert!(steps.contains(&WalkStep::StartScenario {
            name: "Grand V3 Baseline".to_owned(),
            seed: Some(1_592_598_566),
            suppress_hostiles: false,
        }));
        for camera in [
            WalkCameraMode::Map,
            WalkCameraMode::Character,
            WalkCameraMode::FirstPerson,
        ] {
            assert!(
                steps.contains(&WalkStep::AssertCameraMode(camera)),
                "Grand V3 Baseline omits {camera:?} review evidence"
            );
        }

        let clicked_anchors = steps
            .iter()
            .filter_map(|step| match step {
                WalkStep::ClickAnchor { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            clicked_anchors,
            [
                "grand_v3.archipelago",
                "grand_v3.coastal_bridge",
                "grand_v3.valley_bridge",
                "grand_v3.valley_lake",
                "grand_v3.natural_pass",
                "grand_v3.massif",
                "grand_v3.peak_saddle",
                "grand_v3.mountain_lake",
                "grand_v3.frozen_woods",
                "grand_v3.tunnel_mouth",
                "grand_v3.tunnel_midpoint",
                "grand_v3.gothic_transition",
                "grand_v3.ascent_threshold",
                "crystal_ascent.bottom_chamber",
                "crystal_ascent.mid_flight",
                "crystal_ascent.upper_exit",
            ],
            "the walk must retain the approved lowland, upper-route, tunnel, and Ascent order"
        );
        assert!(
            steps
                .windows(4)
                .filter(|window| matches!(window.first(), Some(WalkStep::ClickAnchor { .. })))
                .all(|window| matches!(
                    window,
                    [
                        WalkStep::ClickAnchor { .. },
                        WalkStep::Settle(5),
                        WalkStep::AwaitPartyIdle { .. },
                        WalkStep::AssertSelectedAt { .. },
                    ]
                )),
            "every Grand V3 movement leg needs an exact stale-checked arrival proof"
        );

        let captures = steps
            .iter()
            .filter_map(|step| match step {
                WalkStep::Capture(name) => Some(name.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(captures.len(), 52);
        assert_eq!(
            captures.first(),
            Some(&"01-grand-v3-complete-opaque-world-map")
        );
        assert_eq!(
            captures.last(),
            Some(&"58-grand-v3-crystal-summit-first-person")
        );
        let [_, stops @ ..] = captures.as_slice() else {
            panic!("the complete-world overview must precede the three-view stops");
        };
        for stop in stops.chunks_exact(3) {
            let [map, character, first_person] = stop else {
                panic!("each stop must retain its three camera views");
            };
            assert!(map.ends_with("-map"), "{map} is not a Map frame");
            assert!(
                character.ends_with("-character"),
                "{character} is not a Character frame"
            );
            assert!(
                first_person.ends_with("-first-person"),
                "{first_person} is not a First Person frame"
            );
        }
        assert!(steps.ends_with(&[
            WalkStep::Settle(5),
            WalkStep::Key("Backspace".to_owned()),
            WalkStep::AwaitScreen("Title".to_owned()),
        ]));
        assert!(
            CAMERA_ROUTE_SCRIPTS.contains(&(GRAND_V3_BASELINE_CAMERA_SCRIPT, "Grand V3 Baseline"))
        );
    }

    #[test]
    fn grand_v3_corrective_motion_walk_is_exact_and_bidirectional() {
        let steps: Vec<WalkStep> = ron::from_str(include_str!(
            "../../../walks/camera_grand_v3_corrective_motion.ron"
        ))
        .expect("the Grand V3 corrective motion walk parses");
        validate_script_steps(GRAND_V3_CORRECTIVE_MOTION_SCRIPT, &steps)
            .expect("the Grand V3 corrective motion walk validates");

        assert_eq!(
            AUXILIARY_CAMERA_REVIEW_SCRIPTS,
            &[(GRAND_V3_CORRECTIVE_MOTION_SCRIPT, "Grand V3 Baseline")]
        );
        assert!(steps.contains(&WalkStep::StartScenario {
            name: "Grand V3 Baseline".to_owned(),
            seed: Some(1_592_598_566),
            suppress_hostiles: false,
        }));
        assert!(steps.contains(&WalkStep::AssertCameraMode(WalkCameraMode::Character)));

        let expected_sequences = [
            (
                "grand_v3.tunnel_midpoint",
                CameraRouteTile {
                    q: 22,
                    r: -47,
                    level: 6,
                },
                "05-tunnel-inbound-motion",
                28,
                32,
                18_000,
            ),
            (
                "grand_v3.tunnel_mouth",
                CameraRouteTile {
                    q: 22,
                    r: 31,
                    level: 7,
                },
                "06-tunnel-outbound-motion",
                28,
                32,
                18_000,
            ),
            (
                "crystal_ascent.bottom_chamber",
                CameraRouteTile {
                    q: 6,
                    r: -124,
                    level: 6,
                },
                "07-crystal-threshold-chamber-motion",
                24,
                12,
                7_200,
            ),
            (
                "grand_v3.ascent_threshold",
                CameraRouteTile {
                    q: -10,
                    r: -115,
                    level: 6,
                },
                "08-crystal-chamber-threshold-motion",
                24,
                12,
                7_200,
            ),
            (
                "crystal_ascent.upper_contraction",
                CameraRouteTile {
                    q: 22,
                    r: -113,
                    level: 138,
                },
                "09-crystal-corner-contraction-motion",
                16,
                12,
                4_800,
            ),
            (
                "crystal_ascent.corner_landing",
                CameraRouteTile {
                    q: 33,
                    r: -122,
                    level: 134,
                },
                "10-crystal-contraction-corner-motion",
                16,
                12,
                4_800,
            ),
            (
                "grand_v3.frozen_exit",
                CameraRouteTile {
                    q: 56,
                    r: -151,
                    level: 152,
                },
                "11-crystal-frozen-exit-motion",
                12,
                4,
                1_200,
            ),
            (
                "crystal_ascent.upper_exit",
                CameraRouteTile {
                    q: 53,
                    r: -148,
                    level: 150,
                },
                "12-frozen-crystal-exit-motion",
                12,
                4,
                1_200,
            ),
        ];

        let mut actual_sequences = Vec::new();
        for (index, step) in steps.iter().enumerate() {
            let WalkStep::CaptureWhileMoving {
                prefix,
                every_frames,
                capture_count,
            } = step
            else {
                continue;
            };
            let Some(WalkStep::ClickAnchor { name, expected }) =
                index.checked_sub(1).and_then(|prior| steps.get(prior))
            else {
                panic!("{prefix} is not immediately preceded by ClickAnchor");
            };
            let Some(WalkStep::AwaitPartyIdle { max_frames }) = steps.get(index + 1) else {
                panic!("{prefix} is not immediately followed by AwaitPartyIdle");
            };
            assert_eq!(
                steps.get(index + 2),
                Some(&WalkStep::AssertSelectedAt {
                    expected: *expected,
                }),
                "{prefix} lacks its matching exact arrival proof"
            );
            actual_sequences.push((
                name.as_str(),
                *expected,
                prefix.as_str(),
                *every_frames,
                *capture_count,
                *max_frames,
            ));
        }
        assert_eq!(actual_sequences, expected_sequences);

        let captured_legs = steps
            .iter()
            .enumerate()
            .filter_map(|(index, step)| {
                let WalkStep::CaptureWhileMoving { .. } = step else {
                    return None;
                };
                let source = steps
                    .get(..index)
                    .expect("the enumerated step bounds its preceding prefix")
                    .iter()
                    .rev()
                    .find_map(|prior| match prior {
                        WalkStep::AssertSelectedAt { expected } => Some(*expected),
                        _ => None,
                    })?;
                let destination = match steps.get(index.checked_sub(1)?)? {
                    WalkStep::ClickAnchor { expected, .. } => *expected,
                    _ => return None,
                };
                Some((source, destination))
            })
            .collect::<Vec<_>>();
        let player: hex_assets::PlayerSettings =
            ron::from_str(include_str!("../../../assets/config/player.ron"))
                .expect("the shipped player movement settings parse");
        let frames_per_flat_hex = f64::from(hex_core::config::HEX_SMALL_DIAMETER / player.speed)
            / WALK_FRAME_DURATION.as_secs_f64();
        for ((source, destination), (_, _, prefix, every_frames, capture_count, _)) in
            captured_legs.iter().zip(&actual_sequences)
        {
            let minimum_route_frames = f64::from(
                source
                    .position()
                    .coord
                    .distance(destination.position().coord),
            ) * frames_per_flat_hex;
            let capture_span = f64::from(*every_frames * u32::from(*capture_count));
            assert!(
                capture_span + 8.0 < minimum_route_frames,
                "{prefix} requests its final frame too close to the earliest possible arrival: span={capture_span}, minimum={minimum_route_frames:.2}"
            );
            assert!(
                capture_span * 2.0 >= minimum_route_frames,
                "{prefix} samples less than half of the direct endpoint bound; paired directions cannot even bracket the route ends: span={capture_span}, minimum={minimum_route_frames:.2}"
            );
        }
        assert_eq!(
            actual_sequences
                .iter()
                .map(|(_, _, _, _, capture_count, _)| usize::from(*capture_count))
                .sum::<usize>(),
            120,
            "the temporal approval gate must publish exactly 120 frames"
        );

        let orbit_reversals = steps
            .iter()
            .filter_map(|step| match step {
                WalkStep::OrbitCamera {
                    yaw_turns,
                    pitch_fraction,
                } => Some((*yaw_turns, *pitch_fraction)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            orbit_reversals,
            [
                (0.5, 0.0),
                (-0.5, 0.0),
                (0.5, 0.0),
                (-0.5, 0.0),
                (0.5, 0.0),
                (-0.5, 0.0),
                (0.5, 0.0),
                (-0.5, 0.0),
            ],
            "each forward/reverse pair must inspect opposing camera sides and restore its yaw"
        );
    }

    #[test]
    fn obstructed_route_cards_use_explicit_open_side_azimuths() {
        let manifest: CameraRouteManifest =
            ron::from_str(include_str!("../../../walks/camera_routes.ron"))
                .expect("the camera route manifest parses");
        let expected_manifest = [
            ("Mountains", "stream overlook", vec![0.0, -1.0 / 6.0]),
            ("Mountains", "low bypass", vec![-1.0 / 3.0]),
            (
                "Waterfall",
                "fall overlook",
                vec![0.0, -1.0 / 6.0, -1.0 / 3.0],
            ),
            (
                "Fort",
                "east gate approach",
                vec![0.0, 1.0 / 6.0, -1.0 / 6.0],
            ),
            ("Two Rings", "central confluence", vec![0.0, 1.0 / 6.0]),
            ("Two Rings", "waterfall B", vec![1.0 / 6.0, -1.0 / 6.0]),
            (
                "Two Rings",
                "mountains A water",
                vec![-1.0 / 6.0, -1.0 / 12.0],
            ),
            ("Two Rings", "mountains B pass", vec![1.0 / 3.0, -1.0 / 3.0]),
            ("Two Rings", "mountains C stream", vec![0.0, -1.0 / 6.0]),
            ("Two Rings", "frozen bridge", vec![0.0, 1.0 / 3.0]),
            ("Two Rings", "outlet fall", vec![0.0, -1.0 / 6.0]),
        ];
        for (scenario, label, expected) in expected_manifest {
            let actual = manifest
                .routes
                .iter()
                .find(|route| route.scenario == scenario)
                .and_then(|route| route.points.iter().find(|point| point.label == label))
                .unwrap_or_else(|| panic!("missing {scenario:?} route point {label:?}"));
            assert_eq!(actual.azimuth_turns, expected, "{scenario} {label}");
        }

        let expected_gestures = [
            (
                "../../walks/camera_mountains.ron",
                vec![-1.0 / 6.0, 1.0 / 6.0, -1.0 / 3.0],
            ),
            (
                "../../walks/camera_waterfall.ron",
                vec![-1.0 / 6.0, -1.0 / 6.0],
            ),
            ("../../walks/camera_fort.ron", vec![1.0 / 6.0, -1.0 / 3.0]),
            (
                "../../walks/camera_two_rings.ron",
                vec![1.0 / 6.0, -1.0 / 3.0, 1.0 / 12.0],
            ),
            (
                "../../walks/camera_two_rings_mountains.ron",
                vec![
                    1.0 / 3.0,
                    1.0 / 3.0,
                    1.0 / 3.0,
                    -1.0 / 6.0,
                    1.0 / 6.0,
                    1.0 / 3.0,
                ],
            ),
            (
                "../../walks/camera_two_rings_west.ron",
                vec![-1.0 / 6.0, 1.0 / 6.0, -1.0 / 3.0, 1.0 / 3.0, -1.0 / 3.0],
            ),
        ];
        for (script_path, expected) in expected_gestures {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(script_path);
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
            let steps: Vec<WalkStep> = ron::from_str(&text)
                .unwrap_or_else(|error| panic!("cannot parse {}: {error}", path.display()));
            let actual = steps
                .iter()
                .filter_map(|step| match step {
                    WalkStep::OrbitCamera { yaw_turns, .. } => Some(*yaw_turns),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(actual, expected, "{}", path.display());
        }
    }

    #[test]
    fn critical_camera_scripts_use_only_manifested_real_movement_destinations() {
        let manifest: CameraRouteManifest =
            ron::from_str(include_str!("../../../walks/camera_routes.ron"))
                .expect("the camera route manifest parses");
        let scripted_scenarios = CAMERA_ROUTE_SCRIPTS
            .iter()
            .map(|(_, scenario)| *scenario)
            .collect::<std::collections::BTreeSet<_>>();
        let manifested_scenarios = manifest
            .routes
            .iter()
            .map(|route| route.scenario.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            CAMERA_ROUTE_SCRIPTS.len(),
            scripted_scenarios.len(),
            "camera script scenarios repeat"
        );
        assert_eq!(
            scripted_scenarios, manifested_scenarios,
            "every manifested Map needs exactly one executable camera script"
        );
        for &(script_path, scenario_name) in CAMERA_ROUTE_SCRIPTS {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(script_path);
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
            let steps: Vec<WalkStep> = ron::from_str(&text)
                .unwrap_or_else(|error| panic!("cannot parse {}: {error}", path.display()));
            for step in &steps {
                validate_step(step)
                    .unwrap_or_else(|error| panic!("{} invalid: {error}", path.display()));
            }

            let route = manifest
                .routes
                .iter()
                .find(|route| route.scenario == scenario_name)
                .unwrap_or_else(|| panic!("{scenario_name} is absent from the manifest"));
            let require_exact_arrival_proof = scenario_name != "Two Rings";
            let launches = steps
                .iter()
                .filter_map(|step| match step {
                    WalkStep::StartScenario {
                        name,
                        seed,
                        suppress_hostiles,
                    } => Some((name.as_str(), *seed, *suppress_hostiles)),
                    _ => None,
                })
                .collect::<Vec<_>>();
            let suppress_hostiles = matches!(
                scenario_name,
                "Mountain Range"
                    | "Desert Transition"
                    | "Desert Plain"
                    | "Dunes"
                    | "Desert Oasis Rings"
            );
            assert_eq!(
                launches,
                vec![(scenario_name, route.seed, suppress_hostiles)],
                "only presentation-focused routes may remove hostiles from map evidence"
            );
            assert!(steps.contains(&WalkStep::Key("C".to_owned())));
            assert!(
                steps
                    .iter()
                    .filter(|step| matches!(step, WalkStep::OrbitCamera { .. }))
                    .count()
                    >= 2,
                "{scenario_name} needs multiple player-authored azimuths"
            );
            assert!(
                steps
                    .iter()
                    .filter(|step| matches!(step, WalkStep::Capture(_)))
                    .count()
                    >= 3,
                "{scenario_name} needs before/after multi-azimuth evidence"
            );

            let mut movement_steps = 0_usize;
            let mut pending_proof = None;
            let mut saw_idle_after_click = false;
            let mut proved_manifested_start = false;
            for step in &steps {
                let destination = match step {
                    WalkStep::ClickAnchor { name, expected } => {
                        Some(CameraRouteDestination::Anchor {
                            name: name.clone(),
                            expected: *expected,
                        })
                    }
                    WalkStep::ClickTile {
                        q,
                        r,
                        level: Some(level),
                    } => Some(CameraRouteDestination::Exact(CameraRouteTile {
                        q: *q,
                        r: *r,
                        level: *level,
                    })),
                    WalkStep::ClickTile { level: None, .. } => {
                        panic!(
                            "{} contains an ambiguous camera-route click",
                            path.display()
                        )
                    }
                    _ => None,
                };
                if let Some(destination) = destination {
                    movement_steps += 1;
                    assert!(
                        route
                            .points
                            .iter()
                            .any(|point| point.destination == destination),
                        "{} uses {destination:?}, absent from its stale-checked manifest",
                        path.display()
                    );
                    if require_exact_arrival_proof {
                        assert!(
                            pending_proof.is_none(),
                            "{} clicks another destination before proving the previous movement",
                            path.display()
                        );
                        pending_proof = Some(match destination {
                            CameraRouteDestination::Anchor { expected, .. }
                            | CameraRouteDestination::Exact(expected) => expected,
                        });
                        saw_idle_after_click = false;
                    }
                    continue;
                }

                match step {
                    WalkStep::AwaitPartyIdle { .. } if pending_proof.is_some() => {
                        saw_idle_after_click = true;
                    }
                    WalkStep::AssertSelectedAt { expected } if require_exact_arrival_proof => {
                        if let Some(clicked) = pending_proof.take() {
                            assert!(
                                saw_idle_after_click,
                                "{} proves {clicked:?} before awaiting party idle",
                                path.display()
                            );
                            assert_eq!(
                                *expected,
                                clicked,
                                "{} proves a different surface than it clicked",
                                path.display()
                            );
                        } else {
                            assert_eq!(
                                movement_steps,
                                0,
                                "{} proves a position without a pending movement",
                                path.display()
                            );
                            assert!(
                                !proved_manifested_start,
                                "{} proves its initial position more than once",
                                path.display()
                            );
                            let manifested_start = route
                                .points
                                .first()
                                .map(|point| match &point.destination {
                                    CameraRouteDestination::Anchor { expected, .. }
                                    | CameraRouteDestination::Exact(expected) => *expected,
                                })
                                .unwrap_or_else(|| {
                                    panic!("{} has no manifested route start", path.display())
                                });
                            assert_eq!(
                                *expected,
                                manifested_start,
                                "{} proves an initial position other than its first manifested route point",
                                path.display()
                            );
                            proved_manifested_start = true;
                        }
                    }
                    WalkStep::Capture(name) => assert!(
                        pending_proof.is_none(),
                        "{} captures {name:?} before proving its movement destination",
                        path.display()
                    ),
                    _ => {}
                }
            }
            assert!(
                pending_proof.is_none(),
                "{} ends with an unproved movement destination",
                path.display()
            );
            assert!(
                movement_steps > 0,
                "{scenario_name} has no real movement leg"
            );
            assert!(steps.iter().any(|step| matches!(
                step,
                WalkStep::AwaitPartyIdle { max_frames } if *max_frames > 0
            )));
        }
    }

    #[test]
    fn grouped_two_rings_walks_review_every_region_from_a_proved_destination() {
        let manifest: CameraRouteManifest =
            ron::from_str(include_str!("../../../walks/camera_routes.ron"))
                .expect("the camera route manifest parses");
        let route = manifest
            .routes
            .iter()
            .find(|route| route.scenario == "Two Rings")
            .expect("Two Rings is present in the route manifest");
        let expected = route
            .points
            .iter()
            .map(|point| point.destination.clone())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(expected.len(), 19, "Ring19 needs one point per region");

        let mut reviewed_counts = std::collections::BTreeMap::new();
        for script_path in TWO_RINGS_ROUTE_SCRIPTS {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(script_path);
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
            let steps: Vec<WalkStep> = ron::from_str(&text)
                .unwrap_or_else(|error| panic!("cannot parse {}: {error}", path.display()));
            for step in &steps {
                validate_step(step)
                    .unwrap_or_else(|error| panic!("{} invalid: {error}", path.display()));
            }
            let launches = steps
                .iter()
                .filter_map(|step| match step {
                    WalkStep::StartScenario {
                        name,
                        seed,
                        suppress_hostiles,
                    } => Some((name.as_str(), *seed, *suppress_hostiles)),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert!(
                !launches.is_empty(),
                "{} has no exact Two Rings launch",
                path.display()
            );
            assert!(
                launches.iter().all(|(name, seed, suppress)| {
                    *name == "Two Rings" && *seed == route.seed && !suppress
                }),
                "{} must restart only the exact seed-pinned Two Rings scenario",
                path.display()
            );
            let captures = steps
                .iter()
                .filter(|step| matches!(step, WalkStep::Capture(_)))
                .count();
            assert!(
                captures <= 10,
                "{} exceeds the ten-frame review budget",
                path.display()
            );
            let mut pending_destination = None;
            let mut saw_idle_after_click = false;
            let mut last_proved_destination = None;
            for step in &steps {
                let destination = match step {
                    WalkStep::ClickAnchor { name, expected } => {
                        Some(CameraRouteDestination::Anchor {
                            name: name.clone(),
                            expected: *expected,
                        })
                    }
                    WalkStep::ClickTile {
                        q,
                        r,
                        level: Some(level),
                    } => Some(CameraRouteDestination::Exact(CameraRouteTile {
                        q: *q,
                        r: *r,
                        level: *level,
                    })),
                    _ => None,
                };
                if let Some(destination) = destination {
                    assert!(
                        pending_destination.is_none(),
                        "{} starts a second movement before proving the first destination",
                        path.display()
                    );
                    assert!(
                        route
                            .points
                            .iter()
                            .any(|point| point.destination == destination),
                        "{} uses {destination:?}, absent from the stale-checked Two Rings manifest",
                        path.display()
                    );
                    pending_destination = Some(destination);
                    saw_idle_after_click = false;
                    last_proved_destination = None;
                    continue;
                }

                match step {
                    WalkStep::AwaitPartyIdle { .. } if pending_destination.is_some() => {
                        saw_idle_after_click = true;
                    }
                    WalkStep::AssertSelectedAt { expected } => {
                        let destination = pending_destination.take().unwrap_or_else(|| {
                            panic!(
                                "{} proves a position without a pending movement",
                                path.display()
                            )
                        });
                        assert!(
                            saw_idle_after_click,
                            "{} proves {destination:?} before awaiting party idle",
                            path.display()
                        );
                        let clicked = match destination {
                            CameraRouteDestination::Anchor { expected, .. }
                            | CameraRouteDestination::Exact(expected) => expected,
                        };
                        assert_eq!(
                            *expected,
                            clicked,
                            "{} proves a different surface than it clicked",
                            path.display()
                        );
                        last_proved_destination = Some(destination);
                    }
                    WalkStep::Capture(name) => {
                        assert!(
                            pending_destination.is_none(),
                            "{} captures {name:?} before proving its movement destination",
                            path.display()
                        );
                        if let Some(destination) = &last_proved_destination {
                            *reviewed_counts.entry(destination.clone()).or_insert(0usize) += 1;
                        }
                    }
                    WalkStep::StartScenario { .. } => {
                        assert!(
                            pending_destination.is_none(),
                            "{} changes scenarios before proving its movement destination",
                            path.display()
                        );
                        last_proved_destination = None;
                    }
                    WalkStep::Key(key) if key == "Backspace" => {
                        assert!(
                            pending_destination.is_none(),
                            "{} changes scenarios before proving its movement destination",
                            path.display()
                        );
                        last_proved_destination = None;
                    }
                    _ => {}
                }
            }
            assert!(
                pending_destination.is_none(),
                "{} ends with an unproved movement destination",
                path.display()
            );
        }

        let actual = reviewed_counts
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            actual, expected,
            "grouped Two Rings walks must capture every stale-checked region destination"
        );
        for (destination, captures) in reviewed_counts {
            assert!(
                captures >= 2,
                "Two Rings destination {destination:?} needs two reviewed azimuths, found {captures}"
            );
        }
    }

    #[test]
    fn review_viewport_is_explicit_and_positive() {
        assert_eq!(
            parse_viewport("1280x720@2"),
            hex_ui::ReviewViewport::new(1280, 720, 2.0)
        );
        assert_eq!(
            parse_viewport("1920x1080@1"),
            hex_ui::ReviewViewport::new(1920, 1080, 1.0)
        );
        for invalid in [
            "",
            "1280x720",
            "1280X720@1",
            "x720@1",
            "1280x@1",
            "0x720@1",
            "1280x0@1",
            "1280x720@0",
        ] {
            assert!(parse_viewport(invalid).is_err(), "{invalid:?} should fail");
        }
    }

    #[test]
    fn tile_click_resolution_requires_an_exact_stacked_surface() {
        let mut world = World::new();
        let low = world.spawn_empty().id();
        let high = world.spawn_empty().id();
        let elsewhere = world.spawn_empty().id();
        let coord = HexCoord::from_axial(4, -3);
        let low_pos = TilePos::new(coord, 2);
        let high_pos = TilePos::new(coord, 8);
        let elsewhere_pos = TilePos::new(HexCoord::from_axial(5, -3), 2);
        let open = Headroom(8);
        let tiles = [
            (low, low_pos, open),
            (high, high_pos, open),
            (elsewhere, elsewhere_pos, open),
        ];

        let ambiguous = resolve_tile_click_target(
            tiles
                .iter()
                .map(|(entity, pos, headroom)| (*entity, pos, headroom)),
            coord,
            None,
        )
        .expect_err("a stacked coordinate without a level must be refused");
        assert!(ambiguous.contains("stacked levels [2, 8]"));

        assert_eq!(
            resolve_tile_click_target(
                tiles
                    .iter()
                    .map(|(entity, pos, headroom)| (*entity, pos, headroom)),
                coord,
                Some(8),
            ),
            Ok(Some((high, high_pos)))
        );
        let missing_level = resolve_tile_click_target(
            tiles
                .iter()
                .map(|(entity, pos, headroom)| (*entity, pos, headroom)),
            coord,
            Some(7),
        )
        .expect_err("a missing exact level must not fall back to another run");
        assert!(missing_level.contains("available levels are [2, 8]"));

        let buried = Headroom(0);
        assert_eq!(
            resolve_tile_click_target(
                [(low, &low_pos, &buried), (high, &high_pos, &open)].into_iter(),
                coord,
                None,
            ),
            Ok(Some((high, high_pos))),
            "buried material runs are not pointer-clickable surfaces"
        );
    }

    #[test]
    fn tile_click_resolution_waits_only_before_any_terrain_exists() {
        let coord = HexCoord::from_axial(1, 2);
        assert_eq!(
            resolve_tile_click_target(std::iter::empty(), coord, None),
            Ok(None)
        );

        let mut world = World::new();
        let entity = world.spawn_empty().id();
        let pos = TilePos::new(HexCoord::ORIGIN, 1);
        let open = Headroom(8);
        let missing =
            resolve_tile_click_target(std::iter::once((entity, &pos, &open)), coord, None)
                .expect_err("a missing coordinate cannot appear after terrain publication");
        assert!(missing.contains("names no published terrain coordinate"));
    }

    #[test]
    fn tile_click_uses_the_real_primary_pointer_observer_path() {
        let mut app = App::new();
        let window = app.world_mut().spawn(Window::default()).id();
        let target = app.world_mut().spawn_empty().id();
        app.init_resource::<PointerRecord>()
            .insert_resource(PointerRequest { target, window })
            .add_observer(record_pointer_click)
            .add_systems(Update, issue_requested_pointer_click);

        app.update();

        let record = app.world().resource::<PointerRecord>();
        assert_eq!(record.target, Some(target));
        assert!(record.primary);
    }

    #[test]
    fn tile_hover_uses_the_real_primary_pointer_observer_path() {
        let mut app = App::new();
        let window = app.world_mut().spawn(Window::default()).id();
        let target = app.world_mut().spawn_empty().id();
        app.init_resource::<PointerRecord>()
            .insert_resource(PointerRequest { target, window })
            .add_observer(record_pointer_hover)
            .add_systems(Update, issue_requested_pointer_hover);

        app.update();

        let record = app.world().resource::<PointerRecord>();
        assert_eq!(record.target, Some(target));
        assert!(record.hovered);
    }

    #[test]
    fn party_idle_uses_stable_party_domain_facts_and_the_command_queue() {
        let mut app = App::new();
        app.init_resource::<Party>()
            .init_resource::<UnitRegistry>()
            .init_resource::<CommandQueue>()
            .init_resource::<PartyIdleRecord>()
            .add_systems(Update, record_party_idle);

        app.update();
        assert_eq!(
            app.world().resource::<PartyIdleRecord>().0,
            None,
            "an empty party is not ready rather than vacuously idle"
        );

        let member = hex_core::UnitId(17);
        let entity = app.world_mut().spawn_empty().id();
        app.world_mut().resource_mut::<Party>().members.push(member);
        app.world_mut()
            .resource_mut::<UnitRegistry>()
            .register(member, entity);
        app.update();
        assert_eq!(app.world().resource::<PartyIdleRecord>().0, Some(true));

        app.world_mut()
            .resource_mut::<CommandQueue>()
            .push(hex_core::IssuedCommand {
                seat: hex_core::PlayerSeat::default(),
                command: hex_core::GameCommand::EndTurn { unit: member },
            });
        app.update();
        assert_eq!(app.world().resource::<PartyIdleRecord>().0, Some(false));

        let _ = app.world_mut().resource_mut::<CommandQueue>().pop();
        app.world_mut().entity_mut(entity).insert(Busy);
        app.update();
        assert_eq!(app.world().resource::<PartyIdleRecord>().0, Some(false));

        app.world_mut().entity_mut(entity).remove::<Busy>();
        app.update();
        assert_eq!(app.world().resource::<PartyIdleRecord>().0, Some(true));
    }

    #[test]
    fn temporal_capture_tracks_only_the_selected_actors_domain_movement() {
        let mut app = App::new();
        app.init_resource::<SelectedMovementRecord>()
            .add_systems(Update, record_selected_movement);

        let standing = hex_units::Standing {
            pos: TilePos::ORIGIN,
            span: hex_core::HexSpan::new(0.0, 1.0),
        };
        let selected = app.world_mut().spawn((Selected, StandsOn(standing))).id();
        let _unrelated = app
            .world_mut()
            .spawn(MovingTo::new(vec![standing, standing], 1.0))
            .id();

        app.update();
        assert_eq!(
            app.world().resource::<SelectedMovementRecord>().0,
            Some(false),
            "another entity's movement must not authorize temporal evidence"
        );

        app.world_mut()
            .entity_mut(selected)
            .insert(MovingTo::new(vec![standing, standing], 1.0));
        app.update();
        assert_eq!(
            app.world().resource::<SelectedMovementRecord>().0,
            Some(true)
        );

        app.world_mut().entity_mut(selected).remove::<MovingTo>();
        app.update();
        assert_eq!(
            app.world().resource::<SelectedMovementRecord>().0,
            Some(false)
        );
    }

    #[test]
    fn selected_position_proof_requires_authority_and_camera_projection_to_agree() {
        let mut app = App::new();
        app.init_resource::<SelectedAtRecord>()
            .add_systems(Update, record_selected_at);
        let entity = app
            .world_mut()
            .spawn((
                Selected,
                StandsOn(hex_units::Standing {
                    pos: TilePos::ORIGIN,
                    span: hex_core::HexSpan::new(0.0, 1.0),
                }),
                CameraFocusTarget::new(TilePos::ORIGIN),
            ))
            .id();

        app.update();
        assert!(app
            .world()
            .resource::<SelectedAtRecord>()
            .0
            .as_ref()
            .expect("position proof ran")
            .is_ok());

        let wrong = TilePos::new(HexCoord::from_axial(1, 0), 0);
        app.world_mut()
            .entity_mut(entity)
            .insert(CameraFocusTarget::new(wrong));
        app.update();
        let reason = app
            .world()
            .resource::<SelectedAtRecord>()
            .0
            .as_ref()
            .expect("position proof reran")
            .as_ref()
            .expect_err("stale camera focus must fail");
        assert!(reason.contains("camera focus remains"), "{reason}");

        app.world_mut().entity_mut(entity).remove::<Selected>();
        app.update();
        let reason = app
            .world()
            .resource::<SelectedAtRecord>()
            .0
            .as_ref()
            .expect("missing-selection proof ran")
            .as_ref()
            .expect_err("missing selection must fail");
        assert!(reason.contains("exactly one selected unit"), "{reason}");
    }

    #[test]
    fn every_screen_name_round_trips() {
        for name in [
            "Splash",
            "Title",
            "Multiplayer",
            "Sandbox",
            "Settings",
            "CharacterCreator",
            "SpellCreator",
            "LatticeDemo",
            "VfxTuner",
            "Loading",
            "Gameplay",
        ] {
            parse_screen(name).expect("known screen parses");
        }
        assert!(parse_screen("Gameplay ").is_err());
    }

    #[test]
    fn the_shipped_walk_scripts_parse_and_validate() {
        for script in [
            "../../walks/gameplay_ui.ron",
            "../../walks/multiplayer_session.ron",
            "../../walks/waterfall.ron",
            "../../walks/forest.ron",
            "../../walks/readme_party_trial.ron",
            "../../walks/readme_creator_sandbox.ron",
            "../../walks/vfx_tuner.ron",
        ]
        .into_iter()
        .chain(CAMERA_ROUTE_SCRIPTS.iter().map(|(path, _)| *path))
        .chain(
            AUXILIARY_CAMERA_REVIEW_SCRIPTS
                .iter()
                .map(|(path, _)| *path),
        )
        .chain(std::iter::once(FIRST_PERSON_CAMERA_SCRIPT))
        {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(script);
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
            let steps: Vec<WalkStep> = ron::from_str(&text)
                .unwrap_or_else(|error| panic!("cannot parse {}: {error}", path.display()));
            assert!(!steps.is_empty());
            validate_script_steps(&path.display().to_string(), &steps)
                .unwrap_or_else(|error| panic!("{} invalid: {error}", path.display()));
        }
    }

    #[test]
    fn scoped_gameplay_acceptance_stays_within_the_frame_budget() {
        let steps: Vec<WalkStep> = ron::from_str(include_str!("../../../walks/gameplay_ui.ron"))
            .expect("the gameplay UI walk parses");
        let captures = steps
            .iter()
            .filter(|step| matches!(step, WalkStep::Capture(_) | WalkStep::ReviewCapture { .. }))
            .count();
        let tasks = steps
            .iter()
            .filter_map(|step| match step {
                WalkStep::ReviewCapture { task, .. } => Some(*task),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            captures, 10,
            "scoped gameplay acceptance must capture exactly 10 frames"
        );
        assert_eq!(
            tasks.len(),
            captures,
            "every scoped frame must use a fail-closed task contract"
        );
        let task_ids = tasks
            .iter()
            .map(|task| task.contract().id)
            .collect::<Vec<_>>();
        let expected = [
            "gameplay-exploration",
            "gameplay-player-turn-max",
            "gameplay-hostile-turn",
            "decision-disable",
            "aiming-blocked",
            "gameplay-activity-tabs",
            "gameplay-custom-hud-visibility",
            "gameplay-character-main-view",
            "hud-hidden-required",
            "gameplay-compact-temporary-surface",
        ];
        assert_eq!(
            task_ids, expected,
            "the scoped HUD route must preserve its authored presentation sequence"
        );
    }

    #[test]
    fn forest_walk_pins_the_shipped_hero_seed() {
        let library: ScenarioLibrary =
            ron::from_str(include_str!("../../../assets/config/scenarios.ron"))
                .expect("the shipped scenario library parses");
        let hero_seed = library
            .scenarios
            .iter()
            .find(|scenario| scenario.name == "Forest")
            .and_then(|scenario| scenario.generation_seed)
            .expect("the shipped Forest scenario has a hero seed");
        let steps: Vec<WalkStep> = ron::from_str(include_str!("../../../walks/forest.ron"))
            .expect("the shipped Forest walk parses");

        assert!(steps.contains(&WalkStep::StartScenario {
            name: "Forest".to_owned(),
            seed: Some(hero_seed),
            suppress_hostiles: false,
        }));
    }

    /// Every scenario a shipped walk starts must still exist in `scenarios.ron`.
    ///
    /// Parsing proves a script is well-formed; it does not prove it still points at
    /// anything. `validate_step` only rejects an *empty* name, so renaming a scenario
    /// leaves the walk naming a ghost — and that sails through fmt, clippy, the whole
    /// suite and all six CI jobs, failing only when a person runs the walk by hand.
    /// The wave that added encounters rewrote `scenarios.ron` and got away with it
    /// because it happened to rename nothing.
    #[test]
    fn every_walk_scenario_name_resolves() {
        let library: ScenarioLibrary =
            ron::from_str(include_str!("../../../assets/config/scenarios.ron"))
                .expect("scenarios.ron parses");
        let known: Vec<&str> = library
            .scenarios
            .iter()
            .map(|scenario| scenario.name.as_str())
            .collect();

        let mut checked = 0;
        let mut launches_default = false;
        let mut continues_save = false;
        let mut launches_sandbox = false;
        for script in [
            "../../walks/gameplay_ui.ron",
            "../../walks/multiplayer_session.ron",
            "../../walks/waterfall.ron",
            "../../walks/forest.ron",
            "../../walks/readme_party_trial.ron",
            "../../walks/readme_creator_sandbox.ron",
        ]
        .into_iter()
        .chain(CAMERA_ROUTE_SCRIPTS.iter().map(|(path, _)| *path))
        .chain(
            AUXILIARY_CAMERA_REVIEW_SCRIPTS
                .iter()
                .map(|(path, _)| *path),
        )
        .chain(std::iter::once(FIRST_PERSON_CAMERA_SCRIPT))
        {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(script);
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
            let steps: Vec<WalkStep> = ron::from_str(&text)
                .unwrap_or_else(|error| panic!("cannot parse {}: {error}", path.display()));
            for step in &steps {
                if let WalkStep::StartScenario { name, .. } = step {
                    assert!(
                        known.contains(&name.as_str()),
                        "{} starts {name:?}, which is not in scenarios.ron; it offers {known:?}",
                        path.display(),
                    );
                    checked += 1;
                }
                if matches!(step, WalkStep::Click { name, .. } if name == "New Game") {
                    launches_default = true;
                }
                if matches!(step, WalkStep::Click { name, .. } if name == "Continue") {
                    continues_save = true;
                }
                if matches!(step, WalkStep::Click { name, .. } if name == "Start Sandbox") {
                    launches_sandbox = true;
                }
            }
        }
        assert!(
            checked > 0 || launches_default || continues_save || launches_sandbox,
            "the UI walk must exercise at least one real application launch path"
        );
    }
}
