//! Pure semantic Waterfall recipe for procedural generator V3.
//!
//! Water topology is authored before terrain. The resulting solid volume is fitted
//! around one three-wide watercourse and a separate two-wide ordinary-walker bypass.
//! Rendering and ECS publication remain downstream of this module.

use std::collections::{BTreeMap, BTreeSet};

use hex_assets::RuntimeArtCatalog;
use hex_core::{HexCoord, Level, MapViewHint, SpecialMovementRegion, TilePos};

use super::composition::{compose_single_patch, GeneratedPatchPlan};
use super::layout::{resolve_layout, PatchId, ResolvedLayoutPlan};
use super::liquid::{LiquidBodyId, LiquidBodyPlan, LiquidFlowState, LiquidNode, LiquidPlan};
use super::local_frame::LocalPatchFrame;
use super::patch::{PatchBuildMode, PatchRecipeContext};
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
    append_landform_vegetation, landform_vegetation_metrics, LandformVegetationMetrics,
    LandformVegetationSet,
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
    ProceduralV3Settings, V3EnvironmentSettings, V3LayoutSettings, V3RecipeSettings,
    V3WaterfallSettings,
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
const SECONDARY_HIGH_X: i32 = -7;
const SECONDARY_LOW_X: i32 = 6;
const BRIDGE_FIRST_X: i32 = -7;
const BRIDGE_LAST_X: i32 = -6;
const BRIDGE_BANK_Y: i32 = 3;
const BRIDGE_DECK_LEVEL: i32 = HIGH_LAND_LEVEL + 1;
const RING_BRIDGE_FLANK: TilePos = TilePos::new(HexCoord::from_axial(-8, 2), HIGH_LAND_LEVEL);
const CLIFF_MID_LEVEL: i32 = LOW_LAND_LEVEL + (HIGH_LAND_LEVEL - LOW_LAND_LEVEL) / 2;
const CLIFF_MAX_OFFSET: i32 = 2;
const CLIFF_PATTERN: [i32; 12] = [-2, -2, -1, 0, 1, 2, 2, 1, 0, -1, -2, -2];
const CLIFF_SHELF_REGION: SpecialMovementRegion = SpecialMovementRegion(0);
const RING_ISOLATED_TERRAIN_REGION: SpecialMovementRegion = SpecialMovementRegion(1);
const MAX_RING_CLOSED_POCKET_CELLS: usize = 3;
const RELIEF_CENTERS_PER_BANK: u64 = 3;
const WATERFALL_TREE_TARGET: usize = 3;
const WATERFALL_GRASS_PERCENT: usize = 20;
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
    bridge_tiles(&layout.footprint).map_err(recipe_issues_to_error)?;
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
        V3RecipeSettings::Volcano(_) => "Volcano",
        V3RecipeSettings::DeepForest(_) => "DeepForest",
        V3RecipeSettings::Prairie(_) => "Prairie",
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
    let rotation = waterfall_rotation(&patch)?;
    let frame = LocalPatchFrame::resolve_rotated(
        patch.mask(),
        patch.layout().kind,
        patch.grid_radius(),
        rotation,
    )
    .map_err(|error| vec![recipe_issue(error)])?;
    let mask = frame
        .local_mask(patch.mask())
        .map_err(|error| vec![recipe_issue(error)])?;
    let patch_radius = frame.scale();
    let biome_region = patch.biome_region();
    let streams = mode.seed_streams(&patch).map(|streams| WaterfallStreams {
        relief: streams.stage("waterfall.relief"),
        cliff: streams.stage("waterfall.cliff"),
        trees: streams.stage("waterfall.vegetation.trees"),
        grass: streams.stage("waterfall.vegetation.grass"),
    });
    let watercourse = watercourse(&mask)?;
    let bypass = bypass_tiles(patch_radius, &mask)?;
    let secondary_bypass = secondary_bypass_tiles(patch_radius, &mask)?;
    let secondary_apron = secondary_slope_apron(patch_radius, &mask)?;
    let composite_layout = patch.layout().kind.is_composite();
    let low_water_level = if composite_layout {
        COMPOSITE_LOW_WATER_LEVEL
    } else {
        LOW_WATER_LEVEL
    };
    validate_waterfall_liquid_ports(&patch, frame, &watercourse, low_water_level)?;
    let ring_secondary_flank = if composite_layout {
        ring_secondary_flank_apron(patch_radius, &mask)?
    } else {
        Vec::new()
    };
    let bridge = bridge_tiles(&mask)?;
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
        .map(|position| (position.coord, position.level))
        .collect();
    protected_by_coord.extend(
        bridge
            .iter()
            .map(|position| (position.coord, HIGH_LAND_LEVEL)),
    );
    let mut restored_surface_levels: BTreeMap<_, _> = bypass
        .iter()
        .chain(&secondary_bypass)
        .flatten()
        .chain(&secondary_apron)
        .chain(&ring_secondary_flank)
        .map(|position| (position.coord, position.level))
        .collect();
    restored_surface_levels.extend(
        bridge
            .iter()
            .map(|position| (position.coord, HIGH_LAND_LEVEL)),
    );
    for edge in patch.shared_edges() {
        for coord in edge.protected_approaches() {
            let local = frame.to_local(*coord).map_err(|error| {
                vec![recipe_issue(format!(
                    "Waterfall seam approach conversion failed: {error}"
                ))]
            })?;
            protected_by_coord.insert(local, edge.preferred_level());
        }
    }
    if composite_layout {
        if !mask.contains(&RING_BRIDGE_FLANK.coord) {
            return Err(vec![recipe_issue(
                "Waterfall Ring7 patch cannot fit the bridge-flank landing",
            )]);
        }
        protected_by_coord.insert(RING_BRIDGE_FLANK.coord, RING_BRIDGE_FLANK.level);
        restored_surface_levels.insert(RING_BRIDGE_FLANK.coord, RING_BRIDGE_FLANK.level);
    }
    let mut seam_excluded_shelves = mask
        .iter()
        .copied()
        .filter_map(|coord| {
            let surface = TilePos::new(coord, CLIFF_MID_LEVEL);
            match project_surface_through_walker_seams(&patch, frame, surface) {
                Ok(projected)
                    if projected.level <= LOW_LAND_LEVEL || projected.level >= HIGH_LAND_LEVEL =>
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
    for lane in &watercourse.main_lanes {
        for (index, coord) in lane.iter().copied().enumerate() {
            let next = lane.get(index.saturating_add(1)).copied();
            let cell = water_cell(coord, next, low_water_level);
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
    let view_hint = waterfall_view_hint(patch_radius, level_height)?;

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
        .map(|position| {
            frame.to_world(position.coord).map_err(|error| {
                vec![recipe_issue(format!(
                    "Waterfall vegetation route conversion failed: {error}"
                ))]
            })
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    if composite_layout {
        route_coords.insert(frame.to_world(RING_BRIDGE_FLANK.coord).map_err(|error| {
            vec![recipe_issue(format!(
                "Waterfall vegetation bridge-flank conversion failed: {error}"
            ))]
        })?);
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
        .chain(patch.protected_approaches())
    {
        vegetation_reserved.extend(coord.within_radius(2));
    }
    let eligible_dry = ordinary_surfaces
        .keys()
        .filter(|coord| !vegetation_reserved.contains(coord))
        .copied()
        .collect::<BTreeSet<_>>();
    let grass_target = eligible_dry.len().saturating_mul(WATERFALL_GRASS_PERCENT) / 100;
    append_landform_vegetation(
        "Waterfall",
        vegetation,
        &ordinary_surfaces,
        &eligible_dry,
        &eligible_dry,
        &vegetation_reserved,
        WATERFALL_TREE_TARGET,
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

fn waterfall_rotation(patch: &PatchRecipeContext<'_>) -> Result<u8, Vec<WorldValidationIssue>> {
    if patch.layout().kind == super::layout::LayoutKind::Single {
        return Ok(0);
    }
    let outlets = patch
        .shared_edges()
        .filter_map(|edge| {
            edge.liquid_port()
                .and_then(|(is_source, _)| is_source.then_some(edge.side))
        })
        .collect::<Vec<_>>();
    let [outlet] = outlets.as_slice() else {
        return Err(vec![recipe_issue(
            "Composite Waterfall requires exactly one directed liquid outlet",
        )]);
    };
    Ok(match outlet {
        super::layout::HexSide::East => 0,
        super::layout::HexSide::NorthEast => 1,
        super::layout::HexSide::NorthWest => 2,
        super::layout::HexSide::West => 3,
        super::layout::HexSide::SouthWest => 4,
        super::layout::HexSide::SouthEast => 5,
    })
}

fn validate_waterfall_liquid_ports(
    patch: &PatchRecipeContext<'_>,
    frame: LocalPatchFrame,
    watercourse: &Watercourse,
    low_water_level: Level,
) -> Result<(), Vec<WorldValidationIssue>> {
    if patch.layout().kind == super::layout::LayoutKind::Single {
        return Ok(());
    }
    let mut incoming = Vec::new();
    let mut outgoing = Vec::new();
    for edge in patch.shared_edges() {
        let Some((is_source, port)) = edge.liquid_port() else {
            continue;
        };
        let boundary = port
            .lanes
            .iter()
            .map(|(local, _)| frame.to_local(*local))
            .collect::<Result<BTreeSet<_>, _>>()
            .map_err(|error| {
                vec![recipe_issue(format!(
                    "Waterfall liquid port conversion failed: {error}"
                ))]
            })?;
        let contract = (
            boundary,
            edge.contract.elevation.min,
            edge.contract.elevation.max,
        );
        if is_source {
            outgoing.push(contract);
        } else {
            incoming.push(contract);
        }
    }
    let [outlet] = outgoing.as_slice() else {
        return Err(vec![recipe_issue(format!(
            "Composite Waterfall has {} directed liquid outlets; expected one",
            outgoing.len()
        ))]);
    };
    if incoming.len() > 1 {
        return Err(vec![recipe_issue(format!(
            "Composite Waterfall has {} directed liquid inlets; expected at most one",
            incoming.len()
        ))]);
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
    if outlet.0 != ends {
        return Err(vec![recipe_issue(
            "Composite Waterfall outlet does not exactly match all three downstream water lanes",
        )]);
    }
    if !(outlet.1..=outlet.2).contains(&low_water_level) {
        return Err(vec![recipe_issue(format!(
            "Composite Waterfall outlet level {low_water_level} leaves its declared elevation band {}..={}",
            outlet.1, outlet.2
        ))]);
    }
    if let Some(inlet) = incoming.first() {
        if inlet.0 != starts {
            return Err(vec![recipe_issue(
                "Composite Waterfall inlet does not exactly match all three upstream water lanes",
            )]);
        }
        if !(inlet.1..=inlet.2).contains(&HIGH_WATER_LEVEL) {
            return Err(vec![recipe_issue(format!(
                "Composite Waterfall inlet level {HIGH_WATER_LEVEL} leaves its declared elevation band {}..={}",
                inlet.1, inlet.2
            ))]);
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
            || component.len() > MAX_RING_CLOSED_POCKET_CELLS
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
) -> Result<MapViewHint, Vec<WorldValidationIssue>> {
    let radius = u16::try_from(grid_radius).map_err(|error| {
        vec![recipe_issue(format!(
            "Waterfall radius is too large: {error}"
        ))]
    })?;
    let focus_height = 21.5 * level_height;
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

fn water_cell(coord: HexCoord, next: Option<HexCoord>, low_water_level: Level) -> WaterCell {
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
            low_water_level - 1,
            low_water_level,
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
        (low_water_level - 1, low_water_level, low_water_level, state)
    };
    let top = TilePos::new(coord, top_level);
    let downstream = next.map(|next_coord| {
        let next_level = if next_coord.x() < FALL_TARGET_X {
            HIGH_WATER_LEVEL
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
    Ok(Watercourse { main_lanes, basin })
}

fn bridge_tiles(mask: &BTreeSet<HexCoord>) -> Result<BTreeSet<TilePos>, Vec<WorldValidationIssue>> {
    let bridge: BTreeSet<_> = (BRIDGE_FIRST_X..=BRIDGE_LAST_X)
        .flat_map(|x| {
            (-BRIDGE_BANK_Y..=BRIDGE_BANK_Y)
                .map(move |y| TilePos::new(HexCoord::from_axial(x, y), BRIDGE_DECK_LEVEL))
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

fn ring_secondary_flank_apron(
    grid_radius: u32,
    mask: &BTreeSet<HexCoord>,
) -> Result<Vec<TilePos>, Vec<WorldValidationIssue>> {
    let radius = i32::try_from(grid_radius).unwrap_or(i32::MAX);
    let lane_y = (radius / 3 + 1).min(6).saturating_sub(2);
    let apron = ((SECONDARY_HIGH_X + 2)..=(FALL_SOURCE_X - 1))
        .map(|x| {
            TilePos::new(
                HexCoord::from_axial(x, lane_y),
                secondary_slope_level(x).saturating_sub(1),
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
        return CLIFF_MID_LEVEL;
    }
    let base = escarpment.base_level(coord);
    base + relief.map_or(0, |relief| relief.height_at(coord))
}

#[derive(Debug)]
struct EscarpmentPlan {
    boundary_by_y: BTreeMap<i32, i32>,
    shelves: BTreeSet<HexCoord>,
}

impl EscarpmentPlan {
    fn new(
        grid_radius: u32,
        mask: &BTreeSet<HexCoord>,
        water: &BTreeSet<HexCoord>,
        protected: &BTreeMap<HexCoord, i32>,
        excluded_shelves: &BTreeSet<HexCoord>,
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
            HIGH_LAND_LEVEL
        } else {
            LOW_LAND_LEVEL
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
) -> Result<BTreeSet<HexCoord>, Vec<WorldValidationIssue>> {
    let bypass = bypass_tiles(grid_radius, mask)?;
    let secondary = secondary_bypass_tiles(grid_radius, mask)?;
    let apron = secondary_slope_apron(grid_radius, mask)?;
    let mut centres = bypass
        .iter()
        .chain(&secondary)
        .flatten()
        .chain(&apron)
        .map(|position| position.coord)
        .collect::<BTreeSet<_>>();
    if include_ring_landings {
        centres.extend(
            ring_secondary_flank_apron(grid_radius, mask)?
                .into_iter()
                .map(|position| position.coord),
        );
        centres.insert(RING_BRIDGE_FLANK.coord);
    }
    Ok(centres)
}

fn validate_waterfall_vegetation(
    volume: &VolumePlan,
    liquids: &LiquidPlan,
    features: &FeaturePlan,
    structures: &StructurePlan,
    blockers: &BTreeSet<TilePos>,
    anchors: &BTreeMap<String, TilePos>,
    protected_centres: impl IntoIterator<Item = HexCoord>,
    issues: &mut Vec<WorldValidationIssue>,
) -> (LandformVegetationMetrics, u32) {
    let vegetation = match landform_vegetation_metrics(
        "Waterfall",
        V3EnvironmentSettings::TemperateGrassland,
        features.by_id.values(),
    ) {
        Ok(metrics) => metrics,
        Err(error) => {
            issues.push(recipe_issue(error));
            LandformVegetationMetrics { trees: 0, grass: 0 }
        }
    };
    if !(2..=5).contains(&vegetation.trees) {
        issues.push(recipe_issue(format!(
            "Waterfall has {} authored trees; expected 2 through 5",
            vegetation.trees
        )));
    }

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
    for coord in protected_centres
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
    let authored_blockers = features
        .by_id
        .values()
        .flat_map(|feature| feature.blocker_footprint.iter().copied())
        .collect::<BTreeSet<_>>();
    if authored_blockers != *blockers {
        issues.push(recipe_issue(
            "Waterfall blockers must exactly equal its authored tree footprints",
        ));
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

pub(crate) fn validate_patch(
    patch: PatchRecipeContext<'_>,
    plan: &GeneratedPatchPlan,
) -> WorldValidation<()> {
    let mut issues = validate_patch_walker_seams(&patch, &plan.volume);
    let rotation = match waterfall_rotation(&patch) {
        Ok(rotation) => rotation,
        Err(issues) => return WorldValidation::Invalid(issues),
    };
    let frame = match LocalPatchFrame::resolve_rotated(
        patch.mask(),
        patch.layout().kind,
        patch.grid_radius(),
        rotation,
    ) {
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
    let local_protected_centres = match waterfall_route_centres(
        frame.scale(),
        &local_mask,
        patch.layout().kind.is_composite(),
    ) {
        Ok(centres) => centres,
        Err(mut route_issues) => {
            issues.append(&mut route_issues);
            BTreeSet::new()
        }
    };
    let mut protected_centres = match local_protected_centres
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
    protected_centres.extend(patch.protected_approaches());
    validate_waterfall_vegetation(
        &plan.volume,
        &plan.liquids,
        &plan.features,
        &plan.structures,
        &plan.blockers,
        &plan.anchors,
        protected_centres,
        &mut issues,
    );
    let watercourse = match watercourse(&local_mask) {
        Ok(watercourse) => watercourse,
        Err(issues) => return WorldValidation::Invalid(issues),
    };
    if let Err(port_issues) =
        validate_waterfall_liquid_ports(&patch, frame, &watercourse, COMPOSITE_LOW_WATER_LEVEL)
    {
        return WorldValidation::Invalid(port_issues);
    }
    let seam_context = match stitched_seam_context(patch, plan, frame) {
        Ok(context) => context,
        Err(issue) => return WorldValidation::Invalid(vec![issue]),
    };
    match frame.canonical_local_world(plan) {
        Ok(plan) => {
            validate_stitched_waterfall(&plan, &seam_context, &mut issues);
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
}

fn stitched_seam_context(
    patch: PatchRecipeContext<'_>,
    plan: &GeneratedPatchPlan,
    frame: LocalPatchFrame,
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
    let critical = bypass_tiles(frame.scale(), &local_mask).map_err(first_recipe_issue)?;
    let secondary =
        secondary_bypass_tiles(frame.scale(), &local_mask).map_err(first_recipe_issue)?;
    let apron = secondary_slope_apron(frame.scale(), &local_mask).map_err(first_recipe_issue)?;
    let ring_secondary_flank =
        ring_secondary_flank_apron(frame.scale(), &local_mask).map_err(first_recipe_issue)?;
    let mut projected_authored = BTreeMap::new();
    for expected in critical
        .into_iter()
        .flatten()
        .chain(secondary.into_iter().flatten())
        .chain(apron)
        .chain(ring_secondary_flank.iter().copied())
        .chain(std::iter::once(RING_BRIDGE_FLANK))
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
                TilePos::new(coord, CLIFF_MID_LEVEL),
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
                LOW_LAND_LEVEL,
                LOW_LAND_LEVEL + 1,
                LOW_LAND_LEVEL + 2,
                HIGH_LAND_LEVEL,
                HIGH_LAND_LEVEL + 1,
                HIGH_LAND_LEVEL + 2,
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

    let watercourse = match watercourse(&plan.layout.footprint) {
        Ok(watercourse) => Some(watercourse),
        Err(mut watercourse_issues) => {
            issues.append(&mut watercourse_issues);
            None
        }
    };
    validate_flow_stages(
        body,
        watercourse.as_ref(),
        COMPOSITE_LOW_WATER_LEVEL,
        issues,
    );
    let fall_nodes = body
        .nodes
        .iter()
        .filter_map(|(position, node)| {
            (node.state == LiquidFlowState::Fall).then_some((*position, node.downstream))
        })
        .collect::<Vec<_>>();
    validate_fall(&fall_nodes, COMPOSITE_LOW_WATER_LEVEL, issues);
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
    validate_liquid_beds(plan, body, &surfaces_by_coord, issues);
    validate_bridge(plan, issues);
    validate_stitched_escarpment(plan, seam, issues);
    validate_stitched_closed_pockets(plan, seam, issues);

    let ordinary = OrdinaryGraph::from_volume(&plan.volume, Some(&plan.blockers));
    let bypass = match bypass_tiles(plan.layout.grid_radius, &plan.layout.footprint) {
        Ok(bypass) => validate_stitched_bypass(
            plan,
            &ordinary,
            &bypass,
            "critical",
            inclusive_span_len(BYPASS_HIGH_X, BYPASS_LOW_X),
            seam,
            issues,
        ),
        Err(mut bypass_issues) => {
            issues.append(&mut bypass_issues);
            [Vec::new(), Vec::new()]
        }
    };
    let secondary_bypass =
        match secondary_bypass_tiles(plan.layout.grid_radius, &plan.layout.footprint) {
            Ok(bypass) => validate_stitched_bypass(
                plan,
                &ordinary,
                &bypass,
                "secondary",
                inclusive_span_len(SECONDARY_HIGH_X, SECONDARY_LOW_X),
                seam,
                issues,
            ),
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
    validate_stitched_secondary_apron(plan, &ordinary, &secondary_apron, seam, issues);
    validate_stitched_ring_landings(plan, &ordinary, seam, issues);
    validate_stitched_route_redundancy(&ordinary, &bypass, &secondary_bypass, issues);
    validate_stitched_network(plan, &ordinary, &bypass, issues);
}

fn validate_stitched_bypass(
    plan: &GeneratedWorldPlan,
    ordinary: &OrdinaryGraph,
    bypass: &[Vec<TilePos>; 2],
    name: &str,
    expected_length: usize,
    seam: &StitchedSeamContext,
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
    let minimum_relief = HIGH_LAND_LEVEL
        .saturating_sub(LOW_LAND_LEVEL)
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
    issues: &mut Vec<WorldValidationIssue>,
) {
    let mut resolved_flank = Vec::new();
    for expected in seam
        .ring_secondary_flank
        .iter()
        .copied()
        .chain(std::iter::once(RING_BRIDGE_FLANK))
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
                if expected != RING_BRIDGE_FLANK {
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
    let secondary = secondary_bypass_tiles(plan.layout.grid_radius, &plan.layout.footprint)
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
    let bridge_flank = seam.projected_authored.get(&RING_BRIDGE_FLANK).copied();
    let bridge_deck = TilePos::new(
        HexCoord::from_axial(BRIDGE_FIRST_X, RING_BRIDGE_FLANK.coord.y()),
        BRIDGE_DECK_LEVEL,
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
            if start.level < HIGH_LAND_LEVEL.saturating_sub(2)
                || goal.level > LOW_LAND_LEVEL.saturating_add(2)
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
            .any(|component| component.len() > MAX_RING_CLOSED_POCKET_CELLS)
        || plan
            .anchors
            .values()
            .any(|position| marked.contains(position))
        || !unexpected_dry_access.is_empty()
    {
        issues.push(recipe_issue(format!(
            "Waterfall Ring7 isolated-terrain projection must exactly tag closed pockets of at \
             most {MAX_RING_CLOSED_POCKET_CELLS} cells (marked {}, expected {}, components {:?}, \
             unexpected dry access {:?})",
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

pub(crate) fn validate_waterfall(plan: &GeneratedWorldPlan) -> WorldValidation<WaterfallMetrics> {
    let mut issues = Vec::new();
    let low_water_level = if plan.layout.kind.is_composite() {
        COMPOSITE_LOW_WATER_LEVEL
    } else {
        LOW_WATER_LEVEL
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
    validate_flow_stages(
        body,
        expected_watercourse.as_ref(),
        low_water_level,
        &mut issues,
    );
    if calm_nodes < 9 || current_nodes < 3 || rapid_nodes < 3 {
        issues.push(recipe_issue(
            "Waterfall must realize calm inlet/basin, rapid, and current stages",
        ));
    }
    let fall_height = validate_fall(&fall_nodes, low_water_level, &mut issues);
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
    validate_bridge(plan, &mut issues);
    validate_escarpment(plan, &mut issues);

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
            bridge_tiles(&plan.layout.footprint)
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
            let base = if position.level >= HIGH_LAND_LEVEL {
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
    let protected_centres = match waterfall_route_centres(
        plan.layout.grid_radius,
        &plan.layout.footprint,
        plan.layout.kind.is_composite(),
    ) {
        Ok(centres) => centres,
        Err(mut route_issues) => {
            issues.append(&mut route_issues);
            BTreeSet::new()
        }
    };
    let (vegetation, grass_surface_percent) = validate_waterfall_vegetation(
        &plan.volume,
        &plan.liquids,
        &plan.features,
        &plan.structures,
        &plan.blockers,
        &plan.anchors,
        protected_centres,
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
    low_water_level: Level,
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
    for lane in &watercourse.main_lanes {
        for (index, coord) in lane.iter().copied().enumerate() {
            main_coords.insert(coord);
            let next = lane.get(index.saturating_add(1)).copied();
            let expected = water_cell(coord, next, low_water_level);
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
        let position = TilePos::new(*coord, low_water_level);
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
            let terminal = TilePos::new(*last, low_water_level);
            let leaves_west = !watercourse.coordinates().contains(&HexCoord::from_axial(
                first.x().saturating_sub(1),
                first.y(),
            ));
            let leaves_east = !watercourse
                .coordinates()
                .contains(&HexCoord::from_axial(last.x().saturating_add(1), last.y()));
            if !leaves_west
                || !leaves_east
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
                    "Waterfall lane y={} does not span the world and terminate as still water",
                    last.y()
                )));
            }
        }
    }
}

fn validate_fall(
    fall_nodes: &[(TilePos, Option<TilePos>)],
    low_water_level: Level,
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
    let expected = HIGH_WATER_LEVEL.saturating_sub(low_water_level);
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

fn validate_bridge(plan: &GeneratedWorldPlan, issues: &mut Vec<WorldValidationIssue>) {
    let expected = match bridge_tiles(&plan.layout.footprint) {
        Ok(bridge) => bridge,
        Err(mut bridge_issues) => {
            issues.append(&mut bridge_issues);
            return;
        }
    };
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
    for x in BRIDGE_FIRST_X..=BRIDGE_LAST_X {
        let lane: Vec<_> = (-BRIDGE_BANK_Y..=BRIDGE_BANK_Y)
            .map(|y| TilePos::new(HexCoord::from_axial(x, y), BRIDGE_DECK_LEVEL))
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
    let north = ordinary_at(HexCoord::from_axial(BRIDGE_FIRST_X, -BRIDGE_BANK_Y - 1));
    let south = ordinary_at(HexCoord::from_axial(BRIDGE_FIRST_X, BRIDGE_BANK_Y + 1));
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

fn validate_stitched_escarpment(
    plan: &GeneratedWorldPlan,
    seam: &StitchedSeamContext,
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
                    position.level <= LOW_LAND_LEVEL
                        || position.level >= HIGH_LAND_LEVEL
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
    let mut protected_coords = bypass_tiles(plan.layout.grid_radius, &plan.layout.footprint)
        .into_iter()
        .flat_map(|lanes| lanes.into_iter().flatten().map(|position| position.coord))
        .chain(
            secondary_bypass_tiles(plan.layout.grid_radius, &plan.layout.footprint)
                .into_iter()
                .flat_map(|lanes| lanes.into_iter().flatten().map(|position| position.coord)),
        )
        .chain(
            secondary_slope_apron(plan.layout.grid_radius, &plan.layout.footprint)
                .into_iter()
                .flatten()
                .map(|position| position.coord),
        )
        .chain(
            bridge_tiles(&plan.layout.footprint)
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
    protected_coords.insert(RING_BRIDGE_FLANK.coord);

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
                        HIGH_LAND_LEVEL
                    } else {
                        LOW_LAND_LEVEL
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

fn validate_escarpment(plan: &GeneratedWorldPlan, issues: &mut Vec<WorldValidationIssue>) {
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
            .any(|position| position.level != CLIFF_MID_LEVEL)
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
        bypass_tiles(plan.layout.grid_radius, &plan.layout.footprint)
            .into_iter()
            .flat_map(|lanes| lanes.into_iter().flatten().map(|position| position.coord))
            .chain(
                secondary_bypass_tiles(plan.layout.grid_radius, &plan.layout.footprint)
                    .into_iter()
                    .flat_map(|lanes| lanes.into_iter().flatten().map(|position| position.coord)),
            )
            .chain(
                secondary_slope_apron(plan.layout.grid_radius, &plan.layout.footprint)
                    .into_iter()
                    .flatten()
                    .map(|position| position.coord),
            )
            .chain(
                bridge_tiles(&plan.layout.footprint)
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
        if position.level <= CLIFF_MID_LEVEL {
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
            if start.level != HIGH_LAND_LEVEL || goal.level != LOW_LAND_LEVEL {
                issues.push(recipe_issue(format!(
                    "Waterfall {name} lane {lane_index} spans levels {} -> {}, expected {HIGH_LAND_LEVEL} -> {LOW_LAND_LEVEL}",
                    start.level, goal.level
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
            worked_stone: SubstanceId(12),
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
        let expected = bridge_tiles(&plan.layout.footprint).expect("fixed upper bridge");
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
        let bypass = bypass_tiles(12, &plan.layout.footprint).expect("fixed bypass");
        let secondary =
            secondary_bypass_tiles(12, &plan.layout.footprint).expect("secondary bypass");
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
        let apron = secondary_slope_apron(12, &plan.layout.footprint).expect("slope apron");
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
        let critical = bypass_tiles(12, &plan.layout.footprint).expect("critical bypass");
        let secondary =
            secondary_bypass_tiles(12, &plan.layout.footprint).expect("secondary bypass");
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
        validate_route_redundancy(&ordinary, &critical, &shared, &mut issues);
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
        validate_route_redundancy(&ordinary, &critical, &false_terminal, &mut issues);
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
            let critical = bypass_tiles(radius, &layout.footprint).expect("critical bypass");
            let secondary =
                secondary_bypass_tiles(radius, &layout.footprint).expect("secondary bypass");
            let apron = secondary_slope_apron(radius, &layout.footprint).expect("secondary apron");
            let bridge = bridge_tiles(&layout.footprint).expect("upper bridge");
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
        let critical = bypass_tiles(12, &layout.footprint).expect("critical bypass");
        let secondary = secondary_bypass_tiles(12, &layout.footprint).expect("secondary bypass");
        let apron = secondary_slope_apron(12, &layout.footprint).expect("secondary apron");
        let bridge = bridge_tiles(&layout.footprint).expect("upper bridge");
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
