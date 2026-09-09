//! The user-authorized earth sensor is private to activated, buried Worms.
use super::*;

fn until_phase(f: &mut Fixture, phase: WormPhase) {
    for _ in 0..1200 {
        let out = f.advance();
        f.apply(&out.burrows);
        if f.worm().worm().is_some_and(|state| state.phase == phase) {
            return;
        }
    }
    panic!(
        "phase {phase:?} not reached: {:?}",
        f.session.encounter.worms.get(&7)
    );
}

fn cover_between(f: &mut Fixture) {
    for coord in HexCoord::ORIGIN
        .within_radius(16)
        .into_iter()
        .filter(|c| c.x() == 0)
    {
        for level in 9..=24 {
            f.view
                .voxels
                .insert(TilePos::new(coord, level), f.materials.stone);
        }
    }
    f.view.revision += 1;
    f.view.full_rebuild = true;
    f.session.collision.refresh(&f.view, f.geometry);
}

#[test]
fn buried_sensor_reads_map_wide_positions_but_cannot_supply_attacks_or_party_knowledge() {
    let mut f = fixture();
    until_phase(&mut f, WormPhase::Travel);
    cover_between(&mut f);
    let old_party_point = f
        .session
        .encounter
        .runtime
        .first()
        .expect("party")
        .knowledge
        .map(|k| k.point);
    let target = Vec3::new(25.0, 3.2 + SKIN, 0.0);
    let human = f
        .session
        .actors
        .iter_mut()
        .find(|a| a.id == 0)
        .expect("human");
    human.feet = target;
    human.previous_feet = target;
    human.body.impulse_velocity = Vec3::ZERO;
    f.session
        .encounter
        .worms
        .get_mut(&7)
        .expect("controller")
        .next_sense = f.session.tick + 1;
    let before = f.worm().feet;
    for _ in 0..24 {
        let out = f.advance();
        f.apply(&out.burrows);
        if f.worm().feet.x > before.x + SKIN {
            break;
        }
    }
    let controller = f.session.encounter.worms.get(&7).expect("controller");
    let sensed = controller.burrow_target.expect("private position");
    assert_eq!(sensed.id, 0);
    assert!(sensed.point.distance(target) < SKIN);
    assert!(controller.target.is_none() && controller.seen.is_empty());
    assert!(controller.physically_buried(
        f.worm(),
        &f.view,
        f.geometry,
        f.tuning.encounters.worm_depth_levels
    ));
    assert!(f.worm().feet.x > before.x);
    assert!(f.worm().attack_state().is_none());
    assert_eq!(
        f.session
            .encounter
            .runtime
            .first()
            .expect("party")
            .knowledge
            .map(|k| k.point),
        old_party_point,
        "private earth sensing is never published as party sight"
    );
    f.session
        .encounter
        .worms
        .get_mut(&7)
        .expect("controller")
        .phase(WormPhase::Emerging);
    assert!(f
        .session
        .encounter
        .worms
        .get(&7)
        .expect("controller")
        .burrow_target
        .is_none());
    assert!(f
        .session
        .worm_release_aim(f.worm(), &f.view, f.geometry, &f.tuning)
        .is_none());
}

#[test]
fn exposed_worms_with_identical_sight_history_ignore_divergent_hidden_live_positions() {
    let mut left = fixture();
    let mut right = fixture();
    until_phase(&mut left, WormPhase::Exposed);
    until_phase(&mut right, WormPhase::Exposed);
    cover_between(&mut left);
    cover_between(&mut right);
    for (f, point) in [
        (&mut left, Vec3::new(10.0, 3.2 + SKIN, 3.0)),
        (&mut right, Vec3::new(16.0, 3.2 + SKIN, -3.0)),
    ] {
        let actor = f
            .session
            .actors
            .iter_mut()
            .find(|a| a.id == 0)
            .expect("human");
        actor.feet = point;
        actor.previous_feet = point;
        assert!(!f
            .session
            .collision
            .sight_clear(f.worm().eye(), point + Vec3::Y * 0.4));
    }
    for _ in 0..24 {
        let a = left.advance();
        let b = right.advance();
        left.apply(&a.burrows);
        right.apply(&b.burrows);
        assert_eq!(
            format!("{:?}", left.session.encounter.worms.get(&7)),
            format!("{:?}", right.session.encounter.worms.get(&7))
        );
        assert_eq!(left.worm().feet, right.worm().feet);
        assert!(left
            .session
            .encounter
            .worms
            .get(&7)
            .expect("controller")
            .burrow_target
            .is_none());
        assert!(left
            .session
            .encounter
            .worms
            .get(&7)
            .expect("controller")
            .target
            .is_none());
    }
}

