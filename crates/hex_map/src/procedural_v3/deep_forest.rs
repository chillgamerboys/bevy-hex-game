//! Pure semantic Deep Forest recipe for procedural generator V3.
//!
//! Deep Forest shares authored tree projection and rolling-ground semantics with
//! the other vegetation recipes, but owns its dense blocker coverage, winding
//! protected trail, and three irregular clearings.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use hex_assets::{HexObjectRotation, RuntimeArtCatalog};
use hex_core::{HexCoord, TilePos};
use xxhash_rust::xxh3::xxh3_64;

use super::composition::{compose_single_patch, GeneratedPatchPlan};
use super::layout::{
    resolve_layout, HexSide, LayoutKind, PatchId, ResolvedEdgeReference, ResolvedLayoutPlan,
    ResolvedPatch,
};
use super::local_frame::LocalPatchFrame;
use super::patch::{PatchBuildMode, PatchRecipeContext};
use super::seam::{shape_walker_seams, validate_patch_walker_seams};
use super::seed::SeedStream;
use super::selection::{
    run_recipe, CandidateAttemptError, CandidateContext, FallbackContext, RepairOutcome, V3Recipe,
    ValidatedWorldSelection, WorldValidation,
};
use super::traversal::OrdinaryGraph;
use super::vegetation::{
    TemperateTreeSet, VegetationObjectSpec, OLD_GROWTH_ID, SMALL_BROADLEAF_ID, TALL_NARROW_ID,
};
use super::vegetation_landform::{actor_anchors, grassland_column, rolling_levels, view_hint};
use super::volume::{SurfaceAccess, SurfaceMetadata, VolumePlan};
use super::world::{
    FeatureClearing, FeatureId, FeatureKind, FeaturePlan, GeneratedWorldPlan, InteriorPlan,
    PlannedFeature, ProtectedFeatureRoute, StructurePlan, WorldIssueCode, WorldValidationIssue,
};
use super::V3GenerationError;
use crate::procedural::DeepForestMetrics;
use crate::settings::{
    ProceduralV3Settings, V3DeepForestSettings, V3EnvironmentSettings, V3LayoutSettings,
    V3RecipeSettings,
};

const TRAIL: &str = "deep_forest_trail";
const PARTY_START: &str = "party_start";
const HOSTILE_START: &str = "hostile_start";
const CLEARING_ANCHOR: &str = "deep_forest_clearing";
const CLEARING_PREFIX: &str = "deep_forest_clearing_";
const MINIMUM_TRAIL_TURNS: usize = 4;
const CLEARING_SURFACES: usize = 10;

#[derive(Debug)]
struct DeepForestRecipe {
    level_height: f32,
    layout: ResolvedLayoutPlan,
    trees: TemperateTreeSet,
    #[cfg(test)]
    reject_candidates: bool,
}

#[derive(Debug, Clone, Copy)]
struct DeepForestStreams<'a> {
    landform: SeedStream<'a>,
    trail: SeedStream<'a>,
    clearings: SeedStream<'a>,
    trees: SeedStream<'a>,
    objects: SeedStream<'a>,
    rotations: SeedStream<'a>,
}

#[derive(Debug)]
struct PlannedTrail {
    centerline: Vec<TilePos>,
    surfaces: BTreeSet<TilePos>,
}

#[derive(Debug)]
struct ProjectedTree {
    blockers: BTreeSet<TilePos>,
    structural_cells: BTreeSet<TilePos>,
}

/// Runs the common eight-candidate V3 selector for one Deep Forest world.
pub(crate) fn generate(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV3Settings,
    seed: u64,
    catalog: &RuntimeArtCatalog,
) -> Result<ValidatedWorldSelection<DeepForestMetrics>, V3GenerationError> {
    if !level_height.is_finite() || level_height <= 0.0 {
        return Err(V3GenerationError::RecipeContract(
            "Deep Forest level height must be positive and finite".to_owned(),
        ));
    }
    validate_recipe_settings(settings, grid_radius)?;
    let layout = resolve_layout(grid_radius, settings)
        .map_err(|error| V3GenerationError::RecipeContract(error.to_string()))?;
    if layout.footprint.len() < 127 {
        return Err(V3GenerationError::RecipeContract(
            "Deep Forest requires at least 127 connected columns".to_owned(),
        ));
    }
    let trees = TemperateTreeSet::resolve(catalog, "Deep Forest")
        .map_err(V3GenerationError::RecipeContract)?;
    run_recipe(
        &DeepForestRecipe {
            level_height,
            layout,
            trees,
            #[cfg(test)]
            reject_candidates: false,
        },
        settings,
        grid_radius,
        seed,
    )
}

impl V3Recipe for DeepForestRecipe {
    type Settings = ProceduralV3Settings;
    type Metrics = DeepForestMetrics;
    type Score = (u32, u32, u8);

    fn construct(
        &self,
        context: CandidateContext,
        settings: &Self::Settings,
    ) -> Result<GeneratedWorldPlan, CandidateAttemptError> {
        #[cfg(test)]
        if self.reject_candidates {
            return Err(CandidateAttemptError::Rejected(vec![recipe_issue(
                "forced candidate rejection",
            )]));
        }
        let recipe_settings = validate_recipe_settings(settings, context.grid_radius)
            .map_err(CandidateAttemptError::Fatal)?;
        if context.grid_radius != self.layout.grid_radius {
            return Err(CandidateAttemptError::Fatal(
                V3GenerationError::RecipeContract(
                    "Deep Forest candidate radius disagrees with its resolved layout".to_owned(),
                ),
            ));
        }
        let patch = PatchRecipeContext::resolve(&self.layout, PatchId(0))
            .map_err(CandidateAttemptError::Fatal)?;
        let fragment = construct_patch_with_trees(
            patch,
            recipe_settings,
            V3EnvironmentSettings::TemperateGrassland,
            self.level_height,
            PatchBuildMode::Candidate {
                world_seed: context.seed,
                candidate: context.candidate,
            },
            &self.trees,
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
        let Ok(recipe_settings) = validate_recipe_settings(settings, plan.layout.grid_radius)
        else {
            return WorldValidation::Invalid(vec![recipe_issue(
                "Deep Forest settings changed after construction",
            )]);
        };
        validate_deep_forest(plan, recipe_settings, &self.trees, &BTreeSet::new())
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
                V3RecipeSettings::DeepForest(settings) => {
                    u32::from(settings.blocker_coverage_percent)
                }
                _ => 0,
            },
            V3LayoutSettings::Ring7(_) | V3LayoutSettings::Ring19(_) => 0,
        };
        (
            metrics.blocker_coverage_percent.abs_diff(target),
            metrics.relief.abs_diff(target_relief(settings)),
            candidate,
        )
    }

    fn canonical_fallback(
        &self,
        context: FallbackContext,
        settings: &Self::Settings,
    ) -> Result<GeneratedWorldPlan, V3GenerationError> {
        let recipe_settings = validate_recipe_settings(settings, context.grid_radius)?;
        if context.grid_radius != self.layout.grid_radius {
            return Err(V3GenerationError::RecipeContract(
                "Deep Forest fallback radius disagrees with its resolved layout".to_owned(),
            ));
        }
        let patch = PatchRecipeContext::resolve(&self.layout, PatchId(0))?;
        let fragment = construct_patch_with_trees(
            patch,
            recipe_settings,
            V3EnvironmentSettings::TemperateGrassland,
            self.level_height,
            PatchBuildMode::CanonicalFallback,
            &self.trees,
        )
        .map_err(recipe_issues_to_error)?;
        compose_single_patch(self.layout.clone(), fragment)
            .map_err(|error| V3GenerationError::RecipeContract(format!("{error:?}")))
    }
}

