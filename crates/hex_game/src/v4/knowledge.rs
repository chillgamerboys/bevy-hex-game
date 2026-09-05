//! Principal-private observation and bounded, asynchronous memory persistence.
//!
//! Rendering never supplies observations. Exact resident world revisions feed the
//! shared sight adapter; unavailable observers wait independently. Only nearby fine
//! memory partitions are read, while compact discovery masks may outlive residency.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
};

use bevy::prelude::Resource;
use hex_core::{ExteriorIllumination, IlluminationLevel, SightProfile};
use hex_perception::v4::{
    ObservationResult, ObserverFacts, ObserverRequest, PerceptionConfig, PerceptionWorld,
};
use hex_world_contracts::{ChunkId, ManifestIndex, WorldHex, hash_serializable};
use hex_world_runtime::{
    IoLimits, KnowledgeConfig, KnowledgePartition, KnowledgeReceipt, KnowledgeStore,
    ObservedLandmark, ObservedSurface, WorldRuntime,
};
use serde::Serialize;

use super::Session;

const MAX_ACTORS: usize = 7;
const MAX_IO_JOBS: usize = 8;
const MAX_LOCAL_PARTITIONS: usize = 64;
const MAX_COMPACT_LANDMARKS: usize = 131_072;

/// Facts exposed only for the explicitly selected owning principal.
pub(super) struct PrincipalView {
    pub principal: String,
    /// Current complete sight, absent while this observer's inputs are pending.
    pub current: Option<Arc<ObserverFacts>>,
    /// Observed or locally restored stable landmarks; never a manifest feature dump.
    pub landmarks: BTreeMap<String, ObservedLandmark>,
    /// False when dormant preexisting partitions may hold additional landmarks.
    pub landmark_catalogue_complete: bool,
    discovery: BTreeMap<ChunkId, [u64; 4]>,
}

impl PrincipalView {
    pub fn discovered(&self, column: WorldHex) -> bool {
        let Some(mask) = self.discovery.get(&column.chunk()) else {
            return false;
        };
        let bit = column_bit(column);
        mask.get(bit / 64)
            .is_some_and(|word| word & (1_u64 << (bit % 64)) != 0)
    }

