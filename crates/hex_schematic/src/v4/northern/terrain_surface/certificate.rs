//! Independent affine-overlay and native boundary certificate.
use super::{
    build::{Domain, Source, CORNERS},
    invalid, MacroSurface,
};
use hex_world_contracts::{ChunkId, ContractError};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn area(a: [i16; 2], b: [i16; 2], c: [i16; 2]) -> i64 {
    let [ax, az] = a.map(i64::from);
    let [bx, bz] = b.map(i64::from);
    let [cx, cz] = c.map(i64::from);
    (bx - ax) * (cz - az) - (bz - az) * (cx - ax)
}
fn cross([ax, az]: [f64; 2], [bx, bz]: [f64; 2]) -> f64 {
    ax * bz - az * bx
}
fn sub([ax, az]: [f64; 2], [bx, bz]: [f64; 2]) -> [f64; 2] {
    [ax - bx, az - bz]
}
fn corners([q, r]: [i16; 2]) -> [[f64; 2]; 6] {
    CORNERS.map(|[a, b]| [f64::from(3 * q + a), f64::from(3 * r + b)])
}
pub(super) fn inside_hex(p: [f64; 2], cell: [i16; 2]) -> bool {
    let points = corners(cell);
    points
        .iter()
        .copied()
        .zip(points.iter().copied().cycle().skip(1))
        .take(6)
        .all(|(a, b)| cross(sub(b, a), sub(p, a)) <= 1e-9)
}
fn clip(mut polygon: Vec<[f64; 2]>, cell: [i16; 2]) -> Vec<[f64; 2]> {
    let hex = corners(cell);
    for (a, b) in hex
        .iter()
        .copied()
        .zip(hex.iter().copied().cycle().skip(1))
        .take(6)
    {
        let mut result = Vec::new();
        for (p, q) in polygon
            .iter()
            .copied()
            .zip(polygon.iter().copied().cycle().skip(1))
            .take(polygon.len())
        {
            let dp = cross(sub(b, a), sub(p, a));
            let dq = cross(sub(b, a), sub(q, a));
            if dp <= 0.0 {
                result.push(p);
            }
            if (dp < 0.0 && dq > 0.0) || (dp > 0.0 && dq < 0.0) {
                let t = dp / (dp - dq);
                let [px, pz] = p;
                let [qx, qz] = q;
                result.push([px + t * (qx - px), pz + t * (qz - pz)]);
            }
        }
        polygon = result;
        if polygon.is_empty() {
            break;
        }
    }
    polygon
}
fn polygon_area(polygon: &[[f64; 2]]) -> f64 {
    0.5 * polygon
        .iter()
        .copied()
        .zip(polygon.iter().copied().cycle().skip(1))
        .take(polygon.len())
        .map(|(a, b)| cross(a, b))
        .sum::<f64>()
        .abs()
}
fn interpolate(points: [[f64; 2]; 3], levels: [f64; 3], p: [f64; 2]) -> f64 {
    let [a, b, c] = points;
    let [ya, yb, yc] = levels;
    let ab = sub(b, a);
    let ac = sub(c, a);
    let ap = sub(p, a);
    let denominator = cross(ab, ac);
    ya + cross(ap, ac) / denominator * (yb - ya) + cross(ab, ap) / denominator * (yc - ya)
}

pub(super) struct Inspection {
    pub(super) maximum_error: f64,
    pub(super) refine: BTreeSet<[i16; 2]>,
}

