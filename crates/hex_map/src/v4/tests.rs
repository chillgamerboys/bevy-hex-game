#![expect(
    clippy::expect_used,
    reason = "Test-controlled fixtures and exact publication assertions"
)]

use std::collections::BTreeMap;

use bevy::{mesh::VertexAttributeValues, prelude::*};
use hex_core::{Headroom, HexCoord, TerrainRenderBatch, TilePos};
use hex_world_contracts::*;

use super::*;

fn run(bottom: i32, top: i32, material: &str) -> VoxelRun {
    VoxelRun {
        bottom,
        top,
        material: material.into(),
    }
}

fn fixture(origin: WorldHex) -> WorldPackage {
    let mut chunks: BTreeMap<ChunkId, ChunkPackage> = BTreeMap::new();
    for q in -1_i64..=1 {
        for r in -1_i64..=1 {
            if q.abs().max(r.abs()).max((q + r).abs()) > 1 {
                continue;
            }
            let position = origin
                .checked_add(WorldHex::new(q, r))
                .expect("fixture position");
            chunks
                .entry(position.chunk())
                .or_insert_with(|| ChunkPackage {
                    schema_version: SCHEMA_VERSION,
                    world_id: "fixture".into(),
                    coordinate: position.chunk(),
                    source_fingerprint: 12,
                    columns: Vec::new(),
                    features: Vec::new(),
                    semantics: ChunkSemantics::default(),
                    fingerprint: 0,
                })
                .columns
                .push(ColumnData {
                    position,
                    runs: if position == origin {
                        vec![run(-4, 0, "custom_rock"), run(3, 4, "custom_rock")]
                    } else {
                        vec![run(-4, 0, "custom_rock")]
                    },
                });
        }
    }
    let mut world = WorldPackage {
        manifest: WorldManifest {
            schema_version: SCHEMA_VERSION,
            world_id: "fixture".into(),
            compiler_version: "tests-v1".into(),
            source_fingerprint: 12,
            materials: vec![
                MaterialSpec {
                    id: "custom_rock".into(),
                    solid: true,
                    diggable: true,
                    color: [160, 90, 60, 255],
                },
                MaterialSpec {
                    id: "glass_object".into(),
                    solid: true,
                    diggable: false,
                    color: [30, 150, 200, 180],
                },
            ],
            regions: vec![RegionDescriptor {
                id: "region".into(),
                origin,
                radius: 1,
                source_fingerprint: 7,
            }],
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
    world.seal().expect("fixture seals");
    world
}

fn add_object(world: &mut WorldPackage, root: WorldHex, occupied: WorldHex) {
    world
        .chunks
        .get_mut(&root.chunk())
        .expect("root chunk")
        .semantics
        .objects
        .push(ObjectInstance {
            id: "object".into(),
            region_id: "region".into(),
            asset: "authored_statue".into(),
            origin: VoxelPosition {
                column: root,
                level: 0,
            },
            rotation: 0,
            occupancy: vec![ColumnData {
                position: occupied,
                runs: vec![run(1, 2, "glass_object")],
            }],
        });
    world.seal().expect("object seals");
}

fn publish(
    presenter: &mut TerrainPresenter,
    world: &mut World,
    package: &ChunkPackage,
    revision: u64,
) -> ChunkReceipt {
    let prepared = presenter.prepare(package, revision).expect("prepare");
    presenter.publish(world, prepared).expect("publish")
}

#[test]
fn huge_global_coordinates_are_subtracted_before_float_geometry() {
    let column = WorldHex::new((1_i64 << 54) + 15, -(1_i64 << 54));
    let package = fixture(column);
    let origin = RenderOrigin { column, level: 0 };
    let presenter = TerrainPresenter::new(&package.manifest, origin, 0.2).expect("presenter");
    let prepared = presenter
        .prepare(package.chunks.get(&column.chunk()).expect("chunk"), 1)
        .expect("prepare");
    assert!(prepared.logical_runs() >= 2);
    for batch in &prepared.batches {
        let Some(VertexAttributeValues::Float32x3(vertices)) = batch
            .mesh
            .as_ref()
            .expect("unmasked mesh")
            .attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            unreachable!("the shared terrain mesh has positions");
        };
        assert!(vertices
            .iter()
            .flatten()
            .all(|value| value.is_finite() && value.abs() < 10.0));
    }
    let position = VoxelPosition {
        column: column.checked_add(WorldHex::new(1, -1)).expect("offset"),
        level: -3,
    };
    assert_eq!(
        origin
            .global_voxel(origin.local_voxel(position).expect("local"))
            .expect("global"),
        position
    );
    assert!(origin.local_hex(WorldHex::new(i64::MIN, i64::MAX)).is_err());
    let vertical = RenderOrigin {
        column,
        level: i32::MAX - 5,
    };
    let position = VoxelPosition {
        column,
        level: i32::MAX - 2,
    };
    assert_eq!(
        vertical
            .local_voxel(position)
            .expect("vertical local")
            .level,
        3
    );
    assert_eq!(
        vertical
            .global_voxel(TilePos::new(HexCoord::ORIGIN, 3))
            .expect("vertical global"),
        position
    );
}

#[test]
fn stacked_ground_object_and_bridge_keep_exact_headroom_and_picking() {
    let column = WorldHex::new(15, 0);
    let mut package = fixture(column);
    add_object(&mut package, column, column);
    let mut presenter =
        TerrainPresenter::new(&package.manifest, RenderOrigin { column, level: 0 }, 1.0)
            .expect("presenter");
    let mut world = World::new();
    let receipt = publish(
        &mut presenter,
        &mut world,
        package.chunks.get(&column.chunk()).expect("chunk"),
        7,
    );
    assert_eq!(receipt.object_runs, 1);
    assert_eq!(receipt.unresolved_object_assets, 1);
    assert!(world
        .query::<&hex_core::SubstanceId>()
        .iter(&world)
        .all(|id| *id != hex_core::SubstanceId::AIR));
    let exact: BTreeMap<_, _> = world
        .query::<(Entity, &ResidentRun, &Headroom)>()
        .iter(&world)
        .filter(|(_, run, _)| run.position.column == column)
        .map(|(entity, run, headroom)| (run.position.level, (entity, run.clone(), headroom.0)))
        .collect();
    for (level, bottom, top, clearance) in
        [(-1, -4, 0, Some(1)), (1, 1, 2, Some(1)), (3, 3, 4, None)]
    {
        let (entity, exact, headroom) = exact.get(&level).expect("stacked run");
        assert_eq!(
            (exact.bottom, exact.top, exact.headroom),
            (bottom, top, clearance)
        );
        assert_eq!(
            *headroom,
            clearance.map_or(8, |value| i32::try_from(value).expect("small clearance"))
        );
        let hit = Vec3::new(0.0, f32::from(i16::try_from(top).expect("small top")), 0.0);
        let resolved: Vec<_> = world
            .query::<&TerrainRenderBatch>()
            .iter(&world)
            .filter_map(|batch| batch.resolve_hit(hit, Some(Vec3::Y)))
            .collect();
        assert_eq!(resolved, vec![*entity]);
    }
    // Side hits inside the two air gaps must not resolve to any logical interval.
    for height in [0.5, 2.5] {
        assert!(world
            .query::<&TerrainRenderBatch>()
            .iter(&world)
            .all(|batch| batch
                .resolve_hit(Vec3::new(0.0, height, 0.0), Some(Vec3::X))
                .is_none()));
    }
    assert_eq!(
        presenter
            .package(column.chunk())
            .expect("retained source")
            .semantics
            .objects
            .len(),
        1
    );
    assert!(world
        .resource::<Assets<StandardMaterial>>()
        .iter()
        .any(|(_, material)| matches!(material.alpha_mode, AlphaMode::Blend)));
}

#[test]
fn owner_unloaded_object_occupancy_still_has_exact_geometry() {
    let root = WorldHex::new(15, 0);
    let occupied = WorldHex::new(16, 0);
    let mut package = fixture(root);
    add_object(&mut package, root, occupied);
    let mut presenter = TerrainPresenter::new(
        &package.manifest,
        RenderOrigin {
            column: occupied,
            level: 0,
        },
        1.0,
    )
    .expect("presenter");
    let mut world = World::new();
    let receipt = publish(
        &mut presenter,
        &mut world,
        package
            .chunks
            .get(&occupied.chunk())
            .expect("occupancy chunk"),
        0,
    );
    assert_eq!(receipt.object_runs, 1);
    assert_eq!(receipt.unresolved_object_assets, 0);
    assert!(presenter.package(root.chunk()).is_none());
    let objects: Vec<_> = world
        .query::<&ResidentRun>()
        .iter(&world)
        .filter(|run| run.source == RunSource::StaticObject)
        .cloned()
        .collect();
    assert_eq!(objects.len(), 1);
    assert_eq!(
        objects.first().expect("occupancy run").position,
        VoxelPosition {
            column: occupied,
            level: 1
        }
    );
}

#[test]
fn replace_unload_and_clear_clean_only_owned_assets_and_entities() {
    let column = WorldHex::new(15, 0);
    let package = fixture(column);
    let mut presenter =
        TerrainPresenter::new(&package.manifest, RenderOrigin { column, level: 0 }, 1.0)
            .expect("presenter");
    let mut world = World::new();
    let chunk = package.chunks.get(&column.chunk()).expect("chunk");
    let other = package
        .chunks
        .values()
        .find(|entry| entry.coordinate != chunk.coordinate)
        .expect("other chunk");
    let first = publish(&mut presenter, &mut world, chunk, 0);
    let untouched = publish(&mut presenter, &mut world, other, 0);
    let stale_meshes: Vec<_> = world
        .query::<(&ChildOf, &Mesh3d)>()
        .iter(&world)
        .filter(|(parent, _)| parent.parent() == first.root)
        .map(|(_, mesh)| mesh.0.id())
        .collect();
    let material_count = world.resource::<Assets<StandardMaterial>>().len();
    let mut revised = chunk.clone();
    revised
        .columns
        .iter_mut()
        .find(|entry| entry.position == column)
        .expect("center")
        .runs
        .push(run(6, 7, "custom_rock"));
    revised.seal().expect("revised package");
    let replacement = publish(&mut presenter, &mut world, &revised, 1);
    assert_ne!(first.root, replacement.root);
    assert_eq!(replacement.revision, 1);
    assert!(world.get_entity(first.root).is_err());
    assert!(world.get_entity(untouched.root).is_ok());
    assert!(stale_meshes
        .iter()
        .all(|id| world.resource::<Assets<Mesh>>().get(*id).is_none()));
    assert_eq!(
        world.resource::<Assets<Mesh>>().len(),
        replacement.meshes + untouched.meshes
    );
    assert_eq!(
        world.resource::<Assets<StandardMaterial>>().len(),
        material_count
    );
    let duplicate = publish(&mut presenter, &mut world, &revised, 1);
    assert_eq!(duplicate.root, replacement.root);
    let stale = presenter.prepare(chunk, 0).expect("old preparation");
    assert!(presenter.publish(&mut world, stale).is_err());
    let conflicting = presenter
        .prepare(chunk, 1)
        .expect("conflicting equal revision");
    assert!(presenter.publish(&mut world, conflicting).is_err());
    presenter
        .remove(&mut world, revised.coordinate)
        .expect("remove one");
    assert_eq!(world.resource::<Assets<Mesh>>().len(), untouched.meshes);
    assert!(world.get_entity(untouched.root).is_ok());
    presenter.clear(&mut world);
    assert_eq!(world.resource::<Assets<Mesh>>().len(), 0);
    assert_eq!(world.resource::<Assets<StandardMaterial>>().len(), 0);
    // Bevy 0.19 retains resource entities; only our owned scene entities retire.
    assert_eq!(world.query::<&ResidentRun>().iter(&world).count(), 0);
    assert_eq!(world.query::<&TerrainRenderBatch>().iter(&world).count(), 0);
    assert_eq!(world.query::<&ResidentChunk>().iter(&world).count(), 0);
}

#[test]
fn rebase_changes_local_geometry_preserves_global_identity_and_rejects_old_jobs() {
    let column = WorldHex::new(15, 0);
    let package = fixture(column);
    let origin = RenderOrigin { column, level: 0 };
    let mut presenter = TerrainPresenter::new(&package.manifest, origin, 1.0).expect("presenter");
    let mut world = World::new();
    let chunk = package.chunks.get(&column.chunk()).expect("chunk");
    let first = publish(&mut presenter, &mut world, chunk, 3);
    let stale = presenter
        .preparer()
        .prepare(chunk, 4)
        .expect("worker product");
    let next = RenderOrigin {
        column: WorldHex::new(16, -1),
        level: 2,
    };
    let changed = presenter.rebase(&mut world, next).expect("rebase");
    let receipt = changed.first().expect("replacement");
    assert_ne!(receipt.root, first.root);
    assert_eq!(
        (receipt.revision, receipt.fingerprint),
        (first.revision, first.fingerprint)
    );
    assert_eq!(world.resource::<Assets<Mesh>>().len(), first.meshes);
    for (exact, local) in world.query::<(&ResidentRun, &TilePos)>().iter(&world) {
        assert_eq!(next.global_voxel(*local).expect("global"), exact.position);
    }
    assert!(presenter.publish(&mut world, stale).is_err());
    let roots: Vec<_> = presenter.receipts().map(|receipt| receipt.root).collect();
    let assets = world.resource::<Assets<Mesh>>().len();
    assert!(presenter
        .rebase(
            &mut world,
            RenderOrigin {
                column: WorldHex::new(50_000, 0),
                level: 0
            }
        )
        .is_err());
    assert_eq!(presenter.origin(), next);
    assert_eq!(
        presenter
            .receipts()
            .map(|receipt| receipt.root)
            .collect::<Vec<_>>(),
        roots
    );
    assert_eq!(world.resource::<Assets<Mesh>>().len(), assets);
}

#[test]
fn bad_hash_and_operational_budget_fail_without_mutating_existing_world() {
    let column = WorldHex::new(15, 0);
    let package = fixture(column);
    let mut presenter = TerrainPresenter::with_limits(
        &package.manifest,
        RenderOrigin { column, level: 0 },
        1.0,
        PresentationLimits {
            max_resident_chunks: 1,
            ..PresentationLimits::default()
        },
    )
    .expect("presenter");
    let mut world = World::new();
    let first = package.chunks.get(&column.chunk()).expect("chunk");
    let receipt = publish(&mut presenter, &mut world, first, 0);
    let other = package
        .chunks
        .values()
        .find(|entry| entry.coordinate != first.coordinate)
        .expect("other chunk");
    let next = presenter
        .prepare(other, 0)
        .expect("world size is not capped by presentation budget");
    assert!(presenter.publish(&mut world, next).is_err());
    let mut corrupt = first.clone();
    corrupt.fingerprint ^= 1;
    assert!(presenter.prepare(&corrupt, 1).is_err());
    assert_eq!(
        presenter.receipts().next().expect("unchanged root").root,
        receipt.root
    );
    assert_eq!(world.resource::<Assets<Mesh>>().len(), receipt.meshes);
}

#[test]
fn shared_mesh_engine_keeps_negative_bottom_faces_and_resident_seam_walls() {
    let column = WorldHex::new(15, 0);
    let package = fixture(column);
    let presenter =
        TerrainPresenter::new(&package.manifest, RenderOrigin { column, level: 0 }, 1.0)
            .expect("presenter");
    let right = package
        .chunks
        .get(&WorldHex::new(16, 0).chunk())
        .expect("right chunk");
    let prepared = presenter
        .prepare(right, 0)
        .expect("prepare right at non-aligned origin");
    let mut bottom_faces = 0;
    let mut seam_faces = 0;
    for batch in &prepared.batches {
        let Some(VertexAttributeValues::Float32x3(positions)) = batch
            .mesh
            .as_ref()
            .expect("unmasked mesh")
            .attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            unreachable!("positions");
        };
        let Some(VertexAttributeValues::Float32x3(normals)) = batch
            .mesh
            .as_ref()
            .expect("unmasked mesh")
            .attribute(Mesh::ATTRIBUTE_NORMAL)
        else {
            unreachable!("normals");
        };
        for ([x, y, _z], [nx, ny, _nz]) in positions.iter().zip(normals) {
            bottom_faces += usize::from((*y + 4.0).abs() < 0.001 && *ny < -0.9);
            seam_faces += usize::from(*x < 1.0 && *nx < -0.5 && ny.abs() < 0.1);
        }
    }
    assert!(
        bottom_faces > 0,
        "negative intervals have their exposed underside"
    );
    assert!(
        seam_faces > 0,
        "cross-storage-chunk wall remains even at an unaligned origin"
    );
}

