use super::*;
use crate::collision::SKIN;
use hex_core::{ElementId, SubstanceId};

fn fixture(
    encounter: ArenaEncounter,
) -> (
    ArenaSession,
    ArenaTerrainView,
    ArenaVoxelGeometry,
    ArenaMaterials,
    ArenaTuning,
) {
    let geometry = ArenaVoxelGeometry {
        radius: 20,
        ..Default::default()
    };
    let view = ArenaTerrainView {
        selection: hex_core::arena::ArenaSelection {
            map: ArenaMap::Fort,
            encounter,
        },
        voxels: HexCoord::from_axial(0, 0)
            .within_radius(20)
            .into_iter()
            .map(|coord| (TilePos::new(coord, 0), SubstanceId(1)))
            .collect(),
        spawns: [Vec3::new(-14.0, SKIN, 0.0), Vec3::new(8.0, SKIN, 0.0)],
        ..Default::default()
    };
    let materials = ArenaMaterials {
        stone: SubstanceId(1),
        grass: SubstanceId(2),
        dirt: SubstanceId(3),
        bedrock: SubstanceId(4),
        fire: ElementId(1),
    };
    let tuning = ArenaTuning::default();
    let mut session = ArenaSession {
        bot_enabled: false,
        ..Default::default()
    };
    session.reset(0, &view, geometry);
    session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
    (session, view, geometry, materials, tuning)
}
fn pose(session: &mut ArenaSession, id: u8, feet: Vec3, aim: Vec3) {
    let a = session
        .actors
        .iter_mut()
        .find(|a| a.id == id)
        .expect("actor");
    a.feet = feet + Vec3::Y * SKIN;
    a.previous_feet = a.feet;
    a.aim = aim;
    a.body_yaw = (-aim.x).atan2(-aim.z);
    a.previous_yaw = a.body_yaw;
    a.body = controller::Body::default();
    a.body.grounded = true;
    a.grounded = true;
}
fn start(
    session: &mut ArenaSession,
    id: u8,
    kind: CreatureAbility,
    aim: Vec3,
    tuning: &ArenaTuning,
) {
    let mut brains = std::mem::take(&mut session.encounter.brains);
    session.begin_ability(&mut brains, id, brain::Request { kind, aim }, tuning);
    session.encounter.brains = brains;
}
fn ticks(
    session: &mut ArenaSession,
    n: usize,
    view: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    materials: ArenaMaterials,
    tuning: &ArenaTuning,
) -> Vec<TerrainImpact> {
    let mut impacts = Vec::new();
    for _ in 0..n {
        impacts.extend(
            session
                .advance(ActorIntent::default(), view, geometry, materials, tuning)
                .impacts,
        );
    }
    impacts
}

