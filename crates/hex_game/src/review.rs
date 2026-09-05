//! Deterministic launch and capture hooks for procedural-map review packs.
//!
//! This module is compiled into runtime builds only with the default-off
//! `map-review` feature. Ordinary release builds neither inspect nor react to the
//! review environment variables. In a review build, setting
//! `HEX_REVIEW_SCENARIO` selects a scenario without automating the title-screen UI;
//! `HEX_REVIEW_SEED` optionally replaces its configured procedural seed.
//! `HEX_REVIEW_CAPTURE_PLAN` captures the renderer after validated terrain has
//! settled, then exits. Every automated plan requires a fresh lowercase-hex
//! `HEX_REVIEW_LAUNCH_NONCE` and `HEX_REVIEW_SOURCE_PROVENANCE_SHA256`; the runtime
//! binds both to its own process and executable plus the exact capture-plan bytes.
//! `HEX_REVIEW_TIME` optionally selects the cyclic lighting hour for that launch.
//! `HEX_REVIEW_LIQUID_PHASE` freezes liquid presentation at a deterministic phase;
//! captures default to phase `0.0` when no explicit phase is configured.
//! `HEX_REVIEW_FOCUS_ANCHOR` optionally resolves one exact generated anchor as a
//! presentation-only camera target without relocating the selected actor. This keeps
//! iteration tooling on the same loading and validation path as manual play while
//! avoiding compositor-dependent screenshots.
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
//! `HEX_REVIEW_LIFECYCLE` binds a normal capture plan to one strict provenance
//! request and, after each real capture sequence, tears down and recreates the
//! disposable projection in-process. Exactly 100 verified cycles atomically publish
//! the requested hash-chained lifecycle certificate; any leak or authority change
//! exits without a valid certificate.
//! Unanchored Map-camera TopDown overviews additionally fail closed unless every
//! authoritative terrain run is represented once and every topmost boundary cap fits
//! inside the active viewport with margin and valid near/far depth. Deliberate anchored
//! close-ups retain the ordinary terrain-visibility and pixel-coverage gates instead.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::env;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use atomicwrites::{AllowOverwrite, AtomicFile};
use bevy::camera::{
    Camera3dDepthTextureUsage, CameraUpdateSystems, RenderTarget, ViewportConversionError,
};
use bevy::core_pipeline::oit::{resolve::is_oit_supported, OrderIndependentTransparencySettings};
use bevy::ecs::system::SystemParam;
use bevy::light::{FogVolume, NotShadowCaster, VolumetricFog, VolumetricLight};
use bevy::mesh::Indices;
use bevy::pbr::{ScreenSpaceTransmission, ScreenSpaceTransmissionQuality};
use bevy::picking::Pickable;
use bevy::prelude::*;
use bevy::render::render_resource::{TextureFormat, TextureUsages};
use bevy::render::renderer::{RenderAdapter, RenderDevice};
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::transform::TransformSystems;
use hex_assets::{CameraSettings, GameAssets, Scenario, ScenarioLibrary, SubstanceTable};
use hex_core::{
    config::{HEX_CIRCUMRADIUS, HEX_SMALL_DIAMETER},
    AuthoredObjectVoxelRuns, Busy, CameraFocusTarget, CanopyOccluder, ControlOwner,
    CutawayOccluder, ExteriorIllumination, GameplayLight, GameplaySetup, GameplaySetupFailure,
    Headroom, HexSpan, HexTile, IlluminationLevel, InspectionCameraSubject, KnowledgeState,
    LightDomain, MapAnchorId, MapAnchors, MapObservationAnchors, MapViewHint,
    PresentationOcclusion, PresentationSystems, ResolvedMapSeed, ReviewCrystalLightProfile,
    ReviewEdgeTreatment, ReviewMaterialTreatment, RunBottom, Screen, SubstanceId,
    TargetReticleRequest, TerrainChunkRoot, TerrainPickRun, TerrainReady, TerrainRenderBatch,
    TilePos, TraversalBlockers, TreeOccluder,
};
use hex_map::{
    review_world_detail::{
        ReviewCameraFeaturesV1, ReviewCleanupStateV1, ReviewPerformanceSampleV1,
        ReviewRuntimeReceiptV1, ReviewWorldDetailProfileV1, ReviewWorldDetailReportV1,
        ReviewWorldDetailRuntimeAssetEvidenceV1, ReviewWorldDetailTeardownReceiptV1,
        ReviewWorldDetailTeardownRequestV1,
    },
    CurrentWorldSnapshotV1, LiquidVisualTime, ReviewLiquidMaterial, ReviewSuppressedWaterMaterial,
    ReviewWorldDetailEntity, WorldReplicationStateV1,
};
use hex_multiplayer::{CampaignSaveRefusalV2, CampaignSaveStateV2};
use hex_perception::{FactionMapKnowledge, ResolvedIllumination};
use hex_units::{
    Archetype, Body, Downed, Enemy, Faction, Footing, MovingTo, OutOfRangeOverlay, PathOverlay,
    Player, RangeOverlay, Selected, Standing, StandsOn, UnitRegistry,
};
use hex_world::{CameraMode, CameraSystems, PanOrbitCamera, SkyRuntimeAssetEvidenceV1, TimeOfDay};
use serde::{Deserialize, Serialize};

use crate::capture::{prepare_capture_path, write_png};
use crate::fog::{
    FogPresentationMode, FOG_CAP_DEPTH_BIAS, FOG_CAP_INSET, FOG_CAP_LIFT, FOG_CAP_THICKNESS,
};
use crate::save::{CampaignSaveStatusProjection, CampaignStore};
use crate::scenarios::ScenarioToLoad;
use crate::storage::StoragePaths;

const SCENARIO_ENV: &str = "HEX_REVIEW_SCENARIO";
const SEED_ENV: &str = "HEX_REVIEW_SEED";
const CAPTURE_ENV: &str = "HEX_REVIEW_CAPTURE";
const CAPTURE_PLAN_ENV: &str = "HEX_REVIEW_CAPTURE_PLAN";
const LAUNCH_NONCE_ENV: &str = "HEX_REVIEW_LAUNCH_NONCE";
const SOURCE_PROVENANCE_SHA256_ENV: &str = "HEX_REVIEW_SOURCE_PROVENANCE_SHA256";
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
const WORLD_DETAIL_ENV: &str = "HEX_REVIEW_WORLD_DETAIL";
const LIFECYCLE_ENV: &str = "HEX_REVIEW_LIFECYCLE";
const WORLD_DETAIL_WARNING: &str = "UNAPPROVABLE STRUCTURAL DRAFT — AESTHETIC REVIEW ONLY";
const REVIEW_COLLIDER_STATIC_INVARIANT: &str =
    "review projection construction has no collision-backend dependency or collider bundle";
const SETTLE_FRAMES: u32 = 90;
/// Genuine rendered frames retained for each report sample. The first capture
/// gathers this window during the latter portion of its settle interval so its
/// report is ready at the requested settle deadline.
const PERFORMANCE_WINDOW_FRAMES: usize = 60;
/// The two halves of the rolling p95 window must agree within this fraction.
const PERFORMANCE_P95_DRIFT_LIMIT: f32 = 0.20;
/// Exact logical allocation scope used by matched control/candidate comparisons.
///
/// GPU driver padding, render-pipeline caches, and private ECS table capacity are
/// intentionally outside this public main-world evidence boundary.
const REVIEW_RESIDENT_MEMORY_SCOPE: &str =
    "all live Mesh3d vertex/index buffers; all live StandardMaterial values; all live non-capture Image texture mip payloads; renderer-evidenced liquid/review-water/sky materials; and publicly nameable review-entity component/name payloads";
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

    let captures = request.capture_sequence();
    let lifecycle = match request.lifecycle.clone() {
        Some(configuration) => {
            let Some(runtime_receipt) = request.runtime_receipt.clone() else {
                app.insert_resource(ReviewConfigurationError(
                    "lifecycle automation has no validated runtime receipt".to_owned(),
                ))
                .add_systems(Startup, reject_invalid_configuration);
                return;
            };
            Some(ReviewLifecycleProbeV1::new(
                configuration,
                captures.clone(),
                runtime_receipt,
            ))
        }
        None => None,
    };
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
    app.insert_resource(request.world_detail_profile.clone());
    if let Some(runtime_receipt) = request.runtime_receipt.clone() {
        app.insert_resource(runtime_receipt);
    }
    app.insert_resource(request)
        .add_systems(
            Update,
            launch_review_scenario.run_if(in_state(Screen::Title)),
        )
        .add_systems(
            PostUpdate,
            configure_review_camera_features
                .after(PresentationSystems::ApplyMaterials)
                // Presentation material reconciliation itself follows Bevy's
                // bounds calculation, while CameraUpdateSystems feeds that
                // calculation. Ordering this system before CameraUpdateSystems
                // therefore closes a PostUpdate cycle. Camera feature components
                // are extracted after PostUpdate, and apply_review_view already
                // waits for the deferred restore snapshot, so configuring them
                // here after presentation remains same-frame and deterministic.
                .run_if(in_state(Screen::Gameplay)),
        )
        .add_systems(
            Last,
            release_review_lifecycle_reentry
                .run_if(resource_exists::<ReviewLifecycleProjectionReentryPendingV1>),
        )
        .add_systems(OnExit(Screen::Gameplay), restore_review_camera_features);

    if let Some(lifecycle) = lifecycle {
        app.insert_resource(lifecycle);
    }

    if !captures.is_empty() {
        install_capture_sequence(app, captures);
    }
}

#[cfg(test)]
fn install_capture_systems(app: &mut App, capture: ReviewCapture) {
    install_capture_sequence_inner(app, vec![capture], false);
}

fn install_capture_sequence(app: &mut App, captures: Vec<ReviewCapture>) {
    install_capture_sequence_inner(app, captures, true);
}

fn install_capture_sequence_inner(
    app: &mut App,
    captures: Vec<ReviewCapture>,
    install_authority_guard: bool,
) {
    if captures.iter().any(|capture| capture.full_cutaway) {
        hex_world::install_full_cutaway_review_override(app);
    }
    app.insert_resource(ReviewCaptureState::new_many(captures));
    if install_authority_guard {
        app.add_systems(
            OnEnter(Screen::Gameplay),
            capture_review_authority_baseline.in_set(GameplaySetup::Finalize),
        );
    }
    app.add_systems(
        Update,
        (
            capture_watchdog,
            (
                restore_review_camera_features,
                restore_review_capture_camera,
            )
                .chain()
                .run_if(resource_exists::<ReviewLifecycleCycleTeardownPendingV1>),
            finish_review_capture_after_teardown
                .after(restore_review_capture_camera)
                .run_if(in_state(Screen::Title)),
        ),
    )
    .add_systems(
        OnExit(Screen::Gameplay),
        restore_review_capture_camera.after(restore_review_camera_features),
    )
    .add_systems(
        PostUpdate,
        (
            (
                resolve_review_focus_anchor,
                resolve_review_look_at,
                apply_review_view,
                apply_review_illumination_overlay,
            )
                .chain()
                .before(CameraSystems::FollowCharacter)
                .before(TransformSystems::Propagate)
                .before(CameraUpdateSystems),
            pin_review_focus_pose
                .after(CameraSystems::FollowCharacter)
                .before(CameraSystems::FollowPresentation)
                .before(TransformSystems::Propagate)
                .before(CameraUpdateSystems),
            capture_settled_frame
                .after(configure_review_camera_features)
                .after(TransformSystems::Propagate)
                .after(CameraUpdateSystems),
            finish_review_capture_after_teardown
                .after(capture_settled_frame)
                .after(configure_review_camera_features)
                .run_if(resource_exists::<ReviewLifecycleCycleTeardownPendingV1>),
        )
            .run_if(in_state(Screen::Gameplay)),
    );
}

#[derive(Component, Clone)]
struct ReviewCameraFeatureRestore {
    msaa: Msaa,
    depth_texture_usages: Camera3dDepthTextureUsage,
    transmission: ScreenSpaceTransmission,
    oit: Option<OrderIndependentTransparencySettings>,
    volumetric_fog: Option<VolumetricFog>,
}

#[derive(Component)]
struct ReviewAddedVolumetricLight;

fn configure_review_camera_features(
    mut commands: Commands,
    profile: Res<ReviewWorldDetailProfileV1>,
    lifecycle_teardown: Option<Res<ReviewLifecycleCycleTeardownPendingV1>>,
    lifecycle_reentry: Option<Res<ReviewLifecycleProjectionReentryPendingV1>>,
    mut cameras: Query<
        (
            Entity,
            &mut Camera3d,
            &mut Msaa,
            &mut ScreenSpaceTransmission,
            Option<&OrderIndependentTransparencySettings>,
            Option<&VolumetricFog>,
            Option<&ReviewCameraFeatureRestore>,
        ),
        With<PanOrbitCamera>,
    >,
    lights: Query<
        (
            Entity,
            Has<VolumetricLight>,
            Has<ReviewAddedVolumetricLight>,
        ),
        With<DirectionalLight>,
    >,
) {
    if lifecycle_teardown.is_some() || lifecycle_reentry.is_some() {
        return;
    }
    let needs_oit = profile.requires_oit();
    let needs_transmission = profile.requires_transmission();
    let needs_volumetrics = profile.requires_volumetrics();
    if !needs_oit && !needs_transmission && !needs_volumetrics {
        return;
    }

    let Ok((entity, mut camera_3d, mut msaa, mut transmission, oit, volumetric, restore)) =
        cameras.single_mut()
    else {
        return;
    };
    if restore.is_none() {
        commands.entity(entity).insert(ReviewCameraFeatureRestore {
            msaa: *msaa,
            depth_texture_usages: camera_3d.depth_texture_usages,
            transmission: transmission.clone(),
            oit: oit.copied(),
            volumetric_fog: volumetric.copied(),
        });
    }
    if needs_oit || needs_transmission {
        camera_3d.depth_texture_usages.0 |= TextureUsages::TEXTURE_BINDING.bits();
    }
    if needs_oit {
        *msaa = Msaa::Off;
        if oit.is_none() {
            commands
                .entity(entity)
                .insert(OrderIndependentTransparencySettings::default());
        }
    }
    if needs_transmission {
        transmission.steps = 1;
        transmission.quality = ScreenSpaceTransmissionQuality::Medium;
    }
    if needs_volumetrics && volumetric.is_none() {
        commands.entity(entity).insert(VolumetricFog {
            ambient_color: Color::srgb(0.72, 0.78, 0.84),
            ambient_intensity: 0.08,
            jitter: 0.0,
            step_count: 64,
        });
    }

    if needs_volumetrics {
        for (entity, has_volumetric, added) in &lights {
            if !has_volumetric && !added {
                commands
                    .entity(entity)
                    .insert((VolumetricLight, ReviewAddedVolumetricLight));
            }
        }
    }
}

fn release_review_lifecycle_reentry(mut commands: Commands) {
    commands.remove_resource::<ReviewLifecycleProjectionReentryPendingV1>();
}

fn restore_review_camera_features(
    mut commands: Commands,
    mut cameras: Query<(
        Entity,
        &ReviewCameraFeatureRestore,
        &mut Camera3d,
        &mut Msaa,
        &mut ScreenSpaceTransmission,
    )>,
    lights: Query<Entity, With<ReviewAddedVolumetricLight>>,
) {
    for (entity, restore, mut camera_3d, mut msaa, mut transmission) in &mut cameras {
        *msaa = restore.msaa;
        camera_3d.depth_texture_usages = restore.depth_texture_usages;
        *transmission = restore.transmission.clone();
        let mut camera = commands.entity(entity);
        if let Some(oit) = restore.oit {
            camera.insert(oit);
        } else {
            camera.remove::<OrderIndependentTransparencySettings>();
        }
        if let Some(volumetric) = restore.volumetric_fog {
            camera.insert(volumetric);
        } else {
            camera.remove::<VolumetricFog>();
        }
        camera.remove::<ReviewCameraFeatureRestore>();
    }
    for entity in &lights {
        commands
            .entity(entity)
            .remove::<(VolumetricLight, ReviewAddedVolumetricLight)>();
    }
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
    world_detail_profile: ReviewWorldDetailProfileV1,
    capture: Option<ReviewCapture>,
    additional_captures: Vec<ReviewCapture>,
    lifecycle: Option<ReviewLifecycleRequestV1>,
    runtime_receipt: Option<ReviewRuntimeReceiptV1>,
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
        let world_detail_profile = environment_value(WORLD_DETAIL_ENV)?;
        let capture_plan = environment_value(CAPTURE_PLAN_ENV)?;
        let receipt_capture_plan = capture_plan.clone();
        let launch_nonce = environment_value(LAUNCH_NONCE_ENV)?;
        let source_provenance_sha256 = environment_value(SOURCE_PROVENANCE_SHA256_ENV)?;
        let lifecycle = environment_value(LIFECYCLE_ENV)?;
        let has_capture_plan = capture_plan.is_some();
        if capture_plan.is_some()
            && [
                CAPTURE_ENV,
                VIEW_ENV,
                CAMERA_ENV,
                FOCUS_ANCHOR_ENV,
                LOOK_AT_ANCHOR_ENV,
                LOOK_AT_OFFSET_ENV,
                CHARACTER_RADIUS_SCALE_ENV,
                CUTAWAY_ENV,
                ILLUMINATION_ENV,
            ]
            .into_iter()
            .any(|name| env::var_os(name).is_some())
        {
            return Err(format!(
                "{CAPTURE_PLAN_ENV} is mutually exclusive with the single-capture environment"
            ));
        }
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
        .and_then(|request| Self::with_world_detail_profile(request, world_detail_profile))
        .and_then(|request| Self::with_capture_plan(request, capture_plan))
        .and_then(|request| {
            Self::with_runtime_receipt(
                request,
                launch_nonce,
                source_provenance_sha256,
                receipt_capture_plan,
            )
        })
        .and_then(|request| Self::with_lifecycle(request, lifecycle, has_capture_plan))
    }

    fn with_runtime_receipt(
        mut request: Option<Self>,
        launch_nonce: Option<String>,
        source_provenance_sha256: Option<String>,
        raw_capture_plan: Option<String>,
    ) -> Result<Option<Self>, String> {
        let has_automated_captures = request
            .as_ref()
            .is_some_and(|request| !request.capture_sequence().is_empty());
        if !has_automated_captures {
            if launch_nonce.is_some() || source_provenance_sha256.is_some() {
                return Err(format!(
                    "{LAUNCH_NONCE_ENV} and {SOURCE_PROVENANCE_SHA256_ENV} are valid only with automated {CAPTURE_PLAN_ENV} captures"
                ));
            }
            return Ok(request);
        }

        let request_ref = request
            .as_mut()
            .ok_or_else(|| "automated capture lost its review request".to_owned())?;
        let launch_nonce = launch_nonce.ok_or_else(|| {
            format!("automated {CAPTURE_PLAN_ENV} captures require a fresh {LAUNCH_NONCE_ENV}")
        })?;
        let source_provenance_sha256 = source_provenance_sha256.ok_or_else(|| {
            format!("automated {CAPTURE_PLAN_ENV} captures require {SOURCE_PROVENANCE_SHA256_ENV}")
        })?;
        let raw_capture_plan = raw_capture_plan.ok_or_else(|| {
            format!(
                "automated captures require exact UTF-8 {CAPTURE_PLAN_ENV} bytes; legacy single-capture automation cannot publish a runtime receipt"
            )
        })?;
        let process_id = u64::from(std::process::id());
        if process_id == 0 {
            return Err("runtime process identifier is zero".to_owned());
        }
        let profile_sha256 = request_ref
            .world_detail_profile
            .profile_hash_sha256()
            .map_err(|error| format!("cannot hash resolved world-detail profile: {error}"))?;
        let runtime_receipt = ReviewRuntimeReceiptV1::new(
            launch_nonce,
            process_id,
            runtime_executable_sha256()?,
            source_provenance_sha256,
            sha256_hex(raw_capture_plan.as_bytes()),
            profile_sha256,
        )
        .map_err(|error| format!("invalid automated-capture runtime receipt: {error}"))?;
        request_ref.runtime_receipt = Some(runtime_receipt);
        Ok(request)
    }

    fn with_lifecycle(
        mut request: Option<Self>,
        value: Option<String>,
        has_capture_plan: bool,
    ) -> Result<Option<Self>, String> {
        let Some(value) = value else {
            return Ok(request);
        };
        if !has_capture_plan {
            return Err(format!(
                "{LIFECYCLE_ENV} requires {CAPTURE_PLAN_ENV} so every genuine cycle has a capture"
            ));
        }
        let request_ref = request
            .as_mut()
            .ok_or_else(|| format!("{LIFECYCLE_ENV} requires {SCENARIO_ENV}"))?;
        let runtime_receipt = request_ref.runtime_receipt.as_ref().ok_or_else(|| {
            format!("{LIFECYCLE_ENV} requires a validated automated-capture runtime receipt")
        })?;
        let lifecycle = ReviewLifecycleRequestV1::from_canonical_json(&value)?;
        let profile_hash = request_ref
            .world_detail_profile
            .profile_hash_sha256()
            .map_err(|error| format!("{LIFECYCLE_ENV}: cannot hash tested profile: {error}"))?;
        if lifecycle.tested_profile_sha256 != profile_hash {
            return Err(format!(
                "{LIFECYCLE_ENV} tested_profile_sha256 does not match {WORLD_DETAIL_ENV}"
            ));
        }
        if lifecycle.tested_profile_sha256 != runtime_receipt.profile_sha256 {
            return Err(format!(
                "{LIFECYCLE_ENV} tested_profile_sha256 does not match the runtime receipt"
            ));
        }
        if lifecycle.source_provenance_sha256 != runtime_receipt.source_provenance_sha256 {
            return Err(format!(
                "{LIFECYCLE_ENV} source_provenance_sha256 does not match the runtime receipt"
            ));
        }
        let captures = request_ref.capture_sequence();
        if captures.is_empty() {
            return Err(format!("{LIFECYCLE_ENV} requires at least one capture"));
        }
        if captures
            .iter()
            .any(|capture| capture.full_cutaway || capture.illumination_overlay)
        {
            return Err(format!(
                "{LIFECYCLE_ENV} does not permit global cutaway or illumination-overlay mutations"
            ));
        }
        if captures
            .iter()
            .any(|capture| capture.path == lifecycle.certificate_path)
        {
            return Err(format!(
                "{LIFECYCLE_ENV} certificate_path must differ from every capture path"
            ));
        }
        request_ref.lifecycle = Some(lifecycle);
        Ok(request)
    }

    fn with_world_detail_profile(
        mut request: Option<Self>,
        value: Option<String>,
    ) -> Result<Option<Self>, String> {
        let Some(value) = value else {
            return Ok(request);
        };
        let request_ref = request
            .as_mut()
            .ok_or_else(|| format!("{WORLD_DETAIL_ENV} requires {SCENARIO_ENV}"))?;
        request_ref.world_detail_profile = ReviewWorldDetailProfileV1::from_canonical_json(&value)
            .map_err(|error| format!("{WORLD_DETAIL_ENV}: {error}"))?;
        Ok(request)
    }

    fn with_capture_plan(
        mut request: Option<Self>,
        value: Option<String>,
    ) -> Result<Option<Self>, String> {
        let Some(value) = value else {
            return Ok(request);
        };
        let request_ref = request
            .as_mut()
            .ok_or_else(|| format!("{CAPTURE_PLAN_ENV} requires {SCENARIO_ENV}"))?;
        let captures = parse_capture_plan(&value)?;
        let sequenced_phase = captures
            .first()
            .and_then(|capture| capture.liquid_phase_seconds);
        if sequenced_phase.is_some() && request_ref.liquid_phase_seconds.is_some() {
            return Err(format!(
                "{CAPTURE_PLAN_ENV} version 2 per-capture phases are mutually exclusive with {LIQUID_PHASE_ENV}"
            ));
        }
        let mut captures = captures.into_iter();
        request_ref.capture = captures.next();
        request_ref.additional_captures = captures.collect();
        request_ref.liquid_phase_seconds = sequenced_phase
            .or(request_ref.liquid_phase_seconds)
            .or(Some(0.0));
        Ok(request)
    }

    fn capture_sequence(&self) -> Vec<ReviewCapture> {
        self.capture
            .iter()
            .cloned()
            .chain(self.additional_captures.iter().cloned())
            .collect()
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
                    liquid_phase_seconds,
                    settle_frames: SETTLE_FRAMES,
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
            world_detail_profile: ReviewWorldDetailProfileV1::default(),
            capture,
            additional_captures: Vec::new(),
            lifecycle: None,
            runtime_receipt: None,
            launched: false,
        }))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ReviewLifecycleRequestV1 {
    version: u16,
    certificate_path: PathBuf,
    capture_plan_sha256: String,
    source_provenance_sha256: String,
    profile_matrix_sha256: String,
    tested_profile_sha256: String,
    cycles_requested: u16,
}

