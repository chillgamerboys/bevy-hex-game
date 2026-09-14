use std::collections::VecDeque;

use bevy::camera::Exposure;
use bevy::light::EnvironmentMapLight;
use bevy::prelude::*;

use hex_assets::{to_color, CelestialBody, LightingProfile, LightingSettings, ResolvedLighting};
use hex_core::{
    AppSystems, ExteriorIllumination, GameplaySetup, GameplaySetupFailure, IlluminationLevel,
    PerceptionSystems, Screen,
};

use crate::LightingSystems;

/// Designer-controlled gameplay time in hours.
///
/// The resource exists only for an active cyclic-lighting gameplay session. Scenario
/// setup inserts its configured or profile-default value, and review automation may
/// replace it. The development inspector previews through a separate render-only
/// resource and never edits this clock. Static profiles do not insert it. No runtime
/// system advances it.
#[derive(Resource, Reflect, Debug, Clone, Copy, PartialEq)]
#[reflect(Resource)]
pub struct TimeOfDay {
    /// Hour in the half-open range `0.0..24.0`.
    pub hours: f32,
}

impl Default for TimeOfDay {
    fn default() -> Self {
        Self { hours: 12.0 }
    }
}

/// Optional render-only time selected by the development preview.
///
/// `TimeOfDay` remains the authoritative gameplay/session clock. When `hours` is
/// present, only the resolved presentation frame uses this value. Static lighting
/// profiles ignore the override. Gameplay entry and exit both clear it.
#[cfg(feature = "dev-time-preview")]
#[derive(Resource, Reflect, Debug, Default, Clone, Copy, PartialEq)]
#[reflect(Resource)]
pub struct PresentationTimeOverride {
    /// Preview hour. Resolution wraps finite values into `0.0..24.0`.
    pub hours: Option<f32>,
}

#[cfg(feature = "dev-time-preview")]
impl PresentationTimeOverride {
    /// Restores rendering to the authoritative `TimeOfDay` resource.
    pub fn clear(&mut self) {
        self.hours = None;
    }
}

/// Clears a pending render-only time preview without changing authoritative time.
#[cfg(feature = "dev-time-preview")]
pub fn reset_presentation_time_override(mut preview: ResMut<PresentationTimeOverride>) {
    if preview.bypass_change_detection().hours.is_some() {
        preview.clear();
    }
}

// One retained image per quarter-hour across a full day. The 96-step scrub test is
// the contract: changing the development slider's interval must revisit this bound.
const ENVIRONMENT_MAP_CACHE_CAPACITY: usize = 96;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EnvironmentGradientKey {
    channels: [u32; 9],
}

impl EnvironmentGradientKey {
    fn new(lighting: &ResolvedLighting) -> Self {
        let (zenith_r, zenith_g, zenith_b) = lighting.zenith_color;
        let (horizon_r, horizon_g, horizon_b) = lighting.sky_color;
        let (ground_r, ground_g, ground_b) = lighting.ground_color;
        Self {
            channels: [
                zenith_r.to_bits(),
                zenith_g.to_bits(),
                zenith_b.to_bits(),
                horizon_r.to_bits(),
                horizon_g.to_bits(),
                horizon_b.to_bits(),
                ground_r.to_bits(),
                ground_g.to_bits(),
                ground_b.to_bits(),
            ],
        }
    }
}

#[derive(Resource, Default)]
struct EnvironmentMapCache {
    entries: VecDeque<(EnvironmentGradientKey, Handle<Image>)>,
    allocations: usize,
}

impl EnvironmentMapCache {
    fn image_for(
        &mut self,
        images: &mut Assets<Image>,
        lighting: &ResolvedLighting,
    ) -> Handle<Image> {
        let key = EnvironmentGradientKey::new(lighting);
        if let Some(position) = self
            .entries
            .iter()
            .position(|(candidate, _)| *candidate == key)
        {
            if let Some(entry) = self.entries.remove(position) {
                let handle = entry.1.clone();
                self.entries.push_back(entry);
                return handle;
            }
        }

        if self.entries.len() >= ENVIRONMENT_MAP_CACHE_CAPACITY {
            if let Some((_, evicted)) = self.entries.pop_front() {
                // Cached handles stay private and apply_view_lighting installs them
                // only on the Camera3d views it updates together. Do not let these
                // handles escape to other components without revisiting eviction.
                drop(images.remove(evicted.id()));
            }
        }

        let handle = EnvironmentMapLight::hemispherical_gradient(
            images,
            to_color(lighting.zenith_color),
            to_color(lighting.sky_color),
            to_color(lighting.ground_color),
        )
        .diffuse_map;
        self.entries.push_back((key, handle.clone()));
        self.allocations = self.allocations.saturating_add(1);
        handle
    }

    fn clear(&mut self, images: Option<&mut Assets<Image>>) {
        if let Some(images) = images {
            for (_, handle) in self.entries.drain(..) {
                drop(images.remove(handle.id()));
            }
        } else {
            self.entries.clear();
        }
    }
}

/// Drops all cached hemispherical environment maps owned by world lighting.
///
/// The world plugin invokes this on lighting-profile reload and gameplay entry/exit.
/// It is public so a lighting hot-reload coordinator can request the same cleanup.
pub fn clear_environment_map_cache(world: &mut World) {
    let Some(mut cache) = world.remove_resource::<EnvironmentMapCache>() else {
        return;
    };
    let owned_handles: Vec<_> = cache
        .entries
        .iter()
        .map(|(_, handle)| handle.clone())
        .collect();
    let mut cameras = world.query_filtered::<(Entity, &EnvironmentMapLight), With<Camera3d>>();
    let cameras: Vec<_> = cameras
        .iter(world)
        .filter_map(|(entity, environment)| {
            owned_handles
                .iter()
                .any(|handle| {
                    environment.diffuse_map == *handle || environment.specular_map == *handle
                })
                .then_some(entity)
        })
        .collect();

    for entity in cameras {
        world.entity_mut(entity).remove::<EnvironmentMapLight>();
    }

    if let Some(mut images) = world.get_resource_mut::<Assets<Image>>() {
        cache.clear(Some(&mut images));
    } else {
        cache.clear(None);
    }
    world.insert_resource(cache);
}

/// Registers the celestial key light and the resolved scene/view lighting.
pub fn plugin(app: &mut App) {
    app.register_type::<TimeOfDay>()
        .register_type::<CelestialKeyLight>()
        .register_type::<ExteriorIllumination>()
        .register_type::<IlluminationLevel>()
        .init_resource::<EnvironmentMapCache>()
        .configure_sets(
            Update,
            (LightingSystems::Resolve, LightingSystems::Apply)
                .chain()
                .in_set(AppSystems::Update),
        )
        .add_systems(
            OnEnter(Screen::Gameplay),
            (
                clear_environment_map_cache,
                synchronize_authoritative_time,
                resolve_lighting,
                spawn_celestial_key_light,
                apply_scene_lighting,
                apply_view_lighting,
            )
                .chain()
                .in_set(GameplaySetup::Terrain),
        )
        .add_systems(
            OnEnter(Screen::Gameplay),
            publish_exterior_illumination
                .after(resolve_lighting)
                .in_set(PerceptionSystems::PublishAmbient),
        )
        .add_systems(
            OnExit(Screen::Gameplay),
            (
                despawn_celestial_key_light,
                clear_exterior_illumination,
                restore_authoritative_lighting_on_exit,
                clear_authoritative_time,
                clear_environment_map_cache,
            )
                .chain(),
        )
        // A resolved frame changes only when its source asset or the inspector's
        // session clock changes. Applying it is similarly change-gated, so a frozen
        // time of day has no per-frame lighting work.
        .add_systems(
            Update,
            (
                synchronize_authoritative_time.run_if(resource_exists::<LightingSettings>),
                resolve_lighting.run_if(lighting_inputs_changed),
            )
                .chain()
                .run_if(in_state(Screen::Gameplay))
                .in_set(LightingSystems::Resolve),
        )
        .add_systems(
            Update,
            (
                clear_environment_map_cache.run_if(resource_exists_and_changed::<LightingSettings>),
                apply_scene_lighting.run_if(resource_exists_and_changed::<ResolvedLighting>),
                apply_view_lighting
                    .run_if(view_lighting_inputs_changed)
                    .run_if(in_state(Screen::Gameplay)),
            )
                .chain()
                .in_set(LightingSystems::Apply),
        )
        .add_systems(
            Update,
            publish_exterior_illumination
                .run_if(resource_exists::<LightingSettings>)
                .run_if(in_state(Screen::Gameplay))
                .after(LightingSystems::Resolve)
                .in_set(PerceptionSystems::PublishAmbient),
        );

    #[cfg(feature = "dev-time-preview")]
    app.register_type::<PresentationTimeOverride>()
        .init_resource::<PresentationTimeOverride>()
        .add_systems(
            OnEnter(Screen::Gameplay),
            reset_presentation_time_override.before(clear_environment_map_cache),
        )
        .add_systems(
            OnExit(Screen::Gameplay),
            reset_presentation_time_override.before(clear_environment_map_cache),
        );
}

