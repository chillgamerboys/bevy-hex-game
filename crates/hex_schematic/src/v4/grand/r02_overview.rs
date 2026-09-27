//! Source-owned review cameras and exact distant inland terrain/water faces.
use super::super::northern::{
    InlandWaterChunk, InlandWaterColumn, InlandWaterOverview, NorthernReviewCamera,
    MAX_INLAND_TERRAIN_COLUMNS, MAX_INLAND_TERRAIN_RUNS, MAX_INLAND_WATER_COLUMNS,
    MAX_INLAND_WATER_SIDE_INTERVALS,
};
use super::*;
use std::collections::BTreeSet;
fn subtract(mut spans: Vec<[i32; 2]>, bottom: i32, top: i32) -> Vec<[i32; 2]> {
    let mut out = Vec::new();
    for [a, b] in spans.drain(..) {
        if a < bottom {
            let end = b.min(bottom);
            if a < end {
                out.push([a, end]);
            }
        }
        if b > top {
            let start = a.max(top);
            if start < b {
                out.push([start, b]);
            }
        }
    }
    out
}
impl GrandCompiler {
    pub(super) fn r02_inland(&self) -> Result<InlandWaterOverview, ContractError> {
        let mut groups: BTreeMap<ChunkId, Vec<InlandWaterColumn>> = BTreeMap::new();
        let mut count = 0;
        for &(r, a, b) in &self.source.mainland_rows {
            for q in a..=b {
                let p = WorldHex::new(q, r);
                let (column, liquid) = self.column(p);
                let Some(mut liquid) = liquid.filter(|l| l.body_id == "grand/river") else {
                    continue;
                };
                self.direct_river(&mut liquid);
                count += 1;
                if count > MAX_INLAND_WATER_COLUMNS {
                    return Err(ContractError::new(
                        "grand.inland",
                        "inland water column budget exceeded",
                    ));
                }
                let mut sides: [Vec<[i32; 2]>; 6] = std::array::from_fn(|_| Vec::new());
                for (side, (dq, dr)) in sides.iter_mut().zip(DIRS) {
                    let neighbor = self.column(WorldHex::new(q + dq, r + dr)).0;
                    let mut spans = vec![[liquid.bottom, liquid.top]];
                    for run in neighbor.runs {
                        spans = subtract(spans, run.bottom, run.top);
                    }
                    if spans.len() > MAX_INLAND_WATER_SIDE_INTERVALS {
                        return Err(ContractError::new(
                            "grand.inland",
                            "water side interval budget exceeded",
                        ));
                    }
                    *side = spans;
                }
                let solid_at = |level| {
                    column
                        .runs
                        .iter()
                        .any(|r| r.material != "water" && r.bottom <= level && level < r.top)
                };
                groups
                    .entry(p.chunk())
                    .or_default()
                    .push(InlandWaterColumn {
                        column: p,
                        bottom: liquid.bottom,
                        top: liquid.top,
                        kind: liquid.kind,
                        downstream: liquid.downstream.first().copied(),
                        exposed_top: !solid_at(liquid.top),
                        exposed_bottom: !solid_at(liquid.bottom - 1),
                        sides,
                    });
            }
        }
        let mut chunks = Vec::new();
        let mut total_columns = 0;
        let mut total_runs = 0;
        for (coordinate, mut columns) in groups {
            columns.sort_by_key(|c| c.column);
            let origin = coordinate.origin()?;
            let mut own = BTreeSet::new();
            let mut halo = BTreeSet::new();
            for q in 0..CHUNK_SIZE {
                for r in 0..CHUNK_SIZE {
                    let p = WorldHex::new(origin.q + q, origin.r + r);
                    if p.checked_distance(WorldHex::new(0, 0))? <= RADIUS as u64 {
                        own.insert(p);
                    }
                }
            }
            for &p in &own {
                for (dq, dr) in DIRS {
                    let n = WorldHex::new(p.q + dq, p.r + dr);
                    if !own.contains(&n)
                        && n.checked_distance(WorldHex::new(0, 0))? <= RADIUS as u64
                    {
                        halo.insert(n);
                    }
                }
            }
            let mut exact =
                |positions: BTreeSet<WorldHex>| -> Result<Vec<ColumnData>, ContractError> {
                    let mut out = Vec::new();
                    for p in positions {
                        let (mut column, _) = self.column(p);
                        column.runs.retain(|r| r.material != "water");
                        total_columns += 1;
                        total_runs += column.runs.len();
                        if total_columns > MAX_INLAND_TERRAIN_COLUMNS
                            || total_runs > MAX_INLAND_TERRAIN_RUNS
                        {
                            return Err(ContractError::new(
                                "grand.inland",
                                "exact inland terrain budget exceeded",
                            ));
                        }
                        out.push(column);
                    }
                    Ok(out)
                };
            chunks.push(InlandWaterChunk {
                coordinate,
                columns,
                terrain: exact(own)?,
                halo: exact(halo)?,
            });
        }
        Ok(InlandWaterOverview { version: 1, chunks })
    }
    pub(super) fn r02_cameras(&self) -> BTreeMap<String, NorthernReviewCamera> {
        let Some(d) = &self.geography.document else {
            return BTreeMap::new();
        };
        let position = |p: [f64; 3]| {
            let [x, z] = self.geography.world_xz([p[0], p[2]]);
            [
                x as f32,
                (SEA_TOP as f64 * LEVEL_HEIGHT + p[1] * d.transform.vertical_scale) as f32,
                z as f32,
            ]
        };
        d.review_cameras
            .iter()
            .map(|(id, c)| {
                let mut eye = position(c.eye);
                let mut interest = position(c.interest);
                if c.ground {
                    if let Ok(support) =
                        self.r02_support([c.eye[0], c.eye[2]], Some(c.eye[1] - 1.7))
                    {
                        eye[1] = (f64::from(support.level + 1) * LEVEL_HEIGHT + 1.7) as f32;
                        interest = eye;
                    }
                }
                (
                    id.clone(),
                    NorthernReviewCamera {
                        eye,
                        target: position(c.target),
                        interest,
                        orthographic_span: c
                            .horizontal_span
                            .map(|span| (self.geography.length(span) * 9. / 16.) as f32),
                        ground: c.ground,
                    },
                )
            })
            .collect()
    }
}
