//! Native V3 mountain geometry.
//!
//! Mountains occupy most of their patch with overlapping sharp peak masses. Their
//! outer skirts remain one-level walker terrain, while deliberately steep summit
//! cores are classified from the finished exact traversal graph. Two independently
//! routed, two-wide corridors provide a high pass and a lower bypass.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use hex_core::{HexCoord, Level, MapViewHint, SpecialMovementRegion, TilePos};

use super::composition::{compose_single_patch, GeneratedPatchPlan};
use super::layout::{resolve_layout, PatchId, ResolvedLayoutPlan};
use super::liquid::{LiquidBodyId, LiquidBodyPlan, LiquidFlowState, LiquidNode, LiquidPlan};
use super::local_frame::LocalPatchFrame;
use super::patch::{PatchBuildMode, PatchRecipeContext};
use super::seam::{shape_walker_seams, validate_patch_walker_seams};
use super::seed::SeedStream;
use super::selection::{
    run_recipe, CandidateAttemptError, CandidateContext, FallbackContext, RepairOutcome, V3Recipe,
    ValidatedWorldSelection, WorldValidation,
};
use super::traversal::OrdinaryGraph;
use super::volume::{
    FillMaterialRole, LevelInterval, NonSolidFill, SolidMass, SolidMaterialRole, SurfaceAccess,
    SurfaceMetadata, VolumeColumn, VolumeElement, VolumePlan,
};
use super::world::{
    FeaturePlan, GeneratedWorldPlan, InteriorPlan, ProtectedFeatureRoute, StructurePlan,
    WorldIssueCode, WorldValidationIssue,
};
use super::V3GenerationError;
use crate::settings::{
    ProceduralV3Settings, V3EnvironmentSettings, V3LayoutSettings, V3MountainsSettings,
    V3RecipeSettings,
};

const PARTY_START: &str = "party_start";
const HOSTILE_START: &str = "hostile_start";
const CONFLICT_CENTER: &str = "conflict_center";
const HIGH_PASS: &str = "high_pass";
const LOWER_BYPASS: &str = "lower_bypass";
const LOW_BYPASS_ANCHOR: &str = "low_bypass";
const MOUNTAIN_FALL_HEIGHT: Level = 3;

/// Deterministic measurements for one admitted Mountains plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MountainsMetrics {
    pub(crate) ordinary_surfaces: u32,
    pub(crate) special_surfaces: u32,
    pub(crate) mountain_surfaces: u32,
    pub(crate) mountain_coverage_percent: u32,
    pub(crate) accessible_mountain_surfaces: u32,
    pub(crate) reachable_elevation_levels: u32,
    pub(crate) relief: Level,
    pub(crate) peak_count: u8,
    pub(crate) cliff_edges: u32,
    pub(crate) high_pass_steps: u32,
    pub(crate) lower_bypass_steps: u32,
}

#[derive(Debug)]
struct MountainsRecipe {
    level_height: f32,
    layout: ResolvedLayoutPlan,
    settings: V3MountainsSettings,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct MountainStreams<'a> {
    orientation: SeedStream<'a>,
    peaks: SeedStream<'a>,
}

#[derive(Debug)]
struct MountainStream {
    nodes: BTreeMap<TilePos, LiquidNode>,
}

/// Runs the common eight-candidate selector for one native V3 Mountains world.
pub(crate) fn generate(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV3Settings,
    seed: u64,
) -> Result<ValidatedWorldSelection<MountainsMetrics>, V3GenerationError> {
    if !level_height.is_finite() || level_height <= 0.0 {
        return Err(V3GenerationError::RecipeContract(
            "Mountains level height must be positive and finite".to_owned(),
        ));
    }
    let mountain_settings = recipe_settings(settings)?;
    let layout = resolve_layout(grid_radius, settings)
        .map_err(|error| V3GenerationError::RecipeContract(error.to_string()))?;
    run_recipe(
        &MountainsRecipe {
            level_height,
            layout,
            settings: mountain_settings.clone(),
        },
        settings,
        grid_radius,
        seed,
    )
}

impl V3Recipe for MountainsRecipe {
    type Settings = ProceduralV3Settings;
    type Metrics = MountainsMetrics;
    type Score = (u32, u32, u32, u8);

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
            self.level_height,
            PatchBuildMode::Candidate {
                world_seed: context.seed,
                candidate: context.candidate,
            },
        )
        .map_err(CandidateAttemptError::Rejected)?;
        compose_single_patch(self.layout.clone(), fragment).map_err(|error| {
            CandidateAttemptError::Fatal(V3GenerationError::RecipeContract(format!(
                "Mountains single-patch composition failed: {error:?}"
            )))
        })
    }

    fn validate(
        &self,
        _settings: &Self::Settings,
        plan: &GeneratedWorldPlan,
    ) -> WorldValidation<Self::Metrics> {
        validate_mountains(plan, &self.settings)
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
            metrics.mountain_coverage_percent.abs_diff(62),
            metrics.relief.abs_diff(self.settings.relief),
            u32::MAX.saturating_sub(metrics.cliff_edges),
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
                "Mountains fallback radius disagrees with its resolved layout".to_owned(),
            ));
        }
        let patch = PatchRecipeContext::resolve(&self.layout, PatchId(0))?;
        let fragment = construct_patch(
            patch,
            &self.settings,
            self.level_height,
            PatchBuildMode::CanonicalFallback,
        )
        .map_err(|issues| {
            V3GenerationError::RecipeContract(
                issues
                    .into_iter()
                    .map(|issue| issue.detail)
                    .collect::<Vec<_>>()
                    .join("; "),
            )
        })?;
        compose_single_patch(self.layout.clone(), fragment).map_err(|error| {
            V3GenerationError::RecipeContract(format!(
                "Mountains fallback composition failed: {error:?}"
            ))
        })
    }
}

