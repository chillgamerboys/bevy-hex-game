//! Independent acceptance of shape geometry and world query-mask consumption.

use std::f32::consts::{FRAC_PI_2, FRAC_PI_6};

use bevy_math::Vec3;
use hex_core::arena::{ArenaSolidSpan, ArenaStaticSpan, ArenaTerrainView, ArenaVoxelGeometry};
use hex_core::{HexCoord, SubstanceId, TilePos};

use crate::collision::{CollisionWorld, SKIN};
use crate::{shapes, Actor, ArenaSession, BarrierSnapshot, EncounterTuning, Species};

fn dragon(feet: Vec3, yaw: f32) -> Actor {
    let tuning = EncounterTuning::default();
    let mut actor = Actor::spawn(1, feet, Vec3::NEG_Z);
    actor.species = Species::Dragon;
    actor.dimensions = Vec3::new(
        tuning.dragon_width,
        tuning.dragon_height,
        tuning.dragon_length,
    );
    actor.body_yaw = yaw;
    actor.previous_yaw = yaw;
    actor
}

fn put_run(view: &mut ArenaTerrainView, coord: HexCoord, bottom: i32, top: i32) {
    let substance = SubstanceId(1);
    for level in bottom..=top {
        view.voxels.insert(TilePos::new(coord, level), substance);
    }
    view.columns.entry(coord).or_default().push(ArenaSolidSpan {
        bottom: TilePos::new(coord, bottom),
        top_level: top,
        substance,
    });
}

fn remove_column(view: &mut ArenaTerrainView, coord: HexCoord) {
    view.voxels.retain(|pos, _| pos.coord != coord);
    view.columns.remove(&coord);
}

fn world(view: &ArenaTerrainView) -> CollisionWorld {
    let mut collision = CollisionWorld::default();
    collision.refresh(view, ArenaVoxelGeometry::default());
    collision
}

fn flat_band(view: &mut ArenaTerrainView, level: i32) {
    for coord in HexCoord::ORIGIN.within_radius(4) {
        put_run(view, coord, level, level);
    }
}

#[test]
fn low_dragon_fits_one_level_tunnel_but_cannot_cross_floor_or_ceiling() {
    let mut view = ArenaTerrainView::default();
    flat_band(&mut view, 0);
    flat_band(&mut view, 2);
    let collision = world(&view);
    let actor = dragon(Vec3::ZERO, 0.0);
    let human = Actor::spawn(0, Vec3::ZERO, Vec3::NEG_Z);

    // The open interval is exactly 0.4 units high. A human is two levels high.
    assert!(shapes::clear(&collision, &actor, actor.feet, 0.0));
    assert!(!shapes::clear(&collision, &human, human.feet, 0.0));
    assert!(!shapes::clear(&collision, &actor, Vec3::Y * 0.01, 0.0));
    assert!(!shapes::clear(&collision, &actor, Vec3::NEG_Y * 0.01, 0.0));
    assert!(shapes::sweep(&collision, &actor, actor.feet, Vec3::X).is_none());
    let ceiling = shapes::sweep(&collision, &actor, actor.feet, Vec3::Y)
        .expect("an upward move must contact the tunnel roof immediately");
    assert!(ceiling.fraction.abs() < SKIN);
    assert!(ceiling.normal.y < -0.99);
}

#[test]
fn elongated_body_voxel_overlap_depends_on_length_and_orientation() {
    let voxel = TilePos::new(HexCoord::ORIGIN, 1);
    let mut view = ArenaTerrainView::default();
    put_run(&mut view, voxel.coord, 1, 1);
    let collision = world(&view);
    let geometry = ArenaVoxelGeometry::default();
    let mut actor = dragon(Vec3::new(0.0, 0.0, 2.5), 0.0);

    assert!(shapes::voxel_overlap(voxel, geometry, &actor));
    assert!(!shapes::clear(
        &collision,
        &actor,
        actor.feet,
        actor.body_yaw
    ));
    actor.body_yaw = FRAC_PI_2;
    actor.previous_yaw = FRAC_PI_2;
    assert!(!shapes::voxel_overlap(voxel, geometry, &actor));
    assert!(shapes::clear(
        &collision,
        &actor,
        actor.feet,
        actor.body_yaw
    ));
}

