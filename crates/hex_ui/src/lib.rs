//! Runtime presentation for the game.
//!
//! `hex_ui` renders immutable presentation models and emits typed intentions. It does
//! not inspect or mutate combat, unit, lattice, map, world, or perception authority.
//! The application composition crate owns those adapters.

use bevy::prelude::*;

pub use hex_gameplay_model::{HudComponent, MainViewDestination};

mod action_rail;
mod casting_panel;
mod combat_log;
mod creation_presentation;
mod creator;
mod deployment;
#[cfg(feature = "dev-tools")]
mod dev_time;
mod element_visual;
mod focus;
mod gameplay_frame;
mod gameplay_lattices;
mod initiative;
mod lattice;
mod lattice_demo;
mod layout;
mod main_menu;
mod model;
mod outcome;
mod party;
mod review;
mod sandbox;
mod scale;
mod screens;
mod shell;
mod theme;
mod vfx_tuner;

pub use creation_presentation::{effect_summary, CharacterBuildSummary, SpellBuildSummary};
pub use element_visual::{
    ElementClassification, ElementVisual, ElementVisualCatalog, ResolvedElementVisual,
};
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
    ActionAffordance, ActionAvailability, ActionPriority, ActivityIntent, ActivityKind,
    ActivityLogLineView, ActivityLogView, ActivityTab, CampaignPartyMemberView,
    CampaignSlotStatusView, CampaignSlotView, CastingAimView, CastingIntent,
    CastingPanelContentView, CastingPanelView, CastingSpellView, CreatorEffectKind, CreatorIntent,
    CreatorLibraryView, CreatorNameField, CreatorScreenView, CreatorWorkspace, DecisionChoiceView,
    DeploymentIntent, DeploymentQueueEntryView, DeploymentView, FormationSlotView, GameplayAction,
    GameplayChromeView, GameplayHudView, GameplayLatticesView, InitiativeEntryView,
    InitiativeIntent, InitiativeSide, InitiativeView, LatticeDemoIntent, LatticeDemoSpellView,
    LatticeDemoView, LatticeIntent, MainMenuIntent, MainMenuView, OutcomeAction, OutcomeActionView,
    OutcomeIntent, OutcomeView, OwnLatticeView, PartyIntent, PartyMemberView, PartyView, PauseView,
    SandboxCharacterView, SandboxIntent, SandboxLatticeCellKind, SandboxLatticeCellView,
    SandboxMapView, SandboxRosterSlotView, SandboxView, SettingsIntent, SettingsModalView,
    SettingsTab, TargetLatticeStateView, TargetLatticeView, TargetPulseView, UiBindingRow,
    UiIntent, UiSetting, UiSettingRow, UiSettingsView, VfxTunerControl, VfxTunerField,
    VfxTunerIntent, VfxTunerRowView, VfxTunerSpellView, VfxTunerView,
};
#[cfg(feature = "dev-tools")]
pub use model::{DevTimeIntent, DevTimeView};
#[cfg(any(feature = "visual-review", feature = "test-support"))]
pub use scale::ReviewViewport;
pub use scale::{
    resolve_auto_scale, resolve_ui_metrics, resolve_viewport_class, ResolvedUiMetrics, UiScaleMode,
    UiScalePreference, UiViewportClass,
};
pub use shell::{
    despawn_screen, overlay_root, screen_root, screen_root_node, transparent_screen_root,
    DespawnOnExit,
};
pub use theme::{
    blurb, button, display, divider, element_color, fine, heading, label, panel, panel_node,
    row_button, screen_title, small_button, stacked_row_button, OwnColors, UiAssets, ACCENT,
    ACCENT_EDGE, BLURB_SIZE, DANGER, DISPLAY_SIZE, EDGE, FINE_SIZE, FUSION_COLOR, GEM_COLOR, LABEL,
    LABEL_SIZE, MUTED, PANEL_BG, SCREEN_TITLE_SIZE, SMALL_BUTTON_WIDTH, TITLE_SIZE,
};
pub(crate) use theme::{
    body_text_role, brand_logo, compact_glyph_role, fixed_row_button, fluid_button, hud_heading,
    hud_text_role, owner_resolved_control_role, responsive_control_role, supporting_text_role,
};

#[cfg(any(feature = "visual-review", feature = "test-support"))]
pub use review::apply_ui_review_fixture;

/// Installs the shared runtime design system, responsive scale, focus, and intents.
pub struct UiPlugin;

/// Public ordering seam for composition-root intent handlers.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UiSystems {
    /// A blocking input-capture task may consume raw input before focus activation.
    CaptureInput,
    /// Pointer and keyboard interactions have been translated into [`UiIntent`].
    EmitIntents,
    /// Immutable projections have been converted into runtime presentation.
    Render,
}

#[cfg(all(test, feature = "test-support"))]
#[expect(
    clippy::expect_used,
    clippy::panic,
    reason = "structural fixtures fail immediately when checked-in production content or a case registry is invalid"
)]
mod structural_tests {
    use bevy::prelude::*;
    use hex_gameplay_model::{
        CampaignSlotId, MainMenuRoute, SandboxCharacter, SandboxDeploymentSlot,
        SandboxDeploymentStage, SandboxRoute, SandboxSide, SandboxSlotIndex, SandboxStartBlocker,
    };

    use super::*;
    use crate::test_support::{HeadlessUiPlugin, UiTaskCase, UiTreeSnapshot};

    const REQUIRED_MATRIX: [(UVec2, UiScaleMode); 6] = [
        (UVec2::new(1280, 720), UiScaleMode::Auto),
        (UVec2::new(1920, 1080), UiScaleMode::Auto),
        (UVec2::new(3840, 2160), UiScaleMode::Auto),
        (UVec2::new(1280, 720), UiScaleMode::Percent200),
        (UVec2::new(1920, 1080), UiScaleMode::Percent200),
        (UVec2::new(3840, 2160), UiScaleMode::Percent200),
    ];

    #[derive(Resource, Default)]
    struct CreatorIntentLog(Vec<CreatorIntent>);

    #[derive(Resource, Default)]
    struct PointerActivation(Option<Entity>);

    #[derive(Resource, Default)]
    struct KeyboardActivation(Option<Entity>);

    fn apply_pointer_activation(
        mut request: ResMut<PointerActivation>,
        mut interactions: Query<&mut Interaction, With<Button>>,
    ) {
        let Some(entity) = request.0.take() else {
            return;
        };
        *interactions
            .get_mut(entity)
            .expect("the requested pointer target must remain interactive") = Interaction::Pressed;
    }

    fn apply_keyboard_activation(
        mut request: ResMut<KeyboardActivation>,
        mut focus: ResMut<bevy::input_focus::InputFocus>,
        mut keys: ResMut<ButtonInput<KeyCode>>,
    ) {
        let Some(entity) = request.0.take() else {
            return;
        };
        *focus = bevy::input_focus::InputFocus::from_entity(entity);
        keys.press(KeyCode::Enter);
    }

    fn record_creator_intents(
        mut intents: MessageReader<UiIntent>,
        mut log: ResMut<CreatorIntentLog>,
    ) {
        for intent in intents.read() {
            if let UiIntent::Creator(action @ CreatorIntent::ChooseTool(_)) = intent {
                log.0.push(action.clone());
            }
        }
    }

    fn named_entity(world: &mut World, expected: &str) -> Entity {
        let mut named = world.query::<(Entity, &Name)>();
        named
            .iter(world)
            .find_map(|(entity, name)| (name.as_str() == expected).then_some(entity))
            .unwrap_or_else(|| panic!("expected UI entity {expected:?}"))
    }

    fn settled_snapshot(
        screen: hex_core::Screen,
        size: UVec2,
        device_scale: f32,
        ui_scale: UiScaleMode,
        configure: impl FnOnce(&mut World),
    ) -> UiTreeSnapshot {
        let mut app = App::new();
        app.add_plugins(HeadlessUiPlugin::with_scale_factor(
            size.x,
            size.y,
            device_scale,
        ));
        app.world_mut().insert_resource(UiScalePreference(ui_scale));
        configure(app.world_mut());
        app.world_mut()
            .resource_mut::<NextState<hex_core::Screen>>()
            .set(screen);
        for _ in 0..8 {
            app.update();
        }
        test_support::ui_tree_snapshot(app.world_mut())
    }

    fn mixed_campaign_view(route: MainMenuRoute) -> MainMenuView {
        let lattice_cells = |entries: &[(i32, i32, &str, SandboxLatticeCellKind)]| {
            entries
                .iter()
                .map(|&(q, r, label, kind)| SandboxLatticeCellView {
                    q,
                    r,
                    label: label.to_owned(),
                    kind,
                })
                .collect()
        };
        let hedge_mage = lattice_cells(&[
            (0, 0, "LI", SandboxLatticeCellKind::Fusion),
            (1, 0, "LI", SandboxLatticeCellKind::Gem),
            (0, -1, "FI", SandboxLatticeCellKind::Gem),
            (1, -1, "S", SandboxLatticeCellKind::Spell),
            (-1, 0, "FI", SandboxLatticeCellKind::Gem),
            (-1, 1, "FI", SandboxLatticeCellKind::Gem),
            (0, 1, "FI", SandboxLatticeCellKind::Gem),
            (-1, -1, "LI", SandboxLatticeCellKind::Gem),
            (-2, -1, "S", SandboxLatticeCellKind::Spell),
            (1, 1, "S", SandboxLatticeCellKind::Spell),
            (-1, 2, "WA", SandboxLatticeCellKind::Gem),
            (0, 2, "WA", SandboxLatticeCellKind::Gem),
            (-1, 3, "S", SandboxLatticeCellKind::Spell),
        ]);
        let raider = lattice_cells(&[
            (0, 0, "S", SandboxLatticeCellKind::Spell),
            (1, 0, "ME", SandboxLatticeCellKind::Gem),
            (1, -1, "ME", SandboxLatticeCellKind::Gem),
            (0, -1, "EA", SandboxLatticeCellKind::Gem),
            (-1, 0, "EA", SandboxLatticeCellKind::Gem),
            (-1, 1, "EA", SandboxLatticeCellKind::Gem),
            (0, 1, "EA", SandboxLatticeCellKind::Gem),
        ]);
        let wolf = lattice_cells(&[
            (0, 0, "EA", SandboxLatticeCellKind::Gem),
            (1, 0, "EA", SandboxLatticeCellKind::Gem),
            (0, 1, "EA", SandboxLatticeCellKind::Gem),
            (1, -1, "EA", SandboxLatticeCellKind::Gem),
        ]);
        MainMenuView {
            route,
            setup_failure: None,
            campaign_slots: vec![
                CampaignSlotView {
                    slot: CampaignSlotId::One,
                    status: CampaignSlotStatusView::Empty,
                },
                CampaignSlotView {
                    slot: CampaignSlotId::Two,
                    status: CampaignSlotStatusView::Available {
                        party: vec![
                            CampaignPartyMemberView {
                                name: "Hedge Mage".to_owned(),
                                lattice: "20 mana".to_owned(),
                                cells: hedge_mage,
                            },
                            CampaignPartyMemberView {
                                name: "Raider".to_owned(),
                                lattice: "16 mana".to_owned(),
                                cells: raider,
                            },
                            CampaignPartyMemberView {
                                name: "Wolf".to_owned(),
                                lattice: "4 mana".to_owned(),
                                cells: wolf,
                            },
                        ],
                        active_time: "12h 34m".to_owned(),
                    },
                },
                CampaignSlotView {
                    slot: CampaignSlotId::Three,
                    status: CampaignSlotStatusView::Invalid {
                        reason: "This campaign uses an incompatible content revision.".to_owned(),
                    },
                },
            ],
        }
    }