fn recipe_settings(
    settings: &ProceduralV3Settings,
) -> Result<&V3MountainsSettings, V3GenerationError> {
    let V3LayoutSettings::Single(patch) = &settings.layout else {
        return Err(V3GenerationError::RecipeUnavailable("Ring7"));
    };
    let V3RecipeSettings::Mountains(mountains) = &patch.recipe else {
        return Err(V3GenerationError::RecipeUnavailable(recipe_name(
            &patch.recipe,
        )));
    };
    if patch.environment != V3EnvironmentSettings::Frozen {
        return Err(V3GenerationError::RecipeContract(
            "Mountains requires the Frozen environment".to_owned(),
        ));
    }
    if !patch.overlays.is_empty() {
        return Err(V3GenerationError::RecipeContract(
            "Mountains overlays are not implemented yet".to_owned(),
        ));
    }
    Ok(mountains)
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

pub(crate) fn construct_patch(
    patch: PatchRecipeContext<'_>,
    settings: &V3MountainsSettings,
    level_height: f32,
    mode: PatchBuildMode,
) -> Result<GeneratedPatchPlan, Vec<WorldValidationIssue>> {
    let streams = mode.seed_streams(&patch);
    construct_patch_with_streams(
        patch,
        settings,
        level_height,
        streams.map(|streams| MountainStreams {
            orientation: streams.stage("mountains.orientation"),
            peaks: streams.stage("mountains.peaks"),
        }),
    )
}

fn construct_patch_with_streams(
    patch: PatchRecipeContext<'_>,
    settings: &V3MountainsSettings,
    level_height: f32,
    streams: Option<MountainStreams<'_>>,
) -> Result<GeneratedPatchPlan, Vec<WorldValidationIssue>> {
    let mask = patch.mask().clone();
    let orientation = streams.map_or(0, |streams| {
        u8::try_from(streams.orientation.sample(0) % 6).unwrap_or_default()
    });
    let landing_candidates = seam_landing_candidates(&patch);
    let closed_boundary = closed_boundary_coords(&patch, &landing_candidates);
    let route_mask: BTreeSet<_> = mask.difference(&closed_boundary).copied().collect();
    let (party_coord, hostile_coord) = opposing_landings(&landing_candidates, orientation)?;
    let (high_control, bypass_control) =
        route_controls(&route_mask, party_coord, hostile_coord, orientation)?;
    let high_centerline = route_via(
        &route_mask,
        party_coord,
        high_control,
        hostile_coord,
        &BTreeSet::new(),
    )
    .ok_or_else(|| vec![recipe_issue("Mountains could not route its high pass")])?;
    let landing_overlap: usize = if patch.layout().kind.is_composite() {
        1
    } else {
        2
    };
    let high_interior: BTreeSet<_> = high_centerline
        .iter()
        .copied()
        .skip(landing_overlap)
        .take(
            high_centerline
                .len()
                .saturating_sub(landing_overlap.saturating_mul(2)),
        )
        .collect();
    let bypass_controls = if patch.layout().kind.is_composite() {
        ordered_bypass_controls(
            &route_mask,
            party_coord,
            hostile_coord,
            orientation,
            high_control,
        )
    } else {
        vec![bypass_control]
    };
    let bypass_centerline = bypass_controls
        .into_iter()
        .filter(|control| !high_interior.contains(control))
        .find_map(|control| {
            route_via(
                &route_mask,
                party_coord,
                control,
                hostile_coord,
                &high_interior,
            )
            .filter(|route| {
                route
                    .iter()
                    .filter(|coord| high_centerline.contains(coord))
                    .count()
                    <= 2
            })
        })
        .ok_or_else(|| vec![recipe_issue("Mountains could not route its lower bypass")])?;
    let high_footprint = two_wide_footprint(&route_mask, &high_centerline, orientation, true);
    let bypass_footprint = two_wide_footprint(&route_mask, &bypass_centerline, orientation, false);
    let route_cells: BTreeSet<_> = high_footprint.union(&bypass_footprint).copied().collect();
    let seam_approaches = patch.protected_approaches();
    let seam_buffer = mask
        .iter()
        .copied()
        .filter(|coord| {
            seam_approaches
                .iter()
                .any(|approach| approach.distance(*coord) <= 3)
        })
        .collect::<BTreeSet<_>>();
    let excluded: BTreeSet<_> = route_cells.union(&seam_buffer).copied().collect();
    let peaks = select_peaks(
        &mask,
        usize::from(settings.peak_count),
        &excluded,
        streams.map(|streams| streams.peaks),
    )?;

    let mut surface_by_coord = BTreeMap::new();
    for coord in &mask {
        let rise = peaks
            .iter()
            .map(|peak| mountain_rise(*peak, *coord, settings.relief))
            .max()
            .unwrap_or_default();
        surface_by_coord.insert(*coord, settings.base_level.saturating_add(rise));
    }
    apply_route(
        &mut surface_by_coord,
        &bypass_centerline,
        &bypass_footprint,
        settings.base_level,
        4,
    );
    apply_route(
        &mut surface_by_coord,
        &high_centerline,
        &high_footprint,
        settings.base_level,
        settings.relief / 2,
    );
    let authored_surface_by_coord = surface_by_coord.clone();
    let seam_shape = shape_walker_seams(&patch, &mut surface_by_coord)?;
    for (coord, authored_level) in authored_surface_by_coord {
        let in_peak_core = peaks.iter().any(|peak| peak.distance(coord) <= 2);
        if in_peak_core && !seam_approaches.contains(&coord) && !route_cells.contains(&coord) {
            surface_by_coord.insert(coord, authored_level);
        }
    }
    let mountain_streams = build_mountain_streams(
        &patch,
        &surface_by_coord,
        &peaks,
        &route_cells,
        settings.base_level,
    )?;
    let stream_nodes_by_coord = mountain_streams
        .iter()
        .enumerate()
        .flat_map(|(body, stream)| {
            stream
                .nodes
                .iter()
                .map(move |(position, node)| (position.coord, (body, *position, *node)))
        })
        .collect::<BTreeMap<_, _>>();

    let mut columns = BTreeMap::new();
    let mut surfaces = BTreeMap::new();
    for (coord, level) in &surface_by_coord {
        if let Some((_, position, node)) = stream_nodes_by_coord.get(coord).copied() {
            let crossing = route_cells.contains(coord).then_some(*level);
            let (column, bed, crossing) = mountain_stream_column(position, node, crossing);
            columns.insert(*coord, column);
            surfaces.insert(
                bed,
                SurfaceMetadata {
                    access: SurfaceAccess::NonStandable,
                    interior: None,
                },
            );
            if let Some(crossing) = crossing {
                surfaces.insert(
                    crossing,
                    SurfaceMetadata {
                        access: SurfaceAccess::Ordinary,
                        interior: None,
                    },
                );
            }
        } else {
            let exposed = is_exposed_stone(*coord, *level, &surface_by_coord, settings);
            columns.insert(*coord, mountain_column(*level, exposed));
            surfaces.insert(
                TilePos::new(*coord, *level),
                SurfaceMetadata {
                    access: SurfaceAccess::Ordinary,
                    interior: None,
                },
            );
        }
    }
    let mut volume = VolumePlan {
        mask: mask.clone(),
        columns,
        surfaces,
    };
    seam_shape.apply(&mut volume)?;
    let party = exact_position(&surface_by_coord, party_coord)?;
    let hostile = exact_position(&surface_by_coord, hostile_coord)?;
    let initial_graph = OrdinaryGraph::from_volume(&volume, None);
    let reachable = initial_graph.distances_from(party);
    let special_region = SpecialMovementRegion(0);
    for (position, metadata) in &mut volume.surfaces {
        if metadata.access == SurfaceAccess::Ordinary && !reachable.contains_key(position) {
            metadata.access = SurfaceAccess::SpecialMovement(special_region);
        }
    }

    let high_route = route_membership(&high_centerline, &high_footprint, &surface_by_coord)?;
    let bypass_route = route_membership(&bypass_centerline, &bypass_footprint, &surface_by_coord)?;
    let conflict_coord = high_centerline
        .get(high_centerline.len() / 2)
        .copied()
        .ok_or_else(|| vec![recipe_issue("Mountains high pass has no midpoint")])?;
    let bypass_coord = bypass_centerline
        .get(bypass_centerline.len() / 2)
        .copied()
        .ok_or_else(|| vec![recipe_issue("Mountains lower bypass has no midpoint")])?;
    let anchors = BTreeMap::from([
        (PARTY_START.to_owned(), party),
        (HOSTILE_START.to_owned(), hostile),
        (
            CONFLICT_CENTER.to_owned(),
            exact_position(&surface_by_coord, conflict_coord)?,
        ),
        (
            HIGH_PASS.to_owned(),
            exact_position(&surface_by_coord, conflict_coord)?,
        ),
        (
            LOW_BYPASS_ANCHOR.to_owned(),
            exact_position(&surface_by_coord, bypass_coord)?,
        ),
    ]);
    let features = FeaturePlan {
        by_id: BTreeMap::new(),
        protected_routes: BTreeMap::from([
            (HIGH_PASS.to_owned(), high_route),
            (LOWER_BYPASS.to_owned(), bypass_route),
        ]),
        clearings: BTreeMap::new(),
    };
    let biome_regions = volume
        .surfaces
        .keys()
        .copied()
        .map(|surface| (surface, patch.biome_region()))
        .collect();
    let view_hint =
        mountain_view_hint(&mask, &surface_by_coord, patch.grid_radius(), level_height)?;

    let fragment = GeneratedPatchPlan {
        patch_id: patch.id,
        volume,
        liquids: LiquidPlan {
            bodies: mountain_streams
                .into_iter()
                .enumerate()
                .map(|(index, stream)| {
                    (
                        LiquidBodyId(u32::try_from(index).unwrap_or(u32::MAX)),
                        LiquidBodyPlan {
                            material: FillMaterialRole::Water,
                            nodes: stream.nodes,
                        },
                    )
                })
                .collect(),
        },
        features,
        structures: StructurePlan::default(),
        blockers: BTreeSet::new(),
        lights: BTreeMap::new(),
        biome_regions,
        interiors: InteriorPlan::default(),
        anchors,
        view_hint,
    };
    let mut issues = validate_patch_walker_seams(&patch, &fragment.volume);
    issues.extend(
        fragment
            .validate_against(patch.layout())
            .into_iter()
            .map(|issue| {
                recipe_issue(format!(
                    "Mountains patch {:?} failed {:?}: {}",
                    issue.patch, issue.code, issue.detail
                ))
            }),
    );
    if issues.is_empty() {
        Ok(fragment)
    } else {
        Err(issues)
    }
}

fn seam_landing_candidates(patch: &PatchRecipeContext<'_>) -> BTreeSet<HexCoord> {
    let protected = patch.protected_approaches();
    if protected.is_empty() {
        patch.mask().clone()
    } else {
        protected
    }
}

fn closed_boundary_coords(
    patch: &PatchRecipeContext<'_>,
    open: &BTreeSet<HexCoord>,
) -> BTreeSet<HexCoord> {
    patch
        .shared_edges()
        .flat_map(|edge| edge.boundary_pairs().into_iter().map(|(local, _)| local))
        .filter(|coord| !open.contains(coord))
        .collect()
}

fn opposing_landings(
    mask: &BTreeSet<HexCoord>,
    orientation: u8,
) -> Result<(HexCoord, HexCoord), Vec<WorldValidationIssue>> {
    let party = mask
        .iter()
        .copied()
        .min_by_key(|coord| (axis_value(*coord, orientation), *coord));
    let hostile = mask
        .iter()
        .copied()
        .max_by_key(|coord| (axis_value(*coord, orientation), *coord));
    match (party, hostile) {
        (Some(party), Some(hostile)) if party != hostile => Ok((party, hostile)),
        _ => Err(vec![recipe_issue(
            "Mountains patch cannot fit opposing landings",
        )]),
    }
}

fn route_controls(
    mask: &BTreeSet<HexCoord>,
    start: HexCoord,
    end: HexCoord,
    orientation: u8,
) -> Result<(HexCoord, HexCoord), Vec<WorldValidationIssue>> {
    let start_axis = axis_value(start, orientation);
    let end_axis = axis_value(end, orientation);
    let midpoint = start_axis.saturating_add(end_axis) / 2;
    let half_band = start_axis.abs_diff(end_axis).saturating_div(5).max(1);
    let side_turn = orientation.saturating_add(2) % 6;
    let candidates: Vec<_> = mask
        .iter()
        .copied()
        .filter(|coord| axis_value(*coord, orientation).abs_diff(midpoint) <= half_band)
        .filter(|coord| coord.distance(start) >= 3 && coord.distance(end) >= 3)
        .collect();
    let high = candidates
        .iter()
        .copied()
        .max_by_key(|coord| (axis_value(*coord, side_turn), *coord));
    let bypass = candidates
        .iter()
        .copied()
        .min_by_key(|coord| (axis_value(*coord, side_turn), *coord));
    match (high, bypass) {
        (Some(high), Some(bypass)) if high != bypass => Ok((high, bypass)),
        _ => Err(vec![recipe_issue(
            "Mountains patch cannot separate high-pass and bypass controls",
        )]),
    }
}

fn ordered_bypass_controls(
    mask: &BTreeSet<HexCoord>,
    start: HexCoord,
    end: HexCoord,
    orientation: u8,
    high_control: HexCoord,
) -> Vec<HexCoord> {
    let start_axis = axis_value(start, orientation);
    let end_axis = axis_value(end, orientation);
    let midpoint = start_axis.saturating_add(end_axis) / 2;
    let half_band = start_axis.abs_diff(end_axis).saturating_div(5).max(1);
    let side_turn = orientation.saturating_add(2) % 6;
    let mut candidates = mask
        .iter()
        .copied()
        .filter(|coord| axis_value(*coord, orientation).abs_diff(midpoint) <= half_band)
        .filter(|coord| coord.distance(start) >= 3 && coord.distance(end) >= 3)
        .filter(|coord| *coord != high_control)
        .collect::<Vec<_>>();
    candidates.sort_by_key(|coord| (axis_value(*coord, side_turn), *coord));
    candidates
}

fn route_via(
    mask: &BTreeSet<HexCoord>,
    start: HexCoord,
    control: HexCoord,
    end: HexCoord,
    forbidden: &BTreeSet<HexCoord>,
) -> Option<Vec<HexCoord>> {
    let first = shortest_path(mask, start, control, forbidden)?;
    let mut second_forbidden = forbidden.clone();
    second_forbidden.extend(first.iter().copied().filter(|coord| *coord != control));
    let second = shortest_path(mask, control, end, &second_forbidden)?;
    let mut combined = first;
    combined.extend(second.into_iter().skip(1));
    (combined.iter().copied().collect::<BTreeSet<_>>().len() == combined.len()).then_some(combined)
}

fn shortest_path(
    mask: &BTreeSet<HexCoord>,
    start: HexCoord,
    end: HexCoord,
    forbidden: &BTreeSet<HexCoord>,
) -> Option<Vec<HexCoord>> {
    if !mask.contains(&start) || !mask.contains(&end) {
        return None;
    }
    let mut previous = BTreeMap::<HexCoord, Option<HexCoord>>::from([(start, None)]);
    let mut frontier = VecDeque::from([start]);
    while let Some(current) = frontier.pop_front() {
        if current == end {
            break;
        }
        let mut neighbors = current.neighbors();
        neighbors.sort_by_key(|coord| (coord.distance(end), *coord));
        for neighbor in neighbors {
            if !mask.contains(&neighbor)
                || (forbidden.contains(&neighbor) && neighbor != end)
                || previous.contains_key(&neighbor)
            {
                continue;
            }
            previous.insert(neighbor, Some(current));
            frontier.push_back(neighbor);
        }
    }
    if !previous.contains_key(&end) {
        return None;
    }
    let mut reversed = vec![end];
    let mut current = end;
    while current != start {
        let predecessor = previous.get(&current).copied().flatten()?;
        reversed.push(predecessor);
        current = predecessor;
    }
    reversed.reverse();
    Some(reversed)
}

fn build_mountain_streams(
    patch: &PatchRecipeContext<'_>,
    levels: &BTreeMap<HexCoord, Level>,
    peaks: &[HexCoord],
    route_cells: &BTreeSet<HexCoord>,
    base_level: Level,
) -> Result<Vec<MountainStream>, Vec<WorldValidationIssue>> {
    let boundary = patch
        .mask()
        .iter()
        .copied()
        .filter(|coord| {
            coord
                .neighbors()
                .into_iter()
                .any(|neighbor| !patch.mask().contains(&neighbor))
        })
        .collect::<BTreeSet<_>>();
    if patch.layout().kind == super::layout::LayoutKind::Single {
        let mut outlets = boundary
            .iter()
            .copied()
            .filter(|coord| !route_cells.contains(coord))
            .collect::<Vec<_>>();
        outlets.sort_unstable_by_key(|coord| {
            (levels.get(coord).copied().unwrap_or(Level::MAX), *coord)
        });
        let outlet_level = base_level.saturating_sub(1);
        for outlet in outlets.into_iter().take(12) {
            if let Some(stream) = build_mountain_stream(
                patch.mask(),
                levels,
                peaks,
                route_cells,
                &BTreeSet::new(),
                &boundary,
                outlet,
                outlet_level,
            ) {
                return Ok(vec![stream]);
            }
        }
        return Err(vec![recipe_issue(
            "Mountains could not route a peak-fed stream to the world boundary",
        )]);
    }

    let mut outlets = Vec::new();
    for edge in patch.shared_edges() {
        let Some((is_source, port)) = edge.liquid_port() else {
            continue;
        };
        if !is_source {
            return Err(vec![recipe_issue(
                "Mountains supports source/outlet liquid contracts, not incoming liquid",
            )]);
        }
        let endpoint_level = edge
            .preferred_level()
            .saturating_sub(1)
            .max(edge.contract.elevation.min)
            .min(edge.contract.elevation.max);
        outlets.extend(port.lanes.iter().map(|(local, _)| (*local, endpoint_level)));
    }
    if outlets.is_empty() {
        return Ok(Vec::new());
    }
    outlets.sort_unstable();
    let reserved_outlets = outlets
        .iter()
        .map(|(coord, _)| *coord)
        .collect::<BTreeSet<_>>();
    let mut used = BTreeSet::new();
    let mut streams = Vec::new();
    for (outlet, outlet_level) in outlets {
        let other_outlets = reserved_outlets
            .iter()
            .copied()
            .filter(|coord| *coord != outlet)
            .collect::<BTreeSet<_>>();
        let Some(stream) = build_mountain_stream(
            patch.mask(),
            levels,
            peaks,
            route_cells,
            &used.union(&other_outlets).copied().collect(),
            &boundary,
            outlet,
            outlet_level,
        ) else {
            return Err(vec![recipe_issue(format!(
                "Mountains could not route a peak-fed stream to liquid outlet {outlet:?}"
            ))]);
        };
        used.extend(stream.nodes.keys().map(|position| position.coord));
        streams.push(stream);
    }
    Ok(streams)
}

#[expect(
    clippy::too_many_arguments,
    reason = "stream routing keeps each immutable geometry contract explicit"
)]
fn build_mountain_stream(
    mask: &BTreeSet<HexCoord>,
    levels: &BTreeMap<HexCoord, Level>,
    peaks: &[HexCoord],
    route_cells: &BTreeSet<HexCoord>,
    reserved: &BTreeSet<HexCoord>,
    boundary: &BTreeSet<HexCoord>,
    outlet: HexCoord,
    outlet_level: Level,
) -> Option<MountainStream> {
    let desired_land_level = outlet_level
        .saturating_add(MOUNTAIN_FALL_HEIGHT)
        .saturating_add(1);
    let mut sources = levels
        .iter()
        .filter_map(|(coord, level)| {
            (!route_cells.contains(coord)
                && !reserved.contains(coord)
                && !boundary.contains(coord)
                && !peaks.contains(coord)
                && *level >= desired_land_level
                && coord.distance(outlet) >= 5)
                .then_some((*coord, *level))
        })
        .collect::<Vec<_>>();
    sources.sort_unstable_by_key(|(coord, level)| {
        (
            level.abs_diff(desired_land_level),
            peaks
                .iter()
                .map(|peak| peak.distance(*coord))
                .min()
                .unwrap_or(u32::MAX),
            u32::MAX.saturating_sub(coord.distance(outlet)),
            *coord,
        )
    });
    let mut forbidden = reserved.clone();
    forbidden.extend(peaks.iter().copied());
    forbidden.remove(&outlet);
    for (source, _land_level) in sources.into_iter().take(24) {
        let Some(path) = shortest_path(mask, source, outlet, &forbidden) else {
            continue;
        };
        let edges = path.len().saturating_sub(1);
        if edges < 2 {
            continue;
        }
        let source_level = outlet_level.saturating_add(MOUNTAIN_FALL_HEIGHT);
        let water_levels = std::iter::once(source_level)
            .chain(std::iter::repeat_n(outlet_level, edges))
            .collect::<Vec<_>>();
        if path.iter().zip(&water_levels).any(|(coord, water)| {
            levels
                .get(coord)
                .is_none_or(|surface| *surface < water.saturating_add(1))
        }) {
            continue;
        }
        let positions = path
            .iter()
            .copied()
            .zip(water_levels)
            .map(|(coord, level)| TilePos::new(coord, level))
            .collect::<Vec<_>>();
        let nodes = positions
            .iter()
            .copied()
            .enumerate()
            .map(|(index, position)| {
                let downstream = positions.get(index.saturating_add(1)).copied();
                let state = downstream.map_or(LiquidFlowState::Still, |downstream| match position
                    .level
                    .saturating_sub(downstream.level)
                {
                    0 => LiquidFlowState::Current,
                    1 => LiquidFlowState::Rapid,
                    _ => LiquidFlowState::Fall,
                });
                (position, LiquidNode { state, downstream })
            })
            .collect();
        return Some(MountainStream { nodes });
    }
    None
}

