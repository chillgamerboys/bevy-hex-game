//! Pure semantic Fort recipe for procedural generator V3.
//!
//! The fortress is authored as exact voxel structure geometry. Its two-wide wall
//! walk, gate apertures, and stair terraces are therefore validated by the same
//! ordinary-walker graph used by live movement rather than by a parallel notion of
//! decorative collision.

use std::collections::{BTreeMap, BTreeSet};

use hex_core::{HexCoord, MapViewHint, SpecialMovementRegion, TilePos};

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
use super::volume::{
    LevelInterval, SolidMass, SolidMaterialRole, SurfaceAccess, SurfaceMetadata, VolumeColumn,
    VolumeElement, VolumePlan,
};
use super::world::{
    FeaturePlan, GeneratedWorldPlan, InteriorPlan, PlannedStructure, StructureId, StructureKind,
    StructurePlan, WorldIssueCode, WorldValidationIssue,
};
use super::V3GenerationError;
use crate::settings::{
    ProceduralV3Settings, V3EnvironmentSettings, V3FortSettings, V3LayoutSettings, V3RecipeSettings,
};

const DEFAULT_GROUND_LEVEL: i32 = 15;
const SITE_RADIUS: u32 = 9;
const INNER_WALL_RADIUS: u32 = 6;
const OUTER_WALL_RADIUS: u32 = 7;
const BATTLEMENT_RADIUS: u32 = 8;
const CURTAIN_HEIGHT: i32 = 5;
const TOWER_APRON_RISE: i32 = 1;
const TOWER_CORE_RISE: i32 = 2;
const KEEP_HEIGHT: i32 = 8;
const GATE_CLEAR_LEVELS: i32 = 4;
const BATTLEMENT_REGION: SpecialMovementRegion = SpecialMovementRegion(20);
const KEEP_ROOF_REGION: SpecialMovementRegion = SpecialMovementRegion(21);

const PARTY_START: &str = "party_start";
const HOSTILE_START: &str = "hostile_start";
const COURTYARD: &str = "fort_courtyard";
const WALL_WALK: &str = "fort_wall_walk";
const KEEP_OVERLOOK: &str = "fort_keep";

/// Deterministic Fort diagnostics retained by candidate selection and reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FortMetrics {
    pub(crate) wall_voxels: u32,
    pub(crate) wall_walk_surfaces: u32,
    pub(crate) battlement_columns: u32,
    pub(crate) tower_count: u32,
    pub(crate) gate_count: u32,
    pub(crate) stair_count: u32,
    pub(crate) courtyard_surfaces: u32,
    pub(crate) ordinary_surfaces: u32,
    pub(crate) reachable_elevation_levels: u32,
    pub(crate) relief: u32,
    pub(crate) curtain_height: u32,
    pub(crate) keep_height: u32,
    pub(crate) critical_route_steps: u32,
    pub(crate) independent_gate_routes: u32,
    pub(crate) worked_stone_surfaces: u32,
}

#[derive(Debug)]
struct FortRecipe {
    level_height: f32,
    layout: ResolvedLayoutPlan,
    #[cfg(test)]
    reject_candidates: bool,
}

#[derive(Debug, Clone, Copy)]
struct FortStreams<'a> {
    orientation: SeedStream<'a>,
    keep: SeedStream<'a>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FortCell {
    Wall,
    Gate { index: u8 },
    Stair { index: u8, step: u8 },
    Tower { index: u8, rise: i32 },
    Keep,
    Battlement,
}

#[derive(Debug)]
struct FortTemplate {
    center: HexCoord,
    ground_level: i32,
    rotation: u8,
    cells: BTreeMap<HexCoord, FortCell>,
    gate_floors: [BTreeSet<TilePos>; 2],
    stair_lanes: [[Vec<TilePos>; 2]; 2],
    wall_walk: BTreeSet<TilePos>,
    tower_tops: BTreeSet<TilePos>,
    courtyard: BTreeSet<TilePos>,
}

/// Runs the common eight-candidate V3 selector for one Fort world.
pub(crate) fn generate(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV3Settings,
    seed: u64,
) -> Result<ValidatedWorldSelection<FortMetrics>, V3GenerationError> {
    if !level_height.is_finite() || level_height <= 0.0 {
        return Err(V3GenerationError::RecipeContract(
            "Fort level height must be positive and finite".to_owned(),
        ));
    }
    validate_recipe_settings(settings)?;
    let layout = resolve_layout(grid_radius, settings)
        .map_err(|error| V3GenerationError::RecipeContract(error.to_string()))?;
    validate_footprint_capacity(&layout)?;
    run_recipe(
        &FortRecipe {
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

impl V3Recipe for FortRecipe {
    type Settings = ProceduralV3Settings;
    type Metrics = FortMetrics;
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

        validate_recipe_settings(settings).map_err(CandidateAttemptError::Fatal)?;
        if context.grid_radius != self.layout.grid_radius {
            return Err(CandidateAttemptError::Fatal(
                V3GenerationError::RecipeContract(
                    "Fort candidate radius disagrees with its resolved layout".to_owned(),
                ),
            ));
        }
        let patch = PatchRecipeContext::resolve(&self.layout, PatchId(0))
            .map_err(CandidateAttemptError::Fatal)?;
        let fragment = construct_patch(
            patch,
            &V3FortSettings,
            self.level_height,
            PatchBuildMode::Candidate {
                world_seed: context.seed,
                candidate: context.candidate,
            },
        )
        .map_err(CandidateAttemptError::Rejected)?;
        compose_single_patch(self.layout.clone(), fragment).map_err(|error| {
            CandidateAttemptError::Fatal(V3GenerationError::RecipeContract(format!(
                "Fort single-patch composition failed: {error:?}"
            )))
        })
    }

    fn validate(
        &self,
        _settings: &Self::Settings,
        plan: &GeneratedWorldPlan,
    ) -> WorldValidation<Self::Metrics> {
        validate_fort(plan)
    }

    fn repair(
        &self,
        _context: CandidateContext,
        _settings: &Self::Settings,
        _plan: &mut GeneratedWorldPlan,
        _round: u8,
        _issues: &[WorldValidationIssue],
    ) -> Result<RepairOutcome, CandidateAttemptError> {
        // Fort topology is exact and compact. A malformed gate, wall, or stair is a
        // major topology change, so another whole candidate is safer than carving it.
        Ok(RepairOutcome::NoChange)
    }

    fn score(
        &self,
        _settings: &Self::Settings,
        metrics: &Self::Metrics,
        candidate: u8,
    ) -> Self::Score {
        (
            metrics.critical_route_steps.abs_diff(18),
            metrics.relief.abs_diff(7),
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
                "Fort fallback radius disagrees with its resolved layout".to_owned(),
            ));
        }
        let patch = PatchRecipeContext::resolve(&self.layout, PatchId(0))?;
        let fragment = construct_patch(
            patch,
            &V3FortSettings,
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
                "Fort fallback composition failed: {error:?}"
            ))
        })
    }
}

