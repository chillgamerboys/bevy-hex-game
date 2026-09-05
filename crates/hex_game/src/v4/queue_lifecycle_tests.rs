//! Real headless queue transitions: CPU mesh facts and owned roots, no renderer.
#![expect(
    clippy::expect_used,
    reason = "Small deterministic fixtures and exact lifecycle assertions"
)]

use std::{
    thread,
    time::{Duration, Instant},
};

use bevy::mesh::VertexAttributeValues;
use hex_map::v4::RenderOrigin;
use hex_world_contracts::{
    ChunkDescriptor, ChunkPackage, ChunkSemantics, ColumnData, MaterialSpec, RegionDescriptor,
    ResidencyRequest, VoxelRun, WorldHex, WorldManifest, WorldPackage, SCHEMA_VERSION,
};
use hex_world_runtime::{MemoryChunkSource, RuntimeConfig};

use super::*;

struct Fixture {
    world: World,
    runtime: WorldRuntime,
    presenter: TerrainPresenter,
    queue: MeshQueue,
}

fn chunk(q: i64) -> ChunkId {
    ChunkId { q, r: 0 }
}

fn fixture() -> Fixture {
    let mut chunks = BTreeMap::<ChunkId, ChunkPackage>::new();
    let mut regions = Vec::new();
    // The middle chunks have water at both ends. Remeshing B for A must retain
    // its halo from C, although C is outside A's directly affected set.
    for (index, q) in [15, 16, 31, 32, 47, 48].into_iter().enumerate() {
        let position = WorldHex::new(q, 5);
        regions.push(RegionDescriptor {
            id: format!("water-{index}"),
            origin: position,
            radius: 0,
            source_fingerprint: 1,
        });
        chunks
            .entry(position.chunk())
            .or_insert_with(|| ChunkPackage {
                schema_version: SCHEMA_VERSION,
                world_id: "queue-water".into(),
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
                runs: vec![VoxelRun {
                    bottom: 0,
                    top: 2,
                    material: "water".into(),
                }],
            });
    }
    let mut package = WorldPackage {
        manifest: WorldManifest {
            schema_version: SCHEMA_VERSION,
            world_id: "queue-water".into(),
            compiler_version: "queue-lifecycle-test".into(),
            source_fingerprint: 1,
            materials: vec![MaterialSpec {
                id: "water".into(),
                solid: false,
                diggable: false,
                color: [40, 130, 180, 180],
            }],
            regions,
            chunks: chunks
                .keys()
                .map(|coordinate| ChunkDescriptor {
                    coordinate: *coordinate,
                    fingerprint: 0,
                    path: format!("chunks/{}_{}.ron", coordinate.q, coordinate.r),
                })
                .collect(),
            boundaries: Vec::new(),
            summary: Vec::new(),
            features: Vec::new(),
            fingerprint: 0,
        },
        chunks,
    };
    package.seal().expect("exact finite water fixture");
    let presenter = TerrainPresenter::new(
        &package.manifest,
        RenderOrigin {
            column: WorldHex::new(0, 0),
            level: 0,
        },
        1.0,
    )
    .expect("terrain presenter");
    let runtime = WorldRuntime::new(
        Arc::new(MemoryChunkSource::new(package).expect("validated source")),
        RuntimeConfig {
            max_resident_chunks: 8,
            max_in_flight_jobs: 2,
            ..RuntimeConfig::default()
        },
    )
    .expect("world runtime");
    let mut fixture = Fixture {
        world: World::new(),
        runtime,
        presenter,
        queue: MeshQueue {
            // Cargo supplies BEVY_ASSET_ROOT. This loads the real catalogue but
            // the empty object fixture needs no asset server, window or renderer.
            art: Some(
                StockArt::load(Mesh::from(Cuboid::new(1.0, 1.0, 1.0)))
                    .expect("stock catalogue and test source mesh"),
            ),
            ..MeshQueue::default()
        },
    };
    fixture.reside(&BTreeSet::from([chunk(0), chunk(1), chunk(2), chunk(3)]));
    fixture
}

