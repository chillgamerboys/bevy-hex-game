use super::*;

#[test]
fn shrine_tuning_rejects_nonfinite_or_unbounded_values() {
    assert!(GrandTuning::default().validate().is_ok());
    let mut tuning = GrandTuning::default();
    tuning.earth_run = f32::NAN;
    assert!(tuning.validate().is_err());
    tuning.earth_run = 20.0;
    assert!(tuning.validate().is_err());
}

fn fixture() -> (ArenaSession, ArenaTerrainView, ArenaVoxelGeometry) {
    use hex_core::{HexCoord, SubstanceId, TilePos};
    let geometry = ArenaVoxelGeometry::default();
    let world = ArenaTerrainView {
        revision: 1,
        selection: hex_core::arena::ArenaSelection {
            map: hex_core::arena::ArenaMap::GrandV4,
            ..Default::default()
        },
        voxels: HexCoord::ORIGIN
            .within_radius(16)
            .into_iter()
            .map(|c| (TilePos::new(c, 0), SubstanceId(1)))
            .collect(),
        spawns: [Vec3::new(0.0, SKIN, 0.0), Vec3::new(8.0, SKIN, 0.0)],
        ..Default::default()
    };
    let mut session = ArenaSession::default();
    session.reset(4, &world, geometry);
    session.actors.truncate(1);
    session.actors[0].configure_expedition_player();
    (session, world, geometry)
}

#[test]
fn shrine_interaction_is_once_and_bonuses_stack_without_health() {
    let (mut session, mut world, geometry) = fixture();
    for shrine in ShrineId::ALL {
        world
            .anchors
            .insert(shrine.anchor().into(), session.actors[0].feet);
    }
    let base = session.player_tuning(&crate::ArenaTuning::default());
    let hp = session.actors[0].max_hp;
    session.advance_grand(ActorIntent::default(), &world, geometry);
    assert!(session.grand_progress().unwrap().shrines.is_empty());
    assert_eq!(
        session.grand_progress().unwrap().discovered_shrines.len(),
        5
    );
    session.advance_grand(
        ActorIntent {
            interact: true,
            ..Default::default()
        },
        &world,
        geometry,
    );
    let first = session.player_tuning(&crate::ArenaTuning::default());
    session.advance_grand(
        ActorIntent {
            interact: true,
            ..Default::default()
        },
        &world,
        geometry,
    );
    let second = session.player_tuning(&crate::ArenaTuning::default());
    assert_eq!(session.grand_progress().unwrap().shrines.len(), 5);
    assert!(first.fireball_damage > base.fireball_damage);
    assert!(first.projectile_speed > base.projectile_speed);
    assert!(first.high_jump_height > base.high_jump_height);
    assert_eq!(first.fireball_damage, second.fireball_damage);
    assert_eq!(session.actors[0].max_hp, hp);
    assert_eq!(
        session.player_fireball_mode(),
        crate::FireballMode::Explosive
    );
    assert!(session.earth_construction(0));
    assert!(!session.earth_construction(1));
    assert!(
        session.actors[0].glider_scale > 1.0
            && session.actors[0].swim_scale > 1.0
            && session.actors[0].boat_scale > 1.0
    );
}

