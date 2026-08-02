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
    ScenarioBrowserIntent, ScenarioBrowserKind, ScenarioBrowserView, TargetLatticeStateView,
    TargetLatticeView, TargetPulseView, TitleIntent, TitleScenarioView, TitleView, UiIntent,
    UiSetting, UiSettingRow, UiSettingsView, UnitBadgeView, UnitBadgesView,
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
pub(crate) use theme::{
    body_text_role, compact_glyph_role, fixed_row_button, hud_heading, hud_text_role,
    owner_resolved_control_role, responsive_control_role, supporting_text_role,
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
    /// Attach optional development tooling after persistent gameplay panels.
    Tooling,
    /// Attach secondary surfaces after the persistent readouts they follow.
    Secondary,
}

/// Initial-view requirement consumed by the structural presentation oracle.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiVisibilityRequirement {
    /// The control must be fully visible before any scrolling or drawer changes.
    Immediate,
    /// The control may begin offscreen when an operable scroll owner can reveal it.
    Scrollable,
}

/// Safe default carried by shared controls. A secondary surface must explicitly
/// replace this with [`UiVisibilityRequirement::Scrollable`].
#[derive(Component, Debug, Clone, Copy)]
pub(crate) struct DefaultImmediateControl;

/// Opt-in invariant for controls or headings whose descendant glyphs must stay
/// inside their own presentation box.
#[derive(Component, Debug, Clone, Copy)]
pub(crate) struct UiTextMustFit;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.configure_sets(
            OnEnter(hex_core::Screen::Gameplay),
            (
                UiHudSetup::Frame,
                UiHudSetup::Panels,
                UiHudSetup::Tooling,
                UiHudSetup::Secondary,
            )
                .chain(),
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
    use std::collections::{HashMap, HashSet};

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

    /// Returns the populated, maximum-normal casting projection used to exercise
    /// production rendering without installing a review override.
    #[must_use]
    pub fn populated_gameplay_casting() -> crate::CastingPanelView {
        crate::review::populated_casting()
    }

    /// Returns populated own and disclosed-target lattice projections used to
    /// exercise production rendering without installing a review override.
    #[must_use]
    pub fn populated_gameplay_lattices() -> crate::GameplayLatticesView {
        crate::review::populated_lattices()
    }

    /// One player task whose presentation must remain independently constructible.
    ///
    /// This is intentionally more granular than [`hex_core::Screen`]. A single
    /// screen can contain several materially different tasks and responsive risks.
    #[expect(
        missing_docs,
        reason = "variant meaning is documented by its public UiTaskContract"
    )]
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Deserialize)]
    pub enum UiTaskCase {
        Splash,
        Loading,
        TitleCold,
        TitleResume,
        TitleFailure,
        MapScenarios,
        Demos,
        Settings,
        CharacterLibrary,
        SpellLibrary,
        CreatorLibraryRecovery,
        CharacterInvalid,
        CharacterReady,
        CharacterConfirmDelete,
        SpellInvalid,
        SpellReady,
        SpellConfirmDelete,
        LatticeDemo,
        LabMap,
        LabRosters,
        LabRostersMax,
        LabRules,
        LabFixtures,
        LabReportsEmpty,
        LabReportsPopulated,
        DeploymentIncomplete,
        DeploymentComplete,
        Exploration,
        PlayerTurnMaxActions,
        HostileTurn,
        Casting,
        AimingBlocked,
        DisableDecision,
        RestoreDecision,
        HudHiddenRequired,
        LabStatistics,
        Pause,
        OrdinaryOutcome,
        LabReportOverview,
        LabReportUnits,
        LabReportSpellsEffects,
        LabReportTimeline,
        LabReportCompare,
    }

    /// Static acceptance facts for one [`UiTaskCase`].
    #[derive(Debug, Clone, Copy)]
    pub struct UiTaskContract {
        /// Stable diagnostic/fixture identity.
        pub id: &'static str,
        /// Bevy screen that owns the task.
        pub screen: hex_core::Screen,
        /// Controls that must be completely visible before scrolling.
        pub immediate_controls: &'static [&'static str],
        /// Representative secondary controls that must have a real scroll route.
        pub scrollable_controls: &'static [&'static str],
        /// Whether the case receives the exhaustive viewport/scale matrix.
        pub exhaustive_layout: bool,
    }

    /// Lattice presentation that must accompany a populated gameplay task.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum UiTaskLatticeRequirement {
        /// This task does not present live gameplay lattices.
        None,
        /// The persistent selected-player lattice must be populated.
        Own,
        /// Both the selected-player lattice and an authored disclosed target must be populated.
        OwnAndTarget,
        /// A blocking choice must present either its persistent own lattice or the
        /// promoted Compact choice used when the Inspector yields at extreme scales.
        RequiredChoice,
    }

    impl UiTaskCase {
        /// Every known task case. Adding a task requires adding it here and to the
        /// exhaustive contract match below.
        pub const ALL: [Self; 43] = [
            Self::Splash,
            Self::Loading,
            Self::TitleCold,
            Self::TitleResume,
            Self::TitleFailure,
            Self::MapScenarios,
            Self::Demos,
            Self::Settings,
            Self::CharacterLibrary,
            Self::SpellLibrary,
            Self::CreatorLibraryRecovery,
            Self::CharacterInvalid,
            Self::CharacterReady,
            Self::CharacterConfirmDelete,
            Self::SpellInvalid,
            Self::SpellReady,
            Self::SpellConfirmDelete,
            Self::LatticeDemo,
            Self::LabMap,
            Self::LabRosters,
            Self::LabRostersMax,
            Self::LabRules,
            Self::LabFixtures,
            Self::LabReportsEmpty,
            Self::LabReportsPopulated,
            Self::DeploymentIncomplete,
            Self::DeploymentComplete,
            Self::Exploration,
            Self::PlayerTurnMaxActions,
            Self::HostileTurn,
            Self::Casting,
            Self::AimingBlocked,
            Self::DisableDecision,
            Self::RestoreDecision,
            Self::HudHiddenRequired,
            Self::LabStatistics,
            Self::Pause,
            Self::OrdinaryOutcome,
            Self::LabReportOverview,
            Self::LabReportUnits,
            Self::LabReportSpellsEffects,
            Self::LabReportTimeline,
            Self::LabReportCompare,
        ];

        /// Fail-closed presentation contract for this task.
        #[must_use]
        pub const fn contract(self) -> UiTaskContract {
            use hex_core::Screen;
            match self {
                Self::Splash => task("startup-splash", Screen::Splash, &[], &[], false),
                Self::Loading => task("startup-loading", Screen::Loading, &[], &[], false),
                Self::TitleCold => task("title-cold", Screen::Title, TITLE_CONTROLS, &[], true),
                Self::TitleResume => task("title-resume", Screen::Title, TITLE_CONTROLS, &[], true),
                Self::TitleFailure => {
                    task("title-failure", Screen::Title, TITLE_CONTROLS, &[], true)
                }
                Self::MapScenarios => task(
                    "map-scenarios",
                    Screen::Scenarios,
                    &["Back"],
                    &["The Crossing", "Waterfall"],
                    true,
                ),
                Self::Demos => task(
                    "demos",
                    Screen::Scenarios,
                    &["Back"],
                    &["Ability Lab", "Raider Mirror"],
                    true,
                ),
                Self::Settings => task(
                    "settings",
                    Screen::Settings,
                    &["Back"],
                    &["Setting UiScale", "Setting UiVolume"],
                    true,
                ),
                Self::CharacterLibrary => task(
                    "creator-character-library",
                    Screen::CharacterCreator,
                    &["New Blank Character", "Open Spell Creator", "Title"],
                    &["Wolf Template"],
                    true,
                ),
                Self::SpellLibrary => task(
                    "creator-spell-library",
                    Screen::SpellCreator,
                    &["New Blank Spell", "Title"],
                    &["Training Spark"],
                    true,
                ),
                Self::CreatorLibraryRecovery => task(
                    "creator-library-recovery",
                    Screen::CharacterCreator,
                    &["Open Spell Creator", "Confirm Reset", "Title"],
                    &[],
                    true,
                ),
                Self::CharacterInvalid => task(
                    "creator-character-invalid",
                    Screen::CharacterCreator,
                    &["Library", "Save", "Open Spell Creator"],
                    &[
                        "Creator Palette More Tools Cue",
                        "Erase",
                        "Creator Inspector More Details Cue",
                    ],
                    true,
                ),
                Self::CharacterReady => task(
                    "creator-character-ready",
                    Screen::CharacterCreator,
                    &["Library", "Save", "Local Test", "Test on Map"],
                    &[
                        "Creator Palette More Tools Cue",
                        "Erase",
                        "Creator Inspector More Details Cue",
                    ],
                    true,
                ),
                Self::CharacterConfirmDelete => task(
                    "creator-character-confirm-delete",
                    Screen::CharacterCreator,
                    &["Library", "Save", "Open Spell Creator"],
                    &[
                        "Creator Palette More Tools Cue",
                        "Creator Inspector More Details Cue",
                        "Confirm Delete",
                    ],
                    true,
                ),
                Self::SpellInvalid => task(
                    "creator-spell-invalid",
                    Screen::SpellCreator,
                    &["Library", "Save"],
                    &["+ Reveal"],
                    true,
                ),
                Self::SpellReady => task(
                    "creator-spell-ready",
                    Screen::SpellCreator,
                    &["Library", "Save"],
                    &["Delete"],
                    true,
                ),
                Self::SpellConfirmDelete => task(
                    "creator-spell-confirm-delete",
                    Screen::SpellCreator,
                    &["Library", "Save"],
                    &["Confirm Delete"],
                    true,
                ),
                Self::LatticeDemo => task(
                    "lattice-demo",
                    Screen::LatticeDemo,
                    &["Back", "End Turn", "Reset", "Cast Lightning Bolt"],
                    &[],
                    true,
                ),
                Self::LabMap => task(
                    "lab-map",
                    Screen::CombatLab,
                    LAB_TABS,
                    &["Combat Lab Map Catalog Scroll Cue", "Flat Arena"],
                    true,
                ),
                Self::LabRosters => task(
                    "lab-rosters",
                    Screen::CombatLab,
                    &["Back to Map", "Continue to Rules"],
                    &["Add to roster"],
                    true,
                ),
                Self::LabRostersMax => task(
                    "lab-rosters-max",
                    Screen::CombatLab,
                    &["Back to Map", "Continue to Rules"],
                    &["Remove"],
                    true,
                ),
                Self::LabRules => task(
                    "lab-rules",
                    Screen::CombatLab,
                    &["Back to Rosters", "Load Map & Deploy"],
                    &["Reset to Shipped"],
                    true,
                ),
                Self::LabFixtures => task(
                    "lab-fixtures",
                    Screen::CombatLab,
                    LAB_TABS,
                    &["Run Custom three-step"],
                    true,
                ),
                Self::LabReportsEmpty => {
                    task("lab-reports-empty", Screen::CombatLab, LAB_TABS, &[], true)
                }
                Self::LabReportsPopulated => task(
                    "lab-reports-populated",
                    Screen::CombatLab,
                    LAB_TABS,
                    &["Delete…", "Confirm Delete"],
                    true,
                ),
                Self::DeploymentIncomplete => task(
                    "deployment-incomplete",
                    Screen::Gameplay,
                    &["Undo", "Deterministic Auto-place", "Back to Rules"],
                    &[],
                    true,
                ),
                Self::DeploymentComplete => task(
                    "deployment-complete",
                    Screen::Gameplay,
                    &["Start Combat", "Back to Rules"],
                    &[],
                    true,
                ),
                Self::Exploration => task(
                    "gameplay-exploration",
                    Screen::Gameplay,
                    &[
                        "Primary Action Rail",
                        "Action Rail Rest",
                        "Action Rail Pause",
                    ],
                    &[],
                    true,
                ),
                Self::PlayerTurnMaxActions => task(
                    "gameplay-player-turn-max",
                    Screen::Gameplay,
                    &[
                        "Primary Action Rail",
                        "Action Rail Channel",
                        "Action Rail End Turn",
                        "Action Rail Pause",
                    ],
                    &[],
                    true,
                ),
                Self::HostileTurn => task(
                    "gameplay-hostile-turn",
                    Screen::Gameplay,
                    &["Primary Action Rail"],
                    &[],
                    true,
                ),
                Self::Casting => task(
                    "casting",
                    Screen::Gameplay,
                    &["Primary Action Rail"],
                    &["Cast Lightning Bolt"],
                    true,
                ),
                Self::AimingBlocked => task(
                    "aiming-blocked",
                    Screen::Gameplay,
                    &[
                        "Primary Action Rail",
                        "Confirm Cast Disabled",
                        "Next Target Disabled",
                        "Cancel Aim",
                    ],
                    &[],
                    true,
                ),
                Self::DisableDecision => task(
                    "decision-disable",
                    Screen::Gameplay,
                    &["Primary Action Rail", "Action Rail Confirm 2 / 3"],
                    &[],
                    true,
                ),
                Self::RestoreDecision => task(
                    "decision-restore",
                    Screen::Gameplay,
                    &["Primary Action Rail", "Action Rail Confirm 2 / 3"],
                    &[],
                    true,
                ),
                Self::HudHiddenRequired => task(
                    "hud-hidden-required",
                    Screen::Gameplay,
                    &["Primary Action Rail", "Action Rail Confirm 2 / 3"],
                    &[],
                    true,
                ),
                Self::LabStatistics => task(
                    "lab-statistics",
                    Screen::Gameplay,
                    &["Primary Action Rail"],
                    &[
                        "Expand or collapse live Combat Lab statistics",
                        "Combat Lab Statistics Scroll Cue",
                        "End experiment and save the current Combat Lab report",
                        "Combat Lab Statistics Detail End",
                    ],
                    true,
                ),
                Self::Pause => task("pause", Screen::Gameplay, &["Resume"], &[], true),
                Self::OrdinaryOutcome => task(
                    "outcome-ordinary",
                    Screen::Gameplay,
                    &["Continue", "Retry"],
                    &[],
                    true,
                ),
                Self::LabReportOverview => report_task("report-overview", true),
                Self::LabReportUnits => report_task("report-units", true),
                Self::LabReportSpellsEffects => report_task("report-spells-effects", true),
                Self::LabReportTimeline => report_task("report-timeline", true),
                Self::LabReportCompare => report_task("report-compare", true),
            }
        }

        /// Fail-closed lattice surface required by this task's authored state.
        #[must_use]
        pub const fn lattice_requirement(self) -> UiTaskLatticeRequirement {
            match self {
                Self::Exploration
                | Self::PlayerTurnMaxActions
                | Self::HostileTurn
                | Self::AimingBlocked
                | Self::Pause => UiTaskLatticeRequirement::Own,
                Self::Casting | Self::LabStatistics => UiTaskLatticeRequirement::OwnAndTarget,
                Self::DisableDecision | Self::RestoreDecision | Self::HudHiddenRequired => {
                    UiTaskLatticeRequirement::RequiredChoice
                }
                _ => UiTaskLatticeRequirement::None,
            }
        }

        /// Named controls whose relative keyboard order is part of this task's
        /// contract. Controls not listed still must be keyboard reachable; this
        /// sequence records only ordering relationships that are intentional and
        /// stable across responsive reflow.
        #[must_use]
        pub const fn focus_sequence(self) -> &'static [&'static str] {
            match self {
                Self::TitleCold | Self::TitleResume | Self::TitleFailure => TITLE_CONTROLS,
                Self::MapScenarios => &["The Crossing", "Waterfall", "Back"],
                Self::Demos => &["Ability Lab", "Raider Mirror", "Back"],
                Self::LabMap
                | Self::LabFixtures
                | Self::LabReportsEmpty
                | Self::LabReportsPopulated => LAB_TABS,
                Self::LabReportOverview
                | Self::LabReportUnits
                | Self::LabReportSpellsEffects
                | Self::LabReportTimeline
                | Self::LabReportCompare => &[
                    "Overview",
                    "Units",
                    "Spells & Effects",
                    "Timeline",
                    "Compare",
                ],
                _ => &[],
            }
        }
    }

    const TITLE_CONTROLS: &[&str] = &[
        "Continue",
        "New Game",
        "Character Creator",
        "Spell Creator",
        "Combat Lab",
        "Map Scenarios",
        "Demos",
        "Settings",
        "Quit",
    ];
    const LAB_TABS: &[&str] = &["Sandbox", "Fixed Fixtures", "Saved Reports", "Back"];

    const fn task(
        id: &'static str,
        screen: hex_core::Screen,
        immediate_controls: &'static [&'static str],
        scrollable_controls: &'static [&'static str],
        exhaustive_layout: bool,
    ) -> UiTaskContract {
        UiTaskContract {
            id,
            screen,
            immediate_controls,
            scrollable_controls,
            exhaustive_layout,
        }
    }

    const fn report_task(id: &'static str, exhaustive_layout: bool) -> UiTaskContract {
        task(
            id,
            hex_core::Screen::Gameplay,
            &[
                "Overview",
                "Units",
                "Spells & Effects",
                "Timeline",
                "Compare",
            ],
            &["Outcome Report Body Scroll"],
            exhaustive_layout,
        )
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
                bevy::asset::AssetPlugin {
                    watch_for_changes_override: Some(false),
                    ..default()
                },
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
        /// Whether the entity supplied an authored stable [`Name`].
        pub has_stable_name: bool,
        /// Stable identity of the immediate presentation parent, when named.
        pub parent_name: Option<String>,
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
        pub visibility_requirement: Option<crate::UiVisibilityRequirement>,
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

    impl UiNodeObservation {
        fn layout_bounds(&self) -> Option<Rect> {
            (self.size.x > 0.5 && self.size.y > 0.5)
                .then(|| Rect::from_center_size(self.center, self.size))
        }
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
                    && node.visibility_requirement
                        == Some(crate::UiVisibilityRequirement::Immediate)
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
                        "{} has presentation content outside its box; content {:.1}×{:.1} versus box {:.1}×{:.1}",
                        node.name,
                        node.content_size.x,
                        node.content_size.y,
                        node.size.x,
                        node.size.y,
                    ));
                }
            }
            for node in self.nodes.iter().filter(|node| node.focusable) {
                if !node.has_stable_name {
                    issues.push(format!(
                        "{} is interactive but has no authored stable Name",
                        node.name
                    ));
                }
                let Some(requirement) = node.visibility_requirement else {
                    issues.push(format!(
                        "{} is interactive but has no explicit immediate/scrollable visibility contract",
                        node.name
                    ));
                    continue;
                };
                if node.size.x <= 0.5 || node.size.y <= 0.5 {
                    issues.push(format!("{} has zero layout area", node.name));
                } else if requirement == crate::UiVisibilityRequirement::Immediate
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
            let checked_controls = self
                .nodes
                .iter()
                .filter(|node| {
                    node.visible
                        && (node.in_focus_order
                            || (node.visibility_requirement
                                == Some(crate::UiVisibilityRequirement::Immediate)
                                && node.accessible_label.is_some()))
                })
                .collect::<Vec<_>>();
            for (index, left) in checked_controls.iter().enumerate() {
                for right in checked_controls.iter().skip(index + 1) {
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

        /// Returns fail-closed geometry and named-control failures for one task.
        ///
        /// Visual-walk captures use this same contract as the exhaustive
        /// headless matrix, so reaching the correct screen is insufficient when
        /// the authored task surface failed to render.
        #[must_use]
        pub fn task_issues(&self, case: UiTaskCase) -> Vec<String> {
            let contract = case.contract();
            let mut issues = match case {
                UiTaskCase::Casting => self.review_fixture_issues("casting-list"),
                UiTaskCase::LabStatistics => self.review_fixture_issues("live-statistics"),
                _ => self.layout_issues(),
            };
            for name in contract.immediate_controls {
                let Some(node) = self.nodes.iter().find(|node| node.name == *name) else {
                    issues.push(format!("missing immediate control {name:?}"));
                    continue;
                };
                if node.visibility_requirement != Some(crate::UiVisibilityRequirement::Immediate) {
                    issues.push(format!("control {name:?} is not explicitly Immediate"));
                }
                if !node.fully_visible {
                    issues.push(format!(
                        "control {name:?} is not initially visible: {node:?}"
                    ));
                }
            }
            for name in contract.scrollable_controls {
                let Some(node) = self.nodes.iter().find(|node| node.name == *name) else {
                    issues.push(format!("missing scrollable control {name:?}"));
                    continue;
                };
                if node.visibility_requirement != Some(crate::UiVisibilityRequirement::Scrollable) {
                    issues.push(format!(
                        "control {name:?} did not explicitly opt into Scrollable"
                    ));
                }
                if !node.scroll_reachable {
                    issues.push(format!(
                        "control {name:?} has no complete scroll route: {node:?}"
                    ));
                }
            }
            let mut previous_focus = None;
            for name in case.focus_sequence() {
                let Some(node) = self.nodes.iter().find(|node| node.name == *name) else {
                    continue;
                };
                if !node.focusable {
                    continue;
                }
                let Some(position) = self.focus_order.iter().position(|focused| focused == name)
                else {
                    continue;
                };
                if previous_focus.is_some_and(|previous| position <= previous) {
                    issues.push(format!(
                        "control {name:?} appears out of declared focus order: {:?}",
                        self.focus_order
                    ));
                }
                previous_focus = Some(position);
            }
            issues.extend(self.task_lattice_issues(case));
            if case == UiTaskCase::HudHiddenRequired {
                for hidden in [
                    "Party Strip",
                    "Initiative Panel",
                    "Combat Log Panel",
                    "Combat Lab Live Statistics Drawer",
                ] {
                    if self.nodes.iter().any(|node| node.name == hidden) {
                        issues.push(format!(
                            "ordinary HUD surface {hidden:?} remained visible while the HUD was hidden"
                        ));
                    }
                }
            }
            issues
        }

        fn task_lattice_issues(&self, case: UiTaskCase) -> Vec<String> {
            let mut issues = Vec::new();
            match case.lattice_requirement() {
                UiTaskLatticeRequirement::None => {}
                UiTaskLatticeRequirement::Own => {
                    self.require_lattice_branch(case, "own", &mut issues);
                }
                UiTaskLatticeRequirement::OwnAndTarget => {
                    self.require_lattice_branch(case, "own", &mut issues);
                    self.require_lattice_branch(case, "target", &mut issues);
                }
                UiTaskLatticeRequirement::RequiredChoice => {
                    if self
                        .nodes
                        .iter()
                        .any(|node| node.name == "Compact Required Lattice Choice")
                    {
                        for name in [
                            "Compact Required Lattice Choice",
                            "Compact Required Lattice",
                            "Compact Required Cell (0, 0)",
                        ] {
                            let Some(node) = self.nodes.iter().find(|node| node.name == name)
                            else {
                                issues.push(format!(
                                    "{} is missing required promoted lattice surface {name:?}",
                                    case.contract().id
                                ));
                                continue;
                            };
                            if node.size.x <= 0.5 || node.size.y <= 0.5 {
                                issues.push(format!(
                                    "{} required promoted lattice surface {name:?} has no layout area",
                                    case.contract().id
                                ));
                            } else if !node.fully_visible {
                                issues.push(format!(
                                    "{} required promoted lattice surface {name:?} is not initially visible: {node:?}",
                                    case.contract().id
                                ));
                            }
                        }
                    } else {
                        self.require_lattice_branch(case, "own", &mut issues);
                    }
                }
            }
            issues
        }

        fn require_lattice_branch(&self, case: UiTaskCase, branch: &str, issues: &mut Vec<String>) {
            let (panel, lattice, cell, body) = match branch {
                "own" => (
                    "Own Lattice Panel",
                    "Own Lattice",
                    "Own Cell (0, 0)",
                    "Own Lattice Body",
                ),
                "target" => (
                    "Target Lattice Panel",
                    "Target Lattice",
                    "Target Cell (0, 0)",
                    "Target Lattice Body",
                ),
                other => unreachable!("unknown lattice presentation branch {other:?}"),
            };
            for (name, expected_parent) in [
                (panel, "Lattice Readout Stack"),
                (lattice, body),
                (cell, lattice),
            ] {
                let Some(node) = self.nodes.iter().find(|node| node.name == name) else {
                    issues.push(format!(
                        "{} is missing required {branch} lattice surface {name:?}",
                        case.contract().id
                    ));
                    continue;
                };
                if node.size.x <= 0.5 || node.size.y <= 0.5 {
                    issues.push(format!(
                        "{} required {branch} lattice surface {name:?} has no layout area",
                        case.contract().id
                    ));
                }
                if !node.scroll_reachable {
                    issues.push(format!(
                        "{} required {branch} lattice surface {name:?} has no complete scroll route: {node:?}",
                        case.contract().id
                    ));
                }
                if node.parent_name.as_deref() != Some(expected_parent) {
                    issues.push(format!(
                        "{} required {branch} lattice surface {name:?} must be a child of {expected_parent:?}, not {:?}",
                        case.contract().id,
                        node.parent_name
                    ));
                }
            }
        }

        /// Returns presentation-contract failures for an authored review fixture.
        ///
        /// This supplements generic geometry checks with named composition facts;
        /// it never infers gameplay state from pixels or rendered copy.
        #[must_use]
        pub fn review_fixture_issues(&self, fixture: &str) -> Vec<String> {
            let mut issues = self.layout_issues();
            let drawers = self
                .nodes
                .iter()
                .filter(|node| node.name == "Combat Lab Live Statistics Drawer")
                .collect::<Vec<_>>();
            if !drawers.is_empty() {
                let lattices = self
                    .nodes
                    .iter()
                    .filter(|node| node.name == "Lattice Readout Stack")
                    .collect::<Vec<_>>();
                let own_panels = self
                    .nodes
                    .iter()
                    .filter(|node| node.name == "Own Lattice Panel")
                    .collect::<Vec<_>>();
                if drawers.len() != 1 || lattices.len() != 1 || own_panels.len() != 1 {
                    issues.push(format!(
                        "presented statistics require exactly one lattice stack and own panel: drawers={}, stacks={}, own_panels={}",
                        drawers.len(),
                        lattices.len(),
                        own_panels.len()
                    ));
                } else if let (Some(drawer), Some(lattice)) = (drawers.first(), lattices.first()) {
                    if drawer.parent_name.as_deref() != Some("Inspector HUD Region")
                        || lattice.parent_name.as_deref() != Some("Inspector HUD Region")
                    {
                        issues.push(format!(
                            "statistics and lattice must share the Inspector scroll owner: lattice_parent={:?}, drawer_parent={:?}",
                            lattice.parent_name, drawer.parent_name
                        ));
                    }
                    if let (Some(lattice_bounds), Some(drawer_bounds)) =
                        (lattice.layout_bounds(), drawer.layout_bounds())
                    {
                        if lattice_bounds.max.y > drawer_bounds.min.y + 0.5 {
                            issues.push(format!(
                                "presented statistics must follow the lattice: lattice={lattice_bounds:?}, drawer={drawer_bounds:?}"
                            ));
                        }
                    }
                }
            }
            if !matches!(fixture, "casting-list" | "live-statistics") {
                return issues;
            }

            let required_surface = |name: &str,
                                    requirement: Option<crate::UiVisibilityRequirement>,
                                    issues: &mut Vec<String>| {
                let Some(node) = self.nodes.iter().find(|node| node.name == name) else {
                    issues.push(format!("{fixture} is missing required surface {name:?}"));
                    return None;
                };
                if node.size.x <= 0.5 || node.size.y <= 0.5 {
                    issues.push(format!(
                        "{fixture} required surface {name:?} has no layout area: {node:?}"
                    ));
                } else if requirement == Some(crate::UiVisibilityRequirement::Immediate)
                    && !node.fully_visible
                {
                    issues.push(format!(
                        "{fixture} required surface {name:?} is not fully visible: {node:?}"
                    ));
                } else if requirement == Some(crate::UiVisibilityRequirement::Scrollable)
                    && !node.scroll_reachable
                {
                    issues.push(format!(
                        "{fixture} required surface {name:?} has no complete scroll route: {node:?}"
                    ));
                }
                Some(node)
            };

            let drawer = required_surface("Combat Lab Live Statistics Drawer", None, &mut issues);
            let toggle = required_surface(
                "Expand or collapse live Combat Lab statistics",
                Some(crate::UiVisibilityRequirement::Scrollable),
                &mut issues,
            );
            let end_control = required_surface(
                "End experiment and save the current Combat Lab report",
                Some(crate::UiVisibilityRequirement::Scrollable),
                &mut issues,
            );
            if let Some(drawer_bounds) = drawer.and_then(UiNodeObservation::layout_bounds) {
                for control in [toggle, end_control].into_iter().flatten() {
                    if let Some(control_bounds) = control.layout_bounds() {
                        if control_bounds.min.x < drawer_bounds.min.x - 0.5
                            || control_bounds.max.x > drawer_bounds.max.x + 0.5
                            || control_bounds.min.y < drawer_bounds.min.y - 0.5
                            || control_bounds.max.y > drawer_bounds.max.y + 0.5
                        {
                            issues.push(format!(
                                "statistics control {:?} escapes its drawer: drawer={drawer_bounds:?}, control={control_bounds:?}",
                                control.name
                            ));
                        }
                    }
                }
            }
            if fixture == "live-statistics" {
                let casting = self.nodes.iter().find(|node| node.name == "Casting Panel");
                if casting.is_none() {
                    issues.push(
                        "live-statistics is missing the populated primary casting surface"
                            .to_owned(),
                    );
                }
                if let (Some(drawer_bounds), Some(casting_bounds)) = (
                    drawer.and_then(|node| node.visible_bounds),
                    casting.and_then(|node| node.visible_bounds),
                ) {
                    let overlap = drawer_bounds.intersect(casting_bounds);
                    if overlap.width() > 0.5 && overlap.height() > 0.5 {
                        issues.push(format!(
                            "expanded statistics cover the primary casting surface by {:.1}×{:.1}: drawer={drawer_bounds:?}, casting={casting_bounds:?}",
                            overlap.width(),
                            overlap.height()
                        ));
                    }
                }
                if let Some(drawer_bounds) = drawer.and_then(|node| node.visible_bounds) {
                    for control in self.nodes.iter().filter(|node| {
                        node.focusable
                            && !matches!(
                                node.name.as_str(),
                                "Expand or collapse live Combat Lab statistics"
                                    | "End experiment and save the current Combat Lab report"
                            )
                    }) {
                        let Some(control_bounds) = control.visible_bounds else {
                            continue;
                        };
                        let overlap = drawer_bounds.intersect(control_bounds);
                        if overlap.width() > 0.5 && overlap.height() > 0.5 {
                            issues.push(format!(
                                "expanded statistics cover focusable control {:?} by {:.1}×{:.1}",
                                control.name,
                                overlap.width(),
                                overlap.height()
                            ));
                        }
                    }
                }
                if let Some(body) =
                    required_surface("Combat Lab Statistics Body", None, &mut issues)
                {
                    if body.visibility_requirement
                        != Some(crate::UiVisibilityRequirement::Scrollable)
                    {
                        issues.push(
                            "the expanded statistics body must explicitly be Scrollable".to_owned(),
                        );
                    }
                }
                if let Some(end) = self
                    .nodes
                    .iter()
                    .find(|node| node.name == "Combat Lab Statistics Detail End")
                {
                    if end.visibility_requirement
                        != Some(crate::UiVisibilityRequirement::Scrollable)
                        || !end.scroll_reachable
                    {
                        issues.push(format!(
                            "the complete statistics detail has no scroll route: {end:?}"
                        ));
                    }
                } else {
                    issues.push(
                        "live-statistics is missing its scrollable detail end marker".to_owned(),
                    );
                }
            } else if self
                .nodes
                .iter()
                .any(|node| node.name == "Combat Lab Statistics Body")
            {
                issues
                    .push("collapsed statistics unexpectedly present their detail body".to_owned());
            }

            let inspector = required_surface(
                "Inspector HUD Region",
                Some(crate::UiVisibilityRequirement::Immediate),
                &mut issues,
            );
            let lattice = required_surface("Lattice Readout Stack", None, &mut issues);
            let own_panel = required_surface(
                "Own Lattice Panel",
                Some(crate::UiVisibilityRequirement::Scrollable),
                &mut issues,
            );
            let target_panel = required_surface(
                "Target Lattice Panel",
                Some(crate::UiVisibilityRequirement::Scrollable),
                &mut issues,
            );
            let own_lattice = required_surface(
                "Own Lattice",
                Some(crate::UiVisibilityRequirement::Scrollable),
                &mut issues,
            );
            let target_lattice = required_surface(
                "Target Lattice",
                Some(crate::UiVisibilityRequirement::Scrollable),
                &mut issues,
            );
            let own_cell = required_surface(
                "Own Cell (0, 0)",
                Some(crate::UiVisibilityRequirement::Scrollable),
                &mut issues,
            );
            let target_cell = required_surface(
                "Target Cell (0, 0)",
                Some(crate::UiVisibilityRequirement::Scrollable),
                &mut issues,
            );
            let own_extreme = required_surface(
                "Own Cell (2, -2)",
                Some(crate::UiVisibilityRequirement::Scrollable),
                &mut issues,
            );
            let target_extreme = required_surface(
                "Target Cell (2, 0)",
                Some(crate::UiVisibilityRequirement::Scrollable),
                &mut issues,
            );

            for (node, expected_parent) in [
                (inspector, "Gameplay HUD Safe Frame"),
                (lattice, "Inspector HUD Region"),
                (own_panel, "Lattice Readout Stack"),
                (target_panel, "Lattice Readout Stack"),
                (drawer, "Inspector HUD Region"),
                (own_lattice, "Own Lattice Body"),
                (target_lattice, "Target Lattice Body"),
                (own_cell, "Own Lattice"),
                (target_cell, "Target Lattice"),
                (own_extreme, "Own Lattice"),
                (target_extreme, "Target Lattice"),
            ] {
                if let Some(node) = node {
                    if node.parent_name.as_deref() != Some(expected_parent) {
                        issues.push(format!(
                            "{:?} must be a child of {expected_parent:?}, not {:?}",
                            node.name, node.parent_name
                        ));
                    }
                }
            }

            for (prefix, parent) in [
                ("Own Cell (", own_lattice),
                ("Target Cell (", target_lattice),
            ] {
                let Some(parent_bounds) = parent.and_then(UiNodeObservation::layout_bounds) else {
                    continue;
                };
                for cell in self
                    .nodes
                    .iter()
                    .filter(|node| node.name.starts_with(prefix))
                {
                    if cell.parent_name.as_deref() != parent.map(|node| node.name.as_str()) {
                        issues.push(format!(
                            "{:?} is detached from its lattice parent {:?}",
                            cell.name,
                            parent.map(|node| node.name.as_str())
                        ));
                    }
                    let Some(cell_bounds) = cell.layout_bounds() else {
                        issues.push(format!("{:?} has no lattice-cell layout area", cell.name));
                        continue;
                    };
                    if cell_bounds.min.x < parent_bounds.min.x - 0.5
                        || cell_bounds.max.x > parent_bounds.max.x + 0.5
                        || cell_bounds.min.y < parent_bounds.min.y - 0.5
                        || cell_bounds.max.y > parent_bounds.max.y + 0.5
                    {
                        issues.push(format!(
                            "{:?} escapes its lattice geometry: lattice={parent_bounds:?}, cell={cell_bounds:?}",
                            cell.name
                        ));
                    }
                    if !cell.scroll_reachable {
                        issues.push(format!(
                            "{:?} has no complete Inspector scroll route",
                            cell.name
                        ));
                    }
                }
            }

            if let (Some(lattice_bounds), Some(drawer_bounds)) = (
                lattice.and_then(UiNodeObservation::layout_bounds),
                drawer.and_then(UiNodeObservation::layout_bounds),
            ) {
                if lattice_bounds.max.y > drawer_bounds.min.y + 0.5 {
                    issues.push(format!(
                        "statistics must follow the lattice: lattice={lattice_bounds:?}, drawer={drawer_bounds:?}"
                    ));
                }
            }
            if let Some(inspector_bounds) = inspector.and_then(UiNodeObservation::layout_bounds) {
                for surface in [lattice, drawer].into_iter().flatten() {
                    if let Some(bounds) = surface.layout_bounds() {
                        if bounds.min.x < inspector_bounds.min.x - 0.5
                            || bounds.max.x > inspector_bounds.max.x + 0.5
                        {
                            issues.push(format!(
                                "{:?} escapes the Inspector lane horizontally: inspector={inspector_bounds:?}, surface={bounds:?}",
                                surface.name
                            ));
                        }
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
        let descendant_text_bounds = descendant_text_bounds_by_ancestor(world);
        let entities = {
            let mut query = world.query::<Entity>();
            query.iter(world).collect::<Vec<_>>()
        };
        let mut nodes = entities
            .into_iter()
            .filter(|entity| is_presented(world, *entity))
            .filter_map(|entity| {
                let stable_name = world.get::<Name>(entity);
                let focusable = world.get::<Button>(entity).is_some()
                    || world
                        .get::<TabIndex>(entity)
                        .is_some_and(|index| index.0 >= 0);
                if stable_name.is_none() && !focusable {
                    return None;
                }
                let name = stable_name.map_or_else(
                    || format!("<unnamed UI entity {:?}>", entity),
                    |name| name.as_str().to_owned(),
                );
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
                let descendant_text_overflow = world.get::<crate::UiTextMustFit>(entity).is_some()
                    && descendant_text_bounds
                        .get(&entity)
                        .is_some_and(|text_bounds| descendant_text_overflows(bounds, *text_bounds));
                let visible_bounds =
                    effective_visible_bounds(world, entity, presented_bounds, metrics);
                let fully_visible = rect_contains(
                    Rect::from_corners(Vec2::ZERO, metrics.logical_size),
                    presented_bounds,
                ) && world.get::<CalculatedClip>(entity).is_none_or(|clip| {
                    rect_contains(scale_rect(clip.clip, inverse_scale), presented_bounds)
                });
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
                    name,
                    has_stable_name: stable_name.is_some(),
                    parent_name: world
                        .get::<ChildOf>(entity)
                        .and_then(|parent| world.get::<Name>(parent.parent()))
                        .map(|name| name.as_str().to_owned()),
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
                        .or_else(|| {
                            world
                                .get::<crate::DefaultImmediateControl>(entity)
                                .map(|_| crate::UiVisibilityRequirement::Immediate)
                        }),
                    accessible_label: world
                        .get::<AccessibleLabel>(entity)
                        .map(|label| label.0.clone()),
                    tab_index: world.get::<TabIndex>(entity).map(|index| index.0),
                    overflows: descendant_text_overflow
                        || computed.is_some_and(|node| {
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

    fn descendant_text_bounds_by_ancestor(world: &mut World) -> HashMap<Entity, Rect> {
        let text_entities = {
            let mut query = world.query_filtered::<Entity, With<bevy::text::TextLayoutInfo>>();
            query.iter(world).collect::<Vec<_>>()
        };
        let mut bounds_by_ancestor = HashMap::<Entity, Rect>::new();
        for text_entity in text_entities {
            let Some(bounds) = rendered_text_bounds(world, text_entity) else {
                continue;
            };
            let mut current = Some(text_entity);
            while let Some(entity) = current {
                bounds_by_ancestor
                    .entry(entity)
                    .and_modify(|existing| {
                        *existing = Rect::from_corners(
                            existing.min.min(bounds.min),
                            existing.max.max(bounds.max),
                        );
                    })
                    .or_insert(bounds);
                current = world.get::<ChildOf>(entity).map(ChildOf::parent);
            }
        }
        bounds_by_ancestor
    }

    fn scale_rect(rect: Rect, scale: f32) -> Rect {
        Rect::from_corners(rect.min * scale, rect.max * scale)
    }

    fn rect_contains(outer: Rect, inner: Rect) -> bool {
        rect_contains_with_epsilon(outer, inner, 0.5)
    }

    fn rect_contains_with_epsilon(outer: Rect, inner: Rect, epsilon: f32) -> bool {
        inner.min.x >= outer.min.x - epsilon
            && inner.min.y >= outer.min.y - epsilon
            && inner.max.x <= outer.max.x + epsilon
            && inner.max.y <= outer.max.y + epsilon
    }

    fn descendant_text_overflows(container: Rect, text: Rect) -> bool {
        // Atlas ascent/descent can legitimately overhang a control's Yoga text
        // box vertically without painting into a neighboring action. Horizontal
        // escape is the collision-prone failure (for example two long disabled
        // status labels running together), so keep that tolerance tight while
        // retaining the legacy content-box allowance vertically.
        const HORIZONTAL_EPSILON: f32 = 2.0;
        const VERTICAL_EPSILON: f32 = 10.0;
        text.min.x < container.min.x - HORIZONTAL_EPSILON
            || text.max.x > container.max.x + HORIZONTAL_EPSILON
            || text.min.y < container.min.y - VERTICAL_EPSILON
            || text.max.y > container.max.y + VERTICAL_EPSILON
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
            if let (Some(node), Some(computed), Some(parent_bounds)) = (
                world.get::<Node>(parent),
                world.get::<ComputedNode>(parent),
                node_bounds(world, parent),
            ) {
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
                            if (world.get::<ScrollArea>(parent).is_some()
                                || world
                                    .get::<crate::creator::CompactCreatorCanvasScroll>(parent)
                                    .is_some())
                                && world.get::<ScrollPosition>(parent).is_some()
                                && target_length <= parent_length + 0.5 =>
                        {
                            let Some(scroll) = world.get::<ScrollPosition>(parent) else {
                                return false;
                            };
                            let current_scroll = if horizontal { scroll.x } else { scroll.y };
                            let visible_size = computed.size() * computed.inverse_scale_factor;
                            let content_size =
                                computed.content_size() * computed.inverse_scale_factor;
                            let max_scroll = if horizontal {
                                (content_size.x - visible_size.x).max(0.0)
                            } else {
                                (content_size.y - visible_size.y).max(0.0)
                            };
                            // Increasing ScrollPosition moves content toward the
                            // viewport origin. From the current layout, legal
                            // target shifts span [s - max, s]. That range must
                            // intersect the shifts that fully contain the target.
                            let legal_shift_min = current_scroll - max_scroll;
                            let legal_shift_max = current_scroll;
                            let required_shift_min = parent_min - candidate_min;
                            let required_shift_max = parent_max - candidate_max;
                            if legal_shift_min.max(required_shift_min)
                                > legal_shift_max.min(required_shift_max) + 0.5
                            {
                                return false;
                            }
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

        fn task_contract_issues(case: UiTaskCase, snapshot: &UiTreeSnapshot) -> Vec<String> {
            snapshot.task_issues(case)
        }

        fn assert_task_contract(case: UiTaskCase, snapshot: &UiTreeSnapshot) {
            let issues = task_contract_issues(case, snapshot);
            assert!(
                issues.is_empty(),
                "{} task contract failed: {issues:#?}",
                case.contract().id
            );
        }

        #[test]
        fn task_registry_is_unique_and_exhaustively_classifies_every_screen() {
            let ids = UiTaskCase::ALL
                .into_iter()
                .map(|case| case.contract().id)
                .collect::<HashSet<_>>();
            assert_eq!(ids.len(), UiTaskCase::ALL.len());

            let expected_case_count = |screen| match screen {
                hex_core::Screen::Splash => 1,
                hex_core::Screen::Title => 3,
                hex_core::Screen::Scenarios => 2,
                hex_core::Screen::Settings => 1,
                hex_core::Screen::LatticeDemo => 1,
                hex_core::Screen::CharacterCreator => 5,
                hex_core::Screen::SpellCreator => 4,
                hex_core::Screen::CombatLab => 7,
                hex_core::Screen::Loading => 1,
                hex_core::Screen::Gameplay => 18,
            };
            for screen in [
                hex_core::Screen::Splash,
                hex_core::Screen::Title,
                hex_core::Screen::Scenarios,
                hex_core::Screen::Settings,
                hex_core::Screen::LatticeDemo,
                hex_core::Screen::CharacterCreator,
                hex_core::Screen::SpellCreator,
                hex_core::Screen::CombatLab,
                hex_core::Screen::Loading,
                hex_core::Screen::Gameplay,
            ] {
                assert_eq!(
                    UiTaskCase::ALL
                        .into_iter()
                        .filter(|case| case.contract().screen == screen)
                        .count(),
                    expected_case_count(screen),
                    "{screen:?} task coverage drifted"
                );
            }
        }

        #[test]
        fn every_registered_task_constructs_a_populated_presentation_fixture() {
            for case in UiTaskCase::ALL {
                let snapshot = task_snapshot_at(case, 1280, 720, 1.0, crate::UiScaleMode::Auto);
                assert!(
                    !snapshot.nodes.is_empty(),
                    "{} produced no observable presentation tree",
                    case.contract().id
                );
            }
        }

        #[test]
        fn every_task_passes_its_declared_viewport_and_scale_matrix() {
            let viewports = [
                (UVec2::new(1280, 720), 1.0, crate::UiScaleMode::Auto),
                (UVec2::new(1920, 1080), 1.0, crate::UiScaleMode::Auto),
                (UVec2::new(3840, 2160), 1.0, crate::UiScaleMode::Auto),
                (UVec2::new(1280, 720), 1.0, crate::UiScaleMode::Percent200),
                (UVec2::new(3024, 1898), 2.0, crate::UiScaleMode::Auto),
            ];
            let mut failures = Vec::new();
            let smoke_only_contracts = UiTaskCase::ALL
                .into_iter()
                .filter(|case| !case.contract().exhaustive_layout)
                .map(|case| case.contract().id)
                .collect::<Vec<_>>();
            assert_eq!(
                smoke_only_contracts,
                ["startup-splash", "startup-loading"],
                "every populated interactive task must retain the complete viewport/scale matrix"
            );
            for case in UiTaskCase::ALL {
                if case.contract().exhaustive_layout {
                    for (physical, device_scale) in structural_canvases() {
                        for mode in all_scale_modes() {
                            let snapshot =
                                task_snapshot_at(case, physical.x, physical.y, device_scale, mode);
                            let issues = task_contract_issues(case, &snapshot);
                            if !issues.is_empty() {
                                failures.push((
                                    case.contract().id,
                                    physical,
                                    device_scale,
                                    mode,
                                    issues,
                                ));
                            }
                        }
                    }
                } else {
                    for (physical, device_scale, mode) in viewports {
                        let snapshot =
                            task_snapshot_at(case, physical.x, physical.y, device_scale, mode);
                        let issues = task_contract_issues(case, &snapshot);
                        if !issues.is_empty() {
                            failures.push((
                                case.contract().id,
                                physical,
                                device_scale,
                                mode,
                                issues,
                            ));
                        }
                    }
                }
            }
            assert!(
                failures.is_empty(),
                "declared UI task matrix failures: {failures:#?}"
            );
        }

        #[test]
        fn lattice_demo_back_is_an_immediate_focusable_route() {
            let snapshot = lattice_demo_snapshot(1280, 720, 1.0, crate::UiScaleMode::Auto);
            let back = snapshot
                .nodes
                .iter()
                .find(|node| node.name == "Back")
                .expect("the Lattice Demo must expose a Back control");
            assert!(
                back.fully_visible,
                "Back must be visible without scrolling: {back:?}"
            );
            assert_eq!(
                back.visibility_requirement,
                Some(crate::UiVisibilityRequirement::Immediate)
            );
            assert!(
                snapshot.focus_order.iter().any(|name| name == "Back"),
                "Back must be reachable by keyboard focus: {:?}",
                snapshot.focus_order
            );
            assert!(
                back.accessible_label.is_some(),
                "Back must carry an accessibility label"
            );
        }

        #[test]
        fn startup_title_catalog_and_settings_tasks_satisfy_their_named_contracts() {
            for (case, snapshot) in [
                (
                    UiTaskCase::Splash,
                    simple_screen_snapshot(
                        hex_core::Screen::Splash,
                        1280,
                        720,
                        1.0,
                        crate::UiScaleMode::Auto,
                    ),
                ),
                (
                    UiTaskCase::Loading,
                    simple_screen_snapshot(
                        hex_core::Screen::Loading,
                        1280,
                        720,
                        1.0,
                        crate::UiScaleMode::Auto,
                    ),
                ),
                (
                    UiTaskCase::TitleCold,
                    title_case_snapshot(
                        UiTaskCase::TitleCold,
                        1280,
                        720,
                        1.0,
                        crate::UiScaleMode::Auto,
                    ),
                ),
                (
                    UiTaskCase::TitleResume,
                    title_case_snapshot(
                        UiTaskCase::TitleResume,
                        1280,
                        720,
                        1.0,
                        crate::UiScaleMode::Auto,
                    ),
                ),
                (
                    UiTaskCase::TitleFailure,
                    title_case_snapshot(
                        UiTaskCase::TitleFailure,
                        1280,
                        720,
                        1.0,
                        crate::UiScaleMode::Auto,
                    ),
                ),
                (
                    UiTaskCase::MapScenarios,
                    production_scenario_snapshot(
                        1280,
                        720,
                        1.0,
                        crate::UiScaleMode::Auto,
                        crate::ScenarioBrowserKind::MapScenarios,
                    ),
                ),
                (
                    UiTaskCase::Demos,
                    production_scenario_snapshot(
                        1280,
                        720,
                        1.0,
                        crate::UiScaleMode::Auto,
                        crate::ScenarioBrowserKind::Demos,
                    ),
                ),
                (
                    UiTaskCase::Settings,
                    setup_screen_snapshot(
                        1280,
                        720,
                        1.0,
                        crate::UiScaleMode::Auto,
                        hex_core::Screen::Settings,
                    ),
                ),
            ] {
                assert_task_contract(case, &snapshot);
            }
        }

        #[test]
        fn every_creator_library_and_workspace_has_a_populated_task_contract() {
            let mut failures = Vec::new();
            for case in [
                UiTaskCase::CharacterLibrary,
                UiTaskCase::SpellLibrary,
                UiTaskCase::CreatorLibraryRecovery,
                UiTaskCase::CharacterInvalid,
                UiTaskCase::CharacterReady,
                UiTaskCase::CharacterConfirmDelete,
                UiTaskCase::SpellInvalid,
                UiTaskCase::SpellReady,
                UiTaskCase::SpellConfirmDelete,
            ] {
                let snapshot =
                    creator_case_snapshot(case, 1280, 720, 1.0, crate::UiScaleMode::Auto);
                let issues = task_contract_issues(case, &snapshot);
                if !issues.is_empty() {
                    failures.push((case.contract().id, issues));
                }
            }
            assert!(failures.is_empty(), "Creator task failures: {failures:#?}");
        }

        #[test]
        fn character_workspace_announces_more_sidebar_content() {
            let cues = [
                (
                    "Creator Palette More Tools Cue",
                    "More character creation tools are available below",
                ),
                (
                    "Creator Inspector More Details Cue",
                    "More character build details are available below",
                ),
            ];
            for case in [
                UiTaskCase::CharacterInvalid,
                UiTaskCase::CharacterReady,
                UiTaskCase::CharacterConfirmDelete,
            ] {
                for (width, height, immediately_visible) in [(1920, 1080, true), (960, 540, false)]
                {
                    let snapshot =
                        creator_case_snapshot(case, width, height, 1.0, crate::UiScaleMode::Auto);
                    for (name, accessible_label) in cues {
                        let cue = snapshot
                            .nodes
                            .iter()
                            .find(|node| node.name == name)
                            .unwrap_or_else(|| {
                                panic!("{} is missing {name:?}", case.contract().id)
                            });
                        assert_eq!(
                            cue.accessible_label.as_deref(),
                            Some(accessible_label),
                            "{} must describe {name:?} without relying on color",
                            case.contract().id,
                        );
                        assert!(
                            cue.scroll_reachable,
                            "{} cannot reach {name:?} at {width}x{height}: {cue:?}",
                            case.contract().id,
                        );
                        if immediately_visible {
                            assert!(
                                cue.fully_visible,
                                "{} hides {name:?} in the standard initial view: {cue:?}",
                                case.contract().id,
                            );
                        }
                    }
                }
            }
        }

        #[test]
        fn compact_settings_pointer_scroll_reaches_the_single_page_owner() {
            use bevy::input::{mouse::MouseScrollUnit, touch::TouchPhase};
            use bevy::window::{CursorMoved, PrimaryWindow, WindowEvent};

            let mut app = setup_screen_app(
                960,
                540,
                1.0,
                crate::UiScaleMode::Percent200,
                hex_core::Screen::Settings,
            );
            let root = app
                .world_mut()
                .query::<(Entity, &Name)>()
                .iter(app.world())
                .find_map(|(entity, name)| (name.as_str() == "Settings Screen").then_some(entity))
                .expect("Settings owns a named page root");
            assert!(app.world().get::<ScrollArea>(root).is_some());
            assert_eq!(
                app.world_mut()
                    .query_filtered::<Entity, With<ScrollArea>>()
                    .iter(app.world())
                    .count(),
                1,
                "Compact Settings must not retain a wheel-trapping nested scroll owner"
            );
            let snapshot = ui_tree_snapshot(app.world_mut());
            let target = snapshot
                .nodes
                .iter()
                .find(|node| node.name.starts_with("Setting ") && node.fully_visible)
                .expect("at least one Settings row starts under the cursor");
            let position = target
                .visible_bounds
                .map(|bounds| (bounds.min + bounds.max) * 0.5)
                .expect("the visible Settings row has cursor bounds");
            let window = app
                .world_mut()
                .query_filtered::<Entity, With<PrimaryWindow>>()
                .single(app.world())
                .expect("headless UI owns one primary window");
            let before = app.world().get::<ScrollPosition>(root).unwrap().y;
            app.world_mut()
                .write_message(WindowEvent::CursorMoved(CursorMoved {
                    window,
                    position,
                    delta: None,
                }));
            app.update();
            app.world_mut().write_message(WindowEvent::MouseWheel(
                bevy::input::mouse::MouseWheel {
                    unit: MouseScrollUnit::Line,
                    x: 0.0,
                    y: -100.0,
                    window,
                    phase: TouchPhase::Moved,
                },
            ));
            app.update();
            let after = app.world().get::<ScrollPosition>(root).unwrap().y;
            assert!(
                after > before,
                "real wheel input over a Settings row must move the Compact page: {before} -> {after}"
            );
        }

        #[test]
        fn compact_creator_canvas_preserves_vertical_page_scrolling() {
            use bevy::input::{mouse::MouseScrollUnit, touch::TouchPhase};
            use bevy::window::{CursorMoved, PrimaryWindow, WindowEvent};

            for (width, height, mode) in [
                (960, 540, crate::UiScaleMode::Auto),
                (1280, 720, crate::UiScaleMode::Percent200),
            ] {
                let mut app =
                    creator_case_app(UiTaskCase::CharacterInvalid, width, height, 1.0, mode);
                let named_entity = |app: &mut App, wanted: &str| {
                    app.world_mut()
                        .query::<(Entity, &Name)>()
                        .iter(app.world())
                        .find_map(|(entity, name)| (name.as_str() == wanted).then_some(entity))
                        .unwrap_or_else(|| panic!("missing {wanted:?}"))
                };
                let root = named_entity(&mut app, "Creator Screen");
                let canvas = named_entity(&mut app, "Lattice Canvas");
                assert!(app.world().get::<ScrollArea>(root).is_some());
                assert!(
                    app.world().get::<ScrollArea>(canvas).is_none(),
                    "Compact Creator must not let its lattice consume vertical wheel input"
                );

                let initial = ui_tree_snapshot(app.world_mut());
                let canvas_node = initial
                    .nodes
                    .iter()
                    .find(|node| node.name == "Lattice Canvas")
                    .expect("the Character workspace has a lattice canvas");
                let canvas_top = canvas_node.center.y - canvas_node.size.y * 0.5;
                let root_computed = app.world().get::<ComputedNode>(root).unwrap();
                let root_visible = root_computed.size() * root_computed.inverse_scale_factor;
                let root_content =
                    root_computed.content_size() * root_computed.inverse_scale_factor;
                let root_max_y = (root_content.y - root_visible.y).max(0.0);
                app.world_mut().get_mut::<ScrollPosition>(root).unwrap().y =
                    (canvas_top - 80.0).clamp(0.0, (root_max_y - 80.0).max(0.0));
                for _ in 0..3 {
                    app.update();
                }
                let positioned = ui_tree_snapshot(app.world_mut());
                let canvas_bounds = positioned
                    .nodes
                    .iter()
                    .find(|node| node.name == "Lattice Canvas")
                    .and_then(|node| node.visible_bounds)
                    .expect("the compact page can bring the lattice canvas under the cursor");
                let position = (canvas_bounds.min + canvas_bounds.max) * 0.5;
                let window = app
                    .world_mut()
                    .query_filtered::<Entity, With<PrimaryWindow>>()
                    .single(app.world())
                    .expect("headless UI owns one primary window");
                let send_wheel = |app: &mut App, x, y| {
                    app.world_mut()
                        .write_message(WindowEvent::CursorMoved(CursorMoved {
                            window,
                            position,
                            delta: None,
                        }));
                    app.update();
                    app.world_mut().write_message(WindowEvent::MouseWheel(
                        bevy::input::mouse::MouseWheel {
                            unit: MouseScrollUnit::Line,
                            x,
                            y,
                            window,
                            phase: TouchPhase::Moved,
                        },
                    ));
                    app.update();
                };

                app.world_mut().get_mut::<ScrollPosition>(canvas).unwrap().y = 0.0;
                let root_before_inner = app.world().get::<ScrollPosition>(root).unwrap().y;
                send_wheel(&mut app, 0.0, -1.0);
                assert!(
                    app.world().get::<ScrollPosition>(canvas).unwrap().y > 0.0,
                    "the compact lattice must retain its own vertical pan before reaching its boundary"
                );
                assert!(
                    (app.world().get::<ScrollPosition>(root).unwrap().y - root_before_inner).abs()
                        <= f32::EPSILON,
                    "a fully consumed canvas delta must not also move the page"
                );

                let canvas_computed = app.world().get::<ComputedNode>(canvas).unwrap();
                let canvas_visible = canvas_computed.size() * canvas_computed.inverse_scale_factor;
                let canvas_content =
                    canvas_computed.content_size() * canvas_computed.inverse_scale_factor;
                let canvas_max = (canvas_content - canvas_visible).max(Vec2::ZERO);
                {
                    let mut canvas_position =
                        app.world_mut().get_mut::<ScrollPosition>(canvas).unwrap();
                    canvas_position.x = 0.0;
                    canvas_position.y = canvas_max.y;
                }
                let before = app.world().get::<ScrollPosition>(root).unwrap().y;
                let before_x = app.world().get::<ScrollPosition>(canvas).unwrap().x;
                send_wheel(&mut app, -1.0, -4.0);
                let after = app.world().get::<ScrollPosition>(root).unwrap().y;
                assert!(
                    after > before,
                    "unconsumed vertical wheel input must hand off from the lattice boundary to the Compact Creator page at {width}×{height} {mode:?}: {before} -> {after}"
                );
                if canvas_max.x > 0.5 {
                    assert!(
                        app.world().get::<ScrollPosition>(canvas).unwrap().x > before_x,
                        "a diagonal trackpad event must still pan an overflowing lattice horizontally while its vertical remainder moves the page at {width}×{height} {mode:?}; max={canvas_max:?}, before={before_x}, after={}",
                        app.world().get::<ScrollPosition>(canvas).unwrap().x
                    );
                }
            }
        }

        #[test]
        fn compact_creator_focus_reveals_a_cell_through_both_scroll_owners() {
            let mut app = creator_case_app(
                UiTaskCase::CharacterInvalid,
                960,
                540,
                1.0,
                crate::UiScaleMode::Auto,
            );
            let named_entity = |app: &mut App, wanted: &str| {
                app.world_mut()
                    .query::<(Entity, &Name)>()
                    .iter(app.world())
                    .find_map(|(entity, name)| (name.as_str() == wanted).then_some(entity))
                    .unwrap_or_else(|| panic!("missing {wanted:?}"))
            };
            let root = named_entity(&mut app, "Creator Screen");
            let canvas = named_entity(&mut app, "Lattice Canvas");
            let target = named_entity(&mut app, "Add Cell 0,1");

            app.world_mut().get_mut::<ScrollPosition>(root).unwrap().y = 0.0;
            app.world_mut().get_mut::<ScrollPosition>(canvas).unwrap().0 = Vec2::ZERO;
            for _ in 0..3 {
                app.update();
            }
            let before = ui_tree_snapshot(app.world_mut());
            let target_before = before
                .nodes
                .iter()
                .find(|node| node.name == "Add Cell 0,1")
                .expect("the populated Character fixture must expose a lower lattice cell");
            assert!(
                !target_before.fully_visible,
                "the regression target must begin clipped before focus: {target_before:?}"
            );
            assert_eq!(
                target_before.clipped_by.as_deref(),
                Some("Lattice Canvas"),
                "the initial failure must be nested-canvas clipping, not an unrelated surface"
            );
            let root_before = app.world().get::<ScrollPosition>(root).unwrap().y;
            let canvas_before = app.world().get::<ScrollPosition>(canvas).unwrap().0;

            app.insert_resource(InputFocus::from_entity(target));
            app.insert_resource(bevy::input_focus::InputFocusVisible(true));
            for _ in 0..4 {
                app.update();
            }

            let root_after = app.world().get::<ScrollPosition>(root).unwrap().y;
            let canvas_after = app.world().get::<ScrollPosition>(canvas).unwrap().0;
            assert!(
                canvas_after != canvas_before,
                "focusing a clipped lattice cell must move the custom inner owner: {canvas_before:?} -> {canvas_after:?}"
            );
            assert!(
                root_after > root_before,
                "after revealing the cell in the canvas, focus must reveal that canvas in the Compact page: {root_before} -> {root_after}"
            );

            let after = ui_tree_snapshot(app.world_mut());
            let target_after = after
                .nodes
                .iter()
                .find(|node| node.name == "Add Cell 0,1")
                .expect("the focused lattice cell must remain presented");
            assert!(
                target_after.focused && target_after.fully_visible,
                "the focused cell and its ring must finish fully visible: {target_after:?}"
            );
            assert!(
                app.world().get::<Outline>(target).is_some(),
                "visible keyboard focus must paint the focus ring on the revealed cell"
            );
        }

        #[test]
        fn every_combat_lab_setup_surface_has_a_populated_task_contract() {
            let mut failures = Vec::new();
            for case in [
                UiTaskCase::LabMap,
                UiTaskCase::LabRosters,
                UiTaskCase::LabRostersMax,
                UiTaskCase::LabRules,
                UiTaskCase::LabFixtures,
                UiTaskCase::LabReportsEmpty,
                UiTaskCase::LabReportsPopulated,
            ] {
                let snapshot = lab_case_snapshot(case, 1280, 720, 1.0, crate::UiScaleMode::Auto);
                let issues = task_contract_issues(case, &snapshot);
                if !issues.is_empty() {
                    failures.push((case.contract().id, issues));
                }
            }
            assert!(
                failures.is_empty(),
                "Combat Lab task failures: {failures:#?}"
            );
        }

        #[test]
        fn standard_combat_lab_map_catalog_signposts_its_scroll_route() {
            let snapshot = lab_case_snapshot(
                UiTaskCase::LabMap,
                1920,
                1080,
                1.0,
                crate::UiScaleMode::Auto,
            );
            let cue = snapshot
                .nodes
                .iter()
                .find(|node| node.name == "Combat Lab Map Catalog Scroll Cue")
                .expect("the map catalog must publish a stable scroll affordance");
            assert!(
                cue.fully_visible,
                "the standard review frame must show its map scroll affordance: {cue:?}"
            );
            assert_eq!(
                cue.accessible_label.as_deref(),
                Some("More Combat Lab maps are available by scrolling")
            );
            assert!(
                cue.scroll_reachable,
                "the map scroll affordance must remain reachable when a smaller canvas reflows it"
            );
        }

        #[test]
        fn enlarged_compact_fixture_catalog_has_an_attainable_scroll_range() {
            for (physical_width, physical_height, device_scale) in
                [(960, 540, 1.0), (1920, 1080, 2.0)]
            {
                for mode in [
                    crate::UiScaleMode::Percent175,
                    crate::UiScaleMode::Percent200,
                ] {
                    let snapshot = lab_case_snapshot(
                        UiTaskCase::LabFixtures,
                        physical_width,
                        physical_height,
                        device_scale,
                        mode,
                    );
                    let list = snapshot
                        .nodes
                        .iter()
                        .find(|node| node.name == "Combat Lab Fixture List")
                        .expect("the Fixtures tab owns one named list viewport");
                    let final_control = snapshot
                        .nodes
                        .iter()
                        .find(|node| node.name == "Run Custom three-step")
                        .expect("the populated Fixtures tab exposes its final run control");
                    assert!(
                        final_control.scroll_reachable,
                        "the complete final fixture must fit inside the list's attainable range at {physical_width}×{physical_height} / {device_scale}× in {mode:?}: list={list:?}, target={final_control:?}"
                    );
                }
            }
        }

        #[test]
        fn every_gameplay_decision_pause_and_report_surface_has_a_populated_task_contract() {
            let mut failures = Vec::new();
            let lattice = lattice_demo_snapshot(1280, 720, 1.0, crate::UiScaleMode::Auto);
            let lattice_issues = task_contract_issues(UiTaskCase::LatticeDemo, &lattice);
            if !lattice_issues.is_empty() {
                failures.push((UiTaskCase::LatticeDemo.contract().id, lattice_issues));
            }
            for (case, snapshot) in [
                (
                    UiTaskCase::DeploymentIncomplete,
                    deployment_snapshot(1280, 720, 1.0, crate::UiScaleMode::Auto, false),
                ),
                (
                    UiTaskCase::DeploymentComplete,
                    deployment_snapshot(1280, 720, 1.0, crate::UiScaleMode::Auto, true),
                ),
            ] {
                let issues = task_contract_issues(case, &snapshot);
                if !issues.is_empty() {
                    failures.push((case.contract().id, issues));
                }
            }
            for case in [
                UiTaskCase::Exploration,
                UiTaskCase::PlayerTurnMaxActions,
                UiTaskCase::HostileTurn,
                UiTaskCase::Casting,
                UiTaskCase::AimingBlocked,
                UiTaskCase::DisableDecision,
                UiTaskCase::RestoreDecision,
                UiTaskCase::HudHiddenRequired,
                UiTaskCase::LabStatistics,
                UiTaskCase::Pause,
                UiTaskCase::OrdinaryOutcome,
                UiTaskCase::LabReportOverview,
                UiTaskCase::LabReportUnits,
                UiTaskCase::LabReportSpellsEffects,
                UiTaskCase::LabReportTimeline,
                UiTaskCase::LabReportCompare,
            ] {
                let snapshot =
                    gameplay_case_snapshot(case, 1280, 720, 1.0, crate::UiScaleMode::Auto);
                let issues = task_contract_issues(case, &snapshot);
                if !issues.is_empty() {
                    failures.push((case.contract().id, issues));
                }
            }
            assert!(
                failures.is_empty(),
                "gameplay presentation task failures: {failures:#?}"
            );
        }

        #[test]
        fn active_gameplay_task_contract_rejects_a_missing_own_lattice() {
            let mut snapshot = gameplay_case_snapshot(
                UiTaskCase::Exploration,
                1920,
                1080,
                1.0,
                crate::UiScaleMode::Auto,
            );
            assert_task_contract(UiTaskCase::Exploration, &snapshot);

            snapshot.nodes.retain(|node| node.name != "Own Lattice");
            let issues = task_contract_issues(UiTaskCase::Exploration, &snapshot);
            assert!(
                issues.iter().any(|issue| issue.contains(
                    "gameplay-exploration is missing required own lattice surface \"Own Lattice\""
                )),
                "removing the populated own lattice must fail the active gameplay task contract: {issues:?}"
            );
        }

        #[test]
        fn every_active_gameplay_fixture_publishes_its_lattice_contract() {
            for case in [
                UiTaskCase::Exploration,
                UiTaskCase::PlayerTurnMaxActions,
                UiTaskCase::HostileTurn,
                UiTaskCase::AimingBlocked,
                UiTaskCase::Pause,
                UiTaskCase::Casting,
                UiTaskCase::LabStatistics,
                UiTaskCase::DisableDecision,
                UiTaskCase::RestoreDecision,
                UiTaskCase::HudHiddenRequired,
            ] {
                let snapshot =
                    gameplay_case_snapshot(case, 1920, 1080, 1.0, crate::UiScaleMode::Auto);
                let issues = snapshot.task_lattice_issues(case);
                assert!(
                    issues.is_empty(),
                    "{} must publish its complete lattice presentation contract: {issues:?}",
                    case.contract().id
                );
            }
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

        fn title_case_snapshot(
            case: UiTaskCase,
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
            match case {
                UiTaskCase::TitleCold => {}
                UiTaskCase::TitleResume => {
                    app.world_mut().insert_resource(crate::ResumeView {
                        available: true,
                        message: "Continue the exact saved exploration session.".to_owned(),
                    });
                }
                UiTaskCase::TitleFailure => {
                    app.world_mut().insert_resource(crate::TitleView {
                        setup_failure: Some(
                            "The selected scenario could not be prepared; choose another route."
                                .to_owned(),
                        ),
                    });
                }
                other => panic!("{other:?} is not a title task"),
            }
            app.world_mut()
                .resource_mut::<NextState<hex_core::Screen>>()
                .set(hex_core::Screen::Title);
            for _ in 0..8 {
                app.update();
            }
            ui_tree_snapshot(app.world_mut())
        }

        fn simple_screen_snapshot(
            screen: hex_core::Screen,
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
            app.world_mut()
                .resource_mut::<NextState<hex_core::Screen>>()
                .set(screen);
            for _ in 0..8 {
                app.update();
            }
            ui_tree_snapshot(app.world_mut())
        }

        fn creator_case_app(
            case: UiTaskCase,
            width: u32,
            height: u32,
            scale_factor: f32,
            mode: crate::UiScaleMode,
        ) -> App {
            let screen = case.contract().screen;
            assert!(matches!(
                screen,
                hex_core::Screen::CharacterCreator | hex_core::Screen::SpellCreator
            ));
            let element_file: hex_assets::ElementFile =
                ron::from_str(include_str!("../../../assets/config/elements.ron"))
                    .expect("the production element catalog must parse");
            let elements = hex_assets::ElementCatalog::from_file(&element_file);
            let spell_file: hex_assets::SpellFile =
                ron::from_str(include_str!("../../../assets/config/spells.ron"))
                    .expect("the production spell catalog must parse");
            let spell_book = hex_assets::SpellBook::from_file(&spell_file);
            let presets: hex_assets::CreationPresetCatalog =
                ron::from_str(include_str!("../../../assets/config/creation_presets.ron"))
                    .expect("the production Creator presets must parse");
            let deployable_shipped_spells = spell_book
                .iter()
                .filter(|(_, _, spell)| {
                    matches!(
                        spell.targeting.shape,
                        hex_assets::TargetShape::SelfCast | hex_assets::TargetShape::Single
                    )
                })
                .map(|(_, name, _)| name.to_owned())
                .collect::<Vec<_>>();

            let mut view = crate::CreatorScreenView {
                active: true,
                screen,
                elements: Some(elements),
                spell_book: Some(spell_book),
                spell_file: Some(spell_file),
                presets: Some(presets.clone()),
                deployable_shipped_spells,
                ..default()
            };
            match case {
                UiTaskCase::CharacterLibrary => {
                    view.tab = hex_gameplay_model::CreatorSurface::Characters;
                    view.workspace = crate::CreatorWorkspace::Hub;
                }
                UiTaskCase::SpellLibrary => {
                    view.tab = hex_gameplay_model::CreatorSurface::Spells;
                    view.workspace = crate::CreatorWorkspace::Hub;
                }
                UiTaskCase::CreatorLibraryRecovery => {
                    view.tab = hex_gameplay_model::CreatorSurface::Characters;
                    view.workspace = crate::CreatorWorkspace::Hub;
                    view.library.error = Some(
                        "The local Creator library could not be decoded. Reset it to continue."
                            .to_owned(),
                    );
                    view.confirm_reset = true;
                    view.notice =
                        "Press Confirm Reset to replace the unreadable local library.".to_owned();
                }
                UiTaskCase::CharacterInvalid => {
                    view.tab = hex_gameplay_model::CreatorSurface::Characters;
                    view.workspace = crate::CreatorWorkspace::Character;
                    view.character = Some(hex_assets::SavedCharacter::blank(
                        hex_assets::CustomCharacterId(1),
                        "Validation Fixture",
                    ));
                    view.character_dirty = true;
                    view.character_issues = vec![
                        "Add at least one elemental gem.".to_owned(),
                        "Assign a positive mana capacity.".to_owned(),
                    ];
                }
                UiTaskCase::CharacterReady => {
                    view.tab = hex_gameplay_model::CreatorSurface::Characters;
                    view.workspace = crate::CreatorWorkspace::Character;
                    view.character = presets
                        .characters
                        .iter()
                        .find(|record| record.audience == hex_assets::PresetAudience::HumanTemplate)
                        .map(|record| record.character.clone());
                }
                UiTaskCase::CharacterConfirmDelete => {
                    view.tab = hex_gameplay_model::CreatorSurface::Characters;
                    view.workspace = crate::CreatorWorkspace::Character;
                    view.character = presets
                        .characters
                        .iter()
                        .find(|record| record.audience == hex_assets::PresetAudience::HumanTemplate)
                        .map(|record| record.character.clone());
                    view.confirm_delete = true;
                    view.notice = "Press Confirm Delete to remove this saved character.".to_owned();
                }
                UiTaskCase::SpellInvalid => {
                    view.tab = hex_gameplay_model::CreatorSurface::Spells;
                    view.workspace = crate::CreatorWorkspace::Spell;
                    view.spell = Some(hex_assets::SavedSpell::blank(
                        hex_assets::CustomSpellId(1),
                        "Validation Spell",
                    ));
                    view.spell_dirty = true;
                    view.spell_issues = vec![
                        "Add at least one ordered effect.".to_owned(),
                        "Choose a payable elemental requirement.".to_owned(),
                    ];
                }
                UiTaskCase::SpellReady => {
                    view.tab = hex_gameplay_model::CreatorSurface::Spells;
                    view.workspace = crate::CreatorWorkspace::Spell;
                    view.spell = presets
                        .spells
                        .iter()
                        .find(|record| record.audience == hex_assets::PresetAudience::HumanTemplate)
                        .map(|record| record.spell.clone());
                }
                UiTaskCase::SpellConfirmDelete => {
                    view.tab = hex_gameplay_model::CreatorSurface::Spells;
                    view.workspace = crate::CreatorWorkspace::Spell;
                    view.spell = presets
                        .spells
                        .iter()
                        .find(|record| record.audience == hex_assets::PresetAudience::HumanTemplate)
                        .map(|record| record.spell.clone());
                    view.confirm_delete = true;
                    view.notice = "Press Confirm Delete to remove this saved spell.".to_owned();
                }
                other => panic!("{other:?} is not a Creator task"),
            }

            let mut app = App::new();
            app.add_plugins(HeadlessUiPlugin::with_scale_factor(
                width,
                height,
                scale_factor,
            ));
            app.world_mut()
                .insert_resource(crate::UiScalePreference(mode));
            app.world_mut().insert_resource(view);
            app.world_mut()
                .resource_mut::<NextState<hex_core::Screen>>()
                .set(screen);
            for _ in 0..8 {
                app.update();
            }
            app
        }

        fn creator_case_snapshot(
            case: UiTaskCase,
            width: u32,
            height: u32,
            scale_factor: f32,
            mode: crate::UiScaleMode,
        ) -> UiTreeSnapshot {
            let mut app = creator_case_app(case, width, height, scale_factor, mode);
            ui_tree_snapshot(app.world_mut())
        }

        fn lab_case_snapshot(
            case: UiTaskCase,
            width: u32,
            height: u32,
            scale_factor: f32,
            mode: crate::UiScaleMode,
        ) -> UiTreeSnapshot {
            let maps: hex_assets::CombatLabMapCatalog =
                ron::from_str(include_str!("../../../assets/config/combat_lab_maps.ron"))
                    .expect("the production Combat Lab map catalog must parse");
            let element_file: hex_assets::ElementFile =
                ron::from_str(include_str!("../../../assets/config/elements.ron"))
                    .expect("the production element catalog must parse");
            let elements = hex_assets::ElementCatalog::from_file(&element_file);
            let spell_file: hex_assets::SpellFile =
                ron::from_str(include_str!("../../../assets/config/spells.ron"))
                    .expect("the production spell catalog must parse");
            let spells = hex_assets::SpellBook::from_file(&spell_file);
            let presets: hex_assets::CreationPresetCatalog =
                ron::from_str(include_str!("../../../assets/config/creation_presets.ron"))
                    .expect("the production Creator presets must parse");
            let combat: hex_assets::CombatSettings =
                ron::from_str(include_str!("../../../assets/config/combat.ron"))
                    .expect("the production combat settings must parse");
            let template = |name: &str| hex_gameplay_model::RosterChoice::Template(name.to_owned());
            let map_ready_choices = ["wolf", "raider", "hedge-mage"]
                .into_iter()
                .map(template)
                .collect::<Vec<_>>();
            let mut view = crate::CombatLabScreenView {
                active: true,
                map: maps
                    .maps
                    .first()
                    .map_or_else(String::new, |map| map.id.clone()),
                library: crate::CreatorLibraryView {
                    file: presets.library_for(hex_assets::PresetAudience::HumanTemplate),
                    error: None,
                },
                elements: Some(elements),
                spells: Some(spells),
                presets: Some(presets),
                maps: Some(maps),
                combat: Some(combat.clone()),
                map_ready_choices,
                ..default()
            };
            match case {
                UiTaskCase::LabMap => {}
                UiTaskCase::LabRosters => {
                    view.sandbox_step = hex_gameplay_model::SandboxStep::Rosters;
                    view.players = vec![template("hedge-mage")];
                    view.hostiles = vec![template("raider")];
                }
                UiTaskCase::LabRostersMax => {
                    view.sandbox_step = hex_gameplay_model::SandboxStep::Rosters;
                    view.players = (0..6).map(|_| template("hedge-mage")).collect();
                    view.hostiles = (0..6).map(|_| template("raider")).collect();
                }
                UiTaskCase::LabRules => {
                    view.sandbox_step = hex_gameplay_model::SandboxStep::Rules;
                    view.players = vec![template("hedge-mage")];
                    view.hostiles = vec![template("raider")];
                    view.rules = Some(hex_assets::CombatRulesProfile::shipped(&combat));
                }
                UiTaskCase::LabFixtures => {
                    view.tab = hex_gameplay_model::LabTab::Fixtures;
                }
                UiTaskCase::LabReportsEmpty => {
                    view.tab = hex_gameplay_model::LabTab::Reports;
                }
                UiTaskCase::LabReportsPopulated => {
                    let report = |id, pending_delete| crate::CombatLabReportCardView {
                        id: hex_gameplay_model::CombatLabReportId(id),
                        heading: format!("REPORT {id} · VICTORY"),
                        label: format!("Profile {id}"),
                        notes: "Frozen review notes".to_owned(),
                        metadata: "Flat Arena · seed 77 · fingerprint 04C1B2F7".to_owned(),
                        summary: "Rounds 8 · turns 24 · movement 19 · Channel 4".to_owned(),
                        left_selected: id == 17,
                        right_selected: id == 23,
                        pending_delete,
                    };
                    view.tab = hex_gameplay_model::LabTab::Reports;
                    view.pending_report_delete = Some(hex_gameplay_model::CombatLabReportId(17));
                    view.reports = crate::CombatLabReportsView {
                        error: Some("One older report could not be decoded.".to_owned()),
                        reports: vec![report(17, true), report(23, false)],
                        comparison: Some(crate::CombatLabComparisonView {
                            heading: "COMPARE REPORT 17 ↔ REPORT 23".to_owned(),
                            frozen: "Shipped ↔ Tactical two-step".to_owned(),
                            deltas: "Rounds -2 · commands +3/-1 · Channel +2".to_owned(),
                        }),
                    };
                }
                other => panic!("{other:?} is not a Combat Lab task"),
            }

            let mut app = App::new();
            app.add_plugins(HeadlessUiPlugin::with_scale_factor(
                width,
                height,
                scale_factor,
            ));
            app.world_mut()
                .insert_resource(crate::UiScalePreference(mode));
            app.world_mut().insert_resource(view);
            app.world_mut()
                .resource_mut::<NextState<hex_core::Screen>>()
                .set(hex_core::Screen::CombatLab);
            for _ in 0..8 {
                app.update();
            }
            ui_tree_snapshot(app.world_mut())
        }

        fn lattice_demo_snapshot(
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
            app.world_mut().insert_resource(crate::LatticeDemoView {
                ready: true,
                cells: [
                    (0, 0, "AIR"),
                    (1, 0, "FIRE"),
                    (0, 1, "WATER"),
                    (-1, 1, "EARTH"),
                ]
                .into_iter()
                .map(|(q, r, label)| crate::LatticeCellView {
                    coord: hex_core::LatticeCoord::new(q, r),
                    label: label.to_owned(),
                    detail: "LIVE · 1 MANA".to_owned(),
                    color: Color::srgb(0.35, 0.62, 0.78),
                    known_mana: Some(1),
                    known_locked: Some(false),
                    disabled: false,
                    selected: false,
                    interaction: crate::CellInteraction::Actionable,
                })
                .collect(),
                spells: vec![
                    crate::LatticeDemoSpellView {
                        coord: hex_core::LatticeCoord::new(1, 0),
                        name: "Ember".to_owned(),
                        headline: "Ember · ready".to_owned(),
                        kind: "Evocation".to_owned(),
                        cost: Some(1),
                        blocked: None,
                    },
                    crate::LatticeDemoSpellView {
                        coord: hex_core::LatticeCoord::new(0, 1),
                        name: "Lightning Bolt".to_owned(),
                        headline: "Lightning Bolt · ready".to_owned(),
                        kind: "Evocation".to_owned(),
                        cost: Some(2),
                        blocked: None,
                    },
                ],
                totals: "Mana 4 · disabled 0 · enchantments 0".to_owned(),
                log: (1..=8)
                    .map(|index| format!("Bounded lattice event {index}"))
                    .collect(),
            });
            app.world_mut()
                .resource_mut::<NextState<hex_core::Screen>>()
                .set(hex_core::Screen::LatticeDemo);
            for _ in 0..8 {
                app.update();
            }
            ui_tree_snapshot(app.world_mut())
        }

        fn task_snapshot_at(
            case: UiTaskCase,
            physical_width: u32,
            physical_height: u32,
            scale_factor: f32,
            mode: crate::UiScaleMode,
        ) -> UiTreeSnapshot {
            match case {
                UiTaskCase::Splash => simple_screen_snapshot(
                    hex_core::Screen::Splash,
                    physical_width,
                    physical_height,
                    scale_factor,
                    mode,
                ),
                UiTaskCase::Loading => simple_screen_snapshot(
                    hex_core::Screen::Loading,
                    physical_width,
                    physical_height,
                    scale_factor,
                    mode,
                ),
                UiTaskCase::TitleCold | UiTaskCase::TitleResume | UiTaskCase::TitleFailure => {
                    title_case_snapshot(case, physical_width, physical_height, scale_factor, mode)
                }
                UiTaskCase::MapScenarios => production_scenario_snapshot(
                    physical_width,
                    physical_height,
                    scale_factor,
                    mode,
                    crate::ScenarioBrowserKind::MapScenarios,
                ),
                UiTaskCase::Demos => production_scenario_snapshot(
                    physical_width,
                    physical_height,
                    scale_factor,
                    mode,
                    crate::ScenarioBrowserKind::Demos,
                ),
                UiTaskCase::Settings => setup_screen_snapshot(
                    physical_width,
                    physical_height,
                    scale_factor,
                    mode,
                    hex_core::Screen::Settings,
                ),
                UiTaskCase::CharacterLibrary
                | UiTaskCase::SpellLibrary
                | UiTaskCase::CreatorLibraryRecovery
                | UiTaskCase::CharacterInvalid
                | UiTaskCase::CharacterReady
                | UiTaskCase::CharacterConfirmDelete
                | UiTaskCase::SpellInvalid
                | UiTaskCase::SpellReady
                | UiTaskCase::SpellConfirmDelete => {
                    creator_case_snapshot(case, physical_width, physical_height, scale_factor, mode)
                }
                UiTaskCase::LatticeDemo => {
                    lattice_demo_snapshot(physical_width, physical_height, scale_factor, mode)
                }
                UiTaskCase::LabMap
                | UiTaskCase::LabRosters
                | UiTaskCase::LabRostersMax
                | UiTaskCase::LabRules
                | UiTaskCase::LabFixtures
                | UiTaskCase::LabReportsEmpty
                | UiTaskCase::LabReportsPopulated => {
                    lab_case_snapshot(case, physical_width, physical_height, scale_factor, mode)
                }
                UiTaskCase::DeploymentIncomplete => {
                    deployment_snapshot(physical_width, physical_height, scale_factor, mode, false)
                }
                UiTaskCase::DeploymentComplete => {
                    deployment_snapshot(physical_width, physical_height, scale_factor, mode, true)
                }
                UiTaskCase::Exploration
                | UiTaskCase::PlayerTurnMaxActions
                | UiTaskCase::HostileTurn
                | UiTaskCase::Casting
                | UiTaskCase::AimingBlocked
                | UiTaskCase::DisableDecision
                | UiTaskCase::RestoreDecision
                | UiTaskCase::HudHiddenRequired
                | UiTaskCase::LabStatistics
                | UiTaskCase::Pause
                | UiTaskCase::OrdinaryOutcome
                | UiTaskCase::LabReportOverview
                | UiTaskCase::LabReportUnits
                | UiTaskCase::LabReportSpellsEffects
                | UiTaskCase::LabReportTimeline
                | UiTaskCase::LabReportCompare => gameplay_case_snapshot(
                    case,
                    physical_width,
                    physical_height,
                    scale_factor,
                    mode,
                ),
            }
        }

        fn production_scenario_snapshot(
            physical_width: u32,
            physical_height: u32,
            scale_factor: f32,
            mode: crate::UiScaleMode,
            kind: crate::ScenarioBrowserKind,
        ) -> UiTreeSnapshot {
            let mut app =
                production_scenario_app(physical_width, physical_height, scale_factor, mode, kind);
            ui_tree_snapshot(app.world_mut())
        }

        fn production_scenario_app(
            physical_width: u32,
            physical_height: u32,
            scale_factor: f32,
            mode: crate::UiScaleMode,
            kind: crate::ScenarioBrowserKind,
        ) -> App {
            let library: hex_assets::ScenarioLibrary =
                ron::from_str(include_str!("../../../assets/config/scenarios.ron"))
                    .expect("the production scenario catalog must parse");
            let mut app = App::new();
            app.add_plugins(HeadlessUiPlugin::with_scale_factor(
                physical_width,
                physical_height,
                scale_factor,
            ));
            app.world_mut()
                .insert_resource(crate::UiScalePreference(mode));
            app.world_mut().insert_resource(crate::ScenarioBrowserView {
                kind,
                scenarios: library
                    .visible_scenarios()
                    .filter(|scenario| {
                        scenario.category
                            == match kind {
                                crate::ScenarioBrowserKind::MapScenarios => {
                                    hex_assets::ScenarioCategory::Map
                                }
                                crate::ScenarioBrowserKind::Demos => {
                                    hex_assets::ScenarioCategory::Demo
                                }
                            }
                    })
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
            app
        }

        fn apply_gameplay_review_fixture(app: &mut App, fixture: &str) {
            let mut queue = bevy::ecs::world::CommandQueue::default();
            let mut commands = Commands::new(&mut queue, app.world());
            crate::apply_ui_review_fixture(&mut commands, fixture)
                .expect("the structural fixture name must be valid");
            queue.apply(app.world_mut());
        }

        fn gameplay_fixture_app(
            physical_width: u32,
            physical_height: u32,
            scale_factor: f32,
            mode: crate::UiScaleMode,
            fixture: &str,
        ) -> App {
            let mut app = App::new();
            app.add_plugins(HeadlessUiPlugin::with_scale_factor(
                physical_width,
                physical_height,
                scale_factor,
            ));
            app.world_mut()
                .insert_resource(crate::UiScalePreference(mode));
            // Keep authoritative state ordinary: an authored review fixture must
            // carry every presentation fact it needs instead of relying on a
            // second, test-only reconstruction of gameplay chrome.
            app.world_mut()
                .insert_resource(crate::GameplayChromeView::default());
            apply_gameplay_review_fixture(&mut app, fixture);
            app.world_mut()
                .resource_mut::<NextState<hex_core::Screen>>()
                .set(hex_core::Screen::Gameplay);
            for _ in 0..8 {
                app.update();
            }
            app
        }

        fn gameplay_fixture_snapshot(
            physical_width: u32,
            physical_height: u32,
            scale_factor: f32,
            mode: crate::UiScaleMode,
            fixture: &str,
        ) -> UiTreeSnapshot {
            let mut app =
                gameplay_fixture_app(physical_width, physical_height, scale_factor, mode, fixture);
            ui_tree_snapshot(app.world_mut())
        }

        fn gameplay_case_snapshot(
            case: UiTaskCase,
            physical_width: u32,
            physical_height: u32,
            scale_factor: f32,
            mode: crate::UiScaleMode,
        ) -> UiTreeSnapshot {
            let fixture = match case {
                UiTaskCase::Exploration => "normal-gameplay",
                UiTaskCase::PlayerTurnMaxActions => "player-turn-max",
                UiTaskCase::HostileTurn => "hostile-turn",
                UiTaskCase::Casting => "casting-list",
                UiTaskCase::AimingBlocked => "aiming-disabled",
                UiTaskCase::DisableDecision | UiTaskCase::HudHiddenRequired => "required-decision",
                UiTaskCase::RestoreDecision => "restore-decision",
                UiTaskCase::LabStatistics => "live-statistics",
                UiTaskCase::Pause => "player-turn-max",
                UiTaskCase::OrdinaryOutcome => "ordinary-outcome",
                UiTaskCase::LabReportOverview => "report-overview",
                UiTaskCase::LabReportUnits => "report-units",
                UiTaskCase::LabReportSpellsEffects => "report-spells-effects",
                UiTaskCase::LabReportTimeline => "report-timeline",
                UiTaskCase::LabReportCompare => "report-compare",
                other => panic!("{other:?} is not a gameplay presentation task"),
            };
            let mut app = App::new();
            app.add_plugins(HeadlessUiPlugin::with_scale_factor(
                physical_width,
                physical_height,
                scale_factor,
            ));
            app.world_mut()
                .insert_resource(crate::UiScalePreference(mode));
            app.world_mut().insert_resource(crate::GameplayChromeView {
                shown: case != UiTaskCase::HudHiddenRequired,
                decision_required: matches!(
                    case,
                    UiTaskCase::DisableDecision
                        | UiTaskCase::RestoreDecision
                        | UiTaskCase::HudHiddenRequired
                ),
                encounter_complete: matches!(
                    case,
                    UiTaskCase::OrdinaryOutcome
                        | UiTaskCase::LabReportOverview
                        | UiTaskCase::LabReportUnits
                        | UiTaskCase::LabReportSpellsEffects
                        | UiTaskCase::LabReportTimeline
                        | UiTaskCase::LabReportCompare
                ),
            });
            if case == UiTaskCase::Exploration {
                app.world_mut().insert_resource(crate::PartyView {
                    members: (0..6)
                        .map(|slot| crate::PartyMemberView {
                            slot,
                            label: format!(
                                "{} · {}",
                                if slot == 0 { "Hedge Mage" } else { "Ally" },
                                if slot == 0 { "selected" } else { "ready" }
                            ),
                            active: slot == 0,
                            selected: slot == 0,
                        })
                        .collect(),
                    formation_visible: true,
                    movement_mode: "GROUP · formation follows the selected anchor".to_owned(),
                    presets: vec!["Column".to_owned(), "Wedge".to_owned()],
                    slots: vec![
                        crate::FormationSlotView {
                            offset: hex_core::HexCoord::from_axial(0, 0),
                            anchor: true,
                        },
                        crate::FormationSlotView {
                            offset: hex_core::HexCoord::from_axial(1, 0),
                            anchor: false,
                        },
                        crate::FormationSlotView {
                            offset: hex_core::HexCoord::from_axial(0, 1),
                            anchor: false,
                        },
                    ],
                });
            }
            if case == UiTaskCase::Pause {
                app.world_mut().insert_resource(crate::PauseView {
                    hint: "Esc to resume".to_owned(),
                    notice: Some("Exploration save is current.".to_owned()),
                });
            }
            let mut queue = bevy::ecs::world::CommandQueue::default();
            let mut commands = Commands::new(&mut queue, app.world());
            crate::apply_ui_review_fixture(&mut commands, fixture)
                .expect("every gameplay task must own a registered review fixture");
            queue.apply(app.world_mut());
            app.world_mut()
                .resource_mut::<NextState<hex_core::Screen>>()
                .set(hex_core::Screen::Gameplay);
            for _ in 0..4 {
                app.update();
            }
            if case == UiTaskCase::Pause {
                app.world_mut()
                    .resource_mut::<NextState<hex_core::Pause>>()
                    .set(hex_core::Pause(true));
            }
            for _ in 0..4 {
                app.update();
            }
            ui_tree_snapshot(app.world_mut())
        }

        fn setup_screen_app(
            physical_width: u32,
            physical_height: u32,
            scale_factor: f32,
            mode: crate::UiScaleMode,
            screen: hex_core::Screen,
        ) -> App {
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
                        kind: crate::ScenarioBrowserKind::MapScenarios,
                        scenarios: library
                            .visible_scenarios()
                            .filter(|scenario| {
                                scenario.category == hex_assets::ScenarioCategory::Map
                            })
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
            app
        }

        fn setup_screen_snapshot(
            physical_width: u32,
            physical_height: u32,
            scale_factor: f32,
            mode: crate::UiScaleMode,
            screen: hex_core::Screen,
        ) -> UiTreeSnapshot {
            let mut app =
                setup_screen_app(physical_width, physical_height, scale_factor, mode, screen);
            ui_tree_snapshot(app.world_mut())
        }

        fn deployment_snapshot(
            physical_width: u32,
            physical_height: u32,
            scale_factor: f32,
            mode: crate::UiScaleMode,
            complete: bool,
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
                        position: complete.then(|| {
                            hex_core::TilePos::new(
                                hex_core::HexCoord::from_axial(
                                    i32::try_from(index)
                                        .expect("deployment fixture index is bounded to six"),
                                    0,
                                ),
                                0,
                            )
                        }),
                    })
                    .collect::<Vec<_>>()
            };
            app.world_mut().insert_resource(crate::DeploymentView {
                active: true,
                map_name: "Stacked Surface Arena".to_owned(),
                notice: "Place every player and hostile on an exact legal surface.".to_owned(),
                players: roster("Player"),
                hostiles: roster("Hostile"),
                complete,
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
        fn enlarged_required_choice_collapses_redundant_gameplay_chrome() {
            let snapshot = required_choice_snapshot(1920, 1080, crate::UiScaleMode::Percent200);
            assert_eq!(snapshot.metrics.viewport, crate::UiViewportClass::Compact);
            for redundant in ["Party HUD Region", "Turn HUD Region"] {
                assert!(
                    snapshot.nodes.iter().all(|node| node.name != redundant),
                    "{redundant} must yield to the required choice at 200%"
                );
            }
            for required in ["Primary Action Rail", "Compact Required Lattice Choice"] {
                let node = snapshot
                    .nodes
                    .iter()
                    .find(|node| node.name == required)
                    .unwrap_or_else(|| panic!("missing required surface {required:?}"));
                assert!(
                    node.fully_visible,
                    "{required:?} must remain usable: {node:?}"
                );
            }
        }

        #[test]
        fn visual_walk_fixture_transition_keeps_chrome_and_inspector_coherent() {
            let mut app =
                gameplay_fixture_app(1920, 1080, 1.0, crate::UiScaleMode::Auto, "aiming-disabled");
            let inspector = app
                .world_mut()
                .query::<(Entity, &Name)>()
                .iter(app.world())
                .find_map(|(entity, name)| {
                    (name.as_str() == "Inspector HUD Region").then_some(entity)
                })
                .expect("Gameplay owns one Inspector region");

            app.world_mut()
                .insert_resource(crate::UiScalePreference(crate::UiScaleMode::Percent200));
            apply_gameplay_review_fixture(&mut app, "required-decision");
            for _ in 0..6 {
                app.update();
            }

            assert_eq!(
                *app.world().resource::<crate::GameplayChromeView>(),
                crate::GameplayChromeView::default(),
                "the fixture must not reconstruct or mutate authoritative gameplay chrome"
            );
            let review = app
                .world()
                .resource::<crate::review::UiReviewPresentation>();
            assert!(
                review.chrome.is_some(),
                "the required fixture must author its required-decision presentation fact"
            );
            assert_eq!(
                review.effective_chrome(crate::GameplayChromeView::default()),
                crate::GameplayChromeView {
                    decision_required: true,
                    ..default()
                },
                "the required fixture must merge with authoritative gameplay chrome"
            );
            let required = ui_tree_snapshot(app.world_mut());
            let required_issues = required.review_fixture_issues("required-decision");
            assert!(
                required_issues.is_empty(),
                "the live walk transition must pass the structural oracle: {required_issues:?}"
            );
            let promoted = required
                .nodes
                .iter()
                .find(|node| node.name == "Compact Required Lattice Choice")
                .expect("the enlarged required choice must be promoted into the action region");
            assert!(
                promoted.fully_visible,
                "the promoted required choice must be immediately usable: {promoted:?}"
            );
            assert_eq!(
                app.world().get::<Node>(inspector).map(|node| node.display),
                Some(Display::None),
                "the ordinary Inspector must yield to the promoted required surface"
            );
            assert!(
                required
                    .nodes
                    .iter()
                    .all(|node| node.name != "Inspector HUD Region"),
                "a hidden Inspector must not remain in the visible UI tree"
            );

            app.world_mut()
                .insert_resource(crate::UiScalePreference(crate::UiScaleMode::Auto));
            apply_gameplay_review_fixture(&mut app, "live-statistics");
            for _ in 0..6 {
                app.update();
            }

            assert_eq!(
                app.world()
                    .resource::<crate::review::UiReviewPresentation>()
                    .chrome,
                None,
                "ordinary fixtures must stop overriding authoritative gameplay chrome"
            );
            assert_eq!(
                app.world().get::<Node>(inspector).map(|node| node.display),
                Some(Display::Flex),
                "ordinary presentation must restore the Inspector"
            );
            let ordinary = ui_tree_snapshot(app.world_mut());
            let ordinary_issues = ordinary.review_fixture_issues("live-statistics");
            assert!(
                ordinary_issues.is_empty(),
                "the restored lattice/statistics composition must pass the oracle: {ordinary_issues:?}"
            );
            assert!(
                ordinary
                    .nodes
                    .iter()
                    .all(|node| node.name != "Compact Required Lattice Choice"),
                "the promoted decision surface must leave the visible tree after the decision"
            );
            let lattice = ordinary
                .nodes
                .iter()
                .find(|node| node.name == "Lattice Readout Stack")
                .expect("ordinary Lab presentation must restore the lattice");
            let statistics = ordinary
                .nodes
                .iter()
                .find(|node| node.name == "Combat Lab Live Statistics Drawer")
                .expect("ordinary Lab presentation must restore live statistics");
            assert_eq!(lattice.parent_name.as_deref(), Some("Inspector HUD Region"));
            assert_eq!(
                statistics.parent_name.as_deref(),
                Some("Inspector HUD Region")
            );
            let lattice_bounds = lattice
                .layout_bounds()
                .expect("the restored lattice must have layout");
            let statistics_bounds = statistics
                .layout_bounds()
                .expect("the restored statistics must have layout");
            assert!(
                lattice_bounds.max.y <= statistics_bounds.min.y + 0.5,
                "statistics must remain below the lattice after the transition: lattice={lattice_bounds:?}, statistics={statistics_bounds:?}"
            );
        }

        #[test]
        fn deployment_does_not_reserve_an_empty_combat_action_rail() {
            for complete in [false, true] {
                let snapshot =
                    deployment_snapshot(1920, 1080, 1.0, crate::UiScaleMode::Auto, complete);
                assert!(
                    snapshot
                        .nodes
                        .iter()
                        .all(|node| node.name != "Primary Action Rail"),
                    "deployment primary actions live in its setup surface; an empty combat rail must not cover the map"
                );
            }
        }

        #[test]
        fn wide_dense_report_uses_the_available_comparison_canvas() {
            let snapshot = gameplay_case_snapshot(
                UiTaskCase::LabReportCompare,
                3840,
                2160,
                1.0,
                crate::UiScaleMode::Auto,
            );
            let panel = snapshot
                .nodes
                .iter()
                .find(|node| node.name == "Encounter Outcome Panel")
                .expect("the dense report fixture must render its report panel");
            assert!(
                panel.size.x >= snapshot.metrics.logical_size.x * 0.55,
                "the comparison report must not collapse into a narrow 4K column: {panel:?}"
            );
            assert!(
                panel.size.y <= snapshot.metrics.logical_size.y * 0.75,
                "a populated report should not reserve a nearly empty full-height 4K modal: {panel:?}"
            );
            assert!(
                panel.fully_visible,
                "the report must remain on-canvas: {panel:?}"
            );
        }

        #[test]
        fn statistics_follow_the_populated_lattice_through_reflow_and_decisions() {
            let mut app = App::new();
            app.add_plugins(HeadlessUiPlugin::with_scale_factor(1920, 1080, 1.0));
            app.world_mut().insert_resource(crate::GameplayChromeView {
                shown: true,
                ..default()
            });
            app.world_mut()
                .insert_resource(crate::UiScalePreference(crate::UiScaleMode::Percent200));
            let mut queue = bevy::ecs::world::CommandQueue::default();
            let mut commands = Commands::new(&mut queue, app.world());
            crate::apply_ui_review_fixture(&mut commands, "live-statistics")
                .expect("the live statistics fixture must be registered");
            queue.apply(app.world_mut());
            app.world_mut()
                .resource_mut::<NextState<hex_core::Screen>>()
                .set(hex_core::Screen::Gameplay);
            for _ in 0..8 {
                app.update();
            }

            app.world_mut()
                .insert_resource(crate::UiScalePreference(crate::UiScaleMode::Auto));
            for _ in 0..8 {
                app.update();
            }

            let snapshot = ui_tree_snapshot(app.world_mut());
            assert_eq!(snapshot.metrics.viewport, crate::UiViewportClass::Standard);
            assert!(
                snapshot.review_fixture_issues("live-statistics").is_empty(),
                "the restored drawer must not retain Compact row geometry: {:?}",
                snapshot.review_fixture_issues("live-statistics")
            );
            let end = snapshot
                .nodes
                .iter()
                .find(|node| node.name == "End experiment and save the current Combat Lab report")
                .expect("the expanded drawer must expose its final action");
            assert!(
                end.scroll_reachable,
                "the final drawer action must have a complete Inspector scroll route: {end:?}"
            );
            let drawer = snapshot
                .nodes
                .iter()
                .find(|node| node.name == "Combat Lab Live Statistics Drawer")
                .and_then(UiNodeObservation::layout_bounds)
                .expect("the expanded statistics drawer must retain layout");
            for retained in [
                "Inspector HUD Region",
                "Own Lattice Panel",
                "Target Lattice Panel",
            ] {
                assert!(
                    snapshot.nodes.iter().any(|node| node.name == retained),
                    "expanded statistics must retain {retained}"
                );
            }
            let lattice = snapshot
                .nodes
                .iter()
                .find(|node| node.name == "Lattice Readout Stack")
                .and_then(UiNodeObservation::layout_bounds)
                .expect("the populated lattice stack must retain layout");
            assert!(
                lattice.max.y <= drawer.min.y + 0.5,
                "the lattice must precede expanded statistics: lattice={lattice:?}, drawer={drawer:?}"
            );

            assert!(
                snapshot
                    .nodes
                    .iter()
                    .find(|node| node.name == "Combat Lab Statistics Body")
                    .and_then(UiNodeObservation::layout_bounds)
                    .is_some(),
                "expanded statistics must lay out its body below the lattice"
            );

            app.world_mut()
                .insert_resource(crate::UiScalePreference(crate::UiScaleMode::Percent200));
            for _ in 0..8 {
                app.update();
            }
            let compact = ui_tree_snapshot(app.world_mut());
            assert_eq!(compact.metrics.viewport, crate::UiViewportClass::Compact);
            assert!(
                compact.review_fixture_issues("live-statistics").is_empty(),
                "Standard-to-Compact reparenting failed: {:?}",
                compact.review_fixture_issues("live-statistics")
            );

            app.world_mut()
                .insert_resource(crate::UiScalePreference(crate::UiScaleMode::Auto));
            for _ in 0..8 {
                app.update();
            }
            let restored_standard = ui_tree_snapshot(app.world_mut());
            assert!(
                restored_standard
                    .review_fixture_issues("live-statistics")
                    .is_empty(),
                "Compact-to-Standard reparenting failed: {:?}",
                restored_standard.review_fixture_issues("live-statistics")
            );
            let restored_lattice = restored_standard
                .nodes
                .iter()
                .find(|node| node.name == "Lattice Readout Stack")
                .and_then(UiNodeObservation::layout_bounds)
                .expect("returning to Standard must restore the lattice readout");
            assert!(
                (restored_lattice.min - lattice.min).abs().max_element() <= 0.5
                    && (restored_lattice.max - lattice.max).abs().max_element() <= 0.5,
                "Standard-to-Compact-to-Standard reflow moved the lattice: before={lattice:?}, after={restored_lattice:?}"
            );

            app.world_mut()
                .resource_mut::<crate::GameplayChromeView>()
                .decision_required = true;
            for _ in 0..8 {
                app.update();
            }
            let required = ui_tree_snapshot(app.world_mut());
            assert!(
                required
                    .nodes
                    .iter()
                    .all(|node| node.name != "Combat Lab Live Statistics Drawer"),
                "a blocking decision must hide the secondary statistics drawer"
            );
            for restored in [
                "Inspector HUD Region",
                "Own Lattice Panel",
                "Target Lattice Panel",
            ] {
                assert!(
                    required.nodes.iter().any(|node| node.name == restored),
                    "a blocking decision must retain {restored}"
                );
            }

            app.world_mut()
                .resource_mut::<crate::GameplayChromeView>()
                .decision_required = false;
            app.world_mut()
                .resource_mut::<crate::review::UiReviewPresentation>()
                .statistics
                .as_mut()
                .expect("the review fixture must retain statistics")
                .expanded = false;
            for _ in 0..8 {
                app.update();
            }
            let collapsed = ui_tree_snapshot(app.world_mut());
            assert!(
                collapsed.review_fixture_issues("casting-list").is_empty(),
                "the collapsed inspector contract failed after reflow: {:?}",
                collapsed.review_fixture_issues("casting-list")
            );
            for restored in [
                "Inspector HUD Region",
                "Own Lattice Panel",
                "Target Lattice Panel",
            ] {
                assert!(
                    collapsed.nodes.iter().any(|node| node.name == restored),
                    "collapsing statistics must retain {restored}"
                );
            }
            let collapsed_drawer = collapsed
                .nodes
                .iter()
                .find(|node| node.name == "Combat Lab Live Statistics Drawer")
                .and_then(|node| node.visible_bounds)
                .expect("the collapsed drawer controls remain visible");
            let lattice = collapsed
                .nodes
                .iter()
                .find(|node| node.name == "Lattice Readout Stack")
                .and_then(|node| node.visible_bounds)
                .expect("collapsing statistics must retain the inspector lattice");
            assert!(
                lattice.max.y <= collapsed_drawer.min.y + 0.5,
                "collapsed statistics must follow the lattice: lattice={lattice:?}, drawer={collapsed_drawer:?}"
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
                    let prefix = if crate::layout::is_ultra_constrained(snapshot.metrics) {
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
                    "Character Creator",
                    "Spell Creator",
                    "Combat Lab",
                    "Map Scenarios",
                    "Demos",
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
        fn production_scenario_catalog_scrolls_beneath_a_persistent_footer() {
            let snapshot = production_scenario_snapshot(
                960,
                540,
                1.0,
                crate::UiScaleMode::Percent200,
                crate::ScenarioBrowserKind::MapScenarios,
            );
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
            assert!(
                back.center.y > snapshot.metrics.logical_size.y * 0.75,
                "Back must remain in the persistent lower footer: {back:?}"
            );
            let catalog = snapshot
                .nodes
                .iter()
                .find(|node| node.name == "Scenario Catalog Viewport")
                .expect("scenario catalog has a dedicated scroll viewport");
            assert!(
                catalog.fully_visible,
                "catalog viewport must fit: {catalog:?}"
            );
            let title = snapshot
                .nodes
                .iter()
                .find(|node| node.name == "Scenario Screen Title")
                .expect("scenario catalog has a named screen title");
            let title_glyphs = title
                .rendered_text_bounds
                .expect("the scenario title glyphs must be laid out");
            assert!(
                title.fully_visible && title_glyphs.min.y >= 8.0,
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
        fn production_scenario_catalog_consumes_real_pointer_scroll_input() {
            use bevy::camera::NormalizedRenderTarget;
            use bevy::input::{mouse::MouseScrollUnit, touch::TouchPhase};
            use bevy::picking::{
                backend::HitData,
                events::{Pointer, Scroll},
                pointer::{Location, PointerId},
            };
            use bevy::window::{PrimaryWindow, WindowRef};

            let mut app = production_scenario_app(
                960,
                540,
                1.0,
                crate::UiScaleMode::Auto,
                crate::ScenarioBrowserKind::MapScenarios,
            );
            assert!(
                app.is_plugin_added::<bevy::ui_widgets::ScrollAreaPlugin>(),
                "the runtime UI stack must install the observer that turns wheel/trackpad events into ScrollPosition changes"
            );
            assert!(app
                .world_mut()
                .query::<&Name>()
                .iter(app.world())
                .any(|name| name.as_str() == "Scenario Catalog Scrollbar"));
            let scroller = app
                .world_mut()
                .query::<(Entity, &Name, Option<&ScrollArea>)>()
                .iter(app.world())
                .find_map(|(entity, name, scroll)| {
                    (name.as_str() == "Scenario Catalog Viewport" && scroll.is_some())
                        .then_some(entity)
                })
                .expect("the scenario screen has one catalog scroller");
            let target = app
                .world_mut()
                .query::<(Entity, &Name)>()
                .iter(app.world())
                .find_map(|(entity, name)| (name.as_str() == "The Crossing").then_some(entity))
                .expect("production maps include a button below the scroller");
            let before = app.world().get::<ScrollPosition>(scroller).unwrap().y;
            let window = app
                .world_mut()
                .query_filtered::<Entity, With<PrimaryWindow>>()
                .single(app.world())
                .expect("headless UI owns one primary window");
            let normalized_window = WindowRef::Entity(window)
                .normalize(Some(window))
                .expect("an explicit window always normalizes");
            app.world_mut().trigger(Pointer::new(
                PointerId::Mouse,
                Location {
                    target: NormalizedRenderTarget::Window(normalized_window),
                    position: Vec2::new(480.0, 270.0),
                },
                Scroll {
                    unit: MouseScrollUnit::Line,
                    x: 0.0,
                    y: -4.0,
                    hit: HitData::new(Entity::PLACEHOLDER, 0.0, None, None),
                    phase: TouchPhase::Moved,
                },
                target,
            ));
            let after = app.world().get::<ScrollPosition>(scroller).unwrap().y;
            assert!(
                after > before,
                "a wheel event over a scenario button must bubble to the catalog scroller: {before} -> {after}"
            );
        }

        #[test]
        fn compact_retina_scenario_heading_keeps_display_glyphs_off_the_target_edge() {
            for kind in [
                crate::ScenarioBrowserKind::MapScenarios,
                crate::ScenarioBrowserKind::Demos,
            ] {
                let snapshot =
                    production_scenario_snapshot(2560, 1440, 2.0, crate::UiScaleMode::Auto, kind);
                let title = snapshot
                    .nodes
                    .iter()
                    .find(|node| node.name == "Scenario Screen Title")
                    .unwrap_or_else(|| panic!("the {kind:?} catalog must have a screen heading"));
                let glyphs = title
                    .rendered_text_bounds
                    .expect("the screen heading must have measured glyphs");
                assert!(
                    glyphs.min.y >= 32.0 && !title.overflows,
                    "the game-only Retina target needs a contained, visibly inset screen title: {title:?}"
                );
            }
        }

        #[test]
        fn compact_retina_map_catalog_uses_two_readable_card_columns() {
            let snapshot = production_scenario_snapshot(
                2560,
                1440,
                2.0,
                crate::UiScaleMode::Auto,
                crate::ScenarioBrowserKind::MapScenarios,
            );
            assert!(
                snapshot.layout_issues().is_empty(),
                "the compact Retina map catalog must remain structurally valid: {:?}",
                snapshot.layout_issues()
            );
            let crossing = snapshot
                .nodes
                .iter()
                .find(|node| node.name == "The Crossing")
                .expect("the production map catalog includes The Crossing");
            let hills = snapshot
                .nodes
                .iter()
                .find(|node| node.name == "Procedural Hills")
                .expect("the production map catalog includes Procedural Hills");
            assert!(
                (crossing.center.x - hills.center.x).abs() > 100.0,
                "a 1280px logical catalog should use two scan-friendly columns: crossing={crossing:?}, hills={hills:?}"
            );
            assert!(
                crossing.size.x < snapshot.metrics.logical_size.x * 0.48
                    && hills.size.x < snapshot.metrics.logical_size.x * 0.48,
                "scenario cards must not become full-width slabs: crossing={crossing:?}, hills={hills:?}"
            );
            let scrollbar = snapshot
                .nodes
                .iter()
                .find(|node| node.name == "Scenario Catalog Scrollbar")
                .expect("the production map catalog exposes a visible scrollbar");
            assert!(
                scrollbar.size.x <= 10.5,
                "the scrollbar must remain a secondary affordance: {scrollbar:?}"
            );
        }

        #[test]
        fn ordinary_gameplay_keeps_most_of_the_canvas_for_the_world() {
            let snapshot = gameplay_fixture_snapshot(
                1920,
                1080,
                1.0,
                crate::UiScaleMode::Auto,
                "casting-list",
            );
            let rail = snapshot
                .nodes
                .iter()
                .find(|node| node.name == "Primary Action Rail")
                .expect("ordinary gameplay has a primary action rail");
            let casting = snapshot
                .nodes
                .iter()
                .find(|node| node.name == "Casting Panel")
                .expect("ordinary gameplay has a spell strip");
            let rail_top = rail.center.y - rail.size.y * 0.5;
            let casting_top = casting.center.y - casting.size.y * 0.5;
            assert!(
                rail.size.y <= 120.0 && rail_top >= 940.0,
                "ordinary rail must stay a compact bottom strip: {rail:?}"
            );
            assert!(
                casting.size.y <= 90.0 && casting_top >= 800.0,
                "ordinary spell actions must leave the world readable: {casting:?}"
            );
            assert!(
                snapshot
                    .nodes
                    .iter()
                    .all(|node| node.name != "Action Rail Prompt"),
                "generic instructional copy must not consume a permanent HUD row"
            );
        }

        #[test]
        fn combat_lab_statistics_follow_the_lattice_at_retina_size() {
            // Include the reported outer-window dimensions, their actual client
            // canvas, the previous fullscreen client, and both sides of the
            // ultra-Compact height boundary. A title bar must never be mistaken
            // for renderable Bevy client space.
            for physical in [
                UVec2::new(3024, 1898),
                UVec2::new(2582, 1494),
                UVec2::new(2582, 1442),
                UVec2::new(2582, 1400),
                UVec2::new(2582, 1398),
            ] {
                for mode in all_scale_modes() {
                    let collapsed = gameplay_fixture_snapshot(
                        physical.x,
                        physical.y,
                        2.0,
                        mode,
                        "casting-list",
                    );
                    assert!(
                        collapsed.review_fixture_issues("casting-list").is_empty(),
                        "collapsed Retina presentation contract failed at {physical:?} in {mode:?}: {:?}",
                        collapsed.review_fixture_issues("casting-list")
                    );
                    let bounds = |name: &str| {
                        collapsed
                            .nodes
                            .iter()
                            .find(|node| node.name == name)
                            .and_then(UiNodeObservation::layout_bounds)
                            .unwrap_or_else(|| {
                                panic!(
                                    "{name} must have layout in the collapsed Lab state at {physical:?} in {mode:?}"
                                )
                            })
                    };
                    let drawer = bounds("Combat Lab Live Statistics Drawer");
                    let lattice = bounds("Lattice Readout Stack");
                    assert!(
                        lattice.max.y <= drawer.min.y + 0.5,
                        "collapsed statistics must follow the lattice at {physical:?} in {mode:?}: drawer {drawer:?}, lattice {lattice:?}"
                    );

                    let expanded = gameplay_fixture_snapshot(
                        physical.x,
                        physical.y,
                        2.0,
                        mode,
                        "live-statistics",
                    );
                    assert!(
                        expanded.review_fixture_issues("live-statistics").is_empty(),
                        "expanded Retina presentation contract failed at {physical:?} in {mode:?}: {:?}",
                        expanded.review_fixture_issues("live-statistics")
                    );
                    let expanded_bounds = |name: &str| {
                        expanded
                            .nodes
                            .iter()
                            .find(|node| node.name == name)
                            .and_then(UiNodeObservation::layout_bounds)
                            .unwrap_or_else(|| {
                                panic!(
                                    "{name} must retain layout in the expanded Lab state at {physical:?} in {mode:?}"
                                )
                            })
                    };
                    let expanded_lattice = expanded_bounds("Lattice Readout Stack");
                    let expanded_drawer = expanded_bounds("Combat Lab Live Statistics Drawer");
                    assert!(
                        expanded_lattice.max.y <= expanded_drawer.min.y + 0.5,
                        "expanded statistics must follow the lattice at {physical:?} in {mode:?}: drawer {expanded_drawer:?}, lattice {expanded_lattice:?}"
                    );
                    assert!(
                        (expanded_lattice.min - lattice.min).abs().max_element() <= 0.5
                            && (expanded_lattice.max - lattice.max).abs().max_element() <= 0.5,
                        "expanding statistics must not move or resize the persistent lattice at {physical:?} in {mode:?}: collapsed={lattice:?}, expanded={expanded_lattice:?}"
                    );
                }
            }
        }

        #[test]
        fn hiding_the_hud_cannot_leave_statistics_without_the_lattice() {
            let mut app =
                gameplay_fixture_app(2582, 1442, 2.0, crate::UiScaleMode::Auto, "live-statistics");
            app.world_mut()
                .resource_mut::<crate::GameplayChromeView>()
                .shown = false;
            for _ in 0..4 {
                app.update();
            }
            let inspector = app
                .world_mut()
                .query::<(Entity, &Name)>()
                .iter(app.world())
                .find_map(|(entity, name)| {
                    (name.as_str() == "Inspector HUD Region").then_some(entity)
                })
                .expect("Gameplay owns one Inspector region");
            assert_eq!(
                app.world().get::<Node>(inspector).unwrap().display,
                Display::None,
                "an ordinary hidden HUD must remove the empty Inspector from layout and picking"
            );
            let hidden = ui_tree_snapshot(app.world_mut());
            for surface in [
                "Own Lattice Panel",
                "Target Lattice Panel",
                "Combat Lab Live Statistics Drawer",
            ] {
                assert!(
                    hidden.nodes.iter().all(|node| node.name != surface),
                    "HUD hiding must hide lattice and secondary statistics together; {surface:?} remained: {:?}",
                    hidden
                        .nodes
                        .iter()
                        .filter(|node| node.name.contains("Lattice") || node.name.contains("Statistics"))
                        .collect::<Vec<_>>()
                );
            }
            assert!(
                hidden
                    .nodes
                    .iter()
                    .any(|node| node.name == "Primary Action Rail"),
                "the persistent action rail remains outside ordinary HUD visibility"
            );
            assert!(
                hidden
                    .nodes
                    .iter()
                    .all(|node| node.name != "Inspector HUD Region"),
                "a hidden empty Inspector must not remain a transparent wheel trap"
            );
        }

        #[test]
        fn stale_statistics_cannot_render_without_a_lattice_or_after_the_encounter() {
            let mut app = App::new();
            app.add_plugins(HeadlessUiPlugin::with_scale_factor(2582, 1442, 2.0));
            app.world_mut()
                .insert_resource(crate::UiScalePreference(crate::UiScaleMode::Auto));
            app.world_mut()
                .insert_resource(crate::GameplayChromeView::default());
            app.world_mut().insert_resource(crate::LabStatisticsView {
                present: true,
                visible: true,
                expanded: true,
                text: "Intentionally stale live statistics".to_owned(),
            });
            app.world_mut()
                .resource_mut::<NextState<hex_core::Screen>>()
                .set(hex_core::Screen::Gameplay);
            for _ in 0..8 {
                app.update();
            }
            let drawer = app
                .world_mut()
                .query::<(Entity, &Name)>()
                .iter(app.world())
                .find_map(|(entity, name)| {
                    (name.as_str() == "Combat Lab Live Statistics Drawer").then_some(entity)
                })
                .expect("Gameplay owns one dormant statistics drawer");
            let missing_lattice = ui_tree_snapshot(app.world_mut());
            assert!(
                missing_lattice.nodes.iter().all(|node| {
                    node.name != "Combat Lab Live Statistics Drawer"
                        && node.name != "Own Lattice"
                }),
                "a stale visible statistics projection must fail closed until the persistent lattice exists"
            );
            assert_eq!(
                app.world().get::<Node>(drawer).unwrap().display,
                Display::None
            );
            assert!(
                app.world()
                    .get::<ComputedNode>(drawer)
                    .unwrap()
                    .size()
                    .max_element()
                    <= 0.5,
                "a stale hidden drawer must not leave blank Inspector layout or scroll extent"
            );

            app.world_mut()
                .insert_resource(populated_gameplay_lattices());
            for _ in 0..8 {
                app.update();
            }
            let coherent = ui_tree_snapshot(app.world_mut());
            assert!(coherent
                .nodes
                .iter()
                .any(|node| node.name == "Own Lattice Panel"));
            assert!(coherent
                .nodes
                .iter()
                .any(|node| node.name == "Combat Lab Live Statistics Drawer"));
            assert_eq!(
                app.world().get::<Node>(drawer).unwrap().display,
                Display::Flex
            );
            assert!(app.world().get::<ComputedNode>(drawer).unwrap().size().y > 0.5);

            app.world_mut()
                .resource_mut::<crate::GameplayChromeView>()
                .encounter_complete = true;
            for _ in 0..4 {
                app.update();
            }
            let complete = ui_tree_snapshot(app.world_mut());
            for surface in [
                "Own Lattice Panel",
                "Target Lattice Panel",
                "Combat Lab Live Statistics Drawer",
            ] {
                assert!(
                    complete.nodes.iter().all(|node| node.name != surface),
                    "terminal chrome must hide stale lattice/statistics together; {surface:?} remained"
                );
            }
            assert_eq!(
                app.world().get::<Node>(drawer).unwrap().display,
                Display::None
            );
            assert!(
                app.world()
                    .get::<ComputedNode>(drawer)
                    .unwrap()
                    .size()
                    .max_element()
                    <= 0.5,
                "terminal statistics must not retain hidden scroll extent"
            );
        }

        #[test]
        fn compact_inspector_consumes_real_scroll_input_to_reach_statistics_end() {
            use bevy::input::{mouse::MouseScrollUnit, touch::TouchPhase};
            use bevy::ui_widgets::ScrollArea;
            use bevy::window::{CursorMoved, PrimaryWindow, WindowEvent};

            let mut app =
                gameplay_fixture_app(960, 540, 1.0, crate::UiScaleMode::Auto, "live-statistics");
            let entity_named = |app: &mut App, wanted: &str| {
                app.world_mut()
                    .query::<(Entity, &Name)>()
                    .iter(app.world())
                    .find_map(|(entity, name)| (name.as_str() == wanted).then_some(entity))
                    .unwrap_or_else(|| panic!("missing {wanted:?}"))
            };
            let inspector = entity_named(&mut app, "Inspector HUD Region");
            let body = entity_named(&mut app, "Combat Lab Statistics Body");
            assert!(
                app.world().get::<ScrollArea>(inspector).is_some(),
                "the Inspector must own the vertical scroll route"
            );
            assert!(
                app.world().get::<ScrollArea>(body).is_none(),
                "statistics must not trap wheel input in a nested scroll owner"
            );
            let window = app
                .world_mut()
                .query_filtered::<Entity, With<PrimaryWindow>>()
                .single(app.world())
                .expect("headless UI owns one primary window");
            let initial = ui_tree_snapshot(app.world_mut());
            let own_bounds = initial
                .nodes
                .iter()
                .find(|node| node.name == "Own Lattice Panel")
                .and_then(|node| node.visible_bounds)
                .expect("the own lattice starts inside the Inspector viewport");
            let pointer_position = (own_bounds.min + own_bounds.max) * 0.5;
            let scroll = |app: &mut App, position: Vec2| {
                app.world_mut()
                    .write_message(WindowEvent::CursorMoved(CursorMoved {
                        window,
                        position,
                        delta: None,
                    }));
                app.update();
                app.world_mut().write_message(WindowEvent::MouseWheel(
                    bevy::input::mouse::MouseWheel {
                        unit: MouseScrollUnit::Line,
                        x: 0.0,
                        y: -100.0,
                        window,
                        phase: TouchPhase::Moved,
                    },
                ));
                app.update();
            };

            scroll(&mut app, pointer_position);
            assert!(
                app.world().get::<ScrollPosition>(inspector).unwrap().y > 0.0,
                "a real cursor and wheel event over read-only lattice content must move the Inspector scroll owner"
            );
            for _ in 0..3 {
                app.update();
            }
            let scrolled = ui_tree_snapshot(app.world_mut());
            let end = scrolled
                .nodes
                .iter()
                .find(|node| node.name == "Combat Lab Statistics Detail End")
                .expect("the statistics end marker remains structurally present");
            assert!(
                end.fully_visible,
                "the single real Inspector scroll route must reveal the complete statistics end: {end:?}"
            );
        }

        #[test]
        fn compact_tab_navigation_scrolls_statistics_focus_into_view() {
            use bevy::input::{
                keyboard::{Key, KeyboardInput},
                ButtonState,
            };
            use bevy::window::PrimaryWindow;

            fn send_key(
                app: &mut App,
                window: Entity,
                key_code: KeyCode,
                logical_key: Key,
                state: ButtonState,
            ) {
                app.world_mut().write_message(KeyboardInput {
                    key_code,
                    logical_key,
                    state,
                    text: None,
                    repeat: false,
                    window,
                });
                app.update();
            }

            let mut app =
                gameplay_fixture_app(960, 540, 1.0, crate::UiScaleMode::Auto, "live-statistics");
            let window = app
                .world_mut()
                .query_filtered::<Entity, With<PrimaryWindow>>()
                .single(app.world())
                .expect("headless UI owns one primary window");
            let inspector = app
                .world_mut()
                .query::<(Entity, &Name)>()
                .iter(app.world())
                .find_map(|(entity, name)| {
                    (name.as_str() == "Inspector HUD Region").then_some(entity)
                })
                .expect("gameplay owns an Inspector scroll region");
            let initial = ui_tree_snapshot(app.world_mut());
            let end_initial = initial
                .nodes
                .iter()
                .find(|node| node.name == "End experiment and save the current Combat Lab report")
                .expect("the secondary Lab control is structurally present");
            assert!(
                !end_initial.fully_visible,
                "the compact fixture must genuinely require keyboard-driven scrolling"
            );

            let mut reached_end = false;
            for _ in 0..=initial.focus_order.len() {
                send_key(
                    &mut app,
                    window,
                    KeyCode::Tab,
                    Key::Tab,
                    ButtonState::Pressed,
                );
                send_key(
                    &mut app,
                    window,
                    KeyCode::Tab,
                    Key::Tab,
                    ButtonState::Released,
                );
                let focus = app.world().resource::<InputFocus>().get();
                reached_end = focus.is_some_and(|entity| {
                    app.world().get::<Name>(entity).is_some_and(|name| {
                        name.as_str() == "End experiment and save the current Combat Lab report"
                    })
                });
                if reached_end {
                    break;
                }
            }
            assert!(
                reached_end,
                "real Tab navigation must reach the offscreen Lab control; order={:?}",
                initial.focus_order
            );
            for _ in 0..3 {
                app.update();
            }
            assert!(
                app.world().get::<ScrollPosition>(inspector).unwrap().y > 0.0,
                "focusing an offscreen control must scroll its Inspector owner"
            );
            let end_focused = ui_tree_snapshot(app.world_mut());
            assert!(
                end_focused
                    .nodes
                    .iter()
                    .find(|node| {
                        node.name == "End experiment and save the current Combat Lab report"
                    })
                    .is_some_and(|node| node.fully_visible && node.focused),
                "the keyboard focus ring must be visible with the focused End Experiment control"
            );

            send_key(
                &mut app,
                window,
                KeyCode::ShiftLeft,
                Key::Shift,
                ButtonState::Pressed,
            );
            send_key(
                &mut app,
                window,
                KeyCode::Tab,
                Key::Tab,
                ButtonState::Pressed,
            );
            send_key(
                &mut app,
                window,
                KeyCode::Tab,
                Key::Tab,
                ButtonState::Released,
            );
            send_key(
                &mut app,
                window,
                KeyCode::ShiftLeft,
                Key::Shift,
                ButtonState::Released,
            );
            for _ in 0..3 {
                app.update();
            }
            let toggle_focused = ui_tree_snapshot(app.world_mut());
            assert!(
                toggle_focused
                    .nodes
                    .iter()
                    .find(|node| { node.name == "Expand or collapse live Combat Lab statistics" })
                    .is_some_and(|node| node.fully_visible && node.focused),
                "Shift-Tab must keep the preceding statistics control and its focus ring visible"
            );
        }

        #[test]
        fn live_statistics_oracle_rejects_missing_reordered_or_occluded_surfaces() {
            let snapshot = gameplay_fixture_snapshot(
                3024,
                1964,
                2.0,
                crate::UiScaleMode::Auto,
                "live-statistics",
            );

            let mut missing = snapshot.clone();
            missing
                .nodes
                .retain(|node| node.name != "Lattice Readout Stack");
            let missing_issues = missing.review_fixture_issues("live-statistics");
            assert!(
                missing_issues
                    .iter()
                    .any(|issue| issue
                        .contains("missing required surface \"Lattice Readout Stack\"")),
                "the fixture oracle must reject a missing lattice: {missing_issues:?}"
            );
            let cross_fixture_issues = missing.review_fixture_issues("normal-gameplay");
            assert!(
                cross_fixture_issues.iter().any(|issue| issue.contains(
                    "presented statistics require exactly one lattice stack and own panel"
                )),
                "the statistics/lattice implication must apply outside named Lab fixtures: {cross_fixture_issues:?}"
            );

            let mut zero_area = snapshot.clone();
            let zero_lattice = zero_area
                .nodes
                .iter_mut()
                .find(|node| node.name == "Lattice Readout Stack")
                .expect("the valid fixture must contain a lattice");
            zero_lattice.size = Vec2::ZERO;
            zero_lattice.visible_bounds = None;
            zero_lattice.fully_visible = true;
            let zero_area_issues = zero_area.review_fixture_issues("live-statistics");
            assert!(
                zero_area_issues
                    .iter()
                    .any(|issue| issue.contains("has no layout area")),
                "the fixture oracle must reject a zero-area lattice: {zero_area_issues:?}"
            );

            let mut missing_extreme = snapshot.clone();
            missing_extreme
                .nodes
                .retain(|node| node.name != "Own Cell (2, -2)");
            let missing_extreme_issues = missing_extreme.review_fixture_issues("live-statistics");
            assert!(
                missing_extreme_issues.iter().any(|issue| {
                    issue.contains("missing required surface \"Own Cell (2, -2)\"")
                }),
                "the fixture oracle must reject a clipped or missing lattice edge: {missing_extreme_issues:?}"
            );

            let drawer = snapshot
                .nodes
                .iter()
                .find(|node| node.name == "Combat Lab Live Statistics Drawer")
                .and_then(UiNodeObservation::layout_bounds)
                .expect("the valid fixture must contain statistics");
            let mut reordered = snapshot.clone();
            let lattice = reordered
                .nodes
                .iter_mut()
                .find(|node| node.name == "Lattice Readout Stack")
                .expect("the valid fixture must contain a lattice");
            lattice.center.y = drawer.min.y + lattice.size.y * 0.5 + 1.0;
            let reordered_issues = reordered.review_fixture_issues("live-statistics");
            assert!(
                reordered_issues
                    .iter()
                    .any(|issue| issue.contains("statistics must follow the lattice")),
                "the fixture oracle must reject statistics painted before the lattice: {reordered_issues:?}"
            );

            let mut occluded = snapshot;
            occluded
                .nodes
                .iter_mut()
                .find(|node| node.name == "Combat Lab Live Statistics Drawer")
                .expect("the valid fixture must contain statistics")
                .visible_bounds = Some(drawer);
            let casting = occluded
                .nodes
                .iter_mut()
                .find(|node| node.name == "Casting Panel")
                .expect("the valid fixture must contain casting");
            casting.visible_bounds = Some(drawer);
            let occlusion_issues = occluded.review_fixture_issues("live-statistics");
            assert!(
                occlusion_issues
                    .iter()
                    .any(|issue| issue.contains("cover the primary casting surface")),
                "the fixture oracle must reject an opaque drawer over casting: {occlusion_issues:?}"
            );
        }

        #[test]
        fn gameplay_presentation_states_pass_the_complete_structural_matrix() {
            for (physical_size, scale_factor) in structural_canvases() {
                for mode in all_scale_modes() {
                    let mut collapsed_lattice = None;
                    let mut collapsed_viewport = None;
                    for fixture in [
                        "normal-gameplay",
                        "casting-list",
                        "required-decision",
                        "aiming-disabled",
                        "live-statistics",
                        "dense-report-compare",
                    ] {
                        let snapshot = gameplay_fixture_snapshot(
                            physical_size.x,
                            physical_size.y,
                            scale_factor,
                            mode,
                            fixture,
                        );
                        let issues = snapshot.review_fixture_issues(fixture);
                        assert!(
                            issues.is_empty(),
                            "{fixture} must remain reachable at {physical_size:?} / {scale_factor}× in {mode:?}: {:?}; rail nodes: {:?}",
                            issues,
                            snapshot
                                .nodes
                                .iter()
                                .filter(|node| node.name.contains("Action Rail"))
                                .collect::<Vec<_>>()
                        );
                        let lattice = snapshot
                            .nodes
                            .iter()
                            .find(|node| node.name == "Lattice Readout Stack")
                            .and_then(UiNodeObservation::layout_bounds);
                        if fixture == "casting-list" {
                            collapsed_lattice = lattice;
                            collapsed_viewport = Some(snapshot.metrics.viewport);
                        } else if fixture == "live-statistics" {
                            assert_eq!(
                                collapsed_viewport,
                                Some(snapshot.metrics.viewport),
                                "statistics state must not alter responsive classification"
                            );
                            let collapsed_lattice = collapsed_lattice.unwrap_or_else(|| {
                                panic!(
                                    "collapsed lattice missing at {physical_size:?} / {scale_factor}× in {mode:?}"
                                )
                            });
                            let expanded_lattice = lattice.unwrap_or_else(|| {
                                panic!(
                                    "expanded lattice missing at {physical_size:?} / {scale_factor}× in {mode:?}"
                                )
                            });
                            assert!(
                                (expanded_lattice.min - collapsed_lattice.min)
                                    .abs()
                                    .max_element()
                                    <= 0.5
                                    && (expanded_lattice.max - collapsed_lattice.max)
                                        .abs()
                                        .max_element()
                                        <= 0.5,
                                "statistics expansion moved or resized the lattice at {physical_size:?} / {scale_factor}× in {mode:?}: collapsed={collapsed_lattice:?}, expanded={expanded_lattice:?}"
                            );
                        }
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
                    let snapshot = deployment_snapshot(
                        physical_size.x,
                        physical_size.y,
                        scale_factor,
                        mode,
                        false,
                    );
                    assert!(
                        snapshot.layout_issues().is_empty(),
                        "6v6 deployment must remain reachable at {physical_size:?} / {scale_factor}× in {mode:?}: {:?}; deployment regions: {:?}",
                        snapshot.layout_issues(),
                        snapshot.nodes.iter().filter(|node| node.name.contains("Deployment") || node.name.contains("PLAYER") || node.name.contains("HOSTILE")).collect::<Vec<_>>()
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
                    crate::DefaultImmediateControl,
                    AccessibleLabel::new("Confirm selected lattice cells"),
                    InheritedVisibility::VISIBLE,
                ))
                .id();
            let choose = world
                .spawn((
                    Name::new("Choose Cell"),
                    Button,
                    TabIndex(0),
                    crate::DefaultImmediateControl,
                    AccessibleLabel::new("Choose lattice cell"),
                    InheritedVisibility::VISIBLE,
                ))
                .id();
            world.spawn((
                Name::new("Orphaned Control"),
                Button,
                TabIndex(0),
                crate::DefaultImmediateControl,
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
        fn unclassified_interactive_controls_fail_closed() {
            let mut world = World::new();
            world.spawn((
                Name::new("Unclassified Control"),
                Button,
                TabIndex(0),
                AccessibleLabel::new("Unclassified control"),
                InheritedVisibility::VISIBLE,
            ));
            let snapshot = ui_tree_snapshot(&mut world);
            assert!(snapshot.layout_issues().iter().any(|issue| {
                issue.contains("Unclassified Control")
                    && issue.contains("no explicit immediate/scrollable")
            }));
        }

        #[test]
        fn unnamed_interactive_controls_fail_closed() {
            let mut world = World::new();
            world.spawn((
                Button,
                TabIndex(0),
                AccessibleLabel::new("Generic fallback labels are insufficient"),
                crate::UiVisibilityRequirement::Immediate,
                InheritedVisibility::VISIBLE,
            ));
            let snapshot = ui_tree_snapshot(&mut world);
            assert!(snapshot.layout_issues().iter().any(|issue| {
                issue.contains("unnamed UI entity") && issue.contains("no authored stable Name")
            }));
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
            let control_entity = app
                .world_mut()
                .spawn((
                    Name::new("Clipped Scroll Control"),
                    AccessibleLabel::new("Clipped scroll control"),
                    Button,
                    TabIndex(0),
                    crate::UiVisibilityRequirement::Scrollable,
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
            app.world_mut()
                .entity_mut(scroller)
                .add_child(control_entity);
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

            app.world_mut().get_mut::<Node>(control_entity).unwrap().top = Val::Px(-60.0);
            for _ in 0..2 {
                app.update();
            }
            let snapshot = ui_tree_snapshot(app.world_mut());
            let control = snapshot
                .nodes
                .iter()
                .find(|node| node.name == "Clipped Scroll Control")
                .expect("the negative-offset control remains structurally observable");
            assert!(
                !control.scroll_reachable,
                "a ScrollArea with no attainable negative range cannot reveal an offscreen absolute child"
            );
            assert!(snapshot
                .layout_issues()
                .iter()
                .any(|issue| issue.contains("Clipped Scroll Control is clipped")));
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
