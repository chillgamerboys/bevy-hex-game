//! Designer-facing settings, loaded from RON files under `assets/config/`.
//!
//! Every value here can be changed without touching Rust, and with the
//! `dev` feature on, without restarting the game. See `docs/development/config.md`.
//!
//! Map settings are deliberately *not* here — they live in `hex_map`, alongside the
//! generation and rendering they configure, so the whole map is owned in one crate.
//! Only the loader is shared.
//!
//! Also not here: the hex geometry constants in [`hex_core::config`]. Those describe the dimensions of `hex.glb` rather than
//! any preference — editing them without editing the mesh produces silently
//! overlapping or gapped tiles, so they are not a knob worth exposing.

use bevy::prelude::*;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

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
#[derive(Asset, Resource, Reflect, Debug, Clone)]
#[reflect(Resource)]
pub struct CameraSettings {
    /// Camera position applied whenever gameplay starts.
    pub gameplay_eye: (f32, f32, f32),
    /// Point the camera looks at and orbits around whenever gameplay starts.
    pub gameplay_focus: (f32, f32, f32),
    /// Height above the selected character's feet used as the close-view focus.
    pub character_focus_height: f32,
    /// Initial orbit radius when entering the close character view.
    pub character_radius: f32,
    /// Clearance kept between the close camera and the first obstructing terrain run.
    pub character_collision_margin: f32,
    /// Preferred smallest usable radius while avoiding terrain.
    ///
    /// Character-camera collision first searches nearby yaw directions that can
    /// retain this radius. A complete enclosure may require a smaller radius so
    /// the camera never crosses an actual terrain hit.
    pub character_min_effective_radius: f32,
    /// World units per second used when restoring the close camera after an
    /// obstruction clears.
    pub character_restoration_speed: f32,
    /// Initial close-view pitch as a fraction from the horizon toward straight down.
    pub character_pitch: f32,
    /// Closest the close view may tilt toward the horizon, 0.0–1.0.
    pub character_min_pitch: f32,
    /// Furthest the close view may tilt toward straight down, 0.0–1.0.
    pub character_max_pitch: f32,
    /// WASD pan speed, scaled by zoom distance so panning feels the same when
    /// zoomed out as when zoomed in.
    pub pan_speed: f32,
    /// Added to the zoom radius before scaling pan speed, so panning still works
    /// when fully zoomed in.
    pub pan_speed_offset: f32,
    /// Lowest the map camera may tilt toward the horizon, 0.0–1.0.
    pub min_pitch: f32,
    /// Highest the map camera may tilt toward straight down, 0.0–1.0.
    pub max_pitch: f32,
    /// Closest the camera may zoom in, in world units.
    pub min_zoom: f32,
    /// Furthest the camera may zoom out, in world units.
    pub max_zoom: f32,
    /// Fraction of the current distance covered per scroll notch.
    pub zoom_sensitivity: f32,
}

impl CameraSettings {
    /// Checks camera geometry and controls before settings replace the active asset.
    pub fn validate(&self) -> Result<(), String> {
        validate_finite_vec3("gameplay_eye", self.gameplay_eye)?;
        validate_finite_vec3("gameplay_focus", self.gameplay_focus)?;
        let eye = self.gameplay_eye;
        let focus = self.gameplay_focus;
        let offset_squared = [
            f64::from(eye.0) - f64::from(focus.0),
            f64::from(eye.1) - f64::from(focus.1),
            f64::from(eye.2) - f64::from(focus.2),
        ]
        .into_iter()
        .map(|component| component * component)
        .sum::<f64>();
        if offset_squared <= f64::from(f32::EPSILON) {
            return Err("gameplay_eye and gameplay_focus must be distinct".to_owned());
        }

        validate_nonnegative("character_focus_height", self.character_focus_height)?;
        if !self.character_radius.is_finite() || self.character_radius <= 0.0 {
            return Err("character_radius must be positive and finite".to_owned());
        }
        validate_nonnegative(
            "character_collision_margin",
            self.character_collision_margin,
        )?;
        if self.character_collision_margin >= self.character_radius {
            return Err("character_collision_margin must be less than character_radius".to_owned());
        }
        if !self.character_min_effective_radius.is_finite()
            || self.character_min_effective_radius <= 0.0
        {
            return Err("character_min_effective_radius must be positive and finite".to_owned());
        }
        if self.character_min_effective_radius > self.character_radius {
            return Err(
                "character_min_effective_radius must not exceed character_radius".to_owned(),
            );
        }
        if !self.character_restoration_speed.is_finite() || self.character_restoration_speed <= 0.0
        {
            return Err("character_restoration_speed must be positive and finite".to_owned());
        }
        validate_unit_interval("character_pitch", self.character_pitch)?;
        validate_unit_interval("character_min_pitch", self.character_min_pitch)?;
        validate_unit_interval("character_max_pitch", self.character_max_pitch)?;
        if self.character_min_pitch > self.character_max_pitch {
            return Err("character_min_pitch must not exceed character_max_pitch".to_owned());
        }
        if !(self.character_min_pitch..=self.character_max_pitch).contains(&self.character_pitch) {
            return Err(
                "character_pitch must be within character_min_pitch..=character_max_pitch"
                    .to_owned(),
            );
        }

        validate_nonnegative("pan_speed", self.pan_speed)?;
        validate_nonnegative("pan_speed_offset", self.pan_speed_offset)?;
        validate_unit_interval("min_pitch", self.min_pitch)?;
        validate_unit_interval("max_pitch", self.max_pitch)?;
        if self.min_pitch > self.max_pitch {
            return Err("min_pitch must not exceed max_pitch".to_owned());
        }
        if !self.min_zoom.is_finite() || self.min_zoom <= 0.0 {
            return Err("min_zoom must be positive and finite".to_owned());
        }
        if !self.max_zoom.is_finite() || self.max_zoom <= 0.0 {
            return Err("max_zoom must be positive and finite".to_owned());
        }
        if self.min_zoom > self.max_zoom {
            return Err("min_zoom must not exceed max_zoom".to_owned());
        }
        if !(self.min_zoom..=self.max_zoom).contains(&self.character_radius) {
            return Err("character_radius must be within min_zoom..=max_zoom".to_owned());
        }
        validate_nonnegative("zoom_sensitivity", self.zoom_sensitivity)?;

        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnvalidatedCameraSettings {
    gameplay_eye: (f32, f32, f32),
    gameplay_focus: (f32, f32, f32),
    character_focus_height: f32,
    character_radius: f32,
    character_collision_margin: f32,
    character_min_effective_radius: f32,
    character_restoration_speed: f32,
    character_pitch: f32,
    character_min_pitch: f32,
    character_max_pitch: f32,
    pan_speed: f32,
    pan_speed_offset: f32,
    min_pitch: f32,
    max_pitch: f32,
    min_zoom: f32,
    max_zoom: f32,
    zoom_sensitivity: f32,
}

impl<'de> Deserialize<'de> for CameraSettings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = UnvalidatedCameraSettings::deserialize(deserializer)?;
        let settings = Self {
            gameplay_eye: raw.gameplay_eye,
            gameplay_focus: raw.gameplay_focus,
            character_focus_height: raw.character_focus_height,
            character_radius: raw.character_radius,
            character_collision_margin: raw.character_collision_margin,
            character_min_effective_radius: raw.character_min_effective_radius,
            character_restoration_speed: raw.character_restoration_speed,
            character_pitch: raw.character_pitch,
            character_min_pitch: raw.character_min_pitch,
            character_max_pitch: raw.character_max_pitch,
            pan_speed: raw.pan_speed,
            pan_speed_offset: raw.pan_speed_offset,
            min_pitch: raw.min_pitch,
            max_pitch: raw.max_pitch,
            min_zoom: raw.min_zoom,
            max_zoom: raw.max_zoom,
            zoom_sensitivity: raw.zoom_sensitivity,
        };
        settings.validate().map_err(D::Error::custom)?;
        Ok(settings)
    }
}

/// `assets/config/lighting.ron` — sun, ambient, and sky.
///
/// Bevy uses physical light units: illuminance in lux (~100,000 is direct noon
/// sun, ~10,000 overcast).
// `PartialEq` so a test can assert that two scenarios really do produce different
// lighting. Without it the only check available is "a resource exists", which passes
// against an implementation that loads one file and never re-chooses.
#[derive(Asset, Resource, Reflect, Debug, Clone, PartialEq)]
#[reflect(Resource)]
pub struct LightingSettings {
    /// Optional time-of-day behavior. Older lighting files omit this and remain static.
    pub profile: LightingProfile,
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

/// Whether a lighting asset is a fixed look or a time-resolved celestial cycle.
#[derive(Reflect, Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub enum LightingProfile {
    /// Preserve the authored flat [`LightingSettings`] values exactly.
    #[default]
    Static,
    /// Resolve the authored keyframes at a selected time of day.
    Cycle(CelestialCycleSettings),
}

/// Fixed celestial presentation and the ordered keyframes for a clear-sky day.
#[derive(Reflect, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CelestialCycleSettings {
    /// Time used when a scenario does not provide an override.
    pub default_time_hours: f32,
    /// Visible sRGB colour of the sun disc.
    pub sun_disc_color: Rgb,
    /// Apparent full angular diameter of the sun.
    pub sun_angular_diameter_degrees: f32,
    /// Radial angular thickness of the sun halo outside the disc edge.
    pub sun_halo_width_degrees: f32,
    /// Visible sRGB colour of the moon disc.
    pub moon_disc_color: Rgb,
    /// Apparent full angular diameter of the moon.
    pub moon_angular_diameter_degrees: f32,
    /// Radial angular thickness of the moon halo outside the disc edge.
    pub moon_halo_width_degrees: f32,
    /// Angular radius of the azimuth-local glow mirrored below the sun.
    pub lower_glow_angular_radius_degrees: f32,
    /// Restrained multiplier applied to the interpolated sun halo strength.
    pub lower_glow_strength: f32,
    /// Strictly time-ordered keyframes in the range `[0, 24)`.
    pub keyframes: Vec<LightingKeyframe>,
}

/// The body that supplies the single shadow-casting key light.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CelestialBody {
    /// Direct sunlight supplies the key.
    Sun,
    /// Stylized moonlight supplies the key.
    Moon,
}

