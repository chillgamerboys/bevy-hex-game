use super::*;

fn finite_fixture() -> WorldRuntime {
    let mut source = world(&[(point(0, 0), 2)]);
    let root = source.chunks.get_mut(&point(0, 0).chunk()).expect("root");
    root.semantics.objects.push(ObjectInstance {
        id: "tree".into(),
        region_id: "region-0000".into(),
        asset: "tree.oak".into(),
        origin: voxel(point(0, 0), 1),
        rotation: 0,
        grounding: None,
        occupancy: vec![ColumnData {
            position: point(0, 0),
            runs: vec![run(1, 4, "stone")],
        }],
    });
    let wet = root
        .columns
        .iter_mut()
        .find(|c| c.position == point(1, 0))
        .expect("water column");
    wet.runs.insert(2, run(1, 3, "water"));
    source.seal().expect("initial strict source");
    let mut runtime = make_runtime(source);
    load(&mut runtime, vec![interest("all", point(0, 0), 2, 2)]);
    runtime
}

#[test]
fn finite_checkpoint_preserves_unloaded_carves_refills_and_permissive_semantics() {
    let mut runtime = finite_fixture();
    let original = resident_fingerprints(&runtime);
    let mut session = FiniteWorldSession::streamed(&runtime, -4, 100).expect("session");
    let tree = voxel(point(0, 0), 1);
    session
        .apply_transaction(&edit("carve-tree", tree, 0, None))
        .expect("carve");
    session
        .apply_transaction(&edit("refill-tree", tree, 1, Some("stone")))
        .expect("refill");
    session
        .apply_transaction(&edit("bedrock", voxel(point(0, 0), -4), 2, None))
        .expect("arena can carve bedrock");
    session
        .apply_transaction(&edit("other-chunk", voxel(point(-1, -1), 0), 0, None))
        .expect("second partition");
    let header = session.checkpoint_header();
    let partitions: Vec<_> = session
        .checkpoint_partitions()
        .collect::<RuntimeResult<_>>()
        .expect("sparse export");
    assert_eq!(header.edited_chunks.len(), 2);
    assert_eq!(
        partitions
            .iter()
            .map(|p| p.terrain_edits.len())
            .sum::<usize>(),
        3
    );
    assert_eq!(resident_fingerprints(&runtime), original);
    runtime.set_interests(vec![]).expect("unload");
    runtime.pump();
    session.sync_residency(&runtime);
    assert_eq!(session.resident_source_count(), 0);
    assert_eq!(
        session
            .checkpoint_partitions()
            .collect::<RuntimeResult<Vec<_>>>()
            .expect("export unloaded edits"),
        partitions
    );
    let mut restored = FiniteWorldSession::restore_checkpoint(
        &runtime,
        -4,
        100,
        &header,
        partitions.clone().into_iter().map(Ok),
        &CancellationToken::default(),
    )
    .expect("staged restore");
    assert_eq!(
        restored.resident_source_count(),
        0,
        "validation does not load whole world into residency"
    );
    assert_eq!(runtime.resident_chunks().count(), 0);
    load(&mut runtime, vec![interest("back", point(0, 0), 2, 2)]);
    restored.sync_residency(&runtime);
    assert_eq!(restored.terrain_at(tree), Some("stone"));
    assert!(restored.object_removed(tree));
    assert_eq!(restored.object_at(tree), None);
    assert_eq!(restored.material_at(voxel(point(0, 0), -4)), None);
    assert_eq!(
        restored.material_at(voxel(point(0, 0), 2)),
        Some("stone"),
        "unsupported crown survives"
    );
    assert_eq!(restored.revision(point(0, 0).chunk()), Some(3));
    assert!(restored
        .apply_transaction(&edit("stale", tree, 0, None))
        .is_err());
    restored
        .apply_transaction(&edit("continued-unique-counter", tree, 3, None))
        .expect("new request after resume");
    assert_eq!(restored.revision(point(0, 0).chunk()), Some(4));
    assert_eq!(resident_fingerprints(&runtime), original);
}

