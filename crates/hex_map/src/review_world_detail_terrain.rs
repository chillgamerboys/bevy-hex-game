//! Deterministic terrain-attached projections for the small-geometry review.
//!
//! This module is intentionally pure. It reads an exact, stack-safe description of
//! exposed natural surfaces and generated vegetation, then returns disposable mesh
//! batches and render-child treatments. It never writes voxels, publishes logical
//! tile components, creates blockers, or attaches picking/collision metadata.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use bevy::prelude::Vec3;
use hex_core::{HexCoord, Level, SubstanceId, TilePos};
use xxhash_rust::xxh3::{xxh3_64, xxh3_64_with_seed};

use crate::review_world_detail::{
    AlpineVegetationDetailV1, CliffStrataDetailV1, ReviewWorldDetailProfileV1, SnowDetailV1,
    TerrainPropsDetailV1,
};
use crate::terrain_noise::coherent_level_offset;
use crate::voxel::terrain_chunk_key;

const SNOW_COLOR: [f32; 4] = [0.91, 0.96, 1.0, 1.0];
const BOULDER_COLOR: [f32; 4] = [0.29, 0.30, 0.32, 1.0];
const TUFT_COLOR: [f32; 4] = [0.30, 0.40, 0.23, 1.0];
const DEADWOOD_COLOR: [f32; 4] = [0.24, 0.16, 0.10, 1.0];
const OVERLAY_BIAS: f32 = 0.003;
const SNOW_TOP_BIAS: f32 = 0.004;
const COHERENT_TERRAIN_CORRELATION_HEXES: u16 = 22;

/// One exact side relationship supplied by the map-owned integration adapter.
///
/// `adjacent_surface` names the exact exposed natural surface, if one exists. The
/// separately supplied `exposed_bottom_level` is the first world-height boundary
/// visible on this surface's side; this preserves caves, overhangs, and other stacked
/// cases that cannot be reconstructed from one height per [`HexCoord`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReviewTerrainSideInputV1 {
    pub(crate) direction: HexCoord,
    pub(crate) adjacent_surface: Option<TilePos>,
    pub(crate) exposed_bottom_level: Level,
}

/// One exact natural-material interval on the contiguous solid stack below an
/// exposed terrain surface.
///
/// Cliff shells split at these authored material boundaries so a thin grass or
/// snow cap cannot recolor the stone and bedrock face beneath it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ReviewCliffLayerInputV1 {
    pub(crate) bottom_level: Level,
    pub(crate) top_level: Level,
    pub(crate) substrate: SubstanceId,
    pub(crate) substrate_color: [f32; 4],
}

/// Snow regions whose authored presentation must remain unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReviewSnowExceptionV1 {
    None,
    FrozenWoods,
    Garden,
}

/// Reasons a presentation-only prop may not occupy a surface.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReviewPropExclusionsV1 {
    pub(crate) water: bool,
    pub(crate) path: bool,
    pub(crate) corridor: bool,
    pub(crate) portal: bool,
    pub(crate) spawn: bool,
    pub(crate) structure: bool,
    pub(crate) named_anchor_safety_disk: bool,
}

impl ReviewPropExclusionsV1 {
    fn any(self) -> bool {
        self.water
            || self.path
            || self.corridor
            || self.portal
            || self.spawn
            || self.structure
            || self.named_anchor_safety_disk
    }
}

/// Exact presentation facts for one exposed natural surface.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ReviewTerrainSurfaceInputV1 {
    pub(crate) pos: TilePos,
    pub(crate) substrate: SubstanceId,
    pub(crate) substrate_color: [f32; 4],
    pub(crate) exposed_natural: bool,
    pub(crate) current_snow: bool,
    pub(crate) forced_summit: bool,
    pub(crate) snow_exception: ReviewSnowExceptionV1,
    pub(crate) sides: [ReviewTerrainSideInputV1; 6],
    pub(crate) cliff_layers: Vec<ReviewCliffLayerInputV1>,
    pub(crate) prop_exclusions: ReviewPropExclusionsV1,
}

/// Exact presentation identity for one existing generated vegetation root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReviewVegetationInputV1 {
    pub(crate) stable_id: u64,
    pub(crate) root: TilePos,
    pub(crate) snow_dust_eligible: bool,
}

/// Builder for one immutable terrain-review input.
///
/// Integration feeds this from the already-materialized `VoxelMap` and private
/// presentation projection. Duplicate exact surfaces or stable vegetation identities
/// are rejected before any render commands are possible.
#[derive(Debug)]
pub(crate) struct ReviewTerrainInputBuilderV1 {
    seed: u64,
    level_height: f32,
    surfaces: BTreeMap<TilePos, ReviewTerrainSurfaceInputV1>,
    vegetation: BTreeMap<u64, ReviewVegetationInputV1>,
}

impl ReviewTerrainInputBuilderV1 {
    pub(crate) fn new(seed: u64, level_height: f32) -> Result<Self, ReviewTerrainPlanError> {
        if !level_height.is_finite() || level_height <= 0.0 {
            return Err(ReviewTerrainPlanError::InvalidLevelHeight(level_height));
        }
        Ok(Self {
            seed,
            level_height,
            surfaces: BTreeMap::new(),
            vegetation: BTreeMap::new(),
        })
    }

    pub(crate) fn insert_surface(
        &mut self,
        surface: ReviewTerrainSurfaceInputV1,
    ) -> Result<(), ReviewTerrainPlanError> {
        if !surface
            .substrate_color
            .into_iter()
            .all(|component| component.is_finite() && (0.0..=1.0).contains(&component))
        {
            return Err(ReviewTerrainPlanError::InvalidSubstrateColor(surface.pos));
        }
        let invalid_cliff_layers = surface.cliff_layers.iter().any(|layer| {
            layer.bottom_level < 0
                || layer.bottom_level >= layer.top_level
                || layer.top_level > surface.pos.level.saturating_add(1)
                || !layer
                    .substrate_color
                    .into_iter()
                    .all(|component| component.is_finite() && (0.0..=1.0).contains(&component))
        }) || surface.cliff_layers.windows(2).any(|layers| {
            let [lower, upper] = layers else {
                return true;
            };
            lower.bottom_level > upper.bottom_level || lower.top_level > upper.bottom_level
        });
        let missing_exposed_top_layer = surface.exposed_natural
            && surface.cliff_layers.last().map(|layer| layer.top_level)
                != Some(surface.pos.level.saturating_add(1));
        if invalid_cliff_layers || missing_exposed_top_layer {
            return Err(ReviewTerrainPlanError::InvalidCliffLayers(surface.pos));
        }
        let pos = surface.pos;
        if self.surfaces.insert(pos, surface).is_some() {
            return Err(ReviewTerrainPlanError::DuplicateSurface(pos));
        }
        Ok(())
    }

    pub(crate) fn insert_vegetation(
        &mut self,
        vegetation: ReviewVegetationInputV1,
    ) -> Result<(), ReviewTerrainPlanError> {
        if self
            .vegetation
            .insert(vegetation.stable_id, vegetation)
            .is_some()
        {
            return Err(ReviewTerrainPlanError::DuplicateVegetation(
                vegetation.stable_id,
            ));
        }
        Ok(())
    }

    pub(crate) fn build(self) -> ReviewTerrainInputV1 {
        ReviewTerrainInputV1 {
            seed: self.seed,
            level_height: self.level_height,
            surfaces: self.surfaces,
            vegetation: self.vegetation,
        }
    }
}

/// Complete immutable input to the pure terrain-detail planner.
#[derive(Debug)]
pub(crate) struct ReviewTerrainInputV1 {
    seed: u64,
    level_height: f32,
    surfaces: BTreeMap<TilePos, ReviewTerrainSurfaceInputV1>,
    vegetation: BTreeMap<u64, ReviewVegetationInputV1>,
}

/// Shared material roles used by chunk-batched review geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ReviewTerrainMaterialRoleV1 {
    SnowCap,
    SubstrateRestore,
    CliffValue,
    CliffStrata,
    Boulder,
    Tuft,
    Deadwood,
}

/// One concrete, renderer-neutral triangle-list batch.
///
/// The integration adapter can insert these vectors directly into a Bevy `Mesh`.
/// `substrate` is populated for shared substrate-restoration and substrate-relative
/// cliff materials. It is never used to allocate one material per cell.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ReviewTerrainMeshBatchV1 {
    pub(crate) chunk: (i32, i32),
    pub(crate) material_role: ReviewTerrainMaterialRoleV1,
    pub(crate) substrate: Option<SubstanceId>,
    pub(crate) base_color: [f32; 4],
    pub(crate) positions: Vec<[f32; 3]>,
    pub(crate) normals: Vec<[f32; 3]>,
    pub(crate) uv0: Vec<[f32; 2]>,
    pub(crate) indices: Vec<u32>,
    pub(crate) source_items: u32,
}

/// Presentation-only treatment applied below an existing vegetation root.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ReviewVegetationProjectionV1 {
    pub(crate) stable_id: u64,
    pub(crate) root: TilePos,
    pub(crate) render_child_scale: [f32; 3],
    pub(crate) crown_dust: Option<ReviewCrownDustV1>,
}

/// Snow-shell parameters applied to existing eligible crown render children.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ReviewCrownDustV1 {
    pub(crate) upper_fraction: f32,
    pub(crate) shell_height: f32,
    pub(crate) color: [f32; 4],
}

/// One chunk group of existing vegetation-child treatments.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ReviewVegetationBatchV1 {
    pub(crate) chunk: (i32, i32),
    pub(crate) instances: Vec<ReviewVegetationProjectionV1>,
}

/// Low-poly prop silhouette selected by the presentation planner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ReviewPropKindV1 {
    Boulder,
    Tuft,
    Deadwood,
}

/// Exact presentation-only placement retained for provenance and exclusion checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReviewPropPlacementV1 {
    pub(crate) root: TilePos,
    pub(crate) kind: ReviewPropKindV1,
    pub(crate) cluster: Option<u64>,
}

/// Audit counts for one pure terrain-detail plan.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReviewTerrainPlanCountsV1 {
    pub(crate) mesh_batches: u32,
    pub(crate) vertices: u64,
    pub(crate) triangles: u64,
    pub(crate) snow_caps_added: u32,
    pub(crate) snow_caps_hidden: u32,
    pub(crate) snow_vertical_shell_sides: u32,
    pub(crate) vegetation_transforms: u32,
    pub(crate) vegetation_dust_shells: u32,
    pub(crate) cliff_side_overlays: u32,
    pub(crate) cliff_strata_bands: u32,
    pub(crate) boulders: u32,
    pub(crate) tufts: u32,
    pub(crate) deadwood: u32,
}

