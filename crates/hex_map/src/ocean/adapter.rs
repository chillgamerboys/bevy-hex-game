use super::{sample, OceanBathymetry, OceanSurfaceProfile};
use bevy::prelude::*;
use hex_core::ocean::{OceanEnvironmentSampler, OceanSurfaceSample, OceanWaterColumn};

/// Immutable world-owned wave sampler. Store behind the core environment Arc;
/// every call receives current admitted occupancy rather than retaining residency.
#[derive(Debug)]
pub struct OceanSurfaceAdapter {
    profile: OceanSurfaceProfile,
    bath: OceanBathymetry,
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
        Ok(Self { profile, bath })
    }
}
impl OceanEnvironmentSampler for OceanSurfaceAdapter {
    fn wind_at(
        &self,
        _position: Vec3,
        time: hex_core::ocean::OceanSimulationTime,
        profile: hex_core::ocean::OceanWindProfile,
    ) -> Vec2 {
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
        if !self.bath.contains(at)
            || !column.mean_height.is_finite()
            || !column.bed_height.is_finite()
            || column.bed_height >= column.mean_height
            || (column.mean_height - self.profile.mean_sea_level).abs() > 0.01
        {
            return None;
        }
        let value = sample::sample(&self.profile, &self.bath, at, seconds, true)?;
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
