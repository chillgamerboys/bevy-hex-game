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
#[cfg(feature = "dev-tools")]
mod dev_time;
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
#[cfg(feature = "dev-tools")]
pub use model::{DevTimeIntent, DevTimeView};
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
        #[cfg(feature = "dev-tools")]
        app.init_resource::<DevTimeView>()
            .add_plugins(dev_time::plugin);
    }
}

#[cfg(feature = "test-support")]
pub mod test_support {
    //! Immutable observations for headless presentation tests.

    use bevy::input_focus::{
        tab_navigation::{TabGroup, TabIndex},
        InputFocus,
    };
    use bevy::prelude::*;
    use bevy::window::WindowResolution;

    use crate::{ActionPriority, ResolvedUiMetrics};

    /// Renderer-free plugin for exercising the real UI schedules and layout tree.
    ///
    /// Install this on an otherwise empty [`App`]. It creates one synthetic primary
    /// window, the stable Bevy UI/input/text stack, application states, and
    /// [`crate::UiPlugin`], but never initializes Winit, a renderer, or gameplay.
    pub struct HeadlessUiPlugin {
        logical_size: UVec2,
    }

    impl HeadlessUiPlugin {
        /// Builds a headless UI canvas with an exact logical size.
        #[must_use]
        pub const fn new(width: u32, height: u32) -> Self {
            Self {
                logical_size: UVec2::new(width, height),
            }
        }
    }

    impl Default for HeadlessUiPlugin {
        fn default() -> Self {
            Self::new(1920, 1080)
        }
    }

    impl Plugin for HeadlessUiPlugin {
        fn build(&self, app: &mut App) {
            app.add_plugins((
                MinimalPlugins,
                bevy::transform::TransformPlugin,
                bevy::camera::visibility::VisibilityPlugin,
                bevy::input::InputPlugin,
                bevy::input_focus::InputFocusPlugin,
                bevy::input_focus::InputDispatchPlugin,
                bevy::window::WindowPlugin {
                    primary_window: Some(Window {
                        resolution: WindowResolution::new(self.logical_size.x, self.logical_size.y)
                            .with_scale_factor_override(1.0),
                        ..default()
                    }),
                    ..default()
                },
                bevy::asset::AssetPlugin::default(),
                bevy::image::ImagePlugin::default(),
                bevy::mesh::MeshPlugin,
                bevy::text::TextPlugin,
                bevy::ui::UiPlugin,
                bevy::state::app::StatesPlugin,
            ));
            app.init_asset::<bevy::image::TextureAtlasLayout>();
            app.add_plugins(bevy::picking::DefaultPickingPlugins);
            app.add_plugins(bevy::ui_widgets::UiWidgetsPlugins);
            app.init_state::<hex_core::Screen>();
            app.add_sub_state::<hex_core::Mode>();
            app.add_sub_state::<hex_core::Pause>();
            let logical_size = self.logical_size;
            app.add_systems(Startup, move |mut commands: Commands| {
                commands.spawn((
                    Camera2d,
                    bevy::camera::Camera {
                        computed: bevy::camera::ComputedCameraValues {
                            target_info: Some(bevy::camera::RenderTargetInfo {
                                physical_size: logical_size,
                                scale_factor: 1.0,
                            }),
                            ..default()
                        },
                        ..default()
                    },
                ));
            });
            app.add_plugins(crate::UiPlugin);
        }
    }

    /// One visible named UI node and its presentation-only facts.
    #[derive(Debug, Clone, PartialEq)]
    pub struct UiNodeObservation {
        /// Stable entity name used by review and test automation.
        pub name: String,
        /// Whether the node is inherited-visible.
        pub visible: bool,
        /// Computed logical size when Bevy layout has run.
        pub size: Vec2,
        /// Computed logical content size before clipping.
        pub content_size: Vec2,
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
        /// Named focusable nodes in Bevy's tab-group, index, and hierarchy order.
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
        let focus_order = logical_focus_order(world, focused);
        let entities = {
            let mut query = world.query_filtered::<Entity, With<Name>>();
            query.iter(world).collect::<Vec<_>>()
        };
        let mut nodes = entities
            .into_iter()
            .filter(|entity| is_presented(world, *entity))
            .filter_map(|entity| {
                let name = world.get::<Name>(entity)?;
                let computed = world.get::<ComputedNode>(entity);
                let inverse_scale = computed.map_or(1.0, |node| node.inverse_scale_factor);
                Some(UiNodeObservation {
                    name: name.as_str().to_owned(),
                    visible: true,
                    size: computed
                        .map_or(Vec2::ZERO, |node| node.size() * node.inverse_scale_factor),
                    content_size: computed.map_or(Vec2::ZERO, |node| {
                        node.content_size() * node.inverse_scale_factor
                    }),
                    center: world
                        .get::<bevy::ui::UiGlobalTransform>(entity)
                        .map_or(Vec2::ZERO, |transform| {
                            transform.affine().translation * inverse_scale
                        }),
                    accessible_label: world
                        .get::<AccessibleLabel>(entity)
                        .map(|label| label.0.clone()),
                    tab_index: world.get::<TabIndex>(entity).map(|index| index.0),
                    overflows: computed.is_some_and(|node| {
                        let epsilon = 0.5;
                        node.content_size().x > node.size().x + epsilon
                            || node.content_size().y > node.size().y + epsilon
                    }),
                    focused: focused == Some(entity),
                })
            })
            .collect::<Vec<_>>();
        nodes.sort_by(|left, right| left.name.cmp(&right.name));
        UiTreeSnapshot {
            metrics,
            nodes,
            focus_order,
            action_priority,
        }
    }

