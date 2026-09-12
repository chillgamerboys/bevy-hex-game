use super::*;

fn start() -> (
    ArenaSession,
    ArenaTerrainView,
    ArenaVoxelGeometry,
    ArenaMaterials,
    ArenaTuning,
) {
    let (mut session, view, geometry, materials, tuning) = fixture();
    session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
    (session, view, geometry, materials, tuning)
}

fn kill_role(session: &mut ArenaSession, role: ExpeditionRole, credit: bool) {
    let ids: Vec<_> = session
        .actors
        .iter()
        .filter(|a| a.expedition_role() == Some(role))
        .map(|a| a.id)
        .collect();
    for id in ids {
        defeat(session, id, credit);
    }
}

fn milestone(session: &ArenaSession, reward: ExpeditionReward) -> MilestoneSnapshot {
    *session
        .expedition_progress()
        .expect("expedition")
        .milestones
        .iter()
        .find(|m| m.reward == reward)
        .expect("milestone")
}

fn collect(
    session: &mut ArenaSession,
    reward: ExpeditionReward,
    view: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
) {
    let point = Vec3::from_array(
        milestone(session, reward)
            .available_position
            .expect("available orb"),
    );
    session.actors.first_mut().expect("player").feet = point - Vec3::Y * 0.6;
    session.advance_milestones(view, geometry);
    assert!(milestone(session, reward).collected);
}

#[test]
fn milestone_orders_are_independent_of_xp_and_shadow_never_heals() {
    for order in [
        [
            ExpeditionRole::Troll,
            ExpeditionRole::MountainShadow,
            ExpeditionRole::Dragon,
        ],
        [
            ExpeditionRole::Dragon,
            ExpeditionRole::MountainShadow,
            ExpeditionRole::Troll,
        ],
    ] {
        let (mut session, view, geometry, _, tuning) = start();
        session.actors.first_mut().expect("player").hp = 61.0;
        for role in order {
            let reward = match role {
                ExpeditionRole::Troll => ExpeditionReward::TrollDamage,
                ExpeditionRole::Dragon => ExpeditionReward::DragonExplosions,
                _ => ExpeditionReward::ShadowVitality,
            };
            kill_role(&mut session, role, false);
            session.advance_milestones(&view, geometry);
            let dropped = milestone(&session, reward);
            assert!(dropped.defeated && !dropped.collected && dropped.available_position.is_some());
            assert_eq!(session.progress().expect("progress").total_xp, 0);
            collect(&mut session, reward, &view, geometry);
            for _ in 0..3 {
                session.reconcile_progression();
                session.advance_milestones(&view, geometry);
            }
            assert!(milestone(&session, reward).available_position.is_none());
        }
        assert_eq!(
            session.player_tuning(&tuning).fireball_damage.to_bits(),
            40.0_f32.to_bits()
        );
        assert_eq!(
            session.player_tuning(&tuning).fireball_radius().to_bits(),
            2.5_f32.to_bits()
        );
        let player = session.actors.first().expect("player");
        assert_eq!(player.max_hp.to_bits(), 125.0_f32.to_bits());
        assert_eq!(player.hp.to_bits(), 61.0_f32.to_bits());
        assert_eq!(session.progress().expect("progress").forest_defeated, 0);
        assert!(!session.completed_run());
    }
}

