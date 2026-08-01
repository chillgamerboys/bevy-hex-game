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
use hex_assets::ScenarioLibrary;
use hex_core::{
    Busy, CommandQueue, GameplaySetupFailure, Headroom, HexCoord, HexTile, ResolvedMapSeed, Screen,
    TilePos,
};
use hex_units::{MovingTo, Party, UnitRegistry};
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
    if env::var_os(UI_DEBUG_ENV).is_some() {
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
    app.insert_resource(WalkState::new(steps, PathBuf::from(out), viewport))
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
    /// Wait for every registered party member's domain movement to finish.
    ///
    /// The script owns the frame limit so a stalled route fails deterministically
    /// instead of relying only on the runner's wall-clock watchdog.
    AwaitPartyIdle { max_frames: u32 },
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
    /// Press and release a supported gameplay or menu key.
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
                    | "required-decision"
                    | "aiming-disabled"
                    | "live-statistics"
                    | "dense-report-compare"
            ) =>
        {
            Err(format!("unknown presentation-only UI fixture {name:?}"))
        }
        WalkStep::SetViewport {
            width,
            height,
            device_scale,
        } => hex_ui::ReviewViewport::new(*width, *height, *device_scale).map(|_| ()),
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
        "Scenarios" => Ok(Screen::Scenarios),
        "Settings" => Ok(Screen::Settings),
        "LatticeDemo" => Ok(Screen::LatticeDemo),
        "CharacterCreator" => Ok(Screen::CharacterCreator),
        "SpellCreator" => Ok(Screen::SpellCreator),
        "CombatLab" => Ok(Screen::CombatLab),
        "Loading" => Ok(Screen::Loading),
        "Gameplay" => Ok(Screen::Gameplay),
        _ => Err(format!(
            "unknown screen {name:?}; expected Splash, Title, Scenarios, Settings, CharacterCreator, SpellCreator, CombatLab, LatticeDemo, Loading, or Gameplay"
        )),
    }
}

