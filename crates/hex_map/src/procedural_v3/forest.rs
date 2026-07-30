//! Pure semantic Forest recipe for procedural generator V3.
//!
//! Terrain and clearings are finalized before tree roots. The protected road is
//! then routed around those exact blockers, while tall grass remains
//! presentation-only.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

use hex_assets::{HexObjectRotation, RuntimeArtCatalog};
use hex_core::{HexCoord, MapViewHint, TilePos};
use xxhash_rust::xxh3::xxh3_64;

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
use super::vegetation::{
    TemperateVegetationSet, VegetationObjectSpec, GRASS_TUFT_ID, OLD_GROWTH_ID, SMALL_BROADLEAF_ID,
    TALL_NARROW_ID,
};
use super::volume::{
    LevelInterval, SolidMass, SolidMaterialRole, SurfaceAccess, SurfaceMetadata, VolumeColumn,
    VolumeElement, VolumePlan,
};
use super::world::{
    FeatureClearing, FeatureId, FeatureKind, FeaturePlan, GeneratedWorldPlan, InteriorPlan,
    PlannedFeature, ProtectedFeatureRoute, StructurePlan, WorldIssueCode, WorldValidationIssue,
};
use super::V3GenerationError;
use crate::settings::{
    ProceduralV3Settings, V3EnvironmentSettings, V3ForestSettings, V3LayoutSettings,
    V3RecipeSettings,
};

const BASE_LEVEL: i32 = 15;
const MAX_RELIEF: i32 = 4;
const MOUND_COUNT: u64 = 5;
const CLEARING_COUNT: usize = 4;
const TREE_DENSITY_PERCENT: usize = 22;
const GRASS_DENSITY_PERCENT: usize = 70;
const PRAIRIE_TAPER_DEPTH: i32 = 3;
const ROAD_ROUTE: &str = "forest_road";
const PARTY_START: &str = "party_start";
const HOSTILE_START: &str = "hostile_start";
const FOREST_CLEARING: &str = "forest_clearing";
const PRAIRIE_OVERLOOK: &str = "prairie_overlook";
const OLD_GROWTH_CAPACITY_DETAIL: &str =
    "Forest capacity plan cannot retain one exact Old-Growth instance";
#[cfg(test)]
static CAPACITY_PROJECTION_CACHE_PEAK: AtomicUsize = AtomicUsize::new(0);

/// Recipe metrics retained by the V3 candidate selector and diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ForestMetrics {
    pub(crate) tree_roots: u32,
    pub(crate) tree_blocker_surfaces: u32,
    pub(crate) old_growth_roots: u32,
    pub(crate) old_growth_blocker_surfaces: u32,
    pub(crate) tall_grass_roots: u32,
    pub(crate) woodland_surfaces: u32,
    pub(crate) prairie_surfaces: u32,
    pub(crate) clearing_count: u32,
    pub(crate) clearing_surfaces: u32,
    pub(crate) protected_route_surfaces: u32,
    pub(crate) ordinary_surfaces: u32,
    pub(crate) reachable_elevation_levels: u32,
    pub(crate) relief: u32,
    pub(crate) spawn_height_difference: u32,
    pub(crate) woodland_prairie_high_ground_difference: u32,
    pub(crate) critical_route_steps: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OrdinaryElevationMetrics {
    reachable_levels: BTreeSet<i32>,
    relief: u32,
    woodland_prairie_high_ground_difference: u32,
}

#[derive(Debug)]
struct ForestRecipe {
    level_height: f32,
    layout: ResolvedLayoutPlan,
    objects: TemperateVegetationSet,
    #[cfg(test)]
    reject_candidates: bool,
}

#[derive(Debug, Clone, Copy)]
struct ForestStreams<'a> {
    orientation: SeedStream<'a>,
    landform: SeedStream<'a>,
    clearings: SeedStream<'a>,
    routes: SeedStream<'a>,
    trees: SeedStream<'a>,
    tree_objects: SeedStream<'a>,
    tree_rotations: SeedStream<'a>,
    grass: SeedStream<'a>,
    grass_rotations: SeedStream<'a>,
}

#[derive(Debug)]
struct PlannedRoad {
    centerline: Vec<HexCoord>,
    surfaces: BTreeSet<HexCoord>,
}

impl TemperateVegetationSet {
    fn forest_tree(&self, family: TreeFamily) -> &VegetationObjectSpec {
        match family {
            TreeFamily::SmallBroadleaf => &self.small_broadleaf,
            TreeFamily::TallNarrow => &self.tall_narrow,
        }
    }

