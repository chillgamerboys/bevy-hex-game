//! Responsive placement for persistent gameplay regions.

use bevy::prelude::*;

use crate::UiViewportClass;

/// Semantic role of one persistent gameplay region.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiRegionRole {
    /// Party identity and selection.
    Party,
    /// Current actor and initiative.
    Turn,
    /// Inspectable target details.
    Inspector,
    /// Casting and turn actions above the persistent rail.
    Actions,
    /// Event feed and expanded history.
    Events,
}

/// Ordinary gameplay chrome controlled by the composition root's HUD preference.
#[derive(Component)]
pub struct HudElement;

/// Command-modal gameplay surface that remains visible when ordinary chrome is hidden.
#[derive(Component)]
pub struct RequiredActionSurface;

/// Picking policy for read-only HUD surfaces above the native world.
pub const READ_ONLY_HUD: Pickable = Pickable::IGNORE;

/// Logical space reserved above the persistent action rail.
#[must_use]
pub const fn action_rail_clearance(viewport: UiViewportClass) -> f32 {
    match viewport {
        UiViewportClass::Compact => 180.0,
        UiViewportClass::Standard => 164.0,
        UiViewportClass::Wide => 168.0,
    }
}

/// Height needed by the horizontal casting strip as semantic controls grow.
/// Enlarged text wraps before a simple proportional control scale would account
/// for it, while Auto at ordinary desktop sizes keeps the compact baseline.
pub(crate) fn semantic_action_region_height(metrics: crate::ResolvedUiMetrics) -> f32 {
    92.0 + 160.0 * (metrics.control_scale - 1.0).max(0.0)
        + 32.0 * (metrics.content_scale - 1.0).max(0.0)
}

/// Height needed by the controls-only Action Bar as semantic controls grow.
///
/// The tallest ordinary action is a disabled control with a visible refusal
/// reason. Its two lines grow faster than the authored 92px baseline, so the
/// bar and its parent region must reserve the same additional space.
pub(crate) fn semantic_action_bar_height(metrics: crate::ResolvedUiMetrics) -> f32 {
    92.0 + 116.0 * (metrics.control_scale - 1.0).max(0.0)
}

fn semantic_party_region_width(metrics: crate::ResolvedUiMetrics) -> f32 {
    let base = match metrics.viewport {
        UiViewportClass::Compact => 0.0,
        UiViewportClass::Standard => 216.0,
        UiViewportClass::Wide => 240.0,
    };
    base + 136.0 * (metrics.control_scale - 1.0).max(0.0)
}

fn semantic_activity_region_width(metrics: crate::ResolvedUiMetrics) -> f32 {
    let base = match metrics.viewport {
        UiViewportClass::Compact => 0.0,
        UiViewportClass::Standard => 264.0,
        UiViewportClass::Wide => 308.0,
    };
    // Three stable text tabs remain horizontal on desktop. Give each one the
    // same additional label width as body typography grows.
    base + 144.0 * (metrics.content_scale - 1.0).max(0.0)
}

pub(crate) fn action_region_height(metrics: crate::ResolvedUiMetrics) -> f32 {
    let semantic = semantic_action_region_height(metrics);
    if metrics.viewport == UiViewportClass::Compact
        && !is_ultra_constrained(metrics)
        && metrics.content_scale < 1.5
    {
        semantic.max(132.0)
    } else {
        semantic
    }
}

pub(crate) fn inspector_width(metrics: crate::ResolvedUiMetrics) -> f32 {
    let base_width: f32 = match metrics.viewport {
        UiViewportClass::Compact => 272.0,
        UiViewportClass::Standard => 300.0,
        UiViewportClass::Wide => 332.0,
    };
    base_width.max(250.0 * metrics.control_scale.max(1.0) + 22.0)
}

pub(crate) fn center_right_inset(metrics: crate::ResolvedUiMetrics) -> f32 {
    match metrics.viewport {
        UiViewportClass::Compact => inspector_width(metrics) + 20.0,
        UiViewportClass::Standard => inspector_width(metrics) + 20.0,
        UiViewportClass::Wide => inspector_width(metrics) + 28.0,
    }
}

