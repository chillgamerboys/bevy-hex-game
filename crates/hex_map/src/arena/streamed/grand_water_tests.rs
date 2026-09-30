use super::*;
use hex_world_contracts::{
    ChunkDescriptor, ChunkPackage, ChunkSemantics, ColumnData, LiquidColumn, MaterialSpec,
    RegionDescriptor, ResidencyRequest, VoxelEdit, VoxelPosition, VoxelRun, WorldEditTransaction,
    WorldManifest, WorldPackage, SCHEMA_VERSION,
};
use hex_world_runtime::{CancellationToken, MemoryChunkSource, RuntimeConfig, WorldRuntime};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

fn column() -> InlandWaterColumn {
    InlandWaterColumn {
        column: WorldHex::new(0, 0),
        bottom: 420,
        top: 440,
        kind: LiquidKind::Standing,
        downstream: None,
        exposed_top: true,
        exposed_bottom: false,
        sides: std::array::from_fn(|_| vec![[420, 440]]),
    }
}
fn terrain() -> Vec<ColumnData> {
    (0..16)
        .flat_map(|q| {
            (0..16).map(move |r| ColumnData {
                position: WorldHex::new(q, r),
                runs: if q == 0 && r == 0 {
                    vec![VoxelRun {
                        bottom: 0,
                        top: 420,
                        material: "stone".into(),
                    }]
                } else {
                    vec![]
                },
            })
        })
        .collect()
}
fn halo() -> Vec<ColumnData> {
    terrain()
        .iter()
        .flat_map(|c| neighbors(c.position))
        .filter(|p| p.chunk() != WorldHex::new(0, 0).chunk())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|position| ColumnData {
            position,
            runs: vec![],
        })
        .collect()
}
fn source() -> InlandWaterOverview {
    InlandWaterOverview {
        version: 1,
        chunks: vec![InlandWaterChunk {
            coordinate: WorldHex::new(0, 0).chunk(),
            columns: vec![column()],
            terrain: terrain(),
            halo: halo(),
        }],
    }
}
fn overview(source: InlandWaterOverview) -> NorthernOverview {
    NorthernOverview {
        terrain_surface: None,
        version: 1,
        source_fingerprint: 1,
        package_fingerprint: 2,
        world_id: "grand-v4".into(),
        hex_radius: 1.0,
        level_height: 0.35,
        vertical_offset: 0.35,
        sea_level: 140.0,
        radius: 100,
        level_bounds: [0, 2400],
        origin_xz: [-200.0, -200.0],
        spacing: 8.0,
        width: 51,
        height: 51,
        bed_heights: vec![0.0; 2601],
        surface_materials: vec![0; 2601],
        materials: vec![MaterialSpec {
            id: "stone".into(),
            solid: true,
            diggable: true,
            color: [80, 80, 80, 255],
        }],
        player_spawn: [0.0, 150.0, 0.0],
        anchors: BTreeMap::new(),
        forest: None,
        ground_cover: None,
        inland_water: Some(source),
        review_cameras: BTreeMap::new(),
        islands: vec![],
        tree_count: 0,
        building_count: 0,
    }
}
fn fixture() -> Result<(WorldRuntime, FiniteWorldSession), Box<dyn std::error::Error>> {
    let p = WorldHex::new(0, 0);
    let chunk = p.chunk();
    let mut package = WorldPackage {
        manifest: WorldManifest {
            presentation_fingerprints: Default::default(),
            schema_version: SCHEMA_VERSION,
            world_id: "grand-v4".into(),
            compiler_version: "water-test".into(),
            source_fingerprint: 7,
            materials: vec![
                MaterialSpec {
                    id: "stone".into(),
                    solid: true,
                    diggable: true,
                    color: [80, 80, 80, 255],
                },
                MaterialSpec {
                    id: "water".into(),
                    solid: false,
                    diggable: false,
                    color: [37, 104, 150, 180],
                },
            ],
            regions: vec![RegionDescriptor {
                id: "water".into(),
                origin: p,
                radius: 0,
                source_fingerprint: 7,
            }],
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
                world_id: "grand-v4".into(),
                coordinate: chunk,
                source_fingerprint: 7,
                columns: vec![ColumnData {
                    position: p,
                    runs: vec![
                        VoxelRun {
                            bottom: 0,
                            top: 420,
                            material: "stone".into(),
                        },
                        VoxelRun {
                            bottom: 420,
                            top: 440,
                            material: "water".into(),
                        },
                    ],
                }],
                features: vec![],
                semantics: ChunkSemantics {
                    liquids: vec![LiquidColumn {
                        column: p,
                        bottom: 420,
                        top: 440,
                        kind: LiquidKind::Standing,
                        body_id: "pool".into(),
                        downstream: vec![],
                    }],
                    ..default()
                },
                fingerprint: 0,
            },
        )]),
    };
    package.seal()?;
    let mut runtime = WorldRuntime::new(
        Arc::new(MemoryChunkSource::new(package)?),
        RuntimeConfig::default(),
    )?;
    load(&mut runtime)?;
    let edits = FiniteWorldSession::streamed(&runtime, 0, 500)?;
    Ok((runtime, edits))
}
fn load(runtime: &mut WorldRuntime) -> Result<(), Box<dyn std::error::Error>> {
    runtime.set_interests(vec![ResidencyRequest {
        id: "water".into(),
        center: WorldHex::new(0, 0),
        radius: 0,
        retention_radius: 0,
        priority: 1,
    }])?;
    let until = Instant::now() + Duration::from_secs(5);
    while runtime.resident_chunks().count() != 1 {
        if !runtime.pump().failures.is_empty() {
            return Err("fixture source admission failed".into());
        }
        if Instant::now() >= until {
            return Err("bounded fixture load timed out".into());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    Ok(())
}
fn setup() -> (World, Cache) {
    let mut world = World::new();
    world.init_resource::<Assets<Mesh>>();
    world.init_resource::<Assets<StandardMaterial>>();
    let standard = world
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial::default());
    (
        world,
        Cache {
            identity: (1, 1),
            standard,
            rivers: BTreeMap::new(),
            batches: BTreeMap::new(),
        },
    )
}
#[test]
fn exact_fall_faces_keep_vertical_extent_normals_and_shared_style() {
    let mut column = column();
    column.kind = LiquidKind::Waterfall;
    column.downstream = Some(VoxelPosition {
        column: WorldHex::new(1, 0),
        level: 419,
    });
    column.sides = std::array::from_fn(|i| if i == 0 { vec![[421, 433]] } else { vec![] });
    let mut data = MeshData::default();
    data.column(&column, 0.35);
    assert_eq!(data.positions.len(), 11);
    assert_eq!(data.indices.len(), 24);
    let side = data.positions.split_at_checked(7).unwrap().1;
    assert!(side
        .iter()
        .all(|p| (p.first().unwrap() - 3.0_f32.sqrt() * 0.5).abs() < 0.000_001));
    let heights = side
        .iter()
        .map(|p| p.get(1).copied().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        heights.iter().map(|h| h.to_bits()).collect::<Vec<_>>(),
        [421.0_f32 * 0.35, 421.0 * 0.35, 433.0 * 0.35, 433.0 * 0.35].map(f32::to_bits)
    );
    assert!(data
        .normals
        .split_at_checked(7)
        .unwrap()
        .1
        .iter()
        .all(|n| Vec3::from_array(*n).distance(Vec3::X) < 0.000_001));
    assert_eq!(
        river::liquid_style(column.kind, column.top, column.downstream),
        Some(Style::Fall)
    );
    // Actual surface vertices are independent of clock/flow shading.
    assert!(data
        .positions
        .iter()
        .take(7)
        .all(|p| (Vec3::from_array(*p).y - 154.0).abs() < 0.000_01));
}
#[test]
fn exact_water_rejects_noncanonical_or_out_of_envelope_facts() {
    validate(&overview(source())).unwrap();
    for column in [
        InlandWaterColumn {
            bottom: 440,
            ..column()
        },
        InlandWaterColumn {
            top: 2402,
            ..column()
        },
        InlandWaterColumn {
            top: 400,
            bottom: 390,
            ..column()
        },
        InlandWaterColumn {
            kind: LiquidKind::Directed,
            ..column()
        },
        InlandWaterColumn {
            downstream: Some(VoxelPosition {
                column: WorldHex::new(1, 0),
                level: 430,
            }),
            ..column()
        },
        InlandWaterColumn {
            sides: std::array::from_fn(|_| vec![[419, 440]]),
            ..column()
        },
        InlandWaterColumn {
            sides: std::array::from_fn(|_| vec![[420, 430], [429, 440]]),
            ..column()
        },
        InlandWaterColumn {
            sides: std::array::from_fn(|_| vec![[420, 421]; 17]),
            ..column()
        },
    ] {
        let source = InlandWaterOverview {
            version: 1,
            chunks: vec![InlandWaterChunk {
                coordinate: column.column.chunk(),
                columns: vec![column],
                terrain: terrain(),
                halo: halo(),
            }],
        };
        assert!(validate(&overview(source)).is_err());
    }
    let mut source = source();
    source.chunks.push(source.chunks.first().unwrap().clone());
    assert!(validate(&overview(source)).is_err());
}
#[test]
#[expect(
    clippy::panic_in_result_fn,
    reason = "This lifecycle test propagates fallible fixture setup and asserts independent rendering invariants."
)]
fn exact_water_detail_handoff_retained_edits_and_asset_retirement(
) -> Result<(), Box<dyn std::error::Error>> {
    let (mut runtime, mut edits) = fixture()?;
    let source = source();
    let chunk = WorldHex::new(0, 0).chunk();
    let (mut world, mut cache) = setup();
    cache.update(&mut world, &source, &edits, &BTreeSet::new(), 0.35);
    assert_eq!(world.resource::<Assets<Mesh>>().len(), 1);
    let entity = cache
        .batches
        .get(&chunk)
        .ok_or("missing fixture batch")?
        .parts
        .first()
        .unwrap()
        .0;
    assert_eq!(
        world.get::<Visibility>(entity),
        Some(&Visibility::Inherited)
    );
    // The receipt, even during pending edit preparation, owns the detailed
    // surface. Its unrelated publication counter is deliberately not compared.
    cache.update(&mut world, &source, &edits, &BTreeSet::from([chunk]), 0.35);
    assert_eq!(world.get::<Visibility>(entity), Some(&Visibility::Hidden));
    edits.apply_transaction(&WorldEditTransaction {
        id: "clear-lip".into(),
        expected_revisions: BTreeMap::from([(chunk, edits.revision(chunk).unwrap())]),
        edits: vec![VoxelEdit {
            position: VoxelPosition {
                column: WorldHex::new(0, 0),
                level: 419,
            },
            material: None,
        }],
    })?;
    cache.update(&mut world, &source, &edits, &BTreeSet::from([chunk]), 0.35);
    assert!(world.get_entity(entity).is_err());
    assert!(
        cache
            .batches
            .get(&chunk)
            .ok_or("missing fixture batch")?
            .terrain_edited
    );
    assert!(world.resource::<Assets<Mesh>>().is_empty());
    let header = edits.checkpoint_header();
    let partitions = edits
        .checkpoint_partitions()
        .collect::<Result<Vec<_>, _>>()?;
    runtime.set_interests(vec![])?;
    runtime.pump();
    edits.sync_residency(&runtime);
    let restored = FiniteWorldSession::restore_checkpoint(
        &runtime,
        0,
        500,
        &header,
        partitions.into_iter().map(Ok),
        &CancellationToken::default(),
    )?;
    cache.update(&mut world, &source, &restored, &BTreeSet::new(), 0.35);
    assert!(
        world.resource::<Assets<Mesh>>().is_empty(),
        "eviction/restore must not resurrect edited far water"
    );
    assert_eq!(runtime.resident_chunks().count(), 0);
    assert_eq!(restored.resident_source_count(), 0);
    world.insert_resource(cache);
    clear(&mut world);
    assert!(world.resource::<Assets<StandardMaterial>>().is_empty());
    assert!(!world.contains_resource::<Cache>());
    // A fresh map lifetime can show the original immutable companion again.
    load(&mut runtime)?;
    let fresh = FiniteWorldSession::streamed(&runtime, 0, 500)?;
    let mut cache = Cache {
        identity: (2, 2),
        standard: Handle::default(),
        rivers: BTreeMap::new(),
        batches: BTreeMap::new(),
    };
    cache.standard = world
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial::default());
    cache.update(&mut world, &source, &fresh, &BTreeSet::new(), 0.35);
    assert_eq!(world.resource::<Assets<Mesh>>().len(), 1);
    world.insert_resource(cache);
    clear(&mut world);
    assert!(world.resource::<Assets<Mesh>>().is_empty());
    Ok(())
}

