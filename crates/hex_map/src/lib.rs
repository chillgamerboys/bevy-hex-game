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
//! its inclusive [`RunBottom`](hex_core::RunBottom), [`HexSpan`](hex_core::HexSpan),
//! [`SubstanceId`](hex_core::SubstanceId), and [`Headroom`](hex_core::Headroom).
//! Exact optional-region memberships are published
//! separately through [`SpecialMovementRegions`].
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
//! There is one voxel [`Column`] per coordinate. Each contiguous non-air material run
//! publishes a lightweight logical entity carrying a [`HexSpan`](hex_core::HexSpan)
//! with a `bottom` and a `top`; bounded chunk/material/cutaway meshes combine those
//! prisms for rendering and resolve pointer hits back to the logical entity. Floating
//! platforms, water, overhangs, and bridges remain separate runs within one column.

use bevy::prelude::*;
use hex_assets::{RuntimeArtCatalog, SubstanceTable};
use hex_core::{
    BiomeRegions, InteriorRegions, MapAnchors, MapObservationAnchors, MapViewHint,
    SpecialMovementRegions, TraversalBlockers,
};

mod crystal_render;
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
/// Strict review-only world-detail profiles and capture reports.
#[cfg(feature = "map-review")]
pub mod review_world_detail;
/// Pure liquid and atmospheric projections for world-detail review.
#[cfg(feature = "map-review")]
pub mod review_world_detail_effects;
#[cfg(feature = "map-review")]
mod review_world_detail_render;
#[cfg(feature = "map-review")]
pub use review_world_detail_render::{
    ReviewSuppressedWaterMaterial, ReviewWorldDetailEntity, ReviewWorldDetailLifecycleSystems,
};
/// Pure terrain-attached projections for world-detail review.
#[cfg(feature = "map-review")]
pub(crate) mod review_world_detail_terrain;
/// Designer-facing map settings, loaded from RON.
pub mod settings;
/// Deterministic, renderer-free structural projections for Grand V3 review.
pub mod structural_preview;
/// Pure construction of complete voxel maps from terrain presets.
mod terrain;
mod terrain_damage;
mod terrain_noise;
/// Voxel storage and the run-merging that turns it into prisms.
pub mod voxel;
mod world_snapshot;

pub use generator::{FlatGenerator, HeightGenerator, HeightMap, PerlinGenerator, PerlinStep};
pub use hex_schematic::SchematicPlanV1;
/// Original liquid material type exposed for strict review binding checks.
#[cfg(feature = "map-review")]
pub use liquid_render::LiquidMaterial as ReviewLiquidMaterial;
pub use liquid_render::LiquidVisualTime;
pub use procedural::{
    CavesMetrics as CavesReportMetrics, CrystalAscentMetrics as CrystalAscentReportMetrics,
    DeepForestMetrics as DeepForestReportMetrics, DesertPlainMetrics as DesertPlainReportMetrics,
    DesertTransitionMetrics as DesertTransitionReportMetrics, DunesMetrics as DunesReportMetrics,
    ForestMetrics as ForestReportMetrics, FortMetrics as FortReportMetrics, GenerationReport,
    GrandV3Metrics, MacroMetrics, MountainRangeMetrics, OasisMetrics as OasisReportMetrics,
    OceanArchipelagoMetrics, PrairieMetrics as PrairieReportMetrics, ProceduralRecipeMetrics,
    Ring19Metrics, Ring7Metrics, SandyIsletsMetrics as SandyIsletsReportMetrics, TacticalMetrics,
    VolcanoMetrics as VolcanoReportMetrics, WaterfallMetrics as WaterfallReportMetrics,
    WoodedIslandMetrics as WoodedIslandReportMetrics,
};
#[cfg(feature = "map-review")]
pub use review_world_detail::ReviewWorldDetailProjectionHashesV1;
pub use settings::{
    BridgeSettings, CavesSettings, CrossingSettings, CubeCoord, DerivedHillsCrossing,
    EdgeElevationSettings, EdgeLiquidPortSettings, EdgeLiquidSettings, EnvironmentSettings,
    HillsSettings, LandformSettings, LayeredSkyIslandsSettings, LinkedIslandsSettings,
    MacroAccessSettings, MacroAxisSettings, MacroBiomeInstanceSettings, MacroElevationSettings,
    MacroHeadwaterSettings, MacroLayoutSettings, MacroLiquidConnectionSettings, MapSettings,
    MountainSettings, MountainsSettings, NamedOverlaySettings, PatchEdgeContractSettings,
    PatchEdgesSettings, PatchMaskSettings, PatchSpec, PerlinSettings, PerlinStepSettings,
    ProceduralSettings, ProceduralV1Settings, ProceduralV2Settings, ProceduralV3Settings,
    Ring19BoundaryOutletSettings, Ring19BoundarySide, Ring19LiquidConnectionSettings,
    Ring19RegionSettings, RiverSettings, SharedEdgeSettings, ShowcaseSettings, SkyIslandsSettings,
    TacticalSettings, TerrainSettings, V2EnvironmentSettings, V2HillsSettings, V2RecipeSettings,
    V3BeachSettings, V3CavesSettings, V3CrystalAscentSettings, V3DeepForestSettings,
    V3DeepMountainSettings, V3EnvironmentSettings, V3ForestSettings, V3FortSettings,
    V3GrandV3BasicTerrainProfile, V3HillsSettings, V3LayoutSettings, V3MountainsSettings,
    V3OverlaySettings, V3PrairieSettings, V3RecipeSettings, V3Ring19Settings, V3Ring7Settings,
    V3SandyIsletsSettings, V3SchematicLayoutSettings, V3SchematicTemplate,
    V3SchematicTerrainProfile, V3ShallowSeaSettings, V3ShoreSettings, V3SkyIslandsSettings,
    V3VolcanoSettings, V3WaterfallSettings, V3WoodedIslandSettings, WalkerPortSettings,
    V3_GRAND_V3_TEMPLATE_REVISION, V3_MACRO_CELL_COUNT, V3_MOUNTAIN_RANGE_REGION_COUNT,
    V3_RING19_REGION_COUNT, V3_SCHEMATIC_CELL_PITCH, V3_SCHEMATIC_GRID_RADIUS,
};
pub use voxel::{runs, terrain_chunk_key, Column, SubstanceRun, VoxelMap};
pub use world_snapshot::{
    apply_world_delta_v1, diff_world_snapshots_v1, export_world_snapshot_v1,
    fingerprint_world_snapshot_v1, validate_world_snapshot_v1_against_content,
    CampaignWorldRestoreOutcomeV2, CampaignWorldRestoreRefusalV2, CampaignWorldRestoreResultV2,
    CurrentWorldSnapshotV1, PendingCampaignWorldSnapshotV2, WorldReplicationOutcomeV1,
    WorldReplicationRefusalV1, WorldReplicationRequestV1, WorldReplicationResultV1,
    WorldReplicationStateV1, WorldSnapshotError,
};