#[test]
fn checkpoint_round_trip_preserves_charged_input_projectile_and_clocks() {
    let (mut session, world, geometry) = fixture();
    let tuning = crate::ArenaTuning::default();
    let identity = GrandCheckpointIdentity {
        world_id: "grand-test".into(),
        content_revision: "accepted-content".into(),
    };
    session.tick = 907;
    session.actors[0].charge = Some(crate::ChargeState {
        spell: crate::Spell::Fireball,
        elapsed: 0.45,
    });
    session.actors[0].cooldowns = [0.2, 0.3, 0.4];
    session
        .grand
        .as_mut()
        .unwrap()
        .acquired
        .insert(ShrineId::Water);
    session.release(
        0,
        crate::Spell::Shield,
        &tuning,
        10.0,
        &world,
        geometry,
        hex_core::arena::ArenaMaterials {
            stone: hex_core::SubstanceId(1),
            reinforced_stone: Some(hex_core::SubstanceId(6)),
            bedrock: hex_core::SubstanceId(2),
            grass: hex_core::SubstanceId(3),
            dirt: hex_core::SubstanceId(4),
            fire: hex_core::ElementId(1),
        },
        &mut crate::CommandsOut::default(),
    );
    let bytes = session.encode_grand_checkpoint(&identity).unwrap();
    let restored =
        ArenaSession::decode_grand_checkpoint(&bytes, &identity, &world, geometry, 4).unwrap();
    assert_eq!(restored.tick, 907);
    assert_eq!(restored.actors[0].charge().unwrap().elapsed, 0.45);
    assert_eq!(restored.projectiles.len(), session.projectiles.len());
    assert!(!restored.projectiles.is_empty());
    assert_eq!(bytes, restored.encode_grand_checkpoint(&identity).unwrap());
    assert!(ArenaSession::decode_grand_checkpoint(
        &bytes,
        &GrandCheckpointIdentity {
            content_revision: "different".into(),
            ..identity
        },
        &world,
        geometry,
        4
    )
    .is_err());
}

#[test]
fn teleport_requires_visible_clear_ground_and_only_success_spends_cooldown() {
    let (mut session, world, geometry) = fixture();
    session.grand.as_mut().unwrap().teleport_unlocked = true;
    session.actors[0].aim = Vec3::Y;
    let before = session.actors[0].feet;
    session.advance_grand(
        ActorIntent {
            teleport: true,
            aim: session.actors[0].aim,
            ..Default::default()
        },
        &world,
        geometry,
    );
    assert_eq!(session.actors[0].feet, before);
    assert_eq!(session.grand_progress().unwrap().teleport_cooldown, 0.0);
    session.actors[0].aim = (Vec3::X * 6.0 - Vec3::Y * session.actors[0].eye().y).normalize();
    session.advance_grand(
        ActorIntent {
            teleport: true,
            aim: session.actors[0].aim,
            ..Default::default()
        },
        &world,
        geometry,
    );
    assert!(session.actors[0].feet.x > 5.0);
    assert_eq!(session.grand_progress().unwrap().teleport_cooldown, 6.0);
}

#[test]
fn death_respawns_without_erasing_blessings_or_enemy_health() {
    let (mut session, world, geometry) = fixture();
    session
        .grand
        .as_mut()
        .unwrap()
        .acquired
        .insert(ShrineId::Earth);
    let mut enemy = Actor::spawn(417, Vec3::new(8.0, SKIN, 0.0), Vec3::NEG_X);
    enemy.hp = 37.0;
    session.actors.push(enemy);
    session.actors[0].hp = 0.0;
    session.actors[0].feet = Vec3::new(5.0, SKIN, 0.0);
    session.advance_grand(ActorIntent::default(), &world, geometry);
    assert!(session.actors[0].hp > 0.0);
    assert_eq!(session.actors[1].hp, 37.0);
    assert_eq!(session.grand_progress().unwrap().deaths, 1);
    assert!(session.actors[0].free_flight.is_none());
    assert!(session.earth_construction(0));
}

#[test]
fn configuration_before_reset_survives_new_run() {
    let (mut session, world, geometry) = fixture();
    let mut tuning = GrandTuning::default();
    tuning.earth_run = 1.4;
    session.configure_grand(tuning).unwrap();
    session.reset(5, &world, geometry);
    assert_eq!(session.grand.as_ref().unwrap().tuning.earth_run, 1.4);
    assert!(session.grand_progress().unwrap().shrines.is_empty());
}

#[test]
fn checkpoint_rejects_nonfinite_nested_movement_state() {
    let (mut session, _, _) = fixture();
    let identity = GrandCheckpointIdentity {
        world_id: "grand-test".into(),
        content_revision: "accepted-content".into(),
    };
    session.actors[0].body.impulse_velocity = Vec3::splat(f32::NAN);
    assert!(session.encode_grand_checkpoint(&identity).is_err());
}

