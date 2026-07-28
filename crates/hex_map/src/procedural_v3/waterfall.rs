//! Pure semantic Waterfall recipe for procedural generator V3.
//!
//! Water topology is authored before terrain. The resulting solid volume is fitted
//! around one three-wide watercourse and a separate two-wide ordinary-walker bypass.
//! Rendering and ECS publication remain downstream of this module.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use hex_core::{HexCoord, MapViewHint, TilePos, TraversalEndpoint, TraversalProfile};

use super::layout::{resolve_layout, PatchId, ResolvedLayoutPlan};
use super::liquid::{LiquidBodyId, LiquidBodyPlan, LiquidFlowState, LiquidNode, LiquidPlan};
use super::seed::{SeedStream, SeedStreams};
use super::selection::{
    run_recipe, CandidateAttemptError, CandidateContext, FallbackContext, RepairOutcome, V3Recipe,
    ValidatedWorldSelection, WorldValidation,
};
use super::volume::{
    FillMaterialRole, LevelInterval, NonSolidFill, SolidMass, SolidMaterialRole, SurfaceAccess,
    SurfaceMetadata, VolumeColumn, VolumeElement, VolumePlan,
};
use super::world::{
    FeaturePlan, GeneratedWorldPlan, InteriorPlan, StructurePlan, WorldIssueCode,
    WorldValidationIssue,
};
use super::V3GenerationError;
use crate::settings::{
    ProceduralV3Settings, V3EnvironmentSettings, V3LayoutSettings, V3RecipeSettings,
    V3WaterfallSettings,
};

const HIGH_LAND_LEVEL: i32 = 24;
const LOW_LAND_LEVEL: i32 = 16;
const HIGH_WATER_LEVEL: i32 = HIGH_LAND_LEVEL - 1;
const LOW_WATER_LEVEL: i32 = LOW_LAND_LEVEL - 1;
const FALL_SOURCE_X: i32 = -1;
const FALL_TARGET_X: i32 = 0;
const WATER_HALF_WIDTH: i32 = 1;
const INLET_MARGIN: i32 = 3;
const BASIN_WIDE_END_X: i32 = 1;
const BASIN_END_X: i32 = 3;
const BASIN_MAX_HALF_WIDTH: i32 = 3;
const BYPASS_HIGH_X: i32 = -5;
const BYPASS_LOW_X: i32 = 3;
const SECONDARY_HIGH_X: i32 = -6;
const SECONDARY_LOW_X: i32 = 4;
const RELIEF_CENTERS_PER_BANK: u64 = 3;
const PARTY_START: &str = "party_start";
const HOSTILE_START: &str = "hostile_start";
const FALL_OVERLOOK: &str = "fall_overlook";
const BASIN_OVERLOOK: &str = "basin_overlook";

/// Recipe metrics retained by the V3 candidate selector and later diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WaterfallMetrics {
    pub(crate) water_nodes: u32,
    pub(crate) calm_nodes: u32,
    pub(crate) current_nodes: u32,
    pub(crate) rapid_nodes: u32,
    pub(crate) fall_nodes: u32,
    pub(crate) fall_height: u32,
    pub(crate) ordinary_surfaces: u32,
    pub(crate) reachable_elevation_levels: u32,
    pub(crate) bypass_steps: u32,
    pub(crate) alternate_bypass_steps: u32,
    pub(crate) raised_terrain: u32,
    pub(crate) dry_relief: u32,
    pub(crate) spawn_height_difference: u32,
    pub(crate) bank_high_ground_difference: u32,
    pub(crate) grass_surface_percent: u32,
}

#[derive(Debug)]
struct WaterfallRecipe {
    level_height: f32,
    layout: ResolvedLayoutPlan,
    #[cfg(test)]
    reject_candidates: bool,
}

/// Runs the common eight-candidate V3 selector for one Waterfall world.
pub(crate) fn generate(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV3Settings,
    seed: u64,
) -> Result<ValidatedWorldSelection<WaterfallMetrics>, V3GenerationError> {
    if !level_height.is_finite() || level_height <= 0.0 {
        return Err(V3GenerationError::RecipeContract(
            "Waterfall level height must be positive and finite".to_owned(),
        ));
    }
    validate_recipe_settings(settings)?;
    let layout = resolve_layout(grid_radius, settings)
        .map_err(|error| V3GenerationError::RecipeContract(error.to_string()))?;
    validate_footprint_capacity(&layout)?;
    run_recipe(
        &WaterfallRecipe {
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

impl V3Recipe for WaterfallRecipe {
    type Settings = ProceduralV3Settings;
    type Metrics = WaterfallMetrics;
    type Score = (u32, u8);

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
                    "Waterfall candidate radius disagrees with its resolved layout".to_owned(),
                ),
            ));
        }
        let stream = SeedStreams::new(context.seed, context.candidate, PatchId(0).0)
            .stage("waterfall.relief");
        construct_plan(self.layout.clone(), Some(stream), self.level_height)
            .map_err(CandidateAttemptError::Rejected)
    }

    fn validate(
        &self,
        _settings: &Self::Settings,
        plan: &GeneratedWorldPlan,
    ) -> WorldValidation<Self::Metrics> {
        validate_waterfall(plan)
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
        let target_raised = metrics.ordinary_surfaces / 5;
        (metrics.raised_terrain.abs_diff(target_raised), candidate)
    }

    fn canonical_fallback(
        &self,
        context: FallbackContext,
        settings: &Self::Settings,
    ) -> Result<GeneratedWorldPlan, V3GenerationError> {
        validate_recipe_settings(settings)?;
        if context.grid_radius != self.layout.grid_radius {
            return Err(V3GenerationError::RecipeContract(
                "Waterfall fallback radius disagrees with its resolved layout".to_owned(),
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
            "Waterfall requires the TemperateGrassland environment".to_owned(),
        ));
    }
    if !matches!(
        patch.recipe,
        V3RecipeSettings::Waterfall(V3WaterfallSettings)
    ) {
        return Err(V3GenerationError::RecipeUnavailable(recipe_name(
            &patch.recipe,
        )));
    }
    if !patch.overlays.is_empty() {
        return Err(V3GenerationError::RecipeContract(
            "Waterfall overlays are not implemented yet".to_owned(),
        ));
    }
    Ok(())
}

