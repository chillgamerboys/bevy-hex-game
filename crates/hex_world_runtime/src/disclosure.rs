//! Sequenced replication of already-declassified principal knowledge only.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use hex_world_contracts::{hash_serializable, ChunkId, SCHEMA_VERSION};
use serde::{Deserialize, Serialize};

use crate::{
    knowledge::{ensure_ordered, CheckpointProgress, KnowledgeCursor, KnowledgeHead},
    runtime::validate_identity,
    source::encode_bounded,
    ErrorKind, KnowledgePartition, KnowledgeStore, RuntimeError, RuntimeResult,
};

/// Host-selected principal and interests. Deliberately not deserializable as a client claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizedInterest {
    principal: String,
    chunks: BTreeSet<ChunkId>,
}

impl AuthorizedInterest {
    /// Creates a scope after the host has authenticated/authorized its principal.
    pub fn new(principal: impl Into<String>, chunks: BTreeSet<ChunkId>) -> RuntimeResult<Self> {
        let principal = principal.into();
        validate_identity(&principal)?;
        Ok(Self { principal, chunks })
    }
    /// Authorized principal; never selected from an incoming packet.
    #[must_use]
    pub fn principal(&self) -> &str {
        &self.principal
    }
    /// Exact authorized interest chunks.
    #[must_use]
    pub fn chunks(&self) -> &BTreeSet<ChunkId> {
        &self.chunks
    }
    fn fingerprint(&self) -> RuntimeResult<u64> {
        hash_serializable(&(&self.principal, &self.chunks)).map_err(RuntimeError::invalid)
    }
}

/// Explicit bounds on a connection's retained and emitted knowledge work.
#[derive(Debug, Clone, Copy)]
pub struct DisclosureConfig {
    /// Maximum active authorized chunks, independent of total discovered world size.
    pub max_interest_chunks: usize,
    /// Maximum partition bodies in one batch or checkpoint page.
    pub max_partitions_per_batch: usize,
    /// Maximum serialized bytes in one emitted batch or checkpoint page.
    pub max_batch_bytes: usize,
    /// Maximum retained sequence batches available for replay.
    pub max_retained_batches: usize,
    /// Maximum combined serialized bytes retained for replay.
    pub max_retained_bytes: usize,
}
impl Default for DisclosureConfig {
    fn default() -> Self {
        Self {
            max_interest_chunks: 256,
            max_partitions_per_batch: 32,
            max_batch_bytes: 8 * 1024 * 1024,
            max_retained_batches: 32,
            max_retained_bytes: 16 * 1024 * 1024,
        }
    }
}

/// One integrity-protected private knowledge sequence; contains no raw world columns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SequencedKnowledgeBatch {
    /// Supported fresh protocol version.
    pub schema_version: u32,
    /// Owning world identity.
    pub world_id: String,
    /// Exact source world package.
    pub manifest_fingerprint: u64,
    /// Host-created stream identity.
    pub stream_id: String,
    /// Principal whose already-disclosed memories this batch carries.
    pub principal: String,
    /// Monotonic stream sequence, independent of chunk and encounter clocks.
    pub sequence: u64,
    /// Complete replacements only for selected principal/interest chunks.
    pub partitions: Vec<KnowledgePartition>,
    /// Canonical content hash with this field excluded.
    pub fingerprint: u64,
}

impl SequencedKnowledgeBatch {
    /// Verifies canonical shape, principal agreement and integrity.
    pub fn validate(&self) -> RuntimeResult<()> {
        validate_identity(&self.stream_id)?;
        validate_identity(&self.principal)?;
        if self.schema_version != SCHEMA_VERSION
            || self.sequence == 0
            || self.partitions.is_empty()
            || self.fingerprint != self.expected_fingerprint()?
        {
            return Err(RuntimeError::invalid(
                "knowledge sequence schema, counter or fingerprint mismatch",
            ));
        }
        validate_partitions(&self.principal, &self.partitions)?;
        Ok(())
    }
    fn expected_fingerprint(&self) -> RuntimeResult<u64> {
        let mut value = self.clone();
        value.fingerprint = 0;
        hash_serializable(&value).map_err(RuntimeError::invalid)
    }
    fn seal(&mut self) -> RuntimeResult<()> {
        self.fingerprint = self.expected_fingerprint()?;
        self.validate()
    }
}