fn validate_recipe_settings(settings: &ProceduralV3Settings) -> Result<(), V3GenerationError> {
    let V3LayoutSettings::Single(patch) = &settings.layout else {
        return Err(V3GenerationError::RecipeUnavailable("Ring7"));
    };
    if patch.environment != V3EnvironmentSettings::TemperateGrassland {
        return Err(V3GenerationError::RecipeContract(
            "Fort requires the TemperateGrassland environment".to_owned(),
        ));
    }
    if !matches!(patch.recipe, V3RecipeSettings::Fort(V3FortSettings)) {
        return Err(V3GenerationError::RecipeUnavailable(recipe_name(
            &patch.recipe,
        )));
    }
    if !patch.overlays.is_empty() {
        return Err(V3GenerationError::RecipeContract(
            "Fort overlays are not implemented yet".to_owned(),
        ));
    }
    Ok(())
}

fn validate_footprint_capacity(layout: &ResolvedLayoutPlan) -> Result<(), V3GenerationError> {
    let patch = layout.patches.get(&PatchId(0)).ok_or_else(|| {
        V3GenerationError::RecipeContract("Single Fort layout has no patch zero".to_owned())
    })?;
    if choose_site_center(&patch.mask).is_none() {
        return Err(V3GenerationError::RecipeContract(format!(
            "Fort requires a connected patch containing one unobstructed radius-{SITE_RADIUS} site"
        )));
    }
    Ok(())
}

pub(crate) fn construct_patch(
    patch: PatchRecipeContext<'_>,
    _settings: &V3FortSettings,
    level_height: f32,
    mode: PatchBuildMode,
) -> Result<GeneratedPatchPlan, Vec<WorldValidationIssue>> {
    let streams = mode.seed_streams(&patch);
    construct_patch_with_streams(
        patch,
        streams.map(|streams| FortStreams {
            orientation: streams.stage("fort.orientation"),
            keep: streams.stage("fort.keep"),
        }),
        level_height,
    )
}

