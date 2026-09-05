//! Bevy adapter for disposable world-detail review projections.
//!
//! This module is compiled only by `hex_map/map-review`. It translates the two
//! renderer-neutral planners into chunk-batched, collider-free, non-pickable
//! entities and records enough original state to restore the ordinary renderer.

use std::collections::{BTreeMap, BTreeSet};

use bevy::asset::RenderAssetUsages;
use bevy::ecs::system::SystemParam;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::light::{FogVolume, NotShadowCaster};
use bevy::material::OpaqueRendererMethod;
use bevy::mesh::Indices;
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::picking::Pickable;
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, Face, PrimitiveTopology, ShaderType, TextureDimension, TextureFormat,
};
use bevy::shader::ShaderRef;
use hex_assets::{RuntimeArtCatalog, SubstanceTable};
use hex_core::{
    BiomeRegions, HexCoord, HexGrid, InteriorRegions, MapAnchors, MapObservationAnchors,
    ResolvedMapSeed, SubstanceId, TerrainReady, TerrainRenderBatch, TilePos, TraversalBlockers,
};
use xxhash_rust::xxh3::xxh3_64;

use crate::feature_render::GeneratedFeatureRoot;
use crate::liquid_render::{LiquidMaterial, LiquidVisualTime, ReviewLiquidPresentationRole};
use crate::procedural_v3::{
    FeatureKind, FillMaterialRole, LiquidFlowState, MapPresentationProjection,
};
use crate::review_world_detail::{
    ReviewAnchorClassV1, ReviewAuthorityFingerprintsV1, ReviewCameraFeaturesV1,
    ReviewCleanupStateV1, ReviewPerformanceSampleV1, ReviewPresentationCountsV1,
    ReviewRuntimeReceiptV1, ReviewWorldDetailCountsV1, ReviewWorldDetailEffectValidationV1,
    ReviewWorldDetailProfileV1, ReviewWorldDetailProjectionHashesV1, ReviewWorldDetailReportV1,
    ReviewWorldDetailRuntimeAssetEvidenceV1, ReviewWorldDetailTeardownReceiptV1,
    ReviewWorldDetailTeardownRequestV1, REVIEW_WORLD_DETAIL_REPORT_VERSION_V1,
};
use crate::review_world_detail_effects::{
    build_liquid_atmosphere_review_plan, fog_density_xz_mask, LiquidAtmosphereReviewInputV1,
    LiquidAtmosphereReviewPlanV1, ReviewAlphaModeV1, ReviewChunkKeyV1, ReviewEffectAnchorKindV1,
    ReviewEffectAnchorV1, ReviewFogVolumeV1, ReviewIndexedMeshV1, ReviewLiquidCellV1,
    ReviewLiquidFlowV1, ReviewLiquidKindV1, ReviewMaterialDescriptorV1, ReviewMaterialKeyV1,
    ReviewMeshLayerV1, ReviewPeakSolidSpanV1, ReviewPhysicalSolidRunV1, ReviewShoreSurfaceV1,
    ReviewWaterMaterialStyleV1, FOG_DENSITY_DEPTH, FOG_DENSITY_WIDTH,
};
use crate::review_world_detail_terrain::{
    plan_review_terrain_details, ReviewCliffLayerInputV1, ReviewPropExclusionsV1,
    ReviewSnowExceptionV1, ReviewTerrainInputBuilderV1, ReviewTerrainMaterialRoleV1,
    ReviewTerrainMeshBatchV1, ReviewTerrainSideInputV1, ReviewTerrainSurfaceInputV1,
    ReviewVegetationProjectionV1,
};
use crate::settings::MapSettings;
use crate::voxel::{runs, terrain_chunk_key, Column, VoxelMap};
use crate::GenerationReport;

const REVIEW_WARNING: &str = "UNAPPROVABLE STRUCTURAL DRAFT — AESTHETIC REVIEW ONLY";
const REVIEW_SURFACE_BIAS: f32 = 0.006;
const REVIEW_WATER_SHADER_PATH: &str = "shaders/review_world_detail_water.wgsl";
const REVIEW_WATER_PHASE_WRAP_SECONDS: f32 = 400.0;
const REVIEW_MAX_REFRACTION_UV: f32 = 0.015;
const REVIEW_WATER_MATERIAL_LIMIT: usize = 2;
const REVIEW_WATER_SURFACE_ROUGHNESS: f32 = 0.60;
const REVIEW_WATER_SURFACE_REFLECTANCE: f32 = 0.35;
const REVIEW_WATER_FALL_ROUGHNESS: f32 = 0.28;
const REVIEW_WATER_FALL_REFLECTANCE: f32 = 0.72;
const REVIEW_WATER_FALL_FLOW_SPEED: f32 = 0.85;
const REVIEW_FOG_ABSORPTION: f32 = 0.40;
const REVIEW_FOG_SCATTERING: f32 = 0.60;

/// Review-only uniforms shared by every chunk using one water surface/fall style.
///
/// The first four vectors intentionally mirror the shipped liquid shader's
/// surface/fall parameters. `refraction.x` is the W06-only maximum projected
/// screen-UV displacement; its remaining lanes are reserved and zero.
#[derive(Clone, Copy, Debug, Default, Reflect, ShaderType)]
struct ReviewWaterMaterialParams {
    flow_phase_scale: Vec4,
    modulation: Vec4,
    emission: Vec4,
    foam_color: Vec4,
    refraction: Vec4,
}

/// PBR extension used solely by disposable review water caps and curtains.
#[derive(Asset, AsBindGroup, Clone, Debug, Default, Reflect)]
struct ReviewWaterExtension {
    #[uniform(100)]
    params: ReviewWaterMaterialParams,
}

impl MaterialExtension for ReviewWaterExtension {
    fn fragment_shader() -> ShaderRef {
        REVIEW_WATER_SHADER_PATH.into()
    }
}

type ReviewWaterMaterial = ExtendedMaterial<StandardMaterial, ReviewWaterExtension>;
type ReviewReportBuildResult = Result<
    (
        Option<ReviewWorldDetailReportV1>,
        ReviewWorldDetailProjectionHashesV1,
    ),
    String,
>;

pub(crate) fn plugin(app: &mut App) {
    app.add_plugins(MaterialPlugin::<ReviewWaterMaterial>::default())
        .add_systems(
            Update,
            (
                apply_review_world_detail,
                apply_review_vegetation_scales,
                update_review_water_material_phase,
            )
                .chain()
                .after(hex_core::PausableSystems)
                .run_if(resource_exists::<ReviewWorldDetailProfileV1>)
                .run_if(in_state(hex_core::Screen::Gameplay)),
        )
        .add_systems(
            Update,
            (
                restore_review_world_detail
                    .run_if(resource_exists::<ReviewWorldDetailTeardownRequestV1>)
                    .in_set(ReviewWorldDetailLifecycleSystems::Restore),
                publish_review_world_detail_teardown_receipt
                    .run_if(resource_exists::<ReviewWorldDetailTeardownTargets>)
                    .in_set(ReviewWorldDetailLifecycleSystems::Verify),
            )
                .chain()
                .after(update_review_water_material_phase)
                .run_if(in_state(hex_core::Screen::Gameplay)),
        )
        .add_systems(
            OnExit(hex_core::Screen::Gameplay),
            (
                restore_review_world_detail.in_set(ReviewWorldDetailLifecycleSystems::Restore),
                publish_review_world_detail_teardown_receipt
                    .in_set(ReviewWorldDetailLifecycleSystems::Verify),
            )
                .chain()
                .run_if(resource_exists::<ReviewWorldDetailProfileV1>)
                .before(crate::grid::MapLifecycleSystems::Teardown),
        );
}

/// Marks every disposable ECS entity owned solely by this review projection.
#[derive(Component, Debug)]
pub struct ReviewWorldDetailEntity;

/// Ordered stages of review-projection teardown and live zero-count proof.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReviewWorldDetailLifecycleSystems {
    /// Restore ordinary presentation and remove disposable projection assets.
    Restore,
    /// Query the flushed world and publish an exact teardown receipt.
    Verify,
}

/// Marker applied to review fog instances for exact cleanup/count queries.
#[derive(Component, Debug)]
pub(crate) struct ReviewWorldDetailFog;

/// Complete renderer mutations that must be unwound on review teardown.
#[derive(Resource, Debug, Default)]
struct ReviewWorldDetailProjectionState {
    entities: Vec<Entity>,
    meshes: Vec<Handle<Mesh>>,
    images: Vec<Handle<Image>>,
    materials: Vec<Handle<StandardMaterial>>,
    review_water_materials: Vec<Handle<ReviewWaterMaterial>>,
    suppressed_terrain: BTreeMap<Entity, Handle<StandardMaterial>>,
    suppressed_liquids: BTreeMap<Entity, Handle<LiquidMaterial>>,
    vegetation_treatments: BTreeMap<u32, Vec3>,
    vegetation_original_scales: BTreeMap<Entity, Vec3>,
    effects_phase_neutral_hash: u64,
}

/// Asset identities retained just long enough to verify deferred teardown.
#[derive(Resource, Debug, Default)]
struct ReviewWorldDetailTeardownTargets {
    materials: Vec<Handle<StandardMaterial>>,
    review_water_materials: Vec<Handle<ReviewWaterMaterial>>,
    meshes: Vec<Handle<Mesh>>,
    images: Vec<Handle<Image>>,
    suppressed_terrain: BTreeMap<Entity, Handle<StandardMaterial>>,
    suppressed_liquids: BTreeMap<Entity, Handle<LiquidMaterial>>,
    vegetation_original_scales: BTreeMap<Entity, Vec3>,
}

/// Immutable snapshot of every ordinary water-render target that a successful
/// review commit must suppress. Resolving this before any review asset is added
/// prevents an unavailable or incomplete ordinary presentation from leaving a
/// half-applied transparent-water treatment.
#[derive(Debug)]
struct ReviewWaterSuppressionPlan {
    terrain: Vec<(Entity, Handle<StandardMaterial>)>,
    liquids: Vec<(Entity, Handle<LiquidMaterial>)>,
    water_batches: usize,
}

impl ReviewWaterSuppressionPlan {
    fn is_complete(&self) -> bool {
        self.terrain.is_empty()
            && self.water_batches > 0
            && self.liquids.len() == self.water_batches
    }

    fn suppress(self, commands: &mut Commands, state: &mut ReviewWorldDetailProjectionState) {
        for (entity, original) in self.liquids {
            state.suppressed_liquids.insert(entity, original);
            // Keep the original mesh visible to picking while removing only its
            // render binding. The review surface is the sole visible water path.
            commands
                .entity(entity)
                .remove::<MeshMaterial3d<LiquidMaterial>>();
        }
    }
}

impl ReviewWorldDetailProjectionState {
    fn material_count(&self) -> usize {
        self.materials
            .len()
            .saturating_add(self.review_water_materials.len())
    }
}

#[derive(Debug, Clone)]
struct NaturalSurface {
    position: TilePos,
    run_bottom: i32,
    solid_stack_bottom: i32,
    substance: SubstanceId,
    current_snow: bool,
    exception: ReviewSnowExceptionV1,
    forced_summit: bool,
    excluded: ReviewPropExclusionsV1,
}

#[derive(SystemParam)]
struct ReviewWorldDetailSources<'w> {
    profile: Option<Res<'w, ReviewWorldDetailProfileV1>>,
    runtime_receipt: Option<Res<'w, ReviewRuntimeReceiptV1>>,
    teardown_request: Option<Res<'w, ReviewWorldDetailTeardownRequestV1>>,
    ready: Option<Res<'w, TerrainReady>>,
    map: Option<Res<'w, VoxelMap>>,
    table: Option<Res<'w, SubstanceTable>>,
    settings: Option<Res<'w, MapSettings>>,
    seed: Option<Res<'w, ResolvedMapSeed>>,
    liquid_visual_time: Option<Res<'w, LiquidVisualTime>>,
    presentation: Option<Res<'w, MapPresentationProjection>>,
    anchors: Option<Res<'w, MapAnchors>>,
    observation_anchors: Option<Res<'w, MapObservationAnchors>>,
    interiors: Option<Res<'w, InteriorRegions>>,
    blockers: Option<Res<'w, TraversalBlockers>>,
    biomes: Option<Res<'w, BiomeRegions>>,
    generation: Option<Res<'w, GenerationReport>>,
    art_catalog: Option<Res<'w, RuntimeArtCatalog>>,
}

#[derive(SystemParam)]
struct ReviewWorldDetailAdapter<'w, 's> {
    grids: Query<'w, 's, Entity, With<HexGrid>>,
    terrain_batches: Query<
        'w,
        's,
        (
            Entity,
            &'static TerrainRenderBatch,
            &'static MeshMaterial3d<StandardMaterial>,
        ),
    >,
    liquid_presentations: Query<
        'w,
        's,
        (
            Entity,
            &'static TerrainRenderBatch,
            Option<&'static ReviewLiquidPresentationRole>,
            Option<&'static MeshMaterial3d<LiquidMaterial>>,
        ),
    >,
    liquid_material_bindings: Query<'w, 's, &'static MeshMaterial3d<LiquidMaterial>>,
    materials: ResMut<'w, Assets<StandardMaterial>>,
    liquid_materials: Res<'w, Assets<LiquidMaterial>>,
    review_water_materials: ResMut<'w, Assets<ReviewWaterMaterial>>,
    meshes: ResMut<'w, Assets<Mesh>>,
    images: ResMut<'w, Assets<Image>>,
    existing: Option<Res<'w, ReviewWorldDetailProjectionState>>,
}

/// Builds the review layer after ordinary terrain publication has completed.
fn apply_review_world_detail(
    mut commands: Commands,
    sources: ReviewWorldDetailSources,
    adapter: ReviewWorldDetailAdapter,
) {
    let ReviewWorldDetailSources {
        profile,
        runtime_receipt,
        teardown_request,
        ready,
        map,
        table,
        settings,
        seed,
        liquid_visual_time,
        presentation,
        anchors,
        observation_anchors,
        interiors,
        blockers,
        biomes,
        generation,
        art_catalog,
    } = sources;
    let ReviewWorldDetailAdapter {
        grids,
        terrain_batches,
        liquid_presentations,
        liquid_material_bindings,
        mut materials,
        liquid_materials,
        mut review_water_materials,
        mut meshes,
        mut images,
        existing,
    } = adapter;
    if profile.is_none() || teardown_request.is_some() || ready.is_none() || existing.is_some() {
        return;
    }
    commands.remove_resource::<ReviewWorldDetailTeardownReceiptV1>();
    commands.remove_resource::<ReviewWorldDetailTeardownTargets>();
    let (Some(profile), Some(map), Some(table), Some(settings), Some(seed), Some(anchors)) =
        (profile, map, table, settings, seed, anchors)
    else {
        return;
    };
    let Ok(grid) = grids.single() else {
        error!("world-detail review requires exactly one HexGrid");
        return;
    };
    let observation_anchors = observation_anchors.as_deref();
    let presentation = presentation.as_deref();
    let interiors = interiors.as_deref();
    let blockers = blockers.as_deref();
    let biomes = biomes.as_deref();

    let water_suppression = if profile.water.is_current() {
        None
    } else {
        let Some(water) = table.id("water") else {
            fail_review_world_detail(
                &mut commands,
                "water treatment requires the ordinary water substance",
            );
            return;
        };
        let terrain = (&terrain_batches)
            .into_iter()
            .filter(|(_, batch, _)| batch.substance() == water)
            .map(|(entity, _, material)| (entity, material.0.clone()))
            .collect::<Vec<_>>();
        let water_batches = liquid_presentations
            .iter()
            .filter(|(_, batch, _, _)| batch.substance() == water)
            .collect::<Vec<_>>();
        let liquids = water_batches
            .iter()
            .filter_map(|(entity, _, role, material)| {
                role.filter(|role| role.0 == FillMaterialRole::Water)
                    .and_then(|_| material.map(|material| (*entity, material.0.clone())))
            })
            .collect::<Vec<_>>();
        let plan = ReviewWaterSuppressionPlan {
            terrain,
            liquids,
            water_batches: water_batches.len(),
        };
        if !plan.is_complete() {
            fail_review_world_detail(
                &mut commands,
                &format!(
                    "water treatment requires exactly one extended-material binding per original water batch and no duplicate standard-material water (found {} standard, {} extended of {} water batches)",
                    plan.terrain.len(),
                    plan.liquids.len(),
                    plan.water_batches,
                ),
            );
            return;
        }
        Some(plan)
    };

    let built = build_review_projection(
        &profile,
        &map,
        &table,
        &settings,
        seed.0,
        liquid_visual_time
            .as_deref()
            .map_or(0.0, LiquidVisualTime::phase_seconds),
        presentation,
        &anchors,
        observation_anchors,
        interiors,
        blockers,
        biomes,
        generation.as_deref(),
        art_catalog.as_deref(),
        runtime_receipt.as_deref(),
        &mut materials,
        &mut review_water_materials,
        &mut meshes,
        &mut images,
        &mut commands,
    );
    let (mut state, report, hashes) = match built {
        Ok(built) => built,
        Err(error) => {
            fail_review_world_detail(&mut commands, &error);
            return;
        }
    };

    let live_liquid_materials = liquid_material_bindings
        .iter()
        .map(|binding| binding.0.clone())
        .collect::<Vec<_>>();
    let runtime_asset_evidence = match review_runtime_asset_evidence(
        &state,
        &live_liquid_materials,
        &liquid_materials,
        &review_water_materials,
        &images,
    ) {
        Ok(evidence) => evidence,
        Err(error) => {
            rollback_pending_review_projection(
                &state,
                &mut materials,
                &mut review_water_materials,
                &mut meshes,
                &mut images,
                &mut commands,
            );
            fail_review_world_detail(&mut commands, &error);
            return;
        }
    };

    if let Some(water_suppression) = water_suppression {
        water_suppression.suppress(&mut commands, &mut state);
    }

    if !state.entities.is_empty() {
        commands.entity(grid).add_children(&state.entities);
    }
    info!(
        "applied world-detail review profile {:?}: {} entities, {} materials, terrain_plan={}, effects_plan={}, mesh_projection={}",
        profile.active_treatment_ids(),
        state.entities.len(),
        state.material_count(),
        hashes.terrain_plan,
        hashes.liquid_atmosphere_plan,
        hashes.mesh_projection,
    );
    info!("{REVIEW_WARNING}");
    commands.insert_resource(state);
    if let Some(report) = report {
        commands.insert_resource(report);
    }
    commands.insert_resource(hashes);
    commands.insert_resource(runtime_asset_evidence);
}

fn fail_review_world_detail(commands: &mut Commands, error: &str) {
    error!("world-detail review projection failed closed: {error}");
    commands.remove_resource::<ReviewWorldDetailRuntimeAssetEvidenceV1>();
    commands.remove_resource::<ReviewWorldDetailTeardownTargets>();
    commands.insert_resource(hex_core::GameplaySetupFailure::new(format!(
        "The world-detail review projection is invalid: {error}"
    )));
}

/// Applies deterministic jitter only to the existing render roots. No logical root,
/// blocker, object count, or authored object data changes.
fn apply_review_vegetation_scales(
    state: Option<ResMut<ReviewWorldDetailProjectionState>>,
    roots: Query<(&GeneratedFeatureRoot, &Children)>,
    mut render_children: Query<&mut Transform>,
) {
    let Some(mut state) = state else {
        return;
    };
    for (root, children) in &roots {
        let Some(treatment) = state.vegetation_treatments.get(&root.id.0).copied() else {
            continue;
        };
        for child in children.iter() {
            let Ok(mut transform) = render_children.get_mut(child) else {
                continue;
            };
            let original = *state
                .vegetation_original_scales
                .entry(child)
                .or_insert(transform.scale);
            transform.scale = original * treatment;
        }
    }
}

/// Updates only the bounded shared review-water uniforms. Meshes, entities, and
/// material handles remain unchanged while motion captures advance their phase.
fn update_review_water_material_phase(
    visual_time: Res<LiquidVisualTime>,
    state: Option<Res<ReviewWorldDetailProjectionState>>,
    mut materials: ResMut<Assets<ReviewWaterMaterial>>,
    mut report: Option<ResMut<ReviewWorldDetailReportV1>>,
    mut hashes: Option<ResMut<ReviewWorldDetailProjectionHashesV1>>,
) {
    let Some(state) = state else {
        return;
    };
    debug_assert!(state.review_water_materials.len() <= REVIEW_WATER_MATERIAL_LIMIT);
    let phase = bounded_review_water_phase(visual_time.phase_seconds());
    for handle in &state.review_water_materials {
        if let Some(mut material) = materials.get_mut(handle) {
            set_review_water_material_phase(&mut material, phase);
        }
    }
    let Some(phase_bound_hash) =
        phase_bound_effect_plan_hash(state.effects_phase_neutral_hash, phase)
    else {
        error!("bounded review-water phase unexpectedly became non-finite");
        return;
    };
    if let Some(hashes) = hashes.as_deref_mut() {
        hashes.liquid_atmosphere_plan.clone_from(&phase_bound_hash);
    }
    if let Some(report) = report.as_deref_mut() {
        report
            .projection_hashes
            .liquid_atmosphere_plan
            .clone_from(&phase_bound_hash);
    }
}