/// Applies a complete, reversible layout for one responsive region.
pub fn apply_region_layout(viewport: UiViewportClass, role: UiRegionRole, node: &mut Node) {
    node.display = Display::Flex;
    node.top = Val::Auto;
    node.right = Val::Auto;
    node.bottom = Val::Auto;
    node.left = Val::Auto;
    node.width = Val::Auto;
    node.height = Val::Auto;
    match (viewport, role) {
        (UiViewportClass::Compact, UiRegionRole::Party) => {
            node.display = Display::None;
        }
        (UiViewportClass::Compact, UiRegionRole::Turn) => {
            node.display = Display::None;
        }
        (UiViewportClass::Compact, UiRegionRole::Inspector | UiRegionRole::Events) => {
            node.display = Display::None;
        }
        (UiViewportClass::Compact, UiRegionRole::Actions) => {
            node.display = Display::None;
        }
        (UiViewportClass::Standard, UiRegionRole::Party) => {
            node.top = Val::Px(88.0);
            node.bottom = Val::Px(16.0);
            node.left = Val::Px(16.0);
            node.width = Val::Px(216.0);
        }
        (UiViewportClass::Standard, UiRegionRole::Turn) => {
            node.top = Val::Px(16.0);
            node.left = Val::Px(248.0);
            node.right = Val::Px(16.0);
            node.height = Val::Px(60.0);
        }
        (UiViewportClass::Standard, UiRegionRole::Inspector) => {
            node.top = Val::Px(88.0);
            node.right = Val::Px(296.0);
            node.bottom = Val::Px(216.0);
            node.left = Val::Px(248.0);
        }
        (UiViewportClass::Standard, UiRegionRole::Actions) => {
            node.left = Val::Px(248.0);
            node.right = Val::Px(296.0);
            node.bottom = Val::Px(16.0);
            node.height = Val::Px(184.0);
        }
        (UiViewportClass::Standard, UiRegionRole::Events) => {
            node.top = Val::Px(88.0);
            node.right = Val::Px(16.0);
            node.bottom = Val::Px(16.0);
            node.width = Val::Px(264.0);
        }
        (UiViewportClass::Wide, UiRegionRole::Party) => {
            node.top = Val::Px(92.0);
            node.bottom = Val::Px(16.0);
            node.left = Val::Px(16.0);
            node.width = Val::Px(240.0);
        }
        (UiViewportClass::Wide, UiRegionRole::Turn) => {
            node.top = Val::Px(16.0);
            node.left = Val::Px(272.0);
            node.right = Val::Px(16.0);
            node.height = Val::Px(64.0);
        }
        (UiViewportClass::Wide, UiRegionRole::Inspector) => {
            node.top = Val::Px(92.0);
            node.right = Val::Px(340.0);
            node.bottom = Val::Px(220.0);
            node.left = Val::Px(272.0);
        }
        (UiViewportClass::Wide, UiRegionRole::Actions) => {
            node.left = Val::Px(272.0);
            node.right = Val::Px(340.0);
            node.bottom = Val::Px(16.0);
            node.height = Val::Px(188.0);
        }
        (UiViewportClass::Wide, UiRegionRole::Events) => {
            node.top = Val::Px(92.0);
            node.right = Val::Px(16.0);
            node.bottom = Val::Px(16.0);
            node.width = Val::Px(308.0);
        }
    }
}

/// Applies effective-canvas constraints after the coarse viewport layout.
pub(crate) fn constrain_region_to_canvas(
    metrics: crate::ResolvedUiMetrics,
    role: UiRegionRole,
    node: &mut Node,
) {
    apply_region_layout(metrics.viewport, role, node);
    let base_party_width = match metrics.viewport {
        UiViewportClass::Compact => 0.0,
        UiViewportClass::Standard => 216.0,
        UiViewportClass::Wide => 240.0,
    };
    let party_width = semantic_party_region_width(metrics);
    let party_growth = (party_width - base_party_width).max(0.0);
    let base_activity_width = match metrics.viewport {
        UiViewportClass::Compact => 0.0,
        UiViewportClass::Standard => 264.0,
        UiViewportClass::Wide => 308.0,
    };
    let activity_width = semantic_activity_region_width(metrics);
    let activity_growth = (activity_width - base_activity_width).max(0.0);
    let action_growth = (semantic_action_region_height(metrics) - 92.0).max(0.0)
        + (semantic_action_bar_height(metrics) - 92.0).max(0.0);
    match (metrics.viewport, role) {
        (UiViewportClass::Standard | UiViewportClass::Wide, UiRegionRole::Party) => {
            node.width = Val::Px(party_width);
        }
        (UiViewportClass::Standard | UiViewportClass::Wide, UiRegionRole::Turn) => {
            let Val::Px(left) = node.left else {
                return;
            };
            node.left = Val::Px(left + party_growth);
        }
        (UiViewportClass::Standard, UiRegionRole::Inspector) => {
            node.left = Val::Px(248.0 + party_growth);
            node.right = Val::Px(296.0 + activity_growth);
            node.bottom = Val::Px(216.0 + action_growth);
        }
        (UiViewportClass::Standard, UiRegionRole::Actions) => {
            node.left = Val::Px(248.0 + party_growth);
            node.right = Val::Px(296.0 + activity_growth);
            node.height = Val::Px(184.0 + action_growth);
        }
        (UiViewportClass::Standard | UiViewportClass::Wide, UiRegionRole::Events) => {
            node.width = Val::Px(activity_width);
        }
        (UiViewportClass::Wide, UiRegionRole::Inspector) => {
            node.left = Val::Px(272.0 + party_growth);
            node.right = Val::Px(340.0 + activity_growth);
            node.bottom = Val::Px(220.0 + action_growth);
        }
        (UiViewportClass::Wide, UiRegionRole::Actions) => {
            node.left = Val::Px(272.0 + party_growth);
            node.right = Val::Px(340.0 + activity_growth);
            node.height = Val::Px(188.0 + action_growth);
        }
        _ => {}
    }
}

