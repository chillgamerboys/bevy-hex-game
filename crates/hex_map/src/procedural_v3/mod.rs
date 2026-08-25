//! Procedural world generation V3 foundation.
//!
//! V3 remains isolated from the temporary V1/V2 implementations. Settings dispatch
//! may select it before every recipe is available, but unsupported recipes fail
//! setup explicitly rather than publishing an empty or partially validated world.

use std::fmt;
use std::time::Instant;

use hex_assets::RuntimeArtCatalog;
use hex_core::{
    BiomeRegions, InteriorRegions, MapAnchors, MapViewHint, SpecialMovementRegions, SubstanceId,
    TraversalBlockers,
};

use crate::procedural::{
    CavesMetrics as CavesReportMetrics, CrystalAscentMetrics as CrystalAscentReportMetrics,
    DeepForestMetrics as DeepForestReportMetrics, DesertPlainMetrics as DesertPlainReportMetrics,
    DesertTransitionMetrics as DesertTransitionReportMetrics, DunesMetrics as DunesReportMetrics,
    ForestMetrics as ForestReportMetrics, FortMetrics as FortReportMetrics, GenerationReport,
    GrandV3Metrics as GrandV3ReportMetrics, HillsMetrics as HillsReportMetrics,
    MountainsMetrics as MountainsReportMetrics, OasisMetrics as OasisReportMetrics,
    PrairieMetrics as PrairieReportMetrics, ProceduralRecipeMetrics,
    SandyIsletsMetrics as SandyIsletsReportMetrics, TacticalMetrics,
    VolcanoMetrics as VolcanoReportMetrics, WaterfallMetrics as WaterfallReportMetrics,
    WoodedIslandMetrics as WoodedIslandReportMetrics,
};
use crate::settings::{ProceduralV3Settings, V3LayoutSettings, V3RecipeSettings};
use crate::terrain::TerrainPalette;
use crate::voxel::VoxelMap;
use materialize::{MaterializationError, MaterializedV3World};
use selection::{CandidateNote, ValidatedWorldSelection};
use world::WorldValidationIssue;

mod arid_landform;
mod caves;
pub(crate) use caves::{CaveCrystalAssetError, CaveCrystalObjectSet};
mod coastal_island;
mod crystal_ascent;
mod crystal_ascent_assets;
pub(crate) use crystal_ascent_assets::{CrystalAscentAssetError, CrystalAscentObjectSet};
mod composite_patch;
mod composition;
mod deep_forest;
mod desert_plain;
mod desert_transition;
mod desert_vegetation;
#[cfg(test)]
mod dry_patch_tests;
mod dunes;
mod fingerprint;
mod forest;
mod fort;
mod hills;
#[expect(
    dead_code,
    reason = "resolved layouts are consumed by sequential V3 recipe implementations"
)]
mod layout;
mod local_frame;
mod macro_alpine;
mod macro_landform;
mod macro_spanning;
mod macro_world;
pub(crate) use layout::HexSide;
#[expect(
    dead_code,
    reason = "liquid topology is consumed by the sequential V3 Waterfall recipe"
)]
mod liquid;
pub(crate) use liquid::LiquidFlowState;
mod materialize;
pub(crate) use materialize::{MapPresentationProjection, MaterializedLiquidVoxel};
mod mountains;
mod oasis;
mod patch;
mod prairie;
mod ring19;
mod ring7;
mod river_terrain;
mod routing;
mod sandy_islets;
mod schematic;
mod seam;
mod seed;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the whole-world runner is consumed by sequential V3 recipes"
    )
)]
mod selection;
mod sky;
mod traversal;
mod vegetation;
mod vegetation_landform;
mod volcano;
#[expect(
    dead_code,
    reason = "the volume foundation is consumed by sequential V3 recipe implementations"
)]
mod volume;
pub(crate) use volume::FillMaterialRole;
mod waterfall;
mod wooded_island;
mod world;
pub(crate) use world::{
    CaveCrystalKind, CaveCrystalPresentation, CaveCrystalSiteKind, CrystalAscentCrystalKind,
    CrystalAscentCrystalPresentation, FeatureId, FeatureKind, LightId, PlannedFeature,
    PlannedGameplayLight, PlannedLightPresentation,
};

/// Failure to construct or validate one V3 world.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum V3GenerationError {
    /// The sequential recipe PR has not supplied this implementation yet.
    RecipeUnavailable(&'static str),
    /// A recipe violated an invariant required by the common candidate runner.
    RecipeContract(String),
    /// Candidate construction encountered a failure that is not a normal rejection.
    FatalCandidateConstruction { candidate: u8, source: Box<Self> },
    /// Candidate repair encountered a failure that is not a normal rejection.
    FatalCandidateRepair {
        candidate: u8,
        round: u8,
        source: Box<Self>,
    },
    /// The separately authored canonical fallback could not be constructed.
    FatalFallbackConstruction(Box<Self>),
    /// The canonical fallback failed common or recipe-specific validation.
    InvalidFallback(Vec<WorldValidationIssue>),
    /// A deterministic fingerprint could not encode a semantic value.
    Fingerprint(String),
    /// A validated semantic plan could not be materialized atomically.
    Materialization(MaterializationError),
}

impl fmt::Display for V3GenerationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RecipeUnavailable(recipe) => {
                write!(formatter, "procedural V3 recipe {recipe} is not available")
            }
            Self::RecipeContract(reason) => {
                write!(formatter, "procedural V3 recipe contract failed: {reason}")
            }
            Self::FatalCandidateConstruction { candidate, source } => write!(
                formatter,
                "procedural V3 candidate {candidate} construction failed fatally: {source}"
            ),
            Self::FatalCandidateRepair {
                candidate,
                round,
                source,
            } => write!(
                formatter,
                "procedural V3 candidate {candidate} repair round {round} failed fatally: \
                 {source}"
            ),
            Self::FatalFallbackConstruction(source) => {
                write!(
                    formatter,
                    "procedural V3 canonical fallback failed: {source}"
                )
            }
            Self::InvalidFallback(issues) => write!(
                formatter,
                "invalid procedural V3 canonical fallback: {}",
                issues
                    .iter()
                    .map(|issue| format!("{:?}: {}", issue.code, issue.detail))
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
            Self::Fingerprint(reason) => {
                write!(formatter, "procedural V3 fingerprint failed: {reason}")
            }
            Self::Materialization(error) => {
                write!(formatter, "procedural V3 materialization failed: {error}")
            }
        }
    }
}

impl std::error::Error for V3GenerationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::FatalCandidateConstruction { source, .. }
            | Self::FatalCandidateRepair { source, .. }
            | Self::FatalFallbackConstruction(source) => Some(source),
            Self::Materialization(error) => Some(error),
            Self::RecipeUnavailable(_)
            | Self::RecipeContract(_)
            | Self::InvalidFallback(_)
            | Self::Fingerprint(_) => None,
        }
    }
}