/// Complete disposable result for the four terrain-attached review families.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ReviewTerrainDetailPlanV1 {
    pub(crate) mesh_batches: Vec<ReviewTerrainMeshBatchV1>,
    pub(crate) vegetation_batches: Vec<ReviewVegetationBatchV1>,
    pub(crate) prop_placements: Vec<ReviewPropPlacementV1>,
    pub(crate) resolved_snow_surfaces: BTreeSet<TilePos>,
    pub(crate) counts: ReviewTerrainPlanCountsV1,
    pub(crate) plan_hash: u64,
}

/// Failure to build a finite, deterministic terrain-detail projection.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ReviewTerrainPlanError {
    InvalidLevelHeight(f32),
    InvalidSubstrateColor(TilePos),
    InvalidCliffLayers(TilePos),
    DuplicateSurface(TilePos),
    DuplicateVegetation(u64),
    GeometryOverflow,
    DegenerateGeometry,
}

impl fmt::Display for ReviewTerrainPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLevelHeight(value) => {
                write!(
                    formatter,
                    "review terrain level height must be finite and positive, got {value}"
                )
            }
            Self::InvalidSubstrateColor(pos) => {
                write!(
                    formatter,
                    "review terrain substrate color at {pos:?} is not finite RGBA"
                )
            }
            Self::InvalidCliffLayers(pos) => {
                write!(
                    formatter,
                    "review terrain cliff layers at {pos:?} are empty, overlapping, out of order, out of bounds, omit the exposed top, or use invalid RGBA"
                )
            }
            Self::DuplicateSurface(pos) => {
                write!(
                    formatter,
                    "review terrain input repeats exact surface {pos:?}"
                )
            }
            Self::DuplicateVegetation(id) => {
                write!(
                    formatter,
                    "review terrain input repeats vegetation identity {id}"
                )
            }
            Self::GeometryOverflow => {
                formatter.write_str("review terrain mesh exceeds u32 index capacity")
            }
            Self::DegenerateGeometry => {
                formatter.write_str("review terrain mesh produced degenerate geometry")
            }
        }
    }
}

impl std::error::Error for ReviewTerrainPlanError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct MeshBatchKey {
    chunk: (i32, i32),
    role: ReviewTerrainMaterialRoleV1,
    substrate: Option<SubstanceId>,
}

#[derive(Debug)]
struct MeshBatchBuilder {
    chunk: (i32, i32),
    role: ReviewTerrainMaterialRoleV1,
    substrate: Option<SubstanceId>,
    color: [f32; 4],
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uv0: Vec<[f32; 2]>,
    indices: Vec<u32>,
    source_items: u32,
}

impl MeshBatchBuilder {
    fn new(key: MeshBatchKey, color: [f32; 4]) -> Self {
        Self {
            chunk: key.chunk,
            role: key.role,
            substrate: key.substrate,
            color,
            positions: Vec::new(),
            normals: Vec::new(),
            uv0: Vec::new(),
            indices: Vec::new(),
            source_items: 0,
        }
    }

    fn mark_source(&mut self) {
        self.source_items = self.source_items.saturating_add(1);
    }

    fn triangle(
        &mut self,
        positions: [Vec3; 3],
        uvs: [[f32; 2]; 3],
    ) -> Result<(), ReviewTerrainPlanError> {
        let [first, second, third] = positions;
        let normal = (second - first).cross(third - first);
        if !normal.is_finite() || normal.length_squared() <= f32::EPSILON {
            return Err(ReviewTerrainPlanError::DegenerateGeometry);
        }
        let normal = normal.normalize().to_array();
        let start = u32::try_from(self.positions.len())
            .map_err(|_error| ReviewTerrainPlanError::GeometryOverflow)?;
        self.positions
            .extend(positions.map(|position| position.to_array()));
        self.normals.extend([normal; 3]);
        self.uv0.extend(uvs);
        let second_index = start
            .checked_add(1)
            .ok_or(ReviewTerrainPlanError::GeometryOverflow)?;
        let third_index = start
            .checked_add(2)
            .ok_or(ReviewTerrainPlanError::GeometryOverflow)?;
        self.indices.extend([start, second_index, third_index]);
        Ok(())
    }

    fn quad(
        &mut self,
        positions: [Vec3; 4],
        uvs: [[f32; 2]; 4],
    ) -> Result<(), ReviewTerrainPlanError> {
        let [first, second, third, fourth] = positions;
        let [first_uv, second_uv, third_uv, fourth_uv] = uvs;
        self.triangle([first, second, third], [first_uv, second_uv, third_uv])?;
        self.triangle([first, third, fourth], [first_uv, third_uv, fourth_uv])
    }

    fn finish(self) -> ReviewTerrainMeshBatchV1 {
        ReviewTerrainMeshBatchV1 {
            chunk: self.chunk,
            material_role: self.role,
            substrate: self.substrate,
            base_color: self.color,
            positions: self.positions,
            normals: self.normals,
            uv0: self.uv0,
            indices: self.indices,
            source_items: self.source_items,
        }
    }
}

/// Builds all four terrain-attached treatment families without touching world state.
pub(crate) fn plan_review_terrain_details(
    profile: &ReviewWorldDetailProfileV1,
    input: &ReviewTerrainInputV1,
) -> Result<ReviewTerrainDetailPlanV1, ReviewTerrainPlanError> {
    let mut meshes = BTreeMap::<MeshBatchKey, MeshBatchBuilder>::new();
    let mut vegetation = BTreeMap::<(i32, i32), Vec<ReviewVegetationProjectionV1>>::new();
    let mut prop_placements = Vec::new();
    let mut counts = ReviewTerrainPlanCountsV1::default();

    let resolved_snow_surfaces = plan_snow(&profile.snow, input, &mut meshes, &mut counts)?;
    plan_vegetation(
        &profile.alpine_vegetation,
        input,
        &mut vegetation,
        &mut counts,
    );
    plan_cliffs(&profile.cliff_strata, input, &mut meshes, &mut counts)?;
    plan_props(
        &profile.terrain_props,
        input,
        &mut meshes,
        &mut prop_placements,
        &mut counts,
    )?;

    let mesh_batches = meshes
        .into_values()
        .filter(|batch| !batch.indices.is_empty())
        .map(MeshBatchBuilder::finish)
        .collect::<Vec<_>>();
    let vegetation_batches = vegetation
        .into_iter()
        .map(|(chunk, instances)| ReviewVegetationBatchV1 { chunk, instances })
        .collect::<Vec<_>>();

    counts.mesh_batches = saturating_u32(mesh_batches.len());
    counts.vertices = mesh_batches.iter().fold(0_u64, |total, batch| {
        total.saturating_add(saturating_u64(batch.positions.len()))
    });
    counts.triangles = mesh_batches.iter().fold(0_u64, |total, batch| {
        total.saturating_add(saturating_u64(batch.indices.len()) / 3)
    });

    let plan_hash = hash_plan(
        &mesh_batches,
        &vegetation_batches,
        &prop_placements,
        &resolved_snow_surfaces,
        counts,
    );
    Ok(ReviewTerrainDetailPlanV1 {
        mesh_batches,
        vegetation_batches,
        prop_placements,
        resolved_snow_surfaces,
        counts,
        plan_hash,
    })
}

fn plan_snow(
    detail: &SnowDetailV1,
    input: &ReviewTerrainInputV1,
    meshes: &mut BTreeMap<MeshBatchKey, MeshBatchBuilder>,
    counts: &mut ReviewTerrainPlanCountsV1,
) -> Result<BTreeSet<TilePos>, ReviewTerrainPlanError> {
    if matches!(detail, SnowDetailV1::Current) {
        return Ok(input
            .surfaces
            .values()
            .filter_map(|surface| surface.current_snow.then_some(surface.pos))
            .collect());
    }

    let mut desired = BTreeMap::<TilePos, bool>::new();
    for surface in input
        .surfaces
        .values()
        .filter(|surface| surface.exposed_natural)
    {
        let snow = if surface.snow_exception != ReviewSnowExceptionV1::None {
            surface.current_snow
        } else if surface.forced_summit {
            true
        } else {
            surface.pos.level >= snowline_for(detail, input.seed, surface)
        };
        desired.insert(surface.pos, snow);
    }
    if matches!(detail, SnowDetailV1::TerrainAware { .. }) {
        remove_tiny_snow_components(&input.surfaces, &mut desired);
    }

    let shell_height = match detail {
        SnowDetailV1::TerrainAware {
            vertical_shell_height,
        } => *vertical_shell_height,
        SnowDetailV1::Current
        | SnowDetailV1::StraightThreshold { .. }
        | SnowDetailV1::CoherentLine { .. } => 0.0,
    };

    for (pos, target_snow) in &desired {
        let Some(surface) = input.surfaces.get(pos) else {
            continue;
        };
        if *target_snow && !surface.current_snow {
            let key = MeshBatchKey {
                chunk: terrain_chunk_key(pos.coord),
                role: ReviewTerrainMaterialRoleV1::SnowCap,
                substrate: None,
            };
            let batch = mesh_batch(meshes, key, SNOW_COLOR);
            emit_hex_cap(batch, *pos, input.level_height, SNOW_TOP_BIAS)?;
            batch.mark_source();
            counts.snow_caps_added = counts.snow_caps_added.saturating_add(1);
        } else if !*target_snow && surface.current_snow {
            let key = MeshBatchKey {
                chunk: terrain_chunk_key(pos.coord),
                role: ReviewTerrainMaterialRoleV1::SubstrateRestore,
                substrate: Some(surface.substrate),
            };
            let batch = mesh_batch(meshes, key, surface.substrate_color);
            emit_hex_cap(batch, *pos, input.level_height, SNOW_TOP_BIAS * 1.5)?;
            batch.mark_source();
            for side in surface.sides {
                let exposed_height = physical_exposed_height(*pos, side, input.level_height);
                if exposed_height <= f32::EPSILON {
                    continue;
                }
                emit_surface_lip_side(
                    batch,
                    *pos,
                    side.direction,
                    input.level_height.min(exposed_height),
                    input.level_height,
                    SNOW_TOP_BIAS * 1.5,
                )?;
                batch.mark_source();
            }
            counts.snow_caps_hidden = counts.snow_caps_hidden.saturating_add(1);
        }

        if *target_snow && shell_height > f32::EPSILON {
            for side in surface.sides {
                let exposed_height = physical_exposed_height(*pos, side, input.level_height);
                if exposed_height <= f32::EPSILON {
                    continue;
                }
                let key = MeshBatchKey {
                    chunk: terrain_chunk_key(pos.coord),
                    role: ReviewTerrainMaterialRoleV1::SnowCap,
                    substrate: None,
                };
                let batch = mesh_batch(meshes, key, SNOW_COLOR);
                emit_surface_lip_side(
                    batch,
                    *pos,
                    side.direction,
                    shell_height.min(exposed_height),
                    input.level_height,
                    SNOW_TOP_BIAS,
                )?;
                batch.mark_source();
                counts.snow_vertical_shell_sides =
                    counts.snow_vertical_shell_sides.saturating_add(1);
            }
        }
    }
    let mut resolved = input
        .surfaces
        .values()
        .filter(|surface| !surface.exposed_natural && surface.current_snow)
        .map(|surface| surface.pos)
        .collect::<BTreeSet<_>>();
    resolved.extend(
        desired
            .into_iter()
            .filter_map(|(pos, snow)| snow.then_some(pos)),
    );
    Ok(resolved)
}

