//! Whole-chunk exact solid presentation surrounding compiler-authored inland water.
use super::grand_water::{self, MeshData};
use bevy::prelude::*;
use hex_schematic::v4::northern::{
    InlandWaterChunk, InlandWaterColumn, NorthernOverview, MAX_INLAND_TERRAIN_COLUMNS,
    MAX_INLAND_TERRAIN_RUNS,
};
use hex_world_contracts::{ChunkId, ColumnData, LiquidKind, WorldHex};
use std::collections::{BTreeMap, BTreeSet};

const MAX_VERTICES: usize = 8_000_000;
pub(super) type Edges = BTreeMap<ChunkId, BTreeMap<WorldHex, f32>>;

pub(super) fn validate(map: &NorthernOverview) -> Result<usize, String> {
    let Some(source) = &map.inland_water else {
        return Ok(0);
    };
    let invalid = || "Invalid exact inland terrain replacement".to_owned();
    let solids: BTreeSet<_> = map
        .materials
        .iter()
        .filter(|m| m.solid)
        .map(|m| m.id.as_str())
        .collect();
    let mut columns = 0_usize;
    let mut runs = 0_usize;
    let mut vertices = 0_usize;
    let mut shared = BTreeMap::<WorldHex, &ColumnData>::new();
    for chunk in &source.chunks {
        let mut own = BTreeSet::new();
        for q in 0..16 {
            for r in 0..16 {
                let p = WorldHex::new(
                    chunk
                        .coordinate
                        .q
                        .checked_mul(16)
                        .and_then(|v| v.checked_add(q))
                        .ok_or_else(invalid)?,
                    chunk
                        .coordinate
                        .r
                        .checked_mul(16)
                        .and_then(|v| v.checked_add(r))
                        .ok_or_else(invalid)?,
                );
                if p.checked_distance(WorldHex::new(0, 0))
                    .is_ok_and(|d| d <= u64::from(map.radius))
                {
                    own.insert(p);
                }
            }
        }
        let halo: BTreeSet<_> = own
            .iter()
            .flat_map(|p| grand_water::neighbors(*p))
            .filter(|p| {
                p.chunk() != chunk.coordinate
                    && p.checked_distance(WorldHex::new(0, 0))
                        .is_ok_and(|d| d <= u64::from(map.radius))
            })
            .collect();
        for (actual, expected) in [(&chunk.terrain, &own), (&chunk.halo, &halo)] {
            if actual.len() != expected.len()
                || !actual
                    .iter()
                    .map(|c| c.position)
                    .eq(expected.iter().copied())
            {
                return Err(invalid());
            }
            for column in actual {
                columns += 1;
                runs += column.runs.len();
                if columns > MAX_INLAND_TERRAIN_COLUMNS || runs > MAX_INLAND_TERRAIN_RUNS {
                    return Err(invalid());
                }
                if shared
                    .insert(column.position, column)
                    .is_some_and(|old| old != column)
                {
                    return Err(invalid());
                }
                let mut end = None;
                for run in &column.runs {
                    if run.bottom < map.level_bounds[0]
                        || run.top > map.level_bounds[1].saturating_add(1)
                        || run.bottom >= run.top
                        || end.is_some_and(|v| v > run.bottom)
                        || !solids.contains(run.material.as_str())
                    {
                        return Err(invalid());
                    }
                    end = Some(run.top);
                }
            }
        }
        faces(chunk, |prism, _| {
            vertices += grand_water::vertex_count(prism);
        });
        if vertices > MAX_VERTICES {
            return Err(invalid());
        }
    }
    Ok(vertices)
}
fn exposed(bottom: i32, top: i32, neighbor: Option<&&ColumnData>) -> Vec<[i32; 2]> {
    let mut cursor = bottom;
    let mut result = Vec::new();
    for run in neighbor.into_iter().flat_map(|c| &c.runs) {
        if run.top <= cursor {
            continue;
        }
        if run.bottom >= top {
            break;
        }
        if cursor < run.bottom {
            result.push([cursor, run.bottom.min(top)]);
        }
        cursor = cursor.max(run.top);
        if cursor >= top {
            break;
        }
    }
    if cursor < top {
        result.push([cursor, top]);
    }
    result
}
fn faces(chunk: &InlandWaterChunk, mut visit: impl FnMut(&InlandWaterColumn, &str)) {
    let columns: BTreeMap<_, _> = chunk
        .terrain
        .iter()
        .chain(&chunk.halo)
        .map(|c| (c.position, c))
        .collect();
    for column in &chunk.terrain {
        for run in &column.runs {
            let sides = std::array::from_fn(|i| {
                let Some(&(dq, dr)) = grand_water::DIRECTIONS.get(i) else {
                    return vec![];
                };
                let Some(neighbor) = column.position.checked_add(WorldHex::new(dq, dr)).ok() else {
                    return vec![];
                };
                exposed(run.bottom, run.top, columns.get(&neighbor))
            });
            // Adjacent material strata are buried. Only an actual empty interval
            // exposes a cap, including interior ceilings, never a water-level guess.
            let prism = InlandWaterColumn {
                column: column.position,
                bottom: run.bottom,
                top: run.top,
                kind: LiquidKind::Standing,
                downstream: None,
                exposed_top: !column.runs.iter().any(|other| other.bottom == run.top),
                exposed_bottom: run.bottom > 0
                    && !column.runs.iter().any(|other| other.top == run.bottom),
                sides,
            };
            visit(&prism, &run.material);
        }
    }
}
pub(super) fn chunk(map: &NorthernOverview, at: ChunkId) -> Option<&InlandWaterChunk> {
    let source = map.inland_water.as_ref()?;
    source
        .chunks
        .binary_search_by_key(&at, |c| c.coordinate)
        .ok()
        .and_then(|i| source.chunks.get(i))
}
pub(super) fn mesh(map: &NorthernOverview, chunk: &InlandWaterChunk) -> Mesh {
    let palette: BTreeMap<_, _> = map
        .materials
        .iter()
        .map(|m| {
            let [r, g, b, a] = m.color;
            let c = Color::srgba_u8(r, g, b, a).to_linear();
            (m.id.as_str(), [c.red, c.green, c.blue, c.alpha])
        })
        .collect();
    let mut data = MeshData::default();
    let mut colors = Vec::new();
    faces(chunk, |prism, material| {
        let before = data.vertex_len();
        data.column(prism, map.level_height);
        let color = palette
            .get(material)
            .copied()
            .unwrap_or([1.0, 0.0, 1.0, 1.0]);
        colors.extend(std::iter::repeat_n(color, data.vertex_len() - before));
    });
    let mut mesh = data.mesh();
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh
}
pub(super) fn edges(map: &NorthernOverview) -> Edges {
    map.inland_water
        .iter()
        .flat_map(|s| &s.chunks)
        .map(|chunk| {
            let columns = chunk
                .terrain
                .iter()
                .filter(|c| {
                    let q = c.position.q.rem_euclid(16);
                    let r = c.position.r.rem_euclid(16);
                    q == 0 || q == 15 || r == 0 || r == 15
                })
                .map(|c| {
                    (
                        c.position,
                        grand_water::level(c.runs.last().map_or(0, |r| r.top), map.level_height),
                    )
                })
                .collect();
            (chunk.coordinate, columns)
        })
        .collect()
}