impl Fixture {
    fn reside(&mut self, requested: &BTreeSet<ChunkId>) {
        self.runtime
            .set_interests(
                requested
                    .iter()
                    .map(|coordinate| ResidencyRequest {
                        id: format!("fixture/{}", coordinate.q),
                        center: WorldHex::new(coordinate.q * 16 + 15, 5),
                        radius: 0,
                        retention_radius: 0,
                        priority: 1,
                    })
                    .collect(),
            )
            .expect("bounded local interests");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let update = self.runtime.pump();
            assert!(update.failures.is_empty(), "{:?}", update.failures);
            let counts = self.runtime.counts();
            if counts.in_flight_jobs == 0 && counts.queued_chunks == 0 {
                break;
            }
            assert!(Instant::now() < deadline, "runtime did not settle");
            thread::sleep(Duration::from_millis(1));
        }
        let resident = self
            .runtime
            .resident_chunks()
            .map(|product| product.coordinate)
            .collect::<BTreeSet<_>>();
        assert_eq!(&resident, requested);
    }

    fn tick(&mut self, desired: &BTreeSet<ChunkId>) {
        self.queue
            .art
            .as_mut()
            .expect("stock art")
            .refresh(&self.runtime)
            .expect("resident art refresh");
        self.queue
            .tick(
                &mut self.world,
                &self.runtime,
                &mut self.presenter,
                desired,
                true,
            )
            .expect("bounded presentation transaction");
    }

    fn settle(&mut self, desired: &BTreeSet<ChunkId>) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            self.tick(desired);
            if self.queue.is_idle() {
                break;
            }
            assert!(Instant::now() < deadline, "mesh queue did not settle");
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            self.roots().keys().copied().collect::<BTreeSet<_>>(),
            *desired
        );
    }

    fn roots(&self) -> BTreeMap<ChunkId, Entity> {
        self.presenter
            .receipts()
            .map(|receipt| (receipt.coordinate, receipt.root))
            .collect()
    }

    fn root(&self, coordinate: ChunkId) -> Entity {
        *self.roots().get(&coordinate).expect("published root")
    }

    fn mesh_count(&self) -> usize {
        self.world
            .get_resource::<Assets<Mesh>>()
            .map_or(0, Assets::len)
    }

    fn side_vertices(&self, from_q: i64, toward_q: i64) -> usize {
        let from = WorldHex::new(from_q, 5);
        let toward = WorldHex::new(toward_q, 5);
        let a = self
            .presenter
            .origin()
            .local_hex(from)
            .expect("local column")
            .to_world(0.0);
        let b = self
            .presenter
            .origin()
            .local_hex(toward)
            .expect("local neighbor")
            .to_world(0.0);
        let direction = (b - a).normalize();
        let plane = ((a + b) * 0.5).dot(direction);
        let root = self.root(from.chunk());
        let assets = self.world.resource::<Assets<Mesh>>();
        self.world
            .get::<Children>(root)
            .expect("chunk-owned children")
            .iter()
            .filter_map(|entity| self.world.get::<Mesh3d>(entity))
            .map(|handle| {
                let mesh = assets.get(&handle.0).expect("owned mesh is present");
                let Some(VertexAttributeValues::Float32x3(positions)) =
                    mesh.attribute(Mesh::ATTRIBUTE_POSITION)
                else {
                    unreachable!("terrain mesh needs positions");
                };
                let Some(VertexAttributeValues::Float32x3(normals)) =
                    mesh.attribute(Mesh::ATTRIBUTE_NORMAL)
                else {
                    unreachable!("terrain mesh needs normals");
                };
                positions
                    .iter()
                    .zip(normals)
                    .filter(|(position, normal)| {
                        (Vec3::from_array(**position).dot(direction) - plane).abs() < 0.001
                            && Vec3::from_array(**normal).dot(direction) > 0.99
                    })
                    .count()
            })
            .sum()
    }
}

