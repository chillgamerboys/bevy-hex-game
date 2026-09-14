//! Explicit settings for the isolated water experiment; no world generation here.

use bevy_ecs::prelude::Resource;

/// Identity of the built-in water fixture, independent of external packages.
pub const WATER_LAB_ID: u64 = 0x5741_5445_524c_4142;

/// Repeatable wave comparisons, all using whole-voxel height steps.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum LabWave {
    /// No displacement.
    Flat,
    /// Low ripples, usually only one voxel high.
    Gentle,
    /// A regular train of short waves.
    #[default]
    Regular,
    /// Longer, taller swells.
    Swell,
    /// Two wave trains intersecting at sixty degrees.
    Crossing,
    /// Deliberately extreme storm waves for scale and contact testing.
    Extreme,
}

/// Progressive water readability comparisons.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum LabStyle {
    /// Depth color only.
    Depth,
    /// Depth and lighter crests.
    Crests,
    /// Depth offshore, nearshore crests, and moving shimmer throughout.
    #[default]
    Patterns,
}

/// Deterministic wind comparisons.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum LabWind {
    /// No wind.
    Calm,
    /// Constant nine units per second.
    #[default]
    Steady,
    /// Constant twenty units per second.
    Strong,
    /// Six to twelve units per second.
    Gusts,
    /// Gusts plus gradual direction changes.
    Turning,
    /// Turning gusts with a sheltered cove.
    Shelter,
}

/// Game-owned experimental controls, separate from ordinary-map tuning.
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct WaterLabSettings {
    /// Selected wave comparison.
    pub wave: LabWave,
    /// Selected visual comparison.
    pub style: LabStyle,
    /// Selected wind comparison.
    pub wind: LabWind,
    /// Lab-only fraction of environmental wind used by gliding.
    pub glider_wind_scale: f32,
    /// Phase subtracted from the shared wave and timed-wind clock.
    pub phase_origin: f32,
    /// Frozen environmental phase; player movement remains live.
    pub frozen_phase: Option<f32>,
}

impl Default for WaterLabSettings {
    fn default() -> Self {
        Self {
            wave: LabWave::Regular,
            style: LabStyle::Patterns,
            wind: LabWind::Steady,
            glider_wind_scale: 1.0,
            phase_origin: 0.0,
            frozen_phase: None,
        }
    }
}

impl WaterLabSettings {
    /// Wave phase derived from the same completed tick used by contact queries.
    #[must_use]
    pub fn phase(self, seconds: f32) -> f32 {
        self.frozen_phase
            .unwrap_or((seconds - self.phase_origin).rem_euclid(900.0))
    }
}

/// Named gameplay-owned reset poses for repeatable short routes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabStart {
    /// Supported dry beach start.
    Shore,
    /// Neutral floating position.
    Swim,
    /// Portable boat deployed at the swim anchor.
    Boat,
    /// An open glider with repeatable forward momentum.
    Glider,
}
