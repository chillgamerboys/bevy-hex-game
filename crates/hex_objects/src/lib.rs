//! Runtime presentation for editor-authored voxel objects.
//!
//! [`hex_assets`] owns durable object, style, and instance contracts. This crate
//! consumes their validated runtime projection and turns each blueprint into a
//! small set of shared render chunks. Object producers never need to depend on
//! this presentation implementation.
//!
//! Blend surfaces use Bevy's order-independent transparency. Bevy 0.19 requires
//! OIT cameras to disable MSAA, so cutout materials temporarily behave as
//! single-sample threshold masks while any Blend chunk is live. The renderer
//! restores each camera's previous MSAA mode as soon as the final Blend chunk
//! disappears, restoring true alpha-to-coverage for cutouts.

use std::collections::{BTreeMap, BTreeSet};
use std::f32::consts::TAU;

use bevy::asset::AssetEvent;
use bevy::core_pipeline::oit::OrderIndependentTransparencySettings;
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::prelude::*;
use bevy::render::render_resource::TextureUsages;
use hex_assets::{
    GameAssets, HexObjectRotation, LocalVoxelCoord, ObjectAssetId, ObjectBlueprint, ObjectInstance,
    ResolvedVoxelStyle, RuntimeArtCatalog, VoxelStyleId, VoxelSurfaceMode,
};
use hex_core::HexCoord;

/// Marks a generated render child belonging to one authored object instance.
///
/// Chunks are grouped by style and canopy membership. The grouping keeps entity
/// counts independent of voxel count while retaining the exact authored cutaway
/// boundary for a later world-presentation adapter.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct ObjectRenderChunk {
    style: VoxelStyleId,
    canopy: bool,
}

impl ObjectRenderChunk {
    /// Stable style used by every voxel in this chunk.
    #[must_use]
    pub fn style(&self) -> &VoxelStyleId {
        &self.style
    }

    /// Whether every voxel in this chunk belongs to the authored canopy mask.
    #[must_use]
    pub const fn is_canopy(&self) -> bool {
        self.canopy
    }
}

/// Marks a render chunk containing only exact authored canopy cells.
///
/// The marker intentionally has no camera or visibility behavior. Forest and
/// character-camera integration belongs to a later adapter.
#[derive(Component, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ObjectCanopyChunk;