/// One authored point in a cyclic clear-sky lighting profile.
#[derive(Reflect, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LightingKeyframe {
    /// Keyframe position on the 24-hour clock.
    pub time_hours: f32,
    /// Sun compass angle, measured from world +Z towards +X.
    pub sun_azimuth_degrees: f32,
    /// Sun height above the true horizon.
    pub sun_elevation_degrees: f32,
    /// Body that casts shadows after the dark handoff.
    pub active_body: CelestialBody,
    /// Key light brightness, in lux.
    pub direct_illuminance: f32,
    /// Key light sRGB colour.
    pub direct_color: Rgb,
    /// Uniform fill brightness.
    pub ambient_brightness: f32,
    /// Uniform fill sRGB colour.
    pub ambient_color: Rgb,
    /// Directional sky fill intensity.
    pub sky_light_intensity: f32,
    /// Upward-bounced sRGB ground colour.
    pub ground_color: Rgb,
    /// Horizon sRGB colour.
    pub sky_color: Rgb,
    /// Zenith sRGB colour.
    pub zenith_color: Rgb,
    /// Cloud sRGB colour.
    pub cloud_color: Rgb,
    /// Distance haze sRGB colour.
    pub fog_color: Rgb,
    /// Sun-facing haze sRGB colour.
    pub fog_sun_color: Rgb,
    /// Exponential haze density.
    pub fog_density: f32,
    /// Camera exposure value at ISO 100.
    pub exposure_ev100: f32,
    /// Strength of the sun halo.
    pub sun_halo_strength: f32,
    /// Strength of the moon halo.
    pub moon_halo_strength: f32,
}

/// A fully resolved lighting frame ready to apply to the renderer.
#[derive(Resource, Reflect, Debug, Clone, PartialEq)]
#[reflect(Resource)]
pub struct ResolvedLighting {
    /// Normalized cycle time, or `None` for a static profile.
    pub time_hours: Option<f32>,
    /// The visible body supplying the key light. Static profiles draw no body.
    pub key_body: Option<CelestialBody>,
    /// Interpolated key light brightness, in lux.
    pub key_illuminance: f32,
    /// Interpolated key light sRGB colour.
    pub key_color: Rgb,
    /// Normalized world-space direction from the scene towards the sun.
    pub sun_direction: Vec3,
    /// Normalized roll reference transported from the legacy directional-light frame.
    pub key_light_up: Vec3,
    /// Interpolated uniform fill brightness.
    pub ambient_brightness: f32,
    /// Interpolated uniform fill sRGB colour.
    pub ambient_color: Rgb,
    /// Interpolated directional sky fill intensity.
    pub sky_light_intensity: f32,
    /// Interpolated upward-bounced sRGB ground colour.
    pub ground_color: Rgb,
    /// Interpolated horizon sRGB colour.
    pub sky_color: Rgb,
    /// Interpolated zenith sRGB colour.
    pub zenith_color: Rgb,
    /// Interpolated cloud sRGB colour.
    pub cloud_color: Rgb,
    /// Cloud area fraction inherited from the flat settings.
    pub cloud_coverage: f32,
    /// Hex-cloud spatial scale inherited from the flat settings.
    pub hex_cloud_scale: f32,
    /// Cloud edge softness inherited from the flat settings.
    pub cloud_softness: f32,
    /// Cloud roundness inherited from the flat settings.
    pub cloud_roundness: f32,
    /// Cloud edge-noise strength inherited from the flat settings.
    pub cloud_noise: f32,
    /// Interpolated distance haze sRGB colour.
    pub fog_color: Rgb,
    /// Interpolated sun-facing haze sRGB colour.
    pub fog_sun_color: Rgb,
    /// Interpolated exponential haze density.
    pub fog_density: f32,
    /// Interpolated camera exposure value at ISO 100.
    pub exposure_ev100: f32,
    /// Fixed visible sRGB sun-disc colour.
    pub sun_disc_color: Rgb,
    /// Fixed apparent full angular diameter of the sun.
    pub sun_angular_diameter_degrees: f32,
    /// Fixed radial angular thickness of the sun halo outside the disc edge.
    pub sun_halo_width_degrees: f32,
    /// Interpolated sun-halo strength.
    pub sun_halo_strength: f32,
    /// Fixed visible sRGB moon-disc colour.
    pub moon_disc_color: Rgb,
    /// Fixed apparent full angular diameter of the moon.
    pub moon_angular_diameter_degrees: f32,
    /// Fixed radial angular thickness of the moon halo outside the disc edge.
    pub moon_halo_width_degrees: f32,
    /// Interpolated moon-halo strength.
    pub moon_halo_strength: f32,
    /// Normalized direction mirrored below the horizon at the sun's azimuth.
    pub lower_glow_direction: Vec3,
    /// Interpolated sRGB colour of the mirrored lower glow.
    pub lower_glow_color: Rgb,
    /// Fixed angular radius of the mirrored lower glow.
    pub lower_glow_angular_radius_degrees: f32,
    /// Interpolated restrained strength of the mirrored lower glow.
    pub lower_glow_strength: f32,
}

impl ResolvedLighting {
    /// World direction from the scene towards the body supplying the key light.
    pub fn key_body_direction(&self) -> Option<Vec3> {
        match self.key_body {
            Some(CelestialBody::Sun) => Some(self.sun_direction),
            Some(CelestialBody::Moon) => Some(-self.sun_direction),
            None => None,
        }
    }

    /// Direction travelled by rays from the active celestial key light.
    pub fn key_light_ray_direction(&self) -> Option<Vec3> {
        self.key_body_direction().map(|direction| -direction)
    }
}

impl LightingSettings {
    /// Checks every value before a lighting asset can replace the active profile.
    ///
    /// The shader assumes finite parameters, positive cloud scale, nonnegative
    /// softness/noise strengths, and ordered `smoothstep` edges. Bevy's physical
    /// light inputs likewise cannot represent negative intensity. Keeping these
    /// checks at deserialization means an invalid hot reload leaves the previous
    /// valid resource active.
    pub fn validate(&self) -> Result<(), String> {
        validate_nonnegative("sun_illuminance", self.sun_illuminance)?;
        validate_rgb("sun_color", self.sun_color)?;
        validate_finite_vec3("sun_rotation", self.sun_rotation)?;

        validate_nonnegative("ambient_brightness", self.ambient_brightness)?;
        validate_rgb("ambient_color", self.ambient_color)?;
        validate_nonnegative("sky_light_intensity", self.sky_light_intensity)?;
        validate_rgb("ground_color", self.ground_color)?;

        validate_rgb("sky_color", self.sky_color)?;
        validate_rgb("zenith_color", self.zenith_color)?;
        validate_rgb("cloud_color", self.cloud_color)?;
        validate_unit_interval("cloud_coverage", self.cloud_coverage)?;
        if !self.hex_cloud_scale.is_finite() || self.hex_cloud_scale <= 0.0 {
            return Err("hex_cloud_scale must be positive and finite".to_owned());
        }
        validate_nonnegative("cloud_softness", self.cloud_softness)?;
        validate_unit_interval("cloud_roundness", self.cloud_roundness)?;
        validate_nonnegative("cloud_noise", self.cloud_noise)?;

        validate_rgb("fog_color", self.fog_color)?;
        validate_rgb("fog_sun_color", self.fog_sun_color)?;
        validate_nonnegative("fog_density", self.fog_density)?;

        if let LightingProfile::Cycle(cycle) = &self.profile {
            cycle.validate()?;
        }

        Ok(())
    }

    /// The cycle's authored starting hour, or `None` for static lighting.
    pub fn default_time_hours(&self) -> Option<f32> {
        match &self.profile {
            LightingProfile::Static => None,
            LightingProfile::Cycle(cycle) => Some(cycle.default_time_hours),
        }
    }

    /// Resolves a renderer-ready frame.
    ///
    /// A supplied time is an error for a static profile. Finite cycle times wrap around
    /// the 24-hour clock, which keeps an inspector scrubber useful outside its nominal
    /// range while scenario data can enforce the stricter `[0, 24)` authoring range.
    pub fn resolve(&self, time_hours: Option<f32>) -> Result<ResolvedLighting, String> {
        match (&self.profile, time_hours) {
            (LightingProfile::Static, Some(_)) => {
                Err("static lighting does not support a time override".to_owned())
            }
            (LightingProfile::Static, None) => Ok(self.resolve_static()),
            (LightingProfile::Cycle(cycle), requested) => {
                let time = requested.unwrap_or(cycle.default_time_hours);
                if !time.is_finite() {
                    return Err("time of day must be finite".to_owned());
                }
                cycle.resolve(self, time.rem_euclid(24.0))
            }
        }
    }

    fn resolve_static(&self) -> ResolvedLighting {
        let (x, y, z) = self.sun_rotation;
        let legacy_rotation = Quat::from_euler(EulerRot::XYZ, x, y, z);
        let sun_direction = legacy_rotation * Vec3::Z;
        let key_light_up = legacy_rotation * Vec3::Y;
        let lower_glow_direction = mirrored_below_horizon(sun_direction);

        ResolvedLighting {
            time_hours: None,
            key_body: None,
            key_illuminance: self.sun_illuminance,
            key_color: self.sun_color,
            sun_direction,
            key_light_up,
            ambient_brightness: self.ambient_brightness,
            ambient_color: self.ambient_color,
            sky_light_intensity: self.sky_light_intensity,
            ground_color: self.ground_color,
            sky_color: self.sky_color,
            zenith_color: self.zenith_color,
            cloud_color: self.cloud_color,
            cloud_coverage: self.cloud_coverage,
            hex_cloud_scale: self.hex_cloud_scale,
            cloud_softness: self.cloud_softness,
            cloud_roundness: self.cloud_roundness,
            cloud_noise: self.cloud_noise,
            fog_color: self.fog_color,
            fog_sun_color: self.fog_sun_color,
            fog_density: self.fog_density,
            exposure_ev100: 9.7,
            sun_disc_color: (1.0, 1.0, 1.0),
            sun_angular_diameter_degrees: 0.53,
            sun_halo_width_degrees: 4.0,
            sun_halo_strength: 0.0,
            moon_disc_color: (0.86, 0.90, 1.0),
            moon_angular_diameter_degrees: 0.53,
            moon_halo_width_degrees: 3.0,
            moon_halo_strength: 0.0,
            lower_glow_direction,
            lower_glow_color: self.fog_sun_color,
            lower_glow_angular_radius_degrees: 20.0,
            lower_glow_strength: 0.0,
        }
    }
}

