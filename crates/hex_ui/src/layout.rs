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
        UiViewportClass::Compact | UiViewportClass::Standard => 196.0,
        UiViewportClass::Wide => 200.0,
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
            node.right = Val::Px(268.0);
            node.bottom = Val::Px(action_rail_clearance(viewport));
            node.height = Val::Px(132.0);
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
            node.height = Val::Px(132.0);
        }
        (UiViewportClass::Standard, UiRegionRole::Events) => {
            node.left = Val::Px(244.0);
            node.right = Val::Px(320.0);
            node.bottom = Val::Px(390.0);
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
            node.height = Val::Px(132.0);
        }
        (UiViewportClass::Wide, UiRegionRole::Events) => {
            node.left = Val::Px(288.0);
            node.right = Val::Px(360.0);
            node.bottom = Val::Px(400.0);
        }
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
        assert_eq!(actions.bottom, Val::Px(196.0));
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
