use super::*;
use crate::v4::{RenderOrigin, TerrainPresenter};
use hex_core::TerrainRenderBatch;
use hex_world_contracts::{VoxelRun, WorldPackage};

fn fixture() -> WorldPackage {
    let mut p = super::super::tests::fixture(WorldHex::new(8, 8));
    p.manifest.materials.first_mut().expect("rock").id = "stone".into();
    for chunk in p.chunks.values_mut() {
        chunk.columns.clear();
        for q in -3_i64..=3 {
            for r in -3_i64..=3 {
                if q.abs().max(r.abs()).max((q + r).abs()) <= 3 {
                    chunk.columns.push(ColumnData {
                        position: WorldHex::new(8 + q, 8 + r),
                        runs: vec![VoxelRun {
                            bottom: -4,
                            top: 4,
                            material: "stone".into(),
                        }],
                    });
                }
            }
        }
    }
    p.manifest.regions.first_mut().expect("region").radius = 3;
    p.seal().expect("fixture");
    p
}
fn overview(p: &WorldPackage) -> NorthernOverview {
    NorthernOverview {
        terrain_surface: None,
        version: 1,
        source_fingerprint: 12,
        package_fingerprint: p.manifest.fingerprint,
        world_id: "grand-v4".into(),
        hex_radius: 1.0,
        level_height: 1.0,
        vertical_offset: 0.0,
        radius: 64,
        level_bounds: [-4, 40],
        sea_level: -10.0,
        origin_xz: [-64.0, -64.0],
        spacing: 8.0,
        width: 32,
        height: 32,
        bed_heights: vec![4.0; 1024],
        surface_materials: vec![
            u16::try_from(
                p.manifest
                    .materials
                    .iter()
                    .position(|m| m.id == "stone")
                    .expect("stone material")
            )
            .expect("palette index");
            1024
        ],
        materials: p.manifest.materials.clone(),
        player_spawn: [0.0; 3],
        anchors: BTreeMap::new(),
        islands: vec![],
        tree_count: 0,
        forest: None,
        ground_cover: None,
        inland_water: None,
        review_cameras: BTreeMap::new(),
        building_count: 0,
    }
}
fn triangles(prepared: &PreparedChunk, source: Option<RunSource>) -> Vec<([u32; 9], String)> {
    let mut triangles = vec![];
    for batch in &prepared.batches {
        if source.is_some_and(|s| batch.runs.iter().any(|r| r.exact.source != s)) {
            continue;
        }
        let Some(mesh) = &batch.mesh else {
            continue;
        };
        let Some(VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            continue;
        };
        let indices: Vec<_> = mesh.indices().expect("indices").iter().collect();
        for indices in indices.chunks_exact(3) {
            let mut triangle = [0; 9];
            for (i, index) in indices.iter().enumerate() {
                for (axis, v) in positions.get(*index).expect("vertex").iter().enumerate() {
                    *triangle.get_mut(i * 3 + axis).expect("triangle position") = v.to_bits();
                }
            }
            // Preserve winding, allowing the same face to start at another corner.
            let mut rotated = triangle;
            rotated.rotate_left(3);
            let mut twice = triangle;
            twice.rotate_left(6);
            triangles.push((triangle.min(rotated).min(twice), batch.material.id.clone()));
        }
    }
    triangles.sort();
    triangles
}
#[test]
fn object_mask_preserves_all_geometry_picking_records_and_protects_mixed_faces() {
    let mut p = fixture();
    let at = WorldHex::new(8, 8);
    let chunk = p.chunks.get_mut(&at.chunk()).expect("chunk");
    for column in &mut chunk.columns {
        if column.position != at {
            column.runs.first_mut().expect("run").top = 0;
        }
    }
    let facts = chunk
        .columns
        .iter()
        .map(|c| (c.position, c.clone()))
        .collect();
    // The original shared-material side extends from neighboring ground0 to
    // object top8, crossing the actual terrain/object boundary4 in one triangle.
    chunk
        .columns
        .iter_mut()
        .find(|c| c.position == at)
        .expect("center")
        .runs
        .first_mut()
        .expect("run")
        .top = 8;
    p.seal().expect("package");
    let chunk = p.chunks.get(&at.chunk()).expect("chunk");
    let map = overview(&p);
    let mask = vec![ColumnData {
        position: at,
        runs: vec![VoxelRun {
            bottom: 4,
            top: 8,
            material: "stone".into(),
        }],
    }];
    let mut presenter = TerrainPresenter::new(
        &p.manifest,
        RenderOrigin {
            column: at,
            level: 0,
        },
        1.0,
    )
    .expect("presenter");
    let mut prepared = presenter.prepare(chunk, 1).expect("original preparation");
    let faces = triangles(&prepared, None);
    let geometry = geometry_snapshot(&prepared);
    let records = picking_records(&prepared);
    let metrics = decorate(&mut prepared, &map, &facts, &BTreeSet::new(), &mask).expect("decorate");
    assert!(
        metrics.eligible_vertices > 0,
        "unambiguous terrain still filters"
    );
    assert_eq!(faces, triangles(&prepared, None));
    assert_eq!(geometry, geometry_snapshot(&prepared));
    assert_eq!(records, picking_records(&prepared));
    assert_eq!(prepared.package.as_ref(), chunk);
    let mut protected = 0;
    for batch in &prepared.batches {
        let mesh = batch.mesh.as_ref().expect("mesh");
        let Some(VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("positions")
        };
        let Some(VertexAttributeValues::Float32x4(colors)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR)
        else {
            continue;
        };
        let indices: Vec<_> = mesh.indices().expect("indices").iter().collect();
        for triangle in indices.chunks_exact(3) {
            if triangle
                .iter()
                .any(|i| Vec3::from_array(*positions.get(*i).expect("position")).y > 4.001)
            {
                protected += 1;
                for i in triangle {
                    let [r, g, b, a] = *colors.get(*i).expect("color");
                    assert_eq!(
                        [r.to_bits(), g.to_bits(), b.to_bits(), a.to_bits()],
                        [
                            1.0_f32.to_bits(),
                            1.0_f32.to_bits(),
                            1.0_f32.to_bits(),
                            0.0_f32.to_bits()
                        ]
                    );
                }
            }
        }
    }
    assert!(protected > 0, "actual object and mixed triangles tested");
    let mut world = World::new();
    world.insert_resource(Enabled);
    world.init_resource::<Assets<FilterMaterial>>();
    presenter.publish(&mut world, prepared).expect("publish");
    let hits: Vec<_> = world
        .query::<&TerrainRenderBatch>()
        .iter(&world)
        .filter_map(|b| b.resolve_hit(Vec3::new(0.0, 8.0, 0.0), Some(Vec3::Y)))
        .collect();
    assert_eq!(hits.len(), 1);
    let run = world
        .get::<super::super::ResidentRun>(*hits.first().expect("hit"))
        .expect("run");
    assert_eq!(
        run.source,
        RunSource::Terrain,
        "original merged picking provenance unchanged"
    );
    assert_eq!((run.position.column, run.bottom, run.top), (at, -4, 8));
}
fn picking_records(prepared: &PreparedChunk) -> Vec<super::super::ResidentRun> {
    prepared
        .batches
        .iter()
        .flat_map(|b| b.runs.iter().map(|r| r.exact.clone()))
        .collect()
}
fn geometry_snapshot(
    prepared: &PreparedChunk,
) -> Vec<(
    Option<VertexAttributeValues>,
    Option<VertexAttributeValues>,
    Vec<usize>,
)> {
    prepared
        .batches
        .iter()
        .filter_map(|b| b.mesh.as_ref())
        .map(|m| {
            (
                m.attribute(Mesh::ATTRIBUTE_POSITION).cloned(),
                m.attribute(Mesh::ATTRIBUTE_NORMAL).cloned(),
                m.indices().expect("indices").iter().collect(),
            )
        })
        .collect()
}
#[test]
fn fragment_decoration_keeps_geometry_and_objects_and_protects_stacks_edits_water_cliffs() {
    let p = fixture();
    let mut map = overview(&p);
    map.materials.push(hex_world_contracts::MaterialSpec {
        id: "water".into(),
        solid: false,
        diggable: false,
        color: [40, 100, 200, 180],
    });
    let at = WorldHex::new(8, 8);
    let chunk = p.chunks.get(&at.chunk()).expect("chunk");
    let facts: BTreeMap<_, _> = chunk
        .columns
        .iter()
        .map(|c| (c.position, c.clone()))
        .collect();
    let presenter = TerrainPresenter::new(
        &p.manifest,
        RenderOrigin {
            column: at,
            level: 0,
        },
        1.0,
    )
    .expect("presenter");
    let mut prepared = presenter.prepare(chunk, 1).expect("prepared");
    let faces = triangles(&prepared, None);
    let normals: Vec<_> = prepared
        .batches
        .iter()
        .filter_map(|b| b.mesh.as_ref())
        .map(|m| m.attribute(Mesh::ATTRIBUTE_NORMAL).cloned())
        .collect();
    let metrics = decorate(&mut prepared, &map, &facts, &BTreeSet::new(), &[]).expect("decorate");
    assert!(metrics.eligible_vertices > 0);
    assert_eq!(metrics.extra_bytes, metrics.vertices * 24);
    assert_eq!(faces, triangles(&prepared, None));
    assert_eq!(
        normals,
        prepared
            .batches
            .iter()
            .filter_map(|b| b.mesh.as_ref())
            .map(|m| m.attribute(Mesh::ATTRIBUTE_NORMAL).cloned())
            .collect::<Vec<_>>()
    );
    assert!(feature(at, &map, &facts, &BTreeSet::from([at])).is_none());
    assert!(feature(at, &map, &BTreeMap::new(), &BTreeSet::new()).is_none());
    for replacement in [
        vec![
            VoxelRun {
                bottom: -4,
                top: 0,
                material: "stone".into(),
            },
            VoxelRun {
                bottom: 2,
                top: 4,
                material: "stone".into(),
            },
        ],
        vec![
            VoxelRun {
                bottom: -4,
                top: 4,
                material: "stone".into(),
            },
            VoxelRun {
                bottom: 4,
                top: 5,
                material: "glass_object".into(),
            },
        ],
        vec![
            VoxelRun {
                bottom: -4,
                top: 3,
                material: "stone".into(),
            },
            VoxelRun {
                bottom: 3,
                top: 4,
                material: "water".into(),
            },
        ],
        // A contiguous cliff with thin top strata remains one large exposed span.
        vec![
            VoxelRun {
                bottom: -4,
                top: 19,
                material: "stone".into(),
            },
            VoxelRun {
                bottom: 19,
                top: 20,
                material: "stone".into(),
            },
        ],
    ] {
        let mut changed = facts.clone();
        changed.get_mut(&at).expect("column").runs = replacement;
        assert!(feature(at, &map, &changed, &BTreeSet::new()).is_none());
    }
    let mut changed = facts.clone();
    changed
        .get_mut(&at)
        .expect("column")
        .runs
        .first_mut()
        .expect("run")
        .top = 7;
    assert_eq!(
        feature(at, &map, &changed, &BTreeSet::new())
            .expect("ordinary steps")
            .1
            .to_bits(),
        3.0_f32.to_bits()
    );
}

