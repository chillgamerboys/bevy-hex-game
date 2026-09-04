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
//! `HEX_REVIEW_LOOK_AT_ANCHOR` and `HEX_REVIEW_LOOK_AT_OFFSET=x,y,z` instead frame
//! an exact generated anchor from an explicit review-only world-space offset without
//! moving an actor or entering either gameplay character-camera mode.
//! `HEX_REVIEW_CHARACTER_RADIUS_SCALE` optionally pulls a Character capture farther
//! from that actor for tall or wide landmark evidence without changing gameplay settings.
//! `HEX_REVIEW_CUTAWAY=full` exposes the complete active interior for overview
//! captures; ordinary gameplay keeps every authored roof or enclosing shell intact.
//! `HEX_REVIEW_ILLUMINATION=overlay` draws the authoritative Dark, Dim, and Bright
//! gameplay tiers over exact interior surfaces for diagnostic captures.
//! `HEX_REVIEW_FOG` selects a strict review-only tactical-shroud treatment while
//! preserving authoritative hostile concealment.
//! `HEX_REVIEW_MATERIAL` selects a strict review-only material treatment shared by
//! terrain and authored-object presentation.
//! `HEX_REVIEW_EDGE` selects a strict review-only voxel edge treatment. Normal-only
//! modes are shared by terrain and authored objects; geometric modes chamfer generated
//! terrain render meshes while leaving authoritative geometry untouched.
//! `HEX_REVIEW_CRYSTAL_LIGHT_PROFILE` selects one strict review-only treatment for
//! generated crystal point lights without changing authoritative illumination.
//! Unanchored Map-camera TopDown overviews additionally fail closed unless every
//! authoritative terrain run is represented once and every topmost boundary cap fits
//! inside the active viewport with margin and valid near/far depth. Deliberate anchored
//! close-ups retain the ordinary terrain-visibility and pixel-coverage gates instead.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bevy::camera::{CameraUpdateSystems, RenderTarget, ViewportConversionError};
use bevy::light::NotShadowCaster;
use bevy::picking::Pickable;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::transform::TransformSystems;
use hex_assets::{CameraSettings, GameAssets, Scenario, ScenarioLibrary, SubstanceTable};
use hex_core::{
    config::{HEX_CIRCUMRADIUS, HEX_SMALL_DIAMETER},
    CameraFocusTarget, CutawayOccluder, GameplaySetupFailure, Headroom, HexSpan, HexTile,
    IlluminationLevel, LightDomain, MapAnchorId, MapAnchors, MapObservationAnchors, MapViewHint,
    PresentationOcclusion, ResolvedMapSeed, ReviewCrystalLightProfile, ReviewEdgeTreatment,
    ReviewMaterialTreatment, Screen, SubstanceId, TerrainPickRun, TerrainReady, TerrainRenderBatch,
    TilePos, TraversalBlockers,
};
use hex_map::LiquidVisualTime;
use hex_perception::ResolvedIllumination;
use hex_units::{Body, Footing, Selected, Standing, StandsOn};
use hex_world::{CameraMode, CameraSystems, PanOrbitCamera};

use crate::capture::{prepare_capture_path, write_png};
use crate::fog::{
    FogPresentationMode, FOG_CAP_DEPTH_BIAS, FOG_CAP_INSET, FOG_CAP_LIFT, FOG_CAP_THICKNESS,
};
use crate::scenarios::ScenarioToLoad;

const SCENARIO_ENV: &str = "HEX_REVIEW_SCENARIO";
const SEED_ENV: &str = "HEX_REVIEW_SEED";
const CAPTURE_ENV: &str = "HEX_REVIEW_CAPTURE";
const VIEW_ENV: &str = "HEX_REVIEW_VIEW";
const TIME_ENV: &str = "HEX_REVIEW_TIME";
const LIQUID_PHASE_ENV: &str = "HEX_REVIEW_LIQUID_PHASE";
const CAMERA_ENV: &str = "HEX_REVIEW_CAMERA";
const FOCUS_ANCHOR_ENV: &str = "HEX_REVIEW_FOCUS_ANCHOR";
const LOOK_AT_ANCHOR_ENV: &str = "HEX_REVIEW_LOOK_AT_ANCHOR";
const LOOK_AT_OFFSET_ENV: &str = "HEX_REVIEW_LOOK_AT_OFFSET";
const CHARACTER_RADIUS_SCALE_ENV: &str = "HEX_REVIEW_CHARACTER_RADIUS_SCALE";
const CUTAWAY_ENV: &str = "HEX_REVIEW_CUTAWAY";
const ILLUMINATION_ENV: &str = "HEX_REVIEW_ILLUMINATION";
const FOG_ENV: &str = "HEX_REVIEW_FOG";
const MATERIAL_ENV: &str = "HEX_REVIEW_MATERIAL";
const EDGE_ENV: &str = "HEX_REVIEW_EDGE";
const CRYSTAL_LIGHT_PROFILE_ENV: &str = "HEX_REVIEW_CRYSTAL_LIGHT_PROFILE";
const SETTLE_FRAMES: u32 = 90;
const CAPTURE_WIDTH: u32 = 1920;
const CAPTURE_HEIGHT: u32 = 1080;
const CAPTURE_PHASE_TIMEOUT: Duration = Duration::from_secs(60);
const READBACK_TIMEOUT: Duration = Duration::from_secs(30);
const MIN_VISIBLE_TILES: usize = 32;
const MIN_VISIBLE_TILE_PERCENT: usize = 5;
/// Review overviews must retain a visible gutter around every exact boundary cap.
///
/// A fractional inset scales with the capture target and catches framing that only
/// barely lands on (and will therefore be clipped by) the raster boundary.
const FULL_FOOTPRINT_VIEWPORT_INSET_FRACTION: f32 = 0.02;
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
    if let Some(mode) = request.fog_mode {
        app.insert_resource(mode);
    }
    app.insert_resource(request.material_treatment);
    app.insert_resource(request.edge_treatment);
    app.insert_resource(request.crystal_light_profile);
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
                (
                    relocate_review_focus,
                    resolve_review_look_at,
                    apply_review_view,
                    apply_review_illumination_overlay,
                )
                    .chain()
                    .before(CameraSystems::FollowCharacter)
                    .before(TransformSystems::Propagate)
                    .before(CameraUpdateSystems),
                capture_settled_frame
                    .after(TransformSystems::Propagate)
                    .after(CameraUpdateSystems),
            )
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
    fog_mode: Option<FogPresentationMode>,
    material_treatment: ReviewMaterialTreatment,
    edge_treatment: ReviewEdgeTreatment,
    crystal_light_profile: ReviewCrystalLightProfile,
    capture: Option<ReviewCapture>,
    launched: bool,
}

impl ReviewRequest {
    fn from_environment() -> Result<Option<Self>, String> {
        let radius_scale = environment_value(CHARACTER_RADIUS_SCALE_ENV)?;
        let look_at_anchor = environment_value(LOOK_AT_ANCHOR_ENV)?;
        let look_at_offset = environment_value(LOOK_AT_OFFSET_ENV)?;
        let fog_mode = environment_value(FOG_ENV)?;
        let material_treatment = environment_value(MATERIAL_ENV)?;
        let edge_treatment = environment_value(EDGE_ENV)?;
        let crystal_light_profile = environment_value(CRYSTAL_LIGHT_PROFILE_ENV)?;
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
        .and_then(|request| Self::with_character_radius_scale(request, radius_scale))
        .and_then(|request| Self::with_anchor_look_at(request, look_at_anchor, look_at_offset))
        .and_then(|request| Self::with_fog_mode(request, fog_mode))
        .and_then(|request| Self::with_material_treatment(request, material_treatment))
        .and_then(|request| Self::with_edge_treatment(request, edge_treatment))
        .and_then(|request| Self::with_crystal_light_profile(request, crystal_light_profile))
    }

    fn with_edge_treatment(
        mut request: Option<Self>,
        value: Option<String>,
    ) -> Result<Option<Self>, String> {
        let Some(value) = value else {
            return Ok(request);
        };
        let request_ref = request
            .as_mut()
            .ok_or_else(|| format!("{EDGE_ENV} requires {SCENARIO_ENV}"))?;
        request_ref.edge_treatment = parse_review_edge_treatment(&value)?;
        Ok(request)
    }

    fn with_crystal_light_profile(
        mut request: Option<Self>,
        value: Option<String>,
    ) -> Result<Option<Self>, String> {
        let Some(value) = value else {
            return Ok(request);
        };
        let request_ref = request
            .as_mut()
            .ok_or_else(|| format!("{CRYSTAL_LIGHT_PROFILE_ENV} requires {SCENARIO_ENV}"))?;
        request_ref.crystal_light_profile = parse_review_crystal_light_profile(&value)?;
        Ok(request)
    }

    fn with_material_treatment(
        mut request: Option<Self>,
        value: Option<String>,
    ) -> Result<Option<Self>, String> {
        let Some(value) = value else {
            return Ok(request);
        };
        let request_ref = request
            .as_mut()
            .ok_or_else(|| format!("{MATERIAL_ENV} requires {SCENARIO_ENV}"))?;
        request_ref.material_treatment = parse_review_material_treatment(&value)?;
        Ok(request)
    }

    fn with_fog_mode(
        mut request: Option<Self>,
        value: Option<String>,
    ) -> Result<Option<Self>, String> {
        let Some(value) = value else {
            return Ok(request);
        };
        let request_ref = request
            .as_mut()
            .ok_or_else(|| format!("{FOG_ENV} requires {SCENARIO_ENV}"))?;
        request_ref.fog_mode = Some(
            FogPresentationMode::parse_review(&value)
                .map_err(|error| format!("{FOG_ENV} {error}"))?,
        );
        Ok(request)
    }

