//! Bounded kinematic wind, with shelter from an immutable world publication.
use std::collections::HashMap;

use bevy::prelude::*;
use hex_core::{
    arena::{ArenaTerrainView, ArenaVoxelGeometry},
    ocean::{OceanSimulationTime, OceanWindProfile},
    HexCoord, TilePos,
};

use super::SEA_LEVEL;

/// Immutable natural wind and published solid cover, shared by exploration maps.
#[derive(Debug, Clone)]
pub struct WindField {
    sea_level: f32,
    spans: HashMap<HexCoord, Vec<Vec2>>,
}

impl Default for WindField {
    fn default() -> Self {
        Self {
            sea_level: SEA_LEVEL,
            spans: HashMap::new(),
        }
    }
}

fn smooth(value: f32) -> f32 {
    let x = value.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

// Reduce each oscillator separately in f64, never the shared simulation clock.
#[expect(
    clippy::cast_possible_truncation,
    reason = "Sine is bounded to [-1, 1]."
)]
fn oscillation(seconds: f64, period: f64, phase: f32) -> f32 {
    (seconds.rem_euclid(period) * std::f64::consts::TAU / period + f64::from(phase)).sin() as f32
}

// Curl of a moving stream function: neighboring samples curve around the same
// cells, with alternating clockwise/counterclockwise circulation. Different
// spatial scales and periods keep the combined field from repeating in stripes.
fn curl(at: Vec2, seconds: f64, scale: Vec2, periods: Vec2, phase: f32) -> Vec2 {
    let a = at.x / scale.x + phase;
    let b = at.y / scale.y - phase;
    let quarter = std::f32::consts::FRAC_PI_2;
    let size = scale.min_element();
    Vec2::new(
        oscillation(seconds, f64::from(periods.x), a)
            * oscillation(seconds, f64::from(periods.y), b + quarter)
            * size
            / scale.y,
        -oscillation(seconds, f64::from(periods.x), a + quarter)
            * oscillation(seconds, f64::from(periods.y), b)
            * size
            / scale.x,
    )
}

impl WindField {
    pub(super) fn new(view: &ArenaTerrainView, geometry: ArenaVoxelGeometry) -> Self {
        Self::for_region(view, geometry, SEA_LEVEL, None)
    }

    /// Snapshot cover within 112 units of an optional local center.
    /// This bounds allocations on streamed worlds while covering the 48-unit
    /// upwind probes for the actor and the nearby arrow grid.
    #[must_use]
    pub fn for_region(
        view: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        sea_level: f32,
        center: Option<Vec3>,
    ) -> Self {
        let mut field = Self {
            sea_level,
            ..Self::default()
        };
        let mut insert = |bottom: TilePos, top: i32| {
            if center.is_some_and(|p| {
                bottom.coord.to_world(0.0).distance_squared(p.with_y(0.0)) > 112.0 * 112.0
            }) {
                return;
            }
            field.spans.entry(bottom.coord).or_default().push(Vec2::new(
                geometry.top(bottom) - geometry.level_height,
                geometry.top(TilePos::new(bottom.coord, top)),
            ));
        };
        for span in view
            .columns
            .values()
            .chain(view.object_columns.values())
            .flatten()
        {
            insert(span.bottom, span.top_level);
        }
        for span in &view.static_spans {
            if span.blocks_movement {
                insert(span.bottom, span.top_level);
            }
        }
        // Legacy dense publications have no compact terrain runs.
        if view.columns.is_empty() {
            for pos in view.voxels.keys() {
                insert(*pos, pos.level);
            }
        }
        field
    }

