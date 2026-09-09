//! Swept hex-prism math adapted from the accepted M01 exploration controller.
//! This cache consumes only the arena's world-owned complete voxel projection.

use std::collections::{BTreeSet, HashMap};

#[cfg(test)]
#[path = "collision_candidates_tests.rs"]
mod candidate_tests;

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
    // These are lookup caches, never traversal authorities. Query coordinates
    // remain canonically sorted below, including equal-distance impact ties.
    columns: HashMap<HexCoord, Vec<Span>>,
    pub revision: Option<u64>,
    static_movement: HashMap<HexCoord, Vec<Span>>,
    static_sight: HashMap<HexCoord, Vec<Span>>,
    static_attack: HashMap<HexCoord, Vec<Span>>,
    barriers: Vec<crate::BarrierSnapshot>,
    pub min_y: f32,
}

#[derive(Clone, Copy)]
enum QueryKind {
    Movement,
    Sight,
    Attack,
}

impl CollisionWorld {
    #[expect(
        clippy::cast_precision_loss,
        reason = "world publishes bounded voxel levels"
    )]
    pub fn refresh(&mut self, view: &ArenaTerrainView, geometry: ArenaVoxelGeometry) {
        if self.revision == Some(view.revision) {
            return;
        }
        self.min_y =
            geometry.min_level as f32 * geometry.level_height + geometry.vertical_offset - 10.0;
        let incremental = !view.full_rebuild
            && self
                .revision
                .is_some_and(|old| old.checked_add(1) == Some(view.revision))
            && !view.columns.is_empty();
        if !incremental {
            self.columns.clear();
        }
        let changed: Vec<_> = if incremental {
            view.dirty_columns.iter().copied().collect()
        } else if view.columns.is_empty() {
            view.voxels
                .keys()
                .map(|p| p.coord)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect()
        } else {
            view.columns.keys().copied().collect()
        };
        if view.columns.is_empty() {
            for pos in view.voxels.keys() {
                let top = geometry.top(*pos);
                self.columns.entry(pos.coord).or_default().push(Span {
                    coord: pos.coord,
                    bottom: top - geometry.level_height,
                    top,
                });
            }
        } else {
            for coord in &changed {
                self.columns.remove(coord);
                if let Some(runs) = view.columns.get(coord) {
                    self.columns.insert(
                        *coord,
                        runs.iter()
                            .map(|run| Span {
                                coord: *coord,
                                bottom: geometry.top(run.bottom) - geometry.level_height,
                                top: geometry.top(TilePos::new(*coord, run.top_level)),
                            })
                            .collect(),
                    );
                }
            }
        }
        // Ordinary destruction touches only the published dirty columns.
        for coord in changed {
            let Some(spans) = self.columns.get_mut(&coord) else {
                continue;
            };
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
        // Authored object masks are immutable between complete publications.
        if !incremental {
            self.static_movement.clear();
            self.static_sight.clear();
            self.static_attack.clear();
            for volume in &view.static_spans {
                let span = Span {
                    coord: volume.bottom.coord,
                    bottom: geometry.top(volume.bottom) - geometry.level_height,
                    top: geometry.top(TilePos::new(volume.bottom.coord, volume.top_level)),
                };
                if volume.blocks_movement {
                    self.static_movement
                        .entry(span.coord)
                        .or_default()
                        .push(span);
                }
                if volume.blocks_sight {
                    self.static_sight.entry(span.coord).or_default().push(span);
                }
                if volume.blocks_projectiles {
                    self.static_attack.entry(span.coord).or_default().push(span);
                }
            }
        }
        self.revision = Some(view.revision);
    }

    fn candidates_for(
        &self,
        start: Vec3,
        end: Vec3,
        radius: f32,
        kind: QueryKind,
    ) -> impl Iterator<Item = Span> + '_ {
        let mut ring = 1;
        let mut covered = FACE;
        while radius >= covered {
            ring += 1;
            covered += FACE;
        }
        let mut coords = HexCoord::from_world(start)
            .line_between(HexCoord::from_world(end))
            .into_iter()
            .flat_map(|coord| coord.within_radius(ring))
            .collect::<Vec<_>>();
        coords.sort_unstable();
        coords.dedup();
        let extra = match kind {
            QueryKind::Movement => &self.static_movement,
            QueryKind::Sight => &self.static_sight,
            QueryKind::Attack => &self.static_attack,
        };
        coords.into_iter().flat_map(move |coord| {
            [self.columns.get(&coord), extra.get(&coord)]
                .into_iter()
                .flatten()
                .flatten()
                .copied()
        })
    }

    pub(crate) fn candidates(
        &self,
        start: Vec3,
        end: Vec3,
        radius: f32,
    ) -> impl Iterator<Item = Span> + '_ {
        self.candidates_for(start, end, radius, QueryKind::Movement)
    }

    pub fn sync_barriers(&mut self, barriers: &[crate::BarrierSnapshot]) {
        self.barriers = barriers.to_vec();
    }

    pub fn sight_clear(&self, start: Vec3, end: Vec3) -> bool {
        !self
            .candidates_for(start, end, 0.0, QueryKind::Sight)
            .any(|span| {
                contains(span, start, 0.0, 0.0)
                    || sweep_span(span, start, end - start, 0.0, 0.0).is_some()
            })
    }

    pub fn attack_sweep(
        &self,
        start: Vec3,
        delta: Vec3,
        radius: f32,
    ) -> Option<(Hit, Option<u64>)> {
        let feet = start - Vec3::Y * radius;
        let mut hit = self
            .candidates_for(start, start + delta, radius, QueryKind::Attack)
            .filter_map(|span| {
                if contains(span, feet, radius * 2.0, radius) {
                    Some(Hit {
                        fraction: 0.0,
                        normal: Vec3::ZERO,
                    })
                } else {
                    sweep_span(span, feet, delta, radius * 2.0, radius)
                }
            })
            .min_by(|a, b| a.fraction.total_cmp(&b.fraction))
            .map(|h| (h, None));
        for barrier in self
            .barriers
            .iter()
            .filter(|b| b.hp > 0.0 && b.remaining > 0.0)
        {
            if let Some(contact) = crate::shapes::sweep_barrier(barrier, start, delta, radius) {
                if hit.is_none_or(|(old, _)| contact.fraction < old.fraction) {
                    hit = Some((contact, Some(barrier.id)));
                }
            }
        }
        hit
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