/// Restores every ordinary renderer handle and explicitly removes review
/// assets before the map itself is torn down.
fn restore_review_world_detail(
    mut commands: Commands,
    state: Option<Res<ReviewWorldDetailProjectionState>>,
    mut terrain_batches: Query<&mut MeshMaterial3d<StandardMaterial>>,
    mut render_children: Query<&mut Transform>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut review_water_materials: ResMut<Assets<ReviewWaterMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
) {
    commands.remove_resource::<ReviewWorldDetailTeardownRequestV1>();
    commands.remove_resource::<ReviewWorldDetailTeardownReceiptV1>();
    commands.insert_resource(state.as_deref().map_or_else(
        ReviewWorldDetailTeardownTargets::default,
        |state| ReviewWorldDetailTeardownTargets {
            materials: state.materials.clone(),
            review_water_materials: state.review_water_materials.clone(),
            meshes: state.meshes.clone(),
            images: state.images.clone(),
            suppressed_terrain: state.suppressed_terrain.clone(),
            suppressed_liquids: state.suppressed_liquids.clone(),
            vegetation_original_scales: state.vegetation_original_scales.clone(),
        },
    ));
    let Some(state) = state else {
        commands.remove_resource::<ReviewWorldDetailReportV1>();
        commands.remove_resource::<ReviewWorldDetailProjectionHashesV1>();
        commands.remove_resource::<ReviewWorldDetailRuntimeAssetEvidenceV1>();
        return;
    };
    for (entity, original) in &state.suppressed_terrain {
        if let Ok(mut current) = terrain_batches.get_mut(*entity) {
            current.0 = original.clone();
        }
    }
    for (entity, original) in &state.suppressed_liquids {
        commands
            .entity(*entity)
            .try_insert(MeshMaterial3d(original.clone()));
    }
    for (entity, original) in &state.vegetation_original_scales {
        if let Ok(mut transform) = render_children.get_mut(*entity) {
            transform.scale = *original;
        }
    }
    for entity in &state.entities {
        commands.entity(*entity).try_despawn();
    }
    for handle in &state.meshes {
        meshes.remove(handle.id());
    }
    for handle in &state.materials {
        materials.remove(handle.id());
    }
    for handle in &state.review_water_materials {
        review_water_materials.remove(handle.id());
    }
    for handle in &state.images {
        images.remove(handle.id());
    }
    commands.remove_resource::<ReviewWorldDetailProjectionState>();
    commands.remove_resource::<ReviewWorldDetailReportV1>();
    commands.remove_resource::<ReviewWorldDetailProjectionHashesV1>();
    commands.remove_resource::<ReviewWorldDetailRuntimeAssetEvidenceV1>();
}

/// Runs after `restore_review_world_detail` and its deferred despawns, but before
/// ordinary map teardown, so every zero is verified against the live world and
/// asset stores rather than inferred from bookkeeping.
fn publish_review_world_detail_teardown_receipt(
    mut commands: Commands,
    targets: Option<Res<ReviewWorldDetailTeardownTargets>>,
    entities: Query<Entity, With<ReviewWorldDetailEntity>>,
    materials: Res<Assets<StandardMaterial>>,
    review_water_materials: Res<Assets<ReviewWaterMaterial>>,
    meshes: Res<Assets<Mesh>>,
    images: Res<Assets<Image>>,
    terrain_batches: Query<&MeshMaterial3d<StandardMaterial>>,
    liquid_presentations: Query<
        &MeshMaterial3d<LiquidMaterial>,
        With<ReviewLiquidPresentationRole>,
    >,
    render_children: Query<&Transform>,
) {
    let Some(targets) = targets else {
        return;
    };
    let terrain_material_overrides_remaining = targets
        .suppressed_terrain
        .iter()
        .filter(|(entity, original)| {
            terrain_batches
                .get(**entity)
                .map_or(true, |current| &current.0 != *original)
        })
        .count();
    let liquid_visibility_overrides_remaining = targets
        .suppressed_liquids
        .iter()
        .filter(|(entity, original)| {
            liquid_presentations
                .get(**entity)
                .map_or(true, |current| &current.0 != *original)
        })
        .count();
    let vegetation_scale_overrides_remaining = targets
        .vegetation_original_scales
        .iter()
        .filter(|(entity, original)| {
            render_children
                .get(**entity)
                .map_or(true, |current| &current.scale != *original)
        })
        .count();
    let receipt = ReviewWorldDetailTeardownReceiptV1 {
        review_entities_remaining: bounded_u64(entities.iter().count()),
        standard_materials_remaining: live_owned_asset_count(&targets.materials, &materials),
        meshes_remaining: live_owned_asset_count(&targets.meshes, &meshes),
        review_water_materials_remaining: live_owned_asset_count(
            &targets.review_water_materials,
            &review_water_materials,
        ),
        fog_density_images_remaining: live_owned_asset_count(&targets.images, &images),
        terrain_material_overrides_remaining: bounded_u64(terrain_material_overrides_remaining),
        liquid_visibility_overrides_remaining: bounded_u64(liquid_visibility_overrides_remaining),
        vegetation_scale_overrides_remaining: bounded_u64(vegetation_scale_overrides_remaining),
    };
    commands.remove_resource::<ReviewWorldDetailTeardownTargets>();
    commands.insert_resource(receipt);
}

fn live_owned_asset_count<A: Asset>(handles: &[Handle<A>], assets: &Assets<A>) -> u64 {
    let ids = handles.iter().map(Handle::id).collect::<BTreeSet<_>>();
    bounded_u64(
        ids.into_iter()
            .filter(|id| assets.get(*id).is_some())
            .count(),
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "pure adapter input mirrors the explicit world-authority boundary"
)]
fn build_review_projection(
    profile: &ReviewWorldDetailProfileV1,
    map: &VoxelMap,
    table: &SubstanceTable,
    settings: &MapSettings,
    seed: u64,
    liquid_phase_seconds: f32,
    presentation: Option<&MapPresentationProjection>,
    anchors: &MapAnchors,
    observation_anchors: Option<&MapObservationAnchors>,
    interiors: Option<&InteriorRegions>,
    blockers: Option<&TraversalBlockers>,
    biomes: Option<&BiomeRegions>,
    generation: Option<&GenerationReport>,
    art_catalog: Option<&RuntimeArtCatalog>,
    runtime_receipt: Option<&ReviewRuntimeReceiptV1>,
    materials: &mut Assets<StandardMaterial>,
    review_water_materials: &mut Assets<ReviewWaterMaterial>,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    commands: &mut Commands,
) -> Result<
    (
        ReviewWorldDetailProjectionState,
        Option<ReviewWorldDetailReportV1>,
        ReviewWorldDetailProjectionHashesV1,
    ),
    String,
> {
    profile.validate().map_err(|error| error.to_string())?;
    let profile_hash_sha256 = profile
        .profile_hash_sha256()
        .map_err(|error| error.to_string())?;
    if let Some(receipt) = runtime_receipt {
        receipt.validate().map_err(|error| error.to_string())?;
        if receipt.profile_sha256 != profile_hash_sha256 {
            return Err(
                "runtime receipt profile_sha256 does not match the resolved review profile"
                    .to_owned(),
            );
        }
    }
    let (anchor_heights, anchor_classes, anchor_positions) =
        collect_anchor_evidence(anchors, observation_anchors, settings.level_height)?;
    let build_report = |hashes: &ReviewWorldDetailProjectionHashesV1,
                        effect_validation: &ReviewWorldDetailEffectValidationV1,
                        counts: ReviewWorldDetailCountsV1,
                        state: &ReviewWorldDetailProjectionState|
     -> Result<Option<ReviewWorldDetailReportV1>, String> {
        let Some(runtime_receipt) = runtime_receipt else {
            return Ok(None);
        };
        let authority = authority_fingerprints(
            map,
            table,
            presentation,
            anchors,
            observation_anchors,
            blockers,
            biomes,
            generation,
            settings.level_height,
        )?;
        let report = ReviewWorldDetailReportV1 {
            version: REVIEW_WORLD_DETAIL_REPORT_VERSION_V1,
            runtime_receipt: runtime_receipt.clone(),
            profile_hash_sha256: profile_hash_sha256.clone(),
            authority,
            projection_hashes: hashes.clone(),
            effect_validation: effect_validation.clone(),
            counts,
            anchor_heights: anchor_heights.clone(),
            anchor_classes: anchor_classes.clone(),
            camera_features: ReviewCameraFeaturesV1 {
                oit: false,
                medium_transmission: false,
                depth_texture: false,
                volumetrics: false,
            },
            performance: ReviewPerformanceSampleV1::default(),
            cleanup: ReviewCleanupStateV1 {
                completed_cycles: 0,
                entities_remaining: u64::try_from(state.entities.len()).unwrap_or(u64::MAX),
                materials_remaining: u64::try_from(state.material_count()).unwrap_or(u64::MAX),
                meshes_remaining: u64::try_from(state.meshes.len()).unwrap_or(u64::MAX),
                target_images_remaining: 0,
                camera_state_restored: false,
                oit_state_restored: !profile.requires_oit(),
                transmission_state_restored: !profile.requires_transmission(),
                depth_state_restored: !(profile.requires_oit() || profile.requires_transmission()),
                volumetric_state_restored: !profile.requires_volumetrics(),
            },
        };
        report.validate().map_err(|error| error.to_string())?;
        Ok(Some(report))
    };

    // The shared control is a true no-op projection. It deliberately avoids
    // inspecting natural surfaces, Grand-only effect anchors, or effect palette
    // swatches, so the default profile remains reusable on every valid map.
    if profile.is_current() {
        let terrain_input = ReviewTerrainInputBuilderV1::new(seed, settings.level_height)
            .map_err(|error| error.to_string())?
            .build();
        let terrain_plan = plan_review_terrain_details(profile, &terrain_input)
            .map_err(|error| error.to_string())?;
        let effects_input = LiquidAtmosphereReviewInputV1 {
            seed,
            level_height: settings.level_height,
            phase_seconds: liquid_phase_seconds,
            max_exposed_natural_y: 0.0,
            massif_crest: Vec3::ZERO,
            interaction_peak: Vec3::ZERO,
            interaction_peak_solid_spans: vec![ReviewPeakSolidSpanV1 {
                bottom_y: -1.0,
                top_y: 0.0,
            }],
            cloud_field_radius: 1.0,
            liquids: Vec::new(),
            physical_solid_runs: Vec::new(),
            shore_surfaces: Vec::new(),
            effect_anchors: Vec::new(),
        };
        let effects_plan = build_liquid_atmosphere_review_plan(profile, &effects_input)
            .map_err(|error| error.to_string())?;
        let state = ReviewWorldDetailProjectionState {
            effects_phase_neutral_hash: effects_plan.phase_neutral_hash(),
            ..default()
        };
        let hashes = ReviewWorldDetailProjectionHashesV1 {
            terrain_plan: hex64(terrain_plan.plan_hash),
            liquid_atmosphere_plan: hex64(effects_plan.plan_hash),
            mesh_projection: hex64(xxh3_64(&[])),
        };
        let report = build_report(
            &hashes,
            &effects_plan.effect_validation,
            ReviewWorldDetailCountsV1::default(),
            &state,
        )?;
        return Ok((state, report, hashes));
    }

    let natural = collect_natural_surfaces(
        map,
        table,
        presentation,
        anchors,
        observation_anchors,
        interiors,
        blockers,
        &anchor_positions,
    )?;
    if natural.is_empty() {
        return Err("no exposed natural surfaces were available".to_owned());
    }
    let by_coord = natural_by_coord(&natural);
    let mut terrain_input = ReviewTerrainInputBuilderV1::new(seed, settings.level_height)
        .map_err(|error| error.to_string())?;
    for surface in &natural {
        let substrate = substrate_for_surface(map, table, surface);
        let color = table
            .get(substrate)
            .map(|substance| {
                let (red, green, blue) = substance.color;
                [red, green, blue, 1.0]
            })
            .unwrap_or([0.45, 0.45, 0.45, 1.0]);
        let cliff_layers = cliff_layers_for_surface(map, table, surface)?;
        let sides = surface.position.coord.neighbors().map(|direction| {
            let adjacent_surface = by_coord.get(&direction).and_then(|surfaces| {
                surfaces.iter().copied().min_by_key(|position| {
                    (
                        surface.position.level.abs_diff(position.level),
                        std::cmp::Reverse(position.level),
                    )
                })
            });
            let exposed_bottom_level = exposed_side_bottom_level(map, surface, direction);
            ReviewTerrainSideInputV1 {
                direction,
                adjacent_surface,
                exposed_bottom_level,
            }
        });
        terrain_input
            .insert_surface(ReviewTerrainSurfaceInputV1 {
                pos: surface.position,
                substrate,
                substrate_color: color,
                exposed_natural: true,
                current_snow: surface.current_snow,
                forced_summit: surface.forced_summit,
                snow_exception: surface.exception,
                sides,
                cliff_layers,
                prop_exclusions: surface.excluded,
            })
            .map_err(|error| error.to_string())?;
    }
    if let Some(presentation) = presentation {
        let garden_mask = presentation.review_garden_mask();
        for (id, feature) in presentation.features() {
            if feature.kind == FeatureKind::Tree && feature.root.level >= 104 {
                terrain_input
                    .insert_vegetation(
                        crate::review_world_detail_terrain::ReviewVegetationInputV1 {
                            stable_id: u64::from(id.0),
                            root: feature.root,
                            snow_dust_eligible: vegetation_snow_dust_eligible(
                                feature.root,
                                garden_mask,
                                &natural,
                            ),
                        },
                    )
                    .map_err(|error| error.to_string())?;
            }
        }
    }
    let terrain_input = terrain_input.build();
    let terrain_plan =
        plan_review_terrain_details(profile, &terrain_input).map_err(|error| error.to_string())?;

    let effects_input = build_effects_input(
        map,
        table,
        settings,
        seed,
        liquid_phase_seconds,
        presentation,
        &natural,
        &terrain_plan.resolved_snow_surfaces,
        &anchor_positions,
    )?;
    let effects_plan = build_liquid_atmosphere_review_plan(profile, &effects_input)
        .map_err(|error| error.to_string())?;

    let mut state = ReviewWorldDetailProjectionState {
        effects_phase_neutral_hash: effects_plan.phase_neutral_hash(),
        ..default()
    };
    let built: ReviewReportBuildResult = (|| {
        let mut counts = ReviewWorldDetailCountsV1::default();
        let mut mesh_hash_bytes = Vec::new();
        let mut terrain_materials = BTreeMap::new();
        for batch in &terrain_plan.mesh_batches {
            let family = terrain_family(batch.material_role);
            let material_key = terrain_material_key(batch);
            let material = terrain_materials.entry(material_key).or_insert_with(|| {
                let handle = materials.add(terrain_material(batch));
                state.materials.push(handle.clone());
                family_count_mut(&mut counts, family).materials += 1;
                handle
            });
            append_mesh_stream_hash(
                &mut mesh_hash_bytes,
                &batch.positions,
                &batch.normals,
                &batch.uv0,
                &[],
                &batch.indices,
            );
            let mesh = terrain_batch_mesh(batch)?;
            let vertices = bounded_u64(batch.positions.len());
            let triangles = bounded_u64(batch.indices.len() / 3);
            let handle = meshes.add(mesh);
            state.meshes.push(handle.clone());
            let entity = commands
                .spawn((
                    Mesh3d(handle),
                    MeshMaterial3d(material.clone()),
                    Transform::IDENTITY,
                    Visibility::Inherited,
                    Pickable::IGNORE,
                    NotShadowCaster,
                    ReviewWorldDetailEntity,
                    Name::new(format!("ReviewTerrainDetail[{family:?}]")),
                ))
                .id();
            state.entities.push(entity);
            let count = family_count_mut(&mut counts, family);
            count.entities += 1;
            count.vertices += vertices;
            count.triangles += triangles;
        }
        for batch in &terrain_plan.vegetation_batches {
            for instance in &batch.instances {
                state.vegetation_treatments.insert(
                    u32::try_from(instance.stable_id)
                        .map_err(|_error| "vegetation identity exceeds u32".to_owned())?,
                    Vec3::from_array(instance.render_child_scale),
                );
            }
        }
        spawn_crown_dust(
            &terrain_plan
                .vegetation_batches
                .iter()
                .flat_map(|batch| batch.instances.iter())
                .copied()
                .collect::<Vec<_>>(),
            presentation,
            art_catalog,
            settings.level_height,
            &mut state,
            &mut counts,
            materials,
            meshes,
            commands,
            &mut mesh_hash_bytes,
        )?;

        let water_color = table
            .id("water")
            .and_then(|id| table.get(id))
            .map(|substance| substance.color)
            .unwrap_or((0.18, 0.42, 0.58));
        let foam_color = table.palette_color("liquid/foam").ok_or_else(|| {
            "review water requires the current liquid/foam palette swatch".to_owned()
        })?;
        let review_water_material_count = effects_plan
            .materials
            .iter()
            .filter(|descriptor| matches!(descriptor.key, ReviewMaterialKeyV1::Water { .. }))
            .count();
        if review_water_material_count > REVIEW_WATER_MATERIAL_LIMIT {
            return Err(format!(
                "review water material count {review_water_material_count} exceeds bounded limit {REVIEW_WATER_MATERIAL_LIMIT}"
            ));
        }
        let mut effect_materials = BTreeMap::new();
        for descriptor in &effects_plan.materials {
            let family = effect_material_family(descriptor.key);
            let handle = if matches!(descriptor.key, ReviewMaterialKeyV1::Water { .. }) {
                let handle = review_water_materials.add(review_water_material(
                    descriptor,
                    water_color,
                    foam_color,
                    table,
                    liquid_phase_seconds,
                )?);
                state.review_water_materials.push(handle.clone());
                EffectMaterialHandle::ReviewWater(handle)
            } else {
                let handle = materials.add(effect_material(descriptor, water_color, table)?);
                state.materials.push(handle.clone());
                EffectMaterialHandle::Standard(handle)
            };
            family_count_mut(&mut counts, family).materials += 1;
            effect_materials.insert(descriptor.key, handle);
        }
        for batch in &effects_plan.mesh_batches {
            let family = effect_mesh_family(batch.key.layer);
            let material = effect_materials
                .get(&batch.key.material)
                .ok_or_else(|| format!("missing shared material for {:?}", batch.key.material))?;
            let vertex_colors = if matches!(batch.key.material, ReviewMaterialKeyV1::Water { .. }) {
                water_value_vertex_colors(
                    &batch.mesh.colors,
                    batch.mesh.positions.len(),
                    water_color,
                )?
            } else {
                batch.mesh.colors.clone()
            };
            append_mesh_stream_hash(
                &mut mesh_hash_bytes,
                &batch.mesh.positions,
                &batch.mesh.normals,
                &batch.mesh.uvs,
                &vertex_colors,
                &batch.mesh.indices,
            );
            let mesh = indexed_mesh(&batch.mesh, vertex_colors)?;
            let vertices = bounded_u64(batch.mesh.positions.len());
            let triangles = bounded_u64(batch.mesh.indices.len() / 3);
            let handle = meshes.add(mesh);
            state.meshes.push(handle.clone());
            let entity = spawn_effect_mesh(commands, handle, material, family);
            state.entities.push(entity);
            let count = family_count_mut(&mut counts, family);
            count.entities += 1;
            count.vertices += vertices;
            count.triangles += triangles;
        }
        spawn_spray(
            &effects_plan.spray_volumes,
            &mut state,
            &mut counts,
            materials,
            meshes,
            commands,
            &mut mesh_hash_bytes,
        )?;
        spawn_cloud_shadows(
            &effects_plan.cloud_shadows,
            &natural,
            settings.level_height,
            &mut state,
            &mut counts,
            materials,
            meshes,
            commands,
            &mut mesh_hash_bytes,
        )?;
        spawn_fog(
            &effects_plan.fog_volumes,
            &mut state,
            &mut counts,
            images,
            commands,
        )?;
        counts.total = counts.computed_total();

        let hashes = ReviewWorldDetailProjectionHashesV1 {
            terrain_plan: hex64(terrain_plan.plan_hash),
            liquid_atmosphere_plan: hex64(effects_plan.plan_hash),
            mesh_projection: hex64(xxh3_64(&mesh_hash_bytes)),
        };
        let report = build_report(&hashes, &effects_plan.effect_validation, counts, &state)?;
        Ok((report, hashes))
    })();
    match built {
        Ok((report, hashes)) => Ok((state, report, hashes)),
        Err(error) => {
            rollback_pending_review_projection(
                &state,
                materials,
                review_water_materials,
                meshes,
                images,
                commands,
            );
            Err(error)
        }
    }
}

