//! Pure semantic Outpost recipe for procedural generator V3.
//!
//! The compact tower stairs deliberately revisit the same horizontal coordinates
//! every nine levels. They are authored as exact isolated tread voxels before the
//! result is compressed into material runs, preserving every stacked surface.

use std::collections::{BTreeMap, BTreeSet};

use hex_core::{HexCoord, MapViewHint, SpecialMovementRegion, TilePos};

use super::composition::{compose_single_patch, GeneratedPatchPlan};
use super::layout::{resolve_layout, PatchId, ResolvedLayoutPlan};
use super::patch::{PatchBuildMode, PatchRecipeContext};
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
    ProceduralV3Settings, V3EnvironmentSettings, V3LayoutSettings, V3OutpostSettings,
    V3RecipeSettings,
};

const GROUND_LEVEL: i32 = 15;
const REQUIRED_RADIUS: u32 = 12;
const WALL_WALK_RADIUS: u32 = 9;
const PARAPET_RADIUS: u32 = 10;
const TOWER_WALL_RADIUS: u32 = 3;
const LOOKOUT_RADIUS: u32 = 4;
const FRONT_WALK_RISE: i32 = 7;
const WALL_WALK_RISE: i32 = 11;
const PARAPET_RISE: i32 = 12;
const LOOKOUT_RISE: i32 = 27;
const STAIR_LOOP_RISE: i32 = 9;
const STAIR_LOOPS_PER_TOWER: u32 = 3;
const PARAPET_REGION: SpecialMovementRegion = SpecialMovementRegion(30);

const PARTY_START: &str = "party_start";
const HOSTILE_START: &str = "hostile_start";
const COURTYARD: &str = "outpost_courtyard";
const FRONT_WALK: &str = "outpost_front_walk";
const WALL_WALK: &str = "outpost_wall_walk";
const ROOFTOP: &str = "outpost_rooftop";

/// Deterministic Outpost diagnostics retained by candidate selection and reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OutpostMetrics {
    pub(crate) structure_voxels: u32,
    pub(crate) courtyard_surfaces: u32,
    pub(crate) wall_walk_surfaces: u32,
    pub(crate) stair_surfaces: u32,
    pub(crate) tower_count: u32,
    pub(crate) stair_loops: u32,
    pub(crate) lookout_surfaces: u32,
    pub(crate) gate_surfaces: u32,
    pub(crate) ordinary_surfaces: u32,
    pub(crate) reachable_elevation_levels: u32,
    pub(crate) relief: i32,
    pub(crate) critical_route_steps: u32,
    pub(crate) connected_tower_routes: u32,
    pub(crate) worked_stone_surfaces: u32,
}

#[derive(Debug)]
struct OutpostRecipe {
    level_height: f32,
    layout: ResolvedLayoutPlan,
    #[cfg(test)]
    reject_candidates: bool,
}

#[derive(Debug, Clone, Copy)]
struct TowerSpec {
    center: HexCoord,
    turns: u8,
    mirrored: bool,
    index: u8,
}