#[test]
fn finite_restore_rejects_incomplete_malformed_liquid_and_mismatched_source_atomically() {
    let runtime = finite_fixture();
    let mut session = FiniteWorldSession::streamed(&runtime, -4, 100).expect("session");
    let tree = voxel(point(0, 0), 1);
    session
        .apply_transaction(&edit("cut", tree, 0, None))
        .expect("cut");
    let header = session.checkpoint_header();
    let partition = session
        .checkpoint_partitions()
        .next()
        .expect("partition")
        .expect("export");
    let restore = |header: &FiniteSessionHeader, partitions: Vec<FiniteChunkCheckpoint>| {
        FiniteWorldSession::restore_checkpoint(
            &runtime,
            -4,
            100,
            header,
            partitions.into_iter().map(Ok),
            &CancellationToken::default(),
        )
    };
    assert!(restore(&header, vec![]).is_err());
    assert!(restore(&header, vec![partition.clone(), partition.clone()]).is_err());
    let mut bad = partition.clone();
    bad.base_fingerprint ^= 1;
    assert!(restore(&header, vec![bad]).is_err());
    let mut bad = partition.clone();
    bad.object_removed.clear();
    assert!(restore(&header, vec![bad]).is_err());
    let mut bad = partition.clone();
    bad.terrain_edits
        .push(bad.terrain_edits.first().expect("edit").clone());
    assert!(restore(&header, vec![bad]).is_err());
    let mut bad = partition.clone();
    bad.terrain_edits = vec![VoxelEdit {
        position: voxel(point(1, 0), 1),
        material: None,
    }];
    bad.object_removed.clear();
    assert!(
        restore(&header, vec![bad]).is_err(),
        "liquids cannot be carved by forged saves"
    );
    let mut bad = partition.clone();
    bad.terrain_edits.first_mut().expect("edit").material = Some("water".into());
    assert!(
        restore(&header, vec![bad]).is_err(),
        "liquids cannot be constructed"
    );
    let mut wrong = header.clone();
    wrong.manifest_fingerprint ^= 1;
    assert!(restore(&wrong, vec![partition.clone()]).is_err());
    wrong = header.clone();
    wrong.top += 1;
    assert!(restore(&wrong, vec![partition.clone()]).is_err());
    let cancelled = CancellationToken::default();
    cancelled.cancel();
    assert!(FiniteWorldSession::restore_checkpoint(
        &runtime,
        -4,
        100,
        &header,
        [Ok(partition)],
        &cancelled
    )
    .is_err());
    assert_eq!(
        session.material_at(tree),
        None,
        "existing live session untouched"
    );
    assert_eq!(session.revision(tree.column.chunk()), Some(1));
}

#[test]
fn finite_restore_checks_actual_source_bytes_and_revision_exhaustion() {
    let package = world(&[(point(0, 0), 1)]);
    let mut runtime = make_runtime(package.clone());
    load(&mut runtime, vec![interest("source", point(0, 0), 1, 1)]);
    let at = voxel(point(0, 0), 0);
    let mut finite = FiniteWorldSession::streamed(&runtime, -4, 100).expect("finite");
    finite
        .apply_transaction(&edit("cut", at, 0, None))
        .expect("cut");
    let header = finite.checkpoint_header();
    let mut partition = finite
        .checkpoint_partitions()
        .next()
        .expect("partition")
        .expect("export");
    struct CorruptSource(WorldPackage);
    impl ChunkSource for CorruptSource {
        fn manifest(&self) -> &WorldManifest {
            &self.0.manifest
        }
        fn load_chunk(&self, chunk: ChunkId) -> RuntimeResult<ChunkPackage> {
            let mut package = self.0.chunks.get(&chunk).expect("fixture chunk").clone();
            package.columns.first_mut().expect("column").runs.clear();
            Ok(package)
        }
    }
    let corrupt = WorldRuntime::new(Arc::new(CorruptSource(package)), RuntimeConfig::default())
        .expect("manifest remains valid");
    assert!(
        FiniteWorldSession::restore_checkpoint(
            &corrupt,
            -4,
            100,
            &header,
            [Ok(partition.clone())],
            &CancellationToken::default()
        )
        .is_err(),
        "actual chunk bytes must pass source validation"
    );
    partition.revision = u64::MAX;
    let mut restored = FiniteWorldSession::restore_checkpoint(
        &runtime,
        -4,
        100,
        &header,
        [Ok(partition)],
        &CancellationToken::default(),
    )
    .expect("last representable revision");
    assert!(restored
        .apply_transaction(&edit("overflow", at, u64::MAX, Some("stone")))
        .is_err());
    assert_eq!(
        restored.material_at(at),
        None,
        "overflow rejected before mutating terrain"
    );
    assert_eq!(restored.revision(at.column.chunk()), Some(u64::MAX));
}

