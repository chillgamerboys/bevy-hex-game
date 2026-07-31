//! Saved character and spell authoring.
//!
//! The screen edits name-based drafts and writes only through `CreationStore`.
//! Runtime ids are deliberately absent here.

use std::collections::BTreeSet;
use std::ops::{Deref, DerefMut};

use bevy::prelude::*;
use hex_assets::{
    character_lattice_file, character_runtime_key, combined_spell_file, creator_character_issues,
    creator_spell_issues, normalized_name, ContentIndex, CreationCell, CreationCellKind,
    CreationPresetCatalog, CustomCharacterId, CustomSpellId, Effect, ElementCatalog, LatticeFile,
    LatticeLibrary, SavedCharacter, SavedSpell, SpellBook, SpellFile, SpellReference,
    SubstanceTable, TargetShape, MAX_CREATION_NAME_CHARS,
};
use hex_core::{LatticeCoord, Screen};
use hex_gameplay_model::{
    CreatorDestination, CreatorEntry, CreatorNavigation, CreatorSurface as CreatorTab, EditHistory,
};

use crate::creation_store::CreationStore;
use crate::storage::StoragePaths;
use hex_ui::{
    CreatorEffectKind as EffectKind, CreatorIntent as CreatorAction, CreatorLibraryView,
    CreatorNameField, CreatorScreenView, CreatorWorkspace,
};

use super::despawn_screen;

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
    navigation: CreatorNavigation,
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
    history: EditHistory<CreatorSnapshot>,
    revision: u64,
}

impl Deref for CreatorSession {
    type Target = CreatorNavigation;

    fn deref(&self) -> &Self::Target {
        &self.navigation
    }
}

impl DerefMut for CreatorSession {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.navigation
    }
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
            self.history
                .remember(CreatorSnapshot::Character(character.clone()));
        }
    }

    fn remember_spell(&mut self) {
        if let Some(spell) = &self.spell {
            self.history.remember(CreatorSnapshot::Spell(spell.clone()));
        }
    }
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<CreatorSession>()
        .add_systems(
            OnEnter(Screen::CharacterCreator),
            (initialize_session, publish_creator_view).chain(),
        )
        .add_systems(
            OnEnter(Screen::SpellCreator),
            (initialize_session, publish_creator_view).chain(),
        )
        .add_systems(
            Update,
            (
                handle_actions.after(hex_ui::UiSystems::EmitIntents),
                publish_creator_view,
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
    let entry = match request {
        CreatorEntryRequest::CharacterLibrary => {
            session.view = CreatorView::Hub;
            CreatorEntry::CharacterLibrary
        }
        CreatorEntryRequest::SpellLibrary => {
            session.view = CreatorView::Hub;
            CreatorEntry::SpellLibrary
        }
        CreatorEntryRequest::SpellFromCharacter => {
            session.view = CreatorView::Hub;
            CreatorEntry::SpellFromCharacter
        }
    };
    session.navigation.enter(entry);
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
        let destination = session.navigation.back();
        if destination == CreatorDestination::CharacterEditor {
            session.view = CreatorView::Character;
            next.set(Screen::CharacterCreator);
        } else {
            next.set(match destination {
                CreatorDestination::CombatLab => Screen::CombatLab,
                CreatorDestination::Title => Screen::Title,
                CreatorDestination::CharacterEditor => Screen::CharacterCreator,
            });
        }
    }
}

