use bevy::light::EnvironmentMapLight;
use bevy::prelude::*;

use hex_assets::{to_color, LightingSettings};
use hex_core::{AppSystems, GameplaySetup, Screen};

/// Registers the sun, the optional sky light, and optional distance haze.
///
/// The sky light and haze are both **off in the shipped settings** — they exist so the
/// look can be explored from `lighting.ron` without a code change, and default to
/// values that render exactly as the game did before they were added.
pub fn plugin(app: &mut App) {
    // Lighting is applied once settings have loaded rather than at startup, since
    // the values now come from a file that is still loading at that point.
    app.add_systems(
        OnEnter(Screen::Gameplay),
        (apply_sky_settings, spawn_sun).in_set(GameplaySetup::Terrain),
    )
    .add_systems(OnExit(Screen::Gameplay), despawn_sun)
    // The sky light and haze live on the camera, which outlives gameplay, so these
    // run always rather than per screen.
    .add_systems(
        Update,
        (reload_lighting, apply_view_lighting).in_set(AppSystems::Update),
    );
}

fn apply_sky_settings(
    mut commands: Commands,
    mut ambient: ResMut<GlobalAmbientLight>,
    settings: Res<LightingSettings>,
) {
    apply_ambient(&mut commands, &mut ambient, &settings);
}

/// The uniform fill, and the colour behind everything the dome does not cover.
fn apply_ambient(
    commands: &mut Commands,
    ambient: &mut GlobalAmbientLight,
    settings: &LightingSettings,
) {
    commands.insert_resource(ClearColor(to_color(settings.sky_color)));
    ambient.brightness = settings.ambient_brightness;
    ambient.color = to_color(settings.ambient_color);
}

fn despawn_sun(mut commands: Commands, suns: Query<Entity, With<Sun>>) {
    for entity in &suns {
        commands.entity(entity).despawn();
    }
}

/// Marks the directional light acting as the sun.
#[derive(Component)]
struct Sun;

fn spawn_sun(mut commands: Commands, settings: Res<LightingSettings>) {
    commands.spawn((
        sun_light(&settings),
        sun_transform(&settings),
        Name::new("Sun"),
        Sun,
    ));
}

fn sun_light(settings: &LightingSettings) -> DirectionalLight {
    DirectionalLight {
        illuminance: settings.sun_illuminance,
        color: to_color(settings.sun_color),
        shadow_maps_enabled: true,
        ..default()
    }
}

fn sun_transform(settings: &LightingSettings) -> Transform {
    let (x, y, z) = settings.sun_rotation;
    Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, x, y, z))
}

/// Re-applies the sun and the ambient fill when `lighting.ron` is edited.
///
/// Without this the sun only picked up a change on the next `OnEnter(Gameplay)`, so
/// tuning a light angle meant a round trip through the title screen. The sun exists
/// only during gameplay, so off that screen the query is simply empty.
fn reload_lighting(
    mut commands: Commands,
    settings: Option<Res<LightingSettings>>,
    mut ambient: ResMut<GlobalAmbientLight>,
    mut suns: Query<(&mut DirectionalLight, &mut Transform), With<Sun>>,
) {
    let Some(settings) = settings else {
        return;
    };
    if !settings.is_changed() {
        return;
    }
    apply_ambient(&mut commands, &mut ambient, &settings);
    for (mut light, mut transform) in &mut suns {
        *light = sun_light(&settings);
        *transform = sun_transform(&settings);
    }
}

/// Attaches the optional sky light and distance haze to the camera, and keeps them in
/// step with `lighting.ron`. **Both are off in the shipped settings.**
///
/// The sky light is a directional fill that makes shadows read as sky-lit rather than
/// flat grey: a tiny cubemap built in code, `zenith_color` overhead through `sky_color`
/// at the horizon down to `ground_color` beneath. It is built from the same settings the
/// sky shader uses, so the two cannot drift apart.
///
/// Both are removed rather than merely zeroed when turned off, so an unused sky light
/// costs no cubemap and the camera carries no inert components.
///
/// The haze deliberately does *not* touch the sky: fog is applied by the PBR shader,
/// and the dome is drawn by our own material, which never calls it.
fn apply_view_lighting(
    mut commands: Commands,
    settings: Option<Res<LightingSettings>>,
    mut images: ResMut<Assets<Image>>,
    cameras: Query<Entity, With<Camera3d>>,
) {
    let Some(settings) = settings else {
        return;
    };
    if !settings.is_changed() {
        return;
    }

    for entity in &cameras {
        if settings.sky_light_intensity > 0.0 {
            // Rebuilt rather than patched in place. This runs once per edit of the
            // settings file, and the cubemap is six pixels.
            let mut sky_light = EnvironmentMapLight::hemispherical_gradient(
                &mut images,
                to_color(settings.zenith_color),
                to_color(settings.sky_color),
                to_color(settings.ground_color),
            );
            sky_light.intensity = settings.sky_light_intensity;
            commands.entity(entity).insert(sky_light);
        } else {
            commands.entity(entity).remove::<EnvironmentMapLight>();
        }

        if settings.fog_density > 0.0 {
            commands.entity(entity).insert(DistanceFog {
                color: to_color(settings.fog_color),
                // Haze looking towards the sun, which is what reads as low light.
                directional_light_color: to_color(settings.fog_sun_color),
                directional_light_exponent: 8.0,
                falloff: FogFalloff::Exponential {
                    density: settings.fog_density,
                },
            });
        } else {
            commands.entity(entity).remove::<DistanceFog>();
        }
    }
}
