//! Pure semantic Forest recipe for procedural generator V3.
//!
//! Terrain and clearings are finalized before tree roots. The protected road is
//! then routed around those exact blockers, while tall grass remains
//! presentation-only.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use hex_core::{HexCoord, MapViewHint, TilePos};
use xxhash_rust::xxh3::xxh3_64;

use super::layout::{resolve_layout, PatchId, ResolvedLayoutPlan};
use super::seed::{SeedStream, SeedStreams};
use super::selection::{
    run_recipe, CandidateAttemptError, CandidateContext, FallbackContext, RepairOutcome, V3Recipe,
    ValidatedWorldSelection, WorldValidation,
};
use super::traversal::OrdinaryGraph;
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
const TREE_SPACING: u32 = 2;
const TREE_DENSITY_PERCENT: usize = 22;
const GRASS_DENSITY_PERCENT: usize = 70;
const PRAIRIE_TAPER_DEPTH: i32 = 3;
const ROAD_ROUTE: &str = "forest_road";
const PARTY_START: &str = "party_start";
const HOSTILE_START: &str = "hostile_start";
const FOREST_CLEARING: &str = "forest_clearing";
const PRAIRIE_OVERLOOK: &str = "prairie_overlook";

/// Recipe metrics retained by the V3 candidate selector and diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ForestMetrics {
    pub(crate) tree_roots: u32,
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
    grass: SeedStream<'a>,
}

#[derive(Debug)]
struct PlannedRoad {
    centerline: Vec<HexCoord>,
    surfaces: BTreeSet<HexCoord>,
}

