//! The streamed visual adapter must keep raised liquids without duplicating the ocean.
use super::*;
use bevy::mesh::VertexAttributeValues;
use hex_core::arena::{ArenaTerrainView, ArenaVoxelGeometry};
use hex_core::{HexCoord, SubstanceId, TilePos};
use hex_world_contracts::{
    ChunkDescriptor, ChunkSemantics, LiquidColumn, LiquidKind, MaterialSpec, RegionDescriptor,
    ResidencyRequest, VoxelEdit, VoxelPosition, WorldEditTransaction, WorldHex, WorldManifest,
    WorldPackage, SCHEMA_VERSION,
};
use hex_world_runtime::{FiniteWorldSession, MemoryChunkSource, RuntimeConfig, WorldRuntime};
use std::time::{Duration, Instant};

#[expect(
    clippy::expect_used,
    reason = "The bounded in-memory fixture must pass the production package and residency validators before testing its visual adapter."
)]
fn fixture() -> StreamedArena {
    let chunk = ChunkId { q: 0, r: 0 };
    // Sea, a raised cascade and its lower receiving pool, one level above sea,
    // and a submerged interval. The adjacent cascade bodies share a vertical face.
    let levels = [
        (0, 396, 400),
        (1, 604, 616),
        (2, 600, 608),
        (3, 400, 401),
        (4, 390, 399),
    ];
    let columns = levels
        .iter()
        .map(|&(q, bottom, top)| ColumnData {
            position: WorldHex::new(q, 0),
            runs: vec![
                VoxelRun {
                    bottom: 0,
                    top: bottom,
                    material: "rock".into(),
                },
                VoxelRun {
                    bottom,
                    top,
                    material: "water".into(),
                },
            ],
        })
        .collect();
    let liquids = levels
        .iter()
        .map(|&(q, bottom, top)| LiquidColumn {
            column: WorldHex::new(q, 0),
            bottom,
            top,
            kind: LiquidKind::Standing,
            body_id: "fixture/water".into(),
            downstream: vec![],
        })
        .collect();
    let mut package = WorldPackage {
        manifest: WorldManifest {
            schema_version: SCHEMA_VERSION,
            world_id: "liquid-fixture".into(),
            compiler_version: "tests-v1".into(),
            source_fingerprint: 12,
            materials: vec![
                MaterialSpec {
                    id: "rock".into(),
                    solid: true,
                    diggable: true,
                    color: [90, 90, 90, 255],
                },
                MaterialSpec {
                    id: "water".into(),
                    solid: false,
                    diggable: false,
                    color: [37, 104, 150, 180],
                },
            ],
            regions: levels
                .iter()
                .map(|&(q, _, _)| RegionDescriptor {
                    id: format!("fixture/{q}"),
                    origin: WorldHex::new(q, 0),
                    radius: 0,
                    source_fingerprint: 12,
                })
                .collect(),
            chunks: vec![ChunkDescriptor {
                coordinate: chunk,
                fingerprint: 0,
                path: "chunks/0_0.ron".into(),
            }],
            boundaries: vec![],
            summary: vec![],
            features: vec![],
            fingerprint: 0,
        },
        chunks: BTreeMap::from([(
            chunk,
            ChunkPackage {
                schema_version: SCHEMA_VERSION,
                world_id: "liquid-fixture".into(),
                coordinate: chunk,
                source_fingerprint: 12,
                columns,
                features: vec![],
                semantics: ChunkSemantics {
                    liquids,
                    ..default()
                },
                fingerprint: 0,
            },
        )]),
    };
    package.seal().expect("liquid fixture package");
    let source = Arc::new(MemoryChunkSource::new(package).expect("memory source"));
    let mut runtime = WorldRuntime::new(source, RuntimeConfig::default()).expect("runtime");
    runtime
        .set_interests(vec![ResidencyRequest {
            id: "fixture".into(),
            center: WorldHex::new(0, 0),
            radius: 0,
            retention_radius: 0,
            priority: 255,
        }])
        .expect("one chunk interest");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(runtime.pump().failures.is_empty());
        if runtime.resident_chunk(chunk).is_some() {
            break;
        }
        assert!(Instant::now() < deadline, "bounded fixture chunk admission");
        std::thread::sleep(Duration::from_millis(1));
    }
    let edits = FiniteWorldSession::streamed(&runtime, 0, 4096).expect("finite overlay");
    StreamedArena {
        runtime,
        edits,
        overview: Arc::new(super::tests::planar_overview()),
        biomes: None,
        generation: 0,
        projected: BTreeMap::new(),
        interest_key: None,
        next_transaction: 0,
        policies: BTreeMap::from([
            ("rock".into(), SubstanceId(1)),
            ("water".into(), SubstanceId(2)),
        ]),
        failure: None,
        publication_ms: 0.0,
        peak_resident: 0,
    }
}

