//! Bounded resident V4 sight, using the existing exact illumination and ray rules.
//!
//! The host supplies immutable revision-tagged chunks and an authoritative WorldQuery.
//! Missing in-world dependencies yield Pending before sight evaluation. This module
//! never manufactures knowledge from residency and never owns an encounter clock.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fmt,
    sync::Arc,
};

use hex_core::{
    AuthoredObjectVoxelRun, ExteriorIllumination, GameplayLight, HexCoord, IlluminationLevel,
    InteriorRegionId, LightDomain, RunBottom, SightProfile, TilePos,
};
use hex_units::{AuthoredObjectOccupancy, SightOccupancyCache, TerrainOccupancy};
use hex_world_contracts::{
    hash_serializable, ChunkId, ChunkPackage, ContractError, FeatureSummary, ManifestIndex,
    QueryResult, Surface, VoxelPosition, WorldHex, WorldLight, WorldQuery,
};

use crate::{resolve_illumination_at, LightSourceSnapshot, ResolvedLight};

/// An explicit malformed input, incompatible projection, or bounded-work failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Invalid data or disagreement between the host's typed world projections.
    Invalid(String),
    /// One operation would exceed a configured cost or representable local range.
    Limit(String),
}
impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => write!(formatter, "invalid perception: {message}"),
            Self::Limit(message) => write!(formatter, "perception limit: {message}"),
        }
    }
}
impl std::error::Error for Error {}
impl From<ContractError> for Error {
    fn from(error: ContractError) -> Self {
        Self::Invalid(error.to_string())
    }
}
fn invalid(message: impl fmt::Display) -> Error {
    Error::Invalid(message.to_string())
}
fn limit(message: impl fmt::Display) -> Error {
    Error::Limit(message.to_string())
}

/// Limits apply to active projections, one observer, or retained observation caches.
#[derive(Debug, Clone, Copy)]
pub struct PerceptionConfig {
    /// Largest requested horizontal sight radius.
    pub max_radius: u32,
    /// Largest local axial rectangle examined, including the exact ray fringe.
    pub max_column_probes: usize,
    /// Maximum resident chunk products retained by this adapter.
    pub max_resident_chunks: usize,
    /// Maximum compact terrain/object intervals prepared for one observer.
    pub max_runs_per_observer: usize,
    /// Maximum candidate exposed surfaces for one observer.
    pub max_surfaces_per_observer: usize,
    /// Maximum prior support positions accepted in one memory-aware observation.
    pub max_remembered_positions: usize,
    /// Maximum feature records examined from an observer's local chunks.
    pub max_landmarks_per_observer: usize,
    /// Maximum projected light records examined, including shared copies.
    pub max_light_records_per_observer: usize,
    /// Maximum distinct influencing light identities in one observer window.
    pub max_lights_per_observer: usize,
    /// Maximum cached independent observers.
    pub max_cached_observers: usize,
    /// Maximum surface, invalidation, and landmark facts retained across observer caches.
    pub max_cached_facts: usize,
}
impl Default for PerceptionConfig {
    fn default() -> Self {
        Self {
            max_radius: 96,
            max_column_probes: 65_536,
            max_resident_chunks: 256,
            max_runs_per_observer: 65_536,
            max_surfaces_per_observer: 16_384,
            max_remembered_positions: 16_384,
            max_landmarks_per_observer: 4096,
            max_light_records_per_observer: 65_536,
            max_lights_per_observer: 4096,
            max_cached_observers: 32,
            max_cached_facts: 65_536,
        }
    }
}

/// Exact host-selected observer; principal identity never comes from terrain or a renderer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObserverRequest {
    /// Stable observer identity, independent of its current chunk or render origin.
    pub id: String,
    /// Owning party/player; observations are never pooled across principals here.
    pub principal: String,
    /// Current exact support surface.
    pub position: VoxelPosition,
    /// Existing illumination-dependent sight limits.
    pub profile: SightProfile,
    /// Objective exterior illumination, independent of renderer lights.
    pub exterior: ExteriorIllumination,
}

/// One exact terrain dependency of a completed observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkRevision {
    /// Global storage identity.
    pub coordinate: ChunkId,
    /// Authority revision supplied with the immutable chunk product.
    pub revision: u64,
}