    fn main_menu_snapshot(route: MainMenuRoute, size: UVec2, mode: UiScaleMode) -> UiTreeSnapshot {
        settled_snapshot(hex_core::Screen::Title, size, 1.0, mode, |world| {
            world.insert_resource(mixed_campaign_view(route));
        })
    }

    fn authored_map() -> SandboxMapView {
        SandboxMapView {
            id: "flat-arena".to_owned(),
            name: "Flat Arena".to_owned(),
            description: "A level tactical arena for exact roster experiments.".to_owned(),
            preview: "ui/sandbox/flat-arena.png".to_owned(),
            resolved_seed: None,
            can_regenerate: false,
        }
    }

    fn generated_map() -> SandboxMapView {
        SandboxMapView {
            id: "procedural-hills".to_owned(),
            name: "Procedural Hills".to_owned(),
            description: "Temperate hills split by a river and two tactical crossings.".to_owned(),
            preview: "ui/sandbox/procedural-hills.png".to_owned(),
            resolved_seed: Some(1_592_598_566),
            can_regenerate: true,
        }
    }

    fn shipped_map_catalog() -> Vec<SandboxMapView> {
        let catalog: hex_assets::SandboxMapCatalog =
            ron::from_str(include_str!("../../../assets/config/sandbox_maps.ron"))
                .expect("the production Sandbox map catalog must parse");
        catalog
            .maps
            .into_iter()
            .map(|definition| SandboxMapView {
                id: definition.id,
                name: definition.display_name,
                description: definition.description,
                preview: definition.preview,
                resolved_seed: definition.fixed_seed,
                can_regenerate: definition.fixed_seed.is_some(),
            })
            .collect()
    }

    fn sample_character() -> SandboxCharacterView {
        SandboxCharacterView {
            character: SandboxCharacter::Template("hedge-mage".to_owned()),
            name: "Hedge Mage".to_owned(),
            lattice: "Air · Fire · Lightning Bolt".to_owned(),
            cells: vec![
                SandboxLatticeCellView {
                    q: 0,
                    r: 0,
                    label: "AIR".to_owned(),
                    kind: SandboxLatticeCellKind::Gem,
                },
                SandboxLatticeCellView {
                    q: 1,
                    r: 0,
                    label: "FIRE".to_owned(),
                    kind: SandboxLatticeCellKind::Gem,
                },
                SandboxLatticeCellView {
                    q: 0,
                    r: 1,
                    label: "SPELL".to_owned(),
                    kind: SandboxLatticeCellKind::Spell,
                },
            ],
            blocked: None,
            selected: true,
        }
    }

    fn sample_raider() -> SandboxCharacterView {
        let mut character = sample_character();
        character.character = SandboxCharacter::Template("raider".to_owned());
        character.name = "Raider".to_owned();
        character.lattice = "Fire · Metal · Searing Strike".to_owned();
        character.selected = false;
        character
    }

    fn sandbox_view(route: SandboxRoute) -> SandboxView {
        let mut view = SandboxView::default();
        let character = sample_character();
        view.active = true;
        view.route = route;
        view.map = Some(authored_map());
        view.maps = shipped_map_catalog();
        view.characters = vec![character.clone()];
        view.preview = Some(character.clone());
        view.start_blocker = None;
        if let Some(slot) = view.party.first_mut() {
            slot.character = Some(character.clone());
        }
        if let Some(slot) = view.enemies.first_mut() {
            slot.character = Some(character);
        }
        if route == SandboxRoute::MapDetail {
            view.pending_map = Some(generated_map());
        }
        view
    }

    fn sandbox_roster_view(side: SandboxSide, dense_duplicates: bool) -> SandboxView {
        let mut view = sandbox_view(SandboxRoute::Roster(side));
        if dense_duplicates {
            let characters = [sample_character(), sample_raider()];
            let slots = match side {
                SandboxSide::Party => &mut view.party,
                SandboxSide::Enemies => &mut view.enemies,
            };
            for (slot, character) in slots.iter_mut().zip(characters.iter().cycle()) {
                slot.character = Some(character.clone());
            }
        }
        view
    }

    fn sandbox_view_snapshot(view: SandboxView, size: UVec2, mode: UiScaleMode) -> UiTreeSnapshot {
        settled_snapshot(hex_core::Screen::Sandbox, size, 1.0, mode, |world| {
            world.insert_resource(view);
        })
    }

    fn sandbox_snapshot(route: SandboxRoute, size: UVec2, mode: UiScaleMode) -> UiTreeSnapshot {
        sandbox_view_snapshot(sandbox_view(route), size, mode)
    }

    fn settings_snapshot(size: UVec2, mode: UiScaleMode) -> UiTreeSnapshot {
        use UiSetting::{
            EffectsVolume, Fullscreen, MasterVolume, MusicVolume, Presentation, UiScale, UiVolume,
            WindowSize,
        };
        settled_snapshot(hex_core::Screen::Settings, size, 1.0, mode, |world| {
            world.insert_resource(UiSettingsView {
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
                .map(|(setting, label, value)| UiSettingRow {
                    setting,
                    label: label.to_owned(),
                    value: value.to_owned(),
                })
                .collect(),
                notice: Some("Settings save immediately.".to_owned()),
                ..default()
            });
        })
    }

    fn settings_binding_view(tab: SettingsTab) -> UiSettingsView {
        let category = tab
            .input_category()
            .expect("binding fixture requires a keybinding tab");
        let bindings = hex_core::InputActionInventory::active()
            .iter()
            .filter(|action| *action != hex_core::InputAction::RevealAll)
            .filter(|action| action.metadata().category == category)
            .map(|action| {
                let metadata = action.metadata();
                UiBindingRow {
                    action,
                    label: metadata.label.to_owned(),
                    chord: metadata.default_chord.label(),
                    rebindable: metadata.rebindable,
                    overridden: action == hex_core::InputAction::EndTurn,
                }
            })
            .collect();
        UiSettingsView {
            tab,
            bindings,
            can_restore_all: true,
            ..default()
        }
    }

    fn settings_binding_snapshot(
        tab: SettingsTab,
        size: UVec2,
        mode: UiScaleMode,
    ) -> UiTreeSnapshot {
        settled_snapshot(hex_core::Screen::Settings, size, 1.0, mode, |world| {
            world.insert_resource(settings_binding_view(tab));
        })
    }

    fn settings_modal_snapshot(
        modal: SettingsModalView,
        size: UVec2,
        mode: UiScaleMode,
    ) -> UiTreeSnapshot {
        settled_snapshot(hex_core::Screen::Settings, size, 1.0, mode, |world| {
            let mut view = settings_binding_view(SettingsTab::Gameplay);
            view.modal = Some(modal);
            world.insert_resource(view);
        })
    }

