//! Designer-facing settings, loaded from RON files under `assets/config/`.
//!
//! Every value here can be changed without touching Rust, and with the
//! `dev` feature on, without restarting the game. See `docs/CONTENT.md`.
//!
//! Map settings are deliberately *not* here — they live in `hex_map`, alongside the
//! generation and rendering they configure, so the whole map is owned in one crate.
//! Only the loader is shared.
//!
//! Also not here: the hex geometry constants in [`hex_core::config`]. Those describe the dimensions of `hex.glb` rather than
//! any preference — editing them without editing the mesh produces silently
//! overlapping or gapped tiles, so they are not a knob worth exposing.

use bevy::prelude::*;
use serde::Deserialize;

use hex_core::Level;

/// A colour written as `(r, g, b)` in sRGB, each component 0.0–1.0.
///
/// A plain tuple rather than Bevy's `Color`, whose serialized form is an
/// externally-tagged enum and unpleasant to hand-write.
pub type Rgb = (f32, f32, f32);

/// Converts a settings colour to a Bevy one.
pub fn to_color((r, g, b): Rgb) -> Color {
    Color::srgb(r, g, b)
}

/// `assets/config/camera.ron` — pan, orbit, and zoom feel.
#[derive(Asset, Resource, Reflect, Debug, Clone, Deserialize)]
#[reflect(Resource)]
pub struct CameraSettings {
    /// Camera position applied whenever gameplay starts.
    pub gameplay_eye: (f32, f32, f32),
    /// Point the camera looks at and orbits around whenever gameplay starts.
    pub gameplay_focus: (f32, f32, f32),
    /// WASD pan speed, scaled by zoom distance so panning feels the same when
    /// zoomed out as when zoomed in.
    pub pan_speed: f32,
    /// Added to the zoom radius before scaling pan speed, so panning still works
    /// when fully zoomed in.
    pub pan_speed_offset: f32,
    /// Lowest the camera may tilt toward the horizon, 0.0–1.0.
    pub min_pitch: f32,
    /// Highest the camera may tilt toward straight down, 0.0–1.0.
    pub max_pitch: f32,
    /// Closest the camera may zoom in, in world units.
    pub min_zoom: f32,
    /// Furthest the camera may zoom out, in world units.
    pub max_zoom: f32,
    /// Fraction of the current distance covered per scroll notch.
    pub zoom_sensitivity: f32,
}

/// `assets/config/lighting.ron` — sun, ambient, and sky.
///
/// Bevy uses physical light units: illuminance in lux (~100,000 is direct noon
/// sun, ~10,000 overcast), and skybox brightness in cd/m².
#[derive(Asset, Resource, Reflect, Debug, Clone, Deserialize)]
#[reflect(Resource)]
pub struct LightingSettings {
    /// Sun brightness, in lux.
    pub sun_illuminance: f32,
    /// Sun direction as XYZ Euler angles, in radians.
    pub sun_rotation: (f32, f32, f32),
    /// Fill light applied everywhere, in lux.
    pub ambient_brightness: f32,
    /// Skybox brightness, in cd/m². The cubemap already encodes a bright sky, so
    /// this stays low to avoid blowing out the scene.
    pub skybox_brightness: f32,
    /// Background colour, visible where the skybox is not.
    pub sky_color: Rgb,
}

/// `assets/config/player.ron` — the player piece.
#[derive(Asset, Resource, Reflect, Debug, Clone, Deserialize)]
#[reflect(Resource)]
pub struct PlayerSettings {
    /// Uniform scale applied to the player meshes.
    pub scale: f32,
    /// Movement speed in world units per second.
    pub speed: f32,
    /// Colour of the player piece.
    pub color: Rgb,
    /// How many levels tall the piece is.
    ///
    /// It needs this much clear space above a surface to stand there, so raising it
    /// makes low tunnels and gaps under bridges impassable. Deliberately *not* called
    /// `height`: that word is taken by terrain, and confusing the two is a silent
    /// geometric bug.
    pub levels_tall: Level,
}

/// `assets/config/display.ron` — window and presentation.
#[derive(Asset, Resource, Reflect, Debug, Clone, Deserialize)]
#[reflect(Resource)]
pub struct DisplaySettings {
    /// How frames are handed to the display.
    pub present_mode: PresentModeSetting,
}

/// Frame presentation, i.e. the vsync setting.
///
/// Mirrors Bevy's `PresentMode` rather than using it directly, so the RON stays
/// readable and does not break if Bevy's enum gains variants.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum PresentModeSetting {
    /// Vsync on, capped to the display's refresh rate. No tearing. The default,
    /// and on an adaptive-refresh display (ProMotion, FreeSync, G-Sync) this
    /// already scales down when the scene is static.
    Vsync,
    /// Uncapped. Lowest latency, may tear, and will spin the GPU as fast as it
    /// can go.
    NoVsync,
    /// Vsync without the frame-rate cap, where the platform supports it. Falls
    /// back to `Vsync`.
    Mailbox,
}

impl From<PresentModeSetting> for bevy::window::PresentMode {
    fn from(value: PresentModeSetting) -> Self {
        match value {
            PresentModeSetting::Vsync => Self::AutoVsync,
            PresentModeSetting::NoVsync => Self::AutoNoVsync,
            PresentModeSetting::Mailbox => Self::Mailbox,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_camera_frames_the_showcase() {
        let camera: CameraSettings =
            ron::from_str(include_str!("../../../assets/config/camera.ron"))
                .expect("the shipped camera settings should parse");
        let (eye_x, eye_y, eye_z) = camera.gameplay_eye;
        let (focus_x, focus_y, focus_z) = camera.gameplay_focus;
        assert!(eye_x.abs() < f32::EPSILON);
        assert!((eye_y - 44.0).abs() < f32::EPSILON);
        assert!((eye_z - 38.0).abs() < f32::EPSILON);
        assert!(focus_x.abs() < f32::EPSILON);
        assert!((focus_y - 6.0).abs() < f32::EPSILON);
        assert!(focus_z.abs() < f32::EPSILON);
    }
}
