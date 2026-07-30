//! Saved character and spell authoring.
//!
//! The screen edits name-based drafts and writes only through `CreationStore`.
//! Runtime ids are deliberately absent here.

use std::collections::BTreeSet;

use bevy::input_focus::tab_navigation::TabIndex;
use bevy::prelude::*;
use bevy::text::EditableText;
use bevy::ui_widgets::ScrollArea;
use hex_assets::{
    character_lattice_file, character_runtime_key, combined_spell_file, creator_character_issues,
    creator_spell_issues, normalized_name, ContentIndex, CreationCell, CreationCellKind,
    CreationPresetCatalog, CustomCharacterId, CustomSpellId, Effect, ElementCatalog, LatticeFile,
    LatticeLibrary, PresetAudience, SavedCharacter, SavedSpell, SpellBook, SpellFile,
    SpellReference, SubstanceTable, TargetShape, MAX_CREATION_NAME_CHARS,
};
use hex_core::{LatticeCoord, Screen};

use crate::creation_presentation::{CharacterBuildSummary, SpellBuildSummary};
use crate::creation_store::CreationStore;
use crate::menus::lattice_view::short_name;
use crate::menus::widgets::{
    blurb, display, element_color, fine, heading, label, panel, panel_node, row_button, UiAssets,
    ACCENT, ACCENT_EDGE, DANGER, EDGE, FUSION_COLOR, LABEL,
};
use crate::storage::StoragePaths;

use super::{despawn_screen, screen_root};

const HISTORY_LIMIT: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum CreatorTab {
    #[default]
    Characters,
    Spells,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum CreatorView {
    #[default]
    Hub,
    Character,
    Spell,
}

#[derive(Debug, Clone)]
enum CreatorSnapshot {
    Character(SavedCharacter),
    Spell(SavedSpell),
}

#[derive(Resource, Debug, Default)]
pub(crate) struct CreatorSession {
    tab: CreatorTab,
    view: CreatorView,
    character: Option<SavedCharacter>,
    spell: Option<SavedSpell>,
    selected_cell: Option<LatticeCoord>,
    active_tool: Option<CreationCellKind>,
    erase_tool: bool,
    zoom_step: i8,
    character_dirty: bool,
    spell_dirty: bool,
    notice: String,
    confirm_delete: bool,
    confirm_reset: bool,
    undo: Vec<CreatorSnapshot>,
    redo: Vec<CreatorSnapshot>,
    return_to_combat_lab: bool,
    return_to_character_creator: bool,
    revision: u64,
}

/// Explicit entry intent keeps top-level navigation separate from gameplay returns.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CreatorEntryRequest {
    CharacterLibrary,
    SpellLibrary,
    SpellFromCharacter,
}

