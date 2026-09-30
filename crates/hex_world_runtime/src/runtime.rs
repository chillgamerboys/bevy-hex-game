use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{mpsc, Arc, Mutex},
    thread,
    time::Instant,
};

use hex_world_contracts::{
    ChunkDescriptor, ChunkId, ChunkPackage, ColumnData, ManifestIndex, QueryResult,
    ResidencyRequest, Surface, VoxelPosition, WorldHex, WorldManifest, WorldQuery,
};

use crate::{
    history::HistoryEntry,
    persistence::{ChunkOverlay, OverlayLocation},
    source::{in_disk, source_index, validate_source_chunk},
    CancellationToken, ChunkSource, ErrorKind, RuntimeError, RuntimeResult,
};

/// Explicit bounds on active work; none is a total-world column limit.
#[derive(Debug, Clone, Copy)]
pub struct RuntimeConfig {
    /// Maximum retained fine chunks, including separated interest islands and pins.
    pub max_resident_chunks: usize,
    /// Maximum live worker threads, including canceled jobs until they actually finish.
    pub max_in_flight_jobs: usize,
    /// Maximum completed chunk products admitted by one pump.
    pub max_publications_per_pump: usize,
    /// Maximum distinct interest requests in one update.
    pub max_interests: usize,
    /// Maximum chunk coordinates examined for one interest/retention disk.
    pub max_interest_probes: usize,
    /// Maximum exact terrain assignments, object edits, or affected object columns per transaction.
    pub max_edits_per_transaction: usize,
    /// Maximum independent operation pin owners.
    pub max_pin_owners: usize,
    /// Maximum modified partitions awaiting a durable checkpoint, resident or not.
    pub max_unsaved_chunks: usize,
    /// Maximum opaque owner keys explicitly changed by one checkpoint operation.
    pub max_attachment_updates: usize,
    /// Maximum serialized body bytes for one terrain or object transaction.
    pub max_transaction_bytes: usize,
    /// Maximum uncheckpointed transaction identities with resident bodies.
    pub max_unsaved_transactions: usize,
    /// Maximum combined serialized bytes of uncheckpointed transaction bodies.
    pub max_unsaved_transaction_bytes: usize,
    /// Maximum durable recent transaction bodies retained in memory; zero disables caching.
    pub max_cached_transactions: usize,
    /// Maximum serialized bytes in the durable recent-body cache.
    pub max_cached_transaction_bytes: usize,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            max_resident_chunks: 256,
            max_in_flight_jobs: 4,
            max_publications_per_pump: 4,
            max_interests: 64,
            max_interest_probes: 65_536,
            max_edits_per_transaction: 4096,
            max_pin_owners: 256,
            max_unsaved_chunks: 256,
            max_attachment_updates: 64,
            max_transaction_bytes: 8 * 1024 * 1024,
            max_unsaved_transactions: 256,
            max_unsaved_transaction_bytes: 32 * 1024 * 1024,
            max_cached_transactions: 32,
            max_cached_transaction_bytes: 16 * 1024 * 1024,
        }
    }
}

/// Immutable, revision-tagged payload for an engine's logical/render publication.
#[derive(Debug, Clone)]
pub struct ChunkProduct {
    /// Global chunk identity.
    pub coordinate: ChunkId,
    /// Mutable authority revision; zero is the compiled base.
    pub revision: u64,
    /// Canonical current terrain and semantics, shared without a deep copy.
    pub package: Arc<ChunkPackage>,
}

/// A failed load that published no partial terrain.
#[derive(Debug, Clone)]
pub struct LoadFailure {
    /// Requested global chunk.
    pub coordinate: ChunkId,
    /// Exact rejection; failed desired chunks require an explicit retry.
    pub error: RuntimeError,
}

/// Products emitted by one bounded residency pump.
#[derive(Debug, Default)]
pub struct RuntimeUpdate {
    /// Newly available chunks.
    pub loaded: Vec<ChunkProduct>,
    /// Existing chunks atomically replaced by successful transactions.
    pub changed: Vec<ChunkProduct>,
    /// Retired chunks whose engine entities/assets should be removed.
    pub removed: Vec<ChunkId>,
    /// Failed background jobs; unaffected chunks remain available.
    pub failures: Vec<LoadFailure>,
}

