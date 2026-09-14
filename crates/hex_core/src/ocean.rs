//! Shared ocean sampling, wind and simulation-time contracts.
//!
//! World owns the immutable surface sampler and its bathymetry. Gameplay supplies
//! current admitted liquid columns for every query; the sampler must not retain a
//! residency snapshot. These contracts add no movement or fluid simulation.

use std::{fmt::Debug, sync::Arc};

use bevy_ecs::prelude::Resource;
use bevy_math::{Vec2, Vec3};

use crate::{arena::ArenaAvailability, SubstanceId};

/// Common repeat interval for the ocean's 18-, 25- and 12-second swells.
pub const OCEAN_PHASE_SECONDS: f64 = 900.0;

/// Read-only run time, published from gameplay's completed fixed tick.
///
/// Pause preserves the tick and therefore this value. Restart changes the
/// generation and publishes tick zero. Presentation must consume this snapshot
/// instead of advancing a second clock; frozen captures publish a fixed snapshot.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct OceanSimulationTime {
    /// The reset generation to which this time belongs.
    pub generation: u64,
    /// Unwrapped elapsed simulation seconds, also used by slow wind gusts.
    pub seconds: f64,
}

impl OceanSimulationTime {
    /// Derive time without accumulating a second independently rounded timer.
    #[must_use]
    #[expect(
        clippy::cast_precision_loss,
        reason = "a run cannot approach 2^53 fixed ticks"
    )]
    pub fn from_fixed_tick(generation: u64, tick: u64, step_seconds: f64) -> Self {
        Self {
            generation,
            seconds: tick as f64 * step_seconds,
        }
    }

    /// Identical bounded phase supplied to CPU surface sampling and GPU uniforms.
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the shared shader phase is bounded to 900 seconds"
    )]
    pub fn phase_seconds(self) -> f32 {
        self.seconds.rem_euclid(OCEAN_PHASE_SECONDS) as f32
    }
}

/// One liquid column from the current authoritative terrain publication.
///
/// Consumers obtain this after resolving column availability, using a lookup
/// refreshed with the terrain revision. Approximate bathymetry cannot create a
/// column or replace its exact bounds. This is not a query for water at a Y value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OceanWaterColumn {
    /// Undisplaced upper face of the admitted liquid interval.
    pub mean_height: f32,
    /// Lower face of the admitted liquid interval, preserving the actual seabed.
    pub bed_height: f32,
    /// World-owned material identity of this liquid interval.
    pub water_id: SubstanceId,
}

/// Surface at an absolute world X/Z location and a shared simulation phase.
///
/// Height and slope describe surface contact only. They do not add currents,
/// buoyancy, draining, refilling or a second authority over liquid occupancy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OceanSurfaceSample {
    /// Displaced surface height in world units.
    pub height: f32,
    /// Unit upward surface normal, matching the rendered displacement derivative.
    pub normal: Vec3,
    /// Analytic rate of change of surface height, in world units per second.
    pub vertical_velocity: f32,
    /// Exact undisplaced liquid upper face copied from the supplied column.
    pub mean_height: f32,
    /// Exact liquid lower face copied from the supplied column.
    pub bed_height: f32,
    /// Exact material identity copied from the supplied column.
    pub water_id: SubstanceId,
}

/// World-owned, immutable surface implementation shared through an [`Arc`].
pub trait OceanEnvironmentSampler: Debug + Send + Sync {
    /// Sample a confirmed wet column using the same phase and formulas as rendering.
    ///
    /// The implementation may own immutable bathymetry/shelter data, but must not
    /// retain streamed admission facts. It must preserve the supplied column's
    /// exact bounds and identity, including wet shoreline columns that approximate
    /// bathymetry classifies as dry. Return `None` when the finite sampler does not
    /// cover the location or cannot produce a valid sample; decorative horizon
    /// water must never provide a fallback.
    fn surface_at(
        &self,
        xz: Vec2,
        phase_seconds: f32,
        column: OceanWaterColumn,
    ) -> Option<OceanSurfaceSample>;
}

/// Conservative result of combining current admission with ocean presentation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OceanSurfaceState {
    /// Exact admitted liquid with a usable surface sample.
    ReadyWet(OceanSurfaceSample),
    /// Exact admitted column has no ocean liquid interval.
    ReadyDry,
    /// Exact terrain or the surface required for safe contact is unavailable.
    /// A failed sampler on a known wet column is never converted into dry air.
    Unloaded,
    /// Outside the finite authoritative world, regardless of decorative water.
    OutsideWorld,
}

