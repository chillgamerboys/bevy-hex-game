use super::*;
use crate::{ArenaSession, ArenaTuning, Spell};
use hex_core::{HexCoord, SubstanceId, TilePos};

#[test]
fn transition_ground_open_glider_keeps_same_tick_walk_and_jump_momentum() {
    let (mut actor, mut world, mut view, geometry) = fixture();
    for coord in HexCoord::ORIGIN.within_radius(5) {
        view.voxels.insert(TilePos::new(coord, 0), SubstanceId(1));
    }
    view.revision += 1;
    world.refresh(&view, geometry);
    actor.feet = Vec3::Y * SKIN;
    actor.grounded = true;
    actor.body.grounded = true;
    open(&mut actor, &world, &view, geometry);
    crate::motion::tick(
        &mut actor,
        Vec3::X,
        false,
        true,
        false,
        &world,
        &ArenaTuning::default().encounters,
    );
    let takeoff = actor.body.control_velocity + Vec3::Y * actor.body.vertical_velocity;
    assert!(takeoff.x > 5.0 && takeoff.y > 5.0);
    finish(&mut actor, &view, geometry);
    let mut flying = actor.clone();
    tick(&mut flying, &world, airborne_profile());
    assert!(flying.glider.velocity.x > takeoff.x - 0.2);
    // Folding before the first airborne tick must preserve the same takeoff too.
    fold(&mut actor);
    let folded = actor.body.control_velocity + Vec3::Y * actor.body.vertical_velocity;
    assert!(folded.distance(takeoff) < 0.0001);
}

#[test]
fn wind_relative_airspeed_changes_ground_travel_without_a_toggle_kick() {
    let (mut actor, world, view, geometry) = fixture();
    actor.body.control_velocity = Vec3::NEG_Z * 20.0;
    actor.glider.wind = Vec3::NEG_Z * 10.0;
    open(&mut actor, &world, &view, geometry);
    assert!((actor.glider.snapshot().airspeed - 10.0).abs() < 0.0001);
    assert!(actor.glider.velocity.distance(Vec3::NEG_Z * 20.0) < 0.0001);
    let mut calm = GliderState {
        velocity: Vec3::NEG_Z * 10.0,
        ..Default::default()
    };
    let mut tailwind = GliderState {
        velocity: Vec3::NEG_Z * 20.0,
        wind: Vec3::NEG_Z * 10.0,
        ..Default::default()
    };
    velocity_step(&mut calm);
    velocity_step(&mut tailwind);
    assert!((tailwind.velocity - calm.velocity).distance(Vec3::NEG_Z * 10.0) < 0.0001);
    fold(&mut actor);
    assert!(actor.body.control_velocity.distance(Vec3::NEG_Z * 20.0) < 0.0001);
}

#[test]
fn grounded_canopy_can_open_without_altering_walking_or_jump() {
    let (mut actor, mut world, mut view, geometry) = fixture();
    for coord in HexCoord::ORIGIN.within_radius(5) {
        view.voxels.insert(TilePos::new(coord, 0), SubstanceId(1));
    }
    view.revision += 1;
    world.refresh(&view, geometry);
    actor.feet = Vec3::Y * SKIN;
    actor.grounded = true;
    actor.body.grounded = true;
    let mut closed = actor.clone();
    open(&mut actor, &world, &view, geometry);
    for _ in 0..30 {
        crate::motion::tick(
            &mut actor,
            Vec3::X,
            false,
            false,
            false,
            &world,
            &ArenaTuning::default().encounters,
        );
        crate::motion::tick(
            &mut closed,
            Vec3::X,
            false,
            false,
            false,
            &world,
            &ArenaTuning::default().encounters,
        );
        finish(&mut actor, &view, geometry);
        assert!(actor.feet.distance(closed.feet) < 0.0001);
        assert!(actor.glider.open && actor.grounded);
    }
    crate::motion::tick(
        &mut actor,
        Vec3::X,
        false,
        true,
        false,
        &world,
        &ArenaTuning::default().encounters,
    );
    assert!(!actor.grounded && actor.glider.open && actor.body.vertical_velocity > 0.0);
}

fn fixture() -> (Actor, CollisionWorld, ArenaTerrainView, ArenaVoxelGeometry) {
    let mut actor = Actor::spawn(0, Vec3::Y * 100.0, Vec3::NEG_Z);
    actor.configure_expedition_player();
    let view = ArenaTerrainView::default();
    let geometry = ArenaVoxelGeometry::default();
    let mut world = CollisionWorld::default();
    world.refresh(&view, geometry);
    (actor, world, view, geometry)
}

