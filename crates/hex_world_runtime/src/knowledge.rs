//! Principal-private discovered surfaces and landmarks, independently persisted by chunk.

use std::{
    collections::BTreeMap,
    fs::{self, File},
    path::{Path, PathBuf},
    sync::Arc,
};

use hex_world_contracts::{
    hash_serializable, ChunkId, Surface, VoxelPosition, WorldHex, WorldManifest, SCHEMA_VERSION,
};
use serde::{Deserialize, Serialize};

use crate::{
    persistence::write_immutable,
    runtime::validate_identity,
    source::{
        atomic_write_head, checked_existing_path, encode_bounded, lock_directory, read_bounded,
        sync_directory,
    },
    CancellationToken, ErrorKind, IoLimits, RuntimeError, RuntimeResult,
};

/// One specifically observed support; availability of terrain grants no observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedSurface {
    /// Exact support and observed clearance, retaining stacked identity.
    pub surface: Surface,
    /// Terrain revision when this fact was observed; remembered facts may be older.
    pub world_revision: u64,
}

/// One specifically observed stable landmark, without undisclosed object geometry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedLandmark {
    /// Stable ID from the world feature registry.
    pub id: String,
    /// Exact observed landmark anchor.
    pub position: VoxelPosition,
    /// Terrain revision associated with the observation.
    pub world_revision: u64,
}

/// Complete private memory for one principal and chunk; not terrain authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnowledgePartition {
    /// Owning party/player/disclosure principal selected by the host.
    pub principal: String,
    /// Global storage chunk.
    pub coordinate: ChunkId,
    /// Monotonic knowledge revision, independent of terrain and combat turns.
    pub revision: u64,
    /// Columns actually discovered by this principal, including remembered empty space.
    pub discovered_columns: Vec<WorldHex>,
    /// Exact observed supports sorted by voxel position, retaining every known stack.
    pub surfaces: Vec<ObservedSurface>,
    /// Observed landmarks sorted by stable ID.
    pub landmarks: Vec<ObservedLandmark>,
    /// Canonical content hash with this field excluded.
    pub fingerprint: u64,
}

impl KnowledgePartition {
    /// Creates an unpublished revision-zero draft; set the next revision before sealing.
    #[must_use]
    pub fn new(principal: impl Into<String>, coordinate: ChunkId) -> Self {
        Self {
            principal: principal.into(),
            coordinate,
            revision: 0,
            discovered_columns: Vec::new(),
            surfaces: Vec::new(),
            landmarks: Vec::new(),
            fingerprint: 0,
        }
    }

    /// Canonicalizes producer ordering and seals a nonzero revision atomically.
    pub fn seal(&mut self) -> RuntimeResult<()> {
        let mut candidate = self.clone();
        candidate.discovered_columns.sort();
        candidate.surfaces.sort_by_key(|fact| fact.surface.position);
        candidate.landmarks.sort_by(|a, b| a.id.cmp(&b.id));
        candidate.fingerprint = candidate.expected_fingerprint()?;
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }

    /// Verifies canonical shape and integrity; store admission also verifies the manifest.
    pub fn validate(&self) -> RuntimeResult<()> {
        validate_identity(&self.principal)?;
        self.coordinate.origin().map_err(RuntimeError::invalid)?;
        if self.revision == 0 || self.fingerprint != self.expected_fingerprint()? {
            return Err(RuntimeError::invalid(
                "knowledge revision or fingerprint mismatch",
            ));
        }
        ensure_ordered(
            self.discovered_columns.iter().copied(),
            "discovered columns",
        )?;
        ensure_ordered(
            self.surfaces.iter().map(|fact| fact.surface.position),
            "observed surfaces",
        )?;
        ensure_ordered(
            self.landmarks.iter().map(|fact| fact.id.as_str()),
            "observed landmarks",
        )?;
        if self.discovered_columns.len() > 256 {
            return Err(RuntimeError::new(
                ErrorKind::Limit,
                "knowledge partition exceeds its column footprint",
            ));
        }
        for column in &self.discovered_columns {
            if column.chunk() != self.coordinate {
                return Err(RuntimeError::invalid("discovery belongs to another chunk"));
            }
        }
        for surface in &self.surfaces {
            if self
                .discovered_columns
                .binary_search(&surface.surface.position.column)
                .is_err()
                || surface.surface.headroom == Some(0)
            {
                return Err(RuntimeError::invalid(
                    "observed support lacks discovery or exposed clearance",
                ));
            }
        }
        for landmark in &self.landmarks {
            validate_identity(&landmark.id)?;
            if self
                .discovered_columns
                .binary_search(&landmark.position.column)
                .is_err()
            {
                return Err(RuntimeError::invalid(
                    "observed landmark lacks its discovered owner column",
                ));
            }
        }
        Ok(())
    }