fn validate_footprint_capacity(layout: &ResolvedLayoutPlan) -> Result<(), V3GenerationError> {
    watercourse(&layout.footprint).map_err(recipe_issues_to_error)?;
    bypass_tiles(layout.grid_radius, &layout.footprint).map_err(recipe_issues_to_error)?;
    secondary_bypass_tiles(layout.grid_radius, &layout.footprint)
        .map_err(recipe_issues_to_error)?;
    secondary_slope_apron(layout.grid_radius, &layout.footprint).map_err(recipe_issues_to_error)?;
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

fn construct_plan(
    layout: ResolvedLayoutPlan,
    relief: Option<SeedStream<'_>>,
    level_height: f32,
) -> Result<GeneratedWorldPlan, Vec<WorldValidationIssue>> {
    let patch = layout
        .patches
        .get(&PatchId(0))
        .ok_or_else(|| vec![recipe_issue("Single Waterfall layout has no patch zero")])?;
    let mask = patch.mask.clone();
    let biome_region = patch.biome_region;
    let watercourse = watercourse(&mask)?;
    let bypass = bypass_tiles(layout.grid_radius, &mask)?;
    let secondary_bypass = secondary_bypass_tiles(layout.grid_radius, &mask)?;
    let secondary_apron = secondary_slope_apron(layout.grid_radius, &mask)?;
    let water_coords = watercourse.coordinates();
    let bypass_by_coord: BTreeMap<_, _> = bypass
        .iter()
        .chain(&secondary_bypass)
        .flatten()
        .chain(&secondary_apron)
        .map(|position| (position.coord, position.level))
        .collect();
    let relief = relief.map(|stream| {
        ReliefPlan::new(
            layout.grid_radius,
            &mask,
            &water_coords,
            &bypass_by_coord,
            stream,
        )
    });

    let mut columns = BTreeMap::new();
    let mut surfaces = BTreeMap::new();
    let mut water_nodes = BTreeMap::new();
    let mut water_by_coord = BTreeMap::new();
    for lane in &watercourse.main_lanes {
        for (index, coord) in lane.iter().copied().enumerate() {
            let next = lane.get(index.saturating_add(1)).copied();
            let cell = water_cell(coord, next);
            water_by_coord.insert(coord, cell);
        }
    }
    for coord in &watercourse.basin {
        water_by_coord.entry(*coord).or_insert(WaterCell {
            bed_level: LOW_WATER_LEVEL - 1,
            fill_bottom: LOW_WATER_LEVEL,
            top: TilePos::new(*coord, LOW_WATER_LEVEL),
            state: LiquidFlowState::Still,
            downstream: None,
        });
    }

    for coord in &mask {
        if let Some(water) = water_by_coord.get(coord).copied() {
            let (column, surface) = water_column(water);
            columns.insert(*coord, column);
            surfaces.insert(
                surface,
                SurfaceMetadata {
                    access: SurfaceAccess::NonStandable,
                    interior: None,
                },
            );
            water_nodes.insert(
                water.top,
                LiquidNode {
                    state: water.state,
                    downstream: water.downstream,
                },
            );
        } else {
            let surface_level = land_surface_level(*coord, &bypass_by_coord, relief.as_ref());
            let surface = TilePos::new(*coord, surface_level);
            columns.insert(*coord, land_column(surface_level));
            surfaces.insert(
                surface,
                SurfaceMetadata {
                    access: SurfaceAccess::Ordinary,
                    interior: None,
                },
            );
        }
    }
    let volume = VolumePlan {
        mask: mask.clone(),
        columns,
        surfaces,
    };

    let party_start = bypass
        .first()
        .and_then(|lane| lane.first())
        .copied()
        .ok_or_else(|| vec![recipe_issue("Waterfall bypass has no high landing")])?;
    let hostile_start = bypass
        .first()
        .and_then(|lane| lane.last())
        .copied()
        .ok_or_else(|| vec![recipe_issue("Waterfall bypass has no low landing")])?;
    let surface_at = |coord| {
        volume
            .surfaces
            .keys()
            .find(|surface| surface.coord == coord)
            .copied()
    };
    let fall_overlook = surface_at(HexCoord::from_axial(
        FALL_SOURCE_X - 1,
        BASIN_MAX_HALF_WIDTH + 1,
    ))
    .ok_or_else(|| vec![recipe_issue("Waterfall has no high fall overlook")])?;
    let basin_overlook = surface_at(HexCoord::from_axial(
        BASIN_WIDE_END_X,
        BASIN_MAX_HALF_WIDTH + 1,
    ))
    .ok_or_else(|| vec![recipe_issue("Waterfall has no low basin overlook")])?;
    let anchors = BTreeMap::from([
        (PARTY_START.to_owned(), party_start),
        (HOSTILE_START.to_owned(), hostile_start),
        (FALL_OVERLOOK.to_owned(), fall_overlook),
        (BASIN_OVERLOOK.to_owned(), basin_overlook),
    ]);
    let biome_regions = volume
        .surfaces
        .keys()
        .copied()
        .map(|surface| (surface, biome_region))
        .collect();
    let view_hint = waterfall_view_hint(layout.grid_radius, level_height)?;

    Ok(GeneratedWorldPlan {
        layout,
        volume,
        liquids: LiquidPlan {
            bodies: BTreeMap::from([(
                LiquidBodyId(0),
                LiquidBodyPlan {
                    material: FillMaterialRole::Water,
                    nodes: water_nodes,
                },
            )]),
        },
        features: FeaturePlan::default(),
        structures: StructurePlan::default(),
        blockers: BTreeSet::new(),
        lights: BTreeMap::new(),
        biome_regions,
        interiors: InteriorPlan::default(),
        anchors,
        view_hint,
    })
}

fn waterfall_view_hint(
    grid_radius: u32,
    level_height: f32,
) -> Result<MapViewHint, Vec<WorldValidationIssue>> {
    let radius = u16::try_from(grid_radius).map_err(|error| {
        vec![recipe_issue(format!(
            "Waterfall radius is too large: {error}"
        ))]
    })?;
    let focus_height = 20.0 * level_height;
    let frame = (f32::from(radius) * 3.5).max(8.0 * level_height * 3.0);
    Ok(MapViewHint::new(
        (frame, focus_height + frame, 0.0),
        (0.0, focus_height, 0.0),
    ))
}

#[derive(Debug, Clone, Copy)]
struct WaterCell {
    bed_level: i32,
    fill_bottom: i32,
    top: TilePos,
    state: LiquidFlowState,
    downstream: Option<TilePos>,
}

fn water_cell(coord: HexCoord, next: Option<HexCoord>) -> WaterCell {
    let (bed_level, fill_bottom, top_level, state) = if coord.x() < FALL_SOURCE_X {
        let state = if coord.x() <= BYPASS_HIGH_X {
            LiquidFlowState::Still
        } else {
            LiquidFlowState::Rapid
        };
        (
            HIGH_WATER_LEVEL - 1,
            HIGH_WATER_LEVEL,
            HIGH_WATER_LEVEL,
            state,
        )
    } else if coord.x() == FALL_SOURCE_X {
        (
            LOW_WATER_LEVEL - 1,
            LOW_WATER_LEVEL,
            HIGH_WATER_LEVEL,
            LiquidFlowState::Fall,
        )
    } else {
        let state = if coord.x() <= BYPASS_LOW_X {
            LiquidFlowState::Still
        } else if next.is_some() {
            LiquidFlowState::Current
        } else {
            LiquidFlowState::Still
        };
        (LOW_WATER_LEVEL - 1, LOW_WATER_LEVEL, LOW_WATER_LEVEL, state)
    };
    let top = TilePos::new(coord, top_level);
    let downstream = next.map(|next_coord| {
        let next_level = if next_coord.x() < FALL_TARGET_X {
            HIGH_WATER_LEVEL
        } else {
            LOW_WATER_LEVEL
        };
        TilePos::new(next_coord, next_level)
    });
    WaterCell {
        bed_level,
        fill_bottom,
        top,
        state,
        downstream,
    }
}

#[derive(Debug, Clone)]
struct Watercourse {
    main_lanes: Vec<Vec<HexCoord>>,
    basin: BTreeSet<HexCoord>,
}

impl Watercourse {
    fn coordinates(&self) -> BTreeSet<HexCoord> {
        self.main_lanes
            .iter()
            .flatten()
            .copied()
            .chain(self.basin.iter().copied())
            .collect()
    }
}

fn watercourse(mask: &BTreeSet<HexCoord>) -> Result<Watercourse, Vec<WorldValidationIssue>> {
    let mut rows = BTreeMap::<i32, Vec<HexCoord>>::new();
    for coord in mask {
        if coord.y().abs() <= BASIN_MAX_HALF_WIDTH {
            rows.entry(coord.y()).or_default().push(*coord);
        }
    }
    let mut main_lanes = Vec::new();
    let inlet_x = (-WATER_HALF_WIDTH..=WATER_HALF_WIDTH)
        .filter_map(|y| rows.get(&y))
        .filter_map(|row| row.iter().map(|coord| coord.x()).min())
        .max()
        .and_then(|minimum| minimum.checked_add(INLET_MARGIN))
        .ok_or_else(|| vec![recipe_issue("Waterfall has no common inlet position")])?;
    for y in -WATER_HALF_WIDTH..=WATER_HALF_WIDTH {
        let Some(row) = rows.get(&y) else {
            return Err(vec![recipe_issue(format!(
                "Waterfall mask has insufficient width on water lane y={y}"
            ))]);
        };
        if row.len() < 3 {
            return Err(vec![recipe_issue(format!(
                "Waterfall mask has insufficient length on water lane y={y}"
            ))]);
        }
        let mut lane: Vec<_> = row
            .iter()
            .copied()
            .filter(|coord| coord.x() >= inlet_x)
            .collect();
        lane.sort_unstable_by_key(|coord| coord.x());
        if !lane.windows(2).all(|pair| {
            let [first, second] = pair else {
                return false;
            };
            first.distance(*second) == 1
        }) || !lane.iter().any(|coord| coord.x() == FALL_SOURCE_X)
            || !lane.iter().any(|coord| coord.x() == FALL_TARGET_X)
        {
            return Err(vec![recipe_issue(format!(
                "Waterfall mask cannot realize a contiguous central lane at y={y}"
            ))]);
        }
        main_lanes.push(lane);
    }

    let basin: BTreeSet<_> = (FALL_TARGET_X..=BASIN_END_X)
        .flat_map(|x| {
            let half_width = if x <= BASIN_WIDE_END_X {
                BASIN_MAX_HALF_WIDTH
            } else {
                BASIN_MAX_HALF_WIDTH - 1
            };
            (-half_width..=half_width).map(move |y| HexCoord::from_axial(x, y))
        })
        .collect();
    if basin.iter().any(|coord| !mask.contains(coord)) {
        return Err(vec![recipe_issue(
            "Waterfall mask cannot fit the required widened plunge basin",
        )]);
    }
    Ok(Watercourse { main_lanes, basin })
}

fn bypass_tiles(
    grid_radius: u32,
    mask: &BTreeSet<HexCoord>,
) -> Result<[Vec<TilePos>; 2], Vec<WorldValidationIssue>> {
    bypass_tiles_on_bank(grid_radius, mask)
}

fn secondary_bypass_tiles(
    grid_radius: u32,
    mask: &BTreeSet<HexCoord>,
) -> Result<[Vec<TilePos>; 2], Vec<WorldValidationIssue>> {
    let radius = i32::try_from(grid_radius).unwrap_or(i32::MAX);
    let offset = (radius / 3 + 1).min(6);
    let lane = |lane_y| {
        (SECONDARY_HIGH_X..=SECONDARY_LOW_X)
            .map(|x| TilePos::new(HexCoord::from_axial(x, lane_y), secondary_slope_level(x)))
            .collect::<Vec<_>>()
    };
    let lanes = [lane(offset - 1), lane(offset)];
    if lanes
        .iter()
        .flatten()
        .any(|position| !mask.contains(&position.coord))
    {
        return Err(vec![recipe_issue(
            "Waterfall mask cannot fit the longer two-wide secondary dry bypass",
        )]);
    }
    Ok(lanes)
}

fn bypass_tiles_on_bank(
    grid_radius: u32,
    mask: &BTreeSet<HexCoord>,
) -> Result<[Vec<TilePos>; 2], Vec<WorldValidationIssue>> {
    let radius = i32::try_from(grid_radius).unwrap_or(i32::MAX);
    let offset = (radius / 3 + 1).min(6);
    let y = -offset;
    let lane = |lane_y| {
        (BYPASS_HIGH_X..=BYPASS_LOW_X)
            .map(|x| {
                let level = HIGH_LAND_LEVEL - (x - BYPASS_HIGH_X);
                TilePos::new(HexCoord::from_axial(x, lane_y), level)
            })
            .collect::<Vec<_>>()
    };
    let lanes = [lane(y), lane(y.saturating_add(1))];
    if lanes
        .iter()
        .flatten()
        .any(|position| !mask.contains(&position.coord))
    {
        return Err(vec![recipe_issue(
            "Waterfall mask cannot fit both required two-wide dry bypasses",
        )]);
    }
    Ok(lanes)
}

fn secondary_slope_level(x: i32) -> i32 {
    let step = x.saturating_sub(SECONDARY_HIGH_X);
    let span = SECONDARY_LOW_X.saturating_sub(SECONDARY_HIGH_X).max(1);
    let drop = step
        .saturating_mul(HIGH_LAND_LEVEL.saturating_sub(LOW_LAND_LEVEL))
        .checked_div(span)
        .unwrap_or_default();
    HIGH_LAND_LEVEL.saturating_sub(drop)
}

fn secondary_slope_apron(
    grid_radius: u32,
    mask: &BTreeSet<HexCoord>,
) -> Result<Vec<TilePos>, Vec<WorldValidationIssue>> {
    let radius = i32::try_from(grid_radius).unwrap_or(i32::MAX);
    let offset = (radius / 3 + 1).min(6);
    let mut apron: Vec<_> = ((SECONDARY_HIGH_X + 1)..SECONDARY_LOW_X)
        .map(|x| {
            TilePos::new(
                HexCoord::from_axial(x, offset.saturating_add(1)),
                secondary_slope_level(x),
            )
        })
        .chain(((SECONDARY_HIGH_X + 3)..=(SECONDARY_LOW_X - 3)).map(|x| {
            TilePos::new(
                HexCoord::from_axial(x, offset.saturating_add(2)),
                secondary_slope_level(x),
            )
        }))
        .collect();
    apron.sort_unstable();
    apron.dedup();
    if apron.iter().any(|position| !mask.contains(&position.coord)) {
        return Err(vec![recipe_issue(
            "Waterfall mask cannot fit the widened secondary-slope apron",
        )]);
    }
    Ok(apron)
}

fn land_surface_level(
    coord: HexCoord,
    bypass: &BTreeMap<HexCoord, i32>,
    relief: Option<&ReliefPlan>,
) -> i32 {
    if let Some(level) = bypass.get(&coord) {
        return *level;
    }

    let base = if coord.x() < FALL_TARGET_X {
        HIGH_LAND_LEVEL
    } else {
        LOW_LAND_LEVEL
    };
    base + relief.map_or(0, |relief| relief.height_at(coord))
}

#[derive(Debug)]
struct ReliefPlan {
    centers: BTreeSet<HexCoord>,
    protected: BTreeSet<HexCoord>,
    inner_radius: u32,
    outer_radius: u32,
}

impl ReliefPlan {
    fn new(
        grid_radius: u32,
        mask: &BTreeSet<HexCoord>,
        water: &BTreeSet<HexCoord>,
        bypass: &BTreeMap<HexCoord, i32>,
        stream: SeedStream<'_>,
    ) -> Self {
        let protected: BTreeSet<_> = bypass.keys().copied().collect();
        let mut centers = BTreeSet::new();
        for (bank, high_bank) in [false, true].into_iter().enumerate() {
            let candidates: Vec<_> = mask
                .iter()
                .copied()
                .filter(|coord| (coord.x() < FALL_TARGET_X) == high_bank)
                .filter(|coord| !water.contains(coord) && !protected.contains(coord))
                .filter(|coord| coord.distance(HexCoord::ORIGIN).saturating_add(2) <= grid_radius)
                .collect();
            if candidates.is_empty() {
                continue;
            }
            let candidate_count = u64::try_from(candidates.len()).unwrap_or(u64::MAX);
            for local in 0..RELIEF_CENTERS_PER_BANK {
                let sample = stream.sample(
                    u64::try_from(bank)
                        .unwrap_or_default()
                        .saturating_mul(RELIEF_CENTERS_PER_BANK)
                        .saturating_add(local),
                );
                let index = usize::try_from(sample % candidate_count).unwrap_or_default();
                if let Some(center) = candidates.get(index) {
                    centers.insert(*center);
                }
            }
        }
        let outer_radius = (grid_radius / 5).clamp(3, 8);
        Self {
            centers,
            protected,
            inner_radius: (outer_radius / 2).max(1),
            outer_radius,
        }
    }

    fn height_at(&self, coord: HexCoord) -> i32 {
        let mound = self
            .centers
            .iter()
            .map(|center| {
                let distance = center.distance(coord);
                if distance <= self.inner_radius {
                    2
                } else if distance <= self.outer_radius {
                    1
                } else {
                    0
                }
            })
            .max()
            .unwrap_or_default();
        let protected_distance = self
            .protected
            .iter()
            .map(|protected| protected.distance(coord))
            .min()
            .unwrap_or(u32::MAX);
        mound.min(i32::try_from(protected_distance).unwrap_or(i32::MAX))
    }
}

fn land_column(surface: i32) -> VolumeColumn {
    VolumeColumn {
        elements: vec![
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(0, 1),
                material: SolidMaterialRole::Bedrock,
                cutaway_for: None,
            }),
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(1, surface - 2),
                material: SolidMaterialRole::Stone,
                cutaway_for: None,
            }),
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(surface - 2, surface),
                material: SolidMaterialRole::Dirt,
                cutaway_for: None,
            }),
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(surface, surface + 1),
                material: SolidMaterialRole::Grass,
                cutaway_for: None,
            }),
        ],
    }
}

