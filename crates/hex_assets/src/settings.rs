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
/// sun, ~10,000 overcast).
#[derive(Asset, Resource, Reflect, Debug, Clone, Deserialize)]
#[reflect(Resource)]
pub struct LightingSettings {
    /// Sun brightness, in lux.
    pub sun_illuminance: f32,
    /// Sun colour. Warm tints read as low sun; white is midday.
    pub sun_color: Rgb,
    /// Sun direction as XYZ Euler angles, in radians.
    pub sun_rotation: (f32, f32, f32),
    /// Uniform fill applied everywhere, in lux.
    pub ambient_brightness: f32,
    /// Colour of that uniform fill. White leaves shadows neutral; tinting it towards
    /// the sky cools them.
    pub ambient_color: Rgb,
    /// Strength of the optional sky/ground fill light, in cd/m². **0.0 disables it.**
    ///
    /// A directional ambient: `zenith_color` from above, `sky_color` at the horizon,
    /// `ground_color` from below. Unlike `ambient_brightness` it varies with which way
    /// a surface faces, so it tints shadows rather than flattening everything equally.
    /// Keep it small next to `sun_illuminance` — fill that competes with the sun
    /// removes the shading that gives the terrain its shape.
    pub sky_light_intensity: f32,
    /// Colour bounced up from the ground, the underside of the sky light.
    pub ground_color: Rgb,
    /// Sky colour at the horizon, and the `ClearColor` fallback behind the dome.
    pub sky_color: Rgb,
    /// Sky colour at the zenith (straight up). `sky_color` is the horizon colour.
    pub zenith_color: Rgb,
    /// Colour of the hexagonal clouds.
    pub cloud_color: Rgb,
    /// Fraction of hex sky-cells that carry a cloud, 0.0–1.0.
    pub cloud_coverage: f32,
    /// Size of the hex cloud cells; larger = smaller, more numerous clouds.
    pub hex_cloud_scale: f32,
    /// Edge softness of each cloud, ~0.02 (crisp) to ~0.3 (fluffy).
    pub cloud_softness: f32,
    /// Cloud shape from hexagonal to round: 0.0 keeps hard hex edges, 1.0 is a disc.
    pub cloud_roundness: f32,
    /// Strength of the fbm noise that breaks up cloud edges, ~0.0 (clean) to ~0.5 (wispy).
    pub cloud_noise: f32,
    /// Haze colour in the distance. Usually close to `sky_color`.
    pub fog_color: Rgb,
    /// Colour of the haze looking towards the sun, which is what reads as low light.
    pub fog_sun_color: Rgb,
    /// How quickly the haze thickens with distance. **0.0 turns fog off entirely**,
    /// which is how the game ships — at this camera distance haze costs more colour
    /// than it buys atmosphere.
    pub fog_density: f32,
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
