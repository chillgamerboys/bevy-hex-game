//! Layered V3 sky islands above an independently finalized Hills ground.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use hex_core::{HexCoord, Level, MapViewHint, SpecialMovementRegion, TilePos};

use super::composition::{compose_single_patch, GeneratedPatchPlan};
use super::hills;
use super::layout::{resolve_layout, PatchId, ResolvedLayoutPlan};
use super::patch::{PatchBuildMode, PatchRecipeContext};
use super::seam::validate_patch_walker_seams;
use super::seed::SeedStream;
use super::selection::{
    run_recipe, CandidateAttemptError, CandidateContext, FallbackContext, RepairOutcome, V3Recipe,
    ValidatedWorldSelection, WorldValidation,
};
use super::volume::{
    LevelInterval, SolidMass, SolidMaterialRole, SurfaceAccess, SurfaceMetadata, VolumeElement,
};
use super::world::{
    GeneratedWorldPlan, PlannedStructure, StructureId, StructureKind, WorldIssueCode,
    WorldValidationIssue,
};
use super::V3GenerationError;
use crate::settings::{
    ProceduralV3Settings, V3EnvironmentSettings, V3LayoutSettings, V3RecipeSettings,
    V3SkyIslandsSettings,
};

const PRIMARY_ISLANDS: usize = 3;
const SATELLITES: usize = 1;

/// Measurements owned by the layered Sky Islands recipe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SkyMetrics {
    pub(crate) ground_surfaces: u32,
    pub(crate) upper_surfaces: u32,
    pub(crate) upper_coverage_percent: u32,
    pub(crate) primary_islands: u8,
    pub(crate) satellites: u8,
    pub(crate) bridge_surfaces: u32,
    pub(crate) vertical_clearance: Level,
}

#[derive(Debug)]
struct SkyRecipe {
    level_height: f32,
    layout: ResolvedLayoutPlan,
    settings: V3SkyIslandsSettings,
    environment: V3EnvironmentSettings,
}

pub(crate) fn generate(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV3Settings,
    seed: u64,
) -> Result<ValidatedWorldSelection<SkyMetrics>, V3GenerationError> {
    if !level_height.is_finite() || level_height <= 0.0 {
        return Err(V3GenerationError::RecipeContract(
            "SkyIslands level height must be positive and finite".to_owned(),
        ));
    }
    let (sky, environment) = recipe_settings(settings)?;
    let layout = resolve_layout(grid_radius, settings)
        .map_err(|error| V3GenerationError::RecipeContract(error.to_string()))?;
    run_recipe(
        &SkyRecipe {
            level_height,
            layout,
            settings: sky.clone(),
            environment,
        },
        settings,
        grid_radius,
        seed,
    )
}

impl V3Recipe for SkyRecipe {
    type Settings = ProceduralV3Settings;
    type Metrics = SkyMetrics;
    type Score = (u32, u32, u8);

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
            self.environment,
            self.level_height,
            PatchBuildMode::Candidate {
                world_seed: context.seed,
                candidate: context.candidate,
            },
        )
        .map_err(CandidateAttemptError::Rejected)?;
        compose_single_patch(self.layout.clone(), fragment).map_err(|error| {
            CandidateAttemptError::Fatal(V3GenerationError::RecipeContract(format!(
                "SkyIslands single-patch composition failed: {error:?}"
            )))
        })
    }

    fn validate(
        &self,
        _settings: &Self::Settings,
        plan: &GeneratedWorldPlan,
    ) -> WorldValidation<Self::Metrics> {
        validate_sky(plan, &self.settings)
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
            metrics
                .upper_coverage_percent
                .abs_diff(u32::from(self.settings.upper_coverage_percent)),
            metrics.upper_surfaces,
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
                "SkyIslands fallback radius disagrees with its resolved layout".to_owned(),
            ));
        }
        let patch = PatchRecipeContext::resolve(&self.layout, PatchId(0))?;
        let fragment = construct_patch(
            patch,
            &self.settings,
            self.environment,
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
                "SkyIslands fallback composition failed: {error:?}"
            ))
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SkyStreams<'a> {
    ground_orientation: SeedStream<'a>,
    ground_centres: SeedStream<'a>,
    island_centres: SeedStream<'a>,
    satellite: SeedStream<'a>,
}