impl CelestialCycleSettings {
    fn validate(&self) -> Result<(), String> {
        validate_time("profile.default_time_hours", self.default_time_hours)?;
        validate_rgb("profile.sun_disc_color", self.sun_disc_color)?;
        validate_angle_size(
            "profile.sun_angular_diameter_degrees",
            self.sun_angular_diameter_degrees,
            10.0,
        )?;
        validate_angle_size(
            "profile.sun_halo_width_degrees",
            self.sun_halo_width_degrees,
            90.0,
        )?;
        validate_rgb("profile.moon_disc_color", self.moon_disc_color)?;
        validate_angle_size(
            "profile.moon_angular_diameter_degrees",
            self.moon_angular_diameter_degrees,
            10.0,
        )?;
        validate_angle_size(
            "profile.moon_halo_width_degrees",
            self.moon_halo_width_degrees,
            90.0,
        )?;
        validate_angle_size(
            "profile.lower_glow_angular_radius_degrees",
            self.lower_glow_angular_radius_degrees,
            180.0,
        )?;
        validate_unit_interval("profile.lower_glow_strength", self.lower_glow_strength)?;

        if self.keyframes.len() < 3 {
            return Err("profile.keyframes must contain at least three entries".to_owned());
        }
        for (index, frame) in self.keyframes.iter().enumerate() {
            frame.validate(index)?;
        }
        if self
            .keyframes
            .iter()
            .zip(self.keyframes.iter().skip(1))
            .any(|(previous, current)| previous.time_hours >= current.time_hours)
        {
            return Err(
                "profile.keyframes must be unique and strictly ordered by time_hours".to_owned(),
            );
        }

        if !self
            .keyframes
            .iter()
            .any(|frame| frame.active_body == CelestialBody::Sun)
            || !self
                .keyframes
                .iter()
                .any(|frame| frame.active_body == CelestialBody::Moon)
        {
            return Err("profile.keyframes must include both Sun and Moon key lights".to_owned());
        }

        self.validate_dark_handoffs()
    }

    fn validate_dark_handoffs(&self) -> Result<(), String> {
        let frame_count = self.keyframes.len();
        for index in 0..frame_count {
            let Some(current) = self.keyframes.get(index) else {
                continue;
            };
            let next_index = (index + 1) % frame_count;
            let Some(next) = self.keyframes.get(next_index) else {
                continue;
            };
            if current.active_body == next.active_body {
                continue;
            }

            let current_is_horizon = current.sun_elevation_degrees.abs() <= f32::EPSILON;
            let next_is_horizon = next.sun_elevation_degrees.abs() <= f32::EPSILON;
            if !current_is_horizon && !next_is_horizon {
                return Err(format!(
                    "profile.keyframes body change between indices {index} and {next_index} \
                     crosses the horizon mid-segment; author a zero-elevation keyframe at the \
                     dark handoff"
                ));
            }

            // `handoff_body` changes immediately after a zero-elevation start, at a
            // zero-elevation end, or (when both endpoints sit on the horizon) at the
            // darker endpoint selected by the same illuminance rule.
            let handoff_index = if current_is_horizon
                && (!next_is_horizon || current.direct_illuminance <= next.direct_illuminance)
            {
                index
            } else {
                next_index
            };
            let previous_index = (handoff_index + frame_count - 1) % frame_count;
            let following_index = (handoff_index + 1) % frame_count;
            let Some(handoff) = self.keyframes.get(handoff_index) else {
                continue;
            };
            let Some(previous) = self.keyframes.get(previous_index) else {
                continue;
            };
            let Some(following) = self.keyframes.get(following_index) else {
                continue;
            };
            if handoff.direct_illuminance > previous.direct_illuminance
                || handoff.direct_illuminance > following.direct_illuminance
            {
                return Err(format!(
                    "profile.keyframes body change between indices {index} and {next_index} must \
                     occur at a local minimum of direct_illuminance (index {handoff_index})"
                ));
            }
        }
        Ok(())
    }

    fn resolve(
        &self,
        base: &LightingSettings,
        time_hours: f32,
    ) -> Result<ResolvedLighting, String> {
        let (start, end, amount) = self.segment(time_hours)?;
        let smooth_amount = amount * amount * (3.0 - 2.0 * amount);
        let sun_azimuth_degrees =
            lerp_angle_degrees(start.sun_azimuth_degrees, end.sun_azimuth_degrees, amount);
        let sun_elevation_degrees = lerp(
            start.sun_elevation_degrees,
            end.sun_elevation_degrees,
            amount,
        );
        let sun_direction = celestial_direction(sun_azimuth_degrees, sun_elevation_degrees);
        let sun_halo_strength = lerp(
            start.sun_halo_strength,
            end.sun_halo_strength,
            smooth_amount,
        );
        let key_body = handoff_body(start, end, amount, sun_elevation_degrees);
        let key_body_direction = match key_body {
            CelestialBody::Sun => sun_direction,
            CelestialBody::Moon => -sun_direction,
        };
        let key_light_up = transported_legacy_light_up(base, key_body_direction);
        let lower_glow_elevation =
            low_solar_elevation_factor(sun_elevation_degrees, LOWER_GLOW_MAX_ELEVATION_DEGREES);

        Ok(ResolvedLighting {
            time_hours: Some(time_hours),
            key_body: Some(key_body),
            key_illuminance: lerp(
                start.direct_illuminance,
                end.direct_illuminance,
                smooth_amount,
            ),
            key_color: lerp_rgb_linear(start.direct_color, end.direct_color, smooth_amount),
            sun_direction,
            key_light_up,
            ambient_brightness: lerp(
                start.ambient_brightness,
                end.ambient_brightness,
                smooth_amount,
            ),
            ambient_color: lerp_rgb_linear(start.ambient_color, end.ambient_color, smooth_amount),
            sky_light_intensity: lerp(
                start.sky_light_intensity,
                end.sky_light_intensity,
                smooth_amount,
            ),
            ground_color: lerp_rgb_linear(start.ground_color, end.ground_color, smooth_amount),
            sky_color: lerp_rgb_linear(start.sky_color, end.sky_color, smooth_amount),
            zenith_color: lerp_rgb_linear(start.zenith_color, end.zenith_color, smooth_amount),
            cloud_color: lerp_rgb_linear(start.cloud_color, end.cloud_color, smooth_amount),
            cloud_coverage: base.cloud_coverage,
            hex_cloud_scale: base.hex_cloud_scale,
            cloud_softness: base.cloud_softness,
            cloud_roundness: base.cloud_roundness,
            cloud_noise: base.cloud_noise,
            fog_color: lerp_rgb_linear(start.fog_color, end.fog_color, smooth_amount),
            fog_sun_color: lerp_rgb_linear(start.fog_sun_color, end.fog_sun_color, smooth_amount),
            fog_density: lerp(start.fog_density, end.fog_density, smooth_amount),
            exposure_ev100: lerp(start.exposure_ev100, end.exposure_ev100, amount),
            sun_disc_color: self.sun_disc_color,
            sun_angular_diameter_degrees: self.sun_angular_diameter_degrees,
            sun_halo_width_degrees: self.sun_halo_width_degrees,
            sun_halo_strength,
            moon_disc_color: self.moon_disc_color,
            moon_angular_diameter_degrees: self.moon_angular_diameter_degrees,
            moon_halo_width_degrees: self.moon_halo_width_degrees,
            moon_halo_strength: lerp(
                start.moon_halo_strength,
                end.moon_halo_strength,
                smooth_amount,
            ),
            lower_glow_direction: mirrored_below_horizon(sun_direction),
            lower_glow_color: lerp_rgb_linear(
                start.fog_sun_color,
                end.fog_sun_color,
                smooth_amount,
            ),
            lower_glow_angular_radius_degrees: self.lower_glow_angular_radius_degrees,
            lower_glow_strength: self.lower_glow_strength
                * sun_halo_strength
                * lower_glow_elevation,
        })
    }

    fn segment(
        &self,
        time_hours: f32,
    ) -> Result<(&LightingKeyframe, &LightingKeyframe, f32), String> {
        let first = self
            .keyframes
            .first()
            .ok_or_else(|| "profile.keyframes must not be empty".to_owned())?;
        let last = self
            .keyframes
            .last()
            .ok_or_else(|| "profile.keyframes must not be empty".to_owned())?;
        let start = self
            .keyframes
            .iter()
            .rev()
            .find(|frame| frame.time_hours <= time_hours)
            .unwrap_or(last);
        let end = self
            .keyframes
            .iter()
            .find(|frame| frame.time_hours > time_hours)
            .unwrap_or(first);
        let start_time = start.time_hours;
        let mut end_time = end.time_hours;
        let mut sample_time = time_hours;
        if end_time <= start_time {
            end_time += 24.0;
            if sample_time < start_time {
                sample_time += 24.0;
            }
        }
        let amount = (sample_time - start_time) / (end_time - start_time);
        Ok((start, end, amount.clamp(0.0, 1.0)))
    }
}

impl LightingKeyframe {
    fn validate(&self, index: usize) -> Result<(), String> {
        let field = |name: &str| format!("profile.keyframes[{index}].{name}");
        validate_time(&field("time_hours"), self.time_hours)?;
        validate_range(
            &field("sun_azimuth_degrees"),
            self.sun_azimuth_degrees,
            0.0,
            360.0,
            false,
        )?;
        validate_range(
            &field("sun_elevation_degrees"),
            self.sun_elevation_degrees,
            -90.0,
            90.0,
            true,
        )?;
        let active_body_is_above_horizon = match self.active_body {
            CelestialBody::Sun => self.sun_elevation_degrees >= 0.0,
            CelestialBody::Moon => self.sun_elevation_degrees <= 0.0,
        };
        if !active_body_is_above_horizon {
            return Err(format!(
                "{} must select a body at or above its true horizon",
                field("active_body")
            ));
        }
        validate_nonnegative(&field("direct_illuminance"), self.direct_illuminance)?;
        validate_rgb(&field("direct_color"), self.direct_color)?;
        validate_nonnegative(&field("ambient_brightness"), self.ambient_brightness)?;
        validate_rgb(&field("ambient_color"), self.ambient_color)?;
        validate_nonnegative(&field("sky_light_intensity"), self.sky_light_intensity)?;
        validate_rgb(&field("ground_color"), self.ground_color)?;
        validate_rgb(&field("sky_color"), self.sky_color)?;
        validate_rgb(&field("zenith_color"), self.zenith_color)?;
        validate_rgb(&field("cloud_color"), self.cloud_color)?;
        validate_rgb(&field("fog_color"), self.fog_color)?;
        validate_rgb(&field("fog_sun_color"), self.fog_sun_color)?;
        validate_nonnegative(&field("fog_density"), self.fog_density)?;
        validate_range(
            &field("exposure_ev100"),
            self.exposure_ev100,
            -10.0,
            30.0,
            true,
        )?;
        validate_unit_interval(&field("sun_halo_strength"), self.sun_halo_strength)?;
        validate_unit_interval(&field("moon_halo_strength"), self.moon_halo_strength)
    }
}