fn water_column(cell: WaterCell) -> (VolumeColumn, TilePos) {
    let bed = TilePos::new(cell.top.coord, cell.bed_level);
    (
        VolumeColumn {
            elements: vec![
                VolumeElement::Solid(SolidMass {
                    levels: LevelInterval::new(0, 1),
                    material: SolidMaterialRole::Bedrock,
                    cutaway_for: None,
                }),
                VolumeElement::Solid(SolidMass {
                    levels: LevelInterval::new(1, cell.bed_level),
                    material: SolidMaterialRole::Stone,
                    cutaway_for: None,
                }),
                VolumeElement::Solid(SolidMass {
                    levels: LevelInterval::new(cell.bed_level, cell.bed_level + 1),
                    material: SolidMaterialRole::Gravel,
                    cutaway_for: None,
                }),
                VolumeElement::Fill(NonSolidFill {
                    levels: LevelInterval::new(cell.fill_bottom, cell.top.level + 1),
                    material: FillMaterialRole::Water,
                }),
            ],
        },
        bed,
    )
}

fn validate_waterfall(plan: &GeneratedWorldPlan) -> WorldValidation<WaterfallMetrics> {
    let mut issues = Vec::new();
    let Some(body) = plan.liquids.bodies.get(&LiquidBodyId(0)) else {
        return WorldValidation::Invalid(vec![recipe_issue(
            "Waterfall has no canonical water body",
        )]);
    };
    if plan.liquids.bodies.len() != 1 || body.material != FillMaterialRole::Water {
        issues.push(recipe_issue(
            "Waterfall must contain exactly one water-only liquid body",
        ));
    }

    let mut calm_nodes = 0_u32;
    let mut current_nodes = 0_u32;
    let mut rapid_nodes = 0_u32;
    let mut fall_nodes = Vec::new();
    for (position, node) in &body.nodes {
        match node.state {
            LiquidFlowState::Still => calm_nodes = calm_nodes.saturating_add(1),
            LiquidFlowState::Current => current_nodes = current_nodes.saturating_add(1),
            LiquidFlowState::Rapid => rapid_nodes = rapid_nodes.saturating_add(1),
            LiquidFlowState::Fall => fall_nodes.push((*position, node.downstream)),
        }
    }
    let expected_watercourse = match watercourse(&plan.layout.footprint) {
        Ok(watercourse) => Some(watercourse),
        Err(mut watercourse_issues) => {
            issues.append(&mut watercourse_issues);
            None
        }
    };
    validate_flow_stages(body, expected_watercourse.as_ref(), &mut issues);
    if calm_nodes < 9 || current_nodes < 3 || rapid_nodes < 3 {
        issues.push(recipe_issue(
            "Waterfall must realize calm inlet/basin, rapid, and current stages",
        ));
    }
    let fall_height = validate_fall(&fall_nodes, &mut issues);
    let surfaces_by_coord: BTreeMap<_, _> = plan
        .volume
        .surfaces
        .iter()
        .map(|(position, metadata)| (position.coord, (*position, *metadata)))
        .collect();
    if surfaces_by_coord.len() != plan.volume.surfaces.len() {
        issues.push(recipe_issue(
            "Waterfall must publish exactly one semantic surface per column",
        ));
    }
    validate_liquid_beds(plan, body, &surfaces_by_coord, &mut issues);

    let bypass = match bypass_tiles(plan.layout.grid_radius, &plan.layout.footprint) {
        Ok(bypass) => bypass,
        Err(mut bypass_issues) => {
            issues.append(&mut bypass_issues);
            [Vec::new(), Vec::new()]
        }
    };
    let secondary_bypass =
        match secondary_bypass_tiles(plan.layout.grid_radius, &plan.layout.footprint) {
            Ok(bypass) => bypass,
            Err(mut bypass_issues) => {
                issues.append(&mut bypass_issues);
                [Vec::new(), Vec::new()]
            }
        };
    let secondary_apron =
        match secondary_slope_apron(plan.layout.grid_radius, &plan.layout.footprint) {
            Ok(apron) => apron,
            Err(mut apron_issues) => {
                issues.append(&mut apron_issues);
                Vec::new()
            }
        };
    let ordinary = OrdinaryGraph::from_plan(plan);
    validate_bypass(plan, &ordinary, &bypass, "critical", 9, &mut issues);
    validate_bypass(
        plan,
        &ordinary,
        &secondary_bypass,
        "secondary",
        11,
        &mut issues,
    );
    validate_secondary_apron(plan, &ordinary, &secondary_apron, &mut issues);
    validate_route_redundancy(&ordinary, &bypass, &secondary_bypass, &mut issues);

    let party = plan.anchors.get(PARTY_START).copied();
    let conflict = plan.anchors.get(HOSTILE_START).copied();
    for name in [FALL_OVERLOOK, BASIN_OVERLOOK] {
        if !plan.anchors.contains_key(name) {
            issues.push(recipe_issue(format!(
                "Waterfall is missing required review anchor {name:?}"
            )));
        }
    }
    let distances = party
        .filter(|start| ordinary.contains(*start))
        .map(|start| ordinary.distances_from(start));
    let route_length = match (distances.as_ref(), conflict) {
        (Some(distances), Some(goal)) => distances.get(&goal).copied(),
        _ => None,
    };
    if route_length.is_none() {
        issues.push(recipe_issue(
            "Waterfall critical anchors are not joined by ordinary traversal",
        ));
    }
    if distances
        .as_ref()
        .is_none_or(|distances| distances.len() != ordinary.len())
    {
        issues.push(recipe_issue(
            "Waterfall ordinary network leaves one or more surfaces disconnected",
        ));
    }

    let bypass_coords: BTreeSet<_> = bypass
        .iter()
        .chain(&secondary_bypass)
        .flatten()
        .chain(&secondary_apron)
        .map(|position| position.coord)
        .collect();
    let raised_terrain = ordinary
        .positions()
        .filter(|position| {
            if bypass_coords.contains(&position.coord) {
                return false;
            }
            let base = if position.coord.x() < FALL_TARGET_X {
                HIGH_LAND_LEVEL
            } else {
                LOW_LAND_LEVEL
            };
            position.level > base
        })
        .count();
    let dry_levels: BTreeSet<_> = ordinary
        .positions()
        .map(|position| position.level)
        .collect();
    let dry_relief = dry_levels
        .first()
        .zip(dry_levels.last())
        .map_or(0, |(minimum, maximum)| maximum.saturating_sub(*minimum));
    let spawn_height_difference = party
        .zip(conflict)
        .map_or(0, |(party, hostile)| party.level.abs_diff(hostile.level));
    let high_bank = ordinary
        .positions()
        .filter(|position| position.coord.x() < FALL_TARGET_X)
        .map(|position| position.level)
        .max();
    let low_bank = ordinary
        .positions()
        .filter(|position| position.coord.x() >= FALL_TARGET_X)
        .map(|position| position.level)
        .max();
    let bank_high_ground_difference = high_bank
        .zip(low_bank)
        .map_or(0, |(high, low)| high.abs_diff(low));
    let grass_surface_percent = count_u32(ordinary.len())
        .saturating_mul(100)
        .checked_div(count_u32(plan.volume.surfaces.len()))
        .unwrap_or_default();

    let metrics = WaterfallMetrics {
        water_nodes: count_u32(body.nodes.len()),
        calm_nodes,
        current_nodes,
        rapid_nodes,
        fall_nodes: count_u32(fall_nodes.len()),
        fall_height,
        ordinary_surfaces: count_u32(ordinary.len()),
        reachable_elevation_levels: count_u32(
            ordinary
                .positions()
                .map(|position| position.level)
                .collect::<BTreeSet<_>>()
                .len(),
        ),
        bypass_steps: route_length.unwrap_or_default(),
        alternate_bypass_steps: secondary_bypass
            .first()
            .map_or(0, |lane| count_u32(lane.len().saturating_sub(1))),
        raised_terrain: count_u32(raised_terrain),
        dry_relief: u32::try_from(dry_relief).unwrap_or(u32::MAX),
        spawn_height_difference,
        bank_high_ground_difference,
        grass_surface_percent,
    };
    if issues.is_empty() {
        WorldValidation::Valid(metrics)
    } else {
        WorldValidation::Invalid(issues)
    }
}