fn recipe_settings(
    settings: &ProceduralV3Settings,
) -> Result<(&V3SkyIslandsSettings, V3EnvironmentSettings), V3GenerationError> {
    let V3LayoutSettings::Single(patch) = &settings.layout else {
        return Err(V3GenerationError::RecipeUnavailable("Ring7"));
    };
    let V3RecipeSettings::SkyIslands(sky) = &patch.recipe else {
        return Err(V3GenerationError::RecipeUnavailable(recipe_name(
            &patch.recipe,
        )));
    };
    if !matches!(
        patch.environment,
        V3EnvironmentSettings::TemperateGrassland | V3EnvironmentSettings::Frozen
    ) {
        return Err(V3GenerationError::RecipeContract(
            "SkyIslands requires TemperateGrassland or Frozen".to_owned(),
        ));
    }
    if !patch.overlays.is_empty() {
        return Err(V3GenerationError::RecipeContract(
            "SkyIslands overlays are not implemented yet".to_owned(),
        ));
    }
    Ok((sky, patch.environment))
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

pub(crate) fn construct_patch(
    patch: PatchRecipeContext<'_>,
    settings: &V3SkyIslandsSettings,
    environment: V3EnvironmentSettings,
    level_height: f32,
    mode: PatchBuildMode,
) -> Result<GeneratedPatchPlan, Vec<WorldValidationIssue>> {
    let root_streams = mode.seed_streams(&patch);
    let streams = root_streams.map(|streams| SkyStreams {
        ground_orientation: streams.stage("sky.ground.orientation"),
        ground_centres: streams.stage("sky.ground.centres"),
        island_centres: streams.stage("sky.island_centres"),
        satellite: streams.stage("sky.satellite"),
    });
    let ground_streams =
        streams.map(|streams| (streams.ground_orientation, streams.ground_centres));
    let mut plan = hills::construct_patch_with_streams(
        patch,
        &settings.ground,
        environment,
        level_height,
        ground_streams,
    )?;
    let mask = patch.mask().clone();
    let excluded = patch.protected_approaches();
    let shared_boundary = patch
        .shared_edges()
        .flat_map(|edge| edge.boundary_pairs().into_iter().map(|(local, _)| local))
        .collect::<BTreeSet<_>>();
    let upper_mask = mask
        .difference(&shared_boundary)
        .copied()
        .collect::<BTreeSet<_>>();
    let mut ground_levels = BTreeMap::<HexCoord, Level>::new();
    for surface in plan.volume.surfaces.keys() {
        ground_levels
            .entry(surface.coord)
            .and_modify(|level| *level = (*level).max(surface.level))
            .or_insert(surface.level);
    }
    let highest_ground = ground_levels.values().copied().max().unwrap_or_default();
    let upper_bottom = highest_ground
        .saturating_add(1)
        .saturating_add(settings.min_clearance);
    let upper_base = upper_bottom.saturating_add(5);

    let centres = select_centres(
        &upper_mask,
        &excluded,
        streams.map(|streams| streams.island_centres),
    )?;
    let bridge_rows = bridge_rows(&centres, &upper_mask);
    let bridge_estimate = bridge_rows
        .iter()
        .flat_map(|row| row.iter().copied())
        .collect::<BTreeSet<_>>()
        .len();
    let target = mask
        .len()
        .saturating_mul(usize::from(settings.upper_coverage_percent))
        / 100;
    let primary_target = target
        .saturating_sub(bridge_estimate)
        .saturating_sub(7)
        .max(PRIMARY_ISLANDS);
    let primary_cells = grow_primary_islands(&upper_mask, &excluded, &centres, primary_target);
    let satellite_cells = select_satellite(
        &upper_mask,
        &excluded,
        &primary_cells,
        &bridge_rows,
        streams.map(|streams| streams.satellite),
    )?;

    let primary_region = SpecialMovementRegion(0);
    let satellite_region = SpecialMovementRegion(1);
    let mut upper = BTreeMap::<HexCoord, UpperCell>::new();
    for coord in &primary_cells {
        let owner = centres
            .iter()
            .enumerate()
            .min_by_key(|(index, centre)| (centre.distance(*coord), *index))
            .map(|(index, _)| index)
            .unwrap_or_default();
        let level = upper_base.saturating_add(i32::try_from(owner).unwrap_or_default());
        upper.insert(
            *coord,
            UpperCell {
                level,
                material: surface_material(environment),
                region: primary_region,
                bridge: false,
            },
        );
    }
    let composite_layout = patch.layout().kind.is_composite();
    let first_sky_bridge_id = plan
        .structures
        .by_id
        .keys()
        .next_back()
        .map_or(0, |id| id.0.saturating_add(1));
    let mut reserved_ring_bridge_surfaces = BTreeSet::new();
    for (bridge_index, row) in bridge_rows.iter().enumerate() {
        let Some(start) = row.first().copied() else {
            continue;
        };
        let Some(end) = row.last().copied() else {
            continue;
        };
        let start_level = upper.get(&start).map_or(upper_base, |cell| cell.level);
        let end_level = upper.get(&end).map_or(start_level, |cell| cell.level);
        let denominator = row.len().saturating_sub(1).max(1);
        let ring_route = composite_layout
            .then(|| ring_bridge_route(row, &upper_mask, &reserved_ring_bridge_surfaces))
            .transpose()?;
        if let Some(route) = &ring_route {
            reserved_ring_bridge_surfaces.extend(route.surfaces.iter().copied());
        }
        for (index, coord) in row.iter().copied().enumerate() {
            let level = interpolated_level(start_level, end_level, index, denominator);
            let lane_cells = ring_route.as_ref().map_or_else(
                || two_wide_cells(coord, &upper_mask),
                |route| route.cells_at(index),
            );
            for lane_coord in lane_cells {
                let bridge = UpperCell {
                    level,
                    material: SolidMaterialRole::Metal,
                    region: primary_region,
                    bridge: true,
                };
                if composite_layout {
                    if ring_bridge_corridor_index(index, row.len()) {
                        upper.insert(lane_coord, bridge);
                    }
                } else {
                    upper.entry(lane_coord).or_insert(bridge);
                }
            }
        }
        if let Some(route) = ring_route {
            let surfaces = route
                .surfaces
                .into_iter()
                .filter_map(|coord| {
                    upper
                        .get(&coord)
                        .filter(|cell| cell.bridge)
                        .map(|cell| TilePos::new(coord, cell.level))
                })
                .collect();
            plan.structures.by_id.insert(
                StructureId(
                    first_sky_bridge_id
                        .saturating_add(u32::try_from(bridge_index).unwrap_or(u32::MAX)),
                ),
                PlannedStructure {
                    kind: StructureKind::Bridge,
                    voxels: surfaces,
                },
            );
        }
    }
    for coord in &satellite_cells {
        upper.insert(
            *coord,
            UpperCell {
                level: upper_base.saturating_add(3),
                material: surface_material(environment),
                region: satellite_region,
                bridge: false,
            },
        );
    }
    for (coord, cell) in &upper {
        let Some(column) = plan.volume.columns.get_mut(coord) else {
            return Err(vec![recipe_issue(format!(
                "Sky upper cell {coord:?} has no ground column"
            ))]);
        };
        if cell.bridge {
            column.elements.push(VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(cell.level, cell.level.saturating_add(1)),
                material: cell.material,
                cutaway_for: None,
            }));
        } else {
            column.elements.push(VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(
                    cell.level.saturating_sub(5),
                    cell.level.saturating_sub(3),
                ),
                material: SolidMaterialRole::Stone,
                cutaway_for: None,
            }));
            column.elements.push(VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(cell.level.saturating_sub(3), cell.level),
                material: SolidMaterialRole::Dirt,
                cutaway_for: None,
            }));
            column.elements.push(VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(cell.level, cell.level.saturating_add(1)),
                material: cell.material,
                cutaway_for: None,
            }));
        }
        let position = TilePos::new(*coord, cell.level);
        plan.volume.surfaces.insert(
            position,
            SurfaceMetadata {
                access: SurfaceAccess::SpecialMovement(cell.region),
                interior: None,
            },
        );
        plan.biome_regions.insert(position, patch.biome_region());
    }
    plan.view_hint = sky_view_hint(patch.grid_radius(), upper_base, level_height)?;
    let mut issues = validate_patch_walker_seams(&patch, &plan.volume);
    issues.extend(
        plan.validate_against(patch.layout())
            .into_iter()
            .map(|issue| {
                recipe_issue(format!(
                    "SkyIslands patch {:?} failed {:?}: {}",
                    issue.patch, issue.code, issue.detail
                ))
            }),
    );
    if issues.is_empty() {
        Ok(plan)
    } else {
        Err(issues)
    }
}

