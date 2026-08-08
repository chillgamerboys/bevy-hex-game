//! Composable reasons for hiding presentation entities.
//!
//! Fog, explicit review cutaways, Sandbox deployment, and near-character camera
//! occlusion may affect the same entity. A reason set prevents one owner from restoring
//! visibility while another still requires it hidden.

use bevy_ecs::prelude::*;
use bevy_reflect::prelude::*;

use crate::{TilePos, UnitId};

/// Cross-crate ordering for camera-driven world presentation.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PresentationSystems {
    /// Resolve camera obstruction into renderer-neutral presentation requests.
    ResolveCameraOcclusion,
    /// Apply renderer-owned material changes after requests settle.
    ApplyMaterials,
    /// Apply composable visibility reasons after material presentation.
    ApplyVisibility,
}

/// Presentation-only request for a world-space reticle on one authorized unit.
///
/// The game adapter owns disclosure and may insert this only after deciding the unit
/// is currently presentable to the local player. Renderers consume the request but it
/// grants no target legality, observation, selection, or command authority.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq, Eq)]
#[reflect(Component)]
pub struct TargetReticleRequest {
    /// Stable identity of the entity carrying the request.
    pub unit: UnitId,
}

impl TargetReticleRequest {
    /// Requests a presentation reticle for `unit`.
    #[must_use]
    pub const fn new(unit: UnitId) -> Self {
        Self { unit }
    }
}

/// Phase-level suppression for transient unit markers in the rendered world.
///
/// Deployment and terminal outcomes use this to remove acting rings and target
/// reticles without deleting the underlying presentation requests. The resource is
/// presentation state only and must never be consulted for gameplay legality.
#[derive(Resource, Reflect, Debug, Default, Clone, Copy, PartialEq, Eq)]
#[reflect(Resource)]
pub struct WorldMarkerSuppression {
    suppressed: bool,
}

impl WorldMarkerSuppression {
    /// Whether unit rings and target reticles should currently be absent.
    #[must_use]
    pub const fn is_suppressed(self) -> bool {
        self.suppressed
    }

    /// Sets phase-level presentation suppression.
    pub fn set(&mut self, suppressed: bool) {
        self.suppressed = suppressed;
    }
}

/// Marks one rendered chunk as belonging to an exact authored tree root.
///
/// Every trunk, branch, and canopy chunk carries this marker. The stack-safe root
/// lets presentation group a whole tree without importing the generator's private
/// feature identity.
#[derive(Component, Reflect, Debug, Default, Clone, Copy, PartialEq, Eq)]
#[reflect(Component)]
pub struct TreeOccluder(pub TilePos);

/// Renderer-neutral opacity requested for one tree render chunk.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
#[reflect(Component)]
pub struct TreeFadeAmount(f32);

impl TreeFadeAmount {
    /// Fully opaque tree presentation.
    pub const OPAQUE: Self = Self(1.0);

    /// Creates a finite opacity in the inclusive range `0.0..=1.0`.
    #[must_use]
    pub fn new(amount: f32) -> Option<Self> {
        (amount.is_finite() && (0.0..=1.0).contains(&amount)).then_some(Self(amount))
    }

    /// Current opacity multiplier.
    #[must_use]
    pub const fn amount(self) -> f32 {
        self.0
    }
}

impl Default for TreeFadeAmount {
    fn default() -> Self {
        Self::OPAQUE
    }
}

/// Marks one rendered chunk as part of the exact authored canopy mask.
///
/// The exact root surface keeps stacked forests unambiguous without exposing the
/// generator's private feature plan. Whole-tree camera fading uses [`TreeOccluder`]
/// instead, so trunk, branch, and canopy presentation cannot diverge.
#[derive(Component, Reflect, Debug, Default, Clone, Copy, PartialEq, Eq)]
#[reflect(Component)]
pub struct CanopyOccluder(pub TilePos);

/// One independent owner of presentation occlusion.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PresentationOcclusionReason {
    /// Faction knowledge does not currently permit normal presentation.
    Fog,
    /// Explicit capture tooling hides the selected interior's roof.
    InteriorCutaway,
    /// The Character camera is close enough that the selected unit would obscure it.
    CharacterCameraProximity,
    /// Sandbox setup keeps staged actors hidden until exact deployment is committed.
    SandboxDeployment,
}

impl PresentationOcclusionReason {
    const fn mask(self) -> u8 {
        match self {
            Self::Fog => 1 << 0,
            Self::InteriorCutaway => 1 << 1,
            Self::CharacterCameraProximity => 1 << 2,
            Self::SandboxDeployment => 1 << 3,
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
    fn tree_marker_and_fade_amount_preserve_valid_presentation_facts() {
        let root = TilePos::new(HexCoord::from_axial(2, -1), 12);
        assert_eq!(TreeOccluder(root).0, root);
        assert!((TreeFadeAmount::OPAQUE.amount() - 1.0).abs() < f32::EPSILON);
        let faded = TreeFadeAmount::new(0.2).expect("0.2 is a valid opacity");
        assert!((faded.amount() - 0.2).abs() < f32::EPSILON);
        assert!(TreeFadeAmount::new(-0.1).is_none());
        assert!(TreeFadeAmount::new(f32::NAN).is_none());
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
        assert!(occlusion.insert(PresentationOcclusionReason::InteriorCutaway));
        assert!(!occlusion.insert(PresentationOcclusionReason::InteriorCutaway));
        assert!(occlusion.remove(PresentationOcclusionReason::InteriorCutaway));
        assert!(!occlusion.remove(PresentationOcclusionReason::InteriorCutaway));
    }

    #[test]
    fn camera_proximity_composes_with_other_visibility_owners() {
        let mut occlusion = PresentationOcclusion::from_reason(PresentationOcclusionReason::Fog);
        assert!(occlusion.insert(PresentationOcclusionReason::CharacterCameraProximity));
        assert!(occlusion.remove(PresentationOcclusionReason::CharacterCameraProximity));
        assert!(occlusion.contains(PresentationOcclusionReason::Fog));
        assert!(occlusion.is_hidden());
    }

    #[test]
    fn world_marker_requests_and_suppression_have_no_gameplay_authority() {
        let request = TargetReticleRequest::new(crate::UnitId(9));
        let mut suppression = WorldMarkerSuppression::default();

        assert_eq!(request.unit, crate::UnitId(9));
        assert!(!suppression.is_suppressed());
        suppression.set(true);
        assert!(suppression.is_suppressed());
        suppression.set(false);
        assert!(!suppression.is_suppressed());
    }
}
