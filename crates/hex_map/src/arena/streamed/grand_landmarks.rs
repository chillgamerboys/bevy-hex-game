//! One bounded distant World Tree surface, sourced from its immutable semantic root.
//!
//! This is independent of the ocean bed/terrain overview. Compact column runs
//! preserve the actual grounded trunk and crown without expanding voxel levels.
//! Only the bounded authored tree columns are retained after releasing the root
//! chunk. Persistent object removals mask those columns even after source eviction.
use super::{package_path_for, StreamedArena};
use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
};
use hex_core::{arena::ArenaMap, HexCoord};
use hex_world_contracts::{ChunkId, ColumnData, ObjectInstance, VoxelRun, WorldHex};
use hex_world_runtime::{FileChunkSource, FiniteWorldSession, IoLimits};
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
    geometry: TreeGeometry,
    current_surface: bool,
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
            Ok(Some((id, geometry, mesh))) => {
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
                    chunks = geometry.observed_revisions.len(),
                    "Grand World Tree proxy prepared"
                );
                Some(Tree {
                    id,
                    geometry,
                    current_surface: vertices > 0,
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
        tree.sync(world, &state.edits, detailed_objects.contains_key(&tree.id));
    });
}

impl Tree {
    fn sync(&mut self, world: &mut World, edits: &FiniteWorldSession, complete_detailed: bool) {
        match self.geometry.refresh(edits) {
            Ok(Some(mesh)) => {
                self.current_surface = mesh.count_vertices() > 0;
                if let Some(mut existing) = world.resource_mut::<Assets<Mesh>>().get_mut(&self.mesh)
                {
                    *existing = mesh;
                } else {
                    self.current_surface = false;
                }
            }
            Ok(None) => {} // Ground edits and residency changes keep the same mesh.
            Err(error) => {
                // Never display stale intact geometry if its current mask cannot
                // fit the bounded proxy; detailed publication remains authoritative.
                self.current_surface = false;
                warn!("Grand World Tree masked proxy unavailable: {error}");
            }
        }
        let visible = should_show(complete_detailed, self.current_surface);
        if visible != self.visible {
            if let Some(mut visibility) = world.get_mut::<Visibility>(self.entity) {
                *visibility = if visible {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                };
            }
            self.visible = visible;
        }
    }
}

fn should_show(complete_detailed: bool, current_surface: bool) -> bool {
    !complete_detailed && current_surface
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

type Loaded = (String, TreeGeometry, Mesh);

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
    let (geometry, mesh) =
        TreeGeometry::new(object, state.overview.level_height, palette, &state.edits)?;
    Ok(Some((object.id.clone(), geometry, mesh)))
}

/// Compact authored occupancy is independent of both terrain residency and edits.
/// The keys retain the original full footprint, including removed columns/supports.
struct TreeGeometry {
    authored: Vec<ColumnData>,
    masked: Vec<ColumnData>,
    observed_revisions: BTreeMap<ChunkId, u64>,
    level_height: f32,
    palette: BTreeMap<String, [f32; 4]>,
}

impl TreeGeometry {
    fn new(
        object: &ObjectInstance,
        level_height: f32,
        palette: BTreeMap<String, [f32; 4]>,
        edits: &FiniteWorldSession,
    ) -> Result<(Self, Mesh), String> {
        check_occupancy_budget(&object.occupancy)?;
        let observed_revisions = object
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
            .chain(std::iter::once(object.origin.column.chunk()))
            .map(|chunk| (chunk, edits.revision(chunk).unwrap_or(0)))
            .collect();
        // Apply restored removals before the first mesh can become visible.
        let masked = masked_columns(&object.occupancy, edits)?;
        let mesh = surface(&masked, level_height, &palette)?;
        Ok((
            Self {
                authored: object.occupancy.clone(),
                masked,
                observed_revisions,
                level_height,
                palette,
            },
            mesh,
        ))
    }

    fn refresh(&mut self, edits: &FiniteWorldSession) -> Result<Option<Mesh>, String> {
        let mut changed = false;
        for (chunk, observed) in &mut self.observed_revisions {
            let current = edits.revision(*chunk).unwrap_or(0);
            changed |= current != *observed;
            *observed = current;
        }
        if !changed {
            return Ok(None);
        }
        // Revision changes are only a cheap dirty hint. Terrain damage, refills,
        // or edits to another object in these chunks cannot reshape this tree.
        let masked = masked_columns(&self.authored, edits)?;
        if masked == self.masked {
            return Ok(None);
        }
        self.masked = masked;
        surface(&self.masked, self.level_height, &self.palette).map(Some)
    }
}

