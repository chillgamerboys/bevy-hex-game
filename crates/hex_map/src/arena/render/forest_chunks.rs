//! Bounded terrain mesh publication for the finite V4 battle region.

use super::*;
use crate::grid::{resident_terrain_mesh, ProjectedRun, TerrainMeshRun};
use hex_core::{arena::ArenaRenderStatus, HexSpan};
use hex_world_contracts::{ChunkId, WorldHex};

#[derive(Default)]
pub(super) struct ForestRender {
    generation: Option<u64>,
    chunks: BTreeMap<ChunkId, Vec<(Entity, Handle<Mesh>)>>,
    pending: BTreeSet<ChunkId>,
    materials: BTreeMap<String, Handle<StandardMaterial>>,
}

impl ForestRender {
    pub(super) fn clear(&mut self, commands: &mut Commands, meshes: &mut Assets<Mesh>) {
        for (_, items) in std::mem::take(&mut self.chunks) {
            for (entity, mesh) in items {
                commands.entity(entity).despawn();
                meshes.remove(mesh.id());
            }
        }
        self.pending.clear();
        self.generation = None;
    }
}

pub(super) fn refresh(
    commands: &mut Commands,
    cache: &mut ForestRender,
    state: &mut ArenaWorldState,
    map: &VoxelMap,
    geometry: ArenaVoxelGeometry,
    substances: &SubstanceTable,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    status: &mut ArenaRenderStatus,
) {
    if cache.generation != Some(state.generation) {
        cache.clear(commands, meshes);
        cache.generation = Some(state.generation);
    }
    for coord in std::mem::take(&mut state.render_dirty) {
        for neighbor in coord.within_radius(1) {
            cache
                .pending
                .insert(WorldHex::new(i64::from(neighbor.x()), i64::from(neighbor.y())).chunk());
        }
    }
    let Some(backend) = &state.forest else {
        return;
    };
    let selected: Vec<_> = cache.pending.iter().take(8).copied().collect();
    for chunk in selected {
        let Some(product) = backend.runtime.resident_chunk(chunk) else {
            cache.pending.remove(&chunk);
            continue;
        };
        let mut occluders = BTreeMap::new();
        let mut grouped: BTreeMap<String, Vec<TerrainMeshRun>> = BTreeMap::new();
        for column in &product.package.columns {
            let (Ok(q), Ok(r)) = (
                i32::try_from(column.position.q),
                i32::try_from(column.position.r),
            ) else {
                continue;
            };
            let coord = HexCoord::from_axial(q, r);
            for neighbor in coord.within_radius(1) {
                occluders.entry(neighbor).or_insert_with(|| {
                    map.column(neighbor).map_or_else(Vec::new, |column| {
                        crate::runs(column)
                            .into_iter()
                            .filter(|run| substances.is_solid(run.substance))
                            .map(|run| ProjectedRun { run, cutaway: None })
                            .collect()
                    })
                });
            }
            for run in &column.runs {
                let Some(material) = backend
                    .runtime
                    .manifest()
                    .materials
                    .iter()
                    .find(|material| material.id == run.material)
                else {
                    continue;
                };
                if !material.solid {
                    continue;
                } // animated water has one shared liquid renderer
                grouped
                    .entry(run.material.clone())
                    .or_default()
                    .push(TerrainMeshRun {
                        position: TilePos::new(coord, run.top - 1),
                        bottom: run.bottom,
                        top: run.top,
                        span: HexSpan::new(
                            geometry.top(TilePos::new(coord, run.bottom)) - geometry.level_height,
                            geometry.top(TilePos::new(coord, run.top - 1)),
                        ),
                        cutaway: None,
                    });
            }
        }
        let mut prepared = Vec::new();
        let mut failed = false;
        for (name, runs) in grouped {
            match resident_terrain_mesh(&runs, &occluders, geometry.level_height) {
                Ok(mesh) => prepared.push((name, mesh)),
                Err(error) => {
                    error!("Forest terrain mesh: {error}");
                    failed = true;
                    break;
                }
            }
        }
        if failed {
            continue;
        }
        let mut roots = Vec::new();
        for (name, mesh) in prepared {
            let Some(spec) = backend
                .runtime
                .manifest()
                .materials
                .iter()
                .find(|material| material.id == name)
            else {
                continue;
            };
            let material = cache
                .materials
                .entry(name.clone())
                .or_insert_with(|| {
                    let [r, g, b, _] = spec.color;
                    materials.add(StandardMaterial {
                        base_color: Color::srgb_u8(r, g, b),
                        perceptual_roughness: 0.95,
                        reflectance: 0.08,
                        ..default()
                    })
                })
                .clone();
            let mesh = meshes.add(mesh);
            let entity = commands
                .spawn((
                    Name::new(format!("Forest chunk {chunk:?} {name}")),
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material),
                    Transform::default(),
                ))
                .id();
            roots.push((entity, mesh));
        }
        if let Some(old) = cache.chunks.insert(chunk, roots) {
            for (entity, mesh) in old {
                commands.entity(entity).despawn();
                meshes.remove(mesh.id());
            }
        }
        cache.pending.remove(&chunk);
    }
    status.pending_chunks = cache.pending.len();
}
