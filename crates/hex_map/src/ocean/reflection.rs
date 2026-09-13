use super::{OceanBathymetry, OceanSurfaceProfile};
use bevy::prelude::*;

pub(super) const REFLECTION_RANGE: f32 = 35.0;

#[derive(Default)]
pub(super) struct Reflection {
    pub weight: f32,
    gradient: Vec2,
    anchor: Vec2,
    anchor_dx: Vec2,
    anchor_dz: Vec2,
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "Sampling uses finite positions within a validated bounded grid."
)]
pub(super) fn reflection(
    profile: &OceanSurfaceProfile,
    bed: &OceanBathymetry,
    at: Vec2,
) -> Option<Reflection> {
    if bed.shore_anchors.is_empty() || !bed.contains(at) || profile.shore_reflection <= 0.0 {
        return Some(Reflection::default());
    }
    let grid = (at - bed.origin_xz) / bed.spacing;
    let x = (grid.x.floor() as u32).min(bed.width - 2);
    let z = (grid.y.floor() as u32).min(bed.height - 2);
    let t = grid - Vec2::new(x as f32, z as f32);
    let read = |x: u32, z: u32| {
        bed.shore_anchors
            .get((z * bed.width + x) as usize)
            .copied()
            .filter(|p| p.is_finite())
    };
    let (a, b, c, d) = (
        read(x, z)?,
        read(x + 1, z)?,
        read(x, z + 1)?,
        read(x + 1, z + 1)?,
    );
    let row0 = a.lerp(b, t.x);
    let row1 = c.lerp(d, t.x);
    let anchor = row0.lerp(row1, t.y);
    let anchor_dx = (b - a).lerp(d - c, t.y) / bed.spacing;
    let anchor_dz = (row1 - row0) / bed.spacing;
    let (distance, distance_gradient) = shoreline_distance(at, [a, b, c, d], t, bed.spacing);
    let t = ((distance - 2.0) / (REFLECTION_RANGE - 2.0)).clamp(0.0, 1.0);
    let weight = profile.shore_reflection * (1.0 - t * t * (3.0 - 2.0 * t));
    Some(Reflection {
        weight,
        gradient: -profile.shore_reflection
            * (6.0 * t * (1.0 - t) / (REFLECTION_RANGE - 2.0))
            * distance_gradient,
        anchor,
        anchor_dx,
        anchor_dz,
    })
}

/// Blend distances to real shoreline points, never to their blended position.
/// Opposite shores otherwise invent a nearby anchor in the middle of deep sea.
/// Each distance bounds the nearest real shore from above, as does their convex
/// blend. The gradient includes both distance and interpolation-weight changes.
fn shoreline_distance(at: Vec2, anchors: [Vec2; 4], t: Vec2, spacing: f32) -> (f32, Vec2) {
    let values = anchors.map(|anchor| {
        let offset = at - anchor;
        let distance = offset.length();
        let gradient = offset / distance.max(0.00001);
        Vec3::new(distance, gradient.x, gradient.y)
    });
    let [a, b, c, d] = values;
    let row0 = a.lerp(b, t.x);
    let row1 = c.lerp(d, t.x);
    let value = row0.lerp(row1, t.y);
    let gradient = Vec2::new(
        value.y + ((b.x - a.x) * (1.0 - t.y) + (d.x - c.x) * t.y) / spacing,
        value.z + (row1.x - row0.x) / spacing,
    );
    (value.x, gradient)
}

