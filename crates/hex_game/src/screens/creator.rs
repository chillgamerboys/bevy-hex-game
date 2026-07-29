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
    SpellReference, SubstanceTable, TargetShape, UnvalidatedCell, MAX_CREATION_NAME_CHARS,
};
use hex_core::{LatticeCoord, Screen};

use crate::creation_store::CreationStore;
use crate::menus::widgets::{
    blurb, display, fine, heading, label, panel, panel_node, row_button, UiAssets, ACCENT,
    ACCENT_EDGE, DANGER, EDGE, LABEL,
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

#[derive(Debug, Clone)]
enum CreatorSnapshot {
    Character(SavedCharacter),
    Spell(SavedSpell),
}

#[derive(Resource, Debug, Default)]
pub(crate) struct CreatorSession {
    tab: CreatorTab,
    character: Option<SavedCharacter>,
    spell: Option<SavedSpell>,
    selected_cell: Option<LatticeCoord>,
    character_dirty: bool,
    spell_dirty: bool,
    notice: String,
    confirm_delete: bool,
    confirm_reset: bool,
    undo: Vec<CreatorSnapshot>,
    redo: Vec<CreatorSnapshot>,
    revision: u64,
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
    Tab(CreatorTab),
    Back,
    NewCharacter,
    NewSpell,
    SelectCharacter(CustomCharacterId),
    SelectSpell(CustomSpellId),
    DuplicateCharacter,
    DuplicateSpell,
    DuplicateCharacterTemplate(String),
    DuplicateSpellTemplate(String),
    DuplicatePackagedCharacter(String),
    DuplicatePackagedSpell(String),
    SaveCharacter,
    SaveSpell,
    DeleteCharacter,
    DeleteSpell,
    SelectCell(LatticeCoord),
    AddCell(LatticeCoord),
    RemoveCell,
    SetCell(CreationCellKind),
    AdjustStat {
        element: String,
        channelling: bool,
        delta: i8,
    },
    AddRequirement,
    RemoveRequirement(usize),
    CycleRequirement(usize),
    AdjustRequirement(usize, i8),
    ToggleCasting,
    ToggleTarget,
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
            Update,
            (
                sync_name_fields,
                handle_actions,
                rebuild_when_requested,
                handle_escape,
            )
                .chain()
                .run_if(in_state(Screen::CharacterCreator)),
        )
        .add_systems(
            OnExit(Screen::CharacterCreator),
            despawn_screen(Screen::CharacterCreator),
        );
}

fn initialize_session(
    mut commands: Commands,
    mut session: ResMut<CreatorSession>,
    store: Res<CreationStore>,
    spell_file: Option<Res<SpellFile>>,
    lattice_file: Option<Res<LatticeFile>>,
    elements: Option<Res<ElementCatalog>>,
    substances: Option<Res<SubstanceTable>>,
) {
    super::combat_lab::restore_shipped_content(
        &mut commands,
        spell_file.as_deref(),
        lattice_file.as_deref(),
        elements.as_deref(),
        substances.as_deref(),
    );
    commands.remove_resource::<super::combat_lab::CombatLabSession>();
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
    session.notice = store.error.clone().unwrap_or_default();
    session.confirm_delete = false;
    session.confirm_reset = false;
    session.bump();
}