impl CreatorSession {
    fn bump(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    fn remember_character(&mut self) {
        if let Some(character) = &self.character {
            self.undo
                .push(CreatorSnapshot::Character(character.clone()));
            if self.undo.len() > HISTORY_LIMIT {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
    }

    fn remember_spell(&mut self) {
        if let Some(spell) = &self.spell {
            self.undo.push(CreatorSnapshot::Spell(spell.clone()));
            if self.undo.len() > HISTORY_LIMIT {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
    }
}

#[derive(Component, Debug, Clone)]
enum CreatorAction {
    Back,
    OpenSpellCreator,
    NewCharacter,
    NewSpell,
    SelectCharacter(CustomCharacterId),
    SelectSpell(CustomSpellId),
    DuplicateCharacter,
    DuplicateSpell,
    DuplicatePackagedCharacter(String),
    DuplicatePackagedSpell(String),
    SaveCharacter,
    SaveSpell,
    DeleteCharacter,
    DeleteSpell,
    SelectCell(LatticeCoord),
    AddCell(LatticeCoord),
    InspectTool,
    ChooseTool(CreationCellKind),
    ChooseErase,
    Zoom(i8),
    FitLattice,
    RemoveCell,
    AdjustStat {
        element: String,
        channelling: bool,
        delta: i8,
    },
    AddRequirement,
    RemoveRequirement(usize),
    CycleRequirement(usize),
    AdjustRequirement(usize, i8),
    SetEnchantment(bool),
    SetSingleTarget(bool),
    AdjustRange(i8),
    AdjustDefense(i8),
    AddEffect(EffectKind),
    RemoveEffect(usize),
    MoveEffect(usize, i8),
    AdjustEffect(usize, i8),
    Undo,
    Redo,
    DiscardChanges,
    LocalTest,
    TestOnMap,
    ResetLibrary,
}

#[derive(Debug, Clone, Copy)]
enum EffectKind {
    Disable,
    Burn,
    Restore,
    Reveal,
}

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
enum NameField {
    Character,
    Spell,
}

#[derive(Component)]
struct CreatorRoot;

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<CreatorSession>()
        .add_systems(
            OnEnter(Screen::CharacterCreator),
            (initialize_session, rebuild_screen).chain(),
        )
        .add_systems(
            OnEnter(Screen::SpellCreator),
            (initialize_session, rebuild_screen).chain(),
        )
        .add_systems(
            Update,
            (
                sync_name_fields,
                handle_actions,
                rebuild_when_requested,
                handle_escape,
            )
                .chain()
                .run_if(creator_screen_active),
        )
        .add_systems(
            OnExit(Screen::CharacterCreator),
            despawn_screen(Screen::CharacterCreator),
        )
        .add_systems(
            OnExit(Screen::SpellCreator),
            despawn_screen(Screen::SpellCreator),
        );
}

fn creator_screen_active(screen: Res<State<Screen>>) -> bool {
    matches!(
        screen.get(),
        Screen::CharacterCreator | Screen::SpellCreator
    )
}

fn initialize_session(
    mut commands: Commands,
    mut session: ResMut<CreatorSession>,
    entry_request: Option<Res<CreatorEntryRequest>>,
    edit_request: Option<Res<super::combat_lab::CreatorEditRequest>>,
    store: Res<CreationStore>,
    overlay: Option<Res<super::combat_lab::CreatorContentOverlay>>,
) {
    let editing_from_lab = edit_request.is_some();
    super::combat_lab::restore_shipped_content(&mut commands, overlay.as_deref());
    commands.remove_resource::<super::combat_lab::CombatLabSession>();
    if editing_from_lab {
        session.return_to_combat_lab = true;
    }
    if let Some(request) = entry_request.as_deref().copied() {
        apply_entry_request(&mut session, request);
        commands.remove_resource::<CreatorEntryRequest>();
    }
    if let Some(request) = edit_request {
        if let Some(character) = store
            .file
            .characters
            .iter()
            .find(|character| character.id == request.character)
        {
            session.character = Some(character.clone());
            session.character_dirty = false;
            session.selected_cell = Some(LatticeCoord::ORIGIN);
            session.tab = CreatorTab::Characters;
            session.view = CreatorView::Character;
        }
        commands.remove_resource::<super::combat_lab::CreatorEditRequest>();
    }
    if session.character.is_none() {
        session.character = store.file.characters.first().cloned().or_else(|| {
            Some(SavedCharacter::blank(
                CustomCharacterId(store.file.next_character_id.max(1)),
                "New Character",
            ))
        });
    }
    if session.spell.is_none() {
        session.spell = store.file.spells.first().cloned().or_else(|| {
            Some(SavedSpell::blank(
                CustomSpellId(store.file.next_spell_id.max(1)),
                "New Spell",
            ))
        });
    }
    session.selected_cell = Some(LatticeCoord::ORIGIN);
    session.notice = if editing_from_lab {
        "Sandbox setup preserved. Resolve the blockers, save, then return to Combat Lab.".to_owned()
    } else {
        store.error.clone().unwrap_or_default()
    };
    session.confirm_delete = false;
    session.confirm_reset = false;
    session.bump();
}

fn apply_entry_request(session: &mut CreatorSession, request: CreatorEntryRequest) {
    match request {
        CreatorEntryRequest::CharacterLibrary => {
            session.tab = CreatorTab::Characters;
            session.view = CreatorView::Hub;
            session.return_to_combat_lab = false;
            session.return_to_character_creator = false;
        }
        CreatorEntryRequest::SpellLibrary => {
            session.tab = CreatorTab::Spells;
            session.view = CreatorView::Hub;
            session.return_to_combat_lab = false;
            session.return_to_character_creator = false;
        }
        CreatorEntryRequest::SpellFromCharacter => {
            session.tab = CreatorTab::Spells;
            session.view = CreatorView::Hub;
            session.return_to_character_creator = true;
        }
    }
}

fn handle_escape(
    keys: Res<ButtonInput<KeyCode>>,
    mut next: ResMut<NextState<Screen>>,
    mut session: ResMut<CreatorSession>,
) {
    let dirty = match session.tab {
        CreatorTab::Characters => session.character_dirty,
        CreatorTab::Spells => session.spell_dirty,
    };
    if keys.just_pressed(KeyCode::Escape) && !dirty {
        if session.tab == CreatorTab::Spells && session.return_to_character_creator {
            session.tab = CreatorTab::Characters;
            session.view = CreatorView::Character;
            session.return_to_character_creator = false;
            next.set(Screen::CharacterCreator);
        } else {
            next.set(if session.return_to_combat_lab {
                Screen::CombatLab
            } else {
                Screen::Title
            });
        }
    }
}

fn rebuild_when_requested(
    mut commands: Commands,
    roots: Query<Entity, With<CreatorRoot>>,
    session: Res<CreatorSession>,
    assets: Res<UiAssets>,
    store: Res<CreationStore>,
    elements: Option<Res<ElementCatalog>>,
    spell_book: Option<Res<SpellBook>>,
    spell_file: Option<Res<SpellFile>>,
    lattice_file: Option<Res<LatticeFile>>,
    presets: Option<Res<CreationPresetCatalog>>,
    mut last_revision: Local<u64>,
) {
    if roots.is_empty() || *last_revision != session.revision {
        for root in &roots {
            commands.entity(root).despawn();
        }
        spawn_creator_ui(
            &mut commands,
            &assets,
            &session,
            &store,
            elements.as_deref(),
            spell_book.as_deref(),
            spell_file.as_deref(),
            lattice_file.as_deref(),
            presets.as_deref(),
        );
        *last_revision = session.revision;
    }
}

fn rebuild_screen(
    mut commands: Commands,
    assets: Res<UiAssets>,
    session: Res<CreatorSession>,
    store: Res<CreationStore>,
    elements: Option<Res<ElementCatalog>>,
    spell_book: Option<Res<SpellBook>>,
    spell_file: Option<Res<SpellFile>>,
    lattice_file: Option<Res<LatticeFile>>,
    presets: Option<Res<CreationPresetCatalog>>,
) {
    spawn_creator_ui(
        &mut commands,
        &assets,
        &session,
        &store,
        elements.as_deref(),
        spell_book.as_deref(),
        spell_file.as_deref(),
        lattice_file.as_deref(),
        presets.as_deref(),
    );
}

#[expect(
    clippy::too_many_arguments,
    reason = "one immutable view model builds the whole creator screen"
)]
fn spawn_creator_ui(
    commands: &mut Commands,
    assets: &UiAssets,
    session: &CreatorSession,
    store: &CreationStore,
    elements: Option<&ElementCatalog>,
    spell_book: Option<&SpellBook>,
    spell_file: Option<&SpellFile>,
    lattice_file: Option<&LatticeFile>,
    presets: Option<&CreationPresetCatalog>,
) {
    let screen = match session.tab {
        CreatorTab::Characters => Screen::CharacterCreator,
        CreatorTab::Spells => Screen::SpellCreator,
    };
    commands
        .spawn((screen_root(screen, "Creator Screen"), CreatorRoot))
        .insert(Node {
            padding: UiRect::all(Val::Px(18.0)),
            row_gap: Val::Px(10.0),
            ..screen_root_node()
        })
        .with_children(|root| {
            root.spawn(Node {
                width: Val::Percent(96.0),
                flex_direction: FlexDirection::Row,
                justify_content: JustifyContent::SpaceBetween,
                align_items: AlignItems::Center,
                ..default()
            })
            .with_children(|header| {
                header.spawn(display(
                    assets,
                    match session.view {
                        CreatorView::Hub => match session.tab {
                            CreatorTab::Characters => "Character Library",
                            CreatorTab::Spells => "Spell Library",
                        },
                        CreatorView::Character => "Character Workspace",
                        CreatorView::Spell => "Spell Workspace",
                    },
                ));
                header
                    .spawn(Node {
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(8.0),
                        align_items: AlignItems::Center,
                        ..default()
                    })
                    .with_children(|actions| {
                        if session.view != CreatorView::Hub {
                            action_button(actions, assets, "Undo", CreatorAction::Undo, 90.0);
                            action_button(actions, assets, "Redo", CreatorAction::Redo, 90.0);
                        }
                        if store.error.is_some() {
                            action_button(
                                actions,
                                assets,
                                if session.confirm_reset {
                                    "Confirm Reset"
                                } else {
                                    "Reset Library"
                                },
                                CreatorAction::ResetLibrary,
                                140.0,
                            );
                        }
                        let current_dirty = match session.tab {
                            CreatorTab::Characters => session.character_dirty,
                            CreatorTab::Spells => session.spell_dirty,
                        };
                        if current_dirty {
                            action_button(
                                actions,
                                assets,
                                "Discard Changes",
                                CreatorAction::DiscardChanges,
                                150.0,
                            );
                        }
                        action_button(
                            actions,
                            assets,
                            if session.view == CreatorView::Hub {
                                "Title"
                            } else {
                                "Library"
                            },
                            CreatorAction::Back,
                            100.0,
                        );
                    });
            });
            if !session.notice.is_empty() {
                root.spawn(fine(assets, session.notice.clone()))
                    .insert(TextColor(if session.notice.contains("saved") {
                        ACCENT
                    } else {
                        DANGER
                    }));
            }
            root.spawn(Node {
                width: Val::Percent(100.0),
                height: Val::Px(0.0),
                min_height: Val::Px(0.0),
                flex_basis: Val::Px(0.0),
                flex_grow: 1.0,
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(12.0),
                ..default()
            })
            .with_children(|body| match session.view {
                CreatorView::Hub => {
                    spawn_creator_hub(body, assets, session, store, elements, spell_book, presets)
                }
                CreatorView::Character => spawn_character_tab(
                    body,
                    assets,
                    session,
                    store,
                    elements,
                    spell_book,
                    lattice_file,
                    presets,
                ),
                CreatorView::Spell => spawn_spell_tab(
                    body, assets, session, store, elements, spell_book, spell_file, presets,
                ),
            });
        });
}

fn spawn_creator_hub(
    body: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    session: &CreatorSession,
    store: &CreationStore,
    elements: Option<&ElementCatalog>,
    spell_book: Option<&SpellBook>,
    presets: Option<&CreationPresetCatalog>,
) {
    body.spawn(panel())
        .insert(Node {
            width: Val::Px(240.0),
            min_height: Val::Px(0.0),
            ..panel_node()
        })
        .with_children(|navigation| {
            navigation.spawn(heading(
                assets,
                match session.tab {
                    CreatorTab::Characters => "character creator",
                    CreatorTab::Spells => "spell creator",
                },
            ));
            navigation.spawn(blurb(
                assets,
                match session.tab {
                    CreatorTab::Characters => {
                        "Build saved lattices from templates or start blank. Only clean, Map-ready characters enter Combat Lab."
                    }
                    CreatorTab::Spells => {
                        "Build saved spells from templates or start blank. Ready spells can be inscribed by characters."
                    }
                },
            ));
            if session.tab == CreatorTab::Characters {
                action_button(
                    navigation,
                    assets,
                    "Open Spell Creator",
                    CreatorAction::OpenSpellCreator,
                    190.0,
                );
            }
        });

    body.spawn(panel())
        .insert(Node {
            min_width: Val::Px(0.0),
            min_height: Val::Px(0.0),
            flex_grow: 1.0,
            ..panel_node()
        })
        .with_children(|library| match session.tab {
            CreatorTab::Characters => {
                library.spawn(heading(assets, "saved characters"));
                action_button(
                    library,
                    assets,
                    "New Blank Character",
                    CreatorAction::NewCharacter,
                    220.0,
                );
                library
                    .spawn((
                        ScrollArea,
                        Node {
                            width: Val::Percent(100.0),
                            height: Val::Px(360.0),
                            min_height: Val::Px(160.0),
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px(8.0),
                            overflow: Overflow::scroll_y(),
                            ..default()
                        },
                    ))
                    .with_children(|list| {
                        if store.file.characters.is_empty() {
                            list.spawn(blurb(assets, "No saved characters yet."));
                        }
                        for saved in &store.file.characters {
                            let summary = CharacterBuildSummary::from_saved(
                                saved,
                                &store.file,
                                elements,
                                spell_book,
                            );
                            creator_record_card(
                                list,
                                assets,
                                &saved.name,
                                if summary.ready() {
                                    "MAP READY"
                                } else {
                                    "BLOCKED"
                                },
                                &summary.compact_line(),
                                CreatorAction::SelectCharacter(saved.id),
                                summary.ready(),
                            );
                        }
                    });
                library.spawn(heading(assets, "templates · duplicate to edit"));
                if let Some(presets) = presets {
                    library
                        .spawn(Node {
                            width: Val::Percent(100.0),
                            flex_direction: FlexDirection::Row,
                            flex_wrap: FlexWrap::Wrap,
                            column_gap: Val::Px(8.0),
                            row_gap: Val::Px(8.0),
                            ..default()
                        })
                        .with_children(|shelf| {
                            for record in presets
                                .characters
                                .iter()
                                .filter(|record| record.audience == PresetAudience::HumanTemplate)
                            {
                                action_button(
                                    shelf,
                                    assets,
                                    record.character.name.clone(),
                                    CreatorAction::DuplicatePackagedCharacter(record.key.clone()),
                                    190.0,
                                );
                            }
                        });
                }
            }
            CreatorTab::Spells => {
                library.spawn(heading(assets, "saved spells"));
                action_button(
                    library,
                    assets,
                    "New Blank Spell",
                    CreatorAction::NewSpell,
                    220.0,
                );
                library
                    .spawn((
                        ScrollArea,
                        Node {
                            width: Val::Percent(100.0),
                            height: Val::Px(360.0),
                            min_height: Val::Px(160.0),
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px(8.0),
                            overflow: Overflow::scroll_y(),
                            ..default()
                        },
                    ))
                    .with_children(|list| {
                        if store.file.spells.is_empty() {
                            list.spawn(blurb(assets, "No saved spells yet."));
                        }
                        for saved in &store.file.spells {
                            let summary = SpellBuildSummary::from_saved(saved, elements);
                            creator_record_card(
                                list,
                                assets,
                                &saved.name,
                                if summary.issues.is_empty() {
                                    "READY"
                                } else {
                                    "DRAFT"
                                },
                                &summary.sentence,
                                CreatorAction::SelectSpell(saved.id),
                                summary.issues.is_empty(),
                            );
                        }
                    });
                library.spawn(heading(assets, "templates · duplicate to edit"));
                if let Some(presets) = presets {
                    library
                        .spawn(Node {
                            width: Val::Percent(100.0),
                            flex_direction: FlexDirection::Row,
                            flex_wrap: FlexWrap::Wrap,
                            column_gap: Val::Px(8.0),
                            row_gap: Val::Px(8.0),
                            ..default()
                        })
                        .with_children(|shelf| {
                            for record in presets
                                .spells
                                .iter()
                                .filter(|record| record.audience == PresetAudience::HumanTemplate)
                            {
                                action_button(
                                    shelf,
                                    assets,
                                    record.spell.name.clone(),
                                    CreatorAction::DuplicatePackagedSpell(record.key.clone()),
                                    190.0,
                                );
                            }
                        });
                }
            }
        });

    body.spawn(panel())
        .insert(Node {
            width: Val::Px(330.0),
            min_height: Val::Px(0.0),
            ..panel_node()
        })
        .with_children(|summary| {
            summary.spawn(heading(assets, "testing loop"));
            summary.spawn(blurb(
                assets,
                "Create a spell, save it, inscribe it in a character, then Test on Map to prefill Combat Lab.",
            ));
            summary.spawn(heading(assets, "status language"));
            summary.spawn(fine(assets, "READY · spell can be inscribed and deployed"));
            summary.spawn(fine(assets, "MAP READY · character can enter Combat Lab"));
            summary.spawn(fine(assets, "DRAFT / BLOCKED · saved, editable, not deployable"));
        });
}

fn creator_record_card(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    name: &str,
    status: &str,
    summary: &str,
    action: CreatorAction,
    ready: bool,
) {
    parent
        .spawn((row_button(name.to_owned(), 520.0), action))
        .insert(Node {
            width: Val::Percent(100.0),
            min_height: Val::Px(70.0),
            padding: UiRect::all(Val::Px(12.0)),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::FlexStart,
            row_gap: Val::Px(4.0),
            border: UiRect::all(Val::Px(1.0)),
            ..default()
        })
        .insert(BorderColor::all(if ready { ACCENT_EDGE } else { DANGER }))
        .with_children(|card| {
            card.spawn(label(assets, format!("{name} · {status}")));
            card.spawn(blurb(assets, summary.to_owned()));
        });
}

fn screen_root_node() -> Node {
    Node {
        width: Val::Percent(100.0),
        height: Val::Percent(100.0),
        align_items: AlignItems::Center,
        justify_content: JustifyContent::FlexStart,
        flex_direction: FlexDirection::Column,
        ..default()
    }
}

fn action_button(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    text: impl Into<String>,
    action: CreatorAction,
    width: f32,
) {
    let text = text.into();
    parent
        .spawn((row_button(text.clone(), width), action))
        .with_child(label(assets, text));
}

fn name_input(parent: &mut ChildSpawnerCommands, assets: &UiAssets, value: &str, field: NameField) {
    parent
        .spawn((
            Name::new(match field {
                NameField::Character => "Character Name",
                NameField::Spell => "Spell Name",
            }),
            EditableText {
                max_characters: Some(MAX_CREATION_NAME_CHARS),
                visible_width: Some(24.0),
                ..EditableText::new(value)
            },
            TextFont {
                font: assets.body.clone().into(),
                ..TextFont::from_font_size(18.0)
            },
            TextColor(LABEL),
            BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.08)),
            BorderColor::all(ACCENT_EDGE),
            Node {
                width: Val::Percent(100.0),
                min_height: Val::Px(44.0),
                padding: UiRect::all(Val::Px(9.0)),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            field,
        ))
        .observe(|focus: On<Pointer<Click>>, mut commands: Commands| {
            commands.entity(focus.entity).insert(TabIndex(0));
        });
}

#[expect(
    clippy::too_many_arguments,
    reason = "tab rendering consumes the loaded catalogs it presents"
)]
fn spawn_character_tab(
    body: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    session: &CreatorSession,
    store: &CreationStore,
    elements: Option<&ElementCatalog>,
    spell_book: Option<&SpellBook>,
    _lattice_file: Option<&LatticeFile>,
    _presets: Option<&CreationPresetCatalog>,
) {
    let Some(character) = &session.character else {
        body.spawn(blurb(assets, "No character draft."));
        return;
    };
    let issues = match (elements, spell_book) {
        (Some(elements), Some(spells)) => {
            character_map_issues(character, &store.file, elements, spells)
        }
        _ => vec!["content catalogs are still loading".to_owned()],
    };

    body.spawn(panel())
        .insert(Node {
            width: Val::Px(250.0),
            min_height: Val::Px(0.0),
            overflow: Overflow::scroll_y(),
            ..panel_node()
        })
        .with_children(|palette| {
            palette.spawn(heading(assets, "content palette"));
            palette.spawn(blurb(
                assets,
                "Choose a tool, then click occupied hexes or outlined neighbor slots.",
            ));
            colored_tool_button(
                palette,
                assets,
                "Inspect",
                CreatorAction::InspectTool,
                Color::srgba(0.24, 0.26, 0.31, 0.96),
                session.active_tool.is_none() && !session.erase_tool,
            );
            colored_tool_button(
                palette,
                assets,
                "Blank",
                CreatorAction::ChooseTool(CreationCellKind::Blank),
                Color::srgba(0.28, 0.29, 0.32, 0.96),
                session.active_tool == Some(CreationCellKind::Blank),
            );
            if let Some(elements) = elements {
                palette.spawn(heading(assets, "gems and fusions"));
                for index in 0..elements.len() {
                    let Some(id) = u16::try_from(index).ok().map(hex_core::ElementId) else {
                        continue;
                    };
                    let Some(name) = elements.name(id) else {
                        continue;
                    };
                    let kind = if elements.is_higher_order(id) {
                        CreationCellKind::Fusion(name.to_owned())
                    } else {
                        CreationCellKind::Gem(name.to_owned())
                    };
                    colored_tool_button(
                        palette,
                        assets,
                        if elements.is_higher_order(id) {
                            format!("Fusion · {name}")
                        } else {
                            format!("Gem · {name}")
                        },
                        CreatorAction::ChooseTool(kind.clone()),
                        if elements.is_higher_order(id) {
                            FUSION_COLOR
                        } else {
                            element_color(Some(id), elements)
                        },
                        session.active_tool.as_ref() == Some(&kind),
                    );
                }
            }
            palette.spawn(heading(assets, "ready spells"));
            if let Some(spells) = spell_book {
                for (_, name, _spell) in spells.iter().filter(|(_, _, spell)| {
                    matches!(
                        spell.targeting.shape,
                        TargetShape::SelfCast | TargetShape::Single
                    ) && hex_combat::delivers_anything(spell)
                }) {
                    let kind = CreationCellKind::Spell(SpellReference::Shipped(name.to_owned()));
                    colored_tool_button(
                        palette,
                        assets,
                        format!("Spell · {name}"),
                        CreatorAction::ChooseTool(kind.clone()),
                        Color::srgba(0.30, 0.33, 0.40, 0.96),
                        session.active_tool.as_ref() == Some(&kind),
                    );
                }
            }
            if let Some(elements) = elements {
                for spell in &store.file.spells {
                    if creator_spell_issues(spell, elements).is_empty()
                        && hex_combat::creator_spell_deployability(&spell.spell).is_ok()
                    {
                        let kind = CreationCellKind::Spell(SpellReference::Custom(spell.id));
                        colored_tool_button(
                            palette,
                            assets,
                            format!("Custom · {}", spell.name),
                            CreatorAction::ChooseTool(kind.clone()),
                            Color::srgba(0.37, 0.31, 0.47, 0.96),
                            session.active_tool.as_ref() == Some(&kind),
                        );
                    }
                }
            }
            action_button(
                palette,
                assets,
                "Manage Spells",
                CreatorAction::OpenSpellCreator,
                190.0,
            );
            colored_tool_button(
                palette,
                assets,
                "Erase",
                CreatorAction::ChooseErase,
                Color::srgba(0.46, 0.13, 0.11, 0.96),
                session.erase_tool,
            );
        });

    body.spawn(panel())
        .insert(Node {
            min_width: Val::Px(0.0),
            min_height: Val::Px(0.0),
            flex_grow: 1.0,
            ..panel_node()
        })
        .with_children(|center| {
            name_input(center, assets, &character.name, NameField::Character);
            center
                .spawn(Node {
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::SpaceBetween,
                    align_items: AlignItems::Center,
                    ..default()
                })
                .with_children(|toolbar| {
                    toolbar.spawn(fine(
                        assets,
                        format!(
                            "ACTIVE TOOL · {}",
                            if session.erase_tool {
                                "ERASE".to_owned()
                            } else {
                                session
                                    .active_tool
                                    .as_ref()
                                    .map(cell_label)
                                    .unwrap_or_else(|| "INSPECT".to_owned())
                                    .replace('\n', " ")
                                    .to_uppercase()
                            }
                        ),
                    ));
                    toolbar
                        .spawn(Node {
                            flex_direction: FlexDirection::Row,
                            column_gap: Val::Px(5.0),
                            ..default()
                        })
                        .with_children(|zoom| {
                            action_button(zoom, assets, "Fit", CreatorAction::FitLattice, 58.0);
                            action_button(zoom, assets, "−", CreatorAction::Zoom(-1), 44.0);
                            zoom.spawn(label(
                                assets,
                                format!("{}%", lattice_scale_percent(session.zoom_step)),
                            ));
                            action_button(zoom, assets, "+", CreatorAction::Zoom(1), 44.0);
                        });
                });
            center
                .spawn((
                    Name::new("Lattice Canvas"),
                    ScrollArea,
                    Node {
                        width: Val::Percent(100.0),
                        min_height: Val::Px(0.0),
                        flex_grow: 1.0,
                        overflow: Overflow::scroll(),
                        ..default()
                    },
                    ScrollPosition(Vec2::new(200.0, 120.0)),
                    BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.22)),
                ))
                .with_children(|canvas| {
                    canvas
                        .spawn(Node {
                            width: Val::Px(1_100.0),
                            height: Val::Px(760.0),
                            position_type: PositionType::Relative,
                            ..default()
                        })
                        .with_children(|surface| {
                            spawn_lattice_cells(
                                surface,
                                assets,
                                character,
                                session.selected_cell,
                                session.zoom_step,
                                elements,
                                &store.file,
                            );
                        });
                });
            center
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(8.0),
                    ..default()
                })
                .with_children(|actions| {
                    action_button(actions, assets, "Save", CreatorAction::SaveCharacter, 110.0);
                    action_button(
                        actions,
                        assets,
                        "Duplicate",
                        CreatorAction::DuplicateCharacter,
                        120.0,
                    );
                    action_button(
                        actions,
                        assets,
                        if session.confirm_delete {
                            "Confirm Delete"
                        } else {
                            "Delete"
                        },
                        CreatorAction::DeleteCharacter,
                        140.0,
                    );
                    action_button(
                        actions,
                        assets,
                        "Local Test",
                        CreatorAction::LocalTest,
                        120.0,
                    );
                    action_button(
                        actions,
                        assets,
                        "Test on Map",
                        CreatorAction::TestOnMap,
                        130.0,
                    );
                    if !issues.is_empty() || session.character_dirty {
                        actions.spawn(fine(
                            assets,
                            if session.character_dirty {
                                "Test on Map blocked · save current changes"
                            } else {
                                "Test on Map blocked · resolve checks"
                            },
                        ));
                    }
                });
        });

    body.spawn(panel())
        .insert(Node {
            width: Val::Px(330.0),
            min_height: Val::Px(0.0),
            overflow: Overflow::scroll_y(),
            ..panel_node()
        })
        .with_children(|right| {
            right.spawn(heading(assets, "cell inspector"));
            if let Some(coord) = session.selected_cell {
                let content = character
                    .cells
                    .iter()
                    .find(|cell| cell.coord() == coord)
                    .map(|cell| cell_label(&cell.kind))
                    .unwrap_or_else(|| "Neighbor add slot".to_owned());
                right.spawn(label(
                    assets,
                    format!(
                        "{} · ({}, {}){}",
                        content.replace('\n', " "),
                        coord.q(),
                        coord.r(),
                        if coord == LatticeCoord::ORIGIN {
                            " · ORIGIN"
                        } else {
                            ""
                        }
                    ),
                ));
                right.spawn(blurb(
                    assets,
                    "Palette tools paint directly. Inspect leaves cells unchanged.",
                ));
                if coord != LatticeCoord::ORIGIN {
                    action_button(
                        right,
                        assets,
                        "Remove Selected Cell",
                        CreatorAction::RemoveCell,
                        250.0,
                    );
                }
            }
            right.spawn(heading(assets, "attunement / channel"));
            if let Some(elements) = elements {
                for id in elements.wheel() {
                    let Some(name) = elements.name(*id) else {
                        continue;
                    };
                    let capacity = character.attunement.get(name).copied().unwrap_or(0);
                    let channel = character.channelling.get(name).copied().unwrap_or(0);
                    right.spawn(fine(
                        assets,
                        format!("{name}: capacity {capacity} · channel {channel}"),
                    ));
                    right
                        .spawn(Node {
                            flex_direction: FlexDirection::Row,
                            column_gap: Val::Px(4.0),
                            ..default()
                        })
                        .with_children(|row| {
                            for (text, channelling, delta) in [
                                ("A−", false, -1),
                                ("A+", false, 1),
                                ("C−", true, -1),
                                ("C+", true, 1),
                            ] {
                                action_button(
                                    row,
                                    assets,
                                    text,
                                    CreatorAction::AdjustStat {
                                        element: name.to_owned(),
                                        channelling,
                                        delta,
                                    },
                                    58.0,
                                );
                            }
                        });
                }
            }
            right.spawn(heading(
                assets,
                if issues.is_empty() {
                    "Map Ready"
                } else {
                    "Checks"
                },
            ));
            if issues.is_empty() {
                right.spawn(blurb(assets, "Saved, clean versions may enter Combat Lab."));
            } else {
                for issue in &issues {
                    right
                        .spawn(fine(assets, format!("• {issue}")))
                        .insert(TextColor(DANGER));
                }
            }
            let summary =
                CharacterBuildSummary::from_saved(character, &store.file, elements, spell_book);
            right.spawn(heading(assets, "build summary"));
            right.spawn(label(assets, summary.compact_line()));
            if !summary.attunement.is_empty() {
                right.spawn(fine(
                    assets,
                    format!("Attunement/channel · {}", summary.attunement.join(" · ")),
                ));
            }
            for spell in summary.spells {
                right.spawn(fine(assets, format!("{} · {}", spell.name, spell.sentence)));
            }
        });
}