    /// Sample actual environmental wind before any vehicle influence multiplier.
    #[must_use]
    pub fn velocity(&self, p: Vec3, time: OceanSimulationTime, profile: OceanWindProfile) -> Vec2 {
        if !p.is_finite()
            || p.abs().max_element() > 100_000.0
            || !time.seconds.is_finite()
            || !profile.speed.is_finite()
            || !profile.heading_radians.is_finite()
        {
            return Vec2::ZERO;
        }
        let t = time.seconds;
        let altitude = smooth((p.y - self.sea_level) / 24.0);
        let at = Vec2::new(p.x, p.z);
        // Keep gust coordinates in the fixed prevailing frame. Rotating the
        // spatial basis over time would make distant Ocean gusts unnaturally fast.
        let prevailing = Vec2::new(
            profile.heading_radians.sin(),
            -profile.heading_radians.cos(),
        );
        let phase = at.dot(prevailing) * 0.045 + at.perp_dot(prevailing) * 0.028;
        let veer =
            0.65 * oscillation(t, 23.0, phase * 0.8) + 0.35 * oscillation(t, 37.0, phase * -0.6);
        let heading = profile.heading_radians
            + 25_f32.to_radians() * oscillation(t, 97.0, 0.0)
            + 20_f32.to_radians() * veer;
        let direction = Vec2::new(heading.sin(), -heading.cos());
        let low = 0.7 * oscillation(t, 11.0, phase) + 0.3 * oscillation(t, 8.3, phase * 0.7);
        let high = 0.6 * oscillation(t, 3.7, phase) + 0.4 * oscillation(t, 5.3, phase * 0.7);
        let base = profile.speed.clamp(0.0, 25.0);
        let mean = base * (1.0 + 0.8 * altitude);
        let gust = low + (high - low) * altitude;
        let band = oscillation(t, 41.0, at.x * 0.023 - at.y * 0.019);
        let broad = curl(at, t, Vec2::new(26.0, 34.0), Vec2::new(53.0, 47.0), 0.0);
        let eddy = curl(at, t, Vec2::new(11.0, 15.0), Vec2::new(27.0, 31.0), 1.4);
        // Local eddies grow and subside smoothly, rather than permanently adding
        // the same turbulence everywhere. Strong curls are occasional.
        let pulse = smooth((oscillation(t, 29.0, at.x * 0.017 - at.y * 0.013) - 0.1) / 0.9);
        let swirl = (broad * 0.18 + eddy * (0.75 * pulse)).clamp_length_max(0.85) * mean;
        let flow = direction * mean * (1.0 + 0.45 * gust + 0.12 * band) + swirl;
        // Larger local turns also turn the wind shadow, so cover stays upwind.
        (flow * self.shelter(p, flow.normalize_or(direction))).clamp_length_max(25.0)
    }

    fn shelter(&self, p: Vec3, direction: Vec2) -> f32 {
        if self.spans.is_empty() {
            return 1.0;
        }
        let origin = Vec2::new(p.x, p.z);
        let mut obstruction = 0.0;
        for (angle, weight) in [(-0.12_f32, 0.25), (0.0, 0.5), (0.12, 0.25)] {
            let ray = Vec2::from_angle(angle).rotate(direction);
            let mut shadow = 0.0_f32;
            for step in 1..=48_i16 {
                let distance = f32::from(step);
                let at = origin - ray * distance;
                // Nearby blockers dominate; the wake recovers smoothly over 48u.
                let fade = 1.0 - smooth((distance - 3.0) / 45.0);
                shadow = shadow.max(self.obstruction(at, p.y) * fade);
            }
            obstruction += weight * shadow;
        }
        1.0 - 0.8 * obstruction
    }