    fn creator_view(case: UiTaskCase) -> (hex_core::Screen, CreatorScreenView) {
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
        let mut view = CreatorScreenView {
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
                view.workspace = CreatorWorkspace::Hub;
            }
            UiTaskCase::SpellLibrary => {
                view.tab = hex_gameplay_model::CreatorSurface::Spells;
                view.workspace = CreatorWorkspace::Hub;
            }
            UiTaskCase::CreatorLibraryRecovery => {
                view.tab = hex_gameplay_model::CreatorSurface::Characters;
                view.workspace = CreatorWorkspace::Hub;
                view.library.error = Some(
                    "The local Creator library could not be decoded. Reset it to continue."
                        .to_owned(),
                );
                view.confirm_reset = true;
                view.notice = "Press Confirm Reset to replace the unreadable library.".to_owned();
            }
            UiTaskCase::CharacterInvalid => {
                view.tab = hex_gameplay_model::CreatorSurface::Characters;
                view.workspace = CreatorWorkspace::Character;
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
            UiTaskCase::CharacterReady | UiTaskCase::CharacterConfirmDelete => {
                view.tab = hex_gameplay_model::CreatorSurface::Characters;
                view.workspace = CreatorWorkspace::Character;
                view.character = presets
                    .characters
                    .iter()
                    .find(|record| record.audience == hex_assets::PresetAudience::HumanTemplate)
                    .map(|record| record.character.clone());
                view.active_tool = Some(hex_assets::CreationCellKind::Gem("Air".to_owned()));
                if case == UiTaskCase::CharacterConfirmDelete {
                    view.confirm_delete = true;
                    view.notice = "Press Confirm Delete to remove this saved character.".to_owned();
                }
            }
            UiTaskCase::SpellInvalid => {
                view.tab = hex_gameplay_model::CreatorSurface::Spells;
                view.workspace = CreatorWorkspace::Spell;
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
            UiTaskCase::SpellReady | UiTaskCase::SpellConfirmDelete => {
                view.tab = hex_gameplay_model::CreatorSurface::Spells;
                view.workspace = CreatorWorkspace::Spell;
                view.spell = presets
                    .spells
                    .iter()
                    .find(|record| record.audience == hex_assets::PresetAudience::HumanTemplate)
                    .map(|record| record.spell.clone());
                if case == UiTaskCase::SpellConfirmDelete {
                    view.confirm_delete = true;
                    view.notice = "Press Confirm Delete to remove this saved spell.".to_owned();
                }
            }
            other => panic!("{other:?} is not a Creator task"),
        }
        (screen, view)
    }

    fn creator_snapshot(case: UiTaskCase, size: UVec2, mode: UiScaleMode) -> UiTreeSnapshot {
        let (screen, view) = creator_view(case);
        settled_snapshot(screen, size, 1.0, mode, |world| {
            world.insert_resource(view);
        })
    }

    fn lattice_demo_snapshot(size: UVec2, mode: UiScaleMode) -> UiTreeSnapshot {
        settled_snapshot(hex_core::Screen::LatticeDemo, size, 1.0, mode, |world| {
            world.insert_resource(LatticeDemoView {
                ready: true,
                cells: [
                    (0, 0, "AIR"),
                    (1, 0, "FIRE"),
                    (0, 1, "WATER"),
                    (-1, 1, "EARTH"),
                ]
                .into_iter()
                .map(|(q, r, label)| LatticeCellView {
                    coord: hex_core::LatticeCoord::new(q, r),
                    label: label.to_owned(),
                    detail: "LIVE · 1 MANA".to_owned(),
                    color: Color::srgb(0.35, 0.62, 0.78),
                    known_mana: Some(1),
                    known_locked: Some(false),
                    disabled: false,
                    selected: false,
                    interaction: CellInteraction::Actionable,
                })
                .collect(),
                spells: vec![LatticeDemoSpellView {
                    coord: hex_core::LatticeCoord::new(0, 1),
                    name: "Lightning Bolt".to_owned(),
                    headline: "Lightning Bolt · ready".to_owned(),
                    kind: "Evocation".to_owned(),
                    cost: Some(2),
                    blocked: None,
                }],
                totals: "Mana 4 · disabled 0 · enchantments 0".to_owned(),
                log: (1..=8)
                    .map(|index| format!("Bounded lattice event {index}"))
                    .collect(),
            });
        })
    }

    fn sandbox_outcome_snapshot(size: UVec2, mode: UiScaleMode) -> UiTreeSnapshot {
        settled_snapshot(hex_core::Screen::Gameplay, size, 1.0, mode, |world| {
            world.insert_resource(GameplayChromeView {
                encounter_complete: true,
                ..default()
            });
            world.insert_resource(OutcomeView {
                visible: true,
                title: "VICTORY".to_owned(),
                detail: "The Enemy roster can no longer continue.".to_owned(),
                actions: vec![
                    OutcomeActionView {
                        action: OutcomeAction::RetryExact,
                        label: "Retry Exact".to_owned(),
                    },
                    OutcomeActionView {
                        action: OutcomeAction::Return,
                        label: "Return to Sandbox".to_owned(),
                    },
                ],
            });
        })
    }

    fn deployment_snapshot(size: UVec2, mode: UiScaleMode, complete: bool) -> UiTreeSnapshot {
        settled_snapshot(hex_core::Screen::Gameplay, size, 1.0, mode, |world| {
            let active = SandboxDeploymentSlot::new(SandboxSide::Party, SandboxSlotIndex::One);
            let queue = SandboxSide::ALL
                .into_iter()
                .flat_map(|side| {
                    SandboxSlotIndex::ALL
                        .into_iter()
                        .map(move |slot| DeploymentQueueEntryView {
                            slot: SandboxDeploymentSlot::new(side, slot),
                            name: format!("{side} Character {slot}"),
                            selected: !complete
                                && side == SandboxSide::Party
                                && slot == SandboxSlotIndex::One,
                            placed: complete,
                            selectable: complete
                                || (side == SandboxSide::Party && slot == SandboxSlotIndex::One),
                        })
                })
                .collect::<Vec<_>>();
            world.insert_resource(DeploymentView {
                active: true,
                map_name: "Flat Arena".to_owned(),
                notice: "Click any valid map surface.".to_owned(),
                stage: Some(if complete {
                    SandboxDeploymentStage::Review
                } else {
                    SandboxDeploymentStage::Placing(active)
                }),
                queue,
                can_undo: true,
                complete,
            });
            world.insert_resource(GameplayHudView {
                phase: hex_core::GameplayPhase::Deployment,
                actor_label: "Deployment".to_owned(),
                round: "Setup".to_owned(),
                ..default()
            });
        })
    }

    fn gameplay_party_view() -> PartyView {
        let silhouette = vec![
            SandboxLatticeCellView {
                q: 0,
                r: 0,
                label: "A".to_owned(),
                kind: SandboxLatticeCellKind::Gem,
            },
            SandboxLatticeCellView {
                q: 1,
                r: 0,
                label: "F".to_owned(),
                kind: SandboxLatticeCellKind::Fusion,
            },
            SandboxLatticeCellView {
                q: 0,
                r: 1,
                label: "S".to_owned(),
                kind: SandboxLatticeCellKind::Spell,
            },
        ];
        PartyView {
            members: (0..6)
                .map(|slot| PartyMemberView {
                    slot,
                    label: format!(
                        "{} · {}",
                        if slot == 0 { "Hedge Mage" } else { "Ally" },
                        if slot == 0 { "selected" } else { "ready" }
                    ),
                    cells: silhouette.clone(),
                    active: slot == 0,
                    selected: slot == 0,
                })
                .collect(),
            formation_visible: true,
            movement_mode: "GROUP · formation follows the selected anchor".to_owned(),
            presets: vec!["Column".to_owned(), "Wedge".to_owned()],
            slots: vec![FormationSlotView {
                offset: hex_core::HexCoord::from_axial(0, 0),
                anchor: true,
            }],
        }
    }

    fn gameplay_initiative_view(hostile_turn: bool) -> InitiativeView {
        InitiativeView {
            heading: if hostile_turn {
                "enemy turn"
            } else {
                "your turn"
            }
            .to_owned(),
            entries: vec![
                InitiativeEntryView {
                    unit: hex_core::UnitId(0),
                    name: "Hedge Mage".to_owned(),
                    side: InitiativeSide::Ally,
                    current: !hostile_turn,
                    inspectable: true,
                },
                InitiativeEntryView {
                    unit: hex_core::UnitId(1),
                    name: "Observed Raider".to_owned(),
                    side: InitiativeSide::Hostile,
                    current: hostile_turn,
                    inspectable: true,
                },
                InitiativeEntryView {
                    unit: hex_core::UnitId(2),
                    name: "Unavailable hostile".to_owned(),
                    side: InitiativeSide::Hostile,
                    current: false,
                    inspectable: false,
                },
            ],
        }
    }

    fn gameplay_activity_view() -> ActivityLogView {
        ActivityLogView {
            heading: "ACTIVITY · L".to_owned(),
            tab: ActivityTab::All,
            lines: vec![
                ActivityLogLineView {
                    kind: ActivityKind::Combat,
                    text: "Hedge Mage cast Lightning Bolt".to_owned(),
                    danger: false,
                },
                ActivityLogLineView {
                    kind: ActivityKind::Activity,
                    text: "Party formation changed to Wedge".to_owned(),
                    danger: false,
                },
            ],
        }
    }

    fn gameplay_snapshot(case: UiTaskCase, size: UVec2, mode: UiScaleMode) -> UiTreeSnapshot {
        let fixture = match case {
            UiTaskCase::Exploration => "normal-gameplay",
            UiTaskCase::PlayerTurnMaxActions => "player-turn-max",
            UiTaskCase::HostileTurn => "hostile-turn",
            UiTaskCase::CharacterMainView
            | UiTaskCase::ActivityTabs
            | UiTaskCase::CustomHudVisibility
            | UiTaskCase::CompactTemporarySurface => "normal-gameplay",
            UiTaskCase::FormationMainView => "clear",
            UiTaskCase::Casting => "casting-list",
            UiTaskCase::AimingBlocked => "aiming-disabled",
            UiTaskCase::DisableDecision | UiTaskCase::HudHiddenRequired => "required-decision",
            UiTaskCase::RestoreDecision => "restore-decision",
            UiTaskCase::Pause => "player-turn-max",
            other => panic!("{other:?} is not a live gameplay task"),
        };
        let mut app = App::new();
        app.add_plugins(HeadlessUiPlugin::new(size.x, size.y));
        app.world_mut().insert_resource(UiScalePreference(mode));
        let compact = resolve_ui_metrics(size.as_vec2(), mode).viewport == UiViewportClass::Compact;
        let mut chrome = GameplayChromeView {
            party_shown: false,
            initiative_shown: false,
            activity_shown: false,
            action_bar_shown: false,
            main_view: hex_gameplay_model::MainViewDestination::Closed,
            terrain_health_shown: true,
            encounter_complete: false,
        };
        match case {
            UiTaskCase::Exploration => {
                if compact {
                    chrome.action_bar_shown = true;
                } else {
                    chrome.party_shown = true;
                    chrome.action_bar_shown = true;
                }
            }
            UiTaskCase::PlayerTurnMaxActions => {
                chrome.party_shown = !compact;
                chrome.action_bar_shown = true;
                chrome.initiative_shown = !compact;
            }
            UiTaskCase::Casting | UiTaskCase::AimingBlocked => {
                chrome.action_bar_shown = true;
                chrome.initiative_shown = !compact;
            }
            UiTaskCase::HostileTurn => {
                chrome.party_shown = !compact;
                chrome.initiative_shown = !compact;
            }
            UiTaskCase::CharacterMainView => {
                chrome.main_view =
                    hex_gameplay_model::MainViewDestination::Character(hex_core::UnitId(0));
                if !compact {
                    chrome.party_shown = true;
                    chrome.action_bar_shown = true;
                }
            }
            UiTaskCase::FormationMainView => {
                chrome.main_view = hex_gameplay_model::MainViewDestination::Formation;
                if !compact {
                    chrome.party_shown = true;
                    chrome.action_bar_shown = true;
                }
            }
            UiTaskCase::ActivityTabs => chrome.activity_shown = true,
            UiTaskCase::CustomHudVisibility => {
                chrome.party_shown = true;
                chrome.activity_shown = !compact;
            }
            UiTaskCase::CompactTemporarySurface => chrome.party_shown = true,
            UiTaskCase::DisableDecision | UiTaskCase::RestoreDecision => {
                chrome.main_view = hex_gameplay_model::MainViewDestination::RequiredDecision;
                chrome.party_shown = !compact;
                chrome.initiative_shown = !compact;
            }
            UiTaskCase::HudHiddenRequired => {
                chrome.main_view = hex_gameplay_model::MainViewDestination::RequiredDecision;
                chrome.terrain_health_shown = false;
            }
            UiTaskCase::Pause => chrome.terrain_health_shown = false,
            other => panic!("{other:?} is not a live gameplay task"),
        }
        app.world_mut().insert_resource(chrome);
        if chrome.party_shown || case == UiTaskCase::FormationMainView {
            app.world_mut().insert_resource(gameplay_party_view());
        }
        if chrome.initiative_shown {
            app.world_mut()
                .insert_resource(gameplay_initiative_view(case == UiTaskCase::HostileTurn));
        }
        if chrome.activity_shown {
            app.world_mut().insert_resource(gameplay_activity_view());
        }
        if case == UiTaskCase::Pause {
            app.world_mut().insert_resource(PauseView {
                hint: "Esc to resume".to_owned(),
                notice: Some("Campaign save is current.".to_owned()),
            });
        }
        let mut queue = bevy::ecs::world::CommandQueue::default();
        let mut commands = Commands::new(&mut queue, app.world());
        apply_ui_review_fixture(&mut commands, fixture)
            .expect("every gameplay task owns an authored review fixture");
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
        test_support::ui_tree_snapshot(app.world_mut())
    }

    #[test]
    fn task_registry_contains_only_current_player_routes() {
        let ids = UiTaskCase::ALL
            .into_iter()
            .map(|case| case.contract().id)
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(ids.len(), UiTaskCase::ALL.len());
        assert_eq!(
            UiTaskCase::ALL
                .into_iter()
                .filter(|case| case.contract().screen == hex_core::Screen::Title)
                .count(),
            3
        );
        assert_eq!(
            UiTaskCase::ALL
                .into_iter()
                .filter(|case| case.contract().screen == hex_core::Screen::Sandbox)
                .count(),
            6
        );
        assert_eq!(
            UiTaskCase::ALL
                .into_iter()
                .filter(|case| case.contract().screen == hex_core::Screen::Settings)
                .count(),
            4
        );
    }

    #[test]
    fn main_menu_campaign_and_tools_pass_the_required_matrix() {
        let campaign = mixed_campaign_view(MainMenuRoute::Campaign);
        assert!(campaign.campaign_slots.iter().any(|slot| {
            slot.slot == CampaignSlotId::One && matches!(slot.status, CampaignSlotStatusView::Empty)
        }));
        assert!(campaign.campaign_slots.iter().any(|slot| {
            slot.slot == CampaignSlotId::Two
                && matches!(slot.status, CampaignSlotStatusView::Available { .. })
        }));
        assert!(campaign.campaign_slots.iter().any(|slot| {
            slot.slot == CampaignSlotId::Three
                && matches!(slot.status, CampaignSlotStatusView::Invalid { .. })
        }));

        let mut failures = Vec::new();
        for (size, mode) in REQUIRED_MATRIX {
            for (case, route) in [
                (UiTaskCase::MainMenu, MainMenuRoute::Root),
                (UiTaskCase::Campaign, MainMenuRoute::Campaign),
                (UiTaskCase::Tools, MainMenuRoute::Tools),
            ] {
                let snapshot = main_menu_snapshot(route, size, mode);
                let issues = snapshot.task_issues(case);
                if !issues.is_empty() {
                    failures.push(format!(
                        "{} failed at {size:?} {mode:?}: {issues:#?}",
                        case.contract().id,
                    ));
                }
                if case == UiTaskCase::MainMenu {
                    assert_eq!(
                        snapshot.focus_order,
                        ["Campaign", "Sandbox", "Tools", "Settings"]
                    );
                }
                if case == UiTaskCase::Campaign {
                    for slot in 1..=3 {
                        assert!(snapshot
                            .nodes
                            .iter()
                            .any(|node| node.name == format!("Campaign Slot {slot}")));
                    }
                    for action in ["New Game Save Slot 1", "Continue Save Slot 2"] {
                        let controls = snapshot
                            .nodes
                            .iter()
                            .filter(|node| node.name == action)
                            .collect::<Vec<_>>();
                        assert_eq!(
                            controls.len(),
                            1,
                            "mixed Campaign should render exactly one {action:?} at {size:?} {mode:?}"
                        );
                        let control = controls
                            .first()
                            .expect("the exact control count was checked above");
                        assert!(
                            control.scroll_reachable,
                            "{action:?} should remain reachable at {size:?} {mode:?}"
                        );
                        assert!(
                            snapshot.focus_order.iter().any(|name| name == action),
                            "{action:?} should remain keyboard reachable at {size:?} {mode:?}"
                        );
                    }
                    assert_eq!(
                        snapshot
                            .nodes
                            .iter()
                            .filter(|node| node.name == "Character Lattice Preview")
                            .count(),
                        3,
                        "each occupied-party member should reuse the lattice presentation"
                    );
                }
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }

    #[test]
    fn campaign_duplicate_slot_states_keep_unique_control_and_accessible_names() {
        for (action, status) in [
            ("New Game", CampaignSlotStatusView::Empty),
            (
                "Continue",
                CampaignSlotStatusView::Available {
                    party: Vec::new(),
                    active_time: "0m".to_owned(),
                },
            ),
        ] {
            let snapshot = settled_snapshot(
                hex_core::Screen::Title,
                UVec2::new(1920, 1080),
                1.0,
                UiScaleMode::Auto,
                |world| {
                    world.insert_resource(MainMenuView {
                        route: MainMenuRoute::Campaign,
                        setup_failure: None,
                        campaign_slots: CampaignSlotId::ALL
                            .into_iter()
                            .map(|slot| CampaignSlotView {
                                slot,
                                status: status.clone(),
                            })
                            .collect(),
                    });
                },
            );

            let expected = CampaignSlotId::ALL.map(|slot| {
                (
                    format!("{action} Save Slot {}", slot.number()),
                    format!("{action}, Save Slot {}", slot.number()),
                )
            });
            for (name, accessible_label) in &expected {
                let matching = snapshot
                    .nodes
                    .iter()
                    .filter(|node| node.name == *name)
                    .collect::<Vec<_>>();
                assert_eq!(matching.len(), 1, "missing unique Campaign action {name:?}");
                assert_eq!(
                    matching
                        .first()
                        .expect("the exact Campaign action count was checked")
                        .accessible_label
                        .as_deref(),
                    Some(accessible_label.as_str())
                );
                assert!(
                    snapshot.focus_order.contains(name),
                    "{name:?} must remain in keyboard order"
                );
            }
            assert_eq!(
                snapshot
                    .nodes
                    .iter()
                    .filter(|node| node.name.starts_with(action) && node.focusable)
                    .count(),
                3,
                "all duplicate {action:?} cards must keep distinct controls"
            );
        }
    }

    #[test]
    fn campaign_setup_failure_is_visible_without_breaking_the_matrix() {
        for (size, mode) in REQUIRED_MATRIX {
            let snapshot = settled_snapshot(hex_core::Screen::Title, size, 1.0, mode, |world| {
                let mut view = mixed_campaign_view(MainMenuRoute::Campaign);
                view.setup_failure = Some("Campaign content is still loading.".to_owned());
                world.insert_resource(view);
            });
            assert!(snapshot
                .nodes
                .iter()
                .any(|node| node.name == "Campaign Setup Failure" && node.visible));
            assert!(
                snapshot.task_issues(UiTaskCase::Campaign).is_empty(),
                "Campaign failure view should remain structurally valid at {size:?} {mode:?}"
            );
        }
    }

    #[test]
    fn sandbox_overview_and_character_picker_pass_the_required_matrix() {
        let cases = [
            (UiTaskCase::SandboxOverview, SandboxRoute::Overview),
            (
                UiTaskCase::SandboxCharacterPicker,
                SandboxRoute::CharacterPicker {
                    side: SandboxSide::Party,
                    slot: SandboxSlotIndex::One,
                },
            ),
        ];
        let mut failures = Vec::new();
        for (size, mode) in REQUIRED_MATRIX {
            for (case, route) in cases {
                let snapshot = sandbox_snapshot(route, size, mode);
                let issues = snapshot.task_issues(case);
                if !issues.is_empty() {
                    failures.push(format!(
                        "{} failed at {size:?} {mode:?}: {issues:#?}",
                        case.contract().id
                    ));
                }
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }

    #[test]
    fn full_sandbox_catalog_has_one_reachable_scroll_route_across_the_required_matrix() {
        for (size, mode) in REQUIRED_MATRIX {
            let view = sandbox_view(SandboxRoute::MapBrowser);
            assert_eq!(
                view.maps.len(),
                17,
                "the full shipped catalog is the fixture"
            );
            assert_eq!(
                view.maps.last().map(|map| map.id.as_str()),
                Some("mountain-range")
            );

            let snapshot = sandbox_view_snapshot(view, size, mode);
            let issues = snapshot.task_issues(UiTaskCase::SandboxMapBrowser);
            assert!(
                issues.is_empty(),
                "full Sandbox catalog failed at {size:?} {mode:?}: {issues:#?}"
            );
            assert_eq!(
                snapshot
                    .nodes
                    .iter()
                    .filter(|node| node.name.starts_with("Inspect "))
                    .count(),
                17
            );
            let final_row = snapshot
                .nodes
                .iter()
                .find(|node| node.name == "Inspect Mountain Range")
                .expect("the final shipped map row should render");
            assert_eq!(
                final_row.visibility_requirement,
                Some(UiVisibilityRequirement::Scrollable)
            );
            assert!(
                final_row.scroll_reachable,
                "the final catalog row should be reachable at {size:?} {mode:?}"
            );
        }
    }

    #[test]
    fn generated_and_authored_map_details_pass_the_required_matrix() {
        for (size, mode) in REQUIRED_MATRIX {
            for (variant, pending_map, can_regenerate) in [
                ("generated", generated_map(), true),
                ("authored", authored_map(), false),
            ] {
                let preview_name = format!("{} Large Preview", pending_map.name);
                let mut view = sandbox_view(SandboxRoute::MapDetail);
                view.pending_map = Some(pending_map);
                let snapshot = sandbox_view_snapshot(view, size, mode);
                let issues = snapshot.task_issues(UiTaskCase::SandboxMapDetail);
                assert!(
                    issues.is_empty(),
                    "{variant} map detail failed at {size:?} {mode:?}: {issues:#?}"
                );
                assert!(snapshot.nodes.iter().any(|node| node.name == preview_name));
                assert_eq!(
                    snapshot
                        .nodes
                        .iter()
                        .any(|node| node.name == "Regenerate Seed"),
                    can_regenerate,
                    "only generated maps should expose regeneration"
                );
            }
        }
    }

    #[test]
    fn sparse_and_dense_duplicate_rosters_pass_both_side_routes() {
        for (size, mode) in REQUIRED_MATRIX {
            for side in [SandboxSide::Party, SandboxSide::Enemies] {
                let case = match side {
                    SandboxSide::Party => UiTaskCase::SandboxParty,
                    SandboxSide::Enemies => UiTaskCase::SandboxEnemies,
                };
                for dense_duplicates in [false, true] {
                    let view = sandbox_roster_view(side, dense_duplicates);
                    let slots = match side {
                        SandboxSide::Party => &view.party,
                        SandboxSide::Enemies => &view.enemies,
                    };
                    let occupied = slots
                        .iter()
                        .filter_map(|slot| slot.character.as_ref())
                        .collect::<Vec<_>>();
                    assert_eq!(occupied.len(), if dense_duplicates { 6 } else { 1 });
                    if dense_duplicates {
                        assert_eq!(
                            occupied
                                .iter()
                                .filter(|character| matches!(
                                    &character.character,
                                    SandboxCharacter::Template(id) if id == "hedge-mage"
                                ))
                                .count(),
                            3
                        );
                        assert_eq!(
                            occupied
                                .iter()
                                .filter(|character| matches!(
                                    &character.character,
                                    SandboxCharacter::Template(id) if id == "raider"
                                ))
                                .count(),
                            3
                        );
                    }

                    let snapshot = sandbox_view_snapshot(view, size, mode);
                    let issues = snapshot.task_issues(case);
                    assert!(
                        issues.is_empty(),
                        "{side} {} roster failed at {size:?} {mode:?}: {issues:#?}",
                        if dense_duplicates {
                            "dense duplicate"
                        } else {
                            "sparse"
                        }
                    );
                    for slot in SandboxSlotIndex::ALL {
                        assert!(snapshot
                            .nodes
                            .iter()
                            .any(|node| { node.name == format!("{side} slot {}", slot.number()) }));
                    }
                }
            }
        }
    }

    #[test]
    fn settings_preserve_one_reachable_scroll_owner_across_the_required_matrix() {
        for (size, mode) in REQUIRED_MATRIX {
            let snapshot = settings_snapshot(size, mode);
            let issues = snapshot.task_issues(UiTaskCase::Settings);
            assert!(
                issues.is_empty(),
                "settings failed at {size:?} {mode:?}: {issues:#?}"
            );
        }
    }

    #[test]
    fn settings_binding_tabs_preserve_reachable_controls_across_the_required_matrix() {
        for (size, mode) in REQUIRED_MATRIX {
            for tab in SettingsTab::ALL
                .into_iter()
                .filter(|tab| *tab != SettingsTab::General)
            {
                let snapshot = settings_binding_snapshot(tab, size, mode);
                let issues = snapshot.task_issues(UiTaskCase::SettingsKeybindings);
                assert!(
                    issues.is_empty(),
                    "{} bindings failed at {size:?} {mode:?}: {issues:#?}",
                    tab.label()
                );

                let last_action = hex_core::InputActionInventory::active()
                    .iter()
                    .filter(|action| *action != hex_core::InputAction::RevealAll)
                    .filter(|action| {
                        action.metadata().category == tab.input_category().expect("binding tab")
                            && action.metadata().rebindable
                    })
                    .last()
                    .expect("every binding category has a rebindable action");
                let last_control = format!("Rebind {}", last_action.metadata().label);
                let observation = snapshot
                    .nodes
                    .iter()
                    .find(|node| node.name == last_control)
                    .expect("the final category binding renders");
                assert!(
                    observation.scroll_reachable,
                    "the final {} binding should remain reachable at {size:?} {mode:?}",
                    tab.label()
                );
                assert!(
                    snapshot
                        .nodes
                        .iter()
                        .all(|node| !node.name.contains("Reveal Knowledge (Development)")),
                    "shipping Settings fixture exposed the development-only action"
                );
            }
        }
    }

    #[test]
    fn settings_blocking_tasks_pass_the_required_matrix() {
        for (size, mode) in REQUIRED_MATRIX {
            let capture = settings_modal_snapshot(
                SettingsModalView::Capture {
                    action: hex_core::InputAction::EndTurn,
                    label: "End Turn".to_owned(),
                },
                size,
                mode,
            );
            let capture_issues = capture.task_issues(UiTaskCase::SettingsCapture);
            assert!(
                capture_issues.is_empty(),
                "Settings capture failed at {size:?} {mode:?}: {capture_issues:#?}"
            );

            for (fixture, modal) in [
                (
                    "binding conflict",
                    SettingsModalView::Conflict {
                        requested: "End Turn".to_owned(),
                        existing: "Confirm Decision".to_owned(),
                        chord: "Space".to_owned(),
                    },
                ),
                (
                    "Restore All confirmation",
                    SettingsModalView::ConfirmRestoreAll,
                ),
            ] {
                let conflict = settings_modal_snapshot(modal, size, mode);
                let conflict_issues = conflict.task_issues(UiTaskCase::SettingsConflict);
                assert!(
                    conflict_issues.is_empty(),
                    "Settings {fixture} failed at {size:?} {mode:?}: {conflict_issues:#?}"
                );
            }
        }
    }

    #[test]
    fn creator_routes_preserve_their_structural_matrix() {
        let cases = [
            UiTaskCase::CharacterLibrary,
            UiTaskCase::SpellLibrary,
            UiTaskCase::CreatorLibraryRecovery,
            UiTaskCase::CharacterInvalid,
            UiTaskCase::CharacterReady,
            UiTaskCase::CharacterConfirmDelete,
            UiTaskCase::SpellInvalid,
            UiTaskCase::SpellReady,
            UiTaskCase::SpellConfirmDelete,
        ];
        for (size, mode) in REQUIRED_MATRIX {
            for case in cases {
                let snapshot = creator_snapshot(case, size, mode);
                let issues = snapshot.task_issues(case);
                assert!(
                    issues.is_empty(),
                    "{} failed at {size:?} {mode:?}: {issues:#?}",
                    case.contract().id
                );
                if case == UiTaskCase::CharacterReady {
                    assert_element_grid_contract(&snapshot, size, mode);
                }
            }
        }
    }

    fn assert_element_grid_contract(snapshot: &UiTreeSnapshot, size: UVec2, mode: UiScaleMode) {
        let expected = [
            "Element Tool Air",
            "Element Tool Fire",
            "Element Tool Metal",
            "Element Tool Earth",
            "Element Tool Life",
            "Element Tool Water",
            "Element Tool Space",
            "Element Tool Lightning",
            "Element Tool Destruction",
            "Element Tool Volcano",
            "Element Tool Artifice",
            "Element Tool Crystal",
            "Element Tool Necromancy",
            "Element Tool Transmutation",
            "Element Tool Wild",
            "Element Tool Divination",
            "Element Tool Storm",
            "Element Tool Illusion",
        ];
        let grid = snapshot
            .nodes
            .iter()
            .find(|node| node.name == "Elemental Grid")
            .expect("the elemental chart container must be rendered");
        let grid_bounds = Rect::from_center_size(grid.center, grid.size);
        let focus_order = snapshot
            .focus_order
            .iter()
            .filter(|name| name.starts_with("Element Tool "))
            .map(String::as_str)
            .collect::<Vec<_>>();
        assert_eq!(
            focus_order, expected,
            "element keyboard order drifted at {size:?} {mode:?}"
        );

        for name in expected {
            let node = snapshot
                .nodes
                .iter()
                .find(|node| node.name == name)
                .expect("every canonical element tool must be rendered");
            assert!(
                node.size.x >= 43.5 && node.size.y >= 43.5,
                "{name} fell below the 44×44 target at {size:?} {mode:?}: {node:?}"
            );
            assert!(
                !node.tessellated,
                "{name} is an ordinary rectangular click target and must participate in overlap checks"
            );
            let bounds = Rect::from_center_size(node.center, node.size);
            assert!(
                bounds.min.x >= grid_bounds.min.x - 0.5
                    && bounds.min.y >= grid_bounds.min.y - 0.5
                    && bounds.max.x <= grid_bounds.max.x + 0.5
                    && bounds.max.y <= grid_bounds.max.y + 0.5,
                "{name} escaped the elemental chart at {size:?} {mode:?}: {bounds:?} outside {grid_bounds:?}"
            );
            assert!(
                node.scroll_reachable,
                "{name} lost its single-owner scroll route at {size:?} {mode:?}: {node:?}"
            );
            assert!(
                node.accessible_label
                    .as_deref()
                    .is_some_and(|label| label.contains("formula")),
                "{name} must expose classification and formula copy: {node:?}"
            );
        }
        let tools = snapshot
            .nodes
            .iter()
            .filter(|node| node.name.starts_with("Element Tool "))
            .collect::<Vec<_>>();
        for (index, left) in tools.iter().enumerate() {
            for right in tools.iter().skip(index + 1) {
                let overlap = Rect::from_center_size(left.center, left.size)
                    .intersect(Rect::from_center_size(right.center, right.size));
                assert!(
                    overlap.width() <= 0.5 || overlap.height() <= 0.5,
                    "{} and {} have ambiguous click bounds at {size:?} {mode:?}: {overlap:?}",
                    left.name,
                    right.name
                );
            }
        }
        assert_eq!(
            snapshot
                .nodes
                .iter()
                .filter(|node| node.name.starts_with("Element Formula "))
                .count(),
            18,
            "the visible formula fallback must cover every element"
        );
        assert!(
            snapshot
                .nodes
                .iter()
                .any(|node| node.name == "Selected Element Air"),
            "active selection needs a visible shape marker in addition to tint"
        );
        let air = snapshot
            .nodes
            .iter()
            .find(|node| node.name == "Element Tool Air")
            .expect("Air tool must exist");
        assert!(
            air.accessible_label
                .as_deref()
                .is_some_and(|label| label.contains("; selected;")),
            "active selection must also be spoken: {air:?}"
        );
    }

    #[test]
    fn element_grid_maps_and_emits_exact_typed_tools_for_pointer_and_keyboard() {
        let (screen, view) = creator_view(UiTaskCase::CharacterReady);
        let mut app = App::new();
        app.add_plugins(HeadlessUiPlugin::new(1920, 1080))
            .init_resource::<CreatorIntentLog>()
            .init_resource::<PointerActivation>()
            .init_resource::<KeyboardActivation>()
            .add_systems(
                PreUpdate,
                apply_keyboard_activation
                    .after(bevy::input::InputSystems)
                    .before(UiSystems::CaptureInput),
            )
            .add_systems(
                Update,
                (
                    apply_pointer_activation.before(UiSystems::EmitIntents),
                    record_creator_intents.after(UiSystems::EmitIntents),
                ),
            );
        app.world_mut().insert_resource(view);
        app.world_mut()
            .resource_mut::<NextState<hex_core::Screen>>()
            .set(screen);
        for _ in 0..8 {
            app.update();
        }

        for name in ["Air", "Fire", "Metal", "Earth", "Life", "Water"] {
            let entity = named_entity(app.world_mut(), &format!("Element Tool {name}"));
            let expected =
                CreatorIntent::ChooseTool(hex_assets::CreationCellKind::Gem(name.to_owned()));
            assert_eq!(
                app.world().get::<CreatorIntent>(entity),
                Some(&expected),
                "basic element {name} must paint the exact gem tool"
            );
        }
        for name in [
            "Space",
            "Lightning",
            "Destruction",
            "Volcano",
            "Artifice",
            "Crystal",
            "Necromancy",
            "Transmutation",
            "Wild",
            "Divination",
            "Storm",
            "Illusion",
        ] {
            let entity = named_entity(app.world_mut(), &format!("Element Tool {name}"));
            let expected =
                CreatorIntent::ChooseTool(hex_assets::CreationCellKind::Fusion(name.to_owned()));
            assert_eq!(
                app.world().get::<CreatorIntent>(entity),
                Some(&expected),
                "derived element {name} must paint the exact fusion tool"
            );
        }

        app.world_mut().resource_mut::<CreatorIntentLog>().0.clear();
        let air = named_entity(app.world_mut(), "Element Tool Air");
        app.world_mut().resource_mut::<PointerActivation>().0 = Some(air);
        app.update();
        assert_eq!(
            app.world().resource::<CreatorIntentLog>().0.as_slice(),
            &[CreatorIntent::ChooseTool(
                hex_assets::CreationCellKind::Gem("Air".to_owned())
            )],
            "pointer activation must emit the exact basic tool"
        );

        *app.world_mut()
            .get_mut::<Interaction>(air)
            .expect("the Air tool remains interactive") = Interaction::None;
        app.update();
        app.world_mut().resource_mut::<CreatorIntentLog>().0.clear();

        let space = named_entity(app.world_mut(), "Element Tool Space");
        app.world_mut().resource_mut::<KeyboardActivation>().0 = Some(space);
        app.update();
        assert_eq!(
            app.world().resource::<CreatorIntentLog>().0.as_slice(),
            &[CreatorIntent::ChooseTool(
                hex_assets::CreationCellKind::Fusion("Space".to_owned())
            )],
            "keyboard activation must emit the exact derived tool"
        );
    }

    #[test]
    fn accepted_noncanonical_elements_remain_complete_creator_tools() {
        let mut custom_file: hex_assets::ElementFile =
            ron::from_str(include_str!("../../../assets/config/elements.ron"))
                .expect("the production element catalog must parse");
        custom_file.wheel.push("Aether".to_owned());
        custom_file.wheel.push("Void".to_owned());
        custom_file.fusions.insert(
            "Tempest".to_owned(),
            vec![
                hex_assets::FusionInput {
                    element: "Aether".to_owned(),
                    mana: 1,
                },
                hex_assets::FusionInput {
                    element: "Air".to_owned(),
                    mana: 1,
                },
            ],
        );
        custom_file
            .validate()
            .expect("the extended element catalog must remain valid");
        let custom = hex_assets::ElementCatalog::from_file(&custom_file);
        let (screen, mut view) = creator_view(UiTaskCase::CharacterReady);
        view.elements = Some(custom);
        view.active_tool = Some(hex_assets::CreationCellKind::Gem("Aether".to_owned()));

        let mut app = App::new();
        app.add_plugins(HeadlessUiPlugin::new(1920, 1080))
            .init_resource::<CreatorIntentLog>()
            .init_resource::<PointerActivation>()
            .init_resource::<KeyboardActivation>()
            .add_systems(
                PreUpdate,
                apply_keyboard_activation
                    .after(bevy::input::InputSystems)
                    .before(UiSystems::CaptureInput),
            )
            .add_systems(
                Update,
                (
                    apply_pointer_activation.before(UiSystems::EmitIntents),
                    record_creator_intents.after(UiSystems::EmitIntents),
                ),
            );
        app.world_mut().insert_resource(view);
        app.world_mut()
            .resource_mut::<NextState<hex_core::Screen>>()
            .set(screen);
        for _ in 0..8 {
            app.update();
        }

        named_entity(app.world_mut(), "Element Tool Air");
        for name in ["Aether", "Void"] {
            let entity = named_entity(app.world_mut(), &format!("Element Tool {name}"));
            assert_eq!(
                app.world().get::<CreatorIntent>(entity),
                Some(&CreatorIntent::ChooseTool(
                    hex_assets::CreationCellKind::Gem(name.to_owned())
                )),
                "custom basic {name} must remain authorable"
            );
        }
        let aether = named_entity(app.world_mut(), "Element Tool Aether");
        let aether_label = app
            .world()
            .get::<AccessibleLabel>(aether)
            .expect("Aether has an accessible label");
        assert!(aether_label.0.contains("basic element"));
        assert!(aether_label.0.contains("formula basic element"));
        assert!(aether_label.0.contains("; selected;"));
        named_entity(app.world_mut(), "Selected Element Aether");

        let tempest = named_entity(app.world_mut(), "Element Tool Tempest");
        assert_eq!(
            app.world().get::<CreatorIntent>(tempest),
            Some(&CreatorIntent::ChooseTool(
                hex_assets::CreationCellKind::Fusion("Tempest".to_owned())
            )),
            "custom fusion must remain authorable"
        );
        let tempest_label = app
            .world()
            .get::<AccessibleLabel>(tempest)
            .expect("Tempest has an accessible label");
        assert!(tempest_label.0.contains("pair fusion"));
        assert!(tempest_label.0.contains("formula Aether + Air"));
        assert!(tempest_label.0.contains("; not selected;"));
        for name in ["Aether", "Void", "Tempest"] {
            named_entity(app.world_mut(), &format!("Element Formula {name}"));
        }
        {
            let world = app.world_mut();
            let mut text = world.query::<&Text>();
            assert!(
                text.iter(world).all(|text| !text
                    .as_str()
                    .contains("does not contain every canonical chart school")),
                "a valid catalog superset must not be described as incomplete"
            );
        }

        app.world_mut().resource_mut::<CreatorIntentLog>().0.clear();
        app.world_mut().resource_mut::<PointerActivation>().0 = Some(aether);
        app.update();
        assert_eq!(
            app.world().resource::<CreatorIntentLog>().0.as_slice(),
            &[CreatorIntent::ChooseTool(
                hex_assets::CreationCellKind::Gem("Aether".to_owned())
            )],
            "pointer activation must emit the exact custom basic tool"
        );

        *app.world_mut()
            .get_mut::<Interaction>(aether)
            .expect("the Aether tool remains interactive") = Interaction::None;
        app.update();
        app.world_mut().resource_mut::<CreatorIntentLog>().0.clear();
        app.world_mut().resource_mut::<KeyboardActivation>().0 = Some(tempest);
        app.update();
        assert_eq!(
            app.world().resource::<CreatorIntentLog>().0.as_slice(),
            &[CreatorIntent::ChooseTool(
                hex_assets::CreationCellKind::Fusion("Tempest".to_owned())
            )],
            "keyboard activation must emit the exact custom fusion tool"
        );
    }

    #[test]
    fn local_lattice_test_preserves_its_structural_matrix() {
        for (size, mode) in REQUIRED_MATRIX {
            let snapshot = lattice_demo_snapshot(size, mode);
            let issues = snapshot.task_issues(UiTaskCase::LatticeDemo);
            assert!(
                issues.is_empty(),
                "local lattice test failed at {size:?} {mode:?}: {issues:#?}"
            );
        }
    }

    #[test]
    fn sandbox_outcome_stays_minimal_across_the_required_matrix() {
        for (size, mode) in REQUIRED_MATRIX {
            let snapshot = sandbox_outcome_snapshot(size, mode);
            let issues = snapshot.task_issues(UiTaskCase::SandboxOutcome);
            assert!(
                issues.is_empty(),
                "Sandbox outcome failed at {size:?} {mode:?}: {issues:#?}"
            );
            let actions = snapshot
                .focus_order
                .iter()
                .filter(|name| matches!(name.as_str(), "Retry Exact" | "Return to Sandbox"))
                .cloned()
                .collect::<Vec<_>>();
            assert_eq!(actions, ["Retry Exact", "Return to Sandbox"]);
        }
    }

    #[test]
    fn deployment_surfaces_use_sandbox_vocabulary_across_the_required_matrix() {
        for (size, mode) in REQUIRED_MATRIX {
            for (case, complete) in [
                (UiTaskCase::DeploymentIncomplete, false),
                (UiTaskCase::DeploymentComplete, true),
            ] {
                let snapshot = deployment_snapshot(size, mode, complete);
                let issues = snapshot.task_issues(case);
                assert!(
                    issues.is_empty(),
                    "{} failed at {size:?} {mode:?}: {issues:#?}",
                    case.contract().id
                );
                assert!(snapshot
                    .nodes
                    .iter()
                    .any(|node| node.name == "Sandbox Deployment HUD"));
            }
        }
    }

    #[test]
    fn minimalist_gameplay_surfaces_preserve_their_structural_matrix() {
        let cases = [
            UiTaskCase::Exploration,
            UiTaskCase::PlayerTurnMaxActions,
            UiTaskCase::HostileTurn,
            UiTaskCase::CharacterMainView,
            UiTaskCase::ActivityTabs,
            UiTaskCase::Casting,
            UiTaskCase::AimingBlocked,
            UiTaskCase::DisableDecision,
            UiTaskCase::RestoreDecision,
            UiTaskCase::HudHiddenRequired,
            UiTaskCase::Pause,
        ];
        for (size, mode) in REQUIRED_MATRIX {
            for case in cases {
                let snapshot = gameplay_snapshot(case, size, mode);
                let issues = snapshot.task_issues(case);
                assert!(
                    issues.is_empty(),
                    "{} failed at {size:?} {mode:?}: {issues:#?}",
                    case.contract().id
                );
            }
        }
    }

    #[test]
    fn formation_main_view_passes_the_required_matrix() {
        for (size, mode) in REQUIRED_MATRIX {
            let snapshot = gameplay_snapshot(UiTaskCase::FormationMainView, size, mode);
            let issues = snapshot.task_issues(UiTaskCase::FormationMainView);
            assert!(
                issues.is_empty(),
                "Formation Main View failed at {size:?} {mode:?}: {issues:#?}"
            );
            assert!(snapshot
                .nodes
                .iter()
                .any(|node| node.name == "Formation Panel"));
        }
    }

    #[test]
    fn custom_visibility_and_compact_temporary_surface_are_structurally_distinct() {
        for (size, mode) in [
            (UVec2::new(1920, 1080), UiScaleMode::Auto),
            (UVec2::new(3840, 2160), UiScaleMode::Auto),
        ] {
            let snapshot = gameplay_snapshot(UiTaskCase::CustomHudVisibility, size, mode);
            let issues = snapshot.task_issues(UiTaskCase::CustomHudVisibility);
            assert!(
                issues.is_empty(),
                "custom visibility failed at {size:?} {mode:?}: {issues:#?}"
            );
        }

        for (size, mode) in [
            (UVec2::new(1280, 720), UiScaleMode::Auto),
            (UVec2::new(1920, 1080), UiScaleMode::Percent200),
        ] {
            let snapshot = gameplay_snapshot(UiTaskCase::CompactTemporarySurface, size, mode);
            let issues = snapshot.task_issues(UiTaskCase::CompactTemporarySurface);
            assert!(
                issues.is_empty(),
                "Compact temporary surface failed at {size:?} {mode:?}: {issues:#?}"
            );
        }
    }

    #[test]
    fn initiative_fixture_exposes_only_disclosed_units_as_inspection_controls() {
        let snapshot = gameplay_snapshot(
            UiTaskCase::PlayerTurnMaxActions,
            UVec2::new(1920, 1080),
            UiScaleMode::Auto,
        );
        for name in ["Initiative Unit 0", "Initiative Unit 1"] {
            let node = snapshot
                .nodes
                .iter()
                .find(|node| node.name == name)
                .unwrap_or_else(|| panic!("missing disclosed initiative control {name:?}"));
            assert!(node.focusable && node.keyboard_reachable == Some(true));
        }
        let unavailable = snapshot
            .nodes
            .iter()
            .find(|node| node.name == "Initiative Unit 2 Unavailable")
            .expect("the undisclosed hostile must retain an unavailable row without a control");
        assert!(!unavailable.focusable);
        assert!(snapshot
            .focus_order
            .iter()
            .all(|name| name != "Initiative Unit 2 Unavailable"));
    }

    #[test]
    fn retina_mapping_keeps_physical_pixels_separate_from_logical_layout() {
        let snapshot = settled_snapshot(
            hex_core::Screen::Title,
            UVec2::new(3840, 2160),
            2.0,
            UiScaleMode::Auto,
            |world| world.insert_resource(MainMenuView::default()),
        );
        assert_eq!(snapshot.metrics.logical_size, Vec2::new(1920.0, 1080.0));
        assert_eq!(snapshot.metrics.viewport, UiViewportClass::Standard);
        assert!(snapshot.task_issues(UiTaskCase::MainMenu).is_empty());
    }

    #[test]
    fn sandbox_loading_blocker_copy_remains_centralized() {
        assert_eq!(
            SandboxStartBlocker::MapsLoading.message(),
            "Sandbox maps are still loading."
        );
    }
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
            (UiHudSetup::Frame, UiHudSetup::Panels, UiHudSetup::Tooling).chain(),
        )
        .add_message::<UiIntent>()
        .init_resource::<ActivityLogView>()
        .init_resource::<CastingPanelView>()
        .init_resource::<GameplayChromeView>()
        .init_resource::<GameplayHudView>()
        .init_resource::<GameplayLatticesView>()
        .init_resource::<UiSettingsView>()
        .init_resource::<PauseView>()
        .init_resource::<PartyView>()
        .init_resource::<OutcomeView>()
        .init_resource::<LatticeDemoView>()
        .init_resource::<CreatorScreenView>()
        .init_resource::<SandboxView>()
        .init_resource::<DeploymentView>()
        .init_resource::<InitiativeView>()
        .init_resource::<MainMenuView>()
        .init_resource::<VfxTunerView>()
        .init_resource::<TargetPulseView>()
        .add_plugins(element_visual::plugin)
        .add_plugins((
            theme::plugin,
            casting_panel::plugin,
            combat_log::plugin,
            scale::plugin,
            focus::plugin,
            gameplay_frame::plugin,
            gameplay_lattices::plugin,
            initiative::plugin,
            party::plugin,
            shell::plugin,
            screens::plugin,
            action_rail::plugin,
            main_menu::plugin,
        ))
        .add_plugins((
            sandbox::plugin,
            outcome::plugin,
            lattice_demo::plugin,
            creator::plugin,
            deployment::plugin,
            vfx_tuner::plugin,
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
        MainMenu,
        Campaign,
        Tools,
        Settings,
        SettingsKeybindings,
        SettingsCapture,
        SettingsConflict,
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
        SandboxOverview,
        SandboxMapBrowser,
        SandboxMapDetail,
        SandboxParty,
        SandboxEnemies,
        SandboxCharacterPicker,
        DeploymentIncomplete,
        DeploymentComplete,
        Exploration,
        PlayerTurnMaxActions,
        HostileTurn,
        CharacterMainView,
        FormationMainView,
        ActivityTabs,
        CustomHudVisibility,
        CompactTemporarySurface,
        Casting,
        AimingBlocked,
        DisableDecision,
        RestoreDecision,
        HudHiddenRequired,
        Pause,
        SandboxOutcome,
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
        /// A blocking choice must present its forced Main View lattice at every scale.
        RequiredChoice,
    }

    impl UiTaskCase {
        /// Every known task case. Adding a task requires adding it here and to the
        /// exhaustive contract match below.
        pub const ALL: [Self; 42] = [
            Self::Splash,
            Self::Loading,
            Self::MainMenu,
            Self::Campaign,
            Self::Tools,
            Self::Settings,
            Self::SettingsKeybindings,
            Self::SettingsCapture,
            Self::SettingsConflict,
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
            Self::SandboxOverview,
            Self::SandboxMapBrowser,
            Self::SandboxMapDetail,
            Self::SandboxParty,
            Self::SandboxEnemies,
            Self::SandboxCharacterPicker,
            Self::DeploymentIncomplete,
            Self::DeploymentComplete,
            Self::Exploration,
            Self::PlayerTurnMaxActions,
            Self::HostileTurn,
            Self::CharacterMainView,
            Self::FormationMainView,
            Self::ActivityTabs,
            Self::CustomHudVisibility,
            Self::CompactTemporarySurface,
            Self::Casting,
            Self::AimingBlocked,
            Self::DisableDecision,
            Self::RestoreDecision,
            Self::HudHiddenRequired,
            Self::Pause,
            Self::SandboxOutcome,
        ];

        /// Fail-closed presentation contract for this task.
        #[must_use]
        pub const fn contract(self) -> UiTaskContract {
            use hex_core::Screen;
            match self {
                Self::Splash => task("startup-splash", Screen::Splash, &[], &[], false),
                Self::Loading => task("startup-loading", Screen::Loading, &[], &[], false),
                Self::MainMenu => task("main-menu", Screen::Title, MAIN_MENU_CONTROLS, &[], true),
                Self::Campaign => task("campaign", Screen::Title, &["Back"], &[], true),
                Self::Tools => task(
                    "tools",
                    Screen::Title,
                    &[
                        "Map Creator — Coming Soon",
                        "Character Creator",
                        "Spell Creator",
                        "Back",
                    ],
                    &[],
                    true,
                ),
                Self::Settings => task(
                    "settings",
                    Screen::Settings,
                    &["Back"],
                    &["Setting UiScale", "Setting UiVolume"],
                    true,
                ),
                Self::SettingsKeybindings => task(
                    "settings-keybindings",
                    Screen::Settings,
                    SETTINGS_NAV_CONTROLS,
                    &["Restore All Keybindings"],
                    true,
                ),
                Self::SettingsCapture => task(
                    "settings-capture",
                    Screen::Settings,
                    &["Cancel Key Capture"],
                    &[],
                    true,
                ),
                Self::SettingsConflict => {
                    task("settings-conflict", Screen::Settings, &[], &[], true)
                }
                Self::CharacterLibrary => task(
                    "creator-character-library",
                    Screen::CharacterCreator,
                    &["New Blank Character", "Open Spell Creator", "Back to Tools"],
                    &["Wolf Template"],
                    true,
                ),
                Self::SpellLibrary => task(
                    "creator-spell-library",
                    Screen::SpellCreator,
                    &["New Blank Spell", "Back to Tools"],
                    &["Training Spark"],
                    true,
                ),
                Self::CreatorLibraryRecovery => task(
                    "creator-library-recovery",
                    Screen::CharacterCreator,
                    &["Open Spell Creator", "Confirm Reset", "Back to Tools"],
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
                    &["Library", "Save", "Local Test", "Open in Sandbox"],
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
                Self::SandboxOverview => task(
                    "sandbox-overview",
                    Screen::Sandbox,
                    &["Back", "Start Sandbox"],
                    &[
                        "Choose Sandbox map",
                        "Choose Party characters",
                        "Choose Enemies characters",
                    ],
                    true,
                ),
                Self::SandboxMapBrowser => task(
                    "sandbox-map-browser",
                    Screen::Sandbox,
                    &["Back"],
                    &["Inspect Flat Arena", "Create New Map — Coming Soon"],
                    true,
                ),
                Self::SandboxMapDetail => task(
                    "sandbox-map-detail",
                    Screen::Sandbox,
                    &["Back", "Use Map"],
                    &[],
                    true,
                ),
                Self::SandboxParty => task(
                    "sandbox-party",
                    Screen::Sandbox,
                    &["Back"],
                    &["Party slot 1", "Party slot 6"],
                    true,
                ),
                Self::SandboxEnemies => task(
                    "sandbox-enemies",
                    Screen::Sandbox,
                    &["Back"],
                    &["Enemies slot 1", "Enemies slot 6"],
                    true,
                ),
                Self::SandboxCharacterPicker => task(
                    "sandbox-character-picker",
                    Screen::Sandbox,
                    &["Back", "Create a New Character", "Use Character"],
                    &["Preview Hedge Mage"],
                    true,
                ),
                Self::DeploymentIncomplete => task(
                    "deployment-incomplete",
                    Screen::Gameplay,
                    &["Deployment Party slot 1", "Undo", "Return to Sandbox"],
                    &[],
                    true,
                ),
                Self::DeploymentComplete => task(
                    "deployment-complete",
                    Screen::Gameplay,
                    &[
                        "Deployment Party slot 1",
                        "Deployment Enemies slot 6",
                        "Return to Sandbox",
                        "Start Combat",
                    ],
                    &[],
                    true,
                ),
                Self::Exploration => task(
                    "gameplay-exploration",
                    Screen::Gameplay,
                    &["Action Bar", "Action Bar Rest", "Action Bar Pause"],
                    &[],
                    true,
                ),
                Self::PlayerTurnMaxActions => task(
                    "gameplay-player-turn-max",
                    Screen::Gameplay,
                    &[
                        "Action Bar",
                        "Action Bar Channel",
                        "Action Bar End Turn",
                        "Action Bar Pause",
                    ],
                    &[],
                    true,
                ),
                Self::HostileTurn => {
                    task("gameplay-hostile-turn", Screen::Gameplay, &[], &[], true)
                }
                Self::CharacterMainView => task(
                    "gameplay-character-main-view",
                    Screen::Gameplay,
                    &[],
                    &[],
                    true,
                ),
                Self::FormationMainView => task(
                    "gameplay-formation-main-view",
                    Screen::Gameplay,
                    &[],
                    &[
                        "Formation Member 1",
                        "Formation Member 6",
                        "Party Movement Mode",
                        "Party Rest",
                        "Formation Preset Wedge",
                        "Formation Slot (0, 0)",
                    ],
                    true,
                ),
                Self::ActivityTabs => task(
                    "gameplay-activity-tabs",
                    Screen::Gameplay,
                    &[
                        "Activity Tab All",
                        "Activity Tab Combat",
                        "Activity Tab Activity",
                    ],
                    &[],
                    true,
                ),
                Self::CustomHudVisibility => task(
                    "gameplay-custom-hud-visibility",
                    Screen::Gameplay,
                    &[
                        "Party Member 1",
                        "Activity Tab All",
                        "Activity Tab Combat",
                        "Activity Tab Activity",
                    ],
                    &[],
                    false,
                ),
                Self::CompactTemporarySurface => task(
                    "gameplay-compact-temporary-surface",
                    Screen::Gameplay,
                    &["Party Member 1"],
                    &[],
                    false,
                ),
                Self::Casting => task(
                    "casting",
                    Screen::Gameplay,
                    &["Action Bar"],
                    &["Cast Lightning Bolt"],
                    true,
                ),
                Self::AimingBlocked => task(
                    "aiming-blocked",
                    Screen::Gameplay,
                    &[
                        "Action Bar",
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
                    &["Clear Disable Selection"],
                    &[],
                    true,
                ),
                Self::RestoreDecision => task(
                    "decision-restore",
                    Screen::Gameplay,
                    &["Clear Disable Selection"],
                    &[],
                    true,
                ),
                Self::HudHiddenRequired => task(
                    "hud-hidden-required",
                    Screen::Gameplay,
                    &["Clear Disable Selection"],
                    &[],
                    true,
                ),
                Self::Pause => task("pause", Screen::Gameplay, &["Resume"], &[], true),
                Self::SandboxOutcome => task(
                    "outcome-sandbox",
                    Screen::Gameplay,
                    &["Retry Exact", "Return to Sandbox"],
                    &[],
                    true,
                ),
            }
        }

        /// Fail-closed lattice surface required by this task's authored state.
        #[must_use]
        pub const fn lattice_requirement(self) -> UiTaskLatticeRequirement {
            match self {
                Self::CharacterMainView => UiTaskLatticeRequirement::Own,
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
                Self::MainMenu => MAIN_MENU_CONTROLS,
                Self::Tools => &["Character Creator", "Spell Creator", "Back"],
                Self::SettingsKeybindings => SETTINGS_NAV_CONTROLS,
                Self::FormationMainView => &[
                    "Formation Member 1",
                    "Formation Member 6",
                    "Party Movement Mode",
                    "Party Rest",
                    "Formation Preset Wedge",
                    "Formation Slot (0, 0)",
                ],
                _ => &[],
            }
        }
    }

    const MAIN_MENU_CONTROLS: &[&str] = &["Campaign", "Sandbox", "Tools", "Settings"];
    const SETTINGS_NAV_CONTROLS: &[&str] = &[
        "Back",
        "Settings Tab General",
        "Settings Tab Gameplay",
        "Settings Tab Interface",
        "Settings Tab Main View",
        "Settings Tab Camera",
        "Settings Tab System",
    ];

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
            self.layout_issues_with_overlap_scope(false)
        }

        fn layout_issues_with_overlap_scope(&self, active_focus_only: bool) -> Vec<String> {
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
                        && (node.in_focus_order || !active_focus_only)
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
                UiTaskCase::SettingsCapture | UiTaskCase::SettingsConflict => {
                    self.layout_issues_with_overlap_scope(true)
                }
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
            issues.extend(self.task_settings_issues(case));
            if case == UiTaskCase::FormationMainView {
                for required in ["Main View HUD Region", "Formation Panel"] {
                    if self.nodes.iter().all(|node| node.name != required) {
                        issues.push(format!(
                            "Formation Main View is missing required surface {required:?}"
                        ));
                    }
                }
                if self
                    .nodes
                    .iter()
                    .any(|node| node.name == "Lattice Readout Stack")
                {
                    issues.push(
                        "Formation Main View presented the Character lattice destination"
                            .to_owned(),
                    );
                }
                if self.metrics.viewport == crate::UiViewportClass::Compact {
                    let visible_regions = [
                        "Party HUD Region",
                        "Initiative HUD Region",
                        "Main View HUD Region",
                        "Action Bar HUD Region",
                        "Activity HUD Region",
                    ]
                    .into_iter()
                    .filter(|name| self.nodes.iter().any(|node| node.name == *name))
                    .collect::<Vec<_>>();
                    if visible_regions != ["Main View HUD Region"] {
                        issues.push(format!(
                            "Compact Formation Main View must own exactly one HUD region, found {visible_regions:?}"
                        ));
                    }
                }
            }
            if case == UiTaskCase::HudHiddenRequired {
                for hidden in [
                    "Party Panel",
                    "Initiative Panel",
                    "Activity Log Panel",
                    "Action Bar",
                ] {
                    if self.nodes.iter().any(|node| node.name == hidden) {
                        issues.push(format!(
                            "ordinary HUD surface {hidden:?} remained visible while the HUD was hidden"
                        ));
                    }
                }
            }
            if case == UiTaskCase::CustomHudVisibility {
                for visible in ["Party Panel", "Activity Log Panel"] {
                    if self.nodes.iter().all(|node| node.name != visible) {
                        issues.push(format!(
                            "custom HUD preference did not present requested surface {visible:?}"
                        ));
                    }
                }
                for hidden in ["Initiative Panel", "Action Bar", "Lattice Readout Stack"] {
                    if self.nodes.iter().any(|node| node.name == hidden) {
                        issues.push(format!(
                            "custom HUD preference unexpectedly presented {hidden:?}"
                        ));
                    }
                }
            }
            if case == UiTaskCase::CompactTemporarySurface {
                if self.metrics.viewport != crate::UiViewportClass::Compact {
                    issues.push(
                        "Compact temporary-surface fixture resolved outside Compact".to_owned(),
                    );
                }
                let visible_regions = [
                    "Party HUD Region",
                    "Initiative HUD Region",
                    "Main View HUD Region",
                    "Action Bar HUD Region",
                    "Activity HUD Region",
                ]
                .into_iter()
                .filter(|name| self.nodes.iter().any(|node| node.name == *name))
                .collect::<Vec<_>>();
                if visible_regions != ["Party HUD Region"] {
                    issues.push(format!(
                        "Compact temporary surface must own exactly one HUD region, found {visible_regions:?}"
                    ));
                }
            }
            issues
        }

        fn task_settings_issues(&self, case: UiTaskCase) -> Vec<String> {
            let mut issues = Vec::new();
            match case {
                UiTaskCase::SettingsKeybindings => {
                    if self
                        .nodes
                        .iter()
                        .all(|node| !node.name.starts_with("Binding Row "))
                    {
                        issues.push(
                            "Settings Keybindings contains no immutable binding rows".to_owned(),
                        );
                    }
                    for prefix in ["Rebind ", "Restore "] {
                        if self.nodes.iter().all(|node| !node.name.starts_with(prefix)) {
                            issues.push(format!(
                                "Settings Keybindings contains no named {prefix:?} controls"
                            ));
                        }
                    }
                    if self
                        .nodes
                        .iter()
                        .any(|node| node.name.contains("Reveal Knowledge (Development)"))
                    {
                        issues.push(
                            "shipping Settings Keybindings presented a development-only action"
                                .to_owned(),
                        );
                    }
                    if self.nodes.iter().any(|node| node.name == "Settings Modal") {
                        issues.push(
                            "Settings Keybindings unexpectedly presented a blocking modal"
                                .to_owned(),
                        );
                    }
                }
                UiTaskCase::SettingsCapture => {
                    self.require_settings_modal(case, &["Cancel Key Capture"], &mut issues)
                }
                UiTaskCase::SettingsConflict => {
                    const CONFLICT: &[&str] =
                        &["Swap Conflicting Bindings", "Cancel Binding Conflict"];
                    const RESTORE_ALL: &[&str] = &[
                        "Confirm Restore All Keybindings",
                        "Cancel Restore All Keybindings",
                    ];
                    let has_conflict = CONFLICT
                        .iter()
                        .any(|name| self.nodes.iter().any(|node| node.name == *name));
                    let has_restore_all = RESTORE_ALL
                        .iter()
                        .any(|name| self.nodes.iter().any(|node| node.name == *name));
                    match (has_conflict, has_restore_all) {
                        (true, false) => {
                            self.require_settings_modal(case, CONFLICT, &mut issues);
                        }
                        (false, true) => {
                            self.require_settings_modal(case, RESTORE_ALL, &mut issues);
                        }
                        (false, false) => issues.push(
                            "Settings Conflict requires a binding-conflict or Restore All confirmation modal"
                                .to_owned(),
                        ),
                        (true, true) => issues.push(
                            "Settings Conflict presented two mutually exclusive modal tasks"
                                .to_owned(),
                        ),
                    }
                }
                _ => {}
            }
            issues
        }

        fn require_settings_modal(
            &self,
            case: UiTaskCase,
            expected_controls: &[&str],
            issues: &mut Vec<String>,
        ) {
            if self.nodes.iter().all(|node| node.name != "Settings Modal") {
                issues.push(format!(
                    "{} is missing its blocking Settings Modal",
                    case.contract().id
                ));
            }
            for name in expected_controls {
                let Some(node) = self.nodes.iter().find(|node| node.name == *name) else {
                    issues.push(format!(
                        "{} is missing modal control {name:?}",
                        case.contract().id
                    ));
                    continue;
                };
                if node.visibility_requirement != Some(crate::UiVisibilityRequirement::Immediate) {
                    issues.push(format!(
                        "{} modal control {name:?} is not explicitly Immediate",
                        case.contract().id
                    ));
                }
                if !node.fully_visible {
                    issues.push(format!(
                        "{} modal control {name:?} is not initially visible: {node:?}",
                        case.contract().id
                    ));
                }
            }
            if self.focus_order != expected_controls {
                issues.push(format!(
                    "{} modal must trap focus in {expected_controls:?}, found {:?}",
                    case.contract().id,
                    self.focus_order
                ));
            }
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
        /// Fixture-specific checks stay deliberately small: route/task contracts
        /// own named controls, while this method adds only composition facts that
        /// are common to live visual review and the headless oracle.
        #[must_use]
        pub fn review_fixture_issues(&self, fixture: &str) -> Vec<String> {
            let mut issues = self.layout_issues();
            if fixture == "casting-list"
                && !self.nodes.iter().any(|node| node.name == "Casting Panel")
            {
                issues.push("casting-list is missing the populated casting surface".to_owned());
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
}