#[test]
fn authored_rosters_are_stable_dry_and_reset_without_respawning_dead_parties() {
    for (kind, count) in [
        (ArenaEncounter::Dragon, 2),
        (ArenaEncounter::Goblins, 6),
        (ArenaEncounter::ShamanParty, 5),
        (ArenaEncounter::Shadow, 2),
    ] {
        let (mut session, view, geometry, materials, tuning) = fixture(kind);
        assert_eq!(session.actors.len(), count);
        assert!(session
            .actors
            .iter()
            .all(|a| session.actor_pose_valid(a.id, &view, geometry)));
        assert!(session
            .parties()
            .iter()
            .all(|p| p.phase == PartyPhase::Dormant));
        for a in session.actors.iter_mut().skip(1) {
            a.hp = 0.0;
        }
        ticks(&mut session, 1, &view, geometry, materials, &tuning);
        assert_eq!(session.outcome, Some(ArenaOutcome::Winner(0)));
        ticks(&mut session, 100, &view, geometry, materials, &tuning);
        assert!(session.actors.iter().skip(1).all(|a| a.hp <= 0.0));
        session.reset(1, &view, geometry);
        ticks(&mut session, 1, &view, geometry, materials, &tuning);
        assert!(session.actors.iter().all(|a| a.hp > 0.0));
        assert!(
            session.projectiles.is_empty()
                && session.barriers().is_empty()
                && session.auras().is_empty()
        );
    }
}
#[test]
fn goblin_swipe_has_real_windup_single_hit_and_physical_terrain_contact() {
    let (mut session, mut view, geometry, materials, tuning) = fixture(ArenaEncounter::Goblins);
    pose(&mut session, 1, Vec3::ZERO, Vec3::X);
    pose(&mut session, 0, Vec3::X * 1.3, Vec3::NEG_X);
    for a in session.actors.iter_mut().skip(2) {
        a.feet = Vec3::X * 12.0 + Vec3::Z * f32::from(a.id);
        a.previous_feet = a.feet;
    }
    start(&mut session, 1, CreatureAbility::Swipe, Vec3::X, &tuning);
    ticks(&mut session, 20, &view, geometry, materials, &tuning);
    assert!((session.actors.first().expect("human").hp - 100.0).abs() < 0.001);
    ticks(&mut session, 50, &view, geometry, materials, &tuning);
    assert!((session.actors.first().expect("human").hp - 88.0).abs() < 0.001);
    // A separate swing toward an exposed voxel damages it with Physical power1.
    view.revision += 1;
    let wall = HexCoord::from_axial(1, 0);
    for level in 1..=3 {
        view.voxels
            .insert(TilePos::new(wall, level), materials.stone);
    }
    pose(&mut session, 0, Vec3::X * -10.0, Vec3::X);
    session
        .encounter
        .brains
        .get_mut(&1)
        .expect("brain")
        .cooldowns = [0.0; 7];
    start(&mut session, 1, CreatureAbility::Swipe, Vec3::X, &tuning);
    let impacts = ticks(&mut session, 65, &view, geometry, materials, &tuning);
    assert!(impacts
        .iter()
        .any(|i| i.kind == hex_core::TerrainDamageKind::Physical
            && i.power == 1
            && i.volume.iter().any(|p| p.coord == wall)));
}
#[test]
fn breath_three_pulses_cap_damage_and_dead_windup_never_releases() {
    let (mut session, view, geometry, materials, tuning) = fixture(ArenaEncounter::Dragon);
    pose(&mut session, 1, Vec3::ZERO, Vec3::NEG_Z);
    pose(&mut session, 0, Vec3::NEG_Z * 4.0, Vec3::Z);
    start(
        &mut session,
        1,
        CreatureAbility::FireCone,
        Vec3::NEG_Z,
        &tuning,
    );
    ticks(&mut session, 150, &view, geometry, materials, &tuning);
    assert!((session.actors.first().expect("human").hp - 65.0).abs() < 0.01);
    assert_eq!(
        session
            .encounter
            .ability_counts
            .get(&1)
            .expect("counts")
            .get(2),
        Some(&1)
    );
    let (mut session, view, geometry, materials, tuning) = fixture(ArenaEncounter::Dragon);
    start(&mut session, 1, CreatureAbility::Bite, Vec3::X, &tuning);
    session
        .actors
        .iter_mut()
        .find(|a| a.id == 1)
        .expect("dragon")
        .hp = 0.0;
    ticks(&mut session, 40, &view, geometry, materials, &tuning);
    assert!(session.encounter.ability_counts.is_empty());
}
#[test]
fn barrier_is_temporary_direct_cover_and_fireball_splash_still_passes() {
    let (mut session, view, geometry, materials, tuning) = fixture(ArenaEncounter::Dragon);
    pose(&mut session, 1, Vec3::ZERO, Vec3::NEG_Z);
    pose(&mut session, 0, Vec3::NEG_Z * 7.0, Vec3::Z);
    start(
        &mut session,
        1,
        CreatureAbility::Barrier,
        Vec3::NEG_Z,
        &tuning,
    );
    ticks(&mut session, 1, &view, geometry, materials, &tuning);
    let barrier = session.barriers().first().expect("barrier").clone();
    assert!(session
        .collision
        .sight_clear(barrier.center - Vec3::Z, barrier.center + Vec3::Z));
    pose(
        &mut session,
        0,
        barrier.center - Vec3::Z * 1.5 - Vec3::Y * 0.62,
        Vec3::Z,
    );
    let mut out = CommandsOut::default();
    session.release(
        0,
        Spell::Fireball,
        &tuning,
        32.0,
        &view,
        geometry,
        materials,
        &mut out,
    );
    ticks(&mut session, 15, &view, geometry, materials, &tuning);
    assert!(session.projectiles.is_empty());
    assert_eq!(
        session
            .combat_stats
            .first()
            .expect("stats")
            .fireballs_resolved,
        1
    );
    assert!(session.barriers().first().expect("damaged barrier").hp < barrier.hp);
    assert!(session.actors.first().expect("caster").hp < 100.0);
    ticks(&mut session, 500, &view, geometry, materials, &tuning);
    assert!(session.barriers().is_empty());
}
#[test]
fn allies_are_pass_through_and_splash_immune_but_dead_owner_keeps_self_allegiance() {
    let (mut session, view, geometry, materials, tuning) = fixture(ArenaEncounter::Goblins);
    pose(&mut session, 1, Vec3::ZERO, Vec3::X);
    pose(&mut session, 2, Vec3::X * 2.0, Vec3::NEG_X);
    pose(&mut session, 0, Vec3::X * 5.0, Vec3::NEG_X);
    let mut out = CommandsOut::default();
    session.release(
        1,
        Spell::Fireball,
        &tuning,
        32.0,
        &view,
        geometry,
        materials,
        &mut out,
    );
    session
        .actors
        .iter_mut()
        .find(|a| a.id == 1)
        .expect("owner")
        .hp = 0.0;
    ticks(&mut session, 30, &view, geometry, materials, &tuning);
    assert!(session.actors.first().expect("human").hp < 100.0);
    let ally = session.actors.iter().find(|a| a.id == 2).expect("ally");
    assert!((ally.hp - 50.0).abs() < 0.001 && ally.body.impulse_velocity.length() < 0.001);
}
#[test]
fn aura_requires_same_party_los_excludes_owner_and_ends_on_owner_death() {
    let (mut session, view, geometry, materials, tuning) = fixture(ArenaEncounter::ShamanParty);
    pose(&mut session, 1, Vec3::ZERO, Vec3::X);
    pose(&mut session, 2, Vec3::X * 3.0, Vec3::X);
    session
        .actors
        .iter_mut()
        .find(|a| a.id == 1)
        .expect("shaman")
        .hp = 40.0;
    session
        .actors
        .iter_mut()
        .find(|a| a.id == 2)
        .expect("ally")
        .hp = 20.0;
    start(&mut session, 1, CreatureAbility::Aura, Vec3::X, &tuning);
    ticks(&mut session, 180, &view, geometry, materials, &tuning);
    let shaman = session.actors.iter().find(|a| a.id == 1).expect("shaman");
    assert!((shaman.hp - 40.0).abs() < 0.001);
    let ally = session.actors.iter().find(|a| a.id == 2).expect("ally");
    assert!(ally.hp > 22.5 && (ally.damage_multiplier - 1.25).abs() < 0.001);
    session
        .actors
        .iter_mut()
        .find(|a| a.id == 1)
        .expect("shaman")
        .hp = 0.0;
    ticks(&mut session, 1, &view, geometry, materials, &tuning);
    assert!(session.auras().is_empty());
    assert!(
        (session
            .actors
            .iter()
            .find(|a| a.id == 2)
            .expect("ally")
            .damage_multiplier
            - 1.0)
            .abs()
            < 0.001
    );
}
#[test]
fn damage_wakes_only_its_party_without_revealing_the_hidden_player() {
    let (mut session, mut view, geometry, materials, tuning) = fixture(ArenaEncounter::Goblins);
    view.selection.map = ArenaMap::SevenRegions;
    view.anchors = BTreeMap::from([
        ("mountains_high_pass".into(), Vec3::new(8.0, SKIN, 0.0)),
        ("fort_fort_courtyard".into(), Vec3::new(0.0, SKIN, 14.0)),
        ("caves_cave_entrance".into(), Vec3::new(0.0, SKIN, -14.0)),
    ]);
    session.reset(1, &view, geometry);
    ticks(&mut session, 1, &view, geometry, materials, &tuning);
    let victim = session
        .actors
        .iter()
        .find(|a| a.id == 1)
        .expect("dragon")
        .clone();
    session.record_damage(0, 1, 1.0);
    assert_eq!(
        session
            .encounter
            .runtime
            .iter()
            .filter(|p| p.snapshot.phase == PartyPhase::Active)
            .count(),
        1
    );
    let knowledge = session
        .encounter
        .runtime
        .first()
        .expect("party")
        .knowledge
        .expect("cue");
    assert!(!knowledge.direct);
    assert!(knowledge.point.distance(victim.center()) < 2.0);
    assert!(
        knowledge
            .point
            .distance(session.actors.first().expect("human").feet)
            > 12.0
    );
}
#[test]
fn safe_human_regen_is_map_only_and_activity_resets_its_delay() {
    let (mut session, view, geometry, materials, tuning) = fixture(ArenaEncounter::Dragon);
    session.actors.first_mut().expect("human").hp = 50.0;
    ticks(&mut session, 900, &view, geometry, materials, &tuning);
    assert!((session.actors.first().expect("human").hp - 50.0).abs() < 0.001);
    ticks(&mut session, 180, &view, geometry, materials, &tuning);
    assert!(session.actors.first().expect("human").hp > 51.0);
    session.record_cast(0, Spell::Shield);
    let hp = session.actors.first().expect("human").hp;
    ticks(&mut session, 120, &view, geometry, materials, &tuning);
    assert!((session.actors.first().expect("human").hp - hp).abs() < 0.001);
}
#[test]
fn duel_outcomes_ignore_all_encounter_tuning_and_keep_two_actor_identity() {
    let (mut a, mut view, geometry, materials, tuning) = fixture(ArenaEncounter::Dragon);
    view.selection.map = ArenaMap::Duel;
    a.reset(1, &view, geometry);
    a.bot_enabled = true;
    let mut b = ArenaSession::default();
    b.reset(1, &view, geometry);
    let mut changed = tuning.clone();
    changed.encounters.dragon_hp = 1.0;
    changed.encounters.human_regen_rate = 100.0;
    changed.encounters.activation_radius = 1.0;
    for tick in 0..1800 {
        let input = ActorIntent {
            aim: Vec3::X,
            selected: Some(Spell::AreaBlast),
            cast_pressed: tick % 90 == 0,
            cast_released: tick % 90 == 0,
            ..Default::default()
        };
        let ca = a.advance(input, &view, geometry, materials, &tuning);
        let cb = b.advance(input, &view, geometry, materials, &changed);
        assert_eq!(ca.impacts.len(), cb.impacts.len());
    }
    assert_eq!(a.outcome, b.outcome);
    assert_eq!(a.tick, b.tick);
    assert_eq!(a.actors.len(), 2);
    for (a, b) in a.actors.iter().zip(&b.actors) {
        assert_eq!(
            a.feet.to_array().map(f32::to_bits),
            b.feet.to_array().map(f32::to_bits)
        );
        assert_eq!(a.hp.to_bits(), b.hp.to_bits());
        assert_eq!(a.cooldowns.map(f32::to_bits), b.cooldowns.map(f32::to_bits));
    }
    assert!(!a.encounter_summary().enabled);
}

