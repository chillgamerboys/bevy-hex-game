use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use bevy::prelude::*;
use hex_core::{
    HexCoord, HexSpan, SubstanceId, TerrainChunkRoot, TilePos, MAX_TERRAIN_PICK_RUNS_PER_BATCH,
};
use hex_world_contracts::{
    ChunkPackage, ColumnData, ManifestIndex, MaterialSpec, VoxelPosition, VoxelRun, WorldHex,
    WorldManifest,
};

use super::{PresentationError, ResidentRun, RunSource};
use crate::{
    grid::{resident_terrain_mesh, ProjectedRun, TerrainMeshRun},
    voxel::SubstanceRun,
};

/// Integer anchor for a bounded rendering window, independent of world authority.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderOrigin {
    /// Global horizontal hex rendered at local zero.
    pub column: WorldHex,
    /// Global voxel level rendered at local height zero.
    pub level: i32,
}

impl RenderOrigin {
    /// Subtract the global origin before converting to the legacy integer hex type.
    pub fn local_hex(self, column: WorldHex) -> Result<HexCoord, PresentationError> {
        let q = column
            .q
            .checked_sub(self.column.q)
            .and_then(|v| i32::try_from(v).ok());
        let r = column
            .r
            .checked_sub(self.column.r)
            .and_then(|v| i32::try_from(v).ok());
        let (Some(q), Some(r)) = (q, r) else {
            return Err(PresentationError(
                "render-local horizontal offset overflows i32".into(),
            ));
        };
        if i64::from(q)
            .abs()
            .max(i64::from(r).abs())
            .max((i64::from(q) + i64::from(r)).abs())
            > 4096
        {
            return Err(PresentationError(
                "render-local hex exceeds the precision envelope".into(),
            ));
        }
        Ok(HexCoord::from_axial(q, r))
    }

    /// Convert an exact global voxel into a bounded local picking position.
    pub fn local_voxel(self, position: VoxelPosition) -> Result<TilePos, PresentationError> {
        let level = local_level(position.level, self.level)?;
        Ok(TilePos::new(self.local_hex(position.column)?, level))
    }

    /// Recover exact global identity from a local picking position using integers.
    pub fn global_voxel(self, position: TilePos) -> Result<VoxelPosition, PresentationError> {
        let column = self.column.checked_add(WorldHex::new(
            i64::from(position.coord.x()),
            i64::from(position.coord.y()),
        ))?;
        self.local_hex(column)?;
        let level = self
            .level
            .checked_add(position.level)
            .ok_or_else(|| PresentationError("global picked level overflows i32".into()))?;
        local_level(level, self.level)?;
        Ok(VoxelPosition { column, level })
    }
}

/// Operational bounds for one presentation window, never world/package limits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PresentationLimits {
    /// Maximum resident roots and maximum temporary chunks in an atomic rebase.
    pub max_resident_chunks: usize,
    /// Maximum derived terrain/object intervals in one CPU preparation product.
    pub max_runs_per_chunk: usize,
    /// Maximum axial/cube distance from the current render origin (at most 4096).
    pub max_local_hex: i32,
    /// Maximum absolute relative voxel level (at most 1,048,576).
    pub max_local_level: i32,
    /// Maximum absolute rendered vertical position (at most 4096 world units).
    pub max_render_height: f32,
}

impl Default for PresentationLimits {
    fn default() -> Self {
        Self {
            max_resident_chunks: 256,
            max_runs_per_chunk: 32_768,
            max_local_hex: 1024,
            max_local_level: 4096,
            max_render_height: 4096.0,
        }
    }
}

impl PresentationLimits {
    pub(super) fn validate(self) -> Result<(), PresentationError> {
        if self.max_resident_chunks == 0
            || self.max_runs_per_chunk == 0
            || !(1..=4096).contains(&self.max_local_hex)
            || !(1..=1_048_576).contains(&self.max_local_level)
            || !self.max_render_height.is_finite()
            || !(0.01..=4096.0).contains(&self.max_render_height)
        {
            return Err(PresentationError(
                "invalid operational presentation limits".into(),
            ));
        }
        Ok(())
    }
}

