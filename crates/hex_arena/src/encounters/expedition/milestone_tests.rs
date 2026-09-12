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
    assert_eq!(session.progress().expect("progress").total_xp, 432);
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
        46.0_f32.to_bits()
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
    let player = session.actors.first().expect("player");
    let target = session.actors.iter().find(|a| a.id == 1).expect("target");
    let (aim, _) = crate::bot::ballistic_aim(
        player.eye(),
        target.center(),
        &session.player_tuning(&tuning),
        45.0,
    )
    .expect("physical launch reaches the short target");
    session.actors.first_mut().expect("player").aim = aim;
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
    player.aim = aim;
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

#[test]
fn elevated_death_does_not_leave_a_reward_on_an_unreachable_tree_crown() {
    let (mut session, mut view, geometry, materials, _) = start();
    let crown = TilePos::new(
        HexCoord::from_world(*view.spawns.first().expect("bridge")),
        50,
    );
    for coord in crown.coord.within_radius(2) {
        view.voxels
            .insert(TilePos::new(coord, crown.level), materials.stone);
    }
    view.revision += 1;
    session.collision.refresh(&view, geometry);
    let troll = session
        .actors
        .iter_mut()
        .find(|a| a.expedition_role() == Some(ExpeditionRole::Troll))
        .expect("Troll");
    troll.feet = crown.coord.to_world(geometry.top(crown) + 4.0);
    kill_role(&mut session, ExpeditionRole::Troll, false);
    session.advance_milestones(&view, geometry);
    let point = Vec3::from_array(
        milestone(&session, ExpeditionReward::TrollDamage)
            .available_position
            .expect("reachable orb"),
    );
    let support = geometry
        .voxel_at(point - Vec3::Y * (0.6 + SKIN * 2.0))
        .expect("orb support");
    assert!(view
        .expedition
        .as_ref()
        .expect("sites")
        .encounters
        .values()
        .any(|site| site.deployment.surfaces.contains(&support)));
    assert!(point.y < geometry.top(crown) - 1.0);
}

#[test]
fn shadow_capacity_is_not_healing_and_fountain_uses_the_new_capacity() {
    let (mut session, mut view, geometry, materials, tuning) = start();
    kill_role(&mut session, ExpeditionRole::MountainShadow, false);
    session.advance_milestones(&view, geometry);
    collect(
        &mut session,
        ExpeditionReward::ShadowVitality,
        &view,
        geometry,
    );
    let player = session.actors.first().expect("player");
    assert!((player.hp - 100.0).abs() < 0.001);
    assert!((player.max_hp - 125.0).abs() < 0.001);
    session.tick = 10_000;
    session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
    assert!((session.actors.first().expect("player").hp - 100.0).abs() < 0.001);
    let (_, feet) = pool(&mut view, geometry);
    session.actors.first_mut().expect("player").feet = feet;
    session.advance_fountains(&view, geometry);
    let player = session.actors.first().expect("player");
    assert!((player.hp - 125.0).abs() < 0.001);
    assert!((player.max_hp - 125.0).abs() < 0.001);
    assert_eq!(
        session
            .expedition_progress()
            .expect("snapshot")
            .fountains
            .iter()
            .filter(|f| f.consumed)
            .count(),
        1
    );
}

