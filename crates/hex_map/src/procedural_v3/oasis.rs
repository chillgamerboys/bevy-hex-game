//! Pure semantic Oasis recipe for procedural generator V3.
//!
//! Oasis owns one exact, local still-water pool, its green shore, and an exact
//! count of authored date palms. The water surface sits one voxel below the
//! surrounding grass datum. The pool never participates in a composite liquid
//! seam: ordinary seam shaping is applied only to the surrounding dry terrain,
//! while the complete pool and grass ring remain in the patch-local frame.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use hex_assets::RuntimeArtCatalog;
use hex_core::{HexCoord, Level, TilePos};

use super::arid_landform::{oasis_grass_column, sand_column};
use super::composition::{compose_single_patch, GeneratedPatchPlan};
use super::desert_vegetation::{place_date_palms, DesertVegetationSet, DATE_PALM_ID};
use super::layout::{resolve_layout, PatchId, ResolvedLayoutPlan};
use super::liquid::{LiquidBodyId, LiquidBodyPlan, LiquidFlowState, LiquidNode, LiquidPlan};
use super::patch::{PatchBuildMode, PatchRecipeContext};
use super::seam::{shape_walker_seams, validate_patch_walker_seams};
use super::selection::{
    run_recipe, CandidateAttemptError, CandidateContext, FallbackContext, RepairOutcome, V3Recipe,
    ValidatedWorldSelection, WorldValidation,
};
use super::traversal::OrdinaryGraph;
use super::vegetation::VegetationObjectSpec;
use super::vegetation_landform::{actor_anchors, view_hint};
use super::volume::{
    FillMaterialRole, LevelInterval, NonSolidFill, SolidMaterialRole, SurfaceAccess,
    SurfaceMetadata, VolumeColumn, VolumeElement, VolumePlan,
};
use super::world::{
    FeatureKind, GeneratedWorldPlan, InteriorPlan, StructurePlan, WorldIssueCode,
    WorldValidationIssue,
};
use super::V3GenerationError;
use crate::procedural::OasisMetrics;
use crate::settings::{
    ProceduralV3Settings, V3EnvironmentSettings, V3LayoutSettings, V3OasisSettings,
    V3RecipeSettings, MAX_V3_LEVEL,
};

const PARTY_START: &str = "party_start";
const HOSTILE_START: &str = "hostile_start";
const OASIS_OVERLOOK: &str = "oasis_overlook";
const MIN_SINGLE_RADIUS: u32 = 12;
const MAX_SINGLE_RADIUS: u32 = 40;
const PALM_BELT_SAND_WIDTH: u32 = 4;
const POOL_SURFACE_DROP: Level = 1;
const POOL_BODY: LiquidBodyId = LiquidBodyId(0);

#[derive(Debug)]
struct OasisRecipe {
    level_height: f32,
    layout: ResolvedLayoutPlan,
    settings: V3OasisSettings,
    vegetation: DesertVegetationSet,
    #[cfg(test)]
    reject_candidates: bool,
}

/// Runs the common eight-candidate selector for one standalone Oasis.
pub(crate) fn generate(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV3Settings,
    seed: u64,
    catalog: &RuntimeArtCatalog,
) -> Result<ValidatedWorldSelection<OasisMetrics>, V3GenerationError> {
    if !level_height.is_finite() || level_height <= 0.0 {
        return Err(V3GenerationError::RecipeContract(
            "Oasis level height must be positive and finite".to_owned(),
        ));
    }
    let recipe = *validate_recipe_settings(settings, grid_radius)?;
    let vegetation = DesertVegetationSet::resolve(catalog, "Oasis")
        .map_err(V3GenerationError::RecipeContract)?;
    let layout = resolve_layout(grid_radius, settings)
        .map_err(|error| V3GenerationError::RecipeContract(error.to_string()))?;
    run_recipe(
        &OasisRecipe {
            level_height,
            layout,
            settings: recipe,
            vegetation,
            #[cfg(test)]
            reject_candidates: false,
        },
        settings,
        grid_radius,
        seed,
    )
}

impl V3Recipe for OasisRecipe {
    type Settings = ProceduralV3Settings;
    type Metrics = OasisMetrics;
    type Score = (Level, u8);

    fn construct(
        &self,
        context: CandidateContext,
        _settings: &Self::Settings,
    ) -> Result<GeneratedWorldPlan, CandidateAttemptError> {
        #[cfg(test)]
        if self.reject_candidates {
            return Err(CandidateAttemptError::Rejected(vec![recipe_issue(
                "forced candidate rejection",
            )]));
        }
        if context.grid_radius != self.layout.grid_radius {
            return Err(CandidateAttemptError::Fatal(
                V3GenerationError::RecipeContract(
                    "Oasis candidate radius disagrees with its resolved layout".to_owned(),
                ),
            ));
        }
        let patch = PatchRecipeContext::resolve(&self.layout, PatchId(0))
            .map_err(CandidateAttemptError::Fatal)?;
        let fragment = construct_patch_with_vegetation(
            patch,
            &self.settings,
            V3EnvironmentSettings::Arid,
            self.level_height,
            PatchBuildMode::Candidate {
                world_seed: context.seed,
                candidate: context.candidate,
            },
            &self.vegetation,
        )
        .map_err(CandidateAttemptError::Rejected)?;
        compose_single_patch(self.layout.clone(), fragment).map_err(|error| {
            CandidateAttemptError::Fatal(V3GenerationError::RecipeContract(format!(
                "Oasis single-patch composition failed: {error:?}"
            )))
        })
    }

    fn validate(
        &self,
        _settings: &Self::Settings,
        plan: &GeneratedWorldPlan,
    ) -> WorldValidation<Self::Metrics> {
        validate_oasis(
            plan,
            &self.settings,
            &self.vegetation.date_palm,
            &BTreeSet::new(),
        )
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
        (metrics.relief, candidate)
    }

    fn canonical_fallback(
        &self,
        context: FallbackContext,
        _settings: &Self::Settings,
    ) -> Result<GeneratedWorldPlan, V3GenerationError> {
        if context.grid_radius != self.layout.grid_radius {
            return Err(V3GenerationError::RecipeContract(
                "Oasis fallback radius disagrees with its resolved layout".to_owned(),
            ));
        }
        let patch = PatchRecipeContext::resolve(&self.layout, PatchId(0))?;
        let fragment = construct_patch_with_vegetation(
            patch,
            &self.settings,
            V3EnvironmentSettings::Arid,
            self.level_height,
            PatchBuildMode::CanonicalFallback,
            &self.vegetation,
        )
        .map_err(recipe_issues_to_error)?;
        compose_single_patch(self.layout.clone(), fragment).map_err(|error| {
            V3GenerationError::RecipeContract(format!(
                "Oasis fallback composition failed: {error:?}"
            ))
        })
    }
}

