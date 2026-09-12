use super::*;
use crate::collision::SKIN;
use crate::{ActorIntent, ArenaControl, ChargeState};
use hex_core::arena::{
    ArenaExpeditionSites, ArenaFountainVolume, ArenaMap, ArenaSelection, ArenaSolidSpan,
};
use hex_core::{HexCoord, SubstanceId, TilePos};

fn fixture() -> (
    ArenaSession,
    ArenaTerrainView,
    ArenaVoxelGeometry,
    PlayerObservation,
) {
    let geometry = ArenaVoxelGeometry::default();
    let mut player = Actor::spawn(0, Vec3::Y * SKIN, Vec3::X);
    player.configure_expedition_player();
    let mut shadow = Actor::spawn(1, Vec3::new(8.0, SKIN, 0.0), Vec3::NEG_X);
    shadow.configure_expedition(ExpeditionRole::MountainShadow, &Default::default());
    let mut view = ArenaTerrainView {
        selection: ArenaSelection {
            map: ArenaMap::ForestMassif,
            ..Default::default()
        },
        ..Default::default()
    };
    for coord in HexCoord::ORIGIN.within_radius(20) {
        view.voxels.insert(TilePos::new(coord, 0), SubstanceId(1));
    }
    let mut session = ArenaSession {
        actors: vec![player, shadow],
        progression: Some(Default::default()),
        ..Default::default()
    };
    session.register_forest_roster();
    session
        .player_knowledge
        .register_actor(session.actors.get(1).expect("shadow"), "mountain_shadow");
    let observation = PlayerObservation {
        active: true,
        origin: session.actors.first().expect("player").eye(),
        direction: (session.actors.get(1).expect("shadow").center()
            - session.actors.first().expect("player").eye())
        .normalize(),
        vertical_fov: 75.0_f32.to_radians(),
        aspect: 16.0 / 9.0,
        viewport_height: 900.0,
    };
    (session, view, geometry, observation)
}

fn sample(
    session: &mut ArenaSession,
    view: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    observation: PlayerObservation,
    count: u8,
) {
    for _ in 0..count {
        session.observe_player(view, geometry, observation, 0.1);
    }
}

#[test]
fn projected_size_threshold_is_eight_pixels_and_sampling_is_frame_rate_independent() {
    let (_, _, _, mut observation) = fixture();
    observation.origin = Vec3::ZERO;
    observation.direction = Vec3::X;
    observation.vertical_fov = std::f32::consts::FRAC_PI_2;
    observation.viewport_height = 1600.0;
    assert!(observation.contains(Vec3::X * 100.0, 1.01));
    assert!(!observation.contains(Vec3::X * 100.0, 0.99));
    for dt in [1.0 / 30.0, 1.0 / 60.0, 1.0 / 120.0] {
        let (mut session, view, geometry, observation) = fixture();
        let mut elapsed = 0.0;
        while elapsed + dt < 0.29 {
            session.observe_player(&view, geometry, observation, dt);
            elapsed += dt;
        }
        assert!(
            session.discovered_landmarks().is_empty(),
            "less than .3 seconds cannot discover a marker"
        );
        while elapsed < 0.4 {
            session.observe_player(&view, geometry, observation, dt);
            elapsed += dt;
        }
        assert_eq!(session.discovered_landmarks().len(), 1);
    }
}

