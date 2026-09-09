//! Production-controller regressions for the crater and retreat failures.

use super::*;

fn walker(species: Species, feet: Vec3) -> Actor {
    let mut actor = Actor::spawn(7, feet + Vec3::Y * SKIN, Vec3::X);
    actor.species = species;
    actor.team = 7;
    actor.grounded = true;
    actor.body.grounded = true;
    actor
}

fn lip(
    height: i32,
) -> (
    ArenaTerrainView,
    ArenaVoxelGeometry,
    CollisionWorld,
    EncounterTuning,
) {
    let (_, mut view, geometry, _, tuning) = fixture(ArenaEncounter::Goblins);
    for coord in HexCoord::ORIGIN.within_radius(geometry.radius) {
        if coord.to_world(0.0).x > 0.0 {
            for level in 1..=height {
                view.voxels
                    .insert(TilePos::new(coord, level), SubstanceId(1));
            }
        }
    }
    view.revision += 1;
    let mut collision = CollisionWorld::default();
    collision.refresh(&view, geometry);
    (view, geometry, collision, tuning.encounters)
}

#[test]
fn goblin_and_shaman_escape_a_three_level_lip_by_real_jump_and_landing() {
    let (view, geometry, collision, tuning) = lip(3);
    for species in [Species::Goblin, Species::Shaman] {
        let mut actor = walker(species, Vec3::new(-2.0, 0.0, 0.0));
        let mut steering = steering::Steering::default();
        let mut jumped = false;
        for tick in 1..=600 {
            let (direction, jump) = steering.travel(
                &actor,
                Vec3::X,
                false,
                true,
                &collision,
                &view,
                geometry,
                &tuning,
                tick,
            );
            jumped |= jump;
            motion::tick(
                &mut actor, direction, true, jump, false, &collision, &tuning,
            );
            assert!(shapes::clear(
                &collision,
                &actor,
                actor.feet,
                actor.body_yaw
            ));
            assert!(dry(&actor, &view, geometry));
            if actor.feet.x > 3.0 && actor.grounded {
                break;
            }
        }
        assert!(
            jumped && actor.grounded && actor.feet.x > 3.0 && actor.feet.y > 1.19,
            "{species:?} must actually leave the crater and land, jumped={jumped}: {:?}",
            actor.feet
        );
    }
}

#[test]
fn unsupported_edge_and_dormant_patrol_never_admit_an_unproved_jump() {
    let (_, mut view, geometry, _, tuning) = fixture(ArenaEncounter::Goblins);
    view.voxels.retain(|pos, _| pos.coord.to_world(0.0).x < 0.0);
    view.revision += 1;
    let mut collision = CollisionWorld::default();
    collision.refresh(&view, geometry);
    let mut actor = walker(Species::Goblin, Vec3::new(-2.0, 0.0, 0.0));
    let mut steering = steering::Steering::default();
    for tick in 1..=360 {
        let (direction, jump) = steering.travel(
            &actor,
            Vec3::X,
            false,
            true,
            &collision,
            &view,
            geometry,
            &tuning.encounters,
            tick,
        );
        motion::tick(
            &mut actor,
            direction,
            true,
            jump,
            false,
            &collision,
            &tuning.encounters,
        );
        assert!(
            actor.feet.y > -0.401,
            "falling a little is not proof of ground"
        );
        if !steering.jumping() {
            assert!(
                shapes::ground(&collision, &actor, actor.feet, 0.401).is_some(),
                "an ordinary step must keep support: tick={tick} feet={:?}",
                actor.feet
            );
        }
        assert!(shapes::clear(
            &collision,
            &actor,
            actor.feet,
            actor.body_yaw
        ));
    }
    let (view, geometry, collision, tuning) = lip(3);
    let mut actor = walker(Species::Goblin, Vec3::new(-2.0, 0.0, 0.0));
    let mut steering = steering::Steering::default();
    for tick in 1..=360 {
        let (direction, jump) = steering.travel(
            &actor,
            Vec3::X,
            false,
            false,
            &collision,
            &view,
            geometry,
            &tuning,
            tick,
        );
        assert!(!jump, "Dormant walk must not replay a running jump");
        motion::tick(
            &mut actor, direction, false, jump, false, &collision, &tuning,
        );
    }
    assert!(actor.feet.x < 1.0 && actor.feet.y < 0.01);
}

