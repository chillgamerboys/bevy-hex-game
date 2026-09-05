//! Black-box streamed authority, partition persistence and failure atomicity tests.
#![expect(
    clippy::expect_used,
    reason = "Tests use explicit fixture and assertion expectations."
)]

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use hex_world_contracts::*;
use hex_world_runtime::*;

static TEMP_SEQUENCE: AtomicUsize = AtomicUsize::new(1);

struct TempRoot(PathBuf);
impl TempRoot {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "hex-v4-runtime-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("unique temporary test directory");
        Self(path)
    }
    fn child(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for TempRoot {
    fn drop(&mut self) {
        let _removed = fs::remove_dir_all(&self.0);
    }
}

fn run(bottom: i32, top: i32, material: &str) -> VoxelRun {
    VoxelRun {
        bottom,
        top,
        material: material.into(),
    }
}
fn point(q: i64, r: i64) -> WorldHex {
    WorldHex::new(q, r)
}
fn voxel(column: WorldHex, level: i32) -> VoxelPosition {
    VoxelPosition { column, level }
}

fn world(regions: &[(WorldHex, u32)]) -> WorldPackage {
    let mut chunks: BTreeMap<ChunkId, ChunkPackage> = BTreeMap::new();
    let mut region_descriptors = Vec::new();
    for (index, (center, radius)) in regions.iter().enumerate() {
        region_descriptors.push(RegionDescriptor {
            id: format!("region-{index:04}"),
            origin: *center,
            radius: *radius,
            source_fingerprint: 1,
        });
        let radius = i64::from(*radius);
        for q in -radius..=radius {
            for r in -radius..=radius {
                if q.abs().max(r.abs()).max((q + r).abs()) > radius {
                    continue;
                }
                let position = center.checked_add(point(q, r)).expect("bounded fixture");
                chunks
                    .entry(position.chunk())
                    .or_insert_with(|| ChunkPackage {
                        schema_version: SCHEMA_VERSION,
                        world_id: "test-world".into(),
                        coordinate: position.chunk(),
                        source_fingerprint: 1,
                        columns: Vec::new(),
                        features: Vec::new(),
                        semantics: ChunkSemantics::default(),
                        fingerprint: 0,
                    })
                    .columns
                    .push(ColumnData {
                        position,
                        runs: vec![
                            run(-4, -3, "bedrock"),
                            run(-3, 1, "stone"),
                            run(5, 7, "stone"),
                        ],
                    });
            }
        }
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
            world_id: "test-world".into(),
            compiler_version: "test-1".into(),
            source_fingerprint: 1,
            materials: vec![
                MaterialSpec {
                    id: "bedrock".into(),
                    solid: true,
                    diggable: false,
                    color: [1, 1, 1, 255],
                },
                MaterialSpec {
                    id: "stone".into(),
                    solid: true,
                    diggable: true,
                    color: [128, 128, 128, 255],
                },
                MaterialSpec {
                    id: "water".into(),
                    solid: false,
                    diggable: false,
                    color: [0, 0, 255, 255],
                },
            ],
            regions: region_descriptors,
            chunks: descriptors,
            boundaries: Vec::new(),
            summary: Vec::new(),
            features: Vec::new(),
            fingerprint: 0,
        },
        chunks,
    };
    package.seal().expect("valid world fixture");
    package
}

fn make_runtime(package: WorldPackage) -> WorldRuntime {
    WorldRuntime::new(
        Arc::new(MemoryChunkSource::new(package).expect("source")),
        RuntimeConfig::default(),
    )
    .expect("runtime")
}
fn interest(id: &str, center: WorldHex, radius: u32, retention_radius: u32) -> ResidencyRequest {
    ResidencyRequest {
        id: id.into(),
        center,
        radius,
        retention_radius,
        priority: 1,
    }
}
fn settle(runtime: &mut WorldRuntime) -> Vec<ChunkProduct> {
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut published = Vec::new();
    loop {
        let update = runtime.pump();
        assert!(
            update.failures.is_empty(),
            "unexpected load failures: {:?}",
            update.failures
        );
        published.extend(update.loaded);
        published.extend(update.changed);
        let counts = runtime.counts();
        if counts.queued_chunks == 0 && counts.in_flight_jobs == 0 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "background jobs failed to settle"
        );
        thread::sleep(Duration::from_millis(1));
    }
    published
}
fn load(runtime: &mut WorldRuntime, interests: Vec<ResidencyRequest>) {
    runtime.set_interests(interests).expect("interest union");
    let _published = settle(runtime);
}
fn edit(
    id: &str,
    position: VoxelPosition,
    expected: u64,
    material: Option<&str>,
) -> WorldEditTransaction {
    WorldEditTransaction {
        id: id.into(),
        expected_revisions: BTreeMap::from([(position.column.chunk(), expected)]),
        edits: vec![VoxelEdit {
            position,
            material: material.map(str::to_owned),
        }],
    }
}
fn resident_fingerprints(runtime: &WorldRuntime) -> BTreeMap<ChunkId, (u64, u64)> {
    runtime
        .resident_chunks()
        .map(|chunk| {
            (
                chunk.coordinate,
                (chunk.revision, chunk.package.fingerprint),
            )
        })
        .collect()
}

#[test]
fn availability_negative_coordinates_and_every_stack_are_distinct() {
    let at = point(-1, -1);
    let mut runtime = make_runtime(world(&[(at, 1)]));
    assert_eq!(
        runtime.voxel(voxel(at, 2)),
        QueryResult::Unloaded(ChunkId { q: -1, r: -1 })
    );
    assert_eq!(
        runtime.voxel(voxel(point(900, 900), 2)),
        QueryResult::OutsideWorld
    );
    load(&mut runtime, vec![interest("actor", at, 0, 0)]);
    assert_eq!(runtime.voxel(voxel(at, 2)), QueryResult::Ready(None));
    assert_eq!(
        runtime.voxel(voxel(at, 0)),
        QueryResult::Ready(Some("stone".into()))
    );
    let QueryResult::Ready(surfaces) = runtime.surfaces(at) else {
        panic!("loaded exact column");
    };
    assert_eq!(
        surfaces
            .iter()
            .map(|surface| (surface.position.level, surface.headroom))
            .collect::<Vec<_>>(),
        vec![(0, Some(4)), (6, None)]
    );
    runtime.set_interests(Vec::new()).expect("retire");
    assert_eq!(
        runtime.voxel(voxel(at, 2)),
        QueryResult::Unloaded(at.chunk())
    );
    assert_eq!(runtime.pump().removed, vec![at.chunk()]);
}

#[test]
fn separate_islands_share_global_chunks_and_pins_preserve_union() {
    let a = point(1, 1);
    let shared = point(2, 1);
    let far = point(80, 1);
    let mut runtime = make_runtime(world(&[(a, 0), (shared, 0), (far, 0)]));
    load(
        &mut runtime,
        vec![
            interest("fighter", a, 0, 0),
            interest("nearby", shared, 0, 0),
            interest("explorer", far, 0, 0),
        ],
    );
    assert_eq!(runtime.counts().resident_chunks, 2);
    runtime
        .pin("encounter", BTreeSet::from([a.chunk()]))
        .expect("pin");
    runtime
        .set_interests(vec![interest("explorer", far, 0, 0)])
        .expect("move party");
    assert_eq!(runtime.counts().resident_chunks, 2);
    runtime.unpin("encounter").expect("unpin");
    assert_eq!(runtime.counts().resident_chunks, 1);
    assert_eq!(
        runtime.voxel(voxel(shared, 0)),
        QueryResult::Unloaded(a.chunk())
    );
    assert_eq!(
        runtime.voxel(voxel(far, 0)),
        QueryResult::Ready(Some("stone".into()))
    );
}

#[test]
fn hysteresis_preserves_only_loaded_band_and_over_budget_is_atomic() {
    let a = point(15, 1);
    let b = point(16, 1);
    let far = point(80, 1);
    let package = world(&[(a, 0), (b, 0), (far, 0)]);
    let config = RuntimeConfig {
        max_resident_chunks: 2,
        ..RuntimeConfig::default()
    };
    let mut runtime = WorldRuntime::new(
        Arc::new(MemoryChunkSource::new(package).expect("source")),
        config,
    )
    .expect("runtime");
    load(&mut runtime, vec![interest("a", a, 0, 1)]);
    assert_eq!(
        runtime.counts().resident_chunks,
        1,
        "retention never activates the neighboring chunk"
    );
    load(&mut runtime, vec![interest("a", b, 0, 1)]);
    assert_eq!(
        runtime.counts().resident_chunks,
        2,
        "old resident remains in hysteresis band"
    );
    let before = resident_fingerprints(&runtime);
    assert_eq!(
        runtime
            .set_interests(vec![interest("a", b, 0, 1), interest("b", far, 0, 0)])
            .expect_err("budget")
            .kind,
        ErrorKind::Limit
    );
    assert_eq!(resident_fingerprints(&runtime), before);
    load(&mut runtime, vec![interest("a", far, 0, 0)]);
    assert_eq!(runtime.counts().resident_chunks, 1);
}