/// One currently visible support, preserving every distinct stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibleSurface {
    /// Exact material support and clearance from WorldQuery.
    pub surface: Surface,
    /// Terrain revision at observation time.
    pub world_revision: u64,
    /// Objective illumination used by the shared sight predicate.
    pub illumination: IlluminationLevel,
}

/// One visible stable source feature, tested at its exact authored anchor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibleLandmark {
    /// Registered global feature; no hidden object geometry is included.
    pub feature: FeatureSummary,
    /// Revision of the anchor's chunk at observation time.
    pub world_revision: u64,
}

/// A landmark previously observed by the requesting principal, never a catalogue dump.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RememberedLandmark {
    /// Stable authored feature identity.
    pub id: String,
    /// Exact previously observed anchor.
    pub position: VoxelPosition,
}

/// A remembered asset landmark whose absence is now proved by exact current sight.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidatedLandmark {
    /// Stable remembered identity; never reveals an unobserved deleted source feature.
    pub id: String,
    /// Exact anchor at which the remembered object is visibly absent.
    pub position: VoxelPosition,
    /// Current revision of the anchor's chunk.
    pub world_revision: u64,
}

/// Complete current observations for one observer, suitable for explicit declassification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObserverFacts {
    /// Observer that produced these facts.
    pub observer_id: String,
    /// Principal entitled to receive these observations.
    pub principal: String,
    /// Exact support at which the observation was resolved.
    pub position: VoxelPosition,
    /// Currently visible exact supports, in global voxel order.
    pub surfaces: Vec<VisibleSurface>,
    /// Prior support positions visibly absent now; hidden or unavailable memory is retained.
    pub invalidated_surfaces: Vec<VoxelPosition>,
    /// Currently visible stable landmarks, in ID order.
    pub landmarks: Vec<VisibleLandmark>,
    /// Remembered asset landmarks visibly absent now, in ID order.
    pub invalidated_landmarks: Vec<InvalidatedLandmark>,
    /// Local revision dependencies, in chunk order.
    pub dependencies: Vec<ChunkRevision>,
    /// Local world/outside columns examined; independent of dormant world size.
    pub inspected_columns: usize,
    /// Current and remembered absent support candidates whose visibility was tested.
    pub tested_surfaces: usize,
}

/// Availability of an observation, separate from an empty visible set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObservationResult {
    /// Complete immutable facts, possibly reused from a valid observer cache.
    Ready(Arc<ObserverFacts>),
    /// Required chunks are unavailable or publication/query revisions disagree.
    /// No partial new observations are disclosed.
    Pending(Vec<ChunkId>),
    /// Observer support lies outside the finite world footprint.
    OutsideWorld,
}

/// Typed bounded-work diagnostics, independent of frame timing or pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PerceptionCounts {
    /// Resident source products, with no dormant chunk bodies.
    pub resident_chunks: usize,
    /// Cached independent observer results.
    pub cached_observers: usize,
    /// Total current surface, invalidation, and landmark facts in those bounded caches.
    pub cached_facts: usize,
    /// Requests served without rerunning light or sight work.
    pub cache_hits: u64,
    /// Requests that needed dependency gathering or exact visibility work.
    pub cache_misses: u64,
}

#[derive(Debug)]
struct DomainSpan {
    floor: i32,
    ceiling: i32,
    domain: String,
}
struct ResidentProjection {
    revision: u64,
    package: Arc<ChunkPackage>,
    domains: BTreeMap<WorldHex, Vec<DomainSpan>>,
}
impl ResidentProjection {
    fn new(package: Arc<ChunkPackage>, revision: u64) -> Result<Self, Error> {
        let mut domains: BTreeMap<WorldHex, Vec<DomainSpan>> = BTreeMap::new();
        for span in &package.semantics.interiors {
            domains.entry(span.column).or_default().push(DomainSpan {
                floor: span.floor_level,
                ceiling: span.roof_bottom,
                domain: span.light_domain.clone(),
            });
        }
        for spans in domains.values_mut() {
            spans.sort_by_key(|span| (span.floor, span.ceiling));
            let mut merged: Vec<DomainSpan> = Vec::new();
            for span in spans.drain(..) {
                if let Some(previous) = merged.last_mut() {
                    if span.floor < previous.ceiling {
                        if span.domain != previous.domain {
                            return Err(invalid("overlapping interior domains are ambiguous"));
                        }
                        previous.ceiling = previous.ceiling.max(span.ceiling);
                        continue;
                    }
                }
                merged.push(span);
            }
            *spans = merged;
        }
        Ok(Self {
            revision,
            package,
            domains,
        })
    }
    fn domain(&self, position: VoxelPosition) -> Option<&str> {
        let spans = self.domains.get(&position.column)?;
        let index = spans
            .partition_point(|span| span.floor <= position.level)
            .checked_sub(1)?;
        spans
            .get(index)
            .filter(|span| position.level < span.ceiling)
            .map(|span| span.domain.as_str())
    }
}
struct CachedObserver {
    request: ObserverRequest,
    memory_fingerprint: u64,
    facts: Arc<ObserverFacts>,
}
struct LocalColumn {
    global: Option<WorldHex>,
    local: HexCoord,
}
struct Footprint {
    columns: Vec<LocalColumn>,
    chunks: BTreeSet<ChunkId>,
    radius: u32,
    extent: i32,
}