#[test]
fn long_dragon_edge_between_resident_corners_is_rejected() {
    let tuning = EncounterTuning::default();
    let mut actor = walker(Species::Dragon, Vec3::new(18.261_992, 0.0, 2.328_019_4));
    actor.dimensions = Vec3::new(
        tuning.dragon_width,
        tuning.dragon_height,
        tuning.dragon_length,
    );
    actor.body_yaw = 1.402_431_8;
    let geometry = ArenaVoxelGeometry {
        radius: 12,
        ..Default::default()
    };
    let half = actor.dimensions * 0.5;
    for x in [-1.0, 1.0] {
        for z in [-1.0, 1.0] {
            let corner =
                actor.feet + actor.body_rotation() * Vec3::new(x * half.x, 0.0, z * half.z);
            assert!(geometry.contains_column(HexCoord::from_world(corner)));
        }
    }
    assert!(!geometry.contains_column(HexCoord::from_world(Vec3::new(19.987_247, 0.0, 2.621_267))));
    assert!(!steering::contained(&actor, geometry));
}

#[test]
fn outside_flyer_can_reenter_after_outward_knockback_decays() {
    let (_, view, _, _, tuning) = fixture(ArenaEncounter::Dragon);
    let geometry = ArenaVoxelGeometry {
        radius: 12,
        ..Default::default()
    };
    let mut collision = CollisionWorld::default();
    collision.refresh(&view, geometry);
    let mut actor = walker(Species::Dragon, Vec3::new(23.0, 2.0, 0.0));
    actor.dimensions = Vec3::new(
        tuning.encounters.dragon_width,
        tuning.encounters.dragon_height,
        tuning.encounters.dragon_length,
    );
    actor.flying = true;
    actor.grounded = false;
    actor.body.grounded = false;
    actor.body.impulse_velocity = Vec3::X * 4.0;
    let mut steering = steering::Steering::default();
    assert!(!steering::contained(&actor, geometry));
    for tick in 1..=600 {
        let (direction, jump) = steering.travel(
            &actor,
            Vec3::NEG_X,
            true,
            true,
            &collision,
            &view,
            geometry,
            &tuning.encounters,
            tick,
        );
        assert!(!jump);
        motion::tick(
            &mut actor,
            direction,
            true,
            false,
            true,
            &collision,
            &tuning.encounters,
        );
        assert!(shapes::clear(
            &collision,
            &actor,
            actor.feet,
            actor.body_yaw
        ));
        if steering::contained(&actor, geometry) {
            return;
        }
    }
    panic!(
        "clear inward reentry must not be stuck outside: {:?}",
        actor.feet
    );
}

fn visible_pair(
    species: Species,
) -> (
    Actor,
    Actor,
    PartyRuntime,
    ArenaTerrainView,
    ArenaVoxelGeometry,
    CollisionWorld,
    ArenaTuning,
) {
    let (_, view, geometry, _, tuning) = fixture(ArenaEncounter::Dragon);
    let mut actor = walker(species, Vec3::ZERO);
    actor.party = Some(1);
    if species == Species::Dragon {
        actor.dimensions = Vec3::new(
            tuning.encounters.dragon_width,
            tuning.encounters.dragon_height,
            tuning.encounters.dragon_length,
        );
        actor.body_yaw = -std::f32::consts::FRAC_PI_2;
    }
    let mut target = walker(Species::Goblin, Vec3::X * 8.0);
    target.id = 9;
    target.team = 42;
    let party = PartyRuntime {
        snapshot: PartySnapshot {
            id: 1,
            phase: PartyPhase::Active,
            home: actor.feet,
            living: 1,
        },
        knowledge: None,
        last_sight: 0,
        last_cue_id: None,
        leash: 30.0,
        search: 8.0,
        battle_search: Some(target.feet),
    };
    let mut collision = CollisionWorld::default();
    collision.refresh(&view, geometry);
    (actor, target, party, view, geometry, collision, tuning)
}

