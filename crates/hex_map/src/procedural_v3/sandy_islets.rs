//! Pure V3 Sandy Islets recipe.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

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
use super::volume::{SurfaceAccess, VolumePlan};
use super::world::{
    FeaturePlan, GeneratedWorldPlan, InteriorPlan, ProtectedFeatureRoute, StructurePlan,
    WorldIssueCode, WorldValidationIssue,
};
use super::V3GenerationError;
use crate::procedural::SandyIsletsMetrics;
use crate::settings::{
    ProceduralV3Settings, V3EnvironmentSettings, V3LayoutSettings, V3RecipeSettings,
    V3SandyIsletsSettings,
};

const RECIPE: &str = "sandy-islets";
const FOCUSED_RADIUS: u32 = 24;
const PARTY_START: &str = "party_start";
const HOSTILE_START: &str = "hostile_start";
const PRIMARY_OVERLOOK: &str = "sandy_islets_primary_overlook";
const CHANNEL_OVERLOOK: &str = "sandy_islets_channel_overlook";
const PRIMARY_ROUTE: &str = "sandy_islets_primary_route";

#[derive(Debug)]
struct SandyIsletsRecipe {
    level_height: f32,
    layout: ResolvedLayoutPlan,
}

pub(crate) fn generate(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV3Settings,
    seed: u64,
) -> Result<ValidatedWorldSelection<SandyIsletsMetrics>, V3GenerationError> {
    if !level_height.is_finite() || level_height <= 0.0 {
        return Err(V3GenerationError::RecipeContract(
            "Sandy Islets level height must be positive and finite".to_owned(),
        ));
    }
    let _ = validate_recipe_settings(settings, grid_radius)?;
    let layout = resolve_layout(grid_radius, settings)
        .map_err(|error| V3GenerationError::RecipeContract(error.to_string()))?;
    run_recipe(
        &SandyIsletsRecipe {
            level_height,
            layout,
        },
        settings,
        grid_radius,
        seed,
    )
}

impl V3Recipe for SandyIsletsRecipe {
    type Settings = ProceduralV3Settings;
    type Metrics = SandyIsletsMetrics;
    type Score = (u32, u8);

    fn construct(
        &self,
        context: CandidateContext,
        settings: &Self::Settings,
    ) -> Result<GeneratedWorldPlan, CandidateAttemptError> {
        let recipe = validate_recipe_settings(settings, context.grid_radius)
            .map_err(CandidateAttemptError::Fatal)?;
        if context.grid_radius != self.layout.grid_radius {
            return Err(CandidateAttemptError::Fatal(
                V3GenerationError::RecipeContract(
                    "Sandy Islets candidate radius disagrees with its layout".to_owned(),
                ),
            ));
        }
        let patch = PatchRecipeContext::resolve(&self.layout, PatchId(0))
            .map_err(CandidateAttemptError::Fatal)?;
        let fragment = construct_patch(
            patch,
            recipe,
            V3EnvironmentSettings::Coastal,
            self.level_height,
            PatchBuildMode::Candidate {
                world_seed: context.seed,
                candidate: context.candidate,
            },
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
                "Sandy Islets settings changed after construction",
            )]);
        };
        validate_world(plan, recipe)
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
                V3RecipeSettings::SandyIslets(recipe) => coverage_target(
                    usize::try_from(metrics.world_columns).unwrap_or(usize::MAX),
                    recipe.land_coverage_percent,
                ),
                _ => 0,
            },
            V3LayoutSettings::Ring7(_)
            | V3LayoutSettings::Ring19(_)
            | V3LayoutSettings::Macro(_)
            | V3LayoutSettings::Schematic(_) => 0,
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
        let fragment = construct_patch(
            patch,
            recipe,
            V3EnvironmentSettings::Coastal,
            self.level_height,
            PatchBuildMode::CanonicalFallback,
        )
        .map_err(recipe_issues_to_error)?;
        compose_single_patch(self.layout.clone(), fragment)
            .map_err(|error| V3GenerationError::RecipeContract(format!("{error:?}")))
    }
}