    fn with_character_radius_scale(
        mut request: Option<Self>,
        value: Option<String>,
    ) -> Result<Option<Self>, String> {
        let Some(value) = value else {
            return Ok(request);
        };
        let request_ref = request
            .as_mut()
            .ok_or_else(|| format!("{CHARACTER_RADIUS_SCALE_ENV} requires {CAPTURE_ENV}"))?;
        let capture = request_ref
            .capture
            .as_mut()
            .ok_or_else(|| format!("{CHARACTER_RADIUS_SCALE_ENV} requires {CAPTURE_ENV}"))?;
        if capture.camera != ReviewCamera::Character {
            return Err(format!(
                "{CHARACTER_RADIUS_SCALE_ENV} requires {CAMERA_ENV}=character"
            ));
        }
        capture.character_radius_scale = parse_character_radius_scale(&value)?;
        Ok(request)
    }

    fn with_anchor_look_at(
        mut request: Option<Self>,
        anchor: Option<String>,
        offset: Option<String>,
    ) -> Result<Option<Self>, String> {
        let (anchor, offset) = match (anchor, offset) {
            (None, None) => return Ok(request),
            (Some(_), None) => {
                return Err(format!(
                    "{LOOK_AT_ANCHOR_ENV} requires {LOOK_AT_OFFSET_ENV}=x,y,z"
                ));
            }
            (None, Some(_)) => {
                return Err(format!(
                    "{LOOK_AT_OFFSET_ENV} requires {LOOK_AT_ANCHOR_ENV}"
                ));
            }
            (Some(anchor), Some(offset)) => (anchor, offset),
        };
        if anchor.trim().is_empty() {
            return Err(format!("{LOOK_AT_ANCHOR_ENV} must not be empty"));
        }
        let request_ref = request
            .as_mut()
            .ok_or_else(|| format!("{LOOK_AT_ANCHOR_ENV} requires {CAPTURE_ENV}"))?;
        let capture = request_ref
            .capture
            .as_mut()
            .ok_or_else(|| format!("{LOOK_AT_ANCHOR_ENV} requires {CAPTURE_ENV}"))?;
        if capture.camera != ReviewCamera::Map {
            return Err(format!(
                "{LOOK_AT_ANCHOR_ENV} requires {CAMERA_ENV}=map because it is a review-only free camera"
            ));
        }
        if capture.focus_anchor.is_some() {
            return Err(format!(
                "{LOOK_AT_ANCHOR_ENV} and {FOCUS_ANCHOR_ENV} are mutually exclusive"
            ));
        }
        capture.anchor_look_at = Some(ReviewAnchorLookAt {
            anchor,
            offset: parse_review_look_at_offset(&offset)?,
        });
        Ok(request)
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
                    anchor_look_at: None,
                    character_radius_scale: 1.0,
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
            fog_mode: None,
            material_treatment: ReviewMaterialTreatment::Current,
            edge_treatment: ReviewEdgeTreatment::Current,
            crystal_light_profile: ReviewCrystalLightProfile::Current,
            capture,
            launched: false,
        }))
    }
}

fn parse_review_crystal_light_profile(value: &str) -> Result<ReviewCrystalLightProfile, String> {
    match value {
        "i01-crystal-tight" => Ok(ReviewCrystalLightProfile::Tight),
        "i02-crystal-broad" => Ok(ReviewCrystalLightProfile::Broad),
        "i03-heart-feature-shadow" => Ok(ReviewCrystalLightProfile::HeartFeatureShadow),
        value => Err(format!(
            "{CRYSTAL_LIGHT_PROFILE_ENV} must be i01-crystal-tight, i02-crystal-broad, or i03-heart-feature-shadow; got {value:?}"
        )),
    }
}

fn parse_review_material_treatment(value: &str) -> Result<ReviewMaterialTreatment, String> {
    match value {
        "current" => Ok(ReviewMaterialTreatment::Current),
        "matte-terrain" => Ok(ReviewMaterialTreatment::MatteTerrain),
        "unified-matte" => Ok(ReviewMaterialTreatment::UnifiedMatte),
        value => Err(format!(
            "{MATERIAL_ENV} must be current, matte-terrain, or unified-matte; got {value:?}"
        )),
    }
}

