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
        halo: Vec::new(),
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
    let id = if let Some(c) = source.chunks.iter().find(|c| c.coordinate == p.chunk()) {
        let i = usize::try_from(p.r.rem_euclid(16) * 16 + p.q.rem_euclid(16)).unwrap();
        *c.profiles.get(i).expect("source column ID")
    } else {
        source
            .halo
            .iter()
            .find(|c| c.column == p)
            .expect("sparse halo fact")
            .profile
    };
    source
        .profiles
        .get(usize::from(id))
        .expect("source profile")
}

#[test]
fn sample_edits_refill_halo_eviction_and_restore_never_revive_original_caps() {
    edit_lifecycle(false);
}

#[test]
fn sparse_halo_edits_survive_eviction_and_restore_without_reviving_faces() {
    edit_lifecycle(true);
}

fn edit_lifecycle(compact: bool) {
    let (mut map, mut runtime, mut edits) = fixture();
    if compact {
        let original = meshes(&map, &edits).unwrap();
        map.terrain_surface = Some(
            map.terrain_surface
                .as_ref()
                .unwrap()
                .compact_halo()
                .unwrap(),
        );
        let compact_meshes = meshes(&map, &edits).unwrap();
        for (c, mesh) in &original {
            let other = compact_meshes.get(c).expect("compacted mesh product");
            assert_eq!(mesh.indices(), other.indices());
            for attribute in [
                Mesh::ATTRIBUTE_POSITION,
                Mesh::ATTRIBUTE_NORMAL,
                Mesh::ATTRIBUTE_COLOR,
            ] {
                assert_eq!(mesh.attribute(attribute), other.attribute(attribute));
            }
        }
    }
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

#[test]
fn snapshot_reports_retained_products_and_edit_fallback_then_disappears_on_clear() {
    let mut world = World::new();
    assert!(surface_sample_snapshot(&world).is_none());
    let (map, runtime, edits) = fixture();
    let products = meshes(&map, &edits).unwrap();
    let expected_vertices: usize = products.values().map(Mesh::count_vertices).sum();
    let expected_indices: usize = products
        .values()
        .map(|mesh| mesh.indices().unwrap().len())
        .sum();
    let sample = State {
        selected: products.keys().copied().collect(),
        revisions: revisions(map.terrain_surface.as_ref().unwrap(), &edits),
        exact_edited_fallback: false,
    };
    let presenter = TerrainPresenter::new(
        runtime.manifest(),
        RenderOrigin::default(),
        map.level_height,
    )
    .unwrap();
    world.init_resource::<Assets<Mesh>>();
    world.init_resource::<Assets<StandardMaterial>>();
    let proxies = products
        .into_iter()
        .map(|(coordinate, mesh)| {
            let handle = world.resource_mut::<Assets<Mesh>>().add(mesh);
            let entity = world.spawn(Mesh3d(handle.clone())).id();
            (coordinate, (entity, handle))
        })
        .collect();
    let (sender, receiver) = mpsc::channel();
    world.insert_resource(Renderer {
        surface_sample: Some(sample),
        shading_budget_fallbacks: 0,
        shading_authority: default(),
        shading_metrics: default(),
        presenter,
        accepted: default(),
        visible_objects: default(),
        publication_revision: 0,
        terrain_edges: default(),
        immutable_edges: default(),
        proxies,
        hidden_proxies: default(),
        materials: default(),
        sender,
        receiver: Mutex::new(receiver),
        active: false,
        epoch: 0,
    });
    let identity = (
        runtime.manifest().fingerprint,
        runtime.manifest().source_fingerprint,
    );
    world.insert_resource(StreamedArena {
        runtime,
        edits,
        overview: Arc::new(map),
        biomes: None,
        generation: 0,
        projected: default(),
        interest_key: None,
        next_transaction: 0,
        policies: default(),
        failure: None,
        publication_ms: 0.,
        peak_resident: 0,
    });
    let snapshot = surface_sample_snapshot(&world).unwrap();
    assert_eq!(snapshot.mode, "crystal-four");
    assert_eq!(
        (snapshot.package_fingerprint, snapshot.source_fingerprint),
        identity
    );
    assert_eq!(snapshot.selected_chunks.len(), 4);
    assert_eq!(snapshot.converted_chunks, 4);
    assert_eq!(snapshot.published_chunks, 4);
    assert_eq!(snapshot.vertices, expected_vertices);
    assert_eq!(snapshot.triangles, expected_indices / 3);
    assert_eq!(
        snapshot.packed_bytes,
        expected_vertices * 40 + expected_indices * 4
    );
    assert!(!snapshot.exact_edited_fallback);
    edit(
        &mut world.resource_mut::<StreamedArena>().edits,
        "snapshot-cut",
        WorldHex::new(7, 7),
        9,
        None,
    );
    world.resource_scope(|world, mut renderer: Mut<Renderer>| {
        let products = renderer
            .surface_sample
            .as_mut()
            .unwrap()
            .refresh(world.resource::<StreamedArena>())
            .unwrap()
            .unwrap();
        for (chunk, mesh) in products {
            let (_, handle) = renderer
                .proxies
                .get(&chunk)
                .expect("converted sample proxy");
            *world
                .resource_mut::<Assets<Mesh>>()
                .get_mut(handle)
                .unwrap() = mesh;
        }
    });
    assert!(
        surface_sample_snapshot(&world)
            .unwrap()
            .exact_edited_fallback
    );
    let removed = world
        .resource::<Renderer>()
        .proxies
        .values()
        .next()
        .unwrap()
        .1
        .clone();
    world.resource_mut::<Assets<Mesh>>().remove(removed.id());
    assert_eq!(surface_sample_snapshot(&world).unwrap().published_chunks, 3);
    clear(&mut world);
    assert!(surface_sample_snapshot(&world).is_none());
}

fn expanded_attributes(mesh: &Mesh) -> Vec<[u32; 10]> {
    let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        panic!("positions")
    };
    let Some(VertexAttributeValues::Float32x3(normals)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL)
    else {
        panic!("normals")
    };
    let Some(VertexAttributeValues::Float32x4(colors)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR)
    else {
        panic!("colors")
    };
    mesh.indices()
        .unwrap()
        .iter()
        .map(|i| {
            let p = *positions.get(i).expect("indexed position");
            let n = *normals.get(i).expect("indexed normal");
            let c = *colors.get(i).expect("indexed color");
            let mut result = [0; 10];
            for (out, value) in result.iter_mut().zip(p.into_iter().chain(n).chain(c)) {
                *out = value.to_bits();
            }
            result
        })
        .collect()
}

