//! Native V3 traversable dune-field geometry.
//!
//! A small set of parallel ridge polylines is authored in the resolved patch's
//! local frame. Candidate seed data moves shared interior control points sideways,
//! producing lateral warp without changing ridge spacing or rotation semantics.
//! Surface height is the exact integer hex distance to the nearest ridge, clamped
//! by the configured height. Distance to a set is one-Lipschitz, which keeps every
//! ordinary neighboring surface within one voxel level by construction.

use std::collections::{BTreeMap, BTreeSet};

use hex_core::{HexCoord, Level, TilePos};

use super::arid_landform::sand_column;
use super::composition::{compose_single_patch, GeneratedPatchPlan};
use super::layout::{resolve_layout, PatchId, ResolvedLayoutPlan};
use super::patch::{PatchBuildMode, PatchRecipeContext};
use super::seam::{shape_walker_seams, validate_patch_walker_seams};
use super::seed::SeedStream;
use super::selection::{
    run_recipe, CandidateAttemptError, CandidateContext, FallbackContext, RepairOutcome, V3Recipe,
    ValidatedWorldSelection, WorldValidation,
};
use super::traversal::OrdinaryGraph;
use super::vegetation_landform::view_hint;
use super::volume::{SolidMaterialRole, SurfaceAccess, SurfaceMetadata, VolumeElement, VolumePlan};
use super::world::{
    FeaturePlan, GeneratedWorldPlan, InteriorPlan, StructurePlan, WorldIssueCode,
    WorldValidationIssue,
};
use super::V3GenerationError;
use crate::procedural::DunesMetrics;
use crate::settings::{
    ProceduralV3Settings, V3DunesSettings, V3EnvironmentSettings, V3LayoutSettings,
    V3RecipeSettings, MAX_V3_LEVEL,
};

const PARTY_START: &str = "party_start";
const HOSTILE_START: &str = "hostile_start";
const DUNE_CREST: &str = "dune_crest";
const DUNE_TROUGH: &str = "dune_trough";
const MIN_GRID_RADIUS: u32 = 12;
const MAX_GRID_RADIUS: u32 = 55;
const CONTROL_POINT_COUNT: usize = 5;

#[derive(Debug)]
struct DunesRecipe {
    level_height: f32,
    layout: ResolvedLayoutPlan,
    settings: V3DunesSettings,
}

/// Runs the common eight-candidate V3 selector for one dune field.
pub(crate) fn generate(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV3Settings,
    seed: u64,
) -> Result<ValidatedWorldSelection<DunesMetrics>, V3GenerationError> {
    if !level_height.is_finite() || level_height <= 0.0 {
        return Err(V3GenerationError::RecipeContract(
            "Dunes level height must be positive and finite".to_owned(),
        ));
    }
    let recipe = recipe_settings(settings, grid_radius)?;
    let layout = resolve_layout(grid_radius, settings)
        .map_err(|error| V3GenerationError::RecipeContract(error.to_string()))?;
    run_recipe(
        &DunesRecipe {
            level_height,
            layout,
            settings: *recipe,
        },
        settings,
        grid_radius,
        seed,
    )
}

impl V3Recipe for DunesRecipe {
    type Settings = ProceduralV3Settings;
    type Metrics = DunesMetrics;
    type Score = (u32, u32, u8);

    fn construct(
        &self,
        context: CandidateContext,
        _settings: &Self::Settings,
    ) -> Result<GeneratedWorldPlan, CandidateAttemptError> {
        let patch = PatchRecipeContext::resolve(&self.layout, PatchId(0))
            .map_err(CandidateAttemptError::Fatal)?;
        let fragment = construct_patch(
            patch,
            &self.settings,
            V3EnvironmentSettings::Arid,
            self.level_height,
            PatchBuildMode::Candidate {
                world_seed: context.seed,
                candidate: context.candidate,
            },
        )
        .map_err(CandidateAttemptError::Rejected)?;
        compose_single_patch(self.layout.clone(), fragment).map_err(|error| {
            CandidateAttemptError::Fatal(V3GenerationError::RecipeContract(format!(
                "Dunes single-patch composition failed: {error:?}"
            )))
        })
    }

    fn validate(
        &self,
        _settings: &Self::Settings,
        plan: &GeneratedWorldPlan,
    ) -> WorldValidation<Self::Metrics> {
        validate_dunes(plan, &self.settings)
    }