/// Controlled dispatch point used until sequential V3 recipe PRs land.
///
/// Returning an explicit error is part of the foundation contract: construction
/// never fabricates an empty semantic plan for an unsupported layout or recipe.
#[cfg(test)]
pub(crate) fn ensure_recipe_available(
    settings: &ProceduralV3Settings,
) -> Result<(), V3GenerationError> {
    match &settings.layout {
        V3LayoutSettings::Single(patch)
            if matches!(
                patch.recipe,
                V3RecipeSettings::Hills(_)
                    | V3RecipeSettings::SkyIslands(_)
                    | V3RecipeSettings::Mountains(_)
                    | V3RecipeSettings::Waterfall(_)
                    | V3RecipeSettings::Forest(_)
                    | V3RecipeSettings::Fort(_)
                    | V3RecipeSettings::Caves(_)
                    | V3RecipeSettings::DeepForest(_)
                    | V3RecipeSettings::Volcano(_)
                    | V3RecipeSettings::Prairie(_)
                    | V3RecipeSettings::CrystalAscent(_)
                    | V3RecipeSettings::DesertTransition(_)
                    | V3RecipeSettings::DesertPlain(_)
                    | V3RecipeSettings::Dunes(_)
                    | V3RecipeSettings::Oasis(_)
                    | V3RecipeSettings::SandyIslets(_)
                    | V3RecipeSettings::WoodedIsland(_)
            ) =>
        {
            Ok(())
        }
        V3LayoutSettings::Single(patch) => Err(V3GenerationError::RecipeUnavailable(recipe_name(
            &patch.recipe,
        ))),
        V3LayoutSettings::Ring7(_) => Ok(()),
        V3LayoutSettings::Ring19(_) => Ok(()),
        V3LayoutSettings::Macro(_) => Ok(()),
        V3LayoutSettings::Schematic(_) => Ok(()),
    }
}

/// Fully materialized runtime publication for one admitted V3 world.
#[derive(Debug)]
pub(crate) struct ProceduralBuild {
    pub(crate) map: VoxelMap,
    pub(crate) anchors: MapAnchors,
    pub(crate) special_regions: SpecialMovementRegions,
    pub(crate) interiors: InteriorRegions,
    pub(crate) blockers: TraversalBlockers,
    pub(crate) biome_regions: BiomeRegions,
    pub(crate) view_hint: MapViewHint,
    pub(crate) presentation: MapPresentationProjection,
    pub(crate) report: GenerationReport,
}

