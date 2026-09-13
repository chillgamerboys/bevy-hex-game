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
            for (chunk, (entity, _)) in &renderer.proxies {
                let detailed = renderer.presenter.package(*chunk).is_some();
                if let Some(mut v) = world.get_mut::<Visibility>(*entity) {
                    *v = if detailed {
                        Visibility::Hidden
                    } else {
                        Visibility::Inherited
                    };
                }
            }
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
        materials: vec![material],
        sender,
        receiver: Mutex::new(receiver),
        active: false,
        epoch: state.generation,
    })
}
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "bounded finite overview and chunk coordinates"
)]
fn proxy(map: &hex_schematic::v4::northern::NorthernOverview, c: ChunkId) -> Option<Mesh> {
    let [origin_x, origin_z] = map.origin_xz;
    let mut positions = Vec::new();
    let mut colors = Vec::new();
    let mut indices = Vec::new();
    let mut high = false;
    for r in 0..=4 {
        for q in 0..=4 {
            let x = (c.q * 16) as f32 + q as f32 * 4.0 - 0.5;
            let z = (c.r * 16) as f32 + r as f32 * 4.0 - 0.5;
            let wx = 3.0_f32.sqrt() * (x + z * 0.5);
            let wz = z * 1.5;
            let gx = ((wx - origin_x) / map.spacing).clamp(0.0, map.width.saturating_sub(1) as f32)
                as usize;
            let gz = ((wz - origin_z) / map.spacing).clamp(0.0, map.height.saturating_sub(1) as f32)
                as usize;
            let at = gz * map.width as usize + gx;
            let y = *map.bed_heights.get(at)?;
            high |= y > map.sea_level - 2.0;
            positions.push([wx, y - 0.25, wz]);
            let color = map
                .surface_materials
                .get(at)
                .and_then(|i| map.materials.get(usize::from(*i)))
                .map_or([110, 120, 125, 255], |m| m.color);
            let [r, g, b, a] = color;
            // StandardMaterial converts exact terrain's sRGB palette to linear.
            // Mesh vertex colors are already linear; raw byte/255 values make
            // distant proxies much brighter than the matching fine terrain.
            let linear = Color::srgba_u8(r, g, b, a).to_linear();
            colors.push([linear.red, linear.green, linear.blue, linear.alpha]);
        }
    }
    if !high {
        return None;
    }
    for r in 0..4 {
        for q in 0..4 {
            let a = (r * 5 + q) as u32;
            indices.extend([a, a + 5, a + 1, a + 1, a + 5, a + 6]);
        }
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
    .with_inserted_indices(Indices::U32(indices));
    mesh.compute_smooth_normals();
    Some(mesh)
}