#[test]
fn grand_upgrade_previews_include_cumulative_shrine_bonuses() {
    let (mut session, _, _) = fixture();
    session
        .grand
        .as_mut()
        .unwrap()
        .acquired
        .extend(ShrineId::ALL);
    let tuning = session.player_tuning(&crate::ArenaTuning::default());
    let preview = session
        .upgrade_preview(crate::UpgradeStat::FireballDamage)
        .unwrap();
    assert_eq!(
        preview.before,
        crate::UpgradeValue::Scalar(tuning.fireball_damage)
    );
    let preview = session
        .upgrade_preview(crate::UpgradeStat::ShieldSize)
        .unwrap();
    let size = tuning.player_profile.unwrap().shield_dimensions;
    assert_eq!(
        preview.before,
        crate::UpgradeValue::Dimensions(size.0, size.1)
    );
}

#[test]
fn dead_checkpoint_requests_last_shrine_instead_of_outside_world_corpse() {
    let (mut session, _, _) = fixture();
    let identity = GrandCheckpointIdentity {
        world_id: "grand-test".into(),
        content_revision: "accepted-content".into(),
    };
    let respawn = Vec3::new(14.0, 1.0, 5.0);
    session.grand.as_mut().unwrap().respawn_position = respawn;
    session.actors[0].hp = 0.0;
    session.actors[0].feet = Vec3::new(0.0, -1000.0, 0.0);
    let bytes = session.encode_grand_checkpoint(&identity).unwrap();
    assert_eq!(
        ArenaSession::grand_checkpoint_position(&bytes, &identity).unwrap(),
        respawn
    );
    assert!(session.grand_actor_interests().contains(&respawn));
}

#[test]
fn rebind_rejects_embedded_player_but_accepts_airborne_checkpoint() {
    let (mut session, mut world, geometry) = fixture();
    session.actors[0].feet = Vec3::Y * 12.0;
    assert!(session.rebind_grand_terrain(&world, geometry, 1).is_ok());
    let feet = session.actors[0].feet;
    let cell = geometry.voxel_at(feet + Vec3::Y * 0.2).unwrap();
    world.voxels.insert(cell, hex_core::SubstanceId(1));
    world.revision += 1;
    world.full_rebuild = true;
    assert!(session
        .rebind_grand_terrain(&world, geometry, 1)
        .unwrap_err()
        .contains("embedded actor"));
}

#[test]
fn checkpoint_rejects_fabricated_actor_and_missing_party_member() {
    let (mut session, _, _) = fixture();
    let identity = GrandCheckpointIdentity {
        world_id: "grand-test".into(),
        content_revision: "accepted-content".into(),
    };
    session
        .grand
        .as_mut()
        .unwrap()
        .admitted
        .insert("grand_shadow_tunnel".into());
    assert!(session
        .encode_grand_checkpoint(&identity)
        .unwrap_err()
        .contains("missing encounter"));
    session.grand.as_mut().unwrap().admitted.clear();
    session
        .actors
        .push(Actor::spawn(9999, Vec3::X * 3.0, Vec3::X));
    assert!(session
        .encode_grand_checkpoint(&identity)
        .unwrap_err()
        .contains("unauthored actor"));
}

