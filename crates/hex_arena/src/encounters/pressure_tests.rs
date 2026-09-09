//! Decision and production-motion evidence for the pressure follow-up.
use super::*;
use hex_core::{ElementId, SubstanceId};

fn scene(
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
    let tuning = ArenaTuning::default();
    let geometry = ArenaVoxelGeometry {
        radius: 20,
        ..Default::default()
    };
    let view = ArenaTerrainView {
        voxels: HexCoord::ORIGIN
            .within_radius(20)
            .into_iter()
            .map(|coord| (TilePos::new(coord, 0), SubstanceId(1)))
            .collect(),
        ..Default::default()
    };
    let mut actor = Actor::spawn(7, Vec3::new(-10.0, SKIN, 0.0), Vec3::X);
    actor.configure_species(species, &tuning.encounters);
    actor.party = Some(1);
    actor.team = 1;
    actor.body_yaw = -std::f32::consts::FRAC_PI_2;
    actor.grounded = true;
    actor.body.grounded = true;
    if species == Species::Wisp {
        actor.feet.y += 4.0;
        actor.flight_layer = Some(0);
    }
    let mut target = Actor::spawn(0, Vec3::new(10.0, SKIN, 0.0), Vec3::NEG_X);
    target.team = 0;
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
        leash: 40.0,
        search: 8.0,
        battle_search: None,
    };
    let mut collision = CollisionWorld::default();
    collision.refresh(&view, geometry);
    (actor, target, party, view, geometry, collision, tuning)
}

fn materials() -> ArenaMaterials {
    ArenaMaterials {
        stone: SubstanceId(1),
        grass: SubstanceId(2),
        dirt: SubstanceId(3),
        bedrock: SubstanceId(4),
        fire: ElementId(1),
    }
}

fn wall(view: &mut ArenaTerrainView, geometry: ArenaVoxelGeometry, collision: &mut CollisionWorld) {
    for r in -18..=18 {
        for level in 1..=24 {
            view.voxels.insert(
                TilePos::new(HexCoord::from_axial(0, r), level),
                SubstanceId(1),
            );
        }
    }
    view.revision += 1;
    collision.refresh(view, geometry);
}

#[test]
fn wisp_cover_budget_and_goal_ignore_hidden_motion_and_party_cues() {
    let (actor, target, mut party, mut view, geometry, mut collision, tuning) =
        scene(Species::Wisp);
    let mut a = Brain::new(actor.id, actor.feet);
    let mut b = Brain::new(actor.id, actor.feet);
    let observed = [target.clone(), actor.clone()];
    for brain in [&mut a, &mut b] {
        brain.intent(
            &actor,
            &party,
            &observed,
            &[],
            &[],
            &collision,
            &view,
            geometry,
            &tuning,
            1,
        );
    }
    wall(&mut view, geometry, &mut collision);
    party.knowledge = Some(Knowledge {
        point: Vec3::new(12.0, 0.0, 8.0),
        velocity: Vec3::ZERO,
        tick: 30,
        direct: false,
        cue_kind: Some(CombatCueKind::Impact),
        observed: None,
    });
    let mut moved = target.clone();
    moved.feet.z += 8.0;
    let hidden_a = [target, actor.clone()];
    let hidden_b = [moved, actor.clone()];
    for actors in [&hidden_a, &hidden_b] {
        let hidden = actors.first().expect("hidden hostile");
        assert!(!collision.sight_clear(actor.eye(), hidden.eye()));
        assert!(!collision.sight_clear(actor.eye(), hidden.center()));
    }
    for (brain, actors) in [(&mut a, &hidden_a), (&mut b, &hidden_b)] {
        brain.intent(
            &actor,
            &party,
            actors,
            &[],
            &[],
            &collision,
            &view,
            geometry,
            &tuning,
            31,
        );
        assert!(brain
            .ember_release_aim(
                &actor,
                &collision,
                &view,
                geometry,
                &tuning,
                materials(),
                31
            )
            .is_some());
        assert!(
            Vec3::from_array(brain.decision.as_ref().expect("decision").goal).x
                > actor.feet.x + 1.0,
            "hidden memory must seek closer instead of preserving the distant firing band"
        );
        brain.ember_released();
        brain.intent(
            &actor,
            &party,
            actors,
            &[],
            &[],
            &collision,
            &view,
            geometry,
            &tuning,
            260,
        );
        brain.ember_released();
        brain.intent(
            &actor,
            &party,
            actors,
            &[],
            &[],
            &collision,
            &view,
            geometry,
            &tuning,
            261,
        );
        assert!(brain.ember_target.is_none());
        assert_eq!(brain.ember_cover_remaining, 0);
    }
    assert_eq!(format!("{a:?}"), format!("{b:?}"));
    a.intent(
        &actor,
        &party,
        &hidden_a,
        &[],
        &[],
        &collision,
        &view,
        geometry,
        &tuning,
        600,
    );
    assert!(
        a.ember_target.is_none(),
        "expired own memory cannot be revived by a party cue"
    );
    view.voxels.retain(|pos, _| pos.level == 0);
    view.revision += 1;
    collision.refresh(&view, geometry);
    a.intent(
        &actor,
        &party,
        &hidden_a,
        &[],
        &[],
        &collision,
        &view,
        geometry,
        &tuning,
        601,
    );
    assert_eq!(a.ember_cover_remaining, tuning.encounters.wisp_cover_shots);
}

