//! Pure command planning for exact authored-object removal from a resident fragment.
//!
//! The composition root supplies current and next-waypoint actor body volumes,
//! pumps residency, applies a ready transaction with owner attachments, and releases
//! the operation pin. This module owns no gameplay clock, session, UI, or Bevy state.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use hex_world_contracts::{
    union_object_occupancy, ChunkId, ColumnData, ObjectEdit, ObjectInfluence, ObjectInstance,
    VoxelPosition, WorldHex, WorldObjectEditTransaction, WorldQuery,
};
use hex_world_runtime::{RuntimeError, WorldRuntime};

/// Exact source identity observed by a resident chunk's authoritative picking path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ObjectSelection {
    pub object_id: String,
    pub chunk: ChunkId,
    pub revision: u64,
    pub source_fingerprint: u64,
}

/// Every exact object contribution at a clicked voxel, in stable distinct ID order.
/// Stock-art and proxy picking use this same authoritative resident lookup.
pub(super) fn selections_at(
    runtime: &WorldRuntime,
    position: VoxelPosition,
    expected_revision: u64,
) -> Result<Vec<ObjectSelection>, RemovalError> {
    let coordinate = position.column.chunk();
    let product = runtime
        .resident_chunk(coordinate)
        .ok_or(RemovalError::Stale(coordinate))?;
    if product.revision != expected_revision {
        return Err(RemovalError::Stale(coordinate));
    }
    Ok(product
        .package
        .semantics
        .object_influences
        .iter()
        .filter(|influence| {
            column(&influence.occupancy, position.column)
                .is_some_and(|column| column.material_at(position.level).is_some())
        })
        .map(|influence| ObjectSelection {
            object_id: influence.id.clone(),
            chunk: coordinate,
            revision: product.revision,
            source_fingerprint: influence.source_fingerprint,
        })
        .collect())
}

/// One exact current or next-waypoint body volume supplied by the actor owner.
#[derive(Clone, Debug)]
pub(super) struct ActorVolume {
    pub actor_id: String,
    pub support: VoxelPosition,
    pub levels_tall: u32,
}