/// Bounded reconnect result. Missing replay history requires scoped checkpoint pages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KnowledgeReplay {
    /// The receiver already has the latest sequence.
    UpToDate,
    /// Contiguous retained batches, all still inside the current authorized interests.
    Replay(Vec<SequencedKnowledgeBatch>),
    /// Requested history is unavailable; request a new bounded checkpoint cursor.
    ResyncRequired,
}

/// Opaque bounded-checkpoint continuation; not a grant of disclosure authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeCheckpointCursor {
    /// Fingerprint of only the authorized principal/interest descriptor set.
    pub snapshot_fingerprint: u64,
    /// Stream sequence at checkpoint creation.
    pub watermark: u64,
    /// Last completed chunk; the next page strictly follows it.
    pub after: ChunkId,
}

/// One bounded page of private interest-scoped memory for reconnect recovery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeCheckpointPage {
    /// Supported fresh protocol version.
    pub schema_version: u32,
    /// Owning world.
    pub world_id: String,
    /// Exact compiled source fingerprint.
    pub manifest_fingerprint: u64,
    /// Host-created stream identity.
    pub stream_id: String,
    /// Authorized principal.
    pub principal: String,
    /// Exact authorized interest scope identity.
    pub scope_fingerprint: u64,
    /// Stable identity of only this scope's knowledge descriptor snapshot.
    pub snapshot_fingerprint: u64,
    /// Stream sequence that the final page establishes durably.
    pub watermark: u64,
    /// Previous page's last chunk, or none for the first page.
    pub after: Option<ChunkId>,
    /// Bounded canonical partition bodies.
    pub partitions: Vec<KnowledgePartition>,
    /// Continuation when another page is required.
    pub next: Option<KnowledgeCheckpointCursor>,
    /// Canonical integrity hash with this field excluded.
    pub fingerprint: u64,
}