fn spawn_lattice_cells(
    surface: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    character: &SavedCharacter,
    selected: Option<LatticeCoord>,
    zoom_step: i8,
    elements: Option<&ElementCatalog>,
    library: &hex_assets::CreationLibraryFile,
) {
    let occupied: BTreeSet<LatticeCoord> =
        character.cells.iter().map(CreationCell::coord).collect();
    let mut additions = BTreeSet::new();
    for coord in &occupied {
        additions.extend(
            coord
                .neighbors()
                .into_iter()
                .filter(|neighbor| !occupied.contains(neighbor)),
        );
    }
    let scale = lattice_scale(zoom_step);
    for cell in &character.cells {
        let coord = cell.coord();
        let (left, top) = lattice_pixel(coord, scale);
        let selected_cell = selected == Some(coord);
        let color = brighten(
            cell_color(&cell.kind, elements),
            if selected_cell { 0.24 } else { 0.0 },
        );
        surface
            .spawn((
                Name::new(format!("Creator Cell {},{}", coord.q(), coord.r())),
                Button,
                CreatorAction::SelectCell(coord),
                ImageNode {
                    image: assets.hex_cell.clone(),
                    color,
                    ..default()
                },
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(left),
                    top: Val::Px(top),
                    width: Val::Px(72.0 * scale),
                    height: Val::Px(83.0 * scale),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    flex_direction: FlexDirection::Column,
                    ..default()
                },
            ))
            .with_children(|hex| {
                hex.spawn((
                    Text::new(resolved_cell_label(&cell.kind, library)),
                    TextFont {
                        font: assets.body.clone().into(),
                        ..TextFont::from_font_size((11.0 * scale).max(9.0))
                    },
                    TextColor(LABEL),
                    Pickable::IGNORE,
                ));
                hex.spawn((
                    Text::new(if coord == LatticeCoord::ORIGIN {
                        "ORIGIN"
                    } else if selected_cell {
                        "SELECTED"
                    } else {
                        ""
                    }),
                    TextFont {
                        font: assets.body.clone().into(),
                        ..TextFont::from_font_size((8.0 * scale).max(7.0))
                    },
                    TextColor(if selected_cell { ACCENT } else { LABEL }),
                    Pickable::IGNORE,
                ));
            });
    }
    if character.cells.len() < hex_assets::MAX_CREATION_CELLS {
        for coord in additions {
            let (left, top) = lattice_pixel(coord, scale);
            surface
                .spawn((
                    Name::new(format!("Add Cell {},{}", coord.q(), coord.r())),
                    Button,
                    CreatorAction::AddCell(coord),
                    ImageNode {
                        image: assets.hex_cell.clone(),
                        color: Color::srgba(0.93, 0.79, 0.46, 0.18),
                        ..default()
                    },
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(left + 8.0 * scale),
                        top: Val::Px(top + 9.0 * scale),
                        width: Val::Px(56.0 * scale),
                        height: Val::Px(65.0 * scale),
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        ..default()
                    },
                ))
                .with_child(label(assets, "+"));
        }
    }
}

