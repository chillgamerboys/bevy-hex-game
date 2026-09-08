//! Independent exact-volume contracts for a flying one-level native hex.

use super::*;
use crate::EncounterTuning;
use hex_core::HexCoord;

const FACE: f32 = hex_core::config::HEX_SMALL_DIAMETER * 0.5;

fn creature(species: Species, feet: Vec3) -> Actor {
    let mut actor = Actor::spawn(1, feet, Vec3::X);
    actor.configure_species(species, &EncounterTuning::default());
    actor
}

#[test]
fn supporting_footprint_is_exactly_one_cell_even_after_translation_and_aim_change() {
    let geometry = ArenaVoxelGeometry::default();
    for coord in [HexCoord::ORIGIN, HexCoord::from_axial(6, -4)] {
        let mut actor = creature(Species::Wisp, coord.to_world(-SKIN * 3.0));
        actor.aim = Vec3::new(1.0, 2.0, 3.0).normalize();
        turn(&CollisionWorld::default(), &mut actor, 1.3, 2.0);
        let footprint: Vec<_> = coord
            .within_radius(2)
            .into_iter()
            .filter(|coord| voxel_overlap(TilePos::new(*coord, 0), geometry, &actor))
            .collect();
        assert_eq!(footprint, vec![coord]);
        assert!(actor.body_yaw.abs() < SKIN);
    }
}

#[test]
fn side_top_bottom_and_empty_corner_queries_share_the_short_prism() {
    let feet = Vec3::new(2.0, 4.0, -3.0);
    let actor = creature(Species::Wisp, feet);
    for radius in [0.0, 0.06] {
        let side = sweep_actor(
            feet + Vec3::new(-3.0, 0.2, 0.0),
            Vec3::X * 6.0,
            &actor,
            true,
            radius,
        )
        .expect("actual native side");
        assert!((side.fraction - (3.0 - FACE - radius) / 6.0).abs() < SKIN);
        assert!(side.normal.distance(Vec3::NEG_X) < SKIN);
        let top = sweep_actor(
            feet + Vec3::Y * 2.0,
            Vec3::NEG_Y * 3.0,
            &actor,
            true,
            radius,
        )
        .expect("one-level top");
        assert!((top.fraction - (1.6 - radius) / 3.0).abs() < SKIN);
        let bottom =
            sweep_actor(feet - Vec3::Y, Vec3::Y * 3.0, &actor, true, radius).expect("bottom face");
        assert!((bottom.fraction - (1.0 - radius) / 3.0).abs() < SKIN);
        let corner = feet + Vec3::new(0.8, 2.0, 0.9);
        assert!(sweep_actor(corner, Vec3::NEG_Y * 3.0, &actor, true, radius).is_none());
    }
    assert!((distance(feet + Vec3::Y * 0.6, &actor) - 0.2).abs() < SKIN);
    assert!(distance(feet + Vec3::new(0.8, 0.2, 0.9), &actor) > 0.2);
}

#[test]
fn moving_body_sweep_keeps_actual_thin_height_and_fixed_orientation() {
    let mut actor = creature(Species::Wisp, Vec3::Y * 5.0);
    actor.previous_feet = Vec3::Y * 3.0;
    actor.previous_yaw = 1.7;
    actor.body_yaw = -0.8;
    let hit = sweep_actor(Vec3::Y * 4.0, Vec3::ZERO, &actor, false, 0.06)
        .expect("the translated top crosses this stationary projectile");
    assert!((hit.fraction - 0.27).abs() < SKIN);
    assert!(hit.normal.distance(Vec3::Y) < SKIN);
    assert!(sweep_actor(Vec3::Y * 4.0, Vec3::ZERO, &actor, true, 0.06).is_none());
}

