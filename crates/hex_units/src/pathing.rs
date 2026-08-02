//! Hex-specific movement, built from the generic primitives in [`hex_anim`].
//!
//! This module is the boundary: `hex_anim` moves a transform from one point to
//! another and knows nothing else, and everything about a hex being a fixed width
//! apart lives here.
//!
//! # Paths are sequences of surfaces, not coordinates
//!
//! Surfaces stacked in one coordinate's column are separate places — see the rule in
//! [`hex_core::hex`]. A path is therefore a list of the specific surfaces a unit
//! passes over, and a coordinate on its own is not enough to say where it went.
//!
//! An earlier version of this module keyed surface heights by [`HexCoord`](hex_core::HexCoord), taking
//! the highest surface at each. That silently collapsed every stack, so a unit
//! crossing a bridge would have snapped to the ground beneath it. The lookup is gone
//! rather than fixed: an abstraction that *can* express the wrong thing eventually
//! will.

use bevy::prelude::*;

use hex_anim::{LinearMovement, Transformer, TransformerSeries};

use crate::movement::Standing;
#[cfg(test)]
use hex_core::config::HEX_SMALL_DIAMETER;

/// Horizontal progress over the lower surface before an elevation step reaches the
/// higher surface.
///
/// Keeping the root high across the middle third clears the current piece before its
/// footprint crosses the voxel edge. The bend remains presentation-only: domain
/// movement still advances between the two real [`Standing`] endpoints.
const STEP_OVER_BLEND_FRACTION: f32 = 1.0 / 3.0;

/// Moves a piece along a sequence of surfaces, one hex-crossing at a time.
///
/// The caller decides which surfaces the route passes through. That is deliberate:
/// choosing a route is movement design — it has to respect step heights, stairs, and
/// whatever abilities bypass those — whereas this type only animates a route that has
/// already been chosen.
pub struct HexPathingLine {
    transformers: TransformerSeries,
}

impl HexPathingLine {
    /// Builds an animation following `steps` in order, at `speed` world units per
    /// second.
    ///
    /// Fewer than two steps produces an empty animation that finishes immediately,
    /// which is the correct behaviour for "move to where you already are".
    pub fn new(steps: &[Standing], speed: f32) -> HexPathingLine {
        let mut transformers = TransformerSeries::new();
        let mut start_time = 0.0;

        for pair in steps.windows(2) {
            let [from, to] = pair else {
                unreachable!("windows(2) always yields pairs")
            };

            for segment in leg_waypoints(*from, *to).windows(2) {
                let [start, end] = segment else {
                    unreachable!("windows(2) always yields pairs")
                };
                transformers.push(LinearMovement::new(*start, *end, speed, start_time));
                start_time += segment_duration(*start, *end, speed);
            }
        }

        Self { transformers }
    }
}

/// Rendered world-space points for one logical surface-to-surface leg.
///
/// Flat legs stay straight. An uphill leg reaches the high surface over the lower
/// third, then stays high across the voxel edge; downhill is the exact reverse. The
/// world-space endpoint heights come from the published spans rather than a map
/// setting or reconstructed voxel scale.
fn leg_waypoints(from: Standing, to: Standing) -> Vec<Vec3> {
    let start = from.world_position();
    let end = to.world_position();
    let mut waypoints = Vec::with_capacity(3);
    waypoints.push(start);

    if from.pos.level != to.pos.level {
        let blend = if from.pos.level < to.pos.level {
            STEP_OVER_BLEND_FRACTION
        } else {
            1.0 - STEP_OVER_BLEND_FRACTION
        };
        let mut bend = start.lerp(end, blend);
        bend.y = start.y.max(end.y);
        waypoints.push(bend);
    }

    waypoints.push(end);
    waypoints
}

fn segment_duration(start: Vec3, end: Vec3, speed: f32) -> f64 {
    f64::from(start.distance(end) / speed)
}

/// Seconds needed to travel one route leg at `speed`.
///
/// Shared with logical movement reconciliation so the rendered position and
/// [`StandsOn`](crate::StandsOn) cross a waypoint on the same frame.
pub(crate) fn leg_duration(from: Standing, to: Standing, speed: f32) -> f64 {
    leg_waypoints(from, to)
        .windows(2)
        .map(|segment| {
            let [start, end] = segment else {
                unreachable!("windows(2) always yields pairs")
            };
            segment_duration(*start, *end, speed)
        })
        .sum()
}

/// The index of the last whole route step reached after `elapsed` active seconds.
pub(crate) fn reached_step_index(steps: &[Standing], speed: f32, elapsed: f64) -> Option<usize> {
    steps.first()?;
    let mut reached = 0;
    let mut end_time = 0.0;

    for pair in steps.windows(2) {
        let [from, to] = pair else {
            unreachable!("windows(2) always yields pairs")
        };
        end_time += leg_duration(*from, *to, speed);
        if elapsed < end_time {
            break;
        }
        reached += 1;
    }

    Some(reached)
}

impl Transformer for HexPathingLine {
    fn update(&self, transform: &mut Transform, time: f64) {
        self.transformers.update(transform, time);
    }

