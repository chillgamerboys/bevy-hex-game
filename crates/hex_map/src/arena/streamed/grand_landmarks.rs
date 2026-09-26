//! One bounded distant World Tree surface, sourced from its immutable semantic root.
//!
//! This is independent of the ocean bed/terrain overview. Compact column runs
//! preserve the actual grounded trunk and crown without expanding voxel levels.
//! The full semantic chunk is released after building this disposable mesh.
use super::{package_path_for, StreamedArena};
use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
};
use hex_core::{arena::ArenaMap, HexCoord};
use hex_world_contracts::{ChunkId, ObjectInstance, VoxelRun, WorldHex};
use hex_world_runtime::{FileChunkSource, IoLimits};
use std::collections::{BTreeMap, BTreeSet};

const TREE_ASSET: &str = "plant/grand-world-tree";
const MAX_COLUMNS: usize = 20_000;
const MAX_RUNS: usize = 25_000;
// Grand package 13408690208396973052 uses 15,883 columns / 19,963 runs and
// 496,932 indexed vertices (853,182 indices); retain a bounded ~20% margin.
const MAX_VERTICES: usize = 600_000;
const CORNERS: [Vec3; 6] = [
    Vec3::new(0.0, 0.0, 1.0),
    Vec3::new(0.866_025_4, 0.0, 0.5),
    Vec3::new(0.866_025_4, 0.0, -0.5),
    Vec3::new(0.0, 0.0, -1.0),
    Vec3::new(-0.866_025_4, 0.0, -0.5),
    Vec3::new(-0.866_025_4, 0.0, 0.5),
];

#[derive(Resource)]
struct Cache {
    identity: (u64, u64),
    tree: Option<Tree>,
}

struct Tree {
    id: String,
    footprint: BTreeSet<ChunkId>,
    entity: Entity,
    mesh: Handle<Mesh>,
    material: Handle<StandardMaterial>,
    visible: bool,
}

/// Called by the existing streamed renderer after its complete-object publication.
/// The caller passes published objects, not requested or merely resident chunks.
pub(super) fn sync(
    world: &mut World,
    state: &StreamedArena,
    detailed_objects: &BTreeMap<String, BTreeSet<ChunkId>>,
) {
    if state.runtime.manifest().world_id != "grand-v4" {
        clear(world);
        return;
    }
    if !world.contains_resource::<Assets<Mesh>>()
        || !world.contains_resource::<Assets<StandardMaterial>>()
    {
        return;
    }
    let identity = (state.runtime.manifest().fingerprint, state.generation);
    if world
        .get_resource::<Cache>()
        .is_some_and(|cache| cache.identity != identity)
    {
        clear(world);
    }
    if !world.contains_resource::<Cache>() {
        let tree = match load(state) {
            Ok(Some((id, footprint, mesh))) => {
                let vertices = mesh.count_vertices();
                let mesh = world.resource_mut::<Assets<Mesh>>().add(mesh);
                let material =
                    world
                        .resource_mut::<Assets<StandardMaterial>>()
                        .add(StandardMaterial {
                            base_color: Color::WHITE,
                            perceptual_roughness: 0.96,
                            reflectance: 0.05,
                            ..default()
                        });
                let entity = world
                    .spawn((
                        Name::new("Grand distant World Tree"),
                        Mesh3d(mesh.clone()),
                        MeshMaterial3d(material.clone()),
                        Transform::default(),
                        Visibility::Hidden,
                    ))
                    .id();
                info!(
                    vertices,
                    chunks = footprint.len(),
                    "Grand World Tree proxy prepared"
                );
                Some(Tree {
                    id,
                    footprint,
                    entity,
                    mesh,
                    material,
                    visible: false,
                })
            }
            Ok(None) => None, // The deliberately plain foundation has no dressing.
            Err(error) => {
                warn!("Grand World Tree proxy unavailable: {error}");
                None
            }
        };
        // Cache failures as well: never retry filesystem work every render frame.
        world.insert_resource(Cache { identity, tree });
    }
    world.resource_scope(|world, mut cache: Mut<Cache>| {
        let Some(tree) = cache.tree.as_mut() else {
            return;
        };
        let edited = tree.footprint.iter().any(|chunk| {
            state
                .edits
                .revision(*chunk)
                .is_some_and(|revision| revision > 0)
        });
        let visible = should_show(detailed_objects.contains_key(&tree.id), edited);
        if visible != tree.visible {
            if let Some(mut visibility) = world.get_mut::<Visibility>(tree.entity) {
                *visibility = if visible {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                };
            }
            tree.visible = visible;
        }
    });
}

