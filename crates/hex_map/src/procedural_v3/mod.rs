//! Procedural world generation V3 foundation.
//!
//! V3 remains isolated from the temporary V1/V2 implementations. Settings dispatch
//! may select it before every recipe is available, but unsupported recipes fail
//! setup explicitly rather than publishing an empty or partially validated world.

use std::fmt;
use std::time::Instant;

use hex_core::{
    BiomeRegions, InteriorRegions, MapAnchors, MapViewHint, SpecialMovementRegions, SubstanceId,
    TraversalBlockers,
};

use crate::procedural::{
    GenerationReport, ProceduralRecipeMetrics, TacticalMetrics,
    WaterfallMetrics as WaterfallReportMetrics,
};
use crate::settings::{ProceduralV3Settings, V3LayoutSettings, V3RecipeSettings};
use crate::terrain::TerrainPalette;
use crate::voxel::VoxelMap;
use materialize::{MaterializationError, MaterializedV3World};
use selection::{CandidateNote, ValidatedWorldSelection};
use world::WorldValidationIssue;

mod fingerprint;
#[expect(
    dead_code,
    reason = "resolved layouts are consumed by sequential V3 recipe implementations"
)]
mod layout;
pub(crate) use layout::HexSide;
#[expect(
    dead_code,
    reason = "liquid topology is consumed by the sequential V3 Waterfall recipe"
)]
mod liquid;
pub(crate) use liquid::LiquidFlowState;
mod materialize;
pub(crate) use materialize::MapPresentationProjection;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the seed API is consumed by sequential V3 recipe implementations"
    )
)]
mod seed;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the whole-world runner is consumed by sequential V3 recipes"
    )
)]
mod selection;
#[expect(
    dead_code,
    reason = "the volume foundation is consumed by sequential V3 recipe implementations"
)]
mod volume;
pub(crate) use volume::FillMaterialRole;
mod waterfall;
#[expect(
    dead_code,
    reason = "the complete semantic plan is consumed by the V3 selection runner"
)]
mod world;

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
            if matches!(patch.recipe, V3RecipeSettings::Waterfall(_)) =>
        {
            Ok(())
        }
        V3LayoutSettings::Single(patch) => Err(V3GenerationError::RecipeUnavailable(recipe_name(
            &patch.recipe,
        ))),
        V3LayoutSettings::Ring7(_) => Err(V3GenerationError::RecipeUnavailable("Ring7")),
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
) -> Result<ProceduralBuild, V3GenerationError> {
    let started = Instant::now();
    let selection = match &settings.layout {
        V3LayoutSettings::Single(patch)
            if matches!(patch.recipe, V3RecipeSettings::Waterfall(_)) =>
        {
            waterfall::generate(grid_radius, level_height, settings, seed)?
        }
        V3LayoutSettings::Single(patch) => {
            return Err(V3GenerationError::RecipeUnavailable(recipe_name(
                &patch.recipe,
            )));
        }
        V3LayoutSettings::Ring7(_) => {
            return Err(V3GenerationError::RecipeUnavailable("Ring7"));
        }
    };
    finish_waterfall_build(
        selection,
        grid_radius,
        level_height,
        settings,
        seed,
        palette,
        is_solid,
        started,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "the V3 report boundary explicitly records every generation input"
)]
fn finish_waterfall_build(
    selection: ValidatedWorldSelection<waterfall::WaterfallMetrics>,
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV3Settings,
    seed: u64,
    palette: &TerrainPalette,
    is_solid: &dyn Fn(SubstanceId) -> bool,
    started: Instant,
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
        metrics: waterfall_report_metrics(&metrics),
        recipe_metrics: Some(ProceduralRecipeMetrics::Waterfall(
            waterfall_recipe_metrics(&metrics),
        )),
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
    fn unfinished_recipe_fails_instead_of_fabricating_a_plan() {
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

        assert_eq!(
            ensure_recipe_available(&settings),
            Err(V3GenerationError::RecipeUnavailable("Hills"))
        );
    }
}
