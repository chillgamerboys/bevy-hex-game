//! Actual emitted r02 liquid graphs, selected by canonical authored channels.
use hex_schematic::v4::{
    grand::{GrandCompiler, GrandSpec, LEVEL_HEIGHT, RIVER_PHASE_DIRECTION, SEA_TOP},
    northern::{nearest_hex, world_xz},
};
use hex_world_contracts::{ChunkId, ChunkPackage, LiquidColumn, LiquidKind, WorldHex};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;

type TestResult = Result<(), Box<dyn Error>>;

// This reader selects only the source facts needed by an independent graph
// oracle. It never reproduces terrain or uses the old world's channel windows.
#[derive(Deserialize)]
struct FlowDocument {
    transform: Transform,
    fountain_rill: Channel,
    falls: Channel,
    river: Channel,
    upper_lake: Lake,
    lower_lake: Lake,
}
#[derive(Deserialize)]
struct Transform {
    horizontal_scale: f64,
    vertical_scale: f64,
    translation: [f64; 2],
}
#[derive(Deserialize)]
struct Channel {
    width: f64,
    points: Vec<[f64; 3]>,
}
#[derive(Deserialize)]
struct Lake {
    center: [f64; 2],
    radii: [f64; 2],
    level: f64,
    phase: f64,
}
impl FlowDocument {
    fn channels(&self) -> [&Channel; 3] {
        [&self.fountain_rill, &self.falls, &self.river]
    }
    fn model(&self, p: WorldHex) -> [f64; 2] {
        let [x, z] = world_xz(p);
        let [tx, tz] = self.transform.translation;
        let scale = self.transform.horizontal_scale;
        [(x - tx) / scale, -(z - tz) / scale]
    }
    fn hex(&self, [x, z]: [f64; 2]) -> WorldHex {
        let [tx, tz] = self.transform.translation;
        let scale = self.transform.horizontal_scale;
        nearest_hex(tx + x * scale, tz - z * scale)
    }
    fn reach(&self, p: WorldHex) -> Option<usize> {
        let point = self.model(p);
        self.channels().iter().position(|channel| {
            channel.points.windows(2).any(|pair| {
                let (Some([ax, _, az]), Some([bx, _, bz])) = (pair.first(), pair.get(1)) else {
                    return false;
                };
                distance(point, [*ax, *az], [*bx, *bz]) < channel.width * 0.5 + 0.01
            })
        })
    }
    fn candidates(&self) -> BTreeSet<WorldHex> {
        let mut result = BTreeSet::new();
        for channel in self.channels() {
            for pair in channel.points.windows(2) {
                let (Some([ax, _, az]), Some([bx, _, bz])) = (pair.first(), pair.get(1)) else {
                    continue;
                };
                let pad = channel.width * 0.5 + 2.;
                let corners = [
                    self.hex([ax.min(*bx) - pad, az.min(*bz) - pad]),
                    self.hex([ax.min(*bx) - pad, az.max(*bz) + pad]),
                    self.hex([ax.max(*bx) + pad, az.min(*bz) - pad]),
                    self.hex([ax.max(*bx) + pad, az.max(*bz) + pad]),
                ];
                let q0 = corners.iter().map(|p| p.q).min().unwrap_or(0) - 2;
                let q1 = corners.iter().map(|p| p.q).max().unwrap_or(0) + 2;
                let r0 = corners.iter().map(|p| p.r).min().unwrap_or(0) - 2;
                let r1 = corners.iter().map(|p| p.r).max().unwrap_or(0) + 2;
                for q in q0..=q1 {
                    for r in r0..=r1 {
                        let p = WorldHex::new(q, r);
                        if self.reach(p).is_some() {
                            result.insert(p);
                        }
                    }
                }
            }
        }
        result
    }
    fn receiver(&self, reach: usize, liquid: &LiquidColumn) -> bool {
        if reach == 2 {
            return liquid.top == SEA_TOP && liquid.body_id == "grand/ocean";
        }
        let lake = if reach == 0 {
            &self.upper_lake
        } else {
            &self.lower_lake
        };
        let [x, z] = self.model(liquid.column);
        let [cx, cz] = lake.center;
        let [rx, rz] = lake.radii;
        let dx = (x - cx) / rx;
        let dz = (z - cz) / rz;
        let a = dz.atan2(dx);
        let outline = 1. + 0.08 * (3. * a + lake.phase).sin() + 0.04 * (5. * a - 0.7).cos();
        let expected_top = f64::from(SEA_TOP)
            + (lake.level * self.transform.vertical_scale / LEVEL_HEIGHT).round();
        (f64::from(liquid.top) - expected_top).abs() < 0.01 && dx.hypot(dz) < outline
    }
}
fn distance([x, z]: [f64; 2], [ax, az]: [f64; 2], [bx, bz]: [f64; 2]) -> f64 {
    let dx = bx - ax;
    let dz = bz - az;
    let t = (((x - ax) * dx + (z - az) * dz) / (dx * dx + dz * dz)).clamp(0., 1.);
    (x - ax - t * dx).hypot(z - az - t * dz)
}

