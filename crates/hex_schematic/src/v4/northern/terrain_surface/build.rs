//! Deterministic constrained caps over a source-owned native hex domain.
use super::{
    invalid, MacroSurface, SurfaceChunk, SurfaceVertex, TerrainSurfaceOverview, OUTSIDE_PROFILE,
};
use hex_world_contracts::{ChunkId, ContractError, WorldHex, CHUNK_SIZE};
use spade::{ConstrainedDelaunayTriangulation, HasPosition, Point2, Triangulation};
use std::collections::{BTreeMap, BTreeSet};

pub(super) const CORNERS: [[i16; 2]; 6] = [[1, 1], [2, -1], [1, -2], [-1, -1], [-2, 1], [-1, 2]];
const DIRECTIONS: [[i64; 2]; 6] = [[1, 0], [1, -1], [0, -1], [-1, 0], [-1, 1], [0, 1]];
type Cell = [i16; 2];
type Edge = [[i16; 2]; 2];

/// Read-only complete profile lookup; no source loading or gameplay authority.
pub(super) struct Source<'a> {
    pub(super) overview: &'a TerrainSurfaceOverview,
    chunks: BTreeMap<ChunkId, &'a SurfaceChunk>,
}
impl<'a> Source<'a> {
    pub(super) fn new(overview: &'a TerrainSurfaceOverview) -> Self {
        Self {
            overview,
            chunks: overview.chunks.iter().map(|c| (c.coordinate, c)).collect(),
        }
    }
    fn known(&self, column: WorldHex) -> bool {
        self.chunks.contains_key(&ChunkId::from_world_hex(column))
    }
    pub(super) fn unprotected_top(&self, column: WorldHex) -> Option<i16> {
        let coordinate = ChunkId::from_world_hex(column);
        let chunk = self.chunks.get(&coordinate)?;
        let origin = coordinate.origin().ok()?;
        let index =
            usize::try_from((column.r - origin.r) * CHUNK_SIZE + column.q - origin.q).ok()?;
        if *chunk.protection.get(index)? != 0 {
            return None;
        }
        let id = *chunk.profiles.get(index)?;
        if id == OUTSIDE_PROFILE {
            return None;
        }
        self.overview
            .profiles
            .get(usize::from(id))?
            .runs
            .last()
            .map(|r| r.top)
    }
}

#[derive(Clone, Copy)]
struct Vertex(SurfaceVertex);
impl HasPosition for Vertex {
    type Scalar = f64;
    fn position(&self) -> Point2<f64> {
        let [q, r] = self.0.lattice.map(f64::from);
        // Integer native coordinates keep truly collinear points exactly
        // collinear for the CDT. The nonsingular world transform preserves
        // incidence/coverage; the independent certificate governs refinement.
        Point2::new(q, r)
    }
}

fn add(a: Cell, b: Cell) -> Cell {
    let [x, z] = a;
    let [dx, dz] = b;
    [x + dx, z + dz]
}
fn edge(a: Cell, b: Cell) -> Edge {
    if a < b {
        [a, b]
    } else {
        [b, a]
    }
}

pub(super) struct Domain {
    pub(super) cells: BTreeMap<Cell, i16>,
    pub(super) boundary: BTreeSet<Edge>,
    pub(super) origin: WorldHex,
}
impl Domain {
    pub(super) fn new(source: &Source<'_>, coordinate: ChunkId) -> Result<Self, ContractError> {
        let origin = coordinate.origin()?;
        if !source.chunks.contains_key(&coordinate) {
            return Err(invalid("missing source chunk"));
        }
        let mut cells = BTreeMap::new();
        for r in 0..16_i16 {
            for q in 0..16_i16 {
                if let Some(top) = source.unprotected_top(WorldHex::new(
                    origin.q + i64::from(q),
                    origin.r + i64::from(r),
                )) {
                    cells.insert([q, r], top);
                }
            }
        }
        let mut boundary = BTreeSet::new();
        for &[q, r] in cells.keys() {
            let center = [3 * q, 3 * r];
            for (i, [dq, dr]) in DIRECTIONS.into_iter().enumerate() {
                let nq = i64::from(q) + dq;
                let nr = i64::from(r) + dr;
                let neighbor = i16::try_from(nq)
                    .ok()
                    .zip(i16::try_from(nr).ok())
                    .map(|(a, b)| [a, b]);
                if neighbor.is_some_and(|p| cells.contains_key(&p)) {
                    continue;
                }
                let a = add(
                    center,
                    *CORNERS.get(i).ok_or_else(|| invalid("corner index"))?,
                );
                let b = add(
                    center,
                    *CORNERS
                        .get((i + 1) % 6)
                        .ok_or_else(|| invalid("corner index"))?,
                );
                boundary.insert(edge(a, b));
            }
        }
        Ok(Self {
            cells,
            boundary,
            origin,
        })
    }
    fn global(&self, q: i16, r: i16) -> Result<WorldHex, ContractError> {
        Ok(WorldHex::new(
            self.origin
                .q
                .checked_add(i64::from(q))
                .ok_or_else(|| invalid("corner world coordinate overflow"))?,
            self.origin
                .r
                .checked_add(i64::from(r))
                .ok_or_else(|| invalid("corner world coordinate overflow"))?,
        ))
    }
    pub(super) fn vertex(
        &self,
        source: &Source<'_>,
        p: Cell,
    ) -> Result<SurfaceVertex, ContractError> {
        let [q, r] = p;
        let half_level = if q % 3 == 0 && r % 3 == 0 {
            source
                .unprotected_top(self.global(q / 3, r / 3)?)
                .and_then(|level| level.checked_mul(2))
                .ok_or_else(|| invalid("missing center profile"))?
        } else {
            let mut range: Option<(i16, i16)> = None;
            for [dq, dr] in CORNERS {
                if (q - dq) % 3 != 0 || (r - dr) % 3 != 0 {
                    continue;
                }
                let column = self.global((q - dq) / 3, (r - dr) / 3)?;
                if !source.known(column) {
                    return Err(invalid("missing exact corner halo"));
                }
                if let Some(level) = source.unprotected_top(column) {
                    range = Some(
                        range.map_or((level, level), |(lo, hi)| (lo.min(level), hi.max(level))),
                    );
                }
            }
            let (lo, hi) = range.ok_or_else(|| invalid("missing corner profile/halo"))?;
            lo.checked_add(hi)
                .ok_or_else(|| invalid("half-level overflow"))?
        };
        Ok(SurfaceVertex {
            lattice: p,
            half_level,
        })
    }
    fn contains(&self, p: [f64; 2]) -> bool {
        self.cells
            .keys()
            .any(|&cell| super::certificate::inside_hex(p, cell))
    }
}