fn release(target: &Actor, tick: u64) -> CombatCue {
    CombatCue {
        id: tick,
        tick,
        owner: target.id,
        team: target.team,
        position: target.eye(),
        kind: CombatCueKind::Release,
    }
}

#[test]
fn dragon_burst_requires_disclosed_pressure_and_preserves_destination_cooldown_and_retreat() {
    let (mut actor, target, party, view, geometry, collision, tuning) = scene(Species::Dragon);
    let mut brain = Brain::new(actor.id, actor.feet);
    let actors = [target.clone(), actor.clone()];
    let (quiet, _) = brain.intent(
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
    assert!(!quiet.lunge, "visible silent target is not pressure");
    let cues = [release(&target, 2)];
    let (burst, _) = brain.intent(
        &actor,
        &party,
        &actors,
        &[],
        &cues,
        &collision,
        &view,
        geometry,
        &tuning,
        2,
    );
    assert!(burst.lunge && burst.flight);
    let destination = brain.lunge.expect("committed burst").0;
    let mut moved = target.clone();
    moved.feet.z = 6.0;
    brain.intent(
        &actor,
        &party,
        &[moved, actor.clone()],
        &[],
        &cues,
        &collision,
        &view,
        geometry,
        &tuning,
        20,
    );
    assert_eq!(brain.lunge.expect("same burst").0, destination);
    let (cooling, _) = brain.intent(
        &actor,
        &party,
        &actors,
        &[],
        &[release(&target, 120)],
        &collision,
        &view,
        geometry,
        &tuning,
        120,
    );
    assert!(!cooling.lunge);
    actor.last_damage_tick = Some(121);
    let (retreat, _) = brain.intent(
        &actor,
        &party,
        &[target.clone(), actor.clone()],
        &[],
        &[release(&target, 121)],
        &collision,
        &view,
        geometry,
        &tuning,
        121,
    );
    assert!(!retreat.lunge && brain.lunge.is_none());
    assert!(brain.decision.as_ref().expect("retreat").retreat_seconds > 0.0);
    actor.last_damage_tick = None;
    actor.hp = actor.max_hp * 0.5;
    let (hurt, _) = brain.intent(
        &actor,
        &party,
        &[target.clone(), actor.clone()],
        &[],
        &[release(&target, 1200)],
        &collision,
        &view,
        geometry,
        &tuning,
        1200,
    );
    assert!(!hurt.lunge, "low health never launches an approach burst");
}

#[test]
fn dragon_lunge_uses_swept_speed_and_never_crosses_solid_cover() {
    let (actor, _, _, mut view, geometry, mut collision, tuning) = scene(Species::Dragon);
    let mut ordinary = actor.clone();
    let mut fast = actor.clone();
    motion::tick(
        &mut ordinary,
        Vec3::X,
        true,
        false,
        true,
        &collision,
        &tuning.encounters,
    );
    motion::tick_with_lunge(
        &mut fast,
        Vec3::X,
        true,
        false,
        true,
        true,
        &collision,
        &tuning.encounters,
    );
    assert!(fast.feet.x > ordinary.feet.x);
    wall(&mut view, geometry, &mut collision);
    for _ in 0..200 {
        motion::tick_with_lunge(
            &mut fast,
            Vec3::X,
            true,
            false,
            true,
            true,
            &collision,
            &tuning.encounters,
        );
        assert!(shapes::clear(&collision, &fast, fast.feet, fast.body_yaw));
    }
    assert!(fast.feet.x < 0.0);
}

#[test]
fn goblin_slots_pursue_spotted_targets_beyond_shaman_support_range() {
    let (mut shaman, target, mut party, view, geometry, collision, tuning) = scene(Species::Shaman);
    shaman.feet.x = -5.0;
    let mut allies = vec![target.clone(), shaman.clone()];
    for id in 8..18 {
        let mut goblin = Actor::spawn(id, Vec3::new(-2.0, SKIN, f32::from(id - 8) * 0.4), Vec3::X);
        goblin.configure_species(Species::Goblin, &tuning.encounters);
        goblin.party = shaman.party;
        goblin.team = shaman.team;
        allies.push(goblin);
    }
    let goals: Vec<_> = allies
        .iter()
        .filter(|a| a.species == Species::Goblin)
        .map(|a| {
            goblin_approach(
                a,
                &allies,
                party.snapshot.home,
                target.center(),
                &tuning.encounters,
            )
        })
        .collect();
    for (index, goal) in goals.iter().enumerate() {
        assert!(goals
            .iter()
            .skip(index + 1)
            .all(|other| goal.distance(*other) > 0.2));
    }
    for goblin in allies.iter().filter(|a| a.species == Species::Goblin) {
        let mut brain = Brain::new(goblin.id, goblin.feet);
        brain.intent(
            goblin,
            &party,
            &allies,
            &[],
            &[],
            &collision,
            &view,
            geometry,
            &tuning,
            1,
        );
        let goal = Vec3::from_array(brain.decision.as_ref().expect("escort").goal);
        assert!(goal.distance(shaman.feet) > tuning.encounters.aura_radius * 0.8);
        assert!(goal.distance(target.center()) < 3.0);
        assert!(
            goal.x > goblin.feet.x,
            "escort must pursue the spotted player"
        );
        party.snapshot.phase = PartyPhase::Dormant;
        brain.intent(
            goblin,
            &party,
            &allies,
            &[],
            &[],
            &collision,
            &view,
            geometry,
            &tuning,
            2,
        );
        let idle = Vec3::from_array(brain.decision.expect("idle escort").goal);
        assert!(idle.distance(shaman.feet) <= tuning.encounters.aura_radius * 0.8 + 0.01);
        party.snapshot.phase = PartyPhase::Active;
    }
    let mut brain = Brain::new(shaman.id, shaman.feet);
    brain.intent(
        &shaman,
        &party,
        &allies,
        &[],
        &[],
        &collision,
        &view,
        geometry,
        &tuning,
        1,
    );
    let goal = Vec3::from_array(brain.decision.expect("support").goal);
    assert!(
        goal.distance(frontline_center(&shaman, &allies).expect("escorts"))
            < tuning.encounters.aura_radius
    );
}

#[test]
fn goblin_periodic_jumps_have_proved_landings_and_keep_human_height_unchanged() {
    let (mut actor, _, _, view, geometry, collision, tuning) = scene(Species::Goblin);
    actor.feet.x = -20.0;
    let mut steering = steering::Steering::default();
    let mut starts = Vec::new();
    let mut landed = 0;
    let mut previous_grounded = true;
    let mut highest = 0.0_f32;
    for tick in 1..=720 {
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
        if jump {
            starts.push(tick);
        }
        motion::tick(
            &mut actor,
            direction,
            true,
            jump,
            false,
            &collision,
            &tuning.encounters,
        );
        highest = highest.max(actor.feet.y);
        landed += usize::from(actor.grounded && !previous_grounded);
        previous_grounded = actor.grounded;
        assert!(shapes::clear(
            &collision,
            &actor,
            actor.feet,
            actor.body_yaw
        ));
    }
    assert!(
        starts.len() >= 2 && landed >= 2,
        "starts={starts:?}, landings={landed}"
    );
    assert!(highest > 2.7 && highest < 2.81, "actual apex={highest}");
    for pair in starts.windows(2) {
        let earlier = pair.first().expect("two-entry window start");
        let later = pair.get(1).expect("two-entry window end");
        let gap = elapsed(*later, *earlier);
        assert!(
            gap >= tuning.encounters.goblin_jump_interval_min
                && gap <= tuning.encounters.goblin_jump_interval_max + STEP
        );
    }
    let mut human = Actor::spawn(0, Vec3::new(-15.0, SKIN, 0.0), Vec3::X);
    let mut human_apex = 0.0_f32;
    for tick in 0..160 {
        motion::tick(
            &mut human,
            Vec3::ZERO,
            false,
            tick == 0,
            false,
            &collision,
            &tuning.encounters,
        );
        human_apex = human_apex.max(human.feet.y);
    }
    assert!(human_apex > 1.29 && human_apex < 1.31 && human.grounded);
}

#[test]
fn high_goblin_jump_clears_a_six_level_crater_lip_and_lands() {
    let (mut actor, _, _, mut view, geometry, mut collision, tuning) = scene(Species::Goblin);
    actor.feet.x = -3.0;
    for coord in HexCoord::ORIGIN.within_radius(20) {
        if coord.to_world(0.0).x > 0.0 {
            for level in 1..=6 {
                view.voxels
                    .insert(TilePos::new(coord, level), SubstanceId(1));
            }
        }
    }
    view.revision += 1;
    collision.refresh(&view, geometry);
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
            &tuning.encounters,
            tick,
        );
        jumped |= jump;
        motion::tick(
            &mut actor,
            direction,
            true,
            jump,
            false,
            &collision,
            &tuning.encounters,
        );
        assert!(shapes::clear(
            &collision,
            &actor,
            actor.feet,
            actor.body_yaw
        ));
        if actor.grounded && actor.feet.x > 3.0 {
            break;
        }
    }
    assert!(
        jumped && actor.grounded && actor.feet.x > 3.0 && actor.feet.y > 2.39,
        "feet={:?}",
        actor.feet
    );
}