#[test]
fn cross_chunk_transactions_are_atomic_local_and_idempotent() {
    let a = point(-1, 0);
    let b = point(16, 0);
    let other = point(80, 0);
    let mut runtime = make_runtime(world(&[(a, 0), (b, 0), (other, 0)]));
    load(
        &mut runtime,
        vec![
            interest("a", a, 0, 0),
            interest("b", b, 0, 0),
            interest("other", other, 0, 0),
        ],
    );
    let untouched = runtime
        .resident_chunk(other.chunk())
        .expect("other")
        .package;
    let mut transaction = WorldEditTransaction {
        id: "cross".into(),
        expected_revisions: BTreeMap::from([(a.chunk(), 0), (b.chunk(), 0)]),
        edits: vec![
            VoxelEdit {
                position: voxel(a, 0),
                material: None,
            },
            VoxelEdit {
                position: voxel(b, -4),
                material: None,
            },
        ],
    };
    let before = resident_fingerprints(&runtime);
    assert!(runtime.apply_transaction(&transaction).is_err());
    assert_eq!(resident_fingerprints(&runtime), before);
    transaction.edits.get_mut(1).expect("second").position.level = 0;
    let change = runtime
        .apply_transaction(&transaction)
        .expect("atomic edit");
    assert_eq!(
        change.revisions,
        BTreeMap::from([(a.chunk(), 1), (b.chunk(), 1)])
    );
    assert_eq!(change.changed_columns, vec![a, b]);
    assert_eq!(
        runtime.apply_transaction(&transaction).expect("idempotent"),
        change
    );
    assert_eq!(runtime.revision(a.chunk()), Some(1));
    assert!(Arc::ptr_eq(
        &untouched,
        &runtime
            .resident_chunk(other.chunk())
            .expect("other")
            .package
    ));
    transaction.edits.first_mut().expect("edit").position.level = -1;
    assert_eq!(
        runtime
            .apply_transaction(&transaction)
            .expect_err("reused ID")
            .kind,
        ErrorKind::Conflict
    );
    assert_eq!(
        runtime
            .apply_transaction(&edit("stale", voxel(a, -1), 0, None))
            .expect_err("stale")
            .kind,
        ErrorKind::Conflict
    );
    assert_eq!(runtime.pump().changed.len(), 2);
    assert!(runtime.pump().changed.is_empty());
}

#[test]
fn edits_survive_stream_out_and_back_in_without_source_mutation() {
    let at = point(-1, -1);
    let package = world(&[(at, 0)]);
    let source_fingerprint = package.chunks.get(&at.chunk()).expect("chunk").fingerprint;
    let mut runtime = make_runtime(package);
    load(&mut runtime, vec![interest("actor", at, 0, 0)]);
    runtime
        .apply_transaction(&edit("dig", voxel(at, 0), 0, None))
        .expect("dig");
    let edited = runtime
        .resident_chunk(at.chunk())
        .expect("chunk")
        .package
        .fingerprint;
    runtime.set_interests(Vec::new()).expect("unload");
    assert_eq!(runtime.counts().resident_chunks, 0);
    load(&mut runtime, vec![interest("actor", at, 0, 0)]);
    assert_eq!(runtime.voxel(voxel(at, 0)), QueryResult::Ready(None));
    assert_eq!(runtime.revision(at.chunk()), Some(1));
    assert_eq!(
        runtime
            .resident_chunk(at.chunk())
            .expect("chunk")
            .package
            .fingerprint,
        edited
    );
    assert_eq!(
        runtime
            .manifest()
            .chunks
            .first()
            .expect("base descriptor")
            .fingerprint,
        source_fingerprint
    );
}

#[test]
fn object_blocker_and_surface_remain_exact_with_foreign_owner_unloaded() {
    let owner = point(15, 1);
    let target = point(16, 1);
    let mut package = world(&[(owner, 0), (target, 0)]);
    package
        .chunks
        .get_mut(&owner.chunk())
        .expect("root")
        .semantics
        .objects
        .push(ObjectInstance {
            id: "crossing-rock".into(),
            region_id: "region-0000".into(),
            asset: "rock".into(),
            origin: voxel(owner, 1),
            rotation: 0,
            occupancy: vec![ColumnData {
                position: target,
                runs: vec![run(1, 4, "stone")],
            }],
        });
    package.seal().expect("project foreign occupancy");
    let mut runtime = make_runtime(package);
    load(&mut runtime, vec![interest("actor", target, 0, 0)]);
    assert_eq!(runtime.revision(owner.chunk()), None);
    assert_eq!(
        runtime.voxel(voxel(target, 2)),
        QueryResult::Ready(Some("stone".into()))
    );
    let QueryResult::Ready(surfaces) = runtime.surfaces(target) else {
        panic!("exact target");
    };
    assert_eq!(
        surfaces
            .iter()
            .map(|surface| (surface.position.level, surface.headroom))
            .collect::<Vec<_>>(),
        vec![(3, Some(1)), (6, None)]
    );
    let error = runtime
        .apply_transaction(&edit("object-blocker", voxel(target, 2), 0, None))
        .expect_err("requires semantic edit");
    assert_eq!(error.kind, ErrorKind::Conflict);
    assert!(error.message.contains("semantic regeneration"));
}

#[test]
fn semantic_edit_rejection_preserves_supported_anchor_and_roof_facts() {
    let at = point(1, 1);
    let mut package = world(&[(at, 0)]);
    let semantics = &mut package
        .chunks
        .get_mut(&at.chunk())
        .expect("chunk")
        .semantics;
    semantics.anchors.push(WorldAnchor {
        id: "spawn".into(),
        region_id: "region-0000".into(),
        position: voxel(at, 0),
        role: AnchorRole::Gameplay,
    });
    semantics.interiors.push(InteriorSpan {
        id: "room".into(),
        column: at,
        floor_level: 0,
        roof_bottom: 5,
        roof_top: 7,
        light_domain: "room-light".into(),
    });
    package.seal().expect("valid semantic fixture");
    let mut runtime = make_runtime(package);
    load(&mut runtime, vec![interest("actor", at, 0, 0)]);
    let before = resident_fingerprints(&runtime);
    for (id, level) in [("floor", 0), ("roof", 5)] {
        let error = runtime
            .apply_transaction(&edit(id, voxel(at, level), 0, None))
            .expect_err("semantic regeneration needed");
        assert!(error.message.contains("semantic regeneration"));
        assert_eq!(resident_fingerprints(&runtime), before);
    }
    runtime
        .apply_transaction(&edit("underground", voxel(at, -2), 0, None))
        .expect("supported ordinary edit away from semantic surfaces");
}

#[test]
fn delta_delivery_is_local_ordered_and_duplicate_safe() {
    let a = point(1, 1);
    let b = point(80, 1);
    let package = world(&[(a, 0), (b, 0)]);
    let mut server = make_runtime(package.clone());
    let mut client = make_runtime(package);
    for runtime in [&mut server, &mut client] {
        load(
            runtime,
            vec![interest("a", a, 0, 0), interest("b", b, 0, 0)],
        );
    }
    server
        .apply_transaction(&edit("first", voxel(a, 0), 0, None))
        .expect("first");
    let first = server
        .transaction_delta("first")
        .expect("lookup")
        .expect("delta");
    server
        .apply_transaction(&edit("second", voxel(a, -1), 1, None))
        .expect("second");
    let second = server
        .transaction_delta("second")
        .expect("lookup")
        .expect("delta");
    let untouched = client.resident_chunk(b.chunk()).expect("unrelated").package;
    assert_eq!(
        client.apply_delta(&second).expect_err("out of order").kind,
        ErrorKind::Conflict
    );
    let applied = client.apply_delta(&first).expect("first delta");
    assert_eq!(client.apply_delta(&first).expect("duplicate"), applied);
    let mut mismatched = first.clone();
    mismatched.request_fingerprint ^= 1;
    mismatched.fingerprint = 0;
    mismatched.fingerprint =
        hash_serializable(&mismatched).expect("independently resealed mismatched identity");
    assert_eq!(
        client
            .apply_delta(&mismatched)
            .expect_err("same identity with different valid payload")
            .kind,
        ErrorKind::Conflict
    );
    client.apply_delta(&second).expect("ordered second");
    assert_eq!(
        resident_fingerprints(&client),
        resident_fingerprints(&server)
    );
    assert!(Arc::ptr_eq(
        &untouched,
        &client.resident_chunk(b.chunk()).expect("unchanged").package
    ));
    let mut corrupt = first.clone();
    corrupt
        .chunks
        .first_mut()
        .expect("chunk")
        .columns
        .first_mut()
        .expect("column")
        .runs
        .clear();
    assert!(client.apply_delta(&corrupt).is_err());
    assert_eq!(
        resident_fingerprints(&client),
        resident_fingerprints(&server)
    );
}

#[test]
fn durable_edit_and_remote_ack_restore_fresh_runtime_and_idempotency() {
    let at = point(-1, 1);
    let package = world(&[(at, 0)]);
    let temp = TempRoot::new();
    let mut server = make_runtime(package.clone());
    load(&mut server, vec![interest("a", at, 0, 0)]);
    let command = edit("durable", voxel(at, 0), 0, None);
    let change = server
        .apply_transaction_durable(&command, temp.child("server"), IoLimits::default())
        .expect("durable commit");
    let delta = server
        .transaction_delta("durable")
        .expect("lookup")
        .expect("delta");
    let mut receiver = make_runtime(package.clone());
    load(&mut receiver, vec![interest("a", at, 0, 0)]);
    assert_eq!(
        receiver
            .apply_delta_durable(&delta, temp.child("receiver"), IoLimits::default())
            .expect("durable remote ACK"),
        change
    );
    drop(receiver);
    let mut restored = make_runtime(package);
    restored
        .restore_save(temp.child("receiver"), IoLimits::default())
        .expect("fresh restore");
    assert_eq!(
        restored.counts().resident_chunks,
        0,
        "restore does not read complete chunk terrain"
    );
    assert_eq!(
        restored
            .apply_delta(&delta)
            .expect("durable duplicate after restart"),
        change
    );
    load(&mut restored, vec![interest("a", at, 0, 0)]);
    assert_eq!(restored.voxel(voxel(at, 0)), QueryResult::Ready(None));
    assert_eq!(restored.revision(at.chunk()), Some(1));
    assert_eq!(
        restored
            .apply_transaction(&command)
            .expect("same origin command after restart"),
        change
    );
}

