//! Grand fragment-only terrain filtering. Exact mesh/picking stay intact.
use super::{PreparedChunk, PresentationError, RunSource};
use bevy::{
    mesh::VertexAttributeValues,
    pbr::{ExtendedMaterial, MaterialExtension},
    prelude::*,
    render::render_resource::AsBindGroup,
    shader::ShaderRef,
};
use hex_schematic::v4::northern::NorthernOverview;
use hex_world_contracts::{ColumnData, WorldHex};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Asset, AsBindGroup, TypePath, Clone)]
pub(crate) struct Extension {
    #[uniform(100)]
    pixels: Vec4,
}
impl MaterialExtension for Extension {
    fn fragment_shader() -> ShaderRef {
        "shaders/grand_terrain_shading.wgsl".into()
    }
}
pub(crate) type FilterMaterial = ExtendedMaterial<StandardMaterial, Extension>;
#[derive(Resource)]
pub(crate) struct Enabled;

fn selected(value: Option<&str>) -> bool {
    value != Some("0")
}

pub(crate) fn install(app: &mut App) {
    // Logical/headless apps need neither material resources nor extra systems.
    if app.get_sub_app(bevy::render::RenderApp).is_some()
        && selected(std::env::var("HEX_GRAND_TERRAIN_SHADING").ok().as_deref())
    {
        app.add_plugins(MaterialPlugin::<FilterMaterial>::default())
            .insert_resource(Enabled);
    }
}
pub(super) fn material(base: StandardMaterial) -> FilterMaterial {
    FilterMaterial {
        base,
        extension: Extension {
            pixels: Vec4::new(1.0, 3.0, 2.0, 0.0),
        },
    }
}

#[derive(Default, Debug)]
pub(crate) struct Metrics {
    pub vertices: usize,
    pub eligible_vertices: usize,
    pub protected_triangles: usize,
    pub extra_bytes: usize,
    pub original_uv_bytes: usize,
}

const DIRECTIONS: [(i64, i64); 6] = [(1, 0), (0, 1), (-1, 1), (-1, 0), (0, -1), (1, -1)];
fn single_solid_top(column: &ColumnData, map: &NorthernOverview) -> Option<i32> {
    let mut end = None;
    for run in &column.runs {
        if !map
            .materials
            .iter()
            .any(|m| m.id == run.material && m.solid && m.color.last() == Some(&255))
            || end.is_some_and(|e| e != run.bottom)
        {
            return None;
        }
        end = Some(run.top);
    }
    end
}
#[expect(
    clippy::cast_precision_loss,
    reason = "admitted finite Grand level envelope"
)]
fn feature(
    at: WorldHex,
    map: &NorthernOverview,
    facts: &BTreeMap<WorldHex, ColumnData>,
    edited: &BTreeSet<WorldHex>,
) -> Option<(f32, f32)> {
    let top = single_solid_top(facts.get(&at)?, map)?;
    let mut height = map.level_height;
    for p in std::iter::once(at).chain(
        DIRECTIONS
            .into_iter()
            .filter_map(|(q, r)| at.checked_add(WorldHex::new(q, r)).ok()),
    ) {
        if edited.contains(&p) {
            return None;
        }
        let neighbor = single_solid_top(facts.get(&p)?, map)?;
        if neighbor as f32 * map.level_height <= map.sea_level {
            return None;
        }
        height = height.max((top - neighbor).unsigned_abs() as f32 * map.level_height);
    }
    // Never collapse a real cliff into its thin material bands. This is the
    // whole connected exposed step, with an additional macro-cell physical cap.
    (height <= map.spacing).then_some((top as f32 * map.level_height, height))
}

// These are the stable Grand natural-surface families. Special authored materials
// (including Crystal/worked stone/foliage/timber) never become macro terrain color.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Family {
    Ground,
    Snow,
    Sand,
    Basalt,
}
fn natural_family(id: &str) -> Option<Family> {
    match id {
        "moss" | "soil" | "dirt" | "stone" | "slate" | "limestone" => Some(Family::Ground),
        "snow" => Some(Family::Snow),
        "sand" => Some(Family::Sand),
        "basalt" => Some(Family::Basalt),
        _ => None,
    }
}
struct MacroTarget {
    slope: [f32; 2],
    color: [f32; 3],
    top: f32,
}
#[derive(Clone, Copy)]
struct Feature {
    minimum_top: f32,
    maximum_top: f32,
    height: f32,
}
fn reliable_target(target: &MacroTarget, feature: Feature, spacing: f32) -> bool {
    // Compare to authoritative exterior tops, never a lower side-fragment center.
    // A gross coarse approximation error stays visibly unfiltered.
    (target.top - feature.minimum_top).abs() <= spacing
        && (target.top - feature.maximum_top).abs() <= spacing
}

