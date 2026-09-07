//! Headless acceptance of continuous authority and the shared input/spell path.

#![expect(
    clippy::expect_used,
    reason = "tests construct and require these exact two-actor fixtures and messages"
)]

use super::*;
use hex_core::{ElementId, HexCoord, SubstanceId, TerrainImpactRejection};

fn fixture() -> (
    ArenaSession,
    ArenaTerrainView,
    ArenaVoxelGeometry,
    ArenaMaterials,
    ArenaTuning,
) {
    let geometry = ArenaVoxelGeometry::default();
    let materials = ArenaMaterials {
        stone: SubstanceId(1),
        bedrock: SubstanceId(2),
        grass: SubstanceId(3),
        dirt: SubstanceId(4),
        fire: ElementId(1),
    };
    let view = ArenaTerrainView {
        revision: 1,
        voxels: HexCoord::ORIGIN
            .within_radius(12)
            .into_iter()
            .map(|coord| (TilePos::new(coord, 0), materials.stone))
            .collect(),
        spawns: [Vec3::new(-6.0, SKIN, 0.0), Vec3::new(8.0, SKIN, 0.0)],
    };
    let mut session = ArenaSession {
        bot_enabled: false,
        ..Default::default()
    };
    session.reset(0, &view, geometry);
    (session, view, geometry, materials, ArenaTuning::default())
}

fn human(session: &ArenaSession) -> &Actor {
    session
        .actors
        .iter()
        .find(|a| a.id == 0)
        .expect("fixture human")
}

fn bot(session: &ArenaSession) -> &Actor {
    session
        .actors
        .iter()
        .find(|a| a.id == 1)
        .expect("fixture bot")
}

fn set_actor(session: &mut ArenaSession, id: u8, feet: Vec3, aim: Vec3) {
    let actor = session
        .actors
        .iter_mut()
        .find(|a| a.id == id)
        .expect("fixture actor");
    actor.feet = feet;
    actor.previous_feet = feet;
    actor.aim = aim;
}

#[test]
fn one_click_is_consumed_once_and_cooldown_is_charged_on_release() {
    let (_, view, geometry, materials, tuning) = fixture();
    let mut app = App::new();
    app.insert_resource(view)
        .insert_resource(geometry)
        .insert_resource(materials)
        .insert_resource(tuning)
        .insert_resource(ArenaReset::default())
        .add_plugins(plugin);
    app.world_mut().resource_mut::<ArenaSession>().bot_enabled = false;
    app.world_mut().run_schedule(ArenaTick);
    app.world_mut().resource_mut::<ArenaInput>().human = ActorIntent {
        aim: Vec3::Y,
        cast: true,
        ..Default::default()
    };
    app.world_mut().run_schedule(ArenaTick);
    let session = app.world().resource::<ArenaSession>();
    assert_eq!(session.projectiles.len(), 1);
    assert!((human(session).cooldowns.get(1).copied().unwrap_or_default() - 1.25).abs() < 0.001);
    assert!(!app.world().resource::<ArenaInput>().human.cast);
    app.world_mut().run_schedule(ArenaTick);
    assert_eq!(app.world().resource::<ArenaSession>().projectiles.len(), 1);
    app.world_mut().resource_mut::<ArenaInput>().human.cast = true;
    app.world_mut().run_schedule(ArenaTick);
    assert_eq!(app.world().resource::<ArenaSession>().projectiles.len(), 1);
}

#[test]
fn fireball_damages_and_knocks_back_its_caster() {
    let (mut session, view, geometry, materials, tuning) = fixture();
    let mut published = Vec::new();
    for tick in 0..30 {
        let emitted = session.advance(
            ActorIntent {
                aim: Vec3::NEG_Y,
                cast: tick == 0,
                ..Default::default()
            },
            &view,
            geometry,
            materials,
            &tuning,
        );
        published.extend(emitted.impacts);
    }
    assert!(human(&session).hp < 70.0);
    assert!(human(&session).feet.y > 0.1);
    assert_eq!(published.len(), 1);
    assert!(published.iter().all(TerrainImpact::is_canonical));
}