#[test]
fn checkpoint_failure_preserves_old_head_and_authority_and_unrelated_files() {
    let a = point(1, 1);
    let b = point(80, 1);
    let package = world(&[(a, 0), (b, 0)]);
    let temp = TempRoot::new();
    let save = temp.child("save");
    let mut runtime = make_runtime(package.clone());
    load(
        &mut runtime,
        vec![interest("a", a, 0, 0), interest("b", b, 0, 0)],
    );
    runtime
        .apply_transaction_durable(&edit("a", voxel(a, 0), 0, None), &save, IoLimits::default())
        .expect("first durable");
    let head = fs::read(save.join("current.ron")).expect("head");
    let first_path = fs::read_dir(save.join("partitions"))
        .expect("partitions")
        .next()
        .expect("file")
        .expect("entry")
        .path();
    let first_bytes = fs::read(&first_path).expect("first partition");
    let first_time = fs::metadata(&first_path)
        .expect("metadata")
        .modified()
        .expect("mtime");
    let before = resident_fingerprints(&runtime);
    let limits = IoLimits {
        max_transaction_bytes: 1,
        ..IoLimits::default()
    };
    assert!(runtime
        .apply_transaction_durable(&edit("b", voxel(b, 0), 0, None), &save, limits)
        .is_err());
    assert_eq!(fs::read(save.join("current.ron")).expect("old head"), head);
    assert_eq!(resident_fingerprints(&runtime), before);
    let mut crash_recovery = make_runtime(package.clone());
    crash_recovery
        .restore_save(&save, IoLimits::default())
        .expect("old head remains usable despite orphan staged partition");
    load(&mut crash_recovery, vec![interest("b", b, 0, 0)]);
    assert_eq!(
        crash_recovery.voxel(voxel(b, 0)),
        QueryResult::Ready(Some("stone".into()))
    );
    assert_eq!(crash_recovery.revision(b.chunk()), Some(0));
    runtime
        .apply_transaction_durable(&edit("b", voxel(b, 0), 0, None), &save, IoLimits::default())
        .expect("retry after failed preparation");
    assert_eq!(fs::read(&first_path).expect("partition"), first_bytes);
    assert_eq!(
        fs::metadata(&first_path)
            .expect("metadata")
            .modified()
            .expect("mtime"),
        first_time,
        "unrelated immutable partition was not rewritten"
    );
    runtime
        .set_interests(Vec::new())
        .expect("unload modified chunks");
    load(&mut runtime, vec![interest("a", a, 0, 0)]);
    assert_eq!(runtime.voxel(voxel(a, 0)), QueryResult::Ready(None));
    let mut restored = make_runtime(package);
    restored
        .restore_save(&save, IoLimits::default())
        .expect("restored latest durable state");
    load(&mut restored, vec![interest("b", b, 0, 0)]);
    assert_eq!(restored.voxel(voxel(b, 0)), QueryResult::Ready(None));
}

#[test]
fn immutable_package_publication_is_bounded_and_fail_atomic() {
    let package = world(&[(point(1, 1), 0)]);
    let temp = TempRoot::new();
    let root = temp.child("compiled");
    assert!(publish_package(
        &root,
        &package,
        IoLimits {
            max_chunk_bytes: 1,
            ..IoLimits::default()
        }
    )
    .is_err());
    assert!(
        !root.exists(),
        "no manifest becomes visible after a partial package write"
    );
    publish_package(&root, &package, IoLimits::default()).expect("publish");
    publish_package(&root, &package, IoLimits::default()).expect("identical retry");
    let source = FileChunkSource::open(root.join("manifest.ron"), IoLimits::default())
        .expect("manifest only");
    assert_eq!(source.manifest(), &package.manifest);
    let coordinate = point(1, 1).chunk();
    assert_eq!(
        source.load_chunk(coordinate).expect("chunk"),
        package.chunks.get(&coordinate).expect("expected").clone()
    );
    fs::write(root.join("chunks/0_0.ron"), "(truncated:").expect("corrupt chunk");
    let source = FileChunkSource::open(root.join("manifest.ron"), IoLimits::default())
        .expect("opening source does not eagerly read chunks");
    assert!(source.load_chunk(coordinate).is_err());
    assert!(
        publish_package(&root, &package, IoLimits::default()).is_err(),
        "same manifest does not conceal corrupt retry payload"
    );
}

#[test]
fn malformed_save_head_is_atomic_and_corrupt_lazy_partition_never_publishes() {
    let at = point(1, 1);
    let package = world(&[(at, 0)]);
    let temp = TempRoot::new();
    let save = temp.child("save");
    let mut runtime = make_runtime(package.clone());
    load(&mut runtime, vec![interest("a", at, 0, 0)]);
    runtime
        .apply_transaction_durable(
            &edit("dig", voxel(at, 0), 0, None),
            &save,
            IoLimits::default(),
        )
        .expect("save");
    let head = fs::read(save.join("current.ron")).expect("head");
    let before = resident_fingerprints(&runtime);
    fs::write(save.join("current.ron"), "invalid").expect("corrupt head");
    assert!(runtime.restore_save(&save, IoLimits::default()).is_err());
    assert_eq!(resident_fingerprints(&runtime), before);
    fs::write(save.join("current.ron"), head).expect("restore old marker");
    let partition = fs::read_dir(save.join("partitions"))
        .expect("directory")
        .next()
        .expect("file")
        .expect("entry")
        .path();
    fs::write(partition, "corrupt").expect("corrupt partition");
    let mut restored = make_runtime(package);
    restored
        .restore_save(&save, IoLimits::default())
        .expect("valid head with lazy payload");
    restored
        .set_interests(vec![interest("a", at, 0, 0)])
        .expect("interest");
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let update = restored.pump();
        if !update.failures.is_empty() {
            assert_eq!(update.failures.len(), 1);
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(restored.counts().resident_chunks, 0);
    assert_eq!(
        restored.voxel(voxel(at, 0)),
        QueryResult::Unloaded(at.chunk())
    );
}

#[cfg(unix)]
#[test]
fn package_and_save_symlink_escapes_are_rejected() {
    use std::os::unix::fs::symlink;
    let at = point(1, 1);
    let package = world(&[(at, 0)]);
    let temp = TempRoot::new();
    let root = temp.child("compiled");
    publish_package(&root, &package, IoLimits::default()).expect("publish");
    let target = root.join("chunks/0_0.ron");
    let foreign = temp.child("foreign.ron");
    fs::rename(&target, &foreign).expect("move outside package");
    symlink(&foreign, &target).expect("symlink");
    let source =
        FileChunkSource::open(root.join("manifest.ron"), IoLimits::default()).expect("source");
    assert_eq!(
        source
            .load_chunk(at.chunk())
            .expect_err("outside-root symlink")
            .kind,
        ErrorKind::InvalidData
    );
    let mut runtime = make_runtime(package);
    load(&mut runtime, vec![interest("a", at, 0, 0)]);
    let save = temp.child("save");
    let outside = temp.child("outside");
    fs::create_dir(&save).expect("save");
    fs::create_dir(&outside).expect("outside");
    symlink(&outside, save.join("partitions")).expect("partition symlink");
    let before = resident_fingerprints(&runtime);
    assert!(runtime
        .apply_transaction_durable(
            &edit("dig", voxel(at, 0), 0, None),
            &save,
            IoLimits::default()
        )
        .is_err());
    assert_eq!(resident_fingerprints(&runtime), before);
    assert_eq!(
        fs::read_dir(outside)
            .expect("outside remains empty")
            .count(),
        0
    );
}

struct ControlledSource {
    inner: MemoryChunkSource,
    calls: Arc<AtomicUsize>,
    released: Arc<AtomicBool>,
}
impl ChunkSource for ControlledSource {
    fn manifest(&self) -> &WorldManifest {
        self.inner.manifest()
    }
    fn load_chunk(&self, coordinate: ChunkId) -> RuntimeResult<ChunkPackage> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let deadline = Instant::now() + Duration::from_secs(8);
        // Intentionally ignores cancellation to prove admission and actual-job
        // limits remain safe even for a source that cannot interrupt its IO.
        while !self.released.load(Ordering::SeqCst) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        self.inner.load_chunk(coordinate)
    }
}