#[test]
fn new_damage_refreshes_retreat_without_moving_its_fixed_destination() {
    let (mut actor, target, party, view, geometry, collision, tuning) =
        visible_pair(Species::Dragon);
    let mut brain = brain::Brain::for_battle(actor.id, actor.feet, 1);
    actor.last_damage_tick = Some(100);
    brain.intent(
        &actor,
        &party,
        &[actor.clone(), target.clone()],
        &[],
        &[],
        &collision,
        &view,
        geometry,
        &tuning,
        101,
    );
    let initial = brain.decision.clone().expect("decision");
    brain.intent(
        &actor,
        &party,
        &[actor.clone(), target.clone()],
        &[],
        &[],
        &collision,
        &view,
        geometry,
        &tuning,
        160,
    );
    let aged = brain.decision.clone().expect("decision");
    actor.last_damage_tick = Some(160);
    brain.intent(
        &actor,
        &party,
        &[actor.clone(), target.clone()],
        &[],
        &[],
        &collision,
        &view,
        geometry,
        &tuning,
        161,
    );
    let refreshed = brain.decision.clone().expect("decision");
    assert_eq!(
        initial.goal.map(f32::to_bits),
        refreshed.goal.map(f32::to_bits)
    );
    assert!(refreshed.retreat_seconds > aged.retreat_seconds + 0.45);
    assert!((refreshed.retreat_seconds - (4.0 - STEP)).abs() < 0.001);
    actor.hp = actor.max_hp * 0.4;
    brain.intent(
        &actor,
        &party,
        &[actor.clone(), target],
        &[],
        &[],
        &collision,
        &view,
        geometry,
        &tuning,
        162,
    );
    assert!(brain.decision.expect("hurt retreat").retreat_seconds > 7.9);
}

#[test]
fn shaman_approaches_a_dated_target_until_it_has_its_own_useful_angle() {
    let (actor, target, mut party, mut view, geometry, mut collision, tuning) =
        visible_pair(Species::Shaman);
    for level in 1..=6 {
        view.voxels.insert(
            TilePos::new(HexCoord::from_axial(2, 0), level),
            SubstanceId(1),
        );
    }
    view.revision += 1;
    collision.refresh(&view, geometry);
    assert!(!collision.sight_clear(actor.eye(), target.eye()));
    party.knowledge = Some(Knowledge {
        point: target.feet,
        velocity: Vec3::ZERO,
        tick: 1,
        direct: true,
        cue_kind: None,
        observed: None,
    });
    let mut brain = brain::Brain::for_battle(actor.id, actor.feet, 1);
    let (intent, _) = brain.intent(
        &actor,
        &party,
        &[actor.clone(), target],
        &[],
        &[],
        &collision,
        &view,
        geometry,
        &tuning,
        12,
    );
    let decision = brain.decision.expect("seek");
    assert!(!decision.own_sight && !decision.useful_shot);
    assert!(
        decision.goal[0] > 7.0 && intent.direction.x > 0.5,
        "remembered 8u spacing must not make the covered Shaman stop"
    );
}

#[test]
fn blocked_visible_dragon_uses_a_clear_flight_approach() {
    let (actor, target, party, view, geometry, collision, tuning) = visible_pair(Species::Dragon);
    let mut brain = brain::Brain::for_battle(actor.id, actor.feet, 1);
    let actors = [actor.clone(), target];
    let (initial, _) = brain.intent(
        &actor,
        &party,
        &actors,
        &[],
        &[],
        &collision,
        &view,
        geometry,
        &tuning,
        1,
    );
    assert!(!initial.flight);
    // The body has made no progress despite a live visible target and movement.
    let (recovery, _) = brain.intent(
        &actor,
        &party,
        &actors,
        &[],
        &[],
        &collision,
        &view,
        geometry,
        &tuning,
        61,
    );
    assert!(recovery.flight && recovery.direction.y > 0.0);
    let decision = brain.decision.expect("recovery");
    let mut at_goal = actor;
    at_goal.feet = Vec3::from_array(decision.goal);
    assert!(steering::contained(&at_goal, geometry));
    assert!(shapes::clear(
        &collision,
        &at_goal,
        at_goal.feet,
        at_goal.body_yaw
    ));
}

