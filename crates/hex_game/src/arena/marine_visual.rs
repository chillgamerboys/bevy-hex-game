//! Portable sailboard presentation. Gameplay owns its pose, wind and oxygen.
use super::{ArenaFrame, ViewState};
use bevy::prelude::*;
use hex_arena::ArenaSession;

#[derive(Component)]
struct Sailboat;
#[derive(Component)]
struct Sail;
#[derive(Component)]
pub(super) struct MarineStatus;

pub(super) fn install(app: &mut App) {
    app.add_systems(Startup, spawn)
        .add_systems(Update, (present, status).in_set(ArenaFrame::Present));
}

fn spawn(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let timber = materials.add(StandardMaterial {
        base_color: Color::srgb(0.34, 0.19, 0.085),
        perceptual_roughness: 0.86,
        ..default()
    });
    let edge = materials.add(StandardMaterial {
        base_color: Color::srgb(0.16, 0.095, 0.045),
        perceptual_roughness: 0.92,
        ..default()
    });
    let cloth = materials.add(StandardMaterial {
        base_color: Color::srgb(0.82, 0.72, 0.49),
        cull_mode: None,
        perceptual_roughness: 0.97,
        ..default()
    });
    let stripe = materials.add(StandardMaterial {
        base_color: Color::srgb(0.23, 0.40, 0.45),
        cull_mode: None,
        perceptual_roughness: 0.97,
        ..default()
    });
    commands
        .spawn((
            Name::new("Portable northern sailboat"),
            Sailboat,
            Transform::default(),
            Visibility::Hidden,
        ))
        .with_children(|root| {
            // Seven stepped planks give the small hull a tapered voxel bow.
            for (z, width) in [
                (-1.2, 0.50),
                (-0.8, 0.95),
                (-0.4, 1.20),
                (0.0, 1.30),
                (0.4, 1.30),
                (0.8, 1.10),
                (1.2, 0.80),
            ] {
                root.spawn((
                    Mesh3d(meshes.add(Cuboid::new(width, 0.22, 0.39))),
                    MeshMaterial3d(timber.clone()),
                    Transform::from_xyz(0.0, 0.20, z),
                    Pickable::IGNORE,
                ));
            }
            for x in [-0.62, 0.62] {
                root.spawn((
                    Mesh3d(meshes.add(Cuboid::new(0.08, 0.16, 1.55))),
                    MeshMaterial3d(edge.clone()),
                    Transform::from_xyz(x, 0.31, 0.18),
                    Pickable::IGNORE,
                ));
            }
            root.spawn((
                Mesh3d(meshes.add(Cuboid::new(0.10, 2.65, 0.10))),
                MeshMaterial3d(edge.clone()),
                Transform::from_xyz(0.28, 1.55, -0.64),
                Pickable::IGNORE,
            ));
            root.spawn((
                Sail,
                Transform::from_xyz(0.28, 0.44, -0.64),
                Visibility::Inherited,
            ))
            .with_children(|sail| {
                let top = Vec3::new(0.0, 2.35, 0.0);
                let mid = Vec3::new(0.0, 1.15, 0.0);
                let tip = Vec3::new(1.45, 0.05, 0.0);
                for (triangle, material) in [
                    (Triangle3d::new(top, mid, tip), cloth),
                    (Triangle3d::new(mid, Vec3::ZERO, tip), stripe),
                ] {
                    sail.spawn((
                        Mesh3d(meshes.add(triangle)),
                        MeshMaterial3d(material),
                        Pickable::IGNORE,
                    ));
                }
            });
        });
}

fn present(
    session: Res<ArenaSession>,
    state: Res<ViewState>,
    mut hulls: Query<(&mut Transform, &mut Visibility), (With<Sailboat>, Without<Sail>)>,
    mut sails: Query<&mut Transform, (With<Sail>, Without<Sailboat>)>,
) {
    let boat = session
        .human_actor_id()
        .and_then(|id| session.actors.iter().find(|actor| actor.id == id))
        .and_then(|actor| {
            actor
                .boat()
                .filter(|boat| boat.active)
                .map(|boat| (actor, boat))
        });
    for (mut transform, mut visibility) in &mut hulls {
        let Some((actor, boat)) = boat.filter(|_| state.started) else {
            *visibility = Visibility::Hidden;
            continue;
        };
        *visibility = Visibility::Inherited;
        // The deck offset is physical; no independent sine or camera bob exists.
        transform.translation = actor.feet - Vec3::Y * 0.35;
        let up = boat.surface_normal.normalize_or(Vec3::Y);
        let forward = (boat.heading - up * boat.heading.dot(up)).normalize_or(Vec3::NEG_Z);
        transform.rotation = Transform::IDENTITY.looking_to(forward, up).rotation;
        let across = boat.wind.dot(boat.heading.cross(Vec3::Y));
        for mut sail in &mut sails {
            sail.rotation = Quat::from_rotation_y((across / 10.0).clamp(-0.8, 0.8));
        }
    }
}

fn status(
    session: Res<ArenaSession>,
    mut labels: Query<(&mut Text, &mut TextColor, &mut Node), With<MarineStatus>>,
) {
    let player = session
        .human_actor_id()
        .and_then(|id| session.actors.iter().find(|actor| actor.id == id));
    let boat = player
        .and_then(hex_arena::Actor::boat)
        .filter(|boat| boat.active);
    let swim = player
        .and_then(hex_arena::Actor::swimming)
        .filter(|swim| swim.submerged || swim.oxygen_seconds < swim.oxygen_capacity_seconds - 0.05);
    for (mut text, mut color, mut node) in &mut labels {
        let (label, ink) = if let Some(swim) = swim {
            (
                format!("AIR {:.0}s", swim.oxygen_seconds.max(0.0)),
                if swim.oxygen_seconds < 10.0 {
                    Color::srgb(1.0, 0.32, 0.24)
                } else if swim.oxygen_seconds < 20.0 {
                    Color::srgb(1.0, 0.72, 0.20)
                } else {
                    Color::srgb(0.70, 0.90, 1.0)
                },
            )
        } else if let Some(boat) = boat {
            (
                format!(
                    "SAIL {:.0} u/s · WIND {} {:.0}",
                    boat.velocity.with_y(0.0).length(),
                    wind_heading(boat.wind),
                    boat.wind.length()
                ),
                Color::srgb(0.70, 0.90, 1.0),
            )
        } else {
            (String::new(), Color::WHITE)
        };
        super::ux::set_display(
            &mut node,
            if label.is_empty() {
                Display::None
            } else {
                Display::Flex
            },
        );
        if text.0 != label {
            text.0 = label;
        }
        color.set_if_neq(TextColor(ink));
    }
}

fn wind_heading(wind: Vec3) -> &'static str {
    let x = wind.x;
    let north = -wind.z;
    if x.abs() > north.abs() * 2.414 {
        if x > 0.0 {
            "E"
        } else {
            "W"
        }
    } else if north.abs() > x.abs() * 2.414 {
        if north > 0.0 {
            "N"
        } else {
            "S"
        }
    } else if x > 0.0 {
        if north > 0.0 {
            "NE"
        } else {
            "SE"
        }
    } else if north > 0.0 {
        "NW"
    } else {
        "SW"
    }
}