fn validate_flow_stages(
    body: &LiquidBodyPlan,
    watercourse: Option<&Watercourse>,
    issues: &mut Vec<WorldValidationIssue>,
) {
    let Some(watercourse) = watercourse else {
        return;
    };
    let actual_coords: BTreeSet<_> = body.nodes.keys().map(|position| position.coord).collect();
    let expected_coords = watercourse.coordinates();
    if actual_coords != expected_coords {
        issues.push(recipe_issue(
            "Waterfall liquid nodes do not exactly cover the three lanes and widened basin",
        ));
    }

    let mut main_coords = BTreeSet::new();
    for lane in &watercourse.main_lanes {
        for (index, coord) in lane.iter().copied().enumerate() {
            main_coords.insert(coord);
            let next = lane.get(index.saturating_add(1)).copied();
            let expected = water_cell(coord, next);
            let Some(node) = body.nodes.get(&expected.top) else {
                issues.push(recipe_issue(format!(
                    "Waterfall main lane is missing exact node {:?}",
                    expected.top
                )));
                continue;
            };
            if node.state != expected.state || node.downstream != expected.downstream {
                issues.push(recipe_issue(format!(
                    "Waterfall main-lane stage at {:?} is {:?} -> {:?}, expected {:?} -> {:?}",
                    expected.top, node.state, node.downstream, expected.state, expected.downstream
                )));
            }
        }
    }
    for coord in watercourse
        .basin
        .iter()
        .filter(|coord| !main_coords.contains(coord))
    {
        let position = TilePos::new(*coord, LOW_WATER_LEVEL);
        if !matches!(
            body.nodes.get(&position),
            Some(LiquidNode {
                state: LiquidFlowState::Still,
                downstream: None,
            })
        ) {
            issues.push(recipe_issue(format!(
                "Waterfall plunge-basin cell {position:?} is not authored still water"
            )));
        }
    }
    for lane in &watercourse.main_lanes {
        if let Some(last) = lane.last() {
            let terminal = TilePos::new(*last, LOW_WATER_LEVEL);
            if last.neighbors().into_iter().any(|neighbor| {
                neighbor.y() == last.y()
                    && neighbor.x() > last.x()
                    && expected_coords.contains(&neighbor)
            }) || !matches!(
                body.nodes.get(&terminal),
                Some(LiquidNode {
                    state: LiquidFlowState::Still,
                    downstream: None,
                })
            ) {
                issues.push(recipe_issue(format!(
                    "Waterfall outlet lane y={} does not terminate as still water at the world edge",
                    last.y()
                )));
            }
        }
    }
}

