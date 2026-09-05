//! Deterministic liquid and atmospheric projections for world-detail review.
//!
//! The types in this module deliberately stop before ECS. They describe shared
//! materials, chunk-batched meshes, world-space cloud puffs, spray volumes, and
//! local fog volumes without spawning entities or attaching any authoritative
//! component. The runtime review adapter may translate these values into disposable
//! render entities, but it must keep them collider-free and non-pickable.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use bevy::prelude::{Quat, Vec2, Vec3};
use hex_core::config::{HEX_CIRCUMRADIUS, HEX_SMALL_DIAMETER};
use hex_core::{HexCoord, Level, SubstanceId, TilePos};
use xxhash_rust::xxh3::{xxh3_64, xxh3_64_with_seed};

use crate::review_world_detail::{
    CloudAltitudeBandV1, CloudShapeV1, IceFringeDetailV1, LocalFogDetailV1, LocalFogPlacementV1,
    PhysicalCloudsDetailV1, ReviewCloudCoverageEvidenceV1, ReviewFogCoverageEvidenceV1,
    ReviewIceCoverageEvidenceV1, ReviewWaterfallAnchorEvidenceV1,
    ReviewWorldDetailEffectValidationV1, ReviewWorldDetailProfileV1, ShoreAndFallsDetailV1,
    WaterDetailV1,
};

const HEX_INRADIUS: f32 = 0.5 * HEX_SMALL_DIAMETER;
const WATER_CAP_BIAS: f32 = 0.002;
const ATTACHED_SURFACE_BIAS: f32 = 0.004;
const CLOUD_MAX_CLUSTERS: usize = 192;
const CLOUD_COVERAGE_GRID: u32 = 256;
const CLOUD_COVERAGE_TOLERANCE: f32 = 0.01;
const CLOUD_LAYOUT_DOMAIN: &str = "review-cloud-common-layout-v1";
const ICE_COVERAGE_DOMAIN: &str = "review-ice-common-shore-rank-v1";
const FOG_LAYOUT_DOMAIN: &str = "review-fog-common-anchor-layout-v1";
pub(crate) const FOG_DENSITY_WIDTH: u32 = 32;
pub(crate) const FOG_DENSITY_DEPTH: u32 = 32;
const GLOBAL_BATCH_CHUNK: ReviewChunkKeyV1 = ReviewChunkKeyV1 { q: 0, r: 0 };

/// Stable resident-chunk identity supplied by the terrain renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReviewChunkKeyV1 {
    /// Axial chunk coordinate on the first stored axis.
    pub q: i32,
    /// Axial chunk coordinate on the second stored axis.
    pub r: i32,
}

/// Material class of one exact liquid presentation cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReviewLiquidKindV1 {
    /// Ordinary water eligible for the optics, shoreline, and ice studies.
    Water,
    /// Lava retained for adapter completeness but unaffected by this matrix.
    Lava,
}

/// Renderer-facing flow class copied from the map-owned liquid projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReviewLiquidFlowV1 {
    /// Still pool or ocean water.
    Still,
    /// Directed current.
    Current,
    /// Fast directed current.
    Rapid,
    /// Vertical or steep waterfall cell.
    Fall,
}

/// Existing liquid-render material style retained by a review water batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReviewWaterMaterialStyleV1 {
    /// Horizontal caps and ordinary exposed steps.
    Surface,
    /// A semantic downstream waterfall curtain.
    Fall,
}

/// Exact input for one exposed liquid run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReviewLiquidCellV1 {
    /// Exact topmost liquid voxel, preserving stacked surfaces.
    pub position: TilePos,
    /// Inclusive bottom level of the same contiguous liquid run.
    pub run_bottom: Level,
    /// Water or lava presentation class.
    pub kind: ReviewLiquidKindV1,
    /// Deterministic flow class.
    pub flow: ReviewLiquidFlowV1,
    /// Exact downstream liquid surface when the map publishes one.
    pub downstream: Option<TilePos>,
    /// Existing terrain resident chunk used for batching.
    pub chunk: ReviewChunkKeyV1,
}

/// Exact contiguous authoritative solid run used to close liquid curtain geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReviewPhysicalSolidRunV1 {
    /// Exact topmost solid voxel in the run.
    pub position: TilePos,
    /// Inclusive bottom level of the same contiguous solid run.
    pub run_bottom: Level,
}

/// Exact exposed terrain surface adjacent to potential liquid edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReviewShoreSurfaceV1 {
    /// Exact surface voxel, preserving stacks beneath bridges and overhangs.
    pub position: TilePos,
    /// Inclusive bottom level of the same contiguous solid run.
    pub run_bottom: Level,
    /// Existing terrain resident chunk used for batching attached geometry.
    pub chunk: ReviewChunkKeyV1,
    /// Exact exposed terrain substance whose current material the wet rim modifies.
    pub substance: SubstanceId,
    /// Whether the presentation snow mask covers this surface.
    pub snow_covered: bool,
    /// Whether the surface belongs to the Frozen biome exception.
    pub frozen_biome: bool,
    /// Whether generation marked the surface safe for shoreline presentation.
    pub eligible: bool,
}

/// Placement class for a named local-effect anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReviewEffectAnchorKindV1 {
    /// Water-body surface or bank anchor.
    Water,
    /// Low valley-floor anchor.
    Valley,
    /// A river surface that is also the visible floor of a low valley.
    ///
    /// Keeping this explicit lets the focused river-bend diagnostic exercise
    /// both water-hugging and valley-floor fog without inventing a second
    /// world-space anchor or double-spawning the mixed treatment.
    ValleyWater,
    /// Waterfall landing or plunge-pool anchor.
    Waterfall,
}

/// Named, presentation-only world-space anchor for fog and spray review.
#[derive(Debug, Clone, PartialEq)]
pub struct ReviewEffectAnchorV1 {
    /// Stable generated anchor name retained in reports.
    pub name: String,
    /// Placement class used by the selected fog profile.
    pub kind: ReviewEffectAnchorKindV1,
    /// Exact authored surface tile used for semantic matching.
    pub position: TilePos,
    /// Local surface point in world space.
    pub surface: Vec3,
}

/// One exact occupied vertical interval in the selected peak's voxel column.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReviewPeakSolidSpanV1 {
    /// Inclusive world-space lower boundary.
    pub bottom_y: f32,
    /// Exclusive world-space upper boundary.
    pub top_y: f32,
}

/// Complete renderer-neutral input for the five liquid/atmospheric families.
#[derive(Debug, Clone, PartialEq)]
pub struct LiquidAtmosphereReviewInputV1 {
    /// Exact map seed used to select coverage and cloud placement.
    pub seed: u64,
    /// World-space height of one voxel level.
    pub level_height: f32,
    /// Frozen visual phase for stills; motion evidence may supply another finite value.
    pub phase_seconds: f32,
    /// Maximum exposed natural-terrain height `H` in world units.
    pub max_exposed_natural_y: f32,
    /// World-space location of `grand_v3.massif_crest`.
    pub massif_crest: Vec3,
    /// Highest exposed natural-terrain point used as the peak-touch witness.
    pub interaction_peak: Vec3,
    /// Exact occupied intervals in [`Self::interaction_peak`]'s voxel column.
    pub interaction_peak_solid_spans: Vec<ReviewPeakSolidSpanV1>,
    /// Radius of the deterministic circular massif cloud field whose footprint
    /// defines `projected_coverage`; this is intentionally not the full map.
    pub cloud_field_radius: f32,
    /// Exact liquid presentation cells in arbitrary input order.
    pub liquids: Vec<ReviewLiquidCellV1>,
    /// Every exact authoritative solid run, including structures and buried stacks.
    pub physical_solid_runs: Vec<ReviewPhysicalSolidRunV1>,
    /// Exact eligible and ineligible shore surfaces in arbitrary input order.
    pub shore_surfaces: Vec<ReviewShoreSurfaceV1>,
    /// Named water, valley, and waterfall anchors in arbitrary input order.
    pub effect_anchors: Vec<ReviewEffectAnchorV1>,
}

/// Alpha pipeline requested by one shared review material.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewAlphaModeV1 {
    /// Depth-writing opaque attached geometry.
    Opaque,
    /// Bevy order-independent transparency.
    OrderIndependentTransparency,
}

/// Medium-quality screen-space transmission request for W06.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReviewTransmissionV1 {
    /// Physical index of refraction.
    pub ior: f32,
    /// Effective material thickness in world units.
    pub thickness: f32,
    /// Maximum screen-UV refraction displacement.
    pub max_refraction_uv: f32,
}

/// Stable shared-material slot used by mesh batches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReviewMaterialKeyV1 {
    /// Water material shared by all depths for one existing surface/fall style.
    Water {
        /// Existing surface/fall response that optics must preserve.
        style: ReviewWaterMaterialStyleV1,
    },
    /// Darkened shore rim, shared by every edge with the same terrain substrate.
    WetRim {
        /// Existing terrain substance whose value and roughness are adjusted.
        substrate: SubstanceId,
    },
    /// Shoreline or plunge-pool foam.
    Foam,
    /// Thin shore-attached ice wedge.
    Ice,
    /// Batched low-poly physical cloud body.
    Cloud,
}

/// Overrides applied to one shared review material.
#[derive(Debug, Clone, PartialEq)]
pub struct ReviewMaterialDescriptorV1 {
    /// Stable material slot referenced by batches.
    pub key: ReviewMaterialKeyV1,
    /// Explicit alpha override, or `None` to retain the current material value.
    pub alpha: Option<f32>,
    /// Multiplicative value adjustment applied after the current palette colour.
    pub value_multiplier: f32,
    /// Explicit roughness override, or `None` to retain the current value.
    pub roughness: Option<f32>,
    /// Additive roughness adjustment, used only by wet-rim treatments.
    pub roughness_delta: Option<f32>,
    /// Explicit reflectance override, or `None` to retain the current value.
    pub reflectance: Option<f32>,
    /// Depth-absorption half-distance retained as plan evidence.
    pub depth_half_distance: Option<f32>,
    /// Asymptotic deep-water value multiplier.
    pub deep_value_multiplier: Option<f32>,
    /// Optional screen-space transmission contract.
    pub transmission: Option<ReviewTransmissionV1>,
    /// Required alpha pipeline.
    pub alpha_mode: ReviewAlphaModeV1,
    /// Whether both triangle faces must render.
    pub double_sided: bool,
    /// Optional inward feather width used by I06.
    pub inward_feather: Option<f32>,
}

/// Geometry layer carried by a chunk-batched mesh.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReviewMeshLayerV1 {
    /// Horizontal transparent water caps.
    WaterCaps,
    /// Vertical transparent water curtains.
    WaterCurtains,
    /// Thin land-side wet rims.
    WetRims,
    /// Thin water-side shore foam.
    ShoreFoam,
    /// Foam discs at waterfall landings.
    PoolFoam,
    /// Non-bridging inward ice wedges.
    IceFringes,
    /// Batched low-poly world-space cloud puffs.
    CloudPuffs,
}

/// Stable key for one material- and chunk-batched mesh.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReviewMeshBatchKeyV1 {
    /// Resident terrain chunk; clouds use the global zero chunk.
    pub chunk: ReviewChunkKeyV1,
    /// Visual layer.
    pub layer: ReviewMeshLayerV1,
    /// Shared material slot.
    pub material: ReviewMaterialKeyV1,
}

/// Indexed triangle data ready for conversion into one Bevy mesh.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReviewIndexedMeshV1 {
    /// World-space vertex positions.
    pub positions: Vec<[f32; 3]>,
    /// World-space vertex normals.
    pub normals: Vec<[f32; 3]>,
    /// Stable UV coordinates.
    pub uvs: Vec<[f32; 2]>,
    /// Optional vertex payload. Water uses equal RGB lanes to carry its exact
    /// continuous value response, while I06 uses alpha for its inward feather;
    /// an empty stream means an untinted mesh.
    pub colors: Vec<[f32; 4]>,
    /// Counter-clockwise triangle indices.
    pub indices: Vec<u32>,
}

impl ReviewIndexedMeshV1 {
    /// Validates finite streams, bounded indices, nondegenerate faces, and winding.
    pub fn validate(&self) -> Result<(), ReviewWorldDetailEffectError> {
        validate_mesh(self)
    }
}

/// One shared-material mesh batch.
#[derive(Debug, Clone, PartialEq)]
pub struct ReviewMeshBatchV1 {
    /// Chunk, layer, and material identity.
    pub key: ReviewMeshBatchKeyV1,
    /// Concrete indexed geometry.
    pub mesh: ReviewIndexedMeshV1,
}

/// Exact ownership record for one water-to-land boundary edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReviewOwnedShoreEdgeV1 {
    /// Exact water surface that owns the edge.
    pub water: TilePos,
    /// Clockwise edge side on the water hex.
    pub side: ReviewHexSideV1,
    /// Exact adjacent land surface.
    pub land: TilePos,
}

/// Exact ownership record for one rendered transparent water curtain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReviewOwnedLiquidEdgeV1 {
    /// Exact water surface that exclusively owns the curtain.
    pub water: TilePos,
    /// Clockwise side on the owning water hex.
    pub side: ReviewHexSideV1,
    /// Adjacent lower water or shore surface, absent only at an open map edge.
    pub adjacent_surface: Option<TilePos>,
}

/// Clockwise side names matching [`HexCoord::neighbors`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReviewHexSideV1 {
    /// Positive first cube axis.
    East,
    /// Positive second cube axis.
    SouthEast,
    /// Negative first and positive second cube axes.
    SouthWest,
    /// Negative first cube axis.
    West,
    /// Negative second cube axis.
    NorthWest,
    /// Positive first and negative second cube axes.
    NorthEast,
}

impl ReviewHexSideV1 {
    /// Every side in fixed clockwise order.
    pub const ALL: [Self; 6] = [
        Self::East,
        Self::SouthEast,
        Self::SouthWest,
        Self::West,
        Self::NorthWest,
        Self::NorthEast,
    ];

    /// Adjacent coordinate on this side.
    #[must_use]
    pub fn neighbor(self, coord: HexCoord) -> HexCoord {
        let [east, south_east, south_west, west, north_west, north_east] = coord.neighbors();
        match self {
            Self::East => east,
            Self::SouthEast => south_east,
            Self::SouthWest => south_west,
            Self::West => west,
            Self::NorthWest => north_west,
            Self::NorthEast => north_east,
        }
    }
}

/// Low-poly world-space cloud shape selected by one treatment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReviewCloudPrimitiveShapeV1 {
    /// Sparse angular puff cluster.
    Faceted,
    /// Denser rounded puff cluster.
    Rounded,
    /// Wide, vertically compressed lens.
    Lenticular,
}

/// One deterministic low-poly puff retained for reporting and motion checks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReviewCloudPuffV1 {
    /// Zero-based cloud-cluster identity.
    pub cluster_index: u32,
    /// Zero-based puff identity within the cluster.
    pub puff_index: u8,
    /// Review shape class.
    pub shape: ReviewCloudPrimitiveShapeV1,
    /// Shared centre of the cluster's hard spherical envelope.
    pub cluster_center: Vec3,
    /// Full outer-envelope diameter, always within the requested 16–32 range.
    pub cluster_diameter: f32,
    /// World-space puff centre.
    pub center: Vec3,
    /// Axis-aligned half extents before yaw rotation.
    pub half_extents: Vec3,
    /// Deterministic yaw in radians.
    pub yaw: f32,
}

/// Soft projected cloud shadow used only by C08.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReviewCloudShadowV1 {
    /// Cluster identity casting this shadow.
    pub cluster_index: u32,
    /// Projector centre in the horizontal plane.
    pub center_xz: Vec2,
    /// Approximate projected diameter in world units.
    pub diameter: f32,
    /// Maximum opacity of the projected shadow.
    pub maximum_opacity: f32,
    /// World-space radial width of the outer shadow feather transition.
    pub blur_world: f32,
}

/// Collisionless plunge-spray volume emitted at one unique fall landing.
#[derive(Debug, Clone, PartialEq)]
pub struct ReviewSprayVolumeV1 {
    /// Exact authored waterfall anchor used to resolve this landing.
    pub anchor_name: String,
    /// Authored dry review footing from which the landing was resolved.
    pub anchor_position: TilePos,
    /// Exact axial resolution distance retained for report evidence.
    pub anchor_distance_hexes: u32,
    /// Exact downstream liquid surface used as stable identity.
    pub landing: TilePos,
    /// World-space volume centre.
    pub center: Vec3,
    /// Horizontal radius in world units.
    pub radius: f32,
    /// Full vertical height in world units.
    pub height: f32,
    /// Maximum presentation opacity.
    pub opacity: f32,
}

/// Deterministic descriptor for one Bevy `FogVolume` instance.
#[derive(Debug, Clone, PartialEq)]
pub struct ReviewFogVolumeV1 {
    /// Stable source anchor name.
    pub anchor_name: String,
    /// Source anchor class.
    pub anchor_kind: ReviewEffectAnchorKindV1,
    /// World-space box centre.
    pub center: Vec3,
    /// World-space half extents.
    pub half_extents: Vec3,
    /// Requested integrated opacity through the volume height.
    pub opacity: f32,
    /// Homogeneous density derived from opacity and height.
    pub density: f32,
    /// Fraction of the radius reserved for a soft boundary.
    pub edge_softness: f32,
    /// Fraction of deterministic XZ density samples carrying fog.
    pub coverage: f32,
}

/// Render-state changes the disposable adapter must apply and restore.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReviewRenderStateRequirementsV1 {
    /// Hide only the opaque rendered water mesh, never logical runs or pick proxies.
    pub suppress_opaque_water_geometry: bool,
    /// Enable Bevy order-independent transparency while the review plan is active.
    pub order_independent_transparency: bool,
    /// Enable medium-quality screen-space transmission for W06.
    pub medium_screen_space_transmission: bool,
    /// Enable volumetric fog camera state while local fog is active.
    pub volumetric_fog: bool,
    /// Restore the prior opaque-water render visibility during teardown.
    pub restore_opaque_water_geometry: bool,
    /// Restore prior OIT camera state during teardown.
    pub restore_order_independent_transparency: bool,
    /// Restore prior transmission camera state during teardown.
    pub restore_screen_space_transmission: bool,
    /// Restore prior volumetric camera state during teardown.
    pub restore_volumetric_fog: bool,
}

/// Deterministic cardinalities folded into each runtime report.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReviewWorldDetailEffectCountsV1 {
    /// Number of shared material descriptors.
    pub materials: u32,
    /// Number of chunk/layer/material mesh batches.
    pub mesh_batches: u32,
    /// Total vertices across all batches.
    pub vertices: u64,
    /// Total triangles across all batches.
    pub triangles: u64,
    /// Water cells receiving transparent caps.
    pub water_caps: u32,
    /// Exact-once water height or boundary curtains.
    pub water_curtains: u32,
    /// Exact water-to-land edges receiving shore treatments.
    pub shoreline_edges: u32,
    /// Non-bridging ice wedges.
    pub ice_wedges: u32,
    /// Unique waterfall landing spray volumes.
    pub spray_volumes: u32,
    /// World-space cloud clusters.
    pub cloud_clusters: u32,
    /// Low-poly puffs across all cloud clusters.
    pub cloud_puffs: u32,
    /// Local deterministic fog volumes.
    pub fog_volumes: u32,
}

impl ReviewWorldDetailEffectCountsV1 {
    /// Number of disposable renderer entities when each batch or volume is spawned once.
    #[must_use]
    pub fn presentation_entities(self, cloud_shadow_entities: u32) -> u32 {
        self.mesh_batches
            .saturating_add(self.spray_volumes)
            .saturating_add(self.fog_volumes)
            .saturating_add(cloud_shadow_entities)
    }
}

/// Complete disposable projection for the five delegated review families.
#[derive(Debug, Clone, PartialEq)]
pub struct LiquidAtmosphereReviewPlanV1 {
    /// Active treatment ids in water, clouds, shore, ice, fog order.
    pub treatment_ids: [Option<&'static str>; 5],
    /// Frozen or animated review phase supplied by the capture harness.
    pub phase_seconds: f32,
    /// Shared material descriptors, sorted by stable slot.
    pub materials: Vec<ReviewMaterialDescriptorV1>,
    /// Concrete mesh batches, sorted by stable batch key.
    pub mesh_batches: Vec<ReviewMeshBatchV1>,
    /// Exact-once shoreline ownership evidence.
    pub shoreline_edges: Vec<ReviewOwnedShoreEdgeV1>,
    /// Exact-once transparent-curtain ownership evidence.
    pub water_curtain_edges: Vec<ReviewOwnedLiquidEdgeV1>,
    /// Deterministic low-poly puff descriptors.
    pub cloud_puffs: Vec<ReviewCloudPuffV1>,
    /// Optional C08 shadow projectors.
    pub cloud_shadows: Vec<ReviewCloudShadowV1>,
    /// Unique waterfall landing spray volumes.
    pub spray_volumes: Vec<ReviewSprayVolumeV1>,
    /// Deterministic local fog volumes.
    pub fog_volumes: Vec<ReviewFogVolumeV1>,
    /// Camera and render-state requirements with explicit teardown restoration.
    pub render_state: ReviewRenderStateRequirementsV1,
    /// Deterministic semantic/coverage evidence emitted into the runtime report.
    pub effect_validation: ReviewWorldDetailEffectValidationV1,
    /// Stable report cardinalities.
    pub counts: ReviewWorldDetailEffectCountsV1,
    /// Canonical hash of every resolved descriptor except this field.
    pub plan_hash: u64,
}

impl LiquidAtmosphereReviewPlanV1 {
    /// Whether all five delegated families resolve to the shared control.
    #[must_use]
    pub fn is_current(&self) -> bool {
        self.treatment_ids.iter().all(Option::is_none)
    }