#[test]
fn all_120_shrine_orders_have_identical_cumulative_effects_at_every_prefix() {
    fn visit(order: &mut [ShrineId; 5], index: usize, count: &mut usize) {
        if index < order.len() {
            for next in index..order.len() {
                order.swap(index, next);
                visit(order, index + 1, count);
                order.swap(index, next);
            }
            return;
        }
        *count += 1;
        let (mut session, mut world, geometry) = fixture();
        let base = session.player_tuning(&crate::ArenaTuning::default());
        let base_profile = base.player_profile.unwrap();
        let tuning = GrandTuning::default();
        let hp = (session.actors[0].hp, session.actors[0].max_hp);
        let mut acquired = BTreeSet::new();
        for shrine in order.iter().copied() {
            world.anchors.clear();
            world
                .anchors
                .insert(shrine.anchor().into(), session.actors[0].feet);
            acquired.insert(shrine);
            // The second interaction must not stack the same reward again.
            for _ in 0..2 {
                session.advance_grand(
                    ActorIntent {
                        interact: true,
                        ..Default::default()
                    },
                    &world,
                    geometry,
                );
                let actual = session.player_tuning(&crate::ArenaTuning::default());
                let profile = actual.player_profile.unwrap();
                let factor = |s, amount| if acquired.contains(&s) { amount } else { 1.0 };
                assert_eq!(
                    actual.fireball_damage,
                    base.fireball_damage * factor(ShrineId::Fire, tuning.fire_damage)
                );
                assert_eq!(
                    actual.projectile_speed,
                    base.projectile_speed * factor(ShrineId::Air, tuning.air_projectiles)
                );
                assert_eq!(
                    profile.shield_projectile_speed,
                    base_profile.shield_projectile_speed
                        * factor(ShrineId::Air, tuning.air_projectiles)
                );
                assert_eq!(
                    profile.fireball_radius,
                    base_profile.fireball_radius
                        * factor(ShrineId::Fire, tuning.fire_size)
                        * factor(ShrineId::Plant, tuning.plant_size)
                );
                assert_eq!(
                    actual.high_jump_height,
                    base.high_jump_height * factor(ShrineId::Plant, tuning.plant_jump)
                );
                assert_eq!(
                    profile.walking_speed,
                    base_profile.walking_speed * factor(ShrineId::Earth, tuning.earth_run)
                );
                assert_eq!(
                    session.actors[0].jump_scale,
                    factor(ShrineId::Plant, tuning.plant_jump)
                );
                assert_eq!(
                    session.actors[0].glider_scale,
                    factor(ShrineId::Air, tuning.air_glider)
                );
                assert_eq!(
                    session.actors[0].swim_scale,
                    factor(ShrineId::Water, tuning.water_swim)
                );
                assert_eq!(
                    session.actors[0].boat_scale,
                    factor(ShrineId::Water, tuning.water_boat)
                );
                assert_eq!(
                    session.earth_construction(0),
                    acquired.contains(&ShrineId::Earth)
                );
                assert!(!session.earth_construction(1));
                assert_eq!(
                    session.player_fireball_mode() == crate::FireballMode::Explosive,
                    acquired.contains(&ShrineId::Fire)
                );
                assert_eq!((session.actors[0].hp, session.actors[0].max_hp), hp);
                assert_eq!(
                    session.grand_progress().unwrap().shrines.len(),
                    acquired.len()
                );
                assert_eq!(
                    session.grand_progress().unwrap().respawn_anchor,
                    Some(shrine)
                );
                if acquired.contains(&ShrineId::Plant) {
                    assert!(profile.shield_dimensions.0 > base_profile.shield_dimensions.0);
                    assert!(profile.shield_dimensions.1 > base_profile.shield_dimensions.1);
                } else {
                    assert_eq!(profile.shield_dimensions, base_profile.shield_dimensions);
                }
            }
        }
    }
    let mut count = 0;
    let mut order = ShrineId::ALL;
    visit(&mut order, 0, &mut count);
    assert_eq!(count, 120);
}

#[test]
fn teleport_uses_this_input_aim_crosses_gaps_and_obeys_cooldown() {
    let (mut session, mut world, geometry) = fixture();
    session.grand.as_mut().unwrap().teleport_unlocked = true;
    world.voxels.retain(|cell, _| {
        let x = cell.coord.to_world(0.0).x;
        !(1.5..4.5).contains(&x)
    });
    world.revision += 1;
    world.full_rebuild = true;
    session.collision.refresh(&world, geometry);
    session.actors[0].aim = Vec3::Y; // Deliberately stale, pointing away from ground.
    let aim = (Vec3::X * 6.0 - session.actors[0].eye()).normalize();
    session.advance_grand(
        ActorIntent {
            teleport: true,
            aim,
            ..Default::default()
        },
        &world,
        geometry,
    );
    let target = session.actors[0].feet;
    assert!(
        target.x > 5.0,
        "visible support across the gap must be reachable"
    );
    assert_eq!(session.grand_progress().unwrap().teleport_cooldown, 6.0);
    session.advance_grand(
        ActorIntent {
            teleport: true,
            aim: (Vec3::X * 9.0 - session.actors[0].eye()).normalize(),
            ..Default::default()
        },
        &world,
        geometry,
    );
    assert_eq!(session.actors[0].feet, target);
    assert!((session.grand_progress().unwrap().teleport_cooldown - (6.0 - STEP)).abs() < 1e-6);
}

