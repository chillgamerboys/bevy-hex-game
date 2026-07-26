//! Deterministic launch and capture hooks for procedural-map review packs.
//!
//! This module is compiled into runtime builds only with the default-off
//! `map-review` feature. Ordinary release builds neither inspect nor react to the
//! review environment variables. In a review build, setting
//! `HEX_REVIEW_SCENARIO` selects a scenario without automating the title-screen UI;
//! `HEX_REVIEW_SEED` optionally replaces its configured procedural seed. Adding
//! `HEX_REVIEW_CAPTURE` captures the renderer after the validated terrain has settled,
//! then exits. This keeps iteration tooling on the same loading and validation path as
//! manual play while avoiding compositor-dependent screenshots.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::transform::TransformSystems;
use hex_assets::{CameraSettings, Scenario, ScenarioLibrary};
use hex_core::{GameplaySetupFailure, HexTile, ResolvedMapSeed, Screen, TerrainReady};
use hex_world::PanOrbitCamera;

use crate::scenarios::ScenarioToLoad;

const SCENARIO_ENV: &str = "HEX_REVIEW_SCENARIO";
const SEED_ENV: &str = "HEX_REVIEW_SEED";
const CAPTURE_ENV: &str = "HEX_REVIEW_CAPTURE";
const VIEW_ENV: &str = "HEX_REVIEW_VIEW";
const SETTLE_FRAMES: u32 = 90;
const CAPTURE_WIDTH: u32 = 1920;
const CAPTURE_HEIGHT: u32 = 1080;
const CAPTURE_PHASE_TIMEOUT: Duration = Duration::from_secs(60);
const READBACK_TIMEOUT: Duration = Duration::from_secs(30);
const MIN_VISIBLE_TILES: usize = 32;
const MIN_VISIBLE_TILE_PERCENT: usize = 5;
const VARIATION_GRID_COLUMNS: usize = 8;
const VARIATION_GRID_ROWS: usize = 4;
const MIN_VARIANT_PIXEL_PERCENT: usize = 5;
const MIN_VARIED_REGIONS: usize = 8;

/// Installs review automation only when its environment is present.
pub(super) fn plugin(app: &mut App) {
    let request = match ReviewRequest::from_environment() {
        Ok(Some(request)) => request,
        Ok(None) => return,
        Err(error) => {
            app.insert_resource(ReviewConfigurationError(error))
                .add_systems(Startup, reject_invalid_configuration);
            return;
        }
    };

    let capture = request.capture.clone();
    app.insert_resource(request).add_systems(
        Update,
        launch_review_scenario.run_if(in_state(Screen::Title)),
    );

    if let Some(capture) = capture {
        install_capture_systems(app, capture);
    }
}

fn install_capture_systems(app: &mut App, capture: ReviewCapture) {
    app.insert_resource(ReviewCaptureState::new(capture))
        .add_systems(Update, capture_watchdog)
        .add_systems(
            PostUpdate,
            (apply_review_view, capture_settled_frame)
                .chain()
                .before(TransformSystems::Propagate)
                .run_if(in_state(Screen::Gameplay)),
        );
}

#[derive(Resource, Debug)]
struct ReviewConfigurationError(String);

fn reject_invalid_configuration(
    error: Res<ReviewConfigurationError>,
    mut exit: MessageWriter<AppExit>,
) {
    error!("invalid procedural-map review configuration: {}", error.0);
    exit.write(AppExit::error());
}

#[derive(Resource, Debug, Clone)]
struct ReviewRequest {
    scenario: String,
    seed: Option<u64>,
    capture: Option<ReviewCapture>,
    launched: bool,
}

impl ReviewRequest {
    fn from_environment() -> Result<Option<Self>, String> {
        Self::from_values(
            environment_value(SCENARIO_ENV)?,
            environment_value(SEED_ENV)?,
            environment_value(CAPTURE_ENV)?,
            environment_value(VIEW_ENV)?,
        )
    }