#[test]
fn preparer_and_products_can_cross_worker_boundaries() {
    fn send_sync<T: Send + Sync>() {}
    fn send<T: Send>() {}
    send_sync::<TerrainPreparer>();
    send::<PreparedChunk>();
}

#[test]
fn static_occupancy_overrides_terrain_by_exact_interval_without_voxel_expansion() {
    let column = WorldHex::new(8, 8);
    let mut package = fixture(column);
    add_object(&mut package, column, column);
    package
        .chunks
        .get_mut(&column.chunk())
        .expect("chunk")
        .semantics
        .objects
        .first_mut()
        .expect("object")
        .occupancy
        .first_mut()
        .expect("object column")
        .runs = vec![run(-2, 2, "glass_object")];
    package.seal().expect("overlapping source seals");
    let mut presenter =
        TerrainPresenter::new(&package.manifest, RenderOrigin { column, level: 0 }, 1.0)
            .expect("presenter");
    let mut world = World::new();
    publish(
        &mut presenter,
        &mut world,
        package.chunks.get(&column.chunk()).expect("chunk"),
        0,
    );
    let mut intervals: Vec<_> = world
        .query::<&ResidentRun>()
        .iter(&world)
        .filter(|run| run.position.column == column)
        .map(|run| (run.bottom, run.top, run.material.clone(), run.headroom))
        .collect();
    intervals.sort();
    assert_eq!(
        intervals,
        vec![
            (-4, -2, "custom_rock".into(), Some(0)),
            (-2, 2, "glass_object".into(), Some(1)),
            (3, 4, "custom_rock".into(), None)
        ]
    );
}