fn validate_fall(
    fall_nodes: &[(TilePos, Option<TilePos>)],
    issues: &mut Vec<WorldValidationIssue>,
) -> u32 {
    if fall_nodes.len() != 3 {
        issues.push(recipe_issue(
            "Waterfall must have one contiguous three-wide fall",
        ));
        return 0;
    }
    let coords: BTreeSet<_> = fall_nodes
        .iter()
        .map(|(position, _)| position.coord)
        .collect();
    let connected = coords.iter().all(|coord| {
        coords.len() == 1
            || coord
                .neighbors()
                .into_iter()
                .any(|neighbor| coords.contains(&neighbor))
    });
    if !connected {
        issues.push(recipe_issue("Waterfall fall curtain is not contiguous"));
    }

    let drops: BTreeSet<_> = fall_nodes
        .iter()
        .filter_map(|(position, downstream)| {
            downstream.map(|downstream| position.level.saturating_sub(downstream.level))
        })
        .collect();
    let expected = HIGH_WATER_LEVEL.saturating_sub(LOW_WATER_LEVEL);
    if drops != BTreeSet::from([expected]) {
        issues.push(recipe_issue(format!(
            "Waterfall fall must descend exactly {expected} levels in every lane"
        )));
        0
    } else {
        u32::try_from(expected).unwrap_or_default()
    }
}

