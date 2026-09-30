//! Nearby, batched decorative plants. No occupancy publication or source requests.
use super::StreamedArena;
use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
};
use hex_schematic::v4::northern::ground_cover::{
    GroundCover, GroundCoverChunk, GroundTuft, MAX_CHUNK_TUFTS,
};
use hex_world_contracts::{ChunkId, VoxelPosition, WorldHex};
use hex_world_runtime::FiniteWorldSession;
use std::collections::{BTreeMap, BTreeSet};

const MAX_CHUNKS: usize = 64;
const STEMS: usize = 5;
const VERTICES_PER_TUFT: usize = STEMS * 24;
const MAX_VERTICES: usize = MAX_CHUNKS * MAX_CHUNK_TUFTS * VERTICES_PER_TUFT;

#[derive(Resource)]
struct Cache {
    identity: (u64, u64),
    material: Handle<StandardMaterial>,
    batches: BTreeMap<ChunkId, Batch>,
}
struct Batch {
    revision: u64,
    entity: Option<Entity>,
    mesh: Option<Handle<Mesh>>,
    vertices: usize,
}

pub(super) fn sync(
    world: &mut World,
    state: &StreamedArena,
    accepted: &BTreeMap<ChunkId, u64>,
    detailed: &BTreeSet<ChunkId>,
    center: WorldHex,
) {
    if state.runtime.manifest().world_id != "grand-v4" || state.overview.ground_cover.is_none() {
        clear(world);
        return;
    }
    if !world.contains_resource::<Assets<Mesh>>()
        || !world.contains_resource::<Assets<StandardMaterial>>()
    {
        return;
    }
    let Some(source) = state.overview.ground_cover.as_ref() else {
        return;
    };
    let identity = (state.runtime.manifest().fingerprint, state.generation);
    if world
        .get_resource::<Cache>()
        .is_some_and(|c| c.identity != identity)
    {
        clear(world);
    }
    if !world.contains_resource::<Cache>() {
        let material = world
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                perceptual_roughness: 1.0,
                ..default()
            });
        world.insert_resource(Cache {
            identity,
            material,
            batches: BTreeMap::new(),
        });
    }
    let resident = state
        .runtime
        .resident_chunks()
        .map(|c| c.coordinate)
        .collect();
    world.resource_scope(|world, mut cache: Mut<Cache>| {
        let published = accepted
            .iter()
            .filter(|(c, _)| detailed.contains(c))
            .map(|(c, r)| (*c, *r))
            .collect();
        cache.update(world, source, &state.edits, &published, &resident, center);
    });
}

pub(super) fn clear(world: &mut World) {
    let Some(cache) = world.remove_resource::<Cache>() else {
        return;
    };
    for batch in cache.batches.into_values() {
        retire(world, batch);
    }
    if let Some(mut materials) = world.get_resource_mut::<Assets<StandardMaterial>>() {
        materials.remove(cache.material.id());
    }
}
fn retire(world: &mut World, batch: Batch) {
    if let Some(entity) = batch.entity {
        world.despawn(entity);
    }
    if let (Some(handle), Some(mut meshes)) = (batch.mesh, world.get_resource_mut::<Assets<Mesh>>())
    {
        meshes.remove(handle.id());
    }
}

impl Cache {
    // The mutable references end at rendering assets/entities. Authority and
    // residency are borrowed immutably, so decoration cannot create a source pin.
    fn update(
        &mut self,
        world: &mut World,
        source: &GroundCover,
        edits: &FiniteWorldSession,
        accepted: &BTreeMap<ChunkId, u64>,
        resident: &BTreeSet<ChunkId>,
        center: WorldHex,
    ) {
        let mut candidates: Vec<_> = source
            .chunks
            .iter()
            .filter(|chunk| {
                resident.contains(&chunk.coordinate)
                    && accepted
                        .get(&chunk.coordinate)
                        .is_some_and(|revision| edits.revision(chunk.coordinate) == Some(*revision))
                    && chunk_center(chunk.coordinate)
                        .checked_distance(center)
                        .is_ok_and(|d| d <= 48)
            })
            .collect();
        candidates.sort_by_key(|c| {
            (
                chunk_center(c.coordinate)
                    .checked_distance(center)
                    .unwrap_or(u64::MAX),
                c.coordinate,
            )
        });
        candidates.truncate(MAX_CHUNKS);
        let wanted: BTreeSet<_> = candidates.iter().map(|c| c.coordinate).collect();
        let retired: Vec<_> = self
            .batches
            .keys()
            .filter(|c| !wanted.contains(c))
            .copied()
            .collect();
        for chunk in retired {
            if let Some(batch) = self.batches.remove(&chunk) {
                retire(world, batch);
            }
        }
        for chunk in candidates {
            let Some(revision) = edits.revision(chunk.coordinate) else {
                continue;
            };
            if self
                .batches
                .get(&chunk.coordinate)
                .is_some_and(|b| b.revision == revision)
            {
                continue;
            }
            if let Some(old) = self.batches.remove(&chunk.coordinate) {
                retire(world, old);
            }
            let mesh = chunk_mesh(chunk, edits);
            let vertices = mesh.count_vertices();
            if vertices == 0 {
                self.batches.insert(
                    chunk.coordinate,
                    Batch {
                        revision,
                        entity: None,
                        mesh: None,
                        vertices: 0,
                    },
                );
                continue;
            }
            if vertices + self.batches.values().map(|b| b.vertices).sum::<usize>() > MAX_VERTICES {
                error!("Grand ground cover exceeded its fixed vertex budget");
                continue;
            }
            let handle = world.resource_mut::<Assets<Mesh>>().add(mesh);
            let entity = world
                .spawn((
                    Name::new("Grand ground cover"),
                    Mesh3d(handle.clone()),
                    MeshMaterial3d(self.material.clone()),
                    Transform::default(),
                    Visibility::Inherited,
                ))
                .id();
            self.batches.insert(
                chunk.coordinate,
                Batch {
                    revision,
                    entity: Some(entity),
                    mesh: Some(handle),
                    vertices,
                },
            );
        }
    }
}
fn chunk_center(chunk: ChunkId) -> WorldHex {
    WorldHex::new(chunk.q * 16 + 8, chunk.r * 16 + 8)
}

