//! Presentation of the world that is not the map: sky, collision-aware camera, and review cutaways.
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
/// Adaptive tree fading and explicit review-only interior cutaways.
mod cutaway;
/// Sun, ambient light, and sky colour.
pub mod sky;
/// Procedural sky material.
mod sky_material;
#[cfg(feature = "test-support")]
/// Default-off helpers that compose production presentation plugins headlessly.
pub mod test_support;

pub use camera::{CameraMode, CameraSystems, PanOrbitCamera};
pub use sky::{clear_environment_map_cache, TimeOfDay};
#[cfg(feature = "dev-time-preview")]
pub use sky::{reset_presentation_time_override, PresentationTimeOverride};
#[cfg(feature = "map-review")]
pub use sky_material::SkyRuntimeAssetEvidenceV1;

/// Enables a full-interior cutaway for one deterministic review capture.
///
/// Ordinary gameplay does not call this function and retains every opaque roof.
pub fn install_full_cutaway_review_override(app: &mut App) {
    cutaway::install_full_review_override(app);
}

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
