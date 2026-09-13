//! A camera-centered ocean with world-coherent, presentation-only swells.
//!
//! The host publishes immutable bathymetry and explicit visual time. Neither
//! displacement nor camera tint changes voxel occupancy or actor motion.
mod boundary;
mod mesh;
mod render;
mod sample;

use bevy::prelude::*;

pub use boundary::{OceanBoundaryColumn, OceanNearBoundary};
pub use render::{install, OceanRenderStatus};
pub use sample::{sample_surface, OceanSurfaceSample};

/// One analytic directional swell, with world-unit amplitude and wavelength.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OceanWave {
    /// Horizontal propagation direction, normalized when sampled.
    pub direction: Vec2,
    /// Maximum vertical contribution in world units.
    pub amplitude: f32,
    /// Distance between crests in world units.
    pub wavelength: f32,
    /// Seconds for one full oscillation.
    pub period: f32,
    /// Phase at the world origin and time zero, in radians.
    pub phase_radians: f32,
}

/// World-owned visual ocean profile; ordinary liquids keep zero displacement.
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct OceanSurfaceProfile {
    /// Static physical sea surface, also the displacement's mean height.
    pub mean_sea_level: f32,
    /// The three approved slow swells.
    pub waves: [OceanWave; 3],
    /// Positive depth at which swells reach their full amplitude.
    pub shore_depth: f32,
    /// Shallow-water linear RGBA, before lighting and fog.
    pub shallow_color: Vec4,
    /// Deep-water linear RGBA, before lighting and fog.
    pub deep_color: Vec4,
}

impl Default for OceanSurfaceProfile {
    fn default() -> Self {
        Self {
            mean_sea_level: 0.0,
            waves: [
                OceanWave {
                    direction: Vec2::new(0.94, 0.34),
                    amplitude: 1.2,
                    wavelength: 110.0,
                    period: 18.0,
                    phase_radians: 0.0,
                },
                OceanWave {
                    direction: Vec2::new(-0.35, 0.94),
                    amplitude: 0.6,
                    wavelength: 180.0,
                    period: 25.0,
                    phase_radians: 1.3,
                },
                OceanWave {
                    direction: Vec2::new(0.60, -0.80),
                    amplitude: 0.2,
                    wavelength: 60.0,
                    period: 12.0,
                    phase_radians: 2.4,
                },
            ],
            shore_depth: 12.0,
            shallow_color: Vec4::new(0.035, 0.18, 0.23, 0.62),
            deep_color: Vec4::new(0.008, 0.047, 0.090, 0.88),
        }
    }
}

impl OceanSurfaceProfile {
    /// Rejects invalid uniforms before publishing a material or sampling a camera.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.mean_sea_level.is_finite()
            && self.shore_depth.is_finite()
            && self.shore_depth > 0.0
            && self.shallow_color.is_finite()
            && self.deep_color.is_finite()
            && self.waves.iter().all(|wave| {
                wave.direction.is_finite()
                    && wave.direction.length_squared() > 0.0
                    && wave.amplitude.is_finite()
                    && wave.amplitude >= 0.0
                    && wave.wavelength.is_finite()
                    && wave.wavelength > 0.0
                    && wave.period.is_finite()
                    && wave.period > 0.0
                    && wave.phase_radians.is_finite()
            })
    }
}

/// Regular, row-major world X/Z bathymetry sampled by both CPU and GPU.
/// Heights describe the initial water footprint; live carving never simulates
/// filling new columns. Increment `revision` after replacing this data.
#[derive(Resource, Debug, Clone)]
pub struct OceanBathymetry {
    /// Identity of this exact height-grid publication.
    pub revision: u64,
    /// World X/Z of sample (0,0).
    pub origin_xz: Vec2,
    /// Positive world-unit separation between neighboring samples.
    pub spacing: f32,
    /// Samples along X.
    pub width: u32,
    /// Samples along Z.
    pub height: u32,
    /// Initial bed heights in world Y, Z-major then X.
    pub bed_heights: Vec<f32>,
}

impl Default for OceanBathymetry {
    fn default() -> Self {
        Self {
            revision: 0,
            origin_xz: Vec2::ZERO,
            spacing: 1.0,
            width: 2,
            height: 2,
            bed_heights: vec![-140.0; 4],
        }
    }
}

impl OceanBathymetry {
    /// Checks bounded texture dimensions, exact sample count and finite heights.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.origin_xz.is_finite()
            && self.spacing.is_finite()
            && self.spacing > 0.0
            && (2..=2048).contains(&self.width)
            && (2..=2048).contains(&self.height)
            && self
                .width
                .checked_mul(self.height)
                .and_then(|n| usize::try_from(n).ok())
                == Some(self.bed_heights.len())
            && self.bed_heights.iter().all(|height| height.is_finite())
    }
}

/// Integration-owned camera and clock inputs; changing these never rebuilds meshes.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct OceanFrame {
    /// Enable only for a map that publishes an ocean profile.
    pub enabled: bool,
    /// Actual world-space camera; the bounded mesh follows its horizontal position.
    pub camera_position: Vec3,
    /// Explicit seconds, frozen for review. The approved periods repeat at900s.
    /// Do not feed the legacy liquid clock's incompatible400-second wrap.
    pub phase_seconds: f32,
}