    fn repair(
        &self,
        _context: CandidateContext,
        _settings: &Self::Settings,
        _plan: &mut GeneratedWorldPlan,
        _round: u8,
        _issues: &[WorldValidationIssue],
    ) -> Result<RepairOutcome, CandidateAttemptError> {
        Ok(RepairOutcome::NoChange)
    }

    fn score(
        &self,
        _settings: &Self::Settings,
        metrics: &Self::Metrics,
        candidate: u8,
    ) -> Self::Score {
        (
            metrics.relief.abs_diff(self.settings.ridge_height),
            u32::MAX.saturating_sub(metrics.trough_surfaces),
            candidate,
        )
    }

    fn canonical_fallback(
        &self,
        context: FallbackContext,
        _settings: &Self::Settings,
    ) -> Result<GeneratedWorldPlan, V3GenerationError> {
        if context.grid_radius != self.layout.grid_radius {
            return Err(V3GenerationError::RecipeContract(
                "Dunes fallback radius disagrees with its resolved layout".to_owned(),
            ));
        }
        let patch = PatchRecipeContext::resolve(&self.layout, PatchId(0))?;
        let fragment = construct_patch(
            patch,
            &self.settings,
            V3EnvironmentSettings::Arid,
            self.level_height,
            PatchBuildMode::CanonicalFallback,
        )
        .map_err(recipe_issues_to_error)?;
        compose_single_patch(self.layout.clone(), fragment).map_err(|error| {
            V3GenerationError::RecipeContract(format!(
                "Dunes fallback composition failed: {error:?}"
            ))
        })
    }
}

fn recipe_settings(
    settings: &ProceduralV3Settings,
    grid_radius: u32,
) -> Result<&V3DunesSettings, V3GenerationError> {
    let V3LayoutSettings::Single(patch) = &settings.layout else {
        return Err(V3GenerationError::RecipeUnavailable("Dunes"));
    };
    if patch.environment != V3EnvironmentSettings::Arid {
        return Err(V3GenerationError::RecipeContract(
            "Dunes requires the Arid environment".to_owned(),
        ));
    }
    let V3RecipeSettings::Dunes(recipe) = &patch.recipe else {
        return Err(V3GenerationError::RecipeUnavailable("Dunes"));
    };
    if !patch.overlays.is_empty() {
        return Err(V3GenerationError::RecipeContract(
            "Dunes overlays are not implemented yet".to_owned(),
        ));
    }
    validate_values(recipe, grid_radius)?;
    Ok(recipe)
}

fn validate_values(settings: &V3DunesSettings, grid_radius: u32) -> Result<(), V3GenerationError> {
    let highest = settings.base_level.checked_add(settings.ridge_height);
    if !(MIN_GRID_RADIUS..=MAX_GRID_RADIUS).contains(&grid_radius)
        || settings.base_level < 5
        || !(3..=8).contains(&settings.ridge_height)
        || !(8..=16).contains(&settings.ridge_spacing)
        || !(3..=7).contains(&settings.ridge_count)
        || highest.is_none_or(|level| level > MAX_V3_LEVEL)
    {
        return Err(V3GenerationError::RecipeContract(
            "Dunes settings violate the validated Arid ridge-field range".to_owned(),
        ));
    }
    Ok(())
}