    /// Whether the stored hash still matches every resolved descriptor.
    #[must_use]
    pub fn hash_is_valid(&self) -> bool {
        self.plan_hash == canonical_plan_hash(self)
    }

    /// Stable identity of all effect descriptors and mesh streams other than
    /// the review animation phase.
    #[must_use]
    pub fn phase_neutral_hash(&self) -> u64 {
        phase_neutral_plan_hash(self)
    }

    /// Binds a finite animated phase to a previously computed phase-neutral
    /// plan identity without cloning or rebuilding the plan.
    #[must_use]
    pub fn bind_phase_hash(phase_neutral_hash: u64, phase_seconds: f32) -> Option<u64> {
        phase_seconds
            .is_finite()
            .then(|| phase_bound_plan_hash(phase_neutral_hash, phase_seconds))
    }

    /// Number of disposable renderer entities after the renderer resolves the
    /// C08 shadow surface batches against resident terrain chunks.
    #[must_use]
    pub fn presentation_entities(&self, cloud_shadow_entities: u32) -> u32 {
        self.counts.presentation_entities(cloud_shadow_entities)
    }
}

/// Failure to construct finite, unambiguous review geometry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewWorldDetailEffectError {
    /// The caller bypassed the strict parser or supplied a non-matrix profile.
    InvalidProfile(String),
    /// The map level height was non-finite or non-positive.
    InvalidLevelHeight,
    /// A phase, height, radius, or anchor component was non-finite.
    NonFiniteInput,
    /// The cloud field radius was non-positive.
    InvalidCloudFieldRadius,
    /// Two liquid inputs named the same exact surface.
    DuplicateLiquid(TilePos),
    /// A liquid run bottom was above its top voxel.
    InvalidLiquidRun(TilePos),
    /// Two physical-solid inputs named the same exact run top.
    DuplicatePhysicalSolidRun(TilePos),
    /// A physical-solid run bottom was above its top voxel.
    InvalidPhysicalSolidRun(TilePos),
    /// A shore run bottom was above its top voxel.
    InvalidShoreRun(TilePos),
    /// Two shore inputs named the same exact surface.
    DuplicateShoreSurface(TilePos),
    /// A named effect anchor had an empty or duplicate name.
    InvalidEffectAnchor(String),
    /// Active water optics found no exact water presentation cell.
    MissingWaterSurface,
    /// Active shoreline geometry found no eligible water-to-land edge.
    MissingShoreline,
    /// Active ice geometry found no edge admitted by its level/biome rule.
    MissingIceEligibleEdge,
    /// Active plunge spray found no published waterfall landing.
    MissingWaterfallLanding,
    /// A named waterfall anchor did not resolve to one exact fall downstream.
    UnresolvedWaterfallAnchor(String),
    /// The authored waterfall base had more than one equally near landing.
    AmbiguousWaterfallAnchor(String),
    /// Active local fog found no anchor of its requested class.
    MissingFogAnchors,
    /// A grazing/crossing cloud treatment failed to touch the exact peak column.
    MissingCloudPeakIntersection,
    /// An ice width could meet an opposing wedge across a one-cell channel.
    BridgingIceWidth,
    /// A generated mesh exceeded `u32` index capacity.
    MeshIndexOverflow,
    /// A mesh contained a non-finite component, malformed stream, or bad index.
    InvalidMesh,
    /// A triangle was degenerate or wound against its published normals.
    InvalidTriangleWinding,
    /// Deterministic cloud coverage exceeded its hard cluster bound.
    CloudClusterOverflow,
}

impl fmt::Display for ReviewWorldDetailEffectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidProfile(message) => write!(formatter, "invalid review profile: {message}"),
            Self::InvalidLevelHeight => {
                formatter.write_str("review level height must be finite and positive")
            }
            Self::NonFiniteInput => formatter.write_str("review input contains a non-finite value"),
            Self::InvalidCloudFieldRadius => {
                formatter.write_str("cloud field radius must be positive")
            }
            Self::DuplicateLiquid(position) => {
                write!(formatter, "duplicate liquid surface {position:?}")
            }
            Self::InvalidLiquidRun(position) => {
                write!(formatter, "liquid run bottom is above {position:?}")
            }
            Self::DuplicatePhysicalSolidRun(position) => {
                write!(formatter, "duplicate physical-solid run {position:?}")
            }
            Self::InvalidPhysicalSolidRun(position) => {
                write!(formatter, "physical-solid run bottom is above {position:?}")
            }
            Self::InvalidShoreRun(position) => {
                write!(formatter, "shore run bottom is above {position:?}")
            }
            Self::DuplicateShoreSurface(position) => {
                write!(formatter, "duplicate shore surface {position:?}")
            }
            Self::InvalidEffectAnchor(name) => {
                write!(formatter, "invalid or duplicate effect anchor {name:?}")
            }
            Self::MissingWaterSurface => {
                formatter.write_str("water treatment requires an exact water surface")
            }
            Self::MissingShoreline => {
                formatter.write_str("shore treatment requires an eligible water-to-land edge")
            }
            Self::MissingIceEligibleEdge => {
                formatter.write_str("ice treatment has no eligible shoreline edge")
            }
            Self::MissingWaterfallLanding => {
                formatter.write_str("spray treatment requires a waterfall landing")
            }
            Self::UnresolvedWaterfallAnchor(name) => write!(
                formatter,
                "waterfall effect anchor {name:?} does not resolve to a published fall landing"
            ),
            Self::AmbiguousWaterfallAnchor(name) => write!(
                formatter,
                "waterfall effect anchor {name:?} has multiple equally near published fall landings"
            ),
            Self::MissingFogAnchors => {
                formatter.write_str("local fog treatment has no matching named anchors")
            }
            Self::MissingCloudPeakIntersection => formatter
                .write_str("grazing/crossing clouds do not intersect the selected peak column"),
            Self::BridgingIceWidth => formatter.write_str("ice width could bridge opposing shores"),
            Self::MeshIndexOverflow => {
                formatter.write_str("review mesh exceeds u32 index capacity")
            }
            Self::InvalidMesh => {
                formatter.write_str("review mesh is non-finite or structurally malformed")
            }
            Self::InvalidTriangleWinding => {
                formatter.write_str("review triangle is degenerate or incorrectly wound")
            }
            Self::CloudClusterOverflow => {
                formatter.write_str("cloud coverage exceeds bounded cluster capacity")
            }
        }
    }
}

impl std::error::Error for ReviewWorldDetailEffectError {}

/// Builds all active liquid and atmospheric review projections in one pure pass.
pub fn build_liquid_atmosphere_review_plan(
    profile: &ReviewWorldDetailProfileV1,
    input: &LiquidAtmosphereReviewInputV1,
) -> Result<LiquidAtmosphereReviewPlanV1, ReviewWorldDetailEffectError> {
    profile
        .validate()
        .map_err(|error| ReviewWorldDetailEffectError::InvalidProfile(error.to_string()))?;
    validate_input(input)?;

    let mut builder = PlanBuilder::default();
    plan_water(&mut builder, &profile.water, input)?;
    plan_shore_and_falls(&mut builder, &profile.shore_and_falls, input)?;
    plan_ice(&mut builder, &profile.ice_fringe, input)?;
    plan_clouds(&mut builder, &profile.physical_clouds, input)?;
    plan_fog(&mut builder, &profile.local_fog, input)?;

    let mut plan = builder.finish(profile, input.phase_seconds)?;
    plan.plan_hash = canonical_plan_hash(&plan);
    Ok(plan)
}

#[derive(Debug, Default)]
struct PlanBuilder {
    materials: BTreeMap<ReviewMaterialKeyV1, ReviewMaterialDescriptorV1>,
    meshes: BTreeMap<ReviewMeshBatchKeyV1, ReviewIndexedMeshV1>,
    shoreline_edges: BTreeSet<ReviewOwnedShoreEdgeV1>,
    water_curtain_edges: BTreeSet<ReviewOwnedLiquidEdgeV1>,
    cloud_puffs: Vec<ReviewCloudPuffV1>,
    cloud_shadows: Vec<ReviewCloudShadowV1>,
    spray_volumes: Vec<ReviewSprayVolumeV1>,
    fog_volumes: Vec<ReviewFogVolumeV1>,
    render_state: ReviewRenderStateRequirementsV1,
    effect_validation: ReviewWorldDetailEffectValidationV1,
    water_caps: u32,
    ice_wedges: u32,
    cloud_clusters: u32,
}

impl PlanBuilder {
    fn material(&mut self, descriptor: ReviewMaterialDescriptorV1) {
        self.materials.entry(descriptor.key).or_insert(descriptor);
    }

    fn mesh_mut(&mut self, key: ReviewMeshBatchKeyV1) -> &mut ReviewIndexedMeshV1 {
        self.meshes.entry(key).or_default()
    }

    fn finish(
        self,
        profile: &ReviewWorldDetailProfileV1,
        phase_seconds: f32,
    ) -> Result<LiquidAtmosphereReviewPlanV1, ReviewWorldDetailEffectError> {
        let mut mesh_batches = Vec::with_capacity(self.meshes.len());
        let mut vertices = 0_u64;
        let mut triangles = 0_u64;
        for (key, mesh) in self.meshes {
            validate_mesh(&mesh)?;
            vertices = vertices.saturating_add(bounded_u64(mesh.positions.len()));
            triangles = triangles.saturating_add(bounded_u64(mesh.indices.len() / 3));
            mesh_batches.push(ReviewMeshBatchV1 { key, mesh });
        }
        let counts = ReviewWorldDetailEffectCountsV1 {
            materials: bounded_u32(self.materials.len()),
            mesh_batches: bounded_u32(mesh_batches.len()),
            vertices,
            triangles,
            water_caps: self.water_caps,
            water_curtains: bounded_u32(self.water_curtain_edges.len()),
            shoreline_edges: bounded_u32(self.shoreline_edges.len()),
            ice_wedges: self.ice_wedges,
            spray_volumes: bounded_u32(self.spray_volumes.len()),
            cloud_clusters: self.cloud_clusters,
            cloud_puffs: bounded_u32(self.cloud_puffs.len()),
            fog_volumes: bounded_u32(self.fog_volumes.len()),
        };
        Ok(LiquidAtmosphereReviewPlanV1 {
            treatment_ids: [
                profile.water.treatment_id(),
                profile.physical_clouds.treatment_id(),
                profile.shore_and_falls.treatment_id(),
                profile.ice_fringe.treatment_id(),
                profile.local_fog.treatment_id(),
            ],
            phase_seconds,
            materials: self.materials.into_values().collect(),
            mesh_batches,
            shoreline_edges: self.shoreline_edges.into_iter().collect(),
            water_curtain_edges: self.water_curtain_edges.into_iter().collect(),
            cloud_puffs: self.cloud_puffs,
            cloud_shadows: self.cloud_shadows,
            spray_volumes: self.spray_volumes,
            fog_volumes: self.fog_volumes,
            render_state: self.render_state,
            effect_validation: self.effect_validation,
            counts,
            plan_hash: 0,
        })
    }
}

fn validate_input(
    input: &LiquidAtmosphereReviewInputV1,
) -> Result<(), ReviewWorldDetailEffectError> {
    if !input.level_height.is_finite() || input.level_height <= 0.0 {
        return Err(ReviewWorldDetailEffectError::InvalidLevelHeight);
    }
    if !input.phase_seconds.is_finite()
        || !input.max_exposed_natural_y.is_finite()
        || !vec3_is_finite(input.massif_crest)
        || !vec3_is_finite(input.interaction_peak)
    {
        return Err(ReviewWorldDetailEffectError::NonFiniteInput);
    }
    if !input.cloud_field_radius.is_finite() {
        return Err(ReviewWorldDetailEffectError::NonFiniteInput);
    }
    if input.cloud_field_radius <= 0.0 {
        return Err(ReviewWorldDetailEffectError::InvalidCloudFieldRadius);
    }
    if input.interaction_peak_solid_spans.is_empty()
        || (input.interaction_peak.y - input.max_exposed_natural_y).abs() > 1.0e-4
        || Vec2::new(
            input.interaction_peak.x - input.massif_crest.x,
            input.interaction_peak.z - input.massif_crest.z,
        )
        .length()
            > input.cloud_field_radius
    {
        return Err(ReviewWorldDetailEffectError::MissingCloudPeakIntersection);
    }
    let mut previous_top = f32::NEG_INFINITY;
    for span in &input.interaction_peak_solid_spans {
        if !span.bottom_y.is_finite()
            || !span.top_y.is_finite()
            || span.bottom_y >= span.top_y
            || span.bottom_y < previous_top
        {
            return Err(ReviewWorldDetailEffectError::MissingCloudPeakIntersection);
        }
        previous_top = span.top_y;
    }
    if !input
        .interaction_peak_solid_spans
        .iter()
        .any(|span| (span.top_y - input.interaction_peak.y).abs() <= 1.0e-4)
    {
        return Err(ReviewWorldDetailEffectError::MissingCloudPeakIntersection);
    }

    let mut liquids = BTreeSet::new();
    for cell in &input.liquids {
        if !liquids.insert(cell.position) {
            return Err(ReviewWorldDetailEffectError::DuplicateLiquid(cell.position));
        }
        if cell.run_bottom > cell.position.level {
            return Err(ReviewWorldDetailEffectError::InvalidLiquidRun(
                cell.position,
            ));
        }
    }
    let mut physical_solids = BTreeSet::new();
    for run in &input.physical_solid_runs {
        if !physical_solids.insert(run.position) {
            return Err(ReviewWorldDetailEffectError::DuplicatePhysicalSolidRun(
                run.position,
            ));
        }
        if run.run_bottom > run.position.level {
            return Err(ReviewWorldDetailEffectError::InvalidPhysicalSolidRun(
                run.position,
            ));
        }
    }
    let mut shores = BTreeSet::new();
    for surface in &input.shore_surfaces {
        if !shores.insert(surface.position) {
            return Err(ReviewWorldDetailEffectError::DuplicateShoreSurface(
                surface.position,
            ));
        }
        if surface.run_bottom > surface.position.level {
            return Err(ReviewWorldDetailEffectError::InvalidShoreRun(
                surface.position,
            ));
        }
    }
    let mut names = BTreeSet::new();
    for anchor in &input.effect_anchors {
        if anchor.name.trim().is_empty()
            || !names.insert(anchor.name.as_str())
            || !vec3_is_finite(anchor.surface)
        {
            return Err(ReviewWorldDetailEffectError::InvalidEffectAnchor(
                anchor.name.clone(),
            ));
        }
    }
    Ok(())
}

fn vec3_is_finite(value: Vec3) -> bool {
    value.to_array().into_iter().all(f32::is_finite)
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "report counts saturate explicitly before this bounded conversion"
)]
fn bounded_u32(value: usize) -> u32 {
    value.min(u32::MAX as usize) as u32
}

fn bounded_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn depth_value_multiplier(depth_world: f32, half_distance: f32, deep_value: f32) -> f32 {
    let half_distances = (depth_world / half_distance).max(0.0);
    deep_value + (1.0 - deep_value) * 2.0_f32.powf(-half_distances)
}

fn water_material(
    treatment: &WaterDetailV1,
    style: ReviewWaterMaterialStyleV1,
) -> ReviewMaterialDescriptorV1 {
    let mut descriptor = ReviewMaterialDescriptorV1 {
        key: ReviewMaterialKeyV1::Water { style },
        alpha: None,
        value_multiplier: 1.0,
        roughness: None,
        roughness_delta: None,
        reflectance: None,
        depth_half_distance: None,
        deep_value_multiplier: None,
        transmission: None,
        alpha_mode: ReviewAlphaModeV1::OrderIndependentTransparency,
        double_sided: style == ReviewWaterMaterialStyleV1::Fall,
        inward_feather: None,
    };
    match treatment {
        WaterDetailV1::Current => {}
        WaterDetailV1::UniformAlpha { alpha } => descriptor.alpha = Some(*alpha),
        WaterDetailV1::DepthAbsorption {
            alpha,
            depth_half_distance,
            deep_value_multiplier,
        } => {
            descriptor.alpha = Some(*alpha);
            descriptor.depth_half_distance = Some(*depth_half_distance);
            descriptor.deep_value_multiplier = Some(*deep_value_multiplier);
        }
        WaterDetailV1::Transmission {
            ior,
            thickness,
            max_refraction_uv,
        } => {
            descriptor.alpha_mode = ReviewAlphaModeV1::Opaque;
            descriptor.transmission = Some(ReviewTransmissionV1 {
                ior: *ior,
                thickness: *thickness,
                max_refraction_uv: *max_refraction_uv,
            });
        }
        WaterDetailV1::RoughSurface {
            alpha,
            roughness,
            reflectance,
        } => {
            descriptor.alpha = Some(*alpha);
            descriptor.roughness = Some(*roughness);
            descriptor.reflectance = Some(*reflectance);
        }
    }
    descriptor
}

fn plan_water(
    builder: &mut PlanBuilder,
    treatment: &WaterDetailV1,
    input: &LiquidAtmosphereReviewInputV1,
) -> Result<(), ReviewWorldDetailEffectError> {
    if matches!(treatment, WaterDetailV1::Current) {
        return Ok(());
    }
    let water_cells = water_cells(input);
    if water_cells.is_empty() {
        return Err(ReviewWorldDetailEffectError::MissingWaterSurface);
    }
    builder.render_state.suppress_opaque_water_geometry = true;
    builder.render_state.restore_opaque_water_geometry = true;
    builder.render_state.order_independent_transparency = treatment.requires_oit();
    builder.render_state.restore_order_independent_transparency = treatment.requires_oit();
    if matches!(treatment, WaterDetailV1::Transmission { .. }) {
        builder.render_state.medium_screen_space_transmission = true;
        builder.render_state.restore_screen_space_transmission = true;
    }

    let depth_response = match treatment {
        WaterDetailV1::DepthAbsorption {
            depth_half_distance,
            deep_value_multiplier,
            ..
        } => Some((*depth_half_distance, *deep_value_multiplier)),
        _ => None,
    };
    for cell in water_cells {
        let depth = liquid_depth_world(cell, input.level_height);
        let value_multiplier = depth_response.map_or(1.0, |(half_distance, deep_value)| {
            depth_value_multiplier(depth, half_distance, deep_value)
        });
        let vertex_color = [value_multiplier, value_multiplier, value_multiplier, 1.0];
        let surface_style = ReviewWaterMaterialStyleV1::Surface;
        let material = water_material(treatment, surface_style);
        builder.material(material);
        let cap_key = ReviewMeshBatchKeyV1 {
            chunk: cell.chunk,
            layer: ReviewMeshLayerV1::WaterCaps,
            material: ReviewMaterialKeyV1::Water {
                style: surface_style,
            },
        };
        append_hex_cap(
            builder.mesh_mut(cap_key),
            cell.position.coord,
            surface_y(cell.position.level, input.level_height) + WATER_CAP_BIAS,
            vertex_color,
        )?;
        builder.water_caps = builder.water_caps.saturating_add(1);

        for side in ReviewHexSideV1::ALL {
            if let Some((bottom_y, adjacent_surface)) = curtain_bottom(cell, side, input) {
                let top_y = surface_y(cell.position.level, input.level_height) + WATER_CAP_BIAS;
                if top_y - bottom_y > 1.0e-5 {
                    let style = if cell.flow == ReviewLiquidFlowV1::Fall
                        && cell.downstream.is_some_and(|downstream| {
                            downstream.coord == side.neighbor(cell.position.coord)
                        }) {
                        ReviewWaterMaterialStyleV1::Fall
                    } else {
                        ReviewWaterMaterialStyleV1::Surface
                    };
                    builder.material(water_material(treatment, style));
                    let curtain_key = ReviewMeshBatchKeyV1 {
                        chunk: cell.chunk,
                        layer: ReviewMeshLayerV1::WaterCurtains,
                        material: ReviewMaterialKeyV1::Water { style },
                    };
                    append_vertical_edge_quad(
                        builder.mesh_mut(curtain_key),
                        cell.position.coord,
                        side,
                        top_y,
                        bottom_y,
                        vertex_color,
                    )?;
                    builder.water_curtain_edges.insert(ReviewOwnedLiquidEdgeV1 {
                        water: cell.position,
                        side,
                        adjacent_surface,
                    });
                }
            }
        }
    }
    Ok(())
}