fn validate_liquid_beds(
    plan: &GeneratedWorldPlan,
    body: &LiquidBodyPlan,
    surfaces_by_coord: &BTreeMap<HexCoord, (TilePos, SurfaceMetadata)>,
    issues: &mut Vec<WorldValidationIssue>,
) {
    for position in body.nodes.keys() {
        let bed = surfaces_by_coord.get(&position.coord);
        if !matches!(
            bed,
            Some(&(
                _,
                SurfaceMetadata {
                    access: SurfaceAccess::NonStandable,
                    ..
                }
            ))
        ) {
            issues.push(recipe_issue(format!(
                "water column {:?} does not publish a non-standable bed",
                position.coord
            )));
            continue;
        }
        let Some((bed, _metadata)) = bed else {
            continue;
        };
        let gravel = plan
            .volume
            .columns
            .get(&position.coord)
            .is_some_and(|column| {
                column.elements.iter().any(|element| {
                    matches!(
                        element,
                        VolumeElement::Solid(SolidMass {
                            levels,
                            material: SolidMaterialRole::Gravel,
                            ..
                        }) if levels.top == bed.level.saturating_add(1)
                    )
                })
            });
        if !gravel {
            issues.push(recipe_issue(format!(
                "water column {:?} does not retain its gravel-topped bed",
                position.coord
            )));
        }
    }
}

fn validate_bypass(
    plan: &GeneratedWorldPlan,
    ordinary: &OrdinaryGraph,
    bypass: &[Vec<TilePos>; 2],
    name: &str,
    expected_length: usize,
    issues: &mut Vec<WorldValidationIssue>,
) {
    if bypass.iter().any(|lane| lane.len() != expected_length) {
        issues.push(recipe_issue(format!(
            "Waterfall {name} bypass must contain two complete {expected_length}-tile lanes"
        )));
        return;
    }
    for lane in bypass {
        for expected in lane {
            if plan
                .volume
                .surfaces
                .get(expected)
                .map(|metadata| metadata.access)
                != Some(SurfaceAccess::Ordinary)
            {
                issues.push(recipe_issue(format!(
                    "Waterfall {name} bypass tile {expected:?} is not ordinary footing"
                )));
            }
        }
        for pair in lane.windows(2) {
            let [first, second] = pair else {
                continue;
            };
            if !ordinary.admits(*first, *second) {
                issues.push(recipe_issue(format!(
                    "Waterfall {name} bypass transition {:?} -> {:?} is not walker-admitted",
                    first, second,
                )));
            }
        }
    }
    let [first_lane, second_lane] = bypass;
    for (index, (first, second)) in first_lane.iter().zip(second_lane).enumerate() {
        if !ordinary.admits(*first, *second) {
            issues.push(recipe_issue(format!(
                "Waterfall {name} bypass loses its second lane at step {index}"
            )));
        }
    }
}

fn validate_secondary_apron(
    plan: &GeneratedWorldPlan,
    ordinary: &OrdinaryGraph,
    apron: &[TilePos],
    issues: &mut Vec<WorldValidationIssue>,
) {
    if apron.len() != 14 {
        issues.push(recipe_issue(
            "Waterfall secondary slope must retain its irregular fourteen-tile apron",
        ));
    }
    let apron_set: BTreeSet<_> = apron.iter().copied().collect();
    for expected in apron {
        if plan
            .volume
            .surfaces
            .get(expected)
            .map(|metadata| metadata.access)
            != Some(SurfaceAccess::Ordinary)
        {
            issues.push(recipe_issue(format!(
                "Waterfall secondary-slope apron tile {expected:?} is not ordinary footing"
            )));
            continue;
        }
        if !ordinary.neighbors.get(expected).is_some_and(|neighbors| {
            neighbors
                .iter()
                .any(|neighbor| apron_set.contains(neighbor) || neighbor.level == expected.level)
        }) {
            issues.push(recipe_issue(format!(
                "Waterfall secondary-slope apron tile {expected:?} is not integrated into the terrace"
            )));
        }
    }
}

fn validate_route_redundancy(
    ordinary: &OrdinaryGraph,
    critical: &[Vec<TilePos>; 2],
    secondary: &[Vec<TilePos>; 2],
    issues: &mut Vec<WorldValidationIssue>,
) {
    for (name, removed, retained) in [
        ("critical", critical, secondary),
        ("secondary", secondary, critical),
    ] {
        let blocked: BTreeSet<_> = removed.iter().flatten().copied().collect();
        let Some((start, goal)) = retained
            .first()
            .and_then(|lane| lane.first().copied().zip(lane.last().copied()))
        else {
            issues.push(recipe_issue(format!(
                "Waterfall has no alternate route around its {name} bypass"
            )));
            continue;
        };
        let reachable = ordinary.reachable_avoiding(start, &blocked);
        if !reachable.contains(&goal) {
            issues.push(recipe_issue(format!(
                "removing the Waterfall {name} bypass disconnects the alternate high/low route"
            )));
        }
    }
}

#[derive(Debug)]
struct OrdinaryGraph {
    positions_by_coord: BTreeMap<HexCoord, TilePos>,
    neighbors: BTreeMap<TilePos, Vec<TilePos>>,
}

impl OrdinaryGraph {
    fn from_plan(plan: &GeneratedWorldPlan) -> Self {
        let positions_by_coord: BTreeMap<_, _> = plan
            .volume
            .surfaces
            .iter()
            .filter_map(|(position, metadata)| {
                (metadata.access == SurfaceAccess::Ordinary).then_some((position.coord, *position))
            })
            .collect();
        let endpoints: BTreeMap<_, _> = positions_by_coord
            .values()
            .copied()
            .map(|position| {
                let headroom = plan.volume.surface_headroom(position).unwrap_or_default();
                (position, TraversalEndpoint::new(position, true, headroom))
            })
            .collect();
        let mut neighbors: BTreeMap<_, Vec<_>> = endpoints
            .keys()
            .copied()
            .map(|position| (position, Vec::new()))
            .collect();

        for (coord, from) in &positions_by_coord {
            for neighbor_coord in coord.neighbors() {
                if neighbor_coord <= *coord {
                    continue;
                }
                let Some(to) = positions_by_coord.get(&neighbor_coord).copied() else {
                    continue;
                };
                let Some(from_endpoint) = endpoints.get(from).copied() else {
                    continue;
                };
                let Some(to_endpoint) = endpoints.get(&to).copied() else {
                    continue;
                };
                if TraversalProfile::WALKER.admits_transition(from_endpoint, to_endpoint)
                    && TraversalProfile::WALKER.admits_transition(to_endpoint, from_endpoint)
                {
                    if let Some(from_neighbors) = neighbors.get_mut(from) {
                        from_neighbors.push(to);
                    }
                    if let Some(to_neighbors) = neighbors.get_mut(&to) {
                        to_neighbors.push(*from);
                    }
                }
            }
        }
        for adjacent in neighbors.values_mut() {
            adjacent.sort_unstable();
        }
        Self {
            positions_by_coord,
            neighbors,
        }
    }

    fn len(&self) -> usize {
        self.positions_by_coord.len()
    }

    fn contains(&self, position: TilePos) -> bool {
        self.neighbors.contains_key(&position)
    }

