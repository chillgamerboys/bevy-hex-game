//! Whole-world selection and validation for the fixed V3 seven-region layout.
//!
//! Ring7 deliberately owns one candidate loop. A candidate index is handed to all
//! seven patch recipes unchanged, the fragments are validated in isolation, and
//! only their checked composition may enter scoring.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use hex_assets::RuntimeArtCatalog;
use hex_core::{BiomeRegionId, HexCoord, Level, MapViewHint, TilePos};

use super::composition::{
    compose_world, GeneratedPatchPlan, PatchAnchorRef, WorldCompositionSettings,
};
use super::layout::{
    resolve_layout, PatchId, ResolvedLayoutPlan, ResolvedLiquidElevation, ResolvedLiquidPort,
};
use super::patch::{PatchBuildMode, PatchRecipeContext};
use super::selection::{
    run_recipe, CandidateAttemptError, CandidateContext, FallbackContext, RepairOutcome, V3Recipe,
    ValidatedWorldSelection, WorldValidation,
};
use super::traversal::OrdinaryGraph;
use super::vegetation::CaveVegetationSet;
use super::world::{GeneratedWorldPlan, WorldIssueCode, WorldValidationIssue};
use super::V3GenerationError;
use crate::procedural::Ring7Metrics as Ring7ReportMetrics;
use crate::settings::{
    PatchSpec, ProceduralV3Settings, V3EnvironmentSettings, V3LayoutSettings, V3RecipeSettings,
    V3Ring7Settings,
};

const RING_RADIUS: u32 = 33;
const PATCH_COUNT: u32 = 7;
const PARTY_START: &str = "party_start";
const HOSTILE_START: &str = "hostile_start";
const CONFLICT_CENTER: &str = "conflict_center";

/// Candidate measurements retained beyond the public generation report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Ring7Metrics {
    pub(crate) report: Ring7ReportMetrics,
    max_region_entry_steps: u32,
    region_entry_spread: u32,
}

#[derive(Debug)]
struct Ring7Recipe<'a> {
    level_height: f32,
    layout: ResolvedLayoutPlan,
    settings: &'a V3Ring7Settings,
    art_catalog: &'a RuntimeArtCatalog,
    cave_vegetation: CaveVegetationSet,
    #[cfg(test)]
    _reject_candidates: bool,
}

/// Runs exactly one eight-candidate selection loop for the complete Ring7 world.
pub(crate) fn generate(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV3Settings,
    seed: u64,
    art_catalog: &RuntimeArtCatalog,
) -> Result<ValidatedWorldSelection<Ring7Metrics>, V3GenerationError> {
    generate_with_options(
        grid_radius,
        level_height,
        settings,
        seed,
        art_catalog,
        false,
    )
}

fn generate_with_options(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV3Settings,
    seed: u64,
    art_catalog: &RuntimeArtCatalog,
    _reject_candidates: bool,
) -> Result<ValidatedWorldSelection<Ring7Metrics>, V3GenerationError> {
    if grid_radius != RING_RADIUS {
        return Err(V3GenerationError::RecipeContract(format!(
            "Ring7 requires grid radius {RING_RADIUS}, got {grid_radius}"
        )));
    }
    if !level_height.is_finite() || level_height <= 0.0 {
        return Err(V3GenerationError::RecipeContract(
            "Ring7 level height must be positive and finite".to_owned(),
        ));
    }
    let ring = validate_recipe_settings(settings)?;
    let layout = resolve_layout(grid_radius, settings)
        .map_err(|error| V3GenerationError::RecipeContract(error.to_string()))?;
    if layout.patches.len() != usize::try_from(PATCH_COUNT).unwrap_or(usize::MAX) {
        return Err(V3GenerationError::RecipeContract(format!(
            "Ring7 resolved {} patches instead of {PATCH_COUNT}",
            layout.patches.len()
        )));
    }
    validate_resolved_hydrology(&layout)?;
    let cave_vegetation = CaveVegetationSet::resolve(art_catalog, "Ring7 Caves")
        .map_err(V3GenerationError::RecipeContract)?;

    run_recipe(
        &Ring7Recipe {
            level_height,
            layout,
            settings: ring,
            art_catalog,
            cave_vegetation,
            #[cfg(test)]
            _reject_candidates,
        },
        settings,
        grid_radius,
        seed,
    )
}