fn divider(view: &mut ArenaTerrainView, material: SubstanceId) {
    view.revision += 1;
    for coord in HexCoord::from_axial(0, 0).within_radius(20) {
        if coord.to_world(0.0).x.abs() < 1.0 {
            for level in 1..=20 {
                view.voxels.insert(TilePos::new(coord, level), material);
            }
        }
    }
}

#[test]
fn hidden_human_changes_do_not_change_creature_intents_after_the_same_observation() {
    for encounter in [
        ArenaEncounter::Dragon,
        ArenaEncounter::Goblins,
        ArenaEncounter::ShamanParty,
    ] {
        let (mut session, mut view, geometry, materials, tuning) = fixture(encounter);
        pose(&mut session, 1, Vec3::new(-5.0, 0.0, 0.0), Vec3::X);
        pose(&mut session, 0, Vec3::new(5.0, 0.0, 0.0), Vec3::NEG_X);
        divider(&mut view, materials.stone);
        session.collision.refresh(&view, geometry);
        let party = session.encounter.runtime.first_mut().expect("party");
        party.snapshot.phase = PartyPhase::Active;
        party.knowledge = Some(Knowledge {
            point: Vec3::new(5.0, 0.0, 0.0),
            velocity: Vec3::ZERO,
            tick: 1,
            direct: true,
        });
        let mut left = brain::Brain::new(1, Vec3::new(-5.0, 0.0, 0.0));
        let mut right = brain::Brain::new(1, Vec3::new(-5.0, 0.0, 0.0));
        let mut other = session.actors.clone();
        other.first_mut().expect("hidden human").feet = Vec3::new(11.0, 0.0, 5.0);
        for tick in 2..100 {
            let actor = session.actors.iter().find(|a| a.id == 1).expect("creature");
            let party = session.encounter.runtime.first().expect("party");
            let (a, ar) = left.intent(
                actor,
                party,
                &session.actors,
                &[],
                &[],
                &session.collision,
                &view,
                geometry,
                &tuning,
                tick,
            );
            let (b, br) = right.intent(
                actor,
                party,
                &other,
                &[],
                &[],
                &session.collision,
                &view,
                geometry,
                &tuning,
                tick,
            );
            assert_eq!(format!("{a:?}{ar:?}"), format!("{b:?}{br:?}"));
        }
    }
}