#[test]
fn translucent_neighbor_does_not_cull_the_opaque_wall_behind_it() {
    let column = WorldHex::new(8, 8);
    let mut package = fixture(column);
    let glass = WorldHex::new(7, 8);
    package
        .chunks
        .get_mut(&glass.chunk())
        .expect("chunk")
        .columns
        .iter_mut()
        .find(|entry| entry.position == glass)
        .expect("left neighbor")
        .runs = vec![run(-4, 0, "glass_object")];
    package.seal().expect("glass neighboring stone");
    let presenter =
        TerrainPresenter::new(&package.manifest, RenderOrigin { column, level: 0 }, 1.0)
            .expect("presenter");
    let prepared = presenter
        .prepare(package.chunks.get(&column.chunk()).expect("chunk"), 0)
        .expect("prepare");
    let mut wall_vertices = 0;
    for batch in &prepared.batches {
        if batch.material.id != "custom_rock" {
            continue;
        }
        let Some(VertexAttributeValues::Float32x3(positions)) = batch
            .mesh
            .as_ref()
            .expect("unmasked mesh")
            .attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            unreachable!("positions");
        };
        let Some(VertexAttributeValues::Float32x3(normals)) = batch
            .mesh
            .as_ref()
            .expect("unmasked mesh")
            .attribute(Mesh::ATTRIBUTE_NORMAL)
        else {
            unreachable!("normals");
        };
        for ([x, y, z], [nx, ny, _nz]) in positions.iter().zip(normals) {
            if (*x + 0.866_025_4).abs() < 0.001
                && z.abs() <= 0.501
                && *y <= 0.001
                && *nx < -0.9
                && ny.abs() < 0.01
            {
                wall_vertices += 1;
            }
        }
    }
    assert_eq!(
        wall_vertices, 4,
        "the rock face behind adjacent glass is preserved"
    );
}