/// Clonable immutable preparation snapshot suitable for a bounded worker queue.
///
/// Preparation creates CPU meshes only, without a Bevy `World`, assets, or windows.
/// Stale-origin products are rejected on publication. Queue ownership belongs to
/// the application; this adapter never starts background jobs itself.
#[derive(Clone)]
pub struct TerrainPreparer {
    pub(super) manifest: Arc<WorldManifest>,
    pub(super) index: Arc<ManifestIndex>,
    pub(super) palette: Arc<BTreeMap<String, (SubstanceId, MaterialSpec)>>,
    pub(super) origin: RenderOrigin,
    pub(super) level_height: f32,
    pub(super) limits: PresentationLimits,
}

/// Validated CPU-only product, tied to one exact origin, palette and chunk revision.
///
/// Construction is private so callers cannot publish unchecked or partially
/// prepared geometry. A revised package need not match its immutable base hash.
pub struct PreparedChunk {
    pub(super) package: Arc<ChunkPackage>,
    pub(super) revision: u64,
    pub(super) context: TerrainPreparer,
    pub(super) marker: TerrainChunkRoot,
    pub(super) batches: Vec<PreparedBatch>,
    pub(super) suppression: Arc<Vec<ColumnData>>,
    pub(super) suppression_fingerprint: u64,
}

pub(super) struct PreparedBatch {
    pub substance: SubstanceId,
    pub material: MaterialSpec,
    pub mesh: Option<Mesh>,
    pub runs: Vec<PreparedRun>,
}

#[derive(Clone)]
pub(super) struct PreparedRun {
    pub geometry: TerrainMeshRun,
    pub exact: ResidentRun,
}

impl PreparedChunk {
    /// Exact global coordinate of this product.
    #[must_use]
    pub fn coordinate(&self) -> hex_world_contracts::ChunkId {
        self.package.coordinate
    }
    /// Runtime revision carried by this product.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }
    /// Number of exact logical run entities to publish.
    #[must_use]
    pub fn logical_runs(&self) -> usize {
        self.batches.iter().map(|batch| batch.runs.len()).sum()
    }
    /// Number of independently owned mesh assets to publish.
    #[must_use]
    pub fn meshes(&self) -> usize {
        self.batches
            .iter()
            .filter(|batch| batch.mesh.is_some())
            .count()
    }
    /// Canonical checksum of the exact render-only object suppression mask.
    /// Equal terrain revisions may replace presentation when this signature changes.
    #[must_use]
    pub const fn suppression_fingerprint(&self) -> u64 {
        self.suppression_fingerprint
    }
}

impl TerrainPreparer {
    /// Current integer rendering anchor.
    #[must_use]
    pub const fn origin(&self) -> RenderOrigin {
        self.origin
    }

    /// Validate and prepare one complete resident package without ECS mutation.
    pub fn prepare(
        &self,
        package: &ChunkPackage,
        revision: u64,
    ) -> Result<PreparedChunk, PresentationError> {
        self.prepare_with_suppressed_occupancy(package, revision, &[])
    }