#[test]
fn teleport_refuses_wall_water_occupied_unloaded_and_out_of_range_ground() {
    use hex_core::arena::{ArenaResidency, ArenaSolidSpan};
    use hex_core::{HexCoord, SubstanceId, TilePos};
    for scenario in ["wall", "water", "occupied", "unloaded", "range", "locked"] {
        let (mut session, mut world, geometry) = fixture();
        session.grand.as_mut().unwrap().teleport_unlocked = scenario != "locked";
        let aim = (Vec3::X * if scenario == "range" { 13.0 } else { 6.0 }
            - session.actors[0].eye())
        .normalize();
        session.actors[0].aim = aim;
        let target = teleport_target(
            &session.actors[0],
            &session.actors,
            &session.collision,
            &world,
            geometry,
        );
        match scenario {
            "wall" => {
                for coord in HexCoord::from_world(Vec3::X * 3.0).within_radius(1) {
                    for level in 1..=6 {
                        world
                            .voxels
                            .insert(TilePos::new(coord, level), SubstanceId(1));
                    }
                }
            }
            "water" => world.liquids.push(ArenaSolidSpan {
                bottom: TilePos::new(HexCoord::from_world(target.unwrap()), 1),
                top_level: 3,
                substance: SubstanceId(5),
            }),
            "occupied" => session
                .actors
                .push(Actor::spawn(99, target.unwrap(), Vec3::X)),
            "unloaded" => {
                world.residency = Some(ArenaResidency {
                    catalogue: [(-1, -1), (-1, 0), (0, -1), (0, 0)].into(),
                    ready: Default::default(),
                })
            }
            _ => {}
        }
        world.revision += 1;
        world.full_rebuild = true;
        session.collision.refresh(&world, geometry);
        let original = session.actors[0].feet;
        session.advance_grand(
            ActorIntent {
                teleport: true,
                aim,
                ..Default::default()
            },
            &world,
            geometry,
        );
        assert_eq!(session.actors[0].feet, original, "{scenario}");
        assert_eq!(
            session.grand_progress().unwrap().teleport_cooldown,
            0.0,
            "{scenario}"
        );
    }
}

#[test]
fn grand_dragon_and_shadow_deaths_grant_xp_and_teleport_without_forest_rewards() {
    use crate::{EncounterTuning, ExpeditionRole, FireballMode};
    let (mut session, world, geometry) = fixture();
    let hp = (session.actors[0].hp, session.actors[0].max_hp);
    for (id, role) in [
        (225, ExpeditionRole::Dragon),
        (226, ExpeditionRole::Dragon),
        (227, ExpeditionRole::Dragon),
        (417, ExpeditionRole::MountainShadow),
    ] {
        let mut actor = Actor::spawn(id, Vec3::X * 8.0, Vec3::NEG_X);
        actor.configure_expedition(role, &EncounterTuning::default());
        session.actors.push(actor);
    }
    session.register_forest_roster();
    session.register_expedition_sites(&Default::default());
    for id in [225, 226, 227, 417] {
        session.record_player_hit(0, id);
        session.actors.iter_mut().find(|a| a.id == id).unwrap().hp = 0.0;
    }
    session.reconcile_progression();
    session.advance_milestones(&world, geometry);
    session.advance_grand(ActorIntent::default(), &world, geometry);
    assert_eq!(session.progress().unwrap().total_xp, 160);
    assert_eq!(session.progress().unwrap().dragons_defeated, 3);
    assert_eq!(session.player_fireball_mode(), FireballMode::ContactOnly);
    assert!(session.grand_progress().unwrap().teleport_unlocked);
    assert_eq!((session.actors[0].hp, session.actors[0].max_hp), hp);
    assert!(!session.is_forest_run());
    assert!(session
        .expedition_progress()
        .unwrap()
        .milestones
        .iter()
        .all(|m| !m.collected && m.available_position.is_none()));
    // Reconciliation is idempotent and does not pay XP again.
    session.reconcile_progression();
    assert_eq!(session.progress().unwrap().total_xp, 160);
}

