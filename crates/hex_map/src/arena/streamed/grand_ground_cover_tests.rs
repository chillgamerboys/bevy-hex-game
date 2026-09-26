use super::*;
use hex_core::{
    arena::{ArenaSolidSpan, ArenaTerrainView},
    HexCoord, SubstanceId, TilePos,
};
use hex_world_contracts::{
    ChunkDescriptor, ChunkPackage, ChunkSemantics, ColumnData, MaterialSpec, ObjectInstance,
    RegionDescriptor, ResidencyRequest, VoxelEdit, VoxelRun, WorldEditTransaction, WorldManifest,
    WorldPackage, SCHEMA_VERSION,
};
use hex_world_runtime::{
    CancellationToken, MemoryChunkSource, RuntimeConfig, RuntimeResult, WorldRuntime,
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

fn fixture(
    object_blocker: bool,
) -> Result<(WorldRuntime, FiniteWorldSession, GroundCover), Box<dyn std::error::Error>> {
    let p = WorldHex::new(0, 0);
    let chunk = p.chunk();
    let mut package = WorldPackage {
        manifest: WorldManifest {
            schema_version: SCHEMA_VERSION,
            world_id: "grand-v4".into(),
            compiler_version: "ground-test".into(),
            source_fingerprint: 7,
            materials: ["moss", "stone"]
                .into_iter()
                .map(|id| MaterialSpec {
                    id: id.into(),
                    solid: true,
                    diggable: true,
                    color: [80, 100, 60, 255],
                })
                .collect(),
            regions: vec![RegionDescriptor {
                id: "ground".into(),
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
                    runs: vec![VoxelRun {
                        bottom: 0,
                        top: 421,
                        material: "moss".into(),
                    }],
                }],
                features: vec![],
                semantics: ChunkSemantics {
                    objects: if object_blocker {
                        vec![ObjectInstance {
                            id: "cover-blocker".into(),
                            region_id: "ground".into(),
                            asset: "decor/blocker".into(),
                            origin: VoxelPosition {
                                column: p,
                                level: 421,
                            },
                            rotation: 0,
                            occupancy: vec![ColumnData {
                                position: p,
                                runs: vec![VoxelRun {
                                    bottom: 421,
                                    top: 422,
                                    material: "stone".into(),
                                }],
                            }],
                            grounding: None,
                        }]
                    } else {
                        vec![]
                    },
                    ..ChunkSemantics::default()
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
    load(&mut runtime);
    let edits = FiniteWorldSession::streamed(&runtime, 0, 500)?;
    let source = GroundCover {
        version: 1,
        chunks: vec![GroundCoverChunk {
            coordinate: chunk,
            tufts: vec![GroundTuft {
                support: VoxelPosition {
                    column: p,
                    level: 420,
                },
                material: "moss".into(),
                variant: 0,
            }],
        }],
    };
    source.validate()?;
    Ok((runtime, edits, source))
}
fn load(runtime: &mut WorldRuntime) {
    runtime
        .set_interests(vec![ResidencyRequest {
            id: "test".into(),
            center: WorldHex::new(0, 0),
            radius: 0,
            retention_radius: 0,
            priority: 1,
        }])
        .expect("interest");
    let until = Instant::now() + Duration::from_secs(5);
    while runtime.resident_chunks().count() != 1 {
        assert!(runtime.pump().failures.is_empty());
        assert!(Instant::now() < until, "bounded fixture load");
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn edit(edits: &mut FiniteWorldSession, id: &str, level: i32, material: Option<&str>) {
    let p = WorldHex::new(0, 0);
    edits
        .apply_transaction(&WorldEditTransaction {
            id: id.into(),
            expected_revisions: BTreeMap::from([(
                p.chunk(),
                edits.revision(p.chunk()).expect("source"),
            )]),
            edits: vec![VoxelEdit {
                position: VoxelPosition { column: p, level },
                material: material.map(str::to_owned),
            }],
        })
        .expect("exact terrain mutation");
}
fn setup() -> (World, Cache) {
    let mut world = World::new();
    world.init_resource::<Assets<Mesh>>();
    world.init_resource::<Assets<StandardMaterial>>();
    let mut terrain = ArenaTerrainView::default();
    terrain.columns.insert(
        HexCoord::ORIGIN,
        vec![ArenaSolidSpan {
            bottom: TilePos::new(HexCoord::ORIGIN, 0),
            top_level: 420,
            substance: SubstanceId(0),
        }],
    );
    world.insert_resource(terrain);
    let material = world
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial::default());
    (
        world,
        Cache {
            identity: (1, 1),
            material,
            batches: BTreeMap::new(),
        },
    )
}
fn publication(edits: &FiniteWorldSession) -> BTreeMap<ChunkId, u64> {
    let p = WorldHex::new(0, 0).chunk();
    BTreeMap::from([(p, edits.revision(p).expect("revision"))])
}
fn vertices(cache: &Cache) -> usize {
    cache.batches.values().map(|b| b.vertices).sum()
}

#[test]
fn ground_cover_obeys_publication_retirement_clearance_and_never_adds_collision_or_pins() {
    let (mut runtime, mut edits, source) = fixture(false).expect("actual finite source");
    let (mut world, mut cache) = setup();
    let p = WorldHex::new(0, 0);
    let resident = BTreeSet::from([p.chunk()]);
    let accepted = publication(&edits);
    cache.update(&mut world, &source, &edits, &accepted, &resident, p);
    assert_eq!(vertices(&cache), VERTICES_PER_TUFT);
    assert_eq!(world.resource::<Assets<Mesh>>().len(), 1);
    let view = world.resource::<ArenaTerrainView>();
    assert!(view.object_columns.is_empty());
    assert!(view.static_spans.is_empty());
    assert_eq!(
        view.solid_at(TilePos::new(HexCoord::ORIGIN, 420)),
        Some(SubstanceId(0))
    );
    assert!(view.solid_at(TilePos::new(HexCoord::ORIGIN, 421)).is_none());
    assert_eq!(runtime.resident_chunks().count(), 1);
    assert_eq!(edits.resident_source_count(), 1);

    // Old detailed roots can survive asynchronous meshing; their authority is
    // stale immediately after mutation, so cover must retire before completion.
    edit(&mut edits, "construction", 421, Some("stone"));
    cache.update(&mut world, &source, &edits, &accepted, &resident, p);
    assert_eq!(vertices(&cache), 0);
    assert!(world.resource::<Assets<Mesh>>().is_empty());
    cache.update(
        &mut world,
        &source,
        &edits,
        &publication(&edits),
        &resident,
        p,
    );
    assert_eq!(
        vertices(&cache),
        0,
        "current construction blocks blade volume"
    );
    edit(&mut edits, "clear-construction", 421, None);
    cache.update(
        &mut world,
        &source,
        &edits,
        &publication(&edits),
        &resident,
        p,
    );
    assert_eq!(
        vertices(&cache),
        VERTICES_PER_TUFT,
        "untouched support can recover"
    );

    runtime.set_interests(vec![]).expect("evict");
    runtime.pump();
    edits.sync_residency(&runtime);
    cache.update(
        &mut world,
        &source,
        &edits,
        &publication(&edits),
        &BTreeSet::new(),
        p,
    );
    assert!(cache.batches.is_empty());
    assert!(world.resource::<Assets<Mesh>>().is_empty());
    assert_eq!(
        runtime.resident_chunks().count(),
        0,
        "cover never pins a chunk"
    );
    load(&mut runtime);
    edits.sync_residency(&runtime);
    cache.update(
        &mut world,
        &source,
        &edits,
        &publication(&edits),
        &resident,
        p,
    );
    assert_eq!(vertices(&cache), VERTICES_PER_TUFT);
    world.insert_resource(cache);
    clear(&mut world);
    assert!(!world.contains_resource::<Cache>());
    assert!(world.resource::<Assets<Mesh>>().is_empty());
    assert!(world.resource::<Assets<StandardMaterial>>().is_empty());
}

#[test]
fn ground_cover_support_edits_do_not_regrow_after_refill_eviction_or_checkpoint_restore() {
    let (mut runtime, mut edits, source) = fixture(false).expect("finite fixture");
    let chunk = source.chunks.first().expect("chunk");
    let tuft = chunk.tufts.first().expect("tuft");
    assert!(supported(tuft, &edits));
    edit(&mut edits, "remove-support", 420, None);
    assert!(!supported(tuft, &edits));
    edit(&mut edits, "stone-refill", 420, Some("stone"));
    assert!(!supported(tuft, &edits));
    edit(&mut edits, "original-material-refill", 420, Some("moss"));
    assert_eq!(edits.terrain_at(tuft.support), Some("moss"));
    assert!(edits.terrain_edited(tuft.support));
    assert!(
        !supported(tuft, &edits),
        "same-material refill is not original vegetation support"
    );
    runtime.set_interests(vec![]).expect("evict");
    runtime.pump();
    edits.sync_residency(&runtime);
    let header = edits.checkpoint_header();
    let partitions = edits
        .checkpoint_partitions()
        .collect::<RuntimeResult<Vec<_>>>()
        .expect("partitions");
    let mut restored = FiniteWorldSession::restore_checkpoint(
        &runtime,
        0,
        500,
        &header,
        partitions.into_iter().map(Ok),
        &CancellationToken::default(),
    )
    .expect("restore sparse authority");
    assert_eq!(restored.resident_source_count(), 0);
    assert!(restored.terrain_edited(tuft.support));
    load(&mut runtime);
    restored.sync_residency(&runtime);
    assert_eq!(chunk_mesh(chunk, &restored).count_vertices(), 0);
}

#[test]
fn ground_cover_mesh_is_opaque_bounded_and_stays_inside_each_support_hex() {
    use bevy::mesh::VertexAttributeValues;
    let (_, edits, source) = fixture(false).expect("fixture");
    let mut chunk = source.chunks.first().expect("chunk").clone();
    for variant in 0..6 {
        chunk.tufts.first_mut().expect("tuft").variant = variant;
        let mesh = chunk_mesh(&chunk, &edits);
        assert_eq!(mesh.count_vertices(), VERTICES_PER_TUFT);
        let Some(VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("position contract");
        };
        assert!(positions.iter().all(|&[x, y, z]| x.abs() < 0.5
            && z.abs() < 0.5
            && (147.35 - 0.0001..=147.83 + 0.0001).contains(&y)));
        let Some(VertexAttributeValues::Float32x4(colors)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR)
        else {
            panic!("color contract");
        };
        assert!(colors
            .iter()
            .all(|c| c.last().is_some_and(|a| a.to_bits() == 1.0_f32.to_bits())));
    }
    assert_eq!(MAX_VERTICES, 491_520);
}

#[test]
fn ground_cover_does_not_overlap_actual_authored_object_occupancy() {
    let (_, edits, source) = fixture(true).expect("finite authored blocker");
    let chunk = source.chunks.first().expect("chunk");
    let tuft = chunk.tufts.first().expect("tuft");
    assert_eq!(edits.terrain_at(tuft.support), Some("moss"));
    assert!(edits
        .object_at(VoxelPosition {
            column: tuft.support.column,
            level: 421
        })
        .is_some());
    assert!(!supported(tuft, &edits));
    assert_eq!(chunk_mesh(chunk, &edits).count_vertices(), 0);
}