    fn forest_object(&self, id: &str) -> Option<&VegetationObjectSpec> {
        [
            &self.small_broadleaf,
            &self.tall_narrow,
            &self.old_growth,
            &self.grass_tuft,
        ]
        .into_iter()
        .find(|object| object.id.as_str() == id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TreeFamily {
    SmallBroadleaf,
    TallNarrow,
}

/// Runs the common eight-candidate V3 selector for one Forest world.
pub(crate) fn generate(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV3Settings,
    seed: u64,
    catalog: &RuntimeArtCatalog,
) -> Result<ValidatedWorldSelection<ForestMetrics>, V3GenerationError> {
    let objects = TemperateVegetationSet::resolve(catalog, "Forest")
        .map_err(V3GenerationError::RecipeContract)?;
    generate_with_objects(grid_radius, level_height, settings, seed, objects)
}

fn generate_with_objects(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV3Settings,
    seed: u64,
    objects: TemperateVegetationSet,
) -> Result<ValidatedWorldSelection<ForestMetrics>, V3GenerationError> {
    if !level_height.is_finite() || level_height <= 0.0 {
        return Err(V3GenerationError::RecipeContract(
            "Forest level height must be positive and finite".to_owned(),
        ));
    }
    validate_recipe_settings(settings)?;
    let layout = resolve_layout(grid_radius, settings)
        .map_err(|error| V3GenerationError::RecipeContract(error.to_string()))?;
    validate_footprint_capacity(&layout)?;
    run_recipe(
        &ForestRecipe {
            level_height,
            layout,
            objects,
            #[cfg(test)]
            reject_candidates: false,
        },
        settings,
        grid_radius,
        seed,
    )
}

impl V3Recipe for ForestRecipe {
    type Settings = ProceduralV3Settings;
    type Metrics = ForestMetrics;
    type Score = (u32, u32, u32, u8);

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

        validate_recipe_settings(settings).map_err(CandidateAttemptError::Fatal)?;
        if context.grid_radius != self.layout.grid_radius {
            return Err(CandidateAttemptError::Fatal(
                V3GenerationError::RecipeContract(
                    "Forest candidate radius disagrees with its resolved layout".to_owned(),
                ),
            ));
        }
        let patch = PatchRecipeContext::resolve(&self.layout, PatchId(0))
            .map_err(CandidateAttemptError::Fatal)?;
        let fragment = construct_patch_with_objects(
            patch,
            &V3ForestSettings,
            V3EnvironmentSettings::TemperateGrassland,
            self.level_height,
            PatchBuildMode::Candidate {
                world_seed: context.seed,
                candidate: context.candidate,
            },
            &self.objects,
        )
        .map_err(CandidateAttemptError::Rejected)?;
        compose_single_patch(self.layout.clone(), fragment).map_err(|error| {
            CandidateAttemptError::Rejected(vec![recipe_issue(format!("{error:?}"))])
        })
    }

    fn validate(
        &self,
        _settings: &Self::Settings,
        plan: &GeneratedWorldPlan,
    ) -> WorldValidation<Self::Metrics> {
        validate_forest(plan)
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
        let tree_target = metrics.woodland_surfaces.saturating_mul(22) / 100;
        let grass_target = metrics.prairie_surfaces.saturating_mul(70) / 100;
        (
            metrics.tree_roots.abs_diff(tree_target),
            metrics.tall_grass_roots.abs_diff(grass_target),
            metrics
                .relief
                .abs_diff(u32::try_from(MAX_RELIEF).unwrap_or_default()),
            candidate,
        )
    }

    fn canonical_fallback(
        &self,
        context: FallbackContext,
        settings: &Self::Settings,
    ) -> Result<GeneratedWorldPlan, V3GenerationError> {
        validate_recipe_settings(settings)?;
        if context.grid_radius != self.layout.grid_radius {
            return Err(V3GenerationError::RecipeContract(
                "Forest fallback radius disagrees with its resolved layout".to_owned(),
            ));
        }
        let patch = PatchRecipeContext::resolve(&self.layout, PatchId(0))?;
        let fragment = construct_patch_with_objects(
            patch,
            &V3ForestSettings,
            V3EnvironmentSettings::TemperateGrassland,
            self.level_height,
            PatchBuildMode::CanonicalFallback,
            &self.objects,
        )
        .map_err(recipe_issues_to_error)?;
        compose_single_patch(self.layout.clone(), fragment)
            .map_err(|error| V3GenerationError::RecipeContract(format!("{error:?}")))
    }
}

fn validate_recipe_settings(settings: &ProceduralV3Settings) -> Result<(), V3GenerationError> {
    let V3LayoutSettings::Single(patch) = &settings.layout else {
        return Err(V3GenerationError::RecipeUnavailable("Ring7"));
    };
    if patch.environment != V3EnvironmentSettings::TemperateGrassland {
        return Err(V3GenerationError::RecipeContract(
            "Forest requires the TemperateGrassland environment".to_owned(),
        ));
    }
    if !matches!(patch.recipe, V3RecipeSettings::Forest(V3ForestSettings)) {
        return Err(V3GenerationError::RecipeUnavailable(recipe_name(
            &patch.recipe,
        )));
    }
    if !patch.overlays.is_empty() {
        return Err(V3GenerationError::RecipeContract(
            "Forest overlays are not implemented yet".to_owned(),
        ));
    }
    Ok(())
}

fn validate_footprint_capacity(layout: &ResolvedLayoutPlan) -> Result<(), V3GenerationError> {
    let radius = i32::try_from(layout.grid_radius).map_err(|error| {
        V3GenerationError::RecipeContract(format!("Forest radius exceeds i32: {error}"))
    })?;
    if layout.footprint.len() < 127 {
        return Err(V3GenerationError::RecipeContract(
            "Forest requires at least 127 connected columns for routes and clearings".to_owned(),
        ));
    }
    let road_endpoints = [
        HexCoord::from_axial((-radius).saturating_add(2), 0),
        HexCoord::from_axial(PRAIRIE_TAPER_DEPTH, 0),
    ];
    if road_endpoints
        .iter()
        .any(|coord| !layout.footprint.contains(coord))
    {
        return Err(V3GenerationError::RecipeContract(
            "Forest footprint cannot fit the canonical woodland road".to_owned(),
        ));
    }
    Ok(())
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

pub(crate) fn construct_patch(
    patch: PatchRecipeContext<'_>,
    settings: &V3ForestSettings,
    environment: V3EnvironmentSettings,
    level_height: f32,
    mode: PatchBuildMode,
    catalog: &RuntimeArtCatalog,
) -> Result<GeneratedPatchPlan, Vec<WorldValidationIssue>> {
    let objects = TemperateVegetationSet::resolve(catalog, "Forest")
        .map_err(|error| vec![recipe_issue(error)])?;
    construct_patch_with_objects(patch, settings, environment, level_height, mode, &objects)
}

fn construct_patch_with_objects(
    patch: PatchRecipeContext<'_>,
    _settings: &V3ForestSettings,
    environment: V3EnvironmentSettings,
    level_height: f32,
    mode: PatchBuildMode,
    objects: &TemperateVegetationSet,
) -> Result<GeneratedPatchPlan, Vec<WorldValidationIssue>> {
    if environment != V3EnvironmentSettings::TemperateGrassland {
        return Err(vec![recipe_issue(
            "Forest requires the TemperateGrassland environment",
        )]);
    }
    let frame = patch
        .local_frame()
        .map_err(|error| vec![recipe_issue(error.to_string())])?;
    let stitched_patch = patch.layout().kind.is_composite();
    let mask = frame
        .local_mask(patch.mask())
        .map_err(|error| vec![recipe_issue(error)])?;
    let biome_region = patch.biome_region();
    let patch_radius = frame.scale();
    let radius = i32::try_from(patch_radius)
        .map_err(|error| vec![recipe_issue(format!("Forest radius exceeds i32: {error}"))])?;
    let streams = mode.seed_streams(&patch).map(|streams| ForestStreams {
        orientation: streams.stage("forest.orientation"),
        landform: streams.stage("forest.landform"),
        clearings: streams.stage("forest.clearings"),
        routes: streams.stage("forest.routes"),
        trees: streams.stage("forest.trees"),
        tree_objects: streams.stage("forest.tree-objects"),
        tree_rotations: streams.stage("forest.tree-rotations"),
        grass: streams.stage("forest.grass"),
        grass_rotations: streams.stage("forest.grass-rotations"),
    });
    let rotation = if patch.layout().kind == super::layout::LayoutKind::Ring19 {
        0
    } else {
        streams.map_or(0, |streams| {
            u8::try_from(streams.orientation.sample(0) % 6).unwrap_or_default()
        })
    };
    let route_offset = match streams {
        Some(streams) => streams
            .routes
            .range_i32(0, -1, 1)
            .map_err(|error| vec![recipe_issue(error)])?,
        None => 0,
    };
    let nominal_party_coord = rotate(
        HexCoord::from_axial((-radius).saturating_add(2), route_offset),
        rotation,
    );
    let nominal_hostile_coord = rotate(
        HexCoord::from_axial(radius.saturating_sub(2), route_offset),
        rotation,
    );
    let relief = ReliefPlan::new(
        patch_radius,
        &mask,
        rotation,
        streams.map(|streams| streams.landform),
    )?;

    let local_levels = mask
        .iter()
        .copied()
        .map(|coord| (coord, BASE_LEVEL.saturating_add(relief.height_at(coord))))
        .collect();
    let mut world_levels = frame
        .levels_to_world(local_levels)
        .map_err(|error| vec![recipe_issue(error)])?;
    let seam_shape = shape_walker_seams(&patch, &mut world_levels)?;
    let local_levels = frame
        .levels_to_local(world_levels)
        .map_err(|error| vec![recipe_issue(error)])?;
    let local_protected = patch
        .protected_approaches()
        .into_iter()
        .map(|coord| frame.to_local(coord).map_err(recipe_issue))
        .collect::<Result<BTreeSet<_>, _>>()
        .map_err(|issue| vec![issue])?;
    let mut surfaces = BTreeMap::new();
    let mut surface_by_coord = BTreeMap::new();
    let mut ordinary_surface_by_coord = BTreeMap::new();
    for coord in &mask {
        let surface_level = local_levels.get(coord).copied().ok_or_else(|| {
            vec![recipe_issue(format!(
                "Forest land plan omitted coordinate {coord:?}"
            ))]
        })?;
        let position = TilePos::new(*coord, surface_level);
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
            ordinary_surface_by_coord.insert(*coord, position);
        }
    }
    let ordinary_coords = ordinary_surface_by_coord
        .keys()
        .copied()
        .collect::<BTreeSet<_>>();
    let party_coord = nearest_ordinary_landing(
        nominal_party_coord,
        &ordinary_surface_by_coord,
        rotation,
        true,
    )
    .ok_or_else(|| vec![recipe_issue("Forest route has no ordinary party landing")])?;
    let hostile_coord = nearest_ordinary_landing(
        nominal_hostile_coord,
        &ordinary_surface_by_coord,
        rotation,
        false,
    )
    .ok_or_else(|| vec![recipe_issue("Forest route has no ordinary hostile landing")])?;

    let woodland: BTreeSet<_> = ordinary_coords
        .iter()
        .copied()
        .filter(|coord| is_woodland(*coord, rotation))
        .collect();
    let clearings = clearing_coordinates(
        radius,
        rotation,
        &ordinary_coords,
        &woodland,
        streams.map(|streams| streams.clearings),
        stitched_patch,
    )?;
    let mut clearing_plans = BTreeMap::new();
    let mut clearing_coords = BTreeSet::new();
    for (index, clearing) in clearings.iter().enumerate() {
        let surfaces = exact_position_set(&clearing.coords, &ordinary_surface_by_coord)?;
        clearing_coords.extend(clearing.coords.iter().copied());
        clearing_plans.insert(
            format!("forest_clearing_{index}"),
            FeatureClearing { surfaces },
        );
    }
    let ring19_reserved_road = if patch.layout().kind == super::layout::LayoutKind::Ring19 {
        Some(plan_road(
            rotation,
            route_offset,
            party_coord,
            &ordinary_coords,
            &ordinary_surface_by_coord,
            &BTreeSet::new(),
            &clearings,
            streams.map(|streams| streams.routes),
            stitched_patch,
        )?)
    } else {
        None
    };

    let mut tree_exclusions = clearing_coords.iter().copied().collect::<BTreeSet<_>>();
    tree_exclusions.extend(local_protected.iter().copied());
    if let Some(road) = &ring19_reserved_road {
        tree_exclusions.extend(road.surfaces.iter().copied());
    }
    if stitched_patch {
        tree_exclusions.extend(
            (0..=PRAIRIE_TAPER_DEPTH)
                .map(|x| rotate(HexCoord::from_axial(x, route_offset), rotation)),
        );
    }
    tree_exclusions.extend(
        party_coord
            .within_radius(1)
            .into_iter()
            .filter(|coord| mask.contains(coord)),
    );
    tree_exclusions.extend(
        hostile_coord
            .within_radius(1)
            .into_iter()
            .filter(|coord| mask.contains(coord)),
    );
    let tree_root_candidates = select_tree_root_candidates(
        &woodland,
        &tree_exclusions,
        &ordinary_surface_by_coord,
        streams.map(|streams| streams.trees),
    );
    let eligible_tree_woodland = if patch.layout().kind == super::layout::LayoutKind::Ring19 {
        woodland
            .difference(&tree_exclusions)
            .copied()
            .collect::<BTreeSet<_>>()
    } else {
        woodland.clone()
    };
    let tree_target = eligible_tree_woodland
        .len()
        .saturating_mul(TREE_DENSITY_PERCENT)
        / 100;
    let minimum_tree_count = eligible_tree_woodland
        .len()
        .saturating_mul(20)
        .div_ceil(100);
    let (tree_features, tree_visual_cells) = plan_tree_features(
        tree_root_candidates,
        tree_target,
        minimum_tree_count,
        &woodland,
        &tree_exclusions,
        &ordinary_surface_by_coord,
        objects,
        streams.map(|streams| streams.tree_objects),
        streams.map(|streams| streams.tree_rotations),
    )?;
    let tree_route_obstructions: BTreeSet<_> = tree_visual_cells
        .iter()
        .filter_map(|position| {
            ordinary_surface_by_coord
                .get(&position.coord)
                .filter(|surface| position.level <= surface.level.saturating_add(2))
                .map(|_| position.coord)
        })
        .collect();
    let road = if let Some(road) = ring19_reserved_road {
        road
    } else {
        plan_road(
            rotation,
            route_offset,
            party_coord,
            &ordinary_coords,
            &ordinary_surface_by_coord,
            &tree_route_obstructions,
            &clearings,
            streams.map(|streams| streams.routes),
            stitched_patch,
        )?
    };

    let mut columns = BTreeMap::new();
    for (coord, position) in &surface_by_coord {
        columns.insert(
            *coord,
            land_column(position.level, road.surfaces.contains(coord)),
        );
    }
    let volume = VolumePlan {
        mask: mask.clone(),
        columns,
        surfaces,
    };

    let road_centerline = exact_positions(&road.centerline, &ordinary_surface_by_coord)?;
    let road_surfaces = exact_position_set(&road.surfaces, &ordinary_surface_by_coord)?;
    let prairie: BTreeSet<_> = ordinary_coords.difference(&woodland).copied().collect();
    let prairie_coord = rotate(HexCoord::from_axial(radius / 2, -radius / 4), rotation);
    let prairie_overlook =
        nearest_ordinary_landing(prairie_coord, &ordinary_surface_by_coord, rotation, false)
            .and_then(|coord| ordinary_surface_by_coord.get(&coord).copied())
            .ok_or_else(|| vec![recipe_issue("Forest has no prairie overlook surface")])?;
    let mut grass_exclusions = road.surfaces.clone();
    grass_exclusions.extend(local_protected.iter().copied());
    grass_exclusions.extend(
        hostile_coord
            .within_radius(1)
            .into_iter()
            .filter(|coord| mask.contains(coord)),
    );
    grass_exclusions.extend(
        prairie_overlook
            .coord
            .within_radius(1)
            .into_iter()
            .filter(|coord| mask.contains(coord)),
    );
    let grass_roots = select_grass_roots(
        &prairie,
        &grass_exclusions,
        &ordinary_surface_by_coord,
        streams.map(|streams| streams.grass),
    );
    let grass_features = plan_grass_features(
        grass_roots,
        objects,
        streams.map(|streams| streams.grass_rotations),
    )?;
    let (features, blockers) = build_feature_plan(
        tree_features,
        grass_features,
        road_centerline,
        road_surfaces,
        clearing_plans,
    );

    let party_start = ordinary_surface_by_coord
        .get(&party_coord)
        .copied()
        .ok_or_else(|| vec![recipe_issue("Forest route has no party surface")])?;
    let hostile_start = ordinary_surface_by_coord
        .get(&hostile_coord)
        .copied()
        .ok_or_else(|| vec![recipe_issue("Forest route has no hostile surface")])?;
    let forest_clearing = clearings
        .first()
        .and_then(|clearing| ordinary_surface_by_coord.get(&clearing.center))
        .copied()
        .ok_or_else(|| vec![recipe_issue("Forest has no clearing anchor surface")])?;
    let anchors = BTreeMap::from([
        (PARTY_START.to_owned(), party_start),
        (HOSTILE_START.to_owned(), hostile_start),
        (FOREST_CLEARING.to_owned(), forest_clearing),
        (PRAIRIE_OVERLOOK.to_owned(), prairie_overlook),
    ]);
    let biome_regions = volume
        .surfaces
        .keys()
        .copied()
        .map(|surface| (surface, biome_region))
        .collect();
    let view_hint = forest_view_hint(patch_radius, level_height, rotation)?;

    let mut plan = GeneratedPatchPlan {
        patch_id: patch.id,
        volume,
        liquids: Default::default(),
        features,
        structures: StructurePlan::default(),
        blockers,
        lights: BTreeMap::new(),
        biome_regions,
        interiors: InteriorPlan::default(),
        anchors,
        view_hint,
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

fn nearest_ordinary_landing(
    nominal: HexCoord,
    surfaces: &BTreeMap<HexCoord, TilePos>,
    rotation: u8,
    from_low_x_side: bool,
) -> Option<HexCoord> {
    surfaces.keys().copied().min_by_key(|coord| {
        let local_x = unrotate(*coord, rotation).x();
        let inward_priority = if from_low_x_side {
            local_x.saturating_neg()
        } else {
            local_x
        };
        (nominal.distance(*coord), inward_priority, *coord)
    })
}

fn build_feature_plan(
    tree_features: Vec<PlannedFeature>,
    grass_features: Vec<PlannedFeature>,
    road_centerline: Vec<TilePos>,
    road_surfaces: BTreeSet<TilePos>,
    clearings: BTreeMap<String, FeatureClearing>,
) -> (FeaturePlan, BTreeSet<TilePos>) {
    let blockers = tree_features
        .iter()
        .flat_map(|feature| feature.blocker_footprint.iter().copied())
        .collect();
    let mut by_id = BTreeMap::new();
    let mut next_id = 0_u32;
    for feature in tree_features.into_iter().chain(grass_features) {
        by_id.insert(FeatureId(next_id), feature);
        next_id = next_id.saturating_add(1);
    }
    let protected_routes = BTreeMap::from([(
        ROAD_ROUTE.to_owned(),
        ProtectedFeatureRoute {
            centerline: road_centerline,
            surfaces: road_surfaces,
        },
    )]);
    (
        FeaturePlan {
            by_id,
            protected_routes,
            clearings,
        },
        blockers,
    )
}

pub(crate) fn validate_patch(
    patch: PatchRecipeContext<'_>,
    plan: &GeneratedPatchPlan,
) -> WorldValidation<ForestMetrics> {
    let approach_depth = patch
        .shared_edges()
        .map(|edge| edge.contract.approach_depth)
        .max()
        .unwrap_or_default();
    let frame = match patch.local_frame() {
        Ok(frame) => frame,
        Err(error) => {
            return WorldValidation::Invalid(vec![recipe_issue(format!(
                "Forest validation frame failed: {error}"
            ))]);
        }
    };
    let protected_approaches = match patch
        .protected_approaches()
        .into_iter()
        .map(|coord| frame.to_local(coord).map_err(recipe_issue))
        .collect::<Result<BTreeSet<_>, _>>()
    {
        Ok(protected) => protected,
        Err(issue) => return WorldValidation::Invalid(vec![issue]),
    };
    match frame.canonical_local_world(plan) {
        Ok(plan) => validate_forest_inner(
            &plan,
            Some(approach_depth),
            &protected_approaches,
            patch.layout().kind == super::layout::LayoutKind::Ring19,
        ),
        Err(error) => WorldValidation::Invalid(vec![recipe_issue(format!(
            "Forest validation projection failed: {error}"
        ))]),
    }
}

pub(crate) fn validate_forest(plan: &GeneratedWorldPlan) -> WorldValidation<ForestMetrics> {
    validate_forest_inner(plan, None, &BTreeSet::new(), false)
}

fn validate_forest_inner(
    plan: &GeneratedWorldPlan,
    stitched_approach_depth: Option<u32>,
    stitched_protected_approaches: &BTreeSet<HexCoord>,
    ring19_patch: bool,
) -> WorldValidation<ForestMetrics> {
    let stitched_patch = stitched_approach_depth.is_some();
    let mut issues = Vec::new();
    if !plan.liquids.bodies.is_empty() {
        issues.push(recipe_issue("Forest must not contain liquid topology"));
    }
    if !plan.structures.by_id.is_empty()
        || !plan.lights.is_empty()
        || !plan.interiors.by_id.is_empty()
    {
        issues.push(recipe_issue(
            "Forest must not contain structures, gameplay lights, or interiors",
        ));
    }

    let orientation = if stitched_patch {
        detect_stitched_orientation(plan)
    } else {
        detect_orientation(plan)
    };
    let Some((rotation, route_offset)) = orientation else {
        return WorldValidation::Invalid(vec![recipe_issue(
            "Forest actor anchors do not match one supported orientation",
        )]);
    };
    let Some(road) = plan.features.protected_routes.get(ROAD_ROUTE) else {
        return WorldValidation::Invalid(vec![recipe_issue(
            "Forest does not contain its named woodland road",
        )]);
    };
    if plan.features.protected_routes.len() != 1 {
        issues.push(recipe_issue(
            "Forest must contain exactly one protected woodland road",
        ));
    }
    let all_woodland: BTreeSet<_> = plan
        .volume
        .surfaces
        .keys()
        .copied()
        .filter(|position| is_woodland(position.coord, rotation))
        .collect();
    let woodland = if stitched_patch {
        all_woodland
            .iter()
            .copied()
            .filter(|position| {
                plan.volume
                    .surfaces
                    .get(position)
                    .is_some_and(|metadata| metadata.access == SurfaceAccess::Ordinary)
            })
            .collect()
    } else {
        all_woodland.clone()
    };
    let all_prairie: BTreeSet<_> = plan
        .volume
        .surfaces
        .keys()
        .copied()
        .filter(|position| !is_woodland(position.coord, rotation))
        .collect();
    let prairie = if stitched_patch {
        all_prairie
            .iter()
            .copied()
            .filter(|position| {
                plan.volume
                    .surfaces
                    .get(position)
                    .is_some_and(|metadata| metadata.access == SurfaceAccess::Ordinary)
            })
            .collect()
    } else {
        all_prairie
    };
    let total_surfaces = plan.volume.surfaces.len();
    if all_woodland.len().saturating_mul(100) < total_surfaces.saturating_mul(42)
        || all_woodland.len().saturating_mul(100) > total_surfaces.saturating_mul(58)
    {
        issues.push(recipe_issue(format!(
            "Forest woodland must cover 42-58% of surfaces, got {}/{}",
            all_woodland.len(),
            total_surfaces
        )));
    }

    let tree_roots: BTreeSet<_> = plan
        .features
        .by_id
        .values()
        .filter(|feature| feature.kind == FeatureKind::Tree)
        .map(|feature| feature.root)
        .collect();
    let grass_roots: BTreeSet<_> = plan
        .features
        .by_id
        .values()
        .filter(|feature| feature.kind == FeatureKind::TallGrass)
        .map(|feature| feature.root)
        .collect();
    let tree_blocker_surfaces: BTreeSet<_> = plan
        .features
        .by_id
        .values()
        .filter(|feature| feature.kind == FeatureKind::Tree)
        .flat_map(|feature| feature.blocker_footprint.iter().copied())
        .collect();
    let old_growth: Vec<_> = plan
        .features
        .by_id
        .values()
        .filter(|feature| feature.object_id.as_str() == OLD_GROWTH_ID)
        .collect();
    if old_growth.is_empty() {
        issues.push(recipe_issue(
            "Forest must retain at least one exact authored Old-Growth instance",
        ));
    }
    let old_growth_blocker_surfaces: BTreeSet<_> = old_growth
        .iter()
        .flat_map(|feature| feature.blocker_footprint.iter().copied())
        .collect();
    if !tree_roots.is_subset(&woodland) {
        issues.push(recipe_issue("Forest tree roots leave the woodland side"));
    }
    if !tree_blocker_surfaces.is_subset(&woodland) {
        issues.push(recipe_issue(
            "Forest authored tree blocker footprints leave the woodland side",
        ));
    }
    if !grass_roots.is_subset(&prairie) {
        issues.push(recipe_issue("Forest tall grass leaves the prairie side"));
    }
    for feature in plan.features.by_id.values() {
        let accepted = match feature.kind {
            FeatureKind::Tree => matches!(
                feature.object_id.as_str(),
                SMALL_BROADLEAF_ID | TALL_NARROW_ID | OLD_GROWTH_ID
            ),
            FeatureKind::TallGrass => feature.object_id.as_str() == GRASS_TUFT_ID,
            FeatureKind::CaveVegetation => false,
        };
        if !accepted {
            issues.push(recipe_issue(format!(
                "Forest feature at {:?} uses unsupported authored object '{}'",
                feature.root, feature.object_id
            )));
        }
        if feature.object_id.as_str() == OLD_GROWTH_ID && feature.blocker_footprint.len() != 7 {
            issues.push(recipe_issue(format!(
                "Forest old-growth tree at {:?} does not retain its exact seven-hex roots",
                feature.root
            )));
        }
    }
    let tree_root_coords: BTreeSet<_> = tree_roots.iter().map(|root| root.coord).collect();
    if tree_root_coords.iter().any(|root| {
        root.neighbors()
            .into_iter()
            .any(|neighbor| tree_root_coords.contains(&neighbor))
    }) {
        issues.push(recipe_issue(
            "Forest tree roots violate deterministic Poisson spacing",
        ));
    }
    let mut tree_density_exclusions = plan
        .features
        .clearings
        .values()
        .flat_map(|clearing| clearing.surfaces.iter().map(|surface| surface.coord))
        .chain(stitched_protected_approaches.iter().copied())
        .collect::<BTreeSet<_>>();
    if stitched_patch {
        tree_density_exclusions.extend(
            (0..=PRAIRIE_TAPER_DEPTH)
                .map(|x| rotate(HexCoord::from_axial(x, route_offset), rotation)),
        );
    }
    if ring19_patch {
        tree_density_exclusions.extend(road.surfaces.iter().map(|surface| surface.coord));
    }
    for anchor in [PARTY_START, HOSTILE_START]
        .into_iter()
        .filter_map(|name| plan.anchors.get(name))
    {
        tree_density_exclusions.extend(
            anchor
                .coord
                .within_radius(1)
                .into_iter()
                .filter(|coord| plan.layout.footprint.contains(coord)),
        );
    }
    let eligible_tree_woodland = if ring19_patch {
        woodland
            .iter()
            .filter(|surface| !tree_density_exclusions.contains(&surface.coord))
            .copied()
            .collect::<BTreeSet<_>>()
    } else {
        woodland.clone()
    };
    if tree_roots.len().saturating_mul(100) < eligible_tree_woodland.len().saturating_mul(20)
        || tree_roots.len().saturating_mul(100) > eligible_tree_woodland.len().saturating_mul(24)
    {
        issues.push(recipe_issue(format!(
            "Forest tree density is outside 20-24% of eligible woodland: {}/{}",
            tree_roots.len(),
            eligible_tree_woodland.len()
        )));
    }
    if grass_roots.len().saturating_mul(100) < prairie.len().saturating_mul(65)
        || grass_roots.len().saturating_mul(100) > prairie.len().saturating_mul(75)
    {
        issues.push(recipe_issue(format!(
            "Forest grass density is outside 65-75% of prairie: {}/{}",
            grass_roots.len(),
            prairie.len()
        )));
    }

    let ordinary = OrdinaryGraph::from_volume(&plan.volume, Some(&plan.blockers));
    let tree_blocker_coords = tree_blocker_surfaces
        .iter()
        .map(|position| position.coord)
        .collect();
    validate_road(
        plan,
        &ordinary,
        road,
        rotation,
        route_offset,
        &tree_blocker_coords,
        stitched_approach_depth,
        &mut issues,
    );
    let party = plan.anchors.get(PARTY_START).copied();
    let hostile = plan.anchors.get(HOSTILE_START).copied();
    let mut spawn_height_difference = 0;
    let mut critical_route_steps = 0;
    match (party, hostile) {
        (Some(party), Some(hostile)) => {
            spawn_height_difference = party.level.abs_diff(hostile.level);
            let distances = ordinary.distances_from(party);
            if distances.len() != ordinary.len() {
                issues.push(recipe_issue(format!(
                    "Forest blockers disconnect ordinary footing: {}/{} reachable",
                    distances.len(),
                    ordinary.len()
                )));
            }
            if !distances.contains_key(&hostile) {
                issues.push(recipe_issue(
                    "Forest required actor anchors are disconnected by features",
                ));
            } else {
                critical_route_steps = distances.get(&hostile).copied().unwrap_or_default();
            }
        }
        _ => issues.push(recipe_issue(
            "Forest requires party_start and hostile_start anchors",
        )),
    }

    let clearing_surfaces: BTreeSet<_> = plan
        .features
        .clearings
        .values()
        .flat_map(|clearing| clearing.surfaces.iter().copied())
        .collect();
    let expected_clearing_names: BTreeSet<_> = (0..CLEARING_COUNT)
        .map(|index| format!("forest_clearing_{index}"))
        .collect();
    let actual_clearing_names: BTreeSet<_> = plan.features.clearings.keys().cloned().collect();
    if actual_clearing_names != expected_clearing_names {
        issues.push(recipe_issue(format!(
            "Forest requires exactly the named clearings {expected_clearing_names:?}"
        )));
    }
    let mut claimed_clearing_surfaces = BTreeMap::new();
    for (name, clearing) in &plan.features.clearings {
        if clearing.surfaces.len() < 10 {
            issues.push(recipe_issue(format!(
                "Forest clearing {name:?} has fewer than ten surfaces"
            )));
        }
        if !clearing.surfaces.is_subset(&woodland) {
            issues.push(recipe_issue(format!(
                "Forest clearing {name:?} leaves the woodland side"
            )));
        }
        if !surface_set_connected(&clearing.surfaces, &ordinary) {
            issues.push(recipe_issue(format!(
                "Forest clearing {name:?} is not walker-connected"
            )));
        }
        if let Some((position, previous)) = clearing.surfaces.iter().find_map(|position| {
            claimed_clearing_surfaces
                .get(position)
                .map(|previous| (*position, *previous))
        }) {
            issues.push(recipe_issue(format!(
                "Forest clearings {previous:?} and {name:?} overlap at {position:?}"
            )));
        }
        claimed_clearing_surfaces.extend(
            clearing
                .surfaces
                .iter()
                .copied()
                .map(|position| (position, name.as_str())),
        );
    }
    validate_review_anchors(plan, rotation, &prairie, &mut issues);

    let elevation = ordinary_elevation_metrics(&ordinary, rotation);
    if !(3..=u32::try_from(MAX_RELIEF).unwrap_or_default()).contains(&elevation.relief) {
        issues.push(recipe_issue(format!(
            "Forest relief must be 3-{MAX_RELIEF}, got {}",
            elevation.relief
        )));
    }
    for position in ordinary.positions() {
        for neighbor in ordinary.neighbors(position) {
            if position.level.abs_diff(neighbor.level) > 1 {
                issues.push(recipe_issue(
                    "Forest ordinary graph contains a transition over one level",
                ));
            }
        }
    }

    if !issues.is_empty() {
        return WorldValidation::Invalid(issues);
    }

    let protected_route_surfaces = plan
        .features
        .protected_routes
        .values()
        .flat_map(|route| route.surfaces.iter().copied())
        .collect::<BTreeSet<_>>();
    WorldValidation::Valid(ForestMetrics {
        tree_roots: count_u32(tree_roots.len()),
        tree_blocker_surfaces: count_u32(tree_blocker_surfaces.len()),
        old_growth_roots: count_u32(old_growth.len()),
        old_growth_blocker_surfaces: count_u32(old_growth_blocker_surfaces.len()),
        tall_grass_roots: count_u32(grass_roots.len()),
        woodland_surfaces: count_u32(woodland.len()),
        prairie_surfaces: count_u32(prairie.len()),
        clearing_count: count_u32(plan.features.clearings.len()),
        clearing_surfaces: count_u32(clearing_surfaces.len()),
        protected_route_surfaces: count_u32(protected_route_surfaces.len()),
        ordinary_surfaces: count_u32(ordinary.len()),
        reachable_elevation_levels: count_u32(elevation.reachable_levels.len()),
        relief: elevation.relief,
        spawn_height_difference,
        woodland_prairie_high_ground_difference: elevation.woodland_prairie_high_ground_difference,
        critical_route_steps,
    })
}

fn validate_review_anchors(
    plan: &GeneratedWorldPlan,
    rotation: u8,
    prairie: &BTreeSet<TilePos>,
    issues: &mut Vec<WorldValidationIssue>,
) {
    let required_names = BTreeSet::from([
        PARTY_START,
        HOSTILE_START,
        FOREST_CLEARING,
        PRAIRIE_OVERLOOK,
    ]);
    let actual_names: BTreeSet<_> = plan.anchors.keys().map(String::as_str).collect();
    let missing_names: BTreeSet<_> = required_names.difference(&actual_names).copied().collect();
    if !missing_names.is_empty() {
        issues.push(recipe_issue(format!(
            "Forest is missing required anchors {missing_names:?}"
        )));
    }

    let primary_clearing = plan.features.clearings.get("forest_clearing_0");
    let clearing_anchor = plan.anchors.get(FOREST_CLEARING);
    if !clearing_anchor
        .zip(primary_clearing)
        .is_some_and(|(anchor, clearing)| clearing.surfaces.contains(anchor))
    {
        issues.push(recipe_issue(
            "Forest forest_clearing anchor must name an exact surface in forest_clearing_0",
        ));
    }

    let radius = i32::try_from(plan.layout.grid_radius).unwrap_or(i32::MAX);
    let nominal_overlook_coord = rotate(HexCoord::from_axial(radius / 2, -(radius / 4)), rotation);
    let prairie_by_coord = prairie
        .iter()
        .copied()
        .map(|position| (position.coord, position))
        .collect::<BTreeMap<_, _>>();
    let expected_overlook_coord =
        nearest_ordinary_landing(nominal_overlook_coord, &prairie_by_coord, rotation, false);
    let expected_overlook = expected_overlook_coord.and_then(|coord| prairie_by_coord.get(&coord));
    if plan.anchors.get(PRAIRIE_OVERLOOK) != expected_overlook {
        issues.push(recipe_issue(format!(
            "Forest prairie_overlook anchor must name the exact prairie surface at \
             {expected_overlook_coord:?}"
        )));
    }
}

fn ordinary_elevation_metrics(ordinary: &OrdinaryGraph, rotation: u8) -> OrdinaryElevationMetrics {
    let positions: Vec<_> = ordinary.positions().collect();
    let reachable_levels: BTreeSet<_> = positions.iter().map(|position| position.level).collect();
    let min_level = reachable_levels.first().copied().unwrap_or(BASE_LEVEL);
    let max_level = reachable_levels.last().copied().unwrap_or(BASE_LEVEL);
    let woodland_high = positions
        .iter()
        .filter(|position| is_woodland(position.coord, rotation))
        .map(|position| position.level)
        .max()
        .unwrap_or(BASE_LEVEL);
    let prairie_high = positions
        .iter()
        .filter(|position| !is_woodland(position.coord, rotation))
        .map(|position| position.level)
        .max()
        .unwrap_or(BASE_LEVEL);

    OrdinaryElevationMetrics {
        reachable_levels,
        relief: min_level.abs_diff(max_level),
        woodland_prairie_high_ground_difference: woodland_high.abs_diff(prairie_high),
    }
}

fn validate_road(
    plan: &GeneratedWorldPlan,
    graph: &OrdinaryGraph,
    road: &ProtectedFeatureRoute,
    rotation: u8,
    route_offset: i32,
    trees: &BTreeSet<HexCoord>,
    stitched_approach_depth: Option<u32>,
    issues: &mut Vec<WorldValidationIssue>,
) {
    let radius = i32::try_from(plan.layout.grid_radius).unwrap_or(i32::MAX);
    if road.centerline.len() < usize::try_from(radius.max(0)).unwrap_or(usize::MAX) {
        issues.push(recipe_issue("Forest woodland road is too short"));
    }
    if road
        .centerline
        .windows(2)
        .any(|pair| !matches!(pair, [first, second] if graph.admits(*first, *second)))
    {
        issues.push(recipe_issue(
            "Forest woodland road centerline is not continuously walkable",
        ));
    }
    let Some(first) = road.centerline.first() else {
        return;
    };
    let Some(last) = road.centerline.last() else {
        return;
    };
    let expected_first_coord = rotate(
        HexCoord::from_axial((-radius).saturating_add(2), route_offset),
        rotation,
    );
    let expected_last_coord = rotate(
        HexCoord::from_axial(PRAIRIE_TAPER_DEPTH, route_offset),
        rotation,
    );
    let first_matches = if stitched_approach_depth.is_some() {
        let ordinary_surfaces = plan
            .volume
            .surfaces
            .iter()
            .filter_map(|(position, metadata)| {
                (metadata.access == SurfaceAccess::Ordinary).then_some((position.coord, *position))
            })
            .collect::<BTreeMap<_, _>>();
        nearest_ordinary_landing(expected_first_coord, &ordinary_surfaces, rotation, true)
            == Some(first.coord)
    } else {
        first.coord == expected_first_coord
    };
    if !first_matches
        || last.coord != expected_last_coord
        || plan.anchors.get(PARTY_START) != Some(first)
    {
        issues.push(recipe_issue(
            "Forest woodland road does not use the exact forest landing and prairie-taper endpoint",
        ));
    }

    let directions: Vec<_> = road
        .centerline
        .windows(2)
        .filter_map(|pair| match pair {
            [from, to] => Some((
                to.coord.x().saturating_sub(from.coord.x()),
                to.coord.y().saturating_sub(from.coord.y()),
                to.coord.z().saturating_sub(from.coord.z()),
            )),
            _ => None,
        })
        .collect();
    let turns = directions
        .windows(2)
        .filter(|pair| matches!(pair, [first, second] if first != second))
        .count();
    if turns < 4 {
        issues.push(recipe_issue(format!(
            "Forest woodland road requires at least four bends, got {turns}"
        )));
    }

    let surface_by_coord: BTreeMap<_, _> = plan
        .volume
        .surfaces
        .keys()
        .copied()
        .map(|surface| (surface.coord, surface))
        .collect();
    let centerline_surfaces: BTreeSet<_> = road.centerline.iter().copied().collect();
    let mut allowed_surfaces = centerline_surfaces.clone();
    let mut shoulder_parents = BTreeMap::<TilePos, BTreeSet<TilePos>>::new();
    for position in &road.centerline {
        let local = unrotate(position.coord, rotation);
        let shoulder_coord = rotate(
            HexCoord::from_axial(local.x(), local.y().saturating_add(1)),
            rotation,
        );
        let Some(shoulder) = surface_by_coord.get(&shoulder_coord).copied() else {
            continue;
        };
        allowed_surfaces.insert(shoulder);
        shoulder_parents
            .entry(shoulder)
            .or_default()
            .insert(*position);
    }
    if !centerline_surfaces.is_subset(&road.surfaces) {
        issues.push(recipe_issue(
            "Forest road footprint does not contain its complete ordered centerline",
        ));
    }
    if !road.surfaces.is_subset(&allowed_surfaces) {
        issues.push(recipe_issue(
            "Forest road footprint contains surfaces outside its centerline and canonical shoulders",
        ));
    }
    for shoulder in road.surfaces.difference(&centerline_surfaces) {
        if shoulder_parents.get(shoulder).is_none_or(|parents| {
            !parents
                .iter()
                .any(|centerline| graph.admits(*centerline, *shoulder))
        }) {
            issues.push(recipe_issue(format!(
                "Forest road shoulder {shoulder:?} is not walkable from its centerline"
            )));
        }
    }
    if !surface_set_connected(&road.surfaces, graph) {
        issues.push(recipe_issue(
            "Forest road surface footprint is not walker-connected",
        ));
    }

    let mut narrow = 0_usize;
    let mut woodland_cells = 0_usize;
    let mut woodland_narrow = 0_usize;
    let mut taper_depths = BTreeSet::new();
    for position in &road.centerline {
        let local = unrotate(position.coord, rotation);
        let shoulder_coord = rotate(
            HexCoord::from_axial(local.x(), local.y().saturating_add(1)),
            rotation,
        );
        let has_shoulder = surface_by_coord
            .get(&shoulder_coord)
            .is_some_and(|surface| road.surfaces.contains(surface));
        if !has_shoulder {
            narrow = narrow.saturating_add(1);
        }
        if local.x() <= 0 {
            woodland_cells = woodland_cells.saturating_add(1);
            woodland_narrow = woodland_narrow.saturating_add(usize::from(!has_shoulder));
        } else {
            taper_depths.insert(local.x());
        }
        if local.x() >= 2 && has_shoulder {
            issues.push(recipe_issue(
                "Forest road does not narrow across its final prairie taper",
            ));
        }
    }
    if woodland_narrow == 0 {
        issues.push(recipe_issue(
            "Forest woodland road requires at least one periodic one-tile-wide pinch",
        ));
    }
    if woodland_narrow.saturating_mul(2) > woodland_cells {
        issues.push(recipe_issue(format!(
            "Forest woodland road is not mostly two-wide: {woodland_narrow}/{woodland_cells} narrow cells"
        )));
    }
    if taper_depths != BTreeSet::from([1, 2, PRAIRIE_TAPER_DEPTH]) {
        issues.push(recipe_issue(
            "Forest road does not traverse the exact three-cell prairie taper",
        ));
    }
    if narrow.saturating_mul(2) > road.centerline.len() {
        issues.push(recipe_issue(format!(
            "Forest road must remain mostly two tiles wide, got {narrow}/{} narrow cells",
            road.centerline.len()
        )));
    }

    let gravel_surfaces: BTreeSet<_> = plan
        .volume
        .surfaces
        .keys()
        .copied()
        .filter(|surface| surface_material(plan, *surface) == Some(SolidMaterialRole::Gravel))
        .collect();
    if gravel_surfaces != road.surfaces {
        issues.push(recipe_issue(
            "Forest gravel does not exactly match the authored road footprint",
        ));
    }
    if road
        .surfaces
        .iter()
        .any(|surface| unrotate(surface.coord, rotation).x() > PRAIRIE_TAPER_DEPTH)
    {
        issues.push(recipe_issue(
            "Forest gravel continues beyond the prairie taper",
        ));
    }

    let woodland_road: Vec<_> = road
        .centerline
        .iter()
        .filter(|surface| is_woodland(surface.coord, rotation))
        .collect();
    let flanked = woodland_road
        .iter()
        .filter(|surface| {
            surface
                .coord
                .neighbors()
                .into_iter()
                .any(|neighbor| trees.contains(&neighbor))
        })
        .count();
    if flanked.saturating_mul(5) < woodland_road.len() {
        issues.push(recipe_issue(format!(
            "Forest road is not sufficiently tree-lined: {flanked}/{} woodland cells",
            woodland_road.len()
        )));
    }
}

fn surface_material(plan: &GeneratedWorldPlan, surface: TilePos) -> Option<SolidMaterialRole> {
    plan.volume
        .columns
        .get(&surface.coord)?
        .elements
        .iter()
        .find_map(|element| match element {
            VolumeElement::Solid(mass)
                if mass.levels.bottom <= surface.level && surface.level < mass.levels.top =>
            {
                Some(mass.material)
            }
            VolumeElement::Solid(_) | VolumeElement::Fill(_) => None,
        })
}

fn detect_orientation(plan: &GeneratedWorldPlan) -> Option<(u8, i32)> {
    let party = plan.anchors.get(PARTY_START)?;
    let hostile = plan.anchors.get(HOSTILE_START)?;
    let radius = i32::try_from(plan.layout.grid_radius).ok()?;
    for rotation in 0..6_u8 {
        for offset in -1..=1 {
            let expected_party = rotate(
                HexCoord::from_axial((-radius).saturating_add(2), offset),
                rotation,
            );
            let expected_hostile = rotate(
                HexCoord::from_axial(radius.saturating_sub(2), offset),
                rotation,
            );
            if party.coord == expected_party && hostile.coord == expected_hostile {
                return Some((rotation, offset));
            }
        }
    }
    None
}

fn detect_stitched_orientation(plan: &GeneratedWorldPlan) -> Option<(u8, i32)> {
    let party = plan.anchors.get(PARTY_START)?;
    let hostile = plan.anchors.get(HOSTILE_START)?;
    let road = plan.features.protected_routes.get(ROAD_ROUTE)?;
    let road_start = road.centerline.first()?;
    let road_end = road.centerline.last()?;
    let radius = i32::try_from(plan.layout.grid_radius).ok()?;
    let ordinary_surfaces = plan
        .volume
        .surfaces
        .iter()
        .filter_map(|(position, metadata)| {
            (metadata.access == SurfaceAccess::Ordinary).then_some((position.coord, *position))
        })
        .collect::<BTreeMap<_, _>>();
    for rotation in 0..6_u8 {
        for offset in -1..=1 {
            let nominal_party = rotate(
                HexCoord::from_axial((-radius).saturating_add(2), offset),
                rotation,
            );
            let nominal_hostile = rotate(
                HexCoord::from_axial(radius.saturating_sub(2), offset),
                rotation,
            );
            let expected_road_end =
                rotate(HexCoord::from_axial(PRAIRIE_TAPER_DEPTH, offset), rotation);
            let expected_party =
                nearest_ordinary_landing(nominal_party, &ordinary_surfaces, rotation, true)?;
            let expected_hostile =
                nearest_ordinary_landing(nominal_hostile, &ordinary_surfaces, rotation, false)?;
            if party.coord == road_start.coord
                && party.coord == expected_party
                && hostile.coord == expected_hostile
                && road_end.coord == expected_road_end
            {
                return Some((rotation, offset));
            }
        }
    }
    None
}

fn plan_road(
    rotation: u8,
    route_offset: i32,
    start: HexCoord,
    mask: &BTreeSet<HexCoord>,
    surfaces: &BTreeMap<HexCoord, TilePos>,
    trees: &BTreeSet<HexCoord>,
    clearings: &[PlannedClearing],
    stream: Option<SeedStream<'_>>,
    stitched_patch: bool,
) -> Result<PlannedRoad, Vec<WorldValidationIssue>> {
    let taper = (0..=PRAIRIE_TAPER_DEPTH)
        .map(|x| rotate(HexCoord::from_axial(x, route_offset), rotation))
        .collect::<Vec<_>>();
    let (road_end, maximum_local_x) = if stitched_patch {
        if taper.iter().any(|coord| {
            !mask.contains(coord) || !surfaces.contains_key(coord) || trees.contains(coord)
        }) || taper.windows(2).any(|pair| {
            !matches!(pair, [from, to]
                if from.distance(*to) == 1
                    && surfaces
                        .get(from)
                        .zip(surfaces.get(to))
                        .is_some_and(|(first, second)| first.level.abs_diff(second.level) <= 1))
        }) {
            return Err(vec![recipe_issue(
                "Forest road cannot fit its exact three-cell prairie taper",
            )]);
        }
        let taper_start = taper.first().copied().ok_or_else(|| {
            vec![recipe_issue(
                "Forest road cannot resolve its prairie taper start",
            )]
        })?;
        (taper_start, 0)
    } else {
        (
            rotate(
                HexCoord::from_axial(PRAIRIE_TAPER_DEPTH, route_offset),
                rotation,
            ),
            PRAIRIE_TAPER_DEPTH,
        )
    };
    let mut ordered_clearings: Vec<_> = clearings.iter().map(|clearing| clearing.center).collect();
    ordered_clearings
        .sort_unstable_by_key(|coord| (unrotate(*coord, rotation).x(), std::cmp::Reverse(*coord)));
    if ordered_clearings.len() < 4 {
        return Err(vec![recipe_issue(
            "Forest road requires four clearings for its winding guide",
        )]);
    }
    let (Some(early_clearings), Some(late_clearings)) =
        (ordered_clearings.get(..2), ordered_clearings.get(2..))
    else {
        return Err(vec![recipe_issue(
            "Forest road cannot partition its clearing guides",
        )]);
    };

    let mut early = early_clearings.to_vec();
    early.sort_unstable_by_key(|coord| (feature_priority(stream, *coord, 200), *coord));
    let early = early
        .first()
        .copied()
        .ok_or_else(|| vec![recipe_issue("Forest road has no early clearing")])?;
    let early_y = unrotate(early, rotation).y();
    let mut late = late_clearings.to_vec();
    late.sort_unstable_by_key(|coord| {
        let local_y = unrotate(*coord, rotation).y();
        (
            (local_y.signum() == early_y.signum()) as u8,
            feature_priority(stream, *coord, 201),
            *coord,
        )
    });
    let late = late
        .first()
        .copied()
        .ok_or_else(|| vec![recipe_issue("Forest road has no late clearing")])?;

    let mut centerline = Vec::new();
    for (segment_index, pair) in [start, early, late, road_end].windows(2).enumerate() {
        let [from, to] = pair else {
            continue;
        };
        let mut forbidden: BTreeSet<_> = centerline.iter().copied().collect();
        forbidden.remove(from);
        let segment = find_road_segment(
            *from,
            *to,
            rotation,
            mask,
            surfaces,
            trees,
            &forbidden,
            stream,
            u64::try_from(segment_index).unwrap_or_default(),
            maximum_local_x,
        )
        .ok_or_else(|| {
            vec![recipe_issue(format!(
                "Forest road cannot connect guide points {from:?} and {to:?}"
            ))]
        })?;
        centerline.extend(
            segment
                .into_iter()
                .skip(usize::from(!centerline.is_empty())),
        );
    }
    if stitched_patch {
        centerline.extend(taper.into_iter().skip(1));
    }
    if centerline.iter().copied().collect::<BTreeSet<_>>().len() != centerline.len() {
        return Err(vec![recipe_issue(
            "Forest road guide produced a self-intersecting centerline",
        )]);
    }

    let phase = stream.map_or(0, |stream| {
        usize::try_from(stream.sample(300) % 7).unwrap_or_default()
    });
    let centerline_set: BTreeSet<_> = centerline.iter().copied().collect();
    let mut road_surfaces = centerline_set.clone();
    for (index, coord) in centerline.iter().copied().enumerate() {
        let local = unrotate(coord, rotation);
        let deliberately_narrow =
            local.x() >= 2 || (local.x() <= 0 && index.saturating_add(phase) % 7 == 3);
        if deliberately_narrow {
            continue;
        }
        let shoulder_local = HexCoord::from_axial(local.x(), local.y().saturating_add(1));
        let shoulder = rotate(shoulder_local, rotation);
        if mask.contains(&shoulder)
            && !trees.contains(&shoulder)
            && !centerline_set.contains(&shoulder)
            && surfaces
                .get(&coord)
                .zip(surfaces.get(&shoulder))
                .is_some_and(|(first, second)| first.level.abs_diff(second.level) <= 1)
        {
            road_surfaces.insert(shoulder);
        }
    }

    Ok(PlannedRoad {
        centerline,
        surfaces: road_surfaces,
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "the pure path search receives each exact semantic input explicitly"
)]
fn find_road_segment(
    start: HexCoord,
    goal: HexCoord,
    rotation: u8,
    mask: &BTreeSet<HexCoord>,
    surfaces: &BTreeMap<HexCoord, TilePos>,
    trees: &BTreeSet<HexCoord>,
    forbidden: &BTreeSet<HexCoord>,
    stream: Option<SeedStream<'_>>,
    salt: u64,
    maximum_local_x: i32,
) -> Option<Vec<HexCoord>> {
    if trees.contains(&start) || trees.contains(&goal) {
        return None;
    }
    let mut frontier = BTreeSet::from([(0_u64, 0_u32, start)]);
    let mut best = BTreeMap::from([(start, (0_u64, 0_u32))]);
    let mut parents = BTreeMap::new();

    while let Some((cost, steps, coord)) = frontier.pop_first() {
        if coord == goal {
            let mut path = vec![goal];
            let mut cursor = goal;
            while cursor != start {
                cursor = *parents.get(&cursor)?;
                path.push(cursor);
            }
            path.reverse();
            return Some(path);
        }
        if best.get(&coord).copied() != Some((cost, steps)) {
            continue;
        }

        let mut neighbors = coord.neighbors();
        neighbors.sort_unstable();
        for neighbor in neighbors {
            if !mask.contains(&neighbor)
                || trees.contains(&neighbor)
                || (forbidden.contains(&neighbor) && neighbor != goal)
                || unrotate(neighbor, rotation).x() > maximum_local_x
            {
                continue;
            }
            let Some((from, to)) = surfaces.get(&coord).zip(surfaces.get(&neighbor)) else {
                continue;
            };
            if from.level.abs_diff(to.level) > 1 {
                continue;
            }
            let flanking_trees = neighbor
                .neighbors()
                .into_iter()
                .filter(|adjacent| trees.contains(adjacent))
                .count();
            let flank_discount =
                u64::try_from(flanking_trees.saturating_mul(12)).unwrap_or(u64::MAX);
            let jitter = stream.map_or(0, |stream| {
                stream.sample_coord(neighbor, salt.saturating_add(400)) % 7
            });
            let step_cost = 100_u64.saturating_sub(flank_discount.min(36)) + jitter;
            let next = (cost.saturating_add(step_cost), steps.saturating_add(1));
            let improves = best.get(&neighbor).is_none_or(|current| next < *current);
            if improves {
                best.insert(neighbor, next);
                parents.insert(neighbor, coord);
                frontier.insert((next.0, next.1, neighbor));
            }
        }
    }
    None
}

#[derive(Debug)]
struct PlannedClearing {
    center: HexCoord,
    coords: BTreeSet<HexCoord>,
}

fn clearing_coordinates(
    radius: i32,
    rotation: u8,
    mask: &BTreeSet<HexCoord>,
    woodland: &BTreeSet<HexCoord>,
    stream: Option<SeedStream<'_>>,
    allow_relocation: bool,
) -> Result<Vec<PlannedClearing>, Vec<WorldValidationIssue>> {
    let base_centers = [
        HexCoord::from_axial(-(radius.saturating_mul(3) / 5), -(radius / 6)),
        HexCoord::from_axial(-(radius / 2), radius.saturating_mul(2) / 5),
        HexCoord::from_axial(-(radius / 3), -(radius.saturating_mul(5) / 12)),
        HexCoord::from_axial(-(radius / 4), radius / 4),
    ];
    let mut clearings = Vec::new();
    let mut claimed = BTreeSet::new();
    for (index, base) in base_centers.into_iter().enumerate() {
        let nominal = rotate(base, rotation);
        let initial_options = std::iter::once(nominal)
            .chain(nominal.neighbors())
            .collect::<Vec<_>>();
        let sampled = stream.and_then(|stream| {
            let option_count = u64::try_from(initial_options.len()).ok()?;
            let sampled = usize::try_from(
                stream.sample(u64::try_from(index).unwrap_or_default()) % option_count,
            )
            .ok()?;
            initial_options.get(sampled).copied()
        });
        if !allow_relocation {
            let center = sampled.unwrap_or(nominal);
            let Some(interior) = clearing_footprint(center, mask, woodland, &claimed, stream)
            else {
                return Err(vec![recipe_issue(format!(
                    "Forest clearing {index} cannot fit ten irregular surfaces"
                ))]);
            };
            claimed.extend(interior.iter().copied());
            clearings.push(PlannedClearing {
                center,
                coords: interior,
            });
            continue;
        }
        let mut candidates = woodland.iter().copied().collect::<Vec<_>>();
        candidates.sort_unstable_by_key(|coord| {
            (
                usize::from(Some(*coord) != sampled),
                nominal.distance(*coord),
                feature_priority(stream, *coord, 90),
                *coord,
            )
        });
        let Some((center, interior)) = candidates.into_iter().find_map(|center| {
            clearing_footprint(center, mask, woodland, &claimed, stream)
                .map(|interior| (center, interior))
        }) else {
            return Err(vec![recipe_issue(format!(
                "Forest clearing {index} cannot fit ten distinct woodland surfaces near its \
                 nominal center"
            ))]);
        };
        claimed.extend(interior.iter().copied());
        clearings.push(PlannedClearing {
            center,
            coords: interior,
        });
    }
    Ok(clearings)
}

fn clearing_footprint(
    center: HexCoord,
    mask: &BTreeSet<HexCoord>,
    woodland: &BTreeSet<HexCoord>,
    claimed: &BTreeSet<HexCoord>,
    stream: Option<SeedStream<'_>>,
) -> Option<BTreeSet<HexCoord>> {
    if !mask.contains(&center) || !woodland.contains(&center) || claimed.contains(&center) {
        return None;
    }
    let mut interior = BTreeSet::new();
    let mut optional_edge = Vec::new();
    for coord in center.within_radius(2) {
        if !mask.contains(&coord) || !woodland.contains(&coord) || claimed.contains(&coord) {
            continue;
        }
        if center.distance(coord) <= 1 {
            interior.insert(coord);
        } else {
            optional_edge.push(coord);
        }
    }
    optional_edge.sort_unstable_by_key(|coord| (feature_priority(stream, *coord, 100), *coord));
    for coord in &optional_edge {
        if feature_priority(stream, *coord, 101) % 100 < 68 {
            interior.insert(*coord);
        }
    }
    for coord in optional_edge {
        if interior.len() >= 10 {
            break;
        }
        interior.insert(coord);
    }
    (interior.len() >= 10).then_some(interior)
}

fn select_tree_root_candidates(
    woodland: &BTreeSet<HexCoord>,
    exclusions: &BTreeSet<HexCoord>,
    surfaces: &BTreeMap<HexCoord, TilePos>,
    stream: Option<SeedStream<'_>>,
) -> Vec<TilePos> {
    let mut eligible: Vec<_> = woodland.difference(exclusions).copied().collect();
    eligible.sort_unstable_by_key(|coord| (feature_priority(stream, *coord, 0), *coord));
    let mut color_counts = [0_usize; 3];
    for coord in &eligible {
        let color =
            usize::try_from(coord.x().saturating_sub(coord.y()).rem_euclid(3)).unwrap_or_default();
        if let Some(count) = color_counts.get_mut(color) {
            *count = count.saturating_add(1);
        }
    }
    let phase = color_counts
        .into_iter()
        .enumerate()
        .min_by_key(|(phase, count)| {
            (
                std::cmp::Reverse(*count),
                stream.map_or(u64::try_from(*phase).unwrap_or_default(), |stream| {
                    stream.sample(700_u64.saturating_add(u64::try_from(*phase).unwrap_or_default()))
                }),
                *phase,
            )
        })
        .map(|(phase, _)| phase)
        .unwrap_or_default();
    eligible.sort_by_key(|coord| {
        let color =
            usize::try_from(coord.x().saturating_sub(coord.y()).rem_euclid(3)).unwrap_or_default();
        (color != phase, feature_priority(stream, *coord, 1), *coord)
    });
    eligible
        .into_iter()
        .filter_map(|coord| surfaces.get(&coord).copied())
        .collect()
}

fn plan_tree_features(
    root_candidates: Vec<TilePos>,
    target: usize,
    minimum: usize,
    woodland: &BTreeSet<HexCoord>,
    exclusions: &BTreeSet<HexCoord>,
    surfaces: &BTreeMap<HexCoord, TilePos>,
    objects: &TemperateVegetationSet,
    object_stream: Option<SeedStream<'_>>,
    rotation_stream: Option<SeedStream<'_>>,
) -> Result<(Vec<PlannedFeature>, BTreeSet<TilePos>), Vec<WorldValidationIssue>> {
    let mut best = select_capacity_plan(
        &root_candidates,
        target,
        minimum,
        object_stream.is_none(),
        woodland,
        exclusions,
        surfaces,
        objects,
        object_stream,
        rotation_stream,
    )?;
    if best.len() < minimum {
        return Err(capacity_issue(best.len(), minimum, target));
    }
    match upgrade_old_growth(
        &mut best,
        woodland,
        exclusions,
        surfaces,
        objects,
        object_stream,
        rotation_stream,
    ) {
        Ok(()) => {}
        Err(issues) if old_growth_capacity_failed(&issues) => {
            let mut reserved = select_capacity_plan(
                &root_candidates,
                target,
                minimum,
                true,
                woodland,
                exclusions,
                surfaces,
                objects,
                object_stream,
                rotation_stream,
            )?;
            if reserved.len() < minimum {
                return Err(capacity_issue(reserved.len(), minimum, target));
            }
            upgrade_old_growth(
                &mut reserved,
                woodland,
                exclusions,
                surfaces,
                objects,
                object_stream,
                rotation_stream,
            )?;
            best = reserved;
        }
        Err(issues) => return Err(issues),
    }
    best.sort_unstable_by_key(|feature| feature.root);
    let final_visual_cells = collect_tree_visual_cells(&best, objects)?;
    Ok((best, final_visual_cells))
}

#[expect(
    clippy::too_many_arguments,
    reason = "capacity selection retains each authored spatial constraint explicitly"
)]
fn select_capacity_plan(
    root_candidates: &[TilePos],
    target: usize,
    minimum: usize,
    reserve_old_growth: bool,
    woodland: &BTreeSet<HexCoord>,
    exclusions: &BTreeSet<HexCoord>,
    surfaces: &BTreeMap<HexCoord, TilePos>,
    objects: &TemperateVegetationSet,
    object_stream: Option<SeedStream<'_>>,
    rotation_stream: Option<SeedStream<'_>>,
) -> Result<Vec<PlannedFeature>, Vec<WorldValidationIssue>> {
    let mut best = Vec::new();
    let capacity_trials = [(0_i32, 4_u64), (0, 0), (1, 0), (2, 0)];
    for (phase, schedule) in capacity_trials {
        let features = plan_tree_phase(
            root_candidates,
            target,
            phase,
            schedule,
            reserve_old_growth,
            woodland,
            exclusions,
            surfaces,
            objects,
            object_stream,
            rotation_stream,
        )?;
        let missing = minimum.saturating_sub(features.len());
        let features = if (1..=3).contains(&missing) {
            augment_tree_capacity(
                features,
                root_candidates,
                minimum,
                woodland,
                exclusions,
                surfaces,
                objects,
                object_stream,
                rotation_stream,
            )?
        } else {
            features
        };
        if features.len() > best.len() {
            best = features;
        }
        if best.len() >= minimum {
            break;
        }
    }
    Ok(best)
}

fn old_growth_capacity_failed(issues: &[WorldValidationIssue]) -> bool {
    issues.len() == 1
        && issues
            .first()
            .is_some_and(|issue| issue.detail == OLD_GROWTH_CAPACITY_DETAIL)
}

fn capacity_issue(placed: usize, minimum: usize, target: usize) -> Vec<WorldValidationIssue> {
    vec![recipe_issue(format!(
        "Forest structural vegetation can place only {placed} trees across its bounded legal \
         capacity trials; at least {minimum} of the target {target} are required"
    ))]
}

#[expect(
    clippy::too_many_arguments,
    reason = "capacity augmentation retains each authored spatial constraint explicitly"
)]
fn augment_tree_capacity(
    mut features: Vec<PlannedFeature>,
    root_candidates: &[TilePos],
    target: usize,
    woodland: &BTreeSet<HexCoord>,
    exclusions: &BTreeSet<HexCoord>,
    surfaces: &BTreeMap<HexCoord, TilePos>,
    objects: &TemperateVegetationSet,
    object_stream: Option<SeedStream<'_>>,
    rotation_stream: Option<SeedStream<'_>>,
) -> Result<Vec<PlannedFeature>, Vec<WorldValidationIssue>> {
    let mut projection_cache = BTreeMap::new();
    while features.len() < target {
        let (all_blockers, all_structural, all_roots) = tree_occupancy(&features, objects)?;
        let structural_by_feature = features
            .iter()
            .map(|feature| tree_feature_structural_cells(feature, objects))
            .collect::<Result<Vec<_>, _>>()?;
        let mut replacement = None;
        for removed_index in 0..features.len() {
            let Some(removed) = features.get(removed_index) else {
                return Err(vec![recipe_issue(
                    "Forest capacity removal left the feature collection",
                )]);
            };
            if removed.object_id.as_str() == OLD_GROWTH_ID {
                continue;
            }
            let remaining = features
                .iter()
                .enumerate()
                .filter_map(|(index, feature)| (index != removed_index).then_some(feature.clone()))
                .collect::<Vec<_>>();
            let mut base_blockers = all_blockers.clone();
            for blocker in &removed.blocker_footprint {
                base_blockers.remove(blocker);
            }
            let mut base_structural = all_structural.clone();
            let Some(removed_structural) = structural_by_feature.get(removed_index) else {
                return Err(vec![recipe_issue(
                    "Forest capacity removal left its structural projection",
                )]);
            };
            for structural in removed_structural {
                base_structural.remove(structural);
            }
            let mut base_roots = all_roots.clone();
            base_roots.remove(&removed.root.coord);
            let mut candidates = root_candidates
                .iter()
                .copied()
                .filter(|root| {
                    !base_roots.contains(&root.coord)
                        && !root
                            .coord
                            .neighbors()
                            .into_iter()
                            .any(|neighbor| base_roots.contains(&neighbor))
                })
                .collect::<Vec<_>>();
            candidates.sort_unstable_by_key(|root| {
                (
                    feature_priority(
                        object_stream,
                        root.coord,
                        811_u64.saturating_add(u64::try_from(features.len()).unwrap_or(u64::MAX)),
                    ),
                    *root,
                )
            });
            for (first_index, first_root) in candidates.iter().copied().enumerate() {
                let Some(first) = cached_capacity_tree_options(
                    &mut projection_cache,
                    first_root,
                    woodland,
                    exclusions,
                    surfaces,
                    objects,
                    object_stream,
                    rotation_stream,
                )?
                .iter()
                .find(|option| {
                    option.feature.blocker_footprint.is_disjoint(&base_blockers)
                        && option.structural_cells.is_disjoint(&base_structural)
                })
                .cloned() else {
                    continue;
                };
                let mut first_blockers = base_blockers.clone();
                first_blockers.extend(first.feature.blocker_footprint.iter().copied());
                let mut first_structure = base_structural.clone();
                first_structure.extend(first.structural_cells.iter().copied());
                for second_root in candidates
                    .iter()
                    .copied()
                    .skip(first_index.saturating_add(1))
                {
                    if first.feature.root.coord.distance(second_root.coord) <= 1 {
                        continue;
                    }
                    let Some(second) = cached_capacity_tree_options(
                        &mut projection_cache,
                        second_root,
                        woodland,
                        exclusions,
                        surfaces,
                        objects,
                        object_stream,
                        rotation_stream,
                    )?
                    .iter()
                    .find(|option| {
                        option
                            .feature
                            .blocker_footprint
                            .is_disjoint(&first_blockers)
                            && option.structural_cells.is_disjoint(&first_structure)
                    })
                    .cloned() else {
                        continue;
                    };
                    let mut improved = remaining.clone();
                    improved.extend([first.feature.clone(), second.feature.clone()]);
                    replacement = Some(improved);
                    break;
                }
                if replacement.is_some() {
                    break;
                }
            }
            if replacement.is_some() {
                break;
            }
        }
        let Some(improved) = replacement else {
            break;
        };
        features = improved;
    }
    Ok(features)
}

#[derive(Debug, Clone)]
struct CapacityTreeOption {
    feature: PlannedFeature,
    structural_cells: BTreeSet<TilePos>,
}

fn tree_occupancy(
    features: &[PlannedFeature],
    objects: &TemperateVegetationSet,
) -> Result<(BTreeSet<TilePos>, BTreeSet<TilePos>, BTreeSet<HexCoord>), Vec<WorldValidationIssue>> {
    let mut blockers = BTreeSet::new();
    let mut structural = BTreeSet::new();
    let mut roots = BTreeSet::new();
    for feature in features {
        let Some(object) = objects.forest_object(feature.object_id.as_str()) else {
            return Err(vec![recipe_issue(format!(
                "Forest capacity trial found unsupported object '{}'",
                feature.object_id
            ))]);
        };
        let Some(volume) = object.project_visual_volume(feature.root, feature.rotation) else {
            return Err(vec![recipe_issue(format!(
                "Forest capacity trial cannot project object '{}' at {:?}",
                feature.object_id, feature.root
            ))]);
        };
        blockers.extend(feature.blocker_footprint.iter().copied());
        structural.extend(volume.structural_cells);
        roots.insert(feature.root.coord);
    }
    Ok((blockers, structural, roots))
}

fn tree_feature_structural_cells(
    feature: &PlannedFeature,
    objects: &TemperateVegetationSet,
) -> Result<BTreeSet<TilePos>, Vec<WorldValidationIssue>> {
    let Some(object) = objects.forest_object(feature.object_id.as_str()) else {
        return Err(vec![recipe_issue(format!(
            "Forest capacity trial found unsupported object '{}'",
            feature.object_id
        ))]);
    };
    object
        .project_visual_volume(feature.root, feature.rotation)
        .map(|volume| volume.structural_cells)
        .ok_or_else(|| {
            vec![recipe_issue(format!(
                "Forest capacity trial cannot project object '{}' at {:?}",
                feature.object_id, feature.root
            ))]
        })
}

#[expect(
    clippy::too_many_arguments,
    reason = "the lazy cache retains every authored projection input explicitly"
)]
fn cached_capacity_tree_options<'a>(
    cache: &'a mut BTreeMap<TilePos, Vec<CapacityTreeOption>>,
    root: TilePos,
    woodland: &BTreeSet<HexCoord>,
    exclusions: &BTreeSet<HexCoord>,
    surfaces: &BTreeMap<HexCoord, TilePos>,
    objects: &TemperateVegetationSet,
    object_stream: Option<SeedStream<'_>>,
    rotation_stream: Option<SeedStream<'_>>,
) -> Result<&'a [CapacityTreeOption], Vec<WorldValidationIssue>> {
    #[cfg(test)]
    CAPACITY_PROJECTION_CACHE_PEAK.fetch_max(
        cache
            .len()
            .saturating_add(usize::from(!cache.contains_key(&root))),
        Ordering::Relaxed,
    );
    match cache.entry(root) {
        std::collections::btree_map::Entry::Occupied(entry) => Ok(entry.into_mut().as_slice()),
        std::collections::btree_map::Entry::Vacant(entry) => {
            let options = capacity_tree_options(
                root,
                woodland,
                exclusions,
                surfaces,
                objects,
                object_stream,
                rotation_stream,
            )?;
            Ok(entry.insert(options).as_slice())
        }
    }
}

