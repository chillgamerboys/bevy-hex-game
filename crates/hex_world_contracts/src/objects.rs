//! Exact identity-bearing object projections and bounded atomic object commands.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::validation::{name, ordered};
use crate::{
    hash_serializable, ChunkId, ColumnData, ContractError, ObjectInstance, Validate, VoxelPosition,
    VoxelRun, WorldHex, MAX_EDITS_PER_TRANSACTION, MAX_SEMANTIC_RECORDS,
};

/// Namespace reserved for transaction-allocated runtime objects, never authored roots.
pub const RUNTIME_OBJECT_PREFIX: &str = "@runtime/";

/// One object's exact contribution to a resident chunk, independent of its root residency.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectInfluence {
    /// Globally unique source object identity.
    pub id: String,
    /// Complete source record's canonical serialization fingerprint.
    pub source_fingerprint: u64,
    /// Global source root; the complete record is stored only in this root's chunk.
    pub origin: VoxelPosition,
    /// Source region of the root.
    pub region_id: String,
    /// Exact contribution clipped to this chunk, sorted by column.
    pub occupancy: Vec<ColumnData>,
}

impl ObjectInfluence {
    /// Validate canonical local shape; package admission checks membership and source policy.
    pub fn validate(&self) -> Result<(), ContractError> {
        name(&self.id, "object_influence.id")?;
        name(&self.region_id, "object_influence.region_id")?;
        if self.occupancy.len() > 256 {
            return Err(ContractError::new(
                "object_influence",
                "more than one chunk of columns",
            ));
        }
        ordered(
            self.occupancy.iter().map(|column| column.position),
            "object_influence.occupancy",
        )?;
        for column in &self.occupancy {
            column.validate()?;
        }
        Ok(())
    }
}

impl Validate for ObjectInfluence {
    fn validate(&self) -> Result<(), ContractError> {
        Self::validate(self)
    }
}

impl Validate for ObjectInstance {
    fn validate(&self) -> Result<(), ContractError> {
        name(&self.id, "object.id")?;
        name(&self.region_id, "object.region_id")?;
        name(&self.asset, "object.asset")?;
        if self.rotation >= 6 || self.occupancy.len() > MAX_SEMANTIC_RECORDS {
            return Err(ContractError::new(
                "object",
                "invalid rotation or excessive footprint",
            ));
        }
        ordered(
            self.occupancy.iter().map(|column| column.position),
            "object.occupancy",
        )?;
        for column in &self.occupancy {
            column.validate()?;
        }
        Ok(())
    }
}

impl ObjectInstance {
    /// Validate the canonical complete record without repairing its footprint.
    pub fn validate(&self) -> Result<(), ContractError> {
        <Self as Validate>::validate(self)
    }

    /// Every required owner/member chunk, including a root without occupied voxels.
    pub fn dependency_chunks(&self) -> Result<BTreeSet<ChunkId>, ContractError> {
        self.validate()?;
        Ok(std::iter::once(self.origin.column.chunk())
            .chain(self.occupancy.iter().map(|column| column.position.chunk()))
            .collect())
    }

    /// Produce the exact clipped contribution for one dependency chunk.
    pub fn influence(&self, coordinate: ChunkId) -> Result<Option<ObjectInfluence>, ContractError> {
        self.validate()?;
        let occupancy = self
            .occupancy
            .iter()
            .filter(|column| column.position.chunk() == coordinate)
            .cloned()
            .collect::<Vec<_>>();
        if occupancy.is_empty() && self.origin.column.chunk() != coordinate {
            return Ok(None);
        }
        Ok(Some(ObjectInfluence {
            id: self.id.clone(),
            source_fingerprint: hash_serializable(self)?,
            origin: self.origin,
            region_id: self.region_id.clone(),
            occupancy,
        }))
    }
}

/// Derive canonical shared occupancy while retaining independent contributing identities.
/// Overlapping equal materials coalesce; incompatible overlapping materials fail.
pub fn union_object_occupancy(
    influences: &[ObjectInfluence],
) -> Result<Vec<ColumnData>, ContractError> {
    let mut grouped: BTreeMap<WorldHex, Vec<VoxelRun>> = BTreeMap::new();
    for influence in influences {
        influence.validate()?;
        for column in &influence.occupancy {
            grouped
                .entry(column.position)
                .or_default()
                .extend(column.runs.iter().cloned());
        }
    }
    let mut output = Vec::new();
    for (position, mut runs) in grouped {
        runs.sort_by(|a, b| (a.bottom, a.top, &a.material).cmp(&(b.bottom, b.top, &b.material)));
        let mut union: Vec<VoxelRun> = Vec::new();
        for run in runs {
            if let Some(previous) = union.last_mut() {
                if previous.top > run.bottom && previous.material != run.material {
                    return Err(ContractError::new(
                        "object.occupancy",
                        "overlapping object materials disagree",
                    ));
                }
                if previous.top >= run.bottom && previous.material == run.material {
                    previous.top = previous.top.max(run.top);
                    continue;
                }
            }
            union.push(run);
        }
        if !union.is_empty() {
            let column = ColumnData {
                position,
                runs: union,
            };
            column.validate()?;
            output.push(column);
        }
    }
    Ok(output)
}