#[test]
fn filtered_publication_requires_resources_and_clear_releases_its_assets() {
    let p = fixture();
    let at = WorldHex::new(8, 8);
    let chunk = p.chunks.get(&at.chunk()).expect("chunk");
    let map = overview(&p);
    let facts = chunk
        .columns
        .iter()
        .map(|c| (c.position, c.clone()))
        .collect();
    let mut presenter = TerrainPresenter::new(
        &p.manifest,
        RenderOrigin {
            column: at,
            level: 0,
        },
        1.0,
    )
    .expect("presenter");
    let prepare = |presenter: &TerrainPresenter, revision| {
        let mut prepared = presenter.prepare(chunk, revision).expect("prepare");
        let metrics =
            decorate(&mut prepared, &map, &facts, &BTreeSet::new(), &[]).expect("decorate");
        assert!(metrics.eligible_vertices > 0);
        prepared
    };
    let mut world = World::new();
    assert!(presenter
        .publish(&mut world, prepare(&presenter, 1))
        .is_err());
    assert_eq!(presenter.receipts().count(), 0);
    world.insert_resource(Enabled);
    world.init_resource::<Assets<FilterMaterial>>();
    presenter
        .publish(&mut world, prepare(&presenter, 1))
        .expect("publish");
    assert_eq!(world.resource::<Assets<FilterMaterial>>().len(), 1);
    let mesh_count = world.resource::<Assets<Mesh>>().len();
    presenter
        .publish(&mut world, prepare(&presenter, 2))
        .expect("replace");
    assert_eq!(world.resource::<Assets<FilterMaterial>>().len(), 1);
    assert_eq!(world.resource::<Assets<Mesh>>().len(), mesh_count);
    assert!(presenter
        .publish(&mut world, prepare(&presenter, 1))
        .is_err());
    presenter.remove(&mut world, at.chunk()).expect("remove");
    assert!(world.resource::<Assets<Mesh>>().is_empty());
    presenter.clear(&mut world);
    assert!(world.resource::<Assets<FilterMaterial>>().is_empty());
}