impl ReviewLifecycleRequestV1 {
    fn from_canonical_json(value: &str) -> Result<Self, String> {
        let request: Self = serde_json::from_str(value)
            .map_err(|error| format!("{LIFECYCLE_ENV} must be strict JSON: {error}"))?;
        let canonical = serde_json::to_string(&request)
            .map_err(|error| format!("{LIFECYCLE_ENV} cannot be canonicalized: {error}"))?;
        if canonical != value {
            return Err(format!(
                "{LIFECYCLE_ENV} must be canonical compact JSON in schema field order"
            ));
        }
        if request.version != 1 {
            return Err(format!(
                "{LIFECYCLE_ENV} version must be 1; got {}",
                request.version
            ));
        }
        if request.cycles_requested != 100 {
            return Err(format!(
                "{LIFECYCLE_ENV} cycles_requested must be exactly 100"
            ));
        }
        if !request.certificate_path.is_absolute()
            || !request
                .certificate_path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
        {
            return Err(format!(
                "{LIFECYCLE_ENV} certificate_path must be an absolute .json path"
            ));
        }
        for (field, value) in [
            ("capture_plan_sha256", &request.capture_plan_sha256),
            (
                "source_provenance_sha256",
                &request.source_provenance_sha256,
            ),
            ("profile_matrix_sha256", &request.profile_matrix_sha256),
            ("tested_profile_sha256", &request.tested_profile_sha256),
        ] {
            if !is_lowercase_sha256(value) {
                return Err(format!(
                    "{LIFECYCLE_ENV} {field} must be 64 lowercase hexadecimal characters"
                ));
            }
        }
        Ok(request)
    }
}

fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn runtime_executable_sha256() -> Result<String, String> {
    let current = env::current_exe()
        .map_err(|error| format!("cannot resolve the current executable path: {error}"))?;
    let canonical = fs::canonicalize(&current).map_err(|error| {
        format!(
            "cannot canonicalize current executable {}: {error}",
            current.display()
        )
    })?;
    let metadata = fs::metadata(&canonical).map_err(|error| {
        format!(
            "cannot inspect canonical current executable {}: {error}",
            canonical.display()
        )
    })?;
    if !metadata.is_file() {
        return Err(format!(
            "canonical current executable is not a regular file: {}",
            canonical.display()
        ));
    }
    let bytes = fs::read(&canonical).map_err(|error| {
        format!(
            "cannot read canonical current executable {}: {error}",
            canonical.display()
        )
    })?;
    Ok(sha256_hex(&bytes))
}

#[expect(
    clippy::indexing_slicing,
    clippy::expect_used,
    reason = "the fixed-width SHA-256 schedule uses algorithm-defined indices, and its supported-target size/String conversions are infallible"
)]
fn sha256_hex(input: &[u8]) -> String {
    const INITIAL: [u32; 8] = [
        0x6a09_e667,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];
    const ROUND: [u32; 64] = [
        0x428a_2f98,
        0x7137_4491,
        0xb5c0_fbcf,
        0xe9b5_dba5,
        0x3956_c25b,
        0x59f1_11f1,
        0x923f_82a4,
        0xab1c_5ed5,
        0xd807_aa98,
        0x1283_5b01,
        0x2431_85be,
        0x550c_7dc3,
        0x72be_5d74,
        0x80de_b1fe,
        0x9bdc_06a7,
        0xc19b_f174,
        0xe49b_69c1,
        0xefbe_4786,
        0x0fc1_9dc6,
        0x240c_a1cc,
        0x2de9_2c6f,
        0x4a74_84aa,
        0x5cb0_a9dc,
        0x76f9_88da,
        0x983e_5152,
        0xa831_c66d,
        0xb003_27c8,
        0xbf59_7fc7,
        0xc6e0_0bf3,
        0xd5a7_9147,
        0x06ca_6351,
        0x1429_2967,
        0x27b7_0a85,
        0x2e1b_2138,
        0x4d2c_6dfc,
        0x5338_0d13,
        0x650a_7354,
        0x766a_0abb,
        0x81c2_c92e,
        0x9272_2c85,
        0xa2bf_e8a1,
        0xa81a_664b,
        0xc24b_8b70,
        0xc76c_51a3,
        0xd192_e819,
        0xd699_0624,
        0xf40e_3585,
        0x106a_a070,
        0x19a4_c116,
        0x1e37_6c08,
        0x2748_774c,
        0x34b0_bcb5,
        0x391c_0cb3,
        0x4ed8_aa4a,
        0x5b9c_ca4f,
        0x682e_6ff3,
        0x748f_82ee,
        0x78a5_636f,
        0x84c8_7814,
        0x8cc7_0208,
        0x90be_fffa,
        0xa450_6ceb,
        0xbef9_a3f7,
        0xc671_78f2,
    ];

    let bit_len = u64::try_from(input.len())
        .expect("in-memory SHA-256 input length fits in u64")
        .wrapping_mul(8);
    let mut padded = Vec::with_capacity(input.len().saturating_add(72));
    padded.extend_from_slice(input);
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    let mut hash = INITIAL;
    for chunk in padded.chunks_exact(64) {
        let mut words = [0_u32; 64];
        for (index, word) in words.iter_mut().take(16).enumerate() {
            let offset = index * 4;
            *word = u32::from_be_bytes([
                chunk[offset],
                chunk[offset + 1],
                chunk[offset + 2],
                chunk[offset + 3],
            ]);
        }
        for index in 16..64 {
            let s0 = words[index - 15].rotate_right(7)
                ^ words[index - 15].rotate_right(18)
                ^ (words[index - 15] >> 3);
            let s1 = words[index - 2].rotate_right(17)
                ^ words[index - 2].rotate_right(19)
                ^ (words[index - 2] >> 10);
            words[index] = words[index - 16]
                .wrapping_add(s0)
                .wrapping_add(words[index - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = hash;
        for index in 0..64 {
            let sum1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choice = (e & f) ^ ((!e) & g);
            let temporary1 = h
                .wrapping_add(sum1)
                .wrapping_add(choice)
                .wrapping_add(ROUND[index])
                .wrapping_add(words[index]);
            let sum0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let temporary2 = sum0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temporary1);
            d = c;
            c = b;
            b = a;
            a = temporary1.wrapping_add(temporary2);
        }
        for (slot, value) in hash.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *slot = slot.wrapping_add(value);
        }
    }

    let mut output = String::with_capacity(64);
    for word in hash {
        use std::fmt::Write as _;
        write!(&mut output, "{word:08x}").expect("writing to String cannot fail");
    }
    output
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
    liquid_phase_seconds: Option<f32>,
    settle_frames: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewCapturePlanV1 {
    version: u32,
    captures: Vec<ReviewCapturePlanEntryV1>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewCapturePlanEntryV1 {
    path: String,
    camera: String,
    view: String,
    #[serde(default)]
    focus_anchor: Option<String>,
    #[serde(default)]
    look_at_anchor: Option<String>,
    #[serde(default)]
    look_at_offset: Option<[f32; 3]>,
    #[serde(default)]
    character_radius_scale: Option<f32>,
    #[serde(default)]
    full_cutaway: bool,
    #[serde(default)]
    illumination_overlay: bool,
    #[serde(default)]
    liquid_phase_seconds: Option<f32>,
    #[serde(default)]
    settle_frames: Option<u32>,
}

#[derive(Resource, Debug)]
struct ReviewLifecycleProbeV1 {
    configuration: ReviewLifecycleRequestV1,
    capture_templates: Vec<ReviewCapture>,
    runtime_receipt: ReviewRuntimeReceiptV1,
    cycles: Vec<ReviewLifecycleCycleV1>,
}

#[derive(Resource, Debug, Default)]
struct ReviewLifecycleCycleTeardownPendingV1;

/// Holds camera feature mutation until the frame after an exact teardown receipt.
#[derive(Resource, Debug, Default)]
struct ReviewLifecycleProjectionReentryPendingV1;

impl ReviewLifecycleProbeV1 {
    fn new(
        configuration: ReviewLifecycleRequestV1,
        capture_templates: Vec<ReviewCapture>,
        runtime_receipt: ReviewRuntimeReceiptV1,
    ) -> Self {
        Self {
            configuration,
            capture_templates,
            runtime_receipt,
            cycles: Vec::with_capacity(100),
        }
    }

    fn next_cycle_index(&self) -> Result<u16, String> {
        u16::try_from(self.cycles.len().saturating_add(1))
            .map_err(|_error| "review lifecycle cycle index exceeds u16".to_owned())
    }

    fn previous_cycle_sha256(&self) -> String {
        self.cycles
            .last()
            .map_or_else(|| "0".repeat(64), |cycle| cycle.cycle_sha256.clone())
    }
}

#[derive(Debug, Clone, Serialize)]
struct ReviewLifecycleCycleHashBodyV1 {
    cycle_index: u16,
    launch_nonce: String,
    runtime_receipt_sha256: String,
    profile_hash_sha256: String,
    authority_before_sha256: String,
    authority_after_sha256: String,
    entities_remaining: u64,
    materials_remaining: u64,
    meshes_remaining: u64,
    fog_density_images_remaining: u64,
    target_images_remaining: u64,
    terrain_material_overrides_remaining: u64,
    liquid_visibility_overrides_remaining: u64,
    vegetation_scale_overrides_remaining: u64,
    camera_state_restored: bool,
    oit_state_restored: bool,
    transmission_state_restored: bool,
    depth_state_restored: bool,
    volumetric_state_restored: bool,
    previous_cycle_sha256: String,
}

#[derive(Debug, Clone, Serialize)]
struct ReviewLifecycleCycleV1 {
    cycle_index: u16,
    launch_nonce: String,
    runtime_receipt_sha256: String,
    profile_hash_sha256: String,
    authority_before_sha256: String,
    authority_after_sha256: String,
    entities_remaining: u64,
    materials_remaining: u64,
    meshes_remaining: u64,
    fog_density_images_remaining: u64,
    target_images_remaining: u64,
    terrain_material_overrides_remaining: u64,
    liquid_visibility_overrides_remaining: u64,
    vegetation_scale_overrides_remaining: u64,
    camera_state_restored: bool,
    oit_state_restored: bool,
    transmission_state_restored: bool,
    depth_state_restored: bool,
    volumetric_state_restored: bool,
    previous_cycle_sha256: String,
    cycle_sha256: String,
}

impl ReviewLifecycleCycleV1 {
    fn from_hash_body(body: ReviewLifecycleCycleHashBodyV1) -> Result<Self, String> {
        let canonical = serde_json::to_vec(&body)
            .map_err(|error| format!("cannot serialize lifecycle cycle hash body: {error}"))?;
        let cycle_sha256 = sha256_hex(&canonical);
        Ok(Self {
            cycle_index: body.cycle_index,
            launch_nonce: body.launch_nonce,
            runtime_receipt_sha256: body.runtime_receipt_sha256,
            profile_hash_sha256: body.profile_hash_sha256,
            authority_before_sha256: body.authority_before_sha256,
            authority_after_sha256: body.authority_after_sha256,
            entities_remaining: body.entities_remaining,
            materials_remaining: body.materials_remaining,
            meshes_remaining: body.meshes_remaining,
            fog_density_images_remaining: body.fog_density_images_remaining,
            target_images_remaining: body.target_images_remaining,
            terrain_material_overrides_remaining: body.terrain_material_overrides_remaining,
            liquid_visibility_overrides_remaining: body.liquid_visibility_overrides_remaining,
            vegetation_scale_overrides_remaining: body.vegetation_scale_overrides_remaining,
            camera_state_restored: body.camera_state_restored,
            oit_state_restored: body.oit_state_restored,
            transmission_state_restored: body.transmission_state_restored,
            depth_state_restored: body.depth_state_restored,
            volumetric_state_restored: body.volumetric_state_restored,
            previous_cycle_sha256: body.previous_cycle_sha256,
            cycle_sha256,
        })
    }
}

#[derive(Debug, Serialize)]
struct ReviewLifecycleCertificateV1<'a> {
    version: u16,
    warning: &'static str,
    runtime_receipt: &'a ReviewRuntimeReceiptV1,
    capture_plan_sha256: &'a str,
    source_provenance_sha256: &'a str,
    profile_matrix_sha256: &'a str,
    tested_profile_sha256: &'a str,
    cycles_requested: u16,
    cycles_completed: u16,
    cycles: &'a [ReviewLifecycleCycleV1],
    final_chain_sha256: &'a str,
}

fn parse_capture_plan(value: &str) -> Result<Vec<ReviewCapture>, String> {
    let plan: ReviewCapturePlanV1 = serde_json::from_str(value)
        .map_err(|error| format!("{CAPTURE_PLAN_ENV} must be strict JSON: {error}"))?;
    if !matches!(plan.version, 1 | 2) {
        return Err(format!(
            "{CAPTURE_PLAN_ENV} version must be 1 or 2; got {}",
            plan.version
        ));
    }
    if plan.captures.is_empty() || plan.captures.len() > 256 {
        return Err(format!(
            "{CAPTURE_PLAN_ENV} must contain 1..=256 captures; got {}",
            plan.captures.len()
        ));
    }

    let Some(first_capture) = plan.captures.first() else {
        return Err(format!(
            "{CAPTURE_PLAN_ENV} must contain at least one capture"
        ));
    };
    let expected_cutaway = first_capture.full_cutaway;
    let expected_illumination = first_capture.illumination_overlay;
    let mut unique_paths = BTreeSet::new();
    let mut captures = Vec::with_capacity(plan.captures.len());
    for (index, entry) in plan.captures.into_iter().enumerate() {
        let ordinal = index + 1;
        let (liquid_phase_seconds, settle_frames) = match plan.version {
            1 => {
                if entry.liquid_phase_seconds.is_some() || entry.settle_frames.is_some() {
                    return Err(format!(
                        "{CAPTURE_PLAN_ENV} capture {ordinal} uses v2 timing fields in a version 1 plan"
                    ));
                }
                (None, SETTLE_FRAMES)
            }
            2 => {
                let phase = entry.liquid_phase_seconds.ok_or_else(|| {
                    format!(
                        "{CAPTURE_PLAN_ENV} version 2 capture {ordinal} requires liquid_phase_seconds"
                    )
                })?;
                if !phase.is_finite() {
                    return Err(format!(
                        "{CAPTURE_PLAN_ENV} version 2 capture {ordinal} liquid_phase_seconds must be finite"
                    ));
                }
                let settle = entry.settle_frames.ok_or_else(|| {
                    format!("{CAPTURE_PLAN_ENV} version 2 capture {ordinal} requires settle_frames")
                })?;
                if !(1..=SETTLE_FRAMES).contains(&settle) {
                    return Err(format!(
                        "{CAPTURE_PLAN_ENV} version 2 capture {ordinal} settle_frames must be in 1..={SETTLE_FRAMES}"
                    ));
                }
                (Some(phase), settle)
            }
            _ => {
                return Err(format!(
                    "{CAPTURE_PLAN_ENV} version changed after validation; got {}",
                    plan.version
                ));
            }
        };
        if entry.full_cutaway != expected_cutaway
            || entry.illumination_overlay != expected_illumination
        {
            return Err(format!(
                "{CAPTURE_PLAN_ENV} capture {ordinal} changes full_cutaway or illumination_overlay; these global presentation modes must remain constant within one launch"
            ));
        }
        if entry.path.trim().is_empty() {
            return Err(format!(
                "{CAPTURE_PLAN_ENV} capture {ordinal} path must not be empty"
            ));
        }
        let path = PathBuf::from(entry.path);
        if !path.is_absolute() {
            return Err(format!(
                "{CAPTURE_PLAN_ENV} capture {ordinal} path must be absolute"
            ));
        }
        if !path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
        {
            return Err(format!(
                "{CAPTURE_PLAN_ENV} capture {ordinal} path must name a .png file"
            ));
        }
        if !unique_paths.insert(path.clone()) {
            return Err(format!(
                "{CAPTURE_PLAN_ENV} capture {ordinal} duplicates output path {}",
                path.display()
            ));
        }

        let camera = ReviewCamera::parse(&entry.camera)?;
        let view = ReviewView::parse(&entry.view)?;
        let focus_anchor = entry
            .focus_anchor
            .map(|anchor| {
                if anchor.trim().is_empty() {
                    Err(format!(
                        "{CAPTURE_PLAN_ENV} capture {ordinal} focus_anchor must not be empty"
                    ))
                } else {
                    Ok(anchor)
                }
            })
            .transpose()?;
        let anchor_look_at = match (entry.look_at_anchor, entry.look_at_offset) {
            (None, None) => None,
            (Some(anchor), Some(offset)) => {
                if anchor.trim().is_empty() {
                    return Err(format!(
                        "{CAPTURE_PLAN_ENV} capture {ordinal} look_at_anchor must not be empty"
                    ));
                }
                if camera != ReviewCamera::Map {
                    return Err(format!(
                        "{CAPTURE_PLAN_ENV} capture {ordinal} look_at_anchor requires camera=map"
                    ));
                }
                if focus_anchor.is_some() {
                    return Err(format!(
                        "{CAPTURE_PLAN_ENV} capture {ordinal} cannot combine focus_anchor and look_at_anchor"
                    ));
                }
                let offset = Vec3::from_array(offset);
                let distance = offset.length();
                if !offset.is_finite()
                    || !distance.is_finite()
                    || !(1.0..=2_048.0).contains(&distance)
                {
                    return Err(format!(
                        "{CAPTURE_PLAN_ENV} capture {ordinal} look_at_offset must be finite and 1..=2048 world units from its anchor"
                    ));
                }
                Some(ReviewAnchorLookAt { anchor, offset })
            }
            _ => {
                return Err(format!(
                    "{CAPTURE_PLAN_ENV} capture {ordinal} must provide look_at_anchor and look_at_offset together"
                ));
            }
        };
        let character_radius_scale = entry.character_radius_scale.unwrap_or(1.0);
        if !character_radius_scale.is_finite() || !(1.0..=20.0).contains(&character_radius_scale) {
            return Err(format!(
                "{CAPTURE_PLAN_ENV} capture {ordinal} character_radius_scale must be finite and in [1, 20]"
            ));
        }
        if character_radius_scale.to_bits() != 1.0_f32.to_bits()
            && camera != ReviewCamera::Character
        {
            return Err(format!(
                "{CAPTURE_PLAN_ENV} capture {ordinal} character_radius_scale requires camera=character"
            ));
        }

        captures.push(ReviewCapture {
            path,
            view,
            camera,
            focus_anchor,
            anchor_look_at,
            character_radius_scale,
            full_cutaway: entry.full_cutaway,
            illumination_overlay: entry.illumination_overlay,
            liquid_phase_seconds,
            settle_frames,
        });
    }
    Ok(captures)
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
    const fn as_str(self) -> &'static str {
        match self {
            Self::Map => "map",
            Self::Character => "character",
            Self::FirstPerson => "first-person",
        }
    }

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
    const fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Rotated => "rotated",
            Self::CounterRotated => "counter-rotated",
            Self::Rear => "rear",
            Self::TopDown => "top-down",
        }
    }

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

#[derive(Debug, Clone)]
struct ReviewFocusPose {
    transform: Transform,
    orbit_focus: Vec3,
    orbit_radius: f32,
}

#[derive(Resource, Debug)]
struct ReviewCaptureState {
    capture: ReviewCapture,
    remaining: VecDeque<ReviewCapture>,
    completed_captures: usize,
    total_captures: usize,
    target: Option<Handle<Image>>,
    camera_snapshot: Option<ReviewCameraSnapshot>,
    teardown_requested: bool,
    camera_restored: bool,
    target_removed: bool,
    final_exit_sent: bool,
    focus_relocated: bool,
    focus_world_target: Option<Vec3>,
    focus_pose: Option<ReviewFocusPose>,
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
    authority_baseline: Option<ReviewAuthoritySnapshotV1>,
    authority_validated_captures: usize,
    authority_pre_teardown_verified: bool,
    authority_after_sha256: Option<String>,
    review_entity_ids: BTreeSet<Entity>,
    review_mesh_ids: BTreeSet<AssetId<Mesh>>,
    review_standard_material_ids: BTreeSet<AssetId<StandardMaterial>>,
    review_image_ids: BTreeSet<AssetId<Image>>,
    capture_target_ids: BTreeSet<AssetId<Image>>,
    runtime_report_paths: Vec<PathBuf>,
    performance_frame_window_ms: VecDeque<f32>,
    performance_resident_bytes: Option<u64>,
    performance_sample: Option<ReviewPerformanceSampleV1>,
}

#[derive(Clone)]
struct ReviewCameraSnapshot {
    entity: Entity,
    transform: Transform,
    orbit_focus: Vec3,
    orbit_radius: f32,
    target: RenderTarget,
    projection: Option<Projection>,
    mode: CameraMode,
    msaa: Msaa,
    depth_texture_usages: Camera3dDepthTextureUsage,
    transmission_steps: usize,
    transmission_quality: ScreenSpaceTransmissionQuality,
    oit: Option<OrderIndependentTransparencySettings>,
    volumetric_fog: Option<VolumetricFog>,
}

impl fmt::Debug for ReviewCameraSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReviewCameraSnapshot")
            .field("entity", &self.entity)
            .field("target", &self.target)
            .field("mode", &self.mode)
            .field("msaa", &self.msaa)
            .field("oit_present", &self.oit.is_some())
            .field("volumetric_fog_present", &self.volumetric_fog.is_some())
            .finish_non_exhaustive()
    }
}

impl ReviewCaptureState {
    #[cfg(test)]
    fn new(capture: ReviewCapture) -> Self {
        Self::new_many(vec![capture])
    }

    #[expect(
        clippy::expect_used,
        reason = "this private constructor's non-empty capture-sequence invariant is enforced by every caller"
    )]
    fn new_many(captures: Vec<ReviewCapture>) -> Self {
        assert!(
            !captures.is_empty(),
            "review capture sequence must not be empty"
        );
        let total_captures = captures.len();
        let mut captures = VecDeque::from(captures);
        let capture = captures
            .pop_front()
            .expect("non-empty review capture sequence lost its first capture");
        let focus_relocated = capture.focus_anchor.is_none();
        let anchor_look_at_resolved = capture.anchor_look_at.is_none();
        let illumination_overlay_applied = !capture.illumination_overlay;
        Self {
            capture,
            remaining: captures,
            completed_captures: 0,
            total_captures,
            target: None,
            camera_snapshot: None,
            teardown_requested: false,
            camera_restored: false,
            target_removed: false,
            final_exit_sent: false,
            focus_relocated,
            focus_world_target: None,
            focus_pose: None,
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
            authority_baseline: None,
            authority_validated_captures: 0,
            authority_pre_teardown_verified: false,
            authority_after_sha256: None,
            review_entity_ids: BTreeSet::new(),
            review_mesh_ids: BTreeSet::new(),
            review_standard_material_ids: BTreeSet::new(),
            review_image_ids: BTreeSet::new(),
            capture_target_ids: BTreeSet::new(),
            runtime_report_paths: Vec::with_capacity(total_captures),
            performance_frame_window_ms: VecDeque::with_capacity(PERFORMANCE_WINDOW_FRAMES),
            performance_resident_bytes: None,
            performance_sample: None,
        }
    }

    fn advance_capture(&mut self, now: Instant) -> Option<PathBuf> {
        self.completed_captures = self.completed_captures.saturating_add(1);
        let next = self.remaining.pop_front()?;
        let illumination_overlay_already_applied = self.illumination_overlay_applied;
        self.capture = next;
        self.focus_relocated = self.capture.focus_anchor.is_none();
        self.focus_world_target = None;
        self.focus_pose = None;
        self.anchor_look_at_target = None;
        self.anchor_look_at_resolved = self.capture.anchor_look_at.is_none();
        self.view_applied = false;
        self.illumination_overlay_applied =
            !self.capture.illumination_overlay || illumination_overlay_already_applied;
        self.settled_frames = 0;
        self.requested = false;
        self.phase = CapturePhase::AwaitingCamera;
        self.phase_started = now;
        self.visible_tiles = 0;
        self.total_tiles = 0;
        self.coverage_warning_logged = false;
        self.full_footprint_validated = false;
        Some(self.capture.path.clone())
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
    AwaitingTeardown,
}

impl CapturePhase {
    const fn timeout(self) -> Duration {
        match self {
            Self::Readback | Self::AwaitingTeardown => READBACK_TIMEOUT,
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
            Self::AwaitingTeardown => "review teardown and camera restoration",
        }
    }
}

const AUTHORITY_FINGERPRINT_DOMAIN: &[u8] = b"crystal-ascent-review-authority-v1\0";

/// Immutable gameplay evidence captured after ordinary setup has published terrain,
/// actors, illumination, and faction knowledge, but before review projection runs.
///
/// Each section retains its canonical byte stream rather than only a digest. That
/// makes equality collision-independent and lets a failed capture name the first
/// authority surface that moved. Entity ids are included only for live ECS linkage;
/// all collections are first sorted by their stable domain keys.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ReviewAuthoritySnapshotV1 {
    current_world: Vec<u8>,
    replication: Vec<u8>,
    lighting_condition: Vec<u8>,
    illumination: Vec<u8>,
    faction_knowledge: Vec<u8>,
    units_and_occupancy: Vec<u8>,
    logical_terrain: Vec<u8>,
    terrain_picking: Vec<u8>,
    persistence: Vec<u8>,
}

impl ReviewAuthoritySnapshotV1 {
    fn capture(evidence: &ReviewAuthorityEvidence<'_, '_>) -> Result<Self, String> {
        validate_authority_terrain_projection(evidence)?;
        Ok(Self {
            current_world: encode_current_world_authority(evidence)?,
            replication: encode_replication_authority(evidence)?,
            lighting_condition: encode_lighting_condition_authority(evidence)?,
            illumination: encode_resolved_illumination_authority(evidence)?,
            faction_knowledge: encode_faction_knowledge_authority(evidence)?,
            units_and_occupancy: encode_unit_authority(evidence)?,
            logical_terrain: encode_logical_terrain_authority(evidence)?,
            terrain_picking: encode_terrain_picking_authority(evidence)?,
            persistence: encode_persistence_authority(
                evidence.campaign_store.as_deref(),
                evidence.campaign_save_status.as_deref(),
                evidence.storage_paths.as_deref(),
            )?,
        })
    }

