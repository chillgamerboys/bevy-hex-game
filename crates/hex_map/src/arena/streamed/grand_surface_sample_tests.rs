use super::*;
use bevy::mesh::VertexAttributeValues;
use hex_world_contracts::*;
use hex_world_runtime::{CancellationToken, MemoryChunkSource, RuntimeConfig, WorldRuntime};
use std::time::{Duration, Instant};

fn fixture() -> (NorthernOverview, WorldRuntime, FiniteWorldSession) {
    let materials = vec![
        MaterialSpec {
            id: "stone".into(),
            solid: true,
            diggable: true,
            color: [100, 110, 120, 255],
        },
        MaterialSpec {
            id: "timber".into(),
            solid: true,
            diggable: true,
            color: [140, 80, 40, 255],
        },
    ];
    let mut source = TerrainSurfaceOverview {
        version: 1,
        tolerance: 2.,
        profiles: vec![],
        chunks: vec![],
    };
    let mut chunks = BTreeMap::new();
    let mut dictionary = BTreeMap::new();
    for q in -2..=1 {
        for r in -2..=1 {
            let coordinate = ChunkId { q, r };
            let mut columns = vec![];
            let mut profiles = vec![];
            let mut protection = vec![];
            for lr in 0..16 {
                for lq in 0..16 {
                    let p = WorldHex::new(q * 16 + lq, r * 16 + lr);
                    if p.checked_distance(WorldHex::new(0, 0)).unwrap() > 31 {
                        profiles.push(OUTSIDE_PROFILE);
                        protection.push(0);
                        continue;
                    }
                    let runs = if p == WorldHex::new(15, 9) {
                        vec![
                            SolidRun {
                                bottom: 0,
                                top: 10,
                                material: 0,
                            },
                            SolidRun {
                                bottom: 80,
                                top: 100,
                                material: 0,
                            },
                        ]
                    } else {
                        vec![SolidRun {
                            bottom: 0,
                            top: if p == WorldHex::new(15, 8) { 110 } else { 10 },
                            material: 0,
                        }]
                    };
                    columns.push(ColumnData {
                        position: p,
                        runs: runs
                            .iter()
                            .map(|r| VoxelRun {
                                bottom: i32::from(r.bottom),
                                top: i32::from(r.top),
                                material: "stone".into(),
                            })
                            .collect(),
                    });
                    let profile = SolidProfile { runs };
                    let id = *dictionary.entry(profile.clone()).or_insert_with(|| {
                        let i = u16::try_from(source.profiles.len()).unwrap();
                        source.profiles.push(profile);
                        i
                    });
                    profiles.push(id);
                    let mut flags = if lq == 0 || lq == 15 || lr == 0 || lr == 15 {
                        PATCH_BOUNDARY
                    } else {
                        0
                    };
                    if p == WorldHex::new(15, 9) {
                        flags |= STACKED;
                    }
                    if p == WorldHex::new(8, 8) {
                        flags |= OBJECT_CONTACT;
                    }
                    protection.push(flags);
                }
            }
            if columns.is_empty() {
                continue;
            }
            source.chunks.push(SurfaceChunk {
                coordinate,
                profiles,
                protection,
                surface: None,
            });
            chunks.insert(
                coordinate,
                ChunkPackage {
                    schema_version: SCHEMA_VERSION,
                    world_id: "grand-v4".into(),
                    coordinate,
                    source_fingerprint: 7,
                    columns,
                    features: vec![],
                    semantics: default(),
                    fingerprint: 0,
                },
            );
        }
    }
    chunks
        .get_mut(&ChunkId { q: 0, r: 0 })
        .unwrap()
        .semantics
        .objects
        .push(ObjectInstance {
            id: "sample/object".into(),
            region_id: "sample".into(),
            asset: "sample-object".into(),
            origin: VoxelPosition {
                column: WorldHex::new(8, 8),
                level: 10,
            },
            rotation: 0,
            grounding: None,
            occupancy: vec![ColumnData {
                position: WorldHex::new(8, 8),
                runs: vec![VoxelRun {
                    bottom: 10,
                    top: 12,
                    material: "timber".into(),
                }],
            }],
        });
    let resident_count = chunks.len();
    let mut package = WorldPackage {
        manifest: WorldManifest {
            presentation_fingerprints: default(),
            schema_version: SCHEMA_VERSION,
            world_id: "grand-v4".into(),
            compiler_version: "sample-tests".into(),
            source_fingerprint: 7,
            materials: materials.clone(),
            regions: vec![RegionDescriptor {
                id: "sample".into(),
                origin: WorldHex::new(0, 0),
                radius: 31,
                source_fingerprint: 7,
            }],
            chunks: chunks
                .keys()
                .map(|c| ChunkDescriptor {
                    coordinate: *c,
                    path: format!("chunks/{}_{}.ron", c.q, c.r),
                    fingerprint: 0,
                })
                .collect(),
            boundaries: vec![],
            summary: vec![],
            features: vec![],
            fingerprint: 0,
        },
        chunks,
    };
    package.seal().unwrap();
    let mut map = super::super::tests::planar_overview();
    map.world_id = "grand-v4".into();
    map.materials = materials;
    map.radius = 31;
    map.level_bounds = [0, 150];
    let builder = SurfaceBuilder::new(
        &source,
        map.radius,
        map.level_bounds,
        &map.materials,
        f64::from(map.level_height),
    )
    .unwrap();
    let patches: BTreeMap<_, _> = (-1..=0)
        .flat_map(|q| (-1..=0).map(move |r| ChunkId { q, r }))
        .map(|c| (c, builder.build_patch(c).unwrap()))
        .collect();
    for c in &mut source.chunks {
        c.surface = patches.get(&c.coordinate).cloned();
    }
    map.terrain_surface = Some(source);
    let mut runtime = WorldRuntime::new(
        Arc::new(MemoryChunkSource::new(package).unwrap()),
        RuntimeConfig::default(),
    )
    .unwrap();
    runtime
        .set_interests(vec![ResidencyRequest {
            id: "sample".into(),
            center: WorldHex::new(0, 0),
            radius: 100,
            retention_radius: 100,
            priority: 1,
        }])
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while runtime.resident_chunks().count() != resident_count {
        assert!(runtime.pump().failures.is_empty());
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    let edits = FiniteWorldSession::streamed(&runtime, 0, 150).unwrap();
    (map, runtime, edits)
}

fn edit(
    edits: &mut FiniteWorldSession,
    id: &str,
    column: WorldHex,
    level: i32,
    material: Option<&str>,
) {
    edits
        .apply_transaction(&WorldEditTransaction {
            id: id.into(),
            expected_revisions: BTreeMap::from([(
                column.chunk(),
                edits.revision(column.chunk()).unwrap(),
            )]),
            edits: vec![VoxelEdit {
                position: VoxelPosition { column, level },
                material: material.map(str::to_owned),
            }],
        })
        .unwrap();
}

fn source_profile(source: &TerrainSurfaceOverview, p: WorldHex) -> &SolidProfile {
    let c = source
        .chunks
        .iter()
        .find(|c| c.coordinate == p.chunk())
        .unwrap();
    let i = usize::try_from(p.r.rem_euclid(16) * 16 + p.q.rem_euclid(16)).unwrap();
    source
        .profiles
        .get(usize::from(*c.profiles.get(i).expect("source column ID")))
        .expect("source profile")
}

#[test]
fn sample_edits_refill_halo_eviction_and_restore_never_revive_original_caps() {
    let (map, mut runtime, mut edits) = fixture();
    assert!(edited_source(&map, &edits).unwrap().is_none());
    let cut = WorldHex::new(7, 7);
    edit(&mut edits, "remove", cut, 9, None);
    let changed = edited_source(&map, &edits).unwrap().unwrap();
    assert_eq!(
        source_profile(&changed, cut).runs.last().unwrap().top,
        8 + 1
    );
    assert!(changed
        .chunks
        .iter()
        .filter_map(|c| c.surface.as_ref())
        .all(|s| s.triangles.is_empty()));
    assert_eq!(meshes(&map, &edits).unwrap().len(), 4);
    // Refill is still an edited exact surface, even when it matches original bytes.
    edit(&mut edits, "refill", cut, 9, Some("stone"));
    assert_eq!(
        source_profile(&edited_source(&map, &edits).unwrap().unwrap(), cut)
            .runs
            .last()
            .unwrap()
            .top,
        10
    );
    edit(&mut edits, "new-void", cut, 4, None);
    let hollow = edited_source(&map, &edits).unwrap().unwrap();
    let intervals: Vec<_> = source_profile(&hollow, cut)
        .runs
        .iter()
        .map(|r| (r.bottom, r.top))
        .collect();
    assert_eq!(
        intervals,
        vec![(0, 4), (5, 10)],
        "partial profile changes retain a real new void"
    );
    assert_eq!(meshes(&map, &edits).unwrap().len(), 4);
    let halo = WorldHex::new(16, 7);
    edit(&mut edits, "halo", halo, 9, None);
    let before = edited_source(&map, &edits).unwrap().unwrap();
    let revisions_before = revisions(map.terrain_surface.as_ref().unwrap(), &edits);
    runtime.set_interests(vec![]).unwrap();
    runtime.pump();
    edits.sync_residency(&runtime);
    assert_eq!(edits.resident_source_count(), 0);
    assert_eq!(edited_source(&map, &edits).unwrap().unwrap(), before);
    assert_eq!(
        revisions(map.terrain_surface.as_ref().unwrap(), &edits),
        revisions_before
    );
    let header = edits.checkpoint_header();
    let partitions = edits
        .checkpoint_partitions()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let restored = FiniteWorldSession::restore_checkpoint(
        &runtime,
        0,
        150,
        &header,
        partitions.into_iter().map(Ok),
        &CancellationToken::default(),
    )
    .unwrap();
    assert_eq!(restored.resident_source_count(), 0);
    assert_eq!(edited_source(&map, &restored).unwrap().unwrap(), before);
    assert_eq!(meshes(&map, &restored).unwrap().len(), 4);
}

#[test]
fn sample_native_cliff_void_and_materials_survive_without_mutating_detail_or_picking() {
    let (map, runtime, edits) = fixture();
    let coordinate = ChunkId { q: 0, r: 0 };
    let original = edits.presentation_package(coordinate).unwrap();
    let mut world = World::new();
    world.init_resource::<Assets<Mesh>>();
    world.init_resource::<Assets<StandardMaterial>>();
    let mut presenter = TerrainPresenter::new(
        runtime.manifest(),
        RenderOrigin::default(),
        map.level_height,
    )
    .unwrap();
    presenter
        .publish(&mut world, presenter.prepare(&original, 0).unwrap())
        .unwrap();
    let records: Vec<_> = world
        .query::<&crate::v4::ResidentRun>()
        .iter(&world)
        .cloned()
        .collect();
    assert!(records.iter().any(|r| r.material == "timber"));
    let products = meshes(&map, &edits).unwrap();
    assert_eq!(
        records,
        world
            .query::<&crate::v4::ResidentRun>()
            .iter(&world)
            .cloned()
            .collect::<Vec<_>>()
    );
    assert_eq!(original, edits.presentation_package(coordinate).unwrap());
    let mesh = products.get(&coordinate).expect("selected chunk product");
    let VertexAttributeValues::Float32x3(positions) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap()
    else {
        panic!("positions")
    };
    let VertexAttributeValues::Float32x3(normals) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL).unwrap()
    else {
        panic!("normals")
    };
    assert!(
        positions.iter().any(|p| (p[1] - 38.5).abs() < 1e-5),
        "true 35u cliff keeps its high cap"
    );
    assert!(
        positions
            .iter()
            .zip(normals)
            .any(|(p, n)| (p[1] - 28.).abs() < 1e-5 && n[1] < -0.99),
        "stacked underside stays exposed"
    );
    let VertexAttributeValues::Float32x4(colors) = mesh.attribute(Mesh::ATTRIBUTE_COLOR).unwrap()
    else {
        panic!("colors")
    };
    let timber = Color::srgba_u8(140, 80, 40, 255).to_linear().to_f32_array();
    assert!(
        !colors.contains(&timber),
        "terrain sample never acquires object faces"
    );
    assert_eq!(
        world
            .query::<&crate::v4::ResidentRun>()
            .iter(&world)
            .count(),
        records.len()
    );
    presenter.clear(&mut world);
    assert_eq!(
        world
            .query::<&crate::v4::ResidentRun>()
            .iter(&world)
            .count(),
        0
    );
}