fn validate_recipe_settings(
    settings: &ProceduralV3Settings,
    grid_radius: u32,
) -> Result<&V3OasisSettings, V3GenerationError> {
    let V3LayoutSettings::Single(patch) = &settings.layout else {
        return Err(V3GenerationError::RecipeUnavailable("Oasis Single"));
    };
    let V3RecipeSettings::Oasis(recipe) = &patch.recipe else {
        return Err(V3GenerationError::RecipeUnavailable("Oasis"));
    };
    if patch.environment != V3EnvironmentSettings::Arid {
        return Err(V3GenerationError::RecipeContract(
            "Oasis requires the Arid environment".to_owned(),
        ));
    }
    if !patch.overlays.is_empty() {
        return Err(V3GenerationError::RecipeContract(
            "Oasis overlays are not implemented yet".to_owned(),
        ));
    }
    if !(MIN_SINGLE_RADIUS..=MAX_SINGLE_RADIUS).contains(&grid_radius) {
        return Err(V3GenerationError::RecipeContract(format!(
            "procedural V3 Oasis requires grid_radius from {MIN_SINGLE_RADIUS} through \
             {MAX_SINGLE_RADIUS}"
        )));
    }
    validate_values(recipe).map_err(V3GenerationError::RecipeContract)?;
    let reserved_radius = oasis_outer_radius(recipe).saturating_add(4);
    if reserved_radius > grid_radius {
        return Err(V3GenerationError::RecipeContract(
            "Oasis pool and grass ring must leave four columns for dry approaches".to_owned(),
        ));
    }
    Ok(recipe)
}

fn validate_values(settings: &V3OasisSettings) -> Result<(), String> {
    if settings.base_level < 5 || settings.base_level > MAX_V3_LEVEL {
        return Err(format!(
            "V3 Oasis base_level must be between 5 and {MAX_V3_LEVEL}"
        ));
    }
    if !(3..=6).contains(&settings.pool_radius) {
        return Err("V3 Oasis pool_radius must be between 3 and 6".to_owned());
    }
    if !(8..=18).contains(&settings.palm_count) {
        return Err("V3 Oasis palm_count must be between 8 and 18".to_owned());
    }
    if !(2..=4).contains(&settings.grass_ring_width) {
        return Err("V3 Oasis grass_ring_width must be between 2 and 4".to_owned());
    }
    Ok(())
}

/// Constructs one patch-ready Oasis fragment using the accepted art catalog.
pub(crate) fn construct_patch(
    patch: PatchRecipeContext<'_>,
    settings: &V3OasisSettings,
    environment: V3EnvironmentSettings,
    level_height: f32,
    mode: PatchBuildMode,
    catalog: &RuntimeArtCatalog,
) -> Result<GeneratedPatchPlan, Vec<WorldValidationIssue>> {
    let vegetation = DesertVegetationSet::resolve(catalog, "Oasis")
        .map_err(|error| vec![recipe_issue(error)])?;
    construct_patch_with_vegetation(
        patch,
        settings,
        environment,
        level_height,
        mode,
        &vegetation,
    )
}

