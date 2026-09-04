//! Runtime-loaded, versioned V4 authoring source. This is never compiled output.

use std::collections::BTreeMap;

use hex_world_contracts::{MaterialSpec, WorldHex};
use serde::{Deserialize, Serialize};

/// Current authoring schema; package schemas belong to `hex_world_contracts`.
pub const SOURCE_VERSION: u32 = 1;

/// One authored world with reusable region recipes and independently identified instances.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldSpec {
    /// Exact source schema version.
    pub version: u32,
    /// Stable world identity.
    pub id: String,
    /// World seed; feature streams additionally include stable region and operator IDs.
    pub seed: u64,
    /// Explicit material catalog consumed by both compiler and runtime.
    pub materials: Vec<MaterialSpec>,
    /// Reusable geometry recipes, keyed by stable source identity.
    pub recipes: BTreeMap<String, RegionRecipe>,
    /// Region instances. Input order has no semantic meaning.
    pub regions: Vec<RegionSpec>,
    /// One shared authority for each touching pair of region disks.
    #[serde(default)]
    pub connections: Vec<ConnectionSpec>,
}

/// Placement of one independently reusable authored region.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegionSpec {
    /// Stable instance identity, independent of vector ordering.
    pub id: String,
    /// Recipe key in the containing world.
    pub recipe: String,
    /// Exact world-space origin.
    pub origin: WorldHex,
    /// Exact integer hex-disk radius; Grand-sized regions use 187.
    pub radius: u32,
    /// Counterclockwise sixty-degree turns applied to geometry and metadata.
    pub rotation: u8,
}

/// Data-defined natural and constructed geography in region-local coordinates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegionRecipe {
    /// Baseline exposed terrain level before bounded landform contributions.
    pub base_level: i32,
    /// Material bands below the top surface.
    pub strata: StrataSpec,
    /// Independent named terrain influences.
    #[serde(default)]
    pub landforms: Vec<LandformSpec>,
    /// Material/vegetation zones, resolved by explicit priority then stable ID.
    #[serde(default)]
    pub biomes: Vec<BiomeSpec>,
    /// Standing pools with exact level and depth.
    #[serde(default)]
    pub basins: Vec<BasinSpec>,
    /// Directed channels with explicit grade controls and physical falls.
    #[serde(default)]
    pub channels: Vec<ChannelSpec>,
    /// Reserved ground routes, constructed before vegetation.
    #[serde(default)]
    pub routes: Vec<RouteSpec>,
    /// Additive decks, retaining existing ground or water beneath.
    #[serde(default)]
    pub bridges: Vec<BridgeSpec>,
    /// Subtractive underground rooms/tunnels with exact roofs and entrances.
    #[serde(default)]
    pub caves: Vec<CaveSpec>,
    /// Reusable voxel prefabs and deterministic local feature rules.
    #[serde(default)]
    pub features: Vec<FeatureRule>,
    /// Explicit terrain/material overrides, applied before infrastructure.
    #[serde(default)]
    pub overrides: Vec<OverrideSpec>,
    /// Exact local safe hub for generated boundary approaches.
    pub hub: GradePoint,
}

/// Material stack defined relative to the exposed ground level.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StrataSpec {
    /// Non-diggable foundation material, exactly level zero.
    pub bedrock: String,
    /// Deep solid substrate.
    pub rock: String,
    /// Near-surface solid substrate.
    pub soil: String,
    /// Number of soil voxels immediately beneath the exposed cap.
    pub soil_depth: u32,
    /// Default exposed cap material.
    pub surface: String,
}

/// Bounded field; a ridge is represented by a chain of source points.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LandformSpec {
    /// Stable stage/operator identity.
    pub id: String,
    /// Sources of the bounded ridge/peak/plateau field.
    pub centers: Vec<WorldHex>,
    /// Radius outside which this operator contributes exactly zero.
    pub radius: u32,
    /// Flat core radius; zero yields a peaked profile.
    pub plateau_radius: u32,
    /// Signed change from baseline at the core.
    pub rise: i32,
    /// Maximum coherent fine relief, tapering to zero at the support boundary.
    pub relief: u32,
}

/// Circular authored mask, evaluated in the region's exact local frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiskMask {
    /// Local center.
    pub center: WorldHex,
    /// Inclusive hex radius.
    pub radius: u32,
}

/// Surface-material region independent of landform influence boundaries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BiomeSpec {
    /// Stable zone ID.
    pub id: String,
    /// Exact local mask.
    pub mask: DiskMask,
    /// Higher priority wins overlapping zones; ties use stable ID.
    pub priority: i32,
    /// Surface material.
    pub material: String,
}

/// Standing-water basin and its local terrain bank transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BasinSpec {
    /// Stable body identity within this region.
    pub id: String,
    /// Exact wet footprint.
    pub mask: DiskMask,
    /// Topmost occupied liquid voxel level.
    pub water_level: i32,
    /// Occupied liquid depth.
    pub depth: u32,
    /// Liquid material.
    pub material: String,
    /// Bed cap material.
    pub bed_material: String,
    /// Width over which the banks blend back to the existing terrain.
    pub bank_width: u32,
}

/// Exact local path control with a pinned exposed surface level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GradePoint {
    /// Region-local column.
    pub column: WorldHex,
    /// Topmost occupied voxel of the controlled surface.
    pub level: i32,
}

/// Directed liquid centerline with reproducible integer interpolation between controls.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelSpec {
    /// Stable watercourse identity.
    pub id: String,
    /// Ordered local path controls, upstream first; at least two.
    pub points: Vec<GradePoint>,
    /// Hex radius swept around the centerline; one produces a substantial three-wide core.
    pub half_width: u32,
    /// Liquid depth beneath each water surface.
    pub depth: u32,
    /// Liquid material.
    pub material: String,
    /// Solid channel-bed material.
    pub bed_material: String,
    /// Bank transition support radius outside the wet ribbon.
    pub bank_width: u32,
    /// Control-segment indices whose drop occurs at the final centerline edge,
    /// producing a physical waterfall instead of an interpolated ramp.
    #[serde(default)]
    pub falls_after: Vec<usize>,
}