#[test]
fn discovery_requires_three_distinct_samples_and_resets_dwell_after_occlusion() {
    let (mut session, mut view, geometry, observation) = fixture();
    sample(&mut session, &view, geometry, observation, 2);
    assert!(session.discovered_landmarks().is_empty());
    sample(&mut session, &view, geometry, observation, 1);
    let found = session.discovered_landmarks();
    assert_eq!(found.len(), 1);
    assert_eq!(found.first().expect("marker").id, "mountain_shadow");
    assert_eq!(
        session
            .combat_feedback(&ArenaTuning::default())
            .target
            .expect("target")
            .health_pips,
        3
    );

    session.player_knowledge = PlayerKnowledge::default();
    session
        .player_knowledge
        .register_actor(session.actors.get(1).expect("shadow"), "mountain_shadow");
    sample(&mut session, &view, geometry, observation, 2);
    let wall: Vec<_> = view
        .voxels
        .keys()
        .filter(|at| {
            let point = at.coord.to_world(0.0);
            point.x > 3.0 && point.x < 5.0
        })
        .copied()
        .collect();
    for at in &wall {
        for level in 1..=8 {
            view.voxels
                .insert(TilePos::new(at.coord, level), SubstanceId(1));
        }
    }
    view.revision += 1;
    sample(&mut session, &view, geometry, observation, 4);
    assert!(session.discovered_landmarks().is_empty());
    assert!(session
        .combat_feedback(&ArenaTuning::default())
        .target
        .is_none());
    for at in wall {
        for level in 1..=8 {
            view.voxels.remove(&TilePos::new(at.coord, level));
        }
    }
    view.revision += 1;
    sample(&mut session, &view, geometry, observation, 2);
    assert!(session.discovered_landmarks().is_empty());
    sample(&mut session, &view, geometry, observation, 1);
    assert_eq!(session.discovered_landmarks().len(), 1);
}

#[test]
fn discovery_rejects_small_offscreen_inactive_and_invalid_observations() {
    let (mut session, view, geometry, observation) = fixture();
    for invalid in [
        PlayerObservation {
            active: false,
            ..observation
        },
        PlayerObservation {
            direction: -observation.direction,
            ..observation
        },
        PlayerObservation {
            viewport_height: 1.0,
            ..observation
        },
        PlayerObservation {
            origin: Vec3::NAN,
            ..observation
        },
        PlayerObservation {
            vertical_fov: f32::NAN,
            ..observation
        },
    ] {
        sample(&mut session, &view, geometry, invalid, 4);
        assert!(session.discovered_landmarks().is_empty());
        assert!(session
            .combat_feedback(&ArenaTuning::default())
            .target
            .is_none());
    }
    session.observe_player(&view, geometry, observation, 10.0);
    assert!(
        session.discovered_landmarks().is_empty(),
        "one stalled frame is one observation"
    );
    session.accepted_battle.control = ArenaControl::Spectator;
    sample(&mut session, &view, geometry, observation, 4);
    assert!(session.discovered_landmarks().is_empty());
}

#[test]
fn hidden_movement_health_and_uncredited_death_do_not_update_memory() {
    let (mut session, view, geometry, observation) = fixture();
    sample(&mut session, &view, geometry, observation, 3);
    let initial = session.discovered_landmarks();
    let away = PlayerObservation {
        direction: -observation.direction,
        ..observation
    };
    let target = session.actors.get_mut(1).expect("target");
    target.feet += Vec3::Z * 5.0;
    target.hp = 0.0;
    sample(&mut session, &view, geometry, away, 4);
    assert_eq!(session.discovered_landmarks(), initial);
    assert!(session
        .combat_feedback(&ArenaTuning::default())
        .target
        .is_none());
    let toward_dead = PlayerObservation {
        direction: (session.actors.get(1).expect("dead target").center() - observation.origin)
            .normalize(),
        ..observation
    };
    sample(&mut session, &view, geometry, toward_dead, 4);
    assert_eq!(
        session.discovered_landmarks(),
        initial,
        "returning to an empty place does not witness an earlier hidden death"
    );
    session.record_player_hit(0, 1);
    session.reconcile_progression();
    let marked = session.discovered_landmarks();
    assert!(marked.first().expect("known").defeated);
    assert_eq!(
        marked.first().expect("known").position,
        initial.first().expect("known").position
    );
    session.player_knowledge.credited_defeat(99);
    assert_eq!(
        session.discovered_landmarks().len(),
        1,
        "credit never creates a hidden position"
    );
    session.actors.first_mut().expect("player").hp = 0.0;
    sample(&mut session, &view, geometry, observation, 4);
    assert_eq!(session.discovered_landmarks(), marked);
    session.reset(1, &view, geometry);
    assert!(session.discovered_landmarks().is_empty());
}

