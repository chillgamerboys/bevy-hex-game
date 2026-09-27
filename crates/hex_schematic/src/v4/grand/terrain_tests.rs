//! Exact emitted-column invariants for the canonical geography. Route names and
//! authoring frames replace the former version's absolute coordinate probes.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

fn plain() -> &'static GrandCompiler {
    tests::compiler(false)
}

fn neighbors(p: WorldHex) -> impl Iterator<Item = WorldHex> {
    DIRS.into_iter()
        .map(move |(q, r)| WorldHex::new(p.q + q, p.r + r))
}

fn solid_top(column: &ColumnData) -> Option<i32> {
    column
        .runs
        .iter()
        .filter(|r| r.material != "water")
        .map(|r| r.top)
        .max()
}

fn support_component(
    supports: &BTreeSet<VoxelPosition>,
    start: VoxelPosition,
) -> BTreeSet<VoxelPosition> {
    let mut reached = BTreeSet::from([start]);
    let mut queue = VecDeque::from([start]);
    while let Some(p) = queue.pop_front() {
        for column in neighbors(p.column) {
            for level in p.level - 1..=p.level + 1 {
                let next = VoxelPosition { column, level };
                if supports.contains(&next) && reached.insert(next) {
                    queue.push_back(next);
                }
            }
        }
    }
    reached
}

fn inside_lake(g: &GrandCompiler, lake: &geography::Lake, p: WorldHex) -> bool {
    geography::irregular(g.geography.model_xz(p), lake.center, lake.radii, lake.phase) < 1.
}

fn lake_domain(g: &GrandCompiler, lake: &geography::Lake) -> BTreeSet<WorldHex> {
    let center = g.geography.world_hex(lake.center);
    let [rx, rz] = lake.radii;
    let radius = (g.geography.length(rx.hypot(rz) * 1.4) / 1.5).ceil() as i64 + 2;
    let mut domain = BTreeSet::new();
    for q in -radius..=radius {
        for r in -radius..=radius {
            let p = WorldHex::new(center.q + q, center.r + r);
            if geography::ellipse(g.geography.model_xz(p), lake.center, lake.radii) < 1.4 {
                domain.insert(p);
            }
        }
    }
    domain
}

