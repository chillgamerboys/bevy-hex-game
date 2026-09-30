//! One globally composed, exact World Tree, shared by detailed and distant views.
use super::*;

#[derive(Clone, Copy)]
struct Lobe {
    east: f64,
    north: f64,
    height: f64,
    width: f64,
    depth: f64,
    vertical: f64,
}
// The original woody bough targets are independent of their leaf envelopes.
// Crown editing must not silently reshape the accepted trunk, roots or forks.
const BOUGHS: [[f64; 3]; 7] = [
    [-0.55, 0.14, 0.59],
    [0.55, -0.08, 0.65],
    [0.10, -0.54, 0.53],
    [-0.15, 0.55, 0.65],
    [-0.42, -0.40, 0.49],
    [0.40, 0.42, 0.58],
    [0.21, 0.17, 0.77],
];
// Unequal spreading crowns occupy distinct heights above the exposed forks.
// Their extremes retain the shared footprint and the narrow leader retains the
// full height; air between the layers is part of the authored silhouette.
const LOBES: [Lobe; 8] = [
    Lobe {
        east: -0.04,
        north: 0.02,
        height: 0.84,
        width: 0.27,
        depth: 0.25,
        vertical: 0.16,
    },
    Lobe {
        east: -0.55,
        north: 0.14,
        height: 0.64,
        width: 0.43,
        depth: 0.31,
        vertical: 0.14,
    },
    Lobe {
        east: 0.55,
        north: -0.08,
        height: 0.70,
        width: 0.43,
        depth: 0.28,
        vertical: 0.13,
    },
    Lobe {
        east: 0.10,
        north: -0.54,
        height: 0.50,
        width: 0.36,
        depth: 0.44,
        vertical: 0.14,
    },
    Lobe {
        east: -0.15,
        north: 0.55,
        height: 0.78,
        width: 0.31,
        depth: 0.43,
        vertical: 0.13,
    },
    Lobe {
        east: -0.42,
        north: -0.40,
        height: 0.54,
        width: 0.31,
        depth: 0.30,
        vertical: 0.11,
    },
    Lobe {
        east: 0.40,
        north: 0.42,
        height: 0.60,
        width: 0.31,
        depth: 0.29,
        vertical: 0.13,
    },
    Lobe {
        east: 0.21,
        north: 0.17,
        height: 0.82,
        width: 0.20,
        depth: 0.18,
        vertical: 0.13,
    },
];
#[derive(Clone)]
struct Piece {
    bottom: i32,
    top: i32,
    material: &'static str,
    priority: usize,
}
fn append(out: &mut Vec<VoxelRun>, bottom: i32, top: i32, material: &str) {
    if bottom >= top {
        return;
    }
    if let Some(last) = out.last_mut() {
        if last.top == bottom && last.material == material {
            last.top = top;
            return;
        }
    }
    out.push(run(bottom, top, material));
}
fn levels(base: i32, low: f64, high: f64) -> (i32, i32) {
    (
        base + (low / LEVEL_HEIGHT).floor() as i32,
        base + (high / LEVEL_HEIGHT).ceil() as i32,
    )
}
fn piece(
    out: &mut Vec<Piece>,
    base: i32,
    low: f64,
    high: f64,
    material: &'static str,
    priority: usize,
) {
    let (bottom, top) = levels(base, low, high);
    if bottom < top {
        out.push(Piece {
            bottom,
            top,
            material,
            priority,
        });
    }
}
// Join source intervals once, rather than expanding a monumental crown into
// millions of intermediate cells. Wood wins its actual intersections with leaf.
fn finalized(g: &GrandCompiler, p: WorldHex, pieces: &[Piece]) -> Vec<VoxelRun> {
    let (terrain, _) = g.column(p);
    let mut cuts: Vec<_> = pieces
        .iter()
        .flat_map(|v| [v.bottom, v.top])
        .chain(terrain.runs.iter().flat_map(|v| [v.bottom, v.top]))
        .collect();
    cuts.sort_unstable();
    cuts.dedup();
    let mut out = vec![];
    for pair in cuts.windows(2) {
        let [bottom, top] = *pair else {
            continue;
        };
        if terrain
            .runs
            .iter()
            .any(|v| v.bottom < top && bottom < v.top)
        {
            continue;
        }
        let Some(selected) = pieces
            .iter()
            .filter(|v| v.bottom <= bottom && v.top >= top)
            .max_by_key(|v| v.priority)
        else {
            continue;
        };
        if !g.tree_reserved_interval(p, bottom, top) {
            append(&mut out, bottom, top, selected.material);
            continue;
        }
        // Only intersecting columns need this exact narrow carve. The shared
        // room, doorway and movement reservations remain authoritative.
        for level in bottom..top {
            if !g.tree_reserved_interval(p, level, level + 1) {
                append(&mut out, level, level + 1, selected.material);
            }
        }
    }
    out
}
fn segment_distance(east: f64, north: f64, a: [f64; 2], b: [f64; 2]) -> (f64, f64) {
    let [ae, an] = a;
    let [be, bn] = b;
    let de = be - ae;
    let dn = bn - an;
    let t = (((east - ae) * de + (north - an) * dn) / (de * de + dn * dn).max(0.001)).clamp(0., 1.);
    ((east - ae - de * t).hypot(north - an - dn * t), t)
}