/// Public, fully compiled result of compiling one exact Grand V3 schematic.
///
/// Presentation descriptors remain map-owned implementation detail, while review
/// tools receive every gameplay-authoritative projection, the separate scenic
/// landmark projection, and the same generation report used by runtime setup.
#[derive(Debug)]
pub struct CompiledSchematicMap {
    /// Exact chunk-native voxel storage.
    pub map: VoxelMap,
    /// Stable authored standable gameplay anchors.
    pub anchors: MapAnchors,
    /// Stable scenic camera/review landmarks that are not placement targets.
    pub observation_anchors: MapObservationAnchors,
    /// Exact special-movement memberships.
    pub special_regions: SpecialMovementRegions,
    /// Exact interior memberships.
    pub interiors: InteriorRegions,
    /// Exact movement blockers.
    pub blockers: TraversalBlockers,
    /// Stable biome ownership of exposed surfaces.
    pub biome_regions: BiomeRegions,
    /// Camera framing derived from complete world bounds.
    pub view_hint: MapViewHint,
    /// Deterministic compiler identities, provenance, and measurements.
    pub report: GenerationReport,
    presentation: procedural_v3::MapPresentationProjection,
}

/// Cardinality of exact map-owned presentation descriptors retained by one
/// compiled schematic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchematicPresentationCounts {
    /// Exact liquid voxels carrying material and flow presentation metadata.
    pub liquids: usize,
    /// Exact generated surface features.
    pub features: usize,
    /// Exact authored structures.
    pub structures: usize,
    /// Exact authoritative gameplay lights and their optional visual owners.
    pub lights: usize,
}

impl CompiledSchematicMap {
    /// Returns exact presentation descriptor counts without exposing map-private
    /// planning identities as a second gameplay API.
    #[must_use]
    pub fn presentation_counts(&self) -> SchematicPresentationCounts {
        SchematicPresentationCounts {
            liquids: self.presentation.liquids().len(),
            features: self.presentation.features().len(),
            structures: self.presentation.structures().len(),
            lights: self.presentation.lights().len(),
        }
    }

    /// Installs the exact compiled artifact into a configured review/tool world,
    /// including the private presentation projection.
    ///
    /// The world must already contain the accepted [`MapSettings`] and
    /// [`SubstanceTable`] used for compilation. This is an offline/review resource
    /// boundary for exact inspection and snapshot export, not a second gameplay setup
    /// path. It deliberately does **not** publish [`hex_core::TerrainReady`]: seeded
    /// gameplay still enters through the normal grid materialization pipeline, which
    /// may claim readiness only after every chunk root and global projection exists.
    /// Public snapshot and ECS projections remain the inspection boundary for tools.
    pub fn publish(self, world: &mut World) {
        world.insert_resource(self.map);
        world.insert_resource(self.anchors);
        world.insert_resource(self.observation_anchors);
        world.insert_resource(self.special_regions);
        world.insert_resource(self.interiors);
        world.insert_resource(self.blockers);
        world.insert_resource(self.biome_regions);
        world.insert_resource(self.view_hint);
        world.insert_resource(self.presentation);
        world.insert_resource(self.report);
    }
}

