//! Hex-specific movement, built from the generic primitives in [`crate::animation`].
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

use crate::animation::{LinearMovement, Transformer, TransformerSeries};
use crate::movement::Standing;
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
        let move_duration = f64::from(HEX_SMALL_DIAMETER / speed);
        let mut transformers = TransformerSeries::new();

        for (i, pair) in steps.windows(2).enumerate() {
            let [from, to] = pair else {
                unreachable!("windows(2) always yields pairs")
            };

            // TODO: height differences are traversed as a straight diagonal, so a
            // piece clips through the corner of a tall column. Splitting into a
            // horizontal and a vertical leg, or arcing over, would fix it — but
            // which is right depends on what a "step" means in the movement rules.

            // Segment i starts `i` hex-crossings in, measured from when the whole
            // path starts rather than from an absolute timestamp.
            transformers.push(LinearMovement::new(
                from.world_position(),
                to.world_position(),
                speed,
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "a path index, bounded by the grid diameter"
                )]
                {
                    move_duration * i as f64
                },
            ));
        }

        Self { transformers }
    }
}

impl Transformer for HexPathingLine {
    fn update(&self, transform: &mut Transform, time: f64) {
        self.transformers.update(transform, time);
    }

    fn is_finished(&self, time: f64) -> bool {
        self.transformers.is_finished(time)
    }
}