fn two_wide_footprint(
    mask: &BTreeSet<HexCoord>,
    centerline: &[HexCoord],
    orientation: u8,
    positive_side: bool,
) -> BTreeSet<HexCoord> {
    let centerline_set: BTreeSet<_> = centerline.iter().copied().collect();
    let side_turn = orientation.saturating_add(2) % 6;
    let mut footprint = centerline_set.clone();
    for center in centerline {
        let shoulder = if positive_side {
            center
                .neighbors()
                .into_iter()
                .filter(|coord| mask.contains(coord) && !centerline_set.contains(coord))
                .max_by_key(|coord| (axis_value(*coord, side_turn), *coord))
        } else {
            center
                .neighbors()
                .into_iter()
                .filter(|coord| mask.contains(coord) && !centerline_set.contains(coord))
                .min_by_key(|coord| (axis_value(*coord, side_turn), *coord))
        };
        if let Some(shoulder) = shoulder {
            footprint.insert(shoulder);
        }
    }
    footprint
}

fn select_peaks(
    mask: &BTreeSet<HexCoord>,
    count: usize,
    excluded: &BTreeSet<HexCoord>,
    stream: Option<SeedStream<'_>>,
) -> Result<Vec<HexCoord>, Vec<WorldValidationIssue>> {
    let mut candidates: Vec<_> = mask
        .iter()
        .copied()
        .filter(|coord| !excluded.contains(coord))
        .filter(|coord| {
            coord
                .neighbors()
                .into_iter()
                .all(|next| mask.contains(&next))
        })
        .collect();
    candidates.sort_by_key(|coord| {
        (
            stream.map_or(coord.distance(HexCoord::ORIGIN).into(), |stream| {
                stream.sample_coord(*coord, 0)
            }),
            *coord,
        )
    });
    let mut peaks = Vec::new();
    for separation in [4_u32, 3, 2] {
        for candidate in &candidates {
            if peaks.contains(candidate)
                || peaks
                    .iter()
                    .any(|peak: &HexCoord| peak.distance(*candidate) < separation)
            {
                continue;
            }
            peaks.push(*candidate);
            if peaks.len() == count {
                return Ok(peaks);
            }
        }
    }
    Err(vec![recipe_issue(format!(
        "Mountains patch placed {} separated peaks; expected {count}",
        peaks.len()
    ))])
}

