//! Hex-specific movement, built from the generic primitives in [`crate::animation`].
use bevy::prelude::*;

use hex_core::config::HEX_SMALL_DIAMETER;
use hex_core::{HeightMap, HexCoord};

use crate::animation::{LinearMovement, Transformer, TransformerSeries};

/// Moves piece from its starting coord to another coord,
/// moving to intermediate tiles along a straight line bewteen the two
pub struct HexPathingLine {
    transformers: TransformerSeries
}

impl HexPathingLine {
    pub fn new(start: HexCoord, end: HexCoord, speed: f32, map: &HeightMap) -> HexPathingLine {
        let move_duration = (HEX_SMALL_DIAMETER / speed) as f64;
        let line = start.line_between(end);
        let mut transformers = TransformerSeries::new();

        for (i, this_coord) in line.iter().enumerate() {
            let this_pos = this_coord.to_world(Some(map));

            if let Some(next_coord) = line.get(i + 1) {
                let next_pos = next_coord.to_world(Some(map));

                // TODO:
                // Handle height differences here so we don't clip
                // Either do a horizontal and vertical movement seperately
                // or add some kind of bezier curve jump to get over height difference

                // Segment i starts `i` hex-crossings in, measured from when the whole
                // path starts rather than from an absolute timestamp.
                let transformer = LinearMovement::new(this_pos, next_pos, speed, move_duration * i as f64);
                transformers.push(transformer)
            }
        }
        Self { transformers }
    }
}

impl Transformer for HexPathingLine {
    fn update(&self, transform: &mut Transform, time: f64) {
        self.transformers.update(transform, time)
    }

    fn is_finished(&self, time: f64) -> bool {
        self.transformers.is_finished(time)
    }
}
