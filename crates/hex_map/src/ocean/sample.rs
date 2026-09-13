use super::{OceanBathymetry, OceanSurfaceProfile};
use bevy::prelude::*;

/// Visual height, normal and color at a world position; never collision authority.
#[derive(Debug, Clone, Copy)]
pub struct OceanSurfaceSample {
    /// Displaced water height in world units.
    pub height: f32,
    /// Analytic upward normal including shoreline attenuation.
    pub normal: Vec3,
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
    if !profile.is_valid() || !at.is_finite() || !seconds.is_finite() {
        return None;
    }
    let (height, gradient) = bed.sample(at)?;
    let depth = profile.mean_sea_level - height;
    if depth <= 0.0 {
        return None;
    }
    let t = (depth / profile.shore_depth).clamp(0.0, 1.0);
    let attenuation = t * t * (3.0 - 2.0 * t);
    let slope = if depth < profile.shore_depth {
        -gradient * (6.0 * t * (1.0 - t) / profile.shore_depth)
    } else {
        Vec2::ZERO
    };
    let mut wave_height = 0.0;
    let mut wave_gradient = Vec2::ZERO;
    for wave in &profile.waves {
        let direction = wave.direction.normalize();
        let frequency = std::f32::consts::TAU / wave.wavelength;
        let phase = frequency * direction.dot(at) - std::f32::consts::TAU * seconds / wave.period;
        wave_height += wave.amplitude * phase.sin();
        wave_gradient += direction * (wave.amplitude * frequency * phase.cos());
    }
    let derivative = wave_gradient * attenuation + slope * wave_height;
    Some(OceanSurfaceSample {
        height: profile.mean_sea_level + wave_height * attenuation,
        normal: Vec3::new(-derivative.x, 1.0, -derivative.y).normalize(),
        depth,
        color: profile
            .shallow_color
            .lerp(profile.deep_color, (depth / 40.0).clamp(0.0, 1.0)),
    })
}

impl OceanBathymetry {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "Finite grid coordinates are clamped to validated bounded texture dimensions before integer conversion."
    )]
    pub(super) fn sample(&self, at: Vec2) -> Option<(f32, Vec2)> {
        if !(2..=2048).contains(&self.width)
            || !(2..=2048).contains(&self.height)
            || !self.origin_xz.is_finite()
            || !self.spacing.is_finite()
            || self.spacing <= 0.0
            || usize::try_from(self.width.checked_mul(self.height)?).ok()? != self.bed_heights.len()
        {
            return None;
        }
        let position = (at - self.origin_xz) / self.spacing;
        if position.x < 0.0
            || position.y < 0.0
            || position.x > (self.width - 1) as f32
            || position.y > (self.height - 1) as f32
        {
            return Some((-140.0, Vec2::ZERO));
        }
        let x = (position.x.floor() as u32).min(self.width - 2);
        let z = (position.y.floor() as u32).min(self.height - 2);
        let t = position - Vec2::new(x as f32, z as f32);
        let read = |x: u32, z: u32| {
            self.bed_heights
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