#[derive(Component, Debug, Clone)]
struct RenderedObject {
    object_id: ObjectAssetId,
    catalog_fingerprint: u64,
    source_generation: u64,
    children: Vec<Entity>,
    failed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ChunkKey {
    style: VoxelStyleId,
    canopy: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct FaceCullMask {
    sides: [bool; 6],
    top: bool,
    bottom: bool,
}

impl FaceCullMask {
    fn is_empty(self) -> bool {
        !self.top && !self.bottom && self.sides.iter().all(|culled| !culled)
    }
}

#[derive(Debug, Clone)]
struct CachedChunk {
    key: ChunkKey,
    mesh: Handle<Mesh>,
    material: Handle<StandardMaterial>,
    surface_mode: VoxelSurfaceMode,
    casts_shadows: bool,
}

#[derive(Debug, Clone)]
struct CachedObject {
    chunks: Vec<CachedChunk>,
}

#[derive(Debug, Clone)]
struct CachedMaterial {
    handle: Handle<StandardMaterial>,
}

#[derive(Component, Debug, Default, Clone, Copy, PartialEq, Eq)]
struct ObjectTranslucentChunk;

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
struct ObjectOitCamera {
    previous_msaa: Msaa,
    preserve_oit: bool,
}

#[derive(Resource, Debug, Default)]
struct ObjectRenderCache {
    catalog_fingerprint: Option<u64>,
    source_generation: u64,
    objects: BTreeMap<ObjectAssetId, CachedObject>,
    materials: BTreeMap<VoxelStyleId, CachedMaterial>,
}

impl ObjectRenderCache {
    fn invalidate_objects(&mut self, meshes: &mut Assets<Mesh>) {
        for object in self.objects.values() {
            for chunk in &object.chunks {
                drop(meshes.remove(chunk.mesh.id()));
            }
        }
        self.objects.clear();
    }

    fn invalidate_all(
        &mut self,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<StandardMaterial>,
    ) {
        self.invalidate_objects(meshes);
        for material in self.materials.values() {
            drop(materials.remove(material.handle.id()));
        }
        self.materials.clear();
    }
}

/// Registers authored-object mesh baking and instance reconciliation.
pub fn plugin(app: &mut App) {
    app.init_resource::<ObjectRenderCache>()
        .add_systems(Update, (reconcile_objects, manage_object_oit).chain());
}

fn reconcile_objects(
    mut commands: Commands,
    catalog: Option<Res<RuntimeArtCatalog>>,
    game_assets: Option<Res<GameAssets>>,
    mut mesh_events: MessageReader<AssetEvent<Mesh>>,
    mut removed_instances: RemovedComponents<ObjectInstance>,
    instances: Query<(
        Entity,
        Ref<ObjectInstance>,
        Option<&RenderedObject>,
        Option<&Visibility>,
    )>,
    stale_rendered: Query<&RenderedObject, Without<ObjectInstance>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut cache: ResMut<ObjectRenderCache>,
) {
    for entity in removed_instances.read() {
        if instances.get(entity).is_ok() {
            continue;
        }
        if let Ok(rendered) = stale_rendered.get(entity) {
            despawn_render_children(&mut commands, rendered);
            commands.entity(entity).remove::<RenderedObject>();
        }
    }

    let (Some(catalog), Some(game_assets)) = (catalog, game_assets) else {
        mesh_events.clear();
        return;
    };

    let source_id = game_assets.hex_tile.id();
    let mut source_changed = false;
    for event in mesh_events.read() {
        source_changed |= event.is_added(source_id)
            || event.is_modified(source_id)
            || event.is_loaded_with_dependencies(source_id);
    }
    if meshes.get(&game_assets.hex_tile).is_none() {
        return;
    }

    let catalog_fingerprint = catalog.combined_fingerprint();
    let catalog_changed = cache.catalog_fingerprint != Some(catalog_fingerprint);
    if source_changed {
        cache.source_generation = cache.source_generation.wrapping_add(1);
    }
    if catalog_changed {
        cache.invalidate_all(&mut meshes, &mut materials);
        cache.catalog_fingerprint = Some(catalog_fingerprint);
    } else if source_changed {
        cache.invalidate_objects(&mut meshes);
    }
    let force_rebuild = catalog_changed || source_changed;
    let mut source_mesh = None;

    for (entity, instance, rendered, visibility) in &instances {
        if let Err(error) = instance.validate() {
            let already_reported = rendered.is_some_and(|rendered| {
                rendered.failed
                    && rendered.object_id == *instance.object_id()
                    && rendered.catalog_fingerprint == catalog_fingerprint
                    && rendered.source_generation == cache.source_generation
            });
            if !already_reported {
                if let Some(rendered) = rendered {
                    despawn_render_children(&mut commands, rendered);
                }
                error!(
                    "cannot render invalid authored object instance '{}': {error}",
                    instance.object_id()
                );
                commands.entity(entity).insert(RenderedObject {
                    object_id: instance.object_id().clone(),
                    catalog_fingerprint,
                    source_generation: cache.source_generation,
                    children: Vec::new(),
                    failed: true,
                });
            }
            continue;
        }

        let transform_changed = instance.is_changed();
        if transform_changed || rendered.is_none() {
            let transform = object_root_transform(&instance);
            commands.entity(entity).insert(transform);
        }
        if visibility.is_none() {
            commands.entity(entity).insert(Visibility::Inherited);
        }

        let needs_rebuild = force_rebuild
            || rendered.is_none()
            || rendered.is_some_and(|rendered| {
                rendered.failed
                    || rendered.object_id != *instance.object_id()
                    || rendered.catalog_fingerprint != catalog_fingerprint
                    || rendered.source_generation != cache.source_generation
            });
        if !needs_rebuild {
            continue;
        }
        if let Some(rendered) = rendered {
            despawn_render_children(&mut commands, rendered);
        }
        if source_mesh.is_none() {
            source_mesh = meshes.get(&game_assets.hex_tile).cloned();
        }
        let Some(source_mesh) = source_mesh.as_ref() else {
            continue;
        };

        match cached_object(
            &mut cache,
            &catalog,
            instance.object_id(),
            source_mesh,
            &mut meshes,
            &mut materials,
        ) {
            Ok(cached) => {
                let children = spawn_chunks(&mut commands, entity, instance.object_id(), &cached);
                commands.entity(entity).insert(RenderedObject {
                    object_id: instance.object_id().clone(),
                    catalog_fingerprint,
                    source_generation: cache.source_generation,
                    children,
                    failed: false,
                });
            }
            Err(error) => {
                let already_reported = rendered.is_some_and(|rendered| {
                    rendered.failed
                        && rendered.object_id == *instance.object_id()
                        && rendered.catalog_fingerprint == catalog_fingerprint
                        && rendered.source_generation == cache.source_generation
                });
                if !already_reported {
                    error!(
                        "cannot render authored object '{}': {error}",
                        instance.object_id()
                    );
                }
                commands.entity(entity).insert(RenderedObject {
                    object_id: instance.object_id().clone(),
                    catalog_fingerprint,
                    source_generation: cache.source_generation,
                    children: Vec::new(),
                    failed: true,
                });
            }
        }
    }
}

fn despawn_render_children(commands: &mut Commands, rendered: &RenderedObject) {
    for child in &rendered.children {
        commands.entity(*child).despawn();
    }
}

fn spawn_chunks(
    commands: &mut Commands,
    root: Entity,
    object_id: &ObjectAssetId,
    cached: &CachedObject,
) -> Vec<Entity> {
    let mut children = Vec::with_capacity(cached.chunks.len());
    for chunk in &cached.chunks {
        let mut entity = commands.spawn((
            Mesh3d(chunk.mesh.clone()),
            MeshMaterial3d(chunk.material.clone()),
            Transform::IDENTITY,
            Pickable::IGNORE,
            ObjectRenderChunk {
                style: chunk.key.style.clone(),
                canopy: chunk.key.canopy,
            },
            Name::new(format!(
                "Object {object_id} / {}{}",
                chunk.key.style,
                if chunk.key.canopy { " / canopy" } else { "" }
            )),
        ));
        if chunk.key.canopy {
            entity.insert(ObjectCanopyChunk);
        }
        if !chunk.casts_shadows {
            entity.insert(NotShadowCaster);
        }
        if chunk.surface_mode == VoxelSurfaceMode::Translucent {
            entity.insert(ObjectTranslucentChunk);
        }
        children.push(entity.id());
    }
    commands.entity(root).add_children(&children);
    children
}

fn cached_object(
    cache: &mut ObjectRenderCache,
    catalog: &RuntimeArtCatalog,
    object_id: &ObjectAssetId,
    source_mesh: &Mesh,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) -> Result<CachedObject, String> {
    if let Some(cached) = cache.objects.get(object_id) {
        return Ok(cached.clone());
    }

    let blueprint = catalog
        .object(object_id)
        .ok_or_else(|| format!("runtime art catalog has no object '{object_id}'"))?;
    let baked = bake_blueprint(source_mesh, blueprint)?;
    let mut chunks = Vec::with_capacity(baked.len());
    for (key, mesh) in baked {
        let style = catalog.style(&key.style).ok_or_else(|| {
            format!(
                "validated object '{}' references unresolved style '{}'",
                object_id, key.style
            )
        })?;
        let material = cached_material(cache, &key.style, style, materials);
        chunks.push(CachedChunk {
            key,
            mesh: meshes.add(mesh),
            material,
            surface_mode: style.authored().surface_mode(),
            casts_shadows: matches!(
                style.authored().surface_mode(),
                VoxelSurfaceMode::Opaque | VoxelSurfaceMode::Cutout
            ),
        });
    }
    let cached = CachedObject { chunks };
    cache.objects.insert(object_id.clone(), cached.clone());
    Ok(cached)
}

fn manage_object_oit(
    mut commands: Commands,
    translucent_chunks: Query<(), With<ObjectTranslucentChunk>>,
    mut cameras: Query<
        (
            Entity,
            &mut Camera3d,
            &mut Msaa,
            Has<OrderIndependentTransparencySettings>,
            Option<&ObjectOitCamera>,
        ),
        With<Camera3d>,
    >,
) {
    let needs_oit = !translucent_chunks.is_empty();
    for (entity, mut camera_3d, mut msaa, has_oit, managed) in &mut cameras {
        if needs_oit {
            let texture_binding = TextureUsages::TEXTURE_BINDING.bits();
            if camera_3d.depth_texture_usages.0 & texture_binding == 0 {
                camera_3d.depth_texture_usages.0 |= texture_binding;
            }
            if managed.is_some() {
                if !has_oit {
                    commands
                        .entity(entity)
                        .insert(OrderIndependentTransparencySettings::default());
                }
                if *msaa != Msaa::Off {
                    *msaa = Msaa::Off;
                }
                continue;
            }
            commands.entity(entity).insert(ObjectOitCamera {
                previous_msaa: *msaa,
                preserve_oit: has_oit,
            });
            if !has_oit {
                commands
                    .entity(entity)
                    .insert(OrderIndependentTransparencySettings::default());
            }
            *msaa = Msaa::Off;
            continue;
        }

        let Some(managed) = managed else {
            continue;
        };
        let mut camera = commands.entity(entity);
        camera.insert(managed.previous_msaa);
        if !managed.preserve_oit {
            camera.remove::<OrderIndependentTransparencySettings>();
        }
        camera.remove::<ObjectOitCamera>();
    }
}

fn cached_material(
    cache: &mut ObjectRenderCache,
    id: &VoxelStyleId,
    style: &ResolvedVoxelStyle,
    materials: &mut Assets<StandardMaterial>,
) -> Handle<StandardMaterial> {
    if let Some(cached) = cache.materials.get(id) {
        return cached.handle.clone();
    }
    let handle = materials.add(material_for(style));
    cache.materials.insert(
        id.clone(),
        CachedMaterial {
            handle: handle.clone(),
        },
    );
    handle
}

fn material_for(style: &ResolvedVoxelStyle) -> StandardMaterial {
    let authored = style.authored();
    let alpha_mode = match authored.surface_mode() {
        VoxelSurfaceMode::Opaque => AlphaMode::Opaque,
        VoxelSurfaceMode::Cutout => AlphaMode::AlphaToCoverage,
        VoxelSurfaceMode::Translucent => AlphaMode::Blend,
        VoxelSurfaceMode::Additive => AlphaMode::Add,
    };
    let alpha = if authored.surface_mode() == VoxelSurfaceMode::Opaque {
        1.0
    } else {
        authored.opacity()
    };
    let emissive = style.emission_color().map_or(LinearRgba::BLACK, |color| {
        let linear = color.to_bevy_color().to_linear();
        let strength = authored
            .emission()
            .map_or(0.0, hex_assets::VoxelEmission::strength);
        LinearRgba::new(
            linear.red * strength,
            linear.green * strength,
            linear.blue * strength,
            1.0,
        )
    });
    let color = style.base_color();
    StandardMaterial {
        base_color: Color::srgba(color.red(), color.green(), color.blue(), alpha),
        emissive,
        alpha_mode,
        perceptual_roughness: 0.82,
        metallic: 0.0,
        ..default()
    }
}

fn bake_blueprint(
    source_mesh: &Mesh,
    blueprint: &ObjectBlueprint,
) -> Result<Vec<(ChunkKey, Mesh)>, String> {
    let canopy: BTreeSet<_> = blueprint.canopy_occluders.iter().copied().collect();
    let mut groups: BTreeMap<ChunkKey, Vec<LocalVoxelCoord>> = BTreeMap::new();
    for placement in &blueprint.placements {
        groups
            .entry(ChunkKey {
                style: placement.style.clone(),
                canopy: canopy.contains(&placement.position),
            })
            .or_default()
            .push(placement.position);
    }

    groups
        .into_iter()
        .map(|(key, mut cells)| {
            cells.sort_unstable();
            let occupied: BTreeSet<_> = cells.iter().copied().collect();
            let mesh = merge_cells(source_mesh, blueprint.origin, &cells, &occupied)?;
            Ok((key, mesh))
        })
        .collect()
}

fn merge_cells(
    source_mesh: &Mesh,
    origin: LocalVoxelCoord,
    cells: &[LocalVoxelCoord],
    occupied_with_style: &BTreeSet<LocalVoxelCoord>,
) -> Result<Mesh, String> {
    let mut cells = cells.iter();
    let first = cells
        .next()
        .ok_or_else(|| "cannot bake an empty object chunk".to_owned())?;
    let mut merged = transformed_cell(source_mesh, origin, *first, occupied_with_style)?;
    for cell in cells {
        let transformed = transformed_cell(source_mesh, origin, *cell, occupied_with_style)?;
        merged
            .merge(&transformed)
            .map_err(|error| format!("hex mesh chunks cannot be merged: {error}"))?;
    }
    Ok(merged)
}

fn transformed_cell(
    source_mesh: &Mesh,
    origin: LocalVoxelCoord,
    cell: LocalVoxelCoord,
    occupied_with_style: &BTreeSet<LocalVoxelCoord>,
) -> Result<Mesh, String> {
    let relative_q = cell
        .q
        .checked_sub(origin.q)
        .ok_or_else(|| "object-local q offset overflows i32".to_owned())?;
    let relative_r = cell
        .r
        .checked_sub(origin.r)
        .ok_or_else(|| "object-local r offset overflows i32".to_owned())?;
    let relative_level = cell
        .level
        .checked_sub(origin.level)
        .ok_or_else(|| "object-local level offset overflows i32".to_owned())?;
    #[expect(
        clippy::cast_precision_loss,
        reason = "authored object levels are bounded to 64 and exactly represented by f32"
    )]
    let translation = HexCoord::from_axial(relative_q, relative_r).to_world(relative_level as f32);
    cull_internal_faces(source_mesh, face_cull_mask(cell, occupied_with_style))?
        .try_transformed_by(Transform::from_translation(translation))
        .map_err(|error| format!("hex mesh cannot be transformed: {error}"))
}