/// Rolls back both immediate asset mutations and deferred entity spawns from a
/// failed build. Every mutation in the fallible construction phase is recorded
/// in `state` before another fallible operation can run.
fn rollback_pending_review_projection(
    state: &ReviewWorldDetailProjectionState,
    materials: &mut Assets<StandardMaterial>,
    review_water_materials: &mut Assets<ReviewWaterMaterial>,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    commands: &mut Commands,
) {
    commands.remove_resource::<ReviewWorldDetailRuntimeAssetEvidenceV1>();
    commands.remove_resource::<ReviewWorldDetailTeardownReceiptV1>();
    commands.remove_resource::<ReviewWorldDetailTeardownTargets>();
    for entity in &state.entities {
        commands.entity(*entity).try_despawn();
    }
    for handle in &state.meshes {
        meshes.remove(handle.id());
    }
    for handle in &state.materials {
        materials.remove(handle.id());
    }
    for handle in &state.review_water_materials {
        review_water_materials.remove(handle.id());
    }
    for handle in &state.images {
        images.remove(handle.id());
    }
}

fn review_runtime_asset_evidence(
    state: &ReviewWorldDetailProjectionState,
    live_liquid_materials: &[Handle<LiquidMaterial>],
    liquid_materials: &Assets<LiquidMaterial>,
    review_water_materials: &Assets<ReviewWaterMaterial>,
    images: &Assets<Image>,
) -> Result<ReviewWorldDetailRuntimeAssetEvidenceV1, String> {
    let mut liquid_ids = BTreeSet::new();
    let mut liquid_bytes = 0_u64;
    for handle in live_liquid_materials {
        if !liquid_ids.insert(handle.id()) {
            continue;
        }
        let material = liquid_materials.get(handle).ok_or_else(|| {
            format!(
                "live ordinary liquid material binding {:?} has no asset",
                handle.id()
            )
        })?;
        let allocation_bytes = u64::try_from(std::mem::size_of_val(material))
            .map_err(|_error| "ordinary liquid material size exceeds u64".to_owned())?;
        liquid_bytes = liquid_bytes
            .checked_add(allocation_bytes)
            .ok_or_else(|| "ordinary liquid material byte count overflowed u64".to_owned())?;
    }

    let mut review_water_ids = BTreeSet::new();
    let mut review_water_bytes = 0_u64;
    for handle in &state.review_water_materials {
        if !review_water_ids.insert(handle.id()) {
            continue;
        }
        let material = review_water_materials.get(handle).ok_or_else(|| {
            format!(
                "committed review-water material {:?} is not live",
                handle.id()
            )
        })?;
        let allocation_bytes = u64::try_from(std::mem::size_of_val(material))
            .map_err(|_error| "review-water material size exceeds u64".to_owned())?;
        review_water_bytes = review_water_bytes
            .checked_add(allocation_bytes)
            .ok_or_else(|| "review-water material byte count overflowed u64".to_owned())?;
    }
    let mut fog_image_ids = BTreeSet::new();
    let mut fog_image_bytes = 0_u64;
    for handle in &state.images {
        if !fog_image_ids.insert(handle.id()) {
            continue;
        }
        let image = images.get(handle).ok_or_else(|| {
            format!(
                "committed review fog density image {:?} is not live",
                handle.id()
            )
        })?;
        let allocation_bytes = u64::try_from(image.data.as_ref().map_or(0, Vec::len))
            .map_err(|_error| "review fog density image payload exceeds u64".to_owned())?;
        fog_image_bytes = fog_image_bytes
            .checked_add(allocation_bytes)
            .ok_or_else(|| "review fog density image byte count overflowed u64".to_owned())?;
    }
    Ok(ReviewWorldDetailRuntimeAssetEvidenceV1 {
        liquid_material_count: bounded_u64(liquid_ids.len()),
        liquid_material_bytes: liquid_bytes,
        review_water_material_count: bounded_u64(review_water_ids.len()),
        review_water_material_bytes: review_water_bytes,
        fog_density_image_count: bounded_u64(fog_image_ids.len()),
        fog_density_image_bytes: fog_image_bytes,
    })
}

fn collect_anchor_evidence(
    anchors: &MapAnchors,
    observation: Option<&MapObservationAnchors>,
    level_height: f32,
) -> Result<
    (
        BTreeMap<String, f32>,
        BTreeMap<String, ReviewAnchorClassV1>,
        BTreeMap<String, (TilePos, ReviewAnchorClassV1)>,
    ),
    String,
> {
    let mut positions = BTreeMap::new();
    for (id, position) in anchors.iter() {
        positions.insert(
            id.as_str().to_owned(),
            (position, ReviewAnchorClassV1::Gameplay),
        );
    }
    if let Some(observation) = observation {
        for (id, position) in observation.iter() {
            if positions
                .insert(
                    id.as_str().to_owned(),
                    (position, ReviewAnchorClassV1::Observation),
                )
                .is_some()
            {
                return Err(format!(
                    "anchor {:?} appears in gameplay and observation namespaces",
                    id.as_str()
                ));
            }
        }
    }
    let heights = positions
        .iter()
        .map(|(name, (position, _))| (name.clone(), surface_y(position.level, level_height)))
        .collect();
    let classes = positions
        .iter()
        .map(|(name, (_, class))| (name.clone(), *class))
        .collect();
    Ok((heights, classes, positions))
}

#[expect(
    clippy::too_many_arguments,
    reason = "classification keeps every exclusion source explicit"
)]
fn collect_natural_surfaces(
    map: &VoxelMap,
    table: &SubstanceTable,
    presentation: Option<&MapPresentationProjection>,
    anchors: &MapAnchors,
    observation: Option<&MapObservationAnchors>,
    interiors: Option<&InteriorRegions>,
    blockers: Option<&TraversalBlockers>,
    anchor_positions: &BTreeMap<String, (TilePos, ReviewAnchorClassV1)>,
) -> Result<Vec<NaturalSurface>, String> {
    let structure_voxels = presentation
        .into_iter()
        .flat_map(|projection| projection.structures().values())
        .flat_map(|structure| structure.voxels.iter().copied())
        .collect::<BTreeSet<_>>();
    let protected_route_surfaces = presentation
        .into_iter()
        .flat_map(|projection| projection.review_protected_routes().values())
        .flat_map(|surfaces| surfaces.iter().copied())
        .collect::<BTreeSet<_>>();
    let protected_corridor_surfaces = presentation
        .into_iter()
        .flat_map(|projection| projection.review_protected_routes())
        .filter(|(name, _)| {
            let name = name.to_ascii_lowercase();
            name.contains("tunnel") || name.contains("corridor") || name.contains("interior")
        })
        .flat_map(|(_, surfaces)| surfaces.iter().copied())
        .collect::<BTreeSet<_>>();
    let anchor_coords = anchors
        .iter()
        .map(|(_, position)| position.coord)
        .chain(
            observation
                .into_iter()
                .flat_map(MapObservationAnchors::iter)
                .map(|(_, position)| position.coord),
        )
        .collect::<Vec<_>>();
    let forced_summits = presentation
        .map(MapPresentationProjection::review_forced_summits)
        .cloned()
        .unwrap_or_default();
    let frozen_mask = presentation
        .map(MapPresentationProjection::review_frozen_woods_mask)
        .cloned()
        .unwrap_or_default();
    let garden_mask = presentation
        .map(MapPresentationProjection::review_garden_mask)
        .cloned()
        .unwrap_or_default();
    if anchor_positions.contains_key("grand_v3.frozen_woods") && frozen_mask.is_empty() {
        return Err(
            "Grand V3 review projection is missing the exact authored Frozen-Woods mask".to_owned(),
        );
    }
    if anchor_positions.contains_key("grand_v3.lake_island") && garden_mask.is_empty() {
        return Err(
            "Grand V3 review projection is missing the exact authored Lake-Island garden mask"
                .to_owned(),
        );
    }
    if anchor_positions.contains_key("grand_v3.massif_crest") && forced_summits.is_empty() {
        return Err("Grand V3 review projection is missing its exact forced summits".to_owned());
    }
    let mut surfaces = Vec::new();
    for (coord, column) in map.columns() {
        for run in runs(column) {
            if !column.get(run.top).is_air() || !table.is_solid(run.substance) {
                continue;
            }
            let Some(name) = table.name(run.substance) else {
                continue;
            };
            if !matches!(
                name,
                "grass" | "dirt" | "stone" | "gravel" | "sand" | "snow" | "ice" | "basalt"
            ) {
                continue;
            }
            let position = TilePos::new(coord, run.top.saturating_sub(1));
            if interiors.is_some_and(|regions| regions.get(position).is_some()) {
                continue;
            }
            let exception = if frozen_mask.contains(&coord) {
                ReviewSnowExceptionV1::FrozenWoods
            } else if garden_mask.contains(&coord) {
                ReviewSnowExceptionV1::Garden
            } else {
                ReviewSnowExceptionV1::None
            };
            let near_named_anchor = anchor_coords
                .iter()
                .any(|anchor| anchor.distance(coord) <= 5);
            let anchor_labels = anchor_positions
                .iter()
                .filter(|(_, (anchor, _))| anchor.coord.distance(coord) <= 5)
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>();
            let excluded = ReviewPropExclusionsV1 {
                water: false,
                path: protected_route_surfaces.contains(&position)
                    || name == "gravel"
                    || anchor_labels
                        .iter()
                        .any(|name| name.contains("route") || name.contains("path")),
                // Exact protected route footprints remain presentation-only, but
                // prop scattering must not visually narrow a tunnel/corridor lane.
                // Other map interiors were removed from the natural set above.
                corridor: protected_corridor_surfaces.contains(&position),
                portal: anchor_labels.iter().any(|name| {
                    name.contains("portal") || name.contains("tunnel") || name.contains("exit")
                }),
                spawn: anchor_labels
                    .iter()
                    .any(|name| name.contains("start") || name.contains("spawn")),
                structure: structure_voxels.contains(&position)
                    || blockers.is_some_and(|blockers| blockers.contains(position)),
                named_anchor_safety_disk: near_named_anchor,
            };
            surfaces.push(NaturalSurface {
                position,
                run_bottom: run.bottom,
                solid_stack_bottom: contiguous_solid_stack_bottom(column, table, run.bottom),
                substance: run.substance,
                current_snow: name == "snow",
                exception,
                forced_summit: forced_summits.contains(&position),
                excluded,
            });
        }
    }
    surfaces.sort_by_key(|surface| surface.position);
    Ok(surfaces)
}

fn contiguous_solid_stack_bottom(
    column: &Column,
    table: &SubstanceTable,
    top_run_bottom: i32,
) -> i32 {
    let mut bottom = top_run_bottom.max(0);
    while bottom > 0 && table.is_solid(column.get(bottom.saturating_sub(1))) {
        bottom = bottom.saturating_sub(1);
    }
    bottom
}

fn exposed_side_bottom_level(map: &VoxelMap, surface: &NaturalSurface, direction: HexCoord) -> i32 {
    map.column(direction)
        .and_then(|column| {
            (surface.solid_stack_bottom..=surface.position.level)
                .rev()
                .find(|level| !column.get(*level).is_air())
        })
        .map_or(surface.solid_stack_bottom, |level| level.saturating_add(1))
        .clamp(
            surface.solid_stack_bottom,
            surface.position.level.saturating_add(1),
        )
}

fn is_natural_cliff_substance(name: &str) -> bool {
    matches!(
        name,
        "grass" | "dirt" | "stone" | "gravel" | "sand" | "snow" | "ice" | "basalt" | "bedrock"
    )
}

fn cliff_layers_for_surface(
    map: &VoxelMap,
    table: &SubstanceTable,
    surface: &NaturalSurface,
) -> Result<Vec<ReviewCliffLayerInputV1>, String> {
    let column = map.column(surface.position.coord).ok_or_else(|| {
        format!(
            "natural surface column is missing at {:?}",
            surface.position
        )
    })?;
    let top_level = surface.position.level.saturating_add(1);
    let mut layers = Vec::new();
    for run in runs(column)
        .into_iter()
        .filter(|run| run.top > surface.solid_stack_bottom && run.bottom < top_level)
    {
        let name = table.name(run.substance).ok_or_else(|| {
            format!(
                "cliff layer at {:?} uses an unknown substance {:?}",
                surface.position, run.substance
            )
        })?;
        if !is_natural_cliff_substance(name) {
            continue;
        }
        let substrate = if name == "snow" {
            (0..run.bottom)
                .rev()
                .map(|level| column.get(level))
                .find(|substance| {
                    table.is_solid(*substance)
                        && table.name(*substance).is_some_and(|name| name != "snow")
                })
                .ok_or_else(|| {
                    format!(
                        "snow cliff layer at {:?} has no underlying non-snow substrate",
                        surface.position
                    )
                })?
        } else {
            run.substance
        };
        let substrate_color = table
            .get(substrate)
            .map(|substance| {
                let (red, green, blue) = substance.color;
                [red, green, blue, 1.0]
            })
            .ok_or_else(|| {
                format!(
                    "cliff layer at {:?} cannot resolve substrate {:?}",
                    surface.position, substrate
                )
            })?;
        layers.push(ReviewCliffLayerInputV1 {
            bottom_level: run.bottom.max(surface.solid_stack_bottom),
            top_level: run.top.min(top_level),
            substrate,
            substrate_color,
        });
    }
    if layers.is_empty() {
        return Err(format!(
            "natural surface at {:?} has no natural cliff layers",
            surface.position
        ));
    }
    Ok(layers)
}

fn vegetation_snow_dust_eligible(
    root: TilePos,
    garden_mask: &BTreeSet<HexCoord>,
    natural: &[NaturalSurface],
) -> bool {
    !garden_mask.contains(&root.coord)
        && (root.level >= 124
            || natural
                .iter()
                .any(|surface| surface.position == root && surface.current_snow))
}

fn natural_by_coord(surfaces: &[NaturalSurface]) -> BTreeMap<HexCoord, Vec<TilePos>> {
    let mut by_coord = BTreeMap::<HexCoord, Vec<TilePos>>::new();
    for surface in surfaces {
        by_coord
            .entry(surface.position.coord)
            .or_default()
            .push(surface.position);
    }
    by_coord
}

fn substrate_for_surface(
    map: &VoxelMap,
    table: &SubstanceTable,
    surface: &NaturalSurface,
) -> SubstanceId {
    if !surface.current_snow {
        return surface.substance;
    }
    let Some(column) = map.column(surface.position.coord) else {
        return surface.substance;
    };
    (0..surface.run_bottom)
        .rev()
        .map(|level| column.get(level))
        .find(|substance| {
            table.is_solid(*substance) && table.name(*substance).is_some_and(|name| name != "snow")
        })
        .unwrap_or(surface.substance)
}