fn validate_recipe_settings(
    settings: &ProceduralV3Settings,
    grid_radius: u32,
) -> Result<&V3SandyIsletsSettings, V3GenerationError> {
    if grid_radius != FOCUSED_RADIUS {
        return Err(V3GenerationError::RecipeContract(format!(
            "standalone Sandy Islets requires radius {FOCUSED_RADIUS}"
        )));
    }
    let V3LayoutSettings::Single(patch) = &settings.layout else {
        return Err(V3GenerationError::RecipeContract(
            "Sandy Islets standalone generation requires a Single layout".to_owned(),
        ));
    };
    if patch.environment != V3EnvironmentSettings::Coastal
        || !patch.overlays.is_empty()
        || !matches!(patch.mask, crate::settings::PatchMaskSettings::WholeWorld)
    {
        return Err(V3GenerationError::RecipeContract(
            "Sandy Islets requires a whole-world Coastal patch without overlays".to_owned(),
        ));
    }
    let V3RecipeSettings::SandyIslets(recipe) = &patch.recipe else {
        return Err(V3GenerationError::RecipeUnavailable("SandyIslets"));
    };
    if recipe.sea_level != REQUIRED_SEA_LEVEL
        || !(18..=40).contains(&recipe.land_coverage_percent)
        || !(1..=9).contains(&recipe.islet_count)
        || !(1..=4).contains(&recipe.max_relief)
    {
        return Err(V3GenerationError::RecipeContract(
            "Sandy Islets settings violate the validated coastal range".to_owned(),
        ));
    }
    Ok(recipe)
}

