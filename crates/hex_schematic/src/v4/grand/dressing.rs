//! Bounded, deterministic natural silhouettes and authored landmark geometry.
use super::*;
type Cells = BTreeMap<WorldHex, BTreeMap<i32, &'static str>>;
const CAMPS: [(f64, f64); 6] = [
    (-55., 380.),
    (-263., 273.),
    (132., 294.),
    (-192., 188.),
    (112., 147.),
    (24., 211.),
];
fn add(cells: &mut Cells, p: WorldHex, bottom: i32, top: i32, material: &'static str) {
    for y in bottom..top {
        cells.entry(p).or_default().insert(y, material);
    }
}
fn object(
    g: &GrandCompiler,
    id: String,
    asset: &str,
    root: WorldHex,
    cells: Cells,
) -> Result<ObjectInstance, ContractError> {
    let mut occupancy = vec![];
    let mut grounding = vec![];
    for (p, cells) in cells {
        let (terrain, _) = g.column(p);
        let mut runs: Vec<VoxelRun> = vec![];
        for (y, m) in cells {
            if terrain.runs.iter().any(|r| r.bottom <= y && r.top > y) {
                continue;
            }
            if let Some(r) = runs.last_mut() {
                if r.top == y && r.material == m {
                    r.top += 1;
                    continue;
                }
            }
            runs.push(run(y, y + 1, m));
        }
        if runs.is_empty() {
            continue;
        }
        // Production admission uses the highest solid support per occupied
        // column. Cavern ceilings can create more than one separated contact.
        for r in runs.iter().rev() {
            if terrain
                .runs
                .iter()
                .any(|t| t.top == r.bottom && t.material != "water")
            {
                grounding.push(VoxelPosition {
                    column: p,
                    level: r.bottom - 1,
                });
                break;
            }
        }
        occupancy.push(ColumnData { position: p, runs });
    }
    let out = ObjectInstance {
        id,
        region_id: "grand".into(),
        asset: asset.into(),
        origin: VoxelPosition {
            column: root,
            level: g.surface(root).level + 1,
        },
        rotation: 0,
        occupancy,
        grounding: Some(grounding),
    };
    out.validate()
        .map_err(|e| ContractError::new(&out.id, e.to_string()))?;
    Ok(out)
}
fn tree(
    g: &GrandCompiler,
    root: WorldHex,
    index: usize,
    giant: bool,
) -> Result<ObjectInstance, ContractError> {
    if giant {
        return world_tree(g, root);
    }
    let mut cells = Cells::new();
    let floor = g.surface(root).level + 1;
    let [x, z] = world_xz(root);
    let heart = gaussian(x, z, -50., 160., 250., 230.);
    let height = (35. + heart * 85.) as i32 + (index % 25) as i32;
    let crown = 3 + (heart * 4.) as i64;
    add(&mut cells, root, floor, floor + height, "timber");
    for dq in -crown..=crown {
        for dr in -crown..=crown {
            let d = (dq * dq + dq * dr + dr * dr) as f64;
            if d > (crown * crown) as f64 {
                continue;
            }
            let p = WorldHex::new(root.q + dq, root.r + dr);
            let depth = (1. - d / (crown * crown) as f64).sqrt();
            add(
                &mut cells,
                p,
                floor + height - (depth * 35.) as i32,
                floor + height + (depth * 12.) as i32,
                if floor > 1070 { "snow" } else { "foliage" },
            );
        }
    }
    object(
        g,
        format!("grand/tree/{index:05}"),
        "plant/grand-forest-tree",
        root,
        cells,
    )
}