fn mountain_rise(peak: HexCoord, coord: HexCoord, relief: Level) -> Level {
    let distance = i32::try_from(peak.distance(coord)).unwrap_or(i32::MAX);
    let sharp_core = relief.saturating_sub(distance.saturating_mul(8)).max(0);
    let outer_radius = relief.saturating_div(3).saturating_add(4);
    let walker_skirt = outer_radius.saturating_sub(distance).clamp(0, 12);
    sharp_core.max(walker_skirt)
}

fn apply_route(
    levels: &mut BTreeMap<HexCoord, Level>,
    centerline: &[HexCoord],
    footprint: &BTreeSet<HexCoord>,
    base: Level,
    rise_cap: Level,
) {
    if centerline.is_empty() {
        return;
    }
    let last = centerline.len().saturating_sub(1);
    let center_levels: Vec<_> = centerline
        .iter()
        .enumerate()
        .map(|(index, coord)| {
            let from_start = i32::try_from(index).unwrap_or(i32::MAX);
            let from_end = i32::try_from(last.saturating_sub(index)).unwrap_or(i32::MAX);
            (
                *coord,
                base.saturating_add(from_start.min(from_end).min(rise_cap)),
            )
        })
        .collect();
    for (coord, level) in &center_levels {
        levels.insert(*coord, *level);
    }
    for coord in footprint {
        if centerline.contains(coord) {
            continue;
        }
        if let Some((_, level)) = center_levels
            .iter()
            .min_by_key(|(center, _)| (center.distance(*coord), *center))
        {
            levels.insert(*coord, *level);
        }
    }
}

