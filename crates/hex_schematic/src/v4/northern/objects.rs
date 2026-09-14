//! Globally reserved, exact voxel blueprints. Terrain is sampled, never expanded.
use super::*;

type Cells = BTreeMap<WorldHex, BTreeMap<i32, &'static str>>;
fn add(cells: &mut Cells, p: WorldHex, bottom: i32, top: i32, material: &'static str) {
    let column = cells.entry(p).or_default();
    for level in bottom..top {
        column.insert(level, material);
    }
}
fn object(
    source: &NorthernSpec,
    id: String,
    asset: &str,
    root: WorldHex,
    cells: Cells,
) -> Result<ObjectInstance, ContractError> {
    let mut occupancy = Vec::new();
    let mut grounding = Vec::new();
    for (position, voxels) in cells {
        let mut runs: Vec<VoxelRun> = Vec::new();
        let ground = source.surface(position).level;
        for (level, material) in voxels {
            if level <= ground {
                continue;
            }
            if let Some(last) = runs.last_mut() {
                if last.top == level && last.material == material {
                    last.top += 1;
                    continue;
                }
            }
            runs.push(VoxelRun {
                bottom: level,
                top: level + 1,
                material: material.into(),
            });
        }
        if runs.is_empty() {
            continue;
        }
        if runs.first().is_some_and(|r| r.bottom == ground + 1) {
            grounding.push(VoxelPosition {
                column: position,
                level: ground,
            });
        }
        occupancy.push(ColumnData { position, runs });
    }
    let output = ObjectInstance {
        id,
        region_id: REGION.into(),
        asset: asset.into(),
        origin: VoxelPosition {
            column: root,
            level: source.surface(root).level + 1,
        },
        rotation: 0,
        occupancy,
        grounding: Some(grounding),
    };
    output.validate()?;
    Ok(output)
}
fn tree(
    source: &NorthernSpec,
    root: WorldHex,
    index: usize,
) -> Result<ObjectInstance, ContractError> {
    let mut cells = Cells::new();
    let floor = source.surface(root).level + 1;
    let height = 48 + (index % 34) as i32;
    let lean = if index.is_multiple_of(2) { 1 } else { -1 };
    for level in 0..height {
        let shift = if level > height * 2 / 3 { lean } else { 0 };
        add(
            &mut cells,
            WorldHex::new(root.q + shift, root.r),
            floor + level,
            floor + level + 1,
            "timber",
        );
        if level < 8 {
            add(
                &mut cells,
                WorldHex::new(root.q, root.r + lean),
                floor + level,
                floor + level + 1,
                "timber",
            );
        }
    }
    // Layered uneven conical crowns, each voxel identical for visible and collision geometry.
    for level in height / 3..height + 4 {
        let radius = (((height + 4 - level) as f64 / (height as f64 * 0.8)) * 4.8).ceil() as i64;
        for dq in -radius..=radius {
            for dr in -radius..=radius {
                if dq.abs().max(dr.abs()).max((dq + dr).abs()) > radius {
                    continue;
                }
                if (dq * 7 + dr * 11 + level as i64 + index as i64).rem_euclid(13) == 0 {
                    continue;
                }
                let p = WorldHex::new(
                    root.q + dq + if level > height * 2 / 3 { lean } else { 0 },
                    root.r + dr,
                );
                if p == root && level < height {
                    continue;
                }
                add(&mut cells, p, floor + level, floor + level + 1, "foliage");
            }
        }
    }
    object(
        source,
        format!("northern/tree/{index:04}"),
        "plant/northern-conifer",
        root,
        cells,
    )
}
fn building(
    source: &NorthernSpec,
    name: &str,
    x: f64,
    z: f64,
    half_width: i64,
    half_length: i64,
) -> Result<ObjectInstance, ContractError> {
    let root = nearest_hex(x, z);
    let floor = source.surface(root).level + 1;
    let mut cells = Cells::new();
    for q in -half_width..=half_width {
        for r in -half_length..=half_length {
            let p = WorldHex::new(root.q + q, root.r + r);
            let ground = source.surface(p).level + 1;
            add(&mut cells, p, ground, floor + 2, "stone");
            let wall = q.abs() == half_width || r.abs() == half_length;
            let doorway = r == half_length && q.abs() <= 1;
            let window = q.abs() == half_width && r.abs() < half_length - 1 && r.rem_euclid(5) == 0;
            if wall {
                if !doorway {
                    add(&mut cells, p, floor + 2, floor + 18, "timber");
                }
                if window {
                    for h in floor + 8..floor + 13 {
                        if let Some(c) = cells.get_mut(&p) {
                            c.remove(&h);
                        }
                    }
                }
            }
            let roof = floor + 18 + (half_width - q.abs()) as i32 * 3;
            add(&mut cells, p, roof, roof + 2, "roof");
            if r.abs() == half_length {
                add(&mut cells, p, floor + 18, roof, "timber");
            }
        }
    }
    // Roof overhang and raised ridge, with pointed end posts.
    for q in -half_width - 1..=half_width + 1 {
        for r in [-half_length - 1, half_length + 1] {
            let roof = floor + 18 + (half_width - q.abs()).max(0) as i32 * 3;
            add(
                &mut cells,
                WorldHex::new(root.q + q, root.r + r),
                roof,
                roof + 2,
                "roof",
            );
        }
    }
    for r in [-half_length, half_length] {
        add(
            &mut cells,
            WorldHex::new(root.q, root.r + r),
            floor + 18 + half_width as i32 * 3,
            floor + 25 + half_width as i32 * 3,
            "timber",
        );
    }
    object(
        source,
        format!("northern/building/{name}"),
        &format!("prop/northern-{name}"),
        root,
        cells,
    )
}
pub(super) fn compose(
    source: &NorthernSpec,
) -> Result<(Vec<ObjectInstance>, usize), ContractError> {
    let mut objects = BUILDING_SITES
        .iter()
        .map(|site| {
            building(
                source,
                site.name,
                site.xz[0],
                site.xz[1],
                site.half_width,
                site.half_length,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let root = nearest_hex(FIELD_SITE.xz[0], FIELD_SITE.xz[1]);
    let mut cells = Cells::new();
    for q in -FIELD_SITE.half_width..=FIELD_SITE.half_width {
        for r in -FIELD_SITE.half_length..=FIELD_SITE.half_length {
            let p = WorldHex::new(root.q + q, root.r + r);
            let floor = source.surface(p).level + 1;
            add(&mut cells, p, floor, floor + 1, "soil");
            if q % 2 == 0 {
                add(&mut cells, p, floor + 1, floor + 3, "crop");
            }
        }
    }
    objects.push(object(
        source,
        "northern/field/cultivation".into(),
        "prop/northern-field",
        root,
        cells,
    )?);
    let mut tree_count = 0;
    for q in (-660_i64..=660).step_by(9) {
        for r in (-660_i64..=660).step_by(9) {
            let p = WorldHex::new(q, r);
            let [x, z] = world_xz(p);
            if !source.full_dressing
                && !((-790.0..=-340.0).contains(&x) && (-80.0..=60.0).contains(&z)
                    || (-185.0..=160.0).contains(&x) && (410.0..=680.0).contains(&z))
            {
                continue;
            }
            if ((x + 6.0) / 79.0).powi(2) + ((z - 575.0) / 62.0).powi(2) < 1.0 {
                continue;
            }
            // Reserve the real player's dry observation area and the view of
            // the central water patch before any tree blueprints are authored.
            let [sx, sz] = SPAWN_XZ;
            let [bx, bz] = BAY_XZ;
            let t = (((x - sx) * (bx - sx) + (z - sz) * (bz - sz))
                / ((bx - sx).powi(2) + (bz - sz).powi(2)))
            .clamp(0.0, 1.0);
            if (x - sx - t * (bx - sx)).hypot(z - sz - t * (bz - sz)) < 15.0 {
                continue;
            }
            let surface = source.surface(p);
            let altitude = (surface.level + 1) as f64 * LEVEL_HEIGHT - 140.0;
            if !(7.0..115.0).contains(&altitude) || surface.material != "moss" {
                continue;
            }
            let variation = (q * 71 + r * 173 + source.seed as i64).rem_euclid(7);
            if variation < 2 {
                continue;
            }
            let neighbors = p.neighbors()?;
            if neighbors
                .iter()
                .any(|n| (source.surface(*n).level - surface.level).abs() > 5)
            {
                continue;
            }
            objects.push(tree(source, p, tree_count)?);
            tree_count += 1;
        }
    }
    Ok((objects, tree_count))
}