    pub fn discovered_chunks(&self) -> impl Iterator<Item = ChunkId> + '_ {
        self.discovery.keys().copied()
    }

    pub fn discovered_column_count(&self) -> u64 {
        self.discovery
            .values()
            .flat_map(|mask| mask.iter())
            .map(|word| u64::from(word.count_ones()))
            .sum()
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub(super) struct KnowledgeCounts {
    pub active_observers: usize,
    pub completed_observers: usize,
    pub current_surfaces: usize,
    pub cached_partitions: usize,
    pub dirty_partitions: usize,
    pub pending_io_jobs: usize,
    pub persisted_batches: u64,
    pub perception_cache_hits: u64,
    pub perception_cache_misses: u64,
}

struct CachedPartition {
    draft: KnowledgePartition,
    persisted_revision: u64,
    dirty: bool,
}

struct PendingWrite {
    id: String,
    partitions: BTreeMap<ChunkId, KnowledgePartition>,
}

struct PrincipalState {
    view: PrincipalView,
    catalog_requested: bool,
    catalog_loaded: bool,
    unloaded_landmark_chunks: BTreeSet<ChunkId>,
    required: BTreeSet<ChunkId>,
    cache: BTreeMap<ChunkId, CachedPartition>,
    loading: BTreeSet<ChunkId>,
    pending_write: Option<PendingWrite>,
}

impl PrincipalState {
    fn new(principal: String) -> Self {
        Self {
            view: PrincipalView {
                principal,
                current: None,
                landmarks: BTreeMap::new(),
                landmark_catalogue_complete: false,
                discovery: BTreeMap::new(),
            },
            catalog_requested: false,
            catalog_loaded: false,
            unloaded_landmark_chunks: BTreeSet::new(),
            required: BTreeSet::new(),
            cache: BTreeMap::new(),
            loading: BTreeSet::new(),
            pending_write: None,
        }
    }

    fn install_compact(&mut self, partition: &KnowledgePartition) -> Result<(), String> {
        for column in &partition.discovered_columns {
            mark_column(
                self.view.discovery.entry(column.chunk()).or_default(),
                *column,
            );
        }
        for landmark in &partition.landmarks {
            if !self.view.landmarks.contains_key(&landmark.id)
                && self.view.landmarks.len() >= MAX_COMPACT_LANDMARKS
            {
                return Err("principal compact landmark budget exceeded".into());
            }
            self.view
                .landmarks
                .insert(landmark.id.clone(), landmark.clone());
        }
        self.unloaded_landmark_chunks.remove(&partition.coordinate);
        self.view.landmark_catalogue_complete = self.unloaded_landmark_chunks.is_empty();
        Ok(())
    }

    fn retire_distant(&mut self) {
        let writing = self.pending_write.as_ref();
        self.cache.retain(|coordinate, cached| {
            self.required.contains(coordinate)
                || cached.dirty
                || writing.is_some_and(|batch| batch.partitions.contains_key(coordinate))
        });
    }
}

type Catalog = BTreeMap<ChunkId, [u64; 4]>;

enum IoJob {
    Catalog {
        principal: String,
    },
    Read {
        principal: String,
        coordinate: ChunkId,
    },
    Write {
        principal: String,
        id: String,
        expected: BTreeMap<ChunkId, u64>,
        replacements: BTreeMap<ChunkId, KnowledgePartition>,
    },
}

enum IoEvent {
    Catalog {
        principal: String,
        result: Result<Catalog, String>,
    },
    Read {
        principal: String,
        coordinate: ChunkId,
        result: Result<Option<KnowledgePartition>, String>,
    },
    Write {
        principal: String,
        result: Result<KnowledgeReceipt, String>,
    },
}

/// The worker owns its store and never touches actors, rendering, or world authority.
fn io_worker(
    mut store: KnowledgeStore,
    jobs: mpsc::Receiver<IoJob>,
    events: mpsc::SyncSender<IoEvent>,
    temporary: Option<PathBuf>,
) {
    while let Ok(job) = jobs.recv() {
        let event = match job {
            IoJob::Catalog { principal } => {
                let result = store.discovered_chunks(&principal).and_then(|chunks| {
                    chunks
                        .into_iter()
                        .map(|coordinate| {
                            let mut mask = [0; 4];
                            for column in store.discovered_columns(&principal, coordinate)? {
                                mark_column(&mut mask, column);
                            }
                            Ok((coordinate, mask))
                        })
                        .collect()
                });
                IoEvent::Catalog {
                    principal,
                    result: result.map_err(|error| error.to_string()),
                }
            }
            IoJob::Read {
                principal,
                coordinate,
            } => IoEvent::Read {
                result: store
                    .read(&principal, coordinate)
                    .map_err(|error| error.to_string()),
                principal,
                coordinate,
            },
            IoJob::Write {
                principal,
                id,
                expected,
                replacements,
            } => IoEvent::Write {
                result: store
                    .compare_and_write(&principal, &id, &expected, replacements)
                    .map_err(|error| error.to_string()),
                principal,
            },
        };
        if events.send(event).is_err() {
            break;
        }
    }
    // This path is unique to this transient instance; saved knowledge is never removed.
    if let Some(directory) = temporary {
        let _removed = std::fs::remove_dir_all(directory);
    }
}

#[derive(Resource)]
pub(super) struct WorldKnowledge {
    perception: PerceptionWorld,
    principals: BTreeMap<String, PrincipalState>,
    projected: BTreeMap<ChunkId, u64>,
    jobs: mpsc::SyncSender<IoJob>,
    events: Mutex<mpsc::Receiver<IoEvent>>,
    in_flight: usize,
    persisted_batches: u64,
    active_observers: usize,
    schedule_cursor: usize,
}

impl WorldKnowledge {
    pub fn open(runtime: &WorldRuntime, save: Option<&Path>) -> Result<Self, String> {
        let temporary = if save.is_none() {
            Some(temporary_directory()?)
        } else {
            None
        };
        let directory = match (save, &temporary) {
            (Some(save), _) => save.join("knowledge"),
            (None, Some(directory)) => directory.clone(),
            (None, None) => return Err("transient knowledge directory was not created".into()),
        };
        let store = KnowledgeStore::open(
            &directory,
            runtime.manifest(),
            IoLimits::default(),
            KnowledgeConfig::default(),
        )
        .map_err(|error| error.to_string())?;
        let index = Arc::new(
            ManifestIndex::new(Arc::new(runtime.manifest().clone()))
                .map_err(|error| error.to_string())?,
        );
        let perception = PerceptionWorld::new(
            index,
            PerceptionConfig {
                max_resident_chunks: MAX_ACTORS * MAX_LOCAL_PARTITIONS,
                max_cached_observers: MAX_ACTORS,
                ..PerceptionConfig::default()
            },
        )
        .map_err(|error| error.to_string())?;
        let (jobs, receive_jobs) = mpsc::sync_channel(MAX_IO_JOBS);
        let (send_events, events) = mpsc::sync_channel(MAX_IO_JOBS);
        std::thread::Builder::new()
            .name("v4-knowledge-io".into())
            .spawn(move || io_worker(store, receive_jobs, send_events, temporary))
            .map_err(|error| error.to_string())?;
        Ok(Self {
            perception,
            principals: BTreeMap::new(),
            projected: BTreeMap::new(),
            jobs,
            events: Mutex::new(events),
            in_flight: 0,
            persisted_batches: 0,
            active_observers: 0,
            schedule_cursor: 0,
        })
    }

    pub fn selected(&self, session: &Session) -> Option<&PrincipalView> {
        self.principals
            .get(&session.actors.get(session.selected)?.id)
            .map(|state| &state.view)
    }

    pub fn counts(&self) -> KnowledgeCounts {
        let perception = self.perception.counts();
        KnowledgeCounts {
            active_observers: self.active_observers,
            completed_observers: self
                .principals
                .values()
                .filter(|state| state.view.current.is_some())
                .count(),
            current_surfaces: self
                .principals
                .values()
                .filter_map(|state| state.view.current.as_ref())
                .map(|facts| facts.surfaces.len())
                .sum(),
            cached_partitions: self
                .principals
                .values()
                .map(|state| state.cache.len())
                .sum(),
            dirty_partitions: self
                .principals
                .values()
                .flat_map(|state| state.cache.values())
                .filter(|cached| cached.dirty)
                .count(),
            pending_io_jobs: self.in_flight,
            persisted_batches: self.persisted_batches,
            perception_cache_hits: perception.cache_hits,
            perception_cache_misses: perception.cache_misses,
        }
    }

    /// Capture/save completion also requires a complete observation for every actor.
    pub fn idle(&self) -> bool {
        let counts = self.counts();
        counts.active_observers > 0
            && counts.completed_observers == counts.active_observers
            && counts.pending_io_jobs == 0
            && counts.dirty_partitions == 0
            && self
                .principals
                .values()
                .all(|state| state.catalog_loaded && state.pending_write.is_none())
    }

    /// Call after the runtime pump and ordinary actor movement/edit publication.
    pub fn tick(&mut self, session: &Session, runtime: &mut WorldRuntime) -> Result<(), String> {
        self.receive()?;
        if session.actors.is_empty() || session.actors.len() > MAX_ACTORS {
            return Err("knowledge observer roster must contain one to seven actors".into());
        }
        self.active_observers = session.actors.len();
        let active = session
            .actors
            .iter()
            .map(|actor| actor.id.clone())
            .collect::<BTreeSet<_>>();
        if active.len() != session.actors.len() {
            return Err("knowledge observer IDs must be unique".into());
        }
        let retired = self
            .principals
            .keys()
            .filter(|id| !active.contains(*id))
            .cloned()
            .collect::<Vec<_>>();
        for id in retired {
            runtime
                .unpin(&format!("knowledge/{id}"))
                .map_err(|error| error.to_string())?;
            self.perception.remove_observer(&id);
            // Preserve pending writes until their ACK; a roster replacement is explicit.
            if self.principals.get(&id).is_some_and(|state| {
                state.pending_write.is_some() || state.cache.values().any(|cached| cached.dirty)
            }) {
                return Err("cannot retire an observer with undurable knowledge".into());
            }
            self.principals.remove(&id);
        }
        let mut requests = Vec::new();
        for actor in &session.actors {
            let state = self
                .principals
                .entry(actor.id.clone())
                .or_insert_with(|| PrincipalState::new(actor.id.clone()));
            let Some(position) = actor.standing else {
                state.view.current = None;
                if !state.required.is_empty() {
                    runtime
                        .unpin(&format!("knowledge/{}", actor.id))
                        .map_err(|error| error.to_string())?;
                    state.required.clear();
                    state.retire_distant();
                }
                continue;
            };
            let request = ObserverRequest {
                id: actor.id.clone(),
                principal: actor.id.clone(),
                position,
                profile: SightProfile::DEFAULT,
                exterior: ExteriorIllumination::new(IlluminationLevel::Bright),
            };
            let required = self
                .perception
                .required_chunks(&request)
                .map_err(|error| error.to_string())?
                .into_iter()
                .collect::<BTreeSet<_>>();
            if required.len() > MAX_LOCAL_PARTITIONS {
                return Err("observer sight exceeds the bounded memory neighborhood".into());
            }
            if required != state.required {
                if required.is_empty() {
                    runtime
                        .unpin(&format!("knowledge/{}", actor.id))
                        .map_err(|error| error.to_string())?;
                } else {
                    runtime
                        .pin(format!("knowledge/{}", actor.id), required.clone())
                        .map_err(|error| error.to_string())?;
                }
                state.required = required;
            }
            state.retire_distant();
            requests.push(request);
        }
        let required = self
            .principals
            .values()
            .flat_map(|state| state.required.iter().copied())
            .collect::<BTreeSet<_>>();
        let removed = self
            .projected
            .keys()
            .filter(|coordinate| {
                !required.contains(*coordinate) || runtime.resident_chunk(**coordinate).is_none()
            })
            .copied()
            .collect::<Vec<_>>();
        for coordinate in removed {
            self.projected.remove(&coordinate);
            self.perception.remove(coordinate);
        }
        for coordinate in required {
            if let Some(product) = runtime.resident_chunk(coordinate) {
                if self.projected.get(&coordinate) != Some(&product.revision) {
                    self.perception
                        .publish(product.package, product.revision)
                        .map_err(|error| error.to_string())?;
                    self.projected.insert(coordinate, product.revision);
                }
            }
        }
        if !requests.is_empty() {
            let start = self.schedule_cursor % requests.len();
            requests.rotate_left(start);
            self.schedule_cursor = (start + 1) % requests.len();
        }
        for request in requests {
            self.prepare_memory(&request.principal)?;
            // Outgoing dirty pages must be able to flush while incoming memory waits.
            self.schedule_write(&request.principal)?;
            let state = self
                .principals
                .get_mut(&request.principal)
                .ok_or("observer disappeared")?;
            if !state.catalog_loaded
                || state
                    .required
                    .iter()
                    .any(|coordinate| !state.cache.contains_key(coordinate))
            {
                state.view.current = None;
                continue;
            }
            let radius = SightProfile::DEFAULT.bright.radius;
            let memory = state
                .cache
                .values()
                .flat_map(|cached| cached.draft.surfaces.iter())
                .filter_map(|fact| {
                    let position = fact.surface.position;
                    request
                        .position
                        .column
                        .checked_distance(position.column)
                        .ok()
                        .filter(|distance| *distance <= u64::from(radius))
                        .map(|_| position)
                })
                .collect::<Vec<_>>();
            match self
                .perception
                .observe_with_memory(&request, &memory, runtime)
                .map_err(|error| error.to_string())?
            {
                ObservationResult::Ready(facts) => {
                    if !state
                        .view
                        .current
                        .as_ref()
                        .is_some_and(|previous| Arc::ptr_eq(previous, &facts))
                    {
                        merge_facts(state, &facts)?;
                    }
                    state.view.current = Some(facts);
                }
                ObservationResult::Pending(_) => state.view.current = None,
                ObservationResult::OutsideWorld => {
                    return Err(format!("observer {} stands outside the world", request.id));
                }
            }
            self.schedule_write(&request.principal)?;
        }
        Ok(())
    }

    fn queue(&mut self, job: IoJob) -> Result<bool, String> {
        if self.in_flight >= MAX_IO_JOBS {
            return Ok(false);
        }
        match self.jobs.try_send(job) {
            Ok(()) => {
                self.in_flight += 1;
                Ok(true)
            }
            Err(mpsc::TrySendError::Full(_)) => Ok(false),
            Err(mpsc::TrySendError::Disconnected(_)) => {
                Err("knowledge persistence worker stopped".into())
            }
        }
    }

    fn prepare_memory(&mut self, principal: &str) -> Result<(), String> {
        let state = self
            .principals
            .get_mut(principal)
            .ok_or("missing principal")?;
        if !state.catalog_loaded {
            if !state.catalog_requested
                && self.queue(IoJob::Catalog {
                    principal: principal.into(),
                })?
            {
                self.principals
                    .get_mut(principal)
                    .ok_or("missing principal")?
                    .catalog_requested = true;
            }
            return Ok(());
        }
        let mut read = None;
        for coordinate in &state.required {
            if state.cache.contains_key(coordinate) || state.loading.contains(coordinate) {
                continue;
            }
            if state.cache.len() + state.loading.len() >= MAX_LOCAL_PARTITIONS {
                // Dirty outgoing partitions must become durable before reclaiming a slot.
                break;
            }
            if state.view.discovery.contains_key(coordinate) {
                read = Some(*coordinate);
                break;
            }
            state.cache.insert(
                *coordinate,
                CachedPartition {
                    draft: KnowledgePartition::new(principal, *coordinate),
                    persisted_revision: 0,
                    dirty: false,
                },
            );
        }
        if let Some(coordinate) = read {
            if self.queue(IoJob::Read {
                principal: principal.into(),
                coordinate,
            })? {
                self.principals
                    .get_mut(principal)
                    .ok_or("missing principal")?
                    .loading
                    .insert(coordinate);
            }
        }
        Ok(())
    }

    fn schedule_write(&mut self, principal: &str) -> Result<(), String> {
        let state = self.principals.get(principal).ok_or("missing principal")?;
        if state.pending_write.is_some() {
            return Ok(());
        }
        let mut expected = BTreeMap::new();
        let mut replacements = BTreeMap::new();
        for (coordinate, cached) in &state.cache {
            if !cached.dirty {
                continue;
            }
            let mut replacement = cached.draft.clone();
            replacement.revision = cached
                .persisted_revision
                .checked_add(1)
                .ok_or("knowledge revision overflow")?;
            replacement.seal().map_err(|error| error.to_string())?;
            expected.insert(*coordinate, cached.persisted_revision);
            replacements.insert(*coordinate, replacement);
        }
        if replacements.is_empty() {
            return Ok(());
        }
        let hash = hash_serializable(&(principal, &expected, &replacements))
            .map_err(|error| error.to_string())?;
        let id = format!("observation/{hash:016x}");
        if self.queue(IoJob::Write {
            principal: principal.into(),
            id: id.clone(),
            expected,
            replacements: replacements.clone(),
        })? {
            self.principals
                .get_mut(principal)
                .ok_or("missing principal")?
                .pending_write = Some(PendingWrite {
                id,
                partitions: replacements,
            });
        }
        Ok(())
    }

    fn receive(&mut self) -> Result<(), String> {
        for _ in 0..MAX_IO_JOBS {
            let event = match self
                .events
                .lock()
                .map_err(|_| "knowledge event queue poisoned")?
                .try_recv()
            {
                Ok(event) => event,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err("knowledge persistence worker stopped".into());
                }
            };
            self.in_flight = self
                .in_flight
                .checked_sub(1)
                .ok_or("unsolicited knowledge completion")?;
            match event {
                IoEvent::Catalog { principal, result } => {
                    let discovery = result?;
                    let Some(state) = self.principals.get_mut(&principal) else {
                        continue;
                    };
                    state.unloaded_landmark_chunks = discovery.keys().copied().collect();
                    state.view.landmark_catalogue_complete = discovery.is_empty();
                    state.view.discovery = discovery;
                    state.catalog_loaded = true;
                }
                IoEvent::Read {
                    principal,
                    coordinate,
                    result,
                } => {
                    let partition = result?.ok_or("known memory partition disappeared")?;
                    let Some(state) = self.principals.get_mut(&principal) else {
                        continue;
                    };
                    state.loading.remove(&coordinate);
                    state.install_compact(&partition)?;
                    if state.required.contains(&coordinate) {
                        state.cache.insert(
                            coordinate,
                            CachedPartition {
                                persisted_revision: partition.revision,
                                draft: partition,
                                dirty: false,
                            },
                        );
                    }
                }
                IoEvent::Write { principal, result } => {
                    let receipt = result?;
                    let state = self
                        .principals
                        .get_mut(&principal)
                        .ok_or("retired knowledge write completion")?;
                    let written = state
                        .pending_write
                        .take()
                        .ok_or("unexpected knowledge write completion")?;
                    if receipt.principal != principal
                        || receipt.transaction_id != written.id
                        || receipt.revisions
                            != written
                                .partitions
                                .iter()
                                .map(|(coordinate, partition)| (*coordinate, partition.revision))
                                .collect::<Vec<_>>()
                    {
                        return Err(
                            "knowledge receipt differs from the exact submitted batch".into()
                        );
                    }
                    for (coordinate, partition) in written.partitions {
                        let cached = state
                            .cache
                            .get_mut(&coordinate)
                            .ok_or("pending knowledge partition was evicted")?;
                        cached.persisted_revision = partition.revision;
                        cached.draft.revision = partition.revision;
                        cached.dirty = !same_content(&cached.draft, &partition);
                        cached.draft.fingerprint = if cached.dirty {
                            0
                        } else {
                            partition.fingerprint
                        };
                    }
                    state.retire_distant();
                    self.persisted_batches = self.persisted_batches.saturating_add(1);
                }
            }
        }
        Ok(())
    }
}

