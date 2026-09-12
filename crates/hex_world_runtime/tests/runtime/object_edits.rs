use super::*;

fn object(id: &str, origin: WorldHex, columns: &[WorldHex]) -> ObjectInstance {
    ObjectInstance {
        id: id.into(),
        region_id: "region-0000".into(),
        asset: "tree.oak".into(),
        origin: voxel(origin, 1),
        rotation: 0,
        occupancy: columns
            .iter()
            .map(|position| ColumnData {
                position: *position,
                runs: vec![run(1, 3, "stone")],
            })
            .collect(),
    }
}
fn fixture(overlap: bool) -> (WorldPackage, ObjectInstance) {
    let mut source = world(&[(point(15, 0), 3), (point(1000, 0), 0)]);
    let a = object("authored/tree", point(15, 0), &[point(15, 0), point(16, 0)]);
    source
        .chunks
        .get_mut(&a.origin.column.chunk())
        .expect("root")
        .semantics
        .objects
        .push(a.clone());
    if overlap {
        let mut b = object(
            "authored/other",
            point(1000, 0),
            &[point(15, 0), point(16, 0)],
        );
        b.region_id = "region-0001".into();
        source
            .chunks
            .get_mut(&b.origin.column.chunk())
            .expect("remote root")
            .semantics
            .objects
            .push(b);
    }
    source.seal().expect("source with exact identities");
    (source, a)
}
fn transaction(
    runtime: &WorldRuntime,
    id: &str,
    before: Option<ObjectInstance>,
    after: Option<ObjectInstance>,
) -> WorldObjectEditTransaction {
    let mut tx = WorldObjectEditTransaction {
        id: id.into(),
        expected_revisions: BTreeMap::new(),
        edits: vec![ObjectEdit { before, after }],
    };
    tx.expected_revisions = tx
        .affected_columns()
        .expect("columns")
        .into_iter()
        .map(WorldHex::chunk)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|chunk| (chunk, runtime.revision(chunk).expect("resident dependency")))
        .collect();
    tx.validate().expect("valid command");
    tx
}
fn snapshot(runtime: &WorldRuntime) -> BTreeMap<ChunkId, (u64, ChunkPackage)> {
    runtime
        .resident_chunks()
        .map(|product| {
            (
                product.coordinate,
                (product.revision, (*product.package).clone()),
            )
        })
        .collect()
}
fn loaded(source: WorldPackage) -> WorldRuntime {
    let mut runtime = make_runtime(source);
    load(&mut runtime, vec![interest("near", point(15, 0), 3, 3)]);
    runtime
}
fn moved(before: &ObjectInstance) -> ObjectInstance {
    let mut after = before.clone();
    after.origin.column = point(17, 0);
    after.occupancy = vec![
        ColumnData {
            position: point(17, 0),
            runs: vec![run(1, 3, "stone")],
        },
        ColumnData {
            position: point(18, 0),
            runs: vec![run(1, 3, "stone")],
        },
    ];
    after.rotation = 1;
    after
}

#[test]
fn removing_spanning_object_keeps_shared_occupancy_with_the_other_root_unloaded() {
    let (source, a) = fixture(true);
    let mut runtime = loaded(source);
    let untouched = point(14, -1).chunk();
    let before_untouched = runtime
        .resident_chunk(untouched)
        .expect("near unrelated chunk");
    assert_eq!(runtime.revision(point(1000, 0).chunk()), None);
    let tx = transaction(&runtime, "remove-tree", Some(a.clone()), None);
    let change = runtime
        .apply_object_transaction(&tx)
        .expect("remove exact one identity");
    assert_eq!(change.revisions.len(), 2);
    assert_eq!(change.changed_columns, vec![point(15, 0), point(16, 0)]);
    for column in [point(15, 0), point(16, 0)] {
        assert_eq!(
            runtime.voxel(voxel(column, 1)),
            QueryResult::Ready(Some("stone".into()))
        );
        let product = runtime
            .resident_chunk(column.chunk())
            .expect("changed chunk");
        assert_eq!(product.package.semantics.object_influences.len(), 1);
        assert_eq!(
            product
                .package
                .semantics
                .object_influences
                .first()
                .expect("survivor")
                .id,
            "authored/other"
        );
    }
    assert_eq!(
        runtime
            .resident_chunk(untouched)
            .expect("unrelated")
            .package,
        before_untouched.package
    );
    let update = runtime.pump();
    assert_eq!(update.changed.len(), 2);
    assert!(update.removed.is_empty());
    load(&mut runtime, vec![interest("right", point(16, 0), 0, 0)]);
    assert_eq!(runtime.revision(a.origin.column.chunk()), None);
    assert_eq!(
        runtime.voxel(voxel(point(16, 0), 2)),
        QueryResult::Ready(Some("stone".into()))
    );
    load(&mut runtime, vec![interest("left", point(15, 0), 0, 0)]);
    assert_eq!(
        runtime.voxel(voxel(point(15, 0), 2)),
        QueryResult::Ready(Some("stone".into()))
    );
}

