//! Shared terrain-traversal predicates.
//!
//! Live movement and procedural validation must answer standability and stepping
//! questions identically. Keeping the integer predicates here prevents the map from
//! growing a near-copy of gameplay's rules and accepting terrain the actual walker
//! cannot use.

use bevy_reflect::prelude::*;

use crate::{Headroom, Level, TilePos};

/// Geometry an ordinary traversal mode can occupy and cross.
///
/// All values are quantized voxel levels. A profile admits a destination only when
/// [`Self::admits_surface`] and [`Self::admits_step`] both return true.
#[derive(Reflect, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TraversalProfile {
    /// Clear levels required directly above a surface.
    pub levels_tall: Level,
    /// Maximum upward level delta in one adjacent step.
    pub max_climb: Level,
    /// Maximum downward level delta in one adjacent step.
    pub max_drop: Level,
}

impl TraversalProfile {
    /// Canonical ordinary movement used by generated-map validation and live units.
    pub const WALKER: Self = Self {
        levels_tall: 2,
        max_climb: 1,
        max_drop: 1,
    };

    /// Whether a material surface can support this traversal profile.
    ///
    /// The material property is supplied by the caller because substance tables are
    /// owned above `hex_core`. Non-positive body heights are rejected rather than
    /// accidentally making buried or malformed surfaces standable.
    #[must_use]
    pub const fn admits_surface(self, is_solid: bool, headroom: Headroom) -> bool {
        self.levels_tall > 0 && is_solid && headroom.0 >= self.levels_tall
    }

    /// Whether two exact surfaces are connected by one ordinary step.
    ///
    /// Horizontal adjacency is part of this predicate. Surfaces in the same column
    /// are never connected, even if their levels differ by only one. Climb and drop
    /// are checked independently so a future profile may safely descend farther than
    /// it can climb.
    #[must_use]
    pub fn admits_step(self, from: TilePos, to: TilePos) -> bool {
        if from.coord.distance(to.coord) != 1 {
            return false;
        }

        let delta = from.level_step_to(to);
        if delta > 0 {
            self.max_climb >= delta
        } else if delta < 0 {
            delta >= -self.max_drop
        } else {
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HexCoord;

    #[test]
    fn walker_requires_two_clear_levels() {
        assert!(!TraversalProfile::WALKER.admits_surface(true, Headroom(1)));
        assert!(TraversalProfile::WALKER.admits_surface(true, Headroom(2)));
        assert!(!TraversalProfile::WALKER.admits_surface(false, Headroom(8)));
    }

    #[test]
    fn walker_accepts_one_level_steps_and_rejects_two() {
        let from = TilePos::new(HexCoord::ORIGIN, 4);
        let [neighbour, ..] = from.neighbours();

        assert!(TraversalProfile::WALKER.admits_step(from, neighbour.above()));
        assert!(TraversalProfile::WALKER.admits_step(from, neighbour.below()));
        assert!(!TraversalProfile::WALKER.admits_step(from, neighbour.above().above()));
        assert!(!TraversalProfile::WALKER.admits_step(from, neighbour.below().below()));
    }

    #[test]
    fn step_limits_are_asymmetric() {
        let profile = TraversalProfile {
            levels_tall: 2,
            max_climb: 1,
            max_drop: 3,
        };
        let from = TilePos::new(HexCoord::ORIGIN, 4);
        let [neighbour, ..] = from.neighbours();

        assert!(!profile.admits_step(from, neighbour.above().above()));
        assert!(profile.admits_step(from, neighbour.below().below()));
    }

    #[test]
    fn stacked_and_distant_surfaces_are_not_steps() {
        let from = TilePos::new(HexCoord::ORIGIN, 4);
        let far = TilePos::new(HexCoord::new_cubic(2, -2, 0), 4);

        assert!(!TraversalProfile::WALKER.admits_step(from, from.above()));
        assert!(!TraversalProfile::WALKER.admits_step(from, far));
    }
}