    fn from_values(
        scenario: Option<String>,
        seed: Option<String>,
        capture: Option<String>,
        view: Option<String>,
    ) -> Result<Option<Self>, String> {
        let any_value = scenario.is_some() || seed.is_some() || capture.is_some() || view.is_some();
        if !any_value {
            return Ok(None);
        }

        let scenario = nonempty(scenario, SCENARIO_ENV)?;
        let seed = seed
            .map(|value| {
                value
                    .parse::<u64>()
                    .map_err(|error| format!("{SEED_ENV} must be an unsigned integer: {error}"))
            })
            .transpose()?;

        let capture = match capture {
            Some(path) => {
                if path.trim().is_empty() {
                    return Err(format!("{CAPTURE_ENV} must not be empty"));
                }
                let path = PathBuf::from(path);
                if !path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
                {
                    return Err(format!("{CAPTURE_ENV} must name a .png file"));
                }
                Some(ReviewCapture {
                    path,
                    view: ReviewView::parse(view.as_deref().unwrap_or("default"))?,
                })
            }
            None if view.is_some() => {
                return Err(format!("{VIEW_ENV} requires {CAPTURE_ENV}"));
            }
            None => None,
        };

        Ok(Some(Self {
            scenario,
            seed,
            capture,
            launched: false,
        }))
    }
}

fn environment_value(name: &str) -> Result<Option<String>, String> {
    match env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(value)) => Err(format!(
            "{name} is not valid Unicode: {}",
            display_os_string(value)
        )),
    }
}

fn display_os_string(value: OsString) -> String {
    value.to_string_lossy().into_owned()
}

fn nonempty(value: Option<String>, name: &str) -> Result<String, String> {
    match value {
        Some(value) if !value.trim().is_empty() => Ok(value),
        _ => Err(format!(
            "{name} is required when review automation is enabled"
        )),
    }
}

fn launch_review_scenario(
    mut commands: Commands,
    library: Option<Res<ScenarioLibrary>>,
    failure: Option<Res<GameplaySetupFailure>>,
    mut request: ResMut<ReviewRequest>,
    mut next: ResMut<NextState<Screen>>,
    mut exit: MessageWriter<AppExit>,
) {
    if let Some(failure) = failure {
        error!("review scenario setup failed: {}", failure.reason);
        exit.write(AppExit::error());
        return;
    }
    if request.launched {
        return;
    }
    let Some(library) = library else {
        return;
    };
    let scenario = match uniquely_named_scenario(&library, &request.scenario) {
        Ok(scenario) => scenario,
        Err(error) => {
            error!("{error}");
            request.launched = true;
            exit.write(AppExit::error());
            return;
        }
    };

    let resolved_seed = match resolved_review_seed(&scenario, request.seed) {
        Ok(seed) => seed,
        Err(error) => {
            error!("{error}");
            request.launched = true;
            exit.write(AppExit::error());
            return;
        }
    };

    info!(
        "review automation launching scenario {:?} with seed {:?}",
        scenario.name,
        resolved_seed.map(|seed| seed.0)
    );
    commands.insert_resource(ScenarioToLoad {
        scenario,
        resolved_seed,
    });
    request.launched = true;
    next.set(Screen::Loading);
}

fn uniquely_named_scenario(library: &ScenarioLibrary, name: &str) -> Result<Scenario, String> {
    let mut matches = library
        .scenarios
        .iter()
        .filter(|scenario| scenario.name == name);
    let Some(scenario) = matches.next().cloned() else {
        return Err(format!(
            "{SCENARIO_ENV} names {name:?}, which is not in scenarios.ron"
        ));
    };
    if matches.next().is_some() {
        return Err(format!(
            "{SCENARIO_ENV} names {name:?}, which is duplicated in scenarios.ron"
        ));
    }
    Ok(scenario)
}

fn resolved_review_seed(
    scenario: &Scenario,
    requested: Option<u64>,
) -> Result<Option<ResolvedMapSeed>, String> {
    match (scenario.generation_seed, requested) {
        (Some(configured), None) => Ok(Some(ResolvedMapSeed(configured))),
        (Some(_), Some(requested)) => Ok(Some(ResolvedMapSeed(requested))),
        (None, None) => Ok(None),
        (None, Some(_)) => Err(format!(
            "{SEED_ENV} cannot override unseeded scenario {:?}",
            scenario.name
        )),
    }
}

#[derive(Debug, Clone)]
struct ReviewCapture {
    path: PathBuf,
    view: ReviewView,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReviewView {
    Default,
    Rotated,
    TopDown,
}

impl ReviewView {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "default" => Ok(Self::Default),
            "rotated" => Ok(Self::Rotated),
            "top-down" | "top_down" => Ok(Self::TopDown),
            _ => Err(format!(
                "{VIEW_ENV} must be default, rotated, or top-down; got {value:?}"
            )),
        }
    }
}

