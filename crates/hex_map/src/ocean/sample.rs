use super::{OceanBathymetry, OceanNearBoundary, OceanSurfaceProfile};
use bevy::prelude::*;

/// Surface height, normal and color; exact liquid occupancy remains world authority.
#[derive(Debug, Clone, Copy)]
pub struct OceanSurfaceSample {
    /// Displaced water height in world units.
    pub height: f32,
    /// Analytic upward normal including shoreline attenuation.
    pub normal: Vec3,
    /// Analytic vertical surface velocity; no horizontal current is implied.
    pub vertical_velocity: f32,
    /// Initial static depth at this X/Z point.
    pub depth: f32,
    /// Linear shallow/deep water color.
    pub color: Vec4,
}

/// Samples the same bilinear depth and analytic swell equations as `ocean.wgsl`.
/// Returns `None` on dry land or invalid input. Decorative water outside the grid
/// uses a deep decorative bed, so its horizon has no finite surface wall.
#[must_use]
pub fn sample_surface(
    profile: &OceanSurfaceProfile,
    bed: &OceanBathymetry,
    at: Vec2,
    seconds: f32,
) -> Option<OceanSurfaceSample> {
    sample(profile, bed, at, seconds, false)
}

/// Uses known exact water occupancy at a local shore, including dry carved land.
/// The continuous wave equation remains identical to GPU displacement; an exact
/// wet hex that the coarse grid misclassifies still has its mean-height surface.
#[must_use]
pub fn sample_local_surface(
    profile: &OceanSurfaceProfile,
    bed: &OceanBathymetry,
    near: &OceanNearBoundary,
    at: Vec2,
    seconds: f32,
) -> Option<OceanSurfaceSample> {
    if !at.is_finite() {
        return None;
    }
    let coord = hex_core::HexCoord::from_world(Vec3::new(at.x, 0.0, at.y));
    if near.known_columns.contains(&coord) {
        if !near.columns.iter().any(|column| {
            column.coordinate == coord && (column.top - profile.mean_sea_level).abs() < 0.01
        }) {
            return None;
        }
        return sample(profile, bed, at, seconds, true);
    }
    sample_surface(profile, bed, at, seconds)
}

pub(super) fn sample(
    profile: &OceanSurfaceProfile,
    bed: &OceanBathymetry,
    at: Vec2,
    seconds: f32,
    exact_wet: bool,
) -> Option<OceanSurfaceSample> {
    if !profile.is_valid() || !at.is_finite() || !seconds.is_finite() {
        return None;
    }
    let (height, gradient) = bed.sample(at)?;
    let depth = profile.mean_sea_level - height;
    if depth <= 0.0 && !exact_wet {
        return None;
    }
    let t = (depth / profile.shore_depth).clamp(0.0, 1.0);
    let depth_attenuation = t * t * (3.0 - 2.0 * t);
    let depth_slope = if depth < profile.shore_depth {
        -gradient * (6.0 * t * (1.0 - t) / profile.shore_depth)
    } else {
        Vec2::ZERO
    };
    let (shelter, shelter_slope) = bed.sample_shelter(at)?;
    let attenuation = depth_attenuation * shelter;
    let slope = depth_slope * shelter + shelter_slope * depth_attenuation;
    let mut wave_height = 0.0;
    let mut wave_gradient = Vec2::ZERO;
    let mut wave_velocity = 0.0;
    let reflection = super::reflection::reflection(profile, bed, at)?;
    for wave in &profile.waves {
        let direction = wave.direction.normalize();
        let frequency = std::f32::consts::TAU / wave.wavelength;
        let phase = frequency * direction.dot(at) - std::f32::consts::TAU * seconds / wave.period
            + wave.phase_radians;
        let rate = std::f32::consts::TAU / wave.period;
        let incident = Vec4::new(
            wave.amplitude * phase.sin(),
            direction.x * wave.amplitude * frequency * phase.cos(),
            direction.y * wave.amplitude * frequency * phase.cos(),
            -rate * wave.amplitude * phase.cos(),
        );
        let blended = reflection.blend(
            incident,
            direction,
            wave.amplitude,
            frequency,
            rate,
            wave.phase_radians,
            at,
            seconds,
        );
        wave_height += blended.x;
        wave_gradient += Vec2::new(blended.y, blended.z);
        wave_velocity += blended.w;
    }
    let derivative = wave_gradient * attenuation + slope * wave_height;
    Some(OceanSurfaceSample {
        height: profile.mean_sea_level + wave_height * attenuation,
        normal: Vec3::new(-derivative.x, 1.0, -derivative.y).normalize(),
        vertical_velocity: wave_velocity * attenuation,
        depth,
        color: profile
            .shallow_color
            .lerp(profile.deep_color, (depth / 40.0).clamp(0.0, 1.0)),
    })
}