fn snowline_for(detail: &SnowDetailV1, seed: u64, surface: &ReviewTerrainSurfaceInputV1) -> Level {
    match detail {
        SnowDetailV1::Current => Level::MAX,
        SnowDetailV1::StraightThreshold { level } => i32::from(*level),
        SnowDetailV1::CoherentLine {
            mean_level,
            amplitude_levels,
            correlation_hexes,
        } => i32::from(*mean_level).saturating_add(coherent_level_offset(
            seed,
            b"review-snow-coherent",
            surface.pos.coord,
            *correlation_hexes,
            i32::from(*amplitude_levels),
        )),
        SnowDetailV1::TerrainAware { .. } => {
            let noise = coherent_level_offset(
                seed,
                b"review-snow-terrain-aware",
                surface.pos.coord,
                COHERENT_TERRAIN_CORRELATION_HEXES,
                6,
            );
            let slope = neighbour_relief(surface);
            let slope_offset = match slope {
                0..=2 => -4,
                3..=7 => 0,
                _ => 6,
            };
            let aspect_offset = slope_aspect_offset(surface);
            140_i32
                .saturating_add(noise)
                .saturating_add(slope_offset)
                .saturating_add(aspect_offset)
                .clamp(128, 156)
        }
    }
}

fn neighbour_relief(surface: &ReviewTerrainSurfaceInputV1) -> u32 {
    surface
        .sides
        .iter()
        .filter_map(|side| side.adjacent_surface)
        .map(|adjacent| surface.pos.level.abs_diff(adjacent.level))
        .max()
        .unwrap_or(0)
}

fn slope_aspect_offset(surface: &ReviewTerrainSurfaceInputV1) -> i32 {
    let mut steepest = None::<(i32, i32)>;
    for side in surface.sides {
        let Some(adjacent) = side.adjacent_surface else {
            continue;
        };
        let drop = surface.pos.level.saturating_sub(adjacent.level);
        if drop <= 0 || steepest.is_some_and(|(best, _)| best >= drop) {
            continue;
        }
        let axial_y = adjacent.coord.y().saturating_sub(surface.pos.coord.y());
        steepest = Some((drop, axial_y));
    }
    match steepest.map(|(_, axial_y)| axial_y) {
        Some(axial_y) if axial_y < 0 => -4,
        Some(axial_y) if axial_y > 0 => 4,
        _ => 0,
    }
}

fn remove_tiny_snow_components(
    surfaces: &BTreeMap<TilePos, ReviewTerrainSurfaceInputV1>,
    desired: &mut BTreeMap<TilePos, bool>,
) {
    let mut remaining = desired
        .iter()
        .filter_map(|(pos, snow)| snow.then_some(*pos))
        .collect::<BTreeSet<_>>();
    while let Some(root) = remaining.first().copied() {
        let mut stack = vec![root];
        let mut component = Vec::new();
        remaining.remove(&root);
        while let Some(pos) = stack.pop() {
            component.push(pos);
            let Some(surface) = surfaces.get(&pos) else {
                continue;
            };
            for adjacent in surface
                .sides
                .iter()
                .filter_map(|side| side.adjacent_surface)
            {
                if desired.get(&adjacent).copied().unwrap_or(false) && remaining.remove(&adjacent) {
                    stack.push(adjacent);
                }
            }
        }
        let forced = component.iter().any(|pos| {
            surfaces.get(pos).is_some_and(|surface| {
                surface.forced_summit || surface.snow_exception != ReviewSnowExceptionV1::None
            })
        });
        if component.len() < 3 && !forced {
            for pos in component {
                if let Some(snow) = desired.get_mut(&pos) {
                    *snow = false;
                }
            }
        }
    }
}

fn plan_vegetation(
    detail: &AlpineVegetationDetailV1,
    input: &ReviewTerrainInputV1,
    batches: &mut BTreeMap<(i32, i32), Vec<ReviewVegetationProjectionV1>>,
    counts: &mut ReviewTerrainPlanCountsV1,
) {
    if matches!(detail, AlpineVegetationDetailV1::Current) {
        return;
    }

    for vegetation in input.vegetation.values() {
        let (scale, scale_changed) = vegetation_scale(detail, input.seed, vegetation.stable_id);
        let crown_dust = vegetation
            .snow_dust_eligible
            .then(|| vegetation_dust(detail))
            .flatten();
        if scale_changed {
            counts.vegetation_transforms = counts.vegetation_transforms.saturating_add(1);
        }
        if crown_dust.is_some() {
            counts.vegetation_dust_shells = counts.vegetation_dust_shells.saturating_add(1);
        }
        batches
            .entry(terrain_chunk_key(vegetation.root.coord))
            .or_default()
            .push(ReviewVegetationProjectionV1 {
                stable_id: vegetation.stable_id,
                root: vegetation.root,
                render_child_scale: scale,
                crown_dust,
            });
    }
}

fn vegetation_scale(
    detail: &AlpineVegetationDetailV1,
    seed: u64,
    stable_id: u64,
) -> ([f32; 3], bool) {
    let bounds = match detail {
        AlpineVegetationDetailV1::ScaleJitter {
            horizontal_min,
            horizontal_max,
            vertical_min,
            vertical_max,
        }
        | AlpineVegetationDetailV1::ScaleJitterWithDust {
            horizontal_min,
            horizontal_max,
            vertical_min,
            vertical_max,
            ..
        } => Some((
            *horizontal_min,
            *horizontal_max,
            *vertical_min,
            *vertical_max,
        )),
        AlpineVegetationDetailV1::Current | AlpineVegetationDetailV1::CrownSnowDust { .. } => None,
    };
    let Some((horizontal_min, horizontal_max, vertical_min, vertical_max)) = bounds else {
        return ([1.0; 3], false);
    };
    let horizontal = lerp(
        horizontal_min,
        horizontal_max,
        unit_sample(input_hash(seed, b"review-vegetation-horizontal", stable_id)),
    );
    let vertical = lerp(
        vertical_min,
        vertical_max,
        unit_sample(input_hash(seed, b"review-vegetation-vertical", stable_id)),
    );
    ([horizontal, vertical, horizontal], true)
}

fn vegetation_dust(detail: &AlpineVegetationDetailV1) -> Option<ReviewCrownDustV1> {
    let parameters = match detail {
        AlpineVegetationDetailV1::CrownSnowDust {
            upper_fraction,
            shell_height,
        }
        | AlpineVegetationDetailV1::ScaleJitterWithDust {
            upper_fraction,
            shell_height,
            ..
        } => Some((*upper_fraction, *shell_height)),
        AlpineVegetationDetailV1::Current | AlpineVegetationDetailV1::ScaleJitter { .. } => None,
    };
    parameters.map(|(upper_fraction, shell_height)| ReviewCrownDustV1 {
        upper_fraction,
        shell_height,
        color: SNOW_COLOR,
    })
}

fn plan_cliffs(
    detail: &CliffStrataDetailV1,
    input: &ReviewTerrainInputV1,
    meshes: &mut BTreeMap<MeshBatchKey, MeshBatchBuilder>,
    counts: &mut ReviewTerrainPlanCountsV1,
) -> Result<(), ReviewTerrainPlanError> {
    if matches!(detail, CliffStrataDetailV1::Current) {
        return Ok(());
    }

    let value_delta = match detail {
        CliffStrataDetailV1::SideValue { value_delta }
        | CliffStrataDetailV1::StrataWithValue { value_delta, .. } => Some(*value_delta),
        CliffStrataDetailV1::Current | CliffStrataDetailV1::Strata { .. } => None,
    };
    let strata = match detail {
        CliffStrataDetailV1::Strata {
            period_levels,
            width_levels,
            contrast,
            phase_variation_levels,
            correlation_hexes,
        }
        | CliffStrataDetailV1::StrataWithValue {
            period_levels,
            width_levels,
            contrast,
            phase_variation_levels,
            correlation_hexes,
            ..
        } => Some((
            *period_levels,
            *width_levels,
            *contrast,
            *phase_variation_levels,
            *correlation_hexes,
        )),
        CliffStrataDetailV1::Current | CliffStrataDetailV1::SideValue { .. } => None,
    };

    for surface in input
        .surfaces
        .values()
        .filter(|surface| surface.exposed_natural)
    {
        for side in surface.sides {
            let drop_levels = surface
                .pos
                .level
                .saturating_add(1)
                .saturating_sub(side.exposed_bottom_level);
            if drop_levels < 6 {
                continue;
            }
            let top_level = surface.pos.level.saturating_add(1);
            let bands = if let Some((period, width, _, phase_variation, correlation)) = strata {
                let phase = coherent_level_offset(
                    input.seed,
                    b"review-cliff-strata-phase",
                    surface.pos.coord,
                    correlation.max(1),
                    i32::from(phase_variation),
                );
                strata_bands(
                    side.exposed_bottom_level,
                    top_level,
                    i32::from(period),
                    i32::from(width),
                    phase,
                )
            } else {
                Vec::new()
            };

            if let Some(delta) = value_delta {
                let value_multiplier = (1.0 + delta).clamp(0.0, 1.0);
                // In the combined treatment, emit only the complement of the
                // strata bands. The darker opaque band shells then meet these
                // segments exactly instead of relying on layered alpha blending.
                let value_regions = if strata.is_some() {
                    interval_complement(side.exposed_bottom_level, top_level, &bands)
                } else {
                    vec![(side.exposed_bottom_level, top_level)]
                };
                let mut emitted = false;
                for layer in &surface.cliff_layers {
                    let layer_bottom = layer.bottom_level.max(side.exposed_bottom_level);
                    let layer_top = layer.top_level.min(top_level);
                    if layer_bottom >= layer_top {
                        continue;
                    }
                    let value_color = cliff_relative_color(layer.substrate_color, value_multiplier);
                    let key = MeshBatchKey {
                        chunk: terrain_chunk_key(surface.pos.coord),
                        role: ReviewTerrainMaterialRoleV1::CliffValue,
                        substrate: Some(layer.substrate),
                    };
                    for (bottom, top) in
                        interval_intersections(layer_bottom, layer_top, &value_regions)
                    {
                        let batch = mesh_batch(meshes, key, value_color);
                        emit_cliff_quad(
                            batch,
                            surface.pos.coord,
                            side.direction,
                            bottom,
                            top,
                            input.level_height,
                        )?;
                        batch.mark_source();
                        emitted = true;
                    }
                }
                if emitted {
                    counts.cliff_side_overlays = counts.cliff_side_overlays.saturating_add(1);
                }
            }

            if let Some((_, _, contrast, _, _)) = strata {
                let side_multiplier =
                    value_delta.map_or(1.0, |delta| (1.0 + delta).clamp(0.0, 1.0));
                let band_multiplier = side_multiplier * (1.0 - contrast).clamp(0.0, 1.0);
                for layer in &surface.cliff_layers {
                    let layer_bottom = layer.bottom_level.max(side.exposed_bottom_level);
                    let layer_top = layer.top_level.min(top_level);
                    if layer_bottom >= layer_top {
                        continue;
                    }
                    let band_color = cliff_relative_color(layer.substrate_color, band_multiplier);
                    let key = MeshBatchKey {
                        chunk: terrain_chunk_key(surface.pos.coord),
                        role: ReviewTerrainMaterialRoleV1::CliffStrata,
                        substrate: Some(layer.substrate),
                    };
                    for (bottom, top) in interval_intersections(layer_bottom, layer_top, &bands) {
                        let batch = mesh_batch(meshes, key, band_color);
                        emit_cliff_quad(
                            batch,
                            surface.pos.coord,
                            side.direction,
                            bottom,
                            top,
                            input.level_height,
                        )?;
                        batch.mark_source();
                        counts.cliff_strata_bands = counts.cliff_strata_bands.saturating_add(1);
                    }
                }
            }
        }
    }
    Ok(())
}