#[test]
fn search_expires_into_return_and_preserves_damage_when_home_is_reached() {
    let (mut session, mut view, geometry, materials, tuning) = fixture(ArenaEncounter::Goblins);
    divider(&mut view, materials.stone);
    pose(&mut session, 0, Vec3::new(-9.0, 0.0, 0.0), Vec3::X);
    let home = session
        .encounter
        .runtime
        .first()
        .expect("party")
        .snapshot
        .home;
    for actor in session.actors.iter_mut().skip(1) {
        actor.feet = home + Vec3::new(5.0, 0.0, f32::from(actor.id) * 0.7);
        actor.previous_feet = actor.feet;
        actor.hp = 30.0;
    }
    let party = session.encounter.runtime.first_mut().expect("party");
    party.snapshot.phase = PartyPhase::Active;
    party.knowledge = Some(Knowledge {
        point: Vec3::new(-9.0, 0.0, 0.0),
        velocity: Vec3::ZERO,
        tick: session.tick,
        direct: true,
    });
    party.last_sight = session.tick;
    ticks(&mut session, 510, &view, geometry, materials, &tuning);
    assert_eq!(
        session.parties().first().expect("party").phase,
        PartyPhase::Returning
    );
    session.bot_enabled = true;
    ticks(&mut session, 600, &view, geometry, materials, &tuning);
    assert_eq!(
        session.parties().first().expect("party").phase,
        PartyPhase::Dormant
    );
    assert!(session
        .actors
        .iter()
        .skip(1)
        .all(|a| (a.hp - 30.0).abs() < 0.001));
}