pub(super) fn compose(g: &GrandCompiler, root: WorldHex) -> Result<ObjectInstance, ContractError> {
    let frame = g.geography.frame("world_tree")?;
    let dimensions = g.geography.tree_dimensions();
    let [rx, rz] = dimensions.crown_radii;
    let height = dimensions.height;
    let reach = dimensions.root_reach;
    let scale = frame.length(1.);
    let root = if root == frame.hex([0., 0.]) {
        root
    } else {
        return Err(ContractError::new(
            "grand/world-tree",
            "tree root differs from its shared landmark frame",
        ));
    };
    let base = g.surface(root).level + 1;
    let corners = [-1., 1.]
        .into_iter()
        .flat_map(|x| {
            [-1., 1.].map(|z| frame.hex([x * rx.max(reach) / scale, z * rz.max(reach) / scale]))
        })
        .collect::<Vec<_>>();
    let min_q = corners
        .iter()
        .map(|p| p.q)
        .min()
        .ok_or_else(|| ContractError::new("grand/world-tree", "missing bounds"))?
        - 2;
    let max_q = corners
        .iter()
        .map(|p| p.q)
        .max()
        .ok_or_else(|| ContractError::new("grand/world-tree", "missing bounds"))?
        + 2;
    let min_r = corners
        .iter()
        .map(|p| p.r)
        .min()
        .ok_or_else(|| ContractError::new("grand/world-tree", "missing bounds"))?
        - 2;
    let max_r = corners
        .iter()
        .map(|p| p.r)
        .max()
        .ok_or_else(|| ContractError::new("grand/world-tree", "missing bounds"))?
        + 2;
    let mut occupancy = vec![];
    let mut grounding = vec![];
    for q in min_q..=max_q {
        for r in min_r..=max_r {
            let p = WorldHex::new(q, r);
            let [east, north] = frame.local(p).map(|v| v * scale);
            let mut parts = vec![];
            for (index, lobe) in LOBES.iter().enumerate() {
                let x = (east - lobe.east * rx) / (lobe.width * rx);
                let z = (north - lobe.north * rz) / (lobe.depth * rz);
                let phase = 0.7 + index as f64 * 1.3;
                let angle = z.atan2(x);
                // The accepted Heart sculptor varies the three- and five-fold
                // contours of each unequal bough. Fade this variation at the
                // shared outer bounds so the crown never acquires clipped sides.
                let outer = (east.abs() / rx).max(north.abs() / rz);
                let edge = 1.
                    + smooth(((1. - outer) / 0.18).clamp(0., 1.))
                        * (-0.02
                            + 0.10 * (3. * angle + phase).sin()
                            + 0.065 * (5. * angle - phase * 0.7).cos());
                let radial = (x * x + z * z) / edge.powi(2);
                if radial >= 1. || east.abs() > rx || north.abs() > rz {
                    continue;
                }
                let extent = (1. - radial).sqrt() * lobe.vertical * height;
                // Broad leaf clusters break the regular top and underside at a
                // useful monumental scale. The leader's center retains its height.
                let ripple = ((east * 0.23 + north * 0.11 + phase).sin()
                    + (east * 0.09 - north * 0.19).sin())
                    * height
                    * 0.012
                    * smooth((radial.sqrt() * 4.).clamp(0., 1.));
                let low = (lobe.height * height - extent + ripple * 0.55).max(0.);
                let high = (lobe.height * height + extent + ripple * 0.80).min(height);
                // Shared uneven color strata join overlapping boughs instead of
                // painting each complete mass or vertical column a separate hue.
                // Useful lower and upper layers follow the occupied canopy,
                // rather than reducing the light material to tiny summit tips.
                let shade = ((east * 0.11 + north * 0.07).sin()
                    + 0.5 * (east * 0.05 - north * 0.13).cos())
                    * height
                    * 0.008;
                let dark_top = height * 0.48 + shade;
                let light_bottom = height * 0.80 + shade;
                for (bottom, top, material) in [
                    (low, high.min(dark_top), "foliage_dark"),
                    (low.max(dark_top), high.min(light_bottom), "foliage"),
                    (low.max(light_bottom), high, "foliage_light"),
                ] {
                    if bottom < top {
                        piece(&mut parts, base, bottom, top, material, index);
                    }
                }
            }
            // The bending trunk and fractional contour end each woody column at
            // different heights. The root flare is widest near the actual ground.
            if east.hypot(north) < rx.min(rz) * 0.20 {
                let floor = g.surface(p).level + 1;
                let max = base + (height * 0.80 / LEVEL_HEIGHT).ceil() as i32;
                for level in floor..max {
                    let t = ((f64::from(level - base) * LEVEL_HEIGHT) / height).clamp(0., 1.);
                    let bend = smooth((t / 0.8).clamp(0., 1.));
                    let x = east - rx * (0.035 * bend + 0.020 * (std::f64::consts::PI * t).sin());
                    let z = north + rz * 0.025 * bend;
                    let angle = z.atan2(x);
                    let width = rx.min(rz)
                        * (0.13 - 0.095 * (t / 0.8).clamp(0., 1.).powf(0.72)
                            + 0.040 * (-t / 0.075).exp());
                    let contour =
                        1. + 0.10 * (3. * angle + t).sin() + 0.065 * (5. * angle - 1.3 * t).cos();
                    if x * x + z * z <= (width * contour).powi(2) {
                        if let Some(last) = parts.last_mut() {
                            if last.material == "timber" && last.top == level {
                                last.top += 1;
                                continue;
                            }
                        }
                        parts.push(Piece {
                            bottom: level,
                            top: level + 1,
                            material: "timber",
                            priority: 100,
                        });
                    }
                }
            }
            for (index, bough) in BOUGHS.iter().enumerate() {
                let index = index + 1;
                let [bough_east, bough_north, tip_height] = *bough;
                let target = [bough_east * rx, bough_north * rz];
                let elbow = target.map(|v| v * 0.55);
                for (a, b, low, high) in [
                    (
                        [0., 0.],
                        elbow,
                        height * (0.23 + index as f64 * 0.011),
                        height * 0.43,
                    ),
                    (elbow, target, height * 0.43, height * tip_height),
                ] {
                    let (distance, t) = segment_distance(east, north, a, b);
                    let width = rx.min(rz) * 0.047 * (1. - 0.65 * t);
                    if distance <= width {
                        let center = low + (high - low) * t;
                        let half = (1. - (distance / width).powi(2)).sqrt() * width;
                        piece(
                            &mut parts,
                            base,
                            center - half,
                            center + half,
                            "timber",
                            100,
                        );
                    }
                }
            }
            // Root fins have a grounded far tip, irregular length and a broad
            // buttress. Exact layer reservations cut usable doors through them.
            for arm in 0..7 {
                let angle = arm as f64 * std::f64::consts::TAU / 7. + 0.21;
                let along = east * angle.cos() + north * angle.sin();
                let side = -east * angle.sin() + north * angle.cos();
                let length = reach * (0.82 + 0.18 * ((arm * 3) % 7) as f64 / 6.);
                let width = rx.min(rz) * 0.065 * (1. - 0.68 * (along / length).clamp(0., 1.));
                let bend = (along / length * std::f64::consts::PI).sin() * width * 0.55;
                if along > 0. && along < length && (side - bend).abs() < width {
                    let surface = g.surface(p);
                    if surface.water.is_none() && g.mainland(p) {
                        let bottom = surface.level + 1;
                        let rise = (height
                            * 0.19
                            * (1. - along / length).powf(1.7)
                            * (1. - ((side - bend) / width).powi(2)))
                        .max(LEVEL_HEIGHT);
                        parts.push(Piece {
                            bottom,
                            top: bottom + (rise / LEVEL_HEIGHT).ceil() as i32,
                            material: "timber",
                            priority: 100,
                        });
                    }
                }
            }
            if parts.is_empty() {
                continue;
            }
            let runs = finalized(g, p, &parts);
            if runs.is_empty() {
                continue;
            }
            let (terrain, _) = g.column(p);
            if let Some(run) = runs.iter().rev().find(|run| {
                terrain
                    .runs
                    .iter()
                    .any(|t| t.top == run.bottom && t.material != "water")
            }) {
                grounding.push(VoxelPosition {
                    column: p,
                    level: run.bottom - 1,
                });
            }
            occupancy.push(ColumnData { position: p, runs });
        }
    }
    let object = ObjectInstance {
        id: "grand/world-tree".into(),
        region_id: "grand".into(),
        asset: "plant/grand-world-tree".into(),
        origin: VoxelPosition {
            column: root,
            level: base,
        },
        rotation: 0,
        occupancy,
        grounding: Some(grounding),
    };
    object
        .validate()
        .map_err(|e| ContractError::new("grand/world-tree", e.to_string()))?;
    Ok(object)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Count the same geometric primitives used by the exact landmark surface:
    // a six-vertex cap and four-vertex side for each exposed interval. This is
    // a source-size receipt, independent of the map renderer's own hard limit.
    fn surface_vertices(object: &ObjectInstance) -> usize {
        let columns: BTreeMap<_, _> = object
            .occupancy
            .iter()
            .map(|column| (column.position, &column.runs))
            .collect();
        let mut vertices = 0;
        for column in &object.occupancy {
            for run in &column.runs {
                vertices +=
                    6 * usize::from(!column.runs.iter().any(|other| other.top == run.bottom));
                vertices +=
                    6 * usize::from(!column.runs.iter().any(|other| other.bottom == run.top));
                for (dq, dr) in DIRS {
                    let p = WorldHex::new(column.position.q + dq, column.position.r + dr);
                    let mut exposed = vec![(run.bottom, run.top)];
                    if let Some(neighbor) = columns.get(&p) {
                        for other in *neighbor {
                            let mut remainder = vec![];
                            for (lo, hi) in exposed {
                                if other.top <= lo || other.bottom >= hi {
                                    remainder.push((lo, hi));
                                } else {
                                    if lo < other.bottom {
                                        remainder.push((lo, other.bottom));
                                    }
                                    if other.top < hi {
                                        remainder.push((other.top, hi));
                                    }
                                }
                            }
                            exposed = remainder;
                        }
                    }
                    vertices += 4 * exposed.len();
                }
            }
        }
        vertices
    }

    #[test]
    fn world_tree_retains_approved_bounds_clearance_and_compact_runs() {
        let g = test_compiler(false);
        let frame = g.geography.frame("world_tree").expect("shared tree frame");
        let root = frame.hex([0., 0.]);
        let tree = compose(&g, root).expect("one complete World Tree");
        let dimensions = g.geography.tree_dimensions();
        let scale = frame.length(1.);
        let base = g.surface(root).level + 1;
        let mut max_east = 0_f64;
        let mut max_north = 0_f64;
        let mut max_height = 0_f64;
        let mut low_crown = f64::INFINITY;
        let mut run_count = 0_usize;
        let mut shaded_undersides = 0_usize;
        let mut layered_tops = 0_usize;
        for column in &tree.occupancy {
            let [east, north] = frame.local(column.position).map(|v| v * scale);
            let foliage = column.runs.iter().any(|run| run.material == "foliage");
            shaded_undersides += usize::from(
                foliage && column.runs.iter().any(|run| run.material == "foliage_dark"),
            );
            layered_tops += usize::from(
                foliage
                    && column
                        .runs
                        .iter()
                        .any(|run| run.material == "foliage_light"),
            );
            for run in &column.runs {
                run_count += 1;
                assert!(
                    !g.tree_reserved_interval(column.position, run.bottom, run.top),
                    "tree blocks a shared passage"
                );
                let (terrain, _) = g.column(column.position);
                assert!(!terrain
                    .runs
                    .iter()
                    .any(|ground| ground.bottom < run.top && run.bottom < ground.top));
                if run.material.starts_with("foliage") {
                    max_east = max_east.max(east.abs());
                    max_north = max_north.max(north.abs());
                    max_height = max_height.max(f64::from(run.top - base) * LEVEL_HEIGHT);
                    low_crown = low_crown.min(f64::from(run.bottom - base) * LEVEL_HEIGHT);
                }
            }
        }
        assert!(
            shaded_undersides > 0,
            "underside color must retain vertical layers"
        );
        assert!(
            layered_tops > 0,
            "sunlit crown color must retain vertical layers"
        );
        let [rx, rz] = dimensions.crown_radii;
        assert!(max_east >= rx * 0.96 && max_east <= rx + 1.);
        assert!(max_north >= rz * 0.96 && max_north <= rz + 1.);
        assert!((max_height - dimensions.height).abs() < LEVEL_HEIGHT * 2.);
        assert!(
            max_height - low_crown > dimensions.height * 0.5,
            "the layered crowns must retain substantial overall vertical depth"
        );
        assert!(tree.grounding.as_ref().expect("root supports").len() > 100);
        let vertices = surface_vertices(&tree);
        println!(
            "GRAND_R02_WORLD_TREE columns={} runs={run_count} exact_indexed_vertices={vertices} height={max_height:.3} crown_radii={max_east:.3},{max_north:.3} shaded_undersides={shaded_undersides} layered_tops={layered_tops}",
            tree.occupancy.len()
        );
        // Explicit source-only preview of these exact generated columns. The
        // existing mesh budgets below still decide correctness, even if an
        // over-budget candidate is exported for diagnosis.
        if let Some(path) = std::env::var_os("HEX_GRAND_TREE_STUDY_OUT") {
            let file = std::fs::File::create(path).expect("tree study output");
            serde_json::to_writer(std::io::BufWriter::new(file), &tree)
                .expect("exact tree column export");
        }
        assert!(
            tree.occupancy.len() <= 20_000,
            "exact proxy source-column budget"
        );
        assert!(run_count <= 34_000, "exact proxy compact-run budget");
        assert!(
            vertices <= 750_000,
            "approved bounded exact landmark surface"
        );
        for id in ["root_temple_entrance", "shrine_plant"] {
            let frame = g.geography.frame(id).expect("shared usable entrance");
            let support = g.support_at(&frame, [0., 0.]).expect("interior support");
            assert!(
                tree.occupancy
                    .iter()
                    .filter(|c| c.position == support.column)
                    .flat_map(|c| &c.runs)
                    .all(|run| run.top <= support.level + 1 || run.bottom >= support.level + 9),
                "World Tree must preserve {id}"
            );
        }
    }
}
