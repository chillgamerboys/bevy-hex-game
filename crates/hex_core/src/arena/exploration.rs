//! Map capabilities and explicit local residency for continuous exploration.
use super::ArenaMap;
use crate::HexCoord;
use bevy_ecs::prelude::Resource;
use bevy_math::Vec3;
use std::collections::BTreeSet;

/// Presentation and session capabilities, independent of package identity strings.
#[derive(Clone, Copy, Debug)]
pub struct ArenaMapCapabilities {
    /// Uses the taller expedition player and its starting spell tuning.
    pub expedition_player: bool,
    /// Empty encounter rosters remain playable exploration.
    pub exploration: bool,
    /// Publishes a finite V4 package through bounded residency.
    pub streamed: bool,
    /// Enables the shared sky and translucent liquid environment.
    pub natural_environment: bool,
}
impl ArenaMap {
    /// Behavioral capabilities owned by the selected map kind.
    #[must_use]
    pub const fn capabilities(self) -> ArenaMapCapabilities {
        let forest = matches!(self, Self::ForestMassif);
        let islands = matches!(self, Self::NorthernArchipelago);
        ArenaMapCapabilities {
            expedition_player: forest || islands,
            exploration: islands,
            streamed: islands,
            natural_environment: forest || islands,
        }
    }
}

/// Exact availability of a column; missing streamed data is never air.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArenaAvailability {
    /// Current exact occupancy is admitted, including empty columns.
    Ready,
    /// Inside the package but not admitted yet.
    Unloaded,
    /// Outside the finite world footprint.
    OutsideWorld,
}

/// World-owned compact admission facts for 16 by 16 axial V4 chunks.
#[derive(Clone, Debug, Default)]
pub struct ArenaResidency {
    /// Complete package chunk catalogue, including not-yet-loaded partitions.
    pub catalogue: BTreeSet<(i32, i32)>,
    /// Partitions whose exact collision projection is installed at this revision.
    pub ready: BTreeSet<(i32, i32)>,
}
impl ArenaResidency {
    /// Availability at an absolute axial coordinate.
    #[must_use]
    pub fn at(&self, coord: HexCoord) -> ArenaAvailability {
        let chunk = (coord.x().div_euclid(16), coord.y().div_euclid(16));
        if self.ready.contains(&chunk) {
            ArenaAvailability::Ready
        } else if self.catalogue.contains(&chunk) {
            ArenaAvailability::Unloaded
        } else {
            ArenaAvailability::OutsideWorld
        }
    }
}

/// Actor-owned interest consumed by the world loader; carries no generation policy.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct ArenaStreamInterest {
    /// Current controlled body's absolute feet position.
    pub position: Vec3,
    /// Requested physical movement used for bounded ahead-of-travel prefetch.
    pub velocity: Vec3,
}
