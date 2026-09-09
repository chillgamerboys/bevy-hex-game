//! Actual attack pulses against exposed and covered physical bodies.

use super::*;
use hex_core::{ElementId, SubstanceId};

fn goblin(feet: Vec3) -> Actor {
    let mut actor = Actor::spawn(1, feet, Vec3::NEG_Z);
    actor.species = Species::Goblin;
    actor.team = 2;
    actor.dimensions = Vec3::new(0.5, 0.8, 0.5);
    actor
}

fn fixture(target: Actor) -> (ArenaSession, Actor, ArenaTerrainView, ArenaTuning) {
    let tuning = ArenaTuning::default();
    let origin = Vec3::new(-1.5, 0.4, 0.0);
    let aim = (target.center() - origin).normalize();
    let mut owner = Actor::spawn(0, Vec3::ZERO, aim);
    owner.species = Species::Dragon;
    owner.team = 1;
    owner.dimensions = Vec3::new(
        tuning.encounters.dragon_width,
        tuning.encounters.dragon_height,
        tuning.encounters.dragon_length,
    );
    owner.feet = origin
        - owner.body_rotation() * Vec3::NEG_Z * (owner.dimensions.z * 0.5 - 0.05)
        - Vec3::Y * owner.dimensions.y * 0.5;
    let view = ArenaTerrainView {
        revision: 1,
        voxels: [1, 2]
            .map(|level| (TilePos::new(HexCoord::ORIGIN, level), SubstanceId(1)))
            .into(),
        ..Default::default()
    };
    let mut session = ArenaSession {
        actors: vec![owner.clone(), target],
        ..Default::default()
    };
    session.encounter.initialized = true;
    session
        .collision
        .refresh(&view, ArenaVoxelGeometry::default());
    assert!(owner.eye().distance(origin) < SKIN);
    for actor in &session.actors {
        assert!(shapes::clear(
            &session.collision,
            actor,
            actor.feet,
            actor.body_yaw
        ));
    }
    (session, owner, view, tuning)
}

fn breath(owner: &Actor) -> Cast {
    Cast {
        kind: CreatureAbility::FireCone,
        laser: None,
        age: 0.0,
        windup: 0.0,
        duration: 0.75,
        pulses: 0,
        direction: owner.aim,
        team: owner.team,
        multiplier: 1.0,
        actor_damage: BTreeMap::new(),
        voxels: BTreeSet::new(),
        barrier_damage: BTreeMap::new(),
    }
}

fn pulse(
    session: &mut ArenaSession,
    owner: &Actor,
    cast: &mut Cast,
    view: &ArenaTerrainView,
    tuning: &ArenaTuning,
    out: &mut CommandsOut,
) {
    session.direct_pulse(
        owner,
        cast,
        tuning.encounters.breath_range,
        tuning.encounters.breath_angle.to_radians() * 0.5,
        view,
        ArenaVoxelGeometry::default(),
        ArenaMaterials {
            stone: SubstanceId(1),
            grass: SubstanceId(2),
            dirt: SubstanceId(3),
            bedrock: SubstanceId(4),
            fire: ElementId(1),
        },
        tuning,
        out,
    );
}

fn hp(session: &ArenaSession) -> f32 {
    session
        .actors
        .iter()
        .find(|a| a.id == 1)
        .expect("target")
        .hp
}

#[test]
fn exposed_capsule_flank_receives_pulses_even_when_nearest_contact_is_occluded() {
    let target = goblin(Vec3::new(1.5, 0.0, 2.3));
    let (mut session, owner, view, tuning) = fixture(target.clone());
    let nearest = target.center() - owner.aim * 0.25;
    assert!(session
        .collision
        .attack_sweep(owner.eye(), nearest - owner.eye(), 0.0)
        .is_some());
    let exposed = target.center() + Vec3::Z * 0.25;
    assert!(shapes::distance(exposed, &target) < SKIN);
    assert!(session
        .collision
        .attack_sweep(owner.eye(), exposed - owner.eye(), 0.0)
        .is_none());
    assert!(owner.eye().distance(exposed) < tuning.encounters.breath_range);

    let mut cast = breath(&owner);
    let mut out = CommandsOut::default();
    let before = hp(&session);
    pulse(&mut session, &owner, &mut cast, &view, &tuning, &mut out);
    assert!((before - hp(&session) - tuning.encounters.breath_damage / 3.0).abs() < 0.001);
    for _ in 0..3 {
        pulse(&mut session, &owner, &mut cast, &view, &tuning, &mut out);
    }
    assert!((before - hp(&session) - tuning.encounters.breath_damage).abs() < 0.001);
    assert_eq!(
        out.impacts
            .iter()
            .flat_map(|impact| impact.volume.iter())
            .collect::<Vec<_>>()
            .len(),
        cast.voxels.len(),
        "the alternate body contact must not repeat terrain damage within a cast"
    );
}