fn is_exposed_stone(
    coord: HexCoord,
    level: Level,
    levels: &BTreeMap<HexCoord, Level>,
    settings: &V3MountainsSettings,
) -> bool {
    let snow_capped = level
        >= settings
            .base_level
            .saturating_add(settings.relief.saturating_mul(3) / 4);
    let steep = coord.neighbors().into_iter().any(|neighbor| {
        levels
            .get(&neighbor)
            .is_some_and(|other| level.abs_diff(*other) >= 2)
    });
    steep && !snow_capped
}

fn mountain_column(surface: Level, exposed: bool) -> VolumeColumn {
    let mut elements = vec![VolumeElement::Solid(SolidMass {
        levels: LevelInterval::new(0, 1),
        material: SolidMaterialRole::Bedrock,
        cutaway_for: None,
    })];
    if exposed {
        elements.push(VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(1, surface.saturating_add(1)),
            material: SolidMaterialRole::Stone,
            cutaway_for: None,
        }));
    } else {
        elements.push(VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(1, surface),
            material: SolidMaterialRole::Stone,
            cutaway_for: None,
        }));
        elements.push(VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(surface, surface.saturating_add(1)),
            material: SolidMaterialRole::Snow,
            cutaway_for: None,
        }));
    }
    VolumeColumn { elements }
}

fn mountain_stream_column(
    position: TilePos,
    node: LiquidNode,
    crossing_level: Option<Level>,
) -> (VolumeColumn, TilePos, Option<TilePos>) {
    let (bed_level, fill_bottom) = if node.state == LiquidFlowState::Fall {
        node.downstream.map_or_else(
            || {
                (
                    position.level.saturating_sub(2),
                    position.level.saturating_sub(1),
                )
            },
            |downstream| (downstream.level.saturating_sub(1), downstream.level),
        )
    } else {
        (
            position.level.saturating_sub(2),
            position.level.saturating_sub(1),
        )
    };
    let mut elements = vec![
        VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(0, 1),
            material: SolidMaterialRole::Bedrock,
            cutaway_for: None,
        }),
        VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(1, bed_level),
            material: SolidMaterialRole::Stone,
            cutaway_for: None,
        }),
        VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(bed_level, bed_level.saturating_add(1)),
            material: SolidMaterialRole::Gravel,
            cutaway_for: None,
        }),
        VolumeElement::Fill(NonSolidFill {
            levels: LevelInterval::new(fill_bottom, position.level.saturating_add(1)),
            material: FillMaterialRole::Water,
        }),
    ];
    if let Some(crossing_level) = crossing_level {
        elements.push(VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(crossing_level, crossing_level.saturating_add(1)),
            material: SolidMaterialRole::Metal,
            cutaway_for: None,
        }));
        (
            VolumeColumn { elements },
            TilePos::new(position.coord, bed_level),
            Some(TilePos::new(position.coord, crossing_level)),
        )
    } else {
        (
            VolumeColumn { elements },
            TilePos::new(position.coord, bed_level),
            None,
        )
    }
}

