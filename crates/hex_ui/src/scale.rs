use bevy::prelude::*;
use bevy::ui::UiScale;
use serde::{Deserialize, Serialize};

/// Persistable UI scale choice.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UiScaleMode {
    /// Resolve content size from the logical canvas. OS DPI remains authoritative.
    #[default]
    Auto,
    /// Seventy-five percent of the baseline.
    Percent75,
    /// Baseline size.
    Percent100,
    /// One hundred twenty-five percent.
    Percent125,
    /// One hundred fifty percent.
    Percent150,
    /// One hundred seventy-five percent.
    Percent175,
    /// Two hundred percent.
    Percent200,
}

impl UiScaleMode {
    /// Returns the manual content factor, or `None` for automatic sizing.
    #[must_use]
    pub const fn manual_factor(self) -> Option<f32> {
        match self {
            Self::Auto => None,
            Self::Percent75 => Some(0.75),
            Self::Percent100 => Some(1.0),
            Self::Percent125 => Some(1.25),
            Self::Percent150 => Some(1.5),
            Self::Percent175 => Some(1.75),
            Self::Percent200 => Some(2.0),
        }
    }

    /// Cycles through Auto and every supported explicit scale.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Auto => Self::Percent75,
            Self::Percent75 => Self::Percent100,
            Self::Percent100 => Self::Percent125,
            Self::Percent125 => Self::Percent150,
            Self::Percent150 => Self::Percent175,
            Self::Percent175 => Self::Percent200,
            Self::Percent200 => Self::Auto,
        }
    }

    /// Player-facing setting label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Percent75 => "75%",
            Self::Percent100 => "100%",
            Self::Percent125 => "125%",
            Self::Percent150 => "150%",
            Self::Percent175 => "175%",
            Self::Percent200 => "200%",
        }
    }
}

/// Application-owned preference projected into semantic presentation tokens.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct UiScalePreference(pub UiScaleMode);

/// Exact logical canvas and raster density used by deterministic visual review.
#[cfg(any(feature = "visual-review", feature = "test-support"))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReviewViewport {
    /// UI layout size in logical pixels.
    pub logical_size: UVec2,
    /// Physical pixels rendered for each logical pixel.
    pub device_scale: f32,
}

#[cfg(any(feature = "visual-review", feature = "test-support"))]
impl ReviewViewport {
    /// Baseline 1920×1080 review canvas at one physical pixel per logical pixel.
    pub const DEFAULT: Self = Self {
        logical_size: UVec2::new(1920, 1080),
        device_scale: 1.0,
    };

    /// Validates and constructs one review viewport.
    pub fn new(width: u32, height: u32, device_scale: f32) -> Result<Self, String> {
        if width == 0 || height == 0 {
            return Err("review viewport dimensions must be positive".to_owned());
        }
        if !device_scale.is_finite() || !(1.0..=4.0).contains(&device_scale) {
            return Err("review viewport device scale must be finite and in 1.0..=4.0".to_owned());
        }
        let viewport = Self {
            logical_size: UVec2::new(width, height),
            device_scale,
        };
        viewport.physical_size()?;
        Ok(viewport)
    }

    /// Physical image dimensions required for this viewport.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "validated positive rounded dimensions are bounded by u32::MAX immediately before conversion"
    )]
    pub fn physical_size(self) -> Result<UVec2, String> {
        let width = (self.logical_size.x as f64 * f64::from(self.device_scale)).round();
        let height = (self.logical_size.y as f64 * f64::from(self.device_scale)).round();
        if width > f64::from(u32::MAX) || height > f64::from(u32::MAX) {
            return Err("review viewport physical size overflows u32".to_owned());
        }
        Ok(UVec2::new(width as u32, height as u32))
    }
}

/// Responsive layout class after semantic density adjustment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiViewportClass {
    /// Constrained layout: secondary panes collapse before primary actions.
    Compact,
    /// Ordinary desktop layout.
    Standard,
    /// Extra horizontal room suitable for persistent secondary panes.
    Wide,
}

/// Internal order for resolving semantic metrics before applying them to widgets.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SemanticMetricsSystems {
    /// Resolve the current preference and logical canvas into semantic factors.
    Resolve,
    /// Apply resolved factors to opted-in text and control geometry.
    Apply,
}

/// Current logical canvas and semantic accessibility factors.
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct ResolvedUiMetrics {
    /// Window or review-target size in logical pixels.
    pub logical_size: Vec2,
    /// Full factor applied to essential body and control typography.
    pub content_scale: f32,
    /// Moderated factor applied to display, title, and heading typography.
    pub heading_scale: f32,
    /// Moderated factor applied to control geometry, never below the authored
    /// baseline so interactive targets remain at least 44×44 logical pixels.
    pub control_scale: f32,
    /// Lightly moderated factor applied to layout density and shared spacing.
    pub spacing_scale: f32,
    /// Logical canvas adjusted only for semantic spacing pressure.
    pub effective_size: Vec2,
    /// Responsive class derived from [`Self::effective_size`].
    pub viewport: UiViewportClass,
}

impl Default for ResolvedUiMetrics {
    fn default() -> Self {
        resolve_ui_metrics(Vec2::new(1920.0, 1080.0), UiScaleMode::Auto)
    }
}

/// Resolves automatic content scale from logical dimensions.
#[must_use]
pub fn resolve_auto_scale(logical_size: Vec2) -> f32 {
    (logical_size.x / 1920.0)
        .min(logical_size.y / 1080.0)
        .clamp(1.0, 1.5)
}

