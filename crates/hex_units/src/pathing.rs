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

            // TODO: height differences are traversed as a straight diagonal, so a
            // piece clips through the corner of a tall column. Splitting into a
            // horizontal and a vertical leg, or arcing over, would fix it — but
            // which is right depends on what a "step" means in the movement rules.

            // A climb is longer in 3D than a level crossing. Accumulating each leg's
            // actual duration keeps the next one from starting in the past and
            // jumping partway through as soon as the climb finishes.
            transformers.push(LinearMovement::new(
                from.world_position(),
                to.world_position(),
                speed,
                start_time,
            ));
            start_time += leg_duration(*from, *to, speed);
        }

        Self { transformers }
    }
}

/// Seconds needed to travel one route leg at `speed`.
///
/// Shared with logical movement reconciliation so the rendered position and
/// [`StandsOn`](crate::StandsOn) cross a waypoint on the same frame.
pub(crate) fn leg_duration(from: Standing, to: Standing, speed: f32) -> f64 {
    f64::from(from.world_position().distance(to.world_position()) / speed)
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

    /// A sloped leg takes longer than a flat one and the following leg must wait for it.
    #[test]
    fn elevated_legs_are_scheduled_by_their_actual_3d_length() {
        let a = standing(HexCoord::ORIGIN, 0, 1.0);
        let b = standing(HexCoord::new_cubic(1, -1, 0), 1, 2.0);
        let c = standing(HexCoord::new_cubic(2, -2, 0), 1, 2.0);
        let steps = [a, b, c];
        let speed = 1.0;
        let first_end = leg_duration(a, b, speed);
        let finish = first_end + leg_duration(b, c, speed);
        let line = HexPathingLine::new(&steps, speed);

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
        assert!(line.is_finished(finish));
        line.update(&mut transform, finish);
        assert_close(transform.translation, c.world_position());
    }

    /// Keep the horizontal geometry constant named here as part of the contract: an
    /// elevated leg must be strictly longer than one flat hex crossing.
    #[test]
    fn a_climb_is_longer_than_the_flat_hex_diameter() {
        let a = standing(HexCoord::ORIGIN, 0, 1.0);
        let b = standing(HexCoord::new_cubic(1, -1, 0), 1, 2.0);
        assert!(leg_duration(a, b, 1.0) > f64::from(HEX_SMALL_DIAMETER));
    }
}