#[test]
fn enemy_shaman_payload_is_unchanged_after_milestone_pickups_and_player_upgrades() {
    let mut samples = Vec::new();
    for rewarded in [false, true] {
        let (mut session, view, geometry, materials, mut tuning) = start();
        // Distinct valid enemy settings make accidental substitution of the
        // player's fixed gravity and unlocked radius observable in real flight.
        tuning.projectile_gravity = 16.0;
        tuning.fireball_size = 0;
        tuning.validate().expect("valid enemy comparison profile");
        if rewarded {
            for (role, reward) in [
                (ExpeditionRole::Troll, ExpeditionReward::TrollDamage),
                (ExpeditionRole::Dragon, ExpeditionReward::DragonExplosions),
                (
                    ExpeditionRole::MountainShadow,
                    ExpeditionReward::ShadowVitality,
                ),
            ] {
                kill_role(&mut session, role, true);
                session.advance_milestones(&view, geometry);
                collect(&mut session, reward, &view, geometry);
            }
            for stat in [
                UpgradeStat::FireballSize,
                UpgradeStat::FireballDamage,
                UpgradeStat::ProjectileSpeed,
            ] {
                assert!(session.spend_upgrade(stat));
            }
            let player = session.player_tuning(&tuning);
            assert!((player.fireball_damage - 46.0).abs() < 0.001);
            assert!((player.fireball_radius() - 2.65).abs() < 0.001);
            assert!((player.projectile_speed - 49.5).abs() < 0.001);
            assert!((player.projectile_gravity - 12.0).abs() < 0.001);
        }
        let bridge = view.spawns.first().copied().expect("bridge");
        let player = session.actors.first_mut().expect("player");
        player.feet = bridge + Vec3::X * 0.6;
        player.previous_feet = player.feet;
        assert!((player.hp - 100.0).abs() < 0.001, "Shadow did not heal");
        let actor = session
            .actors
            .iter_mut()
            .find(|actor| actor.expedition_role() == Some(ExpeditionRole::Shaman))
            .expect("living Shaman");
        actor.feet = bridge - Vec3::X * 2.6;
        actor.previous_feet = actor.feet;
        actor.aim = Vec3::X;
        actor.selected = Spell::Fireball;
        let enemy = actor.expedition_tuning(&tuning);
        assert!(actor
            .casting(
                ActorIntent {
                    cast_pressed: true,
                    cast_held: true,
                    ..Default::default()
                },
                &enemy
            )
            .is_none());
        while actor.charge().expect("normal charge").elapsed < enemy.encounters.shaman_charge {
            assert!(actor
                .casting(
                    ActorIntent {
                        cast_held: true,
                        ..Default::default()
                    },
                    &enemy
                )
                .is_none());
        }
        let charge = actor.charge().expect("charged").elapsed;
        let (spell, speed) = actor
            .casting(
                ActorIntent {
                    cast_released: true,
                    ..Default::default()
                },
                &enemy,
            )
            .expect("ordinary charged release");
        assert!((speed - tuning.launch_speed(charge)).abs() < 0.001);
        let owner = actor.id;
        session.release(
            owner,
            spell,
            &tuning,
            speed,
            &view,
            geometry,
            materials,
            &mut CommandsOut::default(),
        );
        let shot = session.projectiles.last().expect("enemy projectile");
        assert_eq!(shot.owner, owner);
        assert_eq!(shot.fireball_mode(), FireballMode::Explosive);
        let initial_velocity = shot.velocity;
        assert!((initial_velocity.length() - speed).abs() < 0.001);
        session.advance_projectiles(&view, geometry, materials, &mut CommandsOut::default());
        let velocity = session.projectiles.last().expect("still flying").velocity;
        let gravity = (initial_velocity.y - velocity.y) / STEP;
        assert!((gravity - 16.0).abs() < 0.001);
        if rewarded {
            // Player tuning changes again while the enemy payload is in flight.
            assert!(session.spend_upgrade(UpgradeStat::FireballDamage));
            assert!(session.spend_upgrade(UpgradeStat::ProjectileSpeed));
            assert!((session.player_tuning(&tuning).fireball_damage - 52.9).abs() < 0.001);
        }
        for _ in 0..120 {
            session.advance_projectiles(&view, geometry, materials, &mut CommandsOut::default());
            if session.projectiles.is_empty() {
                break;
            }
        }
        assert!(session.projectiles.is_empty(), "enemy shot resolved");
        let impact = session
            .effects
            .iter()
            .rev()
            .find(|effect| effect.kind == VisualEffectKind::Fireball)
            .expect("enemy explosion");
        assert!((impact.radius - 1.5).abs() < 0.001);
        let damage = 100.0 - session.actors.first().expect("player").hp;
        assert!(
            damage > 30.0 && damage <= tuning.encounters.shaman_fireball_damage,
            "enemy retains its own damage and one falloff contribution: {damage}"
        );
        samples.push([damage, impact.radius, gravity, initial_velocity.length()]);
    }
    let baseline = samples.first().expect("baseline");
    let rewarded = samples.last().expect("rewarded player");
    assert!(
        baseline
            .iter()
            .zip(rewarded)
            .all(|(a, b)| (a - b).abs() < 0.001),
        "enemy damage/radius/gravity/speed changed after player rewards: {samples:?}"
    );
}

