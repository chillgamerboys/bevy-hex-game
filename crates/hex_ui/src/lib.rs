//! Runtime presentation for the game.
//!
//! `hex_ui` renders immutable presentation models and emits typed intentions. It does
//! not inspect or mutate combat, unit, lattice, map, world, or perception authority.
//! The application composition crate owns those adapters.

use bevy::prelude::*;

mod action_rail;
mod creation_presentation;
mod focus;
mod lattice;
mod layout;
mod model;
mod scale;
mod screens;
mod shell;
mod theme;
mod title;

pub use creation_presentation::{effect_summary, CharacterBuildSummary, SpellBuildSummary};
pub use lattice::{
    paint_interactions as paint_lattice_interactions, short_name, spawn_lattice_cells,
    CellInteraction, LatticeCellView, LatticeScale,
};
pub use layout::{action_rail_clearance, apply_region_layout, UiRegionRole};
pub use model::{
    ActionAffordance, ActionAvailability, ActionPriority, GameplayAction, GameplayHudView,
    PauseView, ResumeView, TitleIntent, TitleScenarioView, TitleView, UiIntent, UiSetting,
    UiSettingRow, UiSettingsView,
};
pub use scale::{
    resolve_auto_scale, resolve_ui_metrics, resolve_viewport_class, ResolvedUiMetrics, UiScaleMode,
    UiScalePreference, UiViewportClass,
};
pub use shell::{despawn_screen, overlay_root, screen_root, screen_root_node, DespawnOnExit};
pub use theme::{
    blurb, button, display, divider, element_color, fine, heading, label, panel, panel_node,
    row_button, screen_title, small_button, OwnColors, UiAssets, ACCENT, ACCENT_EDGE, BLURB_SIZE,
    DANGER, DISPLAY_SIZE, EDGE, FINE_SIZE, FUSION_COLOR, GEM_COLOR, LABEL, LABEL_SIZE, MUTED,
    PANEL_BG, SCREEN_TITLE_SIZE, SMALL_BUTTON_WIDTH, TITLE_SIZE,
};

/// Installs the shared runtime design system, responsive scale, focus, and intents.
pub struct UiPlugin;

/// Public ordering seam for composition-root intent handlers.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UiSystems {
    /// Pointer and keyboard interactions have been translated into [`UiIntent`].
    EmitIntents,
}

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<UiIntent>()
            .init_resource::<GameplayHudView>()
            .init_resource::<UiSettingsView>()
            .init_resource::<PauseView>()
            .init_resource::<TitleView>()
            .init_resource::<ResumeView>()
            .add_plugins((
                theme::plugin,
                scale::plugin,
                focus::plugin,
                shell::plugin,
                screens::plugin,
                action_rail::plugin,
                title::plugin,
            ));
    }
}

#[cfg(feature = "test-support")]
pub mod test_support {
    //! Immutable observations for headless presentation tests.

    use bevy::input_focus::{tab_navigation::TabIndex, InputFocus};
    use bevy::prelude::*;

    use crate::{ActionPriority, ResolvedUiMetrics};

    /// One visible named UI node and its presentation-only facts.
    #[derive(Debug, Clone, PartialEq)]
    pub struct UiNodeObservation {
        /// Stable entity name used by review and test automation.
        pub name: String,
        /// Whether the node is inherited-visible.
        pub visible: bool,
        /// Computed logical size when Bevy layout has run.
        pub size: Vec2,
        /// Computed logical center in the UI camera's coordinate space.
        pub center: Vec2,
        /// Accessible label supplied to assistive technology.
        pub accessible_label: Option<String>,
        /// Explicit tab order, when this node is focusable.
        pub tab_index: Option<i32>,
        /// Whether laid-out content exceeds this node's box on either axis.
        pub overflows: bool,
        /// Whether this node currently has keyboard focus.
        pub focused: bool,
    }

