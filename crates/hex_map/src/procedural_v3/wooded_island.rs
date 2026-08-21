//! Pure V3 Wooded Island recipe.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use hex_assets::RuntimeArtCatalog;
use hex_core::{HexCoord, MapViewHint, TilePos};

use super::coastal_island::{
    build_semantics, connected_components, coverage_target, inward_distances, plan_coast,
    surface_material, validate_ocean_topology, CoastalPlannerSettings, CoastalSurfacePalette,
    REQUIRED_SEA_LEVEL, SAND_FRINGE_WIDTH,
};
use super::composition::{compose_single_patch, GeneratedPatchPlan};
use super::layout::{resolve_layout, PatchId, ResolvedLayoutPlan};
use super::patch::{PatchBuildMode, PatchRecipeContext};
use super::seam::{shape_walker_seams, validate_patch_walker_seams};
use super::selection::{
    run_recipe, CandidateAttemptError, CandidateContext, FallbackContext, RepairOutcome, V3Recipe,
    ValidatedWorldSelection, WorldValidation,
};
use super::traversal::OrdinaryGraph;
use super::vegetation::{
    append_landform_vegetation, validate_landform_vegetation, LandformVegetationDomain,
    LandformVegetationSet, TemperateVegetationSet,
};
use super::volume::{SolidMaterialRole, SurfaceAccess, VolumePlan};
use super::world::{
    FeaturePlan, GeneratedWorldPlan, InteriorPlan, ProtectedFeatureRoute, StructurePlan,
    WorldIssueCode, WorldValidationIssue,
};
use super::V3GenerationError;
use crate::procedural::WoodedIslandMetrics;
use crate::settings::{
    ProceduralV3Settings, V3EnvironmentSettings, V3LayoutSettings, V3RecipeSettings,
    V3WoodedIslandSettings,
};

const RECIPE: &str = "wooded-island";
const FOCUSED_RADIUS: u32 = 40;
const PARTY_START: &str = "party_start";
const HOSTILE_START: &str = "hostile_start";
const MACRO_ROUTE_END: &str = "macro_route_end";
const BEACH: &str = "wooded_island_beach";
const CLEARING: &str = "wooded_island_clearing";
const RIDGE: &str = "wooded_island_ridge";
const CROSSING: &str = "wooded_island_crossing";

#[derive(Debug)]
struct WoodedIslandRecipe {
    level_height: f32,
    layout: ResolvedLayoutPlan,
    vegetation: TemperateVegetationSet,
}

pub(crate) fn generate(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV3Settings,
    seed: u64,
    catalog: &RuntimeArtCatalog,
) -> Result<ValidatedWorldSelection<WoodedIslandMetrics>, V3GenerationError> {
    if !level_height.is_finite() || level_height <= 0.0 {
        return Err(V3GenerationError::RecipeContract(
            "Wooded Island level height must be positive and finite".to_owned(),
        ));
    }
    let _ = validate_recipe_settings(settings, grid_radius)?;
    let layout = resolve_layout(grid_radius, settings)
        .map_err(|error| V3GenerationError::RecipeContract(error.to_string()))?;
    let vegetation = TemperateVegetationSet::resolve(catalog, "Wooded Island")
        .map_err(V3GenerationError::RecipeContract)?;
    run_recipe(
        &WoodedIslandRecipe {
            level_height,
            layout,
            vegetation,
        },
        settings,
        grid_radius,
        seed,
    )
}

impl V3Recipe for WoodedIslandRecipe {
    type Settings = ProceduralV3Settings;
    type Metrics = WoodedIslandMetrics;
    type Score = (u32, u8);

    fn construct(
        &self,
        context: CandidateContext,
        settings: &Self::Settings,
    ) -> Result<GeneratedWorldPlan, CandidateAttemptError> {
        let recipe = validate_recipe_settings(settings, context.grid_radius)
            .map_err(CandidateAttemptError::Fatal)?;
        let patch = PatchRecipeContext::resolve(&self.layout, PatchId(0))
            .map_err(CandidateAttemptError::Fatal)?;
        let fragment = construct_patch_with_vegetation(
            patch,
            recipe,
            V3EnvironmentSettings::Coastal,
            self.level_height,
            PatchBuildMode::Candidate {
                world_seed: context.seed,
                candidate: context.candidate,
            },
            &self.vegetation,
        )
        .map_err(CandidateAttemptError::Rejected)?;
        compose_single_patch(self.layout.clone(), fragment).map_err(|error| {
            CandidateAttemptError::Rejected(vec![recipe_issue(format!("{error:?}"))])
        })
    }