const LOWER_GLOW_MAX_ELEVATION_DEGREES: f32 = 18.0;

fn handoff_body(
    start: &LightingKeyframe,
    end: &LightingKeyframe,
    amount: f32,
    sun_elevation_degrees: f32,
) -> CelestialBody {
    if sun_elevation_degrees > f32::EPSILON {
        CelestialBody::Sun
    } else if sun_elevation_degrees < -f32::EPSILON {
        CelestialBody::Moon
    } else if amount <= f32::EPSILON || start.active_body == end.active_body {
        start.active_body
    } else if start.direct_illuminance <= end.direct_illuminance {
        end.active_body
    } else {
        start.active_body
    }
}

fn low_solar_elevation_factor(elevation_degrees: f32, maximum_degrees: f32) -> f32 {
    let amount = (1.0 - elevation_degrees.abs() / maximum_degrees).clamp(0.0, 1.0);
    amount * amount * (3.0 - 2.0 * amount)
}

fn transported_legacy_light_up(base: &LightingSettings, target_body_direction: Vec3) -> Vec3 {
    let (x, y, z) = base.sun_rotation;
    let legacy_rotation = Quat::from_euler(EulerRot::XYZ, x, y, z);
    let legacy_body_direction = legacy_rotation * Vec3::Z;
    let legacy_up = legacy_rotation * Vec3::Y;
    (Quat::from_rotation_arc(legacy_body_direction, target_body_direction) * legacy_up).normalize()
}

fn celestial_direction(azimuth_degrees: f32, elevation_degrees: f32) -> Vec3 {
    let azimuth = azimuth_degrees.to_radians();
    let elevation = elevation_degrees.to_radians();
    let horizontal = elevation.cos();
    Vec3::new(
        azimuth.sin() * horizontal,
        elevation.sin(),
        azimuth.cos() * horizontal,
    )
    .normalize()
}

fn mirrored_below_horizon(direction: Vec3) -> Vec3 {
    Vec3::new(direction.x, -direction.y.abs(), direction.z).normalize()
}

fn lerp(start: f32, end: f32, amount: f32) -> f32 {
    start + (end - start) * amount
}

fn lerp_angle_degrees(start: f32, end: f32, amount: f32) -> f32 {
    let delta = (end - start + 180.0).rem_euclid(360.0) - 180.0;
    (start + delta * amount).rem_euclid(360.0)
}

fn lerp_rgb_linear(start: Rgb, end: Rgb, amount: f32) -> Rgb {
    if amount <= f32::EPSILON {
        return start;
    }
    if amount >= 1.0 - f32::EPSILON {
        return end;
    }
    let channel =
        |from: f32, to: f32| linear_to_srgb(lerp(srgb_to_linear(from), srgb_to_linear(to), amount));
    (
        channel(start.0, end.0),
        channel(start.1, end.1),
        channel(start.2, end.2),
    )
}

fn srgb_to_linear(value: f32) -> f32 {
    if value <= 0.040_45 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(value: f32) -> f32 {
    if value <= 0.003_130_8 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnvalidatedLightingSettings {
    #[serde(default)]
    profile: LightingProfile,
    sun_illuminance: f32,
    sun_color: Rgb,
    sun_rotation: (f32, f32, f32),
    ambient_brightness: f32,
    ambient_color: Rgb,
    sky_light_intensity: f32,
    ground_color: Rgb,
    sky_color: Rgb,
    zenith_color: Rgb,
    cloud_color: Rgb,
    cloud_coverage: f32,
    hex_cloud_scale: f32,
    cloud_softness: f32,
    cloud_roundness: f32,
    cloud_noise: f32,
    fog_color: Rgb,
    fog_sun_color: Rgb,
    fog_density: f32,
}

impl<'de> Deserialize<'de> for LightingSettings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = UnvalidatedLightingSettings::deserialize(deserializer)?;
        let settings = Self {
            profile: raw.profile,
            sun_illuminance: raw.sun_illuminance,
            sun_color: raw.sun_color,
            sun_rotation: raw.sun_rotation,
            ambient_brightness: raw.ambient_brightness,
            ambient_color: raw.ambient_color,
            sky_light_intensity: raw.sky_light_intensity,
            ground_color: raw.ground_color,
            sky_color: raw.sky_color,
            zenith_color: raw.zenith_color,
            cloud_color: raw.cloud_color,
            cloud_coverage: raw.cloud_coverage,
            hex_cloud_scale: raw.hex_cloud_scale,
            cloud_softness: raw.cloud_softness,
            cloud_roundness: raw.cloud_roundness,
            cloud_noise: raw.cloud_noise,
            fog_color: raw.fog_color,
            fog_sun_color: raw.fog_sun_color,
            fog_density: raw.fog_density,
        };
        settings.validate().map_err(D::Error::custom)?;
        Ok(settings)
    }
}

fn validate_nonnegative(name: &str, value: f32) -> Result<(), String> {
    if !value.is_finite() || value < 0.0 {
        return Err(format!("{name} must be nonnegative and finite"));
    }
    Ok(())
}

fn validate_time(name: &str, value: f32) -> Result<(), String> {
    validate_range(name, value, 0.0, 24.0, false)
}

fn validate_angle_size(name: &str, value: f32, maximum: f32) -> Result<(), String> {
    if !value.is_finite() || value <= 0.0 || value > maximum {
        return Err(format!(
            "{name} must be positive, finite, and no greater than {maximum}"
        ));
    }
    Ok(())
}

fn validate_range(
    name: &str,
    value: f32,
    minimum: f32,
    maximum: f32,
    inclusive_maximum: bool,
) -> Result<(), String> {
    let in_range = if inclusive_maximum {
        (minimum..=maximum).contains(&value)
    } else {
        (minimum..maximum).contains(&value)
    };
    if !value.is_finite() || !in_range {
        let upper = if inclusive_maximum { "..=" } else { ".." };
        return Err(format!(
            "{name} must be finite and in {minimum}{upper}{maximum}"
        ));
    }
    Ok(())
}

fn validate_unit_interval(name: &str, value: f32) -> Result<(), String> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(format!("{name} must be finite and in 0.0..=1.0"));
    }
    Ok(())
}

fn validate_finite_vec3(name: &str, (x, y, z): (f32, f32, f32)) -> Result<(), String> {
    if [x, y, z].into_iter().any(|value| !value.is_finite()) {
        return Err(format!("{name} components must be finite"));
    }
    Ok(())
}

fn validate_rgb(name: &str, (red, green, blue): Rgb) -> Result<(), String> {
    for (channel, value) in [("red", red), ("green", green), ("blue", blue)] {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(format!("{name}.{channel} must be finite and in 0.0..=1.0"));
        }
    }
    Ok(())
}

/// `assets/config/player.ron` — the player piece.
#[derive(Asset, Resource, Reflect, Debug, Clone, Deserialize)]
#[reflect(Resource)]
#[serde(deny_unknown_fields)]
pub struct PlayerSettings {
    /// Uniform scale applied to the player meshes.
    pub scale: f32,
    /// Movement speed in world units per second.
    pub speed: f32,
}

/// `assets/config/display.ron` — window and presentation.
#[derive(Asset, Resource, Reflect, Debug, Clone, Deserialize)]
#[reflect(Resource)]
pub struct DisplaySettings {
    /// How frames are handed to the display.
    pub present_mode: PresentModeSetting,
}

/// `assets/config/menu.ron` — how the menus look.
///
/// Separate from [`LightingSettings`] on purpose. The menus used to borrow
/// `sky_color`, which worked only because lighting happened to load at startup — and
/// stopped being true the moment lighting became something a scenario chooses. The
/// menus are not in the world and should not inherit its weather.
///
/// One field so far. It has a file of its own because the next thing the menus need —
/// a second colour, a background image — has an obvious place to go.
#[derive(Asset, Resource, Reflect, Debug, Clone, Deserialize)]
#[reflect(Resource)]
pub struct MenuSettings {
    /// Flat colour behind the splash, title and loading screens.
    pub background: Rgb,
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

/// A hex coordinate as written in RON: `(x: 0, y: 0, z: 0)`.
///
/// A plain struct rather than `HexCoord`, whose fields are private on purpose — it
/// stores axial and presents cube, and a settings file should not have to know that.
/// Named fields rather than a bare triple because the constraint below is invisible
/// otherwise, and `(2, -2, 0)` gives a designer nothing to check against.
///
/// **The three must sum to zero.** That is what makes cube coordinates a hex grid
/// rather than three independent numbers; see
/// <https://www.redblobgames.com/grids/hexagons/>.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct CubeCoord {
    /// East–west axis.
    pub x: i32,
    /// North-east to south-west axis.
    pub y: i32,
    /// North-west to south-east axis. Always `-x - y`.
    pub z: i32,
}

// Unit placement used to live here, as a `ScenarioSettings` holding exactly one player
// coordinate and one enemy coordinate. It is a roster in `crate::encounter` now, which a
// scenario names by path the same way it names its world and its lighting. `CubeCoord`
// stayed: an authored placement is still a cube coordinate, and so is a formation's
// centre.