fn capacity_tree_options(
    root: TilePos,
    woodland: &BTreeSet<HexCoord>,
    exclusions: &BTreeSet<HexCoord>,
    surfaces: &BTreeMap<HexCoord, TilePos>,
    objects: &TemperateVegetationSet,
    object_stream: Option<SeedStream<'_>>,
    rotation_stream: Option<SeedStream<'_>>,
) -> Result<Vec<CapacityTreeOption>, Vec<WorldValidationIssue>> {
    let family = tree_family(feature_priority(object_stream, root.coord, 17));
    let choices = match family {
        TreeFamily::SmallBroadleaf => [&objects.small_broadleaf, &objects.tall_narrow],
        TreeFamily::TallNarrow => [&objects.tall_narrow, &objects.small_broadleaf],
    };
    let first_rotation = feature_rotation(rotation_stream, root.coord, 29)?;
    let empty = BTreeSet::new();
    let mut options = Vec::new();
    for object in choices {
        for offset in 0..6 {
            let rotation = offset_rotation(first_rotation, offset)?;
            let Some(projected) = project_tree(
                object, root, rotation, woodland, exclusions, surfaces, &empty, &empty,
            ) else {
                continue;
            };
            options.push(CapacityTreeOption {
                feature: PlannedFeature {
                    root,
                    kind: FeatureKind::Tree,
                    object_id: object.id.clone(),
                    rotation,
                    blocker_footprint: projected.blockers,
                },
                structural_cells: projected.structural_cells,
            });
        }
    }
    Ok(options)
}

