//! Whole authored ordinary trees remain visible independently of terrain residency.
//! Intact trees share opaque meshes. Only actual persistent object cuts allocate
//! an instance mesh; ground revisions never alter crowns or pin source chunks.
use super::StreamedArena;
use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
};
use hex_schematic::v4::northern::forest::{
    forest_mesh, ForestInstance, ForestMesh, ForestOverview, ForestShape, MAX_FOREST_VERTICES,
};
use hex_world_contracts::{ChunkId, MaterialSpec, VoxelPosition};
use hex_world_runtime::FiniteWorldSession;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Resource)]
struct Cache {
    identity: (u64, u64),
    forest: Option<Forest>,
}
struct Forest {
    source: ForestOverview,
    shapes: Vec<Handle<Mesh>>,
    material: Handle<StandardMaterial>,
    palette: BTreeMap<String, [f32; 4]>,
    trees: Vec<Tree>,
    submitted_vertices: usize,
}
struct Tree {
    instance: ForestInstance,
    body: Entity,
    root: Option<(Entity, Handle<Mesh>)>,
    edited: Option<Handle<Mesh>>,
    cuts: BTreeSet<VoxelPosition>,
    revisions: BTreeMap<ChunkId, u64>,
    vertices: usize,
    available: bool,
}
/// Called in the same exclusive publication boundary as complete detailed trees.
pub(super) fn sync(
    world: &mut World,
    state: &StreamedArena,
    detailed: &BTreeMap<String, BTreeSet<ChunkId>>,
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
    retire_stale(world, identity);
    if !world.contains_resource::<Cache>() {
        let forest = match state.overview.forest.as_ref() {
            Some(source) => match Forest::new(
                world,
                source,
                &state.runtime.manifest().materials,
                &state.edits,
            ) {
                Ok(forest) => Some(forest),
                Err(error) => {
                    error!("Grand distant forest admission failed: {error}");
                    None
                }
            },
            None => None, // Older packages and the explicitly plain checkpoint.
        };
        world.insert_resource(Cache { identity, forest });
    }
    world.resource_scope(|world,mut cache:Mut<Cache>| {
        let Some(forest)=cache.forest.as_mut() else {return;};
        for tree in &mut forest.trees {
            let Some(shape)=forest.source.shapes.get(tree.instance.shape) else {continue;};
            if tree.changed(&state.edits) {
                match tree.refresh(world,shape,&state.edits,&forest.palette,forest.submitted_vertices) {
                    Ok(Some(total))=>forest.submitted_vertices=total,
                    Ok(None)=>{},
                    Err(error)=>{
                        forest.submitted_vertices=forest.submitted_vertices.saturating_sub(tree.vertices);
                        tree.vertices=0;tree.available=false;
                        error!(object=%tree.instance.id,"Grand distant forest cut update failed: {error}");
                    }
                }
            }
            tree.publish_visibility(world,detailed.contains_key(&tree.instance.id));
        }
    });
}
fn retire_stale(world: &mut World, identity: (u64, u64)) {
    if world
        .get_resource::<Cache>()
        .is_some_and(|cache| cache.identity != identity)
    {
        clear(world);
    }
}
/// Remove every mesh/entity on map exit, source identity replacement or New Run.
pub(super) fn clear(world: &mut World) {
    let Some(cache) = world.remove_resource::<Cache>() else {
        return;
    };
    let Some(forest) = cache.forest else {
        return;
    };
    let mut handles = forest.shapes;
    for tree in forest.trees {
        world.despawn(tree.body);
        if let Some((entity, handle)) = tree.root {
            world.despawn(entity);
            handles.push(handle);
        }
        if let Some(handle) = tree.edited {
            handles.push(handle);
        }
    }
    if let Some(mut meshes) = world.get_resource_mut::<Assets<Mesh>>() {
        for handle in handles {
            meshes.remove(handle.id());
        }
    }
    if let Some(mut materials) = world.get_resource_mut::<Assets<StandardMaterial>>() {
        materials.remove(forest.material.id());
    }
}
struct Prepared {
    instance: ForestInstance,
    root: Option<Mesh>,
    edited: Option<Mesh>,
    cuts: BTreeSet<VoxelPosition>,
    vertices: usize,
}
impl Forest {
    fn new(
        world: &mut World,
        source: &ForestOverview,
        materials: &[MaterialSpec],
        edits: &FiniteWorldSession,
    ) -> Result<Self, String> {
        source.validate().map_err(|e| e.to_string())?;
        let palette: BTreeMap<_, _> = materials
            .iter()
            .map(|m| {
                let [r, g, b, a] = m.color;
                let color = Color::srgba_u8(r, g, b, a).to_linear();
                (
                    m.id.clone(),
                    [color.red, color.green, color.blue, color.alpha],
                )
            })
            .collect();
        let shared: Vec<_> = source
            .shapes
            .iter()
            .map(|shape| {
                forest_mesh(&shape.columns, &BTreeSet::new())
                    .map_err(|e| e.to_string())
                    .and_then(|mesh| render_mesh(mesh, &palette))
            })
            .collect::<Result<_, _>>()?;
        let mut prepared = Vec::new();
        let mut submitted_vertices = 0_usize;
        for instance in &source.instances {
            let shape = source
                .shapes
                .get(instance.shape)
                .ok_or("missing forest shared shape")?;
            let cuts = source_cuts(instance, shape, edits)?;
            let root = if instance.roots.is_empty() {
                None
            } else {
                Some(render_mesh(
                    forest_mesh(&instance.roots, &BTreeSet::new()).map_err(|e| e.to_string())?,
                    &palette,
                )?)
            };
            let edited = if cuts.is_empty() {
                None
            } else {
                Some(instance_mesh(instance, shape, &cuts, &palette)?)
            };
            let vertices = edited.as_ref().map_or_else(
                || {
                    shared.get(instance.shape).map_or(0, Mesh::count_vertices)
                        + root.as_ref().map_or(0, Mesh::count_vertices)
                },
                Mesh::count_vertices,
            );
            submitted_vertices = submitted_vertices
                .checked_add(vertices)
                .ok_or("forest vertex total overflow")?;
            if submitted_vertices > MAX_FOREST_VERTICES {
                return Err("whole forest vertex budget exceeded before publication".into());
            }
            prepared.push(Prepared {
                instance: instance.clone(),
                root,
                edited,
                cuts,
                vertices,
            });
        }
        // Complete validation precedes all ECS publication: no truncated forest.
        let shapes: Vec<_> = shared
            .into_iter()
            .map(|mesh| world.resource_mut::<Assets<Mesh>>().add(mesh))
            .collect();
        let material = world
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                base_color: Color::WHITE,
                perceptual_roughness: 0.96,
                reflectance: 0.05,
                ..default()
            });
        let mut trees = Vec::new();
        for pending in prepared {
            let transform = instance_transform(&pending.instance)?;
            let edited = pending
                .edited
                .map(|mesh| world.resource_mut::<Assets<Mesh>>().add(mesh));
            let body_mesh = edited
                .clone()
                .or_else(|| shapes.get(pending.instance.shape).cloned())
                .ok_or("prepared forest shape disappeared")?;
            let body = world
                .spawn((
                    Name::new(format!("Grand distant {}", pending.instance.id)),
                    Mesh3d(body_mesh),
                    MeshMaterial3d(material.clone()),
                    transform,
                    Visibility::Hidden,
                ))
                .id();
            let root = pending.root.map(|mesh| {
                let handle = world.resource_mut::<Assets<Mesh>>().add(mesh);
                let entity = world
                    .spawn((
                        Name::new("Grand distant grounded roots"),
                        Mesh3d(handle.clone()),
                        MeshMaterial3d(material.clone()),
                        transform,
                        Visibility::Hidden,
                    ))
                    .id();
                (entity, handle)
            });
            let revisions = pending
                .instance
                .footprint
                .iter()
                .map(|chunk| (*chunk, edits.revision(*chunk).unwrap_or(0)))
                .collect();
            trees.push(Tree {
                instance: pending.instance,
                body,
                root,
                edited,
                cuts: pending.cuts,
                revisions,
                vertices: pending.vertices,
                available: true,
            });
        }
        info!(
            instances = trees.len(),
            shapes = shapes.len(),
            submitted_vertices,
            "Grand distant forest prepared without terrain residency"
        );
        Ok(Self {
            source: source.clone(),
            shapes,
            material,
            palette,
            trees,
            submitted_vertices,
        })
    }
}
impl Tree {
    fn changed(&mut self, edits: &FiniteWorldSession) -> bool {
        let mut changed = false;
        for (chunk, observed) in &mut self.revisions {
            let current = edits.revision(*chunk).unwrap_or(0);
            changed |= current != *observed;
            *observed = current;
        }
        changed
    }
    fn refresh(
        &mut self,
        world: &mut World,
        shape: &ForestShape,
        edits: &FiniteWorldSession,
        palette: &BTreeMap<String, [f32; 4]>,
        total: usize,
    ) -> Result<Option<usize>, String> {
        let cuts = source_cuts(&self.instance, shape, edits)?;
        if cuts == self.cuts {
            return Ok(None);
        } // Terrain/refills/other objects leave this mesh unchanged.
        let mesh = instance_mesh(&self.instance, shape, &cuts, palette)?;
        let vertices = mesh.count_vertices();
        let total = total
            .checked_sub(self.vertices)
            .and_then(|n| n.checked_add(vertices))
            .ok_or("forest vertex total overflow")?;
        if total > MAX_FOREST_VERTICES {
            return Err("whole forest cut surface exceeds vertex budget".into());
        }
        if let Some(handle) = &self.edited {
            let mut assets = world.resource_mut::<Assets<Mesh>>();
            let mut previous = assets
                .get_mut(handle)
                .ok_or("edited forest mesh disappeared")?;
            *previous = mesh;
        } else {
            let handle = world.resource_mut::<Assets<Mesh>>().add(mesh);
            let mut body = world
                .get_mut::<Mesh3d>(self.body)
                .ok_or("forest entity disappeared")?;
            body.0 = handle.clone();
            self.edited = Some(handle);
        }
        self.vertices = vertices;
        self.cuts = cuts;
        self.available = vertices > 0;
        Ok(Some(total))
    }
    fn publish_visibility(&self, world: &mut World, complete: bool) {
        let show = self.available && !complete;
        set_visible(world, self.body, show);
        if let Some((entity, _)) = self.root {
            set_visible(world, entity, show && self.edited.is_none());
        }
    }
}
fn set_visible(world: &mut World, entity: Entity, show: bool) {
    let expected = if show {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    if let Some(mut visibility) = world.get_mut::<Visibility>(entity) {
        if *visibility != expected {
            *visibility = expected;
        }
    }
}
fn source_cuts(
    instance: &ForestInstance,
    shape: &ForestShape,
    edits: &FiniteWorldSession,
) -> Result<BTreeSet<VoxelPosition>, String> {
    let mut cuts = BTreeSet::new();
    for column in instance.columns(shape).map_err(|e| e.to_string())? {
        for cut in edits.removed_in_column(column.position) {
            if column.material_at(cut.level).is_some() {
                cuts.insert(instance.local_position(cut).map_err(|e| e.to_string())?);
            }
        }
    }
    Ok(cuts)
}
fn instance_mesh(
    instance: &ForestInstance,
    shape: &ForestShape,
    cuts: &BTreeSet<VoxelPosition>,
    palette: &BTreeMap<String, [f32; 4]>,
) -> Result<Mesh, String> {
    let columns = instance.local_columns(shape).map_err(|e| e.to_string())?;
    render_mesh(
        forest_mesh(&columns, cuts).map_err(|e| e.to_string())?,
        palette,
    )
}
#[expect(
    clippy::cast_precision_loss,
    reason = "Overview admission bounds exact roots to radius900 and levels0..1600."
)]
fn instance_transform(instance: &ForestInstance) -> Result<Transform, String> {
    let local =
        super::local(instance.origin.column).ok_or("forest origin exceeds render coordinates")?;
    Ok(
        Transform::from_translation(local.to_world(instance.base_level as f32 * 0.35))
            .with_rotation(Quat::from_rotation_y(
                -f32::from(instance.rotation) * std::f32::consts::FRAC_PI_3,
            )),
    )
}
fn render_mesh(source: ForestMesh, palette: &BTreeMap<String, [f32; 4]>) -> Result<Mesh, String> {
    let colors: Vec<_> = source
        .materials
        .iter()
        .map(|id| {
            palette
                .get(id)
                .copied()
                .ok_or_else(|| format!("forest material {id} is missing"))
        })
        .collect::<Result<_, _>>()?;
    let uvs = vec![[0., 0.]; source.positions.len()];
    Ok(Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, source.positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, source.normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(source.indices)))
}

#[cfg(test)]
#[path = "grand_forest_tests.rs"]
mod tests;