    /// Presentation-only state. It is never a gameplay oracle.
    #[derive(Debug, Clone, PartialEq)]
    pub struct UiTreeSnapshot {
        /// Resolved global scale and responsive class.
        pub metrics: ResolvedUiMetrics,
        /// Visible named nodes in stable name order.
        pub nodes: Vec<UiNodeObservation>,
        /// Named focusable nodes ordered by tab index, then stable name.
        pub focus_order: Vec<String>,
        /// Highest action priority presented by the action rail.
        pub action_priority: Option<ActionPriority>,
    }

    /// Observes the rendered tree without exposing mutable UI resources.
    #[must_use]
    pub fn ui_tree_snapshot(world: &mut World) -> UiTreeSnapshot {
        let metrics = world
            .get_resource::<ResolvedUiMetrics>()
            .copied()
            .unwrap_or_default();
        let focused = world.get_resource::<InputFocus>().and_then(InputFocus::get);
        let action_priority = world
            .get_resource::<crate::GameplayHudView>()
            .and_then(|view| view.actions.iter().map(|action| action.priority).max());
        let mut query = world.query::<(
            Entity,
            &Name,
            Option<&Visibility>,
            Option<&InheritedVisibility>,
            Option<&ComputedNode>,
            Option<&GlobalTransform>,
            Option<&AccessibleLabel>,
            Option<&TabIndex>,
        )>();
        let mut nodes = query
            .iter(world)
            .map(
                |(
                    entity,
                    name,
                    visibility,
                    inherited_visibility,
                    computed,
                    transform,
                    label,
                    tab_index,
                )| {
                    let inverse_scale = computed.map_or(1.0, |node| node.inverse_scale_factor);
                    UiNodeObservation {
                        name: name.as_str().to_owned(),
                        visible: visibility
                            .is_none_or(|visibility| *visibility != Visibility::Hidden)
                            && inherited_visibility.is_none_or(|visibility| visibility.get()),
                        size: computed
                            .map_or(Vec2::ZERO, |node| node.size() * node.inverse_scale_factor),
                        center: transform.map_or(Vec2::ZERO, |transform| {
                            transform.translation().truncate() * inverse_scale
                        }),
                        accessible_label: label.map(|label| label.0.clone()),
                        tab_index: tab_index.map(|index| index.0),
                        overflows: computed.is_some_and(|node| {
                            let epsilon = 0.5;
                            node.content_size().x > node.size().x + epsilon
                                || node.content_size().y > node.size().y + epsilon
                        }),
                        focused: focused == Some(entity),
                    }
                },
            )
            .collect::<Vec<_>>();
        nodes.sort_by(|left, right| left.name.cmp(&right.name));
        let mut focus_nodes = nodes
            .iter()
            .filter_map(|node| node.tab_index.map(|index| (index, node.name.clone())))
            .collect::<Vec<_>>();
        focus_nodes.sort();
        UiTreeSnapshot {
            metrics,
            nodes,
            focus_order: focus_nodes.into_iter().map(|(_, name)| name).collect(),
            action_priority,
        }
    }

    #[cfg(test)]
    mod tests {
        use bevy::input_focus::tab_navigation::TabIndex;

        use super::*;

        #[test]
        fn snapshot_exposes_accessibility_and_focus_order_without_mutable_ui_state() {
            let mut world = World::new();
            world.spawn((
                Name::new("Confirm Choice"),
                Button,
                TabIndex(2),
                AccessibleLabel::new("Confirm selected lattice cells"),
            ));
            world.spawn((
                Name::new("Choose Cell"),
                Button,
                TabIndex(1),
                AccessibleLabel::new("Choose lattice cell"),
            ));

            let snapshot = ui_tree_snapshot(&mut world);
            assert_eq!(snapshot.focus_order, ["Choose Cell", "Confirm Choice"]);
            let Some(confirm) = snapshot
                .nodes
                .iter()
                .find(|node| node.name == "Confirm Choice")
            else {
                panic!("the named control must be observable");
            };
            assert_eq!(
                confirm.accessible_label.as_deref(),
                Some("Confirm selected lattice cells")
            );
        }
    }
}