fn check_occupancy_budget(occupancy: &[ColumnData]) -> Result<(), String> {
    if occupancy.len() > MAX_COLUMNS
        || occupancy
            .iter()
            .map(|column| column.runs.len())
            .sum::<usize>()
            > MAX_RUNS
    {
        return Err("World Tree occupancy exceeds its presentation budget".into());
    }
    Ok(())
}

/// Subtract sparse persistent removals from the authored intervals. No generated
/// chunk is loaded, and neither intact voxels nor a per-voxel mask is expanded.
fn masked_columns(
    authored: &[ColumnData],
    edits: &FiniteWorldSession,
) -> Result<Vec<ColumnData>, String> {
    let mut columns = Vec::with_capacity(authored.len());
    let mut run_count = 0;
    for column in authored {
        let mut runs = Vec::new();
        for run in &column.runs {
            let mut bottom = run.bottom;
            for removed in edits
                .removed_in_column(column.position)
                .filter(|removed| removed.level >= run.bottom && removed.level < run.top)
            {
                append_fragment(&mut runs, &mut run_count, run, bottom, removed.level)?;
                bottom = removed.level + 1; // Strictly below the exclusive i32 run top.
            }
            append_fragment(&mut runs, &mut run_count, run, bottom, run.top)?;
        }
        columns.push(ColumnData {
            position: column.position,
            runs,
        });
    }
    Ok(columns)
}