#[test]
fn lowland_clear_thresholds_drop_once_and_pickups_apply_only_to_player() {
    for reverse in [false, true] {
        let (mut session, view, geometry, materials, tuning) = start();
        let roles = if reverse {
            [ExpeditionRole::PlainGolem, ExpeditionRole::PlainWisp]
        } else {
            [ExpeditionRole::PlainWisp, ExpeditionRole::PlainGolem]
        };
        for role in roles {
            let (reward, total, xp) = if role == ExpeditionRole::PlainWisp {
                (ExpeditionReward::WispBallistics, 10, 3)
            } else {
                (ExpeditionReward::GolemShield, 3, 25)
            };
            let ids: Vec<_> = session
                .actors
                .iter()
                .filter(|a| a.expedition_role() == Some(role))
                .map(|a| a.id)
                .collect();
            assert_eq!(ids.len(), total);
            for id in ids.iter().take(total - 1) {
                defeat(&mut session, *id, true);
            }
            session.advance_milestones(&view, geometry);
            assert!(!milestone(&session, reward).defeated);
            let prior = session.progress().expect("progress").total_xp;
            let final_id = *ids.last().expect("last enemy");
            defeat(&mut session, final_id, true);
            session.advance_milestones(&view, geometry);
            assert_eq!(session.progress().expect("progress").total_xp, prior + xp);
            assert!(milestone(&session, reward).available_position.is_some());
            let effective = session.player_tuning(&tuning);
            match role {
                ExpeditionRole::PlainWisp => {
                    assert!(
                        !session
                            .progress()
                            .expect("locked guide")
                            .fireball_guide_unlocked
                    );
                    assert!((effective.projectile_speed - 45.0).abs() < 0.001);
                }
                _ => {
                    assert!((effective.spell_projectile_speed(Spell::Shield) - 45.0).abs() < 0.001);
                    assert_eq!(effective.shield_dimensions(), (5, 5));
                }
            }
            collect(&mut session, reward, &view, geometry);
            let before = session.player_tuning(&tuning);
            defeat(&mut session, final_id, true);
            session.advance_milestones(&view, geometry);
            assert_eq!(session.progress().expect("once XP").total_xp, prior + xp);
            assert_eq!(
                before.player_profile,
                session.player_tuning(&tuning).player_profile
            );
            assert_eq!(
                before.projectile_speed.to_bits(),
                session.player_tuning(&tuning).projectile_speed.to_bits()
            );
            assert!(milestone(&session, reward).collected);
            assert!(milestone(&session, reward).available_position.is_none());
        }
        let effective = session.player_tuning(&tuning);
        assert!(session.progress().expect("guide").fireball_guide_unlocked);
        assert!((effective.projectile_speed - 60.0).abs() < 0.001);
        assert!((effective.spell_projectile_speed(Spell::Shield) - 65.0).abs() < 0.001);
        assert_eq!(effective.shield_dimensions(), (7, 7));
        assert!((tuning.projectile_speed - 32.0).abs() < 0.001);
        assert_eq!(tuning.shield_dimensions(), (5, 5));
        assert_eq!(
            session
                .expedition_progress()
                .expect("counts")
                .wisps_defeated,
            10
        );
        assert_eq!(
            session
                .expedition_progress()
                .expect("counts")
                .golems_defeated,
            3
        );
        assert_eq!(session.progress().expect("XP").total_xp, 105);
        session.reset(1, &view, geometry);
        session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
        assert!(
            !session
                .progress()
                .expect("reset guide")
                .fireball_guide_unlocked
        );
        let reset = session.player_tuning(&tuning);
        assert!((reset.projectile_speed - 45.0).abs() < 0.001);
        assert!((reset.spell_projectile_speed(Spell::Shield) - 45.0).abs() < 0.001);
        assert_eq!(reset.shield_dimensions(), (5, 5));
    }
}

#[test]
fn lowland_pickup_and_rank_order_give_identical_effective_spell_stats() {
    let mut outcomes = Vec::new();
    for purchase_first in [false, true] {
        let (mut session, view, geometry, _, tuning) = start();
        session
            .progression
            .as_mut()
            .expect("state")
            .snapshot
            .available_upgrades = 20;
        let buy = |session: &mut ArenaSession| {
            for stat in [
                UpgradeStat::ProjectileSpeed,
                UpgradeStat::ShieldProjectileSpeed,
                UpgradeStat::ShieldSize,
            ] {
                for _ in 0..stat.max_ranks() {
                    assert!(session.spend_upgrade(stat));
                }
            }
        };
        if purchase_first {
            buy(&mut session);
        }
        for (role, reward) in [
            (ExpeditionRole::PlainGolem, ExpeditionReward::GolemShield),
            (ExpeditionRole::PlainWisp, ExpeditionReward::WispBallistics),
        ] {
            kill_role(&mut session, role, true);
            session.advance_milestones(&view, geometry);
            collect(&mut session, reward, &view, geometry);
        }
        if !purchase_first {
            buy(&mut session);
        }
        let effective = session.player_tuning(&tuning);
        effective.validate().expect("max rewarded profile");
        outcomes.push((
            effective.projectile_speed.to_bits(),
            effective.player_profile,
        ));
    }
    assert_eq!(outcomes.first(), outcomes.get(1));
}