const AXIAL_NEIGHBOURS: [(i32, i32); 6] = [(1, 0), (0, 1), (-1, 1), (-1, 0), (0, -1), (1, -1)];

fn face_cull_mask(
    cell: LocalVoxelCoord,
    occupied_with_style: &BTreeSet<LocalVoxelCoord>,
) -> FaceCullMask {
    let mut sides = [false; 6];
    for (culled, (delta_q, delta_r)) in sides.iter_mut().zip(AXIAL_NEIGHBOURS) {
        let neighbour = cell
            .q
            .checked_add(delta_q)
            .zip(cell.r.checked_add(delta_r))
            .map(|(q, r)| LocalVoxelCoord::new(q, r, cell.level));
        *culled = neighbour.is_some_and(|position| occupied_with_style.contains(&position));
    }
    let top = cell
        .level
        .checked_add(1)
        .map(|level| LocalVoxelCoord::new(cell.q, cell.r, level))
        .is_some_and(|position| occupied_with_style.contains(&position));
    let bottom = cell
        .level
        .checked_sub(1)
        .map(|level| LocalVoxelCoord::new(cell.q, cell.r, level))
        .is_some_and(|position| occupied_with_style.contains(&position));
    FaceCullMask { sides, top, bottom }
}

#[derive(Debug, Clone, Copy)]
struct SourcePrismPlanes {
    min_y: f32,
    max_y: f32,
    side_maxima: [f32; 6],
    epsilon: f32,
}

impl SourcePrismPlanes {
    fn measure(positions: &[[f32; 3]]) -> Result<Self, String> {
        if positions.is_empty() {
            return Err("hex mesh has no positions".to_owned());
        }
        let directions = side_directions();
        let mut min_y = f32::INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        let mut side_maxima = [f32::NEG_INFINITY; 6];
        let mut maximum_extent = 0.0_f32;
        for raw in positions {
            let position = Vec3::from(*raw);
            if !position.is_finite() {
                return Err("hex mesh contains a non-finite vertex position".to_owned());
            }
            min_y = min_y.min(position.y);
            max_y = max_y.max(position.y);
            maximum_extent = maximum_extent.max(position.abs().max_element());
            let horizontal = Vec2::new(position.x, position.z);
            for (maximum, direction) in side_maxima.iter_mut().zip(directions) {
                *maximum = maximum.max(horizontal.dot(direction));
            }
        }
        Ok(Self {
            min_y,
            max_y,
            side_maxima,
            epsilon: maximum_extent.max(1.0) * 1.0e-5,
        })
    }
}

fn side_directions() -> [Vec2; 6] {
    AXIAL_NEIGHBOURS.map(|(q, r)| {
        let world = HexCoord::from_axial(q, r).to_world(0.0);
        Vec2::new(world.x, world.z).normalize()
    })
}