    fn validate(
        &self,
        settings: &Self::Settings,
        plan: &GeneratedWorldPlan,
    ) -> WorldValidation<Self::Metrics> {
        let Ok(recipe) = validate_recipe_settings(settings, plan.layout.grid_radius) else {
            return WorldValidation::Invalid(vec![recipe_issue(
                "Wooded Island settings changed after construction",
            )]);
        };
        validate_world(plan, recipe, &self.vegetation)
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
        settings: &Self::Settings,
        metrics: &Self::Metrics,
        candidate: u8,
    ) -> Self::Score {
        let target = match &settings.layout {
            V3LayoutSettings::Single(patch) => match &patch.recipe {
                V3RecipeSettings::WoodedIsland(recipe) => coverage_target(
                    usize::try_from(metrics.world_columns).unwrap_or(usize::MAX),
                    recipe.land_coverage_percent,
                ),
                _ => 0,
            },
            V3LayoutSettings::Ring7(_)
            | V3LayoutSettings::Ring19(_)
            | V3LayoutSettings::Macro(_) => 0,
        };
        (
            metrics
                .land_surfaces
                .abs_diff(u32::try_from(target).unwrap_or(u32::MAX)),
            candidate,
        )
    }

    fn canonical_fallback(
        &self,
        context: FallbackContext,
        settings: &Self::Settings,
    ) -> Result<GeneratedWorldPlan, V3GenerationError> {
        let recipe = validate_recipe_settings(settings, context.grid_radius)?;
        let patch = PatchRecipeContext::resolve(&self.layout, PatchId(0))?;
        let fragment = construct_patch_with_vegetation(
            patch,
            recipe,
            V3EnvironmentSettings::Coastal,
            self.level_height,
            PatchBuildMode::CanonicalFallback,
            &self.vegetation,
        )
        .map_err(recipe_issues_to_error)?;
        compose_single_patch(self.layout.clone(), fragment)
            .map_err(|error| V3GenerationError::RecipeContract(format!("{error:?}")))
    }
}

fn validate_recipe_settings(
    settings: &ProceduralV3Settings,
    grid_radius: u32,
) -> Result<&V3WoodedIslandSettings, V3GenerationError> {
    if grid_radius != FOCUSED_RADIUS {
        return Err(V3GenerationError::RecipeContract(format!(
            "standalone Wooded Island requires radius {FOCUSED_RADIUS}"
        )));
    }
    let V3LayoutSettings::Single(patch) = &settings.layout else {
        return Err(V3GenerationError::RecipeContract(
            "Wooded Island standalone generation requires a Single layout".to_owned(),
        ));
    };
    if patch.environment != V3EnvironmentSettings::Coastal
        || !patch.overlays.is_empty()
        || !matches!(patch.mask, crate::settings::PatchMaskSettings::WholeWorld)
    {
        return Err(V3GenerationError::RecipeContract(
            "Wooded Island requires a whole-world Coastal patch without overlays".to_owned(),
        ));
    }
    let V3RecipeSettings::WoodedIsland(recipe) = &patch.recipe else {
        return Err(V3GenerationError::RecipeUnavailable("WoodedIsland"));
    };
    if recipe.sea_level != REQUIRED_SEA_LEVEL
        || !(50..=80).contains(&recipe.land_coverage_percent)
        || !(3..=8).contains(&recipe.max_relief)
        || !(18..=35).contains(&recipe.tree_coverage_percent)
    {
        return Err(V3GenerationError::RecipeContract(
            "Wooded Island settings violate the validated coastal range".to_owned(),
        ));
    }
    Ok(recipe)
}

