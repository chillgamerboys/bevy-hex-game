//! Presentation of the 3D world: the hex grid, terrain meshes, sky, and camera.
//!
//! This crate must not depend on `hex_gameplay`. Anything the two need to share
//! belongs in `hex_core`.

use bevy::prelude::*;

pub mod camera;
pub mod grid;
pub mod sky;

pub use camera::PanOrbitCamera;

/// Adds every world-presentation system.
pub fn plugin(app: &mut App) {
    app.add_plugins((camera::plugin, grid::plugin, sky::plugin));
}
