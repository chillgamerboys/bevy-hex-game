//! Shared presentation targets produced by gameplay systems.
//!
//! `hex_units` owns which unit is selected while `hex_world` owns the camera. Keeping
//! the marker here lets both crates share that fact without depending on each other.

use bevy_ecs::prelude::*;
use bevy_reflect::prelude::*;

/// Marks the entity a character-focused camera should follow.
///
/// This is a projection of the current unit selection, not a second source of truth.
/// Unit systems keep it synchronized with their selection marker; presentation systems
/// only consume it.
#[derive(Component, Reflect, Debug, Default, Clone, Copy, PartialEq, Eq)]
#[reflect(Component)]
pub struct CameraFocusTarget;
