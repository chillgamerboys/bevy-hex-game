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

// Retain the previous eager-query target selection as a differential oracle.
fn eager_reticle_target(session: &ArenaSession, observation: PlayerObservation) -> Option<ActorId> {
    let team = session.actors.first()?.team;
    let direction = observation.direction.normalize();
    let delta = direction * 1600.0;
    let mut nearest = session
        .collision
        .attack_sweep(observation.origin, delta, 0.0)
        .map_or(1.0, |(hit, _)| hit.fraction);
    let mut aimed = None;
    for actor in session.actors.iter().filter(|a| a.team != team) {
        let right = direction.cross(Vec3::Y).normalize_or(Vec3::X);
        let up = right.cross(direction).normalize();
        let rotation = actor.body_rotation().inverse();
        let diameter = (rotation * right)
            .abs()
            .dot(actor.dimensions)
            .max((rotation * up).abs().dot(actor.dimensions));
        let sighted = [actor.center(), actor.eye()].into_iter().any(|point| {
            observation.contains(point, diameter)
                && session.collision.sight_clear(observation.origin, point)
        });
        if !sighted || actor.hp <= 0.0 {
            continue;
        }
        if let Some(hit) = crate::shapes::sweep_actor(observation.origin, delta, actor, false, 0.0)
        {
            if hit.fraction < nearest {
                nearest = hit.fraction;
                aimed = Some(actor.id);
            }
        }
    }
    aimed
}

#[test]
fn lazy_reticle_occlusion_matches_eager_selection_for_protected_and_live_barriers() {
    use crate::BarrierSnapshot;
    use hex_core::arena::ArenaStaticSpan;
    for blocker in 0..6 {
        let (mut session, mut view, geometry, observation) = fixture();
        // Equal-distance ties retain roster order; another shape is farther away.
        let mut tied = session.actors.get(1).expect("shadow").clone();
        tied.id = 2;
        session.actors.push(tied);
        let mut farther = Actor::spawn(3, Vec3::new(12.0, SKIN, 2.0), Vec3::NEG_X);
        farther.configure_expedition(ExpeditionRole::Goblin, &Default::default());
        session.actors.push(farther);
        if matches!(blocker, 1 | 2) {
            view.static_spans.push(ArenaStaticSpan {
                bottom: TilePos::new(HexCoord::from_world(Vec3::X * 4.0), 0),
                top_level: 8,
                blocks_movement: true,
                blocks_projectiles: true,
                blocks_sight: blocker == 2,
            });
            view.revision += 1;
        }
        if blocker >= 3 {
            let barrier = BarrierSnapshot {
                id: 7,
                owner: 1,
                center: Vec3::new(if blocker == 4 { 12.0 } else { 4.0 }, 0.8, 0.0),
                normal: Vec3::X,
                width: 4.0,
                height: 1.6,
                hp: if blocker == 5 { 0.0 } else { 60.0 },
                max_hp: 60.0,
                remaining: 4.0,
                lifetime: 4.0,
            };
            session.collision.sync_barriers(&[barrier]);
        }
        session.collision.refresh(&view, geometry);
        for yaw_step in -8_i16..=8 {
            for pitch_step in -2_i16..=2 {
                let direction = Vec3::new(
                    1.0,
                    f32::from(pitch_step) * 0.05,
                    f32::from(yaw_step) * 0.04,
                )
                .normalize();
                let observation = PlayerObservation {
                    direction,
                    ..observation
                };
                let expected = eager_reticle_target(&session, observation);
                sample(&mut session, &view, geometry, observation, 1);
                assert_eq!(
                    session
                        .player_knowledge
                        .target
                        .map(|target| target.actor_id),
                    expected,
                    "blocker={blocker}, yaw={yaw_step}, pitch={pitch_step}"
                );
            }
        }
        sample(&mut session, &view, geometry, observation, 1);
        let expected = if matches!(blocker, 1..=3) {
            None
        } else {
            Some(1)
        };
        assert_eq!(
            session
                .player_knowledge
                .target
                .map(|target| target.actor_id),
            expected,
            "blocker={blocker}"
        );
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
        while elapsed + dt < 0.49 {
            session.observe_player(&view, geometry, observation, dt);
            elapsed += dt;
        }
        assert!(
            session.discovered_landmarks().is_empty(),
            "less than .5 seconds cannot discover a marker"
        );
        while elapsed < 0.6 {
            session.observe_player(&view, geometry, observation, dt);
            elapsed += dt;
        }
        assert_eq!(session.discovered_landmarks().len(), 1);
    }
}