#[expect(
    clippy::too_many_arguments,
    reason = "capacity trials retain each authored spatial constraint explicitly"
)]
fn plan_tree_phase(
    root_candidates: &[TilePos],
    target: usize,
    phase: i32,
    schedule: u64,
    reserve_old_growth: bool,
    woodland: &BTreeSet<HexCoord>,
    exclusions: &BTreeSet<HexCoord>,
    surfaces: &BTreeMap<HexCoord, TilePos>,
    objects: &TemperateVegetationSet,
    object_stream: Option<SeedStream<'_>>,
    rotation_stream: Option<SeedStream<'_>>,
) -> Result<Vec<PlannedFeature>, Vec<WorldValidationIssue>> {
    let mut occupied_blockers = BTreeSet::new();
    let mut occupied_structural_cells = BTreeSet::new();
    let mut occupied_roots = BTreeSet::new();
    let mut features = Vec::with_capacity(target);
    let mut ordered = root_candidates.to_vec();
    let candidate_coords = root_candidates
        .iter()
        .map(|root| root.coord)
        .collect::<BTreeSet<_>>();
    ordered.sort_unstable_by_key(|root| {
        let phased = schedule < 4;
        let degree = root
            .coord
            .neighbors()
            .into_iter()
            .filter(|neighbor| candidate_coords.contains(neighbor))
            .count();
        (
            phased && tree_root_phase(root.coord) != phase,
            if schedule == 4 { degree } else { 0 },
            feature_priority(object_stream, root.coord, 703_u64.saturating_add(schedule)),
            *root,
        )
    });
    if reserve_old_growth {
        for root in ordered.iter().copied() {
            let first_rotation = feature_rotation(rotation_stream, root.coord, 29)?;
            let mut selected = None;
            for offset in 0..6 {
                let rotation = offset_rotation(first_rotation, offset)?;
                if let Some(projected) = project_tree(
                    &objects.old_growth,
                    root,
                    rotation,
                    woodland,
                    exclusions,
                    surfaces,
                    &occupied_blockers,
                    &occupied_structural_cells,
                ) {
                    selected = Some((rotation, projected));
                    break;
                }
            }
            let Some((rotation, projected)) = selected else {
                continue;
            };
            occupied_blockers.extend(projected.blockers.iter().copied());
            occupied_structural_cells.extend(projected.structural_cells.iter().copied());
            occupied_roots.insert(root.coord);
            features.push(PlannedFeature {
                root,
                kind: FeatureKind::Tree,
                object_id: objects.old_growth.id.clone(),
                rotation,
                blocker_footprint: projected.blockers,
            });
            break;
        }
    }
    for root in ordered {
        if features.len() >= target {
            break;
        }
        if occupied_roots.contains(&root.coord)
            || root
                .coord
                .neighbors()
                .into_iter()
                .any(|neighbor| occupied_roots.contains(&neighbor))
        {
            continue;
        }
        let family_hash = feature_priority(object_stream, root.coord, 17);
        let family = tree_family(family_hash);
        let first_rotation = feature_rotation(rotation_stream, root.coord, 29)?;
        let preferred = objects.forest_tree(family);
        let secondary = match family {
            TreeFamily::SmallBroadleaf => &objects.tall_narrow,
            TreeFamily::TallNarrow => &objects.small_broadleaf,
        };
        let mut selected = None;
        for object in [preferred, secondary] {
            for offset in 0..6 {
                let rotation = offset_rotation(first_rotation, offset)?;
                if let Some(projected) = project_tree(
                    object,
                    root,
                    rotation,
                    woodland,
                    exclusions,
                    surfaces,
                    &occupied_blockers,
                    &occupied_structural_cells,
                ) {
                    selected = Some((object, rotation, projected));
                    break;
                }
            }
            if selected.is_some() {
                break;
            }
        }
        let Some((object, rotation, projected)) = selected else {
            continue;
        };
        occupied_blockers.extend(projected.blockers.iter().copied());
        occupied_structural_cells.extend(projected.structural_cells.iter().copied());
        occupied_roots.insert(root.coord);
        features.push(PlannedFeature {
            root,
            kind: FeatureKind::Tree,
            object_id: object.id.clone(),
            rotation,
            blocker_footprint: projected.blockers,
        });
    }
    Ok(features)
}