#[test]
fn derived_union_keeps_more_runs_than_either_wire_column_limit() {
    let terrain: Vec<_> = (0..4096)
        .map(|index| run(4 * index, 4 * index + 1, "custom_rock"))
        .collect();
    let objects: Vec<_> = (0..4096)
        .map(|index| run(4 * index + 2, 4 * index + 3, "glass_object"))
        .collect();
    let union = super::prepare::union_runs(&terrain, &objects).expect("derived interval union");
    assert_eq!(union.len(), 8192);
    assert_eq!(union.first().expect("first").0.bottom, 0);
    assert_eq!(union.last().expect("last").0.top, 16_383);
    assert_eq!(
        union
            .iter()
            .filter(|(_, source)| *source == RunSource::StaticObject)
            .count(),
        4096
    );
}

#[test]
fn explicit_preparation_run_budget_rejects_whole_product_without_truncating() {
    let column = WorldHex::new(8, 8);
    let package = fixture(column);
    let presenter = TerrainPresenter::with_limits(
        &package.manifest,
        RenderOrigin { column, level: 0 },
        1.0,
        PresentationLimits {
            max_runs_per_chunk: 1,
            ..PresentationLimits::default()
        },
    )
    .expect("small operational budget");
    let result = presenter.prepare(package.chunks.get(&column.chunk()).expect("chunk"), 0);
    assert!(matches!(result, Err(error) if error.to_string().contains("max_runs_per_chunk")));
    assert_eq!(presenter.receipts().len(), 0);
    package.validate().expect("source remains a valid world");
}