fn parse_review_edge_treatment(value: &str) -> Result<ReviewEdgeTreatment, String> {
    match value {
        "current" => Ok(ReviewEdgeTreatment::Current),
        "micro-bevel-004" => Ok(ReviewEdgeTreatment::MicroBevel04),
        "micro-bevel-008" => Ok(ReviewEdgeTreatment::MicroBevel08),
        "geometric-bevel-004" => Ok(ReviewEdgeTreatment::GeometricBevel04),
        "geometric-bevel-008" => Ok(ReviewEdgeTreatment::GeometricBevel08),
        value => Err(format!(
            "{EDGE_ENV} must be current, micro-bevel-004, micro-bevel-008, geometric-bevel-004, or geometric-bevel-008; got {value:?}"
        )),
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

fn parse_character_radius_scale(value: &str) -> Result<f32, String> {
    let scale = value.parse::<f32>().map_err(|error| {
        format!("{CHARACTER_RADIUS_SCALE_ENV} must be a number in [1, 20]: {error}")
    })?;
    if !scale.is_finite() || !(1.0..=20.0).contains(&scale) {
        return Err(format!(
            "{CHARACTER_RADIUS_SCALE_ENV} must be finite and in [1, 20]; got {value:?}"
        ));
    }
    Ok(scale)
}

fn parse_review_look_at_offset(value: &str) -> Result<Vec3, String> {
    let mut components = value.split(',').map(str::trim);
    let mut parsed = [0.0; 3];
    for (axis, slot) in ["x", "y", "z"].into_iter().zip(&mut parsed) {
        let component = components.next().ok_or_else(|| {
            format!("{LOOK_AT_OFFSET_ENV} must contain exactly x,y,z world-space components")
        })?;
        *slot = component.parse::<f32>().map_err(|error| {
            format!("{LOOK_AT_OFFSET_ENV} {axis} component must be a number: {error}")
        })?;
    }
    if components.next().is_some() {
        return Err(format!(
            "{LOOK_AT_OFFSET_ENV} must contain exactly x,y,z world-space components"
        ));
    }
    let offset = Vec3::from_array(parsed);
    let distance = offset.length();
    if !offset.is_finite() || !distance.is_finite() || !(1.0..=2_048.0).contains(&distance) {
        return Err(format!(
            "{LOOK_AT_OFFSET_ENV} must be finite and 1..=2048 world units from the anchor; got {value:?}"
        ));
    }
    Ok(offset)
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
        "review automation launching scenario {:?} with seed {:?}, material treatment {:?}, edge treatment {:?}, and crystal light profile {:?}",
        scenario.name,
        resolved_seed.map(|seed| seed.0),
        request.material_treatment,
        request.edge_treatment,
        request.crystal_light_profile,
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
    anchor_look_at: Option<ReviewAnchorLookAt>,
    character_radius_scale: f32,
    full_cutaway: bool,
    illumination_overlay: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct ReviewAnchorLookAt {
    anchor: String,
    offset: Vec3,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ResolvedReviewLookAt {
    position: TilePos,
    target: Vec3,
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
    anchor_look_at_target: Option<Vec3>,
    anchor_look_at_resolved: bool,
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
    full_footprint_validated: bool,
}

impl ReviewCaptureState {
    fn new(capture: ReviewCapture) -> Self {
        let focus_relocated = capture.focus_anchor.is_none();
        let anchor_look_at_resolved = capture.anchor_look_at.is_none();
        let illumination_overlay_applied = !capture.illumination_overlay;
        Self {
            capture,
            target: None,
            focus_relocated,
            anchor_look_at_target: None,
            anchor_look_at_resolved,
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
            full_footprint_validated: false,
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

/// Resolves a review-only free-camera target without mutating any actor state.
fn resolve_review_look_at(
    mut state: ResMut<ReviewCaptureState>,
    ready: Option<Res<TerrainReady>>,
    anchors: Option<Res<MapAnchors>>,
    observation_anchors: Option<Res<MapObservationAnchors>>,
    tiles: Query<(&TilePos, &HexSpan), With<HexTile>>,
    mut exit: MessageWriter<AppExit>,
) {
    if state.failed || state.anchor_look_at_resolved {
        return;
    }
    let Some(anchor_name) = state
        .capture
        .anchor_look_at
        .as_ref()
        .map(|look_at| look_at.anchor.clone())
    else {
        state.anchor_look_at_resolved = true;
        return;
    };
    if ready.is_none() {
        return;
    }
    let Some(anchors) = anchors else {
        return;
    };

    let resolved = match resolve_review_look_at_target(
        &anchor_name,
        &anchors,
        observation_anchors.as_deref(),
        tiles.iter(),
    ) {
        Ok(resolved) => resolved,
        Err(error) => {
            error!("review look-at override failed: {error}");
            state.failed = true;
            exit.write(AppExit::error());
            return;
        }
    };
    info!(
        "resolved review-only look-at anchor {:?} at {:?} (world target {:?})",
        anchor_name, resolved.position, resolved.target
    );
    state.anchor_look_at_target = Some(resolved.target);
    state.anchor_look_at_resolved = true;
}

fn resolve_review_look_at_target<'a>(
    anchor_name: &str,
    anchors: &MapAnchors,
    observation_anchors: Option<&MapObservationAnchors>,
    tiles: impl Iterator<Item = (&'a TilePos, &'a HexSpan)>,
) -> Result<ResolvedReviewLookAt, String> {
    let anchor = MapAnchorId::from(anchor_name);
    let gameplay_position = anchors.get(&anchor);
    let observation_position = observation_anchors.and_then(|anchors| anchors.get(&anchor));
    let position = match (gameplay_position, observation_position) {
        (Some(_), Some(_)) => {
            return Err(format!(
                "{LOOK_AT_ANCHOR_ENV} names {anchor_name:?}, which is ambiguously published as both a gameplay and observation anchor"
            ));
        }
        (Some(position), None) | (None, Some(position)) => position,
        (None, None) => {
            return Err(format!(
                "{LOOK_AT_ANCHOR_ENV} names {anchor_name:?}, which the generated map did not publish"
            ));
        }
    };
    let mut matching = tiles.filter(|(candidate, _)| **candidate == position);
    let Some((_, span)) = matching.next() else {
        return Err(format!(
            "{LOOK_AT_ANCHOR_ENV} anchor {anchor_name:?} resolves to {position:?}, which has no exact rendered HexTile surface"
        ));
    };
    if matching.next().is_some() {
        return Err(format!(
            "{LOOK_AT_ANCHOR_ENV} anchor {anchor_name:?} resolves to {position:?}, which has multiple rendered HexTile surfaces"
        ));
    }
    Ok(ResolvedReviewLookAt {
        position,
        target: position.coord.to_world(span.top),
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
    if state.failed
        || state.view_applied
        || !state.focus_relocated
        || !state.anchor_look_at_resolved
    {
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
    let (eye, focus) = match (
        state.capture.anchor_look_at.as_ref(),
        state.anchor_look_at_target,
    ) {
        (Some(look_at), Some(target)) => (target + look_at.offset, target),
        (None, None) => (eye, focus),
        _ => {
            error!("invalid procedural-map review look-at state");
            state.failed = true;
            exit.write(AppExit::error());
            return;
        }
    };
    if let Err(error) =
        apply_camera_view(state.capture.view, eye, focus, &mut transform, &mut orbit)
    {
        error!("invalid procedural-map review camera pose: {error}");
        state.failed = true;
        exit.write(AppExit::error());
        return;
    }
    if state.capture.anchor_look_at.is_some() {
        *mode = CameraMode::Map;
        state.view_applied = true;
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
                state.capture.character_radius_scale,
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
    radius_scale: f32,
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
    let radius = settings.character_radius * radius_scale;
    let offset = horizontal * (radius * pitch.cos()) + Vec3::Y * (radius * pitch.sin());
    let direction = offset.normalize_or_zero();
    let up = if direction.cross(Vec3::Y).length_squared() <= f32::EPSILON {
        Vec3::Z
    } else {
        Vec3::Y
    };

    transform.translation = focus + offset;
    transform.look_at(focus, up);
    orbit.focus = focus;
    orbit.radius = radius;
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

#[derive(Debug, Clone, Copy)]
struct LogicalTerrainRun {
    entity: Entity,
    position: TilePos,
    span: HexSpan,
}

#[derive(Debug, Clone, Copy)]
struct ReviewTerrainBatch<'a> {
    entity: Entity,
    batch: &'a TerrainRenderBatch,
    has_mesh: bool,
    has_material: bool,
    visible: bool,
}

#[derive(Debug, Clone, Copy)]
struct ReviewTerrainBatchOwner {
    entity: Entity,
    has_mesh: bool,
    has_material: bool,
    visible: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FullFootprintCoverage {
    logical_runs: usize,
    boundary_columns: usize,
    projected_corners: usize,
}

#[derive(Debug)]
enum ReviewCaptureCoverageError {
    NoLogicalTerrainRuns,
    DuplicateLogicalTerrainRun {
        entity: Entity,
    },
    LogicalRunMissingPosition {
        entity: Entity,
    },
    LogicalRunMissingSpan {
        entity: Entity,
        position: TilePos,
    },
    InvalidLogicalSpan {
        entity: Entity,
        position: TilePos,
        span: HexSpan,
    },
    UnknownBatchRun {
        entity: Entity,
        position: TilePos,
    },
    BatchRunPositionMismatch {
        entity: Entity,
        logical: TilePos,
        rendered: TilePos,
    },
    BatchRunSpanMismatch {
        entity: Entity,
        logical: HexSpan,
        rendered: HexSpan,
    },
    DuplicateBatchRepresentation {
        entity: Entity,
        position: TilePos,
    },
    MissingBatchRepresentation {
        entity: Entity,
        position: TilePos,
    },
    BoundaryBatchMissingMesh {
        batch: Entity,
        entity: Entity,
        position: TilePos,
    },
    BoundaryBatchMissingMaterial {
        batch: Entity,
        entity: Entity,
        position: TilePos,
    },
    BoundaryBatchHidden {
        batch: Entity,
        entity: Entity,
        position: TilePos,
    },
    NoBoundaryColumns,
    NoActiveReviewCamera,
    MultipleActiveReviewCameras {
        count: usize,
    },
    UnsupportedProjection,
    StaleCameraProjection,
    MissingViewport,
    InvalidViewport {
        viewport: Rect,
    },
    InvalidProjectionDepthRange {
        near: f32,
        far: f32,
    },
    InvalidCameraTransform {
        position: TilePos,
        corner: usize,
    },
    BoundaryPastNearPlane {
        position: TilePos,
        corner: usize,
        depth: f32,
        near: f32,
    },
    BoundaryPastFarPlane {
        position: TilePos,
        corner: usize,
        depth: f32,
        far: f32,
    },
    BoundaryProjectionFailed {
        position: TilePos,
        corner: usize,
        source: ViewportConversionError,
    },
    BoundaryOutsideInset {
        position: TilePos,
        corner: usize,
        projected: Vec2,
        inset: Rect,
    },
}

impl fmt::Display for ReviewCaptureCoverageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoLogicalTerrainRuns => {
                write!(formatter, "the authoritative terrain has no logical runs")
            }
            Self::DuplicateLogicalTerrainRun { entity } => write!(
                formatter,
                "logical terrain entity {entity:?} was published more than once"
            ),
            Self::LogicalRunMissingPosition { entity } => write!(
                formatter,
                "HexTile entity {entity:?} is missing its authoritative TilePos"
            ),
            Self::LogicalRunMissingSpan { entity, position } => write!(
                formatter,
                "HexTile entity {entity:?} at {position:?} is missing its authoritative HexSpan"
            ),
            Self::InvalidLogicalSpan {
                entity,
                position,
                span,
            } => write!(
                formatter,
                "HexTile entity {entity:?} at {position:?} has malformed span {span:?}"
            ),
            Self::UnknownBatchRun { entity, position } => write!(
                formatter,
                "terrain batch represents unknown logical entity {entity:?} at {position:?}"
            ),
            Self::BatchRunPositionMismatch {
                entity,
                logical,
                rendered,
            } => write!(
                formatter,
                "terrain batch moved logical entity {entity:?} from {logical:?} to {rendered:?}"
            ),
            Self::BatchRunSpanMismatch {
                entity,
                logical,
                rendered,
            } => write!(
                formatter,
                "terrain batch changed logical entity {entity:?} from span {logical:?} to {rendered:?}"
            ),
            Self::DuplicateBatchRepresentation { entity, position } => write!(
                formatter,
                "logical terrain entity {entity:?} at {position:?} is represented by more than one batch run"
            ),
            Self::MissingBatchRepresentation { entity, position } => write!(
                formatter,
                "logical terrain entity {entity:?} at {position:?} has no batch representation"
            ),
            Self::BoundaryBatchMissingMesh {
                batch,
                entity,
                position,
            } => write!(
                formatter,
                "topmost boundary terrain entity {entity:?} at {position:?} belongs to batch \
                 {batch:?}, which has no Mesh3d"
            ),
            Self::BoundaryBatchMissingMaterial {
                batch,
                entity,
                position,
            } => write!(
                formatter,
                "topmost boundary terrain entity {entity:?} at {position:?} belongs to batch \
                 {batch:?}, which has no StandardMaterial handle"
            ),
            Self::BoundaryBatchHidden {
                batch,
                entity,
                position,
            } => write!(
                formatter,
                "topmost boundary terrain entity {entity:?} at {position:?} belongs to batch \
                 {batch:?}, which is not currently render-visible"
            ),
            Self::NoBoundaryColumns => {
                write!(
                    formatter,
                    "the authoritative terrain has no boundary columns"
                )
            }
            Self::NoActiveReviewCamera => write!(formatter, "no active review camera exists"),
            Self::MultipleActiveReviewCameras { count } => write!(
                formatter,
                "{count} active review cameras exist; exactly one is required"
            ),
            Self::UnsupportedProjection => write!(
                formatter,
                "the active review camera uses an unsupported custom projection"
            ),
            Self::StaleCameraProjection => write!(
                formatter,
                "the active camera's computed projection is stale relative to Projection"
            ),
            Self::MissingViewport => write!(
                formatter,
                "the active review camera has no computed logical viewport"
            ),
            Self::InvalidViewport { viewport } => {
                write!(
                    formatter,
                    "the active review viewport is invalid: {viewport:?}"
                )
            }
            Self::InvalidProjectionDepthRange { near, far } => write!(
                formatter,
                "the active review projection has invalid depth range {near}..={far}"
            ),
            Self::InvalidCameraTransform { position, corner } => write!(
                formatter,
                "boundary corner {corner} of {position:?} produced a non-finite camera-space point"
            ),
            Self::BoundaryPastNearPlane {
                position,
                corner,
                depth,
                near,
            } => write!(
                formatter,
                "boundary corner {corner} of {position:?} has depth {depth}, before near plane {near}"
            ),
            Self::BoundaryPastFarPlane {
                position,
                corner,
                depth,
                far,
            } => write!(
                formatter,
                "boundary corner {corner} of {position:?} has depth {depth}, beyond far plane {far}"
            ),
            Self::BoundaryProjectionFailed {
                position,
                corner,
                source,
            } => write!(
                formatter,
                "boundary corner {corner} of {position:?} could not be projected: {source}"
            ),
            Self::BoundaryOutsideInset {
                position,
                corner,
                projected,
                inset,
            } => write!(
                formatter,
                "boundary corner {corner} of {position:?} projects to {projected:?}, outside inset viewport {inset:?}"
            ),
        }
    }
}

impl std::error::Error for ReviewCaptureCoverageError {}

fn reconcile_logical_terrain_runs(
    logical_runs: impl IntoIterator<Item = (Entity, Option<TilePos>, Option<HexSpan>)>,
    rendered_runs: impl IntoIterator<Item = TerrainPickRun>,
) -> Result<BTreeMap<Entity, LogicalTerrainRun>, ReviewCaptureCoverageError> {
    let mut logical_by_entity = BTreeMap::new();
    for (entity, position, span) in logical_runs {
        let position =
            position.ok_or(ReviewCaptureCoverageError::LogicalRunMissingPosition { entity })?;
        let span =
            span.ok_or(ReviewCaptureCoverageError::LogicalRunMissingSpan { entity, position })?;
        if !span.bottom.is_finite() || !span.top.is_finite() || span.top <= span.bottom {
            return Err(ReviewCaptureCoverageError::InvalidLogicalSpan {
                entity,
                position,
                span,
            });
        }
        let run = LogicalTerrainRun {
            entity,
            position,
            span,
        };
        if logical_by_entity.insert(entity, run).is_some() {
            return Err(ReviewCaptureCoverageError::DuplicateLogicalTerrainRun { entity });
        }
    }
    if logical_by_entity.is_empty() {
        return Err(ReviewCaptureCoverageError::NoLogicalTerrainRuns);
    }

    let mut represented = BTreeSet::new();
    for rendered in rendered_runs {
        let entity = rendered.entity();
        let position = rendered.position();
        let Some(logical) = logical_by_entity.get(&entity) else {
            return Err(ReviewCaptureCoverageError::UnknownBatchRun { entity, position });
        };
        if logical.position != position {
            return Err(ReviewCaptureCoverageError::BatchRunPositionMismatch {
                entity,
                logical: logical.position,
                rendered: position,
            });
        }
        if logical.span != rendered.span() {
            return Err(ReviewCaptureCoverageError::BatchRunSpanMismatch {
                entity,
                logical: logical.span,
                rendered: rendered.span(),
            });
        }
        if !represented.insert(entity) {
            return Err(ReviewCaptureCoverageError::DuplicateBatchRepresentation {
                entity,
                position,
            });
        }
    }

    if let Some(logical) = logical_by_entity
        .values()
        .find(|logical| !represented.contains(&logical.entity))
    {
        return Err(ReviewCaptureCoverageError::MissingBatchRepresentation {
            entity: logical.entity,
            position: logical.position,
        });
    }
    Ok(logical_by_entity)
}

fn index_review_terrain_batch_owners(
    batches: &[ReviewTerrainBatch<'_>],
) -> Result<BTreeMap<Entity, ReviewTerrainBatchOwner>, ReviewCaptureCoverageError> {
    let mut owners = BTreeMap::new();
    for batch in batches {
        let owner = ReviewTerrainBatchOwner {
            entity: batch.entity,
            has_mesh: batch.has_mesh,
            has_material: batch.has_material,
            visible: batch.visible,
        };
        for run in batch.batch.runs() {
            if owners.insert(run.entity(), owner).is_some() {
                return Err(ReviewCaptureCoverageError::DuplicateBatchRepresentation {
                    entity: run.entity(),
                    position: run.position(),
                });
            }
        }
    }
    Ok(owners)
}

fn validate_boundary_batch_renderability(
    boundary: &[LogicalTerrainRun],
    owners: &BTreeMap<Entity, ReviewTerrainBatchOwner>,
) -> Result<(), ReviewCaptureCoverageError> {
    for run in boundary {
        // Exact one-to-one ownership is established by
        // `reconcile_logical_terrain_runs` before this check. Absence here therefore
        // remains the existing actionable missing-representation failure.
        let Some(owner) = owners.get(&run.entity).copied() else {
            return Err(ReviewCaptureCoverageError::MissingBatchRepresentation {
                entity: run.entity,
                position: run.position,
            });
        };
        if !owner.has_mesh {
            return Err(ReviewCaptureCoverageError::BoundaryBatchMissingMesh {
                batch: owner.entity,
                entity: run.entity,
                position: run.position,
            });
        }
        if !owner.has_material {
            return Err(ReviewCaptureCoverageError::BoundaryBatchMissingMaterial {
                batch: owner.entity,
                entity: run.entity,
                position: run.position,
            });
        }
        if !owner.visible {
            return Err(ReviewCaptureCoverageError::BoundaryBatchHidden {
                batch: owner.entity,
                entity: run.entity,
                position: run.position,
            });
        }
    }
    Ok(())
}

fn topmost_boundary_runs(
    logical_by_entity: &BTreeMap<Entity, LogicalTerrainRun>,
) -> Result<Vec<LogicalTerrainRun>, ReviewCaptureCoverageError> {
    let mut topmost_by_coord = BTreeMap::<_, LogicalTerrainRun>::new();
    for logical in logical_by_entity.values().copied() {
        topmost_by_coord
            .entry(logical.position.coord)
            .and_modify(|current| {
                let top_order = logical.span.top.total_cmp(&current.span.top);
                if top_order.is_gt()
                    || (top_order.is_eq()
                        && (logical.position, logical.entity) < (current.position, current.entity))
                {
                    *current = logical;
                }
            })
            .or_insert(logical);
    }
    let occupied = topmost_by_coord.keys().copied().collect::<BTreeSet<_>>();
    let boundary = topmost_by_coord
        .into_iter()
        .filter_map(|(coord, run)| {
            coord
                .neighbors()
                .iter()
                .any(|neighbor| !occupied.contains(neighbor))
                .then_some(run)
        })
        .collect::<Vec<_>>();
    if boundary.is_empty() {
        return Err(ReviewCaptureCoverageError::NoBoundaryColumns);
    }
    Ok(boundary)
}

fn top_face_corners(run: LogicalTerrainRun) -> [Vec3; 6] {
    let center = run.position.coord.to_world(run.span.top);
    let inner_radius = 0.5 * HEX_SMALL_DIAMETER;
    let half_radius = 0.5 * HEX_CIRCUMRADIUS;
    [
        center + Vec3::new(0.0, 0.0, -HEX_CIRCUMRADIUS),
        center + Vec3::new(-inner_radius, 0.0, -half_radius),
        center + Vec3::new(-inner_radius, 0.0, half_radius),
        center + Vec3::new(0.0, 0.0, HEX_CIRCUMRADIUS),
        center + Vec3::new(inner_radius, 0.0, half_radius),
        center + Vec3::new(inner_radius, 0.0, -half_radius),
    ]
}

fn projection_depth_range(
    projection: &Projection,
) -> Result<(f32, f32), ReviewCaptureCoverageError> {
    let (near, far) = match projection {
        Projection::Perspective(projection) => (projection.near, projection.far),
        Projection::Orthographic(projection) => (projection.near, projection.far),
        Projection::Custom(_) => return Err(ReviewCaptureCoverageError::UnsupportedProjection),
    };
    if !near.is_finite() || !far.is_finite() || near >= far {
        return Err(ReviewCaptureCoverageError::InvalidProjectionDepthRange { near, far });
    }
    Ok((near, far))
}

fn validate_boundary_projection(
    boundary: &[LogicalTerrainRun],
    camera_transform: &GlobalTransform,
    projection: &Projection,
    viewport: Rect,
    mut world_to_viewport: impl FnMut(Vec3) -> Result<Vec2, ViewportConversionError>,
) -> Result<usize, ReviewCaptureCoverageError> {
    let (near, far) = projection_depth_range(projection)?;
    let viewport_size = viewport.size();
    if !viewport.min.is_finite()
        || !viewport.max.is_finite()
        || !viewport_size.is_finite()
        || viewport_size.min_element() <= 0.0
    {
        return Err(ReviewCaptureCoverageError::InvalidViewport { viewport });
    }
    let inset_amount = viewport_size.min_element() * FULL_FOOTPRINT_VIEWPORT_INSET_FRACTION;
    let inset = Rect {
        min: viewport.min + Vec2::splat(inset_amount),
        max: viewport.max - Vec2::splat(inset_amount),
    };
    let view_from_world = camera_transform.affine().inverse();
    let mut projected_corners = 0_usize;

    for run in boundary {
        for (corner, world_point) in top_face_corners(*run).into_iter().enumerate() {
            let view_point = view_from_world.transform_point3(world_point);
            let depth = -view_point.z;
            if !view_point.is_finite() || !depth.is_finite() {
                return Err(ReviewCaptureCoverageError::InvalidCameraTransform {
                    position: run.position,
                    corner,
                });
            }
            if depth < near {
                return Err(ReviewCaptureCoverageError::BoundaryPastNearPlane {
                    position: run.position,
                    corner,
                    depth,
                    near,
                });
            }
            if depth > far {
                return Err(ReviewCaptureCoverageError::BoundaryPastFarPlane {
                    position: run.position,
                    corner,
                    depth,
                    far,
                });
            }

            let projected = world_to_viewport(world_point).map_err(|source| {
                ReviewCaptureCoverageError::BoundaryProjectionFailed {
                    position: run.position,
                    corner,
                    source,
                }
            })?;
            if !inset.contains(projected) {
                return Err(ReviewCaptureCoverageError::BoundaryOutsideInset {
                    position: run.position,
                    corner,
                    projected,
                    inset,
                });
            }
            projected_corners = projected_corners.saturating_add(1);
        }
    }
    Ok(projected_corners)
}

/// Proves that the authoritative footprint is represented by live render batches
/// and fits the exact review camera.
///
/// This is deliberately a structural/frustum gate. Pixel-level completeness,
/// flicker, holes, and visual corruption remain the independent responsibility of
/// post-readback screenshot and motion inspection rather than being inferred from
/// ECS metadata.
fn validate_full_footprint_capture<'a, 'b>(
    logical_runs: impl IntoIterator<Item = (Entity, Option<TilePos>, Option<HexSpan>)>,
    rendered_batches: impl IntoIterator<Item = ReviewTerrainBatch<'b>>,
    cameras: impl IntoIterator<Item = (&'a Camera, &'a GlobalTransform, &'a Projection)>,
) -> Result<FullFootprintCoverage, ReviewCaptureCoverageError> {
    let rendered_batches = rendered_batches.into_iter().collect::<Vec<_>>();
    let logical_by_entity = reconcile_logical_terrain_runs(
        logical_runs,
        rendered_batches.iter().flat_map(|batch| batch.batch.runs()),
    )?;
    let boundary = topmost_boundary_runs(&logical_by_entity)?;
    let owners = index_review_terrain_batch_owners(&rendered_batches)?;
    validate_boundary_batch_renderability(&boundary, &owners)?;
    let active_cameras = cameras
        .into_iter()
        .filter(|(camera, _, _)| camera.is_active)
        .collect::<Vec<_>>();
    if active_cameras.len() != 1 {
        return Err(if active_cameras.is_empty() {
            ReviewCaptureCoverageError::NoActiveReviewCamera
        } else {
            ReviewCaptureCoverageError::MultipleActiveReviewCameras {
                count: active_cameras.len(),
            }
        });
    }
    let Some((camera, camera_transform, projection)) = active_cameras.first().copied() else {
        return Err(ReviewCaptureCoverageError::NoActiveReviewCamera);
    };
    projection_depth_range(projection)?;
    if !camera
        .clip_from_view()
        .abs_diff_eq(projection.get_clip_from_view(), 1e-5)
    {
        return Err(ReviewCaptureCoverageError::StaleCameraProjection);
    }
    let viewport = camera
        .logical_viewport_rect()
        .ok_or(ReviewCaptureCoverageError::MissingViewport)?;
    let projected_corners = validate_boundary_projection(
        &boundary,
        camera_transform,
        projection,
        viewport,
        |world_point| camera.world_to_viewport(camera_transform, world_point),
    )?;

    Ok(FullFootprintCoverage {
        logical_runs: logical_by_entity.len(),
        boundary_columns: boundary.len(),
        projected_corners,
    })
}

fn capture_settled_frame(
    mut commands: Commands,
    ready: Option<Res<TerrainReady>>,
    mut state: ResMut<ReviewCaptureState>,
    terrain_batches: Query<(
        Entity,
        &TerrainRenderBatch,
        Option<&Mesh3d>,
        Option<&MeshMaterial3d<StandardMaterial>>,
        Option<&ViewVisibility>,
    )>,
    logical_runs: Query<(Entity, Option<&TilePos>, Option<&HexSpan>), With<HexTile>>,
    review_cameras: Query<(&Camera, &GlobalTransform, &Projection), With<PanOrbitCamera>>,
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

    (state.total_tiles, state.visible_tiles) = terrain_batches.iter().fold(
        (0usize, 0usize),
        |(total, visible), (_entity, batch, _mesh, _material, visibility)| {
            let represented = batch.runs().len();
            (
                total.saturating_add(represented),
                visible.saturating_add(if visibility.is_some_and(|visibility| visibility.get()) {
                    represented
                } else {
                    0
                }),
            )
        },
    );
    if requires_full_footprint_validation(&state.capture) && !state.full_footprint_validated {
        let coverage = validate_full_footprint_capture(
            logical_runs
                .iter()
                .map(|(entity, position, span)| (entity, position.copied(), span.copied())),
            terrain_batches
                .iter()
                .map(
                    |(entity, batch, mesh, material, visibility)| ReviewTerrainBatch {
                        entity,
                        batch,
                        has_mesh: mesh.is_some(),
                        has_material: material.is_some(),
                        visible: visibility.is_some_and(|visibility| visibility.get()),
                    },
                ),
            review_cameras.iter(),
        );
        match coverage {
            Ok(coverage) => {
                info!(
                    "validated complete review overview: {} logical runs, {} topmost boundary columns, {} projected top-face corners",
                    coverage.logical_runs, coverage.boundary_columns, coverage.projected_corners
                );
                state.full_footprint_validated = true;
            }
            Err(error) => {
                error!("review full-footprint validation failed: {error}");
                state.failed = true;
                exit.write(AppExit::error());
                return;
            }
        }
    }
    if !has_visible_tile_coverage(state.capture.camera, state.visible_tiles, state.total_tiles) {
        if !state.coverage_warning_logged {
            warn!(
                "review capture is waiting for visible terrain: {}/{} logical runs represented by visible batches",
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

fn requires_full_footprint_validation(capture: &ReviewCapture) -> bool {
    capture.camera == ReviewCamera::Map
        && capture.view == ReviewView::TopDown
        && capture.anchor_look_at.is_none()
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
        ExteriorIllumination, GameplayLight, HexCoord, InteriorRegionId, TerrainChunkRoot,
        TerrainPickRun, TraversalProfile,
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
    fn review_fog_mode_is_strict_and_requires_a_review_launch() {
        for (value, expected) in [
            ("current", FogPresentationMode::Current),
            ("none", FogPresentationMode::NoTerrainShading),
            ("dimmed", FogPresentationMode::Dimmed),
            (
                "observed-only",
                FogPresentationMode::ObservedOnlyApproximation,
            ),
            ("softened", FogPresentationMode::SoftenedTwoBand),
        ] {
            assert_eq!(FogPresentationMode::parse_review(value), Ok(expected));
        }
        for invalid in ["", " current", "current ", "DIMMED", "mysterious"] {
            assert!(
                FogPresentationMode::parse_review(invalid).is_err(),
                "{invalid:?} must not be normalized into a review fog mode"
            );
        }

        let request = ReviewRequest::from_values(
            Some("Grand V3 Baseline".to_owned()),
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
        .expect("scenario-only review request parses");
        let configured = ReviewRequest::with_fog_mode(request, Some("dimmed".to_owned()))
            .expect("known fog mode parses")
            .expect("review request remains present");
        assert_eq!(configured.fog_mode, Some(FogPresentationMode::Dimmed));

        let unknown = ReviewRequest::with_fog_mode(Some(configured), Some(" dimmed".to_owned()))
            .expect_err("unknown fog mode fails closed");
        assert!(unknown.contains(FOG_ENV));
        assert!(
            ReviewRequest::with_fog_mode(None, Some("current".to_owned()))
                .expect_err("fog without a review launch fails closed")
                .contains(SCENARIO_ENV)
        );
    }

    #[test]
    fn review_material_treatment_is_strict_and_requires_a_review_launch() {
        for (value, expected) in [
            ("current", ReviewMaterialTreatment::Current),
            ("matte-terrain", ReviewMaterialTreatment::MatteTerrain),
            ("unified-matte", ReviewMaterialTreatment::UnifiedMatte),
        ] {
            assert_eq!(parse_review_material_treatment(value), Ok(expected));
        }
        for invalid in [
            "",
            " current",
            "current ",
            "MATTE-TERRAIN",
            "matte_terrain",
            "mysterious",
        ] {
            assert!(
                parse_review_material_treatment(invalid).is_err(),
                "{invalid:?} must not be normalized into a material treatment"
            );
        }

        let request = ReviewRequest::from_values(
            Some("Grand V3 Baseline".to_owned()),
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
        .expect("scenario-only review request parses");
        assert_eq!(
            request
                .as_ref()
                .expect("scenario produces a request")
                .material_treatment,
            ReviewMaterialTreatment::Current,
        );

        let configured =
            ReviewRequest::with_material_treatment(request, Some("unified-matte".to_owned()))
                .expect("known material treatment parses")
                .expect("review request remains present");
        assert_eq!(
            configured.material_treatment,
            ReviewMaterialTreatment::UnifiedMatte,
        );

        let unknown = ReviewRequest::with_material_treatment(
            Some(configured),
            Some(" unified-matte".to_owned()),
        )
        .expect_err("unknown material treatment fails closed");
        assert!(unknown.contains(MATERIAL_ENV));
        assert!(
            ReviewRequest::with_material_treatment(None, Some("current".to_owned()))
                .expect_err("material treatment without a review launch fails closed")
                .contains(SCENARIO_ENV)
        );
    }

    #[test]
    fn review_edge_treatment_is_strict_and_requires_a_review_launch() {
        for (value, expected) in [
            ("current", ReviewEdgeTreatment::Current),
            ("micro-bevel-004", ReviewEdgeTreatment::MicroBevel04),
            ("micro-bevel-008", ReviewEdgeTreatment::MicroBevel08),
            ("geometric-bevel-004", ReviewEdgeTreatment::GeometricBevel04),
            ("geometric-bevel-008", ReviewEdgeTreatment::GeometricBevel08),
        ] {
            assert_eq!(parse_review_edge_treatment(value), Ok(expected));
        }
        for invalid in [
            "",
            " current",
            "current ",
            "MICRO-BEVEL-004",
            "micro_bevel_004",
            "micro-bevel-04",
            "geometric_bevel_004",
            "geometric-bevel-04",
            "mysterious",
        ] {
            assert!(
                parse_review_edge_treatment(invalid).is_err(),
                "{invalid:?} must not be normalized into an edge treatment"
            );
        }

        let request = ReviewRequest::from_values(
            Some("Grand V3 Baseline".to_owned()),
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
        .expect("scenario-only review request parses");
        assert_eq!(
            request
                .as_ref()
                .expect("scenario produces a request")
                .edge_treatment,
            ReviewEdgeTreatment::Current,
        );

        let configured =
            ReviewRequest::with_edge_treatment(request, Some("geometric-bevel-008".to_owned()))
                .expect("known edge treatment parses")
                .expect("review request remains present");
        assert_eq!(
            configured.edge_treatment,
            ReviewEdgeTreatment::GeometricBevel08,
        );

        let unknown = ReviewRequest::with_edge_treatment(
            Some(configured),
            Some(" geometric-bevel-008".to_owned()),
        )
        .expect_err("unknown edge treatment fails closed");
        assert!(unknown.contains(EDGE_ENV));
        assert!(
            ReviewRequest::with_edge_treatment(None, Some("current".to_owned()))
                .expect_err("edge treatment without a review launch fails closed")
                .contains(SCENARIO_ENV)
        );
    }

    #[test]
    fn review_crystal_light_profile_is_strict_and_requires_a_review_launch() {
        for (value, expected) in [
            ("i01-crystal-tight", ReviewCrystalLightProfile::Tight),
            ("i02-crystal-broad", ReviewCrystalLightProfile::Broad),
            (
                "i03-heart-feature-shadow",
                ReviewCrystalLightProfile::HeartFeatureShadow,
            ),
        ] {
            assert_eq!(parse_review_crystal_light_profile(value), Ok(expected));
        }
        for invalid in [
            "",
            "i01_crystal_tight",
            " i01-crystal-tight",
            "i01-crystal-tight ",
            "I01-CRYSTAL-TIGHT",
            "current",
            "i04-crystal-light",
        ] {
            assert!(
                parse_review_crystal_light_profile(invalid).is_err(),
                "{invalid:?} must not be normalized into a crystal-light profile"
            );
        }

        let request = ReviewRequest::from_values(
            Some("Grand V3 Baseline".to_owned()),
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
        .expect("scenario-only review request parses");
        assert_eq!(
            request
                .as_ref()
                .expect("scenario produces a request")
                .crystal_light_profile,
            ReviewCrystalLightProfile::Current,
        );

        let configured = ReviewRequest::with_crystal_light_profile(
            request,
            Some("i03-heart-feature-shadow".to_owned()),
        )
        .expect("known crystal-light profile parses")
        .expect("review request remains present");
        assert_eq!(
            configured.crystal_light_profile,
            ReviewCrystalLightProfile::HeartFeatureShadow,
        );

        let unknown = ReviewRequest::with_crystal_light_profile(
            Some(configured),
            Some("i03-heart-feature-shadow ".to_owned()),
        )
        .expect_err("unknown crystal-light profile fails closed");
        assert!(unknown.contains(CRYSTAL_LIGHT_PROFILE_ENV));
        assert!(ReviewRequest::with_crystal_light_profile(
            None,
            Some("i01-crystal-tight".to_owned())
        )
        .expect_err("crystal-light profile without a review launch fails closed")
        .contains(SCENARIO_ENV));
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
        assert!(capture.anchor_look_at.is_none());
        assert!((capture.character_radius_scale - 1.0).abs() < f32::EPSILON);
        assert!(capture.full_cutaway);
        assert!(capture.illumination_overlay);
    }

    #[test]
    fn character_radius_scale_is_bounded_and_requires_a_character_capture() {
        let character = ReviewRequest::from_values(
            Some("Procedural Hills".to_owned()),
            None,
            Some(".context/review.png".to_owned()),
            None,
            None,
            None,
            Some("character".to_owned()),
            None,
            None,
            None,
        )
        .expect("character capture parses");
        let scaled = ReviewRequest::with_character_radius_scale(character, Some("4".to_owned()))
            .expect("bounded character radius scale parses")
            .expect("capture remains enabled");
        let scale = scaled
            .capture
            .expect("capture remains configured")
            .character_radius_scale;
        assert!((scale - 4.0).abs() < f32::EPSILON);

        for invalid in ["0.99", "20.01", "NaN", "infinity"] {
            assert!(parse_character_radius_scale(invalid).is_err(), "{invalid}");
        }
        let no_capture = ReviewRequest::with_character_radius_scale(None, Some("2".to_owned()))
            .expect_err("scale without a capture must fail");
        assert!(no_capture.contains(CAPTURE_ENV));

        let map = ReviewRequest::from_values(
            Some("Procedural Hills".to_owned()),
            None,
            Some(".context/review.png".to_owned()),
            None,
            None,
            None,
            Some("map".to_owned()),
            None,
            None,
            None,
        )
        .expect("map capture parses");
        let wrong_camera = ReviewRequest::with_character_radius_scale(map, Some("2".to_owned()))
            .expect_err("scale must not alter a gameplay Map camera");
        assert!(wrong_camera.contains("character"));
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
    fn anchor_look_at_requires_an_explicit_offset_and_map_camera() {
        let map_capture = || {
            ReviewRequest::from_values(
                Some("Grand V3 Baseline".to_owned()),
                None,
                Some(".context/grand.png".to_owned()),
                None,
                None,
                None,
                Some("map".to_owned()),
                None,
                None,
                None,
            )
            .expect("the base Map capture should parse")
        };
        let request = ReviewRequest::with_anchor_look_at(
            map_capture(),
            Some("grand_v3.waterfall_base".to_owned()),
            Some("18, 9, -12".to_owned()),
        )
        .expect("an exact anchor and offset should parse")
        .expect("the capture should remain enabled");
        assert_eq!(
            request.capture.and_then(|capture| capture.anchor_look_at),
            Some(ReviewAnchorLookAt {
                anchor: "grand_v3.waterfall_base".to_owned(),
                offset: Vec3::new(18.0, 9.0, -12.0),
            })
        );

        let missing_offset = ReviewRequest::with_anchor_look_at(
            map_capture(),
            Some("grand_v3.waterfall_base".to_owned()),
            None,
        )
        .expect_err("an implicit camera origin could repeat the framing defect");
        assert!(missing_offset.contains(LOOK_AT_OFFSET_ENV));

        let missing_anchor =
            ReviewRequest::with_anchor_look_at(map_capture(), None, Some("18,9,-12".to_owned()))
                .expect_err("an offset without a semantic target should fail");
        assert!(missing_anchor.contains(LOOK_AT_ANCHOR_ENV));

        let character_capture = ReviewRequest::from_values(
            Some("Grand V3 Baseline".to_owned()),
            None,
            Some(".context/grand.png".to_owned()),
            None,
            None,
            None,
            Some("character".to_owned()),
            None,
            None,
            None,
        )
        .expect("the base Character capture should parse");
        let character_error = ReviewRequest::with_anchor_look_at(
            character_capture,
            Some("grand_v3.waterfall_base".to_owned()),
            Some("18,9,-12".to_owned()),
        )
        .expect_err("the free review camera must not masquerade as Character mode");
        assert!(character_error.contains("requires HEX_REVIEW_CAMERA=map"));

        let focus_capture = ReviewRequest::from_values(
            Some("Grand V3 Baseline".to_owned()),
            None,
            Some(".context/grand.png".to_owned()),
            None,
            None,
            None,
            Some("map".to_owned()),
            Some("grand_v3.party_start".to_owned()),
            None,
            None,
        )
        .expect("the base focus capture should parse");
        let ambiguous = ReviewRequest::with_anchor_look_at(
            focus_capture,
            Some("grand_v3.waterfall_base".to_owned()),
            Some("18,9,-12".to_owned()),
        )
        .expect_err("actor relocation and a free camera target must not be combined");
        assert!(ambiguous.contains("mutually exclusive"));

        for invalid in ["0,0,0", "1,2", "1,2,3,4", "NaN,2,3", "2049,0,0"] {
            assert!(parse_review_look_at_offset(invalid).is_err(), "{invalid}");
        }
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
    fn anchor_look_at_frames_the_exact_surface_without_moving_an_actor() {
        let destination = TilePos::new(HexCoord::from_axial(3, -2), 7);
        let span = HexSpan::new(2.4, 3.2);
        let mut anchors = MapAnchors::new();
        anchors.insert(MapAnchorId::from("waterfall_base"), destination);
        let offset = Vec3::new(12.0, 6.0, -8.0);
        let capture = ReviewCapture {
            path: PathBuf::from("unused.png"),
            view: ReviewView::Rear,
            camera: ReviewCamera::Map,
            focus_anchor: None,
            anchor_look_at: Some(ReviewAnchorLookAt {
                anchor: "waterfall_base".to_owned(),
                offset,
            }),
            character_radius_scale: 1.0,
            full_cutaway: false,
            illumination_overlay: false,
        };

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_systems(
                PostUpdate,
                (resolve_review_look_at, apply_review_view).chain(),
            )
            .insert_resource(ReviewCaptureState::new(capture))
            .insert_resource(TerrainReady)
            .insert_resource(anchors)
            .insert_resource(test_camera_settings())
            .insert_resource(CameraMode::Map)
            .insert_resource(Assets::<Image>::default());
        app.world_mut().spawn((HexTile, destination, span));
        let camera = app
            .world_mut()
            .spawn((
                Transform::default(),
                PanOrbitCamera::default(),
                RenderTarget::default(),
            ))
            .id();
        let actor_position = Vec3::new(-7.0, 4.0, 11.0);
        let actor = app
            .world_mut()
            .spawn((
                Transform::from_translation(actor_position),
                CameraFocusTarget::new(TilePos::ORIGIN),
            ))
            .id();

        app.update();

        let target = destination.coord.to_world(span.top);
        let expected_eye = target + Vec3::new(-offset.x, offset.y, -offset.z);
        let state = app.world().resource::<ReviewCaptureState>();
        assert!(state.anchor_look_at_resolved);
        assert_eq!(state.anchor_look_at_target, Some(target));
        assert!(state.view_applied);
        assert!(!state.failed);
        assert_eq!(*app.world().resource::<CameraMode>(), CameraMode::Map);
        assert_eq!(
            app.world()
                .entity(actor)
                .get::<Transform>()
                .map(|transform| transform.translation),
            Some(actor_position),
            "the review-only free camera must not relocate the selected actor"
        );
        let camera = app.world().entity(camera);
        let transform = camera
            .get::<Transform>()
            .expect("the review camera should retain its pose");
        let orbit = camera
            .get::<PanOrbitCamera>()
            .expect("the review camera should retain its controls");
        assert!(transform.translation.distance(expected_eye) < 0.0001);
        assert!(orbit.focus.distance(target) < 0.0001);
        assert!(
            transform
                .forward()
                .as_vec3()
                .dot((target - expected_eye).normalize())
                > 0.9999
        );
    }

    #[test]
    fn look_at_anchor_requires_one_exact_rendered_surface() {
        let destination = TilePos::new(HexCoord::from_axial(2, -1), 7);
        let span = HexSpan::new(2.4, 3.2);
        let mut anchors = MapAnchors::new();
        anchors.insert(MapAnchorId::from("waterfall_base"), destination);

        let resolved = resolve_review_look_at_target(
            "waterfall_base",
            &anchors,
            None,
            std::iter::once((&destination, &span)),
        )
        .expect("one exact anchor surface should resolve");
        assert_eq!(
            resolved,
            ResolvedReviewLookAt {
                position: destination,
                target: destination.coord.to_world(span.top),
            }
        );

        let missing_anchor = resolve_review_look_at_target(
            "missing",
            &anchors,
            None,
            std::iter::once((&destination, &span)),
        )
        .expect_err("an unpublished look-at anchor should fail");
        assert!(missing_anchor.contains(LOOK_AT_ANCHOR_ENV));
        assert!(missing_anchor.contains("did not publish"));

        let missing_surface = resolve_review_look_at_target(
            "waterfall_base",
            &anchors,
            None,
            std::iter::empty::<(&TilePos, &HexSpan)>(),
        )
        .expect_err("an anchor without an exact rendered surface should fail");
        assert!(missing_surface.contains("no exact rendered HexTile"));

        let duplicate_surface = resolve_review_look_at_target(
            "waterfall_base",
            &anchors,
            None,
            [(&destination, &span), (&destination, &span)].into_iter(),
        )
        .expect_err("an ambiguous rendered surface should fail");
        assert!(duplicate_surface.contains("multiple rendered HexTile"));
    }

    #[test]
    fn look_at_accepts_observation_anchors_but_rejects_namespace_collisions() {
        let destination = TilePos::new(HexCoord::from_axial(4, -3), 225);
        let span = HexSpan::new(89.6, 90.0);
        let gameplay = MapAnchors::new();
        let mut observations = MapObservationAnchors::new();
        observations.insert(MapAnchorId::from("grand_v3.massif_crest"), destination);

        let resolved = resolve_review_look_at_target(
            "grand_v3.massif_crest",
            &gameplay,
            Some(&observations),
            std::iter::once((&destination, &span)),
        )
        .expect("a scenic observation surface should be a valid free-camera target");
        assert_eq!(resolved.position, destination);

        let mut gameplay = MapAnchors::new();
        gameplay.insert(MapAnchorId::from("grand_v3.massif_crest"), destination);
        let collision = resolve_review_look_at_target(
            "grand_v3.massif_crest",
            &gameplay,
            Some(&observations),
            std::iter::once((&destination, &span)),
        )
        .expect_err("an identity in both namespaces must fail closed");
        assert!(collision.contains("ambiguously published"));
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
                1.0,
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
                1.0,
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
    fn character_capture_can_pull_back_without_changing_gameplay_settings() {
        let settings = test_camera_settings();
        let map_focus = Vec3::ZERO;
        let map_eye = Vec3::new(8.0, 10.0, 6.0);
        let target = Vec3::new(-2.0, 4.0, 5.0);
        let mut transform = Transform::default();
        let mut orbit = PanOrbitCamera::default();

        apply_character_camera_view(
            map_eye,
            map_focus,
            target,
            &settings,
            4.0,
            &mut transform,
            &mut orbit,
        );

        assert!((orbit.radius - settings.character_radius * 4.0).abs() < 0.0001);
        assert!(
            (transform
                .translation
                .distance(target + Vec3::Y * settings.character_focus_height)
                - settings.character_radius * 4.0)
                .abs()
                < 0.0001
        );
        assert!(
            (settings.character_radius - test_camera_settings().character_radius).abs()
                < f32::EPSILON
        );
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
                anchor_look_at: None,
                character_radius_scale: 1.0,
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
                anchor_look_at: None,
                character_radius_scale: 1.0,
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
        let logical = app.world_mut().spawn_empty().id();
        let represented = TerrainPickRun::new(
            logical,
            TilePos::new(HexCoord::ORIGIN, 0),
            HexSpan::new(0.0, 0.4),
        );
        app.world_mut().spawn((
            TerrainRenderBatch::new(
                TerrainChunkRoot { q: 0, r: 0 },
                SubstanceId(1),
                vec![represented; MIN_VISIBLE_TILES],
            ),
            ViewVisibility::VISIBLE,
        ));
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
                anchor_look_at: None,
                character_radius_scale: 1.0,
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
                anchor_look_at: None,
                character_radius_scale: 1.0,
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

    fn fixture_entity(raw: u32) -> Entity {
        Entity::from_raw_u32(raw).expect("the fixture entity id should be valid")
    }

    #[test]
    fn terrain_batch_reconciliation_requires_one_exact_run_per_logical_entity() {
        let first = fixture_entity(1);
        let second = fixture_entity(2);
        let first_position = TilePos::new(HexCoord::ORIGIN, 0);
        let second_position = TilePos::new(HexCoord::from_axial(1, 0), 2);
        let first_span = HexSpan::new(0.0, 0.4);
        let second_span = HexSpan::new(0.0, 1.2);
        let logical = || {
            [
                (first, Some(first_position), Some(first_span)),
                (second, Some(second_position), Some(second_span)),
            ]
        };
        let first_rendered = TerrainPickRun::new(first, first_position, first_span);
        let second_rendered = TerrainPickRun::new(second, second_position, second_span);

        let reconciled =
            reconcile_logical_terrain_runs(logical(), [second_rendered, first_rendered])
                .expect("the exact one-to-one projection should reconcile");
        assert_eq!(reconciled.len(), 2);

        assert!(matches!(
            reconcile_logical_terrain_runs(logical(), [first_rendered]),
            Err(ReviewCaptureCoverageError::MissingBatchRepresentation {
                entity,
                ..
            }) if entity == second
        ));
        assert!(matches!(
            reconcile_logical_terrain_runs(
                logical(),
                [first_rendered, first_rendered, second_rendered]
            ),
            Err(ReviewCaptureCoverageError::DuplicateBatchRepresentation {
                entity,
                ..
            }) if entity == first
        ));
        assert!(matches!(
            reconcile_logical_terrain_runs(
                logical(),
                [
                    TerrainPickRun::new(first, second_position, first_span),
                    second_rendered,
                ]
            ),
            Err(ReviewCaptureCoverageError::BatchRunPositionMismatch {
                entity,
                ..
            }) if entity == first
        ));
        assert!(matches!(
            reconcile_logical_terrain_runs(
                [(first, None, Some(first_span))],
                std::iter::empty()
            ),
            Err(ReviewCaptureCoverageError::LogicalRunMissingPosition { entity })
                if entity == first
        ));
        assert!(matches!(
            reconcile_logical_terrain_runs(
                [(first, Some(first_position), Some(HexSpan::default()))],
                std::iter::empty()
            ),
            Err(ReviewCaptureCoverageError::InvalidLogicalSpan { entity, .. })
                if entity == first
        ));
    }

    #[test]
    fn full_footprint_gate_requires_boundary_batch_render_components() {
        let logical_entity = fixture_entity(40);
        let batch_entity = fixture_entity(41);
        let position = TilePos::new(HexCoord::ORIGIN, 0);
        let span = HexSpan::new(0.0, 0.4);
        let batch = TerrainRenderBatch::new(
            TerrainChunkRoot { q: 0, r: 0 },
            SubstanceId(1),
            vec![TerrainPickRun::new(logical_entity, position, span)],
        );
        let logical = || [(logical_entity, Some(position), Some(span))];
        let no_cameras = || {
            std::iter::empty::<(
                &'static Camera,
                &'static GlobalTransform,
                &'static Projection,
            )>()
        };

        assert!(matches!(
            validate_full_footprint_capture(
                logical(),
                [ReviewTerrainBatch {
                    entity: batch_entity,
                    batch: &batch,
                    has_mesh: false,
                    has_material: true,
                    visible: true,
                }],
                no_cameras(),
            ),
            Err(ReviewCaptureCoverageError::BoundaryBatchMissingMesh {
                batch,
                entity,
                position: failed_position,
            }) if batch == batch_entity
                && entity == logical_entity
                && failed_position == position
        ));
        assert!(matches!(
            validate_full_footprint_capture(
                logical(),
                [ReviewTerrainBatch {
                    entity: batch_entity,
                    batch: &batch,
                    has_mesh: true,
                    has_material: false,
                    visible: true,
                }],
                no_cameras(),
            ),
            Err(ReviewCaptureCoverageError::BoundaryBatchMissingMaterial {
                batch,
                entity,
                position: failed_position,
            }) if batch == batch_entity
                && entity == logical_entity
                && failed_position == position
        ));
    }

    #[test]
    fn full_footprint_gate_rejects_a_hidden_boundary_batch() {
        let logical_entity = fixture_entity(42);
        let batch_entity = fixture_entity(43);
        let position = TilePos::new(HexCoord::ORIGIN, 0);
        let span = HexSpan::new(0.0, 0.4);
        let batch = TerrainRenderBatch::new(
            TerrainChunkRoot { q: 0, r: 0 },
            SubstanceId(1),
            vec![TerrainPickRun::new(logical_entity, position, span)],
        );

        let result = validate_full_footprint_capture(
            [(logical_entity, Some(position), Some(span))],
            [ReviewTerrainBatch {
                entity: batch_entity,
                batch: &batch,
                has_mesh: true,
                has_material: true,
                visible: false,
            }],
            std::iter::empty::<(&Camera, &GlobalTransform, &Projection)>(),
        );
        assert!(matches!(
            result,
            Err(ReviewCaptureCoverageError::BoundaryBatchHidden {
                batch,
                entity,
                position: failed_position,
            }) if batch == batch_entity
                && entity == logical_entity
                && failed_position == position
        ));
    }

    #[test]
    fn topmost_boundary_uses_exposed_columns_and_exact_six_corner_caps() {
        let raised_coord = HexCoord::from_axial(1, 0);
        let mut logical = BTreeMap::new();
        for (raw, coord) in (10_u32..).zip(HexCoord::ORIGIN.within_radius(1)) {
            let entity = fixture_entity(raw);
            let run = LogicalTerrainRun {
                entity,
                position: TilePos::new(coord, 0),
                span: HexSpan::new(0.0, 0.4),
            };
            logical.insert(entity, run);
        }
        let raised_entity = fixture_entity(30);
        let raised = LogicalTerrainRun {
            entity: raised_entity,
            position: TilePos::new(raised_coord, 3),
            span: HexSpan::new(0.4, 1.6),
        };
        logical.insert(raised_entity, raised);

        let boundary = topmost_boundary_runs(&logical)
            .expect("a complete radius-one footprint should have a boundary");
        assert_eq!(boundary.len(), 6);
        assert!(boundary
            .iter()
            .all(|run| run.position.coord != HexCoord::ORIGIN));
        assert_eq!(
            boundary
                .iter()
                .find(|run| run.position.coord == raised_coord)
                .expect("the raised boundary column should exist")
                .entity,
            raised_entity,
            "the topmost run must own the projected boundary cap"
        );

        let corners = top_face_corners(raised);
        assert_eq!(corners.len(), 6);
        assert!(corners
            .iter()
            .all(|corner| (corner.y - raised.span.top).abs() < f32::EPSILON));
        let center = raised_coord.to_world(raised.span.top);
        for corner in corners {
            let horizontal = Vec2::new(corner.x - center.x, corner.z - center.z);
            assert!((horizontal.length() - HEX_CIRCUMRADIUS).abs() < 1e-6);
        }
    }

    fn test_project_to_viewport(
        camera_transform: &GlobalTransform,
        projection: &Projection,
        viewport: Rect,
        world_point: Vec3,
    ) -> Result<Vec2, ViewportConversionError> {
        let view_point = camera_transform
            .affine()
            .inverse()
            .transform_point3(world_point);
        let clip = projection.get_clip_from_view() * view_point.extend(1.0);
        if !clip.is_finite() || clip.w.abs() <= f32::EPSILON {
            return Err(ViewportConversionError::InvalidData);
        }
        let ndc = clip.truncate() / clip.w;
        if !ndc.is_finite() {
            return Err(ViewportConversionError::InvalidData);
        }
        Ok((Vec2::new(ndc.x, -ndc.y) + Vec2::ONE) * 0.5 * viewport.size() + viewport.min)
    }

    fn top_down_review_transform(focus: Vec3, distance: f32) -> GlobalTransform {
        let mut transform = Transform::from_translation(focus + Vec3::Y * distance);
        transform.look_at(focus, Vec3::NEG_Z);
        GlobalTransform::from(transform)
    }

    #[test]
    fn grand_top_down_boundary_rejects_old_pose_and_accepts_generated_hint() {
        const GRAND_RADIUS: u32 = 187;
        const GRAND_BOUNDARY_COLUMNS: usize = 6 * 187;
        const MAXIMUM_WORLD_HEIGHT: f32 = 256.0 * 0.4;
        const REVIEW_WIDTH: f32 = 1_920.0;
        const REVIEW_HEIGHT: f32 = 1_080.0;
        const REVIEW_ASPECT: f32 = REVIEW_WIDTH / REVIEW_HEIGHT;

        let boundary = HexCoord::ORIGIN
            .within_radius(GRAND_RADIUS)
            .into_iter()
            .filter(|coord| coord.distance(HexCoord::ORIGIN) == GRAND_RADIUS)
            .zip(100_u32..)
            .map(|(coord, raw)| LogicalTerrainRun {
                entity: fixture_entity(raw),
                position: TilePos::new(coord, 0),
                span: HexSpan::new(0.0, 0.4),
            })
            .collect::<Vec<_>>();
        assert_eq!(boundary.len(), GRAND_BOUNDARY_COLUMNS);
        let viewport = Rect::from_corners(Vec2::ZERO, Vec2::new(REVIEW_WIDTH, REVIEW_HEIGHT));

        let projection_with_far = |far: f32| {
            Projection::Perspective(bevy::camera::PerspectiveProjection {
                fov: 40.0_f32.to_radians(),
                aspect_ratio: REVIEW_ASPECT,
                near: 0.1,
                far,
                ..default()
            })
        };
        let old_eye = Vec3::new(0.0, 48.0, 42.0);
        let old_focus = Vec3::new(0.0, 6.0, 0.0);
        let old_transform = top_down_review_transform(old_focus, old_eye.distance(old_focus));
        let old_projection = projection_with_far(1_000.0);
        let old_result = validate_boundary_projection(
            &boundary,
            &old_transform,
            &old_projection,
            viewport,
            |point| test_project_to_viewport(&old_transform, &old_projection, viewport, point),
        );
        assert!(matches!(
            old_result,
            Err(ReviewCaptureCoverageError::BoundaryOutsideInset { .. })
        ));

        let half_width = f32::sqrt(3.0).mul_add(187.0, 2.0);
        let half_depth = 1.5_f32.mul_add(187.0, 2.0);
        let required_vertical_half_extent = half_depth.max(half_width / REVIEW_ASPECT);
        let hinted_distance = ((required_vertical_half_extent + MAXIMUM_WORLD_HEIGHT * 0.3 + 12.0)
            / 20.0_f32.to_radians().tan())
            * 1.1;
        let hinted_focus = Vec3::Y * (MAXIMUM_WORLD_HEIGHT * 0.35);
        let hinted_transform = top_down_review_transform(hinted_focus, hinted_distance);
        let clipped_projection = projection_with_far(hinted_distance * 0.5);
        assert!(matches!(
            validate_boundary_projection(
                &boundary,
                &hinted_transform,
                &clipped_projection,
                viewport,
                |point| test_project_to_viewport(
                    &hinted_transform,
                    &clipped_projection,
                    viewport,
                    point
                ),
            ),
            Err(ReviewCaptureCoverageError::BoundaryPastFarPlane { .. })
        ));
        let hinted_projection = projection_with_far((hinted_distance * 2.0).max(1_000.0));
        let projected = validate_boundary_projection(
            &boundary,
            &hinted_transform,
            &hinted_projection,
            viewport,
            |point| {
                test_project_to_viewport(&hinted_transform, &hinted_projection, viewport, point)
            },
        )
        .expect("the generated Grand V3 hint should frame every boundary cap with margin");
        assert_eq!(projected, GRAND_BOUNDARY_COLUMNS * 6);
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

    #[test]
    fn anchored_top_down_skips_only_the_full_footprint_overview_gate() {
        let directory = review_test_directory("anchored-top-down-gate");
        let _cleanup = fs::remove_dir_all(&directory);
        let anchored_capture = ReviewCapture {
            path: directory.join("capture.png"),
            view: ReviewView::TopDown,
            camera: ReviewCamera::Map,
            focus_anchor: None,
            anchor_look_at: Some(ReviewAnchorLookAt {
                anchor: "waterfall_base".to_owned(),
                offset: Vec3::new(0.0, 24.0, 0.0),
            }),
            character_radius_scale: 1.0,
            full_cutaway: false,
            illumination_overlay: false,
        };
        assert!(!requires_full_footprint_validation(&anchored_capture));
        let mut overview_capture = anchored_capture.clone();
        overview_capture.anchor_look_at = None;
        assert!(requires_full_footprint_validation(&overview_capture));

        let mut state = ReviewCaptureState::new(anchored_capture);
        state.view_applied = true;
        state.settled_frames = SETTLE_FRAMES - 1;
        state.target = Some(Handle::default());

        let position = TilePos::new(HexCoord::ORIGIN, 0);
        let span = HexSpan::new(0.0, 0.4);
        let mut app = App::new();
        app.add_systems(Update, capture_settled_frame)
            .insert_resource(TerrainReady)
            .insert_resource(state);
        let logical = app.world_mut().spawn((HexTile, position, span)).id();
        let represented = TerrainPickRun::new(logical, position, span);
        let batch_entity = app
            .world_mut()
            .spawn((
                TerrainRenderBatch::new(
                    TerrainChunkRoot { q: 0, r: 0 },
                    SubstanceId(1),
                    vec![represented; MIN_VISIBLE_TILES - 1],
                ),
                ViewVisibility::VISIBLE,
            ))
            .id();

        app.update();
        let state = app.world().resource::<ReviewCaptureState>();
        assert!(!state.failed, "the close-up must not run the overview gate");
        assert!(!state.full_footprint_validated);
        assert!(
            !state.requested,
            "ordinary visible-tile coverage still gates"
        );
        assert!(state.coverage_warning_logged);

        app.world_mut().entity_mut(batch_entity).insert((
            TerrainRenderBatch::new(
                TerrainChunkRoot { q: 0, r: 0 },
                SubstanceId(1),
                vec![represented; MIN_VISIBLE_TILES],
            ),
            ViewVisibility::VISIBLE,
        ));
        app.update();

        let state = app.world().resource::<ReviewCaptureState>();
        assert!(!state.failed);
        assert!(!state.full_footprint_validated);
        assert!(
            state.requested,
            "the anchored close-up should proceed once ordinary coverage passes"
        );
        let _cleanup = fs::remove_dir_all(directory);
    }

    fn review_capture_with_focus(anchor: &str) -> ReviewCapture {
        ReviewCapture {
            path: PathBuf::from("unused.png"),
            view: ReviewView::Default,
            camera: ReviewCamera::Map,
            focus_anchor: Some(anchor.to_owned()),
            anchor_look_at: None,
            character_radius_scale: 1.0,
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
