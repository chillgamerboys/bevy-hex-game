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
use hex_world_contracts::{ChunkId, ChunkPackage, ColumnData, VoxelRun};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{mpsc, Arc, Mutex, OnceLock},
};

struct Completion {
    epoch: u64,
    target: ChunkId,
    revision: Option<u64>,
    authority: BTreeMap<ChunkId, u64>,
    retired: BTreeSet<ChunkId>,
    objects: BTreeMap<String, BTreeSet<ChunkId>>,
    edges: BTreeMap<ChunkId, BTreeMap<hex_world_contracts::WorldHex, f32>>,
    proxies: Vec<(ChunkId, Mesh)>,
    prepared: Result<Vec<PreparedChunk>, String>,
}
#[derive(Resource)]
struct Renderer {
    presenter: TerrainPresenter,
    accepted: BTreeMap<ChunkId, u64>,
    visible_objects: BTreeMap<String, BTreeSet<ChunkId>>,
    publication_revision: u64,
    terrain_edges: BTreeMap<ChunkId, BTreeMap<hex_world_contracts::WorldHex, f32>>,
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
// A root record supplies the complete footprint; clipped influences alone cannot
// prove that a tree crown and its support have all reached detailed presentation.
fn complete_objects(
    state: &StreamedArena,
    detailed: &BTreeSet<ChunkId>,
) -> BTreeMap<String, BTreeSet<ChunkId>> {
    state
        .runtime
        .resident_chunks()
        .flat_map(|p| {
            p.package
                .semantics
                .objects
                .iter()
                .filter_map(|object| {
                    let chunks: BTreeSet<_> = object
                        .occupancy
                        .iter()
                        .map(|column| column.position.chunk())
                        .chain(std::iter::once(object.origin.column.chunk()))
                        .collect();
                    chunks
                        .is_subset(detailed)
                        .then(|| (object.id.clone(), chunks))
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn merged_visible_runs(
    terrain: &[VoxelRun],
    object: &[VoxelRun],
    admitted: &[VoxelRun],
) -> Vec<VoxelRun> {
    let bounds: BTreeSet<_> = terrain
        .iter()
        .chain(object)
        .chain(admitted)
        .flat_map(|r| [r.bottom, r.top])
        .collect();
    let bounds: Vec<_> = bounds.into_iter().collect();
    let mut result: Vec<VoxelRun> = Vec::new();
    for pair in bounds.windows(2) {
        let [bottom, top] = *pair else {
            continue;
        };
        let at = |r: &&VoxelRun| r.bottom <= bottom && r.top > bottom;
        let material = terrain.iter().find(at).map(|r| &r.material).or_else(|| {
            let live = object.iter().find(at)?;
            admitted
                .iter()
                .any(|r| r.bottom <= bottom && r.top > bottom && r.material == live.material)
                .then_some(&live.material)
        });
        let Some(material) = material else {
            continue;
        };
        if let Some(last) = result
            .last_mut()
            .filter(|r| r.top == bottom && &r.material == material)
        {
            last.top = top;
        } else {
            result.push(VoxelRun {
                bottom,
                top,
                material: material.clone(),
            });
        }
    }
    result
}

fn visual_package(
    state: &StreamedArena,
    c: ChunkId,
    objects: &BTreeMap<String, BTreeSet<ChunkId>>,
) -> Result<Arc<ChunkPackage>, String> {
    let source = state
        .runtime
        .resident_chunk(c)
        .ok_or("render source is not resident")?;
    let solid: BTreeSet<_> = state
        .runtime
        .manifest()
        .materials
        .iter()
        .filter(|m| m.solid)
        .map(|m| m.id.as_str())
        .collect();
    let mut admitted = BTreeMap::<_, Vec<VoxelRun>>::new();
    for influence in &source.package.semantics.object_influences {
        if objects.contains_key(&influence.id) {
            for column in &influence.occupancy {
                admitted
                    .entry(column.position)
                    .or_default()
                    .extend(column.runs.iter().cloned());
            }
        }
    }
    let mut package = (*source.package).clone();
    package.columns = source
        .package
        .columns
        .iter()
        .filter_map(|column| {
            let terrain = state.edits.terrain_column(column.position)?;
            let object = state.edits.object_column(column.position);
            let mut runs = merged_visible_runs(
                &terrain.runs,
                object.as_ref().map_or(&[], |c| c.runs.as_slice()),
                admitted.get(&column.position).map_or(&[], Vec::as_slice),
            );
            runs.retain(|r| solid.contains(r.material.as_str()));
            Some(ColumnData {
                position: column.position,
                runs,
            })
        })
        .collect();
    // This disposable source contains surviving geometry, never fresh gameplay
    // promises for a carved object or partial root. Authority retains the source.
    package.semantics = default();
    package.seal().map_err(|e| e.to_string())?;
    Ok(Arc::new(package))
}
#[expect(
    clippy::cast_precision_loss,
    reason = "published finite terrain levels are bounded to 4096"
)]
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
                                        != completion.authority.get(&p.coordinate()).copied()
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
                                for chunk in &completion.retired {
                                    renderer.presenter.remove(world, *chunk);
                                    renderer.accepted.remove(chunk);
                                }
                                let mut published = BTreeMap::new();
                                let mut failed = false;
                                for p in prepared {
                                    match renderer.presenter.publish(world, p) {
                                        Ok(receipt) => {
                                            if let Some(authority) =
                                                completion.authority.get(&receipt.coordinate)
                                            {
                                                published.insert(receipt.coordinate, *authority);
                                            }
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
                                    renderer.visible_objects = completion.objects;
                                    for chunk in &completion.retired {
                                        renderer.terrain_edges.remove(chunk);
                                    }
                                    renderer.terrain_edges.extend(completion.edges);
                                    for (chunk, mesh) in completion.proxies {
                                        if let Some((_, handle)) = renderer.proxies.get(&chunk) {
                                            if let Some(old) =
                                                world.resource_mut::<Assets<Mesh>>().get_mut(handle)
                                            {
                                                *old = mesh;
                                            }
                                        }
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
            let retired: BTreeSet<_> = renderer
                .presenter
                .receipts()
                .map(|r| r.coordinate)
                .filter(|c| !desired.contains(c))
                .collect();
            let mut next_detailed: BTreeSet<_> = renderer
                .presenter
                .receipts()
                .map(|r| r.coordinate)
                .filter(|c| desired.contains(c))
                .collect();
            if revision.is_some() {
                next_detailed.insert(target);
            }
            let objects = complete_objects(&state, &next_detailed);
            let mut affected: BTreeSet<_> = std::iter::once(target)
                .chain(retired.iter().copied())
                .flat_map(|c| std::iter::once(c).chain(neighbors(c)))
                .filter(|c| next_detailed.contains(c))
                .collect();
            // The last arriving section makes all sections visible together; the
            // first retiring section hides all surviving fragments in this same
            // publication. It never exposes only a trunk or a sliver of crown.
            let changed_objects = objects
                .keys()
                .chain(renderer.visible_objects.keys())
                .filter(|id| objects.get(*id) != renderer.visible_objects.get(*id));
            for id in changed_objects {
                for chunk in objects
                    .get(id)
                    .or_else(|| renderer.visible_objects.get(id))
                    .into_iter()
                    .flatten()
                {
                    affected.extend(
                        std::iter::once(*chunk)
                            .chain(neighbors(*chunk))
                            .filter(|c| next_detailed.contains(c)),
                    );
                }
            }
            renderer.publication_revision = renderer.publication_revision.saturating_add(1);
            let render_revision = renderer.publication_revision;
            let mut authority = BTreeMap::new();
            let mut edges =
                BTreeMap::<ChunkId, BTreeMap<hex_world_contracts::WorldHex, f32>>::new();
            let mut snapshots = BTreeMap::new();
            for c in &affected {
                for n in neighbors(*c) {
                    if let Some(p) = renderer
                        .presenter
                        .render_neighbor(n)
                        .filter(|_| next_detailed.contains(&n))
                    {
                        snapshots.insert(n, p);
                    }
                }
            }
            for c in &affected {
                let Some(authority_revision) = current_revision(&state, *c) else {
                    return;
                };
                authority.insert(*c, authority_revision);
                let edge = state
                    .runtime
                    .resident_chunk(*c)
                    .map(|source| {
                        source
                            .package
                            .columns
                            .iter()
                            .filter(|column| {
                                let q = column.position.q.rem_euclid(16);
                                let r = column.position.r.rem_euclid(16);
                                q == 0 || q == 15 || r == 0 || r == 15
                            })
                            .filter_map(|column| {
                                let terrain = state.edits.terrain_column(column.position)?;
                                let top = terrain
                                    .runs
                                    .iter()
                                    .rev()
                                    .find(|run| {
                                        state
                                            .runtime
                                            .manifest()
                                            .materials
                                            .iter()
                                            .any(|m| m.id == run.material && m.solid)
                                    })
                                    .map_or(0, |r| r.top);
                                Some((column.position, top as f32 * state.overview.level_height))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                edges.insert(*c, edge);
                match visual_package(&state, *c, &objects) {
                    Ok(package) => {
                        snapshots.insert(
                            *c,
                            RenderNeighbor {
                                package,
                                revision: render_revision,
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
            let mut next_edges = renderer.terrain_edges.clone();
            for chunk in &retired {
                next_edges.remove(chunk);
            }
            next_edges.extend(edges.iter().map(|(c, columns)| (*c, columns.clone())));
            let changed_proxies: BTreeSet<_> = authority
                .keys()
                .chain(&retired)
                .flat_map(|c| {
                    (-1..=1).flat_map(move |q| {
                        (-1..=1).map(move |r| ChunkId {
                            q: c.q + q,
                            r: c.r + r,
                        })
                    })
                })
                .filter(|c| renderer.proxies.contains_key(c))
                .collect();
            let overview = state.overview.clone();
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
                    let proxies = proxy_updates(&overview, &next_edges, &changed_proxies);
                    let _sent = sender.send(Completion {
                        epoch,
                        target,
                        revision,
                        authority,
                        retired,
                        objects,
                        edges,
                        proxies,
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
        visible_objects: BTreeMap::new(),
        publication_revision: 0,
        terrain_edges: BTreeMap::new(),
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

#[derive(Clone)]
struct BoundaryEdge {
    a: Vec2,
    b: Vec2,
    outside: [i32; 2],
}
struct ProxyTopology {
    positions: Vec<Vec2>,
    indices: Vec<u32>,
    boundary: Vec<BoundaryEdge>,
}
fn cross(a: Vec2, b: Vec2) -> f32 {
    a.x * b.y - a.y * b.x
}
fn clip_axis(poly: &[Vec2], axis: usize, value: f32, minimum: bool) -> Vec<Vec2> {
    let component = |p: Vec2| if axis == 0 { p.x } else { p.y };
    let inside = |p: Vec2| {
        if minimum {
            component(p) >= value - 0.00001
        } else {
            component(p) <= value + 0.00001
        }
    };
    let mut result = Vec::new();
    for (a, b) in poly
        .iter()
        .copied()
        .zip(poly.iter().copied().cycle().skip(1))
        .take(poly.len())
    {
        if inside(a) {
            result.push(a);
        }
        if inside(a) != inside(b) {
            let fraction = (value - component(a)) / (component(b) - component(a));
            result.push(a.lerp(b, fraction));
        }
    }
    result.dedup_by(|a, b| a.distance_squared(*b) < 0.0000001);
    if result
        .first()
        .zip(result.last())
        .is_some_and(|(a, b)| a.distance_squared(*b) < 0.0000001)
    {
        result.truncate(result.len().saturating_sub(1));
    }
    result
}
fn triangulate(poly: &[Vec2]) -> Vec<usize> {
    let mut ring: Vec<_> = (0..poly.len()).collect();
    let mut triangles = Vec::new();
    while ring.len() >= 3 {
        let mut ear = None;
        for (i, b) in ring.iter().copied().enumerate() {
            let Some(&a) = ring.get((i + ring.len() - 1) % ring.len()) else {
                continue;
            };
            let Some(&c) = ring.get((i + 1) % ring.len()) else {
                continue;
            };
            let (Some(pa), Some(pb), Some(pc)) = (poly.get(a), poly.get(b), poly.get(c)) else {
                continue;
            };
            if cross(*pb - *pa, *pc - *pb) >= -0.000001 {
                continue;
            }
            let contains = ring
                .iter()
                .copied()
                .filter(|p| ![a, b, c].contains(p))
                .any(|p| {
                    poly.get(p).is_some_and(|p| {
                        cross(*pb - *pa, *p - *pa) < -0.000001
                            && cross(*pc - *pb, *p - *pb) < -0.000001
                            && cross(*pa - *pc, *p - *pc) < -0.000001
                    })
                });
            if !contains {
                ear = Some((i, [a, b, c]));
                break;
            }
        }
        let Some((index, triangle)) = ear else {
            break;
        };
        triangles.extend(triangle);
        ring.remove(index);
    }
    triangles
}
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    reason = "fixed sixteen-column proxy topology has fewer than 1024 vertices"
)]
fn proxy_topology() -> &'static ProxyTopology {
    static TOPOLOGY: OnceLock<ProxyTopology> = OnceLock::new();
    TOPOLOGY.get_or_init(|| {
        let corners = [(1, 1), (2, -1), (1, -2), (-1, -1), (-2, 1), (-1, 2)];
        let directions = [(1, 0), (1, -1), (0, -1), (-1, 0), (-1, 1), (0, 1)];
        let mut edges = BTreeMap::new();
        for r in 0..16 {
            for q in 0..16 {
                for (i, (dq, dr)) in directions.iter().copied().enumerate() {
                    let outside = [q + dq, r + dr];
                    if outside.into_iter().all(|v| (0..16).contains(&v)) {
                        continue;
                    }
                    let Some(&(aq, ar)) = corners.get(i) else {
                        continue;
                    };
                    let Some(&(bq, br)) = corners.get((i + 1) % 6) else {
                        continue;
                    };
                    edges.insert(
                        (q * 3 + aq, r * 3 + ar),
                        ((q * 3 + bq, r * 3 + br), outside),
                    );
                }
            }
        }
        let mut boundary = Vec::new();
        let mut polygon = Vec::new();
        if let Some(&start) = edges.keys().next() {
            let mut at = start;
            for _ in 0..edges.len() {
                let Some(&(next, outside)) = edges.get(&at) else {
                    break;
                };
                let a = Vec2::new(at.0 as f32, at.1 as f32) / 3.0;
                let b = Vec2::new(next.0 as f32, next.1 as f32) / 3.0;
                polygon.push(a);
                boundary.push(BoundaryEdge { a, b, outside });
                at = next;
            }
        }
        let mut positions = Vec::<Vec2>::new();
        let mut indices = Vec::new();
        for r in 0..PROXY_STEPS {
            for q in 0..PROXY_STEPS {
                let mut cell = polygon.clone();
                for (axis, index) in [(0, q), (1, r)] {
                    if index > 0 {
                        cell = clip_axis(&cell, axis, index as f32 * 2.0 - 0.5, true);
                    }
                    if index + 1 < PROXY_STEPS {
                        cell = clip_axis(&cell, axis, index as f32 * 2.0 + 1.5, false);
                    }
                }
                let local = triangulate(&cell);
                let mapping: Vec<_> = cell
                    .into_iter()
                    .map(|point| {
                        let index = positions
                            .iter()
                            .position(|p| p.distance_squared(point) < 0.0000001)
                            .unwrap_or_else(|| {
                                positions.push(point);
                                positions.len() - 1
                            });
                        index as u32
                    })
                    .collect();
                indices.extend(local.into_iter().filter_map(|i| mapping.get(i).copied()));
            }
        }
        ProxyTopology {
            positions,
            indices,
            boundary,
        }
    })
}

#[derive(Clone)]
struct EdgeTransition {
    owner: ChunkId,
    a: Vec2,
    b: Vec2,
    height: f32,
    top_a: f32,
    top_b: f32,
}
const TRANSITION_WIDTH: f32 = 4.0;
fn segment_projection(point: Vec2, a: Vec2, b: Vec2) -> (Vec2, f32) {
    let t = ((point - a).dot(b - a) / (b - a).length_squared()).clamp(0.0, 1.0);
    (a.lerp(b, t), t)
}
fn transition_height(base: f32, point: Vec2, edges: &[EdgeTransition]) -> f32 {
    let nearest = edges
        .iter()
        .map(|edge| {
            let (at, t) = segment_projection(point, edge.a, edge.b);
            (
                point.distance(at),
                edge.top_a + (edge.top_b - edge.top_a) * t,
            )
        })
        .filter(|(d, _)| *d < TRANSITION_WIDTH)
        .min_by(|a, b| a.0.total_cmp(&b.0));
    let Some((distance, height)) = nearest else {
        return base;
    };
    // All incident edge heights contribute at shared corners, deterministically.
    let height = edges
        .iter()
        .filter_map(|edge| {
            let (at, t) = segment_projection(point, edge.a, edge.b);
            (point.distance(at) <= distance + 0.0001)
                .then_some(edge.top_a + (edge.top_b - edge.top_a) * t)
        })
        .fold(height, f32::max);
    let t = (distance / TRANSITION_WIDTH).clamp(0.0, 1.0);
    height + (base - height) * (t * t * (3.0 - 2.0 * t))
}
fn axial_xz(q: f32, r: f32) -> Vec2 {
    Vec2::new(3.0_f32.sqrt() * (q + r * 0.5), r * 1.5)
}
fn proxy(map: &hex_schematic::v4::northern::NorthernOverview, c: ChunkId) -> Option<Mesh> {
    proxy_with_edges(map, c, &[])
}
#[expect(
    clippy::cast_precision_loss,
    reason = "validated finite chunk coordinates and fixed proxy topology"
)]
fn proxy_with_edges(
    map: &hex_schematic::v4::northern::NorthernOverview,
    c: ChunkId,
    edges: &[EdgeTransition],
) -> Option<Mesh> {
    let topology = proxy_topology();
    let mut positions = Vec::with_capacity(topology.positions.len() + edges.len() * 4);
    let mut normals = Vec::with_capacity(positions.capacity());
    let mut colors = Vec::with_capacity(positions.capacity());
    let mut indices = topology.indices.clone();
    let point_sample = |point: Vec2| -> Option<(Vec3, [f32; 3], [f32; 4])> {
        let sample = proxy_sample(map, point.x, point.y)?;
        let height = transition_height(sample.height, point, edges);
        let normal = if edges.is_empty() {
            sample.normal
        } else {
            let height_at = |p: Vec2| {
                proxy_sample(map, p.x, p.y).map(|sample| transition_height(sample.height, p, edges))
            };
            let dx = height_at(point + Vec2::X * 0.125)? - height_at(point - Vec2::X * 0.125)?;
            let dz = height_at(point + Vec2::Y * 0.125)? - height_at(point - Vec2::Y * 0.125)?;
            Vec3::new(-dx / 0.25, 1.0, -dz / 0.25)
                .normalize()
                .to_array()
        };
        Some((Vec3::new(point.x, height, point.y), normal, sample.color))
    };
    for local in &topology.positions {
        let point = axial_xz((c.q * 16) as f32 + local.x, (c.r * 16) as f32 + local.y);
        let (position, normal, color) = point_sample(point)?;
        positions.push(position.to_array());
        normals.push(normal);
        colors.push(color);
    }
    // A junction of differently elevated voxel tops needs a short vertical cut
    // face. Its endpoints are the actual fine top and the matched coarse edge,
    // never an arbitrary skirt depth or geometry extending into the fine area.
    for edge in edges.iter().filter(|edge| edge.owner == c) {
        let (a, _, color) = point_sample(edge.a)?;
        let (b, _, _) = point_sample(edge.b)?;
        if (a.y - edge.height).abs() < 0.0001 && (b.y - edge.height).abs() < 0.0001 {
            continue;
        }
        let first = u32::try_from(positions.len()).ok()?;
        let bottom_a = Vec3::new(edge.a.x, edge.height, edge.a.y);
        let bottom_b = Vec3::new(edge.b.x, edge.height, edge.b.y);
        let normal = (bottom_b - bottom_a)
            .cross(Vec3::Y)
            .normalize_or_zero()
            .to_array();
        positions.extend([
            bottom_a.to_array(),
            bottom_b.to_array(),
            b.to_array(),
            a.to_array(),
        ]);
        normals.extend([normal; 4]);
        colors.extend([color; 4]);
        if (b.y - edge.height).abs() > 0.0001 {
            indices.extend([first, first + 1, first + 2]);
        }
        if (a.y - edge.height).abs() > 0.0001 {
            indices.extend([first, first + 2, first + 3]);
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
fn match_edge_corners(edges: &mut [EdgeTransition]) {
    let heights: Vec<_> = edges
        .iter()
        .map(|edge| (edge.a, edge.b, edge.height))
        .collect();
    for edge in edges {
        for (point, height) in [(&edge.a, &mut edge.top_a), (&edge.b, &mut edge.top_b)] {
            for (a, b, neighbor_height) in &heights {
                if point.distance_squared(*a) < 0.000001 || point.distance_squared(*b) < 0.000001 {
                    *height = height.max(*neighbor_height);
                }
            }
        }
    }
}
#[expect(
    clippy::cast_precision_loss,
    reason = "validated finite chunk coordinates"
)]
fn nearby_transition_edges(
    chunk: ChunkId,
    terrain_edges: &BTreeMap<ChunkId, BTreeMap<hex_world_contracts::WorldHex, f32>>,
) -> Vec<EdgeTransition> {
    let mut edges = Vec::new();
    // A shared coarse/coarse corner must see the same nearby fine-edge field.
    // Include diagonal storage neighbors; six axial neighbors alone miss a
    // four-unit transition crossing a storage rectangle's corner.
    for dq in -1..=1 {
        for dr in -1..=1 {
            let owner = ChunkId {
                q: chunk.q + dq,
                r: chunk.r + dr,
            };
            if terrain_edges.contains_key(&owner) {
                continue;
            }
            for edge in &proxy_topology().boundary {
                let [q, r] = edge.outside;
                let outside = hex_world_contracts::WorldHex::new(
                    owner.q * 16 + i64::from(q),
                    owner.r * 16 + i64::from(r),
                );
                let Some(height) = terrain_edges
                    .get(&outside.chunk())
                    .and_then(|columns| columns.get(&outside))
                else {
                    continue;
                };
                let origin = Vec2::new((owner.q * 16) as f32, (owner.r * 16) as f32);
                let a = origin + edge.a;
                let b = origin + edge.b;
                edges.push(EdgeTransition {
                    owner,
                    a: axial_xz(a.x, a.y),
                    b: axial_xz(b.x, b.y),
                    height: *height,
                    top_a: *height,
                    top_b: *height,
                });
            }
        }
    }
    match_edge_corners(&mut edges);
    edges
}
fn proxy_updates(
    map: &hex_schematic::v4::northern::NorthernOverview,
    terrain_edges: &BTreeMap<ChunkId, BTreeMap<hex_world_contracts::WorldHex, f32>>,
    changed: &BTreeSet<ChunkId>,
) -> Vec<(ChunkId, Mesh)> {
    changed
        .iter()
        .filter(|c| !terrain_edges.contains_key(c))
        .filter_map(|chunk| {
            let edges = nearby_transition_edges(*chunk, terrain_edges);
            proxy_with_edges(map, *chunk, &edges).map(|mesh| (*chunk, mesh))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_schematic::v4::northern::NorthernOverview;

    #[test]
    fn northern_object_admission_preserves_carves_and_excludes_unadmitted_fragments() {
        let run = |bottom, top, material: &str| VoxelRun {
            bottom,
            top,
            material: material.into(),
        };
        let terrain = vec![run(0, 4, "rock")];
        let blueprint = vec![run(4, 10, "timber"), run(12, 16, "foliage")];
        let surviving = vec![
            run(4, 6, "timber"),
            run(7, 10, "timber"),
            run(12, 16, "foliage"),
        ];
        assert_eq!(
            merged_visible_runs(&terrain, &surviving, &[]),
            terrain,
            "an incomplete object's pieces must not be presented"
        );
        let visible = merged_visible_runs(&terrain, &surviving, &blueprint);
        assert_eq!(
            visible,
            terrain
                .iter()
                .chain(&surviving)
                .cloned()
                .collect::<Vec<_>>()
        );
        assert!(
            !visible.iter().any(|r| r.bottom <= 6 && r.top > 6),
            "whole-footprint admission must not resurrect a carved cell"
        );
    }

    #[test]
    #[ignore = "requires HEX_NORTHERN_WORLD pointing to the full-scale package"]
    fn northern_actual_tree_is_never_visible_from_only_crown_chunks() {
        use hex_core::arena::{ArenaMap, ArenaReset, ArenaSelection};
        let mut world = World::new();
        world.insert_resource(ArenaSelection {
            map: ArenaMap::NorthernArchipelago,
            ..default()
        });
        world.init_resource::<ArenaReset>();
        world.init_resource::<super::super::super::ArenaInbox>();
        world.init_resource::<crate::terrain_damage::TerrainDamageState>();
        world.init_resource::<hex_core::DamagedVoxels>();
        world.init_resource::<Messages<hex_core::TerrainImpactOutcome>>();
        world.init_resource::<Messages<AppExit>>();
        super::super::initialize(
            &mut world,
            super::super::super::load_content().expect("battle catalogs"),
        )
        .expect("actual package");
        for _ in 0..1000 {
            super::super::pump(&mut world);
            let state = world.resource::<StreamedArena>();
            if state.runtime.counts().queued_chunks == 0
                && state.runtime.counts().in_flight_jobs == 0
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let state = world.resource::<StreamedArena>();
        let all: BTreeSet<_> = state
            .runtime
            .resident_chunks()
            .map(|p| p.coordinate)
            .collect();
        let objects = complete_objects(state, &all);
        let chunks = objects
            .get("northern/tree/0003")
            .expect("complete bay tree footprint");
        assert!(
            chunks.len() > 1,
            "reproduce a tree crossing the publication boundary"
        );
        let root = state
            .runtime
            .resident_chunks()
            .find_map(|p| {
                p.package
                    .semantics
                    .objects
                    .iter()
                    .find(|o| o.id == "northern/tree/0003")
                    .cloned()
            })
            .expect("authored tree");
        let mut shared_voxels = 0;
        let mut exclusive_voxels = 0;
        for removed in chunks {
            let mut partial = all.clone();
            partial.remove(removed);
            let admitted = complete_objects(state, &partial);
            assert!(!admitted.contains_key(&root.id));
            for chunk in chunks.intersection(&partial) {
                let package = visual_package(state, *chunk, &admitted).expect("partial view");
                for column in root
                    .occupancy
                    .iter()
                    .filter(|column| column.position.chunk() == *chunk)
                {
                    let presented = package
                        .columns
                        .iter()
                        .find(|c| c.position == column.position)
                        .expect("column");
                    for run in &column.runs {
                        let source = state
                            .runtime
                            .resident_chunk(*chunk)
                            .expect("resident section");
                        for level in run.bottom..run.top {
                            let another_complete_object = source
                                .package
                                .semantics
                                .object_influences
                                .iter()
                                .filter(|influence| {
                                    influence.id != root.id && admitted.contains_key(&influence.id)
                                })
                                .any(|influence| {
                                    influence.occupancy.iter().any(|other| {
                                        other.position == column.position
                                            && other.material_at(level)
                                                == Some(run.material.as_str())
                                    })
                                });
                            if another_complete_object {
                                shared_voxels += 1;
                                assert_eq!(presented.material_at(level), Some(run.material.as_str()),
                                    "shared crown cells still belong to a complete neighboring tree");
                            } else {
                                exclusive_voxels += 1;
                                assert_ne!(
                                    presented.material_at(level),
                                    Some(run.material.as_str()),
                                    "an incomplete tree must have no exclusive floating fragment"
                                );
                            }
                        }
                    }
                }
            }
        }
        assert!(shared_voxels > 0, "exercise overlapping authored crowns");
        assert!(exclusive_voxels > 0, "exercise removed tree fragments");
        for chunk in chunks {
            let package = visual_package(state, *chunk, &objects).expect("whole tree view");
            let presenter = TerrainPresenter::with_limits(
                state.runtime.manifest(),
                RenderOrigin::default(),
                state.overview.level_height,
                PresentationLimits {
                    max_local_hex: 2048,
                    ..default()
                },
            )
            .expect("presenter");
            presenter
                .prepare(&package, 1)
                .expect("canonical render package");
        }
    }

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
        assert!(
            left.count_vertices() <= 256,
            "bounded static chunk vertices"
        );
        assert!(
            left.indices().expect("triangles").len() <= 768,
            "bounded static triangles"
        );
        assert_shared_edge(&left, &right);
    }

    fn assert_shared_edge(left: &Mesh, right: &Mesh) {
        let triples = |mesh: &Mesh, attribute: bevy::mesh::MeshVertexAttribute| {
            let bevy::mesh::VertexAttributeValues::Float32x3(values) =
                mesh.attribute(attribute).expect("attribute")
            else {
                panic!("triples");
            };
            values.clone()
        };
        let lp = triples(left, Mesh::ATTRIBUTE_POSITION);
        let rp = triples(right, Mesh::ATTRIBUTE_POSITION);
        let ln = triples(left, Mesh::ATTRIBUTE_NORMAL);
        let rn = triples(right, Mesh::ATTRIBUTE_NORMAL);
        let mut shared = 0;
        for (i, a) in lp.iter().copied().enumerate() {
            for (j, b) in rp.iter().copied().enumerate() {
                if Vec3::from_array(a)
                    .with_y(0.0)
                    .distance(Vec3::from_array(b).with_y(0.0))
                    < 0.00001
                {
                    shared += 1;
                    assert!(
                        Vec3::from_array(a).distance(Vec3::from_array(b)) < 0.00001,
                        "matched boundary heights"
                    );
                    assert!(
                        Vec3::from_array(*ln.get(i).expect("normal"))
                            .distance(Vec3::from_array(*rn.get(j).expect("normal")))
                            < 0.00001,
                        "matched boundary normals"
                    );
                }
            }
        }
        assert!(shared >= 32, "the full zigzag boundary is shared");
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
        assert!(positions.len() <= 256, "bounded static topology");
        for position in positions {
            let [x, y, _] = *position;
            assert!(y < map.sea_level - 2.0, "exercise the removed depth cutoff");
            assert!(
                (y - (150.0 - x)).abs() < 0.0001,
                "preserve the actual submerged slope"
            );
        }
        let indices: Vec<_> = deep.indices().expect("continuous seabed").iter().collect();
        assert!(indices.len() <= 768, "bounded static triangles");
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
        for ((a, b), count) in &edges {
            if *count != 1 {
                continue;
            }
            let midpoint = (Vec3::from_array(*positions.get(*a).expect("edge"))
                + Vec3::from_array(*positions.get(*b).expect("edge")))
                * 0.5;
            let local = Vec2::new(
                midpoint.x / 3.0_f32.sqrt() - midpoint.z / 3.0 - 16.0,
                midpoint.z / 1.5,
            );
            assert!(
                proxy_topology()
                    .boundary
                    .iter()
                    .any(
                        |edge| segment_projection(local, edge.a, edge.b).0.distance(local) < 0.0001
                    ),
                "only the actual hex footprint perimeter may have an unmatched edge"
            );
        }
        assert!(
            edges.values().all(|count| matches!(*count, 1 | 2)),
            "no duplicated faces or interior cracks"
        );
    }
    #[test]
    fn northern_proxy_transition_meets_exact_edges_without_filling_fine_terrain() {
        let map = planar_overview();
        let mut edges: Vec<_> = proxy_topology()
            .boundary
            .iter()
            .filter(|edge| edge.outside.first().is_some_and(|q| *q < 0))
            .map(|edge| {
                let height = if edge.a.y < 8.0 { 175.0 } else { 182.0 };
                EdgeTransition {
                    owner: ChunkId { q: 0, r: 0 },
                    a: axial_xz(edge.a.x, edge.a.y),
                    b: axial_xz(edge.b.x, edge.b.y),
                    height,
                    top_a: height,
                    top_b: height,
                }
            })
            .collect();
        match_edge_corners(&mut edges);
        let mesh = proxy_with_edges(&map, ChunkId { q: 0, r: 0 }, &edges).expect("transition mesh");
        let bevy::mesh::VertexAttributeValues::Float32x3(vertices) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION).expect("vertices")
        else {
            panic!("triples");
        };
        assert!(vertices.len() <= 512, "edge-only transition bound");
        for edge in &edges {
            for (point, height) in [
                (edge.a, edge.height),
                (edge.b, edge.height),
                (edge.a, edge.top_a),
                (edge.b, edge.top_b),
            ] {
                assert!(
                    vertices.iter().any(|p| Vec3::from_array(*p)
                        .distance(Vec3::new(point.x, height, point.y))
                        < 0.0001),
                    "each voxel edge reaches its true top and the matched coarse profile"
                );
            }
        }
        for point in &proxy_topology().positions {
            let xz = axial_xz(point.x, point.y);
            if edges.iter().all(|edge| {
                segment_projection(xz, edge.a, edge.b).0.distance(xz) >= TRANSITION_WIDTH
            }) {
                let exact = proxy_sample(&map, xz.x, xz.y).expect("sample").height;
                assert!(
                    vertices.iter().any(|p| Vec3::from_array(*p)
                        .distance(Vec3::new(xz.x, exact, xz.y))
                        < 0.0001),
                    "the overview remains unchanged beyond the four-unit boundary band"
                );
            }
        }
        assert!(
            edges
                .iter()
                .any(|edge| (edge.height - edge.top_a).abs() > 0.01
                    || (edge.height - edge.top_b).abs() > 0.01),
            "exercise a stepped junction, not just a flat edge"
        );
    }
    #[test]
    fn northern_transition_field_matches_at_diagonal_storage_corners() {
        let map = planar_overview();
        let mut columns = BTreeMap::new();
        for r in 0_i16..16 {
            for q in 0_i16..16 {
                if q == 0 || q == 15 || r == 0 || r == 15 {
                    columns.insert(
                        hex_world_contracts::WorldHex::new(i64::from(q), i64::from(r)),
                        178.0 + f32::from(r % 3) * 0.35,
                    );
                }
            }
        }
        let cache = BTreeMap::from([(ChunkId { q: 0, r: 0 }, columns)]);
        let left_id = ChunkId { q: 1, r: 0 };
        let right_id = ChunkId { q: 1, r: -1 };
        let left = proxy_with_edges(&map, left_id, &nearby_transition_edges(left_id, &cache))
            .expect("first coarse corner");
        let right = proxy_with_edges(&map, right_id, &nearby_transition_edges(right_id, &cache))
            .expect("diagonal coarse corner");
        let triples = |mesh: &Mesh, attribute: bevy::mesh::MeshVertexAttribute| {
            let bevy::mesh::VertexAttributeValues::Float32x3(v) =
                mesh.attribute(attribute).expect("attribute")
            else {
                panic!("triples");
            };
            v.clone()
        };
        let lp = triples(&left, Mesh::ATTRIBUTE_POSITION);
        let rp = triples(&right, Mesh::ATTRIBUTE_POSITION);
        let ln = triples(&left, Mesh::ATTRIBUTE_NORMAL);
        let rn = triples(&right, Mesh::ATTRIBUTE_NORMAL);
        let mut shared = 0;
        let mut morphed = 0;
        for (i, a) in lp
            .iter()
            .take(proxy_topology().positions.len())
            .copied()
            .enumerate()
        {
            for (j, b) in rp
                .iter()
                .take(proxy_topology().positions.len())
                .copied()
                .enumerate()
            {
                if Vec3::from_array(a)
                    .with_y(0.0)
                    .distance(Vec3::from_array(b).with_y(0.0))
                    < 0.0001
                {
                    shared += 1;
                    assert!(
                        Vec3::from_array(a).distance(Vec3::from_array(b)) < 0.0001,
                        "coarse/coarse corner height seam"
                    );
                    assert!(
                        Vec3::from_array(*ln.get(i).expect("normal"))
                            .distance(Vec3::from_array(*rn.get(j).expect("normal")))
                            < 0.002,
                        "coarse/coarse normal seam"
                    );
                    let [x, y, z] = a;
                    if (proxy_sample(&map, x, z).expect("sample").height - y).abs() > 0.1 {
                        morphed += 1;
                    }
                }
            }
        }
        assert!(shared >= 32, "complete shared zigzag edge");
        assert!(
            morphed > 0,
            "exercise a changed corner shared by coarse chunks"
        );
    }
}
