//! Paged transaction bodies; only bounded recent and unsaved bodies stay in RAM.

use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use hex_world_contracts::{hash_serializable, ChunkId, WorldManifest};
use serde::{Deserialize, Serialize};

use crate::{
    edits::AppliedTransaction,
    runtime::validate_identity,
    source::{checked_existing_path, encode_bounded, read_bounded},
    CancellationToken, ErrorKind, IoLimits, RuntimeError, RuntimeResult, WorldRuntime,
};

/// Fine payload residency for historical transaction diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryCounts {
    /// Lightweight indexed identities, including dormant journal files.
    pub indexed_transactions: usize,
    /// Durable bodies retained in the recent-commit cache.
    pub cached_transactions: usize,
    /// Bodies awaiting a durable checkpoint.
    pub unsaved_transactions: usize,
    /// Serialized byte size of all retained bodies, excluding lightweight metadata.
    pub resident_body_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct JournalDescriptor {
    pub id: String,
    pub fingerprint: u64,
    pub request_fingerprint: u64,
    pub delta_fingerprint: u64,
    pub revisions: Vec<(ChunkId, u64)>,
    pub bytes: usize,
    pub path: String,
}

impl JournalDescriptor {
    pub(crate) fn prepare(record: &AppliedTransaction, maximum: usize) -> RuntimeResult<Self> {
        let bytes = encode_bounded(record, maximum)?.len();
        let fingerprint = hash_serializable(record).map_err(RuntimeError::invalid)?;
        Ok(Self {
            id: record.change.transaction_id.clone(),
            fingerprint,
            request_fingerprint: record.request_fingerprint,
            delta_fingerprint: record.delta.fingerprint,
            revisions: record
                .change
                .revisions
                .iter()
                .map(|(chunk, revision)| (*chunk, *revision))
                .collect(),
            bytes,
            path: format!("transactions/{fingerprint:016x}.ron"),
        })
    }

