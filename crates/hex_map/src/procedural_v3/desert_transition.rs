//! Native V3 grass-to-desert transition geometry.
//!
//! Material bands are selected from complete local-axis slices rather than from
//! world coordinates or per-cell noise. That keeps the grass, dirt ecotone, and
//! sand regions connected and makes an authored patch rotation rotate the whole
//! transition instead of changing its semantic coverage.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use hex_core::{HexCoord, Level, TilePos};

use super::arid_landform::arid_column;
use super::composition::{compose_single_patch, GeneratedPatchPlan};
use super::layout::{resolve_layout, PatchId, ResolvedLayoutPlan};
use super::patch::{PatchBuildMode, PatchRecipeContext};
use super::seam::{shape_walker_seams, validate_patch_walker_seams};
use super::selection::{
    run_recipe, CandidateAttemptError, CandidateContext, FallbackContext, RepairOutcome, V3Recipe,
    ValidatedWorldSelection, WorldValidation,
};
use super::traversal::OrdinaryGraph;
use super::vegetation_landform::{rolling_levels, view_hint};
use super::volume::{
    SolidMaterialRole, SurfaceAccess, SurfaceMetadata, VolumeColumn, VolumeElement, VolumePlan,
};
use super::world::{
    FeaturePlan, GeneratedWorldPlan, InteriorPlan, StructurePlan, WorldIssueCode,
    WorldValidationIssue,
};
use super::V3GenerationError;
use crate::procedural::DesertTransitionMetrics;
use crate::settings::{
    ProceduralV3Settings, V3DesertTransitionSettings, V3EnvironmentSettings, V3LayoutSettings,
    V3RecipeSettings, MAX_V3_LEVEL,
};

const PARTY_START: &str = "party_start";
const HOSTILE_START: &str = "hostile_start";
const TRANSITION_CENTER: &str = "transition_center";
const GRASS_OVERLOOK: &str = "grass_overlook";
const SAND_OVERLOOK: &str = "sand_overlook";
const MIN_GRID_RADIUS: u32 = 12;
const MAX_GRID_RADIUS: u32 = 55;

#[derive(Debug)]
struct DesertTransitionRecipe {
    level_height: f32,
    layout: ResolvedLayoutPlan,
    settings: V3DesertTransitionSettings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum TransitionBand {
    Grass,
    Transition,
    Sand,
}

#[derive(Debug, Clone)]
struct TransitionBands {
    by_coord: BTreeMap<HexCoord, TransitionBand>,
    dry_coverage_percent: u32,
}

/// Runs the common eight-candidate V3 selector for one transition world.
pub(crate) fn generate(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV3Settings,
    seed: u64,
) -> Result<ValidatedWorldSelection<DesertTransitionMetrics>, V3GenerationError> {
    if !level_height.is_finite() || level_height <= 0.0 {
        return Err(V3GenerationError::RecipeContract(
            "DesertTransition level height must be positive and finite".to_owned(),
        ));
    }
    let recipe = recipe_settings(settings, grid_radius)?;
    let layout = resolve_layout(grid_radius, settings)
        .map_err(|error| V3GenerationError::RecipeContract(error.to_string()))?;
    run_recipe(
        &DesertTransitionRecipe {
            level_height,
            layout,
            settings: *recipe,
        },
        settings,
        grid_radius,
        seed,
    )
}

impl V3Recipe for DesertTransitionRecipe {
    type Settings = ProceduralV3Settings;
    type Metrics = DesertTransitionMetrics;
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
                "DesertTransition single-patch composition failed: {error:?}"
            )))
        })
    }

    fn validate(
        &self,
        _settings: &Self::Settings,
        plan: &GeneratedWorldPlan,
    ) -> WorldValidation<Self::Metrics> {
        validate_transition(plan, &self.settings)
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
            metrics
                .dry_coverage_percent
                .abs_diff(u32::from(self.settings.dry_coverage_percent)),
            metrics.relief.abs_diff(self.settings.max_relief),
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
                "DesertTransition fallback radius disagrees with its resolved layout".to_owned(),
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
                "DesertTransition fallback composition failed: {error:?}"
            ))
        })
    }
}