#[expect(
    clippy::cast_precision_loss,
    reason = "creator coordinates are capped to 64 cells"
)]
fn lattice_pixel(coord: LatticeCoord, scale: f32) -> (f32, f32) {
    (
        520.0 + (coord.q() as f32 * 76.0 + coord.r() as f32 * 38.0) * scale,
        330.0 + coord.r() as f32 * 62.0 * scale,
    )
}

fn cell_label(kind: &CreationCellKind) -> String {
    match kind {
        CreationCellKind::Gem(name) => name.clone(),
        CreationCellKind::Fusion(name) => format!("{name}\nFusion"),
        CreationCellKind::Spell(SpellReference::Shipped(name)) => name.clone(),
        CreationCellKind::Spell(SpellReference::Custom(id)) => format!("Custom\n#{}", id.0),
        CreationCellKind::Blank => "Blank".to_owned(),
    }
}

fn resolved_cell_label(
    kind: &CreationCellKind,
    library: &hex_assets::CreationLibraryFile,
) -> String {
    match kind {
        CreationCellKind::Spell(SpellReference::Custom(id)) => library
            .spells
            .iter()
            .find(|spell| spell.id == *id)
            .map_or_else(
                || format!("Missing\n#{}", id.0),
                |spell| short_name(&spell.name),
            ),
        _ => short_name(&cell_label(kind).replace('\n', " ")),
    }
}

fn cell_color(kind: &CreationCellKind, elements: Option<&ElementCatalog>) -> Color {
    match kind {
        CreationCellKind::Gem(name) => elements
            .map_or(Color::srgba(0.16, 0.45, 0.52, 0.96), |elements| {
                element_color(elements.id(name), elements)
            }),
        CreationCellKind::Fusion(_) => FUSION_COLOR,
        CreationCellKind::Spell(_) => Color::srgba(0.30, 0.33, 0.40, 0.96),
        CreationCellKind::Blank => Color::srgba(0.28, 0.29, 0.32, 0.9),
    }
}

fn lattice_scale(zoom_step: i8) -> f32 {
    match zoom_step {
        ..=-2 => 0.7,
        -1 => 0.85,
        0 => 1.0,
        1 => 1.15,
        2 => 1.3,
        _ => 1.45,
    }
}

fn lattice_scale_percent(zoom_step: i8) -> u16 {
    match zoom_step {
        ..=-2 => 70,
        -1 => 85,
        0 => 100,
        1 => 115,
        2 => 130,
        _ => 145,
    }
}

fn brighten(color: Color, lift: f32) -> Color {
    let color = color.to_srgba();
    Color::srgba(
        color.red + (1.0 - color.red) * lift,
        color.green + (1.0 - color.green) * lift,
        color.blue + (1.0 - color.blue) * lift,
        color.alpha,
    )
}

fn colored_tool_button(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    text: impl Into<String>,
    action: CreatorAction,
    color: Color,
    selected: bool,
) {
    let text = text.into();
    parent
        .spawn((row_button(text.clone(), 200.0), action))
        .insert((
            crate::menus::widgets::OwnColors,
            BackgroundColor(brighten(color, if selected { 0.26 } else { 0.0 })),
            BorderColor::all(if selected { ACCENT } else { EDGE }),
        ))
        .with_child(label(assets, text));
}