    fn is_finished(&self, time: f64) -> bool {
        self.transformers.is_finished(time)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::{HexCoord, HexSpan, TilePos};

    fn standing(coord: HexCoord, level: i32, top: f32) -> Standing {
        Standing {
            pos: TilePos::new(coord, level),
            span: HexSpan::new(top - 1.0, top),
        }
    }

    fn assert_close(actual: Vec3, expected: Vec3) {
        assert!(
            actual.distance(expected) < 1e-5,
            "expected {expected:?}, got {actual:?}"
        );
    }

    fn assert_time_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-6,
            "expected {expected}, got {actual}"
        );
    }

    fn position_at(line: &HexPathingLine, time: f64) -> Vec3 {
        let mut transform = Transform::default();
        line.update(&mut transform, time);
        transform.translation
    }

    /// Equal-height movement retains the original straight, constant-speed crossing.
    #[test]
    fn flat_legs_remain_straight() {
        let from = standing(HexCoord::ORIGIN, 4, 2.25);
        let to = standing(HexCoord::new_cubic(1, -1, 0), 4, 2.25);
        let speed = 2.5;
        let duration = f64::from(HEX_SMALL_DIAMETER / speed);
        let line = HexPathingLine::new(&[from, to], speed);

        assert_time_close(leg_duration(from, to, speed), duration);
        assert_close(position_at(&line, 0.0), from.world_position());
        assert_close(
            position_at(&line, duration * 0.5),
            from.world_position().lerp(to.world_position(), 0.5),
        );
        assert_close(position_at(&line, duration), to.world_position());
    }

    /// Uphill movement reaches the destination height before the piece crosses the
    /// shared edge, using the published world-space span rather than a fixed level
    /// height.
    #[test]
    fn uphill_legs_clear_the_voxel_edge() {
        let from = standing(HexCoord::ORIGIN, 7, 1.35);
        let to = standing(HexCoord::new_cubic(1, -1, 0), 8, 2.15);
        let speed = 3.0;
        let start = from.world_position();
        let end = to.world_position();
        let line = HexPathingLine::new(&[from, to], speed);

        let mut bend = start.lerp(end, STEP_OVER_BLEND_FRACTION);
        bend.y = end.y;
        let mut edge = start.lerp(end, 0.5);
        edge.y = end.y;
        let mut high_side = start.lerp(end, 0.75);
        high_side.y = end.y;

        let bend_time = segment_duration(start, bend, speed);
        let edge_time = bend_time + segment_duration(bend, edge, speed);
        let high_side_time = bend_time + segment_duration(bend, high_side, speed);

        assert_close(position_at(&line, bend_time), bend);
        assert_close(position_at(&line, edge_time), edge);
        assert_close(position_at(&line, high_side_time), high_side);
        assert_close(position_at(&line, leg_duration(from, to, speed)), end);
    }

    /// Downhill is the exact time-reverse of uphill, so it stays on the high surface
    /// until the piece has crossed the edge before descending.
    #[test]
    fn downhill_legs_reverse_the_uphill_step_over() {
        let low = standing(HexCoord::ORIGIN, 7, 1.35);
        let high = standing(HexCoord::new_cubic(1, -1, 0), 8, 2.15);
        let speed = 3.0;
        let uphill = HexPathingLine::new(&[low, high], speed);
        let downhill = HexPathingLine::new(&[high, low], speed);
        let uphill_duration = leg_duration(low, high, speed);
        let downhill_duration = leg_duration(high, low, speed);

        assert_time_close(uphill_duration, downhill_duration);
        for elapsed in [
            0.0,
            uphill_duration * 0.2,
            uphill_duration * 0.5,
            uphill_duration * 0.8,
            uphill_duration,
        ] {
            assert_close(
                position_at(&uphill, elapsed),
                position_at(&downhill, downhill_duration - elapsed),
            );
        }
    }

    /// A step-over leg uses its bent world-space length, and the following flat leg
    /// waits for both presentation segments to finish.
    #[test]
    fn step_over_legs_share_their_duration_with_logical_movement() {
        let a = standing(HexCoord::ORIGIN, 0, 1.0);
        let b = standing(HexCoord::new_cubic(1, -1, 0), 1, 2.0);
        let c = standing(HexCoord::new_cubic(2, -2, 0), 1, 2.0);
        let steps = [a, b, c];
        let speed = 1.0;
        let first_end = leg_duration(a, b, speed);
        let finish = first_end + leg_duration(b, c, speed);
        let line = HexPathingLine::new(&steps, speed);

        assert!(
            first_end > f64::from(a.world_position().distance(b.world_position()) / speed),
            "the step-over should be longer than the old straight chord"
        );
        assert_eq!(
            reached_step_index(&steps, speed, first_end - 1e-6),
            Some(0),
            "the logical route advanced before the rendered step-over landed"
        );

        let mut transform = Transform::default();
        line.update(&mut transform, first_end);
        assert_close(transform.translation, b.world_position());
        assert_eq!(
            reached_step_index(&steps, speed, first_end),
            Some(1),
            "the logical schedule should reach the same waypoint"
        );

        assert!(
            !line.is_finished(finish - 1e-6),
            "the flat second leg started before the climb had finished"
        );
        assert_eq!(
            reached_step_index(&steps, speed, finish - 1e-6),
            Some(1),
            "the logical route finished before the rendered flat leg"
        );
        assert!(line.is_finished(finish));
        line.update(&mut transform, finish);
        assert_close(transform.translation, c.world_position());
        assert_eq!(
            reached_step_index(&steps, speed, finish),
            Some(2),
            "the logical route should finish with its animation"
        );
    }
}
