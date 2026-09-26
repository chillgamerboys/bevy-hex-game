//! Bounded, deterministic natural silhouettes and authored landmark geometry.
use super::*;
mod landmarks;
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
// These compact recipes retain the accepted expedition's actual curved boles,
// flared roots, rising branches and differently shaded crown lobes. Grand's three
// wider grove variants use that same sculptor; provenance lives beside the data.
const FOREST_LIBRARY: &str =
    include_str!("../../../../../assets/config/v4/grand-v4/forest/trees.ron");
const MAX_FOREST_TREES: usize = 780;
const MAX_FOREST_DETAILS: usize = 60;
type Occupied = BTreeMap<WorldHex, Vec<(i32, i32)>>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ForestLibrary {
    version: u32,
    trees: Vec<ForestTemplate>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ForestTemplate {
    name: String,
    height: i32,
    radius: i64,
    runs: Vec<(i64, i64, i32, i32, String)>,
}
impl ForestLibrary {
    fn load() -> Result<Self, ContractError> {
        let value: Self = ron::from_str(FOREST_LIBRARY)
            .map_err(|e| ContractError::new("grand/forest", e.to_string()))?;
        let mut names = std::collections::BTreeSet::new();
        if value.version != 1 || value.trees.len() != 15 {
            return Err(ContractError::new(
                "grand/forest",
                "wrong authored template catalogue",
            ));
        }
        for tree in &value.trees {
            if !names.insert(&tree.name)
                || !(18..=114).contains(&tree.height)
                || !(3..=16).contains(&tree.radius)
                || tree.runs.is_empty()
                || tree.runs.len() > 1_500
                || tree.runs.iter().any(|(q, r, lo, hi, material)| {
                    q.abs().max(r.abs()).max((q + r).abs()) > tree.radius
                        || *lo < 0
                        || lo >= hi
                        || *hi > tree.height
                        || forest_material(material).is_none()
                })
            {
                return Err(ContractError::new(
                    "grand/forest",
                    "invalid authored tree bounds or material",
                ));
            }
            if tree
                .runs
                .iter()
                .map(|(_, _, lo, hi, _)| hi - lo)
                .sum::<i32>()
                > 65_536
            {
                return Err(ContractError::new(
                    "grand/forest",
                    "tree exceeds its occupied-cell budget",
                ));
            }
        }
        Ok(value)
    }

    fn select(&self, name: &str) -> Result<&ForestTemplate, ContractError> {
        self.trees
            .iter()
            .find(|tree| tree.name == name)
            .ok_or_else(|| ContractError::new("grand/forest", format!("missing tree {name}")))
    }
}
fn forest_material(name: &str) -> Option<&'static str> {
    match name {
        "timber" => Some("timber"),
        "foliage_dark" => Some("foliage_dark"),
        "foliage" => Some("foliage"),
        "foliage_light" => Some("foliage_light"),
        _ => None,
    }
}
fn turn(mut q: i64, mut r: i64, rotation: u64) -> (i64, i64) {
    for _ in 0..rotation % 6 {
        (q, r) = (-r, q + r);
    }
    (q, r)
}
fn forest_hash(seed: u64, p: WorldHex) -> u64 {
    let mut value = seed
        ^ u64::from_le_bytes(p.q.to_le_bytes()).wrapping_mul(0x9e3779b97f4a7c15)
        ^ u64::from_le_bytes(p.r.to_le_bytes()).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    value ^ (value >> 31)
}
fn forest_density(x: f64, z: f64) -> f64 {
    // A shared biome envelope with a feathered, irregular boundary, not a grid
    // clipped to a hard ellipse. Large gaps form glades rather than missing rows.
    let edge = biomes::forest_extent(
        x + 31. * (z * 0.014).sin() + 12. * (x * 0.031).cos(),
        z + 23. * (x * 0.013).sin() - 17. * (z * 0.025).cos(),
    );
    let boundary = 1. - smooth((edge - 0.72) / 0.32);
    let groves = 0.82 + 0.12 * (x * 0.026 + z * 0.007).sin() + 0.06 * (z * 0.035 - x * 0.012).cos();
    let glades = [
        (-280., 90., 36., 25.),
        (120., 355., 38., 32.),
        (-160., 350., 24., 38.),
    ]
    .into_iter()
    .map(|(cx, cz, rx, rz)| gaussian(x, z, cx, cz, rx, rz))
    .fold(0_f64, f64::max);
    boundary * groves * (1. - smooth((glades - 0.30) / 0.50))
}
fn reserved_growth(g: &GrandCompiler, p: WorldHex) -> bool {
    let [x, z] = world_xz(p);
    sites::reserved_encounter(x, z)
        || sites::PADS
            .iter()
            .any(|a| (x - a.x).hypot(z - a.z) < a.radius + 3.)
        || CAMPS.iter().any(|(cx, cz)| (x - cx).hypot(z - cz) < 16.)
        || ((x + 60.).abs() < 12. && (125. ..260.).contains(&z))
        || g.anchors.iter().any(|a| {
            let [ax, az] = world_xz(a.position.column);
            (x - ax).hypot(z - az) < 8.
        })
}
fn overlaps(occupied: &Occupied, columns: &[ColumnData]) -> bool {
    columns.iter().any(|column| {
        occupied.get(&column.position).is_some_and(|old| {
            column
                .runs
                .iter()
                .any(|run| old.iter().any(|(lo, hi)| *lo < run.top && run.bottom < *hi))
        })
    })
}
fn reserve(occupied: &mut Occupied, object: &ObjectInstance) {
    for column in &object.occupancy {
        occupied
            .entry(column.position)
            .or_default()
            .extend(column.runs.iter().map(|run| (run.bottom, run.top)));
    }
}
fn place_tree(
    g: &GrandCompiler,
    template: &ForestTemplate,
    root: WorldHex,
    rotation: u64,
    occupied: &Occupied,
    frozen: bool,
) -> Result<Option<ObjectInstance>, ContractError> {
    let mut contacts = BTreeMap::new();
    for (q, r, lo, _, material) in &template.runs {
        if *lo == 0 && material == "timber" {
            let (q, r) = turn(*q, *r, rotation);
            let p = WorldHex::new(root.q + q, root.r + r);
            let surface = g.surface(p);
            if !g.mainland(p) || surface.water.is_some() || reserved_growth(g, p) {
                return Ok(None);
            }
            contacts.insert(p, surface.level + 1);
        }
    }
    let Some(floor) = contacts.values().copied().max() else {
        return Err(ContractError::new(
            "grand/forest",
            "tree has no grounded wood",
        ));
    };
    if contacts.values().any(|height| floor - height > 8) {
        return Ok(None); // Preserve the hillside rather than flattening its terrain.
    }
    let mut columns: BTreeMap<WorldHex, Vec<VoxelRun>> = BTreeMap::new();
    for (q, r, lo, hi, name) in &template.runs {
        let (q, r) = turn(*q, *r, rotation);
        let p = WorldHex::new(root.q + q, root.r + r);
        let surface = g.surface(p);
        let material = forest_material(name)
            .ok_or_else(|| ContractError::new("grand/forest", "unknown tree material"))?;
        let bottom = if *lo == 0 && material == "timber" {
            surface.level + 1
        } else {
            floor + lo
        };
        if !g.mainland(p)
            || bottom <= surface.level
            || (material != "timber" && bottom < surface.level + 9)
            || (bottom < surface.level + 17 && reserved_growth(g, p))
        {
            return Ok(None);
        }
        columns.entry(p).or_default().push(run(
            bottom,
            floor + hi,
            if frozen && material == "foliage_light" {
                "snow"
            } else {
                material
            },
        ));
    }
    let occupancy: Vec<_> = columns
        .into_iter()
        .map(|(position, runs)| ColumnData { position, runs })
        .collect();
    if overlaps(occupied, &occupancy) {
        return Ok(None);
    }
    let object = ObjectInstance {
        id: format!("grand/tree/{}_{}", root.q, root.r),
        region_id: "grand".into(),
        asset: format!("plant/grand-{}", template.name),
        origin: VoxelPosition {
            column: root,
            level: g.surface(root).level + 1,
        },
        rotation: 0, // Occupancy has already been rotated into world coordinates.
        grounding: Some(
            contacts
                .into_iter()
                .map(|(column, level)| VoxelPosition {
                    column,
                    level: level - 1,
                })
                .collect(),
        ),
        occupancy,
    };
    object.validate()?;
    Ok(Some(object))
}
fn compose_forest(g: &GrandCompiler, out: &mut Vec<ObjectInstance>) -> Result<(), ContractError> {
    let library = ForestLibrary::load()?;
    let mut occupied = Occupied::new();
    for object in out.iter() {
        reserve(&mut occupied, object);
    }
    let mut candidates = vec![];
    for q in (-480_i64..=480).step_by(4) {
        for r in (-480_i64..=480).step_by(4) {
            let key = forest_hash(g.source.seed, WorldHex::new(q, r));
            let root = WorldHex::new(q + (key % 7) as i64 - 3, r + ((key >> 8) % 7) as i64 - 3);
            let [x, z] = world_xz(root);
            let surface = g.surface(root);
            let edge = biomes::forest_extent(x, z);
            if !g.mainland(root)
                || surface.water.is_some()
                || !(420..=850).contains(&surface.level)
                || g.cavity(root).is_some()
                || reserved_growth(g, root)
                || (key % 10_000) as f64 / 10_000. > forest_density(x, z)
            {
                continue;
            }
            let variant = 1 + (key >> 24) % 3;
            let family = if edge < 0.80 && key % 5 < 3 {
                "grove"
            } else if edge < 0.92 && key % 5 < 4 {
                "ancient"
            } else if edge < 0.90 && !key.is_multiple_of(4) {
                "landmark"
            } else if key.is_multiple_of(3) {
                "understory-pine"
            } else {
                "understory-broadleaf"
            };
            let template = library.select(&format!("{family}-{variant}"))?;
            let layer = if template.name.starts_with("grove-") {
                0
            } else if template.name.starts_with("ancient-") {
                1
            } else if template.height > 34 {
                2
            } else {
                3
            };
            candidates.push((layer, key, root, template));
            if template.height > 34 {
                // A failed mature crown may still leave a valid sheltered site
                // for a small tree. This fills layers without adding more roots.
                let understory = if key.is_multiple_of(3) {
                    "understory-pine"
                } else {
                    "understory-broadleaf"
                };
                let small = library.select(&format!("{understory}-{variant}"))?;
                candidates.push((3, key.rotate_left(17), root, small));
            }
        }
    }
    // Mature groves reserve their branches first; smaller trees fill their gaps.
    // Hash order avoids an axial scan edge when the explicit count budget is met.
    candidates.sort_by_key(|(layer, key, root, template)| {
        (*layer, std::cmp::Reverse(template.radius), *key, *root)
    });
    let mut planted = vec![];
    let mut mature = 0;
    for (_, key, root, template) in candidates {
        if planted.len() >= MAX_FOREST_TREES - 12 {
            break;
        }
        // Keep an understory allocation even if every mature candidate fits.
        if template.height > 34 && mature >= MAX_FOREST_TREES - 12 - 180 {
            continue;
        }
        if planted
            .iter()
            .any(|(_, old): &(u64, WorldHex)| old.checked_distance(root).is_ok_and(|d| d < 6))
        {
            continue;
        }
        if let Some(tree) = place_tree(g, template, root, key % 6, &occupied, false)? {
            reserve(&mut occupied, &tree);
            out.push(tree);
            planted.push((key, root));
            mature += usize::from(template.height > 34);
        }
    }
    // Sparse snowy conifers retain the separately authored highland grove.
    for i in 0..12 {
        let root = nearest_hex(
            -250. + f64::from(i % 4) * 28.,
            -630. + f64::from(i / 4) * 20.,
        );
        let key = forest_hash(g.source.seed, root);
        let template = library.select(&format!("understory-pine-{}", 1 + i % 3))?;
        if let Some(tree) = place_tree(g, template, root, key % 6, &occupied, true)? {
            reserve(&mut occupied, &tree);
            out.push(tree);
        }
    }
    let mut details = 0;
    for (key, root) in planted {
        if details >= MAX_FOREST_DETAILS {
            break;
        }
        if !key.is_multiple_of(5) {
            continue;
        }
        let (q, r) = turn(7, -2, key % 6);
        let root = WorldHex::new(root.q + q, root.r + r);
        if let Some(detail) = forest_detail(g, root, key, &occupied)? {
            reserve(&mut occupied, &detail);
            out.push(detail);
            details += 1;
        }
    }
    Ok(())
}
fn forest_detail(
    g: &GrandCompiler,
    root: WorldHex,
    key: u64,
    occupied: &Occupied,
) -> Result<Option<ObjectInstance>, ContractError> {
    let mut cells = Cells::new();
    for q in -2_i64..=2 {
        for r in -2_i64..=2 {
            let distance = q.abs().max(r.abs()).max((q + r).abs());
            if distance > 2 || (key % 3 == 2 && r != 0) {
                continue;
            }
            let (q, r) = turn(q, r, key % 6);
            let p = WorldHex::new(root.q + q, root.r + r);
            let surface = g.surface(p);
            if !g.mainland(p) || surface.water.is_some() || reserved_growth(g, p) {
                return Ok(None);
            }
            let lo = surface.level + 1;
            if key.is_multiple_of(3) {
                let height = 2 + (2 - distance) as i32 * 2;
                add(&mut cells, p, lo, lo + height, "stone");
                add(&mut cells, p, lo + height - 1, lo + height, "moss");
            } else if key % 3 == 1 {
                if distance == 0 || (q - r).rem_euclid(3) == 0 {
                    add(
                        &mut cells,
                        p,
                        lo,
                        lo + 1 + (2 - distance) as i32,
                        if distance == 0 {
                            "foliage_dark"
                        } else {
                            "foliage"
                        },
                    );
                }
            } else {
                add(&mut cells, p, lo, lo + 2, "timber");
            }
        }
    }
    let object = object(
        g,
        format!("grand/forest-detail/{}_{}", root.q, root.r),
        "decor/grand-forest-floor",
        root,
        cells,
    )?;
    Ok((!overlaps(occupied, &object.occupancy)).then_some(object))
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
    // A gently curved, continuously tapered bole uses fractional contours so
    // individual columns end at different heights. Root fins and the fixed outer
    // crown remain the same landmark; this avoids a straight extruded cylinder.
    for q in -11_i64..=11 {
        for r in -11_i64..=11 {
            let p = WorldHex::new(root.q + q, root.r + r);
            let ground = g.surface(p).level + 1;
            for level in ground..floor + 265 {
                let t = (f64::from(level - floor) / 265.).clamp(0., 1.);
                let drift = smooth(t);
                let axis_q = 2.2 * drift + 0.6 * (std::f64::consts::PI * t).sin();
                let axis_r = -1.5 * drift;
                let x = q as f64 - axis_q + (r as f64 - axis_r) * 0.5;
                let z = (r as f64 - axis_r) * 3_f64.sqrt() * 0.5;
                let angle = z.atan2(x);
                let width = 8.4 - 4.4 * t.powf(0.68);
                let contour =
                    1. + 0.10 * (3. * angle + t).sin() + 0.07 * (5. * angle - 1.3 * t).cos();
                if x * x + z * z <= (width * contour).powi(2) {
                    add(&mut cells, p, level, level + 1, "timber");
                }
            }
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
    let mut out = vec![
        world_tree(g, nearest_hex(-60., 125.))?,
        temple_plant(g)?,
        root_temple_ribs(g)?,
        fire_marker(g)?,
        air_marker(g)?,
        earth_marker(g)?,
    ];
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
    out.extend(landmarks::compose(g)?);
    compose_forest(g, &mut out)?;
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

#[cfg(test)]
mod forest_tests {
    use super::*;
    use std::collections::BTreeSet;

    #[derive(Deserialize)]
    struct Stock {
        placements: Vec<StockCell>,
    }
    #[derive(Deserialize)]
    struct StockCell {
        position: StockPosition,
        style: String,
    }
    #[derive(Deserialize)]
    struct StockPosition {
        q: i64,
        r: i64,
        level: i32,
    }

    fn expanded(template: &ForestTemplate, rotation: u64) -> BTreeSet<(i64, i64, i32, &str)> {
        template
            .runs
            .iter()
            .flat_map(|(q, r, bottom, top, material)| {
                let (q, r) = turn(*q, *r, rotation);
                (*bottom..*top).map(move |level| (q, r, level, material.as_str()))
            })
            .collect()
    }

    #[test]
    fn forest_templates_preserve_accepted_art_and_exact_six_way_rotations() {
        let library = ForestLibrary::load().expect("bounded catalogue");
        for template in &library.trees {
            let original = expanded(template, 0);
            assert!(original.len() <= 65_536);
            assert_eq!(original, expanded(template, 6));
            for rotation in 0..6 {
                let rotated = expanded(template, rotation);
                let restored: BTreeSet<_> = rotated
                    .into_iter()
                    .map(|(q, r, level, material)| {
                        let (q, r) = turn(q, r, 6 - rotation);
                        (q, r, level, material)
                    })
                    .collect();
                assert_eq!(
                    restored, original,
                    "exact local geometry: {}",
                    template.name
                );
            }
            if template.name.starts_with("grove-") {
                continue;
            }
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                "../../assets/art/objects/plant/forest-expedition-{}.ron",
                template.name
            ));
            let stock: Stock =
                ron::from_str(&std::fs::read_to_string(path).expect("stock artwork"))
                    .expect("stock schema");
            let exact: BTreeSet<_> = stock
                .placements
                .iter()
                .map(|cell| {
                    let material = match cell.style.as_str() {
                        "plant/trunk" => "timber",
                        "plant/foliage-dark" => "foliage_dark",
                        "plant/foliage-mid" => "foliage",
                        "plant/foliage-light" => "foliage_light",
                        _ => panic!("unexpected stock tree style"),
                    };
                    (
                        cell.position.q,
                        cell.position.r,
                        cell.position.level,
                        material,
                    )
                })
                .collect();
            assert_eq!(
                original, exact,
                "accepted forest artwork was changed: {}",
                template.name
            );
        }
    }

    #[test]
    fn forest_edge_is_feathered_and_named_glades_stay_open() {
        assert!(forest_density(-280., 90.) < 0.01);
        assert!(forest_density(120., 355.) < 0.01);
        assert!(forest_density(-60., 110.) > 0.70);
        assert!(forest_density(700., 600.) < 0.01);
        let transition = (-400..600)
            .map(|x| forest_density(f64::from(x), 330.))
            .filter(|density| *density > 0.05 && *density < 0.65)
            .count();
        assert!(
            transition > 100,
            "a broad graded edge rather than a hard cutoff"
        );
    }

    #[test]
    fn authored_forest_is_bounded_grounded_layered_and_clear_of_gameplay_sites() {
        let source: GrandSpec = ron::from_str(include_str!(
            "../../../../../assets/config/v4/grand-v4/world.ron"
        ))
        .expect("Grand source");
        let g = GrandCompiler::new(source).expect("complete authored composition");
        let objects: Vec<_> = g.objects.values().flatten().collect();
        let trees: Vec<_> = objects
            .iter()
            .copied()
            .filter(|o| o.id.starts_with("grand/tree/"))
            .collect();
        assert!(trees.len() <= MAX_FOREST_TREES);
        assert!(objects.len() < 900);
        let mut occupied = Occupied::new();
        for object in objects
            .iter()
            .copied()
            .filter(|o| !o.id.starts_with("grand/tree/"))
        {
            reserve(&mut occupied, object);
        }
        let mut families = BTreeSet::new();
        let mut canopy = BTreeSet::new();
        for tree in &trees {
            families.insert(tree.asset.as_str());
            assert!(
                !overlaps(&occupied, &tree.occupancy),
                "exact solid overlap at {}",
                tree.id
            );
            reserve(&mut occupied, tree);
            let contacts = tree.grounding.as_ref().expect("root contacts");
            assert!(!contacts.is_empty());
            for contact in contacts {
                let (terrain, _) = g.column(contact.column);
                assert!(
                    terrain.material_at(contact.level).is_some(),
                    "unsupported root {}",
                    tree.id
                );
                assert!(
                    tree.occupancy.iter().any(|column| {
                        column.position == contact.column
                            && column.runs.iter().any(|run| {
                                run.material == "timber" && run.bottom == contact.level + 1
                            })
                    }),
                    "the declared root must actually meet that terrain"
                );
            }
            for column in &tree.occupancy {
                if column
                    .runs
                    .iter()
                    .any(|run| run.material.starts_with("foliage"))
                {
                    canopy.insert(column.position);
                }
            }
        }
        let mut counts = BTreeMap::new();
        for tree in &trees {
            *counts.entry(tree.asset.as_str()).or_insert(0_usize) += 1;
        }
        println!("GRAND_FOREST_FAMILIES {counts:?}");
        assert!(
            families.len() >= 10,
            "multiple mature and understory silhouettes"
        );
        let mut domain = 0_u32;
        let mut covered = 0_u32;
        let mut core = 0_u32;
        let mut covered_core = 0_u32;
        for &(r, a, b) in &g.source.mainland_rows {
            for q in a..=b {
                let p = WorldHex::new(q, r);
                let [x, z] = world_xz(p);
                let edge = biomes::forest_extent(x, z);
                if edge > 1. || g.surface(p).water.is_some() {
                    continue;
                }
                domain += 1;
                covered += u32::from(canopy.contains(&p));
                if edge < 0.70 {
                    core += 1;
                    covered_core += u32::from(canopy.contains(&p));
                }
            }
        }
        // Count exact union of real canopy columns, including clearings in the
        // denominator. The giant landmark is deliberately excluded from coverage.
        let fraction = f64::from(covered) / f64::from(domain);
        let core_fraction = f64::from(covered_core) / f64::from(core);
        println!("GRAND_FOREST trees={} families={} columns={covered}/{domain} coverage={fraction:.4} core={covered_core}/{core} core_coverage={core_fraction:.4}", trees.len(), families.len());
        assert!(
            fraction >= 0.40,
            "forest must retain substantial foliage between its glades"
        );
        assert!(
            core_fraction >= 0.50,
            "inner groves need overlapping canopy coverage"
        );
        let sites = g.sites(1).expect("published sites");
        for site in &sites.encounters {
            assert!(g.clear_support(site.preferred, 16), "{}", site.id);
        }
        for node in &sites.route_nodes {
            assert!(g.clear_support(node.position, 8), "{}", node.id);
        }
    }
}
