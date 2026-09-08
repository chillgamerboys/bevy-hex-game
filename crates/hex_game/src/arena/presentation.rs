//! Camera and bounded visual projections of authoritative arena state.

use super::{ArenaCamera, ViewState};
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use hex_arena::{preview, ArenaSession, ArenaTuning, Species, Spell};
use hex_core::arena::{ArenaReset, ArenaTerrainView, ArenaVoxelGeometry};

#[derive(Component)]
pub(super) struct ActorModel(u8, Species, u64);

fn player_camera(session: &ArenaSession, state: &ViewState, actor: &hex_arena::Actor) -> Transform {
    let direction = if state.capture.is_none() && state.initialized {
        super::aim(state)
    } else {
        actor.aim.normalize_or(Vec3::NEG_Z)
    };
    let position = super::camera_origin(session, state, actor.eye(), direction);
    Transform::from_translation(position).looking_to(direction, Vec3::Y)
}

#[derive(Clone, Copy)]
struct BodyPart {
    size: Vec3,
    center: Vec3,
    material: usize,
}

fn body_parts(species: Species) -> Vec<BodyPart> {
    let part = |size, center, material| BodyPart {
        size: Vec3::from_array(size),
        center: Vec3::from_array(center),
        material,
    };
    if species == Species::Dragon {
        let mut parts = vec![
            part([0.68, 0.60, 0.55], [0.0, 0.47, 0.045], 0),
            part([0.36, 0.40, 0.18], [0.0, 0.51, -0.30], 0),
            part([0.42, 0.48, 0.16], [0.0, 0.49, -0.42], 0),
            part([0.23, 0.22, 0.08], [0.0, 0.40, -0.46], 1),
            part([0.24, 0.26, 0.18], [0.0, 0.28, 0.40], 0),
        ];
        for side in [-1.0, 1.0] {
            // Decorative folded wings stay within the actual long, low body's footprint.
            parts.push(part([0.16, 0.24, 0.44], [side * 0.42, 0.74, 0.08], 1));
            parts.push(part([0.06, 0.10, 0.03], [side * 0.20, 0.63, -0.465], 3));
            for z in [-0.16, 0.20] {
                parts.push(part([0.18, 0.24, 0.12], [side * 0.30, 0.12, z], 1));
            }
        }
        return parts;
    }
    let mut parts = vec![
        part([0.62, 0.45, 0.48], [0.0, 0.50, 0.0], 0),
        part([0.48, 0.29, 0.44], [0.0, 0.85, 0.0], 2),
        part([0.49, 0.09, 0.08], [0.0, 0.90, -0.24], 1),
    ];
    for side in [-1.0, 1.0] {
        parts.push(part([0.20, 0.35, 0.32], [side * 0.40, 0.47, 0.0], 0));
        parts.push(part([0.26, 0.28, 0.36], [side * 0.21, 0.14, 0.0], 1));
    }
    if species == Species::Shaman {
        parts.push(part([0.56, 0.10, 0.50], [0.0, 0.94, 0.0], 0));
        parts.push(part([0.08, 0.76, 0.10], [0.43, 0.40, -0.31], 3));
    }
    if species == Species::Goblin {
        for side in [-1.0, 1.0] {
            parts.push(part([0.18, 0.12, 0.18], [side * 0.32, 0.87, 0.0], 2));
        }
    }
    parts
}

