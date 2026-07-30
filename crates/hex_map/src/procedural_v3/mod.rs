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
    CavesMetrics as CavesReportMetrics, ForestMetrics as ForestReportMetrics,
    FortMetrics as FortReportMetrics, GenerationReport, HillsMetrics as HillsReportMetrics,
    MountainsMetrics as MountainsReportMetrics, PrairieMetrics as PrairieReportMetrics,
    ProceduralRecipeMetrics, TacticalMetrics, WaterfallMetrics as WaterfallReportMetrics,
};
use crate::settings::{ProceduralV3Settings, V3LayoutSettings, V3RecipeSettings};
use crate::terrain::TerrainPalette;
use crate::voxel::VoxelMap;
use materialize::{MaterializationError, MaterializedV3World};
use selection::{CandidateNote, ValidatedWorldSelection};
use world::WorldValidationIssue;

mod caves;
pub(crate) use caves::{CaveCrystalAssetError, CaveCrystalObjectSet};
mod composition;
#[cfg(test)]
mod dry_patch_tests;
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
pub(crate) use layout::HexSide;
#[expect(
    dead_code,
    reason = "liquid topology is consumed by the sequential V3 Waterfall recipe"
)]
mod liquid;
pub(crate) use liquid::LiquidFlowState;
mod materialize;
pub(crate) use materialize::MapPresentationProjection;
mod mountains;
mod patch;
mod prairie;
mod ring7;
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
#[expect(
    dead_code,
    reason = "the volume foundation is consumed by sequential V3 recipe implementations"
)]
mod volume;
pub(crate) use volume::FillMaterialRole;
mod waterfall;
mod world;
#[cfg(test)]
pub(crate) use world::PlannedFeature;
pub(crate) use world::{
    CaveCrystalKind, FeatureId, FeatureKind, LightId, PlannedLightPresentation,
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
                    | V3RecipeSettings::Prairie(_)
            ) =>
        {
            Ok(())
        }
        V3LayoutSettings::Single(patch) => Err(V3GenerationError::RecipeUnavailable(recipe_name(
            &patch.recipe,
        ))),
        V3LayoutSettings::Ring7(_) => Ok(()),
        V3LayoutSettings::Ring19(_) => Err(V3GenerationError::RecipeUnavailable("Ring19")),
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
            CaveCrystalObjectSet::resolve(art_catalog)
                .map_err(|error| V3GenerationError::RecipeContract(error.to_string()))?;
            finish_build(
                caves::generate(grid_radius, level_height, settings, seed)?,
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
        V3LayoutSettings::Ring19(_) => Err(V3GenerationError::RecipeUnavailable("Ring19")),
    }
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{
        PatchEdgeContractSettings, PatchEdgesSettings, PatchMaskSettings, PatchSpec,
        V3EnvironmentSettings, V3HillsSettings,
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
