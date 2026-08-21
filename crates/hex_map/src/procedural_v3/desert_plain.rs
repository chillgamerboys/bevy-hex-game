//! Pure semantic Desert Plain recipe for procedural generator V3.
//!
//! Desert Plain owns a connected, low-relief sand landform and nothing else.
//! It remains patch-ready by fitting the shared walker-seam authority before
//! voxelization, while the standalone wrapper retains the established V3 Single
//! radius contract.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use hex_core::{HexCoord, TilePos};

use super::arid_landform::sand_column;
use super::composition::{compose_single_patch, GeneratedPatchPlan};
use super::layout::{resolve_layout, PatchId, ResolvedLayoutPlan};
use super::patch::{PatchBuildMode, PatchRecipeContext};
use super::seam::{shape_walker_seams, validate_patch_walker_seams};
use super::selection::{
    run_recipe, CandidateAttemptError, CandidateContext, FallbackContext, RepairOutcome, V3Recipe,
    ValidatedWorldSelection, WorldValidation,
};
use super::traversal::OrdinaryGraph;
use super::vegetation_landform::{actor_anchors, rolling_levels, view_hint};
use super::volume::{
    LevelInterval, SolidMaterialRole, SurfaceAccess, SurfaceMetadata, VolumeElement, VolumePlan,
};
use super::world::{
    FeaturePlan, GeneratedWorldPlan, InteriorPlan, StructurePlan, WorldIssueCode,
    WorldValidationIssue,
};
use super::V3GenerationError;
use crate::procedural::DesertPlainMetrics;
use crate::settings::{
    ProceduralV3Settings, V3DesertPlainSettings, V3EnvironmentSettings, V3LayoutSettings,
    V3RecipeSettings, MAX_V3_LEVEL,
};

const PARTY_START: &str = "party_start";
const HOSTILE_START: &str = "hostile_start";
const DESERT_PLAIN_OVERLOOK: &str = "desert_plain_overlook";
const MIN_SINGLE_RADIUS: u32 = 12;
const MAX_SINGLE_RADIUS: u32 = 40;
const MIN_CONNECTED_COLUMNS: usize = 127;

#[derive(Debug)]
struct DesertPlainRecipe {
    level_height: f32,
    layout: ResolvedLayoutPlan,
    settings: V3DesertPlainSettings,
    #[cfg(test)]
    reject_candidates: bool,
}

/// Runs the common eight-candidate V3 selector for one standalone Desert Plain.
pub(crate) fn generate(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV3Settings,
    seed: u64,
) -> Result<ValidatedWorldSelection<DesertPlainMetrics>, V3GenerationError> {
    if !level_height.is_finite() || level_height <= 0.0 {
        return Err(V3GenerationError::RecipeContract(
            "DesertPlain level height must be positive and finite".to_owned(),
        ));
    }
    let recipe = *validate_recipe_settings(settings, grid_radius)?;
    let layout = resolve_layout(grid_radius, settings)
        .map_err(|error| V3GenerationError::RecipeContract(error.to_string()))?;
    if layout.footprint.len() < MIN_CONNECTED_COLUMNS {
        return Err(V3GenerationError::RecipeContract(format!(
            "DesertPlain requires at least {MIN_CONNECTED_COLUMNS} connected columns"
        )));
    }
    run_recipe(
        &DesertPlainRecipe {
            level_height,
            layout,
            settings: recipe,
            #[cfg(test)]
            reject_candidates: false,
        },
        settings,
        grid_radius,
        seed,
    )
}

impl V3Recipe for DesertPlainRecipe {
    type Settings = ProceduralV3Settings;
    type Metrics = DesertPlainMetrics;
    type Score = (u32, u32, u8);

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
                    "DesertPlain candidate radius disagrees with its resolved layout".to_owned(),
                ),
            ));
        }
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
                "DesertPlain single-patch composition failed: {error:?}"
            )))
        })
    }

    fn validate(
        &self,
        _settings: &Self::Settings,
        plan: &GeneratedWorldPlan,
    ) -> WorldValidation<Self::Metrics> {
        validate_desert_plain(plan, &self.settings)
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
            metrics.relief.abs_diff(self.settings.max_relief),
            metrics.critical_route_steps,
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
                "DesertPlain fallback radius disagrees with its resolved layout".to_owned(),
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
                "DesertPlain fallback composition failed: {error:?}"
            ))
        })
    }
}

