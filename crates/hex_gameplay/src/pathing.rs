//! Hex-specific movement, built from the generic primitives in [`crate::animation`].
//!
//! # Paths are sequences of tiles, not coordinates
//!
//! Columns stacked at one coordinate are separate places — see the rule in
//! [`hex_core::hex`]. A path is therefore a list of the specific columns a unit
//! passes over, and a coordinate on its own is not enough to say where it went.
//!
//! An earlier version of this module keyed surface heights by [`HexCoord`], taking
//! the highest column at each. That silently collapsed every stack, so a unit
//! crossing a bridge would have snapped to the ground beneath it. The lookup is gone
//! rather than fixed: an abstraction that *can* express the wrong thing eventually
//! will.

use bevy::prelude::*;

use hex_core::config::HEX_SMALL_DIAMETER;
use hex_core::{HexCoord, HexSpan};

use crate::animation::{LinearMovement, Transformer, TransformerSeries};

/// One column on a path: where it is, and how high its surface sits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PathStep {
    /// Which hex.
    pub coord: HexCoord,
    /// Which column at that hex.
    pub span: HexSpan,
}

impl PathStep {
    /// The world-space point a unit standing here occupies.
    #[must_use]
    pub fn world_position(self) -> Vec3 {
        self.coord.to_world(self.span.top)
    }
}

/// Moves a piece along a sequence of columns, one hex-crossing at a time.
///
/// The caller decides which columns the route passes through. That is deliberate:
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
    pub fn new(steps: &[PathStep], speed: f32) -> HexPathingLine {
        let move_duration = f64::from(HEX_SMALL_DIAMETER / speed);
        let mut transformers = TransformerSeries::new();

        for (i, pair) in steps.windows(2).enumerate() {
            let (from, to) = (pair[0], pair[1]);

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
                move_duration * i as f64,
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