fn handle_escape(
    keys: Res<ButtonInput<KeyCode>>,
    mut next: ResMut<NextState<Screen>>,
    session: Res<CreatorSession>,
) {
    if keys.just_pressed(KeyCode::Escape) && !session.character_dirty && !session.spell_dirty {
        next.set(Screen::Title);
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
    commands
        .spawn((
            screen_root(
                Screen::CharacterCreator,
                "Character and Spell Creator Screen",
            ),
            CreatorRoot,
        ))
        .insert(Node {
            padding: UiRect::all(Val::Px(18.0)),
            row_gap: Val::Px(10.0),
            ..screen_root_node()
        })
        .with_children(|root| {
            root.spawn(display(assets, "Character & Spell Creator"));
            root.spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(8.0),
                ..default()
            })
            .with_children(|tabs| {
                action_button(
                    tabs,
                    assets,
                    "Characters",
                    CreatorAction::Tab(CreatorTab::Characters),
                    170.0,
                );
                action_button(
                    tabs,
                    assets,
                    "Spells",
                    CreatorAction::Tab(CreatorTab::Spells),
                    170.0,
                );
                action_button(tabs, assets, "Undo", CreatorAction::Undo, 100.0);
                action_button(tabs, assets, "Redo", CreatorAction::Redo, 100.0);
                if store.error.is_some() {
                    action_button(
                        tabs,
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
                if session.character_dirty || session.spell_dirty {
                    action_button(
                        tabs,
                        assets,
                        "Discard Changes",
                        CreatorAction::DiscardChanges,
                        160.0,
                    );
                }
                action_button(tabs, assets, "Back", CreatorAction::Back, 100.0);
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
            .with_children(|body| match session.tab {
                CreatorTab::Characters => spawn_character_tab(
                    body,
                    assets,
                    session,
                    store,
                    elements,
                    spell_book,
                    lattice_file,
                    presets,
                ),
                CreatorTab::Spells => spawn_spell_tab(
                    body, assets, session, store, elements, spell_book, spell_file, presets,
                ),
            });
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
    lattice_file: Option<&LatticeFile>,
    presets: Option<&CreationPresetCatalog>,
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
            width: Val::Px(270.0),
            min_height: Val::Px(0.0),
            ..panel_node()
        })
        .with_children(|left| {
            left.spawn(heading(assets, "characters"));
            action_button(
                left,
                assets,
                "New Blank",
                CreatorAction::NewCharacter,
                220.0,
            );
            if let Some(presets) = presets {
                for record in presets
                    .characters
                    .iter()
                    .filter(|record| record.audience == PresetAudience::HumanTemplate)
                {
                    action_button(
                        left,
                        assets,
                        format!("Template: {}", record.character.name),
                        CreatorAction::DuplicatePackagedCharacter(record.key.clone()),
                        220.0,
                    );
                }
            } else if let Some(file) = lattice_file {
                for name in ["wolf", "raider", "hedge-mage"] {
                    if file.archetypes.contains_key(name) {
                        action_button(
                            left,
                            assets,
                            format!("Template: {name}"),
                            CreatorAction::DuplicateCharacterTemplate(name.to_owned()),
                            220.0,
                        );
                    }
                }
            }
            left.spawn((
                ScrollArea,
                Node {
                    width: Val::Percent(100.0),
                    min_height: Val::Px(0.0),
                    flex_grow: 1.0,
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(6.0),
                    overflow: Overflow::scroll_y(),
                    ..default()
                },
            ))
            .with_children(|list| {
                for saved in &store.file.characters {
                    action_button(
                        list,
                        assets,
                        saved.name.clone(),
                        CreatorAction::SelectCharacter(saved.id),
                        210.0,
                    );
                }
            });
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
                            spawn_lattice_cells(surface, assets, character, session.selected_cell);
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
                right.spawn(label(assets, format!("({}, {})", coord.q(), coord.r())));
                action_button(
                    right,
                    assets,
                    "Blank",
                    CreatorAction::SetCell(CreationCellKind::Blank),
                    270.0,
                );
                if let Some(elements) = elements {
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
                        action_button(
                            right,
                            assets,
                            if elements.is_higher_order(id) {
                                format!("Fusion: {name}")
                            } else {
                                format!("Gem: {name}")
                            },
                            CreatorAction::SetCell(kind),
                            270.0,
                        );
                    }
                }
                if let Some(spells) = spell_book {
                    for (_, name, _) in spells.iter() {
                        action_button(
                            right,
                            assets,
                            format!("Spell: {name}"),
                            CreatorAction::SetCell(CreationCellKind::Spell(
                                SpellReference::Shipped(name.to_owned()),
                            )),
                            270.0,
                        );
                    }
                }
                if let Some(elements) = elements {
                    for spell in &store.file.spells {
                        if creator_spell_issues(spell, elements).is_empty()
                            && hex_combat::creator_spell_deployability(&spell.spell).is_ok()
                        {
                            action_button(
                                right,
                                assets,
                                format!("Custom: {}", spell.name),
                                CreatorAction::SetCell(CreationCellKind::Spell(
                                    SpellReference::Custom(spell.id),
                                )),
                                270.0,
                            );
                        }
                    }
                }
                action_button(
                    right,
                    assets,
                    "Remove Cell",
                    CreatorAction::RemoveCell,
                    270.0,
                );
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
                for issue in issues {
                    right
                        .spawn(fine(assets, format!("• {issue}")))
                        .insert(TextColor(DANGER));
                }
            }
        });
}

fn spawn_lattice_cells(
    surface: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    character: &SavedCharacter,
    selected: Option<LatticeCoord>,
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
    for cell in &character.cells {
        let coord = cell.coord();
        let (left, top) = lattice_pixel(coord);
        surface
            .spawn((
                Name::new(format!("Creator Cell {},{}", coord.q(), coord.r())),
                Button,
                CreatorAction::SelectCell(coord),
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(left),
                    top: Val::Px(top),
                    width: Val::Px(72.0),
                    height: Val::Px(72.0),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    border: UiRect::all(Val::Px(if selected == Some(coord) { 3.0 } else { 1.0 })),
                    ..default()
                },
                BorderColor::all(if selected == Some(coord) {
                    ACCENT
                } else {
                    EDGE
                }),
                BackgroundColor(cell_color(&cell.kind)),
            ))
            .with_child(fine(assets, cell_label(&cell.kind)));
    }
    if character.cells.len() < hex_assets::MAX_CREATION_CELLS {
        for coord in additions {
            let (left, top) = lattice_pixel(coord);
            surface
                .spawn((
                    Name::new(format!("Add Cell {},{}", coord.q(), coord.r())),
                    Button,
                    CreatorAction::AddCell(coord),
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(left + 11.0),
                        top: Val::Px(top + 11.0),
                        width: Val::Px(50.0),
                        height: Val::Px(50.0),
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        border: UiRect::all(Val::Px(1.0)),
                        ..default()
                    },
                    BorderColor::all(EDGE),
                    BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.03)),
                ))
                .with_child(label(assets, "+"));
        }
    }
}