fn validate_recipe_settings(
    settings: &ProceduralV3Settings,
    grid_radius: u32,
) -> Result<&V3DesertPlainSettings, V3GenerationError> {
    let V3LayoutSettings::Single(patch) = &settings.layout else {
        return Err(V3GenerationError::RecipeUnavailable("DesertPlain Single"));
    };
    let V3RecipeSettings::DesertPlain(recipe) = &patch.recipe else {
        return Err(V3GenerationError::RecipeUnavailable("DesertPlain"));
    };
    if patch.environment != V3EnvironmentSettings::Arid {
        return Err(V3GenerationError::RecipeContract(
            "DesertPlain requires the Arid environment".to_owned(),
        ));
    }
    if !patch.overlays.is_empty() {
        return Err(V3GenerationError::RecipeContract(
            "DesertPlain overlays are not implemented yet".to_owned(),
        ));
    }
    if !(MIN_SINGLE_RADIUS..=MAX_SINGLE_RADIUS).contains(&grid_radius) {
        return Err(V3GenerationError::RecipeContract(format!(
            "procedural V3 DesertPlain requires grid_radius from {MIN_SINGLE_RADIUS} through \
             {MAX_SINGLE_RADIUS}"
        )));
    }
    if recipe.base_level < 5 {
        return Err(V3GenerationError::RecipeContract(
            "V3 DesertPlain base_level must leave room for bedrock and sand".to_owned(),
        ));
    }
    if !(1..=4).contains(&recipe.max_relief) {
        return Err(V3GenerationError::RecipeContract(
            "V3 DesertPlain max_relief must be between 1 and 4".to_owned(),
        ));
    }
    let highest = recipe
        .base_level
        .checked_add(recipe.max_relief)
        .ok_or_else(|| {
            V3GenerationError::RecipeContract(
                "V3 DesertPlain level relationship overflows Level".to_owned(),
            )
        })?;
    if highest > MAX_V3_LEVEL {
        return Err(V3GenerationError::RecipeContract(format!(
            "V3 DesertPlain surfaces cannot exceed level {MAX_V3_LEVEL}"
        )));
    }
    Ok(recipe)
}

