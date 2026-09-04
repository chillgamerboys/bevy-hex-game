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
    let first = server.transaction_delta("first").expect("delta").clone();
    server
        .apply_transaction(&edit("second", voxel(a, -1), 1, None))
        .expect("second");
    let second = server.transaction_delta("second").expect("delta").clone();
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
    let delta = server.transaction_delta("durable").expect("delta").clone();
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
    assert!(first.transaction_delta("air").is_none());
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
fn corrupt_transaction_journal_aborts_restore_before_retiring_residents() {
    let at = point(1, 1);
    let package = world(&[(at, 0)]);
    let temp = TempRoot::new();
    let save = temp.child("save");
    let mut runtime = make_runtime(package);
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
    assert!(runtime.restore_save(&save, IoLimits::default()).is_err());
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