#[test]
fn finite_mixed_carve_is_atomic_sparse_idempotent_and_leaves_source_unchanged() {
    let runtime = finite_fixture();
    let fingerprints = resident_fingerprints(&runtime);
    let mut finite = FiniteWorldSession::new(&runtime, -4, 100).expect("finite session");
    let positions = [
        voxel(point(0, 0), -4),
        voxel(point(0, 0), 0),
        voxel(point(0, 0), 1),
    ];
    let tx = WorldEditTransaction {
        id: "mixed".into(),
        expected_revisions: BTreeMap::from([(point(0, 0).chunk(), 0)]),
        edits: positions
            .into_iter()
            .map(|position| VoxelEdit {
                position,
                material: None,
            })
            .collect(),
    };
    let accepted = finite
        .apply_transaction(&tx)
        .expect("terrain bedrock and tree in one carve");
    for at in positions {
        assert_eq!(finite.material_at(at), None);
    }
    assert!(finite.object_removed(voxel(point(0, 0), 1)));
    assert_eq!(
        finite.material_at(voxel(point(0, 0), 2)),
        Some("stone"),
        "unsupported crown remains"
    );
    assert_eq!(finite.apply_transaction(&tx).expect("duplicate"), accepted);
    assert_eq!(
        resident_fingerprints(&runtime),
        fingerprints,
        "source immutable"
    );
    let reset = FiniteWorldSession::new(&runtime, -4, 100).expect("reset");
    assert_eq!(reset.material_at(voxel(point(0, 0), -4)), Some("bedrock"));
    assert_eq!(reset.material_at(voxel(point(0, 0), 1)), Some("stone"));
}

#[test]
fn finite_liquid_and_bounds_rejection_does_not_partially_carve() {
    let runtime = finite_fixture();
    let mut finite = FiniteWorldSession::new(&runtime, -4, 100).expect("finite session");
    let solid = voxel(point(0, 0), 0);
    let wet = voxel(point(1, 0), 1);
    let mut edits = vec![
        VoxelEdit {
            position: solid,
            material: None,
        },
        VoxelEdit {
            position: wet,
            material: None,
        },
    ];
    edits.sort_by_key(|e| e.position);
    let tx = WorldEditTransaction {
        id: "wet-blast".into(),
        expected_revisions: BTreeMap::from([(point(0, 0).chunk(), 0)]),
        edits,
    };
    assert!(finite.apply_transaction(&tx).is_err());
    assert_eq!(finite.material_at(solid), Some("stone"));
    assert_eq!(finite.material_at(wet), Some("water"));
    assert!(finite
        .apply_transaction(&edit("low", voxel(point(0, 0), -5), 0, None))
        .is_err());
    assert!(finite
        .apply_transaction(&edit("high", voxel(point(0, 0), 101), 0, Some("stone")))
        .is_err());
    assert_eq!(finite.revision(point(0, 0).chunk()), Some(0));
}

#[test]
fn finite_refill_is_terrain_and_does_not_resurrect_carved_object() {
    let runtime = finite_fixture();
    let mut finite = FiniteWorldSession::new(&runtime, -4, 100).expect("finite session");
    let at = voxel(point(0, 0), 1);
    finite
        .apply_transaction(&edit("cut", at, 0, None))
        .expect("cut");
    finite
        .apply_transaction(&edit("shield", at, 1, Some("stone")))
        .expect("refill");
    assert_eq!(finite.terrain_at(at), Some("stone"));
    assert_eq!(finite.object_at(at), None);
    assert!(finite.object_removed(at));
    let column = finite.terrain_column(at.column).expect("terrain column");
    assert_eq!(column.material_at(at.level), Some("stone"));
    assert!(finite
        .apply_transaction(&edit("stale", at, 0, None))
        .is_err());
    assert!(finite
        .apply_transaction(&edit("cut", at, 0, Some("stone")))
        .is_err());
}