fn cull_internal_faces(source_mesh: &Mesh, mask: FaceCullMask) -> Result<Mesh, String> {
    if mask.is_empty() {
        return Ok(source_mesh.clone());
    }
    if source_mesh.primitive_topology() != PrimitiveTopology::TriangleList {
        return Err("hex mesh must use TriangleList topology for internal-face culling".to_owned());
    }
    let positions = match source_mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
        Some(VertexAttributeValues::Float32x3(positions)) => positions,
        Some(_) => {
            return Err("hex mesh positions must use Float32x3 format".to_owned());
        }
        None => return Err("hex mesh has no position attribute".to_owned()),
    };
    let planes = SourcePrismPlanes::measure(positions)?;
    let source_indices: Vec<usize> = source_mesh.indices().map_or_else(
        || (0..source_mesh.count_vertices()).collect(),
        |indices| indices.iter().collect(),
    );
    let triangles = source_indices.chunks_exact(3);
    if !triangles.remainder().is_empty() {
        return Err("hex mesh triangle index count is not divisible by three".to_owned());
    }
    let mut retained = Vec::with_capacity(source_indices.len());
    for triangle in triangles {
        let [first, second, third] = triangle else {
            continue;
        };
        let indices = [*first, *second, *third];
        let vertices = [
            position_at(positions, *first)?,
            position_at(positions, *second)?,
            position_at(positions, *third)?,
        ];
        if triangle_is_culled(vertices, planes, mask) {
            continue;
        }
        for index in indices {
            retained.push(
                u32::try_from(index)
                    .map_err(|_error| "hex mesh vertex index exceeds u32".to_owned())?,
            );
        }
    }
    let mut culled = source_mesh.clone();
    culled.insert_indices(Indices::U32(retained));
    Ok(culled)
}

fn position_at(positions: &[[f32; 3]], index: usize) -> Result<Vec3, String> {
    positions
        .get(index)
        .copied()
        .map(Vec3::from)
        .ok_or_else(|| format!("hex mesh index {index} exceeds its position attribute"))
}

fn triangle_is_culled(vertices: [Vec3; 3], planes: SourcePrismPlanes, mask: FaceCullMask) -> bool {
    let near = |left: f32, right: f32| (left - right).abs() <= planes.epsilon;
    if mask.top && vertices.iter().all(|vertex| near(vertex.y, planes.max_y)) {
        return true;
    }
    if mask.bottom && vertices.iter().all(|vertex| near(vertex.y, planes.min_y)) {
        return true;
    }
    let directions = side_directions();
    mask.sides
        .into_iter()
        .zip(directions)
        .zip(planes.side_maxima)
        .any(|((culled, direction), maximum)| {
            culled
                && vertices
                    .iter()
                    .all(|vertex| near(Vec2::new(vertex.x, vertex.z).dot(direction), maximum))
        })
}

fn object_root_transform(instance: &ObjectInstance) -> Transform {
    let origin = instance.origin();
    #[expect(
        clippy::cast_precision_loss,
        reason = "playable map levels are small integers and exactly represented by f32"
    )]
    let y = (origin.level as f32 + 0.5) * instance.level_height();
    Transform {
        translation: origin.coord.to_world(y),
        rotation: rotation_quat(instance.rotation()),
        scale: Vec3::new(1.0, instance.level_height(), 1.0),
    }
}