/// Runtime cardinalities for bounded-work acceptance and presentation diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeCounts {
    /// Currently queryable chunks.
    pub resident_chunks: usize,
    /// Actual live jobs, including canceled ones not yet completed.
    pub in_flight_jobs: usize,
    /// Desired chunks waiting for a worker slot.
    pub queued_chunks: usize,
    /// Union of all explicitly pinned chunks.
    pub pinned_chunks: usize,
    /// Modified partitions, whether resident or persisted/unloaded.
    pub modified_chunks: usize,
}

/// Constant-memory observations of successfully admitted chunk load latency.
///
/// The interval is worker launch through queryable admission: source IO, decode,
/// validation, completion queue wait, and final query-product preparation. It
/// excludes waiting for a worker slot before launch. Failed, canceled, and stale
/// jobs never contribute. A source replacement starts a fresh measurement epoch.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LoadTiming {
    /// Successful current chunks admitted since construction or source replacement.
    pub samples: u64,
    /// Exponential moving average in milliseconds, with fixed new-sample weight 1/8.
    pub ema_milliseconds: Option<f64>,
    /// Largest successful launch-to-admission duration in this source epoch.
    pub max_milliseconds: Option<f64>,
}
impl LoadTiming {
    fn record(&mut self, milliseconds: f64) {
        self.samples = self.samples.saturating_add(1);
        self.ema_milliseconds = Some(self.ema_milliseconds.map_or(milliseconds, |previous| {
            previous + (milliseconds - previous) / 8.0
        }));
        self.max_milliseconds = Some(
            self.max_milliseconds
                .map_or(milliseconds, |previous| previous.max(milliseconds)),
        );
    }
}

pub(crate) struct ResidentChunk {
    pub product: ChunkProduct,
    pub base_fingerprint: u64,
    pub query_columns: BTreeMap<WorldHex, ColumnData>,
}

struct Job {
    launched: Instant,
    coordinate: ChunkId,
    cancellation: CancellationToken,
    handle: thread::JoinHandle<()>,
    completion_received: bool,
}

struct Completion {
    ticket: u64,
    coordinate: ChunkId,
    epoch: u64,
    revision: u64,
    result: RuntimeResult<(ChunkPackage, Option<ChunkOverlay>)>,
}

/// World-owned data and residency independent of any render or encounter clock.
pub struct WorldRuntime {
    pub(crate) source: Arc<dyn ChunkSource>,
    pub(crate) manifest: WorldManifest,
    pub(crate) descriptors: BTreeMap<ChunkId, ChunkDescriptor>,
    pub(crate) config: RuntimeConfig,
    pub(crate) resident: BTreeMap<ChunkId, ResidentChunk>,
    pub(crate) overlays: BTreeMap<ChunkId, OverlayLocation>,
    pub(crate) persisted: BTreeMap<ChunkId, OverlayLocation>,
    pub(crate) dirty: BTreeSet<ChunkId>,
    pub(crate) transactions: BTreeMap<String, HistoryEntry>,
    pub(crate) history_order: VecDeque<String>,
    pub(crate) unsaved_transactions: BTreeSet<String>,
    pub(crate) unsaved_transaction_bytes: usize,
    pub(crate) attachments: crate::attachments::AttachmentLocations,
    pub(crate) attachment_bindings: BTreeMap<String, u64>,
    pub(crate) manifest_index: Arc<ManifestIndex>,
    interests: Vec<ResidencyRequest>,
    desired: BTreeMap<ChunkId, u8>,
    retained: BTreeSet<ChunkId>,
    pins: BTreeMap<String, BTreeSet<ChunkId>>,
    jobs: BTreeMap<u64, Job>,
    tickets: BTreeMap<ChunkId, u64>,
    failed: BTreeSet<ChunkId>,
    next_ticket: u64,
    load_timing: LoadTiming,
    epoch: u64,
    sender: mpsc::SyncSender<Completion>,
    receiver: Mutex<mpsc::Receiver<Completion>>,
    pub(crate) pending_changed: BTreeMap<ChunkId, ChunkProduct>,
    pending_removed: BTreeSet<ChunkId>,
}