fn exact_position(
    levels: &BTreeMap<HexCoord, Level>,
    coord: HexCoord,
) -> Result<TilePos, Vec<WorldValidationIssue>> {
    levels
        .get(&coord)
        .copied()
        .map(|level| TilePos::new(coord, level))
        .ok_or_else(|| {
            vec![recipe_issue(format!(
                "Mountains surface is missing at {coord:?}"
            ))]
        })
}

fn route_membership(
    centerline: &[HexCoord],
    footprint: &BTreeSet<HexCoord>,
    levels: &BTreeMap<HexCoord, Level>,
) -> Result<ProtectedFeatureRoute, Vec<WorldValidationIssue>> {
    let centerline = centerline
        .iter()
        .copied()
        .map(|coord| exact_position(levels, coord))
        .collect::<Result<Vec<_>, _>>()?;
    let surfaces = footprint
        .iter()
        .copied()
        .map(|coord| exact_position(levels, coord))
        .collect::<Result<BTreeSet<_>, _>>()?;
    Ok(ProtectedFeatureRoute {
        centerline,
        surfaces,
    })
}

pub(crate) fn validate_mountains(
    plan: &GeneratedWorldPlan,
    settings: &V3MountainsSettings,
) -> WorldValidation<MountainsMetrics> {
    validate_mountains_inner(
        plan,
        settings,
        settings.base_level.saturating_add(settings.relief / 2),
        1,
    )
}

pub(crate) fn validate_patch(
    patch: PatchRecipeContext<'_>,
    fragment: &GeneratedPatchPlan,
    settings: &V3MountainsSettings,
) -> WorldValidation<MountainsMetrics> {
    let seam_approaches = patch.protected_approaches();
    let available_rise = fragment
        .features
        .protected_routes
        .get(HIGH_PASS)
        .and_then(|route| {
            route
                .centerline
                .iter()
                .map(|position| {
                    seam_approaches
                        .iter()
                        .map(|approach| approach.distance(position.coord))
                        .min()
                        .unwrap_or(u32::MAX)
                })
                .max()
        })
        .and_then(|rise| i32::try_from(rise).ok())
        .unwrap_or(settings.relief / 2)
        .min(settings.relief / 2);
    let frame =
        match LocalPatchFrame::resolve(patch.mask(), patch.layout().kind, patch.grid_radius()) {
            Ok(frame) => frame,
            Err(error) => {
                return WorldValidation::Invalid(vec![recipe_issue(format!(
                    "Mountains validation frame failed: {error}"
                ))]);
            }
        };
    let mut local = match frame.canonical_local_world(fragment) {
        Ok(plan) => plan,
        Err(error) => {
            return WorldValidation::Invalid(vec![recipe_issue(format!(
                "Mountains validation projection failed: {error}"
            ))]);
        }
    };
    local.layout.grid_radius = local
        .layout
        .footprint
        .iter()
        .map(|coord| HexCoord::ORIGIN.distance(*coord))
        .max()
        .unwrap_or_default();
    validate_mountains_inner(
        &local,
        settings,
        settings.base_level.saturating_add(available_rise),
        patch
            .shared_edges()
            .filter_map(|edge| {
                edge.liquid_port()
                    .and_then(|(is_source, port)| is_source.then_some(port.lanes.len()))
            })
            .sum(),
    )
}

fn validate_mountains_inner(
    plan: &GeneratedWorldPlan,
    settings: &V3MountainsSettings,
    required_saddle: Level,
    expected_streams: usize,
) -> WorldValidation<MountainsMetrics> {
    let mut issues = plan.validate();
    validate_mountain_streams(plan, expected_streams, &mut issues);
    let ordinary = OrdinaryGraph::from_volume(&plan.volume, Some(&plan.blockers));
    let Some(party) = plan.anchors.get(PARTY_START).copied() else {
        issues.push(recipe_issue("Mountains is missing party_start"));
        return WorldValidation::Invalid(issues);
    };
    let Some(hostile) = plan.anchors.get(HOSTILE_START).copied() else {
        issues.push(recipe_issue("Mountains is missing hostile_start"));
        return WorldValidation::Invalid(issues);
    };
    let distances = ordinary.distances_from(party);
    let Some(lower_bypass_steps) = distances.get(&hostile).copied() else {
        issues.push(recipe_issue(
            "Mountains actor anchors are not connected by ordinary movement",
        ));
        return WorldValidation::Invalid(issues);
    };
    let Some(high_pass) = plan.features.protected_routes.get(HIGH_PASS) else {
        issues.push(recipe_issue("Mountains is missing its high pass"));
        return WorldValidation::Invalid(issues);
    };
    let Some(lower_bypass) = plan.features.protected_routes.get(LOWER_BYPASS) else {
        issues.push(recipe_issue("Mountains is missing its lower bypass"));
        return WorldValidation::Invalid(issues);
    };
    validate_route(HIGH_PASS, high_pass, &ordinary, &mut issues);
    validate_route(LOWER_BYPASS, lower_bypass, &ordinary, &mut issues);
    let shared_centerline = high_pass
        .centerline
        .iter()
        .filter(|position| lower_bypass.centerline.contains(position))
        .count();
    if shared_centerline > 2 {
        issues.push(recipe_issue(format!(
            "Mountains routes share {shared_centerline} centerline surfaces; only their landings may overlap"
        )));
    }
    let high_peak = high_pass
        .centerline
        .iter()
        .map(|position| position.level)
        .max()
        .unwrap_or_default();
    if high_peak < required_saddle {
        issues.push(recipe_issue(format!(
            "Mountains high pass reaches {high_peak}; expected saddle level {required_saddle}"
        )));
    }
    let bypass_peak = lower_bypass
        .centerline
        .iter()
        .map(|position| position.level)
        .max()
        .unwrap_or_default();
    if bypass_peak > settings.base_level.saturating_add(4) {
        issues.push(recipe_issue(
            "Mountains lower bypass rises above four levels",
        ));
    }

    let all_positions: Vec<_> = plan.volume.surfaces.keys().copied().collect();
    let mountain_surfaces = all_positions
        .iter()
        .filter(|position| position.level > settings.base_level)
        .count();
    let mountain_coverage_percent = percent(mountain_surfaces, all_positions.len());
    if mountain_coverage_percent < 52 {
        issues.push(recipe_issue(format!(
            "Mountains coverage is {mountain_coverage_percent}%; expected at least 52%"
        )));
    }
    let accessible_mountain_surfaces = distances
        .keys()
        .filter(|position| position.level > settings.base_level)
        .count();
    let accessible_mountain_percent = percent(accessible_mountain_surfaces, mountain_surfaces);
    if accessible_mountain_percent < 60 {
        issues.push(recipe_issue(format!(
            "Mountains exposes {accessible_mountain_percent}% of its raised standable surfaces to ordinary movement; expected at least 60%"
        )));
    }
    let max_level = all_positions
        .iter()
        .map(|position| position.level)
        .max()
        .unwrap_or_default();
    let relief = max_level.saturating_sub(settings.base_level);
    if relief != settings.relief {
        issues.push(recipe_issue(format!(
            "Mountains relief is {relief}; expected {}",
            settings.relief
        )));
    }
    let realized_peaks = all_positions
        .iter()
        .filter(|position| position.level == settings.base_level.saturating_add(settings.relief))
        .count();
    if realized_peaks < usize::from(settings.peak_count) {
        issues.push(recipe_issue(format!(
            "Mountains realizes {realized_peaks} summit cells; expected at least {}",
            settings.peak_count
        )));
    }
    let snow_cap_level = settings
        .base_level
        .saturating_add(settings.relief.saturating_mul(3) / 4);
    if all_positions
        .iter()
        .filter(|position| position.level >= snow_cap_level)
        .any(|position| {
            surface_material_at(&plan.volume, *position) != Some(SolidMaterialRole::Snow)
        })
    {
        issues.push(recipe_issue(
            "Mountains highest elevation band is not completely snow-capped",
        ));
    }
    let cliff_edges = count_cliff_edges(&plan.volume);
    if cliff_edges == 0 {
        issues.push(recipe_issue("Mountains contains no deliberate cliff edges"));
    }
    validate_shared_approaches(plan, &ordinary, &mut issues);
    if !issues.is_empty() {
        return WorldValidation::Invalid(issues);
    }
    let reachable_levels: BTreeSet<_> = distances.keys().map(|position| position.level).collect();
    let special_surfaces = plan
        .volume
        .surfaces
        .values()
        .filter(|metadata| matches!(metadata.access, SurfaceAccess::SpecialMovement(_)))
        .count();
    WorldValidation::Valid(MountainsMetrics {
        ordinary_surfaces: count_u32(ordinary.len()),
        special_surfaces: count_u32(special_surfaces),
        mountain_surfaces: count_u32(mountain_surfaces),
        mountain_coverage_percent,
        accessible_mountain_surfaces: count_u32(accessible_mountain_surfaces),
        reachable_elevation_levels: count_u32(reachable_levels.len()),
        relief,
        peak_count: settings.peak_count,
        cliff_edges,
        high_pass_steps: count_u32(high_pass.centerline.len().saturating_sub(1)),
        lower_bypass_steps,
    })
}