#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "validated overview dimensions and clamped finite grid coordinates"
)]
fn sample(map: &NorthernOverview, x: f32, z: f32, family: Family) -> Option<MacroTarget> {
    let [ox, oz] = map.origin_xz;
    let gx = ((x - ox) / map.spacing).clamp(0.0, map.width.checked_sub(1)? as f32);
    let gz = ((z - oz) / map.spacing).clamp(0.0, map.height.checked_sub(1)? as f32);
    let i = (gx.floor() as u32).min(map.width.checked_sub(2)?) as usize;
    let j = (gz.floor() as u32).min(map.height.checked_sub(2)?) as usize;
    let tx = gx - i as f32;
    let tz = gz - j as f32;
    let stride = map.width as usize;
    let at = j * stride + i;
    let ids = [at, at + 1, at + stride, at + stride + 1];
    let [a, b, c, d] = ids.map(|k| map.bed_heights.get(k).copied());
    let (a, b, c, d) = (a?, b?, c?, d?);
    let dx = ((b - a) + ((d - c) - (b - a)) * tz) / map.spacing;
    let dz = ((c + (d - c) * tx) - (a + (b - a) * tx)) / map.spacing;
    let colors = ids.map(|k| {
        let material = map
            .surface_materials
            .get(k)
            .and_then(|i| map.materials.get(usize::from(*i)))?;
        let [r, g, b, a] = material.color;
        if a != 255 || !material.solid || natural_family(&material.id) != Some(family) {
            return None;
        }
        let c = Color::srgba_u8(r, g, b, a).to_linear();
        Some(Vec3::new(c.red, c.green, c.blue))
    });
    let [Some(a), Some(b), Some(c), Some(d)] = colors else {
        return None;
    };
    let top = map.bed_heights.get(at).copied()? * (1.0 - tx) * (1.0 - tz)
        + map.bed_heights.get(at + 1).copied()? * tx * (1.0 - tz)
        + map.bed_heights.get(at + stride).copied()? * (1.0 - tx) * tz
        + map.bed_heights.get(at + stride + 1).copied()? * tx * tz;
    Some(MacroTarget {
        slope: [-dx, -dz],
        color: a.lerp(b, tx).lerp(c.lerp(d, tx), tz).to_array(),
        top,
    })
}

#[expect(clippy::cast_precision_loss, reason = "admitted finite source levels")]
fn object_intersects(
    objects: &[ColumnData],
    at: WorldHex,
    minimum: f32,
    maximum: f32,
    level_height: f32,
) -> bool {
    objects
        .binary_search_by_key(&at, |c| c.position)
        .ok()
        .and_then(|i| objects.get(i))
        .is_some_and(|column| {
            column.runs.iter().any(|r| {
                r.bottom as f32 * level_height <= maximum + 0.002
                    && r.top as f32 * level_height >= minimum - 0.002
            })
        })
}