impl WorldRuntime {
    /// Opens a validated catalogue without loading all world chunks.
    pub fn new(source: Arc<dyn ChunkSource>, config: RuntimeConfig) -> RuntimeResult<Self> {
        let manifest_index = source_index(source.as_ref())?;
        if config.max_resident_chunks == 0
            || config.max_in_flight_jobs == 0
            || config.max_publications_per_pump == 0
            || config.max_interests == 0
            || config.max_interest_probes == 0
            || config.max_edits_per_transaction == 0
            || config.max_pin_owners == 0
            || config.max_unsaved_chunks == 0
            || config.max_attachment_updates == 0
            || config.max_transaction_bytes == 0
            || config.max_unsaved_transactions == 0
            || config.max_unsaved_transaction_bytes == 0
        {
            return Err(RuntimeError::new(
                ErrorKind::Limit,
                "runtime bounds must all be nonzero",
            ));
        }
        let manifest = source.manifest().clone();
        let descriptors = manifest
            .chunks
            .iter()
            .map(|descriptor| (descriptor.coordinate, descriptor.clone()))
            .collect();
        let (sender, receiver) = mpsc::sync_channel(config.max_in_flight_jobs);
        Ok(Self {
            source,
            manifest,
            descriptors,
            config,
            resident: BTreeMap::new(),
            overlays: BTreeMap::new(),
            persisted: BTreeMap::new(),
            dirty: BTreeSet::new(),
            transactions: BTreeMap::new(),
            history_order: VecDeque::new(),
            unsaved_transactions: BTreeSet::new(),
            unsaved_transaction_bytes: 0,
            attachments: BTreeMap::new(),
            attachment_bindings: BTreeMap::new(),
            manifest_index,
            interests: Vec::new(),
            desired: BTreeMap::new(),
            retained: BTreeSet::new(),
            pins: BTreeMap::new(),
            jobs: BTreeMap::new(),
            tickets: BTreeMap::new(),
            failed: BTreeSet::new(),
            next_ticket: 1,
            load_timing: LoadTiming::default(),
            epoch: 0,
            sender,
            receiver: Mutex::new(receiver),
            pending_changed: BTreeMap::new(),
            pending_removed: BTreeSet::new(),
        })
    }

    /// Current immutable world catalogue, also suitable for a coarse map/minimap.
    #[must_use]
    pub fn manifest(&self) -> &WorldManifest {
        &self.manifest
    }

    /// Updates the complete union of actor/operation interests atomically.
    ///
    /// Higher numeric priority loads first; equal priorities use canonical chunk
    /// order. Retention only preserves already resident chunks, never loads the
    /// hysteresis band by itself. Over-budget requests leave prior state unchanged.
    pub fn set_interests(&mut self, interests: Vec<ResidencyRequest>) -> RuntimeResult<()> {
        let (desired, retained) = self.plan_interest(&interests, &self.pins)?;
        self.interests = interests;
        self.desired = desired;
        self.retained = retained;
        self.reconcile_residency();
        Ok(())
    }

    /// Pins exact chunks for an actor route, encounter, or world transaction.
    /// A pin may request a not-yet-loaded chunk; availability remains explicit.
    pub fn pin(
        &mut self,
        owner: impl Into<String>,
        chunks: BTreeSet<ChunkId>,
    ) -> RuntimeResult<()> {
        let owner = owner.into();
        validate_identity(&owner)?;
        if chunks.is_empty() {
            return Err(RuntimeError::invalid("pin has no chunks"));
        }
        if chunks.len() > self.config.max_resident_chunks
            || chunks
                .iter()
                .any(|chunk| !self.descriptors.contains_key(chunk))
        {
            return Err(RuntimeError::new(
                ErrorKind::Limit,
                "pin exceeds resident budget or names an outside-world chunk",
            ));
        }
        let mut pins = self.pins.clone();
        pins.insert(owner, chunks);
        if pins.len() > self.config.max_pin_owners {
            return Err(RuntimeError::new(ErrorKind::Limit, "too many pin owners"));
        }
        let (desired, retained) = self.plan_interest(&self.interests, &pins)?;
        self.pins = pins;
        self.desired = desired;
        self.retained = retained;
        self.reconcile_residency();
        Ok(())
    }

    /// Releases one owner's pin, preserving overlapping pins and interests.
    pub fn unpin(&mut self, owner: &str) -> RuntimeResult<()> {
        let mut pins = self.pins.clone();
        pins.remove(owner);
        let (desired, retained) = self.plan_interest(&self.interests, &pins)?;
        self.pins = pins;
        self.desired = desired;
        self.retained = retained;
        self.reconcile_residency();
        Ok(())
    }

    /// Allows another bounded attempt after a reported load failure.
    pub fn retry(&mut self, coordinate: ChunkId) {
        self.failed.remove(&coordinate);
    }