#[test]
fn moving_root_across_chunks_updates_exact_headroom_and_only_affected_revisions() {
    let (source, a) = fixture(false);
    let mut runtime = loaded(source);
    let tx = transaction(&runtime, "move-tree", Some(a.clone()), Some(moved(&a)));
    let change = runtime.apply_object_transaction(&tx).expect("move");
    assert_eq!(change.revisions.len(), 2);
    assert_eq!(
        runtime.voxel(voxel(point(15, 0), 1)),
        QueryResult::Ready(None)
    );
    assert_eq!(
        runtime.voxel(voxel(point(17, 0), 1)),
        QueryResult::Ready(Some("stone".into()))
    );
    let old = match runtime.surfaces(point(15, 0)) {
        QueryResult::Ready(surfaces) => Some(surfaces),
        _ => None,
    }
    .expect("old surfaces ready");
    assert_eq!(old.first().expect("ground").position.level, 0);
    assert_eq!(old.first().expect("ground").headroom, Some(4));
    let new = match runtime.surfaces(point(17, 0)) {
        QueryResult::Ready(surfaces) => Some(surfaces),
        _ => None,
    }
    .expect("new surfaces ready");
    assert_eq!(new.first().expect("object top").position.level, 2);
    assert_eq!(new.first().expect("object top").headroom, Some(2));
    assert!(runtime
        .resident_chunk(point(15, 0).chunk())
        .expect("old owner")
        .package
        .semantics
        .objects
        .is_empty());
    assert_eq!(
        runtime
            .resident_chunk(point(17, 0).chunk())
            .expect("new owner")
            .package
            .semantics
            .objects,
        vec![moved(&a)]
    );
}

#[test]
fn unavailable_stale_and_forged_before_commands_publish_nothing() {
    let (source, a) = fixture(false);
    let mut runtime = make_runtime(source);
    load(&mut runtime, vec![interest("left", point(15, 0), 0, 0)]);
    let tx = WorldObjectEditTransaction {
        id: "remove".into(),
        expected_revisions: a
            .dependency_chunks()
            .expect("dependencies")
            .into_iter()
            .map(|chunk| (chunk, 0))
            .collect(),
        edits: vec![ObjectEdit {
            before: Some(a.clone()),
            after: None,
        }],
    };
    let before = snapshot(&runtime);
    assert_eq!(
        runtime
            .apply_object_transaction(&tx)
            .expect_err("missing right")
            .kind,
        ErrorKind::Unavailable
    );
    assert_eq!(snapshot(&runtime), before);
    runtime
        .pin(
            "object-operation",
            a.dependency_chunks().expect("dependencies"),
        )
        .expect("request dependencies");
    settle(&mut runtime);
    let before = snapshot(&runtime);
    let mut stale = tx.clone();
    *stale
        .expected_revisions
        .values_mut()
        .last()
        .expect("expected") = 1;
    assert_eq!(
        runtime
            .apply_object_transaction(&stale)
            .expect_err("stale last dependency")
            .kind,
        ErrorKind::Conflict
    );
    let mut forged = tx.clone();
    forged
        .edits
        .first_mut()
        .expect("edit")
        .before
        .as_mut()
        .expect("before")
        .asset = "other-art".into();
    assert_eq!(
        runtime
            .apply_object_transaction(&forged)
            .expect_err("forged complete record")
            .kind,
        ErrorKind::Conflict
    );
    assert_eq!(snapshot(&runtime), before);
    assert!(runtime.pump().changed.is_empty());
    runtime
        .apply_object_transaction(&tx)
        .expect("complete pinned dependencies");
}