#[test]
fn real_water_admission_and_unload_repair_faces_without_remeshing_two_hop_roots() {
    let mut fixture = fixture();
    let initial = BTreeSet::from([chunk(1), chunk(2), chunk(3)]);
    fixture.settle(&initial);
    assert!(
        fixture.side_vertices(16, 15) > 0,
        "unpublished A cannot cull B"
    );
    assert_eq!(fixture.side_vertices(31, 32), 0, "B sees presented C");
    let before = fixture.roots();
    let all = BTreeSet::from([chunk(0), chunk(1), chunk(2), chunk(3)]);
    fixture.tick(&all);
    assert!(fixture.queue.active);
    assert_eq!(
        fixture.roots(),
        before,
        "starting a worker publishes nothing"
    );
    assert!(fixture.side_vertices(16, 15) > 0);
    fixture.settle(&all);
    assert_eq!(fixture.side_vertices(15, 16), 0);
    assert_eq!(fixture.side_vertices(16, 15), 0);
    assert_eq!(
        fixture.side_vertices(31, 32),
        0,
        "two-ring snapshot retains C"
    );
    for coordinate in [chunk(2), chunk(3)] {
        assert_eq!(Some(&fixture.root(coordinate)), before.get(&coordinate));
    }
    let admitted = fixture.roots();
    fixture.reside(&initial);
    assert!(fixture.runtime.resident_chunk(chunk(0)).is_none());
    assert_eq!(
        fixture.roots(),
        admitted,
        "runtime eviction does not retire presentation"
    );
    fixture.tick(&initial);
    assert_eq!(
        fixture.roots(),
        admitted,
        "survivor replacement must finish first"
    );
    fixture.settle(&initial);
    assert!(
        fixture.side_vertices(16, 15) > 0,
        "removal restores the outside water face"
    );
    assert_eq!(fixture.side_vertices(31, 32), 0);
    for coordinate in [chunk(2), chunk(3)] {
        assert_eq!(Some(&fixture.root(coordinate)), before.get(&coordinate));
    }
    let settled_publications = fixture.queue.published;
    fixture.tick(&initial);
    assert!(fixture.queue.is_idle());
    assert_eq!(
        fixture.queue.published, settled_publications,
        "halo dependencies do not oscillate"
    );
}

#[test]
fn returning_desired_chunk_cancels_real_retirement_without_touching_roots_or_assets() {
    let mut fixture = fixture();
    let both = BTreeSet::from([chunk(0), chunk(1)]);
    fixture.settle(&both);
    let roots = fixture.roots();
    let meshes = fixture.mesh_count();
    let published = fixture.queue.published;
    let discarded = fixture.queue.discarded;
    fixture.tick(&BTreeSet::from([chunk(0)]));
    assert!(fixture.queue.active);
    assert_eq!(fixture.roots(), roots);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        fixture.tick(&both);
        assert_eq!(
            fixture.roots(),
            roots,
            "stale retirement must never install its neighbor mesh"
        );
        assert_eq!(fixture.mesh_count(), meshes);
        if fixture.queue.is_idle() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "canceled retirement did not settle"
        );
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(fixture.queue.discarded, discarded + 1);
    assert_eq!(fixture.queue.published, published);
    assert_eq!(fixture.side_vertices(15, 16), 0);
}

#[test]
fn canceled_admission_cannot_publish_old_origin_after_rebase_and_new_work_recovers() {
    let mut fixture = fixture();
    let first = BTreeSet::from([chunk(0)]);
    let both = BTreeSet::from([chunk(0), chunk(1)]);
    fixture.settle(&first);
    fixture.tick(&both);
    assert!(fixture.queue.active);
    let discarded = fixture.queue.discarded;
    fixture.queue.cancel();
    let origin = RenderOrigin {
        column: WorldHex::new(-32, 16),
        level: -3,
    };
    fixture
        .queue
        .art
        .as_ref()
        .expect("stock art")
        .validate_rebase(&fixture.world, origin)
        .expect("art preflight");
    fixture
        .presenter
        .rebase(&mut fixture.world, origin)
        .expect("terrain rebase");
    fixture
        .queue
        .art
        .as_mut()
        .expect("stock art")
        .rebase(&mut fixture.world, origin)
        .expect("stock art rebase");
    let rebased = fixture.roots();
    let meshes = fixture.mesh_count();
    fixture.settle(&first);
    assert_eq!(fixture.queue.discarded, discarded + 1);
    assert_eq!(
        fixture.roots(),
        rebased,
        "old-origin transaction changed rebased roots"
    );
    assert_eq!(fixture.mesh_count(), meshes);
    assert_eq!(fixture.presenter.origin(), origin);
    assert!(fixture.side_vertices(15, 16) > 0);
    fixture.settle(&both);
    assert_eq!(fixture.side_vertices(15, 16), 0);
    assert_eq!(fixture.side_vertices(16, 15), 0);
    assert_eq!(fixture.presenter.origin(), origin);
}