/// Constructs one patch-ready dune fragment in its resolved local frame.
pub(crate) fn construct_patch(
    patch: PatchRecipeContext<'_>,
    settings: &V3DunesSettings,
    environment: V3EnvironmentSettings,
    level_height: f32,
    mode: PatchBuildMode,
) -> Result<GeneratedPatchPlan, Vec<WorldValidationIssue>> {
    if environment != V3EnvironmentSettings::Arid {
        return Err(vec![recipe_issue("Dunes requires the Arid environment")]);
    }
    if !level_height.is_finite() || level_height <= 0.0 {
        return Err(vec![recipe_issue(
            "Dunes level height must be positive and finite",
        )]);
    }
    let frame = patch
        .local_frame()
        .map_err(|error| vec![recipe_issue(error.to_string())])?;
    let mask = frame
        .local_mask(patch.mask())
        .map_err(|error| vec![recipe_issue(error)])?;
    let warp = mode
        .seed_streams(&patch)
        .map(|streams| streams.stage("dunes.lateral-warp"));
    let local_levels = ridge_levels(&mask, settings, warp)?;
    let mut world_levels = frame
        .levels_to_world(local_levels)
        .map_err(|error| vec![recipe_issue(error)])?;
    let seam_shape = shape_walker_seams(&patch, &mut world_levels)?;
    let local_levels = frame
        .levels_to_local(world_levels)
        .map_err(|error| vec![recipe_issue(error)])?;

    let mut surfaces = BTreeMap::new();
    let mut ordinary = BTreeSet::new();
    for (coord, level) in &local_levels {
        let position = TilePos::new(*coord, *level);
        let world_position = frame
            .position_to_world(position)
            .map_err(|error| vec![recipe_issue(error)])?;
        let access = seam_shape.access_for(world_position, SurfaceAccess::Ordinary);
        surfaces.insert(
            position,
            SurfaceMetadata {
                access,
                interior: None,
            },
        );
        if access == SurfaceAccess::Ordinary {
            ordinary.insert(position);
        }
    }
    let party_start = ordinary
        .iter()
        .copied()
        .min_by_key(|position| (position.coord.x(), position.coord.y().abs(), *position))
        .ok_or_else(|| vec![recipe_issue("Dunes has no ordinary party landing")])?;
    let hostile_start = ordinary
        .iter()
        .copied()
        .max_by_key(|position| (position.coord.x(), -position.coord.y().abs(), *position))
        .ok_or_else(|| vec![recipe_issue("Dunes has no ordinary hostile landing")])?;
    let dune_crest = ordinary
        .iter()
        .copied()
        .max_by_key(|position| {
            (
                position.level,
                std::cmp::Reverse(position.coord.distance(HexCoord::ORIGIN)),
                std::cmp::Reverse(*position),
            )
        })
        .ok_or_else(|| vec![recipe_issue("Dunes has no crest review surface")])?;
    let dune_trough = ordinary
        .iter()
        .copied()
        .min_by_key(|position| {
            (
                position.level,
                position.coord.distance(HexCoord::ORIGIN),
                *position,
            )
        })
        .ok_or_else(|| vec![recipe_issue("Dunes has no trough review surface")])?;

    let columns = local_levels
        .iter()
        .map(|(coord, level)| (*coord, sand_column(*level)))
        .collect();
    let volume = VolumePlan {
        mask,
        columns,
        surfaces,
    };
    let anchors = BTreeMap::from([
        (PARTY_START.to_owned(), party_start),
        (HOSTILE_START.to_owned(), hostile_start),
        (DUNE_CREST.to_owned(), dune_crest),
        (DUNE_TROUGH.to_owned(), dune_trough),
    ]);
    let biome_regions = volume
        .surfaces
        .keys()
        .copied()
        .map(|surface| (surface, patch.biome_region()))
        .collect();
    let mut plan = GeneratedPatchPlan {
        patch_id: patch.id,
        volume,
        liquids: Default::default(),
        features: FeaturePlan::default(),
        structures: StructurePlan::default(),
        blockers: BTreeSet::new(),
        lights: BTreeMap::new(),
        biome_regions,
        interiors: InteriorPlan::default(),
        anchors,
        view_hint: view_hint(
            frame.scale(),
            settings.base_level,
            settings.ridge_height,
            level_height,
            "dunes",
        )?,
    };
    frame
        .patch_to_world(&mut plan)
        .map_err(|error| vec![recipe_issue(error)])?;
    seam_shape.apply(&mut plan.volume)?;
    let seam_issues = validate_patch_walker_seams(&patch, &plan.volume);
    if seam_issues.is_empty() {
        Ok(plan)
    } else {
        Err(seam_issues)
    }
}

/// Validates one composed dune fragment through its canonical local frame.
pub(crate) fn validate_patch(
    patch: PatchRecipeContext<'_>,
    settings: &V3DunesSettings,
    plan: &GeneratedPatchPlan,
) -> WorldValidation<DunesMetrics> {
    let frame = match patch.local_frame() {
        Ok(frame) => frame,
        Err(error) => {
            return WorldValidation::Invalid(vec![recipe_issue(format!(
                "Dunes validation frame failed: {error}"
            ))]);
        }
    };
    let mut world = match frame.canonical_local_world(plan) {
        Ok(world) => world,
        Err(error) => {
            return WorldValidation::Invalid(vec![recipe_issue(format!(
                "Dunes validation projection failed: {error}"
            ))]);
        }
    };
    world.layout.grid_radius = world
        .layout
        .footprint
        .iter()
        .map(|coord| HexCoord::ORIGIN.distance(*coord))
        .max()
        .unwrap_or(MIN_GRID_RADIUS)
        .max(MIN_GRID_RADIUS);
    validate_dunes(&world, settings)
}