fn water_cells(input: &LiquidAtmosphereReviewInputV1) -> Vec<&ReviewLiquidCellV1> {
    let mut cells = input
        .liquids
        .iter()
        .filter(|cell| cell.kind == ReviewLiquidKindV1::Water)
        .collect::<Vec<_>>();
    cells.sort_by_key(|cell| cell.position);
    cells
}

#[expect(
    clippy::cast_precision_loss,
    reason = "review voxel levels remain many orders of magnitude below f32 integer precision"
)]
fn surface_y(level: Level, level_height: f32) -> f32 {
    level.saturating_add(1) as f32 * level_height
}

#[expect(
    clippy::cast_precision_loss,
    reason = "review voxel levels remain many orders of magnitude below f32 integer precision"
)]
fn level_boundary_y(level: Level, level_height: f32) -> f32 {
    level as f32 * level_height
}

#[expect(
    clippy::cast_precision_loss,
    reason = "review voxel run depths remain many orders of magnitude below f32 integer precision"
)]
fn liquid_depth_world(cell: &ReviewLiquidCellV1, level_height: f32) -> f32 {
    let levels = cell
        .position
        .level
        .saturating_sub(cell.run_bottom)
        .saturating_add(1);
    levels as f32 * level_height
}

fn liquid_run_at_coord<'a>(
    input: &'a LiquidAtmosphereReviewInputV1,
    coord: HexCoord,
    source: &ReviewLiquidCellV1,
) -> Option<&'a ReviewLiquidCellV1> {
    input
        .liquids
        .iter()
        .filter(|candidate| {
            candidate.kind == ReviewLiquidKindV1::Water && candidate.position.coord == coord
        })
        // Only a run whose inclusive vertical interval overlaps or directly
        // adjoins the source run can own this source edge. A bridge/overhang may
        // legitimately leave another, unrelated water run at the same axial
        // coordinate; choosing the highest one would collapse that stack.
        .filter(|candidate| {
            candidate.run_bottom <= source.position.level.saturating_add(1)
                && candidate.position.level.saturating_add(1) >= source.run_bottom
        })
        .min_by_key(|candidate| {
            let contains_source_surface = candidate.run_bottom <= source.position.level
                && candidate.position.level >= source.position.level;
            (
                !contains_source_surface,
                candidate.position.level.abs_diff(source.position.level),
                std::cmp::Reverse(candidate.position.level),
                candidate.run_bottom,
                candidate.position,
            )
        })
}

fn waterline_shore_at_coord<'a>(
    input: &'a LiquidAtmosphereReviewInputV1,
    coord: HexCoord,
    water: &ReviewLiquidCellV1,
) -> Option<&'a ReviewShoreSurfaceV1> {
    input
        .shore_surfaces
        .iter()
        .filter(|surface| {
            surface.eligible
                && surface.position.coord == coord
                && surface.run_bottom <= water.position.level
                && surface.position.level >= water.position.level
        })
        .min_by_key(|surface| {
            (
                surface.position.level.abs_diff(water.position.level),
                std::cmp::Reverse(surface.position.level),
                std::cmp::Reverse(surface.run_bottom),
                surface.position,
            )
        })
}

fn physical_solid_runs_at_coord<'a>(
    input: &'a LiquidAtmosphereReviewInputV1,
    coord: HexCoord,
) -> impl Iterator<Item = &'a ReviewPhysicalSolidRunV1> + 'a {
    input
        .physical_solid_runs
        .iter()
        .filter(move |run| run.position.coord == coord)
}

fn curtain_bottom(
    cell: &ReviewLiquidCellV1,
    side: ReviewHexSideV1,
    input: &LiquidAtmosphereReviewInputV1,
) -> Option<(f32, Option<TilePos>)> {
    let neighbor_coord = side.neighbor(cell.position.coord);
    if let Some(neighbor) = liquid_run_at_coord(input, neighbor_coord, cell) {
        if neighbor.position.level >= cell.position.level {
            return None;
        }
        return Some((
            surface_y(neighbor.position.level, input.level_height),
            Some(neighbor.position),
        ));
    }
    // A solid run containing the source waterline physically closes this side,
    // whether or not that surface is eligible for decorative shore treatments.
    // In particular, portal/structure exclusions must not make water render
    // through authoritative terrain.
    if physical_solid_runs_at_coord(input, neighbor_coord).any(|run| {
        run.run_bottom <= cell.position.level && run.position.level >= cell.position.level
    }) {
        return None;
    }
    let run_bottom_y = level_boundary_y(cell.run_bottom, input.level_height);
    // A lower adjacent solid run may support only part of the water column. Use
    // the highest run that actually overlaps that column; ignore unrelated
    // ground below it and floating overhangs above it.
    let lower_solid = physical_solid_runs_at_coord(input, neighbor_coord)
        .filter(|run| {
            run.position.level >= cell.run_bottom
                && run.position.level < cell.position.level
                && run.run_bottom <= cell.position.level
        })
        .max_by_key(|run| {
            (
                run.position.level,
                run.run_bottom,
                std::cmp::Reverse(run.position),
            )
        });
    Some(lower_solid.map_or((run_bottom_y, None), |run| {
        (
            surface_y(run.position.level, input.level_height).max(run_bottom_y),
            Some(run.position),
        )
    }))
}

fn shoreline_edges(input: &LiquidAtmosphereReviewInputV1) -> Vec<ReviewOwnedShoreEdgeV1> {
    let mut edges = BTreeSet::new();
    for water in water_cells(input) {
        for side in ReviewHexSideV1::ALL {
            let neighbor = side.neighbor(water.position.coord);
            if liquid_run_at_coord(input, neighbor, water).is_some() {
                continue;
            }
            if let Some(land) = waterline_shore_at_coord(input, neighbor, water) {
                edges.insert(ReviewOwnedShoreEdgeV1 {
                    water: water.position,
                    side,
                    land: land.position,
                });
            }
        }
    }
    edges.into_iter().collect()
}

fn plan_shore_and_falls(
    builder: &mut PlanBuilder,
    treatment: &ShoreAndFallsDetailV1,
    input: &LiquidAtmosphereReviewInputV1,
) -> Result<(), ReviewWorldDetailEffectError> {
    if matches!(treatment, ShoreAndFallsDetailV1::Current) {
        return Ok(());
    }
    let edges = shoreline_edges(input);
    let wettable_edges = edges
        .iter()
        .copied()
        .filter(immediate_water_bank)
        .collect::<Vec<_>>();

    match treatment {
        ShoreAndFallsDetailV1::Current => {}
        ShoreAndFallsDetailV1::WetRim {
            width,
            value_delta,
            roughness_delta,
        } => {
            if wettable_edges.is_empty() {
                return Err(ReviewWorldDetailEffectError::MissingShoreline);
            }
            builder
                .shoreline_edges
                .extend(wettable_edges.iter().copied());
            plan_wet_rims(
                builder,
                input,
                &wettable_edges,
                *width,
                *value_delta,
                *roughness_delta,
            )?;
        }
        ShoreAndFallsDetailV1::Foam { width, opacity } => {
            if edges.is_empty() {
                return Err(ReviewWorldDetailEffectError::MissingShoreline);
            }
            builder.shoreline_edges.extend(edges.iter().copied());
            plan_foam(builder, input, &edges, *width, *opacity)?;
        }
        ShoreAndFallsDetailV1::PlungeSpray {
            radius_hexes,
            height,
            opacity,
            pool_foam_radius_hexes,
        } => plan_plunge_spray(
            builder,
            input,
            *radius_hexes,
            *height,
            *opacity,
            *pool_foam_radius_hexes,
        )?,
        ShoreAndFallsDetailV1::RestrainedCombination {
            wet_rim_width,
            wet_rim_value_delta,
            wet_rim_roughness_delta,
            foam_width,
            foam_opacity,
            spray_radius_hexes,
            spray_height,
            spray_opacity,
            pool_foam_radius_hexes,
        } => {
            if edges.is_empty() || wettable_edges.is_empty() {
                return Err(ReviewWorldDetailEffectError::MissingShoreline);
            }
            builder.shoreline_edges.extend(edges.iter().copied());
            plan_wet_rims(
                builder,
                input,
                &wettable_edges,
                *wet_rim_width,
                *wet_rim_value_delta,
                *wet_rim_roughness_delta,
            )?;
            plan_foam(builder, input, &edges, *foam_width, *foam_opacity)?;
            plan_plunge_spray(
                builder,
                input,
                *spray_radius_hexes,
                *spray_height,
                *spray_opacity,
                *pool_foam_radius_hexes,
            )?;
        }
    }
    Ok(())
}

fn plan_wet_rims(
    builder: &mut PlanBuilder,
    input: &LiquidAtmosphereReviewInputV1,
    edges: &[ReviewOwnedShoreEdgeV1],
    width: f32,
    value_delta: f32,
    roughness_delta: f32,
) -> Result<(), ReviewWorldDetailEffectError> {
    // The rim is an opaque, collisionless presentation cap using the current
    // substrate colour and its actual roughness minus 0.15. A translucent black
    // overlay could darken the composited pixel, but it cannot change the
    // substrate's PBR roughness by a defined amount. Grouping by substance keeps
    // the replacement exact while retaining shared, never-per-cell materials.
    let mut edges_by_substrate = BTreeMap::<SubstanceId, Vec<ReviewOwnedShoreEdgeV1>>::new();
    for edge in edges {
        let shore = input
            .shore_surfaces
            .iter()
            .find(|surface| surface.position == edge.land)
            .ok_or(ReviewWorldDetailEffectError::InvalidMesh)?;
        edges_by_substrate
            .entry(shore.substance)
            .or_default()
            .push(*edge);
    }
    for (substrate, substrate_edges) in edges_by_substrate {
        let material = ReviewMaterialKeyV1::WetRim { substrate };
        builder.material(ReviewMaterialDescriptorV1 {
            key: material,
            alpha: Some(1.0),
            value_multiplier: (1.0 + value_delta).clamp(0.0, 1.0),
            roughness: None,
            roughness_delta: Some(roughness_delta),
            reflectance: None,
            depth_half_distance: None,
            deep_value_multiplier: None,
            transmission: None,
            alpha_mode: ReviewAlphaModeV1::Opaque,
            double_sided: false,
            inward_feather: None,
        });
        for edge in &substrate_edges {
            let shore = input
                .shore_surfaces
                .iter()
                .find(|surface| surface.position == edge.land)
                .ok_or(ReviewWorldDetailEffectError::InvalidMesh)?;
            let key = ReviewMeshBatchKeyV1 {
                chunk: shore.chunk,
                layer: ReviewMeshLayerV1::WetRims,
                material,
            };
            append_attached_edge_strip(
                builder.mesh_mut(key),
                edge.water.coord,
                edge.side,
                surface_y(edge.land.level, input.level_height) + ATTACHED_SURFACE_BIAS,
                width,
                StripDirection::TowardLand,
            )?;
        }
        append_cross_owner_strip_junctions(
            builder,
            input,
            &substrate_edges,
            ReviewMeshLayerV1::WetRims,
            material,
            0.0,
            width,
            StripDirection::TowardLand,
            StripAttachmentSurface::Land,
            ATTACHED_SURFACE_BIAS,
            None,
        )?;
    }
    Ok(())
}

fn plan_foam(
    builder: &mut PlanBuilder,
    input: &LiquidAtmosphereReviewInputV1,
    edges: &[ReviewOwnedShoreEdgeV1],
    width: f32,
    opacity: f32,
) -> Result<(), ReviewWorldDetailEffectError> {
    builder.render_state.order_independent_transparency = true;
    builder.render_state.restore_order_independent_transparency = true;
    builder.material(foam_material(opacity));
    for edge in edges {
        let Some(water) = input
            .liquids
            .iter()
            .find(|cell| cell.position == edge.water)
        else {
            continue;
        };
        let key = ReviewMeshBatchKeyV1 {
            chunk: water.chunk,
            layer: ReviewMeshLayerV1::ShoreFoam,
            material: ReviewMaterialKeyV1::Foam,
        };
        append_attached_edge_strip(
            builder.mesh_mut(key),
            edge.water.coord,
            edge.side,
            surface_y(edge.water.level, input.level_height) + ATTACHED_SURFACE_BIAS,
            width,
            StripDirection::TowardWater,
        )?;
    }
    append_cross_owner_strip_junctions(
        builder,
        input,
        edges,
        ReviewMeshLayerV1::ShoreFoam,
        ReviewMaterialKeyV1::Foam,
        0.0,
        width,
        StripDirection::TowardWater,
        StripAttachmentSurface::Water,
        ATTACHED_SURFACE_BIAS,
        None,
    )?;
    Ok(())
}

fn foam_material(opacity: f32) -> ReviewMaterialDescriptorV1 {
    ReviewMaterialDescriptorV1 {
        key: ReviewMaterialKeyV1::Foam,
        alpha: Some(opacity),
        value_multiplier: 1.0,
        roughness: Some(0.75),
        roughness_delta: None,
        reflectance: Some(0.25),
        depth_half_distance: None,
        deep_value_multiplier: None,
        transmission: None,
        alpha_mode: ReviewAlphaModeV1::OrderIndependentTransparency,
        double_sided: true,
        inward_feather: None,
    }
}

fn plan_plunge_spray(
    builder: &mut PlanBuilder,
    input: &LiquidAtmosphereReviewInputV1,
    radius_hexes: u8,
    height: f32,
    opacity: f32,
    pool_foam_radius_hexes: u8,
) -> Result<(), ReviewWorldDetailEffectError> {
    let published_landings = input
        .liquids
        .iter()
        .filter(|cell| {
            cell.kind == ReviewLiquidKindV1::Water && cell.flow == ReviewLiquidFlowV1::Fall
        })
        .filter_map(|cell| cell.downstream)
        .collect::<BTreeSet<_>>();
    if published_landings.is_empty() {
        return Err(ReviewWorldDetailEffectError::MissingWaterfallLanding);
    }
    const PLUNGE_ANCHOR: &str = "grand_v3.waterfall_base";
    const MAXIMUM_AUTHORED_DISPLACEMENT: u32 = 13;
    let anchor = input
        .effect_anchors
        .iter()
        .find(|anchor| {
            anchor.kind == ReviewEffectAnchorKindV1::Waterfall && anchor.name == PLUNGE_ANCHOR
        })
        .ok_or_else(|| {
            ReviewWorldDetailEffectError::UnresolvedWaterfallAnchor(PLUNGE_ANCHOR.to_owned())
        })?;
    let mut ranked_landings = published_landings
        .into_iter()
        .map(|landing| {
            (
                (
                    anchor.position.coord.distance(landing.coord),
                    anchor.position.level.abs_diff(landing.level),
                ),
                landing,
            )
        })
        .collect::<Vec<_>>();
    ranked_landings.sort_by_key(|(rank, landing)| (*rank, *landing));
    let Some((best_rank, landing)) = ranked_landings.first().copied() else {
        return Err(ReviewWorldDetailEffectError::MissingWaterfallLanding);
    };
    if best_rank.0 > MAXIMUM_AUTHORED_DISPLACEMENT {
        return Err(ReviewWorldDetailEffectError::UnresolvedWaterfallAnchor(
            anchor.name.clone(),
        ));
    }
    if ranked_landings
        .get(1)
        .is_some_and(|(next_rank, _)| *next_rank == best_rank)
    {
        return Err(ReviewWorldDetailEffectError::AmbiguousWaterfallAnchor(
            anchor.name.clone(),
        ));
    }
    builder.render_state.order_independent_transparency = true;
    builder.render_state.restore_order_independent_transparency = true;
    builder.material(foam_material(opacity.max(0.35)));

    let center = landing
        .coord
        .to_world(surface_y(landing.level, input.level_height));
    builder.spray_volumes.push(ReviewSprayVolumeV1 {
        anchor_name: anchor.name.clone(),
        anchor_position: anchor.position,
        anchor_distance_hexes: best_rank.0,
        landing,
        center: center + Vec3::Y * (height * 0.5 + ATTACHED_SURFACE_BIAS),
        radius: f32::from(radius_hexes) * HEX_SMALL_DIAMETER,
        height,
        opacity,
    });
    let chunk = input
        .liquids
        .iter()
        .find(|cell| cell.position == landing)
        .ok_or_else(|| {
            ReviewWorldDetailEffectError::UnresolvedWaterfallAnchor(anchor.name.clone())
        })?
        .chunk;
    let key = ReviewMeshBatchKeyV1 {
        chunk,
        layer: ReviewMeshLayerV1::PoolFoam,
        material: ReviewMaterialKeyV1::Foam,
    };
    append_disc(
        builder.mesh_mut(key),
        center + Vec3::Y * ATTACHED_SURFACE_BIAS,
        f32::from(pool_foam_radius_hexes) * HEX_SMALL_DIAMETER,
        24,
    )?;
    builder.effect_validation.waterfall_anchors = vec![ReviewWaterfallAnchorEvidenceV1 {
        anchor_name: anchor.name.clone(),
        anchor_position: tile_position_array(anchor.position),
        landing_position: tile_position_array(landing),
        distance_hexes: best_rank.0,
    }];
    Ok(())
}

fn tile_position_array(position: TilePos) -> [i32; 3] {
    [position.coord.x(), position.coord.y(), position.level]
}

fn plan_ice(
    builder: &mut PlanBuilder,
    treatment: &IceFringeDetailV1,
    input: &LiquidAtmosphereReviewInputV1,
) -> Result<(), ReviewWorldDetailEffectError> {
    if matches!(treatment, IceFringeDetailV1::Current) {
        return Ok(());
    }
    let (width, coverage, alpha, roughness, reflectance, y_bias, inward_feather) = match treatment {
        IceFringeDetailV1::Current => return Ok(()),
        IceFringeDetailV1::LevelFringe {
            width,
            coverage,
            alpha,
            roughness,
            reflectance,
            y_bias,
            ..
        }
        | IceFringeDetailV1::SnowAdjacent {
            width,
            coverage,
            alpha,
            roughness,
            reflectance,
            y_bias,
            ..
        } => (
            *width,
            *coverage,
            *alpha,
            *roughness,
            *reflectance,
            *y_bias,
            None,
        ),
        IceFringeDetailV1::Feathered {
            width,
            coverage,
            alpha,
            roughness,
            reflectance,
            y_bias,
            inward_feather,
            ..
        } => (
            *width,
            *coverage,
            *alpha,
            *roughness,
            *reflectance,
            *y_bias,
            Some(*inward_feather),
        ),
    };
    if width >= HEX_INRADIUS {
        return Err(ReviewWorldDetailEffectError::BridgingIceWidth);
    }
    builder.render_state.order_independent_transparency = true;
    builder.render_state.restore_order_independent_transparency = true;
    builder.material(ReviewMaterialDescriptorV1 {
        key: ReviewMaterialKeyV1::Ice,
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
        inward_feather,
    });
    let eligible = shoreline_edges(input)
        .into_iter()
        .filter(|edge| ice_edge_eligible(treatment, edge, input))
        .collect::<Vec<_>>();
    if eligible.is_empty() {
        return Err(ReviewWorldDetailEffectError::MissingIceEligibleEdge);
    }
    let selected_count = exact_coverage_count(eligible.len(), coverage);
    let mut selected = eligible.clone();
    selected.sort_by_key(|edge| {
        (
            // Every ice treatment uses one stable shoreline rank. Coverage
            // changes therefore reveal a nested prefix instead of silently
            // replacing the compared shoreline sample.
            coverage_score(input.seed, ICE_COVERAGE_DOMAIN, &edge_bytes(*edge)),
            *edge,
        )
    });
    selected.truncate(selected_count);
    selected.sort_unstable();
    let solid_width = width - inward_feather.unwrap_or(0.0);
    for edge in &selected {
        let water = input
            .liquids
            .iter()
            .find(|cell| cell.position == edge.water)
            .ok_or(ReviewWorldDetailEffectError::MissingIceEligibleEdge)?;
        let y = surface_y(edge.water.level, input.level_height) + y_bias;
        if solid_width > f32::EPSILON {
            let key = ReviewMeshBatchKeyV1 {
                chunk: water.chunk,
                layer: ReviewMeshLayerV1::IceFringes,
                material: ReviewMaterialKeyV1::Ice,
            };
            append_attached_edge_strip(
                builder.mesh_mut(key),
                edge.water.coord,
                edge.side,
                y,
                solid_width,
                StripDirection::TowardWater,
            )?;
        }
        if let Some(feather_width) = inward_feather {
            let key = ReviewMeshBatchKeyV1 {
                chunk: water.chunk,
                layer: ReviewMeshLayerV1::IceFringes,
                material: ReviewMaterialKeyV1::Ice,
            };
            let positions = attached_edge_band_positions(
                edge.water.coord,
                edge.side,
                y,
                solid_width,
                solid_width + feather_width,
                StripDirection::TowardWater,
            );
            append_upward_quad_with_alphas(builder.mesh_mut(key), positions, [1.0, 0.0, 0.0, 1.0])?;
        }
        builder.ice_wedges = builder.ice_wedges.saturating_add(1);
    }
    builder.shoreline_edges.extend(selected.iter().copied());
    builder.effect_validation.ice_coverage = Some(ReviewIceCoverageEvidenceV1 {
        target_fraction: coverage,
        eligible_edges: bounded_u32(eligible.len()),
        selected_edges: bounded_u32(selected.len()),
    });
    if solid_width > f32::EPSILON {
        append_cross_owner_strip_junctions(
            builder,
            input,
            &selected,
            ReviewMeshLayerV1::IceFringes,
            ReviewMaterialKeyV1::Ice,
            0.0,
            solid_width,
            StripDirection::TowardWater,
            StripAttachmentSurface::Water,
            y_bias,
            None,
        )?;
    }
    if let Some(feather_width) = inward_feather {
        append_cross_owner_strip_junctions(
            builder,
            input,
            &selected,
            ReviewMeshLayerV1::IceFringes,
            ReviewMaterialKeyV1::Ice,
            solid_width,
            solid_width + feather_width,
            StripDirection::TowardWater,
            StripAttachmentSurface::Water,
            y_bias,
            Some([1.0, 0.0]),
        )?;
    }
    Ok(())
}