impl V3Recipe for Ring7Recipe<'_> {
    type Settings = ProceduralV3Settings;
    type Metrics = Ring7Metrics;
    type Score = (u32, u32, Reverse<u32>, u8);

    fn construct(
        &self,
        context: CandidateContext,
        settings: &Self::Settings,
    ) -> Result<GeneratedWorldPlan, CandidateAttemptError> {
        validate_recipe_settings(settings).map_err(CandidateAttemptError::Fatal)?;
        if context.grid_radius != self.layout.grid_radius {
            return Err(CandidateAttemptError::Fatal(
                V3GenerationError::RecipeContract(
                    "Ring7 candidate radius disagrees with its resolved layout".to_owned(),
                ),
            ));
        }
        #[cfg(test)]
        if self._reject_candidates {
            return Err(CandidateAttemptError::Rejected(vec![recipe_issue(
                "forced Ring7 candidate rejection",
            )]));
        }

        self.construct_world(PatchBuildMode::Candidate {
            world_seed: context.seed,
            candidate: context.candidate,
        })
        .map_err(CandidateAttemptError::Rejected)
    }

    fn validate(
        &self,
        _settings: &Self::Settings,
        plan: &GeneratedWorldPlan,
    ) -> WorldValidation<Self::Metrics> {
        validate_ring7(plan)
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
            metrics.max_region_entry_steps,
            metrics.region_entry_spread,
            Reverse(metrics.report.reachable_elevation_levels),
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
                "Ring7 fallback radius disagrees with its resolved layout".to_owned(),
            ));
        }
        self.construct_world(PatchBuildMode::CanonicalFallback)
            .map_err(recipe_issues_to_error)
    }
}

impl Ring7Recipe<'_> {
    fn construct_world(
        &self,
        mode: PatchBuildMode,
    ) -> Result<GeneratedWorldPlan, Vec<WorldValidationIssue>> {
        let fragments = self.construct_fragments(mode)?;
        let view_hint = ring_view_hint(&fragments, self.level_height)?;
        compose_world(
            self.layout.clone(),
            fragments,
            WorldCompositionSettings {
                canonical_anchors: BTreeMap::from([
                    (
                        PARTY_START.to_owned(),
                        PatchAnchorRef {
                            patch: PatchId(0),
                            local_name: PARTY_START.to_owned(),
                        },
                    ),
                    (
                        HOSTILE_START.to_owned(),
                        PatchAnchorRef {
                            patch: PatchId(0),
                            local_name: HOSTILE_START.to_owned(),
                        },
                    ),
                    (
                        CONFLICT_CENTER.to_owned(),
                        PatchAnchorRef {
                            patch: PatchId(0),
                            local_name: CONFLICT_CENTER.to_owned(),
                        },
                    ),
                ]),
                view_hint,
            },
        )
        .map_err(|error| {
            vec![recipe_issue(format!(
                "Ring7 checked composition failed: {error:?}"
            ))]
        })
    }

    fn construct_fragments(
        &self,
        mode: PatchBuildMode,
    ) -> Result<Vec<GeneratedPatchPlan>, Vec<WorldValidationIssue>> {
        let specs = ring_specs(self.settings);
        let mut fragments = Vec::with_capacity(specs.len());
        for (id, spec) in specs {
            let patch = PatchRecipeContext::resolve(&self.layout, id)
                .map_err(|error| vec![recipe_issue(error.to_string())])?;
            let fragment = construct_fragment(
                patch,
                spec,
                self.level_height,
                mode,
                self.art_catalog,
                &self.cave_vegetation,
            )
            .map_err(|issues| {
                issues
                    .into_iter()
                    .map(|issue| {
                        recipe_issue(format!(
                            "patch {} {} construction {:?}: {}",
                            id.0,
                            recipe_name(&spec.recipe),
                            issue.code,
                            issue.detail
                        ))
                    })
                    .collect::<Vec<_>>()
            })?;
            validate_fragment(
                patch,
                spec,
                &fragment,
                self.art_catalog,
                &self.cave_vegetation,
            )
            .map_err(|issues| {
                issues
                    .into_iter()
                    .map(|issue| {
                        recipe_issue(format!(
                            "patch {} {} validation {:?}: {}",
                            id.0,
                            recipe_name(&spec.recipe),
                            issue.code,
                            issue.detail
                        ))
                    })
                    .collect::<Vec<_>>()
            })?;
            fragments.push(fragment);
        }
        Ok(fragments)
    }
}

