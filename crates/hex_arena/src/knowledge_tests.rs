//! Knowledge publication and accounting use actual casts, not presentation effects.

use super::*;
use hex_core::{ElementId, HexCoord, SubstanceId};

fn fixture() -> (
    ArenaSession,
    ArenaTerrainView,
    ArenaVoxelGeometry,
    ArenaMaterials,
    ArenaTuning,
) {
    let materials = ArenaMaterials {
        stone: SubstanceId(1),
        bedrock: SubstanceId(2),
        grass: SubstanceId(3),
        dirt: SubstanceId(4),
        fire: ElementId(1),
    };
    let geometry = ArenaVoxelGeometry::default();
    let world = ArenaTerrainView {
        revision: 1,
        voxels: HexCoord::ORIGIN
            .within_radius(12)
            .into_iter()
            .map(|coord| (TilePos::new(coord, 0), materials.stone))
            .collect(),
        spawns: [Vec3::new(-6.3, SKIN, 0.0), Vec3::new(8.0, SKIN, 0.0)],
    };
    let mut session = ArenaSession {
        bot_enabled: false,
        ..Default::default()
    };
    session.reset(0, &world, geometry);
    (session, world, geometry, materials, ArenaTuning::default())
}

#[test]
fn silent_charge_publishes_nothing_and_release_and_impact_are_discrete_coarse_cues() {
    let (mut session, world, geometry, materials, tuning) = fixture();
    for tick in 0..90 {
        session.advance(
            ActorIntent {
                aim: Vec3::NEG_Y,
                cast_pressed: tick == 0,
                cast_held: true,
                ..Default::default()
            },
            &world,
            geometry,
            materials,
            &tuning,
        );
    }
    assert!(session.combat_cues.is_empty());
    session.advance(
        ActorIntent {
            aim: Vec3::NEG_Y,
            cast_released: true,
            ..Default::default()
        },
        &world,
        geometry,
        materials,
        &tuning,
    );
    assert_eq!(session.combat_cues.len(), 1);
    let release = *session.combat_cues.first().expect("release cue");
    assert_eq!(release.kind, CombatCueKind::Release);
    assert_eq!(release.owner, 0);
    assert_eq!(release.position, Vec3::new(-6.0, 0.0, 0.0));
    for _ in 0..12 {
        session.advance(ActorIntent::default(), &world, geometry, materials, &tuning);
    }
    assert_eq!(session.combat_cues.len(), 2);
    let impact = session.combat_cues.last().expect("impact cue");
    assert_eq!(impact.kind, CombatCueKind::Impact);
    assert!(impact.id > release.id && impact.tick > release.tick);
    assert_eq!(
        session.combat_cues.first().expect("stored origin").position,
        release.position
    );
    let stats = session.round_summary();
    let human = stats.actors.first().expect("human stats");
    assert_eq!(human.casts.get(1), Some(&1));
    assert_eq!(human.fireballs_resolved, 1);
    assert_eq!(human.useful_fireballs, 0, "self-harm is not a useful hit");
    assert!(human.self_damage > 0.0 && human.damage_dealt.abs() < SKIN);
    for _ in 0..130 {
        session.advance(ActorIntent::default(), &world, geometry, materials, &tuning);
    }
    assert!(session.combat_cues.is_empty());
    session.reset(1, &world, geometry);
    assert!(session
        .round_summary()
        .actors
        .iter()
        .all(|stats| stats.casts == [0; 3] && stats.damage_received.abs() < SKIN));
    assert!(session
        .round_summary()
        .actors
        .iter()
        .all(|stats| stats.fireballs_resolved == 0 && stats.useful_fireballs == 0));
    assert_eq!(session.next_cue, 0);
}

#[test]
fn useful_fireball_counts_a_real_opponent_impact_even_when_remaining_hp_is_low() {
    let (mut session, world, geometry, materials, tuning) = fixture();
    session.actors.first_mut().expect("caster").feet = Vec3::new(0.0, SKIN, 0.0);
    let victim = session.actors.get_mut(1).expect("victim");
    victim.feet = Vec3::new(4.0, SKIN, 0.0);
    victim.hp = 2.0;
    session.advance(
        ActorIntent {
            aim: Vec3::X,
            cast_pressed: true,
            cast_released: true,
            ..Default::default()
        },
        &world,
        geometry,
        materials,
        &tuning,
    );
    for _ in 0..60 {
        if session.outcome.is_some() {
            break;
        }
        session.advance(ActorIntent::default(), &world, geometry, materials, &tuning);
    }
    let summary = session.round_summary();
    assert_eq!(summary.winner, Some(0));
    let caster = summary.actors.first().expect("caster stats");
    assert_eq!(caster.fireballs_resolved, 1);
    assert_eq!(caster.useful_fireballs, 1);
    assert!((caster.damage_dealt - 2.0).abs() < SKIN);
    assert!(caster.self_damage.abs() < SKIN);
}