#[derive(Debug, Clone, Copy)]
struct UpperCell {
    level: Level,
    material: SolidMaterialRole,
    region: SpecialMovementRegion,
    bridge: bool,
}

fn select_centres(
    mask: &BTreeSet<HexCoord>,
    excluded: &BTreeSet<HexCoord>,
    stream: Option<SeedStream<'_>>,
) -> Result<[HexCoord; PRIMARY_ISLANDS], Vec<WorldValidationIssue>> {
    let mut candidates: Vec<_> = mask
        .iter()
        .copied()
        .filter(|coord| !excluded.contains(coord))
        .collect();
    candidates.sort_by_key(|coord| {
        (
            stream.map_or(0, |stream| stream.sample_coord(*coord, 0)),
            *coord,
        )
    });
    let mut selected = Vec::new();
    for coord in candidates {
        if selected
            .iter()
            .all(|centre: &HexCoord| centre.distance(coord) >= 5)
        {
            selected.push(coord);
            if selected.len() == PRIMARY_ISLANDS {
                break;
            }
        }
    }
    selected.try_into().map_err(|selected: Vec<HexCoord>| {
        vec![recipe_issue(format!(
            "SkyIslands selected {} primary centres; expected {PRIMARY_ISLANDS}",
            selected.len()
        ))]
    })
}

