//! Mathematical acceptance fixtures for the fixed-prism kernel.

use super::*;
use hex_core::HexCoord;

const SKIN: f32 = 0.0001;

fn prism() -> HexPrism {
    HexPrism::new(Vec3::ZERO, 2.0).expect("valid five-level native prism")
}

fn close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < 0.00001,
        "actual={actual}, expected={expected}"
    );
}

fn moved(body: HexPrism, contact: HorizontalContact) -> HexPrism {
    HexPrism::new(
        body.feet + contact.normal * (contact.depth + SKIN),
        body.height,
    )
    .expect("finite separating move")
}

#[test]
fn closest_point_uses_hex_faces_and_caps_instead_of_an_aabb() {
    let body = prism();
    let inside = Vec3::new(0.1, 1.0, 0.1);
    assert_eq!(
        body.closest_point(inside).to_array().map(f32::to_bits),
        inside.to_array().map(f32::to_bits)
    );
    assert!(
        body.closest_point(Vec3::new(2.0, 3.0, 0.0))
            .distance(Vec3::new(FACE, 2.0, 0.0))
            < SKIN
    );
    assert!(
        body.closest_point(Vec3::new(0.0, 2.6, 1.3))
            .distance(Vec3::new(0.0, 2.0, 1.0))
            < SKIN
    );
    let outside = Vec3::new(0.8, 1.0, 0.8);
    let axis = Vec3::new(0.5, 0.0, FACE);
    let expected = outside - axis * (outside.dot(axis) - FACE);
    assert!(body.closest_point(outside).distance(expected) < SKIN);
    assert!(
        body.distance(outside) > 0.22,
        "the AABB corner is not solid"
    );
}

#[test]
fn point_sweep_has_exact_faces_starting_overlap_and_tangent_conventions() {
    let body = prism();
    let hit = body
        .sweep_point(Vec3::new(-3.0, 1.0, 0.0), Vec3::X * 6.0)
        .expect("side face");
    close(hit.fraction, (3.0 - FACE) / 6.0);
    assert!(hit.normal.distance(Vec3::NEG_X) < SKIN);
    let cap = body
        .sweep_point(Vec3::Y * 3.0, Vec3::NEG_Y * 2.0)
        .expect("top cap");
    close(cap.fraction, 0.5);
    assert!(cap.normal.distance(Vec3::Y) < SKIN);
    let inside = body
        .sweep_point(Vec3::Y, Vec3::ZERO)
        .expect("closed initial overlap");
    close(inside.fraction, 0.0);
    assert!(inside.normal.length_squared() < SKIN * SKIN);
    assert!(body.sweep_point(Vec3::X * 3.0, Vec3::ZERO).is_none());
    let touching = body
        .sweep_point(Vec3::new(FACE, 1.0, 0.0), Vec3::X)
        .expect("closed boundary");
    close(touching.fraction, 0.0);
}

#[test]
fn sphere_sweep_face_and_cap_offsets_equal_the_actual_radius() {
    let body = prism();
    let face = body
        .sweep_sphere(Vec3::new(-3.0, 1.0, 0.0), Vec3::X * 6.0, 0.2)
        .expect("face");
    close(face.fraction, (3.0 - FACE - 0.2) / 6.0);
    assert!(face.normal.distance(Vec3::NEG_X) < SKIN);
    let cap = body
        .sweep_sphere(Vec3::Y * 3.0, Vec3::NEG_Y * 2.0, 0.2)
        .expect("cap");
    close(cap.fraction, 0.4);
    assert!(cap.normal.distance(Vec3::Y) < SKIN);
    let initial = body
        .sweep_sphere(Vec3::new(0.0, 1.0, 1.1), Vec3::Z, 0.2)
        .expect("overlap");
    close(initial.fraction, 0.0);
    assert!(initial.normal.distance(Vec3::Z) < SKIN);
}

#[test]
fn rounded_vertical_edge_does_not_gain_the_inflated_planes_sharp_corner() {
    let body = prism();
    let false_corner = Vec3::new(0.0, 0.25, 1.22);
    assert!(body
        .planes()
        .all(|(n, bound)| false_corner.dot(n) <= bound + 0.2));
    assert!(body.distance(false_corner) > 0.2);
    assert!(body
        .sweep_sphere(false_corner, Vec3::Y * 1.5, 0.2)
        .is_none());
    let edge = body
        .sweep_sphere(Vec3::new(0.0, 1.0, 2.0), Vec3::NEG_Z * 2.0, 0.2)
        .expect("round edge");
    close(edge.fraction, 0.4);
    assert!(edge.normal.distance(Vec3::Z) < SKIN);
}