#[test]
fn streamed_session_releases_sources_but_keeps_carves_across_reload() {
    let mut runtime = finite_fixture();
    let mut session = FiniteWorldSession::streamed(&runtime, -4, 100).expect("streamed overlay");
    let at = voxel(point(0, 0), 1);
    session
        .apply_transaction(&WorldEditTransaction {
            id: "stream-cut".into(),
            expected_revisions: BTreeMap::from([(at.column.chunk(), 0)]),
            edits: vec![VoxelEdit {
                position: at,
                material: None,
            }],
        })
        .expect("carve");
    runtime.set_interests(vec![]).expect("retire interests");
    runtime.pump();
    session.sync_residency(&runtime);
    assert_eq!(session.resident_source_count(), 0);
    assert_eq!(session.revision(at.column.chunk()), Some(1));
    load(&mut runtime, vec![interest("return", point(0, 0), 2, 2)]);
    session.sync_residency(&runtime);
    assert_eq!(session.material_at(at), None);
    assert_eq!(session.material_at(voxel(point(0, 0), 2)), Some("stone"));
    let rendered = session
        .presentation_package(at.column.chunk())
        .expect("render projection");
    rendered
        .validate_against_manifest(runtime.manifest())
        .expect("valid surviving geometry");
    assert_eq!(
        rendered
            .columns
            .iter()
            .find(|c| c.position == at.column)
            .and_then(|c| c.material_at(at.level)),
        None
    );
    let reset = FiniteWorldSession::streamed(&runtime, -4, 100).expect("restart");
    assert_eq!(reset.material_at(at), Some("stone"));
}

#[test]
fn streamed_session_accepts_partial_residency_and_preserves_liquid() {
    let runtime = finite_fixture();
    let mut session = FiniteWorldSession::streamed(&runtime, -4, 100).expect("overlay");
    let at = voxel(point(1, 0), 1);
    let tx = WorldEditTransaction {
        id: "water".into(),
        expected_revisions: BTreeMap::from([(at.column.chunk(), 0)]),
        edits: vec![VoxelEdit {
            position: at,
            material: None,
        }],
    };
    assert!(session.apply_transaction(&tx).is_err());
    assert_eq!(session.terrain_at(at), Some("water"));
}

#[test]
fn finite_column_edit_query_has_half_open_bounds_and_retains_refill_provenance() {
    let mut runtime = finite_fixture();
    let mut session = FiniteWorldSession::streamed(&runtime, -4, 100).unwrap();
    let at = voxel(point(0, 0), 1);
    session
        .apply_transaction(&edit("query-carve", at, 0, None))
        .unwrap();
    session
        .apply_transaction(&edit("query-refill", at, 1, Some("stone")))
        .unwrap();
    let check = |session: &FiniteWorldSession| {
        assert!(session.terrain_edited_in_column(at.column, 1, 2));
        assert!(session.terrain_edited_in_column(at.column, i32::MIN, i32::MAX));
        assert!(!session.terrain_edited_in_column(at.column, 0, 1));
        assert!(!session.terrain_edited_in_column(at.column, 2, 3));
        assert!(!session.terrain_edited_in_column(at.column, 1, 1));
        assert!(!session.terrain_edited_in_column(at.column, 2, 1));
        assert!(!session.terrain_edited_in_column(point(0, 1), 1, 2));
    };
    check(&session);
    let header = session.checkpoint_header();
    let partitions = session
        .checkpoint_partitions()
        .collect::<RuntimeResult<Vec<_>>>()
        .unwrap();
    runtime.set_interests(vec![]).unwrap();
    runtime.pump();
    session.sync_residency(&runtime);
    check(&session);
    let restored = FiniteWorldSession::restore_checkpoint(
        &runtime,
        -4,
        100,
        &header,
        partitions.into_iter().map(Ok),
        &CancellationToken::default(),
    )
    .unwrap();
    check(&restored);
    assert_eq!(restored.resident_source_count(), 0);
}
