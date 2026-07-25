use bevy::prelude::*;

use hex_core::config::{SUN_AMBIENT_LIGHT, SUN_INTENSITY, SUN_ROTATION};

pub fn plugin(app: &mut App) {
    app.insert_resource(ClearColor(Color::srgb(0.5294, 0.8087, 0.9216)))
        .insert_resource(GlobalAmbientLight {
            brightness: SUN_AMBIENT_LIGHT,
            ..default()
        })
        .add_systems(Startup, spawn_sun);
}

fn spawn_sun(mut commands: Commands) {
    commands.spawn((
        DirectionalLight {
            illuminance: SUN_INTENSITY,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(
            EulerRot::XYZ,
            SUN_ROTATION.0,
            SUN_ROTATION.1,
            SUN_ROTATION.2,
        )),
        Name::new("Sun"),
    ));
}
