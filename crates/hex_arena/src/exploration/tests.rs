use super::*;
use crate::{ArenaBattleSetup, ArenaTuning, FireballMode, Spell, UpgradeStat};
use hex_core::arena::{ArenaMap, ArenaMaterials, ArenaResidency, ArenaSelection, ArenaSolidSpan};
use hex_core::{ElementId, HexCoord, SubstanceId, TilePos};
use std::collections::BTreeSet;

fn fixture() -> (
    ArenaSession,
    ArenaTerrainView,
    ArenaVoxelGeometry,
    ArenaMaterials,
) {
    let spawn = HexCoord::from_axial(8, 8).to_world(SKIN);
    let geometry = ArenaVoxelGeometry {
        radius: 1000,
        ..Default::default()
    };
    let chunks: BTreeSet<_> = (-2..=2)
        .flat_map(|x| (-2..=2).map(move |y| (x, y)))
        .collect();
    let view = ArenaTerrainView {
        selection: ArenaSelection {
            map: ArenaMap::NorthernArchipelago,
            ..Default::default()
        },
        columns: HexCoord::from_axial(8, 8)
            .within_radius(4)
            .into_iter()
            .map(|coord| {
                (
                    coord,
                    vec![ArenaSolidSpan {
                        bottom: TilePos::new(coord, 0),
                        top_level: 0,
                        substance: SubstanceId(1),
                    }],
                )
            })
            .collect(),
        spawns: [spawn, spawn + Vec3::NEG_Z],
        residency: Some(ArenaResidency {
            catalogue: chunks.clone(),
            ready: chunks,
        }),
        ..Default::default()
    };
    let materials = ArenaMaterials {
        stone: SubstanceId(1),
        grass: SubstanceId(2),
        dirt: SubstanceId(3),
        bedrock: SubstanceId(4),
        fire: ElementId(1),
    };
    let mut session = ArenaSession::default();
    session.reset_with_setup(1, &view, geometry, &ArenaBattleSetup::default());
    session.advance(
        ActorIntent::default(),
        &view,
        geometry,
        materials,
        &ArenaTuning::default(),
    );
    (session, view, geometry, materials)
}

fn fly(session: &mut ArenaSession) {
    let actor = session.actors.first_mut().expect("explorer");
    actor.feet.y = 20.0;
    actor.previous_feet = actor.feet;
    prepare(
        actor,
        ActorIntent {
            flight_toggle: true,
            ..Default::default()
        },
    );
}

#[test]
fn empty_exploration_is_playable_with_starting_profile_and_no_progression() {
    let (mut session, view, geometry, materials) = fixture();
    assert!(session.is_exploration());
    assert_eq!(session.actors.len(), 1);
    assert!(session.progress().is_none());
    assert!(session.expedition_progress().is_none());
    assert!(session.parties().is_empty());
    assert!(!session.can_upgrade(UpgradeStat::WalkingSpeed));
    let player = session.actors.first().expect("player");
    assert!((player.body_dimensions().y - 1.2).abs() < 0.0001);
    assert!((player.eye().y - player.feet.y - 1.02).abs() < 0.0001);
    assert!((player.body_dimensions().x - 0.5).abs() < 0.0001);
    assert!((session.player_walking_speed() - 4.725).abs() < 0.0001);
    let base = ArenaTuning {
        projectile_speed: 70.0,
        fireball_damage: 99.0,
        ..Default::default()
    };
    let tuning = session.player_tuning(&base);
    assert!((tuning.projectile_speed - 45.0).abs() < 0.0001);
    assert!((tuning.projectile_gravity - 12.0).abs() < 0.0001);
    assert!((tuning.fireball_damage - 15.0).abs() < 0.0001);
    assert!((tuning.fireball_cooldown - 0.5).abs() < 0.0001);
    assert_eq!(session.player_fireball_mode(), FireballMode::ContactOnly);
    for _ in 0..120 {
        session.advance(ActorIntent::default(), &view, geometry, materials, &base);
    }
    assert!(!session.is_finished());
    assert!(session.outcome.is_none());
}