#[expect(
    clippy::too_many_arguments,
    clippy::cast_precision_loss,
    reason = "effect input copies exact map-owned projections, and the validated map radius is far below f32 integer precision"
)]
fn build_effects_input(
    map: &VoxelMap,
    table: &SubstanceTable,
    settings: &MapSettings,
    seed: u64,
    phase_seconds: f32,
    presentation: Option<&MapPresentationProjection>,
    natural: &[NaturalSurface],
    resolved_snow: &BTreeSet<TilePos>,
    anchors: &BTreeMap<String, (TilePos, ReviewAnchorClassV1)>,
) -> Result<LiquidAtmosphereReviewInputV1, String> {
    let mut liquids = Vec::new();
    if let Some(presentation) = presentation {
        for (coord, column) in map.columns() {
            for run in runs(column) {
                let kind = match table.name(run.substance) {
                    Some("water") => ReviewLiquidKindV1::Water,
                    Some("lava") => ReviewLiquidKindV1::Lava,
                    _ => continue,
                };
                if !column.get(run.top).is_air() {
                    continue;
                }
                let position = TilePos::new(coord, run.top.saturating_sub(1));
                let descriptor = presentation
                    .liquids()
                    .get(&position)
                    .ok_or_else(|| format!("liquid projection omits {position:?}"))?;
                liquids.push(ReviewLiquidCellV1 {
                    position,
                    run_bottom: run.bottom,
                    kind,
                    flow: match descriptor.flow {
                        LiquidFlowState::Still => ReviewLiquidFlowV1::Still,
                        LiquidFlowState::Current => ReviewLiquidFlowV1::Current,
                        LiquidFlowState::Rapid => ReviewLiquidFlowV1::Rapid,
                        LiquidFlowState::Fall => ReviewLiquidFlowV1::Fall,
                    },
                    downstream: descriptor.downstream,
                    chunk: chunk_key(position.coord),
                });
            }
        }
    }
    let mut physical_solid_runs = map
        .columns()
        .flat_map(|(coord, column)| {
            runs(column)
                .into_iter()
                .filter(|run| table.is_solid(run.substance))
                .map(move |run| ReviewPhysicalSolidRunV1 {
                    position: TilePos::new(coord, run.top.saturating_sub(1)),
                    run_bottom: run.bottom,
                })
        })
        .collect::<Vec<_>>();
    physical_solid_runs.sort_by_key(|run| run.position);
    let shore_surfaces = natural
        .iter()
        .map(|surface| {
            let snow_covered = resolved_snow.contains(&surface.position);
            Ok(ReviewShoreSurfaceV1 {
                position: surface.position,
                // A visible bank can be capped by a one-voxel grass, sand, or
                // snow material run while the contiguous physical bank below
                // it still crosses the adjacent waterline.  Shore ownership is
                // about that complete solid stack, not only the top material
                // run; using the latter made every deliberately raised Grand
                // V3 bank look disconnected from its water.
                run_bottom: surface.solid_stack_bottom,
                chunk: chunk_key(surface.position.coord),
                // Wet-rim value and roughness are relative to the surface that
                // the active review presentation actually exposes. This is snow
                // for a newly covered shore and the underlying non-snow
                // substrate when the selected snow treatment removes a cap.
                substance: resolved_shore_substance(map, table, surface, snow_covered)?,
                snow_covered,
                frozen_biome: surface.exception == ReviewSnowExceptionV1::FrozenWoods,
                eligible: !surface.excluded.structure
                    && !surface.excluded.portal
                    && !surface.excluded.spawn,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut effect_anchors = Vec::new();
    for (name, (position, _)) in anchors {
        let kind = review_effect_anchor_kind(name);
        if let Some(kind) = kind {
            effect_anchors.push(ReviewEffectAnchorV1 {
                name: name.clone(),
                kind,
                position: *position,
                surface: position
                    .coord
                    .to_world(surface_y(position.level, settings.level_height)),
            });
        }
    }
    let massif = anchors
        .get("grand_v3.massif_crest")
        .ok_or_else(|| "grand_v3.massif_crest review anchor is missing".to_owned())?
        .0;
    let max_exposed_natural_y = natural
        .iter()
        .map(|surface| surface_y(surface.position.level, settings.level_height))
        .fold(f32::NEG_INFINITY, f32::max);
    let peak_surface = natural
        .iter()
        .filter(|surface| {
            surface_y(surface.position.level, settings.level_height).to_bits()
                == max_exposed_natural_y.to_bits()
        })
        .min_by_key(|surface| surface.position)
        .ok_or_else(|| "could not resolve the maximum exposed natural-terrain peak".to_owned())?;
    let interaction_peak = peak_surface.position.coord.to_world(max_exposed_natural_y);
    let interaction_peak_solid_spans = map
        .column(peak_surface.position.coord)
        .ok_or_else(|| "selected cloud-interaction peak has no voxel column".to_owned())
        .map(|column| {
            runs(column)
                .into_iter()
                .filter(|run| table.is_solid(run.substance))
                .map(|run| ReviewPeakSolidSpanV1 {
                    bottom_y: run.bottom as f32 * settings.level_height,
                    top_y: run.top as f32 * settings.level_height,
                })
                .collect::<Vec<_>>()
        })?;
    Ok(LiquidAtmosphereReviewInputV1 {
        seed,
        level_height: settings.level_height,
        phase_seconds,
        max_exposed_natural_y,
        massif_crest: massif
            .coord
            .to_world(surface_y(massif.level, settings.level_height)),
        interaction_peak,
        interaction_peak_solid_spans,
        // Coverage is defined against this deterministic circular massif field,
        // not against the complete axial-map footprint.
        cloud_field_radius: (settings.grid_radius as f32 * 0.52).clamp(64.0, 120.0),
        liquids,
        physical_solid_runs,
        shore_surfaces,
        effect_anchors,
    })
}

fn resolved_shore_substance(
    map: &VoxelMap,
    table: &SubstanceTable,
    surface: &NaturalSurface,
    snow_covered: bool,
) -> Result<SubstanceId, String> {
    if snow_covered {
        return table.id("snow").ok_or_else(|| {
            "resolved shoreline snow requires the ordinary snow substance".to_owned()
        });
    }
    let substrate = substrate_for_surface(map, table, surface);
    if surface.current_snow && table.name(substrate) == Some("snow") {
        return Err(format!(
            "could not resolve the non-snow shoreline substrate beneath {:?}",
            surface.position
        ));
    }
    Ok(substrate)
}

fn review_effect_anchor_kind(name: &str) -> Option<ReviewEffectAnchorKindV1> {
    // Keep atmospheric placement tied to the exact published Grand V3 anchor
    // contract. Substring classification can silently admit bridge decks or
    // dry scenic observations such as `grand_v3.lake_island` merely because
    // their names contain `coast`, `valley`, `river`, or `lake`.
    match name {
        "grand_v3.waterfall_crown" | "grand_v3.waterfall_base" | "grand_v3.waterfall_profile" => {
            Some(ReviewEffectAnchorKindV1::Waterfall)
        }
        "grand_v3.valley_lake" => Some(ReviewEffectAnchorKindV1::Valley),
        "grand_v3.river_bend" => Some(ReviewEffectAnchorKindV1::ValleyWater),
        "grand_v3.coast" | "grand_v3.mountain_lake" => Some(ReviewEffectAnchorKindV1::Water),
        _ => None,
    }
}

fn chunk_key(coord: HexCoord) -> ReviewChunkKeyV1 {
    let (q, r) = terrain_chunk_key(coord);
    ReviewChunkKeyV1 { q, r }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    Snow,
    Water,
    PhysicalClouds,
    ShoreAndFalls,
    CliffStrata,
    TerrainProps,
    IceFringe,
}

#[derive(Debug, Clone)]
enum EffectMaterialHandle {
    Standard(Handle<StandardMaterial>),
    ReviewWater(Handle<ReviewWaterMaterial>),
}

fn spawn_effect_mesh(
    commands: &mut Commands,
    mesh: Handle<Mesh>,
    material: &EffectMaterialHandle,
    family: Family,
) -> Entity {
    let mut entity = commands.spawn((
        Mesh3d(mesh),
        Transform::IDENTITY,
        Visibility::Inherited,
        Pickable::IGNORE,
        // All physical cloud puffs carry this marker too. C08's explicit,
        // blurred terrain overlay is the sole cloud-shadow implementation.
        NotShadowCaster,
        ReviewWorldDetailEntity,
        Name::new(format!("ReviewEffectDetail[{family:?}]")),
    ));
    match material {
        EffectMaterialHandle::Standard(handle) => {
            entity.insert(MeshMaterial3d(handle.clone()));
        }
        EffectMaterialHandle::ReviewWater(handle) => {
            entity.insert(MeshMaterial3d(handle.clone()));
        }
    }
    entity.id()
}

fn family_count_mut(
    counts: &mut ReviewWorldDetailCountsV1,
    family: Family,
) -> &mut ReviewPresentationCountsV1 {
    match family {
        Family::Snow => &mut counts.snow,
        Family::Water => &mut counts.water,
        Family::PhysicalClouds => &mut counts.physical_clouds,
        Family::ShoreAndFalls => &mut counts.shore_and_falls,
        Family::CliffStrata => &mut counts.cliff_strata,
        Family::TerrainProps => &mut counts.terrain_props,
        Family::IceFringe => &mut counts.ice_fringe,
    }
}

fn terrain_family(role: ReviewTerrainMaterialRoleV1) -> Family {
    match role {
        ReviewTerrainMaterialRoleV1::SnowCap | ReviewTerrainMaterialRoleV1::SubstrateRestore => {
            Family::Snow
        }
        ReviewTerrainMaterialRoleV1::CliffValue | ReviewTerrainMaterialRoleV1::CliffStrata => {
            Family::CliffStrata
        }
        ReviewTerrainMaterialRoleV1::Boulder
        | ReviewTerrainMaterialRoleV1::Tuft
        | ReviewTerrainMaterialRoleV1::Deadwood => Family::TerrainProps,
    }
}

fn effect_mesh_family(layer: ReviewMeshLayerV1) -> Family {
    match layer {
        ReviewMeshLayerV1::WaterCaps | ReviewMeshLayerV1::WaterCurtains => Family::Water,
        ReviewMeshLayerV1::WetRims | ReviewMeshLayerV1::ShoreFoam | ReviewMeshLayerV1::PoolFoam => {
            Family::ShoreAndFalls
        }
        ReviewMeshLayerV1::IceFringes => Family::IceFringe,
        ReviewMeshLayerV1::CloudPuffs => Family::PhysicalClouds,
    }
}

fn effect_material_family(key: ReviewMaterialKeyV1) -> Family {
    match key {
        ReviewMaterialKeyV1::Water { .. } => Family::Water,
        ReviewMaterialKeyV1::WetRim { .. } | ReviewMaterialKeyV1::Foam => Family::ShoreAndFalls,
        ReviewMaterialKeyV1::Ice => Family::IceFringe,
        ReviewMaterialKeyV1::Cloud => Family::PhysicalClouds,
    }
}

fn terrain_material_key(batch: &ReviewTerrainMeshBatchV1) -> (u8, u16, [u32; 4]) {
    let role = match batch.material_role {
        ReviewTerrainMaterialRoleV1::SnowCap => 0,
        ReviewTerrainMaterialRoleV1::SubstrateRestore => 1,
        ReviewTerrainMaterialRoleV1::CliffValue => 2,
        ReviewTerrainMaterialRoleV1::CliffStrata => 3,
        ReviewTerrainMaterialRoleV1::Boulder => 4,
        ReviewTerrainMaterialRoleV1::Tuft => 5,
        ReviewTerrainMaterialRoleV1::Deadwood => 6,
    };
    (
        role,
        batch.substrate.unwrap_or(SubstanceId::AIR).0,
        batch.base_color.map(f32::to_bits),
    )
}

fn terrain_material(batch: &ReviewTerrainMeshBatchV1) -> StandardMaterial {
    let [red, green, blue, alpha] = batch.base_color;
    let cliff_shell = matches!(
        batch.material_role,
        ReviewTerrainMaterialRoleV1::CliffValue | ReviewTerrainMaterialRoleV1::CliffStrata
    );
    StandardMaterial {
        base_color: Color::srgba(red, green, blue, alpha),
        perceptual_roughness: match batch.material_role {
            // Snow masks and substrate-relative cliff shells change only their
            // named presentation property. They retain the ordinary terrain
            // material's Bevy-default 0.5 roughness.
            ReviewTerrainMaterialRoleV1::SnowCap
            | ReviewTerrainMaterialRoleV1::SubstrateRestore
            | ReviewTerrainMaterialRoleV1::CliffValue
            | ReviewTerrainMaterialRoleV1::CliffStrata => 0.5,
            ReviewTerrainMaterialRoleV1::Boulder => 0.86,
            ReviewTerrainMaterialRoleV1::Tuft => 0.92,
            ReviewTerrainMaterialRoleV1::Deadwood => 0.88,
        },
        alpha_mode: if alpha < 0.999 {
            AlphaMode::Blend
        } else {
            AlphaMode::Opaque
        },
        // Exact cliff value is carried by an opaque substrate-coloured shell,
        // not by unlit black alpha compositing. Depth bias resolves its exact
        // coplanarity with the ordinary hard-normal terrain side.
        unlit: false,
        depth_bias: if cliff_shell { 4.0 } else { 0.0 },
        cull_mode: cliff_shell.then_some(Face::Back),
        double_sided: !cliff_shell,
        ..default()
    }
}

fn effect_material(
    descriptor: &ReviewMaterialDescriptorV1,
    water_color: (f32, f32, f32),
    table: &SubstanceTable,
) -> Result<StandardMaterial, String> {
    let base = match descriptor.key {
        ReviewMaterialKeyV1::Water { .. } => water_color,
        ReviewMaterialKeyV1::WetRim { substrate } => table
            .get(substrate)
            .map(|definition| definition.color)
            .ok_or_else(|| format!("wet-rim substrate {substrate:?} is unavailable"))?,
        ReviewMaterialKeyV1::Foam => table.palette_color("liquid/foam").ok_or_else(|| {
            "review shore foam requires the current liquid/foam palette swatch".to_owned()
        })?,
        ReviewMaterialKeyV1::Ice => table
            .id("ice")
            .and_then(|ice| table.get(ice))
            .map(|definition| definition.color)
            .ok_or_else(|| {
                "review ice fringe requires the current palette-backed ice substance".to_owned()
            })?,
        ReviewMaterialKeyV1::Cloud => (0.92, 0.94, 0.98),
    };
    let value = descriptor.value_multiplier;
    let alpha = descriptor.alpha.unwrap_or(1.0);
    let transmission = descriptor.transmission;
    let current_roughness = match descriptor.key {
        ReviewMaterialKeyV1::Water {
            style: ReviewWaterMaterialStyleV1::Surface,
            ..
        } => REVIEW_WATER_SURFACE_ROUGHNESS,
        ReviewMaterialKeyV1::Water {
            style: ReviewWaterMaterialStyleV1::Fall,
            ..
        } => REVIEW_WATER_FALL_ROUGHNESS,
        // The locked baseline uses the ordinary terrain material's Bevy-default
        // 0.5 roughness. The opaque substrate-coloured rim therefore realizes
        // the requested additive delta directly rather than layering a second
        // translucent PBR lobe over the original surface.
        ReviewMaterialKeyV1::WetRim { .. } => {
            (0.5 + descriptor.roughness_delta.unwrap_or(0.0)).clamp(0.089, 1.0)
        }
        _ => 0.62,
    };
    let current_reflectance = match descriptor.key {
        ReviewMaterialKeyV1::Water {
            style: ReviewWaterMaterialStyleV1::Surface,
            ..
        } => REVIEW_WATER_SURFACE_REFLECTANCE,
        ReviewMaterialKeyV1::Water {
            style: ReviewWaterMaterialStyleV1::Fall,
            ..
        } => REVIEW_WATER_FALL_REFLECTANCE,
        _ => 0.35,
    };
    let mut material = StandardMaterial {
        base_color: Color::srgba(
            (base.0 * value).clamp(0.0, 1.0),
            (base.1 * value).clamp(0.0, 1.0),
            (base.2 * value).clamp(0.0, 1.0),
            alpha,
        ),
        perceptual_roughness: descriptor.roughness.unwrap_or(current_roughness),
        reflectance: descriptor.reflectance.unwrap_or(current_reflectance),
        alpha_mode: match descriptor.alpha_mode {
            ReviewAlphaModeV1::Opaque => AlphaMode::Opaque,
            ReviewAlphaModeV1::OrderIndependentTransparency => AlphaMode::Blend,
        },
        cull_mode: (!descriptor.double_sided).then_some(Face::Back),
        double_sided: descriptor.double_sided,
        depth_bias: 1.0,
        ..default()
    };
    if let Some(transmission) = transmission {
        material.base_color.set_alpha(1.0);
        material.alpha_mode = AlphaMode::Opaque;
        // Bevy's rough transmission path takes displaced taps around the
        // central refracted ray. W06 requires every effective sample to remain
        // within 0.015 screen UV, so use the single-ray zero-roughness path.
        material.perceptual_roughness = 0.0;
        material.specular_transmission = 0.88;
        material.diffuse_transmission = 0.08;
        material.ior = transmission.ior;
        material.thickness = transmission.thickness;
    }
    // W04/W05 depth absorption is carried continuously by the batched mesh's
    // linear RGB vertex multiplier. StandardMaterial's attenuation fields affect
    // only transmitted light, so assigning them on these non-transmission alpha
    // profiles would be dead and misleading.
    Ok(material)
}

fn review_water_material(
    descriptor: &ReviewMaterialDescriptorV1,
    water_color: (f32, f32, f32),
    foam_color: (f32, f32, f32),
    table: &SubstanceTable,
    phase_seconds: f32,
) -> Result<ReviewWaterMaterial, String> {
    let style = match descriptor.key {
        ReviewMaterialKeyV1::Water { style, .. } => style,
        _ => return Err("review water material received a non-water descriptor".to_owned()),
    };
    let maximum_refraction_uv = match descriptor.transmission {
        Some(transmission) => {
            if !transmission.max_refraction_uv.is_finite()
                || transmission.max_refraction_uv <= 0.0
                || transmission.max_refraction_uv > REVIEW_MAX_REFRACTION_UV
            {
                return Err(format!(
                    "review water max_refraction_uv must be finite and in (0,{REVIEW_MAX_REFRACTION_UV}]"
                ));
            }
            transmission.max_refraction_uv
        }
        None => 0.0,
    };
    let mut base = effect_material(descriptor, water_color, table)?;
    // Review water always uses the forward path: transparent profiles participate
    // in OIT and W06 reads Bevy's screen-space transmission texture.
    base.opaque_render_method = OpaqueRendererMethod::Forward;
    let (flow_velocity, modulation) = match style {
        ReviewWaterMaterialStyleV1::Surface => (Vec2::ZERO, Vec4::new(0.08, 0.0, 0.04, 0.65)),
        ReviewWaterMaterialStyleV1::Fall => (
            Vec2::new(0.0, REVIEW_WATER_FALL_FLOW_SPEED),
            Vec4::new(0.34, 0.48, 0.14, 1.25),
        ),
    };
    let foam = Color::srgb(foam_color.0, foam_color.1, foam_color.2).to_linear();
    Ok(ExtendedMaterial {
        base,
        extension: ReviewWaterExtension {
            params: ReviewWaterMaterialParams {
                flow_phase_scale: Vec4::new(
                    flow_velocity.x,
                    flow_velocity.y,
                    bounded_review_water_phase(phase_seconds),
                    3.0,
                ),
                modulation,
                emission: Vec4::ZERO,
                foam_color: Vec4::new(foam.red, foam.green, foam.blue, 1.0),
                refraction: Vec4::new(maximum_refraction_uv, 0.0, 0.0, 0.0),
            },
        },
    })
}

fn bounded_review_water_phase(phase_seconds: f32) -> f32 {
    if phase_seconds.is_finite() {
        phase_seconds.rem_euclid(REVIEW_WATER_PHASE_WRAP_SECONDS)
    } else {
        0.0
    }
}

fn set_review_water_material_phase(material: &mut ReviewWaterMaterial, phase_seconds: f32) {
    material.extension.params.flow_phase_scale.z = bounded_review_water_phase(phase_seconds);
}

fn terrain_batch_mesh(batch: &ReviewTerrainMeshBatchV1) -> Result<Mesh, String> {
    mesh_from_parts(
        batch.positions.clone(),
        batch.normals.clone(),
        batch.uv0.clone(),
        Vec::new(),
        batch.indices.clone(),
    )
}

fn indexed_mesh(mesh: &ReviewIndexedMeshV1, colors: Vec<[f32; 4]>) -> Result<Mesh, String> {
    mesh_from_parts(
        mesh.positions.clone(),
        mesh.normals.clone(),
        mesh.uvs.clone(),
        colors,
        mesh.indices.clone(),
    )
}

fn water_value_vertex_colors(
    colors: &[[f32; 4]],
    expected_vertices: usize,
    water_color: (f32, f32, f32),
) -> Result<Vec<[f32; 4]>, String> {
    let base = [water_color.0, water_color.1, water_color.2];
    if colors.len() != expected_vertices
        || base
            .iter()
            .any(|channel| !channel.is_finite() || !(0.0..=1.0).contains(channel))
    {
        return Err(
            "review water vertex colors must cover the full mesh and use a finite palette"
                .to_owned(),
        );
    }
    colors
        .iter()
        .map(|color| {
            let [value, green, blue, alpha] = *color;
            if !color
                .iter()
                .all(|value| value.is_finite() && (0.0..=1.0).contains(value))
                || value.to_bits() != green.to_bits()
                || green.to_bits() != blue.to_bits()
            {
                return Err(
                    "review water vertex colors must carry one finite value multiplier".to_owned(),
                );
            }
            let linear_ratio = |base_channel: f32| {
                let base_linear = srgb_channel_to_linear(base_channel);
                let target_linear = srgb_channel_to_linear((base_channel * value).clamp(0.0, 1.0));
                if base_linear <= f32::EPSILON {
                    1.0
                } else {
                    (target_linear / base_linear).clamp(0.0, 1.0)
                }
            };
            let [red, green, blue] = base;
            Ok([
                linear_ratio(red),
                linear_ratio(green),
                linear_ratio(blue),
                alpha,
            ])
        })
        .collect()
}

fn srgb_channel_to_linear(channel: f32) -> f32 {
    if channel <= 0.04045 {
        channel / 12.92
    } else {
        ((channel + 0.055) / 1.055).powf(2.4)
    }
}

fn mesh_from_parts(
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    colors: Vec<[f32; 4]>,
    indices: Vec<u32>,
) -> Result<Mesh, String> {
    if positions.is_empty()
        || positions.len() != normals.len()
        || positions.len() != uvs.len()
        || (!colors.is_empty() && positions.len() != colors.len())
        || !indices.len().is_multiple_of(3)
        || positions.iter().flatten().any(|value| !value.is_finite())
        || normals.iter().flatten().any(|value| !value.is_finite())
        || uvs.iter().flatten().any(|value| !value.is_finite())
        || colors
            .iter()
            .flatten()
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        || indices.iter().any(|index| match usize::try_from(*index) {
            Ok(index) => index >= positions.len(),
            Err(_) => true,
        })
    {
        return Err("review mesh streams are malformed or non-finite".to_owned());
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    if !colors.is_empty() {
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    }
    mesh.insert_indices(Indices::U32(indices));
    Ok(mesh)
}

#[expect(
    clippy::too_many_arguments,
    reason = "spawn helper reports exact asset/entity ownership explicitly"
)]
fn spawn_crown_dust(
    vegetation: &[ReviewVegetationProjectionV1],
    presentation: Option<&MapPresentationProjection>,
    art_catalog: Option<&RuntimeArtCatalog>,
    level_height: f32,
    state: &mut ReviewWorldDetailProjectionState,
    counts: &mut ReviewWorldDetailCountsV1,
    materials: &mut Assets<StandardMaterial>,
    meshes: &mut Assets<Mesh>,
    commands: &mut Commands,
    mesh_hash: &mut Vec<u8>,
) -> Result<(), String> {
    let Some(presentation) = presentation else {
        return Ok(());
    };
    let dust = vegetation
        .iter()
        .filter_map(|projection| projection.crown_dust.map(|dust| (projection, dust)))
        .collect::<Vec<_>>();
    if dust.is_empty() {
        return Ok(());
    }
    let art_catalog = art_catalog
        .ok_or_else(|| "crown-dust review requires the accepted runtime art catalog".to_owned())?;
    let dust_color = dust
        .first()
        .map(|(_, dust)| dust.color)
        .ok_or_else(|| "crown-dust selection was unexpectedly empty".to_owned())?;
    let [dust_red, dust_green, dust_blue, dust_alpha] = dust_color;
    let material = materials.add(StandardMaterial {
        base_color: Color::srgba(dust_red, dust_green, dust_blue, dust_alpha),
        perceptual_roughness: 0.5,
        cull_mode: None,
        double_sided: true,
        ..default()
    });
    state.materials.push(material.clone());
    counts.alpine_vegetation.materials += 1;
    let mut batches = BTreeMap::<(i32, i32), RawMesh>::new();
    for (projection, dust) in dust {
        let Ok(id) = u32::try_from(projection.stable_id) else {
            continue;
        };
        let Some(feature) = presentation
            .features()
            .iter()
            .find_map(|(feature_id, feature)| (feature_id.0 == id).then_some(feature))
        else {
            return Err(format!(
                "crown-dust projection references missing feature id {id}"
            ));
        };
        let blueprint = art_catalog.object(&feature.object_id).ok_or_else(|| {
            format!(
                "crown-dust feature {id} references unavailable blueprint '{}'",
                feature.object_id
            )
        })?;
        if blueprint.canopy_occluders.is_empty() {
            return Err(format!(
                "crown-dust feature {id} blueprint '{}' has no canopy mask",
                feature.object_id
            ));
        }
        let minimum_level = blueprint
            .canopy_occluders
            .iter()
            .map(|position| position.level)
            .min()
            .ok_or_else(|| "crown-dust canopy lost its minimum level".to_owned())?;
        let maximum_level = blueprint
            .canopy_occluders
            .iter()
            .map(|position| position.level)
            .max()
            .ok_or_else(|| "crown-dust canopy lost its maximum level".to_owned())?;
        let level_count = maximum_level
            .saturating_sub(minimum_level)
            .saturating_add(1);
        let retained_levels = if dust.upper_fraction <= 0.25 + f32::EPSILON {
            level_count.saturating_add(3) / 4
        } else {
            level_count.saturating_add(1) / 2
        }
        .max(1);
        let minimum_dusted_level = maximum_level
            .saturating_add(1)
            .saturating_sub(retained_levels);
        let occupied = blueprint
            .placements
            .iter()
            .map(|placement| placement.position)
            .collect::<BTreeSet<_>>();
        let root_world = feature.root.coord.to_world(0.0);
        for canopy in blueprint
            .canopy_occluders
            .iter()
            .copied()
            .filter(|position| position.level >= minimum_dusted_level)
            .filter(|position| {
                !occupied.contains(&hex_assets::LocalVoxelCoord::new(
                    position.q,
                    position.r,
                    position.level.saturating_add(1),
                ))
            })
        {
            let rotated = feature
                .rotation
                .rotate_voxel(canopy, blueprint.origin)
                .ok_or_else(|| format!("crown-dust rotation overflow for feature {id}"))?;
            let relative_q = rotated.q.saturating_sub(blueprint.origin.q);
            let relative_r = rotated.r.saturating_sub(blueprint.origin.r);
            let world_q = feature
                .root
                .coord
                .x()
                .checked_add(relative_q)
                .ok_or_else(|| format!("crown-dust q coordinate overflow for feature {id}"))?;
            let world_r = feature
                .root
                .coord
                .y()
                .checked_add(relative_r)
                .ok_or_else(|| format!("crown-dust r coordinate overflow for feature {id}"))?;
            let world_coord = HexCoord::from_axial(world_q, world_r);
            let unscaled_world = world_coord.to_world(0.0);
            let [horizontal_scale, vertical_scale, _] = projection.render_child_scale;
            let mut centre = root_world + (unscaled_world - root_world) * horizontal_scale;
            let relative_level = rotated.level.saturating_sub(blueprint.origin.level);
            // Coat the existing canopy top with an attached volumetric shell.
            // The requested 0.02/0.04 value is full world-space thickness, not
            // a vertical gap beneath a zero-thickness duplicate cap.
            centre.y = canopy_voxel_top_y(
                feature.root.level,
                relative_level,
                level_height,
                vertical_scale,
            );
            append_hex_upper_shell(
                batches.entry(terrain_chunk_key(world_coord)).or_default(),
                centre,
                0.84 * horizontal_scale,
                dust.shell_height,
            )?;
        }
    }
    for ((q, r), raw) in batches {
        let vertices = bounded_u64(raw.positions.len());
        let triangles = bounded_u64(raw.indices.len() / 3);
        append_mesh_stream_hash(
            mesh_hash,
            &raw.positions,
            &raw.normals,
            &raw.uvs,
            &raw.colors,
            &raw.indices,
        );
        let mesh = mesh_from_parts(raw.positions, raw.normals, raw.uvs, raw.colors, raw.indices)?;
        let handle = meshes.add(mesh);
        state.meshes.push(handle.clone());
        let entity = commands
            .spawn((
                Mesh3d(handle),
                MeshMaterial3d(material.clone()),
                Transform::IDENTITY,
                Visibility::Inherited,
                Pickable::IGNORE,
                NotShadowCaster,
                ReviewWorldDetailEntity,
                Name::new(format!("ReviewCrownDust[{q},{r}]")),
            ))
            .id();
        state.entities.push(entity);
        counts.alpine_vegetation.entities += 1;
        counts.alpine_vegetation.vertices += vertices;
        counts.alpine_vegetation.triangles += triangles;
    }
    Ok(())
}

#[expect(
    clippy::cast_precision_loss,
    reason = "authored object levels are bounded and exactly represented by f32"
)]
fn canopy_voxel_top_y(
    root_level: i32,
    relative_level: i32,
    level_height: f32,
    vertical_scale: f32,
) -> f32 {
    let root_voxel_center = surface_y(root_level, level_height) + level_height * 0.5;
    root_voxel_center + (relative_level as f32 + 0.5) * level_height * vertical_scale
}

#[derive(Default)]
struct RawMesh {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    colors: Vec<[f32; 4]>,
    indices: Vec<u32>,
}

fn cloud_shadow_material() -> StandardMaterial {
    StandardMaterial {
        // Per-vertex alpha carries the exact [0, 0.20] radial response. Keeping
        // the shared material fully opaque here avoids a second alpha scale.
        base_color: Color::srgba(0.03, 0.04, 0.07, 1.0),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        double_sided: true,
        cull_mode: None,
        ..default()
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "spawn helper reports exact asset/entity ownership explicitly"
)]
fn spawn_spray(
    sprays: &[crate::review_world_detail_effects::ReviewSprayVolumeV1],
    state: &mut ReviewWorldDetailProjectionState,
    counts: &mut ReviewWorldDetailCountsV1,
    materials: &mut Assets<StandardMaterial>,
    meshes: &mut Assets<Mesh>,
    commands: &mut Commands,
    mesh_hash: &mut Vec<u8>,
) -> Result<(), String> {
    if sprays.is_empty() {
        return Ok(());
    }
    let alpha = sprays.iter().map(|spray| spray.opacity).fold(0.0, f32::max);
    let material = materials.add(StandardMaterial {
        base_color: Color::srgba(0.88, 0.96, 1.0, alpha),
        emissive: LinearRgba::new(0.08, 0.10, 0.12, 1.0),
        perceptual_roughness: 0.95,
        alpha_mode: AlphaMode::Blend,
        cull_mode: None,
        double_sided: true,
        ..default()
    });
    state.materials.push(material.clone());
    counts.shore_and_falls.materials += 1;
    let mut raw = RawMesh::default();
    for spray in sprays {
        append_octahedron(&mut raw, spray.center, spray.radius, spray.height * 0.5)?;
    }
    let vertices = bounded_u64(raw.positions.len());
    let triangles = bounded_u64(raw.indices.len() / 3);
    append_mesh_stream_hash(
        mesh_hash,
        &raw.positions,
        &raw.normals,
        &raw.uvs,
        &raw.colors,
        &raw.indices,
    );
    let mesh = mesh_from_parts(raw.positions, raw.normals, raw.uvs, raw.colors, raw.indices)?;
    let handle = meshes.add(mesh);
    state.meshes.push(handle.clone());
    let entity = commands
        .spawn((
            Mesh3d(handle),
            MeshMaterial3d(material),
            Transform::IDENTITY,
            Visibility::Inherited,
            Pickable::IGNORE,
            NotShadowCaster,
            ReviewWorldDetailEntity,
            Name::new("ReviewPlungeSpray"),
        ))
        .id();
    state.entities.push(entity);
    counts.shore_and_falls.entities += 1;
    counts.shore_and_falls.vertices += vertices;
    counts.shore_and_falls.triangles += triangles;
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "spawn helper reports exact asset/entity ownership explicitly"
)]
fn spawn_cloud_shadows(
    shadows: &[crate::review_world_detail_effects::ReviewCloudShadowV1],
    natural: &[NaturalSurface],
    level_height: f32,
    state: &mut ReviewWorldDetailProjectionState,
    counts: &mut ReviewWorldDetailCountsV1,
    materials: &mut Assets<StandardMaterial>,
    meshes: &mut Assets<Mesh>,
    commands: &mut Commands,
    mesh_hash: &mut Vec<u8>,
) -> Result<(), String> {
    if shadows.is_empty() {
        return Ok(());
    }
    let material = materials.add(cloud_shadow_material());
    state.materials.push(material.clone());
    counts.physical_clouds.materials += 1;
    let mut raw_by_chunk = BTreeMap::<(i32, i32), RawMesh>::new();
    for surface in natural {
        let center = surface
            .position
            .coord
            .to_world(surface_y(surface.position.level, level_height) + REVIEW_SURFACE_BIAS);
        if !cloud_shadow_intersects_hex(center, shadows) {
            continue;
        }
        let (chunk_q, chunk_r) = terrain_chunk_key(surface.position.coord);
        append_cloud_shadow_hex_cap(
            raw_by_chunk.entry((chunk_q, chunk_r)).or_default(),
            center,
            shadows,
        )?;
    }
    for ((chunk_q, chunk_r), raw) in raw_by_chunk {
        if raw.indices.is_empty() {
            continue;
        }
        let vertices = bounded_u64(raw.positions.len());
        let triangles = bounded_u64(raw.indices.len() / 3);
        append_mesh_stream_hash(
            mesh_hash,
            &raw.positions,
            &raw.normals,
            &raw.uvs,
            &raw.colors,
            &raw.indices,
        );
        let mesh = mesh_from_parts(raw.positions, raw.normals, raw.uvs, raw.colors, raw.indices)?;
        let handle = meshes.add(mesh);
        state.meshes.push(handle.clone());
        let entity = commands
            .spawn((
                Mesh3d(handle),
                MeshMaterial3d(material.clone()),
                Transform::IDENTITY,
                Visibility::Inherited,
                Pickable::IGNORE,
                NotShadowCaster,
                ReviewWorldDetailEntity,
                Name::new(format!("ReviewCloudShadow[{chunk_q},{chunk_r}]")),
            ))
            .id();
        state.entities.push(entity);
        counts.physical_clouds.entities += 1;
        counts.physical_clouds.vertices += vertices;
        counts.physical_clouds.triangles += triangles;
    }
    Ok(())
}

