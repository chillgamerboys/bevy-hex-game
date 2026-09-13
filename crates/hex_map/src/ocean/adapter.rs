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