#[test]
fn powered_flight_uses_camera_axes_and_caps_combined_diagonal_speed() {
    for (fast, expected) in [(false, SPEED), (true, FAST_SPEED)] {
        let request = input_velocity(
            ActorIntent {
                movement: Vec2::ONE,
                flight_vertical: 1.0,
                flight_fast: fast,
                glider_look: (Vec3::NEG_Z + Vec3::Y).normalize(),
                ..Default::default()
            },
            Vec3::NEG_Z,
        );
        assert!((request.length() - expected).abs() < 0.001);
        assert!(request.y > 0.0 && request.x > 0.0 && request.z < 0.0);
    }
    let down = input_velocity(
        ActorIntent {
            flight_vertical: -1.0,
            ..Default::default()
        },
        Vec3::NEG_Z,
    );
    assert!(down.distance(Vec3::NEG_Y * SPEED) < 0.001);
}

#[test]
fn stationary_flight_has_no_gravity_and_casting_stays_available() {
    let (mut session, view, geometry, materials) = fixture();
    fly(&mut session);
    let start = session.actors.first().expect("player").feet;
    session.advance(
        ActorIntent {
            cast_pressed: true,
            cast_held: true,
            ..Default::default()
        },
        &view,
        geometry,
        materials,
        &ArenaTuning::default(),
    );
    session.advance(
        ActorIntent {
            cast_released: true,
            ..Default::default()
        },
        &view,
        geometry,
        materials,
        &ArenaTuning::default(),
    );
    let actor = session.actors.first().expect("player");
    assert!(actor.feet.distance(start) < 0.0001);
    assert!(actor.free_flight().expect("flight").active);
    assert_eq!(session.projectiles.len(), 1);
    assert_eq!(
        session.projectiles.first().expect("shot").fireball_mode(),
        FireballMode::ContactOnly
    );
}

#[test]
fn entering_folds_glider_and_exiting_limits_retained_velocity() {
    let (mut session, view, geometry, _) = fixture();
    let actor = session.actors.first_mut().expect("player");
    actor.feet.y = 20.0;
    actor.grounded = false;
    actor.body.control_velocity = Vec3::NEG_Z * 20.0;
    crate::glider::prepare(
        actor,
        ActorIntent {
            glider_toggle: true,
            ..Default::default()
        },
        &session.collision,
        &view,
        geometry,
    );
    assert!(actor.glider.open);
    let start = actor.feet;
    prepare(
        actor,
        ActorIntent {
            flight_toggle: true,
            ..Default::default()
        },
    );
    assert!(!actor.glider.open);
    assert!(actor.feet.distance(start) < 0.0001);
    tick_or_wait(
        actor,
        ActorIntent {
            movement: Vec2::Y,
            flight_fast: true,
            ..Default::default()
        },
        &session.collision,
    );
    prepare(
        actor,
        ActorIntent {
            flight_toggle: true,
            ..Default::default()
        },
    );
    let retained = actor.body.control_velocity + Vec3::Y * actor.body.vertical_velocity;
    assert!((retained.length() - EXIT_SPEED).abs() < 0.001);
    assert!(!actor.free_flight().expect("flight").active);
}

#[test]
fn high_jump_returns_to_gravity_with_ordinary_boost_and_cooldown() {
    let (mut session, view, geometry, materials) = fixture();
    fly(&mut session);
    session.advance(
        ActorIntent {
            high_jump: true,
            ..Default::default()
        },
        &view,
        geometry,
        materials,
        &ArenaTuning::default(),
    );
    let actor = session.actors.first().expect("player");
    assert!(!actor.free_flight().expect("flight").active);
    assert!(actor.body.vertical_velocity > 10.0);
    assert!(actor
        .cooldowns
        .get(Spell::HighJump.index())
        .is_some_and(|cooldown| *cooldown > 0.0));
}

