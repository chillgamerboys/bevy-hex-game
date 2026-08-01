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

/// Vertical clearance required by the rail after semantic control growth.
pub(crate) fn semantic_action_rail_clearance(metrics: crate::ResolvedUiMetrics) -> f32 {
    action_rail_clearance(metrics.viewport) + 160.0 * (metrics.content_scale - 1.0).max(0.0)
}

/// Height needed by the horizontal casting strip as semantic controls grow.
/// Enlarged text wraps before a simple proportional control scale would account
/// for it, while Auto at ordinary desktop sizes keeps the compact baseline.
pub(crate) fn semantic_action_region_height(metrics: crate::ResolvedUiMetrics) -> f32 {
    92.0 + 160.0 * (metrics.control_scale - 1.0).max(0.0)
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
            node.top = Val::Px(8.0);
            node.bottom = Val::Px(8.0);
            node.left = Val::Px(8.0);
            node.width = Val::Px(180.0);
        }
        (UiViewportClass::Compact, UiRegionRole::Turn) => {
            node.top = Val::Px(12.0);
            node.left = Val::Px(196.0);
            node.right = Val::Px(268.0);
            node.height = Val::Px(72.0);
        }
        (UiViewportClass::Compact, UiRegionRole::Inspector | UiRegionRole::Events) => {
            node.display = Display::None;
        }
        (UiViewportClass::Compact, UiRegionRole::Actions) => {
            node.left = Val::Px(196.0);
            node.right = Val::Px(12.0);
            node.bottom = Val::Px(action_rail_clearance(viewport));
            node.height = Val::Px(92.0);
        }
        (UiViewportClass::Standard, UiRegionRole::Party) => {
            node.top = Val::Px(12.0);
            node.bottom = Val::Px(12.0);
            node.left = Val::Px(12.0);
            node.width = Val::Px(224.0);
        }
        (UiViewportClass::Standard, UiRegionRole::Turn) => {
            node.top = Val::Px(12.0);
            node.left = Val::Px(244.0);
            node.right = Val::Px(320.0);
            node.height = Val::Px(72.0);
        }
        (UiViewportClass::Standard, UiRegionRole::Inspector) => {
            node.top = Val::Px(12.0);
            node.right = Val::Px(12.0);
            node.bottom = Val::Px(12.0);
            node.width = Val::Px(300.0);
        }
        (UiViewportClass::Standard, UiRegionRole::Actions) => {
            node.left = Val::Px(244.0);
            node.right = Val::Px(320.0);
            node.bottom = Val::Px(action_rail_clearance(viewport));
            node.height = Val::Px(92.0);
        }
        (UiViewportClass::Standard, UiRegionRole::Events) => {
            node.left = Val::Px(244.0);
            node.right = Val::Px(320.0);
            node.bottom = Val::Px(264.0);
        }
        (UiViewportClass::Wide, UiRegionRole::Party) => {
            node.top = Val::Px(16.0);
            node.bottom = Val::Px(16.0);
            node.left = Val::Px(16.0);
            node.width = Val::Px(260.0);
        }
        (UiViewportClass::Wide, UiRegionRole::Turn) => {
            node.top = Val::Px(16.0);
            node.left = Val::Px(288.0);
            node.right = Val::Px(360.0);
            node.height = Val::Px(76.0);
        }
        (UiViewportClass::Wide, UiRegionRole::Inspector) => {
            node.top = Val::Px(16.0);
            node.right = Val::Px(16.0);
            node.bottom = Val::Px(12.0);
            node.width = Val::Px(332.0);
        }
        (UiViewportClass::Wide, UiRegionRole::Actions) => {
            node.left = Val::Px(288.0);
            node.right = Val::Px(360.0);
            node.bottom = Val::Px(action_rail_clearance(viewport));
            node.height = Val::Px(92.0);
        }
        (UiViewportClass::Wide, UiRegionRole::Events) => {
            node.left = Val::Px(288.0);
            node.right = Val::Px(360.0);
            node.bottom = Val::Px(268.0);
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
    if role == UiRegionRole::Actions {
        let semantic_clearance = semantic_action_rail_clearance(metrics);
        node.bottom = Val::Px(semantic_clearance);
        node.height = Val::Px(semantic_action_region_height(metrics));
        if metrics.viewport == UiViewportClass::Compact
            && metrics.content_scale > 1.0
            && metrics.content_scale < 1.5
        {
            node.height = Val::Px(132.0_f32.max(semantic_action_region_height(metrics)));
        }
        if metrics.content_scale >= 1.5 {
            let top = match metrics.viewport {
                UiViewportClass::Compact | UiViewportClass::Standard => 92.0,
                UiViewportClass::Wide => 96.0,
            };
            node.top = Val::Px(top);
            node.height = Val::Px((metrics.logical_size.y - semantic_clearance - top).max(44.0));
        }
    }
    if metrics.viewport != UiViewportClass::Compact {
        return;
    }
    if is_ultra_constrained(metrics) {
        match role {
            UiRegionRole::Party | UiRegionRole::Turn => node.display = Display::None,
            UiRegionRole::Actions => {
                let top = ultra_action_rail_height(metrics) + 8.0;
                node.top = Val::Px(top);
                node.left = Val::Px(8.0);
                node.right = Val::Px(8.0);
                node.bottom = Val::ZERO;
                node.height = Val::Px((metrics.logical_size.y - top).max(44.0));
            }
            UiRegionRole::Inspector | UiRegionRole::Events => {}
        }
    }
}

