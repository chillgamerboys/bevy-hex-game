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
    ScenarioBrowserIntent, ScenarioBrowserView, TargetLatticeStateView, TargetLatticeView,
    TargetPulseView, TitleIntent, TitleScenarioView, TitleView, UiIntent, UiSetting, UiSettingRow,
    UiSettingsView, UnitBadgeView, UnitBadgesView,
};
#[cfg(feature = "dev-tools")]
pub use model::{DevTimeIntent, DevTimeView};
#[cfg(any(feature = "visual-review", feature = "test-support"))]
pub use scale::ReviewViewport;
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

#[cfg(any(feature = "visual-review", feature = "test-support"))]
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

/// Initial-view requirement consumed by the structural presentation oracle.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiVisibilityRequirement {
    /// The control must be fully visible before any scrolling or drawer changes.
    Immediate,
    /// The control may begin offscreen when an operable scroll owner can reveal it.
    Scrollable,
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
        .init_resource::<ScenarioBrowserView>()
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
        #[cfg(feature = "test-support")]
        app.init_resource::<test_support::LatestUiTreeSnapshot>()
            .add_systems(Last, test_support::publish_ui_tree_snapshot);
    }
}

#[cfg(feature = "test-support")]
pub mod test_support {
    //! Immutable observations for headless presentation tests.

    use bevy::input_focus::{
        tab_navigation::{TabGroup, TabIndex},
        InputFocus,
    };
    use bevy::math::Affine2;
    use bevy::prelude::*;
    use bevy::ui_widgets::ScrollArea;
    use bevy::window::WindowResolution;
    use std::collections::HashSet;

    use crate::{ActionPriority, ResolvedUiMetrics};

    /// Renderer-free plugin for exercising the real UI schedules and layout tree.
    ///
    /// Install this on an otherwise empty [`App`]. It creates one synthetic primary
    /// window, the stable Bevy UI/input/text stack, application states, and
    /// [`crate::UiPlugin`], but never initializes Winit, a renderer, or gameplay.
    pub struct HeadlessUiPlugin {
        physical_size: UVec2,
        scale_factor: f32,
    }

    impl HeadlessUiPlugin {
        /// Builds a headless UI canvas with an exact logical size.
        #[must_use]
        pub const fn new(width: u32, height: u32) -> Self {
            Self {
                physical_size: UVec2::new(width, height),
                scale_factor: 1.0,
            }
        }