    fn positions(&self) -> impl Iterator<Item = TilePos> + '_ {
        self.positions_by_coord.values().copied()
    }

    fn admits(&self, from: TilePos, to: TilePos) -> bool {
        self.neighbors
            .get(&from)
            .is_some_and(|neighbors| neighbors.binary_search(&to).is_ok())
    }

    fn distances_from(&self, start: TilePos) -> BTreeMap<TilePos, u32> {
        let mut distances = BTreeMap::from([(start, 0_u32)]);
        let mut frontier = VecDeque::from([start]);
        while let Some(position) = frontier.pop_front() {
            let Some(distance) = distances.get(&position).copied() else {
                continue;
            };
            let Some(neighbors) = self.neighbors.get(&position) else {
                continue;
            };
            for neighbor in neighbors {
                if distances.contains_key(neighbor) {
                    continue;
                }
                distances.insert(*neighbor, distance.saturating_add(1));
                frontier.push_back(*neighbor);
            }
        }
        distances
    }

    fn reachable_avoiding(&self, start: TilePos, blocked: &BTreeSet<TilePos>) -> BTreeSet<TilePos> {
        if blocked.contains(&start) {
            return BTreeSet::new();
        }
        let mut reachable = BTreeSet::from([start]);
        let mut frontier = VecDeque::from([start]);
        while let Some(position) = frontier.pop_front() {
            let Some(neighbors) = self.neighbors.get(&position) else {
                continue;
            };
            for neighbor in neighbors {
                if !blocked.contains(neighbor) && reachable.insert(*neighbor) {
                    frontier.push_back(*neighbor);
                }
            }
        }
        reachable
    }
}

fn recipe_issue(detail: impl Into<String>) -> WorldValidationIssue {
    WorldValidationIssue::new(WorldIssueCode::Recipe("waterfall"), detail)
}

