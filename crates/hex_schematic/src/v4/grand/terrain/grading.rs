//! Quantized broad mountain treads, composed before objects or package facts.
//!
//! Projected polyline grades disagree across turn bisectors. Solve the entire
//! authored tread as a hex height field instead: one voxel per neighboring cell,
//! preserving shrine/entrance datums and required underground cover. The original
//! relief supplies the preferred shape, and remains authoritative outside the
//! adjoining shoulders. This is compiler work; no runtime terrain is retained.
use super::*;
use std::cmp::Reverse;
use std::collections::{BTreeSet, BinaryHeap};

fn neighbors(p: WorldHex) -> impl Iterator<Item = WorldHex> {
    DIRS.into_iter()
        .map(move |(q, r)| WorldHex::new(p.q + q, p.r + r))
}

fn tread_distance(x: f64, z: f64, island: bool) -> f64 {
    if island {
        let width = 7. + 7. * smooth((x + 1160.) / 30.);
        nearest_grade(x, z, VOLCANO_ASCENT).map_or(f64::INFINITY, |v| v.0 - width)
    } else {
        // Broad 40-unit western/Crystal shoulders; the western upper approach
        // narrows to 16 units between the immutable coast and the library vault.
        // The northern Crystal approach remains the full 40 units to Air.
        let west_width = 20. - 12. * smooth((-z - 470.) / 50.);
        let western =
            nearest_grade(x, z, WESTERN_ASCENT).map_or(f64::INFINITY, |v| v.0 - west_width);
        let crystal = nearest_grade(x, z, CRYSTAL_SHOULDER).map_or(f64::INFINITY, |v| v.0 - 20.);
        western.min(crystal)
    }
}

fn required_cover(g: &GrandCompiler, p: WorldHex) -> i32 {
    let library = library_cavity(p).map_or(0, |(floor, ceiling)| {
        if library_portal(p) {
            floor + 1
        } else {
            ceiling + 8
        }
    });
    library
        .max(shadow_cavity(p).map_or(0, |(floor, _)| floor + 1))
        .max(g.cave_cover.get(&p).copied().unwrap_or(0))
}

fn fixed_datum(p: WorldHex, island: bool) -> Option<i32> {
    let [x, z] = world_xz(p);
    for pad in sites::PADS {
        if (pad.x < -1000.) == island
            && (island || pad.x < 0. && pad.z < -400.)
            && (x - pad.x).hypot(z - pad.z) <= pad.radius
        {
            return Some(pad.level + 1);
        }
    }
    let pins: &[Pin] = if island {
        &[[-1099., 465., 401.]]
    } else {
        &[[-170., -115., 505.], [-105., -618., 592.]]
    };
    pins.iter()
        .find_map(|&[px, pz, h]| (p == nearest_hex(px, pz)).then_some(h as i32))
}

pub(in super::super) fn compile_grades(
    g: &GrandCompiler,
) -> Result<BTreeMap<WorldHex, i32>, ContractError> {
    let mut result = BTreeMap::new();
    for island in [false, true] {
        let mut preferred = BTreeMap::new();
        let mut core = BTreeSet::new();
        let (q0, q1, r0, r1) = if island {
            (-930, -730, 210, 365)
        } else {
            (-390, 290, -480, -45)
        };
        for r in r0..=r1 {
            for q in q0..=q1 {
                let p = WorldHex::new(q, r);
                let [x, z] = world_xz(p);
                let distance = tread_distance(x, z, island);
                let datum = fixed_datum(p, island);
                if distance > if island { 14. } else { 44. } && datum.is_none() {
                    continue;
                }
                let land = if island {
                    ((x + 1180.) / 86.).hypot((z - 450.) / 77.) < 1.
                } else {
                    grid_value(&g.coast, p, 0) > 0
                };
                if !land {
                    continue;
                }
                let s = raw_surface(g, p);
                if s.water.is_some() {
                    continue;
                }
                preferred.insert(p, s.level + 1);
                if distance <= 0. || datum.is_some() {
                    core.insert(p);
                }
            }
        }
        let mut ceiling: BTreeMap<_, _> = core.iter().map(|&p| (p, i32::MAX)).collect();
        let mut queue = BinaryHeap::new();
        for &p in &core {
            if let Some(h) = fixed_datum(p, island) {
                ceiling.insert(p, h);
                queue.push(Reverse((h, p)));
            }
        }
        while let Some(Reverse((h, p))) = queue.pop() {
            if ceiling.get(&p) != Some(&h) {
                continue;
            }
            for n in neighbors(p) {
                if let Some(next) = ceiling.get_mut(&n) {
                    if h + 1 < *next {
                        *next = h + 1;
                        queue.push(Reverse((h + 1, n)));
                    }
                }
            }
        }
        let mut heights = BTreeMap::new();
        let mut queue = BinaryHeap::new();
        for (&p, &limit) in &ceiling {
            let raw = preferred
                .get(&p)
                .copied()
                .ok_or_else(|| ContractError::new("grand.grade", "missing preferred tread"))?;
            let wanted = fixed_datum(p, island).unwrap_or(raw);
            let floor = required_cover(g, p).max(fixed_datum(p, island).unwrap_or(0));
            if limit == i32::MAX || floor > limit {
                return Err(ContractError::new(
                    "grand.grade",
                    format!(
                        "incompatible tread datum/cover at {p:?}: floor {floor}, upper {limit}"
                    ),
                ));
            }
            let h = wanted.min(limit).max(floor);
            heights.insert(p, h);
            queue.push((h, p));
        }
        // The least one-Lipschitz majorant of preferred heights. The propagated
        // datum ceiling is itself one-Lipschitz, so this cannot exceed it.
        while let Some((h, p)) = queue.pop() {
            if heights.get(&p) != Some(&h) {
                continue;
            }
            for n in neighbors(p) {
                if let Some(next) = heights.get_mut(&n) {
                    if h - 1 > *next {
                        *next = h - 1;
                        queue.push((h - 1, n));
                    }
                }
            }
        }
        // Extend continuous grade bounds into the broad adjoining shoulders.
        // Propagating the bounds (not a nearest-cell delta) avoids moving the
        // old turn discontinuity to the tread's edge.
        let (upper, lower) = shoulder_bounds(&heights, &preferred);
        for (&p, &raw) in &preferred {
            if let Some(&h) = heights.get(&p) {
                result.insert(p, h);
                continue;
            }
            let (Some(&hi), Some(&lo)) = (upper.get(&p), lower.get(&p)) else {
                continue;
            };
            let bounded = raw.clamp(lo, hi);
            let [x, z] = world_xz(p);
            let outer = if island { 14. } else { 44. };
            let weight = smooth(tread_distance(x, z, island) / outer);
            let h = (f64::from(bounded) + f64::from(raw - bounded) * weight).round() as i32;
            result.insert(p, h.max(required_cover(g, p)));
        }
    }
    Ok(result)
}