fn recipe_settings(
    settings: &ProceduralV3Settings,
    grid_radius: u32,
) -> Result<&V3DesertTransitionSettings, V3GenerationError> {
    let V3LayoutSettings::Single(patch) = &settings.layout else {
        return Err(V3GenerationError::RecipeUnavailable("DesertTransition"));
    };
    if patch.environment != V3EnvironmentSettings::Arid {
        return Err(V3GenerationError::RecipeContract(
            "DesertTransition requires the Arid environment".to_owned(),
        ));
    }
    let V3RecipeSettings::DesertTransition(recipe) = &patch.recipe else {
        return Err(V3GenerationError::RecipeUnavailable("DesertTransition"));
    };
    if !patch.overlays.is_empty() {
        return Err(V3GenerationError::RecipeContract(
            "DesertTransition overlays are not implemented yet".to_owned(),
        ));
    }
    validate_values(recipe, grid_radius)?;
    Ok(recipe)
}

fn validate_values(
    settings: &V3DesertTransitionSettings,
    grid_radius: u32,
) -> Result<(), V3GenerationError> {
    let highest = settings.base_level.checked_add(settings.max_relief);
    let footprint_width = grid_radius.saturating_mul(2).saturating_add(1);
    if !(MIN_GRID_RADIUS..=MAX_GRID_RADIUS).contains(&grid_radius)
        || settings.base_level < 5
        || !(1..=4).contains(&settings.max_relief)
        || !(5..=12).contains(&settings.transition_width)
        || !(40..=70).contains(&settings.dry_coverage_percent)
        || u32::from(settings.transition_width).saturating_add(6) > footprint_width
        || highest.is_none_or(|level| level > MAX_V3_LEVEL)
    {
        return Err(V3GenerationError::RecipeContract(
            "DesertTransition settings violate the validated Arid landform range".to_owned(),
        ));
    }
    Ok(())
}