fn ice_edge_eligible(
    treatment: &IceFringeDetailV1,
    edge: &ReviewOwnedShoreEdgeV1,
    input: &LiquidAtmosphereReviewInputV1,
) -> bool {
    match treatment {
        IceFringeDetailV1::Current => false,
        IceFringeDetailV1::LevelFringe { minimum_level, .. } => {
            i32::from(*minimum_level) <= edge.water.level
        }
        IceFringeDetailV1::SnowAdjacent { include_frozen, .. }
        | IceFringeDetailV1::Feathered { include_frozen, .. } => {
            immediate_water_bank(edge)
                && input
                    .shore_surfaces
                    .iter()
                    .find(|surface| surface.position == edge.land)
                    .is_some_and(|surface| {
                        surface.snow_covered || (*include_frozen && surface.frozen_biome)
                    })
        }
    }
}

/// Whether a top-attached bank treatment remains physically adjacent to the
/// water instead of jumping to a remote cliff top.
///
/// Grand V3 intentionally constructs ordinary lake and sea banks one voxel
/// above their water surface.  Treat that immediate step like a flush bank,
/// while retaining the existing rejection of taller containing cliffs.
fn immediate_water_bank(edge: &ReviewOwnedShoreEdgeV1) -> bool {
    edge.land.level == edge.water.level || edge.land.level == edge.water.level.saturating_add(1)
}

fn edge_bytes(edge: ReviewOwnedShoreEdgeV1) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(24);
    bytes.extend_from_slice(&edge.water.coord.to_bytes());
    bytes.extend_from_slice(&edge.water.level.to_le_bytes());
    bytes.extend_from_slice(&edge.land.coord.to_bytes());
    bytes.extend_from_slice(&edge.land.level.to_le_bytes());
    bytes.push(side_code(edge.side));
    bytes
}

fn side_code(side: ReviewHexSideV1) -> u8 {
    match side {
        ReviewHexSideV1::East => 0,
        ReviewHexSideV1::SouthEast => 1,
        ReviewHexSideV1::SouthWest => 2,
        ReviewHexSideV1::West => 3,
        ReviewHexSideV1::NorthWest => 4,
        ReviewHexSideV1::NorthEast => 5,
    }
}

fn plan_clouds(
    builder: &mut PlanBuilder,
    treatment: &PhysicalCloudsDetailV1,
    input: &LiquidAtmosphereReviewInputV1,
) -> Result<(), ReviewWorldDetailEffectError> {
    let Some(spec) = cloud_spec(treatment, input.max_exposed_natural_y) else {
        return Ok(());
    };
    builder.render_state.order_independent_transparency = true;
    builder.render_state.restore_order_independent_transparency = true;
    builder.material(ReviewMaterialDescriptorV1 {
        key: ReviewMaterialKeyV1::Cloud,
        alpha: Some(0.84),
        value_multiplier: 1.0,
        roughness: Some(0.92),
        roughness_delta: None,
        reflectance: Some(0.05),
        depth_half_distance: None,
        deep_value_multiplier: None,
        transmission: None,
        alpha_mode: ReviewAlphaModeV1::OrderIndependentTransparency,
        double_sided: true,
        inward_feather: None,
    });

    let (clusters, coverage_evidence) = cloud_cluster_layout(input, spec)?;
    let key = ReviewMeshBatchKeyV1 {
        chunk: GLOBAL_BATCH_CHUNK,
        layer: ReviewMeshLayerV1::CloudPuffs,
        material: ReviewMaterialKeyV1::Cloud,
    };
    for cluster in &clusters {
        for puff in cluster.puffs.iter().copied() {
            if !cloud_puff_within_cluster_envelope(puff) {
                return Err(ReviewWorldDetailEffectError::InvalidMesh);
            }
            append_octahedron(builder.mesh_mut(key), puff)?;
            builder.cloud_puffs.push(puff);
        }
        if let Some((maximum_opacity, blur_world)) = spec.shadow {
            builder.cloud_shadows.push(ReviewCloudShadowV1 {
                cluster_index: cluster.index,
                center_xz: Vec2::new(cluster.center.x, cluster.center.z),
                diameter: cluster.diameter,
                maximum_opacity,
                blur_world,
            });
        }
    }
    builder.cloud_clusters = bounded_u32(clusters.len());
    builder.effect_validation.cloud_coverage = Some(coverage_evidence);
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct CloudSpec {
    shape: ReviewCloudPrimitiveShapeV1,
    coverage: f32,
    diameter_min: f32,
    diameter_max: f32,
    altitude_min: f32,
    altitude_max: f32,
    peak_intersection_required: bool,
    shadow: Option<(f32, f32)>,
}

#[derive(Debug, Clone)]
struct CloudClusterLayout {
    index: u32,
    center: Vec3,
    diameter: f32,
    puffs: Vec<ReviewCloudPuffV1>,
}

#[derive(Debug, Clone, Copy)]
struct CloudPuffProjection {
    center: Vec2,
    local_x_axis: Vec2,
    local_z_axis: Vec2,
    half_x: f32,
    half_z: f32,
}

impl CloudPuffProjection {
    fn from_puff(puff: ReviewCloudPuffV1) -> Self {
        // `append_octahedron` projects its top and bottom vertices to the puff
        // centre in XZ. Its other four vertices therefore form the complete
        // projected silhouette: this rotated diamond, not the cluster envelope.
        let rotation = Quat::from_rotation_y(puff.yaw);
        let local_x = rotation * Vec3::X;
        let local_z = rotation * Vec3::Z;
        Self {
            center: Vec2::new(puff.center.x, puff.center.z),
            local_x_axis: Vec2::new(local_x.x, local_x.z),
            local_z_axis: Vec2::new(local_z.x, local_z.z),
            half_x: puff.half_extents.x,
            half_z: puff.half_extents.z,
        }
    }

    fn contains(&self, point: Vec2) -> bool {
        let relative = point - self.center;
        let local_x = relative.dot(self.local_x_axis).abs();
        let local_z = relative.dot(self.local_z_axis).abs();
        local_x * self.half_z + local_z * self.half_x <= self.half_x * self.half_z
    }
}

fn cloud_spec(treatment: &PhysicalCloudsDetailV1, height: f32) -> Option<CloudSpec> {
    match treatment {
        PhysicalCloudsDetailV1::Current => None,
        PhysicalCloudsDetailV1::FacetedLayer {
            altitude_band,
            projected_coverage,
            diameter_min,
            diameter_max,
        } => {
            let (altitude_min, altitude_max) = altitude_band_bounds(*altitude_band, height);
            Some(CloudSpec {
                shape: ReviewCloudPrimitiveShapeV1::Faceted,
                coverage: *projected_coverage,
                diameter_min: *diameter_min,
                diameter_max: *diameter_max,
                altitude_min,
                altitude_max,
                peak_intersection_required: !matches!(*altitude_band, CloudAltitudeBandV1::Clear),
                shadow: None,
            })
        }
        PhysicalCloudsDetailV1::GrazingShape {
            shape,
            projected_coverage,
            diameter_min,
            diameter_max,
        } => Some(CloudSpec {
            shape: match shape {
                CloudShapeV1::Rounded => ReviewCloudPrimitiveShapeV1::Rounded,
                CloudShapeV1::Lenticular => ReviewCloudPrimitiveShapeV1::Lenticular,
            },
            coverage: *projected_coverage,
            diameter_min: *diameter_min,
            diameter_max: *diameter_max,
            altitude_min: height - 4.0,
            altitude_max: height + 4.0,
            peak_intersection_required: true,
            shadow: None,
        }),
        PhysicalCloudsDetailV1::RoundedCoverage {
            projected_coverage,
            diameter_min,
            diameter_max,
        } => Some(CloudSpec {
            shape: ReviewCloudPrimitiveShapeV1::Rounded,
            coverage: *projected_coverage,
            diameter_min: *diameter_min,
            diameter_max: *diameter_max,
            altitude_min: height - 4.0,
            altitude_max: height + 4.0,
            peak_intersection_required: true,
            shadow: None,
        }),
        PhysicalCloudsDetailV1::RoundedShadow {
            projected_coverage,
            diameter_min,
            diameter_max,
            max_projected_shadow,
            shadow_blur,
        } => Some(CloudSpec {
            shape: ReviewCloudPrimitiveShapeV1::Rounded,
            coverage: *projected_coverage,
            diameter_min: *diameter_min,
            diameter_max: *diameter_max,
            altitude_min: height - 4.0,
            altitude_max: height + 4.0,
            peak_intersection_required: true,
            shadow: Some((*max_projected_shadow, *shadow_blur)),
        }),
    }
}

fn altitude_band_bounds(band: CloudAltitudeBandV1, height: f32) -> (f32, f32) {
    match band {
        CloudAltitudeBandV1::Clear => (height + 4.0, height + 12.0),
        CloudAltitudeBandV1::Grazing => (height - 4.0, height + 4.0),
        CloudAltitudeBandV1::Crossing => (height - 22.0, height - 10.0),
    }
}

#[expect(
    clippy::cast_precision_loss,
    reason = "the fixed 256-square sampling grid and its in-field subset are exactly representable in f32"
)]
fn cloud_cluster_layout(
    input: &LiquidAtmosphereReviewInputV1,
    spec: CloudSpec,
) -> Result<(Vec<CloudClusterLayout>, ReviewCloudCoverageEvidenceV1), ReviewWorldDetailEffectError>
{
    let step = input.cloud_field_radius * 2.0 / CLOUD_COVERAGE_GRID as f32;
    let mut samples = Vec::new();
    for z in 0..CLOUD_COVERAGE_GRID {
        for x in 0..CLOUD_COVERAGE_GRID {
            let local_point = Vec2::new(
                -input.cloud_field_radius + (x as f32 + 0.5) * step,
                -input.cloud_field_radius + (z as f32 + 0.5) * step,
            );
            if local_point.length_squared() <= input.cloud_field_radius * input.cloud_field_radius {
                samples.push(local_point + Vec2::new(input.massif_crest.x, input.massif_crest.z));
            }
        }
    }
    if samples.is_empty() {
        return Err(ReviewWorldDetailEffectError::InvalidCloudFieldRadius);
    }
    let mut covered = vec![false; samples.len()];
    let mut covered_count = 0_usize;
    let mut clusters = Vec::new();
    let mut best_count = 0_usize;
    let mut best_fraction = 0.0_f32;
    let mut best_error = f32::INFINITY;
    for cluster_index in 0..CLOUD_MAX_CLUSTERS {
        let cluster_u32 = u32::try_from(cluster_index)
            .map_err(|_error| ReviewWorldDetailEffectError::CloudClusterOverflow)?;
        let center = cloud_cluster_center(input, cluster_u32, spec);
        let diameter = lerp(
            spec.diameter_min,
            spec.diameter_max,
            hash_unit(input.seed, CLOUD_LAYOUT_DOMAIN, cluster_u32, 3),
        );
        let puffs = cloud_cluster_puffs(input.seed, cluster_u32, center, diameter, spec);
        let projections = puffs
            .iter()
            .copied()
            .map(CloudPuffProjection::from_puff)
            .collect::<Vec<_>>();
        clusters.push(CloudClusterLayout {
            index: cluster_u32,
            center,
            diameter,
            puffs,
        });
        for (sample, is_covered) in samples.iter().zip(&mut covered) {
            if !*is_covered
                && projections
                    .iter()
                    .any(|projection| projection.contains(*sample))
            {
                *is_covered = true;
                covered_count = covered_count.saturating_add(1);
            }
        }
        let fraction = covered_count as f32 / samples.len() as f32;
        let error = (fraction - spec.coverage).abs();
        if error < best_error {
            best_error = error;
            best_count = clusters.len();
            best_fraction = fraction;
        }
        if fraction >= spec.coverage {
            break;
        }
    }
    if best_count == 0 || best_error > CLOUD_COVERAGE_TOLERANCE {
        return Err(ReviewWorldDetailEffectError::CloudClusterOverflow);
    }
    clusters.truncate(best_count);
    let peak_intersecting_puffs = clusters
        .iter()
        .flat_map(|cluster| cluster.puffs.iter().copied())
        .filter(|puff| cloud_puff_intersects_peak_column(*puff, input))
        .count();
    if spec.peak_intersection_required && peak_intersecting_puffs == 0 {
        return Err(ReviewWorldDetailEffectError::MissingCloudPeakIntersection);
    }
    Ok((
        clusters,
        ReviewCloudCoverageEvidenceV1 {
            field_radius: input.cloud_field_radius,
            target_fraction: spec.coverage,
            measured_fraction: best_fraction,
            tolerance: CLOUD_COVERAGE_TOLERANCE,
            sample_count: bounded_u64(samples.len()),
            cloud_clusters: bounded_u32(best_count),
            peak_intersection_required: spec.peak_intersection_required,
            peak_intersecting_puffs: bounded_u32(peak_intersecting_puffs),
        },
    ))
}

fn cloud_puff_intersects_peak_column(
    puff: ReviewCloudPuffV1,
    input: &LiquidAtmosphereReviewInputV1,
) -> bool {
    let peak_xz = Vec2::new(input.interaction_peak.x, input.interaction_peak.z);
    if !CloudPuffProjection::from_puff(puff).contains(peak_xz) {
        return false;
    }
    let puff_bottom = puff.center.y - puff.half_extents.y;
    let puff_top = puff.center.y + puff.half_extents.y;
    input
        .interaction_peak_solid_spans
        .iter()
        .any(|span| puff_bottom < span.top_y && puff_top > span.bottom_y)
}

fn cloud_cluster_puffs(
    seed: u64,
    cluster_index: u32,
    center: Vec3,
    diameter: f32,
    spec: CloudSpec,
) -> Vec<ReviewCloudPuffV1> {
    let puff_count = match spec.shape {
        ReviewCloudPrimitiveShapeV1::Faceted => 4,
        ReviewCloudPrimitiveShapeV1::Rounded => 7,
        ReviewCloudPrimitiveShapeV1::Lenticular => 3,
    };
    (0..puff_count)
        .map(|puff_index| {
            cloud_puff(
                seed,
                cluster_index,
                puff_index,
                center,
                diameter,
                spec.shape,
                spec.altitude_min,
                spec.altitude_max,
            )
        })
        .collect()
}

fn cloud_cluster_center(
    input: &LiquidAtmosphereReviewInputV1,
    cluster_index: u32,
    spec: CloudSpec,
) -> Vec3 {
    let (center_x, center_z) = if cluster_index == 0 {
        (input.interaction_peak.x, input.interaction_peak.z)
    } else {
        let angle =
            hash_unit(input.seed, CLOUD_LAYOUT_DOMAIN, cluster_index, 0) * std::f32::consts::TAU;
        let radial = hash_unit(input.seed, CLOUD_LAYOUT_DOMAIN, cluster_index, 1).sqrt()
            * input.cloud_field_radius;
        (
            input.massif_crest.x + radial * angle.cos(),
            input.massif_crest.z + radial * angle.sin(),
        )
    };
    // Keeping the envelope centre at the band's midpoint means every puff can
    // be clamped to the altitude contract and then contracted toward the same
    // centre without either constraint invalidating the other.
    let altitude = 0.5 * (spec.altitude_min + spec.altitude_max);
    Vec3::new(center_x, altitude, center_z)
}

fn cloud_puff(
    seed: u64,
    cluster_index: u32,
    puff_index: u8,
    cluster_center: Vec3,
    diameter: f32,
    shape: ReviewCloudPrimitiveShapeV1,
    altitude_min: f32,
    altitude_max: f32,
) -> ReviewCloudPuffV1 {
    // Puff zero is the common centre witness used by grazing/crossing peak
    // validation. Puffs one and two touch opposite sides of the envelope. Consequently
    // the complete cluster's actual outer diameter equals (rather than merely
    // remaining below) its deterministic 16–32 world-unit envelope.
    let angle = match puff_index {
        0 => 0.0,
        1 => 0.0,
        2 => std::f32::consts::PI,
        _ => {
            hash_unit(
                seed,
                CLOUD_LAYOUT_DOMAIN,
                cluster_index,
                10 + u32::from(puff_index) * 4,
            ) * std::f32::consts::TAU
        }
    };
    let radial_sample = hash_unit(
        seed,
        CLOUD_LAYOUT_DOMAIN,
        cluster_index,
        11 + u32::from(puff_index) * 4,
    );
    let scale = 0.7
        + hash_unit(
            seed,
            CLOUD_LAYOUT_DOMAIN,
            cluster_index,
            13 + u32::from(puff_index) * 4,
        ) * 0.6;
    let base = diameter * scale;
    let mut half_extents = match shape {
        ReviewCloudPrimitiveShapeV1::Faceted => Vec3::new(base * 0.18, base * 0.14, base * 0.18),
        ReviewCloudPrimitiveShapeV1::Rounded => Vec3::new(base * 0.20, base * 0.16, base * 0.20),
        ReviewCloudPrimitiveShapeV1::Lenticular => Vec3::new(base * 0.34, base * 0.07, base * 0.22),
    };
    // The altitude settings describe the complete layer, not only cluster
    // centres. Constrain each low-poly puff so the nominal clear layer cannot
    // touch a summit and the grazing/crossing treatments remain disjoint tests.
    // Four world units is the narrowest half-band in the matrix. Capping every
    // altitude variant to that common bound keeps puff dimensions identical,
    // so C01/C02/C03 vary altitude and nothing else.
    let band_half_height = (0.5 * (altitude_max - altitude_min).max(0.0)).min(4.0);
    half_extents.y = half_extents.y.min(band_half_height);
    let minimum_center_y = altitude_min + half_extents.y;
    let maximum_center_y = altitude_max - half_extents.y;
    let center_y = cluster_center.y.clamp(minimum_center_y, maximum_center_y);
    // `diameter` is the complete cluster envelope, not merely a scale passed
    // to each puff. Reserve the longest octahedron spoke, then contract the
    // centre offset so every rotated vertex remains inside the radius by the
    // triangle inequality. This bounds faceted, rounded, and lenticular forms.
    let envelope_radius = diameter * 0.5;
    let maximum_spoke = half_extents.max_element();
    let maximum_offset = (envelope_radius - maximum_spoke).max(0.0);
    let desired_radius = if puff_index == 0 {
        0.0
    } else if puff_index < 3 {
        maximum_offset
    } else {
        radial_sample * diameter * 0.24
    };
    let desired_offset = Vec3::new(
        desired_radius * angle.cos(),
        center_y - cluster_center.y,
        desired_radius * angle.sin(),
    );
    let offset = if desired_offset.length_squared() > maximum_offset * maximum_offset {
        desired_offset.normalize_or_zero() * maximum_offset
    } else {
        desired_offset
    };
    ReviewCloudPuffV1 {
        cluster_index,
        puff_index,
        shape,
        cluster_center,
        cluster_diameter: diameter,
        center: cluster_center + offset,
        half_extents,
        yaw: -angle,
    }
}

