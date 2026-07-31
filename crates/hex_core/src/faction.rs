//! Stable sides used by gameplay and world-owned observation.
//!
//! A faction is shared vocabulary rather than unit implementation: combat authority,
//! unit ECS projection, and perception all need the same identity without depending
//! on one another.

use bevy_ecs::prelude::*;
use bevy_reflect::prelude::Reflect;
use serde::{Deserialize, Serialize};

/// Which side a unit belongs to.
#[derive(
    Component,
    Reflect,
    Serialize,
    Deserialize,
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
)]
#[reflect(Component)]
pub enum Faction {
    /// The party the player controls.
    Player,
    /// Everything that wants the party dead.
    Hostile,
}

impl Faction {
    /// Stable lower-case label used by authored encounter validation.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Player => "player",
            Self::Hostile => "hostile",
        }
    }

    /// Whether these two sides fight each other.
    ///
    /// Deliberately not `self != other`: a future neutral faction should be hostile
    /// to nobody unless the authored relationship table says otherwise.
    #[must_use]
    pub fn is_hostile_to(self, other: Self) -> bool {
        matches!(
            (self, other),
            (Self::Player, Self::Hostile) | (Self::Hostile, Self::Player)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_player_and_hostile_are_enemies() {
        assert!(Faction::Player.is_hostile_to(Faction::Hostile));
        assert!(Faction::Hostile.is_hostile_to(Faction::Player));
        assert!(!Faction::Player.is_hostile_to(Faction::Player));
        assert!(!Faction::Hostile.is_hostile_to(Faction::Hostile));
    }
}