fn bridge_rows(
    centres: &[HexCoord; PRIMARY_ISLANDS],
    mask: &BTreeSet<HexCoord>,
) -> Vec<Vec<HexCoord>> {
    centres
        .windows(2)
        .map(|pair| {
            let Some(start) = pair.first().copied() else {
                return Vec::new();
            };
            let Some(end) = pair.last().copied() else {
                return Vec::new();
            };
            start
                .line_between(end)
                .into_iter()
                .filter(|coord| mask.contains(coord))
                .collect()
        })
        .collect()
}

fn grow_primary_islands(
    mask: &BTreeSet<HexCoord>,
    excluded: &BTreeSet<HexCoord>,
    centres: &[HexCoord; PRIMARY_ISLANDS],
    target: usize,
) -> BTreeSet<HexCoord> {
    let mut cells: BTreeSet<_> = centres.iter().copied().collect();
    let mut frontier: VecDeque<_> = centres.iter().copied().collect();
    while cells.len() < target {
        let Some(coord) = frontier.pop_front() else {
            break;
        };
        let mut neighbors: Vec<_> = coord
            .neighbors()
            .into_iter()
            .filter(|neighbor| mask.contains(neighbor) && !excluded.contains(neighbor))
            .collect();
        neighbors.sort_unstable();
        for neighbor in neighbors {
            if cells.insert(neighbor) {
                frontier.push_back(neighbor);
                if cells.len() == target {
                    break;
                }
            }
        }
    }
    cells
}

fn select_satellite(
    mask: &BTreeSet<HexCoord>,
    excluded: &BTreeSet<HexCoord>,
    primary: &BTreeSet<HexCoord>,
    bridge_rows: &[Vec<HexCoord>],
    stream: Option<SeedStream<'_>>,
) -> Result<BTreeSet<HexCoord>, Vec<WorldValidationIssue>> {
    let bridge_cells: BTreeSet<_> = bridge_rows.iter().flatten().copied().collect();
    let mut candidates: Vec<_> = mask
        .iter()
        .copied()
        .filter(|coord| !excluded.contains(coord))
        .filter(|coord| primary.iter().all(|primary| primary.distance(*coord) >= 3))
        .filter(|coord| {
            bridge_cells
                .iter()
                .all(|bridge| bridge.distance(*coord) >= 3)
        })
        .collect();
    candidates.sort_by_key(|coord| {
        (
            stream.map_or(0, |stream| stream.sample_coord(*coord, 0)),
            *coord,
        )
    });
    let Some(center) = candidates.first().copied() else {
        return Err(vec![recipe_issue(
            "SkyIslands cannot place a separated satellite",
        )]);
    };
    Ok(center
        .within_radius(1)
        .into_iter()
        .filter(|coord| mask.contains(coord))
        .collect())
}

fn two_wide_cells(coord: HexCoord, mask: &BTreeSet<HexCoord>) -> BTreeSet<HexCoord> {
    let mut cells = BTreeSet::from([coord]);
    if let Some(neighbor) = coord
        .neighbors()
        .into_iter()
        .filter(|neighbor| mask.contains(neighbor))
        .min()
    {
        cells.insert(neighbor);
    }
    cells
}

#[derive(Debug, Clone)]
struct RingBridgeRoute {
    lanes: Vec<[HexCoord; 2]>,
    surfaces: BTreeSet<HexCoord>,
}

impl RingBridgeRoute {
    fn cells_at(&self, index: usize) -> BTreeSet<HexCoord> {
        self.lanes
            .get(index)
            .copied()
            .map_or_else(BTreeSet::new, BTreeSet::from)
    }
}