    fn sections(&self) -> [(&'static str, &[u8]); 9] {
        [
            ("current_world", &self.current_world),
            ("replication", &self.replication),
            ("lighting_condition", &self.lighting_condition),
            ("resolved_illumination", &self.illumination),
            ("faction_knowledge", &self.faction_knowledge),
            ("units_and_occupancy", &self.units_and_occupancy),
            ("logical_terrain", &self.logical_terrain),
            ("terrain_picking", &self.terrain_picking),
            ("persistence", &self.persistence),
        ]
    }

    #[expect(
        clippy::expect_used,
        reason = "the fixed eight-section table and in-memory section lengths fit their canonical widths on every supported target"
    )]
    fn canonical_bytes(&self) -> Vec<u8> {
        let mut bytes = AUTHORITY_FINGERPRINT_DOMAIN.to_vec();
        for (ordinal, (_, section)) in self.sections().into_iter().enumerate() {
            bytes.push(u8::try_from(ordinal + 1).expect("authority section ordinal fits in u8"));
            bytes.extend_from_slice(
                &u64::try_from(section.len())
                    .expect("an in-memory authority section length fits in u64")
                    .to_le_bytes(),
            );
            bytes.extend_from_slice(section);
        }
        bytes
    }

    fn fingerprint(&self) -> String {
        format!(
            "{:016x}",
            xxhash_rust::xxh3::xxh3_64(&self.canonical_bytes())
        )
    }

    fn sha256(&self) -> String {
        sha256_hex(&self.canonical_bytes())
    }

    fn lighting_condition_key(&self) -> String {
        format!(
            "{:016x}",
            xxhash_rust::xxh3::xxh3_64(&self.lighting_condition)
        )
    }

    fn logical_terrain_picking_fingerprint(&self) -> String {
        fingerprint_authority_subset(
            b"crystal-ascent-review-logical-terrain-picking-v1\0",
            &[&self.logical_terrain, &self.terrain_picking],
        )
    }

    fn gameplay_state_fingerprint(&self) -> String {
        fingerprint_authority_subset(
            b"crystal-ascent-review-gameplay-state-v1\0",
            &[
                &self.replication,
                &self.lighting_condition,
                &self.illumination,
                &self.faction_knowledge,
                &self.persistence,
            ],
        )
    }

    fn verify_unchanged(&self, current: &Self) -> Result<(), String> {
        for ((name, baseline), (_, candidate)) in
            self.sections().into_iter().zip(current.sections())
        {
            if baseline != candidate {
                return Err(format!(
                    "review authority changed in {name} under lighting condition {} \
                     (baseline={:016x}, current={:016x})",
                    self.lighting_condition_key(),
                    xxhash_rust::xxh3::xxh3_64(baseline),
                    xxhash_rust::xxh3::xxh3_64(candidate),
                ));
            }
        }
        Ok(())
    }
}

#[expect(
    clippy::expect_used,
    reason = "the fixed authority subsets and their in-memory byte lengths fit u64"
)]
fn fingerprint_authority_subset(domain: &[u8], sections: &[&[u8]]) -> String {
    let mut bytes = domain.to_vec();
    for (ordinal, section) in sections.iter().enumerate() {
        bytes.extend_from_slice(
            &u64::try_from(ordinal + 1)
                .expect("authority subset ordinal fits in u64")
                .to_le_bytes(),
        );
        bytes.extend_from_slice(
            &u64::try_from(section.len())
                .expect("authority subset length fits in u64")
                .to_le_bytes(),
        );
        bytes.extend_from_slice(section);
    }
    format!("{:016x}", xxhash_rust::xxh3::xxh3_64(&bytes))
}

#[derive(Default)]
struct CanonicalAuthorityEncoder {
    bytes: Vec<u8>,
}

impl CanonicalAuthorityEncoder {
    fn section(tag: u8) -> Self {
        let mut encoder = Self::default();
        encoder
            .bytes
            .extend_from_slice(AUTHORITY_FINGERPRINT_DOMAIN);
        encoder.bytes.push(tag);
        encoder
    }

    fn finish(self) -> Vec<u8> {
        self.bytes
    }

    fn presence(&mut self, present: bool) {
        self.bytes.push(u8::from(present));
    }

    fn boolean(&mut self, value: bool) {
        self.bytes.push(u8::from(value));
    }

    fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    fn u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn i32(&mut self, value: i32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn count(&mut self, value: usize, label: &str) -> Result<(), String> {
        let value = u64::try_from(value)
            .map_err(|_error| format!("{label} count cannot be represented canonically as u64"))?;
        self.u64(value);
        Ok(())
    }

    fn byte_slice(&mut self, value: &[u8], label: &str) -> Result<(), String> {
        self.count(value.len(), label)?;
        self.bytes.extend_from_slice(value);
        Ok(())
    }

    fn entity(&mut self, entity: Entity) {
        self.u64(entity.to_bits());
    }

    fn tile_pos(&mut self, position: TilePos) {
        self.i32(position.coord.x());
        self.i32(position.coord.y());
        self.i32(position.level);
    }

    fn span(&mut self, span: HexSpan, label: &str) -> Result<(), String> {
        if !span.bottom.is_finite() || !span.top.is_finite() || span.top <= span.bottom {
            return Err(format!("{label} has malformed span {span:?}"));
        }
        self.u32(span.bottom.to_bits());
        self.u32(span.top.to_bits());
        Ok(())
    }

    fn pickable(&mut self, pickable: Option<Pickable>) {
        self.presence(pickable.is_some());
        if let Some(pickable) = pickable {
            self.boolean(pickable.should_block_lower);
            self.boolean(pickable.is_hoverable);
        }
    }
}

/// Ordinary batches retain one live material; an explicitly suppressed review
/// water batch retains its saved original material owner instead. Share this
/// classification between picking authority and capture readiness so arbitrary
/// unbound terrain and duplicate visible bindings still fail closed.
type ReviewTerrainMaterialBindings = (
    Has<MeshMaterial3d<StandardMaterial>>,
    Has<MeshMaterial3d<ReviewLiquidMaterial>>,
    Has<ReviewSuppressedWaterMaterial>,
);

type ReviewCaptureTerrainBatchQuery = (
    Entity,
    &'static TerrainRenderBatch,
    Option<&'static Mesh3d>,
    ReviewTerrainMaterialBindings,
    Option<&'static ViewVisibility>,
);

const fn review_terrain_material_tag(
    (standard, liquid, suppressed_water): (bool, bool, bool),
) -> Option<u8> {
    match (standard, liquid, suppressed_water) {
        (true, false, false) => Some(0),
        (false, true, false) | (false, false, true) => Some(1),
        _ => None,
    }
}

#[derive(SystemParam)]
struct ReviewAuthorityEvidence<'w, 's> {
    world_snapshot: Option<Res<'w, CurrentWorldSnapshotV1>>,
    replication: Option<Res<'w, WorldReplicationStateV1>>,
    time_of_day: Option<Res<'w, TimeOfDay>>,
    exterior_illumination: Option<Res<'w, ExteriorIllumination>>,
    illumination: Option<Res<'w, ResolvedIllumination>>,
    knowledge: Option<Res<'w, FactionMapKnowledge>>,
    unit_registry: Option<Res<'w, UnitRegistry>>,
    campaign_store: Option<Res<'w, CampaignStore>>,
    campaign_save_status: Option<Res<'w, CampaignSaveStatusProjection>>,
    storage_paths: Option<Res<'w, StoragePaths>>,
    units: Query<
        'w,
        's,
        (
            Entity,
            &'static Faction,
            Option<&'static Body>,
            Option<&'static StandsOn>,
            Option<&'static MovingTo>,
            Has<Downed>,
        ),
    >,
    logical_terrain: Query<
        'w,
        's,
        (
            Entity,
            Option<&'static TilePos>,
            Option<&'static RunBottom>,
            Option<&'static HexSpan>,
            Option<&'static SubstanceId>,
            Option<&'static Headroom>,
            Option<&'static Pickable>,
        ),
        With<HexTile>,
    >,
    terrain_batches: Query<
        'w,
        's,
        (
            Entity,
            &'static TerrainRenderBatch,
            Option<&'static Pickable>,
            Has<Mesh3d>,
            ReviewTerrainMaterialBindings,
        ),
    >,
}

fn encode_current_world_authority(
    evidence: &ReviewAuthorityEvidence<'_, '_>,
) -> Result<Vec<u8>, String> {
    let mut encoder = CanonicalAuthorityEncoder::section(1);
    encoder.presence(evidence.world_snapshot.is_some());
    let snapshot = evidence
        .world_snapshot
        .as_deref()
        .ok_or_else(|| "CurrentWorldSnapshotV1 is unavailable at the review boundary".to_owned())?;
    encoder.u64(snapshot.fingerprint().0);
    Ok(encoder.finish())
}

fn encode_replication_authority(
    evidence: &ReviewAuthorityEvidence<'_, '_>,
) -> Result<Vec<u8>, String> {
    let mut encoder = CanonicalAuthorityEncoder::section(2);
    encoder.presence(evidence.replication.is_some());
    let replication = evidence.replication.as_deref().ok_or_else(|| {
        "WorldReplicationStateV1 is unavailable at the review boundary".to_owned()
    })?;
    encoder.presence(replication.last_applied_sequence().is_some());
    if let Some(sequence) = replication.last_applied_sequence() {
        encoder.u64(sequence.0);
    }
    Ok(encoder.finish())
}

fn encode_persistence_authority(
    campaign_store: Option<&CampaignStore>,
    campaign_save_status: Option<&CampaignSaveStatusProjection>,
    storage_paths: Option<&StoragePaths>,
) -> Result<Vec<u8>, String> {
    let mut encoder = CanonicalAuthorityEncoder::section(9);
    encoder.presence(campaign_store.is_some());
    let store = campaign_store
        .ok_or_else(|| "CampaignStore is unavailable at the review boundary".to_owned())?;
    // CampaignStore intentionally exposes no typed read API outside its persistence
    // owner. Retain its complete, exact Debug projection alongside the authoritative
    // file bytes below; this is an equality guard, not a claimed stable digest.
    encoder.byte_slice(
        format!("{store:?}").as_bytes(),
        "CampaignStore Debug projection",
    )?;

    encoder.presence(campaign_save_status.is_some());
    let status = campaign_save_status.ok_or_else(|| {
        "CampaignSaveStatusProjection is unavailable at the review boundary".to_owned()
    })?;
    encoder.u64(status.operation_id);
    encoder.presence(status.state.is_some());
    if let Some(state) = status.state {
        match state {
            CampaignSaveStateV2::Saving => encoder.u8(0),
            CampaignSaveStateV2::Saved => encoder.u8(1),
            CampaignSaveStateV2::Refused(refusal) => {
                encoder.u8(2);
                encoder.u8(match refusal {
                    CampaignSaveRefusalV2::NotAuthority => 0,
                    CampaignSaveRefusalV2::UnsafeBoundary => 1,
                    CampaignSaveRefusalV2::IncompleteCheckpoint => 2,
                    CampaignSaveRefusalV2::IncompatibleContent => 3,
                    CampaignSaveRefusalV2::StorageUnavailable => 4,
                });
            }
        }
    }

    encoder.presence(storage_paths.is_some());
    let paths = storage_paths
        .ok_or_else(|| "StoragePaths is unavailable at the review boundary".to_owned())?;
    match fs::read(&paths.campaigns) {
        Ok(bytes) => {
            encoder.presence(true);
            encoder.byte_slice(&bytes, "campaigns file")?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            encoder.presence(false);
        }
        Err(error) => {
            return Err(format!(
                "cannot read configured campaigns file {}: {error}",
                paths.campaigns.display()
            ));
        }
    }
    Ok(encoder.finish())
}

fn encode_lighting_condition_authority(
    evidence: &ReviewAuthorityEvidence<'_, '_>,
) -> Result<Vec<u8>, String> {
    let mut encoder = CanonicalAuthorityEncoder::section(3);
    encoder.presence(evidence.time_of_day.is_some());
    if let Some(time) = evidence.time_of_day.as_deref() {
        if !time.hours.is_finite() || !(0.0..24.0).contains(&time.hours) {
            return Err(format!(
                "TimeOfDay hours must be finite and in 0.0..24.0; got {}",
                time.hours
            ));
        }
        encoder.u32(time.hours.to_bits());
    }
    encoder.presence(evidence.exterior_illumination.is_some());
    let exterior = evidence
        .exterior_illumination
        .as_deref()
        .ok_or_else(|| "ExteriorIllumination is unavailable at the review boundary".to_owned())?;
    encoder.u8(illumination_level_tag(exterior.level));
    Ok(encoder.finish())
}

fn encode_resolved_illumination_authority(
    evidence: &ReviewAuthorityEvidence<'_, '_>,
) -> Result<Vec<u8>, String> {
    let mut encoder = CanonicalAuthorityEncoder::section(4);
    encoder.presence(evidence.illumination.is_some());
    let illumination = evidence
        .illumination
        .as_deref()
        .ok_or_else(|| "ResolvedIllumination is unavailable at the review boundary".to_owned())?;
    if illumination.is_empty() {
        return Err("ResolvedIllumination is empty at the review boundary".to_owned());
    }
    encoder.count(illumination.len(), "resolved illumination")?;
    for (position, resolved) in illumination.iter() {
        encoder.tile_pos(position);
        encoder.u8(illumination_level_tag(resolved.level));
        encode_light_domain(&mut encoder, resolved.domain);
    }
    Ok(encoder.finish())
}

fn encode_faction_knowledge_authority(
    evidence: &ReviewAuthorityEvidence<'_, '_>,
) -> Result<Vec<u8>, String> {
    let mut encoder = CanonicalAuthorityEncoder::section(5);
    encoder.presence(evidence.knowledge.is_some());
    let knowledge = evidence
        .knowledge
        .as_deref()
        .ok_or_else(|| "FactionMapKnowledge is unavailable at the review boundary".to_owned())?;
    for faction in [Faction::Player, Faction::Hostile] {
        encoder.u8(faction_tag(faction));
        let faction_knowledge = knowledge.faction(faction);
        encoder.count(faction_knowledge.surface_count(), "known surface")?;
        for (position, known) in faction_knowledge.surfaces() {
            if known.state() == KnowledgeState::Unknown {
                return Err(format!(
                    "{faction:?} faction knowledge stores an Unknown surface at {position:?}"
                ));
            }
            if known.snapshot().pos != position || known.run_bottom().0 > position.level {
                return Err(format!(
                    "{faction:?} faction knowledge has an inconsistent surface tuple at {position:?}"
                ));
            }
            encoder.tile_pos(position);
            encoder.u8(knowledge_state_tag(known.state()));
            encoder.i32(known.run_bottom().0);
            let surface = known.snapshot();
            encoder.tile_pos(surface.pos);
            encoder.span(surface.span, "known terrain surface")?;
            encoder.u16(surface.substance.0);
            encoder.i32(surface.headroom.0);
            encoder.boolean(surface.is_solid);
            encoder.boolean(surface.blocked);
            encode_light_domain(&mut encoder, surface.domain);
        }
        encoder.count(faction_knowledge.unit_count(), "known unit")?;
        for (id, unit) in faction_knowledge.units() {
            if unit.id != id {
                return Err(format!(
                    "{faction:?} faction knowledge key {} disagrees with observed unit id {}",
                    id.0, unit.id.0
                ));
            }
            encoder.u64(id.0);
            encoder.u64(unit.id.0);
            encoder.u8(faction_tag(unit.faction));
            encoder.tile_pos(unit.pos);
            encoder.boolean(unit.provides_sight);
        }
    }
    Ok(encoder.finish())
}

fn encode_unit_authority(evidence: &ReviewAuthorityEvidence<'_, '_>) -> Result<Vec<u8>, String> {
    let mut encoder = CanonicalAuthorityEncoder::section(6);
    encoder.presence(evidence.unit_registry.is_some());
    let registry = evidence
        .unit_registry
        .as_deref()
        .ok_or_else(|| "UnitRegistry is unavailable at the review boundary".to_owned())?;
    let registered = registry.iter().collect::<Vec<_>>();
    if registered.is_empty() {
        return Err("UnitRegistry is empty at the review boundary".to_owned());
    }
    encoder.count(registered.len(), "registered unit")?;
    let mut registered_entities = BTreeSet::new();
    let mut occupancy = BTreeMap::<TilePos, BTreeSet<u64>>::new();
    for (id, entity) in registered {
        if !registered_entities.insert(entity) {
            return Err(format!(
                "UnitRegistry maps more than one stable id to entity {entity:?}"
            ));
        }
        if registry.id_of(entity) != Some(id) {
            return Err(format!(
                "UnitRegistry reverse mapping disagrees for unit {} on {entity:?}",
                id.0
            ));
        }
        let (_, faction, body, stands_on, moving_to, downed) =
            evidence.units.get(entity).map_err(|_error| {
                format!(
                    "UnitRegistry unit {} maps to {entity:?}, which is not a live Faction entity",
                    id.0
                )
            })?;
        encoder.u64(id.0);
        encoder.entity(entity);
        encoder.u8(faction_tag(*faction));

        encoder.presence(body.is_some());
        let body = body.ok_or_else(|| format!("registered unit {} has no Body", id.0))?;
        let traversal = body.traversal_profile();
        encoder.i32(traversal.levels_tall);
        encoder.i32(traversal.max_climb);
        encoder.i32(traversal.max_drop);

        encoder.presence(stands_on.is_some());
        let standing = stands_on
            .ok_or_else(|| format!("registered unit {} has no authoritative StandsOn", id.0))?
            .0;
        encode_standing(&mut encoder, standing, "registered unit standing")?;
        occupancy.entry(standing.pos).or_default().insert(id.0);

        encoder.presence(moving_to.is_some());
        if let Some(moving) = moving_to {
            if moving.path.is_empty()
                || !moving.speed().is_finite()
                || !moving.elapsed().is_finite()
                || moving.elapsed() < 0.0
                || moving.reconciled_step() >= moving.path.len()
            {
                return Err(format!(
                    "registered unit {} has malformed authoritative MovingTo state",
                    id.0
                ));
            }
            encoder.count(moving.path.len(), "movement path")?;
            for step in &moving.path {
                encode_standing(&mut encoder, *step, "movement path step")?;
                occupancy.entry(step.pos).or_default().insert(id.0);
            }
            encoder.u32(moving.speed().to_bits());
            encoder.u64(moving.elapsed().to_bits());
            encoder.boolean(moving.started());
            encoder.count(moving.reconciled_step(), "reconciled movement step")?;
        }
        encoder.boolean(downed);
    }

    let live_units = evidence
        .units
        .iter()
        .map(|(entity, _, _, _, _, _)| entity)
        .collect::<BTreeSet<_>>();
    if live_units != registered_entities {
        return Err(format!(
            "UnitRegistry/live Faction entity mismatch (registry={}, live={})",
            registered_entities.len(),
            live_units.len()
        ));
    }

    encoder.count(occupancy.len(), "occupied surface")?;
    for (position, occupants) in occupancy {
        encoder.tile_pos(position);
        encoder.count(occupants.len(), "surface occupant")?;
        for unit in occupants {
            encoder.u64(unit);
        }
    }
    Ok(encoder.finish())
}

fn encode_logical_terrain_authority(
    evidence: &ReviewAuthorityEvidence<'_, '_>,
) -> Result<Vec<u8>, String> {
    let mut runs = Vec::new();
    for (entity, position, run_bottom, span, substance, headroom, pickable) in
        &evidence.logical_terrain
    {
        let position = position
            .copied()
            .ok_or_else(|| format!("logical HexTile {entity:?} has no authoritative TilePos"))?;
        let run_bottom = run_bottom
            .copied()
            .ok_or_else(|| format!("logical HexTile {entity:?} has no RunBottom"))?;
        let span = span
            .copied()
            .ok_or_else(|| format!("logical HexTile {entity:?} has no HexSpan"))?;
        let substance = substance
            .copied()
            .ok_or_else(|| format!("logical HexTile {entity:?} has no SubstanceId"))?;
        let headroom = headroom
            .copied()
            .ok_or_else(|| format!("logical HexTile {entity:?} has no Headroom"))?;
        runs.push((
            position,
            entity,
            run_bottom,
            span,
            substance,
            headroom,
            pickable.copied(),
        ));
    }
    if runs.is_empty() {
        return Err("no logical HexTile runs exist at the review boundary".to_owned());
    }
    runs.sort_by_key(|(position, entity, ..)| (*position, entity.to_bits()));

    let mut encoder = CanonicalAuthorityEncoder::section(7);
    encoder.count(runs.len(), "logical terrain run")?;
    for (position, entity, run_bottom, span, substance, headroom, pickable) in runs {
        encoder.tile_pos(position);
        encoder.entity(entity);
        encoder.i32(run_bottom.0);
        encoder.span(span, "logical terrain run")?;
        encoder.u16(substance.0);
        encoder.i32(headroom.0);
        encoder.pickable(pickable);
    }
    Ok(encoder.finish())
}

fn encode_terrain_picking_authority(
    evidence: &ReviewAuthorityEvidence<'_, '_>,
) -> Result<Vec<u8>, String> {
    let mut batches = evidence.terrain_batches.iter().collect::<Vec<_>>();
    if batches.is_empty() {
        return Err("no TerrainRenderBatch entities exist at the review boundary".to_owned());
    }
    batches.sort_by_key(|(entity, batch, _, _, _)| {
        let chunk = batch.chunk();
        (chunk.q, chunk.r, batch.substance().0, entity.to_bits())
    });

    let mut encoder = CanonicalAuthorityEncoder::section(8);
    encoder.count(batches.len(), "terrain render batch")?;
    for (entity, batch, pickable, has_mesh, material_bindings) in batches {
        if !has_mesh {
            return Err(format!(
                "TerrainRenderBatch {entity:?} is missing its mesh handle"
            ));
        }
        let material_tag = review_terrain_material_tag(material_bindings).ok_or_else(|| {
            format!(
                "TerrainRenderBatch {entity:?} requires exactly one ordinary material binding or exclusive review water suppression; found {material_bindings:?}"
            )
        })?;
        let Some(pickable) = pickable.copied() else {
            return Err(format!(
                "TerrainRenderBatch {entity:?} has no explicit Pickable state"
            ));
        };
        if pickable != Pickable::default() {
            return Err(format!(
                "TerrainRenderBatch {entity:?} no longer has ordinary interactive Pickable state"
            ));
        }
        let chunk = batch.chunk();
        encoder.i32(chunk.q);
        encoder.i32(chunk.r);
        encoder.u16(batch.substance().0);
        encoder.entity(entity);
        encoder.boolean(has_mesh);
        encoder.boolean(true);
        encoder.u8(material_tag);
        encoder.pickable(Some(pickable));
        let runs = batch.runs().collect::<Vec<_>>();
        encoder.count(runs.len(), "terrain pick run")?;
        for run in runs {
            encoder.entity(run.entity());
            encoder.tile_pos(run.position());
            encoder.span(run.span(), "terrain pick run")?;
        }
    }
    Ok(encoder.finish())
}

fn validate_authority_terrain_projection(
    evidence: &ReviewAuthorityEvidence<'_, '_>,
) -> Result<(), String> {
    let logical = evidence
        .logical_terrain
        .iter()
        .map(|(entity, position, _, span, _, _, _)| (entity, position.copied(), span.copied()))
        .collect::<Vec<_>>();
    let rendered = evidence
        .terrain_batches
        .iter()
        .flat_map(|(_, batch, _, _, _)| batch.runs())
        .collect::<Vec<_>>();
    reconcile_logical_terrain_runs(logical, rendered).map_err(|error| {
        format!("authority terrain/picking projection is inconsistent: {error}")
    })?;

    let substances = evidence
        .logical_terrain
        .iter()
        .map(|(entity, _, _, _, substance, _, _)| {
            substance
                .copied()
                .map(|substance| (entity, substance))
                .ok_or_else(|| format!("logical HexTile {entity:?} has no SubstanceId"))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    for (_, batch, _, _, _) in &evidence.terrain_batches {
        for run in batch.runs() {
            let substance = substances.get(&run.entity()).ok_or_else(|| {
                format!(
                    "TerrainRenderBatch names unknown logical entity {:?}",
                    run.entity()
                )
            })?;
            if *substance != batch.substance() {
                return Err(format!(
                    "TerrainRenderBatch substance {:?} disagrees with logical entity {:?} substance {:?}",
                    batch.substance(),
                    run.entity(),
                    substance
                ));
            }
        }
    }
    Ok(())
}

fn encode_standing(
    encoder: &mut CanonicalAuthorityEncoder,
    standing: Standing,
    label: &str,
) -> Result<(), String> {
    encoder.tile_pos(standing.pos);
    encoder.span(standing.span, label)
}

const fn illumination_level_tag(level: IlluminationLevel) -> u8 {
    match level {
        IlluminationLevel::Dark => 0,
        IlluminationLevel::Dim => 1,
        IlluminationLevel::Bright => 2,
    }
}

fn encode_light_domain(encoder: &mut CanonicalAuthorityEncoder, domain: LightDomain) {
    match domain {
        LightDomain::Exterior => encoder.u8(0),
        LightDomain::Interior(region) => {
            encoder.u8(1);
            encoder.u32(region.0);
        }
    }
}

const fn knowledge_state_tag(state: KnowledgeState) -> u8 {
    match state {
        KnowledgeState::Unknown => 0,
        KnowledgeState::Remembered => 1,
        KnowledgeState::Observed => 2,
    }
}

const fn faction_tag(faction: Faction) -> u8 {
    match faction {
        Faction::Player => 0,
        Faction::Hostile => 1,
    }
}

type ReviewRuntimeCameraQuery = (
    Entity,
    &'static Camera,
    &'static Transform,
    &'static GlobalTransform,
    &'static PanOrbitCamera,
    &'static RenderTarget,
    Option<&'static Projection>,
    &'static Camera3d,
    &'static Msaa,
    &'static ScreenSpaceTransmission,
    Option<&'static OrderIndependentTransparencySettings>,
    Option<&'static VolumetricFog>,
);

type ReviewEntityAllowedQuery = (
    Option<&'static Pickable>,
    Has<Transform>,
    Has<GlobalTransform>,
    Has<Visibility>,
    Has<InheritedVisibility>,
    Has<ViewVisibility>,
    Option<&'static Mesh3d>,
    Option<&'static MeshMaterial3d<StandardMaterial>>,
    Has<NotShadowCaster>,
    Option<&'static FogVolume>,
    Option<&'static Name>,
);

type ReviewEntityForbiddenTerrainQuery = (
    Has<HexTile>,
    Has<TilePos>,
    Has<RunBottom>,
    Has<HexSpan>,
    Has<SubstanceId>,
    Has<Headroom>,
    Has<TerrainRenderBatch>,
    Has<TerrainChunkRoot>,
);

type ReviewEntityForbiddenUnitQuery = (
    Has<Faction>,
    Has<Body>,
    Has<StandsOn>,
    Has<MovingTo>,
    Has<Downed>,
    Has<Busy>,
    Has<ControlOwner>,
    Has<Player>,
    Has<Enemy>,
    Has<Archetype>,
    Has<Selected>,
);

type ReviewEntityForbiddenPresentationQuery = (
    Has<CameraFocusTarget>,
    Has<InspectionCameraSubject>,
    Has<GameplayLight>,
    Has<AuthoredObjectVoxelRuns>,
    Has<CutawayOccluder>,
    Has<TreeOccluder>,
    Has<CanopyOccluder>,
    Has<PresentationOcclusion>,
    Has<TargetReticleRequest>,
    Has<RangeOverlay>,
    Has<PathOverlay>,
    Has<OutOfRangeOverlay>,
);

type ReviewProjectionEntityQuery = (
    Entity,
    ReviewEntityAllowedQuery,
    ReviewEntityForbiddenTerrainQuery,
    ReviewEntityForbiddenUnitQuery,
    ReviewEntityForbiddenPresentationQuery,
);

#[derive(SystemParam)]
struct ReviewRuntimeEvidence<'w, 's> {
    profile: Option<Res<'w, ReviewWorldDetailProfileV1>>,
    report: Option<Res<'w, ReviewWorldDetailReportV1>>,
    runtime_assets: Option<Res<'w, ReviewWorldDetailRuntimeAssetEvidenceV1>>,
    sky_assets: Option<Res<'w, SkyRuntimeAssetEvidenceV1>>,
    liquid_visual_time: Option<Res<'w, LiquidVisualTime>>,
    real_time: Option<Res<'w, Time<Real>>>,
    images: Option<Res<'w, Assets<Image>>>,
    meshes: Option<Res<'w, Assets<Mesh>>>,
    standard_materials: Option<Res<'w, Assets<StandardMaterial>>>,
    render_adapter: Option<Res<'w, RenderAdapter>>,
    render_device: Option<Res<'w, RenderDevice>>,
    authority: ReviewAuthorityEvidence<'w, 's>,
    live_meshes: Query<'w, 's, &'static Mesh3d>,
    live_standard_materials: Query<'w, 's, &'static MeshMaterial3d<StandardMaterial>>,
    review_entities: Query<'w, 's, ReviewProjectionEntityQuery, With<ReviewWorldDetailEntity>>,
    cameras: Query<'w, 's, ReviewRuntimeCameraQuery, With<PanOrbitCamera>>,
}

/// Minimal live state reconstructed when GPU readback completes. Keeping this
/// separate from [`ReviewRuntimeEvidence`] avoids borrowing `LiquidVisualTime`
/// immutably in the observer that advances the next capture phase.
#[derive(SystemParam)]
struct ReviewCaptureReadbackEvidence<'w, 's> {
    profile: Res<'w, ReviewWorldDetailProfileV1>,
    report: Option<Res<'w, ReviewWorldDetailReportV1>>,
    render_adapter: Option<Res<'w, RenderAdapter>>,
    render_device: Option<Res<'w, RenderDevice>>,
    cameras: Query<'w, 's, ReviewRuntimeCameraQuery, With<PanOrbitCamera>>,
}

/// Immutable request-time state that must still match when Bevy delivers the
/// asynchronous screenshot readback. The sidecar is serialized before requesting
/// the screenshot, so this binding prevents it from describing a later camera or
/// projection state.
#[derive(Clone)]
struct ReviewCaptureReadbackBindingV1 {
    profile_hash_sha256: String,
    projection_hashes: hex_map::review_world_detail::ReviewWorldDetailProjectionHashesV1,
    counts: hex_map::review_world_detail::ReviewWorldDetailCountsV1,
    camera_features: ReviewCameraFeaturesV1,
    camera_entity: Entity,
    transform: Transform,
    global_transform_bits: [u32; 16],
    orbit_focus_bits: [u32; 3],
    orbit_radius_bits: u32,
    render_target: RenderTarget,
    projection: Option<Projection>,
    clip_from_view_bits: [u32; 16],
    msaa: Msaa,
    depth_texture_usages: u32,
    transmission_steps: usize,
    transmission_quality: ScreenSpaceTransmissionQuality,
    oit: Option<OrderIndependentTransparencySettings>,
    volumetric_fog: Option<VolumetricFog>,
}

impl ReviewCaptureReadbackBindingV1 {
    fn verify_same(&self, current: &Self) -> Result<(), String> {
        if self.profile_hash_sha256 != current.profile_hash_sha256 {
            return Err(
                "resolved profile changed between screenshot request and readback".to_owned(),
            );
        }
        if self.projection_hashes != current.projection_hashes {
            return Err(
                "projection hashes changed between screenshot request and readback".to_owned(),
            );
        }
        if self.counts != current.counts {
            return Err(
                "projection counts changed between screenshot request and readback".to_owned(),
            );
        }
        if self.camera_features != current.camera_features {
            return Err(
                "camera features changed between screenshot request and readback".to_owned(),
            );
        }
        if self.camera_entity != current.camera_entity {
            return Err(
                "active camera identity changed between screenshot request and readback".to_owned(),
            );
        }
        if self.transform != current.transform
            || self.global_transform_bits != current.global_transform_bits
            || self.orbit_focus_bits != current.orbit_focus_bits
            || self.orbit_radius_bits != current.orbit_radius_bits
        {
            return Err("camera pose changed between screenshot request and readback".to_owned());
        }
        if !render_targets_equal(&self.render_target, &current.render_target) {
            return Err(
                "camera render target changed between screenshot request and readback".to_owned(),
            );
        }
        if !projections_equal(self.projection.as_ref(), current.projection.as_ref())
            || self.clip_from_view_bits != current.clip_from_view_bits
        {
            return Err(
                "camera projection changed between screenshot request and readback".to_owned(),
            );
        }
        if self.msaa != current.msaa
            || self.depth_texture_usages != current.depth_texture_usages
            || self.transmission_steps != current.transmission_steps
            || self.transmission_quality != current.transmission_quality
            || !oit_settings_equal(self.oit, current.oit)
            || !volumetric_fog_equal(self.volumetric_fog, current.volumetric_fog)
        {
            return Err(
                "camera renderer state changed between screenshot request and readback".to_owned(),
            );
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
struct ReviewProjectionEntityAuditV1 {
    entities: BTreeSet<Entity>,
    meshes: BTreeSet<AssetId<Mesh>>,
    standard_materials: BTreeSet<AssetId<StandardMaterial>>,
    images: BTreeSet<AssetId<Image>>,
    public_payload_bytes: u64,
}

fn audit_review_projection_entities(
    evidence: &ReviewRuntimeEvidence<'_, '_>,
) -> Result<ReviewProjectionEntityAuditV1, String> {
    let source_report = evidence
        .report
        .as_deref()
        .ok_or_else(|| {
            format!(
                "world-detail projection report is not published; static collider contract: {REVIEW_COLLIDER_STATIC_INVARIANT}"
            )
        })?;
    let expected_entities = usize::try_from(source_report.cleanup.entities_remaining)
        .map_err(|_error| "world-detail entity count exceeds usize".to_owned())?;
    let expected_meshes = usize::try_from(source_report.cleanup.meshes_remaining)
        .map_err(|_error| "world-detail mesh count exceeds usize".to_owned())?;
    let expected_images = usize::try_from(
        evidence
            .runtime_assets
            .as_deref()
            .ok_or_else(|| {
                "renderer-owned material/image allocation evidence is unavailable".to_owned()
            })?
            .fog_density_image_count,
    )
    .map_err(|_error| "world-detail fog density image count exceeds usize".to_owned())?;
    let mut audit = ReviewProjectionEntityAuditV1::default();
    for (
        id,
        (
            pickable,
            has_transform,
            has_global_transform,
            has_visibility,
            has_inherited_visibility,
            has_view_visibility,
            mesh,
            material,
            has_not_shadow_caster,
            fog,
            name,
        ),
        (
            has_hex_tile,
            has_tile_pos,
            has_run_bottom,
            has_hex_span,
            has_substance,
            has_headroom,
            has_terrain_batch,
            has_terrain_chunk_root,
        ),
        (
            has_faction,
            has_body,
            has_stands_on,
            has_moving_to,
            has_downed,
            has_busy,
            has_control_owner,
            has_player,
            has_enemy,
            has_archetype,
            has_selected,
        ),
        (
            has_camera_focus,
            has_inspection_subject,
            has_gameplay_light,
            has_authored_runs,
            has_cutaway,
            has_tree_occluder,
            has_canopy_occluder,
            has_presentation_occlusion,
            has_target_reticle,
            has_range_overlay,
            has_path_overlay,
            has_out_of_range_overlay,
        ),
    ) in &evidence.review_entities
    {
        let pickable = pickable.ok_or_else(|| {
            format!("review projection entity {id:?} has no explicit Pickable::IGNORE")
        })?;
        if *pickable != Pickable::IGNORE {
            return Err(format!(
                "review projection entity {id:?} is gameplay-pickable instead of Pickable::IGNORE"
            ));
        }

        macro_rules! reject_component {
            ($present:expr, $name:literal) => {
                if $present {
                    return Err(format!(
                        "review projection entity {id:?} carries forbidden authoritative component {}",
                        $name
                    ));
                }
            };
        }
        reject_component!(has_hex_tile, "HexTile");
        reject_component!(has_tile_pos, "TilePos");
        reject_component!(has_run_bottom, "RunBottom");
        reject_component!(has_hex_span, "HexSpan");
        reject_component!(has_substance, "SubstanceId");
        reject_component!(has_headroom, "Headroom");
        reject_component!(has_terrain_batch, "TerrainRenderBatch");
        reject_component!(has_terrain_chunk_root, "TerrainChunkRoot");
        reject_component!(has_faction, "Faction");
        reject_component!(has_body, "Body");
        reject_component!(has_stands_on, "StandsOn");
        reject_component!(has_moving_to, "MovingTo");
        reject_component!(has_downed, "Downed");
        reject_component!(has_busy, "Busy");
        reject_component!(has_control_owner, "ControlOwner");
        reject_component!(has_player, "Player");
        reject_component!(has_enemy, "Enemy");
        reject_component!(has_archetype, "Archetype");
        reject_component!(has_selected, "Selected");
        reject_component!(has_camera_focus, "CameraFocusTarget");
        reject_component!(has_inspection_subject, "InspectionCameraSubject");
        reject_component!(has_gameplay_light, "GameplayLight");
        reject_component!(has_authored_runs, "AuthoredObjectVoxelRuns");
        reject_component!(has_cutaway, "CutawayOccluder");
        reject_component!(has_tree_occluder, "TreeOccluder");
        reject_component!(has_canopy_occluder, "CanopyOccluder");
        reject_component!(has_presentation_occlusion, "PresentationOcclusion");
        reject_component!(has_target_reticle, "TargetReticleRequest");
        reject_component!(has_range_overlay, "RangeOverlay");
        reject_component!(has_path_overlay, "PathOverlay");
        reject_component!(has_out_of_range_overlay, "OutOfRangeOverlay");

        audit.entities.insert(id);
        if let Some(mesh) = mesh {
            audit.meshes.insert(mesh.0.id());
        }
        if let Some(material) = material {
            audit.standard_materials.insert(material.0.id());
        }
        if let Some(texture) = fog.and_then(|fog| fog.density_texture.as_ref()) {
            audit.images.insert(texture.id());
        }
        audit.public_payload_bytes = audit
            .public_payload_bytes
            .checked_add(review_entity_public_payload_bytes(
                true,
                has_transform,
                has_global_transform,
                has_visibility,
                has_inherited_visibility,
                has_view_visibility,
                mesh.is_some(),
                material.is_some(),
                has_not_shadow_caster,
                fog.is_some(),
                name,
            )?)
            .ok_or_else(|| "review ECS payload byte count overflowed u64".to_owned())?;
    }
    if audit.entities.len() != expected_entities {
        return Err(format!(
            "ReviewWorldDetailEntity count {} disagrees with renderer-owned count {expected_entities}",
            audit.entities.len()
        ));
    }
    if audit.meshes.len() != expected_meshes {
        return Err(format!(
            "review marker mesh count {} disagrees with renderer-owned count {expected_meshes}",
            audit.meshes.len()
        ));
    }
    if audit.images.len() != expected_images {
        return Err(format!(
            "review fog image count {} disagrees with renderer-owned count {expected_images}",
            audit.images.len()
        ));
    }
    Ok(audit)
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ReviewPerformanceObservationV1 {
    frame_time_ms: f32,
    resident_presentation_bytes: u64,
}

fn performance_sampling_start_frame(settle_frames: u32) -> u32 {
    let window_frames = u32::try_from(PERFORMANCE_WINDOW_FRAMES).unwrap_or(u32::MAX);
    settle_frames
        .saturating_sub(window_frames)
        .saturating_add(1)
}

fn measure_review_performance(
    state: &ReviewCaptureState,
    evidence: &ReviewRuntimeEvidence<'_, '_>,
    review_entity_public_payload_bytes: u64,
) -> Result<ReviewPerformanceObservationV1, String> {
    let sampling_start_frame = performance_sampling_start_frame(state.capture.settle_frames);
    if state.settled_frames < sampling_start_frame {
        return Err(format!(
            "performance sample requested at settle frame {}, before sampling frame {}",
            state.settled_frames, sampling_start_frame
        ));
    }
    let real_time = evidence
        .real_time
        .as_deref()
        .ok_or_else(|| "Time<Real> is unavailable for performance sampling".to_owned())?;
    let frame_time_ms_f64 = real_time.delta_secs_f64() * 1_000.0;
    if !frame_time_ms_f64.is_finite() || !(frame_time_ms_f64 > 0.0 && frame_time_ms_f64 <= 10_000.0)
    {
        return Err(format!(
            "immediately preceding rendered-frame duration is outside (0, 10000] ms: {frame_time_ms_f64}"
        ));
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the validated 0..=10000 ms sample is deliberately stored by the report's f32 schema"
    )]
    let frame_time_ms = frame_time_ms_f64 as f32;

    let mesh_ids = evidence
        .live_meshes
        .iter()
        .map(|mesh| mesh.0.id())
        .collect::<BTreeSet<_>>();
    let mut resident_presentation_bytes = 0_u64;
    for id in mesh_ids {
        let mesh = evidence
            .meshes
            .as_deref()
            .ok_or_else(|| "Assets<Mesh> is unavailable for performance sampling".to_owned())?
            .get(id)
            .ok_or_else(|| format!("live Mesh3d references missing mesh asset {id:?}"))?;
        resident_presentation_bytes = resident_presentation_bytes
            .checked_add(mesh_buffer_bytes(mesh)?)
            .ok_or_else(|| "resident mesh byte count overflowed u64".to_owned())?;
    }

    let standard_material_ids = evidence
        .live_standard_materials
        .iter()
        .map(|material| material.0.id())
        .collect::<BTreeSet<_>>();
    for id in standard_material_ids {
        let material = evidence
            .standard_materials
            .as_deref()
            .ok_or_else(|| {
                "Assets<StandardMaterial> is unavailable for performance sampling".to_owned()
            })?
            .get(id)
            .ok_or_else(|| {
                format!("live MeshMaterial3d references missing StandardMaterial asset {id:?}")
            })?;
        resident_presentation_bytes = resident_presentation_bytes
            .checked_add(
                u64::try_from(std::mem::size_of_val(material))
                    .map_err(|_error| "StandardMaterial allocation exceeds u64".to_owned())?,
            )
            .ok_or_else(|| "resident StandardMaterial byte count overflowed u64".to_owned())?;
    }

    // `Assets<Image>` is the public main-world residency boundary. Count every live
    // texture mip payload conservatively, except the one capture target whose cost
    // would be identical on both sides of a matched comparison. This also catches
    // late asset growth during the stability window instead of silently selecting
    // only textures already referenced by StandardMaterial.
    let images = evidence
        .images
        .as_deref()
        .ok_or_else(|| "Assets<Image> is unavailable for performance sampling".to_owned())?;
    for (id, image) in images.iter() {
        if state.capture_target_ids.contains(&id) {
            continue;
        }
        resident_presentation_bytes = resident_presentation_bytes
            .checked_add(image_texture_bytes(image)?)
            .ok_or_else(|| "resident Image texture byte count overflowed u64".to_owned())?;
    }

    resident_presentation_bytes = resident_presentation_bytes
        .checked_add(review_entity_public_payload_bytes)
        .ok_or_else(|| "review ECS component byte count overflowed u64".to_owned())?;

    let runtime_assets = evidence.runtime_assets.as_deref().ok_or_else(|| {
        "renderer-owned liquid material allocation evidence is unavailable".to_owned()
    })?;
    validate_counted_allocation(
        "ordinary liquid materials",
        runtime_assets.liquid_material_count,
        runtime_assets.liquid_material_bytes,
    )?;
    validate_counted_allocation(
        "review-water materials",
        runtime_assets.review_water_material_count,
        runtime_assets.review_water_material_bytes,
    )?;
    validate_counted_allocation(
        "fog density images",
        runtime_assets.fog_density_image_count,
        runtime_assets.fog_density_image_bytes,
    )?;
    let sky_assets = evidence
        .sky_assets
        .as_deref()
        .ok_or_else(|| "sky material allocation evidence is unavailable".to_owned())?;
    validate_counted_allocation(
        "sky materials",
        sky_assets.sky_material_count,
        sky_assets.sky_material_bytes,
    )?;
    if sky_assets.sky_material_count == 0 {
        return Err("matched live scene contains no evidenced sky material".to_owned());
    }
    for bytes in [
        runtime_assets.liquid_material_bytes,
        runtime_assets.review_water_material_bytes,
        sky_assets.sky_material_bytes,
    ] {
        resident_presentation_bytes = resident_presentation_bytes
            .checked_add(bytes)
            .ok_or_else(|| "resident presentation byte count overflowed u64".to_owned())?;
    }
    if resident_presentation_bytes == 0 {
        return Err("matched live scene has zero resident presentation bytes".to_owned());
    }
    Ok(ReviewPerformanceObservationV1 {
        frame_time_ms,
        resident_presentation_bytes,
    })
}

fn sample_review_performance(
    state: &mut ReviewCaptureState,
    evidence: &ReviewRuntimeEvidence<'_, '_>,
    review_entity_public_payload_bytes: u64,
) -> Result<Option<ReviewPerformanceSampleV1>, String> {
    let observation =
        measure_review_performance(state, evidence, review_entity_public_payload_bytes)?;
    if state.performance_resident_bytes != Some(observation.resident_presentation_bytes) {
        state.performance_resident_bytes = Some(observation.resident_presentation_bytes);
        state.performance_frame_window_ms.clear();
    }
    state
        .performance_frame_window_ms
        .push_back(observation.frame_time_ms);
    while state.performance_frame_window_ms.len() > PERFORMANCE_WINDOW_FRAMES {
        state.performance_frame_window_ms.pop_front();
    }
    if state.performance_frame_window_ms.len() < PERFORMANCE_WINDOW_FRAMES {
        return Ok(None);
    }

    let frame_times = state
        .performance_frame_window_ms
        .iter()
        .copied()
        .collect::<Vec<_>>();
    let midpoint = frame_times.len() / 2;
    let (first_half, second_half) = frame_times.split_at(midpoint);
    let first_p95 = nearest_rank_p95(first_half)?;
    let second_p95 = nearest_rank_p95(second_half)?;
    let drift = (first_p95 - second_p95).abs() / first_p95.max(second_p95);
    if !drift.is_finite() || drift > PERFORMANCE_P95_DRIFT_LIMIT {
        return Ok(None);
    }
    let frame_time_ms = nearest_rank_p95(&frame_times)?;
    let resident_presentation_bytes = state
        .performance_resident_bytes
        .ok_or_else(|| {
            format!(
                "stable performance window lost resident-byte evidence for scope: {REVIEW_RESIDENT_MEMORY_SCOPE}"
            )
        })?;
    Ok(Some(ReviewPerformanceSampleV1 {
        frame_time_ms,
        resident_presentation_bytes,
        warmup_complete: true,
    }))
}

fn nearest_rank_p95(samples: &[f32]) -> Result<f32, String> {
    if samples.is_empty()
        || samples
            .iter()
            .any(|sample| !sample.is_finite() || *sample <= 0.0)
    {
        return Err("p95 performance window is empty or contains an invalid frame".to_owned());
    }
    let mut ordered = samples.to_vec();
    ordered.sort_by(f32::total_cmp);
    let rank = ordered
        .len()
        .checked_mul(95)
        .and_then(|value| value.checked_add(99))
        .ok_or_else(|| "p95 performance rank overflowed usize".to_owned())?
        / 100;
    ordered
        .get(rank.saturating_sub(1))
        .copied()
        .ok_or_else(|| "p95 performance rank fell outside the sample window".to_owned())
}

fn image_texture_bytes(image: &Image) -> Result<u64, String> {
    let descriptor = &image.texture_descriptor;
    if descriptor.size.width == 0
        || descriptor.size.height == 0
        || descriptor.size.depth_or_array_layers == 0
        || descriptor.mip_level_count == 0
        || descriptor.sample_count == 0
    {
        return Err("live Image has a zero-sized texture descriptor".to_owned());
    }
    let block_bytes = descriptor
        .format
        .block_copy_size(None)
        .ok_or_else(|| format!("cannot size texture format {:?} exactly", descriptor.format))?;
    let (block_width, block_height) = descriptor.format.block_dimensions();
    let mut bytes = 0_u64;
    for mip in 0..descriptor.mip_level_count {
        let extent = descriptor.size.mip_level_size(mip, descriptor.dimension);
        let blocks_wide = u64::from(extent.width.div_ceil(block_width));
        let blocks_high = u64::from(extent.height.div_ceil(block_height));
        let mip_bytes = blocks_wide
            .checked_mul(blocks_high)
            .and_then(|value| value.checked_mul(u64::from(extent.depth_or_array_layers)))
            .and_then(|value| value.checked_mul(u64::from(block_bytes)))
            .and_then(|value| value.checked_mul(u64::from(descriptor.sample_count)))
            .ok_or_else(|| "Image texture mip byte count overflowed u64".to_owned())?;
        bytes = bytes
            .checked_add(mip_bytes)
            .ok_or_else(|| "Image texture byte count overflowed u64".to_owned())?;
    }
    Ok(bytes)
}

fn review_entity_public_payload_bytes(
    has_pickable: bool,
    has_transform: bool,
    has_global_transform: bool,
    has_visibility: bool,
    has_inherited_visibility: bool,
    has_view_visibility: bool,
    has_mesh: bool,
    has_standard_material: bool,
    has_not_shadow_caster: bool,
    has_fog_volume: bool,
    name: Option<&Name>,
) -> Result<u64, String> {
    let mut bytes = u64::try_from(std::mem::size_of::<Entity>())
        .map_err(|_error| "Entity inline size exceeds u64".to_owned())?;
    macro_rules! count_component {
        ($present:expr, $component:ty) => {
            if $present {
                bytes =
                    bytes
                        .checked_add(u64::try_from(std::mem::size_of::<$component>()).map_err(
                            |_error| "review component inline size exceeds u64".to_owned(),
                        )?)
                        .ok_or_else(|| {
                            "review component inline byte count overflowed u64".to_owned()
                        })?;
            }
        };
    }
    count_component!(true, ReviewWorldDetailEntity);
    count_component!(has_pickable, Pickable);
    count_component!(has_transform, Transform);
    count_component!(has_global_transform, GlobalTransform);
    count_component!(has_visibility, Visibility);
    count_component!(has_inherited_visibility, InheritedVisibility);
    count_component!(has_view_visibility, ViewVisibility);
    count_component!(has_mesh, Mesh3d);
    count_component!(has_standard_material, MeshMaterial3d<StandardMaterial>);
    count_component!(has_not_shadow_caster, NotShadowCaster);
    count_component!(has_fog_volume, FogVolume);
    count_component!(name.is_some(), Name);
    if has_mesh && !has_standard_material && !has_fog_volume {
        // The sole renderer-private alternative is MeshMaterial3d<ReviewWaterMaterial>;
        // generic Handle storage has the same inline representation.
        bytes = bytes
            .checked_add(
                u64::try_from(std::mem::size_of::<Handle<StandardMaterial>>())
                    .map_err(|_error| "review-water material handle size exceeds u64".to_owned())?,
            )
            .ok_or_else(|| "review-water component byte count overflowed u64".to_owned())?;
    }
    if let Some(name) = name {
        bytes = bytes
            .checked_add(
                u64::try_from(name.as_str().len())
                    .map_err(|_error| "review entity name length exceeds u64".to_owned())?,
            )
            .ok_or_else(|| "review entity name byte count overflowed u64".to_owned())?;
    }
    Ok(bytes)
}

fn validate_counted_allocation(label: &str, count: u64, bytes: u64) -> Result<(), String> {
    if (count == 0) != (bytes == 0) {
        return Err(format!(
            "{label} evidence has inconsistent count/byte presence ({count} assets, {bytes} bytes)"
        ));
    }
    Ok(())
}

fn mesh_buffer_bytes(mesh: &Mesh) -> Result<u64, String> {
    let mut attributes = mesh
        .try_attributes()
        .map_err(|error| format!("mesh vertex buffers are unavailable in the main world: {error}"))?
        .peekable();
    if attributes.peek().is_none() {
        return Err("live Mesh3d asset contains no vertex buffers".to_owned());
    }
    let mut vertex_count = None;
    let mut bytes = 0_u64;
    for (attribute, values) in attributes {
        if vertex_count
            .replace(values.len())
            .is_some_and(|count| count != values.len())
        {
            return Err(format!(
                "mesh attribute {} has a mismatched vertex count",
                attribute.name
            ));
        }
        let value_count = u64::try_from(values.len())
            .map_err(|_error| "mesh vertex count exceeds u64".to_owned())?;
        bytes = bytes
            .checked_add(
                value_count
                    .checked_mul(attribute.format.size())
                    .ok_or_else(|| "mesh vertex buffer byte count overflowed u64".to_owned())?,
            )
            .ok_or_else(|| "mesh vertex buffer byte count overflowed u64".to_owned())?;
    }
    if let Some(indices) = mesh
        .try_indices_option()
        .map_err(|error| format!("mesh index buffer is unavailable in the main world: {error}"))?
    {
        let index_count = u64::try_from(indices.len())
            .map_err(|_error| "mesh index count exceeds u64".to_owned())?;
        let index_width = match indices {
            Indices::U16(_) => 2,
            Indices::U32(_) => 4,
        };
        bytes = bytes
            .checked_add(
                index_count
                    .checked_mul(index_width)
                    .ok_or_else(|| "mesh index buffer byte count overflowed u64".to_owned())?,
            )
            .ok_or_else(|| "mesh index buffer byte count overflowed u64".to_owned())?;
    }
    Ok(bytes)
}

/// Captures the one immutable review baseline at the terminal ordinary setup
/// boundary. `GameplaySetup` is globally chained, so Terrain, Actors, Restore,
/// Perception, and View have all flushed before this system runs. Review world-detail
/// projection is an `Update` system and therefore cannot have run yet.
fn capture_review_authority_baseline(
    evidence: ReviewAuthorityEvidence,
    mut state: ResMut<ReviewCaptureState>,
    mut exit: MessageWriter<AppExit>,
) {
    if state.failed || state.authority_baseline.is_some() {
        return;
    }
    match ReviewAuthoritySnapshotV1::capture(&evidence) {
        Ok(snapshot) => {
            info!(
                "captured immutable review authority baseline {} under lighting condition {}",
                snapshot.fingerprint(),
                snapshot.lighting_condition_key()
            );
            state.authority_baseline = Some(snapshot);
        }
        Err(error) => {
            error!("cannot capture immutable review authority baseline: {error}");
            state.failed = true;
            exit.write(AppExit::error());
        }
    }
}

fn verify_review_authority(
    state: &ReviewCaptureState,
    evidence: &ReviewAuthorityEvidence<'_, '_>,
) -> Result<ReviewAuthoritySnapshotV1, String> {
    let baseline = state.authority_baseline.as_ref().ok_or_else(|| {
        "immutable review authority baseline was not captured before projection".to_owned()
    })?;
    let current = ReviewAuthoritySnapshotV1::capture(evidence)?;
    baseline.verify_unchanged(&current)?;
    Ok(current)
}

#[derive(SystemParam)]
struct ReviewAuthorityTeardownEvidence<'w, 's> {
    world_snapshot: Option<Res<'w, CurrentWorldSnapshotV1>>,
    replication: Option<Res<'w, WorldReplicationStateV1>>,
    time_of_day: Option<Res<'w, TimeOfDay>>,
    exterior_illumination: Option<Res<'w, ExteriorIllumination>>,
    illumination: Option<Res<'w, ResolvedIllumination>>,
    knowledge: Option<Res<'w, FactionMapKnowledge>>,
    unit_registry: Option<Res<'w, UnitRegistry>>,
    campaign_store: Option<Res<'w, CampaignStore>>,
    campaign_save_status: Option<Res<'w, CampaignSaveStatusProjection>>,
    storage_paths: Option<Res<'w, StoragePaths>>,
    units: Query<'w, 's, Entity, With<Faction>>,
    logical_terrain: Query<'w, 's, Entity, With<HexTile>>,
    terrain_batches: Query<'w, 's, Entity, With<TerrainRenderBatch>>,
}

#[derive(SystemParam)]
struct ReviewLifecycleTeardownEvidence<'w, 's> {
    receipt: Option<Res<'w, ReviewWorldDetailTeardownReceiptV1>>,
    runtime_assets: Option<Res<'w, ReviewWorldDetailRuntimeAssetEvidenceV1>>,
    teardown_request: Option<Res<'w, ReviewWorldDetailTeardownRequestV1>>,
    authority: ReviewAuthorityEvidence<'w, 's>,
    meshes: Option<Res<'w, Assets<Mesh>>>,
    standard_materials: Option<Res<'w, Assets<StandardMaterial>>>,
    images: Option<Res<'w, Assets<Image>>>,
    all_entities: Query<'w, 's, Entity>,
    cameras: Query<
        'w,
        's,
        (
            Entity,
            &'static Transform,
            &'static PanOrbitCamera,
            &'static RenderTarget,
            Option<&'static Projection>,
            &'static Camera3d,
            &'static Msaa,
            &'static ScreenSpaceTransmission,
            Option<&'static OrderIndependentTransparencySettings>,
            Option<&'static VolumetricFog>,
            Has<ReviewCameraFeatureRestore>,
        ),
    >,
    camera_mode: Option<Res<'w, CameraMode>>,
    camera_feature_restores: Query<'w, 's, Entity, With<ReviewCameraFeatureRestore>>,
    added_volumetric_lights: Query<'w, 's, Entity, With<ReviewAddedVolumetricLight>>,
}

#[derive(Debug)]
struct ReviewLifecycleTeardownSnapshotV1 {
    authority_after: ReviewAuthoritySnapshotV1,
    entities_remaining: u64,
    materials_remaining: u64,
    meshes_remaining: u64,
    fog_density_images_remaining: u64,
    target_images_remaining: u64,
    terrain_material_overrides_remaining: u64,
    liquid_visibility_overrides_remaining: u64,
    vegetation_scale_overrides_remaining: u64,
    camera_state_restored: bool,
    oit_state_restored: bool,
    transmission_state_restored: bool,
    depth_state_restored: bool,
    volumetric_state_restored: bool,
}

#[derive(Debug, Clone, Copy)]
struct ReviewPresentationTeardownSnapshotV1 {
    entities_remaining: u64,
    materials_remaining: u64,
    meshes_remaining: u64,
    fog_density_images_remaining: u64,
    target_images_remaining: u64,
    terrain_material_overrides_remaining: u64,
    liquid_visibility_overrides_remaining: u64,
    vegetation_scale_overrides_remaining: u64,
    camera_state_restored: bool,
    oit_state_restored: bool,
    transmission_state_restored: bool,
    depth_state_restored: bool,
    volumetric_state_restored: bool,
}

impl ReviewPresentationTeardownSnapshotV1 {
    fn cleanup(self, completed_cycles: u16) -> ReviewCleanupStateV1 {
        ReviewCleanupStateV1 {
            completed_cycles,
            entities_remaining: self.entities_remaining,
            materials_remaining: self.materials_remaining,
            meshes_remaining: self.meshes_remaining,
            target_images_remaining: self.target_images_remaining,
            camera_state_restored: self.camera_state_restored,
            oit_state_restored: self.oit_state_restored,
            transmission_state_restored: self.transmission_state_restored,
            depth_state_restored: self.depth_state_restored,
            volumetric_state_restored: self.volumetric_state_restored,
        }
    }
}

fn validate_review_lifecycle_teardown(
    state: &ReviewCaptureState,
    evidence: &ReviewLifecycleTeardownEvidence<'_, '_>,
) -> Result<ReviewLifecycleTeardownSnapshotV1, String> {
    let baseline = state
        .authority_baseline
        .as_ref()
        .ok_or_else(|| "lifecycle teardown lost its immutable authority baseline".to_owned())?;
    let authority_after = ReviewAuthoritySnapshotV1::capture(&evidence.authority)?;
    baseline.verify_unchanged(&authority_after)?;
    let presentation = validate_review_presentation_teardown(state, evidence)?;
    Ok(ReviewLifecycleTeardownSnapshotV1 {
        authority_after,
        entities_remaining: presentation.entities_remaining,
        materials_remaining: presentation.materials_remaining,
        meshes_remaining: presentation.meshes_remaining,
        fog_density_images_remaining: presentation.fog_density_images_remaining,
        target_images_remaining: presentation.target_images_remaining,
        terrain_material_overrides_remaining: presentation.terrain_material_overrides_remaining,
        liquid_visibility_overrides_remaining: presentation.liquid_visibility_overrides_remaining,
        vegetation_scale_overrides_remaining: presentation.vegetation_scale_overrides_remaining,
        camera_state_restored: presentation.camera_state_restored,
        oit_state_restored: presentation.oit_state_restored,
        transmission_state_restored: presentation.transmission_state_restored,
        depth_state_restored: presentation.depth_state_restored,
        volumetric_state_restored: presentation.volumetric_state_restored,
    })
}

fn validate_review_presentation_teardown(
    state: &ReviewCaptureState,
    evidence: &ReviewLifecycleTeardownEvidence<'_, '_>,
) -> Result<ReviewPresentationTeardownSnapshotV1, String> {
    if evidence.runtime_assets.is_some() {
        return Err("renderer-owned live asset evidence survived projection teardown".to_owned());
    }
    if evidence.teardown_request.is_some() {
        return Err("projection teardown request was not consumed".to_owned());
    }
    let receipt = evidence.receipt.as_deref().ok_or_else(|| {
        "renderer did not publish a post-teardown world-detail receipt".to_owned()
    })?;
    if !evidence.camera_feature_restores.is_empty() {
        return Err(format!(
            "{} review camera feature-restore markers survived projection teardown",
            evidence.camera_feature_restores.iter().count()
        ));
    }
    let meshes = evidence
        .meshes
        .as_deref()
        .ok_or_else(|| "Assets<Mesh> disappeared during lifecycle teardown".to_owned())?;
    let standard_materials = evidence.standard_materials.as_deref().ok_or_else(|| {
        "Assets<StandardMaterial> disappeared during lifecycle teardown".to_owned()
    })?;
    let images = evidence
        .images
        .as_deref()
        .ok_or_else(|| "Assets<Image> disappeared during lifecycle teardown".to_owned())?;

    let tracked_entities_remaining = state
        .review_entity_ids
        .iter()
        .filter(|entity| evidence.all_entities.get(**entity).is_ok())
        .count();
    if tracked_entities_remaining != 0 {
        return Err(format!(
            "{tracked_entities_remaining} tracked review entities survived projection teardown"
        ));
    }
    let tracked_meshes_remaining = state
        .review_mesh_ids
        .iter()
        .filter(|id| meshes.get(**id).is_some())
        .count();
    if tracked_meshes_remaining != 0 {
        return Err(format!(
            "{tracked_meshes_remaining} tracked review mesh assets survived projection teardown"
        ));
    }
    let tracked_materials_remaining = state
        .review_standard_material_ids
        .iter()
        .filter(|id| standard_materials.get(**id).is_some())
        .count();
    if tracked_materials_remaining != 0 {
        return Err(format!(
            "{tracked_materials_remaining} tracked review StandardMaterial assets survived projection teardown"
        ));
    }
    let tracked_images_remaining = state
        .review_image_ids
        .iter()
        .filter(|id| images.get(**id).is_some())
        .count();
    if tracked_images_remaining != 0 {
        return Err(format!(
            "{tracked_images_remaining} tracked review fog density images survived projection teardown"
        ));
    }
    let target_images_remaining = u64::try_from(
        state
            .capture_target_ids
            .iter()
            .filter(|id| images.get(**id).is_some())
            .count(),
    )
    .map_err(|_error| "review target image count exceeds u64".to_owned())?;

    let snapshot = state
        .camera_snapshot
        .as_ref()
        .ok_or_else(|| "lifecycle teardown lost its camera snapshot".to_owned())?;
    let camera = evidence
        .cameras
        .get(snapshot.entity)
        .map_err(|_error| "review camera disappeared during lifecycle teardown".to_owned())?;
    let (
        entity,
        transform,
        orbit,
        target,
        projection,
        camera_3d,
        msaa,
        transmission,
        oit,
        volumetric_fog,
        has_feature_restore,
    ) = camera;
    let mode = evidence
        .camera_mode
        .as_deref()
        .ok_or_else(|| "CameraMode disappeared during lifecycle teardown".to_owned())?;
    let camera_state_restored = entity == snapshot.entity
        && *transform == snapshot.transform
        && orbit.focus == snapshot.orbit_focus
        && orbit.radius.to_bits() == snapshot.orbit_radius.to_bits()
        && render_targets_equal(target, &snapshot.target)
        && projections_equal(projection, snapshot.projection.as_ref())
        && *mode == snapshot.mode
        && !has_feature_restore;
    let oit_state_restored =
        oit_settings_equal(oit.copied(), snapshot.oit) && *msaa == snapshot.msaa;
    let transmission_state_restored = transmission.steps == snapshot.transmission_steps
        && transmission.quality == snapshot.transmission_quality;
    let depth_state_restored = camera_3d.depth_texture_usages.0 == snapshot.depth_texture_usages.0;
    let volumetric_state_restored =
        volumetric_fog_equal(volumetric_fog.copied(), snapshot.volumetric_fog)
            && evidence.added_volumetric_lights.is_empty();

    let entities_remaining = receipt.review_entities_remaining;
    let materials_remaining = receipt
        .standard_materials_remaining
        .checked_add(receipt.review_water_materials_remaining)
        .ok_or_else(|| "post-teardown material count overflowed u64".to_owned())?;
    let meshes_remaining = receipt.meshes_remaining;
    if entities_remaining != 0
        || materials_remaining != 0
        || meshes_remaining != 0
        || receipt.fog_density_images_remaining != 0
        || receipt.terrain_material_overrides_remaining != 0
        || receipt.liquid_visibility_overrides_remaining != 0
        || receipt.vegetation_scale_overrides_remaining != 0
        || target_images_remaining != 0
        || !camera_state_restored
        || !oit_state_restored
        || !transmission_state_restored
        || !depth_state_restored
        || !volumetric_state_restored
    {
        return Err(format!(
            "lifecycle teardown was incomplete (entities={entities_remaining}, materials={materials_remaining}, meshes={meshes_remaining}, fog_images={}, terrain_overrides={}, liquid_overrides={}, vegetation_overrides={}, targets={target_images_remaining}, camera={camera_state_restored}, oit={oit_state_restored}, transmission={transmission_state_restored}, depth={depth_state_restored}, volumetric={volumetric_state_restored})",
            receipt.fog_density_images_remaining,
            receipt.terrain_material_overrides_remaining,
            receipt.liquid_visibility_overrides_remaining,
            receipt.vegetation_scale_overrides_remaining,
        ));
    }

    Ok(ReviewPresentationTeardownSnapshotV1 {
        entities_remaining,
        materials_remaining,
        meshes_remaining,
        fog_density_images_remaining: receipt.fog_density_images_remaining,
        target_images_remaining,
        terrain_material_overrides_remaining: receipt.terrain_material_overrides_remaining,
        liquid_visibility_overrides_remaining: receipt.liquid_visibility_overrides_remaining,
        vegetation_scale_overrides_remaining: receipt.vegetation_scale_overrides_remaining,
        camera_state_restored,
        oit_state_restored,
        transmission_state_restored,
        depth_state_restored,
        volumetric_state_restored,
    })
}

fn render_targets_equal(left: &RenderTarget, right: &RenderTarget) -> bool {
    match (left, right) {
        (
            RenderTarget::Window(bevy::window::WindowRef::Primary),
            RenderTarget::Window(bevy::window::WindowRef::Primary),
        ) => true,
        (
            RenderTarget::Window(bevy::window::WindowRef::Entity(left)),
            RenderTarget::Window(bevy::window::WindowRef::Entity(right)),
        ) => left == right,
        (RenderTarget::Image(left), RenderTarget::Image(right)) => left == right,
        (RenderTarget::TextureView(left), RenderTarget::TextureView(right)) => left == right,
        (RenderTarget::None { size: left }, RenderTarget::None { size: right }) => left == right,
        _ => false,
    }
}

fn projections_equal(left: Option<&Projection>, right: Option<&Projection>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            std::mem::discriminant(left) == std::mem::discriminant(right)
                && left.get_clip_from_view().to_cols_array().map(f32::to_bits)
                    == right.get_clip_from_view().to_cols_array().map(f32::to_bits)
        }
        _ => false,
    }
}

fn oit_settings_equal(
    left: Option<OrderIndependentTransparencySettings>,
    right: Option<OrderIndependentTransparencySettings>,
) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            left.sorted_fragment_max_count == right.sorted_fragment_max_count
                && left.fragments_per_pixel_average.to_bits()
                    == right.fragments_per_pixel_average.to_bits()
                && left.alpha_threshold.to_bits() == right.alpha_threshold.to_bits()
        }
        _ => false,
    }
}

fn volumetric_fog_equal(left: Option<VolumetricFog>, right: Option<VolumetricFog>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            left.ambient_color == right.ambient_color
                && left.ambient_intensity.to_bits() == right.ambient_intensity.to_bits()
                && left.jitter.to_bits() == right.jitter.to_bits()
                && left.step_count == right.step_count
        }
        _ => false,
    }
}