fn construct_patch_with_streams(
    patch: PatchRecipeContext<'_>,
    streams: Option<FortStreams<'_>>,
    level_height: f32,
) -> Result<GeneratedPatchPlan, Vec<WorldValidationIssue>> {
    let mask = patch.mask().clone();
    let biome_region = patch.biome_region();
    let protected = patch.protected_approaches();
    let preferred_center = choose_site_center(&mask).ok_or_else(|| {
        vec![recipe_issue(format!(
            "Fort footprint cannot fit an unobstructed radius-{SITE_RADIUS} site"
        ))]
    })?;
    let ground_level = patch_ground_level(patch.layout(), patch.id);
    let rotation = streams.map_or(0, |streams| {
        u8::try_from(streams.orientation.sample(0) % 6).unwrap_or_default()
    });
    let keep_variant = streams.map_or(0, |streams| {
        u8::try_from(streams.keep.sample(0) % 3).unwrap_or_default()
    });
    let template = if patch.layout().kind == super::layout::LayoutKind::Single {
        FortTemplate::new(
            &mask,
            preferred_center,
            ground_level,
            rotation,
            keep_variant,
            &protected,
        )?
    } else {
        site_centers(&mask)
            .into_iter()
            .find_map(|center| {
                (0..6_u8)
                    .map(|offset| rotation.saturating_add(offset) % 6)
                    .flat_map(|rotation| {
                        (0..3_u8).map(move |offset| {
                            (rotation, keep_variant.saturating_add(offset) % 3)
                        })
                    })
                    .find_map(|(rotation, keep_variant)| {
                        FortTemplate::new(
                            &mask,
                            center,
                            ground_level,
                            rotation,
                            keep_variant,
                            &protected,
                        )
                        .ok()
                    })
            })
            .ok_or_else(|| {
                vec![recipe_issue(
                    "Fort cannot orient its exact structure footprint around protected seam approaches",
                )]
            })?
    };
    let center = template.center;

    let mut surface_by_coord = mask
        .iter()
        .copied()
        .map(|coord| (coord, ground_level))
        .collect();
    let seam_shape = shape_walker_seams(&patch, &mut surface_by_coord)?;
    let mut columns = BTreeMap::new();
    let mut surfaces = BTreeMap::new();
    for coord in &mask {
        let local = to_local(*coord, center, rotation);
        let in_courtyard = local.distance(HexCoord::ORIGIN) < INNER_WALL_RADIUS;
        let approach = gate_approach_local(local);
        let ground_material = if in_courtyard || approach {
            SolidMaterialRole::Gravel
        } else {
            SolidMaterialRole::Grass
        };
        let local_ground = surface_by_coord.get(coord).copied().ok_or_else(|| {
            vec![recipe_issue(format!(
                "Fort seam shaping omitted ground level for {coord:?}"
            ))]
        })?;
        match template.cells.get(coord).copied() {
            None => {
                columns.insert(*coord, ground_column(local_ground, ground_material));
                surfaces.insert(TilePos::new(*coord, local_ground), ordinary_surface());
            }
            Some(FortCell::Gate { .. }) => {
                let wall_level = ground_level.saturating_add(CURTAIN_HEIGHT);
                columns.insert(
                    *coord,
                    gate_column(ground_level, wall_level, ground_material),
                );
                surfaces.insert(TilePos::new(*coord, ground_level), ordinary_surface());
                surfaces.insert(TilePos::new(*coord, wall_level), ordinary_surface());
            }
            Some(cell) => {
                let surface = TilePos::new(*coord, cell_surface(cell, ground_level));
                columns.insert(
                    *coord,
                    built_column(ground_level, surface.level, ground_material),
                );
                let access = match cell {
                    FortCell::Battlement => SurfaceAccess::SpecialMovement(BATTLEMENT_REGION),
                    FortCell::Keep => SurfaceAccess::SpecialMovement(KEEP_ROOF_REGION),
                    FortCell::Wall
                    | FortCell::Gate { .. }
                    | FortCell::Stair { .. }
                    | FortCell::Tower { .. } => SurfaceAccess::Ordinary,
                };
                surfaces.insert(
                    surface,
                    SurfaceMetadata {
                        access,
                        interior: None,
                    },
                );
            }
        }
    }
    let mut volume = VolumePlan {
        mask: mask.clone(),
        columns,
        surfaces,
    };
    seam_shape.apply(&mut volume)?;
    let structures = template.structure_plan();
    let party_start = template.party_start();
    let hostile_start = template.hostile_start();
    let courtyard_anchor = template
        .courtyard
        .iter()
        .copied()
        .min_by_key(|position| position.coord.distance(center))
        .ok_or_else(|| vec![recipe_issue("Fort has no courtyard anchor surface")])?;
    let wall_anchor = template
        .wall_walk
        .iter()
        .copied()
        .next()
        .ok_or_else(|| vec![recipe_issue("Fort has no wall-walk anchor surface")])?;
    let keep_anchor = template
        .keep_review_anchor()
        .ok_or_else(|| vec![recipe_issue("Fort has no ordinary keep review surface")])?;
    let anchors = BTreeMap::from([
        (PARTY_START.to_owned(), party_start),
        (HOSTILE_START.to_owned(), hostile_start),
        (COURTYARD.to_owned(), courtyard_anchor),
        (WALL_WALK.to_owned(), wall_anchor),
        (KEEP_OVERLOOK.to_owned(), keep_anchor),
    ]);
    let biome_regions = volume
        .surfaces
        .keys()
        .copied()
        .map(|surface| (surface, biome_region))
        .collect();
    let view_hint = fort_view_hint(
        patch.grid_radius(),
        level_height,
        center,
        ground_level,
        rotation,
    )?;

    let fragment = GeneratedPatchPlan {
        patch_id: patch.id,
        volume,
        liquids: Default::default(),
        features: FeaturePlan::default(),
        structures,
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
                    "Fort patch {:?} failed {:?}: {}",
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

impl FortTemplate {
    fn new(
        mask: &BTreeSet<HexCoord>,
        center: HexCoord,
        ground_level: i32,
        rotation: u8,
        keep_variant: u8,
        protected: &BTreeSet<HexCoord>,
    ) -> Result<Self, Vec<WorldValidationIssue>> {
        let mut local_cells = BTreeMap::new();
        for local in HexCoord::ORIGIN.within_radius(OUTER_WALL_RADIUS) {
            if matches!(
                local.distance(HexCoord::ORIGIN),
                INNER_WALL_RADIUS | OUTER_WALL_RADIUS
            ) {
                local_cells.insert(local, FortCell::Wall);
            }
        }

        let gate_coords = gate_coordinates();
        for (index, coords) in gate_coords.iter().enumerate() {
            for local in coords {
                local_cells.insert(
                    *local,
                    FortCell::Gate {
                        index: u8::try_from(index).unwrap_or(u8::MAX),
                    },
                );
            }
        }

        for (index, tower_center) in tower_centers().into_iter().enumerate() {
            for local in tower_center
                .within_radius(1)
                .into_iter()
                .filter(|local| local.distance(HexCoord::ORIGIN) == OUTER_WALL_RADIUS)
            {
                let rise = if local == tower_center {
                    TOWER_CORE_RISE
                } else {
                    TOWER_APRON_RISE
                };
                local_cells.insert(
                    local,
                    FortCell::Tower {
                        index: u8::try_from(index).unwrap_or(u8::MAX),
                        rise,
                    },
                );
            }
        }

        let stair_coords = stair_coordinates();
        for (stair, lanes) in stair_coords.iter().enumerate() {
            for lane in lanes {
                for (step, local) in lane.iter().copied().enumerate() {
                    local_cells.insert(
                        local,
                        FortCell::Stair {
                            index: u8::try_from(stair).unwrap_or(u8::MAX),
                            step: u8::try_from(step).unwrap_or(u8::MAX),
                        },
                    );
                }
            }
        }

        let keep_center = match keep_variant % 3 {
            0 => HexCoord::from_axial(3, 0),
            1 => HexCoord::from_axial(-3, 0),
            _ => HexCoord::from_axial(3, -1),
        };
        for local in keep_center.within_radius(1) {
            local_cells.insert(local, FortCell::Keep);
        }

        let tower_footprint: BTreeSet<_> = local_cells
            .iter()
            .filter_map(|(coord, cell)| matches!(cell, FortCell::Tower { .. }).then_some(*coord))
            .collect();
        let gate_approaches = gate_approach_coords();
        for (index, local) in ring_coordinates(BATTLEMENT_RADIUS).into_iter().enumerate() {
            if index % 2 == 0
                && !tower_footprint.contains(&local)
                && !gate_approaches.contains(&local)
            {
                local_cells.entry(local).or_insert(FortCell::Battlement);
            }
        }

        let mut cells = BTreeMap::new();
        for (local, cell) in &local_cells {
            let coord = to_world(*local, center, rotation);
            if !mask.contains(&coord) {
                return Err(vec![recipe_issue(format!(
                    "Fort structure at local {local:?} leaves its patch mask"
                ))]);
            }
            if protected.contains(&coord) {
                return Err(vec![recipe_issue(format!(
                    "Fort structure at {coord:?} enters a protected seam approach"
                ))]);
            }
            cells.insert(coord, *cell);
        }

        let gate_floors = gate_coords.map(|coords| {
            coords
                .into_iter()
                .map(|local| TilePos::new(to_world(local, center, rotation), ground_level))
                .collect()
        });
        let stair_lanes = stair_coords.map(|lanes| {
            lanes.map(|lane| {
                lane.into_iter()
                    .enumerate()
                    .map(|(step, local)| {
                        TilePos::new(
                            to_world(local, center, rotation),
                            ground_level.saturating_add(i32::try_from(step).unwrap_or(i32::MAX)),
                        )
                    })
                    .collect()
            })
        });
        let wall_level = ground_level.saturating_add(CURTAIN_HEIGHT);
        let wall_walk = cells
            .iter()
            .filter_map(|(coord, cell)| {
                matches!(cell, FortCell::Wall | FortCell::Gate { .. })
                    .then_some(TilePos::new(*coord, wall_level))
                    .or_else(|| {
                        matches!(cell, FortCell::Stair { step: 5, .. })
                            .then_some(TilePos::new(*coord, wall_level))
                    })
            })
            .collect();
        let tower_tops = cells
            .iter()
            .filter_map(|(coord, cell)| {
                matches!(cell, FortCell::Tower { .. })
                    .then_some(TilePos::new(*coord, cell_surface(*cell, ground_level)))
            })
            .collect();
        let courtyard = mask
            .iter()
            .copied()
            .filter(|coord| {
                let local = to_local(*coord, center, rotation);
                local.distance(HexCoord::ORIGIN) < INNER_WALL_RADIUS && !cells.contains_key(coord)
            })
            .map(|coord| TilePos::new(coord, ground_level))
            .collect();
        Ok(Self {
            center,
            ground_level,
            rotation,
            cells,
            gate_floors,
            stair_lanes,
            wall_walk,
            tower_tops,
            courtyard,
        })
    }

    fn party_start(&self) -> TilePos {
        TilePos::new(
            to_world(HexCoord::from_axial(9, -4), self.center, self.rotation),
            self.ground_level,
        )
    }

    fn hostile_start(&self) -> TilePos {
        let preferred = to_world(HexCoord::from_axial(-2, 0), self.center, self.rotation);
        self.courtyard
            .iter()
            .copied()
            .min_by_key(|position| position.coord.distance(preferred))
            .unwrap_or_else(|| TilePos::new(preferred, self.ground_level))
    }

    fn keep_review_anchor(&self) -> Option<TilePos> {
        let keep_coords: Vec<_> = self
            .cells
            .iter()
            .filter_map(|(coord, cell)| (*cell == FortCell::Keep).then_some(*coord))
            .collect();
        self.courtyard.iter().copied().min_by_key(|surface| {
            keep_coords
                .iter()
                .map(|coord| surface.coord.distance(*coord))
                .min()
                .unwrap_or(u32::MAX)
        })
    }

    fn structure_plan(&self) -> StructurePlan {
        let mut grouped = BTreeMap::<(StructureKind, u8), BTreeSet<TilePos>>::new();
        for (coord, cell) in &self.cells {
            let (kind, index, bottom, top) = match *cell {
                FortCell::Wall | FortCell::Battlement => (
                    StructureKind::Wall,
                    0,
                    self.ground_level.saturating_add(1),
                    cell_surface(*cell, self.ground_level).saturating_add(1),
                ),
                FortCell::Gate { index } => (
                    StructureKind::Gate,
                    index,
                    self.ground_level.saturating_add(GATE_CLEAR_LEVELS + 1),
                    self.ground_level
                        .saturating_add(CURTAIN_HEIGHT)
                        .saturating_add(1),
                ),
                FortCell::Stair { index, .. } => (
                    StructureKind::Stair,
                    index,
                    self.ground_level.saturating_add(1),
                    cell_surface(*cell, self.ground_level).saturating_add(1),
                ),
                FortCell::Tower { index, .. } => (
                    StructureKind::Tower,
                    index,
                    self.ground_level.saturating_add(1),
                    cell_surface(*cell, self.ground_level).saturating_add(1),
                ),
                FortCell::Keep => (
                    StructureKind::Keep,
                    0,
                    self.ground_level.saturating_add(1),
                    cell_surface(*cell, self.ground_level).saturating_add(1),
                ),
            };
            grouped
                .entry((kind, index))
                .or_default()
                .extend((bottom..top).map(|level| TilePos::new(*coord, level)));
        }

        let mut by_id = BTreeMap::new();
        for (next, ((_kind, _index), voxels)) in grouped.into_iter().enumerate() {
            let kind = self
                .cells
                .iter()
                .find_map(|(coord, cell)| {
                    voxels
                        .iter()
                        .any(|position| position.coord == *coord)
                        .then_some(match cell {
                            FortCell::Wall | FortCell::Battlement => StructureKind::Wall,
                            FortCell::Gate { .. } => StructureKind::Gate,
                            FortCell::Stair { .. } => StructureKind::Stair,
                            FortCell::Tower { .. } => StructureKind::Tower,
                            FortCell::Keep => StructureKind::Keep,
                        })
                })
                .unwrap_or(StructureKind::Wall);
            if !voxels.is_empty() {
                by_id.insert(
                    StructureId(u32::try_from(next).unwrap_or(u32::MAX)),
                    PlannedStructure { kind, voxels },
                );
            }
        }
        StructurePlan { by_id }
    }
}

pub(crate) fn validate_fort(plan: &GeneratedWorldPlan) -> WorldValidation<FortMetrics> {
    let mut issues = Vec::new();
    if !plan.liquids.bodies.is_empty()
        || !plan.features.by_id.is_empty()
        || !plan.features.protected_routes.is_empty()
        || !plan.features.clearings.is_empty()
        || !plan.blockers.is_empty()
        || !plan.lights.is_empty()
        || !plan.interiors.by_id.is_empty()
    {
        issues.push(recipe_issue(
            "Fort must not contain liquids, features, blockers, lights, or interiors",
        ));
    }

    let Some(template) = detect_template(plan) else {
        return WorldValidation::Invalid(vec![recipe_issue(
            "Fort structures and actor anchors do not match a supported exact template",
        )]);
    };
    let expected_structures = template.structure_plan();
    if plan.structures != expected_structures {
        issues.push(recipe_issue(
            "Fort structure memberships do not exactly match the selected template",
        ));
    }

    let mut expected_anchors = BTreeMap::from([
        (PARTY_START, template.party_start()),
        (HOSTILE_START, template.hostile_start()),
    ]);
    if let Some(position) = template
        .courtyard
        .iter()
        .copied()
        .min_by_key(|position| position.coord.distance(template.center))
    {
        expected_anchors.insert(COURTYARD, position);
    }
    if let Some(position) = template.wall_walk.iter().copied().next() {
        expected_anchors.insert(WALL_WALK, position);
    }
    if let Some(position) = template.keep_review_anchor() {
        expected_anchors.insert(KEEP_OVERLOOK, position);
    }
    for (name, expected) in expected_anchors {
        if plan.anchors.get(name) != Some(&expected) {
            issues.push(recipe_issue(format!(
                "Fort anchor {name:?} does not name its exact template surface"
            )));
        }
    }

    validate_worked_stone_membership(plan, &expected_structures, &mut issues);
    let ordinary = OrdinaryGraph::from_volume(&plan.volume, None);
    let party = template.party_start();
    let hostile = template.hostile_start();
    let distances = ordinary.distances_from(party);
    if distances.len() != ordinary.len() || !distances.contains_key(&hostile) {
        let examples: Vec<_> = ordinary
            .positions()
            .filter(|position| !distances.contains_key(position))
            .take(8)
            .collect();
        issues.push(recipe_issue(format!(
            "Fort ordinary network reaches {}/{} surfaces from the party landing; disconnected \
             examples: {examples:?}",
            distances.len(),
            ordinary.len()
        )));
    }

    let all_gate_floors: BTreeSet<_> = template
        .gate_floors
        .iter()
        .flat_map(|gate| gate.iter().copied())
        .collect();
    if ordinary
        .reachable_avoiding(party, &all_gate_floors)
        .contains(&hostile)
    {
        issues.push(recipe_issue(
            "Fort curtain permits an accidental ground shortcut outside its two gates",
        ));
    }
    let mut independent_gate_routes = 0_u32;
    for gate_index in 0..template.gate_floors.len() {
        let blocked: BTreeSet<_> = template
            .gate_floors
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != gate_index)
            .flat_map(|(_, gate)| gate.iter().copied())
            .collect();
        if ordinary
            .reachable_avoiding(party, &blocked)
            .contains(&hostile)
        {
            independent_gate_routes = independent_gate_routes.saturating_add(1);
        }
    }
    if independent_gate_routes != 2 {
        issues.push(recipe_issue(format!(
            "Fort requires two independent gate routes, got {independent_gate_routes}"
        )));
    }

    for (index, gate) in template.gate_floors.iter().enumerate() {
        if gate.len() != 4 {
            issues.push(recipe_issue(format!(
                "Fort gate {index} must occupy exactly four two-wide passage surfaces"
            )));
        }
        for floor in gate {
            let headroom = plan
                .volume
                .surface_headroom(*floor)
                .map_or(0, |headroom| headroom.0);
            if headroom < 2 {
                issues.push(recipe_issue(format!(
                    "Fort gate floor {floor:?} has only {headroom} clear levels"
                )));
            }
        }
    }

    for (index, lanes) in template.stair_lanes.iter().enumerate() {
        for lane in lanes {
            if lane.len() != usize::try_from(CURTAIN_HEIGHT + 1).unwrap_or(usize::MAX)
                || lane.windows(2).any(
                    |pair| !matches!(pair, [first, second] if ordinary.admits(*first, *second)),
                )
            {
                issues.push(recipe_issue(format!(
                    "Fort stair {index} is not a full one-level ordinary terrace"
                )));
            }
        }
        if lanes[0]
            .iter()
            .zip(&lanes[1])
            .any(|(first, second)| first.coord.distance(second.coord) != 1)
        {
            issues.push(recipe_issue(format!(
                "Fort stair {index} does not remain two tiles wide"
            )));
        }
    }

    let wall_access = ordinary.distances_from(hostile);
    if template
        .wall_walk
        .iter()
        .chain(&template.tower_tops)
        .any(|surface| !wall_access.contains_key(surface))
    {
        issues.push(recipe_issue(
            "Fort wall walks or tower tops are not accessible from the courtyard",
        ));
    }
    if template.wall_walk.len() < 48 {
        issues.push(recipe_issue(format!(
            "Fort two-wide curtain walk has insufficient usable surface coverage: {}",
            template.wall_walk.len()
        )));
    }

    let battlement_columns = template
        .cells
        .values()
        .filter(|cell| **cell == FortCell::Battlement)
        .count();
    if battlement_columns < 12 {
        issues.push(recipe_issue(
            "Fort requires at least twelve alternating outer battlement columns",
        ));
    }
    if template
        .cells
        .iter()
        .filter(|(_, cell)| **cell == FortCell::Battlement)
        .any(|(coord, _)| {
            plan.volume
                .surfaces
                .get(&TilePos::new(
                    *coord,
                    template.ground_level + CURTAIN_HEIGHT + 1,
                ))
                .is_none_or(|metadata| {
                    metadata.access != SurfaceAccess::SpecialMovement(BATTLEMENT_REGION)
                })
        })
    {
        issues.push(recipe_issue(
            "Fort battlements must remain outside the usable ordinary wall walk",
        ));
    }

    if !issues.is_empty() {
        return WorldValidation::Invalid(issues);
    }

    let ordinary_levels: BTreeSet<_> = ordinary
        .positions()
        .map(|position| position.level)
        .collect();
    let min_level = ordinary_levels
        .first()
        .copied()
        .unwrap_or(template.ground_level);
    let max_level = ordinary_levels
        .last()
        .copied()
        .unwrap_or(template.ground_level);
    let wall_voxels = expected_structures
        .by_id
        .values()
        .filter(|structure| structure.kind == StructureKind::Wall)
        .map(|structure| structure.voxels.len())
        .sum();
    let worked_stone_surfaces = plan
        .volume
        .surfaces
        .keys()
        .filter(|surface| surface_material(plan, **surface) == Some(SolidMaterialRole::WorkedStone))
        .count();
    WorldValidation::Valid(FortMetrics {
        wall_voxels: count_u32(wall_voxels),
        wall_walk_surfaces: count_u32(template.wall_walk.len()),
        battlement_columns: count_u32(battlement_columns),
        tower_count: count_kind(&expected_structures, StructureKind::Tower),
        gate_count: count_kind(&expected_structures, StructureKind::Gate),
        stair_count: count_kind(&expected_structures, StructureKind::Stair),
        courtyard_surfaces: count_u32(template.courtyard.len()),
        ordinary_surfaces: count_u32(ordinary.len()),
        reachable_elevation_levels: count_u32(ordinary_levels.len()),
        relief: min_level.abs_diff(max_level),
        curtain_height: u32::try_from(CURTAIN_HEIGHT).unwrap_or_default(),
        keep_height: u32::try_from(KEEP_HEIGHT).unwrap_or_default(),
        critical_route_steps: distances.get(&hostile).copied().unwrap_or_default(),
        independent_gate_routes,
        worked_stone_surfaces: count_u32(worked_stone_surfaces),
    })
}

fn detect_template(plan: &GeneratedWorldPlan) -> Option<FortTemplate> {
    let patch = plan.layout.patches.get(&PatchId(0))?;
    let protected = protected_approaches(&plan.layout, PatchId(0));
    let ground_level = patch_ground_level(&plan.layout, PatchId(0));
    for center in site_centers(&patch.mask) {
        for rotation in 0..6 {
            for keep_variant in 0..3 {
                let Ok(template) = FortTemplate::new(
                    &patch.mask,
                    center,
                    ground_level,
                    rotation,
                    keep_variant,
                    &protected,
                ) else {
                    continue;
                };
                if plan.anchors.get(PARTY_START) == Some(&template.party_start())
                    && plan.anchors.get(HOSTILE_START) == Some(&template.hostile_start())
                    && plan.structures == template.structure_plan()
                {
                    return Some(template);
                }
            }
        }
    }
    None
}

fn validate_worked_stone_membership(
    plan: &GeneratedWorldPlan,
    structures: &StructurePlan,
    issues: &mut Vec<WorldValidationIssue>,
) {
    let expected: BTreeSet<_> = structures
        .by_id
        .values()
        .flat_map(|structure| structure.voxels.iter().copied())
        .collect();
    let actual: BTreeSet<_> = plan
        .volume
        .columns
        .iter()
        .flat_map(|(coord, column)| {
            column.elements.iter().flat_map(move |element| {
                let VolumeElement::Solid(mass) = *element else {
                    return Vec::new();
                };
                if mass.material != SolidMaterialRole::WorkedStone {
                    return Vec::new();
                }
                (mass.levels.bottom..mass.levels.top)
                    .map(|level| TilePos::new(*coord, level))
                    .collect()
            })
        })
        .collect();
    if actual != expected {
        issues.push(recipe_issue(
            "worked-stone voxels do not exactly match generated structure membership",
        ));
    }
}

fn count_kind(structures: &StructurePlan, kind: StructureKind) -> u32 {
    count_u32(
        structures
            .by_id
            .values()
            .filter(|structure| structure.kind == kind)
            .count(),
    )
}

fn surface_material(plan: &GeneratedWorldPlan, surface: TilePos) -> Option<SolidMaterialRole> {
    plan.volume
        .columns
        .get(&surface.coord)?
        .elements
        .iter()
        .find_map(|element| match element {
            VolumeElement::Solid(mass)
                if mass.levels.bottom <= surface.level && surface.level < mass.levels.top =>
            {
                Some(mass.material)
            }
            VolumeElement::Solid(_) | VolumeElement::Fill(_) => None,
        })
}

fn choose_site_center(mask: &BTreeSet<HexCoord>) -> Option<HexCoord> {
    site_centers(mask).into_iter().next()
}

fn site_centers(mask: &BTreeSet<HexCoord>) -> Vec<HexCoord> {
    let boundary: Vec<_> = mask
        .iter()
        .copied()
        .filter(|coord| {
            coord
                .neighbors()
                .into_iter()
                .any(|neighbor| !mask.contains(&neighbor))
        })
        .collect();
    let mut candidates = mask
        .iter()
        .copied()
        .filter(|center| {
            center
                .within_radius(SITE_RADIUS)
                .into_iter()
                .all(|coord| mask.contains(&coord))
        })
        .collect::<Vec<_>>();
    candidates.sort_unstable_by_key(|center| {
        let clearance = boundary
            .iter()
            .map(|boundary| center.distance(*boundary))
            .min()
            .unwrap_or_default();
        let centrality = mask
            .iter()
            .map(|coord| u64::from(center.distance(*coord)))
            .sum::<u64>();
        (std::cmp::Reverse(clearance), centrality, *center)
    });
    candidates
}

fn protected_approaches(layout: &ResolvedLayoutPlan, patch: PatchId) -> BTreeSet<HexCoord> {
    layout
        .shared_edges
        .values()
        .filter_map(|edge| edge.protected_approaches.get(&patch))
        .flatten()
        .copied()
        .collect()
}

fn patch_ground_level(layout: &ResolvedLayoutPlan, patch: PatchId) -> i32 {
    let mut preferred: Vec<_> = layout
        .shared_edges
        .values()
        .filter(|edge| edge.first.0 == patch || edge.second.0 == patch)
        .map(|edge| edge.elevation.preferred)
        .collect();
    preferred.sort_unstable();
    preferred
        .get(preferred.len() / 2)
        .copied()
        .unwrap_or(DEFAULT_GROUND_LEVEL)
        .clamp(12, 24)
}

fn gate_coordinates() -> [[HexCoord; 4]; 2] {
    [
        [
            HexCoord::from_axial(6, -3),
            HexCoord::from_axial(6, -2),
            HexCoord::from_axial(7, -4),
            HexCoord::from_axial(7, -3),
        ],
        [
            HexCoord::from_axial(-6, 2),
            HexCoord::from_axial(-6, 3),
            HexCoord::from_axial(-7, 3),
            HexCoord::from_axial(-7, 4),
        ],
    ]
}

fn gate_approach_coords() -> BTreeSet<HexCoord> {
    BTreeSet::from([
        HexCoord::from_axial(8, -4),
        HexCoord::from_axial(8, -3),
        HexCoord::from_axial(-8, 3),
        HexCoord::from_axial(-8, 4),
    ])
}

fn gate_approach_local(local: HexCoord) -> bool {
    gate_coordinates()
        .into_iter()
        .flatten()
        .chain(gate_approach_coords())
        .any(|coord| coord == local)
}

fn tower_centers() -> [HexCoord; 6] {
    [
        HexCoord::new_cubic(7, -7, 0),
        HexCoord::new_cubic(7, 0, -7),
        HexCoord::new_cubic(0, 7, -7),
        HexCoord::new_cubic(-7, 7, 0),
        HexCoord::new_cubic(-7, 0, 7),
        HexCoord::new_cubic(0, -7, 7),
    ]
}

fn stair_coordinates() -> [[[HexCoord; 6]; 2]; 2] {
    let north = line_without_origin(HexCoord::from_axial(-3, 6));
    let south = line_without_origin(HexCoord::from_axial(3, -6));
    [
        [
            north,
            north.map(|coord| shift(coord, HexCoord::from_axial(1, 0))),
        ],
        [
            south,
            south.map(|coord| shift(coord, HexCoord::from_axial(-1, 0))),
        ],
    ]
}

fn line_without_origin(target: HexCoord) -> [HexCoord; 6] {
    let line = HexCoord::ORIGIN.line_between(target);
    std::array::from_fn(|index| line.get(index.saturating_add(1)).copied().unwrap_or(target))
}

fn ring_coordinates(radius: u32) -> Vec<HexCoord> {
    if radius == 0 {
        return vec![HexCoord::ORIGIN];
    }
    let directions = [
        HexCoord::new_cubic(1, -1, 0),
        HexCoord::new_cubic(1, 0, -1),
        HexCoord::new_cubic(0, 1, -1),
        HexCoord::new_cubic(-1, 1, 0),
        HexCoord::new_cubic(-1, 0, 1),
        HexCoord::new_cubic(0, -1, 1),
    ];
    let radius_i32 = i32::try_from(radius).unwrap_or(i32::MAX);
    let start = HexCoord::new_cubic(-radius_i32, 0, radius_i32);
    let mut current = start;
    let mut ring = Vec::with_capacity(usize::try_from(radius.saturating_mul(6)).unwrap_or(0));
    for direction in directions {
        for _ in 0..radius {
            ring.push(current);
            current = shift(current, direction);
        }
    }
    ring
}

fn cell_surface(cell: FortCell, ground_level: i32) -> i32 {
    match cell {
        FortCell::Wall | FortCell::Gate { .. } => ground_level.saturating_add(CURTAIN_HEIGHT),
        FortCell::Stair { step, .. } => ground_level.saturating_add(i32::from(step)),
        FortCell::Tower { rise, .. } => ground_level
            .saturating_add(CURTAIN_HEIGHT)
            .saturating_add(rise),
        FortCell::Keep => ground_level.saturating_add(KEEP_HEIGHT),
        FortCell::Battlement => ground_level
            .saturating_add(CURTAIN_HEIGHT)
            .saturating_add(1),
    }
}

fn ordinary_surface() -> SurfaceMetadata {
    SurfaceMetadata {
        access: SurfaceAccess::Ordinary,
        interior: None,
    }
}

fn ground_column(surface: i32, top: SolidMaterialRole) -> VolumeColumn {
    VolumeColumn {
        elements: vec![
            solid(0, 1, SolidMaterialRole::Bedrock),
            solid(1, surface - 3, SolidMaterialRole::Stone),
            solid(surface - 3, surface, SolidMaterialRole::Dirt),
            solid(surface, surface + 1, top),
        ],
    }
}

fn built_column(ground: i32, surface: i32, ground_material: SolidMaterialRole) -> VolumeColumn {
    let mut column = ground_column(ground, ground_material);
    if surface > ground {
        column.elements.push(solid(
            ground + 1,
            surface + 1,
            SolidMaterialRole::WorkedStone,
        ));
    }
    column
}

fn gate_column(ground: i32, wall_surface: i32, ground_material: SolidMaterialRole) -> VolumeColumn {
    let mut column = ground_column(ground, ground_material);
    column.elements.push(solid(
        ground + GATE_CLEAR_LEVELS + 1,
        wall_surface + 1,
        SolidMaterialRole::WorkedStone,
    ));
    column
}

fn solid(bottom: i32, top: i32, material: SolidMaterialRole) -> VolumeElement {
    VolumeElement::Solid(SolidMass {
        levels: LevelInterval::new(bottom, top),
        material,
        cutaway_for: None,
    })
}

fn to_world(local: HexCoord, center: HexCoord, rotation: u8) -> HexCoord {
    shift(center, rotate(local, rotation))
}

fn to_local(world: HexCoord, center: HexCoord, rotation: u8) -> HexCoord {
    unrotate(subtract(world, center), rotation)
}

fn shift(coord: HexCoord, delta: HexCoord) -> HexCoord {
    let [x, y, z] = coord.to_cubic_array();
    let [dx, dy, dz] = delta.to_cubic_array();
    HexCoord::new_cubic(x + dx, y + dy, z + dz)
}

fn subtract(coord: HexCoord, origin: HexCoord) -> HexCoord {
    let [x, y, z] = coord.to_cubic_array();
    let [ox, oy, oz] = origin.to_cubic_array();
    HexCoord::new_cubic(x - ox, y - oy, z - oz)
}

fn rotate(coord: HexCoord, turns: u8) -> HexCoord {
    let mut rotated = coord;
    for _ in 0..(turns % 6) {
        let [x, y, z] = rotated.to_cubic_array();
        rotated = HexCoord::new_cubic(-z, -x, -y);
    }
    rotated
}

fn unrotate(coord: HexCoord, turns: u8) -> HexCoord {
    rotate(coord, (6_u8.saturating_sub(turns % 6)) % 6)
}

fn fort_view_hint(
    grid_radius: u32,
    level_height: f32,
    center: HexCoord,
    ground_level: i32,
    rotation: u8,
) -> Result<MapViewHint, Vec<WorldValidationIssue>> {
    let frame = (f32::from(u16::try_from(grid_radius).unwrap_or(u16::MAX)) * 3.8).max(30.0);
    let focus_height =
        f32::from(i16::try_from(ground_level + CURTAIN_HEIGHT).unwrap_or_default()) * level_height;
    let focus = center.to_world(focus_height);
    let direction_coord = to_world(HexCoord::from_axial(8, -4), center, rotation);
    let direction = direction_coord.to_world(0.0) - center.to_world(0.0);
    let horizontal = direction
        .x
        .mul_add(direction.x, direction.z * direction.z)
        .sqrt();
    if horizontal <= f32::EPSILON {
        return Err(vec![recipe_issue(
            "Fort camera direction is horizontally degenerate",
        )]);
    }
    Ok(MapViewHint::new(
        (
            focus.x + direction.x / horizontal * frame,
            focus.y + frame,
            focus.z + direction.z / horizontal * frame,
        ),
        (focus.x, focus.y, focus.z),
    ))
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
    }
}