fn open(
    actor: &mut Actor,
    world: &CollisionWorld,
    view: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
) {
    prepare(
        actor,
        ActorIntent {
            glider_toggle: true,
            ..Default::default()
        },
        world,
        view,
        geometry,
    );
    assert!(actor.glider.open);
}

fn airborne_profile() -> GroundProfile {
    GroundProfile {
        height: 1.2,
        radius: 0.25,
        ..Default::default()
    }
}

#[test]
fn opening_and_repeated_toggles_conserve_all_velocity_without_position_kicks() {
    let (mut actor, world, view, geometry) = fixture();
    actor.body.control_velocity = Vec3::new(3.0, 0.0, -4.0);
    actor.body.impulse_velocity = Vec3::new(2.0, 1.0, 0.0);
    actor.body.vertical_velocity = -3.0;
    let expected = Vec3::new(5.0, -2.0, -4.0);
    let feet = actor.feet;
    for _ in 0..20 {
        open(&mut actor, &world, &view, geometry);
        assert!(actor.glider.velocity.distance(expected) < 0.0001);
        fold(&mut actor);
        assert!(!actor.glider.open);
        let velocity = actor.body.control_velocity + Vec3::Y * actor.body.vertical_velocity;
        assert!(velocity.distance(expected) < 0.0001);
        assert!(actor.feet.distance(feet) < 0.0001);
    }
}

#[test]
fn acceleration_matches_dive_climb_and_level_targets() {
    for (pitch, expected) in [(-45_f32, 12.0), (30.0, -14.0), (0.0, -0.6)] {
        let angle = pitch.to_radians();
        let direction = Vec3::new(0.0, angle.sin(), -angle.cos());
        assert!((acceleration(20.0, direction) - expected).abs() < 0.4);
    }
}

#[test]
fn pitch_turning_and_speed_are_bounded_without_instant_camera_snap() {
    let mut state = GliderState {
        open: true,
        velocity: Vec3::NEG_Z * 20.0,
        look: clamped_look(Vec3::X + Vec3::Y * 100.0),
        ..Default::default()
    };
    velocity_step(&mut state);
    assert!(state.velocity.normalize().dot(Vec3::NEG_Z) > 0.999);
    for _ in 0..1200 {
        velocity_step(&mut state);
        assert!(state.velocity.length() <= MAX_SPEED + 0.0001);
        assert!(state.heading.y.asin() <= 30_f32.to_radians() + 0.0001);
        assert!(state.heading.y.asin() >= -60_f32.to_radians() - 0.0001);
    }
}

#[test]
fn low_speed_loses_lift_remains_open_and_a_dive_recovers_speed() {
    let mut state = GliderState {
        open: true,
        velocity: Vec3::NEG_Z * 4.0,
        ..Default::default()
    };
    for _ in 0..60 {
        velocity_step(&mut state);
    }
    assert!(state.velocity.y < -2.0, "slow glider must visibly descend");
    assert!(state.open);
    state.look = clamped_look(Vec3::NEG_Z - Vec3::Y * 2.0);
    for _ in 0..120 {
        velocity_step(&mut state);
    }
    assert!(state.velocity.length() > FULL_LIFT_SPEED);
    assert!(state.open);
}

#[test]
fn folded_flight_keeps_horizontal_momentum_when_casting() {
    let (mut actor, world, view, geometry) = fixture();
    actor.body.control_velocity = Vec3::NEG_Z * 20.0;
    open(&mut actor, &world, &view, geometry);
    prepare(
        &mut actor,
        ActorIntent {
            cast_pressed: true,
            cast_held: true,
            ..Default::default()
        },
        &world,
        &view,
        geometry,
    );
    assert!(!actor.glider.open);
    let start = actor.feet;
    actor.body.tick_profile(
        &mut actor.feet,
        Vec3::ZERO,
        false,
        false,
        &world,
        airborne_profile(),
    );
    assert!((actor.feet - start).z < -19.9 * STEP);
}

#[test]
fn repeated_airborne_high_jump_folds_and_requires_manual_reopening() {
    let (mut actor, world, view, geometry) = fixture();
    let tuning = ArenaTuning::default();
    actor.body.control_velocity = Vec3::NEG_Z * 10.0;
    for _ in 0..3 {
        open(&mut actor, &world, &view, geometry);
        prepare(
            &mut actor,
            ActorIntent {
                high_jump: true,
                ..Default::default()
            },
            &world,
            &view,
            geometry,
        );
        assert!(!actor.glider.open);
        assert!(actor.high_jump(&tuning));
        assert!(actor.body.vertical_velocity >= (2.0 * GRAVITY * tuning.high_jump_height).sqrt());
        assert!(!actor.grounded);
        prepare(&mut actor, ActorIntent::default(), &world, &view, geometry);
        assert!(!actor.glider.open);
        // Model elapsed airborne cooldown without inventing a landing event.
        if let Some(cooldown) = actor.cooldowns.get_mut(Spell::HighJump.index()) {
            *cooldown = 0.0;
        }
    }
}