/// Deterministic prevailing wind; velocity is horizontal and measured in units/s.
///
/// Gusts change speed by at most 16% and heading by at most 0.06 radians. They are
/// slow functions of unwrapped simulation time and therefore stop during pause
/// without jumping at the ocean's 900-second phase wrap.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OceanWindProfile {
    /// Clockwise heading from world north (negative Z), in radians.
    pub heading_radians: f32,
    /// Mean wind speed before the bounded gusts.
    pub speed: f32,
}

impl Default for OceanWindProfile {
    fn default() -> Self {
        Self {
            heading_radians: 0.4,
            speed: 10.0,
        }
    }
}

impl OceanWindProfile {
    /// Absolute X/Z wind velocity; invalid profiles or clocks safely yield calm.
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "bounded wind trig results feed f32 world vectors"
    )]
    pub fn velocity_at(self, time: OceanSimulationTime) -> Vec2 {
        if !time.seconds.is_finite() || !self.heading_radians.is_finite() || !self.speed.is_finite()
        {
            return Vec2::ZERO;
        }
        let t = time.seconds;
        let tau = std::f64::consts::TAU;
        let heading = f64::from(self.heading_radians) + 0.06 * (t * tau / 120.0).sin();
        let gust = 1.0 + 0.12 * (t * tau / 75.0).sin() + 0.04 * (t * tau / 31.0 + 1.2).sin();
        let speed = self.speed.max(0.0) * gust as f32;
        Vec2::new(heading.sin() as f32, -heading.cos() as f32) * speed
    }
}

/// Immutable world publication; remove or replace it when switching map/package.
#[derive(Resource, Clone, Debug)]
pub struct OceanEnvironmentView {
    /// Package identity used to reject a sampler retained from a previous map.
    pub package_fingerprint: u64,
    /// World implementation, with no mutable or cached residency information.
    pub sampler: Arc<dyn OceanEnvironmentSampler>,
    /// Shared wind consumed by gameplay and visible wind cues.
    pub wind: OceanWindProfile,
}