#[test]
fn shaman_waits_for_reaction_charges_then_cancels_if_cover_closes_before_release() {
    let (mut session, mut view, geometry, materials, tuning) = fixture(ArenaEncounter::ShamanParty);
    pose(&mut session, 1, Vec3::new(-5.0, 0.0, 0.0), Vec3::X);
    pose(&mut session, 0, Vec3::new(5.0, 0.0, 0.0), Vec3::NEG_X);
    for a in session.actors.iter_mut().skip(2) {
        a.feet = Vec3::new(15.0, 0.0, f32::from(a.id) * 2.0);
        a.previous_feet = a.feet;
    }
    let mut brain = session.encounter.brains.remove(&1).expect("brain");
    let party = session.encounter.runtime.first_mut().expect("party");
    party.snapshot.phase = PartyPhase::Active;
    let mut first_press = None;
    let mut released = false;
    for tick in 0..140 {
        if tick == 90 {
            divider(&mut view, materials.stone);
            session.collision.refresh(&view, geometry);
        }
        let actor = session.actors.iter().find(|a| a.id == 1).expect("shaman");
        let party = session.encounter.runtime.first().expect("party");
        let (input, _) = brain.intent(
            actor,
            party,
            &session.actors,
            &[],
            &[],
            &session.collision,
            &view,
            geometry,
            &tuning,
            tick,
        );
        if input.input.cast_pressed {
            first_press.get_or_insert(tick);
        }
        released |= input.input.cast_released;
        let actor = session
            .actors
            .iter_mut()
            .find(|a| a.id == 1)
            .expect("shaman");
        if let Some(selected) = input.input.selected {
            actor.selected = selected;
        }
        actor.aim = input.input.aim;
        assert!(actor.casting(input.input, &tuning).is_none());
    }
    assert!(first_press.is_some_and(|tick| tick >= 42 && tick <= 44));
    assert!(!released);
    assert!(session
        .actors
        .iter()
        .find(|a| a.id == 1)
        .expect("shaman")
        .charge()
        .is_none());
}