    /// Polls completed jobs without waiting, then dispatches available worker slots.
    ///
    /// Canceled jobs retain their worker slot until completion. No pump can launch
    /// unbounded detached work by rapidly alternating interests.
    pub fn pump(&mut self) -> RuntimeUpdate {
        // A source's thread-local/Arc destructors can outlive its completion send.
        // Never join a worker on the caller's frame until it is already finished.
        let finished = self
            .jobs
            .iter()
            .filter(|(_, job)| job.completion_received && job.handle.is_finished())
            .map(|(ticket, _)| *ticket)
            .collect::<Vec<_>>();
        for ticket in finished {
            if let Some(job) = self.jobs.remove(&ticket) {
                let _joined = job.handle.join();
            }
        }
        let mut update = RuntimeUpdate {
            removed: std::mem::take(&mut self.pending_removed)
                .into_iter()
                .collect(),
            ..RuntimeUpdate::default()
        };
        for _ in 0..self.config.max_publications_per_pump {
            let Some((_, product)) = self.pending_changed.pop_first() else {
                break;
            };
            update.changed.push(product);
        }
        for _ in update.changed.len()..self.config.max_publications_per_pump {
            let completion = match self.receiver.lock() {
                Ok(receiver) => receiver.try_recv(),
                Err(poisoned) => poisoned.into_inner().try_recv(),
            };
            let Ok(completion) = completion else {
                break;
            };
            if let Some(job) = self.jobs.get_mut(&completion.ticket) {
                job.completion_received = true;
            }
            let current = self.tickets.get(&completion.coordinate).copied()
                == Some(completion.ticket)
                && self.epoch == completion.epoch
                && self.desired.contains_key(&completion.coordinate)
                && self.overlay_revision(completion.coordinate) == completion.revision;
            if !current {
                continue;
            }
            self.tickets.remove(&completion.coordinate);
            match completion.result {
                Ok((package, overlay)) => {
                    let Some(descriptor) = self.descriptors.get(&completion.coordinate) else {
                        continue;
                    };
                    let query_columns = match combined_object_columns(&package) {
                        Ok(columns) => columns,
                        Err(error) => {
                            self.failed.insert(completion.coordinate);
                            update.failures.push(LoadFailure {
                                coordinate: completion.coordinate,
                                error,
                            });
                            continue;
                        }
                    };
                    if let Some(overlay) = overlay {
                        self.overlays
                            .insert(completion.coordinate, OverlayLocation::Memory(overlay));
                    }
                    let product = ChunkProduct {
                        coordinate: completion.coordinate,
                        revision: completion.revision,
                        package: Arc::new(package),
                    };
                    self.resident.insert(
                        completion.coordinate,
                        ResidentChunk {
                            product: product.clone(),
                            base_fingerprint: descriptor.fingerprint,
                            query_columns,
                        },
                    );
                    if let Some(job) = self.jobs.get(&completion.ticket) {
                        self.load_timing
                            .record(job.launched.elapsed().as_secs_f64() * 1000.0);
                    }
                    update.loaded.push(product);
                }
                Err(error) => {
                    self.failed.insert(completion.coordinate);
                    update.failures.push(LoadFailure {
                        coordinate: completion.coordinate,
                        error,
                    });
                }
            }
        }
        let mut candidates = self
            .desired
            .iter()
            .filter(|(chunk, _)| {
                !self.resident.contains_key(chunk)
                    && !self.tickets.contains_key(chunk)
                    && !self.failed.contains(chunk)
            })
            .map(|(chunk, priority)| (*priority, *chunk))
            .collect::<Vec<_>>();
        candidates.sort_by_key(|(priority, chunk)| (std::cmp::Reverse(*priority), *chunk));
        for (_, coordinate) in candidates {
            if self.jobs.len() >= self.config.max_in_flight_jobs {
                break;
            }
            if let Err(error) = self.spawn_job(coordinate) {
                self.failed.insert(coordinate);
                update.failures.push(LoadFailure { coordinate, error });
            }
        }
        update
    }

    /// Immutable resident product for engine publication and diagnostics.
    #[must_use]
    pub fn resident_chunk(&self, coordinate: ChunkId) -> Option<ChunkProduct> {
        self.resident
            .get(&coordinate)
            .map(|chunk| chunk.product.clone())
    }

