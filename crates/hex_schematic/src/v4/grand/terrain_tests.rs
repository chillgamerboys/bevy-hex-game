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
fn library_exterior_walls_close_every_unintended_roof_side_aperture() {
    let g = plain();
    let mut perimeter = 0;
    for r in -410..=-190 {
        for q in -220..=400 {
            let p = WorldHex::new(q, r);
            let Some((_, ceiling)) = library_cavity(p) else {
                continue;
            };
            if terrain::library_portal(p) {
                continue;
            }
            for (dq, dr) in DIRS {
                let outside = WorldHex::new(q + dq, r + dr);
                if terrain::library_portal(outside) || library_cavity(outside).is_some() {
                    continue;
                }
                let (column, _) = g.column(outside);
                assert!(
                    column.runs.iter().any(|run| run.material != "water"
                        && run.bottom < ceiling
                        && run.top >= ceiling),
                    "open library side from {p:?} below ceiling {ceiling} to {outside:?}"
                );
                perimeter += 1;
            }
        }
    }
    assert!(perimeter > 2000);
    println!("LIBRARY solid exterior roof-side edges={perimeter}");
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

#[test]
fn headwater_reaches_have_shallow_exact_beds_and_retained_banks() {
    let g = plain();
    let mut reaches = BTreeMap::<i32, usize>::new();
    for r in -296..=-124 {
        let z = world_xz(WorldHex::new(0, r))[1];
        // These three narrow strips are intentional exposed waterfall faces.
        if [-370.0, -275.0, -170.0]
            .iter()
            .any(|fall| (z - fall).abs() < 5.0)
        {
            continue;
        }
        let mut width = 0;
        for q in 200..=400 {
            let p = WorldHex::new(q, r);
            if !river_channel(p) {
                continue;
            }
            let (column, liquid) = g.column(p);
            let liquid = liquid.expect("regular headwater publishes an exact liquid interval");
            assert!(matches!(liquid.top, 540 | 620 | 700));
            let water = column
                .runs
                .iter()
                .find(|run| run.material == "water")
                .expect("liquid semantics have corresponding emitted water voxels");
            assert_eq!(water.top, liquid.top);
            assert!(
                (2..=6).contains(&(water.top - water.bottom)),
                "unsupported regular reach at {p:?}: {water:?}"
            );
            for (dq, dr) in DIRS {
                let neighbor = WorldHex::new(q + dq, r + dr);
                let (bank, next_liquid) = g.column(neighbor);
                if next_liquid.is_some() {
                    continue;
                }
                let support = bank.runs.iter().map(|run| run.top).max().unwrap();
                assert!(
                    support >= water.top,
                    "exposed reach side {p:?}:{water:?} beside {neighbor:?} bank {support}"
                );
            }
            width += 1;
            *reaches.entry(liquid.top).or_default() += 1;
        }
        assert!(width >= 10, "headwater narrowed at row {r}: {width} cells");
    }
    assert_eq!(reaches.len(), 3);
    assert!(reaches.values().all(|count| *count > 400));
    for (z, upper, lower) in [(-370.0, 700, 620), (-275.0, 620, 540)] {
        let p = nearest_hex(headwater_center(z), z);
        let (column, _) = g.column(p);
        let water = column
            .runs
            .iter()
            .find(|run| run.material == "water")
            .unwrap();
        assert_eq!(water.top, upper, "named waterfall disappeared at {p:?}");
        assert_eq!(water.bottom, lower - 5, "waterfall bed moved at {p:?}");
    }
    // The last fall meets the irregular receiving-lake shore, which slightly
    // precedes its nominal cut at the centerline. Preserve that localized join.
    let mut last_drop = Vec::new();
    for r in -126..=-100 {
        let z = world_xz(WorldHex::new(0, r))[1];
        let p = nearest_hex(headwater_center(z), z);
        let (_, liquid) = g.column(p);
        let top = liquid.expect("lower headwater joins the valley lake").top;
        if last_drop.last() != Some(&top) {
            last_drop.push(top);
        }
    }
    assert_eq!(last_drop, [540, terrain::VALLEY_TOP]);
}

#[test]
fn source_lake_and_fountain_intake_are_contained_and_connected() {
    let g = plain();
    let mut wet = BTreeSet::new();
    for r in -340..=-292 {
        for q in 280..=400 {
            let p = WorldHex::new(q, r);
            let [x, z] = world_xz(p);
            if !(260.0..=385.0).contains(&x) || z > -438.0 {
                continue;
            }
            let (column, liquid) = g.column(p);
            if !liquid.is_some_and(|liquid| liquid.top == terrain::LAKE_TOP) {
                continue;
            }
            wet.insert(p);
            let water = column
                .runs
                .iter()
                .find(|run| run.material == "water")
                .unwrap();
            assert!(
                water.top - water.bottom <= 10,
                "deep intake at {p:?}: {water:?}"
            );
            for (dq, dr) in DIRS {
                let neighbor = WorldHex::new(q + dq, r + dr);
                let (bank, next_liquid) = g.column(neighbor);
                if next_liquid.is_some() {
                    continue;
                }
                let support = bank.runs.iter().map(|run| run.top).max().unwrap();
                assert!(
                    support >= terrain::LAKE_TOP,
                    "uncontained lake/intake at {p:?} beside {neighbor:?}, bank {support}"
                );
            }
        }
    }
    let intake = nearest_hex(276.0, -474.0);
    let outlet = nearest_hex(headwater_center(-442.5), -442.5);
    assert!(wet.contains(&intake));
    assert!(wet.contains(&outlet));
    let mut reached = BTreeSet::from([intake]);
    let mut queue = VecDeque::from([intake]);
    while let Some(p) = queue.pop_front() {
        for (dq, dr) in DIRS {
            let neighbor = WorldHex::new(p.q + dq, p.r + dr);
            if wet.contains(&neighbor) && reached.insert(neighbor) {
                queue.push_back(neighbor);
            }
        }
    }
    assert!(
        reached.contains(&outlet),
        "fountain intake does not reach the headwater"
    );
    assert_eq!(wet, reached, "source water contains a disconnected pocket");
}

#[test]
fn retained_lake_shore_preserves_the_garden_approach() {
    let g = plain();
    // The final broad Garden ascent skirts the lake's newly retained dry rim.
    // Check parallel ordinary walking lines through that overlap using emitted
    // columns, including the transition onto the shrine's existing support pad.
    for offset in [-2.0, 0.0, 2.0] {
        let mut previous: Option<(WorldHex, i32)> = None;
        for step in 0..=140 {
            let t = f64::from(step) / 140.0;
            let p = nearest_hex(330.0 - 55.0 * t + offset, -535.0 + 45.0 * t);
            let (column, liquid) = g.column(p);
            assert!(
                liquid.is_none(),
                "lake flooded the Garden approach at {p:?}"
            );
            let h = column.runs.iter().map(|run| run.top).max().unwrap();
            if let Some((old_p, old_h)) = previous {
                if old_p != p {
                    assert!(
                        (h - old_h).abs() <= 1,
                        "retained lake rim blocks Garden approach {old_p:?}:{old_h} -> {p:?}:{h}"
                    );
                }
            }
            previous = Some((p, h));
        }
    }
}