#[test]
fn allocated_addition_is_idempotent_and_cannot_reuse_a_deleted_identity() {
    let source = world(&[(point(15, 0), 3)]);
    let mut runtime = loaded(source);
    let a = object(
        &runtime_object_id("add", 0).expect("allocated ID"),
        point(15, 0),
        &[point(15, 0), point(16, 0)],
    );
    let tx = transaction(&runtime, "add", None, Some(a.clone()));
    let change = runtime.apply_object_transaction(&tx).expect("addition");
    assert_eq!(
        runtime.apply_object_transaction(&tx).expect("duplicate"),
        change
    );
    assert!(runtime
        .transaction_delta("add")
        .expect("history")
        .expect("delta")
        .validate()
        .is_ok());
    let delete = transaction(&runtime, "delete", Some(a.clone()), None);
    runtime.apply_object_transaction(&delete).expect("remove");
    let mut replay_as_new = tx.clone();
    replay_as_new.id = "another-add".into();
    replay_as_new
        .expected_revisions
        .values_mut()
        .for_each(|revision| *revision = 2);
    assert!(runtime.apply_object_transaction(&replay_as_new).is_err());
    assert_eq!(
        runtime.voxel(voxel(point(15, 0), 1)),
        QueryResult::Ready(None)
    );
    let mut changed_body = tx;
    changed_body
        .edits
        .first_mut()
        .expect("edit")
        .after
        .as_mut()
        .expect("after")
        .rotation = 2;
    assert_eq!(
        runtime
            .apply_object_transaction(&changed_body)
            .expect_err("ID reused")
            .kind,
        ErrorKind::Conflict
    );
}

#[test]
fn object_delta_durable_replay_and_lazy_restore_preserve_full_record_and_tombstone() {
    let temp = TempRoot::new();
    let (source, a) = fixture(false);
    let mut host = loaded(source.clone());
    let tx = transaction(&host, "move-durable", Some(a.clone()), Some(moved(&a)));
    let owner = AttachmentUpdate {
        owner: "gameplay".into(),
        key: "object-editor".into(),
        expected_fingerprint: None,
        bytes: Some(vec![4, 2]),
    };
    let change = host
        .apply_object_transaction_durable_with_attachments(
            &tx,
            temp.child("host"),
            IoLimits::default(),
            &[owner],
        )
        .expect("durable move and owner state");
    let delta = host
        .transaction_delta(&tx.id)
        .expect("history")
        .expect("delta");
    assert!(delta.chunks.iter().all(|chunk| chunk.columns.is_empty()));
    assert_eq!(delta.object_edits, tx.edits);
    let mut replica = loaded(source.clone());
    assert_eq!(
        replica
            .apply_delta_durable(&delta, temp.child("replica"), IoLimits::default())
            .expect("durable replica ACK"),
        change
    );
    assert_eq!(snapshot(&replica), snapshot(&host));
    let mut restored = make_runtime(source.clone());
    restored
        .restore_save(temp.child("host"), IoLimits::default())
        .expect("metadata-only restore");
    assert_eq!(restored.counts().resident_chunks, 0);
    assert_eq!(
        restored
            .transaction_delta(&tx.id)
            .expect("paged history")
            .expect("delta"),
        delta
    );
    assert_eq!(
        restored
            .apply_object_transaction(&tx)
            .expect("retry without loading dependencies"),
        change
    );
    assert_eq!(
        restored
            .attachment("gameplay", "object-editor")
            .expect("owner bytes")
            .expect("attachment")
            .bytes,
        vec![4, 2]
    );
    load(&mut restored, vec![interest("right", point(17, 0), 0, 0)]);
    assert_eq!(
        restored.voxel(voxel(point(17, 0), 2)),
        QueryResult::Ready(Some("stone".into()))
    );
    let delete = transaction(&restored, "delete-durable", Some(moved(&a)), None);
    restored
        .apply_object_transaction_durable(&delete, temp.child("host"), IoLimits::default())
        .expect("durable tombstone");
    let mut deleted = make_runtime(source);
    deleted
        .restore_save(temp.child("host"), IoLimits::default())
        .expect("reopen tombstone");
    load(&mut deleted, vec![interest("right", point(17, 0), 0, 0)]);
    assert_eq!(
        deleted.voxel(voxel(point(17, 0), 2)),
        QueryResult::Ready(None)
    );
    assert!(deleted
        .resident_chunk(point(17, 0).chunk())
        .expect("deleted owner")
        .package
        .semantics
        .objects
        .is_empty());
}

