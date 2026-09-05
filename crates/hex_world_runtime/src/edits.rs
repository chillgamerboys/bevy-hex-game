//! Atomic resident terrain/object edits and exact local replication.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use hex_world_contracts::{
    hash_serializable, ChunkId, ChunkPackage, ColumnData, ObjectEdit, VoxelPosition, VoxelRun,
    WorldChange, WorldEditTransaction, WorldHex, WorldObjectEditTransaction, SCHEMA_VERSION,
};
use serde::{Deserialize, Serialize};

use crate::{
    history::JournalDescriptor,
    persistence::{ChunkOverlay, OverlayLocation},
    runtime::{combined_object_columns, validate_identity, ResidentChunk},
    ChunkProduct, ErrorKind, RuntimeError, RuntimeResult, WorldRuntime,
};

/// Exact replacement columns for one revised chunk, never a whole-world snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChunkDelta {
    /// Global partition identity.
    pub coordinate: ChunkId,
    /// Required resident revision before applying this delta.
    pub base_revision: u64,
    /// Exactly the next revision.
    pub revision: u64,
    /// Fingerprint of the complete current package before the change.
    pub base_fingerprint: u64,
    /// Fingerprint of the complete package after replacing these columns.
    pub target_fingerprint: u64,
    /// Only changed terrain columns, sorted and unique; empty for object-only deltas.
    pub columns: Vec<ColumnData>,
}

/// Idempotent local world replication tied to an exact immutable world source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldDelta {
    /// Supported fresh V4 protocol version.
    pub schema_version: u32,
    /// Stable owning world.
    pub world_id: String,
    /// Exact source manifest identity.
    pub manifest_fingerprint: u64,
    /// Stable atomic transaction identity.
    pub transaction_id: String,
    /// Identity of the original canonical edit request.
    pub request_fingerprint: u64,
    /// Changed chunks in canonical order.
    pub chunks: Vec<ChunkDelta>,
    /// Exact atomic object operations, empty for a terrain-only transaction.
    /// Object deltas contain revision/fingerprint chunk records with empty terrain columns.
    #[serde(default)]
    pub object_edits: Vec<ObjectEdit>,
    /// Hash of this value with its own fingerprint zeroed.
    pub fingerprint: u64,
}