fn validate_recipe_settings(
    settings: &ProceduralV3Settings,
) -> Result<&V3Ring7Settings, V3GenerationError> {
    let V3LayoutSettings::Ring7(ring) = &settings.layout else {
        return Err(V3GenerationError::RecipeUnavailable("Single"));
    };
    let expected = [
        (
            "center",
            &ring.center,
            V3EnvironmentSettings::TemperateGrassland,
            "Hills",
        ),
        (
            "mountains",
            &ring.mountains,
            V3EnvironmentSettings::Frozen,
            "Mountains",
        ),
        (
            "waterfall",
            &ring.waterfall,
            V3EnvironmentSettings::TemperateGrassland,
            "Waterfall",
        ),
        (
            "forest",
            &ring.forest,
            V3EnvironmentSettings::TemperateGrassland,
            "Forest",
        ),
        (
            "fort",
            &ring.fort,
            V3EnvironmentSettings::TemperateGrassland,
            "Fort",
        ),
        ("caves", &ring.caves, V3EnvironmentSettings::Rocky, "Caves"),
        (
            "sky_islands",
            &ring.sky_islands,
            V3EnvironmentSettings::TemperateGrassland,
            "SkyIslands",
        ),
    ];
    for (name, patch, environment, recipe) in expected {
        if patch.environment != environment {
            return Err(V3GenerationError::RecipeContract(format!(
                "Ring7 {name} requires the {environment:?} environment"
            )));
        }
        if recipe_name(&patch.recipe) != recipe {
            return Err(V3GenerationError::RecipeContract(format!(
                "Ring7 {name} requires {recipe}, got {}",
                recipe_name(&patch.recipe)
            )));
        }
    }
    if ring
        .center
        .overlays
        .iter()
        .chain(&ring.mountains.overlays)
        .chain(&ring.waterfall.overlays)
        .chain(&ring.forest.overlays)
        .chain(&ring.fort.overlays)
        .chain(&ring.sky_islands.overlays)
        .next()
        .is_some()
    {
        return Err(V3GenerationError::RecipeContract(
            "Ring7 accepts overlays only on the Caves patch for now".to_owned(),
        ));
    }
    if ring
        .caves
        .overlays
        .iter()
        .any(|overlay| overlay.kind != crate::settings::V3OverlaySettings::Lighting)
    {
        return Err(V3GenerationError::RecipeContract(
            "Ring7 Caves accepts only Lighting overlays".to_owned(),
        ));
    }
    Ok(ring)
}

fn ring_specs(ring: &V3Ring7Settings) -> [(PatchId, &PatchSpec); 7] {
    [
        (PatchId(0), &ring.center),
        (PatchId(1), &ring.mountains),
        (PatchId(2), &ring.waterfall),
        (PatchId(3), &ring.forest),
        (PatchId(4), &ring.fort),
        (PatchId(5), &ring.caves),
        (PatchId(6), &ring.sky_islands),
    ]
}

fn validate_resolved_hydrology(layout: &ResolvedLayoutPlan) -> Result<(), V3GenerationError> {
    if !layout.boundary_liquid_outlets.is_empty() {
        return Err(V3GenerationError::RecipeContract(
            "Ring7 cannot own complete-world boundary liquid outlets".to_owned(),
        ));
    }
    let mut actual = BTreeSet::new();
    for edge in layout.shared_edges.values() {
        let ResolvedLiquidPort::Directed {
            source,
            sink,
            elevation,
            ..
        } = &edge.liquid
        else {
            continue;
        };
        if *elevation != ResolvedLiquidElevation::EdgeBand {
            return Err(V3GenerationError::RecipeContract(
                "Ring7 liquid seams must retain legacy edge-band elevation authority".to_owned(),
            ));
        }
        actual.insert((*source, *sink));
    }
    let expected = BTreeSet::from([(PatchId(2), PatchId(0)), (PatchId(0), PatchId(5))]);
    if actual != expected {
        return Err(V3GenerationError::RecipeContract(format!(
            "Ring7 hydrology must be exactly Waterfall(2)->Hills(0)->Caves(5), got {actual:?}"
        )));
    }
    Ok(())
}

fn construct_fragment(
    patch: PatchRecipeContext<'_>,
    spec: &PatchSpec,
    level_height: f32,
    mode: PatchBuildMode,
    art_catalog: &RuntimeArtCatalog,
    cave_vegetation: &CaveVegetationSet,
) -> Result<GeneratedPatchPlan, Vec<WorldValidationIssue>> {
    super::composite_patch::construct_fragment(
        patch,
        spec.environment,
        &spec.recipe,
        level_height,
        mode,
        art_catalog,
        cave_vegetation,
    )
}

fn validate_fragment(
    patch: PatchRecipeContext<'_>,
    spec: &PatchSpec,
    fragment: &GeneratedPatchPlan,
    art_catalog: &RuntimeArtCatalog,
    cave_vegetation: &CaveVegetationSet,
) -> Result<(), Vec<WorldValidationIssue>> {
    super::composite_patch::validate_fragment(
        patch,
        spec.environment,
        &spec.recipe,
        fragment,
        art_catalog,
        cave_vegetation,
    )
}