/// `assets/config/combat.ron` — the combat policy knobs.
///
/// Moves the provisional constants out of compiled code and names the
/// deliberately-open design questions as data. Unbuilt policy variants parse
/// but fail validation with a reason naming what they wait on — flipping a
/// playtest option is a file edit, and nothing gets settled by accident.
#[derive(Asset, Resource, Reflect, Debug, Clone, PartialEq, Eq)]
#[reflect(Resource)]
pub struct CombatSettings {
    /// Hexes between a hostile and the party that start a fight.
    pub engage_range: u32,
    /// Extra hexes beyond `engage_range` a hostile must retreat before combat
    /// ends — the hysteresis that stops boundary flapping.
    pub disengage_margin: u32,
    /// Hexes a unit may move on its turn.
    pub movement_per_turn: u32,
    /// Initiative for a unit that declares none.
    pub default_initiative: u32,
    /// Levels of height that buy one extra hex of range.
    pub levels_per_bonus_range: u32,
    /// Hexes a melee strike disables, before the defender's own defences subtract.
    ///
    /// A strike is the one attack every unit has, spell or not — a wolf is four hexes
    /// and a bite — so it needs a number and there is no content file for it: content
    /// describes spells. One is the smallest thing damage can be, and matches Ember,
    /// the cheapest spell in the roster. It is a knob because it is a balance number
    /// nobody has played with yet, not because it is settled.
    pub strike_disables: u16,
    /// Further round rollovers a tier of divination survives.
    ///
    /// A tier-one Reveal written midway through a round is visible for the remainder
    /// of that partial round and one complete following round, then expires at the
    /// next rollover when this value is `1`.
    pub divination_rounds_per_tier: u32,
    /// How turn order is decided. Only [`InitiativePolicy::FlatComponent`] is built.
    pub initiative_policy: InitiativePolicy,
    /// What a turn affords. Only [`ActionEconomy::MoveAndAction`] is built.
    pub action_economy: ActionEconomy,
    /// Whether mana returns passively. Only [`ChannellingTrickle::BurstOnly`] is built.
    pub channelling_trickle: ChannellingTrickle,
    /// How fights end short of annihilation. Only [`RoutPolicy::FightToTheEnd`] is built.
    pub rout_policy: RoutPolicy,
}

/// The initiative options from `docs/design/game.md` § Open questions.
#[derive(Reflect, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitiativePolicy {
    /// Today's placeholder: a flat number on the unit, ties by stable id.
    FlatComponent,
    /// Derived from the lattice (Air attunement or capacity).
    DerivedFromLattice,
    /// One roll per combat, fixed.
    RolledPerCombat,
    /// Re-rolled each round.
    RerolledEachRound,
    /// Fixed order with a hold/delay action.
    FixedWithHold,
}

/// The action-economy options from the design's open questions.
#[derive(Reflect, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionEconomy {
    /// Today's shape: a movement budget plus one action per turn.
    MoveAndAction,
    /// Strict one action: move *or* cast *or* channel.
    StrictOneAction,
    /// Free small movement plus one action — the design's current preference.
    FreeMovementPlusAction,
    /// Action points, roughly three per turn.
    ActionPoints,
}

/// Whether channelling trickles passively or only bursts on the action.
#[derive(Reflect, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannellingTrickle {
    /// Today's rule: mana returns only when the channel action is taken.
    BurstOnly,
    /// A passive per-turn trickle with the action as a burst refill.
    TrickleWithBurst,
}

/// How a fight can end before one side is annihilated.
#[derive(Reflect, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutPolicy {
    /// Today's rule: fights end only by distance or destruction.
    FightToTheEnd,
    /// A morale threshold routs a side before zero.
    RoutThreshold,
    /// Enemies can offer surrender.
    SurrenderOffers,
}

impl CombatSettings {
    /// Checks the knob ranges and rejects unbuilt policy variants with the
    /// reason each one waits on.
    pub fn validate(&self) -> Result<(), String> {
        if self.engage_range == 0 {
            return Err("combat.ron: engage_range must be at least 1".to_owned());
        }
        if self.disengage_margin == 0 {
            return Err(
                "combat.ron: disengage_margin must be at least 1 — zero re-opens the \
                 boundary flapping the margin exists to stop"
                    .to_owned(),
            );
        }
        if self.movement_per_turn == 0 {
            return Err("combat.ron: movement_per_turn must be at least 1".to_owned());
        }
        if self.levels_per_bonus_range == 0 {
            return Err(
                "combat.ron: levels_per_bonus_range must be at least 1 — zero would make \
                 every height difference an unbounded range bonus"
                    .to_owned(),
            );
        }
        if self.strike_disables == 0 {
            return Err(
                "combat.ron: strike_disables must be at least 1 — zero makes melee do \
                 nothing, which looks exactly like the game before damage existed"
                    .to_owned(),
            );
        }
        if self.divination_rounds_per_tier == 0 {
            return Err(
                "combat.ron: divination_rounds_per_tier must be at least 1 — zero \
                 would make Reveal lapse at the first rollover"
                    .to_owned(),
            );
        }
        match self.initiative_policy {
            InitiativePolicy::FlatComponent => {}
            other => {
                return Err(format!(
                    "combat.ron: initiative_policy {other:?} is not built yet — it waits on \
                     the initiative question being settled (docs/design/game.md, Open \
                     questions) and the selected policy being implemented"
                ));
            }
        }
        match self.action_economy {
            ActionEconomy::MoveAndAction => {}
            other => {
                return Err(format!(
                    "combat.ron: action_economy {other:?} is not built yet — it waits on \
                     the command funnel's turn budgets growing beyond movement-plus-action"
                ));
            }
        }
        match self.channelling_trickle {
            ChannellingTrickle::BurstOnly => {}
            other => {
                return Err(format!(
                    "combat.ron: channelling_trickle {other:?} is not built yet — it waits \
                     on the channelling question and Channel implementation"
                ));
            }
        }
        match self.rout_policy {
            RoutPolicy::FightToTheEnd => {}
            other => {
                return Err(format!(
                    "combat.ron: rout_policy {other:?} is not built yet — it waits on \
                     morale rules that do not exist"
                ));
            }
        }
        Ok(())
    }
}

