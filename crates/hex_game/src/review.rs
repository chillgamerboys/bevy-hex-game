//! Deterministic launch and capture hooks for procedural-map review packs.
//!
//! This module is compiled into runtime builds only with the default-off
//! `map-review` feature. Ordinary release builds neither inspect nor react to the
//! review environment variables. In a review build, setting
//! `HEX_REVIEW_SCENARIO` selects a scenario without automating the title-screen UI;
//! `HEX_REVIEW_SEED` optionally replaces its configured procedural seed. Adding
//! `HEX_REVIEW_CAPTURE` captures the renderer after the validated terrain has settled,
//! then exits. `HEX_REVIEW_TIME` and `HEX_REVIEW_CAMERA` optionally select the cyclic
//! lighting hour and map/character/first-person perspective for that launch.
//! `HEX_REVIEW_LIQUID_PHASE` freezes liquid presentation at a deterministic phase;
//! captures default to phase `0.0` when no explicit phase is configured.
//! `HEX_REVIEW_FOCUS_ANCHOR` optionally relocates the selected actor to one exact
//! generated anchor before framing. This keeps iteration tooling on the same loading
//! and validation path as manual play while avoiding compositor-dependent screenshots.
//! `HEX_REVIEW_CUTAWAY=full` exposes the complete active interior for overview
//! captures; ordinary gameplay keeps every authored roof or enclosing shell intact.
//! `HEX_REVIEW_ILLUMINATION=overlay` draws the authoritative Dark, Dim, and Bright
//! gameplay tiers over exact interior surfaces for diagnostic captures.

use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bevy::camera::RenderTarget;
use bevy::light::NotShadowCaster;
use bevy::picking::Pickable;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::transform::TransformSystems;
use hex_assets::{CameraSettings, GameAssets, Scenario, ScenarioLibrary, SubstanceTable};
use hex_core::{
    CameraFocusTarget, CutawayOccluder, GameplaySetupFailure, Headroom, HexSpan, HexTile,
    IlluminationLevel, LightDomain, MapAnchorId, MapAnchors, MapViewHint, PresentationOcclusion,
    ResolvedMapSeed, Screen, SubstanceId, TerrainReady, TilePos, TraversalBlockers,
};
use hex_map::LiquidVisualTime;
use hex_perception::ResolvedIllumination;
use hex_units::{Body, Footing, Selected, Standing, StandsOn};
use hex_world::{CameraMode, CameraSystems, PanOrbitCamera};

use crate::capture::{prepare_capture_path, write_png};
use crate::fog::{FOG_CAP_DEPTH_BIAS, FOG_CAP_INSET, FOG_CAP_LIFT, FOG_CAP_THICKNESS};
use crate::scenarios::ScenarioToLoad;

const SCENARIO_ENV: &str = "HEX_REVIEW_SCENARIO";
const SEED_ENV: &str = "HEX_REVIEW_SEED";
const CAPTURE_ENV: &str = "HEX_REVIEW_CAPTURE";
const VIEW_ENV: &str = "HEX_REVIEW_VIEW";
const TIME_ENV: &str = "HEX_REVIEW_TIME";
const LIQUID_PHASE_ENV: &str = "HEX_REVIEW_LIQUID_PHASE";
const CAMERA_ENV: &str = "HEX_REVIEW_CAMERA";
const FOCUS_ANCHOR_ENV: &str = "HEX_REVIEW_FOCUS_ANCHOR";
const CUTAWAY_ENV: &str = "HEX_REVIEW_CUTAWAY";
const ILLUMINATION_ENV: &str = "HEX_REVIEW_ILLUMINATION";
const SETTLE_FRAMES: u32 = 90;
const CAPTURE_WIDTH: u32 = 1920;
const CAPTURE_HEIGHT: u32 = 1080;
const CAPTURE_PHASE_TIMEOUT: Duration = Duration::from_secs(60);
const READBACK_TIMEOUT: Duration = Duration::from_secs(30);
const MIN_VISIBLE_TILES: usize = 32;
const MIN_VISIBLE_TILE_PERCENT: usize = 5;
const ILLUMINATION_CAP_THICKNESS: f32 = 0.02;
const ILLUMINATION_CAP_INSET: f32 = FOG_CAP_INSET;
/// Physical separation above the complete tactical-fog prism.
///
/// The air gap is the authoritative non-coplanarity guarantee when OIT is active;
/// the material depth bias below also keeps the ordinary transparent pass ordered.
const ILLUMINATION_CAP_CLEARANCE: f32 = 0.02;
const ILLUMINATION_CAP_LIFT: f32 = FOG_CAP_LIFT + FOG_CAP_THICKNESS + ILLUMINATION_CAP_CLEARANCE;
/// Sorts the translucent diagnostic after tactical fog as well as above it.
const ILLUMINATION_CAP_DEPTH_BIAS: f32 = FOG_CAP_DEPTH_BIAS + 8.0;

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
    if let Some(time) = request
        .liquid_phase_seconds
        .and_then(LiquidVisualTime::frozen_at)
    {
        app.insert_resource(time);
    }
    app.insert_resource(request).add_systems(
        Update,
        launch_review_scenario.run_if(in_state(Screen::Title)),
    );

    if let Some(capture) = capture {
        install_capture_systems(app, capture);
    }
}

fn install_capture_systems(app: &mut App, capture: ReviewCapture) {
    if capture.full_cutaway {
        hex_world::install_full_cutaway_review_override(app);
    }
    app.insert_resource(ReviewCaptureState::new(capture))
        .add_systems(Update, capture_watchdog)
        .add_systems(
            PostUpdate,
            (
                relocate_review_focus,
                apply_review_view,
                apply_review_illumination_overlay,
                capture_settled_frame,
            )
                .chain()
                .before(CameraSystems::FollowCharacter)
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
    time_hours: Option<f32>,
    liquid_phase_seconds: Option<f32>,
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
            environment_value(TIME_ENV)?,
            environment_value(LIQUID_PHASE_ENV)?,
            environment_value(CAMERA_ENV)?,
            environment_value(FOCUS_ANCHOR_ENV)?,
            environment_value(CUTAWAY_ENV)?,
            environment_value(ILLUMINATION_ENV)?,
        )
    }

    fn from_values(
        scenario: Option<String>,
        seed: Option<String>,
        capture: Option<String>,
        view: Option<String>,
        time: Option<String>,
        liquid_phase: Option<String>,
        camera: Option<String>,
        focus_anchor: Option<String>,
        cutaway: Option<String>,
        illumination: Option<String>,
    ) -> Result<Option<Self>, String> {
        let any_value = scenario.is_some()
            || seed.is_some()
            || capture.is_some()
            || view.is_some()
            || time.is_some()
            || liquid_phase.is_some()
            || camera.is_some()
            || focus_anchor.is_some()
            || cutaway.is_some()
            || illumination.is_some();
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
        let time_hours = time.map(|value| parse_review_hour(&value)).transpose()?;
        let liquid_phase_seconds = liquid_phase
            .map(|value| parse_liquid_phase(&value))
            .transpose()?;
        let focus_anchor = match focus_anchor {
            Some(value) if value.trim().is_empty() => {
                return Err(format!("{FOCUS_ANCHOR_ENV} must not be empty"));
            }
            value => value,
        };

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
                    camera: ReviewCamera::parse(camera.as_deref().unwrap_or("map"))?,
                    focus_anchor,
                    full_cutaway: parse_review_cutaway(cutaway.as_deref())?,
                    illumination_overlay: parse_review_illumination(illumination.as_deref())?,
                })
            }
            None if view.is_some()
                || camera.is_some()
                || focus_anchor.is_some()
                || cutaway.is_some()
                || illumination.is_some() =>
            {
                let dependent = if view.is_some() {
                    VIEW_ENV
                } else if camera.is_some() {
                    CAMERA_ENV
                } else if focus_anchor.is_some() {
                    FOCUS_ANCHOR_ENV
                } else if cutaway.is_some() {
                    CUTAWAY_ENV
                } else {
                    ILLUMINATION_ENV
                };
                return Err(format!("{dependent} requires {CAPTURE_ENV}"));
            }
            None => None,
        };

        // A capture must be byte-reproducible, so freeze liquid presentation at phase
        // zero unless the launch names an explicit phase. Launches without a capture
        // keep the live animation.
        let liquid_phase_seconds =
            liquid_phase_seconds.or_else(|| capture.is_some().then_some(0.0));

        Ok(Some(Self {
            scenario,
            seed,
            time_hours,
            liquid_phase_seconds,
            capture,
            launched: false,
        }))
    }
}