#[test]
fn actual_splash_damage_is_capped_and_accounted_without_changing_caster_immunity() {
    let (mut session, world, geometry, materials, tuning) = fixture();
    session.actors.first_mut().expect("caster").feet = Vec3::ZERO;
    let victim = session.actors.get_mut(1).expect("victim");
    victim.feet = Vec3::X;
    victim.hp = 2.0;
    session.advance(
        ActorIntent {
            selected: Some(Spell::AreaBlast),
            cast_pressed: true,
            cast_released: true,
            ..Default::default()
        },
        &world,
        geometry,
        materials,
        &tuning,
    );
    let summary = session.round_summary();
    assert_eq!(summary.winner, Some(0));
    assert!(summary.complete);
    assert!((summary.actors.first().expect("caster").damage_dealt - 2.0).abs() < SKIN);
    assert!(
        summary
            .actors
            .first()
            .expect("caster")
            .damage_received
            .abs()
            < SKIN
    );
    assert_eq!(
        summary.actors.first().expect("caster").first_damage_tick,
        Some(1)
    );
    assert!((summary.actors.get(1).expect("victim").damage_received - 2.0).abs() < SKIN);
}

#[test]
fn forecast_uses_explicit_moving_observation_and_stops_at_nearest_terrain() {
    use spells::{forecast_spell, ForecastBody};
    let (session, mut world, geometry, _, tuning) = fixture();
    let mut caster = session.actors.first().expect("caster").clone();
    caster.feet = Vec3::ZERO;
    caster.aim = Vec3::new(1.0, 0.02, 0.0).normalize();
    let observed = [ForecastBody {
        id: 1,
        feet: Vec3::new(8.0, 0.0, -0.75),
        velocity: Vec3::Z * 3.0,
        predict_seconds: 0.5,
    }];
    let clear = forecast_spell(
        &caster,
        &observed,
        &session.collision,
        &world,
        geometry,
        &tuning,
        32.0,
    );
    let hit = clear.impact.expect("observed moving body intersects shot");
    assert_eq!(hit.actor, Some(1));
    assert!(hit.time < 0.3 && hit.point.x > 7.0);
    for level in 1..=5 {
        world.voxels.insert(
            TilePos::new(HexCoord::from_axial(2, 0), level),
            SubstanceId(1),
        );
    }
    world.revision += 1;
    let mut collision = CollisionWorld::default();
    collision.refresh(&world, geometry);
    let blocked = forecast_spell(
        &caster, &observed, &collision, &world, geometry, &tuning, 32.0,
    );
    let hit = blocked.impact.expect("first wall");
    assert!(hit.actor.is_none() && hit.point.x < 4.0 && hit.time < 0.13);
}

#[test]
fn reset_preserves_the_selected_test_brain_and_resets_its_policy_state() {
    let (mut session, world, geometry, materials, tuning) = fixture();
    session.use_baseline_bot(true);
    for _ in 0..90 {
        session.advance(ActorIntent::default(), &world, geometry, materials, &tuning);
    }
    assert_eq!(
        session.bot_debug().decisions,
        0,
        "the new brain must not run behind baseline"
    );
    session.reset(1, &world, geometry);
    assert!(session.baseline_bot.is_some());
    assert_eq!(session.tick, 0);
    assert!(session
        .round_summary()
        .actors
        .iter()
        .all(|stats| stats.casts == [0; 3]));
    session.use_baseline_bot(false);
    session.reset(2, &world, geometry);
    assert!(session.baseline_bot.is_none());
}

#[test]
fn authored_bot_policy_matches_validated_defaults_and_rejects_invalid_settings() {
    let tuning: ArenaTuning = ron::from_str(include_str!("../../../assets/config/arena.ron"))
        .expect("authored arena settings");
    tuning.validate().expect("valid authored policy");
    assert!((tuning.charge_seconds - 0.75).abs() < SKIN);
    assert!((tuning.bot.memory_seconds - 6.0).abs() < SKIN);
    assert!((tuning.bot.cue_radius - 12.0).abs() < SKIN);
    assert_eq!(
        ron::to_string(&tuning).expect("authored settings"),
        ron::to_string(&ArenaTuning::default()).expect("runtime defaults")
    );
    assert!(ron::from_str::<ArenaTuning>("(bot: (omniscient: true))").is_err());
    for invalid in [f32::NAN, f32::INFINITY, -1.0, 0.0] {
        let mut malformed = tuning.clone();
        malformed.bot.memory_seconds = invalid;
        assert!(malformed.validate().is_err());
    }
    let mut malformed = tuning;
    malformed.bot.flank_replan_seconds = 0.01;
    assert!(
        malformed.validate().is_err(),
        "authored policy cannot remove the route CPU bound"
    );
}