#[test]
fn dragons_require_all_three_and_reward_orbs_require_proximity_sight_and_life() {
    let (mut session, mut view, geometry, _, _) = start();
    let ids: Vec<_> = session
        .actors
        .iter()
        .filter(|a| a.expedition_role() == Some(ExpeditionRole::Dragon))
        .map(|a| a.id)
        .collect();
    for id in ids.iter().take(2) {
        defeat(&mut session, *id, true);
    }
    session.advance_milestones(&view, geometry);
    assert!(!milestone(&session, ExpeditionReward::DragonExplosions).defeated);
    assert!(milestone(&session, ExpeditionReward::DragonExplosions)
        .available_position
        .is_none());
    defeat(&mut session, *ids.last().expect("third dragon"), true);
    session.advance_milestones(&view, geometry);
    let point = Vec3::from_array(
        milestone(&session, ExpeditionReward::DragonExplosions)
            .available_position
            .expect("orb"),
    );
    assert!(!session.progress().expect("progress").explosions_unlocked);
    session.actors.first_mut().expect("player").feet = point - Vec3::Y * 0.6;
    session.actors.first_mut().expect("player").hp = 0.0;
    session.advance_milestones(&view, geometry);
    assert!(!milestone(&session, ExpeditionReward::DragonExplosions).collected);
    session.actors.first_mut().expect("player").hp = 100.0;
    let level = geometry.voxel_at(point).expect("orb voxel");
    view.static_spans.push(hex_core::arena::ArenaStaticSpan {
        bottom: level,
        top_level: level.level,
        blocks_movement: false,
        blocks_sight: true,
        blocks_projectiles: false,
    });
    view.revision += 1;
    session.collision.refresh(&view, geometry);
    session.advance_milestones(&view, geometry);
    assert!(!milestone(&session, ExpeditionReward::DragonExplosions).collected);
    view.static_spans.clear();
    view.revision += 1;
    session.collision.refresh(&view, geometry);
    collect(
        &mut session,
        ExpeditionReward::DragonExplosions,
        &view,
        geometry,
    );
    assert!(session.can_upgrade(UpgradeStat::FireballSize));
}

#[test]
fn fall_reward_uses_nearest_admitted_supported_pose_and_revalidates_destroyed_footing() {
    let (mut session, mut view, geometry, _, _) = start();
    let troll = session
        .actors
        .iter_mut()
        .find(|a| a.expedition_role() == Some(ExpeditionRole::Troll))
        .expect("troll");
    troll.feet += Vec3::new(0.2, -100.0, 0.3);
    let origin = troll.feet;
    kill_role(&mut session, ExpeditionRole::Troll, false);
    session.advance_milestones(&view, geometry);
    let first = Vec3::from_array(
        milestone(&session, ExpeditionReward::TrollDamage)
            .available_position
            .expect("fall orb"),
    );
    let feet = first - Vec3::Y * 0.6;
    assert!(session.reward_standing_pose(feet, &view, geometry));
    let nearest = view
        .expedition
        .as_ref()
        .expect("sites")
        .encounters
        .values()
        .flat_map(|site| site.deployment.surfaces.iter())
        .map(|surface| surface.coord.to_world(geometry.top(*surface) + SKIN))
        .filter(|feet| session.reward_standing_pose(*feet, &view, geometry))
        .map(|feet| feet.distance_squared(origin))
        .min_by(f32::total_cmp)
        .expect("reachable support");
    assert!((feet.distance_squared(origin) - nearest).abs() < 0.01);
    let support = geometry
        .voxel_at(feet - Vec3::Y * SKIN * 2.0)
        .expect("support");
    view.voxels.remove(&support);
    view.revision += 1;
    session.collision.refresh(&view, geometry);
    session.advance_milestones(&view, geometry);
    let second = Vec3::from_array(
        milestone(&session, ExpeditionReward::TrollDamage)
            .available_position
            .expect("resettled orb"),
    );
    assert!(first.distance(second) > 0.1);
    assert!(session.reward_standing_pose(second - Vec3::Y * 0.6, &view, geometry));
}

