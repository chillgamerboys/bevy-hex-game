//! Bounded kinematic wind, with shelter from an immutable world publication.
use std::collections::HashMap;

use bevy::prelude::*;
use hex_core::{
    arena::{ArenaTerrainView, ArenaVoxelGeometry},
    ocean::{OceanSimulationTime, OceanWindProfile},
    HexCoord, TilePos,
};

use super::SEA_LEVEL;

#[derive(Debug, Default, Clone)]
pub(super) struct WindField {
    spans: HashMap<HexCoord, Vec<Vec2>>,
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

impl WindField {
    pub(super) fn new(view: &ArenaTerrainView, geometry: ArenaVoxelGeometry) -> Self {
        let mut field = Self::default();
        let mut insert = |bottom: TilePos, top: i32| {
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

    pub(super) fn velocity(
        &self,
        p: Vec3,
        time: OceanSimulationTime,
        profile: OceanWindProfile,
    ) -> Vec2 {
        if !p.is_finite()
            || p.abs().max_element() > 100_000.0
            || !time.seconds.is_finite()
            || !profile.speed.is_finite()
            || !profile.heading_radians.is_finite()
        {
            return Vec2::ZERO;
        }
        let t = time.seconds;
        let altitude = smooth((p.y - SEA_LEVEL) / 24.0);
        let heading = profile.heading_radians + 15_f32.to_radians() * oscillation(t, 113.0, 0.0);
        let direction = Vec2::new(heading.sin(), -heading.cos());
        let at = Vec2::new(p.x, p.z);
        let phase = at.dot(direction) * 0.045 + at.perp_dot(direction) * 0.028;
        let low = 0.7 * oscillation(t, 11.0, phase) + 0.3 * oscillation(t, 8.3, phase * 0.7);
        let high = 0.6 * oscillation(t, 3.7, phase) + 0.4 * oscillation(t, 5.3, phase * 0.7);
        let base = profile.speed.clamp(0.0, 25.0);
        let mean = base * (1.0 + 0.8 * altitude);
        let gust = low + (high - low) * altitude;
        // Curl of a broad moving stream function. Its components are related,
        // not independent random headings; the prevailing flow remains dominant.
        let a = p.x / 16.0;
        let b = p.z / 20.0;
        let swirl = Vec2::new(
            oscillation(t, 43.0, a) * oscillation(t, 37.0, b + std::f32::consts::FRAC_PI_2),
            -oscillation(t, 43.0, a + std::f32::consts::FRAC_PI_2) * oscillation(t, 37.0, b),
        )
        .clamp_length_max(1.0)
            * (base * 0.2);
        ((direction * mean * (1.0 + 0.3 * gust) + swirl) * self.shelter(p, direction))
            .clamp_length_max(25.0)
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