fn parse_key(name: &str) -> Result<KeyCode, String> {
    match name {
        "Backspace" => Ok(KeyCode::Backspace),
        "C" => Ok(KeyCode::KeyC),
        "H" => Ok(KeyCode::KeyH),
        _ => Err(format!("unknown key {name:?}; expected Backspace, C, or H")),
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
    /// The Bevy image target the game and UI render into for capture.
    target: Option<Handle<Image>>,
    /// The camera entity the UI roots must be pointed at.
    camera: Option<Entity>,
    /// Exact logical canvas and raster density under review.
    viewport: hex_ui::ReviewViewport,
    failed: bool,
}

#[derive(Component)]
struct WalkUiCamera;

#[derive(SystemParam)]
struct WalkContent<'w, 's> {
    failure: Option<Res<'w, GameplaySetupFailure>>,
    library: Option<Res<'w, ScenarioLibrary>>,
    party: Option<Res<'w, Party>>,
    registry: Option<Res<'w, UnitRegistry>>,
    queue: Option<Res<'w, CommandQueue>>,
    movement: Query<'w, 's, (Has<Busy>, Has<MovingTo>)>,
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
}

impl WalkState {
    fn new(steps: Vec<WalkStep>, out_dir: PathBuf, viewport: hex_ui::ReviewViewport) -> Self {
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
            viewport,
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
    content: WalkContent,
    mut ui_scale: ResMut<hex_ui::UiScalePreference>,
    mut primary_window: Query<(Entity, &mut Window), With<bevy::window::PrimaryWindow>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    buttons: Query<(Entity, &Name), With<Button>>,
    tiles: Query<(Entity, &TilePos, &Headroom), With<HexTile>>,
    mut images: ResMut<Assets<Image>>,
    mut game_camera: Query<&mut RenderTarget, (With<Camera3d>, Without<WalkUiCamera>)>,
    mut review_camera: Query<(Entity, &mut RenderTarget), (With<WalkUiCamera>, Without<Camera3d>)>,
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
        let Ok(mut game_target) = game_camera.single_mut() else {
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
        let ui_camera = if let Ok((camera, mut target)) = review_camera.single_mut() {
            *target = render_target.clone();
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
                    render_target,
                ))
                .id()
        };
        state.target = Some(handle);
        state.camera = Some(ui_camera);
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
        WalkStep::Settle(frames) => {
            state.settled += 1;
            if state.settled >= frames {
                state.advance();
            }
        }
        WalkStep::Capture(ref name) => {
            if !state.capture_requested {
                let Some(snapshot) = latest_ui_tree.0.as_ref() else {
                    return;
                };
                let issues = snapshot.layout_issues();
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
                state.target = None;
                state.advance();
            }
            Err(error) => {
                error!("visual walk viewport is invalid: {error}");
                state.failed = true;
                exit.write(AppExit::error());
            }
        },
        WalkStep::Key(ref name) => {
            let key = parse_key(name).unwrap_or(KeyCode::Escape);
            info!("visual walk pressing {name}");
            keys.press(key);
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

    const FULL_SCRIPT: &str = r#"[
        AwaitScreen("Title"),
        Settle(30),
        Capture("01-title"),
        Click(name: "Combat Lab"),
        AwaitScreen("CombatLab"),
        Key("Backspace"),
        StartScenario(name: "The Crossing"),
        AwaitTerrain,
        ClickTile(q: 2, r: -2),
        ClickTile(q: 2, r: -2, level: Some(7)),
        AwaitPartyIdle(max_frames: 600),
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
        assert_eq!(steps.len(), 16);
        assert_eq!(steps.first(), Some(&WalkStep::AwaitScreen("Title".into())));
        assert_eq!(
            steps.get(3),
            Some(&WalkStep::Click {
                name: "Combat Lab".into(),
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
            Some(&WalkStep::AwaitPartyIdle { max_frames: 600 })
        );
        for step in &steps {
            validate_step(step).expect("every step validates");
        }
    }

    #[test]
    fn unknown_screens_and_keys_are_rejected_at_load() {
        assert_eq!(parse_key("C"), Ok(KeyCode::KeyC));
        assert_eq!(parse_key("H"), Ok(KeyCode::KeyH));
        assert!(validate_step(&WalkStep::AwaitScreen("Menu".into())).is_err());
        assert!(validate_step(&WalkStep::Key("F13".into())).is_err());
        assert!(validate_step(&WalkStep::Capture(" ".into())).is_err());
        assert!(validate_step(&WalkStep::Click {
            name: String::new(),
            index: 0
        })
        .is_err());
        assert!(validate_step(&WalkStep::AwaitButton(" ".into())).is_err());
        assert!(validate_step(&WalkStep::AwaitPartyIdle { max_frames: 0 }).is_err());
        validate_step(&WalkStep::AwaitPartyIdle { max_frames: 1 })
            .expect("a positive frame bound is valid");
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
    fn every_screen_name_round_trips() {
        for name in [
            "Splash",
            "Title",
            "Scenarios",
            "CharacterCreator",
            "CombatLab",
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
            "../../walks/readme_creator_lab.ron",
        ] {
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
        let captures = [include_str!("../../../walks/gameplay_ui.ron")]
            .into_iter()
            .map(|script| {
                ron::from_str::<Vec<WalkStep>>(script)
                    .expect("the gameplay UI walk parses")
                    .into_iter()
                    .filter(|step| matches!(step, WalkStep::Capture(_)))
                    .count()
            })
            .sum::<usize>();
        assert_eq!(
            captures, 10,
            "scoped gameplay acceptance must capture exactly 10 frames"
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
        let mut launches_lab_sandbox = false;
        for script in ["../../walks/gameplay_ui.ron"] {
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
                if matches!(step, WalkStep::Click { name, .. } if name == "Load Map & Deploy") {
                    launches_lab_sandbox = true;
                }
            }
        }
        assert!(
            checked > 0 || launches_default || continues_save || launches_lab_sandbox,
            "the UI walk must exercise at least one real application launch path"
        );
    }
}