#[test]
fn unloaded_boundary_holds_pose_and_preserves_prefetch_then_resumes() {
    let (mut session, mut view, geometry, materials) = fixture();
    fly(&mut session);
    let actor = session.actors.first_mut().expect("player");
    actor.feet = HexCoord::from_axial(15, 8).to_world(20.0);
    actor.previous_feet = actor.feet;
    let start = actor.feet;
    view.residency
        .as_mut()
        .expect("residency")
        .ready
        .remove(&(1, 0));
    view.revision += 1;
    let input = ActorIntent {
        movement: Vec2::Y,
        glider_look: Vec3::X,
        aim: Vec3::X,
        flight_fast: true,
        ..Default::default()
    };
    for _ in 0..5 {
        session.advance(input, &view, geometry, materials, &ArenaTuning::default());
    }
    let actor = session.actors.first().expect("player");
    assert!(actor.feet.distance(start) < 0.0001);
    assert!(actor.free_flight().expect("flight").loading);
    let interest = session.stream_interest().expect("interest");
    assert!(interest.position.distance(start) < 0.0001);
    assert!(interest.velocity.distance(Vec3::X * FAST_SPEED) < 0.001);
    view.residency
        .as_mut()
        .expect("residency")
        .ready
        .insert((1, 0));
    view.revision += 1;
    session.advance(input, &view, geometry, materials, &ArenaTuning::default());
    let actor = session.actors.first().expect("player");
    assert!(!actor.free_flight().expect("flight").loading);
    assert!(actor.feet.x > start.x + 1.0);
}

#[test]
fn unknown_columns_block_movement_camera_sight_projectiles_and_shield_cells() {
    let (mut session, mut view, geometry, _) = fixture();
    view.residency
        .as_mut()
        .expect("residency")
        .ready
        .remove(&(1, 0));
    view.revision += 1;
    session.collision.refresh(&view, geometry);
    let start = HexCoord::from_axial(15, 8).to_world(20.0);
    let end = HexCoord::from_axial(17, 8).to_world(20.0);
    let world = &session.collision;
    assert!(!world.clear(end, 1.2, 0.25));
    assert!(world.sweep(start, end - start, 1.2, 0.25).is_some());
    assert!(world.attack_sweep(start, end - start, 0.1).is_some());
    assert!(!world.sight_clear(start, end));
    assert!(session.camera_position(start, end).x < end.x);
    assert!(crate::spells::available_wall_voxels(
        &[geometry.voxel_at(end).expect("voxel")],
        &view,
        geometry,
        &[],
        &BTreeSet::new()
    )
    .is_empty());
    let beyond = HexCoord::from_axial(60, 8).to_world(20.0);
    assert!(!world.clear(beyond, 1.2, 0.25));
    assert!(
        !world.needs_terrain(beyond, Vec3::ZERO, 1.2, 0.25),
        "outside is sealed, not an endless loading request"
    );
}

#[test]
fn thin_solid_wall_stops_fast_flight_without_tunneling() {
    let (mut session, mut view, geometry, materials) = fixture();
    fly(&mut session);
    let coord = HexCoord::from_axial(10, 8);
    view.dirty_columns.insert(coord);
    view.columns.insert(
        coord,
        vec![ArenaSolidSpan {
            bottom: TilePos::new(coord, 0),
            top_level: 100,
            substance: SubstanceId(1),
        }],
    );
    view.revision += 1;
    let start = session.actors.first().expect("player").feet;
    for _ in 0..10 {
        session.advance(
            ActorIntent {
                movement: Vec2::Y,
                glider_look: Vec3::X,
                aim: Vec3::X,
                flight_fast: true,
                ..Default::default()
            },
            &view,
            geometry,
            materials,
            &ArenaTuning::default(),
        );
    }
    let actor = session.actors.first().expect("player");
    assert!(actor.feet.x > start.x);
    assert!(
        actor.feet.x < coord.to_world(0.0).x,
        "start={start:?} feet={:?} wall={:?}",
        actor.feet,
        coord.to_world(0.0)
    );
    assert!(session.collision.clear(actor.feet, 1.2, 0.25));
}

#[test]
fn pause_charge_cancellation_preserves_flight_but_death_restart_and_map_switch_clear_it() {
    let (mut session, view, geometry, materials) = fixture();
    fly(&mut session);
    let before = session.actors.first().expect("player").free_flight();
    let tick = session.tick;
    session.cancel_charges();
    assert_eq!(
        session.actors.first().expect("player").free_flight(),
        before
    );
    assert_eq!(session.tick, tick);
    session.actors.first_mut().expect("player").hp = 0.0;
    session.advance(
        ActorIntent::default(),
        &view,
        geometry,
        materials,
        &ArenaTuning::default(),
    );
    assert!(
        !session
            .actors
            .first()
            .expect("player")
            .free_flight()
            .expect("flight")
            .active
    );
    assert!(session.is_finished());
    session.reset_with_setup(2, &view, geometry, &ArenaBattleSetup::default());
    assert!(!session.is_finished());
    assert_eq!(
        session.actors.first().expect("player").free_flight(),
        Some(FreeFlightSnapshot::default())
    );
    assert!(session.progress().is_none());
    let mut duel = view;
    duel.selection.map = ArenaMap::Duel;
    duel.residency = None;
    session.reset_with_setup(3, &duel, geometry, &ArenaBattleSetup::default());
    assert!(!session.is_exploration());
    assert_eq!(session.actors.len(), 2);
    assert!(session
        .actors
        .iter()
        .all(|actor| actor.free_flight().is_none()));
}