fn spawn_spell_tab(
    body: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    session: &CreatorSession,
    store: &CreationStore,
    elements: Option<&ElementCatalog>,
    spell_book: Option<&SpellBook>,
    _spell_file: Option<&SpellFile>,
    _presets: Option<&CreationPresetCatalog>,
) {
    let Some(saved) = &session.spell else {
        body.spawn(blurb(assets, "No spell draft."));
        return;
    };
    let issues = elements.map_or_else(
        || vec!["element catalog is still loading".to_owned()],
        |elements| spell_map_issues(saved, elements),
    );

    body.spawn(panel())
        .insert(Node {
            width: Val::Px(300.0),
            min_height: Val::Px(0.0),
            overflow: Overflow::scroll_y(),
            ..panel_node()
        })
        .with_children(|left| {
            left.spawn(heading(assets, "requirements · 1–6"));
            for (index, requirement) in saved.spell.requirements.iter().enumerate() {
                let color = elements.map_or(Color::srgba(0.16, 0.45, 0.52, 0.96), |elements| {
                    element_color(elements.id(&requirement.element), elements)
                });
                left.spawn(crate::menus::widgets::panel())
                    .insert((
                        Node {
                            width: Val::Percent(100.0),
                            padding: UiRect::all(Val::Px(10.0)),
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px(6.0),
                            border: UiRect::all(Val::Px(2.0)),
                            ..default()
                        },
                        BorderColor::all(color),
                    ))
                    .with_children(|token| {
                        token.spawn(label(
                            assets,
                            format!("{} · {} mana", requirement.element, requirement.mana),
                        ));
                        token
                            .spawn(Node {
                                flex_direction: FlexDirection::Row,
                                column_gap: Val::Px(4.0),
                                ..default()
                            })
                            .with_children(|row| {
                                action_button(
                                    row,
                                    assets,
                                    "Element",
                                    CreatorAction::CycleRequirement(index),
                                    84.0,
                                );
                                action_button(
                                    row,
                                    assets,
                                    "−",
                                    CreatorAction::AdjustRequirement(index, -1),
                                    42.0,
                                );
                                action_button(
                                    row,
                                    assets,
                                    "+",
                                    CreatorAction::AdjustRequirement(index, 1),
                                    42.0,
                                );
                                action_button(
                                    row,
                                    assets,
                                    "Remove",
                                    CreatorAction::RemoveRequirement(index),
                                    78.0,
                                );
                            });
                    });
            }
            if saved.spell.requirements.len() < 6 {
                action_button(
                    left,
                    assets,
                    "+ Add Requirement",
                    CreatorAction::AddRequirement,
                    220.0,
                );
            }
            left.spawn(heading(assets, "casting and targeting"));
            left.spawn(label(
                assets,
                match saved.spell.casting {
                    hex_assets::CastingAxis::Evocation => "Evocation".to_owned(),
                    hex_assets::CastingAxis::Enchantment { defense } => {
                        format!("Enchantment · defense {defense}")
                    }
                },
            ));
            left.spawn(label(
                assets,
                format!(
                    "{} · range {}",
                    if matches!(saved.spell.targeting.shape, TargetShape::SelfCast) {
                        "Self"
                    } else {
                        "Single target"
                    },
                    saved.spell.targeting.range
                ),
            ));
            left.spawn(Node {
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                column_gap: Val::Px(5.0),
                row_gap: Val::Px(5.0),
                ..default()
            })
            .with_children(|controls| {
                let enchantment = matches!(
                    saved.spell.casting,
                    hex_assets::CastingAxis::Enchantment { .. }
                );
                segmented_button(
                    controls,
                    assets,
                    "Evocation",
                    CreatorAction::SetEnchantment(false),
                    !enchantment,
                    104.0,
                );
                segmented_button(
                    controls,
                    assets,
                    "Enchantment",
                    CreatorAction::SetEnchantment(true),
                    enchantment,
                    120.0,
                );
                let single = saved.spell.targeting.shape == TargetShape::Single;
                segmented_button(
                    controls,
                    assets,
                    "Self",
                    CreatorAction::SetSingleTarget(false),
                    !single,
                    72.0,
                );
                segmented_button(
                    controls,
                    assets,
                    "Single",
                    CreatorAction::SetSingleTarget(true),
                    single,
                    82.0,
                );
                if single {
                    action_button(
                        controls,
                        assets,
                        "Range −",
                        CreatorAction::AdjustRange(-1),
                        84.0,
                    );
                    action_button(
                        controls,
                        assets,
                        "Range +",
                        CreatorAction::AdjustRange(1),
                        84.0,
                    );
                }
                if enchantment {
                    action_button(
                        controls,
                        assets,
                        "Defense −",
                        CreatorAction::AdjustDefense(-1),
                        100.0,
                    );
                    action_button(
                        controls,
                        assets,
                        "Defense +",
                        CreatorAction::AdjustDefense(1),
                        100.0,
                    );
                }
            });
        });

    body.spawn(panel())
        .insert(Node {
            min_width: Val::Px(0.0),
            min_height: Val::Px(0.0),
            flex_grow: 1.0,
            overflow: Overflow::scroll_y(),
            ..panel_node()
        })
        .with_children(|form| {
            name_input(form, assets, &saved.name, NameField::Spell);
            let summary = SpellBuildSummary::from_saved(saved, elements);
            form.spawn(heading(assets, "ordered effects"));
            form.spawn(label(assets, summary.sentence.clone()));
            for (index, effect) in saved.spell.effects.iter().enumerate() {
                let effect_text = crate::creation_presentation::effect_summary(effect);
                form.spawn(crate::menus::widgets::panel())
                    .insert((
                        Node {
                            width: Val::Percent(100.0),
                            min_height: Val::Px(96.0),
                            padding: UiRect::all(Val::Px(12.0)),
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px(7.0),
                            border: UiRect::all(Val::Px(2.0)),
                            ..default()
                        },
                        BorderColor::all(effect_color(effect)),
                    ))
                    .with_children(|card| {
                        card.spawn(label(
                            assets,
                            format!("{} · {}", index + 1, effect_text.to_uppercase()),
                        ));
                        card.spawn(blurb(assets, effect_explanation(effect)));
                        card.spawn(Node {
                            flex_direction: FlexDirection::Row,
                            column_gap: Val::Px(5.0),
                            flex_wrap: FlexWrap::Wrap,
                            ..default()
                        })
                        .with_children(|row| {
                            action_button(
                                row,
                                assets,
                                "←",
                                CreatorAction::MoveEffect(index, -1),
                                44.0,
                            );
                            action_button(
                                row,
                                assets,
                                "→",
                                CreatorAction::MoveEffect(index, 1),
                                44.0,
                            );
                            action_button(
                                row,
                                assets,
                                "Value −",
                                CreatorAction::AdjustEffect(index, -1),
                                76.0,
                            );
                            action_button(
                                row,
                                assets,
                                "Value +",
                                CreatorAction::AdjustEffect(index, 1),
                                76.0,
                            );
                            action_button(
                                row,
                                assets,
                                "Remove",
                                CreatorAction::RemoveEffect(index),
                                86.0,
                            );
                        });
                    });
            }
            form.spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(5.0),
                flex_wrap: FlexWrap::Wrap,
                ..default()
            })
            .with_children(|row| {
                for (name, kind) in [
                    ("+ Disable", EffectKind::Disable),
                    ("+ Burn", EffectKind::Burn),
                    ("+ Restore", EffectKind::Restore),
                    ("+ Reveal", EffectKind::Reveal),
                ] {
                    action_button(row, assets, name, CreatorAction::AddEffect(kind), 105.0);
                }
            });
            form.spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(8.0),
                margin: UiRect::top(Val::Px(8.0)),
                ..default()
            })
            .with_children(|actions| {
                action_button(actions, assets, "Save", CreatorAction::SaveSpell, 110.0);
                action_button(
                    actions,
                    assets,
                    "Duplicate",
                    CreatorAction::DuplicateSpell,
                    120.0,
                );
                action_button(
                    actions,
                    assets,
                    if session.confirm_delete {
                        "Confirm Delete"
                    } else {
                        "Delete"
                    },
                    CreatorAction::DeleteSpell,
                    140.0,
                );
            });
        });

    body.spawn(panel())
        .insert(Node {
            width: Val::Px(320.0),
            min_height: Val::Px(0.0),
            overflow: Overflow::scroll_y(),
            ..panel_node()
        })
        .with_children(|right| {
            let summary = SpellBuildSummary::from_saved(saved, elements);
            right.spawn(heading(
                assets,
                if issues.is_empty() { "Ready" } else { "Draft" },
            ));
            right.spawn(label(assets, summary.sentence));
            if !summary.requirements.is_empty() {
                right.spawn(fine(
                    assets,
                    format!("Requirements · {}", summary.requirements.join(" · ")),
                ));
            }
            right.spawn(fine(assets, summary.casting));
            if issues.is_empty() {
                right.spawn(blurb(
                    assets,
                    "This saved spell can be inscribed and map-tested.",
                ));
            } else {
                for issue in &issues {
                    right
                        .spawn(fine(assets, format!("• {issue}")))
                        .insert(TextColor(DANGER));
                }
            }
            let dependents = store.file.spell_dependents(saved.id);
            if !dependents.is_empty() {
                right.spawn(heading(assets, "used by"));
                for character in dependents {
                    right.spawn(fine(assets, character.name.clone()));
                }
            }
            if spell_book.is_none() {
                right.spawn(blurb(assets, "Shipped spell catalog is loading."));
            }
        });
}

fn effect_color(effect: &Effect) -> Color {
    match effect {
        Effect::DisableHexes { .. } => Color::srgb(0.72, 0.25, 0.20),
        Effect::Burn { .. } => Color::srgb(0.78, 0.38, 0.14),
        Effect::RestoreHexes { .. } => Color::srgb(0.18, 0.55, 0.43),
        Effect::Reveal { .. } => Color::srgb(0.76, 0.64, 0.22),
        Effect::ModifyIncomingDisables { .. } => Color::srgb(0.32, 0.45, 0.64),
        _ => EDGE,
    }
}

fn segmented_button(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    text: &'static str,
    action: CreatorAction,
    selected: bool,
    width: f32,
) {
    parent
        .spawn((row_button(text, width), action))
        .insert(BorderColor::all(if selected { ACCENT } else { EDGE }))
        .with_child(label(
            assets,
            if selected {
                format!("✓ {text}")
            } else {
                text.to_owned()
            },
        ));
}