#[test]
fn a_continuously_observed_uncredited_defeat_updates_only_the_known_encounter() {
    let (mut session, view, geometry, observation) = fixture();
    sample(&mut session, &view, geometry, observation, 3);
    session.actors.get_mut(1).expect("target").hp = 0.0;
    sample(&mut session, &view, geometry, observation, 1);
    assert!(
        session
            .discovered_landmarks()
            .first()
            .expect("known marker")
            .defeated
    );
    assert!(session
        .combat_feedback(&ArenaTuning::default())
        .target
        .is_none());
    assert_eq!(session.progress().expect("progress").total_xp, 0);
}

#[test]
fn target_pips_boundaries_and_visible_recent_hit_fallback_are_authoritative() {
    let (mut session, view, geometry, observation) = fixture();
    for (ratio, expected) in [(1.0, 3), (2.0 / 3.0, 2), (1.0 / 3.0, 1), (0.01, 1)] {
        let target = session.actors.get_mut(1).expect("target");
        target.max_hp = 3.0;
        target.hp = ratio * 3.0;
        sample(&mut session, &view, geometry, observation, 1);
        assert_eq!(
            session
                .combat_feedback(&ArenaTuning::default())
                .target
                .expect("visible")
                .health_pips,
            expected
        );
    }
    session.record_damage(0, 1, 1.0);
    let off_reticle = PlayerObservation {
        direction: Vec3::new(1.0, 0.0, 0.3).normalize(),
        ..observation
    };
    sample(&mut session, &view, geometry, off_reticle, 1);
    let feedback = session.combat_feedback(&ArenaTuning::default());
    assert!(feedback.hit.is_some());
    assert_eq!(feedback.target.expect("visible recent hit").actor_id, 1);
    sample(&mut session, &view, geometry, off_reticle, 20);
    assert!(session
        .combat_feedback(&ArenaTuning::default())
        .target
        .is_none());
    session.record_damage(0, 1, 1.0);
    sample(
        &mut session,
        &view,
        geometry,
        PlayerObservation {
            direction: Vec3::NEG_X,
            ..observation
        },
        1,
    );
    assert!(
        session
            .combat_feedback(&ArenaTuning::default())
            .target
            .is_none(),
        "hit cannot disclose a hidden target"
    );
}

#[test]
fn hit_confirmation_excludes_push_zero_damage_enemy_friendly_and_self_hits() {
    let (mut session, view, geometry, observation) = fixture();
    sample(&mut session, &view, geometry, observation, 1);
    session.record_player_hit(0, 1); // Shield knockback credit, no HP damage.
    session.record_damage(0, 1, 0.0);
    session.record_damage(1, 0, 10.0);
    session.record_damage(0, 0, 10.0);
    assert!(session
        .combat_feedback(&ArenaTuning::default())
        .hit
        .is_none());
    session.actors.get_mut(1).expect("target").team = 0;
    session.record_damage(0, 1, 1.0);
    assert!(session
        .combat_feedback(&ArenaTuning::default())
        .hit
        .is_none());
    session.actors.get_mut(1).expect("target").team = 1;
    session.record_damage(0, 1, 1.0);
    let first = session
        .combat_feedback(&ArenaTuning::default())
        .hit
        .expect("damage hit");
    session.record_damage(0, 1, 1.0);
    assert!(
        session
            .combat_feedback(&ArenaTuning::default())
            .hit
            .expect("second hit")
            .sequence
            > first.sequence
    );
}