fn tall_object_fixture(column: WorldHex) -> WorldPackage {
    let mut package = fixture(column);
    add_object(&mut package, column, column);
    let root = package.chunks.get_mut(&column.chunk()).expect("root");
    root.semantics
        .objects
        .first_mut()
        .expect("object")
        .occupancy
        .first_mut()
        .expect("occupancy")
        .runs = vec![run(1, 6, "glass_object")];
    let terrain = root
        .columns
        .iter_mut()
        .find(|terrain| terrain.position == column)
        .expect("terrain");
    terrain.runs = vec![run(-4, 0, "custom_rock"), run(9, 10, "custom_rock")];
    package.seal().expect("tall object source");
    package
}

fn mask(column: WorldHex, bottom: i32, top: i32, material: &str) -> Vec<ColumnData> {
    vec![ColumnData {
        position: column,
        runs: vec![run(bottom, top, material)],
    }]
}

#[test]
fn partial_object_suppression_splits_only_render_geometry_and_retains_exact_stacks() {
    let column = WorldHex::new(15, 0);
    let package = tall_object_fixture(column);
    let presenter =
        TerrainPresenter::new(&package.manifest, RenderOrigin { column, level: 0 }, 1.0)
            .expect("presenter");
    let chunk = package.chunks.get(&column.chunk()).expect("chunk");
    let ordinary = presenter.prepare(chunk, 4).expect("ordinary");
    let partial = presenter
        .prepare_with_suppressed_occupancy(chunk, 4, &mask(column, 2, 4, "glass_object"))
        .expect("partial");
    assert_eq!(partial.logical_runs(), ordinary.logical_runs());
    assert_ne!(
        partial.suppression_fingerprint(),
        ordinary.suppression_fingerprint()
    );
    let batch = partial
        .batches
        .iter()
        .find(|batch| batch.material.id == "glass_object")
        .expect("object batch");
    assert_eq!(batch.runs.len(), 1);
    let logical = &batch.runs.first().expect("logical run").exact;
    assert_eq!(
        (logical.bottom, logical.top, logical.headroom),
        (1, 6, Some(3))
    );
    let Some(VertexAttributeValues::Float32x3(positions)) = batch
        .mesh
        .as_ref()
        .expect("partial proxy")
        .attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        unreachable!("mesh positions");
    };
    assert!(positions.iter().any(|position| position
        .get(1)
        .is_some_and(|height| (*height - 2.0).abs() < 0.001)));
    assert!(positions.iter().any(|position| position
        .get(1)
        .is_some_and(|height| (*height - 4.0).abs() < 0.001)));
    assert!(!positions.iter().any(|position| position
        .get(1)
        .is_some_and(|height| *height > 2.0 && *height < 4.0)));
    let ordinary_terrain: Vec<_> = ordinary
        .batches
        .iter()
        .filter(|batch| batch.material.id == "custom_rock")
        .map(|batch| batch.mesh.as_ref().expect("terrain").count_vertices())
        .collect();
    let masked_terrain: Vec<_> = partial
        .batches
        .iter()
        .filter(|batch| batch.material.id == "custom_rock")
        .map(|batch| batch.mesh.as_ref().expect("terrain").count_vertices())
        .collect();
    assert_eq!(masked_terrain, ordinary_terrain);
}