fn should_show(complete_detailed: bool, edited_footprint: bool) -> bool {
    !complete_detailed && !edited_footprint
}

/// Clear alongside the streamed renderer on map exit or package replacement.
pub(super) fn clear(world: &mut World) {
    let Some(cache) = world.remove_resource::<Cache>() else {
        return;
    };
    if let Some(tree) = cache.tree {
        world.despawn(tree.entity);
        if let Some(mut meshes) = world.get_resource_mut::<Assets<Mesh>>() {
            meshes.remove(tree.mesh.id());
        }
        if let Some(mut materials) = world.get_resource_mut::<Assets<StandardMaterial>>() {
            materials.remove(tree.material.id());
        }
    }
}

type Loaded = (String, BTreeSet<ChunkId>, Mesh);

fn load(state: &StreamedArena) -> Result<Option<Loaded>, String> {
    let point = state
        .overview
        .anchors
        .get("world_tree")
        .ok_or("missing World Tree anchor")?;
    let root = HexCoord::from_world(Vec3::from_array(*point));
    let root = WorldHex::new(i64::from(root.x()), i64::from(root.y()));
    let source =
        FileChunkSource::open_workspace(package_path_for(ArenaMap::GrandV4), IoLimits::default())
            .map_err(|error| error.to_string())?;
    if source.manifest().fingerprint != state.runtime.manifest().fingerprint {
        return Err("World Tree source changed since arena initialization".into());
    }
    // A single validated root chunk provides the complete object, even from the bay.
    let package = source
        .load_chunk(root.chunk())
        .map_err(|error| error.to_string())?;
    let Some(object) = package
        .semantics
        .objects
        .iter()
        .find(|object| object.asset == TREE_ASSET && object.origin.column == root)
    else {
        return Ok(None);
    };
    let palette = source
        .manifest()
        .materials
        .iter()
        .map(|material| {
            let [r, g, b, a] = material.color;
            let color = Color::srgba_u8(r, g, b, a).to_linear();
            (
                material.id.clone(),
                [color.red, color.green, color.blue, color.alpha],
            )
        })
        .collect();
    let mesh = surface(object, state.overview.level_height, &palette)?;
    let footprint = object
        .occupancy
        .iter()
        .map(|column| column.position.chunk())
        .chain(
            object
                .grounding
                .iter()
                .flatten()
                .map(|position| position.column.chunk()),
        )
        .chain(std::iter::once(root.chunk()))
        .collect();
    Ok(Some((object.id.clone(), footprint, mesh)))
}

#[derive(Default)]
struct Surface {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    colors: Vec<[f32; 4]>,
    indices: Vec<u32>,
}

impl Surface {
    // Each convex planar face owns its vertices, so indexing shares positions
    // within a cap/quad without smoothing normals across separate hex faces.
    fn polygon(&mut self, vertices: &[Vec3], color: [f32; 4]) -> Result<(), String> {
        let [a, b, c, ..] = vertices else {
            return Err("World Tree face has fewer than three vertices".into());
        };
        if !matches!(vertices.len(), 4 | 6) {
            return Err("World Tree faces must be hex caps or side quads".into());
        }
        if self.positions.len() + vertices.len() > MAX_VERTICES {
            return Err("World Tree surface exceeds its vertex budget".into());
        }
        let base = u32::try_from(self.positions.len()).map_err(|error| error.to_string())?;
        let normal = (*b - *a).cross(*c - *a).normalize_or(Vec3::Y).to_array();
        // At most two indices per admitted vertex: six-vertex caps use four
        // triangles, and four-vertex sides use two. The vertex bound also bounds
        // the index allocation; no individual voxel levels are expanded.
        for offset in 1..vertices.len() - 1 {
            let next = base + u32::try_from(offset).map_err(|error| error.to_string())?;
            self.indices.extend([base, next, next + 1]);
        }
        self.positions
            .extend(vertices.iter().map(|vertex| vertex.to_array()));
        self.normals
            .extend(std::iter::repeat_n(normal, vertices.len()));
        self.colors
            .extend(std::iter::repeat_n(color, vertices.len()));
        Ok(())
    }

