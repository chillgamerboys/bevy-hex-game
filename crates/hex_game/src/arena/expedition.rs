//! Visible milestone spheres and charged fountain water from public snapshots.

use bevy::asset::RenderAssetUsages;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::PrimitiveTopology;
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
    glimmer: Handle<StandardMaterial>,
}

#[derive(Component)]
pub(super) struct RewardVisual(ExpeditionReward, u64);

#[derive(Component)]
pub(super) struct PoolVisual {
    name: String,
    generation: u64,
}

#[derive(Component)]
pub(super) struct PoolGlimmer {
    origin: Vec3,
    phase: f32,
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
        cap: meshes.add(water_glow_cap()),
        gold: light(Color::srgb(1.0, 0.76, 0.19)),
        blue: light(Color::srgb(0.35, 0.83, 1.0)),
        violet: light(Color::srgb(0.82, 0.55, 1.0)),
        halo: light(Color::srgba(0.88, 0.96, 1.0, 0.10)),
        pool: light(Color::srgba(0.28, 1.0, 0.80, 0.38)),
        glimmer: light(Color::srgb(0.72, 1.0, 0.88)),
    });
}

/// One surface per liquid column. A closed translucent prism layers its top,
/// bottom and sides over the water and makes a shallow pool look like solid tiles.
fn water_glow_cap() -> Mesh {
    let corners = super::golem::CORNERS;
    let mut positions = Vec::with_capacity(18);
    for (a, b) in corners.into_iter().zip(corners.into_iter().cycle().skip(1)) {
        positions.extend([Vec3::ZERO.to_array(), a.to_array(), b.to_array()]);
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; 18])
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 0.0]; 18])
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
    mut glimmers: Query<(&PoolGlimmer, &mut Transform), Without<RewardVisual>>,
) {
    let progress = session.expedition_progress();
    // Tie animation to simulation time so pausing and deterministic captures freeze it.
    let cycle_tick = u16::try_from(session.tick % 480).unwrap_or_default();
    let phase = f32::from(cycle_tick) / 480.0 * std::f32::consts::TAU;
    for (glimmer, mut transform) in &mut glimmers {
        let drift = phase + glimmer.phase;
        transform.translation =
            glimmer.origin + Vec3::new(drift.cos() * 0.08, drift.sin() * 0.20, drift.sin() * 0.08);
    }
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
            transform.translation = Vec3::from_array(position) + Vec3::Y * phase.sin() * 0.08;
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
                for (index, pos) in caps.values().enumerate() {
                    pool.spawn((
                        Mesh3d(assets.cap.clone()),
                        MeshMaterial3d(assets.pool.clone()),
                        Transform::from_translation(pos.coord.to_world(geometry.top(*pos) + 0.025)),
                        NotShadowCaster,
                        NotShadowReceiver,
                    ));
                    // Small rising lights distinguish a charged spring from
                    // ordinary water without obscuring the animated surface.
                    // The common parent hides every glimmer when it is spent.
                    if index % 3 == 0 {
                        let phase = u16::try_from(index).map_or(0.0, f32::from) * 1.7;
                        let origin = pos
                            .coord
                            .to_world(geometry.top(*pos) + 0.55 + phase.sin().abs() * 0.55);
                        pool.spawn((
                            PoolGlimmer { origin, phase },
                            Transform::from_translation(origin),
                            Visibility::default(),
                        ))
                        .with_children(|glimmer| {
                            for (material, radius) in
                                [(&assets.glimmer, 0.065), (&assets.halo, 0.19)]
                            {
                                glimmer.spawn((
                                    Mesh3d(assets.sphere.clone()),
                                    MeshMaterial3d(material.clone()),
                                    Transform::from_scale(Vec3::splat(radius)),
                                    NotShadowCaster,
                                    NotShadowReceiver,
                                ));
                            }
                        });
                    }
                }
            });
    }
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    use hex_core::arena::{ArenaMap, ArenaSelection, ArenaTick};

    #[test]
    #[ignore = "requires HEX_FOREST_WORLD pointing at the compiled expedition and companion"]
    fn expedition_fountain_presentation_consumes_and_resets_all_charged_children() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(ArenaSelection {
                map: ArenaMap::ForestMassif,
                ..default()
            })
            .add_plugins((hex_map::arena::plugin, hex_arena::plugin));
        app.update();
        app.world_mut().resource_mut::<ArenaSession>().bot_enabled = false;
        app.world_mut().run_schedule(ArenaTick);
        app.init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<StandardMaterial>>();
        app.world_mut()
            .run_system_once(setup)
            .expect("visual assets");
        app.world_mut()
            .run_system_once(present)
            .expect("charged visuals");
        let initial = app
            .world_mut()
            .query::<(Entity, &PoolVisual, &Visibility)>()
            .iter(app.world())
            .map(|(entity, pool, visibility)| {
                assert_eq!(*visibility, Visibility::Visible);
                (pool.name.clone(), entity)
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(initial.len(), 6);
        let glimmers = app
            .world_mut()
            .query_filtered::<&ChildOf, With<PoolGlimmer>>()
            .iter(app.world())
            .map(ChildOf::parent)
            .collect::<Vec<_>>();
        assert_eq!(glimmers.len(), 42);
        assert!(
            glimmers
                .iter()
                .all(|parent| initial.values().any(|entity| entity == parent))
        );

        super::super::expedition_capture::stage(app.world_mut(), 20, "expedition-fountain-spent")
            .expect("wounded player enters actual fountain");
        app.world_mut().run_schedule(ArenaTick);
        app.world_mut()
            .run_system_once(present)
            .expect("spent visuals");
        for (name, entity) in &initial {
            assert_eq!(
                *app.world().get::<Visibility>(*entity).expect("pool parent"),
                if name == "forest_fountain_01" {
                    Visibility::Hidden
                } else {
                    Visibility::Visible
                },
                "only the consumed fountain loses its surface tint and lights",
            );
        }
        app.world_mut().resource_mut::<ArenaReset>().generation += 1;
        app.world_mut().run_schedule(ArenaTick);
        app.world_mut()
            .run_system_once(present)
            .expect("reset visuals");
        assert!(
            initial
                .values()
                .all(|entity| app.world().get_entity(*entity).is_err())
        );
        let mut query = app
            .world_mut()
            .query_filtered::<&Visibility, With<PoolVisual>>();
        let reset = query.iter(app.world()).collect::<Vec<_>>();
        assert_eq!(reset.len(), 6);
        assert!(
            reset
                .iter()
                .all(|visibility| **visibility == Visibility::Visible)
        );
    }
}
