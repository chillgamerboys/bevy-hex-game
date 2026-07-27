//! Shared terrain-traversal predicates.
//!
//! Live movement and procedural validation must answer standability and stepping
//! questions identically. Keeping the integer predicates here prevents the map from
//! growing a near-copy of gameplay's rules and accepting terrain the actual walker
//! cannot use.

use bevy_reflect::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{Headroom, Level, TilePos};

/// Exact geometric and material facts needed at one end of a traversal.
///
/// A position alone cannot describe a legal transition. Two individually standable
/// surfaces may still meet beneath a low lintel with less shared clearance than the
/// body needs while crossing between them.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraversalEndpoint {
    /// Exact surface position, including its vertical level.
    pub pos: TilePos,
    /// Whether the surface material can support a body.
    pub is_solid: bool,
    /// Consecutive clear levels directly above the surface.
    pub headroom: Headroom,
}

impl TraversalEndpoint {
    /// Creates an endpoint from the facts published for one surface.
    #[must_use]
    pub const fn new(pos: TilePos, is_solid: bool, headroom: Headroom) -> Self {
        Self {
            pos,
            is_solid,
            headroom,
        }
    }
}

/// Geometry an ordinary traversal mode can occupy and cross.
///
/// All values are quantized voxel levels. Live traversal should use
/// [`Self::admits_transition`] so endpoint standability, positional stepping, and the
/// shared lateral aperture are evaluated together.
#[derive(Reflect, Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

    /// Whether a body can move through the complete transition between two surfaces.
    ///
    /// Endpoint standability is necessary but not sufficient. On a one-level ramp, a
    /// low ceiling over the lower floor can overlap the clear volume above the higher
    /// floor by only one level. A two-level body fits while stationary at either end,
    /// but cannot pass laterally through that one-level aperture.
    ///
    /// [`Self::admits_step`] remains the position-only part of the contract for frozen
    /// generator compatibility. Live movement and new validators should use this
    /// complete predicate.
    #[must_use]
    pub fn admits_transition(self, from: TraversalEndpoint, to: TraversalEndpoint) -> bool {
        if !self.admits_surface(from.is_solid, from.headroom)
            || !self.admits_surface(to.is_solid, to.headroom)
            || !self.admits_step(from.pos, to.pos)
        {
            return false;
        }

        let higher_floor = from.pos.level.max(to.pos.level);
        let lower_clear_top = from
            .pos
            .level
            .saturating_add(from.headroom.0)
            .min(to.pos.level.saturating_add(to.headroom.0));
        lower_clear_top.saturating_sub(higher_floor) >= self.levels_tall
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

    fn endpoint(pos: TilePos, headroom: Level) -> TraversalEndpoint {
        TraversalEndpoint::new(pos, true, Headroom(headroom))
    }

    #[test]
    fn flat_two_level_aperture_is_walkable() {
        let from = TilePos::new(HexCoord::ORIGIN, 4);
        let [to, ..] = from.neighbours();

        assert!(TraversalProfile::WALKER.admits_transition(endpoint(from, 2), endpoint(to, 2)));
    }

    #[test]
    fn individually_standable_endpoints_can_lack_shared_aperture() {
        let low = TilePos::new(HexCoord::ORIGIN, 4);
        let [neighbor, ..] = low.neighbours();
        let high = neighbor.above();

        assert!(TraversalProfile::WALKER.admits_surface(true, Headroom(2)));
        assert!(!TraversalProfile::WALKER.admits_transition(endpoint(low, 2), endpoint(high, 2)));
        assert!(!TraversalProfile::WALKER.admits_transition(endpoint(high, 2), endpoint(low, 2)));
    }

    #[test]
    fn one_level_ramp_with_shared_aperture_is_walkable() {
        let low = TilePos::new(HexCoord::ORIGIN, 4);
        let [neighbor, ..] = low.neighbours();
        let high = neighbor.above();

        assert!(TraversalProfile::WALKER.admits_transition(endpoint(low, 3), endpoint(high, 2)));
        assert!(TraversalProfile::WALKER.admits_transition(endpoint(high, 2), endpoint(low, 3)));
    }

    #[test]
    fn complete_transition_rejects_bad_endpoints_and_cliffs() {
        let from = TilePos::new(HexCoord::ORIGIN, 4);
        let [neighbor, ..] = from.neighbours();

        assert!(!TraversalProfile::WALKER.admits_transition(
            TraversalEndpoint::new(from, false, Headroom(8)),
            endpoint(neighbor, 8)
        ));
        assert!(
            !TraversalProfile::WALKER.admits_transition(endpoint(from, 1), endpoint(neighbor, 8))
        );
        assert!(!TraversalProfile::WALKER
            .admits_transition(endpoint(from, 8), endpoint(neighbor.above().above(), 8)));
        assert!(!TraversalProfile::WALKER
            .admits_transition(endpoint(from, 8), endpoint(from.above(), 8)));
    }
}