fn ring_bridge_route(
    row: &[HexCoord],
    mask: &BTreeSet<HexCoord>,
    reserved: &BTreeSet<HexCoord>,
) -> Result<RingBridgeRoute, Vec<WorldValidationIssue>> {
    if row.len() < 4 {
        return Err(vec![recipe_issue(format!(
            "SkyIslands bridge row has {} cells; at least four are required",
            row.len()
        ))]);
    }
    let row_cells = row.iter().copied().collect::<BTreeSet<_>>();
    let lane_candidates = row
        .iter()
        .enumerate()
        .map(|(index, coord)| {
            let mut candidates = coord
                .neighbors()
                .into_iter()
                .filter(|neighbor| mask.contains(neighbor) && !row_cells.contains(neighbor))
                .filter(|neighbor| {
                    index == 0 || index + 1 == row.len() || !reserved.contains(neighbor)
                })
                .collect::<Vec<_>>();
            candidates.sort_unstable();
            candidates
        })
        .collect::<Vec<_>>();
    let mut paths = lane_candidates
        .first()
        .into_iter()
        .flatten()
        .copied()
        .map(|coord| (coord, vec![coord]))
        .collect::<BTreeMap<_, _>>();
    for candidates in lane_candidates.iter().skip(1) {
        let mut next_paths = BTreeMap::<HexCoord, Vec<HexCoord>>::new();
        for candidate in candidates {
            for path in paths.values() {
                if path.last().is_some_and(|previous| {
                    previous.distance(*candidate) == 1 && !path.contains(candidate)
                }) {
                    let mut extended = path.clone();
                    extended.push(*candidate);
                    next_paths
                        .entry(*candidate)
                        .and_modify(|existing| {
                            if extended < *existing {
                                *existing = extended.clone();
                            }
                        })
                        .or_insert(extended);
                }
            }
        }
        paths = next_paths;
    }
    let shifted = paths.into_values().min().ok_or_else(|| {
        vec![recipe_issue(
            "SkyIslands could not fit a distinct parallel two-wide upper bridge",
        )]
    })?;
    let lanes = row
        .iter()
        .copied()
        .zip(shifted)
        .map(|(first, second)| [first, second])
        .collect::<Vec<_>>();
    let surfaces = lanes
        .iter()
        .enumerate()
        .filter(|(index, _)| ring_bridge_corridor_index(*index, lanes.len()))
        .flat_map(|(_, lane)| lane.iter().copied())
        .collect::<BTreeSet<_>>();
    if !surfaces.is_disjoint(reserved) {
        return Err(vec![recipe_issue(
            "SkyIslands upper bridge overlaps another retained bridge span",
        )]);
    }
    Ok(RingBridgeRoute { lanes, surfaces })
}

fn ring_bridge_corridor_index(index: usize, row_len: usize) -> bool {
    index > 1 && index.saturating_add(2) < row_len
}

fn interpolated_level(start: Level, end: Level, index: usize, denominator: usize) -> Level {
    let delta = end.saturating_sub(start);
    let index = i32::try_from(index).unwrap_or(i32::MAX);
    let denominator = i32::try_from(denominator).unwrap_or(i32::MAX).max(1);
    start.saturating_add(delta.saturating_mul(index) / denominator)
}

const fn surface_material(environment: V3EnvironmentSettings) -> SolidMaterialRole {
    match environment {
        V3EnvironmentSettings::Frozen => SolidMaterialRole::Snow,
        V3EnvironmentSettings::TemperateGrassland => SolidMaterialRole::Grass,
        V3EnvironmentSettings::Volcanic => SolidMaterialRole::Basalt,
        V3EnvironmentSettings::Rocky => SolidMaterialRole::Stone,
    }
}

