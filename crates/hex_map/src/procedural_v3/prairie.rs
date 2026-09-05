//! Pure semantic Prairie recipe for procedural generator V3.
//!
//! Prairie shares the accepted authored grass object with Forest, but owns its
//! rolling landform, feature density, anchors, and validation. It deliberately
//! publishes no road or tree authority.

use std::collections::{BTreeMap, BTreeSet};

use hex_assets::{HexObjectRotation, RuntimeArtCatalog};
use hex_core::{HexCoord, TilePos};
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
use super::vegetation::{GrassVegetationSpec, VegetationObjectSpec, GRASS_TUFT_ID};
use super::vegetation_landform::{actor_anchors, grassland_column, rolling_levels, view_hint};
use super::volume::{SurfaceAccess, SurfaceMetadata, VolumePlan};
use super::world::{
    FeatureId, FeatureKind, FeaturePlan, GeneratedWorldPlan, InteriorPlan, PlannedFeature,
    StructurePlan, WorldIssueCode, WorldValidationIssue,
};
use super::V3GenerationError;
use crate::procedural::PrairieMetrics;
use crate::settings::{
    ProceduralV3Settings, V3EnvironmentSettings, V3LayoutSettings, V3PrairieSettings,
    V3RecipeSettings,
};

const PARTY_START: &str = "party_start";
const HOSTILE_START: &str = "hostile_start";
const PRAIRIE_OVERLOOK: &str = "prairie_overlook";

#[derive(Debug)]
struct PrairieRecipe {
    level_height: f32,
    layout: ResolvedLayoutPlan,
    objects: GrassVegetationSpec,
    #[cfg(test)]
    reject_candidates: bool,
}

#[derive(Debug, Clone, Copy)]
struct PrairieStreams<'a> {
    landform: SeedStream<'a>,
    grass: SeedStream<'a>,
    rotations: SeedStream<'a>,
}

