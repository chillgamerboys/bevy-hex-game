//! Presentation of the world that is not the map: the sky, camera, and local cutaways.
//!
//! The hex grid used to live here. It moved to `hex_map`, which owns terrain
//! generation, tile spawning, and map settings together — so the map can be worked
//! on in one place without reaching across crates.
//!
//! This crate must not depend on `hex_units` or `hex_map`. Anything shared
//! belongs in `hex_core`.

use bevy::prelude::*;

/// Pan/orbit camera and the sky dome.
pub mod camera;
/// Local opaque-roof cutaways for generated interiors.
mod cutaway;
/// Sun, ambient light, and sky colour.
pub mod sky;
/// Procedural sky material.
mod sky_material;

pub use camera::{CameraMode, PanOrbitCamera};
pub use sky::TimeOfDay;

/// Same-frame ordering for resolving designer inputs and applying one coherent frame.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum LightingSystems {
    /// Turn settings and the session clock into a renderer-safe snapshot.
    Resolve,
    /// Apply that snapshot to lights, views, and the sky material.
    Apply,
}

/// Adds every world-presentation system.
pub fn plugin(app: &mut App) {
    app.add_plugins((
        camera::plugin,
        cutaway::plugin,
        sky::plugin,
        sky_material::plugin,
    ));
}
