use bevy::prelude::*;

use hex_assets::{to_color, LightingSettings};
use hex_core::{GameplaySetup, Screen};

/// Registers the sun, ambient light, and sky colour.
pub fn plugin(app: &mut App) {
    // Lighting is applied once settings have loaded rather than at startup, since
    // the values now come from a file that is still loading at that point.
    app.add_systems(
        OnEnter(Screen::Gameplay),
        (apply_sky_settings, spawn_sun).in_set(GameplaySetup::Terrain),
    )
    .add_systems(OnExit(Screen::Gameplay), despawn_sun);
}

fn apply_sky_settings(
    mut commands: Commands,
    mut ambient: ResMut<GlobalAmbientLight>,
    settings: Res<LightingSettings>,
) {
    commands.insert_resource(ClearColor(to_color(settings.sky_color)));
    ambient.brightness = settings.ambient_brightness;
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
    let (x, y, z) = settings.sun_rotation;
    commands.spawn((
        DirectionalLight {
            illuminance: settings.sun_illuminance,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, x, y, z)),
        Name::new("Sun"),
        Sun,
    ));
}