#[test]
fn exact_inland_terrain_replaces_the_entire_coarse_chunk_and_rejects_missing_halo() {
    let mut map = overview(source());
    map.bed_heights.fill(700.0);
    let mesh = super::super::render::proxy(&map, WorldHex::new(0, 0).chunk()).unwrap();
    let positions = mesh
        .attribute(Mesh::ATTRIBUTE_POSITION)
        .unwrap()
        .as_float3()
        .unwrap();
    assert_eq!(
        positions.len(),
        31,
        "one exposed top and six exact solid sides, no coarse ramp"
    );
    assert!(positions.iter().all(|p| Vec3::from_array(*p).y <= 147.0));
    assert!(positions
        .iter()
        .any(|p| (Vec3::from_array(*p).y - 147.0).abs() < 0.00001));
    let water = map.inland_water.as_mut().unwrap();
    water.chunks.first_mut().unwrap().halo.pop();
    assert!(
        validate(&map).is_err(),
        "missing halo would falsely expose a solid side"
    );
}

#[test]
fn exact_inland_terrain_halo_occludes_without_drawing_or_burying_cave_caps() {
    let mut map = overview(source());
    let source = map.inland_water.as_mut().unwrap();
    let chunk = source.chunks.first_mut().unwrap();
    let column = chunk.terrain.first_mut().unwrap();
    column.runs = vec![
        VoxelRun {
            bottom: 0,
            top: 10,
            material: "stone".into(),
        },
        VoxelRun {
            bottom: 10,
            top: 20,
            material: "stone".into(),
        },
        VoxelRun {
            bottom: 30,
            top: 40,
            material: "stone".into(),
        },
    ];
    // West neighbor is halo only. Its solid wall must hide all west sides;
    // its high top must not itself appear in this chunk's mesh.
    let west = chunk
        .halo
        .iter_mut()
        .find(|c| c.position == WorldHex::new(-1, 0))
        .unwrap();
    west.runs = vec![VoxelRun {
        bottom: 0,
        top: 100,
        material: "stone".into(),
    }];
    let mesh = super::super::grand_inland_terrain::mesh(
        &map,
        map.inland_water.as_ref().unwrap().chunks.first().unwrap(),
    );
    let positions = mesh
        .attribute(Mesh::ATTRIBUTE_POSITION)
        .unwrap()
        .as_float3()
        .unwrap();
    let normals = mesh
        .attribute(Mesh::ATTRIBUTE_NORMAL)
        .unwrap()
        .as_float3()
        .unwrap();
    assert!(positions.iter().all(|p| Vec3::from_array(*p).y <= 14.0));
    let cap = |height: f32, normal: Vec3| {
        positions
            .iter()
            .zip(normals)
            .filter(|(p, n)| {
                (Vec3::from_array(**p).y - height).abs() < 0.00001
                    && Vec3::from_array(**n).distance(normal) < 0.00001
            })
            .count()
    };
    assert_eq!(cap(3.5, Vec3::Y), 0, "buried material boundary has no cap");
    assert_eq!(cap(7.0, Vec3::Y), 7, "cave floor remains");
    assert_eq!(cap(10.5, Vec3::NEG_Y), 7, "cave roof underside remains");
    assert!(normals
        .iter()
        .all(|n| Vec3::from_array(*n).distance(Vec3::NEG_X) > 0.001));
}