    /// Prepare terrain with an exact render-only subset of static object occupancy hidden.
    ///
    /// The canonical mask must use this chunk's object materials and lie wholly
    /// inside static object intervals. Logical runs, headroom and picking metadata
    /// remain complete. Suppressed cells do not occlude render faces: their actual
    /// stock art may be Cutout/Blend even when the logical palette color is opaque.
    /// The application owns atomic publication of matching stock-art fragments and
    /// rejects stale mask jobs before publishing a different suppression signature.
    pub fn prepare_with_suppressed_occupancy(
        &self,
        package: &ChunkPackage,
        revision: u64,
        suppression: &[ColumnData],
    ) -> Result<PreparedChunk, PresentationError> {
        package.validate_with_index(&self.index)?;
        validate_suppression(package, suppression, self.limits.max_runs_per_chunk)?;
        let suppression_fingerprint = hex_world_contracts::hash_serializable(suppression)?;
        let mut projected = BTreeMap::new();
        let mut grouped: BTreeMap<SubstanceId, Vec<PreparedRun>> = BTreeMap::new();
        let mut run_count = 0_usize;
        for column in &package.columns {
            let local = self.origin.local_hex(column.position)?;
            if local.x().abs().max(local.y().abs()).max(local.z().abs()) > self.limits.max_local_hex
            {
                return Err(PresentationError(
                    "chunk lies outside the active presentation window".into(),
                ));
            }
            let occupancy = package
                .semantics
                .occupancy
                .iter()
                .find(|entry| entry.position == column.position)
                .map_or(&[][..], |entry| entry.runs.as_slice());
            let intervals = union_runs(&column.runs, occupancy)?;
            run_count = run_count
                .checked_add(intervals.len())
                .ok_or_else(|| PresentationError("derived run count overflow".into()))?;
            if run_count > self.limits.max_runs_per_chunk {
                return Err(PresentationError(
                    "max_runs_per_chunk presentation preparation budget exceeded".into(),
                ));
            }
            let mut geometry = Vec::new();
            for (index, (run, source)) in intervals.iter().enumerate() {
                let bottom = self.checked_level(run.bottom)?;
                let top = self.checked_level(run.top)?;
                let headroom = intervals
                    .get(index + 1)
                    .map(|(next, _)| u32::try_from(i64::from(next.bottom) - i64::from(run.top)))
                    .transpose()
                    .map_err(|error| PresentationError(error.to_string()))?;
                let (substance, _) = self.palette.get(&run.material).ok_or_else(|| {
                    PresentationError("material missing from prepared palette".into())
                })?;
                let span = HexSpan::new(self.height(bottom)?, self.height(top)?);
                let mesh_run = TerrainMeshRun {
                    position: TilePos::new(local, top - 1),
                    span,
                    bottom,
                    top,
                    cutaway: None,
                };
                let prepared_run = PreparedRun {
                    geometry: mesh_run,
                    exact: ResidentRun {
                        position: VoxelPosition {
                            column: column.position,
                            level: run.top - 1,
                        },
                        bottom: run.bottom,
                        top: run.top,
                        material: run.material.clone(),
                        headroom,
                        source: *source,
                    },
                };
                for (bottom, top) in remaining_intervals(&prepared_run, suppression) {
                    geometry.push(ProjectedRun {
                        run: SubstanceRun {
                            bottom: self.checked_level(bottom)?,
                            top: self.checked_level(top)?,
                            substance: *substance,
                        },
                        cutaway: None,
                    });
                }
                grouped.entry(*substance).or_default().push(prepared_run);
            }
            projected.insert(local, geometry);
        }
        let rendered_runs = grouped.values().flatten().try_fold(0usize, |count, run| {
            count
                .checked_add(remaining_intervals(run, suppression).len())
                .ok_or_else(|| PresentationError("render fragment count overflow".into()))
        })?;
        if rendered_runs > self.limits.max_runs_per_chunk {
            return Err(PresentationError(
                "max_runs_per_chunk render fragment budget exceeded".into(),
            ));
        }
        let opaque: BTreeSet<_> = self
            .palette
            .values()
            .filter(|(_, material)| material.color.last() == Some(&255))
            .map(|(id, _)| *id)
            .collect();
        let mut batches = Vec::new();
        for (substance, runs) in grouped {
            let material = self
                .palette
                .values()
                .find(|(id, _)| *id == substance)
                .map(|(_, material)| material.clone())
                .ok_or_else(|| PresentationError("batch palette entry is missing".into()))?;
            // Transparent prisms cannot hide an opaque wall behind them. Within a
            // translucent body only identical material can hide an internal face.
            let occluders = projected
                .iter()
                .map(|(coord, column)| {
                    let runs = column
                        .iter()
                        .filter(|run| {
                            if opaque.contains(&substance) {
                                opaque.contains(&run.run.substance)
                            } else {
                                run.run.substance == substance
                            }
                        })
                        .copied()
                        .collect();
                    (*coord, runs)
                })
                .collect();
            for partition in runs.chunks(MAX_TERRAIN_PICK_RUNS_PER_BATCH) {
                let mut geometry = Vec::new();
                for run in partition {
                    for (bottom, top) in remaining_intervals(run, suppression) {
                        let bottom = self.checked_level(bottom)?;
                        let top = self.checked_level(top)?;
                        geometry.push(TerrainMeshRun {
                            position: TilePos::new(run.geometry.position.coord, top - 1),
                            span: HexSpan::new(self.height(bottom)?, self.height(top)?),
                            bottom,
                            top,
                            cutaway: None,
                        });
                    }
                }
                let mesh = if geometry.is_empty() {
                    None
                } else {
                    Some(
                        resident_terrain_mesh(&geometry, &occluders, self.level_height)
                            .map_err(PresentationError)?,
                    )
                };
                batches.push(PreparedBatch {
                    substance,
                    material: material.clone(),
                    mesh,
                    runs: partition.to_vec(),
                });
            }
        }
        let chunk_origin = self.origin.local_hex(package.coordinate.origin()?)?;
        let marker = TerrainChunkRoot {
            q: chunk_origin.x().div_euclid(16),
            r: chunk_origin.y().div_euclid(16),
        };
        Ok(PreparedChunk {
            package: Arc::new(package.clone()),
            revision,
            context: self.clone(),
            marker,
            batches,
            suppression: Arc::new(suppression.to_vec()),
            suppression_fingerprint,
        })
    }