#[test]
fn legacy_dead_and_charging_actors_cannot_open() {
    for mode in 1..4 {
        let (mut actor, world, view, geometry) = fixture();
        let mut intent = ActorIntent {
            glider_toggle: true,
            ..Default::default()
        };
        match mode {
            1 => actor.expedition_player = false,
            2 => actor.hp = 0.0,
            _ => intent.cast_held = true,
        }
        prepare(&mut actor, intent, &world, &view, geometry);
        assert!(!actor.glider.open);
    }
}

#[test]
fn complete_body_sweep_folds_at_a_wall_and_preserves_tangential_motion() {
    let (mut actor, mut world, mut view, geometry) = fixture();
    actor.feet = Vec3::new(-1.2, SKIN, 0.0);
    view.voxels = (0..8)
        .map(|level| (TilePos::new(HexCoord::ORIGIN, level), SubstanceId(1)))
        .collect();
    view.revision += 1;
    world.refresh(&view, geometry);
    actor.body.control_velocity = Vec3::new(30.0, 0.0, -5.0);
    open(&mut actor, &world, &view, geometry);
    actor.glider.look = actor.glider.velocity.normalize();
    for _ in 0..30 {
        tick(&mut actor, &world, airborne_profile());
        if !actor.glider.open {
            break;
        }
    }
    assert!(!actor.glider.open);
    assert!(world.clear(actor.feet, 1.2, 0.25));
    assert!(actor.body.control_velocity.x.abs() < 0.01);
    assert!(actor.body.control_velocity.z < -4.0);
}

#[test]
fn landing_keeps_canopy_open_but_liquid_entry_folds_without_changing_water() {
    let (mut actor, mut world, mut view, geometry) = fixture();
    view.voxels
        .insert(TilePos::new(HexCoord::ORIGIN, 0), SubstanceId(1));
    view.revision += 1;
    world.refresh(&view, geometry);
    actor.feet = Vec3::Y * 0.1;
    actor.body.vertical_velocity = -10.0;
    open(&mut actor, &world, &view, geometry);
    for _ in 0..10 {
        tick(&mut actor, &world, airborne_profile());
        if actor.grounded {
            break;
        }
    }
    assert!(actor.grounded && actor.glider.open);
    fold(&mut actor);
    actor.feet = Vec3::Y * 2.0;
    actor.grounded = false;
    open(&mut actor, &world, &view, geometry);
    view.liquids.push(hex_core::arena::ArenaSolidSpan {
        bottom: TilePos::new(HexCoord::ORIGIN, 1),
        top_level: 10,
        substance: SubstanceId(2),
    });
    finish(&mut actor, &view, geometry);
    assert!(!actor.glider.open);
    assert_eq!(view.liquids.len(), 1);
}

#[test]
fn canceling_charges_for_pause_preserves_flight_but_death_and_reset_clear_it() {
    let (mut actor, world, view, geometry) = fixture();
    actor.body.control_velocity = Vec3::NEG_Z * 12.0;
    open(&mut actor, &world, &view, geometry);
    let expected = actor.glider();
    let mut session = ArenaSession {
        actors: vec![actor],
        ..Default::default()
    };
    session.cancel_charges();
    assert_eq!(session.actors.first().and_then(Actor::glider), expected);
    let player = session.actors.first_mut().expect("player");
    player.hp = 0.0;
    finish(player, &view, geometry);
    assert!(!player.glider.open);
    assert!(player.glider.velocity.length_squared() < 0.0001);
    session.reset(1, &view, geometry);
    assert!(session.actors.iter().all(|actor| !actor.glider.open));
}

#[test]
fn diving_and_climbing_spend_mechanical_energy_without_a_toggle_source() {
    let mut state = GliderState {
        open: true,
        velocity: Vec3::NEG_Z * 20.0,
        ..Default::default()
    };
    let mut height = 100.0;
    let initial_energy = 0.5 * state.velocity.length_squared() + GRAVITY * height;
    for tick in 0..2400 {
        state.look = clamped_look(if tick % 480 < 240 {
            Vec3::NEG_Z - Vec3::Y
        } else {
            Vec3::NEG_Z + Vec3::Y
        });
        velocity_step(&mut state);
        height += state.velocity.y * STEP;
        let energy = 0.5 * state.velocity.length_squared() + GRAVITY * height;
        assert!(
            energy < initial_energy + 0.01,
            "flight cannot gain energy from steering alone"
        );
    }
}