pub(crate) fn construct_patch_with_vegetation(
    patch: PatchRecipeContext<'_>,
    settings: &V3WoodedIslandSettings,
    environment: V3EnvironmentSettings,
    level_height: f32,
    mode: PatchBuildMode,
    vegetation: &TemperateVegetationSet,
) -> Result<GeneratedPatchPlan, Vec<WorldValidationIssue>> {
    validate_patch_contract(patch, settings, environment, level_height)?;
    let walker_approaches = patch.walker_protected_approaches();
    let streams = mode.seed_streams(&patch);
    let mut coast = plan_coast(
        patch.mask(),
        CoastalPlannerSettings {
            sea_level: settings.sea_level,
            land_coverage_percent: settings.land_coverage_percent,
            component_count: 1,
            max_relief: settings.max_relief,
        },
        &walker_approaches,
        streams.map(|streams| streams.stage("wooded-island.coastline")),
    )
    .map_err(|error| vec![recipe_issue(error)])?;

    let mut seam_levels = patch
        .mask()
        .iter()
        .copied()
        .map(|coord| {
            (
                coord,
                coast
                    .levels
                    .get(&coord)
                    .copied()
                    .unwrap_or(settings.sea_level),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let seam_shape = shape_walker_seams(&patch, &mut seam_levels)?;
    for (coord, level) in &mut coast.levels {
        if let Some(shaped) = seam_levels.get(coord).copied() {
            *level = shaped;
        }
    }
    let semantics = build_semantics(
        patch.mask(),
        &coast,
        settings.sea_level,
        CoastalSurfacePalette::SandAndGrass,
        |position, access| seam_shape.access_for(position, access),
    )
    .map_err(|error| vec![recipe_issue(error)])?;
    let primary = coast.primary();
    let ordinary_dry = semantics
        .dry_surfaces
        .iter()
        .filter(|(_, surface)| {
            semantics
                .volume
                .surfaces
                .get(surface)
                .is_some_and(|metadata| metadata.access == SurfaceAccess::Ordinary)
        })
        .map(|(coord, surface)| (*coord, *surface))
        .collect::<BTreeMap<_, _>>();
    let party = walker_approaches
        .iter()
        .filter(|coord| primary.contains(coord) && coast.sand_fringe.contains(coord))
        .filter_map(|coord| ordinary_dry.get(coord).copied())
        .min_by_key(|position| *position)
        .or_else(|| beach_anchor(primary, &coast.sand_fringe, &ordinary_dry))
        .ok_or_else(|| vec![recipe_issue("Wooded Island has no beach landing")])?;
    let ridge = ridge_anchor(primary, &ordinary_dry)
        .ok_or_else(|| vec![recipe_issue("Wooded Island has no ridge")])?;
    let clearing = clearing_anchor(&coast.grass_interior, &ordinary_dry)
        .ok_or_else(|| vec![recipe_issue("Wooded Island has no inland clearing")])?;
    let hostile = primary
        .iter()
        .filter_map(|coord| ordinary_dry.get(coord).copied())
        .max_by_key(|position| {
            (
                party.coord.distance(position.coord),
                position.level,
                position.coord,
            )
        })
        .ok_or_else(|| vec![recipe_issue("Wooded Island has no route destination")])?;
    let route = shortest_surface_path(primary, &ordinary_dry, party, hostile)
        .ok_or_else(|| vec![recipe_issue("Wooded Island crossing is disconnected")])?;
    let route_surfaces = route.iter().copied().collect::<BTreeSet<_>>();
    let anchors = BTreeMap::from([
        (PARTY_START.to_owned(), party),
        (HOSTILE_START.to_owned(), hostile),
        (MACRO_ROUTE_END.to_owned(), hostile),
        (BEACH.to_owned(), party),
        (CLEARING.to_owned(), clearing),
        (RIDGE.to_owned(), ridge),
    ]);
    let mut reserved = patch.protected_approaches();
    reserved.extend(route_surfaces.iter().map(|position| position.coord));
    for anchor in anchors.values() {
        reserved.extend(anchor.coord.within_radius(1));
    }
    let tree_candidates = coast
        .grass_interior
        .difference(&reserved)
        .copied()
        .collect::<BTreeSet<_>>();
    let tree_target = canopy_tree_target(tree_candidates.len(), settings.tree_coverage_percent);
    let mut features = FeaturePlan {
        by_id: BTreeMap::new(),
        protected_routes: BTreeMap::from([(
            CROSSING.to_owned(),
            ProtectedFeatureRoute {
                centerline: route,
                surfaces: route_surfaces,
            },
        )]),
        clearings: BTreeMap::new(),
    };
    let mut blockers = BTreeSet::new();
    let objects = LandformVegetationSet::from_coastal_temperate(vegetation);
    append_landform_vegetation(
        "Wooded Island",
        &objects,
        &semantics.dry_surfaces,
        &tree_candidates,
        &BTreeSet::new(),
        &reserved,
        tree_target,
        0,
        streams.map(|streams| streams.stage("wooded-island.trees")),
        None,
        &mut features,
        &mut blockers,
    )
    .map_err(|error| vec![recipe_issue(error)])?;
    let biome_regions = semantics
        .volume
        .surfaces
        .keys()
        .copied()
        .map(|surface| (surface, patch.biome_region()))
        .collect();
    let plan = GeneratedPatchPlan {
        patch_id: patch.id,
        volume: semantics.volume,
        liquids: semantics.liquids,
        features,
        structures: StructurePlan::default(),
        blockers,
        lights: BTreeMap::new(),
        biome_regions,
        interiors: InteriorPlan::default(),
        anchors,
        view_hint: island_view_hint(
            patch.grid_radius(),
            settings.sea_level,
            settings.max_relief,
            level_height,
        )?,
    };
    let seam_issues = validate_patch_walker_seams(&patch, &plan.volume);
    if seam_issues.is_empty() {
        Ok(plan)
    } else {
        Err(seam_issues)
    }
}

pub(crate) fn validate_patch_with_vegetation(
    patch: PatchRecipeContext<'_>,
    settings: &V3WoodedIslandSettings,
    plan: &GeneratedPatchPlan,
    vegetation: &TemperateVegetationSet,
) -> WorldValidation<WoodedIslandMetrics> {
    let mut issues = validate_patch_walker_seams(&patch, &plan.volume);
    match validate_parts(
        &plan.volume,
        &plan.liquids,
        &plan.features,
        &plan.structures,
        &plan.blockers,
        &plan.lights,
        &plan.interiors,
        &plan.anchors,
        settings,
        vegetation,
        &patch.protected_approaches(),
        &patch.walker_protected_approaches(),
    ) {
        WorldValidation::Valid(metrics) if issues.is_empty() => WorldValidation::Valid(metrics),
        WorldValidation::Valid(_) => WorldValidation::Invalid(issues),
        WorldValidation::Invalid(mut recipe_issues) => {
            issues.append(&mut recipe_issues);
            WorldValidation::Invalid(issues)
        }
    }
}

fn validate_world(
    plan: &GeneratedWorldPlan,
    settings: &V3WoodedIslandSettings,
    vegetation: &TemperateVegetationSet,
) -> WorldValidation<WoodedIslandMetrics> {
    let mut common = plan.validate();
    match validate_parts(
        &plan.volume,
        &plan.liquids,
        &plan.features,
        &plan.structures,
        &plan.blockers,
        &plan.lights,
        &plan.interiors,
        &plan.anchors,
        settings,
        vegetation,
        &BTreeSet::new(),
        &BTreeSet::new(),
    ) {
        WorldValidation::Valid(metrics) if common.is_empty() => WorldValidation::Valid(metrics),
        WorldValidation::Valid(_) => WorldValidation::Invalid(common),
        WorldValidation::Invalid(mut issues) => {
            common.append(&mut issues);
            WorldValidation::Invalid(common)
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "recipe validation keeps every semantic authority explicit"
)]
fn validate_parts(
    volume: &VolumePlan,
    liquids: &super::liquid::LiquidPlan,
    features: &FeaturePlan,
    structures: &StructurePlan,
    blockers: &BTreeSet<TilePos>,
    lights: &BTreeMap<super::world::LightId, super::world::PlannedGameplayLight>,
    interiors: &InteriorPlan,
    anchors: &BTreeMap<String, TilePos>,
    settings: &V3WoodedIslandSettings,
    vegetation: &TemperateVegetationSet,
    protected_approaches: &BTreeSet<HexCoord>,
    allowed_dry_boundary: &BTreeSet<HexCoord>,
) -> WorldValidation<WoodedIslandMetrics> {
    let mut issues = Vec::new();
    if !structures.by_id.is_empty() || !lights.is_empty() || !interiors.by_id.is_empty() {
        issues.push(recipe_issue(
            "Wooded Island must not publish structures, lights, or interiors",
        ));
    }
    let expected_anchor_names = BTreeSet::from([
        PARTY_START,
        HOSTILE_START,
        MACRO_ROUTE_END,
        BEACH,
        CLEARING,
        RIDGE,
    ]);
    let actual_anchor_names = anchors.keys().map(String::as_str).collect::<BTreeSet<_>>();
    if actual_anchor_names != expected_anchor_names {
        issues.push(recipe_issue(format!(
            "Wooded Island requires exactly anchors {expected_anchor_names:?}"
        )));
    }
    let water = liquids
        .bodies
        .values()
        .flat_map(|body| body.nodes.keys().map(|position| position.coord))
        .collect::<BTreeSet<_>>();
    let dry_coords = volume
        .mask
        .difference(&water)
        .copied()
        .collect::<BTreeSet<_>>();
    if let Err(error) = validate_ocean_topology(&volume.mask, &water, allowed_dry_boundary) {
        issues.push(recipe_issue(error));
    }
    let mut dry = BTreeMap::new();
    for position in volume
        .surfaces
        .keys()
        .filter(|position| dry_coords.contains(&position.coord))
    {
        dry.entry(position.coord)
            .and_modify(|surface: &mut TilePos| *surface = (*surface).max(*position))
            .or_insert(*position);
    }
    let target = coverage_target(volume.mask.len(), settings.land_coverage_percent);
    if dry.len() != target || connected_components(&dry_coords).len() != 1 {
        issues.push(recipe_issue(format!(
            "Wooded Island requires one connected {target}-surface landmass, got {} surfaces",
            dry.len()
        )));
    }
    validate_water(volume, liquids, &water, settings.sea_level, &mut issues);
    let inward = inward_distances(&dry_coords);
    let sand = inward
        .iter()
        .filter_map(|(coord, distance)| (*distance <= SAND_FRINGE_WIDTH).then_some(*coord))
        .collect::<BTreeSet<_>>();
    let grass = dry_coords
        .difference(&sand)
        .copied()
        .collect::<BTreeSet<_>>();
    for (coord, position) in &dry {
        let expected = if sand.contains(coord) {
            SolidMaterialRole::Sand
        } else {
            SolidMaterialRole::Grass
        };
        if surface_material(volume, *position) != Some(expected) {
            issues.push(recipe_issue(format!(
                "Wooded Island surface {position:?} must use exact {expected:?} shoreline classification"
            )));
        }
    }
    if sand
        .iter()
        .any(|coord| inward.get(coord).is_none_or(|distance| *distance > 2))
        || grass
            .iter()
            .any(|coord| inward.get(coord).is_none_or(|distance| *distance < 3))
    {
        issues.push(recipe_issue(
            "Wooded Island sand fringe must be exactly two inward columns",
        ));
    }

    let route = features.protected_routes.get(CROSSING);
    let route_surfaces = route
        .map(|route| route.surfaces.clone())
        .unwrap_or_default();
    let mut reserved = protected_approaches.clone();
    reserved.extend(route_surfaces.iter().map(|position| position.coord));
    for anchor in anchors.values() {
        reserved.extend(anchor.coord.within_radius(1));
    }
    let eligible = grass
        .difference(&reserved)
        .copied()
        .collect::<BTreeSet<_>>();
    let expected_trees = canopy_tree_target(eligible.len(), settings.tree_coverage_percent);
    let objects = LandformVegetationSet::from_coastal_temperate(vegetation);
    let vegetation_validation = validate_landform_vegetation(
        "Wooded Island",
        &objects,
        &[LandformVegetationDomain {
            surfaces: &dry,
            reserved: &reserved,
        }],
        features,
        &BTreeSet::new(),
        blockers,
    );
    let tree_roots = match vegetation_validation {
        Ok(metrics) => {
            if metrics.trees != expected_trees || metrics.grass != 0 {
                issues.push(recipe_issue(format!(
                    "Wooded Island requires {expected_trees} trees and no grass props; got {} trees and {} grass",
                    metrics.trees, metrics.grass
                )));
            }
            metrics.trees
        }
        Err(errors) => {
            issues.extend(errors.into_iter().map(recipe_issue));
            0
        }
    };
    if features.by_id.values().any(|feature| {
        feature.kind != super::world::FeatureKind::Tree
            || !grass.contains(&feature.root.coord)
            || reserved.contains(&feature.root.coord)
    }) {
        issues.push(recipe_issue(
            "Wooded Island trees must root only in unreserved grass interior",
        ));
    }

    let party = anchors.get(PARTY_START).copied();
    let hostile = anchors.get(HOSTILE_START).copied();
    let graph = OrdinaryGraph::from_volume(volume, Some(blockers));
    let reachable = party.map_or_else(BTreeMap::new, |party| graph.distances_from(party));
    if anchors.get(BEACH).copied() != party
        || anchors.get(MACRO_ROUTE_END).copied() != hostile
        || hostile.is_none_or(|position| !reachable.contains_key(&position))
        || anchors
            .get(CLEARING)
            .is_none_or(|position| !grass.contains(&position.coord))
        || anchors
            .get(RIDGE)
            .is_none_or(|position| !reachable.contains_key(position))
    {
        issues.push(recipe_issue(
            "Wooded Island stable anchors must retain exact beach, clearing, ridge, and route roles",
        ));
    }
    validate_route(features, party, hostile, &graph, blockers, &mut issues);
    let critical_route_steps = hostile
        .and_then(|position| reachable.get(&position).copied())
        .unwrap_or_default();
    let lowest = dry
        .values()
        .map(|position| position.level)
        .min()
        .unwrap_or(settings.sea_level);
    let highest = dry
        .values()
        .map(|position| position.level)
        .max()
        .unwrap_or(settings.sea_level);
    if highest.saturating_sub(settings.sea_level) != settings.max_relief {
        issues.push(recipe_issue(format!(
            "Wooded Island highest dry rise must be {}, got {}",
            settings.max_relief,
            highest.saturating_sub(settings.sea_level)
        )));
    }
    let reachable_levels = reachable
        .keys()
        .map(|position| position.level)
        .collect::<BTreeSet<_>>();
    let metrics = WoodedIslandMetrics {
        world_columns: count_u32(volume.mask.len()),
        land_surfaces: count_u32(dry.len()),
        water_cells: count_u32(water.len()),
        sand_fringe_surfaces: count_u32(sand.len()),
        grass_interior_surfaces: count_u32(grass.len()),
        tree_roots: count_u32(tree_roots),
        reachable_surfaces: count_u32(reachable.len()),
        reachable_elevation_levels: count_u32(reachable_levels.len()),
        relief: highest.saturating_sub(lowest),
        critical_route_steps,
    };
    if issues.is_empty() {
        WorldValidation::Valid(metrics)
    } else {
        WorldValidation::Invalid(issues)
    }
}

fn validate_water(
    volume: &VolumePlan,
    liquids: &super::liquid::LiquidPlan,
    water: &BTreeSet<HexCoord>,
    sea_level: i32,
    issues: &mut Vec<WorldValidationIssue>,
) {
    let actual = liquids
        .bodies
        .values()
        .flat_map(|body| body.nodes.iter())
        .filter_map(|(position, node)| {
            (node.state == super::liquid::LiquidFlowState::Still
                && node.downstream.is_none()
                && position.level == sea_level)
                .then_some(position.coord)
        })
        .collect::<BTreeSet<_>>();
    if actual != *water {
        issues.push(recipe_issue(
            "Wooded Island water nodes must exactly cover every non-land column at level 8",
        ));
    }
    for coord in water {
        let valid_bed = volume.surfaces.iter().any(|(position, metadata)| {
            position.coord == *coord
                && position.level == sea_level.saturating_sub(2)
                && metadata.access == SurfaceAccess::NonStandable
        });
        if !valid_bed {
            issues.push(recipe_issue(format!(
                "Wooded Island water column {coord:?} lacks its exact nonstandable seabed"
            )));
        }
    }
}

fn validate_route(
    features: &FeaturePlan,
    party: Option<TilePos>,
    hostile: Option<TilePos>,
    graph: &OrdinaryGraph,
    blockers: &BTreeSet<TilePos>,
    issues: &mut Vec<WorldValidationIssue>,
) {
    let route = features.protected_routes.get(CROSSING);
    if features.protected_routes.len() != 1 || route.is_none() {
        issues.push(recipe_issue(
            "Wooded Island requires exactly one protected crossing",
        ));
        return;
    }
    let Some(route) = route else {
        return;
    };
    if route.centerline.first().copied() != party
        || route.centerline.last().copied() != hostile
        || route.centerline.iter().copied().collect::<BTreeSet<_>>() != route.surfaces
        || !route.surfaces.is_disjoint(blockers)
        || route
            .centerline
            .windows(2)
            .any(|pair| !matches!(pair, [from, to] if graph.admits(*from, *to)))
    {
        issues.push(recipe_issue(
            "Wooded Island crossing must exactly and continuously join its actor anchors without blockers",
        ));
    }
}

fn validate_patch_contract(
    patch: PatchRecipeContext<'_>,
    settings: &V3WoodedIslandSettings,
    environment: V3EnvironmentSettings,
    level_height: f32,
) -> Result<(), Vec<WorldValidationIssue>> {
    if environment != V3EnvironmentSettings::Coastal
        || settings.sea_level != REQUIRED_SEA_LEVEL
        || !(50..=80).contains(&settings.land_coverage_percent)
        || !(3..=8).contains(&settings.max_relief)
        || !(18..=35).contains(&settings.tree_coverage_percent)
        || !level_height.is_finite()
        || level_height <= 0.0
        || patch.mask().len() < 127
    {
        return Err(vec![recipe_issue(
            "Wooded Island patch violates its Coastal settings or capacity contract",
        )]);
    }
    Ok(())
}

fn beach_anchor(
    primary: &BTreeSet<HexCoord>,
    sand: &BTreeSet<HexCoord>,
    surfaces: &BTreeMap<HexCoord, TilePos>,
) -> Option<TilePos> {
    primary
        .intersection(sand)
        .filter_map(|coord| surfaces.get(coord).copied())
        .min_by_key(|position| (position.coord.x(), position.coord.y(), *position))
}

fn ridge_anchor(
    primary: &BTreeSet<HexCoord>,
    surfaces: &BTreeMap<HexCoord, TilePos>,
) -> Option<TilePos> {
    primary
        .iter()
        .filter_map(|coord| surfaces.get(coord).copied())
        .max_by_key(|position| (position.level, position.coord, *position))
}

fn clearing_anchor(
    grass: &BTreeSet<HexCoord>,
    surfaces: &BTreeMap<HexCoord, TilePos>,
) -> Option<TilePos> {
    grass
        .iter()
        .filter_map(|coord| surfaces.get(coord).copied())
        .max_by_key(|position| {
            let grass_depth = position
                .coord
                .neighbors()
                .into_iter()
                .filter(|neighbor| grass.contains(neighbor))
                .count();
            (grass_depth, position.level, position.coord, *position)
        })
}

fn shortest_surface_path(
    allowed: &BTreeSet<HexCoord>,
    surfaces: &BTreeMap<HexCoord, TilePos>,
    start: TilePos,
    goal: TilePos,
) -> Option<Vec<TilePos>> {
    let mut parent = BTreeMap::from([(start.coord, None)]);
    let mut frontier = VecDeque::from([start.coord]);
    while let Some(coord) = frontier.pop_front() {
        if coord == goal.coord {
            break;
        }
        let from = surfaces.get(&coord).copied()?;
        let mut neighbors = coord.neighbors();
        neighbors.sort_unstable();
        for neighbor in neighbors {
            let Some(to) = surfaces.get(&neighbor).copied() else {
                continue;
            };
            if allowed.contains(&neighbor)
                && from.level.abs_diff(to.level) <= 1
                && !parent.contains_key(&neighbor)
            {
                parent.insert(neighbor, Some(coord));
                frontier.push_back(neighbor);
            }
        }
    }
    if !parent.contains_key(&goal.coord) {
        return None;
    }
    let mut cursor = goal.coord;
    let mut path = vec![goal];
    while cursor != start.coord {
        cursor = parent.get(&cursor).copied().flatten()?;
        path.push(surfaces.get(&cursor).copied()?);
    }
    path.reverse();
    Some(path)
}

fn island_view_hint(
    radius: u32,
    sea_level: i32,
    relief: i32,
    level_height: f32,
) -> Result<MapViewHint, Vec<WorldValidationIssue>> {
    let radius = u16::try_from(radius)
        .map(f32::from)
        .map_err(|error| vec![recipe_issue(format!("radius exceeds u16: {error}"))])?;
    let focus = i16::try_from(sea_level.saturating_add(relief / 2))
        .map(f32::from)
        .map_err(|error| vec![recipe_issue(format!("focus exceeds i16: {error}"))])?
        * level_height;
    let hint = MapViewHint::new(
        (
            radius.mul_add(1.3, 5.0),
            focus + radius.mul_add(0.9, 8.0),
            radius.mul_add(1.4, 5.0),
        ),
        (0.0, focus, 0.0),
    );
    hint.is_valid()
        .then_some(hint)
        .ok_or_else(|| vec![recipe_issue("Wooded Island camera hint is invalid")])
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

/// Converts authored canopy coverage into non-overlapping object roots.
///
/// Both accepted broadleaf silhouettes cover several horizontal columns. One
/// root therefore represents roughly two candidate columns of canopy; treating
/// the authored percentage as a raw root percentage overcommits exact visual
/// volumes even in the radius-40 showcase.
fn canopy_tree_target(eligible_columns: usize, canopy_percent: u8) -> usize {
    eligible_columns.saturating_mul(usize::from(canopy_percent)) / 200
}

fn recipe_issue(detail: impl Into<String>) -> WorldValidationIssue {
    WorldValidationIssue::new(WorldIssueCode::Recipe(RECIPE), detail)
}

#[cfg(test)]
mod tests {
    use super::super::vegetation::tests::runtime_art_catalog;
    use super::*;
    use crate::settings::{
        PatchEdgeContractSettings, PatchEdgesSettings, PatchMaskSettings, PatchSpec,
    };

    fn settings() -> ProceduralV3Settings {
        ProceduralV3Settings {
            layout: V3LayoutSettings::Single(PatchSpec {
                environment: V3EnvironmentSettings::Coastal,
                recipe: V3RecipeSettings::WoodedIsland(V3WoodedIslandSettings {
                    sea_level: 8,
                    land_coverage_percent: 65,
                    max_relief: 6,
                    tree_coverage_percent: 25,
                }),
                overlays: Vec::new(),
                mask: PatchMaskSettings::WholeWorld,
                edges: PatchEdgesSettings {
                    east: PatchEdgeContractSettings::WorldBoundary,
                    south_east: PatchEdgeContractSettings::WorldBoundary,
                    south_west: PatchEdgeContractSettings::WorldBoundary,
                    west: PatchEdgeContractSettings::WorldBoundary,
                    north_west: PatchEdgeContractSettings::WorldBoundary,
                    north_east: PatchEdgeContractSettings::WorldBoundary,
                },
            }),
        }
    }

    #[test]
    fn focused_recipe_builds_one_exact_fringed_wooded_island() {
        let first = generate(40, 0.4, &settings(), 0x15_1a, runtime_art_catalog())
            .expect("Wooded Island should generate");
        let second = generate(40, 0.4, &settings(), 0x15_1a, runtime_art_catalog())
            .expect("Wooded Island should repeat");
        assert_eq!(
            first.validated.semantic_fingerprint,
            second.validated.semantic_fingerprint
        );
        assert_eq!(first.metrics.land_surfaces, 3_199);
        assert_eq!(
            first.metrics.land_surfaces,
            first
                .metrics
                .sand_fringe_surfaces
                .saturating_add(first.metrics.grass_interior_surfaces)
        );
        assert!(first.metrics.tree_roots > 0);
        assert!(first.metrics.critical_route_steps > 0);
        assert_eq!(first.metrics.relief, 5);
    }

    #[test]
    fn standalone_radius_is_strict() {
        assert!(matches!(
            generate(39, 0.4, &settings(), 1, runtime_art_catalog()),
            Err(V3GenerationError::RecipeContract(_))
        ));
    }
}