fn validate_mountain_streams(
    plan: &GeneratedWorldPlan,
    expected_streams: usize,
    issues: &mut Vec<WorldValidationIssue>,
) {
    if plan.liquids.bodies.len() != expected_streams {
        issues.push(recipe_issue(format!(
            "Mountains has {} peak-fed stream bodies; expected {expected_streams}",
            plan.liquids.bodies.len()
        )));
        return;
    }
    for (body_id, body) in &plan.liquids.bodies {
        if body.material != FillMaterialRole::Water {
            issues.push(recipe_issue(format!(
                "Mountains stream {body_id:?} is not water"
            )));
        }
        let fall_nodes = body
            .nodes
            .iter()
            .filter(|(_, node)| node.state == LiquidFlowState::Fall)
            .collect::<Vec<_>>();
        if fall_nodes.len() != 1 {
            issues.push(recipe_issue(format!(
                "Mountains stream {body_id:?} has {} waterfall nodes; expected one",
                fall_nodes.len()
            )));
        } else if fall_nodes.first().is_some_and(|(position, node)| {
            node.downstream.is_none_or(|downstream| {
                position.level.saturating_sub(downstream.level) != MOUNTAIN_FALL_HEIGHT
            })
        }) {
            issues.push(recipe_issue(format!(
                "Mountains stream {body_id:?} does not contain the exact {MOUNTAIN_FALL_HEIGHT}-level fall"
            )));
        }
        let terminals = body
            .nodes
            .iter()
            .filter_map(|(position, node)| {
                (node.state == LiquidFlowState::Still && node.downstream.is_none())
                    .then_some(*position)
            })
            .collect::<Vec<_>>();
        if terminals.len() != 1 {
            issues.push(recipe_issue(format!(
                "Mountains stream {body_id:?} has {} boundary outlets; expected one",
                terminals.len()
            )));
        } else if terminals.first().is_some_and(|terminal| {
            terminal
                .coord
                .neighbors()
                .into_iter()
                .all(|neighbor| plan.layout.footprint.contains(&neighbor))
        }) {
            issues.push(recipe_issue(format!(
                "Mountains stream {body_id:?} does not terminate at a patch boundary"
            )));
        }
        let downstream_targets = body
            .nodes
            .values()
            .filter_map(|node| node.downstream)
            .collect::<BTreeSet<_>>();
        let sources = body
            .nodes
            .keys()
            .filter(|position| !downstream_targets.contains(position))
            .count();
        if sources != 1 {
            issues.push(recipe_issue(format!(
                "Mountains stream {body_id:?} has {sources} spring sources; expected one"
            )));
        }
    }
}

fn validate_route(
    name: &str,
    route: &ProtectedFeatureRoute,
    graph: &OrdinaryGraph,
    issues: &mut Vec<WorldValidationIssue>,
) {
    if route.surfaces.len() < route.centerline.len().saturating_add(2) {
        issues.push(recipe_issue(format!(
            "Mountains {name} is not meaningfully two-wide"
        )));
    }
    for pair in route.centerline.windows(2) {
        let Some(from) = pair.first().copied() else {
            continue;
        };
        let Some(to) = pair.get(1).copied() else {
            continue;
        };
        if !graph.admits(from, to) {
            issues.push(recipe_issue(format!(
                "Mountains {name} contains an illegal step {from:?} -> {to:?}"
            )));
        }
    }
    for center in &route.centerline {
        if !route
            .surfaces
            .iter()
            .any(|surface| surface.coord != center.coord && graph.admits(*center, *surface))
        {
            issues.push(recipe_issue(format!(
                "Mountains {name} has no walkable shoulder at {center:?}"
            )));
        }
    }
}

fn validate_shared_approaches(
    plan: &GeneratedWorldPlan,
    ordinary: &OrdinaryGraph,
    issues: &mut Vec<WorldValidationIssue>,
) {
    for edge in plan.layout.shared_edges.values() {
        for approaches in edge.protected_approaches.values() {
            for coord in approaches {
                let surface = plan
                    .volume
                    .surfaces
                    .keys()
                    .find(|surface| surface.coord == *coord)
                    .copied();
                let Some(surface) = surface else {
                    issues.push(recipe_issue(format!(
                        "Mountains has no seam approach at {coord:?}"
                    )));
                    continue;
                };
                if surface.level != edge.elevation.preferred || !ordinary.contains(surface) {
                    issues.push(recipe_issue(format!(
                        "Mountains seam approach {surface:?} is not ordinary footing at preferred level {}",
                        edge.elevation.preferred
                    )));
                }
            }
        }
    }
}