#[expect(
    clippy::expect_used,
    reason = "Euclidean remainders make the result exactly 0..256 on every platform"
)]
fn column_bit(column: WorldHex) -> usize {
    usize::try_from(column.q.rem_euclid(16) * 16 + column.r.rem_euclid(16))
        .expect("chunk-local coordinates are within 0..256")
}

fn mark_column(mask: &mut [u64; 4], column: WorldHex) {
    let bit = column_bit(column);
    if let Some(word) = mask.get_mut(bit / 64) {
        *word |= 1_u64 << (bit % 64);
    }
}

fn same_content(a: &KnowledgePartition, b: &KnowledgePartition) -> bool {
    a.principal == b.principal
        && a.coordinate == b.coordinate
        && a.discovered_columns == b.discovered_columns
        && a.surfaces == b.surfaces
        && a.landmarks == b.landmarks
}

fn merge_facts(state: &mut PrincipalState, facts: &ObserverFacts) -> Result<(), String> {
    if facts.principal != state.view.principal {
        return Err("observation principal mismatch".into());
    }
    let affected = facts
        .surfaces
        .iter()
        .map(|fact| fact.surface.position.column.chunk())
        .chain(
            facts
                .landmarks
                .iter()
                .map(|fact| fact.feature.anchor.column.chunk()),
        )
        .chain(
            facts
                .invalidated_surfaces
                .iter()
                .map(|position| position.column.chunk()),
        )
        .collect::<BTreeSet<_>>();
    for coordinate in affected {
        let cached = state
            .cache
            .get_mut(&coordinate)
            .ok_or("observation has no loaded private memory partition")?;
        let mut candidate = cached.draft.clone();
        let mut discovery = candidate
            .discovered_columns
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let mut surfaces = candidate
            .surfaces
            .iter()
            .cloned()
            .map(|fact| (fact.surface.position, fact))
            .collect::<BTreeMap<_, _>>();
        let mut landmarks = candidate
            .landmarks
            .iter()
            .cloned()
            .map(|fact| (fact.id.clone(), fact))
            .collect::<BTreeMap<_, _>>();
        for position in &facts.invalidated_surfaces {
            if position.column.chunk() == coordinate {
                surfaces.remove(position);
            }
        }
        for fact in &facts.surfaces {
            if fact.surface.position.column.chunk() == coordinate {
                discovery.insert(fact.surface.position.column);
                surfaces.insert(
                    fact.surface.position,
                    ObservedSurface {
                        surface: fact.surface.clone(),
                        world_revision: fact.world_revision,
                    },
                );
            }
        }
        for fact in &facts.landmarks {
            if fact.feature.anchor.column.chunk() == coordinate {
                discovery.insert(fact.feature.anchor.column);
                landmarks.insert(
                    fact.feature.id.clone(),
                    ObservedLandmark {
                        id: fact.feature.id.clone(),
                        position: fact.feature.anchor,
                        world_revision: fact.world_revision,
                    },
                );
            }
        }
        candidate.discovered_columns = discovery.into_iter().collect();
        candidate.surfaces = surfaces.into_values().collect();
        candidate.landmarks = landmarks.into_values().collect();
        if !same_content(&candidate, &cached.draft) {
            cached.draft = candidate.clone();
            cached.dirty = true;
            state.install_compact(&candidate)?;
        }
    }
    Ok(())
}