/// Selects, validates, materializes, and reports one complete V3 world.
pub(crate) fn build(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV3Settings,
    seed: u64,
    palette: &TerrainPalette,
    is_solid: &dyn Fn(SubstanceId) -> bool,
    art_catalog: Option<&RuntimeArtCatalog>,
) -> Result<ProceduralBuild, V3GenerationError> {
    let started = Instant::now();
    match &settings.layout {
        V3LayoutSettings::Single(patch) if matches!(patch.recipe, V3RecipeSettings::Hills(_)) => {
            let art_catalog = art_catalog.ok_or_else(|| {
                V3GenerationError::RecipeContract(
                    "Hills requires the accepted runtime art catalog".to_owned(),
                )
            })?;
            finish_build(
                hills::generate(grid_radius, level_height, settings, seed, art_catalog)?,
                grid_radius,
                level_height,
                settings,
                seed,
                palette,
                is_solid,
                started,
                hills_report_metrics,
                |metrics| ProceduralRecipeMetrics::Hills(hills_recipe_metrics(metrics)),
            )
        }
        V3LayoutSettings::Single(patch)
            if matches!(patch.recipe, V3RecipeSettings::SkyIslands(_)) =>
        {
            let art_catalog = art_catalog.ok_or_else(|| {
                V3GenerationError::RecipeContract(
                    "Sky Islands requires the accepted runtime art catalog".to_owned(),
                )
            })?;
            finish_build(
                sky::generate(grid_radius, level_height, settings, seed, art_catalog)?,
                grid_radius,
                level_height,
                settings,
                seed,
                palette,
                is_solid,
                started,
                sky_report_metrics,
                |metrics| ProceduralRecipeMetrics::SkyIslands(sky_recipe_metrics(metrics)),
            )
        }
        V3LayoutSettings::Single(patch)
            if matches!(patch.recipe, V3RecipeSettings::Mountains(_)) =>
        {
            let art_catalog = art_catalog.ok_or_else(|| {
                V3GenerationError::RecipeContract(
                    "Mountains requires the accepted runtime art catalog".to_owned(),
                )
            })?;
            finish_build(
                mountains::generate(grid_radius, level_height, settings, seed, art_catalog)?,
                grid_radius,
                level_height,
                settings,
                seed,
                palette,
                is_solid,
                started,
                mountains_report_metrics,
                |metrics| ProceduralRecipeMetrics::Mountains(mountains_recipe_metrics(metrics)),
            )
        }
        V3LayoutSettings::Single(patch)
            if matches!(patch.recipe, V3RecipeSettings::Waterfall(_)) =>
        {
            let art_catalog = art_catalog.ok_or_else(|| {
                V3GenerationError::RecipeContract(
                    "Waterfall requires the accepted runtime art catalog".to_owned(),
                )
            })?;
            finish_build(
                waterfall::generate_with_catalog(
                    grid_radius,
                    level_height,
                    settings,
                    seed,
                    art_catalog,
                )?,
                grid_radius,
                level_height,
                settings,
                seed,
                palette,
                is_solid,
                started,
                waterfall_report_metrics,
                |metrics| ProceduralRecipeMetrics::Waterfall(waterfall_recipe_metrics(metrics)),
            )
        }
        V3LayoutSettings::Single(patch) if matches!(patch.recipe, V3RecipeSettings::Forest(_)) => {
            let art_catalog = art_catalog.ok_or_else(|| {
                V3GenerationError::RecipeContract(
                    "Forest requires the accepted runtime art catalog".to_owned(),
                )
            })?;
            finish_build(
                forest::generate(grid_radius, level_height, settings, seed, art_catalog)?,
                grid_radius,
                level_height,
                settings,
                seed,
                palette,
                is_solid,
                started,
                forest_report_metrics,
                |metrics| ProceduralRecipeMetrics::Forest(forest_recipe_metrics(metrics)),
            )
        }
        V3LayoutSettings::Single(patch) if matches!(patch.recipe, V3RecipeSettings::Fort(_)) => {
            finish_build(
                fort::generate(grid_radius, level_height, settings, seed)?,
                grid_radius,
                level_height,
                settings,
                seed,
                palette,
                is_solid,
                started,
                fort_report_metrics,
                |metrics| ProceduralRecipeMetrics::Fort(fort_recipe_metrics(metrics)),
            )
        }
        V3LayoutSettings::Single(patch) if matches!(patch.recipe, V3RecipeSettings::Caves(_)) => {
            let art_catalog = art_catalog.ok_or_else(|| {
                V3GenerationError::RecipeContract(
                    "Caves requires the accepted runtime art catalog".to_owned(),
                )
            })?;
            finish_build(
                caves::generate(grid_radius, level_height, settings, seed, art_catalog)?,
                grid_radius,
                level_height,
                settings,
                seed,
                palette,
                is_solid,
                started,
                caves_report_metrics,
                |metrics| ProceduralRecipeMetrics::Caves(caves_recipe_metrics(metrics)),
            )
        }
        V3LayoutSettings::Single(patch)
            if matches!(patch.recipe, V3RecipeSettings::DeepForest(_)) =>
        {
            let art_catalog = art_catalog.ok_or_else(|| {
                V3GenerationError::RecipeContract(
                    "Deep Forest requires the accepted runtime art catalog".to_owned(),
                )
            })?;
            finish_build(
                deep_forest::generate(grid_radius, level_height, settings, seed, art_catalog)?,
                grid_radius,
                level_height,
                settings,
                seed,
                palette,
                is_solid,
                started,
                deep_forest_report_metrics,
                |metrics| ProceduralRecipeMetrics::DeepForest(deep_forest_recipe_metrics(metrics)),
            )
        }
        V3LayoutSettings::Single(patch) if matches!(patch.recipe, V3RecipeSettings::Prairie(_)) => {
            let art_catalog = art_catalog.ok_or_else(|| {
                V3GenerationError::RecipeContract(
                    "Prairie requires the accepted runtime art catalog".to_owned(),
                )
            })?;
            finish_build(
                prairie::generate(grid_radius, level_height, settings, seed, art_catalog)?,
                grid_radius,
                level_height,
                settings,
                seed,
                palette,
                is_solid,
                started,
                prairie_report_metrics,
                |metrics| ProceduralRecipeMetrics::Prairie(prairie_recipe_metrics(metrics)),
            )
        }
        V3LayoutSettings::Single(patch) if matches!(patch.recipe, V3RecipeSettings::Volcano(_)) => {
            finish_build(
                volcano::generate(grid_radius, level_height, settings, seed)?,
                grid_radius,
                level_height,
                settings,
                seed,
                palette,
                is_solid,
                started,
                volcano_report_metrics,
                |metrics| ProceduralRecipeMetrics::Volcano(volcano_recipe_metrics(metrics)),
            )
        }
        V3LayoutSettings::Single(patch)
            if matches!(patch.recipe, V3RecipeSettings::CrystalAscent(_)) =>
        {
            let art_catalog = art_catalog.ok_or_else(|| {
                V3GenerationError::RecipeContract(
                    "Crystal Ascent requires the accepted runtime art catalog".to_owned(),
                )
            })?;
            finish_build(
                crystal_ascent::generate(grid_radius, level_height, settings, seed, art_catalog)?,
                grid_radius,
                level_height,
                settings,
                seed,
                palette,
                is_solid,
                started,
                crystal_ascent_report_metrics,
                |metrics| {
                    ProceduralRecipeMetrics::CrystalAscent(crystal_ascent_recipe_metrics(metrics))
                },
            )
        }
        V3LayoutSettings::Single(patch)
            if matches!(patch.recipe, V3RecipeSettings::DesertTransition(_)) =>
        {
            finish_build(
                desert_transition::generate(grid_radius, level_height, settings, seed)?,
                grid_radius,
                level_height,
                settings,
                seed,
                palette,
                is_solid,
                started,
                desert_transition_report_metrics,
                |metrics| {
                    ProceduralRecipeMetrics::DesertTransition(desert_transition_recipe_metrics(
                        metrics,
                    ))
                },
            )
        }
        V3LayoutSettings::Single(patch)
            if matches!(patch.recipe, V3RecipeSettings::DesertPlain(_)) =>
        {
            finish_build(
                desert_plain::generate(grid_radius, level_height, settings, seed)?,
                grid_radius,
                level_height,
                settings,
                seed,
                palette,
                is_solid,
                started,
                desert_plain_report_metrics,
                |metrics| {
                    ProceduralRecipeMetrics::DesertPlain(desert_plain_recipe_metrics(metrics))
                },
            )
        }
        V3LayoutSettings::Single(patch) if matches!(patch.recipe, V3RecipeSettings::Dunes(_)) => {
            finish_build(
                dunes::generate(grid_radius, level_height, settings, seed)?,
                grid_radius,
                level_height,
                settings,
                seed,
                palette,
                is_solid,
                started,
                dunes_report_metrics,
                |metrics| ProceduralRecipeMetrics::Dunes(dunes_recipe_metrics(metrics)),
            )
        }
        V3LayoutSettings::Single(patch) if matches!(patch.recipe, V3RecipeSettings::Oasis(_)) => {
            let art_catalog = art_catalog.ok_or_else(|| {
                V3GenerationError::RecipeContract(
                    "Oasis requires the accepted runtime art catalog".to_owned(),
                )
            })?;
            finish_build(
                oasis::generate(grid_radius, level_height, settings, seed, art_catalog)?,
                grid_radius,
                level_height,
                settings,
                seed,
                palette,
                is_solid,
                started,
                oasis_report_metrics,
                |metrics| ProceduralRecipeMetrics::Oasis(oasis_recipe_metrics(metrics)),
            )
        }
        V3LayoutSettings::Single(patch)
            if matches!(patch.recipe, V3RecipeSettings::SandyIslets(_)) =>
        {
            finish_build(
                sandy_islets::generate(grid_radius, level_height, settings, seed)?,
                grid_radius,
                level_height,
                settings,
                seed,
                palette,
                is_solid,
                started,
                sandy_islets_report_metrics,
                |metrics| {
                    ProceduralRecipeMetrics::SandyIslets(sandy_islets_recipe_metrics(metrics))
                },
            )
        }
        V3LayoutSettings::Single(patch)
            if matches!(patch.recipe, V3RecipeSettings::WoodedIsland(_)) =>
        {
            let art_catalog = art_catalog.ok_or_else(|| {
                V3GenerationError::RecipeContract(
                    "Wooded Island requires the accepted runtime art catalog".to_owned(),
                )
            })?;
            finish_build(
                wooded_island::generate(grid_radius, level_height, settings, seed, art_catalog)?,
                grid_radius,
                level_height,
                settings,
                seed,
                palette,
                is_solid,
                started,
                wooded_island_report_metrics,
                |metrics| {
                    ProceduralRecipeMetrics::WoodedIsland(wooded_island_recipe_metrics(metrics))
                },
            )
        }
        V3LayoutSettings::Single(patch) => Err(V3GenerationError::RecipeUnavailable(recipe_name(
            &patch.recipe,
        ))),
        V3LayoutSettings::Ring7(_) => {
            let art_catalog = art_catalog.ok_or_else(|| {
                V3GenerationError::RecipeContract(
                    "Ring7 requires the accepted runtime art catalog".to_owned(),
                )
            })?;
            finish_build(
                ring7::generate(grid_radius, level_height, settings, seed, art_catalog)?,
                grid_radius,
                level_height,
                settings,
                seed,
                palette,
                is_solid,
                started,
                ring7_report_metrics,
                |metrics| ProceduralRecipeMetrics::Ring7(ring7_recipe_metrics(metrics)),
            )
        }
        V3LayoutSettings::Ring19(_) => {
            let art_catalog = art_catalog.ok_or_else(|| {
                V3GenerationError::RecipeContract(
                    "Ring19 requires the accepted runtime art catalog".to_owned(),
                )
            })?;
            finish_build(
                ring19::generate(grid_radius, level_height, settings, seed, art_catalog)?,
                grid_radius,
                level_height,
                settings,
                seed,
                palette,
                is_solid,
                started,
                ring19_report_metrics,
                |metrics| ProceduralRecipeMetrics::Ring19(ring19_recipe_metrics(metrics)),
            )
        }
        V3LayoutSettings::Macro(_) => {
            let art_catalog = art_catalog.ok_or_else(|| {
                V3GenerationError::RecipeContract(
                    "Macro worlds require the accepted runtime art catalog".to_owned(),
                )
            })?;
            finish_build(
                macro_world::generate(grid_radius, level_height, settings, seed, art_catalog)?,
                grid_radius,
                level_height,
                settings,
                seed,
                palette,
                is_solid,
                started,
                macro_report_metrics,
                |metrics| {
                    if let Some(metrics) = metrics.ocean_archipelago {
                        ProceduralRecipeMetrics::OceanArchipelago(metrics)
                    } else if let Some(metrics) = metrics.mountain_range {
                        ProceduralRecipeMetrics::MountainRange(metrics)
                    } else {
                        ProceduralRecipeMetrics::Macro(metrics.report)
                    }
                },
            )
        }
        V3LayoutSettings::Schematic(_) => finish_build(
            schematic::generate(grid_radius, level_height, settings, seed)?,
            grid_radius,
            level_height,
            settings,
            seed,
            palette,
            is_solid,
            started,
            schematic_report_metrics,
            |metrics| ProceduralRecipeMetrics::GrandV3(schematic_recipe_metrics(metrics)),
        ),
    }
}