pub(super) fn inspect(
    domain: &Domain,
    surface: &MacroSurface,
    level_height: f64,
    tolerance: f64,
) -> Result<Inspection, ContractError> {
    let mut coverage: BTreeMap<[i16; 2], f64> = domain.cells.keys().map(|&c| (c, 0.0)).collect();
    let mut edges = BTreeMap::new();
    let mut double_area = 0_i64;
    let mut maximum_error = 0.0_f64;
    let mut refine = BTreeSet::new();
    for triangle in &surface.triangles {
        let [Some(a), Some(b), Some(c)] = triangle.map(|i| surface.vertices.get(usize::from(i)))
        else {
            return Err(invalid("certificate triangle index"));
        };
        let vertices = [a, b, c];
        let lattice = vertices.map(|v| v.lattice);
        let [pa, pb, pc] = lattice;
        let signed = area(pa, pb, pc);
        if signed >= 0 {
            return Err(invalid(
                "certificate requires nondegenerate upward triangles",
            ));
        }
        double_area -= signed;
        for (a, b) in lattice
            .into_iter()
            .zip(lattice.into_iter().cycle().skip(1))
            .take(3)
        {
            let key = if a < b { [a, b] } else { [b, a] };
            let count = edges.entry(key).or_insert(0_u8);
            if *count >= 2 {
                return Err(invalid("nonmanifold edge incidence"));
            }
            *count += 1;
        }
        let points = lattice.map(|p| p.map(f64::from));
        let heights = vertices.map(|v| f64::from(v.half_level) * level_height * 0.5);
        let [aq, ar] = pa;
        let [bq, br] = pb;
        let [cq, cr] = pc;
        let minq = aq.min(bq).min(cq);
        let maxq = aq.max(bq).max(cq);
        let minr = ar.min(br).min(cr);
        let maxr = ar.max(br).max(cr);
        for (&cell, &top) in &domain.cells {
            let [q, r] = cell;
            if q * 3 + 2 < minq || q * 3 - 2 > maxq || r * 3 + 2 < minr || r * 3 - 2 > maxr {
                continue;
            }
            let polygon = clip(points.to_vec(), cell);
            let area = polygon_area(&polygon);
            if area <= 1e-12 {
                continue;
            }
            *coverage
                .get_mut(&cell)
                .ok_or_else(|| invalid("certificate cell"))? += area;
            // Both functions are affine on each convex intersection. Its vertices
            // include cap corners, TIN vertices, and every crossing of their edges.
            for point in polygon {
                let error =
                    (interpolate(points, heights, point) - f64::from(top) * level_height).abs();
                maximum_error = maximum_error.max(error);
                if error > tolerance + 1e-8 {
                    refine.insert(cell);
                }
            }
        }
    }
    if edges
        .iter()
        .any(|(edge, &count)| count != if domain.boundary.contains(edge) { 1 } else { 2 })
        || domain
            .boundary
            .iter()
            .any(|edge| edges.get(edge) != Some(&1))
    {
        return Err(invalid(
            "missing constraint, nonmanifold edge, or unowned hole",
        ));
    }
    let expected = i64::try_from(domain.cells.len())
        .map_err(|error| invalid(&format!("area count: {error}")))?
        * 18;
    if double_area != expected || coverage.values().any(|value| (value - 9.0).abs() > 1e-7) {
        return Err(invalid(
            "surface does not exactly cover every admitted source hex",
        ));
    }
    Ok(Inspection {
        maximum_error,
        refine,
    })
}

/// Recertify an exported patch against its complete immutable profiles. This
/// rejects holes, missing breaklines, area drift and any exceeded error ceiling.
pub(super) fn certify_patch(
    source: &Source<'_>,
    coordinate: ChunkId,
    surface: &MacroSurface,
    level_height: f64,
) -> Result<f64, ContractError> {
    let overview = source.overview;
    if !level_height.is_finite()
        || level_height <= 0.0
        || !overview.tolerance.is_finite()
        || !(0.0..=2.0).contains(&overview.tolerance)
        || !surface.maximum_error.is_finite()
        || !(0.0..=overview.tolerance + 1e-8).contains(&surface.maximum_error)
        || surface.vertices.len() > super::MAX_PATCH_VERTICES
        || surface.triangles.len() > super::MAX_PATCH_TRIANGLES
    {
        return Err(invalid("invalid certificate level scale"));
    }
    let domain = Domain::new(source, coordinate)?;
    let mut positions = BTreeSet::new();
    for vertex in &surface.vertices {
        let [q, r] = vertex.lattice;
        if !(-2..=47).contains(&q)
            || !(-2..=47).contains(&r)
            || q.rem_euclid(3) != r.rem_euclid(3)
            || !positions.insert(vertex.lattice)
            || domain.vertex(source, vertex.lattice)?.half_level != vertex.half_level
        {
            return Err(invalid("noncanonical or duplicate exported cap vertex"));
        }
    }
    let result = inspect(&domain, surface, level_height, overview.tolerance)?;
    if !result.refine.is_empty() || (result.maximum_error - surface.maximum_error).abs() > 1e-8 {
        return Err(invalid(
            "cap error certificate differs from the emitted surface",
        ));
    }
    Ok(result.maximum_error)
}