fn cliff_relative_color(substrate: [f32; 4], value_multiplier: f32) -> [f32; 4] {
    let [red, green, blue, _alpha] = substrate;
    [
        (red * value_multiplier).clamp(0.0, 1.0),
        (green * value_multiplier).clamp(0.0, 1.0),
        (blue * value_multiplier).clamp(0.0, 1.0),
        1.0,
    ]
}

fn interval_complement(
    bottom: Level,
    top: Level,
    occupied: &[(Level, Level)],
) -> Vec<(Level, Level)> {
    let mut complement = Vec::new();
    let mut cursor = bottom;
    for &(occupied_bottom, occupied_top) in occupied {
        if cursor < occupied_bottom {
            complement.push((cursor, occupied_bottom));
        }
        cursor = cursor.max(occupied_top);
    }
    if cursor < top {
        complement.push((cursor, top));
    }
    complement
}

fn interval_intersections(
    bottom: Level,
    top: Level,
    intervals: &[(Level, Level)],
) -> Vec<(Level, Level)> {
    intervals
        .iter()
        .filter_map(|&(interval_bottom, interval_top)| {
            let intersection_bottom = bottom.max(interval_bottom);
            let intersection_top = top.min(interval_top);
            (intersection_bottom < intersection_top)
                .then_some((intersection_bottom, intersection_top))
        })
        .collect()
}

fn strata_bands(
    bottom: Level,
    top: Level,
    period: i32,
    width: i32,
    phase: i32,
) -> Vec<(Level, Level)> {
    if period <= 0 || width <= 0 || bottom >= top {
        return Vec::new();
    }
    let mut bands = Vec::new();
    let mut level = bottom;
    let mut band_start = None;
    while level < top {
        let in_band = level.saturating_add(phase).rem_euclid(period) < width;
        match (band_start, in_band) {
            (None, true) => band_start = Some(level),
            (Some(start), false) => {
                bands.push((start, level));
                band_start = None;
            }
            _ => {}
        }
        level = level.saturating_add(1);
    }
    if let Some(start) = band_start {
        bands.push((start, top));
    }
    bands
}

fn plan_props(
    detail: &TerrainPropsDetailV1,
    input: &ReviewTerrainInputV1,
    meshes: &mut BTreeMap<MeshBatchKey, MeshBatchBuilder>,
    placements: &mut Vec<ReviewPropPlacementV1>,
    counts: &mut ReviewTerrainPlanCountsV1,
) -> Result<(), ReviewTerrainPlanError> {
    match detail {
        TerrainPropsDetailV1::Current => return Ok(()),
        TerrainPropsDetailV1::Boulders { density, cap } => {
            for surface in
                select_prop_surfaces(input, *density, usize::from(*cap), b"review-props-boulders")
            {
                placements.push(ReviewPropPlacementV1 {
                    root: surface.pos,
                    kind: ReviewPropKindV1::Boulder,
                    cluster: None,
                });
            }
        }
        TerrainPropsDetailV1::GrassLitter { density, cap } => {
            for surface in select_prop_surfaces(
                input,
                *density,
                usize::from(*cap),
                b"review-props-grass-litter",
            ) {
                let kind = if surface_hash(input.seed, b"review-props-litter-kind", surface.pos, 0)
                    % 5
                    == 0
                {
                    ReviewPropKindV1::Deadwood
                } else {
                    ReviewPropKindV1::Tuft
                };
                placements.push(ReviewPropPlacementV1 {
                    root: surface.pos,
                    kind,
                    cluster: None,
                });
            }
        }
        TerrainPropsDetailV1::Mixed {
            boulder_density,
            tuft_density,
            deadwood_density,
            cap,
        } => {
            let total_density = boulder_density + tuft_density + deadwood_density;
            for surface in select_prop_surfaces(
                input,
                total_density,
                usize::from(*cap),
                b"review-props-mixed",
            ) {
                let choice = unit_sample(surface_hash(
                    input.seed,
                    b"review-props-mixed-kind",
                    surface.pos,
                    0,
                )) * total_density;
                let kind = if choice < *boulder_density {
                    ReviewPropKindV1::Boulder
                } else if choice < boulder_density + tuft_density {
                    ReviewPropKindV1::Tuft
                } else {
                    ReviewPropKindV1::Deadwood
                };
                placements.push(ReviewPropPlacementV1 {
                    root: surface.pos,
                    kind,
                    cluster: None,
                });
            }
        }
        TerrainPropsDetailV1::Clustered {
            center_density,
            pieces_min,
            pieces_max,
            cap,
        } => plan_clustered_props(
            input,
            *center_density,
            *pieces_min,
            *pieces_max,
            usize::from(*cap),
            placements,
        ),
    }

    for placement in placements.iter().copied() {
        let role = match placement.kind {
            ReviewPropKindV1::Boulder => ReviewTerrainMaterialRoleV1::Boulder,
            ReviewPropKindV1::Tuft => ReviewTerrainMaterialRoleV1::Tuft,
            ReviewPropKindV1::Deadwood => ReviewTerrainMaterialRoleV1::Deadwood,
        };
        let color = match placement.kind {
            ReviewPropKindV1::Boulder => BOULDER_COLOR,
            ReviewPropKindV1::Tuft => TUFT_COLOR,
            ReviewPropKindV1::Deadwood => DEADWOOD_COLOR,
        };
        let key = MeshBatchKey {
            chunk: terrain_chunk_key(placement.root.coord),
            role,
            substrate: None,
        };
        let batch = mesh_batch(meshes, key, color);
        emit_prop_geometry(batch, placement, input.seed, input.level_height)?;
        batch.mark_source();
        match placement.kind {
            ReviewPropKindV1::Boulder => counts.boulders = counts.boulders.saturating_add(1),
            ReviewPropKindV1::Tuft => counts.tufts = counts.tufts.saturating_add(1),
            ReviewPropKindV1::Deadwood => counts.deadwood = counts.deadwood.saturating_add(1),
        }
    }
    Ok(())
}

fn select_prop_surfaces<'a>(
    input: &'a ReviewTerrainInputV1,
    density: f32,
    cap: usize,
    domain: &[u8],
) -> Vec<&'a ReviewTerrainSurfaceInputV1> {
    let mut selected = input
        .surfaces
        .values()
        .filter(|surface| surface.exposed_natural && !surface.prop_exclusions.any())
        .filter_map(|surface| {
            let rank = surface_hash(input.seed, domain, surface.pos, 0);
            (unit_sample(rank) < density).then_some((rank, surface.pos, surface))
        })
        .collect::<Vec<_>>();
    selected.sort_by_key(|(rank, pos, _)| (*rank, *pos));
    selected
        .into_iter()
        .take(cap)
        .map(|(_, _, surface)| surface)
        .collect()
}

fn plan_clustered_props(
    input: &ReviewTerrainInputV1,
    center_density: f32,
    pieces_min: u8,
    pieces_max: u8,
    cap: usize,
    placements: &mut Vec<ReviewPropPlacementV1>,
) {
    let eligible = input
        .surfaces
        .values()
        .filter(|surface| surface.exposed_natural && !surface.prop_exclusions.any())
        .collect::<Vec<_>>();
    let mut centers = eligible
        .iter()
        .copied()
        .filter_map(|surface| {
            let rank = surface_hash(input.seed, b"review-props-cluster-center", surface.pos, 0);
            (unit_sample(rank) < center_density).then_some((rank, surface))
        })
        .collect::<Vec<_>>();
    centers.sort_by_key(|(rank, surface)| (*rank, surface.pos));

    let mut used = BTreeSet::new();
    for (_, center) in centers {
        let remaining_capacity = cap.saturating_sub(placements.len());
        if remaining_capacity < usize::from(pieces_min) {
            break;
        }
        let span = pieces_max.saturating_sub(pieces_min).saturating_add(1);
        let extra = if span == 0 {
            0
        } else {
            u8::try_from(
                surface_hash(input.seed, b"review-props-cluster-size", center.pos, 0)
                    % u64::from(span),
            )
            .unwrap_or(0)
        };
        let requested = usize::from(pieces_min.saturating_add(extra));
        let cluster_id = surface_hash(input.seed, b"review-props-cluster-id", center.pos, 0);
        let mut neighbours = eligible
            .iter()
            .copied()
            .filter(|surface| surface.pos.coord.distance(center.pos.coord) <= 2)
            .filter(|surface| !used.contains(&surface.pos))
            .map(|surface| {
                (
                    surface_hash(
                        input.seed,
                        b"review-props-cluster-member",
                        surface.pos,
                        cluster_id,
                    ),
                    surface,
                )
            })
            .collect::<Vec<_>>();
        neighbours.sort_by_key(|(rank, surface)| (*rank, surface.pos));
        let selected = neighbours
            .into_iter()
            .take(requested.min(remaining_capacity))
            .collect::<Vec<_>>();
        if selected.len() < usize::from(pieces_min) {
            continue;
        }
        for (_, surface) in selected {
            used.insert(surface.pos);
            let kind_hash = surface_hash(
                input.seed,
                b"review-props-cluster-kind",
                surface.pos,
                cluster_id,
            ) % 5;
            let kind = match kind_hash {
                0 => ReviewPropKindV1::Boulder,
                4 => ReviewPropKindV1::Deadwood,
                _ => ReviewPropKindV1::Tuft,
            };
            placements.push(ReviewPropPlacementV1 {
                root: surface.pos,
                kind,
                cluster: Some(cluster_id),
            });
        }
    }
}