fn cloud_shadow_intersects_hex(
    center: Vec3,
    shadows: &[crate::review_world_detail_effects::ReviewCloudShadowV1],
) -> bool {
    let center_xz = Vec2::new(center.x, center.z);
    shadows.iter().any(|shadow| {
        let support_radius = shadow.diameter * 0.5 + shadow.blur_world;
        center_xz.distance(shadow.center_xz) <= support_radius + hex_core::config::HEX_CIRCUMRADIUS
    })
}

fn cloud_shadow_opacity(
    position: Vec3,
    shadows: &[crate::review_world_detail_effects::ReviewCloudShadowV1],
) -> f32 {
    let position_xz = Vec2::new(position.x, position.z);
    shadows.iter().fold(0.0_f32, |current, shadow| {
        let distance = position_xz.distance(shadow.center_xz);
        let radius = shadow.diameter * 0.5;
        // `blur_world` is the complete radial soft-transition width outside the
        // projected cluster disk. Vertex interpolation keeps this response
        // continuous without opacity-band discontinuities or an early cutoff.
        let blur = shadow.blur_world.max(f32::EPSILON);
        let fade = ((radius + blur - distance) / blur).clamp(0.0, 1.0);
        current.max(shadow.maximum_opacity * fade * fade)
    })
}

fn spawn_fog(
    fogs: &[ReviewFogVolumeV1],
    state: &mut ReviewWorldDetailProjectionState,
    counts: &mut ReviewWorldDetailCountsV1,
    images: &mut Assets<Image>,
    commands: &mut Commands,
) -> Result<(), String> {
    let mut density_textures = BTreeMap::<(u32, u32), Handle<Image>>::new();
    for fog in fogs {
        let texture_key = (fog.edge_softness.to_bits(), fog.coverage.to_bits());
        let texture = if let Some(handle) = density_textures.get(&texture_key) {
            handle.clone()
        } else {
            let image = fog_density_image(fog.edge_softness, fog.coverage)?;
            let handle = images.add(image);
            state.images.push(handle.clone());
            density_textures.insert(texture_key, handle.clone());
            handle
        };
        let entity = commands
            .spawn((
                FogVolume {
                    fog_color: Color::srgb(0.78, 0.84, 0.90),
                    density_factor: fog.density,
                    density_texture: Some(texture),
                    // These sum to one, so the planner's
                    // -ln(1-opacity)/height calibration is exact on the
                    // full-density centre ray.
                    absorption: REVIEW_FOG_ABSORPTION,
                    scattering: REVIEW_FOG_SCATTERING,
                    scattering_asymmetry: 0.32,
                    ..default()
                },
                Transform::from_translation(fog.center).with_scale(fog.half_extents * 2.0),
                Visibility::Inherited,
                Pickable::IGNORE,
                ReviewWorldDetailEntity,
                ReviewWorldDetailFog,
                Name::new(format!("ReviewLocalFog[{}]", fog.anchor_name)),
            ))
            .id();
        state.entities.push(entity);
        counts.local_fog.entities += 1;
    }
    Ok(())
}

#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "small fixed texture coordinates and clamped normalized density quantize deliberately to R8"
)]
fn fog_density_image(edge_softness: f32, coverage: f32) -> Result<Image, String> {
    const HEIGHT: u32 = 16;
    if !edge_softness.is_finite() || !(0.0..=1.0).contains(&edge_softness) {
        return Err("fog edge softness must be finite and in [0,1]".to_owned());
    }
    let (occupancy, _sample_count, _active_samples) = fog_density_xz_mask(coverage)
        .ok_or_else(|| "fog coverage must be finite and in [0,1]".to_owned())?;
    let capacity = usize::try_from(FOG_DENSITY_WIDTH * HEIGHT * FOG_DENSITY_DEPTH)
        .map_err(|_error| "fog density image capacity exceeds usize".to_owned())?;
    let mut data = Vec::with_capacity(capacity);
    for z in 0..FOG_DENSITY_DEPTH {
        let nz = z as f32 / (FOG_DENSITY_DEPTH - 1) as f32 * 2.0 - 1.0;
        for _y in 0..HEIGHT {
            for x in 0..FOG_DENSITY_WIDTH {
                let nx = x as f32 / (FOG_DENSITY_WIDTH - 1) as f32 * 2.0 - 1.0;
                let radial_distance = (nx * nx + nz * nz).sqrt();
                // Preserve homogeneous density over the full requested height so
                // `-ln(1-opacity) / height` remains the exact centre-ray
                // calibration. The finite FogVolume bounds clip the ray at top
                // and bottom; this texture supplies the requested soft lateral
                // edge without silently reducing integrated opacity.
                let occupancy_index = usize::try_from(z * FOG_DENSITY_WIDTH + x)
                    .map_err(|_error| "fog occupancy index exceeds usize".to_owned())?;
                let density = if occupancy.get(occupancy_index).copied().unwrap_or(false) {
                    fog_edge_weight(1.0 - radial_distance, edge_softness)
                } else {
                    0.0
                };
                data.push((density * 255.0).round() as u8);
            }
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: FOG_DENSITY_WIDTH,
            height: HEIGHT,
            depth_or_array_layers: FOG_DENSITY_DEPTH,
        },
        TextureDimension::D3,
        data,
        TextureFormat::R8Unorm,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        address_mode_w: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        ..default()
    });
    Ok(image)
}

fn fog_edge_weight(distance_to_boundary: f32, edge_softness: f32) -> f32 {
    if edge_softness <= f32::EPSILON {
        return if distance_to_boundary >= 0.0 {
            1.0
        } else {
            0.0
        };
    }
    let linear = (distance_to_boundary / edge_softness).clamp(0.0, 1.0);
    linear * linear * (3.0 - 2.0 * linear)
}

fn append_octahedron(
    mesh: &mut RawMesh,
    centre: Vec3,
    radius: f32,
    half_height: f32,
) -> Result<(), String> {
    let top = centre + Vec3::Y * half_height;
    let bottom = centre - Vec3::Y * half_height;
    let ring = [
        centre + Vec3::X * radius,
        centre + Vec3::Z * radius,
        centre - Vec3::X * radius,
        centre - Vec3::Z * radius,
    ];
    let [east, south, west, north] = ring;
    for [current, next] in [[east, south], [south, west], [west, north], [north, east]] {
        append_triangle(mesh, [top, next, current])?;
        append_triangle(mesh, [bottom, current, next])?;
    }
    Ok(())
}

#[expect(
    clippy::cast_precision_loss,
    reason = "the six compile-time hex corner indices are exactly representable by f32"
)]
fn append_cloud_shadow_hex_cap(
    mesh: &mut RawMesh,
    centre: Vec3,
    shadows: &[crate::review_world_detail_effects::ReviewCloudShadowV1],
) -> Result<(), String> {
    let radius = hex_core::config::HEX_CIRCUMRADIUS;
    let corners = std::array::from_fn(|index| {
        let angle = std::f32::consts::TAU * index as f32 / 6.0 - std::f32::consts::FRAC_PI_2;
        centre + Vec3::new(angle.cos() * radius, 0.0, angle.sin() * radius)
    });
    let [north, north_east, south_east, south, south_west, north_west] = corners;
    for [current, next] in [
        [north, north_east],
        [north_east, south_east],
        [south_east, south],
        [south, south_west],
        [south_west, north_west],
        [north_west, north],
    ] {
        let triangle = [centre, next, current];
        let colors =
            triangle.map(|position| [1.0, 1.0, 1.0, cloud_shadow_opacity(position, shadows)]);
        append_triangle_with_colors(mesh, triangle, colors)?;
    }
    Ok(())
}

#[expect(
    clippy::cast_precision_loss,
    reason = "the six compile-time hex corner indices are exactly representable by f32"
)]
fn append_hex_cap(mesh: &mut RawMesh, centre: Vec3, radius_scale: f32) -> Result<(), String> {
    let radius = hex_core::config::HEX_CIRCUMRADIUS * radius_scale;
    let corners = std::array::from_fn(|index| {
        let angle = std::f32::consts::TAU * index as f32 / 6.0 - std::f32::consts::FRAC_PI_2;
        centre + Vec3::new(angle.cos() * radius, 0.0, angle.sin() * radius)
    });
    let [north, north_east, south_east, south, south_west, north_west] = corners;
    for [current, next] in [
        [north, north_east],
        [north_east, south_east],
        [south_east, south],
        [south, south_west],
        [south_west, north_west],
        [north_west, north],
    ] {
        append_triangle(mesh, [centre, next, current])?;
    }
    Ok(())
}

#[expect(
    clippy::cast_precision_loss,
    reason = "the six compile-time hex corner indices are exactly representable by f32"
)]
fn append_hex_upper_shell(
    mesh: &mut RawMesh,
    bottom_centre: Vec3,
    radius_scale: f32,
    height: f32,
) -> Result<(), String> {
    if !height.is_finite() || height <= 0.0 {
        return Err("crown-dust shell height must be finite and positive".to_owned());
    }
    let radius = hex_core::config::HEX_CIRCUMRADIUS * radius_scale;
    let bottom = std::array::from_fn(|index| {
        let angle = std::f32::consts::TAU * index as f32 / 6.0 - std::f32::consts::FRAC_PI_2;
        bottom_centre + Vec3::new(angle.cos() * radius, 0.0, angle.sin() * radius)
    });
    let top_centre = bottom_centre + Vec3::Y * height;
    let top = bottom.map(|corner| corner + Vec3::Y * height);
    append_hex_cap(mesh, top_centre, radius_scale)?;
    let [bottom_north, bottom_north_east, bottom_south_east, bottom_south, bottom_south_west, bottom_north_west] =
        bottom;
    let [top_north, top_north_east, top_south_east, top_south, top_south_west, top_north_west] =
        top;
    for [current_bottom, next_bottom, current_top, next_top] in [
        [bottom_north, bottom_north_east, top_north, top_north_east],
        [
            bottom_north_east,
            bottom_south_east,
            top_north_east,
            top_south_east,
        ],
        [bottom_south_east, bottom_south, top_south_east, top_south],
        [bottom_south, bottom_south_west, top_south, top_south_west],
        [
            bottom_south_west,
            bottom_north_west,
            top_south_west,
            top_north_west,
        ],
        [bottom_north_west, bottom_north, top_north_west, top_north],
    ] {
        append_triangle(mesh, [current_bottom, current_top, next_top])?;
        append_triangle(mesh, [current_bottom, next_top, next_bottom])?;
    }
    Ok(())
}

fn append_triangle(mesh: &mut RawMesh, triangle: [Vec3; 3]) -> Result<(), String> {
    append_triangle_inner(mesh, triangle, None)
}

fn append_triangle_with_colors(
    mesh: &mut RawMesh,
    triangle: [Vec3; 3],
    colors: [[f32; 4]; 3],
) -> Result<(), String> {
    append_triangle_inner(mesh, triangle, Some(colors))
}

fn append_triangle_inner(
    mesh: &mut RawMesh,
    triangle: [Vec3; 3],
    colors: Option<[[f32; 4]; 3]>,
) -> Result<(), String> {
    let [first, second, third] = triangle;
    let normal = (second - first).cross(third - first);
    if !normal.is_finite() || normal.length_squared() <= f32::EPSILON {
        return Err("generated review triangle is degenerate".to_owned());
    }
    let normal = normal.normalize().to_array();
    let start = u32::try_from(mesh.positions.len())
        .map_err(|_error| "review mesh index capacity exceeded".to_owned())?;
    let vertex_start = mesh.positions.len();
    mesh.positions
        .extend([first, second, third].map(|position| position.to_array()));
    mesh.normals.extend([normal; 3]);
    mesh.uvs.extend([[0.5, 0.5], [0.0, 0.0], [1.0, 0.0]]);
    match colors {
        Some(colors) => {
            if mesh.colors.is_empty() && vertex_start > 0 {
                mesh.colors.resize(vertex_start, [1.0; 4]);
            }
            mesh.colors.extend(colors);
        }
        None if !mesh.colors.is_empty() => mesh.colors.extend([[1.0; 4]; 3]),
        None => {}
    }
    mesh.indices.extend([start, start + 1, start + 2]);
    Ok(())
}

