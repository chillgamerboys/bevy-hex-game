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

use std::env;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bevy::camera::{ClearColorConfig, ImageRenderTarget, NormalizedRenderTarget, RenderTarget};
use bevy::ecs::system::SystemParam;
use bevy::input::InputSystems;
use bevy::picking::backend::HitData;
use bevy::picking::events::{Click, Pointer};
use bevy::picking::pointer::{Location, PointerButton, PointerId};
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::window::CursorMoved;
#[cfg(test)]
use hex_assets::SandboxMapCatalog;
use hex_assets::ScenarioLibrary;
use hex_core::{
    Busy, CameraFocusTarget, CommandQueue, GameplaySetupFailure, Headroom, HexCoord, HexTile,
    MapAnchorId, MapAnchors, ResolvedMapSeed, Screen, TilePos,
};
use hex_units::{MovingTo, Party, Selected, StandsOn, UnitRegistry};
use serde::Deserialize;

use crate::capture::write_png;
use crate::scenarios::ScenarioToLoad;

const SCRIPT_ENV: &str = "HEX_WALK_SCRIPT";
const OUT_ENV: &str = "HEX_WALK_OUT";
const VIEWPORT_ENV: &str = "HEX_WALK_VIEWPORT";
const UI_DEBUG_ENV: &str = "HEX_WALK_UI_DEBUG";
const DATA_ENV: &str = "HEX_GAME_DATA_DIR";
const STEP_TIMEOUT: Duration = Duration::from_secs(60);
const WALK_TIME_SCALE: f32 = 12.0;
const MAX_ORBIT_YAW_TURNS: f32 = 0.5;
const MAX_ORBIT_PITCH_FRACTION: f32 = 1.0;
/// Full render frames allowed after both cameras move to a fresh image target.
///
/// Four frames let the asynchronous UI glyph atlas settle on Metal. Two frames
/// occasionally captured a complete 3D pass with only part of the UI text uploaded.
const CAPTURE_TARGET_SETTLE_FRAMES: u8 = 4;

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
    app.insert_resource(WalkState::new(
        steps,
        PathBuf::from(out),
        viewport,
        diagnostic_overlays,
    ))
    .add_systems(Startup, accelerate_walk_time)
    .add_systems(PreUpdate, run_walk.after(InputSystems));
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
    /// Click one generated anchor through the same stack-safe picking path.
    ///
    /// `expected` deliberately duplicates the current hero-seed projection. The
    /// anchor remains the authority, while a moved anchor makes old visual evidence
    /// fail stale instead of silently reviewing a different place.
    ClickAnchor {
        name: String,
        expected: CameraRouteTile,
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
    /// Install an exact internal scenario as a review-only launch input.
    StartScenario {
        name: String,
        #[serde(default)]
        seed: Option<u64>,
    },
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
    for (index, step) in steps.iter().enumerate() {
        validate_step(step).map_err(|error| format!("{path} step {index}: {error}"))?;
    }
    Ok(steps)
}