#[derive(Resource, Debug)]
struct ReviewCaptureState {
    capture: ReviewCapture,
    target: Option<Handle<Image>>,
    view_applied: bool,
    settled_frames: u32,
    requested: bool,
    phase: CapturePhase,
    phase_started: Instant,
    failed: bool,
    visible_tiles: usize,
    total_tiles: usize,
    coverage_warning_logged: bool,
}

impl ReviewCaptureState {
    fn new(capture: ReviewCapture) -> Self {
        Self {
            capture,
            target: None,
            view_applied: false,
            settled_frames: 0,
            requested: false,
            phase: CapturePhase::AwaitingScenario,
            phase_started: Instant::now(),
            failed: false,
            visible_tiles: 0,
            total_tiles: 0,
            coverage_warning_logged: false,
        }
    }

    fn enter_phase(&mut self, phase: CapturePhase, now: Instant) {
        if self.phase != phase {
            self.phase = phase;
            self.phase_started = now;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CapturePhase {
    AwaitingScenario,
    Loading,
    AwaitingCamera,
    AwaitingTerrain,
    Settling,
    Readback,
}

impl CapturePhase {
    const fn timeout(self) -> Duration {
        match self {
            Self::Readback => READBACK_TIMEOUT,
            Self::AwaitingScenario
            | Self::Loading
            | Self::AwaitingCamera
            | Self::AwaitingTerrain
            | Self::Settling => CAPTURE_PHASE_TIMEOUT,
        }
    }

    const fn description(self) -> &'static str {
        match self {
            Self::AwaitingScenario => "scenario launch",
            Self::Loading => "scenario loading",
            Self::AwaitingCamera => "review camera setup",
            Self::AwaitingTerrain => "validated terrain readiness",
            Self::Settling => "terrain visibility and frame settling",
            Self::Readback => "GPU screenshot readback",
        }
    }
}

fn capture_watchdog(
    screen: Res<State<Screen>>,
    ready: Option<Res<TerrainReady>>,
    mut state: ResMut<ReviewCaptureState>,
    mut exit: MessageWriter<AppExit>,
) {
    if state.failed {
        return;
    }

    let now = Instant::now();
    let Some(diagnostic) =
        capture_timeout_diagnostic(&mut state, *screen.get(), ready.is_some(), now)
    else {
        return;
    };
    error!("{diagnostic}");
    state.failed = true;
    exit.write(AppExit::error());
}

fn capture_timeout_diagnostic(
    state: &mut ReviewCaptureState,
    screen: Screen,
    terrain_ready: bool,
    now: Instant,
) -> Option<String> {
    let phase = if state.requested {
        CapturePhase::Readback
    } else {
        match screen {
            Screen::Splash | Screen::Title => CapturePhase::AwaitingScenario,
            Screen::Loading => CapturePhase::Loading,
            Screen::Gameplay if !state.view_applied => CapturePhase::AwaitingCamera,
            Screen::Gameplay if !terrain_ready => CapturePhase::AwaitingTerrain,
            Screen::Gameplay => CapturePhase::Settling,
        }
    };
    state.enter_phase(phase, now);
    if now.duration_since(state.phase_started) < state.phase.timeout() {
        return None;
    }

    Some(format!(
        "procedural-map review timed out during {} after {:.1}s \
         (screen={:?}, visible_tiles={}/{}, requested={})",
        state.phase.description(),
        state.phase.timeout().as_secs_f32(),
        screen,
        state.visible_tiles,
        state.total_tiles,
        state.requested
    ))
}

fn apply_review_view(
    mut state: ResMut<ReviewCaptureState>,
    settings: Res<CameraSettings>,
    mut images: ResMut<Assets<Image>>,
    mut camera: Query<(&mut Transform, &mut PanOrbitCamera, &mut RenderTarget)>,
    mut exit: MessageWriter<AppExit>,
) {
    if state.failed {
        return;
    }
    let Ok((mut transform, mut orbit, mut target)) = camera.single_mut() else {
        return;
    };

    if state.target.is_none() {
        let image = Image::new_target_texture(
            CAPTURE_WIDTH,
            CAPTURE_HEIGHT,
            TextureFormat::Rgba8UnormSrgb,
            None,
        );
        let handle = images.add(image);
        *target = RenderTarget::Image(handle.clone().into());
        state.target = Some(handle);
    }

    let eye = Vec3::from_array([
        settings.gameplay_eye.0,
        settings.gameplay_eye.1,
        settings.gameplay_eye.2,
    ]);
    let focus = Vec3::from_array([
        settings.gameplay_focus.0,
        settings.gameplay_focus.1,
        settings.gameplay_focus.2,
    ]);
    if let Err(error) =
        apply_camera_view(state.capture.view, eye, focus, &mut transform, &mut orbit)
    {
        error!("invalid procedural-map review camera pose: {error}");
        state.failed = true;
        exit.write(AppExit::error());
        return;
    }
    state.view_applied = true;
}

fn apply_camera_view(
    view: ReviewView,
    eye: Vec3,
    focus: Vec3,
    transform: &mut Transform,
    orbit: &mut PanOrbitCamera,
) -> Result<(), &'static str> {
    let offset = eye - focus;
    if !eye.is_finite() || !focus.is_finite() {
        return Err("eye and focus must be finite");
    }
    if offset.length_squared() <= f32::EPSILON {
        return Err("eye and focus must be distinct");
    }
    let (eye, up) = match view {
        ReviewView::Default => (eye, camera_up(eye, focus)),
        ReviewView::Rotated => (
            focus + Quat::from_rotation_y(2.0 * std::f32::consts::PI / 3.0) * offset,
            camera_up(
                focus + Quat::from_rotation_y(2.0 * std::f32::consts::PI / 3.0) * offset,
                focus,
            ),
        ),
        ReviewView::TopDown => (focus + Vec3::Y * offset.length(), Vec3::NEG_Z),
    };
    transform.translation = eye;
    transform.look_at(focus, up);
    orbit.focus = focus;
    orbit.radius = offset.length();
    Ok(())
}

fn camera_up(eye: Vec3, focus: Vec3) -> Vec3 {
    let direction = (focus - eye).normalize();
    if direction.cross(Vec3::Y).length_squared() <= f32::EPSILON {
        Vec3::Z
    } else {
        Vec3::Y
    }
}

fn capture_settled_frame(
    mut commands: Commands,
    ready: Option<Res<TerrainReady>>,
    mut state: ResMut<ReviewCaptureState>,
    tiles: Query<&ViewVisibility, With<HexTile>>,
    mut exit: MessageWriter<AppExit>,
) {
    if state.failed || state.requested || !state.view_applied || ready.is_none() {
        return;
    }
    state.settled_frames += 1;
    if state.settled_frames < SETTLE_FRAMES {
        return;
    }

    state.total_tiles = tiles.iter().count();
    state.visible_tiles = tiles.iter().filter(|visibility| visibility.get()).count();
    if !has_visible_tile_coverage(state.visible_tiles, state.total_tiles) {
        if !state.coverage_warning_logged {
            warn!(
                "review capture is waiting for visible terrain: {}/{} HexTile entities visible",
                state.visible_tiles, state.total_tiles
            );
            state.coverage_warning_logged = true;
        }
        return;
    }

    if let Err(error) = prepare_capture_path(&state.capture.path) {
        error!(
            "cannot prepare review screenshot {}: {error}",
            state.capture.path.display()
        );
        state.failed = true;
        exit.write(AppExit::error());
        return;
    }

    let Some(target) = state.target.clone() else {
        return;
    };
    let output = state.capture.path.clone();
    let observer_output = output.clone();
    commands.spawn(Screenshot::image(target)).observe(
        move |captured: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| {
            let result = persist_screenshot(&captured.image, &observer_output);
            match result {
                Ok(()) => {
                    info!("review screenshot completed: {}", observer_output.display());
                    exit.write(AppExit::Success);
                }
                Err(error) => {
                    error!(
                        "review screenshot failed for {}: {error}",
                        observer_output.display()
                    );
                    exit.write(AppExit::error());
                }
            }
        },
    );
    state.requested = true;
    state.enter_phase(CapturePhase::Readback, Instant::now());
    info!("requested review screenshot: {}", output.display());
}

fn has_visible_tile_coverage(visible: usize, total: usize) -> bool {
    total > 0
        && visible >= MIN_VISIBLE_TILES
        && visible.saturating_mul(100) >= total.saturating_mul(MIN_VISIBLE_TILE_PERCENT)
}

fn prepare_capture_path(path: &Path) -> std::io::Result<()> {
    if path.file_name().is_none() {
        return Err(std::io::Error::new(
            ErrorKind::InvalidInput,
            "capture path must name a file",
        ));
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    match fs::remove_file(temporary_capture_path(path)?) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn persist_screenshot(image: &Image, path: &Path) -> Result<(), String> {
    if image.width() != CAPTURE_WIDTH || image.height() != CAPTURE_HEIGHT {
        return Err(format!(
            "renderer output is {}x{}; expected {CAPTURE_WIDTH}x{CAPTURE_HEIGHT}",
            image.width(),
            image.height()
        ));
    }
    let dynamic = image
        .clone()
        .try_into_dynamic()
        .map_err(|error| format!("cannot convert renderer output: {error}"))?;
    let rgb = dynamic.to_rgb8();
    let has_coverage = has_visual_coverage(
        rgb.as_raw(),
        usize::try_from(rgb.width()).unwrap_or_default(),
        usize::try_from(rgb.height()).unwrap_or_default(),
    );

    prepare_capture_path(path)
        .map_err(|error| format!("cannot prepare staged screenshot output: {error}"))?;
    let temporary = temporary_capture_path(path)
        .map_err(|error| format!("cannot prepare staged screenshot output: {error}"))?;
    if let Err(error) = rgb.save(&temporary) {
        let _cleanup = fs::remove_file(&temporary);
        return Err(format!("cannot write temporary PNG: {error}"));
    }
    if let Err(error) = install_capture(&temporary, path) {
        let _cleanup = fs::remove_file(&temporary);
        return Err(format!("cannot install PNG: {error}"));
    }
    if !has_coverage {
        return Err(
            "renderer output lacks meaningful visual coverage; rejected PNG was preserved"
                .to_owned(),
        );
    }
    Ok(())
}

/// Installs a complete temporary PNG, including over an existing Windows file.
///
/// `std::fs::rename` replaces files on Unix but rejects an existing destination on
/// Windows. Removing the old destination only after the new PNG is fully written
/// keeps the capture path portable without exposing a partially encoded image.
fn install_capture(temporary: &Path, path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    fs::rename(temporary, path)
}

fn temporary_capture_path(path: &Path) -> std::io::Result<PathBuf> {
    let Some(file_name) = path.file_name() else {
        return Err(std::io::Error::new(
            ErrorKind::InvalidInput,
            "capture path must name a file",
        ));
    };
    let mut temporary_name = OsString::from(".");
    temporary_name.push(file_name);
    temporary_name.push(format!(".{}.tmp.png", std::process::id()));
    Ok(path.with_file_name(temporary_name))
}

fn has_visual_coverage(bytes: &[u8], width: usize, height: usize) -> bool {
    let Some(pixel_count) = width.checked_mul(height) else {
        return false;
    };
    if pixel_count == 0 || bytes.len() != pixel_count.saturating_mul(3) {
        return false;
    }

    let region_count = VARIATION_GRID_COLUMNS.saturating_mul(VARIATION_GRID_ROWS);
    let mut region_minimums = vec![[u8::MAX; 3]; region_count];
    let mut region_maximums = vec![[u8::MIN; 3]; region_count];
    let mut histogram = vec![0_usize; 16 * 16 * 16];
    let mut brightest = u8::MIN;

    for (index, pixel) in bytes.chunks_exact(3).enumerate() {
        let &[red, green, blue] = pixel else {
            return false;
        };
        brightest = brightest.max(red).max(green).max(blue);
        let bin =
            usize::from(red >> 4) * 16 * 16 + usize::from(green >> 4) * 16 + usize::from(blue >> 4);
        let Some(count) = histogram.get_mut(bin) else {
            return false;
        };
        *count = count.saturating_add(1);

        let x = index % width;
        let y = index / width;
        let region_x = x.saturating_mul(VARIATION_GRID_COLUMNS) / width;
        let region_y = y.saturating_mul(VARIATION_GRID_ROWS) / height;
        let region = region_y
            .saturating_mul(VARIATION_GRID_COLUMNS)
            .saturating_add(region_x)
            .min(region_count.saturating_sub(1));
        let Some((minimums, maximums)) = region_minimums
            .get_mut(region)
            .zip(region_maximums.get_mut(region))
        else {
            return false;
        };
        for ((minimum, maximum), value) in minimums
            .iter_mut()
            .zip(maximums.iter_mut())
            .zip([red, green, blue])
        {
            *minimum = (*minimum).min(value);
            *maximum = (*maximum).max(value);
        }
    }

    let dominant = histogram.into_iter().max().unwrap_or(pixel_count);
    let variant_pixels = pixel_count.saturating_sub(dominant);
    let varied_regions = region_minimums
        .iter()
        .zip(region_maximums.iter())
        .filter(|(minimum, maximum)| {
            minimum
                .iter()
                .zip(maximum.iter())
                .any(|(low, high)| high.abs_diff(*low) > 12)
        })
        .count();

    brightest > 8
        && variant_pixels.saturating_mul(100)
            >= pixel_count.saturating_mul(MIN_VARIANT_PIXEL_PERCENT)
        && varied_regions >= MIN_VARIED_REGIONS
}

#[cfg(test)]
mod tests {
    use bevy::state::app::StatesPlugin;
    use hex_assets::{CubeCoord, ScenarioPlacement, ScenarioSettings};

    use super::*;

    fn scenario(seed: Option<u64>) -> Scenario {
        Scenario {
            name: "Test".to_owned(),
            blurb: "A test scenario.".to_owned(),
            world: "config/world.ron".to_owned(),
            lighting: "config/lighting.ron".to_owned(),
            generation_seed: seed,
            units: ScenarioSettings {
                player: ScenarioPlacement::Fixed(CubeCoord { x: 0, y: 0, z: 0 }),
                enemy: ScenarioPlacement::Fixed(CubeCoord { x: 1, y: -1, z: 0 }),
            },
        }
    }

    #[test]
    fn review_automation_is_dormant_without_environment_values() {
        assert!(ReviewRequest::from_values(None, None, None, None)
            .expect("empty review configuration should be valid")
            .is_none());
    }

    #[test]
    fn capture_configuration_parses_an_exact_seed_and_view() {
        let request = ReviewRequest::from_values(
            Some("Procedural Hills".to_owned()),
            Some("42".to_owned()),
            Some(".context/review.png".to_owned()),
            Some("top-down".to_owned()),
        )
        .expect("valid review configuration should parse")
        .expect("review configuration should be enabled");

        assert_eq!(request.scenario, "Procedural Hills");
        assert_eq!(request.seed, Some(42));
        let capture = request.capture.expect("capture should be configured");
        assert_eq!(capture.path, PathBuf::from(".context/review.png"));
        assert_eq!(capture.view, ReviewView::TopDown);
    }

    #[test]
    fn view_without_capture_is_rejected() {
        let error = ReviewRequest::from_values(
            Some("Procedural Hills".to_owned()),
            None,
            None,
            Some("rotated".to_owned()),
        )
        .expect_err("a camera view without an output should be invalid");

        assert!(error.contains(CAPTURE_ENV));
    }

    #[test]
    fn configured_and_overridden_scenario_seeds_resolve_explicitly() {
        assert_eq!(
            resolved_review_seed(&scenario(Some(7)), None),
            Ok(Some(ResolvedMapSeed(7)))
        );
        assert_eq!(
            resolved_review_seed(&scenario(Some(7)), Some(11)),
            Ok(Some(ResolvedMapSeed(11)))
        );
        assert!(resolved_review_seed(&scenario(None), Some(11)).is_err());
    }

    #[test]
    fn automated_launch_requires_one_exact_scenario_name() {
        let only = scenario(Some(7));
        let library = ScenarioLibrary {
            scenarios: vec![only.clone()],
        };
        assert_eq!(
            uniquely_named_scenario(&library, "Test")
                .expect("one exact match should be selectable")
                .generation_seed,
            Some(7)
        );
        assert!(uniquely_named_scenario(&library, "Missing").is_err());

        let duplicated = ScenarioLibrary {
            scenarios: vec![only.clone(), only],
        };
        assert!(uniquely_named_scenario(&duplicated, "Test").is_err());
    }

    #[test]
    fn review_views_have_exact_deterministic_poses() {
        let focus = Vec3::new(1.0, 2.0, 3.0);
        let eye = focus + Vec3::new(0.0, 4.0, 3.0);
        let offset = eye - focus;
        let rotated_eye = focus + Quat::from_rotation_y(2.0 * std::f32::consts::PI / 3.0) * offset;
        let top_down_eye = focus + Vec3::Y * offset.length();
        for (view, expected_eye, expected_up) in [
            (ReviewView::Default, eye, camera_up(eye, focus)),
            (
                ReviewView::Rotated,
                rotated_eye,
                camera_up(rotated_eye, focus),
            ),
            (ReviewView::TopDown, top_down_eye, Vec3::NEG_Z),
        ] {
            let mut transform = Transform::from_translation(eye);
            transform.look_at(focus, Vec3::Y);
            let mut orbit = PanOrbitCamera::default();

            apply_camera_view(view, eye, focus, &mut transform, &mut orbit)
                .expect("the deterministic review pose should be valid");

            assert!(transform.translation.distance(expected_eye) < 0.0001);
            assert!((transform.translation.distance(focus) - orbit.radius).abs() < 0.0001);
            assert!(orbit.focus.distance(focus) < 0.0001);
            let forward = transform.rotation * Vec3::NEG_Z;
            assert!(forward.dot((focus - transform.translation).normalize()) > 0.999);
            let expected_rotation =
                Transform::from_translation(expected_eye).looking_at(focus, expected_up);
            assert!(
                (transform.rotation * Vec3::Y).dot(expected_rotation.rotation * Vec3::Y) > 0.9999,
                "{view:?} changed its exact screen-up orientation"
            );
        }
    }

    #[test]
    fn invalid_review_camera_poses_are_rejected_without_mutation() {
        let original = Transform::from_xyz(4.0, 5.0, 6.0);
        for (eye, focus) in [
            (Vec3::NAN, Vec3::ZERO),
            (Vec3::ZERO, Vec3::INFINITY),
            (Vec3::ONE, Vec3::ONE),
        ] {
            let mut transform = original;
            let mut orbit = PanOrbitCamera::default();
            assert!(
                apply_camera_view(ReviewView::Default, eye, focus, &mut transform, &mut orbit)
                    .is_err()
            );
            assert_eq!(transform, original);
        }
    }

    #[test]
    fn weak_sky_and_sparse_noise_are_rejected() {
        let (width, height) = (80, 40);
        let mut sky = vec![0_u8; width * height * 3];
        for pixel in sky.chunks_exact_mut(3) {
            pixel.copy_from_slice(&[105, 185, 230]);
        }
        assert!(!has_visual_coverage(&sky, width, height));

        let mut sparse_noise = sky.clone();
        for pixel in sparse_noise
            .chunks_exact_mut(3)
            .step_by(width.saturating_add(1))
            .take(8)
        {
            pixel.copy_from_slice(&[250, 250, 250]);
        }
        assert!(!has_visual_coverage(&sparse_noise, width, height));

        let mut covered = sky;
        for (index, pixel) in covered.chunks_exact_mut(3).enumerate() {
            let x = index % width;
            let y = index / width;
            if (x / 4 + y / 4) % 2 == 0 {
                pixel.copy_from_slice(&[45, 120, 55]);
            }
        }
        assert!(has_visual_coverage(&covered, width, height));
    }

    #[test]
    fn watchdog_covers_loading_readiness_and_readback_phases() {
        let now = Instant::now();
        for (phase, screen, terrain_ready, requested, expected) in [
            (
                CapturePhase::Loading,
                Screen::Loading,
                false,
                false,
                "scenario loading",
            ),
            (
                CapturePhase::AwaitingTerrain,
                Screen::Gameplay,
                false,
                false,
                "terrain readiness",
            ),
            (
                CapturePhase::Readback,
                Screen::Gameplay,
                true,
                true,
                "GPU screenshot readback",
            ),
        ] {
            let mut state = ReviewCaptureState::new(ReviewCapture {
                path: PathBuf::from("capture.png"),
                view: ReviewView::Default,
            });
            state.view_applied = phase != CapturePhase::AwaitingCamera;
            state.requested = requested;
            state.phase = phase;
            state.phase_started = now
                .checked_sub(phase.timeout() + Duration::from_millis(1))
                .expect("the test deadline should fit before now");

            let diagnostic = capture_timeout_diagnostic(&mut state, screen, terrain_ready, now)
                .expect("the expired phase should time out");
            assert!(diagnostic.contains(expected), "{diagnostic}");
        }
    }

    #[test]
    fn production_capture_schedule_retains_target_and_requests_once() {
        let directory = review_test_directory("schedule");
        let path = directory.join("capture.png");
        let _cleanup = fs::remove_dir_all(&directory);
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin));
        app.init_state::<Screen>();
        app.insert_resource(test_camera_settings());
        app.insert_resource(Assets::<Image>::default());
        install_capture_systems(
            &mut app,
            ReviewCapture {
                path: path.clone(),
                view: ReviewView::Default,
            },
        );
        let camera = app
            .world_mut()
            .spawn((
                Transform::default(),
                PanOrbitCamera::default(),
                RenderTarget::default(),
            ))
            .id();
        for _ in 0..MIN_VISIBLE_TILES {
            app.world_mut().spawn((HexTile, ViewVisibility::VISIBLE));
        }
        app.insert_resource(TerrainReady);
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Gameplay);
        app.update();

        let target = app
            .world()
            .resource::<ReviewCaptureState>()
            .target
            .clone()
            .expect("the production system should retain the off-screen target");
        assert_eq!(
            app.world()
                .entity(camera)
                .get::<RenderTarget>()
                .and_then(RenderTarget::as_image),
            Some(&target)
        );
        let image = app
            .world()
            .resource::<Assets<Image>>()
            .get(&target)
            .expect("the retained target should remain in Assets<Image>");
        assert_eq!(
            (image.width(), image.height()),
            (CAPTURE_WIDTH, CAPTURE_HEIGHT)
        );

        for _ in 0..=SETTLE_FRAMES {
            if app.world().resource::<ReviewCaptureState>().requested {
                break;
            }
            app.update();
        }
        assert!(
            app.world().resource::<ReviewCaptureState>().requested,
            "the production schedule did not request a screenshot after settling"
        );
        let mut screenshots = app.world_mut().query_filtered::<Entity, With<Screenshot>>();
        assert_eq!(screenshots.iter(app.world()).count(), 1);

        app.update();
        let mut screenshots = app.world_mut().query_filtered::<Entity, With<Screenshot>>();
        assert_eq!(
            screenshots.iter(app.world()).count(),
            1,
            "a pending asynchronous capture must not be requested twice"
        );
        assert!(!path.exists());
        let _cleanup = fs::remove_dir_all(directory);
    }

