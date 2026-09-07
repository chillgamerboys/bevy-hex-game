//! Camera and bounded visual projections of authoritative arena state.

use super::{ArenaCamera, ViewState};
use bevy::prelude::*;
use hex_arena::{preview, ArenaSession, ArenaTuning, Spell};
use hex_core::arena::{ArenaTerrainView, ArenaVoxelGeometry};

#[derive(Component)]
pub(super) struct ActorModel(u8);

pub(super) fn actors(
    mut commands: Commands,
    session: Res<ArenaSession>,
    state: Res<ViewState>,
    mut models: Query<(Entity, &ActorModel, &mut Transform, &mut Visibility)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for actor in &session.actors {
        if let Some((_, _, mut transform, mut visibility)) = models
            .iter_mut()
            .find(|(_, model, _, _)| model.0 == actor.id)
        {
            transform.translation = actor.feet;
            transform.rotation = Quat::from_rotation_y((-actor.aim.x).atan2(-actor.aim.z));
            let external_capture = state.capture.is_some()
                && state.capture_view != "first"
                && state.capture_view != "third"
                && state.capture_view != "tuning";
            let retracted_into_body =
                super::camera_origin(&session, &state, actor.eye(), super::aim(&state))
                    .distance(actor.eye())
                    < 0.45;
            *visibility = if actor.id == 0
                && !external_capture
                && (!state.third_person || retracted_into_body)
            {
                Visibility::Hidden
            } else {
                Visibility::Visible
            };
            continue;
        }
        let color = if actor.id == 0 {
            Color::srgb(0.12, 0.57, 0.68)
        } else {
            Color::srgb(0.83, 0.22, 0.16)
        };
        let cloth = materials.add(StandardMaterial {
            base_color: color,
            perceptual_roughness: 0.8,
            ..default()
        });
        let dark = materials.add(StandardMaterial {
            base_color: Color::srgb(0.055, 0.075, 0.095),
            ..default()
        });
        let skin = materials.add(StandardMaterial {
            base_color: Color::srgb(0.83, 0.67, 0.48),
            ..default()
        });
        commands
            .spawn((
                ActorModel(actor.id),
                Transform::from_translation(actor.feet),
                Visibility::default(),
                Name::new(format!("Arena actor {}", actor.id)),
            ))
            .with_children(|body| {
                body.spawn((
                    Mesh3d(meshes.add(Cuboid::new(0.36, 0.36, 0.24))),
                    MeshMaterial3d(cloth.clone()),
                    Transform::from_xyz(0.0, 0.40, 0.0),
                ));
                body.spawn((
                    Mesh3d(meshes.add(Cuboid::new(0.23, 0.23, 0.22))),
                    MeshMaterial3d(skin),
                    Transform::from_xyz(0.0, 0.68, 0.0),
                ));
                body.spawn((
                    Mesh3d(meshes.add(Cuboid::new(0.24, 0.07, 0.04))),
                    MeshMaterial3d(dark.clone()),
                    Transform::from_xyz(0.0, 0.72, -0.12),
                ));
                for side in [-1.0, 1.0] {
                    body.spawn((
                        Mesh3d(meshes.add(Cuboid::new(0.13, 0.28, 0.16))),
                        MeshMaterial3d(cloth.clone()),
                        Transform::from_xyz(side * 0.245, 0.38, 0.0),
                    ));
                    body.spawn((
                        Mesh3d(meshes.add(Cuboid::new(0.13, 0.23, 0.18))),
                        MeshMaterial3d(dark.clone()),
                        Transform::from_xyz(side * 0.105, 0.115, 0.0),
                    ));
                }
            });
    }
}

pub(super) fn camera(
    session: Res<ArenaSession>,
    state: Res<ViewState>,
    mut cameras: Query<&mut Transform, With<ArenaCamera>>,
) {
    let Some(actor) = session.actors.first() else {
        return;
    };
    let Ok(mut transform) = cameras.single_mut() else {
        return;
    };
    if state.capture.is_some() {
        match state.capture_view.as_str() {
            "overview" => {
                *transform = Transform::from_xyz(28.0, 34.0, 32.0)
                    .looking_at(Vec3::new(0.0, 2.0, 0.0), Vec3::Y);
                return;
            }
            "rear" => {
                *transform = Transform::from_xyz(-29.0, 25.0, -34.0)
                    .looking_at(Vec3::new(0.0, 2.0, 0.0), Vec3::Y);
                return;
            }
            view if view.starts_with("shield")
                || view.starts_with("fireball")
                || view.starts_with("blast") =>
            {
                *transform = Transform::from_translation(actor.feet + Vec3::new(-4.0, 8.0, 11.0))
                    .looking_at(actor.feet + Vec3::new(2.5, 0.7, 0.0), Vec3::Y);
                return;
            }
            _ => {}
        }
    }
    let eye = actor.eye();
    let direction = if state.capture.is_none() && state.initialized {
        super::aim(&state)
    } else {
        actor.aim.normalize_or(Vec3::NEG_Z)
    };
    let position = super::camera_origin(&session, &state, eye, direction);
    *transform = Transform::from_translation(position).looking_to(direction, Vec3::Y);
}