#[test]
fn checkpoint_failure_and_forged_delta_are_failure_atomic() {
    let temp = TempRoot::new();
    let bad = temp.child("file-not-directory");
    fs::write(&bad, b"blocked").expect("blocking file");
    let (source, a) = fixture(false);
    let mut runtime = loaded(source.clone());
    let tx = transaction(&runtime, "move", Some(a.clone()), Some(moved(&a)));
    let before = snapshot(&runtime);
    assert!(runtime
        .apply_object_transaction_durable(&tx, &bad, IoLimits::default())
        .is_err());
    assert_eq!(snapshot(&runtime), before);
    assert!(runtime
        .transaction_delta("move")
        .expect("history")
        .is_none());
    runtime.apply_object_transaction(&tx).expect("host result");
    let mut delta = runtime
        .transaction_delta("move")
        .expect("history")
        .expect("delta");
    delta
        .chunks
        .last_mut()
        .expect("last chunk")
        .target_fingerprint ^= 1;
    delta.fingerprint = 0;
    delta.fingerprint = hash_serializable(&delta).expect("reseal forged message");
    let mut replica = loaded(source);
    let before = snapshot(&replica);
    assert!(replica.apply_delta(&delta).is_err());
    assert_eq!(snapshot(&replica), before);
    assert!(replica
        .transaction_delta("move")
        .expect("history")
        .is_none());
    delta.chunks.first_mut().expect("chunk").base_revision = u64::MAX;
    delta.chunks.first_mut().expect("chunk").revision = 0;
    delta.fingerprint = 0;
    delta.fingerprint = hash_serializable(&delta).expect("hash");
    assert!(delta.validate().is_err());
}

#[test]
fn object_and_terrain_partition_overlays_compose_in_both_orders() {
    let temp = TempRoot::new();
    let source = world(&[(point(15, 0), 3)]);
    let mut runtime = loaded(source.clone());
    runtime
        .apply_transaction(&edit(
            "terrain-before",
            voxel(point(14, 0), 3),
            0,
            Some("stone"),
        ))
        .expect("terrain first");
    let a = object(
        &runtime_object_id("object-between", 0).expect("ID"),
        point(15, 0),
        &[point(15, 0)],
    );
    let tx = transaction(&runtime, "object-between", None, Some(a));
    runtime
        .apply_object_transaction(&tx)
        .expect("object second");
    runtime
        .apply_transaction(&edit(
            "terrain-after",
            voxel(point(13, 0), 3),
            2,
            Some("stone"),
        ))
        .expect("terrain preserves object overlay");
    runtime
        .save(temp.child("save"), IoLimits::default())
        .expect("checkpoint");
    let mut restored = make_runtime(source);
    restored
        .restore_save(temp.child("save"), IoLimits::default())
        .expect("restore");
    load(&mut restored, vec![interest("near", point(15, 0), 3, 3)]);
    assert_eq!(snapshot(&restored), snapshot(&runtime));
    for position in [
        voxel(point(14, 0), 3),
        voxel(point(13, 0), 3),
        voxel(point(15, 0), 1),
    ] {
        assert_eq!(
            restored.voxel(position),
            QueryResult::Ready(Some("stone".into()))
        );
    }
}