    fn finish(self) -> Mesh {
        let uvs = vec![[0.0, 0.0]; self.positions.len()];
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, self.colors)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_indices(Indices::U32(self.indices))
    }
}

// Emit only exposed intervals. Neighbors hide material boundaries too, so the
// trunk/crown joint cannot create coincident internal faces or a floating gap.
fn exposed(bottom: i32, top: i32, neighbors: &[VoxelRun]) -> Vec<(i32, i32)> {
    let mut cursor = bottom;
    let mut out = Vec::new();
    for run in neighbors {
        if run.top <= cursor {
            continue;
        }
        if run.bottom >= top {
            break;
        }
        if cursor < run.bottom {
            out.push((cursor, run.bottom.min(top)));
        }
        cursor = cursor.max(run.top);
        if cursor >= top {
            break;
        }
    }
    if cursor < top {
        out.push((cursor, top));
    }
    out
}

#[expect(
    clippy::cast_precision_loss,
    reason = "Validated finite Grand columns are within radius900 and1600levels."
)]
fn surface(
    object: &ObjectInstance,
    level_height: f32,
    palette: &BTreeMap<String, [f32; 4]>,
) -> Result<Mesh, String> {
    if object.occupancy.len() > MAX_COLUMNS
        || object
            .occupancy
            .iter()
            .map(|column| column.runs.len())
            .sum::<usize>()
            > MAX_RUNS
        || !level_height.is_finite()
        || level_height <= 0.0
    {
        return Err("World Tree occupancy exceeds its presentation budget".into());
    }
    let columns: BTreeMap<_, _> = object
        .occupancy
        .iter()
        .map(|column| (column.position, column.runs.as_slice()))
        .collect();
    let mut out = Surface::default();
    for column in &object.occupancy {
        let coord = super::local(column.position).ok_or("World Tree column outside local range")?;
        for run in &column.runs {
            let color = *palette
                .get(&run.material)
                .ok_or("World Tree material missing")?;
            let bottom = run.bottom as f32 * level_height;
            let top = run.top as f32 * level_height;
            let center = coord.to_world(0.0);
            let cap_top = !column
                .runs
                .iter()
                .any(|neighbor| neighbor.bottom == run.top);
            let cap_bottom = !column
                .runs
                .iter()
                .any(|neighbor| neighbor.top == run.bottom);
            if cap_top {
                out.polygon(&CORNERS.map(|corner| (center + corner).with_y(top)), color)?;
            }
            if cap_bottom {
                let mut vertices = CORNERS.map(|corner| (center + corner).with_y(bottom));
                vertices.reverse();
                out.polygon(&vertices, color)?;
            }
            for (first, second) in CORNERS.iter().zip(CORNERS.iter().cycle().skip(1)) {
                let a = center + *first;
                let b = center + *second;
                let neighbor = HexCoord::from_world(center + *first + *second);
                let neighbor = WorldHex::new(i64::from(neighbor.x()), i64::from(neighbor.y()));
                let neighbors = columns.get(&neighbor).copied().unwrap_or(&[]);
                for (lo, hi) in exposed(run.bottom, run.top, neighbors) {
                    let (lo, hi) = (lo as f32 * level_height, hi as f32 * level_height);
                    out.polygon(
                        &[a.with_y(lo), b.with_y(lo), b.with_y(hi), a.with_y(hi)],
                        color,
                    )?;
                }
            }
        }
    }
    Ok(out.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detailed_publication_or_any_footprint_edit_suppresses_proxy() {
        assert!(should_show(false, false));
        assert!(!should_show(true, false));
        assert!(!should_show(false, true));
        assert!(!should_show(true, true));
    }

    #[test]
    fn neighboring_runs_remove_only_their_occupied_side_intervals() {
        let runs = [(2, 4), (6, 12)].map(|(bottom, top)| VoxelRun {
            bottom,
            top,
            material: "timber".into(),
        });
        assert_eq!(exposed(0, 10, &runs), vec![(0, 2), (4, 6)]);
        assert!(exposed(7, 10, &runs).is_empty());
        assert_eq!(exposed(14, 18, &runs), vec![(14, 18)]);
    }

    #[test]
    fn compact_tree_surface_keeps_height_and_removes_shared_sides() {
        use hex_world_contracts::{ColumnData, VoxelPosition};
        let object = ObjectInstance {
            id: "grand/world-tree".into(),
            region_id: "grand".into(),
            asset: TREE_ASSET.into(),
            origin: VoxelPosition {
                column: WorldHex::new(0, 0),
                level: 17,
            },
            rotation: 0,
            occupancy: [WorldHex::new(0, 0), WorldHex::new(1, 0)]
                .map(|position| ColumnData {
                    position,
                    runs: vec![VoxelRun {
                        bottom: 17,
                        top: 337,
                        material: "timber".into(),
                    }],
                })
                .to_vec(),
            grounding: None,
        };
        let palette = BTreeMap::from([("timber".into(), [0.2, 0.1, 0.0, 1.0])]);
        let mesh = surface(&object, 0.35, &palette).expect("bounded tree");
        // 320 levels still cost two caps and five exposed sides per column.
        assert_eq!(mesh.count_vertices(), 64);
        assert_eq!(mesh.indices().expect("indexed faces").len(), 108);
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("positions")
        };
        let bottom = positions
            .iter()
            .map(|p| Vec3::from(*p).y)
            .fold(f32::INFINITY, f32::min);
        let top = positions
            .iter()
            .map(|p| Vec3::from(*p).y)
            .fold(f32::NEG_INFINITY, f32::max);
        assert!((bottom - 17.0 * 0.35).abs() < 0.00001);
        assert!((top - 337.0 * 0.35).abs() < 0.00001);

        let Some(bevy::mesh::VertexAttributeValues::Float32x3(normals)) =
            mesh.attribute(Mesh::ATTRIBUTE_NORMAL)
        else {
            panic!("normals")
        };
        let Some(bevy::mesh::VertexAttributeValues::Float32x4(colors)) =
            mesh.attribute(Mesh::ATTRIBUTE_COLOR)
        else {
            panic!("colors")
        };
        assert_eq!(normals.len(), positions.len());
        assert_eq!(colors.len(), positions.len());
        assert!(colors
            .iter()
            .all(|color| color.map(f32::to_bits) == [0.2_f32, 0.1, 0.0, 1.0].map(f32::to_bits)));
        let indices: Vec<_> = mesh.indices().expect("indices").iter().collect();
        let mut area = 0.0;
        for triangle in indices.chunks_exact(3) {
            let [a, b, c] = triangle else {
                panic!("triangle list")
            };
            let a_pos = Vec3::from(*positions.get(*a).expect("first vertex"));
            let b_pos = Vec3::from(*positions.get(*b).expect("second vertex"));
            let c_pos = Vec3::from(*positions.get(*c).expect("third vertex"));
            let normal = Vec3::from(*normals.get(*a).expect("face normal"));
            let cross = (b_pos - a_pos).cross(c_pos - a_pos);
            assert!(
                cross.dot(normal) > 0.0,
                "winding agrees with the outward face normal"
            );
            assert!((normal.length() - 1.0).abs() < 0.00001);
            assert!(normal.abs_diff_eq(
                Vec3::from(*normals.get(*b).expect("second normal")),
                0.00001
            ));
            assert!(
                normal.abs_diff_eq(Vec3::from(*normals.get(*c).expect("third normal")), 0.00001)
            );
            area += cross.length() * 0.5;
        }
        // Four regular unit-hex caps plus ten side rectangles of width one.
        let expected_area = 4.0 * (3.0 * 3.0_f32.sqrt() * 0.5) + 10.0 * (top - bottom);
        assert!(
            (area - expected_area).abs() < 0.001,
            "indexed triangulation preserves exact exposed area: {area} vs {expected_area}"
        );
    }

    #[test]
    fn indexed_face_budget_rejects_before_mutating_any_mesh_buffer() {
        let mut out = Surface {
            positions: vec![[0.0; 3]; MAX_VERTICES - 4],
            ..default()
        };
        assert!(out.polygon(&CORNERS, [1.0; 4]).is_err());
        assert_eq!(out.positions.len(), MAX_VERTICES - 4);
        assert!(out.normals.is_empty());
        assert!(out.colors.is_empty());
        assert!(out.indices.is_empty());
    }

    #[test]
    fn indexed_hex_cap_has_six_vertices_and_four_triangles() {
        let mut out = Surface::default();
        out.polygon(&CORNERS, [1.0; 4]).expect("hex cap");
        assert_eq!(out.positions.len(), 6);
        assert_eq!(out.indices.len(), 12);
        let [a, b, ..] = CORNERS;
        out.polygon(&[a, b, b + Vec3::Y, a + Vec3::Y], [1.0; 4])
            .expect("side quad");
        assert_eq!(out.positions.len(), 10);
        assert_eq!(out.indices.len(), 18);
    }

    #[test]
    #[ignore = "requires HEX_GRAND_WORLD pointing to the actual full Grand package"]
    fn actual_grand_world_tree_fits_indexed_surface_budget() {
        let directory = std::path::PathBuf::from(
            std::env::var_os("HEX_GRAND_WORLD").expect("explicit actual Grand package"),
        );
        let overview: hex_schematic::v4::northern::NorthernOverview = ron::from_str(
            &std::fs::read_to_string(directory.join("grand-overview.ron")).expect("overview"),
        )
        .expect("overview format");
        let source = FileChunkSource::open_workspace(&directory, IoLimits::default())
            .expect("actual package");
        assert_eq!(source.manifest().fingerprint, overview.package_fingerprint);
        let root = source
            .manifest()
            .features
            .iter()
            .find(|feature| feature.asset.as_deref() == Some(TREE_ASSET))
            .expect("World Tree feature")
            .anchor
            .column;
        let package = source.load_chunk(root.chunk()).expect("root chunk");
        let tree = package
            .semantics
            .objects
            .iter()
            .find(|object| object.asset == TREE_ASSET && object.origin.column == root)
            .expect("complete World Tree object");
        let palette = source
            .manifest()
            .materials
            .iter()
            .map(|material| {
                let [r, g, b, a] = material.color;
                let linear = Color::srgba_u8(r, g, b, a).to_linear();
                (
                    material.id.clone(),
                    [linear.red, linear.green, linear.blue, linear.alpha],
                )
            })
            .collect();
        let mesh =
            surface(tree, overview.level_height, &palette).expect("bounded actual tree mesh");
        let vertices = mesh.count_vertices();
        let indices = mesh.indices().expect("indexed tree").len();
        assert!(vertices > 0 && vertices <= MAX_VERTICES);
        assert!(indices <= 2 * vertices);
        assert!(mesh
            .indices()
            .expect("indices")
            .iter()
            .all(|index| index < vertices));
        if overview.package_fingerprint == 13_408_690_208_396_973_052 {
            // Independent immutable-column face count, not a receipt produced by
            // this mesher. A changed shape uses its own bounded fresh review.
            assert_eq!(vertices, 496_932);
            assert_eq!(indices, 853_182);
        }
        println!(
            "GRAND_TREE_PROXY package={} columns={} runs={} vertices={vertices} indices={indices}",
            overview.package_fingerprint,
            tree.occupancy.len(),
            tree.occupancy
                .iter()
                .map(|column| column.runs.len())
                .sum::<usize>(),
        );
    }
}