#[test]
fn macro_target_requires_compatible_natural_material_and_nearby_exterior_top() {
    let p = fixture();
    let mut map = overview(&p);
    let target = sample(&map, 0.0, 0.0, Family::Ground).expect("natural ground");
    let face = Feature {
        minimum_top: 4.0,
        maximum_top: 4.0,
        height: 3.0,
    };
    assert!(reliable_target(&target, face, map.spacing));
    assert!(!reliable_target(
        &MacroTarget {
            slope: target.slope,
            color: target.color,
            top: 66.0
        },
        face,
        map.spacing
    ));
    // The top datum is independent of a short exposed side's lower center.
    assert!(reliable_target(
        &MacroTarget {
            slope: target.slope,
            color: target.color,
            top: 11.9
        },
        face,
        map.spacing
    ));
    for id in [
        "timber",
        "foliage",
        "worked_stone",
        "crystal",
        "water",
        "reinforced_stone",
    ] {
        assert!(natural_family(id).is_none());
    }
    let target_index = usize::from(*map.surface_materials.first().expect("target index"));
    for id in ["snow", "sand", "basalt", "worked_stone"] {
        map.materials
            .get_mut(target_index)
            .expect("target material")
            .id = id.into();
        assert!(sample(&map, 0.0, 0.0, Family::Ground).is_none());
    }
    map.materials
        .get_mut(target_index)
        .expect("target material")
        .id = "moss".into();
    assert!(sample(&map, 0.0, 0.0, Family::Ground).is_some());
}