/// Decorate only disposable opaque terrain attributes. Source and vertex geometry
/// are untouched. Missing/edited/stacked/wet neighborhoods retain original shading.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "validated finite render origin, world columns and levels"
)]
pub(crate) fn decorate(
    prepared: &mut PreparedChunk,
    map: &NorthernOverview,
    facts: &BTreeMap<WorldHex, ColumnData>,
    edited: &BTreeSet<WorldHex>,
    objects: &[ColumnData],
) -> Result<Metrics, PresentationError> {
    let mut result = Metrics::default();
    if map.world_id != "grand-v4" {
        return Ok(result);
    }
    let origin = prepared.context.origin;
    let [x, z] = hex_schematic::v4::northern::world_xz(origin.column);
    let offset = Vec3::new(x as f32, origin.level as f32 * map.level_height, z as f32);
    let mut feature_cache = BTreeMap::new();
    for batch in &mut prepared.batches {
        if batch.river.is_some()
            || !batch.material.solid
            || batch.material.color.last() != Some(&255)
            || batch
                .runs
                .iter()
                .any(|r| r.exact.source != RunSource::Terrain)
        {
            continue;
        }
        let Some(family) = natural_family(&batch.material.id) else {
            continue;
        };
        let Some(mesh) = &mut batch.mesh else {
            continue;
        };
        if mesh.attribute(Mesh::ATTRIBUTE_COLOR).is_some()
            || mesh.attribute(Mesh::ATTRIBUTE_UV_1).is_some()
        {
            continue;
        }
        let Some(VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            continue;
        };
        let Some(VertexAttributeValues::Float32x3(normals)) =
            mesh.attribute(Mesh::ATTRIBUTE_NORMAL)
        else {
            continue;
        };
        let Some(indices) = mesh.indices() else {
            continue;
        };
        let indices: Vec<_> = indices.iter().collect();
        let mut features = vec![None::<Feature>; positions.len()];
        let mut blocked = vec![false; positions.len()];
        for triangle in indices.chunks_exact(3) {
            let [a, b, c] = triangle else {
                continue;
            };
            let (Some(pa), Some(pb), Some(pc)) =
                (positions.get(*a), positions.get(*b), positions.get(*c))
            else {
                return Err(PresentationError("invalid shading mesh indices".into()));
            };
            let normal = (Vec3::from_array(*pb) - Vec3::from_array(*pa))
                .cross(Vec3::from_array(*pc) - Vec3::from_array(*pa))
                .normalize_or_zero();
            let points = [
                Vec3::from_array(*pa) + offset,
                Vec3::from_array(*pb) + offset,
                Vec3::from_array(*pc) + offset,
            ];
            let center = points.into_iter().sum::<Vec3>() / 3.0;
            let minimum = points
                .into_iter()
                .map(|p| p.y)
                .fold(f32::INFINITY, f32::min);
            let maximum = points
                .into_iter()
                .map(|p| p.y)
                .fold(f32::NEG_INFINITY, f32::max);
            let hex = hex_core::HexCoord::from_world(center - normal * 0.002);
            let at = WorldHex::new(i64::from(hex.x()), i64::from(hex.y()));
            let admitted = (*feature_cache
                .entry(at)
                .or_insert_with(|| feature(at, map, facts, edited)))
            .filter(|(top, height)| {
                normal.y >= -0.001
                    && maximum <= *top + 0.002
                    && minimum >= *top - *height - 0.002
                    && !object_intersects(objects, at, minimum, maximum, map.level_height)
            });
            if admitted.is_none() {
                result.protected_triangles += 1;
            }
            for index in triangle {
                if let Some((top, height)) = admitted {
                    if let Some(value) = features.get_mut(*index) {
                        let previous = value.unwrap_or(Feature {
                            minimum_top: top,
                            maximum_top: top,
                            height,
                        });
                        *value = Some(Feature {
                            minimum_top: previous.minimum_top.min(top),
                            maximum_top: previous.maximum_top.max(top),
                            height: previous.height.max(height),
                        });
                    }
                } else if let Some(value) = blocked.get_mut(*index) {
                    *value = true;
                }
            }
        }
        let mut colors = Vec::with_capacity(positions.len());
        let mut slopes = Vec::with_capacity(positions.len());
        let mut uvs = Vec::with_capacity(positions.len());
        let mut eligible = 0;
        for (i, position) in positions.iter().enumerate() {
            let p = Vec3::from_array(*position) + offset;
            let target = sample(map, p.x, p.z, family);
            let feature = features
                .get(i)
                .copied()
                .flatten()
                .filter(|_| !blocked.get(i).copied().unwrap_or(true));
            let (slope, color, weight, height) = match (target, feature) {
                (Some(target), Some(feature)) if reliable_target(&target, feature, map.spacing) => {
                    eligible += 1;
                    (target.slope, target.color, 1.0, feature.height)
                }
                _ => ([0.0; 2], [1.0; 3], 0.0, 0.0),
            };
            let [r, g, b] = color;
            colors.push([r, g, b, weight]);
            slopes.push(slope);
            uvs.push([height, 0.0]);
        }
        if eligible == 0 {
            continue;
        }
        // Original POSITION/NORMAL/indices are deliberately never reinserted.
        if normals.len() != positions.len() {
            return Err(PresentationError("invalid shading normal count".into()));
        }
        result.vertices += positions.len();
        result.eligible_vertices += eligible;
        result.extra_bytes += positions.len() * 24;
        batch.shading_uv0 = mesh.attribute(Mesh::ATTRIBUTE_UV_0).cloned();
        result.original_uv_bytes += batch
            .shading_uv0
            .as_ref()
            .map_or(0, |uv| uv.get_bytes().len());
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, slopes);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
        batch.shading = true;
    }
    Ok(result)
}

/// Keep optional attributes bounded without rejecting authoritative presentation.
/// Restored completions publish and accept normally; budget pressure schedules no retry.
pub(crate) fn enforce_budget(
    prepared: &mut [PreparedChunk],
    metrics: &mut BTreeMap<hex_world_contracts::ChunkId, Metrics>,
    retained_bytes: usize,
    limit: usize,
) -> Option<usize> {
    let attempted = metrics.values().fold(retained_bytes, |bytes, m| {
        bytes.saturating_add(m.extra_bytes)
    });
    if attempted <= limit {
        return None;
    }
    for chunk in prepared {
        for batch in &mut chunk.batches {
            if !batch.shading {
                continue;
            }
            if let Some(mesh) = &mut batch.mesh {
                mesh.remove_attribute(Mesh::ATTRIBUTE_COLOR);
                mesh.remove_attribute(Mesh::ATTRIBUTE_UV_1);
                if let Some(original) = batch.shading_uv0.take() {
                    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, original);
                } else {
                    mesh.remove_attribute(Mesh::ATTRIBUTE_UV_0);
                }
            }
            batch.shading = false;
        }
    }
    for metric in metrics.values_mut() {
        *metric = Metrics::default();
    }
    Some(attempted)
}

#[cfg(test)]
#[path = "grand_terrain_shading_tests.rs"]
mod tests;