#[test]
fn goblin_and_shaman_descend_a_deep_dry_crater_without_an_unnecessary_jump() {
    let (view, geometry, collision, tuning) = lip(8);
    for species in [Species::Goblin, Species::Shaman] {
        let mut actor = walker(species, Vec3::new(3.0, 3.2, 0.0));
        let mut steering = steering::Steering::default();
        let mut airborne = false;
        for tick in 1..=600 {
            let (direction, jump) = steering.travel(
                &actor,
                Vec3::NEG_X,
                false,
                true,
                &collision,
                &view,
                geometry,
                &tuning,
                tick,
            );
            assert!(!jump, "a proved downward landing should not launch upward");
            motion::tick(
                &mut actor, direction, true, jump, false, &collision, &tuning,
            );
            airborne |= !actor.grounded;
            assert!(shapes::clear(
                &collision,
                &actor,
                actor.feet,
                actor.body_yaw
            ));
            assert!(dry(&actor, &view, geometry) && steering::contained(&actor, geometry));
            if airborne && actor.grounded && actor.feet.y < 0.01 {
                break;
            }
        }
        assert!(
            airborne && actor.grounded && actor.feet.y < 0.01,
            "{species:?} must actually reach the3.2u lower floor: {:?}",
            actor.feet
        );
        assert!(shapes::ground(&collision, &actor, actor.feet, 0.001).is_some());
    }
}

#[test]
fn spectator_dragon_searches_after_lost_sight_without_following_hidden_truth() {
    let mut paths = Vec::new();
    for hidden_z in [-5.0, 5.0] {
        let (mut actor, mut target, party, mut view, geometry, mut collision, tuning) =
            visible_pair(Species::Dragon);
        let mut brain = brain::Brain::for_battle(actor.id, actor.feet, 1);
        brain.intent(
            &actor,
            &party,
            &[actor.clone(), target.clone()],
            &[],
            &[],
            &collision,
            &view,
            geometry,
            &tuning,
            1,
        );
        actor.feet = party.battle_search.expect("public opposing deployment") + Vec3::Y * 2.0;
        actor.previous_feet = actor.feet;
        actor.flying = true;
        actor.grounded = false;
        actor.body.grounded = false;
        target.feet = Vec3::new(-10.0, SKIN, hidden_z);
        target.previous_feet = target.feet;
        divider(&mut view, SubstanceId(1));
        collision.refresh(&view, geometry);
        let start = actor.feet;
        let mut path = Vec::new();
        for tick in 13..=193 {
            assert!(!collision.sight_clear(actor.eye(), target.eye()));
            let (motion, request) = brain.intent(
                &actor,
                &party,
                &[actor.clone(), target.clone()],
                &[],
                &[],
                &collision,
                &view,
                geometry,
                &tuning,
                tick,
            );
            assert!(
                request.is_none(),
                "a search waypoint is not an observed enemy"
            );
            actor.aim = motion.input.aim;
            motion::tick(
                &mut actor,
                motion.direction,
                motion.input.run,
                motion.input.jump,
                motion.flight,
                &collision,
                &tuning.encounters,
            );
            assert!(shapes::clear(
                &collision,
                &actor,
                actor.feet,
                actor.body_yaw
            ));
            assert!(steering::contained(&actor, geometry));
            path.push((
                actor.feet.to_array().map(f32::to_bits),
                brain
                    .decision
                    .as_ref()
                    .expect("decision")
                    .goal
                    .map(f32::to_bits),
            ));
        }
        assert!(
            actor.feet.distance(start) > 2.0,
            "expired memory cannot leave a hovering statue"
        );
        paths.push(path);
    }
    assert_eq!(
        paths.first(),
        paths.get(1),
        "silent hidden changes must not steer the patrol"
    );
}