#[test]
fn incompatible_overlap_anchor_interior_and_boundary_changes_have_named_refusals() {
    let (source, a) = fixture(true);
    let mut runtime = loaded(source);
    let mut after = a.clone();
    after.occupancy.iter_mut().for_each(|column| {
        column
            .runs
            .iter_mut()
            .for_each(|run| run.material = "water".into())
    });
    let tx = transaction(&runtime, "conflicting-material", Some(a), Some(after));
    let before = snapshot(&runtime);
    let error = runtime
        .apply_object_transaction(&tx)
        .expect_err("overlap mismatch");
    assert!(error.message.contains("overlap"));
    assert_eq!(snapshot(&runtime), before);
    for interior in [false, true] {
        let mut source = world(&[(point(15, 0), 3)]);
        let semantics = &mut source
            .chunks
            .get_mut(&point(15, 0).chunk())
            .expect("chunk")
            .semantics;
        if interior {
            semantics.interiors.push(InteriorSpan {
                id: "room".into(),
                column: point(15, 0),
                floor_level: 0,
                roof_bottom: 5,
                roof_top: 7,
                light_domain: "room-light".into(),
            });
        } else {
            semantics.anchors.push(WorldAnchor {
                id: "operation-entry".into(),
                region_id: "region-0000".into(),
                position: voxel(point(15, 0), 0),
                role: AnchorRole::Gameplay,
            });
        }
        source.seal().expect("protected semantic fixture");
        let mut runtime = loaded(source);
        let a = object(
            &runtime_object_id("protected-add", 0).expect("ID"),
            point(15, 0),
            &[point(15, 0)],
        );
        let tx = transaction(&runtime, "protected-add", None, Some(a));
        let before = snapshot(&runtime);
        let error = runtime
            .apply_object_transaction(&tx)
            .expect_err("protected semantics");
        assert!(error.message.contains(if interior {
            "interior regeneration"
        } else {
            "protected anchor"
        }));
        assert_eq!(snapshot(&runtime), before);
    }
    let mut source = world(&[(point(15, 0), 0), (point(16, 0), 0)]);
    source.manifest.boundaries.push(BoundaryContract {
        id: "seam".into(),
        region_a: "region-0000".into(),
        region_b: "region-0001".into(),
        samples: vec![BoundarySample {
            a: point(15, 0),
            b: point(16, 0),
            ground_level: 0,
            water_level: None,
            required_access: true,
        }],
    });
    source.seal().expect("boundary");
    let mut runtime = loaded(source);
    let a = object(
        &runtime_object_id("seam-add", 0).expect("ID"),
        point(15, 0),
        &[point(15, 0)],
    );
    let tx = transaction(&runtime, "seam-add", None, Some(a));
    let before = snapshot(&runtime);
    assert!(runtime
        .apply_object_transaction(&tx)
        .expect_err("seam blocked")
        .message
        .contains("protected boundary"));
    assert_eq!(snapshot(&runtime), before);
}

#[test]
fn identity_only_replacement_updates_every_projection_and_survives_lazy_reload() {
    let temp = TempRoot::new();
    let (source, a) = fixture(true);
    let mut runtime = loaded(source.clone());
    let prior_voxel = runtime.voxel(voxel(point(16, 0), 2));
    let mut after = a.clone();
    after.asset = "tree.changed".into();
    let tx = transaction(&runtime, "change-art-record", Some(a), Some(after.clone()));
    let change = runtime
        .apply_object_transaction_durable(&tx, temp.child("save"), IoLimits::default())
        .expect("identity-only source replacement");
    assert_eq!(change.revisions.len(), 2);
    assert_eq!(runtime.voxel(voxel(point(16, 0), 2)), prior_voxel);
    let mut restored = make_runtime(source);
    restored
        .restore_save(temp.child("save"), IoLimits::default())
        .expect("reopen");
    load(&mut restored, vec![interest("foreign", point(16, 0), 0, 0)]);
    let product = restored
        .resident_chunk(point(16, 0).chunk())
        .expect("foreign product");
    assert!(product.package.semantics.objects.is_empty());
    assert_eq!(
        product
            .package
            .semantics
            .object_influences
            .iter()
            .find(|row| row.id == after.id)
            .expect("changed identity")
            .source_fingerprint,
        hash_serializable(&after).expect("full record hash")
    );
    assert_eq!(restored.voxel(voxel(point(16, 0), 2)), prior_voxel);
}