fn tree_root_phase(coord: HexCoord) -> i32 {
    coord.x().saturating_sub(coord.y()).rem_euclid(3)
}

fn upgrade_old_growth(
    features: &mut [PlannedFeature],
    woodland: &BTreeSet<HexCoord>,
    exclusions: &BTreeSet<HexCoord>,
    surfaces: &BTreeMap<HexCoord, TilePos>,
    objects: &TemperateVegetationSet,
    object_stream: Option<SeedStream<'_>>,
    rotation_stream: Option<SeedStream<'_>>,
) -> Result<(), Vec<WorldValidationIssue>> {
    let mut upgraded = features
        .iter()
        .filter(|feature| feature.object_id.as_str() == OLD_GROWTH_ID)
        .count();
    let Some(object_stream) = object_stream else {
        return if upgraded > 0 {
            Ok(())
        } else {
            Err(vec![recipe_issue(OLD_GROWTH_CAPACITY_DETAIL)])
        };
    };
    let target = features.len().saturating_mul(12).div_ceil(100).max(1);
    let mut ranked_indices = features
        .iter()
        .enumerate()
        .map(|(index, feature)| {
            (
                feature_priority(Some(object_stream), feature.root.coord, 17),
                feature.root,
                index,
            )
        })
        .collect::<Vec<_>>();
    ranked_indices.sort_unstable();
    let mut structural_by_feature = Vec::with_capacity(features.len());
    let mut all_blockers = BTreeSet::new();
    let mut all_structural_cells = BTreeSet::new();
    for feature in features.iter() {
        let Some(object) = objects.forest_object(feature.object_id.as_str()) else {
            return Err(vec![recipe_issue(format!(
                "Forest feature at {:?} uses unsupported authored object '{}'",
                feature.root, feature.object_id
            ))]);
        };
        let Some(volume) = object.project_visual_volume(feature.root, feature.rotation) else {
            return Err(vec![recipe_issue(format!(
                "Forest object '{}' cannot project its complete authored bounds at {:?}",
                feature.object_id, feature.root
            ))]);
        };
        all_blockers.extend(feature.blocker_footprint.iter().copied());
        all_structural_cells.extend(volume.structural_cells.iter().copied());
        structural_by_feature.push(volume.structural_cells);
    }
    for (_, _, index) in ranked_indices {
        if upgraded >= target {
            break;
        }
        let Some(current_feature) = features.get(index).cloned() else {
            return Err(vec![recipe_issue(
                "Forest old-growth ranking left the feature collection",
            )]);
        };
        let Some(current_structural_cells) = structural_by_feature.get(index).cloned() else {
            return Err(vec![recipe_issue(
                "Forest old-growth structural projection left the feature collection",
            )]);
        };
        if current_feature.object_id.as_str() == OLD_GROWTH_ID {
            continue;
        }
        let root = current_feature.root;
        let occupied_blockers = all_blockers
            .difference(&current_feature.blocker_footprint)
            .copied()
            .collect();
        let occupied_structural_cells = all_structural_cells
            .difference(&current_structural_cells)
            .copied()
            .collect();

        let first_rotation = feature_rotation(rotation_stream, root.coord, 29)?;
        for offset in 0..6 {
            let rotation = offset_rotation(first_rotation, offset)?;
            let Some(projected) = project_tree(
                &objects.old_growth,
                root,
                rotation,
                woodland,
                exclusions,
                surfaces,
                &occupied_blockers,
                &occupied_structural_cells,
            ) else {
                continue;
            };
            let replacement = PlannedFeature {
                root,
                kind: FeatureKind::Tree,
                object_id: objects.old_growth.id.clone(),
                rotation,
                blocker_footprint: projected.blockers,
            };
            let mut candidate = features.to_vec();
            let Some(candidate_slot) = candidate.get_mut(index) else {
                return Err(vec![recipe_issue(
                    "Forest old-growth candidate left the feature collection",
                )]);
            };
            *candidate_slot = replacement.clone();
            if !feature_blockers_preserve_connectivity(&candidate, surfaces) {
                continue;
            }
            for blocker in &current_feature.blocker_footprint {
                all_blockers.remove(blocker);
            }
            for structural in &current_structural_cells {
                all_structural_cells.remove(structural);
            }
            all_blockers.extend(replacement.blocker_footprint.iter().copied());
            all_structural_cells.extend(projected.structural_cells.iter().copied());
            let Some(structural_slot) = structural_by_feature.get_mut(index) else {
                return Err(vec![recipe_issue(
                    "Forest old-growth structural slot left the feature collection",
                )]);
            };
            *structural_slot = projected.structural_cells;
            let Some(feature_slot) = features.get_mut(index) else {
                return Err(vec![recipe_issue(
                    "Forest old-growth feature slot left the feature collection",
                )]);
            };
            *feature_slot = replacement;
            upgraded = upgraded.saturating_add(1);
            break;
        }
    }
    if upgraded == 0
        && relocate_one_old_growth(
            features,
            woodland,
            exclusions,
            surfaces,
            objects,
            Some(object_stream),
            rotation_stream,
        )?
    {
        upgraded = 1;
    }
    if upgraded == 0 {
        return Err(vec![recipe_issue(OLD_GROWTH_CAPACITY_DETAIL)]);
    }
    Ok(())
}

