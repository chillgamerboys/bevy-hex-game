//! Fixed native hex-prism geometry for compound actor bodies.
//!
//! Each prism has circumradius one, fixed world orientation, and an independently supplied
//! height. Queries allocate no runtime collections. Sweeps use closed geometric
//! contact; movement callers retain responsibility for their existing skin and
//! tangent response. Overlap queries accept the caller's separation tolerance.

use bevy_math::{DVec3, Quat, Vec3};
use hex_core::arena::ArenaVoxelGeometry;
use hex_core::TilePos;

const FACE: f32 = hex_core::config::HEX_SMALL_DIAMETER * 0.5;
// Preserve closure of coincident parallel faces after world-space translation.
// This is one tenth of movement skin; finite intersection distances remain exact.
const FACE_TOLERANCE: f32 = 0.00001;
const AXES: [Vec3; 3] = [
    Vec3::X,
    Vec3::new(0.5, 0.0, FACE),
    Vec3::new(-0.5, 0.0, FACE),
];
const NORMALS: [Vec3; 6] = [
    Vec3::X,
    Vec3::new(0.5, 0.0, FACE),
    Vec3::new(-0.5, 0.0, FACE),
    Vec3::NEG_X,
    Vec3::new(-0.5, 0.0, -FACE),
    Vec3::new(0.5, 0.0, -FACE),
];
const VERTICES: [Vec3; 6] = [
    Vec3::Z,
    Vec3::new(FACE, 0.0, 0.5),
    Vec3::new(FACE, 0.0, -0.5),
    Vec3::NEG_Z,
    Vec3::new(-FACE, 0.0, -0.5),
    Vec3::new(-FACE, 0.0, 0.5),
];

/// One valid, upright native hex prism. Position is its bottom-center.
#[derive(Debug, Clone, Copy)]
pub(crate) struct HexPrism {
    feet: Vec3,
    height: f32,
}

/// First closed contact along a normalized segment parameter in `[0, 1]`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PrismHit {
    /// Segment fraction; zero includes starting inside or touching the solid.
    pub fraction: f32,
    /// Outward obstacle normal, or zero for an initial point inside the volume.
    pub normal: Vec3,
}

/// Minimum horizontal translation separating one convex component from another.
#[derive(Debug, Clone, Copy)]
pub(crate) struct HorizontalContact {
    /// Direction that moves this prism away from the other body.
    pub normal: Vec3,
    /// Translation distance; union callers still resolve other component pairs.
    pub depth: f32,
}

impl HexPrism {
    /// Reject invalid geometry instead of admitting an empty or inverted body.
    pub(crate) fn new(feet: Vec3, height: f32) -> Option<Self> {
        (feet.is_finite() && height.is_finite() && height > 0.0).then_some(Self { feet, height })
    }

    /// Exact closest point on the closed prism, including its rounded-query edges.
    pub(crate) fn closest_point(self, point: Vec3) -> Vec3 {
        self.feet + self.closest_local(point - self.feet)
    }

    /// Euclidean distance to the closed prism, zero for points inside it.
    pub(crate) fn distance(self, point: Vec3) -> f32 {
        point.distance(self.closest_point(point))
    }

    /// The six actual horizontal corners in world space at the prism's base.
    pub(crate) fn horizontal_vertices(self) -> impl Iterator<Item = Vec3> {
        VERTICES.into_iter().map(move |vertex| self.feet + vertex)
    }

    fn closest_local(self, point: Vec3) -> Vec3 {
        let horizontal = point.with_y(0.0);
        let y = point.y.clamp(0.0, self.height);
        if AXES
            .into_iter()
            .all(|axis| horizontal.dot(axis).abs() <= FACE)
        {
            return horizontal.with_y(y);
        }
        edges()
            .map(|(a, b)| {
                let edge = b - a;
                let t = (horizontal - a).dot(edge) / edge.length_squared();
                (a + edge * t.clamp(0.0, 1.0)).with_y(y)
            })
            .min_by(|a, b| {
                a.distance_squared(point)
                    .total_cmp(&b.distance_squared(point))
            })
            .unwrap_or(Vec3::new(0.0, y, 0.0))
    }

    fn contains_local(self, point: Vec3) -> bool {
        point.y >= 0.0
            && point.y <= self.height
            && AXES.into_iter().all(|axis| point.dot(axis).abs() <= FACE)
    }