/// Resident-only perception authority using shared exact rules and per-observer caches.
pub struct PerceptionWorld {
    index: Arc<ManifestIndex>,
    config: PerceptionConfig,
    resident: BTreeMap<ChunkId, ResidentProjection>,
    cached: BTreeMap<String, CachedObserver>,
    recent: VecDeque<String>,
    cached_facts: usize,
    cache_hits: u64,
    cache_misses: u64,
}
impl PerceptionWorld {
    /// Creates a projection adapter from a validated, retained source index.
    pub fn new(index: Arc<ManifestIndex>, config: PerceptionConfig) -> Result<Self, Error> {
        if config.max_column_probes == 0
            || config.max_resident_chunks == 0
            || config.max_runs_per_observer == 0
            || config.max_surfaces_per_observer == 0
            || config.max_remembered_positions == 0
            || config.max_landmarks_per_observer == 0
            || config.max_light_records_per_observer == 0
            || config.max_lights_per_observer == 0
            || config.max_cached_observers == 0
            || config.max_cached_facts == 0
        {
            return Err(limit("perception budgets must be nonzero"));
        }
        Ok(Self {
            index,
            config,
            resident: BTreeMap::new(),
            cached: BTreeMap::new(),
            recent: VecDeque::new(),
            cached_facts: 0,
            cache_hits: 0,
            cache_misses: 0,
        })
    }

    /// Admits one current world product and invalidates only dependent observers.
    /// Same-revision duplicates are accepted; stale or conflicting revisions fail atomically.
    pub fn publish(&mut self, package: Arc<ChunkPackage>, revision: u64) -> Result<(), Error> {
        package.validate_with_index(&self.index)?;
        if revision == 0
            && self
                .index
                .manifest()
                .chunks
                .binary_search_by_key(&package.coordinate, |descriptor| descriptor.coordinate)
                .ok()
                .and_then(|index| self.index.manifest().chunks.get(index))
                .map(|descriptor| descriptor.fingerprint)
                != Some(package.fingerprint)
        {
            return Err(invalid(
                "base perception product differs from compiled descriptor",
            ));
        }
        if let Some(prior) = self.resident.get(&package.coordinate) {
            if revision < prior.revision
                || (revision == prior.revision && package.fingerprint != prior.package.fingerprint)
            {
                return Err(invalid("stale or conflicting perception chunk revision"));
            }
            if revision == prior.revision {
                return Ok(());
            }
        } else if self.resident.len() >= self.config.max_resident_chunks {
            return Err(limit("resident perception chunk budget exceeded"));
        }
        let coordinate = package.coordinate;
        let projection = ResidentProjection::new(package, revision)?;
        self.invalidate(coordinate);
        self.resident.insert(coordinate, projection);
        Ok(())
    }

    /// Retires one source product and only the observers that depended on it.
    pub fn remove(&mut self, coordinate: ChunkId) {
        self.resident.remove(&coordinate);
        self.invalidate(coordinate);
    }

    /// Retires one observer's cached facts when that observer is no longer active.
    pub fn remove_observer(&mut self, id: &str) {
        self.evict(id);
    }

    /// Exact required in-world chunks, including the one-column paired-ray fringe.
    /// The host may use this request to drive interests/pins before observation.
    pub fn required_chunks(&self, request: &ObserverRequest) -> Result<Vec<ChunkId>, Error> {
        self.validate_request(request)?;
        if !self.index.contains(request.position.column)? {
            return Ok(Vec::new());
        }
        Ok(self.footprint(request)?.chunks.into_iter().collect())
    }

