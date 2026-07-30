//! Layered V3 sky islands above an independently finalized Hills ground.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use hex_core::{HexCoord, Level, MapViewHint, SpecialMovementRegion, TilePos};

use super::hills;
use super::layout::{resolve_layout, PatchId, ResolvedLayoutPlan};
use super::patch::PatchRecipeContext;
use super::seed::SeedStream;
use super::selection::{
    run_recipe, CandidateAttemptError, CandidateContext, FallbackContext, RepairOutcome, V3Recipe,
    ValidatedWorldSelection, WorldValidation,
};
use super::volume::{
    LevelInterval, SolidMass, SolidMaterialRole, SurfaceAccess, SurfaceMetadata, VolumeElement,
};
use super::world::{GeneratedWorldPlan, WorldIssueCode, WorldValidationIssue};
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
        let streams = patch.seed_streams(context.seed, context.candidate);
        construct_plan(
            self.layout.clone(),
            PatchId(0),
            &self.settings,
            self.environment,
            self.level_height,
            Some(SkyStreams {
                ground_orientation: streams.stage("sky.ground.orientation"),
                ground_centres: streams.stage("sky.ground.centres"),
                island_centres: streams.stage("sky.island_centres"),
                satellite: streams.stage("sky.satellite"),
            }),
        )
        .map_err(CandidateAttemptError::Rejected)
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
        construct_plan(
            self.layout.clone(),
            PatchId(0),
            &self.settings,
            self.environment,
            self.level_height,
            None,
        )
        .map_err(|issues| {
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

pub(crate) fn construct_plan(
    layout: ResolvedLayoutPlan,
    patch_id: PatchId,
    settings: &V3SkyIslandsSettings,
    environment: V3EnvironmentSettings,
    level_height: f32,
    streams: Option<SkyStreams<'_>>,
) -> Result<GeneratedWorldPlan, Vec<WorldValidationIssue>> {
    let ground_streams =
        streams.map(|streams| (streams.ground_orientation, streams.ground_centres));
    let mut plan = hills::construct_plan(
        layout,
        patch_id,
        &settings.ground,
        environment,
        level_height,
        ground_streams,
    )?;
    let patch = PatchRecipeContext::resolve(&plan.layout, patch_id)
        .map_err(|error| vec![recipe_issue(error.to_string())])?;
    let mask = patch.mask().clone();
    let excluded = patch.protected_approaches();
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
        &mask,
        &excluded,
        streams.map(|streams| streams.island_centres),
    )?;
    let bridge_rows = bridge_rows(&centres, &mask);
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
    let primary_cells = grow_primary_islands(&mask, &excluded, &centres, primary_target);
    let satellite_cells = select_satellite(
        &mask,
        &excluded,
        &primary_cells,
        &bridge_rows,
        streams.map(|streams| streams.satellite),
    )?;

    let primary_region = SpecialMovementRegion(patch_id.0.saturating_mul(2));
    let satellite_region = SpecialMovementRegion(patch_id.0.saturating_mul(2).saturating_add(1));
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
    let mut bridge_surfaces = BTreeSet::new();
    for row in &bridge_rows {
        let Some(start) = row.first().copied() else {
            continue;
        };
        let Some(end) = row.last().copied() else {
            continue;
        };
        let start_level = upper.get(&start).map_or(upper_base, |cell| cell.level);
        let end_level = upper.get(&end).map_or(start_level, |cell| cell.level);
        let denominator = row.len().saturating_sub(1).max(1);
        for (index, coord) in row.iter().copied().enumerate() {
            let level = interpolated_level(start_level, end_level, index, denominator);
            for lane_coord in two_wide_cells(coord, &mask) {
                bridge_surfaces.insert(lane_coord);
                upper.entry(lane_coord).or_insert(UpperCell {
                    level,
                    material: SolidMaterialRole::Metal,
                    region: primary_region,
                    bridge: true,
                });
            }
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
        let region = plan
            .biome_regions
            .values()
            .next()
            .copied()
            .unwrap_or_default();
        plan.biome_regions.insert(position, region);
    }
    plan.view_hint = sky_view_hint(plan.layout.grid_radius, upper_base, level_height)?;
    Ok(plan)
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

fn validate_sky(
    plan: &GeneratedWorldPlan,
    settings: &V3SkyIslandsSettings,
) -> WorldValidation<SkyMetrics> {
    let mut issues = plan.validate();
    let mut ground = BTreeMap::<HexCoord, TilePos>::new();
    let mut primary = BTreeSet::new();
    let mut satellites = BTreeSet::new();
    let mut bridge_surfaces = 0_u32;
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
                    bridge_surfaces = bridge_surfaces.saturating_add(1);
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
    if bridge_surfaces < 4 {
        issues.push(recipe_issue(
            "SkyIslands primary network lacks two-wide upper bridges",
        ));
    }
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