fn recipe_issue(detail: impl Into<String>) -> WorldValidationIssue {
    WorldValidationIssue::new(WorldIssueCode::Recipe("fort"), detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{
        PatchEdgeContractSettings, PatchEdgesSettings, PatchMaskSettings, PatchSpec,
    };
    use crate::terrain::TerrainPalette;
    use hex_core::SubstanceId;

    const WATER: SubstanceId = SubstanceId(6);
    const LAVA: SubstanceId = SubstanceId(11);

    fn settings() -> ProceduralV3Settings {
        ProceduralV3Settings {
            layout: V3LayoutSettings::Single(PatchSpec {
                environment: V3EnvironmentSettings::TemperateGrassland,
                recipe: V3RecipeSettings::Fort(V3FortSettings),
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

    fn palette() -> TerrainPalette {
        TerrainPalette {
            bedrock: SubstanceId(1),
            stone: SubstanceId(2),
            dirt: SubstanceId(3),
            grass: SubstanceId(4),
            gravel: SubstanceId(5),
            water: WATER,
            metal: SubstanceId(7),
            worked_stone: SubstanceId(12),
            snow: SubstanceId(8),
            ice: SubstanceId(9),
            basalt: SubstanceId(10),
            lava: LAVA,
        }
    }

    fn is_solid(substance: SubstanceId) -> bool {
        !matches!(substance, SubstanceId::AIR | WATER | LAVA)
    }

    #[test]
    #[ignore = "supported-radius Fort corpus is a release stress test"]
    fn fixed_corpus_builds_valid_forts_at_supported_radii() {
        for radius in [12, 20, 40] {
            for seed in [0, 1, 808, 4_294_967_311] {
                let selected =
                    generate(radius, 0.4, &settings(), seed).expect("Fort should generate");
                assert!(!selected.used_fallback);
                assert_eq!(selected.metrics.gate_count, 2);
                assert_eq!(selected.metrics.stair_count, 2);
                assert_eq!(selected.metrics.tower_count, 6);
                assert_eq!(selected.metrics.independent_gate_routes, 2);
                assert_eq!(selected.metrics.curtain_height, 5);
                assert_eq!(selected.validated.plan.validate(), Vec::new());
            }
        }
    }

    #[test]
    fn named_streams_are_repeatable_and_seed_sensitive() {
        let first = generate(12, 0.4, &settings(), 17).expect("Fort should generate");
        let repeated = generate(12, 0.4, &settings(), 17).expect("Fort should repeat");
        let other = generate(12, 0.4, &settings(), 18).expect("other Fort should generate");

        assert_eq!(
            first.validated.semantic_fingerprint,
            repeated.validated.semantic_fingerprint
        );
        assert_ne!(
            first.validated.semantic_fingerprint,
            other.validated.semantic_fingerprint
        );
    }

    #[test]
    fn materialization_uses_worked_stone_for_every_structure_voxel() {
        let selected = generate(12, 0.4, &settings(), 91).expect("Fort should generate");
        let expected: BTreeSet<_> = selected
            .validated
            .plan
            .structures
            .by_id
            .values()
            .flat_map(|structure| structure.voxels.iter().copied())
            .collect();
        let materialized =
            super::super::materialize::materialize(selected.validated, &palette(), &is_solid)
                .expect("Fort should materialize");
        assert!(expected
            .iter()
            .all(|position| materialized.map.get(*position) == SubstanceId(12)));
    }

    #[test]
    fn gate_lintels_preserve_two_level_walker_headroom() {
        let selected = generate(12, 0.4, &settings(), 91).expect("Fort should generate");
        let plan = &selected.validated.plan;
        let template = detect_template(plan).expect("Fort template should be detectable");
        for floor in template.gate_floors.iter().flatten() {
            assert!(plan
                .volume
                .surface_headroom(*floor)
                .is_some_and(|headroom| headroom.0 >= GATE_CLEAR_LEVELS));
        }
    }

    #[test]
    fn six_small_turrets_keep_three_accessible_top_surfaces_each() {
        let selected = generate(12, 0.4, &settings(), 91).expect("Fort should generate");
        let plan = &selected.validated.plan;
        let template = detect_template(plan).expect("Fort template should be detectable");
        assert_eq!(template.tower_tops.len(), 18);

        let graph = OrdinaryGraph::from_volume(&plan.volume, None);
        let courtyard_access = graph.distances_from(template.hostile_start());
        for center in tower_centers() {
            let world_center = to_world(center, template.center, template.rotation);
            let turret = template
                .tower_tops
                .iter()
                .filter(|surface| surface.coord.distance(world_center) <= 1)
                .copied()
                .collect::<BTreeSet<_>>();
            assert_eq!(turret.len(), 3);
            assert!(turret
                .iter()
                .all(|surface| courtyard_access.contains_key(surface)));
        }
    }

    #[test]
    fn closed_curtain_and_each_independent_gate_are_graph_contracts() {
        let selected = generate(12, 0.4, &settings(), 91).expect("Fort should generate");
        let plan = &selected.validated.plan;
        let template = detect_template(plan).expect("Fort template should be detectable");
        let graph = OrdinaryGraph::from_volume(&plan.volume, None);
        let party = template.party_start();
        let hostile = template.hostile_start();
        let all_gates: BTreeSet<_> = template
            .gate_floors
            .iter()
            .flat_map(|gate| gate.iter().copied())
            .collect();
        assert!(!graph
            .reachable_avoiding(party, &all_gates)
            .contains(&hostile));
        for allowed in 0..2 {
            let blocked: BTreeSet<_> = template
                .gate_floors
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != allowed)
                .flat_map(|(_, gate)| gate.iter().copied())
                .collect();
            assert!(graph.reachable_avoiding(party, &blocked).contains(&hostile));
        }
    }

    #[test]
    fn validator_rejects_a_shortcut_carved_through_the_curtain() {
        let selected = generate(12, 0.4, &settings(), 91).expect("Fort should generate");
        let mut plan = selected.validated.plan;
        let template = detect_template(&plan).expect("Fort template should be detectable");
        let wall = template
            .cells
            .iter()
            .find_map(|(coord, cell)| (*cell == FortCell::Wall).then_some(*coord))
            .expect("Fort should contain an ordinary curtain column");
        plan.volume.columns.insert(
            wall,
            ground_column(template.ground_level, SolidMaterialRole::Gravel),
        );
        let old_surface = plan
            .volume
            .surfaces
            .keys()
            .find(|surface| surface.coord == wall)
            .copied()
            .expect("wall should expose one surface");
        let _removed = plan.volume.surfaces.remove(&old_surface);
        plan.volume.surfaces.insert(
            TilePos::new(wall, template.ground_level),
            ordinary_surface(),
        );
        plan.biome_regions.remove(&old_surface);
        plan.biome_regions.insert(
            TilePos::new(wall, template.ground_level),
            plan.layout
                .patches
                .get(&PatchId(0))
                .expect("patch zero")
                .biome_region,
        );

        assert!(matches!(validate_fort(&plan), WorldValidation::Invalid(_)));
    }

    #[test]
    fn all_candidate_rejection_uses_the_independent_canonical_fallback() {
        let layout = resolve_layout(12, &settings()).expect("Fort layout should resolve");
        let recipe = FortRecipe {
            level_height: 0.4,
            layout,
            reject_candidates: true,
        };
        let selected =
            run_recipe(&recipe, &settings(), 12, 91).expect("Fort fallback should validate");

        assert!(selected.used_fallback);
        assert_eq!(selected.selected_candidate, None);
        assert_eq!(selected.valid_candidates, 0);
        assert_eq!(selected.metrics.independent_gate_routes, 2);
    }
}
