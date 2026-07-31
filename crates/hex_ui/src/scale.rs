use bevy::prelude::*;
use bevy::ui::UiScale;
use serde::{Deserialize, Serialize};

/// Persistable UI scale choice.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UiScaleMode {
    /// Resolve from the window's logical size without overriding OS DPI scaling.
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
    /// Returns the manual scale factor, or `None` for automatic scaling.
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

/// Application-owned preference projected into the renderer.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct UiScalePreference(pub UiScaleMode);

/// Responsive layout class after global UI scaling is applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiViewportClass {
    /// Constrained layout: secondary panes collapse before primary actions.
    Compact,
    /// Ordinary 16:9 desktop layout.
    Standard,
    /// Extra horizontal room suitable for persistent secondary panes.
    Wide,
}

/// Current resolved scale and effective canvas.
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct ResolvedUiMetrics {
    /// Scale written to Bevy's global [`UiScale`].
    pub scale: f32,
    /// Logical canvas after dividing by the global UI scale.
    pub effective_size: Vec2,
    /// Responsive class derived from [`Self::effective_size`].
    pub viewport: UiViewportClass,
}

impl Default for ResolvedUiMetrics {
    fn default() -> Self {
        Self {
            scale: 1.0,
            effective_size: Vec2::new(1920.0, 1080.0),
            viewport: UiViewportClass::Standard,
        }
    }
}

/// Resolves automatic scaling from logical window dimensions.
#[must_use]
pub fn resolve_auto_scale(logical_size: Vec2) -> f32 {
    (logical_size.x / 1920.0)
        .min(logical_size.y / 1080.0)
        .clamp(1.0, 2.0)
}

/// Classifies the effective post-scale canvas.
#[must_use]
pub fn resolve_viewport_class(effective_size: Vec2) -> UiViewportClass {
    if effective_size.x < 1600.0 || effective_size.y < 900.0 {
        UiViewportClass::Compact
    } else if effective_size.x >= 2400.0 {
        UiViewportClass::Wide
    } else {
        UiViewportClass::Standard
    }
}

/// Resolves global scale, effective canvas, and responsive class without a renderer.
#[must_use]
pub fn resolve_ui_metrics(logical_size: Vec2, mode: UiScaleMode) -> ResolvedUiMetrics {
    let scale = mode
        .manual_factor()
        .unwrap_or_else(|| resolve_auto_scale(logical_size));
    let effective_size = logical_size / scale;
    ResolvedUiMetrics {
        scale,
        effective_size,
        viewport: resolve_viewport_class(effective_size),
    }
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<UiScalePreference>()
        .init_resource::<ResolvedUiMetrics>()
        .add_systems(Update, apply_ui_scale);
}

fn apply_ui_scale(
    preference: Res<UiScalePreference>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut ui_scale: ResMut<UiScale>,
    mut metrics: ResMut<ResolvedUiMetrics>,
) {
    let Ok(window) = windows.single() else { return };
    let logical_size = Vec2::new(window.width(), window.height());
    let next = resolve_ui_metrics(logical_size, preference.0);
    let scale = next.scale;
    if *metrics != next {
        *metrics = next;
    }
    if (ui_scale.0 - scale).abs() > f32::EPSILON {
        ui_scale.0 = scale;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_scale_keeps_a_native_1080p_effective_canvas() {
        for (size, expected) in [
            (Vec2::new(960.0, 540.0), 1.0),
            (Vec2::new(1280.0, 720.0), 1.0),
            (Vec2::new(1920.0, 1080.0), 1.0),
            (Vec2::new(2560.0, 1440.0), 4.0 / 3.0),
            (Vec2::new(3840.0, 2160.0), 2.0),
        ] {
            assert!((resolve_auto_scale(size) - expected).abs() < 0.001);
        }
    }

    #[test]
    fn responsive_classes_follow_the_post_scale_canvas() {
        assert_eq!(
            resolve_viewport_class(Vec2::new(1280.0, 720.0)),
            UiViewportClass::Compact
        );
        assert_eq!(
            resolve_viewport_class(Vec2::new(1920.0, 1080.0)),
            UiViewportClass::Standard
        );
        assert_eq!(
            resolve_viewport_class(Vec2::new(2560.0, 1080.0)),
            UiViewportClass::Wide
        );
    }

    #[test]
    fn manual_scale_replaces_auto_and_reflows() {
        let logical = Vec2::new(1920.0, 1080.0);
        let scale = UiScaleMode::Percent200.manual_factor().unwrap_or(1.0);
        let effective = logical / scale;
        assert_eq!(resolve_viewport_class(effective), UiViewportClass::Compact);
    }

    #[test]
    fn structural_matrix_resolves_every_required_viewport_without_invalid_geometry() {
        for logical_size in [
            Vec2::new(960.0, 540.0),
            Vec2::new(1280.0, 720.0),
            Vec2::new(1920.0, 1080.0),
            Vec2::new(2560.0, 1440.0),
            Vec2::new(3840.0, 2160.0),
        ] {
            for mode in [UiScaleMode::Auto, UiScaleMode::Percent200] {
                let metrics = resolve_ui_metrics(logical_size, mode);
                assert!(metrics.scale.is_finite() && metrics.scale > 0.0);
                assert!(metrics.effective_size.is_finite());
                assert!(metrics.effective_size.cmpgt(Vec2::ZERO).all());
                if mode == UiScaleMode::Percent200 {
                    assert!((metrics.scale - 2.0).abs() <= f32::EPSILON);
                }
            }
        }
    }
}
