//! Pure semantic Waterfall recipe for procedural generator V3.
//!
//! Water topology is authored before terrain. The resulting solid volume is fitted
//! around one three-wide watercourse and a separate two-wide ordinary-walker bypass.
//! Rendering and ECS publication remain downstream of this module.

use std::collections::{BTreeMap, BTreeSet};

use hex_assets::RuntimeArtCatalog;
use hex_core::{HexCoord, Level, MapViewHint, SpecialMovementRegion, TilePos};

use super::composition::{compose_single_patch, GeneratedPatchPlan};
use super::layout::{
    resolve_layout, HexSide, LayoutKind, PatchId, ResolvedLayoutPlan, ResolvedLiquidElevation,
};
use super::liquid::{LiquidBodyId, LiquidBodyPlan, LiquidFlowState, LiquidNode, LiquidPlan};
use super::local_frame::LocalPatchFrame;
use super::patch::{PatchBuildMode, PatchRecipeContext};
use super::routing::vertex_disjoint_paths;
use super::seam::{is_seam_closure_access, shape_walker_seams, validate_patch_walker_seams};
use super::seed::SeedStream;
#[cfg(test)]
use super::seed::SeedStreams;
use super::selection::{
    run_recipe, CandidateAttemptError, CandidateContext, FallbackContext, RepairOutcome, V3Recipe,
    ValidatedWorldSelection, WorldValidation,
};
use super::traversal::OrdinaryGraph;
use super::vegetation::{
    append_landform_vegetation, validate_landform_vegetation, LandformVegetationDomain,
    LandformVegetationMetrics, LandformVegetationSet,
};
use super::volume::{
    FillMaterialRole, LevelInterval, NonSolidFill, SolidMass, SolidMaterialRole, SurfaceAccess,
    SurfaceMetadata, VolumeColumn, VolumeElement, VolumePlan,
};
use super::world::{
    FeaturePlan, GeneratedWorldPlan, InteriorPlan, PlannedStructure, StructureId, StructureKind,
    StructurePlan, WorldIssueCode, WorldValidationIssue,
};
use super::V3GenerationError;
use crate::settings::{
    ordered_simple_seam_lanes, ProceduralV3Settings, V3EnvironmentSettings, V3LayoutSettings,
    V3RecipeSettings, V3WaterfallSettings, MAX_V3_LEVEL,
};

const HIGH_LAND_LEVEL: i32 = 27;
const LOW_LAND_LEVEL: i32 = 16;
const HIGH_WATER_LEVEL: i32 = HIGH_LAND_LEVEL - 1;
const LOW_WATER_LEVEL: i32 = LOW_LAND_LEVEL - 3;
const COMPOSITE_LOW_WATER_LEVEL: i32 = LOW_LAND_LEVEL - 1;
const FALL_SOURCE_X: i32 = -1;
const FALL_TARGET_X: i32 = 0;
const WATER_HALF_WIDTH: i32 = 1;
const BASIN_WIDE_END_X: i32 = 4;
const BASIN_END_X: i32 = 6;
const BASIN_MAX_HALF_WIDTH: i32 = 3;
const BYPASS_HIGH_X: i32 = -6;
const BYPASS_LOW_X: i32 = 5;
const BENT_BYPASS_HIGH_X: i32 = BYPASS_HIGH_X + 4;
const BENT_BYPASS_LOW_X: i32 = BYPASS_LOW_X + 4;
const SECONDARY_HIGH_X: i32 = -7;
const SECONDARY_LOW_X: i32 = 6;
const BRIDGE_FIRST_X: i32 = -7;
const BRIDGE_LAST_X: i32 = -6;
const BENT_BRIDGE_FIRST_X: i32 = -3;
const BENT_BRIDGE_LAST_X: i32 = -2;
const BENT_FEEDER_TARGET_X: [i32; 3] = [-6, -7, -8];
const BRIDGE_BANK_Y: i32 = 3;
const BRIDGE_DECK_LEVEL: i32 = HIGH_LAND_LEVEL + 1;
const RING_BRIDGE_FLANK: TilePos = TilePos::new(HexCoord::from_axial(-8, 2), HIGH_LAND_LEVEL);
const CLIFF_MID_LEVEL: i32 = LOW_LAND_LEVEL + (HIGH_LAND_LEVEL - LOW_LAND_LEVEL) / 2;
const CLIFF_MAX_OFFSET: i32 = 2;
const CLIFF_PATTERN: [i32; 12] = [-2, -2, -1, 0, 1, 2, 2, 1, 0, -1, -2, -2];
const CLIFF_SHELF_REGION: SpecialMovementRegion = SpecialMovementRegion(0);
const RING_ISOLATED_TERRAIN_REGION: SpecialMovementRegion = SpecialMovementRegion(1);
const MAX_RING_CLOSED_POCKET_CELLS: usize = 3;
const MAX_RING19_CLOSED_POCKET_CELLS: usize = 12;
const RELIEF_CENTERS_PER_BANK: u64 = 3;
const WATERFALL_TREE_TARGET: usize = 3;
const WATERFALL_GRASS_PERCENT: usize = 20;
const PARTY_START: &str = "party_start";
const HOSTILE_START: &str = "hostile_start";
const FALL_OVERLOOK: &str = "fall_overlook";
const BASIN_OVERLOOK: &str = "basin_overlook";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WaterfallElevationProfile {
    high_land: Level,
    low_land: Level,
    high_water: Level,
    low_water: Level,
    bridge_deck: Level,
    cliff_mid: Level,
}

impl WaterfallElevationProfile {
    const SINGLE: Self = Self {
        high_land: HIGH_LAND_LEVEL,
        low_land: LOW_LAND_LEVEL,
        high_water: HIGH_WATER_LEVEL,
        low_water: LOW_WATER_LEVEL,
        bridge_deck: BRIDGE_DECK_LEVEL,
        cliff_mid: CLIFF_MID_LEVEL,
    };

    const RING7: Self = Self {
        low_water: COMPOSITE_LOW_WATER_LEVEL,
        ..Self::SINGLE
    };

