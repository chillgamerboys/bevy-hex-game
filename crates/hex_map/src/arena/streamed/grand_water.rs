//! Exact immutable inland-water faces, separate from the ocean bed-height proxy.
//! This presentation borrows sparse edit provenance and never requests residency.
use super::StreamedArena;
use crate::v4::river::{self, RiverMaterial, Style};
use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
};
use hex_schematic::v4::northern::{
    InlandWaterChunk, InlandWaterColumn, InlandWaterOverview, NorthernOverview,
    MAX_INLAND_WATER_COLUMNS, MAX_INLAND_WATER_SIDE_INTERVALS,
};
use hex_world_contracts::{ChunkId, LiquidKind, WorldHex};
use hex_world_runtime::FiniteWorldSession;
use std::collections::{BTreeMap, BTreeSet};

const MAX_VERTICES: usize = 2_000_000;
pub(super) const DIRECTIONS: [(i64, i64); 6] = [(1, 0), (0, 1), (-1, 1), (-1, 0), (0, -1), (1, -1)];

pub(super) fn validate(map: &NorthernOverview) -> Result<(), String> {
    let Some(source) = &map.inland_water else {
        return Ok(());
    };
    let invalid = || "Invalid bounded exact inland-water companion".to_owned();
    if map.world_id != "grand-v4"
        || source.version != 1
        || source.chunks.len() > MAX_INLAND_WATER_COLUMNS
    {
        return Err(invalid());
    }
    let mut previous_chunk = None;
    let mut count = 0_usize;
    let mut vertices = 0_usize;
    let [minimum, maximum] = map.level_bounds;
    for chunk in &source.chunks {
        if previous_chunk.is_some_and(|c| c >= chunk.coordinate)
            || chunk.columns.is_empty()
            || chunk.columns.len() > 256
        {
            return Err(invalid());
        }
        previous_chunk = Some(chunk.coordinate);
        let mut previous_column = None;
        for column in &chunk.columns {
            let at = column.column;
            if previous_column.is_some_and(|c| c >= at)
                || at.chunk() != chunk.coordinate
                || !at
                    .checked_distance(WorldHex::new(0, 0))
                    .is_ok_and(|d| d <= u64::from(map.radius))
                || column.bottom < minimum
                || column.top > maximum.saturating_add(1)
                || column.bottom >= column.top
                || level(column.top, map.level_height) <= map.sea_level
            {
                return Err(invalid());
            }
            previous_column = Some(at);
            match (column.kind, column.downstream) {
                (LiquidKind::Standing, None) => {}
                (LiquidKind::Directed | LiquidKind::Waterfall, Some(next))
                    if next.level >= minimum
                        && next.level < column.top
                        && at.checked_distance(next.column).is_ok_and(|d| d == 1)
                        && next
                            .column
                            .checked_distance(WorldHex::new(0, 0))
                            .is_ok_and(|d| d <= u64::from(map.radius)) => {}
                _ => return Err(invalid()),
            }
            vertices +=
                usize::from(column.exposed_top) * 7 + usize::from(column.exposed_bottom) * 7;
            for side in &column.sides {
                if side.len() > MAX_INLAND_WATER_SIDE_INTERVALS {
                    return Err(invalid());
                }
                let mut previous_top = None;
                for &[bottom, top] in side {
                    if bottom < column.bottom
                        || top > column.top
                        || bottom >= top
                        || previous_top.is_some_and(|old| old >= bottom)
                    {
                        return Err(invalid());
                    }
                    previous_top = Some(top);
                    vertices += 4;
                }
            }
            count += 1;
            if count > MAX_INLAND_WATER_COLUMNS || vertices > MAX_VERTICES {
                return Err(invalid());
            }
        }
    }
    let terrain_vertices = super::grand_inland_terrain::validate(map)?;
    let total_vertices = terrain_vertices + vertices;
    if total_vertices > 8_000_000 {
        return Err(invalid());
    }
    info!(
        liquid_columns = count,
        liquid_vertices = vertices,
        terrain_vertices,
        total_vertices,
        "Grand exact inland presentation admitted"
    );
    if total_vertices > 2_000_000 {
        warn!(
            total_vertices,
            "Grand inland presentation exceeds the two-million-vertex review target"
        );
    }
    Ok(())
}