/// Marks the one directional light supplied by the active celestial body.
#[derive(Component, Reflect)]
#[reflect(Component)]
struct CelestialKeyLight;

/// Synchronizes the authoritative clock resource with the current profile kind.
///
/// `LightingSettings` is a hard `Res` here because the loading screen blocks gameplay
/// until the selected scenario's profile has loaded and passed cross-asset validation.
fn synchronize_authoritative_time(
    mut commands: Commands,
    settings: Res<LightingSettings>,
    time: Option<Res<TimeOfDay>>,
) {
    // TimeOfDay is derived session state, not an authored scenario override. Keep its
    // presence synchronized with hot-reloaded profile kind: static ignores/removes a
    // stale clock, while a newly cyclic profile publishes its validated default.
    // Cross-asset scenario validation still calls `LightingSettings::resolve` with the
    // authored override directly, so Static + authored time remains an error.
    match &settings.profile {
        LightingProfile::Static => {
            if time.is_some() {
                commands.remove_resource::<TimeOfDay>();
            }
        }
        LightingProfile::Cycle(cycle) if time.is_none() => {
            commands.insert_resource(TimeOfDay {
                hours: cycle.default_time_hours,
            });
        }
        LightingProfile::Cycle(_) => {}
    }
}

fn authoritative_requested_time(
    settings: &LightingSettings,
    time: Option<&TimeOfDay>,
) -> Option<f32> {
    match &settings.profile {
        LightingProfile::Static => None,
        LightingProfile::Cycle(cycle) => Some(
            time.map(|time| time.hours)
                .unwrap_or(cycle.default_time_hours),
        ),
    }
}

fn presentation_requested_time(
    settings: &LightingSettings,
    time: Option<&TimeOfDay>,
    #[cfg(feature = "dev-time-preview")] preview: Option<&PresentationTimeOverride>,
) -> Option<f32> {
    let authoritative = authoritative_requested_time(settings, time);
    match &settings.profile {
        LightingProfile::Static => None,
        LightingProfile::Cycle(_) => {
            #[cfg(feature = "dev-time-preview")]
            if let Some(hours) = preview.and_then(|preview| preview.hours) {
                return Some(hours);
            }
            authoritative
        }
    }
}

fn resolve_lighting(
    mut commands: Commands,
    settings: Res<LightingSettings>,
    time: Option<Res<TimeOfDay>>,
    #[cfg(feature = "dev-time-preview")] preview: Option<Res<PresentationTimeOverride>>,
    current: Option<ResMut<ResolvedLighting>>,
) {
    let requested_time = presentation_requested_time(
        &settings,
        time.as_deref(),
        #[cfg(feature = "dev-time-preview")]
        preview.as_deref(),
    );
    let resolved = match settings.resolve(requested_time) {
        Ok(resolved) => resolved,
        Err(reason) => {
            // The inspector can bypass asset deserialization and enter NaN. Retain the
            // last renderer-safe frame rather than sending invalid values to the GPU.
            error!("could not resolve lighting; retaining the previous frame: {reason}");
            return;
        }
    };

    match current {
        Some(mut current) if *current != resolved => *current = resolved,
        Some(_) => {}
        None => {
            commands.insert_resource(resolved);
        }
    }
}

fn lighting_inputs_changed(
    settings: Option<Res<LightingSettings>>,
    time: Option<Res<TimeOfDay>>,
    #[cfg(feature = "dev-time-preview")] preview: Option<Res<PresentationTimeOverride>>,
) -> bool {
    let Some(settings) = settings else {
        return false;
    };
    settings.is_changed() || time.is_some_and(|time| time.is_changed()) || {
        #[cfg(feature = "dev-time-preview")]
        {
            preview.is_some_and(|preview| preview.is_changed())
        }
        #[cfg(not(feature = "dev-time-preview"))]
        {
            false
        }
    }
}

fn view_lighting_inputs_changed(
    lighting: Option<Res<ResolvedLighting>>,
    settings: Option<Res<LightingSettings>>,
    added_cameras: Query<(), Added<Camera3d>>,
) -> bool {
    let Some(lighting) = lighting else {
        return false;
    };
    lighting.is_changed()
        || settings.is_some_and(|settings| settings.is_changed())
        || !added_cameras.is_empty()
}

fn spawn_celestial_key_light(
    mut commands: Commands,
    lighting: Res<ResolvedLighting>,
    existing: Query<Entity, With<CelestialKeyLight>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    commands.spawn((
        key_light(&lighting),
        key_light_transform(&lighting),
        Name::new("Celestial Key Light"),
        CelestialKeyLight,
    ));
}

fn despawn_celestial_key_light(
    mut commands: Commands,
    lights: Query<Entity, With<CelestialKeyLight>>,
) {
    for entity in &lights {
        commands.entity(entity).despawn();
    }
}

/// Publishes the renderer-independent ambient tier used by gameplay perception.
///
/// Static lighting is authored as a daytime-readable look and therefore remains
/// Bright. A cycle follows the body currently supplying its one key light: sunlight
/// is Bright and stylized moonlight is Dim. Physical intensity and exposure are
/// deliberately absent from this projection.
fn publish_exterior_illumination(
    mut commands: Commands,
    settings: Option<Res<LightingSettings>>,
    time: Option<Res<TimeOfDay>>,
    current: Option<ResMut<ExteriorIllumination>>,
) {
    let Some(settings) = settings else {
        let reason = "Gameplay lighting settings were unavailable for ambient perception.";
        error!("{reason}");
        commands.insert_resource(GameplaySetupFailure::new(reason));
        return;
    };
    let requested_time = authoritative_requested_time(&settings, time.as_deref());
    let authoritative = match settings.resolve(requested_time) {
        Ok(lighting) => lighting,
        Err(reason) => {
            error!("could not resolve authoritative ambient lighting: {reason}");
            return;
        }
    };
    let level = match authoritative.key_body {
        Some(CelestialBody::Moon) => IlluminationLevel::Dim,
        Some(CelestialBody::Sun) | None => IlluminationLevel::Bright,
    };
    let next = ExteriorIllumination::new(level);
    match current {
        Some(mut current) if *current != next => *current = next,
        Some(_) => {}
        None => {
            commands.insert_resource(next);
        }
    }
}

fn clear_exterior_illumination(mut commands: Commands) {
    commands.remove_resource::<ExteriorIllumination>();
}

fn clear_authoritative_time(mut commands: Commands) {
    commands.remove_resource::<TimeOfDay>();
}

fn key_light(lighting: &ResolvedLighting) -> DirectionalLight {
    DirectionalLight {
        illuminance: lighting.key_illuminance,
        color: to_color(lighting.key_color),
        shadow_maps_enabled: true,
        ..default()
    }
}

/// World direction from the scene towards the body currently supplying the key light.
fn key_body_direction(lighting: &ResolvedLighting) -> Vec3 {
    match lighting.key_body {
        Some(CelestialBody::Moon) => -lighting.sun_direction,
        Some(CelestialBody::Sun) | None => lighting.sun_direction,
    }
}

fn key_light_transform(lighting: &ResolvedLighting) -> Transform {
    let body_direction = key_body_direction(lighting);

    // A DirectionalLight shines along local -Z. Looking that axis away from the
    // visible body makes local +Z (`Transform::back`) point scene -> body exactly.
    // The resolved up vector also preserves the authored roll around that direction,
    // which keeps static and clear-noon shadow-map orientation pixel-compatible.
    Transform::default().looking_to(-body_direction, lighting.key_light_up)
}