fn validate_review_authority_teardown(
    state: &ReviewCaptureState,
    evidence: &ReviewAuthorityTeardownEvidence<'_, '_>,
) -> Result<(), String> {
    let baseline = state
        .authority_baseline
        .as_ref()
        .ok_or_else(|| "immutable authority baseline is missing after teardown".to_owned())?;
    if !state.authority_pre_teardown_verified {
        return Err("the final pre-teardown authority boundary was not verified".to_owned());
    }
    if state.authority_validated_captures != state.total_captures {
        return Err(format!(
            "only {}/{} persisted captures passed the authority guard",
            state.authority_validated_captures, state.total_captures
        ));
    }
    if evidence.world_snapshot.is_some()
        || evidence.time_of_day.is_some()
        || evidence.exterior_illumination.is_some()
        || evidence.illumination.is_some()
        || evidence.knowledge.is_some()
    {
        return Err(
            "session-owned world, lighting, or perception authority survived teardown".to_owned(),
        );
    }
    let replication = evidence.replication.as_deref().ok_or_else(|| {
        "WorldReplicationStateV1 disappeared instead of returning to its empty baseline".to_owned()
    })?;
    if replication.last_applied_sequence().is_some() {
        return Err(
            "WorldReplicationStateV1 retained an applied sequence after teardown".to_owned(),
        );
    }
    let registry = evidence
        .unit_registry
        .as_deref()
        .ok_or_else(|| "UnitRegistry disappeared instead of being cleared".to_owned())?;
    if registry.iter().next().is_some() || !evidence.units.is_empty() {
        return Err("registered or live Faction units survived teardown".to_owned());
    }
    if !evidence.logical_terrain.is_empty() || !evidence.terrain_batches.is_empty() {
        return Err(
            "logical terrain or TerrainRenderBatch picking state survived teardown".to_owned(),
        );
    }
    let persistence = encode_persistence_authority(
        evidence.campaign_store.as_deref(),
        evidence.campaign_save_status.as_deref(),
        evidence.storage_paths.as_deref(),
    )?;
    if persistence != baseline.persistence {
        return Err(
            "CampaignStore, save-status projection, or configured campaigns-file bytes changed during the review lifecycle"
                .to_owned(),
        );
    }
    Ok(())
}

