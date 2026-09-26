//! Atomic, partitioned opaque owner records for a complete finite play session.
//!
//! This uses the same content-addressed files, OS lock and durable atomic-head
//! publication as ordinary world saves. Finite arena destruction is deliberately
//! not replayed through the stricter authoring-edit/semantic-regeneration path.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use hex_world_contracts::hash_serializable;
use serde::{Deserialize, Serialize};

use crate::{
    CancellationToken, ErrorKind, RuntimeError, RuntimeResult,
    persistence::write_immutable,
    runtime::validate_identity,
    source::{
        atomic_write_head, checked_existing_path, ensure_relative_directory, lock_directory,
        read_bounded, read_bytes_bounded, sync_directory,
    },
};

const CHECKPOINT_SCHEMA: u32 = 1;
const HEAD_FILE: &str = "session.ron";

/// Exact immutable world and owner-composition version required by this slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointIdentity {
    /// Stable generated world identity.
    pub world_id: String,
    /// Exact compiled world-manifest fingerprint, including content and source recipes.
    pub manifest_fingerprint: u64,
    /// Nonzero application-owned version of the complete session composition.
    pub content_version: u32,
}

impl CheckpointIdentity {
    fn validate(&self) -> RuntimeResult<()> {
        validate_identity(&self.world_id)?;
        if self.content_version == 0 {
            return Err(RuntimeError::invalid(
                "checkpoint content version must be nonzero",
            ));
        }
        Ok(())
    }
}

/// Explicit storage limits independent of whole-world terrain or the ordinary
/// 64-attachment transaction budget. Bodies are written/read one record at a time.
#[derive(Debug, Clone, Copy)]
pub struct CheckpointLimits {
    /// Maximum encoded metadata head size.
    pub max_head_bytes: usize,
    /// Maximum size of one owner payload.
    pub max_record_bytes: usize,
    /// Maximum record count in a complete checkpoint.
    pub max_records: usize,
    /// Maximum sum of owner payload bytes in a complete checkpoint.
    pub max_total_bytes: u64,
}

impl Default for CheckpointLimits {
    fn default() -> Self {
        Self {
            max_head_bytes: 64 * 1024 * 1024,
            max_record_bytes: 8 * 1024 * 1024,
            max_records: 131_072,
            max_total_bytes: 512 * 1024 * 1024,
        }
    }
}

impl CheckpointLimits {
    fn validate(self) -> RuntimeResult<()> {
        if self.max_head_bytes == 0
            || self.max_record_bytes == 0
            || self.max_records == 0
            || self.max_total_bytes == 0
        {
            return Err(RuntimeError::invalid("checkpoint limits must be positive"));
        }
        Ok(())
    }
}

/// Compare-and-write identity of the complete durable head, including its sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointToken {
    /// Monotonically increasing durable generation.
    pub generation: u64,
    /// Fingerprint of the exact canonical metadata head.
    pub fingerprint: u64,
}

/// One independently encoded owner partition. Runtime never decodes these bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerRecord {
    /// Stable subsystem namespace, for example `gameplay` or `world`.
    pub owner: String,
    /// Unique key inside the owner namespace.
    pub key: String,
    /// Exact owner-defined codec/version identifier, checked again when reading.
    pub format: String,
    /// Bounded owner payload; no complete generated terrain/mesh copy is required.
    pub bytes: Vec<u8>,
}

/// Lightweight immutable metadata for one independently loadable owner partition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerRecordDescriptor {
    /// Stable subsystem namespace.
    pub owner: String,
    /// Unique owner-local key.
    pub key: String,
    /// Exact owner codec/version identifier.
    pub format: String,
    /// Content fingerprint of the opaque body.
    pub fingerprint: u64,
    /// Exact body length in bytes.
    pub bytes: usize,
}