/// Applies the key light and uniform fill from one coherent resolved frame.
fn apply_scene_lighting(
    mut commands: Commands,
    lighting: Res<ResolvedLighting>,
    mut ambient: ResMut<GlobalAmbientLight>,
    mut lights: Query<(&mut DirectionalLight, &mut Transform), With<CelestialKeyLight>>,
) {
    commands.insert_resource(ClearColor(to_color(lighting.sky_color)));
    ambient.brightness = lighting.ambient_brightness;
    ambient.color = to_color(lighting.ambient_color);

    for (mut light, mut transform) in &mut lights {
        *light = key_light(&lighting);
        *transform = key_light_transform(&lighting);
    }
}

/// Applies exposure, optional hemispherical fill, and optional distance haze.
///
/// The six-pixel hemispherical cubemap is shared by cameras and cached by its
/// resolved gradient. Zero intensity removes it entirely. Fog similarly affects PBR
/// geometry but not the custom sky material.
fn apply_view_lighting(
    mut commands: Commands,
    lighting: Res<ResolvedLighting>,
    mut images: ResMut<Assets<Image>>,
    mut cache: ResMut<EnvironmentMapCache>,
    cameras: Query<Entity, With<Camera3d>>,
) {
    let environment_map = (lighting.sky_light_intensity > 0.0).then(|| {
        let handle = cache.image_for(&mut images, &lighting);
        EnvironmentMapLight {
            diffuse_map: handle.clone(),
            specular_map: handle,
            intensity: lighting.sky_light_intensity,
            ..default()
        }
    });

    for entity in &cameras {
        if let Some(environment_map) = &environment_map {
            commands.entity(entity).insert(environment_map.clone());
        } else {
            commands.entity(entity).remove::<EnvironmentMapLight>();
        }

        apply_camera_exposure_and_fog(&mut commands, entity, &lighting);
    }
}

/// Restores the authoritative frame before gameplay's render-only clock is cleared.
///
/// Camera entities survive screen changes, while the ordinary view-lighting system
/// is gameplay-gated. Resolve from the authoritative clock directly on exit so
/// renderer resources, exposure, and fog cannot leak a preview onto the title screen.
fn restore_authoritative_lighting_on_exit(
    mut commands: Commands,
    settings: Option<Res<LightingSettings>>,
    time: Option<Res<TimeOfDay>>,
    current: Option<ResMut<ResolvedLighting>>,
    cameras: Query<Entity, With<Camera3d>>,
) {
    let Some(settings) = settings.as_deref() else {
        return;
    };
    let requested_time = authoritative_requested_time(settings, time.as_deref());
    let lighting = match settings.resolve(requested_time) {
        Ok(lighting) => lighting,
        Err(reason) => {
            error!("could not restore authoritative camera lighting on gameplay exit: {reason}");
            return;
        }
    };
    for entity in &cameras {
        apply_camera_exposure_and_fog(&mut commands, entity, &lighting);
    }
    match current {
        Some(mut current) if *current != lighting => *current = lighting,
        Some(_) => {}
        None => {
            commands.insert_resource(lighting);
        }
    }
}

fn apply_camera_exposure_and_fog(
    commands: &mut Commands,
    entity: Entity,
    lighting: &ResolvedLighting,
) {
    commands.entity(entity).insert(Exposure {
        ev100: lighting.exposure_ev100,
    });

    if lighting.fog_density > 0.0 {
        commands.entity(entity).insert(DistanceFog {
            color: to_color(lighting.fog_color),
            directional_light_color: to_color(lighting.fog_sun_color),
            directional_light_exponent: 8.0,
            falloff: FogFalloff::Exponential {
                density: lighting.fog_density,
            },
        });
    } else {
        commands.entity(entity).remove::<DistanceFog>();
    }
}

#[cfg(test)]
mod tests {
    use hex_assets::{CelestialCycleSettings, LightingKeyframe, LightingProfile};
    use hex_test_app::HeadlessAppBuilder;

    use super::*;

    fn flat_settings(profile: LightingProfile) -> LightingSettings {
        LightingSettings {
            profile,
            sun_illuminance: 10_000.0,
            sun_color: (1.0, 1.0, 1.0),
            sun_rotation: (11.4, 0.3, 0.0),
            ambient_brightness: 80.0,
            ambient_color: (1.0, 1.0, 1.0),
            sky_light_intensity: 0.0,
            ground_color: (0.32, 0.27, 0.21),
            sky_color: (0.55, 0.80, 0.95),
            zenith_color: (0.25, 0.50, 0.85),
            cloud_color: (0.97, 0.98, 1.0),
            cloud_coverage: 0.18,
            hex_cloud_scale: 16.0,
            cloud_softness: 0.1,
            cloud_roundness: 0.5,
            cloud_noise: 0.3,
            fog_color: (0.62, 0.72, 0.82),
            fog_sun_color: (1.0, 0.78, 0.50),
            fog_density: 0.0,
        }
    }

    fn keyframe(
        time_hours: f32,
        azimuth: f32,
        elevation: f32,
        active_body: CelestialBody,
        direct_illuminance: f32,
        exposure_ev100: f32,
    ) -> LightingKeyframe {
        LightingKeyframe {
            time_hours,
            sun_azimuth_degrees: azimuth,
            sun_elevation_degrees: elevation,
            active_body,
            direct_illuminance,
            direct_color: if active_body == CelestialBody::Sun {
                (1.0, 0.95, 0.85)
            } else {
                (0.55, 0.67, 1.0)
            },
            ambient_brightness: 60.0 + direct_illuminance / 500.0,
            ambient_color: (0.7, 0.75, 0.9),
            sky_light_intensity: 0.0,
            ground_color: (0.2, 0.2, 0.2),
            sky_color: (0.3, 0.5, 0.7),
            zenith_color: (0.1, 0.25, 0.5),
            cloud_color: (0.8, 0.85, 0.95),
            fog_color: (0.4, 0.5, 0.6),
            fog_sun_color: (1.0, 0.7, 0.4),
            fog_density: 0.0,
            exposure_ev100,
            sun_halo_strength: if active_body == CelestialBody::Sun {
                0.3
            } else {
                0.0
            },
            moon_halo_strength: if active_body == CelestialBody::Moon {
                0.25
            } else {
                0.0
            },
        }
    }

    fn cycle_settings() -> LightingSettings {
        let cycle = CelestialCycleSettings {
            default_time_hours: 12.0,
            sun_disc_color: (1.0, 0.95, 0.8),
            sun_angular_diameter_degrees: 1.2,
            sun_halo_width_degrees: 7.0,
            moon_disc_color: (0.85, 0.9, 1.0),
            moon_angular_diameter_degrees: 1.4,
            moon_halo_width_degrees: 8.0,
            lower_glow_angular_radius_degrees: 34.0,
            lower_glow_strength: 0.18,
            keyframes: vec![
                keyframe(0.0, 220.0, -60.0, CelestialBody::Moon, 800.0, 6.8),
                keyframe(6.0, 330.0, 0.0, CelestialBody::Sun, 500.0, 8.0),
                keyframe(12.0, 40.0, 60.0, CelestialBody::Sun, 10_000.0, 9.7),
                keyframe(18.0, 150.0, 0.0, CelestialBody::Moon, 500.0, 7.8),
            ],
        };
        let settings = flat_settings(LightingProfile::Cycle(cycle));
        settings
            .validate()
            .expect("the runtime test cycle should be valid");
        settings
    }

    #[cfg(feature = "dev-time-preview")]
    fn gradient_cycle_settings() -> LightingSettings {
        let mut settings = cycle_settings();
        let LightingProfile::Cycle(cycle) = &mut settings.profile else {
            unreachable!("cycle_settings always returns a cycle profile");
        };
        let colors = [
            ((0.1, 0.1, 0.2), (0.15, 0.1, 0.25)),
            ((0.9, 0.1, 0.2), (0.85, 0.1, 0.25)),
            ((0.9, 0.9, 0.2), (0.85, 0.9, 0.25)),
            ((0.1, 0.9, 0.2), (0.15, 0.9, 0.25)),
        ];
        for (frame, (sky_color, zenith_color)) in cycle.keyframes.iter_mut().zip(colors) {
            frame.sky_light_intensity = 1.0;
            frame.sky_color = sky_color;
            frame.zenith_color = zenith_color;
            frame.ground_color = (0.2, 0.15, 0.1);
        }
        settings
            .validate()
            .expect("the gradient cache test cycle should be valid");
        settings
    }