fn relocate_one_old_growth(
    features: &mut [PlannedFeature],
    woodland: &BTreeSet<HexCoord>,
    exclusions: &BTreeSet<HexCoord>,
    surfaces: &BTreeMap<HexCoord, TilePos>,
    objects: &TemperateVegetationSet,
    object_stream: Option<SeedStream<'_>>,
    rotation_stream: Option<SeedStream<'_>>,
) -> Result<bool, Vec<WorldValidationIssue>> {
    let mut removal_order = features
        .iter()
        .enumerate()
        .map(|(index, feature)| {
            (
                feature_priority(object_stream, feature.root.coord, 827),
                feature.root,
                index,
            )
        })
        .collect::<Vec<_>>();
    removal_order.sort_unstable();
    for (_, _, index) in removal_order {
        let remaining = features
            .iter()
            .enumerate()
            .filter_map(|(candidate, feature)| (candidate != index).then_some(feature.clone()))
            .collect::<Vec<_>>();
        let (occupied_blockers, occupied_structural, occupied_roots) =
            tree_occupancy(&remaining, objects)?;
        let mut roots = woodland
            .iter()
            .copied()
            .filter(|coord| !exclusions.contains(coord))
            .filter_map(|coord| surfaces.get(&coord).copied())
            .filter(|root| {
                !occupied_roots.contains(&root.coord)
                    && !root
                        .coord
                        .neighbors()
                        .into_iter()
                        .any(|neighbor| occupied_roots.contains(&neighbor))
            })
            .collect::<Vec<_>>();
        roots
            .sort_unstable_by_key(|root| (feature_priority(object_stream, root.coord, 829), *root));
        for root in roots {
            let first_rotation = feature_rotation(rotation_stream, root.coord, 31)?;
            for offset in 0..6 {
                let rotation = offset_rotation(first_rotation, offset)?;
                let Some(projected) = project_tree(
                    &objects.old_growth,
                    root,
                    rotation,
                    woodland,
                    exclusions,
                    surfaces,
                    &occupied_blockers,
                    &occupied_structural,
                ) else {
                    continue;
                };
                let replacement = PlannedFeature {
                    root,
                    kind: FeatureKind::Tree,
                    object_id: objects.old_growth.id.clone(),
                    rotation,
                    blocker_footprint: projected.blockers,
                };
                let mut candidate = remaining.clone();
                candidate.push(replacement.clone());
                if !feature_blockers_preserve_connectivity(&candidate, surfaces) {
                    continue;
                }
                let Some(slot) = features.get_mut(index) else {
                    return Err(vec![recipe_issue(
                        "Forest Old-Growth relocation left the feature collection",
                    )]);
                };
                *slot = replacement;
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn feature_blockers_preserve_connectivity(
    features: &[PlannedFeature],
    surfaces: &BTreeMap<HexCoord, TilePos>,
) -> bool {
    let blockers = features
        .iter()
        .flat_map(|feature| feature.blocker_footprint.iter().copied())
        .collect::<BTreeSet<_>>();
    let Some(start) = surfaces
        .values()
        .copied()
        .find(|position| !blockers.contains(position))
    else {
        return false;
    };
    let expected = surfaces
        .values()
        .filter(|position| !blockers.contains(position))
        .count();
    let mut visited = BTreeSet::from([start]);
    let mut frontier = VecDeque::from([start]);
    while let Some(position) = frontier.pop_front() {
        for neighbor_coord in position.coord.neighbors() {
            let Some(neighbor) = surfaces.get(&neighbor_coord).copied() else {
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

fn plan_grass_features(
    roots: BTreeSet<TilePos>,
    objects: &TemperateVegetationSet,
    rotation_stream: Option<SeedStream<'_>>,
) -> Result<Vec<PlannedFeature>, Vec<WorldValidationIssue>> {
    roots
        .into_iter()
        .map(|root| {
            Ok(PlannedFeature {
                root,
                kind: FeatureKind::TallGrass,
                object_id: objects.grass_tuft.id.clone(),
                rotation: feature_rotation(rotation_stream, root.coord, 41)?,
                blocker_footprint: BTreeSet::new(),
            })
        })
        .collect()
}

fn tree_family(hash: u64) -> TreeFamily {
    if hash.is_multiple_of(4) {
        TreeFamily::TallNarrow
    } else {
        TreeFamily::SmallBroadleaf
    }
}

fn feature_rotation(
    stream: Option<SeedStream<'_>>,
    coord: HexCoord,
    salt: u64,
) -> Result<HexObjectRotation, Vec<WorldValidationIssue>> {
    let steps = u8::try_from(feature_priority(stream, coord, salt) % 6).unwrap_or_default();
    HexObjectRotation::new(steps).map_err(|error| {
        vec![recipe_issue(format!(
            "invalid Forest object rotation: {error}"
        ))]
    })
}

fn offset_rotation(
    first: HexObjectRotation,
    offset: u8,
) -> Result<HexObjectRotation, Vec<WorldValidationIssue>> {
    HexObjectRotation::new(first.steps().saturating_add(offset) % 6).map_err(|error| {
        vec![recipe_issue(format!(
            "invalid Forest object rotation: {error}"
        ))]
    })
}

#[derive(Debug)]
struct ProjectedTree {
    blockers: BTreeSet<TilePos>,
    structural_cells: BTreeSet<TilePos>,
}

#[expect(
    clippy::too_many_arguments,
    reason = "the placement boundary validates each independent spatial constraint explicitly"
)]
fn project_tree(
    object: &VegetationObjectSpec,
    root: TilePos,
    rotation: HexObjectRotation,
    woodland: &BTreeSet<HexCoord>,
    exclusions: &BTreeSet<HexCoord>,
    surfaces: &BTreeMap<HexCoord, TilePos>,
    occupied_blockers: &BTreeSet<TilePos>,
    occupied_structural_cells: &BTreeSet<TilePos>,
) -> Option<ProjectedTree> {
    let blockers = object.project_blockers(root, rotation, surfaces)?;
    if blockers.iter().any(|support| {
        !woodland.contains(&support.coord)
            || exclusions.contains(&support.coord)
            || occupied_blockers.contains(support)
    }) {
        return None;
    }

    let volume = object.project_visual_volume(root, rotation)?;
    for visual in &volume.cells {
        let support = surfaces.get(&visual.coord).copied()?;
        if visual.level <= support.level
            || (exclusions.contains(&visual.coord)
                && visual.level <= support.level.saturating_add(2))
        {
            return None;
        }
    }
    if !volume
        .structural_cells
        .is_disjoint(occupied_structural_cells)
    {
        return None;
    }
    Some(ProjectedTree {
        blockers,
        structural_cells: volume.structural_cells,
    })
}

fn collect_tree_visual_cells(
    features: &[PlannedFeature],
    objects: &TemperateVegetationSet,
) -> Result<BTreeSet<TilePos>, Vec<WorldValidationIssue>> {
    let mut visual_cells = BTreeSet::new();
    let mut structural_cells = BTreeSet::new();
    for feature in features {
        let Some(object) = objects.forest_object(feature.object_id.as_str()) else {
            return Err(vec![recipe_issue(format!(
                "Forest feature at {:?} uses unsupported authored object '{}'",
                feature.root, feature.object_id
            ))]);
        };
        let Some(volume) = object.project_visual_volume(feature.root, feature.rotation) else {
            return Err(vec![recipe_issue(format!(
                "Forest object '{}' cannot project its complete authored bounds at {:?}",
                feature.object_id, feature.root
            ))]);
        };
        if !volume.structural_cells.is_disjoint(&structural_cells) {
            return Err(vec![recipe_issue(format!(
                "Forest object '{}' overlaps neighboring structural vegetation at {:?}",
                feature.object_id, feature.root
            ))]);
        }
        visual_cells.extend(volume.cells);
        structural_cells.extend(volume.structural_cells);
    }
    Ok(visual_cells)
}

fn select_grass_roots(
    prairie: &BTreeSet<HexCoord>,
    exclusions: &BTreeSet<HexCoord>,
    surfaces: &BTreeMap<HexCoord, TilePos>,
    stream: Option<SeedStream<'_>>,
) -> BTreeSet<TilePos> {
    let mut eligible: Vec<_> = prairie.difference(exclusions).copied().collect();
    eligible.sort_unstable_by_key(|coord| (feature_priority(stream, *coord, 0), *coord));
    let target = prairie.len().saturating_mul(GRASS_DENSITY_PERCENT) / 100;
    eligible
        .into_iter()
        .take(target)
        .filter_map(|coord| surfaces.get(&coord).copied())
        .collect()
}

fn feature_priority(stream: Option<SeedStream<'_>>, coord: HexCoord, salt: u64) -> u64 {
    stream.map_or_else(
        || {
            let mut bytes = Vec::with_capacity(40);
            bytes.extend_from_slice(b"bevy-hex-game/v3/forest/fallback-feature");
            bytes.extend_from_slice(&coord.x().to_le_bytes());
            bytes.extend_from_slice(&coord.y().to_le_bytes());
            bytes.extend_from_slice(&coord.z().to_le_bytes());
            bytes.extend_from_slice(&salt.to_le_bytes());
            xxh3_64(&bytes)
        },
        |stream| stream.sample_coord(coord, salt),
    )
}

#[derive(Debug)]
struct ReliefMound {
    center: HexCoord,
    amplitude: i32,
}

#[derive(Debug)]
struct ReliefPlan {
    mounds: Vec<ReliefMound>,
}

impl ReliefPlan {
    fn new(
        grid_radius: u32,
        mask: &BTreeSet<HexCoord>,
        rotation: u8,
        stream: Option<SeedStream<'_>>,
    ) -> Result<Self, Vec<WorldValidationIssue>> {
        let radius = i32::try_from(grid_radius)
            .map_err(|error| vec![recipe_issue(format!("Forest radius exceeds i32: {error}"))])?;
        let mut candidates: Vec<_> = mask
            .iter()
            .copied()
            .filter(|coord| {
                HexCoord::ORIGIN
                    .distance(*coord)
                    .saturating_add(u32::try_from(MAX_RELIEF).unwrap_or_default())
                    <= grid_radius
            })
            .collect();
        if candidates.is_empty() {
            return Err(vec![recipe_issue(
                "Forest footprint has no room for rolling relief",
            )]);
        }
        candidates.sort_unstable();

        let mut centers = BTreeSet::new();
        let mut mounds = Vec::new();
        if let Some(stream) = stream {
            let count = u64::try_from(candidates.len()).unwrap_or(u64::MAX);
            for index in 0..MOUND_COUNT {
                let mut cursor = usize::try_from(stream.sample(index) % count).unwrap_or_default();
                for _ in 0..candidates.len() {
                    let Some(center) = candidates.get(cursor).copied() else {
                        break;
                    };
                    if centers.insert(center) {
                        let amplitude = 3_i32.saturating_add(
                            i32::try_from(stream.sample(index + 50) % 2).unwrap_or_default(),
                        );
                        mounds.push(ReliefMound { center, amplitude });
                        break;
                    }
                    cursor = cursor.saturating_add(1) % candidates.len();
                }
            }
        } else {
            let bases = [
                HexCoord::from_axial(-(radius / 2), -(radius / 4)),
                HexCoord::from_axial(-(radius / 3), radius / 3),
                HexCoord::from_axial(0, -(radius / 2)),
                HexCoord::from_axial(radius / 3, radius / 5),
                HexCoord::from_axial(radius / 2, -(radius / 4)),
            ];
            for (index, base) in bases.into_iter().enumerate() {
                let center = rotate(base, rotation);
                if mask.contains(&center) && centers.insert(center) {
                    let amplitude = if index == 0 { MAX_RELIEF } else { 3 };
                    mounds.push(ReliefMound { center, amplitude });
                }
            }
        }
        if mounds.len() < 3 {
            return Err(vec![recipe_issue(
                "Forest footprint cannot fit at least three distinct relief mounds",
            )]);
        }
        if !mounds.iter().any(|mound| mound.amplitude == MAX_RELIEF) {
            if let Some(first) = mounds.first_mut() {
                first.amplitude = MAX_RELIEF;
            }
        }
        Ok(Self { mounds })
    }

    fn height_at(&self, coord: HexCoord) -> i32 {
        self.mounds
            .iter()
            .map(|mound| {
                mound
                    .amplitude
                    .saturating_sub(i32::try_from(mound.center.distance(coord)).unwrap_or(i32::MAX))
                    .max(0)
            })
            .max()
            .unwrap_or_default()
    }
}

fn land_column(surface: i32, route: bool) -> VolumeColumn {
    VolumeColumn {
        elements: vec![
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(0, 1),
                material: SolidMaterialRole::Bedrock,
                cutaway_for: None,
            }),
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(1, surface - 3),
                material: SolidMaterialRole::Stone,
                cutaway_for: None,
            }),
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(surface - 3, surface),
                material: SolidMaterialRole::Dirt,
                cutaway_for: None,
            }),
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(surface, surface + 1),
                material: if route {
                    SolidMaterialRole::Gravel
                } else {
                    SolidMaterialRole::Grass
                },
                cutaway_for: None,
            }),
        ],
    }
}

fn exact_positions(
    coords: &[HexCoord],
    surfaces: &BTreeMap<HexCoord, TilePos>,
) -> Result<Vec<TilePos>, Vec<WorldValidationIssue>> {
    coords
        .iter()
        .map(|coord| {
            surfaces.get(coord).copied().ok_or_else(|| {
                vec![recipe_issue(format!(
                    "Forest coordinate {coord:?} has no exact surface"
                ))]
            })
        })
        .collect()
}