impl WorldDelta {
    /// Verifies integrity and canonical local shape without trusting a resident world.
    pub fn validate(&self) -> RuntimeResult<()> {
        validate_identity(&self.transaction_id)?;
        validate_identity(&self.world_id)?;
        if self.chunks.is_empty()
            || self.chunks.len() > hex_world_contracts::MAX_EDITS_PER_TRANSACTION
        {
            return Err(RuntimeError::new(
                ErrorKind::Limit,
                "delta must have a bounded nonempty changed chunk set",
            ));
        }
        if self.schema_version != SCHEMA_VERSION
            || self.fingerprint != self.expected_fingerprint()?
        {
            return Err(RuntimeError::invalid(
                "delta schema or fingerprint mismatch",
            ));
        }
        if self.chunks.windows(2).any(|pair| {
            pair.first()
                .zip(pair.get(1))
                .is_some_and(|(a, b)| a.coordinate >= b.coordinate)
        }) {
            return Err(RuntimeError::invalid(
                "delta chunks are not unique canonical coordinates",
            ));
        }
        for chunk in &self.chunks {
            if chunk.base_revision.checked_add(1) != Some(chunk.revision)
                || (chunk.columns.is_empty() && self.object_edits.is_empty())
                || chunk.columns.len() > 256
            {
                return Err(RuntimeError::invalid(
                    "delta revision or column count is invalid",
                ));
            }
            if chunk.columns.windows(2).any(|pair| {
                pair.first()
                    .zip(pair.get(1))
                    .is_some_and(|(a, b)| a.position >= b.position)
            }) {
                return Err(RuntimeError::invalid(
                    "delta columns are not unique canonical positions",
                ));
            }
            for column in &chunk.columns {
                column.validate().map_err(RuntimeError::invalid)?;
                if column.position.chunk() != chunk.coordinate {
                    return Err(RuntimeError::invalid(
                        "delta column belongs to another chunk",
                    ));
                }
            }
        }
        if !self.object_edits.is_empty() {
            if self.chunks.iter().any(|chunk| !chunk.columns.is_empty()) {
                return Err(RuntimeError::invalid(
                    "mixed terrain/object deltas are unsupported",
                ));
            }
            let transaction = self.object_transaction();
            transaction.validate().map_err(RuntimeError::invalid)?;
            if crate::object_edits::request_fingerprint(&transaction)? != self.request_fingerprint {
                return Err(RuntimeError::invalid(
                    "object delta request fingerprint disagrees",
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn object_transaction(&self) -> WorldObjectEditTransaction {
        WorldObjectEditTransaction {
            id: self.transaction_id.clone(),
            expected_revisions: self
                .chunks
                .iter()
                .map(|chunk| (chunk.coordinate, chunk.base_revision))
                .collect(),
            edits: self.object_edits.clone(),
        }
    }

    pub(crate) fn changed_columns(&self) -> RuntimeResult<Vec<WorldHex>> {
        if self.object_edits.is_empty() {
            Ok(self
                .chunks
                .iter()
                .flat_map(|chunk| chunk.columns.iter().map(|column| column.position))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect())
        } else {
            Ok(self
                .object_transaction()
                .affected_columns()
                .map_err(RuntimeError::invalid)?
                .into_iter()
                .collect())
        }
    }

    fn expected_fingerprint(&self) -> RuntimeResult<u64> {
        let mut value = self.clone();
        value.fingerprint = 0;
        hash_serializable(&value).map_err(RuntimeError::invalid)
    }

    pub(crate) fn seal(&mut self) -> RuntimeResult<()> {
        self.fingerprint = self.expected_fingerprint()?;
        self.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AppliedTransaction {
    pub request_fingerprint: u64,
    pub change: WorldChange,
    pub delta: WorldDelta,
}

pub(crate) struct StagedEdit {
    pub chunks: BTreeMap<ChunkId, (ChunkProduct, ChunkOverlay, BTreeMap<WorldHex, ColumnData>)>,
    pub applied: AppliedTransaction,
    pub journal: JournalDescriptor,
}

impl WorldRuntime {
    /// Commits one canonical transaction atomically in memory.
    ///
    /// All revision, material, semantic and size checks finish before any chunk
    /// changes. Use `apply_transaction_durable` when acknowledgment must survive a
    /// process crash, or `save` to checkpoint a group of in-memory transactions.
    pub fn apply_transaction(
        &mut self,
        transaction: &WorldEditTransaction,
    ) -> RuntimeResult<WorldChange> {
        let staged = self.stage_transaction(transaction)?;
        Ok(self.commit_edit(staged))
    }

    /// Loads one exact historical delta, using bounded IO when its body is paged out.
    /// Unrelated transaction bodies and world chunks are never loaded by this call.
    pub fn transaction_delta(&self, transaction_id: &str) -> RuntimeResult<Option<WorldDelta>> {
        self.transactions
            .get(transaction_id)
            .map(|entry| {
                entry
                    .load(&self.manifest)
                    .map(|record| record.delta.clone())
            })
            .transpose()
    }

    /// Applies one integrity-checked local delta atomically and idempotently.
    /// This never loads terrain from the source or reconstructs unrelated chunks.
    /// A historical duplicate may read its one paged journal body.
    pub fn apply_delta(&mut self, delta: &WorldDelta) -> RuntimeResult<WorldChange> {
        let staged = self.stage_delta(delta)?;
        Ok(self.commit_edit(staged))
    }

    pub(crate) fn stage_delta(&self, delta: &WorldDelta) -> RuntimeResult<StagedEdit> {
        delta.validate()?;
        if delta.world_id != self.manifest.world_id
            || delta.manifest_fingerprint != self.manifest.fingerprint
        {
            return Err(RuntimeError::new(
                ErrorKind::Conflict,
                "delta belongs to another world source",
            ));
        }
        if let Some(entry) = self.transactions.get(&delta.transaction_id) {
            return if entry.descriptor.delta_fingerprint == delta.fingerprint {
                let applied = entry.load(&self.manifest)?;
                if applied.delta != *delta {
                    return Err(RuntimeError::new(
                        ErrorKind::Conflict,
                        "transaction identity carries a different delta",
                    ));
                }
                Ok(StagedEdit {
                    chunks: BTreeMap::new(),
                    applied: (*applied).clone(),
                    journal: entry.descriptor.clone(),
                })
            } else {
                Err(RuntimeError::new(
                    ErrorKind::Conflict,
                    "transaction identity already carries a different delta",
                ))
            };
        }
        if !delta.object_edits.is_empty() {
            for change in &delta.chunks {
                let resident = self.resident.get(&change.coordinate).ok_or_else(|| {
                    RuntimeError::new(
                        ErrorKind::Unavailable,
                        "object delta requires an unloaded chunk",
                    )
                })?;
                if resident.product.revision != change.base_revision
                    || resident.product.package.fingerprint != change.base_fingerprint
                {
                    return Err(RuntimeError::new(
                        ErrorKind::Conflict,
                        "object delta base revision or fingerprint disagrees",
                    ));
                }
            }
            let staged = self.stage_object_transaction(&delta.object_transaction())?;
            if staged.applied.delta != *delta {
                return Err(RuntimeError::invalid(
                    "object delta does not describe the canonical exact outcome",
                ));
            }
            return Ok(staged);
        }
        let count = delta
            .chunks
            .iter()
            .try_fold(0_usize, |count, chunk| {
                count.checked_add(chunk.columns.len())
            })
            .ok_or_else(|| RuntimeError::new(ErrorKind::Limit, "delta column count overflow"))?;
        if count > self.config.max_edits_per_transaction
            || delta.chunks.len() > self.config.max_resident_chunks
        {
            return Err(RuntimeError::new(
                ErrorKind::Limit,
                "delta exceeds per-operation bounds",
            ));
        }
        let mut packages = BTreeMap::new();
        for change in &delta.chunks {
            let resident = self.resident.get(&change.coordinate).ok_or_else(|| {
                RuntimeError::new(ErrorKind::Unavailable, "delta requires an unloaded chunk")
            })?;
            if resident.product.revision != change.base_revision
                || resident.product.package.fingerprint != change.base_fingerprint
            {
                return Err(RuntimeError::new(
                    ErrorKind::Conflict,
                    "delta base revision or package fingerprint disagrees",
                ));
            }
            let mut candidate = (*resident.product.package).clone();
            for replacement in &change.columns {
                let column = exact_column_mut(&mut candidate, replacement.position)?;
                protect_materials(column, replacement, &self.manifest.materials)?;
                *column = replacement.clone();
            }
            candidate.seal().map_err(RuntimeError::invalid)?;
            candidate
                .validate_with_index(&self.manifest_index)
                .map_err(RuntimeError::invalid)?;
            if candidate.fingerprint != change.target_fingerprint {
                return Err(RuntimeError::invalid("delta target fingerprint disagrees"));
            }
            packages.insert(change.coordinate, candidate);
        }
        let staged = self.stage_packages(
            packages,
            delta.transaction_id.clone(),
            delta.request_fingerprint,
            Vec::new(),
        )?;
        if staged.applied.delta != *delta {
            return Err(RuntimeError::invalid(
                "delta contains redundant columns or a noncanonical outcome",
            ));
        }
        Ok(staged)
    }

    pub(crate) fn stage_transaction(
        &self,
        transaction: &WorldEditTransaction,
    ) -> RuntimeResult<StagedEdit> {
        transaction.validate().map_err(RuntimeError::invalid)?;
        if transaction.edits.len() > self.config.max_edits_per_transaction {
            return Err(RuntimeError::new(
                ErrorKind::Limit,
                "transaction exceeds its edit budget",
            ));
        }
        let request_fingerprint = hash_serializable(transaction).map_err(RuntimeError::invalid)?;
        if let Some(entry) = self.transactions.get(&transaction.id) {
            return if entry.descriptor.request_fingerprint == request_fingerprint {
                let applied = entry.load(&self.manifest)?;
                Ok(StagedEdit {
                    chunks: BTreeMap::new(),
                    applied: (*applied).clone(),
                    journal: entry.descriptor.clone(),
                })
            } else {
                Err(RuntimeError::new(
                    ErrorKind::Conflict,
                    "transaction identity reused for a different command",
                ))
            };
        }
        let mut packages = BTreeMap::new();
        for (coordinate, expected) in &transaction.expected_revisions {
            let resident = self.resident.get(coordinate).ok_or_else(|| {
                RuntimeError::new(ErrorKind::Unavailable, "edit requires an unloaded chunk")
            })?;
            if resident.product.revision != *expected {
                return Err(RuntimeError::new(
                    ErrorKind::Conflict,
                    "edit expected revision is stale",
                ));
            }
            packages.insert(*coordinate, (*resident.product.package).clone());
        }
        for edit in &transaction.edits {
            if edit.material.as_ref().is_some_and(|id| {
                !self
                    .manifest
                    .materials
                    .iter()
                    .any(|material| material.id == *id)
            }) {
                return Err(RuntimeError::invalid("edit assigns an unknown material"));
            }
            if edit.material.as_ref().is_some_and(|id| {
                self.manifest
                    .materials
                    .iter()
                    .any(|material| material.id == *id && !material.solid)
            }) {
                return Err(RuntimeError::new(
                    ErrorKind::Conflict,
                    "edit requires unsupported liquid semantic regeneration",
                ));
            }
            let candidate = packages
                .get_mut(&edit.position.column.chunk())
                .ok_or_else(|| RuntimeError::invalid("missing exact chunk expectation"))?;
            if candidate.semantics.occupancy.iter().any(|column| {
                column.position == edit.position.column
                    && column.material_at(edit.position.level).is_some()
            }) {
                return Err(RuntimeError::new(
                    ErrorKind::Conflict,
                    "edit requires unsupported static object semantic regeneration",
                ));
            }
            let column = exact_column_mut(candidate, edit.position.column)?;
            let replacement = assigned_column(column, edit.position, edit.material.as_deref())?;
            protect_materials(column, &replacement, &self.manifest.materials)?;
            *column = replacement;
        }
        self.stage_packages(
            packages,
            transaction.id.clone(),
            request_fingerprint,
            Vec::new(),
        )
    }

    pub(crate) fn stage_packages(
        &self,
        packages: BTreeMap<ChunkId, ChunkPackage>,
        transaction_id: String,
        request_fingerprint: u64,
        object_edits: Vec<ObjectEdit>,
    ) -> RuntimeResult<StagedEdit> {
        let mut chunks = BTreeMap::new();
        let mut deltas = Vec::new();
        let mut revisions = BTreeMap::new();
        let mut changed_columns = BTreeSet::new();
        for (coordinate, mut package) in packages {
            let resident = self.resident.get(&coordinate).ok_or_else(|| {
                RuntimeError::new(ErrorKind::Unavailable, "staged chunk is unavailable")
            })?;
            let changed = package
                .columns
                .iter()
                .zip(&resident.product.package.columns)
                .filter(|(a, b)| a != b)
                .map(|(column, _)| column.clone())
                .collect::<Vec<_>>();
            let objects_changed = package.semantics.objects
                != resident.product.package.semantics.objects
                || package.semantics.object_influences
                    != resident.product.package.semantics.object_influences;
            if changed.is_empty() && !objects_changed {
                continue;
            }
            for column in &changed {
                if package
                    .semantics
                    .occupancy
                    .iter()
                    .any(|object| object.position == column.position)
                    || package
                        .semantics
                        .objects
                        .iter()
                        .any(|object| object.origin.column == column.position)
                    || self
                        .manifest_index
                        .boundary_samples_at(column.position)
                        .next()
                        .is_some()
                {
                    return Err(RuntimeError::new(
                        ErrorKind::Conflict,
                        "edit requires unsupported static object or boundary semantic regeneration",
                    ));
                }
            }
            let revision =
                resident.product.revision.checked_add(1).ok_or_else(|| {
                    RuntimeError::new(ErrorKind::Limit, "chunk revision exhausted")
                })?;
            package.seal().map_err(RuntimeError::invalid)?;
            package
                .validate_with_index(&self.manifest_index)
                .map_err(|error| {
                    RuntimeError::new(
                        ErrorKind::Conflict,
                        format!("edit requires semantic regeneration: {error}"),
                    )
                })?;
            let query_columns = combined_object_columns(&package)?;
            let mut overlay_columns = match self.overlays.get(&coordinate) {
                Some(OverlayLocation::Memory(overlay)) => overlay
                    .columns
                    .iter()
                    .map(|column| (column.position, column.clone()))
                    .collect::<BTreeMap<_, _>>(),
                Some(OverlayLocation::Disk { .. }) => {
                    return Err(RuntimeError::invalid(
                        "resident modified chunk lacks its loaded overlay",
                    ))
                }
                None => BTreeMap::new(),
            };
            let object_state = if objects_changed {
                Some(crate::object_edits::ObjectChunkState::from_package(
                    &package,
                ))
            } else {
                match self.overlays.get(&coordinate) {
                    Some(OverlayLocation::Memory(overlay)) => overlay.object_state.clone(),
                    _ => None,
                }
            };
            for column in &changed {
                overlay_columns.insert(column.position, column.clone());
                changed_columns.insert(column.position);
            }
            let mut overlay = ChunkOverlay {
                schema_version: SCHEMA_VERSION,
                world_id: self.manifest.world_id.clone(),
                coordinate,
                base_fingerprint: resident.base_fingerprint,
                revision,
                target_fingerprint: package.fingerprint,
                columns: overlay_columns.into_values().collect(),
                object_state,
                fingerprint: 0,
            };
            overlay.seal()?;
            deltas.push(ChunkDelta {
                coordinate,
                base_revision: resident.product.revision,
                revision,
                base_fingerprint: resident.product.package.fingerprint,
                target_fingerprint: package.fingerprint,
                columns: changed,
            });
            revisions.insert(coordinate, revision);
            chunks.insert(
                coordinate,
                (
                    ChunkProduct {
                        coordinate,
                        revision,
                        package: Arc::new(package),
                    },
                    overlay,
                    query_columns,
                ),
            );
        }
        if !object_edits.is_empty() {
            let transaction = WorldObjectEditTransaction {
                id: transaction_id.clone(),
                expected_revisions: BTreeMap::new(),
                edits: object_edits.clone(),
            };
            changed_columns.extend(
                transaction
                    .affected_columns()
                    .map_err(RuntimeError::invalid)?,
            );
        }
        let change = WorldChange {
            transaction_id: transaction_id.clone(),
            revisions,
            changed_columns: changed_columns.into_iter().collect(),
        };
        if change.changed_columns.is_empty() {
            return Err(RuntimeError::invalid("transaction makes no world change"));
        }
        change.validate().map_err(RuntimeError::invalid)?;
        let new_dirty = chunks
            .keys()
            .filter(|coordinate| !self.dirty.contains(coordinate))
            .count();
        if self.dirty.len().saturating_add(new_dirty) > self.config.max_unsaved_chunks {
            return Err(RuntimeError::new(
                ErrorKind::Limit,
                "unsaved partition backlog exceeds budget; checkpoint before further edits",
            ));
        }
        let mut delta = WorldDelta {
            schema_version: SCHEMA_VERSION,
            world_id: self.manifest.world_id.clone(),
            manifest_fingerprint: self.manifest.fingerprint,
            transaction_id,
            request_fingerprint,
            chunks: deltas,
            object_edits,
            fingerprint: 0,
        };
        delta.seal()?;
        let applied = AppliedTransaction {
            request_fingerprint,
            change,
            delta,
        };
        let journal = JournalDescriptor::prepare(&applied, self.config.max_transaction_bytes)?;
        self.check_history_budget(&journal)?;
        Ok(StagedEdit {
            chunks,
            applied,
            journal,
        })
    }

    pub(crate) fn commit_edit(&mut self, staged: StagedEdit) -> WorldChange {
        for (coordinate, (product, overlay, query_columns)) in staged.chunks {
            let base_fingerprint = overlay.base_fingerprint;
            self.overlays
                .insert(coordinate, OverlayLocation::Memory(overlay));
            self.dirty.insert(coordinate);
            self.pending_changed.insert(coordinate, product.clone());
            self.resident.insert(
                coordinate,
                ResidentChunk {
                    product,
                    base_fingerprint,
                    query_columns,
                },
            );
        }
        let change = staged.applied.change.clone();
        self.cache_transaction(staged.applied, staged.journal);
        change
    }
}

pub(crate) fn exact_column_mut(
    package: &mut ChunkPackage,
    position: WorldHex,
) -> RuntimeResult<&mut ColumnData> {
    let index = package
        .columns
        .binary_search_by_key(&position, |column| column.position)
        .map_err(|error| {
            RuntimeError::invalid(format!("column outside declared chunk footprint ({error})"))
        })?;
    package
        .columns
        .get_mut(index)
        .ok_or_else(|| RuntimeError::invalid("column lookup invariant failed"))
}

fn assigned_column(
    column: &ColumnData,
    position: VoxelPosition,
    material: Option<&str>,
) -> RuntimeResult<ColumnData> {
    let top = position.level.checked_add(1).ok_or_else(|| {
        RuntimeError::invalid("voxel assignment exceeds exclusive level endpoint")
    })?;
    let mut runs = Vec::new();
    for run in &column.runs {
        if run.bottom <= position.level && position.level < run.top {
            if run.bottom < position.level {
                runs.push(VoxelRun {
                    bottom: run.bottom,
                    top: position.level,
                    material: run.material.clone(),
                });
            }
            if top < run.top {
                runs.push(VoxelRun {
                    bottom: top,
                    top: run.top,
                    material: run.material.clone(),
                });
            }
        } else {
            runs.push(run.clone());
        }
    }
    if let Some(material) = material {
        runs.push(VoxelRun {
            bottom: position.level,
            top,
            material: material.to_owned(),
        });
    }
    let mut replacement = ColumnData {
        position: column.position,
        runs,
    };
    replacement.seal().map_err(RuntimeError::invalid)?;
    Ok(replacement)
}

fn protect_materials(
    before: &ColumnData,
    after: &ColumnData,
    materials: &[hex_world_contracts::MaterialSpec],
) -> RuntimeResult<()> {
    for new in &after.runs {
        if materials
            .iter()
            .any(|material| material.id == new.material && !material.solid)
            && !before.runs.iter().any(|run| {
                run.material == new.material && run.bottom <= new.bottom && run.top >= new.top
            })
        {
            return Err(RuntimeError::new(
                ErrorKind::Conflict,
                "edit requires unsupported liquid semantic regeneration",
            ));
        }
    }
    for old in &before.runs {
        if materials
            .iter()
            .any(|material| material.id == old.material && !material.diggable)
            && !after.runs.iter().any(|run| {
                run.material == old.material && run.bottom <= old.bottom && run.top >= old.top
            })
        {
            return Err(RuntimeError::new(
                ErrorKind::Conflict,
                "ordinary terrain edits cannot remove an indestructible material",
            ));
        }
    }
    Ok(())
}