fn supported(tuft: &GroundTuft, edits: &FiniteWorldSession) -> bool {
    !edits.terrain_edited(tuft.support)
        && edits.terrain_at(tuft.support) == Some(tuft.material.as_str())
        && (1..=2).all(|offset| {
            edits
                .material_at(VoxelPosition {
                    column: tuft.support.column,
                    level: tuft.support.level + offset,
                })
                .is_none()
        })
}

#[derive(Default)]
struct Geometry {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    colors: Vec<[f32; 4]>,
    indices: Vec<u32>,
}
#[expect(
    clippy::cast_precision_loss,
    reason = "Explicitly admitted finite ground-cover coordinates fit the f32 render envelope."
)]
fn chunk_mesh(chunk: &GroundCoverChunk, edits: &FiniteWorldSession) -> Mesh {
    let mut mesh = Geometry::default();
    for tuft in &chunk.tufts {
        if !supported(tuft, edits) {
            continue;
        }
        let p = tuft.support.column;
        let x = (p.q as f32 + p.r as f32 * 0.5) * 3_f32.sqrt();
        let z = p.r as f32 * 1.5;
        let y = (tuft.support.level + 1) as f32 * 0.35;
        let angle = f32::from(tuft.variant) * std::f32::consts::TAU / 6.0;
        let (sin, cos) = angle.sin_cos();
        // Five chunky blades remain inside one hex. Their unequal heights and
        // offsets read as little voxel plants, not another raised ground layer.
        for (ordinal, (dx, dz, height)) in [
            (-0.23, -0.11, 0.25),
            (0.05, -0.22, 0.39),
            (0.25, 0.04, 0.30),
            (-0.14, 0.22, 0.34),
            (0.0, 0.02, 0.48),
        ]
        .into_iter()
        .enumerate()
        {
            let cx = x + dx * cos - dz * sin;
            let cz = z + dx * sin + dz * cos;
            let color = match (ordinal + usize::from(tuft.variant)) % 3 {
                0 => Color::srgb_u8(35, 91, 23),
                1 => Color::srgb_u8(67, 125, 31),
                _ => Color::srgb_u8(107, 157, 44),
            }
            .to_linear();
            mesh.box_at(
                [cx - 0.065, y, cz - 0.065],
                [cx + 0.065, y + height, cz + 0.065],
                [color.red, color.green, color.blue, 1.0],
            );
        }
    }
    let uvs = vec![[0., 0.]; mesh.positions.len()];
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, mesh.positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, mesh.normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, mesh.colors)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(mesh.indices))
}
impl Geometry {
    #[expect(
        clippy::expect_used,
        reason = "At most64tufts times120vertices enter one validated chunk mesh; u32 cannot overflow."
    )]
    fn box_at(&mut self, min: [f32; 3], max: [f32; 3], color: [f32; 4]) {
        let [a, b, c] = min;
        let [d, e, f] = max;
        for (normal, face) in [
            ([0., -1., 0.], [[a, b, f], [a, b, c], [d, b, c], [d, b, f]]),
            ([0., 1., 0.], [[a, e, c], [a, e, f], [d, e, f], [d, e, c]]),
            ([0., 0., 1.], [[a, b, f], [d, b, f], [d, e, f], [a, e, f]]),
            ([0., 0., -1.], [[d, b, c], [a, b, c], [a, e, c], [d, e, c]]),
            ([-1., 0., 0.], [[a, b, c], [a, b, f], [a, e, f], [a, e, c]]),
            ([1., 0., 0.], [[d, b, f], [d, b, c], [d, e, c], [d, e, f]]),
        ] {
            let base = u32::try_from(self.positions.len()).expect("bounded chunk vertices");
            self.positions.extend(face);
            self.normals.extend([normal; 4]);
            self.colors.extend([color; 4]);
            self.indices
                .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }
}

#[cfg(test)]
#[path = "grand_ground_cover_tests.rs"]
mod tests;