#[test]
fn object_operation_limits_reject_before_any_chunk_or_history_change() {
    for config in [
        RuntimeConfig {
            max_edits_per_transaction: 1,
            ..RuntimeConfig::default()
        },
        RuntimeConfig {
            max_transaction_bytes: 128,
            ..RuntimeConfig::default()
        },
    ] {
        let (source, a) = fixture(false);
        let mut runtime = WorldRuntime::new(
            Arc::new(MemoryChunkSource::new(source).expect("source")),
            config,
        )
        .expect("runtime");
        load(&mut runtime, vec![interest("near", point(15, 0), 3, 3)]);
        let tx = transaction(&runtime, "bounded-object", Some(a), None);
        let before = snapshot(&runtime);
        assert_eq!(
            runtime
                .apply_object_transaction(&tx)
                .expect_err("operation budget")
                .kind,
            ErrorKind::Limit
        );
        assert_eq!(snapshot(&runtime), before);
        assert!(runtime
            .transaction_delta("bounded-object")
            .expect("history")
            .is_none());
    }
}

struct ReservedSource(WorldPackage);
impl ChunkSource for ReservedSource {
    fn manifest(&self) -> &WorldManifest {
        &self.0.manifest
    }
    fn load_chunk(&self, coordinate: ChunkId) -> RuntimeResult<ChunkPackage> {
        self.0
            .chunks
            .get(&coordinate)
            .cloned()
            .ok_or_else(|| RuntimeError {
                kind: ErrorKind::Unavailable,
                message: "missing fixture chunk".into(),
            })
    }
}

#[test]
fn compiled_source_cannot_smuggle_reserved_ids_through_a_foreign_projection() {
    let (mut source, mut a) = fixture(false);
    a.id = runtime_object_id("future-add", 0).expect("reserved ID");
    for (coordinate, projection) in a.influences().expect("projection") {
        let chunk = source.chunks.get_mut(&coordinate).expect("chunk");
        chunk.semantics.objects = if coordinate == a.origin.column.chunk() {
            vec![a.clone()]
        } else {
            Vec::new()
        };
        chunk.semantics.object_influences = vec![projection];
        chunk.semantics.occupancy =
            union_object_occupancy(&chunk.semantics.object_influences).expect("occupancy");
        chunk.seal().expect("valid runtime shape");
    }
    for descriptor in &mut source.manifest.chunks {
        descriptor.fingerprint = source
            .chunks
            .get(&descriptor.coordinate)
            .expect("chunk")
            .fingerprint;
    }
    source
        .manifest
        .seal()
        .expect("integrity-valid malicious source catalogue");
    let mut runtime = WorldRuntime::new(Arc::new(ReservedSource(source)), RuntimeConfig::default())
        .expect("metadata");
    runtime
        .set_interests(vec![interest("foreign", point(16, 0), 0, 0)])
        .expect("interest");
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut failures = Vec::new();
    loop {
        let update = runtime.pump();
        assert!(update.loaded.is_empty());
        failures.extend(update.failures);
        let counts = runtime.counts();
        if counts.in_flight_jobs == 0 && counts.queued_chunks == 0 {
            break;
        }
        assert!(Instant::now() < deadline, "source failure did not settle");
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(failures.len(), 1);
    assert!(failures
        .first()
        .expect("rejection")
        .error
        .message
        .contains("reserved runtime"));
    assert_eq!(
        runtime.voxel(voxel(point(16, 0), 1)),
        QueryResult::Unloaded(point(16, 0).chunk())
    );
}