#[test]
fn dragon_damage_triggers_retreat_flight_then_regeneration_and_landing() {
    let (mut session, view, geometry, materials, tuning) = fixture(ArenaEncounter::Dragon);
    pose(&mut session, 1, Vec3::ZERO, Vec3::NEG_Z);
    pose(&mut session, 0, Vec3::NEG_Z * 8.0, Vec3::Z);
    let party = session.encounter.runtime.first_mut().expect("party");
    party.snapshot.phase = PartyPhase::Active;
    session
        .actors
        .iter_mut()
        .find(|a| a.id == 1)
        .expect("dragon")
        .hp = 70.0;
    session.record_damage(0, 1, 30.0);
    session.bot_enabled = true;
    ticks(&mut session, 100, &view, geometry, materials, &tuning);
    let dragon = session.actors.iter().find(|a| a.id == 1).expect("dragon");
    assert!(dragon.flying && dragon.feet.y > 0.3);
    assert!((dragon.hp - 70.0).abs() < 0.001);
    assert!(!session.barriers().is_empty());
    session.bot_enabled = false;
    ticks(&mut session, 500, &view, geometry, materials, &tuning);
    let dragon = session.actors.iter().find(|a| a.id == 1).expect("dragon");
    assert!(dragon.hp > 72.5);
    assert!(!dragon.flying && dragon.grounded);
    assert!(session.actor_pose_valid(1, &view, geometry));
}

