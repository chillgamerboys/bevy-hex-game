//! Immutable presentation facts derived from the same authored world geometry.
use hex_world_contracts::{ChunkId, ColumnData, LiquidKind, VoxelPosition, WorldHex};
use serde::{Deserialize, Serialize};

/// Maximum exact inland liquid columns in a distant presentation companion.
pub const MAX_INLAND_WATER_COLUMNS: usize = 100_000;
/// Maximum disconnected exposed intervals along one hex face.
pub const MAX_INLAND_WATER_SIDE_INTERVALS: usize = 16;
/// Total exact solid columns including chunk-edge halo, before allocation.
pub const MAX_INLAND_TERRAIN_COLUMNS: usize = 500_000;
/// Compact solid runs across all exact inland presentation columns.
pub const MAX_INLAND_TERRAIN_RUNS: usize = 1_500_000;

/// An authored camera in final runtime coordinates, independent of app literals.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NorthernReviewCamera {
    /// World-space camera eye before optional ordinary-body grounding.
    pub eye: [f32; 3],
    /// World-space target; grounding retains the complete target-minus-eye direction.
    pub target: [f32; 3],
    /// Exact supported local interest used to admit nearby geometry.
    pub interest: [f32; 3],
    /// Vertical world-unit span for FixedVertical orthographic framing at 1600×900.
    pub orthographic_span: Option<f32>,
    /// Admit an ordinary body and use its actual eye height before capturing.
    pub ground: bool,
}

/// Exact inland-water presentation partitioned without retaining terrain chunks.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InlandWaterOverview {
    /// Schema version, currently one.
    pub version: u32,
    /// Canonically ordered unique storage chunks; identities bind through the overview.
    pub chunks: Vec<InlandWaterChunk>,
}

/// At most 256 exact inland liquid columns in one storage chunk.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InlandWaterChunk {
    /// Owning storage coordinate; distant presentation never pins it.
    pub coordinate: ChunkId,
    /// Canonically ordered unique columns belonging to this chunk.
    pub columns: Vec<InlandWaterColumn>,
    /// Complete solid-only columns of this water-bearing chunk. Replaces the
    /// coarse terrain proxy here; these are presentation facts, not collision.
    pub terrain: Vec<ColumnData>,
    /// One exact neighboring-column ring, for face exposure and edge conformity.
    /// Halo columns must never render as duplicate terrain or request residency.
    pub halo: Vec<ColumnData>,
}

/// Exposed liquid faces measured against actual solid and liquid neighbors.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InlandWaterColumn {
    /// Exact axial liquid column.
    pub column: WorldHex,
    /// Inclusive occupied liquid bottom level.
    pub bottom: i32,
    /// Exclusive occupied liquid top level.
    pub top: i32,
    /// Actual semantic liquid kind, never inferred from distant shading.
    pub kind: LiquidKind,
    /// Exact receiving interval for a directed reach; standing bodies have none.
    pub downstream: Option<VoxelPosition>,
    /// Whether the actual upper face is exposed.
    pub exposed_top: bool,
    /// Whether the actual lower face is exposed.
    pub exposed_bottom: bool,
    /// Half-open exposed level intervals, ordered by axial directions
    /// (1,0),(0,1),(-1,1),(-1,0),(0,-1),(1,-1).
    pub sides: [Vec<[i32; 2]>; 6],
}