/// Classifies a semantic-density-adjusted logical canvas.
#[must_use]
pub fn resolve_viewport_class(effective_size: Vec2) -> UiViewportClass {
    if effective_size.x < 1440.0 || effective_size.y < 810.0 {
        UiViewportClass::Compact
    } else if effective_size.x >= 2400.0 {
        UiViewportClass::Wide
    } else {
        UiViewportClass::Standard
    }
}

/// Resolves semantic content, control, spacing, and responsive metrics.
#[must_use]
pub fn resolve_ui_metrics(logical_size: Vec2, mode: UiScaleMode) -> ResolvedUiMetrics {
    let content_scale = mode
        .manual_factor()
        .unwrap_or_else(|| resolve_auto_scale(logical_size));
    let delta = content_scale - 1.0;
    let heading_scale = (1.0 + 0.5 * delta).clamp(0.875, 1.5);
    let control_scale = (1.0 + 0.5 * delta).clamp(1.0, 1.5);
    let spacing_scale = (1.0 + 0.25 * delta).clamp(0.9375, 1.25);
    let effective_size = logical_size / spacing_scale;
    ResolvedUiMetrics {
        logical_size,
        content_scale,
        heading_scale,
        control_scale,
        spacing_scale,
        effective_size,
        viewport: resolve_viewport_class(effective_size),
    }
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<UiScalePreference>()
        .init_resource::<ResolvedUiMetrics>()
        .configure_sets(
            Update,
            (SemanticMetricsSystems::Resolve, crate::UiSystems::Render).chain(),
        )
        .configure_sets(
            PostUpdate,
            SemanticMetricsSystems::Apply.before(bevy::ui::UiSystems::Prepare),
        )
        .add_systems(
            Update,
            apply_ui_scale.in_set(SemanticMetricsSystems::Resolve),
        );
}

pub(crate) fn apply_ui_scale(
    preference: Res<UiScalePreference>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut ui_scale: ResMut<UiScale>,
    mut metrics: ResMut<ResolvedUiMetrics>,
) {
    let Ok(window) = windows.single() else { return };
    let logical_size = Vec2::new(window.width(), window.height());
    let next = resolve_ui_metrics(logical_size, preference.0);
    if *metrics != next {
        *metrics = next;
    }
    if (ui_scale.0 - 1.0).abs() > f32::EPSILON {
        ui_scale.0 = 1.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_scale_uses_logical_canvas_and_caps_at_one_point_five() {
        for (size, expected) in [
            (Vec2::new(960.0, 540.0), 1.0),
            (Vec2::new(1280.0, 720.0), 1.0),
            (Vec2::new(1920.0, 1080.0), 1.0),
            (Vec2::new(2560.0, 1440.0), 4.0 / 3.0),
            (Vec2::new(3840.0, 2160.0), 1.5),
        ] {
            assert!((resolve_auto_scale(size) - expected).abs() < 0.001);
        }
    }

    #[test]
    fn responsive_classes_use_semantic_density_not_device_pixels() {
        assert_eq!(
            resolve_ui_metrics(Vec2::new(1280.0, 720.0), UiScaleMode::Auto).viewport,
            UiViewportClass::Compact
        );
        assert_eq!(
            resolve_ui_metrics(Vec2::new(1512.0, 949.0), UiScaleMode::Auto).viewport,
            UiViewportClass::Standard
        );
        assert_eq!(
            resolve_ui_metrics(Vec2::new(3840.0, 2160.0), UiScaleMode::Auto).viewport,
            UiViewportClass::Wide
        );
    }

    #[test]
    fn two_hundred_percent_keeps_panel_density_moderated() {
        let metrics = resolve_ui_metrics(Vec2::new(1920.0, 1080.0), UiScaleMode::Percent200);
        assert!((metrics.content_scale - 2.0).abs() <= f32::EPSILON);
        assert!((metrics.heading_scale - 1.5).abs() <= f32::EPSILON);
        assert!((metrics.control_scale - 1.5).abs() <= f32::EPSILON);
        assert!((metrics.spacing_scale - 1.25).abs() <= f32::EPSILON);
        assert_eq!(metrics.viewport, UiViewportClass::Standard);
    }

    #[test]
    fn seventy_five_percent_preserves_control_target_baselines() {
        let metrics = resolve_ui_metrics(Vec2::new(1920.0, 1080.0), UiScaleMode::Percent75);
        assert!((metrics.content_scale - 0.75).abs() <= f32::EPSILON);
        assert!((metrics.heading_scale - 0.875).abs() <= f32::EPSILON);
        assert!((metrics.control_scale - 1.0).abs() <= f32::EPSILON);
        assert!((metrics.spacing_scale - 0.9375).abs() <= f32::EPSILON);
    }

    #[test]
    fn every_scale_mode_resolves_finite_metrics() {
        for logical_size in [
            Vec2::new(960.0, 540.0),
            Vec2::new(1280.0, 720.0),
            Vec2::new(1512.0, 949.0),
            Vec2::new(1920.0, 1080.0),
            Vec2::new(2560.0, 1440.0),
            Vec2::new(3840.0, 2160.0),
        ] {
            for mode in [
                UiScaleMode::Auto,
                UiScaleMode::Percent75,
                UiScaleMode::Percent100,
                UiScaleMode::Percent125,
                UiScaleMode::Percent150,
                UiScaleMode::Percent175,
                UiScaleMode::Percent200,
            ] {
                let metrics = resolve_ui_metrics(logical_size, mode);
                assert!(metrics.content_scale.is_finite() && metrics.content_scale > 0.0);
                assert!(metrics.effective_size.is_finite());
                assert!(metrics.effective_size.cmpgt(Vec2::ZERO).all());
            }
        }
    }
}
