//! Deterministic opaque coarse foliage with exact exposed woody silhouettes.
use super::*;

const MAX_FOLIAGE_CELLS: usize = 512;
const CORNERS: [[f32; 2]; 6] = [
    [0., 1.],
    [0.866_025_4, 0.5],
    [0.866_025_4, -0.5],
    [0., -1.],
    [-0.866_025_4, -0.5],
    [-0.866_025_4, 0.5],
];
const NEIGHBORS: [(i64, i64); 6] = [(0, 1), (1, 0), (1, -1), (0, -1), (-1, 0), (-1, 1)];

/// Renderer-neutral indexed opaque mesh; coordinates use radius one/height0.35.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ForestMesh {
    /// Local physical vertex positions.
    pub positions: Vec<[f32; 3]>,
    /// Flat outward normals, with no smoothing across material faces.
    pub normals: Vec<[f32; 3]>,
    /// Authored material ID per vertex, resolved through the package palette.
    pub materials: Vec<String>,
    /// Triangle indices into the parallel vertex arrays.
    pub indices: Vec<u32>,
}
impl ForestMesh {
    fn face(
        &mut self,
        positions: &[[f32; 3]],
        normal: [f32; 3],
        material: &str,
    ) -> Result<(), ContractError> {
        if self.positions.len() + positions.len() > MAX_FOREST_SHAPE_VERTICES {
            return Err(invalid("forest shape exceeds vertex budget"));
        }
        let base = u32::try_from(self.positions.len())
            .map_err(|error| invalid(&format!("forest mesh index overflow: {error}")))?;
        for i in 1..positions.len() - 1 {
            let i = u32::try_from(i)
                .map_err(|error| invalid(&format!("forest face index overflow: {error}")))?;
            self.indices.extend([base, base + i, base + i + 1]);
        }
        self.positions.extend_from_slice(positions);
        self.normals
            .extend(std::iter::repeat_n(normal, positions.len()));
        self.materials
            .extend(std::iter::repeat_n(material.to_owned(), positions.len()));
        Ok(())
    }
}
/// Derive a bounded mesh from exact source intervals and exact local removals.
///
/// Foliage removals retire the entire contributing coarse cell: a conservative
/// distant hole cannot resurrect carved geometry. Woody cells keep exact cut
/// boundaries. Roots and timber covered by an intact coarse crown are omitted
/// only from this disposable surface; every source interval remains authoritative.
pub fn forest_mesh(
    source: &[ColumnData],
    removed: &BTreeSet<VoxelPosition>,
) -> Result<ForestMesh, ContractError> {
    if source.len() > 512 || source.iter().map(|c| c.runs.len()).sum::<usize>() > 1600 {
        return Err(invalid("forest mesh source budget exceeded"));
    }
    let mut foliage: BTreeMap<(WorldHex, i32), BTreeMap<String, i32>> = BTreeMap::new();
    let leaf_bottom = source
        .iter()
        .flat_map(|c| &c.runs)
        .filter(|r| r.material != "timber")
        .map(|r| r.bottom)
        .min()
        .unwrap_or(0);
    let leaf_top = source
        .iter()
        .flat_map(|c| &c.runs)
        .filter(|r| r.material != "timber")
        .map(|r| r.top)
        .max()
        .unwrap_or(0);
    let supports: BTreeSet<_> = source
        .iter()
        .flat_map(|c| {
            c.runs
                .iter()
                .filter(|r| r.material == "timber" && r.bottom <= 0)
                .map(move |r| (c.position, r.bottom))
        })
        .collect();
    let mut retired = BTreeSet::new();
    for column in source {
        column.validate()?;
        if column.position.checked_distance(WorldHex::new(0, 0))? > 16 {
            return Err(invalid("forest mesh column out of range"));
        }
        for run in &column.runs {
            if run.bottom < -8 || run.top > 114 || !legal_material(&run.material) {
                return Err(invalid("forest mesh interval out of range"));
            }
            if run.material == "timber" {
                continue;
            }
            let coarse = forest_foliage_column(column.position);
            for band in run.bottom.div_euclid(FOREST_FOLIAGE_LEVELS)
                ..=(run.top - 1).div_euclid(FOREST_FOLIAGE_LEVELS)
            {
                let lo = run.bottom.max(band * FOREST_FOLIAGE_LEVELS);
                let hi = run.top.min((band + 1) * FOREST_FOLIAGE_LEVELS);
                *foliage
                    .entry((coarse, band))
                    .or_default()
                    .entry(run.material.clone())
                    .or_default() += hi - lo;
            }
            for cut in removed.range(
                VoxelPosition {
                    column: column.position,
                    level: run.bottom,
                }..VoxelPosition {
                    column: column.position,
                    level: run.top,
                },
            ) {
                retired.insert((coarse, cut.level.div_euclid(FOREST_FOLIAGE_LEVELS)));
            }
        }
    }
    if foliage.len() > MAX_FOLIAGE_CELLS {
        return Err(invalid("forest foliage-cell budget exceeded"));
    }
    for cell in retired {
        foliage.remove(&cell);
    }
    let mut crown: BTreeMap<WorldHex, Vec<VoxelRun>> = BTreeMap::new();
    for ((position, band), weights) in foliage {
        let bottom = (band * FOREST_FOLIAGE_LEVELS).max(leaf_bottom);
        let top = ((band + 1) * FOREST_FOLIAGE_LEVELS).min(leaf_top);
        let material = weights
            .into_iter()
            .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
            .ok_or_else(|| invalid("forest foliage has no source material"))?
            .0;
        append(crown.entry(position).or_default(), bottom, top, &material);
    }
    let mut timber = BTreeMap::new();
    for column in source {
        let mut runs = Vec::new();
        let mut masks = crown
            .get(&forest_foliage_column(column.position))
            .cloned()
            .unwrap_or_default();
        masks.extend(
            removed
                .range(
                    VoxelPosition {
                        column: column.position,
                        level: -8,
                    }..=VoxelPosition {
                        column: column.position,
                        level: 114,
                    },
                )
                .map(|p| VoxelRun {
                    bottom: p.level,
                    top: p.level + 1,
                    material: String::new(),
                }),
        );
        masks.sort_by_key(|r| r.bottom);
        for run in column.runs.iter().filter(|r| r.material == "timber") {
            for (lo, hi) in exposed(run.bottom, run.top, &masks) {
                append(&mut runs, lo, hi, "timber");
            }
        }
        if !runs.is_empty() {
            timber.insert(column.position, runs);
        }
    }
    let mut mesh = ForestMesh::default();
    emit(
        &mut mesh,
        &crown,
        FOREST_FOLIAGE_RADIUS as f32,
        &BTreeSet::new(),
    )?;
    emit(&mut mesh, &timber, 1., &supports)?;
    Ok(mesh)
}
fn append(runs: &mut Vec<VoxelRun>, bottom: i32, top: i32, material: &str) {
    if let Some(last) = runs
        .last_mut()
        .filter(|r| r.top == bottom && r.material == material)
    {
        last.top = top;
    } else {
        runs.push(VoxelRun {
            bottom,
            top,
            material: material.into(),
        });
    }
}
fn exposed(bottom: i32, top: i32, neighbors: &[VoxelRun]) -> Vec<(i32, i32)> {
    let mut cursor = bottom;
    let mut out = Vec::new();
    for run in neighbors {
        if run.top <= cursor {
            continue;
        }
        if run.bottom >= top {
            break;
        }
        if cursor < run.bottom {
            out.push((cursor, run.bottom.min(top)));
        }
        cursor = cursor.max(run.top);
        if cursor >= top {
            break;
        }
    }
    if cursor < top {
        out.push((cursor, top));
    }
    out
}
#[expect(
    clippy::cast_precision_loss,
    reason = "Validated tree coordinates have radius16 and levels-8..114; coarse columns are smaller."
)]
fn emit(
    mesh: &mut ForestMesh,
    columns: &BTreeMap<WorldHex, Vec<VoxelRun>>,
    radius: f32,
    supports: &BTreeSet<(WorldHex, i32)>,
) -> Result<(), ContractError> {
    for (position, runs) in columns {
        let x = (position.q as f32 + position.r as f32 * 0.5) * 1.732_050_8 * radius;
        let z = position.r as f32 * 1.5 * radius;
        for run in runs {
            let bottom = run.bottom as f32 * 0.35;
            let top = run.top as f32 * 0.35;
            if !runs.iter().any(|r| r.bottom == run.top) {
                mesh.face(
                    &CORNERS.map(|[a, b]| [x + a * radius, top, z + b * radius]),
                    [0., 1., 0.],
                    &run.material,
                )?;
            }
            if !runs.iter().any(|r| r.top == run.bottom)
                && !supports.contains(&(*position, run.bottom))
            {
                let mut vertices = CORNERS.map(|[a, b]| [x + a * radius, bottom, z + b * radius]);
                vertices.reverse();
                mesh.face(&vertices, [0., -1., 0.], &run.material)?;
            }
            for ((a, b), (dq, dr)) in CORNERS
                .iter()
                .zip(CORNERS.iter().cycle().skip(1))
                .zip(NEIGHBORS)
            {
                let neighbor = WorldHex::new(position.q + dq, position.r + dr);
                let coverage = columns.get(&neighbor).map_or(&[][..], Vec::as_slice);
                let [ax, az] = *a;
                let [bx, bz] = *b;
                let normal = [az - bz, 0., bx - ax];
                for (lo, hi) in exposed(run.bottom, run.top, coverage) {
                    let lo = lo as f32 * 0.35;
                    let hi = hi as f32 * 0.35;
                    mesh.face(
                        &[
                            [x + ax * radius, lo, z + az * radius],
                            [x + bx * radius, lo, z + bz * radius],
                            [x + bx * radius, hi, z + bz * radius],
                            [x + ax * radius, hi, z + az * radius],
                        ],
                        normal,
                        &run.material,
                    )?;
                }
            }
        }
    }
    Ok(())
}