fn count_cliff_edges(volume: &VolumePlan) -> u32 {
    let by_coord: BTreeMap<_, _> = volume
        .surfaces
        .keys()
        .map(|position| (position.coord, position.level))
        .collect();
    let mut cliffs = 0_usize;
    for (coord, level) in &by_coord {
        for neighbor in coord.neighbors() {
            if neighbor > *coord
                && by_coord
                    .get(&neighbor)
                    .is_some_and(|other| level.abs_diff(*other) >= 2)
            {
                cliffs = cliffs.saturating_add(1);
            }
        }
    }
    count_u32(cliffs)
}

fn surface_material_at(volume: &VolumePlan, position: TilePos) -> Option<SolidMaterialRole> {
    volume.columns.get(&position.coord).and_then(|column| {
        column.elements.iter().find_map(|element| {
            let VolumeElement::Solid(mass) = element else {
                return None;
            };
            (mass.levels.bottom <= position.level && position.level < mass.levels.top)
                .then_some(mass.material)
        })
    })
}

fn mountain_view_hint(
    mask: &BTreeSet<HexCoord>,
    levels: &BTreeMap<HexCoord, Level>,
    grid_radius: u32,
    level_height: f32,
) -> Result<MapViewHint, Vec<WorldValidationIssue>> {
    let total = levels.values().try_fold(0.0_f32, |sum, level| {
        i16::try_from(*level)
            .map(|level| sum + f32::from(level))
            .map_err(|error| {
                vec![recipe_issue(format!(
                    "Mountains camera level does not fit inside i16: {error}"
                ))]
            })
    })?;
    let count = f32::from(u16::try_from(levels.len()).map_err(|error| {
        vec![recipe_issue(format!(
            "Mountains camera footprint does not fit inside u16: {error}"
        ))]
    })?);
    let focus_level = (total / count) * level_height;
    let radius = u16::try_from(grid_radius)
        .map_err(|error| vec![recipe_issue(format!("Mountains camera radius: {error}"))])?;
    let frame = f32::from(radius).mul_add(3.5, 12.0);
    let center = mask
        .iter()
        .copied()
        .min_by_key(|coord| coord.distance(HexCoord::ORIGIN))
        .unwrap_or(HexCoord::ORIGIN);
    let horizontal = f32::from(i16::try_from(center.x()).map_err(|error| {
        vec![recipe_issue(format!(
            "Mountains camera coordinate does not fit inside i16: {error}"
        ))]
    })?) * 0.5;
    Ok(MapViewHint::new(
        (horizontal, focus_level + frame * 0.82, frame * 0.78),
        (horizontal, focus_level, 0.0),
    ))
}

fn axis_value(coord: HexCoord, turns: u8) -> i32 {
    let [x, y, z] = rotate(coord, turns).to_cubic_array();
    x.saturating_sub(z).saturating_add(y / 2)
}

fn rotate(coord: HexCoord, turns: u8) -> HexCoord {
    let [mut x, mut y, mut z] = coord.to_cubic_array();
    for _ in 0..turns % 6 {
        (x, y, z) = (-z, -x, -y);
    }
    HexCoord::new_cubic(x, y, z)
}

fn percent(part: usize, total: usize) -> u32 {
    count_u32(part)
        .saturating_mul(100)
        .checked_div(count_u32(total))
        .unwrap_or_default()
}

fn count_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn recipe_issue(detail: impl Into<String>) -> WorldValidationIssue {
    WorldValidationIssue::new(WorldIssueCode::Recipe("mountains"), detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{PatchEdgeContractSettings, PatchEdgesSettings, PatchMaskSettings};

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

    fn settings() -> ProceduralV3Settings {
        ProceduralV3Settings {
            layout: V3LayoutSettings::Single(crate::settings::PatchSpec {
                environment: V3EnvironmentSettings::Frozen,
                recipe: V3RecipeSettings::Mountains(V3MountainsSettings {
                    base_level: 15,
                    relief: 32,
                    peak_count: 5,
                }),
                overlays: Vec::new(),
                mask: PatchMaskSettings::WholeWorld,
                edges: world_boundaries(),
            }),
        }
    }

    #[test]
    fn native_mountains_are_broad_sharp_and_offer_two_routes() {
        let settings = settings();
        let first = generate(12, 0.4, &settings, 5_181).expect("valid Mountains");
        let second = generate(12, 0.4, &settings, 5_181).expect("same valid Mountains");

        assert_eq!(
            first.validated.semantic_fingerprint,
            second.validated.semantic_fingerprint
        );
        assert!(first.metrics.mountain_coverage_percent >= 52);
        assert!(first.metrics.accessible_mountain_surfaces > 0);
        assert_eq!(first.metrics.relief, 32);
        assert_eq!(first.metrics.peak_count, 5);
        assert!(first.metrics.cliff_edges > 0);
        assert!(first.metrics.high_pass_steps > 0);
        assert!(first.metrics.lower_bypass_steps > 0);
        assert!(
            percent(
                usize::try_from(first.metrics.accessible_mountain_surfaces).unwrap_or(usize::MAX),
                usize::try_from(first.metrics.mountain_surfaces).unwrap_or(usize::MAX)
            ) >= 60
        );
        let streams = &first.validated.plan.liquids.bodies;
        assert_eq!(streams.len(), 1);
        let stream = streams.values().next().expect("one mountain stream");
        assert_eq!(stream.material, FillMaterialRole::Water);
        assert_eq!(
            stream
                .nodes
                .values()
                .filter(|node| node.state == LiquidFlowState::Fall)
                .count(),
            1
        );
    }

    #[test]
    fn revised_mountains_validate_supported_radius_coverage() {
        let settings = settings();
        for radius in [12, 20, 40] {
            let selected = generate(radius, 0.4, &settings, 1_592_598_566)
                .unwrap_or_else(|error| panic!("Mountains radius {radius}: {error}"));
            assert_eq!(selected.metrics.relief, 32);
            assert!(
                percent(
                    usize::try_from(selected.metrics.accessible_mountain_surfaces)
                        .unwrap_or(usize::MAX),
                    usize::try_from(selected.metrics.mountain_surfaces).unwrap_or(usize::MAX)
                ) >= 60
            );
        }
    }

    #[test]
    fn revised_mountains_pr_corpus_validates_128_seeds_and_named_regression() {
        let settings = settings();
        let mut seeds = (0_u64..128).collect::<BTreeSet<_>>();
        seeds.insert(1_592_598_566);
        let mut fallback_seeds = Vec::new();
        for seed in seeds {
            let selected = generate(12, 0.4, &settings, seed)
                .unwrap_or_else(|error| panic!("Mountains seed {seed}: {error}"));
            assert_eq!(selected.candidates_evaluated, 8);
            if selected.used_fallback {
                fallback_seeds.push(seed);
            }
        }
        assert!(
            fallback_seeds.is_empty(),
            "revised Mountains used fallback for seeds {fallback_seeds:?}"
        );
    }

    #[test]
    fn non_frozen_mountains_fail_explicitly() {
        let mut settings = settings();
        let V3LayoutSettings::Single(patch) = &mut settings.layout else {
            unreachable!("test uses Single")
        };
        patch.environment = V3EnvironmentSettings::Rocky;
        assert!(generate(12, 0.4, &settings, 1).is_err());
    }
}