pub(crate) fn validate_sky(
    plan: &GeneratedWorldPlan,
    settings: &V3SkyIslandsSettings,
) -> WorldValidation<SkyMetrics> {
    let mut issues = plan.validate();
    let mut ground = BTreeMap::<HexCoord, TilePos>::new();
    let mut primary = BTreeSet::new();
    let mut satellites = BTreeSet::new();
    let mut metal_primary_surfaces = BTreeSet::new();
    let primary_region = SpecialMovementRegion(0);
    let satellite_region = SpecialMovementRegion(1);
    for (position, metadata) in &plan.volume.surfaces {
        match metadata.access {
            SurfaceAccess::Ordinary | SurfaceAccess::NonStandable => {
                ground
                    .entry(position.coord)
                    .and_modify(|surface| {
                        if position.level > surface.level {
                            *surface = *position;
                        }
                    })
                    .or_insert(*position);
            }
            SurfaceAccess::SpecialMovement(region) if region == primary_region => {
                primary.insert(*position);
                if upper_surface_material(plan, *position) == Some(SolidMaterialRole::Metal) {
                    metal_primary_surfaces.insert(*position);
                }
            }
            SurfaceAccess::SpecialMovement(region) if region == satellite_region => {
                satellites.insert(*position);
            }
            SurfaceAccess::SpecialMovement(_) => {}
        }
    }
    let upper_count = primary.len().saturating_add(satellites.len());
    let coverage = percentage(upper_count, plan.layout.footprint.len());
    if !(15..=25).contains(&coverage) {
        issues.push(recipe_issue(format!(
            "SkyIslands upper coverage {coverage}% leaves the required 15..=25 band"
        )));
    }
    if !surface_network_connected(&primary) {
        issues.push(recipe_issue(
            "SkyIslands primary island and bridge network is not climbable",
        ));
    }
    if satellites.is_empty() {
        issues.push(recipe_issue("SkyIslands has no separated satellite"));
    }
    let minimum_clearance = primary
        .iter()
        .chain(&satellites)
        .filter_map(|upper| {
            ground
                .get(&upper.coord)
                .and_then(|ground| clear_levels_below_surface(plan, *upper, *ground))
        })
        .min()
        .unwrap_or_default();
    if minimum_clearance < settings.min_clearance {
        issues.push(recipe_issue(format!(
            "SkyIslands clearance {minimum_clearance} is below configured {}",
            settings.min_clearance
        )));
    }
    let bridge_surfaces = if plan.layout.kind.is_composite() {
        validate_ring_upper_bridges(plan, &primary, &metal_primary_surfaces, &mut issues)
    } else {
        let count = count_u32(metal_primary_surfaces.len());
        if count < 4 {
            issues.push(recipe_issue(
                "SkyIslands primary network lacks two-wide upper bridges",
            ));
        }
        count
    };
    if !issues.is_empty() {
        return WorldValidation::Invalid(issues);
    }
    WorldValidation::Valid(SkyMetrics {
        ground_surfaces: count_u32(ground.len()),
        upper_surfaces: count_u32(upper_count),
        upper_coverage_percent: coverage,
        primary_islands: u8::try_from(PRIMARY_ISLANDS).unwrap_or(u8::MAX),
        satellites: u8::try_from(SATELLITES).unwrap_or(u8::MAX),
        bridge_surfaces,
        vertical_clearance: minimum_clearance,
    })
}

fn clear_levels_below_surface(
    plan: &GeneratedWorldPlan,
    upper: TilePos,
    ground: TilePos,
) -> Option<Level> {
    let mass_bottom = plan
        .volume
        .columns
        .get(&upper.coord)?
        .elements
        .iter()
        .find_map(|element| {
            let VolumeElement::Solid(mass) = element else {
                return None;
            };
            (mass.levels.bottom <= upper.level && upper.level < mass.levels.top)
                .then_some(mass.levels.bottom)
        })?;
    Some(mass_bottom.saturating_sub(ground.level).saturating_sub(1))
}

fn upper_surface_material(
    plan: &GeneratedWorldPlan,
    position: TilePos,
) -> Option<SolidMaterialRole> {
    plan.volume.columns.get(&position.coord).and_then(|column| {
        column.elements.iter().find_map(|element| {
            let VolumeElement::Solid(mass) = *element else {
                return None;
            };
            (mass.levels.bottom <= position.level && position.level < mass.levels.top)
                .then_some(mass.material)
        })
    })
}

fn surface_network_connected(surfaces: &BTreeSet<TilePos>) -> bool {
    let Some(start) = surfaces.first().copied() else {
        return false;
    };
    let by_coord: BTreeMap<_, _> = surfaces
        .iter()
        .copied()
        .map(|position| (position.coord, position))
        .collect();
    let mut visited = BTreeSet::from([start]);
    let mut frontier = VecDeque::from([start]);
    while let Some(position) = frontier.pop_front() {
        for neighbor_coord in position.coord.neighbors() {
            let Some(neighbor) = by_coord.get(&neighbor_coord).copied() else {
                continue;
            };
            if position.level.abs_diff(neighbor.level) <= 1 && visited.insert(neighbor) {
                frontier.push_back(neighbor);
            }
        }
    }
    visited.len() == surfaces.len()
}

