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
use serde::{Deserialize, Deserializer};

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
// `PartialEq` so a test can assert that two scenarios really do produce different
// lighting. Without it the only check available is "a resource exists", which passes
// against an implementation that loads one file and never re-chooses.
#[derive(Asset, Resource, Reflect, Debug, Clone, PartialEq)]
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

        Ok(())
    }
}

#[derive(Deserialize)]
struct UnvalidatedLightingSettings {
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
    /// Colour of the player piece.
    pub color: Rgb,
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

/// One unit's starting point in a scenario.
///
/// Authored maps use an exact cube coordinate. Generated maps publish named anchors
/// after generation, so their scenarios remain valid when a different seed moves the
/// useful parts of the map.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Deserialize)]
pub enum ScenarioPlacement {
    /// An exact coordinate on an authored map.
    Fixed(CubeCoord),
    /// A generated position resolved from [`hex_core::MapAnchors`].
    Anchor(String),
}

/// Where a scenario's units start.
///
/// **Not loaded from a file of its own.** It is the placements out of whichever
/// scenario was chosen, inserted by `hex_game` before gameplay spawns — see
/// [`ScenarioLibrary`](crate::scenario::ScenarioLibrary).
///
/// A scaffold for trying maps out, not an encounter format: a real one will describe
/// many units, their lattices, and what triggers them.
#[derive(Asset, Resource, Reflect, Debug, Clone, PartialEq, Eq, Deserialize)]
#[reflect(Resource)]
pub struct ScenarioSettings {
    /// Where the player starts.
    pub player: ScenarioPlacement,
    /// Where the single enemy starts.
    pub enemy: ScenarioPlacement,
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

    const LIGHTING_RON: &str = include_str!("../../../assets/config/lighting.ron");
    const PLAYER_RON: &str = include_str!("../../../assets/config/player.ron");

    #[test]
    fn player_settings_reject_removed_fields() {
        let player = ron::from_str::<PlayerSettings>(PLAYER_RON)
            .expect("shipped player settings should parse");
        assert!(player.scale.is_finite());

        let stale = PLAYER_RON.replacen("speed:", "levels_tall: 2,\n    speed:", 1);
        assert_ne!(
            stale, PLAYER_RON,
            "the player fixture no longer contains the speed field"
        );
        let error = ron::from_str::<PlayerSettings>(&stale)
            .expect_err("removed player fields must not be silently ignored");
        assert!(
            error.to_string().contains("levels_tall"),
            "stale player setting returned an unrelated error: {error}"
        );
    }

    #[test]
    fn shipped_camera_frames_the_showcase() {
        let camera: CameraSettings =
            ron::from_str(include_str!("../../../assets/config/camera.ron"))
                .expect("the shipped camera settings should parse");
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

        // The optional extras ship disabled; both are removed from the camera rather
        // than applied at zero, so turning them on is the only way to change the look.
        assert!(
            lighting.sky_light_intensity.abs() < f32::EPSILON,
            "the sky light ships off"
        );
        assert!(lighting.fog_density.abs() < f32::EPSILON, "haze ships off");
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

        let invalid = LIGHTING_RON.replacen("cloud_softness: 0.1", "cloud_softness: -0.1", 1);
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
}