fn append_fragment(
    runs: &mut Vec<VoxelRun>,
    total: &mut usize,
    source: &VoxelRun,
    bottom: i32,
    top: i32,
) -> Result<(), String> {
    if bottom < top {
        if *total >= MAX_RUNS {
            return Err("World Tree removal mask exceeds its run budget".into());
        }
        runs.push(VoxelRun {
            bottom,
            top,
            material: source.material.clone(),
        });
        *total += 1;
    }
    Ok(())
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
    occupancy: &[ColumnData],
    level_height: f32,
    palette: &BTreeMap<String, [f32; 4]>,
) -> Result<Mesh, String> {
    check_occupancy_budget(occupancy)?;
    if !level_height.is_finite() || level_height <= 0.0 {
        return Err("World Tree level height must be finite and positive".into());
    }
    let columns: BTreeMap<_, _> = occupancy
        .iter()
        .map(|column| (column.position, column.runs.as_slice()))
        .collect();
    let mut out = Surface::default();
    for column in occupancy {
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

    use hex_world_contracts::{
        ChunkDescriptor, ChunkPackage, ChunkSemantics, MaterialSpec, RegionDescriptor,
        ResidencyRequest, VoxelEdit, VoxelPosition, WorldEditTransaction, WorldManifest,
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
        object: ObjectInstance,
        palette: BTreeMap<String, [f32; 4]>,
    }

    fn fixture() -> Result<Fixture, Box<dyn std::error::Error>> {
        let root = WorldHex::new(15, 0);
        let crown = WorldHex::new(16, 0); // A second source chunk.
        let object = ObjectInstance {
            id: "grand/world-tree".into(),
            region_id: "tree".into(),
            asset: TREE_ASSET.into(),
            origin: VoxelPosition {
                column: root,
                level: 10,
            },
            rotation: 0,
            grounding: None,
            occupancy: vec![
                ColumnData {
                    position: root,
                    runs: vec![
                        VoxelRun {
                            bottom: 10,
                            top: 22,
                            material: "timber".into(),
                        },
                        VoxelRun {
                            bottom: 22,
                            top: 28,
                            material: "foliage".into(),
                        },
                    ],
                },
                ColumnData {
                    position: crown,
                    runs: vec![VoxelRun {
                        bottom: 22,
                        top: 28,
                        material: "foliage".into(),
                    }],
                },
            ],
        };
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
        let mut package = WorldPackage {
            manifest: WorldManifest {
                schema_version: SCHEMA_VERSION,
                world_id: "grand-v4".into(),
                compiler_version: "test-1".into(),
                source_fingerprint: 7,
                materials: ["stone", "timber", "foliage"]
                    .into_iter()
                    .map(|id| MaterialSpec {
                        id: id.into(),
                        solid: true,
                        diggable: true,
                        color: [80, 100, 60, 255],
                    })
                    .collect(),
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
        let source = Arc::new(MemoryChunkSource::new(package)?);
        let mut runtime = WorldRuntime::new(source, RuntimeConfig::default())?;
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
                return Err(format!("fixture source failures: {:?}", update.failures).into());
            }
            if Instant::now() >= deadline {
                return Err("bounded fixture admission timed out".into());
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let edits = FiniteWorldSession::streamed(&runtime, 0, 50)?;
        Ok(Fixture {
            runtime,
            edits,
            object,
            palette: BTreeMap::from([
                ("timber".into(), [0.2, 0.1, 0.0, 1.0]),
                ("foliage".into(), [0.1, 0.3, 0.0, 1.0]),
            ]),
        })
    }

    fn remove_cells(
        edits: &mut FiniteWorldSession,
        id: &str,
        cells: &[VoxelPosition],
    ) -> RuntimeResult<()> {
        edits.apply_transaction(&WorldEditTransaction {
            id: id.into(),
            expected_revisions: cells
                .iter()
                .map(|position| {
                    let chunk = position.column.chunk();
                    (chunk, edits.revision(chunk).unwrap_or(0))
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

    fn tree_in_world(geometry: TreeGeometry, mesh: Mesh) -> (World, Tree) {
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        let current_surface = mesh.count_vertices() > 0;
        let mesh = world.resource_mut::<Assets<Mesh>>().add(mesh);
        let entity = world.spawn((Mesh3d(mesh.clone()), Visibility::Hidden)).id();
        (
            world,
            Tree {
                id: "grand/world-tree".into(),
                geometry,
                current_surface,
                entity,
                mesh,
                material: Handle::default(),
                visible: false,
            },
        )
    }

    fn mesh_bits(mesh: &Mesh) -> Option<(Vec<[u32; 3]>, Vec<[u32; 3]>, Vec<[u32; 4]>, Vec<usize>)> {
        use bevy::mesh::VertexAttributeValues::{Float32x3, Float32x4};
        let Some(Float32x3(positions)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else {
            return None;
        };
        let Some(Float32x3(normals)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL) else {
            return None;
        };
        let Some(Float32x4(colors)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR) else {
            return None;
        };
        Some((
            positions.iter().map(|p| p.map(f32::to_bits)).collect(),
            normals.iter().map(|p| p.map(f32::to_bits)).collect(),
            colors.iter().map(|p| p.map(f32::to_bits)).collect(),
            mesh.indices()?.iter().collect(),
        ))
    }

    #[test]
    fn terrain_only_edits_keep_the_same_visible_tree_mesh() {
        let mut f = fixture().expect("validated two-chunk tree");
        let (geometry, mesh) =
            TreeGeometry::new(&f.object, 0.35, f.palette, &f.edits).expect("proxy");
        let original = mesh_bits(&mesh).expect("indexed opaque surface");
        let (mut world, mut tree) = tree_in_world(geometry, mesh);
        tree.sync(&mut world, &f.edits, false);
        assert!(tree.visible);
        let handle = tree.mesh.clone();
        remove_cells(
            &mut f.edits,
            "ground-damage",
            &[VoxelPosition {
                column: f.object.origin.column,
                level: 9,
            }],
        )
        .expect("finite terrain removal");
        assert_eq!(f.edits.revision(f.object.origin.column.chunk()), Some(1));
        assert!(
            tree.geometry
                .refresh(&f.edits)
                .expect("ground refresh")
                .is_none(),
            "an actual terrain revision must not request a tree mesh replacement"
        );
        tree.sync(&mut world, &f.edits, false);
        assert!(tree.visible);
        assert_eq!(tree.mesh, handle);
        assert_eq!(
            *world.get::<Visibility>(tree.entity).expect("visibility"),
            Visibility::Inherited
        );
        let current = world
            .resource::<Assets<Mesh>>()
            .get(&tree.mesh)
            .expect("tree mesh");
        assert_eq!(mesh_bits(current).expect("surface"), original);
    }

    #[test]
    fn trunk_and_crown_removals_survive_eviction_resume_and_full_detail_handoff() {
        let mut f = fixture().expect("validated two-chunk tree");
        let (geometry, mesh) =
            TreeGeometry::new(&f.object, 0.35, f.palette.clone(), &f.edits).expect("proxy");
        let before = mesh_bits(&mesh).expect("surface");
        let (mut world, mut tree) = tree_in_world(geometry, mesh);
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
        remove_cells(&mut f.edits, "trunk-and-crown", &cuts).expect("object removal");
        tree.sync(&mut world, &f.edits, false);
        assert!(
            tree.visible,
            "partial detailed admission retains the whole masked proxy"
        );
        let after = mesh_bits(
            world
                .resource::<Assets<Mesh>>()
                .get(&tree.mesh)
                .expect("masked mesh"),
        )
        .expect("surface");
        assert_ne!(after, before, "actual tree removals reshape its surface");
        let remaining = tree.geometry.masked.clone();
        for cut in cuts {
            assert!(f.edits.object_removed(cut));
            assert!(remaining
                .iter()
                .find(|c| c.position == cut.column)
                .expect("column")
                .material_at(cut.level)
                .is_none());
            assert_eq!(
                remaining
                    .iter()
                    .find(|c| c.position == cut.column)
                    .expect("column")
                    .material_at(cut.level + 1),
                f.object
                    .occupancy
                    .iter()
                    .find(|c| c.position == cut.column)
                    .expect("original column")
                    .material_at(cut.level + 1),
                "an adjacent surviving cell retains its authored material"
            );
        }
        assert_eq!(
            tree.geometry.observed_revisions.len(),
            2,
            "original whole footprint retained"
        );
        tree.sync(&mut world, &f.edits, true);
        assert!(
            !tree.visible,
            "only complete detailed publication hides the proxy"
        );
        tree.sync(&mut world, &f.edits, false);
        assert!(
            tree.visible,
            "retreat to partial detail restores the already-masked proxy"
        );
        assert_eq!(
            mesh_bits(
                world
                    .resource::<Assets<Mesh>>()
                    .get(&tree.mesh)
                    .expect("same mesh")
            )
            .expect("surface"),
            after
        );

        f.runtime.set_interests(vec![]).expect("evict all");
        f.runtime.pump();
        f.edits.sync_residency(&f.runtime);
        assert_eq!(f.edits.resident_source_count(), 0);
        assert_eq!(f.runtime.resident_chunks().count(), 0);
        assert!(tree
            .geometry
            .refresh(&f.edits)
            .expect("unloaded refresh")
            .is_none());
        assert_eq!(
            masked_columns(&f.object.occupancy, &f.edits).expect("unloaded mask"),
            remaining
        );
        let header = f.edits.checkpoint_header();
        let partitions = f
            .edits
            .checkpoint_partitions()
            .collect::<RuntimeResult<Vec<_>>>()
            .expect("save unloaded edits");
        let encoded = ron::to_string(&(header, partitions)).expect("persistent codec");
        let (header, partitions): (FiniteSessionHeader, Vec<FiniteChunkCheckpoint>) =
            ron::from_str(&encoded).expect("resumed codec");
        let resumed = FiniteWorldSession::restore_checkpoint(
            &f.runtime,
            0,
            50,
            &header,
            partitions.into_iter().map(Ok),
            &CancellationToken::default(),
        )
        .expect("restore");
        assert_eq!(
            resumed.resident_source_count(),
            0,
            "proxy mask does not pin source chunks"
        );
        let (restored, mesh) =
            TreeGeometry::new(&f.object, 0.35, f.palette, &resumed).expect("first resumed proxy");
        assert_eq!(restored.masked, remaining);
        assert_eq!(
            restored.observed_revisions,
            tree.geometry.observed_revisions
        );
        assert_eq!(
            mesh_bits(&mesh).expect("resumed surface"),
            after,
            "the first mesh after resume already contains all trunk and crown cuts"
        );
    }

    #[test]
    fn removal_fragment_budget_fails_before_adding_an_unbounded_run() {
        let source = VoxelRun {
            bottom: 0,
            top: 100,
            material: "timber".into(),
        };
        let mut runs = vec![];
        let mut count = MAX_RUNS;
        assert!(append_fragment(&mut runs, &mut count, &source, 1, 2).is_err());
        assert!(runs.is_empty());
        assert_eq!(count, MAX_RUNS);
        append_fragment(&mut runs, &mut count, &source, 2, 2)
            .expect("empty interval costs nothing");
    }

    #[test]
    fn only_complete_detail_or_invalid_current_surface_suppresses_proxy() {
        assert!(should_show(false, true));
        assert!(!should_show(true, true));
        assert!(!should_show(false, false));
        assert!(!should_show(true, false));
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
        let mesh = surface(&object.occupancy, 0.35, &palette).expect("bounded tree");
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
        let mesh = surface(&tree.occupancy, overview.level_height, &palette)
            .expect("bounded actual tree mesh");
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
