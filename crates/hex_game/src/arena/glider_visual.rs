//! Disposable canopy presentation; flight and collision stay in hex_arena.
use super::{ArenaFrame, ViewState};
use bevy::prelude::*;
use hex_arena::ArenaSession;

#[derive(Component)]
struct GliderCanopy;

pub(super) fn install(app: &mut App) {
    app.add_systems(Startup, spawn)
        .add_systems(Update, present.in_set(ArenaFrame::Present));
}
fn spawn(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let fabric = materials.add(StandardMaterial {
        base_color: Color::srgb(0.62, 0.18, 0.13),
        perceptual_roughness: 0.9,
        cull_mode: None,
        ..default()
    });
    let trim = materials.add(StandardMaterial {
        base_color: Color::srgb(0.83, 0.73, 0.48),
        perceptual_roughness: 0.9,
        cull_mode: None,
        ..default()
    });
    let spar = materials.add(StandardMaterial {
        base_color: Color::srgb(0.20, 0.12, 0.065),
        ..default()
    });
    commands
        .spawn((
            Transform::default(),
            Visibility::Hidden,
            GliderCanopy,
            Name::new("Momentum glider"),
        ))
        .with_children(|root| {
            let nose = Vec3::new(0.0, 0.2, -0.9);
            let tail = Vec3::new(0.0, 0.08, 0.65);
            for (tip, material) in [
                (Vec3::new(-1.8, 0.0, 0.65), fabric),
                (Vec3::new(1.8, 0.0, 0.65), trim),
            ] {
                root.spawn((
                    Mesh3d(meshes.add(Triangle3d::new(nose, tip, tail))),
                    MeshMaterial3d(material),
                    Transform::IDENTITY,
                    Pickable::IGNORE,
                ));
            }
            root.spawn((
                Mesh3d(meshes.add(Cuboid::new(3.5, 0.04, 0.04))),
                MeshMaterial3d(spar.clone()),
                Transform::from_xyz(0.0, -0.02, 0.58),
                Pickable::IGNORE,
            ));
            root.spawn((
                Mesh3d(meshes.add(Cuboid::new(0.04, 0.04, 1.5))),
                MeshMaterial3d(spar),
                Transform::from_xyz(0.0, 0.10, -0.1),
                Pickable::IGNORE,
            ));
        });
}
fn present(
    session: Res<ArenaSession>,
    state: Res<ViewState>,
    mut canopy: Query<(&mut Transform, &mut Visibility), With<GliderCanopy>>,
) {
    let player = session
        .human_actor_id()
        .and_then(|id| session.actors.iter().find(|actor| actor.id == id));
    let flight = player.and_then(|actor| {
        actor
            .glider()
            .filter(|flight| flight.open)
            .map(|flight| (actor, flight))
    });
    for (mut transform, mut visibility) in &mut canopy {
        let Some((actor, flight)) = flight.filter(|_| state.started) else {
            *visibility = Visibility::Hidden;
            continue;
        };
        *visibility = Visibility::Inherited;
        transform.translation = actor.feet + Vec3::Y * (actor.body_dimensions().y + 0.3);
        transform.rotation =
            Quat::from_rotation_arc(Vec3::NEG_Z, flight.direction.normalize_or(Vec3::NEG_Z));
    }
}