#[test]
fn ordinary_lowland_is_a_broad_connected_landscape() {
    let g = plain();
    let mut ground = BTreeMap::new();
    for &(r, a, b) in &g.source.mainland_rows {
        for q in a..=b {
            let p = WorldHex::new(q, r);
            // Survey the complete authored lowland forest region, including its
            // glades. Mountain and sea boundaries do not define ordinary hills.
            if g.geography.forest_density(p) <= 0. {
                continue;
            }
            let (column, liquid) = g.column(p);
            if let Some(top) = solid_top(&column).filter(|top| {
                *top >= SEA_TOP && liquid.as_ref().is_none_or(|water| water.top <= *top)
            }) {
                ground.insert(p, top);
            }
        }
    }
    assert!(ground.len() > 50_000, "survey must cover a broad landscape");
    let mut edges = 0;
    let mut walkable = 0;
    for (p, h) in &ground {
        for next in neighbors(*p) {
            if let Some(other) = ground.get(&next) {
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
            let h = ground.get(&p).expect("surveyed ground");
            for next in neighbors(p) {
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
    let largest = components.first().copied().expect("survey is nonempty");
    let second = components.get(1).copied().unwrap_or(0);
    println!("LOWLAND columns={} walkable_edges={walkable}/{edges} largest={largest} second={second} components={}", ground.len(), components.len());
    // Retain the earlier broad-hill usability standard. The river may divide
    // the field into two banks; this does not claim controller traversal proof.
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
    for (p, layers) in &g.layered.columns {
        let Some(ceiling) = layers
            .iter()
            .filter(|l| {
                !l.open
                    && matches!(
                        l.layer,
                        SupportLayer::LibraryLower | SupportLayer::LibraryUpper
                    )
            })
            .map(|l| l.ceiling)
            .max()
        else {
            continue;
        };
        // Named entrance terminals intentionally open to the surface. A higher
        // stacked room may contain an earlier room's nominal ceiling.
        if layers.iter().any(|l| l.open && l.top <= ceiling) {
            continue;
        }
        let roof = layers
            .iter()
            .filter(|l| !l.open && l.top <= ceiling)
            .map(|l| l.ceiling)
            .max()
            .unwrap_or(ceiling);
        let (column, _) = g.column(*p);
        assert!(
            column.material_at(roof).is_some_and(|m| m != "water"),
            "exposed library roof at {p:?}, level {roof}"
        );
        covered += 1;
    }
    assert!(
        covered > 5000,
        "both library halls and their connecting galleries need real cover"
    );
    println!("LIBRARY covered columns={covered}");
}

#[test]
fn library_exterior_walls_close_every_unintended_roof_side_aperture() {
    let g = plain();
    let mut perimeter = 0;
    for (p, layers) in &g.layered.columns {
        for layer in layers.iter().filter(|l| {
            !l.open
                && matches!(
                    l.layer,
                    SupportLayer::LibraryLower | SupportLayer::LibraryUpper
                )
        }) {
            for outside in neighbors(*p) {
                if g.layered.columns.get(&outside).is_some_and(|other| {
                    other
                        .iter()
                        .any(|l| l.top < layer.ceiling && (l.open || l.ceiling > layer.top))
                }) {
                    continue; // Declared adjacent room, stair, or entrance.
                }
                let (column, _) = g.column(outside);
                assert!(
                    column
                        .material_at(layer.ceiling - 1)
                        .is_some_and(|m| m != "water"),
                    "open library side from {p:?} below ceiling {} to {outside:?}",
                    layer.ceiling
                );
                perimeter += 1;
            }
        }
    }
    assert!(
        perimeter > 0,
        "the closed library must have an exterior perimeter"
    );
    println!("LIBRARY solid exterior roof-side edges={perimeter}");
}

#[test]
fn volcano_landing_has_wide_shallow_walkout() {
    let g = plain();
    let route = g
        .layered
        .routes
        .iter()
        .find(|r| r.id == "volcano_ascent")
        .expect("volcano route");
    let start = *route.supports.first().expect("landing");
    let approach: BTreeSet<_> = route
        .ribbon
        .iter()
        .copied()
        .filter(|s| {
            s.column
                .checked_distance(start.column)
                .is_ok_and(|d| d <= 12)
        })
        .collect();
    assert!(approach.len() > 100, "landing must be a broad apron");
    for support in &approach {
        assert!(
            g.clear_support(*support, 8),
            "landing body clearance {support:?}"
        );
        assert!(
            g.column(support.column)
                .1
                .is_none_or(|l| l.top <= support.level + 1),
            "landing is above the water"
        );
    }
    let reached = support_component(&approach, start);
    assert_eq!(reached, approach, "landing apron fragmented by a high step");
}

#[test]
fn export_plain_relief_when_requested() {
    let Ok(path) = std::env::var("HEX_GRAND_TERRAIN_REVIEW") else {
        return;
    };
    let overview = plain().overview();
    std::fs::write(
        path,
        ron::to_string(&overview).expect("overview serializes"),
    )
    .expect("review destination");
}

#[test]
fn island_landing_turn_has_a_broad_connected_surface() {
    let g = plain();
    let d = g.geography.document.as_ref().expect("geography");
    let route = g
        .layered
        .routes
        .iter()
        .find(|r| r.id == "volcano_ascent")
        .expect("volcano route");
    let [cx, cz] = d.volcano.center;
    let points: Vec<_> = d
        .volcano_route
        .local_points
        .iter()
        .take(3)
        .map(|&[x, y, z]| [cx + x, y, cz + z])
        .collect();
    let centerline: BTreeSet<_> = tests::line_columns(g, &points).into_iter().collect();
    let width = g.geography.length(d.volcano_route.width * 0.5);
    let ribbon: BTreeSet<_> = route
        .ribbon
        .iter()
        .copied()
        .filter(|s| {
            let [x, z] = world_xz(s.column);
            centerline.iter().any(|p| {
                let [px, pz] = world_xz(*p);
                (x - px).hypot(z - pz) <= width
            })
        })
        .collect();
    assert!(
        ribbon.len() > centerline.len() * 5,
        "first turn needs usable width"
    );
    for s in &ribbon {
        assert!(g.clear_support(*s, 8), "first turn clearance {s:?}");
    }
    let start = *route.supports.first().expect("landing");
    assert_eq!(
        support_component(&ribbon, start),
        ribbon,
        "first turn contains a disconnected tread"
    );
}

#[test]
fn shadow_outlet_flat_landing_joins_crystal_shoulder_without_a_lip() {
    let g = plain();
    let shadow = g
        .layered
        .routes
        .iter()
        .find(|r| r.id == "shadow_tunnel")
        .expect("Shadow route");
    let crystal = g
        .layered
        .routes
        .iter()
        .find(|r| r.id == "crystal_ascent")
        .expect("Crystal route");
    let exit = *shadow.supports.last().expect("exit");
    assert_eq!(
        Some(&exit),
        crystal.supports.first(),
        "the tunnel meets the actual bottom tread"
    );
    let landing: BTreeSet<_> = shadow
        .ribbon
        .iter()
        .chain(&crystal.ribbon)
        .copied()
        .filter(|s| {
            s.column.checked_distance(exit.column).is_ok_and(|d| d <= 5)
                && (s.level - exit.level).abs() <= 8
        })
        .collect();
    assert!(landing.len() > 30, "the join has body width");
    for s in &landing {
        assert!(g.clear_support(*s, 8), "landing clearance {s:?}");
    }
    assert_eq!(
        support_component(&landing, exit),
        landing,
        "outlet lip separates tunnel and bottom stair"
    );
}

#[test]
fn headwater_reaches_have_shallow_exact_beds_and_retained_banks() {
    let g = plain();
    let d = g.geography.document.as_ref().expect("geography");
    let mut surveyed = 0;
    let mut banks = 0;
    for channel in [&d.falls, &d.river] {
        let radius = (g.geography.length(channel.width * 0.5) / 1.5).ceil() as i64 + 2;
        let mut candidates = BTreeSet::new();
        for center in tests::line_columns(g, &channel.points) {
            for q in -radius..=radius {
                for r in -radius..=radius {
                    candidates.insert(WorldHex::new(center.q + q, center.r + r));
                }
            }
        }
        for p in candidates {
            let point = g.geography.model_xz(p);
            let (distance, _, _) = geography::route_distance(point, &channel.points);
            if distance >= channel.width * 0.5 {
                continue;
            }
            // Preserve the old distinction between regular reaches and genuine
            // falls. A plunge exposes a vertical water face by design.
            let near_drop = channel.points.windows(2).any(|pair| {
                let [a, b] = pair else { return false };
                let [ax, ay, az] = *a;
                let [bx, by, bz] = *b;
                ay - by > 10.
                    && (ax - bx).hypot(az - bz) < (ay - by) * 0.5
                    && geography::segment(point, [ax, az], [bx, bz]).0 < channel.width * 2.
            });
            if near_drop || inside_lake(g, &d.upper_lake, p) || inside_lake(g, &d.lower_lake, p) {
                continue;
            }
            let (column, liquid) = g.column(p);
            let water = liquid.expect("regular reach stays wet");
            if water.top <= SEA_TOP {
                continue;
            } // The receiving sea is not a shallow river bed.
            assert!(
                column
                    .material_at(water.bottom - 1)
                    .is_some_and(|m| m != "water"),
                "supported bed at {p:?}"
            );
            assert!(
                (1..=g.geography.top_level(4.) - SEA_TOP).contains(&(water.top - water.bottom)),
                "unsupported regular reach at {p:?}: {water:?}"
            );
            for neighbor in neighbors(p) {
                let (bank, next_water) = g.column(neighbor);
                if next_water.is_some() {
                    continue;
                }
                let top = solid_top(&bank).expect("bank terrain");
                assert!(
                    top >= water.top,
                    "exposed regular water side {p:?}:{water:?} beside {neighbor:?}, bank {top}"
                );
                banks += 1;
            }
            surveyed += 1;
        }
    }
    assert!(
        surveyed > 1000 && banks > 100,
        "survey includes the broad channel and its actual banks"
    );
    println!("HEADWATER regular columns={surveyed} retained bank edges={banks}");
}

#[test]
fn source_lake_and_fountain_intake_are_contained_and_connected() {
    let g = plain();
    let d = g.geography.document.as_ref().expect("geography");
    let domain = lake_domain(g, &d.upper_lake);
    let wet: BTreeMap<_, _> = domain
        .iter()
        .filter_map(|p| g.column(*p).1.map(|l| (*p, l)))
        .collect();
    let intake = g.geography.world_hex(d.fountain_basin.center);
    let &[x, _, z] = d.falls.points.first().expect("lake outlet");
    let outlet = g.geography.world_hex([x, z]);
    assert!(wet.contains_key(&intake) && wet.contains_key(&outlet));
    let mut reached = BTreeSet::from([intake]);
    let mut queue = VecDeque::from([intake]);
    while let Some(p) = queue.pop_front() {
        let water = wet.get(&p).expect("wet queue");
        for neighbor in neighbors(p) {
            if wet
                .get(&neighbor)
                .is_some_and(|next| water.bottom < next.top && next.bottom < water.top)
                && reached.insert(neighbor)
            {
                queue.push_back(neighbor);
            }
        }
    }
    assert!(
        reached.contains(&outlet),
        "the real fountain liquid cannot reach the upper fall intake"
    );
    assert_eq!(
        reached.len(),
        wet.len(),
        "source water contains a disconnected pocket"
    );
    let lake_top = g.geography.top_level(d.upper_lake.level);
    let mut banks = 0;
    for (p, water) in &wet {
        if water.top != lake_top || !inside_lake(g, &d.upper_lake, *p) {
            continue;
        }
        for neighbor in neighbors(*p) {
            let (bank, liquid) = g.column(neighbor);
            if liquid.is_some() {
                continue;
            }
            assert!(
                solid_top(&bank).is_some_and(|h| h >= lake_top),
                "uncontained source lake at {p:?} beside {neighbor:?}"
            );
            banks += 1;
        }
    }
    assert!(banks > 0, "actual shoreline was surveyed");
}

#[test]
fn frozen_shore_preserves_a_water_separated_garden_island() {
    let g = plain();
    let d = g.geography.document.as_ref().expect("geography");
    let frozen = g
        .layered
        .routes
        .iter()
        .find(|r| r.id == "frozen_shore")
        .expect("Frozen shore route");
    let shore = *frozen.supports.last().expect("shore landing");
    assert!(
        g.clear_support(shore, 8),
        "the ordinary mountain route reaches dry shore"
    );
    assert!(g
        .column(shore.column)
        .1
        .is_none_or(|l| l.top <= shore.level + 1));
    let garden = g.geography.frame("shrine_water").expect("garden shrine");
    let court = g.support_at(&garden, [0., 0.]).expect("garden court");
    assert!(g.clear_support(court, 8));
    // The former dry Garden approach is intentionally superseded: this is an
    // island in the enclosed lake. Flood-fill actual land, not the ellipse mask.
    let domain = lake_domain(g, &d.upper_lake);
    let lake_top = g.geography.top_level(d.upper_lake.level);
    let dry: BTreeSet<_> = domain
        .iter()
        .copied()
        .filter(|p| {
            let (column, liquid) = g.column(*p);
            solid_top(&column)
                .is_some_and(|top| top >= lake_top && liquid.is_none_or(|l| l.top <= top))
        })
        .collect();
    assert!(dry.contains(&court.column));
    let mut island = BTreeSet::from([court.column]);
    let mut queue = VecDeque::from([court.column]);
    while let Some(p) = queue.pop_front() {
        for next in neighbors(p) {
            if dry.contains(&next) && island.insert(next) {
                queue.push_back(next);
            }
        }
    }
    assert!(
        !island.contains(&shore.column),
        "the Garden must not have a hidden land bridge to the shore"
    );
    assert!(
        island.len() > 100,
        "the garden has a real usable island footprint"
    );
    let mut wet_boundary = 0;
    for p in &island {
        for next in neighbors(*p).filter(|next| !island.contains(next)) {
            assert!(
                domain.contains(&next),
                "garden land reaches the external mainland"
            );
            assert!(
                g.column(next).1.is_some_and(|l| l.top >= lake_top),
                "garden boundary lacks surrounding water at {next:?}"
            );
            wet_boundary += 1;
        }
    }
    assert!(wet_boundary > 0);
    println!(
        "GARDEN island columns={} water boundary edges={wet_boundary}",
        island.len()
    );
}