fn validate_ring7(plan: &GeneratedWorldPlan) -> WorldValidation<Ring7Metrics> {
    let mut issues = plan.validate();
    if plan.layout.grid_radius != RING_RADIUS
        || plan.layout.patches.len() != usize::try_from(PATCH_COUNT).unwrap_or(usize::MAX)
    {
        issues.push(recipe_issue(
            "Ring7 final layout is not the fixed radius-33 seven-patch world",
        ));
    }

    let ordinary = OrdinaryGraph::from_volume(&plan.volume, Some(&plan.blockers));
    let Some(party) = plan.anchors.get(PARTY_START).copied() else {
        return WorldValidation::Invalid(vec![recipe_issue(
            "Ring7 is missing its canonical party_start alias",
        )]);
    };
    let Some(hostile) = plan.anchors.get(HOSTILE_START).copied() else {
        return WorldValidation::Invalid(vec![recipe_issue(
            "Ring7 is missing its canonical hostile_start alias",
        )]);
    };
    if !plan.anchors.contains_key(CONFLICT_CENTER) {
        issues.push(recipe_issue(
            "Ring7 is missing its canonical conflict_center alias",
        ));
    }
    let distances = ordinary.distances_from(party);
    if distances.len() != ordinary.len() {
        let disconnected = ordinary
            .positions()
            .filter(|position| !distances.contains_key(position))
            .take(8)
            .collect::<Vec<_>>();
        issues.push(recipe_issue(format!(
            "Ring7 ordinary terrain is not one connected world network: reached {}/{} surfaces; \
             disconnected examples: {disconnected:?}",
            distances.len(),
            ordinary.len(),
        )));
    }
    let critical_route_steps = distances.get(&hostile).copied();
    if critical_route_steps.is_none() {
        issues.push(recipe_issue(
            "Ring7 canonical actor anchors are not joined by ordinary traversal",
        ));
    }

    let mut region_entry_steps = BTreeMap::<PatchId, u32>::new();
    for id in 0..PATCH_COUNT {
        let region = BiomeRegionId(id);
        let entry = distances
            .iter()
            .filter(|(position, _)| plan.biome_regions.get(position) == Some(&region))
            .map(|(_, distance)| *distance)
            .min();
        match entry {
            Some(distance) => {
                region_entry_steps.insert(PatchId(id), distance);
            }
            None => issues.push(recipe_issue(format!(
                "Ring7 patch {id} has no ordinary surface reachable from party_start"
            ))),
        }
    }

    let macro_edges = open_macro_edges(plan, &ordinary);
    let expected_macro_edges = plan
        .layout
        .shared_edges
        .values()
        .filter(|edge| edge.walker.count > 0)
        .count();
    if macro_edges.len() != expected_macro_edges {
        issues.push(recipe_issue(format!(
            "Ring7 opens {} of {expected_macro_edges} declared walker seams",
            macro_edges.len()
        )));
    }
    let redundant_regions = redundant_region_count(&macro_edges);
    if redundant_regions != PATCH_COUNT {
        issues.push(recipe_issue(format!(
            "Ring7 macro graph retains redundant routes for {redundant_regions}/{PATCH_COUNT} patches"
        )));
    }

    if plan
        .liquids
        .bodies
        .values()
        .flat_map(|body| body.nodes.keys())
        .any(|position| {
            plan.biome_regions.get(position) == Some(&BiomeRegionId(PatchId(1).0))
                || plan.volume.surfaces.iter().any(|(surface, _)| {
                    surface.coord == position.coord
                        && plan.biome_regions.get(surface) == Some(&BiomeRegionId(1))
                })
        })
    {
        issues.push(recipe_issue("Ring7 Mountains must remain dry"));
    }
    let directed_liquid_seams = validate_directed_liquid_seams(plan, &mut issues);
    if directed_liquid_seams != 2 {
        issues.push(recipe_issue(format!(
            "Ring7 must install exactly two directed liquid seams, got {directed_liquid_seams}"
        )));
    }
    let liquid_cells = plan
        .liquids
        .bodies
        .values()
        .flat_map(|body| body.nodes.keys().map(|position| position.coord))
        .collect::<BTreeSet<_>>()
        .len();

    let reachable_levels = distances
        .keys()
        .map(|position| position.level)
        .collect::<BTreeSet<_>>();
    let min_level = reachable_levels.iter().next().copied().unwrap_or_default();
    let max_level = reachable_levels
        .iter()
        .next_back()
        .copied()
        .unwrap_or_default();
    let min_outer_entry = region_entry_steps
        .iter()
        .filter_map(|(patch, distance)| (patch.0 != 0).then_some(*distance))
        .min()
        .unwrap_or_default();
    let max_outer_entry = region_entry_steps
        .iter()
        .filter_map(|(patch, distance)| (patch.0 != 0).then_some(*distance))
        .max()
        .unwrap_or_default();
    let report = Ring7ReportMetrics {
        ordinary_surfaces: count_u32(ordinary.len()),
        reachable_surfaces: count_u32(distances.len()),
        reachable_elevation_levels: count_u32(reachable_levels.len()),
        relief: max_level.saturating_sub(min_level),
        critical_route_steps: critical_route_steps.unwrap_or_default(),
        macro_edges: count_u32(macro_edges.len()),
        redundant_regions,
        directed_liquid_seams,
        liquid_cells: count_u32(liquid_cells),
        feature_instances: count_u32(plan.features.by_id.len()),
        structures: count_u32(plan.structures.by_id.len()),
        gameplay_lights: count_u32(plan.lights.len()),
        interiors: count_u32(plan.interiors.by_id.len()),
    };
    if issues.is_empty() {
        WorldValidation::Valid(Ring7Metrics {
            report,
            max_region_entry_steps: max_outer_entry,
            region_entry_spread: max_outer_entry.saturating_sub(min_outer_entry),
        })
    } else {
        WorldValidation::Invalid(issues)
    }
}