#[test]
fn finite_cones_reach_the_real_top_and_do_not_fill_an_empty_hex_corner() {
    let actor = creature(Species::Wisp, Vec3::Y * 4.0);
    let origin = Vec3::Y * 4.8;
    assert!(exposed_cone_contact(&actor, origin, Vec3::NEG_Y, 0.39, 0.02, |_| true).is_none());
    let contact = exposed_cone_contact(&actor, origin, Vec3::NEG_Y, 0.41, 0.02, |_| true)
        .expect("exposed one-level top inside finite reach");
    assert!((contact.y - 4.4).abs() < SKIN * 4.0);
    assert!(distance(contact, &actor) < SKIN * 4.0);
    assert!(exposed_cone_contact(
        &actor,
        Vec3::new(0.8, 4.8, 0.9),
        Vec3::NEG_Y,
        0.5,
        0.01,
        |_| true
    )
    .is_none());
    assert!(exposed_cone_contact(&actor, origin, Vec3::NEG_Y, 0.41, 0.02, |_| false).is_none());
}

#[test]
fn mixed_contacts_respect_layers_and_clear_the_complete_other_body() {
    let wisp = creature(Species::Wisp, Vec3::Y * 4.0);
    let above = creature(Species::Wisp, Vec3::Y * 4.8);
    assert!(compound_separation(&wisp, &above).is_none());
    for species in [
        Species::Wisp,
        Species::Golem,
        Species::Dragon,
        Species::Shadow,
    ] {
        let mut a = wisp.clone();
        let mut b = creature(species, Vec3::new(0.1, 4.0, 0.0));
        let push = compound_separation(&a, &b).expect("actual mixed body overlap");
        a.feet += push;
        b.feet -= push;
        assert!(
            compound_separation(&a, &b).is_none(),
            "unresolved {species:?}"
        );
    }
    for coord in HexCoord::ORIGIN.neighbors() {
        let adjacent = creature(Species::Wisp, coord.to_world(4.0));
        assert!(compound_separation(&wisp, &adjacent).is_none());
    }
}

#[test]
fn liquid_and_playable_hull_queries_use_the_complete_single_hex() {
    let actor = creature(Species::Wisp, Vec3::Y * 4.0);
    assert!(!hex_span_overlap(&actor, HexCoord::ORIGIN, 0.0, 3.9));
    assert!(hex_span_overlap(&actor, HexCoord::ORIGIN, 4.2, 4.6));
    assert!(!hex_span_overlap(
        &actor,
        HexCoord::from_axial(1, 0),
        4.0,
        4.4
    ));
    let geometry = ArenaVoxelGeometry::default();
    let mut at_edge = actor.clone();
    at_edge.feet.z = 17.0;
    assert!(compound_contained(&at_edge, geometry));
    at_edge.feet.z += 0.01;
    assert!(!compound_contained(&at_edge, geometry));
}

#[test]
fn translated_native_neighbors_are_tangent_but_real_penetration_still_separates() {
    for center in [
        HexCoord::from_axial(-6, 0),
        HexCoord::from_axial(6, 0),
        HexCoord::from_axial(-4, 2),
        HexCoord::from_axial(-2, -2),
        HexCoord::from_axial(-31, 16),
    ] {
        for layer in [4.0, 4.8] {
            let a = creature(Species::Wisp, center.to_world(layer));
            for coord in center.neighbors() {
                let mut b = creature(Species::Wisp, coord.to_world(layer));
                assert!(
                    compound_separation(&a, &b).is_none(),
                    "{center:?} to {coord:?}"
                );
                assert!(compound_separation(&b, &a).is_none(), "reverse tangency");
                let toward = (a.feet - b.feet).normalize();
                b.feet += toward * (SKIN * 8.0);
                let push = compound_separation(&a, &b).expect("finite real penetration");
                assert!(push.length() > SKIN);
                let mut separated = a.clone();
                separated.feet += push;
                b.feet -= push;
                assert!(compound_separation(&separated, &b).is_none());
            }
        }
    }
}