fn effect_explanation(effect: &Effect) -> String {
    match effect {
        Effect::DisableHexes { count, .. } => {
            format!("The defender chooses {count} live lattice cell(s) to disable.")
        }
        Effect::Burn { turns } => {
            format!("Disables one additional cell at the start of {turns} target turn(s).")
        }
        Effect::RestoreHexes { count } => {
            format!("The caster chooses up to {count} disabled cell(s) to restore.")
        }
        Effect::Reveal { tier } => {
            format!("Reveals the target lattice at tier {tier}.")
        }
        Effect::ModifyIncomingDisables { amount } => {
            format!("Reduces incoming disable count by {amount}.")
        }
        _ => "This effect is not deployable from the Wave 6 Creator.".to_owned(),
    }
}

fn sync_name_fields(
    fields: Query<(&EditableText, &NameField), Changed<EditableText>>,
    mut session: ResMut<CreatorSession>,
) {
    for (field, kind) in &fields {
        let value = field.value().to_string();
        match kind {
            NameField::Character => {
                if let Some(character) = &mut session.character {
                    if character.name != value {
                        character.name = value;
                        session.character_dirty = true;
                    }
                }
            }
            NameField::Spell => {
                if let Some(spell) = &mut session.spell {
                    if spell.name != value {
                        spell.name = value;
                        session.spell_dirty = true;
                    }
                }
            }
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the creator command reducer needs the catalogs used by its commands"
)]
fn handle_actions(
    clicked: Query<(&Interaction, &CreatorAction), Changed<Interaction>>,
    mut session: ResMut<CreatorSession>,
    mut store: ResMut<CreationStore>,
    paths: Res<StoragePaths>,
    elements: Option<Res<ElementCatalog>>,
    spell_book: Option<Res<SpellBook>>,
    spell_file: Option<Res<SpellFile>>,
    _lattice_file: Option<Res<LatticeFile>>,
    substances: Option<Res<SubstanceTable>>,
    presets: Option<Res<CreationPresetCatalog>>,
    mut commands: Commands,
    mut next: ResMut<NextState<Screen>>,
) {
    for (interaction, action) in &clicked {
        if *interaction != Interaction::Pressed {
            continue;
        }
        if !matches!(
            action,
            CreatorAction::DeleteCharacter | CreatorAction::DeleteSpell
        ) {
            session.confirm_delete = false;
        }
        if !matches!(action, CreatorAction::ResetLibrary) {
            session.confirm_reset = false;
        }
        match action {
            CreatorAction::Back => {
                let dirty = match session.tab {
                    CreatorTab::Characters => session.character_dirty,
                    CreatorTab::Spells => session.spell_dirty,
                };
                if dirty {
                    session.notice = "Save or discard the current edits before leaving.".to_owned();
                } else if session.view != CreatorView::Hub {
                    session.view = CreatorView::Hub;
                    session.active_tool = None;
                    session.erase_tool = false;
                } else if session.tab == CreatorTab::Spells && session.return_to_character_creator {
                    session.tab = CreatorTab::Characters;
                    session.view = CreatorView::Character;
                    session.return_to_character_creator = false;
                    next.set(Screen::CharacterCreator);
                } else {
                    next.set(if session.return_to_combat_lab {
                        Screen::CombatLab
                    } else {
                        Screen::Title
                    });
                }
            }
            CreatorAction::OpenSpellCreator => {
                commands.insert_resource(CreatorEntryRequest::SpellFromCharacter);
                next.set(Screen::SpellCreator);
            }
            CreatorAction::NewCharacter => {
                if session.character_dirty {
                    session.notice =
                        "Save the current character before starting another.".to_owned();
                } else {
                    let id = store.file.allocate_character_id();
                    session.character = Some(SavedCharacter::blank(
                        id,
                        unique_character_name(&store.file, "New Character", None),
                    ));
                    session.character_dirty = true;
                    session.selected_cell = Some(LatticeCoord::ORIGIN);
                    session.view = CreatorView::Character;
                    session.tab = CreatorTab::Characters;
                    session.active_tool = None;
                    session.erase_tool = false;
                }
            }
            CreatorAction::NewSpell => {
                if session.spell_dirty {
                    session.notice = "Save the current spell before starting another.".to_owned();
                } else {
                    let id = store.file.allocate_spell_id();
                    session.spell = Some(SavedSpell::blank(
                        id,
                        unique_spell_name(&store.file, spell_book.as_deref(), "New Spell", None),
                    ));
                    session.spell_dirty = true;
                    session.view = CreatorView::Spell;
                    session.tab = CreatorTab::Spells;
                }
            }
            CreatorAction::SelectCharacter(id) => {
                if session.character_dirty {
                    session.notice = "Save the current character before switching.".to_owned();
                } else if let Some(saved) =
                    store.file.characters.iter().find(|saved| saved.id == *id)
                {
                    session.character = Some(saved.clone());
                    session.selected_cell = saved.cells.first().map(CreationCell::coord);
                    session.confirm_delete = false;
                    session.view = CreatorView::Character;
                    session.tab = CreatorTab::Characters;
                    session.active_tool = None;
                    session.erase_tool = false;
                }
            }
            CreatorAction::SelectSpell(id) => {
                if session.spell_dirty {
                    session.notice = "Save the current spell before switching.".to_owned();
                } else if let Some(saved) = store.file.spells.iter().find(|saved| saved.id == *id) {
                    session.spell = Some(saved.clone());
                    session.confirm_delete = false;
                    session.view = CreatorView::Spell;
                    session.tab = CreatorTab::Spells;
                }
            }
            CreatorAction::DuplicateCharacter => {
                if let Some(mut copy) = session.character.clone() {
                    copy.id = store.file.allocate_character_id();
                    copy.name =
                        unique_character_name(&store.file, &format!("{} Copy", copy.name), None);
                    session.character = Some(copy);
                    session.character_dirty = true;
                    session.confirm_delete = false;
                    session.view = CreatorView::Character;
                }
            }
            CreatorAction::DuplicateSpell => {
                if let Some(mut copy) = session.spell.clone() {
                    copy.id = store.file.allocate_spell_id();
                    copy.name = unique_spell_name(
                        &store.file,
                        spell_book.as_deref(),
                        &format!("{} Copy", copy.name),
                        None,
                    );
                    session.spell = Some(copy);
                    session.spell_dirty = true;
                    session.confirm_delete = false;
                    session.view = CreatorView::Spell;
                }
            }
            CreatorAction::DuplicatePackagedCharacter(key) => {
                if let Some(record) = presets
                    .as_deref()
                    .and_then(|catalog| catalog.characters.iter().find(|record| record.key == *key))
                {
                    let mut copy = record.character.clone();
                    copy.id = store.file.allocate_character_id();
                    copy.name =
                        unique_character_name(&store.file, &format!("{} Copy", copy.name), None);
                    session.character = Some(copy);
                    session.character_dirty = true;
                    session.selected_cell = Some(LatticeCoord::ORIGIN);
                    session.view = CreatorView::Character;
                }
            }
            CreatorAction::DuplicatePackagedSpell(key) => {
                if let Some(record) = presets
                    .as_deref()
                    .and_then(|catalog| catalog.spells.iter().find(|record| record.key == *key))
                {
                    let mut copy = record.spell.clone();
                    copy.id = store.file.allocate_spell_id();
                    copy.name = unique_spell_name(
                        &store.file,
                        spell_book.as_deref(),
                        &format!("{} Copy", copy.name),
                        None,
                    );
                    session.spell = Some(copy);
                    session.spell_dirty = true;
                    session.view = CreatorView::Spell;
                }
            }
            CreatorAction::SaveCharacter => {
                let Some(character) = session.character.clone() else {
                    continue;
                };
                let collision = store.file.characters.iter().any(|saved| {
                    saved.id != character.id
                        && normalized_name(&saved.name) == normalized_name(&character.name)
                });
                if collision {
                    session.notice = "A saved character already has that name.".to_owned();
                } else {
                    match store.save_character(character, &paths) {
                        Ok(()) => {
                            session.character_dirty = false;
                            session.notice = "Character saved.".to_owned();
                        }
                        Err(error) => session.notice = error,
                    }
                }
            }
            CreatorAction::SaveSpell => {
                let Some(spell) = session.spell.clone() else {
                    continue;
                };
                let custom_collision = store.file.spells.iter().any(|saved| {
                    saved.id != spell.id
                        && normalized_name(&saved.name) == normalized_name(&spell.name)
                });
                let shipped_collision = spell_book
                    .as_deref()
                    .and_then(|book| book.id(&spell.name))
                    .is_some();
                if custom_collision || shipped_collision {
                    session.notice =
                        "Spell names must be unique across shipped and custom content.".to_owned();
                } else {
                    match store.save_spell(spell, &paths) {
                        Ok(()) => {
                            session.spell_dirty = false;
                            session.notice = "Spell saved.".to_owned();
                        }
                        Err(error) => session.notice = error,
                    }
                }
            }
            CreatorAction::DeleteCharacter => {
                if !session.confirm_delete {
                    session.confirm_delete = true;
                    session.notice =
                        "Press Confirm Delete to remove this saved character.".to_owned();
                } else if let Some(character) = &session.character {
                    match store.delete_character(character.id, &paths) {
                        Ok(()) => {
                            session.character = store.file.characters.first().cloned();
                            session.character_dirty = false;
                            session.confirm_delete = false;
                            session.notice = "Character deleted.".to_owned();
                            session.view = CreatorView::Hub;
                        }
                        Err(error) => session.notice = error,
                    }
                }
            }
            CreatorAction::DeleteSpell => {
                if !session.confirm_delete {
                    session.confirm_delete = true;
                    session.notice = "Press Confirm Delete to remove this saved spell.".to_owned();
                } else if let Some(spell) = &session.spell {
                    match store.delete_spell(spell.id, &paths) {
                        Ok(()) => {
                            session.spell = store.file.spells.first().cloned();
                            session.spell_dirty = false;
                            session.confirm_delete = false;
                            session.notice = "Spell deleted.".to_owned();
                            session.view = CreatorView::Hub;
                        }
                        Err(error) => {
                            session.notice = format!("Spell cannot be deleted: {error}");
                            session.confirm_delete = false;
                        }
                    }
                }
            }
            CreatorAction::SelectCell(coord) => {
                session.selected_cell = Some(*coord);
                if session.erase_tool {
                    if *coord == LatticeCoord::ORIGIN {
                        session.notice = "The origin cell cannot be removed.".to_owned();
                    } else {
                        session.remember_character();
                        let next_selected = if let Some(character) = &mut session.character {
                            character.cells.retain(|cell| cell.coord() != *coord);
                            character.cells.first().map(CreationCell::coord)
                        } else {
                            None
                        };
                        session.character_dirty = true;
                        session.selected_cell = next_selected;
                    }
                } else if let Some(kind) = session.active_tool.clone() {
                    session.remember_character();
                    if let Some(character) = &mut session.character {
                        if let Some(cell) = character
                            .cells
                            .iter_mut()
                            .find(|cell| cell.coord() == *coord)
                        {
                            cell.kind = kind;
                            session.character_dirty = true;
                        }
                    }
                }
            }
            CreatorAction::AddCell(coord) => {
                session.remember_character();
                let kind = session
                    .active_tool
                    .clone()
                    .unwrap_or(CreationCellKind::Blank);
                if let Some(character) = &mut session.character {
                    character.cells.push(CreationCell {
                        q: coord.q(),
                        r: coord.r(),
                        kind,
                    });
                    session.selected_cell = Some(*coord);
                    session.character_dirty = true;
                }
            }
            CreatorAction::RemoveCell => {
                if let Some(coord) = session.selected_cell {
                    if coord == LatticeCoord::ORIGIN {
                        session.notice = "The origin cell cannot be removed.".to_owned();
                        session.bump();
                        continue;
                    }
                    session.remember_character();
                    if let Some(character) = &mut session.character {
                        character.cells.retain(|cell| cell.coord() != coord);
                        session.selected_cell = character.cells.first().map(CreationCell::coord);
                        session.character_dirty = true;
                    }
                }
            }
            CreatorAction::InspectTool => {
                session.active_tool = None;
                session.erase_tool = false;
            }
            CreatorAction::ChooseTool(kind) => {
                session.active_tool = Some(kind.clone());
                session.erase_tool = false;
            }
            CreatorAction::ChooseErase => {
                session.active_tool = None;
                session.erase_tool = true;
            }
            CreatorAction::Zoom(delta) => {
                session.zoom_step = (session.zoom_step + *delta).clamp(-2, 3);
            }
            CreatorAction::FitLattice => {
                session.zoom_step = 0;
            }
            CreatorAction::AdjustStat {
                element,
                channelling,
                delta,
            } => {
                session.remember_character();
                if let Some(character) = &mut session.character {
                    let map = if *channelling {
                        &mut character.channelling
                    } else {
                        &mut character.attunement
                    };
                    let amount = map.entry(element.clone()).or_default();
                    *amount = adjust_u16(*amount, *delta, 64);
                    session.character_dirty = true;
                }
            }
            CreatorAction::AddRequirement => {
                if let (Some(elements), Some(_)) = (elements.as_deref(), &session.spell) {
                    session.remember_spell();
                    if let Some(spell) = &mut session.spell {
                        let element = elements
                            .wheel()
                            .first()
                            .and_then(|id| elements.name(*id))
                            .unwrap_or("Fire")
                            .to_owned();
                        spell
                            .spell
                            .requirements
                            .push(hex_assets::GemRequirement { element, mana: 1 });
                        session.spell_dirty = true;
                    }
                }
            }
            CreatorAction::RemoveRequirement(index) => {
                session.remember_spell();
                if let Some(spell) = &mut session.spell {
                    if *index < spell.spell.requirements.len() {
                        spell.spell.requirements.remove(*index);
                        session.spell_dirty = true;
                    }
                }
            }
            CreatorAction::CycleRequirement(index) => {
                if let Some(elements) = elements.as_deref() {
                    session.remember_spell();
                    if let Some(spell) = &mut session.spell {
                        if let Some(requirement) = spell.spell.requirements.get_mut(*index) {
                            let names = element_names(elements);
                            if let Some(position) =
                                names.iter().position(|name| *name == requirement.element)
                            {
                                if let Some(next) = names.get((position + 1) % names.len()) {
                                    requirement.element = next.clone();
                                }
                            } else if let Some(first) = names.first() {
                                requirement.element = first.clone();
                            }
                            session.spell_dirty = true;
                        }
                    }
                }
            }
            CreatorAction::AdjustRequirement(index, delta) => {
                session.remember_spell();
                if let Some(spell) = &mut session.spell {
                    if let Some(requirement) = spell.spell.requirements.get_mut(*index) {
                        requirement.mana = adjust_u16(requirement.mana, *delta, 64).max(1);
                        session.spell_dirty = true;
                    }
                }
            }
            CreatorAction::SetEnchantment(enchantment) => {
                session.remember_spell();
                if let Some(saved) = &mut session.spell {
                    saved.spell.casting = if *enchantment {
                        let defense = match saved.spell.casting {
                            hex_assets::CastingAxis::Enchantment { defense } => defense.max(1),
                            hex_assets::CastingAxis::Evocation => 1,
                        };
                        hex_assets::CastingAxis::Enchantment { defense }
                    } else {
                        hex_assets::CastingAxis::Evocation
                    };
                    session.spell_dirty = true;
                }
            }
            CreatorAction::SetSingleTarget(single) => {
                session.remember_spell();
                if let Some(saved) = &mut session.spell {
                    saved.spell.targeting.shape = if *single {
                        TargetShape::Single
                    } else {
                        TargetShape::SelfCast
                    };
                    if *single && saved.spell.targeting.range == 0 {
                        saved.spell.targeting.range = 1;
                    } else if !*single {
                        saved.spell.targeting.range = 0;
                    }
                    session.spell_dirty = true;
                }
            }
            CreatorAction::AdjustRange(delta) => {
                session.remember_spell();
                if let Some(saved) = &mut session.spell {
                    if !matches!(saved.spell.targeting.shape, TargetShape::SelfCast) {
                        saved.spell.targeting.range =
                            adjust_u8(saved.spell.targeting.range, *delta, 16);
                    }
                    session.spell_dirty = true;
                }
            }
            CreatorAction::AdjustDefense(delta) => {
                session.remember_spell();
                if let Some(saved) = &mut session.spell {
                    if let hex_assets::CastingAxis::Enchantment { defense } =
                        &mut saved.spell.casting
                    {
                        *defense = adjust_u16(*defense, *delta, 64);
                        session.spell_dirty = true;
                    }
                }
            }
            CreatorAction::AddEffect(kind) => {
                session.remember_spell();
                if let Some(saved) = &mut session.spell {
                    saved.spell.effects.push(match kind {
                        EffectKind::Disable => Effect::DisableHexes {
                            count: 1,
                            targeted: false,
                        },
                        EffectKind::Burn => Effect::Burn { turns: 1 },
                        EffectKind::Restore => Effect::RestoreHexes { count: 1 },
                        EffectKind::Reveal => Effect::Reveal { tier: 1 },
                    });
                    session.spell_dirty = true;
                }
            }
            CreatorAction::RemoveEffect(index) => {
                session.remember_spell();
                if let Some(saved) = &mut session.spell {
                    if *index < saved.spell.effects.len() {
                        saved.spell.effects.remove(*index);
                        session.spell_dirty = true;
                    }
                }
            }
            CreatorAction::MoveEffect(index, delta) => {
                session.remember_spell();
                if let Some(saved) = &mut session.spell {
                    let other = if *delta < 0 {
                        index.saturating_sub(1)
                    } else {
                        index.saturating_add(1)
                    };
                    if *index < saved.spell.effects.len() && other < saved.spell.effects.len() {
                        saved.spell.effects.swap(*index, other);
                        session.spell_dirty = true;
                    }
                }
            }
            CreatorAction::AdjustEffect(index, delta) => {
                session.remember_spell();
                if let Some(saved) = &mut session.spell {
                    if let Some(effect) = saved.spell.effects.get_mut(*index) {
                        match effect {
                            Effect::DisableHexes { count, .. } | Effect::RestoreHexes { count } => {
                                *count = adjust_u8(*count, *delta, 64).max(1);
                            }
                            Effect::Burn { turns } => {
                                *turns = adjust_u16(*turns, *delta, 64).max(1);
                            }
                            Effect::Reveal { tier } => {
                                *tier = adjust_u8(*tier, *delta, 6).max(1);
                            }
                            _ => {}
                        }
                        session.spell_dirty = true;
                    }
                }
            }
            CreatorAction::Undo => undo(&mut session),
            CreatorAction::Redo => redo(&mut session),
            CreatorAction::DiscardChanges => match session.tab {
                CreatorTab::Characters => {
                    if let Some(current) = &session.character {
                        session.character = store
                            .file
                            .characters
                            .iter()
                            .find(|saved| saved.id == current.id)
                            .cloned()
                            .or_else(|| {
                                Some(SavedCharacter::blank(
                                    CustomCharacterId(store.file.next_character_id.max(1)),
                                    unique_character_name(&store.file, "New Character", None),
                                ))
                            });
                    }
                    session.character_dirty = false;
                }
                CreatorTab::Spells => {
                    if let Some(current) = &session.spell {
                        session.spell = store
                            .file
                            .spells
                            .iter()
                            .find(|saved| saved.id == current.id)
                            .cloned()
                            .or_else(|| {
                                Some(SavedSpell::blank(
                                    CustomSpellId(store.file.next_spell_id.max(1)),
                                    unique_spell_name(
                                        &store.file,
                                        spell_book.as_deref(),
                                        "New Spell",
                                        None,
                                    ),
                                ))
                            });
                    }
                    session.spell_dirty = false;
                }
            },
            CreatorAction::LocalTest => {
                let Some(character) = session.character.as_ref() else {
                    continue;
                };
                match build_local_test(
                    character,
                    &store.file,
                    spell_file.as_deref(),
                    elements.as_deref(),
                    substances.as_deref(),
                ) {
                    Ok(request) => {
                        commands.insert_resource(request);
                        next.set(Screen::LatticeDemo);
                    }
                    Err(error) => session.notice = format!("Local Test blocked: {error}"),
                }
            }
            CreatorAction::TestOnMap => {
                let Some(character) = &session.character else {
                    continue;
                };
                let ready = !session.character_dirty
                    && store
                        .file
                        .characters
                        .iter()
                        .any(|saved| saved.id == character.id)
                    && elements.as_deref().is_some_and(|elements| {
                        spell_book.as_deref().is_some_and(|spells| {
                            character_map_issues(character, &store.file, elements, spells)
                                .is_empty()
                        })
                    });
                if ready {
                    commands.insert_resource(super::combat_lab::CreatorTestRequest {
                        character: character.id,
                    });
                    next.set(Screen::CombatLab);
                } else {
                    session.notice =
                        "Test on Map requires a saved, clean, Map-ready character.".to_owned();
                }
            }
            CreatorAction::ResetLibrary => {
                if session.confirm_reset {
                    match store.reset(&paths) {
                        Ok(()) => {
                            session.confirm_reset = false;
                            session.notice = "Creation library reset.".to_owned();
                        }
                        Err(error) => session.notice = error,
                    }
                } else {
                    session.confirm_reset = true;
                    session.notice =
                        "Press Confirm Reset to replace the unreadable local library.".to_owned();
                }
            }
        }
        session.bump();
    }
}

fn build_local_test(
    character: &SavedCharacter,
    library: &hex_assets::CreationLibraryFile,
    shipped: Option<&SpellFile>,
    elements: Option<&ElementCatalog>,
    substances: Option<&SubstanceTable>,
) -> Result<super::lattice_demo::LocalDemoRequest, String> {
    let (Some(shipped), Some(elements), Some(substances)) = (shipped, elements, substances) else {
        return Err("content catalogs are still loading".to_owned());
    };
    let shipped_book = SpellBook::from_file(shipped);
    let character_issues = creator_character_issues(character, library, elements, &shipped_book);
    if !character_issues.is_empty() {
        return Err(character_issues.join("; "));
    }
    let referenced: BTreeSet<_> = character.custom_spell_references().collect();
    let custom = library
        .spells
        .iter()
        .filter(|spell| referenced.contains(&spell.id))
        .cloned()
        .collect::<Vec<_>>();
    for spell in &custom {
        let issues = spell_map_issues(spell, elements);
        if !issues.is_empty() {
            return Err(format!("{}: {}", spell.name, issues.join("; ")));
        }
    }
    let combined = combined_spell_file(shipped, custom)?;
    let spells = SpellBook::from_file(&combined);
    let index = ContentIndex::build(elements, &spells, substances).map_err(|issues| {
        issues
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ")
    })?;
    let file = character_lattice_file(character, library)?;
    let lattices = LatticeLibrary::build(&file, elements, &spells).map_err(|issues| {
        issues
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ")
    })?;
    let key = character_runtime_key(character.id);
    let archetype = lattices
        .get(&key)
        .ok_or_else(|| "resolved draft is missing".to_owned())?;
    Ok(super::lattice_demo::LocalDemoRequest {
        spec: archetype.spec.clone(),
        stats: archetype.stats.clone(),
        spells,
        index,
        return_to: Screen::CharacterCreator,
    })
}

fn undo(session: &mut CreatorSession) {
    let Some(snapshot) = session.undo.pop() else {
        return;
    };
    match snapshot {
        CreatorSnapshot::Character(previous) => {
            if let Some(current) = session.character.replace(previous) {
                session.redo.push(CreatorSnapshot::Character(current));
            }
            session.character_dirty = true;
        }
        CreatorSnapshot::Spell(previous) => {
            if let Some(current) = session.spell.replace(previous) {
                session.redo.push(CreatorSnapshot::Spell(current));
            }
            session.spell_dirty = true;
        }
    }
}

fn redo(session: &mut CreatorSession) {
    let Some(snapshot) = session.redo.pop() else {
        return;
    };
    match snapshot {
        CreatorSnapshot::Character(next) => {
            if let Some(current) = session.character.replace(next) {
                session.undo.push(CreatorSnapshot::Character(current));
            }
            session.character_dirty = true;
        }
        CreatorSnapshot::Spell(next) => {
            if let Some(current) = session.spell.replace(next) {
                session.undo.push(CreatorSnapshot::Spell(current));
            }
            session.spell_dirty = true;
        }
    }
}

fn spell_map_issues(saved: &SavedSpell, elements: &ElementCatalog) -> Vec<String> {
    let mut issues = creator_spell_issues(saved, elements);
    if let Err(mut combat) = hex_combat::creator_spell_deployability(&saved.spell) {
        issues.append(&mut combat);
    }
    issues.sort();
    issues.dedup();
    issues
}

fn character_map_issues(
    character: &SavedCharacter,
    library: &hex_assets::CreationLibraryFile,
    elements: &ElementCatalog,
    spells: &SpellBook,
) -> Vec<String> {
    let mut issues = creator_character_issues(character, library, elements, spells);
    if !character
        .cells
        .iter()
        .any(|cell| matches!(cell.kind, CreationCellKind::Spell(_)))
    {
        issues.push("at least one inscribed spell is required for map testing".to_owned());
    }
    for cell in &character.cells {
        match &cell.kind {
            CreationCellKind::Spell(SpellReference::Shipped(name)) => {
                if let Some(id) = spells.id(name) {
                    if let Some(spell) = spells.spell(id) {
                        if !matches!(
                            spell.targeting.shape,
                            hex_assets::TargetShape::SelfCast | hex_assets::TargetShape::Single
                        ) {
                            issues.push(format!(
                                "{name}: shipped spell uses an unsupported target shape"
                            ));
                        }
                        if !hex_combat::delivers_anything(spell) {
                            issues.push(format!("{name}: shipped spell has no delivered behavior"));
                        }
                    }
                }
            }
            CreationCellKind::Spell(SpellReference::Custom(id)) => {
                if let Some(spell) = library.spells.iter().find(|spell| spell.id == *id) {
                    issues.extend(
                        spell_map_issues(spell, elements)
                            .into_iter()
                            .map(|issue| format!("{}: {issue}", spell.name)),
                    );
                }
            }
            _ => {}
        }
    }
    issues.sort();
    issues.dedup();
    issues
}

fn element_names(elements: &ElementCatalog) -> Vec<String> {
    (0..elements.len())
        .filter_map(|index| {
            u16::try_from(index)
                .ok()
                .and_then(|id| elements.name(hex_core::ElementId(id)))
                .map(str::to_owned)
        })
        .collect()
}

fn unique_character_name(
    library: &hex_assets::CreationLibraryFile,
    base: &str,
    except: Option<CustomCharacterId>,
) -> String {
    unique_name(base, |candidate| {
        library.characters.iter().any(|saved| {
            Some(saved.id) != except && normalized_name(&saved.name) == normalized_name(candidate)
        })
    })
}

fn unique_spell_name(
    library: &hex_assets::CreationLibraryFile,
    shipped: Option<&SpellBook>,
    base: &str,
    except: Option<CustomSpellId>,
) -> String {
    unique_name(base, |candidate| {
        shipped.and_then(|book| book.id(candidate)).is_some()
            || library.spells.iter().any(|saved| {
                Some(saved.id) != except
                    && normalized_name(&saved.name) == normalized_name(candidate)
            })
    })
}

fn unique_name(base: &str, collision: impl Fn(&str) -> bool) -> String {
    let trimmed: String = base.chars().take(MAX_CREATION_NAME_CHARS).collect();
    if !collision(&trimmed) {
        return trimmed;
    }
    for suffix in 2..10_000 {
        let tail = format!(" {suffix}");
        let keep = MAX_CREATION_NAME_CHARS.saturating_sub(tail.chars().count());
        let head: String = base.chars().take(keep).collect();
        let candidate = format!("{head}{tail}");
        if !collision(&candidate) {
            return candidate;
        }
    }
    "Untitled".to_owned()
}

fn adjust_u16(value: u16, delta: i8, max: u16) -> u16 {
    if delta < 0 {
        value.saturating_sub(delta.unsigned_abs().into())
    } else {
        value.saturating_add(delta.unsigned_abs().into()).min(max)
    }
}

fn adjust_u8(value: u8, delta: i8, max: u8) -> u8 {
    if delta < 0 {
        value.saturating_sub(delta.unsigned_abs())
    } else {
        value.saturating_add(delta.unsigned_abs()).min(max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_names_are_case_insensitively_unique_and_bounded() {
        let mut library = hex_assets::CreationLibraryFile::default();
        library
            .characters
            .push(SavedCharacter::blank(CustomCharacterId(1), "New Character"));
        let name = unique_character_name(&library, "new character", None);
        assert_eq!(name, "new character 2");
        assert!(name.chars().count() <= MAX_CREATION_NAME_CHARS);
    }

    #[test]
    fn arbitrary_effect_lists_can_be_reordered_without_a_count_limit() {
        let mut spell = SavedSpell::blank(CustomSpellId(1), "Many");
        for _ in 0..100 {
            spell.spell.effects.push(Effect::Burn { turns: 1 });
        }
        spell.spell.effects.swap(0, 99);
        assert_eq!(spell.spell.effects.len(), 100);
    }

    #[test]
    fn spell_management_preserves_a_combat_lab_return_route() {
        let mut session = CreatorSession {
            return_to_combat_lab: true,
            ..default()
        };
        apply_entry_request(&mut session, CreatorEntryRequest::SpellFromCharacter);
        assert!(session.return_to_combat_lab);
        assert!(session.return_to_character_creator);
        assert_eq!(session.tab, CreatorTab::Spells);

        apply_entry_request(&mut session, CreatorEntryRequest::SpellLibrary);
        assert!(!session.return_to_combat_lab);
        assert!(!session.return_to_character_creator);
    }
}
