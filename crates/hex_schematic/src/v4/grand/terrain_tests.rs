use super::*;
use std::collections::{BTreeMap, BTreeSet};

fn plain() -> GrandCompiler {
    let mut source: GrandSpec = ron::from_str(include_str!(
        "../../../../../assets/config/v4/grand-v4/world.ron"
    ))
    .unwrap();
    source.full_dressing = false;
    GrandCompiler::new(source).unwrap()
}

#[test]
fn ordinary_lowland_is_a_broad_connected_landscape() {
    let g = plain();
    let mut ground = BTreeMap::new();
    for &(r, a, b) in &g.source.mainland_rows {
        if r < -100 {
            continue;
        }
        for q in a..=b {
            let p = WorldHex::new(q, r);
            let s = g.surface(p);
            if s.water.is_none() && s.level >= SEA_TOP {
                ground.insert(p, s.level);
            }
        }
    }
    let mut edges = 0;
    let mut walkable = 0;
    for (p, h) in &ground {
        for (q, r) in DIRS {
            if let Some(other) = ground.get(&WorldHex::new(p.q + q, p.r + r)) {
                edges += 1;
                walkable += usize::from((h - other).abs() <= 1);
            }
        }
    }
    let mut unseen: BTreeSet<_> = ground.keys().copied().collect();
    let mut components = vec![];
    while let Some(start) = unseen.pop_first() {
        let mut queue = VecDeque::from([start]);
        let mut size = 0;
        while let Some(p) = queue.pop_front() {
            size += 1;
            let h = ground
                .get(&p)
                .expect("queued terrain belongs to the surveyed field");
            for (q, r) in DIRS {
                let next = WorldHex::new(p.q + q, p.r + r);
                if ground
                    .get(&next)
                    .is_some_and(|other| (h - other).abs() <= 1)
                    && unseen.remove(&next)
                {
                    queue.push_back(next);
                }
            }
        }
        components.push(size);
    }
    components.sort_unstable_by(|a, b| b.cmp(a));
    let largest = components.first().copied().unwrap();
    let second = components.get(1).copied().unwrap_or(0);
    println!(
        "LOWLAND columns={} walkable_edges={walkable}/{edges} largest={largest} second={second} components={}",
        ground.len(),
        components.len()
    );
    // The river intentionally separates two broad ordinary-walkable banks.
    assert!(
        walkable * 100 >= edges * 96,
        "ordinary hills need broad usable slopes"
    );
    assert!(
        (largest + second) * 100 >= ground.len() * 96,
        "lowland fragmented into isolated hilltops"
    );
}

#[test]
fn library_is_covered_by_one_massif_not_exposed_stair_strips() {
    let g = plain();
    let mut covered = 0;
    for r in -369..=-200 {
        for q in -210..=310 {
            let p = WorldHex::new(q, r);
            if terrain::library_portal(p) {
                continue;
            }
            let Some((floor, ceiling)) = library_cavity(p) else {
                continue;
            };
            if floor < terrain::LIBRARY_FLOOR {
                continue;
            }
            let roof = g.surface(p).level;
            assert!(
                roof >= ceiling,
                "exposed library roof at {p:?}: {floor}..{ceiling}, surface {roof}"
            );
            covered += 1;
        }
    }
    assert!(covered > 5000);
}

#[test]
fn eastern_volcano_landing_has_wide_shallow_walkout() {
    let g = plain();
    for z in [459., 463.5, 468., 471.] {
        let mut last: Option<i32> = None;
        for x in (-1128..=-1097).rev() {
            let p = nearest_hex(f64::from(x), z);
            let [px, pz] = world_xz(p);
            if ((px + 1180.) / 86.).hypot((pz - 450.) / 77.) >= 1. {
                continue;
            }
            let h = g.surface(p).level;
            if let Some(old) = last {
                assert!((h - old).abs() <= 1, "shore step {old}->{h} at {p:?}");
            }
            last = Some(h);
        }
    }
}

#[test]
fn export_plain_relief_when_requested() {
    let Ok(path) = std::env::var("HEX_GRAND_TERRAIN_REVIEW") else {
        return;
    };
    let g = plain();
    let overview = g.overview();
    std::fs::write(path, ron::to_string(&overview).unwrap()).unwrap();
}

#[test]
fn island_landing_turn_has_a_broad_connected_surface() {
    let g = plain();
    // The former rectangular apron edge crossed this entire first turn. Probe
    // parallel bands around the authored center, not one fortunate grid line.
    let (sx, sz) = (-1118.0_f64, 475.0_f64);
    let (ex, ez) = (-1131.0_f64, 502.0_f64);
    let dx = ex - sx;
    let dz = ez - sz;
    let length = dx.hypot(dz);
    for offset in [-4.0, -2.0, 0.0, 2.0, 4.0] {
        let mut previous: Option<(WorldHex, i32)> = None;
        for step in 0..=60 {
            let t = f64::from(step) / 60.0;
            let p = nearest_hex(
                sx + dx * t - dz / length * offset,
                sz + dz * t + dx / length * offset,
            );
            let h = g.surface(p).level;
            if let Some((old_p, old_h)) = previous {
                if old_p != p {
                    assert!(
                        (h - old_h).abs() <= 1,
                        "landing turn barrier {old_p:?}:{old_h} -> {p:?}:{h}, band {offset}"
                    );
                }
            }
            previous = Some((p, h));
        }
    }
}

#[test]
fn shadow_outlet_flat_landing_joins_crystal_shoulder_without_a_lip() {
    let g = plain();
    for offset in [-2.0, 0.0, 2.0] {
        let mut previous: Option<(WorldHex, i32)> = None;
        for step in 0..=60 {
            let t = f64::from(step) / 60.0;
            let p = nearest_hex(-105.0 + 35.0 * t, -618.0 - 15.0 * t + offset);
            let h = g.surface(p).level;
            if let Some((old_p, old_h)) = previous {
                if old_p != p {
                    assert!(
                        (h - old_h).abs() <= 1,
                        "outlet lip {old_p:?}:{old_h} -> {p:?}:{h}, band {offset}"
                    );
                }
            }
            previous = Some((p, h));
        }
    }
}