    /// Every resident product in canonical chunk order.
    pub fn resident_chunks(&self) -> impl Iterator<Item = ChunkProduct> + '_ {
        self.resident.values().map(|chunk| chunk.product.clone())
    }

    /// Successful source-load timing for bounded directional prefetch heuristics.
    /// No measurement exists until a current chunk has reached queryable admission.
    #[must_use]
    pub fn load_timing(&self) -> LoadTiming {
        self.load_timing
    }

    /// Exact cardinalities; completed/canceled work still occupies a slot until pumped.
    #[must_use]
    pub fn counts(&self) -> RuntimeCounts {
        RuntimeCounts {
            resident_chunks: self.resident.len(),
            in_flight_jobs: self.jobs.len(),
            queued_chunks: self
                .desired
                .keys()
                .filter(|chunk| {
                    !self.resident.contains_key(chunk)
                        && !self.tickets.contains_key(chunk)
                        && !self.failed.contains(chunk)
                })
                .count(),
            pinned_chunks: self
                .pins
                .values()
                .flatten()
                .copied()
                .collect::<BTreeSet<_>>()
                .len(),
            modified_chunks: self.overlays.len(),
        }
    }

    /// Replaces an immutable source catalogue, rejecting incompatible durable edits.
    /// Changed pinned chunks cannot be replaced. Every older job loses admission rights.
    pub fn replace_source(&mut self, source: Arc<dyn ChunkSource>) -> RuntimeResult<()> {
        let manifest_index = source_index(source.as_ref())?;
        if source.manifest().world_id != self.manifest.world_id {
            return Err(RuntimeError::new(
                ErrorKind::Conflict,
                "source belongs to a different world",
            ));
        }
        // Consumer revision proofs do not carry a separate material/boundary policy epoch.
        // Retaining their identities across a policy change would permit stale cached facts.
        if source.manifest().materials != self.manifest.materials
            || source.manifest().boundaries != self.manifest.boundaries
        {
            return Err(RuntimeError::new(
                ErrorKind::Conflict,
                "changed material or boundary policy requires a new runtime and fresh consumer adapters",
            ));
        }
        if (!self.transactions.is_empty() || !self.attachments.is_empty())
            && source.manifest().fingerprint != self.manifest.fingerprint
        {
            return Err(RuntimeError::new(
                ErrorKind::Conflict,
                "edited worlds require the exact original source or a fresh save",
            ));
        }
        let descriptors: BTreeMap<_, _> = source
            .manifest()
            .chunks
            .iter()
            .map(|descriptor| (descriptor.coordinate, descriptor.clone()))
            .collect();
        for (coordinate, overlay) in &self.overlays {
            if descriptors
                .get(coordinate)
                .map(|descriptor| descriptor.fingerprint)
                != Some(overlay.base_fingerprint())
            {
                return Err(RuntimeError::new(
                    ErrorKind::Conflict,
                    "new source invalidates a modified partition",
                ));
            }
        }
        let changed: BTreeSet<_> = self
            .descriptors
            .iter()
            .filter(|(chunk, descriptor)| {
                descriptors
                    .get(chunk)
                    .is_none_or(|new| new.fingerprint != descriptor.fingerprint)
            })
            .map(|(chunk, _)| *chunk)
            .collect();
        if self
            .pins
            .values()
            .flatten()
            .any(|chunk| changed.contains(chunk))
        {
            return Err(RuntimeError::new(
                ErrorKind::Pinned,
                "source change touches a pinned chunk",
            ));
        }
        let next_epoch = self
            .epoch
            .checked_add(1)
            .ok_or_else(|| RuntimeError::new(ErrorKind::Limit, "source epoch exhausted"))?;
        // Validate new interest applicability before replacing any authority state.
        let previous_descriptors = std::mem::replace(&mut self.descriptors, descriptors);
        let previous_manifest = std::mem::replace(&mut self.manifest, source.manifest().clone());
        let previous_index = std::mem::replace(&mut self.manifest_index, manifest_index);
        let planned = self.plan_interest(&self.interests, &self.pins);
        let (desired, retained) = match planned {
            Ok(planned) => planned,
            Err(error) => {
                self.descriptors = previous_descriptors;
                self.manifest = previous_manifest;
                self.manifest_index = previous_index;
                return Err(error);
            }
        };
        self.source = source;
        self.epoch = next_epoch;
        self.load_timing = LoadTiming::default();
        for job in self.jobs.values() {
            job.cancellation.cancel();
        }
        self.tickets.clear();
        self.failed.clear();
        for coordinate in changed {
            self.retire(coordinate);
        }
        self.desired = desired;
        self.retained = retained;
        self.reconcile_residency();
        Ok(())
    }

    pub(crate) fn overlay_revision(&self, coordinate: ChunkId) -> u64 {
        self.overlays
            .get(&coordinate)
            .map_or(0, OverlayLocation::revision)
    }

    pub(crate) fn has_running_jobs(&self) -> bool {
        !self.jobs.is_empty()
    }

    pub(crate) fn has_pins(&self) -> bool {
        !self.pins.is_empty()
    }

    pub(crate) fn invalidate_after_restore(&mut self) {
        let coordinates = self.resident.keys().copied().collect::<Vec<_>>();
        for coordinate in coordinates {
            self.retire(coordinate);
        }
        self.failed.clear();
    }

    fn plan_interest(
        &self,
        interests: &[ResidencyRequest],
        pins: &BTreeMap<String, BTreeSet<ChunkId>>,
    ) -> RuntimeResult<(BTreeMap<ChunkId, u8>, BTreeSet<ChunkId>)> {
        if interests.len() > self.config.max_interests {
            return Err(RuntimeError::new(
                ErrorKind::Limit,
                "too many residency interests",
            ));
        }
        let mut identities = BTreeSet::new();
        let mut desired: BTreeMap<ChunkId, u8> = BTreeMap::new();
        let mut retained = BTreeSet::new();
        for request in interests {
            validate_identity(&request.id)?;
            if !identities.insert(&request.id) || request.retention_radius < request.radius {
                return Err(RuntimeError::invalid(
                    "duplicate interest or retention radius smaller than activation radius",
                ));
            }
            for coordinate in self.disk_chunks(request.center, request.radius)? {
                desired
                    .entry(coordinate)
                    .and_modify(|priority| *priority = (*priority).max(request.priority))
                    .or_insert(request.priority);
            }
            retained.extend(self.disk_chunks(request.center, request.retention_radius)?);
        }
        for coordinate in pins.values().flatten() {
            if !self.descriptors.contains_key(coordinate) {
                return Err(RuntimeError::new(
                    ErrorKind::Conflict,
                    "pin names a chunk absent from source",
                ));
            }
            desired.insert(*coordinate, u8::MAX);
            retained.insert(*coordinate);
        }
        let resident_retained = self
            .resident
            .keys()
            .filter(|coordinate| retained.contains(coordinate))
            .copied();
        if desired
            .keys()
            .copied()
            .chain(resident_retained)
            .collect::<BTreeSet<_>>()
            .len()
            > self.config.max_resident_chunks
        {
            return Err(RuntimeError::new(
                ErrorKind::Limit,
                "interest union and retained/pinned chunks exceed resident budget",
            ));
        }
        Ok((desired, retained))
    }

    fn disk_chunks(&self, center: WorldHex, radius: u32) -> RuntimeResult<Vec<ChunkId>> {
        let radius = i64::from(radius);
        let minimum_q = center
            .q
            .checked_sub(radius)
            .ok_or_else(|| RuntimeError::invalid("interest q underflow"))?
            .div_euclid(16);
        let maximum_q = center
            .q
            .checked_add(radius)
            .ok_or_else(|| RuntimeError::invalid("interest q overflow"))?
            .div_euclid(16);
        let minimum_r = center
            .r
            .checked_sub(radius)
            .ok_or_else(|| RuntimeError::invalid("interest r underflow"))?
            .div_euclid(16);
        let maximum_r = center
            .r
            .checked_add(radius)
            .ok_or_else(|| RuntimeError::invalid("interest r overflow"))?
            .div_euclid(16);
        let probes = (i128::from(maximum_q) - i128::from(minimum_q) + 1)
            * (i128::from(maximum_r) - i128::from(minimum_r) + 1);
        if probes > i128::try_from(self.config.max_interest_probes).unwrap_or(i128::MAX) {
            return Err(RuntimeError::new(
                ErrorKind::Limit,
                "interest disk exceeds bounded coordinate probes",
            ));
        }
        let mut chunks = Vec::new();
        for q in minimum_q..=maximum_q {
            for r in minimum_r..=maximum_r {
                let coordinate = ChunkId { q, r };
                if !self.descriptors.contains_key(&coordinate) {
                    continue;
                }
                let origin = coordinate.origin().map_err(RuntimeError::invalid)?;
                // Exact disk/chunk intersection including clipped region boundaries.
                let mut intersects = false;
                for local_q in 0..16_i64 {
                    for local_r in 0..16_i64 {
                        let position = origin
                            .checked_add(WorldHex::new(local_q, local_r))
                            .map_err(RuntimeError::invalid)?;
                        if in_disk(position, center, u32::try_from(radius).unwrap_or(u32::MAX))
                            && self.contains_column(position)
                        {
                            intersects = true;
                            break;
                        }
                    }
                    if intersects {
                        break;
                    }
                }
                if intersects {
                    chunks.push(coordinate);
                }
            }
        }
        Ok(chunks)
    }

    fn contains_column(&self, position: WorldHex) -> bool {
        matches!(self.manifest_index.contains(position), Ok(true))
    }

    fn reconcile_residency(&mut self) {
        let retired = self
            .resident
            .keys()
            .filter(|coordinate| {
                !self.retained.contains(coordinate) && !self.desired.contains_key(coordinate)
            })
            .copied()
            .collect::<Vec<_>>();
        for coordinate in retired {
            self.retire(coordinate);
        }
        for job in self.jobs.values() {
            if !self.desired.contains_key(&job.coordinate) {
                job.cancellation.cancel();
                self.tickets.remove(&job.coordinate);
            }
        }
        self.failed
            .retain(|coordinate| self.desired.contains_key(coordinate));
    }

    fn retire(&mut self, coordinate: ChunkId) {
        if self.resident.remove(&coordinate).is_some() {
            self.pending_changed.remove(&coordinate);
            self.pending_removed.insert(coordinate);
        }
        if let Some(location) = self.persisted.get(&coordinate) {
            if location.revision() == self.overlay_revision(coordinate) {
                self.overlays.insert(coordinate, location.clone());
            }
        }
    }

    fn spawn_job(&mut self, coordinate: ChunkId) -> RuntimeResult<()> {
        let ticket = self.next_ticket;
        self.next_ticket = ticket
            .checked_add(1)
            .ok_or_else(|| RuntimeError::new(ErrorKind::Limit, "load ticket exhausted"))?;
        let source = Arc::clone(&self.source);
        let index = Arc::clone(&self.manifest_index);
        let descriptor = self
            .descriptors
            .get(&coordinate)
            .cloned()
            .ok_or_else(|| RuntimeError::invalid("requested chunk absent from manifest"))?;
        let overlay = self.overlays.get(&coordinate).cloned();
        let revision = self.overlay_revision(coordinate);
        let epoch = self.epoch;
        let cancellation = CancellationToken::default();
        let worker_cancellation = cancellation.clone();
        let sender = self.sender.clone();
        let launched = Instant::now();
        let handle = thread::Builder::new()
            .name(format!("hex-chunk-{}-{}", coordinate.q, coordinate.r))
            .spawn(move || {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    let mut package =
                        source.load_chunk_cancelled(coordinate, &worker_cancellation)?;
                    validate_source_chunk(&index, &descriptor, &package)?;
                    let overlay = overlay
                        .map(|location| location.load(&worker_cancellation))
                        .transpose()?;
                    if let Some(overlay) = &overlay {
                        overlay.apply(&mut package, &index)?;
                    }
                    worker_cancellation.check()?;
                    Ok((package, overlay))
                }))
                .unwrap_or_else(|_| Err(RuntimeError::invalid("chunk source worker panicked")));
                let _delivered = sender.send(Completion {
                    ticket,
                    coordinate,
                    epoch,
                    revision,
                    result,
                });
            })
            .map_err(RuntimeError::io)?;
        self.tickets.insert(coordinate, ticket);
        self.jobs.insert(
            ticket,
            Job {
                launched,
                coordinate,
                cancellation,
                handle,
                completion_received: false,
            },
        );
        Ok(())
    }

    fn column(&self, position: WorldHex) -> QueryResult<&ColumnData> {
        if !self.contains_column(position) {
            return QueryResult::OutsideWorld;
        }
        let coordinate = position.chunk();
        let Some(resident) = self.resident.get(&coordinate) else {
            return QueryResult::Unloaded(coordinate);
        };
        if let Some(combined) = resident.query_columns.get(&position) {
            return QueryResult::Ready(combined);
        }
        resident
            .product
            .package
            .columns
            .binary_search_by_key(&position, |column| column.position)
            .ok()
            .and_then(|index| resident.product.package.columns.get(index))
            .map_or(QueryResult::Unloaded(coordinate), QueryResult::Ready)
    }
}

