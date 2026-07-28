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
const WATER_END_MARGIN: usize = 2;
const BYPASS_HIGH_X: i32 = -5;
const BYPASS_LOW_X: i32 = 3;
const RELIEF_SAMPLE_DIVISOR: u64 = 5;
const PARTY_START: &str = "party_start";
const HOSTILE_START: &str = "hostile_start";

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
    pub(crate) raised_terrain: u32,
}

#[derive(Debug)]
struct WaterfallRecipe {
    level_height: f32,
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
    validate_footprint_capacity(grid_radius, settings)?;
    run_recipe(
        &WaterfallRecipe {
            level_height,
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
        let layout = resolve_layout(context.grid_radius, settings).map_err(|error| {
            CandidateAttemptError::Fatal(V3GenerationError::RecipeContract(error.to_string()))
        })?;
        let stream = SeedStreams::new(context.seed, context.candidate, PatchId(0).0)
            .stage("waterfall.relief");
        construct_plan(layout, Some(stream), self.level_height)
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
        let layout = resolve_layout(context.grid_radius, settings)
            .map_err(|error| V3GenerationError::RecipeContract(error.to_string()))?;
        construct_plan(layout, None, self.level_height).map_err(|issues| {
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

fn validate_footprint_capacity(
    grid_radius: u32,
    settings: &ProceduralV3Settings,
) -> Result<(), V3GenerationError> {
    let layout = resolve_layout(grid_radius, settings)
        .map_err(|error| V3GenerationError::RecipeContract(error.to_string()))?;
    watercourse(&layout.footprint).map_err(recipe_issues_to_error)?;
    bypass_tiles(layout.grid_radius, &layout.footprint).map_err(recipe_issues_to_error)?;
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

    let mut volume = VolumePlan::new(mask.clone());
    let mut water_nodes = BTreeMap::new();
    let mut water_by_coord = BTreeMap::new();
    for lane in &watercourse {
        for (index, coord) in lane.iter().copied().enumerate() {
            let next = lane.get(index.saturating_add(1)).copied();
            let cell = water_cell(coord, next);
            water_by_coord.insert(coord, cell);
        }
    }

    for coord in &mask {
        if let Some(water) = water_by_coord.get(coord).copied() {
            let (column, surface) = water_column(water);
            volume.columns.insert(*coord, column);
            volume.surfaces.insert(
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
            let surface_level = land_surface_level(*coord, &bypass, relief);
            let surface = TilePos::new(*coord, surface_level);
            volume.columns.insert(*coord, land_column(surface_level));
            volume.surfaces.insert(
                surface,
                SurfaceMetadata {
                    access: SurfaceAccess::Ordinary,
                    interior: None,
                },
            );
        }
    }

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
    let anchors = BTreeMap::from([
        (PARTY_START.to_owned(), party_start),
        (HOSTILE_START.to_owned(), hostile_start),
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
        (0.0, focus_height + frame, frame),
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

fn watercourse(mask: &BTreeSet<HexCoord>) -> Result<Vec<Vec<HexCoord>>, Vec<WorldValidationIssue>> {
    let mut lanes = Vec::new();
    for y in -WATER_HALF_WIDTH..=WATER_HALF_WIDTH {
        let row: Vec<_> = mask
            .iter()
            .copied()
            .filter(|coord| coord.y() == y)
            .collect();
        if row.len() <= WATER_END_MARGIN.saturating_mul(2).saturating_add(2) {
            return Err(vec![recipe_issue(format!(
                "Waterfall mask has insufficient width on water lane y={y}"
            ))]);
        }
        let lane = row
            .get(WATER_END_MARGIN..row.len() - WATER_END_MARGIN)
            .ok_or_else(|| vec![recipe_issue("Waterfall lane trimming exceeded its row")])?
            .to_vec();
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
        lanes.push(lane);
    }
    Ok(lanes)
}

fn bypass_tiles(
    grid_radius: u32,
    mask: &BTreeSet<HexCoord>,
) -> Result<[Vec<TilePos>; 2], Vec<WorldValidationIssue>> {
    let y = -(i32::try_from(grid_radius).unwrap_or(i32::MAX) / 2);
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
            "Waterfall mask cannot fit the required two-wide dry bypass",
        )]);
    }
    Ok(lanes)
}

fn land_surface_level(
    coord: HexCoord,
    bypass: &[Vec<TilePos>; 2],
    relief: Option<SeedStream<'_>>,
) -> i32 {
    if let Some(position) = bypass
        .iter()
        .flatten()
        .find(|position| position.coord == coord)
    {
        return position.level;
    }

    let base = if coord.x() < FALL_TARGET_X {
        HIGH_LAND_LEVEL
    } else {
        LOW_LAND_LEVEL
    };
    let raised =
        relief.is_some_and(|stream| stream.sample_coord(coord, 0) % RELIEF_SAMPLE_DIVISOR == 0);
    base + i32::from(raised)
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
    let mut targets = BTreeSet::new();
    let mut terminals = Vec::new();
    for (position, node) in &body.nodes {
        match node.state {
            LiquidFlowState::Still => calm_nodes = calm_nodes.saturating_add(1),
            LiquidFlowState::Current => current_nodes = current_nodes.saturating_add(1),
            LiquidFlowState::Rapid => rapid_nodes = rapid_nodes.saturating_add(1),
            LiquidFlowState::Fall => fall_nodes.push((*position, node.downstream)),
        }
        if let Some(downstream) = node.downstream {
            targets.insert(downstream);
        } else {
            terminals.push(*position);
        }
    }
    let sources: Vec<_> = body
        .nodes
        .keys()
        .copied()
        .filter(|position| !targets.contains(position))
        .collect();

    if sources.len() != 3 || terminals.len() != 3 {
        issues.push(recipe_issue(
            "Waterfall must have exactly three inlet lanes and three outlet terminals",
        ));
    }
    validate_flow_stages(body, &sources, &terminals, &mut issues);
    if calm_nodes < 9 || current_nodes < 3 || rapid_nodes < 3 {
        issues.push(recipe_issue(
            "Waterfall must realize calm inlet/basin, rapid, and current stages",
        ));
    }
    let fall_height = validate_fall(&fall_nodes, &mut issues);
    validate_liquid_beds(plan, body, &mut issues);

    let bypass = match bypass_tiles(plan.layout.grid_radius, &plan.layout.footprint) {
        Ok(bypass) => bypass,
        Err(mut bypass_issues) => {
            issues.append(&mut bypass_issues);
            [Vec::new(), Vec::new()]
        }
    };
    validate_bypass(plan, &bypass, &mut issues);

    let party = plan.anchors.get(PARTY_START).copied();
    let conflict = plan.anchors.get(HOSTILE_START).copied();
    let ordinary_by_coord = ordinary_surfaces_by_coord(plan);
    let route_length = match (party, conflict) {
        (Some(start), Some(goal)) => shortest_ordinary_route(plan, &ordinary_by_coord, start, goal),
        _ => None,
    };
    if route_length.is_none() {
        issues.push(recipe_issue(
            "Waterfall critical anchors are not joined by ordinary traversal",
        ));
    }
    validate_ordinary_connectivity(plan, &ordinary_by_coord, party, &mut issues);

    let ordinary_surfaces = plan
        .volume
        .surfaces
        .values()
        .filter(|metadata| metadata.access == SurfaceAccess::Ordinary)
        .count();
    let raised_terrain = plan
        .volume
        .surfaces
        .iter()
        .filter(|(position, metadata)| {
            if metadata.access != SurfaceAccess::Ordinary
                || bypass
                    .iter()
                    .flatten()
                    .any(|bypass| bypass.coord == position.coord)
            {
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

    let metrics = WaterfallMetrics {
        water_nodes: count_u32(body.nodes.len()),
        calm_nodes,
        current_nodes,
        rapid_nodes,
        fall_nodes: count_u32(fall_nodes.len()),
        fall_height,
        ordinary_surfaces: count_u32(ordinary_surfaces),
        reachable_elevation_levels: count_u32(
            ordinary_by_coord
                .values()
                .map(|position| position.level)
                .collect::<BTreeSet<_>>()
                .len(),
        ),
        bypass_steps: route_length.unwrap_or_default(),
        raised_terrain: count_u32(raised_terrain),
    };
    if issues.is_empty() {
        WorldValidation::Valid(metrics)
    } else {
        WorldValidation::Invalid(issues)
    }
}

fn validate_flow_stages(
    body: &LiquidBodyPlan,
    sources: &[TilePos],
    terminals: &[TilePos],
    issues: &mut Vec<WorldValidationIssue>,
) {
    let expected_lanes: BTreeSet<_> = (-WATER_HALF_WIDTH..=WATER_HALF_WIDTH).collect();
    let source_lanes: BTreeSet<_> = sources.iter().map(|position| position.coord.y()).collect();
    let terminal_lanes: BTreeSet<_> = terminals
        .iter()
        .map(|position| position.coord.y())
        .collect();
    if source_lanes != expected_lanes || terminal_lanes != expected_lanes {
        issues.push(recipe_issue(
            "Waterfall inlet and outlet do not preserve all three authored lanes",
        ));
    }

    for (position, node) in &body.nodes {
        let expected_state = if node.downstream.is_none() {
            LiquidFlowState::Still
        } else if position.coord.x() < FALL_SOURCE_X {
            if position.coord.x() <= BYPASS_HIGH_X {
                LiquidFlowState::Still
            } else {
                LiquidFlowState::Rapid
            }
        } else if position.coord.x() == FALL_SOURCE_X {
            LiquidFlowState::Fall
        } else if position.coord.x() <= BYPASS_LOW_X {
            LiquidFlowState::Still
        } else {
            LiquidFlowState::Current
        };
        if node.state != expected_state {
            issues.push(recipe_issue(format!(
                "Waterfall flow stage at {position:?} is {:?}, expected {expected_state:?}",
                node.state
            )));
        }
        if let Some(downstream) = node.downstream {
            let expected_coord =
                HexCoord::from_axial(position.coord.x().saturating_add(1), position.coord.y());
            if downstream.coord != expected_coord {
                issues.push(recipe_issue(format!(
                    "Waterfall lane at {position:?} does not drain to its ordered next section"
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
    issues: &mut Vec<WorldValidationIssue>,
) {
    for position in body.nodes.keys() {
        let bed = plan
            .volume
            .surfaces
            .iter()
            .find(|(surface, _)| surface.coord == position.coord);
        if !matches!(
            bed,
            Some((
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
        }
    }
}

fn validate_bypass(
    plan: &GeneratedWorldPlan,
    bypass: &[Vec<TilePos>; 2],
    issues: &mut Vec<WorldValidationIssue>,
) {
    if bypass.iter().any(|lane| lane.len() != 9) {
        issues.push(recipe_issue(
            "Waterfall bypass must contain two complete nine-tile lanes",
        ));
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
                    "Waterfall bypass tile {expected:?} is not ordinary footing"
                )));
            }
        }
        for pair in lane.windows(2) {
            let [first, second] = pair else {
                continue;
            };
            if !admits_transition(plan, *first, *second) {
                issues.push(recipe_issue(format!(
                    "Waterfall bypass transition {:?} -> {:?} is not walker-admitted",
                    first, second
                )));
            }
        }
    }
    let [first_lane, second_lane] = bypass;
    for (index, (first, second)) in first_lane.iter().zip(second_lane).enumerate() {
        if !admits_transition(plan, *first, *second) {
            issues.push(recipe_issue(format!(
                "Waterfall bypass loses its second lane at step {index}"
            )));
        }
    }
}

fn validate_ordinary_connectivity(
    plan: &GeneratedWorldPlan,
    ordinary_by_coord: &BTreeMap<HexCoord, TilePos>,
    start: Option<TilePos>,
    issues: &mut Vec<WorldValidationIssue>,
) {
    let ordinary: BTreeSet<_> = ordinary_by_coord.values().copied().collect();
    let Some(start) = start.filter(|start| ordinary.contains(start)) else {
        issues.push(recipe_issue(
            "Waterfall party anchor is not ordinary footing",
        ));
        return;
    };
    let reachable = reachable_ordinary(plan, ordinary_by_coord, start);
    if reachable != ordinary {
        issues.push(recipe_issue(format!(
            "Waterfall ordinary network leaves {} surface(s) disconnected",
            ordinary.len().saturating_sub(reachable.len())
        )));
    }
}

fn shortest_ordinary_route(
    plan: &GeneratedWorldPlan,
    ordinary_by_coord: &BTreeMap<HexCoord, TilePos>,
    start: TilePos,
    goal: TilePos,
) -> Option<u32> {
    let mut distances = BTreeMap::from([(start, 0_u32)]);
    let mut frontier = VecDeque::from([start]);
    while let Some(position) = frontier.pop_front() {
        let Some(distance) = distances.get(&position).copied() else {
            continue;
        };
        if position == goal {
            return Some(distance);
        }
        for neighbor in ordinary_neighbors(plan, ordinary_by_coord, position) {
            if distances.contains_key(&neighbor) {
                continue;
            }
            distances.insert(neighbor, distance.saturating_add(1));
            frontier.push_back(neighbor);
        }
    }
    None
}

fn reachable_ordinary(
    plan: &GeneratedWorldPlan,
    ordinary_by_coord: &BTreeMap<HexCoord, TilePos>,
    start: TilePos,
) -> BTreeSet<TilePos> {
    let mut reachable = BTreeSet::from([start]);
    let mut frontier = VecDeque::from([start]);
    while let Some(position) = frontier.pop_front() {
        for neighbor in ordinary_neighbors(plan, ordinary_by_coord, position) {
            if reachable.insert(neighbor) {
                frontier.push_back(neighbor);
            }
        }
    }
    reachable
}

fn ordinary_surfaces_by_coord(plan: &GeneratedWorldPlan) -> BTreeMap<HexCoord, TilePos> {
    plan.volume
        .surfaces
        .iter()
        .filter_map(|(position, metadata)| {
            (metadata.access == SurfaceAccess::Ordinary).then_some((position.coord, *position))
        })
        .collect()
}

fn ordinary_neighbors(
    plan: &GeneratedWorldPlan,
    ordinary_by_coord: &BTreeMap<HexCoord, TilePos>,
    from: TilePos,
) -> Vec<TilePos> {
    from.coord
        .neighbors()
        .into_iter()
        .filter_map(|coord| ordinary_by_coord.get(&coord).copied())
        .filter(|to| admits_transition(plan, from, *to))
        .collect()
}

fn admits_transition(plan: &GeneratedWorldPlan, from: TilePos, to: TilePos) -> bool {
    let endpoint = |position| {
        let metadata = plan.volume.surfaces.get(&position)?;
        (metadata.access == SurfaceAccess::Ordinary).then(|| {
            plan.volume
                .surface_headroom(position)
                .map(|headroom| TraversalEndpoint::new(position, true, headroom))
        })?
    };
    let Some(from_endpoint) = endpoint(from) else {
        return false;
    };
    let Some(to_endpoint) = endpoint(to) else {
        return false;
    };
    TraversalProfile::WALKER.admits_transition(from_endpoint, to_endpoint)
        && TraversalProfile::WALKER.admits_transition(to_endpoint, from_endpoint)
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
    fn bypass_is_two_wide_climbable_and_connects_every_ordinary_surface() {
        let selected = generate(12, 0.4, &settings(), 91).expect("Waterfall should generate");
        let plan = &selected.validated.plan;
        let bypass = bypass_tiles(12, &plan.layout.footprint).expect("fixed bypass");

        for lane in &bypass {
            assert_eq!(lane.len(), 9);
            assert!(lane.windows(2).all(|pair| {
                matches!(
                    pair,
                    [first, second] if first.level.saturating_sub(second.level) == 1
                )
            }));
            assert!(lane.windows(2).all(|pair| {
                matches!(
                    pair,
                    [first, second] if admits_transition(plan, *first, *second)
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
        let [first_lane, second_lane] = &bypass;
        assert!(first_lane
            .iter()
            .zip(second_lane)
            .all(|(first, second)| admits_transition(plan, *first, *second)));
        let party = plan
            .anchors
            .get(PARTY_START)
            .copied()
            .expect("Waterfall should publish party_start");
        let ordinary_by_coord = ordinary_surfaces_by_coord(plan);
        let ordinary: BTreeSet<_> = ordinary_by_coord.values().copied().collect();
        assert_eq!(
            reachable_ordinary(plan, &ordinary_by_coord, party),
            ordinary
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
    fn generated_view_uses_world_space_level_height_and_rejects_invalid_scale() {
        let compact = generate(12, 0.4, &settings(), 12).expect("Waterfall should generate");
        let tall = generate(12, 0.8, &settings(), 12).expect("Waterfall should generate");
        assert_ne!(
            compact.validated.plan.view_hint,
            tall.validated.plan.view_hint
        );
        assert_eq!(
            compact.validated.plan.view_hint.focus.1 * 2.0,
            tall.validated.plan.view_hint.focus.1
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
}
