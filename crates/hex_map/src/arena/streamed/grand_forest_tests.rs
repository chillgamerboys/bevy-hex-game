use super::*;
use hex_world_contracts::{
    ChunkDescriptor, ChunkPackage, ChunkSemantics, ColumnData, ObjectInstance, RegionDescriptor,
    ResidencyRequest, VoxelEdit, VoxelRun, WorldEditTransaction, WorldHex, WorldManifest,
    WorldPackage, SCHEMA_VERSION,
};
use hex_world_runtime::{
    CancellationToken, FiniteChunkCheckpoint, FiniteSessionHeader, MemoryChunkSource,
    RuntimeConfig, RuntimeResult, WorldRuntime,
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

struct Fixture {
    runtime: WorldRuntime,
    edits: FiniteWorldSession,
    source: ForestOverview,
    materials: Vec<MaterialSpec>,
}
fn fixture() -> Result<Fixture, Box<dyn std::error::Error>> {
    let root = WorldHex::new(15, 0);
    let crown = WorldHex::new(16, 0);
    let shape = ForestShape {
        id: "plant/grand-fixture".into(),
        asset: "plant/grand-fixture".into(),
        columns: vec![
            ColumnData {
                position: WorldHex::new(0, 0),
                runs: vec![
                    VoxelRun {
                        bottom: 0,
                        top: 12,
                        material: "timber".into(),
                    },
                    VoxelRun {
                        bottom: 12,
                        top: 18,
                        material: "foliage".into(),
                    },
                ],
            },
            ColumnData {
                position: WorldHex::new(1, 0),
                runs: vec![VoxelRun {
                    bottom: 12,
                    top: 18,
                    material: "foliage".into(),
                }],
            },
        ],
    };
    let instance = ForestInstance {
        id: "grand/tree/15_0".into(),
        shape: 0,
        origin: VoxelPosition {
            column: root,
            level: 10,
        },
        rotation: 0,
        base_level: 10,
        roots: vec![],
        footprint: BTreeSet::from([root.chunk(), crown.chunk()]),
    };
    let object = ObjectInstance {
        id: instance.id.clone(),
        region_id: "tree".into(),
        asset: shape.asset.clone(),
        origin: instance.origin,
        rotation: 0,
        occupancy: instance.columns(&shape)?,
        grounding: None,
    };
    let source = ForestOverview {
        version: 1,
        shapes: vec![shape],
        instances: vec![instance],
    };
    source.validate()?;
    let chunks: BTreeMap<_, _> = [root, crown]
        .into_iter()
        .map(|position| {
            (
                position.chunk(),
                ChunkPackage {
                    schema_version: SCHEMA_VERSION,
                    world_id: "grand-v4".into(),
                    coordinate: position.chunk(),
                    source_fingerprint: 7,
                    columns: vec![ColumnData {
                        position,
                        runs: vec![VoxelRun {
                            bottom: 0,
                            top: 10,
                            material: "stone".into(),
                        }],
                    }],
                    features: vec![],
                    semantics: ChunkSemantics {
                        objects: if position == root {
                            vec![object.clone()]
                        } else {
                            vec![]
                        },
                        ..default()
                    },
                    fingerprint: 0,
                },
            )
        })
        .collect();
    let materials: Vec<_> = ["stone", "timber", "foliage"]
        .into_iter()
        .map(|id| MaterialSpec {
            id: id.into(),
            solid: true,
            diggable: true,
            color: [80, 100, 60, 255],
        })
        .collect();
    let mut package = WorldPackage {
        manifest: WorldManifest {
            schema_version: SCHEMA_VERSION,
            world_id: "grand-v4".into(),
            compiler_version: "test-1".into(),
            source_fingerprint: 7,
            materials: materials.clone(),
            regions: [("tree", root), ("crown", crown)]
                .into_iter()
                .map(|(id, origin)| RegionDescriptor {
                    id: id.into(),
                    origin,
                    radius: 0,
                    source_fingerprint: 7,
                })
                .collect(),
            chunks: chunks
                .keys()
                .map(|coordinate| ChunkDescriptor {
                    coordinate: *coordinate,
                    fingerprint: 0,
                    path: format!("chunks/{}_{}.ron", coordinate.q, coordinate.r),
                })
                .collect(),
            boundaries: vec![],
            summary: vec![],
            features: vec![],
            fingerprint: 0,
        },
        chunks,
    };
    package.seal()?;
    let mut runtime = WorldRuntime::new(
        Arc::new(MemoryChunkSource::new(package)?),
        RuntimeConfig::default(),
    )?;
    runtime.set_interests(vec![ResidencyRequest {
        id: "fixture".into(),
        center: root,
        radius: 1,
        retention_radius: 1,
        priority: 1,
    }])?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while runtime.resident_chunks().count() != 2 {
        let update = runtime.pump();
        if !update.failures.is_empty() {
            return Err(format!("fixture admission failed: {:?}", update.failures).into());
        }
        if Instant::now() >= deadline {
            return Err("fixture admission timed out".into());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let edits = FiniteWorldSession::streamed(&runtime, 0, 50)?;
    Ok(Fixture {
        runtime,
        edits,
        source,
        materials,
    })
}
fn remove(edits: &mut FiniteWorldSession, id: &str, cells: &[VoxelPosition]) -> RuntimeResult<()> {
    edits.apply_transaction(&WorldEditTransaction {
        id: id.into(),
        expected_revisions: cells
            .iter()
            .map(|p| {
                (
                    p.column.chunk(),
                    edits.revision(p.column.chunk()).unwrap_or(0),
                )
            })
            .collect(),
        edits: cells
            .iter()
            .map(|position| VoxelEdit {
                position: *position,
                material: None,
            })
            .collect(),
    })?;
    Ok(())
}
fn world() -> World {
    let mut world = World::new();
    world.init_resource::<Assets<Mesh>>();
    world.init_resource::<Assets<StandardMaterial>>();
    world
}
fn mask_mesh(source: &ForestOverview, edits: &FiniteWorldSession) -> Result<ForestMesh, String> {
    let instance = source.instances.first().ok_or("instance")?;
    let shape = source.shapes.first().ok_or("shape")?;
    let cuts = source_cuts(instance, shape, edits)?;
    forest_mesh(
        &instance.local_columns(shape).map_err(|e| e.to_string())?,
        &cuts,
    )
    .map_err(|e| e.to_string())
}
#[test]
fn forest_ground_only_edits_keep_shared_geometry_and_partial_detail_keeps_whole_tree() {
    let mut f = fixture().expect("actual compact finite source");
    let mut world = world();
    let mut forest = Forest::new(
        &mut world,
        &f.source,
        &f.materials,
        &f.edits,
        (900, [0, 1600]),
    )
    .expect("forest");
    let before = mask_mesh(&f.source, &f.edits).expect("source mesh");
    let shape = forest.source.shapes.first().expect("shape");
    let tree = forest.trees.first_mut().expect("tree");
    tree.publish_visibility(&mut world, false);
    assert_eq!(
        *world.get::<Visibility>(tree.body).expect("visibility"),
        Visibility::Inherited
    );
    let handle = world.get::<Mesh3d>(tree.body).expect("body").0.clone();
    remove(
        &mut f.edits,
        "ground-only",
        &[VoxelPosition {
            column: WorldHex::new(15, 0),
            level: 9,
        }],
    )
    .expect("terrain edit");
    assert!(tree.changed(&f.edits));
    assert!(tree
        .refresh(
            &mut world,
            shape,
            &f.edits,
            &forest.palette,
            forest.submitted_vertices
        )
        .expect("refresh")
        .is_none());
    assert_eq!(world.get::<Mesh3d>(tree.body).expect("body").0, handle);
    assert!(tree.edited.is_none());
    assert_eq!(mask_mesh(&f.source, &f.edits).expect("same mesh"), before);
    tree.publish_visibility(&mut world, true);
    assert_eq!(
        *world.get::<Visibility>(tree.body).expect("visibility"),
        Visibility::Hidden
    );
    tree.publish_visibility(&mut world, false);
    assert_eq!(
        *world.get::<Visibility>(tree.body).expect("visibility"),
        Visibility::Inherited
    );
    assert_eq!(tree.revisions.len(), 2);
}
#[test]
fn forest_actual_trunk_and_crown_cuts_survive_zero_residency_checkpoint_resume() {
    let mut f = fixture().expect("source");
    let before = mask_mesh(&f.source, &f.edits).expect("mesh");
    let cuts = [
        VoxelPosition {
            column: WorldHex::new(15, 0),
            level: 12,
        },
        VoxelPosition {
            column: WorldHex::new(16, 0),
            level: 25,
        },
    ];
    remove(&mut f.edits, "object-cuts", &cuts).expect("actual object edits");
    for cut in cuts {
        assert!(f.edits.object_removed(cut));
    }
    let after = mask_mesh(&f.source, &f.edits).expect("masked mesh");
    assert_ne!(after, before);
    for triangle in after.indices.chunks_exact(3) {
        let material = after
            .materials
            .get(*triangle.first().expect("triangle") as usize)
            .expect("material");
        let heights: Vec<_> = triangle
            .iter()
            .map(|index| {
                after
                    .positions
                    .get(*index as usize)
                    .expect("position")
                    .get(1)
                    .copied()
                    .expect("Y")
            })
            .collect();
        if material == "timber" {
            let low = heights.iter().copied().fold(f32::INFINITY, f32::min);
            let high = heights.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            assert!(
                !(low < 0.875 && high > 0.875),
                "no timber triangle spans the removed exact level2 cell"
            );
        } else {
            assert!(
                heights.iter().all(|height| *height >= 5.6),
                "the carved foliage band is conservatively absent"
            );
        }
    }

    let mut world = world();
    let forest = Forest::new(
        &mut world,
        &f.source,
        &f.materials,
        &f.edits,
        (900, [0, 1600]),
    )
    .expect("masked initialization");
    let tree = forest.trees.first().expect("tree");
    assert_eq!(tree.cuts.len(), 2);
    assert!(tree.edited.is_some());
    tree.publish_visibility(&mut world, false);
    assert_eq!(
        *world.get::<Visibility>(tree.body).expect("visibility"),
        Visibility::Inherited
    );
    tree.publish_visibility(&mut world, true);
    assert_eq!(
        *world.get::<Visibility>(tree.body).expect("visibility"),
        Visibility::Hidden
    );
    tree.publish_visibility(&mut world, false);
    assert_eq!(
        *world.get::<Visibility>(tree.body).expect("visibility"),
        Visibility::Inherited
    );
    f.runtime.set_interests(vec![]).expect("evict");
    f.runtime.pump();
    f.edits.sync_residency(&f.runtime);
    assert_eq!(f.edits.resident_source_count(), 0);
    assert_eq!(f.runtime.resident_chunks().count(), 0);
    assert_eq!(
        mask_mesh(&f.source, &f.edits).expect("zero residency mask"),
        after
    );
    let header = f.edits.checkpoint_header();
    let partitions = f
        .edits
        .checkpoint_partitions()
        .collect::<RuntimeResult<Vec<_>>>()
        .expect("persistent partitions");
    let encoded = ron::to_string(&(header, partitions)).expect("save");
    let (header, partitions): (FiniteSessionHeader, Vec<FiniteChunkCheckpoint>) =
        ron::from_str(&encoded).expect("decode");
    let resumed = FiniteWorldSession::restore_checkpoint(
        &f.runtime,
        0,
        50,
        &header,
        partitions.into_iter().map(Ok),
        &CancellationToken::default(),
    )
    .expect("resume with zero loaded chunks");
    assert_eq!(resumed.resident_source_count(), 0);
    assert_eq!(
        mask_mesh(&f.source, &resumed).expect("first restored mesh"),
        after
    );
}
#[test]
fn forest_source_replacement_and_map_exit_release_all_meshes_entities_and_materials() {
    let f = fixture().expect("source");
    let mut world = world();
    let forest = Forest::new(
        &mut world,
        &f.source,
        &f.materials,
        &f.edits,
        (900, [0, 1600]),
    )
    .expect("forest");
    let entities: Vec<_> = forest.trees.iter().map(|t| t.body).collect();
    world.insert_resource(Cache {
        identity: (7, 1),
        forest: Some(forest),
    });
    retire_stale(&mut world, (7, 1));
    assert!(
        world.contains_resource::<Cache>(),
        "unchanged identity retains geometry"
    );
    retire_stale(&mut world, (8, 1));
    assert!(!world.contains_resource::<Cache>());
    for entity in entities {
        assert!(world.get_entity(entity).is_err());
    }
    assert_eq!(world.resource::<Assets<Mesh>>().len(), 0);
    assert_eq!(world.resource::<Assets<StandardMaterial>>().len(), 0);
    let forest = Forest::new(
        &mut world,
        &f.source,
        &f.materials,
        &f.edits,
        (900, [0, 1600]),
    )
    .expect("reenter forest");
    world.insert_resource(Cache {
        identity: (8, 1),
        forest: Some(forest),
    });
    retire_stale(&mut world, (8, 2));
    assert!(
        !world.contains_resource::<Cache>(),
        "New Run retires the previous generation"
    );
    assert_eq!(world.resource::<Assets<Mesh>>().len(), 0);
    assert_eq!(world.resource::<Assets<StandardMaterial>>().len(), 0);
    clear(&mut world);
}
#[test]
fn forest_local_rotation_matches_exact_axial_placement_and_normals_face_outward() {
    let f = fixture().expect("source");
    let mut instance = f.source.instances.first().expect("instance").clone();
    let point = Vec3::new(1.732_050_8, 0., 0.);
    for rotation in 0..6 {
        instance.rotation = rotation;
        let transform = instance_transform(&instance).expect("finite transform");
        let logical =
            hex_schematic::v4::northern::forest::forest_rotate(WorldHex::new(1, 0), rotation)
                .expect("axial transform");
        let logical = super::super::local(WorldHex::new(
            instance.origin.column.q + logical.q,
            instance.origin.column.r + logical.r,
        ))
        .expect("column")
        .to_world(
            f32::from(i16::try_from(instance.base_level).expect("bounded fixture level")) * 0.35,
        );
        assert!(transform.transform_point(point).distance(logical) < 0.0001);
    }
    let mesh = mask_mesh(&f.source, &f.edits).expect("opaque mesh");
    for triangle in mesh.indices.chunks_exact(3) {
        let [a, b, c] = triangle else {
            continue;
        };
        let pa = Vec3::from_array(*mesh.positions.get(*a as usize).expect("vertex"));
        let pb = Vec3::from_array(*mesh.positions.get(*b as usize).expect("vertex"));
        let pc = Vec3::from_array(*mesh.positions.get(*c as usize).expect("vertex"));
        let normal = Vec3::from_array(*mesh.normals.get(*a as usize).expect("normal"));
        assert!((pb - pa).cross(pc - pa).dot(normal) > 0.);
        assert!((normal.length() - 1.).abs() < 0.0001);
    }
}