fn cloud_puff_within_cluster_envelope(puff: ReviewCloudPuffV1) -> bool {
    let rotation = Quat::from_rotation_y(puff.yaw);
    let radius = puff.cluster_diameter * 0.5;
    let tolerance = 1.0e-4;
    [
        Vec3::new(0.0, puff.half_extents.y, 0.0),
        Vec3::new(0.0, -puff.half_extents.y, 0.0),
        Vec3::new(puff.half_extents.x, 0.0, 0.0),
        Vec3::new(-puff.half_extents.x, 0.0, 0.0),
        Vec3::new(0.0, 0.0, -puff.half_extents.z),
        Vec3::new(0.0, 0.0, puff.half_extents.z),
    ]
    .into_iter()
    .all(|local| {
        (puff.center + rotation * local).distance(puff.cluster_center) <= radius + tolerance
    })
}

fn plan_fog(
    builder: &mut PlanBuilder,
    treatment: &LocalFogDetailV1,
    input: &LiquidAtmosphereReviewInputV1,
) -> Result<(), ReviewWorldDetailEffectError> {
    let LocalFogDetailV1::Layer {
        placement,
        radius_min,
        radius_max,
        height,
        coverage,
        opacity,
        bottom_offset,
    } = treatment
    else {
        return Ok(());
    };
    let mut eligible = input
        .effect_anchors
        .iter()
        .filter(|anchor| fog_anchor_eligible(*placement, anchor.kind))
        .collect::<Vec<_>>();
    eligible.sort_by(|left, right| left.name.cmp(&right.name));
    if eligible.is_empty() {
        return Err(ReviewWorldDetailEffectError::MissingFogAnchors);
    }
    // Coverage is spatial occupancy inside each named-anchor volume. It must
    // not be rounded against the tiny anchor list: one valley anchor made both
    // 12% and 24% select the same full-density volume. The shared 32x32 mask
    // supplies hundreds of deterministic, nested samples instead.
    let (_mask, sample_count, active_samples) =
        fog_density_xz_mask(*coverage).ok_or(ReviewWorldDetailEffectError::NonFiniteInput)?;
    for anchor in eligible {
        let radius = lerp(
            *radius_min,
            *radius_max,
            hash_unit_bytes(input.seed, FOG_LAYOUT_DOMAIN, anchor.name.as_bytes()),
        );
        let center = anchor.surface + Vec3::Y * (*bottom_offset + *height * 0.5);
        let density = -(1.0 - *opacity).ln() / *height;
        builder.fog_volumes.push(ReviewFogVolumeV1 {
            anchor_name: anchor.name.clone(),
            anchor_kind: anchor.kind,
            center,
            half_extents: Vec3::new(radius, *height * 0.5, radius),
            opacity: *opacity,
            density,
            edge_softness: 0.28,
            coverage: *coverage,
        });
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "The density mask has at most 1024 samples, exactly representable as f32; this fraction is renderer evidence"
    )]
    let measured_fraction = active_samples as f32 / sample_count as f32;
    builder.effect_validation.fog_coverage = Some(ReviewFogCoverageEvidenceV1 {
        target_fraction: *coverage,
        measured_fraction,
        sample_count,
        active_samples,
        fog_volumes: bounded_u32(builder.fog_volumes.len()),
    });
    builder.render_state.volumetric_fog = true;
    builder.render_state.restore_volumetric_fog = true;
    Ok(())
}

#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "validated finite coverage selects a bounded exact subset of an in-memory anchor list"
)]
fn exact_coverage_count(eligible_count: usize, coverage: f32) -> usize {
    ((eligible_count as f64 * f64::from(coverage)).ceil() as usize).min(eligible_count)
}

/// Returns a deterministic, coherent, nested XZ occupancy mask for local fog.
/// The boolean vector is row-major `z * width + x`; samples outside the unit
/// circle remain false and are excluded from the reported denominator.
#[expect(
    clippy::cast_precision_loss,
    reason = "fixed 32-square density coordinates and counts are exactly representable in f32"
)]
pub(crate) fn fog_density_xz_mask(coverage: f32) -> Option<(Vec<bool>, u32, u32)> {
    if !coverage.is_finite() || !(0.0..=1.0).contains(&coverage) {
        return None;
    }
    let mut ranked = Vec::new();
    for z in 0..FOG_DENSITY_DEPTH {
        let nz = z as f32 / (FOG_DENSITY_DEPTH - 1) as f32 * 2.0 - 1.0;
        for x in 0..FOG_DENSITY_WIDTH {
            let nx = x as f32 / (FOG_DENSITY_WIDTH - 1) as f32 * 2.0 - 1.0;
            let radius_squared = nx * nx + nz * nz;
            if radius_squared > 1.0 {
                continue;
            }
            // Two low-frequency waves form coherent islands across the full
            // footprint. A compact centre bias preserves the exact calibrated
            // opacity ray without collapsing coverage to a smaller-radius disc.
            let coherence = (nx * 5.1 + nz * 2.3).sin()
                + 0.72 * (nx * -2.7 + nz * 4.4).cos()
                + 0.38 * (nx * 8.2 - nz * 1.6).sin();
            let centre_bias = (-20.0 * radius_squared).exp() * 4.0;
            ranked.push((coherence + centre_bias, x, z));
        }
    }
    ranked.sort_by(|left, right| {
        right
            .0
            .total_cmp(&left.0)
            .then_with(|| left.2.cmp(&right.2))
            .then_with(|| left.1.cmp(&right.1))
    });
    let sample_count = u32::try_from(ranked.len()).ok()?;
    let selected_count = exact_coverage_count(ranked.len(), coverage);
    let mut mask = vec![false; usize::try_from(FOG_DENSITY_WIDTH * FOG_DENSITY_DEPTH).ok()?];
    for (_score, x, z) in ranked.into_iter().take(selected_count) {
        let index = usize::try_from(z * FOG_DENSITY_WIDTH + x).ok()?;
        *mask.get_mut(index)? = true;
    }
    Some((mask, sample_count, u32::try_from(selected_count).ok()?))
}

fn fog_anchor_eligible(placement: LocalFogPlacementV1, kind: ReviewEffectAnchorKindV1) -> bool {
    match placement {
        LocalFogPlacementV1::WaterHugging => matches!(
            kind,
            ReviewEffectAnchorKindV1::Water
                | ReviewEffectAnchorKindV1::ValleyWater
                | ReviewEffectAnchorKindV1::Waterfall
        ),
        LocalFogPlacementV1::ValleyFloor => matches!(
            kind,
            ReviewEffectAnchorKindV1::Valley | ReviewEffectAnchorKindV1::ValleyWater
        ),
        LocalFogPlacementV1::Mixed => matches!(
            kind,
            ReviewEffectAnchorKindV1::Water
                | ReviewEffectAnchorKindV1::Waterfall
                | ReviewEffectAnchorKindV1::Valley
                | ReviewEffectAnchorKindV1::ValleyWater
        ),
    }
}

fn coverage_score(seed: u64, domain: &str, key: &[u8]) -> u64 {
    let domain_seed = xxh3_64_with_seed(domain.as_bytes(), seed);
    xxh3_64_with_seed(key, domain_seed)
}

#[expect(
    clippy::cast_precision_loss,
    reason = "24 high hash bits intentionally form a deterministic unit interval"
)]
fn hash_unit_bytes(seed: u64, domain: &str, key: &[u8]) -> f32 {
    let sample = coverage_score(seed, domain, key) >> 40;
    sample as f32 / 16_777_215.0
}

#[expect(
    clippy::cast_precision_loss,
    reason = "24 high hash bits intentionally form a deterministic unit interval"
)]
fn hash_unit(seed: u64, domain: &str, major: u32, minor: u32) -> f32 {
    let mut bytes = Vec::with_capacity(domain.len().saturating_add(8));
    bytes.extend_from_slice(domain.as_bytes());
    bytes.extend_from_slice(&major.to_le_bytes());
    bytes.extend_from_slice(&minor.to_le_bytes());
    let sample = xxh3_64_with_seed(&bytes, seed) >> 40;
    sample as f32 / 16_777_215.0
}

fn lerp(minimum: f32, maximum: f32, amount: f32) -> f32 {
    minimum + (maximum - minimum) * amount
}

#[derive(Debug, Clone, Copy)]
enum StripDirection {
    TowardLand,
    TowardWater,
}

#[derive(Debug, Clone, Copy)]
enum StripAttachmentSurface {
    Water,
    Land,
}

#[derive(Debug, Clone, Copy)]
struct StripJunctionArm {
    owner: HexCoord,
    chunk: ReviewChunkKeyV1,
    near: Vec3,
    far: Vec3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct ReviewHexCornerKey {
    x_half_inner: i64,
    z_half_radius: i64,
}

fn corner_keys(coord: HexCoord) -> [ReviewHexCornerKey; 6] {
    let q = i64::from(coord.x());
    let r = i64::from(coord.y());
    let x = 2 * q + r;
    let z = 3 * r;
    [
        ReviewHexCornerKey {
            x_half_inner: x,
            z_half_radius: z - 2,
        },
        ReviewHexCornerKey {
            x_half_inner: x - 1,
            z_half_radius: z - 1,
        },
        ReviewHexCornerKey {
            x_half_inner: x - 1,
            z_half_radius: z + 1,
        },
        ReviewHexCornerKey {
            x_half_inner: x,
            z_half_radius: z + 2,
        },
        ReviewHexCornerKey {
            x_half_inner: x + 1,
            z_half_radius: z + 1,
        },
        ReviewHexCornerKey {
            x_half_inner: x + 1,
            z_half_radius: z - 1,
        },
    ]
}

fn edge_corner_keys(
    coord: HexCoord,
    side: ReviewHexSideV1,
) -> (ReviewHexCornerKey, ReviewHexCornerKey) {
    let [north, north_west, south_west, south, south_east, north_east] = corner_keys(coord);
    match side {
        ReviewHexSideV1::East => (north_east, south_east),
        ReviewHexSideV1::SouthEast => (south_east, south),
        ReviewHexSideV1::SouthWest => (south, south_west),
        ReviewHexSideV1::West => (south_west, north_west),
        ReviewHexSideV1::NorthWest => (north_west, north),
        ReviewHexSideV1::NorthEast => (north, north_east),
    }
}

#[expect(
    clippy::cast_precision_loss,
    reason = "review coordinates are bounded by the generated map radius"
)]
fn corner_world(key: ReviewHexCornerKey, y: f32) -> Vec3 {
    Vec3::new(
        HEX_INRADIUS * key.x_half_inner as f32,
        y,
        0.5 * HEX_CIRCUMRADIUS * key.z_half_radius as f32,
    )
}

fn edge_frame(coord: HexCoord, side: ReviewHexSideV1, y: f32) -> (Vec3, Vec3, Vec3) {
    let center = coord.to_world(y);
    let (first_key, second_key) = edge_corner_keys(coord, side);
    let first = corner_world(first_key, y);
    let second = corner_world(second_key, y);
    let outward = (0.5 * (first + second) - center).normalize();
    (first, second, outward)
}

fn append_hex_cap(
    mesh: &mut ReviewIndexedMeshV1,
    coord: HexCoord,
    y: f32,
    color: [f32; 4],
) -> Result<(), ReviewWorldDetailEffectError> {
    let center = coord.to_world(y);
    let corners = [
        center + Vec3::new(0.0, 0.0, -HEX_CIRCUMRADIUS),
        center + Vec3::new(-HEX_INRADIUS, 0.0, -0.5 * HEX_CIRCUMRADIUS),
        center + Vec3::new(-HEX_INRADIUS, 0.0, 0.5 * HEX_CIRCUMRADIUS),
        center + Vec3::new(0.0, 0.0, HEX_CIRCUMRADIUS),
        center + Vec3::new(HEX_INRADIUS, 0.0, 0.5 * HEX_CIRCUMRADIUS),
        center + Vec3::new(HEX_INRADIUS, 0.0, -0.5 * HEX_CIRCUMRADIUS),
    ];
    let base = push_colored_vertex(mesh, center, Vec3::Y, continuous_uv(center), color)?;
    let mut rim = Vec::with_capacity(corners.len());
    for corner in corners {
        rim.push(push_colored_vertex(
            mesh,
            corner,
            Vec3::Y,
            continuous_uv(corner),
            color,
        )?);
    }
    let Some(first) = rim.first().copied() else {
        return Err(ReviewWorldDetailEffectError::InvalidMesh);
    };
    for pair in rim.windows(2) {
        let [left, right] = pair else {
            return Err(ReviewWorldDetailEffectError::InvalidMesh);
        };
        mesh.indices.extend([base, *left, *right]);
    }
    let Some(last) = rim.last().copied() else {
        return Err(ReviewWorldDetailEffectError::InvalidMesh);
    };
    mesh.indices.extend([base, last, first]);
    Ok(())
}

fn continuous_uv(position: Vec3) -> Vec2 {
    Vec2::new(
        position.z / (2.0 * HEX_CIRCUMRADIUS),
        position.x / (2.0 * HEX_INRADIUS),
    )
}

fn append_vertical_edge_quad(
    mesh: &mut ReviewIndexedMeshV1,
    coord: HexCoord,
    side: ReviewHexSideV1,
    top_y: f32,
    bottom_y: f32,
    color: [f32; 4],
) -> Result<(), ReviewWorldDetailEffectError> {
    let (top_first, top_second, normal) = edge_frame(coord, side, top_y);
    let bottom_first = Vec3::new(top_first.x, bottom_y, top_first.z);
    let bottom_second = Vec3::new(top_second.x, bottom_y, top_second.z);
    append_colored_quad(
        mesh,
        [top_first, top_second, bottom_second, bottom_first],
        normal,
        [
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(1.0, top_y - bottom_y),
            Vec2::new(0.0, top_y - bottom_y),
        ],
        color,
    )
}

fn append_attached_edge_strip(
    mesh: &mut ReviewIndexedMeshV1,
    coord: HexCoord,
    side: ReviewHexSideV1,
    y: f32,
    width: f32,
    direction: StripDirection,
) -> Result<(), ReviewWorldDetailEffectError> {
    append_upward_quad(
        mesh,
        attached_edge_strip_positions(coord, side, y, width, direction),
    )
}

fn attached_edge_strip_positions(
    coord: HexCoord,
    side: ReviewHexSideV1,
    y: f32,
    width: f32,
    direction: StripDirection,
) -> [Vec3; 4] {
    attached_edge_band_positions(coord, side, y, 0.0, width, direction)
}

fn attached_edge_band_positions(
    coord: HexCoord,
    side: ReviewHexSideV1,
    y: f32,
    near_offset: f32,
    far_offset: f32,
    direction: StripDirection,
) -> [Vec3; 4] {
    let (first, second, _outward) = edge_frame(coord, side, y);
    let center = coord.to_world(y);
    let radial_scale = |offset: f32| match direction {
        StripDirection::TowardLand => (HEX_INRADIUS + offset) / HEX_INRADIUS,
        StripDirection::TowardWater => (HEX_INRADIUS - offset) / HEX_INRADIUS,
    };
    // Scale both edge corners about the owning water hex. Adjacent shoreline
    // sectors therefore share the same inner/outer miter vertex instead of
    // overlapping independent normal-offset rectangles at every hex corner.
    let near_first = center + (first - center) * radial_scale(near_offset);
    let far_first = center + (first - center) * radial_scale(far_offset);
    let far_second = center + (second - center) * radial_scale(far_offset);
    let near_second = center + (second - center) * radial_scale(near_offset);
    [near_first, far_first, far_second, near_second]
}

#[expect(
    clippy::too_many_arguments,
    reason = "junction ownership keeps surface, layer, material, width, and bias explicit"
)]
fn append_cross_owner_strip_junctions(
    builder: &mut PlanBuilder,
    input: &LiquidAtmosphereReviewInputV1,
    edges: &[ReviewOwnedShoreEdgeV1],
    layer: ReviewMeshLayerV1,
    material: ReviewMaterialKeyV1,
    near_offset: f32,
    far_offset: f32,
    direction: StripDirection,
    attachment: StripAttachmentSurface,
    y_bias: f32,
    alpha_range: Option<[f32; 2]>,
) -> Result<(), ReviewWorldDetailEffectError> {
    let mut by_corner =
        BTreeMap::<(ReviewHexCornerKey, u32, TilePos), Vec<StripJunctionArm>>::new();
    for edge in edges {
        let (surface, chunk) = match attachment {
            StripAttachmentSurface::Water => input
                .liquids
                .iter()
                .find(|cell| cell.position == edge.water)
                .map(|cell| (cell.position, cell.chunk)),
            StripAttachmentSurface::Land => input
                .shore_surfaces
                .iter()
                .find(|surface| surface.position == edge.land)
                .map(|surface| (surface.position, surface.chunk)),
        }
        .ok_or(ReviewWorldDetailEffectError::InvalidMesh)?;
        let y = surface_y(surface.level, input.level_height) + y_bias;
        let positions = attached_edge_band_positions(
            edge.water.coord,
            edge.side,
            y,
            near_offset,
            far_offset,
            direction,
        );
        let [near_first, far_first, far_second, near_second] = positions;
        let (first_corner, second_corner) = edge_corner_keys(edge.water.coord, edge.side);
        by_corner
            .entry((first_corner, y.to_bits(), edge.land))
            .or_default()
            .push(StripJunctionArm {
                owner: edge.water.coord,
                chunk,
                near: near_first,
                far: far_first,
            });
        by_corner
            .entry((second_corner, y.to_bits(), edge.land))
            .or_default()
            .push(StripJunctionArm {
                owner: edge.water.coord,
                chunk,
                near: near_second,
                far: far_second,
            });
    }

    for ((_corner, _y, _land), mut arms) in by_corner {
        arms.sort_by_key(|arm| (arm.owner, arm.chunk));
        arms.dedup_by_key(|arm| arm.owner);
        if arms.len() < 2 {
            continue;
        }
        let [first, second] = arms.as_slice() else {
            return Err(ReviewWorldDetailEffectError::InvalidMesh);
        };
        let key = ReviewMeshBatchKeyV1 {
            chunk: first.chunk.min(second.chunk),
            layer,
            material,
        };
        if first.near.distance_squared(second.near) <= 1.0e-12 {
            let positions = [first.near, first.far, second.far];
            if let Some([near_alpha, far_alpha]) = alpha_range {
                append_upward_triangle_with_alphas(
                    builder.mesh_mut(key),
                    positions,
                    [near_alpha, far_alpha, far_alpha],
                )?;
            } else {
                append_upward_triangle(builder.mesh_mut(key), positions)?;
            }
        } else {
            let positions = [first.near, first.far, second.far, second.near];
            if let Some([near_alpha, far_alpha]) = alpha_range {
                append_upward_quad_with_alphas(
                    builder.mesh_mut(key),
                    positions,
                    [near_alpha, far_alpha, far_alpha, near_alpha],
                )?;
            } else {
                append_upward_quad(builder.mesh_mut(key), positions)?;
            }
        }
    }
    Ok(())
}

fn append_upward_triangle(
    mesh: &mut ReviewIndexedMeshV1,
    mut positions: [Vec3; 3],
) -> Result<(), ReviewWorldDetailEffectError> {
    let [first, second, third] = positions;
    if (second - first).cross(third - first).dot(Vec3::Y) < 0.0 {
        positions.swap(1, 2);
    }
    let [first, second, third] = positions;
    let first_index = push_vertex(mesh, first, Vec3::Y, continuous_uv(first))?;
    let second_index = push_vertex(mesh, second, Vec3::Y, continuous_uv(second))?;
    let third_index = push_vertex(mesh, third, Vec3::Y, continuous_uv(third))?;
    mesh.indices
        .extend([first_index, second_index, third_index]);
    Ok(())
}

fn append_upward_triangle_with_alphas(
    mesh: &mut ReviewIndexedMeshV1,
    mut positions: [Vec3; 3],
    mut alphas: [f32; 3],
) -> Result<(), ReviewWorldDetailEffectError> {
    let [first, second, third] = positions;
    if (second - first).cross(third - first).dot(Vec3::Y) < 0.0 {
        positions.swap(1, 2);
        alphas.swap(1, 2);
    }
    let [first, second, third] = positions;
    let [first_alpha, second_alpha, third_alpha] = alphas;
    let first_index = push_colored_vertex(
        mesh,
        first,
        Vec3::Y,
        continuous_uv(first),
        [1.0, 1.0, 1.0, first_alpha],
    )?;
    let second_index = push_colored_vertex(
        mesh,
        second,
        Vec3::Y,
        continuous_uv(second),
        [1.0, 1.0, 1.0, second_alpha],
    )?;
    let third_index = push_colored_vertex(
        mesh,
        third,
        Vec3::Y,
        continuous_uv(third),
        [1.0, 1.0, 1.0, third_alpha],
    )?;
    mesh.indices
        .extend([first_index, second_index, third_index]);
    Ok(())
}

