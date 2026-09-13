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