fn target_relief(settings: &ProceduralV3Settings) -> i32 {
    match &settings.layout {
        V3LayoutSettings::Single(patch) => match &patch.recipe {
            V3RecipeSettings::DeepForest(settings) => settings.max_relief,
            _ => 0,
        },
        V3LayoutSettings::Ring7(_) | V3LayoutSettings::Ring19(_) => 0,
    }
}

fn validate_recipe_settings(
    settings: &ProceduralV3Settings,
    grid_radius: u32,
) -> Result<&V3DeepForestSettings, V3GenerationError> {
    let V3LayoutSettings::Single(patch) = &settings.layout else {
        return Err(V3GenerationError::RecipeUnavailable("Ring19"));
    };
    if patch.environment != V3EnvironmentSettings::TemperateGrassland {
        return Err(V3GenerationError::RecipeContract(
            "Deep Forest requires the TemperateGrassland environment".to_owned(),
        ));
    }
    let V3RecipeSettings::DeepForest(recipe) = &patch.recipe else {
        return Err(V3GenerationError::RecipeUnavailable(recipe_name(
            &patch.recipe,
        )));
    };
    if !patch.overlays.is_empty() {
        return Err(V3GenerationError::RecipeContract(
            "Deep Forest overlays are not implemented yet".to_owned(),
        ));
    }
    if !(12..=55).contains(&grid_radius)
        || recipe.base_level < 5
        || !(1..=12).contains(&recipe.max_relief)
        || !(28..=32).contains(&recipe.blocker_coverage_percent)
        || recipe.clearing_count != 3
        || recipe
            .base_level
            .checked_add(recipe.max_relief)
            .is_none_or(|highest| highest > 96)
    {
        return Err(V3GenerationError::RecipeContract(
            "Deep Forest settings violate the validated V3 vegetation-landform range".to_owned(),
        ));
    }
    Ok(recipe)
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "patch entry point is consumed when Ring19 composition integrates"
    )
)]
pub(crate) fn construct_patch(
    patch: PatchRecipeContext<'_>,
    settings: &V3DeepForestSettings,
    environment: V3EnvironmentSettings,
    level_height: f32,
    mode: PatchBuildMode,
    catalog: &RuntimeArtCatalog,
) -> Result<GeneratedPatchPlan, Vec<WorldValidationIssue>> {
    let trees = TemperateTreeSet::resolve(catalog, "Deep Forest")
        .map_err(|error| vec![recipe_issue(error)])?;
    construct_patch_with_trees(patch, settings, environment, level_height, mode, &trees)
}

