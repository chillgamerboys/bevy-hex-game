//! Composable reasons for hiding presentation entities.
//!
//! Fog, cave roof cutaways, and canopy cutaways may all affect the same
//! entity. A reason set prevents one owner from restoring visibility while another
//! owner still requires the entity to remain hidden.

use bevy_ecs::prelude::*;
use bevy_reflect::prelude::*;

use crate::TilePos;

/// Marks one rendered tree part as eligible for character-camera canopy cutaway.
///
/// The exact root surface keeps stacked forests unambiguous without exposing the
/// generator's private feature plan. Presentation uses the entity transform for
/// smooth intersection tests and this position for the bounded horizontal search.
#[derive(Component, Reflect, Debug, Default, Clone, Copy, PartialEq, Eq)]
#[reflect(Component)]
pub struct CanopyOccluder(pub TilePos);

/// One independent owner of presentation occlusion.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PresentationOcclusionReason {
    /// Faction knowledge does not currently permit normal presentation.
    Fog,
    /// An interior cutaway hides a cave roof near an actor.
    InteriorCutaway,
    /// A character-camera cutaway hides obstructing tree canopy.
    CanopyCutaway,
}

impl PresentationOcclusionReason {
    const fn mask(self) -> u8 {
        match self {
            Self::Fog => 1 << 0,
            Self::InteriorCutaway => 1 << 1,
            Self::CanopyCutaway => 1 << 2,
        }
    }
}

/// Independent reasons an entity must remain hidden from normal presentation.
///
/// Owners add and remove only their own [`PresentationOcclusionReason`]. The
/// presentation adapter hides the entity while [`Self::is_hidden`] is true and
/// restores it only after the final reason is removed.
#[derive(Component, Reflect, Debug, Default, Clone, Copy, PartialEq, Eq)]
#[reflect(Component)]
pub struct PresentationOcclusion {
    reasons: u8,
}

impl PresentationOcclusion {
    /// Creates a reason set containing one initial reason.
    #[must_use]
    pub const fn from_reason(reason: PresentationOcclusionReason) -> Self {
        Self {
            reasons: reason.mask(),
        }
    }

    /// Adds one owner's reason.
    ///
    /// Returns whether the reason was newly added.
    pub fn insert(&mut self, reason: PresentationOcclusionReason) -> bool {
        let mask = reason.mask();
        let was_present = self.reasons & mask != 0;
        self.reasons |= mask;
        !was_present
    }

    /// Removes one owner's reason.
    ///
    /// Returns whether the reason was present.
    pub fn remove(&mut self, reason: PresentationOcclusionReason) -> bool {
        let mask = reason.mask();
        let was_present = self.reasons & mask != 0;
        self.reasons &= !mask;
        was_present
    }

    /// Whether one reason is active.
    #[must_use]
    pub const fn contains(self, reason: PresentationOcclusionReason) -> bool {
        self.reasons & reason.mask() != 0
    }

    /// Whether at least one owner requires the entity to remain hidden.
    #[must_use]
    pub const fn is_hidden(self) -> bool {
        self.reasons != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HexCoord;

    #[test]
    fn canopy_marker_preserves_the_exact_stacked_root() {
        let lower = TilePos::new(HexCoord::ORIGIN, 5);
        let upper = TilePos::new(HexCoord::ORIGIN, 15);

        assert_ne!(CanopyOccluder(lower), CanopyOccluder(upper));
        assert_eq!(CanopyOccluder(upper).0, upper);
    }

    #[test]
    fn removing_one_reason_does_not_clear_another() {
        let mut occlusion = PresentationOcclusion::from_reason(PresentationOcclusionReason::Fog);
        assert!(occlusion.insert(PresentationOcclusionReason::InteriorCutaway));
        assert!(occlusion.remove(PresentationOcclusionReason::Fog));

        assert!(!occlusion.contains(PresentationOcclusionReason::Fog));
        assert!(occlusion.contains(PresentationOcclusionReason::InteriorCutaway));
        assert!(occlusion.is_hidden());

        assert!(occlusion.remove(PresentationOcclusionReason::InteriorCutaway));
        assert!(!occlusion.is_hidden());
    }

    #[test]
    fn duplicate_reason_operations_report_no_change() {
        let mut occlusion = PresentationOcclusion::default();
        assert!(occlusion.insert(PresentationOcclusionReason::CanopyCutaway));
        assert!(!occlusion.insert(PresentationOcclusionReason::CanopyCutaway));
        assert!(occlusion.remove(PresentationOcclusionReason::CanopyCutaway));
        assert!(!occlusion.remove(PresentationOcclusionReason::CanopyCutaway));
    }
}