/// Constructs one patch-ready Desert Plain fragment.
pub(crate) fn construct_patch(
    patch: PatchRecipeContext<'_>,
    settings: &V3DesertPlainSettings,
    environment: V3EnvironmentSettings,
    level_height: f32,
    mode: PatchBuildMode,
) -> Result<GeneratedPatchPlan, Vec<WorldValidationIssue>> {
    if environment != V3EnvironmentSettings::Arid {
        return Err(vec![recipe_issue(
            "DesertPlain requires the Arid environment",
        )]);
    }
    if !level_height.is_finite() || level_height <= 0.0 {
        return Err(vec![recipe_issue(
            "DesertPlain level height must be positive and finite",
        )]);
    }
    if settings.base_level < 5
        || !(1..=4).contains(&settings.max_relief)
        || settings
            .base_level
            .checked_add(settings.max_relief)
            .is_none_or(|highest| highest > MAX_V3_LEVEL)
    {
        return Err(vec![recipe_issue(
            "DesertPlain patch settings violate the validated low-relief range",
        )]);
    }

    let frame = patch
        .local_frame()
        .map_err(|error| vec![recipe_issue(error.to_string())])?;
    let mask = frame
        .local_mask(patch.mask())
        .map_err(|error| vec![recipe_issue(error)])?;
    let landform = mode
        .seed_streams(&patch)
        .map(|streams| streams.stage("desert_plain.landform"));
    let local_levels = rolling_levels(
        &mask,
        settings.base_level,
        settings.max_relief,
        landform,
        "desert_plain",
    )?;
    let mut world_levels = frame
        .levels_to_world(local_levels)
        .map_err(|error| vec![recipe_issue(error)])?;
    let seam_shape = shape_walker_seams(&patch, &mut world_levels)?;
    let local_levels = frame
        .levels_to_local(world_levels)
        .map_err(|error| vec![recipe_issue(error)])?;

    let mut columns = BTreeMap::new();
    let mut surfaces = BTreeMap::new();
    let mut ordinary_by_coord = BTreeMap::new();
    for coord in &mask {
        let level = local_levels.get(coord).copied().ok_or_else(|| {
            vec![recipe_issue(format!(
                "DesertPlain land plan omitted coordinate {coord:?}"
            ))]
        })?;
        let position = TilePos::new(*coord, level);
        let world_position = frame
            .position_to_world(position)
            .map_err(|error| vec![recipe_issue(error)])?;
        let access = seam_shape.access_for(world_position, SurfaceAccess::Ordinary);
        columns.insert(*coord, sand_column(level));
        surfaces.insert(
            position,
            SurfaceMetadata {
                access,
                interior: None,
            },
        );
        if access == SurfaceAccess::Ordinary {
            ordinary_by_coord.insert(*coord, position);
        }
    }

    let (party_start, hostile_start) = actor_anchors(&ordinary_by_coord, "desert_plain")?;
    let overlook = select_overlook(&ordinary_by_coord)
        .ok_or_else(|| vec![recipe_issue("DesertPlain has no ordinary review surface")])?;
    let volume = VolumePlan {
        mask: mask.clone(),
        columns,
        surfaces,
    };
    let anchors = BTreeMap::from([
        (PARTY_START.to_owned(), party_start),
        (HOSTILE_START.to_owned(), hostile_start),
        (DESERT_PLAIN_OVERLOOK.to_owned(), overlook),
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
            "desert_plain",
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

/// Validates one patch-ready Desert Plain fragment in its canonical local frame.
pub(crate) fn validate_patch(
    patch: PatchRecipeContext<'_>,
    settings: &V3DesertPlainSettings,
    plan: &GeneratedPatchPlan,
) -> WorldValidation<DesertPlainMetrics> {
    let frame = match patch.local_frame() {
        Ok(frame) => frame,
        Err(error) => {
            return WorldValidation::Invalid(vec![recipe_issue(format!(
                "DesertPlain validation frame failed: {error}"
            ))]);
        }
    };
    let mut world = match frame.canonical_local_world(plan) {
        Ok(world) => world,
        Err(error) => {
            return WorldValidation::Invalid(vec![recipe_issue(format!(
                "DesertPlain validation projection failed: {error}"
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
    validate_desert_plain(&world, settings)
}

fn validate_desert_plain(
    plan: &GeneratedWorldPlan,
    settings: &V3DesertPlainSettings,
) -> WorldValidation<DesertPlainMetrics> {
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
            "DesertPlain must not contain liquids, features, structures, blockers, lights, or interiors",
        ));
    }

    let expected_anchor_names = BTreeSet::from([
        PARTY_START.to_owned(),
        HOSTILE_START.to_owned(),
        DESERT_PLAIN_OVERLOOK.to_owned(),
    ]);
    let actual_anchor_names = plan.anchors.keys().cloned().collect::<BTreeSet<_>>();
    if actual_anchor_names != expected_anchor_names {
        issues.push(recipe_issue(format!(
            "DesertPlain anchors must be exactly party_start, hostile_start, and \
             desert_plain_overlook; got {actual_anchor_names:?}"
        )));
    }

    let ordinary = OrdinaryGraph::from_volume(&plan.volume, None);
    let ordinary_by_coord = ordinary
        .positions()
        .map(|position| (position.coord, position))
        .collect::<BTreeMap<_, _>>();
    let expected_actors = actor_anchors(&ordinary_by_coord, "desert_plain");
    let party = plan.anchors.get(PARTY_START).copied();
    let hostile = plan.anchors.get(HOSTILE_START).copied();
    let overlook = plan.anchors.get(DESERT_PLAIN_OVERLOOK).copied();
    let mut critical_route_steps = 0;
    match expected_actors {
        Ok((expected_party, expected_hostile)) => {
            if party != Some(expected_party) || hostile != Some(expected_hostile) {
                issues.push(recipe_issue(format!(
                    "DesertPlain actor anchors drifted from their deterministic landings: \
                     party {party:?}/{expected_party:?}, hostile {hostile:?}/{expected_hostile:?}"
                )));
            }
        }
        Err(anchor_issues) => issues.extend(anchor_issues),
    }
    let expected_overlook = select_overlook(&ordinary_by_coord);
    if overlook != expected_overlook {
        issues.push(recipe_issue(format!(
            "DesertPlain overlook drifted from its deterministic highest central landing: \
             {overlook:?}/{expected_overlook:?}"
        )));
    }
    match party {
        Some(party) => {
            let distances = ordinary.distances_from(party);
            if distances.len() != ordinary.len() {
                issues.push(recipe_issue(format!(
                    "DesertPlain ordinary terrain is disconnected: {}/{} reachable",
                    distances.len(),
                    ordinary.len()
                )));
            }
            if let Some(hostile) = hostile {
                critical_route_steps = distances.get(&hostile).copied().unwrap_or_default();
                if !distances.contains_key(&hostile) {
                    issues.push(recipe_issue(
                        "DesertPlain actor anchors are not ordinarily connected",
                    ));
                }
            }
            if overlook.is_some_and(|overlook| !distances.contains_key(&overlook)) {
                issues.push(recipe_issue(
                    "DesertPlain overlook is not ordinarily connected to party_start",
                ));
            }
        }
        None => issues.push(recipe_issue("DesertPlain requires party_start")),
    }

    let surfaces_by_coord = plan
        .volume
        .surfaces
        .keys()
        .map(|surface| (surface.coord, *surface))
        .collect::<BTreeMap<_, _>>();
    for (coord, column) in &plan.volume.columns {
        let Some(surface) = surfaces_by_coord.get(coord).copied() else {
            issues.push(recipe_issue(format!(
                "DesertPlain column {coord:?} has no exact exposed sand surface"
            )));
            continue;
        };
        let [VolumeElement::Solid(bedrock), VolumeElement::Solid(stone), VolumeElement::Solid(dirt), VolumeElement::Solid(sand)] =
            column.elements.as_slice()
        else {
            issues.push(recipe_issue(format!(
                "DesertPlain column {coord:?} must contain the exact dry strata and one sand cap"
            )));
            continue;
        };
        if bedrock.levels != LevelInterval::new(0, 1)
            || bedrock.material != SolidMaterialRole::Bedrock
            || bedrock.cutaway_for.is_some()
        {
            issues.push(recipe_issue(format!(
                "DesertPlain column {coord:?} has an invalid bedrock foundation"
            )));
        }
        if stone.levels != LevelInterval::new(1, surface.level.saturating_sub(3))
            || stone.material != SolidMaterialRole::Stone
            || stone.cutaway_for.is_some()
            || dirt.levels != LevelInterval::new(surface.level.saturating_sub(3), surface.level)
            || dirt.material != SolidMaterialRole::Dirt
            || dirt.cutaway_for.is_some()
        {
            issues.push(recipe_issue(format!(
                "DesertPlain column {coord:?} does not use the shared supported dry strata"
            )));
        }
        if sand.levels != LevelInterval::new(surface.level, surface.level.saturating_add(1))
            || sand.material != SolidMaterialRole::Sand
            || sand.cutaway_for.is_some()
        {
            issues.push(recipe_issue(format!(
                "DesertPlain column {coord:?} does not expose an exact all-sand surface cap at {surface:?}"
            )));
        }
    }
    if surfaces_by_coord.len() != plan.volume.mask.len() {
        issues.push(recipe_issue(format!(
            "DesertPlain requires one sand surface in every owned column: {}/{}",
            surfaces_by_coord.len(),
            plan.volume.mask.len()
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
    let highest_allowed = settings
        .base_level
        .checked_add(settings.max_relief)
        .unwrap_or(MAX_V3_LEVEL);
    if levels
        .first()
        .is_none_or(|lowest| *lowest < settings.base_level)
        || levels
            .last()
            .is_none_or(|highest| *highest > highest_allowed)
        || relief > settings.max_relief
    {
        issues.push(recipe_issue(format!(
            "DesertPlain ordinary levels must remain in {}..={} with relief at most {}; got \
             {levels:?}",
            settings.base_level, highest_allowed, settings.max_relief
        )));
    }

    if !issues.is_empty() {
        return WorldValidation::Invalid(issues);
    }
    WorldValidation::Valid(DesertPlainMetrics {
        sand_surfaces: count_u32(surfaces_by_coord.len()),
        ordinary_surfaces: count_u32(ordinary.len()),
        sand_surface_percent: count_u32(surfaces_by_coord.len())
            .saturating_mul(100)
            .checked_div(count_u32(plan.volume.mask.len()))
            .unwrap_or_default(),
        reachable_elevation_levels: count_u32(levels.len()),
        relief,
        critical_route_steps,
    })
}

fn select_overlook(ordinary_by_coord: &BTreeMap<HexCoord, TilePos>) -> Option<TilePos> {
    ordinary_by_coord.values().copied().min_by_key(|position| {
        (
            Reverse(position.level),
            position.coord.distance(HexCoord::ORIGIN),
            *position,
        )
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
    WorldValidationIssue::new(WorldIssueCode::Recipe("desert_plain"), detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{
        MapSettings, PatchEdgeContractSettings, PatchEdgesSettings, PatchMaskSettings, PatchSpec,
        ProceduralSettings, TerrainSettings,
    };

    fn settings() -> ProceduralV3Settings {
        ProceduralV3Settings {
            layout: V3LayoutSettings::Single(PatchSpec {
                environment: V3EnvironmentSettings::Arid,
                recipe: V3RecipeSettings::DesertPlain(V3DesertPlainSettings {
                    base_level: 12,
                    max_relief: 3,
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

    fn generate(
        radius: u32,
        seed: u64,
    ) -> Result<ValidatedWorldSelection<DesertPlainMetrics>, V3GenerationError> {
        super::generate(radius, 0.4, &settings(), seed)
    }

    fn assert_all_sand(plan: &GeneratedWorldPlan) {
        assert_eq!(plan.volume.columns.len(), plan.volume.mask.len());
        for (coord, column) in &plan.volume.columns {
            let surface = plan
                .volume
                .surfaces
                .keys()
                .find(|surface| surface.coord == *coord)
                .copied()
                .expect("every desert column should expose one surface");
            let [VolumeElement::Solid(bedrock), VolumeElement::Solid(stone), VolumeElement::Solid(dirt), VolumeElement::Solid(sand)] =
                column.elements.as_slice()
            else {
                panic!("every desert column should use the exact dry strata and sand cap");
            };
            assert_eq!(bedrock.levels, LevelInterval::new(0, 1));
            assert_eq!(bedrock.material, SolidMaterialRole::Bedrock);
            assert_eq!(
                stone.levels,
                LevelInterval::new(1, surface.level.saturating_sub(3))
            );
            assert_eq!(stone.material, SolidMaterialRole::Stone);
            assert_eq!(
                dirt.levels,
                LevelInterval::new(surface.level.saturating_sub(3), surface.level)
            );
            assert_eq!(dirt.material, SolidMaterialRole::Dirt);
            assert_eq!(
                sand.levels,
                LevelInterval::new(surface.level, surface.level + 1)
            );
            assert_eq!(sand.material, SolidMaterialRole::Sand);
        }
    }

    #[test]
    fn fixed_corpus_builds_deterministic_all_sand_plains() {
        for radius in [12, 20, 40] {
            for seed in [0, 1, 1_592_598_566, 4_294_967_311] {
                let first = generate(radius, seed).expect("DesertPlain should generate");
                let repeated = generate(radius, seed).expect("DesertPlain should repeat");
                assert_eq!(
                    first.validated.semantic_fingerprint,
                    repeated.validated.semantic_fingerprint
                );
                assert!(!first.used_fallback);
                assert_eq!(first.candidates_evaluated, 8);
                assert_eq!(first.valid_candidates, 8);
                assert!(first.notes.is_empty());
                assert_eq!(first.metrics.relief, 3);
                assert_eq!(
                    first.metrics.sand_surfaces,
                    u32::try_from(first.validated.plan.volume.mask.len()).unwrap_or(u32::MAX)
                );
                assert_eq!(first.metrics.sand_surface_percent, 100);
                assert!(first.validated.plan.features.by_id.is_empty());
                assert!(first.validated.plan.structures.by_id.is_empty());
                assert!(first.validated.plan.blockers.is_empty());
                assert_all_sand(&first.validated.plan);
            }
        }
    }

    #[test]
    fn deterministic_anchors_are_exact_and_ordinarily_connected() {
        let selected = generate(20, 1_592_598_566).expect("DesertPlain should generate");
        let plan = &selected.validated.plan;
        assert_eq!(
            plan.anchors
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([PARTY_START, HOSTILE_START, DESERT_PLAIN_OVERLOOK])
        );
        let ordinary = OrdinaryGraph::from_volume(&plan.volume, None);
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
            .get(DESERT_PLAIN_OVERLOOK)
            .copied()
            .expect("desert_plain_overlook should remain published");
        let distances = ordinary.distances_from(party);
        assert_eq!(distances.len(), ordinary.len());
        assert!(distances.contains_key(&hostile));
        assert!(distances.contains_key(&overlook));
        assert!(selected.metrics.critical_route_steps > 0);
        assert_eq!(
            selected.metrics.ordinary_surfaces,
            u32::try_from(ordinary.len()).unwrap_or(u32::MAX)
        );
    }

    #[test]
    fn stitched_patch_shapes_and_validates_every_declared_walker_port() {
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
        let recipe = V3DesertPlainSettings {
            base_level: 15,
            max_relief: 2,
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
        )
        .expect("DesertPlain should fit a stitched dry patch");
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
        let metrics = match validate_patch(patch, &recipe, &plan) {
            WorldValidation::Valid(metrics) => metrics,
            WorldValidation::Invalid(issues) => {
                panic!("stitched DesertPlain patch must validate: {issues:?}");
            }
        };
        assert!(metrics.relief <= recipe.max_relief);
        assert_all_sand(
            &patch
                .local_frame()
                .expect("frame")
                .canonical_local_world(&plan)
                .expect("local world"),
        );
    }

    #[test]
    fn validator_rejects_non_sand_material_and_anchor_drift() {
        let selected = generate(12, 77).expect("DesertPlain should generate");
        let mut wrong_material = selected.validated.plan.clone();
        let column = wrong_material
            .volume
            .columns
            .values_mut()
            .next()
            .expect("one desert column");
        let sand = column
            .elements
            .iter_mut()
            .find_map(|element| match element {
                VolumeElement::Solid(solid) if solid.material == SolidMaterialRole::Sand => {
                    Some(solid)
                }
                _ => None,
            })
            .expect("one sand mass");
        sand.material = SolidMaterialRole::Dirt;
        assert_validation_rejects_with(
            validate_desert_plain(
                &wrong_material,
                &V3DesertPlainSettings {
                    base_level: 12,
                    max_relief: 3,
                },
            ),
            "all-sand surface cap",
        );

        let mut wrong_anchor = selected.validated.plan;
        wrong_anchor.anchors.remove(DESERT_PLAIN_OVERLOOK);
        assert_validation_rejects_with(
            validate_desert_plain(
                &wrong_anchor,
                &V3DesertPlainSettings {
                    base_level: 12,
                    max_relief: 3,
                },
            ),
            "anchors must be exactly",
        );
    }

    #[test]
    fn forced_candidate_failure_uses_seed_independent_fallback() {
        let settings = settings();
        let layout = resolve_layout(12, &settings).expect("fixture layout should resolve");
        let force = |seed| {
            run_recipe(
                &DesertPlainRecipe {
                    level_height: 0.4,
                    layout: layout.clone(),
                    settings: V3DesertPlainSettings {
                        base_level: 12,
                        max_relief: 3,
                    },
                    reject_candidates: true,
                },
                &settings,
                12,
                seed,
            )
            .expect("canonical DesertPlain fallback should validate")
        };
        let first = force(44);
        let other_seed = force(9_999);
        for selected in [&first, &other_seed] {
            assert!(selected.used_fallback);
            assert_eq!(selected.selected_candidate, None);
            assert_eq!(selected.candidates_evaluated, 8);
            assert_eq!(selected.valid_candidates, 0);
            assert_all_sand(&selected.validated.plan);
        }
        assert_eq!(
            first.validated.semantic_fingerprint, other_seed.validated.semantic_fingerprint,
            "canonical fallback must not depend on the rejected world seed"
        );
        assert_eq!(first.metrics, other_seed.metrics);
    }

    #[test]
    fn standalone_contract_rejects_wrong_environment_radius_and_relief() {
        let mut wrong_environment = settings();
        let V3LayoutSettings::Single(patch) = &mut wrong_environment.layout else {
            unreachable!();
        };
        patch.environment = V3EnvironmentSettings::Coastal;
        assert!(super::generate(12, 0.4, &wrong_environment, 0)
            .expect_err("DesertPlain should require Arid")
            .to_string()
            .contains("Arid"));

        for radius in [11, 41] {
            assert!(generate(radius, 0)
                .expect_err("DesertPlain should reject an unsupported Single radius")
                .to_string()
                .contains("grid_radius"));
        }

        for relief in [0, 5] {
            let mut invalid = settings();
            let V3LayoutSettings::Single(patch) = &mut invalid.layout else {
                unreachable!();
            };
            let V3RecipeSettings::DesertPlain(recipe) = &mut patch.recipe else {
                unreachable!();
            };
            recipe.max_relief = relief;
            assert!(super::generate(12, 0.4, &invalid, 0)
                .expect_err("DesertPlain should reject relief outside 1..=4")
                .to_string()
                .contains("max_relief"));
        }
    }

    fn assert_validation_rejects_with(
        validation: WorldValidation<DesertPlainMetrics>,
        expected: &str,
    ) {
        let WorldValidation::Invalid(issues) = validation else {
            panic!("corrupted DesertPlain unexpectedly validated");
        };
        assert!(
            issues.iter().any(|issue| issue.detail.contains(expected)),
            "expected issue containing {expected:?}, got {issues:?}"
        );
    }
}
