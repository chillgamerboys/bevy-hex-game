//! Disposable source-exact interval faces around certified exterior caps.
use super::build::{Domain, Source, CORNERS};
use super::{invalid, MacroSurface, SolidRun};
use hex_world_contracts::{ChunkId, ContractError, WorldHex};

const DIRECTIONS: [[i64; 2]; 6] = [[1, 0], [1, -1], [0, -1], [-1, 0], [-1, 1], [0, 1]];
const EPSILON: f64 = 1.0e-9;

/// Disposable native-space vertex. It does not replace a collision or source fact.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FacePoint {
    /// Global axial coordinates multiplied by three, including clipped edge points.
    pub lattice: [f64; 2],
    /// Height in source voxel levels, possibly between levels at a macro interface.
    pub level: f64,
}
/// Distinguishes unchanged physical strata from the certified exterior approximation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaceKind {
    /// A certified macro triangle; consumers retain their existing macro color sampling.
    Macro,
    /// An exact exposed horizontal solid cap.
    Cap,
    /// An exact downward-facing ceiling over a source void.
    Ceiling,
    /// An owned material interval exposed against the adjacent rendered solid intervals.
    Side,
}
/// A convex, consistently wound polygon ready for bounded fan triangulation.
#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceFace {
    /// Native positions; shared boundaries use the same source corner rule.
    pub points: Vec<FacePoint>,
    /// Exact material registry index. Macro triangles use the existing macro palette.
    pub material: Option<u16>,
    /// Presentation ownership of this face.
    pub kind: FaceKind,
}
/// One bounded, disposable chunk face product; it is never serialized as source.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SurfaceFaces {
    /// Exact strata plus certified macro triangles, in canonical source order.
    pub faces: Vec<SurfaceFace>,
    /// Conservative unshared vertex count, before identical attribute interning.
    pub vertices: usize,
    /// Triangle count after convex fan triangulation.
    pub triangles: usize,
}
impl SurfaceFaces {
    fn push(
        &mut self,
        points: Vec<FacePoint>,
        material: Option<u16>,
        kind: FaceKind,
        maximum_vertices: usize,
    ) -> Result<(), ContractError> {
        if points.len() < 3 {
            return Ok(());
        }
        let Some(a) = points.first() else {
            return Ok(());
        };
        // Native (q3, level, r3) is an invertible linear map of world space.
        // A zero-area clipping remnant must not become a duplicate boundary face.
        let area: f64 = points
            .windows(2)
            .skip(1)
            .map(|pair| {
                let [b, c] = pair else { return 0.0 };
                let u = [
                    b.lattice[0] - a.lattice[0],
                    b.level - a.level,
                    b.lattice[1] - a.lattice[1],
                ];
                let v = [
                    c.lattice[0] - a.lattice[0],
                    c.level - a.level,
                    c.lattice[1] - a.lattice[1],
                ];
                (u[1] * v[2] - u[2] * v[1]).abs()
                    + (u[2] * v[0] - u[0] * v[2]).abs()
                    + (u[0] * v[1] - u[1] * v[0]).abs()
            })
            .sum();
        if area <= EPSILON {
            return Ok(());
        }
        self.vertices = self
            .vertices
            .checked_add(points.len())
            .ok_or_else(|| invalid("face vertex count overflow"))?;
        if self.vertices > maximum_vertices {
            return Err(invalid("explicit face vertex budget exceeded"));
        }
        self.triangles += points.len() - 2;
        self.faces.push(SurfaceFace {
            points,
            material,
            kind,
        });
        Ok(())
    }
}

// A side polygon uses t along one exact native hex edge and source level vertically.
type Point = [f64; 2];
fn clip(poly: &[Point], line: impl Fn(Point) -> f64, positive: bool) -> Vec<Point> {
    let mut out = Vec::new();
    for (&a, &b) in poly
        .iter()
        .zip(poly.iter().cycle().skip(1))
        .take(poly.len())
    {
        let da = line(a);
        let db = line(b);
        let inside = |distance: f64| {
            if positive {
                distance >= -EPSILON
            } else {
                distance <= EPSILON
            }
        };
        let ia = inside(da);
        let ib = inside(db);
        if ia {
            out.push(a);
        }
        if ia != ib {
            let t = da / (da - db);
            out.push([a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])]);
        }
    }
    out.dedup_by(|a, b| (a[0] - b[0]).abs() <= EPSILON && (a[1] - b[1]).abs() <= EPSILON);
    if out
        .first()
        .zip(out.last())
        .is_some_and(|(a, b)| (a[0] - b[0]).abs() <= EPSILON && (a[1] - b[1]).abs() <= EPSILON)
    {
        out.pop();
    }
    out
}
fn at([a, b]: [f64; 2], t: f64) -> f64 {
    a + (b - a) * t
}
fn merged(runs: &[SolidRun]) -> Vec<[f64; 2]> {
    let mut result: Vec<[f64; 2]> = Vec::new();
    for run in runs {
        if let Some(last) = result
            .last_mut()
            .filter(|last| (last[1] - f64::from(run.bottom)).abs() <= EPSILON)
        {
            last[1] = f64::from(run.top);
        } else {
            result.push([f64::from(run.bottom), f64::from(run.top)]);
        }
    }
    result
}
fn corner(
    domain: &Domain,
    source: &Source<'_>,
    column: WorldHex,
    local: [i16; 2],
) -> Result<f64, ContractError> {
    if source.unprotected_top(column).is_some() {
        return domain
            .vertex(source, local)
            .map(|v| f64::from(v.half_level) * 0.5);
    }
    Ok(source
        .profile(column)?
        .and_then(|(p, _)| p.runs.last())
        .map_or(0.0, |r| f64::from(r.top)))
}