pub(crate) fn is_ultra_constrained(metrics: crate::ResolvedUiMetrics) -> bool {
    metrics.viewport == UiViewportClass::Compact
        && (metrics.effective_size.y <= 540.0
            || (metrics.content_scale >= 1.5 && metrics.effective_size.y < 700.0))
}

pub(crate) fn ultra_action_rail_height(metrics: crate::ResolvedUiMetrics) -> f32 {
    // Enlarged typography needs reflow, not an empty proportional slab. The
    // rail contains two lines of essential copy and one required control; its
    // controls grow only to 1.5x, so this bounded allowance keeps the command
    // visible while returning most of the canvas to the actual decision.
    let semantic_height = 205.0 + 110.0 * (metrics.control_scale - 1.0).max(0.0);
    let narrow_wrap_allowance = if metrics.logical_size.x < 1100.0 && metrics.content_scale >= 1.5 {
        95.0
    } else {
        0.0
    };
    semantic_height + narrow_wrap_allowance
}

/// Left edge reserved for the optional development-time controls on the
/// shortest Compact canvases. Enlarged semantic UI and blocking choices own
/// the complete width because required actions take priority over tooling.
pub(crate) fn ultra_action_rail_left(
    metrics: crate::ResolvedUiMetrics,
    decision_required: bool,
) -> f32 {
    if is_ultra_constrained(metrics) && !decision_required {
        196.0
    } else {
        12.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_layout_collapses_secondary_regions_before_actions() {
        let mut inspector = Node::default();
        let mut events = Node::default();
        let mut actions = Node::default();
        apply_region_layout(
            UiViewportClass::Compact,
            UiRegionRole::Inspector,
            &mut inspector,
        );
        apply_region_layout(UiViewportClass::Compact, UiRegionRole::Events, &mut events);
        apply_region_layout(
            UiViewportClass::Compact,
            UiRegionRole::Actions,
            &mut actions,
        );
        assert_eq!(inspector.display, Display::None);
        assert_eq!(events.display, Display::None);
        assert_eq!(actions.display, Display::Flex);
        assert_eq!(actions.top, Val::Auto);
        assert_eq!(
            actions.bottom,
            Val::Px(action_rail_clearance(UiViewportClass::Compact))
        );
        assert_eq!(actions.right, Val::Px(12.0));
    }

    #[test]
    fn ultra_constrained_canvas_keeps_actions_onscreen_and_collapses_duplicate_context() {
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
        constrain_region_to_canvas(metrics, UiRegionRole::Party, &mut party);
        constrain_region_to_canvas(metrics, UiRegionRole::Actions, &mut actions);
        assert_eq!(party.display, Display::None);
        assert_eq!(actions.top, Val::Px(363.0));
        assert_eq!(actions.left, Val::Px(8.0));
        assert_eq!(actions.right, Val::Px(8.0));
        assert_eq!(actions.bottom, Val::ZERO);
        assert_eq!(actions.height, Val::Px(177.0));
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
        assert_eq!(node.width, Val::Px(300.0));
    }
}
