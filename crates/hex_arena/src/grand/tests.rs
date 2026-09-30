use super::*;

fn player(session: &ArenaSession) -> &Actor {
    session
        .actors
        .iter()
        .find(|actor| actor.id == 0)
        .expect("Grand player")
}

fn player_mut(session: &mut ArenaSession) -> &mut Actor {
    session
        .actors
        .iter_mut()
        .find(|actor| actor.id == 0)
        .expect("Grand player")
}

#[track_caller]
fn assert_exact_f32(actual: f32, expected: f32) {
    // These equalities require bit-preserving saves or idempotent blessings.
    assert_eq!(
        actual.to_bits(),
        expected.to_bits(),
        "{actual} != {expected}"
    );
}

#[test]
fn shrine_tuning_rejects_nonfinite_or_unbounded_values() {
    assert!(GrandTuning::default().validate().is_ok());
    let mut tuning = GrandTuning {
        earth_run: f32::NAN,
        ..Default::default()
    };
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
    player_mut(&mut session).configure_expedition_player();
    (session, world, geometry)
}

#[test]
fn shrine_interaction_is_once_and_bonuses_stack_without_health() {
    let (mut session, mut world, geometry) = fixture();
    for shrine in ShrineId::ALL {
        world
            .anchors
            .insert(shrine.anchor().into(), player(&session).feet);
    }
    let base = session.player_tuning(&crate::ArenaTuning::default());
    let hp = player(&session).max_hp;
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
    assert_exact_f32(first.fireball_damage, second.fireball_damage);
    assert_exact_f32(player(&session).max_hp, hp);
    assert_eq!(
        session.player_fireball_mode(),
        crate::FireballMode::Explosive
    );
    assert!(session.earth_construction(0));
    assert!(!session.earth_construction(1));
    assert!(
        player(&session).glider_scale > 1.0
            && player(&session).swim_scale > 1.0
            && player(&session).boat_scale > 1.0
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
    player_mut(&mut session).charge = Some(crate::ChargeState {
        spell: crate::Spell::Fireball,
        elapsed: 0.45,
    });
    player_mut(&mut session).cooldowns = [0.2, 0.3, 0.4];
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
    assert_exact_f32(player(&restored).charge().unwrap().elapsed, 0.45);
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
    player_mut(&mut session).aim = Vec3::Y;
    let before = player(&session).feet;
    session.advance_grand(
        ActorIntent {
            teleport: true,
            aim: player(&session).aim,
            ..Default::default()
        },
        &world,
        geometry,
    );
    assert_eq!(player(&session).feet, before);
    assert_exact_f32(session.grand_progress().unwrap().teleport_cooldown, 0.0);
    player_mut(&mut session).aim = (Vec3::X * 6.0 - Vec3::Y * player(&session).eye().y).normalize();
    session.advance_grand(
        ActorIntent {
            teleport: true,
            aim: player(&session).aim,
            ..Default::default()
        },
        &world,
        geometry,
    );
    assert!(player(&session).feet.x > 5.0);
    assert_exact_f32(session.grand_progress().unwrap().teleport_cooldown, 6.0);
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
    player_mut(&mut session).hp = 0.0;
    player_mut(&mut session).feet = Vec3::new(5.0, SKIN, 0.0);
    session.advance_grand(ActorIntent::default(), &world, geometry);
    assert!(player(&session).hp > 0.0);
    assert_exact_f32(session.actors.get(1).expect("retained enemy").hp, 37.0);
    assert_eq!(session.grand_progress().unwrap().deaths, 1);
    assert!(player(&session).free_flight.is_none());
    assert!(session.earth_construction(0));
}

#[test]
fn configuration_before_reset_survives_new_run() {
    let (mut session, world, geometry) = fixture();
    let tuning = GrandTuning {
        earth_run: 1.4,
        ..Default::default()
    };
    session.configure_grand(tuning).unwrap();
    session.reset(5, &world, geometry);
    assert_exact_f32(session.grand.as_ref().unwrap().tuning.earth_run, 1.4);
    assert!(session.grand_progress().unwrap().shrines.is_empty());
}

#[test]
fn checkpoint_rejects_nonfinite_nested_movement_state() {
    let (mut session, _, _) = fixture();
    let identity = GrandCheckpointIdentity {
        world_id: "grand-test".into(),
        content_revision: "accepted-content".into(),
    };
    player_mut(&mut session).body.impulse_velocity = Vec3::splat(f32::NAN);
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
    player_mut(&mut session).hp = 0.0;
    player_mut(&mut session).feet = Vec3::new(0.0, -1000.0, 0.0);
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
    player_mut(&mut session).feet = Vec3::Y * 12.0;
    assert!(session.rebind_grand_terrain(&world, geometry, 1).is_ok());
    let feet = player(&session).feet;
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
        let hp = (player(&session).hp, player(&session).max_hp);
        let mut acquired = BTreeSet::new();
        for shrine in order.iter().copied() {
            world.anchors.clear();
            world
                .anchors
                .insert(shrine.anchor().into(), player(&session).feet);
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
                assert_exact_f32(
                    actual.fireball_damage,
                    base.fireball_damage * factor(ShrineId::Fire, tuning.fire_damage),
                );
                assert_exact_f32(
                    actual.projectile_speed,
                    base.projectile_speed * factor(ShrineId::Air, tuning.air_projectiles),
                );
                assert_exact_f32(
                    profile.shield_projectile_speed,
                    base_profile.shield_projectile_speed
                        * factor(ShrineId::Air, tuning.air_projectiles),
                );
                assert_exact_f32(
                    profile.fireball_radius,
                    base_profile.fireball_radius
                        * factor(ShrineId::Fire, tuning.fire_size)
                        * factor(ShrineId::Plant, tuning.plant_size),
                );
                assert_exact_f32(
                    actual.high_jump_height,
                    base.high_jump_height * factor(ShrineId::Plant, tuning.plant_jump),
                );
                assert_exact_f32(
                    profile.walking_speed,
                    base_profile.walking_speed * factor(ShrineId::Earth, tuning.earth_run),
                );
                assert_exact_f32(
                    player(&session).jump_scale,
                    factor(ShrineId::Plant, tuning.plant_jump),
                );
                assert_exact_f32(
                    player(&session).glider_scale,
                    factor(ShrineId::Air, tuning.air_glider),
                );
                assert_exact_f32(
                    player(&session).swim_scale,
                    factor(ShrineId::Water, tuning.water_swim),
                );
                assert_exact_f32(
                    player(&session).boat_scale,
                    factor(ShrineId::Water, tuning.water_boat),
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
                assert_eq!((player(&session).hp, player(&session).max_hp), hp);
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
    player_mut(&mut session).aim = Vec3::Y; // Deliberately stale, pointing away from ground.
    let aim = (Vec3::X * 6.0 - player(&session).eye()).normalize();
    session.advance_grand(
        ActorIntent {
            teleport: true,
            aim,
            ..Default::default()
        },
        &world,
        geometry,
    );
    let target = player(&session).feet;
    assert!(
        target.x > 5.0,
        "visible support across the gap must be reachable"
    );
    assert_exact_f32(session.grand_progress().unwrap().teleport_cooldown, 6.0);
    session.advance_grand(
        ActorIntent {
            teleport: true,
            aim: (Vec3::X * 9.0 - player(&session).eye()).normalize(),
            ..Default::default()
        },
        &world,
        geometry,
    );
    assert_eq!(player(&session).feet, target);
    assert!((session.grand_progress().unwrap().teleport_cooldown - (6.0 - STEP)).abs() < 1e-6);
}

#[test]
fn teleport_refuses_wall_water_occupied_unloaded_and_out_of_range_ground() {
    use hex_core::arena::{ArenaResidency, ArenaSolidSpan};
    use hex_core::{HexCoord, SubstanceId, TilePos};
    for scenario in ["wall", "water", "occupied", "unloaded", "range", "locked"] {
        let (mut session, mut world, geometry) = fixture();
        session.grand.as_mut().unwrap().teleport_unlocked = scenario != "locked";
        let aim = (Vec3::X * if scenario == "range" { 13.0 } else { 6.0 } - player(&session).eye())
            .normalize();
        player_mut(&mut session).aim = aim;
        let target = teleport_target(
            player(&session),
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
        let original = player(&session).feet;
        session.advance_grand(
            ActorIntent {
                teleport: true,
                aim,
                ..Default::default()
            },
            &world,
            geometry,
        );
        assert_eq!(player(&session).feet, original, "{scenario}");
        assert_eq!(
            session
                .grand_progress()
                .unwrap()
                .teleport_cooldown
                .to_bits(),
            0.0_f32.to_bits(),
            "{scenario}"
        );
    }
}

#[test]
fn grand_dragon_and_shadow_deaths_grant_xp_and_teleport_without_forest_rewards() {
    use crate::{EncounterTuning, ExpeditionRole, FireballMode};
    let (mut session, world, geometry) = fixture();
    let hp = (player(&session).hp, player(&session).max_hp);
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
    assert_eq!((player(&session).hp, player(&session).max_hp), hp);
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
    assert!(player(&session).glider().unwrap().open);
    assert!(player(&session).free_flight.is_none());
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
    assert!(player(&session).free_flight.is_none());
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
    let before = player(&session).feet;
    let aim = (Vec3::X * 6.0 - player(&session).eye()).normalize();
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
    assert_eq!(player(&session).feet, before);
    assert_exact_f32(session.grand_progress().unwrap().teleport_cooldown, 0.0);
}

fn checkpoint_materials() -> hex_core::arena::ArenaMaterials {
    hex_core::arena::ArenaMaterials {
        // Shipped compatibility slots: these do not depend on RON map order.
        stone: hex_core::SubstanceId(10),
        reinforced_stone: Some(hex_core::SubstanceId(18)),
        bedrock: hex_core::SubstanceId(2),
        grass: hex_core::SubstanceId(4),
        dirt: hex_core::SubstanceId(3),
        fire: hex_core::ElementId(1),
    }
}

fn cast_construction(
    session: &mut ArenaSession,
    world: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    aim: Vec3,
) -> Vec<hex_core::TerrainEdit> {
    let tuning = crate::ArenaTuning::default();
    let materials = checkpoint_materials();
    for _ in 0..720 {
        if player(session)
            .cooldowns
            .get(crate::Spell::Shield.index())
            .is_some_and(|c| *c <= 0.0)
        {
            break;
        }
        let out = session.advance(ActorIntent::default(), world, geometry, materials, &tuning);
        assert!(out.edits.is_empty() && out.impacts.is_empty());
    }
    let mut edits = Vec::new();
    for tick in 0..240 {
        let out = session.advance(
            ActorIntent {
                aim,
                selected: Some(crate::Spell::Shield),
                cast_pressed: tick == 0,
                cast_held: tick < 12,
                cast_released: tick == 12,
                ..Default::default()
            },
            world,
            geometry,
            materials,
            &tuning,
        );
        assert!(out.impacts.is_empty(), "Shield must not emit damage");
        edits.extend(out.edits);
        if !edits.is_empty() {
            break;
        }
    }
    edits
        .first()
        .expect("a real Shield impact must emit terrain");
    assert!(session.projectiles.is_empty() && session.pending_walls.is_empty());
    edits
}

#[test]
fn earth_blesses_only_new_shield_materials_without_changing_existing_stone_or_health() {
    #[derive(Deserialize)]
    struct Substance {
        toughness: Option<u8>,
        solid: bool,
        conjurable: bool,
    }
    #[derive(Deserialize)]
    struct Catalogue {
        substances: std::collections::BTreeMap<String, Substance>,
    }
    let catalogue: Catalogue =
        ron::from_str(include_str!("../../../../assets/config/substances.ron"))
            .expect("the shipped material definitions");
    let ordinary = catalogue.substances.get("stone").expect("ordinary stone");
    let reinforced = catalogue
        .substances
        .get("reinforced_stone")
        .expect("Earth construction material");
    assert!(ordinary.solid && ordinary.conjurable && reinforced.solid && reinforced.conjurable);
    assert_eq!(ordinary.toughness, Some(4));
    assert_eq!(reinforced.toughness, Some(8));

    let (mut session, mut world, geometry) = fixture();
    world.expedition = Some(Default::default());
    session.bot_enabled = false;
    let health = (player(&session).hp, player(&session).max_hp);
    let materials = checkpoint_materials();
    let old = cast_construction(
        &mut session,
        &world,
        geometry,
        (Vec3::X - Vec3::Y * 0.35).normalize(),
    );
    let mut old_cells = std::collections::BTreeMap::new();
    for edit in old {
        let hex_core::TerrainEdit::Set { pos, substance } = edit else {
            panic!("Shield must create material, never clear it");
        };
        assert_eq!(
            substance, materials.stone,
            "pre-Earth material has toughness4"
        );
        assert!(world.voxels.insert(pos, substance).is_none());
        old_cells.insert(pos, substance);
    }
    world.revision += 1;
    world
        .anchors
        .insert(ShrineId::Earth.anchor().into(), player(&session).feet);
    let blessing = session.advance(
        ActorIntent {
            interact: true,
            ..Default::default()
        },
        &world,
        geometry,
        materials,
        &crate::ArenaTuning::default(),
    );
    assert!(session.earth_construction(0));
    assert!(
        blessing.edits.is_empty(),
        "claiming Earth does not rewrite old terrain"
    );
    let new = cast_construction(
        &mut session,
        &world,
        geometry,
        (Vec3::Z - Vec3::Y * 0.35).normalize(),
    );
    for edit in new {
        let hex_core::TerrainEdit::Set { pos, substance } = edit else {
            panic!("Shield must create material, never clear it");
        };
        assert_eq!(
            Some(substance),
            materials.reinforced_stone,
            "post-Earth material has toughness8"
        );
        assert!(!old_cells.contains_key(&pos));
        assert!(world.voxels.insert(pos, substance).is_none());
    }
    assert_eq!(session.shields_raised, 2);
    for (pos, substance) in old_cells {
        assert_eq!(world.voxels.get(&pos), Some(&substance));
        assert_eq!(substance, materials.stone);
    }
    assert_exact_f32(player(&session).hp, health.0);
    assert_exact_f32(player(&session).max_hp, health.1);
}

#[derive(Debug, Deserialize)]
struct AbilityCheckpointProbe {
    session: AbilitySessionProbe,
}
#[derive(Debug, Deserialize)]
struct AbilitySessionProbe {
    encounter: AbilityEncounterProbe,
}
#[derive(Debug, Deserialize)]
struct AbilityEncounterProbe {
    brains: std::collections::BTreeMap<crate::ActorId, AbilityBrainProbe>,
}
#[derive(Debug, Deserialize)]
struct AbilityBrainProbe {
    active: Option<AbilityCastProbe>,
}
#[derive(Debug, Deserialize)]
struct AbilityCastProbe {
    kind: crate::CreatureAbility,
    pulses: u8,
    actor_damage: std::collections::BTreeMap<crate::ActorId, f32>,
    voxels: BTreeSet<hex_core::TilePos>,
}

fn cast_probe(bytes: &[u8], owner: crate::ActorId) -> AbilityCastProbe {
    let mut probe: AbilityCheckpointProbe =
        ron::de::from_bytes(bytes).expect("typed checkpoint probe");
    probe
        .session
        .encounter
        .brains
        .remove(&owner)
        .expect("saved enemy brain")
        .active
        .expect("saved active ability")
}

fn advance_over_bedrock(
    session: &mut ArenaSession,
    world: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
) -> crate::CommandsOut {
    use hex_core::{
        TerrainImpactDisposition, TerrainImpactOutcome, TerrainImpactResult, TerrainVoxelOutcome,
    };
    let materials = checkpoint_materials();
    let out = session.advance(
        ActorIntent::default(),
        world,
        geometry,
        materials,
        &crate::ArenaTuning::default(),
    );
    assert!(out.edits.is_empty() && out.burrows.is_empty());
    let acknowledged: Vec<_> = out
        .impacts
        .iter()
        .map(|impact| {
            let result = TerrainImpactResult::Applied(
                impact
                    .volume
                    .iter()
                    .map(|pos| {
                        assert_eq!(
                            world.solid_at(*pos),
                            Some(materials.bedrock),
                            "this fixture contains only protected ground"
                        );
                        TerrainVoxelOutcome {
                            pos: *pos,
                            disposition: TerrainImpactDisposition::Resisted,
                            before: Some(materials.bedrock),
                            after: Some(materials.bedrock),
                            health_before: None,
                            health_after: None,
                        }
                    })
                    .collect(),
            );
            let outcome = TerrainImpactOutcome {
                batch: impact.batch,
                result,
            };
            assert!(outcome.is_consistent_with(impact));
            outcome
        })
        .collect();
    session
        .settle_checkpoint_outcomes(&acknowledged, &[])
        .expect("settled protected ground");
    out
}

#[test]
fn real_enemy_windup_and_pulses_resume_without_replaying_damage_terrain_or_rewards() {
    use crate::{AttackPhase, CreatureAbility, Spell};
    use hex_core::arena::{ArenaDeploymentRegion, ArenaEncounterSite, ArenaExpeditionSites};
    use hex_core::{HexCoord, TilePos};
    let (mut session, mut world, geometry) = fixture();
    let materials = checkpoint_materials();
    world.voxels = HexCoord::ORIGIN
        .within_radius(40)
        .into_iter()
        .map(|coord| (TilePos::new(coord, 0), materials.bedrock))
        .collect();
    let home = HexCoord::from_axial(6, 0);
    world.expedition = Some(ArenaExpeditionSites {
        encounters: [(
            "grand_dragon_01".into(),
            ArenaEncounterSite {
                deployment: ArenaDeploymentRegion {
                    preferred: TilePos::new(home, 0),
                    surfaces: home
                        .within_radius(7)
                        .into_iter()
                        .map(|coord| TilePos::new(coord, 0))
                        .collect(),
                },
                rally_entry: None,
            },
        )]
        .into(),
        ..Default::default()
    });
    session.bot_enabled = false;
    advance_over_bedrock(&mut session, &world, geometry);
    assert_eq!(
        session.actors.iter().map(|a| a.id).collect::<BTreeSet<_>>(),
        BTreeSet::from([0, 225, 226, 227])
    );
    // Keep a living target throughout two unmodified Dragon bursts. This extra
    // fixture health is unrelated to shrine tuning and never changes after save.
    player_mut(&mut session).max_hp = 1000.0;
    player_mut(&mut session).hp = 1000.0;
    let player_eye = player(&session).eye();
    let victim_id = session
        .actors
        .iter()
        .filter(|a| a.id != 0)
        .min_by(|a, b| {
            a.center()
                .distance_squared(player_eye)
                .total_cmp(&b.center().distance_squared(player_eye))
        })
        .expect("nearest admitted Dragon")
        .id;
    let victim = session
        .actors
        .iter_mut()
        .find(|a| a.id == victim_id)
        .expect("authored Dragon");
    victim.hp = 1.0;
    let target = victim.center();
    player_mut(&mut session).aim = crate::bot::ballistic_aim(
        player(&session).eye(),
        target,
        &crate::ArenaTuning::default(),
        30.0,
    )
    .expect("reachable Dragon at the real projectile gravity")
    .0;
    session.release(
        0,
        Spell::Fireball,
        &crate::ArenaTuning::default(),
        30.0,
        &world,
        geometry,
        materials,
        &mut crate::CommandsOut::default(),
    );
    for _ in 0..240 {
        advance_over_bedrock(&mut session, &world, geometry);
        if session
            .actors
            .iter()
            .find(|a| a.id == victim_id)
            .expect("retained Dragon")
            .hp
            <= 0.0
        {
            break;
        }
    }
    assert!(
        session
            .actors
            .iter()
            .find(|a| a.id == victim_id)
            .expect("retained Dragon")
            .hp
            <= 0.0,
        "real Fireball must earn the prior reward: {:?}, projectiles={:?}",
        session
            .actors
            .iter()
            .map(|a| (a.id, a.hp, a.feet, a.team))
            .collect::<Vec<_>>(),
        session
            .projectiles
            .iter()
            .map(|p| (p.position, p.velocity))
            .collect::<Vec<_>>()
    );
    assert_eq!(session.progress().expect("Grand XP").total_xp, 20);
    assert_eq!(session.progress().expect("Grand XP").dragons_defeated, 1);
    session.bot_enabled = true;
    let mut owner = None;
    for _ in 0..1200 {
        advance_over_bedrock(&mut session, &world, geometry);
        owner = session
            .actors
            .iter()
            .find(|a| {
                a.attack_state().is_some_and(|s| {
                    s.kind == CreatureAbility::FireCone && s.phase == AttackPhase::Windup
                })
            })
            .map(|a| a.id);
        if owner.is_some() {
            break;
        }
    }
    let owner = owner.expect("ordinary Grand AI must reach a real FireCone windup");
    let identity = GrandCheckpointIdentity {
        world_id: "grand-test".into(),
        content_revision: "active-enemy-ability".into(),
    };
    let windup = session
        .encode_grand_checkpoint(&identity)
        .expect("settled windup save");
    assert_eq!(cast_probe(&windup, owner).pulses, 0);
    let mut windup_resumed =
        ArenaSession::decode_grand_checkpoint(&windup, &identity, &world, geometry, 4)
            .expect("windup restore");
    let hp_before_pulse = player(&session).hp;
    let mut first_pulse = None;
    for _ in 0..180 {
        let direct = advance_over_bedrock(&mut session, &world, geometry);
        let restored = advance_over_bedrock(&mut windup_resumed, &world, geometry);
        assert_eq!(
            direct.impacts, restored.impacts,
            "windup resume must emit the same actual terrain operations"
        );
        assert_exact_f32(player(&session).hp, player(&windup_resumed).hp);
        if session
            .actors
            .iter()
            .find(|a| a.id == owner)
            .and_then(Actor::attack_state)
            .is_some_and(|s| s.phase == AttackPhase::Active)
        {
            let bytes = session
                .encode_grand_checkpoint(&identity)
                .expect("settled active save");
            let cast = cast_probe(&bytes, owner);
            if cast.pulses == 1
                && cast
                    .actor_damage
                    .get(&0)
                    .is_some_and(|damage| *damage > 0.0)
                && !cast.voxels.is_empty()
            {
                first_pulse = Some((bytes, cast));
                break;
            }
        }
    }
    let (pulse_bytes, saved_cast) =
        first_pulse.expect("the real first pulse must hit the player and protected terrain");
    assert!(player(&session).hp < hp_before_pulse);
    assert_eq!(saved_cast.kind, CreatureAbility::FireCone);
    assert_eq!(saved_cast.pulses, 1);
    let mut pulse_resumed =
        ArenaSession::decode_grand_checkpoint(&pulse_bytes, &identity, &world, geometry, 4)
            .expect("active pulse restore");
    let reencoded = pulse_resumed
        .encode_grand_checkpoint(&identity)
        .expect("restored ledger");
    let restored_cast = cast_probe(&reencoded, owner);
    assert_eq!(restored_cast.pulses, saved_cast.pulses);
    assert_eq!(restored_cast.voxels, saved_cast.voxels);
    assert_exact_f32(
        *restored_cast.actor_damage.get(&0).expect("saved hit"),
        *saved_cast.actor_damage.get(&0).expect("real hit"),
    );
    let hp_at_save = player(&session).hp;
    let mut last_pulses = saved_cast.pulses;
    let mut finished = false;
    for _ in 0..120 {
        let direct = advance_over_bedrock(&mut session, &world, geometry);
        let from_windup = advance_over_bedrock(&mut windup_resumed, &world, geometry);
        let from_pulse = advance_over_bedrock(&mut pulse_resumed, &world, geometry);
        assert_eq!(direct.impacts, from_windup.impacts);
        assert_eq!(direct.impacts, from_pulse.impacts);
        if !finished {
            let caster = pulse_resumed
                .actors
                .iter()
                .find(|a| a.id == owner)
                .expect("restored caster");
            if caster.attack_state().is_some() {
                let bytes = pulse_resumed
                    .encode_grand_checkpoint(&identity)
                    .expect("continuing real cast");
                let cast = cast_probe(&bytes, owner);
                assert_eq!(cast.kind, CreatureAbility::FireCone);
                assert!((last_pulses..=3).contains(&cast.pulses));
                last_pulses = cast.pulses;
                assert!(saved_cast.voxels.is_subset(&cast.voxels));
                let admitted = *cast.actor_damage.get(&0).expect("retained player hit");
                assert!(admitted >= *saved_cast.actor_damage.get(&0).expect("saved hit"));
                assert!(admitted <= crate::ArenaTuning::default().encounters.breath_damage);
            } else {
                finished = true;
            }
        }
        assert_exact_f32(player(&session).hp, player(&windup_resumed).hp);
        assert_exact_f32(player(&session).hp, player(&pulse_resumed).hp);
        assert_eq!(session.tick, pulse_resumed.tick);
        for run in [&session, &windup_resumed, &pulse_resumed] {
            let progress = run.progress().expect("Grand XP");
            assert_eq!(
                progress.total_xp, 20,
                "a restored credited corpse must not grant XP again"
            );
            assert_eq!(progress.dragons_defeated, 1);
            assert_eq!(run.grand_progress().expect("Grand state").deaths, 0);
        }
    }
    assert!(
        player(&session).hp < hp_at_save,
        "remaining real breath pulses must still execute"
    );
    assert_eq!(
        last_pulses, 3,
        "the restored cast must finish all three pulses"
    );
    assert!(finished, "the resumed finite cast must finish");
    assert_eq!(
        pulse_resumed
            .encode_grand_checkpoint(&identity)
            .expect("completed resumed checkpoint"),
        session
            .encode_grand_checkpoint(&identity)
            .expect("completed uninterrupted checkpoint")
    );
}