fn open_macro_edges(
    plan: &GeneratedWorldPlan,
    ordinary: &OrdinaryGraph,
) -> BTreeSet<(PatchId, PatchId)> {
    let by_coord = ordinary.positions().fold(
        BTreeMap::<HexCoord, Vec<TilePos>>::new(),
        |mut grouped, position| {
            grouped.entry(position.coord).or_default().push(position);
            grouped
        },
    );
    plan.layout
        .shared_edges
        .values()
        .filter(|edge| {
            edge.walker.ports.iter().any(|port| {
                port.lanes.iter().any(|(first, second)| {
                    by_coord.get(first).is_some_and(|first_positions| {
                        by_coord.get(second).is_some_and(|second_positions| {
                            first_positions.iter().any(|first| {
                                second_positions
                                    .iter()
                                    .any(|second| ordinary.admits(*first, *second))
                            })
                        })
                    })
                })
            })
        })
        .map(|edge| ordered_edge(edge.first.0, edge.second.0))
        .collect()
}

fn redundant_region_count(edges: &BTreeSet<(PatchId, PatchId)>) -> u32 {
    (0..PATCH_COUNT)
        .filter(|target| {
            edges
                .iter()
                .all(|removed| reachable_patches(edges, Some(*removed)).contains(&PatchId(*target)))
        })
        .count()
        .try_into()
        .unwrap_or(u32::MAX)
}

fn reachable_patches(
    edges: &BTreeSet<(PatchId, PatchId)>,
    removed: Option<(PatchId, PatchId)>,
) -> BTreeSet<PatchId> {
    let mut reached = BTreeSet::from([PatchId(0)]);
    let mut queue = VecDeque::from([PatchId(0)]);
    while let Some(current) = queue.pop_front() {
        for edge in edges {
            if Some(*edge) == removed {
                continue;
            }
            let neighbor = if edge.0 == current {
                Some(edge.1)
            } else if edge.1 == current {
                Some(edge.0)
            } else {
                None
            };
            if let Some(neighbor) = neighbor.filter(|neighbor| reached.insert(*neighbor)) {
                queue.push_back(neighbor);
            }
        }
    }
    reached
}

fn validate_directed_liquid_seams(
    plan: &GeneratedWorldPlan,
    issues: &mut Vec<WorldValidationIssue>,
) -> u32 {
    let mut seam_count = 0_u32;
    for edge in plan.layout.shared_edges.values() {
        let ResolvedLiquidPort::Directed {
            source,
            sink,
            port,
            elevation,
        } = &edge.liquid
        else {
            continue;
        };
        seam_count = seam_count.saturating_add(1);
        if *elevation != ResolvedLiquidElevation::EdgeBand {
            issues.push(recipe_issue(
                "Ring7 directed liquid seams must retain legacy edge-band elevation authority",
            ));
            continue;
        }
        let source_is_first = *source == edge.first.0 && *sink == edge.second.0;
        for (first, second) in &port.lanes {
            let (source_coord, sink_coord) = if source_is_first {
                (*first, *second)
            } else {
                (*second, *first)
            };
            let source_node =
                liquid_endpoint(plan, source_coord, edge.elevation.min, edge.elevation.max);
            let sink_node =
                liquid_endpoint(plan, sink_coord, edge.elevation.min, edge.elevation.max);
            match (source_node, sink_node) {
                (Some((source_position, source)), Some((sink_position, _))) => {
                    if source.downstream != Some(sink_position) {
                        issues.push(recipe_issue(format!(
                            "Ring7 directed liquid seam {:?}->{:?} does not link exact endpoints",
                            source_position, sink_position
                        )));
                    }
                }
                _ => issues.push(recipe_issue(format!(
                    "Ring7 directed liquid seam {source:?}->{sink:?} is missing a unique endpoint \
                     at {source_coord:?}/{sink_coord:?}"
                ))),
            }
        }
    }
    seam_count
}

