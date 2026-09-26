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
    let mut cells = Cells::new();
    let floor = g.surface(root).level + 1;
    let [x, z] = world_xz(root);
    let heart = gaussian(x, z, -50., 160., 250., 230.);
    let height = if giant {
        320
    } else {
        (35. + heart * 85.) as i32 + (index % 25) as i32
    };
    let crown = if giant { 29 } else { 3 + (heart * 4.) as i64 };
    let trunk: i64 = if giant { 6 } else { 0 };
    for dq in -trunk..=trunk {
        for dr in -trunk..=trunk {
            if dq.abs().max(dr.abs()).max((dq + dr).abs()) <= trunk {
                let p = WorldHex::new(root.q + dq, root.r + dr);
                add(
                    &mut cells,
                    p,
                    g.surface(p).level + 1,
                    floor + height,
                    "timber",
                );
            }
        }
    }
    for dq in -crown..=crown {
        for dr in -crown..=crown {
            let d = (dq * dq + dq * dr + dr * dr) as f64;
            if d > (crown * crown) as f64 {
                continue;
            }
            let p = WorldHex::new(root.q + dq, root.r + dr);
            let depth = (1. - d / (crown * crown) as f64).sqrt();
            let lo = floor + height - (depth * if giant { 100. } else { 35. }) as i32;
            let hi = floor + height + (depth * if giant { 35. } else { 12. }) as i32;
            add(
                &mut cells,
                p,
                lo,
                hi,
                if floor > 1070 { "snow" } else { "foliage" },
            );
        }
    }
    if giant {
        // Low, spreading buttress roots follow the actual terrain. The south
        // approach remains clear above the authored passage into the temple.
        for (arm, (dq, dr)) in DIRS.into_iter().enumerate() {
            for distance in 5_i64..=24 {
                let bend = ((distance / 5 + arm as i64) % 3) - 1;
                let center = WorldHex::new(
                    root.q + dq * distance - dr * bend,
                    root.r + dr * distance + dq * bend,
                );
                let width: i64 = if distance < 13 { 2 } else { 1 };
                for aq in -width..=width {
                    for ar in -width..=width {
                        if aq.abs().max(ar.abs()).max((aq + ar).abs()) > width {
                            continue;
                        }
                        let p = WorldHex::new(center.q + aq, center.r + ar);
                        let [x, z] = world_xz(p);
                        if (x + 60.).abs() < 10. && (135. ..200.).contains(&z) {
                            continue;
                        }
                        let lo = g.surface(p).level + 1;
                        let rise = (29 - distance) as i32 / 3 + 1;
                        add(&mut cells, p, lo, lo + rise, "timber");
                    }
                }
            }
        }
    }
    object(
        g,
        if giant {
            "grand/world-tree".into()
        } else {
            format!("grand/tree/{index:05}")
        },
        if giant {
            "plant/grand-world-tree"
        } else {
            "plant/grand-forest-tree"
        },
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
            if d == 7 && (q + r) % 3 == 0 {
                add(&mut cells, p, floor, floor + 18, "stone");
            }
            if d == 8 {
                add(&mut cells, p, floor + 18, floor + 20, "stone");
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
    // Fort palisade: wide south gate and corner towers; center remains deployment clear.
    let root = nearest_hex(-60., 225.);
    let floor = g.surface(root).level + 1;
    let mut cells = Cells::new();
    for q in -17_i64..=17 {
        for r in -17_i64..=17 {
            if q.abs() != 17 && r.abs() != 17 {
                continue;
            }
            if r == 17 && q.abs() < 5 {
                continue;
            }
            let p = WorldHex::new(root.q + q, root.r + r);
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
            } else if index % 2 == 0 && [(-5, 0), (-5, 1), (-4, -1), (5, -3)].contains(&(q, r)) {
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
            let depth = index(p).map_or(0, |i| g.coast[i]);
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