#[derive(Debug)]
struct OutpostTemplate {
    volume: VolumePlan,
    structures: StructurePlan,
    party_start: TilePos,
    hostile_start: TilePos,
    courtyard: BTreeSet<TilePos>,
    front_walk: BTreeSet<TilePos>,
    wall_walk: BTreeSet<TilePos>,
    stair_paths: [Vec<TilePos>; 2],
    stair_surfaces: BTreeSet<TilePos>,
    lookouts: [BTreeSet<TilePos>; 2],
    gate_floors: BTreeSet<TilePos>,
    gate_closure: BTreeSet<TilePos>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AuthoredVoxel {
    material: SolidMaterialRole,
    structure: Option<(StructureKind, u8)>,
}

#[derive(Debug, Default)]
struct VoxelAuthor {
    columns: BTreeMap<HexCoord, BTreeMap<i32, AuthoredVoxel>>,
    special_surfaces: BTreeSet<TilePos>,
}

/// Runs the common eight-candidate V3 selector for one Outpost world.
pub(crate) fn generate(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV3Settings,
    seed: u64,
) -> Result<ValidatedWorldSelection<OutpostMetrics>, V3GenerationError> {
    if grid_radius != REQUIRED_RADIUS {
        return Err(V3GenerationError::RecipeContract(format!(
            "Outpost requires grid radius exactly {REQUIRED_RADIUS}"
        )));
    }
    if !level_height.is_finite() || level_height <= 0.0 {
        return Err(V3GenerationError::RecipeContract(
            "Outpost level height must be positive and finite".to_owned(),
        ));
    }
    validate_recipe_settings(settings)?;
    let layout = resolve_layout(grid_radius, settings)
        .map_err(|error| V3GenerationError::RecipeContract(error.to_string()))?;
    run_recipe(
        &OutpostRecipe {
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

impl V3Recipe for OutpostRecipe {
    type Settings = ProceduralV3Settings;
    type Metrics = OutpostMetrics;
    type Score = u8;

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
        let patch = PatchRecipeContext::resolve(&self.layout, PatchId(0))
            .map_err(CandidateAttemptError::Fatal)?;
        let fragment = construct_patch(
            patch,
            &V3OutpostSettings,
            self.level_height,
            PatchBuildMode::Candidate {
                world_seed: context.seed,
                candidate: context.candidate,
            },
        )
        .map_err(CandidateAttemptError::Rejected)?;
        compose_single_patch(self.layout.clone(), fragment).map_err(|error| {
            CandidateAttemptError::Fatal(V3GenerationError::RecipeContract(format!(
                "Outpost single-patch composition failed: {error:?}"
            )))
        })
    }

    fn validate(
        &self,
        _settings: &Self::Settings,
        plan: &GeneratedWorldPlan,
    ) -> WorldValidation<Self::Metrics> {
        validate_outpost(plan)
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

    fn score(&self, _settings: &Self::Settings, _metrics: &Self::Metrics, candidate: u8) -> u8 {
        candidate
    }

    fn canonical_fallback(
        &self,
        _context: FallbackContext,
        settings: &Self::Settings,
    ) -> Result<GeneratedWorldPlan, V3GenerationError> {
        validate_recipe_settings(settings)?;
        let patch = PatchRecipeContext::resolve(&self.layout, PatchId(0))?;
        let fragment = construct_patch(
            patch,
            &V3OutpostSettings,
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
                "Outpost fallback composition failed: {error:?}"
            ))
        })
    }
}

fn validate_recipe_settings(settings: &ProceduralV3Settings) -> Result<(), V3GenerationError> {
    let V3LayoutSettings::Single(patch) = &settings.layout else {
        return Err(V3GenerationError::RecipeUnavailable("Outpost composite"));
    };
    if patch.environment != V3EnvironmentSettings::TemperateGrassland {
        return Err(V3GenerationError::RecipeContract(
            "Outpost requires the TemperateGrassland environment".to_owned(),
        ));
    }
    if !matches!(patch.recipe, V3RecipeSettings::Outpost(V3OutpostSettings)) {
        return Err(V3GenerationError::RecipeUnavailable(recipe_name(
            &patch.recipe,
        )));
    }
    if !patch.overlays.is_empty() {
        return Err(V3GenerationError::RecipeContract(
            "Outpost overlays are not implemented".to_owned(),
        ));
    }
    Ok(())
}

pub(crate) fn construct_patch(
    patch: PatchRecipeContext<'_>,
    _settings: &V3OutpostSettings,
    level_height: f32,
    _mode: PatchBuildMode,
) -> Result<GeneratedPatchPlan, Vec<WorldValidationIssue>> {
    if patch.grid_radius() != REQUIRED_RADIUS || !patch.mask().contains(&HexCoord::ORIGIN) {
        return Err(vec![recipe_issue(format!(
            "Outpost requires a centered radius-{REQUIRED_RADIUS} patch"
        ))]);
    }
    let template = OutpostTemplate::build(patch.mask())?;
    let anchors = BTreeMap::from([
        (PARTY_START.to_owned(), template.party_start),
        (HOSTILE_START.to_owned(), template.hostile_start),
        (
            COURTYARD.to_owned(),
            TilePos::new(HexCoord::ORIGIN, GROUND_LEVEL),
        ),
        (
            FRONT_WALK.to_owned(),
            TilePos::new(HexCoord::from_axial(8, -4), GROUND_LEVEL + FRONT_WALK_RISE),
        ),
        (
            WALL_WALK.to_owned(),
            TilePos::new(HexCoord::from_axial(-9, 0), GROUND_LEVEL + WALL_WALK_RISE),
        ),
        (
            ROOFTOP.to_owned(),
            TilePos::new(tower_specs()[0].center, GROUND_LEVEL + LOOKOUT_RISE),
        ),
    ]);
    let biome_regions = template
        .volume
        .surfaces
        .keys()
        .copied()
        .map(|surface| (surface, patch.biome_region()))
        .collect();
    let view_hint = outpost_view_hint(level_height)?;
    let fragment = GeneratedPatchPlan {
        patch_id: patch.id,
        volume: template.volume,
        liquids: Default::default(),
        features: FeaturePlan::default(),
        structures: template.structures,
        blockers: BTreeSet::new(),
        lights: BTreeMap::new(),
        biome_regions,
        interiors: InteriorPlan::default(),
        anchors,
        view_hint,
    };
    let issues = fragment
        .validate_against(patch.layout())
        .into_iter()
        .map(|issue| {
            recipe_issue(format!(
                "Outpost patch {:?} failed {:?}: {}",
                issue.patch, issue.code, issue.detail
            ))
        })
        .collect::<Vec<_>>();
    if issues.is_empty() {
        Ok(fragment)
    } else {
        Err(issues)
    }
}

impl OutpostTemplate {
    fn build(mask: &BTreeSet<HexCoord>) -> Result<Self, Vec<WorldValidationIssue>> {
        let expected_mask: BTreeSet<_> = HexCoord::ORIGIN
            .within_radius(REQUIRED_RADIUS)
            .into_iter()
            .collect();
        if mask != &expected_mask {
            return Err(vec![recipe_issue(
                "Outpost requires the complete centered radius-12 mask",
            )]);
        }
        let party_start = TilePos::new(HexCoord::from_axial(12, -6), GROUND_LEVEL);
        let hostile_start = TilePos::new(HexCoord::ORIGIN, GROUND_LEVEL);
        let gate_coords = gate_path_coords();
        let gate_floors = gate_coords
            .iter()
            .copied()
            .map(|coord| TilePos::new(coord, GROUND_LEVEL))
            .collect::<BTreeSet<_>>();
        let gate_closure = gate_floors
            .iter()
            .copied()
            .filter(|floor| {
                matches!(
                    floor.coord.distance(HexCoord::ORIGIN),
                    WALL_WALK_RADIUS | PARAPET_RADIUS
                )
            })
            .collect::<BTreeSet<_>>();
        let towers = tower_specs();
        let mut author = VoxelAuthor::default();
        for coord in mask {
            let courtyard = coord.distance(HexCoord::ORIGIN) < WALL_WALK_RADIUS;
            let tower_interior = towers
                .iter()
                .any(|tower| coord.distance(tower.center) <= TOWER_WALL_RADIUS);
            let top = if (courtyard && !tower_interior) || gate_coords.contains(coord) {
                SolidMaterialRole::Gravel
            } else {
                SolidMaterialRole::Grass
            };
            author.ground_column(*coord, top);
        }

        let mut wall_walk = BTreeSet::new();
        let mut parapets = BTreeSet::new();
        for coord in ring_coordinates(WALL_WALK_RADIUS) {
            if gate_coords.contains(&coord) || in_tower_footprint(coord, &towers) {
                continue;
            }
            if coord == HexCoord::from_axial(9, -5) {
                author.worked_range(
                    coord,
                    GROUND_LEVEL + 1,
                    GROUND_LEVEL + PARAPET_RISE,
                    StructureKind::Gate,
                    0,
                )?;
                let surface = TilePos::new(coord, GROUND_LEVEL + PARAPET_RISE);
                parapets.insert(surface);
                author.special_surfaces.insert(surface);
                continue;
            }
            author.worked_range(
                coord,
                GROUND_LEVEL + 1,
                GROUND_LEVEL + WALL_WALK_RISE,
                StructureKind::Wall,
                0,
            )?;
            wall_walk.insert(TilePos::new(coord, GROUND_LEVEL + WALL_WALK_RISE));
        }
        for coord in ring_coordinates(PARAPET_RADIUS) {
            if gate_coords.contains(&coord) || in_tower_footprint(coord, &towers) {
                continue;
            }
            author.worked_range(
                coord,
                GROUND_LEVEL + 1,
                GROUND_LEVEL + PARAPET_RISE,
                StructureKind::Wall,
                0,
            )?;
            let surface = TilePos::new(coord, GROUND_LEVEL + PARAPET_RISE);
            parapets.insert(surface);
            author.special_surfaces.insert(surface);
        }

        for tower in towers {
            author_tower_shell(&mut author, tower)?;
            author_tower_core(&mut author, tower)?;
        }

        let mut stair_paths = [Vec::new(), Vec::new()];
        let mut stair_surfaces = BTreeSet::new();
        let mut lookouts = [BTreeSet::new(), BTreeSet::new()];
        let mut front_walk = BTreeSet::new();
        for tower in towers {
            let authored = author_stair(&mut author, tower)?;
            let Some(path_slot) = stair_paths.get_mut(usize::from(tower.index)) else {
                return Err(vec![recipe_issue(
                    "Outpost tower index exceeds its path slots",
                )]);
            };
            *path_slot = authored.0;
            stair_surfaces.extend(authored.1);
            let Some(lookout_slot) = lookouts.get_mut(usize::from(tower.index)) else {
                return Err(vec![recipe_issue(
                    "Outpost tower index exceeds its roof slots",
                )]);
            };
            *lookout_slot = author_lookout(&mut author, tower, &stair_surfaces)?;
        }

        for (coord, owner, index, already_authored) in front_walk_cells() {
            let position = TilePos::new(coord, GROUND_LEVEL + FRONT_WALK_RISE);
            if already_authored {
                front_walk.insert(position);
                continue;
            }
            author.worked_voxel(position, owner, index)?;
            if owner == StructureKind::Stair {
                stair_surfaces.insert(position);
            }
            front_walk.insert(position);
        }
        for coord in HexCoord::from_axial(8, -4).line_between(HexCoord::from_axial(10, -5)) {
            author.worked_voxel(
                TilePos::new(coord, GROUND_LEVEL + FRONT_WALK_RISE),
                StructureKind::Gate,
                0,
            )?;
            front_walk.insert(TilePos::new(coord, GROUND_LEVEL + FRONT_WALK_RISE));
        }

        for (tower, connector) in towers.into_iter().zip(upper_connectors()) {
            for coord in connector {
                let position = TilePos::new(coord, GROUND_LEVEL + WALL_WALK_RISE);
                if coord.distance(tower.center) == 2 {
                    author.worked_voxel(position, StructureKind::Stair, tower.index)?;
                    stair_surfaces.insert(position);
                }
                wall_walk.insert(position);
            }
        }

        let (volume, structures) = author.finish(mask.clone())?;
        stair_surfaces.retain(|surface| volume.surfaces.contains_key(surface));
        let courtyard = volume
            .surfaces
            .keys()
            .filter(|surface| {
                surface.level == GROUND_LEVEL
                    && surface.coord.distance(HexCoord::ORIGIN) < WALL_WALK_RADIUS
                    && !in_tower_footprint(surface.coord, &towers)
            })
            .copied()
            .collect();
        if !volume.surfaces.contains_key(&party_start)
            || !volume.surfaces.contains_key(&hostile_start)
        {
            return Err(vec![recipe_issue(
                "Outpost actor anchors are not exposed ground surfaces",
            )]);
        }
        if parapets.iter().any(|surface| {
            volume.surfaces.get(surface).is_none_or(|metadata| {
                metadata.access != SurfaceAccess::SpecialMovement(PARAPET_REGION)
            })
        }) {
            return Err(vec![recipe_issue(
                "Outpost parapet surfaces lost their special-movement classification",
            )]);
        }
        Ok(Self {
            volume,
            structures,
            party_start,
            hostile_start,
            courtyard,
            front_walk,
            wall_walk,
            stair_paths,
            stair_surfaces,
            lookouts,
            gate_floors,
            gate_closure,
        })
    }
}

impl VoxelAuthor {
    fn ground_column(&mut self, coord: HexCoord, top: SolidMaterialRole) {
        self.insert_material(TilePos::new(coord, 0), SolidMaterialRole::Bedrock);
        for level in 1..GROUND_LEVEL - 3 {
            self.insert_material(TilePos::new(coord, level), SolidMaterialRole::Stone);
        }
        for level in GROUND_LEVEL - 3..GROUND_LEVEL {
            self.insert_material(TilePos::new(coord, level), SolidMaterialRole::Dirt);
        }
        self.insert_material(TilePos::new(coord, GROUND_LEVEL), top);
    }