/// Compiles one already-generated Grand V3 schematic through the same semantic
/// validation, materialization, and reporting boundary as runtime generation.
///
/// This path deliberately skips schematic candidate generation: review tools can
/// compile an exact reference or saved plan without asking the planner to select
/// another candidate.
pub(crate) fn compile_schematic_plan(
    plan: &hex_schematic::SchematicPlanV1,
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV3Settings,
    palette: &TerrainPalette,
    is_solid: &dyn Fn(SubstanceId) -> bool,
) -> Result<ProceduralBuild, V3GenerationError> {
    let started = Instant::now();
    finish_build(
        schematic::compile_schematic(plan, settings, grid_radius, level_height)?,
        grid_radius,
        level_height,
        settings,
        plan.provenance.world_seed,
        palette,
        is_solid,
        started,
        schematic_report_metrics,
        |metrics| ProceduralRecipeMetrics::GrandV3(schematic_recipe_metrics(metrics)),
    )
}

fn schematic_report_metrics(metrics: &schematic::SchematicWorldMetrics) -> TacticalMetrics {
    TacticalMetrics {
        relief: metrics
            .maximum_surface
            .saturating_sub(metrics.minimum_surface),
        barrier_cells: metrics.water_columns,
        // The proxy labels ordinary surfaces but intentionally does not compile the
        // final route graph yet. Do not present authored intent as proven reachability.
        reachable_surfaces: 0,
        environment_signature_percent: 0,
        ..Default::default()
    }
}

const fn schematic_recipe_metrics(
    metrics: &schematic::SchematicWorldMetrics,
) -> GrandV3ReportMetrics {
    GrandV3ReportMetrics {
        schematic_cells: metrics.schematic_cells,
        world_columns: metrics.world_columns,
        resident_chunks: metrics.expected_chunks,
        ordinary_surfaces: metrics.ordinary_surfaces,
        water_columns: metrics.water_columns,
        liquid_bodies: metrics.liquid_bodies,
        minimum_surface: metrics.minimum_surface,
        maximum_surface: metrics.maximum_surface,
        schematic_fingerprint: metrics.schematic_fingerprint,
    }
}

fn macro_report_metrics(metrics: &macro_world::MacroWorldMetrics) -> TacticalMetrics {
    TacticalMetrics {
        relief: metrics.report.relief,
        barrier_cells: metrics.report.liquid_cells,
        critical_route_steps: metrics.report.critical_route_steps,
        reachable_surfaces: metrics.report.reachable_surfaces,
        reachable_elevation_levels: metrics.report.reachable_elevation_levels,
        environment_signature_percent: 0,
        ..Default::default()
    }
}

fn ring19_report_metrics(metrics: &ring19::Ring19Metrics) -> TacticalMetrics {
    TacticalMetrics {
        relief: metrics.report.relief,
        barrier_cells: metrics.report.liquid_cells,
        critical_route_steps: metrics.report.critical_route_steps,
        reachable_surfaces: metrics.report.reachable_surfaces,
        reachable_elevation_levels: metrics.report.reachable_elevation_levels,
        environment_signature_percent: 0,
        ..Default::default()
    }
}

const fn ring19_recipe_metrics(
    metrics: &ring19::Ring19Metrics,
) -> crate::procedural::Ring19Metrics {
    metrics.report
}

fn ring7_report_metrics(metrics: &ring7::Ring7Metrics) -> TacticalMetrics {
    TacticalMetrics {
        relief: metrics.report.relief,
        barrier_cells: metrics.report.liquid_cells,
        critical_route_steps: metrics.report.critical_route_steps,
        reachable_surfaces: metrics.report.reachable_surfaces,
        reachable_elevation_levels: metrics.report.reachable_elevation_levels,
        environment_signature_percent: 0,
        ..Default::default()
    }
}

const fn ring7_recipe_metrics(metrics: &ring7::Ring7Metrics) -> crate::procedural::Ring7Metrics {
    metrics.report
}

fn hills_report_metrics(metrics: &hills::HillsMetrics) -> TacticalMetrics {
    TacticalMetrics {
        relief: metrics.relief,
        critical_route_steps: metrics.critical_route_steps,
        reachable_surfaces: metrics.ordinary_surfaces,
        reachable_elevation_levels: metrics.reachable_elevation_levels,
        environment_signature_percent: 100,
        ..Default::default()
    }
}

fn hills_recipe_metrics(metrics: &hills::HillsMetrics) -> HillsReportMetrics {
    HillsReportMetrics {
        ordinary_surfaces: metrics.ordinary_surfaces,
        reachable_elevation_levels: metrics.reachable_elevation_levels,
        relief: metrics.relief,
        critical_route_steps: metrics.critical_route_steps,
        hill_centres: metrics.hill_centres,
        barrier_cells: metrics.barrier_cells,
        bridge_surfaces: metrics.bridge_surfaces,
        alternate_crossing_surfaces: metrics.alternate_crossing_surfaces,
    }
}

fn sky_report_metrics(metrics: &sky::SkyMetrics) -> TacticalMetrics {
    TacticalMetrics {
        relief: metrics.vertical_clearance,
        reachable_surfaces: metrics.ground_surfaces,
        environment_signature_percent: metrics.upper_coverage_percent,
        ..Default::default()
    }
}

