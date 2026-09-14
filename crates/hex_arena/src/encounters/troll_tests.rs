use super::*;
use crate::ExpeditionRole;

#[test]
fn troll_support_crosses_forest_parties_but_shaman_support_remains_local() {
    let (mut session, view, geometry, materials, tuning) = fixture(ArenaEncounter::ShamanParty);
    pose(&mut session, 1, Vec3::ZERO, Vec3::X);
    pose(&mut session, 0, Vec3::new(-14.0, 0.0, 0.0), Vec3::X);
    for (id, role) in [
        (1, ExpeditionRole::Troll),
        (2, ExpeditionRole::BabyGoblin),
        (3, ExpeditionRole::Goblin),
        (4, ExpeditionRole::Shaman),
        (5, ExpeditionRole::Dragon),
        (6, ExpeditionRole::MountainShadow),
    ] {
        let actor = session
            .actors
            .iter_mut()
            .find(|a| a.id == id)
            .expect("actor");
        actor.configure_expedition(role, &tuning.encounters);
        actor.party = Some(u16::from(id));
        actor.feet = Vec3::new(f32::from(id), SKIN, 0.0);
        actor.previous_feet = actor.feet;
        actor.hp = 20.0;
    }
    start(&mut session, 1, CreatureAbility::Aura, Vec3::X, &tuning);
    // Resolve the real aura cast without unrelated movement or combat.
    let mut brains = std::mem::take(&mut session.encounter.brains);
    for _ in 0..120 {
        session.advance_abilities(
            &mut brains,
            &view,
            geometry,
            materials,
            &tuning,
            &mut CommandsOut::default(),
        );
    }
    session.encounter.brains = brains;
    assert!((session.auras().first().expect("Troll aura").radius - 9.0).abs() < 0.001);
    for _ in 0..120 {
        session.advance_support(&tuning, ArenaMap::ForestMassif);
    }
    for actor in session.actors.iter().filter(|a| (1..=6).contains(&a.id)) {
        let eligible = (2..=4).contains(&actor.id);
        assert!((actor.damage_multiplier - if eligible { 1.25 } else { 1.0 }).abs() < 0.001);
        assert!((actor.hp - if eligible { 23.0 } else { 20.0 }).abs() < 0.01);
    }
    // A surviving ordinary Shaman cannot inherit the boss's cross-party scope.
    let owner = session
        .actors
        .iter_mut()
        .find(|a| a.id == 1)
        .expect("owner");
    owner.configure_expedition(ExpeditionRole::Shaman, &tuning.encounters);
    session.refresh_support_buffs(&tuning);
    assert!(session
        .actors
        .iter()
        .all(|a| (a.damage_multiplier - 1.0).abs() < 0.001));
    session
        .actors
        .iter_mut()
        .find(|a| a.id == 2)
        .expect("ally")
        .party = Some(1);
    session.refresh_support_buffs(&tuning);
    assert!(
        (session
            .actors
            .iter()
            .find(|a| a.id == 2)
            .expect("ally")
            .damage_multiplier
            - 1.25)
            .abs()
            < 0.001
    );
    session
        .actors
        .iter_mut()
        .find(|a| a.id == 1)
        .expect("owner")
        .hp = 0.0;
    session.advance_support(&tuning, ArenaMap::ForestMassif);
    assert!(session.auras().is_empty());
    assert!(session
        .actors
        .iter()
        .all(|a| (a.damage_multiplier - 1.0).abs() < 0.001));
}

#[test]
fn troll_brain_uses_melee_nearby_and_releases_profiled_fireballs_at_range() {
    let (mut session, view, geometry, materials, tuning) = fixture(ArenaEncounter::ShamanParty);
    pose(&mut session, 1, Vec3::ZERO, Vec3::X);
    pose(&mut session, 0, Vec3::X * 1.5, Vec3::NEG_X);
    session.actors.retain(|a| a.id <= 1);
    let troll = session
        .actors
        .iter_mut()
        .find(|a| a.id == 1)
        .expect("Troll");
    troll.configure_expedition(ExpeditionRole::Troll, &tuning.encounters);
    let party = session.encounter.runtime.first_mut().expect("party");
    party.snapshot.phase = PartyPhase::Active;
    let mut brain = session.encounter.brains.remove(&1).expect("brain");
    let (_, request) = brain.intent(
        session.actors.iter().find(|a| a.id == 1).expect("Troll"),
        session.encounter.runtime.first().expect("party"),
        &session.actors,
        &[],
        &[],
        &session.collision,
        &view,
        geometry,
        &tuning,
        1,
    );
    assert_eq!(request.expect("close swipe").kind, CreatureAbility::Swipe);
    pose(&mut session, 0, Vec3::X * 10.0, Vec3::NEG_X);
    let mut released = false;
    for tick in 2..240 {
        let actor = session.actors.iter().find(|a| a.id == 1).expect("Troll");
        let (motion, request) = brain.intent(
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
        assert!(request.is_none());
        let actor = session
            .actors
            .iter_mut()
            .find(|a| a.id == 1)
            .expect("Troll");
        if let Some(selected) = motion.input.selected {
            actor.selected = selected;
        }
        actor.aim = motion.input.aim;
        let profile = actor.expedition_tuning(&tuning);
        if let Some((spell, speed)) = actor.casting(motion.input, &profile) {
            assert_eq!(spell, Spell::Fireball);
            session.release(
                1,
                spell,
                &tuning,
                speed,
                &view,
                geometry,
                materials,
                &mut CommandsOut::default(),
            );
            released = true;
            break;
        }
    }
    assert!(released, "Troll must use its caster branch");
    assert_eq!(
        session
            .projectiles
            .first()
            .expect("fireball")
            .fireball_mode(),
        crate::FireballMode::Explosive
    );
    for _ in 0..240 {
        session.advance_projectiles(&view, geometry, materials, &mut CommandsOut::default());
    }
    let hp = session.actors.first().expect("player").hp;
    assert!((hp - 65.0).abs() < 1.0, "profiled 35-damage shot, hp={hp}");
}