fn construct_patch_with_trees(
    patch: PatchRecipeContext<'_>,
    settings: &V3DeepForestSettings,
    environment: V3EnvironmentSettings,
    level_height: f32,
    mode: PatchBuildMode,
    trees: &TemperateTreeSet,
) -> Result<GeneratedPatchPlan, Vec<WorldValidationIssue>> {
    if environment != V3EnvironmentSettings::TemperateGrassland {
        return Err(vec![recipe_issue(
            "Deep Forest requires the TemperateGrassland environment",
        )]);
    }
    let frame = LocalPatchFrame::resolve(patch.mask(), patch.layout().kind, patch.grid_radius())
        .map_err(|error| vec![recipe_issue(error)])?;
    let mask = frame
        .local_mask(patch.mask())
        .map_err(|error| vec![recipe_issue(error)])?;
    let streams = mode.seed_streams(&patch).map(|streams| DeepForestStreams {
        landform: streams.stage("deep-forest.landform"),
        trail: streams.stage("deep-forest.trail"),
        clearings: streams.stage("deep-forest.clearings"),
        trees: streams.stage("deep-forest.trees"),
        objects: streams.stage("deep-forest.objects"),
        rotations: streams.stage("deep-forest.rotations"),
    });
    let local_levels = rolling_levels(
        &mask,
        settings.base_level,
        settings.max_relief,
        streams.map(|streams| streams.landform),
        "deep forest",
    )?;
    let mut world_levels = frame
        .levels_to_world(local_levels)
        .map_err(|error| vec![recipe_issue(error)])?;
    let seam_shape = shape_walker_seams(&patch, &mut world_levels)?;
    let local_levels = frame
        .levels_to_local(world_levels)
        .map_err(|error| vec![recipe_issue(error)])?;

    let mut surfaces = BTreeMap::new();
    let mut surface_by_coord = BTreeMap::new();
    let mut ordinary_by_coord = BTreeMap::new();
    for coord in &mask {
        let level = local_levels.get(coord).copied().ok_or_else(|| {
            vec![recipe_issue(format!(
                "Deep Forest land plan omitted coordinate {coord:?}"
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
        surface_by_coord.insert(*coord, position);
        if access == SurfaceAccess::Ordinary {
            ordinary_by_coord.insert(*coord, position);
        }
    }
    let (party_start, hostile_start) = actor_anchors(&ordinary_by_coord, "deep forest")?;
    let trail = plan_trail(
        &ordinary_by_coord,
        party_start,
        hostile_start,
        streams.map(|streams| streams.trail),
    )?;
    let protected_approaches = patch
        .protected_approaches()
        .into_iter()
        .map(|coord| frame.to_local(coord).map_err(recipe_issue))
        .collect::<Result<BTreeSet<_>, _>>()
        .map_err(|issue| vec![issue])?;
    let clearings = plan_clearings(
        &ordinary_by_coord,
        &trail.surfaces,
        &protected_approaches,
        [party_start, hostile_start],
        settings.clearing_count,
        streams.map(|streams| streams.clearings),
    )?;
    let clearing_anchor = clearings
        .values()
        .next()
        .and_then(|clearing| clearing.surfaces.iter().next())
        .copied()
        .ok_or_else(|| vec![recipe_issue("Deep Forest has no primary clearing anchor")])?;
    let anchors = BTreeMap::from([
        (PARTY_START.to_owned(), party_start),
        (HOSTILE_START.to_owned(), hostile_start),
        (CLEARING_ANCHOR.to_owned(), clearing_anchor),
    ]);
    let eligible = eligible_tree_surfaces(
        &ordinary_by_coord,
        &trail.surfaces,
        &clearings,
        anchors.values().copied(),
        &protected_approaches,
    );
    let blocker_target = eligible
        .len()
        .saturating_mul(usize::from(settings.blocker_coverage_percent))
        / 100;
    let (tree_features, blockers) = plan_trees(
        &ordinary_by_coord,
        &surface_by_coord,
        &eligible,
        blocker_target,
        trees,
        streams.map(|streams| streams.trees),
        streams.map(|streams| streams.objects),
        streams.map(|streams| streams.rotations),
    )?;

    let trail_coords = trail
        .surfaces
        .iter()
        .map(|surface| surface.coord)
        .collect::<BTreeSet<_>>();
    let columns = surface_by_coord
        .iter()
        .map(|(coord, position)| {
            (
                *coord,
                grassland_column(position.level, trail_coords.contains(coord)),
            )
        })
        .collect();
    let volume = VolumePlan {
        mask: mask.clone(),
        columns,
        surfaces,
    };
    let by_id = tree_features
        .into_iter()
        .enumerate()
        .map(|(index, feature)| (FeatureId(u32::try_from(index).unwrap_or(u32::MAX)), feature))
        .collect();
    let protected_routes = BTreeMap::from([(
        TRAIL.to_owned(),
        ProtectedFeatureRoute {
            centerline: trail.centerline,
            surfaces: trail.surfaces,
        },
    )]);
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
        features: FeaturePlan {
            by_id,
            protected_routes,
            clearings,
        },
        structures: StructurePlan::default(),
        blockers,
        lights: BTreeMap::new(),
        biome_regions,
        interiors: InteriorPlan::default(),
        anchors,
        view_hint: frame.view_hint_to_world(view_hint(
            frame.scale(),
            settings.base_level,
            settings.max_relief,
            level_height,
            "deep forest",
        )?),
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

fn plan_trail(
    ordinary: &BTreeMap<HexCoord, TilePos>,
    party: TilePos,
    hostile: TilePos,
    stream: Option<SeedStream<'_>>,
) -> Result<PlannedTrail, Vec<WorldValidationIssue>> {
    let radius = ordinary
        .keys()
        .map(|coord| coord.distance(HexCoord::ORIGIN))
        .max()
        .and_then(|distance| i32::try_from(distance).ok())
        .unwrap_or_default();
    let targets = [
        HexCoord::from_axial(-(radius / 2), radius / 3),
        HexCoord::from_axial(0, -(radius / 3)),
        HexCoord::from_axial(radius / 2, radius / 4),
    ];
    let waypoints = targets
        .into_iter()
        .map(|target| {
            ordinary
                .values()
                .copied()
                .min_by_key(|surface| {
                    (
                        surface.coord.distance(target),
                        feature_priority(stream, surface.coord, 100),
                        *surface,
                    )
                })
                .ok_or_else(|| vec![recipe_issue("Deep Forest trail has no waypoint surface")])
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut centerline = Vec::new();
    let points = std::iter::once(party)
        .chain(waypoints)
        .chain(std::iter::once(hostile))
        .collect::<Vec<_>>();
    for (index, pair) in points.windows(2).enumerate() {
        let [start, end] = pair else {
            continue;
        };
        let segment = shortest_path(
            ordinary,
            *start,
            *end,
            stream,
            u64::try_from(index).unwrap_or_default(),
        )
        .ok_or_else(|| {
            vec![recipe_issue(format!(
                "Deep Forest trail cannot connect waypoint {index}"
            ))]
        })?;
        if centerline.last() == segment.first() {
            centerline.extend(segment.into_iter().skip(1));
        } else {
            centerline.extend(segment);
        }
    }
    centerline = erase_route_loops(centerline);
    if centerline.first() != Some(&party) || centerline.last() != Some(&hostile) {
        return Err(vec![recipe_issue(
            "Deep Forest trail does not join both actor anchors",
        )]);
    }
    let turns = trail_turns(&centerline);
    if turns < MINIMUM_TRAIL_TURNS {
        return Err(vec![recipe_issue(format!(
            "Deep Forest trail has only {turns} bends"
        ))]);
    }
    let surfaces = centerline.iter().copied().collect();
    Ok(PlannedTrail {
        centerline,
        surfaces,
    })
}

fn erase_route_loops(route: Vec<TilePos>) -> Vec<TilePos> {
    let mut simple = Vec::with_capacity(route.len());
    let mut indices = BTreeMap::<TilePos, usize>::new();
    for position in route {
        if let Some(index) = indices.get(&position).copied() {
            simple.truncate(index.saturating_add(1));
            indices.retain(|_, current| *current <= index);
            continue;
        }
        indices.insert(position, simple.len());
        simple.push(position);
    }
    simple
}

fn shortest_path(
    surfaces: &BTreeMap<HexCoord, TilePos>,
    start: TilePos,
    goal: TilePos,
    stream: Option<SeedStream<'_>>,
    salt: u64,
) -> Option<Vec<TilePos>> {
    if start == goal {
        return Some(vec![start]);
    }
    let mut parents = BTreeMap::new();
    let mut visited = BTreeSet::from([start]);
    let mut frontier = VecDeque::from([start]);
    while let Some(position) = frontier.pop_front() {
        let mut neighbors = position
            .coord
            .neighbors()
            .into_iter()
            .filter_map(|coord| surfaces.get(&coord).copied())
            .filter(|neighbor| position.level.abs_diff(neighbor.level) <= 1)
            .collect::<Vec<_>>();
        neighbors.sort_unstable_by_key(|neighbor| {
            (
                feature_priority(stream, neighbor.coord, salt.saturating_add(200)),
                *neighbor,
            )
        });
        for neighbor in neighbors {
            if !visited.insert(neighbor) {
                continue;
            }
            parents.insert(neighbor, position);
            if neighbor == goal {
                return rebuild_path(start, goal, &parents);
            }
            frontier.push_back(neighbor);
        }
    }
    None
}

fn rebuild_path(
    start: TilePos,
    goal: TilePos,
    parents: &BTreeMap<TilePos, TilePos>,
) -> Option<Vec<TilePos>> {
    let mut reversed = vec![goal];
    let mut cursor = goal;
    while cursor != start {
        cursor = parents.get(&cursor).copied()?;
        reversed.push(cursor);
    }
    reversed.reverse();
    Some(reversed)
}

fn trail_turns(centerline: &[TilePos]) -> usize {
    let directions = centerline
        .windows(2)
        .filter_map(|pair| match pair {
            [from, to] => Some((
                to.coord.x().saturating_sub(from.coord.x()),
                to.coord.y().saturating_sub(from.coord.y()),
                to.coord.z().saturating_sub(from.coord.z()),
            )),
            _ => None,
        })
        .collect::<Vec<_>>();
    directions
        .windows(2)
        .filter(|pair| matches!(pair, [first, second] if first != second))
        .count()
}

fn plan_clearings(
    ordinary: &BTreeMap<HexCoord, TilePos>,
    trail: &BTreeSet<TilePos>,
    protected_approaches: &BTreeSet<HexCoord>,
    anchors: impl IntoIterator<Item = TilePos>,
    count: u8,
    stream: Option<SeedStream<'_>>,
) -> Result<BTreeMap<String, FeatureClearing>, Vec<WorldValidationIssue>> {
    let radius = ordinary
        .keys()
        .map(|coord| coord.distance(HexCoord::ORIGIN))
        .max()
        .and_then(|distance| i32::try_from(distance).ok())
        .unwrap_or_default();
    let targets = [
        HexCoord::from_axial(-(radius / 3), -(radius / 3)),
        HexCoord::from_axial(0, radius / 2),
        HexCoord::from_axial(radius / 3, -(radius / 4)),
    ];
    let mut forbidden = protected_approaches.clone();
    for surface in trail {
        forbidden.extend(surface.coord.within_radius(1));
    }
    for anchor in anchors {
        forbidden.extend(anchor.coord.within_radius(1));
    }
    let mut claimed = BTreeSet::new();
    let mut clearings = BTreeMap::new();
    for index in 0..usize::from(count) {
        let target = targets.get(index).copied().unwrap_or(HexCoord::ORIGIN);
        let mut candidates = ordinary.keys().copied().collect::<Vec<_>>();
        candidates.sort_unstable_by_key(|coord| {
            (
                coord.distance(target),
                feature_priority(
                    stream,
                    *coord,
                    300_u64.saturating_add(u64::try_from(index).unwrap_or_default()),
                ),
                *coord,
            )
        });
        let Some(coords) = candidates.into_iter().find_map(|center| {
            clearing_footprint(center, ordinary, &forbidden, &claimed, stream, index)
        }) else {
            return Err(vec![recipe_issue(format!(
                "Deep Forest clearing {index} cannot fit {CLEARING_SURFACES} irregular surfaces"
            ))]);
        };
        let surfaces = coords
            .iter()
            .filter_map(|coord| ordinary.get(coord).copied())
            .collect::<BTreeSet<_>>();
        if surfaces.len() != CLEARING_SURFACES {
            return Err(vec![recipe_issue(format!(
                "Deep Forest clearing {index} lost authored surfaces"
            ))]);
        }
        claimed.extend(coords);
        clearings.insert(
            format!("{CLEARING_PREFIX}{index}"),
            FeatureClearing { surfaces },
        );
    }
    Ok(clearings)
}

fn clearing_footprint(
    center: HexCoord,
    ordinary: &BTreeMap<HexCoord, TilePos>,
    forbidden: &BTreeSet<HexCoord>,
    claimed: &BTreeSet<HexCoord>,
    stream: Option<SeedStream<'_>>,
    index: usize,
) -> Option<BTreeSet<HexCoord>> {
    let core = center.within_radius(1).into_iter().collect::<BTreeSet<_>>();
    if core.iter().any(|coord| {
        !ordinary.contains_key(coord) || forbidden.contains(coord) || claimed.contains(coord)
    }) {
        return None;
    }
    let mut footprint = core;
    let radius_two = center.within_radius(2).into_iter().collect::<BTreeSet<_>>();
    let mut edge = radius_two
        .difference(&footprint)
        .copied()
        .filter(|coord| {
            ordinary.contains_key(coord) && !forbidden.contains(coord) && !claimed.contains(coord)
        })
        .collect::<Vec<_>>();
    edge.sort_unstable_by_key(|coord| {
        (
            feature_priority(
                stream,
                *coord,
                400_u64.saturating_add(u64::try_from(index).unwrap_or_default()),
            ),
            *coord,
        )
    });
    footprint.extend(
        edge.into_iter()
            .take(CLEARING_SURFACES.saturating_sub(footprint.len())),
    );
    (footprint.len() == CLEARING_SURFACES).then_some(footprint)
}

fn eligible_tree_surfaces(
    ordinary: &BTreeMap<HexCoord, TilePos>,
    trail: &BTreeSet<TilePos>,
    clearings: &BTreeMap<String, FeatureClearing>,
    anchors: impl IntoIterator<Item = TilePos>,
    protected_approaches: &BTreeSet<HexCoord>,
) -> BTreeSet<TilePos> {
    let mut excluded = protected_approaches.clone();
    excluded.extend(trail.iter().map(|surface| surface.coord));
    excluded.extend(
        clearings
            .values()
            .flat_map(|clearing| clearing.surfaces.iter())
            .map(|surface| surface.coord),
    );
    for anchor in anchors {
        excluded.extend(anchor.coord.within_radius(1));
    }
    ordinary
        .iter()
        .filter_map(|(coord, position)| (!excluded.contains(coord)).then_some(*position))
        .collect()
}

#[expect(
    clippy::too_many_arguments,
    reason = "tree placement keeps each independent authored spatial input explicit"
)]
fn plan_trees(
    ordinary: &BTreeMap<HexCoord, TilePos>,
    all_surfaces: &BTreeMap<HexCoord, TilePos>,
    eligible: &BTreeSet<TilePos>,
    target_blockers: usize,
    trees: &TemperateTreeSet,
    tree_stream: Option<SeedStream<'_>>,
    object_stream: Option<SeedStream<'_>>,
    rotation_stream: Option<SeedStream<'_>>,
) -> Result<(Vec<PlannedFeature>, BTreeSet<TilePos>), Vec<WorldValidationIssue>> {
    if target_blockers < 9 {
        return Err(vec![recipe_issue(
            "Deep Forest footprint cannot fit all three authored tree forms",
        )]);
    }
    let eligible_coords = eligible
        .iter()
        .map(|surface| surface.coord)
        .collect::<BTreeSet<_>>();
    let mut exclusions = ordinary
        .keys()
        .filter(|coord| !eligible_coords.contains(coord))
        .copied()
        .collect::<BTreeSet<_>>();
    exclusions.extend(
        all_surfaces
            .keys()
            .filter(|coord| !ordinary.contains_key(coord))
            .copied(),
    );
    let mut occupied_blockers = BTreeSet::new();
    let mut occupied_structural = BTreeSet::new();
    let mut occupied_roots = BTreeSet::new();
    let mut features = Vec::new();

    let mut old_growth_candidates = eligible.iter().copied().collect::<Vec<_>>();
    old_growth_candidates
        .sort_unstable_by_key(|root| (feature_priority(tree_stream, root.coord, 500), *root));
    let old_growth_target = target_blockers.saturating_div(40).clamp(1, 3);
    for root in old_growth_candidates {
        if features
            .iter()
            .filter(|feature: &&PlannedFeature| feature.object_id.as_str() == OLD_GROWTH_ID)
            .count()
            >= old_growth_target
        {
            break;
        }
        if target_blockers.saturating_sub(occupied_blockers.len()) < 7 {
            break;
        }
        let Some((rotation, projected)) = select_projection(
            &trees.old_growth,
            root,
            eligible,
            all_surfaces,
            &exclusions,
            &occupied_blockers,
            &occupied_structural,
            rotation_stream,
            601,
        )?
        else {
            continue;
        };
        let mut trial = occupied_blockers.clone();
        trial.extend(projected.blockers.iter().copied());
        if !unblocked_connected(ordinary, &trial) {
            continue;
        }
        occupied_blockers = trial;
        occupied_structural.extend(projected.structural_cells);
        occupied_roots.insert(root.coord);
        features.push(PlannedFeature {
            root,
            kind: FeatureKind::Tree,
            object_id: trees.old_growth.id.clone(),
            rotation,
            blocker_footprint: projected.blockers,
        });
    }
    if !features
        .iter()
        .any(|feature| feature.object_id.as_str() == OLD_GROWTH_ID)
    {
        return Err(vec![recipe_issue(
            "Deep Forest cannot place its old-growth anchor tree",
        )]);
    }

    let phase = (0..3_i32)
        .max_by_key(|phase| {
            eligible
                .iter()
                .filter(|surface| tree_color(surface.coord) == *phase)
                .count()
        })
        .unwrap_or_default();
    let mut candidates = eligible.iter().copied().collect::<Vec<_>>();
    candidates.sort_unstable_by_key(|root| {
        (
            tree_color(root.coord) != phase,
            feature_priority(tree_stream, root.coord, 700),
            *root,
        )
    });
    for root in candidates {
        if occupied_blockers.len() >= target_blockers {
            break;
        }
        if occupied_blockers.contains(&root) || occupied_roots.contains(&root.coord) {
            continue;
        }
        let prefer_tall = feature_priority(object_stream, root.coord, 701).is_multiple_of(3);
        let choices = if prefer_tall {
            [&trees.tall_narrow, &trees.small_broadleaf]
        } else {
            [&trees.small_broadleaf, &trees.tall_narrow]
        };
        let mut selected = None;
        for object in choices {
            let projection = select_projection(
                object,
                root,
                eligible,
                all_surfaces,
                &exclusions,
                &occupied_blockers,
                &occupied_structural,
                rotation_stream,
                702,
            )?;
            if let Some((rotation, projected)) = projection {
                selected = Some((object, rotation, projected));
                break;
            }
        }
        let Some((object, rotation, projected)) = selected else {
            continue;
        };
        let mut trial = occupied_blockers.clone();
        trial.extend(projected.blockers.iter().copied());
        if trial.len() > target_blockers {
            continue;
        }
        occupied_blockers = trial;
        occupied_structural.extend(projected.structural_cells);
        occupied_roots.insert(root.coord);
        features.push(PlannedFeature {
            root,
            kind: FeatureKind::Tree,
            object_id: object.id.clone(),
            rotation,
            blocker_footprint: projected.blockers,
        });
    }
    if occupied_blockers.len() != target_blockers {
        return Err(vec![recipe_issue(format!(
            "Deep Forest could place {} of {target_blockers} target blocker surfaces",
            occupied_blockers.len()
        ))]);
    }
    if !unblocked_connected(ordinary, &occupied_blockers) {
        return Err(vec![recipe_issue(
            "Deep Forest authored blocker plan disconnects ordinary footing",
        )]);
    }
    if !features
        .iter()
        .any(|feature| feature.object_id.as_str() == SMALL_BROADLEAF_ID)
        || !features
            .iter()
            .any(|feature| feature.object_id.as_str() == TALL_NARROW_ID)
    {
        return Err(vec![recipe_issue(
            "Deep Forest must place small, tall, and old-growth tree forms",
        )]);
    }
    features.sort_unstable_by_key(|feature| feature.root);
    Ok((features, occupied_blockers))
}

#[expect(
    clippy::too_many_arguments,
    reason = "authored projection validates each independent spatial constraint explicitly"
)]
fn select_projection(
    object: &VegetationObjectSpec,
    root: TilePos,
    eligible: &BTreeSet<TilePos>,
    surfaces: &BTreeMap<HexCoord, TilePos>,
    exclusions: &BTreeSet<HexCoord>,
    occupied_blockers: &BTreeSet<TilePos>,
    occupied_structural: &BTreeSet<TilePos>,
    stream: Option<SeedStream<'_>>,
    salt: u64,
) -> Result<Option<(HexObjectRotation, ProjectedTree)>, Vec<WorldValidationIssue>> {
    let first = object_rotation(stream, root.coord, salt)?;
    for offset in 0..6 {
        let rotation =
            HexObjectRotation::new(first.steps().saturating_add(offset) % 6).map_err(|error| {
                vec![recipe_issue(format!(
                    "invalid Deep Forest object rotation: {error}"
                ))]
            })?;
        let Some(blockers) = object.project_blockers(root, rotation, surfaces) else {
            continue;
        };
        if !blockers.is_subset(eligible) || !blockers.is_disjoint(occupied_blockers) {
            continue;
        }
        let Some(volume) = object.project_visual_volume(root, rotation) else {
            continue;
        };
        if !volume.structural_cells.is_disjoint(occupied_structural) {
            continue;
        }
        let valid_volume = volume.cells.iter().all(|visual| {
            surfaces.get(&visual.coord).is_some_and(|support| {
                visual.level > support.level
                    && !(exclusions.contains(&visual.coord)
                        && visual.level <= support.level.saturating_add(2))
            })
        });
        if valid_volume {
            return Ok(Some((
                rotation,
                ProjectedTree {
                    blockers,
                    structural_cells: volume.structural_cells,
                },
            )));
        }
    }
    Ok(None)
}

fn unblocked_connected(
    ordinary: &BTreeMap<HexCoord, TilePos>,
    blockers: &BTreeSet<TilePos>,
) -> bool {
    let Some(start) = ordinary
        .values()
        .copied()
        .find(|surface| !blockers.contains(surface))
    else {
        return false;
    };
    let expected = ordinary
        .values()
        .filter(|surface| !blockers.contains(surface))
        .count();
    let mut visited = BTreeSet::from([start]);
    let mut frontier = VecDeque::from([start]);
    while let Some(position) = frontier.pop_front() {
        for coord in position.coord.neighbors() {
            let Some(neighbor) = ordinary.get(&coord).copied() else {
                continue;
            };
            if blockers.contains(&neighbor)
                || position.level.abs_diff(neighbor.level) > 1
                || !visited.insert(neighbor)
            {
                continue;
            }
            frontier.push_back(neighbor);
        }
    }
    visited.len() == expected
}

fn tree_color(coord: HexCoord) -> i32 {
    coord.x().saturating_sub(coord.y()).rem_euclid(3)
}

fn object_rotation(
    stream: Option<SeedStream<'_>>,
    coord: HexCoord,
    salt: u64,
) -> Result<HexObjectRotation, Vec<WorldValidationIssue>> {
    let steps = u8::try_from(feature_priority(stream, coord, salt) % 6).unwrap_or_default();
    HexObjectRotation::new(steps).map_err(|error| {
        vec![recipe_issue(format!(
            "invalid Deep Forest rotation: {error}"
        ))]
    })
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "patch validator is consumed when Ring19 composition integrates"
    )
)]
pub(crate) fn validate_patch(
    patch: PatchRecipeContext<'_>,
    settings: &V3DeepForestSettings,
    plan: &GeneratedPatchPlan,
    catalog: &RuntimeArtCatalog,
) -> WorldValidation<DeepForestMetrics> {
    let protected_approaches = patch.protected_approaches();
    let trees = match TemperateTreeSet::resolve(catalog, "Deep Forest") {
        Ok(trees) => trees,
        Err(error) => return WorldValidation::Invalid(vec![recipe_issue(error)]),
    };
    let world = isolated_patch_world(patch, plan);
    validate_deep_forest(&world, settings, &trees, &protected_approaches)
}

fn isolated_patch_world(
    patch: PatchRecipeContext<'_>,
    plan: &GeneratedPatchPlan,
) -> GeneratedWorldPlan {
    let edges = HexSide::ALL
        .into_iter()
        .map(|side| (side, ResolvedEdgeReference::WorldBoundary))
        .collect();
    let layout = ResolvedLayoutPlan {
        kind: LayoutKind::Single,
        grid_radius: patch.grid_radius(),
        footprint: plan.volume.mask.clone(),
        patches: BTreeMap::from([(
            PatchId(0),
            ResolvedPatch {
                biome_region: patch.biome_region(),
                mask: plan.volume.mask.clone(),
                edges,
            },
        )]),
        shared_edges: BTreeMap::new(),
        boundary_liquid_outlets: BTreeMap::new(),
    };
    GeneratedWorldPlan {
        layout,
        volume: plan.volume.clone(),
        liquids: plan.liquids.clone(),
        features: plan.features.clone(),
        structures: plan.structures.clone(),
        blockers: plan.blockers.clone(),
        lights: plan.lights.clone(),
        biome_regions: plan.biome_regions.clone(),
        interiors: plan.interiors.clone(),
        anchors: plan.anchors.clone(),
        view_hint: plan.view_hint,
    }
}

fn validate_deep_forest(
    plan: &GeneratedWorldPlan,
    settings: &V3DeepForestSettings,
    trees: &TemperateTreeSet,
    protected_approaches: &BTreeSet<HexCoord>,
) -> WorldValidation<DeepForestMetrics> {
    let mut issues = plan.validate();
    if !plan.liquids.bodies.is_empty()
        || !plan.structures.by_id.is_empty()
        || !plan.lights.is_empty()
        || !plan.interiors.by_id.is_empty()
    {
        issues.push(recipe_issue(
            "Deep Forest must not contain liquids, structures, lights, or interiors",
        ));
    }
    let base_graph = OrdinaryGraph::from_volume(&plan.volume, None);
    let ordinary_by_coord = base_graph
        .positions()
        .map(|position| (position.coord, position))
        .collect::<BTreeMap<_, _>>();
    let surface_by_coord = plan
        .volume
        .surfaces
        .keys()
        .map(|surface| (surface.coord, *surface))
        .collect::<BTreeMap<_, _>>();

    let expected_anchor_names = BTreeSet::from([PARTY_START, HOSTILE_START, CLEARING_ANCHOR]);
    let actual_anchor_names = plan
        .anchors
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if actual_anchor_names != expected_anchor_names {
        issues.push(recipe_issue(format!(
            "Deep Forest requires exactly anchors {expected_anchor_names:?}"
        )));
    }
    let party = plan.anchors.get(PARTY_START).copied();
    let hostile = plan.anchors.get(HOSTILE_START).copied();

    let trail = plan.features.protected_routes.get(TRAIL);
    if plan.features.protected_routes.len() != 1 || trail.is_none() {
        issues.push(recipe_issue(
            "Deep Forest requires exactly one named protected trail",
        ));
    }
    if let Some(trail) = trail {
        if trail.centerline.first().copied() != party || trail.centerline.last().copied() != hostile
        {
            issues.push(recipe_issue(
                "Deep Forest trail must join the exact actor anchors",
            ));
        }
        if trail.centerline.iter().copied().collect::<BTreeSet<_>>() != trail.surfaces {
            issues.push(recipe_issue(
                "Deep Forest trail surfaces must exactly equal its centerline",
            ));
        }
        if trail
            .centerline
            .windows(2)
            .any(|pair| !matches!(pair, [from, to] if base_graph.admits(*from, *to)))
        {
            issues.push(recipe_issue(
                "Deep Forest trail centerline is not continuously walkable",
            ));
        }
        let turns = trail_turns(&trail.centerline);
        if turns < MINIMUM_TRAIL_TURNS {
            issues.push(recipe_issue(format!(
                "Deep Forest trail requires at least {MINIMUM_TRAIL_TURNS} bends, got {turns}"
            )));
        }
    }

    let expected_clearings = (0..usize::from(settings.clearing_count))
        .map(|index| format!("{CLEARING_PREFIX}{index}"))
        .collect::<BTreeSet<_>>();
    let actual_clearings = plan
        .features
        .clearings
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    if actual_clearings != expected_clearings {
        issues.push(recipe_issue(format!(
            "Deep Forest requires exactly clearings {expected_clearings:?}"
        )));
    }
    let mut clearing_surfaces = BTreeSet::new();
    for (name, clearing) in &plan.features.clearings {
        if clearing.surfaces.len() != CLEARING_SURFACES {
            issues.push(recipe_issue(format!(
                "Deep Forest clearing {name:?} must contain exactly {CLEARING_SURFACES} surfaces"
            )));
        }
        if !surface_set_connected(&clearing.surfaces, &base_graph) {
            issues.push(recipe_issue(format!(
                "Deep Forest clearing {name:?} is not walker-connected"
            )));
        }
        if !clearing.surfaces.is_disjoint(&clearing_surfaces) {
            issues.push(recipe_issue(format!(
                "Deep Forest clearing {name:?} overlaps another clearing"
            )));
        }
        if trail.is_some_and(|trail| !clearing.surfaces.is_disjoint(&trail.surfaces)) {
            issues.push(recipe_issue(format!(
                "Deep Forest clearing {name:?} overlaps its protected trail"
            )));
        }
        clearing_surfaces.extend(clearing.surfaces.iter().copied());
    }
    if !plan
        .anchors
        .get(CLEARING_ANCHOR)
        .zip(plan.features.clearings.get(&format!("{CLEARING_PREFIX}0")))
        .is_some_and(|(anchor, clearing)| clearing.surfaces.contains(anchor))
    {
        issues.push(recipe_issue(
            "Deep Forest clearing anchor must name a surface in its first clearing",
        ));
    }

    let empty_trail = BTreeSet::new();
    let trail_surfaces = trail.map_or(&empty_trail, |trail| &trail.surfaces);
    let eligible = eligible_tree_surfaces(
        &ordinary_by_coord,
        trail_surfaces,
        &plan.features.clearings,
        plan.anchors.values().copied(),
        protected_approaches,
    );
    let expected_blockers = eligible
        .len()
        .saturating_mul(usize::from(settings.blocker_coverage_percent))
        / 100;
    let mut accepted_roots = BTreeSet::new();
    let mut authored_blockers = BTreeSet::new();
    let mut structural_cells = BTreeSet::new();
    let mut accepted_ids = BTreeSet::new();
    for feature in plan.features.by_id.values() {
        if feature.kind != FeatureKind::Tree {
            issues.push(recipe_issue(format!(
                "Deep Forest feature at {:?} is not an authored tree",
                feature.root
            )));
            continue;
        }
        let Some(object) = trees.object(feature.object_id.as_str()) else {
            issues.push(recipe_issue(format!(
                "Deep Forest feature at {:?} uses unsupported object '{}'",
                feature.root, feature.object_id
            )));
            continue;
        };
        accepted_ids.insert(feature.object_id.as_str());
        if !accepted_roots.insert(feature.root) {
            issues.push(recipe_issue(format!(
                "Deep Forest repeats a tree root at {:?}",
                feature.root
            )));
        }
        let expected = object.project_blockers(feature.root, feature.rotation, &surface_by_coord);
        if expected.as_ref() != Some(&feature.blocker_footprint) {
            issues.push(recipe_issue(format!(
                "Deep Forest tree at {:?} does not publish its exact rotated blocker footprint",
                feature.root
            )));
        }
        if !feature.blocker_footprint.is_subset(&eligible) {
            issues.push(recipe_issue(format!(
                "Deep Forest tree at {:?} enters a route, clearing, anchor, or seam reservation",
                feature.root
            )));
        }
        if !feature.blocker_footprint.is_disjoint(&authored_blockers) {
            issues.push(recipe_issue(format!(
                "Deep Forest tree at {:?} overlaps another blocker footprint",
                feature.root
            )));
        }
        authored_blockers.extend(feature.blocker_footprint.iter().copied());
        let Some(volume) = object.project_visual_volume(feature.root, feature.rotation) else {
            issues.push(recipe_issue(format!(
                "Deep Forest tree at {:?} cannot project its complete rotated authored volume",
                feature.root
            )));
            continue;
        };
        if !volume.structural_cells.is_disjoint(&structural_cells) {
            issues.push(recipe_issue(format!(
                "Deep Forest tree at {:?} overlaps neighboring woody structure",
                feature.root
            )));
        }
        structural_cells.extend(volume.structural_cells);
        for visual in volume.cells {
            let Some(support) = surface_by_coord.get(&visual.coord).copied() else {
                issues.push(recipe_issue(format!(
                    "Deep Forest tree at {:?} leaves terrain at {visual:?}",
                    feature.root
                )));
                continue;
            };
            if visual.level <= support.level {
                issues.push(recipe_issue(format!(
                    "Deep Forest tree at {:?} intersects terrain at {visual:?}",
                    feature.root
                )));
            }
            if !eligible.contains(&support) && visual.level <= support.level.saturating_add(2) {
                issues.push(recipe_issue(format!(
                    "Deep Forest tree at {:?} enters protected walker volume at {visual:?}",
                    feature.root
                )));
            }
        }
    }
    if accepted_roots.len() != plan.features.by_id.len() {
        issues.push(recipe_issue(
            "Deep Forest feature identities do not map one-to-one to unique tree roots",
        ));
    }
    let expected_ids = BTreeSet::from([SMALL_BROADLEAF_ID, TALL_NARROW_ID, OLD_GROWTH_ID]);
    if accepted_ids != expected_ids {
        issues.push(recipe_issue(format!(
            "Deep Forest requires all three authored tree forms, got {accepted_ids:?}"
        )));
    }
    if authored_blockers != plan.blockers {
        issues.push(recipe_issue(
            "Deep Forest global blockers must exactly equal authored tree footprints",
        ));
    }
    if authored_blockers.len() != expected_blockers {
        issues.push(recipe_issue(format!(
            "Deep Forest requires exactly {expected_blockers} blocker surfaces, got {}",
            authored_blockers.len()
        )));
    }
    let coverage = count_u32(authored_blockers.len())
        .saturating_mul(100)
        .checked_div(count_u32(eligible.len()))
        .unwrap_or_default();
    if !(28..=32).contains(&coverage)
        || coverage.abs_diff(u32::from(settings.blocker_coverage_percent)) > 1
    {
        issues.push(recipe_issue(format!(
            "Deep Forest blocker coverage is outside its authored target: {coverage}%"
        )));
    }

    let ordinary = OrdinaryGraph::from_volume(&plan.volume, Some(&plan.blockers));
    let mut critical_route_steps = 0;
    match (party, hostile) {
        (Some(party), Some(hostile)) => {
            let distances = ordinary.distances_from(party);
            if distances.len() != ordinary.len() {
                issues.push(recipe_issue(format!(
                    "Deep Forest blockers disconnect ordinary footing: {}/{} reachable",
                    distances.len(),
                    ordinary.len()
                )));
            }
            if let Some(distance) = distances.get(&hostile).copied() {
                critical_route_steps = distance;
            } else {
                issues.push(recipe_issue(
                    "Deep Forest actor anchors are disconnected by trees",
                ));
            }
        }
        _ => issues.push(recipe_issue(
            "Deep Forest requires party_start and hostile_start anchors",
        )),
    }
    let levels = ordinary
        .positions()
        .map(|position| position.level)
        .collect::<BTreeSet<_>>();
    let relief = levels
        .first()
        .zip(levels.last())
        .map_or(0, |(lowest, highest)| highest.saturating_sub(*lowest));
    if relief < settings.max_relief.saturating_sub(1) || relief > settings.max_relief {
        issues.push(recipe_issue(format!(
            "Deep Forest ordinary relief must remain within one level of {}, got {relief}",
            settings.max_relief
        )));
    }
    if !issues.is_empty() {
        return WorldValidation::Invalid(issues);
    }
    WorldValidation::Valid(DeepForestMetrics {
        tree_roots: count_u32(accepted_roots.len()),
        tree_blocker_surfaces: count_u32(authored_blockers.len()),
        blocker_coverage_percent: coverage,
        clearing_count: count_u32(plan.features.clearings.len()),
        clearing_surfaces: count_u32(clearing_surfaces.len()),
        protected_trail_surfaces: count_u32(trail_surfaces.len()),
        ordinary_surfaces: count_u32(ordinary.len()),
        reachable_elevation_levels: count_u32(levels.len()),
        relief,
        critical_route_steps,
    })
}

fn surface_set_connected(surfaces: &BTreeSet<TilePos>, graph: &OrdinaryGraph) -> bool {
    let Some(start) = surfaces.iter().next().copied() else {
        return false;
    };
    let mut visited = BTreeSet::from([start]);
    let mut frontier = VecDeque::from([start]);
    while let Some(position) = frontier.pop_front() {
        for neighbor in graph.neighbors(position) {
            if surfaces.contains(neighbor) && visited.insert(*neighbor) {
                frontier.push_back(*neighbor);
            }
        }
    }
    visited == *surfaces
}

fn feature_priority(stream: Option<SeedStream<'_>>, coord: HexCoord, salt: u64) -> u64 {
    stream.map_or_else(
        || {
            let mut bytes = Vec::with_capacity(48);
            bytes.extend_from_slice(b"bevy-hex-game/v3/deep-forest/fallback-feature");
            bytes.extend_from_slice(&coord.x().to_le_bytes());
            bytes.extend_from_slice(&coord.y().to_le_bytes());
            bytes.extend_from_slice(&coord.z().to_le_bytes());
            bytes.extend_from_slice(&salt.to_le_bytes());
            xxh3_64(&bytes)
        },
        |stream| stream.sample_coord(coord, salt),
    )
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

fn recipe_issue(detail: impl Into<String>) -> WorldValidationIssue {
    WorldValidationIssue::new(WorldIssueCode::Recipe("deep_forest"), detail)
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
                environment: V3EnvironmentSettings::TemperateGrassland,
                recipe: V3RecipeSettings::DeepForest(V3DeepForestSettings {
                    base_level: 15,
                    max_relief: 4,
                    blocker_coverage_percent: 30,
                    clearing_count: 3,
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

    fn generate(
        radius: u32,
        seed: u64,
    ) -> Result<ValidatedWorldSelection<DeepForestMetrics>, V3GenerationError> {
        super::generate(
            radius,
            0.4,
            &settings(),
            seed,
            super::super::vegetation::tests::runtime_art_catalog(),
        )
    }

    #[test]
    fn fixed_corpus_builds_deterministic_dense_forests() {
        for radius in [12, 20, 40] {
            for seed in [0, 1, 1_592_598_566, 4_294_967_311] {
                let first = generate(radius, seed).expect("Deep Forest should generate");
                let repeated = generate(radius, seed).expect("Deep Forest should repeat");
                assert_eq!(
                    first.validated.semantic_fingerprint,
                    repeated.validated.semantic_fingerprint
                );
                assert!(!first.used_fallback);
                assert_eq!(first.candidates_evaluated, 8);
                assert_eq!(first.valid_candidates, 8);
                assert!(first.notes.is_empty());
                assert!((28..=32).contains(&first.metrics.blocker_coverage_percent));
                assert_eq!(first.metrics.clearing_count, 3);
                assert_eq!(first.validated.plan.features.protected_routes.len(), 1);
                assert!(first
                    .validated
                    .plan
                    .features
                    .by_id
                    .values()
                    .all(|feature| feature.kind == FeatureKind::Tree));
            }
        }
    }

    #[test]
    fn radius_12_pr_corpus_validates_128_deep_forest_seeds() {
        let fallbacks = (0..128_u64)
            .filter(|seed| {
                generate(12, *seed)
                    .expect("Deep Forest should generate")
                    .used_fallback
            })
            .count();
        assert_eq!(
            fallbacks, 0,
            "actual Deep Forest fallback count for the 128-seed PR corpus"
        );
    }

    #[test]
    fn deep_forest_resolves_from_a_tree_only_art_catalog() {
        let selected = super::generate(
            12,
            0.4,
            &settings(),
            1_592_598_566,
            super::super::vegetation::tests::tree_only_runtime_art_catalog(),
        )
        .expect("Deep Forest should not require unrelated grass or environment art");
        assert_eq!(selected.candidates_evaluated, 8);
        assert_eq!(selected.valid_candidates, 8);
    }

    #[test]
    fn forced_candidate_failure_uses_independent_deep_forest_fallback() {
        let settings = settings();
        let layout = resolve_layout(12, &settings).expect("fixture layout should resolve");
        let trees = TemperateTreeSet::resolve(
            super::super::vegetation::tests::tree_only_runtime_art_catalog(),
            "Deep Forest",
        )
        .expect("fixture tree art should resolve");
        let force = |seed| {
            run_recipe(
                &DeepForestRecipe {
                    level_height: 0.4,
                    layout: layout.clone(),
                    trees: trees.clone(),
                    reject_candidates: true,
                },
                &settings,
                12,
                seed,
            )
            .expect("canonical Deep Forest fallback should validate")
        };
        let first = force(44);
        let other_seed = force(9_999);
        for selected in [&first, &other_seed] {
            assert!(selected.used_fallback);
            assert_eq!(selected.selected_candidate, None);
            assert_eq!(selected.candidates_evaluated, 8);
            assert_eq!(selected.valid_candidates, 0);
        }
        assert_eq!(
            first.validated.semantic_fingerprint, other_seed.validated.semantic_fingerprint,
            "canonical fallback must not depend on the rejected world seed"
        );
        assert_eq!(first.metrics, other_seed.metrics);
    }
}