fn capture_watchdog(
    screen: Res<State<Screen>>,
    ready: Option<Res<TerrainReady>>,
    failure: Option<Res<GameplaySetupFailure>>,
    mut state: ResMut<ReviewCaptureState>,
    mut exit: MessageWriter<AppExit>,
) {
    if state.failed {
        return;
    }
    if let Some(failure) = failure {
        error!(
            "review capture aborted after setup failure: {}",
            failure.reason
        );
        state.failed = true;
        exit.write(AppExit::error());
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
    if state.teardown_requested {
        state.enter_phase(CapturePhase::AwaitingTeardown, now);
        return (now.duration_since(state.phase_started) >= state.phase.timeout()).then(|| {
            format!(
                "procedural-map review timed out during {} after {:.1}s",
                state.phase.description(),
                state.phase.timeout().as_secs_f32()
            )
        });
    }
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

/// Resolves a standable generated anchor into a presentation-only camera target.
/// No actor, footing, transform, focus component, or gameplay authority is mutated.
fn resolve_review_focus_anchor(
    mut state: ResMut<ReviewCaptureState>,
    ready: Option<Res<TerrainReady>>,
    anchors: Option<Res<MapAnchors>>,
    table: Option<Res<SubstanceTable>>,
    blockers: Option<Res<TraversalBlockers>>,
    tiles: ReviewTileQuery,
    selected: Query<&Body, With<Selected>>,
    mut exit: MessageWriter<AppExit>,
) {
    if state.failed || state.focus_relocated {
        return;
    }
    let Some(anchor_name) = state.capture.focus_anchor.clone() else {
        state.focus_relocated = true;
        return;
    };
    if ready.is_none() {
        return;
    }
    let (Some(anchors), Some(table)) = (anchors, table) else {
        return;
    };
    let Ok(body) = selected.single() else {
        return;
    };

    let destination = resolve_review_focus(
        &anchor_name,
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

    state.focus_world_target = Some(destination.world_position());
    info!(
        "resolved review-only camera focus at generated anchor {:?} at {:?}",
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
    profile: Option<Res<ReviewWorldDetailProfileV1>>,
    mut images: ResMut<Assets<Image>>,
    targets: Query<&Transform, (With<CameraFocusTarget>, Without<PanOrbitCamera>)>,
    mut camera: Query<
        (
            Entity,
            &mut Transform,
            &mut PanOrbitCamera,
            &mut RenderTarget,
            Option<&mut Projection>,
            Option<&Camera3d>,
            Option<&Msaa>,
            Option<&ScreenSpaceTransmission>,
            Option<&OrderIndependentTransparencySettings>,
            Option<&VolumetricFog>,
            Option<&ReviewCameraFeatureRestore>,
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
    let Ok((
        camera_entity,
        mut transform,
        mut orbit,
        mut target,
        mut projection,
        camera_3d,
        msaa,
        transmission,
        oit,
        volumetric_fog,
        feature_restore,
    )) = camera.single_mut()
    else {
        return;
    };

    if profile.as_deref().is_some_and(|profile| {
        (profile.requires_oit()
            || profile.requires_transmission()
            || profile.requires_volumetrics())
            && feature_restore.is_none()
    }) {
        // Feature configuration records the pre-review state through deferred
        // commands. Wait one frame so the immutable camera snapshot observes that
        // record rather than mistaking the configured state for the baseline.
        return;
    }

    if state.camera_snapshot.is_none() {
        let (original_msaa, depth_texture_usages, original_transmission, oit, volumetric_fog) =
            feature_restore.map_or_else(
                || {
                    let default_camera_3d = Camera3d::default();
                    let default_transmission = ScreenSpaceTransmission::default();
                    (
                        msaa.copied().unwrap_or_default(),
                        camera_3d.map_or(default_camera_3d.depth_texture_usages, |camera| {
                            camera.depth_texture_usages
                        }),
                        transmission.cloned().unwrap_or(default_transmission),
                        oit.copied(),
                        volumetric_fog.copied(),
                    )
                },
                |restore| {
                    (
                        restore.msaa,
                        restore.depth_texture_usages,
                        restore.transmission.clone(),
                        restore.oit,
                        restore.volumetric_fog,
                    )
                },
            );
        state.camera_snapshot = Some(ReviewCameraSnapshot {
            entity: camera_entity,
            transform: *transform,
            orbit_focus: orbit.focus,
            orbit_radius: orbit.radius,
            target: target.clone(),
            projection: projection.as_deref().cloned(),
            mode: *mode,
            msaa: original_msaa,
            depth_texture_usages,
            transmission_steps: original_transmission.steps,
            transmission_quality: original_transmission.quality,
            oit,
            volumetric_fog,
        });
    }
    let Some(snapshot) = state.camera_snapshot.clone() else {
        error!("review camera snapshot was not initialized");
        state.failed = true;
        exit.write(AppExit::error());
        return;
    };
    if snapshot.entity != camera_entity {
        error!(
            "review camera identity changed from {:?} to {:?}",
            snapshot.entity, camera_entity
        );
        state.failed = true;
        exit.write(AppExit::error());
        return;
    }
    *transform = snapshot.transform;
    orbit.focus = snapshot.orbit_focus;
    orbit.radius = snapshot.orbit_radius;
    *mode = snapshot.mode;
    match (snapshot.projection, projection.as_deref_mut()) {
        (Some(original), Some(current)) => *current = original,
        (None, None) => {}
        _ => {
            error!("review camera projection component changed during capture");
            state.failed = true;
            exit.write(AppExit::error());
            return;
        }
    }

    if state.target.is_none() {
        let image = Image::new_target_texture(
            CAPTURE_WIDTH,
            CAPTURE_HEIGHT,
            TextureFormat::Rgba8UnormSrgb,
            None,
        );
        let handle = images.add(image);
        state.capture_target_ids.insert(handle.id());
        *target = RenderTarget::Image(handle.clone().into());
        state.target = Some(handle);
    } else if let Some(handle) = state.target.as_ref() {
        *target = RenderTarget::Image(handle.clone().into());
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
    let close_camera_target = state
        .focus_world_target
        .or_else(|| targets.single().ok().map(|target| target.translation));
    match state.capture.camera {
        ReviewCamera::Map => *mode = CameraMode::Map,
        ReviewCamera::Character => {
            let Some(target) = close_camera_target else {
                return;
            };
            apply_character_camera_view(
                eye,
                focus,
                target,
                &settings,
                state.capture.character_radius_scale,
                &mut transform,
                &mut orbit,
            );
            *mode = CameraMode::Character;
        }
        ReviewCamera::FirstPerson => {
            let Some(target) = close_camera_target else {
                return;
            };
            apply_first_person_camera_view(
                eye,
                focus,
                target,
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
    if state.focus_world_target.is_some() && state.capture.camera != ReviewCamera::Map {
        state.focus_pose = Some(ReviewFocusPose {
            transform: *transform,
            orbit_focus: orbit.focus,
            orbit_radius: orbit.radius,
        });
    }
    state.view_applied = true;
}

/// Keep an explicitly anchored review camera at its resolved viewpoint through
/// every settle frame. The ordinary follower still owns native gameplay, and
/// actors retain their exact original footing, transforms and focus components.
/// These are fixed review views using the configured close-camera lens and eye
/// height, not evidence for native following or collision response.
fn pin_review_focus_pose(
    state: Res<ReviewCaptureState>,
    mut cameras: Query<(&mut Transform, &mut PanOrbitCamera)>,
) {
    if state.failed || state.teardown_requested || !state.view_applied {
        return;
    }
    let (Some(pose), Some(snapshot)) = (&state.focus_pose, &state.camera_snapshot) else {
        return;
    };
    let Ok((mut transform, mut orbit)) = cameras.get_mut(snapshot.entity) else {
        return;
    };
    *transform = pose.transform;
    orbit.focus = pose.orbit_focus;
    orbit.radius = pose.orbit_radius;
}

/// Restores the exact camera pose, lens, orbit state, target, and mode captured
/// before the first review view. The temporary 1080p render target is removed
/// only after the ordinary target is back on the camera.
fn restore_review_capture_camera(
    mut state: ResMut<ReviewCaptureState>,
    mut images: ResMut<Assets<Image>>,
    mut cameras: Query<(
        &mut Transform,
        &mut PanOrbitCamera,
        &mut RenderTarget,
        Option<&mut Projection>,
    )>,
    mut mode: ResMut<CameraMode>,
) {
    let Some(snapshot) = state.camera_snapshot.clone() else {
        error!("review teardown has no captured camera baseline");
        state.failed = true;
        return;
    };
    let Ok((mut transform, mut orbit, mut target, mut projection)) =
        cameras.get_mut(snapshot.entity)
    else {
        error!(
            "review camera {:?} disappeared before teardown",
            snapshot.entity
        );
        state.failed = true;
        return;
    };
    *transform = snapshot.transform;
    orbit.focus = snapshot.orbit_focus;
    orbit.radius = snapshot.orbit_radius;
    *target = snapshot.target;
    *mode = snapshot.mode;
    match (snapshot.projection, projection.as_deref_mut()) {
        (Some(original), Some(current)) => *current = original,
        (None, None) => {}
        _ => {
            error!("review camera projection component changed before teardown");
            state.failed = true;
            return;
        }
    }
    state.camera_restored = true;
    state.target_removed = state
        .target
        .take()
        .is_none_or(|handle| images.remove(handle.id()).is_some());
    if !state.target_removed {
        error!("review render target was missing before teardown could remove it");
        state.failed = true;
    }
}

/// Waits until the gameplay exit systems have restored disposable map and camera
/// state before allowing the headless capture process to exit successfully.
fn finish_review_capture_after_teardown(
    mut commands: Commands,
    mut state: ResMut<ReviewCaptureState>,
    report: Option<Res<ReviewWorldDetailReportV1>>,
    pending_lifecycle_teardown: Option<Res<ReviewLifecycleCycleTeardownPendingV1>>,
    mut lifecycle: Option<ResMut<ReviewLifecycleProbeV1>>,
    lifecycle_evidence: ReviewLifecycleTeardownEvidence,
    authority: ReviewAuthorityTeardownEvidence,
    mut liquid_time: Option<ResMut<LiquidVisualTime>>,
    mut exit: MessageWriter<AppExit>,
) {
    if !state.teardown_requested || state.final_exit_sent || report.is_some() {
        return;
    }
    if state.failed || !state.camera_restored || !state.target_removed || state.target.is_some() {
        error!(
            "review teardown failed (camera_restored={}, target_removed={}, target_live={})",
            state.camera_restored,
            state.target_removed,
            state.target.is_some()
        );
        exit.write(AppExit::error());
        return;
    }

    if let Some(lifecycle) = lifecycle.as_deref_mut() {
        if pending_lifecycle_teardown.is_none() {
            error!("lifecycle probe reached its verifier without a pending teardown marker");
            state.failed = true;
            exit.write(AppExit::error());
            return;
        }
        if !state.authority_pre_teardown_verified
            || state.authority_validated_captures != state.total_captures
        {
            error!(
                "lifecycle cycle persisted only {}/{} authority-validated captures",
                state.authority_validated_captures, state.total_captures
            );
            state.failed = true;
            exit.write(AppExit::error());
            return;
        }
        let teardown = match validate_review_lifecycle_teardown(&state, &lifecycle_evidence) {
            Ok(teardown) => teardown,
            Err(error) => {
                error!("review lifecycle teardown verification failed: {error}");
                state.failed = true;
                exit.write(AppExit::error());
                return;
            }
        };
        let Some(baseline) = state.authority_baseline.as_ref() else {
            error!("review lifecycle teardown lost its authority baseline");
            state.failed = true;
            exit.write(AppExit::error());
            return;
        };
        let authority_before_sha256 = baseline.sha256();
        let authority_after_sha256 = teardown.authority_after.sha256();
        if state.authority_after_sha256.as_deref() != Some(authority_after_sha256.as_str()) {
            error!(
                "final screenshot authority evidence differs from post-projection teardown authority"
            );
            state.failed = true;
            exit.write(AppExit::error());
            return;
        }
        let cycle_index = match lifecycle.next_cycle_index() {
            Ok(index) => index,
            Err(error) => {
                error!("cannot advance review lifecycle: {error}");
                state.failed = true;
                exit.write(AppExit::error());
                return;
            }
        };
        if cycle_index > lifecycle.configuration.cycles_requested {
            error!("review lifecycle attempted more cycles than requested");
            state.failed = true;
            exit.write(AppExit::error());
            return;
        }
        let cycle = match ReviewLifecycleCycleV1::from_hash_body(ReviewLifecycleCycleHashBodyV1 {
            cycle_index,
            launch_nonce: lifecycle.runtime_receipt.launch_nonce.clone(),
            runtime_receipt_sha256: lifecycle.runtime_receipt.receipt_sha256.clone(),
            profile_hash_sha256: lifecycle.configuration.tested_profile_sha256.clone(),
            authority_before_sha256,
            authority_after_sha256,
            entities_remaining: teardown.entities_remaining,
            materials_remaining: teardown.materials_remaining,
            meshes_remaining: teardown.meshes_remaining,
            fog_density_images_remaining: teardown.fog_density_images_remaining,
            target_images_remaining: teardown.target_images_remaining,
            terrain_material_overrides_remaining: teardown.terrain_material_overrides_remaining,
            liquid_visibility_overrides_remaining: teardown.liquid_visibility_overrides_remaining,
            vegetation_scale_overrides_remaining: teardown.vegetation_scale_overrides_remaining,
            camera_state_restored: teardown.camera_state_restored,
            oit_state_restored: teardown.oit_state_restored,
            transmission_state_restored: teardown.transmission_state_restored,
            depth_state_restored: teardown.depth_state_restored,
            volumetric_state_restored: teardown.volumetric_state_restored,
            previous_cycle_sha256: lifecycle.previous_cycle_sha256(),
        }) {
            Ok(cycle) => cycle,
            Err(error) => {
                error!("cannot hash review lifecycle cycle: {error}");
                state.failed = true;
                exit.write(AppExit::error());
                return;
            }
        };
        lifecycle.cycles.push(cycle);

        if lifecycle.cycles.len() < usize::from(lifecycle.configuration.cycles_requested) {
            let captures = lifecycle.capture_templates.clone();
            let Some(first_capture) = captures.first() else {
                error!("review lifecycle lost its capture template");
                state.failed = true;
                exit.write(AppExit::error());
                return;
            };
            if let Some(phase) = first_capture.liquid_phase_seconds {
                let Some(liquid_time) = liquid_time.as_deref_mut() else {
                    error!("review lifecycle lost LiquidVisualTime before re-entry");
                    state.failed = true;
                    exit.write(AppExit::error());
                    return;
                };
                if !liquid_time.freeze(phase) {
                    error!("review lifecycle capture template has a non-finite liquid phase");
                    state.failed = true;
                    exit.write(AppExit::error());
                    return;
                }
            }
            let authority_baseline = teardown.authority_after;
            *state = ReviewCaptureState::new_many(captures);
            state.authority_baseline = Some(authority_baseline);
            state.enter_phase(CapturePhase::AwaitingCamera, Instant::now());
            commands.remove_resource::<ReviewLifecycleCycleTeardownPendingV1>();
            commands.insert_resource(ReviewLifecycleProjectionReentryPendingV1);
            info!(
                "verified review projection lifecycle cycle {cycle_index}/{}; re-entering the projection in-process",
                lifecycle.configuration.cycles_requested
            );
            return;
        }

        let cycles_completed = match u16::try_from(lifecycle.cycles.len()) {
            Ok(count) => count,
            Err(_) => {
                error!("review lifecycle completed-cycle count exceeds u16");
                state.failed = true;
                exit.write(AppExit::error());
                return;
            }
        };
        let cleanup = ReviewCleanupStateV1 {
            completed_cycles: cycles_completed,
            entities_remaining: teardown.entities_remaining,
            materials_remaining: teardown.materials_remaining,
            meshes_remaining: teardown.meshes_remaining,
            target_images_remaining: teardown.target_images_remaining,
            camera_state_restored: teardown.camera_state_restored,
            oit_state_restored: teardown.oit_state_restored,
            transmission_state_restored: teardown.transmission_state_restored,
            depth_state_restored: teardown.depth_state_restored,
            volumetric_state_restored: teardown.volumetric_state_restored,
        };
        if let Err(error) = finalize_capture_runtime_report_cleanup(
            &state.runtime_report_paths,
            state.total_captures,
            &cleanup,
        ) {
            error!("cannot finalize lifecycle runtime-report cleanup evidence: {error}");
            state.failed = true;
            exit.write(AppExit::error());
            return;
        }
        let Some(final_chain_sha256) = lifecycle
            .cycles
            .last()
            .map(|cycle| cycle.cycle_sha256.as_str())
        else {
            error!("review lifecycle produced no cycle records");
            state.failed = true;
            exit.write(AppExit::error());
            return;
        };
        let certificate = ReviewLifecycleCertificateV1 {
            version: 1,
            warning: WORLD_DETAIL_WARNING,
            runtime_receipt: &lifecycle.runtime_receipt,
            capture_plan_sha256: &lifecycle.configuration.capture_plan_sha256,
            source_provenance_sha256: &lifecycle.configuration.source_provenance_sha256,
            profile_matrix_sha256: &lifecycle.configuration.profile_matrix_sha256,
            tested_profile_sha256: &lifecycle.configuration.tested_profile_sha256,
            cycles_requested: lifecycle.configuration.cycles_requested,
            cycles_completed,
            cycles: &lifecycle.cycles,
            final_chain_sha256,
        };
        let certificate_json = match serde_json::to_string(&certificate) {
            Ok(json) => json,
            Err(error) => {
                error!("cannot serialize review lifecycle certificate: {error}");
                state.failed = true;
                exit.write(AppExit::error());
                return;
            }
        };
        if let Err(error) =
            persist_runtime_report(&lifecycle.configuration.certificate_path, &certificate_json)
        {
            error!("cannot persist review lifecycle certificate: {error}");
            state.failed = true;
            exit.write(AppExit::error());
            return;
        }
        // Retain the teardown marker through the terminal frame so the later
        // PostUpdate camera-feature system cannot reapply review state after the
        // certificate's exact restoration boundary.
        state.final_exit_sent = true;
        info!(
            "completed {} genuine in-process review projection lifecycles: {}",
            cycles_completed,
            lifecycle.configuration.certificate_path.display()
        );
        exit.write(AppExit::Success);
        return;
    }

    if let Err(error) = validate_review_authority_teardown(&state, &authority) {
        error!("review authority teardown verification failed: {error}");
        state.failed = true;
        exit.write(AppExit::error());
        return;
    }
    let presentation_teardown =
        match validate_review_presentation_teardown(&state, &lifecycle_evidence) {
            Ok(teardown) => teardown,
            Err(error) => {
                error!("review presentation teardown verification failed: {error}");
                state.failed = true;
                exit.write(AppExit::error());
                return;
            }
        };
    let cleanup = presentation_teardown.cleanup(1);
    if let Err(error) = finalize_capture_runtime_report_cleanup(
        &state.runtime_report_paths,
        state.total_captures,
        &cleanup,
    ) {
        error!("cannot finalize capture runtime-report cleanup evidence: {error}");
        state.failed = true;
        exit.write(AppExit::error());
        return;
    }
    let Some(baseline) = state.authority_baseline.as_ref() else {
        error!("review authority teardown verification lost its baseline");
        state.failed = true;
        exit.write(AppExit::error());
        return;
    };
    let teardown_certificate = serde_json::json!({
        "version": 1,
        "warning": WORLD_DETAIL_WARNING,
        "authority_guard": {
            "baseline_fingerprint": baseline.fingerprint(),
            "lighting_condition_key": baseline.lighting_condition_key(),
            "validated_captures": state.authority_validated_captures,
            "total_captures": state.total_captures,
            "final_pre_teardown_match": state.authority_pre_teardown_verified,
            "world_snapshot_removed": authority.world_snapshot.is_none(),
            "time_of_day_removed": authority.time_of_day.is_none(),
            "exterior_illumination_removed": authority.exterior_illumination.is_none(),
            "resolved_illumination_removed": authority.illumination.is_none(),
            "faction_knowledge_removed": authority.knowledge.is_none(),
            "unit_registry_cleared": authority
                .unit_registry
                .as_deref()
                .is_some_and(|registry| registry.iter().next().is_none()),
            "live_units_removed": authority.units.is_empty(),
            "logical_terrain_removed": authority.logical_terrain.is_empty(),
            "terrain_picking_removed": authority.terrain_batches.is_empty(),
            "replication_sequence_cleared": authority
                .replication
                .as_deref()
                .is_some_and(|replication| replication.last_applied_sequence().is_none()),
        },
        "presentation_cleanup": cleanup,
    });
    let certificate_json = match serde_json::to_string(&teardown_certificate) {
        Ok(json) => json,
        Err(error) => {
            error!("cannot serialize review authority teardown certificate: {error}");
            state.failed = true;
            exit.write(AppExit::error());
            return;
        }
    };
    let certificate_path = match authority_teardown_report_path(&state.capture.path) {
        Ok(path) => path,
        Err(error) => {
            error!("cannot resolve review authority teardown certificate: {error}");
            state.failed = true;
            exit.write(AppExit::error());
            return;
        }
    };
    if let Err(error) = persist_runtime_report(&certificate_path, &certificate_json) {
        error!("cannot persist review authority teardown certificate: {error}");
        state.failed = true;
        exit.write(AppExit::error());
        return;
    }
    info!(
        "review teardown restored the camera, removed the temporary target, and cleared authoritative session state: {}",
        certificate_path.display()
    );
    state.final_exit_sent = true;
    exit.write(AppExit::Success);
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
    let mut dark_count = 0usize;
    let mut dim_count = 0usize;
    let mut bright_count = 0usize;
    for surface in surfaces {
        match surface.level {
            IlluminationLevel::Dark => dark_count += 1,
            IlluminationLevel::Dim => dim_count += 1,
            IlluminationLevel::Bright => bright_count += 1,
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
        dark_count, dim_count, bright_count
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
                 {batch:?}, which lacks exactly one ordinary material owner or exclusive review water suppression"
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
    terrain_batches: Query<ReviewCaptureTerrainBatchQuery>,
    logical_runs: Query<(Entity, Option<&TilePos>, Option<&HexSpan>), With<HexTile>>,
    review_cameras: Query<(&Camera, &GlobalTransform, &Projection), With<PanOrbitCamera>>,
    evidence: ReviewRuntimeEvidence,
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
    state.settled_frames = state.settled_frames.saturating_add(1);
    let evidence_start_frame = if state.performance_sample.is_some() {
        state.capture.settle_frames
    } else {
        performance_sampling_start_frame(state.capture.settle_frames)
    };
    if state.settled_frames < evidence_start_frame {
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
                        has_material: review_terrain_material_tag(material).is_some(),
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

    // Visibility diagnostics stay independently testable, but no real capture
    // may proceed until the complete authority and renderer evidence exists.
    let (Some(profile), Some(_report), Some(_authority_baseline)) = (
        evidence.profile.as_deref(),
        evidence.report.as_deref(),
        state.authority_baseline.as_ref(),
    ) else {
        return;
    };

    let authority = match verify_review_authority(&state, &evidence.authority) {
        Ok(authority) => authority,
        Err(error) => {
            error!("review screenshot blocked by immutable authority guard: {error}");
            state.failed = true;
            exit.write(AppExit::error());
            return;
        }
    };

    let projection_audit = match audit_review_projection_entities(&evidence) {
        Ok(audit) => audit,
        Err(error) => {
            error!("review projection authority-component audit failed: {error}");
            state.failed = true;
            exit.write(AppExit::error());
            return;
        }
    };
    state
        .review_entity_ids
        .extend(projection_audit.entities.iter().copied());
    state
        .review_mesh_ids
        .extend(projection_audit.meshes.iter().copied());
    state
        .review_standard_material_ids
        .extend(projection_audit.standard_materials.iter().copied());
    state
        .review_image_ids
        .extend(projection_audit.images.iter().copied());

    let performance = if let Some(sample) = state.performance_sample {
        sample
    } else {
        match sample_review_performance(
            &mut state,
            &evidence,
            projection_audit.public_payload_bytes,
        ) {
            Ok(Some(sample)) => {
                state.performance_sample = Some(sample);
                sample
            }
            Ok(None) => return,
            Err(error) => {
                error!("cannot sample review performance: {error}");
                state.failed = true;
                exit.write(AppExit::error());
                return;
            }
        }
    };
    if state.settled_frames < state.capture.settle_frames {
        return;
    }

    let (runtime_report_path, runtime_report_json) =
        match build_capture_runtime_report(&state, &evidence, &authority, performance) {
            Ok(report) => report,
            Err(error) => {
                error!("cannot build world-detail runtime report: {error}");
                state.failed = true;
                exit.write(AppExit::error());
                return;
            }
        };
    let readback_binding = match build_capture_readback_binding(
        profile,
        evidence.report.as_deref(),
        evidence.render_adapter.as_deref(),
        evidence.render_device.as_deref(),
        &evidence.cameras,
    ) {
        Ok(binding) => binding,
        Err(error) => {
            error!("cannot bind world-detail report to screenshot request: {error}");
            state.failed = true;
            exit.write(AppExit::error());
            return;
        }
    };

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
    let observer_report_path = runtime_report_path;
    let observer_report_json = runtime_report_json;
    let observer_readback_binding = readback_binding;
    commands.spawn(Screenshot::image(target)).observe(
        move |captured: On<ScreenshotCaptured>,
              mut commands: Commands,
              mut state: ResMut<ReviewCaptureState>,
              authority: ReviewAuthorityEvidence,
              readback: ReviewCaptureReadbackEvidence,
              lifecycle: Option<Res<ReviewLifecycleProbeV1>>,
              mut liquid_time: ResMut<LiquidVisualTime>,
              mut next_screen: ResMut<NextState<Screen>>,
              mut exit: MessageWriter<AppExit>| {
            let result = build_capture_readback_binding(
                &readback.profile,
                readback.report.as_deref(),
                readback.render_adapter.as_deref(),
                readback.render_device.as_deref(),
                &readback.cameras,
            )
            .and_then(|current| observer_readback_binding.verify_same(&current))
            .and_then(|()| verify_review_authority(&state, &authority))
            .and_then(|snapshot| {
                persist_screenshot(&captured.image, &observer_output)?;
                persist_runtime_report(&observer_report_path, &observer_report_json)?;
                state
                    .runtime_report_paths
                    .push(observer_report_path.clone());
                Ok(snapshot.sha256())
            });
            match result {
                Ok(authority_after_sha256) => {
                    state.authority_after_sha256 = Some(authority_after_sha256);
                    state.authority_validated_captures =
                        state.authority_validated_captures.saturating_add(1);
                    info!("review screenshot completed: {}", observer_output.display());
                    if let Some(next) = state.advance_capture(Instant::now()) {
                        if state
                            .capture
                            .liquid_phase_seconds
                            .is_some_and(|phase| !liquid_time.freeze(phase))
                        {
                            error!("next review capture has a non-finite liquid phase");
                            state.failed = true;
                            exit.write(AppExit::error());
                            return;
                        }
                        info!(
                            "advancing review capture sequence to {}/{}: {}",
                            state.completed_captures + 1,
                            state.total_captures,
                            next.display()
                        );
                    } else {
                        state.authority_pre_teardown_verified = true;
                        state.requested = false;
                        state.teardown_requested = true;
                        state.enter_phase(CapturePhase::AwaitingTeardown, Instant::now());
                        if lifecycle.is_some() {
                            commands.insert_resource(ReviewWorldDetailTeardownRequestV1);
                            commands.insert_resource(
                                ReviewLifecycleCycleTeardownPendingV1,
                            );
                            info!(
                                "all {} review captures completed; requesting verified in-place projection teardown",
                                state.total_captures
                            );
                        } else {
                            next_screen.set(Screen::Title);
                            info!(
                                "all {} review captures completed; requesting verified gameplay teardown",
                                state.total_captures
                            );
                        }
                    }
                }
                Err(error) => {
                    error!(
                        "review screenshot failed for {}: {error}",
                        observer_output.display()
                    );
                    state.failed = true;
                    exit.write(AppExit::error());
                }
            }
        },
    );
    state.requested = true;
    state.enter_phase(CapturePhase::Readback, Instant::now());
    info!("requested review screenshot: {}", output.display());
}

fn build_capture_runtime_report(
    state: &ReviewCaptureState,
    evidence: &ReviewRuntimeEvidence<'_, '_>,
    authority: &ReviewAuthoritySnapshotV1,
    performance: ReviewPerformanceSampleV1,
) -> Result<(PathBuf, String), String> {
    let capture = &state.capture;
    let source_report = evidence
        .report
        .as_deref()
        .ok_or_else(|| "world-detail projection report is not published".to_owned())?;
    let mut active_cameras = evidence.cameras.iter().filter(|camera| camera.1.is_active);
    let Some((_, _, _, _, _, _, _, camera_3d, _, transmission, oit, volumetric)) =
        active_cameras.next()
    else {
        return Err("no active world review camera is available".to_owned());
    };
    if active_cameras.next().is_some() {
        return Err("multiple active world review cameras are available".to_owned());
    }
    let camera_features = resolved_review_camera_features(
        evidence
            .profile
            .as_deref()
            .ok_or_else(|| "world-detail profile is not published".to_owned())?,
        camera_3d,
        oit.is_some(),
        transmission,
        volumetric,
        evidence.render_adapter.as_deref(),
        evidence.render_device.as_deref(),
    )?;

    let mut report = source_report.clone();
    report.camera_features = camera_features;
    report.performance = performance;
    let target = state
        .target
        .as_ref()
        .ok_or_else(|| "capture report has no live target image handle".to_owned())?;
    if evidence
        .images
        .as_deref()
        .ok_or_else(|| "Assets<Image> is unavailable for capture reporting".to_owned())?
        .get(target.id())
        .is_none()
    {
        return Err("capture report target image is not live in Assets<Image>".to_owned());
    }
    report.cleanup.target_images_remaining = 1;
    report.authority.logical_terrain_picking = authority.logical_terrain_picking_fingerprint();
    report.authority.gameplay_state = authority.gameplay_state_fingerprint();
    report.validate().map_err(|error| error.to_string())?;
    let look_at_anchor = capture
        .anchor_look_at
        .as_ref()
        .map(|look_at| look_at.anchor.as_str());
    let look_at_offset = capture
        .anchor_look_at
        .as_ref()
        .map(|look_at| look_at.offset.to_array());
    let runtime = serde_json::json!({
        "version": 1,
        "warning": WORLD_DETAIL_WARNING,
        "capture": {
            "path": capture.path.to_string_lossy(),
            "camera": capture.camera.as_str(),
            "view": capture.view.as_str(),
            "focus_anchor": capture.focus_anchor.as_deref(),
            "look_at_anchor": look_at_anchor,
            "look_at_offset": look_at_offset,
            "character_radius_scale": capture.character_radius_scale,
            "full_cutaway": capture.full_cutaway,
            "illumination_overlay": capture.illumination_overlay,
            "settle_frames": capture.settle_frames,
            "time_hours": evidence.authority.time_of_day.as_deref().map(|time| time.hours),
            "liquid_phase_seconds": evidence
                .liquid_visual_time
                .as_deref()
                .map(LiquidVisualTime::phase_seconds),
        },
        "report": report,
    });
    let json = serde_json::to_string(&runtime)
        .map_err(|error| format!("cannot serialize capture runtime report: {error}"))?;
    Ok((runtime_report_path(&capture.path)?, json))
}

fn resolved_review_camera_features(
    profile: &ReviewWorldDetailProfileV1,
    camera_3d: &Camera3d,
    has_oit: bool,
    transmission: &ScreenSpaceTransmission,
    volumetric: Option<&VolumetricFog>,
    render_adapter: Option<&RenderAdapter>,
    render_device: Option<&RenderDevice>,
) -> Result<ReviewCameraFeaturesV1, String> {
    let depth_texture = TextureUsages::from(camera_3d.depth_texture_usages)
        .contains(TextureUsages::TEXTURE_BINDING);
    let camera_features = ReviewCameraFeaturesV1 {
        oit: operational_oit_available(has_oit, render_adapter, render_device),
        medium_transmission: profile.requires_transmission()
            && transmission.steps > 0
            && transmission.quality == ScreenSpaceTransmissionQuality::Medium,
        depth_texture,
        volumetrics: volumetric.is_some(),
    };
    if profile.requires_oit() && !camera_features.oit {
        return Err(
            "profile requires operational OIT but the adapter/device capability preflight failed"
                .to_owned(),
        );
    }
    if profile.requires_transmission() && !camera_features.medium_transmission {
        return Err(
            "profile requires medium screen-space transmission but the active camera does not provide it"
                .to_owned(),
        );
    }
    if (profile.requires_oit() || profile.requires_transmission()) && !camera_features.depth_texture
    {
        return Err(
            "profile requires a sampled depth texture but the active camera lacks it".to_owned(),
        );
    }
    if profile.requires_volumetrics() && !camera_features.volumetrics {
        return Err(
            "profile requires volumetric processing but the active camera does not provide it"
                .to_owned(),
        );
    }
    Ok(camera_features)
}

fn build_capture_readback_binding(
    profile: &ReviewWorldDetailProfileV1,
    report: Option<&ReviewWorldDetailReportV1>,
    render_adapter: Option<&RenderAdapter>,
    render_device: Option<&RenderDevice>,
    cameras: &Query<'_, '_, ReviewRuntimeCameraQuery, With<PanOrbitCamera>>,
) -> Result<ReviewCaptureReadbackBindingV1, String> {
    let report = report
        .ok_or_else(|| "world-detail projection report disappeared before screenshot".to_owned())?;
    let profile_hash_sha256 = profile
        .profile_hash_sha256()
        .map_err(|error| format!("cannot hash readback profile: {error}"))?;
    if report.profile_hash_sha256 != profile_hash_sha256 {
        return Err("world-detail report profile changed before screenshot".to_owned());
    }
    let mut active_cameras = cameras.iter().filter(|camera| camera.1.is_active);
    let Some((
        entity,
        camera,
        transform,
        global_transform,
        orbit,
        render_target,
        projection,
        camera_3d,
        msaa,
        transmission,
        oit,
        volumetric_fog,
    )) = active_cameras.next()
    else {
        return Err("no active world review camera is available for readback binding".to_owned());
    };
    if active_cameras.next().is_some() {
        return Err(
            "multiple active world review cameras are available for readback binding".to_owned(),
        );
    }
    let camera_features = resolved_review_camera_features(
        profile,
        camera_3d,
        oit.is_some(),
        transmission,
        volumetric_fog,
        render_adapter,
        render_device,
    )?;
    Ok(ReviewCaptureReadbackBindingV1 {
        profile_hash_sha256,
        projection_hashes: report.projection_hashes.clone(),
        counts: report.counts.clone(),
        camera_features,
        camera_entity: entity,
        transform: *transform,
        global_transform_bits: global_transform
            .to_matrix()
            .to_cols_array()
            .map(f32::to_bits),
        orbit_focus_bits: orbit.focus.to_array().map(f32::to_bits),
        orbit_radius_bits: orbit.radius.to_bits(),
        render_target: render_target.clone(),
        projection: projection.cloned(),
        clip_from_view_bits: camera.clip_from_view().to_cols_array().map(f32::to_bits),
        msaa: *msaa,
        depth_texture_usages: camera_3d.depth_texture_usages.0,
        transmission_steps: transmission.steps,
        transmission_quality: transmission.quality,
        oit: oit.copied(),
        volumetric_fog: volumetric_fog.copied(),
    })
}

fn operational_oit_available(
    camera_has_settings: bool,
    adapter: Option<&RenderAdapter>,
    device: Option<&RenderDevice>,
) -> bool {
    camera_has_settings
        && adapter
            .zip(device)
            .is_some_and(|(adapter, device)| is_oit_supported(adapter, device, false))
}

fn runtime_report_path(capture: &Path) -> Result<PathBuf, String> {
    let stem = capture
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| "review capture filename is not valid UTF-8".to_owned())?;
    let file_name = format!("{stem}.world-detail-report.json");
    Ok(capture.with_file_name(file_name))
}

fn authority_teardown_report_path(capture: &Path) -> Result<PathBuf, String> {
    let stem = capture
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| "review capture filename is not valid UTF-8".to_owned())?;
    Ok(capture.with_file_name(format!("{stem}.authority-teardown-report.json")))
}

fn persist_runtime_report(path: &Path, json: &str) -> Result<(), String> {
    prepare_capture_path(path)
        .map_err(|error| format!("cannot prepare runtime report path: {error}"))?;
    AtomicFile::new(path, AllowOverwrite)
        .write(|file| file.write_all(json.as_bytes()))
        .map_err(|error| format!("cannot atomically write runtime report: {error}"))
}

fn finalize_capture_runtime_report_cleanup(
    paths: &[PathBuf],
    expected_reports: usize,
    cleanup: &ReviewCleanupStateV1,
) -> Result<(), String> {
    if paths.len() != expected_reports {
        return Err(format!(
            "only {}/{} persisted capture reports reached teardown finalization",
            paths.len(),
            expected_reports
        ));
    }
    if paths.iter().collect::<BTreeSet<_>>().len() != paths.len() {
        return Err("duplicate capture runtime-report paths reached teardown".to_owned());
    }
    if !cleanup.is_complete() || cleanup.completed_cycles == 0 {
        return Err("capture runtime reports require a completed teardown state".to_owned());
    }
    for path in paths {
        let bytes = fs::read(path).map_err(|error| {
            format!(
                "cannot read persisted runtime report {} for teardown finalization: {error}",
                path.display()
            )
        })?;
        let mut runtime: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| {
            format!(
                "cannot parse persisted runtime report {} for teardown finalization: {error}",
                path.display()
            )
        })?;
        let report_value = runtime
            .get_mut("report")
            .ok_or_else(|| format!("runtime report {} has no report object", path.display()))?;
        let mut report: ReviewWorldDetailReportV1 = serde_json::from_value(report_value.take())
            .map_err(|error| {
                format!(
                    "cannot decode world-detail report {} for teardown finalization: {error}",
                    path.display()
                )
            })?;
        report.cleanup = cleanup.clone();
        report.validate().map_err(|error| {
            format!(
                "teardown-finalized world-detail report {} is invalid: {error}",
                path.display()
            )
        })?;
        *report_value = serde_json::to_value(report).map_err(|error| {
            format!(
                "cannot encode teardown-finalized world-detail report {}: {error}",
                path.display()
            )
        })?;
        let json = serde_json::to_string(&runtime).map_err(|error| {
            format!(
                "cannot serialize teardown-finalized runtime report {}: {error}",
                path.display()
            )
        })?;
        persist_runtime_report(path, &json)?;
    }
    Ok(())
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

// A typed, renderer-free seam for the combined-feature launch ownership test.
// It deliberately registers the actual launch system without capture or camera
// presentation systems, which require the renderer under test elsewhere.
#[cfg(all(test, feature = "dev"))]
pub(super) fn install_headless_review_launch_for_test(
    app: &mut App,
    scenario: Option<&str>,
) -> Result<(), String> {
    let request = ReviewRequest::from_values(
        scenario.map(str::to_owned),
        scenario.map(|_| "1592598566".to_owned()),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )?;
    if let Some(request) = request {
        app.insert_resource(request).add_systems(
            Update,
            launch_review_scenario.run_if(in_state(Screen::Title)),
        );
    }
    Ok(())
}

#[cfg(all(test, feature = "dev"))]
pub(super) fn headless_review_launch_state_for_test(world: &World) -> Option<bool> {
    world
        .get_resource::<ReviewRequest>()
        .map(|request| request.launched)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::{Extent3d, PrimitiveTopology, TextureDimension};
    use bevy::state::app::StatesPlugin;
    use hex_assets::{ArtPalette, ScenarioCategory, Substance, SubstanceFile, SwatchId};
    use hex_core::{
        ExteriorIllumination, GameplayLight, HexCoord, InteriorRegionId, TerrainChunkRoot,
        TerrainPickRun, TraversalProfile,
    };
    use hex_perception::LightSourceSnapshot;

    use crate::capture::{has_visual_coverage, temporary_capture_path};

    use super::*;

    fn authority_snapshot_fixture() -> ReviewAuthoritySnapshotV1 {
        ReviewAuthoritySnapshotV1 {
            current_world: vec![1],
            replication: vec![2],
            lighting_condition: vec![3],
            illumination: vec![4],
            faction_knowledge: vec![5],
            units_and_occupancy: vec![6],
            logical_terrain: vec![7],
            terrain_picking: vec![8],
            persistence: vec![9],
        }
    }

    #[derive(Resource, Default)]
    struct AuthorityTeardownObservation(Option<Result<(), String>>);

    fn observe_authority_teardown(
        state: Res<ReviewCaptureState>,
        evidence: ReviewAuthorityTeardownEvidence,
        mut observation: ResMut<AuthorityTeardownObservation>,
    ) {
        observation.0 = Some(validate_review_authority_teardown(&state, &evidence));
    }

    #[test]
    fn original_water_binding_is_required_once_for_authority_and_capture_readiness() {
        use bevy::ecs::system::SystemState;

        fn inspect(
            world: &mut World,
        ) -> (
            Result<Vec<u8>, String>,
            Result<FullFootprintCoverage, ReviewCaptureCoverageError>,
        ) {
            let mut query = SystemState::<(
                ReviewAuthorityEvidence,
                Query<ReviewCaptureTerrainBatchQuery>,
            )>::new(world);
            let (evidence, batches) = query
                .get(world)
                .expect("review authority queries are available");
            let authority = validate_authority_terrain_projection(&evidence)
                .and_then(|()| encode_terrain_picking_authority(&evidence));
            let coverage = validate_full_footprint_capture(
                evidence
                    .logical_terrain
                    .iter()
                    .map(|(entity, position, _, span, _, _, _)| {
                        (entity, position.copied(), span.copied())
                    }),
                batches
                    .iter()
                    .map(
                        |(entity, batch, mesh, material, visibility)| ReviewTerrainBatch {
                            entity,
                            batch,
                            has_mesh: mesh.is_some(),
                            has_material: review_terrain_material_tag(material).is_some(),
                            visible: visibility.is_some_and(|visibility| visibility.get()),
                        },
                    ),
                std::iter::empty::<(&Camera, &GlobalTransform, &Projection)>(),
            );
            (authority, coverage)
        }

        let mut world = World::new();
        let position = TilePos::new(HexCoord::from_axial(2, -1), 3);
        let span = HexSpan::new(0.0, 1.6);
        let substance = SubstanceId(1);
        let logical = world
            .spawn((
                HexTile,
                position,
                span,
                substance,
                RunBottom(0),
                Headroom(2),
            ))
            .id();
        let batch = world
            .spawn((
                TerrainRenderBatch::new(
                    TerrainChunkRoot { q: 0, r: 0 },
                    substance,
                    vec![TerrainPickRun::new(logical, position, span)],
                ),
                Mesh3d::default(),
                MeshMaterial3d::<ReviewLiquidMaterial>::default(),
                Pickable::default(),
                ViewVisibility::VISIBLE,
            ))
            .id();

        let (water, coverage) = inspect(&mut world);
        let water = water.expect("one original-water binding retains exact picking authority");
        assert!(
            matches!(
                coverage,
                Err(ReviewCaptureCoverageError::NoActiveReviewCamera)
            ),
            "water must pass renderability and reach the separate camera requirement"
        );

        let original = world
            .get::<MeshMaterial3d<ReviewLiquidMaterial>>(batch)
            .expect("the water fixture starts with its ordinary material")
            .0
            .clone();
        world
            .entity_mut(batch)
            .remove::<MeshMaterial3d<ReviewLiquidMaterial>>()
            .insert(ReviewSuppressedWaterMaterial(original.clone()));
        let (suppressed, coverage) = inspect(&mut world);
        assert_eq!(
            suppressed.expect("explicit review suppression retains the original pick authority"),
            water,
        );
        assert!(matches!(coverage, Err(ReviewCaptureCoverageError::NoActiveReviewCamera)),
            "suppressed original water remains a valid pick proxy for the separately owned review surface");
        world
            .entity_mut(batch)
            .insert(MeshMaterial3d(original.clone()));
        let (duplicate_owner, coverage) = inspect(&mut world);
        assert!(duplicate_owner
            .expect_err("live water cannot coexist with a suppression owner")
            .contains("exactly one"));
        assert!(matches!(
            coverage,
            Err(ReviewCaptureCoverageError::BoundaryBatchMissingMaterial { .. })
        ));
        world
            .entity_mut(batch)
            .remove::<ReviewSuppressedWaterMaterial>();
        assert_eq!(
            inspect(&mut world)
                .0
                .expect("restoring the original binding restores ownership"),
            water
        );

        world
            .entity_mut(batch)
            .insert(MeshMaterial3d::<StandardMaterial>::default());
        let (duplicate, coverage) = inspect(&mut world);
        assert!(duplicate
            .expect_err("two materials would render the same water twice")
            .contains("exactly one"));
        assert!(matches!(
            coverage,
            Err(ReviewCaptureCoverageError::BoundaryBatchMissingMaterial { .. })
        ));

        world
            .entity_mut(batch)
            .remove::<MeshMaterial3d<ReviewLiquidMaterial>>();
        let (standard, coverage) = inspect(&mut world);
        assert_ne!(
            water,
            standard.expect("one standard terrain binding remains valid"),
            "authority must retain the exact material kind rather than only a presence bit"
        );
        assert!(matches!(
            coverage,
            Err(ReviewCaptureCoverageError::NoActiveReviewCamera)
        ));

        world
            .entity_mut(batch)
            .remove::<MeshMaterial3d<StandardMaterial>>();
        let (missing, coverage) = inspect(&mut world);
        assert!(missing
            .expect_err("unbound terrain is still rejected")
            .contains("exactly one"));
        assert!(matches!(
            coverage,
            Err(ReviewCaptureCoverageError::BoundaryBatchMissingMaterial { .. })
        ));

        world.entity_mut(batch).insert((
            MeshMaterial3d::<ReviewLiquidMaterial>::default(),
            TerrainRenderBatch::new(
                TerrainChunkRoot { q: 0, r: 0 },
                substance,
                vec![TerrainPickRun::new(
                    logical,
                    TilePos::new(position.coord, position.level + 1),
                    span,
                )],
            ),
        ));
        let (wrong_run, coverage) = inspect(&mut world);
        assert!(wrong_run
            .expect_err("a liquid material cannot excuse a changed exact pick run")
            .contains("inconsistent"));
        assert!(matches!(
            coverage,
            Err(ReviewCaptureCoverageError::BatchRunPositionMismatch { .. })
        ));
    }

    #[test]
    fn canonical_authority_encoding_uses_explicit_presence_and_pickability_tags() {
        let mut encoder = CanonicalAuthorityEncoder::section(42);
        encoder.presence(false);
        encoder.presence(true);
        encoder.pickable(None);
        encoder.pickable(Some(Pickable::default()));

        let mut expected = AUTHORITY_FINGERPRINT_DOMAIN.to_vec();
        expected.extend_from_slice(&[42, 0, 1, 0, 1, 1, 1]);
        assert_eq!(encoder.finish(), expected);
    }

    #[test]
    fn authority_guard_names_every_canonical_section_that_changes() {
        let baseline = authority_snapshot_fixture();
        let mutations: [(&str, fn(&mut ReviewAuthoritySnapshotV1)); 9] = [
            ("current_world", |snapshot| snapshot.current_world.push(0)),
            ("replication", |snapshot| snapshot.replication.push(0)),
            ("lighting_condition", |snapshot| {
                snapshot.lighting_condition.push(0);
            }),
            ("resolved_illumination", |snapshot| {
                snapshot.illumination.push(0);
            }),
            ("faction_knowledge", |snapshot| {
                snapshot.faction_knowledge.push(0);
            }),
            ("units_and_occupancy", |snapshot| {
                snapshot.units_and_occupancy.push(0);
            }),
            ("logical_terrain", |snapshot| {
                snapshot.logical_terrain.push(0);
            }),
            ("terrain_picking", |snapshot| {
                snapshot.terrain_picking.push(0);
            }),
            ("persistence", |snapshot| snapshot.persistence.push(0)),
        ];

        assert_eq!(baseline.verify_unchanged(&baseline), Ok(()));
        for (expected_section, mutate) in mutations {
            let mut current = baseline.clone();
            mutate(&mut current);
            let error = baseline
                .verify_unchanged(&current)
                .expect_err("a changed authority section must block capture");
            assert!(error.contains(expected_section), "{error}");
            assert_ne!(baseline.fingerprint(), current.fingerprint());
        }
    }

    #[test]
    fn report_authority_subsets_bind_exact_logical_picking_and_gameplay_streams() {
        let baseline = authority_snapshot_fixture();
        let logical = baseline.logical_terrain_picking_fingerprint();
        let gameplay = baseline.gameplay_state_fingerprint();

        let mut changed = baseline.clone();
        changed.logical_terrain.push(10);
        assert_ne!(changed.logical_terrain_picking_fingerprint(), logical);
        assert_eq!(changed.gameplay_state_fingerprint(), gameplay);

        changed = baseline.clone();
        changed.terrain_picking.push(10);
        assert_ne!(changed.logical_terrain_picking_fingerprint(), logical);
        assert_eq!(changed.gameplay_state_fingerprint(), gameplay);

        changed = baseline.clone();
        changed.illumination.push(10);
        assert_eq!(changed.logical_terrain_picking_fingerprint(), logical);
        assert_ne!(changed.gameplay_state_fingerprint(), gameplay);

        changed = baseline.clone();
        changed.units_and_occupancy.push(10);
        assert_eq!(changed.logical_terrain_picking_fingerprint(), logical);
        assert_eq!(changed.gameplay_state_fingerprint(), gameplay);
    }

    #[test]
    fn post_teardown_authority_certificate_fails_when_logical_terrain_survives() {
        let mut state = ReviewCaptureState::new(review_capture_with_focus("fixture"));
        let persistence_root = review_test_directory("authority-teardown-persistence");
        let _cleanup = fs::remove_dir_all(&persistence_root);
        let campaign_store = CampaignStore::default();
        let campaign_save_status = CampaignSaveStatusProjection::default();
        let storage_paths = StoragePaths::under(&persistence_root);
        let persistence = encode_persistence_authority(
            Some(&campaign_store),
            Some(&campaign_save_status),
            Some(&storage_paths),
        )
        .expect("the missing campaigns-file fixture should encode exactly");
        state.authority_baseline = Some(authority_snapshot_fixture());
        state
            .authority_baseline
            .as_mut()
            .expect("the fixture baseline was inserted")
            .persistence = persistence;
        state.authority_validated_captures = 1;
        state.authority_pre_teardown_verified = true;

        let mut app = App::new();
        app.insert_resource(state)
            .insert_resource(WorldReplicationStateV1::default())
            .insert_resource(UnitRegistry::default())
            .insert_resource(campaign_store)
            .insert_resource(campaign_save_status)
            .insert_resource(storage_paths)
            .init_resource::<AuthorityTeardownObservation>()
            .add_systems(Update, observe_authority_teardown);
        app.update();
        assert!(app
            .world()
            .resource::<AuthorityTeardownObservation>()
            .0
            .as_ref()
            .is_some_and(Result::is_ok));

        app.world_mut().spawn(HexTile);
        app.update();
        let error = app
            .world()
            .resource::<AuthorityTeardownObservation>()
            .0
            .as_ref()
            .expect("the teardown audit should run")
            .as_ref()
            .expect_err("surviving logical terrain must fail teardown")
            .clone();
        assert!(error.contains("logical terrain"), "{error}");
        let _cleanup = fs::remove_dir_all(persistence_root);
    }

    #[test]
    fn persistence_authority_distinguishes_missing_and_exact_campaign_file_bytes() {
        let root = review_test_directory("persistence-presence");
        let _cleanup = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("the persistence fixture directory should be creatable");
        let store = CampaignStore::default();
        let status = CampaignSaveStatusProjection::default();
        let paths = StoragePaths::under(&root);
        let missing = encode_persistence_authority(Some(&store), Some(&status), Some(&paths))
            .expect("a missing campaigns file has an explicit canonical tag");
        fs::write(&paths.campaigns, b"exact campaign bytes")
            .expect("the campaigns fixture should be writable");
        let present = encode_persistence_authority(Some(&store), Some(&status), Some(&paths))
            .expect("present campaigns bytes should encode exactly");
        assert_ne!(missing, present);
        let _cleanup = fs::remove_dir_all(root);
    }

    #[test]
    fn local_sha256_matches_published_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn automated_capture_receipt_binds_runtime_and_exact_capture_plan_bytes() {
        let request = ReviewRequest::from_values(
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
        .expect("capture request parses");
        let exact_capture_plan = "exact UTF-8 capture-plan bytes".to_owned();
        let configured = ReviewRequest::with_runtime_receipt(
            request.clone(),
            Some("a".repeat(64)),
            Some("b".repeat(64)),
            Some(exact_capture_plan.clone()),
        )
        .expect("strict receipt inputs bind")
        .expect("capture request remains present");
        let receipt = configured
            .runtime_receipt
            .expect("automated capture publishes a receipt");
        assert_ne!(receipt.process_id, 0);
        assert_eq!(
            receipt.executable_sha256,
            runtime_executable_sha256().expect("running test executable hashes")
        );
        assert_eq!(
            receipt.capture_plan_sha256,
            sha256_hex(exact_capture_plan.as_bytes())
        );
        assert_eq!(
            receipt.profile_sha256,
            ReviewWorldDetailProfileV1::default()
                .profile_hash_sha256()
                .expect("control profile hashes")
        );
        assert!(receipt.validate().is_ok());

        assert!(ReviewRequest::with_runtime_receipt(
            request.clone(),
            None,
            Some("b".repeat(64)),
            Some(exact_capture_plan.clone()),
        )
        .is_err());
        assert!(ReviewRequest::with_runtime_receipt(
            request,
            Some("A".repeat(64)),
            Some("b".repeat(64)),
            Some(exact_capture_plan),
        )
        .is_err());
    }

    #[test]
    fn lifecycle_cycle_hash_matches_the_external_validator_contract() {
        let cycle = ReviewLifecycleCycleV1::from_hash_body(ReviewLifecycleCycleHashBodyV1 {
            cycle_index: 1,
            launch_nonce: "c".repeat(64),
            runtime_receipt_sha256: "d".repeat(64),
            profile_hash_sha256: "a".repeat(64),
            authority_before_sha256: "b".repeat(64),
            authority_after_sha256: "b".repeat(64),
            entities_remaining: 0,
            materials_remaining: 0,
            meshes_remaining: 0,
            fog_density_images_remaining: 0,
            target_images_remaining: 0,
            terrain_material_overrides_remaining: 0,
            liquid_visibility_overrides_remaining: 0,
            vegetation_scale_overrides_remaining: 0,
            camera_state_restored: true,
            oit_state_restored: true,
            transmission_state_restored: true,
            depth_state_restored: true,
            volumetric_state_restored: true,
            previous_cycle_sha256: "0".repeat(64),
        })
        .expect("the canonical lifecycle hash body should serialize");
        assert_eq!(
            cycle.cycle_sha256,
            "074bca522fd548a27ec64cc129030af12cddb28afb1080c33f49dfe2b0e9e16f"
        );
    }

    #[test]
    fn lifecycle_request_is_strict_canonical_and_exactly_one_hundred_cycles() {
        let request = ReviewLifecycleRequestV1 {
            version: 1,
            certificate_path: env::temp_dir().join("hex-review-lifecycle-certificate.json"),
            capture_plan_sha256: "a".repeat(64),
            source_provenance_sha256: "b".repeat(64),
            profile_matrix_sha256: "c".repeat(64),
            tested_profile_sha256: "d".repeat(64),
            cycles_requested: 100,
        };
        let canonical = serde_json::to_string(&request)
            .expect("the lifecycle request fixture should serialize");
        assert_eq!(
            ReviewLifecycleRequestV1::from_canonical_json(&canonical),
            Ok(request.clone())
        );
        assert!(ReviewLifecycleRequestV1::from_canonical_json(&format!("{canonical}\n")).is_err());

        let wrong_count = canonical.replace("\"cycles_requested\":100", "\"cycles_requested\":99");
        assert!(ReviewLifecycleRequestV1::from_canonical_json(&wrong_count).is_err());
    }

    #[test]
    fn resident_mesh_measure_counts_exact_main_world_buffer_bytes() {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::MAIN_WORLD,
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![[0.0_f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0_f32, 0.0, 1.0]; 3]);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0_f32, 0.0]; 3]);
        mesh.insert_indices(Indices::U16(vec![0, 1, 2]));

        assert_eq!(mesh_buffer_bytes(&mesh), Ok(102));
    }

    #[test]
    fn performance_window_uses_nearest_rank_p95_and_exact_texture_mips() {
        let samples = (1_u8..=60).map(f32::from).collect::<Vec<_>>();
        assert_eq!(nearest_rank_p95(&samples), Ok(57.0));

        let mut image = Image::new_uninit(
            Extent3d {
                width: 4,
                height: 4,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::MAIN_WORLD,
        );
        image.texture_descriptor.mip_level_count = 3;
        assert_eq!(image_texture_bytes(&image), Ok(84));
        assert!(REVIEW_RESIDENT_MEMORY_SCOPE.contains("Image texture mip"));
    }

    #[test]
    fn performance_window_finishes_at_the_first_ninety_frame_settle_deadline() {
        let start = performance_sampling_start_frame(SETTLE_FRAMES);
        assert_eq!(start, 31);
        assert_eq!(SETTLE_FRAMES - start + 1, 60);
        assert_eq!(performance_sampling_start_frame(1), 1);
        assert_eq!(performance_sampling_start_frame(2), 1);
    }

    #[test]
    fn performance_sample_is_reused_within_a_sequence_and_reset_on_reentry() {
        let first = ReviewCapture {
            path: PathBuf::from("first.png"),
            view: ReviewView::Default,
            camera: ReviewCamera::Map,
            focus_anchor: None,
            anchor_look_at: None,
            character_radius_scale: 1.0,
            full_cutaway: false,
            illumination_overlay: false,
            liquid_phase_seconds: Some(0.0),
            settle_frames: SETTLE_FRAMES,
        };
        let mut second = first.clone();
        second.path = PathBuf::from("second.png");
        second.settle_frames = 2;
        let sample = ReviewPerformanceSampleV1 {
            frame_time_ms: 10.0,
            resident_presentation_bytes: 1_024,
            warmup_complete: true,
        };

        let mut state = ReviewCaptureState::new_many(vec![first.clone(), second]);
        state.performance_frame_window_ms.extend([10.0; 60]);
        state.performance_resident_bytes = Some(sample.resident_presentation_bytes);
        state.performance_sample = Some(sample);
        state
            .advance_capture(Instant::now())
            .expect("the second capture remains in the sequence");

        assert_eq!(state.performance_sample, Some(sample));
        assert_eq!(state.performance_frame_window_ms.len(), 60);
        assert_eq!(
            state.performance_resident_bytes,
            Some(sample.resident_presentation_bytes)
        );
        assert_eq!(state.capture.settle_frames, 2);

        let reentered = ReviewCaptureState::new(first);
        assert_eq!(reentered.performance_sample, None);
        assert!(reentered.performance_frame_window_ms.is_empty());
        assert_eq!(reentered.performance_resident_bytes, None);
    }

    #[test]
    fn oit_restoration_equality_compares_every_setting_exactly() {
        let baseline = OrderIndependentTransparencySettings {
            sorted_fragment_max_count: 5,
            fragments_per_pixel_average: 3.5,
            alpha_threshold: 0.125,
        };
        assert!(oit_settings_equal(Some(baseline), Some(baseline)));
        assert!(!oit_settings_equal(
            Some(baseline),
            Some(OrderIndependentTransparencySettings {
                alpha_threshold: f32::from_bits(baseline.alpha_threshold.to_bits() + 1),
                ..baseline
            })
        ));
        assert!(!oit_settings_equal(Some(baseline), None));
    }

    fn capture_readback_binding_fixture() -> ReviewCaptureReadbackBindingV1 {
        ReviewCaptureReadbackBindingV1 {
            profile_hash_sha256: "0".repeat(64),
            projection_hashes: hex_map::review_world_detail::ReviewWorldDetailProjectionHashesV1 {
                terrain_plan: "0".repeat(16),
                liquid_atmosphere_plan: "0".repeat(16),
                mesh_projection: "0".repeat(16),
            },
            counts: hex_map::review_world_detail::ReviewWorldDetailCountsV1::default(),
            camera_features: ReviewCameraFeaturesV1 {
                oit: false,
                medium_transmission: false,
                depth_texture: false,
                volumetrics: false,
            },
            camera_entity: Entity::PLACEHOLDER,
            transform: Transform::default(),
            global_transform_bits: Mat4::IDENTITY.to_cols_array().map(f32::to_bits),
            orbit_focus_bits: Vec3::ZERO.to_array().map(f32::to_bits),
            orbit_radius_bits: 5.0_f32.to_bits(),
            render_target: RenderTarget::default(),
            projection: Some(Projection::Perspective(
                bevy::camera::PerspectiveProjection::default(),
            )),
            clip_from_view_bits: Mat4::IDENTITY.to_cols_array().map(f32::to_bits),
            msaa: Msaa::Sample4,
            depth_texture_usages: Camera3d::default().depth_texture_usages.0,
            transmission_steps: 0,
            transmission_quality: ScreenSpaceTransmissionQuality::Low,
            oit: None,
            volumetric_fog: None,
        }
    }

    #[test]
    fn screenshot_readback_binding_rejects_projection_count_and_camera_drift() {
        let request = capture_readback_binding_fixture();

        let mut projection_drift = request.clone();
        projection_drift.projection_hashes.mesh_projection = "1".repeat(16);
        assert!(request
            .verify_same(&projection_drift)
            .expect_err("projection drift must invalidate screenshot readback")
            .contains("projection hashes"));

        let mut count_drift = request.clone();
        count_drift.counts.total.entities = 1;
        assert!(request
            .verify_same(&count_drift)
            .expect_err("count drift must invalidate screenshot readback")
            .contains("projection counts"));

        let mut camera_drift = request.clone();
        camera_drift.transform.translation.x = 1.0;
        assert!(request
            .verify_same(&camera_drift)
            .expect_err("camera drift must invalidate screenshot readback")
            .contains("camera pose"));

        let mut lens_drift = request.clone();
        let Some(first_clip_bit) = lens_drift.clip_from_view_bits.first_mut() else {
            panic!("camera projection fixture must contain one clip-space bit");
        };
        *first_clip_bit ^= 1;
        assert!(request
            .verify_same(&lens_drift)
            .expect_err("lens drift must invalidate screenshot readback")
            .contains("camera projection"));

        let mut renderer_drift = request.clone();
        renderer_drift.camera_features.depth_texture = true;
        assert!(request
            .verify_same(&renderer_drift)
            .expect_err("renderer drift must invalidate screenshot readback")
            .contains("camera features"));
    }

    #[test]
    fn collider_free_claim_is_tied_to_the_review_renderer_dependency_boundary() {
        let game_manifest = include_str!("../Cargo.toml").to_ascii_lowercase();
        let map_manifest = include_str!("../../hex_map/Cargo.toml").to_ascii_lowercase();
        for collision_backend in ["rapier", "avian", "xpbd"] {
            assert!(!game_manifest.contains(collision_backend));
            assert!(!map_manifest.contains(collision_backend));
        }
        let renderer = include_str!("../../hex_map/src/review_world_detail_render.rs");
        assert!(!renderer.contains("Collider"));
        assert!(!REVIEW_COLLIDER_STATIC_INVARIANT.is_empty());
    }

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
    fn focus_anchor_resolves_exact_surface_without_mutating_selected_actor() {
        let destination = TilePos::new(HexCoord::from_axial(3, -2), 7);
        let span = HexSpan::new(2.4, 3.2);
        let (table, stone) = review_substance_table();
        let mut anchors = MapAnchors::new();
        anchors.insert(MapAnchorId::from("deep_chamber"), destination);

        let mut app = App::new();
        app.add_systems(PostUpdate, resolve_review_focus_anchor);
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
        assert_eq!(
            state.focus_world_target,
            Some(
                Standing {
                    pos: destination,
                    span,
                }
                .world_position()
            )
        );
        let actor = app.world().entity(actor);
        assert_eq!(
            actor.get::<StandsOn>().map(|standing| standing.0),
            Some(original)
        );
        assert_eq!(
            actor
                .get::<Transform>()
                .map(|transform| transform.translation),
            Some(original.world_position())
        );
        assert_eq!(
            actor.get::<CameraFocusTarget>().map(|focus| focus.surface),
            Some(original.pos)
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
        app.add_systems(PostUpdate, resolve_review_focus_anchor);
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
            liquid_phase_seconds: None,
            settle_frames: SETTLE_FRAMES,
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
                liquid_phase_seconds: None,
                settle_frames: SETTLE_FRAMES,
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
    fn production_capture_schedule_retains_target_and_waits_for_runtime_evidence() {
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
                liquid_phase_seconds: None,
                settle_frames: SETTLE_FRAMES,
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
            !app.world().resource::<ReviewCaptureState>().requested,
            "a capture without authority and renderer evidence must remain gated"
        );
        let mut screenshots = app.world_mut().query_filtered::<Entity, With<Screenshot>>();
        assert_eq!(screenshots.iter(app.world()).count(), 0);

        app.update();
        let mut screenshots = app.world_mut().query_filtered::<Entity, With<Screenshot>>();
        assert_eq!(
            screenshots.iter(app.world()).count(),
            0,
            "missing runtime evidence must not enqueue an asynchronous capture"
        );
        assert!(!path.exists());
        let _cleanup = fs::remove_dir_all(directory);
    }

    #[test]
    fn anchored_first_person_pose_survives_following_during_every_settle_frame() {
        fn ordinary_follow(mut cameras: Query<(&mut Transform, &mut PanOrbitCamera)>) {
            for (mut transform, mut orbit) in &mut cameras {
                transform.translation = Vec3::splat(999.0);
                orbit.focus = Vec3::splat(999.0);
            }
        }
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin));
        app.init_state::<Screen>();
        app.insert_resource(test_camera_settings());
        app.insert_resource(CameraMode::Map);
        app.insert_resource(Assets::<Image>::default());
        app.add_systems(
            PostUpdate,
            ordinary_follow.in_set(CameraSystems::FollowCharacter),
        );
        let mut capture = review_capture_with_focus("deep_chamber");
        capture.camera = ReviewCamera::FirstPerson;
        install_capture_systems(&mut app, capture);
        let target = Vec3::new(32.0, 4.0, 16.0);
        {
            let mut state = app.world_mut().resource_mut::<ReviewCaptureState>();
            state.focus_relocated = true;
            state.focus_world_target = Some(target);
        }
        let camera = app
            .world_mut()
            .spawn((
                Transform::default(),
                PanOrbitCamera::default(),
                RenderTarget::default(),
                Projection::Perspective(PerspectiveProjection::default()),
            ))
            .id();
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Gameplay);
        for _ in 0..3 {
            app.update();
            let state = app.world().resource::<ReviewCaptureState>();
            let expected = state
                .focus_pose
                .as_ref()
                .expect("anchored close view stores its exact pose");
            let actual = app.world().entity(camera);
            assert_eq!(actual.get::<Transform>(), Some(&expected.transform));
            assert_eq!(
                actual.get::<PanOrbitCamera>().expect("orbit remains").focus,
                expected.orbit_focus
            );
            assert!((expected.transform.translation - target).length() < 4.0);
        }
        assert_eq!(
            *app.world().resource::<CameraMode>(),
            CameraMode::FirstPerson
        );
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
                liquid_phase_seconds: None,
                settle_frames: SETTLE_FRAMES,
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
                liquid_phase_seconds: None,
                settle_frames: SETTLE_FRAMES,
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
    fn capture_sequence_restores_map_lens_after_first_person() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin));
        app.init_state::<Screen>();
        app.insert_resource(test_camera_settings());
        app.insert_resource(CameraMode::Map);
        app.insert_resource(Assets::<Image>::default());
        let first_person = ReviewCapture {
            path: PathBuf::from("first-person.png"),
            view: ReviewView::Default,
            camera: ReviewCamera::FirstPerson,
            focus_anchor: None,
            anchor_look_at: None,
            character_radius_scale: 1.0,
            full_cutaway: false,
            illumination_overlay: false,
            liquid_phase_seconds: None,
            settle_frames: SETTLE_FRAMES,
        };
        let map = ReviewCapture {
            path: PathBuf::from("map.png"),
            view: ReviewView::Default,
            camera: ReviewCamera::Map,
            focus_anchor: None,
            anchor_look_at: None,
            character_radius_scale: 1.0,
            full_cutaway: false,
            illumination_overlay: false,
            liquid_phase_seconds: None,
            settle_frames: SETTLE_FRAMES,
        };
        // This fixture exercises only the multi-capture lens transition. The
        // production authority guard is covered by the runtime-evidence tests
        // and deliberately fails closed when their full gameplay resources are
        // absent.
        install_capture_sequence_inner(&mut app, vec![first_person, map], false);
        let base_fov = 0.91;
        let camera = app
            .world_mut()
            .spawn((
                Transform::default(),
                PanOrbitCamera::default(),
                RenderTarget::default(),
                Projection::Perspective(bevy::camera::PerspectiveProjection {
                    fov: base_fov,
                    ..default()
                }),
            ))
            .id();
        app.world_mut().spawn((
            Transform::from_translation(Vec3::new(2.0, 3.0, 4.0)),
            CameraFocusTarget::new(TilePos::ORIGIN),
        ));
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Gameplay);
        app.update();

        let Projection::Perspective(first_projection) = app
            .world()
            .entity(camera)
            .get::<Projection>()
            .expect("review camera retains its projection")
        else {
            panic!("review camera remains perspective");
        };
        assert!(
            (first_projection.fov - test_camera_settings().first_person_fov_degrees.to_radians())
                .abs()
                < f32::EPSILON
        );

        app.world_mut()
            .resource_mut::<ReviewCaptureState>()
            .advance_capture(Instant::now())
            .expect("the map capture remains in the sequence");
        app.update();

        assert_eq!(*app.world().resource::<CameraMode>(), CameraMode::Map);
        let Projection::Perspective(map_projection) = app
            .world()
            .entity(camera)
            .get::<Projection>()
            .expect("review camera retains its projection")
        else {
            panic!("review camera remains perspective");
        };
        assert!((map_projection.fov - base_fov).abs() < f32::EPSILON);
    }

    #[test]
    fn gameplay_exit_restores_complete_camera_snapshot_and_removes_target_image() {
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
                liquid_phase_seconds: None,
                settle_frames: SETTLE_FRAMES,
            },
        );
        let original_transform =
            Transform::from_xyz(9.0, 12.0, -7.0).looking_at(Vec3::new(1.0, 2.0, 3.0), Vec3::Y);
        let original_focus = Vec3::new(1.0, 2.0, 3.0);
        let original_radius = 17.0;
        let original_target = RenderTarget::default();
        let original_fov = 0.91;
        let camera = app
            .world_mut()
            .spawn((
                original_transform,
                PanOrbitCamera {
                    focus: original_focus,
                    radius: original_radius,
                },
                original_target.clone(),
                Projection::Perspective(bevy::camera::PerspectiveProjection {
                    fov: original_fov,
                    ..default()
                }),
            ))
            .id();
        app.world_mut().spawn((
            Transform::from_translation(Vec3::new(2.0, 3.0, 4.0)),
            CameraFocusTarget::new(TilePos::ORIGIN),
        ));
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Gameplay);
        app.update();

        let temporary_target = app
            .world()
            .resource::<ReviewCaptureState>()
            .target
            .clone()
            .expect("first-person review should allocate its capture target");
        assert!(app
            .world()
            .resource::<Assets<Image>>()
            .get(&temporary_target)
            .is_some());

        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Title);
        app.update();

        let camera_entity = app.world().entity(camera);
        assert_eq!(
            camera_entity
                .get::<Transform>()
                .expect("camera keeps transform"),
            &original_transform
        );
        let orbit = camera_entity
            .get::<PanOrbitCamera>()
            .expect("camera keeps orbit state");
        assert_eq!(orbit.focus, original_focus);
        assert_eq!(orbit.radius.to_bits(), original_radius.to_bits());
        assert_eq!(
            format!(
                "{:?}",
                camera_entity
                    .get::<RenderTarget>()
                    .expect("camera keeps render target")
            ),
            format!("{original_target:?}")
        );
        let Projection::Perspective(projection) = camera_entity
            .get::<Projection>()
            .expect("camera keeps projection")
        else {
            panic!("camera remains perspective");
        };
        assert_eq!(projection.fov.to_bits(), original_fov.to_bits());
        assert_eq!(*app.world().resource::<CameraMode>(), CameraMode::Map);
        assert!(app
            .world()
            .resource::<Assets<Image>>()
            .get(&temporary_target)
            .is_none());
        let state = app.world().resource::<ReviewCaptureState>();
        assert!(state.camera_restored);
        assert!(state.target_removed);
        assert!(state.target.is_none());
    }

    #[test]
    fn renderer_features_only_mutate_the_review_camera() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(ReviewWorldDetailProfileV1 {
            water: hex_map::review_world_detail::WaterDetailV1::UniformAlpha { alpha: 0.85 },
            ..default()
        });
        app.add_systems(Update, configure_review_camera_features);

        let original_depth = Camera3d::default().depth_texture_usages;
        let original_transmission = ScreenSpaceTransmission {
            steps: 0,
            quality: ScreenSpaceTransmissionQuality::Low,
        };
        let review_camera = app
            .world_mut()
            .spawn((
                Camera3d::default(),
                Msaa::Sample4,
                original_transmission.clone(),
                PanOrbitCamera::default(),
            ))
            .id();
        let secondary_camera = app
            .world_mut()
            .spawn((
                Camera3d::default(),
                Msaa::Sample4,
                original_transmission.clone(),
            ))
            .id();

        app.update();

        let review = app.world().entity(review_camera);
        assert_eq!(review.get::<Msaa>(), Some(&Msaa::Off));
        assert!(TextureUsages::from(
            review
                .get::<Camera3d>()
                .expect("review camera keeps Camera3d")
                .depth_texture_usages
        )
        .contains(TextureUsages::TEXTURE_BINDING));
        assert!(review
            .get::<OrderIndependentTransparencySettings>()
            .is_some());
        assert!(review.get::<ReviewCameraFeatureRestore>().is_some());

        let secondary = app.world().entity(secondary_camera);
        assert_eq!(secondary.get::<Msaa>(), Some(&Msaa::Sample4));
        assert_eq!(
            secondary
                .get::<Camera3d>()
                .expect("secondary camera keeps Camera3d")
                .depth_texture_usages
                .0,
            original_depth.0
        );
        let secondary_transmission = secondary
            .get::<ScreenSpaceTransmission>()
            .expect("secondary camera keeps transmission settings");
        assert_eq!(secondary_transmission.steps, original_transmission.steps);
        assert_eq!(
            secondary_transmission.quality,
            original_transmission.quality
        );
        assert!(secondary
            .get::<OrderIndependentTransparencySettings>()
            .is_none());
        assert!(secondary.get::<ReviewCameraFeatureRestore>().is_none());
    }

    #[test]
    fn renderer_feature_restore_covers_every_marked_camera() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_systems(Update, restore_review_camera_features);

        let original_depth = Camera3d::default().depth_texture_usages;
        let original_transmission = ScreenSpaceTransmission {
            steps: 0,
            quality: ScreenSpaceTransmissionQuality::Low,
        };
        let mut cameras = Vec::new();
        for include_review_marker in [true, false] {
            let mut entity = app.world_mut().spawn((
                Camera3d {
                    depth_texture_usages: (TextureUsages::RENDER_ATTACHMENT
                        | TextureUsages::TEXTURE_BINDING)
                        .into(),
                    ..default()
                },
                Msaa::Off,
                ScreenSpaceTransmission {
                    steps: 1,
                    quality: ScreenSpaceTransmissionQuality::Medium,
                },
                OrderIndependentTransparencySettings::default(),
                VolumetricFog::default(),
                ReviewCameraFeatureRestore {
                    msaa: Msaa::Sample4,
                    depth_texture_usages: original_depth,
                    transmission: original_transmission.clone(),
                    oit: None,
                    volumetric_fog: None,
                },
            ));
            if include_review_marker {
                entity.insert(PanOrbitCamera::default());
            }
            cameras.push(entity.id());
        }

        app.update();

        for camera in cameras {
            let entity = app.world().entity(camera);
            assert_eq!(entity.get::<Msaa>(), Some(&Msaa::Sample4));
            assert_eq!(
                entity
                    .get::<Camera3d>()
                    .expect("restored camera keeps Camera3d")
                    .depth_texture_usages
                    .0,
                original_depth.0
            );
            let transmission = entity
                .get::<ScreenSpaceTransmission>()
                .expect("restored camera keeps transmission settings");
            assert_eq!(transmission.steps, original_transmission.steps);
            assert_eq!(transmission.quality, original_transmission.quality);
            assert!(entity
                .get::<OrderIndependentTransparencySettings>()
                .is_none());
            assert!(entity.get::<VolumetricFog>().is_none());
            assert!(entity.get::<ReviewCameraFeatureRestore>().is_none());
        }
    }

    #[test]
    fn renderer_camera_features_restore_over_one_hundred_exit_cycles() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin));
        app.init_state::<Screen>();
        app.add_systems(OnExit(Screen::Gameplay), restore_review_camera_features);

        let original_depth = Camera3d::default().depth_texture_usages;
        let original_transmission = ScreenSpaceTransmission {
            steps: 0,
            quality: ScreenSpaceTransmissionQuality::Low,
        };
        let camera = app
            .world_mut()
            .spawn((
                Camera3d::default(),
                Msaa::Sample4,
                original_transmission.clone(),
            ))
            .id();
        let light = app.world_mut().spawn(DirectionalLight::default()).id();

        for cycle in 0..100 {
            app.world_mut()
                .resource_mut::<NextState<Screen>>()
                .set(Screen::Gameplay);
            app.update();

            {
                let mut camera_entity = app.world_mut().entity_mut(camera);
                camera_entity
                    .get_mut::<Camera3d>()
                    .expect("camera fixture keeps Camera3d")
                    .depth_texture_usages =
                    (TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING).into();
                *camera_entity
                    .get_mut::<Msaa>()
                    .expect("camera fixture keeps MSAA") = Msaa::Off;
                let mut transmission = camera_entity
                    .get_mut::<ScreenSpaceTransmission>()
                    .expect("camera fixture keeps transmission settings");
                transmission.steps = 1;
                transmission.quality = ScreenSpaceTransmissionQuality::Medium;
                camera_entity.insert((
                    OrderIndependentTransparencySettings::default(),
                    VolumetricFog::default(),
                    ReviewCameraFeatureRestore {
                        msaa: Msaa::Sample4,
                        depth_texture_usages: original_depth,
                        transmission: original_transmission.clone(),
                        oit: None,
                        volumetric_fog: None,
                    },
                ));
            }
            app.world_mut()
                .entity_mut(light)
                .insert((VolumetricLight, ReviewAddedVolumetricLight));

            app.world_mut()
                .resource_mut::<NextState<Screen>>()
                .set(Screen::Title);
            app.update();

            let camera_entity = app.world().entity(camera);
            assert_eq!(camera_entity.get::<Msaa>(), Some(&Msaa::Sample4));
            assert_eq!(
                camera_entity
                    .get::<Camera3d>()
                    .expect("camera fixture keeps Camera3d")
                    .depth_texture_usages
                    .0,
                original_depth.0,
                "cycle {cycle} did not restore depth usage"
            );
            let transmission = camera_entity
                .get::<ScreenSpaceTransmission>()
                .expect("camera fixture keeps transmission settings");
            assert_eq!(transmission.steps, original_transmission.steps);
            assert_eq!(transmission.quality, original_transmission.quality);
            assert!(camera_entity
                .get::<OrderIndependentTransparencySettings>()
                .is_none());
            assert!(camera_entity.get::<VolumetricFog>().is_none());
            assert!(camera_entity.get::<ReviewCameraFeatureRestore>().is_none());
            assert!(app.world().get::<VolumetricLight>(light).is_none());
            assert!(app
                .world()
                .get::<ReviewAddedVolumetricLight>(light)
                .is_none());
        }
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
            liquid_phase_seconds: None,
            settle_frames: SETTLE_FRAMES,
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
            !state.requested,
            "runtime evidence still gates the close-up"
        );
        assert_eq!(state.visible_tiles, MIN_VISIBLE_TILES);
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
            liquid_phase_seconds: None,
            settle_frames: SETTLE_FRAMES,
        }
    }

    #[test]
    fn oit_operational_evidence_fails_closed_without_device_capabilities() {
        assert!(!operational_oit_available(false, None, None));
        assert!(!operational_oit_available(true, None, None));
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