pub(super) fn vertex_count(column: &InlandWaterColumn) -> usize {
    usize::from(column.exposed_top) * 7
        + usize::from(column.exposed_bottom) * 7
        + column
            .sides
            .iter()
            .map(|side| side.len() * 4)
            .sum::<usize>()
}
fn terrain_edited(chunk: &InlandWaterChunk, edits: &FiniteWorldSession) -> bool {
    chunk
        .terrain
        .iter()
        .chain(&chunk.halo)
        .any(|c| edits.terrain_edited_in_column(c.position, i32::MIN, i32::MAX))
}
pub(super) fn suppressed_terrain(world: &World) -> BTreeSet<ChunkId> {
    world
        .get_resource::<Cache>()
        .into_iter()
        .flat_map(|c| &c.batches)
        .filter(|(_, b)| b.terrain_edited)
        .map(|(c, _)| *c)
        .collect()
}

#[derive(Resource)]
struct Cache {
    identity: (u64, u64),
    standard: Handle<StandardMaterial>,
    rivers: BTreeMap<Style, Handle<RiverMaterial>>,
    batches: BTreeMap<ChunkId, Batch>,
}
struct Batch {
    revisions: Vec<(ChunkId, u64)>,
    parts: Vec<(Entity, Handle<Mesh>)>,
    terrain_edited: bool,
}

pub(super) fn sync(world: &mut World, state: &StreamedArena, detailed: &BTreeSet<ChunkId>) {
    let Some(source) = state.overview.inland_water.as_ref() else {
        clear(world);
        return;
    };
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
        let Some(color) = state
            .overview
            .materials
            .iter()
            .find(|m| m.id == "water")
            .map(|m| m.color)
        else {
            return;
        };
        let [r, g, b, a] = color;
        let color = Color::srgba_u8(r, g, b, a);
        let standard = world
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                base_color: color,
                alpha_mode: if a < 255 {
                    AlphaMode::Blend
                } else {
                    AlphaMode::Opaque
                },
                perceptual_roughness: 0.9,
                ..default()
            });
        let mut rivers = BTreeMap::new();
        if world.contains_resource::<river::Enabled>()
            && world.contains_resource::<Assets<RiverMaterial>>()
        {
            let seconds = world
                .get_resource::<crate::ocean::OceanFrame>()
                .map_or(0.0, |frame| frame.time().seconds);
            for style in [Style::Current, Style::Rapid, Style::Fall] {
                let mut material = river::material(style, color, seconds);
                river::set_origin(
                    &mut material,
                    crate::v4::RenderOrigin::default(),
                    state.overview.level_height,
                );
                rivers.insert(
                    style,
                    world.resource_mut::<Assets<RiverMaterial>>().add(material),
                );
            }
        }
        world.insert_resource(Cache {
            identity,
            standard,
            rivers,
            batches: BTreeMap::new(),
        });
    }
    world.resource_scope(|world, mut cache: Mut<Cache>| {
        cache.update(
            world,
            source,
            &state.edits,
            detailed,
            state.overview.level_height,
        );
    });
}

