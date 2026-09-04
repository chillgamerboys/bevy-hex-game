//! Immutable partition files with one atomically replaced durable save head.

use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use atomicwrites::{AllowOverwrite, AtomicFile};
use hex_world_contracts::{
    hash_serializable, ChunkId, ChunkPackage, ColumnData, WorldChange, WorldEditTransaction,
    WorldManifest, SCHEMA_VERSION,
};
use serde::{Deserialize, Serialize};

use crate::{
    edits::{exact_column_mut, AppliedTransaction, StagedEdit},
    source::{
        checked_existing_path, checked_relative_path, encode_bounded, read_bounded,
        read_bytes_bounded, sync_directory, write_new,
    },
    CancellationToken, ErrorKind, IoLimits, RuntimeError, RuntimeResult, WorldRuntime,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChunkOverlay {
    pub schema_version: u32,
    pub world_id: String,
    pub coordinate: ChunkId,
    pub base_fingerprint: u64,
    pub revision: u64,
    pub target_fingerprint: u64,
    pub columns: Vec<ColumnData>,
    pub fingerprint: u64,
}

impl ChunkOverlay {
    fn expected_fingerprint(&self) -> RuntimeResult<u64> {
        let mut value = self.clone();
        value.fingerprint = 0;
        hash_serializable(&value).map_err(RuntimeError::invalid)
    }

    pub(crate) fn seal(&mut self) -> RuntimeResult<()> {
        self.fingerprint = self.expected_fingerprint()?;
        self.validate()
    }

    fn validate(&self) -> RuntimeResult<()> {
        if self.schema_version != SCHEMA_VERSION
            || self.revision == 0
            || self.columns.is_empty()
            || self.columns.len() > 256
            || self.fingerprint != self.expected_fingerprint()?
        {
            return Err(RuntimeError::invalid(
                "partition schema, revision, column count or fingerprint mismatch",
            ));
        }
        if self.columns.windows(2).any(|pair| {
            pair.first()
                .zip(pair.get(1))
                .is_some_and(|(a, b)| a.position >= b.position)
        }) {
            return Err(RuntimeError::invalid(
                "partition columns not canonical and unique",
            ));
        }
        for column in &self.columns {
            column.validate().map_err(RuntimeError::invalid)?;
            if column.position.chunk() != self.coordinate {
                return Err(RuntimeError::invalid("partition contains foreign column"));
            }
        }
        Ok(())
    }

    pub(crate) fn apply(
        &self,
        package: &mut ChunkPackage,
        manifest: &WorldManifest,
    ) -> RuntimeResult<()> {
        self.validate()?;
        if self.world_id != package.world_id
            || self.coordinate != package.coordinate
            || self.base_fingerprint != package.fingerprint
        {
            return Err(RuntimeError::new(
                ErrorKind::Conflict,
                "saved partition belongs to a different compiled base",
            ));
        }
        for column in &self.columns {
            *exact_column_mut(package, column.position)? = column.clone();
        }
        package.seal().map_err(RuntimeError::invalid)?;
        package
            .validate_against_manifest(manifest)
            .map_err(RuntimeError::invalid)?;
        if package.fingerprint != self.target_fingerprint {
            return Err(RuntimeError::invalid(
                "saved partition reconstructed the wrong package",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OverlayDescriptor {
    coordinate: ChunkId,
    revision: u64,
    base_fingerprint: u64,
    fingerprint: u64,
    path: String,
}

#[derive(Debug, Clone)]
pub(crate) enum OverlayLocation {
    Memory(ChunkOverlay),
    Disk {
        root: PathBuf,
        descriptor: OverlayDescriptor,
        limits: IoLimits,
    },
}

impl OverlayLocation {
    pub(crate) fn revision(&self) -> u64 {
        match self {
            Self::Memory(value) => value.revision,
            Self::Disk { descriptor, .. } => descriptor.revision,
        }
    }
    pub(crate) fn base_fingerprint(&self) -> u64 {
        match self {
            Self::Memory(value) => value.base_fingerprint,
            Self::Disk { descriptor, .. } => descriptor.base_fingerprint,
        }
    }
    pub(crate) fn load(&self, cancellation: &CancellationToken) -> RuntimeResult<ChunkOverlay> {
        match self {
            Self::Memory(value) => {
                cancellation.check()?;
                Ok(value.clone())
            }
            Self::Disk {
                root,
                descriptor,
                limits,
            } => {
                let path = checked_existing_path(root, &descriptor.path)?;
                let value: ChunkOverlay =
                    read_bounded(&path, limits.max_chunk_bytes, cancellation)?;
                value.validate()?;
                if value.coordinate != descriptor.coordinate
                    || value.revision != descriptor.revision
                    || value.base_fingerprint != descriptor.base_fingerprint
                    || value.fingerprint != descriptor.fingerprint
                {
                    return Err(RuntimeError::invalid(
                        "saved partition identity disagrees with save head",
                    ));
                }
                Ok(value)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TransactionDescriptor {
    id: String,
    fingerprint: u64,
    path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SaveHead {
    schema_version: u32,
    world_id: String,
    manifest_fingerprint: u64,
    // Vectors keep duplicate wire keys observable rather than silently overwriting.
    partitions: Vec<OverlayDescriptor>,
    transactions: Vec<TransactionDescriptor>,
    fingerprint: u64,
}

impl SaveHead {
    fn expected_fingerprint(&self) -> RuntimeResult<u64> {
        let mut value = self.clone();
        value.fingerprint = 0;
        hash_serializable(&value).map_err(RuntimeError::invalid)
    }

    fn validate(&self) -> RuntimeResult<()> {
        if self.schema_version != SCHEMA_VERSION
            || self.fingerprint != self.expected_fingerprint()?
        {
            return Err(RuntimeError::invalid(
                "save head schema or fingerprint mismatch",
            ));
        }
        if self.partitions.windows(2).any(|pair| {
            pair.first()
                .zip(pair.get(1))
                .is_some_and(|(a, b)| a.coordinate >= b.coordinate)
        }) || self.transactions.windows(2).any(|pair| {
            pair.first()
                .zip(pair.get(1))
                .is_some_and(|(a, b)| a.id >= b.id)
        }) {
            return Err(RuntimeError::invalid(
                "save head contains duplicate or unordered identities",
            ));
        }
        for descriptor in &self.partitions {
            let _path = checked_relative_path(Path::new("."), &descriptor.path)?;
            if descriptor.revision == 0 {
                return Err(RuntimeError::invalid("saved partition has base revision"));
            }
        }
        for descriptor in &self.transactions {
            let _path = checked_relative_path(Path::new("."), &descriptor.path)?;
        }
        Ok(())
    }
}

impl WorldRuntime {
    /// Saves a fresh V4 checkpoint with independently addressable modified chunks.
    ///
    /// Immutable content files are flushed first; `current.ron` is then atomically
    /// replaced. A crash before that switch preserves the previous checkpoint.
    /// Unchanged partition content keeps its exact path and is never rewritten.
    /// Historical orphan content is retained; garbage collection is a separate job.
    pub fn save(&mut self, root: impl AsRef<Path>, limits: IoLimits) -> RuntimeResult<()> {
        let locations = self.checkpoint(root.as_ref(), limits, None)?;
        self.demote_persisted_unloaded(locations);
        Ok(())
    }

    /// Commits a terrain transaction durably before returning its acknowledgment.
    /// Any preparation or checkpoint failure leaves in-memory authority unchanged.
    pub fn apply_transaction_durable(
        &mut self,
        transaction: &WorldEditTransaction,
        root: impl AsRef<Path>,
        limits: IoLimits,
    ) -> RuntimeResult<WorldChange> {
        let staged = self.stage_transaction(transaction)?;
        let locations = self.checkpoint(root.as_ref(), limits, Some(&staged))?;
        let change = self.commit_edit(staged);
        self.demote_persisted_unloaded(locations);
        Ok(change)
    }

    /// Applies a remote local delta durably before returning its acknowledgment.
    /// Stale/out-of-order revisions and mismatched repeated IDs publish nothing.
    pub fn apply_delta_durable(
        &mut self,
        delta: &crate::WorldDelta,
        root: impl AsRef<Path>,
        limits: IoLimits,
    ) -> RuntimeResult<WorldChange> {
        let staged = self.stage_delta(delta)?;
        let locations = self.checkpoint(root.as_ref(), limits, Some(&staged))?;
        let change = self.commit_edit(staged);
        self.demote_persisted_unloaded(locations);
        Ok(change)
    }

    /// Restores a source-matching save head and idempotency journal atomically.
    ///
    /// Chunk terrain and saved column payloads remain lazy; their bounded worker
    /// reads verify hashes before admission. Corrupt lazy partitions produce a
    /// failed load, never a partially queryable chunk. Restore requires no running
    /// jobs or operation pins; resident engine products are retired on next pump.
    pub fn restore_save(&mut self, root: impl AsRef<Path>, limits: IoLimits) -> RuntimeResult<()> {
        if self.has_running_jobs() {
            return Err(RuntimeError::new(
                ErrorKind::Conflict,
                "restore requires drained background jobs",
            ));
        }
        if self.has_pins() {
            return Err(RuntimeError::new(
                ErrorKind::Pinned,
                "restore cannot invalidate operation pins",
            ));
        }
        let root = root.as_ref().canonicalize().map_err(RuntimeError::io)?;
        let head_path = checked_existing_path(&root, "current.ron")?;
        let head: SaveHead = read_bounded(
            &head_path,
            limits.max_manifest_bytes,
            &CancellationToken::default(),
        )?;
        head.validate()?;
        if head.world_id != self.manifest.world_id
            || head.manifest_fingerprint != self.manifest.fingerprint
        {
            return Err(RuntimeError::new(
                ErrorKind::Conflict,
                "save belongs to a different compiled world",
            ));
        }
        let mut overlays = BTreeMap::new();
        for descriptor in head.partitions {
            if self
                .descriptors
                .get(&descriptor.coordinate)
                .map(|chunk| chunk.fingerprint)
                != Some(descriptor.base_fingerprint)
            {
                return Err(RuntimeError::new(
                    ErrorKind::Conflict,
                    "saved partition base is absent or changed",
                ));
            }
            let _safe_existing = checked_existing_path(&root, &descriptor.path)?;
            overlays.insert(
                descriptor.coordinate,
                OverlayLocation::Disk {
                    root: root.clone(),
                    descriptor,
                    limits,
                },
            );
        }
        let mut transactions = BTreeMap::new();
        for descriptor in head.transactions {
            let path = checked_existing_path(&root, &descriptor.path)?;
            let record: AppliedTransaction = read_bounded(
                &path,
                limits.max_transaction_bytes,
                &CancellationToken::default(),
            )?;
            record.delta.validate()?;
            record.change.validate().map_err(RuntimeError::invalid)?;
            if hash_serializable(&record).map_err(RuntimeError::invalid)? != descriptor.fingerprint
                || record.change.transaction_id != descriptor.id
                || record.delta.transaction_id != descriptor.id
                || record.delta.world_id != self.manifest.world_id
                || record.delta.manifest_fingerprint != self.manifest.fingerprint
                || record.request_fingerprint != record.delta.request_fingerprint
            {
                return Err(RuntimeError::invalid(
                    "saved transaction identity or fingerprint mismatch",
                ));
            }
            let expected_revisions = record
                .delta
                .chunks
                .iter()
                .map(|chunk| (chunk.coordinate, chunk.revision))
                .collect::<BTreeMap<_, _>>();
            let expected_columns = record
                .delta
                .chunks
                .iter()
                .flat_map(|chunk| chunk.columns.iter().map(|column| column.position))
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            if record.change.revisions != expected_revisions
                || record.change.changed_columns != expected_columns
                || record.delta.chunks.iter().any(|chunk| {
                    overlays
                        .get(&chunk.coordinate)
                        .is_none_or(|overlay| overlay.revision() < chunk.revision)
                })
            {
                return Err(RuntimeError::invalid(
                    "save journal outcome disagrees with partition revisions",
                ));
            }
            transactions.insert(descriptor.id, record);
        }
        self.persisted = overlays.clone();
        self.overlays = overlays;
        self.transactions = transactions;
        self.dirty.clear();
        self.invalidate_after_restore();
        Ok(())
    }

    fn demote_persisted_unloaded(&mut self, locations: BTreeMap<ChunkId, OverlayLocation>) {
        self.persisted = locations.clone();
        self.dirty.clear();
        for (coordinate, location) in locations {
            if !self.resident.contains_key(&coordinate) {
                self.overlays.insert(coordinate, location);
            }
        }
    }

    fn checkpoint(
        &self,
        root: &Path,
        limits: IoLimits,
        staged: Option<&StagedEdit>,
    ) -> RuntimeResult<BTreeMap<ChunkId, OverlayLocation>> {
        fs::create_dir_all(root).map_err(RuntimeError::io)?;
        let root = root.canonicalize().map_err(RuntimeError::io)?;
        if let Some(parent) = root.parent() {
            sync_directory(parent)?;
        }
        let lock_path = root.join("writer.lock");
        if lock_path.exists() {
            let _safe = checked_existing_path(&root, "writer.lock")?;
        }
        let writer_lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(RuntimeError::io)?;
        writer_lock.try_lock().map_err(|error| {
            RuntimeError::new(
                ErrorKind::Conflict,
                format!("save already has a writer or cannot be locked: {error}"),
            )
        })?;
        if root.join("current.ron").exists() {
            let path = checked_existing_path(&root, "current.ron")?;
            let existing: SaveHead = read_bounded(
                &path,
                limits.max_manifest_bytes,
                &CancellationToken::default(),
            )?;
            existing.validate()?;
            if existing.world_id != self.manifest.world_id
                || existing.manifest_fingerprint != self.manifest.fingerprint
            {
                return Err(RuntimeError::new(
                    ErrorKind::Conflict,
                    "save destination already belongs to another compiled world",
                ));
            }
            for descriptor in &existing.transactions {
                let record = self.transactions.get(&descriptor.id).or_else(|| {
                    staged
                        .filter(|edit| edit.applied.change.transaction_id == descriptor.id)
                        .map(|edit| &edit.applied)
                });
                if record
                    .map(hash_serializable)
                    .transpose()
                    .map_err(RuntimeError::invalid)?
                    != Some(descriptor.fingerprint)
                {
                    return Err(RuntimeError::new(ErrorKind::Conflict, "save has acknowledged transactions absent from this authority; restore before writing"));
                }
            }
            for descriptor in &existing.partitions {
                let revision = staged
                    .and_then(|edit| edit.chunks.get(&descriptor.coordinate))
                    .map_or_else(
                        || self.overlay_revision(descriptor.coordinate),
                        |(_, overlay, _)| overlay.revision,
                    );
                if revision < descriptor.revision {
                    return Err(RuntimeError::new(
                        ErrorKind::Conflict,
                        "save would roll back a durable partition",
                    ));
                }
            }
        }
        let partition_directory = root.join("partitions");
        let journal_directory = root.join("transactions");
        fs::create_dir_all(&partition_directory).map_err(RuntimeError::io)?;
        fs::create_dir_all(&journal_directory).map_err(RuntimeError::io)?;
        // Never let save writes follow an existing content-directory symlink outward.
        for directory in [&partition_directory, &journal_directory] {
            if !directory
                .canonicalize()
                .map_err(RuntimeError::io)?
                .starts_with(&root)
            {
                return Err(RuntimeError::invalid("save directory escapes root"));
            }
        }
        let mut selected = self.overlays.clone();
        if let Some(staged) = staged {
            for (coordinate, (_, overlay, _)) in &staged.chunks {
                selected.insert(*coordinate, OverlayLocation::Memory(overlay.clone()));
            }
        }
        let mut descriptors = Vec::new();
        let mut locations = BTreeMap::new();
        for (coordinate, location) in selected {
            if let Some(OverlayLocation::Disk {
                root: saved_root,
                descriptor,
                ..
            }) = self.persisted.get(&coordinate)
            {
                if saved_root == &root && descriptor.revision == location.revision() {
                    let _safe_existing = checked_existing_path(&root, &descriptor.path)?;
                    descriptors.push(descriptor.clone());
                    locations.insert(
                        coordinate,
                        OverlayLocation::Disk {
                            root: root.clone(),
                            descriptor: descriptor.clone(),
                            limits,
                        },
                    );
                    continue;
                }
            }
            let overlay = location.load(&CancellationToken::default())?;
            overlay.validate()?;
            let path = format!(
                "partitions/{}_{}-{:016x}.ron",
                coordinate.q, coordinate.r, overlay.fingerprint
            );
            write_immutable(
                &root,
                &path,
                &encode_bounded(&overlay, limits.max_chunk_bytes)?,
            )?;
            let descriptor = OverlayDescriptor {
                coordinate,
                revision: overlay.revision,
                base_fingerprint: overlay.base_fingerprint,
                fingerprint: overlay.fingerprint,
                path,
            };
            descriptors.push(descriptor.clone());
            locations.insert(
                coordinate,
                OverlayLocation::Disk {
                    root: root.clone(),
                    descriptor,
                    limits,
                },
            );
        }
        let mut selected_transactions = self
            .transactions
            .iter()
            .map(|(id, record)| (id.as_str(), record))
            .collect::<BTreeMap<_, _>>();
        if let Some(staged) = staged {
            selected_transactions.insert(&staged.applied.change.transaction_id, &staged.applied);
        }
        let mut transactions = Vec::new();
        for (id, record) in selected_transactions {
            let fingerprint = hash_serializable(record).map_err(RuntimeError::invalid)?;
            let path = format!("transactions/{fingerprint:016x}.ron");
            write_immutable(
                &root,
                &path,
                &encode_bounded(record, limits.max_transaction_bytes)?,
            )?;
            transactions.push(TransactionDescriptor {
                id: id.to_owned(),
                fingerprint,
                path,
            });
        }
        sync_directory(&partition_directory)?;
        sync_directory(&journal_directory)?;
        let mut head = SaveHead {
            schema_version: SCHEMA_VERSION,
            world_id: self.manifest.world_id.clone(),
            manifest_fingerprint: self.manifest.fingerprint,
            partitions: descriptors,
            transactions,
            fingerprint: 0,
        };
        head.fingerprint = head.expected_fingerprint()?;
        head.validate()?;
        let bytes = encode_bounded(&head, limits.max_manifest_bytes)?;
        AtomicFile::new(root.join("current.ron"), AllowOverwrite)
            .write(|file| {
                file.write_all(&bytes)?;
                file.sync_all()
            })
            .map_err(RuntimeError::io)?;
        sync_directory(&root)?;
        Ok(locations)
    }
}

fn write_immutable(root: &Path, relative: &str, bytes: &[u8]) -> RuntimeResult<()> {
    let path = checked_relative_path(root, relative)?;
    if path.exists() {
        let path = checked_existing_path(root, relative)?;
        let metadata = fs::metadata(&path).map_err(RuntimeError::io)?;
        if metadata.len() != u64::try_from(bytes.len()).unwrap_or(u64::MAX)
            || read_bytes_bounded(&path, bytes.len(), &CancellationToken::default())? != bytes
        {
            return Err(RuntimeError::invalid(
                "existing immutable save file disagrees with its content address",
            ));
        }
        return Ok(());
    }
    write_new(&path, bytes)
}