fn parse_review_cutaway(value: Option<&str>) -> Result<bool, String> {
    match value {
        None => Ok(false),
        Some("full") => Ok(true),
        Some(value) => Err(format!("{CUTAWAY_ENV} must be full; got {value:?}")),
    }
}

fn parse_review_illumination(value: Option<&str>) -> Result<bool, String> {
    match value {
        None => Ok(false),
        Some("overlay") => Ok(true),
        Some(value) => Err(format!("{ILLUMINATION_ENV} must be overlay; got {value:?}")),
    }
}

fn parse_review_hour(value: &str) -> Result<f32, String> {
    let hours = value
        .parse::<f32>()
        .map_err(|error| format!("{TIME_ENV} must be a number in [0, 24): {error}"))?;
    if !hours.is_finite() || !(0.0..24.0).contains(&hours) {
        return Err(format!(
            "{TIME_ENV} must be finite and in [0, 24); got {value:?}"
        ));
    }
    Ok(hours)
}

fn parse_liquid_phase(value: &str) -> Result<f32, String> {
    let phase = value
        .parse::<f32>()
        .map_err(|error| format!("{LIQUID_PHASE_ENV} must be a finite number: {error}"))?;
    if !phase.is_finite() {
        return Err(format!("{LIQUID_PHASE_ENV} must be finite; got {value:?}"));
    }
    Ok(phase)
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
    let mut scenario = match uniquely_named_scenario(&library, &request.scenario) {
        Ok(scenario) => scenario,
        Err(error) => {
            error!("{error}");
            request.launched = true;
            exit.write(AppExit::error());
            return;
        }
    };
    apply_review_time_override(&mut scenario, request.time_hours);

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
        encounter_override: None,
    });
    request.launched = true;
    next.set(Screen::Loading);
}

fn apply_review_time_override(scenario: &mut Scenario, requested: Option<f32>) {
    if let Some(hours) = requested {
        scenario.starting_time_hours = Some(hours);
    }
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
    camera: ReviewCamera,
    focus_anchor: Option<String>,
    full_cutaway: bool,
    illumination_overlay: bool,
}

/// Marks deterministic gameplay-illumination caps owned by capture tooling.
#[derive(Component)]
struct ReviewIlluminationOverlay;

#[derive(Debug, Clone, Copy, PartialEq)]
struct ReviewIlluminationSurface {
    position: TilePos,
    span: HexSpan,
    level: IlluminationLevel,
    cutaway: Option<CutawayOccluder>,
}

struct ReviewIlluminationMaterials {
    dark: Handle<StandardMaterial>,
    dim: Handle<StandardMaterial>,
    bright: Handle<StandardMaterial>,
}

impl ReviewIlluminationMaterials {
    fn for_level(&self, level: IlluminationLevel) -> Handle<StandardMaterial> {
        match level {
            IlluminationLevel::Dark => self.dark.clone(),
            IlluminationLevel::Dim => self.dim.clone(),
            IlluminationLevel::Bright => self.bright.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReviewView {
    Default,
    Rotated,
    CounterRotated,
    Rear,
    TopDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReviewCamera {
    Map,
    Character,
    FirstPerson,
}

impl ReviewCamera {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "map" => Ok(Self::Map),
            "character" => Ok(Self::Character),
            "first-person" => Ok(Self::FirstPerson),
            _ => Err(format!(
                "{CAMERA_ENV} must be map, character, or first-person; got {value:?}"
            )),
        }
    }
}

impl ReviewView {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "default" => Ok(Self::Default),
            "rotated" => Ok(Self::Rotated),
            "counter-rotated" | "counter_rotated" => Ok(Self::CounterRotated),
            "rear" => Ok(Self::Rear),
            "top-down" | "top_down" => Ok(Self::TopDown),
            _ => Err(format!(
                "{VIEW_ENV} must be default, rotated, counter-rotated, rear, or top-down; got {value:?}"
            )),
        }
    }
}

#[derive(Resource, Debug)]
struct ReviewCaptureState {
    capture: ReviewCapture,
    target: Option<Handle<Image>>,
    focus_relocated: bool,
    view_applied: bool,
    illumination_overlay_applied: bool,
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
        let focus_relocated = capture.focus_anchor.is_none();
        let illumination_overlay_applied = !capture.illumination_overlay;
        Self {
            capture,
            target: None,
            focus_relocated,
            view_applied: false,
            illumination_overlay_applied,
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
            Screen::Splash
            | Screen::Title
            | Screen::Settings
            | Screen::LatticeDemo
            | Screen::VfxTuner
            | Screen::CharacterCreator
            | Screen::SpellCreator
            | Screen::Sandbox
            | Screen::Multiplayer => CapturePhase::AwaitingScenario,
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

type ReviewTileQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static TilePos,
        &'static HexSpan,
        &'static SubstanceId,
        &'static Headroom,
    ),
    With<HexTile>,
>;

/// Relocates the selected actor before the requested camera framing is applied.
fn relocate_review_focus(
    mut state: ResMut<ReviewCaptureState>,
    ready: Option<Res<TerrainReady>>,
    anchors: Option<Res<MapAnchors>>,
    table: Option<Res<SubstanceTable>>,
    blockers: Option<Res<TraversalBlockers>>,
    tiles: ReviewTileQuery,
    mut selected: Query<
        (&Body, &mut StandsOn, &mut Transform, &mut CameraFocusTarget),
        With<Selected>,
    >,
    mut exit: MessageWriter<AppExit>,
) {
    if state.failed || state.focus_relocated {
        return;
    }
    let Some(anchor_name) = state.capture.focus_anchor.as_deref() else {
        state.focus_relocated = true;
        return;
    };
    if ready.is_none() {
        return;
    }
    let (Some(anchors), Some(table)) = (anchors, table) else {
        return;
    };
    let Ok((body, mut standing, mut transform, mut focus)) = selected.single_mut() else {
        return;
    };

    let destination = resolve_review_focus(
        anchor_name,
        &anchors,
        &table,
        *body,
        blockers.as_deref(),
        tiles.iter(),
    );
    let destination = match destination {
        Ok(destination) => destination,
        Err(error) => {
            error!("review focus override failed: {error}");
            state.failed = true;
            exit.write(AppExit::error());
            return;
        }
    };

    standing.0 = destination;
    transform.translation = destination.world_position();
    focus.surface = destination.pos;
    info!(
        "relocated review focus to generated anchor {:?} at {:?}",
        anchor_name, destination.pos
    );
    state.focus_relocated = true;
}

fn resolve_review_focus<'a>(
    anchor_name: &str,
    anchors: &MapAnchors,
    table: &SubstanceTable,
    body: Body,
    blockers: Option<&TraversalBlockers>,
    tiles: impl Iterator<Item = (&'a TilePos, &'a HexSpan, &'a SubstanceId, &'a Headroom)>,
) -> Result<Standing, String> {
    let anchor = MapAnchorId::from(anchor_name);
    let Some(position) = anchors.get(&anchor) else {
        return Err(format!(
            "{FOCUS_ANCHOR_ENV} names {anchor_name:?}, which the generated map did not publish"
        ));
    };
    let footing = Footing::from_tiles(tiles, table, body, blockers);
    footing.at(position).ok_or_else(|| {
        format!(
            "{FOCUS_ANCHOR_ENV} anchor {anchor_name:?} resolves to {position:?}, \
             which the selected actor cannot stand on"
        )
    })
}