        /// Builds a headless canvas from physical client pixels and the OS DPI
        /// scale factor reported for that window.
        #[must_use]
        pub const fn with_scale_factor(
            physical_width: u32,
            physical_height: u32,
            scale_factor: f32,
        ) -> Self {
            Self {
                physical_size: UVec2::new(physical_width, physical_height),
                scale_factor,
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
            assert!(
                self.scale_factor.is_finite() && self.scale_factor > 0.0,
                "headless UI scale factor must be finite and positive"
            );
            app.add_plugins((
                MinimalPlugins,
                bevy::transform::TransformPlugin,
                bevy::camera::visibility::VisibilityPlugin,
                bevy::input::InputPlugin,
                bevy::input_focus::InputFocusPlugin,
                bevy::input_focus::InputDispatchPlugin,
                bevy::window::WindowPlugin {
                    primary_window: Some(Window {
                        resolution: WindowResolution::new(
                            self.physical_size.x,
                            self.physical_size.y,
                        )
                        .with_scale_factor_override(self.scale_factor),
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
            let physical_size = self.physical_size;
            let scale_factor = self.scale_factor;
            app.add_systems(Startup, move |mut commands: Commands| {
                commands.spawn((
                    Camera2d,
                    bevy::camera::Camera {
                        computed: bevy::camera::ComputedCameraValues {
                            target_info: Some(bevy::camera::RenderTargetInfo {
                                physical_size,
                                scale_factor,
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

    /// One presented named UI node and its presentation-only facts.
    #[derive(Debug, Clone, PartialEq)]
    pub struct UiNodeObservation {
        /// Stable entity name used by review and test automation.
        pub name: String,
        /// Whether any part of the node is visible after ancestor and canvas clipping.
        pub visible: bool,
        /// Computed logical size when Bevy layout has run.
        pub size: Vec2,
        /// Computed logical content size before clipping.
        pub content_size: Vec2,
        /// Computed logical center in the UI camera's coordinate space.
        pub center: Vec2,
        /// Effective visible rectangle after inherited clipping and the canvas edge.
        pub visible_bounds: Option<Rect>,
        /// Actual glyph bounds for a named text node, when text has been laid out.
        pub rendered_text_bounds: Option<Rect>,
        /// Whether the complete node rectangle is currently visible.
        pub fully_visible: bool,
        /// First clipping ancestor, when inherited clipping reduces the visible rectangle.
        pub clipped_by: Option<String>,
        /// Whether the complete node can be brought into view through its scroll ancestors.
        pub scroll_reachable: bool,
        /// Whether this control must be visible immediately or may use scrolling.
        pub visibility_requirement: crate::UiVisibilityRequirement,
        /// Accessible label supplied to assistive technology.
        pub accessible_label: Option<String>,
        /// Explicit tab order, when this node is focusable.
        pub tab_index: Option<i32>,
        /// Whether laid-out content exceeds this node's box on either axis.
        pub overflows: bool,
        /// Whether this node currently has keyboard focus.
        pub focused: bool,
        /// Whether the node participates in keyboard or pointer interaction.
        pub focusable: bool,
        /// Whether this exact node instance belongs to the active tab sequence.
        pub in_focus_order: bool,
        /// Whether an enabled control belongs to the active keyboard focus scope.
        pub keyboard_reachable: Option<bool>,
        /// Whether an interactive node meets the 44×44 logical target minimum.
        pub meets_minimum_target: Option<bool>,
        /// Whether the persistent action rail geometrically obscures this control.
        pub obscured_by_action_rail: Option<Vec2>,
        /// Whether transparent corners intentionally tessellate with sibling controls.
        pub tessellated: bool,
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

    /// Most recent post-layout tree observation for live review automation.
    #[derive(Resource, Debug, Clone, Default)]
    pub struct LatestUiTreeSnapshot(pub Option<UiTreeSnapshot>);

    pub(crate) fn publish_ui_tree_snapshot(world: &mut World) {
        let snapshot = ui_tree_snapshot(world);
        world.resource_mut::<LatestUiTreeSnapshot>().0 = Some(snapshot);
    }

    impl UiTreeSnapshot {
        /// Returns structural failures for interactive controls in the current tree.
        ///
        /// Scroll-offscreen controls are accepted only when every clipping boundary
        /// can bring the complete 44×44 target into view. This deliberately does not
        /// infer any gameplay fact from rendered text.
        #[must_use]
        pub fn layout_issues(&self) -> Vec<String> {
            let mut issues = Vec::new();
            for node in self.nodes.iter().filter(|node| {
                !node.focusable
                    && node.visibility_requirement == crate::UiVisibilityRequirement::Immediate
            }) {
                if node.size.x <= 0.5 || node.size.y <= 0.5 {
                    issues.push(format!("{} has zero layout area", node.name));
                } else if !node.fully_visible {
                    issues.push(format!(
                        "{} is required presentation but is not fully visible in the initial viewport",
                        node.name
                    ));
                }
                if node.overflows {
                    issues.push(format!(
                        "{} has presentation content outside its box",
                        node.name
                    ));
                }
            }
            for node in self.nodes.iter().filter(|node| node.focusable) {
                if node.size.x <= 0.5 || node.size.y <= 0.5 {
                    issues.push(format!("{} has zero layout area", node.name));
                } else if node.visibility_requirement == crate::UiVisibilityRequirement::Immediate
                    && !node.fully_visible
                {
                    issues.push(format!(
                        "{} is a primary control but is not fully visible in the initial viewport; box {:.1}×{:.1} at ({:.1}, {:.1}), visible {:?}{}",
                        node.name,
                        node.size.x,
                        node.size.y,
                        node.center.x,
                        node.center.y,
                        node.visible_bounds,
                        node.clipped_by
                            .as_deref()
                            .map_or_else(String::new, |clip| format!(" (clipped by {clip})")),
                    ));
                } else if !node.scroll_reachable {
                    issues.push(format!(
                        "{} is clipped or off-canvas without a reachable scroll path{}; box {:.1}×{:.1} at ({:.1}, {:.1})",
                        node.name,
                        node.clipped_by
                            .as_deref()
                            .map_or_else(String::new, |clip| format!(" (clipped by {clip})")),
                        node.size.x,
                        node.size.y,
                        node.center.x,
                        node.center.y,
                    ));
                }
                if node.accessible_label.as_deref().is_none_or(str::is_empty) {
                    issues.push(format!("{} has no accessible label", node.name));
                }
                if node.keyboard_reachable == Some(false) {
                    issues.push(format!(
                        "{} is enabled but absent from the active focus order",
                        node.name
                    ));
                }
                if node.meets_minimum_target == Some(false) {
                    issues.push(format!(
                        "{} is {:.1}×{:.1}, below the 44×44 target minimum",
                        node.name, node.size.x, node.size.y
                    ));
                }
                if node.overflows {
                    issues.push(format!(
                        "{} has interactive content outside its box; content {:.1}×{:.1} versus box {:.1}×{:.1}",
                        node.name,
                        node.content_size.x,
                        node.content_size.y,
                        node.size.x,
                        node.size.y,
                    ));
                }
                if let Some(overlap) = node.obscured_by_action_rail {
                    issues.push(format!(
                        "{} is obscured by the persistent action rail by {:.1}×{:.1}",
                        node.name, overlap.x, overlap.y
                    ));
                }
            }
            let visible_focusable = self
                .nodes
                .iter()
                .filter(|node| node.in_focus_order && node.visible)
                .collect::<Vec<_>>();
            for (index, left) in visible_focusable.iter().enumerate() {
                for right in visible_focusable.iter().skip(index + 1) {
                    if left.tessellated && right.tessellated {
                        continue;
                    }
                    let (Some(left_bounds), Some(right_bounds)) =
                        (left.visible_bounds, right.visible_bounds)
                    else {
                        continue;
                    };
                    let overlap = left_bounds.intersect(right_bounds);
                    if overlap.width() > 0.5 && overlap.height() > 0.5 {
                        issues.push(format!(
                            "{} overlaps {} by {:.1}×{:.1}",
                            left.name,
                            right.name,
                            overlap.width(),
                            overlap.height()
                        ));
                    }
                }
            }
            issues
        }
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
        let active_modal = active_modal_group(world, focused);
        let action_rail = {
            let mut query = world.query_filtered::<Entity, With<crate::action_rail::ActionRail>>();
            query
                .iter(world)
                .find_map(|entity| node_bounds(world, entity).map(|bounds| (entity, bounds)))
        };
        let focus_entries = logical_focus_order(world, focused);
        let focus_entities = focus_entries
            .iter()
            .map(|(entity, _)| *entity)
            .collect::<HashSet<_>>();
        let focus_order = focus_entries
            .into_iter()
            .map(|(_, name)| name)
            .collect::<Vec<_>>();
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
                let size =
                    computed.map_or(Vec2::ZERO, |node| node.size() * node.inverse_scale_factor);
                let center = world
                    .get::<bevy::ui::UiGlobalTransform>(entity)
                    .map_or(Vec2::ZERO, |transform| {
                        transform.affine().translation * inverse_scale
                    });
                let bounds = Rect::from_center_size(center, size);
                let rendered_text_bounds = rendered_text_bounds(world, entity);
                let presented_bounds = rendered_text_bounds.unwrap_or(bounds);
                let visible_bounds =
                    effective_visible_bounds(world, entity, presented_bounds, metrics);
                let fully_visible = rect_contains(
                    Rect::from_corners(Vec2::ZERO, metrics.logical_size),
                    presented_bounds,
                ) && world.get::<CalculatedClip>(entity).is_none_or(|clip| {
                    rect_contains(scale_rect(clip.clip, inverse_scale), presented_bounds)
                });
                let focusable = world.get::<Button>(entity).is_some()
                    || world
                        .get::<TabIndex>(entity)
                        .is_some_and(|index| index.0 >= 0);
                let enabled_in_active_scope = focusable
                    && world.get::<bevy::ui::InteractionDisabled>(entity).is_none()
                    && active_modal.is_none_or(|modal| is_descendant_or_self(world, entity, modal));
                let in_active_scope =
                    active_modal.is_none_or(|modal| is_descendant_or_self(world, entity, modal));
                // A true modal owns the interaction and paint plane above the rail.
                // The rail must not cover ordinary gameplay/drawer controls, but
                // geometrically intersecting a higher-z modal is not occlusion.
                let obscured_by_action_rail = if active_modal.is_none()
                    && focusable
                    && in_active_scope
                {
                    action_rail.and_then(|(rail, rail_bounds)| {
                        if is_descendant_or_self(world, entity, rail) {
                            None
                        } else {
                            visible_bounds
                                .and_then(|visible| non_empty_intersection(visible, rail_bounds))
                                .map(|overlap| overlap.size())
                        }
                    })
                } else {
                    None
                };
                Some(UiNodeObservation {
                    name: name.as_str().to_owned(),
                    visible: visible_bounds.is_some(),
                    size,
                    content_size: computed.map_or(Vec2::ZERO, |node| {
                        node.content_size() * node.inverse_scale_factor
                    }),
                    center,
                    visible_bounds,
                    rendered_text_bounds,
                    fully_visible,
                    clipped_by: first_clipping_ancestor(world, entity, presented_bounds),
                    scroll_reachable: scroll_reachable(world, entity, presented_bounds, metrics),
                    visibility_requirement: world
                        .get::<crate::UiVisibilityRequirement>(entity)
                        .copied()
                        .unwrap_or(crate::UiVisibilityRequirement::Scrollable),
                    accessible_label: world
                        .get::<AccessibleLabel>(entity)
                        .map(|label| label.0.clone()),
                    tab_index: world.get::<TabIndex>(entity).map(|index| index.0),
                    overflows: computed.is_some_and(|node| {
                        // Yoga text measurement can extend a few logical pixels
                        // beyond the border box for glyph overhang and borders.
                        // Keep the tolerance logical so 1× and Retina inputs use
                        // the same oracle; larger layout overflow still fails.
                        let epsilon = 10.0;
                        let content = node.content_size() * node.inverse_scale_factor;
                        let size = node.size() * node.inverse_scale_factor;
                        content.x > size.x + epsilon || content.y > size.y + epsilon
                    }),
                    focused: focused == Some(entity),
                    focusable,
                    in_focus_order: focus_entities.contains(&entity),
                    keyboard_reachable: enabled_in_active_scope
                        .then_some(focus_entities.contains(&entity)),
                    // Yoga rounds scaled edges to physical pixels. Permit at most
                    // half a logical pixel so an authored 44px target does not fail
                    // solely because a fractional Auto scale rasterizes to 43.5px.
                    meets_minimum_target: focusable
                        .then_some(size.x + 0.51 >= 44.0 && size.y + 0.51 >= 44.0),
                    obscured_by_action_rail,
                    tessellated: world
                        .get::<crate::lattice::TessellatedControl>(entity)
                        .is_some(),
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

    fn node_bounds(world: &World, entity: Entity) -> Option<Rect> {
        let computed = world.get::<ComputedNode>(entity)?;
        let transform = world.get::<bevy::ui::UiGlobalTransform>(entity)?;
        let inverse_scale = computed.inverse_scale_factor;
        Some(Rect::from_center_size(
            transform.affine().translation * inverse_scale,
            computed.size() * inverse_scale,
        ))
    }

    fn rendered_text_bounds(world: &World, entity: Entity) -> Option<Rect> {
        let computed = world.get::<ComputedNode>(entity)?;
        let transform = world.get::<bevy::ui::UiGlobalTransform>(entity)?;
        let layout = world.get::<bevy::text::TextLayoutInfo>(entity)?;
        let local_to_world =
            Affine2::from(*transform) * Affine2::from_translation(computed.content_box().min);
        let inverse_scale = computed.inverse_scale_factor;
        layout
            .glyphs
            .iter()
            .map(|glyph| {
                let half_size = glyph.atlas_info.rect.size() * 0.5;
                let local_min = glyph.position - half_size;
                let local_max = glyph.position + half_size;
                let first = local_to_world.transform_point2(local_min) * inverse_scale;
                [
                    local_to_world.transform_point2(Vec2::new(local_max.x, local_min.y)),
                    local_to_world.transform_point2(local_max),
                    local_to_world.transform_point2(Vec2::new(local_min.x, local_max.y)),
                ]
                .into_iter()
                .fold(Rect::from_corners(first, first), |mut bounds, point| {
                    let point = point * inverse_scale;
                    bounds.min = bounds.min.min(point);
                    bounds.max = bounds.max.max(point);
                    bounds
                })
            })
            .reduce(|left, right| {
                Rect::from_corners(left.min.min(right.min), left.max.max(right.max))
            })
    }

    fn scale_rect(rect: Rect, scale: f32) -> Rect {
        Rect::from_corners(rect.min * scale, rect.max * scale)
    }

    fn rect_contains(outer: Rect, inner: Rect) -> bool {
        const EPSILON: f32 = 0.5;
        inner.min.x >= outer.min.x - EPSILON
            && inner.min.y >= outer.min.y - EPSILON
            && inner.max.x <= outer.max.x + EPSILON
            && inner.max.y <= outer.max.y + EPSILON
    }

    fn non_empty_intersection(left: Rect, right: Rect) -> Option<Rect> {
        let intersection = left.intersect(right);
        (intersection.width() > 0.5 && intersection.height() > 0.5).then_some(intersection)
    }

    fn effective_visible_bounds(
        world: &World,
        entity: Entity,
        bounds: Rect,
        metrics: ResolvedUiMetrics,
    ) -> Option<Rect> {
        let canvas = Rect::from_corners(Vec2::ZERO, metrics.logical_size);
        let canvas_visible = non_empty_intersection(bounds, canvas)?;
        let Some(computed) = world.get::<ComputedNode>(entity) else {
            return Some(canvas_visible);
        };
        world
            .get::<CalculatedClip>(entity)
            .map_or(Some(canvas_visible), |clip| {
                non_empty_intersection(
                    canvas_visible,
                    scale_rect(clip.clip, computed.inverse_scale_factor),
                )
            })
    }

    fn first_clipping_ancestor(world: &World, entity: Entity, bounds: Rect) -> Option<String> {
        let mut current = entity;
        while let Some(parent) = world.get::<ChildOf>(current).map(ChildOf::parent) {
            if let (Some(node), Some(parent_bounds)) =
                (world.get::<Node>(parent), node_bounds(world, parent))
            {
                let clips_x = !node.overflow.x.is_visible()
                    && (bounds.min.x < parent_bounds.min.x - 0.5
                        || bounds.max.x > parent_bounds.max.x + 0.5);
                let clips_y = !node.overflow.y.is_visible()
                    && (bounds.min.y < parent_bounds.min.y - 0.5
                        || bounds.max.y > parent_bounds.max.y + 0.5);
                if clips_x || clips_y {
                    return Some(
                        world
                            .get::<Name>(parent)
                            .map_or_else(|| format!("Entity {parent:?}"), |name| name.to_string()),
                    );
                }
            }
            current = parent;
        }
        None
    }

    fn scroll_reachable(
        world: &World,
        entity: Entity,
        bounds: Rect,
        metrics: ResolvedUiMetrics,
    ) -> bool {
        if bounds.width() <= 0.5 || bounds.height() <= 0.5 {
            return false;
        }
        axis_reachable(world, entity, bounds, metrics.logical_size, true)
            && axis_reachable(world, entity, bounds, metrics.logical_size, false)
    }

    fn axis_reachable(
        world: &World,
        entity: Entity,
        bounds: Rect,
        canvas: Vec2,
        horizontal: bool,
    ) -> bool {
        let (target_min, target_max, target_length, canvas_max) = if horizontal {
            (bounds.min.x, bounds.max.x, bounds.width(), canvas.x)
        } else {
            (bounds.min.y, bounds.max.y, bounds.height(), canvas.y)
        };
        let mut current = entity;
        let mut candidate_min = target_min;
        let mut candidate_max = target_max;
        while let Some(parent) = world.get::<ChildOf>(current).map(ChildOf::parent) {
            if let (Some(node), Some(parent_bounds)) =
                (world.get::<Node>(parent), node_bounds(world, parent))
            {
                let axis = if horizontal {
                    node.overflow.x
                } else {
                    node.overflow.y
                };
                let (parent_min, parent_max, parent_length) = if horizontal {
                    (
                        parent_bounds.min.x,
                        parent_bounds.max.x,
                        parent_bounds.width(),
                    )
                } else {
                    (
                        parent_bounds.min.y,
                        parent_bounds.max.y,
                        parent_bounds.height(),
                    )
                };
                let outside = candidate_min < parent_min - 0.5 || candidate_max > parent_max + 0.5;
                if outside {
                    match axis {
                        OverflowAxis::Visible => {}
                        OverflowAxis::Scroll
                            if world.get::<ScrollArea>(parent).is_some()
                                && world.get::<ScrollPosition>(parent).is_some()
                                && target_length <= parent_length + 0.5 =>
                        {
                            // Once this scroll viewport can reveal the target, outer
                            // clippers constrain the viewport rather than the target's
                            // current offscreen coordinates.
                            candidate_min = parent_min;
                            candidate_max = parent_max;
                        }
                        OverflowAxis::Scroll | OverflowAxis::Clip | OverflowAxis::Hidden => {
                            return false;
                        }
                    }
                }
            }
            current = parent;
        }

        candidate_min >= -0.5 && candidate_max <= canvas_max + 0.5
    }

    fn logical_focus_order(world: &mut World, focused: Option<Entity>) -> Vec<(Entity, String)> {
        let mut groups = world
            .query::<(Entity, &TabGroup)>()
            .iter(world)
            .map(|(entity, group)| (entity, *group))
            .filter(|(entity, _)| is_presented(world, *entity))
            .collect::<Vec<_>>();
        groups.sort_by_key(|(entity, group)| (group.order, entity.to_bits()));

        let active_modal = active_modal_group_from_groups(world, focused, &groups);
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
            within_group.sort_by_key(|(index, position, _, _)| (*index, *position));
            order.extend(
                within_group
                    .into_iter()
                    .map(|(_, _, entity, name)| (entity, name)),
            );
        }
        order
    }

    fn active_modal_group(world: &mut World, focused: Option<Entity>) -> Option<Entity> {
        let mut groups = world
            .query::<(Entity, &TabGroup)>()
            .iter(world)
            .map(|(entity, group)| (entity, *group))
            .filter(|(entity, _)| is_presented(world, *entity))
            .collect::<Vec<_>>();
        groups.sort_by_key(|(entity, group)| (group.order, entity.to_bits()));
        active_modal_group_from_groups(world, focused, &groups)
    }

    fn active_modal_group_from_groups(
        world: &World,
        focused: Option<Entity>,
        groups: &[(Entity, TabGroup)],
    ) -> Option<Entity> {
        focused
            .and_then(|focused| {
                groups
                    .iter()
                    .find(|(group, settings)| {
                        settings.modal && is_descendant_or_self(world, focused, *group)
                    })
                    .map(|(group, _)| *group)
            })
            .or_else(|| {
                groups
                    .iter()
                    .rev()
                    .find_map(|(group, settings)| settings.modal.then_some(*group))
            })
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
        output: &mut Vec<(i32, usize, Entity, String)>,
    ) {
        if entity != group && world.get::<TabGroup>(entity).is_some() {
            return;
        }
        let visible = is_presented(world, entity);
        if visible && world.get::<bevy::ui::InteractionDisabled>(entity).is_none() {
            match (world.get::<TabIndex>(entity), world.get::<Name>(entity)) {
                (Some(index), Some(name)) if index.0 >= 0 => output.push((
                    index.0,
                    *hierarchy_position,
                    entity,
                    name.as_str().to_owned(),
                )),
                _ => {}
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

        fn all_scale_modes() -> [crate::UiScaleMode; 7] {
            [
                crate::UiScaleMode::Auto,
                crate::UiScaleMode::Percent75,
                crate::UiScaleMode::Percent100,
                crate::UiScaleMode::Percent125,
                crate::UiScaleMode::Percent150,
                crate::UiScaleMode::Percent175,
                crate::UiScaleMode::Percent200,
            ]
        }

        fn structural_canvases() -> [(UVec2, f32); 12] {
            [
                (UVec2::new(960, 540), 1.0),
                (UVec2::new(1280, 720), 1.0),
                (UVec2::new(1512, 949), 1.0),
                (UVec2::new(1920, 1080), 1.0),
                (UVec2::new(2560, 1440), 1.0),
                (UVec2::new(3840, 2160), 1.0),
                (UVec2::new(1920, 1080), 2.0),
                (UVec2::new(2560, 1440), 2.0),
                (UVec2::new(3024, 1898), 2.0),
                (UVec2::new(3840, 2160), 2.0),
                (UVec2::new(5120, 2880), 2.0),
                (UVec2::new(7680, 4320), 2.0),
            ]
        }

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
            required_choice_snapshot_at_scale(width, height, 1.0, mode)
        }

        fn required_choice_snapshot_at_scale(
            physical_width: u32,
            physical_height: u32,
            scale_factor: f32,
            mode: crate::UiScaleMode,
        ) -> UiTreeSnapshot {
            let mut app = App::new();
            app.add_plugins(HeadlessUiPlugin::with_scale_factor(
                physical_width,
                physical_height,
                scale_factor,
            ));
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

        fn production_title_snapshot(
            physical_width: u32,
            physical_height: u32,
            scale_factor: f32,
        ) -> UiTreeSnapshot {
            let mut app = App::new();
            app.add_plugins(HeadlessUiPlugin::with_scale_factor(
                physical_width,
                physical_height,
                scale_factor,
            ));
            app.world_mut().insert_resource(crate::TitleView::default());
            app.world_mut()
                .resource_mut::<NextState<hex_core::Screen>>()
                .set(hex_core::Screen::Title);
            for _ in 0..8 {
                app.update();
            }
            ui_tree_snapshot(app.world_mut())
        }

        fn production_scenario_snapshot(
            physical_width: u32,
            physical_height: u32,
            scale_factor: f32,
        ) -> UiTreeSnapshot {
            let library: hex_assets::ScenarioLibrary =
                ron::from_str(include_str!("../../../assets/config/scenarios.ron"))
                    .expect("the production scenario catalog must parse");
            let mut app = App::new();
            app.add_plugins(HeadlessUiPlugin::with_scale_factor(
                physical_width,
                physical_height,
                scale_factor,
            ));
            app.world_mut().insert_resource(crate::ScenarioBrowserView {
                scenarios: library
                    .visible_scenarios()
                    .cloned()
                    .map(|scenario| crate::TitleScenarioView {
                        resolved_seed: scenario.generation_seed,
                        scenario,
                    })
                    .collect(),
            });
            app.world_mut()
                .resource_mut::<NextState<hex_core::Screen>>()
                .set(hex_core::Screen::Scenarios);
            for _ in 0..8 {
                app.update();
            }
            ui_tree_snapshot(app.world_mut())
        }

        fn gameplay_fixture_snapshot(
            physical_width: u32,
            physical_height: u32,
            scale_factor: f32,
            mode: crate::UiScaleMode,
            fixture: &str,
        ) -> UiTreeSnapshot {
            let mut app = App::new();
            app.add_plugins(HeadlessUiPlugin::with_scale_factor(
                physical_width,
                physical_height,
                scale_factor,
            ));
            app.world_mut()
                .insert_resource(crate::UiScalePreference(mode));
            app.world_mut().insert_resource(crate::GameplayChromeView {
                shown: true,
                decision_required: fixture == "required-decision",
                encounter_complete: fixture == "dense-report-compare",
            });
            let mut queue = bevy::ecs::world::CommandQueue::default();
            let mut commands = Commands::new(&mut queue, app.world());
            crate::apply_ui_review_fixture(&mut commands, fixture)
                .expect("the structural fixture name must be valid");
            queue.apply(app.world_mut());
            app.world_mut()
                .resource_mut::<NextState<hex_core::Screen>>()
                .set(hex_core::Screen::Gameplay);
            for _ in 0..8 {
                app.update();
            }
            ui_tree_snapshot(app.world_mut())
        }

        fn setup_screen_snapshot(
            physical_width: u32,
            physical_height: u32,
            scale_factor: f32,
            mode: crate::UiScaleMode,
            screen: hex_core::Screen,
        ) -> UiTreeSnapshot {
            let mut app = App::new();
            app.add_plugins(HeadlessUiPlugin::with_scale_factor(
                physical_width,
                physical_height,
                scale_factor,
            ));
            app.world_mut()
                .insert_resource(crate::UiScalePreference(mode));
            match screen {
                hex_core::Screen::Title => {
                    app.world_mut().insert_resource(crate::TitleView::default());
                }
                hex_core::Screen::Scenarios => {
                    let library: hex_assets::ScenarioLibrary =
                        ron::from_str(include_str!("../../../assets/config/scenarios.ron"))
                            .expect("the production scenario catalog must parse");
                    app.world_mut().insert_resource(crate::ScenarioBrowserView {
                        scenarios: library
                            .visible_scenarios()
                            .cloned()
                            .map(|scenario| crate::TitleScenarioView {
                                resolved_seed: scenario.generation_seed,
                                scenario,
                            })
                            .collect(),
                    });
                }
                hex_core::Screen::Settings => {
                    use crate::UiSetting::{
                        EffectsVolume, Fullscreen, MasterVolume, MusicVolume, Presentation,
                        UiScale, UiVolume, WindowSize,
                    };
                    app.world_mut().insert_resource(crate::UiSettingsView {
                        rows: [
                            (Fullscreen, "Display mode", "Windowed"),
                            (WindowSize, "Window size", "1920 × 1080"),
                            (Presentation, "Presentation", "VSync"),
                            (UiScale, "UI scale", "200%"),
                            (MasterVolume, "Master volume", "100%"),
                            (MusicVolume, "Music volume", "80%"),
                            (EffectsVolume, "Effects volume", "90%"),
                            (UiVolume, "UI volume", "90%"),
                        ]
                        .into_iter()
                        .map(|(setting, label, value)| crate::UiSettingRow {
                            setting,
                            label: label.to_owned(),
                            value: value.to_owned(),
                        })
                        .collect(),
                        notice: Some(
                            "Settings save immediately and persist after restart.".to_owned(),
                        ),
                    });
                }
                hex_core::Screen::CharacterCreator => {
                    let element_file: hex_assets::ElementFile =
                        ron::from_str(include_str!("../../../assets/config/elements.ron"))
                            .expect("the production element catalog must parse");
                    let elements = hex_assets::ElementCatalog::from_file(&element_file);
                    let spell_file: hex_assets::SpellFile =
                        ron::from_str(include_str!("../../../assets/config/spells.ron"))
                            .expect("the production spell catalog must parse");
                    let spell_book = hex_assets::SpellBook::from_file(&spell_file);
                    let deployable_shipped_spells = spell_book
                        .iter()
                        .filter(|(_, _, spell)| {
                            matches!(
                                spell.targeting.shape,
                                hex_assets::TargetShape::SelfCast | hex_assets::TargetShape::Single
                            )
                        })
                        .map(|(_, name, _)| name.to_owned())
                        .collect();
                    app.world_mut().insert_resource(crate::CreatorScreenView {
                        active: true,
                        screen,
                        workspace: crate::CreatorWorkspace::Character,
                        character: Some(hex_assets::SavedCharacter::blank(
                            hex_assets::CustomCharacterId(1),
                            "Validation Fixture",
                        )),
                        notice: "Cannot save until every required field is valid.".to_owned(),
                        character_issues: vec![
                            "Add at least one elemental gem.".to_owned(),
                            "Assign a positive mana capacity.".to_owned(),
                        ],
                        character_dirty: true,
                        elements: Some(elements),
                        spell_book: Some(spell_book),
                        spell_file: Some(spell_file),
                        deployable_shipped_spells,
                        ..default()
                    });
                }
                hex_core::Screen::CombatLab => {
                    let maps: hex_assets::CombatLabMapCatalog =
                        ron::from_str(include_str!("../../../assets/config/combat_lab_maps.ron"))
                            .expect("the production Combat Lab map catalog must parse");
                    app.world_mut().insert_resource(crate::CombatLabScreenView {
                        active: true,
                        map: maps
                            .maps
                            .first()
                            .map_or_else(String::new, |map| map.id.clone()),
                        maps: Some(maps),
                        notice: "Choose a map before continuing to roster setup.".to_owned(),
                        ..default()
                    });
                }
                other => panic!("{other:?} is not a setup-screen fixture"),
            }
            app.world_mut()
                .resource_mut::<NextState<hex_core::Screen>>()
                .set(screen);
            for _ in 0..8 {
                app.update();
            }
            ui_tree_snapshot(app.world_mut())
        }

        fn deployment_snapshot(
            physical_width: u32,
            physical_height: u32,
            scale_factor: f32,
            mode: crate::UiScaleMode,
        ) -> UiTreeSnapshot {
            let mut app = App::new();
            app.add_plugins(HeadlessUiPlugin::with_scale_factor(
                physical_width,
                physical_height,
                scale_factor,
            ));
            app.world_mut()
                .insert_resource(crate::UiScalePreference(mode));
            let roster = |side: &str| {
                (0..6)
                    .map(|index| crate::DeploymentRosterEntryView {
                        index,
                        name: format!("{side} Unit {}", index + 1),
                        selected: index == 0,
                        position: None,
                    })
                    .collect::<Vec<_>>()
            };
            app.world_mut().insert_resource(crate::DeploymentView {
                active: true,
                map_name: "Stacked Surface Arena".to_owned(),
                notice: "Place every player and hostile on an exact legal surface.".to_owned(),
                players: roster("Player"),
                hostiles: roster("Hostile"),
                complete: false,
            });
            app.world_mut().insert_resource(crate::GameplayHudView {
                phase: hex_core::GameplayPhase::Deployment,
                actor_label: "Deployment".to_owned(),
                round: "Setup".to_owned(),
                ..default()
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
            let snapshot = required_choice_snapshot(1280, 720, crate::UiScaleMode::Percent200);
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
                UVec2::new(1512, 949),
                UVec2::new(1920, 1080),
                UVec2::new(2560, 1440),
                UVec2::new(3840, 2160),
            ] {
                for mode in all_scale_modes() {
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
                        if required != "Primary Action Rail" {
                            assert!(
                                !node.overflows,
                                "{required:?} must not overflow at {logical_size:?} in {mode:?}: {node:?}"
                            );
                        }
                        let half = node.size * 0.5;
                        let min = node.center - half;
                        let max = node.center + half;
                        assert!(
                            min.cmpge(Vec2::ZERO).all()
                                && max.cmple(snapshot.metrics.logical_size).all(),
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
        fn retina_mappings_use_physical_pixels_and_os_scale_as_separate_inputs() {
            for (physical_size, scale_factor, expected_logical_size, expected_viewport) in [
                (
                    UVec2::new(2560, 1440),
                    2.0,
                    Vec2::new(1280.0, 720.0),
                    crate::UiViewportClass::Compact,
                ),
                (
                    UVec2::new(3024, 1898),
                    2.0,
                    Vec2::new(1512.0, 949.0),
                    crate::UiViewportClass::Standard,
                ),
            ] {
                let snapshot = required_choice_snapshot_at_scale(
                    physical_size.x,
                    physical_size.y,
                    scale_factor,
                    crate::UiScaleMode::Auto,
                );
                assert_eq!(snapshot.metrics.logical_size, expected_logical_size);
                assert_eq!(snapshot.metrics.viewport, expected_viewport);
                assert!(
                    snapshot.layout_issues().is_empty(),
                    "required UI must remain structurally reachable at {physical_size:?} / {scale_factor}×: {:?}",
                    snapshot.layout_issues()
                );
            }
        }

        #[test]
        fn primary_title_routes_are_initially_visible_in_compact_retina_windows() {
            for (physical_size, scale_factor) in
                [(UVec2::new(1280, 720), 1.0), (UVec2::new(2560, 1440), 2.0)]
            {
                let snapshot =
                    production_title_snapshot(physical_size.x, physical_size.y, scale_factor);
                assert_eq!(snapshot.metrics.viewport, crate::UiViewportClass::Compact);
                assert!(
                    snapshot.layout_issues().is_empty(),
                    "primary title routes must remain visible at {physical_size:?} / {scale_factor}×: {:?}",
                    snapshot.layout_issues()
                );
                for required in [
                    "Continue",
                    "New Game",
                    "Creators",
                    "Combat Lab",
                    "Scenarios",
                    "Settings",
                    "Quit",
                ] {
                    let Some(node) = snapshot.nodes.iter().find(|node| node.name == required)
                    else {
                        panic!("full production title is missing {required:?}");
                    };
                    assert!(
                        node.fully_visible,
                        "{required:?} must be initially visible at {physical_size:?} / {scale_factor}×: {node:?}"
                    );
                }
            }
        }

        #[test]
        fn production_scenario_catalog_has_one_outer_scroll_path() {
            let snapshot = production_scenario_snapshot(1280, 720, 1.0);
            assert!(
                snapshot.layout_issues().is_empty(),
                "the production scenario catalog must remain reachable: {:?}; immediate scenario nodes: {:?}",
                snapshot.layout_issues(),
                snapshot
                    .nodes
                    .iter()
                    .filter(|node| node.name.starts_with("Scenario Screen"))
                    .collect::<Vec<_>>()
            );
            let back = snapshot
                .nodes
                .iter()
                .find(|node| node.name == "Back")
                .expect("scenario catalog has a Back control");
            assert!(back.fully_visible);
            let title = snapshot
                .nodes
                .iter()
                .find(|node| node.name == "Scenario Screen Title")
                .expect("scenario catalog has a named screen title");
            assert!(
                title.rendered_text_bounds.is_some() && title.fully_visible,
                "the actual title glyphs must fit the initial canvas: {title:?}"
            );
            for scenario in ["The Crossing", "Waterfall"] {
                let node = snapshot
                    .nodes
                    .iter()
                    .find(|node| node.name == scenario)
                    .unwrap_or_else(|| panic!("production catalog is missing {scenario}"));
                assert!(node.scroll_reachable);
            }
        }

        #[test]
        fn gameplay_presentation_states_pass_the_complete_structural_matrix() {
            for fixture in [
                "normal-gameplay",
                "required-decision",
                "aiming-disabled",
                "live-statistics",
                "dense-report-compare",
            ] {
                for (physical_size, scale_factor) in structural_canvases() {
                    for mode in all_scale_modes() {
                        let snapshot = gameplay_fixture_snapshot(
                            physical_size.x,
                            physical_size.y,
                            scale_factor,
                            mode,
                            fixture,
                        );
                        assert!(
                            snapshot.layout_issues().is_empty(),
                            "{fixture} must remain reachable at {physical_size:?} / {scale_factor}× in {mode:?}: {:?}; rail nodes: {:?}",
                            snapshot.layout_issues(),
                            snapshot
                                .nodes
                                .iter()
                                .filter(|node| node.name.contains("Action Rail"))
                                .collect::<Vec<_>>()
                        );
                    }
                }
            }
        }

        #[test]
        fn setup_and_deployment_surfaces_pass_the_complete_structural_matrix() {
            for (physical_size, scale_factor) in structural_canvases() {
                for mode in all_scale_modes() {
                    for screen in [
                        hex_core::Screen::Title,
                        hex_core::Screen::Scenarios,
                        hex_core::Screen::Settings,
                        hex_core::Screen::CharacterCreator,
                        hex_core::Screen::CombatLab,
                    ] {
                        let snapshot = setup_screen_snapshot(
                            physical_size.x,
                            physical_size.y,
                            scale_factor,
                            mode,
                            screen,
                        );
                        assert!(
                            snapshot.layout_issues().is_empty(),
                            "{screen:?} must remain reachable at {physical_size:?} / {scale_factor}× in {mode:?}: {:?}",
                            snapshot.layout_issues()
                        );
                    }
                    let snapshot =
                        deployment_snapshot(physical_size.x, physical_size.y, scale_factor, mode);
                    assert!(
                        snapshot.layout_issues().is_empty(),
                        "6v6 deployment must remain reachable at {physical_size:?} / {scale_factor}× in {mode:?}: {:?}",
                        snapshot.layout_issues()
                    );
                }
            }
        }

        #[test]
        fn production_creator_catalog_uses_an_operable_standard_scroll_owner() {
            let snapshot = setup_screen_snapshot(
                1600,
                900,
                1.0,
                crate::UiScaleMode::Auto,
                hex_core::Screen::CharacterCreator,
            );
            assert_eq!(snapshot.metrics.viewport, crate::UiViewportClass::Standard);
            assert!(
                snapshot.layout_issues().is_empty(),
                "the production-populated creator must remain reachable: {:?}",
                snapshot.layout_issues()
            );
            let erase = snapshot
                .nodes
                .iter()
                .find(|node| node.name == "Erase")
                .expect("the production creator exposes its final palette action");
            assert!(
                !erase.fully_visible,
                "the fixture must exercise a control that actually needs scrolling"
            );
            assert!(
                erase.scroll_reachable,
                "the final production palette action must have an operable scroll path"
            );
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
            world.spawn((
                Name::new("Orphaned Control"),
                Button,
                TabIndex(0),
                AccessibleLabel::new("Control without an active focus group"),
                InheritedVisibility::VISIBLE,
            ));
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
            assert_eq!(confirm.keyboard_reachable, Some(true));
            let orphan = snapshot
                .nodes
                .iter()
                .find(|node| node.name == "Orphaned Control")
                .expect("the orphaned control remains structurally observable");
            assert_eq!(orphan.keyboard_reachable, Some(false));
            assert!(snapshot
                .layout_issues()
                .iter()
                .any(|issue| issue.contains("Orphaned Control is enabled but absent")));
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
        fn scroll_styling_without_a_scroll_owner_is_not_reachable() {
            let mut app = App::new();
            app.add_plugins(HeadlessUiPlugin::default());
            let scroller = app
                .world_mut()
                .spawn((
                    Name::new("Unowned Scroll Viewport"),
                    TabGroup::new(50),
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(20.0),
                        top: Val::Px(20.0),
                        width: Val::Px(100.0),
                        height: Val::Px(100.0),
                        overflow: Overflow::scroll_y(),
                        ..default()
                    },
                ))
                .id();
            let control = app
                .world_mut()
                .spawn((
                    Name::new("Clipped Scroll Control"),
                    AccessibleLabel::new("Clipped scroll control"),
                    Button,
                    TabIndex(0),
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(0.0),
                        top: Val::Px(120.0),
                        width: Val::Px(44.0),
                        height: Val::Px(44.0),
                        ..default()
                    },
                ))
                .id();
            app.world_mut().entity_mut(scroller).add_child(control);
            for _ in 0..4 {
                app.update();
            }

            let snapshot = ui_tree_snapshot(app.world_mut());
            let control = snapshot
                .nodes
                .iter()
                .find(|node| node.name == "Clipped Scroll Control")
                .expect("the clipped control remains structurally observable");
            assert!(!control.scroll_reachable);
            assert!(snapshot
                .layout_issues()
                .iter()
                .any(|issue| issue.contains("Clipped Scroll Control is clipped")));

            app.world_mut().entity_mut(scroller).insert(ScrollArea);
            for _ in 0..2 {
                app.update();
            }
            let snapshot = ui_tree_snapshot(app.world_mut());
            let control = snapshot
                .nodes
                .iter()
                .find(|node| node.name == "Clipped Scroll Control")
                .expect("the clipped control remains structurally observable");
            assert!(control.scroll_reachable);
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