/// Failure to compile an exact Grand V3 schematic into runtime map projections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchematicCompileError(String);

impl std::fmt::Display for SchematicCompileError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for SchematicCompileError {}

/// Deterministic evidence from the lightweight Grand V3 fine-topology gate.
///
/// This admission deliberately stops before authored-object construction,
/// vegetation, presentation, and voxel materialization. It does resolve the
/// complete radius-187 ownership masks, seeded coast, directed hydrology,
/// bridges, natural-pass grading and threshold admission, and the exact
/// four-wide tunnel splice used by the full compiler. The complete compiler
/// separately proves the final two upper routes as blocker-aware graph cuts
/// after Crystal construction and decoration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GrandV3TopologyAdmission {
    /// Semantic identity of the exact schematic plan admitted.
    pub schematic_fingerprint: u64,
    /// Number of canonical radius-eight schematic cells.
    pub schematic_cells: u32,
    /// Number of fine radius-187 columns assigned exactly once.
    pub fine_columns: u32,
    /// Number of non-empty fine biome owners after the Crystal site claim.
    pub fine_owners: u32,
    /// Ordered rows in the exact three-wide waterfall/river ribbon.
    pub hydrology_rows: u32,
    /// Unique fine cells in that ribbon after legal bend overlap.
    pub hydrology_cells: u32,
    /// Exact lane count at the single semantic sea outlet.
    pub hydrology_outlet_lanes: u32,
    /// Exact authored river crossings admitted before route grading.
    pub river_bridges: u32,
    /// Fine ordinary surfaces reserved by the seeded natural pass.
    pub natural_pass_surfaces: u32,
    /// Exact physical width selected by the independent `pass_width` stream.
    pub natural_pass_width: u32,
    /// Ordered rows in the exact four-wide tunnel splice.
    pub tunnel_rows: u32,
    /// Unique fine cells covered by those tunnel rows.
    pub tunnel_cells: u32,
    /// Declared upper-region routes whose fine terminal authorities resolve:
    /// natural pass and Crystal/tunnel.
    ///
    /// This lightweight gate does not construct the complete Crystal route;
    /// full compilation proves both routes independently connect and that no
    /// third lower-to-upper route survives their removal.
    pub upper_routes: u32,
}

/// Admits the seed-dependent fine topology of one validated Grand V3 plan.
///
/// Unlike [`compile_schematic`], this boundary does not load art or materialize
/// gameplay projections. It is intended for broad deterministic corpora that
/// must still exercise real fine ownership, hydrology, and route solvers.
pub fn admit_schematic_topology(
    plan: &SchematicPlanV1,
    settings: &MapSettings,
) -> Result<GrandV3TopologyAdmission, SchematicCompileError> {
    settings.validate().map_err(SchematicCompileError)?;
    let TerrainSettings::Procedural(ProceduralSettings::V3(v3)) = &settings.terrain else {
        return Err(SchematicCompileError(
            "schematic topology admission requires procedural V3 map settings".to_owned(),
        ));
    };
    procedural_v3::admit_schematic_topology(plan, settings.grid_radius, settings.level_height, v3)
        .map_err(|error| SchematicCompileError(error.to_string()))
}

/// Compiles a validated reference or generated schematic without rerunning the
/// schematic planner's 32-candidate selection.
///
/// The selected map settings must use the Grand V3 schematic layout. Substance
/// resolution and solidity are taken from the accepted runtime table, and authored
/// landmark geometry is resolved from the same accepted runtime art catalog used by
/// gameplay, so this path cannot diverge from normal materialization.
pub fn compile_schematic(
    plan: &SchematicPlanV1,
    settings: &MapSettings,
    substances: &SubstanceTable,
    art_catalog: &RuntimeArtCatalog,
) -> Result<CompiledSchematicMap, SchematicCompileError> {
    settings.validate().map_err(SchematicCompileError)?;
    let TerrainSettings::Procedural(ProceduralSettings::V3(v3)) = &settings.terrain else {
        return Err(SchematicCompileError(
            "exact schematic compilation requires procedural V3 map settings".to_owned(),
        ));
    };
    let palette = terrain::TerrainPalette::for_terrain(substances, &settings.terrain)
        .map_err(SchematicCompileError)?;
    let compiled = procedural_v3::compile_schematic_plan(
        plan,
        settings.grid_radius,
        settings.level_height,
        v3,
        &palette,
        &|substance| substances.is_solid(substance),
        art_catalog,
    )
    .map_err(|error| SchematicCompileError(error.to_string()))?;
    Ok(CompiledSchematicMap {
        map: compiled.map,
        anchors: compiled.anchors,
        observation_anchors: compiled.observation_anchors,
        special_regions: compiled.special_regions,
        interiors: compiled.interiors,
        blockers: compiled.blockers,
        biome_regions: compiled.biome_regions,
        view_hint: compiled.view_hint,
        report: compiled.report,
        presentation: compiled.presentation,
    })
}

/// Registers map settings, terrain generation, and tile spawning.
pub fn plugin(app: &mut App) {
    app.add_plugins((settings::plugin, grid::plugin));
}