fn validate_step(step: &WalkStep) -> Result<(), String> {
    match step {
        WalkStep::AwaitScreen(name) => parse_screen(name).map(|_| ()),
        WalkStep::Key(name) => parse_key(name).map(|_| ()),
        WalkStep::Capture(name) | WalkStep::ReviewCapture { name, .. }
            if name.trim().is_empty() =>
        {
            Err("capture name must not be empty".to_owned())
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
        "Sandbox" => Ok(Screen::Sandbox),
        "Settings" => Ok(Screen::Settings),
        "LatticeDemo" => Ok(Screen::LatticeDemo),
        "CharacterCreator" => Ok(Screen::CharacterCreator),
        "SpellCreator" => Ok(Screen::SpellCreator),
        "Loading" => Ok(Screen::Loading),
        "Gameplay" => Ok(Screen::Gameplay),
        _ => Err(format!(
            "unknown screen {name:?}; expected Splash, Title, Sandbox, Settings, CharacterCreator, SpellCreator, LatticeDemo, Loading, or Gameplay"
        )),
    }
}

fn parse_key(name: &str) -> Result<KeyCode, String> {
    match name {
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

/// What the capture observer reports back to the runner.
#[derive(Debug)]
enum CaptureOutcome {
    Written { brightest: u8, coverage: bool },
    Failed(String),
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
    settled: u32,
    step_started: Instant,
    capture_requested: bool,
    capture_outcome: Option<CaptureOutcome>,
    /// A button pressed by the previous step, to be reset to `None`.
    pressed: Option<Entity>,
    /// A key pressed by the previous step, to be released.
    held_key: Option<KeyCode>,
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
    library: Option<Res<'w, ScenarioLibrary>>,
    anchors: Option<Res<'w, MapAnchors>>,
    party: Option<Res<'w, Party>>,
    registry: Option<Res<'w, UnitRegistry>>,
    queue: Option<Res<'w, CommandQueue>>,
    movement: Query<'w, 's, (Has<Busy>, Has<MovingTo>)>,
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
            let Ok((busy, moving)) = self.movement.get(entity) else {
                return None;
            };
            if busy || moving {
                return Some(false);
            }
        }
        Some(true)
    }

    fn assert_selected_at(&self, expected: TilePos) -> Result<(), String> {
        let (entity, standing, focus) = self.selected.single().map_err(|error| {
            format!("visual walk needs exactly one selected unit before position proof: {error}")
        })?;
        if standing.0.pos != expected {
            return Err(format!(
                "selected unit {entity:?} stands at {:?}, not expected {expected:?}",
                standing.0.pos
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
    ) -> Self {
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
            Screen::CharacterCreator | Screen::SpellCreator => &["Creator Screen"],
            Screen::Sandbox => &["Sandbox"],
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
    mut state: ResMut<WalkState>,
    screen: Res<State<Screen>>,
    mut next: ResMut<NextState<Screen>>,
    content: WalkContent,
    mut ui_scale: ResMut<hex_ui::UiScalePreference>,
    mut primary_window: Query<(Entity, &mut Window), With<bevy::window::PrimaryWindow>>,
    mut input: WalkInput,
    buttons: Query<(Entity, &Name), With<Button>>,
    tiles: Query<(Entity, &TilePos, &Headroom), With<HexTile>>,
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
            return;
        };
        let Ok(physical_size) = state.viewport.physical_size() else {
            error!("visual walk viewport became invalid");
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
            return;
        }
    }
    if let Some(failure) = content.failure.as_deref() {
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
        input.keys.release(key);
    }

    let Some(step) = state.steps.get(state.cursor).cloned() else {
        info!("visual walk complete: {} steps", state.steps.len());
        input.mouse.release(MouseButton::Right);
        exit.write(AppExit::Success);
        state.failed = true;
        return;
    };
    let review_task = match &step {
        WalkStep::ReviewCapture { task, .. } => Some(*task),
        _ => None,
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
            if !tiles.is_empty() {
                state.advance();
            }
        }
        WalkStep::AwaitPartyIdle { max_frames } => {
            state.settled = state.settled.saturating_add(1);
            if content.party_is_idle() == Some(true) {
                state.advance();
            } else if state.settled >= max_frames {
                error!(
                    "visual walk exhausted AwaitPartyIdle after {max_frames} frames; party facts: {:?}",
                    content.party_is_idle()
                );
                state.failed = true;
                exit.write(AppExit::error());
            }
        }
        WalkStep::AssertSelectedAt { expected } => {
            let expected = expected.position();
            match content.assert_selected_at(expected) {
                Ok(()) => {
                    info!("visual walk proved selected unit and camera focus at {expected:?}");
                    state.advance();
                }
                Err(reason) => {
                    error!("visual walk rejected position evidence: {reason}");
                    state.failed = true;
                    exit.write(AppExit::error());
                }
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
        WalkStep::ClickTile { q, r, level } => {
            let coord = HexCoord::from_axial(q, r);
            match resolve_tile_click_target(tiles.iter(), coord, level) {
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
        WalkStep::ClickAnchor { ref name, expected } => {
            let Some(anchors) = content.anchors.as_deref() else {
                return;
            };
            let id = MapAnchorId::from(name.as_str());
            let Some(actual) = anchors.get(&id) else {
                error!("visual walk anchor {name:?} is not published by this map");
                state.failed = true;
                exit.write(AppExit::error());
                return;
            };
            let expected = expected.position();
            if actual != expected {
                error!(
                    "visual walk anchor {name:?} moved from expected {expected:?} to {actual:?}; \
                     recapture and review the route before updating its stale detector"
                );
                state.failed = true;
                exit.write(AppExit::error());
                return;
            }
            match resolve_tile_click_target(tiles.iter(), actual.coord, Some(actual.level)) {
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
                    info!(
                        "visual walk clicking anchor {name:?} at {pos:?} through pointer picking"
                    );
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
        WalkStep::AwaitButton(ref name) => {
            if buttons
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
        WalkStep::Key(ref name) => {
            let key = parse_key(name).unwrap_or(KeyCode::Escape);
            info!("visual walk pressing {name}");
            input.keys.press(key);
            state.held_key = Some(key);
            state.advance();
        }
        WalkStep::StartScenario { ref name, seed } => {
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
        ("../../walks/camera_fort.ron", "Fort"),
        ("../../walks/camera_seven_regions.ron", "Seven Regions"),
        ("../../walks/camera_two_rings.ron", "Two Rings"),
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

    fn record_pointer_click(click: On<Pointer<Click>>, mut record: ResMut<PointerRecord>) {
        record.target = Some(click.event_target());
        record.primary = click.button == PointerButton::Primary;
    }

    #[derive(Resource, Default, Debug, PartialEq, Eq)]
    struct PartyIdleRecord(Option<bool>);

    fn record_party_idle(content: WalkContent, mut record: ResMut<PartyIdleRecord>) {
        record.0 = content.party_is_idle();
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
        ClickAnchor(name: "bridge", expected: (q: 0, r: 0, level: 16)),
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
    fn a_full_script_parses_with_every_step_kind() {
        let steps: Vec<WalkStep> = ron::from_str(FULL_SCRIPT).expect("script parses");
        assert_eq!(steps.len(), 19);
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
                seed: None
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
            steps.get(11),
            Some(&WalkStep::AwaitPartyIdle { max_frames: 600 })
        );
        assert_eq!(
            steps.get(12),
            Some(&WalkStep::AssertSelectedAt {
                expected: CameraRouteTile {
                    q: 0,
                    r: 0,
                    level: 16,
                },
            })
        );
        assert_eq!(
            steps.get(13),
            Some(&WalkStep::OrbitCamera {
                yaw_turns: 0.33333334,
                pitch_fraction: -0.1,
            })
        );
        for step in &steps {
            validate_step(step).expect("every step validates");
        }
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
        assert!(validate_step(&WalkStep::Key("F13".into())).is_err());
        assert!(validate_step(&WalkStep::Capture(" ".into())).is_err());
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
        assert!(validate_step(&WalkStep::ClickAnchor {
            name: " ".to_owned(),
            expected: CameraRouteTile {
                q: 0,
                r: 0,
                level: 1,
            },
        })
        .is_err());
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
        let mut state = WalkState::new(
            steps,
            PathBuf::from("captures"),
            hex_ui::ReviewViewport::DEFAULT,
            false,
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
        assert_eq!(routes.len(), 15);

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
                    WalkStep::StartScenario { name, seed } => Some((name.as_str(), *seed)),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(launches, vec![(scenario_name, route.seed)]);
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
                        let clicked = pending_proof.take().unwrap_or_else(|| {
                            panic!(
                                "{} proves a position without a pending movement",
                                path.display()
                            )
                        });
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
                    WalkStep::StartScenario { name, seed } => Some((name.as_str(), *seed)),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert!(
                !launches.is_empty(),
                "{} has no exact Two Rings launch",
                path.display()
            );
            assert!(
                launches
                    .iter()
                    .all(|(name, seed)| *name == "Two Rings" && *seed == route.seed),
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
            "Sandbox",
            "Settings",
            "CharacterCreator",
            "SpellCreator",
            "LatticeDemo",
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
            "../../walks/waterfall.ron",
            "../../walks/forest.ron",
            "../../walks/readme_party_trial.ron",
            "../../walks/readme_creator_sandbox.ron",
        ]
        .into_iter()
        .chain(CAMERA_ROUTE_SCRIPTS.iter().map(|(path, _)| *path))
        {
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
            "../../walks/waterfall.ron",
            "../../walks/forest.ron",
            "../../walks/readme_party_trial.ron",
            "../../walks/readme_creator_sandbox.ron",
        ]
        .into_iter()
        .chain(CAMERA_ROUTE_SCRIPTS.iter().map(|(path, _)| *path))
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