#[test]
fn aura_loses_range_and_los_without_stacking_and_releases_freeze_their_buff() {
    let (mut session, mut view, geometry, materials, tuning) = fixture(ArenaEncounter::ShamanParty);
    pose(&mut session, 1, Vec3::new(-3.0, 0.0, 0.0), Vec3::X);
    pose(&mut session, 2, Vec3::new(2.0, 0.0, 0.0), Vec3::X);
    pose(&mut session, 0, Vec3::new(7.0, 0.0, 0.0), Vec3::NEG_X);
    session
        .actors
        .iter_mut()
        .find(|a| a.id == 2)
        .expect("ally")
        .hp = 20.0;
    start(&mut session, 1, CreatureAbility::Aura, Vec3::X, &tuning);
    ticks(&mut session, 61, &view, geometry, materials, &tuning);
    let aura = *session.auras().first().expect("aura");
    session.encounter.auras.push(aura);
    let hp = session.actors.iter().find(|a| a.id == 2).expect("ally").hp;
    ticks(&mut session, 120, &view, geometry, materials, &tuning);
    let ally = session.actors.iter().find(|a| a.id == 2).expect("ally");
    assert!((ally.hp - hp - 3.0).abs() < 0.01);
    assert!((ally.damage_multiplier - 1.25).abs() < 0.001);
    let mut out = CommandsOut::default();
    session.release(
        2,
        Spell::Fireball,
        &tuning,
        32.0,
        &view,
        geometry,
        materials,
        &mut out,
    );
    session
        .actors
        .iter_mut()
        .find(|a| a.id == 1)
        .expect("shaman")
        .hp = 0.0;
    ticks(&mut session, 30, &view, geometry, materials, &tuning);
    assert!(
        session
            .encounter
            .stats
            .get(&2)
            .expect("caster stats")
            .damage_dealt
            > 35.0
    );
    assert!(
        (session
            .actors
            .iter()
            .find(|a| a.id == 2)
            .expect("ally")
            .damage_multiplier
            - 1.0)
            .abs()
            < 0.001
    );
    // A fresh aura cannot heal or buff through terrain and loses out-of-range allies.
    session
        .actors
        .iter_mut()
        .find(|a| a.id == 1)
        .expect("shaman")
        .hp = 60.0;
    session.encounter.auras = vec![AuraSnapshot {
        remaining: 5.0,
        ..aura
    }];
    divider(&mut view, materials.stone);
    session.collision.refresh(&view, geometry);
    session.refresh_support_buffs(&tuning);
    assert!(
        (session
            .actors
            .iter()
            .find(|a| a.id == 2)
            .expect("ally")
            .damage_multiplier
            - 1.0)
            .abs()
            < 0.001
    );
    pose(&mut session, 2, Vec3::new(-13.0, 0.0, 0.0), Vec3::X);
    session.refresh_support_buffs(&tuning);
    assert!(
        (session
            .actors
            .iter()
            .find(|a| a.id == 2)
            .expect("ally")
            .damage_multiplier
            - 1.0)
            .abs()
            < 0.001
    );
}

#[test]
fn direct_attacks_stop_at_cover_and_breath_pays_terrain_power_once_per_cell() {
    let (mut session, mut view, geometry, materials, tuning) = fixture(ArenaEncounter::Dragon);
    pose(&mut session, 1, Vec3::new(-3.5, 0.0, 0.0), Vec3::X);
    pose(&mut session, 0, Vec3::new(1.5, 0.0, 0.0), Vec3::NEG_X);
    divider(&mut view, materials.stone);
    start(&mut session, 1, CreatureAbility::FireCone, Vec3::X, &tuning);
    let impacts = ticks(&mut session, 160, &view, geometry, materials, &tuning);
    assert!((session.actors.first().expect("human").hp - 100.0).abs() < 0.001);
    let mut seen = std::collections::BTreeSet::new();
    assert!(!impacts.is_empty());
    for impact in impacts {
        assert_eq!(impact.power, 2);
        for pos in impact.volume {
            assert!(seen.insert(pos), "one terrain budget per breath and voxel");
            assert!(pos.coord.to_world(0.0).x.abs() < 1.0);
        }
    }
}

#[test]
fn aura_disappearing_during_melee_windup_does_not_buff_the_unreleased_hit() {
    let (mut session, view, geometry, materials, tuning) = fixture(ArenaEncounter::ShamanParty);
    pose(&mut session, 1, Vec3::new(-3.0, 0.0, 0.0), Vec3::X);
    pose(&mut session, 2, Vec3::ZERO, Vec3::X);
    pose(&mut session, 0, Vec3::X * 1.3, Vec3::NEG_X);
    session.encounter.auras.push(AuraSnapshot {
        owner: 1,
        center: Vec3::new(-3.0, 0.4, 0.0),
        radius: 6.0,
        remaining: 5.0,
        lifetime: 5.0,
    });
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
    start(&mut session, 2, CreatureAbility::Swipe, Vec3::X, &tuning);
    ticks(&mut session, 10, &view, geometry, materials, &tuning);
    session
        .actors
        .iter_mut()
        .find(|a| a.id == 1)
        .expect("shaman")
        .hp = 0.0;
    ticks(&mut session, 40, &view, geometry, materials, &tuning);
    assert!((session.actors.first().expect("human").hp - 88.0).abs() < 0.001);
}
