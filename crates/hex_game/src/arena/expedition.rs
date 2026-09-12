//! Visible milestone spheres and charged fountain water from public snapshots.

use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::prelude::*;
use hex_arena::{ArenaSession, ExpeditionReward};
use hex_core::arena::{ArenaReset, ArenaTerrainView, ArenaVoxelGeometry};
use std::collections::BTreeMap;

#[derive(Resource)]
pub(super) struct ExpeditionVisualAssets {
    sphere: Handle<Mesh>,
    cap: Handle<Mesh>,
    gold: Handle<StandardMaterial>,
    blue: Handle<StandardMaterial>,
    violet: Handle<StandardMaterial>,
    halo: Handle<StandardMaterial>,
    pool: Handle<StandardMaterial>,
}

#[derive(Component)]
pub(super) struct RewardVisual(ExpeditionReward, u64);

#[derive(Component)]
pub(super) struct PoolVisual {
    name: String,
    generation: u64,
}

pub(super) fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mut light = |color: Color| {
        materials.add(StandardMaterial {
            base_color: color,
            unlit: true,
            alpha_mode: if color.alpha() < 1.0 {
                AlphaMode::Blend
            } else {
                AlphaMode::Opaque
            },
            cull_mode: None,
            ..default()
        })
    };
    commands.insert_resource(ExpeditionVisualAssets {
        sphere: meshes.add(Sphere::new(1.0).mesh().uv(20, 14)),
        cap: meshes.add(super::golem::native_hex_prism()),
        gold: light(Color::srgb(1.0, 0.76, 0.19)),
        blue: light(Color::srgb(0.35, 0.83, 1.0)),
        violet: light(Color::srgb(0.82, 0.55, 1.0)),
        halo: light(Color::srgba(0.88, 0.96, 1.0, 0.10)),
        pool: light(Color::srgba(0.28, 1.0, 0.80, 0.38)),
    });
}

fn reward_color(reward: ExpeditionReward) -> Color {
    match reward {
        ExpeditionReward::TrollDamage => Color::srgb(1.0, 0.76, 0.19),
        ExpeditionReward::DragonExplosions => Color::srgb(0.35, 0.83, 1.0),
        ExpeditionReward::ShadowVitality => Color::srgb(0.82, 0.55, 1.0),
    }
}

pub(super) fn present(
    mut commands: Commands,
    session: Res<ArenaSession>,
    world: Res<ArenaTerrainView>,
    geometry: Res<ArenaVoxelGeometry>,
    reset: Res<ArenaReset>,
    assets: Res<ExpeditionVisualAssets>,
    mut rewards: Query<(Entity, &RewardVisual, &mut Transform)>,
    mut pools: Query<(Entity, &PoolVisual, &mut Visibility)>,
) {
    let progress = session.expedition_progress();
    // Tie animation to simulation time so pausing and deterministic captures freeze it.
    let phase = session.tick as f32 / 120.0;
    for (entity, visual, mut transform) in &mut rewards {
        let position = progress.as_ref().and_then(|p| {
            p.milestones
                .iter()
                .find(|m| m.reward == visual.0 && !m.collected)
                .and_then(|m| m.available_position)
        });
        if visual.1 != reset.generation || position.is_none() {
            commands.entity(entity).despawn();
        } else if let Some(position) = position {
            transform.translation =
                Vec3::from_array(position) + Vec3::Y * (phase * 1.8).sin() * 0.08;
        }
    }
    for (entity, visual, mut visible) in &mut pools {
        let fountain = progress
            .as_ref()
            .and_then(|p| p.fountains.iter().find(|p| p.name == visual.name));
        if visual.generation != reset.generation || fountain.is_none() {
            commands.entity(entity).despawn();
        } else {
            *visible = if fountain.is_some_and(|f| !f.consumed) {
                Visibility::Visible
            } else {
                Visibility::Hidden
            };
        }
    }
    let Some(progress) = progress else {
        return;
    };
    for milestone in progress.milestones {
        let Some(position) = milestone
            .available_position
            .filter(|_| !milestone.collected)
        else {
            continue;
        };
        if rewards
            .iter()
            .any(|(_, v, _)| v.0 == milestone.reward && v.1 == reset.generation)
        {
            continue;
        }
        let material = match milestone.reward {
            ExpeditionReward::TrollDamage => &assets.gold,
            ExpeditionReward::DragonExplosions => &assets.blue,
            ExpeditionReward::ShadowVitality => &assets.violet,
        };
        commands
            .spawn((
                RewardVisual(milestone.reward, reset.generation),
                Name::new("Milestone reward sphere"),
                Transform::from_translation(Vec3::from_array(position)),
                Visibility::default(),
            ))
            .with_children(|orb| {
                orb.spawn((
                    Mesh3d(assets.sphere.clone()),
                    MeshMaterial3d(material.clone()),
                    Transform::from_scale(Vec3::splat(0.43)),
                    NotShadowCaster,
                    NotShadowReceiver,
                ));
                orb.spawn((
                    Mesh3d(assets.sphere.clone()),
                    MeshMaterial3d(assets.halo.clone()),
                    Transform::from_scale(Vec3::splat(0.68)),
                    NotShadowCaster,
                    NotShadowReceiver,
                ));
                orb.spawn((
                    PointLight {
                        color: reward_color(milestone.reward),
                        intensity: 170.0,
                        range: 4.0,
                        shadow_maps_enabled: false,
                        ..default()
                    },
                    Transform::default(),
                ));
            });
    }
    let Some(sites) = &world.expedition else {
        return;
    };
    for fountain in &progress.fountains {
        if pools
            .iter()
            .any(|(_, v, _)| v.name == fountain.name && v.generation == reset.generation)
        {
            continue;
        }
        let Some(volume) = sites.fountains.get(&fountain.name) else {
            continue;
        };
        let mut caps = BTreeMap::new();
        for pos in &volume.cells {
            let current = caps.entry(pos.coord).or_insert(*pos);
            if pos.level > current.level {
                *current = *pos;
            }
        }
        commands
            .spawn((
                PoolVisual {
                    name: fountain.name.clone(),
                    generation: reset.generation,
                },
                Name::new("Fountain charged water"),
                Transform::default(),
                if fountain.consumed {
                    Visibility::Hidden
                } else {
                    Visibility::Visible
                },
            ))
            .with_children(|pool| {
                for pos in caps.values() {
                    pool.spawn((
                        Mesh3d(assets.cap.clone()),
                        MeshMaterial3d(assets.pool.clone()),
                        Transform::from_translation(pos.coord.to_world(geometry.top(*pos) + 0.018))
                            .with_scale(Vec3::new(0.97, 0.012, 0.97)),
                        NotShadowCaster,
                        NotShadowReceiver,
                    ));
                }
            });
    }
}