#[expect(
    clippy::cast_precision_loss,
    reason = "creator coordinates are capped to 64 cells"
)]
fn lattice_pixel(coord: LatticeCoord) -> (f32, f32) {
    (
        500.0 + coord.q() as f32 * 78.0 + coord.r() as f32 * 39.0,
        330.0 + coord.r() as f32 * 68.0,
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

fn cell_color(kind: &CreationCellKind) -> Color {
    match kind {
        CreationCellKind::Gem(_) => Color::srgba(0.16, 0.45, 0.52, 0.92),
        CreationCellKind::Fusion(_) => Color::srgba(0.42, 0.30, 0.62, 0.92),
        CreationCellKind::Spell(_) => Color::srgba(0.55, 0.34, 0.12, 0.94),
        CreationCellKind::Blank => Color::srgba(0.28, 0.29, 0.32, 0.9),
    }
}

fn spawn_spell_tab(
    body: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    session: &CreatorSession,
    store: &CreationStore,
    elements: Option<&ElementCatalog>,
    spell_book: Option<&SpellBook>,
    spell_file: Option<&SpellFile>,
    presets: Option<&CreationPresetCatalog>,
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
            width: Val::Px(280.0),
            min_height: Val::Px(0.0),
            ..panel_node()
        })
        .with_children(|left| {
            left.spawn(heading(assets, "spells"));
            action_button(left, assets, "New Blank", CreatorAction::NewSpell, 220.0);
            if let Some(presets) = presets {
                for record in presets
                    .spells
                    .iter()
                    .filter(|record| record.audience == PresetAudience::HumanTemplate)
                {
                    action_button(
                        left,
                        assets,
                        format!("Template: {}", record.spell.name),
                        CreatorAction::DuplicatePackagedSpell(record.key.clone()),
                        220.0,
                    );
                }
            } else if let Some(file) = spell_file {
                let mut names: Vec<_> = file.spells.keys().cloned().collect();
                names.sort();
                for name in names {
                    action_button(
                        left,
                        assets,
                        format!("Template: {name}"),
                        CreatorAction::DuplicateSpellTemplate(name),
                        220.0,
                    );
                }
            }
            left.spawn((
                ScrollArea,
                Node {
                    width: Val::Percent(100.0),
                    min_height: Val::Px(0.0),
                    flex_grow: 1.0,
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(6.0),
                    overflow: Overflow::scroll_y(),
                    ..default()
                },
            ))
            .with_children(|list| {
                for spell in &store.file.spells {
                    let ready = elements
                        .is_some_and(|elements| spell_map_issues(spell, elements).is_empty());
                    action_button(
                        list,
                        assets,
                        format!("{} · {}", spell.name, if ready { "Ready" } else { "Draft" }),
                        CreatorAction::SelectSpell(spell.id),
                        220.0,
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
            form.spawn(heading(assets, "requirements"));
            for (index, requirement) in saved.spell.requirements.iter().enumerate() {
                form.spawn(fine(
                    assets,
                    format!(
                        "{}. {} × {}",
                        index + 1,
                        requirement.element,
                        requirement.mana
                    ),
                ));
                form.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(5.0),
                    ..default()
                })
                .with_children(|row| {
                    action_button(
                        row,
                        assets,
                        "Element",
                        CreatorAction::CycleRequirement(index),
                        95.0,
                    );
                    action_button(
                        row,
                        assets,
                        "Mana −",
                        CreatorAction::AdjustRequirement(index, -1),
                        82.0,
                    );
                    action_button(
                        row,
                        assets,
                        "Mana +",
                        CreatorAction::AdjustRequirement(index, 1),
                        82.0,
                    );
                    action_button(
                        row,
                        assets,
                        "Remove",
                        CreatorAction::RemoveRequirement(index),
                        82.0,
                    );
                });
            }
            if saved.spell.requirements.len() < 6 {
                action_button(
                    form,
                    assets,
                    "Add Requirement",
                    CreatorAction::AddRequirement,
                    180.0,
                );
            }
            form.spawn(heading(assets, "casting and targeting"));
            form.spawn(blurb(
                assets,
                format!(
                    "{} · {} · range {}",
                    match saved.spell.casting {
                        hex_assets::CastingAxis::Evocation => "Evocation".to_owned(),
                        hex_assets::CastingAxis::Enchantment { defense } =>
                            format!("Enchantment (defense {defense})"),
                    },
                    match saved.spell.targeting.shape {
                        TargetShape::SelfCast => "Self",
                        _ => "Single",
                    },
                    saved.spell.targeting.range
                ),
            ));
            form.spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(5.0),
                ..default()
            })
            .with_children(|row| {
                action_button(row, assets, "Axis", CreatorAction::ToggleCasting, 85.0);
                action_button(row, assets, "Target", CreatorAction::ToggleTarget, 85.0);
                action_button(row, assets, "Range −", CreatorAction::AdjustRange(-1), 85.0);
                action_button(row, assets, "Range +", CreatorAction::AdjustRange(1), 85.0);
                action_button(
                    row,
                    assets,
                    "Defense −",
                    CreatorAction::AdjustDefense(-1),
                    95.0,
                );
                action_button(
                    row,
                    assets,
                    "Defense +",
                    CreatorAction::AdjustDefense(1),
                    95.0,
                );
            });
            form.spawn(heading(assets, "effects"));
            for (index, effect) in saved.spell.effects.iter().enumerate() {
                form.spawn(fine(assets, format!("{}. {effect:?}", index + 1)));
                form.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(5.0),
                    ..default()
                })
                .with_children(|row| {
                    action_button(row, assets, "↑", CreatorAction::MoveEffect(index, -1), 48.0);
                    action_button(row, assets, "↓", CreatorAction::MoveEffect(index, 1), 48.0);
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
                        90.0,
                    );
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
            right.spawn(heading(
                assets,
                if issues.is_empty() { "Ready" } else { "Draft" },
            ));
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
    lattice_file: Option<Res<LatticeFile>>,
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
            CreatorAction::Tab(tab) => {
                session.tab = *tab;
                session.confirm_delete = false;
            }
            CreatorAction::Back => {
                if session.character_dirty || session.spell_dirty {
                    session.notice = "Save or discard the current edits before leaving.".to_owned();
                } else {
                    next.set(Screen::Title);
                }
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
                }
            }
            CreatorAction::SelectSpell(id) => {
                if session.spell_dirty {
                    session.notice = "Save the current spell before switching.".to_owned();
                } else if let Some(saved) = store.file.spells.iter().find(|saved| saved.id == *id) {
                    session.spell = Some(saved.clone());
                    session.confirm_delete = false;
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
                }
            }
            CreatorAction::DuplicateCharacterTemplate(name) => {
                if let Some(file) = lattice_file.as_deref() {
                    if let Some(raw) = file.archetypes.get(name) {
                        let id = store.file.allocate_character_id();
                        session.character = Some(character_from_template(
                            id,
                            unique_character_name(&store.file, &format!("{name} Copy"), None),
                            raw,
                        ));
                        session.character_dirty = true;
                        session.selected_cell = Some(LatticeCoord::ORIGIN);
                    }
                }
            }
            CreatorAction::DuplicateSpellTemplate(name) => {
                if let Some(file) = spell_file.as_deref() {
                    if let Some(spell) = file.spells.get(name) {
                        let id = store.file.allocate_spell_id();
                        session.spell = Some(SavedSpell {
                            id,
                            name: unique_spell_name(
                                &store.file,
                                spell_book.as_deref(),
                                &format!("{name} Copy"),
                                None,
                            ),
                            spell: spell.clone(),
                        });
                        session.spell_dirty = true;
                    }
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
                        }
                        Err(error) => {
                            session.notice = format!("Spell cannot be deleted: {error}");
                            session.confirm_delete = false;
                        }
                    }
                }
            }
            CreatorAction::SelectCell(coord) => session.selected_cell = Some(*coord),
            CreatorAction::AddCell(coord) => {
                session.remember_character();
                if let Some(character) = &mut session.character {
                    character.cells.push(CreationCell {
                        q: coord.q(),
                        r: coord.r(),
                        kind: CreationCellKind::Blank,
                    });
                    session.selected_cell = Some(*coord);
                    session.character_dirty = true;
                }
            }
            CreatorAction::RemoveCell => {
                if let Some(coord) = session.selected_cell {
                    session.remember_character();
                    if let Some(character) = &mut session.character {
                        character.cells.retain(|cell| cell.coord() != coord);
                        session.selected_cell = character.cells.first().map(CreationCell::coord);
                        session.character_dirty = true;
                    }
                }
            }
            CreatorAction::SetCell(kind) => {
                if let Some(coord) = session.selected_cell {
                    session.remember_character();
                    if let Some(character) = &mut session.character {
                        if let Some(cell) = character
                            .cells
                            .iter_mut()
                            .find(|cell| cell.coord() == coord)
                        {
                            cell.kind = kind.clone();
                            session.character_dirty = true;
                        }
                    }
                }
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
            CreatorAction::ToggleCasting => {
                session.remember_spell();
                if let Some(saved) = &mut session.spell {
                    saved.spell.casting = match saved.spell.casting {
                        hex_assets::CastingAxis::Evocation => {
                            hex_assets::CastingAxis::Enchantment { defense: 1 }
                        }
                        hex_assets::CastingAxis::Enchantment { .. } => {
                            hex_assets::CastingAxis::Evocation
                        }
                    };
                    session.spell_dirty = true;
                }
            }
            CreatorAction::ToggleTarget => {
                session.remember_spell();
                if let Some(saved) = &mut session.spell {
                    saved.spell.targeting.shape = match saved.spell.targeting.shape {
                        TargetShape::SelfCast => TargetShape::Single,
                        _ => TargetShape::SelfCast,
                    };
                    if matches!(saved.spell.targeting.shape, TargetShape::SelfCast) {
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

fn character_from_template(
    id: CustomCharacterId,
    name: String,
    raw: &hex_assets::UnvalidatedArchetype,
) -> SavedCharacter {
    SavedCharacter {
        id,
        name,
        cells: raw
            .cells
            .iter()
            .map(|cell| CreationCell {
                q: cell.at.q,
                r: cell.at.r,
                kind: match &cell.kind {
                    UnvalidatedCell::Gem(name) => CreationCellKind::Gem(name.clone()),
                    UnvalidatedCell::Fusion(name) => CreationCellKind::Fusion(name.clone()),
                    UnvalidatedCell::Spell(name) => {
                        CreationCellKind::Spell(SpellReference::Shipped(name.clone()))
                    }
                    UnvalidatedCell::Blank => CreationCellKind::Blank,
                },
            })
            .collect(),
        attunement: raw.attunement.clone(),
        channelling: raw.channelling.clone(),
    }
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
}
