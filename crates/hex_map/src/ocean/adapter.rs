use super::{sample, OceanBathymetry, OceanSurfaceProfile};
use bevy::prelude::*;
use hex_core::ocean::{OceanEnvironmentSampler, OceanSurfaceSample, OceanWaterColumn};

/// Immutable world-owned wave sampler. Store behind the core environment Arc;
/// every call receives current admitted occupancy rather than retaining residency.
#[derive(Debug, Clone)]
pub struct OceanSurfaceAdapter {
    profile: OceanSurfaceProfile,
    bath: std::sync::Arc<OceanBathymetry>,
    #[cfg(feature = "arena-prototype")]
    wind: Option<std::sync::Arc<crate::water_lab::WindField>>,
}
impl OceanSurfaceAdapter {
    /// Takes a validated immutable snapshot once per package/profile publication.
    ///
    /// # Errors
    /// Rejects incomplete/nonfinite bathymetry or invalid wave parameters.
    pub fn new(profile: OceanSurfaceProfile, bath: OceanBathymetry) -> Result<Self, &'static str> {
        if !profile.is_valid() || !bath.is_valid() {
            return Err("Invalid ocean environment snapshot");
        }
        Ok(Self {
            profile,
            bath: std::sync::Arc::new(bath),
            #[cfg(feature = "arena-prototype")]
            wind: None,
        })
    }
}
#[cfg(feature = "arena-prototype")]
impl OceanSurfaceAdapter {
    /// Attach a refreshed cover snapshot without copying the ocean bathymetry.
    #[must_use]
    pub fn with_wind_field(mut self, wind: crate::water_lab::WindField) -> Self {
        self.wind = Some(std::sync::Arc::new(wind));
        self
    }
}
impl OceanEnvironmentSampler for OceanSurfaceAdapter {
    fn wind_at(
        &self,
        position: Vec3,
        time: hex_core::ocean::OceanSimulationTime,
        profile: hex_core::ocean::OceanWindProfile,
    ) -> Vec2 {
        #[cfg(feature = "arena-prototype")]
        if let Some(field) = &self.wind {
            return field.velocity(position, time, profile);
        }
        #[cfg(not(feature = "arena-prototype"))]
        let _ = position;
        if self.profile.voxel_height > 0.0 {
            Vec2::X * 9.0
        } else {
            profile.velocity_at(time)
        }
    }

    fn surface_at(
        &self,
        at: Vec2,
        seconds: f32,
        column: OceanWaterColumn,
    ) -> Option<OceanSurfaceSample> {
        self.surface_at_time(
            at,
            hex_core::ocean::OceanSimulationTime {
                generation: 0,
                seconds: f64::from(seconds),
            },
            column,
        )
    }

