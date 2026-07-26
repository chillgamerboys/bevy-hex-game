//! Shared failure state for cross-crate gameplay construction.
//!
//! Terrain and actors are created by different crates in one ordered setup schedule.
//! A failure in either stage must survive the state transition back to the title
//! screen so the player can see why the scenario did not start.

use bevy_ecs::prelude::*;
use bevy_reflect::prelude::*;

/// A gameplay setup error that must be shown before another scenario is attempted.
#[derive(Resource, Reflect, Debug, Clone, PartialEq, Eq)]
#[reflect(Resource)]
pub struct GameplaySetupFailure {
    /// Human-readable reason suitable for the title-screen notice and logs.
    pub reason: String,
}

impl GameplaySetupFailure {
    /// Records a setup failure with a player-visible reason.
    #[must_use]
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_preserves_its_visible_reason() {
        let failure = GameplaySetupFailure::new("missing party anchor");
        assert_eq!(failure.reason, "missing party anchor");
    }
}