#[test]
fn uncollected_final_rewards_survive_victory_and_reset_restores_every_reward() {
    let (mut session, view, geometry, materials, tuning) = start();
    let ids: Vec<_> = session.actors.iter().skip(1).map(|a| a.id).collect();
    for id in ids {
        defeat(&mut session, id, true);
    }
    session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
    assert!(session.completed_run() && !session.is_finished());
    assert_eq!(session.progress().expect("progress").total_xp, 327);
    assert!(session
        .expedition_progress()
        .expect("snapshot")
        .milestones
        .iter()
        .all(|m| m.defeated && !m.collected && m.available_position.is_some()));
    assert!(session.spend_upgrade(UpgradeStat::FireballDamage));
    collect(&mut session, ExpeditionReward::TrollDamage, &view, geometry);
    assert_eq!(
        session.player_tuning(&tuning).fireball_damage.to_bits(),
        45.0_f32.to_bits()
    );
    collect(
        &mut session,
        ExpeditionReward::DragonExplosions,
        &view,
        geometry,
    );
    collect(
        &mut session,
        ExpeditionReward::ShadowVitality,
        &view,
        geometry,
    );
    session.reset(1, &view, geometry);
    session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
    assert_eq!(
        session.player_tuning(&tuning).fireball_damage.to_bits(),
        15.0_f32.to_bits()
    );
    assert_eq!(
        session.actors.first().expect("player").max_hp.to_bits(),
        100.0_f32.to_bits()
    );
    assert!(session
        .expedition_progress()
        .expect("snapshot")
        .milestones
        .iter()
        .all(|m| !m.defeated && !m.collected && m.available_position.is_none()));
}

#[test]
fn pickup_keeps_the_damage_and_mode_of_an_already_flying_contact_shot() {
    let (mut session, view, geometry, materials, tuning) = start();
    let bridge = view.spawns.first().copied().expect("bridge");
    for (id, offset) in [
        (0, Vec3::NEG_X * 4.0),
        (1, Vec3::NEG_X),
        (2, Vec3::NEG_X + Vec3::Z),
    ] {
        let actor = session
            .actors
            .iter_mut()
            .find(|a| a.id == id)
            .expect("actor");
        actor.feet = bridge + offset;
        actor.previous_feet = actor.feet;
        actor.hp = 100.0;
        actor.max_hp = 100.0;
        actor.aim = Vec3::X;
    }
    session.release(
        0,
        Spell::Fireball,
        &tuning,
        45.0,
        &view,
        geometry,
        materials,
        &mut CommandsOut::default(),
    );
    kill_role(&mut session, ExpeditionRole::Troll, false);
    kill_role(&mut session, ExpeditionRole::Dragon, false);
    session.advance_milestones(&view, geometry);
    collect(&mut session, ExpeditionReward::TrollDamage, &view, geometry);
    collect(
        &mut session,
        ExpeditionReward::DragonExplosions,
        &view,
        geometry,
    );
    assert_eq!(
        session
            .projectiles
            .first()
            .expect("old shot")
            .fireball_mode(),
        FireballMode::ContactOnly
    );
    for _ in 0..120 {
        session.advance_projectiles(&view, geometry, materials, &mut CommandsOut::default());
    }
    assert_eq!(
        session
            .actors
            .iter()
            .find(|a| a.id == 1)
            .expect("target")
            .hp
            .to_bits(),
        85.0_f32.to_bits()
    );
    assert_eq!(
        session
            .actors
            .iter()
            .find(|a| a.id == 2)
            .expect("neighbor")
            .hp
            .to_bits(),
        100.0_f32.to_bits()
    );
    let player = session.actors.first_mut().expect("player");
    player.feet = bridge + Vec3::NEG_X * 4.0;
    player.previous_feet = player.feet;
    player.aim = Vec3::X;
    session.release(
        0,
        Spell::Fireball,
        &tuning,
        45.0,
        &view,
        geometry,
        materials,
        &mut CommandsOut::default(),
    );
    assert_eq!(
        session
            .projectiles
            .first()
            .expect("new shot")
            .fireball_mode(),
        FireballMode::Explosive
    );
    for _ in 0..120 {
        session.advance_projectiles(&view, geometry, materials, &mut CommandsOut::default());
    }
    let target = session.actors.iter().find(|a| a.id == 1).expect("target");
    assert!(
        target.hp >= 45.0 && target.hp < 46.0,
        "one frozen radial contribution: {}",
        target.hp
    );
    assert!(
        session
            .actors
            .iter()
            .find(|a| a.id == 2)
            .expect("neighbor")
            .hp
            < 100.0
    );
}