    fn checked_level(&self, global: i32) -> Result<i32, PresentationError> {
        let local = local_level(global, self.origin.level)?;
        if local.abs() > self.limits.max_local_level {
            return Err(PresentationError(
                "chunk exceeds the active vertical presentation window".into(),
            ));
        }
        Ok(local)
    }

    fn height(&self, local: i32) -> Result<f32, PresentationError> {
        #[expect(
            clippy::cast_precision_loss,
            reason = "checked relative integer levels are exactly representable as f32"
        )]
        let value = local as f32 * self.level_height;
        if !value.is_finite() || value.abs() > self.limits.max_render_height {
            return Err(PresentationError(
                "rendered height exceeds the local precision envelope".into(),
            ));
        }
        Ok(value)
    }
}

fn validate_suppression(
    package: &ChunkPackage,
    suppression: &[ColumnData],
    run_limit: usize,
) -> Result<(), PresentationError> {
    if suppression.len() > 256 {
        return Err(PresentationError(
            "suppression exceeds one chunk's column budget".into(),
        ));
    }
    let mut previous = None;
    let mut runs = 0usize;
    for column in suppression {
        if column.position.chunk() != package.coordinate
            || previous.is_some_and(|position| position >= column.position)
            || column.runs.is_empty()
        {
            return Err(PresentationError(
                "suppression columns must be local, ordered, unique and nonempty".into(),
            ));
        }
        column.validate()?;
        runs = runs.saturating_add(column.runs.len());
        if runs > run_limit {
            return Err(PresentationError(
                "max_runs_per_chunk suppression budget exceeded".into(),
            ));
        }
        previous = Some(column.position);
        let owner = package
            .semantics
            .occupancy
            .binary_search_by_key(&column.position, |entry| entry.position)
            .ok()
            .and_then(|index| package.semantics.occupancy.get(index))
            .ok_or_else(|| {
                PresentationError("suppression targets terrain or absent object occupancy".into())
            })?;
        for run in &column.runs {
            let index = owner
                .runs
                .partition_point(|candidate| candidate.top <= run.bottom);
            if !owner.runs.get(index).is_some_and(|candidate| {
                candidate.bottom <= run.bottom
                    && candidate.top >= run.top
                    && candidate.material == run.material
            }) {
                return Err(PresentationError("suppression is not an exact material-matching subset of static object occupancy".into()));
            }
        }
    }
    Ok(())
}