fn authority_fingerprints(
    map: &VoxelMap,
    table: &SubstanceTable,
    presentation: Option<&MapPresentationProjection>,
    anchors: &MapAnchors,
    observation: Option<&MapObservationAnchors>,
    blockers: Option<&TraversalBlockers>,
    biomes: Option<&BiomeRegions>,
    generation: Option<&GenerationReport>,
    level_height: f32,
) -> Result<ReviewAuthorityFingerprintsV1, String> {
    let presentation = presentation.ok_or_else(|| {
        "runtime review report requires the authoritative presentation projection".to_owned()
    })?;
    let blockers = blockers.ok_or_else(|| {
        "runtime review report requires the authoritative traversal blockers".to_owned()
    })?;
    let biomes = biomes.ok_or_else(|| {
        "runtime review report requires the authoritative biome projection".to_owned()
    })?;
    let generation = generation.ok_or_else(|| {
        "runtime review report requires the authoritative generation report".to_owned()
    })?;
    let structural = require_structural_fingerprint(generation.semantic_plan_fingerprint)?;
    let voxel = hash_voxel_map(map);
    let topology = hash_topology(map);
    let traversal = hash_traversal(map, table, blockers);
    let logical = hash_logical_runs(map, level_height);
    Ok(ReviewAuthorityFingerprintsV1 {
        voxel_map: hex64(voxel),
        structural: hex64(structural),
        materialized: hex64(generation.map_fingerprint),
        liquid_graph: hex64(hash_liquids(presentation)),
        topology: hex64(topology),
        traversal: hex64(traversal),
        blockers: hex64(hash_positions(blockers.iter())),
        anchors: hex64(hash_anchors(anchors, observation)),
        biomes: hex64(hash_biomes(biomes)),
        feature_roots: hex64(hash_features(presentation)),
        logical_terrain_picking: hex64(logical),
        gameplay_state: hex64(generation.map_fingerprint),
    })
}

fn require_structural_fingerprint(fingerprint: Option<u64>) -> Result<u64, String> {
    fingerprint.ok_or_else(|| {
        "runtime review report requires the Grand V3 structural-plan fingerprint".to_owned()
    })
}

fn hash_voxel_map(map: &VoxelMap) -> u64 {
    let mut bytes = b"review-voxel-map-v1".to_vec();
    for (coord, column) in map.columns() {
        push_coord(&mut bytes, coord);
        bytes.extend_from_slice(&column.top().to_le_bytes());
        for substance in column.iter() {
            bytes.extend_from_slice(&substance.0.to_le_bytes());
        }
    }
    xxh3_64(&bytes)
}

fn hash_topology(map: &VoxelMap) -> u64 {
    let mut bytes = b"review-topology-v1".to_vec();
    for (coord, column) in map.columns() {
        push_coord(&mut bytes, coord);
        for run in runs(column) {
            bytes.extend_from_slice(&run.bottom.to_le_bytes());
            bytes.extend_from_slice(&run.top.to_le_bytes());
        }
    }
    xxh3_64(&bytes)
}

fn hash_traversal(map: &VoxelMap, table: &SubstanceTable, blockers: &TraversalBlockers) -> u64 {
    let mut bytes = b"review-traversal-v1".to_vec();
    for (coord, column) in map.columns() {
        for run in runs(column) {
            let position = TilePos::new(coord, run.top.saturating_sub(1));
            push_pos(&mut bytes, position);
            bytes.push(u8::from(table.is_solid(run.substance)));
            bytes.extend_from_slice(&column.headroom_above(run.top).0.to_le_bytes());
            bytes.push(u8::from(blockers.contains(position)));
        }
    }
    xxh3_64(&bytes)
}

#[expect(
    clippy::cast_precision_loss,
    reason = "review voxel levels remain far below the exact f32 integer range"
)]
fn hash_logical_runs(map: &VoxelMap, level_height: f32) -> u64 {
    let mut bytes = b"review-logical-picking-v1".to_vec();
    for (coord, column) in map.columns() {
        for run in runs(column) {
            push_pos(&mut bytes, TilePos::new(coord, run.top.saturating_sub(1)));
            bytes.extend_from_slice(&run.bottom.to_le_bytes());
            bytes.extend_from_slice(&run.top.to_le_bytes());
            bytes.extend_from_slice(&run.substance.0.to_le_bytes());
            bytes.extend_from_slice(&(run.bottom as f32 * level_height).to_bits().to_le_bytes());
            bytes.extend_from_slice(&(run.top as f32 * level_height).to_bits().to_le_bytes());
            bytes.extend_from_slice(&column.headroom_above(run.top).0.to_le_bytes());
        }
    }
    xxh3_64(&bytes)
}

fn hash_liquids(presentation: &MapPresentationProjection) -> u64 {
    let mut bytes = b"review-liquid-graph-v1".to_vec();
    for (position, liquid) in presentation.liquids() {
        push_pos(&mut bytes, *position);
        bytes.push(match liquid.material {
            FillMaterialRole::Water => 0,
            FillMaterialRole::Lava => 1,
        });
        bytes.push(match liquid.flow {
            LiquidFlowState::Still => 0,
            LiquidFlowState::Current => 1,
            LiquidFlowState::Rapid => 2,
            LiquidFlowState::Fall => 3,
        });
        if let Some(downstream) = liquid.downstream {
            bytes.push(1);
            push_pos(&mut bytes, downstream);
        } else {
            bytes.push(0);
        }
    }
    xxh3_64(&bytes)
}

fn hash_anchors(anchors: &MapAnchors, observation: Option<&MapObservationAnchors>) -> u64 {
    let mut rows = anchors
        .iter()
        .map(|(id, position)| (id.as_str().to_owned(), 0_u8, position))
        .chain(
            observation
                .into_iter()
                .flat_map(MapObservationAnchors::iter)
                .map(|(id, position)| (id.as_str().to_owned(), 1_u8, position)),
        )
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| left.0.cmp(&right.0));
    let mut bytes = b"review-anchors-v1".to_vec();
    for (name, class, position) in rows {
        bytes.extend_from_slice(name.as_bytes());
        bytes.push(0);
        bytes.push(class);
        push_pos(&mut bytes, position);
    }
    xxh3_64(&bytes)
}

fn hash_biomes(biomes: &BiomeRegions) -> u64 {
    let mut bytes = b"review-biomes-v1".to_vec();
    for (position, region) in biomes.iter() {
        push_pos(&mut bytes, position);
        bytes.extend_from_slice(&region.0.to_le_bytes());
    }
    xxh3_64(&bytes)
}

fn hash_features(presentation: &MapPresentationProjection) -> u64 {
    let mut bytes = b"review-features-v1".to_vec();
    for (id, feature) in presentation.features() {
        bytes.extend_from_slice(&id.0.to_le_bytes());
        push_pos(&mut bytes, feature.root);
        bytes.push(match feature.kind {
            FeatureKind::Tree => 0,
            FeatureKind::TallGrass => 1,
            FeatureKind::CaveVegetation => 2,
        });
        bytes.extend_from_slice(feature.object_id.as_str().as_bytes());
        bytes.push(0);
        bytes.push(feature.rotation.steps());
    }
    xxh3_64(&bytes)
}

fn hash_positions(positions: impl IntoIterator<Item = TilePos>) -> u64 {
    let mut bytes = b"review-positions-v1".to_vec();
    for position in positions {
        push_pos(&mut bytes, position);
    }
    xxh3_64(&bytes)
}

fn push_coord(bytes: &mut Vec<u8>, coord: HexCoord) {
    bytes.extend_from_slice(&coord.x().to_le_bytes());
    bytes.extend_from_slice(&coord.y().to_le_bytes());
}

fn push_pos(bytes: &mut Vec<u8>, position: TilePos) {
    push_coord(bytes, position.coord);
    bytes.extend_from_slice(&position.level.to_le_bytes());
}

fn append_mesh_stream_hash(
    bytes: &mut Vec<u8>,
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    uvs: &[[f32; 2]],
    colors: &[[f32; 4]],
    indices: &[u32],
) {
    for length in [
        positions.len(),
        normals.len(),
        uvs.len(),
        colors.len(),
        indices.len(),
    ] {
        bytes.extend_from_slice(&u64::try_from(length).unwrap_or(u64::MAX).to_le_bytes());
    }
    for value in positions
        .iter()
        .flatten()
        .chain(normals.iter().flatten())
        .chain(uvs.iter().flatten())
        .chain(colors.iter().flatten())
    {
        bytes.extend_from_slice(&value.to_bits().to_le_bytes());
    }
    for index in indices {
        bytes.extend_from_slice(&index.to_le_bytes());
    }
}

fn hex64(value: u64) -> String {
    format!("{value:016x}")
}

fn bounded_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn phase_bound_effect_plan_hash(phase_neutral_hash: u64, phase_seconds: f32) -> Option<String> {
    LiquidAtmosphereReviewPlanV1::bind_phase_hash(phase_neutral_hash, phase_seconds).map(hex64)
}

#[expect(
    clippy::cast_precision_loss,
    reason = "review map levels are far below the exact f32 integer range"
)]
fn surface_y(level: i32, level_height: f32) -> f32 {
    level.saturating_add(1) as f32 * level_height
}

#[cfg(test)]
mod tests {
    use bevy::asset::AssetPlugin;
    use bevy::ecs::world::CommandQueue;
    use bevy::platform::collections::HashMap;
    use bevy::state::app::StatesPlugin;
    use hex_assets::{ArtPalette, PaletteSwatch, SrgbColor, Substance, SubstanceFile, SwatchId};

    use super::*;
    use crate::settings::{PerlinSettings, TerrainSettings};

    fn material_test_table(include_effect_palette: bool) -> (SubstanceTable, SubstanceId) {
        let stone_swatch_id =
            SwatchId::new("terrain/review-stone").expect("fixture swatch id should be valid");
        let stone_swatch = PaletteSwatch::new(
            "Review Stone",
            SrgbColor::new(0.25, 0.50, 0.75).expect("fixture color should be valid"),
            BTreeSet::from(["test".to_owned()]),
        )
        .expect("fixture swatch should be valid");
        let snow_swatch_id =
            SwatchId::new("terrain/review-snow").expect("fixture swatch id should be valid");
        let snow_swatch = PaletteSwatch::new(
            "Review Snow",
            SrgbColor::new(0.90, 0.94, 0.98).expect("fixture color should be valid"),
            BTreeSet::from(["test".to_owned()]),
        )
        .expect("fixture swatch should be valid");
        let mut swatches = BTreeMap::from([
            (stone_swatch_id.clone(), stone_swatch),
            (snow_swatch_id.clone(), snow_swatch),
        ]);
        let mut substances = HashMap::from_iter([
            ("air".to_owned(), Substance::invisible(false, false)),
            (
                "stone".to_owned(),
                Substance::from_swatch(stone_swatch_id, true, true),
            ),
            (
                "snow".to_owned(),
                Substance::from_swatch(snow_swatch_id, true, true),
            ),
        ]);
        if include_effect_palette {
            let foam_swatch_id =
                SwatchId::new("liquid/foam").expect("fixture foam swatch id should be valid");
            swatches.insert(
                foam_swatch_id,
                PaletteSwatch::new(
                    "Review Foam",
                    SrgbColor::new(0.93, 0.99, 1.0).expect("fixture color should be valid"),
                    BTreeSet::from(["test".to_owned()]),
                )
                .expect("fixture swatch should be valid"),
            );
            let ice_swatch_id =
                SwatchId::new("terrain/ice").expect("fixture ice swatch id should be valid");
            swatches.insert(
                ice_swatch_id.clone(),
                PaletteSwatch::new(
                    "Review Ice",
                    SrgbColor::new(0.33, 0.82, 0.98).expect("fixture color should be valid"),
                    BTreeSet::from(["test".to_owned()]),
                )
                .expect("fixture swatch should be valid"),
            );
            substances.insert(
                "ice".to_owned(),
                Substance::from_swatch(ice_swatch_id, true, true),
            );
        }
        let palette = ArtPalette::new(swatches).expect("fixture palette should be valid");
        let table = SubstanceTable::from_file(&SubstanceFile { substances }, &palette)
            .expect("fixture substances should resolve through their palette");
        let stone = table.id("stone").expect("fixture stone should resolve");
        (table, stone)
    }

    #[test]
    fn garden_mask_blocks_high_altitude_vegetation_snow_dust() {
        let (_table, stone) = material_test_table(false);
        let coord = HexCoord::ORIGIN;
        let root = TilePos::new(coord, 151);
        let surface = NaturalSurface {
            position: root,
            run_bottom: 151,
            solid_stack_bottom: 0,
            substance: stone,
            current_snow: true,
            exception: ReviewSnowExceptionV1::Garden,
            forced_summit: false,
            excluded: ReviewPropExclusionsV1::default(),
        };
        assert!(!vegetation_snow_dust_eligible(
            root,
            &BTreeSet::from([coord]),
            std::slice::from_ref(&surface),
        ));
        assert!(vegetation_snow_dust_eligible(
            root,
            &BTreeSet::new(),
            std::slice::from_ref(&surface),
        ));
    }

    #[test]
    fn cliff_layers_reach_through_a_thin_snow_cap_and_resolve_its_substrate() {
        let (table, stone) = material_test_table(false);
        let snow = table.id("snow").expect("fixture snow should resolve");
        let coord = HexCoord::ORIGIN;
        let mut map = VoxelMap::new();
        for level in 0..9 {
            map.set(TilePos::new(coord, level), stone);
        }
        map.set(TilePos::new(coord, 9), snow);
        let column = map.column(coord).expect("fixture column should exist");
        assert_eq!(contiguous_solid_stack_bottom(column, &table, 9), 0);
        let surface = NaturalSurface {
            position: TilePos::new(coord, 9),
            run_bottom: 9,
            solid_stack_bottom: 0,
            substance: snow,
            current_snow: true,
            exception: ReviewSnowExceptionV1::None,
            forced_summit: false,
            excluded: ReviewPropExclusionsV1::default(),
        };
        let neighbor = coord
            .neighbors()
            .into_iter()
            .next()
            .expect("a hex should have a first neighbor");
        assert_eq!(exposed_side_bottom_level(&map, &surface, neighbor), 0);
        for level in 0..4 {
            map.set(TilePos::new(neighbor, level), stone);
        }
        assert_eq!(exposed_side_bottom_level(&map, &surface, neighbor), 4);
        let layers = cliff_layers_for_surface(&map, &table, &surface)
            .expect("thin snow cap should resolve exact vertical cliff layers");
        assert_eq!(layers.len(), 2);
        let stone_layer = layers.first().expect("stone layer should exist");
        let snow_layer = layers.last().expect("snow layer should exist");
        assert_eq!((stone_layer.bottom_level, stone_layer.top_level), (0, 9));
        assert_eq!((snow_layer.bottom_level, snow_layer.top_level), (9, 10));
        assert!(layers.iter().all(|layer| layer.substrate == stone));
    }

    #[test]
    fn wet_rim_substrate_tracks_resolved_snow_addition_and_removal() {
        let (table, stone) = material_test_table(true);
        let snow = table.id("snow").expect("fixture snow should resolve");
        let coord = HexCoord::ORIGIN;
        let mut map = VoxelMap::new();
        map.set(TilePos::new(coord, 0), stone);
        map.set(TilePos::new(coord, 1), snow);

        let exposed_stone = NaturalSurface {
            position: TilePos::new(coord, 0),
            run_bottom: 0,
            solid_stack_bottom: 0,
            substance: stone,
            current_snow: false,
            exception: ReviewSnowExceptionV1::None,
            forced_summit: false,
            excluded: ReviewPropExclusionsV1::default(),
        };
        assert_eq!(
            resolved_shore_substance(&map, &table, &exposed_stone, true)
                .expect("new review snow should become the visible wet-rim substrate"),
            snow
        );

        let exposed_snow = NaturalSurface {
            position: TilePos::new(coord, 1),
            run_bottom: 1,
            solid_stack_bottom: 0,
            substance: snow,
            current_snow: true,
            exception: ReviewSnowExceptionV1::None,
            forced_summit: false,
            excluded: ReviewPropExclusionsV1::default(),
        };
        assert_eq!(
            resolved_shore_substance(&map, &table, &exposed_snow, false)
                .expect("removed review snow should reveal the underlying wet-rim substrate"),
            stone
        );
    }

    #[test]
    fn effect_input_preserves_the_exact_shore_solid_run() {
        let (table, stone) = material_test_table(false);
        let coord = HexCoord::ORIGIN;
        let physical_only_coord = HexCoord::new_cubic(1, 0, -1);
        let mut map = VoxelMap::new();
        for level in 3..=7 {
            map.set(TilePos::new(coord, level), stone);
        }
        for level in 1..=4 {
            map.set(TilePos::new(physical_only_coord, level), stone);
        }
        let surface = NaturalSurface {
            position: TilePos::new(coord, 7),
            // Model the canonical raised-bank shape: a one-voxel presentation
            // cap above a deeper contiguous solid stack.
            run_bottom: 7,
            solid_stack_bottom: 3,
            substance: stone,
            current_snow: false,
            exception: ReviewSnowExceptionV1::None,
            forced_summit: false,
            excluded: ReviewPropExclusionsV1::default(),
        };
        let settings = MapSettings {
            grid_radius: 4,
            level_height: 0.35,
            terrain: TerrainSettings::Perlin(PerlinSettings {
                seed: Some(7),
                steps: Vec::new(),
            }),
        };
        let massif = TilePos::new(HexCoord::new_cubic(1, -1, 0), 9);
        let anchors = BTreeMap::from([(
            "grand_v3.massif_crest".to_owned(),
            (massif, ReviewAnchorClassV1::Gameplay),
        )]);

        let input = build_effects_input(
            &map,
            &table,
            &settings,
            7,
            0.0,
            None,
            &[surface],
            &BTreeSet::new(),
            &anchors,
        )
        .expect("valid natural surface should become an exact shore interval");
        let [shore] = input.shore_surfaces.as_slice() else {
            panic!("one natural surface must produce one shore input");
        };
        assert_eq!(shore.position, TilePos::new(coord, 7));
        assert_eq!(shore.run_bottom, 3);
        assert!(input
            .physical_solid_runs
            .contains(&ReviewPhysicalSolidRunV1 {
                position: TilePos::new(physical_only_coord, 4),
                run_bottom: 1,
            }));
    }

    #[test]
    fn foam_and_ice_materials_preserve_current_palette_colors() {
        let (table, _) = material_test_table(true);
        let descriptor = |key, alpha, roughness, reflectance| ReviewMaterialDescriptorV1 {
            key,
            alpha: Some(alpha),
            value_multiplier: 1.0,
            roughness: Some(roughness),
            roughness_delta: None,
            reflectance: Some(reflectance),
            depth_half_distance: None,
            deep_value_multiplier: None,
            transmission: None,
            alpha_mode: ReviewAlphaModeV1::OrderIndependentTransparency,
            double_sided: true,
            inward_feather: None,
        };
        let foam = effect_material(
            &descriptor(ReviewMaterialKeyV1::Foam, 0.35, 0.75, 0.25),
            (0.1, 0.2, 0.3),
            &table,
        )
        .expect("foam palette swatch should resolve");
        let ice = effect_material(
            &descriptor(ReviewMaterialKeyV1::Ice, 0.82, 0.32, 0.30),
            (0.1, 0.2, 0.3),
            &table,
        )
        .expect("ice substance color should resolve");
        let foam_srgba = foam.base_color.to_srgba();
        let ice_srgba = ice.base_color.to_srgba();
        for (actual, expected) in [foam_srgba.red, foam_srgba.green, foam_srgba.blue]
            .into_iter()
            .zip([0.93, 0.99, 1.0])
        {
            assert!((actual - expected).abs() < 1.0e-6);
        }
        for (actual, expected) in [ice_srgba.red, ice_srgba.green, ice_srgba.blue]
            .into_iter()
            .zip([0.33, 0.82, 0.98])
        {
            assert!((actual - expected).abs() < 1.0e-6);
        }

        let (incomplete, _) = material_test_table(false);
        assert!(effect_material(
            &descriptor(ReviewMaterialKeyV1::Foam, 0.35, 0.75, 0.25),
            (0.1, 0.2, 0.3),
            &incomplete,
        )
        .is_err());
        assert!(effect_material(
            &descriptor(ReviewMaterialKeyV1::Ice, 0.82, 0.32, 0.30),
            (0.1, 0.2, 0.3),
            &incomplete,
        )
        .is_err());
    }