fn temporary_directory() -> Result<PathBuf, String> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "hex-v4-knowledge-{}-{nanos}-{serial}",
        std::process::id()
    ));
    std::fs::create_dir(&directory).map_err(|error| error.to_string())?;
    Ok(directory)
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "fixture construction and explicit success assertions"
)]
mod tests {
    use super::*;
    use hex_perception::v4::VisibleSurface;
    use hex_world_contracts::{
        Surface, VoxelEdit, VoxelPosition, WorldEditTransaction, WorldQuery,
    };

    fn position(q: i64, level: i32) -> VoxelPosition {
        VoxelPosition {
            column: WorldHex::new(q, 0),
            level,
        }
    }

    fn observed(position: VoxelPosition, revision: u64) -> ObservedSurface {
        ObservedSurface {
            surface: Surface {
                position,
                material: "stone".into(),
                headroom: None,
            },
            world_revision: revision,
        }
    }

    #[test]
    fn visible_absence_removes_only_its_exact_stack_and_unchanged_facts_do_not_dirty() {
        let low = position(-1, 2);
        let high = position(-1, 8);
        let hidden = position(-2, 2);
        let coordinate = low.column.chunk();
        let mut state = PrincipalState::new("a".into());
        let mut partition = KnowledgePartition::new("a", coordinate);
        partition.discovered_columns = vec![hidden.column, low.column];
        partition.surfaces = vec![observed(hidden, 0), observed(low, 0), observed(high, 0)];
        partition.revision = 1;
        partition.seal().expect("stacked memory");
        state.install_compact(&partition).expect("compact memory");
        state.cache.insert(
            coordinate,
            CachedPartition {
                draft: partition,
                persisted_revision: 1,
                dirty: false,
            },
        );
        let mut facts = ObserverFacts {
            observer_id: "a".into(),
            principal: "a".into(),
            position: high,
            surfaces: vec![VisibleSurface {
                surface: observed(high, 1).surface,
                world_revision: 1,
                illumination: IlluminationLevel::Bright,
            }],
            invalidated_surfaces: vec![low],
            landmarks: Vec::new(),
            dependencies: Vec::new(),
            inspected_columns: 2,
            tested_surfaces: 2,
        };
        merge_facts(&mut state, &facts).expect("private visible update");
        let cached = state.cache.get_mut(&coordinate).expect("local memory");
        assert!(cached.dirty);
        assert_eq!(
            cached
                .draft
                .surfaces
                .iter()
                .map(|fact| fact.surface.position)
                .collect::<Vec<_>>(),
            vec![hidden, high]
        );
        cached.dirty = false;
        merge_facts(&mut state, &facts).expect("same observation");
        assert!(!state.cache.get(&coordinate).expect("cache").dirty);
        assert!(state.view.discovered(low.column));
        assert_eq!(state.view.discovered_column_count(), 2);
        assert_eq!(
            state.view.discovered_chunks().collect::<Vec<_>>(),
            vec![coordinate]
        );
        facts.principal = "b".into();
        assert!(merge_facts(&mut state, &facts).is_err());
    }