// Object projections are clipped by the compiler into every affected chunk. A
// foreign root may be unloaded without removing collision or inventing free air.
pub(crate) fn combined_object_columns(
    package: &ChunkPackage,
) -> RuntimeResult<BTreeMap<WorldHex, ColumnData>> {
    let mut combined = BTreeMap::new();
    for object in &package.semantics.occupancy {
        let terrain = package
            .columns
            .binary_search_by_key(&object.position, |column| column.position)
            .ok()
            .and_then(|index| package.columns.get(index))
            .ok_or_else(|| RuntimeError::invalid("object projection lacks local terrain column"))?;
        let endpoints = terrain
            .runs
            .iter()
            .chain(&object.runs)
            .flat_map(|run| [run.bottom, run.top])
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let mut runs: Vec<hex_world_contracts::VoxelRun> = Vec::new();
        let mut terrain_runs = terrain.runs.iter().peekable();
        let mut object_runs = object.runs.iter().peekable();
        for pair in endpoints.windows(2) {
            let Some((&bottom, &top)) = pair.first().zip(pair.get(1)) else {
                continue;
            };
            while terrain_runs.peek().is_some_and(|run| run.top <= bottom) {
                terrain_runs.next();
            }
            while object_runs.peek().is_some_and(|run| run.top <= bottom) {
                object_runs.next();
            }
            let Some(material) = object_runs
                .peek()
                .filter(|run| run.bottom <= bottom)
                .map(|run| run.material.as_str())
                .or_else(|| {
                    terrain_runs
                        .peek()
                        .filter(|run| run.bottom <= bottom)
                        .map(|run| run.material.as_str())
                })
            else {
                continue;
            };
            if let Some(prior) = runs.last_mut() {
                if prior.top == bottom && prior.material == material {
                    prior.top = top;
                    continue;
                }
            }
            runs.push(hex_world_contracts::VoxelRun {
                bottom,
                top,
                material: material.to_owned(),
            });
        }
        let column = ColumnData {
            position: object.position,
            runs,
        };
        // This derived union can have twice the wire column's run count; each
        // independently validated input remains bounded. The interval sweep
        // above already guarantees ordering, disjointness, and coalescing.
        combined.insert(object.position, column);
    }
    Ok(combined)
}