impl KnowledgeCheckpointPage {
    /// Checks integrity, canonical rows, and exact continuation shape.
    pub fn validate(&self) -> RuntimeResult<()> {
        validate_identity(&self.stream_id)?;
        validate_identity(&self.principal)?;
        if self.schema_version != SCHEMA_VERSION
            || self.fingerprint != self.expected_fingerprint()?
        {
            return Err(RuntimeError::invalid(
                "checkpoint page schema or fingerprint mismatch",
            ));
        }
        validate_partitions(&self.principal, &self.partitions)?;
        if self.after.is_some_and(|after| {
            self.partitions
                .first()
                .is_some_and(|partition| partition.coordinate <= after)
        }) {
            return Err(RuntimeError::invalid(
                "checkpoint page does not follow its cursor",
            ));
        }
        if let Some(next) = &self.next {
            if next.snapshot_fingerprint != self.snapshot_fingerprint
                || next.watermark != self.watermark
                || self.partitions.last().map(|partition| partition.coordinate) != Some(next.after)
            {
                return Err(RuntimeError::invalid(
                    "checkpoint continuation disagrees with page",
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
    fn seal(&mut self) -> RuntimeResult<()> {
        self.fingerprint = self.expected_fingerprint()?;
        self.validate()
    }
}

/// Exact private snapshot covered by a completed checkpoint acknowledgment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeCheckpointIdentity {
    /// Principal and interest scope of the acknowledged snapshot.
    pub scope_fingerprint: u64,
    /// Exact private snapshot fingerprint, independent of unrelated party updates.
    pub snapshot_fingerprint: u64,
}

/// Durable acknowledgment, emitted only after private partitions and cursor commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeAck {
    /// Acknowledged host-created stream.
    pub stream_id: String,
    /// Durable sequence, or checkpoint watermark once every page has committed.
    pub sequence: u64,
    /// False while a multi-page checkpoint remains incomplete.
    pub checkpoint_complete: bool,
    /// Present only for checkpoint pages; ordinary incremental ACKs cannot establish a new scope.
    pub checkpoint: Option<KnowledgeCheckpointIdentity>,
}

/// Connection-local bounded replay outbox. It reads only stored declassified knowledge.
pub struct DisclosureStream {
    stream_id: String,
    scope: AuthorizedInterest,
    config: DisclosureConfig,
    sequence: u64,
    acknowledged: u64,
    checkpoint_required: bool,
    pending_checkpoint: Option<(u64, u64, u64)>,
    retained: VecDeque<(SequencedKnowledgeBatch, usize)>,
    retained_bytes: usize,
}

impl DisclosureStream {
    /// Creates a fresh host stream. A restarted transport must use a new identity or `resume`.
    pub fn new(
        stream_id: impl Into<String>,
        scope: AuthorizedInterest,
        config: DisclosureConfig,
    ) -> RuntimeResult<Self> {
        Self::resume(stream_id, scope, 0, config)
    }

    /// Restarts after a host-verified durable sequence; replay initially requires a checkpoint.
    pub fn resume(
        stream_id: impl Into<String>,
        scope: AuthorizedInterest,
        sequence: u64,
        config: DisclosureConfig,
    ) -> RuntimeResult<Self> {
        let stream_id = stream_id.into();
        validate_identity(&stream_id)?;
        if config.max_interest_chunks == 0
            || config.max_partitions_per_batch == 0
            || config.max_batch_bytes == 0
            || config.max_retained_batches == 0
            || config.max_retained_bytes == 0
            || scope.chunks.len() > config.max_interest_chunks
        {
            return Err(RuntimeError::new(
                ErrorKind::Limit,
                "disclosure bounds or scope are invalid",
            ));
        }
        Ok(Self {
            stream_id,
            scope,
            config,
            sequence,
            acknowledged: 0,
            checkpoint_required: true,
            pending_checkpoint: None,
            retained: VecDeque::new(),
            retained_bytes: 0,
        })
    }

    /// Replaces host-authorized interests, preserving the principal and sequence clock.
    /// Old-scope replay is discarded so reconnect cannot resend withdrawn interests.
    pub fn set_interests(&mut self, chunks: BTreeSet<ChunkId>) -> RuntimeResult<()> {
        if chunks.len() > self.config.max_interest_chunks {
            return Err(RuntimeError::new(
                ErrorKind::Limit,
                "disclosure interest budget exceeded",
            ));
        }
        if chunks != self.scope.chunks {
            self.scope.chunks = chunks;
            self.checkpoint_required = true;
            self.pending_checkpoint = None;
            self.retained.clear();
            self.retained_bytes = 0;
        }
        Ok(())
    }

    /// Emits only changed chunks inside this principal's authorized interests.
    /// `None` means no disclosed affected partition exists; it does not advance sequence.
    pub fn publish(
        &mut self,
        store: &KnowledgeStore,
        changed_chunks: &BTreeSet<ChunkId>,
    ) -> RuntimeResult<Option<SequencedKnowledgeBatch>> {
        let selected = changed_chunks
            .intersection(&self.scope.chunks)
            .copied()
            .collect::<Vec<_>>();
        if selected.len() > self.config.max_partitions_per_batch {
            return Err(RuntimeError::new(
                ErrorKind::Limit,
                "disclosure batch exceeds partition budget",
            ));
        }
        let mut partitions = Vec::new();
        let mut payload_bytes = 0_usize;
        for coordinate in selected {
            if let Some(partition) = store.read(&self.scope.principal, coordinate)? {
                payload_bytes = payload_bytes
                    .saturating_add(encode_bounded(&partition, self.config.max_batch_bytes)?.len());
                if payload_bytes > self.config.max_batch_bytes {
                    return Err(RuntimeError::new(
                        ErrorKind::Limit,
                        "disclosure batch exceeds byte budget",
                    ));
                }
                partitions.push(partition);
            }
        }
        if partitions.is_empty() {
            return Ok(None);
        }
        let sequence = self
            .sequence
            .checked_add(1)
            .ok_or_else(|| RuntimeError::new(ErrorKind::Limit, "knowledge sequence exhausted"))?;
        let mut batch = SequencedKnowledgeBatch {
            schema_version: SCHEMA_VERSION,
            world_id: store.manifest.world_id.clone(),
            manifest_fingerprint: store.manifest.fingerprint,
            stream_id: self.stream_id.clone(),
            principal: self.scope.principal.clone(),
            sequence,
            partitions,
            fingerprint: 0,
        };
        batch.seal()?;
        let bytes = encode_bounded(&batch, self.config.max_batch_bytes)?.len();
        if bytes > self.config.max_retained_bytes {
            return Err(RuntimeError::new(
                ErrorKind::Limit,
                "one batch exceeds replay byte budget",
            ));
        }
        while self.retained.len() >= self.config.max_retained_batches
            || self.retained_bytes.saturating_add(bytes) > self.config.max_retained_bytes
        {
            if let Some((_, retired_bytes)) = self.retained.pop_front() {
                self.retained_bytes = self.retained_bytes.saturating_sub(retired_bytes);
            } else {
                break;
            }
        }
        self.sequence = sequence;
        self.retained_bytes += bytes;
        self.retained.push_back((batch.clone(), bytes));
        Ok(Some(batch))
    }

    /// Records a receiver's durable ACK; incomplete checkpoint pages cannot acknowledge a sequence.
    pub fn acknowledge(&mut self, ack: &KnowledgeAck) -> RuntimeResult<()> {
        if ack.stream_id != self.stream_id
            || ack.sequence > self.sequence
            || !ack.checkpoint_complete
        {
            return Err(RuntimeError::new(
                ErrorKind::Conflict,
                "ACK is outside this stream's committed sequence",
            ));
        }
        if let Some(checkpoint) = &ack.checkpoint {
            if self.pending_checkpoint
                != Some((
                    checkpoint.scope_fingerprint,
                    checkpoint.snapshot_fingerprint,
                    ack.sequence,
                ))
                || checkpoint.scope_fingerprint != self.scope.fingerprint()?
            {
                return Err(RuntimeError::new(
                    ErrorKind::Conflict,
                    "checkpoint ACK does not match the current issued scope and snapshot",
                ));
            }
            self.checkpoint_required = false;
        }
        self.acknowledged = self.acknowledged.max(ack.sequence);
        Ok(())
    }

    /// Returns contiguous retained replay or requests bounded scoped checkpoint pages.
    #[must_use]
    pub fn reconnect(&self, after_sequence: u64) -> KnowledgeReplay {
        if self.checkpoint_required {
            return KnowledgeReplay::ResyncRequired;
        }
        if after_sequence == self.sequence {
            return KnowledgeReplay::UpToDate;
        }
        if after_sequence > self.sequence
            || self
                .retained
                .front()
                .is_none_or(|(batch, _)| batch.sequence > after_sequence.saturating_add(1))
        {
            return KnowledgeReplay::ResyncRequired;
        }
        KnowledgeReplay::Replay(
            self.retained
                .iter()
                .filter(|(batch, _)| batch.sequence > after_sequence)
                .map(|(batch, _)| batch.clone())
                .collect(),
        )
    }

    /// Produces one bounded reconnect page; unrelated principal updates do not invalidate it.
    /// A changed scoped snapshot requires restarting from a fresh first page.
    pub fn checkpoint_page(
        &mut self,
        store: &KnowledgeStore,
        cursor: Option<&KnowledgeCheckpointCursor>,
    ) -> RuntimeResult<KnowledgeCheckpointPage> {
        let descriptors = self
            .scope
            .chunks
            .iter()
            .filter_map(|coordinate| {
                KnowledgeStore::head_descriptor(&store.head, &self.scope.principal, *coordinate)
            })
            .collect::<Vec<_>>();
        let snapshot_fingerprint =
            hash_serializable(&(&self.scope.principal, &self.scope.chunks, &descriptors))
                .map_err(RuntimeError::invalid)?;
        if cursor.is_some_and(|cursor| {
            cursor.snapshot_fingerprint != snapshot_fingerprint || cursor.watermark > self.sequence
        }) {
            return Err(RuntimeError::new(
                ErrorKind::Conflict,
                "scoped checkpoint snapshot changed; restart paging",
            ));
        }
        let after = cursor.map(|cursor| cursor.after);
        let watermark = cursor.map_or(self.sequence, |cursor| cursor.watermark);
        let remaining = descriptors
            .into_iter()
            .filter(|descriptor| after.is_none_or(|after| descriptor.coordinate > after))
            .collect::<Vec<_>>();
        let mut partitions = Vec::new();
        let mut payload_bytes = 0_usize;
        for descriptor in remaining.iter().take(self.config.max_partitions_per_batch) {
            let partition = store.read_descriptor(descriptor)?;
            let bytes = encode_bounded(&partition, self.config.max_batch_bytes)?.len();
            if payload_bytes.saturating_add(bytes)
                > self.config.max_batch_bytes.saturating_sub(1024)
            {
                if partitions.is_empty() {
                    return Err(RuntimeError::new(
                        ErrorKind::Limit,
                        "one knowledge partition exceeds checkpoint byte budget",
                    ));
                }
                break;
            }
            payload_bytes += bytes;
            partitions.push(partition);
        }
        let next = if partitions.len() < remaining.len() {
            Some(KnowledgeCheckpointCursor {
                snapshot_fingerprint,
                watermark,
                after: partitions
                    .last()
                    .ok_or_else(|| {
                        RuntimeError::new(
                            ErrorKind::Limit,
                            "checkpoint page cannot fit any partition",
                        )
                    })?
                    .coordinate,
            })
        } else {
            None
        };
        let mut page = KnowledgeCheckpointPage {
            schema_version: SCHEMA_VERSION,
            world_id: store.manifest.world_id.clone(),
            manifest_fingerprint: store.manifest.fingerprint,
            stream_id: self.stream_id.clone(),
            principal: self.scope.principal.clone(),
            scope_fingerprint: self.scope.fingerprint()?,
            snapshot_fingerprint,
            watermark,
            after,
            partitions,
            next,
            fingerprint: 0,
        };
        page.seal()?;
        let _bounded = encode_bounded(&page, self.config.max_batch_bytes)?;
        self.pending_checkpoint = Some((
            page.scope_fingerprint,
            page.snapshot_fingerprint,
            page.watermark,
        ));
        Ok(page)
    }

    /// Bounded replay payload cardinality and encoded byte count.
    #[must_use]
    pub fn retained_counts(&self) -> (usize, usize) {
        (self.retained.len(), self.retained_bytes)
    }
}

impl KnowledgeStore {
    /// Last durable sequence for a specific principal and stream, without loading memories.
    pub fn sequence(&self, principal: &str, stream_id: &str) -> RuntimeResult<u64> {
        validate_identity(principal)?;
        validate_identity(stream_id)?;
        Ok(cursor_for(&self.head, principal, stream_id).map_or(0, |cursor| cursor.sequence))
    }

    /// Applies one authorized sequence and persists partitions plus cursor before ACK.
    /// Gaps, scope violations and mismatched duplicates leave all authority unchanged.
    pub fn apply_sequence_durable(
        &mut self,
        scope: &AuthorizedInterest,
        batch: &SequencedKnowledgeBatch,
    ) -> RuntimeResult<KnowledgeAck> {
        self.check_envelope(
            scope,
            &batch.world_id,
            batch.manifest_fingerprint,
            &batch.principal,
            &batch.partitions,
        )?;
        batch.validate()?;
        let (_writer, mut head) = self.locked_head()?;
        let id = sequence_id(&batch.stream_id, batch.sequence)?;
        if Self::duplicate(&head, &batch.principal, &id, batch.fingerprint)?.is_some() {
            self.install_head(head);
            return Ok(KnowledgeAck {
                stream_id: batch.stream_id.clone(),
                sequence: batch.sequence,
                checkpoint_complete: true,
                checkpoint: None,
            });
        }
        let mut cursor = cursor_for(&head, &batch.principal, &batch.stream_id)
            .cloned()
            .unwrap_or_else(|| KnowledgeCursor {
                principal: batch.principal.clone(),
                stream_id: batch.stream_id.clone(),
                sequence: 0,
                checkpoint: None,
            });
        if cursor.checkpoint.is_some() || cursor.sequence.checked_add(1) != Some(batch.sequence) {
            return Err(RuntimeError::new(
                ErrorKind::Conflict,
                "knowledge sequence gap or incomplete checkpoint",
            ));
        }
        let replacements = forward_partitions(&head, &batch.principal, &batch.partitions)?;
        cursor.sequence = batch.sequence;
        replace_cursor(&mut head, cursor);
        let _receipt =
            self.commit_prepared(head, &batch.principal, &id, batch.fingerprint, replacements)?;
        Ok(KnowledgeAck {
            stream_id: batch.stream_id.clone(),
            sequence: batch.sequence,
            checkpoint_complete: true,
            checkpoint: None,
        })
    }

    /// Durably applies one scoped reconnect page; only the final page advances the stream ACK.
    /// Page progress survives restart, and incoming ordinary sequences wait for completion.
    pub fn apply_checkpoint_page_durable(
        &mut self,
        scope: &AuthorizedInterest,
        page: &KnowledgeCheckpointPage,
    ) -> RuntimeResult<KnowledgeAck> {
        self.check_envelope(
            scope,
            &page.world_id,
            page.manifest_fingerprint,
            &page.principal,
            &page.partitions,
        )?;
        page.validate()?;
        if page.scope_fingerprint != scope.fingerprint()? {
            return Err(RuntimeError::new(
                ErrorKind::Conflict,
                "checkpoint was prepared for another authorization scope",
            ));
        }
        let (_writer, mut head) = self.locked_head()?;
        let id = format!(
            "checkpoint-{:016x}",
            hash_serializable(&(
                &page.stream_id,
                page.snapshot_fingerprint,
                page.watermark,
                page.after
            ))
            .map_err(RuntimeError::invalid)?
        );
        if Self::duplicate(&head, &page.principal, &id, page.fingerprint)?.is_some() {
            self.install_head(head);
            return Ok(KnowledgeAck {
                stream_id: page.stream_id.clone(),
                sequence: page.watermark,
                checkpoint_complete: page.next.is_none(),
                checkpoint: Some(KnowledgeCheckpointIdentity {
                    scope_fingerprint: page.scope_fingerprint,
                    snapshot_fingerprint: page.snapshot_fingerprint,
                }),
            });
        }
        let mut cursor = cursor_for(&head, &page.principal, &page.stream_id)
            .cloned()
            .unwrap_or_else(|| KnowledgeCursor {
                principal: page.principal.clone(),
                stream_id: page.stream_id.clone(),
                sequence: 0,
                checkpoint: None,
            });
        if page.watermark < cursor.sequence {
            return Err(RuntimeError::new(
                ErrorKind::Conflict,
                "checkpoint would roll back stream progress",
            ));
        }
        if page.after.is_some()
            && cursor.checkpoint.as_ref().is_none_or(|progress| {
                progress.snapshot_fingerprint != page.snapshot_fingerprint
                    || progress.watermark != page.watermark
                    || progress.after != page.after
            })
        {
            return Err(RuntimeError::new(
                ErrorKind::Conflict,
                "checkpoint page is missing its committed predecessor",
            ));
        }
        let replacements = forward_partitions(&head, &page.principal, &page.partitions)?;
        if let Some(next) = &page.next {
            cursor.checkpoint = Some(CheckpointProgress {
                snapshot_fingerprint: page.snapshot_fingerprint,
                watermark: page.watermark,
                after: Some(next.after),
            });
        } else {
            cursor.sequence = page.watermark;
            cursor.checkpoint = None;
        }
        replace_cursor(&mut head, cursor);
        let _receipt =
            self.commit_prepared(head, &page.principal, &id, page.fingerprint, replacements)?;
        Ok(KnowledgeAck {
            stream_id: page.stream_id.clone(),
            sequence: page.watermark,
            checkpoint_complete: page.next.is_none(),
            checkpoint: Some(KnowledgeCheckpointIdentity {
                scope_fingerprint: page.scope_fingerprint,
                snapshot_fingerprint: page.snapshot_fingerprint,
            }),
        })
    }

    fn check_envelope(
        &self,
        scope: &AuthorizedInterest,
        world_id: &str,
        manifest_fingerprint: u64,
        principal: &str,
        partitions: &[KnowledgePartition],
    ) -> RuntimeResult<()> {
        if principal != scope.principal
            || world_id != self.manifest.world_id
            || manifest_fingerprint != self.manifest.fingerprint
        {
            return Err(RuntimeError::new(
                ErrorKind::Conflict,
                "knowledge packet is outside the authorized principal/world",
            ));
        }
        if partitions.len() > self.config.max_partitions_per_operation {
            return Err(RuntimeError::new(
                ErrorKind::Limit,
                "knowledge packet exceeds partition budget",
            ));
        }
        for partition in partitions {
            if partition.principal != scope.principal
                || !scope.chunks.contains(&partition.coordinate)
            {
                return Err(RuntimeError::new(
                    ErrorKind::Conflict,
                    "knowledge packet includes an unauthorized interest or principal",
                ));
            }
        }
        let _bounded = encode_bounded(&partitions, self.limits.max_transaction_bytes)?;
        Ok(())
    }
}

fn validate_partitions(principal: &str, partitions: &[KnowledgePartition]) -> RuntimeResult<()> {
    ensure_ordered(
        partitions.iter().map(|partition| partition.coordinate),
        "disclosure partitions",
    )?;
    for partition in partitions {
        partition.validate()?;
        if partition.principal != principal {
            return Err(RuntimeError::new(
                ErrorKind::Conflict,
                "batch includes another principal's private memory",
            ));
        }
    }
    Ok(())
}

fn sequence_id(stream_id: &str, sequence: u64) -> RuntimeResult<String> {
    Ok(format!(
        "sequence-{:016x}",
        hash_serializable(&(stream_id, sequence)).map_err(RuntimeError::invalid)?
    ))
}

fn cursor_for<'a>(
    head: &'a KnowledgeHead,
    principal: &str,
    stream_id: &str,
) -> Option<&'a KnowledgeCursor> {
    head.cursors
        .binary_search_by(|cursor| {
            (cursor.principal.as_str(), cursor.stream_id.as_str()).cmp(&(principal, stream_id))
        })
        .ok()
        .and_then(|index| head.cursors.get(index))
}

fn replace_cursor(head: &mut KnowledgeHead, cursor: KnowledgeCursor) {
    head.cursors
        .retain(|prior| prior.principal != cursor.principal || prior.stream_id != cursor.stream_id);
    head.cursors.push(cursor);
}

fn forward_partitions(
    head: &KnowledgeHead,
    principal: &str,
    partitions: &[KnowledgePartition],
) -> RuntimeResult<BTreeMap<ChunkId, KnowledgePartition>> {
    let mut replacements = BTreeMap::new();
    for partition in partitions {
        if let Some(previous) =
            KnowledgeStore::head_descriptor(head, principal, partition.coordinate)
        {
            if previous.revision > partition.revision {
                continue;
            }
            if previous.revision == partition.revision {
                if previous.fingerprint != partition.fingerprint {
                    return Err(RuntimeError::new(
                        ErrorKind::Conflict,
                        "same knowledge revision carries different disclosed facts",
                    ));
                }
                continue;
            }
        }
        replacements.insert(partition.coordinate, partition.clone());
    }
    Ok(replacements)
}