#[test]
fn streamed_raised_water_survives_visual_admission_edit_and_retirement_without_a_second_sea() {
    let mut state = fixture();
    let chunk = ChunkId { q: 0, r: 0 };
    let geometry = ArenaVoxelGeometry {
        level_height: 0.35,
        vertical_offset: 0.35,
        ..default()
    };
    let mut view = ArenaTerrainView::default();
    super::super::publish(&mut state, &mut view, &geometry);
    let expected_liquids = view.liquids.clone();
    assert_eq!(
        expected_liquids.len(),
        5,
        "both local water and ocean remain authoritative liquid"
    );
    for span in &expected_liquids {
        assert!(
            view.solid_at(span.bottom).is_none(),
            "water cannot become collision footing"
        );
        assert!(view
            .solid_at(TilePos::new(span.bottom.coord, span.top_level))
            .is_none());
    }
    let visual = visual_package(&state, chunk, &BTreeMap::new()).expect("visual package");
    let wet_columns: Vec<_> = visual
        .columns
        .iter()
        .filter(|column| column.runs.iter().any(|run| run.material == "water"))
        .map(|column| column.position)
        .collect();
    assert_eq!(wet_columns, vec![WorldHex::new(1, 0), WorldHex::new(2, 0), WorldHex::new(3, 0)],
        "raised cascade, pool and one-level-above-sea survive; ocean and submerged intervals do not duplicate its surface");
    let mut presenter =
        TerrainPresenter::new(state.runtime.manifest(), RenderOrigin::default(), 0.35)
            .expect("presenter");
    let mut world = World::new();
    let prepared = presenter
        .prepare(&visual, 1)
        .expect("prepared raised water");
    let first = presenter
        .publish(&mut world, prepared)
        .expect("water publication");
    assert!(first.vertices > 0);
    let handles: Vec<_> = world
        .query::<(&Mesh3d, &MeshMaterial3d<StandardMaterial>)>()
        .iter(&world)
        .map(|(mesh, material)| (mesh.0.clone(), material.0.clone()))
        .collect();
    let water_mesh = handles
        .iter()
        .find_map(|(mesh, material)| {
            let material = world.resource::<Assets<StandardMaterial>>().get(material)?;
            matches!(material.alpha_mode, AlphaMode::Blend)
                .then(|| world.resource::<Assets<Mesh>>().get(mesh))
                .flatten()
        })
        .expect("actual translucent water mesh");
    let VertexAttributeValues::Float32x3(positions) = water_mesh
        .attribute(Mesh::ATTRIBUTE_POSITION)
        .expect("positions")
    else {
        panic!("float3 water positions")
    };
    let VertexAttributeValues::Float32x3(normals) = water_mesh
        .attribute(Mesh::ATTRIBUTE_NORMAL)
        .expect("normals")
    else {
        panic!("float3 water normals")
    };
    let wall_x = (HexCoord::from_axial(1, 0).to_world(0.0).x
        + HexCoord::from_axial(2, 0).to_world(0.0).x)
        * 0.5;
    let wall_heights: Vec<_> = positions
        .iter()
        .zip(normals)
        .filter_map(|(position, normal)| {
            let p = Vec3::from(*position);
            let n = Vec3::from(*normal);
            ((p.x - wall_x).abs() < 0.00001 && n.x > 0.99 && n.y.abs() < 0.00001).then_some(p.y)
        })
        .collect();
    assert!(
        !wall_heights.is_empty(),
        "raised cascade keeps its exposed vertical curtain"
    );
    assert!(wall_heights
        .iter()
        .any(|y| (*y - 608.0 * 0.35).abs() < 0.0001));
    assert!(wall_heights
        .iter()
        .any(|y| (*y - 616.0 * 0.35).abs() < 0.0001));
    assert!(
        wall_heights.iter().all(|y| *y >= 608.0 * 0.35 - 0.0001),
        "the lower same-material neighbor hides only their overlapping internal wall"
    );

    state
        .edits
        .apply_transaction(&WorldEditTransaction {
            id: "carve-bank".into(),
            expected_revisions: BTreeMap::from([(chunk, 0)]),
            edits: vec![VoxelEdit {
                position: VoxelPosition {
                    column: WorldHex::new(1, 0),
                    level: 603,
                },
                material: None,
            }],
        })
        .expect("real finite solid edit");
    super::super::publish(&mut state, &mut view, &geometry);
    assert_eq!(
        view.liquids, expected_liquids,
        "a solid rebuild cannot consume immutable liquid"
    );
    let revised = visual_package(&state, chunk, &BTreeMap::new()).expect("revised visual package");
    assert_eq!(
        revised
            .columns
            .iter()
            .flat_map(|c| c.runs.iter())
            .filter(|r| r.material == "water")
            .count(),
        3
    );
    let prepared = presenter
        .prepare(&revised, 2)
        .expect("revised water geometry");
    presenter
        .publish(&mut world, prepared)
        .expect("replace water chunk");
    assert_eq!(
        presenter.receipts().count(),
        1,
        "replacement cannot leak a second water root"
    );
    assert!(
        world.get_entity(first.root).is_err(),
        "old geometry retired during replacement"
    );
    presenter
        .remove(&mut world, chunk)
        .expect("retire water chunk");
    assert_eq!(world.query::<&Mesh3d>().iter(&world).count(), 0);
    assert!(
        world.resource::<Assets<Mesh>>().is_empty(),
        "water meshes share chunk lifetime"
    );
    presenter.clear(&mut world);
    assert!(world.resource::<Assets<StandardMaterial>>().is_empty());
    assert_eq!(
        view.liquids, expected_liquids,
        "render retirement cannot mutate liquid authority"
    );
}

#[test]
fn raised_water_boundary_uses_the_exact_exclusive_level_transform() {
    let run = |top| VoxelRun {
        bottom: top - 1,
        top,
        material: "water".into(),
    };
    assert!(!raised_water(&run(399), 0.35, 140.0));
    assert!(!raised_water(&run(400), 0.35, 140.0));
    assert!(raised_water(&run(401), 0.35, 140.0));
    assert!(raised_water(&run(900), 0.35, 140.0));
    assert!(!raised_water(
        &VoxelRun {
            material: "unrelated_nonsolid".into(),
            ..run(900)
        },
        0.35,
        140.0
    ));
}