/// Runs the common eight-candidate V3 selector for one Forest world.
pub(crate) fn generate(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV3Settings,
    seed: u64,
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
        let streams = SeedStreams::new(context.seed, context.candidate, PatchId(0).0);
        let streams = ForestStreams {
            orientation: streams.stage("forest.orientation"),
            landform: streams.stage("forest.landform"),
            clearings: streams.stage("forest.clearings"),
            routes: streams.stage("forest.routes"),
            trees: streams.stage("forest.trees"),
            grass: streams.stage("forest.grass"),
        };
        construct_plan(self.layout.clone(), Some(streams), self.level_height)
            .map_err(CandidateAttemptError::Rejected)
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
        construct_plan(self.layout.clone(), None, self.level_height).map_err(|issues| {
            V3GenerationError::RecipeContract(
                issues
                    .into_iter()
                    .map(|issue| issue.detail)
                    .collect::<Vec<_>>()
                    .join("; "),
            )
        })
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

fn construct_plan(
    layout: ResolvedLayoutPlan,
    streams: Option<ForestStreams<'_>>,
    level_height: f32,
) -> Result<GeneratedWorldPlan, Vec<WorldValidationIssue>> {
    let patch = layout
        .patches
        .get(&PatchId(0))
        .ok_or_else(|| vec![recipe_issue("Single Forest layout has no patch zero")])?;
    let mask = patch.mask.clone();
    let biome_region = patch.biome_region;
    let radius = i32::try_from(layout.grid_radius)
        .map_err(|error| vec![recipe_issue(format!("Forest radius exceeds i32: {error}"))])?;
    let rotation = streams.map_or(0, |streams| {
        u8::try_from(streams.orientation.sample(0) % 6).unwrap_or_default()
    });
    let route_offset = match streams {
        Some(streams) => streams
            .routes
            .range_i32(0, -1, 1)
            .map_err(|error| vec![recipe_issue(error)])?,
        None => 0,
    };
    let party_coord = rotate(
        HexCoord::from_axial((-radius).saturating_add(2), route_offset),
        rotation,
    );
    let hostile_coord = rotate(
        HexCoord::from_axial(radius.saturating_sub(2), route_offset),
        rotation,
    );
    if !mask.contains(&party_coord) || !mask.contains(&hostile_coord) {
        return Err(vec![recipe_issue(
            "Forest footprint cannot fit its selected actor landings",
        )]);
    }
    let relief = ReliefPlan::new(
        layout.grid_radius,
        &mask,
        rotation,
        streams.map(|streams| streams.landform),
    )?;

    let mut surfaces = BTreeMap::new();
    let mut surface_by_coord = BTreeMap::new();
    for coord in &mask {
        let surface_level = BASE_LEVEL.saturating_add(relief.height_at(*coord));
        let position = TilePos::new(*coord, surface_level);
        surfaces.insert(
            position,
            SurfaceMetadata {
                access: SurfaceAccess::Ordinary,
                interior: None,
            },
        );
        surface_by_coord.insert(*coord, position);
    }

    let woodland: BTreeSet<_> = mask
        .iter()
        .copied()
        .filter(|coord| is_woodland(*coord, rotation))
        .collect();
    let clearings = clearing_coordinates(
        radius,
        rotation,
        &mask,
        &woodland,
        streams.map(|streams| streams.clearings),
    )?;
    let mut clearing_plans = BTreeMap::new();
    let mut clearing_coords = BTreeSet::new();
    for (index, clearing) in clearings.iter().enumerate() {
        let surfaces = exact_position_set(&clearing.coords, &surface_by_coord)?;
        clearing_coords.extend(clearing.coords.iter().copied());
        clearing_plans.insert(
            format!("forest_clearing_{index}"),
            FeatureClearing { surfaces },
        );
    }

    let mut tree_exclusions = clearing_coords.iter().copied().collect::<BTreeSet<_>>();
    tree_exclusions.extend(
        party_coord
            .within_radius(1)
            .into_iter()
            .filter(|coord| mask.contains(coord)),
    );
    let tree_roots = select_tree_roots(
        &woodland,
        &tree_exclusions,
        &surface_by_coord,
        streams.map(|streams| streams.trees),
    );
    let tree_root_coords: BTreeSet<_> = tree_roots.iter().map(|root| root.coord).collect();
    let road = plan_road(
        radius,
        rotation,
        route_offset,
        &mask,
        &surface_by_coord,
        &tree_root_coords,
        &clearings,
        streams.map(|streams| streams.routes),
    )?;

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

    let road_centerline = exact_positions(&road.centerline, &surface_by_coord)?;
    let road_surfaces = exact_position_set(&road.surfaces, &surface_by_coord)?;
    let prairie: BTreeSet<_> = mask.difference(&woodland).copied().collect();
    let mut grass_exclusions = road.surfaces.clone();
    grass_exclusions.extend(
        hostile_coord
            .within_radius(1)
            .into_iter()
            .filter(|coord| mask.contains(coord)),
    );
    let grass_roots = select_grass_roots(
        &prairie,
        &grass_exclusions,
        &surface_by_coord,
        streams.map(|streams| streams.grass),
    );
    let (features, blockers) = build_feature_plan(
        tree_roots,
        grass_roots,
        road_centerline,
        road_surfaces,
        clearing_plans,
    );

    let party_start = surface_by_coord
        .get(&party_coord)
        .copied()
        .ok_or_else(|| vec![recipe_issue("Forest route has no party surface")])?;
    let hostile_start = surface_by_coord
        .get(&hostile_coord)
        .copied()
        .ok_or_else(|| vec![recipe_issue("Forest route has no hostile surface")])?;
    let forest_clearing = clearings
        .first()
        .and_then(|clearing| surface_by_coord.get(&clearing.center))
        .copied()
        .ok_or_else(|| vec![recipe_issue("Forest has no clearing anchor surface")])?;
    let prairie_coord = rotate(HexCoord::from_axial(radius / 2, -radius / 4), rotation);
    let prairie_overlook = surface_by_coord
        .get(&prairie_coord)
        .copied()
        .ok_or_else(|| vec![recipe_issue("Forest has no prairie overlook surface")])?;
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
    let view_hint = forest_view_hint(layout.grid_radius, level_height, rotation)?;

    Ok(GeneratedWorldPlan {
        layout,
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
    })
}

fn build_feature_plan(
    tree_roots: BTreeSet<TilePos>,
    grass_roots: BTreeSet<TilePos>,
    road_centerline: Vec<TilePos>,
    road_surfaces: BTreeSet<TilePos>,
    clearings: BTreeMap<String, FeatureClearing>,
) -> (FeaturePlan, BTreeSet<TilePos>) {
    let blockers = tree_roots.clone();
    let mut by_id = BTreeMap::new();
    let mut next_id = 0_u32;
    for (kind, roots) in [
        (FeatureKind::Tree, tree_roots),
        (FeatureKind::TallGrass, grass_roots),
    ] {
        for root in roots {
            by_id.insert(FeatureId(next_id), PlannedFeature { root, kind });
            next_id = next_id.saturating_add(1);
        }
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

fn validate_forest(plan: &GeneratedWorldPlan) -> WorldValidation<ForestMetrics> {
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

    let Some((rotation, route_offset)) = detect_orientation(plan) else {
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
    let woodland: BTreeSet<_> = plan
        .volume
        .surfaces
        .keys()
        .copied()
        .filter(|position| is_woodland(position.coord, rotation))
        .collect();
    let prairie: BTreeSet<_> = plan
        .volume
        .surfaces
        .keys()
        .copied()
        .filter(|position| !is_woodland(position.coord, rotation))
        .collect();
    let total_surfaces = plan.volume.surfaces.len();
    if woodland.len().saturating_mul(100) < total_surfaces.saturating_mul(42)
        || woodland.len().saturating_mul(100) > total_surfaces.saturating_mul(58)
    {
        issues.push(recipe_issue(format!(
            "Forest woodland must cover 42-58% of surfaces, got {}/{}",
            woodland.len(),
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
    if !tree_roots.is_subset(&woodland) {
        issues.push(recipe_issue("Forest tree roots leave the woodland side"));
    }
    if !grass_roots.is_subset(&prairie) {
        issues.push(recipe_issue("Forest tall grass leaves the prairie side"));
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
    if tree_roots.len().saturating_mul(100) < woodland.len().saturating_mul(20)
        || tree_roots.len().saturating_mul(100) > woodland.len().saturating_mul(24)
    {
        issues.push(recipe_issue(format!(
            "Forest tree density is outside 20-24% of woodland: {}/{}",
            tree_roots.len(),
            woodland.len()
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
    validate_road(
        plan,
        &ordinary,
        road,
        rotation,
        route_offset,
        &tree_root_coords,
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
    let expected_overlook_coord = rotate(HexCoord::from_axial(radius / 2, -(radius / 4)), rotation);
    let expected_overlook = prairie
        .iter()
        .find(|position| position.coord == expected_overlook_coord);
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
    if first.coord != expected_first_coord
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

fn plan_road(
    radius: i32,
    rotation: u8,
    route_offset: i32,
    mask: &BTreeSet<HexCoord>,
    surfaces: &BTreeMap<HexCoord, TilePos>,
    trees: &BTreeSet<HexCoord>,
    clearings: &[PlannedClearing],
    stream: Option<SeedStream<'_>>,
) -> Result<PlannedRoad, Vec<WorldValidationIssue>> {
    let start = rotate(
        HexCoord::from_axial((-radius).saturating_add(2), route_offset),
        rotation,
    );
    let end = rotate(
        HexCoord::from_axial(PRAIRIE_TAPER_DEPTH, route_offset),
        rotation,
    );
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
    for (segment_index, pair) in [start, early, late, end].windows(2).enumerate() {
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
                || unrotate(neighbor, rotation).x() > PRAIRIE_TAPER_DEPTH
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
        let mut center = rotate(base, rotation);
        if let Some(stream) = stream {
            let options = std::iter::once(center)
                .chain(center.neighbors())
                .collect::<Vec<_>>();
            let option_count = u64::try_from(options.len()).unwrap_or(1);
            let sampled = usize::try_from(
                stream.sample(u64::try_from(index).unwrap_or_default()) % option_count,
            )
            .unwrap_or_default();
            if let Some(candidate) = options.get(sampled).copied() {
                center = candidate;
            }
        }
        if !mask.contains(&center) || !woodland.contains(&center) || claimed.contains(&center) {
            return Err(vec![recipe_issue(format!(
                "Forest clearing {index} center leaves its available woodland footprint"
            ))]);
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
        if interior.len() < 10 {
            return Err(vec![recipe_issue(format!(
                "Forest clearing {index} cannot fit ten irregular surfaces"
            ))]);
        }
        claimed.extend(interior.iter().copied());
        clearings.push(PlannedClearing {
            center,
            coords: interior,
        });
    }
    Ok(clearings)
}

fn select_tree_roots(
    woodland: &BTreeSet<HexCoord>,
    exclusions: &BTreeSet<HexCoord>,
    surfaces: &BTreeMap<HexCoord, TilePos>,
    stream: Option<SeedStream<'_>>,
) -> BTreeSet<TilePos> {
    let mut eligible: Vec<_> = woodland.difference(exclusions).copied().collect();
    eligible.sort_unstable_by_key(|coord| (feature_priority(stream, *coord, 0), *coord));
    let target = woodland.len().saturating_mul(TREE_DENSITY_PERCENT) / 100;
    let mut selected = BTreeSet::new();
    for coord in eligible {
        if selected.len() >= target {
            break;
        }
        let spacing_radius = TREE_SPACING.saturating_sub(1);
        if coord
            .within_radius(spacing_radius)
            .into_iter()
            .all(|neighbor| !selected.contains(&neighbor))
        {
            selected.insert(coord);
        }
    }
    selected
        .into_iter()
        .filter_map(|coord| surfaces.get(&coord).copied())
        .collect()
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
    }
}

fn recipe_issue(detail: impl Into<String>) -> WorldValidationIssue {
    WorldValidationIssue::new(WorldIssueCode::Recipe("forest"), detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{
        PatchEdgeContractSettings, PatchEdgesSettings, PatchMaskSettings, PatchSpec,
    };
    use crate::terrain::TerrainPalette;
    use hex_core::SubstanceId;

    const BEDROCK: SubstanceId = SubstanceId(1);
    const STONE: SubstanceId = SubstanceId(2);
    const DIRT: SubstanceId = SubstanceId(3);
    const GRASS: SubstanceId = SubstanceId(4);
    const GRAVEL: SubstanceId = SubstanceId(5);
    const WATER: SubstanceId = SubstanceId(6);
    const METAL: SubstanceId = SubstanceId(7);
    const SNOW: SubstanceId = SubstanceId(8);
    const ICE: SubstanceId = SubstanceId(9);
    const BASALT: SubstanceId = SubstanceId(10);
    const LAVA: SubstanceId = SubstanceId(11);

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
        assert!(
            (51..=55).contains(&selected.metrics.tree_roots),
            "the prior hero had 46 tree roots; review asked for 10-20% more"
        );
        assert_eq!(selected.metrics.tall_grass_roots, 155);
        assert!(
            selected.metrics.tall_grass_roots.saturating_mul(2) > selected.metrics.prairie_surfaces
        );
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
            plan.blockers.len(),
            plan.features
                .by_id
                .values()
                .filter(|feature| feature.kind == FeatureKind::Tree)
                .count()
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
            let warmup =
                super::super::build(radius, 0.4, &settings(), u64::MAX, &palette, &is_solid)
                    .expect("warm-up Forest should build");
            std::hint::black_box(warmup);

            let mut samples = Vec::new();
            for seed in 0..12 {
                let started = std::time::Instant::now();
                let build =
                    super::super::build(radius, 0.4, &settings(), seed, &palette, &is_solid)
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
            eprintln!("V3 Forest full build radius {radius}: median={median:?} p95={p95:?}");
            assert!(
                median < budget && p95 < budget,
                "radius {radius} median={median:?} p95={p95:?}, budget={budget:?}"
            );
        }
    }
}