impl OwnerRecordDescriptor {
    fn validate(&self, limits: CheckpointLimits) -> RuntimeResult<()> {
        validate_identity(&self.owner)?;
        validate_identity(&self.key)?;
        validate_identity(&self.format)?;
        if self.bytes > limits.max_record_bytes {
            return Err(RuntimeError::new(
                ErrorKind::Limit,
                "checkpoint record byte limit exceeded",
            ));
        }
        Ok(())
    }

    fn path(&self) -> String {
        format!("session-records/{:016x}.bin", self.fingerprint)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionHead {
    schema_version: u32,
    identity: CheckpointIdentity,
    generation: u64,
    records: Vec<OwnerRecordDescriptor>,
    fingerprint: u64,
}

impl SessionHead {
    fn expected_fingerprint(&self) -> RuntimeResult<u64> {
        let mut canonical = self.clone();
        canonical.fingerprint = 0;
        hash_serializable(&canonical).map_err(RuntimeError::invalid)
    }

    fn token(&self) -> CheckpointToken {
        CheckpointToken {
            generation: self.generation,
            fingerprint: self.fingerprint,
        }
    }

    fn validate(&self, limits: CheckpointLimits) -> RuntimeResult<()> {
        self.identity.validate()?;
        if self.schema_version != CHECKPOINT_SCHEMA || self.generation == 0 {
            return Err(RuntimeError::invalid(
                "unsupported session schema or generation",
            ));
        }
        if self.records.len() > limits.max_records {
            return Err(RuntimeError::new(
                ErrorKind::Limit,
                "checkpoint record count exceeded",
            ));
        }
        let mut total = 0_u64;
        let mut previous = None;
        for record in &self.records {
            record.validate(limits)?;
            let key = (&record.owner, &record.key);
            if previous.is_some_and(|prior| prior >= key) {
                return Err(RuntimeError::invalid(
                    "checkpoint records are not canonical and unique",
                ));
            }
            previous = Some(key);
            total = total
                .checked_add(u64::try_from(record.bytes).map_err(RuntimeError::invalid)?)
                .ok_or_else(|| RuntimeError::invalid("checkpoint byte total overflow"))?;
            if total > limits.max_total_bytes {
                return Err(RuntimeError::new(
                    ErrorKind::Limit,
                    "checkpoint total byte limit exceeded",
                ));
            }
        }
        if self.fingerprint != self.expected_fingerprint()? {
            return Err(RuntimeError::invalid(
                "checkpoint head fingerprint mismatch",
            ));
        }
        Ok(())
    }
}

/// One immutable loaded head. Reading a newer save never changes this snapshot.
/// Owner records must be decoded into staged candidates before live adoption.
#[derive(Debug, Clone)]
pub struct SessionCheckpoint {
    root: PathBuf,
    head: SessionHead,
    limits: CheckpointLimits,
}

impl SessionCheckpoint {
    /// Exact generation/fingerprint required by the next complete save.
    #[must_use]
    pub fn token(&self) -> CheckpointToken {
        self.head.token()
    }

    /// Identity already verified against the requested world and composition.
    #[must_use]
    pub fn identity(&self) -> &CheckpointIdentity {
        &self.head.identity
    }

    /// Canonical lightweight descriptor list; no payloads are loaded here.
    #[must_use]
    pub fn records(&self) -> &[OwnerRecordDescriptor] {
        &self.head.records
    }

    /// Load a body only if its exact owner format is supported by the consumer.
    /// Missing records are explicit; corrupt records never become empty/default state.
    pub fn record(&self, owner: &str, key: &str, format: &str) -> RuntimeResult<Option<Vec<u8>>> {
        validate_identity(owner)?;
        validate_identity(key)?;
        validate_identity(format)?;
        let index = self
            .head
            .records
            .binary_search_by(|r| (r.owner.as_str(), r.key.as_str()).cmp(&(owner, key)));
        let Ok(index) = index else {
            return Ok(None);
        };
        let descriptor = self
            .head
            .records
            .get(index)
            .ok_or_else(|| RuntimeError::invalid("checkpoint record index is invalid"))?;
        if descriptor.format != format {
            return Err(RuntimeError::new(
                ErrorKind::Conflict,
                "owner record format is incompatible",
            ));
        }
        self.read_descriptor(descriptor, &CancellationToken::default())
            .map(Some)
    }

    /// Verify every body sequentially before owner-specific staged restoration.
    /// This does not decode owner schemas or publish any in-memory gameplay state.
    pub fn verify_all(&self, cancellation: &CancellationToken) -> RuntimeResult<()> {
        for descriptor in &self.head.records {
            self.read_descriptor(descriptor, cancellation)?;
        }
        Ok(())
    }

    fn read_descriptor(
        &self,
        descriptor: &OwnerRecordDescriptor,
        cancellation: &CancellationToken,
    ) -> RuntimeResult<Vec<u8>> {
        descriptor.validate(self.limits)?;
        let path = checked_existing_path(&self.root, &descriptor.path())?;
        let bytes = read_bytes_bounded(&path, descriptor.bytes, cancellation)?;
        if bytes.len() != descriptor.bytes
            || hash_serializable(&bytes).map_err(RuntimeError::invalid)? != descriptor.fingerprint
        {
            return Err(RuntimeError::invalid(
                "checkpoint record length or fingerprint mismatch",
            ));
        }
        Ok(bytes)
    }
}

/// Durable one-slot storage for a complete finite session. Independent owners
/// supply records at one quiescent application boundary; this store owns only IO.
#[derive(Debug, Clone)]
pub struct SessionCheckpointStore {
    root: PathBuf,
    identity: CheckpointIdentity,
    limits: CheckpointLimits,
}

impl SessionCheckpointStore {
    /// Prepare a package-bound store without creating or changing any file.
    pub fn new(
        root: impl AsRef<Path>,
        identity: CheckpointIdentity,
        limits: CheckpointLimits,
    ) -> RuntimeResult<Self> {
        identity.validate()?;
        limits.validate()?;
        Ok(Self {
            root: root.as_ref().to_path_buf(),
            identity,
            limits,
        })
    }

    /// Read and validate a lightweight head. An absent slot returns `None`;
    /// incompatible or corrupt saves are errors and never silently become a new run.
    pub fn load(&self) -> RuntimeResult<Option<SessionCheckpoint>> {
        if !self.root.exists() {
            return Ok(None);
        }
        let root = self.root.canonicalize().map_err(RuntimeError::io)?;
        let path = root.join(HEAD_FILE);
        // symlink_metadata also notices a dangling symlink instead of treating it as absent.
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(RuntimeError::io(error)),
            Ok(_) => {}
        }
        let path = checked_existing_path(&root, HEAD_FILE)?;
        let head: SessionHead = read_bounded(
            &path,
            self.limits.max_head_bytes,
            &CancellationToken::default(),
        )?;
        head.validate(self.limits)?;
        if head.identity != self.identity {
            return Err(RuntimeError::new(
                ErrorKind::Conflict,
                "session belongs to a different world or content version",
            ));
        }
        Ok(Some(SessionCheckpoint {
            root,
            head,
            limits: self.limits,
        }))
    }

    /// Preserve the current raw head for an explicitly confirmed replacement run.
    /// Immutable owner records stay in this directory, so the archive remains complete.
    /// A valid head stays active until the replacement commits against the returned
    /// token. An invalid/incompatible head is quarantined without decoding its bytes.
    /// The directory and its writer-lock inode never move, preserving stale-writer
    /// protection across concurrent processes and interrupted new-run creation.
    pub fn archive_current_head(&self) -> RuntimeResult<Option<CheckpointToken>> {
        fs::create_dir_all(&self.root).map_err(RuntimeError::io)?;
        let root = self.root.canonicalize().map_err(RuntimeError::io)?;
        let _writer = lock_directory(&root)?;
        match fs::symlink_metadata(root.join(HEAD_FILE)) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(RuntimeError::io(error)),
            Ok(_) => {}
        }
        let head = checked_existing_path(&root, HEAD_FILE)?;
        let token = self.load().ok().flatten().map(|snapshot| snapshot.token());
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(RuntimeError::invalid)?
            .as_nanos();
        let archive = root.join(format!("archived-session-{stamp}.ron"));
        // Head publication always replaces files atomically; the archive link keeps
        // the complete old inode without copying unbounded corrupt input into RAM.
        fs::hard_link(&head, &archive).map_err(RuntimeError::io)?;
        sync_directory(&root)?;
        if token.is_none() {
            fs::remove_file(&head).map_err(RuntimeError::io)?;
            sync_directory(&root)?;
        }
        Ok(token)
    }

