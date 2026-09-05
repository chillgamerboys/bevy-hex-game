use crate::{ChunkId, VoxelPosition, WorldHex};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A non-air material interval `[bottom, top)`; top is exclusive.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoxelRun {
    /// Lowest occupied voxel.
    pub bottom: i32,
    /// First voxel above the run.
    pub top: i32,
    /// Stable material name.
    pub material: String,
}

/// All non-air intervals in a column, sorted, disjoint, and maximally coalesced.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColumnData {
    /// Exact world position.
    pub position: WorldHex,
    /// Canonical material intervals. An empty vector represents an actual empty column.
    pub runs: Vec<VoxelRun>,
}

/// Stable material policy shared by generation, queries and disposable presentation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterialSpec {
    /// Unique non-air stable name.
    pub id: String,
    /// Whether this material supplies solid occupancy and support.
    pub solid: bool,
    /// Whether ordinary terrain edits may remove it.
    pub diggable: bool,
    /// Display RGBA bytes; never a gameplay disclosure grant.
    pub color: [u8; 4],
}

/// Stable regional feature identity and map-display summary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureSummary {
    /// World-unique feature ID.
    pub id: String,
    /// Source region ID, independent of the storage chunk.
    pub region_id: String,
    /// Stable capability/category name.
    pub kind: String,
    /// Exact feature root or observation location.
    pub anchor: VoxelPosition,
    /// Optional stable renderer asset ID, not a filesystem path.
    pub asset: Option<String>,
}

/// Coarse world-map product, independent of terrain residency.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapSummaryCell {
    /// Sample position in world coordinates.
    pub position: WorldHex,
    /// Representative material voxel level.
    pub level: i32,
    /// Stable display material.
    pub material: String,
    /// Region supplying this sample.
    pub region_id: String,
}

/// Exact finite authoring region; never a storage, render, or encounter unit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegionDescriptor {
    /// Stable world-unique source region ID.
    pub id: String,
    /// World coordinate of the disk center.
    pub origin: WorldHex,
    /// Inclusive integer hex-disk radius.
    pub radius: u32,
    /// Identity of the complete region source input.
    pub source_fingerprint: u64,
}

/// An immutable compiled global chunk, possibly combining several regions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChunkPackage {
    /// Must equal the supported [`crate::SCHEMA_VERSION`].
    pub schema_version: u32,
    /// Stable owning world ID.
    pub world_id: String,
    /// Global chunk address.
    pub coordinate: ChunkId,
    /// Complete source-set identity used to compile this package.
    pub source_fingerprint: u64,
    /// Exactly the world's columns in this chunk, including empty columns.
    pub columns: Vec<ColumnData>,
    /// Features rooted in this chunk, sorted by ID.
    pub features: Vec<FeatureSummary>,
    /// Exact semantic consequences; omitted legacy producer values default to empty.
    #[serde(default)]
    pub semantics: ChunkSemantics,
    /// Hash of the canonical package with this field zeroed.
    pub fingerprint: u64,
}

/// Content-addressed package location supplied by the filesystem adapter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChunkDescriptor {
    /// Global chunk address.
    pub coordinate: ChunkId,
    /// Expected sealed package fingerprint.
    pub fingerprint: u64,
    /// Portable safe relative path, never absolute or traversing a parent.
    pub path: String,
}

/// One exact neighboring pair owned by a shared boundary contract.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundarySample {
    /// Column in the first region.
    pub a: WorldHex,
    /// Adjacent column in the second region.
    pub b: WorldHex,
    /// Agreed topmost solid voxel level on both sides.
    pub ground_level: i32,
    /// Optional topmost liquid voxel level on both sides.
    pub water_level: Option<i32>,
    /// Whether this seam must admit the ordinary two-level walker.
    pub required_access: bool,
}

/// One shared border authority, rather than independent neighboring guesses.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundaryContract {
    /// Stable world-unique boundary ID.
    pub id: String,
    /// First participating region.
    pub region_a: String,
    /// Second participating region.
    pub region_b: String,
    /// Canonical pairs, sorted by `(a, b)`.
    pub samples: Vec<BoundarySample>,
}