impl Default for CombatSettings {
    /// The shipped `combat.ron` values, for tests that never load assets.
    /// Production never defaults: the resource is absent until the file parses.
    fn default() -> Self {
        Self {
            engage_range: 4,
            disengage_margin: 2,
            movement_per_turn: 4,
            default_initiative: 10,
            levels_per_bonus_range: 5,
            strike_disables: 1,
            divination_rounds_per_tier: 1,
            initiative_policy: InitiativePolicy::FlatComponent,
            action_economy: ActionEconomy::MoveAndAction,
            channelling_trickle: ChannellingTrickle::BurstOnly,
            rout_policy: RoutPolicy::FightToTheEnd,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnvalidatedCombatSettings {
    engage_range: u32,
    disengage_margin: u32,
    movement_per_turn: u32,
    default_initiative: u32,
    levels_per_bonus_range: u32,
    strike_disables: u16,
    divination_rounds_per_tier: u32,
    initiative_policy: InitiativePolicy,
    action_economy: ActionEconomy,
    channelling_trickle: ChannellingTrickle,
    rout_policy: RoutPolicy,
}

impl<'de> Deserialize<'de> for CombatSettings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = UnvalidatedCombatSettings::deserialize(deserializer)?;
        let settings = Self {
            engage_range: raw.engage_range,
            disengage_margin: raw.disengage_margin,
            movement_per_turn: raw.movement_per_turn,
            default_initiative: raw.default_initiative,
            levels_per_bonus_range: raw.levels_per_bonus_range,
            strike_disables: raw.strike_disables,
            divination_rounds_per_tier: raw.divination_rounds_per_tier,
            initiative_policy: raw.initiative_policy,
            action_economy: raw.action_economy,
            channelling_trickle: raw.channelling_trickle,
            rout_policy: raw.rout_policy,
        };
        settings.validate().map_err(D::Error::custom)?;
        Ok(settings)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    use bevy::asset::{AssetLoadFailedEvent, AssetPlugin};
    use bevy::ecs::system::RunSystemOnce;

    use crate::loader::{choose_settings, LoadSettings, SelectSettings, SettingsRegistry};

    use super::*;

    const CAMERA_RON: &str = include_str!("../../../assets/config/camera.ron");
    const LIGHTING_RON: &str = include_str!("../../../assets/config/lighting.ron");
    const OVERCAST_RON: &str = include_str!("../../../assets/config/lighting/overcast.ron");
    const PLAYER_RON: &str = include_str!("../../../assets/config/player.ron");

    fn assert_approx_eq(actual: f32, expected: f32) {
        let tolerance = f32::EPSILON * expected.abs().max(1.0);
        assert!(
            (actual - expected).abs() <= tolerance,
            "expected {actual} to be within {tolerance} of {expected}"
        );
    }

    #[test]
    fn player_settings_reject_removed_fields() {
        let player = ron::from_str::<PlayerSettings>(PLAYER_RON)
            .expect("shipped player settings should parse");
        assert!(player.scale.is_finite());

        for (field, value) in [("levels_tall", "2"), ("color", "(1.0, 0.2, 0.2)")] {
            let stale = PLAYER_RON.replacen("speed:", &format!("{field}: {value},\n    speed:"), 1);
            assert_ne!(
                stale, PLAYER_RON,
                "the player fixture no longer contains the speed field"
            );
            let error = ron::from_str::<PlayerSettings>(&stale)
                .expect_err("removed player fields must not be silently ignored");
            assert!(
                error.to_string().contains(field),
                "stale player setting returned an unrelated error: {error}"
            );
        }
    }

    #[test]
    fn shipped_camera_frames_the_showcase() {
        let camera: CameraSettings =
            ron::from_str(CAMERA_RON).expect("the shipped camera settings should parse");
        let (eye_x, eye_y, eye_z) = camera.gameplay_eye;
        let (focus_x, focus_y, focus_z) = camera.gameplay_focus;
        assert!(eye_x.abs() < f32::EPSILON);
        assert!((eye_y - 48.0).abs() < f32::EPSILON);
        assert!((eye_z - 42.0).abs() < f32::EPSILON);
        assert!(focus_x.abs() < f32::EPSILON);
        assert!((focus_y - 6.0).abs() < f32::EPSILON);
        assert!(focus_z.abs() < f32::EPSILON);

        let initial_radius =
            ((eye_x - focus_x).powi(2) + (eye_y - focus_y).powi(2) + (eye_z - focus_z).powi(2))
                .sqrt();
        assert!(
            initial_radius <= camera.max_zoom * 0.9,
            "the full-map frame should retain at least 10% manual zoom-out headroom"
        );
        assert!((camera.character_focus_height - 0.4).abs() < f32::EPSILON);
        assert!((camera.character_radius - 7.0).abs() < f32::EPSILON);
        assert!((camera.character_collision_margin - 0.35).abs() < f32::EPSILON);
        assert!((camera.character_min_effective_radius - 1.5).abs() < f32::EPSILON);
        assert!((camera.character_restoration_speed - 8.0).abs() < f32::EPSILON);
        assert!((camera.character_pitch - 0.3).abs() < f32::EPSILON);
        assert!((camera.character_min_pitch - 0.05).abs() < f32::EPSILON);
        assert!((camera.character_max_pitch - 0.95).abs() < f32::EPSILON);
    }

    #[test]
    fn invalid_camera_values_are_rejected_during_deserialization() {
        for (needle, replacement, expected) in [
            (
                "gameplay_eye: (0.0, 48.0, 42.0)",
                "gameplay_eye: (NaN, 48.0, 42.0)",
                "gameplay_eye",
            ),
            (
                "gameplay_focus: (0.0, 6.0, 0.0)",
                "gameplay_focus: (0.0, inf, 0.0)",
                "gameplay_focus",
            ),
            (
                "gameplay_focus: (0.0, 6.0, 0.0)",
                "gameplay_focus: (0.0, 48.0, 42.0)",
                "distinct",
            ),
            (
                "character_focus_height: 0.4",
                "character_focus_height: -0.1",
                "character_focus_height",
            ),
            (
                "character_radius: 7.0",
                "character_radius: 0.0",
                "character_radius",
            ),
            (
                "character_radius: 7.0",
                "character_radius: 4.0",
                "character_radius",
            ),
            (
                "character_collision_margin: 0.35",
                "character_collision_margin: -0.1",
                "character_collision_margin",
            ),
            (
                "character_collision_margin: 0.35",
                "character_collision_margin: 7.0",
                "character_collision_margin",
            ),
            (
                "character_min_effective_radius: 1.5",
                "character_min_effective_radius: 0.0",
                "character_min_effective_radius",
            ),
            (
                "character_min_effective_radius: 1.5",
                "character_min_effective_radius: 8.0",
                "character_min_effective_radius",
            ),
            (
                "character_restoration_speed: 8.0",
                "character_restoration_speed: 0.0",
                "character_restoration_speed",
            ),
            (
                "character_pitch: 0.3",
                "character_pitch: 1.0",
                "character_pitch",
            ),
            (
                "character_min_pitch: 0.05",
                "character_min_pitch: -0.1",
                "character_min_pitch",
            ),
            (
                "character_max_pitch: 0.95",
                "character_max_pitch: 1.1",
                "character_max_pitch",
            ),
            (
                "character_min_pitch: 0.05",
                "character_min_pitch: 0.96",
                "character_min_pitch",
            ),
            ("pan_speed: 0.4", "pan_speed: -0.1", "pan_speed"),
            (
                "pan_speed_offset: 10.0",
                "pan_speed_offset: NaN",
                "pan_speed_offset",
            ),
            ("min_pitch: 0.25", "min_pitch: -0.1", "min_pitch"),
            ("max_pitch: 0.95", "max_pitch: 1.1", "max_pitch"),
            ("min_pitch: 0.25", "min_pitch: 0.96", "min_pitch"),
            ("min_zoom: 5.0", "min_zoom: 0.0", "min_zoom"),
            ("max_zoom: 70.0", "max_zoom: inf", "max_zoom"),
            ("max_zoom: 70.0", "max_zoom: 4.0", "min_zoom"),
            (
                "zoom_sensitivity: 0.2",
                "zoom_sensitivity: -0.1",
                "zoom_sensitivity",
            ),
        ] {
            let invalid = CAMERA_RON.replacen(needle, replacement, 1);
            assert_ne!(
                invalid, CAMERA_RON,
                "the test fixture no longer contains {needle:?}"
            );

            let error = ron::from_str::<CameraSettings>(&invalid)
                .expect_err("invalid camera settings should fail deserialization");
            assert!(
                error.to_string().contains(expected),
                "{replacement:?} returned an unrelated error: {error}"
            );
        }
    }

    /// Every field must be present, or the game hangs on "loading…" with the reason
    /// only in the terminal. `LightingSettings` has eighteen of them and no default,
    /// so a rename or a dropped line is otherwise caught by launching the game.
    /// The menu's own settings parse.
    ///
    /// Its own file, and its own test, because the menu deliberately no longer borrows
    /// anything from the lighting: that coupling only worked while lighting loaded at
    /// startup, and stopped being true when scenarios started choosing it.
    #[test]
    fn shipped_menu_settings_parse() {
        let menu: MenuSettings = ron::from_str(include_str!("../../../assets/config/menu.ron"))
            .expect("the shipped menu settings should parse");

        let (r, g, b) = menu.background;
        for channel in [r, g, b] {
            assert!(
                (0.0..=1.0).contains(&channel),
                "menu.ron: background channels are 0.0-1.0, got {menu:?}"
            );
        }
    }

    #[test]
    fn shipped_lighting_settings_parse() {
        let lighting: LightingSettings =
            ron::from_str(LIGHTING_RON).expect("the shipped lighting settings should parse");

        assert!(matches!(lighting.profile, LightingProfile::Cycle(_)));
        assert_eq!(lighting.default_time_hours(), Some(12.0));

        // The optional extras ship disabled; both are removed from the camera rather
        // than applied at zero, so turning them on is the only way to change the look.
        assert!(
            lighting.sky_light_intensity.abs() < f32::EPSILON,
            "the sky light ships off"
        );
        assert!(lighting.fog_density.abs() < f32::EPSILON, "haze ships off");
    }

    #[test]
    fn legacy_lighting_without_a_profile_remains_static() {
        let lighting: LightingSettings =
            ron::from_str(OVERCAST_RON).expect("the legacy flat format should still parse");

        assert_eq!(lighting.profile, LightingProfile::Static);
        assert_eq!(lighting.default_time_hours(), None);
        let resolved = lighting
            .resolve(None)
            .expect("static lighting should resolve without an override");
        assert_eq!(resolved.time_hours, None);
        assert_eq!(resolved.key_body, None);
        assert_approx_eq(resolved.key_illuminance, lighting.sun_illuminance);
        assert_eq!(resolved.key_color, lighting.sun_color);
        assert_approx_eq(resolved.ambient_brightness, lighting.ambient_brightness);
        assert_eq!(resolved.sky_color, lighting.sky_color);
        assert_approx_eq(resolved.exposure_ev100, 9.7);
        assert_approx_eq(resolved.sun_halo_strength, 0.0);
        assert_approx_eq(resolved.moon_halo_strength, 0.0);
        assert_approx_eq(resolved.lower_glow_strength, 0.0);

        let expected_direction = {
            let (x, y, z) = lighting.sun_rotation;
            Quat::from_euler(EulerRot::XYZ, x, y, z) * Vec3::Z
        };
        assert!(resolved
            .sun_direction
            .abs_diff_eq(expected_direction, 1.0e-6));
        let (x, y, z) = lighting.sun_rotation;
        let legacy_rotation = Quat::from_euler(EulerRot::XYZ, x, y, z);
        assert!(resolved
            .key_light_up
            .abs_diff_eq(legacy_rotation * Vec3::Y, 1.0e-6));
        let restored = Transform::default()
            .looking_to(-resolved.sun_direction, resolved.key_light_up)
            .rotation;
        assert!(
            restored.dot(legacy_rotation).abs() >= 1.0 - 1.0e-6,
            "static lighting must retain the complete legacy orientation"
        );

        let error = lighting
            .resolve(Some(12.0))
            .expect_err("a static profile must reject a time override");
        assert!(error.contains("static lighting"));
    }

    #[test]
    fn clear_noon_resolves_to_the_existing_flat_look() {
        let lighting: LightingSettings =
            ron::from_str(LIGHTING_RON).expect("the shipped cycle should parse");
        let noon = lighting
            .resolve(None)
            .expect("the default clear-sky hour should resolve");

        assert_eq!(noon.time_hours, Some(12.0));
        assert_eq!(noon.key_body, Some(CelestialBody::Sun));
        assert_approx_eq(noon.key_illuminance, lighting.sun_illuminance);
        assert_eq!(noon.key_color, lighting.sun_color);
        assert_approx_eq(noon.ambient_brightness, lighting.ambient_brightness);
        assert_eq!(noon.ambient_color, lighting.ambient_color);
        assert_approx_eq(noon.sky_light_intensity, lighting.sky_light_intensity);
        assert_eq!(noon.ground_color, lighting.ground_color);
        assert_eq!(noon.sky_color, lighting.sky_color);
        assert_eq!(noon.zenith_color, lighting.zenith_color);
        assert_eq!(noon.cloud_color, lighting.cloud_color);
        assert_approx_eq(noon.cloud_coverage, lighting.cloud_coverage);
        assert_approx_eq(noon.hex_cloud_scale, lighting.hex_cloud_scale);
        assert_approx_eq(noon.cloud_softness, lighting.cloud_softness);
        assert_approx_eq(noon.cloud_roundness, lighting.cloud_roundness);
        assert_approx_eq(noon.cloud_noise, lighting.cloud_noise);
        assert_eq!(noon.fog_color, lighting.fog_color);
        assert_eq!(noon.fog_sun_color, lighting.fog_sun_color);
        assert_approx_eq(noon.fog_density, lighting.fog_density);
        assert_approx_eq(noon.exposure_ev100, 9.7);
        assert_approx_eq(noon.lower_glow_strength, 0.0);

        let (x, y, z) = lighting.sun_rotation;
        let legacy_rotation = Quat::from_euler(EulerRot::XYZ, x, y, z);
        let existing_direction = legacy_rotation * Vec3::Z;
        assert!(
            noon.sun_direction.abs_diff_eq(existing_direction, 1.0e-6),
            "the explicit noon azimuth/elevation must retain the current shadow direction"
        );
        assert!(noon.key_light_up.is_finite() && noon.key_light_up.is_normalized());
        assert!(noon.key_light_up.dot(noon.sun_direction).abs() <= 1.0e-6);
        let restored = Transform::default()
            .looking_to(-noon.sun_direction, noon.key_light_up)
            .rotation;
        assert!(
            restored.dot(legacy_rotation).abs() >= 1.0 - 1.0e-6,
            "cycle noon must retain the complete legacy orientation"
        );
    }

    #[test]
    fn cycle_wraps_at_midnight_and_uses_shortest_path_azimuth() {
        let lighting: LightingSettings =
            ron::from_str(LIGHTING_RON).expect("the shipped cycle should parse");

        let midnight = lighting
            .resolve(Some(0.0))
            .expect("midnight should resolve");
        let wrapped = lighting
            .resolve(Some(24.0))
            .expect("finite inspector values should wrap");
        assert_eq!(wrapped, midnight);

        let before_midnight = lighting
            .resolve(Some(23.0))
            .expect("the cyclic final segment should resolve");
        assert_eq!(before_midnight.time_hours, Some(23.0));
        assert_eq!(before_midnight.key_body, Some(CelestialBody::Moon));
        assert!(before_midnight.sun_direction.y < 0.0);

        // Sunrise -> noon crosses 360 degrees. Halfway must be close to north, not
        // on the opposite side of the sky.
        let morning = lighting
            .resolve(Some(9.25))
            .expect("the morning segment should resolve");
        let azimuth = morning
            .sun_direction
            .x
            .atan2(morning.sun_direction.z)
            .to_degrees()
            .rem_euclid(360.0);
        assert!(
            !(10.0..=350.0).contains(&azimuth),
            "shortest-path interpolation should cross north, got {azimuth}"
        );
    }

    #[test]
    fn exact_keyframes_and_dark_body_handoffs_are_deterministic() {
        let lighting: LightingSettings =
            ron::from_str(LIGHTING_RON).expect("the shipped cycle should parse");
        let LightingProfile::Cycle(cycle) = &lighting.profile else {
            panic!("the shipped clear profile should cycle");
        };

        for frame in &cycle.keyframes {
            let resolved = lighting
                .resolve(Some(frame.time_hours))
                .expect("an authored keyframe should resolve exactly");
            assert_eq!(resolved.key_body, Some(frame.active_body));
            assert_approx_eq(resolved.key_illuminance, frame.direct_illuminance);
            assert_eq!(resolved.key_color, frame.direct_color);
            assert_approx_eq(resolved.exposure_ev100, frame.exposure_ev100);
        }

        assert_eq!(
            lighting.resolve(Some(6.19)).unwrap().key_body,
            Some(CelestialBody::Moon)
        );
        assert_eq!(
            lighting.resolve(Some(6.2)).unwrap().key_body,
            Some(CelestialBody::Sun)
        );
        assert_eq!(
            lighting.resolve(Some(18.74)).unwrap().key_body,
            Some(CelestialBody::Sun)
        );
        assert_eq!(
            lighting.resolve(Some(18.75)).unwrap().key_body,
            Some(CelestialBody::Moon)
        );
    }

    #[test]
    fn celestial_key_stays_above_the_horizon_and_aligned_for_the_full_cycle() {
        let lighting: LightingSettings =
            ron::from_str(LIGHTING_RON).expect("the shipped cycle should parse");

        for minute in 0_u16..24 * 60 {
            let hour = f32::from(minute) / 60.0;
            let resolved = lighting
                .resolve(Some(hour))
                .expect("every minute in the shipped cycle should resolve");
            let body_direction = resolved
                .key_body_direction()
                .expect("a cycle should always have a celestial key");
            let light_ray_direction = resolved
                .key_light_ray_direction()
                .expect("a cycle should always have celestial light rays");

            assert!(
                body_direction.y >= -1.0e-6,
                "{hour:.4}h selected {:?} below its horizon: {body_direction:?}",
                resolved.key_body
            );
            assert!(
                body_direction.is_normalized() && light_ray_direction.is_normalized(),
                "{hour:.4}h produced a non-normalized celestial direction"
            );
            assert!(
                resolved.key_light_up.is_finite() && resolved.key_light_up.is_normalized(),
                "{hour:.4}h produced an invalid key-light roll reference"
            );
            assert!(
                resolved.key_light_up.dot(body_direction).abs() <= 1.0e-5,
                "{hour:.4}h key-light roll reference must be orthogonal to its body direction"
            );
            assert!(
                (body_direction + light_ray_direction).length_squared() <= 1.0e-10,
                "{hour:.4}h body direction must be inverse to the light-ray direction"
            );
        }
    }

    #[test]
    fn mirrored_lower_glow_is_limited_to_low_solar_elevations() {
        let lighting: LightingSettings =
            ron::from_str(LIGHTING_RON).expect("the shipped cycle should parse");
        let noon = lighting.resolve(Some(12.0)).unwrap();
        let sunset = lighting.resolve(Some(18.5)).unwrap();
        let night = lighting.resolve(Some(0.0)).unwrap();

        assert_approx_eq(noon.lower_glow_strength, 0.0);
        assert!(
            sunset.lower_glow_strength > 0.0,
            "the map camera should retain a localized sunset reflection"
        );
        assert_approx_eq(night.lower_glow_strength, 0.0);
        assert_approx_eq(
            low_solar_elevation_factor(0.0, LOWER_GLOW_MAX_ELEVATION_DEGREES),
            1.0,
        );
        assert_approx_eq(
            low_solar_elevation_factor(18.0, LOWER_GLOW_MAX_ELEVATION_DEGREES),
            0.0,
        );
        assert_approx_eq(
            low_solar_elevation_factor(-18.0, LOWER_GLOW_MAX_ELEVATION_DEGREES),
            0.0,
        );
    }

    #[test]
    fn cycle_colors_interpolate_in_linear_rgb_and_exposure_in_ev() {
        let midpoint = lerp_rgb_linear((0.0, 0.0, 0.0), (1.0, 1.0, 1.0), 0.5);
        for channel in [midpoint.0, midpoint.1, midpoint.2] {
            assert!((channel - 0.735_357).abs() < 1.0e-5);
        }

        let lighting: LightingSettings =
            ron::from_str(LIGHTING_RON).expect("the shipped cycle should parse");
        let morning = lighting.resolve(Some(9.25)).unwrap();
        assert_approx_eq(morning.exposure_ev100, (8.0 + 9.7) / 2.0);
    }

    #[test]
    fn malformed_cycle_profiles_are_rejected_during_deserialization() {
        for (needle, replacement, expected) in [
            (
                "default_time_hours: 12.0",
                "default_time_hours: 24.0",
                "default_time_hours",
            ),
            (
                "sun_angular_diameter_degrees: 1.2",
                "sun_angular_diameter_degrees: 0.0",
                "sun_angular_diameter_degrees",
            ),
            (
                "sun_azimuth_degrees: 218.17207",
                "sun_azimuth_degrees: 360.0",
                "sun_azimuth_degrees",
            ),
            (
                "sun_elevation_degrees: -61.434143",
                "sun_elevation_degrees: -91.0",
                "sun_elevation_degrees",
            ),
            (
                "sun_elevation_degrees: -61.434143,\n                active_body: Moon",
                "sun_elevation_degrees: -61.434143,\n                active_body: Sun",
                "active_body",
            ),
            ("time_hours: 5.0", "time_hours: 0.0", "strictly ordered"),
            (
                "sun_halo_strength: 0.80",
                "sun_halo_strength: 1.1",
                "sun_halo_strength",
            ),
            (
                "direct_illuminance: 220.0",
                "direct_illuminance: 4000.0",
                "local minimum",
            ),
            (
                "sun_elevation_degrees: 0.0,\n                active_body: Sun",
                "sun_elevation_degrees: 1.0,\n                active_body: Sun",
                "crosses the horizon mid-segment",
            ),
        ] {
            let invalid = LIGHTING_RON.replacen(needle, replacement, 1);
            assert_ne!(invalid, LIGHTING_RON, "missing fixture needle {needle:?}");
            let error = ron::from_str::<LightingSettings>(&invalid)
                .expect_err("invalid cycle settings should fail deserialization");
            assert!(
                error.to_string().contains(expected),
                "{replacement:?} returned an unrelated error: {error}"
            );
        }

        let error = ron::from_str::<LightingSettings>(&LIGHTING_RON.replacen(
            "lower_glow_strength: 0.18",
            "lower_glow_strength: NaN",
            1,
        ))
        .expect_err("nonfinite cycle values should fail deserialization");
        assert!(error.to_string().contains("lower_glow_strength"));
        assert!(LightingSettings {
            profile: LightingProfile::Static,
            ..ron::from_str::<LightingSettings>(OVERCAST_RON).unwrap()
        }
        .resolve(Some(f32::NAN))
        .unwrap_err()
        .contains("static lighting"));

        let cycle = ron::from_str::<LightingSettings>(LIGHTING_RON).unwrap();
        assert!(
            cycle.resolve(Some(f32::INFINITY)).is_err(),
            "a nonfinite inspector time must not enter the resolver"
        );
    }

    #[test]
    fn invalid_lighting_values_are_rejected_during_deserialization() {
        for (needle, replacement, expected) in [
            (
                "sun_illuminance: 10000.0",
                "sun_illuminance: -1.0",
                "sun_illuminance",
            ),
            (
                "sun_color: (1.0, 1.0, 1.0)",
                "sun_color: (1.01, 1.0, 1.0)",
                "sun_color.red",
            ),
            (
                "sun_rotation: (11.4, 0.3, 0.0)",
                "sun_rotation: (11.4, NaN, 0.0)",
                "sun_rotation",
            ),
            (
                "ambient_brightness: 80.0",
                "ambient_brightness: inf",
                "ambient_brightness",
            ),
            (
                "ambient_color: (1.0, 1.0, 1.0)",
                "ambient_color: (1.0, NaN, 1.0)",
                "ambient_color.green",
            ),
            (
                "sky_light_intensity: 0.0",
                "sky_light_intensity: -0.1",
                "sky_light_intensity",
            ),
            (
                "ground_color: (0.32, 0.27, 0.21)",
                "ground_color: (0.32, 0.27, NaN)",
                "ground_color.blue",
            ),
            (
                "sky_color: (0.55, 0.80, 0.95)",
                "sky_color: (-0.01, 0.80, 0.95)",
                "sky_color.red",
            ),
            (
                "zenith_color: (0.25, 0.50, 0.85)",
                "zenith_color: (0.25, inf, 0.85)",
                "zenith_color.green",
            ),
            (
                "cloud_color: (0.97, 0.98, 1.0)",
                "cloud_color: (0.97, 0.98, NaN)",
                "cloud_color.blue",
            ),
            (
                "cloud_coverage: 0.18",
                "cloud_coverage: 1.01",
                "cloud_coverage",
            ),
            (
                "hex_cloud_scale: 16.0",
                "hex_cloud_scale: 0.0",
                "hex_cloud_scale",
            ),
            (
                "cloud_softness: 0.1",
                "cloud_softness: -0.1",
                "cloud_softness",
            ),
            (
                "cloud_roundness: 0.5",
                "cloud_roundness: NaN",
                "cloud_roundness",
            ),
            ("cloud_noise: 0.3", "cloud_noise: -0.1", "cloud_noise"),
            (
                "fog_color: (0.62, 0.72, 0.82)",
                "fog_color: (NaN, 0.72, 0.82)",
                "fog_color.red",
            ),
            (
                "fog_sun_color: (1.0, 0.78, 0.50)",
                "fog_sun_color: (1.0, 1.01, 0.50)",
                "fog_sun_color.green",
            ),
            ("fog_density: 0.0", "fog_density: inf", "fog_density"),
        ] {
            let invalid = LIGHTING_RON.replacen(needle, replacement, 1);
            assert_ne!(
                invalid, LIGHTING_RON,
                "the test fixture no longer contains {needle:?}"
            );

            let error = ron::from_str::<LightingSettings>(&invalid)
                .expect_err("invalid lighting should fail deserialization");
            assert!(
                error.to_string().contains(expected),
                "{replacement:?} returned an unrelated error: {error}"
            );
        }
    }

    #[test]
    fn zero_cloud_softness_keeps_only_the_shaders_analytic_edge_width() {
        let no_extra_softness =
            LIGHTING_RON.replacen("cloud_softness: 0.1", "cloud_softness: 0.0", 1);

        assert!(
            ron::from_str::<LightingSettings>(&no_extra_softness).is_ok(),
            "zero softness is valid because the shader still supplies an analytic width"
        );
    }

    #[derive(Resource, Default)]
    struct SawLightingLoadFailure(bool);

    fn record_lighting_load_failure(
        mut failures: MessageReader<AssetLoadFailedEvent<LightingSettings>>,
        mut saw_failure: ResMut<SawLightingLoadFailure>,
    ) {
        if failures.read().next().is_some() {
            saw_failure.0 = true;
        }
    }

    fn update_until(app: &mut App, mut predicate: impl FnMut(&World) -> bool) -> bool {
        for _ in 0..600 {
            app.update();
            if predicate(app.world()) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        false
    }

    static TEMP_ASSET_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TempAssetRoot(PathBuf);

    impl TempAssetRoot {
        fn new() -> std::io::Result<Self> {
            let sequence = TEMP_ASSET_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "hex-assets-lighting-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir_all(&path)?;
            Ok(Self(path))
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempAssetRoot {
        fn drop(&mut self) {
            if let Err(error) = fs::remove_dir_all(&self.0) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    warn!(
                        "could not remove test asset directory {:?}: {error}",
                        self.0
                    );
                }
            }
        }
    }

    #[test]
    fn invalid_hot_reload_keeps_the_previous_lighting_resource_and_recovers() {
        let root = TempAssetRoot::new().expect("the temporary asset directory should be created");
        let lighting_path = root.path().join("lighting.ron");
        fs::write(&lighting_path, LIGHTING_RON)
            .expect("the valid lighting fixture should be written");

        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin {
                file_path: root.path().to_string_lossy().into_owned(),
                ..default()
            },
        ));
        app.load_settings::<LightingSettings>("lighting.ron", &["ron"]);
        app.init_resource::<SawLightingLoadFailure>();
        app.add_systems(Update, record_lighting_load_failure);
        app.finish();
        app.cleanup();

        assert!(
            update_until(&mut app, |world| world
                .contains_resource::<LightingSettings>()),
            "the valid lighting fixture did not load"
        );
        let previous = app.world().resource::<LightingSettings>().clone();

        let invalid = LIGHTING_RON.replacen("profile:", "profle:", 1);
        fs::write(&lighting_path, invalid).expect("the invalid lighting edit should be written");
        app.world().resource::<AssetServer>().reload("lighting.ron");

        assert!(
            update_until(&mut app, |world| {
                world.resource::<SawLightingLoadFailure>().0
            }),
            "the invalid reload did not report an asset failure"
        );
        assert_eq!(
            app.world().resource::<LightingSettings>(),
            &previous,
            "an invalid reload replaced the last valid resource"
        );

        let recovered =
            LIGHTING_RON.replacen("sun_illuminance: 10000.0", "sun_illuminance: 12000.0", 1);
        fs::write(&lighting_path, recovered)
            .expect("the corrected lighting edit should be written");
        app.world().resource::<AssetServer>().reload("lighting.ron");

        assert!(
            update_until(&mut app, |world| {
                (world.resource::<LightingSettings>().sun_illuminance - 12000.0).abs()
                    < f32::EPSILON
            }),
            "a valid edit after an invalid reload did not replace the lighting resource"
        );
    }

    #[test]
    fn selected_lighting_keeps_the_previous_resource_on_invalid_reload_and_recovers() {
        let root = TempAssetRoot::new().expect("the temporary asset directory should be created");
        let lighting_path = root.path().join("lighting.ron");
        fs::write(&lighting_path, LIGHTING_RON)
            .expect("the valid lighting fixture should be written");

        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin {
                file_path: root.path().to_string_lossy().into_owned(),
                ..default()
            },
        ));
        app.select_settings::<LightingSettings>(&["ron"]);
        app.init_resource::<SawLightingLoadFailure>();
        app.add_systems(Update, record_lighting_load_failure);
        app.finish();
        app.cleanup();