#[test]
fn discovery_requires_five_distinct_samples_and_resets_dwell_after_occlusion() {
    let (mut session, mut view, geometry, observation) = fixture();
    sample(&mut session, &view, geometry, observation, 4);
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
    sample(&mut session, &view, geometry, observation, 4);
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
    sample(&mut session, &view, geometry, observation, 4);
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
    sample(&mut session, &view, geometry, observation, 5);
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
    sample(&mut session, &view, geometry, observation, 5);
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
fn personally_used_fountain_updates_known_state_without_another_sighting() {
    let (mut session, mut view, geometry, observation) = fixture();
    let at = TilePos::new(HexCoord::from_axial(4, 0), 1);
    let unused = TilePos::new(HexCoord::from_axial(8, 0), 1);
    let mut sites = ArenaExpeditionSites::default();
    for (name, at) in [("forest_fountain_01", at), ("forest_fountain_02", unused)] {
        sites
            .fountains
            .insert(name.into(), ArenaFountainVolume { cells: [at].into() });
        view.liquids.push(ArenaSolidSpan {
            bottom: at,
            top_level: 1,
            substance: SubstanceId(2),
        });
    }
    session.register_expedition_sites(&sites);
    view.expedition = Some(sites);
    sample(&mut session, &view, geometry, observation, 5);
    let known: BTreeMap<_, _> = session
        .discovered_landmarks()
        .into_iter()
        .filter(|m| m.kind == LandmarkKind::Fountain)
        .map(|m| (m.id.clone(), m))
        .collect();
    assert_eq!(known.len(), 2);
    assert!(known.values().all(|m| !m.consumed));
    let player = session.actors.first_mut().expect("player");
    player.hp = 10.0;
    player.feet = at
        .coord
        .to_world(geometry.top(at) - geometry.level_height + SKIN);
    session.advance_fountains(&view, geometry);
    assert!(session
        .discovered_landmarks()
        .iter()
        .any(|m| m.id == "forest_fountain_01" && m.consumed));
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
    for marker in session
        .discovered_landmarks()
        .into_iter()
        .filter(|m| m.kind == LandmarkKind::Fountain)
    {
        let previous = known.get(&marker.id).expect("previously observed pool");
        assert_eq!(marker.position, previous.position);
        assert_eq!(marker.consumed, marker.id == "forest_fountain_01");
    }
}

#[test]
fn using_an_undiscovered_fountain_does_not_bypass_visual_acquisition() {
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
    let player = session.actors.first_mut().expect("player");
    player.hp = 10.0;
    player.feet = at
        .coord
        .to_world(geometry.top(at) - geometry.level_height + SKIN);
    session.advance_fountains(&view, geometry);
    assert!((session.actors.first().expect("healed player").hp - 50.0).abs() < SKIN);
    assert!(session.discovered_landmarks().is_empty());
    sample(&mut session, &view, geometry, observation, 4);
    assert!(!session
        .discovered_landmarks()
        .iter()
        .any(|m| m.kind == LandmarkKind::Fountain));
    sample(&mut session, &view, geometry, observation, 1);
    assert!(session
        .discovered_landmarks()
        .iter()
        .any(|m| m.id == "forest_fountain_01" && m.consumed));
}

#[test]
fn health_announcements_are_event_driven_and_same_band_hits_do_not_extend_them() {
    let (mut session, view, geometry, observation) = fixture();
    sample(&mut session, &view, geometry, observation, 1);
    assert!(session
        .combat_feedback(&ArenaTuning::default())
        .health_cues
        .is_empty());
    session.record_damage(0, 1, 1.0);
    sample(&mut session, &view, geometry, observation, 1);
    let cue = session.combat_feedback(&ArenaTuning::default()).health_cues;
    assert_eq!(cue.len(), 1);
    assert_eq!(cue.first().expect("first hit cue").health_pips, 3);
    sample(&mut session, &view, geometry, observation, 8);
    session.record_damage(0, 1, 1.0);
    sample(&mut session, &view, geometry, observation, 3);
    assert!(session
        .combat_feedback(&ArenaTuning::default())
        .health_cues
        .is_empty());
    let enemy = session.actors.get_mut(1).expect("enemy");
    enemy.hp = enemy.max_hp * 0.5;
    sample(&mut session, &view, geometry, observation, 1);
    assert_eq!(
        session
            .combat_feedback(&ArenaTuning::default())
            .health_cues
            .first()
            .expect("changed")
            .health_pips,
        2
    );
    let enemy = session.actors.get_mut(1).expect("enemy");
    enemy.hp = enemy.max_hp;
    sample(&mut session, &view, geometry, observation, 1);
    assert_eq!(
        session
            .combat_feedback(&ArenaTuning::default())
            .health_cues
            .first()
            .expect("healed")
            .health_pips,
        3
    );
}

#[test]
fn hidden_health_changes_announce_once_on_return_and_death_clears() {
    let (mut session, view, geometry, observation) = fixture();
    sample(&mut session, &view, geometry, observation, 1);
    session.record_damage(0, 1, 1.0);
    sample(&mut session, &view, geometry, observation, 1);
    let hidden = PlayerObservation {
        direction: Vec3::NEG_X,
        ..observation
    };
    sample(&mut session, &view, geometry, hidden, 12);
    assert!(session
        .combat_feedback(&ArenaTuning::default())
        .health_cues
        .is_empty());
    sample(&mut session, &view, geometry, observation, 1);
    assert!(
        session
            .combat_feedback(&ArenaTuning::default())
            .health_cues
            .is_empty(),
        "same band never replays"
    );
    sample(&mut session, &view, geometry, hidden, 1);
    session.actors.get_mut(1).expect("enemy").hp = 1.0;
    sample(&mut session, &view, geometry, hidden, 1);
    assert!(session
        .combat_feedback(&ArenaTuning::default())
        .health_cues
        .is_empty());
    sample(&mut session, &view, geometry, observation, 1);
    assert_eq!(
        session
            .combat_feedback(&ArenaTuning::default())
            .health_cues
            .first()
            .expect("new band")
            .health_pips,
        1
    );
    session.actors.get_mut(1).expect("enemy").hp = 0.0;
    sample(&mut session, &view, geometry, observation, 1);
    assert!(session
        .combat_feedback(&ArenaTuning::default())
        .health_cues
        .is_empty());
}

#[test]
fn landmark_distance_is_euclidean_and_projected_size_is_resolution_independent() {
    let (_, _, _, mut observation) = fixture();
    observation.origin = Vec3::ZERO;
    observation.direction = Vec3::X;
    for (kind, range) in [
        (LandmarkKind::Dragon, 120.0),
        (LandmarkKind::Troll, 60.0),
        (LandmarkKind::Shadow, 60.0),
        (LandmarkKind::Golem, 60.0),
        (LandmarkKind::Fountain, 35.0),
    ] {
        assert!(observation.landmark_contains(Vec3::X * range, 8.0, kind));
        assert!(!observation.landmark_contains(Vec3::X * (range + 0.01), 8.0, kind));
        assert!(!observation.landmark_contains(Vec3::new(range, 1.0, 0.0), 8.0, kind));
    }
    for height in [720.0, 900.0, 1080.0, 2160.0] {
        observation.viewport_height = height;
        assert!(!observation.landmark_contains(Vec3::X * 34.0, 0.4, LandmarkKind::Fountain));
        assert!(observation.landmark_contains(Vec3::X * 34.0, 1.0, LandmarkKind::Fountain));
    }
}

#[test]
fn first_hit_uses_current_frame_visibility_before_next_discovery_sample() {
    let (mut session, view, geometry, observation) = fixture();
    session.observe_player(&view, geometry, observation, 0.016);
    assert!(session.player_knowledge.visible_last_sample.is_empty());
    session.record_damage(0, 1, 1.0);
    session.observe_player(&view, geometry, observation, 0.016);
    assert_eq!(
        session
            .combat_feedback(&ArenaTuning::default())
            .health_cues
            .len(),
        1
    );
    assert!(session.discovered_landmarks().is_empty());
    session.player_knowledge.health_announcements.clear();
    session.record_damage(0, 1, 1.0);
    session.observe_player(
        &view,
        geometry,
        PlayerObservation {
            direction: Vec3::NEG_X,
            ..observation
        },
        0.016,
    );
    sample(&mut session, &view, geometry, observation, 1);
    assert!(
        session
            .combat_feedback(&ArenaTuning::default())
            .health_cues
            .is_empty(),
        "unseen hit must not replay on return"
    );
}

#[test]
fn active_health_dots_hide_and_clear_between_discovery_samples() {
    let (mut session, view, geometry, observation) = fixture();
    sample(&mut session, &view, geometry, observation, 1);
    session.record_damage(0, 1, 1.0);
    session.observe_player(&view, geometry, observation, 0.016);
    assert_eq!(
        session
            .combat_feedback(&ArenaTuning::default())
            .health_cues
            .len(),
        1
    );
    session.observe_player(
        &view,
        geometry,
        PlayerObservation {
            direction: Vec3::NEG_X,
            ..observation
        },
        0.016,
    );
    assert!(session
        .combat_feedback(&ArenaTuning::default())
        .health_cues
        .is_empty());
    session.observe_player(&view, geometry, observation, 0.016);
    assert_eq!(
        session
            .combat_feedback(&ArenaTuning::default())
            .health_cues
            .len(),
        1
    );
    session.actors.get_mut(1).expect("enemy").hp = 0.0;
    session.observe_player(&view, geometry, observation, 0.016);
    assert!(session
        .combat_feedback(&ArenaTuning::default())
        .health_cues
        .is_empty());
}

#[test]
fn fountain_discovery_uses_the_central_exposed_top_not_a_visible_edge() {
    let (mut session, mut view, geometry, observation) = fixture();
    let center = HexCoord::from_axial(4, 0);
    let mut sites = ArenaExpeditionSites::default();
    let cells = [
        center,
        HexCoord::from_axial(3, 2),
        HexCoord::from_axial(5, -2),
    ]
    .into_iter()
    .flat_map(|coord| [TilePos::new(coord, 1), TilePos::new(coord, 2)])
    .collect();
    sites
        .fountains
        .insert("forest_fountain_01".into(), ArenaFountainVolume { cells });
    session.register_expedition_sites(&sites);
    view.expedition = Some(sites);
    let observation = PlayerObservation {
        direction: (center.to_world(geometry.top(TilePos::new(center, 2)) + 0.01)
            - observation.origin)
            .normalize(),
        ..observation
    };
    sample(&mut session, &view, geometry, observation, 5);
    let pool = session
        .discovered_landmarks()
        .into_iter()
        .find(|landmark| landmark.kind == LandmarkKind::Fountain)
        .expect("pool");
    assert!((pool.position.y - geometry.top(TilePos::new(center, 2)) - 0.01).abs() < 0.0001);
    session.player_knowledge.landmarks.clear();
    session.player_knowledge.dwell.clear();
    view.static_spans.push(hex_core::arena::ArenaStaticSpan {
        bottom: TilePos::new(HexCoord::from_axial(2, 0), 1),
        top_level: 8,
        blocks_sight: true,
        blocks_movement: true,
        blocks_projectiles: true,
    });
    view.revision += 1;
    view.full_rebuild = true;
    session.collision.refresh(&view, geometry);
    let edge = HexCoord::from_axial(3, 2).to_world(pool.position.y);
    assert!(
        session.collision.sight_clear(observation.origin, edge),
        "test requires a visible edge"
    );
    assert!(
        !session
            .collision
            .sight_clear(observation.origin, pool.position),
        "central patch must be blocked"
    );
    sample(&mut session, &view, geometry, observation, 5);
    assert!(!session
        .discovered_landmarks()
        .iter()
        .any(|landmark| landmark.kind == LandmarkKind::Fountain));
}