    pub(crate) fn validate(&self, maximum: usize) -> RuntimeResult<()> {
        validate_identity(&self.id)?;
        if self.bytes == 0
            || self.bytes > maximum
            || self.revisions.is_empty()
            || self.revisions.iter().any(|(_, revision)| *revision == 0)
            || self.revisions.windows(2).any(|pair| {
                pair.first()
                    .zip(pair.get(1))
                    .is_some_and(|(a, b)| a.0 >= b.0)
            })
        {
            return Err(RuntimeError::invalid(
                "journal descriptor has invalid size or revisions",
            ));
        }
        if self.path != format!("transactions/{:016x}.ron", self.fingerprint) {
            return Err(RuntimeError::invalid(
                "journal path does not match its content address",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub(crate) struct HistoryEntry {
    pub descriptor: JournalDescriptor,
    pub body: Option<Arc<AppliedTransaction>>,
    pub disk: Option<PathBuf>,
    pub limits: IoLimits,
}

impl HistoryEntry {
    pub(crate) fn load(&self, manifest: &WorldManifest) -> RuntimeResult<Arc<AppliedTransaction>> {
        if let Some(body) = &self.body {
            return Ok(Arc::clone(body));
        }
        self.descriptor
            .validate(self.limits.max_transaction_bytes)?;
        let root = self
            .disk
            .as_ref()
            .ok_or_else(|| RuntimeError::invalid("journal body is neither resident nor durable"))?;
        let path = checked_existing_path(root, &self.descriptor.path)?;
        let record: AppliedTransaction =
            read_bounded(&path, self.descriptor.bytes, &CancellationToken::default())?;
        record.delta.validate()?;
        record.change.validate().map_err(RuntimeError::invalid)?;
        let revisions = record
            .delta
            .chunks
            .iter()
            .map(|chunk| (chunk.coordinate, chunk.revision))
            .collect::<BTreeMap<_, _>>();
        let columns = record.delta.changed_columns()?;
        if hash_serializable(&record).map_err(RuntimeError::invalid)? != self.descriptor.fingerprint
            || record.request_fingerprint != self.descriptor.request_fingerprint
            || record.delta.request_fingerprint != self.descriptor.request_fingerprint
            || record.delta.fingerprint != self.descriptor.delta_fingerprint
            || record.change.transaction_id != self.descriptor.id
            || record.delta.transaction_id != self.descriptor.id
            || record.delta.world_id != manifest.world_id
            || record.delta.manifest_fingerprint != manifest.fingerprint
            || record
                .change
                .revisions
                .iter()
                .map(|(chunk, revision)| (*chunk, *revision))
                .collect::<Vec<_>>()
                != self.descriptor.revisions
            || record.change.revisions != revisions
            || record.change.changed_columns != columns
        {
            return Err(RuntimeError::invalid(
                "paged transaction body disagrees with its trusted index",
            ));
        }
        Ok(Arc::new(record))
    }
}

impl WorldRuntime {
    /// Current bounded transaction body residency, separate from history metadata.
    #[must_use]
    pub fn history_counts(&self) -> HistoryCounts {
        let mut cached = 0;
        let mut bytes = 0_usize;
        for id in &self.history_order {
            if let Some(entry) = self
                .transactions
                .get(id)
                .filter(|entry| entry.body.is_some())
            {
                bytes = bytes.saturating_add(entry.descriptor.bytes);
                if entry.disk.is_some() {
                    cached += 1;
                }
            }
        }
        HistoryCounts {
            indexed_transactions: self.transactions.len(),
            cached_transactions: cached,
            unsaved_transactions: self.unsaved_transactions.len(),
            resident_body_bytes: bytes,
        }
    }

    pub(crate) fn check_history_budget(&self, descriptor: &JournalDescriptor) -> RuntimeResult<()> {
        if self.transactions.contains_key(&descriptor.id) {
            return Ok(());
        }
        if self.unsaved_transactions.len() >= self.config.max_unsaved_transactions
            || self
                .unsaved_transaction_bytes
                .saturating_add(descriptor.bytes)
                > self.config.max_unsaved_transaction_bytes
        {
            return Err(RuntimeError::new(
                ErrorKind::Limit,
                "unsaved transaction backlog exceeds budget; checkpoint before further edits",
            ));
        }
        Ok(())
    }

    pub(crate) fn cache_transaction(
        &mut self,
        record: AppliedTransaction,
        descriptor: JournalDescriptor,
    ) {
        let id = descriptor.id.clone();
        let disk = self
            .transactions
            .get(&id)
            .and_then(|entry| entry.disk.clone());
        if !self.transactions.contains_key(&id) {
            self.unsaved_transactions.insert(id.clone());
            self.unsaved_transaction_bytes = self
                .unsaved_transaction_bytes
                .saturating_add(descriptor.bytes);
        }
        self.transactions.insert(
            id.clone(),
            HistoryEntry {
                descriptor,
                body: Some(Arc::new(record)),
                disk,
                limits: IoLimits {
                    max_transaction_bytes: self.config.max_transaction_bytes,
                    ..IoLimits::default()
                },
            },
        );
        self.history_order.retain(|prior| prior != &id);
        self.history_order.push_back(id);
        self.trim_history_cache();
    }

    pub(crate) fn trim_history_cache(&mut self) {
        let mut durable_count = 0_usize;
        let mut durable_bytes = 0_usize;
        for id in self.history_order.iter().rev() {
            if let Some(entry) = self.transactions.get_mut(id) {
                if entry.disk.is_some() && entry.body.is_some() {
                    if durable_count >= self.config.max_cached_transactions
                        || durable_bytes.saturating_add(entry.descriptor.bytes)
                            > self.config.max_cached_transaction_bytes
                    {
                        entry.body = None;
                    } else {
                        durable_count += 1;
                        durable_bytes += entry.descriptor.bytes;
                    }
                }
            }
        }
        self.history_order.retain(|id| {
            self.transactions
                .get(id)
                .is_some_and(|entry| entry.body.is_some())
        });
    }
}
