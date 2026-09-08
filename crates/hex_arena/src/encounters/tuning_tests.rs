//! Behavior evidence for the first numeric Dragon and party-support trial.

use super::*;

#[test]
fn retreating_dragon_turns_into_prospective_mouth_reach_before_admitting_breath() {
    let (mut session, view, geometry, materials, tuning) = fixture(ArenaEncounter::Dragon);
    pose(&mut session, 1, Vec3::ZERO, Vec3::Z);
    pose(&mut session, 0, Vec3::NEG_Z * 5.0, Vec3::Z);
    session
        .encounter
        .runtime
        .first_mut()
        .expect("party")
        .snapshot
        .phase = PartyPhase::Active;
    *session
        .encounter
        .brains
        .get_mut(&1)
        .expect("brain")
        .cooldowns
        .get_mut(abilities::index(CreatureAbility::Barrier))
        .expect("barrier") = 600.0;
    let dragon = session
        .actors
        .iter_mut()
        .find(|a| a.id == 1)
        .expect("dragon");
    dragon.hp -= 10.0;
    assert!(
        dragon.eye().distance(Vec3::NEG_Z * 5.0 + Vec3::Y * 0.4) > tuning.encounters.breath_range
    );
    session.record_damage(0, 1, 10.0);
    session.bot_enabled = true;
    let mut started = false;
    for _ in 0..240 {
        session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
        let dragon = session.actors.iter().find(|a| a.id == 1).expect("dragon");
        assert!(shapes::clear(
            &session.collision,
            dragon,
            dragon.feet,
            dragon.body_yaw
        ));
        assert!(
            dragon.feet.length() < 0.01,
            "turn preparation must not flee farther away"
        );
        if dragon
            .attack_state()
            .is_some_and(|a| a.kind == CreatureAbility::FireCone)
        {
            let target = session.actors.first().expect("target").center();
            let aim = (target - dragon.eye()).normalize();
            assert!(target.distance(dragon.eye()) <= tuning.encounters.breath_range);
            assert!(
                (dragon.body_rotation() * Vec3::NEG_Z).dot(aim.with_y(0.0).normalize())
                    >= (tuning.encounters.breath_angle.to_radians() * 0.5).cos() - 0.001
            );
            started = true;
            break;
        }
    }
    assert!(
        started,
        "a target reachable after turning must permit the physical turn"
    );
}

fn support_scene() -> (
    ArenaSession,
    ArenaTerrainView,
    ArenaVoxelGeometry,
    ArenaMaterials,
    ArenaTuning,
) {
    let (mut session, view, geometry, materials, tuning) = fixture(ArenaEncounter::ShamanParty);
    session.actors.retain(|a| a.id <= 3);
    session
        .encounter
        .runtime
        .first_mut()
        .expect("party")
        .snapshot
        .phase = PartyPhase::Active;
    pose(&mut session, 1, Vec3::ZERO, Vec3::X);
    pose(&mut session, 2, Vec3::new(1.0, 0.0, -0.6), Vec3::X);
    pose(&mut session, 3, Vec3::new(1.0, 0.0, 0.6), Vec3::X);
    pose(&mut session, 0, Vec3::X * 18.0, Vec3::NEG_X);
    session
        .actors
        .iter_mut()
        .find(|a| a.id == 1)
        .expect("shaman")
        .cooldowns = [600.0; 3];
    session.bot_enabled = true;
    (session, view, geometry, materials, tuning)
}

#[test]
fn healthy_approaching_party_saves_aura_until_two_allies_are_engaged() {
    let (mut session, view, geometry, materials, tuning) = support_scene();
    ticks(&mut session, 60, &view, geometry, materials, &tuning);
    assert!(session.auras().is_empty());
    assert!(session
        .encounter
        .brains
        .get(&1)
        .expect("brain")
        .cooldowns
        .get(abilities::index(CreatureAbility::Aura))
        .is_some_and(|cd| *cd <= 0.0));
    pose(&mut session, 1, Vec3::NEG_X * 5.0, Vec3::X);
    pose(&mut session, 2, Vec3::new(-1.0, 0.0, -0.6), Vec3::X);
    pose(&mut session, 3, Vec3::new(-1.0, 0.0, 0.6), Vec3::X);
    pose(&mut session, 0, Vec3::ZERO, Vec3::NEG_X);
    ticks(&mut session, 65, &view, geometry, materials, &tuning);
    assert_eq!(
        session.auras().len(),
        1,
        "actual melee engagement should release support"
    );
    assert!(session
        .actors
        .iter()
        .filter(|a| a.id == 2 || a.id == 3)
        .all(|a| a.damage_multiplier > 1.0));
}

#[test]
fn shaman_firing_position_remains_behind_and_in_aura_range_of_its_frontline() {
    let (mut session, view, geometry, materials, tuning) = support_scene();
    pose(&mut session, 1, Vec3::NEG_X * 8.0, Vec3::X);
    pose(&mut session, 2, Vec3::new(-1.0, 0.0, -0.6), Vec3::X);
    pose(&mut session, 3, Vec3::new(-1.0, 0.0, 0.6), Vec3::X);
    pose(&mut session, 0, Vec3::ZERO, Vec3::NEG_X);
    ticks(&mut session, 1, &view, geometry, materials, &tuning);
    let decision = session
        .creature_decisions()
        .into_iter()
        .find(|d| d.id == 1)
        .expect("decision");
    let goal = Vec3::from_array(decision.goal);
    assert!(
        decision.useful_shot,
        "fixture has an admitted ordinary firing lane"
    );
    assert!(goal.x < -1.0, "the shaman remains behind its fighters");
    for ally in session.actors.iter().filter(|a| a.id == 2 || a.id == 3) {
        assert!(
            goal.distance(ally.feet) < tuning.encounters.aura_radius,
            "support goal must reach both engaged allies: {goal:?} vs {:?}",
            ally.feet
        );
    }
    assert!(
        session
            .actors
            .iter()
            .find(|a| a.id == 1)
            .expect("shaman")
            .feet
            .x
            > -8.0,
        "the ordinary controller must approach its useful support position"
    );
}