    fn insert_material(&mut self, position: TilePos, material: SolidMaterialRole) {
        self.columns.entry(position.coord).or_default().insert(
            position.level,
            AuthoredVoxel {
                material,
                structure: None,
            },
        );
    }

    fn worked_range(
        &mut self,
        coord: HexCoord,
        bottom: i32,
        top: i32,
        kind: StructureKind,
        index: u8,
    ) -> Result<(), Vec<WorldValidationIssue>> {
        for level in bottom..=top {
            self.worked_voxel(TilePos::new(coord, level), kind, index)?;
        }
        Ok(())
    }

    fn worked_voxel(
        &mut self,
        position: TilePos,
        kind: StructureKind,
        index: u8,
    ) -> Result<(), Vec<WorldValidationIssue>> {
        let column = self.columns.entry(position.coord).or_default();
        if let Some(existing) = column.get(&position.level) {
            if let Some(owner) = existing.structure {
                if owner != (kind, index) {
                    return Err(vec![recipe_issue(format!(
                        "Outpost structures overlap at {position:?}: {owner:?} and {:?}",
                        (kind, index)
                    ))]);
                }
            }
        }
        column.insert(
            position.level,
            AuthoredVoxel {
                material: SolidMaterialRole::WorkedStone,
                structure: Some((kind, index)),
            },
        );
        Ok(())
    }