    /// Atomically replace the complete slot from an iterator of owner records.
    /// `None` expects an absent slot; otherwise the exact previous token is required.
    /// Unmentioned records are removed from the new head, so owners must all export
    /// before publication. Existing immutable files remain available to old readers.
    /// An error/cancellation before head publication leaves the previous save intact.
    pub fn commit<I>(
        &self,
        expected: Option<CheckpointToken>,
        records: I,
        cancellation: &CancellationToken,
    ) -> RuntimeResult<SessionCheckpoint>
    where
        I: IntoIterator<Item = RuntimeResult<OwnerRecord>>,
    {
        cancellation.check()?;
        fs::create_dir_all(&self.root).map_err(RuntimeError::io)?;
        let root = self.root.canonicalize().map_err(RuntimeError::io)?;
        let _writer = lock_directory(&root)?;
        let previous = self.load()?;
        if previous.as_ref().map(SessionCheckpoint::token) != expected {
            return Err(RuntimeError::new(
                ErrorKind::Conflict,
                "session checkpoint writer is stale",
            ));
        }
        let generation = expected
            .map_or(Some(1), |token| token.generation.checked_add(1))
            .ok_or_else(|| RuntimeError::invalid("checkpoint generation exhausted"))?;
        let directory = ensure_relative_directory(&root, Path::new("session-records"))?;
        let mut descriptors = BTreeMap::new();
        let mut total = 0_u64;
        for record in records {
            cancellation.check()?;
            let record = record?;
            if record.bytes.len() > self.limits.max_record_bytes {
                return Err(RuntimeError::new(
                    ErrorKind::Limit,
                    "checkpoint record byte limit exceeded",
                ));
            }
            let descriptor = OwnerRecordDescriptor {
                owner: record.owner,
                key: record.key,
                format: record.format,
                fingerprint: hash_serializable(&record.bytes).map_err(RuntimeError::invalid)?,
                bytes: record.bytes.len(),
            };
            descriptor.validate(self.limits)?;
            if descriptors.len() >= self.limits.max_records {
                return Err(RuntimeError::new(
                    ErrorKind::Limit,
                    "checkpoint record count exceeded",
                ));
            }
            total = total
                .checked_add(u64::try_from(descriptor.bytes).map_err(RuntimeError::invalid)?)
                .ok_or_else(|| RuntimeError::invalid("checkpoint byte total overflow"))?;
            if total > self.limits.max_total_bytes {
                return Err(RuntimeError::new(
                    ErrorKind::Limit,
                    "checkpoint total byte limit exceeded",
                ));
            }
            let key = (descriptor.owner.clone(), descriptor.key.clone());
            if descriptors.contains_key(&key) {
                return Err(RuntimeError::invalid("duplicate checkpoint owner/key"));
            }
            write_immutable(&root, &descriptor.path(), &record.bytes)?;
            descriptors.insert(key, descriptor);
        }
        sync_directory(&directory)?;
        let mut head = SessionHead {
            schema_version: CHECKPOINT_SCHEMA,
            identity: self.identity.clone(),
            generation,
            records: descriptors.into_values().collect(),
            fingerprint: 0,
        };
        head.fingerprint = head.expected_fingerprint()?;
        head.validate(self.limits)?;
        cancellation.check()?;
        atomic_write_head(&root, HEAD_FILE, &head, self.limits.max_head_bytes)?;
        // No cancellable work after the durable publication point.
        Ok(SessionCheckpoint {
            root,
            head,
            limits: self.limits,
        })
    }
}