pub(super) fn actors(
    mut commands: Commands,
    session: Res<ArenaSession>,
    state: Res<ViewState>,
    reset: Res<ArenaReset>,
    mut models: Query<(Entity, &ActorModel, &mut Transform, &mut Visibility)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (entity, model, _, _) in &mut models {
        if model.2 != reset.generation
            || !session
                .actors
                .iter()
                .any(|actor| actor.id == model.0 && actor.species == model.1)
        {
            commands.entity(entity).despawn();
        }
    }
    for actor in &session.actors {
        let retracted_into_body = actor.id == 0
            && player_camera(&session, &state, actor)
                .translation
                .distance(actor.eye())
                < 0.45;
        let actor_visibility = if actor.hp <= 0.0
            || (actor.id == 0
                && !state.external_camera()
                && (!state.third_person || retracted_into_body))
        {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
        let transform =
            Transform::from_translation(actor.feet).with_rotation(actor.body_rotation());
        if let Some((_, _, mut existing, mut visibility)) =
            models.iter_mut().find(|(_, model, _, _)| {
                model.0 == actor.id && model.1 == actor.species && model.2 == reset.generation
            })
        {
            *existing = transform;
            *visibility = actor_visibility;
            continue;
        }
        let (cloth, skin) = match actor.species {
            Species::Human => (Color::srgb(0.12, 0.57, 0.68), Color::srgb(0.83, 0.67, 0.48)),
            Species::Shadow => (Color::srgb(0.83, 0.22, 0.16), Color::srgb(0.83, 0.67, 0.48)),
            Species::Dragon => (
                Color::srgb(0.52, 0.12, 0.055),
                Color::srgb(0.69, 0.27, 0.08),
            ),
            Species::Goblin => (Color::srgb(0.27, 0.22, 0.12), Color::srgb(0.30, 0.57, 0.19)),
            Species::Shaman => (Color::srgb(0.40, 0.15, 0.54), Color::srgb(0.44, 0.66, 0.26)),
        };
        let palette = [
            cloth,
            Color::srgb(0.055, 0.075, 0.095),
            skin,
            Color::srgb(1.0, 0.62, 0.15),
        ]
        .map(|base_color| {
            materials.add(StandardMaterial {
                base_color,
                perceptual_roughness: 0.8,
                ..default()
            })
        });
        let dimensions = actor.body_dimensions();
        commands
            .spawn((
                ActorModel(actor.id, actor.species, reset.generation),
                transform,
                actor_visibility,
                Name::new(format!("Arena actor {}", actor.id)),
            ))
            .with_children(|body| {
                for part in body_parts(actor.species) {
                    body.spawn((
                        Mesh3d(meshes.add(Cuboid::from_size(part.size * dimensions))),
                        MeshMaterial3d(palette.get(part.material).cloned().unwrap_or_default()),
                        Transform::from_translation(part.center * dimensions),
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
    if state.external_camera() {
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
    *transform = player_camera(&session, &state, actor);
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
        // Outline the landing footprint, keeping the wall's full height out of
        // the aiming view. These pointy-hex corners match the public voxel geometry.
        let corners = [
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(0.866_025_4, 0.0, 0.5),
            Vec3::new(0.866_025_4, 0.0, -0.5),
            Vec3::new(0.0, 0.0, -1.0),
            Vec3::new(-0.866_025_4, 0.0, -0.5),
            Vec3::new(-0.866_025_4, 0.0, 0.5),
        ];
        for pos in shield_footprint(&predicted.wall_voxels) {
            let center = geometry.center(pos) - Vec3::Y * (geometry.level_height * 0.5 - 0.015);
            for (a, b) in corners.into_iter().zip(corners.into_iter().cycle().skip(1)) {
                gizmos.line(center + a, center + b, c);
            }
        }
    } else if let Some(impact) = predicted.impact {
        gizmos.sphere(
            Isometry3d::from_translation(impact),
            tuning.fireball_radius(),
            c.with_alpha(0.5),
        );
    }
}

/// Partial shields can begin at a different surviving level in every column.
pub(super) fn shield_footprint(voxels: &[hex_core::TilePos]) -> Vec<hex_core::TilePos> {
    let mut columns = std::collections::BTreeMap::new();
    for pos in voxels {
        columns
            .entry(pos.coord)
            .and_modify(|lowest: &mut hex_core::TilePos| {
                if pos.level < lowest.level {
                    *lowest = *pos;
                }
            })
            .or_insert(*pos);
    }
    columns.into_values().collect()
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

// These meshes visualize effects; they must not project opaque geometry shadows.
#[derive(Component)]
#[require(NotShadowCaster)]
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

#[cfg(test)]
mod model_tests {
    use super::*;
    #[test]
    fn creature_parts_follow_the_physical_body_bounds() {
        for species in [Species::Dragon, Species::Goblin, Species::Shaman] {
            let parts = body_parts(species);
            assert!(!parts.is_empty());
            for part in parts {
                let minimum = part.center - part.size * 0.5;
                let maximum = part.center + part.size * 0.5;
                assert!(
                    minimum.cmpge(Vec3::new(-0.501, -0.001, -0.501)).all(),
                    "{species:?}: {minimum:?}"
                );
                assert!(
                    maximum.cmple(Vec3::new(0.501, 1.001, 0.501)).all(),
                    "{species:?}: {maximum:?}"
                );
            }
        }
    }
}