fn apply_review_view(
    mut state: ResMut<ReviewCaptureState>,
    settings: Res<CameraSettings>,
    hint: Option<Res<MapViewHint>>,
    mut images: ResMut<Assets<Image>>,
    targets: Query<&Transform, (With<CameraFocusTarget>, Without<PanOrbitCamera>)>,
    mut camera: Query<
        (
            &mut Transform,
            &mut PanOrbitCamera,
            &mut RenderTarget,
            Option<&mut Projection>,
        ),
        Without<CameraFocusTarget>,
    >,
    mut mode: ResMut<CameraMode>,
    mut exit: MessageWriter<AppExit>,
) {
    if state.failed || state.view_applied || !state.focus_relocated {
        return;
    }
    let Ok((mut transform, mut orbit, mut target, mut projection)) = camera.single_mut() else {
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

    let fallback_eye = Vec3::from_array([
        settings.gameplay_eye.0,
        settings.gameplay_eye.1,
        settings.gameplay_eye.2,
    ]);
    let fallback_focus = Vec3::from_array([
        settings.gameplay_focus.0,
        settings.gameplay_focus.1,
        settings.gameplay_focus.2,
    ]);
    let (eye, focus) = hint.as_deref().filter(|hint| hint.is_valid()).map_or(
        (fallback_eye, fallback_focus),
        |hint| {
            (
                Vec3::new(hint.eye.0, hint.eye.1, hint.eye.2),
                Vec3::new(hint.focus.0, hint.focus.1, hint.focus.2),
            )
        },
    );
    if let Err(error) =
        apply_camera_view(state.capture.view, eye, focus, &mut transform, &mut orbit)
    {
        error!("invalid procedural-map review camera pose: {error}");
        state.failed = true;
        exit.write(AppExit::error());
        return;
    }
    match state.capture.camera {
        ReviewCamera::Map => *mode = CameraMode::Map,
        ReviewCamera::Character => {
            let Ok(target) = targets.single() else {
                return;
            };
            apply_character_camera_view(
                eye,
                focus,
                target.translation,
                &settings,
                &mut transform,
                &mut orbit,
            );
            *mode = CameraMode::Character;
        }
        ReviewCamera::FirstPerson => {
            let Ok(target) = targets.single() else {
                return;
            };
            apply_first_person_camera_view(
                eye,
                focus,
                target.translation,
                &settings,
                &mut transform,
                &mut orbit,
            );
            if let Some(Projection::Perspective(perspective)) = projection.as_deref_mut() {
                perspective.fov = settings.first_person_fov_degrees.to_radians();
            }
            *mode = CameraMode::FirstPerson;
        }
    }
    state.view_applied = true;
}

/// Converts the deterministic map pose into a close pose without changing its azimuth.
fn apply_character_camera_view(
    map_eye: Vec3,
    map_focus: Vec3,
    target: Vec3,
    settings: &CameraSettings,
    transform: &mut Transform,
    orbit: &mut PanOrbitCamera,
) {
    let map_offset = transform.translation - orbit.focus;
    let original_offset = map_eye - map_focus;
    let horizontal = Vec3::new(map_offset.x, 0.0, map_offset.z)
        .try_normalize()
        .or_else(|| Vec3::new(original_offset.x, 0.0, original_offset.z).try_normalize())
        .unwrap_or(Vec3::Z);
    let pitch = settings.character_pitch * std::f32::consts::FRAC_PI_2;
    let focus = target + Vec3::Y * settings.character_focus_height;
    let offset = horizontal * (settings.character_radius * pitch.cos())
        + Vec3::Y * (settings.character_radius * pitch.sin());
    let direction = offset.normalize_or_zero();
    let up = if direction.cross(Vec3::Y).length_squared() <= f32::EPSILON {
        Vec3::Z
    } else {
        Vec3::Y
    };

    transform.translation = focus + offset;
    transform.look_at(focus, up);
    orbit.focus = focus;
    orbit.radius = settings.character_radius;
}

/// Converts the deterministic map azimuth into an eye-level in-place view.
fn apply_first_person_camera_view(
    map_eye: Vec3,
    map_focus: Vec3,
    target: Vec3,
    settings: &CameraSettings,
    transform: &mut Transform,
    orbit: &mut PanOrbitCamera,
) {
    let map_offset = transform.translation - orbit.focus;
    let original_offset = map_eye - map_focus;
    let backward = Vec3::new(map_offset.x, 0.0, map_offset.z)
        .try_normalize()
        .or_else(|| Vec3::new(original_offset.x, 0.0, original_offset.z).try_normalize())
        .unwrap_or(Vec3::Z);
    let pitch = settings.first_person_pitch * std::f32::consts::FRAC_PI_2;
    let forward = -backward * pitch.cos() + Vec3::NEG_Y * pitch.sin();
    let eye = target + Vec3::Y * settings.first_person_eye_height;
    let focus = eye + forward;
    let up = if forward.cross(Vec3::Y).length_squared() <= f32::EPSILON {
        Vec3::Z
    } else {
        Vec3::Y
    };

    transform.translation = eye;
    transform.look_at(focus, up);
    orbit.focus = focus;
    orbit.radius = 1.0;
}

type ReviewIlluminationTileQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static TilePos,
        &'static HexSpan,
        Option<&'static CutawayOccluder>,
    ),
    With<HexTile>,
>;

/// Draws one unlit cap per authoritative exact-interior illumination result.
///
/// This is deliberately a review-only projection. It reads the headless gameplay
/// result and never feeds a renderer value back into perception. Copying a roof
/// run's cutaway marker lets the existing composable cutaway system hide the cap
/// together with its source geometry.
fn apply_review_illumination_overlay(
    mut commands: Commands,
    ready: Option<Res<TerrainReady>>,
    illumination: Option<Res<ResolvedIllumination>>,
    game_assets: Option<Res<GameAssets>>,
    mut materials: Option<ResMut<Assets<StandardMaterial>>>,
    tiles: ReviewIlluminationTileQuery,
    mut state: ResMut<ReviewCaptureState>,
    mut exit: MessageWriter<AppExit>,
) {
    if state.failed || state.illumination_overlay_applied || !state.view_applied || ready.is_none()
    {
        return;
    }
    let (Some(illumination), Some(game_assets), Some(materials)) =
        (illumination, game_assets, materials.as_mut())
    else {
        return;
    };

    let surfaces = match collect_review_illumination_surfaces(&illumination, tiles.iter()) {
        Ok(surfaces) => surfaces,
        Err(error) => {
            error!("review illumination overlay failed: {error}");
            state.failed = true;
            exit.write(AppExit::error());
            return;
        }
    };
    let overlay_materials = ReviewIlluminationMaterials {
        dark: materials.add(review_illumination_material(IlluminationLevel::Dark)),
        dim: materials.add(review_illumination_material(IlluminationLevel::Dim)),
        bright: materials.add(review_illumination_material(IlluminationLevel::Bright)),
    };
    let mut counts = [0usize; 3];
    for surface in surfaces {
        match surface.level {
            IlluminationLevel::Dark => counts[0] += 1,
            IlluminationLevel::Dim => counts[1] += 1,
            IlluminationLevel::Bright => counts[2] += 1,
        }
        let mut overlay = commands.spawn((
            Mesh3d(game_assets.hex_tile.clone()),
            MeshMaterial3d(overlay_materials.for_level(surface.level)),
            review_illumination_transform(surface.position, surface.span),
            Pickable::IGNORE,
            NotShadowCaster,
            ReviewIlluminationOverlay,
            surface.position,
            Name::new("ReviewIlluminationOverlay"),
        ));
        if let Some(cutaway) = surface.cutaway {
            overlay.insert((cutaway, PresentationOcclusion::default()));
        }
    }
    state.illumination_overlay_applied = true;
    info!(
        "applied review illumination overlay: dark={}, dim={}, bright={}",
        counts[0], counts[1], counts[2]
    );
}