    fn enter(app: &mut App, screen: Screen) {
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(screen);
        app.update();
    }

    fn assert_approx_eq(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-5,
            "expected {expected}, got {actual}"
        );
    }

    #[cfg(feature = "dev-time-preview")]
    fn assert_rgb_finite(rgb: (f32, f32, f32)) {
        assert!(rgb.0.is_finite());
        assert!(rgb.1.is_finite());
        assert!(rgb.2.is_finite());
    }

    #[cfg(feature = "dev-time-preview")]
    fn assert_resolved_finite(lighting: &ResolvedLighting) {
        if let Some(time) = lighting.time_hours {
            assert!(time.is_finite());
        }
        for value in [
            lighting.key_illuminance,
            lighting.ambient_brightness,
            lighting.sky_light_intensity,
            lighting.cloud_coverage,
            lighting.hex_cloud_scale,
            lighting.cloud_softness,
            lighting.cloud_roundness,
            lighting.cloud_noise,
            lighting.fog_density,
            lighting.exposure_ev100,
            lighting.sun_angular_diameter_degrees,
            lighting.sun_halo_width_degrees,
            lighting.sun_halo_strength,
            lighting.moon_angular_diameter_degrees,
            lighting.moon_halo_width_degrees,
            lighting.moon_halo_strength,
            lighting.lower_glow_angular_radius_degrees,
            lighting.lower_glow_strength,
        ] {
            assert!(value.is_finite());
        }
        for rgb in [
            lighting.key_color,
            lighting.ambient_color,
            lighting.ground_color,
            lighting.sky_color,
            lighting.zenith_color,
            lighting.cloud_color,
            lighting.fog_color,
            lighting.fog_sun_color,
            lighting.sun_disc_color,
            lighting.moon_disc_color,
            lighting.lower_glow_color,
        ] {
            assert_rgb_finite(rgb);
        }
        assert!(lighting.sun_direction.is_finite());
        assert!(lighting.key_light_up.is_finite());
        assert!(lighting.lower_glow_direction.is_finite());
    }

    #[cfg(feature = "dev-time-preview")]
    fn assert_sky_params_equal(
        actual: &crate::sky_material::SkyParams,
        expected: &crate::sky_material::SkyParams,
    ) {
        assert_eq!(actual.horizon_color, expected.horizon_color);
        assert_approx_eq(actual.cloud_coverage, expected.cloud_coverage);
        assert_eq!(actual.zenith_color, expected.zenith_color);
        assert_approx_eq(actual.hex_scale, expected.hex_scale);
        assert_eq!(actual.cloud_color, expected.cloud_color);
        assert_approx_eq(actual.cloud_softness, expected.cloud_softness);
        assert_approx_eq(actual.cloud_roundness, expected.cloud_roundness);
        assert_approx_eq(actual.cloud_noise, expected.cloud_noise);
        assert_eq!(actual.sun_direction, expected.sun_direction);
        assert_approx_eq(
            actual.celestial_bodies_enabled,
            expected.celestial_bodies_enabled,
        );
        assert_eq!(actual.sun_disc_color, expected.sun_disc_color);
        assert_approx_eq(
            actual.sun_angular_radius_radians,
            expected.sun_angular_radius_radians,
        );
        assert_eq!(actual.moon_direction, expected.moon_direction);
        assert_approx_eq(
            actual.moon_angular_radius_radians,
            expected.moon_angular_radius_radians,
        );
        assert_eq!(actual.moon_disc_color, expected.moon_disc_color);
        assert_approx_eq(
            actual.sun_halo_width_radians,
            expected.sun_halo_width_radians,
        );
        assert_eq!(actual.lower_glow_direction, expected.lower_glow_direction);
        assert_approx_eq(
            actual.moon_halo_width_radians,
            expected.moon_halo_width_radians,
        );
        assert_eq!(actual.lower_glow_color, expected.lower_glow_color);
        assert_approx_eq(actual.sun_halo_strength, expected.sun_halo_strength);
        assert_approx_eq(actual.moon_halo_strength, expected.moon_halo_strength);
        assert_approx_eq(
            actual.lower_glow_angular_radius_radians,
            expected.lower_glow_angular_radius_radians,
        );
        assert_approx_eq(actual.lower_glow_strength, expected.lower_glow_strength);
        assert_approx_eq(actual._padding, expected._padding);
    }

    #[cfg(feature = "dev-time-preview")]
    fn assert_view_lighting(
        world: &World,
        camera: Entity,
        expected: &ResolvedLighting,
        expected_environment: Option<&Handle<Image>>,
    ) -> Option<Handle<Image>> {
        let camera = world.entity(camera);
        let exposure = camera.get::<Exposure>();
        assert!(exposure.is_some(), "the camera should receive exposure");
        if let Some(exposure) = exposure {
            assert_approx_eq(exposure.ev100, expected.exposure_ev100);
        }

        let environment = camera.get::<EnvironmentMapLight>();
        if expected.sky_light_intensity > 0.0 {
            assert!(
                environment.is_some(),
                "positive sky fill should install a cubemap"
            );
            if let Some(environment) = environment {
                assert_eq!(environment.diffuse_map, environment.specular_map);
                assert_approx_eq(environment.intensity, expected.sky_light_intensity);
                assert_eq!(environment.rotation, Quat::IDENTITY);
                assert!(environment.affects_lightmapped_mesh_diffuse);
                if let Some(expected_environment) = expected_environment {
                    assert_eq!(&environment.diffuse_map, expected_environment);
                }
            }
        } else {
            assert!(environment.is_none());
        }

        let fog = camera.get::<DistanceFog>();
        if expected.fog_density > 0.0 {
            assert!(
                fog.is_some(),
                "positive distance haze should install camera fog"
            );
            if let Some(fog) = fog {
                assert_eq!(fog.color, to_color(expected.fog_color));
                assert_eq!(
                    fog.directional_light_color,
                    to_color(expected.fog_sun_color)
                );
                assert_approx_eq(fog.directional_light_exponent, 8.0);
                let FogFalloff::Exponential { density } = &fog.falloff else {
                    unreachable!("world lighting should install exponential distance haze");
                };
                assert_approx_eq(*density, expected.fog_density);
            }
        } else {
            assert!(fog.is_none());
        }

        environment.map(|environment| environment.diffuse_map.clone())
    }

    #[cfg(feature = "dev-time-preview")]
    fn assert_applied_frame(
        app: &mut App,
        camera: Entity,
        sky_material: &Handle<crate::sky_material::SkyMaterial>,
        expected: &ResolvedLighting,
        expected_environment: Option<&Handle<Image>>,
    ) -> Option<Handle<Image>> {
        assert_eq!(*app.world().resource::<ResolvedLighting>(), *expected);
        assert_eq!(
            app.world().resource::<ClearColor>().0,
            to_color(expected.sky_color)
        );
        let ambient = app.world().resource::<GlobalAmbientLight>();
        assert_eq!(ambient.color, to_color(expected.ambient_color));
        assert_approx_eq(ambient.brightness, expected.ambient_brightness);
        assert!(ambient.affects_lightmapped_meshes);

        {
            let world = app.world_mut();
            let mut query =
                world.query_filtered::<(&DirectionalLight, &Transform), With<CelestialKeyLight>>();
            let (light, transform) = query
                .single(world)
                .expect("the applied frame must retain exactly one celestial key light");
            assert_eq!(light.color, to_color(expected.key_color));
            assert_approx_eq(light.illuminance, expected.key_illuminance);
            assert!(light.shadow_maps_enabled);
            assert!(!light.contact_shadows_enabled);
            assert!(light.affects_lightmapped_mesh_diffuse);
            assert_approx_eq(
                light.shadow_depth_bias,
                DirectionalLight::DEFAULT_SHADOW_DEPTH_BIAS,
            );
            assert_approx_eq(
                light.shadow_normal_bias,
                DirectionalLight::DEFAULT_SHADOW_NORMAL_BIAS,
            );
            assert_eq!(*transform, key_light_transform(expected));
        }

        let environment = assert_view_lighting(app.world(), camera, expected, expected_environment);
        let materials = app
            .world()
            .resource::<Assets<crate::sky_material::SkyMaterial>>();
        let sky = materials.get(sky_material);
        assert!(sky.is_some(), "the sky material should remain present");
        if let Some(sky) = sky {
            let expected_sky = crate::camera::sky_params(expected);
            assert_sky_params_equal(&sky.params, &expected_sky);
        }
        environment
    }

    fn runtime_app(settings: LightingSettings, time: Option<f32>) -> (App, Entity) {
        let mut builder = HeadlessAppBuilder::new()
            .with_minimal_plugins()
            .with_state_plugin();
        builder.app_mut().init_state::<Screen>();
        builder.app_mut().insert_resource(settings);
        builder
            .app_mut()
            .insert_resource(GlobalAmbientLight::default());
        builder
            .app_mut()
            .insert_resource(Assets::<Image>::default());
        if let Some(hours) = time {
            builder.app_mut().insert_resource(TimeOfDay { hours });
        }
        builder.app_mut().add_plugins(super::plugin);
        let camera = builder
            .app_mut()
            .world_mut()
            .spawn(Camera3d::default())
            .id();
        (builder.build(), camera)
    }

    fn key_light_snapshot(app: &mut App) -> (usize, f32, Vec3, Vec3, bool) {
        let world = app.world_mut();
        let mut query =
            world.query_filtered::<(&DirectionalLight, &Transform), With<CelestialKeyLight>>();
        let entries: Vec<_> = query
            .iter(world)
            .map(|(light, transform)| {
                let body_direction: Vec3 = transform.back().into();
                let ray_direction: Vec3 = transform.forward().into();
                (
                    light.illuminance,
                    body_direction,
                    ray_direction,
                    light.shadow_maps_enabled,
                )
            })
            .collect();
        let Some((illuminance, body_direction, ray_direction, shadows)) = entries.first().copied()
        else {
            return (0, 0.0, Vec3::ZERO, Vec3::ZERO, false);
        };
        (
            entries.len(),
            illuminance,
            body_direction,
            ray_direction,
            shadows,
        )
    }

    fn shadow_casting_directional_light_count(app: &mut App) -> usize {
        let world = app.world_mut();
        let mut query = world.query::<&DirectionalLight>();
        query
            .iter(world)
            .filter(|light| light.shadow_maps_enabled)
            .count()
    }

    #[test]
    fn key_light_ray_is_inverse_of_the_active_visible_body() {
        for (hour, body) in [(12.0, CelestialBody::Sun), (0.0, CelestialBody::Moon)] {
            let lighting = cycle_settings()
                .resolve(Some(hour))
                .expect("the cycle should resolve");
            assert_eq!(lighting.key_body, Some(body));

            let transform = key_light_transform(&lighting);
            let body_direction: Vec3 = transform.back().into();
            let ray_direction: Vec3 = transform.forward().into();
            let expected = if body == CelestialBody::Sun {
                lighting.sun_direction
            } else {
                -lighting.sun_direction
            };

            assert!(body_direction.distance(expected) < 1e-5);
            assert!(ray_direction.distance(-expected) < 1e-5);
            assert!(body_direction.distance(-ray_direction) < 1e-6);
        }
    }

    #[test]
    fn static_key_light_retains_the_complete_legacy_orientation() {
        let settings = flat_settings(LightingProfile::Static);
        let resolved = settings
            .resolve(None)
            .expect("static lighting should resolve");
        let transform = key_light_transform(&resolved);
        let (x, y, z) = settings.sun_rotation;
        let legacy_rotation = Quat::from_euler(EulerRot::XYZ, x, y, z);

        assert!(
            transform.rotation.dot(legacy_rotation).abs() >= 1.0 - 1.0e-6,
            "runtime application must preserve the legacy directional-light roll"
        );
        let applied_up: Vec3 = transform.up().into();
        assert!(applied_up.distance(resolved.key_light_up) < 1.0e-6);
    }

    #[test]
    fn sky_material_keeps_sun_and_moon_opposite_and_static_bodies_hidden() {
        let cycle = cycle_settings()
            .resolve(Some(18.0))
            .expect("the cycle should resolve");
        let cycle_params = crate::camera::sky_params(&cycle);
        assert!(cycle_params.sun_direction.distance(cycle.sun_direction) < 1e-6);
        assert!(cycle_params.moon_direction.distance(-cycle.sun_direction) < 1e-6);
        assert_approx_eq(cycle_params.celestial_bodies_enabled, 1.0);
        assert!(
            (cycle_params.sun_angular_radius_radians
                - 0.5 * cycle.sun_angular_diameter_degrees.to_radians())
            .abs()
                < f32::EPSILON
        );

        let static_lighting = flat_settings(LightingProfile::Static)
            .resolve(None)
            .expect("static lighting should resolve");
        let static_params = crate::camera::sky_params(&static_lighting);
        assert_approx_eq(static_params.celestial_bodies_enabled, 0.0);
        assert_approx_eq(static_params.sun_halo_strength, 0.0);
        assert_approx_eq(static_params.moon_halo_strength, 0.0);
    }

    #[test]
    fn time_changes_update_one_shadow_caster_and_camera_exposure() {
        let (mut app, camera) = runtime_app(cycle_settings(), Some(12.0));
        enter(&mut app, Screen::Gameplay);

        let (count, illuminance, body_direction, ray_direction, shadows) =
            key_light_snapshot(&mut app);
        assert_eq!(count, 1);
        assert_eq!(shadow_casting_directional_light_count(&mut app), 1);
        assert_approx_eq(illuminance, 10_000.0);
        assert!(shadows);
        assert!(body_direction.distance(-ray_direction) < 1e-6);
        assert_approx_eq(
            app.world()
                .entity(camera)
                .get::<Exposure>()
                .expect("the camera should receive exposure")
                .ev100,
            9.7,
        );

        app.world_mut().resource_mut::<TimeOfDay>().hours = 0.0;
        app.update();

        let resolved = app.world().resource::<ResolvedLighting>();
        assert_eq!(resolved.time_hours, Some(0.0));
        assert_eq!(resolved.key_body, Some(CelestialBody::Moon));
        let expected_moon = -resolved.sun_direction;
        let (count, illuminance, body_direction, ray_direction, shadows) =
            key_light_snapshot(&mut app);
        assert_eq!(count, 1);
        assert_eq!(shadow_casting_directional_light_count(&mut app), 1);
        assert_approx_eq(illuminance, 800.0);
        assert!(shadows);
        assert!(body_direction.distance(expected_moon) < 1e-5);
        assert!(ray_direction.distance(-expected_moon) < 1e-5);
        assert_approx_eq(
            app.world()
                .entity(camera)
                .get::<Exposure>()
                .expect("the camera exposure should update")
                .ev100,
            6.8,
        );
    }

    #[cfg(feature = "dev-time-preview")]
    #[test]
    fn presentation_override_changes_rendering_without_changing_gameplay_illumination() {
        let mut settings = cycle_settings();
        let LightingProfile::Cycle(cycle) = &mut settings.profile else {
            unreachable!("cycle_settings always returns a cycle profile");
        };
        let midnight = cycle
            .keyframes
            .first_mut()
            .expect("the cycle should retain midnight");
        midnight.ambient_brightness = 37.0;
        midnight.sky_light_intensity = 21.0;
        midnight.sky_color = (0.07, 0.12, 0.25);
        midnight.zenith_color = (0.01, 0.03, 0.11);
        midnight.cloud_color = (0.22, 0.30, 0.48);
        midnight.fog_density = 0.0007;
        settings
            .validate()
            .expect("the presentation-change fixture should remain valid");
        let authoritative = settings
            .resolve(Some(12.0))
            .expect("the authoritative hour should resolve");
        let (mut app, camera) = runtime_app(settings, Some(12.0));
        app.insert_resource(Assets::<crate::sky_material::SkyMaterial>::default());
        let material = app
            .world_mut()
            .resource_mut::<Assets<crate::sky_material::SkyMaterial>>()
            .add(crate::sky_material::SkyMaterial {
                params: crate::camera::sky_params(&authoritative),
            });
        app.world_mut()
            .spawn((MeshMaterial3d(material.clone()), crate::camera::SkyDome));
        app.add_systems(
            Update,
            crate::camera::apply_sky_material
                .run_if(resource_exists_and_changed::<ResolvedLighting>)
                .in_set(LightingSystems::Apply),
        );
        enter(&mut app, Screen::Gameplay);

        app.world_mut()
            .resource_mut::<PresentationTimeOverride>()
            .hours = Some(0.0);
        app.update();

        let rendered = app.world().resource::<ResolvedLighting>().clone();
        assert_eq!(rendered.time_hours, Some(0.0));
        assert_eq!(rendered.key_body, Some(CelestialBody::Moon));
        assert!((rendered.key_illuminance - authoritative.key_illuminance).abs() > f32::EPSILON);
        assert!(
            (rendered.ambient_brightness - authoritative.ambient_brightness).abs() > f32::EPSILON
        );
        assert!(
            (rendered.sky_light_intensity - authoritative.sky_light_intensity).abs() > f32::EPSILON
        );
        assert!((rendered.sky_color.0 - authoritative.sky_color.0).abs() > f32::EPSILON);
        assert!((rendered.zenith_color.1 - authoritative.zenith_color.1).abs() > f32::EPSILON);
        assert!((rendered.cloud_color.2 - authoritative.cloud_color.2).abs() > f32::EPSILON);
        assert_approx_eq(rendered.cloud_coverage, authoritative.cloud_coverage);
        assert!((rendered.fog_density - authoritative.fog_density).abs() > f32::EPSILON);
        assert!((rendered.exposure_ev100 - authoritative.exposure_ev100).abs() > f32::EPSILON);
        assert_approx_eq(
            app.world().resource::<GlobalAmbientLight>().brightness,
            rendered.ambient_brightness,
        );
        assert_approx_eq(
            app.world()
                .entity(camera)
                .get::<Exposure>()
                .expect("preview should update camera exposure")
                .ev100,
            rendered.exposure_ev100,
        );
        assert!(app.world().entity(camera).contains::<EnvironmentMapLight>());
        assert!(app.world().entity(camera).contains::<DistanceFog>());
        let sky = app
            .world()
            .resource::<Assets<crate::sky_material::SkyMaterial>>()
            .get(&material)
            .expect("preview should update the sky material");
        assert_approx_eq(sky.params.cloud_coverage, rendered.cloud_coverage);
        assert_approx_eq(app.world().resource::<TimeOfDay>().hours, 12.0);
        assert_eq!(
            app.world().resource::<ExteriorIllumination>().level,
            IlluminationLevel::Bright
        );
    }

    #[cfg(feature = "dev-time-preview")]
    #[test]
    fn static_profile_ignores_the_presentation_override() {
        let settings = flat_settings(LightingProfile::Static);
        let expected = settings
            .resolve(None)
            .expect("static lighting should resolve");
        let (mut app, _) = runtime_app(settings, None);
        enter(&mut app, Screen::Gameplay);

        app.world_mut()
            .resource_mut::<PresentationTimeOverride>()
            .hours = Some(0.0);
        app.update();

        assert!(!app.world().contains_resource::<TimeOfDay>());
        assert_eq!(*app.world().resource::<ResolvedLighting>(), expected);
        assert_eq!(
            app.world().resource::<ExteriorIllumination>().level,
            IlluminationLevel::Bright
        );
    }

    #[cfg(feature = "dev-time-preview")]
    #[test]
    fn preview_resets_on_entry_and_exit_and_restores_authoritative_parity() {
        let mut settings = cycle_settings();
        let LightingProfile::Cycle(cycle) = &mut settings.profile else {
            unreachable!("cycle_settings always returns a cycle profile");
        };
        cycle
            .keyframes
            .first_mut()
            .expect("the cycle should retain midnight")
            .fog_density = 0.0007;
        settings
            .validate()
            .expect("the exit-restoration fixture should remain valid");
        let expected = settings
            .resolve(Some(12.0))
            .expect("the authoritative hour should resolve");
        let preview = settings
            .resolve(Some(0.0))
            .expect("the preview hour should resolve");
        let (mut app, camera) = runtime_app(settings, Some(12.0));
        app.world_mut()
            .resource_mut::<PresentationTimeOverride>()
            .hours = Some(0.0);

        enter(&mut app, Screen::Gameplay);

        assert_eq!(
            app.world().resource::<PresentationTimeOverride>().hours,
            None
        );
        assert_eq!(*app.world().resource::<ResolvedLighting>(), expected);

        app.world_mut()
            .resource_mut::<PresentationTimeOverride>()
            .hours = Some(0.0);
        app.update();
        assert_eq!(
            app.world().resource::<ResolvedLighting>().time_hours,
            Some(0.0)
        );
        assert_approx_eq(
            app.world()
                .entity(camera)
                .get::<Exposure>()
                .expect("preview should install camera exposure")
                .ev100,
            preview.exposure_ev100,
        );
        assert!(
            app.world().entity(camera).contains::<DistanceFog>(),
            "the preview fixture should install haze before exit"
        );

        enter(&mut app, Screen::Title);
        assert_eq!(
            app.world().resource::<PresentationTimeOverride>().hours,
            None
        );
        assert!(
            !app.world().contains_resource::<TimeOfDay>(),
            "authoritative scenario time must not survive gameplay exit"
        );
        assert_eq!(*app.world().resource::<ResolvedLighting>(), expected);
        assert_eq!(
            app.world().resource::<ClearColor>().0,
            to_color(expected.sky_color)
        );
        assert_approx_eq(
            app.world()
                .entity(camera)
                .get::<Exposure>()
                .expect("persistent camera exposure must be authoritative after exit")
                .ev100,
            expected.exposure_ev100,
        );
        assert!(
            !app.world().entity(camera).contains::<DistanceFog>(),
            "preview-only haze must not leak onto the title screen"
        );

        enter(&mut app, Screen::Gameplay);
        assert_eq!(*app.world().resource::<ResolvedLighting>(), expected);
    }

    #[cfg(feature = "dev-time-preview")]
    #[test]
    fn reset_restores_the_complete_applied_authoritative_frame() {
        let mut settings = gradient_cycle_settings();
        let LightingProfile::Cycle(cycle) = &mut settings.profile else {
            unreachable!("gradient_cycle_settings always returns a cycle profile");
        };
        cycle
            .keyframes
            .first_mut()
            .expect("the cycle must retain its midnight keyframe")
            .fog_density = 0.0007;
        cycle
            .keyframes
            .get_mut(2)
            .expect("the cycle must retain its noon keyframe")
            .fog_density = 0.0003;
        settings
            .validate()
            .expect("the applied-frame fixture should remain valid");
        let authoritative = settings
            .resolve(Some(12.0))
            .expect("the authoritative frame should resolve");
        let preview = settings
            .resolve(Some(0.0))
            .expect("the preview frame should resolve");
        assert_ne!(preview, authoritative);

        let (mut app, camera) = runtime_app(settings, Some(12.0));
        app.insert_resource(Assets::<crate::sky_material::SkyMaterial>::default());
        let material = app
            .world_mut()
            .resource_mut::<Assets<crate::sky_material::SkyMaterial>>()
            .add(crate::sky_material::SkyMaterial {
                params: crate::camera::sky_params(&authoritative),
            });
        app.world_mut()
            .spawn((MeshMaterial3d(material.clone()), crate::camera::SkyDome));
        app.add_systems(
            Update,
            crate::camera::apply_sky_material
                .run_if(resource_exists_and_changed::<ResolvedLighting>)
                .in_set(LightingSystems::Apply),
        );
        enter(&mut app, Screen::Gameplay);

        let authoritative_environment =
            assert_applied_frame(&mut app, camera, &material, &authoritative, None)
                .expect("the fixture should install authoritative sky fill");

        app.world_mut()
            .resource_mut::<PresentationTimeOverride>()
            .hours = Some(0.0);
        app.update();
        let preview_environment = assert_applied_frame(&mut app, camera, &material, &preview, None)
            .expect("the fixture should install preview sky fill");
        assert_ne!(preview_environment, authoritative_environment);
        assert_approx_eq(app.world().resource::<TimeOfDay>().hours, 12.0);
        assert_eq!(
            app.world().resource::<ExteriorIllumination>().level,
            IlluminationLevel::Bright
        );

        app.world_mut()
            .resource_mut::<PresentationTimeOverride>()
            .clear();
        app.update();

        let reset_environment = assert_applied_frame(
            &mut app,
            camera,
            &material,
            &authoritative,
            Some(&authoritative_environment),
        )
        .expect("reset should reinstall authoritative sky fill");
        assert_eq!(reset_environment, authoritative_environment);
        assert_eq!(
            app.world().resource::<PresentationTimeOverride>().hours,
            None
        );
        assert_approx_eq(app.world().resource::<TimeOfDay>().hours, 12.0);
        assert_eq!(
            app.world().resource::<ExteriorIllumination>().level,
            IlluminationLevel::Bright
        );
    }

    #[cfg(feature = "dev-time-preview")]
    #[test]
    fn preview_scrubs_all_quarter_hours_wraps_and_stays_finite() {
        let (mut app, _) = runtime_app(cycle_settings(), Some(12.0));
        enter(&mut app, Screen::Gameplay);

        for step in 0_u8..=96 {
            let requested = f32::from(step) * 0.25;
            app.world_mut()
                .resource_mut::<PresentationTimeOverride>()
                .hours = Some(requested);
            app.update();

            let lighting = app.world().resource::<ResolvedLighting>();
            assert_eq!(lighting.time_hours, Some(requested.rem_euclid(24.0)));
            assert_resolved_finite(lighting);
            assert_approx_eq(app.world().resource::<TimeOfDay>().hours, 12.0);
        }
    }

    #[cfg(feature = "dev-time-preview")]
    #[test]
    fn camera_added_during_gameplay_receives_current_view_lighting_without_reallocation() {
        let mut settings = gradient_cycle_settings();
        let LightingProfile::Cycle(cycle) = &mut settings.profile else {
            unreachable!("gradient_cycle_settings always returns a cycle profile");
        };
        cycle
            .keyframes
            .get_mut(2)
            .expect("the cycle must retain its noon keyframe")
            .fog_density = 0.0003;
        settings
            .validate()
            .expect("the late-camera fixture should remain valid");
        let expected = settings
            .resolve(Some(12.0))
            .expect("the authoritative frame should resolve");
        let (mut app, original_camera) = runtime_app(settings, Some(12.0));
        enter(&mut app, Screen::Gameplay);

        let original_environment =
            assert_view_lighting(app.world(), original_camera, &expected, None)
                .expect("the original camera should have sky fill");
        let allocations = app.world().resource::<EnvironmentMapCache>().allocations;
        let late_camera = app.world_mut().spawn(Camera3d::default()).id();
        assert!(app.world().entity(late_camera).get::<Exposure>().is_none());

        app.update();

        let late_environment = assert_view_lighting(
            app.world(),
            late_camera,
            &expected,
            Some(&original_environment),
        )
        .expect("the late camera should share the current sky fill");
        assert_eq!(late_environment, original_environment);
        assert_eq!(
            app.world().resource::<EnvironmentMapCache>().allocations,
            allocations,
            "applying the current gradient to a new camera must reuse its cached image"
        );
    }

    #[cfg(feature = "dev-time-preview")]
    #[test]
    fn second_full_scrub_reuses_all_environment_images_and_clear_releases_them() {
        let (mut app, camera) = runtime_app(gradient_cycle_settings(), Some(12.0));
        let second_camera = app.world_mut().spawn(Camera3d::default()).id();
        enter(&mut app, Screen::Gameplay);
        let baseline_images = app.world().resource::<Assets<Image>>().len();
        let initial_allocations = app.world().resource::<EnvironmentMapCache>().allocations;
        let mut first_handles = Vec::with_capacity(ENVIRONMENT_MAP_CACHE_CAPACITY);

        for step in 0_u8..96 {
            app.world_mut()
                .resource_mut::<PresentationTimeOverride>()
                .hours = Some(f32::from(step) * 0.25);
            app.update();
            let environment = app
                .world()
                .entity(camera)
                .get::<EnvironmentMapLight>()
                .expect("positive sky fill should install an environment map");
            assert_eq!(environment.diffuse_map, environment.specular_map);
            let second_environment = app
                .world()
                .entity(second_camera)
                .get::<EnvironmentMapLight>()
                .expect("cameras should share the cached environment map");
            assert_eq!(environment.diffuse_map, second_environment.diffuse_map);
            first_handles.push(environment.diffuse_map.id());
        }

        let first_allocations = app.world().resource::<EnvironmentMapCache>().allocations;
        let first_image_count = app.world().resource::<Assets<Image>>().len();
        assert_eq!(
            app.world().resource::<EnvironmentMapCache>().entries.len(),
            ENVIRONMENT_MAP_CACHE_CAPACITY
        );
        assert_eq!(
            first_allocations - initial_allocations,
            ENVIRONMENT_MAP_CACHE_CAPACITY - 1
        );
        assert_eq!(
            first_image_count - baseline_images,
            ENVIRONMENT_MAP_CACHE_CAPACITY - 1
        );

        for (step, expected_handle) in (0_u8..96).zip(&first_handles) {
            app.world_mut()
                .resource_mut::<PresentationTimeOverride>()
                .hours = Some(f32::from(step) * 0.25);
            app.update();
            let environment = app
                .world()
                .entity(camera)
                .get::<EnvironmentMapLight>()
                .expect("the cached environment map should stay installed");
            assert_eq!(environment.diffuse_map.id(), *expected_handle);
        }

        assert_eq!(
            app.world().resource::<EnvironmentMapCache>().allocations,
            first_allocations
        );
        assert_eq!(
            app.world().resource::<Assets<Image>>().len(),
            first_image_count
        );

        app.world_mut()
            .resource_mut::<PresentationTimeOverride>()
            .hours = Some(0.0);
        app.update();
        app.world_mut()
            .resource_mut::<PresentationTimeOverride>()
            .hours = Some(0.125);
        app.update();

        let bounded_allocations = app.world().resource::<EnvironmentMapCache>().allocations;
        assert_eq!(bounded_allocations, first_allocations + 1);
        assert_eq!(
            app.world().resource::<EnvironmentMapCache>().entries.len(),
            ENVIRONMENT_MAP_CACHE_CAPACITY
        );
        assert_eq!(
            app.world().resource::<Assets<Image>>().len(),
            first_image_count
        );
        let retained_handle = first_handles
            .first()
            .copied()
            .expect("the first scrub should record every handle");
        let evicted_handle = first_handles
            .get(1)
            .copied()
            .expect("the first scrub should record every handle");
        assert!(app
            .world()
            .resource::<Assets<Image>>()
            .get(retained_handle)
            .is_some());
        assert!(app
            .world()
            .resource::<Assets<Image>>()
            .get(evicted_handle)
            .is_none());

        let previous_environment = app
            .world()
            .entity(camera)
            .get::<EnvironmentMapLight>()
            .expect("the last scrub step should have an environment map")
            .diffuse_map
            .clone();
        app.world_mut()
            .resource_mut::<LightingSettings>()
            .cloud_coverage = 0.2;
        app.update();

        assert_eq!(
            app.world().resource::<EnvironmentMapCache>().entries.len(),
            1
        );
        assert_eq!(app.world().resource::<Assets<Image>>().len(), 1);
        assert_eq!(
            app.world().resource::<EnvironmentMapCache>().allocations,
            bounded_allocations + 1
        );
        assert_ne!(
            app.world()
                .entity(camera)
                .get::<EnvironmentMapLight>()
                .expect("hot reload should reinstall an environment map")
                .diffuse_map,
            previous_environment
        );

        clear_environment_map_cache(app.world_mut());

        assert!(app
            .world()
            .resource::<EnvironmentMapCache>()
            .entries
            .is_empty());
        assert_eq!(app.world().resource::<Assets<Image>>().len(), 0);
        assert!(app
            .world()
            .entity(camera)
            .get::<EnvironmentMapLight>()
            .is_none());
        assert!(app
            .world()
            .entity(second_camera)
            .get::<EnvironmentMapLight>()
            .is_none());

        app.world_mut()
            .resource_mut::<PresentationTimeOverride>()
            .hours = Some(0.0);
        app.update();
        assert_eq!(app.world().resource::<Assets<Image>>().len(), 1);

        enter(&mut app, Screen::Title);
        assert!(app
            .world()
            .resource::<EnvironmentMapCache>()
            .entries
            .is_empty());
        assert_eq!(app.world().resource::<Assets<Image>>().len(), 0);
        assert!(app
            .world()
            .entity(camera)
            .get::<EnvironmentMapLight>()
            .is_none());
        assert!(app
            .world()
            .entity(second_camera)
            .get::<EnvironmentMapLight>()
            .is_none());
    }

    #[test]
    fn one_update_keeps_key_light_and_sky_material_on_the_same_hour() {
        let settings = cycle_settings();
        let initial = settings
            .resolve(Some(12.0))
            .expect("the initial hour should resolve");
        let (mut app, _) = runtime_app(settings, Some(12.0));
        app.insert_resource(Assets::<crate::sky_material::SkyMaterial>::default());
        let material = app
            .world_mut()
            .resource_mut::<Assets<crate::sky_material::SkyMaterial>>()
            .add(crate::sky_material::SkyMaterial {
                params: crate::camera::sky_params(&initial),
            });
        app.world_mut()
            .spawn((MeshMaterial3d(material.clone()), crate::camera::SkyDome));
        app.add_systems(
            Update,
            crate::camera::apply_sky_material
                .run_if(resource_exists_and_changed::<ResolvedLighting>)
                .in_set(LightingSystems::Apply),
        );
        enter(&mut app, Screen::Gameplay);

        let before_direction = app
            .world()
            .resource::<Assets<crate::sky_material::SkyMaterial>>()
            .get(&material)
            .expect("the test sky material should exist")
            .params
            .sun_direction;

        app.world_mut().resource_mut::<TimeOfDay>().hours = 0.0;
        app.update();

        let resolved = app.world().resource::<ResolvedLighting>().clone();
        let params = app
            .world()
            .resource::<Assets<crate::sky_material::SkyMaterial>>()
            .get(&material)
            .expect("the test sky material should still exist")
            .params
            .clone();
        let (_, illuminance, body_direction, ray_direction, _) = key_light_snapshot(&mut app);

        assert_eq!(resolved.time_hours, Some(0.0));
        assert_eq!(resolved.key_body, Some(CelestialBody::Moon));
        assert!(before_direction.distance(resolved.sun_direction) > 0.1);
        assert!(params.sun_direction.distance(resolved.sun_direction) < 1e-6);
        assert!(params.moon_direction.distance(-resolved.sun_direction) < 1e-6);
        assert_approx_eq(illuminance, resolved.key_illuminance);
        assert!(body_direction.distance(-resolved.sun_direction) < 1e-5);
        assert!(ray_direction.distance(resolved.sun_direction) < 1e-5);
    }

    #[test]
    fn invalid_scrubber_values_retain_the_previous_render_frame() {
        let (mut app, _) = runtime_app(cycle_settings(), Some(12.0));
        enter(&mut app, Screen::Gameplay);
        let previous = app.world().resource::<ResolvedLighting>().clone();
        let previous_light = key_light_snapshot(&mut app);

        app.world_mut().resource_mut::<TimeOfDay>().hours = f32::NAN;
        app.update();

        assert_eq!(*app.world().resource::<ResolvedLighting>(), previous);
        let current_light = key_light_snapshot(&mut app);
        assert_eq!(current_light.0, previous_light.0);
        assert_approx_eq(current_light.1, previous_light.1);
        assert!(current_light.2.distance(previous_light.2) < f32::EPSILON);
    }

    #[test]
    fn hot_reload_from_cycle_to_static_removes_and_ignores_session_time() {
        let (mut app, camera) = runtime_app(cycle_settings(), Some(0.0));
        enter(&mut app, Screen::Gameplay);
        assert!(app.world().contains_resource::<TimeOfDay>());
        assert_eq!(
            app.world().resource::<ResolvedLighting>().key_body,
            Some(CelestialBody::Moon)
        );

        let static_settings = flat_settings(LightingProfile::Static);
        let expected = static_settings
            .resolve(None)
            .expect("static settings should resolve");
        *app.world_mut().resource_mut::<LightingSettings>() = static_settings;
        app.update();

        assert!(!app.world().contains_resource::<TimeOfDay>());
        assert_eq!(*app.world().resource::<ResolvedLighting>(), expected);
        let (_, illuminance, body_direction, _, _) = key_light_snapshot(&mut app);
        assert_approx_eq(illuminance, expected.key_illuminance);
        assert!(body_direction.distance(expected.sun_direction) < 1.0e-6);
        assert_approx_eq(
            app.world()
                .entity(camera)
                .get::<Exposure>()
                .expect("static exposure should be applied")
                .ev100,
            expected.exposure_ev100,
        );
    }

    #[test]
    fn hot_reload_from_static_to_cycle_inserts_the_cycle_default_time() {
        let (mut app, _) = runtime_app(flat_settings(LightingProfile::Static), None);
        enter(&mut app, Screen::Gameplay);
        assert!(!app.world().contains_resource::<TimeOfDay>());
        assert_eq!(app.world().resource::<ResolvedLighting>().key_body, None);

        let cycle = cycle_settings();
        let expected = cycle
            .resolve(None)
            .expect("the cycle default should resolve");
        *app.world_mut().resource_mut::<LightingSettings>() = cycle;
        app.update();

        assert_approx_eq(
            app.world().resource::<TimeOfDay>().hours,
            expected
                .time_hours
                .expect("a cycle should publish a session time"),
        );
        assert_eq!(*app.world().resource::<ResolvedLighting>(), expected);
        let (_, illuminance, body_direction, ray_direction, _) = key_light_snapshot(&mut app);
        assert_approx_eq(illuminance, expected.key_illuminance);
        assert!(body_direction.distance(expected.sun_direction) < 1.0e-6);
        assert!(ray_direction.distance(-expected.sun_direction) < 1.0e-6);
    }

    #[test]
    fn gameplay_reentry_replaces_instead_of_accumulating_key_lights() {
        let (mut app, _) = runtime_app(flat_settings(LightingProfile::Static), None);
        enter(&mut app, Screen::Gameplay);
        assert_eq!(key_light_snapshot(&mut app).0, 1);
        assert_eq!(shadow_casting_directional_light_count(&mut app), 1);

        enter(&mut app, Screen::Title);
        assert_eq!(key_light_snapshot(&mut app).0, 0);
        assert_eq!(shadow_casting_directional_light_count(&mut app), 0);

        enter(&mut app, Screen::Gameplay);
        let (count, _, body_direction, ray_direction, shadows) = key_light_snapshot(&mut app);
        assert_eq!(count, 1);
        assert_eq!(shadow_casting_directional_light_count(&mut app), 1);
        assert!(shadows);
        assert!(body_direction.distance(-ray_direction) < 1e-6);
    }

    #[test]
    fn ambient_projection_tracks_static_sun_and_moon_profiles() {
        let (mut static_app, _) = runtime_app(flat_settings(LightingProfile::Static), None);
        enter(&mut static_app, Screen::Gameplay);
        assert_eq!(
            static_app.world().resource::<ExteriorIllumination>().level,
            IlluminationLevel::Bright
        );

        let (mut cycle_app, _) = runtime_app(cycle_settings(), Some(12.0));
        enter(&mut cycle_app, Screen::Gameplay);
        assert_eq!(
            cycle_app.world().resource::<ExteriorIllumination>().level,
            IlluminationLevel::Bright
        );

        cycle_app.world_mut().resource_mut::<TimeOfDay>().hours = 0.0;
        cycle_app.update();
        assert_eq!(
            cycle_app.world().resource::<ExteriorIllumination>().level,
            IlluminationLevel::Dim
        );
    }

    #[test]
    fn ambient_projection_is_session_scoped() {
        let (mut app, _) = runtime_app(flat_settings(LightingProfile::Static), None);
        enter(&mut app, Screen::Gameplay);
        assert!(app.world().contains_resource::<ExteriorIllumination>());

        enter(&mut app, Screen::Title);
        assert!(!app.world().contains_resource::<ExteriorIllumination>());

        enter(&mut app, Screen::Gameplay);
        assert_eq!(
            app.world().resource::<ExteriorIllumination>().level,
            IlluminationLevel::Bright
        );
    }

    #[test]
    fn missing_lighting_settings_are_an_explicit_setup_failure() {
        let mut builder = HeadlessAppBuilder::new()
            .with_minimal_plugins()
            .with_state_plugin();
        builder.app_mut().init_state::<Screen>();
        builder
            .app_mut()
            .add_systems(OnEnter(Screen::Gameplay), publish_exterior_illumination);
        let mut app = builder.build();

        enter(&mut app, Screen::Gameplay);

        let failure = app.world().resource::<GameplaySetupFailure>();
        assert!(failure.reason.contains("settings were unavailable"));
        assert!(!app.world().contains_resource::<ExteriorIllumination>());
    }
}
