//! Default-off composition helpers for headless integration tests.
//!
//! Callers remain responsible for installing the asset, input, state, window, and
//! transform capabilities their test needs. This module only makes the camera's
//! private sky-material prerequisite available alongside the exact production camera
//! plugin; it does not expose or duplicate camera state.

use bevy::prelude::*;

/// Installs the production camera and its private material prerequisite.
pub fn headless_camera_plugin(app: &mut App) {
    app.add_plugins((crate::sky_material::plugin, crate::camera::plugin));
}