fn collect_review_illumination_surfaces<'a>(
    illumination: &ResolvedIllumination,
    tiles: impl Iterator<Item = (&'a TilePos, &'a HexSpan, Option<&'a CutawayOccluder>)>,
) -> Result<Vec<ReviewIlluminationSurface>, String> {
    let mut by_position = BTreeMap::new();
    for (position, span, cutaway) in tiles {
        if by_position
            .insert(*position, (*span, cutaway.copied()))
            .is_some()
        {
            return Err(format!(
                "multiple rendered HexTile entities project the exact surface {position:?}"
            ));
        }
    }

    let mut surfaces = Vec::with_capacity(illumination.len());
    for (position, resolved) in illumination.iter() {
        if !matches!(resolved.domain, LightDomain::Interior(_)) {
            continue;
        }
        let Some((span, cutaway)) = by_position.get(&position).copied() else {
            return Err(format!(
                "authoritative illumination names {position:?}, but no rendered HexTile projects it"
            ));
        };
        surfaces.push(ReviewIlluminationSurface {
            position,
            span,
            level: resolved.level,
            cutaway,
        });
    }
    if surfaces.is_empty() {
        return Err("authoritative illumination contains no interior exact surfaces".to_owned());
    }
    Ok(surfaces)
}

fn review_illumination_transform(position: TilePos, span: HexSpan) -> Transform {
    Transform {
        translation: position
            .coord
            .to_world(span.top + ILLUMINATION_CAP_LIFT + ILLUMINATION_CAP_THICKNESS * 0.5),
        scale: Vec3::new(
            ILLUMINATION_CAP_INSET,
            ILLUMINATION_CAP_THICKNESS,
            ILLUMINATION_CAP_INSET,
        ),
        ..default()
    }
}

