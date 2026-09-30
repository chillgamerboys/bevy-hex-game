//! Bounded, deterministic natural silhouettes and authored landmark geometry.
use super::*;
mod landmarks;
mod world_tree;
type Cells = BTreeMap<WorldHex, BTreeMap<i32, &'static str>>;
const CAMP_FRAMES: [&str; 6] = [
    "camp_01", "camp_02", "camp_03", "camp_04", "camp_05", "camp_06",
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
            if terrain.runs.iter().any(|r| r.bottom <= y && r.top > y)
                || g.reserved_interval(p, y, y + 1)
            {
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
pub(super) fn forest_hash(seed: u64, p: WorldHex) -> u64 {
    let mut value = seed
        ^ u64::from_le_bytes(p.q.to_le_bytes()).wrapping_mul(0x9e3779b97f4a7c15)
        ^ u64::from_le_bytes(p.r.to_le_bytes()).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    value ^ (value >> 31)
}
pub(super) fn forest_density(g: &GrandCompiler, p: WorldHex) -> f64 {
    g.geography.forest_density(p)
}
pub(super) fn reserved_growth(g: &GrandCompiler, p: WorldHex) -> bool {
    let bottom = g.surface(p).level + 1;
    g.reserved_interval(p, bottom, bottom + 17)
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
            let (terrain, _) = g.column(p);
            if !g.mainland(p)
                || surface.water.is_some()
                || reserved_growth(g, p)
                || terrain
                    .material_at(surface.level)
                    .is_none_or(|material| material == "water")
            {
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
        let (terrain, _) = g.column(p);
        if !g.mainland(p)
            || bottom <= surface.level
            || (material != "timber" && bottom < surface.level + 9)
            || g.reserved_interval(p, bottom, floor + hi)
            || terrain
                .runs
                .iter()
                .any(|run| run.bottom < floor + hi && bottom < run.top)
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
    let mut frozen_candidates = vec![];
    for &(r, first, last) in &g.source.mainland_rows {
        if r.rem_euclid(4) != 0 {
            continue;
        }
        for q in (first..=last).filter(|q| q.rem_euclid(4) == 0) {
            let key = forest_hash(g.source.seed, WorldHex::new(q, r));
            let root = WorldHex::new(q + (key % 7) as i64 - 3, r + ((key >> 8) % 7) as i64 - 3);
            let surface = g.surface(root);
            if !g.mainland(root) || surface.water.is_some() || reserved_growth(g, root) {
                continue;
            }
            let variant = 1 + (key >> 24) % 3;
            let frozen = g.geography.frozen_planting_weight(root);
            if frozen > 0. && (key % 10_000) as f64 / 10_000. <= frozen {
                let template = library.select(&format!("understory-pine-{variant}"))?;
                frozen_candidates.push((key, root, template));
                continue;
            }
            let density = forest_density(g, root);
            if (key % 10_000) as f64 / 10_000. > density || density <= 0. {
                continue;
            }
            let family = if density > 0.70 && key % 5 < 3 {
                "grove"
            } else if density > 0.45 && key % 5 < 4 {
                "ancient"
            } else if density > 0.35 && !key.is_multiple_of(4) {
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
    // Reserve a bounded part of the existing whole-forest budget for the
    // continuous high route, rather than another fixed rectangular grove.
    frozen_candidates.sort_by_key(|(key, root, _)| (*key, *root));
    let mut frozen_planted: Vec<WorldHex> = vec![];
    for (key, root, template) in frozen_candidates {
        if frozen_planted.len() >= 72 {
            break;
        }
        if frozen_planted
            .iter()
            .any(|old| old.checked_distance(root).is_ok_and(|d| d < 8))
        {
            continue;
        }
        if let Some(tree) = place_tree(g, template, root, key % 6, &occupied, true)? {
            reserve(&mut occupied, &tree);
            out.push(tree);
            frozen_planted.push(root);
        }
    }
    let ordinary_budget = MAX_FOREST_TREES - frozen_planted.len();
    // Mature groves reserve their branches first; smaller trees fill their gaps.
    // Hash order avoids an axial scan edge when the explicit count budget is met.
    candidates.sort_by_key(|(layer, key, root, template)| {
        (*layer, std::cmp::Reverse(template.radius), *key, *root)
    });
    let mut planted = vec![];
    let mut mature = 0;
    for (_, key, root, template) in candidates {
        if planted.len() >= ordinary_budget {
            break;
        }
        // Keep an understory allocation even if every mature candidate fits.
        if template.height > 34 && mature >= ordinary_budget.saturating_sub(180) {
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
    // Thin only the geography-owned landmark glade after ordinary placement.
    // Do not refill its allocation elsewhere: the surrounding forest keeps its
    // stable authored instances and density. Whole trees are omitted rather
    // than trimming crowns into a straight clearing boundary; low detail props
    // and the actual World Tree/camp structures remain untouched.
    out.retain(|object| {
        if !object.id.starts_with("grand/tree/") {
            return true;
        }
        if object
            .occupancy
            .iter()
            .any(|column| g.geography.landmark_glade_weight(column.position) >= 1.)
        {
            return false;
        }
        let weight = g.geography.landmark_glade_weight(object.origin.column);
        let sample = (forest_hash(g.source.seed, object.origin.column) % 10_000) as f64 / 10_000.;
        sample >= weight
    });
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

fn shrine(
    g: &GrandCompiler,
    id: &str,
    occupied: &Occupied,
) -> Result<ObjectInstance, ContractError> {
    let frame = g.geography.frame(&format!("shrine_{id}"))?;
    let root = frame.hex([0., 0.]);
    let entrance = frame.hex([0., -10.]);
    // Keep a small colonnade where possible. Wider route terminals need their
    // piers outside the walking ribbon; each pier starts on its own real floor.
    for radius in [7_i64, 10, 13] {
        let mut piers = Vec::new();
        for q in -radius..=radius {
            for r in -radius..=radius {
                if q.abs().max(r.abs()).max((q + r).abs()) != radius || (q + r) % 3 != 0 {
                    continue;
                }
                let p = WorldHex::new(root.q + q, root.r + r);
                if id == "plant" && p.checked_distance(entrance).is_ok_and(|d| d < 5) {
                    continue;
                }
                let Ok(support) = g.support_at(&frame, frame.local(p)) else {
                    continue;
                };
                let floor = support.level + 1;
                if !g.reserved_interval(p, floor, floor + 20)
                    && g.column(p)
                        .0
                        .runs
                        .iter()
                        .all(|run| run.bottom >= floor + 20 || run.top <= floor)
                {
                    piers.push((p, floor));
                }
            }
        }
        if piers.len() < 3 {
            continue;
        }
        let Some(roof) = piers.iter().map(|(_, floor)| floor + 18).max() else {
            continue;
        };
        piers.retain(|(p, floor)| {
            occupied
                .get(p)
                .is_none_or(|runs| runs.iter().all(|(lo, hi)| *lo >= roof || *hi <= *floor))
        });
        if piers.len() < 3 {
            continue;
        }
        let mut cells = Cells::new();
        for (p, floor) in piers {
            add(
                &mut cells,
                p,
                floor,
                roof,
                if id == "plant" { "timber" } else { "stone" },
            );
        }
        for q in -radius - 1..=radius + 1 {
            for r in -radius - 1..=radius + 1 {
                let p = WorldHex::new(root.q + q, root.r + r);
                if q.abs().max(r.abs()).max((q + r).abs()) == radius + 1
                    && occupied.get(&p).is_none_or(|runs| {
                        runs.iter().all(|(lo, hi)| *lo >= roof + 2 || *hi <= roof)
                    })
                {
                    add(
                        &mut cells,
                        p,
                        roof,
                        roof + 2,
                        if id == "plant" { "moss" } else { "stone" },
                    );
                }
            }
        }
        return object(
            g,
            format!("grand/shrine/{id}"),
            "structure/grand-shrine",
            root,
            cells,
        );
    }
    Err(ContractError::new(
        format!("grand/shrine/{id}"),
        "no grounded colonnade outside the authored routes",
    ))
}

/// Reserve a sparse global forest (bounded roots and exact compact occupancies).
pub(super) fn compose(g: &GrandCompiler) -> Result<Vec<ObjectInstance>, ContractError> {
    let mut out = vec![
        world_tree::compose(g, g.geography.frame("world_tree")?.hex([0., 0.]))?,
        temple_plant(g)?,
        root_temple_ribs(g)?,
        fire_marker(g)?,
        air_marker(g)?,
        earth_marker(g)?,
    ];
    for (i, id) in CAMP_FRAMES.into_iter().enumerate() {
        let frame = g.geography.frame(id)?;
        out.push(camp(g, i, &frame)?);
    }
    for (i, root) in coastal_roots(g).into_iter().enumerate() {
        out.push(coastal_rock(g, i, root)?);
    }
    // Giant central crystal, with an accessible shrine on its eastern shoulder.
    let frame = g.geography.frame("crystal_heart")?;
    let root = frame.hex([0., 0.]);
    let floor = g.support_at(&frame, [0., 0.])?.level + 1;
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
    // Sculptures have priority over their architectural frame. This prevents
    // stone piers replacing flame/crystal/foliage voxels in the same column.
    let mut occupied = Occupied::new();
    for landmark in &out {
        reserve(&mut occupied, landmark);
    }
    for id in ["water", "air", "earth", "plant", "fire"] {
        let colonnade = shrine(g, id, &occupied)?;
        reserve(&mut occupied, &colonnade);
        out.push(colonnade);
    }
    // The approved lake-side composition uses the existing small camps and
    // clearings. A fort enclosure would obstruct the shared root-temple approach.
    let landmarks = landmarks::compose(g, &out)?;
    out.extend(landmarks);
    compose_forest(g, &mut out)?;
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

fn temple_plant(g: &GrandCompiler) -> Result<ObjectInstance, ContractError> {
    // Behind the interaction point, inside the ring of temple columns.
    let frame = g.geography.frame("root_temple_altar")?;
    let root = frame.hex([0., 0.]);
    let floor = g.support_at(&frame, [0., 0.])?.level + 1;
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
    // Ground the piers at the room's sides. The narrow approach is reserved
    // across its whole width, so placing piers there would sever their feet.
    let frame = g.geography.frame("root_temple")?;
    let root = frame.hex([0., 0.]);
    let mut cells = Cells::new();
    for north in [-14., 0., 14.] {
        for east in [-17., 17.] {
            let p = frame.hex([east, north]);
            let floor = g.support_at(&frame, [east, north])?.level + 1;
            add(&mut cells, p, floor, floor + 26, "timber");
        }
        for east in -17..=17 {
            let local = [f64::from(east), north];
            let p = frame.hex(local);
            let floor = g.support_at(&frame, local)?.level + 1;
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

fn marker_base(
    g: &GrandCompiler,
    frame: &LandmarkFrame,
) -> Result<(WorldHex, Cells, i32), ContractError> {
    // These small, authored alternatives stay near the claim point. A route
    // terminal may occupy the old rear position, so choose a complete supported
    // pedestal before composing the sculpture rather than clipping off its feet.
    for local in [
        [0., 6.],
        [18., 0.],
        [-18., 0.],
        [0., -18.],
        [0., 18.],
        [26., 0.],
        [-26., 0.],
        [0., -26.],
        [0., 26.],
    ] {
        let root = frame.hex(local);
        let mut supports = Vec::new();
        for q in -2_i64..=2 {
            for r in -2_i64..=2 {
                if q.abs().max(r.abs()).max((q + r).abs()) > 2 {
                    continue;
                }
                let p = WorldHex::new(root.q + q, root.r + r);
                if let Ok(support) = g.support_at(frame, frame.local(p)) {
                    supports.push((p, support.level + 1));
                }
            }
        }
        if supports.len() != 19 {
            continue;
        }
        let Some(floor) = supports.iter().map(|(_, floor)| *floor).max() else {
            continue;
        };
        if supports.iter().any(|(p, bottom)| {
            g.reserved_interval(*p, *bottom, floor + 16)
                || g.column(*p)
                    .0
                    .runs
                    .iter()
                    .any(|run| run.bottom < floor + 16 && run.top > *bottom)
        }) {
            continue;
        }
        let mut cells = Cells::new();
        for (p, bottom) in supports {
            add(&mut cells, p, bottom, floor + 2, "stone");
        }
        return Ok((root, cells, floor));
    }
    Err(ContractError::new(
        "grand.shrine_marker",
        format!(
            "no supported unreserved local pedestal around {:?}",
            frame.hex([0., 0.])
        ),
    ))
}

fn fire_marker(g: &GrandCompiler) -> Result<ObjectInstance, ContractError> {
    // A static flame-shaped sculpture behind the claim point. Its opaque warm
    // masonry has ordinary solid behavior; it is not an active fire or hazard.
    let frame = g.geography.frame("shrine_fire")?;
    let (root, mut cells, floor) = marker_base(g, &frame)?;
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
    let frame = g.geography.frame("shrine_air")?;
    let (root, mut cells, floor) = marker_base(g, &frame)?;
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
    let frame = g.geography.frame("shrine_earth")?;
    let (root, mut cells, floor) = marker_base(g, &frame)?;
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

fn camp(
    g: &GrandCompiler,
    index: usize,
    frame: &LandmarkFrame,
) -> Result<ObjectInstance, ContractError> {
    let root = frame.hex([0., 30.]);
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
    // One small open-front hut sits beside each encounter clearing. The room
    // stays navigable, and neither a perimeter wall nor a fort is reconstructed.
    let hut = frame.hex([31., 0.]);
    let base = g.surface(hut).level + 1;
    for east in -5_i32..=5 {
        for north in -7_i32..=7 {
            let p = frame.hex([31. + f64::from(east), f64::from(north)]);
            let surface = g.surface(p);
            if surface.water.is_some() || !g.mainland(p) {
                continue;
            }
            let wall = east == 5 || north.abs() == 7 || (east == -5 && north.abs() >= 4);
            let roof = base + 14 + (5 - east.abs());
            if wall {
                add(&mut cells, p, surface.level + 1, roof, "timber");
            }
            add(&mut cells, p, roof, roof + 2, "timber");
            if (east + north + index as i32).rem_euclid(7) == 0 {
                add(&mut cells, p, roof + 2, roof + 3, "moss");
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

fn coastal_roots(g: &GrandCompiler) -> Vec<WorldHex> {
    let mut candidates = vec![];
    for &(r, first, last) in &g.source.mainland_rows {
        for q in first..=last {
            let p = WorldHex::new(q, r);
            if !(2..=5).contains(&grid_value(&g.coast, p, 0)) {
                continue;
            }
            let surface = g.surface(p);
            if surface.water.is_some()
                || surface.level > SEA_TOP + 30
                || g.reserved_interval(p, surface.level + 1, surface.level + 12)
            {
                continue;
            }
            candidates.push((forest_hash(g.source.seed ^ 0x0043_4f41_5354, p), p));
        }
    }
    candidates.sort_unstable();
    let mut roots: Vec<WorldHex> = vec![];
    for (_, p) in candidates {
        if roots
            .iter()
            .any(|old| old.checked_distance(p).is_ok_and(|d| d < 80))
        {
            continue;
        }
        roots.push(p);
        if roots.len() == 4 {
            break;
        }
    }
    roots
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
pub(super) fn test_compiler(dressed: bool) -> GrandCompiler {
    let mut source: GrandSpec = ron::from_str(include_str!(
        "../../../../../assets/config/v4/grand-v4/world.ron"
    ))
    .expect("Grand source");
    source.full_dressing = dressed;
    source.geography = Some("geography-r02.json".into());
    let bytes = include_bytes!("../../../../../assets/config/v4/grand-v4/geography-r02.json");
    let geography: GrandGeographyDocument =
        serde_json::from_slice(bytes).expect("approved geography");
    GrandCompiler::with_geography(source, geography, bytes).expect("complete requested geography")
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
    fn shrine_markers_keep_complete_grounded_pedestals_outside_body_reservations() {
        let g = super::super::tests::compiler(false);
        for (id, marker) in [
            ("fire", fire_marker(g)),
            ("air", air_marker(g)),
            ("earth", earth_marker(g)),
        ] {
            let marker = marker.expect("nearby grounded marker");
            assert_eq!(marker.occupancy.len(), 19, "{id}");
            assert_eq!(
                marker.grounding.as_ref().expect("actual contacts").len(),
                19,
                "{id}"
            );
            let frame = g
                .geography
                .frame(&format!("shrine_{id}"))
                .expect("claim frame");
            let [east, north] = frame.local(marker.origin.column);
            assert!(east.hypot(north) < 28., "{id}: bounded local placement");
            for column in &marker.occupancy {
                let (terrain, _) = g.column(column.position);
                for run in &column.runs {
                    assert!(
                        !g.reserved_interval(column.position, run.bottom, run.top),
                        "{id}: claimed walking space"
                    );
                    assert!(
                        !terrain
                            .runs
                            .iter()
                            .any(|t| t.bottom < run.top && t.top > run.bottom),
                        "{id}: terrain intersection"
                    );
                }
            }
        }
        for id in ["water", "air", "earth", "plant", "fire"] {
            let colonnade = shrine(g, id, &Occupied::new()).expect("grounded shrine perimeter");
            assert!(
                colonnade.grounding.as_ref().expect("piers").len() >= 3,
                "{id}"
            );
        }
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
        let g = test_compiler(false);
        let mut transitional = 0_usize;
        let mut dense = 0_usize;
        for &(r, a, b) in &g.source.mainland_rows {
            for q in a..=b {
                let p = WorldHex::new(q, r);
                let density = forest_density(&g, p);
                transitional += usize::from(density > 0.05 && density < 0.65);
                dense += usize::from(density > 0.70);
            }
        }
        assert!(
            transitional > 1_000,
            "forest edge must retain a broad gradient"
        );
        assert!(
            dense > 10_000,
            "substantial interior woods, not edge-only decoration"
        );
        for name in CAMP_FRAMES {
            let frame = g.geography.frame(name).expect("clearing");
            assert!(reserved_growth(&g, frame.hex([0., 0.])), "{name}");
        }
        for name in ["world_tree", "camp_06"] {
            let frame = g.geography.frame(name).expect("landmark glade");
            assert!(
                (g.geography.landmark_glade_weight(frame.hex([0., 0.])) - 1.).abs() < f64::EPSILON
            );
        }
    }

    #[test]
    fn authored_forest_is_bounded_grounded_layered_and_clear_of_gameplay_sites() {
        let g = test_compiler(true);
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
            assert!(
                tree.occupancy
                    .iter()
                    .all(|column| g.geography.landmark_glade_weight(column.position) < 1.),
                "whole ordinary crown enters the root/lakeside glade: {}",
                tree.id
            );
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
                    terrain
                        .material_at(contact.level)
                        .is_some_and(|material| material != "water"),
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
                let (terrain, _) = g.column(column.position);
                assert!(
                    column.runs.iter().all(|run| terrain
                        .runs
                        .iter()
                        .all(|old| old.top <= run.bottom || run.top <= old.bottom)),
                    "tree intersects composed terrain: {} at {:?}",
                    tree.id,
                    column.position
                );
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
                let density = forest_density(&g, p);
                if density <= 0. || g.surface(p).water.is_some() {
                    continue;
                }
                domain += 1;
                covered += u32::from(canopy.contains(&p));
                if density > 0.70 {
                    core += 1;
                    covered_core += u32::from(canopy.contains(&p));
                }
            }
        }
        // Count exact union of real canopy columns, including clearings in the
        // denominator. The giant landmark is deliberately excluded from coverage.
        let fraction = f64::from(covered) / f64::from(domain);
        let core_fraction = f64::from(covered_core) / f64::from(core);
        println!(
            "GRAND_FOREST trees={} families={} columns={covered}/{domain} coverage={fraction:.4} core={covered_core}/{core} core_coverage={core_fraction:.4}",
            trees.len(),
            families.len()
        );
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
        for anchor in g.anchors.iter().filter(|a| a.role == AnchorRole::Gameplay) {
            assert!(g.clear_support(anchor.position, 8), "{}", anchor.id);
        }
    }
}