#[test]
fn identical_vertex_sharing_preserves_every_indexed_attribute_and_winding() {
    let (map, runtime, edits) = fixture();
    let source = map.terrain_surface.as_ref().unwrap();
    let builder = SurfaceBuilder::new(
        source,
        map.radius,
        map.level_bounds,
        &map.materials,
        f64::from(map.level_height),
    )
    .unwrap();
    let mut saved = 0;
    for chunk in &source.chunks {
        let Some(cap) = &chunk.surface else { continue };
        let original_package = edits.presentation_package(chunk.coordinate).unwrap();
        let faces = builder
            .build_faces(chunk.coordinate, cap, MAX_VERTICES)
            .unwrap();
        let reference = mesh_with_sharing(&map, &faces, false).unwrap();
        let shared = mesh(&map, &faces).unwrap();
        assert_eq!(
            expanded_attributes(&reference),
            expanded_attributes(&shared),
            "each ordered triangle retains exact positions, normals and colors"
        );
        assert_eq!(
            original_package,
            edits.presentation_package(chunk.coordinate).unwrap()
        );
        saved += reference.count_vertices() - shared.count_vertices();
    }
    assert!(saved > 0);
    assert_eq!(runtime.manifest().world_id, "grand-v4");
}

#[test]
#[ignore = "Explicit immutable sample-package measurement; no source mutation or GPU"]
fn actual_sample_sparse_halo_and_shared_vertices_preserve_complete_mesh() {
    let path = std::path::PathBuf::from(
        std::env::var("HEX_GRAND_SURFACE_MEASURE_PACKAGE")
            .expect("explicit immutable sample package"),
    );
    let map: NorthernOverview =
        ron::from_str(&std::fs::read_to_string(path.join("grand-overview.ron")).unwrap()).unwrap();
    let original = map
        .terrain_surface
        .as_ref()
        .expect("explicit surface payload");
    let compact = original.compact_halo().unwrap();
    let old = SurfaceBuilder::new(
        original,
        map.radius,
        map.level_bounds,
        &map.materials,
        f64::from(map.level_height),
    )
    .unwrap();
    let new = SurfaceBuilder::new(
        &compact,
        map.radius,
        map.level_bounds,
        &map.materials,
        f64::from(map.level_height),
    )
    .unwrap();
    let mut old_vertices = 0;
    let mut new_vertices = 0;
    let mut triangles = 0;
    for chunk in &compact.chunks {
        let cap = chunk.surface.as_ref().unwrap();
        let reference_faces = old
            .build_faces(chunk.coordinate, cap, MAX_VERTICES)
            .unwrap();
        let compact_faces = new
            .build_faces(chunk.coordinate, cap, MAX_VERTICES)
            .unwrap();
        assert_eq!(reference_faces, compact_faces);
        let reference = mesh_with_sharing(&map, &reference_faces, false).unwrap();
        let shared = mesh(&map, &compact_faces).unwrap();
        assert_eq!(
            expanded_attributes(&reference),
            expanded_attributes(&shared)
        );
        old_vertices += reference.count_vertices();
        new_vertices += shared.count_vertices();
        triangles += shared.indices().unwrap().len() / 3;
    }
    println!("SURFACE_COMPACTION old_blocks={} drawn_blocks={} sparse_halo={} old_columns={} new_columns={} old_vertices={old_vertices} shared_vertices={new_vertices} triangles={triangles} old_bytes={} shared_bytes={} source_fingerprint={} compact_fingerprint={}", original.chunks.len(), compact.chunks.len(), compact.halo.len(), original.chunks.len()*256+original.halo.len(), compact.chunks.len()*256+compact.halo.len(), old_vertices*40+triangles*12, new_vertices*40+triangles*12, original.fingerprint().unwrap(), compact.fingerprint().unwrap());
}