impl WorldQuery for WorldRuntime {
    fn voxel(&self, position: VoxelPosition) -> QueryResult<Option<String>> {
        match self.column(position.column) {
            QueryResult::Ready(column) => QueryResult::Ready(
                column
                    .runs
                    .iter()
                    .find(|run| run.bottom <= position.level && position.level < run.top)
                    .map(|run| run.material.clone()),
            ),
            QueryResult::Unloaded(coordinate) => QueryResult::Unloaded(coordinate),
            QueryResult::OutsideWorld => QueryResult::OutsideWorld,
        }
    }

    fn surfaces(&self, position: WorldHex) -> QueryResult<Vec<Surface>> {
        match self.column(position) {
            QueryResult::Ready(column) => {
                let mut surfaces = Vec::new();
                for (index, run) in column.runs.iter().enumerate() {
                    if !self
                        .manifest
                        .materials
                        .iter()
                        .any(|material| material.id == run.material && material.solid)
                    {
                        continue;
                    }
                    let next = column.runs.get(index.saturating_add(1));
                    if next.is_some_and(|next| next.bottom == run.top) {
                        continue;
                    }
                    let headroom = next.map(|next| {
                        u32::try_from(i64::from(next.bottom) - i64::from(run.top))
                            .unwrap_or(u32::MAX)
                    });
                    if let Some(level) = run.top.checked_sub(1) {
                        surfaces.push(Surface {
                            position: VoxelPosition {
                                column: position,
                                level,
                            },
                            material: run.material.clone(),
                            headroom,
                        });
                    }
                }
                QueryResult::Ready(surfaces)
            }
            QueryResult::Unloaded(coordinate) => QueryResult::Unloaded(coordinate),
            QueryResult::OutsideWorld => QueryResult::OutsideWorld,
        }
    }

    fn revision(&self, coordinate: ChunkId) -> Option<u64> {
        self.resident
            .get(&coordinate)
            .map(|chunk| chunk.product.revision)
    }
}

impl Drop for WorldRuntime {
    fn drop(&mut self) {
        for job in self.jobs.values() {
            job.cancellation.cancel();
        }
    }
}

pub(crate) fn validate_identity(value: &str) -> RuntimeResult<()> {
    if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
        return Err(RuntimeError::invalid(
            "identity must contain 1..128 bytes without controls",
        ));
    }
    Ok(())
}
