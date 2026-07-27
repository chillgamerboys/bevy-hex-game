use bevy::camera::Exposure;
use bevy::light::EnvironmentMapLight;
use bevy::prelude::*;

use hex_assets::{to_color, CelestialBody, LightingProfile, LightingSettings, ResolvedLighting};
use hex_core::{AppSystems, GameplaySetup, Screen};

use crate::LightingSystems;

/// Designer-controlled gameplay time in hours.
///
/// The resource exists only for an active cyclic-lighting gameplay session. Scenario
/// setup inserts its configured or profile-default value, review automation may
/// replace it, and the development inspector can edit `hours` directly. Static
/// profiles do not insert it. No runtime system advances it.
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

/// Registers the celestial key light and the resolved scene/view lighting.
pub fn plugin(app: &mut App) {
    app.register_type::<TimeOfDay>()
        .register_type::<CelestialKeyLight>()
        .configure_sets(
            Update,
            (LightingSystems::Resolve, LightingSystems::Apply)
                .chain()
                .in_set(AppSystems::Update),
        )
        .add_systems(
            OnEnter(Screen::Gameplay),
            (
                resolve_lighting,
                spawn_celestial_key_light,
                apply_scene_lighting,
                apply_view_lighting,
            )
                .chain()
                .in_set(GameplaySetup::Terrain),
        )
        .add_systems(OnExit(Screen::Gameplay), despawn_celestial_key_light)
        // A resolved frame changes only when its source asset or the inspector's
        // session clock changes. Applying it is similarly change-gated, so a frozen
        // time of day has no per-frame lighting work.
        .add_systems(
            Update,
            resolve_lighting
                .run_if(lighting_inputs_changed)
                .in_set(LightingSystems::Resolve),
        )
        .add_systems(
            Update,
            (apply_scene_lighting, apply_view_lighting)
                .run_if(resource_exists_and_changed::<ResolvedLighting>)
                .in_set(LightingSystems::Apply),
        );
}

/// Marks the one directional light supplied by the active celestial body.
#[derive(Component, Reflect)]
#[reflect(Component)]
struct CelestialKeyLight;

/// Resolves the current profile without advancing its session clock.
///
/// `LightingSettings` is a hard `Res` here because the loading screen blocks gameplay
/// until the selected scenario's profile has loaded and passed cross-asset validation.
fn resolve_lighting(
    mut commands: Commands,
    settings: Res<LightingSettings>,
    time: Option<Res<TimeOfDay>>,
    current: Option<ResMut<ResolvedLighting>>,
) {
    // TimeOfDay is derived session state, not an authored scenario override. Keep its
    // presence synchronized with hot-reloaded profile kind: static ignores/removes a
    // stale clock, while a newly cyclic profile publishes its validated default.
    // Cross-asset scenario validation still calls `LightingSettings::resolve` with the
    // authored override directly, so Static + authored time remains an error.
    let requested_time = match &settings.profile {
        LightingProfile::Static => {
            if time.is_some() {
                commands.remove_resource::<TimeOfDay>();
            }
            None
        }
        LightingProfile::Cycle(cycle) => match time.as_deref() {
            Some(time) => Some(time.hours),
            None => {
                commands.insert_resource(TimeOfDay {
                    hours: cycle.default_time_hours,
                });
                Some(cycle.default_time_hours)
            }
        },
    };
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
) -> bool {
    settings.is_some_and(|settings| settings.is_changed())
        || time.is_some_and(|time| time.is_changed())
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
/// The six-pixel hemispherical cubemap is rebuilt on edits so it cannot drift away
/// from the interpolated sky. Zero intensity removes it entirely. Fog similarly
/// affects PBR geometry but not the custom sky material.
fn apply_view_lighting(
    mut commands: Commands,
    lighting: Res<ResolvedLighting>,
    mut images: ResMut<Assets<Image>>,
    cameras: Query<Entity, With<Camera3d>>,
) {
    for entity in &cameras {
        commands.entity(entity).insert(Exposure {
            ev100: lighting.exposure_ev100,
        });

        if lighting.sky_light_intensity > 0.0 {
            let mut sky_light = EnvironmentMapLight::hemispherical_gradient(
                &mut images,
                to_color(lighting.zenith_color),
                to_color(lighting.sky_color),
                to_color(lighting.ground_color),
            );
            sky_light.intensity = lighting.sky_light_intensity;
            commands.entity(entity).insert(sky_light);
        } else {
            commands.entity(entity).remove::<EnvironmentMapLight>();
        }

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
}

#[cfg(test)]
mod tests {
    use bevy::state::app::StatesPlugin;
    use hex_assets::{CelestialCycleSettings, LightingKeyframe, LightingProfile};

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
                keyframe(6.0, 330.0, 5.0, CelestialBody::Sun, 500.0, 8.0),
                keyframe(12.0, 40.0, 60.0, CelestialBody::Sun, 10_000.0, 9.7),
                keyframe(18.0, 150.0, -5.0, CelestialBody::Moon, 500.0, 7.8),
            ],
        };
        let settings = flat_settings(LightingProfile::Cycle(cycle));
        settings
            .validate()
            .expect("the runtime test cycle should be valid");
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

    fn runtime_app(settings: LightingSettings, time: Option<f32>) -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin));
        app.init_state::<Screen>();
        app.insert_resource(settings);
        app.insert_resource(GlobalAmbientLight::default());
        app.insert_resource(Assets::<Image>::default());
        if let Some(hours) = time {
            app.insert_resource(TimeOfDay { hours });
        }
        app.add_plugins(super::plugin);
        let camera = app.world_mut().spawn(Camera3d::default()).id();
        (app, camera)
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
}
