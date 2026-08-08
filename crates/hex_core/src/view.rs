//! Shared presentation targets produced by gameplay systems.
//!
//! `hex_units` owns which unit is selected while `hex_world` owns the camera. Keeping
//! the projection here lets both crates share that fact without depending on each
//! other.

use bevy_ecs::prelude::*;
use bevy_reflect::prelude::*;

use crate::{TilePos, UnitId};

/// Marks the entity a character-focused camera should follow.
///
/// This is a projection of the current unit selection, not a second source of truth.
/// Unit systems keep it synchronized with their selection marker; presentation systems
/// only consume it. The exact surface disambiguates stacked places at one horizontal
/// coordinate, while the entity's transform remains the source of smooth visual motion.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq, Eq)]
#[reflect(Component)]
pub struct CameraFocusTarget {
    /// The exact surface the selected unit currently occupies.
    pub surface: TilePos,
}

impl CameraFocusTarget {
    /// Projects a selected unit standing on `surface`.
    #[must_use]
    pub const fn new(surface: TilePos) -> Self {
        Self { surface }
    }
}

/// One disclosure-authorized unit the presentation camera may inspect.
///
/// Unlike [`CameraFocusTarget`], this is not a projection of gameplay selection and
/// carries no command authority. The shared game adapter may place it on exactly one
/// currently disclosed unit, keep its exact surface current, and remove it when that
/// disclosure expires. Camera presentation may consume the projection but must never
/// turn it back into selection, turn, caster, or ownership state.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq, Eq)]
#[reflect(Component)]
pub struct InspectionCameraSubject {
    /// Stable identity authorized for presentation inspection.
    pub unit: UnitId,
    /// Exact surface used by Character-camera terrain collision.
    pub surface: TilePos,
}

impl InspectionCameraSubject {
    /// Projects one authorized unit standing on `surface`.
    #[must_use]
    pub const fn new(unit: UnitId, surface: TilePos) -> Self {
        Self { unit, surface }
    }
}

/// Requests one Map-camera centering operation for an authorized inspection subject.
///
/// This is deliberately a message rather than retained camera state: moving the
/// subject afterward does not drag the free Map camera. Both character camera modes
/// independently follow the current [`InspectionCameraSubject`].
#[derive(Message, Reflect, Debug, Clone, Copy, PartialEq, Eq)]
pub struct CenterInspectionCamera {
    /// Stable identity of the subject to center.
    pub unit: UnitId,
}

impl CenterInspectionCamera {
    /// Requests one presentation-only center operation for `unit`.
    #[must_use]
    pub const fn new(unit: UnitId) -> Self {
        Self { unit }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HexCoord;

    #[test]
    fn inspection_contract_keeps_stable_identity_and_exact_surface() {
        let unit = UnitId(41);
        let lower = TilePos::new(HexCoord::ORIGIN, 3);
        let upper = TilePos::new(HexCoord::ORIGIN, 11);

        assert_ne!(
            InspectionCameraSubject::new(unit, lower),
            InspectionCameraSubject::new(unit, upper),
            "stacked inspection subjects must retain their exact surface"
        );
        assert_eq!(CenterInspectionCamera::new(unit).unit, unit);
    }
}