fn publish_creator_view(
    session: Res<CreatorSession>,
    store: Res<CreationStore>,
    elements: Option<Res<ElementCatalog>>,
    spell_book: Option<Res<SpellBook>>,
    spell_file: Option<Res<SpellFile>>,
    lattice_file: Option<Res<LatticeFile>>,
    presets: Option<Res<CreationPresetCatalog>>,
    mut view: ResMut<CreatorScreenView>,
    mut last_revision: Local<u64>,
) {
    let content_changed = elements.as_ref().is_some_and(|value| value.is_changed())
        || spell_book.as_ref().is_some_and(|value| value.is_changed())
        || spell_file.as_ref().is_some_and(|value| value.is_changed())
        || lattice_file
            .as_ref()
            .is_some_and(|value| value.is_changed())
        || presets.as_ref().is_some_and(|value| value.is_changed());
    if *last_revision == session.revision && !store.is_changed() && !content_changed {
        return;
    }
    let character_issues = session
        .character
        .as_ref()
        .map_or_else(Vec::new, |character| {
            match (elements.as_deref(), spell_book.as_deref()) {
                (Some(elements), Some(spells)) => {
                    character_map_issues(character, &store.file, elements, spells)
                }
                _ => vec!["content catalogs are still loading".to_owned()],
            }
        });
    let spell_issues = session.spell.as_ref().map_or_else(Vec::new, |spell| {
        elements.as_deref().map_or_else(
            || vec!["element catalog is still loading".to_owned()],
            |elements| spell_map_issues(spell, elements),
        )
    });
    let deployable_shipped_spells = spell_book.as_deref().map_or_else(Vec::new, |spells| {
        spells
            .iter()
            .filter(|(_, _, spell)| {
                matches!(
                    spell.targeting.shape,
                    TargetShape::SelfCast | TargetShape::Single
                ) && hex_combat::delivers_anything(spell)
            })
            .map(|(_, name, _)| name.to_owned())
            .collect()
    });
    let deployable_custom_spells = elements.as_deref().map_or_else(Vec::new, |elements| {
        store
            .file
            .spells
            .iter()
            .filter(|spell| {
                creator_spell_issues(spell, elements).is_empty()
                    && hex_combat::creator_spell_deployability(&spell.spell).is_ok()
            })
            .map(|spell| spell.id)
            .collect()
    });
    *view = CreatorScreenView {
        active: true,
        screen: match session.tab {
            CreatorTab::Characters => Screen::CharacterCreator,
            CreatorTab::Spells => Screen::SpellCreator,
        },
        tab: session.tab,
        workspace: match session.view {
            CreatorView::Hub => CreatorWorkspace::Hub,
            CreatorView::Character => CreatorWorkspace::Character,
            CreatorView::Spell => CreatorWorkspace::Spell,
        },
        character: session.character.clone(),
        spell: session.spell.clone(),
        selected_cell: session.selected_cell,
        active_tool: session.active_tool.clone(),
        erase_tool: session.erase_tool,
        zoom_step: session.zoom_step,
        character_dirty: session.character_dirty,
        spell_dirty: session.spell_dirty,
        notice: session.notice.clone(),
        confirm_delete: session.confirm_delete,
        confirm_reset: session.confirm_reset,
        library: CreatorLibraryView {
            file: store.file.clone(),
            error: store.error.clone(),
        },
        elements: elements.as_deref().cloned(),
        spell_book: spell_book.as_deref().cloned(),
        spell_file: spell_file.as_deref().cloned(),
        lattice_file: lattice_file.as_deref().cloned(),
        presets: presets.as_deref().cloned(),
        character_issues,
        spell_issues,
        deployable_shipped_spells,
        deployable_custom_spells,
    };
    *last_revision = session.revision;
}

fn handle_actions(
    mut intents: MessageReader<hex_ui::UiIntent>,
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
    for intent in intents.read() {
        let hex_ui::UiIntent::Creator(action) = intent else {
            continue;
        };
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
                } else {
                    let destination = session.navigation.back();
                    if destination == CreatorDestination::CharacterEditor {
                        session.view = CreatorView::Character;
                        next.set(Screen::CharacterCreator);
                    } else {
                        next.set(match destination {
                            CreatorDestination::CombatLab => Screen::CombatLab,
                            CreatorDestination::Title => Screen::Title,
                            CreatorDestination::CharacterEditor => Screen::CharacterCreator,
                        });
                    }
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
            CreatorAction::SetName(field, value) => {
                let changed = match field {
                    CreatorNameField::Character => session
                        .character
                        .as_ref()
                        .is_some_and(|character| character.name != *value),
                    CreatorNameField::Spell => session
                        .spell
                        .as_ref()
                        .is_some_and(|spell| spell.name != *value),
                };
                if !changed {
                    continue;
                }
                match field {
                    CreatorNameField::Character => {
                        if let Some(character) = &mut session.character {
                            character.name.clone_from(value);
                            session.character_dirty = true;
                        }
                    }
                    CreatorNameField::Spell => {
                        if let Some(spell) = &mut session.spell {
                            spell.name.clone_from(value);
                            session.spell_dirty = true;
                        }
                    }
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
    let character = session.character.clone();
    let spell = session.spell.clone();
    let Some(snapshot) = session.history.try_undo(|previous| match previous {
        CreatorSnapshot::Character(_) => character.map(CreatorSnapshot::Character),
        CreatorSnapshot::Spell(_) => spell.map(CreatorSnapshot::Spell),
    }) else {
        return;
    };
    match snapshot {
        CreatorSnapshot::Character(previous) => {
            session.character = Some(previous);
            session.character_dirty = true;
        }
        CreatorSnapshot::Spell(previous) => {
            session.spell = Some(previous);
            session.spell_dirty = true;
        }
    }
}

fn redo(session: &mut CreatorSession) {
    let character = session.character.clone();
    let spell = session.spell.clone();
    let Some(snapshot) = session.history.try_redo(|next| match next {
        CreatorSnapshot::Character(_) => character.map(CreatorSnapshot::Character),
        CreatorSnapshot::Spell(_) => spell.map(CreatorSnapshot::Spell),
    }) else {
        return;
    };
    match snapshot {
        CreatorSnapshot::Character(next) => {
            session.character = Some(next);
            session.character_dirty = true;
        }
        CreatorSnapshot::Spell(next) => {
            session.spell = Some(next);
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
        let mut session = CreatorSession::default();
        session.navigation.return_to_combat_lab = true;
        apply_entry_request(&mut session, CreatorEntryRequest::SpellFromCharacter);
        assert!(session.return_to_combat_lab);
        assert!(session.return_to_character_creator);
        assert_eq!(session.tab, CreatorTab::Spells);

        apply_entry_request(&mut session, CreatorEntryRequest::SpellLibrary);
        assert!(!session.return_to_combat_lab);
        assert!(!session.return_to_character_creator);
    }
}
