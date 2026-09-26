//! Optional decorative ground cover. These records never supply world occupancy.
use hex_world_contracts::{ChunkId, ContractError, VoxelPosition};
use serde::{Deserialize, Serialize};

/// Maximum authored tufts in one terrain chunk.
pub const MAX_CHUNK_TUFTS: usize = 64;
/// Maximum records in the compact, whole-world companion.
pub const MAX_GROUND_TUFTS: usize = 60_000;

/// Exact grounded placements of small, nonblocking voxel plants.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroundCover {
    /// Companion schema version.
    pub version: u32,
    /// Strictly sorted nonempty chunk partitions.
    pub chunks: Vec<GroundCoverChunk>,
}
/// A bounded chunk batch, admitted only over resident detailed terrain.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroundCoverChunk {
    /// Owning terrain chunk; no neighboring source dependency is needed.
    pub coordinate: ChunkId,
    /// Strictly sorted, unique supporting columns.
    pub tufts: Vec<GroundTuft>,
}
/// Presentation-only plants rooted on an exact original terrain material.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroundTuft {
    /// Topmost occupied terrain cell beneath the plant.
    pub support: VoxelPosition,
    /// Required original support material; construction must not grow grass.
    pub material: String,
    /// One of six deterministic asymmetric low voxel silhouettes.
    pub variant: u8,
}
impl GroundCover {
    /// Check finite geometry, unique partitions and explicit allocation budgets.
    pub fn validate(&self) -> Result<(), ContractError> {
        let invalid =
            || ContractError::new("overview/ground_cover", "invalid bounded ground cover");
        if self.version != 1 || self.chunks.len() > 4096 {
            return Err(invalid());
        }
        let mut count = 0;
        let mut previous = None;
        for chunk in &self.chunks {
            if previous.is_some_and(|p| p >= chunk.coordinate)
                || chunk.tufts.is_empty()
                || chunk.tufts.len() > MAX_CHUNK_TUFTS
            {
                return Err(invalid());
            }
            previous = Some(chunk.coordinate);
            let mut column = None;
            for tuft in &chunk.tufts {
                let p = tuft.support.column;
                if p.chunk() != chunk.coordinate
                    || column.is_some_and(|old| old >= p)
                    || p.q.unsigned_abs().max(p.r.unsigned_abs()) > 900
                    || p.q.checked_add(p.r).is_none_or(|s| s.unsigned_abs() > 900)
                    || !(400..=1000).contains(&tuft.support.level)
                    || !matches!(tuft.material.as_str(), "moss" | "soil")
                    || tuft.variant >= 6
                {
                    return Err(invalid());
                }
                column = Some(p);
            }
            count += chunk.tufts.len();
            if count > MAX_GROUND_TUFTS {
                return Err(invalid());
            }
        }
        Ok(())
    }
}