pub(crate) fn construct_patch(
    patch: PatchRecipeContext<'_>,
    settings: &V3SandyIsletsSettings,
    environment: V3EnvironmentSettings,
    level_height: f32,
    mode: PatchBuildMode,
) -> Result<GeneratedPatchPlan, Vec<WorldValidationIssue>> {
    validate_patch_contract(patch, settings, environment, level_height)?;
    let walker_approaches = patch.walker_protected_approaches();
    let stream = mode
        .seed_streams(&patch)
        .map(|streams| streams.stage("sandy-islets.coastline"));
    let mut coast = plan_coast(
        patch.mask(),
        CoastalPlannerSettings {
            sea_level: settings.sea_level,
            land_coverage_percent: settings.land_coverage_percent,
            component_count: settings.islet_count,
            max_relief: settings.max_relief,
        },
        &walker_approaches,
        stream,
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
        CoastalSurfacePalette::AllSand,
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
    let (party, hostile) = actor_anchors(primary, &ordinary_dry)?;
    let route = shortest_surface_path(primary, &ordinary_dry, party, hostile)
        .ok_or_else(|| vec![recipe_issue("Sandy Islets primary route is disconnected")])?;
    let overlook = primary
        .iter()
        .filter_map(|coord| ordinary_dry.get(coord).copied())
        .max_by_key(|position| (position.level, position.coord, *position))
        .ok_or_else(|| vec![recipe_issue("Sandy Islets has no primary overlook")])?;
    let channel_overlook = primary
        .iter()
        .filter_map(|coord| ordinary_dry.get(coord).copied())
        .min_by_key(|position| {
            let nearest_other = coast
                .components
                .iter()
                .filter(|component| *component != primary)
                .flat_map(BTreeSet::iter)
                .map(|other| position.coord.distance(*other))
                .min()
                .unwrap_or(u32::MAX);
            (nearest_other, position.level, *position)
        })
        .ok_or_else(|| vec![recipe_issue("Sandy Islets has no channel overlook")])?;
    let route_surfaces = route.iter().copied().collect();
    let features = FeaturePlan {
        by_id: BTreeMap::new(),
        protected_routes: BTreeMap::from([(
            PRIMARY_ROUTE.to_owned(),
            ProtectedFeatureRoute {
                centerline: route,
                surfaces: route_surfaces,
            },
        )]),
        clearings: BTreeMap::new(),
    };
    let anchors = BTreeMap::from([
        (PARTY_START.to_owned(), party),
        (HOSTILE_START.to_owned(), hostile),
        (PRIMARY_OVERLOOK.to_owned(), overlook),
        (CHANNEL_OVERLOOK.to_owned(), channel_overlook),
    ]);
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
        blockers: BTreeSet::new(),
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

pub(crate) fn validate_patch(
    patch: PatchRecipeContext<'_>,
    settings: &V3SandyIsletsSettings,
    plan: &GeneratedPatchPlan,
) -> WorldValidation<SandyIsletsMetrics> {
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
    settings: &V3SandyIsletsSettings,
) -> WorldValidation<SandyIsletsMetrics> {
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
    settings: &V3SandyIsletsSettings,
    allowed_dry_boundary: &BTreeSet<HexCoord>,
) -> WorldValidation<SandyIsletsMetrics> {
    let mut issues = Vec::new();
    if !features.by_id.is_empty()
        || !features.clearings.is_empty()
        || !structures.by_id.is_empty()
        || !blockers.is_empty()
        || !lights.is_empty()
        || !interiors.by_id.is_empty()
    {
        issues.push(recipe_issue(
            "Sandy Islets must not publish objects, clearings, structures, blockers, lights, or interiors",
        ));
    }
    let expected_anchor_names = BTreeSet::from([
        PARTY_START,
        HOSTILE_START,
        PRIMARY_OVERLOOK,
        CHANNEL_OVERLOOK,
    ]);
    let actual_anchor_names = anchors.keys().map(String::as_str).collect::<BTreeSet<_>>();
    if actual_anchor_names != expected_anchor_names {
        issues.push(recipe_issue(format!(
            "Sandy Islets requires exactly anchors {expected_anchor_names:?}"
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
    if dry.len() != target {
        issues.push(recipe_issue(format!(
            "Sandy Islets requires {target} dry surfaces, got {}",
            dry.len()
        )));
    }
    for position in dry.values().copied() {
        if position.level <= settings.sea_level
            || surface_material(volume, position) != Some(super::volume::SolidMaterialRole::Sand)
        {
            issues.push(recipe_issue(format!(
                "Sandy Islets dry surface {position:?} must be sand above sea level"
            )));
        }
    }
    validate_water(volume, liquids, &water, settings.sea_level, &mut issues);
    let components = connected_components(&dry_coords);
    if components.len() != usize::from(settings.islet_count) {
        issues.push(recipe_issue(format!(
            "Sandy Islets requires {} dry components, got {}",
            settings.islet_count,
            components.len()
        )));
    }
    let party = anchors.get(PARTY_START).copied();
    let hostile = anchors.get(HOSTILE_START).copied();
    let overlook = anchors.get(PRIMARY_OVERLOOK).copied();
    let channel_overlook = anchors.get(CHANNEL_OVERLOOK).copied();
    let graph = OrdinaryGraph::from_volume(volume, Some(blockers));
    let primary_reachable = party.map_or_else(BTreeMap::new, |party| graph.distances_from(party));
    if party.is_none_or(|position| !dry.values().any(|surface| *surface == position))
        || hostile.is_none_or(|position| !primary_reachable.contains_key(&position))
        || overlook.is_none_or(|position| !primary_reachable.contains_key(&position))
        || channel_overlook.is_none_or(|position| !primary_reachable.contains_key(&position))
    {
        issues.push(recipe_issue(
            "Sandy Islets anchors must lie on one playable primary component",
        ));
    }
    let playable_dry = volume
        .surfaces
        .iter()
        .filter_map(|(position, metadata)| {
            (metadata.access == SurfaceAccess::Ordinary && dry_coords.contains(&position.coord))
                .then_some(position.coord)
        })
        .collect::<BTreeSet<_>>();
    if components.first().is_some_and(|largest| {
        largest.intersection(&playable_dry).count() != primary_reachable.len()
    }) {
        issues.push(recipe_issue(
            "Sandy Islets playable primary component must be uniquely largest",
        ));
    }
    let critical_route_steps = hostile
        .and_then(|position| primary_reachable.get(&position).copied())
        .unwrap_or_default();
    validate_route(features, party, hostile, &graph, &mut issues);
    let inward = inward_distances(&dry_coords);
    let fringe = inward
        .iter()
        .filter(|(_, distance)| **distance <= SAND_FRINGE_WIDTH)
        .count();
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
            "Sandy Islets highest dry rise must be {}, got {}",
            settings.max_relief,
            highest.saturating_sub(settings.sea_level)
        )));
    }
    let reachable_levels = primary_reachable
        .keys()
        .map(|position| position.level)
        .collect::<BTreeSet<_>>();
    let metrics = SandyIsletsMetrics {
        world_columns: count_u32(volume.mask.len()),
        land_surfaces: count_u32(dry.len()),
        water_cells: count_u32(water.len()),
        land_components: u8::try_from(components.len()).unwrap_or(u8::MAX),
        primary_reachable_surfaces: count_u32(primary_reachable.len()),
        sand_fringe_surfaces: count_u32(fringe),
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
            "Sandy Islets water nodes must exactly cover every non-land column at level 8",
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
                "Sandy Islets water column {coord:?} lacks its exact nonstandable seabed"
            )));
        }
    }
}

