//! Production flight integration for the one-level physical body, without AI.

use super::*;
use hex_core::arena::{ArenaTerrainView, ArenaVoxelGeometry};
use hex_core::{HexCoord, SubstanceId, TilePos};

fn wisp(feet: Vec3) -> Actor {
    let mut actor = Actor::spawn(1, feet, Vec3::X);
    actor.configure_species(Species::Wisp, &EncounterTuning::default());
    actor
}

fn world(cells: impl IntoIterator<Item = TilePos>) -> CollisionWorld {
    let mut view = ArenaTerrainView::default();
    for cell in cells {
        view.voxels.insert(cell, SubstanceId(1));
    }
    let mut world = CollisionWorld::default();
    world.refresh(&view, ArenaVoxelGeometry::default());
    world
}

#[test]
fn hover_is_intrinsic_and_all_voluntary_directions_keep_the_same_slow_speed() {
    let tuning = EncounterTuning::default();
    let world = CollisionWorld::default();
    for direction in [Vec3::ZERO, Vec3::X, Vec3::new(2.0, 3.0, -4.0)] {
        let start = Vec3::new(2.0, 4.0, 3.0);
        let mut actor = wisp(start);
        actor.body_yaw = 1.2;
        for tick_number in 0_u16..120 {
            // Ordinary bootstrap and stationary casts need not request flight.
            tick(
                &mut actor,
                direction,
                tick_number.is_multiple_of(2),
                true,
                false,
                &world,
                &tuning,
            );
            assert!(actor.flying && !actor.grounded && !actor.body.grounded);
            assert!(actor.body_yaw.abs() < SKIN && actor.previous_yaw.abs() < SKIN);
            assert!(actor.body.vertical_velocity.abs() < SKIN);
            assert!(actor.body.step_rise.abs() < SKIN);
        }
        let expected = start + direction.clamp_length_max(1.0) * tuning.wisp_flight_speed;
        assert!(actor.feet.distance(expected) < SKIN * 8.0);
    }
}

#[test]
fn stationary_windup_preserves_and_damps_real_horizontal_and_vertical_impulse() {
    let tuning = EncounterTuning::default();
    let mut actor = wisp(Vec3::Y * 4.0);
    actor.body.impulse_velocity = Vec3::new(6.0, 2.0, -3.0);
    actor.body.vertical_velocity = 1.0;
    let initial = Vec3::new(6.0, 3.0, -3.0);
    let start = actor.feet;
    tick(
        &mut actor,
        Vec3::ZERO,
        false,
        false,
        false,
        &CollisionWorld::default(),
        &tuning,
    );
    assert!(actor.feet.distance(start + initial * STEP) < SKIN);
    assert!(
        actor
            .body
            .impulse_velocity
            .distance(initial * (-3.0 * STEP).exp())
            < SKIN
    );
    assert!(actor.body.vertical_velocity.abs() < SKIN);
    for _ in 0..119 {
        tick(
            &mut actor,
            Vec3::ZERO,
            false,
            false,
            false,
            &CollisionWorld::default(),
            &tuning,
        );
    }
    assert!(actor.feet.y > start.y + 0.9);
    assert!(
        actor
            .body
            .impulse_velocity
            .distance(initial * (-3.0_f32).exp())
            < SKIN
    );
}

#[test]
fn large_sideways_impulse_cannot_cross_a_single_hex_wall_and_keeps_tangent_motion() {
    let obstacle = HexCoord::from_axial(2, 0);
    let world = world((1..=20).map(|level| TilePos::new(obstacle, level)));
    let tuning = EncounterTuning::default();
    let mut actor = wisp(Vec3::Y * 4.0);
    actor.body.impulse_velocity = Vec3::new(500.0, 0.0, 2.0);
    tick(&mut actor, Vec3::ZERO, false, false, true, &world, &tuning);
    let limit = hex_core::config::HEX_SMALL_DIAMETER;
    assert!((actor.feet.x - (limit - SKIN)).abs() < SKIN * 2.0);
    assert!(actor.feet.z > 0.01);
    assert!(actor.body.impulse_velocity.x.abs() < SKIN);
    assert!(actor.body.impulse_velocity.z > 1.9);
    assert!(shapes::clear(&world, &actor, actor.feet, 0.0));
}

#[test]
fn swept_roof_and_floor_stop_vertical_impulse_without_grounding_or_gravity() {
    let cells = HexCoord::ORIGIN
        .within_radius(3)
        .into_iter()
        .flat_map(|coord| [TilePos::new(coord, 0), TilePos::new(coord, 14)]);
    let world = world(cells);
    let tuning = EncounterTuning::default();
    for (impulse, expected_y) in [(500.0, 4.8 - SKIN), (-1000.0, SKIN)] {
        let mut actor = wisp(Vec3::Y * 4.0);
        actor.body.impulse_velocity = Vec3::Y * impulse;
        tick(&mut actor, Vec3::ZERO, false, false, true, &world, &tuning);
        assert!((actor.feet.y - expected_y).abs() < SKIN * 4.0);
        assert!(actor.body.impulse_velocity.y.abs() < SKIN);
        assert!(actor.flying && !actor.grounded && !actor.body.grounded);
        assert!(shapes::clear(&world, &actor, actor.feet, 0.0));
        let resting = actor.feet;
        for _ in 0..120 {
            tick(&mut actor, Vec3::ZERO, true, true, false, &world, &tuning);
        }
        assert!(actor.feet.distance(resting) < SKIN);
    }
}

#[test]
fn continuous_flight_keeps_two_admitted_layers_distinct() {
    let mut lower = wisp(Vec3::Y * 4.0);
    let mut upper = wisp(Vec3::Y * 4.8);
    let world = CollisionWorld::default();
    let tuning = EncounterTuning::default();
    for _ in 0..360 {
        for actor in [&mut lower, &mut upper] {
            tick(actor, Vec3::X, false, false, true, &world, &tuning);
            assert!(shapes::clear(&world, actor, actor.feet, 0.0));
        }
        assert!((upper.feet.y - lower.feet.y - 0.8).abs() < SKIN);
        assert!(shapes::compound_separation(&lower, &upper).is_none());
    }
}
