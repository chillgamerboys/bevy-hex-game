use super::super::tests::{fixture_for, mesh_bits, remove_cells};
use super::*;
use hex_world_contracts::VoxelPosition;
use hex_world_runtime::{
    CancellationToken, FiniteChunkCheckpoint, FiniteSessionHeader, RuntimeConfig, RuntimeResult,
    WorldRuntime,
};
use std::sync::Arc;

#[test]
fn garden_feature_selection_is_exact_bounded_and_all_or_nothing() {
    let f = fixture_for("grand/garden-arcades", "structure/grand-garden-court").expect("fixture");
    let mut manifest = f.runtime.manifest().clone();
    assert!(features(&manifest, false)
        .expect("plain has no garden")
        .is_empty());
    assert!(features(&manifest, true).is_err());
    for (id, asset) in OBJECTS {
        manifest.features.push(FeatureSummary {
            id: id.into(),
            region_id: "tree".into(),
            kind: "structure".into(),
            anchor: f.object.origin,
            asset: Some(asset.into()),
        });
    }
    assert_eq!(features(&manifest, true).expect("complete names").len(), 4);
    manifest.features.first_mut().expect("first").asset = Some(TREE_ASSET.into());
    assert!(features(&manifest, true).is_err());
    manifest.features.remove(0);
    assert!(
        features(&manifest, false).is_err(),
        "partial sets never silently publish"
    );
}

#[test]
fn garden_edits_survive_unloaded_resume_and_whole_object_handoff() {
    let mut f = fixture_for("grand/garden-arcades", "structure/grand-garden-court")
        .expect("two-chunk architecture");
    let (geometry, mesh) =
        TreeGeometry::with_limits(&f.object, 0.35, f.palette.clone(), &f.edits, LIMITS)
            .expect("bounded proxy");
    let before = mesh_bits(&mesh).expect("mesh");
    let mut world = World::new();
    world.init_resource::<Assets<Mesh>>();
    world.init_resource::<Assets<StandardMaterial>>();
    let mut object = spawn_landmark(&mut world, f.object.id.clone(), geometry, mesh);
    remove_cells(
        &mut f.edits,
        "ground-only",
        &[VoxelPosition {
            column: f.object.origin.column,
            level: 9,
        }],
    )
    .expect("ground removal");
    object.sync(&mut world, &f.edits, false);
    assert_eq!(
        mesh_bits(
            world
                .resource::<Assets<Mesh>>()
                .get(&object.mesh)
                .expect("mesh")
        )
        .expect("bits"),
        before
    );
    let cut = VoxelPosition {
        column: WorldHex::new(16, 0),
        level: 25,
    };
    remove_cells(&mut f.edits, "arch-cut", &[cut]).expect("actual object removal");
    object.sync(&mut world, &f.edits, false);
    assert!(
        object.visible,
        "partial detailed publication keeps the entire masked silhouette"
    );
    let after = mesh_bits(
        world
            .resource::<Assets<Mesh>>()
            .get(&object.mesh)
            .expect("mesh"),
    )
    .expect("bits");
    assert_ne!(after, before);
    assert!(object
        .geometry
        .masked
        .iter()
        .find(|c| c.position == cut.column)
        .expect("column")
        .material_at(cut.level)
        .is_none());
    object.sync(&mut world, &f.edits, true);
    assert!(!object.visible);
    object.sync(&mut world, &f.edits, false);
    assert!(object.visible);
    assert_eq!(
        object.geometry.observed_revisions.len(),
        2,
        "original full footprint remains after cuts"
    );
    f.runtime.set_interests(vec![]).expect("evict");
    f.runtime.pump();
    f.edits.sync_residency(&f.runtime);
    assert_eq!(f.edits.resident_source_count(), 0);
    let encoded = ron::to_string(&(
        f.edits.checkpoint_header(),
        f.edits
            .checkpoint_partitions()
            .collect::<RuntimeResult<Vec<_>>>()
            .expect("partitions"),
    ))
    .expect("codec");
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
    .expect("resume");
    let (_, mesh) = TreeGeometry::with_limits(&f.object, 0.35, f.palette, &resumed, LIMITS)
        .expect("masked first frame");
    assert_eq!(mesh_bits(&mesh).expect("restored"), after);
    assert_eq!(
        resumed.resident_source_count(),
        0,
        "distant architecture pins no terrain"
    );
    assert_eq!(
        world
            .resource::<Assets<StandardMaterial>>()
            .get(&object.material)
            .expect("material")
            .alpha_mode,
        AlphaMode::Opaque
    );
    let entity = object.entity;
    world.insert_resource(Cache {
        identity: (7, 0),
        tree: None,
        garden: vec![object],
    });
    retire_stale(&mut world, (7, 0));
    assert!(world.contains_resource::<Cache>());
    retire_stale(&mut world, (8, 0));
    assert!(!world.contains_resource::<Cache>());
    assert!(world.get_entity(entity).is_err());
    assert!(world.resource::<Assets<Mesh>>().is_empty());
    assert!(world.resource::<Assets<StandardMaterial>>().is_empty());
}