#[test]
fn full_suppression_preserves_logical_and_pick_metadata_without_object_meshes() {
    let column = WorldHex::new(15, 0);
    let package = tall_object_fixture(column);
    let mut presenter =
        TerrainPresenter::new(&package.manifest, RenderOrigin { column, level: 0 }, 1.0)
            .expect("presenter");
    let chunk = package.chunks.get(&column.chunk()).expect("chunk");
    let suppressed = presenter
        .prepare_with_suppressed_occupancy(chunk, 1, &chunk.semantics.occupancy)
        .expect("full suppression");
    let object_batch = suppressed
        .batches
        .iter()
        .find(|batch| batch.material.id == "glass_object")
        .expect("logical object batch");
    assert!(object_batch.mesh.is_none());
    assert_eq!(object_batch.runs.len(), 1);
    let mut world = World::new();
    let receipt = presenter
        .publish(&mut world, suppressed)
        .expect("publish logical metadata");
    assert_eq!(receipt.object_runs, 1);
    let object = world
        .query::<(Entity, &ResidentRun, &Headroom)>()
        .iter(&world)
        .find(|(_, run, _)| run.source == RunSource::StaticObject)
        .map(|(entity, run, headroom)| (entity, run.clone(), headroom.0))
        .expect("exact object remains");
    assert_eq!((object.1.bottom, object.1.top, object.2), (1, 6, 3));
    assert_eq!(
        world
            .query::<&TerrainRenderBatch>()
            .iter(&world)
            .filter_map(|batch| batch.resolve_hit(Vec3::new(0.0, 6.0, 0.0), Some(Vec3::Y)))
            .collect::<Vec<_>>(),
        vec![object.0]
    );
    assert_eq!(
        world.query::<&Mesh3d>().iter(&world).count(),
        receipt.meshes
    );
    assert_eq!(
        world.query::<&ResidentRun>().iter(&world).count(),
        receipt.logical_runs
    );
}