#[test]
fn projectile_hits_long_body_tip_and_respects_its_low_height_and_yaw() {
    let mut actor = dragon(Vec3::ZERO, 0.0);
    let start = Vec3::new(-3.0, 0.2, 1.5);
    let delta = Vec3::X * 6.0;
    let hit = shapes::sweep_dragon(start, delta, &actor, true, 0.02)
        .expect("the longitudinal tip extends well beyond a human capsule");
    assert!((hit.fraction - (3.0 - 0.866_025_4 - 0.02) / 6.0).abs() < 0.001);
    assert!(shapes::sweep_dragon(start + Vec3::Y * 0.25, delta, &actor, true, 0.02).is_none());

    actor.body_yaw = FRAC_PI_2;
    actor.previous_yaw = FRAC_PI_2;
    assert!(shapes::sweep_dragon(start, delta, &actor, true, 0.02).is_none());
    assert!(shapes::sweep_dragon(Vec3::new(-3.0, 0.2, 0.0), delta, &actor, true, 0.02).is_some());
}

#[test]
fn projectile_sweep_accounts_for_dragon_crossing_between_tick_poses() {
    let mut actor = dragon(Vec3::new(3.0, 0.0, 0.0), 0.0);
    actor.previous_feet = Vec3::new(-3.0, 0.0, 0.0);
    let start = Vec3::Y * 0.2;
    let hit = shapes::sweep_dragon(start, Vec3::ZERO, &actor, false, 0.0)
        .expect("the moving body crosses the stationary projectile during this tick");
    assert!((hit.fraction - (3.0 - 0.866_025_4) / 6.0).abs() < 0.001);
    assert!(shapes::sweep_dragon(start, Vec3::ZERO, &actor, true, 0.0).is_none());
}

#[test]
fn safe_turn_rejects_blocked_corner_arc_even_when_both_end_poses_fit() {
    let voxel = TilePos::new(HexCoord::from_axial(1, 1), 1);
    let mut view = ArenaTerrainView::default();
    put_run(&mut view, voxel.coord, 1, 1);
    let collision = world(&view);
    let mut actor = dragon(Vec3::new(0.19, 0.0, 0.0), 0.0);

    assert!(shapes::clear(&collision, &actor, actor.feet, 0.0));
    assert!(shapes::clear(&collision, &actor, actor.feet, FRAC_PI_2));
    actor.body_yaw = FRAC_PI_6;
    assert!(shapes::voxel_overlap(
        voxel,
        ArenaVoxelGeometry::default(),
        &actor
    ));
    actor.body_yaw = 0.0;
    shapes::turn(&collision, &mut actor, FRAC_PI_2, FRAC_PI_2);
    assert!(actor.body_yaw < FRAC_PI_2 - 0.01);
    assert!(shapes::clear(
        &collision,
        &actor,
        actor.feet,
        actor.body_yaw
    ));

    let empty = CollisionWorld::default();
    shapes::turn(&empty, &mut actor, FRAC_PI_2, FRAC_PI_2);
    assert!((actor.body_yaw - FRAC_PI_2).abs() < 0.001);
}

#[test]
fn flight_sweep_stops_at_roof_and_ground_query_preserves_stacked_support() {
    let mut view = ArenaTerrainView::default();
    flat_band(&mut view, 0);
    flat_band(&mut view, 4);
    let collision = world(&view);
    let mut actor = dragon(Vec3::Y * 0.5, 0.0);
    actor.flying = true;

    let (stopped, contacts) = shapes::slide(&collision, &actor, actor.feet, Vec3::Y * 2.0);
    assert!((stopped.y - (0.8 - SKIN)).abs() < 0.001);
    assert!(contacts.iter().any(|normal| normal.y < -0.99));
    assert!(shapes::clear(&collision, &actor, stopped, actor.body_yaw));
    let floor = shapes::ground(&collision, &actor, stopped, 2.0).expect("lower dry support");
    assert!((floor.y - SKIN).abs() < 0.001);
    let upper = shapes::ground(&collision, &actor, Vec3::Y * 2.0, 3.0)
        .expect("the same horizontal column has a separate roof support");
    assert!((upper.y - (1.6 + SKIN)).abs() < 0.001);

    let mut roof_only = ArenaTerrainView::default();
    flat_band(&mut roof_only, 4);
    assert!(shapes::ground(&world(&roof_only), &actor, stopped, 2.0).is_none());
}

