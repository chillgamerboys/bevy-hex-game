//! Slow energy response to the broad modes of `water_lab::WindField`.
//!
//! Analytic first-order filtering (12-second response time) gives reproducible
//! sea state without another mutable clock or per-camera integration. The long
//! wave trains keep fixed directions; veering redistributes their energy instead
//! of rotating phases around the world origin. Small air eddies do not move them.
use bevy::prelude::*;
use hex_core::ocean::OceanSimulationTime;

use super::OceanSurfaceProfile;

const RESPONSE_SECONDS: f64 = 12.0;

/// Packed uniforms shared by CPU and WGSL. `energy.w` enables shore response;
/// `band.xy` is the reduced phase and gain of the spatial 41-second wind band.
#[derive(Clone, Copy, Debug)]
pub(super) struct WindResponse {
    pub energy: Vec4,
    pub band: Vec4,
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "Individually reduced oscillator phases and gains are bounded."
)]
fn filtered_phase(seconds: f64, period: f64) -> (f32, f32) {
    let rate = std::f64::consts::TAU / period;
    let response = rate * RESPONSE_SECONDS;
    (
        (seconds.rem_euclid(period) * rate - response.atan()) as f32,
        (1.0 / (1.0 + response * response).sqrt()) as f32,
    )
}

impl WindResponse {
    pub fn new(profile: &OceanSurfaceProfile, time: OceanSimulationTime) -> Self {
        let Some(wind) = profile.wind_response else {
            return Self {
                energy: Vec4::new(1.0, 1.0, 1.0, 0.0),
                band: Vec4::ZERO,
            };
        };
        let oscillator = |period| {
            let (phase, gain) = filtered_phase(time.seconds, period);
            phase.sin() * gain
        };
        // Broad natural-wind modes at the world origin. Keeping this frame
        // fixed prevents distant waves racing when the wind changes heading.
        let heading = wind.heading_radians
            + 25_f32.to_radians() * oscillator(97.0)
            + 20_f32.to_radians() * (0.65 * oscillator(23.0) + 0.35 * oscillator(37.0));
        let direction = Vec2::new(heading.sin(), -heading.cos());
        let gust = 0.7 * oscillator(11.0) + 0.3 * oscillator(8.3);
        let strength = (wind.speed / 9.0).sqrt() * (1.0 + 0.45 * gust);
        let [primary, secondary, detail] = profile.waves.map(|wave| {
            let alignment = wave.direction.normalize().dot(direction).max(0.0);
            strength * (0.3 + 0.7 * alignment * alignment)
        });
        let (phase, gain) = filtered_phase(time.seconds, 41.0);
        Self {
            energy: Vec4::new(primary, secondary, detail, 1.0),
            band: Vec4::new(phase, 0.12 * gain, 0.0, 0.0),
        }
    }

    pub fn local_gain(self, at: Vec2, depth: f32, shelter: f32) -> f32 {
        if self.energy.w < 0.5 {
            return 1.0;
        }
        let band = 1.0 + self.band.y * (at.x * 0.023 - at.y * 0.019 + self.band.x).sin();
        // Shoaling is already part of the accepted carrier. Dissipate the
        // steepened wave in the final 0.8 units of water rather than amplify
        // it onto the dry beach; the exact wet mask retains coverage authority.
        let depth_t = (depth / 0.8).clamp(0.0, 1.0);
        let breaking = depth_t * depth_t * (3.0 - 2.0 * depth_t);
        band * breaking * (0.45 + 0.55 * shelter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::ocean::OceanWindProfile;

    fn profile(speed: f32) -> OceanSurfaceProfile {
        OceanSurfaceProfile::regular_voxels(0.0, 0.4).with_wind_response(OceanWindProfile {
            heading_radians: std::f32::consts::FRAC_PI_2,
            speed,
        })
    }

    #[test]
    fn wind_energy_is_continuous_across_carrier_phase_wrap_and_long_sessions() {
        let profile = profile(9.0);
        for seconds in [900.0, 1_800.0, 86_400.0, 1_000_000_000.0] {
            let before = WindResponse::new(
                &profile,
                OceanSimulationTime {
                    generation: 2,
                    seconds: seconds - 0.001,
                },
            );
            let after = WindResponse::new(
                &profile,
                OceanSimulationTime {
                    generation: 2,
                    seconds: seconds + 0.001,
                },
            );
            assert!(before.energy.distance(after.energy) < 0.001);
            assert!(
                (before.local_gain(Vec2::new(731.0, -456.0), 3.0, 1.0)
                    - after.local_gain(Vec2::new(731.0, -456.0), 3.0, 1.0))
                .abs()
                    < 0.001
            );
        }
        let at_zero = WindResponse::new(&profile, OceanSimulationTime::default());
        let at_wrap = WindResponse::new(
            &profile,
            OceanSimulationTime {
                generation: 0,
                seconds: 900.0,
            },
        );
        assert!(
            at_zero.energy.distance(at_wrap.energy) > 0.01,
            "wind must not repeat on the carrier clock"
        );
    }

    #[test]
    fn stronger_wind_increases_energy_and_shallow_shelter_dissipates_it() {
        let time = OceanSimulationTime {
            generation: 7,
            seconds: 143.0,
        };
        let low = WindResponse::new(&profile(4.0), time);
        let high = WindResponse::new(&profile(16.0), time);
        assert!(high
            .energy
            .truncate()
            .abs_diff_eq(low.energy.truncate() * 2.0, 0.00001));
        assert_eq!(
            WindResponse::new(&profile(0.0), time).energy.truncate(),
            Vec3::ZERO
        );
        assert!(high.local_gain(Vec2::ZERO, 0.2, 1.0) < high.local_gain(Vec2::ZERO, 2.0, 1.0));
        assert!(high.local_gain(Vec2::ZERO, 2.0, 0.0) < high.local_gain(Vec2::ZERO, 2.0, 1.0));
        assert!(high.local_gain(Vec2::ZERO, 0.0, 1.0).abs() < f32::EPSILON);
        let resumed = WindResponse::new(
            &profile(16.0),
            OceanSimulationTime {
                generation: 8,
                ..time
            },
        );
        assert_eq!(resumed.energy, high.energy);
        assert_eq!(resumed.band, high.band);
    }
}
