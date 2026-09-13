//! One bounded asynchronous V4 target/neighbor preparation transaction at a time.
use super::StreamedArena;
use crate::v4::{
    PreparedChunk, PresentationLimits, RenderNeighbor, RenderOrigin, TerrainPresenter,
};
use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
};
use hex_core::arena::{ArenaRenderStatus, ArenaStreamInterest};
use hex_world_contracts::{ChunkId, ChunkPackage};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{mpsc, Arc, Mutex},
};

struct Completion {
    epoch: u64,
    target: ChunkId,
    revision: Option<u64>,
    prepared: Result<Vec<PreparedChunk>, String>,
}
#[derive(Resource)]
struct Renderer {
    presenter: TerrainPresenter,
    accepted: BTreeMap<ChunkId, u64>,
    proxies: BTreeMap<ChunkId, (Entity, Handle<Mesh>)>,
    hidden_proxies: BTreeSet<ChunkId>,
    materials: Vec<Handle<StandardMaterial>>,
    sender: mpsc::Sender<Completion>,
    receiver: Mutex<mpsc::Receiver<Completion>>,
    active: bool,
    epoch: u64,
}

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        PostUpdate,
        draw.before(TransformSystems::Propagate)
            .run_if(resource_exists::<StreamedArena>),
    );
}
pub(super) fn clear(world: &mut World) {
    if let Some(mut render) = world.remove_resource::<Renderer>() {
        render.presenter.clear(world);
        for (_, (entity, mesh)) in render.proxies {
            world.despawn(entity);
            if let Some(mut meshes) = world.get_resource_mut::<Assets<Mesh>>() {
                meshes.remove(mesh.id());
            }
        }
        if let Some(mut materials) = world.get_resource_mut::<Assets<StandardMaterial>>() {
            for material in render.materials {
                materials.remove(material.id());
            }
        }
    }
}
fn neighbors(c: ChunkId) -> impl Iterator<Item = ChunkId> {
    [(1, 0), (0, 1), (-1, 1), (-1, 0), (0, -1), (1, -1)]
        .into_iter()
        .map(move |(q, r)| ChunkId {
            q: c.q + q,
            r: c.r + r,
        })
}
fn visual_package(state: &StreamedArena, c: ChunkId) -> Result<Arc<ChunkPackage>, String> {
    let package = state
        .edits
        .presentation_package(c)
        .map_err(|e| e.to_string())?;
    // Water has one continuous ocean renderer; the V4 terrain presenter must not
    // draw opaque liquid prisms through that surface.
    let solid: BTreeSet<_> = state
        .runtime
        .manifest()
        .materials
        .iter()
        .filter(|m| m.solid)
        .map(|m| m.id.as_str())
        .collect();
    let mut package = (*package).clone();
    for column in &mut package.columns {
        column.runs.retain(|r| solid.contains(r.material.as_str()));
    }
    package.semantics.liquids.clear();
    package.seal().map_err(|e| e.to_string())?;
    Ok(Arc::new(package))
}
fn draw(world: &mut World) {
    if !world.contains_resource::<Assets<Mesh>>() {
        return;
    }
    if !world.contains_resource::<Renderer>() {
        let result =
            world.resource_scope(|world, state: Mut<StreamedArena>| new_renderer(world, &state));
        match result {
            Ok(r) => world.insert_resource(r),
            Err(e) => {
                error!("Northern renderer: {e}");
                return;
            }
        }
    }
    let interest = *world.resource::<ArenaStreamInterest>();
    let center = super::world_hex(hex_core::HexCoord::from_world(interest.position));
    world.resource_scope(|world, state: Mut<StreamedArena>| {
        world.resource_scope(|world, mut renderer: Mut<Renderer>| {
            let mut candidates: Vec<_> = state
                .runtime
                .resident_chunks()
                .map(|p| p.coordinate)
                .filter(|c| {
                    let q = c.q * 16 + 8 - center.q;
                    let r = c.r * 16 + 8 - center.r;
                    q.abs().max(r.abs()).max((q + r).abs()) <= 96
                })
                .collect();
            candidates.sort_by_key(|c| {
                let q = c.q * 16 + 8 - center.q;
                let r = c.r * 16 + 8 - center.r;
                q * q + r * r + q * r
            });
            candidates.truncate(256);
            let desired: BTreeSet<_> = candidates.iter().copied().collect();
            let completion = renderer
                .receiver
                .lock()
                .ok()
                .and_then(|r| r.try_recv().ok());
            if let Some(completion) = completion {
                renderer.active = false;
                if completion.epoch == renderer.epoch
                    && completion.revision.map_or_else(
                        || !desired.contains(&completion.target),
                        |revision| {
                            desired.contains(&completion.target)
                                && current_revision(&state, completion.target) == Some(revision)
                        },
                    )
                {
                    match completion.prepared {
                        Ok(prepared) => {
                            // Every rebuilt neighbor must still represent admitted
                            // current occupancy. Checking only the target lets an
                            // in-flight halo rebuild resurrect a carved neighbor.
                            let valid = prepared.iter().all(|p| {
                                if !desired.contains(&p.coordinate())
                                    || current_revision(&state, p.coordinate())
                                        != Some(p.revision())
                                {
                                    return false;
                                }
                                if let Err(e) = renderer.presenter.validate_publication(p) {
                                    error!("Northern publication preflight: {e}");
                                    return false;
                                }
                                true
                            });
                            if valid {
                                let mut published = BTreeMap::new();
                                let mut failed = false;
                                for p in prepared {
                                    match renderer.presenter.publish(world, p) {
                                        Ok(receipt) => {
                                            published.insert(receipt.coordinate, receipt.revision);
                                        }
                                        Err(e) => {
                                            error!("Northern publication: {e}");
                                            failed = true;
                                            break;
                                        }
                                    }
                                }
                                // All preflights run before publication, and this
                                // exclusive system keeps the presenter stable. If
                                // an unexpected admission failure still occurs,
                                // leave the target pending instead of hiding its
                                // proxy and permanently recording a false success.
                                if !failed {
                                    renderer.accepted.extend(published);
                                    if completion.revision.is_none() {
                                        renderer.presenter.remove(world, completion.target);
                                        renderer.accepted.remove(&completion.target);
                                    }
                                } else {
                                    renderer.accepted.remove(&completion.target);
                                }
                            }
                        }
                        Err(e) => error!("Northern mesh preparation: {e}"),
                    }
                }
            }
            // The static seabed covers the whole finite world. Only the bounded
            // detailed set changes visibility; do not touch thousands of distant
            // proxy entities every frame or invalidate their visibility caches.
            let detailed: BTreeSet<_> = renderer
                .presenter
                .receipts()
                .map(|receipt| receipt.coordinate)
                .collect();
            for chunk in renderer.hidden_proxies.symmetric_difference(&detailed) {
                let Some((entity, _)) = renderer.proxies.get(chunk) else {
                    continue;
                };
                if let Some(mut v) = world.get_mut::<Visibility>(*entity) {
                    *v = if detailed.contains(chunk) {
                        Visibility::Hidden
                    } else {
                        Visibility::Inherited
                    };
                }
            }
            renderer.hidden_proxies = detailed;
            let retired = renderer
                .presenter
                .receipts()
                .map(|receipt| &receipt.coordinate)
                .find(|c| !desired.contains(*c))
                .copied();
            let changed = candidates
                .iter()
                .find(|c| renderer.accepted.get(*c) != state.edits.revision(**c).as_ref())
                .copied();
            if let Some(mut status) = world.get_resource_mut::<ArenaRenderStatus>() {
                status.pending_chunks = usize::from(renderer.active)
                    + renderer
                        .presenter
                        .receipts()
                        .filter(|receipt| !desired.contains(&receipt.coordinate))
                        .count()
                    + candidates
                        .iter()
                        .filter(|c| renderer.accepted.get(*c) != state.edits.revision(**c).as_ref())
                        .count();
            }
            if renderer.active {
                return;
            }
            let Some(target) = retired.or(changed) else {
                return;
            };
            let revision = if retired.is_some() {
                None
            } else {
                state.edits.revision(target)
            };
            let mut snapshots = BTreeMap::new();
            let affected: Vec<_> = std::iter::once(target)
                .chain(neighbors(target))
                .filter(|c| {
                    *c == target
                        || (desired.contains(c) && renderer.presenter.package(*c).is_some())
                })
                .collect();
            for c in &affected {
                for n in std::iter::once(*c).chain(neighbors(*c)) {
                    if let Some(p) = renderer.presenter.render_neighbor(n) {
                        snapshots.insert(n, p);
                    }
                }
            }
            snapshots.remove(&target);
            // A previously rendered neighbor may already have newer live edits.
            // Refresh its immutable snapshot now; otherwise rejecting its stale
            // revision at completion would retry the same old package forever.
            for c in affected.iter().filter(|c| **c != target) {
                let Some(revision) = current_revision(&state, *c) else {
                    continue;
                };
                if snapshots.get(c).is_some_and(|old| old.revision == revision) {
                    continue;
                }
                match visual_package(&state, *c) {
                    Ok(package) => {
                        snapshots.insert(
                            *c,
                            RenderNeighbor {
                                package,
                                revision,
                                suppression: Arc::new(Vec::new()),
                            },
                        );
                    }
                    Err(e) => {
                        error!("Northern neighbor render source: {e}");
                        return;
                    }
                }
            }
            if let Some(revision) = revision {
                match visual_package(&state, target) {
                    Ok(package) => {
                        snapshots.insert(
                            target,
                            RenderNeighbor {
                                package,
                                revision,
                                suppression: Arc::new(Vec::new()),
                            },
                        );
                    }
                    Err(e) => {
                        error!("Northern render source: {e}");
                        return;
                    }
                }
            }
            let context = renderer.presenter.preparer();
            let sender = renderer.sender.clone();
            let epoch = renderer.epoch;
            match std::thread::Builder::new()
                .name("northern-mesh".into())
                .spawn(move || {
                    let prepared = affected
                        .into_iter()
                        .filter(|c| snapshots.contains_key(c))
                        .map(|c| {
                            let own = snapshots.get(&c).ok_or("missing preparation source")?;
                            let adjacent = neighbors(c)
                                .filter_map(|n| snapshots.get(&n).cloned())
                                .collect::<Vec<_>>();
                            let halo = context
                                .render_halo(c, &adjacent)
                                .map_err(|e| e.to_string())?;
                            context
                                .prepare_with_render_halo(
                                    &own.package,
                                    own.revision,
                                    &own.suppression,
                                    &halo,
                                )
                                .map_err(|e| e.to_string())
                        })
                        .collect();
                    let _sent = sender.send(Completion {
                        epoch,
                        target,
                        revision,
                        prepared,
                    });
                }) {
                Ok(_) => renderer.active = true,
                Err(e) => error!("Northern mesh worker: {e}"),
            }
        });
    });
}
fn current_revision(state: &StreamedArena, chunk: ChunkId) -> Option<u64> {
    // Edit revisions survive unloading; the revision ledger alone is not a
    // residency/readiness grant.
    state.runtime.resident_chunk(chunk)?;
    state.edits.revision(chunk)
}
fn new_renderer(world: &mut World, state: &StreamedArena) -> Result<Renderer, String> {
    let presenter = TerrainPresenter::with_limits(
        state.runtime.manifest(),
        RenderOrigin::default(),
        state.overview.level_height,
        PresentationLimits {
            max_resident_chunks: 256,
            max_local_hex: 2048,
            max_local_level: 4096,
            max_render_height: 1024.0,
            ..default()
        },
    )
    .map_err(|e| e.to_string())?;
    let (sender, receiver) = mpsc::channel();
    let material = world
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.96,
            reflectance: 0.05,
            ..default()
        });
    let mut proxies = BTreeMap::new();
    for descriptor in &state.runtime.manifest().chunks {
        let c = descriptor.coordinate;
        if let Some(mesh) = proxy(&state.overview, c) {
            let mesh = world.resource_mut::<Assets<Mesh>>().add(mesh);
            let entity = world
                .spawn((
                    Name::new("Northern distant terrain"),
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material.clone()),
                    Transform::default(),
                    Visibility::Inherited,
                ))
                .id();
            proxies.insert(c, (entity, mesh));
        }
    }
    Ok(Renderer {
        presenter,
        accepted: BTreeMap::new(),
        proxies,
        hidden_proxies: BTreeSet::new(),
        materials: vec![material],
        sender,
        receiver: Mutex::new(receiver),
        active: false,
        epoch: state.generation,
    })
}
const PROXY_STEPS: u32 = 8;