#[test]
fn canceled_delayed_jobs_keep_slots_and_never_publish_stale_interest() {
    let a = point(1, 1);
    let b = point(80, 1);
    let package = world(&[(a, 0), (b, 0)]);
    let calls = Arc::new(AtomicUsize::new(0));
    let released = Arc::new(AtomicBool::new(false));
    let source = ControlledSource {
        inner: MemoryChunkSource::new(package).expect("source"),
        calls: Arc::clone(&calls),
        released: Arc::clone(&released),
    };
    let mut runtime = WorldRuntime::new(
        Arc::new(source),
        RuntimeConfig {
            max_in_flight_jobs: 1,
            ..RuntimeConfig::default()
        },
    )
    .expect("runtime");
    runtime
        .set_interests(vec![interest("actor", a, 0, 0)])
        .expect("a");
    let _update = runtime.pump();
    let deadline = Instant::now() + Duration::from_secs(8);
    while calls.load(Ordering::SeqCst) == 0 {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    for _ in 0..40 {
        runtime
            .set_interests(vec![interest("actor", b, 0, 0)])
            .expect("b");
        let update = runtime.pump();
        assert!(update.loaded.is_empty());
        assert_eq!(runtime.counts().in_flight_jobs, 1);
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "canceling cannot release a slot before its worker finishes"
    );
    released.store(true, Ordering::SeqCst);
    let published = settle(&mut runtime);
    assert_eq!(
        published
            .iter()
            .map(|product| product.coordinate)
            .collect::<Vec<_>>(),
        vec![b.chunk()]
    );
    assert_eq!(runtime.voxel(voxel(a, 0)), QueryResult::Unloaded(a.chunk()));
    assert_eq!(runtime.counts().resident_chunks, 1);
}

#[test]
fn delayed_old_source_result_loses_admission_after_source_replacement() {
    let at = point(1, 1);
    let package = world(&[(at, 0)]);
    let calls = Arc::new(AtomicUsize::new(0));
    let released = Arc::new(AtomicBool::new(false));
    let source = ControlledSource {
        inner: MemoryChunkSource::new(package.clone()).expect("source"),
        calls: Arc::clone(&calls),
        released: Arc::clone(&released),
    };
    let mut runtime = WorldRuntime::new(
        Arc::new(source),
        RuntimeConfig {
            max_in_flight_jobs: 1,
            ..RuntimeConfig::default()
        },
    )
    .expect("runtime");
    runtime
        .set_interests(vec![interest("a", at, 0, 0)])
        .expect("interest");
    let _update = runtime.pump();
    let deadline = Instant::now() + Duration::from_secs(8);
    while calls.load(Ordering::SeqCst) == 0 {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    let mut replacement = package;
    replacement
        .chunks
        .get_mut(&at.chunk())
        .expect("chunk")
        .columns
        .first_mut()
        .expect("column")
        .runs
        .get_mut(1)
        .expect("stone")
        .top = 2;
    replacement.seal().expect("replacement");
    let expected = replacement
        .chunks
        .get(&at.chunk())
        .expect("new package")
        .fingerprint;
    runtime
        .replace_source(Arc::new(
            MemoryChunkSource::new(replacement).expect("new source"),
        ))
        .expect("replace");
    released.store(true, Ordering::SeqCst);
    let published = settle(&mut runtime);
    assert_eq!(published.len(), 1);
    assert_eq!(
        published.first().expect("new product").package.fingerprint,
        expected
    );
    assert_eq!(
        runtime.voxel(voxel(at, 1)),
        QueryResult::Ready(Some("stone".into()))
    );
}

#[test]
fn distant_catalogue_growth_does_not_load_dormant_payloads_or_enlarge_active_work() {
    let at = point(1, 1);
    for distant_count in [0, 80] {
        let mut regions = vec![(at, 0)];
        regions
            .extend((0..distant_count).map(|index| (point(10_000 + i64::from(index) * 64, 1), 0)));
        let calls = Arc::new(AtomicUsize::new(0));
        let released = Arc::new(AtomicBool::new(true));
        let source = ControlledSource {
            inner: MemoryChunkSource::new(world(&regions)).expect("source"),
            calls: Arc::clone(&calls),
            released,
        };
        let mut runtime =
            WorldRuntime::new(Arc::new(source), RuntimeConfig::default()).expect("runtime");
        load(&mut runtime, vec![interest("a", at, 0, 0)]);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        for _ in 0..100 {
            assert_eq!(runtime.voxel(voxel(at, 2)), QueryResult::Ready(None));
            assert!(runtime.pump().loaded.is_empty());
        }
        assert_eq!(
            runtime.counts(),
            RuntimeCounts {
                resident_chunks: 1,
                in_flight_jobs: 0,
                queued_chunks: 0,
                pinned_chunks: 0,
                modified_chunks: 0
            }
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn runtime_is_send_and_sync_for_an_engine_owned_resource() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<WorldRuntime>();
}

#[test]
fn manifest_traversal_unknown_fields_and_oversized_reads_fail_closed() {
    let package = world(&[(point(1, 1), 0)]);
    let temp = TempRoot::new();
    let root = temp.child("compiled");
    publish_package(&root, &package, IoLimits::default()).expect("publish");
    assert!(FileChunkSource::open(
        root.join("manifest.ron"),
        IoLimits {
            max_manifest_bytes: 1,
            ..IoLimits::default()
        }
    )
    .is_err());
    let mut unsafe_manifest = package.manifest.clone();
    unsafe_manifest.chunks.first_mut().expect("descriptor").path = "../foreign.ron".into();
    unsafe_manifest.fingerprint =
        fingerprint(&unsafe_manifest).expect("tampered but internally hashed manifest");
    fs::write(
        root.join("manifest.ron"),
        ron::to_string(&unsafe_manifest).expect("wire"),
    )
    .expect("unsafe manifest");
    assert!(FileChunkSource::open(root.join("manifest.ron"), IoLimits::default()).is_err());
    let wire = ron::to_string(&package.manifest).expect("wire");
    let wire = wire.replacen('(', "(unexpected:1,", 1);
    fs::write(root.join("manifest.ron"), wire).expect("unknown field");
    assert!(FileChunkSource::open(root.join("manifest.ron"), IoLimits::default()).is_err());
}

#[test]
fn save_refuses_different_source_and_restore_respects_pins() {
    let at = point(1, 1);
    let package = world(&[(at, 0)]);
    let temp = TempRoot::new();
    let save = temp.child("save");
    let mut runtime = make_runtime(package.clone());
    load(&mut runtime, vec![interest("a", at, 0, 0)]);
    runtime
        .apply_transaction_durable(
            &edit("dig", voxel(at, 0), 0, None),
            &save,
            IoLimits::default(),
        )
        .expect("save");
    runtime
        .pin("active-encounter", BTreeSet::from([at.chunk()]))
        .expect("pin");
    assert_eq!(
        runtime
            .restore_save(&save, IoLimits::default())
            .expect_err("pinned restore")
            .kind,
        ErrorKind::Pinned
    );
    let mut other = package;
    other.manifest.compiler_version = "other-version".into();
    other.seal().expect("new source");
    assert_eq!(
        runtime
            .replace_source(Arc::new(
                MemoryChunkSource::new(other.clone()).expect("source")
            ))
            .expect_err("edited source immutable")
            .kind,
        ErrorKind::Conflict
    );
    let mut wrong = make_runtime(other);
    assert_eq!(
        wrong
            .save(&save, IoLimits::default())
            .expect_err("save cannot replace another source")
            .kind,
        ErrorKind::Conflict
    );
    assert_eq!(
        wrong
            .restore_save(&save, IoLimits::default())
            .expect_err("wrong source")
            .kind,
        ErrorKind::Conflict
    );
}

#[test]
fn no_op_commands_and_stale_save_writers_cannot_erase_durable_history() {
    let at = point(1, 1);
    let package = world(&[(at, 0)]);
    let temp = TempRoot::new();
    let save = temp.child("save");
    let mut first = make_runtime(package.clone());
    load(&mut first, vec![interest("a", at, 0, 0)]);
    assert!(first
        .apply_transaction(&edit("air", voxel(at, 3), 0, None))
        .is_err());
    assert_eq!(first.revision(at.chunk()), Some(0));
    assert!(first.transaction_delta("air").expect("lookup").is_none());
    first
        .apply_transaction_durable(
            &edit("first", voxel(at, 0), 0, None),
            &save,
            IoLimits::default(),
        )
        .expect("first writer");
    let before = fs::read(save.join("current.ron")).expect("head");
    let mut stale = make_runtime(package);
    load(&mut stale, vec![interest("a", at, 0, 0)]);
    let error = stale
        .apply_transaction_durable(
            &edit("stale-writer", voxel(at, -1), 0, None),
            &save,
            IoLimits::default(),
        )
        .expect_err("stale save writer");
    assert_eq!(error.kind, ErrorKind::Conflict);
    assert_eq!(stale.revision(at.chunk()), Some(0));
    assert_eq!(fs::read(save.join("current.ron")).expect("head"), before);
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(save.join("writer.lock"))
        .expect("lock file");
    lock.lock().expect("external writer lock");
    assert_eq!(
        first
            .save(&save, IoLimits::default())
            .expect_err("locked")
            .kind,
        ErrorKind::Conflict
    );
    lock.unlock().expect("release lock");
}

#[test]
fn unsaved_partition_backlog_is_bounded_and_checkpoint_releases_it() {
    let a = point(1, 1);
    let b = point(80, 1);
    let package = world(&[(a, 0), (b, 0)]);
    let temp = TempRoot::new();
    let mut runtime = WorldRuntime::new(
        Arc::new(MemoryChunkSource::new(package).expect("source")),
        RuntimeConfig {
            max_unsaved_chunks: 1,
            ..RuntimeConfig::default()
        },
    )
    .expect("runtime");
    load(&mut runtime, vec![interest("a", a, 0, 0)]);
    runtime
        .apply_transaction(&edit("first", voxel(a, 0), 0, None))
        .expect("first dirty partition");
    load(&mut runtime, vec![interest("b", b, 0, 0)]);
    assert_eq!(
        runtime
            .apply_transaction(&edit("second", voxel(b, 0), 0, None))
            .expect_err("unsaved backlog")
            .kind,
        ErrorKind::Limit
    );
    assert_eq!(runtime.revision(b.chunk()), Some(0));
    runtime
        .save(temp.child("save"), IoLimits::default())
        .expect("flush backlog");
    runtime
        .apply_transaction(&edit("second", voxel(b, 0), 0, None))
        .expect("budget restored");
}

#[test]
fn publication_budget_applies_to_completed_jobs_and_transaction_products() {
    let regions = (0..9)
        .map(|index| (point(1 + i64::from(index) * 32, 1), 0))
        .collect::<Vec<_>>();
    let mut runtime = WorldRuntime::new(
        Arc::new(MemoryChunkSource::new(world(&regions)).expect("source")),
        RuntimeConfig {
            max_in_flight_jobs: 3,
            max_publications_per_pump: 1,
            ..RuntimeConfig::default()
        },
    )
    .expect("runtime");
    runtime
        .set_interests(
            regions
                .iter()
                .enumerate()
                .map(|(index, (at, _))| interest(&format!("actor-{index}"), *at, 0, 0))
                .collect(),
        )
        .expect("interests");
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut loaded = 0;
    loop {
        let update = runtime.pump();
        assert!(update.failures.is_empty());
        assert!(update.loaded.len() + update.changed.len() <= 1);
        loaded += update.loaded.len();
        assert!(runtime.counts().in_flight_jobs <= 3);
        if runtime.counts().in_flight_jobs == 0 && runtime.counts().queued_chunks == 0 {
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(loaded, 9);
    for (index, (at, _)) in regions.iter().enumerate() {
        runtime
            .apply_transaction(&edit(&format!("dig-{index}"), voxel(*at, 0), 0, None))
            .expect("edit");
    }
    for _ in 0..9 {
        let update = runtime.pump();
        assert_eq!(update.changed.len(), 1);
    }
    assert!(runtime.pump().changed.is_empty());
}

#[test]
fn corrupt_paged_transaction_body_fails_lookup_without_changing_residents() {
    let at = point(1, 1);
    let package = world(&[(at, 0)]);
    let temp = TempRoot::new();
    let save = temp.child("save");
    let mut runtime = WorldRuntime::new(
        Arc::new(MemoryChunkSource::new(package).expect("source")),
        RuntimeConfig {
            max_cached_transactions: 0,
            ..RuntimeConfig::default()
        },
    )
    .expect("runtime");
    load(&mut runtime, vec![interest("a", at, 0, 0)]);
    runtime
        .apply_transaction_durable(
            &edit("dig", voxel(at, 0), 0, None),
            &save,
            IoLimits::default(),
        )
        .expect("durable");
    let before = resident_fingerprints(&runtime);
    let journal = fs::read_dir(save.join("transactions"))
        .expect("directory")
        .next()
        .expect("file")
        .expect("entry")
        .path();
    fs::write(journal, "(corrupt: true)").expect("corrupt journal");
    assert!(runtime.transaction_delta("dig").is_err());
    assert!(runtime
        .apply_transaction(&edit("dig", voxel(at, 0), 0, None))
        .is_err());
    assert_eq!(resident_fingerprints(&runtime), before);
}

#[test]
fn derived_object_terrain_union_does_not_reapply_the_individual_wire_run_cap() {
    let at = point(1, 1);
    let mut package = world(&[(at, 0)]);
    let chunk = package.chunks.get_mut(&at.chunk()).expect("chunk");
    let count = i32::try_from(MAX_RUNS_PER_COLUMN).expect("bounded cap");
    chunk.columns.first_mut().expect("column").runs = (0..count)
        .map(|index| run(index * 4, index * 4 + 1, "stone"))
        .collect();
    chunk.semantics.objects.push(ObjectInstance {
        id: "layered-object".into(),
        region_id: "region-0000".into(),
        asset: "layered-rock".into(),
        origin: voxel(at, 1),
        rotation: 0,
        occupancy: vec![ColumnData {
            position: at,
            runs: (0..count)
                .map(|index| run(index * 4 + 2, index * 4 + 3, "stone"))
                .collect(),
        }],
    });
    package
        .seal()
        .expect("both bounded source columns are legal");
    let mut runtime = make_runtime(package);
    load(&mut runtime, vec![interest("a", at, 0, 0)]);
    let QueryResult::Ready(surfaces) = runtime.surfaces(at) else {
        panic!("derived valid union");
    };
    assert_eq!(surfaces.len(), MAX_RUNS_PER_COLUMN * 2);
    assert_eq!(
        runtime.voxel(voxel(at, count * 4 - 2)),
        QueryResult::Ready(Some("stone".into()))
    );
}

#[test]
fn historical_bodies_page_out_and_old_transactions_remain_idempotent() {
    let at = point(1, 1);
    let package = world(&[(at, 0)]);
    let temp = TempRoot::new();
    let config = RuntimeConfig {
        max_cached_transactions: 2,
        max_cached_transaction_bytes: 8192,
        ..RuntimeConfig::default()
    };
    let mut runtime = WorldRuntime::new(
        Arc::new(MemoryChunkSource::new(package.clone()).expect("source")),
        config,
    )
    .expect("runtime");
    load(&mut runtime, vec![interest("a", at, 0, 0)]);
    for revision in 0..40 {
        let material = (revision % 2 == 0).then_some("stone");
        runtime
            .apply_transaction_durable(
                &edit(
                    &format!("history-{revision}"),
                    voxel(at, 2),
                    revision,
                    material,
                ),
                temp.child("save"),
                IoLimits::default(),
            )
            .expect("durable history");
        let counts = runtime.history_counts();
        assert!(counts.cached_transactions <= 2);
        assert!(counts.resident_body_bytes <= 8192);
        assert_eq!(counts.unsaved_transactions, 0);
    }
    assert_eq!(runtime.history_counts().indexed_transactions, 40);
    drop(runtime);
    let mut restored = WorldRuntime::new(
        Arc::new(MemoryChunkSource::new(package).expect("source")),
        config,
    )
    .expect("runtime");
    restored
        .restore_save(temp.child("save"), IoLimits::default())
        .expect("metadata restore");
    assert_eq!(restored.history_counts().cached_transactions, 0);
    let first = restored
        .transaction_delta("history-0")
        .expect("paged read")
        .expect("old delta");
    assert_eq!(restored.history_counts().resident_body_bytes, 0);
    assert_eq!(
        restored
            .apply_delta(&first)
            .expect("duplicate unloaded historical command")
            .revisions
            .get(&at.chunk()),
        Some(&1)
    );
    assert_eq!(restored.history_counts().indexed_transactions, 40);
}

#[test]
fn same_chunk_unsaved_history_has_independent_count_and_byte_bounds() {
    let at = point(1, 1);
    let package = world(&[(at, 0)]);
    let temp = TempRoot::new();
    let mut runtime = WorldRuntime::new(
        Arc::new(MemoryChunkSource::new(package.clone()).expect("source")),
        RuntimeConfig {
            max_unsaved_transactions: 2,
            ..RuntimeConfig::default()
        },
    )
    .expect("runtime");
    load(&mut runtime, vec![interest("a", at, 0, 0)]);
    runtime
        .apply_transaction(&edit("one", voxel(at, 2), 0, Some("stone")))
        .expect("first");
    runtime
        .apply_transaction(&edit("two", voxel(at, 2), 1, None))
        .expect("second");
    assert_eq!(
        runtime
            .apply_transaction(&edit("three", voxel(at, 2), 2, Some("stone")))
            .expect_err("count bound")
            .kind,
        ErrorKind::Limit
    );
    assert_eq!(runtime.revision(at.chunk()), Some(2));
    runtime
        .save(temp.child("save"), IoLimits::default())
        .expect("checkpoint");
    runtime
        .apply_transaction(&edit("three", voxel(at, 2), 2, Some("stone")))
        .expect("released backlog");
    let mut bytes = WorldRuntime::new(
        Arc::new(MemoryChunkSource::new(package).expect("source")),
        RuntimeConfig {
            max_unsaved_transaction_bytes: 1,
            ..RuntimeConfig::default()
        },
    )
    .expect("runtime");
    load(&mut bytes, vec![interest("a", at, 0, 0)]);
    assert_eq!(
        bytes
            .apply_transaction(&edit("one", voxel(at, 2), 0, Some("stone")))
            .expect_err("byte bound")
            .kind,
        ErrorKind::Limit
    );
    assert_eq!(bytes.revision(at.chunk()), Some(0));
}

#[test]
fn stable_workspace_publication_retries_and_failures_preserve_current_revision() {
    let at = point(1, 1);
    let mut package = world(&[(at, 0)]);
    let temp = TempRoot::new();
    let workspace = temp.child("workspace");
    let first =
        publish_revision(&workspace, &package, IoLimits::default()).expect("initial revision");
    assert_eq!(
        FileChunkSource::open_workspace(
            first.parent().expect("package directory"),
            IoLimits::default()
        )
        .expect("direct immutable open")
        .manifest()
        .fingerprint,
        package.manifest.fingerprint
    );
    assert_eq!(
        publish_revision(&workspace, &package, IoLimits::default()).expect("retry"),
        first
    );
    let original_head = fs::read(workspace.join("current.ron")).expect("head");
    package
        .chunks
        .get_mut(&at.chunk())
        .expect("chunk")
        .columns
        .first_mut()
        .expect("column")
        .runs
        .first_mut()
        .expect("run")
        .bottom -= 1;
    package.seal().expect("revised package");
    assert!(publish_revision(
        &workspace,
        &package,
        IoLimits {
            max_chunk_bytes: 1,
            ..IoLimits::default()
        }
    )
    .is_err());
    assert_eq!(
        fs::read(workspace.join("current.ron")).expect("head"),
        original_head
    );
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(workspace.join("writer.lock"))
        .expect("writer lock");
    lock.lock().expect("lock");
    assert_eq!(
        publish_revision(&workspace, &package, IoLimits::default())
            .expect_err("concurrent writer")
            .kind,
        ErrorKind::Conflict
    );
    lock.unlock().expect("unlock");
    let second = publish_revision(&workspace, &package, IoLimits::default()).expect("new revision");
    assert_ne!(first, second);
    assert!(first.is_file());
    assert_eq!(
        FileChunkSource::open_workspace(&workspace, IoLimits::default())
            .expect("current open")
            .manifest()
            .fingerprint,
        package.manifest.fingerprint
    );
    fs::write(
        workspace.join("current.ron"),
        "(schema_version:1,manifest_path:\"../elsewhere/manifest.ron\")",
    )
    .expect("corrupt pointer");
    assert!(FileChunkSource::open_workspace(&workspace, IoLimits::default()).is_err());
}

fn knowledge_world() -> WorldPackage {
    let positions = [point(-1, -1), point(33, 1), point(65, 1), point(97, 1)];
    let mut package = world(&positions.map(|position| (position, 0)));
    for (index, position) in positions.iter().enumerate() {
        let feature = FeatureSummary {
            id: format!("landmark-{index}"),
            region_id: format!("region-{index:04}"),
            kind: "landmark".into(),
            anchor: voxel(*position, 0),
            asset: None,
        };
        package.manifest.features.push(feature.clone());
        package
            .chunks
            .get_mut(&position.chunk())
            .expect("chunk")
            .features
            .push(feature);
    }
    package.seal().expect("knowledge world");
    package
}
fn knowledge_store(temp: &TempRoot, name: &str, package: &WorldPackage) -> KnowledgeStore {
    KnowledgeStore::open(
        temp.child(name),
        &package.manifest,
        IoLimits::default(),
        KnowledgeConfig::default(),
    )
    .expect("knowledge store")
}
fn observation(principal: &str, at: WorldHex, revision: u64, level: i32) -> KnowledgePartition {
    let mut partition = KnowledgePartition::new(principal, at.chunk());
    partition.revision = revision;
    partition.discovered_columns.push(at);
    partition.surfaces.push(ObservedSurface {
        surface: Surface {
            position: voxel(at, level),
            material: "stone".into(),
            headroom: None,
        },
        world_revision: revision - 1,
    });
    partition.seal().expect("observation");
    partition
}
fn remember(
    store: &mut KnowledgeStore,
    principal: &str,
    id: &str,
    partitions: Vec<KnowledgePartition>,
) -> KnowledgeReceipt {
    let expected = partitions
        .iter()
        .map(|partition| (partition.coordinate, partition.revision - 1))
        .collect();
    store
        .compare_and_write(
            principal,
            id,
            &expected,
            partitions
                .into_iter()
                .map(|partition| (partition.coordinate, partition))
                .collect(),
        )
        .expect("durable knowledge")
}
fn scope(principal: &str, columns: &[WorldHex]) -> AuthorizedInterest {
    AuthorizedInterest::new(
        principal,
        columns.iter().map(|column| column.chunk()).collect(),
    )
    .expect("host authorized scope")
}

#[test]
fn private_knowledge_has_exact_stacked_identity_and_atomic_compare_and_write() {
    let package = knowledge_world();
    let temp = TempRoot::new();
    let a = point(-1, -1);
    let b = point(33, 1);
    let mut store = knowledge_store(&temp, "knowledge", &package);
    let mut other = knowledge_store(&temp, "knowledge", &package);
    let mut ground = observation("party-a", a, 1, 0);
    ground.landmarks.push(ObservedLandmark {
        id: "landmark-0".into(),
        position: voxel(a, 0),
        world_revision: 0,
    });
    ground.seal().expect("landmark observation");
    let initial = remember(
        &mut store,
        "party-a",
        "a-first",
        vec![ground.clone(), observation("party-a", b, 1, 0)],
    );
    remember(
        &mut other,
        "party-b",
        "b-first",
        vec![observation("party-b", a, 1, 6)],
    );
    store.refresh().expect("merge metadata");
    assert_eq!(
        store.read("party-a", a.chunk()).expect("read"),
        Some(ground.clone())
    );
    assert_eq!(
        store
            .read("party-b", a.chunk())
            .expect("read")
            .expect("known")
            .surfaces
            .first()
            .expect("upper support")
            .surface
            .position
            .level,
        6
    );
    assert_eq!(
        store
            .discovered_columns("party-a", a.chunk())
            .expect("negative chunk bitmask"),
        vec![a]
    );
    assert_eq!(
        store.discovered_chunks("party-b").expect("private map"),
        vec![a.chunk()]
    );
    let before = fs::read(temp.child("knowledge/knowledge.ron")).expect("head");
    let wrong_expected = BTreeMap::from([(a.chunk(), 1), (b.chunk(), 0)]);
    let replacements = BTreeMap::from([
        (a.chunk(), observation("party-a", a, 2, 0)),
        (b.chunk(), observation("party-a", b, 2, 0)),
    ]);
    assert!(store
        .compare_and_write("party-a", "bad-cas", &wrong_expected, replacements)
        .is_err());
    assert_eq!(
        fs::read(temp.child("knowledge/knowledge.ron")).expect("head"),
        before
    );
    assert_eq!(
        remember(
            &mut store,
            "party-a",
            "a-first",
            vec![ground, observation("party-a", b, 1, 0)]
        ),
        initial
    );
    assert!(store
        .compare_and_write(
            "party-a",
            "a-first",
            &BTreeMap::from([(a.chunk(), 0)]),
            BTreeMap::from([(a.chunk(), observation("party-a", a, 1, 6))])
        )
        .is_err());
    remember(
        &mut store,
        "party-a",
        "next",
        vec![observation("party-a", a, 2, 0)],
    );
    assert_eq!(
        store
            .read("party-a", b.chunk())
            .expect("unrelated read")
            .expect("partition")
            .revision,
        1
    );
    assert_eq!(
        store
            .read("party-b", a.chunk())
            .expect("other party")
            .expect("partition")
            .revision,
        1
    );
}

#[test]
fn knowledge_rejects_unregistered_or_regressed_observations_and_corrupt_files() {
    let package = knowledge_world();
    let temp = TempRoot::new();
    let at = point(-1, -1);
    let mut store = knowledge_store(&temp, "knowledge", &package);
    let mut known = observation("party-a", at, 1, 0);
    known.surfaces.first_mut().expect("support").world_revision = 4;
    known.seal().expect("known");
    remember(&mut store, "party-a", "known", vec![known]);
    let mut invalid = observation("party-a", at, 2, 0);
    assert!(store
        .compare_and_write(
            "party-a",
            "regression",
            &BTreeMap::from([(at.chunk(), 1)]),
            BTreeMap::from([(at.chunk(), invalid.clone())])
        )
        .is_err());
    invalid
        .surfaces
        .first_mut()
        .expect("support")
        .world_revision = 4;
    invalid.landmarks.push(ObservedLandmark {
        id: "invented".into(),
        position: voxel(at, 0),
        world_revision: 4,
    });
    invalid
        .seal()
        .expect("well shaped unregistered observation");
    assert!(store
        .compare_and_write(
            "party-a",
            "unregistered",
            &BTreeMap::from([(at.chunk(), 1)]),
            BTreeMap::from([(at.chunk(), invalid)])
        )
        .is_err());
    let owner_dir = fs::read_dir(temp.child("knowledge/knowledge"))
        .expect("private partitions")
        .next()
        .expect("owner")
        .expect("entry")
        .path();
    let file = fs::read_dir(owner_dir)
        .expect("files")
        .next()
        .expect("partition")
        .expect("entry")
        .path();
    fs::write(file, "corrupt remembered terrain").expect("corrupt one body");
    let reopened = knowledge_store(&temp, "knowledge", &package);
    assert_eq!(
        reopened
            .discovered_columns("party-a", at.chunk())
            .expect("metadata without body"),
        vec![at]
    );
    assert!(reopened.read("party-a", at.chunk()).is_err());
    assert_eq!(
        reopened
            .read("party-b", at.chunk())
            .expect("other principal absent"),
        None
    );
    let mut wrong_source = package.clone();
    wrong_source.manifest.compiler_version = "different-source".into();
    wrong_source.seal().expect("source revision");
    assert!(KnowledgeStore::open(
        temp.child("knowledge"),
        &wrong_source.manifest,
        IoLimits::default(),
        KnowledgeConfig::default()
    )
    .is_err());
}

#[test]
fn disclosure_filters_principal_and_interest_then_durably_orders_and_deduplicates() {
    let package = knowledge_world();
    let temp = TempRoot::new();
    let a = point(-1, -1);
    let b = point(33, 1);
    let mut host = knowledge_store(&temp, "host", &package);
    remember(
        &mut host,
        "party-a",
        "a",
        vec![
            observation("party-a", a, 1, 0),
            observation("party-a", b, 1, 0),
        ],
    );
    remember(
        &mut host,
        "party-b",
        "b",
        vec![observation("party-b", a, 1, 6)],
    );
    let grant = scope("party-a", &[a]);
    let mut stream = DisclosureStream::new(
        "connection",
        grant.clone(),
        DisclosureConfig {
            max_retained_batches: 1,
            ..DisclosureConfig::default()
        },
    )
    .expect("stream");
    let changed = BTreeSet::from([a.chunk(), b.chunk()]);
    let first = stream
        .publish(&host, &changed)
        .expect("publish")
        .expect("first");
    assert_eq!(first.partitions.len(), 1);
    assert_eq!(first.partitions.first().expect("part").principal, "party-a");
    assert_eq!(
        first.partitions.first().expect("part").coordinate,
        a.chunk()
    );
    remember(
        &mut host,
        "party-a",
        "a-next",
        vec![observation("party-a", a, 2, 0)],
    );
    let second = stream
        .publish(&host, &changed)
        .expect("publish")
        .expect("second");
    assert!(matches!(
        stream.reconnect(0),
        KnowledgeReplay::ResyncRequired
    ));
    assert!(
        matches!(stream.reconnect(1), KnowledgeReplay::Replay(batches) if batches == vec![second.clone()])
    );
    assert_eq!(stream.retained_counts().0, 1);
    let mut receiver = knowledge_store(&temp, "receiver", &package);
    assert!(receiver.apply_sequence_durable(&grant, &second).is_err());
    assert_eq!(
        receiver.sequence("party-a", "connection").expect("cursor"),
        0
    );
    assert!(receiver
        .apply_sequence_durable(&scope("party-b", &[a]), &first)
        .is_err());
    assert!(receiver
        .apply_sequence_durable(&scope("party-a", &[b]), &first)
        .is_err());
    let ack = receiver
        .apply_sequence_durable(&grant, &first)
        .expect("durable ack");
    assert_eq!(ack.sequence, 1);
    drop(receiver);
    let mut receiver = knowledge_store(&temp, "receiver", &package);
    assert_eq!(
        receiver
            .apply_sequence_durable(&grant, &first)
            .expect("duplicate after restart"),
        ack
    );
    let mut changed_duplicate = first.clone();
    changed_duplicate.partitions = second.partitions.clone();
    changed_duplicate.fingerprint = 0;
    changed_duplicate.fingerprint =
        hash_serializable(&changed_duplicate).expect("valid conflicting payload hash");
    assert!(receiver
        .apply_sequence_durable(&grant, &changed_duplicate)
        .is_err());
    let ack = receiver
        .apply_sequence_durable(&grant, &second)
        .expect("next sequence");
    stream.acknowledge(&ack).expect("host ack");
    assert_eq!(
        receiver
            .read("party-a", a.chunk())
            .expect("received")
            .expect("partition")
            .revision,
        2
    );
    assert_eq!(
        receiver.read("party-a", b.chunk()).expect("hidden chunk"),
        None
    );
    assert_eq!(
        receiver
            .read("party-b", a.chunk())
            .expect("hidden principal"),
        None
    );
    stream
        .set_interests(BTreeSet::from([b.chunk()]))
        .expect("scope change");
    assert_eq!(stream.retained_counts(), (0, 0));
    assert!(matches!(
        stream.reconnect(1),
        KnowledgeReplay::ResyncRequired
    ));
}

#[test]
fn reconnect_checkpoint_is_paged_private_and_restart_safe() {
    let package = knowledge_world();
    let temp = TempRoot::new();
    let columns = [point(-1, -1), point(33, 1), point(65, 1)];
    let mut host = knowledge_store(&temp, "host", &package);
    remember(
        &mut host,
        "party-a",
        "known",
        columns
            .iter()
            .map(|at| observation("party-a", *at, 1, 0))
            .collect(),
    );
    let grant = scope("party-a", &columns);
    let stream = DisclosureStream::resume(
        "reconnected",
        grant.clone(),
        7,
        DisclosureConfig {
            max_partitions_per_batch: 1,
            ..DisclosureConfig::default()
        },
    )
    .expect("resumed host stream");
    assert!(matches!(
        stream.reconnect(0),
        KnowledgeReplay::ResyncRequired
    ));
    let first = stream.checkpoint_page(&host, None).expect("first page");
    let second = stream
        .checkpoint_page(&host, first.next.as_ref())
        .expect("second page");
    let third = stream
        .checkpoint_page(&host, second.next.as_ref())
        .expect("third page");
    assert!(third.next.is_none());
    let mut receiver = knowledge_store(&temp, "receiver", &package);
    assert!(
        !receiver
            .apply_checkpoint_page_durable(&grant, &first)
            .expect("first ack")
            .checkpoint_complete
    );
    assert_eq!(
        receiver
            .sequence("party-a", "reconnected")
            .expect("incomplete cursor"),
        0
    );
    drop(receiver);
    let mut receiver = knowledge_store(&temp, "receiver", &package);
    assert!(receiver
        .apply_checkpoint_page_durable(&grant, &third)
        .is_err());
    receiver
        .apply_checkpoint_page_durable(&grant, &second)
        .expect("resumed second page");
    let ack = receiver
        .apply_checkpoint_page_durable(&grant, &third)
        .expect("complete checkpoint");
    assert_eq!(ack.sequence, 7);
    assert!(ack.checkpoint_complete);
    receiver
        .apply_checkpoint_page_durable(&grant, &first)
        .expect("old duplicate page idempotent");
    assert_eq!(
        receiver
            .sequence("party-a", "reconnected")
            .expect("cursor must not roll back"),
        7
    );
    assert_eq!(
        receiver
            .discovered_chunks("party-a")
            .expect("private map")
            .len(),
        3
    );
    assert!(receiver
        .discovered_chunks("party-b")
        .expect("other principal")
        .is_empty());
}

#[test]
fn checkpoint_snapshot_changes_only_for_scoped_principal_and_restarts_on_change() {
    let package = knowledge_world();
    let temp = TempRoot::new();
    let a = point(-1, -1);
    let b = point(33, 1);
    let hidden = point(97, 1);
    let mut host = knowledge_store(&temp, "host", &package);
    remember(
        &mut host,
        "party-a",
        "initial",
        vec![
            observation("party-a", a, 1, 0),
            observation("party-a", b, 1, 0),
        ],
    );
    let stream = DisclosureStream::resume(
        "stream",
        scope("party-a", &[a, b]),
        3,
        DisclosureConfig {
            max_partitions_per_batch: 1,
            ..DisclosureConfig::default()
        },
    )
    .expect("stream");
    let first = stream.checkpoint_page(&host, None).expect("page");
    remember(
        &mut host,
        "party-b",
        "private",
        vec![observation("party-b", a, 1, 6)],
    );
    remember(
        &mut host,
        "party-a",
        "outside",
        vec![observation("party-a", hidden, 1, 0)],
    );
    assert!(stream.checkpoint_page(&host, first.next.as_ref()).is_ok());
    remember(
        &mut host,
        "party-a",
        "scope-changed",
        vec![observation("party-a", b, 2, 0)],
    );
    assert!(stream.checkpoint_page(&host, first.next.as_ref()).is_err());
    assert!(stream.checkpoint_page(&host, None).is_ok());
}

#[test]
fn disclosure_io_budget_failure_cannot_ack_or_advance_a_receiver() {
    let package = knowledge_world();
    let temp = TempRoot::new();
    let at = point(-1, -1);
    let mut host = knowledge_store(&temp, "host", &package);
    remember(
        &mut host,
        "party-a",
        "known",
        vec![observation("party-a", at, 1, 0)],
    );
    let grant = scope("party-a", &[at]);
    let mut stream = DisclosureStream::new("stream", grant.clone(), DisclosureConfig::default())
        .expect("stream");
    let batch = stream
        .publish(&host, &BTreeSet::from([at.chunk()]))
        .expect("publish")
        .expect("batch");
    let mut receiver = KnowledgeStore::open(
        temp.child("receiver"),
        &package.manifest,
        IoLimits {
            max_manifest_bytes: 1,
            ..IoLimits::default()
        },
        KnowledgeConfig::default(),
    )
    .expect("empty receiver");
    assert!(receiver.apply_sequence_durable(&grant, &batch).is_err());
    assert_eq!(receiver.sequence("party-a", "stream").expect("cursor"), 0);
    assert_eq!(
        receiver
            .read("party-a", at.chunk())
            .expect("unchanged memory"),
        None
    );
    assert!(!temp.child("receiver/knowledge.ron").exists());
}

#[cfg(unix)]
#[test]
fn knowledge_and_workspace_reject_symlink_parent_escapes_before_writing() {
    use std::os::unix::fs::symlink;
    let package = knowledge_world();
    let temp = TempRoot::new();
    let outside = temp.child("outside");
    fs::create_dir(&outside).expect("outside");
    let mut store = knowledge_store(&temp, "knowledge", &package);
    symlink(&outside, temp.child("knowledge/knowledge")).expect("escape");
    let at = point(-1, -1);
    assert!(store
        .compare_and_write(
            "party-a",
            "first",
            &BTreeMap::from([(at.chunk(), 0)]),
            BTreeMap::from([(at.chunk(), observation("party-a", at, 1, 0))])
        )
        .is_err());
    assert_eq!(
        fs::read_dir(&outside).expect("untouched outside").count(),
        0
    );
    fs::create_dir(temp.child("workspace")).expect("workspace");
    symlink(&outside, temp.child("workspace/packages")).expect("escape");
    assert!(publish_revision(temp.child("workspace"), &package, IoLimits::default()).is_err());
    assert_eq!(
        fs::read_dir(&outside).expect("untouched outside").count(),
        0
    );
}

fn attachment_update(
    key: &str,
    expected_fingerprint: Option<u64>,
    bytes: Option<&[u8]>,
) -> AttachmentUpdate {
    AttachmentUpdate {
        owner: "gameplay".into(),
        key: key.into(),
        expected_fingerprint,
        bytes: bytes.map(<[u8]>::to_vec),
    }
}

#[test]
fn terrain_and_actor_attachments_publish_under_one_head_and_failure_preserves_both() {
    let at = point(1, 1);
    let package = world(&[(at, 0)]);
    let temp = TempRoot::new();
    let save = temp.child("save");
    let mut runtime = make_runtime(package.clone());
    load(&mut runtime, vec![interest("a", at, 0, 0)]);
    runtime
        .save_with_attachments(
            &save,
            IoLimits::default(),
            &[attachment_update(
                "actors",
                None,
                Some(b"standing at old support"),
            )],
        )
        .expect("initial actor checkpoint");
    let old = runtime
        .attachment("gameplay", "actors")
        .expect("read")
        .expect("actors");
    let old_head = fs::read(save.join("current.ron")).expect("initial head");
    let updates = vec![attachment_update(
        "actors",
        Some(old.fingerprint),
        Some(b"standing at revised support"),
    )];
    let command = edit("terrain-and-actor", voxel(at, 2), 0, Some("stone"));
    let limits = IoLimits {
        max_manifest_bytes: old_head.len(),
        ..IoLimits::default()
    };
    assert!(runtime
        .apply_transaction_durable_with_attachments(&command, &save, limits, &updates)
        .is_err());
    assert_eq!(
        fs::read(save.join("current.ron")).expect("unchanged head"),
        old_head
    );
    assert_eq!(runtime.revision(at.chunk()), Some(0));
    assert_eq!(runtime.voxel(voxel(at, 2)), QueryResult::Ready(None));
    assert_eq!(
        runtime
            .attachment("gameplay", "actors")
            .expect("unchanged actors"),
        Some(old.clone())
    );
    assert_eq!(
        fs::read_dir(save.join("attachments"))
            .expect("prepared immutable bodies")
            .count(),
        2,
        "failure occurs after immutable preparation, before the head switch"
    );
    let mut restored = make_runtime(package.clone());
    restored
        .restore_save(&save, IoLimits::default())
        .expect("restore previous complete head");
    assert_eq!(
        restored
            .attachment("gameplay", "actors")
            .expect("previous actor body"),
        Some(old)
    );
    load(&mut restored, vec![interest("a", at, 0, 0)]);
    assert_eq!(restored.voxel(voxel(at, 2)), QueryResult::Ready(None));
    runtime
        .apply_transaction_durable_with_attachments(&command, &save, IoLimits::default(), &updates)
        .expect("atomic retry");
    let mut restored = make_runtime(package);
    restored
        .restore_save(&save, IoLimits::default())
        .expect("new complete head");
    assert_eq!(
        restored
            .attachment("gameplay", "actors")
            .expect("new actors")
            .expect("body")
            .bytes,
        b"standing at revised support"
    );
    load(&mut restored, vec![interest("a", at, 0, 0)]);
    assert_eq!(
        restored.voxel(voxel(at, 2)),
        QueryResult::Ready(Some("stone".into()))
    );
}

#[test]
fn attachment_cas_is_owner_local_and_ordinary_checkpoints_retain_durable_keys() {
    let at = point(1, 1);
    let package = world(&[(at, 0)]);
    let temp = TempRoot::new();
    let save = temp.child("save");
    let mut first = make_runtime(package.clone());
    first
        .save_with_attachments(
            &save,
            IoLimits::default(),
            &[
                attachment_update("party-a", None, Some(b"a1")),
                attachment_update("party-b", None, Some(b"b1")),
            ],
        )
        .expect("initial owners");
    let mut stale = make_runtime(package.clone());
    stale
        .restore_save(&save, IoLimits::default())
        .expect("second owner snapshot");
    let a1 = first
        .attachment("gameplay", "party-a")
        .expect("read")
        .expect("a1");
    first
        .save_with_attachments(
            &save,
            IoLimits::default(),
            &[attachment_update(
                "party-a",
                Some(a1.fingerprint),
                Some(b"a2"),
            )],
        )
        .expect("next actor checkpoint");
    assert!(stale
        .save_with_attachments(
            &save,
            IoLimits::default(),
            &[attachment_update(
                "party-a",
                Some(a1.fingerprint),
                Some(b"stale actor")
            )]
        )
        .is_err());
    stale
        .save(&save, IoLimits::default())
        .expect("ordinary save retains latest locked owner head");
    assert_eq!(
        stale
            .attachment("gameplay", "party-a")
            .expect("retained")
            .expect("a2")
            .bytes,
        b"a2"
    );
    assert_eq!(
        stale
            .attachment("gameplay", "party-b")
            .expect("unrelated")
            .expect("b1")
            .bytes,
        b"b1"
    );
    let a2 = stale
        .attachment("gameplay", "party-a")
        .expect("read")
        .expect("a2");
    stale
        .save_with_attachments(
            &save,
            IoLimits::default(),
            &[attachment_update("party-a", Some(a2.fingerprint), None)],
        )
        .expect("explicit removal");
    assert_eq!(
        stale.attachment("gameplay", "party-a").expect("removed"),
        None
    );
    stale
        .save(temp.child("copy"), IoLimits::default())
        .expect("copy preserves remaining opaque body");
    let mut copied = make_runtime(package);
    copied
        .restore_save(temp.child("copy"), IoLimits::default())
        .expect("copy restore");
    assert_eq!(
        copied
            .attachment("gameplay", "party-b")
            .expect("copied")
            .expect("b1")
            .bytes,
        b"b1"
    );
}

#[test]
fn durable_transaction_retries_bind_actor_payloads_and_never_roll_back_later_movement() {
    let at = point(1, 1);
    let package = world(&[(at, 0)]);
    let temp = TempRoot::new();
    let save = temp.child("save");
    let mut runtime = make_runtime(package.clone());
    load(&mut runtime, vec![interest("a", at, 0, 0)]);
    let command = edit("combined", voxel(at, 2), 0, Some("stone"));
    let updates = vec![attachment_update("actors", None, Some(b"after edit"))];
    let result = runtime
        .apply_transaction_durable_with_attachments(&command, &save, IoLimits::default(), &updates)
        .expect("combined commit");
    let actor = runtime
        .attachment("gameplay", "actors")
        .expect("read")
        .expect("actor");
    runtime
        .save_with_attachments(
            &save,
            IoLimits::default(),
            &[attachment_update(
                "actors",
                Some(actor.fingerprint),
                Some(b"later movement"),
            )],
        )
        .expect("actor only progress");
    let mut restored = make_runtime(package);
    restored
        .restore_save(&save, IoLimits::default())
        .expect("restore idempotency binding");
    assert_eq!(
        restored
            .apply_transaction_durable_with_attachments(
                &command,
                &save,
                IoLimits::default(),
                &updates
            )
            .expect("exact duplicate"),
        result
    );
    assert_eq!(
        restored
            .attachment("gameplay", "actors")
            .expect("later movement retained")
            .expect("actor")
            .bytes,
        b"later movement"
    );
    assert!(restored
        .apply_transaction_durable_with_attachments(
            &command,
            &save,
            IoLimits::default(),
            &[attachment_update("actors", None, Some(b"altered retry"))]
        )
        .is_err());
    restored
        .apply_transaction_durable(&command, &save, IoLimits::default())
        .expect("ordinary duplicate retains actors");
    load(&mut restored, vec![interest("a", at, 0, 0)]);
    let plain = edit("plain", voxel(at, 2), 1, None);
    restored
        .apply_transaction_durable(&plain, &save, IoLimits::default())
        .expect("plain terrain commit");
    assert!(restored
        .apply_transaction_durable_with_attachments(
            &plain,
            &save,
            IoLimits::default(),
            &[attachment_update(
                "new-owner",
                None,
                Some(b"retroactive payload")
            )]
        )
        .is_err());
}

#[test]
fn attachment_budgets_and_corruption_fail_without_interpreting_owner_bytes() {
    let at = point(1, 1);
    let package = world(&[(at, 0)]);
    let temp = TempRoot::new();
    let save = temp.child("save");
    let mut runtime = WorldRuntime::new(
        Arc::new(MemoryChunkSource::new(package.clone()).expect("source")),
        RuntimeConfig {
            max_attachment_updates: 1,
            ..RuntimeConfig::default()
        },
    )
    .expect("runtime");
    assert!(runtime
        .save_with_attachments(
            &save,
            IoLimits::default(),
            &[
                attachment_update("a", None, Some(b"a")),
                attachment_update("b", None, Some(b"b"))
            ]
        )
        .is_err());
    assert!(runtime
        .save_with_attachments(
            &save,
            IoLimits {
                max_chunk_bytes: 1,
                ..IoLimits::default()
            },
            &[attachment_update("a", None, Some(b"too large"))]
        )
        .is_err());
    assert!(runtime
        .save_with_attachments(
            &save,
            IoLimits {
                max_transaction_bytes: 1,
                ..IoLimits::default()
            },
            &[attachment_update("a", None, Some(b"too large"))]
        )
        .is_err());
    assert!(!save.join("current.ron").exists());
    runtime
        .save_with_attachments(
            &save,
            IoLimits::default(),
            &[attachment_update(
                "a",
                None,
                Some(b"opaque-ron-or-json-or-binary"),
            )],
        )
        .expect("opaque bytes");
    let file = fs::read_dir(save.join("attachments"))
        .expect("files")
        .next()
        .expect("file")
        .expect("entry")
        .path();
    fs::write(file, b"corrupt").expect("corrupt body");
    let mut restored = make_runtime(package);
    restored
        .restore_save(&save, IoLimits::default())
        .expect("metadata-only restore");
    assert!(restored.attachment("gameplay", "a").is_err());
}