    fn settle(knowledge: &mut WorldKnowledge, session: &Session, runtime: &mut WorldRuntime) {
        for _ in 0..2_000 {
            assert!(runtime.pump().failures.is_empty());
            knowledge
                .tick(session, runtime)
                .expect("bounded knowledge update");
            if knowledge.idle() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(
            knowledge.idle(),
            "knowledge did not settle: {:?}",
            knowledge.counts()
        );
    }

    #[test]
    fn separated_step_mode_observers_progress_privately_and_stationary_sight_does_not_write() {
        let (mut session, mut runtime) = crate::v4::walk::tests::fixture();
        session.actors.get_mut(0).expect("first actor").turn_steps = true;
        let mut knowledge = WorldKnowledge::open(&runtime, None).expect("transient knowledge");
        settle(&mut knowledge, &session, &mut runtime);
        assert_eq!(knowledge.counts().completed_observers, 2);
        let first = knowledge
            .selected(&session)
            .expect("selected first principal");
        assert!(first.discovered(position(14, 2).column));
        assert!(!first.discovered(position(100, 2).column));
        session.selected = 1;
        let second = knowledge
            .selected(&session)
            .expect("selected second principal");
        assert!(second.discovered(position(100, 2).column));
        assert!(!second.discovered(position(14, 2).column));
        let writes = knowledge.counts().persisted_batches;
        for _ in 0..16 {
            knowledge
                .tick(&session, &mut runtime)
                .expect("unchanged observations");
        }
        assert!(knowledge.idle());
        assert_eq!(knowledge.counts().persisted_batches, writes);
        assert_eq!(knowledge.counts().pending_io_jobs, 0);
        assert!(knowledge.counts().perception_cache_hits > 0);
    }

    #[test]
    fn pending_actor_does_not_block_another_principal_and_capture_cannot_finish_early() {
        let (mut session, mut runtime) = crate::v4::walk::tests::fixture();
        session.actors.get_mut(0).expect("first actor").standing = None;
        let mut knowledge = WorldKnowledge::open(&runtime, None).expect("transient knowledge");
        for _ in 0..2_000 {
            knowledge
                .tick(&session, &mut runtime)
                .expect("independent pending observer");
            if knowledge.counts().completed_observers == 1
                && knowledge.counts().pending_io_jobs == 0
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(knowledge.counts().completed_observers, 1);
        assert!(!knowledge.idle());
        session.selected = 1;
        assert!(
            knowledge
                .selected(&session)
                .expect("second principal")
                .current
                .is_some()
        );
        session.actors.get_mut(0).expect("first actor").standing = Some(position(14, 2));
        settle(&mut knowledge, &session, &mut runtime);
        assert_eq!(knowledge.counts().completed_observers, 2);
    }

    #[test]
    fn edit_observation_updates_exact_memory_and_reopens_without_rewriting_it() {
        let (session, mut runtime) = crate::v4::walk::tests::fixture();
        let directory = temporary_directory().expect("saved fixture directory");
        let mut knowledge =
            WorldKnowledge::open(&runtime, Some(&directory)).expect("saved knowledge");
        settle(&mut knowledge, &session, &mut runtime);
        let removed = position(13, 2);
        runtime
            .apply_transaction(&WorldEditTransaction {
                id: "knowledge-dig".into(),
                expected_revisions: BTreeMap::from([(
                    removed.column.chunk(),
                    runtime
                        .revision(removed.column.chunk())
                        .expect("loaded terrain"),
                )]),
                edits: vec![VoxelEdit {
                    position: removed,
                    material: None,
                }],
            })
            .expect("exact neighboring dig");
        settle(&mut knowledge, &session, &mut runtime);
        let state = knowledge.principals.get("a").expect("first party");
        let cached = state
            .cache
            .get(&removed.column.chunk())
            .expect("nearby fine memory");
        assert!(
            !cached
                .draft
                .surfaces
                .iter()
                .any(|fact| fact.surface.position == removed)
        );
        assert!(
            cached
                .draft
                .surfaces
                .iter()
                .any(|fact| fact.surface.position == position(13, 1))
        );
        drop(knowledge);
        let mut reopened = WorldKnowledge::open(&runtime, Some(&directory))
            .expect("reopened source-bound knowledge");
        settle(&mut reopened, &session, &mut runtime);
        assert_eq!(reopened.counts().persisted_batches, 0);
        assert!(
            reopened
                .selected(&session)
                .expect("restored private view")
                .discovered(removed.column)
        );
        drop(reopened);
        std::fs::remove_dir_all(directory).expect("remove saved fixture");
    }
}