    /// Resolves one observer without reading dormant chunks or pooling another principal.
    /// The WorldQuery and published packages must name matching current revisions.
    pub fn observe(
        &mut self,
        request: &ObserverRequest,
        query: &impl WorldQuery,
    ) -> Result<ObservationResult, Error> {
        self.observe_with_memory(request, &[], query)
    }

    /// Resolves current sight plus explicitly supplied principal-owned remembered supports.
    /// Only visibly absent positions are invalidated; unseen memory is never removed.
    /// The host must supply this principal's memory, not another principal's private state.
    pub fn observe_with_memory(
        &mut self,
        request: &ObserverRequest,
        remembered: &[VoxelPosition],
        query: &impl WorldQuery,
    ) -> Result<ObservationResult, Error> {
        self.observe_with_landmark_memory(request, remembered, &[], query)
    }

    /// Resolve current sight and principal-owned support/landmark memory together.
    /// Hidden, distant, or unavailable landmarks are never invalidated. Asset-free
    /// semantic features remain authored observations independent of object edits.
    pub fn observe_with_landmark_memory(
        &mut self,
        request: &ObserverRequest,
        remembered: &[VoxelPosition],
        remembered_landmarks: &[RememberedLandmark],
        query: &impl WorldQuery,
    ) -> Result<ObservationResult, Error> {
        self.validate_request(request)?;
        if remembered.len() > self.config.max_remembered_positions {
            return Err(limit("remembered support input budget exceeded"));
        }
        if remembered_landmarks.len() > self.config.max_landmarks_per_observer {
            return Err(limit("remembered landmark input budget exceeded"));
        }
        if !self.index.contains(request.position.column)? {
            return Ok(ObservationResult::OutsideWorld);
        }
        let mut memory = Vec::new();
        for position in remembered {
            let q = i128::from(position.column.q) - i128::from(request.position.column.q);
            let r = i128::from(position.column.r) - i128::from(request.position.column.r);
            if q.abs().max(r.abs()).max((q + r).abs()) <= i128::from(radius(request.profile))
                && self.index.contains(position.column)?
            {
                memory.push(*position);
            }
        }
        memory.sort_unstable();
        memory.dedup();
        let mut landmark_memory = BTreeMap::new();
        for remembered in remembered_landmarks {
            let Some(feature) = self.index.feature(&remembered.id) else {
                return Err(invalid("remembered landmark is not a registered feature"));
            };
            if feature.anchor != remembered.position {
                return Err(invalid(
                    "remembered landmark anchor differs from its registry",
                ));
            }
            if request
                .position
                .column
                .checked_distance(remembered.position.column)?
                <= u64::from(radius(request.profile))
            {
                landmark_memory.insert(remembered.id.clone(), remembered.position);
            }
        }
        let memory_fingerprint = hash_serializable(&(&memory, &landmark_memory))?;
        if let Some(cached) = self.cached.get(&request.id) {
            if cached.request == *request
                && cached.memory_fingerprint == memory_fingerprint
                && cached.facts.dependencies.iter().all(|dependency| {
                    query.revision(dependency.coordinate) == Some(dependency.revision)
                })
            {
                let facts = Arc::clone(&cached.facts);
                self.cache_hits = self.cache_hits.saturating_add(1);
                return Ok(ObservationResult::Ready(facts));
            }
        }
        self.cache_misses = self.cache_misses.saturating_add(1);
        let footprint = self.footprint(request)?;
        let missing = footprint
            .chunks
            .iter()
            .filter(|coordinate| {
                self.resident
                    .get(coordinate)
                    .is_none_or(|source| query.revision(**coordinate) != Some(source.revision))
            })
            .copied()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Ok(ObservationResult::Pending(missing));
        }
        let result = self.resolve(request, &memory, &landmark_memory, query, &footprint)?;
        if let ObservationResult::Ready(facts) = &result {
            self.cache(request, memory_fingerprint, Arc::clone(facts));
        }
        Ok(result)
    }

    /// Current bounded cardinalities and exact cache work counters.
    #[must_use]
    pub fn counts(&self) -> PerceptionCounts {
        PerceptionCounts {
            resident_chunks: self.resident.len(),
            cached_observers: self.cached.len(),
            cached_facts: self.cached_facts,
            cache_hits: self.cache_hits,
            cache_misses: self.cache_misses,
        }
    }

    fn validate_request(&self, request: &ObserverRequest) -> Result<(), Error> {
        for id in [&request.id, &request.principal] {
            if id.is_empty() || id.len() > 128 || id.chars().any(char::is_control) {
                return Err(invalid(
                    "observer and principal IDs must be bounded nonempty identities",
                ));
            }
        }
        if radius(request.profile) > self.config.max_radius {
            return Err(limit("observer sight radius exceeds configured bound"));
        }
        Ok(())
    }
    fn footprint(&self, request: &ObserverRequest) -> Result<Footprint, Error> {
        let radius = radius(request.profile);
        let extent = radius
            .checked_add(1)
            .and_then(|value| i32::try_from(value).ok())
            .ok_or_else(|| limit("sight fringe cannot be represented"))?;
        let width = i64::from(extent) * 2 + 1;
        let probes = i128::from(width) * i128::from(width);
        if probes > i128::try_from(self.config.max_column_probes).map_err(limit)? {
            return Err(limit("observer local column probe budget exceeded"));
        }
        let mut columns = Vec::new();
        let mut chunks = BTreeSet::new();
        for q in -extent..=extent {
            for r in -extent..=extent {
                if i64::from(q)
                    .abs()
                    .max(i64::from(r).abs())
                    .max((i64::from(q) + i64::from(r)).abs())
                    > i64::from(extent)
                {
                    continue;
                }
                let global = request
                    .position
                    .column
                    .checked_add(WorldHex::new(i64::from(q), i64::from(r)))
                    .ok()
                    .filter(|position| self.index.contains(*position).unwrap_or(false));
                if let Some(position) = global {
                    chunks.insert(position.chunk());
                }
                columns.push(LocalColumn {
                    global,
                    local: HexCoord::from_axial(q, r),
                });
            }
        }
        Ok(Footprint {
            columns,
            chunks,
            radius,
            extent,
        })
    }
    fn resolve(
        &self,
        request: &ObserverRequest,
        memory: &[VoxelPosition],
        landmark_memory: &BTreeMap<String, VoxelPosition>,
        query: &impl WorldQuery,
        footprint: &Footprint,
    ) -> Result<ObservationResult, Error> {
        let mut terrain_runs = Vec::new();
        let mut object_runs = Vec::new();
        let mut targets: BTreeMap<TilePos, (VoxelPosition, Option<String>)> = BTreeMap::new();
        let mut supports = BTreeMap::new();
        let mut landmarks = BTreeMap::new();
        let mut absent_landmarks = BTreeMap::new();
        let mut lights = BTreeMap::<String, WorldLight>::new();
        let mut dependencies = Vec::new();
        let mut light_records = 0_usize;
        let mut feature_records = 0_usize;
        for coordinate in &footprint.chunks {
            let source = self
                .resident
                .get(coordinate)
                .ok_or_else(|| invalid("missing preflighted source chunk"))?;
            dependencies.push(ChunkRevision {
                coordinate: *coordinate,
                revision: source.revision,
            });
            light_records =
                light_records.saturating_add(source.package.semantics.light_influences.len());
            if light_records > self.config.max_light_records_per_observer {
                return Err(limit("local light record budget exceeded"));
            }
            for light in &source.package.semantics.light_influences {
                if let Some(prior) = lights.insert(light.id.clone(), light.clone()) {
                    if prior != *light {
                        return Err(invalid("resident chunks disagree on an influencing light"));
                    }
                }
                if lights.len() > self.config.max_lights_per_observer {
                    return Err(limit("local distinct light budget exceeded"));
                }
            }
            feature_records = feature_records.saturating_add(source.package.features.len());
            if feature_records > self.config.max_landmarks_per_observer {
                return Err(limit("local landmark record budget exceeded"));
            }
            for feature in &source.package.features {
                if request
                    .position
                    .column
                    .checked_distance(feature.anchor.column)?
                    <= u64::from(footprint.radius)
                {
                    // Compiler feature anchors name the supporting ground while
                    // object origins start above it. Match root column, not level.
                    let present = feature.asset.as_ref().is_none_or(|asset| {
                        source
                            .package
                            .semantics
                            .objects
                            .binary_search_by(|object| object.id.cmp(&feature.id))
                            .ok()
                            .and_then(|index| source.package.semantics.objects.get(index))
                            .is_some_and(|object| {
                                object.asset == *asset
                                    && object.region_id == feature.region_id
                                    && object.origin.column == feature.anchor.column
                            })
                    });
                    if !present && !landmark_memory.contains_key(&feature.id) {
                        continue;
                    }
                    let local = local_position(feature.anchor, request.position)?;
                    targets.insert(
                        local,
                        (
                            feature.anchor,
                            source.domain(feature.anchor).map(str::to_owned),
                        ),
                    );
                    if present {
                        landmarks.insert(
                            feature.id.clone(),
                            (local, feature.clone(), source.revision),
                        );
                    } else {
                        absent_landmarks.insert(
                            feature.id.clone(),
                            (
                                local,
                                InvalidatedLandmark {
                                    id: feature.id.clone(),
                                    position: feature.anchor,
                                    world_revision: source.revision,
                                },
                            ),
                        );
                    }
                }
            }
        }
        for column in &footprint.columns {
            let Some(global) = column.global else {
                // Outside-world availability is opaque only to LOS; never publish this as terrain.
                object_runs.push(AuthoredObjectVoxelRun {
                    top: TilePos::new(column.local, i32::MAX),
                    bottom: i32::MIN,
                });
                if terrain_runs.len().saturating_add(object_runs.len())
                    > self.config.max_runs_per_observer
                {
                    return Err(limit("local occupancy run budget exceeded"));
                }
                continue;
            };
            let source = self
                .resident
                .get(&global.chunk())
                .ok_or_else(|| invalid("missing source column owner"))?;
            let terrain = source
                .package
                .columns
                .binary_search_by_key(&global, |column| column.position)
                .ok()
                .and_then(|index| source.package.columns.get(index))
                .ok_or_else(|| invalid("resident source lacks declared column"))?;
            for run in &terrain.runs {
                if !self.index.material(&run.material)?.solid {
                    continue;
                }
                if let Some((bottom, top)) =
                    local_interval(run.bottom, run.top, request.position.level)
                {
                    terrain_runs.push((TilePos::new(column.local, top), RunBottom(bottom)));
                }
            }
            if let Some(occupancy) = source
                .package
                .semantics
                .occupancy
                .binary_search_by_key(&global, |column| column.position)
                .ok()
                .and_then(|index| source.package.semantics.occupancy.get(index))
            {
                for run in &occupancy.runs {
                    if !self.index.material(&run.material)?.solid {
                        continue;
                    }
                    if let Some((bottom, top)) =
                        local_interval(run.bottom, run.top, request.position.level)
                    {
                        object_runs.push(AuthoredObjectVoxelRun {
                            top: TilePos::new(column.local, top),
                            bottom,
                        });
                    }
                }
            }
            if terrain_runs.len().saturating_add(object_runs.len())
                > self.config.max_runs_per_observer
            {
                return Err(limit("local occupancy run budget exceeded"));
            }
            if request.position.column.checked_distance(global)? > u64::from(footprint.radius) {
                continue;
            }
            let surfaces = match query.surfaces(global) {
                QueryResult::Ready(surfaces) => surfaces,
                QueryResult::Unloaded(coordinate) => {
                    return Ok(ObservationResult::Pending(vec![coordinate]))
                }
                QueryResult::OutsideWorld => {
                    return Err(invalid("query and indexed world coverage disagree"))
                }
            };
            if supports.len().saturating_add(surfaces.len()) > self.config.max_surfaces_per_observer
            {
                return Err(limit("local candidate surface budget exceeded"));
            }
            for surface in surfaces {
                if surface.position.column != global
                    || surface.headroom == Some(0)
                    || !self.index.material(&surface.material)?.solid
                {
                    return Err(invalid("query returned an invalid exposed support"));
                }
                let local = local_position(surface.position, request.position)?;
                targets.insert(
                    local,
                    (
                        surface.position,
                        source.domain(surface.position).map(str::to_owned),
                    ),
                );
                if supports.insert(local, (surface, source.revision)).is_some() {
                    return Err(invalid("duplicate exact support from world query"));
                }
            }
        }
        let observer = TilePos::new(HexCoord::ORIGIN, 0);
        if !supports.contains_key(&observer) {
            return Err(invalid("observer does not occupy an exposed exact support"));
        }
        let mut remembered_targets = BTreeMap::new();
        for position in memory {
            let local = local_position(*position, request.position)?;
            if !supports.contains_key(&local) {
                let source = self
                    .resident
                    .get(&position.column.chunk())
                    .ok_or_else(|| invalid("missing remembered support dependency"))?;
                targets.insert(
                    local,
                    (*position, source.domain(*position).map(str::to_owned)),
                );
                remembered_targets.insert(local, *position);
            }
        }
        let mut domain_names = BTreeSet::new();
        for (_, domain) in targets.values() {
            if let Some(domain) = domain {
                domain_names.insert(domain.clone());
            }
        }
        for light in lights.values() {
            if let Some(domain) = &light.domain {
                domain_names.insert(domain.clone());
            }
        }
        let domains = domain_names
            .into_iter()
            .enumerate()
            .map(|(index, name)| {
                u32::try_from(index)
                    .map(|index| (name, InteriorRegionId(index)))
                    .map_err(limit)
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        let domain = |name: Option<&str>| -> Result<LightDomain, Error> {
            name.map_or(Ok(LightDomain::Exterior), |name| {
                domains
                    .get(name)
                    .copied()
                    .map(LightDomain::Interior)
                    .ok_or_else(|| invalid("missing indexed local light domain"))
            })
        };
        let mut local_lights = BTreeMap::new();
        for light in lights.values() {
            let pos = local_position(light.position, request.position)?;
            let domain = domain(light.domain.as_deref())?;
            local_lights.insert(
                light.id.as_str(),
                [
                    LightSourceSnapshot {
                        pos,
                        domain,
                        light: GameplayLight::new(IlluminationLevel::Bright, light.bright_radius),
                    },
                    LightSourceSnapshot {
                        pos,
                        domain,
                        light: GameplayLight::new(IlluminationLevel::Dim, light.dim_radius),
                    },
                ],
            );
        }
        let mut lights_by_chunk = BTreeMap::new();
        for coordinate in &footprint.chunks {
            let source = self
                .resident
                .get(coordinate)
                .ok_or_else(|| invalid("missing indexed light projection"))?;
            let mut affecting = Vec::new();
            for light in &source.package.semantics.light_influences {
                affecting.extend_from_slice(
                    local_lights
                        .get(light.id.as_str())
                        .ok_or_else(|| invalid("missing local light identity"))?,
                );
            }
            lights_by_chunk.insert(*coordinate, affecting);
        }
        let mut illumination = BTreeMap::new();
        for (local, (global, name)) in &targets {
            let domain = domain(name.as_deref())?;
            let affecting = lights_by_chunk
                .get(&global.column.chunk())
                .ok_or_else(|| invalid("missing target light bucket"))?;
            illumination.insert(
                *local,
                ResolvedLight {
                    domain,
                    level: resolve_illumination_at(*local, domain, request.exterior, affecting),
                },
            );
        }
        let terrain = TerrainOccupancy::from_runs(terrain_runs).map_err(invalid)?;
        let objects = AuthoredObjectOccupancy::from_runs(object_runs).map_err(invalid)?;
        let cache = SightOccupancyCache::try_new(
            &terrain,
            &objects,
            HexCoord::from_axial(-footprint.extent, -footprint.extent),
            HexCoord::from_axial(footprint.extent, footprint.extent),
            self.config.max_column_probes,
        )
        .ok_or_else(|| limit("local exact ray cache budget exceeded"))?;
        let visible = targets
            .keys()
            .copied()
            .filter(|target| {
                illumination.get(target).copied().is_some_and(|light| {
                    crate::sight::can_observe_cached(
                        observer,
                        *target,
                        light,
                        request.profile,
                        &cache,
                    )
                })
            })
            .collect::<BTreeSet<_>>();
        let tested_surfaces = supports.len().saturating_add(remembered_targets.len());
        let mut invalidated_surfaces = remembered_targets
            .into_iter()
            .filter(|(local, _)| visible.contains(local))
            .map(|(_, position)| position)
            .collect::<Vec<_>>();
        invalidated_surfaces.sort_unstable();
        let mut surfaces = supports
            .into_iter()
            .filter(|(local, _)| visible.contains(local))
            .map(|(local, (surface, world_revision))| {
                illumination
                    .get(&local)
                    .map(|light| VisibleSurface {
                        surface,
                        world_revision,
                        illumination: light.level,
                    })
                    .ok_or_else(|| invalid("visible surface lost objective illumination"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        surfaces.sort_by_key(|fact| fact.surface.position);
        let landmarks = landmarks
            .into_values()
            .filter(|(local, _, _)| visible.contains(local))
            .map(|(_, feature, world_revision)| VisibleLandmark {
                feature,
                world_revision,
            })
            .collect();
        let invalidated_landmarks = absent_landmarks
            .into_values()
            .filter(|(local, _)| visible.contains(local))
            .map(|(_, fact)| fact)
            .collect();
        let missing = dependencies
            .iter()
            .filter(|dependency| query.revision(dependency.coordinate) != Some(dependency.revision))
            .map(|dependency| dependency.coordinate)
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Ok(ObservationResult::Pending(missing));
        }
        Ok(ObservationResult::Ready(Arc::new(ObserverFacts {
            observer_id: request.id.clone(),
            principal: request.principal.clone(),
            position: request.position,
            surfaces,
            invalidated_surfaces,
            landmarks,
            invalidated_landmarks,
            dependencies,
            inspected_columns: footprint.columns.len(),
            tested_surfaces,
        })))
    }
    fn invalidate(&mut self, coordinate: ChunkId) {
        let affected = self
            .cached
            .iter()
            .filter(|(_, cached)| {
                cached
                    .facts
                    .dependencies
                    .binary_search_by_key(&coordinate, |dependency| dependency.coordinate)
                    .is_ok()
            })
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in affected {
            self.evict(&id);
        }
    }
    fn evict(&mut self, id: &str) {
        if let Some(cached) = self.cached.remove(id) {
            self.cached_facts = self.cached_facts.saturating_sub(
                cached
                    .facts
                    .surfaces
                    .len()
                    .saturating_add(cached.facts.landmarks.len())
                    .saturating_add(cached.facts.invalidated_landmarks.len())
                    .saturating_add(cached.facts.invalidated_surfaces.len()),
            );
        }
        self.recent.retain(|entry| entry != id);
    }
    fn cache(
        &mut self,
        request: &ObserverRequest,
        memory_fingerprint: u64,
        facts: Arc<ObserverFacts>,
    ) {
        self.evict(&request.id);
        let count = facts
            .surfaces
            .len()
            .saturating_add(facts.landmarks.len())
            .saturating_add(facts.invalidated_landmarks.len())
            .saturating_add(facts.invalidated_surfaces.len());
        if count > self.config.max_cached_facts {
            return;
        }
        while self.cached.len() >= self.config.max_cached_observers
            || self.cached_facts.saturating_add(count) > self.config.max_cached_facts
        {
            let Some(id) = self.recent.front().cloned() else {
                break;
            };
            self.evict(&id);
        }
        self.cached_facts += count;
        self.recent.push_back(request.id.clone());
        self.cached.insert(
            request.id.clone(),
            CachedObserver {
                request: request.clone(),
                memory_fingerprint,
                facts,
            },
        );
    }
}

fn radius(profile: SightProfile) -> u32 {
    profile
        .bright
        .radius
        .max(profile.dim.radius)
        .max(profile.dark.radius)
}
fn local_position(position: VoxelPosition, origin: VoxelPosition) -> Result<TilePos, Error> {
    let q = i128::from(position.column.q) - i128::from(origin.column.q);
    let r = i128::from(position.column.r) - i128::from(origin.column.r);
    let level = i64::from(position.level) - i64::from(origin.level);
    // Keep a margin for standing eye/body corners and observer-relative cover bands.
    if level <= i64::from(i32::MIN) + 4 || level >= i64::from(i32::MAX) - 4 {
        return Err(limit(
            "vertical target range cannot be represented in the exact local sight frame",
        ));
    }
    Ok(TilePos::new(
        HexCoord::from_axial(
            i32::try_from(q).map_err(limit)?,
            i32::try_from(r).map_err(limit)?,
        ),
        i32::try_from(level).map_err(limit)?,
    ))
}
fn local_interval(bottom: i32, top: i32, origin: i32) -> Option<(i32, i32)> {
    let bottom = (i64::from(bottom) - i64::from(origin)).max(i64::from(i32::MIN));
    let top = (i64::from(top) - 1 - i64::from(origin)).min(i64::from(i32::MAX));
    if bottom > top {
        return None;
    }
    Some((i32::try_from(bottom).ok()?, i32::try_from(top).ok()?))
}

#[cfg(test)]
mod tests;