struct ProxySample {
    height: f32,
    normal: [f32; 3],
    color: [f32; 4],
}

#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "validated finite overview and bounded grid coordinates"
)]
fn proxy_sample(
    map: &hex_schematic::v4::northern::NorthernOverview,
    wx: f32,
    wz: f32,
) -> Option<ProxySample> {
    let [origin_x, origin_z] = map.origin_xz;
    let gx = ((wx - origin_x) / map.spacing).clamp(0.0, map.width.checked_sub(1)? as f32);
    let gz = ((wz - origin_z) / map.spacing).clamp(0.0, map.height.checked_sub(1)? as f32);
    let x = (gx.floor() as u32).min(map.width.checked_sub(2)?) as usize;
    let z = (gz.floor() as u32).min(map.height.checked_sub(2)?) as usize;
    let tx = gx - x as f32;
    let tz = gz - z as f32;
    let stride = map.width as usize;
    let at = z * stride + x;
    let corners = [at, at + 1, at + stride, at + stride + 1];
    let [a, b, c, d] = corners.map(|i| map.bed_heights.get(i).copied());
    let (a, b, c, d) = (a?, b?, c?, d?);
    let row0 = a + (b - a) * tx;
    let row1 = c + (d - c) * tx;
    // Match the ocean's bilinear bed surface. Flooring samples or lowering the
    // proxy separately lets the water mask expose a different coastline.
    let height = row0 + (row1 - row0) * tz;
    let dx = ((b - a) + ((d - c) - (b - a)) * tz) / map.spacing;
    let dz = (row1 - row0) / map.spacing;
    // A world-grid derivative gives both sides of every proxy seam the same
    // normal; per-mesh smooth normals only know each chunk's interior faces.
    let normal = Vec3::new(-dx, 1.0, -dz).normalize().to_array();
    let [a, b, c, d] = corners.map(|i| {
        let [r, g, b, a] = map
            .surface_materials
            .get(i)
            .and_then(|i| map.materials.get(usize::from(*i)))
            .map_or([110, 120, 125, 255], |m| m.color);
        // Mesh vertex colors are linear, just like the exact terrain material.
        let color = Color::srgba_u8(r, g, b, a).to_linear();
        Vec4::new(color.red, color.green, color.blue, color.alpha)
    });
    let color = a.lerp(b, tx).lerp(c.lerp(d, tx), tz).to_array();
    Some(ProxySample {
        height,
        normal,
        color,
    })
}