#[test]
fn static_object_masks_independently_control_body_camera_sight_and_attack_queries() {
    let start = Vec3::new(-4.0, 0.6, 0.0);
    let delta = Vec3::X * 8.0;
    for (movement, sight, attack) in [
        (true, false, false),
        (false, true, false),
        (false, false, true),
        (true, true, true),
    ] {
        let view = ArenaTerrainView {
            static_spans: vec![ArenaStaticSpan {
                bottom: TilePos::new(HexCoord::ORIGIN, 1),
                top_level: 3,
                blocks_movement: movement,
                blocks_projectiles: attack,
                blocks_sight: sight,
            }],
            ..Default::default()
        };
        let collision = world(&view);
        assert_eq!(
            collision
                .sweep(start - Vec3::Y * 0.4, delta, 0.4, 0.2)
                .is_some(),
            movement
        );
        assert_eq!(collision.sight_clear(start, start + delta), !sight);
        assert_eq!(collision.attack_sweep(start, delta, 0.02).is_some(), attack);
        let session = ArenaSession {
            collision,
            ..Default::default()
        };
        let camera = session.camera_position(start, start + delta);
        assert_eq!(camera.distance(start + delta) > 0.1, movement);
    }
}

#[test]
fn live_barrier_intercepts_attacks_both_ways_but_not_bodies_sight_or_camera() {
    let mut collision = CollisionWorld::default();
    let mut barrier = BarrierSnapshot {
        id: 7,
        owner: 1,
        center: Vec3::Y * 0.8,
        normal: Vec3::X,
        width: 3.464_101_6,
        height: 1.6,
        hp: 60.0,
        max_hp: 60.0,
        remaining: 4.0,
        lifetime: 4.0,
    };
    collision.sync_barriers(std::slice::from_ref(&barrier));
    let start = Vec3::new(-4.0, 0.8, 0.0);
    let delta = Vec3::X * 8.0;
    for (origin, travel) in [(start, delta), (start + delta, -delta)] {
        let (hit, id) = collision
            .attack_sweep(origin, travel, 0.0)
            .expect("live barrier contact");
        assert_eq!(id, Some(7));
        assert!((hit.fraction - (4.0 - 0.025) / 8.0).abs() < 0.001);
        assert!(collision.sight_clear(origin, origin + travel));
        assert!(collision
            .sweep(origin - Vec3::Y * 0.4, travel, 0.8, 0.25)
            .is_none());
    }
    assert!(collision
        .attack_sweep(start + Vec3::Y * 2.0, delta, 0.0)
        .is_none());
    let session = ArenaSession {
        collision: collision.clone(),
        ..Default::default()
    };
    assert!(
        session
            .camera_position(start, start + delta)
            .distance(start + delta)
            < SKIN
    );

    barrier.remaining = 0.0;
    collision.sync_barriers(std::slice::from_ref(&barrier));
    assert!(collision.attack_sweep(start, delta, 0.0).is_none());
    barrier.remaining = 4.0;
    barrier.hp = 0.0;
    collision.sync_barriers(std::slice::from_ref(&barrier));
    assert!(collision.attack_sweep(start, delta, 0.0).is_none());
}

#[test]
fn collision_refresh_adopts_dirty_columns_and_recovers_after_a_missed_revision() {
    let a = HexCoord::ORIGIN;
    let b = HexCoord::from_axial(4, 0);
    let c = HexCoord::from_axial(-4, 0);
    let geometry = ArenaVoxelGeometry::default();
    let mut view = ArenaTerrainView {
        revision: 10,
        full_rebuild: true,
        ..Default::default()
    };
    put_run(&mut view, a, 1, 2);
    put_run(&mut view, b, 1, 2);
    let mut collision = world(&view);
    let blocked =
        |world: &CollisionWorld, coord: HexCoord| !world.clear(coord.to_world(0.1), 0.4, 0.1);
    assert!(blocked(&collision, a) && blocked(&collision, b));

    remove_column(&mut view, a);
    view.revision = 11;
    view.full_rebuild = false;
    view.dirty_columns = [a].into();
    collision.refresh(&view, geometry);
    assert!(!blocked(&collision, a));
    assert!(
        blocked(&collision, b),
        "an untouched column must survive an incremental edit"
    );

    // Revision 12 added C while this consumer was absent. Revision 13 only dirties B.
    put_run(&mut view, c, 1, 2);
    remove_column(&mut view, b);
    view.revision = 13;
    view.dirty_columns = [b].into();
    collision.refresh(&view, geometry);
    assert_eq!(collision.revision, Some(13));
    assert!(!blocked(&collision, a) && !blocked(&collision, b));
    assert!(
        blocked(&collision, c),
        "a skipped revision requires complete current occupancy"
    );

    remove_column(&mut view, c);
    view.revision = 14;
    view.dirty_columns = [c].into();
    collision.refresh(&view, geometry);
    assert!(
        !blocked(&collision, c),
        "removing the last compact column must clear the cache"
    );
}