impl OceanBathymetry {
    #[expect(
        clippy::cast_precision_loss,
        reason = "Validated grid dimensions are at most2048."
    )]
    pub(super) fn contains(&self, at: Vec2) -> bool {
        let grid = (at - self.origin_xz) / self.spacing;
        at.is_finite()
            && grid.is_finite()
            && grid.min_element() >= 0.0
            && grid.x <= self.width.saturating_sub(1) as f32
            && grid.y <= self.height.saturating_sub(1) as f32
    }
    pub(super) fn sample(&self, at: Vec2) -> Option<(f32, Vec2)> {
        self.sample_values(&self.bed_heights, at, -140.0)
    }
    pub(super) fn sample_shelter(&self, at: Vec2) -> Option<(f32, Vec2)> {
        if self.shore_shelter.is_empty() {
            return Some((1.0, Vec2::ZERO));
        }
        self.sample_values(&self.shore_shelter, at, 1.0)
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "Finite coordinates are clamped to bounded texture dimensions."
    )]
    fn sample_values(&self, values: &[f32], at: Vec2, outside: f32) -> Option<(f32, Vec2)> {
        if !(2..=2048).contains(&self.width)
            || !(2..=2048).contains(&self.height)
            || !self.origin_xz.is_finite()
            || !self.spacing.is_finite()
            || self.spacing <= 0.0
            || usize::try_from(self.width.checked_mul(self.height)?).ok()? != values.len()
        {
            return None;
        }
        let position = (at - self.origin_xz) / self.spacing;
        if position.x < 0.0
            || position.y < 0.0
            || position.x > (self.width - 1) as f32
            || position.y > (self.height - 1) as f32
        {
            return Some((outside, Vec2::ZERO));
        }
        let x = (position.x.floor() as u32).min(self.width - 2);
        let z = (position.y.floor() as u32).min(self.height - 2);
        let t = position - Vec2::new(x as f32, z as f32);
        let read = |x: u32, z: u32| {
            values
                .get((z.checked_mul(self.width)?.checked_add(x)?) as usize)
                .copied()
                .filter(|height| height.is_finite())
        };
        let (a, b, c, d) = (
            read(x, z)?,
            read(x + 1, z)?,
            read(x, z + 1)?,
            read(x + 1, z + 1)?,
        );
        let row0 = a + (b - a) * t.x;
        let row1 = c + (d - c) * t.x;
        Some((
            row0 + (row1 - row0) * t.y,
            Vec2::new((b - a) + ((d - c) - (b - a)) * t.y, row1 - row0) / self.spacing,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_near_mask_preserves_zero_amplitude_wet_edges_and_dry_carves() {
        use super::super::OceanBoundaryColumn;
        use hex_core::HexCoord;
        let mut profile = OceanSurfaceProfile::default();
        for wave in &mut profile.waves {
            wave.amplitude = 0.0;
        }
        let bed = OceanBathymetry {
            bed_heights: vec![2.0; 4],
            ..default()
        };
        let mut near = OceanNearBoundary {
            known_columns: [HexCoord::ORIGIN].into_iter().collect(),
            columns: vec![OceanBoundaryColumn {
                coordinate: HexCoord::ORIGIN,
                bottom: -3.0,
                top: 0.0,
            }],
            ..default()
        };
        assert!(sample_surface(&profile, &bed, Vec2::ZERO, 0.0).is_none());
        let wet = sample_local_surface(&profile, &bed, &near, Vec2::ZERO, 0.0).unwrap();
        assert!(wet.height.abs() < 0.00001);
        assert!((wet.normal - Vec3::Y).length() < 0.00001);
        // Removing dry terrain does not add a water interval to the known column.
        near.columns.clear();
        assert!(sample_local_surface(
            &profile,
            &OceanBathymetry::default(),
            &near,
            Vec2::ZERO,
            0.0
        )
        .is_none());
    }

    #[test]
    fn sheltered_wave_normal_includes_cached_exposure_gradient() {
        let profile = OceanSurfaceProfile::default();
        let bed = OceanBathymetry {
            spacing: 100.0,
            shore_shelter: vec![0.25, 0.8, 0.4, 1.0],
            ..default()
        };
        let at = Vec2::splat(40.0);
        let center = sample_surface(&profile, &bed, at, 3.0).unwrap();
        let dx = (sample_surface(&profile, &bed, at + Vec2::X * 0.01, 3.0)
            .unwrap()
            .height
            - sample_surface(&profile, &bed, at - Vec2::X * 0.01, 3.0)
                .unwrap()
                .height)
            / 0.02;
        assert!((dx + center.normal.x / center.normal.y).abs() < 0.0001);
    }
    #[test]
    fn displacement_is_bounded_periodic_and_zero_on_shore() {
        let profile = OceanSurfaceProfile::default();
        let mut bed = OceanBathymetry::default();
        for phase in 0..900 {
            let a = sample_surface(
                &profile,
                &bed,
                Vec2::ZERO,
                f32::from(u16::try_from(phase).unwrap()),
            )
            .unwrap();
            assert!(a.height.abs() <= 2.0001);
            assert!((a.normal.length() - 1.0).abs() < 0.0001);
        }
        let a = sample_surface(&profile, &bed, Vec2::ZERO, 2.0).unwrap();
        let b = sample_surface(&profile, &bed, Vec2::ZERO, 902.0).unwrap();
        assert!((a.height - b.height).abs() < 0.0001);
        bed.bed_heights.fill(-0.001);
        assert!(
            sample_surface(&profile, &bed, Vec2::ZERO, 2.0)
                .unwrap()
                .height
                .abs()
                < 0.00001
        );
        bed.bed_heights.fill(0.0);
        assert!(sample_surface(&profile, &bed, Vec2::ZERO, 2.0).is_none());
    }
    #[test]
    fn analytic_normal_matches_spatial_height_derivative_through_shallows() {
        let profile = OceanSurfaceProfile::default();
        let bed = OceanBathymetry {
            spacing: 100.0,
            bed_heights: vec![-2.0, -10.0, -8.0, -12.0],
            ..default()
        };
        let at = Vec2::splat(40.0);
        let center = sample_surface(&profile, &bed, at, 3.0).unwrap();
        let dx = (sample_surface(&profile, &bed, at + Vec2::X * 0.01, 3.0)
            .unwrap()
            .height
            - sample_surface(&profile, &bed, at - Vec2::X * 0.01, 3.0)
                .unwrap()
                .height)
            / 0.02;
        assert!((dx + center.normal.x / center.normal.y).abs() < 0.0001);
        assert!(!OceanBathymetry {
            width: 0,
            ..default()
        }
        .is_valid());
        assert!(sample_surface(&profile, &bed, at, f32::NAN).is_none());
    }
}