    #[test]
    fn cliff_shell_material_is_opaque_lit_baseline_roughness_and_shared_across_chunks() {
        let batch = ReviewTerrainMeshBatchV1 {
            chunk: (0, 0),
            material_role: ReviewTerrainMaterialRoleV1::CliffValue,
            substrate: Some(SubstanceId(4)),
            base_color: [0.376, 0.47, 0.564, 1.0],
            positions: vec![[0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 1.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            uv0: vec![[0.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
            indices: vec![0, 1, 2],
            source_items: 1,
        };
        let material = terrain_material(&batch);
        assert_eq!(material.alpha_mode, AlphaMode::Opaque);
        assert!(!material.unlit);
        assert_eq!(material.perceptual_roughness.to_bits(), 0.5_f32.to_bits());
        assert_eq!(material.depth_bias.to_bits(), 4.0_f32.to_bits());
        assert_eq!(material.cull_mode, Some(Face::Back));
        assert!(!material.double_sided);
        let color = material.base_color.to_srgba();
        for (actual, expected) in [color.red, color.green, color.blue, color.alpha]
            .into_iter()
            .zip(batch.base_color)
        {
            assert!((actual - expected).abs() < 1.0e-6);
        }

        let mut another_chunk = batch.clone();
        another_chunk.chunk = (9, -7);
        assert_eq!(
            terrain_material_key(&batch),
            terrain_material_key(&another_chunk)
        );
    }

    #[test]
    fn empty_raw_mesh_rejects_and_triangle_winding_is_finite() {
        assert!(
            mesh_from_parts(Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()).is_err()
        );
        let mut mesh = RawMesh::default();
        append_triangle(&mut mesh, [Vec3::ZERO, Vec3::X, Vec3::Z])
            .expect("nondegenerate triangle should be admitted");
        assert_eq!(mesh.indices, [0, 1, 2]);
        assert!(mesh.normals.iter().flatten().all(|value| value.is_finite()));
    }

    #[test]
    fn octahedron_faces_are_wound_outward() {
        let centre = Vec3::new(2.0, 3.0, 4.0);
        let mut mesh = RawMesh::default();
        append_octahedron(&mut mesh, centre, 2.0, 1.0)
            .expect("valid octahedron should be admitted");
        for (index, triangle) in mesh.positions.chunks_exact(3).enumerate() {
            let face_centre = triangle.iter().copied().map(Vec3::from_array).sum::<Vec3>() / 3.0;
            let normal = Vec3::from_array(
                *mesh
                    .normals
                    .get(index.saturating_mul(3))
                    .expect("every triangle retains its first normal"),
            );
            assert!(normal.dot(face_centre - centre) > 0.0);
        }
    }

    #[test]
    fn crown_dust_is_an_attached_volumetric_upper_shell() {
        let bottom = Vec3::new(2.0, 10.0, -3.0);
        let mut mesh = RawMesh::default();
        append_hex_upper_shell(&mut mesh, bottom, 0.84, 0.04)
            .expect("positive crown shell must build");

        assert_eq!(mesh.indices.len() / 3, 18);
        let minimum_y = mesh
            .positions
            .iter()
            .map(|position| {
                let [_, y, _] = *position;
                y
            })
            .fold(f32::INFINITY, f32::min);
        let maximum_y = mesh
            .positions
            .iter()
            .map(|position| {
                let [_, y, _] = *position;
                y
            })
            .fold(f32::NEG_INFINITY, f32::max);
        assert_eq!(minimum_y.to_bits(), bottom.y.to_bits());
        assert!((maximum_y - bottom.y - 0.04).abs() < 1.0e-6);
        assert!(mesh.normals.iter().flatten().all(|value| value.is_finite()));
    }

    #[test]
    fn crown_dust_bottom_is_the_scaled_canopy_voxel_top() {
        let height = 0.35;
        let unscaled_actual = canopy_voxel_top_y(100, 0, height, 1.0);
        let unscaled_expected = 102.0_f32 * height;
        assert!(
            (unscaled_actual - unscaled_expected).abs() <= f32::EPSILON * unscaled_expected.abs()
        );
        let actual = canopy_voxel_top_y(100, 4, height, 1.1);
        let expected = surface_y(100, height) + 0.5 * height + 4.5 * height * 1.1;
        assert!((actual - expected).abs() <= f32::EPSILON * expected.abs());
    }

    #[test]
    fn authority_hash_spelling_is_stable_lowercase_hex() {
        assert_eq!(hex64(0x1234_abcd), "000000001234abcd");
    }

    #[test]
    fn current_profile_builds_a_truly_empty_projection_without_grand_inputs() {
        let profile = ReviewWorldDetailProfileV1::default();
        let map = VoxelMap::new();
        let (table, _) = material_test_table(false);
        let settings = MapSettings {
            grid_radius: 4,
            level_height: 0.35,
            terrain: TerrainSettings::Perlin(PerlinSettings {
                seed: Some(7),
                steps: Vec::new(),
            }),
        };
        let anchors = MapAnchors::new();
        let mut materials = Assets::<StandardMaterial>::default();
        let mut review_water_materials = Assets::<ReviewWaterMaterial>::default();
        let mut meshes = Assets::<Mesh>::default();
        let mut images = Assets::<Image>::default();
        let mut world = World::new();
        let mut queue = CommandQueue::default();
        let built = {
            let mut commands = Commands::new(&mut queue, &world);
            build_review_projection(
                &profile,
                &map,
                &table,
                &settings,
                7,
                0.0,
                None,
                &anchors,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                &mut materials,
                &mut review_water_materials,
                &mut meshes,
                &mut images,
                &mut commands,
            )
            .expect("control must not require Grand anchors, surfaces, water, foam, or ice")
        };
        queue.apply(&mut world);

        let (state, report, hashes) = built;
        assert!(state.entities.is_empty());
        assert!(state.meshes.is_empty());
        assert!(state.images.is_empty());
        assert!(state.materials.is_empty());
        assert!(state.review_water_materials.is_empty());
        assert!(state.suppressed_terrain.is_empty());
        assert!(state.suppressed_liquids.is_empty());
        assert!(state.vegetation_treatments.is_empty());
        assert!(state.vegetation_original_scales.is_empty());
        assert!(report.is_none());
        assert_eq!(materials.len(), 0);
        assert_eq!(review_water_materials.len(), 0);
        assert_eq!(meshes.len(), 0);
        assert_eq!(images.len(), 0);
        assert!(hashes
            .terrain_plan
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
        assert_eq!(hashes.terrain_plan.len(), 16);
        assert_eq!(hashes.liquid_atmosphere_plan.len(), 16);
        assert_eq!(hashes.mesh_projection.len(), 16);
    }

    #[test]
    fn runtime_authority_fingerprints_reject_every_missing_required_source() {
        let map = VoxelMap::new();
        let (table, _) = material_test_table(false);
        let anchors = MapAnchors::new();
        let presentation = MapPresentationProjection::default();
        let blockers = TraversalBlockers::new();
        let biomes = BiomeRegions::new();

        let missing_presentation = authority_fingerprints(
            &map,
            &table,
            None,
            &anchors,
            None,
            Some(&blockers),
            Some(&biomes),
            None,
            0.35,
        )
        .expect_err("missing presentation authority must fail closed");
        assert!(missing_presentation.contains("presentation projection"));

        let missing_blockers = authority_fingerprints(
            &map,
            &table,
            Some(&presentation),
            &anchors,
            None,
            None,
            Some(&biomes),
            None,
            0.35,
        )
        .expect_err("missing blocker authority must fail closed");
        assert!(missing_blockers.contains("traversal blockers"));

        let missing_biomes = authority_fingerprints(
            &map,
            &table,
            Some(&presentation),
            &anchors,
            None,
            Some(&blockers),
            None,
            None,
            0.35,
        )
        .expect_err("missing biome authority must fail closed");
        assert!(missing_biomes.contains("biome projection"));

        let missing_generation = authority_fingerprints(
            &map,
            &table,
            Some(&presentation),
            &anchors,
            None,
            Some(&blockers),
            Some(&biomes),
            None,
            0.35,
        )
        .expect_err("missing generation authority must fail closed");
        assert!(missing_generation.contains("generation report"));
        assert!(require_structural_fingerprint(None).is_err());
    }

    #[test]
    fn fog_classification_whitelists_only_published_wet_terrain_anchors() {
        for name in [
            "grand_v3.waterfall_crown",
            "grand_v3.waterfall_base",
            "grand_v3.waterfall_profile",
        ] {
            assert_eq!(
                review_effect_anchor_kind(name),
                Some(ReviewEffectAnchorKindV1::Waterfall)
            );
        }
        assert_eq!(review_effect_anchor_kind("grand_v3.valley_bridge"), None);
        assert_eq!(
            review_effect_anchor_kind("grand_v3.valley_lake"),
            Some(ReviewEffectAnchorKindV1::Valley)
        );
        assert_eq!(review_effect_anchor_kind("grand_v3.coastal_bridge"), None);
        assert_eq!(
            review_effect_anchor_kind("grand_v3.mountain_lake"),
            Some(ReviewEffectAnchorKindV1::Water)
        );
        assert_eq!(
            review_effect_anchor_kind("grand_v3.river_bend"),
            Some(ReviewEffectAnchorKindV1::ValleyWater)
        );
        for name in ["grand_v3.coast"] {
            assert_eq!(
                review_effect_anchor_kind(name),
                Some(ReviewEffectAnchorKindV1::Water)
            );
        }
        for name in [
            "grand_v3.lake_island",
            "grand_v3.river_overlook",
            "grand_v3.massif_crest",
            "grand_v3.frozen_exit",
        ] {
            assert_eq!(review_effect_anchor_kind(name), None);
        }
    }

    #[test]
    fn effect_plan_hash_rebinds_to_each_finite_motion_phase() {
        let neutral = 0x1020_3040_5060_7080;
        let first = phase_bound_effect_plan_hash(neutral, 0.0)
            .expect("finite phase should produce a bound effects hash");
        let next = phase_bound_effect_plan_hash(neutral, 1.0)
            .expect("finite phase should produce a bound effects hash");
        assert_ne!(first, next);
        assert_eq!(first.len(), 16);
        assert_eq!(next.len(), 16);
        assert!(phase_bound_effect_plan_hash(neutral, f32::NAN).is_none());
    }

    #[test]
    fn mesh_projection_hash_uses_canonical_stream_bytes() {
        let positions = [[0.0, 1.0, 2.0], [3.0, 4.0, 5.0], [6.0, 7.0, 8.0]];
        let normals = [[0.0, 1.0, 0.0]; 3];
        let uvs = [[0.0, 0.0], [0.5, 1.0], [1.0, 0.0]];
        let indices = [0, 1, 2];
        let mut first = Vec::new();
        let mut second = Vec::new();
        let colors = [[1.0, 1.0, 1.0, 0.20]; 3];
        append_mesh_stream_hash(&mut first, &positions, &normals, &uvs, &colors, &indices);
        append_mesh_stream_hash(&mut second, &positions, &normals, &uvs, &colors, &indices);
        assert_eq!(first, second);

        let changed_positions = [[0.0, 1.0, 2.0], [3.0, 4.0, 5.0], [6.0, 7.0, 8.5]];
        let mut changed = Vec::new();
        append_mesh_stream_hash(
            &mut changed,
            &changed_positions,
            &normals,
            &uvs,
            &colors,
            &indices,
        );
        assert_ne!(first, changed);

        let mut changed_color = Vec::new();
        let mut colors_with_fade = colors;
        let [_, _, faded_color] = &mut colors_with_fade;
        let [_, _, _, alpha] = faded_color;
        *alpha = 0.0;
        append_mesh_stream_hash(
            &mut changed_color,
            &positions,
            &normals,
            &uvs,
            &colors_with_fade,
            &indices,
        );
        assert_ne!(first, changed_color);
    }

    #[test]
    fn incomplete_water_suppression_plan_is_rejected_before_commit() {
        let mut materials = Assets::<StandardMaterial>::default();
        let material = materials.add(StandardMaterial::default());
        let liquid_materials = Assets::<LiquidMaterial>::default();
        let liquid = liquid_materials.reserve_handle();
        let terrain_only = ReviewWaterSuppressionPlan {
            terrain: vec![(Entity::PLACEHOLDER, material.clone())],
            liquids: Vec::new(),
            water_batches: 1,
        };
        let liquid_only = ReviewWaterSuppressionPlan {
            terrain: Vec::new(),
            liquids: vec![(Entity::PLACEHOLDER, liquid.clone())],
            water_batches: 1,
        };
        let duplicate_paths = ReviewWaterSuppressionPlan {
            terrain: vec![(Entity::PLACEHOLDER, material)],
            liquids: vec![(Entity::PLACEHOLDER, liquid.clone())],
            water_batches: 1,
        };
        let incomplete = ReviewWaterSuppressionPlan {
            terrain: Vec::new(),
            liquids: vec![(Entity::PLACEHOLDER, liquid)],
            water_batches: 2,
        };
        assert!(!terrain_only.is_complete());
        assert!(liquid_only.is_complete());
        assert!(!duplicate_paths.is_complete());
        assert!(!incomplete.is_complete());
    }

    #[test]
    fn physical_cloud_effect_meshes_are_real_shadow_inert() {
        let mut world = World::new();
        let mut queue = CommandQueue::default();
        let mut meshes = Assets::<Mesh>::default();
        let mut materials = Assets::<StandardMaterial>::default();
        let mesh = meshes.add(Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::MAIN_WORLD,
        ));
        let material = EffectMaterialHandle::Standard(materials.add(StandardMaterial::default()));
        let entity = {
            let mut commands = Commands::new(&mut queue, &world);
            spawn_effect_mesh(&mut commands, mesh, &material, Family::PhysicalClouds)
        };
        queue.apply(&mut world);

        let cloud = world.entity(entity);
        assert!(cloud.contains::<NotShadowCaster>());
        assert!(cloud.contains::<ReviewWorldDetailEntity>());
        assert_eq!(cloud.get::<Pickable>(), Some(&Pickable::IGNORE));
    }

    #[test]
    fn cloud_shadow_uses_one_vertex_tinted_material() {
        let material = cloud_shadow_material();
        assert_eq!(material.alpha_mode, AlphaMode::Blend);
        assert_eq!(
            material.base_color.to_srgba().alpha.to_bits(),
            1.0_f32.to_bits()
        );
        assert!(material.unlit);
    }

    #[test]
    fn cloud_shadow_has_full_continuous_radial_blur_and_exact_cap() {
        let shadow = crate::review_world_detail_effects::ReviewCloudShadowV1 {
            cluster_index: 0,
            center_xz: Vec2::ZERO,
            diameter: 16.0,
            maximum_opacity: 0.20,
            blur_world: 24.0,
        };
        let shadows = [shadow];
        let opacity_at = |radius: f32| cloud_shadow_opacity(Vec3::X * radius, &shadows);

        assert!((opacity_at(8.0) - 0.20).abs() < 1.0e-6);
        assert!((opacity_at(20.0) - 0.05).abs() < 1.0e-6);
        assert_eq!(opacity_at(32.0).to_bits(), 0.0_f32.to_bits());
        assert!(opacity_at(31.0) > 0.0);

        let mut mesh = RawMesh::default();
        append_cloud_shadow_hex_cap(&mut mesh, Vec3::X * 20.0, &shadows)
            .expect("continuous shadow cap should build");
        assert_eq!(mesh.colors.len(), mesh.positions.len());
        assert!(mesh.colors.iter().all(|color| {
            let [_, _, _, alpha] = color;
            (0.0..=0.20).contains(alpha)
        }));
        assert!(mesh.colors.windows(2).any(|pair| {
            let [first, second] = pair else {
                return false;
            };
            let [_, _, _, first_alpha] = first;
            let [_, _, _, second_alpha] = second;
            first_alpha.to_bits() != second_alpha.to_bits()
        }));
        let rendered = mesh_from_parts(
            mesh.positions,
            mesh.normals,
            mesh.uvs,
            mesh.colors,
            mesh.indices,
        )
        .expect("continuous shadow colors should be a valid Bevy mesh stream");
        assert!(rendered.attribute(Mesh::ATTRIBUTE_COLOR).is_some());
    }

    #[test]
    fn rollback_removes_immediate_assets_and_deferred_entities() {
        let mut world = World::new();
        world.insert_resource(ReviewWorldDetailRuntimeAssetEvidenceV1 {
            liquid_material_count: 1,
            liquid_material_bytes: 1,
            review_water_material_count: 1,
            review_water_material_bytes: 1,
            ..default()
        });
        let mut queue = CommandQueue::default();
        let mut meshes = Assets::<Mesh>::default();
        let mut materials = Assets::<StandardMaterial>::default();
        let mut review_water_materials = Assets::<ReviewWaterMaterial>::default();
        let mut images = Assets::<Image>::default();
        let mesh = meshes.add(Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::MAIN_WORLD,
        ));
        let material = materials.add(StandardMaterial::default());
        let water_material = review_water_materials.add(ReviewWaterMaterial::default());
        let fog_image = images.add(fog_density_image(0.28, 0.20).expect("fog fixture image"));
        let entity;
        {
            let mut commands = Commands::new(&mut queue, &world);
            entity = commands.spawn(ReviewWorldDetailEntity).id();
            let state = ReviewWorldDetailProjectionState {
                entities: vec![entity],
                meshes: vec![mesh.clone()],
                images: vec![fog_image.clone()],
                materials: vec![material.clone()],
                review_water_materials: vec![water_material.clone()],
                ..default()
            };
            rollback_pending_review_projection(
                &state,
                &mut materials,
                &mut review_water_materials,
                &mut meshes,
                &mut images,
                &mut commands,
            );
        }
        queue.apply(&mut world);

        assert!(world.get_entity(entity).is_err());
        assert!(meshes.get(&mesh).is_none());
        assert!(materials.get(&material).is_none());
        assert!(review_water_materials.get(&water_material).is_none());
        assert!(images.get(&fog_image).is_none());
        assert!(!world.contains_resource::<ReviewWorldDetailRuntimeAssetEvidenceV1>());
    }

    #[test]
    fn runtime_asset_evidence_deduplicates_live_extended_water_materials() {
        let liquid_materials = Assets::<LiquidMaterial>::default();
        let mut review_water_materials = Assets::<ReviewWaterMaterial>::default();
        let mut images = Assets::<Image>::default();
        let fog_image = images.add(fog_density_image(0.28, 0.20).expect("fog fixture image"));
        let material = review_water_materials.add(ReviewWaterMaterial::default());
        let state = ReviewWorldDetailProjectionState {
            review_water_materials: vec![material.clone(), material],
            images: vec![fog_image.clone(), fog_image],
            ..default()
        };

        let evidence = review_runtime_asset_evidence(
            &state,
            &[],
            &liquid_materials,
            &review_water_materials,
            &images,
        )
        .expect("live review-water materials should produce runtime evidence");
        assert_eq!(evidence.liquid_material_count, 0);
        assert_eq!(evidence.liquid_material_bytes, 0);
        assert_eq!(evidence.review_water_material_count, 1);
        assert_eq!(
            evidence.review_water_material_bytes,
            bounded_u64(std::mem::size_of::<ReviewWaterMaterial>())
        );
        assert_eq!(evidence.fog_density_image_count, 1);
        assert_eq!(evidence.fog_density_image_bytes, 32 * 16 * 32);
        let no_review_water = review_runtime_asset_evidence(
            &ReviewWorldDetailProjectionState::default(),
            &[],
            &liquid_materials,
            &review_water_materials,
            &images,
        )
        .expect("a projection without extended review-water materials is valid");
        assert_eq!(no_review_water.liquid_material_count, 0);
        assert_eq!(no_review_water.review_water_material_count, 0);
        assert_eq!(no_review_water.review_water_material_bytes, 0);
        assert_eq!(no_review_water.fog_density_image_count, 0);
        assert_eq!(no_review_water.fog_density_image_bytes, 0);
    }

    #[test]
    fn wet_rim_is_an_opaque_substrate_relative_gloss_cap() {
        let (table, stone) = material_test_table(true);
        let descriptor = ReviewMaterialDescriptorV1 {
            key: ReviewMaterialKeyV1::WetRim { substrate: stone },
            alpha: Some(1.0),
            value_multiplier: 0.88,
            roughness: None,
            roughness_delta: Some(-0.15),
            reflectance: None,
            depth_half_distance: None,
            deep_value_multiplier: None,
            transmission: None,
            alpha_mode: ReviewAlphaModeV1::Opaque,
            double_sided: false,
            inward_feather: None,
        };
        let material = effect_material(&descriptor, (0.18, 0.42, 0.58), &table)
            .expect("valid substrate-relative wet rim should build");
        let color = material.base_color.to_srgba();

        assert_eq!(color.red.to_bits(), (0.25_f32 * 0.88).to_bits());
        assert_eq!(color.green.to_bits(), (0.50_f32 * 0.88).to_bits());
        assert_eq!(color.blue.to_bits(), (0.75_f32 * 0.88).to_bits());
        assert_eq!(color.alpha.to_bits(), 1.0_f32.to_bits());
        assert_eq!(material.perceptual_roughness.to_bits(), 0.35_f32.to_bits());
        assert_eq!(material.alpha_mode, AlphaMode::Opaque);
    }

    #[test]
    fn review_water_material_preserves_profile_fields_and_bounds_uniforms() {
        let (table, _) = material_test_table(true);
        let rough = ReviewMaterialDescriptorV1 {
            key: ReviewMaterialKeyV1::Water {
                style: ReviewWaterMaterialStyleV1::Surface,
            },
            alpha: Some(0.70),
            value_multiplier: 1.0,
            roughness: Some(0.40),
            roughness_delta: None,
            reflectance: Some(0.50),
            depth_half_distance: None,
            deep_value_multiplier: None,
            transmission: None,
            alpha_mode: ReviewAlphaModeV1::OrderIndependentTransparency,
            double_sided: true,
            inward_feather: None,
        };
        let material = review_water_material(
            &rough,
            (0.18, 0.42, 0.58),
            (0.94, 0.98, 1.0),
            &table,
            401.25,
        )
        .expect("valid rough-water material should build");
        assert_eq!(material.base.alpha_mode, AlphaMode::Blend);
        assert_eq!(material.base.base_color.to_srgba().alpha, 0.70);
        assert_eq!(material.base.perceptual_roughness, 0.40);
        assert_eq!(material.base.reflectance, 0.50);
        assert_eq!(
            material.extension.params.flow_phase_scale.z.to_bits(),
            1.25_f32.to_bits()
        );
        assert_eq!(
            material.extension.params.refraction.x.to_bits(),
            0.0_f32.to_bits()
        );
        assert_eq!(
            material.extension.params.modulation,
            Vec4::new(0.08, 0.0, 0.04, 0.65)
        );

        let transmission = ReviewMaterialDescriptorV1 {
            key: ReviewMaterialKeyV1::Water {
                style: ReviewWaterMaterialStyleV1::Surface,
            },
            alpha: None,
            value_multiplier: 1.0,
            roughness: None,
            roughness_delta: None,
            reflectance: None,
            depth_half_distance: None,
            deep_value_multiplier: None,
            transmission: Some(crate::review_world_detail_effects::ReviewTransmissionV1 {
                ior: 1.333,
                thickness: 0.08,
                max_refraction_uv: REVIEW_MAX_REFRACTION_UV,
            }),
            alpha_mode: ReviewAlphaModeV1::Opaque,
            double_sided: true,
            inward_feather: None,
        };
        let material = review_water_material(
            &transmission,
            (0.18, 0.42, 0.58),
            (0.94, 0.98, 1.0),
            &table,
            0.0,
        )
        .expect("valid transmission water should build");
        assert_eq!(material.base.alpha_mode, AlphaMode::Opaque);
        assert_eq!(material.base.ior, 1.333);
        assert_eq!(material.base.thickness, 0.08);
        assert!(material.base.specular_transmission > 0.0);
        assert_eq!(material.base.perceptual_roughness, 0.0);
        assert_eq!(
            material.extension.params.refraction.x.to_bits(),
            REVIEW_MAX_REFRACTION_UV.to_bits()
        );

        let mut excessive = transmission;
        excessive
            .transmission
            .as_mut()
            .expect("transmission fixture retains its contract")
            .max_refraction_uv = REVIEW_MAX_REFRACTION_UV + 0.001;
        assert!(review_water_material(
            &excessive,
            (0.18, 0.42, 0.58),
            (0.94, 0.98, 1.0),
            &table,
            0.0,
        )
        .is_err());
    }

    #[test]
    fn water_vertex_colors_preserve_the_srgb_value_multiplier() {
        let base = (0.18, 0.42, 0.58);
        let value = 0.81;
        let colors = water_value_vertex_colors(&[[value, value, value, 1.0]], 1, base)
            .expect("valid depth multiplier should convert to a linear vertex tint");
        let [converted] = colors.as_slice() else {
            panic!("one source color must produce one converted color");
        };
        let [converted_red, converted_green, converted_blue, converted_alpha] = *converted;
        for (base_channel, converted_channel) in [base.0, base.1, base.2].into_iter().zip([
            converted_red,
            converted_green,
            converted_blue,
        ]) {
            let rendered = srgb_channel_to_linear(base_channel) * converted_channel;
            let expected = srgb_channel_to_linear(base_channel * value);
            assert!((rendered - expected).abs() < 1.0e-7);
        }
        assert_eq!(converted_alpha.to_bits(), 1.0_f32.to_bits());
        assert!(water_value_vertex_colors(&[], 1, base).is_err());
    }

    #[test]
    #[expect(
        clippy::cast_precision_loss,
        reason = "the fixed 16-texel test height is exactly representable in f32"
    )]
    fn fog_density_texture_has_soft_lateral_edges_and_exact_centre_extinction() {
        let image = fog_density_image(0.28, 0.20).expect("valid shared fog density texture");
        assert_eq!(image.texture_descriptor.dimension, TextureDimension::D3);
        assert_eq!(image.texture_descriptor.format, TextureFormat::R8Unorm);
        let data = image
            .data
            .as_ref()
            .expect("fog texture retains its payload");
        assert_eq!(data.len(), 32 * 16 * 32);
        assert_eq!(data.first(), Some(&0));
        assert_eq!(data.last(), Some(&0));
        assert_eq!(data.iter().copied().max(), Some(255));
        let width = 32_usize;
        let height = 16_usize;
        let centre_x = width / 2;
        let centre_z = 32_usize / 2;
        let centre_ray_weight = (0..height)
            .map(|y| {
                let index = centre_z * width * height + y * width + centre_x;
                f32::from(
                    data.get(index)
                        .copied()
                        .expect("centre-ray texel stays inside the fixed texture"),
                ) / 255.0
            })
            .sum::<f32>()
            / height as f32;
        assert_eq!(centre_ray_weight.to_bits(), 1.0_f32.to_bits());
        assert_eq!(
            (REVIEW_FOG_ABSORPTION + REVIEW_FOG_SCATTERING).to_bits(),
            1.0_f32.to_bits()
        );
        let opacity = 0.10_f32;
        let height = 2.8_f32;
        let density = -(1.0 - opacity).ln() / height;
        let resolved = 1.0
            - (-(REVIEW_FOG_ABSORPTION + REVIEW_FOG_SCATTERING)
                * density
                * height
                * centre_ray_weight)
                .exp();
        assert!((resolved - opacity).abs() < 1.0e-6);
        let ImageSampler::Descriptor(sampler) = &image.sampler else {
            panic!("fog density texture requires an explicit clamp sampler");
        };
        assert_eq!(sampler.address_mode_u, ImageAddressMode::ClampToEdge);
        assert_eq!(sampler.address_mode_v, ImageAddressMode::ClampToEdge);
        assert_eq!(sampler.address_mode_w, ImageAddressMode::ClampToEdge);
    }

    #[test]
    fn phase_update_changes_only_the_bounded_review_uniform() {
        let (table, _) = material_test_table(true);
        let descriptor = ReviewMaterialDescriptorV1 {
            key: ReviewMaterialKeyV1::Water {
                style: ReviewWaterMaterialStyleV1::Surface,
            },
            alpha: Some(0.70),
            value_multiplier: 0.82,
            roughness: None,
            roughness_delta: None,
            reflectance: None,
            depth_half_distance: Some(1.40),
            deep_value_multiplier: Some(0.82),
            transmission: None,
            alpha_mode: ReviewAlphaModeV1::OrderIndependentTransparency,
            double_sided: true,
            inward_feather: None,
        };
        let mut material = review_water_material(
            &descriptor,
            (0.18, 0.42, 0.58),
            (0.94, 0.98, 1.0),
            &table,
            0.0,
        )
        .expect("valid depth-water material should build");
        let alpha = material.base.base_color.to_srgba().alpha;
        let roughness = material.base.perceptual_roughness;
        let fixed_uniforms = (
            material.extension.params.flow_phase_scale.x,
            material.extension.params.flow_phase_scale.y,
            material.extension.params.flow_phase_scale.w,
            material.extension.params.modulation,
            material.extension.params.emission,
            material.extension.params.foam_color,
            material.extension.params.refraction,
        );

        set_review_water_material_phase(&mut material, -0.5);

        assert_eq!(
            material.extension.params.flow_phase_scale.z.to_bits(),
            399.5_f32.to_bits()
        );
        assert_eq!(
            (
                material.extension.params.flow_phase_scale.x,
                material.extension.params.flow_phase_scale.y,
                material.extension.params.flow_phase_scale.w,
                material.extension.params.modulation,
                material.extension.params.emission,
                material.extension.params.foam_color,
                material.extension.params.refraction,
            ),
            fixed_uniforms
        );
        assert_eq!(material.base.base_color.to_srgba().alpha, alpha);
        assert_eq!(material.base.perceptual_roughness, roughness);
    }

    #[test]
    fn review_shader_contains_phase_oit_and_executed_refraction_cap() {
        let shader = include_str!("../../../assets/shaders/review_world_detail_water.wgsl");
        assert!(shader.contains("review_water.flow_phase_scale.z * 0.025"));
        assert!(shader.contains("review_water.modulation.z"));
        assert!(!shader.contains("pbr_input.N ="));
        assert!(shader.contains("oit_draw(in.position, out.color)"));
        assert!(shader.contains("fn capped_refraction_thickness"));
        assert!(shader.contains("clamp(maximum_uv, 0.0, 0.015)"));
        assert!(shader.contains("pbr_input.material.thickness = capped_refraction_thickness"));
        assert!(shader.contains("pbr_input.material.perceptual_roughness = 0.0"));
    }

    #[test]
    fn review_renderer_is_inert_without_a_review_profile_or_image_assets() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default(), StatesPlugin));
        app.init_state::<hex_core::Screen>();
        plugin(&mut app);

        assert!(!app
            .world()
            .contains_resource::<ReviewWorldDetailProfileV1>());
        assert!(!app.world().contains_resource::<Assets<Image>>());

        app.world_mut()
            .resource_mut::<NextState<hex_core::Screen>>()
            .set(hex_core::Screen::Gameplay);
        app.update();
        app.update();
        app.world_mut()
            .resource_mut::<NextState<hex_core::Screen>>()
            .set(hex_core::Screen::Title);
        app.update();

        assert!(!app.world().contains_resource::<Assets<Image>>());
    }

    #[test]
    fn teardown_receipt_counts_every_missing_or_mismatched_ordinary_restoration_target() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(Assets::<Mesh>::default());
        app.insert_resource(Assets::<StandardMaterial>::default());
        app.insert_resource(Assets::<ReviewWaterMaterial>::default());
        app.insert_resource(Assets::<Image>::default());
        app.add_systems(Update, publish_review_world_detail_teardown_receipt);

        let original_material = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial::default());
        let wrong_material = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial::default());
        let liquid_materials = Assets::<LiquidMaterial>::default();
        let original_liquid = liquid_materials.reserve_handle();
        let wrong_liquid_material = liquid_materials.reserve_handle();
        let terrain = app.world_mut().spawn_empty().id();
        let liquid = app.world_mut().spawn_empty().id();
        let vegetation = app.world_mut().spawn_empty().id();
        assert!(app.world_mut().despawn(terrain));
        assert!(app.world_mut().despawn(liquid));
        assert!(app.world_mut().despawn(vegetation));
        let wrong_terrain = app.world_mut().spawn(MeshMaterial3d(wrong_material)).id();
        let wrong_liquid = app
            .world_mut()
            .spawn((
                ReviewLiquidPresentationRole(FillMaterialRole::Water),
                MeshMaterial3d(wrong_liquid_material),
            ))
            .id();
        let wrong_vegetation = app
            .world_mut()
            .spawn(Transform::from_scale(Vec3::splat(2.0)))
            .id();
        app.world_mut()
            .insert_resource(ReviewWorldDetailTeardownTargets {
                suppressed_terrain: BTreeMap::from([
                    (terrain, original_material.clone()),
                    (wrong_terrain, original_material),
                ]),
                suppressed_liquids: BTreeMap::from([
                    (liquid, original_liquid.clone()),
                    (wrong_liquid, original_liquid),
                ]),
                vegetation_original_scales: BTreeMap::from([
                    (vegetation, Vec3::ONE),
                    (wrong_vegetation, Vec3::ONE),
                ]),
                ..default()
            });

        app.update();

        let receipt = app.world().resource::<ReviewWorldDetailTeardownReceiptV1>();
        assert_eq!(receipt.terrain_material_overrides_remaining, 2);
        assert_eq!(receipt.liquid_visibility_overrides_remaining, 2);
        assert_eq!(receipt.vegetation_scale_overrides_remaining, 2);
    }

    #[test]
    fn water_review_suppression_preserves_pick_mesh_and_restores_exact_material() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(Assets::<Mesh>::default());
        app.insert_resource(Assets::<StandardMaterial>::default());
        app.insert_resource(Assets::<ReviewWaterMaterial>::default());
        app.insert_resource(Assets::<Image>::default());
        app.add_systems(
            Update,
            (
                restore_review_world_detail,
                publish_review_world_detail_teardown_receipt,
            )
                .chain(),
        );

        let liquid_materials = Assets::<LiquidMaterial>::default();
        let original = liquid_materials.reserve_handle();
        let mesh = app.world().resource::<Assets<Mesh>>().reserve_handle();
        let water = app
            .world_mut()
            .spawn((
                ReviewLiquidPresentationRole(FillMaterialRole::Water),
                MeshMaterial3d(original.clone()),
                Mesh3d(mesh.clone()),
                Visibility::Inherited,
                Pickable::default(),
            ))
            .id();
        let lava = app
            .world_mut()
            .spawn((
                ReviewLiquidPresentationRole(FillMaterialRole::Lava),
                MeshMaterial3d(original.clone()),
                Visibility::Inherited,
            ))
            .id();
        let plan = ReviewWaterSuppressionPlan {
            terrain: Vec::new(),
            liquids: vec![(water, original.clone())],
            water_batches: 1,
        };
        assert!(plan.is_complete());
        let mut state = ReviewWorldDetailProjectionState::default();
        let mut queue = CommandQueue::default();
        plan.suppress(&mut Commands::new(&mut queue, app.world()), &mut state);
        queue.apply(app.world_mut());

        assert!(app
            .world()
            .get::<MeshMaterial3d<LiquidMaterial>>(water)
            .is_none());
        assert_eq!(
            app.world().get::<Mesh3d>(water).map(|mesh| &mesh.0),
            Some(&mesh)
        );
        assert_eq!(
            app.world().get::<Visibility>(water),
            Some(&Visibility::Inherited)
        );
        assert_eq!(
            app.world().get::<Pickable>(water),
            Some(&Pickable::default())
        );
        assert_eq!(
            app.world()
                .get::<MeshMaterial3d<LiquidMaterial>>(lava)
                .map(|material| &material.0),
            Some(&original)
        );
        assert_eq!(
            state.material_count(),
            0,
            "suppressing a binding allocates no material asset"
        );

        app.insert_resource(state);
        app.update();

        assert_eq!(
            app.world()
                .get::<MeshMaterial3d<LiquidMaterial>>(water)
                .map(|material| &material.0),
            Some(&original)
        );
        assert_eq!(
            app.world().get::<Mesh3d>(water).map(|mesh| &mesh.0),
            Some(&mesh)
        );
        assert_eq!(
            app.world().get::<Visibility>(water),
            Some(&Visibility::Inherited)
        );
        assert_eq!(
            app.world()
                .resource::<ReviewWorldDetailTeardownReceiptV1>()
                .liquid_visibility_overrides_remaining,
            0
        );
    }

    #[test]
    fn presentation_assets_and_transforms_restore_over_one_hundred_exit_cycles() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin));
        app.init_state::<hex_core::Screen>();
        app.insert_resource(Assets::<Mesh>::default());
        app.insert_resource(Assets::<StandardMaterial>::default());
        app.insert_resource(Assets::<ReviewWaterMaterial>::default());
        app.insert_resource(Assets::<Image>::default());
        app.add_systems(
            OnExit(hex_core::Screen::Gameplay),
            (
                restore_review_world_detail,
                publish_review_world_detail_teardown_receipt,
            )
                .chain(),
        );

        let vegetation_child = app.world_mut().spawn(Transform::from_scale(Vec3::ONE)).id();
        for cycle in 0..100 {
            app.world_mut()
                .resource_mut::<NextState<hex_core::Screen>>()
                .set(hex_core::Screen::Gameplay);
            app.update();

            let material = app
                .world_mut()
                .resource_mut::<Assets<StandardMaterial>>()
                .add(StandardMaterial::default());
            let review_water_material = app
                .world_mut()
                .resource_mut::<Assets<ReviewWaterMaterial>>()
                .add(ReviewWaterMaterial::default());
            let mesh = app
                .world_mut()
                .resource_mut::<Assets<Mesh>>()
                .add(Mesh::new(
                    PrimitiveTopology::TriangleList,
                    RenderAssetUsages::MAIN_WORLD,
                ));
            let fog_image = app
                .world_mut()
                .resource_mut::<Assets<Image>>()
                .add(fog_density_image(0.28, 0.20).expect("fog fixture image"));
            let review_entity = app.world_mut().spawn(ReviewWorldDetailEntity).id();
            app.world_mut()
                .entity_mut(vegetation_child)
                .get_mut::<Transform>()
                .expect("vegetation fixture keeps its transform")
                .scale = Vec3::splat(1.5);
            let mut vegetation_original_scales = BTreeMap::new();
            vegetation_original_scales.insert(vegetation_child, Vec3::ONE);
            app.world_mut()
                .insert_resource(ReviewWorldDetailProjectionState {
                    entities: vec![review_entity],
                    meshes: vec![mesh.clone()],
                    images: vec![fog_image.clone()],
                    materials: vec![material.clone()],
                    review_water_materials: vec![review_water_material.clone()],
                    suppressed_terrain: BTreeMap::new(),
                    suppressed_liquids: BTreeMap::new(),
                    vegetation_treatments: BTreeMap::new(),
                    vegetation_original_scales,
                    effects_phase_neutral_hash: 0,
                });
            app.world_mut()
                .insert_resource(ReviewWorldDetailProjectionHashesV1::default());
            app.world_mut()
                .insert_resource(ReviewWorldDetailRuntimeAssetEvidenceV1 {
                    liquid_material_count: 1,
                    liquid_material_bytes: bounded_u64(std::mem::size_of::<LiquidMaterial>()),
                    review_water_material_count: 1,
                    review_water_material_bytes: bounded_u64(std::mem::size_of::<
                        ReviewWaterMaterial,
                    >()),
                    ..default()
                });

            app.world_mut()
                .resource_mut::<NextState<hex_core::Screen>>()
                .set(hex_core::Screen::Title);
            app.update();

            assert!(
                app.world().get_entity(review_entity).is_err(),
                "cycle {cycle} retained a disposable review entity"
            );
            assert!(
                app.world().resource::<Assets<Mesh>>().get(&mesh).is_none(),
                "cycle {cycle} retained a disposable review mesh"
            );
            assert!(
                app.world()
                    .resource::<Assets<StandardMaterial>>()
                    .get(&material)
                    .is_none(),
                "cycle {cycle} retained a disposable review material"
            );
            assert!(
                app.world()
                    .resource::<Assets<ReviewWaterMaterial>>()
                    .get(&review_water_material)
                    .is_none(),
                "cycle {cycle} retained a disposable review water material"
            );
            assert!(
                app.world()
                    .resource::<Assets<Image>>()
                    .get(&fog_image)
                    .is_none(),
                "cycle {cycle} retained a disposable fog density image"
            );
            assert_eq!(
                app.world()
                    .entity(vegetation_child)
                    .get::<Transform>()
                    .expect("vegetation fixture keeps its transform")
                    .scale,
                Vec3::ONE,
                "cycle {cycle} did not restore a presentation-child scale"
            );
            assert!(!app
                .world()
                .contains_resource::<ReviewWorldDetailProjectionState>());
            assert!(!app
                .world()
                .contains_resource::<ReviewWorldDetailProjectionHashesV1>());
            assert!(!app
                .world()
                .contains_resource::<ReviewWorldDetailRuntimeAssetEvidenceV1>());
            assert_eq!(
                app.world().resource::<ReviewWorldDetailTeardownReceiptV1>(),
                &ReviewWorldDetailTeardownReceiptV1::default(),
                "cycle {cycle} did not publish an exact zero-count teardown receipt"
            );
            assert!(!app
                .world()
                .contains_resource::<ReviewWorldDetailTeardownTargets>());
        }
    }
}