fn remaining_intervals(run: &PreparedRun, suppression: &[ColumnData]) -> Vec<(i32, i32)> {
    if run.exact.source != RunSource::StaticObject {
        return vec![(run.exact.bottom, run.exact.top)];
    }
    let Some(mask) = suppression
        .binary_search_by_key(&run.exact.position.column, |column| column.position)
        .ok()
        .and_then(|index| suppression.get(index))
    else {
        return vec![(run.exact.bottom, run.exact.top)];
    };
    let mut cursor = run.exact.bottom;
    let mut intervals = Vec::new();
    let start = mask
        .runs
        .partition_point(|hidden| hidden.top <= run.exact.bottom);
    for hidden in mask
        .runs
        .iter()
        .skip(start)
        .take_while(|hidden| hidden.bottom < run.exact.top)
    {
        if hidden.bottom > cursor {
            intervals.push((cursor, hidden.bottom));
        }
        cursor = cursor.max(hidden.top).min(run.exact.top);
    }
    if cursor < run.exact.top {
        intervals.push((cursor, run.exact.top));
    }
    intervals
}

fn local_level(global: i32, origin: i32) -> Result<i32, PresentationError> {
    let difference = i64::from(global) - i64::from(origin);
    if difference.abs() > 1_048_576 {
        return Err(PresentationError(
            "render-local level exceeds the precision envelope".into(),
        ));
    }
    i32::try_from(difference).map_err(|error| PresentationError(error.to_string()))
}

// Occupancy has the same precedence as the runtime query. Interval endpoints,
// rather than voxel expansion, preserve large negative and stacked source runs.
pub(super) fn union_runs(
    terrain: &[VoxelRun],
    occupancy: &[VoxelRun],
) -> Result<Vec<(VoxelRun, RunSource)>, PresentationError> {
    let mut endpoints: Vec<_> = terrain
        .iter()
        .chain(occupancy)
        .flat_map(|run| [run.bottom, run.top])
        .collect();
    endpoints.sort_unstable();
    endpoints.dedup();
    let mut combined: Vec<(VoxelRun, RunSource)> = Vec::new();
    let mut terrain_cursor = 0;
    let mut occupancy_cursor = 0;
    for pair in endpoints.windows(2) {
        let [bottom, top] = pair else {
            return Err(PresentationError("invalid interval partition".into()));
        };
        while terrain
            .get(terrain_cursor)
            .is_some_and(|run| run.top <= *bottom)
        {
            terrain_cursor += 1;
        }
        while occupancy
            .get(occupancy_cursor)
            .is_some_and(|run| run.top <= *bottom)
        {
            occupancy_cursor += 1;
        }
        let source = occupancy
            .get(occupancy_cursor)
            .filter(|run| run.bottom <= *bottom && *bottom < run.top)
            .map(|run| (run, RunSource::StaticObject))
            .or_else(|| {
                terrain
                    .get(terrain_cursor)
                    .filter(|run| run.bottom <= *bottom && *bottom < run.top)
                    .map(|run| (run, RunSource::Terrain))
            });
        if let Some((run, source)) = source {
            if let Some((last, last_source)) = combined.last_mut() {
                if last.top == *bottom && last.material == run.material && *last_source == source {
                    last.top = *top;
                    continue;
                }
            }
            combined.push((
                VoxelRun {
                    bottom: *bottom,
                    top: *top,
                    material: run.material.clone(),
                },
                source,
            ));
        }
    }
    Ok(combined)
}