#[test]
fn garden_budget_refuses_before_any_asset_is_spawned() {
    let mut f =
        fixture_for("grand/garden-arcades", "structure/grand-garden-court").expect("fixture");
    f.object.occupancy = (0..=LIMITS.columns)
        .map(|q| ColumnData {
            position: WorldHex::new(i64::try_from(q).expect("small q"), 0),
            runs: vec![VoxelRun {
                bottom: 10,
                top: 20,
                material: "timber".into(),
            }],
        })
        .collect();
    assert!(TreeGeometry::with_limits(&f.object, 0.35, f.palette, &f.edits, LIMITS).is_err());
}

#[test]
fn garden_fragment_overflow_hides_stale_surface_without_retaining_oversized_mask() {
    let mut f =
        fixture_for("grand/garden-arcades", "structure/grand-garden-court").expect("fixture");
    let limits = SurfaceLimits { runs: 3, ..LIMITS };
    let (geometry, mesh) = TreeGeometry::with_limits(&f.object, 0.35, f.palette, &f.edits, limits)
        .expect("three original runs fit");
    let original = geometry.masked.clone();
    let mut world = World::new();
    world.init_resource::<Assets<Mesh>>();
    world.init_resource::<Assets<StandardMaterial>>();
    let mut object = spawn_landmark(&mut world, f.object.id, geometry, mesh);
    object.sync(&mut world, &f.edits, false);
    assert!(object.visible);
    remove_cells(
        &mut f.edits,
        "split-arch",
        &[VoxelPosition {
            column: WorldHex::new(16, 0),
            level: 25,
        }],
    )
    .expect("cut splits one run");
    object.sync(&mut world, &f.edits, false);
    assert!(
        !object.visible,
        "never show the old intact arch after a failed mask"
    );
    assert_eq!(
        object.geometry.masked, original,
        "retained occupancy stays bounded"
    );
    object.sync(&mut world, &f.edits, false);
    assert!(
        !object.visible,
        "an unchanged revision cannot resurrect stale geometry"
    );
}

#[test]
fn garden_vertex_budget_rejects_before_appending_face_buffers() {
    let mut surface = Surface {
        max_vertices: 3,
        ..default()
    };
    assert!(surface
        .polygon(&[Vec3::ZERO, Vec3::X, Vec3::X + Vec3::Y, Vec3::Y], [1.0; 4])
        .is_err());
    assert!(surface.positions.is_empty());
    assert!(surface.normals.is_empty());
    assert!(surface.colors.is_empty());
    assert!(surface.indices.is_empty());
}

#[test]
#[ignore = "requires HEX_GRAND_WORLD pointing to the immutable dressed Grand package"]
fn actual_grand_garden_loads_exact_geometry_without_resident_sources() {
    let directory =
        std::path::PathBuf::from(std::env::var_os("HEX_GRAND_WORLD").expect("explicit package"));
    let overview: hex_schematic::v4::northern::NorthernOverview = ron::from_str(
        &std::fs::read_to_string(directory.join("grand-overview.ron")).expect("overview"),
    )
    .expect("schema");
    let source = Arc::new(
        FileChunkSource::open_workspace(&directory, IoLimits::default()).expect("package"),
    );
    assert_eq!(source.manifest().fingerprint, overview.package_fingerprint);
    let runtime = WorldRuntime::new(source.clone(), RuntimeConfig::default()).expect("runtime");
    let [min_level, max_level] = overview.level_bounds;
    let edits = FiniteWorldSession::streamed(&runtime, min_level, max_level).expect("edits");
    let loaded =
        load_source(&source, overview.level_height, &edits, true).expect("exact named garden");
    assert_eq!(loaded.len(), 4);
    let columns: usize = loaded.iter().map(|(_, g, _)| g.authored.len()).sum();
    let runs: usize = loaded
        .iter()
        .flat_map(|(_, g, _)| &g.authored)
        .map(|c| c.runs.len())
        .sum();
    let vertices: usize = loaded.iter().map(|(_, _, m)| m.count_vertices()).sum();
    assert_eq!(runtime.resident_chunks().count(), 0);
    assert_eq!(edits.resident_source_count(), 0);
    if overview.package_fingerprint == 4_086_254_255_885_708_135 {
        assert_eq!((columns, runs), (216, 307));
    }
    assert!(vertices > 0 && vertices <= 4 * LIMITS.vertices);
    for (_, geometry, mesh) in &loaded {
        assert_eq!(
            geometry.authored, geometry.masked,
            "pristine source remains exact"
        );
        assert_eq!(
            mesh_bits(mesh),
            mesh_bits(
                &surface(&geometry.authored, overview.level_height, &geometry.palette)
                    .expect("independent original surface")
            )
        );
    }
    println!("GRAND_GARDEN_PROXY package={} objects=4 columns={columns} runs={runs} vertices={vertices} resident_sources=0",overview.package_fingerprint);
}