fn validate_dunes(
    plan: &GeneratedWorldPlan,
    settings: &V3DunesSettings,
) -> WorldValidation<DunesMetrics> {
    let mut issues = plan.validate();
    if !plan.liquids.bodies.is_empty()
        || !plan.features.by_id.is_empty()
        || !plan.features.protected_routes.is_empty()
        || !plan.features.clearings.is_empty()
        || !plan.structures.by_id.is_empty()
        || !plan.blockers.is_empty()
        || !plan.lights.is_empty()
        || !plan.interiors.by_id.is_empty()
    {
        issues.push(recipe_issue(
            "Dunes must not contain features, liquids, structures, blockers, lights, or interiors",
        ));
    }
    let ordinary = OrdinaryGraph::from_volume(&plan.volume, None);
    let party = plan.anchors.get(PARTY_START).copied();
    let hostile = plan.anchors.get(HOSTILE_START).copied();
    let crest_anchor = plan.anchors.get(DUNE_CREST).copied();
    let trough_anchor = plan.anchors.get(DUNE_TROUGH).copied();
    let mut critical_route_steps = 0;
    match (party, hostile) {
        (Some(party), Some(hostile)) => {
            let distances = ordinary.distances_from(party);
            if distances.len() != ordinary.len() {
                issues.push(recipe_issue(format!(
                    "Dunes ordinary terrain is disconnected: {}/{} reachable",
                    distances.len(),
                    ordinary.len()
                )));
            }
            critical_route_steps = distances.get(&hostile).copied().unwrap_or_default();
            if !distances.contains_key(&hostile) {
                issues.push(recipe_issue(
                    "Dunes actor anchors are not ordinarily connected",
                ));
            }
        }
        _ => issues.push(recipe_issue(
            "Dunes requires party_start and hostile_start anchors",
        )),
    }
    let by_coord = ordinary
        .positions()
        .map(|position| (position.coord, position))
        .collect::<BTreeMap<_, _>>();
    for position in by_coord.values().copied() {
        if surface_material(&plan.volume, position) != Some(SolidMaterialRole::Sand) {
            issues.push(recipe_issue(format!(
                "Dunes surface {position:?} is not topped by exact sand"
            )));
        }
        for neighbor in position.coord.neighbors() {
            if let Some(other) = by_coord.get(&neighbor).copied() {
                if position.level.abs_diff(other.level) > 1 {
                    issues.push(recipe_issue(format!(
                        "Dunes neighbor delta exceeds one between {position:?} and {other:?}"
                    )));
                }
            }
        }
    }
    let levels = ordinary
        .positions()
        .map(|position| position.level)
        .collect::<BTreeSet<_>>();
    let lowest = levels.first().copied().unwrap_or(settings.base_level);
    let highest = levels.last().copied().unwrap_or(settings.base_level);
    let relief = highest.saturating_sub(lowest);
    if relief != settings.ridge_height {
        issues.push(recipe_issue(format!(
            "Dunes relief must be {}, got {relief}",
            settings.ridge_height
        )));
    }
    let crest_surfaces = ordinary
        .positions()
        .filter(|position| position.level == highest)
        .collect::<BTreeSet<_>>();
    let trough_surfaces = ordinary
        .positions()
        .filter(|position| position.level == lowest)
        .collect::<BTreeSet<_>>();
    if crest_surfaces.is_empty() || trough_surfaces.is_empty() {
        issues.push(recipe_issue(
            "Dunes requires at least one exact crest and one exact trough surface",
        ));
    }
    if crest_anchor.is_none_or(|anchor| !crest_surfaces.contains(&anchor)) {
        issues.push(recipe_issue(
            "Dunes dune_crest anchor must lie on an exact highest surface",
        ));
    }
    if trough_anchor.is_none_or(|anchor| !trough_surfaces.contains(&anchor)) {
        issues.push(recipe_issue(
            "Dunes dune_trough anchor must lie on an exact lowest surface",
        ));
    }
    if !issues.is_empty() {
        return WorldValidation::Invalid(issues);
    }
    WorldValidation::Valid(DunesMetrics {
        ordinary_surfaces: count_u32(ordinary.len()),
        crest_surfaces: count_u32(crest_surfaces.len()),
        trough_surfaces: count_u32(trough_surfaces.len()),
        ridge_count: settings.ridge_count,
        ridge_height: settings.ridge_height,
        reachable_elevation_levels: count_u32(levels.len()),
        relief,
        critical_route_steps,
    })
}