    fn finish(
        self,
        mask: BTreeSet<HexCoord>,
    ) -> Result<(VolumePlan, StructurePlan), Vec<WorldValidationIssue>> {
        let mut columns = BTreeMap::new();
        let mut surfaces = BTreeMap::new();
        let mut grouped_structures = BTreeMap::<(StructureKind, u8), BTreeSet<TilePos>>::new();
        for coord in &mask {
            let Some(voxels) = self.columns.get(coord) else {
                return Err(vec![recipe_issue(format!(
                    "Outpost authoring omitted column {coord:?}"
                ))]);
            };
            for (level, voxel) in voxels {
                if let Some(owner) = voxel.structure {
                    grouped_structures
                        .entry(owner)
                        .or_default()
                        .insert(TilePos::new(*coord, *level));
                }
                if !voxels.contains_key(&level.saturating_add(1)) {
                    let position = TilePos::new(*coord, *level);
                    let access = if self.special_surfaces.contains(&position) {
                        SurfaceAccess::SpecialMovement(PARAPET_REGION)
                    } else {
                        SurfaceAccess::Ordinary
                    };
                    surfaces.insert(
                        position,
                        SurfaceMetadata {
                            access,
                            interior: None,
                        },
                    );
                }
            }
            columns.insert(*coord, compress_column(voxels));
        }
        let by_id = grouped_structures
            .into_iter()
            .enumerate()
            .map(|(next, ((kind, _index), voxels))| {
                (
                    StructureId(u32::try_from(next).unwrap_or(u32::MAX)),
                    PlannedStructure { kind, voxels },
                )
            })
            .collect();
        Ok((
            VolumePlan {
                mask,
                columns,
                surfaces,
            },
            StructurePlan { by_id },
        ))
    }
}

fn compress_column(voxels: &BTreeMap<i32, AuthoredVoxel>) -> VolumeColumn {
    let mut elements = Vec::new();
    let mut current: Option<(i32, i32, SolidMaterialRole)> = None;
    for (level, voxel) in voxels {
        match current {
            Some((bottom, top, material))
                if *level == top.saturating_add(1) && voxel.material == material =>
            {
                current = Some((bottom, *level, material));
            }
            Some((bottom, top, material)) => {
                elements.push(solid(bottom, top.saturating_add(1), material));
                current = Some((*level, *level, voxel.material));
            }
            None => current = Some((*level, *level, voxel.material)),
        }
    }
    if let Some((bottom, top, material)) = current {
        elements.push(solid(bottom, top.saturating_add(1), material));
    }
    VolumeColumn { elements }
}

fn author_tower_shell(
    author: &mut VoxelAuthor,
    tower: TowerSpec,
) -> Result<(), Vec<WorldValidationIssue>> {
    let ground_door = ground_door(tower);
    let inner_door = inner_door(tower);
    let outer_door = outer_door(tower);
    for coord in tower.center.within_radius(TOWER_WALL_RADIUS) {
        if coord.distance(tower.center) != TOWER_WALL_RADIUS {
            continue;
        }
        if coord == ground_door {
            author.worked_range(
                coord,
                GROUND_LEVEL + 3,
                GROUND_LEVEL + LOOKOUT_RISE,
                StructureKind::Tower,
                tower.index,
            )?;
        } else if coord == inner_door {
            author.worked_range(
                coord,
                GROUND_LEVEL + 1,
                GROUND_LEVEL + FRONT_WALK_RISE,
                StructureKind::Tower,
                tower.index,
            )?;
            author.worked_range(
                coord,
                GROUND_LEVEL + WALL_WALK_RISE,
                GROUND_LEVEL + LOOKOUT_RISE,
                StructureKind::Tower,
                tower.index,
            )?;
        } else if coord == outer_door {
            author.worked_range(
                coord,
                GROUND_LEVEL + 1,
                GROUND_LEVEL + WALL_WALK_RISE,
                StructureKind::Tower,
                tower.index,
            )?;
            author.worked_range(
                coord,
                GROUND_LEVEL + 15,
                GROUND_LEVEL + LOOKOUT_RISE,
                StructureKind::Tower,
                tower.index,
            )?;
        } else {
            author.worked_range(
                coord,
                GROUND_LEVEL + 1,
                GROUND_LEVEL + LOOKOUT_RISE,
                StructureKind::Tower,
                tower.index,
            )?;
        }
    }
    Ok(())
}

fn author_tower_core(
    author: &mut VoxelAuthor,
    tower: TowerSpec,
) -> Result<(), Vec<WorldValidationIssue>> {
    let mut route_footprint = stair_phases()
        .into_iter()
        .map(|coord| transform_local(coord, tower))
        .collect::<BTreeSet<_>>();
    for (_phase, wings) in landing_wings() {
        route_footprint.extend(wings.into_iter().map(|coord| transform_local(coord, tower)));
    }
    route_footprint.extend(
        front_walk_cells()
            .into_iter()
            .map(|(coord, _, _, _)| coord)
            .filter(|coord| coord.distance(tower.center) <= 2),
    );
    let connector_index = usize::from(tower.index);
    if let Some(connector) = upper_connectors().get(connector_index) {
        route_footprint.extend(
            connector
                .iter()
                .copied()
                .filter(|coord| coord.distance(tower.center) <= 2),
        );
    }
    for coord in tower.center.within_radius(2) {
        if route_footprint.contains(&coord) {
            continue;
        }
        author.worked_range(
            coord,
            GROUND_LEVEL,
            GROUND_LEVEL + LOOKOUT_RISE,
            StructureKind::Tower,
            tower.index,
        )?;
    }
    Ok(())
}

fn author_stair(
    author: &mut VoxelAuthor,
    tower: TowerSpec,
) -> Result<(Vec<TilePos>, BTreeSet<TilePos>), Vec<WorldValidationIssue>> {
    let phases = stair_phases();
    let landings = landing_wings();
    let mut primary = Vec::new();
    let mut surfaces = BTreeSet::new();
    for cycle in 0..STAIR_LOOPS_PER_TOWER {
        let base = i32::try_from(cycle)
            .unwrap_or(i32::MAX)
            .saturating_mul(STAIR_LOOP_RISE);
        for (phase, local) in phases.iter().copied().enumerate() {
            let rise = base.saturating_add(i32::try_from(phase).unwrap_or(i32::MAX));
            let position = TilePos::new(
                transform_local(local, tower),
                GROUND_LEVEL.saturating_add(rise),
            );
            author_stair_surface(author, position, rise, tower.index)?;
            primary.push(position);
            surfaces.insert(position);
        }
        for (phase, wings) in &landings {
            let rise = base.saturating_add(*phase);
            for local in wings {
                let position = TilePos::new(
                    transform_local(*local, tower),
                    GROUND_LEVEL.saturating_add(rise),
                );
                author_stair_surface(author, position, rise, tower.index)?;
                surfaces.insert(position);
            }
        }
    }
    let final_rise = LOOKOUT_RISE;
    for local in [
        stair_phases()[0],
        HexCoord::from_axial(-2, 0),
        HexCoord::from_axial(0, -2),
    ] {
        let position = TilePos::new(transform_local(local, tower), GROUND_LEVEL + final_rise);
        author.worked_voxel(position, StructureKind::Stair, tower.index)?;
        surfaces.insert(position);
    }
    primary.push(TilePos::new(
        transform_local(stair_phases()[0], tower),
        GROUND_LEVEL + final_rise,
    ));
    Ok((primary, surfaces))
}

fn author_stair_surface(
    author: &mut VoxelAuthor,
    position: TilePos,
    rise: i32,
    index: u8,
) -> Result<(), Vec<WorldValidationIssue>> {
    if rise < STAIR_LOOP_RISE {
        author.worked_range(
            position.coord,
            GROUND_LEVEL,
            position.level,
            StructureKind::Stair,
            index,
        )
    } else {
        author.worked_voxel(position, StructureKind::Stair, index)
    }
}

fn author_lookout(
    author: &mut VoxelAuthor,
    tower: TowerSpec,
    stair_surfaces: &BTreeSet<TilePos>,
) -> Result<BTreeSet<TilePos>, Vec<WorldValidationIssue>> {
    let holes: BTreeSet<_> = [
        HexCoord::from_axial(-1, 2),
        HexCoord::from_axial(0, 2),
        HexCoord::from_axial(-2, 2),
        HexCoord::from_axial(-1, 1),
        HexCoord::from_axial(-1, 0),
    ]
    .into_iter()
    .map(|local| transform_local(local, tower))
    .collect();
    let level = GROUND_LEVEL + LOOKOUT_RISE;
    let mut surfaces = BTreeSet::new();
    for coord in tower.center.within_radius(LOOKOUT_RADIUS) {
        let [_, axial_r, _] = coord.to_cubic_array();
        if (tower.index == 0 && axial_r >= -4) || (tower.index == 1 && axial_r <= -4) {
            continue;
        }
        if holes.contains(&coord) {
            continue;
        }
        let position = TilePos::new(coord, level);
        if !stair_surfaces.contains(&position) {
            author.worked_voxel(position, StructureKind::Tower, tower.index)?;
        }
        surfaces.insert(position);
    }
    surfaces.extend(stair_surfaces.iter().copied().filter(|position| {
        position.level == level && position.coord.distance(tower.center) <= LOOKOUT_RADIUS
    }));
    Ok(surfaces)
}

fn validate_outpost(plan: &GeneratedWorldPlan) -> WorldValidation<OutpostMetrics> {
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
            "Outpost must not contain liquids, features, blockers, lights, or interiors",
        ));
    }
    let Some(patch) = plan.layout.patches.get(&PatchId(0)) else {
        return WorldValidation::Invalid(vec![recipe_issue("Outpost has no patch zero")]);
    };
    let template = match OutpostTemplate::build(&patch.mask) {
        Ok(template) => template,
        Err(mut template_issues) => {
            issues.append(&mut template_issues);
            return WorldValidation::Invalid(issues);
        }
    };
    if plan.volume != template.volume || plan.structures != template.structures {
        issues.push(recipe_issue(
            "Outpost volume or structure membership differs from its exact authored template",
        ));
    }
    let expected_anchors = BTreeMap::from([
        (PARTY_START, template.party_start),
        (HOSTILE_START, template.hostile_start),
        (COURTYARD, TilePos::new(HexCoord::ORIGIN, GROUND_LEVEL)),
        (
            FRONT_WALK,
            TilePos::new(HexCoord::from_axial(8, -4), GROUND_LEVEL + FRONT_WALK_RISE),
        ),
        (
            WALL_WALK,
            TilePos::new(HexCoord::from_axial(-9, 0), GROUND_LEVEL + WALL_WALK_RISE),
        ),
        (
            ROOFTOP,
            TilePos::new(tower_specs()[0].center, GROUND_LEVEL + LOOKOUT_RISE),
        ),
    ]);
    for (name, position) in expected_anchors {
        if plan.anchors.get(name) != Some(&position) {
            issues.push(recipe_issue(format!(
                "Outpost anchor {name:?} does not name its exact authored surface"
            )));
        }
    }
    validate_worked_stone_membership(plan, &mut issues);
    let tower_structures = count_kind(&plan.structures, StructureKind::Tower);
    let stair_structures = count_kind(&plan.structures, StructureKind::Stair);
    if tower_structures != 2 || stair_structures != 2 {
        issues.push(recipe_issue(format!(
            "Outpost requires two tower and two stair structures, got {tower_structures} and {stair_structures}"
        )));
    }
    let actual_stair_surfaces = plan
        .structures
        .by_id
        .values()
        .filter(|structure| structure.kind == StructureKind::Stair)
        .flat_map(|structure| structure.voxels.iter().copied())
        .filter(|voxel| plan.volume.surfaces.contains_key(voxel))
        .collect::<BTreeSet<_>>();
    if actual_stair_surfaces != template.stair_surfaces {
        issues.push(recipe_issue(
            "Outpost stair metrics omit or invent an exposed stair surface",
        ));
    }
    let ordinary = OrdinaryGraph::from_volume(&plan.volume, None);
    let distances = ordinary.distances_from(template.party_start);
    if distances.len() != ordinary.len() || !distances.contains_key(&template.hostile_start) {
        let examples = ordinary
            .positions()
            .filter(|position| !distances.contains_key(position))
            .take(8)
            .collect::<Vec<_>>();
        issues.push(recipe_issue(format!(
            "Outpost ordinary network reaches {}/{} surfaces; disconnected examples: {examples:?}",
            distances.len(),
            ordinary.len()
        )));
    }
    if template
        .front_walk
        .iter()
        .any(|surface| !distances.contains_key(surface))
    {
        issues.push(recipe_issue(
            "Outpost front walk is not completely reachable from the front gate",
        ));
    }
    let non_ground = ordinary
        .positions()
        .filter(|surface| surface.level != GROUND_LEVEL)
        .collect::<BTreeSet<_>>();
    let courtyard_ground = ordinary.reachable_avoiding(template.hostile_start, &non_ground);
    for tower in tower_specs() {
        let doorway = TilePos::new(ground_door(tower), GROUND_LEVEL);
        if !courtyard_ground.contains(&doorway) {
            issues.push(recipe_issue(format!(
                "Outpost tower {} has no independent ground-level courtyard entrance",
                tower.index
            )));
        }
    }
    for (tower_index, path) in template.stair_paths.iter().enumerate() {
        if path.len() != 28
            || path
                .windows(2)
                .any(|pair| !matches!(pair, [first, second] if ordinary.admits(*first, *second)))
        {
            issues.push(recipe_issue(format!(
                "Outpost stair {tower_index} is not one continuous 27-rise ordinary path"
            )));
        }
        let Some(tower) = tower_specs().get(tower_index).copied() else {
            issues.push(recipe_issue("Outpost stair has no matching tower"));
            continue;
        };
        let ground_door = TilePos::new(ground_door(tower), GROUND_LEVEL);
        if path
            .first()
            .is_none_or(|entry| !ordinary.admits(ground_door, *entry))
        {
            issues.push(recipe_issue(format!(
                "Outpost stair {tower_index} has no traversable courtyard doorway"
            )));
        }
        if path.last().is_none_or(|exit| {
            template
                .lookouts
                .get(tower_index)
                .is_none_or(|lookout| !lookout.contains(exit))
        }) {
            issues.push(recipe_issue(format!(
                "Outpost stair {tower_index} does not terminate on its own lookout"
            )));
        }
        let phases = stair_phases();
        for cycle in 1..=STAIR_LOOPS_PER_TOWER {
            let phase_index = usize::try_from(cycle.saturating_mul(9)).unwrap_or(usize::MAX);
            let Some(position) = path.get(phase_index) else {
                issues.push(recipe_issue(format!(
                    "Outpost stair {tower_index} omitted loop closure {cycle}"
                )));
                continue;
            };
            let expected_coord = transform_local(phases[0], tower);
            if position.coord != expected_coord {
                issues.push(recipe_issue(
                    "Outpost stair loop does not close above its origin",
                ));
            }
        }
    }
    let courtyard_reach = ordinary.distances_from(template.hostile_start);
    let connected_tower_routes = template
        .lookouts
        .iter()
        .filter(|lookout| {
            lookout
                .iter()
                .any(|surface| courtyard_reach.contains_key(surface))
        })
        .count();
    if connected_tower_routes != 2 {
        issues.push(recipe_issue(format!(
            "Outpost requires two independently reachable lookout towers, got {connected_tower_routes}"
        )));
    }
    if ordinary
        .reachable_avoiding(template.party_start, &template.gate_closure)
        .contains(&template.hostile_start)
    {
        issues.push(recipe_issue(
            "Outpost permits an accidental ground shortcut around its front gate",
        ));
    }
    for floor in &template.gate_floors {
        let headroom = plan
            .volume
            .surface_headroom(*floor)
            .map_or(0, |headroom| headroom.0);
        if headroom < 2 {
            issues.push(recipe_issue(format!(
                "Outpost gate floor {floor:?} has only {headroom} clear levels"
            )));
        }
    }
    if !issues.is_empty() {
        return WorldValidation::Invalid(issues);
    }
    let levels = ordinary
        .positions()
        .map(|position| position.level)
        .collect::<BTreeSet<_>>();
    let min_level = levels.first().copied().unwrap_or(GROUND_LEVEL);
    let max_level = levels.last().copied().unwrap_or(GROUND_LEVEL);
    let structure_voxels = plan
        .structures
        .by_id
        .values()
        .map(|structure| structure.voxels.len())
        .sum::<usize>();
    let worked_stone_surfaces = plan
        .volume
        .surfaces
        .keys()
        .filter(|surface| surface_material(plan, **surface) == Some(SolidMaterialRole::WorkedStone))
        .count();
    let stair_loops = template
        .stair_paths
        .iter()
        .map(|path| path.len().saturating_sub(1) / usize::try_from(STAIR_LOOP_RISE).unwrap_or(1))
        .sum::<usize>();
    WorldValidation::Valid(OutpostMetrics {
        structure_voxels: count_u32(structure_voxels),
        courtyard_surfaces: count_u32(template.courtyard.len()),
        wall_walk_surfaces: count_u32(template.wall_walk.len()),
        stair_surfaces: count_u32(actual_stair_surfaces.len()),
        tower_count: tower_structures,
        stair_loops: count_u32(stair_loops),
        lookout_surfaces: count_u32(template.lookouts.iter().map(BTreeSet::len).sum::<usize>()),
        gate_surfaces: count_u32(template.gate_floors.len()),
        ordinary_surfaces: count_u32(ordinary.len()),
        reachable_elevation_levels: count_u32(levels.len()),
        relief: max_level.saturating_sub(min_level),
        critical_route_steps: distances
            .get(&template.hostile_start)
            .copied()
            .unwrap_or_default(),
        connected_tower_routes: count_u32(connected_tower_routes),
        worked_stone_surfaces: count_u32(worked_stone_surfaces),
    })
}