    fn logical_focus_order(world: &mut World, focused: Option<Entity>) -> Vec<String> {
        let mut groups = world
            .query::<(Entity, &TabGroup)>()
            .iter(world)
            .map(|(entity, group)| (entity, *group))
            .filter(|(entity, _)| is_presented(world, *entity))
            .collect::<Vec<_>>();
        groups.sort_by_key(|(entity, group)| (group.order, entity.to_bits()));

        let focused_modal = focused.and_then(|focused| {
            groups
                .iter()
                .find(|(group, settings)| {
                    settings.modal && is_descendant_or_self(world, focused, *group)
                })
                .map(|(group, _)| *group)
        });
        let active_modal = focused_modal.or_else(|| {
            groups
                .iter()
                .rev()
                .find_map(|(group, settings)| settings.modal.then_some(*group))
        });
        let groups = groups.into_iter().filter(|(group, settings)| {
            active_modal.map_or(!settings.modal, |active_modal| *group == active_modal)
        });

        let mut order = Vec::new();
        for (group, _) in groups {
            let mut within_group = Vec::new();
            let mut hierarchy_position = 0_usize;
            gather_focusable(
                world,
                group,
                group,
                &mut hierarchy_position,
                &mut within_group,
            );
            within_group.sort_by_key(|(index, position, _)| (*index, *position));
            order.extend(within_group.into_iter().map(|(_, _, name)| name));
        }
        order
    }

    fn is_descendant_or_self(world: &World, mut entity: Entity, ancestor: Entity) -> bool {
        loop {
            if entity == ancestor {
                return true;
            }
            let Some(parent) = world.get::<ChildOf>(entity) else {
                return false;
            };
            entity = parent.parent();
        }
    }

    fn is_presented(world: &World, mut entity: Entity) -> bool {
        loop {
            if world
                .get::<Visibility>(entity)
                .is_some_and(|visibility| *visibility == Visibility::Hidden)
                || world
                    .get::<InheritedVisibility>(entity)
                    .is_some_and(|visibility| !visibility.get())
                || world
                    .get::<Node>(entity)
                    .is_some_and(|node| node.display == Display::None)
            {
                return false;
            }
            let Some(parent) = world.get::<ChildOf>(entity) else {
                return true;
            };
            entity = parent.parent();
        }
    }