fn ridge_levels(
    mask: &BTreeSet<HexCoord>,
    settings: &V3DunesSettings,
    stream: Option<SeedStream<'_>>,
) -> Result<BTreeMap<HexCoord, Level>, Vec<WorldValidationIssue>> {
    let ridges = authored_ridges(mask, settings, stream)?;
    let ridge_cells = ridges
        .iter()
        .flat_map(|ridge| ridge.iter().copied())
        .collect::<BTreeSet<_>>();
    if ridge_cells.is_empty() {
        return Err(vec![recipe_issue("Dunes authored no ridge cells")]);
    }
    let levels = mask
        .iter()
        .copied()
        .map(|coord| {
            let distance = ridge_cells
                .iter()
                .map(|ridge| coord.distance(*ridge))
                .min()
                .unwrap_or(u32::MAX);
            let distance = i32::try_from(distance).unwrap_or(i32::MAX);
            let rise = settings.ridge_height.saturating_sub(distance).max(0);
            (coord, settings.base_level.saturating_add(rise))
        })
        .collect::<BTreeMap<_, _>>();
    validate_one_lipschitz(&levels)?;
    let lowest = levels
        .values()
        .copied()
        .min()
        .unwrap_or(settings.base_level);
    let highest = levels
        .values()
        .copied()
        .max()
        .unwrap_or(settings.base_level);
    if highest.saturating_sub(lowest) != settings.ridge_height {
        return Err(vec![recipe_issue(format!(
            "Dunes mask does not expose the requested ridge-to-trough relief {}",
            settings.ridge_height
        ))]);
    }
    Ok(levels)
}

fn authored_ridges(
    mask: &BTreeSet<HexCoord>,
    settings: &V3DunesSettings,
    stream: Option<SeedStream<'_>>,
) -> Result<Vec<BTreeSet<HexCoord>>, Vec<WorldValidationIssue>> {
    let min_y = mask.iter().map(|coord| coord.y()).min().ok_or_else(|| {
        vec![recipe_issue(
            "Dunes cannot author ridges for an empty local mask",
        )]
    })?;
    let max_y = mask.iter().map(|coord| coord.y()).max().unwrap_or(min_y);
    let inset = settings.ridge_height.max(1);
    let mut start_y = min_y.saturating_add(inset);
    let mut end_y = max_y.saturating_sub(inset);
    if start_y > end_y {
        let midpoint = min_y.saturating_add(max_y.saturating_sub(min_y) / 2);
        start_y = midpoint;
        end_y = midpoint;
    }
    let span = end_y.saturating_sub(start_y);
    let control_y = std::array::from_fn::<_, CONTROL_POINT_COUNT, _>(|index| {
        let numerator = i32::try_from(index).unwrap_or_default();
        let denominator = i32::try_from(CONTROL_POINT_COUNT.saturating_sub(1)).unwrap_or(1);
        start_y.saturating_add(span.saturating_mul(numerator) / denominator)
    });
    let warp_limit = i32::from(settings.ridge_spacing / 4).clamp(1, 2);
    let warp = std::array::from_fn::<_, CONTROL_POINT_COUNT, _>(|index| {
        if index == 0 || index + 1 == CONTROL_POINT_COUNT {
            0
        } else {
            stream.map_or_else(
                || match index {
                    1 => 1,
                    2 => -1,
                    _ => 0,
                },
                |stream| {
                    stream
                        .range_i32(
                            u64::try_from(index).unwrap_or_default(),
                            -warp_limit,
                            warp_limit,
                        )
                        .unwrap_or_default()
                },
            )
        }
    });
    let control_points = control_y
        .into_iter()
        .zip(warp)
        .map(|(y, x)| HexCoord::from_axial(x, y))
        .collect::<Vec<_>>();
    let base_line = control_points
        .windows(2)
        .flat_map(|pair| {
            pair.first()
                .zip(pair.get(1))
                .map_or_else(Vec::new, |(start, end)| start.line_between(*end))
        })
        .collect::<BTreeSet<_>>();
    if base_line.is_empty() {
        return Err(vec![recipe_issue("Dunes base ridge line is empty")]);
    }
    let count = i32::from(settings.ridge_count);
    let spacing = i32::from(settings.ridge_spacing);
    let first_offset = (count.saturating_sub(1))
        .saturating_mul(spacing)
        .saturating_neg()
        / 2;
    let ridges = (0..count)
        .map(|index| {
            let offset = first_offset.saturating_add(index.saturating_mul(spacing));
            base_line
                .iter()
                .map(|coord| HexCoord::from_axial(coord.x().saturating_add(offset), coord.y()))
                .collect::<BTreeSet<_>>()
        })
        .collect::<Vec<_>>();
    if ridges.len() != usize::from(settings.ridge_count) || ridges.iter().any(BTreeSet::is_empty) {
        return Err(vec![recipe_issue(
            "Dunes did not author the exact configured ridge count",
        )]);
    }
    Ok(ridges)
}