    fn surface_at_time(
        &self,
        at: Vec2,
        time: hex_core::ocean::OceanSimulationTime,
        column: OceanWaterColumn,
    ) -> Option<OceanSurfaceSample> {
        if !self.bath.contains(at)
            || !time.seconds.is_finite()
            || !column.mean_height.is_finite()
            || !column.bed_height.is_finite()
            || column.bed_height >= column.mean_height
        {
            return None;
        }
        // Admitted inland lakes and rivers use the ordinary flat liquid mesh.
        // Their exact local surface must not be rejected or displaced by waves
        // belonging to the ocean's different mean sea level.
        if (column.mean_height - self.profile.mean_sea_level).abs() > 0.01 {
            return Some(OceanSurfaceSample {
                height: column.mean_height,
                normal: Vec3::Y,
                vertical_velocity: 0.0,
                mean_height: column.mean_height,
                bed_height: column.bed_height,
                water_id: column.water_id,
            });
        }
        let value = sample::sample_at_time(&self.profile, &self.bath, at, time, true)?;
        Some(OceanSurfaceSample {
            height: value.height + (column.mean_height - self.profile.mean_sea_level),
            normal: value.normal,
            vertical_velocity: value.vertical_velocity,
            mean_height: column.mean_height,
            bed_height: column.bed_height,
            water_id: column.water_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inland_liquids_keep_their_exact_flat_local_height_and_identity() {
        use hex_core::ocean::OceanSimulationTime;
        let adapter = OceanSurfaceAdapter::new(
            OceanSurfaceProfile::regular_voxels(160.0, 0.4),
            OceanBathymetry::default(),
        )
        .unwrap();
        let at = Vec2::splat(0.5);
        for mean_height in [148.0, 173.6, 209.2] {
            let column = OceanWaterColumn {
                mean_height,
                bed_height: mean_height - 4.0,
                water_id: hex_core::SubstanceId(7),
            };
            for seconds in [0.0, 19.25, 900.001] {
                let time = OceanSimulationTime {
                    generation: 2,
                    seconds,
                };
                let sample = adapter.surface_at_time(at, time, column).unwrap();
                assert_eq!(sample.height.to_bits(), mean_height.to_bits());
                assert_eq!(sample.mean_height.to_bits(), mean_height.to_bits());
                assert_eq!(sample.bed_height.to_bits(), column.bed_height.to_bits());
                assert_eq!(sample.normal, Vec3::Y);
                assert!(sample.vertical_velocity.abs() < f32::EPSILON);
                assert_eq!(sample.water_id, column.water_id);
            }
            assert!(adapter
                .surface_at_time(Vec2::splat(2000.0), OceanSimulationTime::default(), column)
                .is_none());
            assert!(adapter
                .surface_at_time(
                    at,
                    OceanSimulationTime::default(),
                    OceanWaterColumn {
                        bed_height: mean_height,
                        ..column
                    }
                )
                .is_none());
            assert!(adapter
                .surface_at_time(
                    at,
                    OceanSimulationTime {
                        generation: 0,
                        seconds: f64::NAN
                    },
                    column
                )
                .is_none());
        }
    }

    #[test]
    fn unwrapped_wind_contact_agrees_with_camera_and_preserves_admitted_bounds() {
        use hex_core::ocean::{OceanSimulationTime, OceanWindProfile};
        let profile =
            OceanSurfaceProfile::regular_voxels(0.0, 0.4).with_wind_response(OceanWindProfile {
                heading_radians: 1.1,
                speed: 12.0,
            });
        let bed = OceanBathymetry::default();
        let adapter = OceanSurfaceAdapter::new(profile.clone(), bed.clone()).unwrap();
        let column = OceanWaterColumn {
            mean_height: 0.0,
            bed_height: -23.0,
            water_id: hex_core::SubstanceId(3),
        };
        for seconds in [899.999, 900.001, 1_800.01, 100_000.0] {
            let time = OceanSimulationTime {
                generation: 3,
                seconds,
            };
            let at = Vec2::splat(0.5);
            let contact = adapter.surface_at_time(at, time, column).unwrap();
            let camera = super::super::sample_surface_at_time(&profile, &bed, at, time).unwrap();
            assert!((contact.height - camera.height).abs() < 0.00001);
            assert_eq!(contact.bed_height.to_bits(), column.bed_height.to_bits());
            assert_eq!(contact.mean_height.to_bits(), column.mean_height.to_bits());
            assert_eq!(contact.water_id, column.water_id);
        }
    }

    #[cfg(feature = "arena-prototype")]
    #[test]
    fn attached_natural_field_is_shared_by_ocean_consumers() {
        use hex_core::{
            arena::{ArenaTerrainView, ArenaVoxelGeometry},
            ocean::{OceanSimulationTime, OceanWindProfile},
        };
        let terrain = ArenaTerrainView::default();
        let field = crate::water_lab::WindField::for_region(
            &terrain,
            ArenaVoxelGeometry::default(),
            160.0,
            None,
        );
        let adapter = OceanSurfaceAdapter::new(
            OceanSurfaceProfile::regular_voxels(160.0, 0.4),
            OceanBathymetry::default(),
        )
        .unwrap()
        .with_wind_field(field.clone());
        let wind = OceanWindProfile {
            heading_radians: std::f32::consts::FRAC_PI_2,
            speed: 9.0,
        };
        for height in [160.0, 174.0, 507.0] {
            for seconds in [0.0, 5.0, 900.001] {
                let p = Vec3::new(7.0, height, 3.0);
                let time = OceanSimulationTime {
                    generation: 1,
                    seconds,
                };
                assert_eq!(
                    adapter.wind_at(p, time, wind),
                    field.velocity(p, time, wind)
                );
            }
        }
    }

    #[test]
    fn taller_swells_agree_for_camera_and_admitted_gameplay_sampling() {
        let profile = OceanSurfaceProfile::default();
        let bed = OceanBathymetry::default();
        let adapter = OceanSurfaceAdapter::new(profile.clone(), bed.clone()).unwrap();
        let column = OceanWaterColumn {
            mean_height: profile.mean_sea_level,
            bed_height: -140.0,
            water_id: hex_core::SubstanceId(3),
        };
        let at = Vec2::splat(0.5);
        for phase in [0.0, 7.0, 13.0, 31.0] {
            let gameplay = adapter.surface_at(at, phase, column).unwrap();
            let camera = super::super::sample_local_surface(
                &profile,
                &bed,
                &super::super::OceanNearBoundary::default(),
                at,
                phase,
            )
            .unwrap();
            assert!((gameplay.height - camera.height).abs() < 0.00001);
            assert!(gameplay.normal.distance(camera.normal) < 0.00001);
            assert!((gameplay.vertical_velocity - camera.vertical_velocity).abs() < 0.00001);
        }
    }

    #[test]
    fn voxel_profile_keeps_camera_float_and_steady_wind_in_agreement() {
        let profile = OceanSurfaceProfile::regular_voxels(0.0, 0.4);
        let bed = OceanBathymetry::default();
        let adapter = OceanSurfaceAdapter::new(profile.clone(), bed.clone()).unwrap();
        let column = OceanWaterColumn {
            mean_height: 0.0,
            bed_height: -140.0,
            water_id: hex_core::SubstanceId(3),
        };
        let at = Vec2::splat(0.5);
        for phase in [0.0, 1.0, 3.0, 7.0] {
            let gameplay = adapter.surface_at(at, phase, column).unwrap();
            let camera = super::super::sample_surface(&profile, &bed, at, phase).unwrap();
            assert!((gameplay.height - camera.height).abs() < 0.00001);
            assert!((gameplay.height / 0.4 - (gameplay.height / 0.4).round()).abs() < 0.00001);
            assert_eq!(gameplay.normal, Vec3::Y);
        }
        assert_eq!(
            adapter.wind_at(Vec3::ZERO, Default::default(), Default::default()),
            Vec2::X * 9.0
        );
        assert!(adapter
            .surface_at(
                at,
                0.0,
                OceanWaterColumn {
                    bed_height: 1.0,
                    ..column
                }
            )
            .is_none());
    }

    #[test]
    fn ocean_wave_groups_vary_along_crests_and_between_primary_periods() {
        let profile = OceanSurfaceProfile::regular_voxels(0.0, 0.4);
        let bed = OceanBathymetry {
            origin_xz: Vec2::splat(-128.0),
            spacing: 256.0,
            ..default()
        };
        let adapter = OceanSurfaceAdapter::new(profile, bed).unwrap();
        let column = OceanWaterColumn {
            mean_height: 0.0,
            bed_height: -140.0,
            water_id: hex_core::SubstanceId(3),
        };
        let height = |at, seconds| adapter.surface_at(at, seconds, column).unwrap().height;
        let mut along_crest = false;
        let mut across_periods = false;
        for tick in 0..90 {
            let t = tick as f32 * 0.1;
            let center = height(Vec2::ZERO, t);
            along_crest |= (center - height(Vec2::new(0.0, 24.0), t)).abs() > 0.39;
            across_periods |= (center - height(Vec2::ZERO, t + 9.0)).abs() > 0.39;
            assert!(center.abs() <= 1.6);
        }
        assert!(
            along_crest,
            "wave crests must not remain straight identical bands"
        );
        assert!(across_periods, "successive primary waves must differ");
        // Both render and gameplay wrap the shared clock at 900 seconds.
        for at in [Vec2::ZERO, Vec2::new(0.0, 24.0), Vec2::new(19.0, -31.0)] {
            assert!((height(at, 900.0) - height(at, 0.0)).abs() < 0.0001);
        }
    }

    #[test]
    fn adapter_keeps_exact_wet_bounds_without_decorative_or_residency_fallback() {
        let bed = OceanBathymetry {
            bed_heights: vec![2.0; 4],
            ..default()
        };
        let adapter = OceanSurfaceAdapter::new(OceanSurfaceProfile::default(), bed).unwrap();
        let column = OceanWaterColumn {
            mean_height: 0.0,
            bed_height: -7.0,
            water_id: hex_core::SubstanceId(3),
        };
        let wet = adapter.surface_at(Vec2::ZERO, 0.0, column).unwrap();
        assert!(wet.height.abs() < 0.00001 && wet.vertical_velocity.abs() < 0.00001);
        assert_eq!(wet.bed_height.to_bits(), column.bed_height.to_bits());
        assert_eq!(wet.water_id, column.water_id);
        assert!(adapter
            .surface_at(Vec2::new(-0.01, 0.0), 0.0, column)
            .is_none());
        assert!(adapter.surface_at(Vec2::ZERO, f32::NAN, column).is_none());
    }
}