    fn gather_focusable(
        world: &World,
        group: Entity,
        entity: Entity,
        hierarchy_position: &mut usize,
        output: &mut Vec<(i32, usize, String)>,
    ) {
        if entity != group && world.get::<TabGroup>(entity).is_some() {
            return;
        }
        let visible = is_presented(world, entity);
        if visible {
            if let (Some(index), Some(name)) =
                (world.get::<TabIndex>(entity), world.get::<Name>(entity))
            {
                if index.0 >= 0 {
                    output.push((index.0, *hierarchy_position, name.as_str().to_owned()));
                }
            }
        }
        *hierarchy_position += 1;
        if let Some(children) = world.get::<Children>(entity) {
            for child in children.iter() {
                gather_focusable(world, group, child, hierarchy_position, output);
            }
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
        use bevy::input_focus::tab_navigation::{TabGroup, TabIndex};

        use super::*;

        fn required_cell(q: i32, r: i32) -> crate::LatticeCellView {
            crate::LatticeCellView {
                coord: hex_core::LatticeCoord::new(q, r),
                label: "AIR".to_owned(),
                detail: "LIVE".to_owned(),
                color: Color::srgb(0.3, 0.6, 0.8),
                known_mana: Some(1),
                known_locked: Some(false),
                disabled: false,
                selected: false,
                interaction: crate::CellInteraction::Actionable,
            }
        }

        fn overlaps(left: &UiNodeObservation, right: &UiNodeObservation) -> bool {
            let left_min = left.center - left.size * 0.5;
            let left_max = left.center + left.size * 0.5;
            let right_min = right.center - right.size * 0.5;
            let right_max = right.center + right.size * 0.5;
            left_min.x < right_max.x
                && left_max.x > right_min.x
                && left_min.y < right_max.y
                && left_max.y > right_min.y
        }

        fn required_choice_snapshot(
            width: u32,
            height: u32,
            mode: crate::UiScaleMode,
        ) -> UiTreeSnapshot {
            let mut app = App::new();
            app.add_plugins(HeadlessUiPlugin::new(width, height));
            app.world_mut()
                .insert_resource(crate::UiScalePreference(mode));
            app.world_mut().insert_resource(crate::GameplayChromeView {
                shown: false,
                decision_required: true,
                encounter_complete: false,
            });
            app.world_mut().insert_resource(crate::GameplayHudView {
                phase: hex_core::GameplayPhase::Active,
                actor: Some(hex_core::UnitId(0)),
                actor_label: "Hedge Mage".to_owned(),
                round: "Round 1".to_owned(),
                movement_remaining: 2,
                action_remaining: true,
                required_prompt: Some("Choose one live cell".to_owned()),
                actions: vec![crate::ActionAffordance {
                    action: crate::GameplayAction::ConfirmDecision,
                    label: "Confirm 0 / 1".to_owned(),
                    shortcut: Some("Enter".to_owned()),
                    availability: crate::ActionAvailability::Disabled {
                        reason: "Choose one live cell".to_owned(),
                    },
                    priority: crate::ActionPriority::Required,
                }],
            });
            app.world_mut()
                .insert_resource(crate::GameplayLatticesView {
                    own: Some(crate::OwnLatticeView {
                        heading: "required choice".to_owned(),
                        identity: "Hedge Mage".to_owned(),
                        cells: vec![
                            required_cell(0, 0),
                            required_cell(1, 0),
                            required_cell(0, 1),
                            required_cell(-1, 1),
                        ],
                        decision: Some(crate::DecisionChoiceView {
                            chosen: 0,
                            owed: 1,
                            restoring: false,
                        }),
                    }),
                    target: None,
                });
            app.world_mut()
                .resource_mut::<NextState<hex_core::Screen>>()
                .set(hex_core::Screen::Gameplay);

            for _ in 0..8 {
                app.update();
            }

            ui_tree_snapshot(app.world_mut())
        }

        #[test]
        fn headless_plugin_lays_out_required_compact_controls_without_a_renderer() {
            let snapshot = required_choice_snapshot(1920, 1080, crate::UiScaleMode::Percent200);
            assert_eq!(snapshot.metrics.viewport, crate::UiViewportClass::Compact);
            for required in [
                "Compact Required Lattice Summary",
                "Compact Required Lattice",
                "Compact Required Lattice Body",
                "Compact Required Cell (0, 0)",
                "Compact Required Lattice Choice",
                "Primary Action Rail",
            ] {
                let Some(node) = snapshot.nodes.iter().find(|node| node.name == required) else {
                    panic!(
                        "required compact control {required:?} must be visible; saw {:?}",
                        snapshot
                            .nodes
                            .iter()
                            .map(|node| node.name.as_str())
                            .collect::<Vec<_>>()
                    );
                };
                assert!(node.size.cmpgt(Vec2::ZERO).all());
                assert!(
                    !node.overflows,
                    "{required:?} must fit its layout box: {node:?}; compact tree: {:?}",
                    snapshot
                        .nodes
                        .iter()
                        .filter(|candidate| candidate.name.contains("Compact Required"))
                        .collect::<Vec<_>>()
                );
            }
            assert!(snapshot
                .focus_order
                .iter()
                .any(|name| name == "Compact Required Cell (0, 0)"));
            assert_eq!(
                snapshot.action_priority,
                Some(crate::ActionPriority::Required)
            );
        }

        #[test]
        fn required_choice_is_reachable_across_the_structural_matrix() {
            for logical_size in [
                UVec2::new(960, 540),
                UVec2::new(1280, 720),
                UVec2::new(1920, 1080),
                UVec2::new(2560, 1440),
                UVec2::new(3840, 2160),
            ] {
                for mode in [crate::UiScaleMode::Auto, crate::UiScaleMode::Percent200] {
                    let snapshot = required_choice_snapshot(logical_size.x, logical_size.y, mode);
                    let prefix = if snapshot.metrics.viewport == crate::UiViewportClass::Compact {
                        "Compact Required"
                    } else {
                        "Own"
                    };
                    let cell_names = [
                        format!("{prefix} Cell (0, 0)"),
                        format!("{prefix} Cell (1, 0)"),
                        format!("{prefix} Cell (0, 1)"),
                        format!("{prefix} Cell (-1, 1)"),
                    ];
                    for required in std::iter::once("Primary Action Rail")
                        .chain(cell_names.iter().map(String::as_str))
                    {
                        let Some(node) = snapshot.nodes.iter().find(|node| node.name == required)
                        else {
                            panic!("{required:?} must be visible at {logical_size:?} in {mode:?}");
                        };
                        assert!(
                            node.size.cmpgt(Vec2::ZERO).all(),
                            "{required:?} must have a layout box at {logical_size:?} in {mode:?}"
                        );
                        assert!(
                            !node.overflows,
                            "{required:?} must not overflow at {logical_size:?} in {mode:?}: {node:?}"
                        );
                        let half = node.size * 0.5;
                        let min = node.center - half;
                        let max = node.center + half;
                        assert!(
                            min.cmpge(Vec2::ZERO).all()
                                && max.cmple(snapshot.metrics.effective_size).all(),
                            "{required:?} must remain on canvas at {logical_size:?} in {mode:?}: {node:?}"
                        );
                    }
                    let Some(rail) = snapshot
                        .nodes
                        .iter()
                        .find(|node| node.name == "Primary Action Rail")
                    else {
                        panic!("the required rail was already asserted visible");
                    };
                    for cell_name in &cell_names {
                        let Some(cell) = snapshot.nodes.iter().find(|node| node.name == *cell_name)
                        else {
                            panic!("the required cell was already asserted visible");
                        };
                        assert!(
                            !overlaps(rail, cell),
                            "the action rail must not obscure {cell_name:?} at {logical_size:?} in {mode:?}: rail={rail:?}, cell={cell:?}"
                        );
                    }
                    assert!(cell_names
                        .iter()
                        .all(|cell| snapshot.focus_order.iter().any(|name| name == cell)));
                }
            }
        }

        #[test]
        fn snapshot_exposes_accessibility_and_focus_order_without_mutable_ui_state() {
            let mut world = World::new();
            let group = world
                .spawn((
                    Name::new("Screen"),
                    TabGroup::new(0),
                    Visibility::Inherited,
                    InheritedVisibility::VISIBLE,
                ))
                .id();
            let confirm = world
                .spawn((
                    Name::new("Confirm Choice"),
                    Button,
                    TabIndex(0),
                    AccessibleLabel::new("Confirm selected lattice cells"),
                    InheritedVisibility::VISIBLE,
                ))
                .id();
            let choose = world
                .spawn((
                    Name::new("Choose Cell"),
                    Button,
                    TabIndex(0),
                    AccessibleLabel::new("Choose lattice cell"),
                    InheritedVisibility::VISIBLE,
                ))
                .id();
            world.entity_mut(group).add_children(&[confirm, choose]);

            let snapshot = ui_tree_snapshot(&mut world);
            assert_eq!(snapshot.focus_order, ["Confirm Choice", "Choose Cell"]);
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

        #[test]
        fn snapshot_excludes_display_none_subtrees() {
            let mut world = World::new();
            let collapsed = world
                .spawn((
                    Name::new("Collapsed Drawer"),
                    Node {
                        display: Display::None,
                        ..default()
                    },
                    InheritedVisibility::VISIBLE,
                ))
                .id();
            let hidden_control = world
                .spawn((
                    Name::new("Hidden Drawer Control"),
                    Node::default(),
                    TabIndex(0),
                    InheritedVisibility::VISIBLE,
                ))
                .id();
            world.entity_mut(collapsed).add_child(hidden_control);

            let snapshot = ui_tree_snapshot(&mut world);
            assert!(snapshot.nodes.is_empty());
            assert!(snapshot.focus_order.is_empty());
        }

        #[test]
        fn pause_overlay_exposes_a_focusable_mouse_and_keyboard_resume_action() {
            let mut app = App::new();
            app.add_plugins(HeadlessUiPlugin::default());
            app.world_mut()
                .resource_mut::<NextState<hex_core::Screen>>()
                .set(hex_core::Screen::Gameplay);
            for _ in 0..4 {
                app.update();
            }
            app.world_mut()
                .resource_mut::<NextState<hex_core::Pause>>()
                .set(hex_core::Pause(true));
            for _ in 0..4 {
                app.update();
            }

            let snapshot = ui_tree_snapshot(app.world_mut());
            let Some(resume) = snapshot.nodes.iter().find(|node| node.name == "Resume") else {
                panic!("the pause modal must expose its Resume control");
            };
            assert_eq!(resume.accessible_label.as_deref(), Some("Resume"));
            assert!(snapshot.focus_order.iter().any(|name| name == "Resume"));
        }
    }
}
