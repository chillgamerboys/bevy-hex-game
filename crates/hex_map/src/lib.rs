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
//! [`Headroom`](hex_core::Headroom). Exact optional-region memberships are published
//! separately through [`SpecialMovementRegions`](hex_core::SpecialMovementRegions).
//! `hex_units` queries the tile components off the entities. It never reads
//! [`HeightMap`] or any other type defined here.
//!
//! The practical consequence: **how terrain is generated and stored is entirely
//! internal.** Replace the generator or key the map differently — as long as tile
//! entities preserve the complete component contract, the rest of the game keeps
//! working.
//!
//! # Columns
//!
//! There is one voxel [`Column`] per coordinate. Rendering merges each contiguous
//! non-air material run into an entity carrying a [`HexSpan`](hex_core::HexSpan) with
//! a `bottom` and a `top`. Floating platforms, water, overhangs, and bridges are
//! separate runs within that same column.

use bevy::prelude::*;

mod feature_render;
/// Terrain height generation.
pub mod generator;
/// Turning generated terrain into tile entities.
pub mod grid;
mod liquid_render;
/// Versioned semantic-first procedural map generation and diagnostics.
mod procedural;
mod procedural_v2;
mod procedural_v3;
/// Designer-facing map settings, loaded from RON.
pub mod settings;
/// Pure construction of complete voxel maps from terrain presets.
mod terrain;
/// Voxel storage and the run-merging that turns it into prisms.
pub mod voxel;

pub use generator::{FlatGenerator, HeightGenerator, HeightMap, PerlinGenerator, PerlinStep};
pub use liquid_render::LiquidVisualTime;
pub use procedural::{
    ForestMetrics as ForestReportMetrics, FortMetrics as FortReportMetrics, GenerationReport,
    ProceduralRecipeMetrics, TacticalMetrics, WaterfallMetrics as WaterfallReportMetrics,
};
pub use settings::{
    BridgeSettings, CavesSettings, CrossingSettings, CubeCoord, DerivedHillsCrossing,
    EdgeElevationSettings, EdgeLiquidPortSettings, EdgeLiquidSettings, EnvironmentSettings,
    HillsSettings, LandformSettings, LayeredSkyIslandsSettings, LinkedIslandsSettings, MapSettings,
    MountainSettings, MountainsSettings, NamedOverlaySettings, PatchEdgeContractSettings,
    PatchEdgesSettings, PatchMaskSettings, PatchSpec, PerlinSettings, PerlinStepSettings,
    ProceduralSettings, ProceduralV1Settings, ProceduralV2Settings, ProceduralV3Settings,
    RiverSettings, SharedEdgeSettings, ShowcaseSettings, SkyIslandsSettings, TacticalSettings,
    TerrainSettings, V2EnvironmentSettings, V2HillsSettings, V2RecipeSettings, V3CavesSettings,
    V3EnvironmentSettings, V3ForestSettings, V3FortSettings, V3HillsSettings, V3LayoutSettings,
    V3MountainsSettings, V3OverlaySettings, V3RecipeSettings, V3Ring7Settings,
    V3SkyIslandsSettings, V3WaterfallSettings, WalkerPortSettings,
};
pub use voxel::{runs, Column, SubstanceRun, VoxelMap};

/// Registers map settings, terrain generation, and tile spawning.
pub fn plugin(app: &mut App) {
    app.add_plugins((settings::plugin, grid::plugin));
}
