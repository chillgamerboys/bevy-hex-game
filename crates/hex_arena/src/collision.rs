//! Swept hex-prism math adapted from the accepted M01 exploration controller.
//! This cache consumes only the arena's world-owned complete voxel projection.

use std::collections::{BTreeMap, BTreeSet};

use bevy_math::Vec3;
use hex_core::arena::{ArenaTerrainView, ArenaVoxelGeometry};
use hex_core::{HexCoord, TilePos};

pub(crate) const SKIN: f32 = 0.0001;
const FACE: f32 = hex_core::config::HEX_SMALL_DIAMETER * 0.5;
const NORMALS: [Vec3; 4] = [
    Vec3::X,
    Vec3::new(0.5, 0.0, 0.866_025_4),
    Vec3::new(-0.5, 0.0, 0.866_025_4),
    Vec3::Y,
];

#[derive(Clone, Copy, Debug)]
pub(crate) struct Span {
    pub coord: HexCoord,
    pub bottom: f32,
    pub top: f32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Hit {
    pub fraction: f32,
    pub normal: Vec3,
}

#[derive(Default, Debug, Clone)]
pub(crate) struct CollisionWorld {
    columns: BTreeMap<HexCoord, Vec<Span>>,
    pub revision: Option<u64>,
}

impl CollisionWorld {
    pub fn refresh(&mut self, view: &ArenaTerrainView, geometry: ArenaVoxelGeometry) {
        if self.revision == Some(view.revision) {
            return;
        }
        self.columns.clear();
        for pos in view.voxels.keys() {
            let top = geometry.top(*pos);
            self.columns.entry(pos.coord).or_default().push(Span {
                coord: pos.coord,
                bottom: top - geometry.level_height,
                top,
            });
        }
        // Merge contiguous material into spans. The published view is sorted,
        // but use physical order here rather than depending on TilePos ordering.
        for spans in self.columns.values_mut() {
            spans.sort_by(|a, b| a.bottom.total_cmp(&b.bottom));
            let mut merged: Vec<Span> = Vec::new();
            for span in spans.drain(..) {
                if let Some(last) = merged.last_mut() {
                    if span.bottom <= last.top + SKIN {
                        last.top = last.top.max(span.top);
                        continue;
                    }
                }
                merged.push(span);
            }
            *spans = merged;
        }
        self.revision = Some(view.revision);
    }

    fn candidates(&self, start: Vec3, end: Vec3, radius: f32) -> impl Iterator<Item = Span> + '_ {
        let mut ring = 1;
        let mut covered = FACE;
        while radius >= covered {
            ring += 1;
            covered += FACE;
        }
        let coords = HexCoord::from_world(start)
            .line_between(HexCoord::from_world(end))
            .into_iter()
            .flat_map(|coord| coord.within_radius(ring))
            .collect::<BTreeSet<_>>();
        coords
            .into_iter()
            .filter_map(|coord| self.columns.get(&coord))
            .flatten()
            .copied()
    }

    pub fn clear(&self, feet: Vec3, height: f32, radius: f32) -> bool {
        feet.is_finite()
            && !self
                .candidates(feet, feet, radius)
                .any(|span| contains(span, feet, height, radius))
    }

    pub fn sweep(&self, feet: Vec3, delta: Vec3, height: f32, radius: f32) -> Option<Hit> {
        self.candidates(feet, feet + delta, radius)
            .filter_map(|span| sweep_span(span, feet, delta, height, radius))
            .min_by(|a, b| a.fraction.total_cmp(&b.fraction))
    }

    pub fn ground(&self, feet: Vec3, height: f32, radius: f32, distance: f32) -> Option<Vec3> {
        let delta = Vec3::NEG_Y * distance;
        self.sweep(feet, delta, height, radius)
            .filter(|hit| hit.normal.y > 0.5)
            .map(|hit| feet + delta * hit.fraction + Vec3::Y * SKIN)
    }

    pub fn sweep_sphere(&self, center: Vec3, delta: Vec3, radius: f32) -> Option<Hit> {
        let feet = center - Vec3::Y * radius;
        let height = radius * 2.0;
        // Terrain can publish around an in-flight projectile. That is an
        // immediate contact, even when its velocity points out of the solid.
        // Keep body sweeps unchanged: their tangent/step semantics are distinct.
        if !self.clear(feet, height, radius) {
            return Some(Hit {
                fraction: 0.0,
                normal: Vec3::ZERO,
            });
        }
        self.sweep(feet, delta, height, radius)
    }
}

