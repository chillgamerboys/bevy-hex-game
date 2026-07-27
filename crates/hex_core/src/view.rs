//! Shared presentation targets produced by gameplay systems.
//!
//! `hex_units` owns which unit is selected while `hex_world` owns the camera. Keeping
//! the projection here lets both crates share that fact without depending on each
//! other.

use bevy_ecs::prelude::*;
use bevy_reflect::prelude::*;

use crate::TilePos;

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