#[test]
fn full_cover_range_and_angle_still_reject_body_damage() {
    for (feet, range, angle) in [
        (Vec3::new(1.5, 0.0, 0.0), 4.0, 50.0),
        // The nearest blocked point is 3.5302 units away. A 3.55-unit cone
        // legitimately reaches an exposed grazing flank, so keep this cap below
        // that flank while still including the original blocked contact.
        (Vec3::new(1.5, 0.0, 2.3), 3.531, 50.0),
        (Vec3::new(1.5, 0.0, 2.3), 4.0, 1.0),
    ] {
        let (mut session, owner, view, mut tuning) = fixture(goblin(feet));
        tuning.encounters.breath_range = range;
        tuning.encounters.breath_angle = angle;
        let mut cast = breath(&owner);
        let before = hp(&session);
        pulse(
            &mut session,
            &owner,
            &mut cast,
            &view,
            &tuning,
            &mut CommandsOut::default(),
        );
        assert_eq!(
            hp(&session).to_bits(),
            before.to_bits(),
            "covered or out-of-cone target at {feet:?}, range={range}, angle={angle}"
        );
    }
}

#[test]
fn transparent_attack_barrier_blocks_all_fallback_contacts() {
    let (mut session, owner, mut view, tuning) = fixture(goblin(Vec3::new(1.5, 0.0, 2.3)));
    view.voxels.clear();
    view.revision += 1;
    session
        .collision
        .refresh(&view, ArenaVoxelGeometry::default());
    session.encounter.barriers.push(BarrierSnapshot {
        id: 99,
        owner: 1,
        center: Vec3::new(0.0, 0.4, 1.2),
        normal: Vec3::X,
        width: 4.0,
        height: 2.0,
        hp: 60.0,
        max_hp: 60.0,
        remaining: 4.0,
        lifetime: 4.0,
    });
    session.collision.sync_barriers(&session.encounter.barriers);
    assert!(session
        .collision
        .sight_clear(owner.eye(), Vec3::new(1.5, 0.4, 2.3)));
    let before = hp(&session);
    pulse(
        &mut session,
        &owner,
        &mut breath(&owner),
        &view,
        &tuning,
        &mut CommandsOut::default(),
    );
    assert_eq!(hp(&session).to_bits(), before.to_bits());
    assert!(session.encounter.barriers.first().expect("barrier").hp < 60.0);
}

#[test]
fn elongated_rotated_body_contacts_remain_inside_its_real_shape_and_cone() {
    for yaw in [0.0, std::f32::consts::FRAC_PI_2] {
        let tuning = ArenaTuning::default();
        let mut target = goblin(Vec3::new(1.9, 0.2, 3.0));
        target.species = Species::Dragon;
        target.dimensions = Vec3::new(
            tuning.encounters.dragon_width,
            tuning.encounters.dragon_height,
            tuning.encounters.dragon_length,
        );
        target.body_yaw = yaw;
        let (session, owner, _, _) = fixture(target.clone());
        let angle = tuning.encounters.breath_angle.to_radians() * 0.5;
        let mut queries = 0;
        let point = shapes::exposed_cone_contact(
            &target,
            owner.eye(),
            owner.aim,
            tuning.encounters.breath_range,
            angle,
            |point| {
                queries += 1;
                session
                    .collision
                    .attack_sweep(owner.eye(), point - owner.eye(), 0.0)
                    .is_none()
            },
        )
        .expect("the real Dragon has an exposed in-range flank");
        let delta = point - owner.eye();
        assert!(shapes::distance(point, &target) < SKIN);
        assert!(delta.length() <= tuning.encounters.breath_range + SKIN);
        assert!(delta.normalize().dot(owner.aim) >= angle.cos() - SKIN);
        assert!(queries <= 27, "contact queries have a fixed per-body bound");
    }
}