fn liquid_endpoint(
    plan: &GeneratedWorldPlan,
    coord: HexCoord,
    min: Level,
    max: Level,
) -> Option<(TilePos, super::liquid::LiquidNode)> {
    let mut endpoints = plan
        .liquids
        .bodies
        .values()
        .flat_map(|body| body.nodes.iter())
        .filter(|(position, _)| {
            position.coord == coord && min <= position.level && position.level <= max
        })
        .map(|(position, node)| (*position, *node));
    let endpoint = endpoints.next()?;
    endpoints.next().is_none().then_some(endpoint)
}

fn ring_view_hint(
    fragments: &[GeneratedPatchPlan],
    level_height: f32,
) -> Result<MapViewHint, Vec<WorldValidationIssue>> {
    if fragments
        .iter()
        .flat_map(|fragment| fragment.volume.surfaces.keys())
        .next()
        .is_none()
    {
        return Err(vec![recipe_issue(
            "Ring7 cannot frame a world without semantic surfaces",
        )]);
    }
    let focus_height = 15.0 * level_height;
    let frame = 118.0_f32;
    Ok(MapViewHint::new(
        (0.0, focus_height + frame * 0.72, frame * 0.82),
        (0.0, focus_height, 0.0),
    ))
}

fn ordered_edge(first: PatchId, second: PatchId) -> (PatchId, PatchId) {
    if first < second {
        (first, second)
    } else {
        (second, first)
    }
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

fn recipe_issue(detail: impl Into<String>) -> WorldValidationIssue {
    WorldValidationIssue::new(WorldIssueCode::Recipe("ring7"), detail)
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
    }
}