pub(crate) fn is_ultra_constrained(metrics: crate::ResolvedUiMetrics) -> bool {
    metrics.viewport == UiViewportClass::Compact
        && (metrics.logical_size.y <= 540.0 || metrics.effective_size.y < 700.0)
}

pub(crate) fn ultra_action_rail_height(metrics: crate::ResolvedUiMetrics) -> f32 {
    // The minimalist bar owns controls only. Its former multi-line summary and
    // prompt allowance left a mostly empty slab that pushed the actual buttons
    // below Compact's initial viewport at enlarged scales.
    semantic_action_bar_height(metrics)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coarse_compact_layout_starts_with_every_hud_region_fully_suppressed() {
        for role in [
            UiRegionRole::Party,
            UiRegionRole::Turn,
            UiRegionRole::Inspector,
            UiRegionRole::Actions,
            UiRegionRole::Events,
        ] {
            let mut node = Node::default();
            apply_region_layout(UiViewportClass::Compact, role, &mut node);
            assert_eq!(node.display, Display::None, "role={role:?}");
            assert_eq!(node.top, Val::Auto);
            assert_eq!(node.right, Val::Auto);
            assert_eq!(node.bottom, Val::Auto);
            assert_eq!(node.left, Val::Auto);
        }
    }

    #[test]
    fn compact_constraints_do_not_reserve_a_hidden_inspector_or_action_lane() {
        let metrics = crate::resolve_ui_metrics(Vec2::new(1291.0, 747.0), crate::UiScaleMode::Auto);
        assert_eq!(metrics.viewport, UiViewportClass::Compact);
        assert!(!is_ultra_constrained(metrics));

        let mut inspector = Node::default();
        let mut actions = Node::default();
        constrain_region_to_canvas(metrics, UiRegionRole::Inspector, &mut inspector);
        constrain_region_to_canvas(metrics, UiRegionRole::Actions, &mut actions);
        assert_eq!(inspector.display, Display::None);
        assert_eq!(actions.display, Display::None);
        assert_eq!(inspector.width, Val::Auto);
        assert_eq!(actions.width, Val::Auto);
    }

    #[test]
    fn ultra_constrained_canvas_also_starts_without_drawers_or_reserved_lanes() {
        let metrics = crate::ResolvedUiMetrics {
            logical_size: Vec2::new(960.0, 540.0),
            content_scale: 2.0,
            heading_scale: 1.5,
            control_scale: 1.5,
            spacing_scale: 1.25,
            effective_size: Vec2::new(480.0, 270.0),
            viewport: UiViewportClass::Compact,
        };
        let mut party = Node::default();
        let mut actions = Node::default();
        let mut inspector = Node::default();
        constrain_region_to_canvas(metrics, UiRegionRole::Party, &mut party);
        constrain_region_to_canvas(metrics, UiRegionRole::Actions, &mut actions);
        constrain_region_to_canvas(metrics, UiRegionRole::Inspector, &mut inspector);
        assert_eq!(party.display, Display::None);
        assert_eq!(actions.display, Display::None);
        assert_eq!(inspector.display, Display::None);
        assert_eq!(actions.left, Val::Auto);
        assert_eq!(actions.right, Val::Auto);
        assert_eq!(actions.height, Val::Auto);
        assert_eq!(inspector.width, Val::Auto);
    }

    #[test]
    fn switching_back_from_compact_restores_secondary_regions() {
        let mut node = Node::default();
        apply_region_layout(UiViewportClass::Compact, UiRegionRole::Inspector, &mut node);
        apply_region_layout(
            UiViewportClass::Standard,
            UiRegionRole::Inspector,
            &mut node,
        );
        assert_eq!(node.display, Display::Flex);
        assert_eq!(node.width, Val::Auto);
        assert_eq!(node.left, Val::Px(248.0));
        assert_eq!(node.right, Val::Px(296.0));
        assert_eq!(node.top, Val::Px(88.0));
        assert_eq!(node.bottom, Val::Px(216.0));
    }

    #[test]
    fn enlarged_action_region_and_main_view_clearance_grow_together() {
        let metrics =
            crate::resolve_ui_metrics(Vec2::new(3840.0, 2160.0), crate::UiScaleMode::Auto);
        assert_eq!(metrics.viewport, UiViewportClass::Wide);
        assert!(semantic_action_region_height(metrics) > 92.0);
        assert!(semantic_action_bar_height(metrics) > 92.0);

        let mut main_view = Node::default();
        let mut actions = Node::default();
        constrain_region_to_canvas(metrics, UiRegionRole::Inspector, &mut main_view);
        constrain_region_to_canvas(metrics, UiRegionRole::Actions, &mut actions);

        let (Val::Px(main_view_bottom), Val::Px(action_height), Val::Px(action_bottom)) =
            (main_view.bottom, actions.height, actions.bottom)
        else {
            panic!("wide Main View and Action Bar regions must have bounded geometry");
        };
        assert!((main_view_bottom - (action_bottom + action_height + 16.0)).abs() < f32::EPSILON);
    }
}