fn mesh_batch<'a>(
    batches: &'a mut BTreeMap<MeshBatchKey, MeshBatchBuilder>,
    key: MeshBatchKey,
    color: [f32; 4],
) -> &'a mut MeshBatchBuilder {
    batches
        .entry(key)
        .or_insert_with(|| MeshBatchBuilder::new(key, color))
}

fn emit_hex_cap(
    batch: &mut MeshBatchBuilder,
    pos: TilePos,
    level_height: f32,
    y_bias: f32,
) -> Result<(), ReviewTerrainPlanError> {
    let y = level_boundary_world(pos.level.saturating_add(1), level_height) + y_bias;
    let centre = pos.coord.to_world(y);
    let [north, north_west, south_west, south, south_east, north_east] = terrain_hex_corners();
    for [first, second] in [
        [north, north_west],
        [north_west, south_west],
        [south_west, south],
        [south, south_east],
        [south_east, north_east],
        [north_east, north],
    ] {
        batch.triangle(
            [centre, centre + first, centre + second],
            [[0.5, 0.5], cap_uv(first), cap_uv(second)],
        )?;
    }
    Ok(())
}

fn emit_surface_lip_side(
    batch: &mut MeshBatchBuilder,
    pos: TilePos,
    direction: HexCoord,
    shell_height: f32,
    level_height: f32,
    top_bias: f32,
) -> Result<(), ReviewTerrainPlanError> {
    let top = level_boundary_world(pos.level.saturating_add(1), level_height) + top_bias;
    let bottom = top - shell_height;
    let (edge, normal) = terrain_side_toward(pos.coord, direction);
    emit_vertical_quad(batch, pos.coord, edge, normal, bottom, top, OVERLAY_BIAS)
}

fn emit_cliff_quad(
    batch: &mut MeshBatchBuilder,
    coord: HexCoord,
    direction: HexCoord,
    bottom_level: Level,
    top_level: Level,
    level_height: f32,
) -> Result<(), ReviewTerrainPlanError> {
    let bottom = level_boundary_world(bottom_level, level_height);
    let top = level_boundary_world(top_level, level_height);
    let (edge, normal) = terrain_side_toward(coord, direction);
    // Exact coplanarity preserves the existing cliff silhouette. The render adapter
    // resolves depth ordering with material depth bias rather than moving vertices.
    emit_vertical_quad(batch, coord, edge, normal, bottom, top, 0.0)
}

fn emit_vertical_quad(
    batch: &mut MeshBatchBuilder,
    coord: HexCoord,
    edge: [Vec3; 2],
    normal: Vec3,
    bottom: f32,
    top: f32,
    normal_bias: f32,
) -> Result<(), ReviewTerrainPlanError> {
    if !bottom.is_finite() || !top.is_finite() || top - bottom <= f32::EPSILON {
        return Err(ReviewTerrainPlanError::DegenerateGeometry);
    }
    let [first, second] = edge;
    let base = coord.to_world(0.0) + normal * normal_bias;
    batch.quad(
        [
            base + first + Vec3::Y * bottom,
            base + second + Vec3::Y * bottom,
            base + second + Vec3::Y * top,
            base + first + Vec3::Y * top,
        ],
        [[0.0, bottom], [1.0, bottom], [1.0, top], [0.0, top]],
    )
}

fn physical_exposed_height(pos: TilePos, side: ReviewTerrainSideInputV1, level_height: f32) -> f32 {
    let levels = pos
        .level
        .saturating_add(1)
        .saturating_sub(side.exposed_bottom_level)
        .max(0);
    level_boundary_world(levels, level_height)
}

fn terrain_hex_corners() -> [Vec3; 6] {
    let radius = hex_core::config::HEX_CIRCUMRADIUS;
    let inradius = 0.5 * hex_core::config::HEX_SMALL_DIAMETER;
    [
        Vec3::new(0.0, 0.0, -radius),
        Vec3::new(-inradius, 0.0, -0.5 * radius),
        Vec3::new(-inradius, 0.0, 0.5 * radius),
        Vec3::new(0.0, 0.0, radius),
        Vec3::new(inradius, 0.0, 0.5 * radius),
        Vec3::new(inradius, 0.0, -0.5 * radius),
    ]
}

fn terrain_hex_sides() -> [([Vec3; 2], Vec3); 6] {
    let [north, north_west, south_west, south, south_east, north_east] = terrain_hex_corners();
    [
        ([south_east, north_east], Vec3::X),
        ([south, south_east], Vec3::new(0.5, 0.0, 0.866_025_4)),
        ([south_west, south], Vec3::new(-0.5, 0.0, 0.866_025_4)),
        ([north_west, south_west], Vec3::NEG_X),
        ([north, north_west], Vec3::new(-0.5, 0.0, -0.866_025_4)),
        ([north_east, north], Vec3::new(0.5, 0.0, -0.866_025_4)),
    ]
}

fn terrain_side_toward(coord: HexCoord, direction: HexCoord) -> ([Vec3; 2], Vec3) {
    let delta = direction.to_world(0.0) - coord.to_world(0.0);
    let horizontal = Vec3::new(delta.x, 0.0, delta.z).normalize_or_zero();
    let mut best = None::<(f32, [Vec3; 2], Vec3)>;
    for (edge, normal) in terrain_hex_sides() {
        let score = normal.dot(horizontal);
        if best.is_none_or(|(best_score, _, _)| score > best_score) {
            best = Some((score, edge, normal));
        }
    }
    best.map_or_else(
        || {
            let [(edge, normal), ..] = terrain_hex_sides();
            (edge, normal)
        },
        |(_, edge, normal)| (edge, normal),
    )
}

fn cap_uv(corner: Vec3) -> [f32; 2] {
    let radius = hex_core::config::HEX_CIRCUMRADIUS;
    [
        0.5 + corner.x / (2.0 * radius),
        0.5 + corner.z / (2.0 * radius),
    ]
}

#[expect(
    clippy::cast_precision_loss,
    reason = "review levels remain far below the range where i32-to-f32 loses a visible step"
)]
fn level_boundary_world(level: Level, level_height: f32) -> f32 {
    (level as f32) * level_height
}

fn emit_prop_geometry(
    batch: &mut MeshBatchBuilder,
    placement: ReviewPropPlacementV1,
    seed: u64,
    level_height: f32,
) -> Result<(), ReviewTerrainPlanError> {
    let base_y = level_boundary_world(placement.root.level.saturating_add(1), level_height)
        + OVERLAY_BIAS * 2.0;
    let centre = placement.root.coord.to_world(base_y)
        + prop_offset(seed, placement.root, placement.cluster.unwrap_or(0));
    match placement.kind {
        ReviewPropKindV1::Boulder => emit_boulder(batch, centre, seed, placement),
        ReviewPropKindV1::Tuft => emit_tuft(batch, centre, seed, placement),
        ReviewPropKindV1::Deadwood => emit_deadwood(batch, centre, seed, placement),
    }
}

fn emit_boulder(
    batch: &mut MeshBatchBuilder,
    centre: Vec3,
    seed: u64,
    placement: ReviewPropPlacementV1,
) -> Result<(), ReviewTerrainPlanError> {
    let scale = lerp(
        0.22,
        0.42,
        unit_sample(surface_hash(
            seed,
            b"review-prop-boulder-scale",
            placement.root,
            placement.cluster.unwrap_or(0),
        )),
    );
    let height = scale
        * lerp(
            0.55,
            0.95,
            unit_sample(surface_hash(
                seed,
                b"review-prop-boulder-height",
                placement.root,
                placement.cluster.unwrap_or(0),
            )),
        );
    let apex_offset = Vec3::new(
        (unit_sample(surface_hash(
            seed,
            b"review-prop-boulder-apex-x",
            placement.root,
            0,
        )) - 0.5)
            * scale
            * 0.35,
        height,
        (unit_sample(surface_hash(
            seed,
            b"review-prop-boulder-apex-z",
            placement.root,
            0,
        )) - 0.5)
            * scale
            * 0.35,
    );
    let apex = centre + apex_offset;
    let [north, north_west, south_west, south, south_east, north_east] =
        terrain_hex_corners().map(|corner| centre + corner * scale * 0.55);
    for [first, second] in [
        [north, north_west],
        [north_west, south_west],
        [south_west, south],
        [south, south_east],
        [south_east, north_east],
        [north_east, north],
    ] {
        batch.triangle([apex, first, second], [[0.5, 1.0], [0.0, 0.0], [1.0, 0.0]])?;
    }
    Ok(())
}

fn emit_tuft(
    batch: &mut MeshBatchBuilder,
    centre: Vec3,
    seed: u64,
    placement: ReviewPropPlacementV1,
) -> Result<(), ReviewTerrainPlanError> {
    let height = lerp(
        0.20,
        0.39,
        unit_sample(surface_hash(
            seed,
            b"review-prop-tuft-height",
            placement.root,
            placement.cluster.unwrap_or(0),
        )),
    );
    let width = height * 0.22;
    let yaw = surface_hash(seed, b"review-prop-tuft-yaw", placement.root, 0) % 6;
    for offset in [0_u64, 2, 4] {
        let direction = planar_direction(yaw.saturating_add(offset));
        let perpendicular = Vec3::new(-direction.z, 0.0, direction.x);
        let first = centre - perpendicular * width;
        let second = centre + perpendicular * width;
        let top = centre + Vec3::Y * height + direction * width * 0.45;
        batch.triangle([first, second, top], [[0.0, 0.0], [1.0, 0.0], [0.5, 1.0]])?;
        batch.triangle([second, first, top], [[1.0, 0.0], [0.0, 0.0], [0.5, 1.0]])?;
    }
    Ok(())
}

