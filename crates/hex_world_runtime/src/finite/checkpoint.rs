//! Sparse finite destruction checkpoints, independent of residency and meshes.

use super::*;
use crate::{CancellationToken, ErrorKind};
use hex_world_contracts::{VoxelEdit, CHUNK_SIZE};
use serde::{Deserialize, Serialize};

const FINITE_SCHEMA: u32 = 1;
const MAX_PARTITION_CELLS: usize = 1_048_576;
const MAX_TOTAL_CELLS: usize = 16_777_216;

/// Metadata for exactly one immutable world and finite live-edit policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FiniteSessionHeader {
    /// Finite overlay codec version, currently one.
    pub schema_version: u32,
    /// Immutable compiled world identity.
    pub world_id: String,
    /// Exact manifest content fingerprint; changed generated worlds are incompatible.
    pub manifest_fingerprint: u64,
    /// Inclusive permitted editing floor.
    pub bottom: i32,
    /// Inclusive permitted editing ceiling.
    pub top: i32,
    /// Canonical complete list of modified chunks; omission during restore is an error.
    pub edited_chunks: Vec<ChunkId>,
}

/// Final sparse assignments/removals for one chunk, without its generated columns,
/// semantic objects, transient meshes, or obsolete transaction replay history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FiniteChunkCheckpoint {
    /// Exact storage partition.
    pub coordinate: ChunkId,
    /// Original immutable source package fingerprint, never a rendered-edit hash.
    pub base_fingerprint: u64,
    /// Live finite revision to continue after restoration.
    pub revision: u64,
    /// Canonical unique final terrain assignments; `None` represents carved air.
    pub terrain_edits: Vec<VoxelEdit>,
    /// Canonical exact removed object cells, including those subsequently refilled.
    pub object_removed: Vec<VoxelPosition>,
}

impl FiniteWorldSession {
    /// Export only identity, finite bounds and modified partition identities.
    /// Read this and the partition iterator at the same quiescent owner boundary.
    #[must_use]
    pub fn checkpoint_header(&self) -> FiniteSessionHeader {
        FiniteSessionHeader {
            schema_version: FINITE_SCHEMA,
            world_id: self.manifest_index.manifest().world_id.clone(),
            manifest_fingerprint: self.manifest_index.manifest().fingerprint,
            bottom: self.bottom,
            top: self.top,
            edited_chunks: self
                .revisions
                .iter()
                .filter_map(|(coordinate, revision)| (*revision > 0).then_some(*coordinate))
                .collect(),
        }
    }

    /// Export one bounded sparse partition at a time, including unloaded edited chunks.
    /// The caller can encode each item into an independent atomic owner record.
    pub fn checkpoint_partitions(
        &self,
    ) -> impl Iterator<Item = RuntimeResult<FiniteChunkCheckpoint>> + '_ {
        self.revisions
            .iter()
            .filter(|(_, revision)| **revision > 0)
            .map(|(coordinate, revision)| {
                let chunks = &self.manifest_index.manifest().chunks;
                let descriptor = chunks
                    .binary_search_by_key(coordinate, |descriptor| descriptor.coordinate)
                    .ok()
                    .and_then(|index| chunks.get(index))
                    .ok_or_else(|| {
                        RuntimeError::invalid("finite checkpoint chunk has no source")
                    })?;
                if self
                    .sources
                    .get(coordinate)
                    .is_some_and(|source| source.fingerprint != descriptor.fingerprint)
                {
                    return Err(RuntimeError::new(
                        ErrorKind::Conflict,
                        "finite checkpoint cannot mix a strict edited base with immutable source",
                    ));
                }
                let origin = coordinate.origin().map_err(RuntimeError::invalid)?;
                let mut terrain_edits = Vec::new();
                let mut object_removed = Vec::new();
                // Sixteen bounded row ranges avoid rescanning all session edits for
                // every exported chunk and retain only one partition's payload.
                for local_q in 0..CHUNK_SIZE {
                    let first = origin
                        .checked_add(WorldHex::new(local_q, 0))
                        .map_err(RuntimeError::invalid)?;
                    let last = origin
                        .checked_add(WorldHex::new(local_q, CHUNK_SIZE - 1))
                        .map_err(RuntimeError::invalid)?;
                    let range = VoxelPosition {
                        column: first,
                        level: i32::MIN,
                    }..=VoxelPosition {
                        column: last,
                        level: i32::MAX,
                    };
                    terrain_edits.extend(self.terrain_edits.range(range.clone()).map(
                        |(position, material)| VoxelEdit {
                            position: *position,
                            material: material.clone(),
                        },
                    ));
                    object_removed.extend(self.object_removed.range(range).copied());
                }
                let partition = FiniteChunkCheckpoint {
                    coordinate: *coordinate,
                    base_fingerprint: descriptor.fingerprint,
                    revision: *revision,
                    terrain_edits,
                    object_removed,
                };
                partition.validate_shape()?;
                Ok(partition)
            })
    }

    /// Stage a complete finite-session replacement without changing the runtime or
    /// live session. Trusted expected bounds must match the saved bounds exactly.
    /// Each changed source package is loaded and strictly verified in turn; these
    /// temporary payloads are not made resident or retained in the restored session.
    /// Liquid edits remain forbidden; broken support/anchors/bedrock and carved
    /// object cells retain the arena's deliberately permissive existing semantics.
    /// The owner must also restore its transaction counter; historical transaction
    /// retries are not resumed, while saved per-chunk revisions still reject stale edits.
    pub fn restore_checkpoint<I>(
        runtime: &WorldRuntime,
        bottom: i32,
        top: i32,
        header: &FiniteSessionHeader,
        partitions: I,
        cancellation: &CancellationToken,
    ) -> RuntimeResult<Self>
    where
        I: IntoIterator<Item = RuntimeResult<FiniteChunkCheckpoint>>,
    {
        cancellation.check()?;
        if header.schema_version != FINITE_SCHEMA
            || header.bottom != bottom
            || header.top != top
            || header.world_id != runtime.manifest().world_id
            || header.manifest_fingerprint != runtime.manifest().fingerprint
        {
            return Err(RuntimeError::new(
                ErrorKind::Conflict,
                "finite checkpoint source, schema or bounds changed",
            ));
        }
        if header.edited_chunks.len() > runtime.manifest().chunks.len()
            || header
                .edited_chunks
                .windows(2)
                .any(|pair| pair.first() >= pair.get(1))
            || header
                .edited_chunks
                .iter()
                .any(|chunk| !runtime.descriptors.contains_key(chunk))
        {
            return Err(RuntimeError::invalid(
                "finite checkpoint chunk list is invalid",
            ));
        }
        let mut candidate = Self::streamed(runtime, bottom, top)?;
        let mut count = 0_usize;
        let mut total = 0_usize;
        for partition in partitions {
            cancellation.check()?;
            let partition = partition?;
            if header.edited_chunks.get(count) != Some(&partition.coordinate) {
                return Err(RuntimeError::invalid(
                    "missing, duplicate or out-of-order finite partition",
                ));
            }
            partition.validate_shape()?;
            total = total
                .saturating_add(partition.terrain_edits.len())
                .saturating_add(partition.object_removed.len());
            if total > MAX_TOTAL_CELLS {
                return Err(RuntimeError::new(
                    ErrorKind::Limit,
                    "finite checkpoint total cell limit exceeded",
                ));
            }
            let descriptor = runtime
                .descriptors
                .get(&partition.coordinate)
                .ok_or_else(|| RuntimeError::invalid("finite checkpoint source absent"))?;
            if partition.base_fingerprint != descriptor.fingerprint {
                return Err(RuntimeError::new(
                    ErrorKind::Conflict,
                    "finite partition base fingerprint changed",
                ));
            }
            let source = runtime
                .source
                .load_chunk_cancelled(partition.coordinate, cancellation)?;
            crate::source::validate_source_chunk(&runtime.manifest_index, descriptor, &source)?;
            partition.validate_against_source(&source, &candidate)?;
            candidate
                .revisions
                .insert(partition.coordinate, partition.revision);
            for edit in partition.terrain_edits {
                candidate.terrain_edits.insert(edit.position, edit.material);
            }
            candidate.object_removed.extend(partition.object_removed);
            count += 1;
        }
        if count != header.edited_chunks.len() {
            return Err(RuntimeError::invalid("finite checkpoint is incomplete"));
        }
        cancellation.check()?;
        Ok(candidate)
    }
}