#[test]
fn checkpoint_settlement_waits_for_matching_ack_and_replay_never_ticks() {
    use hex_core::arena::{ArenaBurrowOutcome, ArenaBurrowRequest, ArenaBurrowResult};
    let (mut session, _, _) = fixture();
    session.tick = 678;
    session.pending_burrows.insert(
        417,
        ArenaBurrowRequest {
            generation: 4,
            actor: 417,
            sequence: 7,
            volume: Vec::new(),
        },
    );
    let mut ack = ArenaBurrowOutcome {
        generation: 4,
        actor: 417,
        sequence: 6,
        result: ArenaBurrowResult::Accepted {
            changed: Vec::new(),
        },
    };
    assert!(session
        .settle_checkpoint_outcomes(&[], &[ack.clone()])
        .is_err());
    assert_eq!(session.grand_pending_world_operations(), 1);
    ack.sequence = 7;
    session
        .settle_checkpoint_outcomes(&[], &[ack.clone()])
        .unwrap();
    assert_eq!(session.grand_pending_world_operations(), 0);
    session.settle_checkpoint_outcomes(&[], &[ack]).unwrap();
    assert_eq!(session.tick, 678);
}

#[test]
fn grand_glide_start_does_not_enable_exploration_powered_flight() {
    let (mut session, world, geometry) = fixture();
    assert!(session.start_exploration_glide(Vec3::Y * 12.0, Vec3::X, &world, geometry));
    assert!(session.actors[0].glider().unwrap().open);
    assert!(session.actors[0].free_flight.is_none());
}

#[test]
fn grand_initialization_never_grants_powered_exploration_flight() {
    let (mut session, mut world, geometry) = fixture();
    world.expedition = Some(Default::default());
    session.advance(
        ActorIntent::default(),
        &world,
        geometry,
        hex_core::arena::ArenaMaterials {
            stone: hex_core::SubstanceId(1),
            reinforced_stone: None,
            bedrock: hex_core::SubstanceId(2),
            grass: hex_core::SubstanceId(3),
            dirt: hex_core::SubstanceId(4),
            fire: hex_core::ElementId(1),
        },
        &crate::ArenaTuning::default(),
    );
    assert!(session.encounter.initialized);
    assert!(session.actors[0].free_flight.is_none());
}

#[test]
fn teleport_rejects_implicit_ocean_above_visible_seabed() {
    use hex_core::ocean::{
        OceanEnvironmentSampler, OceanEnvironmentView, OceanSurfaceSample, OceanWaterColumn,
    };
    #[derive(Debug)]
    struct Ocean;
    impl OceanEnvironmentSampler for Ocean {
        fn inundation_column_at(&self, _: bevy_math::Vec2) -> Option<OceanWaterColumn> {
            Some(OceanWaterColumn {
                mean_height: 2.0,
                bed_height: 0.0,
                water_id: hex_core::SubstanceId(5),
            })
        }
        fn surface_at(
            &self,
            _: bevy_math::Vec2,
            _: f32,
            column: OceanWaterColumn,
        ) -> Option<OceanSurfaceSample> {
            Some(OceanSurfaceSample {
                height: 2.0,
                normal: Vec3::Y,
                vertical_velocity: 0.0,
                mean_height: column.mean_height,
                bed_height: column.bed_height,
                water_id: column.water_id,
            })
        }
    }
    let (mut session, world, geometry) = fixture();
    session.grand.as_mut().unwrap().teleport_unlocked = true;
    session.ocean_environment = Some(OceanEnvironmentView {
        package_fingerprint: 1,
        sampler: std::sync::Arc::new(Ocean),
        wind: Default::default(),
    });
    let before = session.actors[0].feet;
    let aim = (Vec3::X * 6.0 - session.actors[0].eye()).normalize();
    assert!(world.liquids.is_empty());
    session.advance_grand(
        ActorIntent {
            teleport: true,
            aim,
            ..Default::default()
        },
        &world,
        geometry,
    );
    assert_eq!(session.actors[0].feet, before);
    assert_eq!(session.grand_progress().unwrap().teleport_cooldown, 0.0);
}