#[test]
fn suppression_rejects_terrain_wrong_material_outside_chunk_and_noncanonical_masks() {
    let column = WorldHex::new(15, 0);
    let package = tall_object_fixture(column);
    let presenter =
        TerrainPresenter::new(&package.manifest, RenderOrigin { column, level: 0 }, 1.0)
            .expect("presenter");
    let chunk = package.chunks.get(&column.chunk()).expect("chunk");
    let outside = column
        .checked_add(WorldHex::new(1, 0))
        .expect("outside chunk");
    for bad in [
        mask(column, -4, 0, "custom_rock"),
        mask(column, 1, 6, "custom_rock"),
        mask(column, 0, 6, "glass_object"),
        mask(outside, 1, 6, "glass_object"),
        vec![ColumnData {
            position: column,
            runs: Vec::new(),
        }],
        vec![ColumnData {
            position: column,
            runs: vec![run(1, 3, "glass_object"), run(3, 6, "glass_object")],
        }],
    ] {
        assert!(presenter
            .prepare_with_suppressed_occupancy(chunk, 1, &bad)
            .is_err());
    }
    let mut duplicate = mask(column, 1, 2, "glass_object");
    duplicate.extend(duplicate.clone());
    assert!(presenter
        .prepare_with_suppressed_occupancy(chunk, 1, &duplicate)
        .is_err());
    package.validate().expect("source untouched");
}

