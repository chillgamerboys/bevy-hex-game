//! Opaque owner state committed in the same durable head as terrain.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use hex_world_contracts::hash_serializable;
use serde::{Deserialize, Serialize};

use crate::{
    persistence::write_immutable,
    runtime::validate_identity,
    source::{
        checked_existing_path, ensure_relative_directory, read_bytes_bounded, sync_directory,
    },
    CancellationToken, ErrorKind, IoLimits, RuntimeError, RuntimeResult, WorldRuntime,
};

/// One owner-selected opaque checkpoint update, with compare-and-write protection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttachmentUpdate {
    /// Stable subsystem namespace, for example `gameplay`.
    pub owner: String,
    /// Stable partition/key inside that namespace.
    pub key: String,
    /// Required current content fingerprint; `None` requires an absent key.
    pub expected_fingerprint: Option<u64>,
    /// Replacement bytes, or `None` to explicitly delete the key.
    pub bytes: Option<Vec<u8>>,
}

/// Verified opaque owner payload; the runtime never interprets its schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointAttachment {
    /// Fingerprint of these exact bytes, suitable for the next update's expectation.
    pub fingerprint: u64,
    /// Independently loaded payload for the owning subsystem to decode and validate.
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AttachmentDescriptor {
    pub owner: String,
    pub key: String,
    pub fingerprint: u64,
    pub bytes: usize,
    pub path: String,
}

impl AttachmentDescriptor {
    pub(crate) fn validate(&self, maximum: usize) -> RuntimeResult<()> {
        validate_identity(&self.owner)?;
        validate_identity(&self.key)?;
        if self.bytes > maximum || self.path != format!("attachments/{:016x}.bin", self.fingerprint)
        {
            return Err(RuntimeError::invalid(
                "attachment size or content path is invalid",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AttachmentLocation {
    pub root: PathBuf,
    pub descriptor: AttachmentDescriptor,
    pub limits: IoLimits,
}

impl AttachmentLocation {
    pub(crate) fn load(&self) -> RuntimeResult<CheckpointAttachment> {
        self.descriptor.validate(self.limits.max_chunk_bytes)?;
        let path = checked_existing_path(&self.root, &self.descriptor.path)?;
        let bytes =
            read_bytes_bounded(&path, self.descriptor.bytes, &CancellationToken::default())?;
        if bytes.len() != self.descriptor.bytes
            || hash_serializable(&bytes).map_err(RuntimeError::invalid)?
                != self.descriptor.fingerprint
        {
            return Err(RuntimeError::invalid(
                "attachment bytes disagree with the committed fingerprint",
            ));
        }
        Ok(CheckpointAttachment {
            fingerprint: self.descriptor.fingerprint,
            bytes,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AttachmentBinding {
    pub transaction_id: String,
    pub request_fingerprint: u64,
}

pub(crate) type AttachmentLocations = BTreeMap<(String, String), AttachmentLocation>;

pub(crate) struct AttachmentPlan {
    pub locations: AttachmentLocations,
    pub bindings: BTreeMap<String, u64>,
}

impl WorldRuntime {
    /// Reads one bounded owner payload from the last committed/restored checkpoint.
    /// Ordinary saves retain unmentioned keys. The owner validates the returned format.
    pub fn attachment(
        &self,
        owner: &str,
        key: &str,
    ) -> RuntimeResult<Option<CheckpointAttachment>> {
        validate_identity(owner)?;
        validate_identity(key)?;
        self.attachments
            .get(&(owner.to_owned(), key.to_owned()))
            .map(AttachmentLocation::load)
            .transpose()
    }

    pub(crate) fn prepare_attachments(
        &self,
        root: &Path,
        limits: IoLimits,
        updates: &[AttachmentUpdate],
        transaction_id: Option<&str>,
        mut plan: AttachmentPlan,
    ) -> RuntimeResult<AttachmentPlan> {
        if updates.len() > self.config.max_attachment_updates {
            return Err(RuntimeError::new(
                ErrorKind::Limit,
                "checkpoint attachment update count exceeded",
            ));
        }
        let mut ordered = BTreeMap::new();
        let mut total = 0_usize;
        for update in updates {
            validate_identity(&update.owner)?;
            validate_identity(&update.key)?;
            if ordered
                .insert((update.owner.as_str(), update.key.as_str()), update)
                .is_some()
            {
                return Err(RuntimeError::invalid(
                    "duplicate attachment update identity",
                ));
            }
            let bytes = update.bytes.as_ref().map_or(0, Vec::len);
            total = total.saturating_add(bytes);
            if bytes > limits.max_chunk_bytes || total > limits.max_transaction_bytes {
                return Err(RuntimeError::new(
                    ErrorKind::Limit,
                    "checkpoint attachment byte budget exceeded",
                ));
            }
        }
        let request_fingerprint = hash_serializable(&ordered.values().collect::<Vec<_>>())
            .map_err(RuntimeError::invalid)?;
        let mut duplicate = false;
        if let Some(id) = transaction_id.filter(|_| !updates.is_empty()) {
            if let Some(previous) = plan.bindings.get(id) {
                if *previous != request_fingerprint {
                    return Err(RuntimeError::new(
                        ErrorKind::Conflict,
                        "terrain transaction ID carries different attachment updates",
                    ));
                }
                duplicate = true;
            } else if self.transactions.contains_key(id) {
                return Err(RuntimeError::new(
                    ErrorKind::Conflict,
                    "terrain transaction was already committed without these attachments",
                ));
            } else {
                plan.bindings.insert(id.to_owned(), request_fingerprint);
            }
        }
        let mut bodies = BTreeMap::new();
        if !duplicate {
            for update in ordered.values() {
                let identity = (update.owner.clone(), update.key.clone());
                let current = plan
                    .locations
                    .get(&identity)
                    .map(|location| location.descriptor.fingerprint);
                if current != update.expected_fingerprint {
                    return Err(RuntimeError::new(
                        ErrorKind::Conflict,
                        "attachment compare-and-write expectation is stale",
                    ));
                }
                if let Some(bytes) = &update.bytes {
                    let fingerprint = hash_serializable(bytes).map_err(RuntimeError::invalid)?;
                    let descriptor = AttachmentDescriptor {
                        owner: update.owner.clone(),
                        key: update.key.clone(),
                        fingerprint,
                        bytes: bytes.len(),
                        path: format!("attachments/{fingerprint:016x}.bin"),
                    };
                    bodies.insert(identity.clone(), bytes.as_slice());
                    plan.locations.insert(
                        identity,
                        AttachmentLocation {
                            root: root.to_path_buf(),
                            descriptor,
                            limits,
                        },
                    );
                } else {
                    plan.locations.remove(&identity);
                }
            }
        }
        // Every CAS and request check completes before writing even orphan payload files.
        if !plan.locations.is_empty() {
            let directory = ensure_relative_directory(root, Path::new("attachments"))?;
            for (identity, location) in &mut plan.locations {
                location.descriptor.validate(limits.max_chunk_bytes)?;
                if let Some(bytes) = bodies.get(identity) {
                    write_immutable(root, &location.descriptor.path, bytes)?;
                } else if location.root != root {
                    let payload = location.load()?;
                    write_immutable(root, &location.descriptor.path, &payload.bytes)?;
                } else {
                    let _safe = checked_existing_path(root, &location.descriptor.path)?;
                }
                location.root = root.to_path_buf();
                location.limits = limits;
            }
            sync_directory(&directory)?;
        }
        Ok(plan)
    }
}
