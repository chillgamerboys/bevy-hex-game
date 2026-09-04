//! Checked integer geometry independent of rendering and process scheduling.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use hex_world_contracts::{VoxelPosition, WorldHex};

pub(super) const DIRECTIONS: [(i64, i64); 6] = [(1, 0), (0, 1), (-1, 1), (-1, 0), (0, -1), (1, -1)];

pub(super) fn neighbors(point: WorldHex) -> impl Iterator<Item = WorldHex> {
    DIRECTIONS.into_iter().filter_map(move |(q, r)| {
        Some(WorldHex::new(
            point.q.checked_add(q)?,
            point.r.checked_add(r)?,
        ))
    })
}

pub(super) fn distance(a: WorldHex, b: WorldHex) -> Result<u64, String> {
    a.checked_distance(b).map_err(|error| error.to_string())
}

pub(super) fn disk(center: WorldHex, radius: u32) -> Result<Vec<WorldHex>, String> {
    let radius = i64::from(radius);
    let mut output = Vec::new();
    for q in -radius..=radius {
        for r in (-radius).max(-q - radius)..=radius.min(-q + radius) {
            output.push(
                center
                    .checked_add(WorldHex::new(q, r))
                    .map_err(|error| error.to_string())?,
            );
        }
    }
    Ok(output)
}

pub(super) fn ring(center: WorldHex, radius: u32) -> Result<Vec<WorldHex>, String> {
    if radius == 0 {
        return Ok(vec![center]);
    }
    let mut output = Vec::new();
    let radius = i64::from(radius);
    for q in -radius..=radius {
        let low = (-radius).max(-q - radius);
        let high = radius.min(-q + radius);
        if q.abs() == radius {
            for r in low..=high {
                output.push(
                    center
                        .checked_add(WorldHex::new(q, r))
                        .map_err(|error| error.to_string())?,
                );
            }
        } else {
            for r in [low, high] {
                output.push(
                    center
                        .checked_add(WorldHex::new(q, r))
                        .map_err(|error| error.to_string())?,
                );
            }
        }
    }
    Ok(output)
}

/// Nearest integer cube-coordinate rasterization with canonical tie breaking.
pub(super) fn line(start: WorldHex, end: WorldHex) -> Result<Vec<WorldHex>, String> {
    let count = distance(start, end)?;
    if count > 1_000_000 {
        return Err("authored path segment exceeds one million columns".to_owned());
    }
    if count == 0 {
        return Ok(vec![start]);
    }
    let n = i128::from(count);
    let mut output = Vec::new();
    for step in 0..=count {
        let step = i128::from(step);
        let q = i128::from(start.q) * (n - step) + i128::from(end.q) * step;
        let r = i128::from(start.r) * (n - step) + i128::from(end.r) * step;
        let q0 = q.div_euclid(n);
        let r0 = r.div_euclid(n);
        let mut best = None;
        for cq in [q0, q0 + 1] {
            for cr in [r0, r0 + 1] {
                let dq = q - cq * n;
                let dr = r - cr * n;
                let score = dq * dq + dr * dr + (dq + dr) * (dq + dr);
                let candidate = (score, cq, cr);
                if best.is_none_or(|old| candidate < old) {
                    best = Some(candidate);
                }
            }
        }
        let (_, q, r) = best.ok_or_else(|| "empty line rasterization".to_owned())?;
        let point = WorldHex::new(
            i64::try_from(q).map_err(|error| error.to_string())?,
            i64::try_from(r).map_err(|error| error.to_string())?,
        );
        if output.last() != Some(&point) {
            output.push(point);
        }
    }
    Ok(output)
}

pub(super) fn polyline(points: &[WorldHex]) -> Result<Vec<WorldHex>, String> {
    let mut output = Vec::new();
    for pair in points.windows(2) {
        if let [start, end] = pair {
            let segment = line(*start, *end)?;
            let skip = usize::from(!output.is_empty());
            output.extend(segment.into_iter().skip(skip));
        }
    }
    if points.len() == 1 {
        output.extend(points.iter().copied());
    }
    Ok(output)
}

pub(super) fn ribbon(points: &[WorldHex], radius: u32) -> Result<BTreeSet<WorldHex>, String> {
    let mut output = BTreeSet::new();
    for point in points {
        output.extend(disk(*point, radius)?);
    }
    Ok(output)
}

pub(super) fn distances(mask: &BTreeSet<WorldHex>, start: WorldHex) -> BTreeMap<WorldHex, u32> {
    let mut result = BTreeMap::new();
    if !mask.contains(&start) {
        return result;
    }
    result.insert(start, 0_u32);
    let mut queue = VecDeque::from([start]);
    while let Some(point) = queue.pop_front() {
        let next = result
            .get(&point)
            .copied()
            .unwrap_or_default()
            .saturating_add(1);
        for neighbor in neighbors(point) {
            if mask.contains(&neighbor) && !result.contains_key(&neighbor) {
                result.insert(neighbor, next);
                queue.push_back(neighbor);
            }
        }
    }
    result
}

/// Extracted from Grand's small generic corridor operator, retaining its exact
/// endpoint and one-Lipschitz guarantees without its surrounding scenario logic.
pub(super) fn grade(
    mask: &BTreeSet<WorldHex>,
    start: VoxelPosition,
    end: VoxelPosition,
) -> Result<BTreeMap<WorldHex, i32>, String> {
    let a = distances(mask, start.column);
    let b = distances(mask, end.column);
    if a.len() != mask.len()
        || b.len() != mask.len()
        || a.get(&end.column)
            .is_none_or(|d| *d < start.level.abs_diff(end.level))
    {
        return Err(
            "corridor cannot preserve its endpoint levels with ordinary one-level steps".to_owned(),
        );
    }
    let mut result = BTreeMap::new();
    for point in mask {
        let da = i32::try_from(a.get(point).copied().unwrap_or_default())
            .map_err(|error| error.to_string())?;
        let db = i32::try_from(b.get(point).copied().unwrap_or_default())
            .map_err(|error| error.to_string())?;
        let lower = start
            .level
            .saturating_sub(da)
            .max(end.level.saturating_sub(db));
        let upper = start
            .level
            .saturating_add(da)
            .min(end.level.saturating_add(db));
        if lower > upper {
            return Err("contradictory corridor grade envelopes".to_owned());
        }
        result.insert(
            *point,
            lower.saturating_add(upper.saturating_sub(lower) / 2),
        );
    }
    Ok(result)
}
