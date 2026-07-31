//! Runtime presentation for the game.
//!
//! `hex_ui` renders immutable presentation models and emits typed intentions. It does
//! not inspect or mutate combat, unit, lattice, map, world, or perception authority.
//! The application composition crate owns those adapters.

use bevy::prelude::*;

mod action_rail;
mod casting_panel;
mod combat_lab;
mod combat_log;
mod creation_presentation;
mod creator;
mod deployment;
mod focus;
mod gameplay_frame;
mod gameplay_lattices;
mod initiative;
mod lab_statistics;
mod lattice;
mod lattice_demo;
mod layout;
mod model;
mod outcome_report;
mod party;
mod review;
mod scale;
mod screens;
mod shell;
mod theme;
mod title;
mod unit_badges;

pub use creation_presentation::{effect_summary, CharacterBuildSummary, SpellBuildSummary};
pub use gameplay_lattices::spawn_decision_controls;
pub use lattice::{
    paint_interactions as paint_lattice_interactions, short_name, spawn_lattice_cells,
    CellInteraction, LatticeCellView, LatticeScale,
};
pub use layout::{
    action_rail_clearance, apply_region_layout, HudElement, RequiredActionSurface, UiRegionRole,
    READ_ONLY_HUD,
};
pub use model::{
    ActionAffordance, ActionAvailability, ActionPriority, BadgeKind, CastingAimView, CastingIntent,
    CastingPanelContentView, CastingPanelView, CastingSpellView, CombatLabComparisonView,
    CombatLabIntent, CombatLabReportCardView, CombatLabReportField, CombatLabReportsView,
    CombatLabRulesVariant, CombatLabScreenView, CombatLogLineView, CombatLogView,
    CreatorEffectKind, CreatorIntent, CreatorLibraryView, CreatorNameField, CreatorScreenView,
    CreatorWorkspace, DecisionChoiceView, DeploymentIntent, DeploymentRosterEntryView,
    DeploymentView, FormationSlotView, GameplayAction, GameplayChromeView, GameplayHudView,
    GameplayLatticesView, InitiativeEntryView, InitiativeSide, InitiativeView, LabStatisticsIntent,
    LabStatisticsView, LatticeDemoIntent, LatticeDemoSpellView, LatticeDemoView, LatticeIntent,
    OutcomeAction, OutcomeActionView, OutcomeCompareChoiceView, OutcomeIntent, OutcomeReportView,
    OwnLatticeView, PartyIntent, PartyMemberView, PartyView, PauseView, ResumeView,
    TargetLatticeStateView, TargetLatticeView, TargetPulseView, TitleIntent, TitleScenarioView,
    TitleView, UiIntent, UiSetting, UiSettingRow, UiSettingsView, UnitBadgeView, UnitBadgesView,
};
pub use scale::{
    resolve_auto_scale, resolve_ui_metrics, resolve_viewport_class, ResolvedUiMetrics, UiScaleMode,
    UiScalePreference, UiViewportClass,
};
pub use shell::{despawn_screen, overlay_root, screen_root, screen_root_node, DespawnOnExit};
pub use theme::{
    blurb, button, display, divider, element_color, fine, heading, label, panel, panel_node,
    row_button, screen_title, small_button, stacked_row_button, OwnColors, UiAssets, ACCENT,
    ACCENT_EDGE, BLURB_SIZE, DANGER, DISPLAY_SIZE, EDGE, FINE_SIZE, FUSION_COLOR, GEM_COLOR, LABEL,
    LABEL_SIZE, MUTED, PANEL_BG, SCREEN_TITLE_SIZE, SMALL_BUTTON_WIDTH, TITLE_SIZE,
};

#[cfg(feature = "visual-review")]
pub use review::apply_ui_review_fixture;

/// Installs the shared runtime design system, responsive scale, focus, and intents.
pub struct UiPlugin;

/// Public ordering seam for composition-root intent handlers.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UiSystems {
    /// Pointer and keyboard interactions have been translated into [`UiIntent`].
    EmitIntents,
    /// Immutable projections have been converted into runtime presentation.
    Render,
}

/// Ordered gameplay UI construction stages shared with application adapters that
/// still attach domain-specific projections to renderer-owned regions.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UiHudSetup {
    /// Create the responsive safe-frame regions.
    Frame,
    /// Attach presentation panels to those regions.
    Panels,
}

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.configure_sets(
            OnEnter(hex_core::Screen::Gameplay),
            (UiHudSetup::Frame, UiHudSetup::Panels).chain(),
        )
        .add_message::<UiIntent>()
        .init_resource::<CombatLogView>()
        .init_resource::<CastingPanelView>()
        .init_resource::<GameplayChromeView>()
        .init_resource::<GameplayHudView>()
        .init_resource::<GameplayLatticesView>()
        .init_resource::<UiSettingsView>()
        .init_resource::<PauseView>()
        .init_resource::<PartyView>()
        .init_resource::<LabStatisticsView>()
        .init_resource::<OutcomeReportView>()
        .init_resource::<LatticeDemoView>()
        .init_resource::<CreatorScreenView>()
        .init_resource::<CombatLabScreenView>()
        .init_resource::<DeploymentView>()
        .init_resource::<InitiativeView>()
        .init_resource::<TitleView>()
        .init_resource::<TargetPulseView>()
        .init_resource::<UnitBadgesView>()
        .init_resource::<ResumeView>()
        .add_plugins((
            theme::plugin,
            casting_panel::plugin,
            combat_log::plugin,
            scale::plugin,
            focus::plugin,
            gameplay_frame::plugin,
            gameplay_lattices::plugin,
            initiative::plugin,
            lab_statistics::plugin,
            party::plugin,
            shell::plugin,
            screens::plugin,
            action_rail::plugin,
            title::plugin,
            unit_badges::plugin,
        ))
        .add_plugins((
            combat_lab::plugin,
            outcome_report::plugin,
            lattice_demo::plugin,
            creator::plugin,
            deployment::plugin,
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

    /// Returns the stable identities of visible Combat Lab fixture cards.
    ///
    /// This observes presentation filtering only; fixture behavior remains a
    /// canonical gameplay-model concern.
    #[must_use]
    pub fn visible_combat_lab_fixture_ids(world: &mut World) -> Vec<String> {
        crate::combat_lab::visible_fixture_ids(world)
    }

    /// Exercises the production fixture filter and clear cycle.
    #[must_use]
    pub fn combat_lab_fixture_filter_cycle(query: &str) -> (Vec<String>, Vec<String>, bool) {
        crate::combat_lab::observe_fixture_filter(query)
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