fn validate_worked_stone_membership(
    plan: &GeneratedWorldPlan,
    issues: &mut Vec<WorldValidationIssue>,
) {
    let expected = plan
        .structures
        .by_id
        .values()
        .flat_map(|structure| structure.voxels.iter().copied())
        .collect::<BTreeSet<_>>();
    let actual = plan
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
        .collect::<BTreeSet<_>>();
    if actual != expected {
        issues.push(recipe_issue(
            "Outpost worked-stone voxels do not exactly match structure membership",
        ));
    }
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

fn count_kind(structures: &StructurePlan, kind: StructureKind) -> u32 {
    count_u32(
        structures
            .by_id
            .values()
            .filter(|structure| structure.kind == kind)
            .count(),
    )
}

fn tower_specs() -> [TowerSpec; 2] {
    [
        TowerSpec {
            center: HexCoord::from_axial(8, -8),
            turns: 1,
            mirrored: false,
            index: 0,
        },
        TowerSpec {
            center: HexCoord::from_axial(8, 0),
            turns: 0,
            mirrored: true,
            index: 1,
        },
    ]
}

fn stair_phases() -> [HexCoord; 9] {
    [
        HexCoord::from_axial(-1, -1),
        HexCoord::from_axial(0, -1),
        HexCoord::from_axial(1, -1),
        HexCoord::from_axial(2, -1),
        HexCoord::from_axial(1, 0),
        HexCoord::from_axial(0, 1),
        HexCoord::from_axial(-1, 2),
        HexCoord::from_axial(-1, 1),
        HexCoord::from_axial(-1, 0),
    ]
}

fn landing_wings() -> [(i32, [HexCoord; 2]); 3] {
    [
        (
            0,
            [HexCoord::from_axial(-2, 0), HexCoord::from_axial(0, -2)],
        ),
        (3, [HexCoord::from_axial(2, -2), HexCoord::from_axial(2, 0)]),
        (6, [HexCoord::from_axial(0, 2), HexCoord::from_axial(-2, 2)]),
    ]
}

fn transform_local(local: HexCoord, tower: TowerSpec) -> HexCoord {
    let transformed = if tower.mirrored {
        mirror_across_front_axis(local)
    } else {
        rotate(local, tower.turns)
    };
    shift(tower.center, transformed)
}

fn mirror_across_front_axis(coord: HexCoord) -> HexCoord {
    let [x, y, z] = coord.to_cubic_array();
    HexCoord::new_cubic(-z, -y, -x)
}

fn inner_door(tower: TowerSpec) -> HexCoord {
    match tower.index {
        0 => HexCoord::from_axial(7, -5),
        _ => HexCoord::from_axial(7, -2),
    }
}

fn ground_door(tower: TowerSpec) -> HexCoord {
    match tower.index {
        0 => HexCoord::from_axial(5, -7),
        _ => HexCoord::from_axial(5, 2),
    }
}

fn outer_door(tower: TowerSpec) -> HexCoord {
    match tower.index {
        0 => HexCoord::from_axial(6, -9),
        _ => HexCoord::from_axial(6, 3),
    }
}

fn front_walk_cells() -> [(HexCoord, StructureKind, u8, bool); 8] {
    [
        (HexCoord::from_axial(8, -7), StructureKind::Stair, 0, true),
        (HexCoord::from_axial(7, -6), StructureKind::Stair, 0, false),
        (HexCoord::from_axial(7, -5), StructureKind::Tower, 0, true),
        (HexCoord::from_axial(7, -4), StructureKind::Gate, 0, false),
        (HexCoord::from_axial(7, -3), StructureKind::Gate, 0, false),
        (HexCoord::from_axial(7, -2), StructureKind::Tower, 1, true),
        (HexCoord::from_axial(7, -1), StructureKind::Stair, 1, false),
        (HexCoord::from_axial(8, -1), StructureKind::Stair, 1, true),
    ]
}

fn upper_connectors() -> [[HexCoord; 4]; 2] {
    [
        [
            HexCoord::from_axial(8, -9),
            HexCoord::from_axial(7, -9),
            HexCoord::from_axial(6, -9),
            HexCoord::from_axial(5, -9),
        ],
        [
            HexCoord::from_axial(8, 1),
            HexCoord::from_axial(7, 2),
            HexCoord::from_axial(6, 3),
            HexCoord::from_axial(5, 4),
        ],
    ]
}

fn gate_path_coords() -> BTreeSet<HexCoord> {
    HexCoord::from_axial(12, -6)
        .line_between(HexCoord::from_axial(6, -3))
        .into_iter()
        .collect()
}

fn in_tower_footprint(coord: HexCoord, towers: &[TowerSpec; 2]) -> bool {
    towers
        .iter()
        .any(|tower| coord.distance(tower.center) <= TOWER_WALL_RADIUS)
}

fn ring_coordinates(radius: u32) -> Vec<HexCoord> {
    HexCoord::ORIGIN
        .within_radius(radius)
        .into_iter()
        .filter(|coord| coord.distance(HexCoord::ORIGIN) == radius)
        .collect()
}

fn solid(bottom: i32, top: i32, material: SolidMaterialRole) -> VolumeElement {
    VolumeElement::Solid(SolidMass {
        levels: LevelInterval::new(bottom, top),
        material,
        cutaway_for: None,
    })
}

fn shift(coord: HexCoord, delta: HexCoord) -> HexCoord {
    let [x, y, z] = coord.to_cubic_array();
    let [dx, dy, dz] = delta.to_cubic_array();
    HexCoord::new_cubic(x + dx, y + dy, z + dz)
}

fn rotate(coord: HexCoord, turns: u8) -> HexCoord {
    let mut rotated = coord;
    for _ in 0..(turns % 6) {
        let [x, y, z] = rotated.to_cubic_array();
        rotated = HexCoord::new_cubic(-z, -x, -y);
    }
    rotated
}

fn outpost_view_hint(level_height: f32) -> Result<MapViewHint, Vec<WorldValidationIssue>> {
    let focus_height =
        f32::from(i16::try_from(GROUND_LEVEL + 14).unwrap_or_default()) * level_height;
    let focus = HexCoord::ORIGIN.to_world(focus_height);
    let front = HexCoord::from_axial(8, -4).to_world(0.0);
    let horizontal = front.x.mul_add(front.x, front.z * front.z).sqrt();
    if horizontal <= f32::EPSILON {
        return Err(vec![recipe_issue(
            "Outpost camera direction is horizontally degenerate",
        )]);
    }
    let frame = 48.0;
    Ok(MapViewHint::new(
        (
            focus.x + front.x / horizontal * frame,
            focus.y + frame,
            focus.z + front.z / horizontal * frame,
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
        V3RecipeSettings::Outpost(_) => "Outpost",
        V3RecipeSettings::Volcano(_) => "Volcano",
        V3RecipeSettings::DeepForest(_) => "DeepForest",
        V3RecipeSettings::Prairie(_) => "Prairie",
    }
}

fn recipe_issue(detail: impl Into<String>) -> WorldValidationIssue {
    WorldValidationIssue::new(WorldIssueCode::Recipe("outpost"), detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{
        PatchEdgeContractSettings, PatchEdgesSettings, PatchMaskSettings, PatchSpec,
    };

    fn settings() -> ProceduralV3Settings {
        let boundary = || PatchEdgeContractSettings::WorldBoundary;
        ProceduralV3Settings {
            layout: V3LayoutSettings::Single(PatchSpec {
                environment: V3EnvironmentSettings::TemperateGrassland,
                recipe: V3RecipeSettings::Outpost(V3OutpostSettings),
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
    fn sketched_outpost_builds_two_three_loop_towers() {
        let selected = generate(12, 0.4, &settings(), 1_290_212)
            .expect("the sketched Outpost should generate");
        assert!(!selected.used_fallback, "{:?}", selected.notes);
        assert_eq!(selected.metrics.tower_count, 2);
        assert_eq!(selected.metrics.stair_loops, 6);
        assert_eq!(selected.metrics.connected_tower_routes, 2);
        assert_eq!(selected.metrics.relief, 27);
        assert_eq!(selected.metrics.reachable_elevation_levels, 28);
        assert_eq!(selected.validated.plan.validate(), Vec::new());
    }

    #[test]
    fn stair_loops_repeat_their_horizontal_phase_every_nine_levels() {
        let selected = generate(12, 0.4, &settings(), 1_290_212)
            .expect("the sketched Outpost should generate");
        let patch = selected
            .validated
            .plan
            .layout
            .patches
            .get(&PatchId(0))
            .expect("Outpost should retain patch zero");
        let template = OutpostTemplate::build(&patch.mask).expect("template should rebuild");
        for (tower, path) in tower_specs().into_iter().zip(template.stair_paths) {
            for index in 0..=18 {
                let lower = path.get(index).expect("lower phase");
                let upper = path.get(index + 9).expect("upper phase");
                assert_eq!(lower.coord, upper.coord);
                assert_eq!(upper.level - lower.level, 9);
            }
            for cycle in 0..STAIR_LOOPS_PER_TOWER {
                let base = i32::try_from(cycle).expect("cycle fits") * STAIR_LOOP_RISE;
                for (phase, wings) in landing_wings() {
                    let level = GROUND_LEVEL + base + phase;
                    let phase_index = usize::try_from(base + phase).expect("phase fits");
                    let primary = path.get(phase_index).expect("landing primary");
                    assert_eq!(primary.level, level);
                    for wing in wings {
                        assert!(template
                            .stair_surfaces
                            .contains(&TilePos::new(transform_local(wing, tower), level)));
                    }
                }
            }
        }
    }

    #[test]
    fn mirrored_towers_open_inward_and_keep_independent_lookouts() {
        let layout = resolve_layout(12, &settings()).expect("Outpost layout should resolve");
        let patch = layout
            .patches
            .get(&PatchId(0))
            .expect("Outpost should retain patch zero");
        let template = OutpostTemplate::build(&patch.mask).expect("template should build");
        let ordinary = OrdinaryGraph::from_volume(&template.volume, None);

        let expected_entries = [HexCoord::from_axial(6, -7), HexCoord::from_axial(6, 1)];
        for (tower_index, (path, expected_entry)) in template
            .stair_paths
            .iter()
            .zip(expected_entries)
            .enumerate()
        {
            let tower = tower_specs()
                .get(tower_index)
                .copied()
                .expect("each stair has one tower");
            let entry = path.first().copied().expect("stair entry");
            assert_eq!(entry.coord, expected_entry);
            assert!(entry.coord.distance(HexCoord::ORIGIN) < WALL_WALK_RADIUS);
            assert!(ordinary.admits(TilePos::new(ground_door(tower), GROUND_LEVEL), entry));
            assert!(template
                .lookouts
                .get(tower_index)
                .expect("each tower has one lookout")
                .contains(path.last().expect("stair exit")));
        }

        let [first_lookout, second_lookout] = &template.lookouts;
        assert!(first_lookout.is_disjoint(second_lookout));
        assert!(first_lookout.iter().all(|first| second_lookout
            .iter()
            .all(|second| first.coord.distance(second.coord) > 1)));
        assert!(!first_lookout.contains(&TilePos::new(
            HexCoord::from_axial(8, -4),
            GROUND_LEVEL + LOOKOUT_RISE,
        )));
        assert!(template
            .lookouts
            .iter()
            .flatten()
            .any(|surface| surface.coord.distance(HexCoord::ORIGIN) == REQUIRED_RADIUS));
        assert!(!template.gate_closure.is_empty());
        assert!(!template.gate_closure.contains(&template.party_start));
        assert!(!ordinary
            .reachable_avoiding(template.party_start, &template.gate_closure)
            .contains(&template.hostile_start));
    }

    #[test]
    fn all_candidate_rejection_uses_the_independent_fallback() {
        let layout = resolve_layout(12, &settings()).expect("Outpost layout should resolve");
        let selected = run_recipe(
            &OutpostRecipe {
                level_height: 0.4,
                layout,
                reject_candidates: true,
            },
            &settings(),
            12,
            1_290_212,
        )
        .expect("Outpost fallback should validate");
        assert!(selected.used_fallback);
        assert_eq!(selected.metrics.connected_tower_routes, 2);
    }
}