fn exact_position_set(
    coords: &BTreeSet<HexCoord>,
    surfaces: &BTreeMap<HexCoord, TilePos>,
) -> Result<BTreeSet<TilePos>, Vec<WorldValidationIssue>> {
    coords
        .iter()
        .map(|coord| {
            surfaces.get(coord).copied().ok_or_else(|| {
                vec![recipe_issue(format!(
                    "Forest coordinate {coord:?} has no exact surface"
                ))]
            })
        })
        .collect()
}

fn surface_set_connected(surfaces: &BTreeSet<TilePos>, graph: &OrdinaryGraph) -> bool {
    let Some(start) = surfaces.first().copied() else {
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

fn is_woodland(coord: HexCoord, rotation: u8) -> bool {
    unrotate(coord, rotation).x() <= 0
}

fn rotate(coord: HexCoord, turns: u8) -> HexCoord {
    let mut rotated = coord;
    for _ in 0..(turns % 6) {
        let [x, y, z] = rotated.to_cubic_array();
        rotated = HexCoord::new_cubic(-z, -x, -y);
    }
    rotated
}

fn unrotate(coord: HexCoord, turns: u8) -> HexCoord {
    rotate(coord, (6_u8.saturating_sub(turns % 6)) % 6)
}

fn forest_view_hint(
    grid_radius: u32,
    level_height: f32,
    rotation: u8,
) -> Result<MapViewHint, Vec<WorldValidationIssue>> {
    let radius = i32::try_from(grid_radius)
        .map_err(|error| vec![recipe_issue(format!("Forest radius exceeds i32: {error}"))])?;
    let focus_height = f32::from(i16::try_from(BASE_LEVEL + 1).unwrap_or_default()) * level_height;
    let frame = (f32::from(u16::try_from(grid_radius).unwrap_or(u16::MAX)) * 3.5).max(24.0);
    let direction_coord = rotate(HexCoord::from_axial(radius, -(radius / 2)), rotation);
    let direction = direction_coord.to_world(0.0);
    let horizontal = direction
        .x
        .mul_add(direction.x, direction.z * direction.z)
        .sqrt();
    if horizontal <= f32::EPSILON {
        return Err(vec![recipe_issue(
            "Forest camera direction is horizontally degenerate",
        )]);
    }
    Ok(MapViewHint::new(
        (
            direction.x / horizontal * frame,
            focus_height + frame,
            direction.z / horizontal * frame,
        ),
        (0.0, focus_height, 0.0),
    ))
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
    WorldValidationIssue::new(WorldIssueCode::Recipe("forest"), detail)
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use super::*;
    use crate::settings::{
        PatchEdgeContractSettings, PatchEdgesSettings, PatchMaskSettings, PatchSpec,
    };
    use crate::terrain::TerrainPalette;
    use hex_assets::{ArtPalette, ObjectBlueprint, ObjectCatalogFile, VoxelStyleCatalog};
    use hex_core::SubstanceId;

    // `SubstanceTable` assigns stable ids from sorted authored names. Keep this
    // materialization fixture aligned with that public runtime contract so its
    // map fingerprint is directly comparable with gameplay publication.
    const BASALT: SubstanceId = SubstanceId(1);
    const BEDROCK: SubstanceId = SubstanceId(2);
    const DIRT: SubstanceId = SubstanceId(3);
    const GRASS: SubstanceId = SubstanceId(4);
    const GRAVEL: SubstanceId = SubstanceId(5);
    const ICE: SubstanceId = SubstanceId(6);
    const LAVA: SubstanceId = SubstanceId(7);
    const METAL: SubstanceId = SubstanceId(8);
    const SNOW: SubstanceId = SubstanceId(9);
    const STONE: SubstanceId = SubstanceId(10);
    const WATER: SubstanceId = SubstanceId(11);

    fn runtime_art_catalog() -> &'static RuntimeArtCatalog {
        static CATALOG: OnceLock<RuntimeArtCatalog> = OnceLock::new();
        CATALOG.get_or_init(|| {
            let palette: ArtPalette = ron::from_str(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/art/palette.ron"
            )))
            .expect("tracked art palette should parse");
            let styles: VoxelStyleCatalog = ron::from_str(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/art/voxel_styles.ron"
            )))
            .expect("tracked voxel styles should parse");
            let manifest: ObjectCatalogFile = ron::from_str(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/art/object_catalog.ron"
            )))
            .expect("tracked object catalog should parse");
            let mut objects = BTreeMap::new();
            for source in [
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../assets/art/objects/plant/small-broadleaf.ron"
                )),
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../assets/art/objects/plant/tall-narrow.ron"
                )),
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../assets/art/objects/plant/old-growth.ron"
                )),
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../assets/art/objects/plant/snowy-old-growth.ron"
                )),
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../assets/art/objects/plant/snowy-small-broadleaf.ron"
                )),
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../assets/art/objects/plant/snowy-tall-narrow.ron"
                )),
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../assets/art/objects/prop/cave-lichen.ron"
                )),
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../assets/art/objects/prop/cave-moss.ron"
                )),
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../assets/art/objects/prop/grass-tuft.ron"
                )),
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../assets/art/objects/prop/snowy-grass-tuft.ron"
                )),
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../assets/art/objects/prop/crystal-low-cluster.ron"
                )),
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../assets/art/objects/prop/crystal-branched.ron"
                )),
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../assets/art/objects/prop/crystal-spire.ron"
                )),
            ] {
                let blueprint: ObjectBlueprint =
                    ron::from_str(source).expect("tracked object blueprint should parse");
                objects.insert(blueprint.id.clone(), blueprint);
            }
            RuntimeArtCatalog::from_sources(&palette, &styles, &manifest, objects)
                .expect("tracked runtime art graph should resolve")
        })
    }

    fn generate(
        grid_radius: u32,
        level_height: f32,
        settings: &ProceduralV3Settings,
        seed: u64,
    ) -> Result<ValidatedWorldSelection<ForestMetrics>, V3GenerationError> {
        let objects = TemperateVegetationSet::resolve(runtime_art_catalog(), "Forest")
            .map_err(V3GenerationError::RecipeContract)?;
        generate_with_objects(grid_radius, level_height, settings, seed, objects)
    }

    fn settings() -> ProceduralV3Settings {
        ProceduralV3Settings {
            layout: V3LayoutSettings::Single(PatchSpec {
                environment: V3EnvironmentSettings::TemperateGrassland,
                recipe: V3RecipeSettings::Forest(V3ForestSettings),
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

    fn palette() -> TerrainPalette {
        TerrainPalette {
            bedrock: BEDROCK,
            stone: STONE,
            dirt: DIRT,
            grass: GRASS,
            gravel: GRAVEL,
            water: WATER,
            metal: METAL,
            worked_stone: SubstanceId(12),
            snow: SNOW,
            ice: ICE,
            basalt: BASALT,
            lava: LAVA,
        }
    }

    fn is_solid(substance: SubstanceId) -> bool {
        !matches!(substance, SubstanceId::AIR | WATER | LAVA)
    }

    fn exact_surface(plan: &GeneratedWorldPlan, coord: HexCoord) -> TilePos {
        plan.volume
            .surfaces
            .keys()
            .find(|surface| surface.coord == coord)
            .copied()
            .expect("the Forest fixture should contain the requested surface")
    }

    fn set_surface_material(
        plan: &mut GeneratedWorldPlan,
        surface: TilePos,
        material: SolidMaterialRole,
    ) {
        let column = plan
            .volume
            .columns
            .get_mut(&surface.coord)
            .expect("the Forest fixture should contain the requested column");
        let mass = column
            .elements
            .iter_mut()
            .find_map(|element| match element {
                VolumeElement::Solid(mass)
                    if mass.levels.bottom <= surface.level && surface.level < mass.levels.top =>
                {
                    Some(mass)
                }
                VolumeElement::Solid(_) | VolumeElement::Fill(_) => None,
            })
            .expect("the Forest fixture should contain the requested surface mass");
        mass.material = material;
    }

    fn assert_distinct_clearings(plan: &GeneratedWorldPlan) {
        let expected_names: BTreeSet<_> = (0..CLEARING_COUNT)
            .map(|index| format!("forest_clearing_{index}"))
            .collect();
        assert_eq!(
            plan.features
                .clearings
                .keys()
                .cloned()
                .collect::<BTreeSet<_>>(),
            expected_names
        );
        let mut claimed = BTreeSet::new();
        for (name, clearing) in &plan.features.clearings {
            assert!(
                claimed.is_disjoint(&clearing.surfaces),
                "Forest clearing {name:?} overlaps an earlier named clearing"
            );
            claimed.extend(clearing.surfaces.iter().copied());
        }
    }

    fn walker_volume(surfaces: impl IntoIterator<Item = TilePos>) -> BTreeSet<TilePos> {
        surfaces
            .into_iter()
            .flat_map(|surface| {
                [1, 2]
                    .map(|offset| TilePos::new(surface.coord, surface.level.saturating_add(offset)))
            })
            .collect()
    }

    fn assert_complete_authored_vegetation_bounds(plan: &GeneratedWorldPlan) {
        let objects = TemperateVegetationSet::resolve(runtime_art_catalog(), "Forest")
            .expect("the tracked Forest art graph should resolve");
        let route_and_review_surfaces = plan
            .features
            .protected_routes
            .values()
            .flat_map(|route| route.surfaces.iter().copied())
            .chain(
                plan.features
                    .clearings
                    .values()
                    .flat_map(|clearing| clearing.surfaces.iter().copied()),
            )
            .chain(plan.anchors.values().copied())
            .collect::<BTreeSet<_>>();
        let protected_walker_volume = walker_volume(route_and_review_surfaces);
        let surface_by_coord = plan
            .volume
            .surfaces
            .keys()
            .map(|surface| (surface.coord, *surface))
            .collect::<BTreeMap<_, _>>();
        let mut claimed_structural_cells = BTreeSet::new();

        for feature in plan.features.by_id.values() {
            let object = objects
                .forest_object(feature.object_id.as_str())
                .unwrap_or_else(|| panic!("unexpected Forest object '{}'", feature.object_id));
            let projected = object
                .project_visual_volume(feature.root, feature.rotation)
                .unwrap_or_else(|| {
                    panic!(
                        "Forest object '{}' should project its complete authored bounds",
                        feature.object_id
                    )
                });
            for visual in &projected.cells {
                let support = surface_by_coord.get(&visual.coord).unwrap_or_else(|| {
                    panic!(
                        "Forest object '{}' leaves the generated terrain at {visual:?}",
                        feature.object_id
                    )
                });
                assert!(
                    visual.level > support.level,
                    "Forest object '{}' intersects terrain at {visual:?}",
                    feature.object_id
                );
            }
            assert!(
                projected
                    .structural_cells
                    .is_disjoint(&claimed_structural_cells),
                "Forest object '{}' overlaps neighboring structural vegetation",
                feature.object_id
            );
            assert!(
                projected.cells.is_disjoint(&protected_walker_volume),
                "Forest object '{}' intersects a protected route, clearing, or review-anchor \
                 walker volume",
                feature.object_id
            );
            claimed_structural_cells.extend(projected.structural_cells);
        }
    }

    #[test]
    fn fixed_corpus_builds_valid_forests_at_supported_radii() {
        for radius in [12, 20, 40] {
            for seed in [0, 1, 808, 4_294_967_311] {
                let selected =
                    generate(radius, 0.4, &settings(), seed).expect("Forest should generate");
                assert!(!selected.used_fallback);
                assert_eq!(selected.metrics.clearing_count, 4);
                assert!(selected.metrics.tree_roots > 0);
                assert!(selected.metrics.tall_grass_roots > 0);
                assert_eq!(selected.validated.plan.validate(), Vec::new());
                assert_distinct_clearings(&selected.validated.plan);
            }
        }
    }

    #[test]
    fn complete_authored_vegetation_bounds_clear_protected_walker_volumes() {
        for seed in [91, 2_026, 381_654_729] {
            let selected = generate(12, 0.4, &settings(), seed).expect("Forest should generate");
            assert_complete_authored_vegetation_bounds(&selected.validated.plan);
        }
    }

    #[test]
    fn radius_12_pr_corpus_validates_128_forest_seeds_and_named_regressions() {
        let mut seeds: BTreeSet<u64> = (0..128).collect();
        seeds.extend([808, 4_294_967_311]);
        let mut fallbacks = 0_usize;

        for &seed in &seeds {
            let selected = generate(12, 0.4, &settings(), seed)
                .unwrap_or_else(|error| panic!("radius-12 Forest seed {seed}: {error}"));
            fallbacks += usize::from(selected.used_fallback);
        }

        assert!(
            fallbacks.saturating_mul(100) < seeds.len(),
            "{fallbacks}/{} radius-12 Forest seeds used fallback",
            seeds.len()
        );
    }

    #[test]
    fn named_streams_make_output_repeatable_and_seed_sensitive() {
        let first = generate(12, 0.4, &settings(), 17).expect("Forest should generate");
        let repeated = generate(12, 0.4, &settings(), 17).expect("Forest should repeat");
        let other = generate(12, 0.4, &settings(), 18).expect("other Forest should generate");

        assert_eq!(
            first.validated.semantic_fingerprint,
            repeated.validated.semantic_fingerprint
        );
        assert_ne!(
            first.validated.semantic_fingerprint,
            other.validated.semantic_fingerprint
        );
    }

    #[test]
    fn woodland_road_bends_narrows_tapers_and_remains_feature_free() {
        let selected = generate(12, 0.4, &settings(), 91).expect("Forest should generate");
        let plan = &selected.validated.plan;
        let (rotation, _) =
            detect_orientation(plan).expect("Forest should expose its exact orientation");
        let road = plan
            .features
            .protected_routes
            .get(ROAD_ROUTE)
            .expect("Forest should expose its exact road");
        let graph = OrdinaryGraph::from_volume(&plan.volume, Some(&plan.blockers));
        let roots: BTreeSet<_> = plan
            .features
            .by_id
            .values()
            .map(|feature| feature.root)
            .collect();

        assert!(road
            .surfaces
            .iter()
            .all(|position| !roots.contains(position)));
        assert!(road
            .centerline
            .windows(2)
            .all(|pair| matches!(pair, [first, second] if graph.admits(*first, *second))));
        assert!(road.centerline.windows(3).any(|window| {
            matches!(window, [first, middle, last] if (
                middle.coord.x() - first.coord.x(),
                middle.coord.y() - first.coord.y()
            ) != (
                last.coord.x() - middle.coord.x(),
                last.coord.y() - middle.coord.y()
            ))
        }));
        assert!(road
            .surfaces
            .iter()
            .all(|surface| unrotate(surface.coord, rotation).x() <= PRAIRIE_TAPER_DEPTH));

        let narrow = road
            .centerline
            .iter()
            .filter(|position| {
                let local = unrotate(position.coord, rotation);
                let shoulder = rotate(
                    HexCoord::from_axial(local.x(), local.y().saturating_add(1)),
                    rotation,
                );
                !road
                    .surfaces
                    .iter()
                    .any(|surface| surface.coord == shoulder)
            })
            .count();
        assert!(narrow >= 2);
        assert!(narrow.saturating_mul(2) <= road.centerline.len());
        let woodland_narrow = road
            .centerline
            .iter()
            .filter(|position| {
                let local = unrotate(position.coord, rotation);
                if local.x() > 0 {
                    return false;
                }
                let shoulder = rotate(
                    HexCoord::from_axial(local.x(), local.y().saturating_add(1)),
                    rotation,
                );
                !road
                    .surfaces
                    .iter()
                    .any(|surface| surface.coord == shoulder)
            })
            .count();
        assert!(
            woodland_narrow > 0,
            "the woodland itself needs a one-wide pinch independently of the taper"
        );

        let trees: BTreeSet<_> = plan
            .features
            .by_id
            .values()
            .filter(|feature| feature.kind == FeatureKind::Tree)
            .map(|feature| feature.root.coord)
            .collect();
        let woodland_road: Vec<_> = road
            .centerline
            .iter()
            .filter(|surface| is_woodland(surface.coord, rotation))
            .collect();
        let flanked = woodland_road
            .iter()
            .filter(|surface| {
                surface
                    .coord
                    .neighbors()
                    .into_iter()
                    .any(|neighbor| trees.contains(&neighbor))
            })
            .count();
        assert!(flanked.saturating_mul(5) >= woodland_road.len());
        assert!(road
            .surfaces
            .iter()
            .all(|surface| surface_material(plan, *surface) == Some(SolidMaterialRole::Gravel)));
    }

    #[test]
    fn hero_seed_increases_trees_and_covers_most_of_the_prairie_with_grass() {
        let selected =
            generate(12, 0.4, &settings(), 381_654_729).expect("hero Forest should generate");

        assert_eq!(selected.metrics.tree_roots, 53);
        assert!(selected.metrics.old_growth_roots > 0);
        assert_eq!(
            selected.metrics.old_growth_blocker_surfaces,
            selected.metrics.old_growth_roots.saturating_mul(7)
        );
        assert_eq!(
            selected.metrics.tree_blocker_surfaces,
            selected
                .metrics
                .tree_roots
                .saturating_add(selected.metrics.old_growth_roots.saturating_mul(6))
        );
        assert!(
            (51..=55).contains(&selected.metrics.tree_roots),
            "the prior hero had 46 tree roots; review asked for 10-20% more"
        );
        assert_eq!(selected.metrics.tall_grass_roots, 155);
        assert!(
            selected.metrics.tall_grass_roots.saturating_mul(2) > selected.metrics.prairie_surfaces
        );
        let object_ids = selected
            .validated
            .plan
            .features
            .by_id
            .values()
            .map(|feature| feature.object_id.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            object_ids,
            BTreeSet::from([
                SMALL_BROADLEAF_ID,
                TALL_NARROW_ID,
                OLD_GROWTH_ID,
                GRASS_TUFT_ID,
            ])
        );
        let build = super::super::build(
            12,
            0.4,
            &settings(),
            381_654_729,
            &palette(),
            &is_solid,
            Some(runtime_art_catalog()),
        )
        .expect("hero Forest should materialize");
        assert_eq!(
            selected.validated.semantic_fingerprint,
            3_116_162_104_822_374_845
        );
        assert_eq!(build.report.map_fingerprint, 18_084_914_740_711_593_486);
        assert_eq!(build.map.len(), 469);
        assert_eq!(
            build.blockers.len(),
            usize::try_from(selected.metrics.tree_blocker_surfaces).unwrap_or(usize::MAX)
        );
        assert!(build.special_regions.is_empty());
        assert!(build.interiors.is_empty());
    }

    #[test]
    fn validator_rejects_a_forest_without_exact_old_growth() {
        let selected = generate(12, 0.4, &settings(), 91).expect("Forest should generate");
        let mut plan = selected.validated.plan;
        let replacement = hex_assets::ObjectAssetId::new(SMALL_BROADLEAF_ID)
            .expect("the stable small broadleaf id should remain valid");
        let mut replaced = 0_usize;
        for feature in plan.features.by_id.values_mut() {
            if feature.object_id.as_str() == OLD_GROWTH_ID {
                feature.object_id = replacement.clone();
                replaced = replaced.saturating_add(1);
            }
        }
        assert!(replaced > 0, "the valid fixture must start with Old-Growth");

        let WorldValidation::Invalid(issues) = validate_forest(&plan) else {
            panic!("removing every exact Old-Growth instance must fail");
        };
        assert!(issues.iter().any(|issue| {
            issue
                .detail
                .contains("must retain at least one exact authored Old-Growth")
        }));
    }

    #[test]
    fn tree_family_preference_retains_one_quarter_tall_narrow() {
        let tall = (0..400_u64)
            .filter(|hash| tree_family(*hash) == TreeFamily::TallNarrow)
            .count();
        assert_eq!(tall, 100);
    }

    #[test]
    fn validator_rejects_a_route_that_loses_its_winding_centerline() {
        let selected = generate(12, 0.4, &settings(), 91).expect("Forest should generate");
        let mut plan = selected.validated.plan;
        let road = plan
            .features
            .protected_routes
            .get_mut(ROAD_ROUTE)
            .expect("Forest should expose its road");
        let first = *road.centerline.first().expect("road has a start");
        let last = *road.centerline.last().expect("road has an end");
        road.centerline = vec![first, last];

        let WorldValidation::Invalid(issues) = validate_forest(&plan) else {
            panic!("a disconnected straight-line replacement must fail");
        };
        assert!(issues.iter().any(|issue| {
            issue.detail.contains("not continuously walkable")
                || issue.detail.contains("requires at least four bends")
        }));
    }

    #[test]
    fn validator_binds_the_road_to_the_detected_route_offset() {
        let selected = generate(12, 0.4, &settings(), 91).expect("Forest should generate");
        let mut plan = selected.validated.plan;
        let (rotation, route_offset) =
            detect_orientation(&plan).expect("Forest should expose its exact orientation");
        let radius = i32::try_from(plan.layout.grid_radius).expect("test radius fits i32");
        let (alternate_party, alternate_hostile) = [-1, 0, 1]
            .into_iter()
            .filter(|offset| *offset != route_offset)
            .find_map(|offset| {
                let party = exact_surface(
                    &plan,
                    rotate(
                        HexCoord::from_axial((-radius).saturating_add(2), offset),
                        rotation,
                    ),
                );
                let hostile = exact_surface(
                    &plan,
                    rotate(
                        HexCoord::from_axial(radius.saturating_sub(2), offset),
                        rotation,
                    ),
                );
                (!plan.blockers.contains(&party) && !plan.blockers.contains(&hostile))
                    .then_some((party, hostile))
            })
            .expect("one alternate actor row should remain unblocked");
        plan.anchors.insert(PARTY_START.to_owned(), alternate_party);
        plan.anchors
            .insert(HOSTILE_START.to_owned(), alternate_hostile);

        assert_eq!(
            plan.validate(),
            Vec::new(),
            "the common plan remains valid so the recipe must enforce its exact road endpoints"
        );
        let WorldValidation::Invalid(issues) = validate_forest(&plan) else {
            panic!("a road on a different row from the actor landings must fail");
        };
        assert!(issues.iter().any(|issue| {
            issue
                .detail
                .contains("does not use the exact forest landing")
        }));
    }

    #[test]
    fn validator_requires_exact_review_anchors_in_their_semantic_regions() {
        let selected = generate(12, 0.4, &settings(), 91).expect("Forest should generate");
        let baseline = selected.validated.plan;

        for missing in [FOREST_CLEARING, PRAIRIE_OVERLOOK] {
            let mut plan = baseline.clone();
            let _removed = plan
                .anchors
                .remove(missing)
                .expect("the generated Forest should publish every review anchor");
            assert_eq!(
                plan.validate(),
                Vec::new(),
                "the common plan deliberately permits recipe-specific anchor sets"
            );
            let WorldValidation::Invalid(issues) = validate_forest(&plan) else {
                panic!("missing Forest review anchor {missing:?} must fail");
            };
            assert!(issues.iter().any(|issue| issue.detail.contains(missing)));
        }

        let mut misplaced_clearing = baseline.clone();
        let party = misplaced_clearing
            .anchors
            .get(PARTY_START)
            .copied()
            .expect("Forest should publish party_start");
        misplaced_clearing
            .anchors
            .insert(FOREST_CLEARING.to_owned(), party);
        let WorldValidation::Invalid(issues) = validate_forest(&misplaced_clearing) else {
            panic!("a clearing anchor outside forest_clearing_0 must fail");
        };
        assert!(issues
            .iter()
            .any(|issue| issue.detail.contains("forest_clearing_0")));

        let mut misplaced_overlook = baseline.clone();
        let hostile = misplaced_overlook
            .anchors
            .get(HOSTILE_START)
            .copied()
            .expect("Forest should publish hostile_start");
        misplaced_overlook
            .anchors
            .insert(PRAIRIE_OVERLOOK.to_owned(), hostile);
        let WorldValidation::Invalid(issues) = validate_forest(&misplaced_overlook) else {
            panic!("a prairie overlook away from its exact review surface must fail");
        };
        assert!(issues
            .iter()
            .any(|issue| issue.detail.contains("exact prairie surface")));

        let mut extended = baseline.clone();
        extended.anchors.insert(
            "future_review_anchor".to_owned(),
            extended
                .anchors
                .get(PARTY_START)
                .copied()
                .expect("Forest should publish party_start"),
        );
        assert!(
            matches!(validate_forest(&extended), WorldValidation::Valid(_)),
            "recipe validation must preserve the open generated-anchor vocabulary"
        );
    }

    #[test]
    fn validator_rejects_overlapping_named_clearings() {
        let selected = generate(12, 0.4, &settings(), 91).expect("Forest should generate");
        let mut plan = selected.validated.plan;
        let first = plan
            .features
            .clearings
            .get("forest_clearing_0")
            .cloned()
            .expect("Forest should publish its primary clearing");
        plan.features
            .clearings
            .insert("forest_clearing_1".to_owned(), first);

        assert_eq!(
            plan.validate(),
            Vec::new(),
            "the common feature vocabulary permits recipe-owned clearing topology"
        );
        let WorldValidation::Invalid(issues) = validate_forest(&plan) else {
            panic!("overlapping Forest clearings must fail");
        };
        assert!(issues
            .iter()
            .any(|issue| issue.detail.contains("overlap at")));
    }

    #[test]
    fn validator_rejects_stray_disconnected_gravel_in_the_road_footprint() {
        let selected = generate(12, 0.4, &settings(), 91).expect("Forest should generate");
        let mut plan = selected.validated.plan;
        let (rotation, _) =
            detect_orientation(&plan).expect("Forest should expose its exact orientation");
        let road_surfaces = plan
            .features
            .protected_routes
            .get(ROAD_ROUTE)
            .expect("Forest should expose its road")
            .surfaces
            .clone();
        let feature_roots: BTreeSet<_> = plan
            .features
            .by_id
            .values()
            .map(|feature| feature.root)
            .collect();
        let graph = OrdinaryGraph::from_volume(&plan.volume, Some(&plan.blockers));
        let stray = graph
            .positions()
            .find(|surface| {
                unrotate(surface.coord, rotation).x() <= PRAIRIE_TAPER_DEPTH
                    && !road_surfaces.contains(surface)
                    && !feature_roots.contains(surface)
                    && graph
                        .neighbors(*surface)
                        .iter()
                        .all(|neighbor| !road_surfaces.contains(neighbor))
            })
            .expect("the Forest fixture should contain a disconnected non-feature surface");
        set_surface_material(&mut plan, stray, SolidMaterialRole::Gravel);
        plan.features
            .protected_routes
            .get_mut(ROAD_ROUTE)
            .expect("Forest should expose its road")
            .surfaces
            .insert(stray);

        assert_eq!(
            plan.validate(),
            Vec::new(),
            "the common plan remains valid so the recipe must enforce its road shape"
        );
        let WorldValidation::Invalid(issues) = validate_forest(&plan) else {
            panic!("stray disconnected gravel must fail Forest validation");
        };
        assert!(issues.iter().any(|issue| {
            issue
                .detail
                .contains("outside its centerline and canonical shoulders")
        }));
        assert!(issues
            .iter()
            .any(|issue| issue.detail.contains("footprint is not walker-connected")));
    }

    #[test]
    fn tree_blockers_leave_every_other_surface_connected() {
        let selected = generate(12, 0.4, &settings(), 2026).expect("Forest should generate");
        let plan = &selected.validated.plan;
        let graph = OrdinaryGraph::from_volume(&plan.volume, Some(&plan.blockers));
        let party = plan
            .anchors
            .get(PARTY_START)
            .copied()
            .expect("Forest should publish party_start");

        assert_eq!(graph.distances_from(party).len(), graph.len());
        assert_eq!(
            plan.blockers,
            plan.features
                .by_id
                .values()
                .filter(|feature| feature.kind == FeatureKind::Tree)
                .flat_map(|feature| feature.blocker_footprint.iter().copied())
                .collect()
        );
    }

    #[test]
    fn blocked_summits_do_not_inflate_ordinary_elevation_metrics() {
        let selected = generate(12, 0.4, &settings(), 2026).expect("Forest should generate");
        let plan = &selected.validated.plan;
        let (rotation, _) =
            detect_orientation(plan).expect("Forest should expose its exact orientation");
        let baseline_graph = OrdinaryGraph::from_volume(&plan.volume, Some(&plan.blockers));
        let baseline = ordinary_elevation_metrics(&baseline_graph, rotation);
        let summit_level = baseline
            .reachable_levels
            .last()
            .copied()
            .expect("Forest should expose ordinary surfaces");
        let summit_surfaces: Vec<_> = baseline_graph
            .positions()
            .filter(|position| position.level == summit_level)
            .collect();
        assert!(!summit_surfaces.is_empty());

        let mut blockers = plan.blockers.clone();
        blockers.extend(summit_surfaces);
        let blocked_graph = OrdinaryGraph::from_volume(&plan.volume, Some(&blockers));
        let blocked = ordinary_elevation_metrics(&blocked_graph, rotation);

        assert!(
            blocked
                .reachable_levels
                .last()
                .is_some_and(|level| *level < summit_level),
            "blocked summit surfaces must not count as reachable elevation"
        );
        assert!(
            blocked.relief < baseline.relief,
            "blocked summit surfaces must not inflate ordinary relief"
        );
        let expected_woodland_high = blocked_graph
            .positions()
            .filter(|position| is_woodland(position.coord, rotation))
            .map(|position| position.level)
            .max()
            .unwrap_or(BASE_LEVEL);
        let expected_prairie_high = blocked_graph
            .positions()
            .filter(|position| !is_woodland(position.coord, rotation))
            .map(|position| position.level)
            .max()
            .unwrap_or(BASE_LEVEL);
        assert_eq!(
            blocked.woodland_prairie_high_ground_difference,
            expected_woodland_high.abs_diff(expected_prairie_high)
        );
    }

    #[test]
    fn forced_candidate_failure_uses_independent_fallback() {
        let selected = run_recipe(
            &ForestRecipe {
                level_height: 0.4,
                layout: resolve_layout(12, &settings()).expect("test layout should resolve"),
                objects: TemperateVegetationSet::resolve(runtime_art_catalog(), "Forest")
                    .expect("tracked Forest objects should resolve"),
                reject_candidates: true,
            },
            &settings(),
            12,
            999,
        )
        .expect("canonical Forest fallback should be valid");

        assert!(selected.used_fallback);
        assert_eq!(selected.selected_candidate, None);
        assert_eq!(selected.valid_candidates, 0);
        assert_eq!(selected.metrics.clearing_count, 4);
    }

    #[test]
    fn unsupported_recipe_and_invalid_level_height_fail_explicitly() {
        for invalid in [0.0, -0.4, f32::NAN, f32::INFINITY] {
            assert!(matches!(
                generate(12, invalid, &settings(), 12),
                Err(V3GenerationError::RecipeContract(_))
            ));
        }

        let mut wrong = settings();
        let V3LayoutSettings::Single(patch) = &mut wrong.layout else {
            unreachable!()
        };
        patch.recipe = V3RecipeSettings::Waterfall(crate::settings::V3WaterfallSettings);
        assert!(matches!(
            generate(12, 0.4, &wrong, 1),
            Err(V3GenerationError::RecipeUnavailable("Waterfall"))
        ));
    }

    #[test]
    #[ignore = "10,000 seeds are a manual V3 Forest stress corpus"]
    fn ten_thousand_seed_corpus_has_less_than_one_percent_fallbacks() {
        let mut fallbacks = 0_u32;
        for seed in 0..10_000 {
            let selected =
                generate(12, 0.4, &settings(), seed).expect("every final Forest should be valid");
            fallbacks = fallbacks.saturating_add(u32::from(selected.used_fallback));
        }
        assert!(fallbacks < 100, "fallbacks: {fallbacks}/10000");
    }

    #[test]
    #[ignore = "manual release/debug V3 Forest full-build benchmark"]
    fn forest_full_build_benchmark_tracks_median_and_p95() {
        let budget = if cfg!(debug_assertions) {
            std::time::Duration::from_millis(250)
        } else {
            std::time::Duration::from_millis(50)
        };
        let palette = palette();
        for radius in [12, 20, 40] {
            CAPACITY_PROJECTION_CACHE_PEAK.store(0, Ordering::Relaxed);
            let warmup = super::super::build(
                radius,
                0.4,
                &settings(),
                u64::MAX,
                &palette,
                &is_solid,
                Some(runtime_art_catalog()),
            )
            .expect("warm-up Forest should build");
            std::hint::black_box(warmup);

            let mut samples = Vec::new();
            for seed in 0..12 {
                let started = std::time::Instant::now();
                let build = super::super::build(
                    radius,
                    0.4,
                    &settings(),
                    seed,
                    &palette,
                    &is_solid,
                    Some(runtime_art_catalog()),
                )
                .expect("benchmark Forest should build");
                assert!(!build.report.used_fallback);
                samples.push(started.elapsed());
                std::hint::black_box(build);
            }
            samples.sort_unstable();
            let median = samples
                .get(samples.len() / 2)
                .copied()
                .expect("the benchmark records twelve samples");
            let p95 = samples
                .last()
                .copied()
                .expect("the benchmark records twelve samples");
            let cache_peak = CAPACITY_PROJECTION_CACHE_PEAK.load(Ordering::Relaxed);
            eprintln!(
                "V3 Forest full build radius {radius}: median={median:?} p95={p95:?} \
                 target={budget:?} capacity_cache_peak={cache_peak} \
                 max_cached_projections={} (trend only)",
                cache_peak.saturating_mul(12)
            );
        }
    }
}
