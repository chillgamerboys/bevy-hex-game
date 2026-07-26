//! Shared terrain-traversal predicates.
//!
//! Live movement and procedural validation must answer standability and stepping
//! questions identically. Keeping the integer predicates here prevents the map from
//! growing a near-copy of gameplay's rules and accepting terrain the actual walker
//! cannot use.

use bevy_ecs::prelude::*;
use bevy_platform::collections::HashMap;
use bevy_reflect::prelude::*;

use crate::{Headroom, Level, TilePos};

/// Stable id of a traversal ruleset.
///
/// Numeric ids are compact in components and stable across renames. Save data should
/// treat these values as a compatibility contract once more profiles are introduced.
#[derive(Reflect, Debug, Default, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TraversalProfileId(pub u16);

impl TraversalProfileId {
    /// The ordinary ground walker used by the player and current enemies.
    pub const WALKER: Self = Self(0);
}

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

/// Traversal rules published for the active gameplay setup.
///
/// Missing ids return [`None`]; generators and gameplay should fail their own setup
/// visibly rather than guessing at movement rules.
#[derive(Resource, Debug, Default, Clone)]
pub struct TraversalProfiles {
    by_id: HashMap<TraversalProfileId, TraversalProfile>,
}

impl TraversalProfiles {
    /// Creates an empty profile collection.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds or replaces a profile, returning the previous value if it existed.
    pub fn insert(
        &mut self,
        id: TraversalProfileId,
        profile: TraversalProfile,
    ) -> Option<TraversalProfile> {
        self.by_id.insert(id, profile)
    }

    /// Finds a profile without guessing when an id was not published.
    #[must_use]
    pub fn get(&self, id: TraversalProfileId) -> Option<&TraversalProfile> {
        self.by_id.get(&id)
    }

    /// The canonical ordinary walker, if it has been published.
    #[must_use]
    pub fn walker(&self) -> Option<&TraversalProfile> {
        self.get(TraversalProfileId::WALKER)
    }

    /// Every registered id and profile, in unspecified order.
    pub fn iter(&self) -> impl Iterator<Item = (TraversalProfileId, &TraversalProfile)> {
        self.by_id.iter().map(|(id, profile)| (*id, profile))
    }
}

impl FromIterator<(TraversalProfileId, TraversalProfile)> for TraversalProfiles {
    fn from_iter<T: IntoIterator<Item = (TraversalProfileId, TraversalProfile)>>(
        profiles: T,
    ) -> Self {
        Self {
            by_id: profiles.into_iter().collect(),
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