impl FiniteChunkCheckpoint {
    fn validate_shape(&self) -> RuntimeResult<()> {
        if self.revision == 0 || self.terrain_edits.is_empty() {
            return Err(RuntimeError::invalid(
                "finite partition requires edits and a nonzero revision",
            ));
        }
        if self
            .terrain_edits
            .len()
            .saturating_add(self.object_removed.len())
            > MAX_PARTITION_CELLS
        {
            return Err(RuntimeError::new(
                ErrorKind::Limit,
                "finite checkpoint partition cell limit exceeded",
            ));
        }
        if self
            .terrain_edits
            .windows(2)
            .any(|pair| pair.first().map(|e| e.position) >= pair.get(1).map(|e| e.position))
            || self
                .object_removed
                .windows(2)
                .any(|pair| pair.first() >= pair.get(1))
            || self
                .terrain_edits
                .iter()
                .any(|edit| edit.position.column.chunk() != self.coordinate)
            || self
                .object_removed
                .iter()
                .any(|position| position.column.chunk() != self.coordinate)
        {
            return Err(RuntimeError::invalid(
                "finite partition cells are not canonical, unique and local",
            ));
        }
        Ok(())
    }

    fn validate_against_source(
        &self,
        source: &ChunkPackage,
        session: &FiniteWorldSession,
    ) -> RuntimeResult<()> {
        let removed: BTreeSet<_> = self.object_removed.iter().copied().collect();
        let mut expected_removed = BTreeSet::new();
        for edit in &self.terrain_edits {
            if !(session.bottom..=session.top).contains(&edit.position.level) {
                return Err(RuntimeError::invalid(
                    "saved finite edit exceeds height bounds",
                ));
            }
            let column = source
                .columns
                .binary_search_by_key(&edit.position.column, |column| column.position)
                .ok()
                .and_then(|index| source.columns.get(index))
                .ok_or_else(|| {
                    RuntimeError::invalid("saved finite edit is outside source footprint")
                })?;
            if column.material_at(edit.position.level).is_some_and(|name| {
                session
                    .materials
                    .get(name)
                    .is_some_and(|material| !material.solid)
            }) {
                return Err(RuntimeError::invalid(
                    "saved finite edit modifies immutable liquid",
                ));
            }
            if edit.material.as_ref().is_some_and(|name| {
                !session
                    .materials
                    .get(name)
                    .is_some_and(|material| material.solid)
            }) {
                return Err(RuntimeError::invalid(
                    "saved finite edit assigns unknown or liquid material",
                ));
            }
            if source
                .semantics
                .occupancy
                .binary_search_by_key(&edit.position.column, |column| column.position)
                .ok()
                .and_then(|index| source.semantics.occupancy.get(index))
                .and_then(|column| column.material_at(edit.position.level))
                .is_some()
            {
                expected_removed.insert(edit.position);
            }
        }
        if removed != expected_removed {
            return Err(RuntimeError::invalid(
                "saved object removals disagree with exact source occupancy and edits",
            ));
        }
        Ok(())
    }
}