    fn expected_fingerprint(&self) -> RuntimeResult<u64> {
        let mut value = self.clone();
        value.fingerprint = 0;
        hash_serializable(&value).map_err(RuntimeError::invalid)
    }
}

/// Per-operation and per-partition limits, independent of total discovered world size.
#[derive(Debug, Clone, Copy)]
pub struct KnowledgeConfig {
    /// Maximum chunks in an atomic observation batch or checkpoint page.
    pub max_partitions_per_operation: usize,
    /// Maximum exact remembered supports in one chunk.
    pub max_surfaces_per_partition: usize,
    /// Maximum remembered landmark IDs in one chunk.
    pub max_landmarks_per_partition: usize,
}

impl Default for KnowledgeConfig {
    fn default() -> Self {
        Self {
            max_partitions_per_operation: 64,
            max_surfaces_per_partition: 16_384,
            max_landmarks_per_partition: 4096,
        }
    }
}

/// Durable compare-and-write outcome, with no other principal's state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeReceipt {
    /// Principal whose private memory changed.
    pub principal: String,
    /// Exact idempotency identity.
    pub transaction_id: String,
    /// Affected chunks and published knowledge revisions in canonical order.
    pub revisions: Vec<(ChunkId, u64)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct KnowledgeDescriptor {
    pub principal: String,
    pub coordinate: ChunkId,
    pub revision: u64,
    pub fingerprint: u64,
    pub path: String,
    pub discovery: [u64; 4],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct KnowledgeRecord {
    pub principal: String,
    pub id: String,
    pub request_fingerprint: u64,
    pub receipt: KnowledgeReceipt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CheckpointProgress {
    pub snapshot_fingerprint: u64,
    pub watermark: u64,
    pub after: Option<ChunkId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct KnowledgeCursor {
    pub principal: String,
    pub stream_id: String,
    pub sequence: u64,
    pub checkpoint: Option<CheckpointProgress>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct KnowledgeHead {
    pub schema_version: u32,
    pub world_id: String,
    pub manifest_fingerprint: u64,
    pub partitions: Vec<KnowledgeDescriptor>,
    pub transactions: Vec<KnowledgeRecord>,
    pub cursors: Vec<KnowledgeCursor>,
    pub fingerprint: u64,
}

impl KnowledgeHead {
    fn expected_fingerprint(&self) -> RuntimeResult<u64> {
        let mut value = self.clone();
        value.fingerprint = 0;
        hash_serializable(&value).map_err(RuntimeError::invalid)
    }
    fn seal(&mut self) -> RuntimeResult<()> {
        self.partitions
            .sort_by(|a, b| (&a.principal, a.coordinate).cmp(&(&b.principal, b.coordinate)));
        self.transactions
            .sort_by(|a, b| (&a.principal, &a.id).cmp(&(&b.principal, &b.id)));
        self.cursors
            .sort_by(|a, b| (&a.principal, &a.stream_id).cmp(&(&b.principal, &b.stream_id)));
        self.fingerprint = self.expected_fingerprint()?;
        self.validate()
    }
    fn validate(&self) -> RuntimeResult<()> {
        if self.schema_version != SCHEMA_VERSION
            || self.fingerprint != self.expected_fingerprint()?
        {
            return Err(RuntimeError::invalid(
                "knowledge head schema or fingerprint mismatch",
            ));
        }
        ensure_ordered(
            self.partitions
                .iter()
                .map(|row| (&row.principal, row.coordinate)),
            "knowledge partitions",
        )?;
        ensure_ordered(
            self.transactions
                .iter()
                .map(|row| (&row.principal, &row.id)),
            "knowledge transactions",
        )?;
        ensure_ordered(
            self.cursors
                .iter()
                .map(|row| (&row.principal, &row.stream_id)),
            "knowledge cursors",
        )?;
        for descriptor in &self.partitions {
            validate_identity(&descriptor.principal)?;
            if descriptor.revision == 0
                || descriptor.path
                    != knowledge_path(
                        &descriptor.principal,
                        descriptor.coordinate,
                        descriptor.fingerprint,
                    )?
            {
                return Err(RuntimeError::invalid(
                    "knowledge descriptor has invalid revision or content path",
                ));
            }
        }
        for record in &self.transactions {
            validate_identity(&record.principal)?;
            validate_identity(&record.id)?;
            if record.receipt.principal != record.principal
                || record.receipt.transaction_id != record.id
            {
                return Err(RuntimeError::invalid("knowledge receipt identity mismatch"));
            }
            ensure_ordered(
                record.receipt.revisions.iter().map(|(chunk, _)| *chunk),
                "knowledge receipt revisions",
            )?;
            if record
                .receipt
                .revisions
                .iter()
                .any(|(_, revision)| *revision == 0)
            {
                return Err(RuntimeError::invalid("knowledge receipt has zero revision"));
            }
        }
        for cursor in &self.cursors {
            validate_identity(&cursor.principal)?;
            validate_identity(&cursor.stream_id)?;
        }
        Ok(())
    }
}

/// Durable principal/chunk knowledge store. It never owns undisclosed terrain.
/// Callers must choose an authorized principal; this local API is not authentication.
pub struct KnowledgeStore {
    pub(crate) root: PathBuf,
    pub(crate) manifest: WorldManifest,
    manifest_index: Arc<hex_world_contracts::ManifestIndex>,
    pub(crate) limits: IoLimits,
    pub(crate) config: KnowledgeConfig,
    pub(crate) head: KnowledgeHead,
    index: BTreeMap<(String, ChunkId), KnowledgeDescriptor>,
}

impl KnowledgeStore {
    /// Opens or initializes a source-bound store by reading index metadata only.
    pub fn open(
        root: impl AsRef<Path>,
        manifest: &WorldManifest,
        limits: IoLimits,
        config: KnowledgeConfig,
    ) -> RuntimeResult<Self> {
        let manifest_index = Arc::new(
            hex_world_contracts::ManifestIndex::new(Arc::new(manifest.clone()))
                .map_err(RuntimeError::invalid)?,
        );
        if config.max_partitions_per_operation == 0
            || config.max_surfaces_per_partition == 0
            || config.max_landmarks_per_partition == 0
        {
            return Err(RuntimeError::new(
                ErrorKind::Limit,
                "knowledge operation bounds must be nonzero",
            ));
        }
        fs::create_dir_all(root.as_ref()).map_err(RuntimeError::io)?;
        let root = root.as_ref().canonicalize().map_err(RuntimeError::io)?;
        let mut head = KnowledgeHead {
            schema_version: SCHEMA_VERSION,
            world_id: manifest.world_id.clone(),
            manifest_fingerprint: manifest.fingerprint,
            partitions: Vec::new(),
            transactions: Vec::new(),
            cursors: Vec::new(),
            fingerprint: 0,
        };
        head.seal()?;
        let mut store = Self {
            root,
            manifest: manifest.clone(),
            manifest_index,
            limits,
            config,
            head,
            index: BTreeMap::new(),
        };
        store.refresh()?;
        Ok(store)
    }

    /// Refreshes lightweight index metadata; no partition bodies are eagerly read.
    pub fn refresh(&mut self) -> RuntimeResult<()> {
        let head = self.read_head()?;
        self.install_head(head);
        Ok(())
    }

    /// Reads exactly one principal/chunk body and verifies it before returning it.
    pub fn read(
        &self,
        principal: &str,
        coordinate: ChunkId,
    ) -> RuntimeResult<Option<KnowledgePartition>> {
        validate_identity(principal)?;
        self.index
            .get(&(principal.to_owned(), coordinate))
            .map(|descriptor| self.read_descriptor(descriptor))
            .transpose()
    }

    /// Coarse discovery addresses for this principal alone, without loading fine memories.
    pub fn discovered_chunks(&self, principal: &str) -> RuntimeResult<Vec<ChunkId>> {
        validate_identity(principal)?;
        Ok(self
            .index
            .range(
                (
                    principal.to_owned(),
                    ChunkId {
                        q: i64::MIN,
                        r: i64::MIN,
                    },
                )
                    ..=(
                        principal.to_owned(),
                        ChunkId {
                            q: i64::MAX,
                            r: i64::MAX,
                        },
                    ),
            )
            .filter(|(_, descriptor)| descriptor.discovery.iter().any(|word| *word != 0))
            .map(|((_, coordinate), _)| *coordinate)
            .collect())
    }

    /// Exact discovered columns from a private compact mask, without reading observations.
    pub fn discovered_columns(
        &self,
        principal: &str,
        coordinate: ChunkId,
    ) -> RuntimeResult<Vec<WorldHex>> {
        validate_identity(principal)?;
        let Some(descriptor) = self.index.get(&(principal.to_owned(), coordinate)) else {
            return Ok(Vec::new());
        };
        let origin = coordinate.origin().map_err(RuntimeError::invalid)?;
        let mut columns = Vec::new();
        for local in 0..256_usize {
            if descriptor
                .discovery
                .get(local / 64)
                .is_some_and(|word| word & (1_u64 << (local % 64)) != 0)
            {
                columns.push(
                    origin
                        .checked_add(WorldHex::new(
                            i64::try_from(local / 16).map_err(RuntimeError::invalid)?,
                            i64::try_from(local % 16).map_err(RuntimeError::invalid)?,
                        ))
                        .map_err(RuntimeError::invalid)?,
                );
            }
        }
        Ok(columns)
    }

    /// Atomically replaces only the specified principal/chunk partitions and fsyncs before ACK.
    /// Expected revisions must exactly name the replacement chunks; IDs are idempotent.
    pub fn compare_and_write(
        &mut self,
        principal: &str,
        transaction_id: &str,
        expected_revisions: &BTreeMap<ChunkId, u64>,
        replacements: BTreeMap<ChunkId, KnowledgePartition>,
    ) -> RuntimeResult<KnowledgeReceipt> {
        validate_identity(principal)?;
        validate_identity(transaction_id)?;
        if replacements.len() > self.config.max_partitions_per_operation {
            return Err(RuntimeError::new(
                ErrorKind::Limit,
                "knowledge batch exceeds partition budget",
            ));
        }
        if expected_revisions.keys().ne(replacements.keys()) || replacements.is_empty() {
            return Err(RuntimeError::invalid(
                "knowledge CAS requires exact nonempty revision expectations",
            ));
        }
        // Bound all producer-supplied bodies before hashing or cloning the request.
        let mut body_bytes = 0_usize;
        for partition in replacements.values() {
            self.validate_partition(partition)?;
            body_bytes = body_bytes
                .saturating_add(encode_bounded(partition, self.limits.max_chunk_bytes)?.len());
            if body_bytes > self.limits.max_transaction_bytes {
                return Err(RuntimeError::new(
                    ErrorKind::Limit,
                    "knowledge CAS exceeds byte budget",
                ));
            }
        }
        let request_fingerprint =
            hash_serializable(&(principal, transaction_id, expected_revisions, &replacements))
                .map_err(RuntimeError::invalid)?;
        let (_writer, head) = self.locked_head()?;
        if let Some(receipt) =
            Self::duplicate(&head, principal, transaction_id, request_fingerprint)?
        {
            self.install_head(head);
            return Ok(receipt);
        }
        for (coordinate, partition) in &replacements {
            let current = Self::head_descriptor(&head, principal, *coordinate)
                .map_or(0, |descriptor| descriptor.revision);
            if expected_revisions.get(coordinate).copied() != Some(current)
                || current.checked_add(1) != Some(partition.revision)
            {
                return Err(RuntimeError::new(
                    ErrorKind::Conflict,
                    "knowledge expected or next revision disagrees",
                ));
            }
        }
        self.commit_prepared(
            head,
            principal,
            transaction_id,
            request_fingerprint,
            replacements,
        )
    }

    pub(crate) fn locked_head(&self) -> RuntimeResult<(File, KnowledgeHead)> {
        let writer = lock_directory(&self.root)?;
        let head = self.read_head()?;
        Ok((writer, head))
    }

    fn read_head(&self) -> RuntimeResult<KnowledgeHead> {
        if !self.root.join("knowledge.ron").exists() {
            if !self.head.partitions.is_empty()
                || !self.head.transactions.is_empty()
                || !self.head.cursors.is_empty()
            {
                return Err(RuntimeError::new(
                    ErrorKind::Io,
                    "committed knowledge head is missing",
                ));
            }
            return Ok(self.head.clone());
        }
        let path = checked_existing_path(&self.root, "knowledge.ron")?;
        let head: KnowledgeHead = read_bounded(
            &path,
            self.limits.max_manifest_bytes,
            &CancellationToken::default(),
        )?;
        head.validate()?;
        if head.world_id != self.manifest.world_id
            || head.manifest_fingerprint != self.manifest.fingerprint
        {
            return Err(RuntimeError::new(
                ErrorKind::Conflict,
                "knowledge belongs to another compiled world",
            ));
        }
        for descriptor in &head.partitions {
            if !self.manifest_index.contains_chunk(descriptor.coordinate) {
                return Err(RuntimeError::invalid(
                    "knowledge index names an outside-world chunk",
                ));
            }
            let origin = descriptor
                .coordinate
                .origin()
                .map_err(RuntimeError::invalid)?;
            for local in 0..256_usize {
                if descriptor
                    .discovery
                    .get(local / 64)
                    .is_some_and(|word| word & (1_u64 << (local % 64)) != 0)
                {
                    let position = origin
                        .checked_add(WorldHex::new(
                            i64::try_from(local / 16).map_err(RuntimeError::invalid)?,
                            i64::try_from(local % 16).map_err(RuntimeError::invalid)?,
                        ))
                        .map_err(RuntimeError::invalid)?;
                    if !self
                        .manifest_index
                        .contains(position)
                        .map_err(RuntimeError::invalid)?
                    {
                        return Err(RuntimeError::invalid(
                            "knowledge mask exposes outside-world discovery",
                        ));
                    }
                }
            }
        }
        Ok(head)
    }

    pub(crate) fn install_head(&mut self, head: KnowledgeHead) {
        self.index = head
            .partitions
            .iter()
            .map(|descriptor| {
                (
                    (descriptor.principal.clone(), descriptor.coordinate),
                    descriptor.clone(),
                )
            })
            .collect();
        self.head = head;
    }

    pub(crate) fn read_descriptor(
        &self,
        descriptor: &KnowledgeDescriptor,
    ) -> RuntimeResult<KnowledgePartition> {
        let path = checked_existing_path(&self.root, &descriptor.path)?;
        let partition: KnowledgePartition = read_bounded(
            &path,
            self.limits.max_chunk_bytes,
            &CancellationToken::default(),
        )?;
        self.validate_partition(&partition)?;
        if partition.principal != descriptor.principal
            || partition.coordinate != descriptor.coordinate
            || partition.revision != descriptor.revision
            || partition.fingerprint != descriptor.fingerprint
            || discovery_mask(&partition)? != descriptor.discovery
        {
            return Err(RuntimeError::invalid(
                "knowledge body disagrees with its principal/chunk index",
            ));
        }
        Ok(partition)
    }

    pub(crate) fn validate_partition(&self, partition: &KnowledgePartition) -> RuntimeResult<()> {
        if !self.manifest_index.contains_chunk(partition.coordinate) {
            return Err(RuntimeError::invalid(
                "knowledge partition belongs to an outside-world chunk",
            ));
        }
        if partition.discovered_columns.len() > 256
            || partition.surfaces.len() > self.config.max_surfaces_per_partition
            || partition.landmarks.len() > self.config.max_landmarks_per_partition
        {
            return Err(RuntimeError::new(
                ErrorKind::Limit,
                "knowledge partition exceeds observation budget",
            ));
        }
        let _bounded = encode_bounded(partition, self.limits.max_chunk_bytes)?;
        partition.validate()?;
        for position in &partition.discovered_columns {
            if !self
                .manifest_index
                .contains(*position)
                .map_err(RuntimeError::invalid)?
            {
                return Err(RuntimeError::invalid("discovery outside world footprint"));
            }
        }
        for fact in &partition.surfaces {
            if !self
                .manifest_index
                .material(&fact.surface.material)
                .map_err(RuntimeError::invalid)?
                .solid
            {
                return Err(RuntimeError::invalid(
                    "observed support uses a nonsolid material",
                ));
            }
        }
        for fact in &partition.landmarks {
            let landmark = self.manifest_index.feature(&fact.id).ok_or_else(|| {
                RuntimeError::invalid("observed landmark has no stable world feature ID")
            })?;
            if landmark.anchor != fact.position {
                return Err(RuntimeError::invalid(
                    "observed landmark position disagrees with registry",
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn head_descriptor<'a>(
        head: &'a KnowledgeHead,
        principal: &str,
        coordinate: ChunkId,
    ) -> Option<&'a KnowledgeDescriptor> {
        head.partitions
            .binary_search_by(|descriptor| {
                (descriptor.principal.as_str(), descriptor.coordinate).cmp(&(principal, coordinate))
            })
            .ok()
            .and_then(|index| head.partitions.get(index))
    }

    pub(crate) fn duplicate(
        head: &KnowledgeHead,
        principal: &str,
        id: &str,
        fingerprint: u64,
    ) -> RuntimeResult<Option<KnowledgeReceipt>> {
        let record = head
            .transactions
            .binary_search_by(|record| {
                (record.principal.as_str(), record.id.as_str()).cmp(&(principal, id))
            })
            .ok()
            .and_then(|index| head.transactions.get(index));
        match record {
            Some(record) if record.request_fingerprint == fingerprint => {
                Ok(Some(record.receipt.clone()))
            }
            Some(_) => Err(RuntimeError::new(
                ErrorKind::Conflict,
                "knowledge transaction identity reused for another payload",
            )),
            None => Ok(None),
        }
    }

    pub(crate) fn commit_prepared(
        &mut self,
        mut head: KnowledgeHead,
        principal: &str,
        id: &str,
        request_fingerprint: u64,
        replacements: BTreeMap<ChunkId, KnowledgePartition>,
    ) -> RuntimeResult<KnowledgeReceipt> {
        if replacements.len() > self.config.max_partitions_per_operation {
            return Err(RuntimeError::new(
                ErrorKind::Limit,
                "knowledge batch exceeds partition budget",
            ));
        }
        let mut prepared = Vec::new();
        for (coordinate, partition) in &replacements {
            self.validate_partition(partition)?;
            if partition.principal != principal || partition.coordinate != *coordinate {
                return Err(RuntimeError::new(
                    ErrorKind::Conflict,
                    "knowledge replacement belongs to another principal or chunk",
                ));
            }
            if let Some(previous) = Self::head_descriptor(&head, principal, *coordinate) {
                if partition.revision < previous.revision
                    || (partition.revision == previous.revision
                        && partition.fingerprint != previous.fingerprint)
                {
                    return Err(RuntimeError::new(
                        ErrorKind::Conflict,
                        "knowledge replacement rolls back or conflicts with a revision",
                    ));
                }
                let old = self.read_descriptor(previous)?;
                for fact in &partition.surfaces {
                    if old
                        .surfaces
                        .binary_search_by_key(&fact.surface.position, |prior| {
                            prior.surface.position
                        })
                        .ok()
                        .and_then(|index| old.surfaces.get(index))
                        .is_some_and(|prior| prior.world_revision > fact.world_revision)
                    {
                        return Err(RuntimeError::new(
                            ErrorKind::Conflict,
                            "observation rolls back a known terrain revision",
                        ));
                    }
                }
                for fact in &partition.landmarks {
                    if old
                        .landmarks
                        .binary_search_by(|prior| prior.id.cmp(&fact.id))
                        .ok()
                        .and_then(|index| old.landmarks.get(index))
                        .is_some_and(|prior| prior.world_revision > fact.world_revision)
                    {
                        return Err(RuntimeError::new(
                            ErrorKind::Conflict,
                            "observation rolls back a known landmark revision",
                        ));
                    }
                }
            }
            let descriptor = KnowledgeDescriptor {
                principal: principal.to_owned(),
                coordinate: *coordinate,
                revision: partition.revision,
                fingerprint: partition.fingerprint,
                path: knowledge_path(principal, *coordinate, partition.fingerprint)?,
                discovery: discovery_mask(partition)?,
            };
            prepared.push((
                descriptor,
                encode_bounded(partition, self.limits.max_chunk_bytes)?,
            ));
        }
        let receipt = KnowledgeReceipt {
            principal: principal.to_owned(),
            transaction_id: id.to_owned(),
            revisions: replacements
                .iter()
                .map(|(coordinate, partition)| (*coordinate, partition.revision))
                .collect(),
        };
        for (descriptor, _) in &prepared {
            head.partitions.retain(|prior| {
                prior.principal != principal || prior.coordinate != descriptor.coordinate
            });
            head.partitions.push(descriptor.clone());
        }
        head.transactions.push(KnowledgeRecord {
            principal: principal.to_owned(),
            id: id.to_owned(),
            request_fingerprint,
            receipt: receipt.clone(),
        });
        head.seal()?;
        // Preflight the bounded head before any publication; payload files are immutable.
        let _head_bytes = encode_bounded(&head, self.limits.max_manifest_bytes)?;
        for (descriptor, bytes) in prepared {
            let relative = Path::new(&descriptor.path);
            let directory = relative
                .parent()
                .ok_or_else(|| RuntimeError::invalid("knowledge path lacks directory"))?;
            let path = crate::source::ensure_relative_directory(&self.root, directory)?;
            write_immutable(&self.root, &descriptor.path, &bytes)?;
            sync_directory(&path)?;
            if let Some(parent) = path.parent() {
                sync_directory(parent)?;
            }
        }
        atomic_write_head(
            &self.root,
            "knowledge.ron",
            &head,
            self.limits.max_manifest_bytes,
        )?;
        self.install_head(head);
        Ok(receipt)
    }
}

fn knowledge_path(principal: &str, coordinate: ChunkId, fingerprint: u64) -> RuntimeResult<String> {
    let principal_hash = hash_serializable(principal).map_err(RuntimeError::invalid)?;
    Ok(format!(
        "knowledge/{principal_hash:016x}/{}_{}-{fingerprint:016x}.ron",
        coordinate.q, coordinate.r
    ))
}

fn discovery_mask(partition: &KnowledgePartition) -> RuntimeResult<[u64; 4]> {
    let mut mask = [0_u64; 4];
    for column in &partition.discovered_columns {
        let (q, r) = column.local();
        let local = usize::try_from(q * 16 + r).map_err(RuntimeError::invalid)?;
        let word = mask
            .get_mut(local / 64)
            .ok_or_else(|| RuntimeError::invalid("discovery local index outside chunk"))?;
        *word |= 1_u64 << (local % 64);
    }
    Ok(mask)
}

pub(crate) fn ensure_ordered<T: Ord>(
    values: impl IntoIterator<Item = T>,
    name: &str,
) -> RuntimeResult<()> {
    let mut previous = None;
    for value in values {
        if previous.as_ref().is_some_and(|prior| prior >= &value) {
            return Err(RuntimeError::invalid(format!(
                "{name} must be sorted and unique"
            )));
        }
        previous = Some(value);
    }
    Ok(())
}
