//! Pure Main Menu and Campaign-slot navigation.

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Renderer-free route within the Main Menu screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MainMenuRoute {
    /// Four-action Main Menu root.
    #[default]
    Root,
    /// Exactly three Campaign slots.
    Campaign,
    /// Creator and future tool entry points.
    Tools,
}

/// Stable identity for one of the three Campaign save slots.
///
/// Using an enum prevents invalid slot identities from being constructed or
/// deserialized. Persistence can therefore distinguish a corrupt slot record
/// from an empty slot without silently reassigning it.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CampaignSlotId {
    /// First Campaign slot.
    One,
    /// Second Campaign slot.
    Two,
    /// Third Campaign slot.
    Three,
}

impl CampaignSlotId {
    /// All Campaign slots in stable display and persistence order.
    pub const ALL: [Self; 3] = [Self::One, Self::Two, Self::Three];

    /// One-based player-facing slot number.
    #[must_use]
    pub const fn number(self) -> u8 {
        match self {
            Self::One => 1,
            Self::Two => 2,
            Self::Three => 3,
        }
    }

    /// Zero-based array index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.number() as usize - 1
    }

    /// Converts a zero-based array index into a valid slot identity.
    #[must_use]
    pub const fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::One),
            1 => Some(Self::Two),
            2 => Some(Self::Three),
            _ => None,
        }
    }
}

impl fmt::Display for CampaignSlotId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.number())
    }
}

/// Authoritative renderer-free Main Menu navigation state.
#[derive(Resource, Debug, Clone, PartialEq, Eq, Default)]
pub struct MainMenuModel {
    /// Current child route.
    pub route: MainMenuRoute,
    /// Monotonic immutable-view invalidation token.
    pub revision: u64,
}

impl MainMenuModel {
    /// Opens one Main Menu route.
    pub fn show(&mut self, route: MainMenuRoute) {
        if self.route != route {
            self.route = route;
            self.bump();
        }
    }

    /// Returns a child route to the root.
    ///
    /// Returns `false` when already at the root so the adapter can distinguish
    /// a consumed child-route Back from an application-level action.
    #[must_use]
    pub fn back(&mut self) -> bool {
        if self.route == MainMenuRoute::Root {
            return false;
        }
        self.route = MainMenuRoute::Root;
        self.bump();
        true
    }

    fn bump(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn campaign_slot_identity_is_exact_and_stably_ordered() {
        assert_eq!(CampaignSlotId::ALL.map(CampaignSlotId::number), [1, 2, 3]);
        assert_eq!(CampaignSlotId::ALL.map(CampaignSlotId::index), [0, 1, 2]);
        assert_eq!(CampaignSlotId::from_index(0), Some(CampaignSlotId::One));
        assert_eq!(CampaignSlotId::from_index(2), Some(CampaignSlotId::Three));
        assert_eq!(CampaignSlotId::from_index(3), None);
    }

    #[test]
    fn every_child_route_backs_to_root_without_a_phantom_transition() {
        for route in [MainMenuRoute::Campaign, MainMenuRoute::Tools] {
            let mut model = MainMenuModel::default();
            assert!(!model.back());
            model.show(route);
            assert_eq!(model.route, route);
            assert_eq!(model.revision, 1);
            assert!(model.back());
            assert_eq!(model.route, MainMenuRoute::Root);
            assert_eq!(model.revision, 2);
            assert!(!model.back());
            assert_eq!(model.revision, 2);
        }
    }
}