/// One addition, removal, or exact replacement of a complete authored-object record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectEdit {
    /// Exact old record, or none for a transaction-allocated new identity.
    pub before: Option<ObjectInstance>,
    /// Exact new record, or none to remove the identity permanently.
    pub after: Option<ObjectInstance>,
}

impl ObjectEdit {
    /// Validated stable edited identity; replacements must retain it.
    pub fn object_id(&self) -> Result<&str, ContractError> {
        let object = self
            .before
            .as_ref()
            .or(self.after.as_ref())
            .ok_or_else(|| ContractError::new("object_edit", "empty operation"))?;
        for record in self.before.iter().chain(&self.after) {
            record.validate()?;
            if record.id != object.id {
                return Err(ContractError::new(
                    "object_edit",
                    "replacement changes identity",
                ));
            }
        }
        if self.before == self.after {
            return Err(ContractError::new("object_edit", "unchanged object record"));
        }
        Ok(&object.id)
    }
}

/// Canonical object command with every old/new owner and footprint revision explicit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldObjectEditTransaction {
    /// Shared world transaction idempotency identity.
    pub id: String,
    /// Exactly all old/new owner and footprint chunks, even where occupancy overlaps.
    #[serde(deserialize_with = "crate::validation::deserialize_unique_map")]
    pub expected_revisions: BTreeMap<ChunkId, u64>,
    /// Unique operations in canonical object-ID order.
    pub edits: Vec<ObjectEdit>,
}

/// Allocate an addition's ID from its transaction and zero-based addition ordinal.
/// Ordinals count additions in canonical edit order, excluding removals/replacements.
pub fn runtime_object_id(
    transaction_id: &str,
    addition_ordinal: usize,
) -> Result<String, ContractError> {
    name(transaction_id, "object_transaction.id")?;
    if addition_ordinal >= MAX_EDITS_PER_TRANSACTION {
        return Err(ContractError::new(
            "object_transaction",
            "addition ordinal exceeds operation limit",
        ));
    }
    let id = format!("{RUNTIME_OBJECT_PREFIX}{transaction_id}/{addition_ordinal:05}");
    name(&id, "object.id")?;
    Ok(id)
}

impl WorldObjectEditTransaction {
    /// Unique affected owner/member columns; changes include identity-only replacements.
    pub fn affected_columns(&self) -> Result<BTreeSet<WorldHex>, ContractError> {
        let mut columns = BTreeSet::new();
        for edit in &self.edits {
            edit.object_id()?;
            for object in edit.before.iter().chain(&edit.after) {
                columns.insert(object.origin.column);
                columns.extend(object.occupancy.iter().map(|column| column.position));
                if columns.len() > MAX_EDITS_PER_TRANSACTION {
                    return Err(ContractError::new(
                        "object_transaction",
                        "affected column limit exceeded",
                    ));
                }
            }
        }
        Ok(columns)
    }

    /// Validate exact dependency coverage, allocation and canonical operation identity.
    pub fn validate(&self) -> Result<(), ContractError> {
        name(&self.id, "object_transaction.id")?;
        if self.edits.is_empty() || self.edits.len() > MAX_EDITS_PER_TRANSACTION {
            return Err(ContractError::new(
                "object_transaction",
                "expected bounded nonempty operations",
            ));
        }
        let mut ids = Vec::new();
        let mut additions = 0;
        for edit in &self.edits {
            let id = edit.object_id()?;
            ids.push(id);
            if edit.before.is_none() {
                if id != runtime_object_id(&self.id, additions)? {
                    return Err(ContractError::new(
                        "object_transaction",
                        "addition must use its transaction-allocated ID",
                    ));
                }
                additions += 1;
            }
        }
        ordered(ids, "object_transaction.edits")?;
        let required = self
            .affected_columns()?
            .into_iter()
            .map(WorldHex::chunk)
            .collect::<BTreeSet<_>>();
        if required != self.expected_revisions.keys().copied().collect() {
            return Err(ContractError::new(
                "object_transaction",
                "revisions must exactly cover old/new dependency chunks",
            ));
        }
        Ok(())
    }
}

impl Validate for WorldObjectEditTransaction {
    fn validate(&self) -> Result<(), ContractError> {
        Self::validate(self)
    }
}

pub(crate) fn project_objects(
    package: &crate::WorldPackage,
) -> Result<BTreeMap<ChunkId, Vec<ObjectInfluence>>, ContractError> {
    let mut projected: BTreeMap<ChunkId, Vec<ObjectInfluence>> = BTreeMap::new();
    for object in package
        .chunks
        .values()
        .flat_map(|chunk| &chunk.semantics.objects)
    {
        for coordinate in object.dependency_chunks()? {
            if !package.chunks.contains_key(&coordinate) {
                return Err(ContractError::new(
                    "world.object",
                    "missing dependency chunk",
                ));
            }
            if let Some(influence) = object.influence(coordinate)? {
                projected.entry(coordinate).or_default().push(influence);
            }
        }
    }
    for influences in projected.values_mut() {
        influences.sort_by(|a, b| a.id.cmp(&b.id));
    }
    Ok(projected)
}