impl OceanEnvironmentView {
    /// Combine a surface with current exact column facts, checking admission first.
    ///
    /// `availability` and `column` must come from the same current publication.
    /// A ready wet column whose sampler fails remains blocked as `Unloaded`;
    /// callers must hold the last safe pose instead of proceeding through air.
    #[must_use]
    pub fn sample(
        &self,
        xz: Vec2,
        time: OceanSimulationTime,
        availability: ArenaAvailability,
        column: Option<OceanWaterColumn>,
    ) -> OceanSurfaceState {
        match availability {
            ArenaAvailability::Unloaded => return OceanSurfaceState::Unloaded,
            ArenaAvailability::OutsideWorld => return OceanSurfaceState::OutsideWorld,
            ArenaAvailability::Ready => {}
        }
        let Some(column) = column else {
            return OceanSurfaceState::ReadyDry;
        };
        if !xz.is_finite()
            || !time.seconds.is_finite()
            || !column.mean_height.is_finite()
            || !column.bed_height.is_finite()
            || column.bed_height >= column.mean_height
        {
            return OceanSurfaceState::Unloaded;
        }
        let Some(sample) = self.sampler.surface_at(xz, time.phase_seconds(), column) else {
            return OceanSurfaceState::Unloaded;
        };
        if !sample.height.is_finite()
            || !sample.normal.is_finite()
            || sample.normal.y <= 0.0
            || (sample.normal.length_squared() - 1.0).abs() > 0.01
            || !sample.vertical_velocity.is_finite()
            || sample.mean_height.to_bits() != column.mean_height.to_bits()
            || sample.bed_height.to_bits() != column.bed_height.to_bits()
            || sample.water_id != column.water_id
        {
            return OceanSurfaceState::Unloaded;
        }
        OceanSurfaceState::ReadyWet(sample)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Debug, Default)]
    struct ProbeSampler {
        calls: AtomicUsize,
        fail: bool,
        wrong_identity: bool,
    }

    impl OceanEnvironmentSampler for ProbeSampler {
        fn surface_at(
            &self,
            _xz: Vec2,
            phase_seconds: f32,
            column: OceanWaterColumn,
        ) -> Option<OceanSurfaceSample> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            (!self.fail).then_some(OceanSurfaceSample {
                height: column.mean_height + phase_seconds.sin(),
                normal: Vec3::Y,
                vertical_velocity: phase_seconds.cos(),
                mean_height: column.mean_height,
                bed_height: column.bed_height,
                water_id: if self.wrong_identity {
                    SubstanceId(9)
                } else {
                    column.water_id
                },
            })
        }
    }

    fn column() -> OceanWaterColumn {
        OceanWaterColumn {
            mean_height: 20.0,
            bed_height: -8.0,
            water_id: SubstanceId(3),
        }
    }

    fn view(sampler: Arc<ProbeSampler>) -> OceanEnvironmentView {
        OceanEnvironmentView {
            package_fingerprint: 7,
            sampler,
            wind: OceanWindProfile::default(),
        }
    }

    #[test]
    fn current_admission_precedes_cached_surface_and_dry_columns() {
        let sampler = Arc::new(ProbeSampler::default());
        let world = view(Arc::clone(&sampler));
        let time = OceanSimulationTime::default();
        for wet in [None, Some(column())] {
            assert_eq!(
                world.sample(Vec2::ZERO, time, ArenaAvailability::Unloaded, wet),
                OceanSurfaceState::Unloaded
            );
            assert_eq!(
                world.sample(Vec2::ZERO, time, ArenaAvailability::OutsideWorld, wet),
                OceanSurfaceState::OutsideWorld
            );
        }
        assert_eq!(
            world.sample(Vec2::ZERO, time, ArenaAvailability::Ready, None),
            OceanSurfaceState::ReadyDry
        );
        assert_eq!(sampler.calls.load(Ordering::Relaxed), 0);
        assert!(matches!(
            world.sample(Vec2::ZERO, time, ArenaAvailability::Ready, Some(column())),
            OceanSurfaceState::ReadyWet(_)
        ));
        assert_eq!(sampler.calls.load(Ordering::Relaxed), 1);
        // Unloading after a successful sample cannot expose the cached wet column.
        assert_eq!(
            world.sample(
                Vec2::ZERO,
                time,
                ArenaAvailability::Unloaded,
                Some(column())
            ),
            OceanSurfaceState::Unloaded
        );
        assert_eq!(sampler.calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn rejected_or_inconsistent_wet_samples_never_become_dry_air() {
        for sampler in [
            ProbeSampler {
                fail: true,
                ..Default::default()
            },
            ProbeSampler {
                wrong_identity: true,
                ..Default::default()
            },
        ] {
            let world = view(Arc::new(sampler));
            assert_eq!(
                world.sample(
                    Vec2::ZERO,
                    OceanSimulationTime::default(),
                    ArenaAvailability::Ready,
                    Some(column())
                ),
                OceanSurfaceState::Unloaded
            );
        }
    }

    #[test]
    fn fixed_tick_time_preserves_pause_reset_and_common_surface_phase() {
        let paused = OceanSimulationTime::from_fixed_tick(4, 108_015, 1.0 / 120.0);
        let same_tick = OceanSimulationTime::from_fixed_tick(4, 108_015, 1.0 / 120.0);
        assert_eq!(paused, same_tick);
        assert!((paused.seconds - 900.125).abs() < 0.000_001);
        assert!((paused.phase_seconds() - 0.125).abs() < 0.000_001);
        let reset = OceanSimulationTime::from_fixed_tick(5, 0, 1.0 / 120.0);
        assert_eq!(reset.generation, 5);
        assert!(reset.seconds.abs() < f64::EPSILON);
        let world = view(Arc::new(ProbeSampler::default()));
        let sampled = world.sample(Vec2::ZERO, paused, ArenaAvailability::Ready, Some(column()));
        assert!(matches!(sampled, OceanSurfaceState::ReadyWet(_)));
        if let OceanSurfaceState::ReadyWet(sample) = sampled {
            assert!((sample.height - (20.0 + paused.phase_seconds().sin())).abs() < 0.000_001);
        }
    }

    #[test]
    fn wind_is_bounded_deterministic_and_continuous_across_wave_wrap() {
        let wind = OceanWindProfile::default();
        for seconds in 0..1800 {
            let time = OceanSimulationTime {
                generation: 1,
                seconds: f64::from(seconds),
            };
            let velocity = wind.velocity_at(time);
            assert!((8.4 - 0.000_01..=11.6 + 0.000_01).contains(&velocity.length()));
            assert!((velocity - wind.velocity_at(time)).length() < f32::EPSILON);
        }
        let before = wind.velocity_at(OceanSimulationTime {
            generation: 1,
            seconds: 899.999,
        });
        let after = wind.velocity_at(OceanSimulationTime {
            generation: 1,
            seconds: 900.001,
        });
        assert!(before.distance(after) < 0.001);
        assert!(
            wind.velocity_at(OceanSimulationTime {
                generation: 1,
                seconds: f64::NAN
            })
            .length()
                < f32::EPSILON
        );
    }
}