    fn planes(self) -> impl Iterator<Item = (Vec3, f32)> {
        NORMALS
            .into_iter()
            .map(|normal| (normal, FACE))
            .chain([(Vec3::NEG_Y, 0.0), (Vec3::Y, self.height)])
    }

    fn line_interval(self, start: Vec3, direction: Vec3) -> Option<(f32, f32)> {
        let start = start - self.feet;
        let mut enter = -f32::INFINITY;
        let mut exit = f32::INFINITY;
        for (axis, bound) in self.planes() {
            let distance = start.dot(axis) - bound;
            let speed = direction.dot(axis);
            if speed.abs() <= f32::MIN_POSITIVE {
                if distance > FACE_TOLERANCE {
                    return None;
                }
                continue;
            }
            let t = -distance / speed;
            if speed < 0.0 {
                enter = enter.max(t);
            } else {
                exit = exit.min(t);
            }
            if enter > exit {
                return None;
            }
        }
        Some((enter, exit))
    }

    /// Exact segment against the closed prism. No AABB replaces its hex faces.
    pub(crate) fn sweep_point(self, start: Vec3, delta: Vec3) -> Option<PrismHit> {
        if !start.is_finite() || !delta.is_finite() {
            return None;
        }
        let start = start - self.feet;
        if self.contains_local(start) {
            return Some(PrismHit {
                fraction: 0.0,
                normal: Vec3::ZERO,
            });
        }
        let mut enter = 0.0_f32;
        let mut exit = 1.0_f32;
        let mut normal = Vec3::ZERO;
        for (axis, bound) in self.planes() {
            let distance = start.dot(axis) - bound;
            let speed = delta.dot(axis);
            if speed.abs() <= f32::MIN_POSITIVE {
                if distance > FACE_TOLERANCE {
                    return None;
                }
                continue;
            }
            let t = -distance / speed;
            if speed < 0.0 {
                if t > enter {
                    enter = t;
                    normal = axis;
                }
            } else {
                exit = exit.min(t);
            }
            if enter > exit {
                return None;
            }
        }
        (enter <= 1.0 && exit >= 0.0).then_some(PrismHit {
            fraction: enter.max(0.0),
            normal,
        })
    }

    /// Exact sphere sweep via the prism's faces, finite edge cylinders and vertices.
    ///
    /// Merely expanding all face planes creates sharp artificial corners. This
    /// decomposition preserves the sphere's true rounded Minkowski boundary.
    /// It visits 8 faces, 18 edges and 12 vertices with no heap allocation.
    pub(crate) fn sweep_sphere(self, start: Vec3, delta: Vec3, radius: f32) -> Option<PrismHit> {
        if !start.is_finite() || !delta.is_finite() || !radius.is_finite() || radius < 0.0 {
            return None;
        }
        if radius <= 0.0 {
            return self.sweep_point(start, delta);
        }
        let start = start - self.feet;
        let initial = start - self.closest_local(start);
        if initial.length_squared() <= radius * radius {
            return Some(PrismHit {
                fraction: 0.0,
                normal: initial.normalize_or_zero(),
            });
        }
        let mut best = None;
        for (normal, bound) in self.planes() {
            let speed = delta.dot(normal);
            if speed >= 0.0 {
                continue;
            }
            let t = (bound + radius - start.dot(normal)) / speed;
            if !(0.0..=1.0).contains(&t) {
                continue;
            }
            let point = start + delta * t - normal * radius;
            // The projection must belong to the finite face, not its plane's
            // infinite extension. A tiny arithmetic tolerance is unrelated to
            // movement skin and cannot expand the sphere by a body-sized amount.
            if self.closest_local(point).distance_squared(point) <= 1.0e-10 {
                keep_first(&mut best, t, normal);
            }
        }
        for (a, b) in edges() {
            let top_a = a + Vec3::Y * self.height;
            let top_b = b + Vec3::Y * self.height;
            for (from, to) in [(a, b), (top_a, top_b), (a, top_a)] {
                let edge = to - from;
                let along = edge.normalize();
                let axis = edge.as_dvec3().normalize();
                let relative = (start - from).as_dvec3();
                let motion = delta.as_dvec3();
                let perpendicular = relative - axis * relative.dot(axis);
                let velocity = motion - axis * motion.dot(axis);
                for t in roots(perpendicular, velocity, radius).into_iter().flatten() {
                    let point = start + delta * t;
                    let distance = (point - from).dot(along);
                    if (0.0..=edge.length()).contains(&distance) {
                        let on_edge = from + along * distance;
                        keep_first(&mut best, t, (point - on_edge).normalize_or_zero());
                    }
                }
            }
            for vertex in [a, top_a] {
                let relative = (start - vertex).as_dvec3();
                for t in roots(relative, delta.as_dvec3(), radius)
                    .into_iter()
                    .flatten()
                {
                    keep_first(
                        &mut best,
                        t,
                        (start + delta * t - vertex).normalize_or_zero(),
                    );
                }
            }
        }
        best
    }