fn attributes(prepared: &PreparedChunk) -> Vec<Vec<(bevy::mesh::MeshVertexAttributeId, Vec<u8>)>> {
    prepared
        .batches
        .iter()
        .filter_map(|b| b.mesh.as_ref())
        .map(|m| {
            m.attributes()
                .map(|(a, v)| (a.id, v.get_bytes().to_vec()))
                .collect()
        })
        .collect()
}

#[test]
fn budget_fallback_restores_exact_attributes_and_publishes_replaces_retires_normally() {
    let p = fixture();
    let at = WorldHex::new(8, 8);
    let chunk = p.chunks.get(&at.chunk()).expect("chunk");
    let map = overview(&p);
    let facts = chunk
        .columns
        .iter()
        .map(|c| (c.position, c.clone()))
        .collect();
    let mut presenter = TerrainPresenter::new(
        &p.manifest,
        RenderOrigin {
            column: at,
            level: 0,
        },
        1.0,
    )
    .expect("presenter");
    let mut world = World::new();
    world.insert_resource(Enabled);
    world.init_resource::<Assets<FilterMaterial>>();
    let mut first = presenter.prepare(chunk, 1).expect("first");
    let first_metrics =
        decorate(&mut first, &map, &facts, &BTreeSet::new(), &[]).expect("decorate");
    let mut first_products = vec![first];
    let mut first_stats = BTreeMap::from([(at.chunk(), first_metrics)]);
    assert!(enforce_budget(&mut first_products, &mut first_stats, 0, 64 * 1024 * 1024).is_none());
    presenter
        .publish(&mut world, first_products.pop().expect("first product"))
        .expect("filtered publication");
    let mesh_count = world.resource::<Assets<Mesh>>().len();
    assert_eq!(world.resource::<Assets<FilterMaterial>>().len(), 1);
    let mut second = presenter.prepare(chunk, 2).expect("replacement");
    for batch in &mut second.batches {
        if let Some(mesh) = &mut batch.mesh {
            // Deliberately nonzero and varying: restoration must not invent UV0.
            let uv: Vec<_> = (0..mesh.count_vertices())
                .map(|i| {
                    [
                        f32::from(u16::try_from(i).expect("fixture count")) * 0.125,
                        -0.75,
                    ]
                })
                .collect();
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
        }
    }
    let original = attributes(&second);
    let geometry = geometry_snapshot(&second);
    let records = picking_records(&second);
    let metrics = decorate(&mut second, &map, &facts, &BTreeSet::new(), &[]).expect("decorate");
    assert!(metrics.eligible_vertices > 0);
    assert_eq!(metrics.original_uv_bytes, metrics.vertices * 8);
    let mut products = vec![second];
    let mut stats = BTreeMap::from([(at.chunk(), metrics)]);
    let retained = 4096;
    assert!(enforce_budget(&mut products, &mut stats, retained, retained).is_some());
    let restored = products.pop().expect("restored product");
    assert_eq!(original, attributes(&restored));
    assert_eq!(geometry, geometry_snapshot(&restored));
    assert_eq!(records, picking_records(&restored));
    assert!(restored
        .batches
        .iter()
        .all(|b| !b.shading && b.shading_uv0.is_none()));
    assert!(stats
        .values()
        .all(|m| m.extra_bytes == 0 && m.original_uv_bytes == 0));
    let receipt = presenter
        .publish(&mut world, restored)
        .expect("budget fallback publishes");
    assert_eq!(receipt.revision, 2);
    assert_eq!(world.resource::<Assets<Mesh>>().len(), mesh_count);
    assert_eq!(
        world
            .query::<&MeshMaterial3d<FilterMaterial>>()
            .iter(&world)
            .count(),
        0
    );
    // Accepting the original revision is idempotent, with no budget retry request.
    let repeated = presenter
        .publish(
            &mut world,
            presenter.prepare(chunk, 2).expect("same revision"),
        )
        .expect("idempotent");
    assert_eq!(receipt.root, repeated.root);
    presenter.remove(&mut world, at.chunk()).expect("retire");
    assert!(world.resource::<Assets<Mesh>>().is_empty());
    presenter.clear(&mut world);
    assert!(world.resource::<Assets<FilterMaterial>>().is_empty());
    assert!(world.resource::<Assets<StandardMaterial>>().is_empty());
}