fn validate_ring_upper_bridges(
    plan: &GeneratedWorldPlan,
    primary: &BTreeSet<TilePos>,
    metal_primary: &BTreeSet<TilePos>,
    issues: &mut Vec<WorldValidationIssue>,
) -> u32 {
    let bridges = plan
        .structures
        .by_id
        .values()
        .filter(|structure| {
            structure.kind == StructureKind::Bridge
                && !structure.voxels.is_empty()
                && structure.voxels.iter().all(|voxel| primary.contains(voxel))
        })
        .collect::<Vec<_>>();
    if bridges.len() != 2 {
        issues.push(recipe_issue(format!(
            "SkyIslands requires exactly two upper bridge structures, found {}",
            bridges.len()
        )));
        return count_u32(metal_primary.len());
    }
    let [first_bridge, second_bridge] = bridges.as_slice() else {
        return count_u32(metal_primary.len());
    };

    let bridge_union = bridges
        .iter()
        .flat_map(|bridge| bridge.voxels.iter().copied())
        .collect::<BTreeSet<_>>();
    if bridge_union != *metal_primary {
        issues.push(recipe_issue(
            "SkyIslands upper bridge structures do not exactly cover the metal primary surfaces",
        ));
    }
    if !first_bridge.voxels.is_disjoint(&second_bridge.voxels) {
        issues.push(recipe_issue(
            "SkyIslands upper bridge structures overlap instead of naming distinct spans",
        ));
    }

    let island_surfaces = primary
        .difference(&bridge_union)
        .copied()
        .collect::<BTreeSet<_>>();
    let island_components = surface_components(&island_surfaces);
    if island_components.len() != PRIMARY_ISLANDS {
        issues.push(recipe_issue(format!(
            "SkyIslands has {} non-bridge primary components; expected {PRIMARY_ISLANDS}",
            island_components.len()
        )));
        return count_u32(bridge_union.len());
    }

    let mut links = BTreeSet::new();
    for (index, bridge) in bridges.iter().enumerate() {
        if bridge.voxels.len() < 4 || bridge.voxels.len() % 2 != 0 {
            issues.push(recipe_issue(format!(
                "SkyIslands upper bridge {index} has {} voxels and is not a two-wide span",
                bridge.voxels.len()
            )));
            continue;
        }
        if !surface_network_connected(&bridge.voxels) {
            issues.push(recipe_issue(format!(
                "SkyIslands upper bridge {index} is not internally connected"
            )));
        }
        let contacts = island_components
            .iter()
            .enumerate()
            .filter_map(|(component, island)| {
                let bridge_contacts = bridge
                    .voxels
                    .iter()
                    .copied()
                    .filter(|voxel| {
                        island
                            .iter()
                            .any(|surface| surfaces_adjoin(*voxel, *surface))
                    })
                    .collect::<BTreeSet<_>>();
                (!bridge_contacts.is_empty()).then_some((component, bridge_contacts))
            })
            .collect::<Vec<_>>();
        if contacts.len() != 2 {
            issues.push(recipe_issue(format!(
                "SkyIslands upper bridge {index} touches {} primary islands; expected two",
                contacts.len()
            )));
            continue;
        }
        if contacts.iter().any(|(_, contacts)| contacts.len() < 2) {
            issues.push(recipe_issue(format!(
                "SkyIslands upper bridge {index} does not have two independent landing contacts"
            )));
            continue;
        }
        let [(first_component, first_contacts), (second_component, second_contacts)] =
            contacts.as_slice()
        else {
            continue;
        };
        for removed in &bridge.voxels {
            if !bridge_connects_contacts(
                &bridge.voxels,
                first_contacts,
                second_contacts,
                Some(*removed),
            ) {
                issues.push(recipe_issue(format!(
                    "SkyIslands upper bridge {index} narrows to a one-voxel choke at {removed:?}"
                )));
                break;
            }
        }
        links.insert((
            *first_component.min(second_component),
            *first_component.max(second_component),
        ));
    }
    if links.len() != 2 || !component_links_connected(PRIMARY_ISLANDS, &links) {
        issues.push(recipe_issue(
            "SkyIslands upper bridges do not form two distinct links across all three primary islands",
        ));
    }
    count_u32(bridge_union.len())
}

fn surface_components(surfaces: &BTreeSet<TilePos>) -> Vec<BTreeSet<TilePos>> {
    let mut remaining = surfaces.clone();
    let mut components = Vec::new();
    while let Some(start) = remaining.first().copied() {
        remaining.remove(&start);
        let mut component = BTreeSet::from([start]);
        let mut frontier = VecDeque::from([start]);
        while let Some(position) = frontier.pop_front() {
            let neighbors = remaining
                .iter()
                .copied()
                .filter(|neighbor| surfaces_adjoin(position, *neighbor))
                .collect::<Vec<_>>();
            for neighbor in neighbors {
                remaining.remove(&neighbor);
                component.insert(neighbor);
                frontier.push_back(neighbor);
            }
        }
        components.push(component);
    }
    components
}

