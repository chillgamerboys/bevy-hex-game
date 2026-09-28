//! Disposable, bounded presentation of validated V4 resident chunks.
//!
//! This adapter never installs the legacy grid plugin or a complete `VoxelMap`.
//! [`crate::v4::ResidentRun`] retains exact global identity; the legacy `HexCoord`, `TilePos`
//! and `SubstanceId` components are local picking mirrors, not world authority.
//! Liquid intervals and static-object occupancy use exact prism geometry. Grand's
//! directed liquids can add saved-clock flow shading without displacing the prisms.
//! Authored object assets and interior cutaways are not rendered by this adapter.
//! The original semantic descriptors remain available through [`crate::v4::TerrainPresenter::package`].

pub(crate) mod grand_terrain_shading;
mod halo;
mod prepare;
mod publish;
pub(crate) mod river;

pub use halo::{RenderHalo, RenderHaloDependency, RenderNeighbor, MAX_RENDER_HALO_COLUMNS};
pub use prepare::{PreparedChunk, PresentationLimits, RenderOrigin, TerrainPreparer};
#[cfg(any(feature = "arena-prototype", test))]
pub(crate) use publish::DryMaterialFinish;
pub use publish::{ChunkReceipt, ResidentChunk, TerrainPresenter};

use bevy::prelude::Component;
use hex_world_contracts::VoxelPosition;

/// Which resident occupancy layer supplied this exact visible interval.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunSource {
    /// Editable terrain, including exact liquid-material intervals.
    Terrain,
    /// Clipped static-object occupancy; detailed authored art remains unresolved.
    StaticObject,
}

/// Exact global metadata on a disposable logical picking entity.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct ResidentRun {
    /// Topmost global material voxel of this interval.
    pub position: VoxelPosition,
    /// Inclusive global bottom level.
    pub bottom: i32,
    /// Exclusive global top level.
    pub top: i32,
    /// Stable material ID from this world's palette.
    pub material: String,
    /// Exact clear levels immediately above the interval; `None` is open sky.
    pub headroom: Option<u32>,
    /// Terrain or static-object provenance for this interval.
    pub source: RunSource,
}

/// A rejected preparation or publication; existing resident roots remain intact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresentationError(String);

impl std::fmt::Display for PresentationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PresentationError {}

impl From<hex_world_contracts::ContractError> for PresentationError {
    fn from(error: hex_world_contracts::ContractError) -> Self {
        Self(error.to_string())
    }
}

#[cfg(test)]
mod tests;