/// Operational planning bounds; these do not restrict world or authored-object size.
#[derive(Clone, Copy, Debug)]
pub(super) struct RemovalLimits {
    pub max_dependency_chunks: usize,
    pub max_footprint_columns: usize,
    pub max_actor_volumes: usize,
    pub max_body_levels: u32,
}
impl Default for RemovalLimits {
    fn default() -> Self {
        Self {
            max_dependency_chunks: 256,
            max_footprint_columns: 4096,
            max_actor_volumes: 256,
            max_body_levels: 64,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum RemovalError {
    Invalid(String),
    Stale(ChunkId),
    ActorProtected {
        actor_id: String,
        position: VoxelPosition,
        reason: &'static str,
    },
    Runtime(RuntimeError),
    Cancelled,
}
impl fmt::Display for RemovalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => write!(formatter, "object removal: {message}"),
            Self::Stale(chunk) => write!(
                formatter,
                "object removal observed a stale dependency {chunk:?}"
            ),
            Self::ActorProtected {
                actor_id,
                position,
                reason,
            } => write!(
                formatter,
                "object removal protects {actor_id} at {position:?}: {reason}"
            ),
            Self::Runtime(error) => error.fmt(formatter),
            Self::Cancelled => formatter.write_str("object removal was cancelled"),
        }
    }
}
impl std::error::Error for RemovalError {}
impl From<RuntimeError> for RemovalError {
    fn from(error: RuntimeError) -> Self {
        Self::Runtime(error)
    }
}
fn invalid(message: impl fmt::Display) -> RemovalError {
    RemovalError::Invalid(message.to_string())
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum RemovalStatus {
    Pending(Vec<ChunkId>),
    Ready(WorldObjectEditTransaction),
}

/// One command intent, fixed source identity, retained revisions, and an owned pin.
///
/// Transaction IDs must be unique among active command intents. Call `cancel` on
/// every terminal path, including after applying Ready; dropping this value cannot
/// release runtime pins because it does not retain a mutable runtime borrow.
#[derive(Debug)]
pub(super) struct ObjectRemoval {
    transaction_id: String,
    pin_owner: String,
    selection: ObjectSelection,
    clicked_influence: ObjectInfluence,
    manifest_fingerprint: u64,
    world_id: String,
    limits: RemovalLimits,
    dependencies: BTreeSet<ChunkId>,
    expected_revisions: BTreeMap<ChunkId, u64>,
    source: Option<ObjectInstance>,
    projections: BTreeMap<ChunkId, ObjectInfluence>,
    cancelled: bool,
}

impl ObjectRemoval {
    /// Validate the clicked source and pin its known clicked/root dependencies.
    /// A failure during begin releases any pin created by this call.
    pub(super) fn begin(
        runtime: &mut WorldRuntime,
        transaction_id: String,
        selection: ObjectSelection,
        limits: RemovalLimits,
    ) -> Result<Self, RemovalError> {
        if transaction_id.is_empty()
            || transaction_id.len() > 100
            || transaction_id.trim() != transaction_id
            || transaction_id.chars().any(char::is_control)
            || limits.max_dependency_chunks == 0
            || limits.max_footprint_columns == 0
            || limits.max_actor_volumes == 0
            || limits.max_body_levels == 0
        {
            return Err(invalid(
                "invalid bounded transaction identity or planning limits",
            ));
        }
        let product = runtime
            .resident_chunk(selection.chunk)
            .ok_or(RemovalError::Stale(selection.chunk))?;
        if product.revision != selection.revision {
            return Err(RemovalError::Stale(selection.chunk));
        }
        let influence = product
            .package
            .semantics
            .object_influences
            .binary_search_by(|row| row.id.cmp(&selection.object_id))
            .ok()
            .and_then(|index| product.package.semantics.object_influences.get(index))
            .filter(|row| row.source_fingerprint == selection.source_fingerprint)
            .cloned()
            .ok_or(RemovalError::Stale(selection.chunk))?;
        let dependencies = BTreeSet::from([selection.chunk, influence.origin.column.chunk()]);
        if dependencies.len() > limits.max_dependency_chunks {
            return Err(invalid("object dependency limit exceeded"));
        }
        let pin_owner = format!("v4/remove/{transaction_id}");
        runtime.pin(pin_owner.clone(), dependencies.clone())?;
        let mut plan = Self {
            transaction_id,
            pin_owner,
            expected_revisions: BTreeMap::from([(selection.chunk, selection.revision)]),
            selection,
            clicked_influence: influence,
            manifest_fingerprint: runtime.manifest().fingerprint,
            world_id: runtime.manifest().world_id.clone(),
            limits,
            dependencies,
            source: None,
            projections: BTreeMap::new(),
            cancelled: false,
        };
        if let Err(error) = plan.discover(runtime) {
            plan.cancel(runtime)?;
            return Err(error);
        }
        Ok(plan)
    }

    /// Preserve observed revisions, finish loading, and protect current actor volumes.
    /// Ready is a proposal; runtime application rechecks the complete transaction.
    /// Pins remain held on Ready or error until `cancel` is called.
    pub(super) fn poll(
        &mut self,
        runtime: &mut WorldRuntime,
        actors: &[ActorVolume],
    ) -> Result<RemovalStatus, RemovalError> {
        if self.cancelled {
            return Err(RemovalError::Cancelled);
        }
        if runtime.manifest().fingerprint != self.manifest_fingerprint
            || runtime.manifest().world_id != self.world_id
        {
            return Err(invalid("world source changed during object removal"));
        }
        if actors.len() > self.limits.max_actor_volumes {
            return Err(invalid("actor protection volume limit exceeded"));
        }
        for actor in actors {
            if actor.actor_id.is_empty()
                || actor.actor_id.len() > 128
                || actor.actor_id.trim() != actor.actor_id
                || actor.actor_id.chars().any(char::is_control)
                || actor.levels_tall == 0
                || actor.levels_tall > self.limits.max_body_levels
                || i64::from(actor.support.level) + i64::from(actor.levels_tall)
                    > i64::from(i32::MAX)
            {
                return Err(invalid(
                    "invalid or unrepresentable actor protection volume",
                ));
            }
        }
        self.discover(runtime)?;
        let mut pending = Vec::new();
        for coordinate in &self.dependencies {
            let Some(product) = runtime.resident_chunk(*coordinate) else {
                pending.push(*coordinate);
                continue;
            };
            match self.expected_revisions.get(coordinate) {
                Some(revision) if *revision != product.revision => {
                    return Err(RemovalError::Stale(*coordinate))
                }
                None => {
                    self.expected_revisions
                        .insert(*coordinate, product.revision);
                }
                Some(_) => {}
            }
            if self.source.is_some() {
                let actual = product
                    .package
                    .semantics
                    .object_influences
                    .binary_search_by(|row| row.id.cmp(&self.selection.object_id))
                    .ok()
                    .and_then(|index| product.package.semantics.object_influences.get(index));
                if actual != self.projections.get(coordinate) {
                    return Err(RemovalError::Stale(*coordinate));
                }
            }
        }
        if !pending.is_empty() {
            return Ok(RemovalStatus::Pending(pending));
        }
        let source = self
            .source
            .as_ref()
            .ok_or_else(|| invalid("loaded root lacks complete source record"))?;
        self.protect_actors(runtime, actors)?;
        let transaction = WorldObjectEditTransaction {
            id: self.transaction_id.clone(),
            expected_revisions: self.expected_revisions.clone(),
            edits: vec![ObjectEdit {
                before: Some(source.clone()),
                after: None,
            }],
        };
        transaction.validate().map_err(invalid)?;
        Ok(RemovalStatus::Ready(transaction))
    }

    /// Release exactly this operation's pin, on cancellation or after successful apply.
    pub(super) fn cancel(&mut self, runtime: &mut WorldRuntime) -> Result<(), RemovalError> {
        if !self.cancelled {
            runtime.unpin(&self.pin_owner)?;
            self.cancelled = true;
        }
        Ok(())
    }

    fn discover(&mut self, runtime: &mut WorldRuntime) -> Result<(), RemovalError> {
        for coordinate in &self.dependencies {
            if let Some(revision) = runtime.revision(*coordinate) {
                if self
                    .expected_revisions
                    .get(coordinate)
                    .is_some_and(|expected| *expected != revision)
                {
                    return Err(RemovalError::Stale(*coordinate));
                }
                self.expected_revisions
                    .entry(*coordinate)
                    .or_insert(revision);
            }
        }
        if self.source.is_some() {
            return Ok(());
        }
        let owner = self.clicked_influence.origin.column.chunk();
        let Some(product) = runtime.resident_chunk(owner) else {
            return Ok(());
        };
        let source = product
            .package
            .semantics
            .objects
            .binary_search_by(|row| row.id.cmp(&self.selection.object_id))
            .ok()
            .and_then(|index| product.package.semantics.objects.get(index))
            .ok_or(RemovalError::Stale(owner))?;
        if source.occupancy.len() > self.limits.max_footprint_columns {
            return Err(invalid("object footprint limit exceeded"));
        }
        let projections = source.influences().map_err(invalid)?;
        if projections.get(&self.selection.chunk) != Some(&self.clicked_influence) {
            return Err(RemovalError::Stale(owner));
        }
        let dependencies = projections.keys().copied().collect::<BTreeSet<_>>();
        if dependencies.len() > self.limits.max_dependency_chunks {
            return Err(invalid("object dependency limit exceeded"));
        }
        runtime.pin(self.pin_owner.clone(), dependencies.clone())?;
        for coordinate in &dependencies {
            if let Some(revision) = runtime.revision(*coordinate) {
                if self
                    .expected_revisions
                    .get(coordinate)
                    .is_some_and(|expected| *expected != revision)
                {
                    return Err(RemovalError::Stale(*coordinate));
                }
                self.expected_revisions
                    .entry(*coordinate)
                    .or_insert(revision);
            }
        }
        self.dependencies = dependencies;
        self.source = Some(source.clone());
        self.projections = projections;
        Ok(())
    }

    fn protect_actors(
        &self,
        runtime: &WorldRuntime,
        actors: &[ActorVolume],
    ) -> Result<(), RemovalError> {
        let mut remaining: BTreeMap<ChunkId, Vec<ColumnData>> = BTreeMap::new();
        for actor in actors {
            let coordinate = actor.support.column.chunk();
            let Some(influence) = self.projections.get(&coordinate) else {
                continue;
            };
            if influence
                .occupancy
                .binary_search_by_key(&actor.support.column, |column| column.position)
                .is_err()
            {
                continue;
            }
            let product = runtime
                .resident_chunk(coordinate)
                .ok_or(RemovalError::Stale(coordinate))?;
            if let std::collections::btree_map::Entry::Vacant(entry) = remaining.entry(coordinate) {
                let survivors = product
                    .package
                    .semantics
                    .object_influences
                    .iter()
                    .filter(|row| row.id != self.selection.object_id)
                    .cloned()
                    .collect::<Vec<_>>();
                entry.insert(union_object_occupancy(&survivors).map_err(invalid)?);
            }
            let projected = remaining
                .get(&coordinate)
                .ok_or_else(|| invalid("missing checked survivor projection"))?;
            let objects = column(projected, actor.support.column);
            let terrain = column(&product.package.columns, actor.support.column)
                .ok_or_else(|| invalid("actor support outside checked source footprint"))?;
            let material_at = |level| {
                objects
                    .and_then(|column| column.material_at(level))
                    .or_else(|| terrain.material_at(level))
            };
            let solid = material_at(actor.support.level)
                .map(|id| runtime.manifest().material(id))
                .transpose()
                .map_err(invalid)?
                .is_some_and(|material| material.solid);
            if !solid {
                return Err(RemovalError::ActorProtected {
                    actor_id: actor.actor_id.clone(),
                    position: actor.support,
                    reason: "exact support would disappear",
                });
            }
            for offset in 1..=actor.levels_tall {
                let level = i32::try_from(i64::from(actor.support.level) + i64::from(offset))
                    .map_err(invalid)?;
                if material_at(level).is_some() {
                    return Err(RemovalError::ActorProtected {
                        actor_id: actor.actor_id.clone(),
                        position: VoxelPosition {
                            column: actor.support.column,
                            level,
                        },
                        reason: "required body clearance would be occupied",
                    });
                }
            }
        }
        Ok(())
    }
}

fn column(columns: &[ColumnData], position: WorldHex) -> Option<&ColumnData> {
    columns
        .binary_search_by_key(&position, |column| column.position)
        .ok()
        .and_then(|index| columns.get(index))
}

#[cfg(test)]
mod tests {

    use super::*;
    use hex_world_contracts::{
        ChunkDescriptor, ChunkPackage, ChunkSemantics, MaterialSpec, ObjectEdit, QueryResult,
        RegionDescriptor, ResidencyRequest, VoxelRun, WorldManifest, WorldPackage, SCHEMA_VERSION,
    };
    use hex_world_runtime::{ErrorKind, MemoryChunkSource, RuntimeConfig};
    use std::{
        sync::Arc,
        thread,
        time::{Duration, Instant},
    };

    fn at(q: i64, level: i32) -> VoxelPosition {
        VoxelPosition {
            column: WorldHex::new(q, 0),
            level,
        }
    }
    fn run(bottom: i32, top: i32) -> VoxelRun {
        VoxelRun {
            bottom,
            top,
            material: "stone".into(),
        }
    }
    fn fixture(overlap: bool) -> WorldRuntime {
        let mut chunks = BTreeMap::new();
        let mut regions = Vec::new();
        for (index, q) in [0, 16, 32, 1000].into_iter().enumerate() {
            let position = at(q, 0).column;
            regions.push(RegionDescriptor {
                id: format!("region-{index}"),
                origin: position,
                radius: 0,
                source_fingerprint: 1,
            });
            chunks.insert(
                position.chunk(),
                ChunkPackage {
                    schema_version: SCHEMA_VERSION,
                    world_id: "planner-world".into(),
                    coordinate: position.chunk(),
                    source_fingerprint: 1,
                    columns: vec![ColumnData {
                        position,
                        runs: vec![run(-2, 1)],
                    }],
                    features: Vec::new(),
                    semantics: ChunkSemantics::default(),
                    fingerprint: 0,
                },
            );
        }
        let a = ObjectInstance {
            id: "a-tree".into(),
            region_id: "region-0".into(),
            asset: "tree".into(),
            origin: at(0, 1),
            rotation: 0,
            occupancy: [0, 16, 32]
                .into_iter()
                .map(|q| ColumnData {
                    position: at(q, 0).column,
                    runs: vec![run(1, 3)],
                })
                .collect(),
        };
        chunks
            .get_mut(&at(0, 0).column.chunk())
            .expect("root")
            .semantics
            .objects
            .push(a);
        if overlap {
            let b = ObjectInstance {
                id: "b-tree".into(),
                region_id: "region-3".into(),
                asset: "tree".into(),
                origin: at(1000, 1),
                rotation: 0,
                occupancy: vec![ColumnData {
                    position: at(16, 0).column,
                    runs: vec![run(2, 3)],
                }],
            };
            chunks
                .get_mut(&at(1000, 0).column.chunk())
                .expect("remote root")
                .semantics
                .objects
                .push(b);
        }
        let descriptors = chunks
            .keys()
            .map(|coordinate| ChunkDescriptor {
                coordinate: *coordinate,
                fingerprint: 0,
                path: format!("chunks/{}_{}.ron", coordinate.q, coordinate.r),
            })
            .collect();
        let mut package = WorldPackage {
            manifest: WorldManifest {
                schema_version: SCHEMA_VERSION,
                world_id: "planner-world".into(),
                compiler_version: "planner-fixture".into(),
                source_fingerprint: 1,
                materials: vec![MaterialSpec {
                    id: "stone".into(),
                    solid: true,
                    diggable: true,
                    color: [128, 128, 128, 255],
                }],
                regions,
                chunks: descriptors,
                boundaries: Vec::new(),
                summary: Vec::new(),
                features: Vec::new(),
                fingerprint: 0,
            },
            chunks,
        };
        package.seal().expect("valid cross-chunk source");
        WorldRuntime::new(
            Arc::new(MemoryChunkSource::new(package).expect("source")),
            RuntimeConfig::default(),
        )
        .expect("runtime")
    }
    fn interest(q: i64) -> ResidencyRequest {
        ResidencyRequest {
            id: format!("actor-{q}"),
            center: at(q, 0).column,
            radius: 0,
            retention_radius: 0,
            priority: 1,
        }
    }
    fn settle(runtime: &mut WorldRuntime) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let update = runtime.pump();
            assert!(
                update.failures.is_empty(),
                "unexpected source failure: {:?}",
                update.failures
            );
            let counts = runtime.counts();
            if counts.in_flight_jobs == 0 && counts.queued_chunks == 0 {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "fixture residency did not settle"
            );
            thread::sleep(Duration::from_millis(1));
        }
    }
    fn clicked(runtime: &mut WorldRuntime) -> ObjectSelection {
        runtime
            .set_interests(vec![interest(16)])
            .expect("clicked fragment interest");
        settle(runtime);
        selections_at(runtime, at(16, 2), 0)
            .expect("exact selection")
            .into_iter()
            .find(|row| row.object_id == "a-tree")
            .expect("clicked object")
    }
    fn ready(
        plan: &mut ObjectRemoval,
        runtime: &mut WorldRuntime,
        actors: &[ActorVolume],
    ) -> WorldObjectEditTransaction {
        let mut result = None;
        for _ in 0..4 {
            match plan.poll(runtime, actors).expect("poll") {
                RemovalStatus::Ready(transaction) => {
                    result = Some(transaction);
                    break;
                }
                RemovalStatus::Pending(_) => settle(runtime),
            }
        }
        result.expect("bounded fixture did not become ready")
    }

    #[test]
    fn selection_is_exact_stack_local_stable_and_rejects_stale_clicks() {
        let mut runtime = fixture(true);
        clicked(&mut runtime);
        let overlapping = selections_at(&runtime, at(16, 2), 0).expect("shared voxel");
        assert_eq!(
            overlapping
                .iter()
                .map(|row| row.object_id.as_str())
                .collect::<Vec<_>>(),
            vec!["a-tree", "b-tree"]
        );
        assert_eq!(
            selections_at(&runtime, at(16, 1), 0)
                .expect("lower voxel")
                .len(),
            1
        );
        assert!(selections_at(&runtime, at(16, 3), 0)
            .expect("air")
            .is_empty());
        assert_eq!(
            selections_at(&runtime, at(16, 2), 1).expect_err("old click"),
            RemovalError::Stale(at(16, 0).column.chunk())
        );
        assert_eq!(
            selections_at(&runtime, at(0, 2), 0).expect_err("unloaded root"),
            RemovalError::Stale(at(0, 0).column.chunk())
        );
    }

    #[test]
    fn foreign_selection_discovers_and_pins_full_root_footprint_before_ready() {
        let mut runtime = fixture(true);
        let selection = clicked(&mut runtime);
        assert!(runtime.resident_chunk(at(0, 0).column.chunk()).is_none());
        let mut plan = ObjectRemoval::begin(
            &mut runtime,
            "remove-foreign".into(),
            selection,
            RemovalLimits::default(),
        )
        .expect("begin");
        assert_eq!(runtime.counts().pinned_chunks, 2);
        assert_eq!(
            plan.poll(&mut runtime, &[]).expect("root pending"),
            RemovalStatus::Pending(vec![at(0, 0).column.chunk()])
        );
        settle(&mut runtime);
        assert_eq!(
            plan.poll(&mut runtime, &[]).expect("tail discovered"),
            RemovalStatus::Pending(vec![at(32, 0).column.chunk()])
        );
        assert_eq!(runtime.counts().pinned_chunks, 3);
        settle(&mut runtime);
        let actor = ActorVolume {
            actor_id: "standing-on-overlap".into(),
            support: at(16, 2),
            levels_tall: 2,
        };
        let transaction = ready(&mut plan, &mut runtime, &[actor]);
        assert_eq!(
            transaction.expected_revisions,
            [0, 16, 32]
                .into_iter()
                .map(|q| (at(q, 0).column.chunk(), 0))
                .collect()
        );
        assert!(runtime.resident_chunk(at(1000, 0).column.chunk()).is_none());
        runtime
            .apply_object_transaction(&transaction)
            .expect("remove selected identity only");
        assert_eq!(
            runtime.voxel(at(16, 2)),
            QueryResult::Ready(Some("stone".into()))
        );
        assert_eq!(runtime.voxel(at(16, 1)), QueryResult::Ready(None));
        assert_eq!(runtime.voxel(at(32, 2)), QueryResult::Ready(None));
        plan.cancel(&mut runtime)
            .expect("release after application");
        assert_eq!(runtime.counts().pinned_chunks, 0);
    }

    #[test]
    fn current_and_next_waypoint_support_and_complete_body_aperture_are_protected() {
        let mut runtime = fixture(true);
        let selection = clicked(&mut runtime);
        let mut plan = ObjectRemoval::begin(
            &mut runtime,
            "protect-actor".into(),
            selection,
            RemovalLimits::default(),
        )
        .expect("begin");
        ready(&mut plan, &mut runtime, &[]);
        let standing = ActorVolume {
            actor_id: "party-current".into(),
            support: at(16, 2),
            levels_tall: 2,
        };
        let next = ActorVolume {
            actor_id: "party-next-waypoint".into(),
            support: at(32, 2),
            levels_tall: 2,
        };
        let before = runtime
            .resident_chunks()
            .map(|row| (row.coordinate, row.revision, row.package.fingerprint))
            .collect::<Vec<_>>();
        assert!(
            matches!(plan.poll(&mut runtime, &[standing, next]), Err(RemovalError::ActorProtected { actor_id, reason: "exact support would disappear", .. }) if actor_id == "party-next-waypoint")
        );
        let short = ActorVolume {
            actor_id: "short-body".into(),
            support: at(16, 0),
            levels_tall: 1,
        };
        assert!(matches!(
            plan.poll(&mut runtime, std::slice::from_ref(&short)),
            Ok(RemovalStatus::Ready(_))
        ));
        let tall = ActorVolume {
            levels_tall: 2,
            ..short
        };
        assert!(
            matches!(plan.poll(&mut runtime, &[tall]), Err(RemovalError::ActorProtected { position, reason: "required body clearance would be occupied", .. }) if position == at(16, 2))
        );
        assert_eq!(
            runtime
                .resident_chunks()
                .map(|row| (row.coordinate, row.revision, row.package.fingerprint))
                .collect::<Vec<_>>(),
            before
        );
        plan.cancel(&mut runtime).expect("release rejected command");
    }

    #[test]
    fn observed_dependency_revisions_stay_fixed_and_runtime_rejects_ready_command_if_stale() {
        let mut runtime = fixture(false);
        let selection = clicked(&mut runtime);
        let mut plan = ObjectRemoval::begin(
            &mut runtime,
            "remove-stale".into(),
            selection,
            RemovalLimits::default(),
        )
        .expect("begin");
        let proposed = ready(&mut plan, &mut runtime, &[]);
        let before = proposed
            .edits
            .first()
            .expect("edit")
            .before
            .clone()
            .expect("source");
        let mut after = before.clone();
        after.asset = "replacement-art".into();
        let replacement = WorldObjectEditTransaction {
            id: "another-command".into(),
            expected_revisions: proposed.expected_revisions.clone(),
            edits: vec![ObjectEdit {
                before: Some(before),
                after: Some(after),
            }],
        };
        runtime
            .apply_object_transaction(&replacement)
            .expect("intervening command");
        assert!(matches!(
            plan.poll(&mut runtime, &[]),
            Err(RemovalError::Stale(_))
        ));
        assert_eq!(
            runtime
                .apply_object_transaction(&proposed)
                .expect_err("stale handed-off transaction")
                .kind,
            ErrorKind::Conflict
        );
        plan.cancel(&mut runtime).expect("release stale command");
    }

    #[test]
    fn cancellation_releases_only_operation_pins_and_rejects_future_polling() {
        let mut runtime = fixture(false);
        let selection = clicked(&mut runtime);
        let route_chunk = at(32, 0).column.chunk();
        runtime
            .pin("route", BTreeSet::from([route_chunk]))
            .expect("independent operation pin");
        let mut plan = ObjectRemoval::begin(
            &mut runtime,
            "cancel-before-load".into(),
            selection,
            RemovalLimits::default(),
        )
        .expect("begin");
        plan.cancel(&mut runtime).expect("cancel");
        plan.cancel(&mut runtime).expect("idempotent release");
        assert_eq!(runtime.counts().pinned_chunks, 1);
        assert_eq!(
            plan.poll(&mut runtime, &[]).expect_err("cancelled intent"),
            RemovalError::Cancelled
        );
        settle(&mut runtime);
        assert!(runtime.resident_chunk(route_chunk).is_some());
        assert!(runtime.resident_chunk(at(0, 0).column.chunk()).is_none());
    }

    #[test]
    fn invalid_selection_or_discovered_footprint_limit_never_leaks_operation_pins() {
        let mut runtime = fixture(false);
        let mut selection = clicked(&mut runtime);
        selection.source_fingerprint ^= 1;
        assert!(matches!(
            ObjectRemoval::begin(
                &mut runtime,
                "invalid-selection".into(),
                selection,
                RemovalLimits::default()
            ),
            Err(RemovalError::Stale(_))
        ));
        assert_eq!(runtime.counts().pinned_chunks, 0);
        runtime
            .set_interests(vec![interest(0), interest(16)])
            .expect("known root");
        settle(&mut runtime);
        let selection = selections_at(&runtime, at(16, 2), 0)
            .expect("selection")
            .into_iter()
            .next()
            .expect("object");
        let limits = RemovalLimits {
            max_dependency_chunks: 2,
            ..RemovalLimits::default()
        };
        assert!(matches!(
            ObjectRemoval::begin(
                &mut runtime,
                "limit-after-discovery".into(),
                selection,
                limits
            ),
            Err(RemovalError::Invalid(_))
        ));
        assert_eq!(runtime.counts().pinned_chunks, 0);
    }
}