/// World index and independent low-resolution map product; no resident terrain required.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldManifest {
    /// Supported wire version.
    pub schema_version: u32,
    /// Stable world identity.
    pub world_id: String,
    /// Stable version of the compiler/capability set.
    pub compiler_version: String,
    /// Identity of the complete transitive world source.
    pub source_fingerprint: u64,
    /// Canonical material registry sorted by ID.
    pub materials: Vec<MaterialSpec>,
    /// Canonical finite region registry sorted by ID.
    pub regions: Vec<RegionDescriptor>,
    /// Chunk index sorted by coordinate.
    pub chunks: Vec<ChunkDescriptor>,
    /// Shared border contracts sorted by ID.
    pub boundaries: Vec<BoundaryContract>,
    /// World-map samples sorted by position.
    pub summary: Vec<MapSummaryCell>,
    /// World feature registry sorted by ID.
    pub features: Vec<FeatureSummary>,
    /// Hash of this canonical manifest with this field zeroed.
    pub fingerprint: u64,
}

/// Complete compile result; runtime adapters may load its chunks independently.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldPackage {
    /// Independently validated world index.
    pub manifest: WorldManifest,
    /// Exact packages indexed by their global coordinates.
    #[serde(deserialize_with = "crate::validation::deserialize_unique_map")]
    pub chunks: BTreeMap<ChunkId, ChunkPackage>,
}

/// Nature of a liquid interval; exact downstream links are separate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LiquidKind {
    /// Standing connected body with no directed outflow on this interval.
    Standing,
    /// Surface current or channel.
    Directed,
    /// Falling liquid interval.
    Waterfall,
}

/// Exact liquid interval and directed downstream consequences.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiquidColumn {
    /// Column containing the liquid, owned by this chunk.
    pub column: WorldHex,
    /// Inclusive lowest liquid voxel.
    pub bottom: i32,
    /// Exclusive upper liquid level.
    pub top: i32,
    /// Standing, flowing, or falling interpretation.
    pub kind: LiquidKind,
    /// Stable connected-body grouping, allowed across chunks.
    pub body_id: String,
    /// Exact downstream interval topmost voxels (`top - 1`), sorted and unique.
    pub downstream: Vec<VoxelPosition>,
}

/// Intended use of an anchor; observation does not grant actor placement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnchorRole {
    /// An ordinary two-level walker's gameplay placement surface.
    Gameplay,
    /// Scenic look-at point only.
    Observation,
    /// Required traversal connection surface.
    Transit,
}

/// An explicitly classified stable anchor rooted in exactly one chunk.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldAnchor {
    /// World-unique anchor ID.
    pub id: String,
    /// Source region ID.
    pub region_id: String,
    /// Exact supported voxel or scenic look-at location.
    pub position: VoxelPosition,
    /// Consumer authority this anchor is allowed to grant.
    pub role: AnchorRole,
}

/// Exact interior membership and roof extent at one column.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InteriorSpan {
    /// Interior group ID; repeated across columns and chunks intentionally.
    pub id: String,
    /// Member column.
    pub column: WorldHex,
    /// Topmost solid floor voxel.
    pub floor_level: i32,
    /// Inclusive first solid roof voxel, above the interior's clear interval.
    pub roof_bottom: i32,
    /// Exclusive roof end.
    pub roof_top: i32,
    /// Stable semantic light-domain ID, independent of renderer lights.
    pub light_domain: String,
}

/// Exact authored illumination source; no rendering or disclosure authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldLight {
    /// World-unique light ID.
    pub id: String,
    /// Exact source voxel, possibly air in an interior.
    pub position: VoxelPosition,
    /// Interior light domain, or exterior when absent.
    pub domain: Option<String>,
    /// Inclusive bright range in logical voxels.
    pub bright_radius: u32,
    /// Inclusive dim range, at least the bright range.
    pub dim_radius: u32,
}

/// Static authored object plus its complete exact occupancy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectInstance {
    /// World-unique object ID, retained across unload/reload.
    pub id: String,
    /// Source region ID.
    pub region_id: String,
    /// Stable renderer asset ID, independent of any filesystem root.
    pub asset: String,
    /// Placement root; the containing chunk owns the complete record exactly once.
    pub origin: VoxelPosition,
    /// Six-way rotation in `0..6`; occupancy below is already in world coordinates.
    pub rotation: u8,
    /// Exact material intervals, possibly spanning neighboring chunks.
    pub occupancy: Vec<ColumnData>,
}