#[test]
fn horizontal_edge_and_cap_vertex_contacts_use_euclidean_rounding() {
    let body = prism();
    let start = Vec3::new(2.0, 3.0, 0.0);
    let delta = Vec3::new(-2.0, -2.0, 0.0);
    let edge = body
        .sweep_sphere(start, delta, 0.25)
        .expect("side/top edge");
    close(body.distance(start + delta * edge.fraction), 0.25);
    assert!(edge.normal.x > 0.1 && edge.normal.y > 0.1);
    assert!(body.distance(start + delta * (edge.fraction - SKIN)) > 0.25);
    let vertex = body
        .sweep_sphere(Vec3::new(0.0, 3.0, 2.0), Vec3::new(0.0, -2.0, -2.0), 0.2)
        .expect("cap vertex");
    close(vertex.fraction, (1.0 - 0.2 / 2.0_f32.sqrt()) / 2.0);
    assert!(vertex.normal.distance(Vec3::new(0.0, 1.0, 1.0).normalize()) < SKIN);
}

#[test]
fn long_tangent_sweep_keeps_the_small_radius_and_rejects_a_real_near_miss() {
    let body = prism();
    let start = Vec3::new(-80.0, 1.0, 1.125);
    let tangent = body
        .sweep_sphere(start, Vec3::X * 160.0, 0.125)
        .expect("exact binary-fraction tangent");
    close(tangent.fraction, 0.5);
    assert!(tangent.normal.distance(Vec3::Z) < SKIN);
    assert!(body
        .sweep_sphere(start + Vec3::Z * 0.001, Vec3::X * 160.0, 0.125)
        .is_none());
}

#[test]
fn hex_contacts_separate_actual_faces_and_leave_exact_tangency_free() {
    let body = prism();
    let touching = HexPrism::new(Vec3::X * (2.0 * FACE), 2.0).expect("neighbor");
    assert!(body.overlap_prism(touching, SKIN).is_none());
    let overlapping = HexPrism::new(Vec3::X * (2.0 * FACE - 0.05), 2.0).expect("overlap");
    let contact = body
        .overlap_prism(overlapping, SKIN)
        .expect("positive overlap");
    close(contact.depth, 0.05);
    assert!(contact.normal.distance(Vec3::NEG_X) < SKIN);
    assert!(moved(body, contact)
        .overlap_prism(overlapping, SKIN)
        .is_none());
    let above = HexPrism::new(Vec3::Y * 2.0, 2.0).expect("stacked neighbor");
    assert!(body.overlap_prism(above, SKIN).is_none());
}

#[test]
fn offset_world_voxels_and_seven_hex_deployment_do_not_require_an_extra_ring() {
    let offset = ArenaVoxelGeometry {
        vertical_offset: 0.4,
        ..Default::default()
    };
    let voxel = TilePos::new(HexCoord::ORIGIN, 2);
    let body = HexPrism::new(Vec3::Y * 0.8, 0.4).expect("one layer");
    assert!(body.overlaps_voxel(voxel, offset, SKIN));
    assert!(!body.overlaps_voxel(voxel.above(), offset, SKIN));
    let geometry = ArenaVoxelGeometry::default();
    let centers = HexCoord::ORIGIN.within_radius(1);
    let prisms: Vec<_> = centers
        .iter()
        .map(|coord| {
            HexPrism::new(coord.to_world(-SKIN * 3.0), 2.0).expect("lowered Golem support probe")
        })
        .collect();
    let overlaps: Vec<_> = HexCoord::ORIGIN
        .within_radius(3)
        .into_iter()
        .filter(|coord| {
            prisms
                .iter()
                .any(|prism| prism.overlaps_voxel(TilePos::new(*coord, 0), geometry, SKIN))
        })
        .collect();
    assert_eq!(overlaps.len(), 7);
    assert!(centers.iter().all(|coord| overlaps.contains(coord)));
}

#[test]
fn capsule_endcap_uses_its_smaller_section_and_inside_centers_separate() {
    let body = prism();
    // The capsule axis starts at 2.15, above this prism's top. At the prism's
    // highest plane the spherical cap radius is sqrt(.25^2-.15^2) = .2.
    assert!(body
        .overlap_capsule(Vec3::new(FACE + 0.21, 1.9, 0.0), 0.8, 0.25, SKIN)
        .is_none());
    let feet = Vec3::new(FACE + 0.19, 1.9, 0.0);
    let contact = body
        .overlap_capsule(feet, 0.8, 0.25, SKIN)
        .expect("cap clips face");
    close(contact.depth, 0.01);
    assert!(contact.normal.distance(Vec3::NEG_X) < SKIN);
    assert!(moved(body, contact)
        .overlap_capsule(feet, 0.8, 0.25, SKIN)
        .is_none());
    let inside = body
        .overlap_capsule(Vec3::Y * 0.4, 0.8, 0.25, SKIN)
        .expect("interior circle");
    assert!(inside.normal.is_finite() && inside.normal.length() > 0.99);
    close(inside.depth, FACE + 0.25);
    assert!(moved(body, inside)
        .overlap_capsule(Vec3::Y * 0.4, 0.8, 0.25, SKIN)
        .is_none());
}

#[test]
fn box_contacts_use_dragon_orientation_and_reject_phantom_aabb_corners() {
    let body = prism();
    let feet = Vec3::new(0.0, 0.0, 2.3);
    let size = Vec3::new(1.0, 0.4, 3.5);
    let contact = body
        .overlap_box(feet, size, 0.0, SKIN)
        .expect("long forward body overlaps");
    assert!(moved(body, contact)
        .overlap_box(feet, size, 0.0, SKIN)
        .is_none());
    assert!(body
        .overlap_box(feet, size, std::f32::consts::FRAC_PI_2, SKIN)
        .is_none());
    assert!(body
        .overlap_box(
            Vec3::new(0.82, 0.0, 0.91),
            Vec3::new(0.1, 0.4, 0.1),
            0.0,
            SKIN
        )
        .is_none());
}

