//! Regressions for the native matchup's failed outgoing shots and passive breath.

use super::*;

#[test]
fn shaman_authored_spread_is_a_world_space_target_offset_at_near_and_far_range() {
    for distance in [10.0, 20.0] {
        let (mut session, view, geometry, _, tuning) = fixture(ArenaEncounter::ShamanParty);
        session.actors.retain(|a| a.id <= 1);
        pose(&mut session, 1, Vec3::ZERO, Vec3::X);
        pose(&mut session, 0, Vec3::X * distance, Vec3::NEG_X);
        session
            .encounter
            .runtime
            .first_mut()
            .expect("party")
            .snapshot
            .phase = PartyPhase::Active;
        let mut brain = session.encounter.brains.remove(&1).expect("shaman brain");
        let mut launched = None;
        for tick in 0..180 {
            let actor = session.actors.iter().find(|a| a.id == 1).expect("shaman");
            let (motion, _) = brain.intent(
                actor,
                session.encounter.runtime.first().expect("party"),
                &session.actors,
                &[],
                &[],
                &session.collision,
                &view,
                geometry,
                &tuning,
                tick,
            );
            let actor = session
                .actors
                .iter_mut()
                .find(|a| a.id == 1)
                .expect("shaman");
            actor.aim = motion.input.aim;
            if let Some(selected) = motion.input.selected {
                actor.selected = selected;
            }
            if let Some((spell, speed)) = actor.casting(motion.input, &tuning) {
                assert_eq!(spell, Spell::Fireball);
                launched = Some((actor.eye(), actor.aim * speed));
                break;
            }
        }
        let (origin, velocity) = launched.expect("visible stationary target gets a charged shot");
        let time = (distance - origin.x) / velocity.x;
        let at_target =
            origin + velocity * time - Vec3::Y * tuning.projectile_gravity * time * time * 0.5;
        let center = session.actors.first().expect("target").center();
        let spread = tuning.bot.aim_error * tuning.encounters.shaman_spread_multiplier;
        assert!(
            at_target.distance(center) <= spread * 1.7,
            "world-space spread must not expand into an angular miss at {distance}u: {at_target:?} vs {center:?}"
        );
    }
}

#[test]
fn retreating_dragon_can_breathe_at_visible_attacker_without_resetting_its_retreat() {
    let (mut session, view, geometry, materials, tuning) = fixture(ArenaEncounter::Dragon);
    pose(&mut session, 1, Vec3::ZERO, Vec3::NEG_Z);
    pose(&mut session, 0, Vec3::NEG_Z * 4.0, Vec3::Z);
    session
        .encounter
        .runtime
        .first_mut()
        .expect("party")
        .snapshot
        .phase = PartyPhase::Active;
    // The barrier has already been spent; this exercises the next defensive option.
    *session
        .encounter
        .brains
        .get_mut(&1)
        .expect("brain")
        .cooldowns
        .get_mut(abilities::index(CreatureAbility::Barrier))
        .expect("barrier") = 600.0;
    session
        .actors
        .iter_mut()
        .find(|a| a.id == 1)
        .expect("dragon")
        .hp = 70.0;
    session.record_damage(0, 1, 30.0);
    session.bot_enabled = true;
    ticks(&mut session, 60, &view, geometry, materials, &tuning);
    session.record_damage(0, 1, 1.0);
    ticks(&mut session, 60, &view, geometry, materials, &tuning);
    let dragon = session.actors.iter().find(|a| a.id == 1).expect("dragon");
    assert!(dragon.flying && (dragon.hp - 70.0).abs() < 0.001);
    assert!(session.actors.first().expect("attacker").hp <= 65.01);
    assert!(
        session
            .creature_decisions()
            .iter()
            .find(|d| d.id == 1)
            .expect("decision")
            .retreat_seconds
            > 3.4,
        "new damage still refreshes retreat"
    );
    assert_eq!(
        session
            .encounter
            .ability_counts
            .get(&1)
            .expect("casts")
            .get(2),
        Some(&1)
    );
}

#[test]
fn active_breath_tracks_visible_strafe_through_the_physical_bounded_body_turn() {
    let (mut session, view, geometry, materials, tuning) = fixture(ArenaEncounter::Dragon);
    pose(&mut session, 1, Vec3::ZERO, Vec3::NEG_Z);
    pose(&mut session, 0, Vec3::NEG_Z * 4.5, Vec3::Z);
    session
        .encounter
        .runtime
        .first_mut()
        .expect("party")
        .snapshot
        .phase = PartyPhase::Active;
    session.bot_enabled = true;
    let mut previous = 0.0_f32;
    let mut snapshots = 0;
    for _ in 0..100 {
        session.advance(
            ActorIntent {
                aim: Vec3::Z,
                movement: Vec2::X,
                ..Default::default()
            },
            &view,
            geometry,
            materials,
            &tuning,
        );
        let dragon = session.actors.iter().find(|a| a.id == 1).expect("dragon");
        if let Some(attack) = dragon
            .attack_state()
            .filter(|a| a.kind == CreatureAbility::FireCone)
        {
            let forward = dragon.body_rotation() * Vec3::NEG_Z;
            assert!(attack.direction.with_y(0.0).normalize().dot(forward) > 0.9999);
            assert!(attack.origin.distance(dragon.eye()) < 0.0001);
            assert!(
                (dragon.body_yaw - previous).abs()
                    <= tuning.encounters.dragon_turn_speed * STEP + 0.0001
            );
            snapshots += 1;
        }
        previous = dragon.body_yaw;
    }
    assert!(snapshots > 50 && previous.abs() > 0.15);
    assert!(
        session.actors.first().expect("moving target").hp < 77.0,
        "tracking must land multiple real pulses on the strafing target"
    );
}

#[test]
fn active_breath_does_not_track_a_target_that_moves_behind_cover() {
    let mut directions = Vec::new();
    for hidden_z in [-5.0, 5.0] {
        let (mut session, mut view, geometry, materials, tuning) = fixture(ArenaEncounter::Dragon);
        pose(&mut session, 1, Vec3::NEG_X * 4.0, Vec3::X);
        pose(&mut session, 0, Vec3::X, Vec3::NEG_X);
        session
            .encounter
            .runtime
            .first_mut()
            .expect("party")
            .snapshot
            .phase = PartyPhase::Active;
        session.bot_enabled = true;
        ticks(&mut session, 5, &view, geometry, materials, &tuning);
        assert!(session
            .actors
            .iter()
            .find(|a| a.id == 1)
            .expect("dragon")
            .attack_state()
            .is_some_and(|a| a.kind == CreatureAbility::FireCone));
        divider(&mut view, materials.stone);
        pose(&mut session, 0, Vec3::new(4.0, 0.0, hidden_z), Vec3::NEG_X);
        ticks(&mut session, 20, &view, geometry, materials, &tuning);
        let dragon = session.actors.iter().find(|a| a.id == 1).expect("dragon");
        assert!(
            !session
                .creature_decisions()
                .iter()
                .find(|d| d.id == 1)
                .expect("decision")
                .own_sight
        );
        directions.push(
            dragon
                .attack_state()
                .expect("ongoing breath")
                .direction
                .to_array()
                .map(f32::to_bits),
        );
    }
    assert_eq!(directions.first(), directions.get(1));
}