fn validate_one_lipschitz(
    levels: &BTreeMap<HexCoord, Level>,
) -> Result<(), Vec<WorldValidationIssue>> {
    for (coord, level) in levels {
        for neighbor in coord.neighbors() {
            if let Some(other) = levels.get(&neighbor) {
                if level.abs_diff(*other) > 1 {
                    return Err(vec![recipe_issue(format!(
                        "Dunes exact ridge distance is not one-Lipschitz between {coord:?} and {neighbor:?}"
                    ))]);
                }
            }
        }
    }
    Ok(())
}

fn surface_material(volume: &VolumePlan, position: TilePos) -> Option<SolidMaterialRole> {
    volume.columns.get(&position.coord).and_then(|column| {
        column.elements.iter().find_map(|element| {
            let VolumeElement::Solid(mass) = *element else {
                return None;
            };
            (mass.levels.bottom <= position.level && position.level < mass.levels.top)
                .then_some(mass.material)
        })
    })
}

fn recipe_issues_to_error(issues: Vec<WorldValidationIssue>) -> V3GenerationError {
    V3GenerationError::RecipeContract(
        issues
            .into_iter()
            .map(|issue| issue.detail)
            .collect::<Vec<_>>()
            .join("; "),
    )
}

fn count_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn recipe_issue(detail: impl Into<String>) -> WorldValidationIssue {
    WorldValidationIssue::new(WorldIssueCode::Recipe("dunes"), detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{
        PatchEdgeContractSettings, PatchEdgesSettings, PatchMaskSettings, PatchSpec,
    };

    fn settings() -> ProceduralV3Settings {
        ProceduralV3Settings {
            layout: V3LayoutSettings::Single(PatchSpec {
                environment: V3EnvironmentSettings::Arid,
                recipe: V3RecipeSettings::Dunes(V3DunesSettings {
                    base_level: 15,
                    ridge_height: 4,
                    ridge_spacing: 10,
                    ridge_count: 3,
                }),
                overlays: Vec::new(),
                mask: PatchMaskSettings::WholeWorld,
                edges: world_boundaries(),
            }),
        }
    }

    fn world_boundaries() -> PatchEdgesSettings {
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
    fn fixed_corpus_builds_deterministic_traversable_dunes() {
        for radius in [12, 20, 40] {
            for seed in [0, 1, 1_592_598_566, 4_294_967_311] {
                let first =
                    generate(radius, 0.4, &settings(), seed).expect("Dunes should generate");
                let repeated =
                    generate(radius, 0.4, &settings(), seed).expect("Dunes should repeat");
                assert_eq!(
                    first.validated.semantic_fingerprint,
                    repeated.validated.semantic_fingerprint
                );
                assert!(!first.used_fallback);
                assert_eq!(first.candidates_evaluated, 8);
                assert_eq!(first.valid_candidates, 8);
                assert_eq!(first.metrics.ridge_count, 3);
                assert_eq!(first.metrics.ridge_height, 4);
                assert_eq!(first.metrics.relief, 4);
                assert!(first.metrics.crest_surfaces > 0);
                assert!(first.metrics.trough_surfaces > 0);
            }
        }
    }

    #[test]
    fn exact_ridge_distance_is_one_lipschitz_for_seeded_warp() {
        let mask = HexCoord::ORIGIN
            .within_radius(20)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let recipe = V3DunesSettings {
            base_level: 15,
            ridge_height: 6,
            ridge_spacing: 12,
            ridge_count: 5,
        };
        let stream = super::super::seed::SeedStreams::new(91, 3, 0).stage("dunes.lateral-warp");
        let levels = ridge_levels(&mask, &recipe, Some(stream)).expect("ridges should resolve");
        validate_one_lipschitz(&levels).expect("distance field must be one-Lipschitz");
        let lowest = levels.values().copied().min().expect("levels exist");
        let highest = levels.values().copied().max().expect("levels exist");
        assert_eq!(highest - lowest, recipe.ridge_height);
    }

    #[test]
    fn seeded_warp_changes_shape_without_changing_parallel_ridge_count() {
        let mask = HexCoord::ORIGIN
            .within_radius(20)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let recipe = V3DunesSettings {
            base_level: 15,
            ridge_height: 4,
            ridge_spacing: 10,
            ridge_count: 5,
        };
        let first = super::super::seed::SeedStreams::new(11, 0, 0).stage("dunes.lateral-warp");
        let second = super::super::seed::SeedStreams::new(12, 0, 0).stage("dunes.lateral-warp");
        let first_ridges =
            authored_ridges(&mask, &recipe, Some(first)).expect("first ridges should resolve");
        let repeated = authored_ridges(&mask, &recipe, Some(first)).expect("ridges should repeat");
        let second_ridges =
            authored_ridges(&mask, &recipe, Some(second)).expect("second ridges should resolve");
        assert_eq!(first_ridges, repeated);
        assert_ne!(first_ridges, second_ridges);
        assert_eq!(first_ridges.len(), usize::from(recipe.ridge_count));
        assert_eq!(second_ridges.len(), usize::from(recipe.ridge_count));
    }

    #[test]
    fn local_dune_field_round_trips_through_all_six_rotations() {
        let mask = HexCoord::ORIGIN
            .within_radius(12)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let recipe = V3DunesSettings {
            base_level: 15,
            ridge_height: 4,
            ridge_spacing: 10,
            ridge_count: 3,
        };
        let levels = ridge_levels(&mask, &recipe, None).expect("fallback ridges should resolve");
        for turns in 0..6 {
            let frame = super::super::local_frame::LocalPatchFrame::from_resolved_ring19(
                HexCoord::ORIGIN,
                12,
                turns,
            );
            let world = levels
                .iter()
                .map(|(coord, level)| {
                    (frame.to_world(*coord).expect("rotation should fit"), *level)
                })
                .collect::<BTreeMap<_, _>>();
            let normalized = world
                .into_iter()
                .map(|(coord, level)| {
                    (
                        frame.to_local(coord).expect("inverse rotation should fit"),
                        level,
                    )
                })
                .collect::<BTreeMap<_, _>>();
            assert_eq!(normalized, levels);
        }
    }

    #[test]
    fn dune_column_has_exact_sand_surface_and_valid_strata() {
        let position = TilePos::new(HexCoord::ORIGIN, 15);
        let column = sand_column(15);
        let strata = column
            .elements
            .iter()
            .map(|element| match *element {
                VolumeElement::Solid(mass) => (mass.levels.bottom, mass.levels.top, mass.material),
                VolumeElement::Fill(_) => panic!("a dry dune column cannot contain fill"),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            strata,
            vec![
                (0, 1, SolidMaterialRole::Bedrock),
                (1, 12, SolidMaterialRole::Stone),
                (12, 15, SolidMaterialRole::Dirt),
                (15, 16, SolidMaterialRole::Sand),
            ],
            "Dunes must retain the shared exact three-level arid substrate"
        );
        let volume = VolumePlan {
            mask: BTreeSet::from([HexCoord::ORIGIN]),
            columns: BTreeMap::from([(HexCoord::ORIGIN, column)]),
            surfaces: BTreeMap::from([(
                position,
                SurfaceMetadata {
                    access: SurfaceAccess::Ordinary,
                    interior: None,
                },
            )]),
        };
        assert_eq!(
            surface_material(&volume, position),
            Some(SolidMaterialRole::Sand)
        );
        assert!(volume.validate().is_ok());
    }
}
