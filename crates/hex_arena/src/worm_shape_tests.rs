//! Dynamic segment history and observed-copy checks through shared shape queries.
use super::*;
use crate::{BodyHexPrism, BodyPrismSnapshot};

fn snapshot(offsets: impl IntoIterator<Item = Vec3>) -> BodyPrismSnapshot {
    let parts: Vec<_> = offsets
        .into_iter()
        .map(|offset| BodyHexPrism {
            offset,
            height: 0.4,
        })
        .collect();
    BodyPrismSnapshot::try_from_parts(&parts).expect("valid physical fixture")
}

fn worm(parts: BodyPrismSnapshot) -> Actor {
    let mut actor = Actor::spawn(1, Vec3::Y * 0.4, Vec3::X);
    actor.species = Species::Worm;
    actor
        .set_observed_prisms(parts)
        .expect("four/six segment body");
    actor
}

#[test]
fn a_rising_head_can_hit_a_stationary_projectile_between_clear_endpoint_poses() {
    for count in [4_u8, 6] {
        let before = snapshot((0..count).map(|i| Vec3::NEG_X * f32::from(i) * 1.732_050_8));
        let after = snapshot((0..count).map(|i| {
            Vec3::NEG_X * f32::from(i) * 1.732_050_8
                + Vec3::Y * (2.0 * (1.0 - f32::from(i) / f32::from(count - 1)))
        }));
        let ray = Vec3::new(0.0, 1.2, 0.0);
        let start = worm(before);
        let mut end = worm(after);
        assert!(sweep_actor(ray, Vec3::ZERO, &start, true, 0.02).is_none());
        assert!(sweep_actor(ray, Vec3::ZERO, &end, true, 0.02).is_none());
        end.worm.as_mut().expect("body").previous = before;
        let hit = sweep_actor(ray, Vec3::ZERO, &end, false, 0.02).expect("rising head crosses ray");
        assert!(hit.fraction > 0.0 && hit.fraction < 1.0);
        assert!(hit.normal.y > 0.99);
        assert!(end.feet.distance(end.previous_feet) < SKIN);
    }
}

#[test]
fn rotating_tail_history_is_swept_independently_of_the_stationary_head_anchor() {
    let before = snapshot((0_u8..4).map(|i| Vec3::NEG_X * f32::from(i) * 1.732_050_8));
    let after = snapshot((0_u8..4).map(|i| Vec3::NEG_Z * f32::from(i) * 1.732_050_8));
    let ray = Vec3::new(-2.598_076, 0.6, -2.598_076);
    let start = worm(before);
    let mut end = worm(after);
    assert!(sweep_actor(ray, Vec3::ZERO, &start, true, 0.0).is_none());
    assert!(sweep_actor(ray, Vec3::ZERO, &end, true, 0.0).is_none());
    end.worm.as_mut().expect("body").previous = before;
    assert!(sweep_actor(ray, Vec3::ZERO, &end, false, 0.0).is_some());
}

#[test]
fn copied_exposed_geometry_remains_frozen_after_the_live_worm_dives_and_turns() {
    let parts = snapshot((0_u8..4).map(|i| {
        Vec3::NEG_X * f32::from(i) * 1.732_050_8 + Vec3::Y * (1.2 * (1.0 - f32::from(i) / 3.0))
    }));
    let mut live = worm(parts);
    let fact = crate::spells::ForecastBody {
        id: live.id,
        feet: live.feet,
        velocity: Vec3::X * 2.0,
        predict_seconds: 0.5,
        species: live.species,
        team: live.team,
        dimensions: live.dimensions,
        yaw: 0.0,
        yaw_velocity: 3.0,
        prisms: live.body_prism_snapshot(),
    };
    let copied = fact.actor_at(0.5).expect("observed body");
    let ray = copied.eye() + Vec3::Z * 3.0;
    let delta = Vec3::NEG_Z * 6.0;
    let before = sweep_actor(ray, delta, &copied, true, 0.04).expect("observed head");
    live.set_observed_prisms(snapshot(
        (0_u8..4).map(|i| Vec3::Z * f32::from(i) * 1.732_050_8),
    ))
    .expect("dive body");
    live.feet += Vec3::NEG_Y * 0.8;
    let after = fact.actor_at(0.5).expect("same copied body");
    let hit = sweep_actor(ray, delta, &after, true, 0.04).expect("same observed head");
    assert_eq!(before.fraction.to_bits(), hit.fraction.to_bits());
    assert_eq!(
        before.normal.to_array().map(f32::to_bits),
        hit.normal.to_array().map(f32::to_bits)
    );
    assert_eq!(
        copied.center().to_array().map(f32::to_bits),
        after.center().to_array().map(f32::to_bits)
    );
    assert!(sweep_actor(ray, delta, &live, true, 0.04).is_none());
    for (observed, predicted) in parts.iter().zip(after.body_hex_prisms()) {
        assert_eq!(
            observed.offset.to_array().map(f32::to_bits),
            predicted.offset.to_array().map(f32::to_bits)
        );
    }
}

#[test]
fn raised_head_bounds_do_not_make_the_air_above_a_buried_tail_solid() {
    let actor = worm(snapshot((0_u8..4).map(|i| {
        Vec3::NEG_X * f32::from(i) * 1.732_050_8 + Vec3::Y * (2.0 * (1.0 - f32::from(i) / 3.0))
    })));
    let air = Vec3::new(-5.196_152, 2.2, 0.0);
    assert!(distance(air, &actor) > 1.0);
    assert!(!voxel_overlap(
        TilePos::new(hex_core::HexCoord::from_axial(-3, 0), 6),
        ArenaVoxelGeometry::default(),
        &actor,
    ));
    assert!(sweep_actor(air + Vec3::Z * 3.0, Vec3::NEG_Z * 6.0, &actor, true, 0.04).is_none());
    assert!(
        exposed_cone_contact(&actor, air + Vec3::Z * 3.0, Vec3::NEG_Z, 3.0, 0.12, |_| {
            true
        })
        .is_none()
    );
}

#[test]
fn head_anchored_tail_separation_brackets_a_truly_clear_union_pose() {
    let a = worm(snapshot(
        (0_u8..6).map(|i| Vec3::Z * f32::from(i) * 1.732_050_8),
    ));
    let mut b = Actor::spawn(2, Vec3::new(0.0, 0.4, 3.0), Vec3::X);
    assert!(compound_contact_at(&a, a.feet, &b).is_some());
    let push = compound_separation(&a, &b).expect("overlapping tail");
    let mut moved = a.clone();
    moved.feet += push;
    b.feet -= push;
    assert!(compound_contact_at(&moved, moved.feet, &b).is_none());
}