fn emit_deadwood(
    batch: &mut MeshBatchBuilder,
    centre: Vec3,
    seed: u64,
    placement: ReviewPropPlacementV1,
) -> Result<(), ReviewTerrainPlanError> {
    let length = lerp(
        0.35,
        0.68,
        unit_sample(surface_hash(
            seed,
            b"review-prop-deadwood-length",
            placement.root,
            placement.cluster.unwrap_or(0),
        )),
    );
    let half_width = length * 0.075;
    let half_height = length * 0.055;
    let direction = planar_direction(surface_hash(
        seed,
        b"review-prop-deadwood-yaw",
        placement.root,
        0,
    ));
    let side = Vec3::new(-direction.z, 0.0, direction.x);
    let low = centre - direction * (length * 0.5) + Vec3::Y * half_height;
    let high = centre + direction * (length * 0.5) + Vec3::Y * half_height;
    let a = low - side * half_width - Vec3::Y * half_height;
    let b = low + side * half_width - Vec3::Y * half_height;
    let c = low + side * half_width + Vec3::Y * half_height;
    let d = low - side * half_width + Vec3::Y * half_height;
    let e = high - side * half_width - Vec3::Y * half_height;
    let f = high + side * half_width - Vec3::Y * half_height;
    let g = high + side * half_width + Vec3::Y * half_height;
    let h = high - side * half_width + Vec3::Y * half_height;
    for face in [
        [a, b, c, d],
        [e, h, g, f],
        [a, d, h, e],
        [b, f, g, c],
        [d, c, g, h],
        [a, e, f, b],
    ] {
        batch.quad(face, [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]])?;
    }
    Ok(())
}

fn prop_offset(seed: u64, root: TilePos, salt: u64) -> Vec3 {
    let x = unit_sample(surface_hash(seed, b"review-prop-offset-x", root, salt)) - 0.5;
    let z = unit_sample(surface_hash(seed, b"review-prop-offset-z", root, salt)) - 0.5;
    Vec3::new(x * 0.42, 0.0, z * 0.42)
}

fn planar_direction(value: u64) -> Vec3 {
    match value % 6 {
        0 => Vec3::X,
        1 => Vec3::new(0.5, 0.0, 0.866_025_4),
        2 => Vec3::new(-0.5, 0.0, 0.866_025_4),
        3 => Vec3::NEG_X,
        4 => Vec3::new(-0.5, 0.0, -0.866_025_4),
        _ => Vec3::new(0.5, 0.0, -0.866_025_4),
    }
}

fn surface_hash(seed: u64, domain: &[u8], pos: TilePos, salt: u64) -> u64 {
    let mut bytes = Vec::with_capacity(domain.len().saturating_add(28));
    bytes.extend_from_slice(domain);
    bytes.extend_from_slice(&pos.coord.x().to_le_bytes());
    bytes.extend_from_slice(&pos.coord.y().to_le_bytes());
    bytes.extend_from_slice(&pos.coord.z().to_le_bytes());
    bytes.extend_from_slice(&pos.level.to_le_bytes());
    bytes.extend_from_slice(&salt.to_le_bytes());
    xxh3_64_with_seed(&bytes, seed)
}

fn input_hash(seed: u64, domain: &[u8], stable_id: u64) -> u64 {
    let mut bytes = Vec::with_capacity(domain.len().saturating_add(8));
    bytes.extend_from_slice(domain);
    bytes.extend_from_slice(&stable_id.to_le_bytes());
    xxh3_64_with_seed(&bytes, seed)
}

fn unit_sample(value: u64) -> f32 {
    let sample = u16::try_from(value >> 48).unwrap_or(0);
    f32::from(sample) / f32::from(u16::MAX)
}

fn lerp(min: f32, max: f32, amount: f32) -> f32 {
    min + (max - min) * amount
}

fn saturating_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn saturating_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn hash_plan(
    mesh_batches: &[ReviewTerrainMeshBatchV1],
    vegetation_batches: &[ReviewVegetationBatchV1],
    prop_placements: &[ReviewPropPlacementV1],
    resolved_snow_surfaces: &BTreeSet<TilePos>,
    counts: ReviewTerrainPlanCountsV1,
) -> u64 {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"review-terrain-detail-plan-v1");
    write_u64(&mut bytes, saturating_u64(mesh_batches.len()));
    for batch in mesh_batches {
        write_i32(&mut bytes, batch.chunk.0);
        write_i32(&mut bytes, batch.chunk.1);
        bytes.push(material_role_tag(batch.material_role));
        write_u16(
            &mut bytes,
            batch.substrate.map_or(u16::MAX, |substance| substance.0),
        );
        for component in batch.base_color {
            write_f32(&mut bytes, component);
        }
        write_u64(&mut bytes, saturating_u64(batch.positions.len()));
        for position in &batch.positions {
            for component in *position {
                write_f32(&mut bytes, component);
            }
        }
        for normal in &batch.normals {
            for component in *normal {
                write_f32(&mut bytes, component);
            }
        }
        for uv in &batch.uv0 {
            for component in *uv {
                write_f32(&mut bytes, component);
            }
        }
        write_u64(&mut bytes, saturating_u64(batch.indices.len()));
        for index in &batch.indices {
            write_u32(&mut bytes, *index);
        }
        write_u32(&mut bytes, batch.source_items);
    }

    write_u64(&mut bytes, saturating_u64(vegetation_batches.len()));
    for batch in vegetation_batches {
        write_i32(&mut bytes, batch.chunk.0);
        write_i32(&mut bytes, batch.chunk.1);
        write_u64(&mut bytes, saturating_u64(batch.instances.len()));
        for instance in &batch.instances {
            write_u64(&mut bytes, instance.stable_id);
            write_tile_pos(&mut bytes, instance.root);
            for component in instance.render_child_scale {
                write_f32(&mut bytes, component);
            }
            if let Some(dust) = instance.crown_dust {
                bytes.push(1);
                write_f32(&mut bytes, dust.upper_fraction);
                write_f32(&mut bytes, dust.shell_height);
                for component in dust.color {
                    write_f32(&mut bytes, component);
                }
            } else {
                bytes.push(0);
            }
        }
    }

    write_u64(&mut bytes, saturating_u64(prop_placements.len()));
    for placement in prop_placements {
        write_tile_pos(&mut bytes, placement.root);
        bytes.push(prop_kind_tag(placement.kind));
        write_u64(&mut bytes, placement.cluster.unwrap_or(u64::MAX));
    }
    write_u64(&mut bytes, saturating_u64(resolved_snow_surfaces.len()));
    for surface in resolved_snow_surfaces {
        write_tile_pos(&mut bytes, *surface);
    }
    for value in [
        counts.mesh_batches,
        counts.snow_caps_added,
        counts.snow_caps_hidden,
        counts.snow_vertical_shell_sides,
        counts.vegetation_transforms,
        counts.vegetation_dust_shells,
        counts.cliff_side_overlays,
        counts.cliff_strata_bands,
        counts.boulders,
        counts.tufts,
        counts.deadwood,
    ] {
        write_u32(&mut bytes, value);
    }
    write_u64(&mut bytes, counts.vertices);
    write_u64(&mut bytes, counts.triangles);
    xxh3_64(&bytes)
}

fn material_role_tag(role: ReviewTerrainMaterialRoleV1) -> u8 {
    match role {
        ReviewTerrainMaterialRoleV1::SnowCap => 0,
        ReviewTerrainMaterialRoleV1::SubstrateRestore => 1,
        ReviewTerrainMaterialRoleV1::CliffValue => 2,
        ReviewTerrainMaterialRoleV1::CliffStrata => 3,
        ReviewTerrainMaterialRoleV1::Boulder => 4,
        ReviewTerrainMaterialRoleV1::Tuft => 5,
        ReviewTerrainMaterialRoleV1::Deadwood => 6,
    }
}

fn prop_kind_tag(kind: ReviewPropKindV1) -> u8 {
    match kind {
        ReviewPropKindV1::Boulder => 0,
        ReviewPropKindV1::Tuft => 1,
        ReviewPropKindV1::Deadwood => 2,
    }
}

fn write_tile_pos(bytes: &mut Vec<u8>, pos: TilePos) {
    write_i32(bytes, pos.coord.x());
    write_i32(bytes, pos.coord.y());
    write_i32(bytes, pos.coord.z());
    write_i32(bytes, pos.level);
}

fn write_f32(bytes: &mut Vec<u8>, value: f32) {
    write_u32(bytes, value.to_bits());
}