pub(super) fn set_detailed(world: &mut World, detailed: &BTreeSet<ChunkId>) {
    if !world.contains_resource::<Cache>() {
        return;
    }
    world.resource_scope(|world, cache: Mut<Cache>| {
        for (chunk, batch) in &cache.batches {
            let wanted = visibility(detailed.contains(chunk));
            for (entity, _) in &batch.parts {
                if world.get::<Visibility>(*entity) != Some(&wanted) {
                    if let Ok(mut entity) = world.get_entity_mut(*entity) {
                        entity.insert(wanted);
                    }
                }
            }
        }
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
        materials.remove(cache.standard.id());
    }
    if let Some(mut materials) = world.get_resource_mut::<Assets<RiverMaterial>>() {
        for handle in cache.rivers.into_values() {
            materials.remove(handle.id());
        }
    }
}
fn retire(world: &mut World, batch: Batch) {
    for (entity, mesh) in batch.parts {
        world.despawn(entity);
        if let Some(mut meshes) = world.get_resource_mut::<Assets<Mesh>>() {
            meshes.remove(mesh.id());
        }
    }
}
impl Cache {
    fn update(
        &mut self,
        world: &mut World,
        source: &InlandWaterOverview,
        edits: &FiniteWorldSession,
        detailed: &BTreeSet<ChunkId>,
        pitch: f32,
    ) {
        for chunk in &source.chunks {
            let revisions = if let Some(batch) = self.batches.get(&chunk.coordinate) {
                batch
                    .revisions
                    .iter()
                    .map(|(c, _)| (*c, edits.revision(*c).unwrap_or(0)))
                    .collect::<Vec<_>>()
            } else {
                dependencies(chunk)
                    .into_iter()
                    .map(|c| (c, edits.revision(c).unwrap_or(0)))
                    .collect::<Vec<_>>()
            };
            if self
                .batches
                .get(&chunk.coordinate)
                .is_none_or(|batch| batch.revisions != revisions)
            {
                if let Some(old) = self.batches.remove(&chunk.coordinate) {
                    retire(world, old);
                }
                let meshes = chunk_meshes(chunk, edits, pitch);
                let mut parts = Vec::new();
                for (style, mesh) in meshes {
                    if mesh.count_vertices() == 0 {
                        continue;
                    }
                    let mesh = world.resource_mut::<Assets<Mesh>>().add(mesh);
                    let mut entity = world.spawn((
                        Name::new("Grand exact distant inland water"),
                        Mesh3d(mesh.clone()),
                        Transform::default(),
                        visibility(detailed.contains(&chunk.coordinate)),
                    ));
                    if let Some(material) = style.and_then(|s| self.rivers.get(&s)) {
                        entity.insert(MeshMaterial3d(material.clone()));
                    } else {
                        entity.insert(MeshMaterial3d(self.standard.clone()));
                    }
                    parts.push((entity.id(), mesh));
                }
                self.batches.insert(
                    chunk.coordinate,
                    Batch {
                        revisions,
                        parts,
                        terrain_edited: terrain_edited(chunk, edits),
                    },
                );
            }
            if let Some(batch) = self.batches.get(&chunk.coordinate) {
                for (entity, _) in &batch.parts {
                    let wanted = visibility(detailed.contains(&chunk.coordinate));
                    if world.get::<Visibility>(*entity) != Some(&wanted) {
                        if let Ok(mut entity) = world.get_entity_mut(*entity) {
                            entity.insert(wanted);
                        }
                    }
                }
            }
        }
    }
}
fn visibility(detailed: bool) -> Visibility {
    if detailed {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    }
}
pub(super) fn neighbors(at: WorldHex) -> impl Iterator<Item = WorldHex> {
    std::iter::once(at).chain(
        DIRECTIONS
            .into_iter()
            .filter_map(move |(q, r)| at.checked_add(WorldHex::new(q, r)).ok()),
    )
}
fn dependencies(chunk: &InlandWaterChunk) -> BTreeSet<ChunkId> {
    chunk
        .columns
        .iter()
        .flat_map(|c| neighbors(c.column))
        .chain(chunk.terrain.iter().chain(&chunk.halo).map(|c| c.position))
        .map(WorldHex::chunk)
        .collect()
}
fn edited(column: &InlandWaterColumn, edits: &FiniteWorldSession) -> bool {
    // Source side exposure depends on this column and its six neighbors. A
    // retained edit at any intersecting face level invalidates the far column;
    // detailed publication remains the only authority for the changed shape.
    neighbors(column.column).any(|at| {
        edits.terrain_edited_in_column(
            at,
            column.bottom.saturating_sub(1),
            column.top.saturating_add(1),
        )
    })
}
fn chunk_meshes(
    chunk: &InlandWaterChunk,
    edits: &FiniteWorldSession,
    pitch: f32,
) -> BTreeMap<Option<Style>, Mesh> {
    let mut meshes = BTreeMap::<Option<Style>, MeshData>::new();
    for column in &chunk.columns {
        if edited(column, edits) {
            continue;
        }
        meshes
            .entry(river::liquid_style(
                column.kind,
                column.top,
                column.downstream,
            ))
            .or_default()
            .column(column, pitch);
    }
    meshes
        .into_iter()
        .map(|(style, data)| (style, data.mesh()))
        .collect()
}
#[derive(Default)]
pub(super) struct MeshData {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    indices: Vec<u32>,
}
#[expect(
    clippy::cast_precision_loss,
    reason = "Admitted integer levels and coordinates lie inside the bounded f32 local presentation envelope."
)]
pub(super) fn level(value: i32, pitch: f32) -> f32 {
    value as f32 * pitch
}
impl MeshData {
    pub(super) fn vertex_len(&self) -> usize {
        self.positions.len()
    }
    fn face(&mut self, points: &[Vec3], normal: Vec3, triangles: &[u32]) {
        let Ok(start) = u32::try_from(self.positions.len()) else {
            return;
        };
        self.positions.extend(points.iter().map(|p| p.to_array()));
        self.normals
            .extend(std::iter::repeat_n(normal.to_array(), points.len()));
        self.indices.extend(triangles.iter().map(|i| start + i));
    }
    pub(super) fn column(&mut self, column: &InlandWaterColumn, pitch: f32) {
        let [x, z] = hex_schematic::v4::northern::world_xz(column.column);
        let center = bevy::math::DVec3::new(x, 0.0, z).as_vec3();
        let h = 3.0_f32.sqrt() * 0.5;
        let corners = [
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(h, 0.0, 0.5),
            Vec3::new(h, 0.0, -0.5),
            Vec3::new(0.0, 0.0, -1.0),
            Vec3::new(-h, 0.0, -0.5),
            Vec3::new(-h, 0.0, 0.5),
        ];
        for (shown, height, normal, reverse) in [
            (column.exposed_top, column.top, Vec3::Y, false),
            (column.exposed_bottom, column.bottom, Vec3::NEG_Y, true),
        ] {
            if !shown {
                continue;
            }
            let center = center + Vec3::Y * level(height, pitch);
            let mut points = vec![center];
            points.extend(corners.iter().map(|p| center + *p));
            let mut triangles = Vec::new();
            for i in 0..6_u32 {
                let a = i + 1;
                let b = (i + 1) % 6 + 1;
                triangles.extend(if reverse { [0, b, a] } else { [0, a, b] });
            }
            self.face(&points, normal, &triangles);
        }
        for (side, (a, b)) in
            column
                .sides
                .iter()
                .zip([(1, 2), (0, 1), (5, 0), (4, 5), (3, 4), (2, 3)])
        {
            let (Some(a), Some(b)) = (corners.get(a), corners.get(b)) else {
                continue;
            };
            let normal = (*b - *a).cross(Vec3::Y).normalize();
            for &[bottom, top] in side {
                let low = Vec3::Y * level(bottom, pitch);
                let high = Vec3::Y * level(top, pitch);
                self.face(
                    &[
                        center + *a + low,
                        center + *b + low,
                        center + *b + high,
                        center + *a + high,
                    ],
                    normal,
                    &[0, 1, 2, 0, 2, 3],
                );
            }
        }
    }
    pub(super) fn mesh(self) -> Mesh {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, self.positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals);
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_UV_0,
            vec![[0.0, 0.0]; mesh.count_vertices()],
        );
        mesh.insert_indices(Indices::U32(self.indices));
        mesh
    }
}

#[cfg(test)]
#[path = "grand_water_tests.rs"]
mod tests;