pub(crate) fn contains(span: Span, feet: Vec3, height: f32, radius: f32) -> bool {
    let local = feet - span.coord.to_world(0.0);
    NORMALS
        .into_iter()
        .take(3)
        .all(|normal| local.dot(normal).abs() < FACE + radius - SKIN)
        && feet.y > span.bottom - height + SKIN
        && feet.y < span.top - SKIN
}

pub(crate) fn voxel_overlaps_body(
    voxel: TilePos,
    geometry: ArenaVoxelGeometry,
    feet: Vec3,
    height: f32,
    radius: f32,
) -> bool {
    contains(
        Span {
            coord: voxel.coord,
            bottom: geometry.top(voxel) - geometry.level_height,
            top: geometry.top(voxel),
        },
        feet,
        height,
        radius,
    )
}

fn sweep_span(span: Span, feet: Vec3, delta: Vec3, height: f32, radius: f32) -> Option<Hit> {
    let local = feet - span.coord.to_world(0.0);
    let mut enter: f32 = -f32::INFINITY;
    let mut exit: f32 = f32::INFINITY;
    let mut normal = Vec3::ZERO;
    for axis in NORMALS {
        let (lower, upper) = if axis.y > 0.5 {
            (span.bottom - height, span.top)
        } else {
            (-FACE - radius, FACE + radius)
        };
        let position = local.dot(axis);
        let velocity = delta.dot(axis);
        if velocity.abs() <= f32::EPSILON {
            // Tangency belongs to free space, so support floors never snag.
            if position <= lower + SKIN || position >= upper - SKIN {
                return None;
            }
            continue;
        }
        let a = (lower - position) / velocity;
        let b = (upper - position) / velocity;
        let near = a.min(b);
        if near > enter {
            enter = near;
            normal = if velocity > 0.0 { -axis } else { axis };
        }
        exit = exit.min(a.max(b));
        if enter > exit {
            return None;
        }
    }
    if exit < 0.0 || !(-SKIN..=1.0).contains(&enter) || normal.dot(delta) >= 0.0 {
        return None;
    }
    Some(Hit {
        fraction: enter.max(0.0),
        normal,
    })
}

pub(crate) fn slide(
    world: &CollisionWorld,
    feet: Vec3,
    delta: Vec3,
    height: f32,
    radius: f32,
) -> Vec3 {
    slide_with_contacts(world, feet, delta, height, radius).0
}

pub(crate) fn slide_with_contacts(
    world: &CollisionWorld,
    mut feet: Vec3,
    mut delta: Vec3,
    height: f32,
    radius: f32,
) -> (Vec3, Vec<Vec3>) {
    let mut contacts = Vec::new();
    for _ in 0..6 {
        if delta.length_squared() < SKIN * SKIN {
            break;
        }
        let Some(hit) = world.sweep(feet, delta, height, radius) else {
            return (feet + delta, contacts);
        };
        contacts.push(hit.normal);
        feet += delta * hit.fraction + hit.normal * SKIN;
        delta *= 1.0 - hit.fraction;
        delta -= hit.normal * delta.dot(hit.normal).min(0.0);
    }
    (feet, contacts)
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "these tests require the explicitly constructed collision contact"
)]
mod tests {
    use super::*;

    #[test]
    fn long_sweeps_stop_at_every_thin_hex_face() {
        let span = Span {
            coord: HexCoord::ORIGIN,
            bottom: 0.0,
            top: 4.0,
        };
        for axis in NORMALS.into_iter().take(3).flat_map(|axis| [axis, -axis]) {
            let hit = sweep_span(span, axis * 10.0 + Vec3::Y, -axis * 20.0, 0.8, 0.25)
                .expect("each prism face must block the sweep");
            assert!((hit.fraction - (10.0 - FACE - 0.25) / 20.0).abs() < 0.00001);
            assert!(hit.normal.dot(axis) > 0.99);
        }
    }

    #[test]
    fn tangent_floor_is_free_but_ceiling_blocks() {
        let floor = Span {
            coord: HexCoord::ORIGIN,
            bottom: -1.0,
            top: 0.0,
        };
        assert!(sweep_span(floor, Vec3::ZERO, Vec3::X, 0.8, 0.25).is_none());
        let ceiling = Span {
            bottom: 1.0,
            top: 2.0,
            ..floor
        };
        let hit = sweep_span(ceiling, Vec3::ZERO, Vec3::Y, 0.8, 0.25).expect("ceiling");
        assert!((hit.fraction - 0.2).abs() < 0.00001);
    }
}