#[test]
fn hostile_damage_from_any_actor_wakes_retracts_and_sustains_only_the_worm_pursuit() {
    let mut f = fixture();
    until_phase(&mut f, WormPhase::Exposed);
    cover_between(&mut f);
    let attacker_point = Vec3::new(25.0, 3.2 + SKIN, 0.0);
    let mut attacker = Actor::spawn(42, attacker_point, Vec3::NEG_X);
    attacker.team = 0;
    f.session.actors.push(attacker);
    let goblin_home = Vec3::new(-12.0, 3.2 + SKIN, 8.0);
    let mut goblin = Actor::spawn(9, goblin_home, Vec3::X);
    goblin.configure_species(Species::Goblin, &f.tuning.encounters);
    goblin.party = Some(3);
    f.session.actors.push(goblin);
    f.session
        .encounter
        .brains
        .insert(9, brain::Brain::new(9, goblin_home));
    f.session.encounter.runtime.push(PartyRuntime {
        snapshot: PartySnapshot {
            id: 3,
            phase: PartyPhase::Dormant,
            home: goblin_home,
            living: 1,
        },
        knowledge: None,
        last_sight: 0,
        last_cue_id: None,
        leash: 18.0,
        search: 4.0,
        battle_search: None,
    });
    let party = f.session.encounter.runtime.first_mut().expect("party");
    party.snapshot.phase = PartyPhase::Dormant;
    party.snapshot.home = Vec3::new(-40.0, 3.2, 0.0);
    party.leash = 1.0;
    party.search = 0.05;
    // Production HP admission calls this same hook with the frozen source ID.
    f.session.record_damage(42, 7, 1.0);
    let controller = f.session.encounter.worms.get(&7).expect("controller");
    assert!(controller.pursuing() && controller.target.is_none());
    assert_eq!(controller.phase, WormPhase::Diving);
    assert_eq!(controller.preferred_threat, Some(42));
    assert!(controller.burrow_target.is_none());
    assert!(f
        .session
        .worm_release_aim(f.worm(), &f.view, f.geometry, &f.tuning)
        .is_none());
    until_phase(&mut f, WormPhase::Travel);
    for _ in 0..24 {
        let out = f.advance();
        f.apply(&out.burrows);
    }
    let controller = f.session.encounter.worms.get(&7).expect("controller");
    let sensed = controller
        .burrow_target
        .expect("attacker sensed underground");
    assert_eq!(sensed.id, 42);
    assert!(sensed.point.distance(attacker_point) < SKIN);
    assert_eq!(
        f.session
            .encounter
            .runtime
            .first()
            .expect("party")
            .snapshot
            .phase,
        PartyPhase::Active
    );
    let independent = f
        .session
        .encounter
        .runtime
        .iter()
        .find(|party| party.snapshot.id == 3)
        .expect("independent party");
    assert_eq!(independent.snapshot.phase, PartyPhase::Dormant);
    assert!(independent.knowledge.is_none());
    assert!(
        f.worm().attack_state().is_none(),
        "distant covered damage causes pursuit, not a stationary shot"
    );
}

#[test]
fn dormant_worms_do_not_activate_from_the_underground_position_exception() {
    let mut f = fixture();
    f.session
        .encounter
        .runtime
        .first_mut()
        .expect("party")
        .snapshot
        .phase = PartyPhase::Dormant;
    cover_between(&mut f);
    for _ in 0..900 {
        let out = f.advance();
        f.apply(&out.burrows);
        let controller = f.session.encounter.worms.get(&7).expect("controller");
        assert!(!controller.pursuing());
        assert!(controller.burrow_target.is_none());
        assert_eq!(
            f.session
                .encounter
                .runtime
                .first()
                .expect("party")
                .snapshot
                .phase,
            PartyPhase::Dormant
        );
    }
}

#[test]
fn consecutive_boulders_require_retraction_and_real_underground_repositioning() {
    let mut f = fixture();
    let mut releases = 0;
    let mut after_first = false;
    let mut retracted = false;
    let mut traveled = 0.0;
    for _ in 0..1500 {
        let before = f.worm().feet;
        let previous_count = f.session.next_projectile;
        let out = f.advance();
        f.apply(&out.burrows);
        let state = f.worm().worm().expect("phase");
        let horizontal = (f.worm().feet - before).with_y(0.0).length();
        if state.phase != WormPhase::Travel {
            assert!(horizontal < SKIN, "no voluntary exposed/transition crawl");
        } else if horizontal > SKIN {
            assert!(f
                .session
                .encounter
                .worms
                .get(&7)
                .expect("controller")
                .physically_buried(
                    f.worm(),
                    &f.view,
                    f.geometry,
                    f.tuning.encounters.worm_depth_levels
                ));
            if after_first {
                traveled += horizontal;
            }
        }
        if after_first {
            retracted |= state.phase == WormPhase::Diving && !state.exposed;
        }
        if f.session.projectiles.iter().any(|shot| {
            shot.id >= previous_count
                && shot.owner == 7
                && shot.source_ability() == Some(CreatureAbility::WormBoulder)
        }) {
            releases += 1;
            if after_first {
                assert!(
                    retracted && traveled > 1.0,
                    "second exposure requires a physical dive and travel"
                );
                break;
            }
            after_first = true;
        }
    }
    assert_eq!(releases, 2);
}

#[test]
fn surface_phase_refuses_voluntary_translation_while_swept_knockback_remains_physical() {
    let mut f = fixture();
    let before = f.worm().feet;
    f.session
        .actors
        .iter_mut()
        .find(|actor| actor.id == 7)
        .expect("Worm")
        .body
        .impulse_velocity = Vec3::X * 0.5;
    let intents = [(
        7,
        brain::MotionIntent {
            input: ActorIntent::default(),
            direction: Vec3::NEG_Z,
            flight: false,
            lunge: false,
        },
    )]
    .into();
    let mut out = CommandsOut::default();
    f.session.move_worms(
        &intents,
        &f.view,
        f.geometry,
        f.materials,
        &f.tuning,
        &mut out,
    );
    assert!((f.worm().feet.z - before.z).abs() < SKIN);
    assert!(f.worm().feet.x > before.x + SKIN);
    assert!(f.worm().body.impulse_velocity.x > 0.0);
    assert!(f.worm().body_hex_prisms().next().expect("head").offset.y > 0.0);

    // An invalid phase label alone cannot grant either travel or earth sensing.
    f.session
        .encounter
        .worms
        .get_mut(&7)
        .expect("controller")
        .phase(WormPhase::Travel);
    let out = f.advance();
    f.apply(&out.burrows);
    let controller = f.session.encounter.worms.get(&7).expect("controller");
    assert_eq!(controller.phase, WormPhase::Diving);
    assert!(controller.burrow_target.is_none());
}