#[expect(
    clippy::cast_precision_loss,
    reason = "bounded finite chunk coordinates and fixed eight-step proxy topology"
)]
fn proxy(map: &hex_schematic::v4::northern::NorthernOverview, c: ChunkId) -> Option<Mesh> {
    let steps = usize::try_from(PROXY_STEPS).ok()?;
    let vertex_count = (steps + 1) * (steps + 1);
    let mut positions = Vec::with_capacity(vertex_count);
    let mut normals = Vec::with_capacity(vertex_count);
    let mut colors = Vec::with_capacity(vertex_count);
    let mut indices = Vec::with_capacity(steps * steps * 6);
    for r in 0..=PROXY_STEPS {
        for q in 0..=PROXY_STEPS {
            let x = (c.q * 16) as f32 + q as f32 * 2.0 - 0.5;
            let z = (c.r * 16) as f32 + r as f32 * 2.0 - 0.5;
            let wx = 3.0_f32.sqrt() * (x + z * 0.5);
            let wz = z * 1.5;
            let sample = proxy_sample(map, wx, wz)?;
            positions.push([wx, sample.height, wz]);
            normals.push(sample.normal);
            colors.push(sample.color);
        }
    }
    // Transparent water can reveal every submerged sample, including from below
    // the surface. Retain the original sampled relief throughout the finite
    // catalogue: a sea-level cutoff leaves an open sawtooth rim around islands.
    let stride = PROXY_STEPS + 1;
    for r in 0..PROXY_STEPS {
        for q in 0..PROXY_STEPS {
            let a = r * stride + q;
            indices.extend([a, a + stride, a + 1, a + 1, a + stride, a + stride + 1]);
        }
    }
    Some(
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
        .with_inserted_indices(Indices::U32(indices)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_schematic::v4::northern::NorthernOverview;

    fn planar_overview() -> NorthernOverview {
        NorthernOverview {
            version: 1,
            source_fingerprint: 0,
            package_fingerprint: 0,
            world_id: "proxy-test".into(),
            hex_radius: 1.0,
            level_height: 0.35,
            vertical_offset: 0.35,
            radius: 700,
            level_bounds: [0, 1400],
            sea_level: 140.0,
            origin_xz: [-64.0, -64.0],
            spacing: 8.0,
            width: 25,
            height: 25,
            bed_heights: (0_u16..25)
                .flat_map(|z| {
                    (0_u16..25).map(move |x| {
                        180.0 + 0.25 * (-64.0 + f32::from(x) * 8.0)
                            - 0.1 * (-64.0 + f32::from(z) * 8.0)
                    })
                })
                .collect(),
            surface_materials: vec![0; 625],
            materials: Vec::new(),
            player_spawn: [0.0; 3],
            anchors: BTreeMap::new(),
            islands: Vec::new(),
            tree_count: 0,
            building_count: 0,
        }
    }

    #[test]
    fn northern_proxy_preserves_fractional_world_height_without_vertical_bias() {
        let map = planar_overview();
        for (x, z) in [(3.25, 5.75), (-17.0, 21.0), (64.0, -32.0)] {
            let sample = proxy_sample(&map, x, z).expect("valid grid");
            assert!((sample.height - (180.0 + 0.25 * x - 0.1 * z)).abs() < 0.0001);
            assert!(
                Vec3::from_array(sample.normal).distance(Vec3::new(-0.25, 1.0, 0.1).normalize())
                    < 0.00001
            );
        }
    }

    #[test]
    fn northern_proxy_neighbors_share_positions_and_normals() {
        let map = planar_overview();
        let left = proxy(&map, ChunkId { q: 0, r: 0 }).expect("left land");
        let right = proxy(&map, ChunkId { q: 1, r: 0 }).expect("right land");
        assert_eq!(left.count_vertices(), 81);
        assert_eq!(left.indices().expect("triangles").len(), 384);
        assert_shared_edge(&left, &right);
    }

    fn assert_shared_edge(left: &Mesh, right: &Mesh) {
        for attribute in [Mesh::ATTRIBUTE_POSITION, Mesh::ATTRIBUTE_NORMAL] {
            let bevy::mesh::VertexAttributeValues::Float32x3(a) =
                left.attribute(attribute).expect("left attribute")
            else {
                panic!("float triples");
            };
            let bevy::mesh::VertexAttributeValues::Float32x3(b) =
                right.attribute(attribute).expect("right attribute")
            else {
                panic!("float triples");
            };
            for r in 0..9 {
                let a = Vec3::from_array(*a.get(r * 9 + 8).expect("left edge"));
                let b = Vec3::from_array(*b.get(r * 9).expect("right edge"));
                assert!(a.distance(b) < 0.00001, "shared edge discontinuity");
            }
        }
    }

    #[test]
    fn northern_proxy_submerged_chunks_continue_the_shore_without_missing_faces() {
        let mut map = planar_overview();
        for (index, height) in map.bed_heights.iter_mut().enumerate() {
            let column = u16::try_from(index % 25).expect("small test grid");
            let wx = -64.0 + f32::from(column) * 8.0;
            *height = 150.0 - wx;
        }
        let shore = proxy(&map, ChunkId { q: 0, r: 0 }).expect("partly exposed shore");
        let deep = proxy(&map, ChunkId { q: 1, r: 0 }).expect("entirely submerged continuation");
        assert_shared_edge(&shore, &deep);
        let bevy::mesh::VertexAttributeValues::Float32x3(positions) =
            deep.attribute(Mesh::ATTRIBUTE_POSITION).expect("positions")
        else {
            panic!("float triples");
        };
        assert_eq!(positions.len(), 81, "bounded static topology");
        for position in positions {
            let [x, y, _] = *position;
            assert!(y < map.sea_level - 2.0, "exercise the removed depth cutoff");
            assert!(
                (y - (150.0 - x)).abs() < 0.0001,
                "preserve the actual submerged slope"
            );
        }
        let indices: Vec<_> = deep.indices().expect("continuous seabed").iter().collect();
        assert_eq!(indices.len(), 384);
        let mut edges = BTreeMap::<(usize, usize), usize>::new();
        let mut projected_area = 0.0;
        for triangle in indices.chunks_exact(3) {
            let [a, b, c] = triangle else {
                panic!("three indices");
            };
            let [pa, pb, pc] = [a, b, c].map(|i| {
                Vec3::from_array(*positions.get(*i).expect("valid triangle index")).with_y(0.0)
            });
            let signed_area = (pb - pa).cross(pc - pa).y * 0.5;
            assert!(signed_area > 0.0, "upward-facing nondegenerate surface");
            projected_area += signed_area;
            for (a, b) in [(*a, *b), (*b, *c), (*c, *a)] {
                *edges.entry((a.min(b), a.max(b))).or_default() += 1;
            }
        }
        assert!(
            (projected_area - 384.0 * 3.0_f32.sqrt()).abs() < 0.001,
            "cover the complete chunk footprint without missing triangles"
        );
        assert_eq!(
            edges.values().filter(|count| **count == 1).count(),
            32,
            "only the four outer edges remain open for neighboring chunks"
        );
        assert!(
            edges.values().all(|count| matches!(*count, 1 | 2)),
            "no duplicated faces or interior cracks"
        );
    }
}