// Independent, asymmetric lobes make a broad living canopy rather than a single
// sphere on a pole. Compact per-column intervals keep the full landmark bounded.
const WORLD_TREE_LOBES: [(i64, i64, i64, i32); 7] = [
    (0, 0, 44, 300),
    (-35, 8, 35, 278),
    (38, -14, 34, 294),
    (8, -38, 36, 269),
    (-12, 41, 34, 288),
    (-36, -24, 30, 259),
    (30, 28, 31, 280),
];
fn world_tree(g: &GrandCompiler, root: WorldHex) -> Result<ObjectInstance, ContractError> {
    let floor = g.surface(root).level + 1;
    let mut cells = Cells::new();
    let mut canopy: BTreeMap<WorldHex, (i32, i32)> = BTreeMap::new();
    for (cq, cr, radius, height) in WORLD_TREE_LOBES {
        for q in -radius..=radius {
            for r in -radius..=radius {
                let metric = q * q + q * r + r * r;
                if metric > radius * radius {
                    continue;
                }
                let depth = (1. - metric as f64 / (radius * radius) as f64).sqrt();
                let low = floor + height - (depth * 90.) as i32;
                let high = floor + height + (depth * 55.) as i32 + 1;
                let p = WorldHex::new(root.q + cq + q, root.r + cr + r);
                canopy
                    .entry(p)
                    .and_modify(|(lo, hi)| {
                        *lo = (*lo).min(low);
                        *hi = (*hi).max(high);
                    })
                    .or_insert((low, high));
            }
        }
    }
    for (p, (lo, hi)) in canopy {
        add(&mut cells, p, lo, hi, "foliage");
    }
    // Flared trunk narrows as the major boughs take over its load.
    for q in -9_i64..=9 {
        for r in -9_i64..=9 {
            let distance = q.abs().max(r.abs()).max((q + r).abs());
            if distance > 9 {
                continue;
            }
            let p = WorldHex::new(root.q + q, root.r + r);
            let top = floor
                + if distance <= 5 {
                    265
                } else {
                    250 - (distance as i32 - 5) * 38
                };
            add(&mut cells, p, g.surface(p).level + 1, top, "timber");
        }
    }
    // Each outer lobe has a visible rising branch from the trunk into its heart.
    for (cq, cr, _, height) in WORLD_TREE_LOBES.into_iter().skip(1) {
        let steps = cq.abs().max(cr.abs()).max((cq + cr).abs());
        for step in 0..=steps {
            let t = step as f64 / steps as f64;
            let q = (cq as f64 * t).round() as i64;
            let r = (cr as f64 * t).round() as i64;
            let center = floor + 110 + ((height - 130) as f64 * t.sqrt()) as i32;
            let width = if t < 0.30 {
                4_i64
            } else if t < 0.70 {
                3
            } else {
                2
            };
            for aq in -width..=width {
                for ar in -width..=width {
                    if aq.abs().max(ar.abs()).max((aq + ar).abs()) > width {
                        continue;
                    }
                    add(
                        &mut cells,
                        WorldHex::new(root.q + q + aq, root.r + r + ar),
                        center - 6,
                        center + 7,
                        "timber",
                    );
                }
            }
        }
    }
    // Broad buttresses and branching surface roots follow terrain, retaining the
    // main south passage and every encounter/camp's usable deployment ground.
    for (arm, (dq, dr)) in DIRS.into_iter().enumerate() {
        for distance in 5_i64..=68 {
            let bend = ((distance as f64 / 13. + arm as f64).sin() * 4.).round() as i64;
            let center = WorldHex::new(
                root.q + dq * distance - dr * bend,
                root.r + dr * distance + dq * bend,
            );
            let width: i64 = if distance < 18 {
                4
            } else if distance < 40 {
                2
            } else {
                1
            };
            for aq in -width..=width {
                for ar in -width..=width {
                    if aq.abs().max(ar.abs()).max((aq + ar).abs()) > width {
                        continue;
                    }
                    let p = WorldHex::new(center.q + aq, center.r + ar);
                    let [x, z] = world_xz(p);
                    if (x + 60.).abs() < 12. && (130. ..260.).contains(&z)
                        || sites::reserved_encounter(x, z)
                        || CAMPS.iter().any(|(cx, cz)| (x - cx).hypot(z - cz) < 12.)
                    {
                        continue;
                    }
                    let lo = g.surface(p).level + 1;
                    let rise = 2 + (68 - distance) as i32 / 4;
                    add(&mut cells, p, lo, lo + rise, "timber");
                }
            }
        }
    }
    object(
        g,
        "grand/world-tree".into(),
        "plant/grand-world-tree",
        root,
        cells,
    )
}
fn shrine(
    g: &GrandCompiler,
    id: &str,
    x: f64,
    z: f64,
    inside: bool,
) -> Result<ObjectInstance, ContractError> {
    let root = nearest_hex(x, z);
    let floor = g.support(x, z, inside).level + 1;
    let mut cells = Cells::new();
    for q in -8_i64..=8 {
        for r in -8_i64..=8 {
            let d = q.abs().max(r.abs()).max((q + r).abs());
            let p = WorldHex::new(root.q + q, root.r + r);
            let [px, pz] = world_xz(p);
            let plant_door = id == "plant" && (px + 60.).abs() < 4. && pz > 125.;
            if d == 7 && (q + r) % 3 == 0 && !plant_door {
                add(
                    &mut cells,
                    p,
                    floor,
                    floor + 18,
                    if id == "plant" { "timber" } else { "stone" },
                );
            }
            if d == 8 {
                add(
                    &mut cells,
                    p,
                    floor + 18,
                    floor + 20,
                    if id == "plant" { "moss" } else { "stone" },
                );
            }
        }
    }
    object(
        g,
        format!("grand/shrine/{id}"),
        "structure/grand-shrine",
        root,
        cells,
    )
}
/// Reserve a sparse global forest (bounded roots and exact compact occupancies).
pub(super) fn compose(g: &GrandCompiler) -> Result<Vec<ObjectInstance>, ContractError> {
    let mut out = vec![];
    let mut index = 0;
    // Thirteen-column spacing bounds roots at <6000 on the entire mainland.
    for q in (-480..=480).step_by(13) {
        for r in (-480..=480).step_by(13) {
            let root = WorldHex::new(q + (r * 7_i64).rem_euclid(5), r);
            let [x, z] = world_xz(root);
            let s = g.surface(root);
            let forest = biomes::forest_extent(x, z);
            let frozen = ((x + 200.) / 80.).hypot((z + 610.) / 45.);
            if !g.mainland(root)
                || s.water.is_some()
                || s.level < 420
                || s.level > 1200
                || forest > 1.
                || (frozen < 1. && index % 5 != 0)
            {
                continue;
            }
            if sites::PADS
                .iter()
                .any(|a| (x - a.x).hypot(z - a.z) < a.radius + 22.)
                || ((x + 60.) / 65.).hypot((z - 150.) / 105.) < 1.
                || g.cavity(root).is_some()
                || sites::reserved_encounter(x, z)
                || CAMPS.iter().any(|(cx, cz)| (x - cx).hypot(z - cz) < 16.)
            {
                continue;
            }
            if forest > 0.8 && (q + r).rem_euclid(3) != 0 {
                continue;
            }
            out.push(tree(g, root, index, false)?);
            index += 1;
        }
    }
    // Sparse frozen trees sit atop the ascent, independent of the lowland forest.
    for i in 0..12 {
        let x = -250. + (i % 4) as f64 * 28.;
        let z = -630. + (i / 4) as f64 * 20.;
        out.push(tree(g, nearest_hex(x, z), index, false)?);
        index += 1;
    }
    out.push(tree(g, nearest_hex(-60., 125.), index, true)?);
    out.push(temple_plant(g)?);
    out.push(root_temple_ribs(g)?);
    out.push(fire_marker(g)?);
    out.push(air_marker(g)?);
    out.push(earth_marker(g)?);
    for (i, (x, z)) in CAMPS.into_iter().enumerate() {
        out.push(camp(g, i, nearest_hex(x, z))?);
    }
    for (i, (x, z)) in [(-390., 430.), (-555., 360.), (535., 580.), (700., 260.)]
        .into_iter()
        .enumerate()
    {
        if let Some(root) = coastal_root(g, x, z) {
            out.push(coastal_rock(g, i, root)?);
        }
    }
    for (id, x, z, inside) in [
        ("water", 275., -490., false),
        ("air", -400., -565., false),
        ("earth", -179., -518., false),
        ("plant", -60., 125., true),
        ("fire", -1170., 455., false),
    ] {
        out.push(shrine(g, id, x, z, inside)?);
    }
    // Giant central crystal, with an accessible shrine on its eastern shoulder.
    let root = nearest_hex(-214., -531.);
    let floor = g.surface(root).level + 1;
    let mut cells = Cells::new();
    for q in -9_i64..=9 {
        for r in -9_i64..=9 {
            let d = q.abs().max(r.abs()).max((q + r).abs());
            if d > 9 {
                continue;
            }
            add(
                &mut cells,
                WorldHex::new(root.q + q, root.r + r),
                floor,
                floor + 210 - d as i32 * 16,
                "crystal",
            );
        }
    }
    out.push(object(
        g,
        "grand/crystal-heart".into(),
        "structure/grand-crystal",
        root,
        cells,
    )?);
    // Gates align with the world-space temple approach across both skewed axial walls.
    let root = nearest_hex(-60., 225.);
    let floor = g.surface(root).level + 1;
    let mut cells = Cells::new();
    for q in -17_i64..=17 {
        for r in -17_i64..=17 {
            if q.abs() != 17 && r.abs() != 17 {
                continue;
            }
            let p = WorldHex::new(root.q + q, root.r + r);
            if r.abs() == 17 && (world_xz(p)[0] + 60.).abs() < 8. {
                continue;
            }
            let lo = g.surface(p).level + 1;
            add(
                &mut cells,
                p,
                lo,
                floor + if q.abs() > 13 && r.abs() > 13 { 48 } else { 28 },
                "timber",
            );
        }
    }
    out.push(object(
        g,
        "grand/goblin-fort".into(),
        "structure/grand-fort",
        root,
        cells,
    )?);
    // Shelves in the lower/upper library, never in the central staircase or encounters.
    for (i, (x, z)) in [
        (-420., -382.),
        (-340., -382.),
        (-430., -525.),
        (-370., -540.),
    ]
    .into_iter()
    .enumerate()
    {
        let root = nearest_hex(x, z);
        let floor = g.support(x, z, true).level + 1;
        let mut cells = Cells::new();
        for q in -4..=4 {
            let p = WorldHex::new(root.q + q, root.r);
            for y in 0..26 {
                if y % 7 <= 1 || q.abs() == 4 {
                    add(&mut cells, p, floor + y, floor + y + 1, "timber");
                }
            }
        }
        out.push(object(
            g,
            format!("grand/library-shelf/{i}"),
            "structure/grand-bookshelf",
            root,
            cells,
        )?);
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

fn temple_plant(g: &GrandCompiler) -> Result<ObjectInstance, ContractError> {
    // Behind the interaction point, inside the ring of temple columns.
    let root = nearest_hex(-60., 120.);
    let floor = g.support(-60., 120., true).level + 1;
    let mut cells = Cells::new();
    add(&mut cells, root, floor, floor + 10, "timber");
    for q in -2_i64..=2 {
        for r in -2_i64..=2 {
            let distance = q.abs().max(r.abs()).max((q + r).abs());
            if distance == 0 || distance > 2 {
                continue;
            }
            let p = WorldHex::new(root.q + q, root.r + r);
            let bottom = floor + 4 + distance as i32;
            add(&mut cells, p, bottom, bottom + 2, "foliage");
        }
    }
    object(
        g,
        "grand/root-temple-plant".into(),
        "decor/grand-temple-plant",
        root,
        cells,
    )
}

fn root_temple_ribs(g: &GrandCompiler) -> Result<ObjectInstance, ContractError> {
    let root = nearest_hex(-60., 161.);
    let mut cells = Cells::new();
    for z in [147., 161., 175.] {
        for x in [-65., -55.] {
            let p = nearest_hex(x, z);
            let floor = g.support(x, z, true).level + 1;
            add(&mut cells, p, floor, floor + 26, "timber");
        }
        for x in -65..=-55 {
            let p = nearest_hex(f64::from(x), z);
            let floor = g.support(f64::from(x), z, true).level + 1;
            add(&mut cells, p, floor + 26, floor + 29, "timber");
        }
    }
    object(
        g,
        "grand/root-temple-ribs".into(),
        "structure/grand-root-arches",
        root,
        cells,
    )
}

fn marker_base(g: &GrandCompiler, root: WorldHex) -> (Cells, i32) {
    let floor = g.surface(root).level + 1;
    let mut cells = Cells::new();
    for q in -2_i64..=2 {
        for r in -2_i64..=2 {
            if q.abs().max(r.abs()).max((q + r).abs()) <= 2 {
                let p = WorldHex::new(root.q + q, root.r + r);
                add(&mut cells, p, g.surface(p).level + 1, floor + 2, "stone");
            }
        }
    }
    (cells, floor)
}

fn fire_marker(g: &GrandCompiler) -> Result<ObjectInstance, ContractError> {
    // A static flame-shaped sculpture behind the claim point. Its opaque warm
    // masonry has ordinary solid behavior; it is not an active fire or hazard.
    let root = nearest_hex(-1170., 449.);
    let (mut cells, floor) = marker_base(g, root);
    for q in -1_i64..=1 {
        for r in -1_i64..=1 {
            if q.abs().max(r.abs()).max((q + r).abs()) <= 1 {
                let p = WorldHex::new(root.q + q, root.r + r);
                let height = if q == 0 && r == 0 {
                    13
                } else {
                    7 + (q - r) as i32
                };
                add(&mut cells, p, floor + 2, floor + height, "worked_stone");
            }
        }
    }
    // The bent central tongue and uneven side tips give the small silhouette
    // direction without extending over the interaction point or temple roof.
    add(&mut cells, root, floor + 13, floor + 15, "worked_stone");
    add(
        &mut cells,
        WorldHex::new(root.q + 1, root.r),
        floor + 11,
        floor + 16,
        "worked_stone",
    );
    add(
        &mut cells,
        WorldHex::new(root.q - 1, root.r + 1),
        floor + 5,
        floor + 11,
        "worked_stone",
    );
    object(
        g,
        "grand/fire-flame-marker".into(),
        "decor/grand-static-flame",
        root,
        cells,
    )
}

fn air_marker(g: &GrandCompiler) -> Result<ObjectInstance, ContractError> {
    // A grounded, pale spiral around a blue center, behind the claim point.
    // This is bounded static art rather than a wind simulation or new ability.
    let root = nearest_hex(-400., -571.);
    let (mut cells, floor) = marker_base(g, root);
    add(&mut cells, root, floor + 2, floor + 16, "crystal");
    for (step, &(q, r)) in DIRS.iter().cycle().take(12).enumerate() {
        let bottom = floor + 2 + step as i32;
        add(
            &mut cells,
            WorldHex::new(root.q + q, root.r + r),
            bottom,
            bottom + 3,
            "snow",
        );
    }
    object(
        g,
        "grand/air-spiral-marker".into(),
        "decor/grand-static-spiral",
        root,
        cells,
    )
}

fn earth_marker(g: &GrandCompiler) -> Result<ObjectInstance, ContractError> {
    let root = nearest_hex(-179., -524.);
    let (mut cells, floor) = marker_base(g, root);
    for q in -2_i64..=2 {
        for r in -2_i64..=2 {
            let distance = q.abs().max(r.abs()).max((q + r).abs());
            if distance <= 2 {
                add(
                    &mut cells,
                    WorldHex::new(root.q + q, root.r + r),
                    floor + 2,
                    floor + 15 - distance as i32 * 4,
                    "crystal",
                );
            }
        }
    }
    object(
        g,
        "grand/earth-heart-marker".into(),
        "decor/grand-earth-heart",
        root,
        cells,
    )
}

fn camp(g: &GrandCompiler, index: usize, root: WorldHex) -> Result<ObjectInstance, ContractError> {
    let mut cells = Cells::new();
    for q in -6_i64..=6 {
        for r in -6_i64..=6 {
            let p = WorldHex::new(root.q + q, root.r + r);
            let surface = g.surface(p);
            if surface.water.is_some() || !g.mainland(p) {
                continue;
            }
            let lo = surface.level + 1;
            let distance = q.abs().max(r.abs()).max((q + r).abs());
            if distance == 2 {
                add(&mut cells, p, lo, lo + 2, "stone");
            } else if r.abs() == 4 && q.abs() <= 3 {
                add(&mut cells, p, lo, lo + 3, "timber");
            } else if index.is_multiple_of(2)
                && [(-5, 0), (-5, 1), (-4, -1), (5, -3)].contains(&(q, r))
            {
                let height = 7 + ((q - r + index as i64).rem_euclid(7)) as i32;
                add(&mut cells, p, lo, lo + height, "stone");
            }
        }
    }
    object(
        g,
        format!("grand/forest-camp/{index}"),
        "structure/grand-camp",
        root,
        cells,
    )
}

fn coastal_root(g: &GrandCompiler, x: f64, z: f64) -> Option<WorldHex> {
    let hint = nearest_hex(x, z);
    let mut best: Option<(f64, WorldHex)> = None;
    for q in -60_i64..=60 {
        for r in -60_i64..=60 {
            if q.abs().max(r.abs()).max((q + r).abs()) > 60 {
                continue;
            }
            let p = WorldHex::new(hint.q + q, hint.r + r);
            let depth = grid_value(&g.coast, p, 0);
            if !(2..=5).contains(&depth) {
                continue;
            }
            let surface = g.surface(p);
            if surface.water.is_some() || surface.level > 430 {
                continue;
            }
            let [px, pz] = world_xz(p);
            if g.anchors
                .iter()
                .filter(|a| a.id.ends_with("party_start") || a.id.ends_with("sailing_start"))
                .any(|a| {
                    let [ax, az] = world_xz(a.position.column);
                    (px - ax).hypot(pz - az) < 35.
                })
            {
                continue;
            }
            let distance = (px - x).powi(2) + (pz - z).powi(2);
            if best.is_none_or(|(old, _)| distance < old) {
                best = Some((distance, p));
            }
        }
    }
    best.map(|(_, p)| p)
}

fn coastal_rock(
    g: &GrandCompiler,
    index: usize,
    root: WorldHex,
) -> Result<ObjectInstance, ContractError> {
    let mut cells = Cells::new();
    for q in -3_i64..=3 {
        for r in -3_i64..=3 {
            let metric = q * q + q * r + r * r;
            if metric > 9 {
                continue;
            }
            let p = WorldHex::new(root.q + q, root.r + r);
            let lo = g.surface(p).level + 1;
            let rise =
                3 + ((9 - metric) * 2 / 3) as i32 + ((q + index as i64).rem_euclid(3)) as i32;
            add(&mut cells, p, lo, lo + rise, "stone");
        }
    }
    object(
        g,
        format!("grand/coastal-rock/{index}"),
        "terrain/grand-coastal-rock",
        root,
        cells,
    )
}
