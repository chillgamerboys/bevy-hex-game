//! Neutral oracle for the pre-optimization ordered SAT calculations.

use super::*;
use hex_core::arena::{ArenaStaticSpan, ArenaTerrainView};
use hex_core::{HexCoord, SubstanceId};

// Keep the accepted Vec implementation intact as the independent arithmetic and
// axis-order oracle. The allocation is intentionally confined to tests.
fn legacy_box_span_axes(span: Span, feet: Vec3, size: Vec3, yaw: f32) -> Vec<(Vec3, f32, f32)> {
    let half = size * 0.5;
    let [right, _, forward] = axes(yaw);
    let center = feet + Vec3::Y * half.y;
    let local = center - span.coord.to_world((span.bottom + span.top) * 0.5);
    [
        Vec3::X,
        Vec3::new(0.5, 0.0, 0.866_025_4),
        Vec3::new(-0.5, 0.0, 0.866_025_4),
        right,
        forward,
        Vec3::Y,
    ]
    .into_iter()
    .map(|axis| {
        let extent = if axis.y > 0.5 {
            (span.top - span.bottom) * 0.5 + half.y
        } else {
            hex_support(axis) + half.x * right.dot(axis).abs() + half.z * forward.dot(axis).abs()
        };
        (axis, local.dot(axis), extent)
    })
    .collect()
}

fn plane_bits(planes: impl IntoIterator<Item = (Vec3, f32, f32)>) -> Vec<([u32; 3], u32, u32)> {
    planes
        .into_iter()
        .map(|(axis, position, extent)| {
            (
                axis.to_array().map(f32::to_bits),
                position.to_bits(),
                extent.to_bits(),
            )
        })
        .collect()
}

fn hit_bits(hit: Option<Hit>) -> Option<(u32, [u32; 3])> {
    hit.map(|hit| {
        (
            hit.fraction.to_bits(),
            hit.normal.to_array().map(f32::to_bits),
        )
    })
}

#[test]
fn fixed_planes_preserve_every_axis_and_scalar_bit_of_the_accepted_vec() {
    for coord in [
        HexCoord::ORIGIN,
        HexCoord::from_axial(-6, 2),
        HexCoord::from_axial(27, -19),
    ] {
        for (bottom, top) in [(-0.4, 0.0), (3.2, 3.6), (6.4, 9.6), (48.0, 51.2)] {
            let span = Span { coord, bottom, top };
            for size in [Vec3::new(1.732_050_8, 0.4, 6.0), Vec3::new(2.3, 0.4, 6.7)] {
                for offset in [
                    Vec3::ZERO,
                    Vec3::new(-0.0, 0.4, 0.0),
                    Vec3::new(0.3, -0.2, 2.7),
                ] {
                    let feet = coord.to_world(bottom) + offset;
                    for yaw in [
                        0.0,
                        -0.0,
                        std::f32::consts::FRAC_PI_6,
                        std::f32::consts::FRAC_PI_2,
                        1.3,
                        -2.4,
                    ] {
                        let actual: [(Vec3, f32, f32); 6] = box_span_axes(span, feet, size, yaw);
                        let expected = legacy_box_span_axes(span, feet, size, yaw);
                        assert_eq!(plane_bits(actual), plane_bits(expected.iter().copied()));
                        for delta in [
                            Vec3::ZERO,
                            Vec3::X * 4.0,
                            Vec3::NEG_Z * 7.0,
                            Vec3::new(0.8, -0.6, 0.7),
                        ] {
                            for inside_hit in [false, true] {
                                assert_eq!(
                                    hit_bits(sweep_axes(actual, delta, inside_hit)),
                                    hit_bits(sweep_axes(
                                        expected.iter().copied(),
                                        delta,
                                        inside_hit
                                    ))
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

fn fixture() -> CollisionWorld {
    let mut view = ArenaTerrainView::default();
    for coord in HexCoord::ORIGIN.within_radius(5) {
        view.voxels.insert(TilePos::new(coord, 0), SubstanceId(1));
    }
    for level in 1..=10 {
        view.voxels.insert(
            TilePos::new(HexCoord::from_axial(2, 0), level),
            SubstanceId(1),
        );
    }
    for coord in HexCoord::from_axial(0, 2).within_radius(1) {
        view.voxels.insert(TilePos::new(coord, 3), SubstanceId(1));
    }
    view.static_spans.push(ArenaStaticSpan {
        bottom: TilePos::new(HexCoord::from_axial(-2, 0), 1),
        top_level: 4,
        blocks_movement: true,
        blocks_projectiles: true,
        blocks_sight: true,
    });
    let mut world = CollisionWorld::default();
    world.refresh(&view, ArenaVoxelGeometry::default());
    world
}

#[test]
fn dragon_clear_and_sweep_keep_legacy_hits_across_floor_cover_roof_and_ties() {
    let world = fixture();
    let mut actor = Actor::spawn(1, Vec3::ZERO, Vec3::Z);
    actor.configure_species(Species::Dragon, &crate::EncounterTuning::default());
    let mut hits = 0;
    let mut clears = 0;
    let mut overlaps = 0;
    for feet in [
        Vec3::ZERO,
        Vec3::new(-6.0, SKIN, -2.0),
        Vec3::new(0.0, 0.5, 3.0),
        Vec3::new(5.0, 2.0, 0.0),
    ] {
        for yaw in [
            0.0,
            -0.0,
            std::f32::consts::FRAC_PI_6,
            std::f32::consts::FRAC_PI_2,
            -2.4,
        ] {
            actor.feet = feet;
            actor.body_yaw = yaw;
            let radius = (actor.dimensions.x * actor.dimensions.x
                + actor.dimensions.z * actor.dimensions.z)
                .sqrt()
                * 0.5;
            let expected_clear = !world.candidates(feet, feet, radius).any(|span| {
                legacy_box_span_axes(span, feet, actor.dimensions, yaw)
                    .into_iter()
                    .all(|(_, position, extent)| position.abs() < extent - SKIN)
            });
            assert_eq!(clear(&world, &actor, feet, yaw), expected_clear);
            if expected_clear {
                clears += 1;
            } else {
                overlaps += 1;
            }
            for delta in [
                Vec3::ZERO,
                Vec3::X * 8.0,
                Vec3::NEG_Z * 8.0,
                Vec3::NEG_Y * 3.0,
                Vec3::new(4.0, 2.0, 4.0),
            ] {
                let expected = world
                    .candidates(feet, feet + delta, radius)
                    .filter_map(|span| {
                        sweep_axes(
                            legacy_box_span_axes(span, feet, actor.dimensions, yaw),
                            delta,
                            false,
                        )
                    })
                    .min_by(|a, b| a.fraction.total_cmp(&b.fraction));
                assert_eq!(
                    hit_bits(sweep(&world, &actor, feet, delta)),
                    hit_bits(expected)
                );
                hits += usize::from(expected.is_some());
            }
        }
    }
    assert!(
        hits > 0 && clears > 0 && overlaps > 0,
        "exercise contact and no-contact branches"
    );
}