fn validate_route(
    features: &FeaturePlan,
    party: Option<TilePos>,
    hostile: Option<TilePos>,
    graph: &OrdinaryGraph,
    issues: &mut Vec<WorldValidationIssue>,
) {
    let route = features.protected_routes.get(PRIMARY_ROUTE);
    if features.protected_routes.len() != 1 || route.is_none() {
        issues.push(recipe_issue(
            "Sandy Islets requires exactly one primary protected route",
        ));
        return;
    }
    let Some(route) = route else {
        return;
    };
    if route.centerline.first().copied() != party
        || route.centerline.last().copied() != hostile
        || route.centerline.iter().copied().collect::<BTreeSet<_>>() != route.surfaces
        || route
            .centerline
            .windows(2)
            .any(|pair| !matches!(pair, [from, to] if graph.admits(*from, *to)))
    {
        issues.push(recipe_issue(
            "Sandy Islets protected route must exactly and continuously join its actor anchors",
        ));
    }
}

fn validate_patch_contract(
    patch: PatchRecipeContext<'_>,
    settings: &V3SandyIsletsSettings,
    environment: V3EnvironmentSettings,
    level_height: f32,
) -> Result<(), Vec<WorldValidationIssue>> {
    if environment != V3EnvironmentSettings::Coastal
        || settings.sea_level != REQUIRED_SEA_LEVEL
        || !(18..=40).contains(&settings.land_coverage_percent)
        || !(1..=9).contains(&settings.islet_count)
        || !(1..=4).contains(&settings.max_relief)
        || !level_height.is_finite()
        || level_height <= 0.0
        || patch.mask().len() < 127
    {
        return Err(vec![recipe_issue(
            "Sandy Islets patch violates its Coastal settings or capacity contract",
        )]);
    }
    Ok(())
}

fn actor_anchors(
    primary: &BTreeSet<HexCoord>,
    surfaces: &BTreeMap<HexCoord, TilePos>,
) -> Result<(TilePos, TilePos), Vec<WorldValidationIssue>> {
    let party = primary
        .iter()
        .filter_map(|coord| surfaces.get(coord).copied())
        .min_by_key(|position| (position.coord.x(), position.coord.y(), *position))
        .ok_or_else(|| vec![recipe_issue("Sandy Islets primary has no party surface")])?;
    let hostile = primary
        .iter()
        .filter_map(|coord| surfaces.get(coord).copied())
        .max_by_key(|position| {
            (
                party.coord.distance(position.coord),
                position.level,
                position.coord,
            )
        })
        .ok_or_else(|| vec![recipe_issue("Sandy Islets primary has no hostile surface")])?;
    Ok((party, hostile))
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
        .ok_or_else(|| vec![recipe_issue("Sandy Islets camera hint is invalid")])
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
    WorldValidationIssue::new(WorldIssueCode::Recipe(RECIPE), detail)
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
                environment: V3EnvironmentSettings::Coastal,
                recipe: V3RecipeSettings::SandyIslets(V3SandyIsletsSettings {
                    sea_level: 8,
                    land_coverage_percent: 32,
                    islet_count: 5,
                    max_relief: 3,
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
    fn focused_recipe_builds_five_deterministic_playable_islets() {
        let first = generate(24, 0.4, &settings(), 0x51a7).expect("Sandy Islets should generate");
        let second = generate(24, 0.4, &settings(), 0x51a7).expect("Sandy Islets should repeat");

        assert_eq!(
            first.validated.semantic_fingerprint,
            second.validated.semantic_fingerprint
        );
        assert_eq!(first.metrics.land_components, 5);
        assert_eq!(first.metrics.land_surfaces, 576);
        assert!(first.metrics.primary_reachable_surfaces > 100);
        assert_eq!(first.metrics.relief, 2);
    }

    #[test]
    fn standalone_radius_is_strict() {
        assert!(matches!(
            generate(23, 0.4, &settings(), 1),
            Err(V3GenerationError::RecipeContract(_))
        ));
    }
}