fn surfaces_adjoin(first: TilePos, second: TilePos) -> bool {
    first.coord.distance(second.coord) == 1 && first.level.abs_diff(second.level) <= 1
}

fn bridge_connects_contacts(
    bridge: &BTreeSet<TilePos>,
    starts: &BTreeSet<TilePos>,
    goals: &BTreeSet<TilePos>,
    removed: Option<TilePos>,
) -> bool {
    let mut visited = starts
        .iter()
        .copied()
        .filter(|start| Some(*start) != removed)
        .collect::<BTreeSet<_>>();
    let mut frontier = VecDeque::from_iter(visited.iter().copied());
    while let Some(position) = frontier.pop_front() {
        if goals.contains(&position) {
            return true;
        }
        for neighbor in bridge {
            if Some(*neighbor) != removed
                && surfaces_adjoin(position, *neighbor)
                && visited.insert(*neighbor)
            {
                frontier.push_back(*neighbor);
            }
        }
    }
    false
}

fn component_links_connected(count: usize, links: &BTreeSet<(usize, usize)>) -> bool {
    let mut visited = BTreeSet::from([0]);
    let mut frontier = VecDeque::from([0]);
    while let Some(component) = frontier.pop_front() {
        for (first, second) in links {
            let neighbor = if *first == component {
                Some(*second)
            } else if *second == component {
                Some(*first)
            } else {
                None
            };
            if let Some(neighbor) = neighbor {
                if visited.insert(neighbor) {
                    frontier.push_back(neighbor);
                }
            }
        }
    }
    visited.len() == count
}

fn sky_view_hint(
    grid_radius: u32,
    upper_level: Level,
    level_height: f32,
) -> Result<MapViewHint, Vec<WorldValidationIssue>> {
    let radius = u16::try_from(grid_radius)
        .map_err(|error| vec![recipe_issue(format!("Sky camera radius: {error}"))])?;
    let frame = f32::from(radius).mul_add(3.6, 14.0);
    let focus = f32::from(i16::try_from(upper_level).unwrap_or(i16::MAX)) * level_height * 0.65;
    Ok(MapViewHint::new(
        (0.0, focus + frame * 0.75, frame),
        (0.0, focus, 0.0),
    ))
}

fn percentage(part: usize, whole: usize) -> u32 {
    if whole == 0 {
        return 0;
    }
    let numerator = u64::try_from(part).unwrap_or(u64::MAX).saturating_mul(100);
    let denominator = u64::try_from(whole).unwrap_or(u64::MAX).max(1);
    u32::try_from(numerator / denominator).unwrap_or(u32::MAX)
}

fn recipe_issue(detail: impl Into<String>) -> WorldValidationIssue {
    WorldValidationIssue::new(WorldIssueCode::Recipe("sky_islands"), detail)
}

fn count_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{
        PatchEdgeContractSettings, PatchEdgesSettings, PatchMaskSettings, PatchSpec,
        V3HillsSettings,
    };

    fn settings() -> ProceduralV3Settings {
        let boundary = || PatchEdgeContractSettings::WorldBoundary;
        ProceduralV3Settings {
            layout: V3LayoutSettings::Single(PatchSpec {
                environment: V3EnvironmentSettings::TemperateGrassland,
                recipe: V3RecipeSettings::SkyIslands(V3SkyIslandsSettings {
                    ground: V3HillsSettings {
                        valley_level: 15,
                        max_relief: 8,
                        hills_per_bank: 3,
                    },
                    min_clearance: 14,
                    upper_coverage_percent: 20,
                }),
                overlays: Vec::new(),
                mask: PatchMaskSettings::WholeWorld,
                edges: PatchEdgesSettings {
                    east: boundary(),
                    south_east: boundary(),
                    south_west: boundary(),
                    west: boundary(),
                    north_west: boundary(),
                    north_east: boundary(),
                },
            }),
        }
    }

    #[test]
    fn layered_sky_is_deterministic_clear_and_grounded_independently() {
        let settings = settings();
        let first = generate(12, 0.4, &settings, 991).expect("valid sky");
        let second = generate(12, 0.4, &settings, 991).expect("same valid sky");
        assert_eq!(
            first.validated.semantic_fingerprint,
            second.validated.semantic_fingerprint
        );
        assert_eq!(first.metrics.ground_surfaces, 469);
        assert!((15..=25).contains(&first.metrics.upper_coverage_percent));
        assert!(first.metrics.vertical_clearance >= 14);
        assert_eq!(first.metrics.primary_islands, 3);
        assert_eq!(first.metrics.satellites, 1);
    }
}