fn review_illumination_material(level: IlluminationLevel) -> StandardMaterial {
    let color = match level {
        IlluminationLevel::Dark => Color::srgba(0.03, 0.04, 0.10, 0.72),
        IlluminationLevel::Dim => Color::srgba(0.24, 0.38, 0.88, 0.58),
        IlluminationLevel::Bright => Color::srgba(0.22, 0.96, 0.78, 0.48),
    };
    StandardMaterial {
        base_color: color,
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        depth_bias: ILLUMINATION_CAP_DEPTH_BIAS,
        ..default()
    }
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
        ReviewView::CounterRotated => (
            focus + Quat::from_rotation_y(-2.0 * std::f32::consts::PI / 3.0) * offset,
            camera_up(
                focus + Quat::from_rotation_y(-2.0 * std::f32::consts::PI / 3.0) * offset,
                focus,
            ),
        ),
        ReviewView::Rear => {
            let eye = focus + Quat::from_rotation_y(std::f32::consts::PI) * offset;
            (eye, camera_up(eye, focus))
        }
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
    if state.failed
        || state.requested
        || !state.view_applied
        || !state.illumination_overlay_applied
        || ready.is_none()
    {
        return;
    }
    state.settled_frames += 1;
    if state.settled_frames < SETTLE_FRAMES {
        return;
    }

    state.total_tiles = tiles.iter().count();
    state.visible_tiles = tiles.iter().filter(|visibility| visibility.get()).count();
    if !has_visible_tile_coverage(state.capture.camera, state.visible_tiles, state.total_tiles) {
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

fn has_visible_tile_coverage(camera: ReviewCamera, visible: usize, total: usize) -> bool {
    if total == 0 || visible < MIN_VISIBLE_TILES {
        return false;
    }
    camera == ReviewCamera::FirstPerson
        || visible.saturating_mul(100) >= total.saturating_mul(MIN_VISIBLE_TILE_PERCENT)
}

/// Persists a review frame under review policy: the exact configured target
/// size, and full visual coverage — a blank or near-uniform frame is a failed
/// capture even though its PNG is preserved for inspection.
fn persist_screenshot(image: &Image, path: &Path) -> Result<(), String> {
    if image.width() != CAPTURE_WIDTH || image.height() != CAPTURE_HEIGHT {
        return Err(format!(
            "renderer output is {}x{}; expected {CAPTURE_WIDTH}x{CAPTURE_HEIGHT}",
            image.width(),
            image.height()
        ));
    }
    let stats = write_png(image, path)?;
    if stats.brightest <= 8 {
        return Err("renderer output is effectively black; rejected PNG was preserved".to_owned());
    }
    if !stats.has_coverage {
        return Err(
            "renderer output lacks meaningful visual coverage; rejected PNG was preserved"
                .to_owned(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use bevy::state::app::StatesPlugin;
    use hex_assets::{ArtPalette, ScenarioCategory, Substance, SubstanceFile, SwatchId};
    use hex_core::{
        ExteriorIllumination, GameplayLight, HexCoord, InteriorRegionId, TraversalProfile,
    };
    use hex_perception::LightSourceSnapshot;

    use crate::capture::{has_visual_coverage, temporary_capture_path};

    use super::*;

    #[derive(Resource, Default)]
    struct CharacterFollowObservation {
        saw_review_pose: bool,
    }

    fn observe_review_pose_before_character_follow(
        mode: Res<CameraMode>,
        settings: Res<CameraSettings>,
        cameras: Query<&PanOrbitCamera>,
        mut observation: ResMut<CharacterFollowObservation>,
    ) {
        let Ok(camera) = cameras.single() else {
            return;
        };
        if *mode == CameraMode::Character
            && (camera.radius - settings.character_radius).abs() < f32::EPSILON
        {
            observation.saw_review_pose = true;
        }
    }

    fn scenario(seed: Option<u64>) -> Scenario {
        Scenario {
            name: "Test".to_owned(),
            category: ScenarioCategory::Demo,
            blurb: "A test scenario.".to_owned(),
            world: "config/world.ron".to_owned(),
            lighting: "config/lighting.ron".to_owned(),
            generation_seed: seed,
            starting_time_hours: None,
            encounter: "config/encounters/bridge-crossing.ron".to_owned(),
        }
    }

    #[test]
    fn review_automation_is_dormant_without_environment_values() {
        assert!(ReviewRequest::from_values(
            None, None, None, None, None, None, None, None, None, None,
        )
        .expect("empty review configuration should be valid")
        .is_none());
    }

    #[test]
    fn capture_configuration_parses_all_deterministic_overrides() {
        let request = ReviewRequest::from_values(
            Some("Procedural Hills".to_owned()),
            Some("42".to_owned()),
            Some(".context/review.png".to_owned()),
            Some("top-down".to_owned()),
            Some("18.5".to_owned()),
            Some("0.5".to_owned()),
            Some("character".to_owned()),
            Some("deep_chamber".to_owned()),
            Some("full".to_owned()),
            Some("overlay".to_owned()),
        )
        .expect("valid review configuration should parse")
        .expect("review configuration should be enabled");

        assert_eq!(request.scenario, "Procedural Hills");
        assert_eq!(request.seed, Some(42));
        assert_eq!(request.time_hours, Some(18.5));
        assert_eq!(request.liquid_phase_seconds, Some(0.5));
        let capture = request.capture.expect("capture should be configured");
        assert_eq!(capture.path, PathBuf::from(".context/review.png"));
        assert_eq!(capture.view, ReviewView::TopDown);
        assert_eq!(capture.camera, ReviewCamera::Character);
        assert_eq!(capture.focus_anchor.as_deref(), Some("deep_chamber"));
        assert!(capture.full_cutaway);
        assert!(capture.illumination_overlay);
    }

    #[test]
    fn captures_default_to_a_frozen_liquid_phase_and_launches_do_not() {
        let capture = ReviewRequest::from_values(
            Some("Procedural Hills".to_owned()),
            None,
            Some(".context/review.png".to_owned()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("capture configuration should parse")
        .expect("capture configuration should be enabled");
        assert_eq!(capture.liquid_phase_seconds, Some(0.0));

        let launch = ReviewRequest::from_values(
            Some("Procedural Hills".to_owned()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("launch configuration should parse")
        .expect("launch configuration should be enabled");
        assert_eq!(launch.liquid_phase_seconds, None);
    }

    #[test]
    fn view_without_capture_is_rejected() {
        let error = ReviewRequest::from_values(
            Some("Procedural Hills".to_owned()),
            None,
            None,
            Some("rotated".to_owned()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect_err("a camera view without an output should be invalid");

        assert!(error.contains(CAPTURE_ENV));
    }

    #[test]
    fn camera_without_capture_is_rejected() {
        let error = ReviewRequest::from_values(
            Some("Procedural Hills".to_owned()),
            None,
            None,
            None,
            None,
            None,
            Some("character".to_owned()),
            None,
            None,
            None,
        )
        .expect_err("a camera mode without an output should be invalid");

        assert!(error.contains(CAMERA_ENV));
        assert!(error.contains(CAPTURE_ENV));
    }

    #[test]
    fn focus_anchor_requires_capture_and_a_nonempty_name() {
        let without_capture = ReviewRequest::from_values(
            Some("Caves".to_owned()),
            None,
            None,
            None,
            None,
            None,
            None,
            Some("deep_chamber".to_owned()),
            None,
            None,
        )
        .expect_err("a focus override without an output should be invalid");
        assert!(without_capture.contains(FOCUS_ANCHOR_ENV));
        assert!(without_capture.contains(CAPTURE_ENV));

        let empty = ReviewRequest::from_values(
            Some("Caves".to_owned()),
            None,
            Some(".context/cave.png".to_owned()),
            None,
            None,
            None,
            None,
            Some("  ".to_owned()),
            None,
            None,
        )
        .expect_err("an empty focus anchor should be invalid");
        assert!(empty.contains(FOCUS_ANCHOR_ENV));
        assert!(empty.contains("must not be empty"));
    }

    #[test]
    fn full_cutaway_requires_capture_and_rejects_unknown_modes() {
        let without_capture = ReviewRequest::from_values(
            Some("Caves".to_owned()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some("full".to_owned()),
            None,
        )
        .expect_err("a cutaway override without an output should be invalid");
        assert!(without_capture.contains(CUTAWAY_ENV));
        assert!(without_capture.contains(CAPTURE_ENV));

        let unknown = ReviewRequest::from_values(
            Some("Caves".to_owned()),
            None,
            Some(".context/cave.png".to_owned()),
            None,
            None,
            None,
            None,
            None,
            Some("wide".to_owned()),
            None,
        )
        .expect_err("an unknown cutaway mode should be invalid");
        assert!(unknown.contains(CUTAWAY_ENV));
        assert!(unknown.contains("must be full"));
    }

    #[test]
    fn illumination_overlay_requires_capture_and_rejects_unknown_modes() {
        let without_capture = ReviewRequest::from_values(
            Some("Caves".to_owned()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some("overlay".to_owned()),
        )
        .expect_err("an illumination override without an output should be invalid");
        assert!(without_capture.contains(ILLUMINATION_ENV));
        assert!(without_capture.contains(CAPTURE_ENV));

        let unknown = ReviewRequest::from_values(
            Some("Caves".to_owned()),
            None,
            Some(".context/cave.png".to_owned()),
            None,
            None,
            None,
            None,
            None,
            None,
            Some("physical".to_owned()),
        )
        .expect_err("an unknown illumination mode should be invalid");
        assert!(unknown.contains(ILLUMINATION_ENV));
        assert!(unknown.contains("must be overlay"));
    }

    #[test]
    fn illumination_caps_layer_above_tactical_fog_with_independent_render_priority() {
        let position = TilePos::new(HexCoord::from_axial(2, -1), 6);
        let span = HexSpan::new(2.0, 2.8);
        let transform = review_illumination_transform(position, span);
        let diagnostic_bottom = transform.translation.y - transform.scale.y * 0.5;
        let fog_top = span.top + FOG_CAP_LIFT + FOG_CAP_THICKNESS;

        assert!(
            diagnostic_bottom > fog_top,
            "the diagnostic prism must not be coplanar with tactical fog"
        );
        assert!(
            diagnostic_bottom - fog_top >= ILLUMINATION_CAP_CLEARANCE - 1e-5,
            "the complete diagnostic prism needs its authored air gap above fog"
        );
        assert!((transform.scale.x - FOG_CAP_INSET).abs() < f32::EPSILON);
        assert!((transform.scale.y - ILLUMINATION_CAP_THICKNESS).abs() < f32::EPSILON);

        for level in [
            IlluminationLevel::Dark,
            IlluminationLevel::Dim,
            IlluminationLevel::Bright,
        ] {
            let material = review_illumination_material(level);
            assert!(material.unlit);
            assert_eq!(material.alpha_mode, AlphaMode::Blend);
            assert!((material.depth_bias - ILLUMINATION_CAP_DEPTH_BIAS).abs() < f32::EPSILON);
            assert!(
                material.depth_bias - FOG_CAP_DEPTH_BIAS >= 1.0,
                "the diagnostic needs a distinct integer bias bucket after tactical fog"
            );
        }
    }

    #[test]
    fn illumination_overlay_projects_exact_stacked_surfaces_and_cutaway_markers() {
        let coord = HexCoord::from_axial(2, -1);
        let cave_floor = TilePos::new(coord, 6);
        let cave_landing = TilePos::new(HexCoord::from_axial(3, -1), 7);
        let cave_chamber = TilePos::new(HexCoord::from_axial(4, -2), 7);
        let exterior = TilePos::new(coord, 15);
        let cave = LightDomain::Interior(InteriorRegionId(4));
        let illumination = ResolvedIllumination::try_resolve(
            [
                (cave_floor, cave),
                (cave_landing, cave),
                (cave_chamber, cave),
                (exterior, LightDomain::Exterior),
            ],
            ExteriorIllumination::new(IlluminationLevel::Bright),
            &[
                LightSourceSnapshot {
                    pos: cave_landing,
                    domain: cave,
                    light: GameplayLight::new(IlluminationLevel::Dim, 0),
                },
                LightSourceSnapshot {
                    pos: cave_chamber,
                    domain: cave,
                    light: GameplayLight::new(IlluminationLevel::Bright, 0),
                },
            ],
        )
        .expect("the exact illumination fixture should resolve");
        let cave_span = HexSpan::new(2.0, 2.8);
        let landing_span = HexSpan::new(2.4, 3.2);
        let chamber_span = HexSpan::new(2.4, 3.2);
        let exterior_span = HexSpan::new(5.6, 6.4);
        let roof = CutawayOccluder(InteriorRegionId(4));

        let surfaces = collect_review_illumination_surfaces(
            &illumination,
            [
                (&exterior, &exterior_span, Some(&roof)),
                (&cave_landing, &landing_span, None),
                (&cave_chamber, &chamber_span, None),
                (&cave_floor, &cave_span, Some(&roof)),
            ]
            .into_iter(),
        )
        .expect("each exact illumination result has one rendered source tile");
        let by_position = surfaces
            .into_iter()
            .map(|surface| (surface.position, surface))
            .collect::<BTreeMap<_, _>>();

        assert_eq!(by_position.len(), 3);
        assert!(!by_position.contains_key(&exterior));
        assert_eq!(
            by_position.get(&cave_floor).map(|surface| surface.level),
            Some(IlluminationLevel::Dark)
        );
        assert_eq!(
            by_position.get(&cave_landing).map(|surface| surface.level),
            Some(IlluminationLevel::Dim)
        );
        assert_eq!(
            by_position.get(&cave_chamber).map(|surface| surface.level),
            Some(IlluminationLevel::Bright)
        );
        assert_eq!(
            by_position
                .get(&cave_floor)
                .and_then(|surface| surface.cutaway),
            Some(roof)
        );
        assert_eq!(
            by_position.get(&cave_floor).map(|surface| surface.span),
            Some(cave_span)
        );
    }

    #[test]
    fn illumination_overlay_rejects_missing_and_duplicate_rendered_surfaces() {
        let position = TilePos::new(HexCoord::ORIGIN, 6);
        let illumination = ResolvedIllumination::try_resolve(
            [(position, LightDomain::Interior(InteriorRegionId(1)))],
            ExteriorIllumination::new(IlluminationLevel::Bright),
            &[],
        )
        .expect("the one-surface fixture should resolve");
        let span = HexSpan::new(2.0, 2.8);

        let missing = collect_review_illumination_surfaces(
            &illumination,
            std::iter::empty::<(&TilePos, &HexSpan, Option<&CutawayOccluder>)>(),
        )
        .expect_err("an authoritative surface without a rendered tile must fail");
        assert!(missing.contains("no rendered HexTile"));

        let duplicate = collect_review_illumination_surfaces(
            &illumination,
            [(&position, &span, None), (&position, &span, None)].into_iter(),
        )
        .expect_err("duplicate rendered exact surfaces must fail");
        assert!(duplicate.contains("multiple rendered HexTile"));
    }

    #[test]
    fn focus_anchor_relocates_the_selected_actor_to_the_exact_surface() {
        let destination = TilePos::new(HexCoord::from_axial(3, -2), 7);
        let span = HexSpan::new(2.4, 3.2);
        let (table, stone) = review_substance_table();
        let mut anchors = MapAnchors::new();
        anchors.insert(MapAnchorId::from("deep_chamber"), destination);

        let mut app = App::new();
        app.add_systems(PostUpdate, relocate_review_focus);
        app.insert_resource(ReviewCaptureState::new(review_capture_with_focus(
            "deep_chamber",
        )));
        app.insert_resource(TerrainReady);
        app.insert_resource(anchors);
        app.insert_resource(table);
        app.world_mut()
            .spawn((HexTile, destination, span, stone, Headroom(2)));

        let original = Standing {
            pos: TilePos::ORIGIN,
            span: HexSpan::new(0.0, 0.4),
        };
        let actor = app
            .world_mut()
            .spawn((
                Selected,
                Body::new(TraversalProfile::WALKER),
                StandsOn(original),
                Transform::from_translation(original.world_position()),
                CameraFocusTarget::new(original.pos),
            ))
            .id();

        app.update();

        let state = app.world().resource::<ReviewCaptureState>();
        assert!(state.focus_relocated);
        assert!(!state.failed);
        let actor = app.world().entity(actor);
        assert_eq!(
            actor.get::<StandsOn>().map(|standing| standing.0),
            Some(Standing {
                pos: destination,
                span,
            })
        );
        assert_eq!(
            actor
                .get::<Transform>()
                .map(|transform| transform.translation),
            Some(destination.coord.to_world(span.top))
        );
        assert_eq!(
            actor.get::<CameraFocusTarget>().map(|focus| focus.surface),
            Some(destination)
        );
    }

    #[test]
    fn unknown_and_unstandable_focus_anchors_are_rejected() {
        let destination = TilePos::new(HexCoord::from_axial(2, -1), 7);
        let (table, stone) = review_substance_table();
        let body = Body::new(TraversalProfile::WALKER);
        let anchors = MapAnchors::new();
        let no_tiles = std::iter::empty();
        let missing = resolve_review_focus("missing", &anchors, &table, body, None, no_tiles)
            .expect_err("an unpublished anchor should fail");
        assert!(missing.contains(FOCUS_ANCHOR_ENV));
        assert!(missing.contains("did not publish"));

        let mut anchors = MapAnchors::new();
        anchors.insert(MapAnchorId::from("low"), destination);
        let span = HexSpan::new(2.4, 3.2);
        let headroom = Headroom(1);
        let unstandable = resolve_review_focus(
            "low",
            &anchors,
            &table,
            body,
            None,
            std::iter::once((&destination, &span, &stone, &headroom)),
        )
        .expect_err("one level of headroom should reject the normal actor");
        assert!(unstandable.contains(FOCUS_ANCHOR_ENV));
        assert!(unstandable.contains("cannot stand"));
    }

    #[test]
    fn unresolved_runtime_focus_exits_the_capture_cleanly() {
        let (table, _) = review_substance_table();
        let mut app = App::new();
        app.add_systems(PostUpdate, relocate_review_focus);
        app.insert_resource(ReviewCaptureState::new(review_capture_with_focus(
            "not_published",
        )));
        app.insert_resource(TerrainReady);
        app.insert_resource(MapAnchors::new());
        app.insert_resource(table);
        let original = Standing {
            pos: TilePos::ORIGIN,
            span: HexSpan::new(0.0, 0.4),
        };
        app.world_mut().spawn((
            Selected,
            Body::new(TraversalProfile::WALKER),
            StandsOn(original),
            Transform::default(),
            CameraFocusTarget::new(original.pos),
        ));

        app.update();

        let state = app.world().resource::<ReviewCaptureState>();
        assert!(state.failed);
        assert!(!state.focus_relocated);
        assert!(
            !app.world().resource::<Messages<AppExit>>().is_empty(),
            "runtime anchor failure should request a nonzero review exit"
        );
    }

    #[test]
    fn review_camera_accepts_first_person_and_rejects_unknown_tokens() {
        let request = ReviewRequest::from_values(
            Some("Procedural Hills".to_owned()),
            None,
            Some(".context/review.png".to_owned()),
            None,
            None,
            None,
            Some("first-person".to_owned()),
            None,
            None,
            None,
        )
        .expect("first-person should be a supported review camera")
        .expect("the capture request should be present");
        assert_eq!(
            request.capture.map(|capture| capture.camera),
            Some(ReviewCamera::FirstPerson)
        );

        let error = ReviewCamera::parse("first_person")
            .expect_err("the undocumented underscore token should be rejected");

        assert!(error.contains(CAMERA_ENV));
        assert!(error.contains("first-person"));
    }

    #[test]
    fn review_time_requires_a_finite_hour_in_the_day() {
        for invalid in ["-0.1", "24", "NaN", "inf", "not-a-time"] {
            let error = ReviewRequest::from_values(
                Some("Procedural Hills".to_owned()),
                None,
                None,
                None,
                Some(invalid.to_owned()),
                None,
                None,
                None,
                None,
                None,
            )
            .expect_err("an invalid review time should be rejected");
            assert!(error.contains(TIME_ENV), "{error}");
        }
    }

    #[test]
    fn review_liquid_phase_requires_a_finite_number() {
        assert_eq!(parse_liquid_phase("0.5"), Ok(0.5));
        assert_eq!(parse_liquid_phase("-2.25"), Ok(-2.25));
        for invalid in ["NaN", "inf", "-inf", "not-a-phase"] {
            let error =
                parse_liquid_phase(invalid).expect_err("non-finite liquid phases must fail");
            assert!(error.contains(LIQUID_PHASE_ENV), "{error}");
        }
    }

    #[test]
    fn review_time_wins_only_on_the_cloned_launch_scenario() {
        let mut configured = scenario(Some(7));
        configured.starting_time_hours = Some(9.0);
        let mut launched = configured.clone();

        apply_review_time_override(&mut launched, Some(18.5));

        assert_eq!(launched.starting_time_hours, Some(18.5));
        assert_eq!(configured.starting_time_hours, Some(9.0));
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
            default_game: only.name.clone(),
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
            default_game: only.name.clone(),
            scenarios: vec![only.clone(), only],
        };
        assert!(uniquely_named_scenario(&duplicated, "Test").is_err());
    }

    #[test]
    fn review_views_have_exact_deterministic_poses() {
        assert_eq!(ReviewView::parse("rear"), Ok(ReviewView::Rear));
        assert_eq!(
            ReviewView::parse("counter-rotated"),
            Ok(ReviewView::CounterRotated)
        );
        let focus = Vec3::new(1.0, 2.0, 3.0);
        let eye = focus + Vec3::new(0.0, 4.0, 3.0);
        let offset = eye - focus;
        let rotated_eye = focus + Quat::from_rotation_y(2.0 * std::f32::consts::PI / 3.0) * offset;
        let counter_rotated_eye =
            focus + Quat::from_rotation_y(-2.0 * std::f32::consts::PI / 3.0) * offset;
        let rear_eye = focus + Quat::from_rotation_y(std::f32::consts::PI) * offset;
        let top_down_eye = focus + Vec3::Y * offset.length();
        for (view, expected_eye, expected_up) in [
            (ReviewView::Default, eye, camera_up(eye, focus)),
            (
                ReviewView::Rotated,
                rotated_eye,
                camera_up(rotated_eye, focus),
            ),
            (
                ReviewView::CounterRotated,
                counter_rotated_eye,
                camera_up(counter_rotated_eye, focus),
            ),
            (ReviewView::Rear, rear_eye, camera_up(rear_eye, focus)),
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
    fn character_capture_keeps_map_azimuth_and_uses_close_settings() {
        let settings = test_camera_settings();
        let map_focus = Vec3::new(1.0, 2.0, 3.0);
        let map_eye = map_focus + Vec3::new(8.0, 10.0, 6.0);
        let target = Vec3::new(-2.0, 4.0, 5.0);

        for view in [
            ReviewView::Default,
            ReviewView::Rotated,
            ReviewView::CounterRotated,
            ReviewView::Rear,
            ReviewView::TopDown,
        ] {
            let mut transform = Transform::default();
            let mut orbit = PanOrbitCamera::default();
            apply_camera_view(view, map_eye, map_focus, &mut transform, &mut orbit)
                .expect("the map pose should be valid");
            let map_horizontal = Vec3::new(
                transform.translation.x - orbit.focus.x,
                0.0,
                transform.translation.z - orbit.focus.z,
            )
            .try_normalize()
            .unwrap_or_else(|| {
                Vec3::new(map_eye.x - map_focus.x, 0.0, map_eye.z - map_focus.z).normalize()
            });

            apply_character_camera_view(
                map_eye,
                map_focus,
                target,
                &settings,
                &mut transform,
                &mut orbit,
            );

            let expected_focus = target + Vec3::Y * settings.character_focus_height;
            let close_offset = transform.translation - expected_focus;
            let close_horizontal = Vec3::new(close_offset.x, 0.0, close_offset.z).normalize();
            assert!(orbit.focus.distance(expected_focus) < 0.0001);
            assert!((close_offset.length() - settings.character_radius).abs() < 0.0001);
            assert!(close_horizontal.dot(map_horizontal) > 0.9999);
            assert!((orbit.radius - settings.character_radius).abs() < 0.0001);
        }
    }

    #[test]
    fn character_capture_supports_both_vertical_poles() {
        let map_focus = Vec3::ZERO;
        let map_eye = Vec3::new(8.0, 10.0, 6.0);
        let target = Vec3::new(-2.0, 4.0, 5.0);

        for (pitch, wanted_forward) in [(-1.0, Vec3::Y), (1.0, Vec3::NEG_Y)] {
            let mut settings = test_camera_settings();
            settings.character_pitch = pitch;
            let mut transform = Transform::default();
            let mut orbit = PanOrbitCamera::default();
            apply_character_camera_view(
                map_eye,
                map_focus,
                target,
                &settings,
                &mut transform,
                &mut orbit,
            );

            assert!(transform.translation.is_finite());
            assert!(transform.rotation.is_finite());
            assert!((transform.rotation * Vec3::NEG_Z).dot(wanted_forward) > 0.99999);
            assert!((orbit.radius - settings.character_radius).abs() < f32::EPSILON);
        }
    }

    #[test]
    fn first_person_capture_uses_head_height_horizon_and_every_map_azimuth() {
        let settings = test_camera_settings();
        let map_focus = Vec3::ZERO;
        let map_eye = Vec3::new(8.0, 10.0, 6.0);
        let target = Vec3::new(-2.0, 4.0, 5.0);

        for view in [
            ReviewView::Default,
            ReviewView::Rotated,
            ReviewView::CounterRotated,
            ReviewView::Rear,
            ReviewView::TopDown,
        ] {
            let mut transform = Transform::from_translation(map_eye).looking_at(map_focus, Vec3::Y);
            let mut orbit = PanOrbitCamera {
                focus: map_focus,
                radius: map_eye.length(),
            };
            apply_camera_view(view, map_eye, map_focus, &mut transform, &mut orbit)
                .expect("the map pose should be valid");
            let backward = Vec3::new(
                transform.translation.x - orbit.focus.x,
                0.0,
                transform.translation.z - orbit.focus.z,
            )
            .try_normalize()
            .or_else(|| {
                Vec3::new(map_eye.x - map_focus.x, 0.0, map_eye.z - map_focus.z).try_normalize()
            })
            .expect("the fallback review azimuth should be nonzero");
            let expected_forward = -backward;

            apply_first_person_camera_view(
                map_eye,
                map_focus,
                target,
                &settings,
                &mut transform,
                &mut orbit,
            );

            let expected_eye = target + Vec3::Y * settings.first_person_eye_height;
            assert!(transform.translation.distance(expected_eye) < 1e-5);
            assert!(transform.forward().as_vec3().dot(expected_forward) > 0.99999);
            assert!(orbit.focus.distance(expected_eye + expected_forward) < 1e-5);
            assert!((orbit.radius - 1.0).abs() < f32::EPSILON);
        }
    }

    #[test]
    fn first_person_capture_has_stable_orientation_at_vertical_pitch_endpoints() {
        let map_focus = Vec3::ZERO;
        let map_eye = Vec3::new(8.0, 10.0, 6.0);
        let target = Vec3::new(-2.0, 4.0, 5.0);

        for (pitch, expected_forward) in [(-1.0, Vec3::Y), (1.0, Vec3::NEG_Y)] {
            let mut settings = test_camera_settings();
            settings.first_person_pitch = pitch;
            let mut transform = Transform::from_translation(map_eye).looking_at(map_focus, Vec3::Y);
            let mut orbit = PanOrbitCamera {
                focus: map_focus,
                radius: map_eye.length(),
            };

            apply_first_person_camera_view(
                map_eye,
                map_focus,
                target,
                &settings,
                &mut transform,
                &mut orbit,
            );

            let forward = transform.forward().as_vec3();
            let up = transform.up().as_vec3();
            let right = transform.right().as_vec3();
            assert!(transform.translation.is_finite());
            assert!(transform.rotation.is_finite());
            assert!(orbit.focus.is_finite());
            assert!(forward.is_finite() && up.is_finite() && right.is_finite());
            assert!(forward.dot(expected_forward) > 0.99999);
            assert!(up.dot(Vec3::Z) > 0.99999);
            assert!(forward.dot(up).abs() < 1e-5);
            assert!(forward.dot(right).abs() < 1e-5);
            assert!(up.dot(right).abs() < 1e-5);
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
                camera: ReviewCamera::Map,
                focus_anchor: None,
                full_cutaway: false,
                illumination_overlay: false,
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
        app.insert_resource(CameraMode::Map);
        app.insert_resource(Assets::<Image>::default());
        install_capture_systems(
            &mut app,
            ReviewCapture {
                path: path.clone(),
                view: ReviewView::Default,
                camera: ReviewCamera::Map,
                focus_anchor: None,
                full_cutaway: false,
                illumination_overlay: false,
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
    fn character_capture_waits_until_a_focus_target_exists() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin));
        app.init_state::<Screen>();
        app.insert_resource(test_camera_settings());
        app.insert_resource(CameraMode::Map);
        app.insert_resource(Assets::<Image>::default());
        app.init_resource::<CharacterFollowObservation>()
            .add_systems(
                PostUpdate,
                observe_review_pose_before_character_follow.in_set(CameraSystems::FollowCharacter),
            );
        install_capture_systems(
            &mut app,
            ReviewCapture {
                path: PathBuf::from("unused.png"),
                view: ReviewView::Rotated,
                camera: ReviewCamera::Character,
                focus_anchor: None,
                full_cutaway: false,
                illumination_overlay: false,
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
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Gameplay);
        app.update();

        assert!(
            !app.world().resource::<ReviewCaptureState>().view_applied,
            "the character pose should wait for actor selection"
        );
        assert_eq!(*app.world().resource::<CameraMode>(), CameraMode::Map);

        let target = Vec3::new(2.0, 3.0, 4.0);
        app.world_mut().spawn((
            Transform::from_translation(target),
            CameraFocusTarget::new(hex_core::TilePos::ORIGIN),
        ));
        app.update();

        assert!(app.world().resource::<ReviewCaptureState>().view_applied);
        assert_eq!(*app.world().resource::<CameraMode>(), CameraMode::Character);
        let mut cameras = app.world_mut().query::<&PanOrbitCamera>();
        let orbit = cameras
            .single(app.world())
            .expect("the test should have exactly one camera");
        let expected_focus = target + Vec3::Y * test_camera_settings().character_focus_height;
        assert!(orbit.focus.distance(expected_focus) < 0.0001);
        assert!(
            app.world()
                .resource::<CharacterFollowObservation>()
                .saw_review_pose,
            "the one-shot review pose must publish before Character collision follows it"
        );

        let retained_translation = Vec3::new(31.0, 41.0, 59.0);
        let retained_focus = Vec3::new(26.0, 35.0, 53.0);
        {
            let mut camera = app.world_mut().entity_mut(camera);
            camera
                .get_mut::<Transform>()
                .expect("the test camera should keep its transform")
                .translation = retained_translation;
            let mut orbit = camera
                .get_mut::<PanOrbitCamera>()
                .expect("the test camera should keep its controls");
            orbit.focus = retained_focus;
            orbit.radius = 3.0;
        }
        app.update();
        let camera = app.world().entity(camera);
        assert!(
            camera
                .get::<Transform>()
                .expect("the test camera should keep its transform")
                .translation
                .distance(retained_translation)
                < f32::EPSILON,
            "review automation must not reapply its initial pose after success"
        );
        let orbit = camera
            .get::<PanOrbitCamera>()
            .expect("the test camera should keep its controls");
        assert!(orbit.focus.distance(retained_focus) < f32::EPSILON);
        assert!((orbit.radius - 3.0).abs() < f32::EPSILON);
    }

    #[test]
    fn first_person_capture_applies_exact_eye_and_lens() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin));
        app.init_state::<Screen>();
        app.insert_resource(test_camera_settings());
        app.insert_resource(CameraMode::Map);
        app.insert_resource(Assets::<Image>::default());
        install_capture_systems(
            &mut app,
            ReviewCapture {
                path: PathBuf::from("unused.png"),
                view: ReviewView::Rear,
                camera: ReviewCamera::FirstPerson,
                focus_anchor: None,
                full_cutaway: false,
                illumination_overlay: false,
            },
        );
        let camera = app
            .world_mut()
            .spawn((
                Transform::default(),
                PanOrbitCamera::default(),
                RenderTarget::default(),
                Projection::default(),
            ))
            .id();
        let target = Vec3::new(2.0, 3.0, 4.0);
        app.world_mut().spawn((
            Transform::from_translation(target),
            CameraFocusTarget::new(TilePos::ORIGIN),
        ));
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Gameplay);
        app.update();

        assert!(app.world().resource::<ReviewCaptureState>().view_applied);
        assert_eq!(
            *app.world().resource::<CameraMode>(),
            CameraMode::FirstPerson
        );
        let camera = app.world().entity(camera);
        let transform = camera
            .get::<Transform>()
            .expect("the review camera should keep its transform");
        assert!(
            transform
                .translation
                .distance(target + Vec3::Y * test_camera_settings().first_person_eye_height)
                < 1e-5
        );
        let Projection::Perspective(projection) = camera
            .get::<Projection>()
            .expect("the review camera should keep its projection")
        else {
            panic!("the review camera should remain perspective");
        };
        assert!(
            (projection.fov - test_camera_settings().first_person_fov_degrees.to_radians()).abs()
                < f32::EPSILON
        );
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
            error.contains("effectively black") && error.contains("rejected PNG was preserved"),
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
        assert!(!has_visible_tile_coverage(ReviewCamera::Map, 0, 0));
        assert!(!has_visible_tile_coverage(
            ReviewCamera::FirstPerson,
            MIN_VISIBLE_TILES - 1,
            1_000
        ));
        assert!(!has_visible_tile_coverage(
            ReviewCamera::Map,
            MIN_VISIBLE_TILES,
            1_000
        ));
        assert!(has_visible_tile_coverage(ReviewCamera::Map, 50, 1_000));
        assert!(has_visible_tile_coverage(
            ReviewCamera::FirstPerson,
            MIN_VISIBLE_TILES,
            1_000
        ));
    }

    fn review_capture_with_focus(anchor: &str) -> ReviewCapture {
        ReviewCapture {
            path: PathBuf::from("unused.png"),
            view: ReviewView::Default,
            camera: ReviewCamera::Map,
            focus_anchor: Some(anchor.to_owned()),
            full_cutaway: false,
            illumination_overlay: false,
        }
    }

    fn review_substance_table() -> (SubstanceTable, SubstanceId) {
        let palette: ArtPalette = ron::from_str(include_str!("../../../assets/art/palette.ron"))
            .expect("the shipped art palette should parse");
        let mut substances = bevy::platform::collections::HashMap::default();
        substances.insert("air".to_owned(), Substance::invisible(false, false));
        substances.insert(
            "stone".to_owned(),
            Substance::from_swatch(
                SwatchId::new("terrain/stone").expect("the shipped swatch id should be valid"),
                true,
                true,
            ),
        );
        let table = SubstanceTable::from_file(&SubstanceFile { substances }, &palette)
            .expect("the review substances should resolve through the palette");
        let stone = table
            .id("stone")
            .expect("the review test table should contain stone");
        (table, stone)
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
            character_focus_height: 0.4,
            character_radius: 7.0,
            character_probe_radius: 0.1,
            character_collision_margin: 0.35,
            character_restoration_speed: 8.0,
            character_collision_release_delay: 0.2,
            character_self_hide_radius: 1.0,
            character_pitch: 0.3,
            first_person_eye_height: 0.6,
            first_person_pitch: 0.0,
            first_person_fov_degrees: 60.0,
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