/// A route grades the complete ribbon and protects its clear volume from decoration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteSpec {
    /// Stable route identity.
    pub id: String,
    /// Ordered surface pins.
    pub points: Vec<GradePoint>,
    /// Protected ribbon radius.
    pub half_width: u32,
    /// Bounded shoulder transition width.
    pub shoulder_width: u32,
    /// Exact exposed route material.
    pub material: String,
}

/// Elevated solid deck with genuinely separate lower geometry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BridgeSpec {
    /// Stable structure identity.
    pub id: String,
    /// Ordered deck controls; endpoint approaches must already meet these levels.
    pub points: Vec<GradePoint>,
    /// Deck ribbon radius.
    pub half_width: u32,
    /// Solid deck thickness in voxels.
    pub thickness: u32,
    /// Deck material.
    pub material: String,
}

/// Underground room chain with a connecting tunnel and optional explicit entrances.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaveSpec {
    /// Stable interior identity.
    pub id: String,
    /// Room masks; each shares the same floor and headroom.
    pub rooms: Vec<DiskMask>,
    /// Ordered local tunnel centerline controls, including exterior entrances.
    pub path: Vec<WorldHex>,
    /// Tunnel radius around its centerline.
    pub half_width: u32,
    /// Topmost occupied cave-floor voxel.
    pub floor_level: i32,
    /// Number of clear air voxels above the floor.
    pub clearance: u32,
    /// Minimum retained solid roof thickness.
    pub roof_thickness: u32,
    /// Exact floor/roof lining material.
    pub material: String,
    /// Explicit exterior columns with an open sky aperture instead of a roof.
    #[serde(default)]
    pub entrances: Vec<WorldHex>,
    /// Deterministic gameplay light spacing along the route; zero disables lights.
    pub light_spacing: u32,
}

/// Exact reusable structure or plant pattern in root-relative coordinates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureRule {
    /// Stable rule identity.
    pub id: String,
    /// Informational feature family, e.g. tree, ruin, crystal or shrub.
    pub kind: String,
    /// Existing art asset name used by presentation adapters.
    pub asset: String,
    /// Source provenance for an exported stock blueprint, or `None` for an
    /// explicitly procedural prefab. The pure compiler never reads this path.
    #[serde(default)]
    pub provenance: Option<PrefabProvenance>,
    /// Eligible local disk; excludes water, structures, routes and interiors.
    pub mask: DiskMask,
    /// Per-column acceptance in parts per ten thousand. No world-wide top-K cap.
    pub density: u32,
    /// Exact local roots always requested, independently of density.
    #[serde(default)]
    pub roots: Vec<WorldHex>,
    /// Reusable occupied offsets above the supporting ground surface.
    pub voxels: Vec<FeatureVoxel>,
}

/// Occupied vertical interval relative to the feature's origin above ground.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureVoxel {
    /// Local column offset before rotation.
    pub offset: WorldHex,
    /// Inclusive bottom offset; zero starts immediately above support.
    pub bottom: i32,
    /// Exclusive top offset.
    pub top: i32,
    /// Occupied material.
    pub material: String,
}

/// Versioned evidence for exact stock-art geometry exported into authoring data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrefabProvenance {
    /// Repository-relative source blueprint path.
    pub source_path: String,
    /// Exact source git revision used for the export.
    pub source_revision: String,
    /// Explicit conversion from visual style IDs to this world's material policy.
    /// Geometry parity does not imply parity with a legacy two-dimensional blocker mask.
    pub style_materials: BTreeMap<String, String>,
}

/// A hard terrain constraint applied before infrastructure, then rechecked after
/// every modifying stage. Conflicts identify both source operators.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverrideSpec {
    /// Stable override identity.
    pub id: String,
    /// Affected local disk.
    pub mask: DiskMask,
    /// Optional exact new terrain surface height.
    pub surface_level: Option<i32>,
    /// Optional new exposed cap material.
    pub material: Option<String>,
}

/// Shared boundary authority; both region inputs derive from this one record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionSpec {
    /// Stable world-level seam identity.
    pub id: String,
    /// First region ID.
    pub region_a: String,
    /// Second region ID.
    pub region_b: String,
    /// Exact dry ground boundary datum.
    pub ground_level: i32,
    /// Bounded interior transition width in each region.
    pub transition_width: u32,
    /// Whether an ordinary walking approach connects each region hub to the seam.
    pub required_access: bool,
    /// Optional common standing-water continuation through the seam midpoint.
    pub water: Option<BoundaryWaterSpec>,
}

/// Shared still-water crossing with exact same-level approaches on both sides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundaryWaterSpec {
    /// Topmost occupied liquid voxel on both sides.
    pub level: i32,
    /// Occupied liquid depth.
    pub depth: u32,
    /// Liquid ribbon radius.
    pub half_width: u32,
    /// Liquid material.
    pub material: String,
    /// Bed material.
    pub bed_material: String,
    /// Optional exact global endpoints of a directed same-level crossing.
    /// Omitting this creates a shared standing-water ribbon.
    #[serde(default)]
    pub flow: Option<BoundaryFlowSpec>,
}

/// Exact world-coordinate flow controls crossing the two declared source regions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundaryFlowSpec {
    /// Upstream surface column in either participating region.
    pub upstream: WorldHex,
    /// Downstream terminal surface in the opposite participating region.
    pub downstream: WorldHex,
}