        app.world_mut()
            .run_system_once(
                |mut commands: Commands,
                 asset_server: Res<AssetServer>,
                 mut registry: ResMut<SettingsRegistry>| {
                    choose_settings::<LightingSettings>(
                        &mut commands,
                        &asset_server,
                        &mut registry,
                        "lighting.ron",
                    );
                },
            )
            .expect("the lighting choice system should run");

        assert!(
            update_until(&mut app, |world| {
                world.contains_resource::<LightingSettings>()
                    && world.resource::<SettingsRegistry>().all_loaded()
            }),
            "the selected valid lighting fixture did not load"
        );
        let previous = app.world().resource::<LightingSettings>().clone();

        let invalid = LIGHTING_RON.replacen("cloud_softness: 0.1", "cloud_softness: -0.1", 1);
        fs::write(&lighting_path, invalid).expect("the invalid lighting edit should be written");
        app.world().resource::<AssetServer>().reload("lighting.ron");

        assert!(
            update_until(&mut app, |world| {
                world.resource::<SawLightingLoadFailure>().0
            }),
            "the invalid selected-settings reload did not report an asset failure"
        );
        assert_eq!(
            app.world().resource::<LightingSettings>(),
            &previous,
            "an invalid selected-settings reload replaced the last valid resource"
        );