fn count_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use hex_assets::{ArtPalette, ObjectBlueprint, ObjectCatalogFile, VoxelStyleCatalog};

    use super::*;
    use crate::settings::{
        EdgeLiquidPortSettings, EdgeLiquidSettings, MapSettings, PatchEdgeContractSettings,
        ProceduralSettings, TerrainSettings,
    };

    const PINNED_SEED: u64 = 703_700_113;

    #[test]
    fn fixed_seed_selects_one_deterministic_complete_world() {
        let settings = settings();
        let first = generate(
            RING_RADIUS,
            0.4,
            settings,
            PINNED_SEED,
            runtime_art_catalog(),
        )
        .expect("the pinned Ring7 should generate");
        let second = generate(
            RING_RADIUS,
            0.4,
            settings,
            PINNED_SEED,
            runtime_art_catalog(),
        )
        .expect("the pinned Ring7 should reproduce");

        assert_eq!(first.selected_candidate, second.selected_candidate);
        assert_eq!(
            first.validated.semantic_fingerprint,
            second.validated.semantic_fingerprint
        );
        assert_eq!(first.metrics, second.metrics);
        assert_eq!(first.candidates_evaluated, 8);
        assert!(
            !first.used_fallback,
            "pinned Ring7 candidates unexpectedly fell back: {:#?}",
            first.notes
        );
        assert!(
            first.valid_candidates >= 2,
            "pinned Ring7 should retain multiple complete candidates: {:#?}",
            first.notes
        );
        assert!(first.repair_rounds.is_empty());
    }

    #[test]
    fn report_covers_the_complete_redundant_world_and_exact_liquid_dag() {
        let selection = generate(
            RING_RADIUS,
            0.4,
            settings(),
            PINNED_SEED,
            runtime_art_catalog(),
        )
        .expect("the pinned Ring7 should generate");
        let metrics = selection.metrics.report;
        let plan = &selection.validated.plan;
        let ordinary = OrdinaryGraph::from_volume(&plan.volume, Some(&plan.blockers));
        let party = plan
            .anchors
            .get(PARTY_START)
            .copied()
            .expect("validated Ring7 party anchor");
        let reachable = ordinary.distances_from(party);
        let liquid_cells = plan
            .liquids
            .bodies
            .values()
            .flat_map(|body| body.nodes.keys().map(|position| position.coord))
            .collect::<BTreeSet<_>>();
        let outer_entries = (1..PATCH_COUNT)
            .map(|id| {
                reachable
                    .iter()
                    .filter(|(position, _distance)| {
                        plan.biome_regions.get(position) == Some(&BiomeRegionId(id))
                    })
                    .map(|(_position, distance)| *distance)
                    .min()
                    .expect("validated outer patch has a reachable ordinary entry")
            })
            .collect::<Vec<_>>();
        let min_outer_entry = outer_entries.iter().copied().min().unwrap_or_default();
        let max_outer_entry = outer_entries.iter().copied().max().unwrap_or_default();

        assert_eq!(metrics.macro_edges, 12);
        assert_eq!(metrics.redundant_regions, PATCH_COUNT);
        assert_eq!(metrics.directed_liquid_seams, 2);
        assert_eq!(metrics.ordinary_surfaces, count_u32(ordinary.len()));
        assert_eq!(metrics.reachable_surfaces, count_u32(reachable.len()));
        assert_eq!(metrics.reachable_surfaces, metrics.ordinary_surfaces);
        assert_eq!(metrics.liquid_cells, count_u32(liquid_cells.len()));
        assert_eq!(
            metrics.feature_instances,
            count_u32(plan.features.by_id.len())
        );
        assert_eq!(metrics.structures, count_u32(plan.structures.by_id.len()));
        assert_eq!(metrics.gameplay_lights, count_u32(plan.lights.len()));
        assert_eq!(metrics.interiors, count_u32(plan.interiors.by_id.len()));
        assert!(metrics.reachable_elevation_levels > 1);
        assert_eq!(selection.metrics.max_region_entry_steps, max_outer_entry);
        assert_eq!(
            selection.metrics.region_entry_spread,
            max_outer_entry.saturating_sub(min_outer_entry)
        );
        assert!(
            selection.metrics.region_entry_spread < selection.metrics.max_region_entry_steps,
            "outer-region spread must not duplicate the farthest-entry score key"
        );
    }

    #[test]
    fn fixed_seed_corpus_selects_complete_candidates_without_fallback() {
        // These seeds exercised the Forest, Caves, Sky, and seam failures closed
        // while hardening the initial 0..32 audit. Keep the blocking CI corpus
        // focused; the ignored stress test below owns broad statistical coverage.
        const REGRESSION_SEEDS: [u64; 8] = [0, 7, 14, 17, 19, 23, 28, 31];
        for seed in REGRESSION_SEEDS {
            let selected = generate(RING_RADIUS, 0.4, settings(), seed, runtime_art_catalog())
                .expect("every Ring7 seed must produce a validated final world");
            assert!(
                !selected.used_fallback,
                "fixed Ring7 seed {seed} unexpectedly used fallback: {:#?}",
                selected.notes
            );
            assert!(
                selected.valid_candidates >= 1,
                "fixed Ring7 seed {seed} must retain at least one complete candidate"
            );
            let forest_old_growth = selected
                .validated
                .plan
                .features
                .by_id
                .values()
                .filter(|feature| {
                    feature.object_id.as_str() == super::super::vegetation::OLD_GROWTH_ID
                        && selected.validated.plan.biome_regions.get(&feature.root)
                            == Some(&BiomeRegionId(3))
                })
                .collect::<Vec<_>>();
            assert!(
                !forest_old_growth.is_empty(),
                "fixed Ring7 seed {seed} must retain authored Old-Growth in Forest"
            );
            assert!(
                forest_old_growth
                    .iter()
                    .all(|feature| feature.blocker_footprint.len() == 7),
                "fixed Ring7 seed {seed} must retain exact seven-cell Old-Growth roots"
            );
        }
    }

    #[test]
    #[ignore = "10,000-seed Ring7 stress corpus"]
    fn ten_thousand_seed_corpus_has_less_than_one_percent_fallbacks() {
        let mut fallback_count = 0_usize;
        for seed in 0..10_000_u64 {
            let selection = generate(RING_RADIUS, 0.4, settings(), seed, runtime_art_catalog())
                .expect("every Ring7 seed must produce a validated final world");
            fallback_count = fallback_count.saturating_add(usize::from(selection.used_fallback));
        }
        assert!(
            fallback_count < 100,
            "Ring7 fallback rate must remain below 1%, got {fallback_count}/10000"
        );
    }

    #[test]
    fn legacy_report_uses_liquid_occupancy_without_fabricating_an_environment_percentage() {
        let metrics = Ring7Metrics {
            report: Ring7ReportMetrics {
                ordinary_surfaces: 100,
                reachable_surfaces: 100,
                reachable_elevation_levels: 8,
                relief: 16,
                critical_route_steps: 42,
                macro_edges: 12,
                redundant_regions: 7,
                directed_liquid_seams: 2,
                liquid_cells: 73,
                feature_instances: 90,
                structures: 18,
                gameplay_lights: 7,
                interiors: 1,
            },
            max_region_entry_steps: 31,
            region_entry_spread: 12,
        };

        let reported = super::super::ring7_report_metrics(&metrics);
        assert_eq!(reported.barrier_cells, 73);
        assert_eq!(reported.environment_signature_percent, 0);
        assert!(reported.environment_signature_percent <= 100);
    }

    #[test]
    fn forced_fallback_is_seed_independent_and_uses_all_canonical_fragments() {
        let first =
            generate_with_options(RING_RADIUS, 0.4, settings(), 1, runtime_art_catalog(), true)
                .expect("the Ring7 fallback should validate");
        let second = generate_with_options(
            RING_RADIUS,
            0.4,
            settings(),
            u64::MAX,
            runtime_art_catalog(),
            true,
        )
        .expect("the Ring7 fallback should ignore seed state");

        assert!(first.used_fallback);
        assert_eq!(first.selected_candidate, None);
        assert_eq!(first.valid_candidates, 0);
        assert_eq!(
            first.validated.semantic_fingerprint,
            second.validated.semantic_fingerprint
        );
        assert_eq!(first.metrics, second.metrics);
    }

    #[test]
    fn validator_rejects_arbitrary_special_tags_on_shared_seam_closures() {
        let settings = settings();
        let layout = resolve_layout(RING_RADIUS, settings).expect("Ring7 layout should resolve");
        let recipe = Ring7Recipe {
            level_height: 0.4,
            layout,
            settings: validate_recipe_settings(settings).expect("Ring7 settings should validate"),
            art_catalog: runtime_art_catalog(),
            cave_vegetation: CaveVegetationSet::resolve(runtime_art_catalog(), "Ring7 test Caves")
                .expect("tracked cave vegetation should resolve"),
            _reject_candidates: false,
        };
        let mut fragment = recipe
            .construct_fragments(PatchBuildMode::CanonicalFallback)
            .expect("canonical Ring7 fragments should construct")
            .into_iter()
            .next()
            .expect("Ring7 must contain its center patch");
        let patch = PatchRecipeContext::resolve(&recipe.layout, fragment.patch_id)
            .expect("center patch context should resolve");
        let closure = fragment
            .volume
            .surfaces
            .iter()
            .find_map(|(position, metadata)| {
                super::super::seam::is_seam_closure_access(metadata.access).then_some(*position)
            })
            .expect("Ring7 must contain a closed shared-seam surface");
        fragment
            .volume
            .surfaces
            .get_mut(&closure)
            .expect("selected closure remains present")
            .access = super::super::volume::SurfaceAccess::SpecialMovement(
            hex_core::SpecialMovementRegion(17),
        );

        let issues = super::super::seam::validate_patch_walker_seams(&patch, &fragment.volume);
        assert!(issues
            .iter()
            .any(|issue| issue.detail.contains("expected exact seam closure")));
    }

    #[test]
    fn ring7_rejects_wrong_radius_level_height_and_roster_environment() {
        let error = generate(32, 0.4, settings(), PINNED_SEED, runtime_art_catalog())
            .expect_err("Ring7 radius is fixed");
        assert!(error.to_string().contains("radius 33"));

        let error = generate(
            RING_RADIUS,
            f32::NAN,
            settings(),
            PINNED_SEED,
            runtime_art_catalog(),
        )
        .expect_err("Ring7 level height must be finite");
        assert!(error.to_string().contains("positive and finite"));

        let mut wrong = settings().clone();
        let V3LayoutSettings::Ring7(ring) = &mut wrong.layout else {
            unreachable!("the shipped fixture remains Ring7");
        };
        ring.mountains.environment = V3EnvironmentSettings::TemperateGrassland;
        let error = generate(RING_RADIUS, 0.4, &wrong, PINNED_SEED, runtime_art_catalog())
            .expect_err("Ring7 environments are part of its fixed roster");
        assert!(error.to_string().contains("mountains requires the Frozen"));

        let mut wrong = settings().clone();
        let V3LayoutSettings::Ring7(ring) = &mut wrong.layout else {
            unreachable!("the shipped fixture remains Ring7");
        };
        set_liquid(
            &mut ring.center.edges.east,
            EdgeLiquidSettings::Outlet(EdgeLiquidPortSettings { width: 3 }),
        );
        set_liquid(
            &mut ring.waterfall.edges.west,
            EdgeLiquidSettings::Inlet(EdgeLiquidPortSettings { width: 3 }),
        );
        let error = generate(RING_RADIUS, 0.4, &wrong, PINNED_SEED, runtime_art_catalog())
            .expect_err("Ring7's directed liquid DAG is fixed");
        assert!(error
            .to_string()
            .contains("Waterfall(2)->Hills(0)->Caves(5)"));
    }

    fn set_liquid(edge: &mut PatchEdgeContractSettings, liquid: EdgeLiquidSettings) {
        let PatchEdgeContractSettings::Shared(shared) = edge else {
            panic!("the fixed Ring7 hydrology edge must remain shared");
        };
        shared.liquid = liquid;
    }

    fn settings() -> &'static ProceduralV3Settings {
        static SETTINGS: OnceLock<ProceduralV3Settings> = OnceLock::new();
        SETTINGS.get_or_init(|| {
            let map: MapSettings = ron::from_str(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/config/worlds/procedural-ring7.ron"
            )))
            .expect("the tracked Ring7 world should parse");
            let TerrainSettings::Procedural(ProceduralSettings::V3(settings)) = map.terrain else {
                panic!("the tracked Ring7 world should select procedural V3");
            };
            settings
        })
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
}
