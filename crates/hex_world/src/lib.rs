//! Presentation of the world that is not the map: the sky and the camera.
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
/// Sun, ambient light, and sky colour.
pub mod sky;
/// Procedural sky material.
mod sky_material;

pub use camera::{CameraMode, PanOrbitCamera};

/// Adds every world-presentation system.
pub fn plugin(app: &mut App) {
    app.add_plugins((camera::plugin, sky::plugin, sky_material::plugin));
}