fn count_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::procedural_v3::fingerprint::semantic_plan_fingerprint;
    use crate::settings::{
        CubeCoord, PatchEdgeContractSettings, PatchEdgesSettings, PatchMaskSettings, PatchSpec,
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
        let boundary = || PatchEdgeContractSettings::WorldBoundary;
        ProceduralV3Settings {
            layout: V3LayoutSettings::Single(PatchSpec {
                environment: V3EnvironmentSettings::TemperateGrassland,
                recipe: V3RecipeSettings::Waterfall(V3WaterfallSettings),
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

    #[test]
    fn fixed_corpus_builds_valid_waterfalls_at_supported_radii() {
        for radius in [12, 20, 40] {
            for seed in [0, 1, 808, 4_294_967_311] {
                let selected =
                    generate(radius, 0.4, &settings(), seed).expect("Waterfall should generate");
                assert!(!selected.used_fallback);
                assert_eq!(selected.metrics.fall_nodes, 3);
                assert_eq!(selected.metrics.fall_height, 8);
                assert_eq!(selected.metrics.bypass_steps, 8);
                assert_eq!(selected.validated.plan.validate(), Vec::new());
            }
        }
    }

    #[test]
    fn authored_flow_contains_every_required_stage_and_exact_three_wide_fall() {
        let selected = generate(12, 0.4, &settings(), 77).expect("Waterfall should generate");
        let metrics = &selected.metrics;

        assert!(metrics.calm_nodes >= 9);
        assert!(metrics.current_nodes >= 3);
        assert!(metrics.rapid_nodes >= 3);
        assert_eq!(metrics.fall_nodes, 3);
        assert_eq!(metrics.fall_height, 8);
        assert!(metrics.water_nodes > 30);
    }

    #[test]
    fn inlet_basin_and_boundary_outlet_have_exact_geometry() {
        let selected = generate(12, 0.4, &settings(), 77).expect("Waterfall should generate");
        let plan = &selected.validated.plan;
        let course = watercourse(&plan.layout.footprint).expect("fixed watercourse");
        let ordinary = OrdinaryGraph::from_plan(plan);

        let inlet_xs: BTreeSet<_> = course
            .main_lanes
            .iter()
            .filter_map(|lane| lane.first())
            .map(|coord| coord.x())
            .collect();
        assert_eq!(inlet_xs.len(), 1, "all inlet lanes start together");
        let inlet_x = inlet_xs.first().copied().expect("three inlet lanes");
        for x in [inlet_x - 2, inlet_x - 1] {
            let circulation: Vec<_> = (-2..=2)
                .map(|y| {
                    ordinary
                        .positions_by_coord
                        .get(&HexCoord::from_axial(x, y))
                        .copied()
                        .expect("the headwater margin should remain dry and ordinary")
                })
                .collect();
            assert!(
                circulation.windows(2).all(
                    |pair| matches!(pair, [first, second] if ordinary.admits(*first, *second))
                ),
                "the inlet needs two independent ordinary circulation lanes"
            );
        }

        for (x, expected_width) in [(0, 7), (1, 7), (2, 5), (3, 5)] {
            assert_eq!(
                course.basin.iter().filter(|coord| coord.x() == x).count(),
                expected_width,
                "unexpected plunge-basin width at x={x}"
            );
        }
        assert!(course.basin.iter().all(|coord| {
            course.basin.len() == 1
                || coord
                    .neighbors()
                    .into_iter()
                    .any(|neighbor| course.basin.contains(&neighbor))
        }));

        for lane in &course.main_lanes {
            let last = lane.last().copied().expect("every lane has an outlet");
            assert!(
                !plan
                    .layout
                    .footprint
                    .contains(&super::super::layout::HexSide::East.neighbor(last)),
                "each low-water lane must terminate on the resolved east boundary"
            );
        }
    }

    #[test]
    fn bypass_is_two_wide_climbable_and_connects_every_ordinary_surface() {
        let selected = generate(12, 0.4, &settings(), 91).expect("Waterfall should generate");
        let plan = &selected.validated.plan;
        let bypass = bypass_tiles(12, &plan.layout.footprint).expect("fixed bypass");
        let secondary =
            secondary_bypass_tiles(12, &plan.layout.footprint).expect("secondary bypass");
        let ordinary = OrdinaryGraph::from_plan(plan);

        for (route, expected_length) in [(&bypass, 9), (&secondary, 11)] {
            for lane in route {
                assert_eq!(lane.len(), expected_length);
                assert!(lane.windows(2).all(|pair| {
                    matches!(
                        pair,
                        [first, second] if first.level.abs_diff(second.level) <= 1
                    )
                }));
                assert_eq!(
                    lane.first()
                        .zip(lane.last())
                        .map_or(0, |(first, last)| first.level.saturating_sub(last.level)),
                    8
                );
                assert!(lane.windows(2).all(|pair| {
                    matches!(
                        pair,
                        [first, second] if ordinary.admits(*first, *second)
                    )
                }));
                assert!(lane.iter().all(|position| {
                    !plan
                        .volume
                        .fill_runs_by_top()
                        .keys()
                        .any(|liquid| liquid.coord == position.coord)
                }));
            }
            let [first_lane, second_lane] = route;
            assert!(first_lane
                .iter()
                .zip(second_lane)
                .all(|(first, second)| ordinary.admits(*first, *second)));
        }
        let apron = secondary_slope_apron(12, &plan.layout.footprint).expect("slope apron");
        assert_eq!(apron.len(), 14);
        assert!(apron.iter().all(|position| ordinary.contains(*position)));
        let party = plan
            .anchors
            .get(PARTY_START)
            .copied()
            .expect("Waterfall should publish party_start");
        let expected: BTreeSet<_> = ordinary.positions().collect();
        let reachable: BTreeSet<_> = ordinary.distances_from(party).into_keys().collect();
        assert_eq!(
            reachable, expected,
            "the cached ordinary graph should reach every surface"
        );
    }

    #[test]
    fn every_water_column_has_a_non_standable_gravel_bed() {
        let selected = generate(12, 0.4, &settings(), 22).expect("Waterfall should generate");
        let plan = &selected.validated.plan;
        let body = plan
            .liquids
            .bodies
            .get(&LiquidBodyId(0))
            .expect("Waterfall should publish its canonical body");

        for node in body.nodes.keys() {
            let (bed, metadata) = plan
                .volume
                .surfaces
                .iter()
                .find(|(surface, _)| surface.coord == node.coord)
                .expect("every water column has a bed");
            assert_eq!(metadata.access, SurfaceAccess::NonStandable);
            assert!(matches!(
                plan.volume
                    .columns
                    .get(&bed.coord)
                    .and_then(|column| column.elements.get(2)),
                Some(VolumeElement::Solid(SolidMass {
                    material: SolidMaterialRole::Gravel,
                    ..
                }))
            ));
        }
    }

    #[test]
    fn named_relief_is_deterministic_and_seed_sensitive() {
        let first = generate(12, 0.4, &settings(), 12).expect("Waterfall should generate");
        let repeated = generate(12, 0.4, &settings(), 12).expect("Waterfall should repeat");
        let other = generate(12, 0.4, &settings(), 13).expect("other seed should generate");

        assert_eq!(
            first.validated.semantic_fingerprint,
            repeated.validated.semantic_fingerprint
        );
        assert_ne!(
            first.validated.semantic_fingerprint,
            other.validated.semantic_fingerprint
        );
        assert_eq!(
            semantic_plan_fingerprint(&first.validated.plan),
            Ok(first.validated.semantic_fingerprint)
        );
    }

    #[test]
    fn relief_forms_coherent_terraces_and_scales_with_radius() {
        let mut raised_counts = Vec::new();
        for (radius, expected_outer_radius) in [(12, 3), (20, 4), (40, 8)] {
            let layout = resolve_layout(radius, &settings()).expect("test layout should resolve");
            let course = watercourse(&layout.footprint).expect("test watercourse");
            let critical = bypass_tiles(radius, &layout.footprint).expect("critical bypass");
            let secondary =
                secondary_bypass_tiles(radius, &layout.footprint).expect("secondary bypass");
            let apron = secondary_slope_apron(radius, &layout.footprint).expect("secondary apron");
            let bypass: BTreeMap<_, _> = critical
                .iter()
                .chain(&secondary)
                .flatten()
                .chain(&apron)
                .map(|position| (position.coord, position.level))
                .collect();
            let relief = ReliefPlan::new(
                radius,
                &layout.footprint,
                &course.coordinates(),
                &bypass,
                SeedStreams::new(912_441, 3, PatchId(0).0).stage("waterfall.relief"),
            );
            assert_eq!(relief.outer_radius, expected_outer_radius);
            assert_eq!(relief.inner_radius, (expected_outer_radius / 2).max(1));

            let raised: BTreeSet<_> = layout
                .footprint
                .iter()
                .copied()
                .filter(|coord| relief.height_at(*coord) > 0)
                .collect();
            assert!(!raised.is_empty());
            assert!(
                raised.iter().all(|coord| coord
                    .neighbors()
                    .into_iter()
                    .any(|neighbor| raised.contains(&neighbor))),
                "terraced relief must not produce isolated one-cell pads"
            );
            assert!(raised
                .iter()
                .all(|coord| matches!(relief.height_at(*coord), 1 | 2)));
            raised_counts.push(raised.len());
        }
        let [compact, _medium, expanded] = raised_counts.as_slice() else {
            panic!("the relief test must cover exactly three radii");
        };
        assert!(
            expanded > compact,
            "radius-40 relief should occupy more terrain than radius-12 relief"
        );
    }

    #[test]
    fn generated_view_uses_world_space_level_height_and_rejects_invalid_scale() {
        let compact = generate(12, 0.4, &settings(), 12).expect("Waterfall should generate");
        let tall = generate(12, 0.8, &settings(), 12).expect("Waterfall should generate");
        assert_ne!(
            compact.validated.plan.view_hint,
            tall.validated.plan.view_hint
        );
        assert!(
            (compact.validated.plan.view_hint.focus.1 * 2.0
                - tall.validated.plan.view_hint.focus.1)
                .abs()
                <= f32::EPSILON
        );
        for invalid in [0.0, -0.4, f32::NAN, f32::INFINITY] {
            assert!(matches!(
                generate(12, invalid, &settings(), 12),
                Err(V3GenerationError::RecipeContract(_))
            ));
        }
    }

    #[test]
    fn forced_candidate_failure_uses_the_independent_canonical_fallback() {
        let selected = run_recipe(
            &WaterfallRecipe {
                level_height: 0.4,
                layout: resolve_layout(12, &settings()).expect("test layout should resolve"),
                reject_candidates: true,
            },
            &settings(),
            12,
            999,
        )
        .expect("canonical Waterfall fallback should be valid");

        assert!(selected.used_fallback);
        assert_eq!(selected.selected_candidate, None);
        assert_eq!(selected.valid_candidates, 0);
        assert_eq!(selected.metrics.raised_terrain, 0);
        assert_eq!(selected.metrics.fall_height, 8);
    }

    #[test]
    fn unsupported_layout_and_recipe_fail_explicitly() {
        let mut wrong = settings();
        let V3LayoutSettings::Single(patch) = &mut wrong.layout else {
            unreachable!()
        };
        patch.recipe = V3RecipeSettings::Forest(crate::settings::V3ForestSettings);

        assert!(matches!(
            generate(12, 0.4, &wrong, 1),
            Err(V3GenerationError::RecipeUnavailable("Forest"))
        ));
    }

    #[test]
    fn connected_explicit_mask_without_recipe_capacity_fails_before_selection() {
        let mut narrow = settings();
        let V3LayoutSettings::Single(patch) = &mut narrow.layout else {
            unreachable!()
        };
        patch.mask =
            PatchMaskSettings::Explicit((-12..=12).map(|x| CubeCoord { x, y: 0, z: -x }).collect());

        let error =
            generate(12, 0.4, &narrow, 1).expect_err("a one-wide mask cannot fit Waterfall");
        assert!(matches!(error, V3GenerationError::RecipeContract(_)));
        assert!(
            error
                .to_string()
                .contains("insufficient width on water lane"),
            "{error}"
        );
    }

    #[test]
    #[ignore = "10,000 seeds are a manual V3 Waterfall stress corpus"]
    fn ten_thousand_seed_corpus_has_less_than_one_percent_fallbacks() {
        let mut fallbacks = 0_u32;
        for seed in 0..10_000 {
            let selected =
                generate(12, 0.4, &settings(), seed).expect("every final map should be valid");
            fallbacks += u32::from(selected.used_fallback);
        }
        assert!(fallbacks < 100, "fallbacks: {fallbacks}/10000");
    }

    #[test]
    #[ignore = "manual release/debug V3 Waterfall full-build benchmark"]
    fn waterfall_full_build_benchmark_tracks_median_and_p95() {
        let budget = if cfg!(debug_assertions) {
            std::time::Duration::from_millis(250)
        } else {
            std::time::Duration::from_millis(50)
        };
        let palette = palette();
        for radius in [12, 20, 40] {
            let warmup =
                super::super::build(radius, 0.4, &settings(), u64::MAX, &palette, &is_solid)
                    .expect("warm-up Waterfall should build");
            std::hint::black_box(warmup);

            let mut samples = Vec::new();
            for seed in 0..12 {
                let started = std::time::Instant::now();
                let build =
                    super::super::build(radius, 0.4, &settings(), seed, &palette, &is_solid)
                        .expect("benchmark Waterfall should build");
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
            eprintln!("V3 Waterfall full build radius {radius}: median={median:?} p95={p95:?}");
            assert!(
                median < budget && p95 < budget,
                "radius {radius} median={median:?} p95={p95:?}, budget={budget:?}"
            );
        }
    }
}