pub(super) fn effects(
    session: Res<ArenaSession>,
    tuning: Res<ArenaTuning>,
    view: Res<ArenaTerrainView>,
    geometry: Res<ArenaVoxelGeometry>,
    state: Res<ViewState>,
    mut gizmos: Gizmos,
) {
    let color = |spell| match spell {
        Spell::Shield => Color::srgb(0.36, 0.85, 0.95),
        Spell::Fireball => Color::srgb(1.0, 0.37, 0.07),
        Spell::AreaBlast => Color::srgb(0.70, 0.40, 1.0),
    };
    for projectile in &session.projectiles {
        let c = color(projectile.spell);
        gizmos.sphere(Isometry3d::from_translation(projectile.position), 0.13, c);
        let start = projectile.position - projectile.velocity.normalize_or_zero() * 0.8;
        for offset in [Vec3::ZERO, Vec3::Y * 0.025, Vec3::X * 0.025] {
            gizmos.line(start + offset, projectile.position + offset, c);
        }
    }
    for effect in &session.effects {
        let progress = (effect.age / effect.lifetime.max(0.01)).clamp(0.0, 1.0);
        let radius = effect.radius * (0.3 + progress * 0.7);
        let c = color(effect.kind).with_alpha(1.0 - progress);
        gizmos.sphere(Isometry3d::from_translation(effect.center), radius, c);
        gizmos.sphere(
            Isometry3d::from_translation(effect.center),
            radius * 0.88,
            c,
        );
    }
    let Some(actor) = session.actors.first() else {
        return;
    };
    let enabled = match actor.selected {
        Spell::Shield => state.previews.first().copied().unwrap_or(false),
        Spell::Fireball => state.previews.get(1).copied().unwrap_or(false),
        Spell::AreaBlast => false,
    };
    if !enabled || state.paused {
        return;
    }
    let predicted = preview(&session, &view, &geometry, &tuning);
    let c = if predicted.valid {
        color(actor.selected)
    } else {
        Color::srgba(1.0, 0.45, 0.26, 0.7)
    };
    for pair in predicted.points.windows(2) {
        if let [a, b] = pair {
            gizmos.line(*a, *b, c);
        }
    }
    if actor.selected == Spell::Shield {
        for pos in predicted.wall_voxels {
            let center = geometry.center(pos);
            gizmos.cube(
                Transform::from_translation(center).with_scale(Vec3::new(
                    1.6,
                    geometry.level_height,
                    1.6,
                )),
                c.with_alpha(0.4),
            );
        }
    } else if let Some(impact) = predicted.impact {
        gizmos.sphere(
            Isometry3d::from_translation(impact),
            tuning.fireball_radius(),
            c.with_alpha(0.5),
        );
    }
}

#[derive(Resource)]
pub(super) struct EffectAssets {
    sphere: Handle<Mesh>,
    block: Handle<Mesh>,
    seed: Handle<StandardMaterial>,
    fire: Handle<StandardMaterial>,
    shield_wave: Handle<StandardMaterial>,
    fire_wave: Handle<StandardMaterial>,
    blast_wave: Handle<StandardMaterial>,
}

#[derive(Component)]
pub(super) struct TransientEffect;

pub(super) fn setup_effects(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mut material = |color: Color, alpha: f32| {
        materials.add(StandardMaterial {
            base_color: color.with_alpha(alpha),
            unlit: true,
            cull_mode: None,
            alpha_mode: if alpha < 1.0 {
                AlphaMode::Blend
            } else {
                AlphaMode::Opaque
            },
            ..default()
        })
    };
    commands.insert_resource(EffectAssets {
        sphere: meshes.add(Sphere::new(1.0).mesh().uv(16, 12)),
        block: meshes.add(Cuboid::new(1.6, 1.0, 1.6)),
        seed: material(Color::srgb(0.25, 0.86, 0.98), 1.0),
        fire: material(Color::srgb(1.0, 0.30, 0.035), 1.0),
        shield_wave: material(Color::srgb(0.25, 0.86, 0.98), 0.14),
        fire_wave: material(Color::srgb(1.0, 0.30, 0.035), 0.16),
        blast_wave: material(Color::srgb(0.70, 0.35, 1.0), 0.16),
    });
}

pub(super) fn solid_effects(
    mut commands: Commands,
    session: Res<ArenaSession>,
    assets: Res<EffectAssets>,
    geometry: Res<ArenaVoxelGeometry>,
    previous: Query<Entity, With<TransientEffect>>,
) {
    // Shared handles bound allocations: effects own only short-lived entities.
    for entity in &previous {
        commands.entity(entity).despawn();
    }
    for projectile in &session.projectiles {
        commands.spawn((
            TransientEffect,
            Mesh3d(assets.sphere.clone()),
            MeshMaterial3d(if projectile.spell == Spell::Shield {
                assets.seed.clone()
            } else {
                assets.fire.clone()
            }),
            Transform::from_translation(projectile.position).with_scale(Vec3::splat(0.105)),
        ));
    }
    for (volume, progress) in session.emerging_shields() {
        let minimum = volume.iter().map(|p| p.level).min().unwrap_or(0);
        let maximum = volume.iter().map(|p| p.level).max().unwrap_or(minimum);
        let levels = u16::try_from(maximum - minimum + 1).unwrap_or(1);
        let drop = f32::from(levels) * geometry.level_height * (1.0 - progress);
        for pos in volume {
            commands.spawn((
                TransientEffect,
                Mesh3d(assets.block.clone()),
                MeshMaterial3d(assets.shield_wave.clone()),
                Transform::from_translation(geometry.center(*pos) - Vec3::Y * drop)
                    .with_scale(Vec3::new(1.0, geometry.level_height, 1.0)),
            ));
        }
    }
    for effect in &session.effects {
        let progress = (effect.age / effect.lifetime.max(0.01)).clamp(0.0, 1.0);
        let material = match effect.kind {
            Spell::Shield => &assets.shield_wave,
            Spell::Fireball => &assets.fire_wave,
            Spell::AreaBlast => &assets.blast_wave,
        };
        commands.spawn((
            TransientEffect,
            Mesh3d(assets.sphere.clone()),
            MeshMaterial3d(material.clone()),
            Transform::from_translation(effect.center)
                .with_scale(Vec3::splat(effect.radius * (0.3 + progress * 0.7))),
        ));
    }
}