/// Runs the common eight-candidate V3 selector for one Prairie world.
pub(crate) fn generate(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV3Settings,
    seed: u64,
    catalog: &RuntimeArtCatalog,
) -> Result<ValidatedWorldSelection<PrairieMetrics>, V3GenerationError> {
    if !level_height.is_finite() || level_height <= 0.0 {
        return Err(V3GenerationError::RecipeContract(
            "Prairie level height must be positive and finite".to_owned(),
        ));
    }
    validate_recipe_settings(settings, grid_radius)?;
    let layout = resolve_layout(grid_radius, settings)
        .map_err(|error| V3GenerationError::RecipeContract(error.to_string()))?;
    if layout.footprint.len() < 127 {
        return Err(V3GenerationError::RecipeContract(
            "Prairie requires at least 127 connected columns".to_owned(),
        ));
    }
    let objects = GrassVegetationSpec::resolve(catalog, "Prairie")
        .map_err(V3GenerationError::RecipeContract)?;
    run_recipe(
        &PrairieRecipe {
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

impl V3Recipe for PrairieRecipe {
    type Settings = ProceduralV3Settings;
    type Metrics = PrairieMetrics;
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
                    "Prairie candidate radius disagrees with its resolved layout".to_owned(),
                ),
            ));
        }
        let patch = PatchRecipeContext::resolve(&self.layout, PatchId(0))
            .map_err(CandidateAttemptError::Fatal)?;
        let fragment = construct_patch_with_objects(
            patch,
            recipe_settings,
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
        settings: &Self::Settings,
        plan: &GeneratedWorldPlan,
    ) -> WorldValidation<Self::Metrics> {
        let Ok(recipe_settings) = validate_recipe_settings(settings, plan.layout.grid_radius)
        else {
            return WorldValidation::Invalid(vec![recipe_issue(
                "Prairie settings changed after construction",
            )]);
        };
        validate_prairie(
            plan,
            recipe_settings,
            &self.objects.grass_tuft,
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
        settings: &Self::Settings,
        metrics: &Self::Metrics,
        candidate: u8,
    ) -> Self::Score {
        let target = match &settings.layout {
            V3LayoutSettings::Single(patch) => match &patch.recipe {
                V3RecipeSettings::Prairie(settings) => u32::from(settings.grass_coverage_percent),
                _ => 0,
            },
            V3LayoutSettings::Ring7(_)
            | V3LayoutSettings::Ring19(_)
            | V3LayoutSettings::Macro(_)
            | V3LayoutSettings::Schematic(_) => 0,
        };
        (
            metrics.grass_coverage_percent.abs_diff(target),
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
                "Prairie fallback radius disagrees with its resolved layout".to_owned(),
            ));
        }
        let patch = PatchRecipeContext::resolve(&self.layout, PatchId(0))?;
        let fragment = construct_patch_with_objects(
            patch,
            recipe_settings,
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

fn target_relief(settings: &ProceduralV3Settings) -> i32 {
    match &settings.layout {
        V3LayoutSettings::Single(patch) => match &patch.recipe {
            V3RecipeSettings::Prairie(settings) => settings.max_relief,
            _ => 0,
        },
        V3LayoutSettings::Ring7(_)
        | V3LayoutSettings::Ring19(_)
        | V3LayoutSettings::Macro(_)
        | V3LayoutSettings::Schematic(_) => 0,
    }
}

fn validate_recipe_settings(
    settings: &ProceduralV3Settings,
    grid_radius: u32,
) -> Result<&V3PrairieSettings, V3GenerationError> {
    let V3LayoutSettings::Single(patch) = &settings.layout else {
        return Err(V3GenerationError::RecipeUnavailable("Ring19"));
    };
    if patch.environment != V3EnvironmentSettings::TemperateGrassland {
        return Err(V3GenerationError::RecipeContract(
            "Prairie requires the TemperateGrassland environment".to_owned(),
        ));
    }
    let V3RecipeSettings::Prairie(recipe) = &patch.recipe else {
        return Err(V3GenerationError::RecipeUnavailable(recipe_name(
            &patch.recipe,
        )));
    };
    if !patch.overlays.is_empty() {
        return Err(V3GenerationError::RecipeContract(
            "Prairie overlays are not implemented yet".to_owned(),
        ));
    }
    if !(12..=40).contains(&grid_radius)
        || recipe.base_level < 5
        || !(1..=12).contains(&recipe.max_relief)
        || !(65..=75).contains(&recipe.grass_coverage_percent)
        || recipe
            .base_level
            .checked_add(recipe.max_relief)
            .is_none_or(|highest| highest > 96)
    {
        return Err(V3GenerationError::RecipeContract(
            "Prairie settings violate the validated V3 vegetation-landform range".to_owned(),
        ));
    }
    Ok(recipe)
}

pub(crate) fn construct_patch(
    patch: PatchRecipeContext<'_>,
    settings: &V3PrairieSettings,
    environment: V3EnvironmentSettings,
    level_height: f32,
    mode: PatchBuildMode,
    catalog: &RuntimeArtCatalog,
) -> Result<GeneratedPatchPlan, Vec<WorldValidationIssue>> {
    let objects = GrassVegetationSpec::resolve(catalog, "Prairie")
        .map_err(|error| vec![recipe_issue(error)])?;
    construct_patch_with_objects(patch, settings, environment, level_height, mode, &objects)
}

fn construct_patch_with_objects(
    patch: PatchRecipeContext<'_>,
    settings: &V3PrairieSettings,
    environment: V3EnvironmentSettings,
    level_height: f32,
    mode: PatchBuildMode,
    objects: &GrassVegetationSpec,
) -> Result<GeneratedPatchPlan, Vec<WorldValidationIssue>> {
    if environment != V3EnvironmentSettings::TemperateGrassland {
        return Err(vec![recipe_issue(
            "Prairie requires the TemperateGrassland environment",
        )]);
    }
    let frame = patch
        .local_frame()
        .map_err(|error| vec![recipe_issue(error.to_string())])?;
    let mask = frame
        .local_mask(patch.mask())
        .map_err(|error| vec![recipe_issue(error)])?;
    let streams = mode.seed_streams(&patch).map(|streams| PrairieStreams {
        landform: streams.stage("prairie.landform"),
        grass: streams.stage("prairie.grass"),
        rotations: streams.stage("prairie.rotations"),
    });
    let local_levels = rolling_levels(
        &mask,
        settings.base_level,
        settings.max_relief,
        streams.map(|streams| streams.landform),
        "prairie",
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
                "Prairie land plan omitted coordinate {coord:?}"
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
    let (party_start, hostile_start) = actor_anchors(&ordinary_by_coord, "prairie")?;
    let prairie_overlook = ordinary_by_coord
        .values()
        .copied()
        .min_by_key(|position| {
            (
                position.coord.distance(HexCoord::ORIGIN),
                std::cmp::Reverse(position.level),
                *position,
            )
        })
        .ok_or_else(|| vec![recipe_issue("Prairie has no review surface")])?;

    let protected_approaches = patch
        .protected_approaches()
        .into_iter()
        .map(|coord| frame.to_local(coord).map_err(recipe_issue))
        .collect::<Result<BTreeSet<_>, _>>()
        .map_err(|issue| vec![issue])?;
    let eligible = eligible_grass_surfaces(
        &ordinary_by_coord,
        [party_start, hostile_start, prairie_overlook],
        &protected_approaches,
    );
    let grass_target = eligible
        .len()
        .saturating_mul(usize::from(settings.grass_coverage_percent))
        / 100;
    let mut ranked = eligible.iter().copied().collect::<Vec<_>>();
    ranked.sort_unstable_by_key(|root| {
        (
            feature_priority(streams.map(|streams| streams.grass), root.coord, 0),
            *root,
        )
    });
    let mut by_id = BTreeMap::new();
    for (index, root) in ranked.into_iter().take(grass_target).enumerate() {
        let rotation = object_rotation(streams.map(|streams| streams.rotations), root.coord, 17)?;
        let volume = objects
            .grass_tuft
            .project_visual_volume(root, rotation)
            .ok_or_else(|| {
                vec![recipe_issue(format!(
                    "Prairie grass cannot project its complete authored bounds at {root:?}"
                ))]
            })?;
        if volume.cells.iter().any(|visual| {
            surface_by_coord
                .get(&visual.coord)
                .is_none_or(|support| visual.level <= support.level)
        }) {
            return Err(vec![recipe_issue(format!(
                "Prairie grass intersects or leaves terrain at {root:?}"
            ))]);
        }
        by_id.insert(
            FeatureId(u32::try_from(index).unwrap_or(u32::MAX)),
            PlannedFeature {
                root,
                kind: FeatureKind::TallGrass,
                object_id: objects.grass_tuft.id.clone(),
                rotation,
                blocker_footprint: BTreeSet::new(),
            },
        );
    }

    let columns = surface_by_coord
        .iter()
        .map(|(coord, position)| (*coord, grassland_column(position.level, false)))
        .collect();
    let volume = VolumePlan {
        mask: mask.clone(),
        columns,
        surfaces,
    };
    let anchors = BTreeMap::from([
        (PARTY_START.to_owned(), party_start),
        (HOSTILE_START.to_owned(), hostile_start),
        (PRAIRIE_OVERLOOK.to_owned(), prairie_overlook),
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
        features: FeaturePlan {
            by_id,
            protected_routes: BTreeMap::new(),
            clearings: BTreeMap::new(),
        },
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
            "prairie",
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

pub(crate) fn validate_patch(
    patch: PatchRecipeContext<'_>,
    settings: &V3PrairieSettings,
    plan: &GeneratedPatchPlan,
    catalog: &RuntimeArtCatalog,
) -> WorldValidation<PrairieMetrics> {
    let frame = match patch.local_frame() {
        Ok(frame) => frame,
        Err(error) => {
            return WorldValidation::Invalid(vec![recipe_issue(format!(
                "Prairie validation frame failed: {error}"
            ))]);
        }
    };
    let protected_approaches = match patch
        .protected_approaches()
        .into_iter()
        .map(|coord| frame.to_local(coord))
        .collect::<Result<BTreeSet<_>, _>>()
    {
        Ok(protected) => protected,
        Err(error) => {
            return WorldValidation::Invalid(vec![recipe_issue(format!(
                "Prairie protected approach projection failed: {error}"
            ))]);
        }
    };
    let objects = match GrassVegetationSpec::resolve(catalog, "Prairie") {
        Ok(objects) => objects,
        Err(error) => return WorldValidation::Invalid(vec![recipe_issue(error)]),
    };
    let mut world = match frame.canonical_local_world(plan) {
        Ok(world) => world,
        Err(error) => {
            return WorldValidation::Invalid(vec![recipe_issue(format!(
                "Prairie validation projection failed: {error}"
            ))]);
        }
    };
    world.layout.grid_radius = world
        .layout
        .footprint
        .iter()
        .map(|coord| HexCoord::ORIGIN.distance(*coord))
        .max()
        .unwrap_or(12)
        .max(12);
    validate_prairie(&world, settings, &objects.grass_tuft, &protected_approaches)
}

fn validate_prairie(
    plan: &GeneratedWorldPlan,
    settings: &V3PrairieSettings,
    grass: &VegetationObjectSpec,
    protected_approaches: &BTreeSet<HexCoord>,
) -> WorldValidation<PrairieMetrics> {
    let mut issues = plan.validate();
    if !plan.liquids.bodies.is_empty()
        || !plan.structures.by_id.is_empty()
        || !plan.lights.is_empty()
        || !plan.interiors.by_id.is_empty()
        || !plan.blockers.is_empty()
    {
        issues.push(recipe_issue(
            "Prairie must not contain liquids, structures, lights, interiors, or blockers",
        ));
    }
    if !plan.features.protected_routes.is_empty() || !plan.features.clearings.is_empty() {
        issues.push(recipe_issue(
            "Prairie must not publish an authored road or clearing",
        ));
    }
    let ordinary = OrdinaryGraph::from_volume(&plan.volume, None);
    let party = plan.anchors.get(PARTY_START).copied();
    let hostile = plan.anchors.get(HOSTILE_START).copied();
    let overlook = plan.anchors.get(PRAIRIE_OVERLOOK).copied();
    let mut critical_route_steps = 0;
    match (party, hostile) {
        (Some(party), Some(hostile)) => {
            let distances = ordinary.distances_from(party);
            if distances.len() != ordinary.len() {
                issues.push(recipe_issue(format!(
                    "Prairie ordinary terrain is disconnected: {}/{} reachable",
                    distances.len(),
                    ordinary.len()
                )));
            }
            critical_route_steps = distances.get(&hostile).copied().unwrap_or_default();
            if !distances.contains_key(&hostile) {
                issues.push(recipe_issue(
                    "Prairie actor anchors are not ordinarily connected",
                ));
            }
        }
        _ => issues.push(recipe_issue(
            "Prairie requires party_start and hostile_start anchors",
        )),
    }
    if overlook.is_none() {
        issues.push(recipe_issue("Prairie requires prairie_overlook"));
    }

    let ordinary_by_coord = ordinary
        .positions()
        .map(|position| (position.coord, position))
        .collect::<BTreeMap<_, _>>();
    let eligible = eligible_grass_surfaces(
        &ordinary_by_coord,
        [party, hostile, overlook].into_iter().flatten(),
        protected_approaches,
    );
    let surface_by_coord = plan
        .volume
        .surfaces
        .keys()
        .map(|surface| (surface.coord, *surface))
        .collect::<BTreeMap<_, _>>();
    let mut grass_roots = BTreeSet::new();
    for feature in plan.features.by_id.values() {
        if feature.kind != FeatureKind::TallGrass
            || feature.object_id != grass.id
            || feature.object_id.as_str() != GRASS_TUFT_ID
            || !feature.blocker_footprint.is_empty()
        {
            issues.push(recipe_issue(format!(
                "Prairie feature at {:?} is not the accepted nonblocking grass tuft",
                feature.root
            )));
            continue;
        }
        if !grass_roots.insert(feature.root) {
            issues.push(recipe_issue(format!(
                "Prairie repeats a grass root at {:?}",
                feature.root
            )));
            continue;
        }
        let Some(projected) = grass.project_visual_volume(feature.root, feature.rotation) else {
            issues.push(recipe_issue(format!(
                "Prairie grass at {:?} cannot project its complete rotated authored volume",
                feature.root
            )));
            continue;
        };
        for visual in projected.cells {
            let Some(support) = surface_by_coord.get(&visual.coord).copied() else {
                issues.push(recipe_issue(format!(
                    "Prairie grass at {:?} leaves terrain at {visual:?}",
                    feature.root
                )));
                continue;
            };
            if visual.level <= support.level {
                issues.push(recipe_issue(format!(
                    "Prairie grass at {:?} intersects terrain at {visual:?}",
                    feature.root
                )));
            }
            if !eligible.contains(&support) && visual.level <= support.level.saturating_add(2) {
                issues.push(recipe_issue(format!(
                    "Prairie grass at {:?} enters a protected walker volume at {visual:?}",
                    feature.root
                )));
            }
        }
    }
    if !grass_roots.is_subset(&eligible) {
        issues.push(recipe_issue(
            "Prairie grass leaves its exact route-and-anchor-safe eligible set",
        ));
    }
    let expected_grass = eligible
        .len()
        .saturating_mul(usize::from(settings.grass_coverage_percent))
        / 100;
    if plan.features.by_id.len() != expected_grass || grass_roots.len() != expected_grass {
        issues.push(recipe_issue(format!(
            "Prairie requires exactly {expected_grass} unique grass features, got {} features and \
             {} unique accepted roots",
            plan.features.by_id.len(),
            grass_roots.len()
        )));
    }
    let coverage = count_u32(grass_roots.len())
        .saturating_mul(100)
        .checked_div(count_u32(eligible.len()))
        .unwrap_or_default();
    if !(65..=75).contains(&coverage)
        || coverage.abs_diff(u32::from(settings.grass_coverage_percent)) > 1
    {
        issues.push(recipe_issue(format!(
            "Prairie grass coverage is outside its authored target: {coverage}%"
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
            "Prairie relief must be {}, got {relief}",
            settings.max_relief
        )));
    }
    if !issues.is_empty() {
        return WorldValidation::Invalid(issues);
    }
    WorldValidation::Valid(PrairieMetrics {
        grass_roots: count_u32(grass_roots.len()),
        eligible_grass_surfaces: count_u32(eligible.len()),
        grass_coverage_percent: coverage,
        ordinary_surfaces: count_u32(ordinary.len()),
        reachable_elevation_levels: count_u32(levels.len()),
        relief,
        critical_route_steps,
    })
}

fn eligible_grass_surfaces(
    ordinary_by_coord: &BTreeMap<HexCoord, TilePos>,
    anchors: impl IntoIterator<Item = TilePos>,
    protected_approaches: &BTreeSet<HexCoord>,
) -> BTreeSet<TilePos> {
    let mut excluded_coords = protected_approaches.clone();
    for anchor in anchors {
        excluded_coords.extend(anchor.coord.within_radius(1));
    }
    ordinary_by_coord
        .iter()
        .filter_map(|(coord, position)| (!excluded_coords.contains(coord)).then_some(*position))
        .collect()
}

fn object_rotation(
    stream: Option<SeedStream<'_>>,
    coord: HexCoord,
    salt: u64,
) -> Result<HexObjectRotation, Vec<WorldValidationIssue>> {
    let steps = u8::try_from(feature_priority(stream, coord, salt) % 6).unwrap_or_default();
    HexObjectRotation::new(steps)
        .map_err(|error| vec![recipe_issue(format!("invalid Prairie rotation: {error}"))])
}

fn feature_priority(stream: Option<SeedStream<'_>>, coord: HexCoord, salt: u64) -> u64 {
    stream.map_or_else(
        || {
            let mut bytes = Vec::with_capacity(48);
            bytes.extend_from_slice(b"bevy-hex-game/v3/prairie/fallback-feature");
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
        V3RecipeSettings::ShallowSea(_) => "ShallowSea",
        V3RecipeSettings::Beach(_) => "Beach",
        V3RecipeSettings::Shore(_) => "Shore",
        V3RecipeSettings::DeepMountain(_) => "DeepMountain",
        V3RecipeSettings::CrystalAscent(_) => "CrystalAscent",
        V3RecipeSettings::DesertTransition(_) => "DesertTransition",
        V3RecipeSettings::DesertPlain(_) => "DesertPlain",
        V3RecipeSettings::Dunes(_) => "Dunes",
        V3RecipeSettings::Oasis(_) => "Oasis",
        V3RecipeSettings::SandyIslets(_) => "SandyIslets",
        V3RecipeSettings::WoodedIsland(_) => "WoodedIsland",
    }
}

fn recipe_issue(detail: impl Into<String>) -> WorldValidationIssue {
    WorldValidationIssue::new(WorldIssueCode::Recipe("prairie"), detail)
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
                recipe: V3RecipeSettings::Prairie(V3PrairieSettings {
                    base_level: 15,
                    max_relief: 4,
                    grass_coverage_percent: 70,
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
    ) -> Result<ValidatedWorldSelection<PrairieMetrics>, V3GenerationError> {
        super::generate(
            radius,
            0.4,
            &settings(),
            seed,
            super::super::vegetation::tests::runtime_art_catalog(),
        )
    }

    #[test]
    fn fixed_corpus_builds_deterministic_open_prairies() {
        for radius in [12, 20, 40] {
            for seed in [0, 1, 1592598566, 4_294_967_311] {
                let first = generate(radius, seed).expect("Prairie should generate");
                let repeated = generate(radius, seed).expect("Prairie should repeat");
                assert_eq!(
                    first.validated.semantic_fingerprint,
                    repeated.validated.semantic_fingerprint
                );
                assert!(!first.used_fallback);
                assert_eq!(first.candidates_evaluated, 8);
                assert_eq!(first.valid_candidates, 8);
                assert!(first.notes.is_empty());
                assert!((65..=75).contains(&first.metrics.grass_coverage_percent));
                assert_eq!(first.metrics.relief, 4);
                assert!(first.validated.plan.blockers.is_empty());
                assert!(first.validated.plan.features.protected_routes.is_empty());
            }
        }
    }

    #[test]
    fn radius_12_pr_corpus_validates_128_prairie_seeds() {
        let fallbacks = (0..128_u64)
            .filter(|seed| {
                generate(12, *seed)
                    .expect("Prairie should generate")
                    .used_fallback
            })
            .count();
        assert_eq!(
            fallbacks, 0,
            "actual Prairie fallback count for the 128-seed PR corpus"
        );
    }

    #[test]
    fn prairie_resolves_from_a_grass_only_art_catalog() {
        let selected = super::generate(
            12,
            0.4,
            &settings(),
            1592598566,
            super::super::vegetation::tests::grass_only_runtime_art_catalog(),
        )
        .expect("Prairie should not require unrelated tree assets");
        assert_eq!(selected.candidates_evaluated, 8);
        assert_eq!(selected.valid_candidates, 8);
    }

    #[test]
    fn forced_candidate_failure_uses_independent_prairie_fallback() {
        let settings = settings();
        let layout = resolve_layout(12, &settings).expect("fixture layout should resolve");
        let objects = GrassVegetationSpec::resolve(
            super::super::vegetation::tests::grass_only_runtime_art_catalog(),
            "Prairie",
        )
        .expect("fixture art should resolve");
        let force = |seed| {
            run_recipe(
                &PrairieRecipe {
                    level_height: 0.4,
                    layout: layout.clone(),
                    objects: objects.clone(),
                    reject_candidates: true,
                },
                &settings,
                12,
                seed,
            )
            .expect("canonical Prairie fallback should validate")
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
