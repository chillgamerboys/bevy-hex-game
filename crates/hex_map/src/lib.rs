//! The map: terrain generation, tile spawning, and map settings.
//!
//! # This crate is a leaf
//!
//! Nothing depends on `hex_map` except the binary that wires it up. `hex_world`,
//! `hex_units`, `hex_core` and `hex_assets` cannot see it, which means changes
//! here cannot break the camera, the player, the screens or the menus. Cargo
//! enforces that — it is not a convention to remember.
//!
//! That is the point: this is the crate the map is owned in, and its blast radius
//! is deliberately bounded.
//!
//! # How the rest of the game sees the map
//!
//! Through components, not through this crate's types.
//!
//! Tile entities are spawned carrying [`HexCoord`](hex_core::HexCoord) and
//! [`HexSpan`](hex_core::HexSpan) — both defined in `hex_core`. `hex_units`
//! queries those components off the entities. It never reads [`HeightMap`] or any
//! other type defined here.
//!
//! The practical consequence: **how terrain is generated and stored is entirely
//! internal.** Replace the generator, key the map differently, spawn several
//! columns per coordinate — as long as tiles come out carrying a `HexCoord` and a
//! `HexSpan`, the rest of the game keeps working.
//!
//! # Columns
//!
//! [`HexSpan`](hex_core::HexSpan) is a column with a `bottom` and a `top`, not a
//! single elevation. Today every span the generator produces starts at ground level
//! (`bottom: 0`), because the terrain is a simple height field. Floating platforms,
//! overhangs, and bridges are expressible in the same type without changing anything
//! outside this crate.

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