#[test]
fn changed_suppression_can_replace_same_revision_without_touching_other_roots() {
    let column = WorldHex::new(15, 0);
    let package = tall_object_fixture(column);
    let mut presenter =
        TerrainPresenter::new(&package.manifest, RenderOrigin { column, level: 0 }, 1.0)
            .expect("presenter");
    let chunk = package.chunks.get(&column.chunk()).expect("chunk");
    let other = package
        .chunks
        .values()
        .find(|chunk| chunk.coordinate != column.chunk())
        .expect("neighbor chunk");
    let mut world = World::new();
    let original = publish(&mut presenter, &mut world, chunk, 7);
    let neighbor = publish(&mut presenter, &mut world, other, 2);
    let prepared = presenter
        .prepare_with_suppressed_occupancy(chunk, 7, &chunk.semantics.occupancy)
        .expect("suppressed");
    let fingerprint = prepared.suppression_fingerprint();
    let replacement = presenter
        .publish(&mut world, prepared)
        .expect("same revision replacement");
    assert_ne!(original.root, replacement.root);
    assert_eq!(replacement.revision, original.revision);
    assert_eq!(replacement.fingerprint, original.fingerprint);
    assert_eq!(replacement.suppression_fingerprint, fingerprint);
    assert!(world.get_entity(original.root).is_err());
    assert!(world.get_entity(neighbor.root).is_ok());
    let repeated = presenter
        .prepare_with_suppressed_occupancy(chunk, 7, &chunk.semantics.occupancy)
        .expect("repeat");
    assert_eq!(
        presenter.publish(&mut world, repeated).expect("idempotent"),
        replacement
    );
    let older = presenter
        .prepare_with_suppressed_occupancy(chunk, 6, &[])
        .expect("older");
    assert!(presenter.publish(&mut world, older).is_err());
}

#[test]
fn rebase_preserves_full_suppression_and_failure_keeps_the_prior_mask() {
    let column = WorldHex::new(15, 0);
    let package = tall_object_fixture(column);
    let mut presenter =
        TerrainPresenter::new(&package.manifest, RenderOrigin { column, level: 0 }, 1.0)
            .expect("presenter");
    let chunk = package.chunks.get(&column.chunk()).expect("chunk");
    let prepared = presenter
        .prepare_with_suppressed_occupancy(chunk, 3, &chunk.semantics.occupancy)
        .expect("suppressed");
    let mut world = World::new();
    let before = presenter.publish(&mut world, prepared).expect("published");
    let origin = RenderOrigin {
        column: column.checked_add(WorldHex::new(1, 0)).expect("new origin"),
        level: 1,
    };
    let rebased = presenter
        .rebase(&mut world, origin)
        .expect("rebase with mask");
    let after = rebased.first().expect("receipt");
    assert_eq!(
        after.suppression_fingerprint,
        before.suppression_fingerprint
    );
    assert_eq!(after.meshes, before.meshes);
    assert_eq!(after.object_runs, before.object_runs);
    assert_eq!(after.logical_runs, before.logical_runs);
    assert!(presenter
        .rebase(
            &mut world,
            RenderOrigin {
                column: WorldHex::new(1_000_000, 0),
                level: 0
            }
        )
        .is_err());
    assert_eq!(presenter.receipts().next(), Some(after));
    assert_eq!(world.query::<&Mesh3d>().iter(&world).count(), before.meshes);
}
