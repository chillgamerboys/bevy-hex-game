//! Bounded object-only transactions, with independent identity-bearing chunk overlays.

use std::{collections::BTreeMap, path::Path};

use hex_world_contracts::{
    hash_serializable, union_object_occupancy, AnchorRole, ChunkId, ChunkPackage, ChunkSemantics,
    ColumnData, ManifestIndex, ObjectInfluence, ObjectInstance, VoxelPosition, WorldChange,
    WorldObjectEditTransaction,
};
use serde::{Deserialize, Serialize};

use crate::{
    edits::StagedEdit, runtime::combined_object_columns, AttachmentUpdate, ErrorKind, IoLimits,
    RuntimeError, RuntimeResult, WorldRuntime,
};

/// Complete object-owned partition state; other chunk semantics stay immutable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ObjectChunkState {
    pub roots: Vec<ObjectInstance>,
    pub influences: Vec<ObjectInfluence>,
}
impl ObjectChunkState {
    pub(crate) fn from_package(package: &ChunkPackage) -> Self {
        Self {
            roots: package.semantics.objects.clone(),
            influences: package.semantics.object_influences.clone(),
        }
    }
    pub(crate) fn validate(&self, coordinate: ChunkId) -> RuntimeResult<()> {
        let semantics = ChunkSemantics {
            objects: self.roots.clone(),
            object_influences: self.influences.clone(),
            occupancy: union_object_occupancy(&self.influences).map_err(RuntimeError::invalid)?,
            ..ChunkSemantics::default()
        };
        semantics.validate().map_err(RuntimeError::invalid)?;
        if self
            .roots
            .iter()
            .any(|object| object.origin.column.chunk() != coordinate)
            || self
                .influences
                .iter()
                .flat_map(|row| &row.occupancy)
                .any(|column| column.position.chunk() != coordinate)
        {
            return Err(RuntimeError::invalid(
                "object partition contains foreign roots or columns",
            ));
        }
        Ok(())
    }
    pub(crate) fn apply(&self, package: &mut ChunkPackage) -> RuntimeResult<()> {
        self.validate(package.coordinate)?;
        package.semantics.objects = self.roots.clone();
        package.semantics.object_influences = self.influences.clone();
        package.semantics.occupancy =
            union_object_occupancy(&self.influences).map_err(RuntimeError::invalid)?;
        Ok(())
    }
}

pub(crate) fn request_fingerprint(transaction: &WorldObjectEditTransaction) -> RuntimeResult<u64> {
    hash_serializable(&("object-edit-v1", transaction)).map_err(RuntimeError::invalid)
}

impl WorldRuntime {
    /// Atomically add, remove or replace exact authored objects in resident chunks.
    ///
    /// The full old/new dependency set must be resident at every expected revision.
    /// Callers may use `ObjectInstance::dependency_chunks` and operation pins to load
    /// it beforehand. This exclusive operation never loads, unpins, or evicts chunks.
    /// Actor collision and gameplay command authority remain caller responsibilities.
    pub fn apply_object_transaction(
        &mut self,
        transaction: &WorldObjectEditTransaction,
    ) -> RuntimeResult<WorldChange> {
        let staged = self.stage_object_transaction(transaction)?;
        Ok(self.commit_edit(staged))
    }

    /// Persist exact object changes before acknowledging their atomic publication.
    pub fn apply_object_transaction_durable(
        &mut self,
        transaction: &WorldObjectEditTransaction,
        root: impl AsRef<Path>,
        limits: IoLimits,
    ) -> RuntimeResult<WorldChange> {
        self.apply_object_transaction_durable_with_attachments(transaction, root, limits, &[])
    }

    /// Persist objects and opaque owner bytes under the same save head before ACK.
    pub fn apply_object_transaction_durable_with_attachments(
        &mut self,
        transaction: &WorldObjectEditTransaction,
        root: impl AsRef<Path>,
        limits: IoLimits,
        updates: &[AttachmentUpdate],
    ) -> RuntimeResult<WorldChange> {
        let staged = self.stage_object_transaction(transaction)?;
        self.commit_object_durable(staged, root.as_ref(), limits, updates)
    }

