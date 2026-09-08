//! Continuous translation has the same exact native boundaries as static overlap.
use super::*;
use crate::collision::SKIN;

#[test]
fn swept_native_faces_admit_tangency_but_keep_real_small_penetration() {
    let a = HexPrism::new(Vec3::new(-10.392_304, 0.4, 0.0), 0.4).expect("prism");
    let tangent = HexPrism::new(a.feet + Vec3::X * (2.0 * FACE), 0.4).expect("neighbor");
    assert!(!a.swept_overlaps_prism(Vec3::ZERO, tangent, SKIN));
    assert!(a.swept_overlaps_prism(Vec3::X * SKIN * 8.0, tangent, SKIN));
    assert!(!a.swept_overlaps_prism(Vec3::NEG_X * 0.2, tangent, SKIN));
}

#[test]
fn capsule_lower_cap_keeps_its_rounded_cross_section_during_translation() {
    let prism = HexPrism::new(Vec3::Y * 0.7, 0.4).expect("prism");
    let clear = Vec3::new(FACE + 0.23, 1.0, 0.0);
    let overlap = Vec3::new(FACE + 0.18, 1.0, 0.0);
    assert!(prism.overlap_capsule(clear, 0.8, 0.25, SKIN).is_none());
    assert!(!prism.swept_overlaps_capsule(Vec3::Z * 0.1, clear, 0.8, 0.25, SKIN));
    assert!(prism.overlap_capsule(overlap, 0.8, 0.25, SKIN).is_some());
    assert!(prism.swept_overlaps_capsule(Vec3::Z * 0.1, overlap, 0.8, 0.25, SKIN));
}

#[test]
fn empty_hex_bounding_corner_does_not_intercept_a_small_box() {
    let prism = HexPrism::new(Vec3::ZERO, 0.4).expect("prism");
    let feet = Vec3::new(0.78, 0.0, 0.9);
    let size = Vec3::new(0.1, 0.3, 0.1);
    assert!(prism.overlap_box(feet, size, 0.3, SKIN).is_none());
    assert!(!prism.swept_overlaps_box(Vec3::X * 0.02, feet, size, 0.3, SKIN));
}

#[test]
fn continuous_capsule_and_box_sweeps_retain_oblique_static_contacts() {
    let size = Vec3::new(1.1, 0.5, 2.7);
    let obstacle = Vec3::new(0.2, 0.6, -0.4);
    for start in [Vec3::new(-3.0, 0.5, -1.0), Vec3::new(1.0, 0.8, -3.0)] {
        for delta in [Vec3::new(4.0, 0.0, 2.0), Vec3::new(-2.0, -0.3, 4.0)] {
            let part = HexPrism::new(start, 0.4).expect("part");
            for yaw in [0.0, 0.7, 1.9] {
                let samples_box = (0_u16..=256).any(|step| {
                    HexPrism::new(start + delta * (f32::from(step) / 256.0), 0.4)
                        .expect("sample")
                        .overlap_box(obstacle, size, yaw, SKIN)
                        .is_some()
                });
                assert!(!samples_box || part.swept_overlaps_box(delta, obstacle, size, yaw, SKIN));
            }
            let samples_capsule = (0_u16..=256).any(|step| {
                HexPrism::new(start + delta * (f32::from(step) / 256.0), 0.4)
                    .expect("sample")
                    .overlap_capsule(obstacle, 0.8, 0.25, SKIN * 2.0)
                    .is_some()
            });
            assert!(
                !samples_capsule || part.swept_overlaps_capsule(delta, obstacle, 0.8, 0.25, SKIN)
            );
        }
    }
}