fn write_i32(bytes: &mut Vec<u8>, value: i32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn write_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn write_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn write_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEVEL_HEIGHT: f32 = 0.35;

    fn terrain_level(coord: HexCoord) -> Level {
        let base = 121_i32.saturating_add(
            coord
                .x()
                .saturating_mul(7)
                .saturating_add(coord.y().saturating_mul(11))
                .rem_euclid(35),
        );
        if coord.x().rem_euclid(9) == 0 {
            base.saturating_add(11).min(160)
        } else {
            base
        }
    }

    fn surface_for(coord: HexCoord, radius: u32) -> ReviewTerrainSurfaceInputV1 {
        let level = terrain_level(coord);
        let sides = coord.neighbors().map(|direction| {
            let adjacent = (direction.distance(HexCoord::ORIGIN) <= radius)
                .then(|| TilePos::new(direction, terrain_level(direction)));
            let exposed_bottom_level = adjacent.map_or(level.saturating_sub(10), |pos| {
                pos.level.saturating_add(1).min(level.saturating_add(1))
            });
            ReviewTerrainSideInputV1 {
                direction,
                adjacent_surface: adjacent,
                exposed_bottom_level,
            }
        });
        ReviewTerrainSurfaceInputV1 {
            pos: TilePos::new(coord, level),
            substrate: SubstanceId(4),
            substrate_color: [0.38, 0.40, 0.36, 1.0],
            exposed_natural: true,
            current_snow: level >= 140,
            forced_summit: level >= 158,
            snow_exception: ReviewSnowExceptionV1::None,
            sides,
            cliff_layers: vec![ReviewCliffLayerInputV1 {
                bottom_level: level.saturating_sub(10),
                top_level: level.saturating_add(1),
                substrate: SubstanceId(4),
                substrate_color: [0.38, 0.40, 0.36, 1.0],
            }],
            prop_exclusions: ReviewPropExclusionsV1::default(),
        }
    }

    fn set_surface_level(surface: &mut ReviewTerrainSurfaceInputV1, level: Level) {
        surface.pos.level = level;
        surface.cliff_layers = vec![ReviewCliffLayerInputV1 {
            bottom_level: level.saturating_sub(10),
            top_level: level.saturating_add(1),
            substrate: surface.substrate,
            substrate_color: surface.substrate_color,
        }];
    }

    fn fixture(radius: u32) -> ReviewTerrainInputV1 {
        let mut builder = ReviewTerrainInputBuilderV1::new(1_592_598_566, LEVEL_HEIGHT)
            .expect("valid fixture level height");
        for (index, coord) in HexCoord::ORIGIN
            .within_radius(radius)
            .into_iter()
            .enumerate()
        {
            let mut surface = surface_for(coord, radius);
            if index % 97 == 0 {
                surface.prop_exclusions.path = true;
            }
            if index % 149 == 0 {
                surface.prop_exclusions.named_anchor_safety_disk = true;
            }
            builder
                .insert_surface(surface)
                .expect("fixture exact surfaces are unique");
            if index % 37 == 0 {
                builder
                    .insert_vegetation(ReviewVegetationInputV1 {
                        stable_id: u64::try_from(index).expect("fixture index fits u64"),
                        root: TilePos::new(coord, terrain_level(coord)),
                        snow_dust_eligible: index % 74 == 0,
                    })
                    .expect("fixture vegetation ids are unique");
            }
        }
        builder.build()
    }

    fn profile_with_snow(snow: SnowDetailV1) -> ReviewWorldDetailProfileV1 {
        ReviewWorldDetailProfileV1 {
            snow,
            ..ReviewWorldDetailProfileV1::default()
        }
    }

    #[test]
    fn control_plan_is_empty_and_stably_hashed() {
        let input = fixture(5);
        let first = plan_review_terrain_details(&ReviewWorldDetailProfileV1::default(), &input)
            .expect("control plan is valid");
        let second = plan_review_terrain_details(&ReviewWorldDetailProfileV1::default(), &input)
            .expect("control plan is reproducible");

        assert!(first.mesh_batches.is_empty());
        assert!(first.vegetation_batches.is_empty());
        assert!(first.prop_placements.is_empty());
        let current_snow = input
            .surfaces
            .values()
            .filter_map(|surface| surface.current_snow.then_some(surface.pos))
            .collect::<BTreeSet<_>>();
        assert_eq!(first.resolved_snow_surfaces, current_snow);
        assert_eq!(first.counts, ReviewTerrainPlanCountsV1::default());
        assert_eq!(first, second);
        assert_ne!(first.plan_hash, 0);
    }

    #[test]
    fn builder_keeps_stacked_surfaces_distinct_and_rejects_exact_duplicates() {
        let coord = HexCoord::from_axial(2, -1);
        let mut builder =
            ReviewTerrainInputBuilderV1::new(7, LEVEL_HEIGHT).expect("valid level height");
        let low = surface_for(coord, 0);
        let mut high = low.clone();
        let high_level = high.pos.level.saturating_add(20);
        set_surface_level(&mut high, high_level);
        builder
            .insert_surface(low.clone())
            .expect("first exact surface is valid");
        builder
            .insert_surface(high)
            .expect("stacked exact surface must not collapse by coordinate");
        assert_eq!(
            builder.insert_surface(low.clone()),
            Err(ReviewTerrainPlanError::DuplicateSurface(low.pos))
        );
        assert_eq!(builder.build().surfaces.len(), 2);
    }

    #[test]
    fn literal_snowlines_preserve_small_components_and_terrain_aware_cleans_them() {
        let isolated = HexCoord::ORIGIN;
        let frozen = HexCoord::from_axial(10, 0);
        let garden = HexCoord::from_axial(-10, 0);
        let mut builder =
            ReviewTerrainInputBuilderV1::new(11, LEVEL_HEIGHT).expect("valid level height");
        let mut isolated_surface = surface_for(isolated, 0);
        set_surface_level(&mut isolated_surface, 150);
        isolated_surface.current_snow = false;
        isolated_surface.forced_summit = false;
        let mut frozen_surface = surface_for(frozen, 0);
        set_surface_level(&mut frozen_surface, 120);
        frozen_surface.current_snow = true;
        frozen_surface.snow_exception = ReviewSnowExceptionV1::FrozenWoods;
        let mut garden_surface = surface_for(garden, 0);
        set_surface_level(&mut garden_surface, 155);
        garden_surface.current_snow = false;
        garden_surface.snow_exception = ReviewSnowExceptionV1::Garden;
        for surface in [isolated_surface.clone(), frozen_surface, garden_surface] {
            builder
                .insert_surface(surface)
                .expect("fixture surfaces are unique");
        }
        let input = builder.build();
        let profile = profile_with_snow(SnowDetailV1::StraightThreshold { level: 128 });
        let plan = plan_review_terrain_details(&profile, &input).expect("snow plan is valid");
        assert_eq!(plan.counts.snow_caps_added, 1);
        assert_eq!(plan.counts.snow_caps_hidden, 0);
        assert_eq!(plan.resolved_snow_surfaces.len(), 2);
        assert!(plan
            .resolved_snow_surfaces
            .contains(&TilePos::new(isolated, 150)));
        assert!(plan
            .resolved_snow_surfaces
            .contains(&TilePos::new(frozen, 120)));

        let coherent = profile_with_snow(SnowDetailV1::CoherentLine {
            mean_level: 136,
            amplitude_levels: 8,
            correlation_hexes: 22,
        });
        let coherent_plan =
            plan_review_terrain_details(&coherent, &input).expect("coherent snow plan is valid");
        assert!(coherent_plan
            .resolved_snow_surfaces
            .contains(&TilePos::new(isolated, 150)));

        let terrain_aware = profile_with_snow(SnowDetailV1::TerrainAware {
            vertical_shell_height: 0.0,
        });
        let cleaned = plan_review_terrain_details(&terrain_aware, &input)
            .expect("terrain-aware snow plan is valid");
        assert_eq!(cleaned.counts.snow_caps_added, 0);
        assert_eq!(cleaned.resolved_snow_surfaces.len(), 1);
        assert!(cleaned
            .resolved_snow_surfaces
            .contains(&TilePos::new(frozen, 120)));

        let mut summit_builder =
            ReviewTerrainInputBuilderV1::new(11, LEVEL_HEIGHT).expect("valid level height");
        isolated_surface.forced_summit = true;
        summit_builder
            .insert_surface(isolated_surface)
            .expect("summit is valid");
        let summit_plan = plan_review_terrain_details(&profile, &summit_builder.build())
            .expect("summit snow plan is valid");
        assert_eq!(summit_plan.counts.snow_caps_added, 1);
        assert!(summit_plan
            .resolved_snow_surfaces
            .contains(&TilePos::new(isolated, 150)));
    }

    #[test]
    fn terrain_aware_offsets_follow_relief_and_aspect_bands() {
        let coord = HexCoord::ORIGIN;
        let mut flat = surface_for(coord, 1);
        set_surface_level(&mut flat, 140);
        flat.sides = coord.neighbors().map(|direction| ReviewTerrainSideInputV1 {
            direction,
            adjacent_surface: Some(TilePos::new(direction, 139)),
            exposed_bottom_level: 140,
        });
        let mut north_steep = flat.clone();
        north_steep.sides = coord.neighbors().map(|direction| {
            let north = direction.y() < coord.y();
            ReviewTerrainSideInputV1 {
                direction,
                adjacent_surface: Some(TilePos::new(direction, if north { 130 } else { 139 })),
                exposed_bottom_level: if north { 131 } else { 140 },
            }
        });
        let mut south_steep = flat.clone();
        south_steep.sides = coord.neighbors().map(|direction| {
            let south = direction.y() > coord.y();
            ReviewTerrainSideInputV1 {
                direction,
                adjacent_surface: Some(TilePos::new(direction, if south { 130 } else { 139 })),
                exposed_bottom_level: if south { 131 } else { 140 },
            }
        });

        assert_eq!(neighbour_relief(&flat), 1);
        assert_eq!(slope_aspect_offset(&flat), 0);
        assert_eq!(neighbour_relief(&north_steep), 10);
        assert_eq!(slope_aspect_offset(&north_steep), -4);
        assert_eq!(slope_aspect_offset(&south_steep), 4);
        let detail = SnowDetailV1::TerrainAware {
            vertical_shell_height: 0.0,
        };
        let flat_line = snowline_for(&detail, 99, &flat);
        let north_line = snowline_for(&detail, 99, &north_steep);
        let south_line = snowline_for(&detail, 99, &south_steep);
        assert_eq!(north_line.saturating_sub(flat_line), 6);
        assert_eq!(south_line.saturating_sub(north_line), 8);
        assert!((128..=156).contains(&south_line));
    }

    #[test]
    fn every_terrain_family_matrix_member_has_a_distinct_plan() {
        let input = fixture(45);
        let mut snow = BTreeSet::new();
        let mut vegetation = BTreeSet::new();
        let mut cliffs = BTreeSet::new();
        let mut props = BTreeSet::new();
        for profile in ReviewWorldDetailProfileV1::atomic_matrix() {
            let plan = plan_review_terrain_details(&profile, &input)
                .expect("validated atomic matrix member must plan");
            if profile.snow.treatment_id().is_some() {
                snow.insert(plan.plan_hash);
            }
            if profile.alpine_vegetation.treatment_id().is_some() {
                vegetation.insert(plan.plan_hash);
            }
            if profile.cliff_strata.treatment_id().is_some() {
                cliffs.insert(plan.plan_hash);
            }
            if profile.terrain_props.treatment_id().is_some() {
                props.insert(plan.plan_hash);
            }
        }
        assert_eq!(snow.len(), 9);
        assert_eq!(vegetation.len(), 6);
        assert_eq!(cliffs.len(), 6);
        assert_eq!(props.len(), 6);
    }

    #[test]
    fn cliff_shells_are_exact_opaque_substrate_values_and_share_material_identities() {
        let mut builder =
            ReviewTerrainInputBuilderV1::new(91, LEVEL_HEIGHT).expect("valid fixture height");
        for (coord, substrate, color) in [
            (HexCoord::ORIGIN, SubstanceId(4), [0.40, 0.50, 0.60, 1.0]),
            (
                HexCoord::from_axial(17, 0),
                SubstanceId(4),
                [0.40, 0.50, 0.60, 1.0],
            ),
            (
                HexCoord::from_axial(-17, 0),
                SubstanceId(5),
                [0.80, 0.30, 0.20, 1.0],
            ),
        ] {
            let mut surface = surface_for(coord, 0);
            set_surface_level(&mut surface, 63);
            surface.substrate = substrate;
            surface.substrate_color = color;
            surface.sides = coord.neighbors().map(|direction| ReviewTerrainSideInputV1 {
                direction,
                adjacent_surface: None,
                exposed_bottom_level: 0,
            });
            surface.cliff_layers = vec![ReviewCliffLayerInputV1 {
                bottom_level: 0,
                top_level: 64,
                substrate,
                substrate_color: color,
            }];
            builder
                .insert_surface(surface)
                .expect("distinct cliff surface should be valid");
        }
        let profile = ReviewWorldDetailProfileV1 {
            cliff_strata: CliffStrataDetailV1::StrataWithValue {
                period_levels: 32,
                width_levels: 3,
                contrast: 0.08,
                phase_variation_levels: 4,
                correlation_hexes: 22,
                value_delta: -0.08,
            },
            ..ReviewWorldDetailProfileV1::default()
        };
        let plan = plan_review_terrain_details(&profile, &builder.build())
            .expect("substrate-relative cliff plan should build");
        let cliff_batches = plan
            .mesh_batches
            .iter()
            .filter(|batch| {
                matches!(
                    batch.material_role,
                    ReviewTerrainMaterialRoleV1::CliffValue
                        | ReviewTerrainMaterialRoleV1::CliffStrata
                )
            })
            .collect::<Vec<_>>();
        assert!(!cliff_batches.is_empty());
        let material_identities = cliff_batches
            .iter()
            .map(|batch| {
                (
                    batch.material_role,
                    batch.substrate.expect("cliff shell retains its substrate"),
                    batch.base_color.map(f32::to_bits),
                )
            })
            .collect::<BTreeSet<_>>();
        // Two roles times two substances, shared across all chunks and sides.
        assert_eq!(material_identities.len(), 4);
        for batch in cliff_batches {
            let substrate = match batch.substrate {
                Some(SubstanceId(4)) => [0.40, 0.50, 0.60, 1.0],
                Some(SubstanceId(5)) => [0.80, 0.30, 0.20, 1.0],
                other => panic!("unexpected cliff substrate {other:?}"),
            };
            let multiplier = match batch.material_role {
                ReviewTerrainMaterialRoleV1::CliffValue => 0.92,
                ReviewTerrainMaterialRoleV1::CliffStrata => 0.92 * 0.92,
                _ => unreachable!("filtered to cliff roles"),
            };
            let expected = cliff_relative_color(substrate, multiplier);
            assert!(batch
                .base_color
                .iter()
                .zip(expected)
                .all(|(actual, expected)| (actual - expected).abs() < 1.0e-6));
            let [_, _, _, alpha] = batch.base_color;
            assert_eq!(alpha.to_bits(), 1.0_f32.to_bits());
        }
    }

    #[test]
    fn cliff_shells_preserve_vertical_material_boundaries_below_a_thin_cap() {
        let coord = HexCoord::ORIGIN;
        let stone = SubstanceId(4);
        let grass = SubstanceId(5);
        let stone_color = [0.40, 0.50, 0.60, 1.0];
        let grass_color = [0.20, 0.55, 0.18, 1.0];
        let mut surface = surface_for(coord, 0);
        set_surface_level(&mut surface, 9);
        surface.substrate = grass;
        surface.substrate_color = grass_color;
        surface.sides = coord.neighbors().map(|direction| ReviewTerrainSideInputV1 {
            direction,
            adjacent_surface: None,
            exposed_bottom_level: 0,
        });
        surface.cliff_layers = vec![
            ReviewCliffLayerInputV1 {
                bottom_level: 0,
                top_level: 9,
                substrate: stone,
                substrate_color: stone_color,
            },
            ReviewCliffLayerInputV1 {
                bottom_level: 9,
                top_level: 10,
                substrate: grass,
                substrate_color: grass_color,
            },
        ];
        let mut builder =
            ReviewTerrainInputBuilderV1::new(91, LEVEL_HEIGHT).expect("valid fixture height");
        builder
            .insert_surface(surface)
            .expect("contiguous material layers should be valid");
        let profile = ReviewWorldDetailProfileV1 {
            cliff_strata: CliffStrataDetailV1::SideValue { value_delta: -0.06 },
            ..ReviewWorldDetailProfileV1::default()
        };
        let plan = plan_review_terrain_details(&profile, &builder.build())
            .expect("layered cliff plan should build");
        let cliff_materials = plan
            .mesh_batches
            .iter()
            .filter(|batch| batch.material_role == ReviewTerrainMaterialRoleV1::CliffValue)
            .map(|batch| {
                (
                    batch.substrate.expect("cliff layer keeps its substrate"),
                    batch.base_color.map(f32::to_bits),
                )
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(cliff_materials.len(), 2);
        assert!(cliff_materials.contains(&(
            stone,
            cliff_relative_color(stone_color, 0.94).map(f32::to_bits)
        )));
        assert!(cliff_materials.contains(&(
            grass,
            cliff_relative_color(grass_color, 0.94).map(f32::to_bits)
        )));
    }

    #[test]
    fn combined_cliff_value_and_strata_partition_without_layering() {
        let bands = strata_bands(0, 64, 32, 3, 0);
        let value_regions = interval_complement(0, 64, &bands);
        for level in 0..64 {
            let band_owners = bands
                .iter()
                .filter(|(bottom, top)| (*bottom..*top).contains(&level))
                .count();
            let value_owners = value_regions
                .iter()
                .filter(|(bottom, top)| (*bottom..*top).contains(&level))
                .count();
            assert_eq!(band_owners + value_owners, 1);
        }
        assert_eq!(bands, [(0, 3), (32, 35)]);
        assert_eq!(value_regions, [(3, 32), (35, 64)]);
    }

    #[test]
    fn prop_exclusions_and_caps_are_hard_boundaries() {
        let input = fixture(45);
        for profile in ReviewWorldDetailProfileV1::atomic_matrix()
            .into_iter()
            .filter(|profile| profile.terrain_props.treatment_id().is_some())
        {
            let plan = plan_review_terrain_details(&profile, &input)
                .expect("prop matrix member must plan");
            let cap = match profile.terrain_props {
                TerrainPropsDetailV1::Boulders { cap, .. }
                | TerrainPropsDetailV1::GrassLitter { cap, .. }
                | TerrainPropsDetailV1::Mixed { cap, .. }
                | TerrainPropsDetailV1::Clustered { cap, .. } => usize::from(cap),
                TerrainPropsDetailV1::Current => 0,
            };
            assert!(plan.prop_placements.len() <= cap);
            for placement in &plan.prop_placements {
                let source = input
                    .surfaces
                    .get(&placement.root)
                    .expect("every prop retains an exact source surface");
                assert!(!source.prop_exclusions.any());
            }
            if matches!(
                profile.terrain_props,
                TerrainPropsDetailV1::Clustered { .. }
            ) {
                let mut cluster_sizes = BTreeMap::<u64, usize>::new();
                for placement in &plan.prop_placements {
                    let cluster = placement
                        .cluster
                        .expect("clustered treatment labels every piece");
                    *cluster_sizes.entry(cluster).or_default() += 1;
                }
                assert!(cluster_sizes.values().all(|count| (3..=5).contains(count)));
            }
        }
    }

    #[test]
    fn deadwood_faces_are_wound_outward() {
        let mut batch = MeshBatchBuilder::new(
            MeshBatchKey {
                chunk: (0, 0),
                role: ReviewTerrainMaterialRoleV1::Deadwood,
                substrate: None,
            },
            [0.3, 0.2, 0.1, 1.0],
        );
        emit_deadwood(
            &mut batch,
            Vec3::new(2.0, 3.0, 4.0),
            1_592_598_566,
            ReviewPropPlacementV1 {
                root: TilePos::ORIGIN,
                kind: ReviewPropKindV1::Deadwood,
                cluster: None,
            },
        )
        .expect("deadwood fixture should build");
        let (minimum, maximum) = batch.positions.iter().copied().map(Vec3::from_array).fold(
            (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
            |(minimum, maximum), position| (minimum.min(position), maximum.max(position)),
        );
        let centre = (minimum + maximum) * 0.5;
        for (triangle, normals) in batch
            .positions
            .chunks_exact(3)
            .zip(batch.normals.chunks_exact(3))
        {
            let face_centre = triangle.iter().copied().map(Vec3::from_array).sum::<Vec3>() / 3.0;
            let [first_normal, _, _] = normals else {
                panic!("each triangle must retain three normals");
            };
            let normal = Vec3::from_array(*first_normal);
            assert!(normal.dot(face_centre - centre) > 0.0);
        }
    }

    #[test]
    fn concrete_batches_are_finite_aligned_and_wound_to_their_normals() {
        let mut profile = ReviewWorldDetailProfileV1::default();
        profile.snow = SnowDetailV1::TerrainAware {
            vertical_shell_height: 0.08,
        };
        profile.alpine_vegetation = AlpineVegetationDetailV1::ScaleJitterWithDust {
            horizontal_min: 0.90,
            horizontal_max: 1.10,
            vertical_min: 0.95,
            vertical_max: 1.05,
            upper_fraction: 0.25,
            shell_height: 0.02,
        };
        profile.cliff_strata = CliffStrataDetailV1::StrataWithValue {
            period_levels: 32,
            width_levels: 3,
            contrast: 0.08,
            phase_variation_levels: 4,
            correlation_hexes: 22,
            value_delta: -0.08,
        };
        profile.terrain_props = TerrainPropsDetailV1::Mixed {
            boulder_density: 0.0012,
            tuft_density: 0.0030,
            deadwood_density: 0.0005,
            cap: 500,
        };
        let input = fixture(35);
        let plan = plan_review_terrain_details(&profile, &input)
            .expect("combined terrain plan must be valid");
        assert!(!plan.mesh_batches.is_empty());
        for batch in &plan.mesh_batches {
            assert_eq!(batch.positions.len(), batch.normals.len());
            assert_eq!(batch.positions.len(), batch.uv0.len());
            assert_eq!(batch.indices.len() % 3, 0);
            assert!(batch
                .positions
                .iter()
                .flatten()
                .chain(batch.normals.iter().flatten())
                .chain(batch.uv0.iter().flatten())
                .all(|component| component.is_finite()));
            assert!(batch
                .indices
                .iter()
                .all(|index| usize::try_from(*index)
                    .is_ok_and(|value| value < batch.positions.len())));
            for (positions, normals) in batch
                .positions
                .chunks_exact(3)
                .zip(batch.normals.chunks_exact(3))
            {
                let [first, second, third] = positions else {
                    continue;
                };
                let expected = (Vec3::from_array(*second) - Vec3::from_array(*first))
                    .cross(Vec3::from_array(*third) - Vec3::from_array(*first))
                    .normalize();
                let Some(actual) = normals.first().copied() else {
                    continue;
                };
                assert!(expected.dot(Vec3::from_array(actual)) > 0.999);
            }
        }
        assert_eq!(
            plan.counts.vertices,
            plan.mesh_batches.iter().fold(0_u64, |total, batch| total
                .saturating_add(u64::try_from(batch.positions.len()).unwrap_or(u64::MAX)))
        );
        assert_eq!(
            plan.counts.triangles,
            plan.mesh_batches.iter().fold(0_u64, |total, batch| total
                .saturating_add(u64::try_from(batch.indices.len() / 3).unwrap_or(u64::MAX)))
        );
    }
}