fn sky_recipe_metrics(metrics: &sky::SkyMetrics) -> crate::procedural::SkyIslandsMetrics {
    crate::procedural::SkyIslandsMetrics {
        ground_surfaces: metrics.ground_surfaces,
        upper_surfaces: metrics.upper_surfaces,
        upper_coverage_percent: metrics.upper_coverage_percent,
        primary_islands: metrics.primary_islands,
        satellites: metrics.satellites,
        bridge_surfaces: metrics.bridge_surfaces,
        vertical_clearance: metrics.vertical_clearance,
    }
}

fn mountains_report_metrics(metrics: &mountains::MountainsMetrics) -> TacticalMetrics {
    TacticalMetrics {
        relief: metrics.relief,
        critical_route_steps: metrics.lower_bypass_steps,
        reachable_surfaces: metrics.ordinary_surfaces,
        reachable_elevation_levels: metrics.reachable_elevation_levels,
        environment_signature_percent: metrics.mountain_coverage_percent,
        ..Default::default()
    }
}

fn mountains_recipe_metrics(metrics: &mountains::MountainsMetrics) -> MountainsReportMetrics {
    MountainsReportMetrics {
        ordinary_surfaces: metrics.ordinary_surfaces,
        special_surfaces: metrics.special_surfaces,
        mountain_surfaces: metrics.mountain_surfaces,
        mountain_coverage_percent: metrics.mountain_coverage_percent,
        accessible_mountain_surfaces: metrics.accessible_mountain_surfaces,
        reachable_elevation_levels: metrics.reachable_elevation_levels,
        relief: metrics.relief,
        peak_count: metrics.peak_count,
        cliff_edges: metrics.cliff_edges,
        high_pass_steps: metrics.high_pass_steps,
        lower_bypass_steps: metrics.lower_bypass_steps,
    }
}

fn desert_transition_report_metrics(metrics: &DesertTransitionReportMetrics) -> TacticalMetrics {
    TacticalMetrics {
        relief: metrics.relief,
        critical_route_steps: metrics.critical_route_steps,
        reachable_surfaces: metrics.ordinary_surfaces,
        reachable_elevation_levels: metrics.reachable_elevation_levels,
        environment_signature_percent: metrics.dry_coverage_percent,
        ..Default::default()
    }
}

const fn desert_transition_recipe_metrics(
    metrics: &DesertTransitionReportMetrics,
) -> DesertTransitionReportMetrics {
    *metrics
}

fn desert_plain_report_metrics(metrics: &DesertPlainReportMetrics) -> TacticalMetrics {
    TacticalMetrics {
        relief: metrics.relief,
        critical_route_steps: metrics.critical_route_steps,
        reachable_surfaces: metrics.ordinary_surfaces,
        reachable_elevation_levels: metrics.reachable_elevation_levels,
        environment_signature_percent: metrics.sand_surface_percent,
        ..Default::default()
    }
}

const fn desert_plain_recipe_metrics(
    metrics: &DesertPlainReportMetrics,
) -> DesertPlainReportMetrics {
    *metrics
}

fn dunes_report_metrics(metrics: &DunesReportMetrics) -> TacticalMetrics {
    TacticalMetrics {
        relief: metrics.relief,
        critical_route_steps: metrics.critical_route_steps,
        reachable_surfaces: metrics.ordinary_surfaces,
        reachable_elevation_levels: metrics.reachable_elevation_levels,
        environment_signature_percent: metrics
            .crest_surfaces
            .saturating_mul(100)
            .checked_div(metrics.ordinary_surfaces)
            .unwrap_or_default(),
        ..Default::default()
    }
}

const fn dunes_recipe_metrics(metrics: &DunesReportMetrics) -> DunesReportMetrics {
    *metrics
}

fn oasis_report_metrics(metrics: &OasisReportMetrics) -> TacticalMetrics {
    TacticalMetrics {
        relief: metrics.relief,
        barrier_cells: metrics.water_cells,
        critical_route_steps: metrics.critical_route_steps,
        reachable_surfaces: metrics.ordinary_surfaces,
        reachable_elevation_levels: metrics.reachable_elevation_levels,
        environment_signature_percent: metrics
            .water_cells
            .saturating_add(metrics.grass_ring_surfaces)
            .saturating_mul(100)
            .checked_div(
                metrics
                    .ordinary_surfaces
                    .saturating_add(metrics.water_cells),
            )
            .unwrap_or_default(),
        ..Default::default()
    }
}

const fn oasis_recipe_metrics(metrics: &OasisReportMetrics) -> OasisReportMetrics {
    *metrics
}

fn sandy_islets_report_metrics(metrics: &SandyIsletsReportMetrics) -> TacticalMetrics {
    TacticalMetrics {
        relief: metrics.relief,
        barrier_cells: metrics.water_cells,
        critical_route_steps: metrics.critical_route_steps,
        reachable_surfaces: metrics.primary_reachable_surfaces,
        reachable_elevation_levels: metrics.reachable_elevation_levels,
        environment_signature_percent: metrics
            .land_surfaces
            .saturating_mul(100)
            .checked_div(metrics.world_columns)
            .unwrap_or_default(),
        ..Default::default()
    }
}

const fn sandy_islets_recipe_metrics(
    metrics: &SandyIsletsReportMetrics,
) -> SandyIsletsReportMetrics {
    *metrics
}

fn wooded_island_report_metrics(metrics: &WoodedIslandReportMetrics) -> TacticalMetrics {
    TacticalMetrics {
        relief: metrics.relief,
        barrier_cells: metrics.water_cells,
        critical_route_steps: metrics.critical_route_steps,
        reachable_surfaces: metrics.reachable_surfaces,
        reachable_elevation_levels: metrics.reachable_elevation_levels,
        environment_signature_percent: metrics
            .tree_roots
            .saturating_mul(100)
            .checked_div(metrics.grass_interior_surfaces)
            .unwrap_or_default(),
        ..Default::default()
    }
}

const fn wooded_island_recipe_metrics(
    metrics: &WoodedIslandReportMetrics,
) -> WoodedIslandReportMetrics {
    *metrics
}