#[test]
fn unready_spawn_waits_for_admission_without_invalidating_the_run() {
    let (mut session, mut view, geometry, materials) = fixture();
    view.residency
        .as_mut()
        .expect("residency")
        .ready
        .remove(&(0, 0));
    view.revision += 1;
    session.reset_with_setup(2, &view, geometry, &ArenaBattleSetup::default());
    session.advance(
        ActorIntent::default(),
        &view,
        geometry,
        materials,
        &ArenaTuning::default(),
    );
    assert!(!session.encounter.initialized);
    assert!(!session.is_finished());
    assert!(
        session
            .actors
            .first()
            .expect("player")
            .free_flight()
            .expect("flight")
            .loading
    );
    view.residency
        .as_mut()
        .expect("residency")
        .ready
        .insert((0, 0));
    view.revision += 1;
    session.advance(
        ActorIntent::default(),
        &view,
        geometry,
        materials,
        &ArenaTuning::default(),
    );
    assert!(session.encounter.initialized);
    assert!(!session.is_finished());
}

#[test]
fn walking_and_gliding_also_wait_at_unknown_columns_without_losing_momentum() {
    for gliding in [false, true] {
        let (mut session, mut view, geometry, materials) = fixture();
        let actor = session.actors.first_mut().expect("player");
        actor.feet = HexCoord::from_axial(15, 8).to_world(20.0) + Vec3::X * 0.60;
        actor.previous_feet = actor.feet;
        actor.aim = Vec3::X;
        actor.grounded = false;
        actor.body.control_velocity = Vec3::X * 20.0;
        if gliding {
            crate::glider::prepare(
                actor,
                ActorIntent {
                    glider_toggle: true,
                    glider_look: Vec3::X,
                    ..Default::default()
                },
                &session.collision,
                &view,
                geometry,
            );
        }
        let start = actor.feet;
        let glide_before = actor.glider();
        view.residency
            .as_mut()
            .expect("residency")
            .ready
            .remove(&(1, 0));
        view.revision += 1;
        session.advance(
            ActorIntent {
                movement: Vec2::Y,
                aim: Vec3::X,
                glider_look: Vec3::X,
                ..Default::default()
            },
            &view,
            geometry,
            materials,
            &ArenaTuning::default(),
        );
        let actor = session.actors.first().expect("player");
        assert!(actor.feet.distance(start) < 0.0001);
        assert!(actor.free_flight().expect("flight").loading);
        assert_eq!(actor.glider(), glide_before);
    }
}

#[test]
fn player_prediction_stops_at_unknown_terrain_and_health_does_not_regenerate() {
    let (mut session, mut view, geometry, materials) = fixture();
    fly(&mut session);
    let actor = session.actors.first_mut().expect("player");
    actor.feet = HexCoord::from_axial(15, 8).to_world(20.0);
    actor.aim = Vec3::X;
    actor.hp = 50.0;
    view.residency
        .as_mut()
        .expect("residency")
        .ready
        .remove(&(1, 0));
    view.revision += 1;
    session.tick = 100_000;
    session.advance(
        ActorIntent {
            aim: Vec3::X,
            ..Default::default()
        },
        &view,
        geometry,
        materials,
        &ArenaTuning::default(),
    );
    assert!((session.actors.first().expect("player").hp - 50.0).abs() < 0.0001);
    let predicted = crate::preview(&session, &view, &geometry, &ArenaTuning::default());
    let impact = predicted.impact.expect("unknown partition blocks preview");
    assert!(impact.x < HexCoord::from_axial(16, 8).to_world(20.0).x);
    assert!(predicted
        .points
        .iter()
        .all(|point| point.x <= impact.x + 0.001));
}