pub(super) fn build_faces(
    source: &Source<'_>,
    coordinate: ChunkId,
    surface: &MacroSurface,
    maximum_vertices: usize,
) -> Result<SurfaceFaces, ContractError> {
    let domain = Domain::new(source, coordinate)?;
    let origin = coordinate.origin()?;
    let [oq, or] = [origin.q, origin.r].map(|v| {
        v.checked_mul(3)
            .ok_or_else(|| invalid("native world coordinate overflow"))
    });
    let (oq, or) = (oq?, or?);
    #[expect(
        clippy::cast_precision_loss,
        reason = "validated finite-world native coordinates are below the exact f64 integer envelope"
    )]
    let point = |p: [f64; 2], level| FacePoint {
        lattice: [oq as f64 + p[0], or as f64 + p[1]],
        level,
    };
    let mut result = SurfaceFaces::default();
    for tri in &surface.triangles {
        let points = tri
            .iter()
            .map(|&i| {
                surface
                    .vertices
                    .get(usize::from(i))
                    .map(|v| point(v.lattice.map(f64::from), f64::from(v.half_level) * 0.5))
                    .ok_or_else(|| invalid("missing certified face vertex"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        result.push(points, None, FaceKind::Macro, maximum_vertices)?;
    }
    for r in 0..16_i16 {
        for q in 0..16_i16 {
            let column = origin.checked_add(WorldHex::new(i64::from(q), i64::from(r)))?;
            let Some((profile, protection)) = source.profile(column)? else {
                continue;
            };
            if profile.runs.is_empty() {
                continue;
            }
            let macro_cap = protection == 0;
            let corners = CORNERS.map(|[a, b]| [3 * q + a, 3 * r + b]);
            if !macro_cap {
                for run in &profile.runs {
                    if !profile.runs.iter().any(|n| n.bottom == run.top) {
                        result.push(
                            corners
                                .iter()
                                .map(|&c| point(c.map(f64::from), f64::from(run.top)))
                                .collect(),
                            Some(run.material),
                            FaceKind::Cap,
                            maximum_vertices,
                        )?;
                    }
                    if run.bottom > 0 && !profile.runs.iter().any(|n| n.top == run.bottom) {
                        result.push(
                            corners
                                .iter()
                                .rev()
                                .map(|&c| point(c.map(f64::from), f64::from(run.bottom)))
                                .collect(),
                            Some(run.material),
                            FaceKind::Ceiling,
                            maximum_vertices,
                        )?;
                    }
                }
            }
            for (side, [dq, dr]) in DIRECTIONS.into_iter().enumerate() {
                let neighbor = column.checked_add(WorldHex::new(dq, dr))?;
                let adjacent = source.profile(neighbor)?;
                let neighbor_macro =
                    adjacent.is_some_and(|(p, flags)| flags == 0 && !p.runs.is_empty());
                if macro_cap && neighbor_macro {
                    continue;
                }
                let a = *corners
                    .get(side)
                    .ok_or_else(|| invalid("face corner index"))?;
                let b = *corners
                    .get((side + 1) % 6)
                    .ok_or_else(|| invalid("face corner index"))?;
                let tops = [
                    corner(&domain, source, column, a)?,
                    corner(&domain, source, column, b)?,
                ];
                let neighbor_tops = [
                    corner(&domain, source, neighbor, a)?,
                    corner(&domain, source, neighbor, b)?,
                ];
                let empty = Vec::new();
                let neighbor_runs = adjacent.map_or(empty.as_slice(), |(p, _)| p.runs.as_slice());
                let solids = merged(neighbor_runs);
                for (index, run) in profile.runs.iter().enumerate() {
                    let lo = f64::from(run.bottom);
                    let hi = f64::from(run.top);
                    let last = index + 1 == profile.runs.len();
                    let roof = if macro_cap && last { tops } else { [hi; 2] };
                    let mut poly = vec![[0.0, lo], [1.0, lo], [1.0, roof[1]], [0.0, roof[0]]];
                    poly = clip(&poly, |p| p[1] - lo, true);
                    if macro_cap {
                        poly = clip(&poly, |p| p[1] - at(tops, p[0]), false);
                    }
                    let mut parts = vec![poly];
                    for (n, &[bottom, top]) in solids.iter().enumerate() {
                        let roof = if neighbor_macro && n + 1 == solids.len() {
                            neighbor_tops
                        } else {
                            [top; 2]
                        };
                        if roof.into_iter().any(|h| h < bottom - EPSILON) {
                            return Err(invalid("macro roof crossed a protected floor"));
                        }
                        let mut next = Vec::new();
                        for poly in parts {
                            let below = clip(&poly, |p| p[1] - bottom, false);
                            let above = clip(&poly, |p| p[1] - at(roof, p[0]), true);
                            if below.len() >= 3 {
                                next.push(below);
                            }
                            if above.len() >= 3 {
                                next.push(above);
                            }
                        }
                        if next.len() > maximum_vertices {
                            return Err(invalid("side clipping work budget exceeded"));
                        }
                        parts = next;
                    }
                    for poly in parts {
                        let a = a.map(f64::from);
                        let b = b.map(f64::from);
                        let points = poly
                            .into_iter()
                            .map(|[t, y]| {
                                point([a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])], y)
                            })
                            .collect();
                        result.push(
                            points,
                            Some(run.material),
                            FaceKind::Side,
                            maximum_vertices,
                        )?;
                    }
                }
            }
        }
    }
    Ok(result)
}