#[test]
fn spell_feedback_tracks_actual_charge_latch_pause_cooldown_and_reset() {
    let (mut session, view, geometry, observation) = fixture();
    let tuning = ArenaTuning::default();
    sample(&mut session, &view, geometry, observation, 1);
    assert!(session
        .combat_feedback(&tuning)
        .spells
        .iter()
        .all(|s| s.state == SpellAvailabilityState::Ready));
    session.actors.first_mut().expect("player").species = Species::Golem;
    assert_eq!(
        session
            .combat_feedback(&tuning)
            .spells
            .get(2)
            .expect("jump")
            .state,
        SpellAvailabilityState::Unavailable
    );
    session.actors.first_mut().expect("player").species = Species::Human;
    let player = session.actors.first_mut().expect("player");
    player.casting(
        ActorIntent {
            cast_pressed: true,
            cast_held: true,
            ..Default::default()
        },
        &tuning,
    );
    player.charge = Some(ChargeState {
        spell: Spell::Fireball,
        elapsed: tuning.charge_seconds * 0.5,
    });
    let states = session.combat_feedback(&tuning).spells;
    let fire = states.get(1).expect("fire");
    assert_eq!(fire.state, SpellAvailabilityState::Charging);
    assert!((fire.charge_fraction - 0.5).abs() < 0.0001);
    assert_eq!(
        states.first().expect("shield").state,
        SpellAvailabilityState::Unavailable
    );
    assert_eq!(
        states.get(2).expect("jump").state,
        SpellAvailabilityState::Ready
    );
    let player = session.actors.first_mut().expect("player");
    player.cancel_charge();
    assert_eq!(
        session
            .combat_feedback(&tuning)
            .spells
            .get(1)
            .expect("fire")
            .state,
        SpellAvailabilityState::Unavailable
    );
    let player = session.actors.first_mut().expect("player");
    player.casting(ActorIntent::default(), &tuning);
    *player.cooldowns.get_mut(1).expect("fire cooldown") = STEP * 0.011;
    assert_eq!(
        session
            .combat_feedback(&tuning)
            .spells
            .get(1)
            .expect("fire")
            .state,
        SpellAvailabilityState::CoolingDown
    );
    *session
        .actors
        .first_mut()
        .expect("player")
        .cooldowns
        .get_mut(1)
        .expect("fire cooldown") = STEP * 0.01;
    assert_eq!(
        session
            .combat_feedback(&tuning)
            .spells
            .get(1)
            .expect("fire")
            .state,
        SpellAvailabilityState::Ready
    );
    sample(
        &mut session,
        &view,
        geometry,
        PlayerObservation {
            active: false,
            ..observation
        },
        1,
    );
    assert!(session
        .combat_feedback(&tuning)
        .spells
        .iter()
        .all(|s| s.state == SpellAvailabilityState::Unavailable));
    session.reset(2, &view, geometry);
    assert!(session.combat_feedback(&tuning).hit.is_none());
    session.outcome = Some(crate::ArenaOutcome::Winner(0));
    sample(&mut session, &view, geometry, observation, 1);
    assert!(session
        .combat_feedback(&tuning)
        .spells
        .iter()
        .all(|s| s.state == SpellAvailabilityState::Unavailable));
}

#[test]
fn fountain_marker_is_observed_then_updates_only_when_consumption_is_observed() {
    let (mut session, mut view, geometry, observation) = fixture();
    let at = TilePos::new(HexCoord::from_axial(4, 0), 1);
    let mut sites = ArenaExpeditionSites::default();
    sites.fountains.insert(
        "forest_fountain_01".into(),
        ArenaFountainVolume { cells: [at].into() },
    );
    session.register_expedition_sites(&sites);
    view.expedition = Some(sites);
    view.liquids.push(ArenaSolidSpan {
        bottom: at,
        top_level: 1,
        substance: SubstanceId(2),
    });
    sample(&mut session, &view, geometry, observation, 3);
    assert!(session
        .discovered_landmarks()
        .iter()
        .any(|m| m.kind == LandmarkKind::Fountain && !m.consumed));
    let player = session.actors.first_mut().expect("player");
    player.hp = 10.0;
    player.feet = at
        .coord
        .to_world(geometry.top(at) - geometry.level_height + SKIN);
    session.advance_fountains(&view, geometry);
    sample(
        &mut session,
        &view,
        geometry,
        PlayerObservation {
            direction: Vec3::NEG_X,
            ..observation
        },
        1,
    );
    assert!(session
        .discovered_landmarks()
        .iter()
        .any(|m| m.kind == LandmarkKind::Fountain && !m.consumed));
    sample(&mut session, &view, geometry, observation, 1);
    assert!(session
        .discovered_landmarks()
        .iter()
        .any(|m| m.kind == LandmarkKind::Fountain && m.consumed));
}