fn append_upward_quad(
    mesh: &mut ReviewIndexedMeshV1,
    mut positions: [Vec3; 4],
) -> Result<(), ReviewWorldDetailEffectError> {
    let [first, second, third, _fourth] = positions;
    if (second - first).cross(third - first).dot(Vec3::Y) < 0.0 {
        positions.reverse();
    }
    let [first, second, third, fourth] = positions;
    append_quad(
        mesh,
        [first, second, third, fourth],
        Vec3::Y,
        [
            Vec2::new(0.0, 0.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(1.0, 0.0),
        ],
    )
}

fn append_upward_quad_with_alphas(
    mesh: &mut ReviewIndexedMeshV1,
    mut positions: [Vec3; 4],
    mut alphas: [f32; 4],
) -> Result<(), ReviewWorldDetailEffectError> {
    let [first, second, third, _fourth] = positions;
    if (second - first).cross(third - first).dot(Vec3::Y) < 0.0 {
        positions.reverse();
        alphas.reverse();
    }
    let colors = alphas.map(|alpha| [1.0, 1.0, 1.0, alpha]);
    append_quad_with_vertex_colors(
        mesh,
        positions,
        Vec3::Y,
        [
            Vec2::new(0.0, 0.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(1.0, 0.0),
        ],
        colors,
    )
}

fn append_quad(
    mesh: &mut ReviewIndexedMeshV1,
    positions: [Vec3; 4],
    normal: Vec3,
    uvs: [Vec2; 4],
) -> Result<(), ReviewWorldDetailEffectError> {
    append_quad_with_color(mesh, positions, normal, uvs, None)
}

fn append_colored_quad(
    mesh: &mut ReviewIndexedMeshV1,
    positions: [Vec3; 4],
    normal: Vec3,
    uvs: [Vec2; 4],
    color: [f32; 4],
) -> Result<(), ReviewWorldDetailEffectError> {
    append_quad_with_color(mesh, positions, normal, uvs, Some(color))
}

fn append_quad_with_color(
    mesh: &mut ReviewIndexedMeshV1,
    positions: [Vec3; 4],
    normal: Vec3,
    uvs: [Vec2; 4],
    color: Option<[f32; 4]>,
) -> Result<(), ReviewWorldDetailEffectError> {
    let [first, second, third, fourth] = positions;
    let [first_uv, second_uv, third_uv, fourth_uv] = uvs;
    let base = push_vertex_with_color(mesh, first, normal, first_uv, color)?;
    let second_index = push_vertex_with_color(mesh, second, normal, second_uv, color)?;
    let third_index = push_vertex_with_color(mesh, third, normal, third_uv, color)?;
    let fourth_index = push_vertex_with_color(mesh, fourth, normal, fourth_uv, color)?;
    mesh.indices.extend([
        base,
        second_index,
        third_index,
        base,
        third_index,
        fourth_index,
    ]);
    Ok(())
}

fn append_quad_with_vertex_colors(
    mesh: &mut ReviewIndexedMeshV1,
    positions: [Vec3; 4],
    normal: Vec3,
    uvs: [Vec2; 4],
    colors: [[f32; 4]; 4],
) -> Result<(), ReviewWorldDetailEffectError> {
    let [first, second, third, fourth] = positions;
    let [first_uv, second_uv, third_uv, fourth_uv] = uvs;
    let [first_color, second_color, third_color, fourth_color] = colors;
    let base = push_colored_vertex(mesh, first, normal, first_uv, first_color)?;
    let second_index = push_colored_vertex(mesh, second, normal, second_uv, second_color)?;
    let third_index = push_colored_vertex(mesh, third, normal, third_uv, third_color)?;
    let fourth_index = push_colored_vertex(mesh, fourth, normal, fourth_uv, fourth_color)?;
    mesh.indices.extend([
        base,
        second_index,
        third_index,
        base,
        third_index,
        fourth_index,
    ]);
    Ok(())
}

#[expect(
    clippy::cast_precision_loss,
    reason = "review discs use at most 24 segments, exactly representable in f32"
)]
fn append_disc(
    mesh: &mut ReviewIndexedMeshV1,
    center: Vec3,
    radius: f32,
    segments: u32,
) -> Result<(), ReviewWorldDetailEffectError> {
    let center_index = push_vertex(mesh, center, Vec3::Y, Vec2::splat(0.5))?;
    let mut rim = Vec::with_capacity(segments as usize);
    for index in 0..segments {
        let angle = std::f32::consts::TAU * index as f32 / segments as f32;
        let position = center + Vec3::new(angle.cos() * radius, 0.0, angle.sin() * radius);
        rim.push(push_vertex(
            mesh,
            position,
            Vec3::Y,
            Vec2::new(0.5 + 0.5 * angle.cos(), 0.5 + 0.5 * angle.sin()),
        )?);
    }
    let Some(first) = rim.first().copied() else {
        return Err(ReviewWorldDetailEffectError::InvalidMesh);
    };
    for pair in rim.windows(2) {
        let [left, right] = pair else {
            return Err(ReviewWorldDetailEffectError::InvalidMesh);
        };
        mesh.indices.extend([center_index, *right, *left]);
    }
    let Some(last) = rim.last().copied() else {
        return Err(ReviewWorldDetailEffectError::InvalidMesh);
    };
    mesh.indices.extend([center_index, first, last]);
    Ok(())
}

fn append_octahedron(
    mesh: &mut ReviewIndexedMeshV1,
    puff: ReviewCloudPuffV1,
) -> Result<(), ReviewWorldDetailEffectError> {
    let rotation = Quat::from_rotation_y(puff.yaw);
    let local = [
        Vec3::new(0.0, puff.half_extents.y, 0.0),
        Vec3::new(0.0, -puff.half_extents.y, 0.0),
        Vec3::new(puff.half_extents.x, 0.0, 0.0),
        Vec3::new(-puff.half_extents.x, 0.0, 0.0),
        Vec3::new(0.0, 0.0, -puff.half_extents.z),
        Vec3::new(0.0, 0.0, puff.half_extents.z),
    ];
    let mut indices = Vec::with_capacity(local.len());
    for vertex in local {
        let rotated = rotation * vertex;
        let normal = rotated.normalize_or_zero();
        indices.push(push_vertex(
            mesh,
            puff.center + rotated,
            normal,
            Vec2::new(0.5 + 0.5 * normal.x, 0.5 + 0.5 * normal.z),
        )?);
    }
    let [top, bottom, east, west, north, south] = indices.as_slice() else {
        return Err(ReviewWorldDetailEffectError::InvalidMesh);
    };
    mesh.indices.extend([
        *top, *north, *west, *top, *west, *south, *top, *south, *east, *top, *east, *north,
        *bottom, *west, *north, *bottom, *south, *west, *bottom, *east, *south, *bottom, *north,
        *east,
    ]);
    Ok(())
}

fn push_vertex(
    mesh: &mut ReviewIndexedMeshV1,
    position: Vec3,
    normal: Vec3,
    uv: Vec2,
) -> Result<u32, ReviewWorldDetailEffectError> {
    push_vertex_with_color(mesh, position, normal, uv, None)
}

fn push_colored_vertex(
    mesh: &mut ReviewIndexedMeshV1,
    position: Vec3,
    normal: Vec3,
    uv: Vec2,
    color: [f32; 4],
) -> Result<u32, ReviewWorldDetailEffectError> {
    push_vertex_with_color(mesh, position, normal, uv, Some(color))
}

fn push_vertex_with_color(
    mesh: &mut ReviewIndexedMeshV1,
    position: Vec3,
    normal: Vec3,
    uv: Vec2,
    color: Option<[f32; 4]>,
) -> Result<u32, ReviewWorldDetailEffectError> {
    let index = u32::try_from(mesh.positions.len())
        .map_err(|_error| ReviewWorldDetailEffectError::MeshIndexOverflow)?;
    let vertex_index = mesh.positions.len();
    mesh.positions.push(position.to_array());
    mesh.normals.push(normal.to_array());
    mesh.uvs.push(uv.to_array());
    match color {
        Some(color) => {
            if mesh.colors.is_empty() && vertex_index > 0 {
                mesh.colors.resize(vertex_index, [1.0; 4]);
            }
            mesh.colors.push(color);
        }
        None if !mesh.colors.is_empty() => mesh.colors.push([1.0; 4]),
        None => {}
    }
    Ok(index)
}

fn validate_mesh(mesh: &ReviewIndexedMeshV1) -> Result<(), ReviewWorldDetailEffectError> {
    if mesh.positions.is_empty()
        || mesh.positions.len() != mesh.normals.len()
        || mesh.positions.len() != mesh.uvs.len()
        || (!mesh.colors.is_empty() && mesh.positions.len() != mesh.colors.len())
        || !mesh.indices.len().is_multiple_of(3)
    {
        return Err(ReviewWorldDetailEffectError::InvalidMesh);
    }
    let finite = mesh
        .positions
        .iter()
        .flatten()
        .chain(mesh.normals.iter().flatten())
        .chain(mesh.uvs.iter().flatten())
        .chain(mesh.colors.iter().flatten())
        .all(|value| value.is_finite());
    if !finite {
        return Err(ReviewWorldDetailEffectError::InvalidMesh);
    }
    if mesh
        .colors
        .iter()
        .flatten()
        .any(|value| !(0.0..=1.0).contains(value))
    {
        return Err(ReviewWorldDetailEffectError::InvalidMesh);
    }
    for triangle in mesh.indices.chunks_exact(3) {
        let [first_index, second_index, third_index] = triangle else {
            return Err(ReviewWorldDetailEffectError::InvalidMesh);
        };
        let first = mesh_vertex(mesh, *first_index)?;
        let second = mesh_vertex(mesh, *second_index)?;
        let third = mesh_vertex(mesh, *third_index)?;
        let cross = (second - first).cross(third - first);
        let average_normal = mesh_normal(mesh, *first_index)?
            + mesh_normal(mesh, *second_index)?
            + mesh_normal(mesh, *third_index)?;
        if cross.length_squared() <= 1.0e-10 || cross.dot(average_normal) <= 0.0 {
            return Err(ReviewWorldDetailEffectError::InvalidTriangleWinding);
        }
    }
    Ok(())
}

fn mesh_vertex(
    mesh: &ReviewIndexedMeshV1,
    index: u32,
) -> Result<Vec3, ReviewWorldDetailEffectError> {
    let index =
        usize::try_from(index).map_err(|_error| ReviewWorldDetailEffectError::InvalidMesh)?;
    mesh.positions
        .get(index)
        .copied()
        .map(Vec3::from_array)
        .ok_or(ReviewWorldDetailEffectError::InvalidMesh)
}

fn mesh_normal(
    mesh: &ReviewIndexedMeshV1,
    index: u32,
) -> Result<Vec3, ReviewWorldDetailEffectError> {
    let index =
        usize::try_from(index).map_err(|_error| ReviewWorldDetailEffectError::InvalidMesh)?;
    mesh.normals
        .get(index)
        .copied()
        .map(Vec3::from_array)
        .ok_or(ReviewWorldDetailEffectError::InvalidMesh)
}

fn canonical_plan_hash(plan: &LiquidAtmosphereReviewPlanV1) -> u64 {
    phase_bound_plan_hash(phase_neutral_plan_hash(plan), plan.phase_seconds)
}

fn phase_neutral_plan_hash(plan: &LiquidAtmosphereReviewPlanV1) -> u64 {
    let mut canonical = plan.clone();
    canonical.plan_hash = 0;
    canonical.phase_seconds = 0.0;
    xxh3_64(format!("{canonical:?}").as_bytes())
}