    /// Strict voxel overlap using the exact native hex footprint and authored Y.
    pub(crate) fn overlaps_voxel(
        self,
        voxel: TilePos,
        geometry: ArenaVoxelGeometry,
        skin: f32,
    ) -> bool {
        Self::new(
            voxel
                .coord
                .to_world(geometry.top(voxel) - geometry.level_height),
            geometry.level_height,
        )
        .is_some_and(|other| self.overlap_prism(other, skin).is_some())
    }

    /// Minimum horizontal contact between two aligned native hex prisms.
    pub(crate) fn overlap_prism(self, other: Self, skin: f32) -> Option<HorizontalContact> {
        if !vertical_overlap(self.feet.y, self.height, other.feet.y, other.height, skin) {
            return None;
        }
        horizontal_sat(
            self.feet - other.feet,
            AXES.map(|axis| (axis, 2.0 * FACE)),
            skin,
        )
    }

    /// Minimum horizontal contact with an upright, yawed box (the Dragon body).
    pub(crate) fn overlap_box(
        self,
        feet: Vec3,
        size: Vec3,
        yaw: f32,
        skin: f32,
    ) -> Option<HorizontalContact> {
        if !feet.is_finite()
            || !size.is_finite()
            || !yaw.is_finite()
            || size.min_element() <= 0.0
            || !vertical_overlap(self.feet.y, self.height, feet.y, size.y, skin)
        {
            return None;
        }
        let rotation = Quat::from_rotation_y(yaw);
        let right = rotation * Vec3::X;
        let forward = rotation * Vec3::Z;
        let planes = AXES.into_iter().chain([right, forward]).map(|axis| {
            let hex = VERTICES
                .into_iter()
                .map(|vertex| vertex.dot(axis).abs())
                .fold(0.0, f32::max);
            let rectangle =
                size.x * 0.5 * right.dot(axis).abs() + size.z * 0.5 * forward.dot(axis).abs();
            (axis, hex + rectangle)
        });
        horizontal_sat(self.feet - feet, planes, skin)
    }

    /// Exact vertical-capsule intersection and minimum horizontal separation.
    ///
    /// Intersect the capsule's vertical axis with the prism's Y interval first.
    /// A separated cap produces a smaller circular section, not a full cylinder.
    pub(crate) fn overlap_capsule(
        self,
        feet: Vec3,
        height: f32,
        radius: f32,
        skin: f32,
    ) -> Option<HorizontalContact> {
        if !feet.is_finite()
            || !height.is_finite()
            || !radius.is_finite()
            || radius <= 0.0
            || height < radius * 2.0
            || !vertical_overlap(self.feet.y, self.height, feet.y, height, skin)
        {
            return None;
        }
        let low = feet.y + radius;
        let high = feet.y + height - radius;
        let axis_y = (self.feet.y + self.height * 0.5).clamp(low, high);
        let near_y = axis_y.clamp(self.feet.y, self.feet.y + self.height);
        let gap = (axis_y - near_y).abs();
        if gap >= radius {
            return None;
        }
        let section = (radius * radius - gap * gap).sqrt();
        let local = (feet - self.feet).with_y(0.0);
        let nearest = self.closest_local(local).with_y(0.0);
        let separation = nearest - local;
        let distance = separation.length();
        if distance > f32::EPSILON {
            let depth = section - distance;
            return (depth > skin).then_some(HorizontalContact {
                normal: separation / distance,
                depth,
            });
        }
        // The circle center lies inside the hex. Move this prism away along its
        // nearest face; zero-length center deltas need a deterministic real normal.
        NORMALS
            .into_iter()
            .map(|normal| HorizontalContact {
                normal: -normal,
                depth: FACE - local.dot(normal) + section,
            })
            .min_by(|a, b| a.depth.total_cmp(&b.depth))
            .filter(|contact| contact.depth > skin)
    }
}