    fn translated(delta: Level) -> Self {
        Self {
            high_land: HIGH_LAND_LEVEL.saturating_add(delta),
            low_land: LOW_LAND_LEVEL.saturating_add(delta),
            high_water: HIGH_WATER_LEVEL.saturating_add(delta),
            low_water: LOW_WATER_LEVEL.saturating_add(delta),
            bridge_deck: BRIDGE_DECK_LEVEL.saturating_add(delta),
            cliff_mid: CLIFF_MID_LEVEL.saturating_add(delta),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WaterfallPortElevation {
    Band { minimum: Level, maximum: Level },
    Exact(Level),
}

impl WaterfallPortElevation {
    const fn admits(self, level: Level) -> bool {
        match self {
            Self::Band { minimum, maximum } => minimum <= level && level <= maximum,
            Self::Exact(expected) => level == expected,
        }
    }

    const fn exact(self) -> Option<Level> {
        match self {
            Self::Exact(level) => Some(level),
            Self::Band { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WaterfallPort {
    side: HexSide,
    boundary: BTreeSet<HexCoord>,
    ordered_boundary: Vec<HexCoord>,
    inward_approach: BTreeSet<HexCoord>,
    approach_depth: u32,
    elevation: WaterfallPortElevation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WaterfallFlowShape {
    Straight,
    BentNorthWest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WaterfallHydrology {
    kind: LayoutKind,
    profile: WaterfallElevationProfile,
    inlet: Option<WaterfallPort>,
    outlet: Option<WaterfallPort>,
}

impl WaterfallHydrology {
    fn resolve(patch: &PatchRecipeContext<'_>) -> Result<Self, Vec<WorldValidationIssue>> {
        if patch.layout().kind == LayoutKind::Single {
            return Ok(Self {
                kind: LayoutKind::Single,
                profile: WaterfallElevationProfile::SINGLE,
                inlet: None,
                outlet: None,
            });
        }

        let mut incoming = Vec::new();
        let mut outgoing = Vec::new();
        for edge in patch.shared_edges() {
            let Some(liquid) = edge.liquid_port() else {
                continue;
            };
            let Some(ordered_lanes) = ordered_simple_seam_lanes(&liquid.port.lanes) else {
                return Err(vec![recipe_issue(
                    "Composite Waterfall liquid seam lanes are not one simple ordered aperture",
                )]);
            };
            let elevation = match liquid.elevation {
                ResolvedLiquidElevation::EdgeBand => WaterfallPortElevation::Band {
                    minimum: edge.contract.elevation.min,
                    maximum: edge.contract.elevation.max,
                },
                ResolvedLiquidElevation::Exact(level) => WaterfallPortElevation::Exact(level),
            };
            let port = WaterfallPort {
                side: edge.side,
                boundary: liquid
                    .port
                    .lanes
                    .iter()
                    .map(|(inside, _)| *inside)
                    .collect(),
                ordered_boundary: ordered_lanes
                    .into_iter()
                    .map(|(inside, _)| inside)
                    .collect(),
                inward_approach: liquid.port.first_approach,
                approach_depth: edge.contract.approach_depth,
                elevation,
            };
            if liquid.is_source {
                outgoing.push(port);
            } else {
                incoming.push(port);
            }
        }
        for outlet in patch.boundary_liquid_outlets() {
            let Some(ordered_lanes) = ordered_simple_seam_lanes(&outlet.lanes) else {
                return Err(vec![recipe_issue(
                    "Composite Waterfall boundary outlet lanes are not one simple ordered aperture",
                )]);
            };
            outgoing.push(WaterfallPort {
                side: outlet.side,
                boundary: outlet.lanes.iter().map(|(inside, _)| *inside).collect(),
                ordered_boundary: ordered_lanes
                    .into_iter()
                    .map(|(inside, _)| inside)
                    .collect(),
                inward_approach: outlet.inward_approach.clone(),
                approach_depth: outlet.approach_depth,
                elevation: WaterfallPortElevation::Exact(outlet.level),
            });
        }

        let [outlet] = outgoing.as_slice() else {
            return Err(vec![recipe_issue(format!(
                "Composite Waterfall has {} liquid outlets; expected one",
                outgoing.len()
            ))]);
        };
        if outlet.boundary.len() != 3 {
            return Err(vec![recipe_issue(
                "Composite Waterfall outlet must contain exactly three lanes",
            )]);
        }
        match patch.layout().kind {
            LayoutKind::Single => unreachable!("handled above"),
            LayoutKind::Ring7 => {
                if incoming.len() > 1 {
                    return Err(vec![recipe_issue(format!(
                        "Ring7 Waterfall has {} liquid inlets; expected at most one",
                        incoming.len()
                    ))]);
                }
                if !outlet
                    .elevation
                    .admits(WaterfallElevationProfile::RING7.low_water)
                    || incoming.first().is_some_and(|inlet| {
                        !inlet
                            .elevation
                            .admits(WaterfallElevationProfile::RING7.high_water)
                    })
                {
                    return Err(vec![recipe_issue(
                        "Ring7 Waterfall liquid elevations do not admit its legacy 26-to-15 profile",
                    )]);
                }
                Ok(Self {
                    kind: LayoutKind::Ring7,
                    profile: WaterfallElevationProfile::RING7,
                    inlet: incoming.pop(),
                    outlet: Some(outlet.clone()),
                })
            }
            LayoutKind::Ring19 => {
                let [inlet] = incoming.as_slice() else {
                    return Err(vec![recipe_issue(format!(
                        "Ring19 Waterfall has {} liquid inlets; expected one",
                        incoming.len()
                    ))]);
                };
                if inlet.boundary.len() != 3 {
                    return Err(vec![recipe_issue(
                        "Ring19 Waterfall inlet must contain exactly three lanes",
                    )]);
                }
                let Some(inlet_level) = inlet.elevation.exact() else {
                    return Err(vec![recipe_issue(
                        "Ring19 Waterfall inlet requires exact elevation authority",
                    )]);
                };
                let Some(outlet_level) = outlet.elevation.exact() else {
                    return Err(vec![recipe_issue(
                        "Ring19 Waterfall outlet requires exact elevation authority",
                    )]);
                };
                let inlet_delta = inlet_level.saturating_sub(HIGH_WATER_LEVEL);
                let outlet_delta = outlet_level.saturating_sub(LOW_WATER_LEVEL);
                if inlet_delta != outlet_delta {
                    return Err(vec![recipe_issue(format!(
                        "Ring19 Waterfall inlet/outlet levels {inlet_level}->{outlet_level} are not one complete translation of the canonical 26-to-13 profile"
                    ))]);
                }
                let profile = WaterfallElevationProfile::translated(inlet_delta);
                if profile.low_water < 3 || profile.low_land <= profile.low_water {
                    return Err(vec![recipe_issue(
                        "Ring19 Waterfall translated profile leaves insufficient lowland support",
                    )]);
                }
                if profile.bridge_deck > MAX_V3_LEVEL {
                    return Err(vec![recipe_issue(format!(
                        "Ring19 Waterfall translated bridge deck exceeds level {MAX_V3_LEVEL}"
                    ))]);
                }
                let resolved = Self {
                    kind: LayoutKind::Ring19,
                    profile,
                    inlet: Some(inlet.clone()),
                    outlet: Some(outlet.clone()),
                };
                if patch.rotation_turns() != resolved.rotation() {
                    return Err(vec![recipe_issue(format!(
                        "Ring19 Waterfall rotation_turns {} disagrees with its resolved outlet orientation {}",
                        patch.rotation_turns(),
                        resolved.rotation()
                    ))]);
                }
                Ok(resolved)
            }
            LayoutKind::Macro => Err(vec![recipe_issue(
                "Macro Waterfall hydrology is resolved by the authored Macro runner",
            )]),
            LayoutKind::Schematic => Err(vec![recipe_issue(
                "Schematic hydrology is resolved by the global schematic compiler",
            )]),
        }
    }

    const fn rotation(&self) -> u8 {
        let Some(outlet) = &self.outlet else {
            return 0;
        };
        match outlet.side {
            HexSide::East => 0,
            HexSide::NorthEast => 1,
            HexSide::NorthWest => 2,
            HexSide::West => 3,
            HexSide::SouthWest => 4,
            HexSide::SouthEast => 5,
        }
    }

    fn flow_shape(&self) -> Result<WaterfallFlowShape, Vec<WorldValidationIssue>> {
        if self.kind == LayoutKind::Single {
            return Ok(WaterfallFlowShape::Straight);
        }
        if self
            .outlet
            .as_ref()
            .is_none_or(|outlet| outlet.side != HexSide::East)
        {
            return Err(vec![recipe_issue(
                "Composite Waterfall outlet does not normalize to the authored east side",
            )]);
        }
        match self.inlet.as_ref().map(|inlet| inlet.side) {
            None | Some(HexSide::West) => Ok(WaterfallFlowShape::Straight),
            Some(HexSide::NorthWest) if self.kind == LayoutKind::Ring19 => {
                Ok(WaterfallFlowShape::BentNorthWest)
            }
            Some(side) => Err(vec![recipe_issue(format!(
                "Composite Waterfall inlet normalizes to unsupported side {side:?}"
            ))]),
        }
    }

    fn to_local(&self, frame: LocalPatchFrame) -> Result<Self, Vec<WorldValidationIssue>> {
        let inverse_rotation = (6_u8.saturating_sub(self.rotation())) % 6;
        let convert = |port: &WaterfallPort| {
            let boundary = port
                .boundary
                .iter()
                .map(|coord| frame.to_local(*coord))
                .collect::<Result<BTreeSet<_>, _>>()
                .map_err(|error| {
                    vec![recipe_issue(format!(
                        "Waterfall liquid port conversion failed: {error}"
                    ))]
                })?;
            let ordered_boundary = port
                .ordered_boundary
                .iter()
                .map(|coord| frame.to_local(*coord))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| {
                    vec![recipe_issue(format!(
                        "Waterfall ordered liquid port conversion failed: {error}"
                    ))]
                })?;
            let inward_approach = port
                .inward_approach
                .iter()
                .map(|coord| frame.to_local(*coord))
                .collect::<Result<BTreeSet<_>, _>>()
                .map_err(|error| {
                    vec![recipe_issue(format!(
                        "Waterfall liquid approach conversion failed: {error}"
                    ))]
                })?;
            Ok::<WaterfallPort, Vec<WorldValidationIssue>>(WaterfallPort {
                side: rotate_side(port.side, inverse_rotation),
                boundary,
                ordered_boundary,
                inward_approach,
                approach_depth: port.approach_depth,
                elevation: port.elevation,
            })
        };
        Ok(Self {
            kind: self.kind,
            profile: self.profile,
            inlet: self.inlet.as_ref().map(convert).transpose()?,
            outlet: self.outlet.as_ref().map(convert).transpose()?,
        })
    }
}

const fn rotate_side(side: HexSide, turns: u8) -> HexSide {
    let index = match side {
        HexSide::East => 0_u8,
        HexSide::NorthEast => 1,
        HexSide::NorthWest => 2,
        HexSide::West => 3,
        HexSide::SouthWest => 4,
        HexSide::SouthEast => 5,
    };
    match index.saturating_add(turns) % 6 {
        0 => HexSide::East,
        1 => HexSide::NorthEast,
        2 => HexSide::NorthWest,
        3 => HexSide::West,
        4 => HexSide::SouthWest,
        _ => HexSide::SouthEast,
    }
}

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
    pub(crate) tree_roots: u32,
    pub(crate) grass_roots: u32,
    pub(crate) grass_surface_percent: u32,
}

#[derive(Debug)]
struct WaterfallRecipe {
    level_height: f32,
    layout: ResolvedLayoutPlan,
    vegetation: LandformVegetationSet,
    #[cfg(test)]
    reject_candidates: bool,
}

#[derive(Debug, Clone, Copy)]
struct WaterfallStreams<'a> {
    relief: SeedStream<'a>,
    cliff: SeedStream<'a>,
    trees: SeedStream<'a>,
    grass: SeedStream<'a>,
}

/// Runs the common eight-candidate V3 selector for one Waterfall world.
pub(crate) fn generate_with_catalog(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV3Settings,
    seed: u64,
    catalog: &RuntimeArtCatalog,
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
    let vegetation = LandformVegetationSet::resolve(
        catalog,
        V3EnvironmentSettings::TemperateGrassland,
        "Waterfall",
    )
    .map_err(V3GenerationError::RecipeContract)?;
    run_recipe(
        &WaterfallRecipe {
            level_height,
            layout,
            vegetation,
            #[cfg(test)]
            reject_candidates: false,
        },
        settings,
        grid_radius,
        seed,
    )
}

#[cfg(test)]
fn generate(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV3Settings,
    seed: u64,
) -> Result<ValidatedWorldSelection<WaterfallMetrics>, V3GenerationError> {
    generate_with_catalog(
        grid_radius,
        level_height,
        settings,
        seed,
        super::vegetation::tests::runtime_art_catalog(),
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
        let patch = PatchRecipeContext::resolve(&self.layout, PatchId(0))
            .map_err(CandidateAttemptError::Fatal)?;
        let fragment = construct_patch_with_objects(
            patch,
            &V3WaterfallSettings,
            V3EnvironmentSettings::TemperateGrassland,
            self.level_height,
            PatchBuildMode::Candidate {
                world_seed: context.seed,
                candidate: context.candidate,
            },
            &self.vegetation,
        )
        .map_err(CandidateAttemptError::Rejected)?;
        compose_single_patch(self.layout.clone(), fragment).map_err(|error| {
            CandidateAttemptError::Rejected(vec![recipe_issue(format!("{error:?}"))])
        })
    }

    fn validate(
        &self,
        _settings: &Self::Settings,
        plan: &GeneratedWorldPlan,
    ) -> WorldValidation<Self::Metrics> {
        validate_waterfall(plan, &self.vegetation)
    }

    fn repair(
        &self,
        _context: CandidateContext,
        _settings: &Self::Settings,
        _plan: &mut GeneratedWorldPlan,
        _round: u8,
        _issues: &[WorldValidationIssue],
    ) -> Result<RepairOutcome, CandidateAttemptError> {
        // Waterfall repair is intentionally staged out. Candidates are admitted only
        // when construction already satisfies the complete recipe contract; an
        // invalid candidate is rejected and the separately validated fallback
        // remains the bounded recovery path.
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
        let patch = PatchRecipeContext::resolve(&self.layout, PatchId(0))?;
        let fragment = construct_patch_with_objects(
            patch,
            &V3WaterfallSettings,
            V3EnvironmentSettings::TemperateGrassland,
            self.level_height,
            PatchBuildMode::CanonicalFallback,
            &self.vegetation,
        )
        .map_err(recipe_issues_to_error)?;
        compose_single_patch(self.layout.clone(), fragment)
            .map_err(|error| V3GenerationError::RecipeContract(format!("{error:?}")))
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
    bridge_tiles(&layout.footprint, WaterfallElevationProfile::SINGLE)
        .map_err(recipe_issues_to_error)?;
    bypass_tiles(
        layout.grid_radius,
        &layout.footprint,
        WaterfallElevationProfile::SINGLE,
    )
    .map_err(recipe_issues_to_error)?;
    secondary_bypass_tiles(
        layout.grid_radius,
        &layout.footprint,
        WaterfallElevationProfile::SINGLE,
    )
    .map_err(recipe_issues_to_error)?;
    secondary_slope_apron(
        layout.grid_radius,
        &layout.footprint,
        WaterfallElevationProfile::SINGLE,
    )
    .map_err(recipe_issues_to_error)?;
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

pub(crate) fn construct_patch_with_catalog(
    patch: PatchRecipeContext<'_>,
    settings: &V3WaterfallSettings,
    environment: V3EnvironmentSettings,
    level_height: f32,
    mode: PatchBuildMode,
    catalog: &RuntimeArtCatalog,
) -> Result<GeneratedPatchPlan, Vec<WorldValidationIssue>> {
    let vegetation = LandformVegetationSet::resolve(catalog, environment, "Waterfall")
        .map_err(|error| vec![recipe_issue(error)])?;
    construct_patch_with_objects(
        patch,
        settings,
        environment,
        level_height,
        mode,
        &vegetation,
    )
}

fn construct_patch_with_objects(
    patch: PatchRecipeContext<'_>,
    _settings: &V3WaterfallSettings,
    environment: V3EnvironmentSettings,
    level_height: f32,
    mode: PatchBuildMode,
    vegetation: &LandformVegetationSet,
) -> Result<GeneratedPatchPlan, Vec<WorldValidationIssue>> {
    if environment != V3EnvironmentSettings::TemperateGrassland {
        return Err(vec![recipe_issue(
            "Waterfall requires the TemperateGrassland environment",
        )]);
    }
    let hydrology = WaterfallHydrology::resolve(&patch)?;
    let rotation = hydrology.rotation();
    let frame = patch
        .local_frame_with_rotation(rotation)
        .map_err(|error| vec![recipe_issue(error.to_string())])?;
    let mask = frame
        .local_mask(patch.mask())
        .map_err(|error| vec![recipe_issue(error)])?;
    let local_hydrology = hydrology.to_local(frame)?;
    let profile = local_hydrology.profile;
    let flow_shape = local_hydrology.flow_shape()?;
    let patch_radius = frame.scale();
    let biome_region = patch.biome_region();
    let streams = mode.seed_streams(&patch).map(|streams| WaterfallStreams {
        relief: streams.stage("waterfall.relief"),
        cliff: streams.stage("waterfall.cliff"),
        trees: streams.stage("waterfall.vegetation.trees"),
        grass: streams.stage("waterfall.vegetation.grass"),
    });
    let bypass = bypass_tiles_for_shape(patch_radius, &mask, profile, flow_shape)?;
    let secondary_bypass = secondary_bypass_tiles(patch_radius, &mask, profile)?;
    let secondary_apron = secondary_slope_apron(patch_radius, &mask, profile)?;
    let bridge = bridge_tiles_for_shape(&mask, profile, flow_shape)?;
    let bridge_abutment = bent_bridge_abutment(&mask, profile, flow_shape)?;
    let composite_layout = patch.layout().kind.is_composite();
    let ring7_layout = patch.layout().kind == LayoutKind::Ring7;
    let mut dry_reservations =
        waterfall_feeder_exclusions(patch_radius, &mask, ring7_layout, profile, flow_shape)?;
    extend_feeder_seam_exclusions(&patch, frame, &local_hydrology, &mut dry_reservations)?;
    let watercourse =
        watercourse_for_hydrology(&mask, &local_hydrology, flow_shape, &dry_reservations)?;
    let low_water_level = profile.low_water;
    validate_waterfall_liquid_ports(&local_hydrology, &watercourse)?;
    let ring_secondary_flank = if ring7_layout {
        ring_secondary_flank_apron(patch_radius, &mask, profile)?
    } else {
        Vec::new()
    };
    let bridge_by_coord: BTreeMap<_, _> = bridge
        .iter()
        .copied()
        .map(|position| (position.coord, position))
        .collect();
    let water_coords = watercourse.coordinates();
    let mut protected_by_coord: BTreeMap<_, _> = bypass
        .iter()
        .chain(&secondary_bypass)
        .flatten()
        .chain(&secondary_apron)
        .chain(&ring_secondary_flank)
        .chain(&bridge_abutment)
        .map(|position| (position.coord, position.level))
        .collect();
    protected_by_coord.extend(
        bridge
            .iter()
            .map(|position| (position.coord, profile.high_land)),
    );
    let mut restored_surface_levels: BTreeMap<_, _> = bypass
        .iter()
        .chain(&secondary_bypass)
        .flatten()
        .chain(&secondary_apron)
        .chain(&ring_secondary_flank)
        .chain(&bridge_abutment)
        .map(|position| (position.coord, position.level))
        .collect();
    restored_surface_levels.extend(
        bridge
            .iter()
            .map(|position| (position.coord, profile.high_land)),
    );
    for edge in patch.shared_edges() {
        let protected_approaches = if patch.layout().kind == LayoutKind::Ring19 {
            edge.walker_protected_approaches()
        } else {
            edge.protected_approaches().clone()
        };
        for coord in protected_approaches {
            let local = frame.to_local(coord).map_err(|error| {
                vec![recipe_issue(format!(
                    "Waterfall seam approach conversion failed: {error}"
                ))]
            })?;
            protected_by_coord.insert(local, edge.preferred_level());
        }
    }
    if ring7_layout {
        let bridge_flank = ring_bridge_flank(profile);
        if !mask.contains(&bridge_flank.coord) {
            return Err(vec![recipe_issue(
                "Waterfall Ring7 patch cannot fit the bridge-flank landing",
            )]);
        }
        protected_by_coord.insert(bridge_flank.coord, bridge_flank.level);
        restored_surface_levels.insert(bridge_flank.coord, bridge_flank.level);
    }
    let mut seam_excluded_shelves = mask
        .iter()
        .copied()
        .filter_map(|coord| {
            let surface = TilePos::new(coord, profile.cliff_mid);
            match project_surface_through_walker_seams(&patch, frame, surface) {
                Ok(projected)
                    if projected.level <= profile.low_land
                        || projected.level >= profile.high_land =>
                {
                    Some(Ok(coord))
                }
                Ok(_) => None,
                Err(error) => Some(Err(recipe_issue(error))),
            }
        })
        .collect::<Result<BTreeSet<_>, _>>()
        .map_err(|issue| vec![issue])?;
    seam_excluded_shelves.extend(
        patch
            .shared_edges()
            .flat_map(|edge| edge.boundary_pairs().into_iter().map(|(local, _)| local))
            .map(|coord| frame.to_local(coord).map_err(recipe_issue))
            .collect::<Result<BTreeSet<_>, _>>()
            .map_err(|issue| vec![issue])?,
    );
    let escarpment = EscarpmentPlan::new(
        patch_radius,
        &mask,
        &water_coords,
        &protected_by_coord,
        &seam_excluded_shelves,
        profile,
        streams.map(|streams| streams.cliff),
    )?;
    let relief = streams.map(|streams| {
        ReliefPlan::new(
            patch_radius,
            &mask,
            &water_coords,
            &protected_by_coord,
            &escarpment,
            streams.relief,
        )
    });

    let mut columns = BTreeMap::new();
    let mut surfaces = BTreeMap::new();
    let mut water_nodes = BTreeMap::new();
    let mut water_by_coord = BTreeMap::new();
    for (lane_index, lane) in watercourse.main_lanes.iter().enumerate() {
        let feeder_prefix_len = watercourse
            .feeder_prefix_lengths
            .get(lane_index)
            .copied()
            .unwrap_or_default();
        for (index, coord) in lane.iter().copied().enumerate() {
            let cell =
                water_cell_for_lane(lane, index, feeder_prefix_len, profile).ok_or_else(|| {
                    vec![recipe_issue(format!(
                        "Waterfall lane omitted indexed coordinate {coord:?}"
                    ))]
                })?;
            water_by_coord.insert(coord, cell);
        }
    }
    for coord in &watercourse.basin {
        water_by_coord.entry(*coord).or_insert(WaterCell {
            bed_level: low_water_level - 1,
            fill_bottom: low_water_level,
            top: TilePos::new(*coord, low_water_level),
            state: LiquidFlowState::Still,
            downstream: None,
        });
    }

    let local_surface_levels = mask
        .iter()
        .copied()
        .map(|coord| {
            let level = water_by_coord.get(&coord).map_or_else(
                || land_surface_level(coord, &protected_by_coord, &escarpment, relief.as_ref()),
                |water| water.bed_level,
            );
            (coord, level)
        })
        .collect();
    let mut world_surface_levels = frame
        .levels_to_world(local_surface_levels)
        .map_err(|error| vec![recipe_issue(error)])?;
    let seam_shape = shape_walker_seams(&patch, &mut world_surface_levels)?;
    for (coord, level) in restored_surface_levels {
        let world_coord = frame
            .to_world(coord)
            .map_err(|error| vec![recipe_issue(error)])?;
        if seam_shape.required_surface(world_coord).is_none() {
            world_surface_levels.insert(world_coord, level);
        }
    }
    let surface_levels = frame
        .levels_to_local(world_surface_levels)
        .map_err(|error| vec![recipe_issue(error)])?;

    for coord in &mask {
        let bridge_deck = bridge_by_coord.get(coord).copied();
        if let Some(water) = water_by_coord.get(coord).copied() {
            let (column, bed) = water_column(water, bridge_deck);
            columns.insert(*coord, column);
            surfaces.insert(
                bed,
                SurfaceMetadata {
                    access: SurfaceAccess::NonStandable,
                    interior: None,
                },
            );
            if let Some(deck) = bridge_deck {
                surfaces.insert(
                    deck,
                    SurfaceMetadata {
                        access: SurfaceAccess::Ordinary,
                        interior: None,
                    },
                );
            }
            water_nodes.insert(
                water.top,
                LiquidNode {
                    state: water.state,
                    downstream: water.downstream,
                },
            );
        } else {
            let surface_level = surface_levels.get(coord).copied().ok_or_else(|| {
                vec![recipe_issue(format!(
                    "Waterfall land plan omitted coordinate {coord:?}"
                ))]
            })?;
            let surface = bridge_deck.unwrap_or_else(|| TilePos::new(*coord, surface_level));
            columns.insert(*coord, land_column(surface_level, bridge_deck));
            surfaces.insert(
                surface,
                SurfaceMetadata {
                    access: if escarpment.shelves.contains(coord) && bridge_deck.is_none() {
                        SurfaceAccess::SpecialMovement(CLIFF_SHELF_REGION)
                    } else {
                        SurfaceAccess::Ordinary
                    },
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
        volume.surfaces.iter().find_map(|(surface, metadata)| {
            (surface.coord == coord && metadata.access == SurfaceAccess::Ordinary)
                .then_some(*surface)
        })
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
    let view_hint = waterfall_view_hint(patch_radius, level_height, profile)?;

    let mut plan = GeneratedPatchPlan {
        patch_id: patch.id,
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
        structures: StructurePlan {
            by_id: BTreeMap::from([(
                StructureId(0),
                PlannedStructure {
                    kind: StructureKind::Bridge,
                    voxels: bridge,
                },
            )]),
        },
        blockers: BTreeSet::new(),
        lights: BTreeMap::new(),
        biome_regions,
        interiors: InteriorPlan::default(),
        anchors,
        view_hint,
    };
    frame
        .patch_to_world(&mut plan)
        .map_err(|error| vec![recipe_issue(error)])?;
    seam_shape.apply(&mut plan.volume)?;
    if composite_layout {
        let critical_landing = bypass
            .first()
            .and_then(|lane| lane.first())
            .copied()
            .ok_or_else(|| vec![recipe_issue("Waterfall bypass has no high landing")])
            .and_then(|position| {
                frame
                    .position_to_world(position)
                    .map_err(|error| vec![recipe_issue(error)])
            })?;
        remove_closed_ordinary_pockets(&patch, critical_landing, &mut plan.volume);
        let ordinary = OrdinaryGraph::from_volume(&plan.volume, None);
        let world_bypass = bypass
            .iter()
            .flatten()
            .copied()
            .map(|position| {
                frame
                    .position_to_world(position)
                    .map_err(|error| vec![recipe_issue(error)])
            })
            .collect::<Result<Vec<_>, _>>()?;
        let party_start = world_bypass
            .iter()
            .copied()
            .find(|position| ordinary.contains(*position))
            .ok_or_else(|| {
                vec![recipe_issue(
                    "Waterfall bypass has no ordinary high landing",
                )]
            })?;
        let hostile_start = world_bypass
            .iter()
            .rev()
            .copied()
            .find(|position| ordinary.contains(*position))
            .ok_or_else(|| vec![recipe_issue("Waterfall bypass has no ordinary low landing")])?;
        plan.anchors.insert(PARTY_START.to_owned(), party_start);
        plan.anchors.insert(HOSTILE_START.to_owned(), hostile_start);
    }
    let ordinary_surfaces = plan
        .volume
        .surfaces
        .iter()
        .filter_map(|(position, metadata)| {
            (metadata.access == SurfaceAccess::Ordinary).then_some((position.coord, *position))
        })
        .collect::<BTreeMap<_, _>>();
    let mut vegetation_reserved = plan
        .liquids
        .bodies
        .values()
        .flat_map(|body| body.nodes.keys().map(|position| position.coord))
        .collect::<BTreeSet<_>>();
    let mut route_coords = bypass
        .iter()
        .chain(&secondary_bypass)
        .flatten()
        .chain(&secondary_apron)
        .chain(&ring_secondary_flank)
        .chain(&bridge_abutment)
        .map(|position| {
            frame.to_world(position.coord).map_err(|error| {
                vec![recipe_issue(format!(
                    "Waterfall vegetation route conversion failed: {error}"
                ))]
            })
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    if ring7_layout {
        route_coords.insert(
            frame
                .to_world(ring_bridge_flank(profile).coord)
                .map_err(|error| {
                    vec![recipe_issue(format!(
                        "Waterfall vegetation bridge-flank conversion failed: {error}"
                    ))]
                })?,
        );
    }
    for coord in route_coords
        .into_iter()
        .chain(
            plan.structures
                .by_id
                .values()
                .flat_map(|structure| structure.voxels.iter().map(|voxel| voxel.coord)),
        )
        .chain(plan.anchors.values().map(|anchor| anchor.coord))
    {
        vegetation_reserved.extend(coord.within_radius(2));
    }
    vegetation_reserved.extend(waterfall_seam_vegetation_reservations(&patch));
    let eligible_dry = ordinary_surfaces
        .keys()
        .filter(|coord| !vegetation_reserved.contains(coord))
        .copied()
        .collect::<BTreeSet<_>>();
    let grass_target = eligible_dry.len().saturating_mul(WATERFALL_GRASS_PERCENT) / 100;
    let tree_target = if patch.layout().kind == LayoutKind::Ring19 {
        2
    } else {
        WATERFALL_TREE_TARGET
    };
    append_landform_vegetation(
        "Waterfall",
        vegetation,
        &ordinary_surfaces,
        &eligible_dry,
        &eligible_dry,
        &vegetation_reserved,
        tree_target,
        grass_target,
        streams.map(|streams| streams.trees),
        streams.map(|streams| streams.grass),
        &mut plan.features,
        &mut plan.blockers,
    )
    .map_err(|error| vec![recipe_issue(error)])?;
    let seam_issues = validate_patch_walker_seams(&patch, &plan.volume);
    if seam_issues.is_empty() {
        Ok(plan)
    } else {
        Err(seam_issues)
    }
}

#[cfg(test)]
fn waterfall_rotation(patch: &PatchRecipeContext<'_>) -> Result<u8, Vec<WorldValidationIssue>> {
    Ok(WaterfallHydrology::resolve(patch)?.rotation())
}

fn validate_waterfall_liquid_ports(
    hydrology: &WaterfallHydrology,
    watercourse: &Watercourse,
) -> Result<(), Vec<WorldValidationIssue>> {
    if hydrology.kind == LayoutKind::Single {
        return Ok(());
    }
    let starts = watercourse
        .main_lanes
        .iter()
        .filter_map(|lane| lane.first().copied())
        .collect::<BTreeSet<_>>();
    let ends = watercourse
        .main_lanes
        .iter()
        .filter_map(|lane| lane.last().copied())
        .collect::<BTreeSet<_>>();
    let Some(outlet) = &hydrology.outlet else {
        return Err(vec![recipe_issue(
            "Composite Waterfall has no resolved liquid outlet",
        )]);
    };
    if outlet.boundary != ends {
        return Err(vec![recipe_issue(
            "Composite Waterfall outlet does not exactly match all three downstream water lanes",
        )]);
    }
    if !outlet.elevation.admits(hydrology.profile.low_water) {
        return Err(vec![recipe_issue(format!(
            "Composite Waterfall outlet does not admit low-water level {}",
            hydrology.profile.low_water
        ))]);
    }
    let wet = watercourse.coordinates();
    if !outlet.inward_approach.is_subset(&wet) {
        return Err(vec![recipe_issue(
            "Composite Waterfall outlet does not keep its complete resolved approach wet",
        )]);
    }
    if let Some(inlet) = &hydrology.inlet {
        if inlet.boundary != starts {
            return Err(vec![recipe_issue(
                "Composite Waterfall inlet does not exactly match all three upstream water lanes",
            )]);
        }
        if !inlet.elevation.admits(hydrology.profile.high_water) {
            return Err(vec![recipe_issue(format!(
                "Composite Waterfall inlet does not admit high-water level {}",
                hydrology.profile.high_water
            ))]);
        }
        if !inlet.inward_approach.is_subset(&wet) {
            return Err(vec![recipe_issue(
                "Composite Waterfall inlet does not keep its complete resolved approach wet",
            )]);
        }
    }
    Ok(())
}

fn project_surface_through_walker_seams(
    patch: &PatchRecipeContext<'_>,
    frame: LocalPatchFrame,
    surface: TilePos,
) -> Result<TilePos, String> {
    let world = frame.position_to_world(surface)?;
    let mut level = world.level;
    for edge in patch.shared_edges() {
        let approaches = edge
            .walker_ports()
            .into_iter()
            .flat_map(|port| port.first_approach)
            .collect::<BTreeSet<_>>();
        let Some(distance) = approaches
            .iter()
            .map(|approach| approach.distance(world.coord))
            .min()
        else {
            continue;
        };
        let distance = i32::try_from(distance).unwrap_or(i32::MAX);
        level = level
            .min(edge.preferred_level().saturating_add(distance))
            .max(edge.preferred_level().saturating_sub(distance));
    }
    frame.position_to_local(TilePos::new(world.coord, level))
}

fn remove_closed_ordinary_pockets(
    patch: &PatchRecipeContext<'_>,
    critical_landing: TilePos,
    volume: &mut VolumePlan,
) {
    let maximum_closed_pocket_cells = if patch.layout().kind == LayoutKind::Ring19 {
        MAX_RING19_CLOSED_POCKET_CELLS
    } else {
        MAX_RING_CLOSED_POCKET_CELLS
    };
    let open_approaches = patch
        .shared_edges()
        .flat_map(|edge| {
            let level = edge.preferred_level();
            edge.walker_ports().into_iter().flat_map(move |port| {
                port.first_approach
                    .into_iter()
                    .map(move |coord| TilePos::new(coord, level))
            })
        })
        .collect::<BTreeSet<_>>();
    let ordinary = OrdinaryGraph::from_volume(volume, None);
    for component in ordinary_components(&ordinary) {
        if component.contains(&critical_landing)
            || !component.is_disjoint(&open_approaches)
            || component.len() > maximum_closed_pocket_cells
        {
            continue;
        }
        for position in component {
            if let Some(metadata) = volume.surfaces.get_mut(&position) {
                metadata.access = SurfaceAccess::SpecialMovement(RING_ISOLATED_TERRAIN_REGION);
            }
        }
    }
}

fn waterfall_view_hint(
    grid_radius: u32,
    level_height: f32,
    profile: WaterfallElevationProfile,
) -> Result<MapViewHint, Vec<WorldValidationIssue>> {
    let radius = u16::try_from(grid_radius).map_err(|error| {
        vec![recipe_issue(format!(
            "Waterfall radius is too large: {error}"
        ))]
    })?;
    let focus_level_twice = i16::try_from(profile.high_land.saturating_add(profile.low_land))
        .map_err(|error| {
            vec![recipe_issue(format!(
                "Waterfall camera focus level is unsupported: {error}"
            ))]
        })?;
    let focus_height = f32::from(focus_level_twice) * 0.5 * level_height;
    let frame = (f32::from(radius) * 3.5).max(13.0 * level_height * 3.0);
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

fn water_cell(
    coord: HexCoord,
    next: Option<HexCoord>,
    profile: WaterfallElevationProfile,
) -> WaterCell {
    let low_water_level = profile.low_water;
    let (bed_level, fill_bottom, top_level, state) = if coord.x() < FALL_SOURCE_X {
        let state = if coord.x() <= BYPASS_HIGH_X {
            LiquidFlowState::Still
        } else {
            LiquidFlowState::Rapid
        };
        (
            profile.high_water - 1,
            profile.high_water,
            profile.high_water,
            state,
        )
    } else if coord.x() == FALL_SOURCE_X {
        (
            low_water_level - 1,
            low_water_level,
            profile.high_water,
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
        (low_water_level - 1, low_water_level, low_water_level, state)
    };
    let top = TilePos::new(coord, top_level);
    let downstream = next.map(|next_coord| {
        let next_level = if next_coord.x() < FALL_TARGET_X {
            profile.high_water
        } else {
            low_water_level
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

fn water_cell_for_lane(
    lane: &[HexCoord],
    index: usize,
    feeder_prefix_len: usize,
    profile: WaterfallElevationProfile,
) -> Option<WaterCell> {
    let coord = lane.get(index).copied()?;
    let next = lane.get(index.saturating_add(1)).copied();
    if index >= feeder_prefix_len {
        return Some(water_cell(coord, next, profile));
    }
    Some(WaterCell {
        bed_level: profile.high_water - 1,
        fill_bottom: profile.high_water,
        top: TilePos::new(coord, profile.high_water),
        state: if index == 0 {
            LiquidFlowState::Still
        } else {
            LiquidFlowState::Current
        },
        downstream: next.map(|next| TilePos::new(next, profile.high_water)),
    })
}

#[derive(Debug, Clone)]
struct Watercourse {
    main_lanes: Vec<Vec<HexCoord>>,
    feeder_prefix_lengths: Vec<usize>,
    basin: BTreeSet<HexCoord>,
    flow_shape: WaterfallFlowShape,
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
        let mut lane = row.clone();
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
    let feeder_prefix_lengths = vec![0; main_lanes.len()];
    Ok(Watercourse {
        main_lanes,
        feeder_prefix_lengths,
        basin,
        flow_shape: WaterfallFlowShape::Straight,
    })
}

fn watercourse_for_hydrology(
    mask: &BTreeSet<HexCoord>,
    hydrology: &WaterfallHydrology,
    flow_shape: WaterfallFlowShape,
    dry_reservations: &BTreeSet<HexCoord>,
) -> Result<Watercourse, Vec<WorldValidationIssue>> {
    let mut watercourse = watercourse(mask)?;
    watercourse.flow_shape = flow_shape;
    let starts = watercourse
        .main_lanes
        .iter()
        .filter_map(|lane| lane.first().copied())
        .collect::<BTreeSet<_>>();
    let ends = watercourse
        .main_lanes
        .iter()
        .filter_map(|lane| lane.last().copied())
        .collect::<BTreeSet<_>>();
    if hydrology
        .outlet
        .as_ref()
        .is_some_and(|outlet| outlet.boundary != ends)
    {
        return Err(vec![recipe_issue(
            "Composite Waterfall outlet does not exactly match all three downstream water lanes",
        )]);
    }
    let Some(inlet) = &hydrology.inlet else {
        return validate_watercourse_reservations(watercourse, dry_reservations);
    };
    if inlet.boundary == starts {
        if flow_shape != WaterfallFlowShape::Straight {
            return Err(vec![recipe_issue(
                "Bent Waterfall authority unexpectedly matches the straight west inlet",
            )]);
        }
        return validate_watercourse_reservations(watercourse, dry_reservations);
    }
    if hydrology.kind != LayoutKind::Ring19
        || flow_shape != WaterfallFlowShape::BentNorthWest
        || inlet.side != HexSide::NorthWest
    {
        return Err(vec![recipe_issue(
            "Ring7 Waterfall inlet does not exactly match all three upstream water lanes",
        )]);
    }

    let inlet_prefixes = exact_inlet_prefixes(inlet, mask)?;
    let target_x = BENT_FEEDER_TARGET_X;
    let targets = watercourse
        .main_lanes
        .iter()
        .zip(target_x)
        .map(|(lane, target_x)| {
            lane.iter()
                .copied()
                .find(|coord| coord.x() == target_x)
                .ok_or_else(|| {
                    vec![recipe_issue(format!(
                        "Ring19 Waterfall lane cannot fit staggered feeder join x={target_x}"
                    ))]
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut retained_water = watercourse.basin.clone();
    for (lane, target) in watercourse.main_lanes.iter().zip(&targets) {
        let Some(index) = lane.iter().position(|coord| coord == target) else {
            return Err(vec![recipe_issue(
                "Ring19 Waterfall feeder target disappeared from its main lane",
            )]);
        };
        retained_water.extend(lane.get(index..).into_iter().flatten().copied());
    }
    let Some(feeders) = route_bent_feeders(
        mask,
        &inlet_prefixes,
        &targets,
        &retained_water,
        dry_reservations,
    ) else {
        return Err(vec![recipe_issue(
            "Ring19 Waterfall cannot route three disjoint bent feeder lanes before the bridge",
        )]);
    };
    for (lane_index, ((lane, target), feeder)) in watercourse
        .main_lanes
        .iter_mut()
        .zip(targets)
        .zip(feeders)
        .enumerate()
    {
        let Some(index) = lane.iter().position(|coord| *coord == target) else {
            return Err(vec![recipe_issue(
                "Ring19 Waterfall feeder target disappeared during replacement",
            )]);
        };
        let replacement = feeder
            .iter()
            .copied()
            .chain(
                lane.get(index.saturating_add(1)..)
                    .into_iter()
                    .flatten()
                    .copied(),
            )
            .collect();
        let Some(prefix_len) = watercourse.feeder_prefix_lengths.get_mut(lane_index) else {
            return Err(vec![recipe_issue(
                "Ring19 Waterfall feeder count disagrees with its authored lanes",
            )]);
        };
        *prefix_len = feeder.len().saturating_sub(1);
        *lane = replacement;
    }
    validate_bent_feeder_ribbon(&watercourse)?;
    validate_watercourse_reservations(watercourse, dry_reservations)
}

fn validate_bent_feeder_ribbon(watercourse: &Watercourse) -> Result<(), Vec<WorldValidationIssue>> {
    let prefixes = watercourse
        .main_lanes
        .iter()
        .zip(&watercourse.feeder_prefix_lengths)
        .map(|(lane, length)| {
            lane.iter()
                .take(length.saturating_add(1))
                .copied()
                .collect::<BTreeSet<_>>()
        })
        .collect::<Vec<_>>();
    if prefixes.len() != 3
        || prefixes.iter().any(BTreeSet::is_empty)
        || prefixes.windows(2).any(|pair| {
            let [first, second] = pair else {
                return true;
            };
            first
                .iter()
                .any(|coord| !second.iter().any(|other| coord.distance(*other) == 1))
                || second
                    .iter()
                    .any(|coord| !first.iter().any(|other| coord.distance(*other) == 1))
        })
    {
        return Err(vec![recipe_issue(
            "Ring19 Waterfall bent feeders do not form one cohesive three-lane ribbon",
        )]);
    }
    Ok(())
}

fn validate_watercourse_reservations(
    watercourse: Watercourse,
    dry_reservations: &BTreeSet<HexCoord>,
) -> Result<Watercourse, Vec<WorldValidationIssue>> {
    let overlap = watercourse
        .coordinates()
        .intersection(dry_reservations)
        .copied()
        .take(6)
        .collect::<Vec<_>>();
    if overlap.is_empty() {
        Ok(watercourse)
    } else {
        Err(vec![recipe_issue(format!(
            "Waterfall liquid overlaps protected dry or undeclared seam cells: {overlap:?}"
        ))])
    }
}

fn exact_inlet_prefixes(
    inlet: &WaterfallPort,
    mask: &BTreeSet<HexCoord>,
) -> Result<Vec<Vec<HexCoord>>, Vec<WorldValidationIssue>> {
    if inlet.approach_depth == 0 {
        return Err(vec![recipe_issue(
            "Ring19 Waterfall inlet has no protected inward approach",
        )]);
    }
    let depth = usize::try_from(inlet.approach_depth).map_err(|error| {
        vec![recipe_issue(format!(
            "Ring19 Waterfall inlet approach depth is unsupported: {error}"
        ))]
    })?;
    let inward = inlet.side.opposite();
    let mut observed = BTreeSet::new();
    let prefixes = inlet
        .ordered_boundary
        .iter()
        .map(|boundary| {
            let mut prefix = Vec::with_capacity(depth);
            let mut coord = *boundary;
            for _ in 0..depth {
                if !mask.contains(&coord) || !observed.insert(coord) {
                    return Err(vec![recipe_issue(
                        "Ring19 Waterfall inlet approach is outside its mask or overlaps another lane",
                    )]);
                }
                prefix.push(coord);
                coord = inward.neighbor(coord);
            }
            Ok(prefix)
        })
        .collect::<Result<Vec<_>, _>>()?;
    if observed != inlet.inward_approach {
        return Err(vec![recipe_issue(
            "Ring19 Waterfall inlet does not exactly match its resolved protected approach",
        )]);
    }
    Ok(prefixes)
}

fn route_bent_feeders(
    mask: &BTreeSet<HexCoord>,
    inlet_prefixes: &[Vec<HexCoord>],
    targets: &[HexCoord],
    retained_water: &BTreeSet<HexCoord>,
    dry_reservations: &BTreeSet<HexCoord>,
) -> Option<Vec<Vec<HexCoord>>> {
    let [first, second, third] = inlet_prefixes else {
        return None;
    };
    let [first_target, second_target, third_target] = targets else {
        return None;
    };
    let targets = [*first_target, *second_target, *third_target];
    let prefixes = [first, second, third];
    let mut blocked = retained_water
        .union(dry_reservations)
        .copied()
        .collect::<BTreeSet<_>>();
    for prefix in prefixes {
        blocked.extend(prefix.iter().copied());
    }
    let starts = prefixes
        .iter()
        .map(|prefix| prefix.last().copied())
        .collect::<Option<Vec<_>>>()?;
    for terminal in starts.iter().chain(&targets) {
        blocked.remove(terminal);
    }
    let allowed = mask.difference(&blocked).copied().collect::<BTreeSet<_>>();
    let routed = vertex_disjoint_paths(&allowed, &starts, &targets)?;
    let mut by_target = vec![Vec::new(), Vec::new(), Vec::new()];
    for (prefix, path) in prefixes.into_iter().zip(routed) {
        let endpoint = path.last().copied()?;
        let target_index = targets.iter().position(|target| *target == endpoint)?;
        let target_path = by_target.get_mut(target_index)?;
        if !target_path.is_empty() {
            return None;
        }
        let mut full = prefix.get(..prefix.len().saturating_sub(1))?.to_vec();
        full.extend(path);
        *target_path = full;
    }
    by_target
        .iter()
        .all(|path| !path.is_empty())
        .then_some(by_target)
}

fn bridge_tiles(
    mask: &BTreeSet<HexCoord>,
    profile: WaterfallElevationProfile,
) -> Result<BTreeSet<TilePos>, Vec<WorldValidationIssue>> {
    bridge_tiles_for_shape(mask, profile, WaterfallFlowShape::Straight)
}

fn bridge_tiles_for_shape(
    mask: &BTreeSet<HexCoord>,
    profile: WaterfallElevationProfile,
    flow_shape: WaterfallFlowShape,
) -> Result<BTreeSet<TilePos>, Vec<WorldValidationIssue>> {
    let (first_x, last_x) = bridge_span(flow_shape);
    let bridge: BTreeSet<_> = (first_x..=last_x)
        .flat_map(|x| {
            (-BRIDGE_BANK_Y..=BRIDGE_BANK_Y)
                .map(move |y| TilePos::new(HexCoord::from_axial(x, y), profile.bridge_deck))
        })
        .collect();
    if bridge
        .iter()
        .any(|position| !mask.contains(&position.coord))
    {
        return Err(vec![recipe_issue(
            "Waterfall mask cannot fit the required two-wide upstream bridge",
        )]);
    }
    Ok(bridge)
}

const fn bridge_span(flow_shape: WaterfallFlowShape) -> (i32, i32) {
    match flow_shape {
        WaterfallFlowShape::Straight => (BRIDGE_FIRST_X, BRIDGE_LAST_X),
        WaterfallFlowShape::BentNorthWest => (BENT_BRIDGE_FIRST_X, BENT_BRIDGE_LAST_X),
    }
}

fn bypass_tiles(
    grid_radius: u32,
    mask: &BTreeSet<HexCoord>,
    profile: WaterfallElevationProfile,
) -> Result<[Vec<TilePos>; 2], Vec<WorldValidationIssue>> {
    bypass_tiles_for_shape(grid_radius, mask, profile, WaterfallFlowShape::Straight)
}

fn bypass_tiles_for_shape(
    grid_radius: u32,
    mask: &BTreeSet<HexCoord>,
    profile: WaterfallElevationProfile,
    flow_shape: WaterfallFlowShape,
) -> Result<[Vec<TilePos>; 2], Vec<WorldValidationIssue>> {
    let (high_x, low_x) = critical_bypass_span(flow_shape);
    bypass_tiles_on_bank(grid_radius, mask, profile, high_x, low_x)
}

fn secondary_bypass_tiles(
    grid_radius: u32,
    mask: &BTreeSet<HexCoord>,
    profile: WaterfallElevationProfile,
) -> Result<[Vec<TilePos>; 2], Vec<WorldValidationIssue>> {
    let radius = i32::try_from(grid_radius).unwrap_or(i32::MAX);
    let offset = (radius / 3 + 1).min(6);
    let lane = |lane_y| {
        (SECONDARY_HIGH_X..=SECONDARY_LOW_X)
            .map(|x| {
                TilePos::new(
                    HexCoord::from_axial(x, lane_y),
                    secondary_slope_level(x, profile),
                )
            })
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
    profile: WaterfallElevationProfile,
    high_x: i32,
    low_x: i32,
) -> Result<[Vec<TilePos>; 2], Vec<WorldValidationIssue>> {
    let radius = i32::try_from(grid_radius).unwrap_or(i32::MAX);
    let offset = (radius / 3 + 1).min(6);
    let y = -offset;
    let lane = |lane_y| {
        (high_x..=low_x)
            .map(|x| {
                let level = profile.high_land - (x - high_x);
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

const fn critical_bypass_span(flow_shape: WaterfallFlowShape) -> (i32, i32) {
    match flow_shape {
        WaterfallFlowShape::Straight => (BYPASS_HIGH_X, BYPASS_LOW_X),
        WaterfallFlowShape::BentNorthWest => (BENT_BYPASS_HIGH_X, BENT_BYPASS_LOW_X),
    }
}

fn secondary_slope_level(x: i32, profile: WaterfallElevationProfile) -> i32 {
    let step = x.saturating_sub(SECONDARY_HIGH_X);
    let span = SECONDARY_LOW_X.saturating_sub(SECONDARY_HIGH_X).max(1);
    let drop = step
        .saturating_mul(profile.high_land.saturating_sub(profile.low_land))
        .checked_div(span)
        .unwrap_or_default();
    profile.high_land.saturating_sub(drop)
}

fn secondary_slope_apron(
    grid_radius: u32,
    mask: &BTreeSet<HexCoord>,
    profile: WaterfallElevationProfile,
) -> Result<Vec<TilePos>, Vec<WorldValidationIssue>> {
    let radius = i32::try_from(grid_radius).unwrap_or(i32::MAX);
    let offset = (radius / 3 + 1).min(6);
    let mut apron: Vec<_> = ((SECONDARY_HIGH_X + 1)..SECONDARY_LOW_X)
        .map(|x| {
            TilePos::new(
                HexCoord::from_axial(x, offset.saturating_add(1)),
                secondary_slope_level(x, profile),
            )
        })
        .chain(((SECONDARY_HIGH_X + 3)..=(SECONDARY_LOW_X - 3)).map(|x| {
            TilePos::new(
                HexCoord::from_axial(x, offset.saturating_add(2)),
                secondary_slope_level(x, profile),
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

fn ring_secondary_flank_apron(
    grid_radius: u32,
    mask: &BTreeSet<HexCoord>,
    profile: WaterfallElevationProfile,
) -> Result<Vec<TilePos>, Vec<WorldValidationIssue>> {
    let radius = i32::try_from(grid_radius).unwrap_or(i32::MAX);
    let lane_y = (radius / 3 + 1).min(6).saturating_sub(2);
    let apron = ((SECONDARY_HIGH_X + 2)..=(FALL_SOURCE_X - 1))
        .map(|x| {
            TilePos::new(
                HexCoord::from_axial(x, lane_y),
                secondary_slope_level(x, profile).saturating_sub(1),
            )
        })
        .collect::<Vec<_>>();
    if apron.iter().any(|position| !mask.contains(&position.coord)) {
        return Err(vec![recipe_issue(
            "Waterfall Ring7 mask cannot fit the secondary-slope flank apron",
        )]);
    }
    Ok(apron)
}

fn ring_bridge_flank(profile: WaterfallElevationProfile) -> TilePos {
    TilePos::new(RING_BRIDGE_FLANK.coord, profile.high_land)
}

fn bent_bridge_abutment(
    mask: &BTreeSet<HexCoord>,
    profile: WaterfallElevationProfile,
    flow_shape: WaterfallFlowShape,
) -> Result<Vec<TilePos>, Vec<WorldValidationIssue>> {
    if flow_shape == WaterfallFlowShape::Straight {
        return Ok(Vec::new());
    }
    let abutment = vec![
        TilePos::new(
            HexCoord::from_axial(BENT_BRIDGE_FIRST_X - 1, BRIDGE_BANK_Y),
            profile.high_land,
        ),
        TilePos::new(
            HexCoord::from_axial(BENT_BRIDGE_FIRST_X - 2, BRIDGE_BANK_Y),
            profile.high_land.saturating_sub(1),
        ),
    ];
    if abutment
        .iter()
        .any(|position| !mask.contains(&position.coord))
    {
        return Err(vec![recipe_issue(
            "Waterfall Ring19 mask cannot fit the bent bridge abutment",
        )]);
    }
    Ok(abutment)
}

fn land_surface_level(
    coord: HexCoord,
    bypass: &BTreeMap<HexCoord, i32>,
    escarpment: &EscarpmentPlan,
    relief: Option<&ReliefPlan>,
) -> i32 {
    if let Some(level) = bypass.get(&coord) {
        return *level;
    }
    if escarpment.shelves.contains(&coord) {
        return escarpment.profile.cliff_mid;
    }
    let base = escarpment.base_level(coord);
    base + relief.map_or(0, |relief| relief.height_at(coord))
}

#[derive(Debug)]
struct EscarpmentPlan {
    boundary_by_y: BTreeMap<i32, i32>,
    shelves: BTreeSet<HexCoord>,
    profile: WaterfallElevationProfile,
}

impl EscarpmentPlan {
    fn new(
        grid_radius: u32,
        mask: &BTreeSet<HexCoord>,
        water: &BTreeSet<HexCoord>,
        protected: &BTreeMap<HexCoord, i32>,
        excluded_shelves: &BTreeSet<HexCoord>,
        profile: WaterfallElevationProfile,
        stream: Option<SeedStream<'_>>,
    ) -> Result<Self, Vec<WorldValidationIssue>> {
        let phase = stream.map_or(0, |stream| {
            usize::try_from(stream.sample(0) % u64::try_from(CLIFF_PATTERN.len()).unwrap_or(1))
                .unwrap_or_default()
        });
        let reverse = stream.is_some_and(|stream| stream.sample(1) & 1 == 1);
        let ys: BTreeSet<_> = mask.iter().map(|coord| coord.y()).collect();
        let mut boundary_by_y = BTreeMap::new();
        for y in ys {
            let shifted = usize::try_from(
                y.rem_euclid(i32::try_from(CLIFF_PATTERN.len()).unwrap_or(i32::MAX)),
            )
            .unwrap_or_default();
            let index = if reverse {
                phase
                    .saturating_add(CLIFF_PATTERN.len())
                    .saturating_sub(shifted % CLIFF_PATTERN.len())
                    % CLIFF_PATTERN.len()
            } else {
                phase.saturating_add(shifted) % CLIFF_PATTERN.len()
            };
            let offset = if y.abs() <= BASIN_MAX_HALF_WIDTH {
                FALL_TARGET_X
            } else {
                CLIFF_PATTERN.get(index).copied().unwrap_or_default()
            }
            .clamp(-CLIFF_MAX_OFFSET, CLIFF_MAX_OFFSET);
            boundary_by_y.insert(y, offset);
        }

        let shelf_offsets: &[i32] = if excluded_shelves.is_empty() {
            &[0]
        } else {
            &[0, -1, 1]
        };
        let mut candidates: Vec<_> = boundary_by_y
            .iter()
            .flat_map(|(&y, &x)| {
                shelf_offsets.iter().filter_map(move |offset| {
                    let coord = HexCoord::from_axial(
                        x.saturating_add(*offset)
                            .clamp(-CLIFF_MAX_OFFSET, CLIFF_MAX_OFFSET),
                        y,
                    );
                    (mask.contains(&coord)
                        && !water.contains(&coord)
                        && !protected.contains_key(&coord)
                        && !excluded_shelves.contains(&coord))
                    .then_some(coord)
                })
            })
            .collect();
        candidates.sort_unstable();
        candidates.dedup();
        candidates.sort_unstable_by_key(|coord| {
            (
                stream.map_or_else(
                    || fallback_cliff_priority(*coord),
                    |stream| stream.sample_coord(*coord, 2),
                ),
                *coord,
            )
        });
        let target = usize::try_from((grid_radius / 2).clamp(4, 8)).unwrap_or(4);
        let mut shelves = BTreeSet::new();
        for coord in &candidates {
            if shelves.len() >= target {
                break;
            }
            if shelves
                .iter()
                .all(|selected: &HexCoord| selected.y().abs_diff(coord.y()) >= 2)
            {
                shelves.insert(*coord);
            }
        }
        for coord in candidates {
            if shelves.len() >= target {
                break;
            }
            shelves.insert(coord);
        }
        if shelves.len() < target {
            return Err(vec![recipe_issue(format!(
                "Waterfall escarpment can fit only {} of {target} required shelf cells",
                shelves.len()
            ))]);
        }

        Ok(Self {
            boundary_by_y,
            shelves,
            profile,
        })
    }

    fn boundary_x(&self, y: i32) -> i32 {
        self.boundary_by_y.get(&y).copied().unwrap_or(FALL_TARGET_X)
    }

    fn high_side(&self, coord: HexCoord) -> bool {
        coord.x() < self.boundary_x(coord.y())
    }

    fn base_level(&self, coord: HexCoord) -> i32 {
        if self.high_side(coord) {
            self.profile.high_land
        } else {
            self.profile.low_land
        }
    }
}

fn fallback_cliff_priority(coord: HexCoord) -> u64 {
    let x = u64::from(u32::from_le_bytes(coord.x().to_le_bytes()));
    let y = u64::from(u32::from_le_bytes(coord.y().to_le_bytes()));
    let z = u64::from(u32::from_le_bytes(coord.z().to_le_bytes()));
    let mut value = x ^ y.rotate_left(21) ^ z.rotate_left(42) ^ 0x9e37_79b9_7f4a_7c15;
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
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
        escarpment: &EscarpmentPlan,
        stream: SeedStream<'_>,
    ) -> Self {
        let protected: BTreeSet<_> = bypass.keys().copied().collect();
        let mut centers = BTreeSet::new();
        for (bank, high_bank) in [false, true].into_iter().enumerate() {
            let candidates: Vec<_> = mask
                .iter()
                .copied()
                .filter(|coord| escarpment.high_side(*coord) == high_bank)
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

fn land_column(surface: i32, bridge_deck: Option<TilePos>) -> VolumeColumn {
    let mut elements = vec![
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
    ];
    if let Some(deck) = bridge_deck {
        elements.push(VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(deck.level, deck.level.saturating_add(1)),
            material: SolidMaterialRole::Metal,
            cutaway_for: None,
        }));
    }
    VolumeColumn { elements }
}

fn water_column(cell: WaterCell, bridge_deck: Option<TilePos>) -> (VolumeColumn, TilePos) {
    let bed = TilePos::new(cell.top.coord, cell.bed_level);
    let mut elements = vec![
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
    ];
    if let Some(deck) = bridge_deck {
        elements.push(VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(deck.level, deck.level.saturating_add(1)),
            material: SolidMaterialRole::Metal,
            cutaway_for: None,
        }));
    }
    (VolumeColumn { elements }, bed)
}

fn waterfall_route_centres(
    grid_radius: u32,
    mask: &BTreeSet<HexCoord>,
    include_ring_landings: bool,
    profile: WaterfallElevationProfile,
    flow_shape: WaterfallFlowShape,
) -> Result<BTreeSet<HexCoord>, Vec<WorldValidationIssue>> {
    let bypass = bypass_tiles_for_shape(grid_radius, mask, profile, flow_shape)?;
    let secondary = secondary_bypass_tiles(grid_radius, mask, profile)?;
    let apron = secondary_slope_apron(grid_radius, mask, profile)?;
    let bridge = bridge_tiles_for_shape(mask, profile, flow_shape)?;
    let bridge_abutment = bent_bridge_abutment(mask, profile, flow_shape)?;
    let mut centres = bypass
        .iter()
        .chain(&secondary)
        .flatten()
        .chain(&apron)
        .map(|position| position.coord)
        .chain(bridge.into_iter().map(|position| position.coord))
        .chain(bridge_abutment.into_iter().map(|position| position.coord))
        .collect::<BTreeSet<_>>();
    if include_ring_landings {
        centres.extend(
            ring_secondary_flank_apron(grid_radius, mask, profile)?
                .into_iter()
                .map(|position| position.coord),
        );
        centres.insert(RING_BRIDGE_FLANK.coord);
    }
    Ok(centres)
}

fn waterfall_feeder_exclusions(
    grid_radius: u32,
    mask: &BTreeSet<HexCoord>,
    include_ring_landings: bool,
    profile: WaterfallElevationProfile,
    flow_shape: WaterfallFlowShape,
) -> Result<BTreeSet<HexCoord>, Vec<WorldValidationIssue>> {
    let mut exclusions = waterfall_route_centres(
        grid_radius,
        mask,
        include_ring_landings,
        profile,
        flow_shape,
    )?;
    for bridge in bridge_tiles_for_shape(mask, profile, flow_shape)? {
        exclusions.remove(&bridge.coord);
    }
    Ok(exclusions)
}

fn extend_feeder_seam_exclusions(
    patch: &PatchRecipeContext<'_>,
    frame: LocalPatchFrame,
    hydrology: &WaterfallHydrology,
    exclusions: &mut BTreeSet<HexCoord>,
) -> Result<(), Vec<WorldValidationIssue>> {
    let authorized_liquid = hydrology
        .inlet
        .iter()
        .chain(hydrology.outlet.iter())
        .flat_map(|port| {
            port.boundary
                .iter()
                .chain(port.inward_approach.iter())
                .copied()
        })
        .collect::<BTreeSet<_>>();
    let geometric_boundary = if hydrology.kind == LayoutKind::Ring19 {
        patch
            .mask()
            .iter()
            .copied()
            .filter(|coord| {
                coord
                    .neighbors()
                    .into_iter()
                    .any(|neighbor| !patch.mask().contains(&neighbor))
            })
            .collect()
    } else {
        BTreeSet::new()
    };
    let protected = patch
        .protected_approaches()
        .into_iter()
        .chain(
            patch
                .shared_edges()
                .flat_map(|edge| edge.boundary_pairs().into_iter().map(|(inside, _)| inside)),
        )
        .chain(geometric_boundary)
        .map(|coord| {
            frame.to_local(coord).map_err(|error| {
                vec![recipe_issue(format!(
                    "Waterfall feeder seam exclusion conversion failed: {error}"
                ))]
            })
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    exclusions.extend(protected.difference(&authorized_liquid).copied());
    Ok(())
}

fn validate_waterfall_vegetation(
    vegetation_objects: &LandformVegetationSet,
    volume: &VolumePlan,
    liquids: &LiquidPlan,
    features: &FeaturePlan,
    structures: &StructurePlan,
    blockers: &BTreeSet<TilePos>,
    anchors: &BTreeMap<String, TilePos>,
    buffered_centres: impl IntoIterator<Item = HexCoord>,
    exact_reserved: impl IntoIterator<Item = HexCoord>,
    issues: &mut Vec<WorldValidationIssue>,
) -> (LandformVegetationMetrics, u32) {
    let ordinary_surfaces = volume
        .surfaces
        .iter()
        .filter_map(|(position, metadata)| {
            (metadata.access == SurfaceAccess::Ordinary).then_some((position.coord, *position))
        })
        .collect::<BTreeMap<_, _>>();
    let mut reserved = liquids
        .bodies
        .values()
        .flat_map(|body| body.nodes.keys().map(|position| position.coord))
        .collect::<BTreeSet<_>>();
    for coord in buffered_centres
        .into_iter()
        .chain(
            structures
                .by_id
                .values()
                .flat_map(|structure| structure.voxels.iter().map(|voxel| voxel.coord)),
        )
        .chain(anchors.values().map(|anchor| anchor.coord))
    {
        reserved.extend(coord.within_radius(2));
    }
    reserved.extend(exact_reserved);
    let no_nonvegetation_blockers = BTreeSet::new();
    let vegetation = match validate_landform_vegetation(
        "Waterfall",
        vegetation_objects,
        &[LandformVegetationDomain {
            surfaces: &ordinary_surfaces,
            reserved: &reserved,
        }],
        features,
        &no_nonvegetation_blockers,
        blockers,
    ) {
        Ok(metrics) => metrics,
        Err(errors) => {
            issues.extend(errors.into_iter().map(recipe_issue));
            LandformVegetationMetrics { trees: 0, grass: 0 }
        }
    };
    if !(2..=5).contains(&vegetation.trees) {
        issues.push(recipe_issue(format!(
            "Waterfall has {} authored trees; expected 2 through 5",
            vegetation.trees
        )));
    }
    let eligible_dry = ordinary_surfaces
        .keys()
        .filter(|coord| !reserved.contains(coord))
        .copied()
        .collect::<BTreeSet<_>>();
    for feature in features.by_id.values() {
        if !eligible_dry.contains(&feature.root.coord) {
            issues.push(recipe_issue(format!(
                "Waterfall authored vegetation at {:?} leaves eligible dry terrain (reserved={}, \
                 surface_access={:?})",
                feature.root,
                reserved.contains(&feature.root.coord),
                volume
                    .surfaces
                    .get(&feature.root)
                    .map(|metadata| metadata.access)
            )));
        }
    }
    let grass_percent = count_u32(vegetation.grass)
        .saturating_mul(100)
        .checked_div(count_u32(eligible_dry.len()))
        .unwrap_or_default();
    if !(15..=25).contains(&grass_percent) {
        issues.push(recipe_issue(format!(
            "Waterfall covers {grass_percent}% of eligible dry surfaces with grass; expected 15 through 25%"
        )));
    }
    (vegetation, grass_percent)
}

fn waterfall_seam_vegetation_reservations(patch: &PatchRecipeContext<'_>) -> BTreeSet<HexCoord> {
    let protected = patch.protected_approaches();
    if patch.layout().kind == LayoutKind::Ring19 {
        protected
    } else {
        protected
            .into_iter()
            .flat_map(|coord| coord.within_radius(2))
            .collect()
    }
}

pub(crate) fn validate_patch(
    patch: PatchRecipeContext<'_>,
    plan: &GeneratedPatchPlan,
    catalog: &RuntimeArtCatalog,
) -> WorldValidation<()> {
    let vegetation = match LandformVegetationSet::resolve(
        catalog,
        V3EnvironmentSettings::TemperateGrassland,
        "Waterfall",
    ) {
        Ok(vegetation) => vegetation,
        Err(error) => return WorldValidation::Invalid(vec![recipe_issue(error)]),
    };
    let mut issues = validate_patch_walker_seams(&patch, &plan.volume);
    let hydrology = match WaterfallHydrology::resolve(&patch) {
        Ok(hydrology) => hydrology,
        Err(hydrology_issues) => return WorldValidation::Invalid(hydrology_issues),
    };
    let rotation = hydrology.rotation();
    let frame = match patch.local_frame_with_rotation(rotation) {
        Ok(frame) => frame,
        Err(error) => {
            return WorldValidation::Invalid(vec![recipe_issue(format!(
                "Waterfall validation frame failed: {error}"
            ))]);
        }
    };
    let local_mask = match frame.local_mask(patch.mask()) {
        Ok(mask) => mask,
        Err(error) => {
            return WorldValidation::Invalid(vec![recipe_issue(format!(
                "Waterfall validation mask conversion failed: {error}"
            ))]);
        }
    };
    let local_hydrology = match hydrology.to_local(frame) {
        Ok(hydrology) => hydrology,
        Err(hydrology_issues) => return WorldValidation::Invalid(hydrology_issues),
    };
    let profile = local_hydrology.profile;
    let flow_shape = match local_hydrology.flow_shape() {
        Ok(flow_shape) => flow_shape,
        Err(flow_issues) => return WorldValidation::Invalid(flow_issues),
    };
    let local_protected_centres = match waterfall_route_centres(
        frame.scale(),
        &local_mask,
        patch.layout().kind == LayoutKind::Ring7,
        profile,
        flow_shape,
    ) {
        Ok(centres) => centres,
        Err(mut route_issues) => {
            issues.append(&mut route_issues);
            BTreeSet::new()
        }
    };
    let protected_centres = match local_protected_centres
        .into_iter()
        .map(|coord| frame.to_world(coord).map_err(recipe_issue))
        .collect::<Result<BTreeSet<_>, _>>()
    {
        Ok(centres) => centres,
        Err(issue) => {
            issues.push(issue);
            BTreeSet::new()
        }
    };
    validate_waterfall_vegetation(
        &vegetation,
        &plan.volume,
        &plan.liquids,
        &plan.features,
        &plan.structures,
        &plan.blockers,
        &plan.anchors,
        protected_centres,
        waterfall_seam_vegetation_reservations(&patch),
        &mut issues,
    );
    let mut dry_reservations = match waterfall_feeder_exclusions(
        frame.scale(),
        &local_mask,
        patch.layout().kind == LayoutKind::Ring7,
        profile,
        flow_shape,
    ) {
        Ok(reservations) => reservations,
        Err(reservation_issues) => return WorldValidation::Invalid(reservation_issues),
    };
    if let Err(exclusion_issues) =
        extend_feeder_seam_exclusions(&patch, frame, &local_hydrology, &mut dry_reservations)
    {
        return WorldValidation::Invalid(exclusion_issues);
    }
    let watercourse = match watercourse_for_hydrology(
        &local_mask,
        &local_hydrology,
        flow_shape,
        &dry_reservations,
    ) {
        Ok(watercourse) => watercourse,
        Err(issues) => return WorldValidation::Invalid(issues),
    };
    if let Err(port_issues) = validate_waterfall_liquid_ports(&local_hydrology, &watercourse) {
        return WorldValidation::Invalid(port_issues);
    }
    let seam_context = match stitched_seam_context(patch, plan, frame, profile, flow_shape) {
        Ok(context) => context,
        Err(issue) => return WorldValidation::Invalid(vec![issue]),
    };
    match frame.canonical_local_world(plan) {
        Ok(plan) => {
            validate_stitched_waterfall(&plan, &seam_context, &watercourse, profile, &mut issues);
            if issues.is_empty() {
                WorldValidation::Valid(())
            } else {
                WorldValidation::Invalid(issues)
            }
        }
        Err(error) => WorldValidation::Invalid(vec![recipe_issue(format!(
            "Waterfall validation projection failed: {error}"
        ))]),
    }
}

#[derive(Debug, Default)]
struct StitchedSeamContext {
    open_approaches: BTreeSet<TilePos>,
    closures: BTreeSet<TilePos>,
    projected_authored: BTreeMap<TilePos, TilePos>,
    mid_thresholds: BTreeMap<HexCoord, i32>,
    projected_relief_levels: BTreeMap<(HexCoord, i32), i32>,
    ring_secondary_flank: Vec<TilePos>,
    maximum_closed_pocket_cells: usize,
}

fn stitched_seam_context(
    patch: PatchRecipeContext<'_>,
    plan: &GeneratedPatchPlan,
    frame: LocalPatchFrame,
    profile: WaterfallElevationProfile,
    flow_shape: WaterfallFlowShape,
) -> Result<StitchedSeamContext, WorldValidationIssue> {
    let mut boundary_coords = BTreeSet::new();
    let mut open_approaches = BTreeSet::new();
    for edge in patch.shared_edges() {
        boundary_coords.extend(edge.boundary_pairs().into_iter().map(|(local, _)| local));
        for port in edge.walker_ports() {
            for coord in port.first_approach {
                open_approaches.insert(
                    frame
                        .position_to_local(TilePos::new(coord, edge.preferred_level()))
                        .map_err(recipe_issue)?,
                );
            }
        }
    }
    let local_mask = frame.local_mask(patch.mask()).map_err(recipe_issue)?;
    let critical = bypass_tiles_for_shape(frame.scale(), &local_mask, profile, flow_shape)
        .map_err(first_recipe_issue)?;
    let secondary =
        secondary_bypass_tiles(frame.scale(), &local_mask, profile).map_err(first_recipe_issue)?;
    let apron =
        secondary_slope_apron(frame.scale(), &local_mask, profile).map_err(first_recipe_issue)?;
    let bridge_abutment =
        bent_bridge_abutment(&local_mask, profile, flow_shape).map_err(first_recipe_issue)?;
    let ring_secondary_flank = if patch.layout().kind == LayoutKind::Ring7 {
        ring_secondary_flank_apron(frame.scale(), &local_mask, profile)
            .map_err(first_recipe_issue)?
    } else {
        Vec::new()
    };
    let ring_bridge_flank =
        (patch.layout().kind == LayoutKind::Ring7).then(|| ring_bridge_flank(profile));
    let mut projected_authored = BTreeMap::new();
    for expected in critical
        .into_iter()
        .flatten()
        .chain(secondary.into_iter().flatten())
        .chain(apron)
        .chain(bridge_abutment)
        .chain(ring_secondary_flank.iter().copied())
        .chain(ring_bridge_flank)
    {
        let projected =
            project_surface_through_walker_seams(&patch, frame, expected).map_err(recipe_issue)?;
        let actual = if open_approaches
            .iter()
            .any(|approach| approach.coord == expected.coord)
        {
            projected
        } else {
            expected
        };
        projected_authored.insert(expected, actual);
    }
    let mid_thresholds = local_mask
        .iter()
        .copied()
        .map(|coord| {
            project_surface_through_walker_seams(
                &patch,
                frame,
                TilePos::new(coord, profile.cliff_mid),
            )
            .map(|projected| (coord, projected.level))
            .map_err(recipe_issue)
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let projected_relief_levels = local_mask
        .iter()
        .copied()
        .flat_map(|coord| {
            [
                profile.low_land,
                profile.low_land + 1,
                profile.low_land + 2,
                profile.high_land,
                profile.high_land + 1,
                profile.high_land + 2,
            ]
            .into_iter()
            .map(move |level| (coord, level))
        })
        .map(|(coord, level)| {
            project_surface_through_walker_seams(&patch, frame, TilePos::new(coord, level))
                .map(|projected| ((coord, level), projected.level))
                .map_err(recipe_issue)
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let closures = plan
        .volume
        .surfaces
        .iter()
        .filter(|(position, metadata)| {
            boundary_coords.contains(&position.coord) && is_seam_closure_access(metadata.access)
        })
        .map(|(position, _)| frame.position_to_local(*position).map_err(recipe_issue))
        .collect::<Result<BTreeSet<_>, _>>()?;
    Ok(StitchedSeamContext {
        open_approaches,
        closures,
        projected_authored,
        mid_thresholds,
        projected_relief_levels,
        ring_secondary_flank,
        maximum_closed_pocket_cells: if patch.layout().kind == LayoutKind::Ring19 {
            MAX_RING19_CLOSED_POCKET_CELLS
        } else {
            MAX_RING_CLOSED_POCKET_CELLS
        },
    })
}

fn first_recipe_issue(mut issues: Vec<WorldValidationIssue>) -> WorldValidationIssue {
    issues
        .drain(..)
        .next()
        .unwrap_or_else(|| recipe_issue("Waterfall authored projection failed"))
}

fn validate_stitched_waterfall(
    plan: &GeneratedWorldPlan,
    seam: &StitchedSeamContext,
    watercourse: &Watercourse,
    profile: WaterfallElevationProfile,
    issues: &mut Vec<WorldValidationIssue>,
) {
    let Some(body) = plan.liquids.bodies.get(&LiquidBodyId(0)) else {
        issues.push(recipe_issue("Waterfall patch has no canonical water body"));
        return;
    };
    if plan.liquids.bodies.len() != 1 || body.material != FillMaterialRole::Water {
        issues.push(recipe_issue(
            "Waterfall patch must contain exactly one water-only liquid body",
        ));
    }

    validate_flow_stages(body, Some(watercourse), profile, issues);
    let fall_nodes = body
        .nodes
        .iter()
        .filter_map(|(position, node)| {
            (node.state == LiquidFlowState::Fall).then_some((*position, node.downstream))
        })
        .collect::<Vec<_>>();
    validate_fall(&fall_nodes, profile, issues);
    if !body
        .nodes
        .values()
        .any(|node| node.state == LiquidFlowState::Rapid)
        || !body
            .nodes
            .values()
            .any(|node| node.state == LiquidFlowState::Current)
        || !body
            .nodes
            .values()
            .any(|node| node.state == LiquidFlowState::Still)
    {
        issues.push(recipe_issue(
            "Waterfall patch must retain still, rapid, fall, and current flow stages",
        ));
    }

    let mut surfaces_by_coord = BTreeMap::<HexCoord, Vec<(TilePos, SurfaceMetadata)>>::new();
    for (position, metadata) in &plan.volume.surfaces {
        surfaces_by_coord
            .entry(position.coord)
            .or_default()
            .push((*position, *metadata));
    }
    let ordinary = OrdinaryGraph::from_volume(&plan.volume, Some(&plan.blockers));
    validate_liquid_beds(plan, body, &surfaces_by_coord, issues);
    validate_bridge(plan, profile, watercourse.flow_shape, issues);
    validate_bent_bridge_abutment(
        plan,
        &ordinary,
        seam,
        profile,
        watercourse.flow_shape,
        issues,
    );
    validate_stitched_escarpment(plan, seam, profile, watercourse.flow_shape, issues);
    validate_stitched_closed_pockets(plan, seam, issues);

    let bypass = match bypass_tiles_for_shape(
        plan.layout.grid_radius,
        &plan.layout.footprint,
        profile,
        watercourse.flow_shape,
    ) {
        Ok(bypass) => validate_stitched_bypass(
            plan,
            &ordinary,
            &bypass,
            "critical",
            {
                let (high_x, low_x) = critical_bypass_span(watercourse.flow_shape);
                inclusive_span_len(high_x, low_x)
            },
            seam,
            profile,
            issues,
        ),
        Err(mut bypass_issues) => {
            issues.append(&mut bypass_issues);
            [Vec::new(), Vec::new()]
        }
    };
    let secondary_bypass =
        match secondary_bypass_tiles(plan.layout.grid_radius, &plan.layout.footprint, profile) {
            Ok(bypass) => validate_stitched_bypass(
                plan,
                &ordinary,
                &bypass,
                "secondary",
                inclusive_span_len(SECONDARY_HIGH_X, SECONDARY_LOW_X),
                seam,
                profile,
                issues,
            ),
            Err(mut bypass_issues) => {
                issues.append(&mut bypass_issues);
                [Vec::new(), Vec::new()]
            }
        };
    let secondary_apron =
        match secondary_slope_apron(plan.layout.grid_radius, &plan.layout.footprint, profile) {
            Ok(apron) => apron,
            Err(mut apron_issues) => {
                issues.append(&mut apron_issues);
                Vec::new()
            }
        };
    validate_stitched_secondary_apron(plan, &ordinary, &secondary_apron, seam, issues);
    if !seam.ring_secondary_flank.is_empty() {
        validate_stitched_ring_landings(plan, &ordinary, seam, profile, issues);
    }
    validate_stitched_route_redundancy(&ordinary, &bypass, &secondary_bypass, profile, issues);
    validate_stitched_network(plan, &ordinary, &bypass, issues);
}

fn validate_stitched_bypass(
    plan: &GeneratedWorldPlan,
    ordinary: &OrdinaryGraph,
    bypass: &[Vec<TilePos>; 2],
    name: &str,
    expected_length: usize,
    seam: &StitchedSeamContext,
    profile: WaterfallElevationProfile,
    issues: &mut Vec<WorldValidationIssue>,
) -> [Vec<TilePos>; 2] {
    let [first_lane, second_lane] = bypass;
    if first_lane.len() != expected_length || second_lane.len() != expected_length {
        issues.push(recipe_issue(format!(
            "Waterfall {name} bypass must retain two authored {expected_length}-tile lanes"
        )));
        return [Vec::new(), Vec::new()];
    }
    let first_resolved = first_lane
        .iter()
        .map(|position| stitched_ordinary_surface(*position, ordinary, seam))
        .collect::<Vec<_>>();
    let second_resolved = second_lane
        .iter()
        .map(|position| stitched_ordinary_surface(*position, ordinary, seam))
        .collect::<Vec<_>>();
    for expected in first_lane.iter().chain(second_lane) {
        let Some(projected) = seam.projected_authored.get(expected).copied() else {
            issues.push(recipe_issue(format!(
                "Waterfall {name} bypass has no exact seam projection for {expected:?}"
            )));
            continue;
        };
        if !seam.closures.contains(&projected)
            && (!ordinary.contains(projected)
                || plan
                    .volume
                    .surfaces
                    .get(&projected)
                    .is_none_or(|metadata| metadata.access != SurfaceAccess::Ordinary))
        {
            issues.push(recipe_issue(format!(
                "Waterfall {name} bypass exact projected surface {projected:?} is not ordinary"
            )));
        }
    }

    let open_pair = |index: usize| {
        first_resolved
            .get(index)
            .copied()
            .flatten()
            .zip(second_resolved.get(index).copied().flatten())
            .is_some_and(|(first, second)| {
                plan.volume
                    .surfaces
                    .get(&first)
                    .is_some_and(|metadata| metadata.access == SurfaceAccess::Ordinary)
                    && plan
                        .volume
                        .surfaces
                        .get(&second)
                        .is_some_and(|metadata| metadata.access == SurfaceAccess::Ordinary)
                    && ordinary.admits(first, second)
            })
    };
    let mut current_start = 0_usize;
    let mut current_len = 0_usize;
    let mut best_start = 0_usize;
    let mut best_len = 0_usize;
    for index in 0..expected_length {
        let connected_to_previous = index > 0
            && open_pair(index)
            && open_pair(index - 1)
            && first_resolved
                .get(index - 1)
                .copied()
                .flatten()
                .zip(first_resolved.get(index).copied().flatten())
                .zip(
                    second_resolved
                        .get(index - 1)
                        .copied()
                        .flatten()
                        .zip(second_resolved.get(index).copied().flatten()),
                )
                .is_some_and(|((previous_first, first), (previous_second, second))| {
                    ordinary.admits(previous_first, first)
                        && ordinary.admits(previous_second, second)
                });
        if connected_to_previous {
            current_len = current_len.saturating_add(1);
        } else if open_pair(index) {
            current_start = index;
            current_len = 1;
        } else {
            current_len = 0;
        }
        if current_len > best_len {
            best_start = current_start;
            best_len = current_len;
        }
    }
    let minimum = expected_length.saturating_sub(2);
    if best_len < minimum {
        issues.push(recipe_issue(format!(
            "Waterfall {name} bypass retains only {best_len}/{expected_length} contiguous \
             two-wide walker steps after seam shaping; expected at least {minimum}"
        )));
        return [Vec::new(), Vec::new()];
    }
    let best_end = best_start.saturating_add(best_len);
    let invalid_omissions = (0..expected_length)
        .filter(|index| *index < best_start || *index >= best_end)
        .filter(|index| {
            first_lane
                .get(*index)
                .copied()
                .zip(second_lane.get(*index).copied())
                .is_none_or(|(first, second)| {
                    ![first, second].iter().any(|expected| {
                        has_projected_seam_closure(*expected, seam)
                            || is_exact_seam_substitution(*expected, seam)
                    })
                })
        })
        .take(6)
        .collect::<Vec<_>>();
    if !invalid_omissions.is_empty() {
        let diagnostics = invalid_omissions
            .iter()
            .map(|index| {
                let expected = [
                    first_lane.get(*index).copied(),
                    second_lane.get(*index).copied(),
                ];
                let projected = expected.map(|position| {
                    position.map(|position| {
                        (
                            position,
                            seam.projected_authored.get(&position).copied(),
                            has_projected_seam_closure(position, seam),
                        )
                    })
                });
                (*index, projected)
            })
            .collect::<Vec<_>>();
        issues.push(recipe_issue(format!(
            "Waterfall {name} bypass omissions do not follow exact seam consequences: \
             {diagnostics:?}"
        )));
    }

    let retained: [Vec<TilePos>; 2] = [
        first_resolved
            .get(best_start..best_end)
            .unwrap_or_default()
            .iter()
            .copied()
            .flatten()
            .collect(),
        second_resolved
            .get(best_start..best_end)
            .unwrap_or_default()
            .iter()
            .copied()
            .flatten()
            .collect(),
    ];
    let ordinary_levels = retained
        .iter()
        .flatten()
        .map(|position| position.level)
        .collect::<BTreeSet<_>>();
    let relief = ordinary_levels
        .first()
        .zip(ordinary_levels.last())
        .map_or(0, |(low, high)| high.saturating_sub(*low));
    let minimum_relief = profile
        .high_land
        .saturating_sub(profile.low_land)
        .saturating_sub(2);
    if relief < minimum_relief {
        issues.push(recipe_issue(format!(
            "Waterfall {name} bypass spans only {relief} levels after seam shaping; expected at \
             least {minimum_relief}"
        )));
    }
    retained
}

fn stitched_ordinary_surface(
    expected: TilePos,
    ordinary: &OrdinaryGraph,
    seam: &StitchedSeamContext,
) -> Option<TilePos> {
    seam.projected_authored
        .get(&expected)
        .copied()
        .filter(|projected| ordinary.contains(*projected))
}

fn has_projected_seam_closure(expected: TilePos, seam: &StitchedSeamContext) -> bool {
    seam.projected_authored
        .get(&expected)
        .is_some_and(|projected| seam.closures.contains(projected))
}

fn is_exact_seam_substitution(expected: TilePos, seam: &StitchedSeamContext) -> bool {
    seam.projected_authored
        .get(&expected)
        .is_some_and(|projected| *projected != expected && seam.open_approaches.contains(projected))
}

fn validate_stitched_secondary_apron(
    _plan: &GeneratedWorldPlan,
    ordinary: &OrdinaryGraph,
    apron: &[TilePos],
    seam: &StitchedSeamContext,
    issues: &mut Vec<WorldValidationIssue>,
) {
    let ordinary_apron = apron
        .iter()
        .filter_map(|position| stitched_ordinary_surface(*position, ordinary, seam))
        .collect::<Vec<_>>();
    if apron.len() != secondary_apron_len() || ordinary_apron.len().saturating_add(2) < apron.len()
    {
        issues.push(recipe_issue(format!(
            "Waterfall stitched secondary apron retains {}/{} ordinary tiles",
            ordinary_apron.len(),
            apron.len()
        )));
    }
    let ordinary_set = ordinary_apron.iter().copied().collect::<BTreeSet<_>>();
    for position in ordinary_apron {
        if !ordinary
            .neighbors(position)
            .iter()
            .any(|neighbor| ordinary_set.contains(neighbor) || neighbor.level == position.level)
        {
            issues.push(recipe_issue(format!(
                "Waterfall stitched secondary-apron tile {position:?} is isolated"
            )));
        }
    }
    if apron
        .iter()
        .filter(|position| stitched_ordinary_surface(**position, ordinary, seam).is_none())
        .any(|position| !has_projected_seam_closure(*position, seam))
    {
        issues.push(recipe_issue(
            "Waterfall stitched secondary apron loses authored surfaces outside seam closures",
        ));
    }
}

fn validate_stitched_ring_landings(
    plan: &GeneratedWorldPlan,
    ordinary: &OrdinaryGraph,
    seam: &StitchedSeamContext,
    profile: WaterfallElevationProfile,
    issues: &mut Vec<WorldValidationIssue>,
) {
    let bridge_flank = ring_bridge_flank(profile);
    let mut resolved_flank = Vec::new();
    for expected in seam
        .ring_secondary_flank
        .iter()
        .copied()
        .chain(std::iter::once(bridge_flank))
    {
        let projected = seam.projected_authored.get(&expected).copied();
        match projected {
            Some(projected)
                if ordinary.contains(projected)
                    && plan
                        .volume
                        .surfaces
                        .get(&projected)
                        .is_some_and(|metadata| metadata.access == SurfaceAccess::Ordinary) =>
            {
                if expected != bridge_flank {
                    resolved_flank.push(projected);
                }
            }
            _ => issues.push(recipe_issue(format!(
                "Waterfall Ring7 authored landing {expected:?} lacks its exact ordinary seam \
                 projection {projected:?}"
            ))),
        }
    }
    if resolved_flank.len() != seam.ring_secondary_flank.len()
        || resolved_flank
            .windows(2)
            .any(|pair| !matches!(pair, [first, second] if ordinary.admits(*first, *second)))
    {
        issues.push(recipe_issue(
            "Waterfall Ring7 secondary flank apron is not an exact contiguous walker route",
        ));
    }
    let secondary =
        secondary_bypass_tiles(plan.layout.grid_radius, &plan.layout.footprint, profile)
            .unwrap_or_else(|_| [Vec::new(), Vec::new()]);
    for (flank, expected_lane) in
        seam.ring_secondary_flank
            .iter()
            .zip(secondary.first().into_iter().flatten().filter(|position| {
                seam.ring_secondary_flank
                    .iter()
                    .any(|flank| flank.coord.x() == position.coord.x())
            }))
    {
        let flank = seam.projected_authored.get(flank).copied();
        let lane = seam.projected_authored.get(expected_lane).copied();
        if flank
            .zip(lane)
            .is_none_or(|(flank, lane)| !ordinary.admits(flank, lane))
        {
            issues.push(recipe_issue(format!(
                "Waterfall Ring7 secondary flank does not join its bypass at \
                 {flank:?}/{lane:?}"
            )));
        }
    }
    let authored_bridge_flank = ring_bridge_flank(profile);
    let bridge_flank = seam.projected_authored.get(&authored_bridge_flank).copied();
    let bridge_deck = TilePos::new(
        HexCoord::from_axial(BRIDGE_FIRST_X, RING_BRIDGE_FLANK.coord.y()),
        profile.bridge_deck,
    );
    if bridge_flank.is_none_or(|flank| !ordinary.admits(flank, bridge_deck)) {
        issues.push(recipe_issue(
            "Waterfall Ring7 bridge-flank landing does not join the metal bridge deck",
        ));
    }
}

fn validate_stitched_route_redundancy(
    ordinary: &OrdinaryGraph,
    critical: &[Vec<TilePos>; 2],
    secondary: &[Vec<TilePos>; 2],
    profile: WaterfallElevationProfile,
    issues: &mut Vec<WorldValidationIssue>,
) {
    let critical_tiles = critical.iter().flatten().copied().collect::<BTreeSet<_>>();
    let secondary_tiles = secondary.iter().flatten().copied().collect::<BTreeSet<_>>();
    if !critical_tiles.is_disjoint(&secondary_tiles) {
        issues.push(recipe_issue(
            "Waterfall stitched critical and secondary bypasses are not independent",
        ));
    }
    let terminals = [critical, secondary]
        .into_iter()
        .flat_map(|route| {
            route.iter().flat_map(|lane| {
                lane.first()
                    .copied()
                    .into_iter()
                    .chain(lane.last().copied())
            })
        })
        .collect::<Vec<_>>();
    if let Some(start) = terminals.first().copied() {
        let reachable = ordinary.distances_from(start);
        if terminals
            .iter()
            .any(|terminal| !reachable.contains_key(terminal))
        {
            issues.push(recipe_issue(
                "Waterfall stitched high/low bypass landings are not mutually reachable",
            ));
        }
    } else {
        issues.push(recipe_issue(
            "Waterfall stitched bypasses have no retained high/low landings",
        ));
    }
    for (name, route) in [("critical", critical), ("secondary", secondary)] {
        for (lane_index, lane) in route.iter().enumerate() {
            let Some((start, goal)) = lane.first().copied().zip(lane.last().copied()) else {
                issues.push(recipe_issue(format!(
                    "Waterfall stitched {name} lane {lane_index} is empty"
                )));
                continue;
            };
            let connected = ordinary.distances_from(start).contains_key(&goal);
            if start.level < profile.high_land.saturating_sub(2)
                || goal.level > profile.low_land.saturating_add(2)
                || !connected
            {
                issues.push(recipe_issue(format!(
                    "Waterfall stitched {name} lane {lane_index} does not retain an independent \
                     high-to-low ordinary route ({start:?} -> {goal:?}, connected={connected})"
                )));
            }
        }
    }
}

fn validate_stitched_closed_pockets(
    plan: &GeneratedWorldPlan,
    seam: &StitchedSeamContext,
    issues: &mut Vec<WorldValidationIssue>,
) {
    let marked = plan
        .volume
        .surfaces
        .iter()
        .filter_map(|(position, metadata)| {
            (metadata.access == SurfaceAccess::SpecialMovement(RING_ISOLATED_TERRAIN_REGION))
                .then_some(*position)
        })
        .collect::<BTreeSet<_>>();
    let Some(party) = plan.anchors.get(PARTY_START).copied() else {
        return;
    };
    let liquid_coords = plan
        .liquids
        .bodies
        .values()
        .flat_map(|body| body.nodes.keys().map(|position| position.coord))
        .collect::<BTreeSet<_>>();
    let mut reconstructed = plan.volume.clone();
    let mut unexpected_dry_access = Vec::new();
    for (position, metadata) in &mut reconstructed.surfaces {
        let exact_liquid_bed = metadata.access == SurfaceAccess::NonStandable
            && liquid_coords.contains(&position.coord);
        let exact_shelf = metadata.access == SurfaceAccess::SpecialMovement(CLIFF_SHELF_REGION);
        let exact_closure = seam.closures.contains(position);
        if exact_liquid_bed || exact_shelf || exact_closure {
            continue;
        }
        if !matches!(
            metadata.access,
            SurfaceAccess::Ordinary | SurfaceAccess::SpecialMovement(RING_ISOLATED_TERRAIN_REGION)
        ) {
            unexpected_dry_access.push((*position, metadata.access));
        }
        metadata.access = SurfaceAccess::Ordinary;
    }
    let graph = OrdinaryGraph::from_volume(&reconstructed, None);
    let closed_components = ordinary_components(&graph)
        .into_iter()
        .filter(|component| {
            !component.contains(&party) && component.is_disjoint(&seam.open_approaches)
        })
        .collect::<Vec<_>>();
    let expected = closed_components
        .iter()
        .flat_map(|component| component.iter().copied())
        .collect::<BTreeSet<_>>();
    if marked != expected
        || closed_components
            .iter()
            .any(|component| component.len() > seam.maximum_closed_pocket_cells)
        || plan
            .anchors
            .values()
            .any(|position| marked.contains(position))
        || !unexpected_dry_access.is_empty()
    {
        issues.push(recipe_issue(format!(
            "Waterfall stitched isolated-terrain projection must exactly tag closed pockets of at \
             most {} cells (marked {}, expected {}, components {:?}, \
             unexpected dry access {:?})",
            seam.maximum_closed_pocket_cells,
            marked.len(),
            expected.len(),
            closed_components
                .iter()
                .map(|component| {
                    let boundary = plan
                        .volume
                        .surfaces
                        .iter()
                        .filter(|(position, _)| {
                            !component.contains(position)
                                && component
                                    .iter()
                                    .any(|member| member.coord.distance(position.coord) == 1)
                        })
                        .map(|(position, metadata)| (*position, metadata.access))
                        .take(16)
                        .collect::<Vec<_>>();
                    (
                        component.len(),
                        component.iter().copied().take(12).collect::<Vec<_>>(),
                        boundary,
                    )
                })
                .take(6)
                .collect::<Vec<_>>(),
            unexpected_dry_access
                .into_iter()
                .take(6)
                .collect::<Vec<_>>(),
        )));
    }
}

fn validate_stitched_network(
    plan: &GeneratedWorldPlan,
    ordinary: &OrdinaryGraph,
    critical: &[Vec<TilePos>; 2],
    issues: &mut Vec<WorldValidationIssue>,
) {
    let required = [PARTY_START, HOSTILE_START, FALL_OVERLOOK, BASIN_OVERLOOK];
    let anchors = required
        .into_iter()
        .filter_map(|name| match plan.anchors.get(name).copied() {
            Some(position) if ordinary.contains(position) => Some((name, position)),
            Some(position) => {
                issues.push(recipe_issue(format!(
                    "Waterfall review anchor {name:?} is not ordinary footing at {position:?}"
                )));
                None
            }
            None => {
                issues.push(recipe_issue(format!(
                    "Waterfall is missing required review anchor {name:?}"
                )));
                None
            }
        })
        .collect::<Vec<_>>();
    let Some((_, party)) = anchors
        .iter()
        .find(|(name, _)| *name == PARTY_START)
        .copied()
    else {
        return;
    };
    let distances = ordinary.distances_from(party);
    let components = ordinary_components(ordinary);
    if components.len() != 1 {
        issues.push(recipe_issue(format!(
            "Waterfall ordinary terrain must form one connected network after seam shaping; \
             found {} components: {:?}",
            components.len(),
            components
                .iter()
                .filter_map(|component| component.first().map(|first| (component.len(), first)))
                .take(6)
                .collect::<Vec<_>>()
        )));
    }
    if anchors
        .iter()
        .any(|(_, position)| !distances.contains_key(position))
    {
        issues.push(recipe_issue(
            "Waterfall stitched review anchors are not mutually reachable",
        ));
    }
    let critical_terminals = critical
        .iter()
        .flat_map(|lane| {
            lane.first()
                .copied()
                .into_iter()
                .chain(lane.last().copied())
        })
        .collect::<BTreeSet<_>>();
    if !critical_terminals.contains(&party)
        || plan
            .anchors
            .get(HOSTILE_START)
            .is_none_or(|hostile| !critical_terminals.contains(hostile))
    {
        issues.push(recipe_issue(
            "Waterfall stitched actor anchors do not use the retained critical bypass landings",
        ));
    }
}

fn ordinary_components(ordinary: &OrdinaryGraph) -> Vec<BTreeSet<TilePos>> {
    let mut remaining = ordinary.positions().collect::<BTreeSet<_>>();
    let mut components = Vec::new();
    while let Some(start) = remaining.first().copied() {
        let component = ordinary
            .distances_from(start)
            .into_keys()
            .filter(|position| remaining.contains(position))
            .collect::<BTreeSet<_>>();
        for position in &component {
            remaining.remove(position);
        }
        components.push(component);
    }
    components
}

fn validate_waterfall(
    plan: &GeneratedWorldPlan,
    vegetation_objects: &LandformVegetationSet,
) -> WorldValidation<WaterfallMetrics> {
    let mut issues = Vec::new();
    let profile = if plan.layout.kind.is_composite() {
        WaterfallElevationProfile::RING7
    } else {
        WaterfallElevationProfile::SINGLE
    };
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
    validate_flow_stages(body, expected_watercourse.as_ref(), profile, &mut issues);
    if calm_nodes < 9 || current_nodes < 3 || rapid_nodes < 3 {
        issues.push(recipe_issue(
            "Waterfall must realize calm inlet/basin, rapid, and current stages",
        ));
    }
    let fall_height = validate_fall(&fall_nodes, profile, &mut issues);
    let mut surfaces_by_coord = BTreeMap::<HexCoord, Vec<(TilePos, SurfaceMetadata)>>::new();
    for (position, metadata) in &plan.volume.surfaces {
        surfaces_by_coord
            .entry(position.coord)
            .or_default()
            .push((*position, *metadata));
    }
    if surfaces_by_coord
        .values()
        .any(|surfaces| surfaces.is_empty() || surfaces.len() > 2)
    {
        issues.push(recipe_issue(
            "Waterfall columns must publish one surface, or one bed plus one bridge deck",
        ));
    }
    validate_liquid_beds(plan, body, &surfaces_by_coord, &mut issues);
    validate_bridge(plan, profile, WaterfallFlowShape::Straight, &mut issues);
    validate_escarpment(plan, profile, &mut issues);

    let bypass = match bypass_tiles(plan.layout.grid_radius, &plan.layout.footprint, profile) {
        Ok(bypass) => bypass,
        Err(mut bypass_issues) => {
            issues.append(&mut bypass_issues);
            [Vec::new(), Vec::new()]
        }
    };
    let secondary_bypass =
        match secondary_bypass_tiles(plan.layout.grid_radius, &plan.layout.footprint, profile) {
            Ok(bypass) => bypass,
            Err(mut bypass_issues) => {
                issues.append(&mut bypass_issues);
                [Vec::new(), Vec::new()]
            }
        };
    let secondary_apron =
        match secondary_slope_apron(plan.layout.grid_radius, &plan.layout.footprint, profile) {
            Ok(apron) => apron,
            Err(mut apron_issues) => {
                issues.append(&mut apron_issues);
                Vec::new()
            }
        };
    let ordinary = OrdinaryGraph::from_volume(&plan.volume, Some(&plan.blockers));
    validate_bypass(
        plan,
        &ordinary,
        &bypass,
        "critical",
        inclusive_span_len(BYPASS_HIGH_X, BYPASS_LOW_X),
        &mut issues,
    );
    validate_bypass(
        plan,
        &ordinary,
        &secondary_bypass,
        "secondary",
        inclusive_span_len(SECONDARY_HIGH_X, SECONDARY_LOW_X),
        &mut issues,
    );
    validate_secondary_apron(plan, &ordinary, &secondary_apron, &mut issues);
    validate_route_redundancy(&ordinary, &bypass, &secondary_bypass, profile, &mut issues);

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
        let reached = distances.as_ref().map_or(0, BTreeMap::len);
        let examples = distances.as_ref().map_or_else(Vec::new, |distances| {
            ordinary
                .positions()
                .filter(|position| !distances.contains_key(position))
                .take(6)
                .collect::<Vec<_>>()
        });
        issues.push(recipe_issue(format!(
            "Waterfall ordinary network reaches {reached}/{} surfaces; disconnected examples: {examples:?}",
            ordinary.len()
        )));
    }

    let bypass_coords: BTreeSet<_> = bypass
        .iter()
        .chain(&secondary_bypass)
        .flatten()
        .chain(&secondary_apron)
        .map(|position| position.coord)
        .chain(
            bridge_tiles(&plan.layout.footprint, profile)
                .into_iter()
                .flatten()
                .map(|position| position.coord),
        )
        .collect();
    let raised_terrain = ordinary
        .positions()
        .filter(|position| {
            if bypass_coords.contains(&position.coord) {
                return false;
            }
            let base = if position.level >= profile.high_land {
                profile.high_land
            } else {
                profile.low_land
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
    let protected_centres = match waterfall_route_centres(
        plan.layout.grid_radius,
        &plan.layout.footprint,
        plan.layout.kind.is_composite(),
        profile,
        WaterfallFlowShape::Straight,
    ) {
        Ok(centres) => centres,
        Err(mut route_issues) => {
            issues.append(&mut route_issues);
            BTreeSet::new()
        }
    };
    let (vegetation, grass_surface_percent) = validate_waterfall_vegetation(
        vegetation_objects,
        &plan.volume,
        &plan.liquids,
        &plan.features,
        &plan.structures,
        &plan.blockers,
        &plan.anchors,
        protected_centres,
        std::iter::empty(),
        &mut issues,
    );

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
        tree_roots: count_u32(vegetation.trees),
        grass_roots: count_u32(vegetation.grass),
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
    profile: WaterfallElevationProfile,
    issues: &mut Vec<WorldValidationIssue>,
) {
    let Some(watercourse) = watercourse else {
        return;
    };
    let actual_coords: BTreeSet<_> = body.nodes.keys().map(|position| position.coord).collect();
    let expected_coords = watercourse.coordinates();
    if actual_coords != expected_coords {
        issues.push(recipe_issue(
            "Waterfall liquid nodes do not exactly cover the edge-to-edge lanes and widened basin",
        ));
    }

    let mut main_coords = BTreeSet::new();
    for (lane_index, lane) in watercourse.main_lanes.iter().enumerate() {
        let feeder_prefix_len = watercourse
            .feeder_prefix_lengths
            .get(lane_index)
            .copied()
            .unwrap_or_default();
        for (index, coord) in lane.iter().copied().enumerate() {
            main_coords.insert(coord);
            let Some(expected) = water_cell_for_lane(lane, index, feeder_prefix_len, profile)
            else {
                issues.push(recipe_issue(format!(
                    "Waterfall lane {lane_index} omitted indexed coordinate {coord:?}"
                )));
                continue;
            };
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
        let position = TilePos::new(*coord, profile.low_water);
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
        let Some(first) = lane.first().copied() else {
            issues.push(recipe_issue("Waterfall has an empty main lane"));
            continue;
        };
        if let Some(last) = lane.last() {
            let terminal = TilePos::new(*last, profile.low_water);
            let has_predecessor = body.nodes.values().any(|node| {
                node.downstream
                    .is_some_and(|downstream| downstream.coord == first)
            });
            if has_predecessor
                || last.neighbors().into_iter().any(|neighbor| {
                    neighbor.y() == last.y()
                        && neighbor.x() > last.x()
                        && expected_coords.contains(&neighbor)
                })
                || !matches!(
                    body.nodes.get(&terminal),
                    Some(LiquidNode {
                        state: LiquidFlowState::Still,
                        downstream: None,
                    })
                )
            {
                issues.push(recipe_issue(format!(
                    "Waterfall lane y={} does not begin at an inlet and terminate as still water",
                    last.y()
                )));
            }
        }
    }
}

fn validate_fall(
    fall_nodes: &[(TilePos, Option<TilePos>)],
    profile: WaterfallElevationProfile,
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
    let expected = profile.high_water.saturating_sub(profile.low_water);
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
    surfaces_by_coord: &BTreeMap<HexCoord, Vec<(TilePos, SurfaceMetadata)>>,
    issues: &mut Vec<WorldValidationIssue>,
) {
    for position in body.nodes.keys() {
        let bed = surfaces_by_coord
            .get(&position.coord)
            .and_then(|surfaces| {
                surfaces
                    .iter()
                    .find(|(_, metadata)| metadata.access == SurfaceAccess::NonStandable)
            })
            .copied();
        if bed.is_none() {
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

fn validate_bridge(
    plan: &GeneratedWorldPlan,
    profile: WaterfallElevationProfile,
    flow_shape: WaterfallFlowShape,
    issues: &mut Vec<WorldValidationIssue>,
) {
    let expected = match bridge_tiles_for_shape(&plan.layout.footprint, profile, flow_shape) {
        Ok(bridge) => bridge,
        Err(mut bridge_issues) => {
            issues.append(&mut bridge_issues);
            return;
        }
    };
    let (first_x, last_x) = bridge_span(flow_shape);
    let bridges: Vec<_> = plan
        .structures
        .by_id
        .values()
        .filter(|structure| structure.kind == StructureKind::Bridge)
        .collect();
    if bridges.len() != 1
        || bridges
            .first()
            .is_none_or(|bridge| bridge.voxels != expected)
    {
        issues.push(recipe_issue(
            "Waterfall must retain one exact two-wide upstream bridge structure",
        ));
    }

    for deck in &expected {
        if plan
            .volume
            .surfaces
            .get(deck)
            .map(|metadata| metadata.access)
            != Some(SurfaceAccess::Ordinary)
        {
            issues.push(recipe_issue(format!(
                "Waterfall bridge deck {deck:?} is not ordinary footing"
            )));
        }
        let Some(column) = plan.volume.columns.get(&deck.coord) else {
            issues.push(recipe_issue(format!(
                "Waterfall bridge deck {deck:?} has no volume column"
            )));
            continue;
        };
        if !column.elements.iter().any(|element| {
            matches!(
                element,
                VolumeElement::Solid(SolidMass {
                    levels,
                    material: SolidMaterialRole::Metal,
                    ..
                }) if *levels == LevelInterval::new(deck.level, deck.level.saturating_add(1))
            )
        }) {
            issues.push(recipe_issue(format!(
                "Waterfall bridge deck {deck:?} is not one metal voxel thick"
            )));
        }
        if deck.coord.y().abs() <= WATER_HALF_WIDTH {
            let clearance_level = deck.level.saturating_sub(1);
            if column.elements.iter().any(|element| {
                let levels = match element {
                    VolumeElement::Solid(mass) => mass.levels,
                    VolumeElement::Fill(fill) => fill.levels,
                };
                levels.bottom <= clearance_level && clearance_level < levels.top
            }) {
                issues.push(recipe_issue(format!(
                    "Waterfall bridge deck {deck:?} does not leave one air level over the river"
                )));
            }
        }
    }

    let ordinary = OrdinaryGraph::from_volume(&plan.volume, None);
    for x in first_x..=last_x {
        let lane: Vec<_> = (-BRIDGE_BANK_Y..=BRIDGE_BANK_Y)
            .map(|y| TilePos::new(HexCoord::from_axial(x, y), profile.bridge_deck))
            .collect();
        if lane
            .windows(2)
            .any(|pair| !matches!(pair, [from, to] if ordinary.admits(*from, *to)))
        {
            issues.push(recipe_issue(format!(
                "Waterfall bridge lane x={x} is not continuously walkable"
            )));
        }
    }

    let ordinary_at = |coord| {
        plan.volume
            .surfaces
            .iter()
            .find_map(|(position, metadata)| {
                (position.coord == coord && metadata.access == SurfaceAccess::Ordinary)
                    .then_some(*position)
            })
    };
    let bank_landing = |deck_y, bank_y| {
        (first_x..=last_x)
            .flat_map(|x| HexCoord::from_axial(x, deck_y).neighbors())
            .filter(|coord| coord.y() == bank_y)
            .find_map(ordinary_at)
    };
    let north = bank_landing(-BRIDGE_BANK_Y, -BRIDGE_BANK_Y - 1);
    let south = bank_landing(BRIDGE_BANK_Y, BRIDGE_BANK_Y + 1);
    match (north, south) {
        (Some(north), Some(south)) => {
            let reachable_without_bridge = ordinary.reachable_avoiding(north, &expected);
            if reachable_without_bridge.contains(&south) {
                issues.push(recipe_issue(
                    "Waterfall water barrier can be crossed without the upper bridge",
                ));
            }
        }
        _ => issues.push(recipe_issue(
            "Waterfall bridge has no ordinary landing on both river banks",
        )),
    }
}

fn validate_bent_bridge_abutment(
    plan: &GeneratedWorldPlan,
    ordinary: &OrdinaryGraph,
    seam: &StitchedSeamContext,
    profile: WaterfallElevationProfile,
    flow_shape: WaterfallFlowShape,
    issues: &mut Vec<WorldValidationIssue>,
) {
    if flow_shape != WaterfallFlowShape::BentNorthWest {
        return;
    }
    let Ok(abutment) = bent_bridge_abutment(&plan.layout.footprint, profile, flow_shape) else {
        issues.push(recipe_issue(
            "Waterfall Ring19 bent bridge cannot resolve its authored abutment",
        ));
        return;
    };
    let Some(first) = abutment.first().copied() else {
        issues.push(recipe_issue(
            "Waterfall Ring19 bent bridge has no authored abutment",
        ));
        return;
    };
    let secondary = TilePos::new(
        HexCoord::from_axial(BENT_BRIDGE_FIRST_X - 2, BRIDGE_BANK_Y + 1),
        secondary_slope_level(BENT_BRIDGE_FIRST_X - 2, profile),
    );
    let deck = TilePos::new(
        HexCoord::from_axial(BENT_BRIDGE_FIRST_X, BRIDGE_BANK_Y),
        profile.bridge_deck,
    );
    let mut chain = vec![deck];
    chain.extend(abutment.into_iter().map(|position| {
        seam.projected_authored
            .get(&position)
            .copied()
            .unwrap_or(position)
    }));
    chain.push(
        seam.projected_authored
            .get(&secondary)
            .copied()
            .unwrap_or(secondary),
    );
    if first.level != profile.high_land
        || chain.iter().any(|position| !ordinary.contains(*position))
        || chain
            .windows(2)
            .any(|pair| !matches!(pair, [from, to] if ordinary.admits(*from, *to)))
    {
        issues.push(recipe_issue(format!(
            "Waterfall Ring19 bent bridge abutment is not the exact ordinary deck-to-secondary \
             chain: {chain:?}"
        )));
    }
}

fn validate_stitched_escarpment(
    plan: &GeneratedWorldPlan,
    seam: &StitchedSeamContext,
    profile: WaterfallElevationProfile,
    flow_shape: WaterfallFlowShape,
    issues: &mut Vec<WorldValidationIssue>,
) {
    let shelves = plan
        .volume
        .surfaces
        .iter()
        .filter_map(|(position, metadata)| {
            (metadata.access == SurfaceAccess::SpecialMovement(CLIFF_SHELF_REGION))
                .then_some(*position)
        })
        .collect::<BTreeSet<_>>();
    let expected_shelves = usize::try_from((plan.layout.grid_radius / 2).clamp(4, 8)).unwrap_or(4);
    if shelves.len() != expected_shelves
        || shelves.iter().any(|position| {
            seam.mid_thresholds
                .get(&position.coord)
                .is_none_or(|threshold| {
                    position.level <= profile.low_land
                        || position.level >= profile.high_land
                        || position.level != *threshold
                })
                || position.coord.x().abs() > CLIFF_MAX_OFFSET
        })
    {
        issues.push(recipe_issue(format!(
            "Waterfall stitched escarpment retains {}/{} valid mid-height shelf cells: {:?}",
            shelves.len(),
            expected_shelves,
            shelves.iter().take(8).collect::<Vec<_>>()
        )));
    }

    let water_coords = plan
        .liquids
        .bodies
        .values()
        .flat_map(|body| body.nodes.keys().map(|position| position.coord))
        .collect::<BTreeSet<_>>();
    let mut protected_coords = bypass_tiles_for_shape(
        plan.layout.grid_radius,
        &plan.layout.footprint,
        profile,
        flow_shape,
    )
    .into_iter()
    .flat_map(|lanes| lanes.into_iter().flatten().map(|position| position.coord))
    .chain(
        secondary_bypass_tiles(plan.layout.grid_radius, &plan.layout.footprint, profile)
            .into_iter()
            .flat_map(|lanes| lanes.into_iter().flatten().map(|position| position.coord)),
    )
    .chain(
        secondary_slope_apron(plan.layout.grid_radius, &plan.layout.footprint, profile)
            .into_iter()
            .flatten()
            .map(|position| position.coord),
    )
    .chain(
        bridge_tiles_for_shape(&plan.layout.footprint, profile, flow_shape)
            .into_iter()
            .flatten()
            .map(|position| position.coord),
    )
    .chain(
        bent_bridge_abutment(&plan.layout.footprint, profile, flow_shape)
            .into_iter()
            .flatten()
            .map(|position| position.coord),
    )
    .chain(
        seam.ring_secondary_flank
            .iter()
            .map(|position| position.coord),
    )
    .collect::<BTreeSet<_>>();
    if !seam.ring_secondary_flank.is_empty() {
        protected_coords.insert(ring_bridge_flank(profile).coord);
    }

    let observed = plan
        .volume
        .surfaces
        .keys()
        .copied()
        .filter(|position| {
            !water_coords.contains(&position.coord)
                && !protected_coords.contains(&position.coord)
                && !shelves.iter().any(|shelf| shelf.coord == position.coord)
        })
        .collect::<Vec<_>>();
    let mut matching_families = Vec::new();
    let mut closest = None::<(usize, usize, bool, Vec<TilePos>)>;
    for reverse in [false, true] {
        for phase in 0..CLIFF_PATTERN.len() {
            let mismatches = observed
                .iter()
                .copied()
                .filter(|position| {
                    let boundary = cliff_boundary_for(position.coord.y(), phase, reverse);
                    let base = if position.coord.x() < boundary {
                        profile.high_land
                    } else {
                        profile.low_land
                    };
                    !(base..=base.saturating_add(2)).any(|authored| {
                        seam.projected_relief_levels
                            .get(&(position.coord, authored))
                            .is_some_and(|projected| *projected == position.level)
                    })
                })
                .collect::<Vec<_>>();
            let shelves_match = shelves.iter().all(|shelf| {
                shelf
                    .coord
                    .x()
                    .abs_diff(cliff_boundary_for(shelf.coord.y(), phase, reverse))
                    <= 1
            });
            if mismatches.is_empty() && shelves_match {
                matching_families.push((phase, reverse));
            }
            let mismatch_count = mismatches.len().saturating_add(usize::from(!shelves_match));
            if closest
                .as_ref()
                .is_none_or(|(count, ..)| mismatch_count < *count)
            {
                closest = Some((
                    mismatch_count,
                    phase,
                    reverse,
                    mismatches.into_iter().take(8).collect(),
                ));
            }
        }
    }
    if observed.len() < expected_shelves.saturating_mul(4) || matching_families.is_empty() {
        issues.push(recipe_issue(format!(
            "Waterfall stitched escarpment does not match the exact authored meander family \
             across {} dry non-route cells; closest candidate: {closest:?}",
            observed.len()
        )));
    }
}

fn cliff_boundary_for(y: i32, phase: usize, reverse: bool) -> i32 {
    if y.abs() <= BASIN_MAX_HALF_WIDTH {
        return FALL_TARGET_X;
    }
    let shifted =
        usize::try_from(y.rem_euclid(i32::try_from(CLIFF_PATTERN.len()).unwrap_or(i32::MAX)))
            .unwrap_or_default();
    let index = if reverse {
        phase
            .saturating_add(CLIFF_PATTERN.len())
            .saturating_sub(shifted % CLIFF_PATTERN.len())
            % CLIFF_PATTERN.len()
    } else {
        phase.saturating_add(shifted) % CLIFF_PATTERN.len()
    };
    CLIFF_PATTERN
        .get(index)
        .copied()
        .unwrap_or_default()
        .clamp(-CLIFF_MAX_OFFSET, CLIFF_MAX_OFFSET)
}

fn validate_escarpment(
    plan: &GeneratedWorldPlan,
    profile: WaterfallElevationProfile,
    issues: &mut Vec<WorldValidationIssue>,
) {
    let shelves: BTreeSet<_> = plan
        .volume
        .surfaces
        .iter()
        .filter_map(|(position, metadata)| {
            (metadata.access == SurfaceAccess::SpecialMovement(CLIFF_SHELF_REGION))
                .then_some(*position)
        })
        .collect();
    let expected_shelves = usize::try_from((plan.layout.grid_radius / 2).clamp(4, 8)).unwrap_or(4);
    if shelves.len() != expected_shelves
        || shelves
            .iter()
            .any(|position| position.level != profile.cliff_mid)
    {
        issues.push(recipe_issue(format!(
            "Waterfall escarpment must retain {expected_shelves} mid-height shelf cells"
        )));
    }

    let water_coords: BTreeSet<_> = plan
        .liquids
        .bodies
        .values()
        .flat_map(|body| body.nodes.keys().map(|position| position.coord))
        .collect();
    let protected_coords: BTreeSet<_> =
        bypass_tiles(plan.layout.grid_radius, &plan.layout.footprint, profile)
            .into_iter()
            .flat_map(|lanes| lanes.into_iter().flatten().map(|position| position.coord))
            .chain(
                secondary_bypass_tiles(plan.layout.grid_radius, &plan.layout.footprint, profile)
                    .into_iter()
                    .flat_map(|lanes| lanes.into_iter().flatten().map(|position| position.coord)),
            )
            .chain(
                secondary_slope_apron(plan.layout.grid_radius, &plan.layout.footprint, profile)
                    .into_iter()
                    .flatten()
                    .map(|position| position.coord),
            )
            .chain(
                bridge_tiles(&plan.layout.footprint, profile)
                    .into_iter()
                    .flatten()
                    .map(|position| position.coord),
            )
            .collect();
    let protected_rows: BTreeSet<_> = protected_coords.iter().map(|coord| coord.y()).collect();
    let mut fronts = BTreeMap::<i32, i32>::new();
    for position in plan.volume.surfaces.keys() {
        if position.coord.y().abs() <= BASIN_MAX_HALF_WIDTH
            || protected_rows.contains(&position.coord.y())
            || water_coords.contains(&position.coord)
            || protected_coords.contains(&position.coord)
        {
            continue;
        }
        if position.level <= profile.cliff_mid {
            fronts
                .entry(position.coord.y())
                .and_modify(|x| *x = (*x).min(position.coord.x()))
                .or_insert(position.coord.x());
        }
    }
    let front_values: Vec<_> = fronts.into_iter().collect();
    let lateral_span = front_values
        .iter()
        .map(|(_, x)| *x)
        .min()
        .zip(front_values.iter().map(|(_, x)| *x).max())
        .map_or(0, |(minimum, maximum)| maximum.saturating_sub(minimum));
    if lateral_span < 3 || lateral_span > CLIFF_MAX_OFFSET.saturating_mul(2) {
        issues.push(recipe_issue(format!(
            "Waterfall escarpment lateral variation must span 3-{}, got {lateral_span}",
            CLIFF_MAX_OFFSET.saturating_mul(2)
        )));
    }
    if front_values.windows(2).any(|pair| {
        matches!(pair, [(first_y, first_x), (second_y, second_x)]
            if second_y.saturating_sub(*first_y) == 1
                && first_x.abs_diff(*second_x) > 2)
    }) {
        issues.push(recipe_issue(
            "Waterfall escarpment moves more than two hexes between adjacent rows",
        ));
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
    let expected = secondary_apron_len();
    if apron.len() != expected {
        issues.push(recipe_issue(format!(
            "Waterfall secondary slope must retain its irregular {expected}-tile apron"
        )));
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
        if !ordinary
            .neighbors(*expected)
            .iter()
            .any(|neighbor| apron_set.contains(neighbor) || neighbor.level == expected.level)
        {
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
    profile: WaterfallElevationProfile,
    issues: &mut Vec<WorldValidationIssue>,
) {
    let routes = [("critical", critical), ("secondary", secondary)];
    let route_tiles =
        routes.map(|(_, route)| route.iter().flatten().copied().collect::<BTreeSet<_>>());

    let overlap: Vec<_> = route_tiles[0]
        .intersection(&route_tiles[1])
        .copied()
        .take(6)
        .collect();
    if !overlap.is_empty() {
        issues.push(recipe_issue(format!(
            "Waterfall high/low routes are not independent; shared surfaces: {overlap:?}"
        )));
    }

    // Each validated lane is itself a concrete graph path. Exact high and low
    // terminals plus disjoint route footprints therefore prove two independent
    // high-to-low routes without inferring topology from authored names.
    for (name, route) in routes {
        for (lane_index, lane) in route.iter().enumerate() {
            let Some((start, goal)) = lane.first().copied().zip(lane.last().copied()) else {
                issues.push(recipe_issue(format!(
                    "Waterfall {name} lane {lane_index} has no high/low terminals"
                )));
                continue;
            };
            if start.level != profile.high_land || goal.level != profile.low_land {
                issues.push(recipe_issue(format!(
                    "Waterfall {name} lane {lane_index} spans levels {} -> {}, expected {} -> {}",
                    start.level, goal.level, profile.high_land, profile.low_land
                )));
            }
            if lane
                .windows(2)
                .any(|pair| !matches!(pair, [from, to] if ordinary.admits(*from, *to)))
            {
                issues.push(recipe_issue(format!(
                    "Waterfall {name} lane {lane_index} is not a complete ordinary high/low path"
                )));
            }
        }
    }
}

fn recipe_issue(detail: impl Into<String>) -> WorldValidationIssue {
    WorldValidationIssue::new(WorldIssueCode::Recipe("waterfall"), detail)
}

fn count_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn inclusive_span_len(start: i32, end: i32) -> usize {
    usize::try_from(end.saturating_sub(start).saturating_add(1)).unwrap_or_default()
}

fn secondary_apron_len() -> usize {
    let broad = SECONDARY_LOW_X
        .saturating_sub(SECONDARY_HIGH_X)
        .saturating_sub(1);
    let shoulder = SECONDARY_LOW_X
        .saturating_sub(SECONDARY_HIGH_X)
        .saturating_sub(5)
        .max(0);
    usize::try_from(broad.saturating_add(shoulder)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::procedural_v3::fingerprint::semantic_plan_fingerprint;
    use crate::procedural_v3::layout::{
        ResolvedBoundaryLiquidOutlet, ResolvedEdgeContract, ResolvedEdgeId, ResolvedEdgeReference,
        ResolvedElevationBand, ResolvedLiquidPort, ResolvedPatch, ResolvedPort,
        ResolvedWalkerPorts,
    };
    use crate::settings::{
        CubeCoord, PatchEdgeContractSettings, PatchEdgesSettings, PatchMaskSettings, PatchSpec,
    };
    use crate::terrain::TerrainPalette;
    use hex_core::{BiomeRegionId, SubstanceId};

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
            sand: SubstanceId::AIR,
            water: WATER,
            metal: METAL,
            worked_stone: SubstanceId(12),
            snow: SNOW,
            ice: ICE,
            basalt: BASALT,
            lava: LAVA,
        }
    }

    fn radius_mask(radius: i32) -> BTreeSet<HexCoord> {
        (-radius..=radius)
            .flat_map(|x| {
                (-radius..=radius).filter_map(move |y| {
                    let coord = HexCoord::from_axial(x, y);
                    (coord.distance(HexCoord::ORIGIN) <= u32::try_from(radius).unwrap_or_default())
                        .then_some(coord)
                })
            })
            .collect()
    }

    fn ring19_slot_mask(slot: usize) -> BTreeSet<HexCoord> {
        let centres = [
            (0, 0),
            (22, -22),
            (22, 0),
            (0, 22),
            (-22, 22),
            (-22, 0),
            (0, -22),
            (44, -44),
            (44, -22),
            (44, 0),
            (22, 22),
            (0, 44),
            (-22, 44),
            (-44, 44),
            (-44, 22),
            (-44, 0),
            (-22, -22),
            (0, -44),
            (22, -44),
        ]
        .map(|(x, y)| HexCoord::from_axial(x, y));
        radius_mask(55)
            .into_iter()
            .filter(|coord| {
                centres
                    .iter()
                    .enumerate()
                    .min_by_key(|(index, centre)| (coord.distance(**centre), *index))
                    .is_some_and(|(index, _)| index == slot)
            })
            .collect()
    }

    fn approach_cells(
        boundary: &BTreeSet<HexCoord>,
        side: HexSide,
        depth: u32,
    ) -> BTreeSet<HexCoord> {
        boundary
            .iter()
            .flat_map(|boundary| {
                let mut coord = *boundary;
                (0..depth).map(move |_| {
                    let current = coord;
                    coord = side.opposite().neighbor(coord);
                    current
                })
            })
            .collect()
    }

    fn resolved_liquid_edge(
        edge_id: ResolvedEdgeId,
        patch_side: HexSide,
        boundary: &BTreeSet<HexCoord>,
        level: Level,
        patch_is_source: bool,
    ) -> ResolvedEdgeContract {
        let patch_id = PatchId(0);
        let other_id = PatchId(edge_id.0.saturating_add(10));
        let outside = boundary
            .iter()
            .map(|inside| patch_side.neighbor(*inside))
            .collect::<BTreeSet<_>>();
        let current_approach = approach_cells(boundary, patch_side, 3);
        let other_approach = outside.clone();
        let (first, second, lanes, first_approach, second_approach, source, sink) =
            if patch_is_source {
                (
                    (patch_id, patch_side),
                    (other_id, patch_side.opposite()),
                    boundary
                        .iter()
                        .map(|inside| (*inside, patch_side.neighbor(*inside)))
                        .collect::<BTreeSet<_>>(),
                    current_approach.clone(),
                    other_approach.clone(),
                    patch_id,
                    other_id,
                )
            } else {
                (
                    (other_id, patch_side.opposite()),
                    (patch_id, patch_side),
                    boundary
                        .iter()
                        .map(|inside| (patch_side.neighbor(*inside), *inside))
                        .collect::<BTreeSet<_>>(),
                    other_approach.clone(),
                    current_approach.clone(),
                    other_id,
                    patch_id,
                )
            };
        let port = ResolvedPort {
            lanes: lanes.clone(),
            first_approach,
            second_approach,
        };
        ResolvedEdgeContract {
            first,
            second,
            elevation: ResolvedElevationBand {
                preferred: level,
                min: level,
                max: level,
            },
            walker: ResolvedWalkerPorts {
                count: 0,
                width: 0,
                ports: Vec::new(),
            },
            liquid: ResolvedLiquidPort::Directed {
                source,
                sink,
                port,
                elevation: ResolvedLiquidElevation::Exact(level),
            },
            approach_depth: 3,
            boundary_pairs: lanes,
            protected_approaches: BTreeMap::from([
                (patch_id, current_approach),
                (other_id, other_approach),
            ]),
        }
    }

    #[derive(Debug, Clone, Copy)]
    enum FixtureOutlet {
        Shared,
        Boundary,
    }

    fn ring19_hydrology_fixture(
        mask: BTreeSet<HexCoord>,
        inlet_side: HexSide,
        inlet_boundary: BTreeSet<HexCoord>,
        inlet_level: Level,
        outlet_side: HexSide,
        outlet_boundary: BTreeSet<HexCoord>,
        outlet_level: Level,
        outlet_kind: FixtureOutlet,
    ) -> ResolvedLayoutPlan {
        let inlet_id = ResolvedEdgeId(1);
        let outlet_id = ResolvedEdgeId(2);
        let inlet = resolved_liquid_edge(inlet_id, inlet_side, &inlet_boundary, inlet_level, false);
        let rotation_turns = match outlet_side {
            HexSide::East => 0,
            HexSide::NorthEast => 1,
            HexSide::NorthWest => 2,
            HexSide::West => 3,
            HexSide::SouthWest => 4,
            HexSide::SouthEast => 5,
        };
        let mut edges = BTreeMap::from_iter(
            HexSide::ALL.map(|side| (side, ResolvedEdgeReference::WorldBoundary)),
        );
        edges.insert(inlet_side, ResolvedEdgeReference::Shared(inlet_id));
        let mut shared_edges = BTreeMap::from([(inlet_id, inlet)]);
        let mut boundary_liquid_outlets = BTreeMap::new();
        match outlet_kind {
            FixtureOutlet::Shared => {
                edges.insert(outlet_side, ResolvedEdgeReference::Shared(outlet_id));
                shared_edges.insert(
                    outlet_id,
                    resolved_liquid_edge(
                        outlet_id,
                        outlet_side,
                        &outlet_boundary,
                        outlet_level,
                        true,
                    ),
                );
            }
            FixtureOutlet::Boundary => {
                boundary_liquid_outlets.insert(
                    (PatchId(0), outlet_side),
                    ResolvedBoundaryLiquidOutlet {
                        source: PatchId(0),
                        side: outlet_side,
                        lanes: outlet_boundary
                            .iter()
                            .map(|inside| (*inside, outlet_side.neighbor(*inside)))
                            .collect(),
                        inward_approach: approach_cells(&outlet_boundary, outlet_side, 3),
                        approach_depth: 3,
                        level: outlet_level,
                    },
                );
            }
        }
        ResolvedLayoutPlan {
            kind: LayoutKind::Ring19,
            grid_radius: 55,
            footprint: mask.clone(),
            patches: BTreeMap::from([(
                PatchId(0),
                ResolvedPatch {
                    biome_region: BiomeRegionId(0),
                    rotation_turns,
                    mask,
                    edges,
                },
            )]),
            shared_edges,
            boundary_liquid_outlets,
        }
    }

    fn local_coords(coords: &[(i32, i32)]) -> BTreeSet<HexCoord> {
        coords
            .iter()
            .map(|(x, y)| HexCoord::from_axial(*x, *y))
            .collect()
    }

    fn slot5_walker_lanes(side: HexSide) -> [BTreeSet<HexCoord>; 2] {
        match side {
            HexSide::East => [
                local_coords(&[(7, 6), (8, 5)]),
                local_coords(&[(13, -6), (13, -5)]),
            ],
            HexSide::SouthEast => [
                local_coords(&[(-1, 11), (0, 10)]),
                local_coords(&[(5, 8), (6, 7)]),
            ],
            HexSide::SouthWest => [
                local_coords(&[(-11, 10), (-11, 11)]),
                local_coords(&[(-8, 13), (-8, 14)]),
            ],
            HexSide::West => [
                local_coords(&[(-11, 0), (-11, 1)]),
                local_coords(&[(-8, -6), (-8, -5)]),
            ],
            HexSide::NorthWest => [
                local_coords(&[(-6, -8), (-5, -8)]),
                local_coords(&[(5, -13), (6, -14)]),
            ],
            HexSide::NorthEast => [
                local_coords(&[(10, -11), (11, -11)]),
                local_coords(&[(13, -8), (14, -8)]),
            ],
        }
    }

    fn ring19_slot5_layout() -> ResolvedLayoutPlan {
        let patch_id = PatchId(5);
        let mask = ring19_slot_mask(5);
        let frame = LocalPatchFrame::resolve_rotated(&mask, LayoutKind::Ring19, 55, 0)
            .expect("slot-5 frame");
        let local_inlet = local_coords(&[(-1, -10), (0, -11), (1, -11)]);
        let local_outlet = local_coords(&[(10, 0), (10, 1), (11, -1)]);
        let to_world = |coords: &BTreeSet<HexCoord>| {
            coords
                .iter()
                .map(|coord| frame.to_world(*coord))
                .collect::<Result<BTreeSet<_>, _>>()
                .expect("slot-5 local coordinates fit world space")
        };
        let inlet = to_world(&local_inlet);
        let outlet = to_world(&local_outlet);
        let mut edges = BTreeMap::new();
        let mut shared_edges = BTreeMap::new();
        for (side_index, side) in HexSide::ALL.into_iter().enumerate() {
            let edge_id =
                ResolvedEdgeId(u32::try_from(side_index).expect("six side indices fit u32"));
            let neighbor_slot = match side {
                HexSide::East => 0,
                HexSide::SouthEast => 4,
                HexSide::SouthWest => 14,
                HexSide::West => 15,
                HexSide::NorthWest => 16,
                HexSide::NorthEast => 6,
            };
            let other_id = PatchId(neighbor_slot);
            let other_mask =
                ring19_slot_mask(usize::try_from(neighbor_slot).expect("Ring19 slot fits usize"));
            edges.insert(side, ResolvedEdgeReference::Shared(edge_id));
            let boundary = mask
                .iter()
                .copied()
                .filter(|inside| other_mask.contains(&side.neighbor(*inside)))
                .collect::<BTreeSet<_>>();
            assert_eq!(boundary.len(), 15, "slot-5 {side:?} boundary");
            let boundary_pairs = boundary
                .iter()
                .map(|inside| (*inside, side.neighbor(*inside)))
                .collect::<BTreeSet<_>>();
            let mut current_protected = BTreeSet::new();
            let walker_ports = slot5_walker_lanes(side)
                .into_iter()
                .map(|local_lanes| {
                    let lanes = to_world(&local_lanes);
                    let first_approach = approach_cells(&lanes, side, 3);
                    current_protected.extend(first_approach.iter().copied());
                    ResolvedPort {
                        lanes: lanes
                            .iter()
                            .map(|inside| (*inside, side.neighbor(*inside)))
                            .collect(),
                        first_approach,
                        second_approach: lanes
                            .iter()
                            .map(|inside| side.neighbor(*inside))
                            .collect(),
                    }
                })
                .collect::<Vec<_>>();
            let liquid = if side == HexSide::NorthWest || side == HexSide::East {
                let (lanes, level, source, sink) = if side == HexSide::NorthWest {
                    (inlet.clone(), 29, other_id, patch_id)
                } else {
                    (outlet.clone(), 16, patch_id, other_id)
                };
                let first_approach = approach_cells(&lanes, side, 3);
                current_protected.extend(first_approach.iter().copied());
                ResolvedLiquidPort::Directed {
                    source,
                    sink,
                    port: ResolvedPort {
                        lanes: lanes
                            .iter()
                            .map(|inside| (*inside, side.neighbor(*inside)))
                            .collect(),
                        first_approach,
                        second_approach: lanes
                            .iter()
                            .map(|inside| side.neighbor(*inside))
                            .collect(),
                    },
                    elevation: ResolvedLiquidElevation::Exact(level),
                }
            } else {
                ResolvedLiquidPort::Dry
            };
            shared_edges.insert(
                edge_id,
                ResolvedEdgeContract {
                    first: (patch_id, side),
                    second: (other_id, side.opposite()),
                    elevation: ResolvedElevationBand {
                        preferred: 17,
                        min: 16,
                        max: 18,
                    },
                    walker: ResolvedWalkerPorts {
                        count: 2,
                        width: 2,
                        ports: walker_ports,
                    },
                    liquid,
                    approach_depth: 3,
                    boundary_pairs,
                    protected_approaches: BTreeMap::from([
                        (patch_id, current_protected),
                        (other_id, BTreeSet::new()),
                    ]),
                },
            );
        }
        ResolvedLayoutPlan {
            kind: LayoutKind::Ring19,
            grid_radius: 55,
            footprint: radius_mask(55),
            patches: BTreeMap::from([(
                patch_id,
                ResolvedPatch {
                    biome_region: BiomeRegionId(5),
                    rotation_turns: 0,
                    mask,
                    edges,
                },
            )]),
            shared_edges,
            boundary_liquid_outlets: BTreeMap::new(),
        }
    }

    fn canonical_straight_ports(
        mask: &BTreeSet<HexCoord>,
    ) -> (BTreeSet<HexCoord>, BTreeSet<HexCoord>) {
        let course = watercourse(mask).expect("canonical fixture watercourse");
        let starts = course
            .main_lanes
            .iter()
            .filter_map(|lane| lane.first().copied())
            .collect();
        let ends = course
            .main_lanes
            .iter()
            .filter_map(|lane| lane.last().copied())
            .collect();
        (starts, ends)
    }

    fn resolve_fixture_course(
        layout: &ResolvedLayoutPlan,
    ) -> (
        WaterfallHydrology,
        WaterfallHydrology,
        Watercourse,
        BTreeSet<HexCoord>,
    ) {
        let patch_id = layout
            .patches
            .keys()
            .next()
            .copied()
            .expect("fixture patch id");
        let patch = PatchRecipeContext::resolve(layout, patch_id).expect("fixture patch");
        let rotation = waterfall_rotation(&patch).expect("fixture rotation");
        let frame = LocalPatchFrame::resolve_rotated(
            patch.mask(),
            patch.layout().kind,
            patch.grid_radius(),
            rotation,
        )
        .expect("fixture frame");
        let local_mask = frame.local_mask(patch.mask()).expect("fixture local mask");
        let hydrology = WaterfallHydrology::resolve(&patch).expect("fixture hydrology");
        let local_hydrology = hydrology.to_local(frame).expect("local fixture hydrology");
        let flow_shape = local_hydrology.flow_shape().expect("fixture flow shape");
        let mut dry = waterfall_feeder_exclusions(
            frame.scale(),
            &local_mask,
            layout.kind == LayoutKind::Ring7,
            local_hydrology.profile,
            flow_shape,
        )
        .expect("fixture dry routes");
        extend_feeder_seam_exclusions(&patch, frame, &local_hydrology, &mut dry)
            .expect("fixture seam exclusions");
        let course = watercourse_for_hydrology(&local_mask, &local_hydrology, flow_shape, &dry)
            .expect("fixture watercourse");
        validate_waterfall_liquid_ports(&local_hydrology, &course).expect("fixture ports match");
        (hydrology, local_hydrology, course, dry)
    }

    fn is_solid(substance: SubstanceId) -> bool {
        !matches!(substance, SubstanceId::AIR | WATER | LAVA)
    }

    #[test]
    fn ring19_straight_waterfall_translates_the_complete_profile_up_three_levels() {
        let mask = radius_mask(12);
        let (starts, ends) = canonical_straight_ports(&mask);
        let layout = ring19_hydrology_fixture(
            mask,
            HexSide::West,
            starts.clone(),
            29,
            HexSide::East,
            ends.clone(),
            16,
            FixtureOutlet::Shared,
        );
        let (hydrology, local, course, _dry) = resolve_fixture_course(&layout);

        assert_eq!(hydrology.rotation(), 0);
        assert_eq!(local.profile, WaterfallElevationProfile::translated(3));
        assert_eq!(local.flow_shape(), Ok(WaterfallFlowShape::Straight));
        assert_eq!(
            course
                .main_lanes
                .iter()
                .filter_map(|lane| lane.first().copied())
                .collect::<BTreeSet<_>>(),
            starts
        );
        assert_eq!(
            course
                .main_lanes
                .iter()
                .filter_map(|lane| lane.last().copied())
                .collect::<BTreeSet<_>>(),
            ends
        );
        let fall = course
            .main_lanes
            .iter()
            .flat_map(|lane| {
                lane.iter().enumerate().filter_map(|(index, coord)| {
                    let cell = water_cell_for_lane(lane, index, 0, local.profile)?;
                    (cell.state == LiquidFlowState::Fall).then_some((*coord, cell))
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(fall.len(), 3);
        assert!(fall.iter().all(|(_, cell)| {
            cell.top.level == 29
                && cell
                    .downstream
                    .is_some_and(|downstream| downstream.level == 16)
        }));
    }

    #[test]
    fn ring19_north_west_inlet_routes_three_disjoint_high_water_feeders() {
        let layout = ring19_slot5_layout();
        let mask = layout
            .patches
            .get(&PatchId(5))
            .expect("slot-5 patch")
            .mask
            .clone();
        assert_eq!(mask.len(), 491);
        let frame = LocalPatchFrame::resolve_rotated(&mask, LayoutKind::Ring19, 55, 0)
            .expect("slot-5 frame");
        assert_eq!(frame.center(), HexCoord::from_axial(-22, 0));
        let local_inlet = BTreeSet::from([
            HexCoord::from_axial(-1, -10),
            HexCoord::from_axial(0, -11),
            HexCoord::from_axial(1, -11),
        ]);
        let local_outlet = BTreeSet::from([
            HexCoord::from_axial(10, 0),
            HexCoord::from_axial(10, 1),
            HexCoord::from_axial(11, -1),
        ]);
        let (_hydrology, local, course, dry) = resolve_fixture_course(&layout);

        assert_eq!(local.flow_shape(), Ok(WaterfallFlowShape::BentNorthWest));
        assert!(course
            .feeder_prefix_lengths
            .iter()
            .all(|length| *length >= 3));
        let lane_sets = course
            .main_lanes
            .iter()
            .map(|lane| lane.iter().copied().collect::<BTreeSet<_>>())
            .collect::<Vec<_>>();
        assert!(lane_sets.iter().enumerate().all(|(index, lane)| {
            lane_sets
                .iter()
                .enumerate()
                .all(|(other_index, other)| index == other_index || lane.is_disjoint(other))
        }));
        assert!(course.main_lanes.iter().all(|lane| {
            lane.windows(2)
                .all(|pair| matches!(pair, [first, second] if first.distance(*second) == 1))
        }));
        assert!(course
            .main_lanes
            .iter()
            .zip(&course.feeder_prefix_lengths)
            .all(|(lane, length)| lane.iter().take(*length).all(|coord| !dry.contains(coord))));
        assert_eq!(
            course
                .main_lanes
                .iter()
                .filter_map(|lane| lane.first().copied())
                .collect::<BTreeSet<_>>(),
            local_inlet
        );
        assert_eq!(
            course
                .main_lanes
                .iter()
                .filter_map(|lane| lane.last().copied())
                .collect::<BTreeSet<_>>(),
            local_outlet
        );
        for (lane_index, lane) in course.main_lanes.iter().enumerate() {
            let feeder_len = course
                .feeder_prefix_lengths
                .get(lane_index)
                .copied()
                .expect("one feeder length per lane");
            let first =
                water_cell_for_lane(lane, 0, feeder_len, local.profile).expect("first feeder cell");
            assert_eq!(first.top.level, 29);
            assert_eq!(first.state, LiquidFlowState::Still);
            for index in 1..feeder_len {
                let feeder = water_cell_for_lane(lane, index, feeder_len, local.profile)
                    .expect("remaining feeder cell");
                assert_eq!(feeder.top.level, 29);
                assert_eq!(feeder.state, LiquidFlowState::Current);
            }
        }
    }

    #[test]
    fn ring19_slot5_builds_and_validates_the_complete_waterfall_fragment() {
        let layout = ring19_slot5_layout();
        let mask = layout
            .patches
            .get(&PatchId(5))
            .expect("slot-5 patch")
            .mask
            .clone();
        let patch = PatchRecipeContext::resolve(&layout, PatchId(5)).expect("slot-5 patch");
        let catalog = crate::procedural_v3::vegetation::tests::runtime_art_catalog();
        let fragment = construct_patch_with_catalog(
            patch,
            &V3WaterfallSettings,
            V3EnvironmentSettings::TemperateGrassland,
            0.4,
            PatchBuildMode::Candidate {
                world_seed: 1592598566,
                candidate: 0,
            },
            catalog,
        )
        .expect("slot-5 Waterfall fragment");

        assert_eq!(fragment.volume.mask, mask);
        assert_eq!(
            waterfall_seam_vegetation_reservations(&patch),
            patch.protected_approaches()
        );
        assert_eq!(
            fragment
                .features
                .by_id
                .values()
                .filter(|feature| feature.kind == super::super::world::FeatureKind::Tree)
                .count(),
            2
        );
        match validate_patch(patch, &fragment, catalog) {
            WorldValidation::Valid(()) => {}
            WorldValidation::Invalid(issues) => {
                panic!("slot-5 Waterfall validation failed: {issues:?}");
            }
        }
    }

    #[test]
    fn ring19_boundary_waterfall_translates_down_ten_without_outside_nodes() {
        let mask = radius_mask(12);
        let (local_starts, local_ends) = canonical_straight_ports(&mask);
        let frame = LocalPatchFrame::resolve_rotated(&mask, LayoutKind::Ring19, 55, 5)
            .expect("rotated fixture frame");
        let world_starts = local_starts
            .iter()
            .map(|coord| frame.to_world(*coord))
            .collect::<Result<BTreeSet<_>, _>>()
            .expect("world inlet");
        let world_ends = local_ends
            .iter()
            .map(|coord| frame.to_world(*coord))
            .collect::<Result<BTreeSet<_>, _>>()
            .expect("world outlet");
        let layout = ring19_hydrology_fixture(
            mask,
            HexSide::NorthWest,
            world_starts,
            16,
            HexSide::SouthEast,
            world_ends,
            3,
            FixtureOutlet::Boundary,
        );
        let (hydrology, local, course, _dry) = resolve_fixture_course(&layout);

        assert_eq!(hydrology.rotation(), 5);
        assert_eq!(local.profile, WaterfallElevationProfile::translated(-10));
        assert_eq!(local.flow_shape(), Ok(WaterfallFlowShape::Straight));
        assert_eq!(
            course
                .main_lanes
                .iter()
                .filter_map(|lane| lane.first().copied())
                .collect::<BTreeSet<_>>(),
            local_starts
        );
        assert_eq!(
            course
                .main_lanes
                .iter()
                .filter_map(|lane| lane.last().copied())
                .collect::<BTreeSet<_>>(),
            local_ends
        );
        for lane in &course.main_lanes {
            let terminal_index = lane.len().saturating_sub(1);
            let terminal = water_cell_for_lane(lane, terminal_index, 0, local.profile)
                .expect("boundary terminal");
            assert_eq!(terminal.top.level, 3);
            assert_eq!(terminal.state, LiquidFlowState::Still);
            assert_eq!(terminal.downstream, None);
            assert!(layout.patches.get(&PatchId(0)).is_some_and(|patch| {
                patch.mask.contains(
                    &frame
                        .to_world(terminal.top.coord)
                        .expect("terminal world coordinate"),
                )
            }));
        }
    }

    #[test]
    fn ring19_waterfall_rejects_nonuniform_level_translation() {
        let mask = radius_mask(12);
        let (starts, ends) = canonical_straight_ports(&mask);
        let layout = ring19_hydrology_fixture(
            mask,
            HexSide::West,
            starts,
            29,
            HexSide::East,
            ends,
            15,
            FixtureOutlet::Shared,
        );
        let patch = PatchRecipeContext::resolve(&layout, PatchId(0)).expect("fixture patch");
        let issues = WaterfallHydrology::resolve(&patch).expect_err("mismatched profile must fail");
        assert!(issues
            .iter()
            .any(|issue| issue.detail.contains("not one complete translation")));
    }

    #[test]
    fn ring19_waterfall_rejects_a_translated_bridge_above_the_volume_limit() {
        let mask = radius_mask(12);
        let (starts, ends) = canonical_straight_ports(&mask);
        let layout = ring19_hydrology_fixture(
            mask,
            HexSide::West,
            starts,
            MAX_V3_LEVEL - 1,
            HexSide::East,
            ends,
            MAX_V3_LEVEL - 14,
            FixtureOutlet::Shared,
        );
        let patch = PatchRecipeContext::resolve(&layout, PatchId(0)).expect("fixture patch");
        let issues =
            WaterfallHydrology::resolve(&patch).expect_err("out-of-volume profile must fail");
        assert!(issues
            .iter()
            .any(|issue| issue.detail.contains("bridge deck exceeds level")));
    }

    #[test]
    fn fixed_corpus_builds_valid_waterfalls_at_supported_radii() {
        for radius in [12, 20, 40] {
            for seed in [0, 1, 808, 4_294_967_311] {
                let selected =
                    generate(radius, 0.4, &settings(), seed).expect("Waterfall should generate");
                assert!(!selected.used_fallback);
                assert_eq!(selected.metrics.fall_nodes, 3);
                assert_eq!(selected.metrics.fall_height, 13);
                assert_eq!(selected.metrics.bypass_steps, 11);
                assert_eq!(selected.metrics.tree_roots, 3);
                assert!(selected.metrics.grass_roots > 0);
                assert!((15..=25).contains(&selected.metrics.grass_surface_percent));
                assert_eq!(selected.validated.plan.validate(), Vec::new());
            }
        }
    }

    #[test]
    fn radius_12_pr_corpus_validates_128_waterfall_seeds_and_named_regressions() {
        let mut seeds: BTreeSet<u64> = (0..128).collect();
        seeds.extend([808, 4_294_967_311]);
        let mut fallbacks = 0_usize;

        for &seed in &seeds {
            let selected = generate(12, 0.4, &settings(), seed)
                .unwrap_or_else(|error| panic!("radius-12 Waterfall seed {seed}: {error}"));
            fallbacks += usize::from(selected.used_fallback);
        }

        assert!(
            fallbacks.saturating_mul(100) < seeds.len(),
            "{fallbacks}/{} radius-12 Waterfall seeds used fallback",
            seeds.len()
        );
    }

    #[test]
    fn authored_flow_contains_every_required_stage_and_exact_three_wide_fall() {
        let selected = generate(12, 0.4, &settings(), 77).expect("Waterfall should generate");
        let metrics = &selected.metrics;

        assert!(metrics.calm_nodes >= 9);
        assert!(metrics.current_nodes >= 3);
        assert!(metrics.rapid_nodes >= 3);
        assert_eq!(metrics.fall_nodes, 3);
        assert_eq!(metrics.fall_height, 13);
        assert!(metrics.water_nodes > 60);
    }

    #[test]
    fn inlet_basin_and_boundary_outlet_have_exact_geometry() {
        let selected = generate(12, 0.4, &settings(), 77).expect("Waterfall should generate");
        let plan = &selected.validated.plan;
        let course = watercourse(&plan.layout.footprint).expect("fixed watercourse");
        for (x, expected_width) in [(0, 7), (1, 7), (2, 7), (3, 7), (4, 7), (5, 5), (6, 5)] {
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
            let first = lane.first().copied().expect("every lane has an inlet");
            let last = lane.last().copied().expect("every lane has an outlet");
            assert!(
                !plan.layout.footprint.contains(&HexCoord::from_axial(
                    first.x().saturating_sub(1),
                    first.y(),
                )),
                "each high-water lane must begin on the west boundary"
            );
            assert!(
                !plan
                    .layout
                    .footprint
                    .contains(&HexCoord::from_axial(last.x().saturating_add(1), last.y(),)),
                "each low-water lane must terminate on the resolved east boundary"
            );
        }
    }

    #[test]
    fn upstream_bridge_is_two_wide_and_the_only_ordinary_bank_crossing() {
        let selected = generate(12, 0.4, &settings(), 77).expect("Waterfall should generate");
        let plan = &selected.validated.plan;
        let expected = bridge_tiles(&plan.layout.footprint, WaterfallElevationProfile::SINGLE)
            .expect("fixed upper bridge");
        let structure = plan
            .structures
            .by_id
            .values()
            .find(|structure| structure.kind == StructureKind::Bridge)
            .expect("Waterfall should publish a bridge structure");
        assert_eq!(structure.voxels, expected);
        assert_eq!(expected.len(), 14);

        let ordinary = OrdinaryGraph::from_volume(&plan.volume, None);
        for x in BRIDGE_FIRST_X..=BRIDGE_LAST_X {
            let lane: Vec<_> = (-BRIDGE_BANK_Y..=BRIDGE_BANK_Y)
                .map(|y| TilePos::new(HexCoord::from_axial(x, y), BRIDGE_DECK_LEVEL))
                .collect();
            assert!(lane
                .windows(2)
                .all(|pair| matches!(pair, [from, to] if ordinary.admits(*from, *to))));
        }
        let ordinary_at = |coord| {
            plan.volume
                .surfaces
                .iter()
                .find_map(|(position, metadata)| {
                    (position.coord == coord && metadata.access == SurfaceAccess::Ordinary)
                        .then_some(*position)
                })
                .expect("bridge bank should retain ordinary footing")
        };
        let first_bank = ordinary_at(HexCoord::from_axial(BRIDGE_FIRST_X, -BRIDGE_BANK_Y - 1));
        let second_bank = ordinary_at(HexCoord::from_axial(BRIDGE_FIRST_X, BRIDGE_BANK_Y + 1));
        assert!(
            !ordinary
                .reachable_avoiding(first_bank, &expected)
                .contains(&second_bank),
            "edge-to-edge water should make the bridge the only ordinary bank crossing"
        );
    }

    #[test]
    fn bypass_is_two_wide_climbable_and_connects_every_ordinary_surface() {
        let selected = generate(12, 0.4, &settings(), 91).expect("Waterfall should generate");
        let plan = &selected.validated.plan;
        let bypass = bypass_tiles(
            12,
            &plan.layout.footprint,
            WaterfallElevationProfile::SINGLE,
        )
        .expect("fixed bypass");
        let secondary = secondary_bypass_tiles(
            12,
            &plan.layout.footprint,
            WaterfallElevationProfile::SINGLE,
        )
        .expect("secondary bypass");
        let ordinary = OrdinaryGraph::from_volume(&plan.volume, None);

        for (route, expected_length) in [
            (&bypass, inclusive_span_len(BYPASS_HIGH_X, BYPASS_LOW_X)),
            (
                &secondary,
                inclusive_span_len(SECONDARY_HIGH_X, SECONDARY_LOW_X),
            ),
        ] {
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
                    HIGH_LAND_LEVEL.saturating_sub(LOW_LAND_LEVEL)
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
        let apron = secondary_slope_apron(
            12,
            &plan.layout.footprint,
            WaterfallElevationProfile::SINGLE,
        )
        .expect("slope apron");
        assert_eq!(apron.len(), secondary_apron_len());
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
    fn route_redundancy_rejects_shared_and_false_high_low_paths() {
        let selected = generate(12, 0.4, &settings(), 91).expect("Waterfall should generate");
        let plan = &selected.validated.plan;
        let critical = bypass_tiles(
            12,
            &plan.layout.footprint,
            WaterfallElevationProfile::SINGLE,
        )
        .expect("critical bypass");
        let secondary = secondary_bypass_tiles(
            12,
            &plan.layout.footprint,
            WaterfallElevationProfile::SINGLE,
        )
        .expect("secondary bypass");
        let ordinary = OrdinaryGraph::from_volume(&plan.volume, None);

        let mut shared = secondary.clone();
        let critical_start = critical
            .first()
            .and_then(|lane| lane.first())
            .copied()
            .expect("critical high terminal");
        let shared_start = shared
            .first_mut()
            .and_then(|lane| lane.first_mut())
            .expect("secondary high terminal");
        *shared_start = critical_start;
        let mut issues = Vec::new();
        validate_route_redundancy(
            &ordinary,
            &critical,
            &shared,
            WaterfallElevationProfile::SINGLE,
            &mut issues,
        );
        assert!(issues
            .iter()
            .any(|issue| issue.detail.contains("high/low routes are not independent")));

        let mut false_terminal = secondary;
        let terminal = false_terminal
            .first_mut()
            .and_then(|lane| lane.first_mut())
            .expect("secondary high terminal");
        terminal.level = terminal.level.saturating_sub(1);
        let mut issues = Vec::new();
        validate_route_redundancy(
            &ordinary,
            &critical,
            &false_terminal,
            WaterfallElevationProfile::SINGLE,
            &mut issues,
        );
        assert!(issues
            .iter()
            .any(|issue| issue.detail.contains("expected 27 -> 16")));
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
                .find(|(surface, metadata)| {
                    surface.coord == node.coord && metadata.access == SurfaceAccess::NonStandable
                })
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
            let critical =
                bypass_tiles(radius, &layout.footprint, WaterfallElevationProfile::SINGLE)
                    .expect("critical bypass");
            let secondary = secondary_bypass_tiles(
                radius,
                &layout.footprint,
                WaterfallElevationProfile::SINGLE,
            )
            .expect("secondary bypass");
            let apron =
                secondary_slope_apron(radius, &layout.footprint, WaterfallElevationProfile::SINGLE)
                    .expect("secondary apron");
            let bridge = bridge_tiles(&layout.footprint, WaterfallElevationProfile::SINGLE)
                .expect("upper bridge");
            let mut bypass: BTreeMap<_, _> = critical
                .iter()
                .chain(&secondary)
                .flatten()
                .chain(&apron)
                .map(|position| (position.coord, position.level))
                .collect();
            bypass.extend(
                bridge
                    .iter()
                    .map(|position| (position.coord, HIGH_LAND_LEVEL)),
            );
            let streams = SeedStreams::new(912_441, 3, PatchId(0).0);
            let escarpment = EscarpmentPlan::new(
                radius,
                &layout.footprint,
                &course.coordinates(),
                &bypass,
                &BTreeSet::new(),
                WaterfallElevationProfile::SINGLE,
                Some(streams.stage("waterfall.cliff")),
            )
            .expect("cliff should fit");
            let relief = ReliefPlan::new(
                radius,
                &layout.footprint,
                &course.coordinates(),
                &bypass,
                &escarpment,
                streams.stage("waterfall.relief"),
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
    fn cliff_front_meanders_and_retains_mid_height_shelves() {
        let layout = resolve_layout(12, &settings()).expect("test layout should resolve");
        let course = watercourse(&layout.footprint).expect("test watercourse");
        let critical = bypass_tiles(12, &layout.footprint, WaterfallElevationProfile::SINGLE)
            .expect("critical bypass");
        let secondary =
            secondary_bypass_tiles(12, &layout.footprint, WaterfallElevationProfile::SINGLE)
                .expect("secondary bypass");
        let apron = secondary_slope_apron(12, &layout.footprint, WaterfallElevationProfile::SINGLE)
            .expect("secondary apron");
        let bridge = bridge_tiles(&layout.footprint, WaterfallElevationProfile::SINGLE)
            .expect("upper bridge");
        let mut protected: BTreeMap<_, _> = critical
            .iter()
            .chain(&secondary)
            .flatten()
            .chain(&apron)
            .map(|position| (position.coord, position.level))
            .collect();
        protected.extend(
            bridge
                .iter()
                .map(|position| (position.coord, HIGH_LAND_LEVEL)),
        );
        let cliff = EscarpmentPlan::new(
            12,
            &layout.footprint,
            &course.coordinates(),
            &protected,
            &BTreeSet::new(),
            WaterfallElevationProfile::SINGLE,
            Some(SeedStreams::new(77, 2, PatchId(0).0).stage("waterfall.cliff")),
        )
        .expect("cliff should fit");

        let offsets: BTreeSet<_> = cliff.boundary_by_y.values().copied().collect();
        assert!(offsets.contains(&-CLIFF_MAX_OFFSET));
        assert!(offsets.contains(&CLIFF_MAX_OFFSET));
        assert!(cliff
            .boundary_by_y
            .values()
            .all(|offset| offset.abs() <= CLIFF_MAX_OFFSET));
        assert_eq!(cliff.shelves.len(), 6);
        assert!(cliff
            .shelves
            .iter()
            .all(|coord| !course.coordinates().contains(coord)));
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
                vegetation: LandformVegetationSet::resolve(
                    super::super::vegetation::tests::runtime_art_catalog(),
                    V3EnvironmentSettings::TemperateGrassland,
                    "Waterfall",
                )
                .expect("tracked Waterfall vegetation should resolve"),
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
        assert_eq!(selected.metrics.fall_height, 13);
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
            let warmup = super::super::build(
                radius,
                0.4,
                &settings(),
                u64::MAX,
                &palette,
                &is_solid,
                None,
            )
            .expect("warm-up Waterfall should build");
            std::hint::black_box(warmup);

            let mut samples = Vec::new();
            for seed in 0..12 {
                let started = std::time::Instant::now();
                let build =
                    super::super::build(radius, 0.4, &settings(), seed, &palette, &is_solid, None)
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
            eprintln!(
                "V3 Waterfall full build radius {radius}: median={median:?} p95={p95:?} \
                 target={budget:?} (trend only)"
            );
        }
    }
}