#[test]
fn self_centered_area_excludes_caster_and_damages_through_cover() {
    let (mut session, mut view, geometry, materials, tuning) = fixture();
    set_actor(&mut session, 0, Vec3::new(0.0, SKIN, 0.0), Vec3::X);
    set_actor(&mut session, 1, Vec3::new(3.4, SKIN, 0.0), Vec3::NEG_X);
    for level in 1..=6 {
        view.voxels.insert(
            TilePos::new(HexCoord::from_axial(1, 0), level),
            materials.stone,
        );
    }
    view.revision += 1;
    let emitted = session.advance(
        ActorIntent {
            selected: Some(Spell::AreaBlast),
            cast: true,
            aim: Vec3::X,
            ..Default::default()
        },
        &view,
        geometry,
        materials,
        &tuning,
    );
    assert!((human(&session).hp - 100.0).abs() < 0.001);
    assert!(human(&session).impulse_velocity().length() < 0.001);
    assert!(bot(&session).hp < 100.0);
    assert!(bot(&session).impulse_velocity().x > 0.0);
    let impact = emitted.impacts.first().expect("sphere intersects floor");
    assert_eq!(
        impact.volume,
        geometry.sphere(&view, human(&session).center(), tuning.blast_radius())
    );
    assert!(
        impact
            .volume
            .contains(&TilePos::new(HexCoord::from_axial(2, 0), 0)),
        "sphere includes terrain behind the wall"
    );
}

#[test]
fn invalid_shield_fizzles_without_damage_and_keeps_cooldown() {
    let (mut session, view, geometry, materials, tuning) = fixture();
    let mut edits = Vec::new();
    for tick in 0..45 {
        let emitted = session.advance(
            ActorIntent {
                selected: Some(Spell::Shield),
                aim: Vec3::NEG_Y,
                cast: tick == 0,
                ..Default::default()
            },
            &view,
            geometry,
            materials,
            &tuning,
        );
        edits.extend(emitted.edits);
        assert!(emitted.impacts.is_empty());
    }
    assert!(
        edits.is_empty(),
        "a wall cannot materialize through its caster"
    );
    assert_eq!(session.shields_raised, 0);
    assert!(
        human(&session)
            .cooldowns
            .first()
            .copied()
            .unwrap_or_default()
            > 4.5
    );
    assert!((human(&session).hp - 100.0).abs() < 0.001);
    assert!(session.notice.contains("fizzled"));
}

#[test]
fn supported_shield_emits_one_complete_persistent_wall() {
    let (mut session, view, geometry, materials, tuning) = fixture();
    let aim = Vec3::new(1.0, -0.12, 0.0).normalize();
    let mut batches = Vec::new();
    for tick in 0..90 {
        let emitted = session.advance(
            ActorIntent {
                selected: Some(Spell::Shield),
                aim,
                cast: tick == 0,
                ..Default::default()
            },
            &view,
            geometry,
            materials,
            &tuning,
        );
        if !emitted.edits.is_empty() {
            batches.push(emitted.edits);
        }
        assert!(emitted.impacts.is_empty());
    }
    assert_eq!(batches.len(), 1);
    assert_eq!(batches.first().map(Vec::len), Some(25));
    assert_eq!(session.shields_raised, 1);
    for _ in 0..900 {
        assert!(session
            .advance(
                ActorIntent {
                    aim,
                    ..Default::default()
                },
                &view,
                geometry,
                materials,
                &tuning
            )
            .edits
            .is_empty());
    }
}

#[test]
fn shield_emergence_revalidates_an_actor_entering_the_footprint() {
    let (mut session, view, geometry, materials, tuning) = fixture();
    let aim = Vec3::new(1.0, -0.12, 0.0).normalize();
    for tick in 0..30 {
        let emitted = session.advance(
            ActorIntent {
                selected: Some(Spell::Shield),
                aim,
                cast: tick == 0,
                ..Default::default()
            },
            &view,
            geometry,
            materials,
            &tuning,
        );
        assert!(emitted.edits.is_empty());
        if !session.pending_walls.is_empty() {
            break;
        }
    }
    assert_eq!(session.pending_walls.len(), 1);
    // At this launch angle the contact column is axial (-1,0), with a clear
    // support. Moving the bot there before emergence must cancel all voxels.
    set_actor(
        &mut session,
        1,
        HexCoord::from_axial(-1, 0).to_world(SKIN),
        Vec3::NEG_X,
    );
    for _ in 0..40 {
        assert!(session
            .advance(
                ActorIntent {
                    aim,
                    ..Default::default()
                },
                &view,
                geometry,
                materials,
                &tuning
            )
            .edits
            .is_empty());
    }
    assert_eq!(session.shields_raised, 0);
}