/// Constructs one patch-ready transition fragment in its resolved local frame.
pub(crate) fn construct_patch(
    patch: PatchRecipeContext<'_>,
    settings: &V3DesertTransitionSettings,
    environment: V3EnvironmentSettings,
    level_height: f32,
    mode: PatchBuildMode,
) -> Result<GeneratedPatchPlan, Vec<WorldValidationIssue>> {
    if environment != V3EnvironmentSettings::Arid {
        return Err(vec![recipe_issue(
            "DesertTransition requires the Arid environment",
        )]);
    }
    if !level_height.is_finite() || level_height <= 0.0 {
        return Err(vec![recipe_issue(
            "DesertTransition level height must be positive and finite",
        )]);
    }
    let frame = patch
        .local_frame()
        .map_err(|error| vec![recipe_issue(error.to_string())])?;
    let mask = frame
        .local_mask(patch.mask())
        .map_err(|error| vec![recipe_issue(error)])?;
    let bands = assign_bands(&mask, settings)?;
    let landform = mode
        .seed_streams(&patch)
        .map(|streams| streams.stage("desert-transition.landform"));
    let local_levels = rolling_levels(
        &mask,
        settings.base_level,
        settings.max_relief,
        landform,
        "desert-transition",
    )?;
    let mut world_levels = frame
        .levels_to_world(local_levels)
        .map_err(|error| vec![recipe_issue(error)])?;
    let seam_shape = shape_walker_seams(&patch, &mut world_levels)?;
    let local_levels = frame
        .levels_to_local(world_levels)
        .map_err(|error| vec![recipe_issue(error)])?;

    let mut surfaces = BTreeMap::new();
    let mut positions_by_band = BTreeMap::<TransitionBand, BTreeSet<TilePos>>::new();
    for coord in &mask {
        let level = local_levels.get(coord).copied().ok_or_else(|| {
            vec![recipe_issue(format!(
                "DesertTransition land plan omitted coordinate {coord:?}"
            ))]
        })?;
        let band = bands.by_coord.get(coord).copied().ok_or_else(|| {
            vec![recipe_issue(format!(
                "DesertTransition material plan omitted coordinate {coord:?}"
            ))]
        })?;
        let position = TilePos::new(*coord, level);
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
            positions_by_band.entry(band).or_default().insert(position);
        }
    }

    let grass = positions_by_band
        .get(&TransitionBand::Grass)
        .ok_or_else(|| vec![recipe_issue("DesertTransition has no ordinary grass band")])?;
    let transition = positions_by_band
        .get(&TransitionBand::Transition)
        .ok_or_else(|| vec![recipe_issue("DesertTransition has no ordinary dirt band")])?;
    let sand = positions_by_band
        .get(&TransitionBand::Sand)
        .ok_or_else(|| vec![recipe_issue("DesertTransition has no ordinary sand band")])?;
    let party_start = select_axis_anchor(grass, false)
        .ok_or_else(|| vec![recipe_issue("DesertTransition has no grass landing")])?;
    let hostile_start = select_axis_anchor(sand, true)
        .ok_or_else(|| vec![recipe_issue("DesertTransition has no sand landing")])?;
    let transition_center = select_center_anchor(transition).ok_or_else(|| {
        vec![recipe_issue(
            "DesertTransition has no ecotone review surface",
        )]
    })?;
    let grass_overlook = select_center_anchor(grass)
        .ok_or_else(|| vec![recipe_issue("DesertTransition has no grass review surface")])?;
    let sand_overlook = select_center_anchor(sand)
        .ok_or_else(|| vec![recipe_issue("DesertTransition has no sand review surface")])?;

    let columns = mask
        .iter()
        .map(|coord| {
            let level = local_levels
                .get(coord)
                .copied()
                .unwrap_or(settings.base_level);
            let band = bands
                .by_coord
                .get(coord)
                .copied()
                .unwrap_or(TransitionBand::Transition);
            (*coord, transition_column(level, band))
        })
        .collect();
    let volume = VolumePlan {
        mask,
        columns,
        surfaces,
    };
    let anchors = BTreeMap::from([
        (PARTY_START.to_owned(), party_start),
        (HOSTILE_START.to_owned(), hostile_start),
        (TRANSITION_CENTER.to_owned(), transition_center),
        (GRASS_OVERLOOK.to_owned(), grass_overlook),
        (SAND_OVERLOOK.to_owned(), sand_overlook),
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
            settings.max_relief,
            level_height,
            "desert-transition",
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

/// Validates one composed transition fragment through its canonical local frame.
pub(crate) fn validate_patch(
    patch: PatchRecipeContext<'_>,
    settings: &V3DesertTransitionSettings,
    plan: &GeneratedPatchPlan,
) -> WorldValidation<DesertTransitionMetrics> {
    let frame = match patch.local_frame() {
        Ok(frame) => frame,
        Err(error) => {
            return WorldValidation::Invalid(vec![recipe_issue(format!(
                "DesertTransition validation frame failed: {error}"
            ))]);
        }
    };
    let mut world = match frame.canonical_local_world(plan) {
        Ok(world) => world,
        Err(error) => {
            return WorldValidation::Invalid(vec![recipe_issue(format!(
                "DesertTransition validation projection failed: {error}"
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
    validate_transition(&world, settings)
}

fn validate_transition(
    plan: &GeneratedWorldPlan,
    settings: &V3DesertTransitionSettings,
) -> WorldValidation<DesertTransitionMetrics> {
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
            "DesertTransition must not contain features, liquids, structures, blockers, lights, or interiors",
        ));
    }

    let ordinary = OrdinaryGraph::from_volume(&plan.volume, None);
    let party = plan.anchors.get(PARTY_START).copied();
    let hostile = plan.anchors.get(HOSTILE_START).copied();
    let transition_center = plan.anchors.get(TRANSITION_CENTER).copied();
    let grass_overlook = plan.anchors.get(GRASS_OVERLOOK).copied();
    let sand_overlook = plan.anchors.get(SAND_OVERLOOK).copied();
    let mut critical_route_steps = 0;
    match (party, hostile) {
        (Some(party), Some(hostile)) => {
            let distances = ordinary.distances_from(party);
            if distances.len() != ordinary.len() {
                issues.push(recipe_issue(format!(
                    "DesertTransition ordinary terrain is disconnected: {}/{} reachable",
                    distances.len(),
                    ordinary.len()
                )));
            }
            critical_route_steps = distances.get(&hostile).copied().unwrap_or_default();
            if !distances.contains_key(&hostile) {
                issues.push(recipe_issue(
                    "DesertTransition actor anchors are not ordinarily connected",
                ));
            }
        }
        _ => issues.push(recipe_issue(
            "DesertTransition requires party_start and hostile_start anchors",
        )),
    }

    let mut grass = BTreeSet::new();
    let mut transition = BTreeSet::new();
    let mut sand = BTreeSet::new();
    for position in ordinary.positions() {
        match surface_material(&plan.volume, position) {
            Some(SolidMaterialRole::Grass) => {
                grass.insert(position);
            }
            Some(SolidMaterialRole::Dirt) => {
                transition.insert(position);
            }
            Some(SolidMaterialRole::Sand) => {
                sand.insert(position);
            }
            actual => issues.push(recipe_issue(format!(
                "DesertTransition ordinary surface {position:?} has invalid top material {actual:?}"
            ))),
        }
    }
    for (label, positions) in [
        ("grass", &grass),
        ("transition", &transition),
        ("sand", &sand),
    ] {
        let coords = positions
            .iter()
            .map(|position| position.coord)
            .collect::<BTreeSet<_>>();
        if !coords_are_connected(&coords) {
            issues.push(recipe_issue(format!(
                "DesertTransition {label} band is not one connected local region"
            )));
        }
    }
    if grass.iter().any(|surface| {
        surface
            .coord
            .neighbors()
            .into_iter()
            .any(|neighbor| sand.iter().any(|candidate| candidate.coord == neighbor))
    }) {
        issues.push(recipe_issue(
            "DesertTransition grass and sand bands touch without the dirt ecotone",
        ));
    }
    validate_anchor_material(party, &grass, "party_start", "grass", &mut issues);
    validate_anchor_material(hostile, &sand, "hostile_start", "sand", &mut issues);
    validate_anchor_material(
        transition_center,
        &transition,
        TRANSITION_CENTER,
        "transition",
        &mut issues,
    );
    validate_anchor_material(grass_overlook, &grass, GRASS_OVERLOOK, "grass", &mut issues);
    validate_anchor_material(sand_overlook, &sand, SAND_OVERLOOK, "sand", &mut issues);

    let dry_surfaces = transition.len().saturating_add(sand.len());
    let dry_coverage_percent = count_u32(dry_surfaces)
        .saturating_mul(100)
        .checked_div(count_u32(ordinary.len()))
        .unwrap_or_default();
    if dry_coverage_percent.abs_diff(u32::from(settings.dry_coverage_percent)) > 4 {
        issues.push(recipe_issue(format!(
            "DesertTransition dry coverage {dry_coverage_percent}% differs from target {}% by more than four points",
            settings.dry_coverage_percent
        )));
    }
    let levels = ordinary
        .positions()
        .map(|position| position.level)
        .collect::<BTreeSet<_>>();
    let relief = levels
        .first()
        .zip(levels.last())
        .map_or(0, |(lowest, highest)| highest.saturating_sub(*lowest));
    if relief != settings.max_relief {
        issues.push(recipe_issue(format!(
            "DesertTransition relief must be {}, got {relief}",
            settings.max_relief
        )));
    }

    if !issues.is_empty() {
        return WorldValidation::Invalid(issues);
    }
    WorldValidation::Valid(DesertTransitionMetrics {
        ordinary_surfaces: count_u32(ordinary.len()),
        grass_surfaces: count_u32(grass.len()),
        transition_surfaces: count_u32(transition.len()),
        sand_surfaces: count_u32(sand.len()),
        dry_coverage_percent,
        reachable_elevation_levels: count_u32(levels.len()),
        relief,
        critical_route_steps,
    })
}

fn assign_bands(
    mask: &BTreeSet<HexCoord>,
    settings: &V3DesertTransitionSettings,
) -> Result<TransitionBands, Vec<WorldValidationIssue>> {
    let axis_values = mask
        .iter()
        .map(|coord| coord.x())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let width = usize::from(settings.transition_width);
    if axis_values.len() < width.saturating_add(2) {
        return Err(vec![recipe_issue(
            "DesertTransition local mask cannot fit all three material bands",
        )]);
    }
    let total = count_u32(mask.len());
    let start = (1..axis_values.len().saturating_sub(width))
        .min_by_key(|start| {
            let threshold = axis_values.get(*start).copied().unwrap_or_default();
            let dry = count_u32(mask.iter().filter(|coord| coord.x() >= threshold).count());
            let target = u64::from(total) * u64::from(settings.dry_coverage_percent);
            (u64::from(dry) * 100).abs_diff(target)
        })
        .ok_or_else(|| {
            vec![recipe_issue(
                "DesertTransition could not place its local-axis ecotone",
            )]
        })?;
    let transition_start = axis_values.get(start).copied().unwrap_or_default();
    let sand_start = axis_values
        .get(start.saturating_add(width))
        .copied()
        .unwrap_or(i32::MAX);
    let by_coord = mask
        .iter()
        .copied()
        .map(|coord| {
            let band = if coord.x() < transition_start {
                TransitionBand::Grass
            } else if coord.x() < sand_start {
                TransitionBand::Transition
            } else {
                TransitionBand::Sand
            };
            (coord, band)
        })
        .collect::<BTreeMap<_, _>>();
    let dry = by_coord
        .values()
        .filter(|band| **band != TransitionBand::Grass)
        .count();
    let dry_coverage_percent = count_u32(dry)
        .saturating_mul(100)
        .checked_div(total)
        .unwrap_or_default();
    let bands = TransitionBands {
        by_coord,
        dry_coverage_percent,
    };
    if bands
        .dry_coverage_percent
        .abs_diff(u32::from(settings.dry_coverage_percent))
        > 4
    {
        return Err(vec![recipe_issue(format!(
            "DesertTransition local-axis coverage {}% differs from target {}% by more than four points",
            bands.dry_coverage_percent, settings.dry_coverage_percent
        ))]);
    }
    for band in [
        TransitionBand::Grass,
        TransitionBand::Transition,
        TransitionBand::Sand,
    ] {
        let coords = bands
            .by_coord
            .iter()
            .filter_map(|(coord, actual)| (*actual == band).then_some(*coord))
            .collect::<BTreeSet<_>>();
        if !coords_are_connected(&coords) {
            return Err(vec![recipe_issue(format!(
                "DesertTransition local {band:?} band is disconnected"
            ))]);
        }
    }
    Ok(bands)
}

fn transition_column(surface: Level, band: TransitionBand) -> VolumeColumn {
    let cap = match band {
        TransitionBand::Grass => SolidMaterialRole::Grass,
        TransitionBand::Transition => SolidMaterialRole::Dirt,
        TransitionBand::Sand => SolidMaterialRole::Sand,
    };
    arid_column(surface, cap)
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

fn select_axis_anchor(surfaces: &BTreeSet<TilePos>, maximum: bool) -> Option<TilePos> {
    if maximum {
        surfaces
            .iter()
            .copied()
            .max_by_key(|position| (position.coord.x(), -position.coord.y().abs(), *position))
    } else {
        surfaces
            .iter()
            .copied()
            .min_by_key(|position| (position.coord.x(), position.coord.y().abs(), *position))
    }
}

fn select_center_anchor(surfaces: &BTreeSet<TilePos>) -> Option<TilePos> {
    surfaces.iter().copied().min_by_key(|position| {
        (
            position.coord.distance(HexCoord::ORIGIN),
            position.coord.y().abs(),
            *position,
        )
    })
}

fn validate_anchor_material(
    anchor: Option<TilePos>,
    expected: &BTreeSet<TilePos>,
    name: &str,
    material: &str,
    issues: &mut Vec<WorldValidationIssue>,
) {
    if anchor.is_none_or(|position| !expected.contains(&position)) {
        issues.push(recipe_issue(format!(
            "DesertTransition anchor {name} must lie on its {material} band"
        )));
    }
}

fn coords_are_connected(coords: &BTreeSet<HexCoord>) -> bool {
    let Some(start) = coords.first().copied() else {
        return false;
    };
    let mut visited = BTreeSet::from([start]);
    let mut frontier = VecDeque::from([start]);
    while let Some(coord) = frontier.pop_front() {
        for neighbor in coord.neighbors() {
            if coords.contains(&neighbor) && visited.insert(neighbor) {
                frontier.push_back(neighbor);
            }
        }
    }
    visited.len() == coords.len()
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
    WorldValidationIssue::new(WorldIssueCode::Recipe("desert-transition"), detail)
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
                recipe: V3RecipeSettings::DesertTransition(V3DesertTransitionSettings {
                    base_level: 15,
                    max_relief: 3,
                    transition_width: 7,
                    dry_coverage_percent: 55,
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
    fn fixed_corpus_builds_deterministic_connected_transitions() {
        for radius in [12, 20, 40] {
            for seed in [0, 1, 1_592_598_566, 4_294_967_311] {
                let first = generate(radius, 0.4, &settings(), seed)
                    .expect("DesertTransition should generate");
                let repeated = generate(radius, 0.4, &settings(), seed)
                    .expect("DesertTransition should repeat");
                assert_eq!(
                    first.validated.semantic_fingerprint,
                    repeated.validated.semantic_fingerprint
                );
                assert!(!first.used_fallback);
                assert_eq!(first.candidates_evaluated, 8);
                assert_eq!(first.valid_candidates, 8);
                assert_eq!(first.metrics.relief, 3);
                assert!(first.metrics.dry_coverage_percent.abs_diff(55) <= 4);
                assert!(first.metrics.grass_surfaces > 0);
                assert!(first.metrics.transition_surfaces > 0);
                assert!(first.metrics.sand_surfaces > 0);
            }
        }
    }

    #[test]
    fn local_band_quota_is_connected_and_rotation_invariant() {
        let mask = HexCoord::ORIGIN
            .within_radius(12)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let recipe = V3DesertTransitionSettings {
            base_level: 15,
            max_relief: 3,
            transition_width: 7,
            dry_coverage_percent: 55,
        };
        let baseline = assign_bands(&mask, &recipe).expect("bands should resolve");
        assert!(baseline.dry_coverage_percent.abs_diff(55) <= 4);

        for turns in 0..6 {
            let frame = super::super::local_frame::LocalPatchFrame::from_resolved_ring19(
                HexCoord::ORIGIN,
                12,
                turns,
            );
            let world = baseline
                .by_coord
                .iter()
                .map(|(coord, band)| (frame.to_world(*coord).expect("rotation should fit"), *band))
                .collect::<BTreeMap<_, _>>();
            let normalized = world
                .into_iter()
                .map(|(coord, band)| {
                    (
                        frame.to_local(coord).expect("inverse rotation should fit"),
                        band,
                    )
                })
                .collect::<BTreeMap<_, _>>();
            assert_eq!(normalized, baseline.by_coord);
            for band in [
                TransitionBand::Grass,
                TransitionBand::Transition,
                TransitionBand::Sand,
            ] {
                let coords = baseline
                    .by_coord
                    .iter()
                    .filter_map(|(coord, actual)| (*actual == band).then_some(*coord))
                    .collect::<BTreeSet<_>>();
                assert!(coords_are_connected(&coords));
            }
        }
    }

    #[test]
    fn material_columns_publish_exact_grass_dirt_and_sand_tops() {
        for (band, expected) in [
            (TransitionBand::Grass, SolidMaterialRole::Grass),
            (TransitionBand::Transition, SolidMaterialRole::Dirt),
            (TransitionBand::Sand, SolidMaterialRole::Sand),
        ] {
            let position = TilePos::new(HexCoord::ORIGIN, 15);
            let column = transition_column(15, band);
            let strata = column
                .elements
                .iter()
                .map(|element| match *element {
                    VolumeElement::Solid(mass) => {
                        (mass.levels.bottom, mass.levels.top, mass.material)
                    }
                    VolumeElement::Fill(_) => {
                        panic!("a dry transition column cannot contain fill")
                    }
                })
                .collect::<Vec<_>>();
            let expected_strata = if expected == SolidMaterialRole::Dirt {
                vec![
                    (0, 1, SolidMaterialRole::Bedrock),
                    (1, 12, SolidMaterialRole::Stone),
                    (12, 16, SolidMaterialRole::Dirt),
                ]
            } else {
                vec![
                    (0, 1, SolidMaterialRole::Bedrock),
                    (1, 12, SolidMaterialRole::Stone),
                    (12, 15, SolidMaterialRole::Dirt),
                    (15, 16, expected),
                ]
            };
            assert_eq!(
                strata, expected_strata,
                "every transition band must retain the shared exact three-level arid substrate"
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
            assert_eq!(surface_material(&volume, position), Some(expected));
            assert!(volume.validate().is_ok());
        }
    }
}