/// Semantic projections accompanying authoritative intervals.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChunkSemantics {
    /// Exact static object occupancy clipped to this global chunk, sorted by column.
    /// Queries use this projection even when an object's root chunk is unloaded.
    /// [`WorldPackage::seal`] derives it from the complete root-object registry.
    #[serde(default)]
    pub occupancy: Vec<ColumnData>,
    /// Liquid intervals sorted by `(column, bottom, top, body_id)`.
    pub liquids: Vec<LiquidColumn>,
    /// Anchors sorted by world-unique ID.
    pub anchors: Vec<WorldAnchor>,
    /// Interior memberships sorted by `(id, column, floor_level)`.
    pub interiors: Vec<InteriorSpan>,
    /// Authored lights sorted by world-unique ID.
    pub lights: Vec<WorldLight>,
    /// Complete root-light records influencing any world column in this chunk.
    /// Sorted by light ID; retains illumination when the uniquely owning root chunk unloads.
    /// [`WorldPackage::seal`] derives the horizontal dim-radius footprint exactly.
    #[serde(default)]
    pub light_influences: Vec<WorldLight>,
    /// Authored objects sorted by world-unique ID.
    pub objects: Vec<ObjectInstance>,
}

/// Availability is independent from empty space and from gameplay disclosure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum QueryResult<T> {
    /// Exact resident value. `Ready(None)` alone can represent known air.
    Ready(T),
    /// World data exists but the relevant chunk is not resident.
    Unloaded(ChunkId),
    /// Outside all declared world region footprints.
    OutsideWorld,
}

/// Exact solid exposed support surface and known air clearance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Surface {
    /// Topmost material voxel of this support.
    pub position: VoxelPosition,
    /// Stable solid material ID.
    pub material: String,
    /// Exact clear voxels above; `None` means unbounded sky, never missing data.
    pub headroom: Option<u32>,
}

/// Shared terrain-query boundary, usable by both turn-based and continuous consumers.
pub trait WorldQuery {
    /// Query one voxel. Air must be `Ready(None)`, never an unavailable fallback.
    fn voxel(&self, position: VoxelPosition) -> QueryResult<Option<String>>;
    /// Query every exposed solid stack in a column, retaining exact levels.
    fn surfaces(&self, column: WorldHex) -> QueryResult<Vec<Surface>>;
    /// Current resident chunk revision, or none when not resident.
    fn revision(&self, chunk: ChunkId) -> Option<u64>;
}

/// A residency interest held by an actor or operation independently of encounter turns.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResidencyRequest {
    /// Stable interest ID.
    pub id: String,
    /// World-space interest center.
    pub center: WorldHex,
    /// Required activation radius.
    pub radius: u32,
    /// Hysteresis radius, at least the activation radius.
    pub retention_radius: u32,
    /// Relative scheduler priority.
    pub priority: u8,
}

/// One requested exact terrain assignment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoxelEdit {
    /// Exact target voxel.
    pub position: VoxelPosition,
    /// Stable material ID, or `None` to clear to air.
    pub material: Option<String>,
}

/// An atomic edit command with complete explicit chunk revision expectations.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldEditTransaction {
    /// Stable idempotency identity.
    pub id: String,
    /// Exactly the affected chunks and their expected revisions.
    #[serde(deserialize_with = "crate::validation::deserialize_unique_map")]
    pub expected_revisions: BTreeMap<ChunkId, u64>,
    /// Unique edits in canonical voxel order.
    pub edits: Vec<VoxelEdit>,
}

/// Immutable outcome notification after a committed world edit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldChange {
    /// Transaction whose atomic publication completed.
    pub transaction_id: String,
    /// Published revisions for changed chunks.
    #[serde(deserialize_with = "crate::validation::deserialize_unique_map")]
    pub revisions: BTreeMap<ChunkId, u64>,
    /// Unique affected columns in canonical order.
    pub changed_columns: Vec<WorldHex>,
}
