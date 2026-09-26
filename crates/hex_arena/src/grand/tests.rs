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