fn shoulder_bounds(
    core: &BTreeMap<WorldHex, i32>,
    domain: &BTreeMap<WorldHex, i32>,
) -> (BTreeMap<WorldHex, i32>, BTreeMap<WorldHex, i32>) {
    // Extend the nearest *composed* tread grade. Propagating every high summit
    // globally through the outer slopes would incorrectly raise the low beach
    // where successive switchbacks are close in plan view.
    let mut upper = core.clone();
    let mut lower = core.clone();
    let mut queue: VecDeque<_> = core.keys().copied().collect();
    while let Some(p) = queue.pop_front() {
        let (Some(&hi), Some(&lo)) = (upper.get(&p), lower.get(&p)) else {
            continue;
        };
        for n in neighbors(p) {
            if !domain.contains_key(&n) || upper.contains_key(&n) {
                continue;
            }
            upper.insert(n, hi + 1);
            lower.insert(n, lo - 1);
            queue.push_back(n);
        }
    }
    (upper, lower)
}

/// Solid collar outside the cavity footprint. Protecting just the interior roof
/// leaves horizontal holes when the exterior ground ends below its underside.
pub(in super::super) fn compile_cave_cover() -> BTreeMap<WorldHex, i32> {
    let mut cover = BTreeMap::new();
    for r in -410..=-190 {
        for q in -220..=400 {
            let p = WorldHex::new(q, r);
            let Some((_, ceiling)) = library_cavity(p) else {
                continue;
            };
            if library_portal(p) {
                continue;
            }
            for dq in -2_i64..=2 {
                for dr in -2_i64..=2 {
                    let distance = dq.abs().max(dr.abs()).max((dq + dr).abs());
                    if distance > 2 {
                        continue;
                    }
                    let n = WorldHex::new(q + dq, r + dr);
                    if library_portal(n) {
                        continue;
                    }
                    let h = ceiling + 8 - distance as i32;
                    let entry = cover.entry(n).or_insert(h);
                    *entry = (*entry).max(h);
                }
            }
        }
    }
    cover
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_mountain_treads_preserve_datums_cover_and_one_level_adjacency() {
        let mut source: GrandSpec = ron::from_str(include_str!(
            "../../../../../../assets/config/v4/grand-v4/world.ron"
        ))
        .unwrap();
        source.full_dressing = false;
        let g = GrandCompiler::new(source).unwrap();
        let mut core_columns = 0;
        let mut edges = 0;
        let mut changed = 0;
        let mut delta_min = 0;
        let mut delta_max = 0;
        for (&p, &h) in &g.graded_shoulders {
            let raw = raw_surface(&g, p).level + 1;
            changed += usize::from(raw != h);
            delta_min = delta_min.min(h - raw);
            delta_max = delta_max.max(h - raw);
            assert!(
                h >= required_cover(&g, p),
                "buried route uncovered at {p:?}"
            );
            let [x, z] = world_xz(p);
            let island = x < -1000.;
            if let Some(datum) = fixed_datum(p, island) {
                assert_eq!(h, datum, "site datum moved at {p:?}");
            }
            if tread_distance(x, z, island) > 0. {
                continue;
            }
            core_columns += 1;
            for n in neighbors(p) {
                let [nx, nz] = world_xz(n);
                if tread_distance(nx, nz, island) > 0. {
                    continue;
                }
                if let Some(&next) = g.graded_shoulders.get(&n) {
                    assert!(
                        (next - h).abs() <= 1,
                        "tread barrier {p:?}:{h} -> {n:?}:{next}"
                    );
                    edges += 1;
                }
            }
        }
        println!("SHOULDERS core_columns={core_columns} edges={edges} composed={} changed={changed} delta={delta_min}..{delta_max}", g.graded_shoulders.len());
        assert!(core_columns > 20_000);
        assert!(edges > 100_000);
    }
}