#[test]
fn invalid_shapes_and_sweep_inputs_fail_without_a_capsule_fallback() {
    assert!(HexPrism::new(Vec3::ZERO, 0.0).is_none());
    assert!(HexPrism::new(Vec3::NAN, 2.0).is_none());
    assert!(prism().sweep_sphere(Vec3::NAN, Vec3::X, 0.2).is_none());
    assert!(prism().sweep_sphere(Vec3::ZERO, Vec3::X, -0.2).is_none());
    assert!(prism()
        .overlap_capsule(Vec3::ZERO, 2.0, 2.598, SKIN)
        .is_none());
}

fn seven_prisms() -> [HexPrism; 7] {
    [
        Vec3::ZERO,
        Vec3::new(2.0 * FACE, 0.0, 0.0),
        Vec3::new(FACE, 0.0, 1.5),
        Vec3::new(-FACE, 0.0, 1.5),
        Vec3::new(-2.0 * FACE, 0.0, 0.0),
        Vec3::new(-FACE, 0.0, -1.5),
        Vec3::new(FACE, 0.0, -1.5),
    ]
    .map(|feet| HexPrism::new(feet, 2.0).expect("native Golem component"))
}

#[test]
fn union_mouth_exit_follows_axis_and_diagonal_faces_including_the_notch() {
    let eye = Vec3::Y * 1.4;
    for (direction, expected) in [
        (Vec3::X, 3.0 * FACE),
        (Vec3::Z, 2.0),
        (Vec3::new(0.5, 0.0, FACE), 3.0 * FACE),
        (Vec3::new(FACE, 0.0, 0.5), 2.0),
    ] {
        for sign in [-1.0, 1.0] {
            let direction = direction * sign;
            let distance = planar_union_exit(seven_prisms(), eye, direction)
                .expect("continuous body interval from the central eye");
            close(distance, expected);
            let inside = eye + direction * (distance - SKIN);
            let outside = eye + direction * (distance + SKIN);
            assert!(seven_prisms()
                .into_iter()
                .any(|part| part.distance(inside) < SKIN * 0.1));
            assert!(seven_prisms()
                .into_iter()
                .all(|part| part.distance(outside) > SKIN * 0.1));
        }
    }
    // The bounding box reaches Z=2.5, but the center ray exits the notch at Z=2.
    let notch = planar_union_exit(seven_prisms(), eye, Vec3::Z).expect("notch");
    assert!(notch < 2.5 - 0.4);
    let planar = planar_union_exit(seven_prisms(), eye, Vec3::new(0.0, 90.0, 8.0))
        .expect("aim is normalized only in the horizontal plane");
    close(planar, notch);
}

#[test]
fn union_exit_is_order_independent_and_never_jumps_a_gap_or_missing_origin() {
    let eye = Vec3::Y * 1.4;
    close(
        planar_union_exit(seven_prisms().into_iter().rev(), eye, Vec3::X)
            .expect("reversed connected components"),
        3.0 * FACE,
    );
    let remote = HexPrism::new(Vec3::X * 5.0, 2.0).expect("disconnected component");
    close(
        planar_union_exit([remote, prism()], eye, Vec3::X).expect("central component only"),
        FACE,
    );
    assert!(planar_union_exit([remote], eye, Vec3::X).is_none());
    assert!(planar_union_exit(seven_prisms(), Vec3::Y * 2.1, Vec3::X).is_none());
    assert!(planar_union_exit(seven_prisms(), eye, Vec3::Y).is_none());
    assert!(planar_union_exit(seven_prisms(), eye, Vec3::NAN).is_none());
    assert!(planar_union_exit(seven_prisms().into_iter().chain([prism()]), eye, Vec3::X).is_none());
}

#[test]
fn translated_shared_edges_remain_closed_for_mouth_exit_and_point_rays() {
    for translation in [
        Vec3::new(2.0, 4.0, 1.0),
        Vec3::new(-17.3, 6.4, -9.1),
        Vec3::new(50.0, 0.4, 30.0),
    ] {
        let parts = seven_prisms().map(|part| {
            HexPrism::new(part.feet + translation, part.height).expect("translated native union")
        });
        let eye = translation + Vec3::Y * 1.4;
        for direction in [Vec3::Z, Vec3::NEG_Z] {
            let exit = planar_union_exit(parts, eye, direction)
                .expect("both translated side prisms remain on the shared edge");
            close(exit, 2.0);
            let start = eye + direction * 3.0;
            let hit = parts
                .into_iter()
                .filter_map(|part| part.sweep_point(start, -direction * 4.0))
                .min_by(|a, b| a.fraction.total_cmp(&b.fraction))
                .expect("zero-radius ray contacts the outer shared edge");
            close(hit.fraction, 0.25);
        }
    }
}
