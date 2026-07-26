//! The map: terrain generation, tile spawning, and map settings.
//!
//! # This crate is a leaf
//!
//! Nothing depends on `hex_map` except the binary that wires it up. `hex_world`,
//! `hex_units`, `hex_core` and `hex_assets` cannot see its implementation. Cargo
//! enforces that dependency direction; the component contract below is what keeps
//! runtime behaviour correct.
//!
//! That is the point: this is the crate the map is owned in, and its blast radius
//! is deliberately bounded.
//!
//! # How the rest of the game sees the map
//!
//! Through components, not through this crate's types.
//!
//! Tile entities are spawned carrying [`HexTile`](hex_core::HexTile),
//! [`HexCoord`](hex_core::HexCoord), a surface [`TilePos`](hex_core::TilePos),
//! [`HexSpan`](hex_core::HexSpan), [`SubstanceId`](hex_core::SubstanceId), and
//! [`Headroom`](hex_core::Headroom). `hex_units` queries those components off the
//! entities. It never reads [`HeightMap`] or any other type defined here.
//!
//! The practical consequence: **how terrain is generated and stored is entirely
//! internal.** Replace the generator or key the map differently — as long as tile
//! entities preserve the complete component contract, the rest of the game keeps
//! working.
//!
//! # Columns
//!
//! There is one voxel [`Column`] per coordinate. Rendering merges each contiguous
//! solid run into an entity carrying a [`HexSpan`](hex_core::HexSpan) with a `bottom`
//! and a `top`. Floating platforms, overhangs, and bridges are separate runs within
//! that same column.

use bevy::prelude::*;

/// Terrain height generation.
pub mod generator;
/// Turning generated terrain into tile entities.
pub mod grid;
/// Designer-facing map settings, loaded from RON.
pub mod settings;
/// Voxel storage and the run-merging that turns it into prisms.
pub mod voxel;

pub use generator::{FlatGenerator, HeightGenerator, HeightMap, PerlinGenerator, PerlinStep};
pub use settings::{MapSettings, PerlinStepSettings, TerrainSettings};
pub use voxel::{runs, Column, SubstanceRun, VoxelMap};

/// Registers map settings, terrain generation, and tile spawning.
pub fn plugin(app: &mut App) {
    app.add_plugins((settings::plugin, grid::plugin));
}