#[test]
fn default_selection_leaves_headless_and_non_grand_preparation_unchanged() {
    assert!(selected(None));
    assert!(selected(Some("1")));
    assert!(!selected(Some("0")));
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    install(&mut app);
    app.update();
    assert!(!app.world().contains_resource::<Enabled>());
    assert!(!app.world().contains_resource::<Assets<FilterMaterial>>());
    let p = fixture();
    let at = WorldHex::new(8, 8);
    let chunk = p.chunks.get(&at.chunk()).expect("chunk");
    let mut map = overview(&p);
    map.world_id = "northern-test".into();
    let mut presenter = TerrainPresenter::new(
        &p.manifest,
        RenderOrigin {
            column: at,
            level: 0,
        },
        1.0,
    )
    .expect("presenter");
    let mut prepared = presenter.prepare(chunk, 1).expect("prepare");
    let before = attributes(&prepared);
    let metrics =
        decorate(&mut prepared, &map, &BTreeMap::new(), &BTreeSet::new(), &[]).expect("unaffected");
    assert_eq!(metrics.extra_bytes, 0);
    assert_eq!(before, attributes(&prepared));
    presenter
        .publish(app.world_mut(), prepared)
        .expect("ordinary publication needs no shading resources");
    presenter.clear(app.world_mut());
}