#[expect(
    clippy::too_many_arguments,
    reason = "the V3 report boundary explicitly records every generation input"
)]
fn finish_build<M>(
    selection: ValidatedWorldSelection<M>,
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV3Settings,
    seed: u64,
    palette: &TerrainPalette,
    is_solid: &dyn Fn(SubstanceId) -> bool,
    started: Instant,
    tactical_metrics: fn(&M) -> TacticalMetrics,
    recipe_metrics: fn(&M) -> ProceduralRecipeMetrics,
) -> Result<ProceduralBuild, V3GenerationError> {
    let ValidatedWorldSelection {
        validated,
        metrics,
        selected_candidate,
        candidates_evaluated,
        valid_candidates,
        repair_rounds,
        used_fallback,
        notes,
    } = selection;
    let materialized = materialize::materialize(validated, palette, is_solid)
        .map_err(V3GenerationError::Materialization)?;
    let MaterializedV3World {
        map,
        anchors,
        special_regions,
        interiors,
        blockers,
        biome_regions,
        view_hint,
        semantic_fingerprint,
        materialized_fingerprint,
        presentation,
    } = materialized;
    let repair_actions = repair_rounds
        .iter()
        .flat_map(|round| {
            round.actions.iter().map(move |action| {
                format!("round {} {}: {}", round.index, action.code, action.detail)
            })
        })
        .collect();
    let repair_round_count = u8::try_from(repair_rounds.len()).unwrap_or(u8::MAX);
    let elapsed_micros = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
    let settings_fingerprint =
        fingerprint::settings_fingerprint(grid_radius, level_height, settings)
            .map_err(V3GenerationError::Fingerprint)?;
    let report = GenerationReport {
        generator_version: 3,
        seed,
        selected_candidate,
        candidates_evaluated,
        valid_candidates,
        repair_rounds: repair_round_count,
        repair_actions,
        used_fallback,
        settings_fingerprint,
        semantic_plan_fingerprint: Some(semantic_fingerprint),
        map_fingerprint: materialized_fingerprint,
        metrics: tactical_metrics(&metrics),
        recipe_metrics: Some(recipe_metrics(&metrics)),
        elapsed_micros,
        notes: candidate_notes(notes),
    };

    Ok(ProceduralBuild {
        map,
        anchors,
        special_regions,
        interiors,
        blockers,
        biome_regions,
        view_hint,
        presentation,
        report,
    })
}

fn waterfall_report_metrics(metrics: &waterfall::WaterfallMetrics) -> TacticalMetrics {
    let alternate_detour_percent = metrics
        .alternate_bypass_steps
        .saturating_sub(metrics.bypass_steps)
        .saturating_mul(100)
        .checked_div(metrics.bypass_steps)
        .unwrap_or_default();
    TacticalMetrics {
        relief: i32::try_from(metrics.dry_relief).unwrap_or(i32::MAX),
        barrier_cells: metrics.water_nodes,
        critical_route_steps: metrics.bypass_steps,
        spawn_height_difference: i32::try_from(metrics.spawn_height_difference).unwrap_or(i32::MAX),
        bank_high_ground_difference: i32::try_from(metrics.bank_high_ground_difference)
            .unwrap_or(i32::MAX),
        reachable_surfaces: metrics.ordinary_surfaces,
        reachable_elevation_levels: metrics.reachable_elevation_levels,
        alternate_detour_percent,
        river_sinuosity_percent: 0,
        environment_signature_percent: metrics.grass_surface_percent,
    }
}

fn waterfall_recipe_metrics(metrics: &waterfall::WaterfallMetrics) -> WaterfallReportMetrics {
    WaterfallReportMetrics {
        water_nodes: metrics.water_nodes,
        still_nodes: metrics.calm_nodes,
        current_nodes: metrics.current_nodes,
        rapid_nodes: metrics.rapid_nodes,
        fall_nodes: metrics.fall_nodes,
        fall_height: i32::try_from(metrics.fall_height).unwrap_or(i32::MAX),
        ordinary_surfaces: metrics.ordinary_surfaces,
        reachable_elevation_levels: metrics.reachable_elevation_levels,
        bypass_steps: metrics.bypass_steps,
        alternate_bypass_steps: metrics.alternate_bypass_steps,
        raised_terrain: metrics.raised_terrain,
    }
}

fn forest_report_metrics(metrics: &forest::ForestMetrics) -> TacticalMetrics {
    TacticalMetrics {
        relief: i32::try_from(metrics.relief).unwrap_or(i32::MAX),
        barrier_cells: 0,
        critical_route_steps: metrics.critical_route_steps,
        spawn_height_difference: i32::try_from(metrics.spawn_height_difference).unwrap_or(i32::MAX),
        bank_high_ground_difference: 0,
        reachable_surfaces: metrics.ordinary_surfaces,
        reachable_elevation_levels: metrics.reachable_elevation_levels,
        alternate_detour_percent: 0,
        river_sinuosity_percent: 0,
        environment_signature_percent: metrics
            .tree_roots
            .saturating_add(metrics.tall_grass_roots)
            .saturating_mul(100)
            .checked_div(
                metrics
                    .woodland_surfaces
                    .saturating_add(metrics.prairie_surfaces),
            )
            .unwrap_or_default(),
    }
}

fn forest_recipe_metrics(metrics: &forest::ForestMetrics) -> ForestReportMetrics {
    ForestReportMetrics {
        tree_roots: metrics.tree_roots,
        tree_blocker_surfaces: metrics.tree_blocker_surfaces,
        old_growth_roots: metrics.old_growth_roots,
        old_growth_blocker_surfaces: metrics.old_growth_blocker_surfaces,
        tall_grass_roots: metrics.tall_grass_roots,
        woodland_surfaces: metrics.woodland_surfaces,
        prairie_surfaces: metrics.prairie_surfaces,
        clearing_count: metrics.clearing_count,
        clearing_surfaces: metrics.clearing_surfaces,
        protected_route_surfaces: metrics.protected_route_surfaces,
        ordinary_surfaces: metrics.ordinary_surfaces,
        reachable_elevation_levels: metrics.reachable_elevation_levels,
        relief: i32::try_from(metrics.relief).unwrap_or(i32::MAX),
        critical_route_steps: metrics.critical_route_steps,
        spawn_height_difference: i32::try_from(metrics.spawn_height_difference).unwrap_or(i32::MAX),
        woodland_prairie_high_ground_difference: i32::try_from(
            metrics.woodland_prairie_high_ground_difference,
        )
        .unwrap_or(i32::MAX),
    }
}

fn deep_forest_report_metrics(metrics: &DeepForestReportMetrics) -> TacticalMetrics {
    TacticalMetrics {
        relief: metrics.relief,
        critical_route_steps: metrics.critical_route_steps,
        reachable_surfaces: metrics.ordinary_surfaces,
        reachable_elevation_levels: metrics.reachable_elevation_levels,
        environment_signature_percent: metrics.blocker_coverage_percent,
        ..Default::default()
    }
}

const fn deep_forest_recipe_metrics(metrics: &DeepForestReportMetrics) -> DeepForestReportMetrics {
    *metrics
}

fn prairie_report_metrics(metrics: &PrairieReportMetrics) -> TacticalMetrics {
    TacticalMetrics {
        relief: metrics.relief,
        critical_route_steps: metrics.critical_route_steps,
        reachable_surfaces: metrics.ordinary_surfaces,
        reachable_elevation_levels: metrics.reachable_elevation_levels,
        environment_signature_percent: metrics.grass_coverage_percent,
        ..Default::default()
    }
}

const fn prairie_recipe_metrics(metrics: &PrairieReportMetrics) -> PrairieReportMetrics {
    *metrics
}

fn volcano_report_metrics(metrics: &volcano::VolcanoMetrics) -> TacticalMetrics {
    TacticalMetrics {
        relief: metrics.summit_relief,
        barrier_cells: metrics.lava_nodes,
        critical_route_steps: metrics.critical_route_steps,
        reachable_surfaces: metrics.ordinary_surfaces,
        reachable_elevation_levels: metrics.reachable_elevation_levels,
        environment_signature_percent: metrics.massif_coverage_percent,
        ..Default::default()
    }
}