/// Build one deterministic cap patch. Complete source profiles and protection
/// must include the query halo so neighboring chunks choose identical corner Y.
/// Every source-cap overlay polygon is certified before the patch is returned.
pub(super) fn build_patch(
    source: &Source<'_>,
    coordinate: ChunkId,
    level_height: f64,
) -> Result<MacroSurface, ContractError> {
    let overview = source.overview;
    if !level_height.is_finite()
        || level_height <= 0.0
        || !overview.tolerance.is_finite()
        || !(0.0..=2.0).contains(&overview.tolerance)
    {
        return Err(invalid("invalid cap error/level scale"));
    }
    let domain = Domain::new(source, coordinate)?;
    if domain.cells.is_empty() {
        return Ok(MacroSurface {
            vertices: Vec::new(),
            triangles: Vec::new(),
            maximum_error: 0.0,
        });
    }
    let mut points: BTreeSet<Cell> = domain.boundary.iter().flatten().copied().collect();
    points.extend(
        domain
            .cells
            .keys()
            .filter(|[q, r]| q % 4 == 0 && r % 4 == 0)
            .map(|[q, r]| [3 * q, 3 * r]),
    );
    // Each unsuccessful iteration adds at least one of the bounded source centers
    // or corners. No tolerance increase or topology fallback can make it pass.
    loop {
        if points.len() > super::MAX_PATCH_VERTICES {
            return Err(invalid("cap vertex budget"));
        }
        let vertices = points
            .iter()
            .map(|&p| domain.vertex(source, p).map(Vertex))
            .collect::<Result<Vec<_>, _>>()?;
        let indices: BTreeMap<Cell, usize> = points
            .iter()
            .copied()
            .enumerate()
            .map(|(i, p)| (p, i))
            .collect();
        let constraints = domain
            .boundary
            .iter()
            .map(|[a, b]| {
                Ok([
                    *indices
                        .get(a)
                        .ok_or_else(|| invalid("missing constraint endpoint"))?,
                    *indices
                        .get(b)
                        .ok_or_else(|| invalid("missing constraint endpoint"))?,
                ])
            })
            .collect::<Result<Vec<_>, ContractError>>()?;
        let mut conflict = false;
        let cdt = ConstrainedDelaunayTriangulation::<Vertex>::try_bulk_load_cdt(
            vertices,
            constraints,
            |_edge| conflict = true,
        )
        .map_err(|error| invalid(&format!("cap triangulation: {error}")))?;
        if conflict || cdt.num_vertices() != points.len() {
            return Err(invalid("conflicting/deduplicated source constraints"));
        }
        let mut triangles = Vec::new();
        for face in cdt.inner_faces() {
            let vertices = face.vertices().map(|v| v.data().0);
            let [a, b, c] = vertices.map(|v| v.lattice);
            let area = super::certificate::area(a, b, c);
            if area == 0 {
                continue;
            }
            let [aq, ar] = a;
            let [bq, br] = b;
            let [cq, cr] = c;
            let center = [f64::from(aq + bq + cq) / 3.0, f64::from(ar + br + cr) / 3.0];
            if !domain.contains(center) {
                continue;
            }
            let mut triangle = face.vertices().map(|v| v.index());
            if area > 0 {
                triangle.swap(1, 2);
            }
            let min = triangle
                .iter()
                .enumerate()
                .min_by_key(|(_, value)| **value)
                .map_or(0, |(i, _)| i);
            triangle.rotate_left(min);
            let [ia, ib, ic] = triangle;
            let index = |value| {
                u16::try_from(value)
                    .map_err(|error| invalid(&format!("cap index overflow: {error}")))
            };
            triangles.push([index(ia)?, index(ib)?, index(ic)?]);
        }
        triangles.sort_unstable();
        let mut surface = MacroSurface {
            vertices: cdt.vertices().map(|v| v.data().0).collect(),
            triangles,
            maximum_error: 0.0,
        };
        if surface.triangles.len() > super::MAX_PATCH_TRIANGLES {
            return Err(invalid("cap triangle budget"));
        }
        let proof =
            super::certificate::inspect(&domain, &surface, level_height, overview.tolerance)?;
        if proof.refine.is_empty() {
            surface.maximum_error = proof.maximum_error;
            return Ok(surface);
        }
        let previous = points.len();
        for [q, r] in proof.refine {
            let center = [3 * q, 3 * r];
            points.insert(center);
            points.extend(CORNERS.map(|c| add(center, c)));
        }
        if previous == points.len() {
            return Err(invalid(
                "native cap cannot meet certified error; protection incomplete",
            ));
        }
    }
}