#[test]
fn all_r02_channel_paths_publish_exact_downhill_edges_and_reach_receiving_water() -> TestResult {
    let bytes = include_bytes!("../../../../assets/config/v4/grand-v4/geography-r02.json");
    let document: FlowDocument = serde_json::from_slice(bytes)?;
    let mut spec: GrandSpec = ron::from_str(include_str!(
        "../../../../assets/config/v4/grand-v4/world.ron"
    ))?;
    spec.full_dressing = false;
    spec.geography = Some("geography-r02.json".into());
    let compiler = GrandCompiler::with_geography(spec, serde_json::from_slice(bytes)?, bytes)?;
    // Travelling shading and physical drainage have different invariants:
    // lateral circulation at a flat bank tip may oppose the global chart. The
    // authored main channel must still advance it through every reach and fall.
    let [dx, dz] = RIVER_PHASE_DIRECTION;
    for channel in document.channels() {
        let phase = |[x, y, z]: [f64; 3]| {
            x * document.transform.horizontal_scale * f64::from(dx)
                - z * document.transform.horizontal_scale * f64::from(dz)
                - y * document.transform.vertical_scale
        };
        for pair in channel.points.windows(2) {
            let [a, b] = pair else { continue };
            assert!(
                phase(*b) > phase(*a),
                "main-channel travelling wave reverses"
            );
        }
    }
    let candidates = document.candidates();
    let mut pending: BTreeSet<_> = candidates.iter().map(|p| p.chunk()).collect();
    let mut chunks: BTreeMap<ChunkId, ChunkPackage> = BTreeMap::new();
    while let Some(id) = pending.pop_first() {
        if chunks.contains_key(&id) {
            continue;
        }
        if chunks.len() >= 1024 {
            return Err("channel diagnostic exceeded its chunk budget".into());
        }
        let chunk = compiler.chunk(id)?.ok_or("missing actual channel chunk")?;
        chunk.validate()?;
        pending.extend(
            chunk
                .semantics
                .liquids
                .iter()
                .flat_map(|l| &l.downstream)
                .map(|p| p.column.chunk()),
        );
        chunks.insert(id, chunk);
    }
    let graph: BTreeMap<_, _> = chunks
        .values()
        .flat_map(|c| &c.semantics.liquids)
        .map(|l| (l.column, l))
        .collect();
    let mut counts = [0_usize; 3];
    let mut waterfalls = 0;
    let mut completed: [BTreeSet<WorldHex>; 3] = std::array::from_fn(|_| BTreeSet::new());
    for &start in &candidates {
        let Some(liquid) = graph.get(&start).filter(|l| l.body_id == "grand/river") else {
            continue;
        };
        let reach = document.reach(start).ok_or("unclassified channel")?;
        let count = counts.get_mut(reach).ok_or("invalid reach index")?;
        *count += 1;
        waterfalls += usize::from(liquid.kind == LiquidKind::Waterfall);
        let complete = completed.get_mut(reach).ok_or("invalid completion set")?;
        let mut at = start;
        let mut path = BTreeSet::new();
        while !complete.contains(&at) {
            if !path.insert(at) {
                return Err(format!("cycle from {start:?} at {at:?}").into());
            }
            let liquid = graph
                .get(&at)
                .ok_or("missing emitted downstream interval")?;
            let Some(next) = liquid.downstream.first() else {
                if !document.receiver(reach, liquid) {
                    return Err(format!("reach{reach} path from{start:?} stops at unintended standing sink{at:?}, model={:?}, top={}", document.model(at), liquid.top).into());
                }
                break;
            };
            assert_eq!(liquid.downstream.len(), 1, "single authored outflow");
            let target = graph
                .get(&next.column)
                .ok_or("missing exact downstream target chunk")?;
            assert_eq!(at.checked_distance(next.column)?, 1);
            assert_eq!(next.level, target.top - 1);
            assert!(
                target.top <= liquid.top,
                "physical drainage never travels uphill"
            );
            let column = chunks
                .get(&next.column.chunk())
                .and_then(|c| c.columns.iter().find(|c| c.position == next.column))
                .ok_or("missing target geometry")?;
            assert_eq!(column.material_at(next.level), Some("water"));
            at = next.column;
        }
        complete.extend(path);
    }
    assert!(
        counts.into_iter().all(|n| n > 0),
        "rill, falls and river must all execute"
    );
    assert!(
        waterfalls > 0,
        "the real dominant fall must publish falling intervals"
    );
    // Distant and detailed renderers must consume the same emitted flow facts.
    let overview = compiler.overview();
    for water in overview
        .inland_water
        .ok_or("missing inland facts")?
        .chunks
        .iter()
        .flat_map(|c| &c.columns)
    {
        if !candidates.contains(&water.column) {
            continue;
        }
        let detail = graph
            .get(&water.column)
            .ok_or("missing detailed liquid facts")?;
        assert_eq!(
            (water.bottom, water.top, water.kind, water.downstream),
            (
                detail.bottom,
                detail.top,
                detail.kind,
                detail.downstream.first().copied()
            )
        );
    }
    Ok(())
}