fn volcano_recipe_metrics(metrics: &volcano::VolcanoMetrics) -> VolcanoReportMetrics {
    VolcanoReportMetrics {
        ordinary_surfaces: metrics.ordinary_surfaces,
        reachable_elevation_levels: metrics.reachable_elevation_levels,
        summit_relief: metrics.summit_relief,
        massif_surfaces: metrics.massif_surfaces,
        massif_coverage_percent: metrics.massif_coverage_percent,
        lava_nodes: metrics.lava_nodes,
        fall_nodes: metrics.fall_nodes,
        maximum_fall_height: metrics.maximum_fall_height,
        bridge_surfaces: metrics.bridge_surfaces,
        bridge_clearance: metrics.bridge_clearance,
        critical_route_steps: metrics.critical_route_steps,
    }
}

fn fort_report_metrics(metrics: &fort::FortMetrics) -> TacticalMetrics {
    TacticalMetrics {
        relief: i32::try_from(metrics.relief).unwrap_or(i32::MAX),
        barrier_cells: 0,
        critical_route_steps: metrics.critical_route_steps,
        spawn_height_difference: 0,
        bank_high_ground_difference: 0,
        reachable_surfaces: metrics.ordinary_surfaces,
        reachable_elevation_levels: metrics.reachable_elevation_levels,
        alternate_detour_percent: 0,
        river_sinuosity_percent: 0,
        environment_signature_percent: metrics
            .worked_stone_surfaces
            .saturating_mul(100)
            .checked_div(metrics.ordinary_surfaces)
            .unwrap_or_default(),
    }
}

fn fort_recipe_metrics(metrics: &fort::FortMetrics) -> FortReportMetrics {
    FortReportMetrics {
        wall_voxels: metrics.wall_voxels,
        wall_walk_surfaces: metrics.wall_walk_surfaces,
        battlement_columns: metrics.battlement_columns,
        tower_count: metrics.tower_count,
        gate_count: metrics.gate_count,
        stair_count: metrics.stair_count,
        courtyard_surfaces: metrics.courtyard_surfaces,
        ordinary_surfaces: metrics.ordinary_surfaces,
        reachable_elevation_levels: metrics.reachable_elevation_levels,
        relief: i32::try_from(metrics.relief).unwrap_or(i32::MAX),
        curtain_height: i32::try_from(metrics.curtain_height).unwrap_or(i32::MAX),
        keep_height: i32::try_from(metrics.keep_height).unwrap_or(i32::MAX),
        critical_route_steps: metrics.critical_route_steps,
        independent_gate_routes: metrics.independent_gate_routes,
        worked_stone_surfaces: metrics.worked_stone_surfaces,
    }
}

fn caves_report_metrics(metrics: &caves::CavesMetrics) -> TacticalMetrics {
    TacticalMetrics {
        relief: i32::try_from(metrics.surface_relief).unwrap_or(i32::MAX),
        barrier_cells: 0,
        critical_route_steps: metrics.critical_route_steps,
        spawn_height_difference: 0,
        bank_high_ground_difference: 0,
        reachable_surfaces: metrics.reachable_surfaces,
        reachable_elevation_levels: metrics.reachable_elevation_levels,
        alternate_detour_percent: 0,
        river_sinuosity_percent: 0,
        environment_signature_percent: metrics.gravel_surface_percent,
    }
}

fn caves_recipe_metrics(metrics: &caves::CavesMetrics) -> CavesReportMetrics {
    CavesReportMetrics {
        chamber_count: metrics.chamber_count,
        covered_floors: metrics.covered_floors,
        critical_floors: metrics.critical_floors,
        optional_dark_floors: metrics.optional_dark_floors,
        gameplay_lights: metrics.gameplay_lights,
        moss_roots: metrics.moss_roots,
        lichen_roots: metrics.lichen_roots,
        vegetation_visual_voxels: metrics.vegetation_visual_voxels,
        minimum_roof_thickness: metrics.minimum_roof_thickness,
        minimum_clearance: metrics.minimum_clearance,
        maximum_clearance: metrics.maximum_clearance,
        surface_relief: i32::try_from(metrics.surface_relief).unwrap_or(i32::MAX),
        floor_relief: i32::try_from(metrics.floor_relief).unwrap_or(i32::MAX),
        entrance_steps: metrics.entrance_steps,
        critical_route_steps: metrics.critical_route_steps,
        reachable_surfaces: metrics.reachable_surfaces,
        reachable_elevation_levels: metrics.reachable_elevation_levels,
        gravel_surface_percent: metrics.gravel_surface_percent,
    }
}

fn crystal_ascent_report_metrics(
    metrics: &crystal_ascent::CrystalAscentMetrics,
) -> TacticalMetrics {
    TacticalMetrics {
        relief: metrics.rise_levels,
        critical_route_steps: metrics.critical_route_steps,
        reachable_surfaces: metrics.ordinary_surfaces,
        reachable_elevation_levels: metrics.reachable_elevation_levels,
        environment_signature_percent: 100,
        ..Default::default()
    }
}

fn crystal_ascent_recipe_metrics(
    metrics: &crystal_ascent::CrystalAscentMetrics,
) -> CrystalAscentReportMetrics {
    CrystalAscentReportMetrics {
        circuits: metrics.circuits,
        flights: metrics.flights,
        landings: metrics.landings,
        stair_surfaces: metrics.stair_surfaces,
        chamber_surfaces: metrics.chamber_surfaces,
        crown_surfaces: metrics.crown_surfaces,
        tree_roots: metrics.tree_roots,
        crystal_fixtures: metrics.crystal_fixtures,
        gameplay_lights: metrics.gameplay_lights,
        ordinary_surfaces: metrics.ordinary_surfaces,
        reachable_elevation_levels: metrics.reachable_elevation_levels,
        critical_route_steps: metrics.critical_route_steps,
        rise_levels: metrics.rise_levels,
        minimum_stair_headroom: metrics.minimum_stair_headroom,
    }
}

fn candidate_notes(notes: Vec<CandidateNote>) -> Vec<String> {
    let mut reported = Vec::new();
    for note in notes {
        match note {
            CandidateNote::ConstructionRejected { candidate, issues } => {
                append_issues(&mut reported, candidate, "construction", None, issues);
            }
            CandidateNote::ValidationRejected { candidate, issues } => {
                append_issues(&mut reported, candidate, "validation", None, issues);
            }
            CandidateNote::RepairRejected {
                candidate,
                round,
                issues,
            } => {
                append_issues(&mut reported, candidate, "repair", Some(round), issues);
            }
            CandidateNote::FallbackSelected => {
                reported.push("canonical fallback selected".to_owned());
            }
        }
    }
    reported
}

fn append_issues(
    reported: &mut Vec<String>,
    candidate: u8,
    stage: &str,
    round: Option<u8>,
    issues: Vec<WorldValidationIssue>,
) {
    let round = round.map_or_else(String::new, |round| format!(" round {round}"));
    for issue in issues {
        reported.push(format!(
            "candidate {candidate} {stage}{round} {:?}: {}",
            issue.code, issue.detail
        ));
    }
}