    fn obstruction(&self, at: Vec2, height: f32) -> f32 {
        // Interpolate occupied spans, retaining openings in Y. This also makes
        // changes in probe direction continuous across hex-column boundaries.
        let grid = at / 0.5;
        let cell = grid.floor();
        let blend = grid - cell;
        let mut result = 0.0;
        for (dx, wx) in [(0.0, 1.0 - blend.x), (1.0, blend.x)] {
            for (dz, wz) in [(0.0, 1.0 - blend.y), (1.0, blend.y)] {
                let node = (cell + Vec2::new(dx, dz)) * 0.5;
                let coord = HexCoord::from_world(Vec3::new(node.x, height, node.y));
                let density = self.spans.get(&coord).map_or(0.0, |spans| {
                    spans
                        .iter()
                        .map(|span| {
                            smooth((height - span.x + 0.3) / 0.6)
                                * smooth((span.y - height + 0.3) / 0.6)
                        })
                        .fold(0.0_f32, f32::max)
                });
                result += wx * wz * density;
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::arena::ArenaStaticSpan;

    fn time(seconds: f64) -> OceanSimulationTime {
        OceanSimulationTime {
            generation: 7,
            seconds,
        }
    }
    fn east() -> OceanWindProfile {
        OceanWindProfile {
            heading_radians: std::f32::consts::FRAC_PI_2,
            speed: 9.0,
        }
    }
    fn wall() -> ArenaTerrainView {
        let mut view = ArenaTerrainView::default();
        for coord in HexCoord::ORIGIN.within_radius(16) {
            let p = coord.to_world(0.0);
            if p.x.abs() <= 1.5 && p.z.abs() <= 10.0 {
                view.static_spans.push(ArenaStaticSpan {
                    bottom: TilePos::new(coord, 20),
                    top_level: 32,
                    blocks_movement: true,
                    blocks_projectiles: true,
                    blocks_sight: true,
                });
            }
        }
        view
    }

    #[test]
    fn exposed_field_has_spatial_contrast_and_smooth_ocean_gusts() {
        let field = WindField::default();
        let mut min_speed = f32::MAX;
        let mut max_speed = 0.0_f32;
        let mut min_heading = f32::MAX;
        let mut max_heading = f32::MIN;
        // A neighborhood at one instant should contain different wind, not just
        // a single globally turning vector. All samples remain forward-biased.
        for x in -6..=6_i16 {
            for z in -6..=6_i16 {
                let p = Vec3::new(f32::from(x) * 8.0, SEA_LEVEL, f32::from(z) * 8.0);
                let v = field.velocity(p, time(12.0), east());
                min_speed = min_speed.min(v.length());
                max_speed = max_speed.max(v.length());
                min_heading = min_heading.min(v.y.atan2(v.x));
                max_heading = max_heading.max(v.y.atan2(v.x));
            }
        }
        assert!(
            max_speed - min_speed > 5.0,
            "speed contrast: {}",
            max_speed - min_speed
        );
        assert!(
            max_heading - min_heading > 35_f32.to_radians(),
            "direction contrast: {}",
            (max_heading - min_heading).to_degrees()
        );
        for n in 0..1000 {
            let t = 895.0 + f64::from(n) * 0.01;
            let p = Vec3::new(-570.0, 448.0, -201.0);
            let v = field.velocity(p, time(t), east());
            assert!(
                (v - field.velocity(p + Vec3::splat(0.001), time(t + 0.001), east())).length()
                    < 0.05
            );
            assert!(v.is_finite() && v.length() <= 25.001);
        }
    }

    #[test]
    fn ocean_sea_level_is_relative_and_region_contains_local_cover() {
        let view = wall();
        let geometry = ArenaVoxelGeometry::default();
        let near = WindField::for_region(&view, geometry, SEA_LEVEL, Some(Vec3::ZERO));
        let all = WindField::new(&view, geometry);
        let p = Vec3::new(4.0, 10.0, 0.0);
        assert_eq!(
            near.velocity(p, time(3.0), east()),
            all.velocity(p, time(3.0), east())
        );
        let empty = ArenaTerrainView::default();
        let lab = WindField::for_region(&empty, geometry, 8.0, None);
        let ocean = WindField::for_region(&empty, geometry, 160.0, None);
        for height in [0.0, 12.0, 24.0] {
            assert_eq!(
                lab.velocity(p.with_y(8.0 + height), time(5.0), east()),
                ocean.velocity(p.with_y(160.0 + height), time(5.0), east())
            );
        }
    }

    #[test]
    fn repeatable_bounded_continuous_and_unwrapped() {
        let field = WindField::new(&wall(), ArenaVoxelGeometry::default());
        for n in 0..1200 {
            let t = f64::from(n) * 0.9;
            let p = Vec3::new(4.3, 10.1, 2.4);
            let v = field.velocity(p, time(t), east());
            assert_eq!(v, field.velocity(p, time(t), east())); // includes paused clock
            assert!(v.is_finite() && v.length() <= 25.001);
            assert!(
                (v - field.velocity(p + Vec3::splat(0.001), time(t + 0.001), east())).length()
                    < 0.05
            );
        }
        let p = Vec3::new(0.0, 38.0, 0.0);
        assert!(
            (field.velocity(p, time(899.999), east()) - field.velocity(p, time(900.001), east()))
                .length()
                < 0.05
        );
        assert_eq!(field.velocity(Vec3::NAN, time(0.0), east()), Vec2::ZERO);
        assert_eq!(field.velocity(p, time(f64::NAN), east()), Vec2::ZERO);
        let extreme = OceanWindProfile {
            speed: f32::MAX,
            ..east()
        };
        assert!(field.velocity(p, time(2.0), extreme).length() <= 25.001);
    }

    #[test]
    fn cover_follows_heading_clears_above_and_republishes() {
        let mut view = wall();
        let geometry = ArenaVoxelGeometry::default();
        let covered = WindField::new(&view, geometry);
        let lee = Vec3::new(4.0, 10.0, 0.0);
        let windward = Vec3::new(-4.0, 10.0, 0.0);
        assert!(covered.shelter(lee, Vec2::X) < 0.28);
        assert!(covered.shelter(windward, Vec2::X) > 0.99);
        assert!(covered.shelter(windward, -Vec2::X) < 0.28);
        assert!(covered.shelter(lee, -Vec2::X) > 0.99);
        assert!(covered.shelter(lee.with_y(14.0), Vec2::X) > 0.99);
        assert!(covered.shelter(lee.with_y(7.0), Vec2::X) > 0.99);
        view.static_spans.clear();
        view.revision += 1;
        let removed = WindField::new(&view, geometry);
        assert!(removed.shelter(lee, Vec2::X) > 0.99);
        assert!(covered.shelter(lee, Vec2::X) < 0.28); // immutable old publication
        assert!(covered.shelter(lee.with_x(45.0), Vec2::X) > 0.95);
    }

    #[test]
    fn exposed_peaks_are_faster_and_change_faster() {
        let field = WindField::default();
        let stats = |height| {
            let mut speed = 0.0;
            let mut variation = 0.0;
            for n in 0..1200 {
                let t = f64::from(n) * 0.1;
                let p = Vec3::new(8.0, height, 4.0);
                let v = field.velocity(p, time(t), east());
                speed += v.length();
                variation += (v - field.velocity(p, time(t + 0.1), east())).length();
            }
            (speed / 1200.0, variation / 1200.0)
        };
        let sea = stats(SEA_LEVEL);
        let peak = stats(SEA_LEVEL + 24.0);
        assert!((8.0..10.0).contains(&sea.0));
        assert!(peak.0 > sea.0 * 1.65 && peak.0 < sea.0 * 1.95);
        assert!(peak.1 / peak.0 > 1.8 * sea.1 / sea.0);
    }
    #[test]
    fn published_solid_wall_preserves_openings_and_wave_controls_do_not_change_wind() {
        use hex_core::ocean::OceanEnvironmentSampler;
        use hex_core::{
            arena::ArenaSolidSpan,
            water_lab::{LabWind, WaterLabSettings},
            SubstanceId,
        };
        let geometry = ArenaVoxelGeometry::default();
        let mut view = wall();
        for span in std::mem::take(&mut view.static_spans) {
            // Two occupied runs separated by a genuine opening at eye height.
            view.columns.entry(span.bottom.coord).or_default().extend([
                ArenaSolidSpan {
                    bottom: span.bottom,
                    top_level: 22,
                    substance: SubstanceId(1),
                },
                ArenaSolidSpan {
                    bottom: TilePos::new(span.bottom.coord, 29),
                    top_level: 32,
                    substance: SubstanceId(1),
                },
            ]);
        }
        let opening = WindField::new(&view, geometry);
        assert!(opening.shelter(Vec3::new(4.0, 10.0, 0.0), Vec2::X) > 0.99);
        for spans in view.columns.values_mut() {
            spans.push(ArenaSolidSpan {
                bottom: spans.first().expect("lower span").bottom,
                top_level: 32,
                substance: SubstanceId(1),
            });
        }
        view.revision += 1; // Same compact publication used by a placed shield wall.
        let settings = WaterLabSettings {
            wind: LabWind::Field,
            ..default()
        };
        let mut surface = super::super::LabSurface::new(&view, geometry, settings);
        let p = Vec3::new(4.0, 10.0, 0.0);
        let blocked = surface.wind_at(p, time(900.0), east());
        surface.settings.frozen_phase = Some(2.0);
        surface.settings.phase_origin = 899.0;
        assert_eq!(blocked, surface.wind_at(p, time(900.0), east()));
        view.columns.clear();
        view.revision += 1;
        let removed = super::super::LabSurface::new(&view, geometry, settings);
        assert!(removed.wind_at(p, time(900.0), east()).length() > blocked.length() * 2.0);
    }
}