        let recovered =
            LIGHTING_RON.replacen("sun_illuminance: 10000.0", "sun_illuminance: 12000.0", 1);
        fs::write(&lighting_path, recovered)
            .expect("the corrected lighting edit should be written");
        app.world().resource::<AssetServer>().reload("lighting.ron");

        assert!(
            update_until(&mut app, |world| {
                (world.resource::<LightingSettings>().sun_illuminance - 12000.0).abs()
                    < f32::EPSILON
            }),
            "a valid selected-settings edit after an invalid reload did not replace the resource"
        );
    }

    #[test]
    fn the_shipped_combat_settings_parse() {
        let combat: CombatSettings =
            ron::from_str(include_str!("../../../assets/config/combat.ron"))
                .expect("the shipped combat settings should parse");
        assert_eq!(
            combat,
            CombatSettings::default(),
            "the shipped file and the test default must agree"
        );
    }

    #[test]
    fn invalid_combat_values_are_rejected_during_deserialization() {
        let shipped = include_str!("../../../assets/config/combat.ron");
        let cases = [
            ("engage_range: 4", "engage_range: 0", "engage_range"),
            (
                "disengage_margin: 2",
                "disengage_margin: 0",
                "disengage_margin",
            ),
            (
                "movement_per_turn: 4",
                "movement_per_turn: 0",
                "movement_per_turn",
            ),
            (
                "levels_per_bonus_range: 5",
                "levels_per_bonus_range: 0",
                "levels_per_bonus_range",
            ),
            (
                "divination_rounds_per_tier: 1",
                "divination_rounds_per_tier: 0",
                "divination_rounds_per_tier",
            ),
        ];
        for (from, to, named) in cases {
            let invalid = shipped.replace(from, to);
            let error = ron::from_str::<CombatSettings>(&invalid)
                .expect_err("a zero knob should be rejected")
                .to_string();
            assert!(
                error.contains(named),
                "the rejection should name {named}: {error}"
            );
        }
    }

    #[test]
    fn divination_duration_is_required() {
        let shipped = include_str!("../../../assets/config/combat.ron");
        let missing = shipped.replace("    divination_rounds_per_tier: 1,\n", "");
        let error = ron::from_str::<CombatSettings>(&missing)
            .expect_err("a missing divination duration must not default silently")
            .to_string();
        assert!(
            error.contains("divination_rounds_per_tier"),
            "the missing-field error should name the required knob: {error}"
        );
    }

    #[test]
    fn unbuilt_policy_variants_reject_with_a_reason() {
        let shipped = include_str!("../../../assets/config/combat.ron");
        let cases = [
            (
                "initiative_policy: FlatComponent",
                "initiative_policy: DerivedFromLattice",
                "initiative question being settled",
            ),
            (
                "action_economy: MoveAndAction",
                "action_economy: FreeMovementPlusAction",
                "command funnel",
            ),
            (
                "channelling_trickle: BurstOnly",
                "channelling_trickle: TrickleWithBurst",
                "channelling question",
            ),
            (
                "rout_policy: FightToTheEnd",
                "rout_policy: RoutThreshold",
                "morale",
            ),
        ];
        for (from, to, reason) in cases {
            let flipped = shipped.replace(from, to);
            let error = ron::from_str::<CombatSettings>(&flipped)
                .expect_err("an unbuilt policy variant should be rejected")
                .to_string();
            assert!(
                error.contains(reason),
                "the rejection should say what it waits on ({reason}): {error}"
            );
        }
    }

    #[test]
    fn combat_settings_reject_unknown_fields() {
        let shipped = include_str!("../../../assets/config/combat.ron");
        let stale = shipped.replace("engage_range: 4,", "engage_range: 4,\n    engage_rage: 9,");
        ron::from_str::<CombatSettings>(&stale)
            .expect_err("a misspelled knob must fail loudly, not be silently ignored");
    }
}