#[test]
fn preview_and_released_projectile_report_the_same_impact() {
    let (mut session, view, geometry, materials, tuning) = fixture();
    let aim = Vec3::new(1.0, -0.2, 0.0).normalize();
    let feet = human(&session).feet;
    set_actor(&mut session, 0, feet, aim);
    let predicted = preview(&session, &view, &geometry, &tuning)
        .impact
        .expect("ground contact");
    let mut actual = None;
    for tick in 0..60 {
        session.advance(
            ActorIntent {
                aim,
                cast: tick == 0,
                ..Default::default()
            },
            &view,
            geometry,
            materials,
            &tuning,
        );
        if let Some(effect) = session.effects.iter().find(|e| e.kind == Spell::Fireball) {
            actual = Some(effect.center);
            break;
        }
    }
    assert!(
        actual.is_some_and(|point| point.distance(predicted) < 0.001),
        "released shot must reach predicted contact"
    );
}

#[test]
fn reset_restores_health_cooldowns_projectiles_and_bot_preference() {
    let (mut session, view, geometry, materials, tuning) = fixture();
    session.advance(
        ActorIntent {
            aim: Vec3::Y,
            cast: true,
            ..Default::default()
        },
        &view,
        geometry,
        materials,
        &tuning,
    );
    assert!(!session.projectiles.is_empty());
    if let Some(actor) = session.actors.first_mut() {
        actor.hp = 7.0;
    }
    session.reset(1, &view, geometry);
    assert_eq!(session.tick, 0);
    assert!(!session.bot_enabled);
    assert!(session.outcome.is_none() && session.projectiles.is_empty());
    assert!(session.pending_walls.is_empty() && session.pending_impacts.is_empty());
    assert!(session
        .actors
        .iter()
        .all(|a| (a.hp - 100.0).abs() < 0.001 && a.cooldowns.iter().all(|c| c.abs() < 0.001)));
}

#[test]
fn correlated_world_rejection_is_reported_and_not_silently_retried() {
    let (mut session, view, geometry, materials, tuning) = fixture();
    let emitted = session.advance(
        ActorIntent {
            selected: Some(Spell::AreaBlast),
            cast: true,
            ..Default::default()
        },
        &view,
        geometry,
        materials,
        &tuning,
    );
    let impact = emitted.impacts.first().expect("blast terrain message");
    let outcome = TerrainImpactOutcome {
        batch: impact.batch,
        result: TerrainImpactResult::Rejected(TerrainImpactRejection::TerrainUnavailable),
    };
    session.accept_outcome(&outcome);
    assert_eq!(session.terrain_outcomes, 1);
    assert!(session.pending_impacts.is_empty());
    assert!(session.notice.contains("TerrainUnavailable"));
}

#[test]
fn collision_refresh_uses_world_revision_and_new_walls_block_the_next_tick() {
    let (mut session, mut view, geometry, materials, tuning) = fixture();
    set_actor(&mut session, 0, Vec3::new(-1.2, SKIN, 0.0), Vec3::X);
    for level in 1..=5 {
        view.voxels
            .insert(TilePos::new(HexCoord::ORIGIN, level), materials.stone);
    }
    view.revision += 1;
    for _ in 0..120 {
        session.advance(
            ActorIntent {
                movement: Vec2::Y,
                aim: Vec3::X,
                run: true,
                ..Default::default()
            },
            &view,
            geometry,
            materials,
            &tuning,
        );
    }
    assert!(human(&session).feet.x < -1.1);
    assert_eq!(session.collision.revision, Some(view.revision));
}

#[test]
fn zero_knockback_is_a_valid_comparison_but_nan_and_bad_size_are_rejected() {
    let mut tuning = ArenaTuning {
        fireball_knockback: 0.0,
        blast_knockback: 0.0,
        ..Default::default()
    };
    assert!(tuning.validate().is_ok());
    tuning.projectile_gravity = f32::NAN;
    assert!(tuning.validate().is_err());
    tuning = ArenaTuning {
        shield_size: 3,
        ..Default::default()
    };
    assert!(tuning.validate().is_err());
    assert!(ron::from_str::<ArenaTuning>("(unknown: 2)").is_err());
}