/// Boundary distance along normalized planar aim, through the union from origin.
///
/// This accepts at most the seven Golem components and allocates nothing. It
/// merges only intervals connected to the component containing the origin;
/// taking the farthest isolated intersection would put a mouth in empty space.
/// Shared-face arithmetic may differ by a few ulps, so merges admit a `1e-5`
/// world-unit tolerance. The returned boundary itself is not expanded by it.
pub(crate) fn planar_union_exit(
    prisms: impl IntoIterator<Item = HexPrism>,
    origin: Vec3,
    aim: Vec3,
) -> Option<f32> {
    if !origin.is_finite() || !aim.is_finite() {
        return None;
    }
    let direction = aim.with_y(0.0).try_normalize()?;
    let mut intervals = [None; 7];
    let mut prisms = prisms.into_iter();
    for slot in &mut intervals {
        let Some(prism) = prisms.next() else {
            break;
        };
        *slot = prism.line_interval(origin, direction);
    }
    if prisms.next().is_some() {
        return None;
    }
    let mut end = intervals
        .into_iter()
        .flatten()
        .filter(|(enter, exit)| *enter <= 0.0 && *exit >= 0.0)
        .map(|(_, exit)| exit)
        .max_by(f32::total_cmp)?;
    // Seven passes also admit arbitrary component order without a sort or heap.
    for _ in 0..7 {
        for (enter, exit) in intervals.into_iter().flatten() {
            if enter <= end + FACE_TOLERANCE && exit >= 0.0 {
                end = end.max(exit);
            }
        }
    }
    end.is_finite().then_some(end)
}

fn edges() -> impl Iterator<Item = (Vec3, Vec3)> {
    VERTICES
        .into_iter()
        .zip(VERTICES.into_iter().cycle().skip(1))
        .take(6)
}

fn vertical_overlap(a: f32, ah: f32, b: f32, bh: f32, skin: f32) -> bool {
    a < b + bh - skin && b < a + ah - skin
}

fn horizontal_sat(
    delta: Vec3,
    planes: impl IntoIterator<Item = (Vec3, f32)>,
    skin: f32,
) -> Option<HorizontalContact> {
    let mut best: Option<HorizontalContact> = None;
    for (axis, extent) in planes {
        let position = delta.dot(axis);
        let depth = extent - position.abs();
        if depth <= skin {
            return None;
        }
        if best.is_none_or(|old| depth < old.depth) {
            best = Some(HorizontalContact {
                normal: if position < 0.0 { -axis } else { axis },
                depth,
            });
        }
    }
    best
}

fn keep_first(best: &mut Option<PrismHit>, fraction: f32, normal: Vec3) {
    if (0.0..=1.0).contains(&fraction) && best.is_none_or(|old| fraction < old.fraction) {
        *best = Some(PrismHit { fraction, normal });
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "only finite normalized segment fractions in [0,1] narrow back to f32"
)]
fn roots(relative: DVec3, velocity: DVec3, radius: f32) -> [Option<f32>; 2] {
    let a = velocity.length_squared();
    let b = 2.0 * relative.dot(velocity);
    let c = relative.length_squared() - f64::from(radius) * f64::from(radius);
    if a <= 0.0 {
        return [None, None];
    }
    let discriminant = b * b - 4.0 * a * c;
    if discriminant < 0.0 {
        return [None, None];
    }
    // The stable quadratic form avoids cancellation on long approaching rays.
    let q = -0.5 * (b + discriminant.sqrt().copysign(b));
    let values = if q.abs() <= f64::EPSILON {
        [-b / (2.0 * a); 2]
    } else {
        [q / a, c / q]
    };
    values.map(|value| {
        if (0.0..=1.0).contains(&value) {
            Some(value as f32)
        } else {
            None
        }
    })
}

#[cfg(test)]
#[path = "hex_prisms_tests.rs"]
mod tests;