fn construct_patch_with_vegetation(
    patch: PatchRecipeContext<'_>,
    settings: &V3OasisSettings,
    environment: V3EnvironmentSettings,
    level_height: f32,
    mode: PatchBuildMode,
    vegetation: &DesertVegetationSet,
) -> Result<GeneratedPatchPlan, Vec<WorldValidationIssue>> {
    if environment != V3EnvironmentSettings::Arid {
        return Err(vec![recipe_issue("Oasis requires the Arid environment")]);
    }
    if !level_height.is_finite() || level_height <= 0.0 {
        return Err(vec![recipe_issue(
            "Oasis level height must be positive and finite",
        )]);
    }
    validate_values(settings).map_err(|error| vec![recipe_issue(error)])?;

    let frame = patch
        .local_frame()
        .map_err(|error| vec![recipe_issue(error.to_string())])?;
    let mask = frame
        .local_mask(patch.mask())
        .map_err(|error| vec![recipe_issue(error)])?;
    let (pool_coords, grass_coords) = oasis_footprint(settings);
    if !pool_coords.is_subset(&mask) || !grass_coords.is_subset(&mask) {
        return Err(vec![recipe_issue(
            "Oasis patch mask cannot contain the complete local pool and grass ring",
        )]);
    }
    let protected_approaches = local_protected_approaches(patch, frame)?;
    if !pool_coords.is_disjoint(&protected_approaches)
        || !grass_coords.is_disjoint(&protected_approaches)
    {
        return Err(vec![recipe_issue(
            "Oasis local pool and grass ring must not enter a protected seam approach",
        )]);
    }

    let local_levels = mask
        .iter()
        .copied()
        .map(|coord| (coord, settings.base_level))
        .collect();
    let mut world_levels = frame
        .levels_to_world(local_levels)
        .map_err(|error| vec![recipe_issue(error)])?;
    let seam_shape = shape_walker_seams(&patch, &mut world_levels)?;
    let local_levels = frame
        .levels_to_local(world_levels)
        .map_err(|error| vec![recipe_issue(error)])?;

    let mut columns = BTreeMap::new();
    let mut surfaces = BTreeMap::new();
    let mut dry_surfaces = BTreeMap::new();
    let mut ordinary_dry = BTreeMap::new();
    let mut grass_surfaces = BTreeMap::new();
    let mut water_nodes = BTreeMap::new();
    let water_level = oasis_water_level(settings);
    for coord in &mask {
        if pool_coords.contains(coord) {
            let bed_level = water_level.saturating_sub(1);
            let bed = TilePos::new(*coord, bed_level);
            columns.insert(*coord, oasis_pool_column(water_level));
            surfaces.insert(
                bed,
                SurfaceMetadata {
                    access: SurfaceAccess::NonStandable,
                    interior: None,
                },
            );
            water_nodes.insert(
                TilePos::new(*coord, water_level),
                LiquidNode {
                    state: LiquidFlowState::Still,
                    downstream: None,
                },
            );
            continue;
        }

        let level = local_levels.get(coord).copied().ok_or_else(|| {
            vec![recipe_issue(format!(
                "Oasis dry land plan omitted coordinate {coord:?}"
            ))]
        })?;
        let position = TilePos::new(*coord, level);
        let world_position = frame
            .position_to_world(position)
            .map_err(|error| vec![recipe_issue(error)])?;
        let access = seam_shape.access_for(world_position, SurfaceAccess::Ordinary);
        let grass = grass_coords.contains(coord);
        columns.insert(
            *coord,
            if grass {
                oasis_grass_column(level)
            } else {
                sand_column(level)
            },
        );
        surfaces.insert(
            position,
            SurfaceMetadata {
                access,
                interior: None,
            },
        );
        dry_surfaces.insert(*coord, position);
        if access == SurfaceAccess::Ordinary {
            ordinary_dry.insert(*coord, position);
            if grass {
                grass_surfaces.insert(*coord, position);
            }
        }
    }

    let (party_start, hostile_start) = actor_anchors(&ordinary_dry, "oasis")?;
    let oasis_overlook = select_overlook(&grass_surfaces)
        .ok_or_else(|| vec![recipe_issue("Oasis has no ordinary grass-ring overlook")])?;
    let anchors = BTreeMap::from([
        (PARTY_START.to_owned(), party_start),
        (HOSTILE_START.to_owned(), hostile_start),
        (OASIS_OVERLOOK.to_owned(), oasis_overlook),
    ]);
    let reserved = palm_reserved_coords(
        &pool_coords,
        &protected_approaches,
        anchors.values().copied(),
    );
    let palm_belt = palm_belt_coords(&mask, settings);
    let palm_candidates = ordinary_dry
        .keys()
        .copied()
        .filter(|coord| palm_belt.contains(coord))
        .collect::<BTreeSet<_>>();
    let streams = mode.seed_streams(&patch);
    let (features, blockers) = place_date_palms(
        "Oasis",
        &vegetation.date_palm,
        &dry_surfaces,
        &palm_candidates,
        &reserved,
        usize::from(settings.palm_count),
        streams.map(|streams| streams.stage("oasis.palm-priority")),
        streams.map(|streams| streams.stage("oasis.palm-rotation")),
    )
    .map_err(|error| vec![recipe_issue(error)])?;

    let volume = VolumePlan {
        mask: mask.clone(),
        columns,
        surfaces,
    };
    let biome_regions = volume
        .surfaces
        .keys()
        .copied()
        .map(|surface| (surface, patch.biome_region()))
        .collect();
    let mut plan = GeneratedPatchPlan {
        patch_id: patch.id,
        volume,
        liquids: LiquidPlan {
            bodies: BTreeMap::from([(
                POOL_BODY,
                LiquidBodyPlan {
                    material: FillMaterialRole::Water,
                    nodes: water_nodes,
                },
            )]),
        },
        features,
        structures: StructurePlan::default(),
        blockers,
        lights: BTreeMap::new(),
        biome_regions,
        interiors: InteriorPlan::default(),
        anchors,
        view_hint: view_hint(frame.scale(), settings.base_level, 0, level_height, "oasis")?,
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

/// Validates one patch-ready Oasis fragment in its canonical local frame.
pub(crate) fn validate_patch(
    patch: PatchRecipeContext<'_>,
    settings: &V3OasisSettings,
    plan: &GeneratedPatchPlan,
    catalog: &RuntimeArtCatalog,
) -> WorldValidation<OasisMetrics> {
    let frame = match patch.local_frame() {
        Ok(frame) => frame,
        Err(error) => {
            return WorldValidation::Invalid(vec![recipe_issue(format!(
                "Oasis validation frame failed: {error}"
            ))]);
        }
    };
    let protected_approaches = match local_protected_approaches(patch, frame) {
        Ok(protected) => protected,
        Err(issues) => return WorldValidation::Invalid(issues),
    };
    let vegetation = match DesertVegetationSet::resolve(catalog, "Oasis") {
        Ok(vegetation) => vegetation,
        Err(error) => return WorldValidation::Invalid(vec![recipe_issue(error)]),
    };
    let mut world = match frame.canonical_local_world(plan) {
        Ok(world) => world,
        Err(error) => {
            return WorldValidation::Invalid(vec![recipe_issue(format!(
                "Oasis validation projection failed: {error}"
            ))]);
        }
    };
    world.layout.grid_radius = world
        .layout
        .footprint
        .iter()
        .map(|coord| HexCoord::ORIGIN.distance(*coord))
        .max()
        .unwrap_or(MIN_SINGLE_RADIUS)
        .max(MIN_SINGLE_RADIUS);
    validate_oasis(
        &world,
        settings,
        &vegetation.date_palm,
        &protected_approaches,
    )
}

fn validate_oasis(
    plan: &GeneratedWorldPlan,
    settings: &V3OasisSettings,
    palm: &VegetationObjectSpec,
    protected_approaches: &BTreeSet<HexCoord>,
) -> WorldValidation<OasisMetrics> {
    let mut issues = plan.validate();
    if !plan.structures.by_id.is_empty()
        || !plan.lights.is_empty()
        || !plan.interiors.by_id.is_empty()
        || !plan.features.protected_routes.is_empty()
        || !plan.features.clearings.is_empty()
    {
        issues.push(recipe_issue(
            "Oasis must not contain structures, lights, interiors, feature routes, or clearings",
        ));
    }
    if let Err(error) = validate_values(settings) {
        issues.push(recipe_issue(error));
    }

    let (pool_coords, grass_coords) = oasis_footprint(settings);
    if !pool_coords.is_subset(&plan.volume.mask) || !grass_coords.is_subset(&plan.volume.mask) {
        issues.push(recipe_issue(
            "Oasis mask does not contain its exact pool and grass ring",
        ));
    }
    if !pool_coords.is_disjoint(protected_approaches)
        || !grass_coords.is_disjoint(protected_approaches)
    {
        issues.push(recipe_issue(
            "Oasis pool or grass ring enters a protected seam approach",
        ));
    }
    validate_pool(plan, settings, &pool_coords, &mut issues);
    let surface_by_coord =
        validate_columns(plan, settings, &pool_coords, &grass_coords, &mut issues);

    let expected_anchor_names = BTreeSet::from([
        PARTY_START.to_owned(),
        HOSTILE_START.to_owned(),
        OASIS_OVERLOOK.to_owned(),
    ]);
    let actual_anchor_names = plan.anchors.keys().cloned().collect::<BTreeSet<_>>();
    if actual_anchor_names != expected_anchor_names {
        issues.push(recipe_issue(format!(
            "Oasis anchors must be exactly party_start, hostile_start, and oasis_overlook; got \
             {actual_anchor_names:?}"
        )));
    }

    let ordinary = OrdinaryGraph::from_volume(&plan.volume, Some(&plan.blockers));
    let ordinary_by_coord = ordinary
        .positions()
        .map(|position| (position.coord, position))
        .collect::<BTreeMap<_, _>>();
    let party = plan.anchors.get(PARTY_START).copied();
    let hostile = plan.anchors.get(HOSTILE_START).copied();
    let overlook = plan.anchors.get(OASIS_OVERLOOK).copied();
    match actor_anchors(&ordinary_by_coord, "oasis") {
        Ok((expected_party, expected_hostile)) => {
            if party != Some(expected_party) || hostile != Some(expected_hostile) {
                issues.push(recipe_issue(format!(
                    "Oasis actor anchors drifted from their deterministic dry landings: party \
                     {party:?}/{expected_party:?}, hostile {hostile:?}/{expected_hostile:?}"
                )));
            }
        }
        Err(anchor_issues) => issues.extend(anchor_issues),
    }
    let grass_ordinary = ordinary_by_coord
        .iter()
        .filter_map(|(coord, position)| grass_coords.contains(coord).then_some((*coord, *position)))
        .collect::<BTreeMap<_, _>>();
    let expected_overlook = select_overlook(&grass_ordinary);
    if overlook != expected_overlook {
        issues.push(recipe_issue(format!(
            "Oasis overlook drifted from its deterministic inner grass-ring landing: \
             {overlook:?}/{expected_overlook:?}"
        )));
    }

    let mut critical_route_steps = 0;
    if let Some(party) = party {
        let distances = ordinary.distances_from(party);
        if distances.len() != ordinary.len() {
            issues.push(recipe_issue(format!(
                "Oasis ordinary dry terrain is disconnected: {}/{} reachable",
                distances.len(),
                ordinary.len()
            )));
        }
        if let Some(hostile) = hostile {
            critical_route_steps = distances.get(&hostile).copied().unwrap_or_default();
            if !distances.contains_key(&hostile) {
                issues.push(recipe_issue(
                    "Oasis actor anchors are not ordinarily connected",
                ));
            }
        }
        if overlook.is_some_and(|overlook| !distances.contains_key(&overlook)) {
            issues.push(recipe_issue(
                "Oasis overlook is not ordinarily connected to party_start",
            ));
        }
    } else {
        issues.push(recipe_issue("Oasis requires party_start"));
    }

    let reserved = palm_reserved_coords(
        &pool_coords,
        protected_approaches,
        plan.anchors.values().copied(),
    );
    let palm_belt = palm_belt_coords(&plan.volume.mask, settings);
    validate_palms(
        plan,
        settings,
        palm,
        &palm_belt,
        &surface_by_coord,
        &reserved,
        &mut issues,
    );

    let levels = ordinary
        .positions()
        .map(|position| position.level)
        .collect::<BTreeSet<_>>();
    let relief = levels
        .first()
        .zip(levels.last())
        .map_or(0, |(lowest, highest)| highest.saturating_sub(*lowest));
    if levels
        .first()
        .is_none_or(|level| *level < settings.base_level)
        || levels.last().is_none_or(|level| *level > MAX_V3_LEVEL)
    {
        issues.push(recipe_issue(format!(
            "Oasis dry surfaces must remain within {}..={MAX_V3_LEVEL}; got {levels:?}",
            settings.base_level
        )));
    }

    if !issues.is_empty() {
        return WorldValidation::Invalid(issues);
    }
    WorldValidation::Valid(OasisMetrics {
        ordinary_surfaces: count_u32(ordinary.len()),
        water_cells: count_u32(pool_coords.len()),
        grass_ring_surfaces: count_u32(grass_coords.len()),
        palm_roots: count_u32(plan.features.by_id.len()),
        reachable_elevation_levels: count_u32(levels.len()),
        relief,
        critical_route_steps,
    })
}

fn validate_pool(
    plan: &GeneratedWorldPlan,
    settings: &V3OasisSettings,
    pool_coords: &BTreeSet<HexCoord>,
    issues: &mut Vec<WorldValidationIssue>,
) {
    let water_level = oasis_water_level(settings);
    let expected_nodes = pool_coords
        .iter()
        .copied()
        .map(|coord| {
            (
                TilePos::new(coord, water_level),
                LiquidNode {
                    state: LiquidFlowState::Still,
                    downstream: None,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let expected_body_ids = BTreeSet::from([POOL_BODY]);
    let actual_body_ids = plan.liquids.bodies.keys().copied().collect::<BTreeSet<_>>();
    if actual_body_ids != expected_body_ids {
        issues.push(recipe_issue(format!(
            "Oasis requires exactly one local liquid body {POOL_BODY:?}; got {actual_body_ids:?}"
        )));
        return;
    }
    let Some(body) = plan.liquids.bodies.get(&POOL_BODY) else {
        return;
    };
    if body.material != FillMaterialRole::Water || body.nodes != expected_nodes {
        issues.push(recipe_issue(
            "Oasis pool must exactly fill its local radius with Still water one level below the \
             grass datum and no downstream",
        ));
    }
}

fn validate_columns(
    plan: &GeneratedWorldPlan,
    settings: &V3OasisSettings,
    pool_coords: &BTreeSet<HexCoord>,
    grass_coords: &BTreeSet<HexCoord>,
    issues: &mut Vec<WorldValidationIssue>,
) -> BTreeMap<HexCoord, TilePos> {
    let surface_by_coord = plan
        .volume
        .surfaces
        .keys()
        .map(|surface| (surface.coord, *surface))
        .collect::<BTreeMap<_, _>>();
    if surface_by_coord.len() != plan.volume.mask.len()
        || plan.volume.surfaces.len() != plan.volume.mask.len()
    {
        issues.push(recipe_issue(format!(
            "Oasis requires exactly one exposed bed or dry surface per owned column: {}/{}",
            plan.volume.surfaces.len(),
            plan.volume.mask.len()
        )));
    }
    for coord in &plan.volume.mask {
        let Some(surface) = surface_by_coord.get(coord).copied() else {
            issues.push(recipe_issue(format!(
                "Oasis column {coord:?} has no exposed surface"
            )));
            continue;
        };
        let Some(column) = plan.volume.columns.get(coord) else {
            continue;
        };
        if pool_coords.contains(coord) {
            let water_level = oasis_water_level(settings);
            let expected_surface = TilePos::new(*coord, water_level.saturating_sub(1));
            if surface != expected_surface
                || plan.volume.surfaces.get(&surface).is_none_or(|metadata| {
                    metadata.access != SurfaceAccess::NonStandable || metadata.interior.is_some()
                })
                || *column != oasis_pool_column(water_level)
            {
                issues.push(recipe_issue(format!(
                    "Oasis pool column {coord:?} must expose one nonstandable sand bed exactly one \
                     level below its one-level water fill, with the water surface exactly one \
                     level below the grass datum"
                )));
            }
            continue;
        }
        if plan.volume.surfaces.get(&surface).is_none_or(|metadata| {
            !matches!(
                metadata.access,
                SurfaceAccess::Ordinary | SurfaceAccess::SpecialMovement(_)
            ) || metadata.interior.is_some()
        }) {
            issues.push(recipe_issue(format!(
                "Oasis dry surface {surface:?} is not standable exterior terrain"
            )));
        }
        let expected = if grass_coords.contains(coord) {
            oasis_grass_column(surface.level)
        } else {
            sand_column(surface.level)
        };
        if *column != expected {
            let expected_cap = if grass_coords.contains(coord) {
                SolidMaterialRole::Grass
            } else {
                SolidMaterialRole::Sand
            };
            issues.push(recipe_issue(format!(
                "Oasis dry column {coord:?} does not use exact shared strata with \
                 {expected_cap:?} at {surface:?}"
            )));
        }
    }
    surface_by_coord
}

fn validate_palms(
    plan: &GeneratedWorldPlan,
    settings: &V3OasisSettings,
    palm: &VegetationObjectSpec,
    palm_belt: &BTreeSet<HexCoord>,
    surfaces: &BTreeMap<HexCoord, TilePos>,
    reserved: &BTreeSet<HexCoord>,
    issues: &mut Vec<WorldValidationIssue>,
) {
    let mut roots = BTreeSet::new();
    let mut visual_cells = BTreeSet::new();
    let mut exact_blockers = BTreeSet::new();
    for feature in plan.features.by_id.values() {
        if feature.kind != FeatureKind::Tree
            || feature.object_id != palm.id
            || feature.object_id.as_str() != DATE_PALM_ID
        {
            issues.push(recipe_issue(format!(
                "Oasis feature at {:?} is not the accepted date palm",
                feature.root
            )));
            continue;
        }
        if !roots.insert(feature.root) {
            issues.push(recipe_issue(format!(
                "Oasis repeats a date-palm root at {:?}",
                feature.root
            )));
        }
        if !palm_belt.contains(&feature.root.coord) || reserved.contains(&feature.root.coord) {
            issues.push(recipe_issue(format!(
                "Oasis date palm at {:?} leaves its route-and-anchor-safe oasis belt",
                feature.root
            )));
        }
        let Some(projected) = palm.project_visual_volume(feature.root, feature.rotation) else {
            issues.push(recipe_issue(format!(
                "Oasis date palm at {:?} cannot project its complete rotated authored volume",
                feature.root
            )));
            continue;
        };
        if projected.cells.iter().any(|cell| {
            surfaces
                .get(&cell.coord)
                .is_none_or(|support| cell.level <= support.level)
        }) {
            issues.push(recipe_issue(format!(
                "Oasis date palm at {:?} intersects terrain or leaves the owned surface mask",
                feature.root
            )));
        }
        if !visual_cells.is_disjoint(&projected.cells) {
            issues.push(recipe_issue(format!(
                "Oasis date-palm authored volumes overlap at {:?}",
                feature.root
            )));
        }
        visual_cells.extend(projected.cells);

        let Some(projected_blockers) =
            palm.project_blockers(feature.root, feature.rotation, surfaces)
        else {
            issues.push(recipe_issue(format!(
                "Oasis date palm at {:?} cannot project its exact blocker",
                feature.root
            )));
            continue;
        };
        if projected_blockers != feature.blocker_footprint {
            issues.push(recipe_issue(format!(
                "Oasis date palm at {:?} stores a blocker different from its authored projection",
                feature.root
            )));
        }
        if projected_blockers
            .iter()
            .any(|blocker| reserved.contains(&blocker.coord))
            || !exact_blockers.is_disjoint(&projected_blockers)
        {
            issues.push(recipe_issue(format!(
                "Oasis date palm at {:?} overlaps a reserved or already blocked footing",
                feature.root
            )));
        }
        exact_blockers.extend(projected_blockers);
    }
    let expected = usize::from(settings.palm_count);
    if plan.features.by_id.len() != expected || roots.len() != expected {
        issues.push(recipe_issue(format!(
            "Oasis requires exactly {expected} unique date palms, got {} features and {} roots",
            plan.features.by_id.len(),
            roots.len()
        )));
    }
    if plan.blockers != exact_blockers {
        issues.push(recipe_issue(
            "Oasis traversal blockers must exactly equal projected date-palm roots",
        ));
    }
}

fn oasis_footprint(settings: &V3OasisSettings) -> (BTreeSet<HexCoord>, BTreeSet<HexCoord>) {
    let pool_radius = u32::from(settings.pool_radius);
    let pool = HexCoord::ORIGIN
        .within_radius(pool_radius)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let grass = HexCoord::ORIGIN
        .within_radius(oasis_outer_radius(settings))
        .into_iter()
        .filter(|coord| !pool.contains(coord))
        .collect();
    (pool, grass)
}

fn oasis_outer_radius(settings: &V3OasisSettings) -> u32 {
    u32::from(settings.pool_radius).saturating_add(u32::from(settings.grass_ring_width))
}

fn oasis_water_level(settings: &V3OasisSettings) -> Level {
    settings.base_level.saturating_sub(POOL_SURFACE_DROP)
}

fn palm_belt_coords(mask: &BTreeSet<HexCoord>, settings: &V3OasisSettings) -> BTreeSet<HexCoord> {
    let pool_radius = u32::from(settings.pool_radius);
    let belt_radius = oasis_outer_radius(settings).saturating_add(PALM_BELT_SAND_WIDTH);
    mask.iter()
        .copied()
        .filter(|coord| {
            let distance = coord.distance(HexCoord::ORIGIN);
            distance > pool_radius && distance <= belt_radius
        })
        .collect()
}

fn oasis_pool_column(water_level: Level) -> VolumeColumn {
    let mut column = sand_column(water_level.saturating_sub(1));
    column.elements.push(VolumeElement::Fill(NonSolidFill {
        levels: LevelInterval::new(water_level, water_level.saturating_add(1)),
        material: FillMaterialRole::Water,
    }));
    column
}

fn select_overlook(grass: &BTreeMap<HexCoord, TilePos>) -> Option<TilePos> {
    grass.values().copied().min_by_key(|position| {
        (
            position.coord.distance(HexCoord::ORIGIN),
            Reverse(position.level),
            *position,
        )
    })
}

fn palm_reserved_coords(
    pool: &BTreeSet<HexCoord>,
    protected_approaches: &BTreeSet<HexCoord>,
    anchors: impl IntoIterator<Item = TilePos>,
) -> BTreeSet<HexCoord> {
    let mut reserved = pool.clone();
    reserved.extend(protected_approaches.iter().copied());
    for anchor in anchors {
        reserved.extend(anchor.coord.within_radius(1));
    }
    reserved
}

fn local_protected_approaches(
    patch: PatchRecipeContext<'_>,
    frame: super::local_frame::LocalPatchFrame,
) -> Result<BTreeSet<HexCoord>, Vec<WorldValidationIssue>> {
    patch
        .protected_approaches()
        .into_iter()
        .map(|coord| frame.to_local(coord).map_err(recipe_issue))
        .collect::<Result<_, _>>()
        .map_err(|issue| vec![issue])
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
    WorldValidationIssue::new(WorldIssueCode::Recipe("oasis"), detail)
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use hex_assets::{ArtPalette, ObjectBlueprint, ObjectCatalogFile, VoxelStyleCatalog};

    use super::*;
    use crate::settings::{
        MapSettings, PatchEdgeContractSettings, PatchEdgesSettings, PatchMaskSettings, PatchSpec,
        ProceduralSettings, TerrainSettings,
    };

    fn settings() -> ProceduralV3Settings {
        ProceduralV3Settings {
            layout: V3LayoutSettings::Single(PatchSpec {
                environment: V3EnvironmentSettings::Arid,
                recipe: V3RecipeSettings::Oasis(V3OasisSettings {
                    base_level: 15,
                    pool_radius: 5,
                    palm_count: 12,
                    grass_ring_width: 3,
                }),
                overlays: Vec::new(),
                mask: PatchMaskSettings::WholeWorld,
                edges: world_boundary_edges(),
            }),
        }
    }

    fn world_boundary_edges() -> PatchEdgesSettings {
        PatchEdgesSettings {
            east: PatchEdgeContractSettings::WorldBoundary,
            south_east: PatchEdgeContractSettings::WorldBoundary,
            south_west: PatchEdgeContractSettings::WorldBoundary,
            west: PatchEdgeContractSettings::WorldBoundary,
            north_west: PatchEdgeContractSettings::WorldBoundary,
            north_east: PatchEdgeContractSettings::WorldBoundary,
        }
    }

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
            let palm: ObjectBlueprint = ron::from_str(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/art/objects/plant/date-palm.ron"
            )))
            .expect("tracked date-palm blueprint should parse");
            let manifest = ObjectCatalogFile::new([palm.id.clone()])
                .expect("date-palm-only object manifest should validate");
            RuntimeArtCatalog::from_sources(
                &palette,
                &styles,
                &manifest,
                BTreeMap::from([(palm.id.clone(), palm)]),
            )
            .expect("date-palm-only runtime art graph should resolve")
        })
    }

    fn generate(
        radius: u32,
        seed: u64,
    ) -> Result<ValidatedWorldSelection<OasisMetrics>, V3GenerationError> {
        super::generate(radius, 0.4, &settings(), seed, runtime_art_catalog())
    }

    #[test]
    fn fixed_corpus_builds_exact_local_oases_without_fallback() {
        for radius in [12, 20, 40] {
            for seed in [0, 1, 1_592_598_566, 4_294_967_311] {
                let first = generate(radius, seed).expect("Oasis should generate");
                let repeated = generate(radius, seed).expect("Oasis should repeat");
                assert_eq!(
                    first.validated.semantic_fingerprint,
                    repeated.validated.semantic_fingerprint
                );
                assert!(!first.used_fallback);
                assert_eq!(first.candidates_evaluated, 8);
                assert!(first.valid_candidates > 0);
                assert_eq!(first.metrics.water_cells, exact_disk_size(5));
                assert_eq!(first.metrics.grass_ring_surfaces, exact_ring_size(5, 3));
                assert_eq!(first.metrics.palm_roots, 12);
                assert_exact_oasis(
                    &first.validated.plan,
                    &V3OasisSettings {
                        base_level: 15,
                        pool_radius: 5,
                        palm_count: 12,
                        grass_ring_width: 3,
                    },
                );
            }
        }
    }

    #[test]
    fn pool_surface_is_one_voxel_below_the_ring_and_still_topology_is_exact() {
        let selected = generate(20, 77).expect("Oasis should generate");
        let plan = &selected.validated.plan;
        let recipe = V3OasisSettings {
            base_level: 15,
            pool_radius: 5,
            palm_count: 12,
            grass_ring_width: 3,
        };
        let (pool, grass) = oasis_footprint(&recipe);
        assert_eq!(
            pool.len(),
            usize::try_from(exact_disk_size(5)).unwrap_or_default()
        );
        assert_eq!(
            grass.len(),
            usize::try_from(exact_ring_size(5, 3)).unwrap_or_default()
        );
        let body = plan
            .liquids
            .bodies
            .get(&POOL_BODY)
            .expect("Oasis should publish one local pool body");
        assert_eq!(body.material, FillMaterialRole::Water);
        assert_eq!(body.nodes.len(), pool.len());
        assert!(plan.lights.is_empty(), "Oasis must remain daylight-only");
        for coord in pool {
            let water_level = oasis_water_level(&recipe);
            let bed = TilePos::new(coord, water_level.saturating_sub(1));
            assert_eq!(
                plan.volume
                    .surfaces
                    .get(&bed)
                    .map(|metadata| metadata.access),
                Some(SurfaceAccess::NonStandable)
            );
            assert_eq!(
                plan.volume.columns.get(&coord),
                Some(&oasis_pool_column(water_level))
            );
            assert_eq!(
                body.nodes.get(&TilePos::new(coord, water_level)),
                Some(&LiquidNode {
                    state: LiquidFlowState::Still,
                    downstream: None,
                })
            );
            assert_eq!(
                water_level.saturating_add(POOL_SURFACE_DROP),
                recipe.base_level,
                "the authoritative water surface must sit exactly one voxel below the grass ring"
            );
        }
        for coord in grass {
            let surface = TilePos::new(coord, 15);
            assert_eq!(
                plan.volume.columns.get(&coord),
                Some(&oasis_grass_column(surface.level))
            );
        }
    }

    #[test]
    fn palms_are_exact_nonoverlapping_and_leave_anchors_clear() {
        let selected = generate(20, 1_592_598_566).expect("Oasis should generate");
        let plan = &selected.validated.plan;
        let recipe = V3OasisSettings {
            base_level: 15,
            pool_radius: 5,
            palm_count: 12,
            grass_ring_width: 3,
        };
        let (pool, _grass) = oasis_footprint(&recipe);
        let palm_belt = palm_belt_coords(&plan.volume.mask, &recipe);
        let reserved =
            palm_reserved_coords(&pool, &BTreeSet::new(), plan.anchors.values().copied());
        let vegetation =
            DesertVegetationSet::resolve(runtime_art_catalog(), "Oasis test").expect("palm");
        let surface_by_coord = plan
            .volume
            .surfaces
            .keys()
            .map(|surface| (surface.coord, *surface))
            .collect::<BTreeMap<_, _>>();
        let mut visuals = BTreeSet::new();
        let mut blockers = BTreeSet::new();
        assert_eq!(plan.features.by_id.len(), 12);
        for feature in plan.features.by_id.values() {
            assert_eq!(feature.kind, FeatureKind::Tree);
            assert_eq!(feature.object_id.as_str(), DATE_PALM_ID);
            assert!(palm_belt.contains(&feature.root.coord));
            assert!(!reserved.contains(&feature.root.coord));
            let projected = vegetation
                .date_palm
                .project_visual_volume(feature.root, feature.rotation)
                .expect("accepted palm volume should reproject");
            assert!(visuals.is_disjoint(&projected.cells));
            visuals.extend(projected.cells);
            let projected_blockers = vegetation
                .date_palm
                .project_blockers(feature.root, feature.rotation, &surface_by_coord)
                .expect("accepted palm blocker should reproject");
            assert_eq!(projected_blockers, feature.blocker_footprint);
            assert!(projected_blockers
                .iter()
                .all(|blocker| !reserved.contains(&blocker.coord)));
            assert!(blockers.is_disjoint(&projected_blockers));
            blockers.extend(projected_blockers);
        }
        assert_eq!(blockers, plan.blockers);
        assert!(plan
            .anchors
            .values()
            .all(|anchor| !plan.blockers.contains(anchor)));
    }

    #[test]
    fn anchors_and_unblocked_dry_land_are_ordinarily_connected() {
        let selected = generate(20, 91).expect("Oasis should generate");
        let plan = &selected.validated.plan;
        assert_eq!(
            plan.anchors
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([PARTY_START, HOSTILE_START, OASIS_OVERLOOK])
        );
        let ordinary = OrdinaryGraph::from_volume(&plan.volume, Some(&plan.blockers));
        let party = plan
            .anchors
            .get(PARTY_START)
            .copied()
            .expect("party_start should remain published");
        let hostile = plan
            .anchors
            .get(HOSTILE_START)
            .copied()
            .expect("hostile_start should remain published");
        let overlook = plan
            .anchors
            .get(OASIS_OVERLOOK)
            .copied()
            .expect("oasis_overlook should remain published");
        let distances = ordinary.distances_from(party);
        assert_eq!(distances.len(), ordinary.len());
        assert!(distances.contains_key(&hostile));
        assert!(distances.contains_key(&overlook));
        assert_eq!(
            selected.metrics.ordinary_surfaces,
            u32::try_from(ordinary.len()).unwrap_or(u32::MAX)
        );
        assert!(selected.metrics.critical_route_steps > 0);
    }

    #[test]
    fn stitched_patch_preserves_pool_and_every_declared_walker_port() {
        let map: MapSettings = ron::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/config/worlds/procedural-ring7.ron"
        )))
        .expect("tracked Ring7 settings should parse");
        let TerrainSettings::Procedural(ProceduralSettings::V3(ring_settings)) = map.terrain else {
            panic!("tracked Ring7 settings should remain V3");
        };
        let layout = resolve_layout(33, &ring_settings).expect("Ring7 layout should resolve");
        let patch = PatchRecipeContext::resolve(&layout, PatchId(4))
            .expect("the dry Fort slot should resolve");
        let recipe = V3OasisSettings {
            base_level: 15,
            pool_radius: 3,
            palm_count: 8,
            grass_ring_width: 2,
        };
        let plan = construct_patch(
            patch,
            &recipe,
            V3EnvironmentSettings::Arid,
            0.4,
            PatchBuildMode::Candidate {
                world_seed: 1_592_598_566,
                candidate: 0,
            },
            runtime_art_catalog(),
        )
        .expect("Oasis should fit a stitched dry patch");
        assert_eq!(
            validate_patch_walker_seams(&patch, &plan.volume),
            Vec::new()
        );
        assert!(plan.validate_against(&layout).is_empty());
        for edge in patch.shared_edges() {
            for port in edge.walker_ports() {
                for coord in port.first_approach {
                    let surface = TilePos::new(coord, edge.preferred_level());
                    assert_eq!(
                        plan.volume
                            .surfaces
                            .get(&surface)
                            .map(|metadata| metadata.access),
                        Some(SurfaceAccess::Ordinary)
                    );
                }
            }
        }
        let metrics = match validate_patch(patch, &recipe, &plan, runtime_art_catalog()) {
            WorldValidation::Valid(metrics) => metrics,
            WorldValidation::Invalid(issues) => {
                panic!("stitched Oasis patch must validate: {issues:?}");
            }
        };
        assert_eq!(metrics.water_cells, exact_disk_size(3));
        assert_eq!(metrics.grass_ring_surfaces, exact_ring_size(3, 2));
        assert_eq!(metrics.palm_roots, 8);
    }

    #[test]
    fn validator_rejects_current_wrong_cap_missing_blocker_and_anchor_drift() {
        let selected = generate(20, 123).expect("Oasis should generate");
        let recipe = V3OasisSettings {
            base_level: 15,
            pool_radius: 5,
            palm_count: 12,
            grass_ring_width: 3,
        };
        let palm = DesertVegetationSet::resolve(runtime_art_catalog(), "Oasis test")
            .expect("palm")
            .date_palm;

        let mut current = selected.validated.plan.clone();
        let node = current
            .liquids
            .bodies
            .get_mut(&POOL_BODY)
            .and_then(|body| body.nodes.values_mut().next())
            .expect("pool node");
        node.state = LiquidFlowState::Current;
        assert_validation_rejects_with(
            validate_oasis(&current, &recipe, &palm, &BTreeSet::new()),
            "Still water",
        );

        let mut wrong_cap = selected.validated.plan.clone();
        let grass_coord = oasis_footprint(&recipe)
            .1
            .into_iter()
            .next()
            .expect("grass coord");
        let cap = wrong_cap
            .volume
            .columns
            .get_mut(&grass_coord)
            .and_then(|column| {
                column
                    .elements
                    .iter_mut()
                    .find_map(|element| match element {
                        VolumeElement::Solid(solid)
                            if solid.material == SolidMaterialRole::Grass =>
                        {
                            Some(solid)
                        }
                        _ => None,
                    })
            })
            .expect("grass cap");
        cap.material = SolidMaterialRole::Sand;
        assert_validation_rejects_with(
            validate_oasis(&wrong_cap, &recipe, &palm, &BTreeSet::new()),
            "exact shared strata",
        );

        let mut missing_blocker = selected.validated.plan.clone();
        missing_blocker.blockers.pop_first();
        assert_validation_rejects_with(
            validate_oasis(&missing_blocker, &recipe, &palm, &BTreeSet::new()),
            "exactly equal projected",
        );

        let mut missing_anchor = selected.validated.plan;
        missing_anchor.anchors.remove(OASIS_OVERLOOK);
        assert_validation_rejects_with(
            validate_oasis(&missing_anchor, &recipe, &palm, &BTreeSet::new()),
            "anchors must be exactly",
        );
    }

    #[test]
    fn forced_candidate_failure_uses_seed_independent_fallback() {
        let settings = settings();
        let layout = resolve_layout(20, &settings).expect("fixture layout should resolve");
        let vegetation =
            DesertVegetationSet::resolve(runtime_art_catalog(), "Oasis test").expect("palm");
        let force = |seed| {
            run_recipe(
                &OasisRecipe {
                    level_height: 0.4,
                    layout: layout.clone(),
                    settings: V3OasisSettings {
                        base_level: 15,
                        pool_radius: 5,
                        palm_count: 12,
                        grass_ring_width: 3,
                    },
                    vegetation: vegetation.clone(),
                    reject_candidates: true,
                },
                &settings,
                20,
                seed,
            )
            .expect("canonical Oasis fallback should validate")
        };
        let first = force(44);
        let other_seed = force(9_999);
        for selected in [&first, &other_seed] {
            assert!(selected.used_fallback);
            assert_eq!(selected.selected_candidate, None);
            assert_eq!(selected.candidates_evaluated, 8);
            assert_eq!(selected.valid_candidates, 0);
            assert_eq!(selected.metrics.palm_roots, 12);
        }
        assert_eq!(
            first.validated.semantic_fingerprint, other_seed.validated.semantic_fingerprint,
            "canonical fallback must not depend on the rejected world seed"
        );
        assert_eq!(first.metrics, other_seed.metrics);
    }

    #[test]
    fn standalone_contract_rejects_wrong_environment_radius_and_oasis_bounds() {
        let mut wrong_environment = settings();
        let V3LayoutSettings::Single(patch) = &mut wrong_environment.layout else {
            unreachable!();
        };
        patch.environment = V3EnvironmentSettings::Coastal;
        assert!(
            super::generate(20, 0.4, &wrong_environment, 0, runtime_art_catalog())
                .expect_err("Oasis should require Arid")
                .to_string()
                .contains("Arid")
        );

        for radius in [11, 41] {
            assert!(generate(radius, 0)
                .expect_err("Oasis should reject an unsupported Single radius")
                .to_string()
                .contains("grid_radius"));
        }

        let invalid_values = [
            (2, 12, 3),
            (7, 12, 3),
            (5, 7, 3),
            (5, 19, 3),
            (5, 12, 1),
            (5, 12, 5),
        ];
        for (pool_radius, palm_count, grass_ring_width) in invalid_values {
            let mut invalid = settings();
            let V3LayoutSettings::Single(patch) = &mut invalid.layout else {
                unreachable!();
            };
            let V3RecipeSettings::Oasis(recipe) = &mut patch.recipe else {
                unreachable!();
            };
            recipe.pool_radius = pool_radius;
            recipe.palm_count = palm_count;
            recipe.grass_ring_width = grass_ring_width;
            assert!(super::generate(20, 0.4, &invalid, 0, runtime_art_catalog()).is_err());
        }
    }

    fn assert_exact_oasis(plan: &GeneratedWorldPlan, settings: &V3OasisSettings) {
        let vegetation =
            DesertVegetationSet::resolve(runtime_art_catalog(), "Oasis test").expect("palm");
        match validate_oasis(plan, settings, &vegetation.date_palm, &BTreeSet::new()) {
            WorldValidation::Valid(_) => {}
            WorldValidation::Invalid(issues) => panic!("exact Oasis must validate: {issues:?}"),
        }
    }

    const fn exact_disk_size(radius: u32) -> u32 {
        1_u32.saturating_add(
            3_u32
                .saturating_mul(radius)
                .saturating_mul(radius.saturating_add(1)),
        )
    }

    const fn exact_ring_size(pool_radius: u32, width: u32) -> u32 {
        exact_disk_size(pool_radius.saturating_add(width))
            .saturating_sub(exact_disk_size(pool_radius))
    }

    fn assert_validation_rejects_with(validation: WorldValidation<OasisMetrics>, expected: &str) {
        let WorldValidation::Invalid(issues) = validation else {
            panic!("corrupted Oasis unexpectedly validated");
        };
        assert!(
            issues.iter().any(|issue| issue.detail.contains(expected)),
            "expected issue containing {expected:?}, got {issues:?}"
        );
    }
}