    #[test]
    fn failed_visual_validation_still_persists_the_rejected_png() {
        let directory = review_test_directory("atomic-failure");
        let path = directory.join("capture.png");
        let _cleanup = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).expect("the test output directory should be creatable");
        let uniform = Image::new_target_texture(
            CAPTURE_WIDTH,
            CAPTURE_HEIGHT,
            TextureFormat::Rgba8UnormSrgb,
            None,
        );
        fs::write(&path, b"previous capture").expect("the existing capture should be writable");

        let error = persist_screenshot(&uniform, &path)
            .expect_err("a uniform renderer output should fail visual validation");
        assert!(
            error.contains("rejected PNG was preserved"),
            "unexpected validation error: {error}"
        );
        let png = fs::read(&path).expect("the rejected capture should remain readable");
        assert!(
            png.starts_with(b"\x89PNG\r\n\x1a\n"),
            "the rejected capture was not persisted as a PNG"
        );
        assert!(!temporary_capture_path(&path)
            .expect("the temporary path should be valid")
            .exists());
        let _cleanup = fs::remove_dir_all(directory);
    }

    #[test]
    fn visible_tile_gate_requires_meaningful_coverage() {
        assert!(!has_visible_tile_coverage(0, 0));
        assert!(!has_visible_tile_coverage(MIN_VISIBLE_TILES - 1, 1_000));
        assert!(!has_visible_tile_coverage(MIN_VISIBLE_TILES, 1_000));
        assert!(has_visible_tile_coverage(50, 1_000));
    }

    fn review_test_directory(label: &str) -> PathBuf {
        env::temp_dir().join(format!(
            "hex-game-review-test-{}-{label}",
            std::process::id()
        ))
    }

    fn test_camera_settings() -> CameraSettings {
        CameraSettings {
            gameplay_eye: (0.0, 48.0, 42.0),
            gameplay_focus: (0.0, 6.0, 0.0),
            pan_speed: 0.4,
            pan_speed_offset: 10.0,
            min_pitch: 0.25,
            max_pitch: 0.95,
            min_zoom: 5.0,
            max_zoom: 70.0,
            zoom_sensitivity: 0.2,
        }
    }
}