    pub(crate) fn stage_object_transaction(
        &self,
        transaction: &WorldObjectEditTransaction,
    ) -> RuntimeResult<StagedEdit> {
        transaction.validate().map_err(RuntimeError::invalid)?;
        if transaction.edits.len() > self.config.max_edits_per_transaction
            || transaction
                .affected_columns()
                .map_err(RuntimeError::invalid)?
                .len()
                > self.config.max_edits_per_transaction
            || transaction.expected_revisions.len() > self.config.max_resident_chunks
        {
            return Err(RuntimeError::new(
                ErrorKind::Limit,
                "object transaction exceeds edit/dependency budget",
            ));
        }
        // Reject oversized complete records before cloning them into dependent packages.
        let _bounded =
            crate::source::encode_bounded(transaction, self.config.max_transaction_bytes)?;
        let fingerprint = request_fingerprint(transaction)?;
        if let Some(entry) = self.transactions.get(&transaction.id) {
            return if entry.descriptor.request_fingerprint == fingerprint {
                let applied = entry.load(&self.manifest)?;
                if applied.delta.object_edits != transaction.edits {
                    return Err(RuntimeError::new(
                        ErrorKind::Conflict,
                        "object request disagrees with recorded transaction body",
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
                    "transaction identity reused for a different command",
                ))
            };
        }
        let mut packages = BTreeMap::new();
        for (coordinate, expected) in &transaction.expected_revisions {
            let resident = self.resident.get(coordinate).ok_or_else(|| {
                RuntimeError::new(
                    ErrorKind::Unavailable,
                    format!("object edit requires unloaded dependency {coordinate:?}"),
                )
            })?;
            if resident.product.revision != *expected {
                return Err(RuntimeError::new(
                    ErrorKind::Conflict,
                    "object dependency revision is stale",
                ));
            }
            packages.insert(*coordinate, (*resident.product.package).clone());
        }
        let projections = transaction
            .edits
            .iter()
            .map(|edit| {
                let before = edit
                    .before
                    .as_ref()
                    .map(ObjectInstance::influences)
                    .transpose()
                    .map_err(RuntimeError::invalid)?
                    .unwrap_or_default();
                let after = edit
                    .after
                    .as_ref()
                    .map(ObjectInstance::influences)
                    .transpose()
                    .map_err(RuntimeError::invalid)?
                    .unwrap_or_default();
                Ok((edit, before, after))
            })
            .collect::<RuntimeResult<Vec<_>>>()?;
        // Check every old root and every clipped old record before modifying any candidate.
        for (edit, before, _) in &projections {
            let id = edit.object_id().map_err(RuntimeError::invalid)?;
            for (coordinate, package) in &packages {
                let prior = package
                    .semantics
                    .object_influences
                    .binary_search_by(|row| row.id.as_str().cmp(id))
                    .ok()
                    .and_then(|index| package.semantics.object_influences.get(index));
                if prior != before.get(coordinate) {
                    return Err(RuntimeError::new(
                        ErrorKind::Conflict,
                        "object before record or clipped source fingerprint is stale",
                    ));
                }
                let root = package
                    .semantics
                    .objects
                    .binary_search_by(|row| row.id.as_str().cmp(id))
                    .ok()
                    .and_then(|index| package.semantics.objects.get(index));
                let expected_root = edit
                    .before
                    .as_ref()
                    .filter(|object| object.origin.column.chunk() == *coordinate);
                if root != expected_root {
                    return Err(RuntimeError::new(
                        ErrorKind::Conflict,
                        "object before root record disagrees",
                    ));
                }
            }
        }
        for (edit, _, projected_after) in &projections {
            let id = edit.object_id().map_err(RuntimeError::invalid)?;
            for (coordinate, package) in &mut packages {
                package.semantics.objects.retain(|object| object.id != id);
                package
                    .semantics
                    .object_influences
                    .retain(|object| object.id != id);
                if let Some(after) = &edit.after {
                    if after.origin.column.chunk() == *coordinate {
                        package.semantics.objects.push(after.clone());
                    }
                    if let Some(influence) = projected_after.get(coordinate) {
                        package.semantics.object_influences.push(influence.clone());
                    }
                }
            }
        }
        for (coordinate, package) in &mut packages {
            package
                .semantics
                .object_influences
                .sort_by(|a, b| a.id.cmp(&b.id));
            package.semantics.occupancy =
                union_object_occupancy(&package.semantics.object_influences).map_err(|error| {
                    RuntimeError::new(
                        ErrorKind::Conflict,
                        format!("object overlap is incompatible: {error}"),
                    )
                })?;
            let resident = self
                .resident
                .get(coordinate)
                .ok_or_else(|| RuntimeError::invalid("preflighted dependency disappeared"))?;
            protect_semantics(&resident.product.package, package, &self.manifest_index)?;
        }
        self.stage_packages(
            packages,
            transaction.id.clone(),
            fingerprint,
            transaction.edits.clone(),
        )
    }
}

fn terrain_column(
    package: &ChunkPackage,
    position: hex_world_contracts::WorldHex,
) -> RuntimeResult<&ColumnData> {
    package
        .columns
        .binary_search_by_key(&position, |column| column.position)
        .ok()
        .and_then(|index| package.columns.get(index))
        .ok_or_else(|| RuntimeError::invalid("semantic support lacks its source column"))
}

fn protect_semantics(
    before: &ChunkPackage,
    after: &ChunkPackage,
    index: &ManifestIndex,
) -> RuntimeResult<()> {
    let combined = combined_object_columns(after)?;
    for anchor in &after.semantics.anchors {
        if anchor.role != AnchorRole::Observation {
            let column = combined
                .get(&anchor.position.column)
                .map(Ok)
                .unwrap_or_else(|| terrain_column(after, anchor.position.column))?;
            if !supports(column, anchor.position, index)? {
                return Err(RuntimeError::new(
                    ErrorKind::Conflict,
                    format!("object edit obstructs protected anchor {}", anchor.id),
                ));
            }
        }
    }
    for position in before
        .semantics
        .occupancy
        .iter()
        .chain(&after.semantics.occupancy)
        .map(|column| column.position)
    {
        let column = combined
            .get(&position)
            .map(Ok)
            .unwrap_or_else(|| terrain_column(after, position))?;
        for sample in index.boundary_samples_at(position) {
            let support = VoxelPosition {
                column: position,
                level: sample.ground_level,
            };
            let solid_ground = column.material_at(support.level).is_some_and(|material| {
                index
                    .material(material)
                    .is_ok_and(|material| material.solid)
            });
            let above_solid = support
                .level
                .checked_add(1)
                .and_then(|level| column.material_at(level))
                .is_some_and(|material| {
                    index
                        .material(material)
                        .is_ok_and(|material| material.solid)
                });
            if !solid_ground
                || above_solid
                || (sample.required_access && !supports(column, support, index)?)
            {
                return Err(RuntimeError::new(
                    ErrorKind::Conflict,
                    "object edit obstructs a protected boundary support",
                ));
            }
        }
    }
    // Interior membership stays an authored envelope. This initial object-edit lane
    // can remove obstructions but cannot introduce or replace occupied room-air runs;
    // that needs an explicit future interior regeneration policy.
    for interior in &after.semantics.interiors {
        let old = before
            .semantics
            .occupancy
            .iter()
            .find(|column| column.position == interior.column);
        let new = after
            .semantics
            .occupancy
            .iter()
            .find(|column| column.position == interior.column);
        if let Some(new) = new {
            for run in &new.runs {
                let bottom = i64::from(run.bottom).max(i64::from(interior.floor_level) + 1);
                let top = i64::from(run.top).min(i64::from(interior.roof_bottom));
                if bottom < top && !old.is_some_and(|old| covers(old, bottom, top, &run.material)) {
                    return Err(RuntimeError::new(
                        ErrorKind::Conflict,
                        format!(
                            "object edit requires interior regeneration for {}",
                            interior.id
                        ),
                    ));
                }
                if i64::from(run.bottom) <= i64::from(interior.floor_level)
                    && i64::from(run.top) > i64::from(interior.floor_level)
                    && !index
                        .material(&run.material)
                        .map_err(RuntimeError::invalid)?
                        .solid
                {
                    return Err(RuntimeError::new(
                        ErrorKind::Conflict,
                        "object edit removes solid interior support",
                    ));
                }
                if run.bottom < interior.roof_top
                    && run.top > interior.roof_bottom
                    && !index
                        .material(&run.material)
                        .map_err(RuntimeError::invalid)?
                        .solid
                {
                    return Err(RuntimeError::new(
                        ErrorKind::Conflict,
                        "object edit invalidates the interior roof",
                    ));
                }
            }
        }
    }
    Ok(())
}

fn covers(column: &ColumnData, bottom: i64, top: i64, material: &str) -> bool {
    column.runs.iter().any(|run| {
        i64::from(run.bottom) <= bottom && i64::from(run.top) >= top && run.material == material
    })
}
fn supports(
    column: &ColumnData,
    position: VoxelPosition,
    index: &ManifestIndex,
) -> RuntimeResult<bool> {
    let Some(material) = column.material_at(position.level) else {
        return Ok(false);
    };
    if !index
        .material(material)
        .map_err(RuntimeError::invalid)?
        .solid
    {
        return Ok(false);
    }
    for offset in 1..=2 {
        let Some(level) = position.level.checked_add(offset) else {
            return Ok(false);
        };
        if column.material_at(level).is_some() {
            return Ok(false);
        }
    }
    Ok(true)
}