impl Reflection {
    #[expect(
        clippy::too_many_arguments,
        reason = "The CPU/GPU analytic wave contract passes the same scalar wave uniforms explicitly."
    )]
    pub fn blend(
        &self,
        incident: Vec4,
        direction: Vec2,
        amplitude: f32,
        frequency: f32,
        rate: f32,
        offset: f32,
        at: Vec2,
        seconds: f32,
    ) -> Vec4 {
        if self.weight <= 0.0 {
            return incident;
        }
        let phase = -frequency * direction.dot(at) + 2.0 * frequency * direction.dot(self.anchor)
            - rate * seconds
            + offset;
        let gradient = -frequency * direction
            + 2.0
                * frequency
                * Vec2::new(direction.dot(self.anchor_dx), direction.dot(self.anchor_dz));
        let reflected = Vec4::new(
            amplitude * phase.sin(),
            amplitude * phase.cos() * gradient.x,
            amplitude * phase.cos() * gradient.y,
            -rate * amplitude * phase.cos(),
        );
        let denominator = 1.0 + self.weight;
        let mut result = (incident + self.weight * reflected) / denominator;
        let correction = self.gradient * ((reflected.x - incident.x) / (denominator * denominator));
        result.y += correction.x;
        result.z += correction.y;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocean::sample_surface;

    fn coast() -> OceanBathymetry {
        OceanBathymetry {
            spacing: 50.0,
            bed_heights: vec![-20.0; 4],
            shore_shelter: vec![0.4, 0.8, 0.6, 1.0],
            shore_anchors: vec![
                Vec2::ZERO,
                Vec2::ZERO,
                Vec2::new(0.0, 50.0),
                Vec2::new(0.0, 50.0),
            ],
            ..default()
        }
    }

    #[test]
    fn disconnected_coasts_do_not_reflect_in_the_deep_channel_between_them() {
        let bed = OceanBathymetry {
            origin_xz: Vec2::splat(-2.0),
            spacing: 4.0,
            bed_heights: vec![-40.0; 4],
            shore_anchors: vec![
                Vec2::new(-60.0, 0.0),
                Vec2::new(60.0, 0.0),
                Vec2::new(-60.0, 0.0),
                Vec2::new(60.0, 0.0),
            ],
            ..default()
        };
        let profile = OceanSurfaceProfile::default();
        for x in [-1.5, 0.0, 1.5] {
            assert_eq!(
                reflection(&profile, &bed, Vec2::new(x, 0.0))
                    .unwrap()
                    .weight,
                0.0
            );
        }
    }

    #[test]
    fn settlement_offshore_zigzag_probes_have_no_reflection() {
        // Actual package02 four-unit cells along the visible offshore artifact.
        // Every cached anchor is real land over35 units from its query point.
        let probes = [
            (
                [167.9, 633.2],
                [164.0, 632.0],
                [
                    [144.0, 576.0],
                    [144.0, 576.0],
                    [164.0, 700.0],
                    [168.0, 700.0],
                ],
            ),
            (
                [131.2, 644.8],
                [128.0, 644.0],
                [
                    [96.0, 596.0],
                    [148.0, 700.0],
                    [148.0, 700.0],
                    [148.0, 700.0],
                ],
            ),
            (
                [113.5, 653.8],
                [112.0, 652.0],
                [
                    [72.0, 608.0],
                    [148.0, 700.0],
                    [148.0, 700.0],
                    [148.0, 700.0],
                ],
            ),
            (
                [98.6, 662.5],
                [96.0, 660.0],
                [[56.0, 616.0], [56.0, 616.0], [56.0, 616.0], [144.0, 704.0]],
            ),
            (
                [91.4, 668.7],
                [88.0, 668.0],
                [[48.0, 620.0], [48.0, 620.0], [40.0, 624.0], [144.0, 704.0]],
            ),
            (
                [84.5, 674.8],
                [84.0, 672.0],
                [[40.0, 624.0], [40.0, 624.0], [40.0, 624.0], [144.0, 704.0]],
            ),
        ];
        let profile = OceanSurfaceProfile::default();
        for (at, origin, anchors) in probes {
            let at = Vec2::from_array(at);
            let anchors = anchors.map(Vec2::from_array);
            assert!(anchors
                .iter()
                .all(|anchor| at.distance(*anchor) > REFLECTION_RANGE));
            let bed = OceanBathymetry {
                origin_xz: Vec2::from_array(origin),
                spacing: 4.0,
                bed_heights: vec![-40.0; 4],
                shore_anchors: anchors.to_vec(),
                ..default()
            };
            assert_eq!(reflection(&profile, &bed, at).unwrap().weight, 0.0);
        }
    }
    #[test]
    fn reflected_normal_and_vertical_velocity_match_finite_derivatives() {
        let profile = OceanSurfaceProfile::default();
        let bed = coast();
        let at = Vec2::new(12.0, 15.0);
        let seconds = 7.0;
        let value = sample_surface(&profile, &bed, at, seconds).unwrap();
        let height = |at, seconds| sample_surface(&profile, &bed, at, seconds).unwrap().height;
        let dx =
            (height(at + Vec2::X * 0.01, seconds) - height(at - Vec2::X * 0.01, seconds)) / 0.02;
        let dz =
            (height(at + Vec2::Y * 0.01, seconds) - height(at - Vec2::Y * 0.01, seconds)) / 0.02;
        let dt = (height(at, seconds + 0.01) - height(at, seconds - 0.01)) / 0.02;
        assert!((dx + value.normal.x / value.normal.y).abs() < 0.0001);
        assert!((dz + value.normal.z / value.normal.y).abs() < 0.0001);
        assert!((dt - value.vertical_velocity).abs() < 0.0001);
    }
    #[test]
    fn shore_interference_preserves_swell_envelope_and_wraps_without_a_jump() {
        let profile = OceanSurfaceProfile::default();
        let bed = coast();
        for seconds in 0_u16..900 {
            for at in [
                Vec2::new(3.0, 10.0),
                Vec2::new(18.0, 30.0),
                Vec2::new(40.0, 20.0),
            ] {
                assert!(
                    sample_surface(&profile, &bed, at, f32::from(seconds))
                        .unwrap()
                        .height
                        .abs()
                        <= 2.00001
                );
            }
        }
        let a = sample_surface(&profile, &bed, Vec2::new(12.0, 15.0), 7.0).unwrap();
        let b = sample_surface(&profile, &bed, Vec2::new(12.0, 15.0), 907.0).unwrap();
        assert!((a.height - b.height).abs() < 0.0001);
        assert!((a.vertical_velocity - b.vertical_velocity).abs() < 0.0001);
        assert!(
            reflection(&profile, &bed, Vec2::new(40.0, 20.0))
                .unwrap()
                .weight
                .abs()
                < 0.00001
        );
    }
}