fn phase_bound_plan_hash(phase_neutral_hash: u64, phase_seconds: f32) -> u64 {
    let mut bytes = b"review-liquid-atmosphere-phase-v1".to_vec();
    bytes.extend_from_slice(&phase_neutral_hash.to_le_bytes());
    bytes.extend_from_slice(&phase_seconds.to_bits().to_le_bytes());
    xxh3_64(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn position(coord: HexCoord, level: Level) -> TilePos {
        TilePos { coord, level }
    }

    fn water(
        coord: HexCoord,
        level: Level,
        run_bottom: Level,
        flow: ReviewLiquidFlowV1,
        downstream: Option<TilePos>,
    ) -> ReviewLiquidCellV1 {
        ReviewLiquidCellV1 {
            position: position(coord, level),
            run_bottom,
            kind: ReviewLiquidKindV1::Water,
            flow,
            downstream,
            chunk: ReviewChunkKeyV1 {
                q: coord.x().div_euclid(16),
                r: coord.y().div_euclid(16),
            },
        }
    }

    fn shore(
        coord: HexCoord,
        level: Level,
        run_bottom: Level,
        eligible: bool,
    ) -> ReviewShoreSurfaceV1 {
        ReviewShoreSurfaceV1 {
            position: position(coord, level),
            run_bottom,
            chunk: ReviewChunkKeyV1 {
                q: coord.x().div_euclid(16),
                r: coord.y().div_euclid(16),
            },
            substance: SubstanceId(3),
            snow_covered: true,
            frozen_biome: true,
            eligible,
        }
    }

    fn replace_shores(
        input: &mut LiquidAtmosphereReviewInputV1,
        shores: Vec<ReviewShoreSurfaceV1>,
    ) {
        input.physical_solid_runs = shores
            .iter()
            .map(|shore| ReviewPhysicalSolidRunV1 {
                position: shore.position,
                run_bottom: shore.run_bottom,
            })
            .collect();
        input.shore_surfaces = shores;
    }

    fn fixture() -> LiquidAtmosphereReviewInputV1 {
        let origin = HexCoord::ORIGIN;
        let [east, _south_east, _south_west, _west, _north_west, _north_east] = origin.neighbors();
        let fall_coord = HexCoord::new_cubic(8, -4, -4);
        let [fall_downstream_coord, _, _, _, _, _] = fall_coord.neighbors();
        let [fall_pool_neighbor, _, _, _, _, _] = fall_downstream_coord.neighbors();
        let fall_landing = position(fall_downstream_coord, 136);
        let liquids = vec![
            water(origin, 140, 136, ReviewLiquidFlowV1::Still, None),
            water(east, 140, 139, ReviewLiquidFlowV1::Current, None),
            water(
                fall_coord,
                148,
                144,
                ReviewLiquidFlowV1::Fall,
                Some(fall_landing),
            ),
            water(
                fall_downstream_coord,
                136,
                135,
                ReviewLiquidFlowV1::Still,
                None,
            ),
        ];
        let liquid_coords = liquids
            .iter()
            .map(|cell| cell.position.coord)
            .collect::<BTreeSet<_>>();
        let mut shore_by_position = BTreeMap::new();
        for cell in &liquids {
            for neighbor in cell.position.coord.neighbors() {
                if liquid_coords.contains(&neighbor) {
                    continue;
                }
                let shore_position = position(neighbor, cell.position.level);
                shore_by_position
                    .entry(shore_position)
                    .or_insert(ReviewShoreSurfaceV1 {
                        position: shore_position,
                        run_bottom: shore_position.level,
                        chunk: cell.chunk,
                        substance: SubstanceId(3),
                        snow_covered: true,
                        frozen_biome: true,
                        eligible: true,
                    });
            }
        }
        let shore_surfaces = shore_by_position.into_values().collect::<Vec<_>>();
        let physical_solid_runs = shore_surfaces
            .iter()
            .map(|shore| ReviewPhysicalSolidRunV1 {
                position: shore.position,
                run_bottom: shore.run_bottom,
            })
            .collect();
        LiquidAtmosphereReviewInputV1 {
            seed: 1_592_598_566,
            level_height: 0.35,
            phase_seconds: 0.0,
            max_exposed_natural_y: 56.0,
            massif_crest: Vec3::new(12.0, 54.0, -8.0),
            interaction_peak: Vec3::new(12.0, 56.0, -8.0),
            interaction_peak_solid_spans: vec![ReviewPeakSolidSpanV1 {
                bottom_y: 0.0,
                top_y: 56.0,
            }],
            cloud_field_radius: 72.0,
            liquids,
            physical_solid_runs,
            shore_surfaces,
            effect_anchors: vec![
                ReviewEffectAnchorV1 {
                    name: "grand_v3.waterfall_base".to_owned(),
                    kind: ReviewEffectAnchorKindV1::Waterfall,
                    position: position(fall_pool_neighbor, 136),
                    surface: fall_pool_neighbor.to_world(surface_y(136, 0.35)),
                },
                ReviewEffectAnchorV1 {
                    name: "grand_v3.river_outlet".to_owned(),
                    kind: ReviewEffectAnchorKindV1::Water,
                    position: position(origin, 140),
                    surface: origin.to_world(surface_y(140, 0.35)),
                },
                ReviewEffectAnchorV1 {
                    name: "grand_v3.valley_floor.west".to_owned(),
                    kind: ReviewEffectAnchorKindV1::Valley,
                    position: position(HexCoord::new_cubic(-12, 4, 8), 33),
                    surface: HexCoord::new_cubic(-12, 4, 8).to_world(12.0),
                },
                ReviewEffectAnchorV1 {
                    name: "grand_v3.valley_floor.east".to_owned(),
                    kind: ReviewEffectAnchorKindV1::Valley,
                    position: position(HexCoord::new_cubic(10, 2, -12), 39),
                    surface: HexCoord::new_cubic(10, 2, -12).to_world(14.0),
                },
            ],
        }
    }

    fn atomic_profiles(prefix: &str) -> Vec<ReviewWorldDetailProfileV1> {
        ReviewWorldDetailProfileV1::atomic_matrix()
            .into_iter()
            .filter(|profile| {
                profile
                    .active_treatment_ids()
                    .first()
                    .is_some_and(|id| id.starts_with(prefix))
            })
            .collect()
    }

    #[test]
    fn control_builds_no_projection_or_render_state_change() {
        let plan =
            build_liquid_atmosphere_review_plan(&ReviewWorldDetailProfileV1::default(), &fixture())
                .expect("control plan");

        assert!(plan.materials.is_empty());
        assert!(plan.mesh_batches.is_empty());
        assert!(plan.shoreline_edges.is_empty());
        assert!(plan.water_curtain_edges.is_empty());
        assert!(plan.cloud_puffs.is_empty());
        assert!(plan.cloud_shadows.is_empty());
        assert!(plan.spray_volumes.is_empty());
        assert!(plan.fog_volumes.is_empty());
        assert_eq!(
            plan.effect_validation,
            ReviewWorldDetailEffectValidationV1::default()
        );
        assert_eq!(
            plan.render_state,
            ReviewRenderStateRequirementsV1::default()
        );
        assert!(plan.is_current());
        assert_ne!(plan.plan_hash, 0);
        assert!(plan.hash_is_valid());
    }

    #[test]
    fn all_seven_water_settings_build_caps_and_restorable_state() {
        let input = fixture();
        let profiles = atomic_profiles("water-");
        assert_eq!(profiles.len(), 7);
        for profile in profiles {
            let id = profile.active_treatment_ids().remove(0);
            let plan = build_liquid_atmosphere_review_plan(&profile, &input)
                .expect("water plan must build");
            assert_eq!(plan.counts.water_caps, 4, "{id}");
            assert!(plan.counts.water_curtains > 0, "{id}");
            assert_eq!(
                plan.water_curtain_edges
                    .iter()
                    .copied()
                    .collect::<BTreeSet<_>>()
                    .len(),
                plan.water_curtain_edges.len(),
                "{id}"
            );
            assert!(plan.render_state.suppress_opaque_water_geometry, "{id}");
            assert!(plan.render_state.restore_opaque_water_geometry, "{id}");
            assert_eq!(
                plan.render_state.order_independent_transparency,
                id != "water-06-transmission",
                "{id}"
            );
            assert_eq!(
                plan.render_state.restore_order_independent_transparency,
                id != "water-06-transmission",
                "{id}"
            );
            assert_eq!(
                plan.render_state.medium_screen_space_transmission,
                id == "water-06-transmission",
                "{id}"
            );
            let expected_alpha_mode = if id == "water-06-transmission" {
                ReviewAlphaModeV1::Opaque
            } else {
                ReviewAlphaModeV1::OrderIndependentTransparency
            };
            assert!(
                plan.materials
                    .iter()
                    .all(|material| material.alpha_mode == expected_alpha_mode),
                "{id}"
            );
        }
    }

    #[test]
    fn depth_absorption_uses_continuous_vertex_response_and_shared_materials() {
        let profile = atomic_profiles("water-")
            .into_iter()
            .find(|profile| {
                profile
                    .active_treatment_ids()
                    .contains(&"water-04-depth-short")
            })
            .expect("W04 profile");
        let plan = build_liquid_atmosphere_review_plan(&profile, &fixture()).expect("W04 plan");

        assert_eq!(plan.materials.len(), 2);
        assert!(plan.materials.iter().any(|material| matches!(
            material.key,
            ReviewMaterialKeyV1::Water {
                style: ReviewWaterMaterialStyleV1::Fall,
                ..
            }
        )));
        assert!(plan.materials.iter().all(|material| {
            material.value_multiplier.to_bits() == 1.0_f32.to_bits()
                && material.depth_half_distance == Some(0.70)
                && material.deep_value_multiplier == Some(0.62)
        }));
        assert!(plan.mesh_batches.iter().all(|batch| {
            batch.mesh.colors.len() == batch.mesh.positions.len()
                && batch.mesh.colors.iter().all(|color| {
                    let [red, green, blue, alpha] = *color;
                    red.to_bits() == green.to_bits()
                        && green.to_bits() == blue.to_bits()
                        && alpha.to_bits() == 1.0_f32.to_bits()
                })
        }));
        let exact_half_value = 0.62 + (1.0 - 0.62) * 0.5;
        assert!(plan.mesh_batches.iter().any(|batch| {
            batch.mesh.colors.iter().any(|color| {
                let [value, ..] = *color;
                (value - exact_half_value).abs() < 1.0e-6
            })
        }));
        assert_eq!(
            depth_value_multiplier(0.70, 0.70, 0.62).to_bits(),
            exact_half_value.to_bits()
        );
        assert!(plan.hash_is_valid());
        let mut changed_color = plan.clone();
        let Some(first_batch) = changed_color.mesh_batches.first_mut() else {
            panic!("W04 must emit a water mesh batch");
        };
        let Some(first_color) = first_batch.mesh.colors.first_mut() else {
            panic!("W04 water mesh batch must carry vertex colors");
        };
        let [red, ..] = first_color;
        *red *= 0.99;
        assert!(!changed_color.hash_is_valid());
    }

    #[test]
    fn shoreline_edges_are_owned_once_across_adjacent_water_cells() {
        let profile = atomic_profiles("shore-")
            .into_iter()
            .find(|profile| {
                profile
                    .active_treatment_ids()
                    .contains(&"shore-03-foam-narrow")
            })
            .expect("Q03 profile");
        let plan = build_liquid_atmosphere_review_plan(&profile, &fixture()).expect("foam plan");
        let unique = plan
            .shoreline_edges
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();

        assert_eq!(unique.len(), plan.shoreline_edges.len());
        assert_eq!(plan.counts.shoreline_edges, bounded_u32(unique.len()));
        assert!(plan
            .shoreline_edges
            .iter()
            .all(|edge| { edge.side.neighbor(edge.water.coord) == edge.land.coord }));
    }

    #[test]
    fn stacked_liquid_lookup_uses_only_the_source_related_run() {
        let source_coord = HexCoord::ORIGIN;
        let [neighbor_coord, _, _, _, _, _] = source_coord.neighbors();
        let source = water(source_coord, 20, 15, ReviewLiquidFlowV1::Still, None);
        let related = water(neighbor_coord, 18, 15, ReviewLiquidFlowV1::Still, None);
        let unrelated_upper = water(neighbor_coord, 40, 38, ReviewLiquidFlowV1::Still, None);
        let mut input = fixture();
        input.liquids = vec![source, unrelated_upper, related];

        assert_eq!(
            liquid_run_at_coord(&input, neighbor_coord, &source).map(|cell| cell.position),
            Some(related.position)
        );
        let (bottom, adjacent) = curtain_bottom(&source, ReviewHexSideV1::East, &input)
            .expect("lower related run leaves a curtain");
        assert_eq!(adjacent, Some(related.position));
        assert_eq!(
            bottom.to_bits(),
            surface_y(related.position.level, input.level_height).to_bits()
        );
    }

    #[test]
    fn shore_matching_ignores_noncontact_stacks_and_uses_the_highest_overlapping_run() {
        let source_coord = HexCoord::ORIGIN;
        let [neighbor_coord, _, _, _, _, _] = source_coord.neighbors();
        let source = water(source_coord, 20, 15, ReviewLiquidFlowV1::Still, None);
        let lower_contact = shore(neighbor_coord, 17, 12, true);
        let floating_overhang = shore(neighbor_coord, 26, 24, true);
        let mut input = fixture();
        input.liquids = vec![source];
        replace_shores(&mut input, vec![floating_overhang, lower_contact]);

        let expected = (
            surface_y(lower_contact.position.level, input.level_height),
            Some(lower_contact.position),
        );
        assert_eq!(
            curtain_bottom(&source, ReviewHexSideV1::East, &input),
            Some(expected)
        );
        assert!(shoreline_edges(&input).is_empty());

        input.shore_surfaces.reverse();
        assert_eq!(
            curtain_bottom(&source, ReviewHexSideV1::East, &input),
            Some(expected)
        );
        assert!(shoreline_edges(&input).is_empty());
    }

    #[test]
    fn containing_cliff_closes_water_but_cannot_receive_a_detached_top_rim() {
        let source_coord = HexCoord::ORIGIN;
        let [neighbor_coord, _, _, _, _, _] = source_coord.neighbors();
        let source = water(source_coord, 20, 15, ReviewLiquidFlowV1::Still, None);
        let cliff = shore(neighbor_coord, 24, 10, true);
        let mut input = fixture();
        input.liquids = vec![source];
        replace_shores(&mut input, vec![cliff]);

        assert_eq!(curtain_bottom(&source, ReviewHexSideV1::East, &input), None);
        let edges = shoreline_edges(&input);
        assert_eq!(edges.len(), 1);
        assert_eq!(
            edges
                .first()
                .expect("the containing cliff owns one shoreline edge")
                .land,
            cliff.position,
        );

        let wet = atomic_profiles("shore-")
            .into_iter()
            .find(|profile| {
                profile
                    .active_treatment_ids()
                    .contains(&"shore-01-wet-rim-narrow")
            })
            .expect("wet-rim profile");
        assert_eq!(
            build_liquid_atmosphere_review_plan(&wet, &input),
            Err(ReviewWorldDetailEffectError::MissingShoreline)
        );

        let foam = atomic_profiles("shore-")
            .into_iter()
            .find(|profile| {
                profile
                    .active_treatment_ids()
                    .contains(&"shore-03-foam-narrow")
            })
            .expect("foam profile");
        let foam_plan = build_liquid_atmosphere_review_plan(&foam, &input)
            .expect("waterline foam remains valid against a containing cliff");
        let expected_y =
            surface_y(source.position.level, input.level_height) + ATTACHED_SURFACE_BIAS;
        assert!(foam_plan
            .mesh_batches
            .iter()
            .flat_map(|batch| batch.mesh.positions.iter())
            .all(|position| position[1].to_bits() == expected_y.to_bits()));
    }

    #[test]
    fn immediate_raised_bank_accepts_top_rim_and_snow_adjacent_ice() {
        let source_coord = HexCoord::ORIGIN;
        let [neighbor_coord, _, _, _, _, _] = source_coord.neighbors();
        let source = water(source_coord, 20, 15, ReviewLiquidFlowV1::Still, None);
        let bank = shore(neighbor_coord, 21, 10, true);
        let mut input = fixture();
        input.liquids = vec![source];
        replace_shores(&mut input, vec![bank]);

        let edges = shoreline_edges(&input);
        assert_eq!(edges.len(), 1);
        assert!(immediate_water_bank(
            edges
                .first()
                .expect("the raised bank owns one shoreline edge")
        ));

        let wet = atomic_profiles("shore-")
            .into_iter()
            .find(|profile| {
                profile
                    .active_treatment_ids()
                    .contains(&"shore-01-wet-rim-narrow")
            })
            .expect("wet-rim profile");
        let wet_plan = build_liquid_atmosphere_review_plan(&wet, &input)
            .expect("one-voxel raised bank remains attached to its top rim");
        assert_eq!(wet_plan.counts.shoreline_edges, 1);
        let expected_rim_y =
            surface_y(bank.position.level, input.level_height) + ATTACHED_SURFACE_BIAS;
        assert!(wet_plan
            .mesh_batches
            .iter()
            .flat_map(|batch| batch.mesh.positions.iter())
            .all(|position| position[1].to_bits() == expected_rim_y.to_bits()));

        let snow_adjacent = atomic_profiles("ice-")
            .into_iter()
            .find(|profile| {
                profile
                    .active_treatment_ids()
                    .contains(&"ice-04-snow-adjacent")
            })
            .expect("snow-adjacent profile");
        let ice_plan = build_liquid_atmosphere_review_plan(&snow_adjacent, &input)
            .expect("one-voxel raised snowy bank remains adjacent to water-side ice");
        assert_eq!(ice_plan.counts.ice_wedges, 1);
    }

    #[test]
    fn ineligible_waterline_contact_still_occludes_water_without_owning_shore_detail() {
        let source_coord = HexCoord::ORIGIN;
        let [neighbor_coord, _, _, _, _, _] = source_coord.neighbors();
        let source = water(source_coord, 20, 15, ReviewLiquidFlowV1::Still, None);
        let mut input = fixture();
        input.liquids = vec![source];
        replace_shores(&mut input, vec![shore(neighbor_coord, 20, 10, false)]);

        assert_eq!(curtain_bottom(&source, ReviewHexSideV1::East, &input), None);
        assert!(shoreline_edges(&input).is_empty());
    }

    #[test]
    fn physical_solid_without_a_decorative_shore_surface_closes_water() {
        let source_coord = HexCoord::ORIGIN;
        let [neighbor_coord, _, _, _, _, _] = source_coord.neighbors();
        let source = water(source_coord, 20, 15, ReviewLiquidFlowV1::Still, None);
        let mut input = fixture();
        input.liquids = vec![source];
        input.shore_surfaces.clear();
        input.physical_solid_runs = vec![ReviewPhysicalSolidRunV1 {
            position: position(neighbor_coord, 24),
            run_bottom: 10,
        }];

        assert_eq!(curtain_bottom(&source, ReviewHexSideV1::East, &input), None);
        assert!(shoreline_edges(&input).is_empty());
    }

    #[test]
    fn snow_adjacent_ice_does_not_inherit_a_remote_cliff_top_mask() {
        let source_coord = HexCoord::ORIGIN;
        let [neighbor_coord, _, _, _, _, _] = source_coord.neighbors();
        let source = water(source_coord, 145, 140, ReviewLiquidFlowV1::Still, None);
        let cliff = shore(neighbor_coord, 150, 130, true);
        let edge = ReviewOwnedShoreEdgeV1 {
            water: source.position,
            side: ReviewHexSideV1::East,
            land: cliff.position,
        };
        let mut input = fixture();
        input.liquids = vec![source];
        replace_shores(&mut input, vec![cliff]);
        let snow_adjacent = atomic_profiles("ice-")
            .into_iter()
            .find(|profile| {
                profile
                    .active_treatment_ids()
                    .contains(&"ice-04-snow-adjacent")
            })
            .expect("snow-adjacent profile");
        let level_fringe = atomic_profiles("ice-")
            .into_iter()
            .find(|profile| {
                profile
                    .active_treatment_ids()
                    .contains(&"ice-01-level-narrow")
            })
            .expect("level-fringe profile");

        assert!(!ice_edge_eligible(&snow_adjacent.ice_fringe, &edge, &input));
        assert!(ice_edge_eligible(&level_fringe.ice_fringe, &edge, &input));
    }

    #[test]
    fn invalid_shore_run_fails_before_projection() {
        let mut input = fixture();
        let Some(surface) = input.shore_surfaces.first_mut() else {
            panic!("fixture must retain a shore surface");
        };
        surface.run_bottom = surface.position.level.saturating_add(1);
        let invalid_position = surface.position;

        assert_eq!(
            build_liquid_atmosphere_review_plan(&ReviewWorldDetailProfileV1::default(), &input),
            Err(ReviewWorldDetailEffectError::InvalidShoreRun(
                invalid_position
            ))
        );
    }

    #[test]
    fn attached_edge_strips_share_exact_miter_vertices_and_wind_upward() {
        let coord = HexCoord::ORIGIN;
        for direction in [StripDirection::TowardLand, StripDirection::TowardWater] {
            let mut mesh = ReviewIndexedMeshV1::default();
            let sides = ReviewHexSideV1::ALL;
            for (side, next) in sides.into_iter().zip(sides.into_iter().cycle().skip(1)) {
                let current = attached_edge_strip_positions(coord, side, 12.0, 0.25, direction);
                let adjacent = attached_edge_strip_positions(coord, next, 12.0, 0.25, direction);
                let [_, _, current_far_second, current_near_second] = current;
                let [adjacent_near_first, adjacent_far_first, _, _] = adjacent;
                assert_eq!(current_near_second, adjacent_near_first);
                assert_eq!(current_far_second, adjacent_far_first);
                append_upward_quad(&mut mesh, current).expect("valid miter sector");
            }
            validate_mesh(&mesh).expect("mitered strip ring must be finite and upward-wound");
        }
    }

    #[test]
    fn strips_from_distinct_water_owners_receive_a_wound_corner_junction() {
        let land = HexCoord::ORIGIN;
        let [east, south_east, ..] = land.neighbors();
        let mut input = fixture();
        input.liquids = vec![
            water(east, 140, 139, ReviewLiquidFlowV1::Still, None),
            water(south_east, 140, 139, ReviewLiquidFlowV1::Still, None),
        ];
        replace_shores(
            &mut input,
            vec![ReviewShoreSurfaceV1 {
                position: position(land, 140),
                run_bottom: 140,
                chunk: ReviewChunkKeyV1 { q: 0, r: 0 },
                substance: SubstanceId(3),
                snow_covered: true,
                frozen_biome: true,
                eligible: true,
            }],
        );
        let edges = [
            ReviewOwnedShoreEdgeV1 {
                water: position(east, 140),
                side: ReviewHexSideV1::West,
                land: position(land, 140),
            },
            ReviewOwnedShoreEdgeV1 {
                water: position(south_east, 140),
                side: ReviewHexSideV1::NorthWest,
                land: position(land, 140),
            },
        ];

        let mut solid = PlanBuilder::default();
        append_cross_owner_strip_junctions(
            &mut solid,
            &input,
            &edges,
            ReviewMeshLayerV1::IceFringes,
            ReviewMaterialKeyV1::Ice,
            0.0,
            0.25,
            StripDirection::TowardWater,
            StripAttachmentSurface::Water,
            0.006,
            None,
        )
        .expect("cross-owner ice corner should join");
        let solid_mesh = solid.meshes.values().next().expect("one solid junction");
        assert_eq!(solid_mesh.positions.len(), 3);
        assert_eq!(solid_mesh.indices.len(), 3);
        validate_mesh(solid_mesh).expect("solid junction must wind upward");

        let mut feather = PlanBuilder::default();
        append_cross_owner_strip_junctions(
            &mut feather,
            &input,
            &edges,
            ReviewMeshLayerV1::IceFringes,
            ReviewMaterialKeyV1::Ice,
            0.25,
            0.35,
            StripDirection::TowardWater,
            StripAttachmentSurface::Water,
            0.006,
            Some([1.0, 0.0]),
        )
        .expect("cross-owner feather corner should join");
        let feather_mesh = feather
            .meshes
            .values()
            .next()
            .expect("one feather junction");
        assert_eq!(feather_mesh.positions.len(), 4);
        assert_eq!(feather_mesh.indices.len(), 6);
        assert_eq!(feather_mesh.colors.len(), feather_mesh.positions.len());
        assert_eq!(
            feather_mesh
                .colors
                .iter()
                .map(|color| {
                    let [_, _, _, alpha] = *color;
                    alpha.to_bits()
                })
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([0.0_f32.to_bits(), 1.0_f32.to_bits()])
        );
        validate_mesh(feather_mesh).expect("feather junction must wind upward");
    }

    #[test]
    fn all_six_shore_settings_encode_wet_foam_and_spray_variants() {
        let profiles = atomic_profiles("shore-");
        assert_eq!(profiles.len(), 6);
        for profile in profiles {
            let id = profile.active_treatment_ids().remove(0);
            let plan = build_liquid_atmosphere_review_plan(&profile, &fixture())
                .expect("shore plan must build");
            if id == "shore-05-plunge-spray" || id == "shore-06-restrained-combination" {
                assert!(!plan.spray_volumes.is_empty(), "{id}");
                assert_eq!(plan.counts.spray_volumes, 1, "{id}");
                let binding = plan
                    .effect_validation
                    .waterfall_anchors
                    .first()
                    .expect("spray retains its authored anchor binding");
                assert_eq!(binding.anchor_name, "grand_v3.waterfall_base");
                assert_eq!(binding.distance_hexes, 1);
                let spray = plan
                    .spray_volumes
                    .first()
                    .expect("spray treatment must retain its planned volume");
                assert_eq!(binding.landing_position, tile_position_array(spray.landing));
            }
            if id.starts_with("shore-01")
                || id.starts_with("shore-02")
                || id == "shore-06-restrained-combination"
            {
                let wet_materials = plan
                    .materials
                    .iter()
                    .filter(|material| matches!(material.key, ReviewMaterialKeyV1::WetRim { .. }))
                    .collect::<Vec<_>>();
                assert_eq!(
                    wet_materials.len(),
                    1,
                    "{id} should share one wet-rim material for its one substrate"
                );
                let wet = wet_materials
                    .first()
                    .copied()
                    .expect("one wet-rim material was asserted above");
                let wet_batches = plan
                    .mesh_batches
                    .iter()
                    .filter(|batch| batch.key.layer == ReviewMeshLayerV1::WetRims)
                    .collect::<Vec<_>>();
                assert!(!wet_batches.is_empty(), "{id}");
                assert!(wet_batches
                    .iter()
                    .all(|batch| batch.key.material == wet.key));
                assert!(wet.roughness_delta.is_some(), "{id}");
                assert!(wet.roughness.is_none(), "{id}");
                let expected_value = if id.starts_with("shore-02") {
                    0.82
                } else {
                    0.88
                };
                assert_eq!(wet.alpha, Some(1.0), "{id}");
                assert!(
                    (wet.value_multiplier - expected_value).abs() <= f32::EPSILON,
                    "{id}"
                );
                assert_eq!(wet.alpha_mode, ReviewAlphaModeV1::Opaque, "{id}");
                if id.starts_with("shore-01") || id.starts_with("shore-02") {
                    assert!(!plan.render_state.order_independent_transparency, "{id}");
                }
            }
        }
    }

    #[test]
    fn plunge_spray_fails_closed_without_a_unique_waterfall_base_binding() {
        let profile = atomic_profiles("shore-")
            .into_iter()
            .find(|profile| {
                profile
                    .active_treatment_ids()
                    .contains(&"shore-05-plunge-spray")
            })
            .expect("Q05 profile");
        let mut missing = fixture();
        missing
            .effect_anchors
            .retain(|anchor| anchor.name != "grand_v3.waterfall_base");
        assert_eq!(
            build_liquid_atmosphere_review_plan(&profile, &missing),
            Err(ReviewWorldDetailEffectError::UnresolvedWaterfallAnchor(
                "grand_v3.waterfall_base".to_owned()
            ))
        );

        let mut ambiguous = fixture();
        let anchor = ambiguous
            .effect_anchors
            .iter()
            .find(|anchor| anchor.name == "grand_v3.waterfall_base")
            .expect("fixture base anchor")
            .position;
        let [landing_coord, source_coord, _, _, _, _] = anchor.coord.neighbors();
        let second_landing = position(landing_coord, 136);
        let second_source = position(source_coord, 148);
        ambiguous.liquids.push(water(
            second_source.coord,
            second_source.level,
            144,
            ReviewLiquidFlowV1::Fall,
            Some(second_landing),
        ));
        ambiguous.liquids.push(water(
            second_landing.coord,
            second_landing.level,
            135,
            ReviewLiquidFlowV1::Still,
            None,
        ));
        assert_eq!(
            build_liquid_atmosphere_review_plan(&profile, &ambiguous),
            Err(ReviewWorldDetailEffectError::AmbiguousWaterfallAnchor(
                "grand_v3.waterfall_base".to_owned()
            ))
        );
    }

    #[test]
    fn all_six_ice_settings_build_selected_nonbridging_wedges() {
        let profiles = atomic_profiles("ice-");
        assert_eq!(profiles.len(), 6);
        for profile in profiles {
            let id = profile.active_treatment_ids().remove(0);
            let plan = build_liquid_atmosphere_review_plan(&profile, &fixture())
                .expect("ice plan must build");
            assert!(plan.counts.ice_wedges > 0, "{id}");
            assert!(
                plan.counts.ice_wedges <= plan.counts.shoreline_edges.max(1),
                "{id}"
            );
            let coverage = plan
                .effect_validation
                .ice_coverage
                .expect("active ice treatment retains exact coverage evidence");
            assert_eq!(coverage.selected_edges, plan.counts.ice_wedges, "{id}");
            let eligible_edges = usize::try_from(coverage.eligible_edges)
                .expect("the supported targets represent every u32 as usize");
            assert_eq!(
                coverage.selected_edges,
                bounded_u32(exact_coverage_count(
                    eligible_edges,
                    coverage.target_fraction,
                )),
                "{id}"
            );
            let ice = plan
                .materials
                .iter()
                .find(|material| material.key == ReviewMaterialKeyV1::Ice)
                .expect("ice material");
            assert_eq!(ice.alpha, Some(0.82));
            assert_eq!(ice.roughness, Some(0.32));
            assert_eq!(ice.reflectance, Some(0.30));
            assert_eq!(plan.materials.len(), 1, "{id}");
            assert_eq!(plan.counts.materials, 1, "{id}");
            assert!(plan
                .mesh_batches
                .iter()
                .all(|batch| batch.key.material == ReviewMaterialKeyV1::Ice));
            if id == "ice-06-frozen-or-snow-feathered" {
                assert_eq!(ice.inward_feather, Some(0.10));
                assert!(plan.mesh_batches.iter().all(|batch| {
                    batch.mesh.colors.len() == batch.mesh.positions.len()
                        && batch.mesh.colors.iter().all(|color| {
                            let [red, green, blue, alpha] = *color;
                            red.to_bits() == 1.0_f32.to_bits()
                                && green.to_bits() == 1.0_f32.to_bits()
                                && blue.to_bits() == 1.0_f32.to_bits()
                                && (0.0..=1.0).contains(&alpha)
                        })
                }));
                let alphas = plan
                    .mesh_batches
                    .iter()
                    .flat_map(|batch| {
                        batch.mesh.colors.iter().map(|color| {
                            let [_, _, _, alpha] = *color;
                            alpha.to_bits()
                        })
                    })
                    .collect::<BTreeSet<_>>();
                assert!(alphas.contains(&0.0_f32.to_bits()));
                assert!(alphas.contains(&1.0_f32.to_bits()));
            } else {
                assert_eq!(ice.inward_feather, None, "{id}");
                assert!(
                    plan.mesh_batches
                        .iter()
                        .all(|batch| batch.mesh.colors.is_empty()),
                    "{id}"
                );
            }
        }
    }

    #[test]
    fn i06_feather_is_one_continuous_exact_width_color_ramp_and_is_hashed() {
        let profile = atomic_profiles("ice-")
            .into_iter()
            .find(|profile| {
                profile
                    .active_treatment_ids()
                    .contains(&"ice-06-frozen-or-snow-feathered")
            })
            .expect("I06 profile");
        let plan = build_liquid_atmosphere_review_plan(&profile, &fixture()).expect("I06 plan");

        assert_eq!(plan.materials.len(), 1);
        assert_eq!(plan.counts.materials, 1);
        assert!(plan.hash_is_valid());
        let mut changed_alpha = plan.clone();
        let changed = changed_alpha
            .mesh_batches
            .iter_mut()
            .flat_map(|batch| batch.mesh.colors.iter_mut())
            .find(|color| {
                let [_, _, _, alpha] = **color;
                alpha.to_bits() == 0.0_f32.to_bits()
            })
            .expect("I06 must carry transparent feather-edge vertices");
        let [_, _, _, alpha] = changed;
        *alpha = 0.01;
        assert!(!changed_alpha.hash_is_valid());

        let side = ReviewHexSideV1::East;
        let positions = attached_edge_band_positions(
            HexCoord::ORIGIN,
            side,
            12.0,
            0.25,
            0.35,
            StripDirection::TowardWater,
        );
        let [near_first, far_first, far_second, near_second] = positions;
        let (_, _, outward) = edge_frame(HexCoord::ORIGIN, side, 12.0);
        assert!(((near_first - far_first).dot(outward) - 0.10).abs() < 1.0e-6);
        assert!(((near_second - far_second).dot(outward) - 0.10).abs() < 1.0e-6);
        let mut mesh = ReviewIndexedMeshV1::default();
        append_upward_quad_with_alphas(&mut mesh, positions, [1.0, 0.0, 0.0, 1.0])
            .expect("continuous feather quad");
        assert_eq!(mesh.colors.len(), 4);
        assert_eq!(
            mesh.colors
                .iter()
                .map(|color| {
                    let [_, _, _, alpha] = *color;
                    alpha.to_bits()
                })
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([0.0_f32.to_bits(), 1.0_f32.to_bits()])
        );
        validate_mesh(&mesh).expect("continuous feather must be valid");
    }

    #[test]
    #[expect(
        clippy::cast_precision_loss,
        reason = "the fixed 256-square test grid and its counters are exactly representable in f32"
    )]
    fn cloud_coverage_samples_rendered_xz_silhouettes_not_cluster_envelopes() {
        let synthetic = ReviewCloudPuffV1 {
            cluster_index: 0,
            puff_index: 0,
            shape: ReviewCloudPrimitiveShapeV1::Faceted,
            cluster_center: Vec3::ZERO,
            cluster_diameter: 10.0,
            center: Vec3::ZERO,
            half_extents: Vec3::new(2.0, 1.0, 2.0),
            yaw: 0.0,
        };
        let projection = CloudPuffProjection::from_puff(synthetic);
        assert!(projection.contains(Vec2::new(0.5, 0.5)));
        let envelope_only_point = Vec2::new(3.0, 3.0);
        assert!(envelope_only_point.length_squared() < (synthetic.cluster_diameter * 0.5).powi(2));
        assert!(!projection.contains(envelope_only_point));

        let profile = atomic_profiles("clouds-")
            .into_iter()
            .find(|profile| {
                profile
                    .active_treatment_ids()
                    .contains(&"clouds-01-faceted-clear")
            })
            .expect("C01 profile");
        let input = fixture();
        let plan = build_liquid_atmosphere_review_plan(&profile, &input)
            .expect("faceted cloud plan must build");
        let evidence = plan
            .effect_validation
            .cloud_coverage
            .expect("faceted clouds retain coverage evidence");
        let projections = plan
            .cloud_puffs
            .iter()
            .copied()
            .map(CloudPuffProjection::from_puff)
            .collect::<Vec<_>>();
        let envelopes = plan
            .cloud_puffs
            .iter()
            .filter(|puff| puff.puff_index == 0)
            .map(|puff| {
                (
                    Vec2::new(puff.cluster_center.x, puff.cluster_center.z),
                    (puff.cluster_diameter * 0.5).powi(2),
                )
            })
            .collect::<Vec<_>>();
        let step = input.cloud_field_radius * 2.0 / CLOUD_COVERAGE_GRID as f32;
        let mut sample_count = 0_usize;
        let mut silhouette_count = 0_usize;
        let mut envelope_count = 0_usize;
        for z in 0..CLOUD_COVERAGE_GRID {
            for x in 0..CLOUD_COVERAGE_GRID {
                let local_point = Vec2::new(
                    -input.cloud_field_radius + (x as f32 + 0.5) * step,
                    -input.cloud_field_radius + (z as f32 + 0.5) * step,
                );
                if local_point.length_squared()
                    > input.cloud_field_radius * input.cloud_field_radius
                {
                    continue;
                }
                sample_count = sample_count.saturating_add(1);
                let point = local_point + Vec2::new(input.massif_crest.x, input.massif_crest.z);
                if projections
                    .iter()
                    .any(|projection| projection.contains(point))
                {
                    silhouette_count = silhouette_count.saturating_add(1);
                }
                if envelopes.iter().any(|(center, radius_squared)| {
                    point.distance_squared(*center) <= *radius_squared
                }) {
                    envelope_count = envelope_count.saturating_add(1);
                }
            }
        }
        let silhouette_fraction = silhouette_count as f32 / sample_count as f32;
        assert_eq!(bounded_u64(sample_count), evidence.sample_count);
        assert_eq!(
            silhouette_fraction.to_bits(),
            evidence.measured_fraction.to_bits(),
            "coverage evidence must measure the emitted low-poly puff union"
        );
        assert!(
            envelope_count > silhouette_count,
            "sparse faceted silhouettes must not be counted as complete envelope disks"
        );
    }

    #[test]
    fn all_eight_cloud_settings_are_world_space_bounded_and_shadow_only_c08() {
        let profiles = atomic_profiles("clouds-");
        assert_eq!(profiles.len(), 8);
        for profile in profiles {
            let id = profile.active_treatment_ids().remove(0);
            let input = fixture();
            let spec = cloud_spec(&profile.physical_clouds, input.max_exposed_natural_y)
                .expect("active cloud profile has a layer specification");
            let plan = build_liquid_atmosphere_review_plan(&profile, &input)
                .expect("cloud plan must build");
            assert!(plan.counts.cloud_clusters > 0, "{id}");
            let coverage = plan
                .effect_validation
                .cloud_coverage
                .expect("active cloud treatment retains measured coverage evidence");
            assert_eq!(coverage.cloud_clusters, plan.counts.cloud_clusters, "{id}");
            assert_eq!(
                coverage.peak_intersection_required,
                id != "clouds-01-faceted-clear",
                "{id}"
            );
            assert_eq!(
                coverage.peak_intersecting_puffs > 0,
                coverage.peak_intersection_required,
                "{id}"
            );
            assert!(
                (coverage.measured_fraction - coverage.target_fraction).abs() <= coverage.tolerance,
                "{id}"
            );
            assert!(
                plan.counts.cloud_puffs >= plan.counts.cloud_clusters * 3,
                "{id}"
            );
            assert!(plan.cloud_puffs.iter().all(|puff| {
                vec3_is_finite(puff.center)
                    && vec3_is_finite(puff.half_extents)
                    && (16.0..=32.0).contains(&puff.cluster_diameter)
                    && cloud_puff_within_cluster_envelope(*puff)
                    && puff.center.y < 80.0
                    && puff.center.y > 20.0
                    && puff.center.y - puff.half_extents.y >= spec.altitude_min - 1.0e-5
                    && puff.center.y + puff.half_extents.y <= spec.altitude_max + 1.0e-5
            }));
            for cluster_index in 0..plan.counts.cloud_clusters {
                let first = plan
                    .cloud_puffs
                    .iter()
                    .find(|puff| puff.cluster_index == cluster_index && puff.puff_index == 1)
                    .expect("cluster keeps its first envelope witness");
                let second = plan
                    .cloud_puffs
                    .iter()
                    .find(|puff| puff.cluster_index == cluster_index && puff.puff_index == 2)
                    .expect("cluster keeps its opposite envelope witness");
                let first_outer = first.center
                    + Quat::from_rotation_y(first.yaw) * Vec3::new(first.half_extents.x, 0.0, 0.0);
                let second_outer = second.center
                    + Quat::from_rotation_y(second.yaw)
                        * Vec3::new(second.half_extents.x, 0.0, 0.0);
                assert!(
                    (first_outer.distance(second_outer) - first.cluster_diameter).abs() < 1.0e-3,
                    "{id} cluster {cluster_index}"
                );
            }
            assert_eq!(
                !plan.cloud_shadows.is_empty(),
                id == "clouds-08-rounded-shadow",
                "{id}"
            );
            if id == "clouds-08-rounded-shadow" {
                assert!(plan.cloud_shadows.iter().all(|shadow| {
                    (shadow.maximum_opacity - 0.20).abs() < 1.0e-6
                        && (shadow.blur_world - 24.0).abs() < 1.0e-6
                }));
                let without_resolved_shadow_batches = plan.presentation_entities(0);
                assert_eq!(
                    plan.presentation_entities(7),
                    without_resolved_shadow_batches.saturating_add(7),
                    "C08 entity accounting must use renderer-observed chunk batches, not projector count"
                );
            }
        }
    }

    #[test]
    fn faceted_altitude_variants_share_exact_xz_layout_and_dimensions() {
        let profiles = atomic_profiles("clouds-")
            .into_iter()
            .map(|profile| (profile.active_treatment_ids().remove(0).to_owned(), profile))
            .collect::<BTreeMap<_, _>>();
        let input = fixture();
        let layout = |id: &str| {
            build_liquid_atmosphere_review_plan(profiles.get(id).expect("cloud profile"), &input)
                .expect("cloud plan")
                .cloud_puffs
                .into_iter()
                .map(|puff| {
                    (
                        puff.cluster_index,
                        puff.puff_index,
                        puff.cluster_center.x.to_bits(),
                        puff.cluster_center.z.to_bits(),
                        puff.cluster_diameter.to_bits(),
                        puff.center.x.to_bits(),
                        puff.center.z.to_bits(),
                        puff.half_extents.to_array().map(f32::to_bits),
                        puff.yaw.to_bits(),
                    )
                })
                .collect::<Vec<_>>()
        };
        let clear = layout("clouds-01-faceted-clear");
        assert_eq!(clear, layout("clouds-02-faceted-grazing"));
        assert_eq!(clear, layout("clouds-03-faceted-crossing"));
    }

    #[test]
    fn cloud_peak_intersection_fails_closed_when_the_required_band_is_hollow() {
        let profile = atomic_profiles("clouds-")
            .into_iter()
            .find(|profile| {
                profile
                    .active_treatment_ids()
                    .contains(&"clouds-03-faceted-crossing")
            })
            .expect("crossing cloud profile");
        let mut input = fixture();
        input.interaction_peak_solid_spans = vec![ReviewPeakSolidSpanV1 {
            bottom_y: 52.0,
            top_y: 56.0,
        }];
        assert_eq!(
            build_liquid_atmosphere_review_plan(&profile, &input),
            Err(ReviewWorldDetailEffectError::MissingCloudPeakIntersection)
        );
    }

    #[test]
    fn valley_water_anchor_serves_both_single_placement_fog_profiles_once() {
        let kind = ReviewEffectAnchorKindV1::ValleyWater;
        assert!(fog_anchor_eligible(LocalFogPlacementV1::WaterHugging, kind));
        assert!(fog_anchor_eligible(LocalFogPlacementV1::ValleyFloor, kind));
        assert!(fog_anchor_eligible(LocalFogPlacementV1::Mixed, kind));
    }

    #[test]
    fn all_six_fog_settings_keep_bottoms_and_measure_non_degenerate_coverage() {
        let input = fixture();
        let profiles = atomic_profiles("fog-");
        assert_eq!(profiles.len(), 6);
        let mut active_samples_by_id = BTreeMap::new();
        for profile in profiles {
            let id = profile.active_treatment_ids().remove(0);
            let plan =
                build_liquid_atmosphere_review_plan(&profile, &input).expect("fog plan must build");
            let (placement, coverage) = match profile.local_fog {
                LocalFogDetailV1::Layer {
                    placement,
                    coverage,
                    ..
                } => (placement, coverage),
                LocalFogDetailV1::Current => panic!("atomic fog profile must be active"),
            };
            let eligible_count = input
                .effect_anchors
                .iter()
                .filter(|anchor| fog_anchor_eligible(placement, anchor.kind))
                .count();
            assert_eq!(plan.fog_volumes.len(), eligible_count, "{id}");
            let evidence = plan
                .effect_validation
                .fog_coverage
                .expect("active fog retains spatial coverage evidence");
            let (_mask, sample_count, active_samples) =
                fog_density_xz_mask(coverage).expect("valid matrix coverage");
            assert_eq!(
                evidence.target_fraction.to_bits(),
                coverage.to_bits(),
                "{id}"
            );
            assert_eq!(evidence.sample_count, sample_count, "{id}");
            assert_eq!(evidence.active_samples, active_samples, "{id}");
            assert_eq!(evidence.fog_volumes as usize, eligible_count, "{id}");
            active_samples_by_id.insert(id.to_owned(), active_samples);
            assert!(plan.render_state.volumetric_fog, "{id}");
            assert!(plan.render_state.restore_volumetric_fog, "{id}");
            for volume in &plan.fog_volumes {
                let anchor = input
                    .effect_anchors
                    .iter()
                    .find(|anchor| anchor.name == volume.anchor_name)
                    .expect("named fog anchor");
                let bottom = volume.center.y - volume.half_extents.y;
                assert!((bottom - anchor.surface.y - 0.15).abs() < 1.0e-5, "{id}");
                assert_eq!(volume.coverage.to_bits(), coverage.to_bits(), "{id}");
            }
        }
        for (light, heavy) in [
            ("fog-01-water-light", "fog-02-water-heavy"),
            ("fog-03-valley-light", "fog-04-valley-heavy"),
            ("fog-05-mixed", "fog-06-mixed-cinematic"),
        ] {
            let light_samples = active_samples_by_id
                .get(light)
                .expect("the light fog profile retains its active sample count");
            let heavy_samples = active_samples_by_id
                .get(heavy)
                .expect("the heavy fog profile retains its active sample count");
            assert!(light_samples < heavy_samples);
        }
    }

    #[test]
    fn ice_coverage_variants_are_nested_under_one_common_shore_rank() {
        let profiles = atomic_profiles("ice-")
            .into_iter()
            .map(|profile| (profile.active_treatment_ids().remove(0).to_owned(), profile))
            .collect::<BTreeMap<_, _>>();
        let input = fixture();
        let edges = |id: &str| {
            build_liquid_atmosphere_review_plan(profiles.get(id).expect("ice profile"), &input)
                .expect("ice plan")
                .shoreline_edges
                .into_iter()
                .collect::<BTreeSet<_>>()
        };
        let level_40 = edges("ice-01-level-narrow");
        let level_65 = edges("ice-02-level-medium");
        let level_85 = edges("ice-03-level-wide");
        assert!(level_40.is_subset(&level_65));
        assert!(level_65.is_subset(&level_85));
        let snow_65 = edges("ice-04-snow-adjacent");
        let frozen_65 = edges("ice-05-frozen-or-snow");
        let frozen_75 = edges("ice-06-frozen-or-snow-feathered");
        assert!(snow_65.is_subset(&frozen_65));
        assert!(frozen_65.is_subset(&frozen_75));
    }

    #[test]
    fn reordering_inputs_preserves_plan_and_hash() {
        let profile = atomic_profiles("water-")
            .into_iter()
            .find(|profile| {
                profile
                    .active_treatment_ids()
                    .contains(&"water-04-depth-short")
            })
            .expect("W04 profile");
        let input = fixture();
        let mut reordered = input.clone();
        reordered.liquids.reverse();
        reordered.shore_surfaces.reverse();
        reordered.effect_anchors.reverse();

        let first = build_liquid_atmosphere_review_plan(&profile, &input).expect("first plan");
        let second =
            build_liquid_atmosphere_review_plan(&profile, &reordered).expect("second plan");
        assert_eq!(first, second);
        assert_eq!(first.plan_hash, second.plan_hash);
    }

    #[test]
    fn phase_bound_plan_hash_updates_without_rebuilding_static_projection() {
        let profile = atomic_profiles("water-")
            .into_iter()
            .find(|profile| {
                profile
                    .active_treatment_ids()
                    .contains(&"water-06-transmission")
            })
            .expect("W06 profile");
        let plan = build_liquid_atmosphere_review_plan(&profile, &fixture()).expect("W06 plan");
        let neutral = plan.phase_neutral_hash();
        assert_eq!(
            LiquidAtmosphereReviewPlanV1::bind_phase_hash(neutral, plan.phase_seconds),
            Some(plan.plan_hash)
        );
        let next = LiquidAtmosphereReviewPlanV1::bind_phase_hash(neutral, 1.0)
            .expect("finite phase must hash");
        assert_ne!(next, plan.plan_hash);
        assert_eq!(
            LiquidAtmosphereReviewPlanV1::bind_phase_hash(neutral, f32::NAN),
            None
        );
        let mut advanced = plan;
        advanced.phase_seconds = 1.0;
        advanced.plan_hash = next;
        assert!(advanced.hash_is_valid());
    }

    #[test]
    fn every_atomic_effect_plan_has_finite_correctly_wound_meshes() {
        let input = fixture();
        for profile in ReviewWorldDetailProfileV1::atomic_matrix()
            .into_iter()
            .filter(|profile| {
                profile.active_treatment_ids().first().is_some_and(|id| {
                    id.starts_with("water-")
                        || id.starts_with("clouds-")
                        || id.starts_with("shore-")
                        || id.starts_with("ice-")
                        || id.starts_with("fog-")
                })
            })
        {
            let id = profile.active_treatment_ids().remove(0);
            let plan = build_liquid_atmosphere_review_plan(&profile, &input)
                .unwrap_or_else(|error| panic!("{id} did not build: {error}"));
            for batch in &plan.mesh_batches {
                validate_mesh(&batch.mesh)
                    .unwrap_or_else(|error| panic!("{id} invalid mesh: {error}"));
            }
        }
    }

    #[test]
    fn malformed_exact_inputs_fail_before_any_projection() {
        let mut duplicate = fixture();
        let first = duplicate.liquids.first().copied().expect("fixture liquid");
        duplicate.liquids.push(first);
        assert_eq!(
            build_liquid_atmosphere_review_plan(&ReviewWorldDetailProfileV1::default(), &duplicate,),
            Err(ReviewWorldDetailEffectError::DuplicateLiquid(
                first.position
            ))
        );

        let mut non_finite = fixture();
        non_finite.phase_seconds = f32::NAN;
        assert_eq!(
            build_liquid_atmosphere_review_plan(
                &ReviewWorldDetailProfileV1::default(),
                &non_finite,
            ),
            Err(ReviewWorldDetailEffectError::NonFiniteInput)
        );
    }
}