fn rotation_quat(rotation: HexObjectRotation) -> Quat {
    Quat::from_rotation_y(-f32::from(rotation.steps()) * TAU / 6.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use bevy::asset::{AssetId, AssetPlugin};
    use bevy::mesh::VertexAttributeValues;
    use bevy::prelude::Cuboid;
    use hex_assets::{
        ArtPalette, EffectPart, ObjectBounds, ObjectCatalogFile, ObjectCategory, ObjectPart,
        ObjectPlacement, PaletteSwatch, PlantPart, SrgbColor, SwatchId, VoxelEmission, VoxelStyle,
        VoxelStyleCatalog,
    };
    use hex_core::TilePos;

    fn object_id(value: &str) -> ObjectAssetId {
        match ObjectAssetId::new(value) {
            Ok(id) => id,
            Err(error) => unreachable!("valid object id fixture failed: {error}"),
        }
    }

    fn style_id(value: &str) -> VoxelStyleId {
        match VoxelStyleId::new(value) {
            Ok(id) => id,
            Err(error) => unreachable!("valid style id fixture failed: {error}"),
        }
    }

    fn swatch_id(value: &str) -> SwatchId {
        match SwatchId::new(value) {
            Ok(id) => id,
            Err(error) => unreachable!("valid swatch id fixture failed: {error}"),
        }
    }

    fn color(red: f32, green: f32, blue: f32) -> SrgbColor {
        match SrgbColor::new(red, green, blue) {
            Ok(color) => color,
            Err(error) => unreachable!("valid colour fixture failed: {error}"),
        }
    }

    fn swatch(name: &str, color: SrgbColor) -> PaletteSwatch {
        match PaletteSwatch::new(name, color, BTreeSet::from(["renderer-test".to_owned()])) {
            Ok(swatch) => swatch,
            Err(error) => unreachable!("valid swatch fixture failed: {error}"),
        }
    }

    fn style(
        name: &str,
        swatch: &str,
        mode: VoxelSurfaceMode,
        opacity: f32,
        emission: Option<(&str, f32)>,
    ) -> VoxelStyle {
        let emission = emission.map(|(swatch, strength)| {
            match VoxelEmission::new(swatch_id(swatch), strength) {
                Ok(emission) => emission,
                Err(error) => unreachable!("valid emission fixture failed: {error}"),
            }
        });
        match VoxelStyle::new(name, swatch_id(swatch), mode, opacity, emission) {
            Ok(style) => style,
            Err(error) => unreachable!("valid style fixture failed: {error}"),
        }
    }

    fn placement(position: LocalVoxelCoord, style: &str, part: PlantPart) -> ObjectPlacement {
        ObjectPlacement {
            position,
            style: style_id(style),
            part: ObjectPart::Plant(part),
        }
    }

    fn fixture_blueprint() -> ObjectBlueprint {
        let origin = LocalVoxelCoord::new(-2, 1, 0);
        ObjectBlueprint {
            schema_version: hex_assets::OBJECT_BLUEPRINT_SCHEMA_VERSION,
            id: object_id("plant/test"),
            display_name: "Renderer Test".to_owned(),
            category: ObjectCategory::Plant,
            bounds: ObjectBounds {
                radius: 3,
                min_level: 0,
                height: 8,
            },
            connectivity: hex_assets::ConnectivityPolicy::Grounded,
            origin,
            placements: vec![
                placement(origin, "plant/trunk", PlantPart::Root),
                placement(
                    LocalVoxelCoord::new(-2, 1, 1),
                    "plant/trunk",
                    PlantPart::Trunk,
                ),
                placement(
                    LocalVoxelCoord::new(-1, 1, 1),
                    "plant/leaf",
                    PlantPart::Foliage,
                ),
                placement(
                    LocalVoxelCoord::new(0, 1, 1),
                    "plant/leaf",
                    PlantPart::Foliage,
                ),
            ],
            blocker_footprint: vec![origin.axial()],
            canopy_occluders: vec![LocalVoxelCoord::new(-1, 1, 1)],
        }
    }

    fn material_fixture_blueprint() -> ObjectBlueprint {
        let origin = LocalVoxelCoord::new(-1, 1, -2);
        ObjectBlueprint {
            schema_version: hex_assets::OBJECT_BLUEPRINT_SCHEMA_VERSION,
            id: object_id("effect/material-test"),
            display_name: "Material Test".to_owned(),
            category: ObjectCategory::Effect,
            bounds: ObjectBounds {
                radius: 2,
                min_level: -2,
                height: 5,
            },
            connectivity: hex_assets::ConnectivityPolicy::Free,
            origin,
            placements: vec![
                ObjectPlacement {
                    position: origin,
                    style: style_id("test/opaque"),
                    part: ObjectPart::Effect(EffectPart::Core),
                },
                ObjectPlacement {
                    position: LocalVoxelCoord::new(0, 0, -1),
                    style: style_id("test/cutout"),
                    part: ObjectPart::Effect(EffectPart::Trail),
                },
                ObjectPlacement {
                    position: LocalVoxelCoord::new(1, -1, 0),
                    style: style_id("test/translucent"),
                    part: ObjectPart::Effect(EffectPart::Accent),
                },
                ObjectPlacement {
                    position: LocalVoxelCoord::new(2, -1, 0),
                    style: style_id("test/translucent"),
                    part: ObjectPart::Effect(EffectPart::Accent),
                },
                ObjectPlacement {
                    position: LocalVoxelCoord::new(0, 1, 1),
                    style: style_id("test/additive"),
                    part: ObjectPart::Effect(EffectPart::Accent),
                },
            ],
            blocker_footprint: Vec::new(),
            canopy_occluders: Vec::new(),
        }
    }

    fn fixture_catalog(leaf_red: f32) -> RuntimeArtCatalog {
        let palette = match ArtPalette::new(BTreeMap::from([
            (
                swatch_id("test/bark"),
                swatch("Test Bark", color(0.28, 0.16, 0.08)),
            ),
            (
                swatch_id("test/leaf"),
                swatch("Test Leaf", color(leaf_red, 0.48, 0.16)),
            ),
            (
                swatch_id("test/glow"),
                swatch("Test Glow", color(0.25, 0.60, 1.0)),
            ),
        ])) {
            Ok(palette) => palette,
            Err(error) => unreachable!("valid palette fixture failed: {error}"),
        };
        let styles = match VoxelStyleCatalog::new(BTreeMap::from([
            (
                style_id("plant/leaf"),
                style(
                    "Plant Leaf",
                    "test/leaf",
                    VoxelSurfaceMode::Cutout,
                    0.72,
                    None,
                ),
            ),
            (
                style_id("plant/trunk"),
                style(
                    "Plant Trunk",
                    "test/bark",
                    VoxelSurfaceMode::Opaque,
                    1.0,
                    None,
                ),
            ),
            (
                style_id("test/additive"),
                style(
                    "Additive",
                    "test/glow",
                    VoxelSurfaceMode::Additive,
                    0.8,
                    Some(("test/glow", 2.0)),
                ),
            ),
            (
                style_id("test/cutout"),
                style("Cutout", "test/leaf", VoxelSurfaceMode::Cutout, 0.65, None),
            ),
            (
                style_id("test/opaque"),
                style("Opaque", "test/bark", VoxelSurfaceMode::Opaque, 1.0, None),
            ),
            (
                style_id("test/translucent"),
                style(
                    "Translucent",
                    "test/leaf",
                    VoxelSurfaceMode::Translucent,
                    0.45,
                    None,
                ),
            ),
        ])) {
            Ok(styles) => styles,
            Err(error) => unreachable!("valid style fixture failed: {error}"),
        };
        let plant = fixture_blueprint();
        let effect = material_fixture_blueprint();
        assert_eq!(plant.validate(&styles), Ok(()));
        assert_eq!(effect.validate(&styles), Ok(()));
        let manifest = match ObjectCatalogFile::new([plant.id.clone(), effect.id.clone()]) {
            Ok(manifest) => manifest,
            Err(error) => unreachable!("valid manifest fixture failed: {error}"),
        };
        let objects = BTreeMap::from([(plant.id.clone(), plant), (effect.id.clone(), effect)]);
        match RuntimeArtCatalog::from_sources(&palette, &styles, &manifest, objects) {
            Ok(catalog) => catalog,
            Err(error) => unreachable!("valid runtime catalog fixture failed: {error}"),
        }
    }

    fn rotation(steps: u8) -> HexObjectRotation {
        match HexObjectRotation::new(steps) {
            Ok(rotation) => rotation,
            Err(error) => unreachable!("valid rotation fixture failed: {error}"),
        }
    }

    fn instance(
        id: &str,
        coord: HexCoord,
        level: i32,
        level_height: f32,
        steps: u8,
    ) -> ObjectInstance {
        match ObjectInstance::new(
            object_id(id),
            TilePos::new(coord, level),
            level_height,
            rotation(steps),
        ) {
            Ok(instance) => instance,
            Err(error) => unreachable!("valid object instance fixture failed: {error}"),
        }
    }

    fn test_app(catalog: RuntimeArtCatalog) -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()));
        app.init_asset::<Mesh>();
        app.init_asset::<StandardMaterial>();
        let source = app
            .world_mut()
            .resource_mut::<Assets<Mesh>>()
            .add(Mesh::from(Cuboid::new(1.0, 1.0, 1.0)));
        app.insert_resource(GameAssets {
            hex_tile: source,
            player_pieces: [Handle::default(), Handle::default()],
        });
        app.insert_resource(catalog);
        plugin(&mut app);
        app
    }

    fn settle(app: &mut App) {
        app.update();
        app.update();
    }

    fn child_entities(app: &App, root: Entity) -> Vec<Entity> {
        app.world()
            .get::<Children>(root)
            .map(|children| children.iter().collect())
            .unwrap_or_default()
    }

    fn chunk_handles(
        app: &App,
        root: Entity,
    ) -> BTreeMap<(VoxelStyleId, bool), (AssetId<Mesh>, AssetId<StandardMaterial>, Entity)> {
        child_entities(app, root)
            .into_iter()
            .filter_map(|entity| {
                let chunk = app.world().get::<ObjectRenderChunk>(entity)?;
                let mesh = app.world().get::<Mesh3d>(entity)?;
                let material = app
                    .world()
                    .get::<MeshMaterial3d<StandardMaterial>>(entity)?;
                Some((
                    (chunk.style().clone(), chunk.is_canopy()),
                    (mesh.id(), material.id(), entity),
                ))
            })
            .collect()
    }

    fn mesh_positions(mesh: &Mesh) -> &[[f32; 3]] {
        let Some(VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            unreachable!("cuboid fixture must expose Float32x3 positions")
        };
        positions
    }

    fn position_centroid(mesh: &Mesh) -> Vec3 {
        let positions = mesh_positions(mesh);
        let sum = positions
            .iter()
            .map(|position| Vec3::from(*position))
            .sum::<Vec3>();
        #[expect(
            clippy::cast_precision_loss,
            reason = "the tiny test mesh vertex count is exactly representable by f32"
        )]
        let count = positions.len() as f32;
        sum / count
    }

    fn mesh_index_count(mesh: &Mesh) -> usize {
        mesh.indices()
            .map_or_else(|| mesh.count_vertices(), Indices::len)
    }

    #[test]
    fn six_rotations_return_to_the_original_orientation() {
        let mut combined = Quat::IDENTITY;
        for _ in 0..6 {
            combined = rotation_quat(rotation(1)) * combined;
        }
        assert!((combined * Vec3::X).abs_diff_eq(Vec3::X, 1.0e-5));
        assert!((combined * Vec3::Y).abs_diff_eq(Vec3::Y, 1.0e-5));
        assert!((combined * Vec3::Z).abs_diff_eq(Vec3::Z, 1.0e-5));
    }

    #[test]
    fn bevy_yaw_matches_all_six_authored_axial_rotations() {
        let origin = LocalVoxelCoord::new(-2, 1, 0);
        let cell = LocalVoxelCoord::new(-1, 1, 2);
        let unrotated = HexCoord::from_axial(cell.q - origin.q, cell.r - origin.r).to_world(2.0);
        for steps in 0..6 {
            let rotation = rotation(steps);
            let rotated = match rotation.rotate_voxel(cell, origin) {
                Some(rotated) => rotated,
                None => unreachable!("bounded fixture rotation cannot overflow"),
            };
            let expected =
                HexCoord::from_axial(rotated.q - origin.q, rotated.r - origin.r).to_world(2.0);
            let actual = rotation_quat(rotation) * unrotated;
            assert!(
                actual.abs_diff_eq(expected, 1.0e-5),
                "rotation {steps}: {actual:?} != {expected:?}"
            );
        }
    }

    #[test]
    fn blueprint_bakes_one_chunk_per_style_and_canopy_partition() {
        let source = Mesh::from(Cuboid::new(1.0, 1.0, 1.0));
        let blueprint = fixture_blueprint();
        assert_eq!(blueprint.validate_intrinsic(), Ok(()));
        let baked = match bake_blueprint(&source, &blueprint) {
            Ok(baked) => baked,
            Err(error) => unreachable!("valid blueprint should bake: {error}"),
        };

        assert_eq!(baked.len(), 3);
        let keys: Vec<_> = baked.iter().map(|(key, _)| key.clone()).collect();
        assert_eq!(
            keys,
            vec![
                ChunkKey {
                    style: style_id("plant/leaf"),
                    canopy: false,
                },
                ChunkKey {
                    style: style_id("plant/leaf"),
                    canopy: true,
                },
                ChunkKey {
                    style: style_id("plant/trunk"),
                    canopy: false,
                },
            ]
        );
    }

    #[test]
    fn independently_hideable_canopy_boundary_keeps_both_faces_closed() {
        let source = Mesh::from(Cuboid::new(1.0, 1.0, 1.0));
        let source_indices = mesh_index_count(&source);
        let blueprint = fixture_blueprint();
        let baked = match bake_blueprint(&source, &blueprint) {
            Ok(baked) => baked,
            Err(error) => unreachable!("valid canopy fixture should bake: {error}"),
        };
        let leaf_chunks: Vec<_> = baked
            .iter()
            .filter(|(key, _)| key.style == style_id("plant/leaf"))
            .collect();

        assert_eq!(leaf_chunks.len(), 2);
        assert!(leaf_chunks
            .iter()
            .any(|(key, mesh)| key.canopy && mesh_index_count(mesh) == source_indices));
        assert!(leaf_chunks
            .iter()
            .any(|(key, mesh)| !key.canopy && mesh_index_count(mesh) == source_indices));
    }

    #[test]
    fn adjacent_translucent_cells_share_no_closed_internal_prism_faces() {
        let source = Mesh::from(Cuboid::new(1.0, 1.0, 1.0));
        let blueprint = material_fixture_blueprint();
        let baked = match bake_blueprint(&source, &blueprint) {
            Ok(baked) => baked,
            Err(error) => unreachable!("valid material fixture should bake: {error}"),
        };
        let translucent_key = ChunkKey {
            style: style_id("test/translucent"),
            canopy: false,
        };
        let Some((_, translucent)) = baked.iter().find(|(key, _)| *key == translucent_key) else {
            unreachable!("material fixture must contain one translucent chunk")
        };
        let closed_prisms = mesh_index_count(&source) * 2;

        assert_eq!(mesh_index_count(translucent), closed_prisms - 12);
    }

    #[test]
    fn merged_chunk_vertex_count_scales_with_cells_not_entities() {
        let source = Mesh::from(Cuboid::new(1.0, 1.0, 1.0));
        let source_vertices = source.count_vertices();
        let origin = LocalVoxelCoord::new(0, 0, 0);
        let cells = [
            origin,
            LocalVoxelCoord::new(1, 0, 0),
            LocalVoxelCoord::new(1, -1, 2),
        ];
        let occupied = BTreeSet::from(cells);
        let merged = match merge_cells(&source, origin, &cells, &occupied) {
            Ok(mesh) => mesh,
            Err(error) => unreachable!("compatible meshes should merge: {error}"),
        };

        assert_eq!(merged.count_vertices(), source_vertices * cells.len());
    }

    #[test]
    fn signed_nonzero_origin_is_subtracted_before_baking() {
        let source = Mesh::from(Cuboid::new(1.0, 1.0, 1.0));
        let origin = LocalVoxelCoord::new(-2, 1, -3);
        let cell = LocalVoxelCoord::new(-1, 0, -1);
        let occupied = BTreeSet::from([cell]);
        let merged = match merge_cells(&source, origin, &[cell], &occupied) {
            Ok(mesh) => mesh,
            Err(error) => unreachable!("compatible mesh should transform: {error}"),
        };
        let expected = HexCoord::from_axial(1, -1).to_world(2.0);

        assert!(position_centroid(&merged).abs_diff_eq(expected, 1.0e-5));
    }

    #[test]
    fn empty_chunk_is_rejected_without_panicking() {
        let source = Mesh::from(Cuboid::new(1.0, 1.0, 1.0));
        let result = merge_cells(
            &source,
            LocalVoxelCoord::new(0, 0, 0),
            &[],
            &BTreeSet::new(),
        );

        assert_eq!(
            result.err().as_deref(),
            Some("cannot bake an empty object chunk")
        );
    }

    #[test]
    fn root_transform_places_the_exact_signed_origin_voxel() {
        let mut app = test_app(fixture_catalog(0.18));
        let origin = HexCoord::from_axial(-3, 2);
        let instance = instance("plant/test", origin, -4, 0.625, 5);
        let expected = object_root_transform(&instance);
        let root = app.world_mut().spawn(instance).id();
        settle(&mut app);

        let Some(actual) = app.world().get::<Transform>(root) else {
            unreachable!("renderer must add a root transform")
        };
        assert!(actual.translation.abs_diff_eq(expected.translation, 1.0e-5));
        assert!(actual.rotation.abs_diff_eq(expected.rotation, 1.0e-5));
        assert!(actual.scale.abs_diff_eq(Vec3::new(1.0, 0.625, 1.0), 1.0e-6));
        assert!(actual.translation.y < 0.0);
    }

    #[test]
    fn instances_share_cached_meshes_and_materials_by_visual_chunk() {
        let mut app = test_app(fixture_catalog(0.18));
        let first = app
            .world_mut()
            .spawn(instance("plant/test", HexCoord::ORIGIN, 3, 0.4, 0))
            .id();
        let second = app
            .world_mut()
            .spawn(instance(
                "plant/test",
                HexCoord::from_axial(4, -2),
                7,
                0.4,
                4,
            ))
            .id();
        settle(&mut app);

        let first_handles = chunk_handles(&app, first);
        let second_handles = chunk_handles(&app, second);
        assert_eq!(first_handles.len(), 3);
        assert_eq!(second_handles.len(), 3);
        assert_eq!(
            first_handles
                .iter()
                .map(|(key, (mesh, material, _))| (key, mesh, material))
                .collect::<Vec<_>>(),
            second_handles
                .iter()
                .map(|(key, (mesh, material, _))| (key, mesh, material))
                .collect::<Vec<_>>()
        );
        assert_eq!(app.world().resource::<Assets<Mesh>>().len(), 4);
        assert_eq!(app.world().resource::<Assets<StandardMaterial>>().len(), 2);
    }

    #[test]
    fn one_hundred_idle_frames_preserve_object_entities_assets_and_cache() {
        let mut app = test_app(fixture_catalog(0.18));
        let root = app
            .world_mut()
            .spawn(instance("plant/test", HexCoord::ORIGIN, 3, 0.4, 0))
            .id();
        settle(&mut app);
        let chunks_before = chunk_handles(&app, root);
        let meshes_before = app.world().resource::<Assets<Mesh>>().len();
        let materials_before = app.world().resource::<Assets<StandardMaterial>>().len();
        app.world_mut().clear_trackers();

        for _ in 0..100 {
            app.update();
        }

        assert_eq!(chunk_handles(&app, root), chunks_before);
        assert_eq!(app.world().resource::<Assets<Mesh>>().len(), meshes_before);
        assert_eq!(
            app.world().resource::<Assets<StandardMaterial>>().len(),
            materials_before
        );
        assert!(
            !app.world().resource_ref::<ObjectRenderCache>().is_changed(),
            "idle reconciliation marked the unchanged object cache dirty"
        );
    }

    #[test]
    fn chunk_materials_shadow_policy_picking_and_canopy_are_exact() {
        let mut app = test_app(fixture_catalog(0.18));
        let plant = app
            .world_mut()
            .spawn(instance("plant/test", HexCoord::ORIGIN, 0, 0.4, 0))
            .id();
        let effect = app
            .world_mut()
            .spawn(instance(
                "effect/material-test",
                HexCoord::from_axial(3, 0),
                2,
                0.4,
                0,
            ))
            .id();
        settle(&mut app);

        let plant_handles = chunk_handles(&app, plant);
        let canopy_count = plant_handles
            .values()
            .filter(|(_, _, entity)| app.world().get::<ObjectCanopyChunk>(*entity).is_some())
            .count();
        assert_eq!(canopy_count, 1);

        let expectations = [
            ("test/opaque", AlphaMode::Opaque, false),
            ("test/cutout", AlphaMode::AlphaToCoverage, false),
            ("test/translucent", AlphaMode::Blend, true),
            ("test/additive", AlphaMode::Add, true),
        ];
        let handles = chunk_handles(&app, effect);
        assert_eq!(handles.len(), expectations.len());
        for (style, alpha_mode, no_shadow) in expectations {
            let Some((_, material_id, entity)) = handles.get(&(style_id(style), false)).copied()
            else {
                unreachable!("material fixture should render style {style}")
            };
            let Some(material) = app
                .world()
                .resource::<Assets<StandardMaterial>>()
                .get(material_id)
            else {
                unreachable!("rendered material must remain in the asset store")
            };
            assert_eq!(material.alpha_mode, alpha_mode);
            assert_eq!(
                app.world().get::<NotShadowCaster>(entity).is_some(),
                no_shadow
            );
            assert_eq!(app.world().get::<Pickable>(entity), Some(&Pickable::IGNORE));
        }
    }

    #[test]
    fn removing_an_instance_despawns_only_its_managed_children() {
        let mut app = test_app(fixture_catalog(0.18));
        let root = app
            .world_mut()
            .spawn((
                instance("plant/test", HexCoord::ORIGIN, 0, 0.4, 0),
                Name::new("producer-owned-root"),
            ))
            .id();
        settle(&mut app);
        let children = child_entities(&app, root);
        assert_eq!(children.len(), 3);

        assert!(app.world().get::<ObjectInstance>(root).is_some());
        app.world_mut().entity_mut(root).remove::<ObjectInstance>();
        app.update();

        assert!(app.world().get::<RenderedObject>(root).is_none());
        assert!(app.world().get::<Name>(root).is_some());
        assert!(child_entities(&app, root).is_empty());
        assert!(children
            .iter()
            .all(|child| app.world().get_entity(*child).is_err()));
    }

    #[test]
    fn transform_only_instance_change_preserves_cached_children() {
        let mut app = test_app(fixture_catalog(0.18));
        let root = app
            .world_mut()
            .spawn(instance("plant/test", HexCoord::ORIGIN, 0, 0.4, 0))
            .id();
        settle(&mut app);
        let before = chunk_handles(&app, root);

        let replacement = instance("plant/test", HexCoord::from_axial(-4, 3), -2, 0.65, 3);
        let expected = object_root_transform(&replacement);
        app.world_mut().entity_mut(root).insert(replacement);
        app.update();

        assert_eq!(chunk_handles(&app, root), before);
        let Some(actual) = app.world().get::<Transform>(root) else {
            unreachable!("rendered object root must keep its transform")
        };
        assert!(actual.translation.abs_diff_eq(expected.translation, 1.0e-5));
        assert!(actual.rotation.abs_diff_eq(expected.rotation, 1.0e-5));
        assert!(actual.scale.abs_diff_eq(expected.scale, 1.0e-6));
    }

    #[test]
    fn missing_root_visibility_is_repaired_without_rebuilding_chunks() {
        let mut app = test_app(fixture_catalog(0.18));
        let root = app
            .world_mut()
            .spawn(instance("plant/test", HexCoord::ORIGIN, 0, 0.4, 0))
            .id();
        settle(&mut app);
        let before = chunk_handles(&app, root);
        assert!(app.world().get::<Visibility>(root).is_some());

        app.world_mut().entity_mut(root).remove::<Visibility>();
        app.update();

        assert_eq!(
            app.world().get::<Visibility>(root),
            Some(&Visibility::Inherited)
        );
        assert_eq!(chunk_handles(&app, root), before);
    }

    #[test]
    fn accepted_catalog_change_rebuilds_chunks_and_retires_old_assets() {
        let mut app = test_app(fixture_catalog(0.18));
        let root = app
            .world_mut()
            .spawn(instance("plant/test", HexCoord::ORIGIN, 0, 0.4, 0))
            .id();
        settle(&mut app);
        let before = chunk_handles(&app, root);
        let before_entities: Vec<_> = before.values().map(|(_, _, entity)| *entity).collect();

        app.insert_resource(fixture_catalog(0.24));
        app.update();

        let after = chunk_handles(&app, root);
        assert_eq!(before.len(), after.len());
        for (key, (old_mesh, old_material, old_entity)) in before {
            let Some((new_mesh, new_material, new_entity)) = after.get(&key) else {
                unreachable!("catalog rebuild must preserve chunk key {key:?}")
            };
            assert_ne!(&old_mesh, new_mesh);
            assert_ne!(&old_material, new_material);
            assert_ne!(&old_entity, new_entity);
            assert!(app.world().get_entity(old_entity).is_err());
        }
        assert!(before_entities
            .iter()
            .all(|entity| app.world().get_entity(*entity).is_err()));
        assert_eq!(app.world().resource::<Assets<Mesh>>().len(), 4);
        assert_eq!(app.world().resource::<Assets<StandardMaterial>>().len(), 2);
    }

    #[test]
    fn source_hex_mesh_hot_reload_rebakes_cached_objects() {
        let mut app = test_app(fixture_catalog(0.18));
        let root = app
            .world_mut()
            .spawn(instance("plant/test", HexCoord::ORIGIN, 0, 0.4, 0))
            .id();
        settle(&mut app);
        let before = chunk_handles(&app, root);
        let source_id = app.world().resource::<GameAssets>().hex_tile.id();
        let replacement = Mesh::from(Cuboid::new(1.25, 1.0, 1.25));
        let result = app
            .world_mut()
            .resource_mut::<Assets<Mesh>>()
            .insert(source_id, replacement);
        assert_eq!(result, Ok(()));

        app.update();
        app.update();

        let after = chunk_handles(&app, root);
        assert_eq!(before.len(), after.len());
        for (key, (old_mesh, old_material, old_entity)) in before {
            let Some((new_mesh, new_material, new_entity)) = after.get(&key) else {
                unreachable!("source hot reload must preserve chunk key {key:?}")
            };
            assert_ne!(&old_mesh, new_mesh);
            assert_eq!(&old_material, new_material);
            assert_ne!(&old_entity, new_entity);
            assert!(app.world().get_entity(old_entity).is_err());
        }
    }

    #[test]
    fn every_source_event_is_drained_into_one_global_rebuild() {
        let mut app = test_app(fixture_catalog(0.18));
        let root = app
            .world_mut()
            .spawn(instance("plant/test", HexCoord::ORIGIN, 0, 0.4, 0))
            .id();
        settle(&mut app);
        let before_generation = app
            .world()
            .resource::<ObjectRenderCache>()
            .source_generation;
        let before = chunk_handles(&app, root);
        let source_id = app.world().resource::<GameAssets>().hex_tile.id();
        {
            let mut events = app.world_mut().resource_mut::<Messages<AssetEvent<Mesh>>>();
            events.write(AssetEvent::Added { id: source_id });
            events.write(AssetEvent::LoadedWithDependencies { id: source_id });
        }

        app.update();

        let after_first = chunk_handles(&app, root);
        assert_ne!(after_first, before);
        assert_eq!(
            app.world()
                .resource::<ObjectRenderCache>()
                .source_generation,
            before_generation.wrapping_add(1)
        );

        app.update();

        assert_eq!(chunk_handles(&app, root), after_first);
        assert_eq!(
            app.world()
                .resource::<ObjectRenderCache>()
                .source_generation,
            before_generation.wrapping_add(1)
        );
    }

    #[test]
    fn oit_is_live_only_with_blend_chunks_and_restores_camera_msaa() {
        let mut app = test_app(fixture_catalog(0.18));
        let camera = app
            .world_mut()
            .spawn((Camera3d::default(), Msaa::Sample8))
            .id();
        let plant = app
            .world_mut()
            .spawn(instance("plant/test", HexCoord::ORIGIN, 0, 0.4, 0))
            .id();
        settle(&mut app);

        assert_eq!(app.world().get::<Msaa>(camera), Some(&Msaa::Sample8));
        assert!(app
            .world()
            .get::<OrderIndependentTransparencySettings>(camera)
            .is_none());
        assert!(app.world().get::<ObjectOitCamera>(camera).is_none());
        assert_eq!(child_entities(&app, plant).len(), 3);

        let effect = app
            .world_mut()
            .spawn(instance(
                "effect/material-test",
                HexCoord::from_axial(3, -1),
                2,
                0.4,
                0,
            ))
            .id();
        app.update();

        assert_eq!(app.world().get::<Msaa>(camera), Some(&Msaa::Off));
        assert!(app
            .world()
            .get::<OrderIndependentTransparencySettings>(camera)
            .is_some());
        assert!(app.world().get::<ObjectOitCamera>(camera).is_some());
        let camera_3d = app
            .world()
            .get::<Camera3d>(camera)
            .expect("camera must keep its Camera3d component");
        assert!(TextureUsages::from(camera_3d.depth_texture_usages)
            .contains(TextureUsages::TEXTURE_BINDING));
        assert_eq!(
            child_entities(&app, effect)
                .into_iter()
                .filter(|entity| { app.world().get::<ObjectTranslucentChunk>(*entity).is_some() })
                .count(),
            1
        );

        app.world_mut()
            .entity_mut(effect)
            .remove::<ObjectInstance>();
        app.update();

        assert_eq!(app.world().get::<Msaa>(camera), Some(&Msaa::Sample8));
        assert!(app
            .world()
            .get::<OrderIndependentTransparencySettings>(camera)
            .is_none());
        assert!(app.world().get::<ObjectOitCamera>(camera).is_none());
        let camera_3d = app
            .world()
            .get::<Camera3d>(camera)
            .expect("camera must keep its Camera3d component");
        assert!(
            TextureUsages::from(camera_3d.depth_texture_usages)
                .contains(TextureUsages::TEXTURE_BINDING),
            "the harmless sampling usage remains because another renderer may need it"
        );
    }

    #[test]
    fn a_camera_spawned_while_blend_is_live_gets_a_complete_oit_configuration() {
        let mut app = test_app(fixture_catalog(0.18));
        app.world_mut().spawn(instance(
            "effect/material-test",
            HexCoord::ORIGIN,
            0,
            0.4,
            0,
        ));
        settle(&mut app);

        let camera = app
            .world_mut()
            .spawn((Camera3d::default(), Msaa::Sample4))
            .id();
        app.update();

        assert_eq!(app.world().get::<Msaa>(camera), Some(&Msaa::Off));
        assert!(app
            .world()
            .get::<OrderIndependentTransparencySettings>(camera)
            .is_some());
        assert!(app.world().get::<ObjectOitCamera>(camera).is_some());
        let camera_3d = app
            .world()
            .get::<Camera3d>(camera)
            .expect("camera must keep its Camera3d component");
        assert!(TextureUsages::from(camera_3d.depth_texture_usages)
            .contains(TextureUsages::TEXTURE_BINDING));
    }

    #[test]
    fn renderer_preserves_camera_owned_oit_after_blend_chunks_leave() {
        let mut app = test_app(fixture_catalog(0.18));
        let camera = app
            .world_mut()
            .spawn((
                Camera3d::default(),
                Msaa::Off,
                OrderIndependentTransparencySettings {
                    sorted_fragment_max_count: 5,
                    ..default()
                },
            ))
            .id();
        let effect = app
            .world_mut()
            .spawn(instance(
                "effect/material-test",
                HexCoord::ORIGIN,
                0,
                0.4,
                0,
            ))
            .id();
        settle(&mut app);
        assert_eq!(app.world().get::<Msaa>(camera), Some(&Msaa::Off));

        app.world_mut()
            .entity_mut(effect)
            .remove::<ObjectInstance>();
        app.update();

        assert_eq!(app.world().get::<Msaa>(camera), Some(&Msaa::Off));
        let Some(settings) = app
            .world()
            .get::<OrderIndependentTransparencySettings>(camera)
        else {
            unreachable!("camera-owned OIT settings must survive renderer cleanup")
        };
        assert_eq!(settings.sorted_fragment_max_count, 5);
    }
}