const fn recipe_name(recipe: &V3RecipeSettings) -> &'static str {
    match recipe {
        V3RecipeSettings::Hills(_) => "Hills",
        V3RecipeSettings::SkyIslands(_) => "SkyIslands",
        V3RecipeSettings::Mountains(_) => "Mountains",
        V3RecipeSettings::Caves(_) => "Caves",
        V3RecipeSettings::Waterfall(_) => "Waterfall",
        V3RecipeSettings::Forest(_) => "Forest",
        V3RecipeSettings::Fort(_) => "Fort",
        V3RecipeSettings::Volcano(_) => "Volcano",
        V3RecipeSettings::DeepForest(_) => "DeepForest",
        V3RecipeSettings::Prairie(_) => "Prairie",
        V3RecipeSettings::ShallowSea(_) => "ShallowSea",
        V3RecipeSettings::Beach(_) => "Beach",
        V3RecipeSettings::Shore(_) => "Shore",
        V3RecipeSettings::DeepMountain(_) => "DeepMountain",
        V3RecipeSettings::CrystalAscent(_) => "CrystalAscent",
        V3RecipeSettings::DesertTransition(_) => "DesertTransition",
        V3RecipeSettings::DesertPlain(_) => "DesertPlain",
        V3RecipeSettings::Dunes(_) => "Dunes",
        V3RecipeSettings::Oasis(_) => "Oasis",
        V3RecipeSettings::SandyIslets(_) => "SandyIslets",
        V3RecipeSettings::WoodedIsland(_) => "WoodedIsland",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{
        PatchEdgeContractSettings, PatchEdgesSettings, PatchMaskSettings, PatchSpec,
        V3CrystalAscentSettings, V3DesertPlainSettings, V3DesertTransitionSettings,
        V3DunesSettings, V3EnvironmentSettings, V3HillsSettings, V3OasisSettings,
        V3SandyIsletsSettings, V3WoodedIslandSettings,
    };

    fn world_edges() -> PatchEdgesSettings {
        PatchEdgesSettings {
            east: PatchEdgeContractSettings::WorldBoundary,
            south_east: PatchEdgeContractSettings::WorldBoundary,
            south_west: PatchEdgeContractSettings::WorldBoundary,
            west: PatchEdgeContractSettings::WorldBoundary,
            north_west: PatchEdgeContractSettings::WorldBoundary,
            north_east: PatchEdgeContractSettings::WorldBoundary,
        }
    }

    #[test]
    fn implemented_native_hills_is_reported_available() {
        let settings = ProceduralV3Settings {
            layout: V3LayoutSettings::Single(PatchSpec {
                environment: V3EnvironmentSettings::TemperateGrassland,
                recipe: V3RecipeSettings::Hills(V3HillsSettings {
                    valley_level: 15,
                    max_relief: 8,
                    hills_per_bank: 3,
                }),
                overlays: Vec::new(),
                mask: PatchMaskSettings::WholeWorld,
                edges: world_edges(),
            }),
        };

        assert_eq!(ensure_recipe_available(&settings), Ok(()));
    }

    #[test]
    fn implemented_crystal_ascent_is_reported_available() {
        let settings = ProceduralV3Settings {
            layout: V3LayoutSettings::Single(PatchSpec {
                environment: V3EnvironmentSettings::TemperateGrassland,
                recipe: V3RecipeSettings::CrystalAscent(V3CrystalAscentSettings {
                    base_level: 6,
                    rise_levels: 144,
                }),
                overlays: Vec::new(),
                mask: PatchMaskSettings::WholeWorld,
                edges: world_edges(),
            }),
        };

        assert_eq!(ensure_recipe_available(&settings), Ok(()));
    }

    #[test]
    fn all_native_desert_recipes_are_reported_available() {
        let recipes = [
            V3RecipeSettings::DesertTransition(V3DesertTransitionSettings {
                base_level: 15,
                max_relief: 3,
                transition_width: 8,
                dry_coverage_percent: 55,
            }),
            V3RecipeSettings::DesertPlain(V3DesertPlainSettings {
                base_level: 15,
                max_relief: 2,
            }),
            V3RecipeSettings::Dunes(V3DunesSettings {
                base_level: 15,
                ridge_height: 6,
                ridge_spacing: 12,
                ridge_count: 5,
            }),
            V3RecipeSettings::Oasis(V3OasisSettings {
                base_level: 15,
                pool_radius: 5,
                palm_count: 12,
                grass_ring_width: 3,
            }),
        ];

        for recipe in recipes {
            let settings = ProceduralV3Settings {
                layout: V3LayoutSettings::Single(PatchSpec {
                    environment: V3EnvironmentSettings::Arid,
                    recipe,
                    overlays: Vec::new(),
                    mask: PatchMaskSettings::WholeWorld,
                    edges: world_edges(),
                }),
            };
            assert_eq!(ensure_recipe_available(&settings), Ok(()));
        }
    }

    #[test]
    fn both_native_coastal_island_recipes_are_reported_available() {
        let recipes = [
            V3RecipeSettings::SandyIslets(V3SandyIsletsSettings {
                sea_level: 8,
                land_coverage_percent: 28,
                islet_count: 5,
                max_relief: 3,
            }),
            V3RecipeSettings::WoodedIsland(V3WoodedIslandSettings {
                sea_level: 8,
                land_coverage_percent: 68,
                max_relief: 6,
                tree_coverage_percent: 26,
            }),
        ];

        for recipe in recipes {
            let settings = ProceduralV3Settings {
                layout: V3LayoutSettings::Single(PatchSpec {
                    environment: V3EnvironmentSettings::Coastal,
                    recipe,
                    overlays: Vec::new(),
                    mask: PatchMaskSettings::WholeWorld,
                    edges: world_edges(),
                }),
            };
            assert_eq!(ensure_recipe_available(&settings), Ok(()));
        }
    }

    #[test]
    fn forest_reports_only_recipe_appropriate_legacy_metrics() {
        let metrics = forest::ForestMetrics {
            tree_roots: 20,
            tree_blocker_surfaces: 32,
            old_growth_roots: 2,
            old_growth_blocker_surfaces: 14,
            tall_grass_roots: 40,
            woodland_surfaces: 120,
            prairie_surfaces: 180,
            clearing_count: 4,
            clearing_surfaces: 40,
            protected_route_surfaces: 30,
            ordinary_surfaces: 280,
            reachable_elevation_levels: 4,
            relief: 4,
            spawn_height_difference: 0,
            woodland_prairie_high_ground_difference: 1,
            critical_route_steps: 24,
        };

        let reported = forest_report_metrics(&metrics);
        let recipe = forest_recipe_metrics(&metrics);

        assert_eq!(reported.environment_signature_percent, 20);
        assert_eq!(reported.barrier_cells, 0);
        assert_eq!(reported.bank_high_ground_difference, 0);
        assert_eq!(recipe.critical_route_steps, 24);
        assert_eq!(recipe.spawn_height_difference, 0);
        assert_eq!(recipe.woodland_prairie_high_ground_difference, 1);
    }
}
