//! Character and spell Creator presentation from an immutable application projection.

use std::collections::BTreeSet;

use bevy::input_focus::tab_navigation::TabIndex;
use bevy::prelude::*;
use bevy::text::EditableText;
use bevy::ui_widgets::ScrollArea;
use hex_assets::{
    CreationCell, CreationCellKind, CreationPresetCatalog, Effect, ElementCatalog, LatticeFile,
    PresetAudience, SavedCharacter, SpellBook, SpellFile, SpellReference, TargetShape,
    MAX_CREATION_NAME_CHARS,
};
use hex_core::{LatticeCoord, Screen};
use hex_gameplay_model::CreatorSurface as CreatorTab;

use crate::{
    blurb, body_text_role, compact_glyph_role, display, effect_summary, element_color, fine,
    heading, label, owner_resolved_control_role, panel, panel_node, responsive_control_role,
    row_button, screen_root, short_name, CharacterBuildSummary, CreatorEffectKind, CreatorIntent,
    CreatorLibraryView, CreatorNameField, CreatorScreenView, CreatorWorkspace, OwnColors,
    ResolvedUiMetrics, SpellBuildSummary, UiAssets, UiIntent, UiSystems, UiViewportClass, ACCENT,
    ACCENT_EDGE, DANGER, EDGE, FUSION_COLOR, LABEL,
};

#[derive(Component)]
struct CreatorRoot;

#[derive(Component)]
struct CreatorHeader;

#[derive(Component)]
struct CreatorHeaderActions;

#[derive(Component)]
struct CreatorResponsiveBody;

#[derive(Component, Clone, Copy)]
enum CreatorBodyPanel {
    Sidebar { width: f32, compact_row: i16 },
    Main,
}

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        (
            (render, apply_creator_layout)
                .chain()
                .in_set(UiSystems::Render),
            emit_actions.in_set(UiSystems::EmitIntents),
            emit_name_changes.in_set(UiSystems::EmitIntents),
        )
            .run_if(creator_screen_active),
    );
}

fn apply_creator_layout(
    mut commands: Commands,
    metrics: Res<ResolvedUiMetrics>,
    added_bodies: Query<(), Added<CreatorResponsiveBody>>,
    mut roots: Query<&mut Node, With<CreatorRoot>>,
    mut headers: Query<&mut Node, (With<CreatorHeader>, Without<CreatorRoot>)>,
    mut header_actions: Query<
        &mut Node,
        (
            With<CreatorHeaderActions>,
            Without<CreatorHeader>,
            Without<CreatorRoot>,
        ),
    >,
    mut bodies: Query<
        &mut Node,
        (
            With<CreatorResponsiveBody>,
            Without<CreatorHeader>,
            Without<CreatorHeaderActions>,
            Without<CreatorRoot>,
        ),
    >,
    mut panels: Query<
        (Entity, &CreatorBodyPanel, &mut Node),
        (
            Without<CreatorResponsiveBody>,
            Without<CreatorHeader>,
            Without<CreatorHeaderActions>,
            Without<CreatorRoot>,
        ),
    >,
) {
    if !metrics.is_changed() && added_bodies.is_empty() {
        return;
    }
    let compact = metrics.viewport == UiViewportClass::Compact;
    for mut node in &mut roots {
        node.overflow = if compact {
            Overflow::scroll_y()
        } else {
            Overflow::clip_y()
        };
    }
    for mut node in &mut headers {
        node.flex_direction = if compact {
            FlexDirection::Column
        } else {
            FlexDirection::Row
        };
        node.align_items = if compact {
            AlignItems::Stretch
        } else {
            AlignItems::Center
        };
        node.row_gap = if compact { Val::Px(8.0) } else { Val::ZERO };
    }
    for mut node in &mut header_actions {
        node.flex_wrap = if compact {
            FlexWrap::Wrap
        } else {
            FlexWrap::NoWrap
        };
        node.row_gap = if compact { Val::Px(6.0) } else { Val::ZERO };
    }
    for mut node in &mut bodies {
        node.display = if compact {
            Display::Grid
        } else {
            Display::Flex
        };
        node.flex_direction = if compact {
            FlexDirection::Column
        } else {
            FlexDirection::Row
        };
        node.height = if compact { Val::Auto } else { Val::Px(0.0) };
        node.flex_grow = if compact { 0.0 } else { 1.0 };
        node.overflow = Overflow::default();
        node.row_gap = if compact { Val::Px(12.0) } else { Val::ZERO };
        node.grid_template_columns = if compact {
            vec![GridTrack::flex(1.0)]
        } else {
            Vec::new()
        };
        node.grid_template_rows = if compact {
            vec![GridTrack::auto(), GridTrack::auto(), GridTrack::auto()]
        } else {
            Vec::new()
        };
    }
    for (entity, role, mut node) in &mut panels {
        if compact {
            node.width = Val::Percent(100.0);
            node.height = match role {
                CreatorBodyPanel::Sidebar { .. } => Val::Auto,
                CreatorBodyPanel::Main => Val::Px(620.0),
            };
            node.min_height = match role {
                CreatorBodyPanel::Sidebar { .. } => Val::Auto,
                CreatorBodyPanel::Main => Val::Px(620.0),
            };
            node.grid_column = GridPlacement::start(1);
            node.grid_row = GridPlacement::start(match role {
                CreatorBodyPanel::Main => 1,
                CreatorBodyPanel::Sidebar { compact_row, .. } => *compact_row,
            });
            node.flex_grow = 0.0;
            if matches!(role, CreatorBodyPanel::Sidebar { .. }) {
                node.overflow = Overflow::default();
                // Compact owns one continuous page at the root. Leaving an idle
                // ScrollArea on a now-visible sidebar would swallow wheel events
                // before they reach that root owner.
                commands.entity(entity).remove::<ScrollArea>();
            }
        } else {
            node.width = match role {
                CreatorBodyPanel::Sidebar { width, .. } => Val::Px(*width),
                CreatorBodyPanel::Main => Val::Auto,
            };
            node.height = Val::Auto;
            node.min_height = Val::Px(0.0);
            node.grid_column = GridPlacement::auto();
            node.grid_row = GridPlacement::auto();
            node.flex_grow = if matches!(role, CreatorBodyPanel::Main) {
                1.0
            } else {
                0.0
            };
            if matches!(role, CreatorBodyPanel::Sidebar { .. }) {
                node.overflow = Overflow::scroll_y();
                // Overflow is only styling until Bevy's ScrollArea owns pointer
                // scrolling and ScrollIntoView for keyboard focus.
                commands.entity(entity).insert(ScrollArea);
            }
        }
    }
}

fn creator_screen_active(screen: Res<State<Screen>>) -> bool {
    matches!(
        screen.get(),
        Screen::CharacterCreator | Screen::SpellCreator
    )
}

fn render(
    mut commands: Commands,
    roots: Query<Entity, With<CreatorRoot>>,
    view: Res<CreatorScreenView>,
    metrics: Res<ResolvedUiMetrics>,
    assets: Res<UiAssets>,
) {
    if !view.is_changed() && !metrics.is_changed() {
        return;
    }
    for root in &roots {
        commands.entity(root).despawn();
    }
    if !view.active {
        return;
    }
    spawn_creator_ui(
        &mut commands,
        &assets,
        &view,
        &view.library,
        view.elements.as_ref(),
        view.spell_book.as_ref(),
        view.spell_file.as_ref(),
        view.lattice_file.as_ref(),
        view.presets.as_ref(),
        metrics.control_scale,
    );
}

fn emit_actions(
    clicked: Query<(&Interaction, &CreatorIntent), Changed<Interaction>>,
    mut intents: MessageWriter<UiIntent>,
) {
    for (interaction, action) in &clicked {
        if *interaction == Interaction::Pressed {
            intents.write(UiIntent::Creator(action.clone()));
        }
    }
}

fn emit_name_changes(
    fields: Query<(&EditableText, &CreatorNameField), Changed<EditableText>>,
    mut intents: MessageWriter<UiIntent>,
) {
    for (value, field) in &fields {
        intents.write(UiIntent::Creator(CreatorIntent::SetName(
            *field,
            value.value().to_string(),
        )));
    }
}

fn spawn_creator_ui(
    commands: &mut Commands,
    assets: &UiAssets,
    session: &CreatorScreenView,
    store: &CreatorLibraryView,
    elements: Option<&ElementCatalog>,
    spell_book: Option<&SpellBook>,
    spell_file: Option<&SpellFile>,
    lattice_file: Option<&LatticeFile>,
    presets: Option<&CreationPresetCatalog>,
    semantic_control_scale: f32,
) {
    let screen = match session.tab {
        CreatorTab::Characters => Screen::CharacterCreator,
        CreatorTab::Spells => Screen::SpellCreator,
    };
    commands
        .spawn((
            screen_root(screen, "Creator Screen"),
            CreatorRoot,
            ScrollArea,
            ScrollPosition::default(),
        ))
        .insert(Node {
            padding: UiRect::all(Val::Px(18.0)),
            row_gap: Val::Px(10.0),
            ..screen_root_node()
        })
        .with_children(|root| {
            root.spawn((
                CreatorHeader,
                Node {
                    width: Val::Percent(96.0),
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::SpaceBetween,
                    align_items: AlignItems::Center,
                    ..default()
                },
            ))
            .with_children(|header| {
                header.spawn(display(
                    assets,
                    match session.workspace {
                        CreatorWorkspace::Hub => match session.tab {
                            CreatorTab::Characters => "Character Library",
                            CreatorTab::Spells => "Spell Library",
                        },
                        CreatorWorkspace::Character => "Character Workspace",
                        CreatorWorkspace::Spell => "Spell Workspace",
                    },
                ));
                header
                    .spawn((
                        CreatorHeaderActions,
                        Node {
                            flex_direction: FlexDirection::Row,
                            flex_wrap: FlexWrap::Wrap,
                            column_gap: Val::Px(8.0),
                            row_gap: Val::Px(6.0),
                            align_items: AlignItems::Center,
                            ..default()
                        },
                    ))
                    .with_children(|actions| {
                        if session.workspace != CreatorWorkspace::Hub {
                            action_button(actions, assets, "Undo", CreatorIntent::Undo, 160.0);
                            action_button(actions, assets, "Redo", CreatorIntent::Redo, 160.0);
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
                                CreatorIntent::ResetLibrary,
                                220.0,
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
                                CreatorIntent::DiscardChanges,
                                260.0,
                            );
                        }
                        action_button(
                            actions,
                            assets,
                            if session.workspace == CreatorWorkspace::Hub {
                                "Title"
                            } else {
                                "Library"
                            },
                            CreatorIntent::Back,
                            180.0,
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
            root.spawn((
                Name::new("Creator Content"),
                CreatorResponsiveBody,
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Px(0.0),
                    min_height: Val::Px(0.0),
                    flex_basis: Val::Px(0.0),
                    flex_grow: 1.0,
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(12.0),
                    ..default()
                },
            ))
            .with_children(|body| match session.workspace {
                CreatorWorkspace::Hub => {
                    spawn_creator_hub(body, assets, session, store, elements, spell_book, presets)
                }
                CreatorWorkspace::Character => spawn_character_tab(
                    body,
                    assets,
                    session,
                    store,
                    elements,
                    spell_book,
                    lattice_file,
                    presets,
                    semantic_control_scale,
                ),
                CreatorWorkspace::Spell => spawn_spell_tab(
                    body, assets, session, store, elements, spell_book, spell_file, presets,
                ),
            });
        });
}

fn spawn_creator_hub(
    body: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    session: &CreatorScreenView,
    store: &CreatorLibraryView,
    elements: Option<&ElementCatalog>,
    spell_book: Option<&SpellBook>,
    presets: Option<&CreationPresetCatalog>,
) {
    body.spawn((
        CreatorBodyPanel::Sidebar {
            width: 240.0,
            compact_row: 2,
        },
        panel(),
    ))
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
                    CreatorIntent::OpenSpellCreator,
                    190.0,
                );
            }
        });

    body.spawn((CreatorBodyPanel::Main, panel()))
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
                    CreatorIntent::NewCharacter,
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
                                CreatorIntent::SelectCharacter(saved.id),
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
                                    CreatorIntent::DuplicatePackagedCharacter(record.key.clone()),
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
                    CreatorIntent::NewSpell,
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
                                CreatorIntent::SelectSpell(saved.id),
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
                                    CreatorIntent::DuplicatePackagedSpell(record.key.clone()),
                                    190.0,
                                );
                            }
                        });
                }
            }
        });

    body.spawn((
        CreatorBodyPanel::Sidebar {
            width: 330.0,
            compact_row: 3,
        },
        panel(),
    ))
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
    action: CreatorIntent,
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
    action: CreatorIntent,
    width: f32,
) {
    let text = text.into();
    parent
        .spawn((row_button(text.clone(), width), action))
        .with_child(label(assets, text));
}

fn name_input(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    value: &str,
    field: CreatorNameField,
) {
    let accessible = match field {
        CreatorNameField::Character => "Character Name",
        CreatorNameField::Spell => "Spell Name",
    };
    parent.spawn((
        Name::new(accessible),
        AccessibleLabel::new(accessible),
        TabIndex(0),
        EditableText {
            max_characters: Some(MAX_CREATION_NAME_CHARS),
            visible_width: Some(24.0),
            ..EditableText::new(value)
        },
        body_text_role(),
        responsive_control_role(),
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
            flex_shrink: 0.0,
            padding: UiRect::all(Val::Px(9.0)),
            border: UiRect::all(Val::Px(1.0)),
            ..default()
        },
        field,
    ));
}

#[expect(
    clippy::too_many_arguments,
    reason = "tab rendering consumes the loaded catalogs it presents"
)]
fn spawn_character_tab(
    body: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    session: &CreatorScreenView,
    store: &CreatorLibraryView,
    elements: Option<&ElementCatalog>,
    spell_book: Option<&SpellBook>,
    _lattice_file: Option<&LatticeFile>,
    _presets: Option<&CreationPresetCatalog>,
    semantic_control_scale: f32,
) {
    let Some(character) = &session.character else {
        body.spawn(blurb(assets, "No character draft."));
        return;
    };
    let issues = session.character_issues.clone();

    body.spawn((
        CreatorBodyPanel::Sidebar {
            width: 250.0,
            compact_row: 2,
        },
        panel(),
    ))
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
            CreatorIntent::InspectTool,
            Color::srgba(0.24, 0.26, 0.31, 0.96),
            session.active_tool.is_none() && !session.erase_tool,
        );
        colored_tool_button(
            palette,
            assets,
            "Blank",
            CreatorIntent::ChooseTool(CreationCellKind::Blank),
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
                    CreatorIntent::ChooseTool(kind.clone()),
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
            for (_, name, _spell) in spells.iter().filter(|(_, name, _)| {
                session
                    .deployable_shipped_spells
                    .iter()
                    .any(|candidate| candidate == name)
            }) {
                let kind = CreationCellKind::Spell(SpellReference::Shipped(name.to_owned()));
                colored_tool_button(
                    palette,
                    assets,
                    format!("Spell · {name}"),
                    CreatorIntent::ChooseTool(kind.clone()),
                    Color::srgba(0.30, 0.33, 0.40, 0.96),
                    session.active_tool.as_ref() == Some(&kind),
                );
            }
        }
        if elements.is_some() {
            for spell in &store.file.spells {
                if session.deployable_custom_spells.contains(&spell.id) {
                    let kind = CreationCellKind::Spell(SpellReference::Custom(spell.id));
                    colored_tool_button(
                        palette,
                        assets,
                        format!("Custom · {}", spell.name),
                        CreatorIntent::ChooseTool(kind.clone()),
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
            CreatorIntent::OpenSpellCreator,
            190.0,
        );
        colored_tool_button(
            palette,
            assets,
            "Erase",
            CreatorIntent::ChooseErase,
            Color::srgba(0.46, 0.13, 0.11, 0.96),
            session.erase_tool,
        );
    });

    body.spawn((CreatorBodyPanel::Main, panel()))
        .insert(Node {
            min_width: Val::Px(0.0),
            min_height: Val::Px(0.0),
            flex_grow: 1.0,
            ..panel_node()
        })
        .with_children(|center| {
            name_input(center, assets, &character.name, CreatorNameField::Character);
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
                            action_button(zoom, assets, "Fit", CreatorIntent::FitLattice, 58.0);
                            action_button(zoom, assets, "−", CreatorIntent::Zoom(-1), 44.0);
                            zoom.spawn(label(
                                assets,
                                format!("{}%", lattice_scale_percent(session.zoom_step)),
                            ));
                            action_button(zoom, assets, "+", CreatorIntent::Zoom(1), 44.0);
                        });
                });
            spawn_character_actions(center, assets, session, &issues);
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
                                semantic_control_scale,
                            );
                        });
                });
        });

    body.spawn((
        CreatorBodyPanel::Sidebar {
            width: 330.0,
            compact_row: 3,
        },
        panel(),
    ))
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
                    CreatorIntent::RemoveCell,
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
                                CreatorIntent::AdjustStat {
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

fn spawn_character_actions(
    center: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    session: &CreatorScreenView,
    issues: &[String],
) {
    center
        .spawn((
            Name::new("Character Workspace Actions"),
            Node {
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                column_gap: Val::Px(8.0),
                row_gap: Val::Px(6.0),
                align_items: AlignItems::Center,
                ..default()
            },
        ))
        .with_children(|actions| {
            action_button(actions, assets, "Save", CreatorIntent::SaveCharacter, 110.0);
            action_button(
                actions,
                assets,
                "Duplicate",
                CreatorIntent::DuplicateCharacter,
                220.0,
            );
            action_button(
                actions,
                assets,
                if session.confirm_delete {
                    "Confirm Delete"
                } else {
                    "Delete"
                },
                CreatorIntent::DeleteCharacter,
                140.0,
            );
            action_button(
                actions,
                assets,
                "Local Test",
                CreatorIntent::LocalTest,
                120.0,
            );
            action_button(
                actions,
                assets,
                "Test on Map",
                CreatorIntent::TestOnMap,
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
}

fn spawn_lattice_cells(
    surface: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    character: &SavedCharacter,
    selected: Option<LatticeCoord>,
    zoom_step: i8,
    elements: Option<&ElementCatalog>,
    library: &hex_assets::CreationLibraryFile,
    semantic_control_scale: f32,
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
    let intrinsic_scale = lattice_scale(zoom_step);
    let scale = intrinsic_scale * semantic_control_scale.max(1.0);
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
                crate::lattice::TessellatedControl,
                Button,
                owner_resolved_control_role(),
                CreatorIntent::SelectCell(coord),
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
                    compact_glyph_role((11.0 * intrinsic_scale).max(9.0)),
                    TextFont {
                        font: assets.body.clone().into(),
                        ..TextFont::from_font_size((11.0 * intrinsic_scale).max(9.0))
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
                    compact_glyph_role((8.0 * intrinsic_scale).max(7.0)),
                    TextFont {
                        font: assets.body.clone().into(),
                        ..TextFont::from_font_size((8.0 * intrinsic_scale).max(7.0))
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
                    crate::lattice::TessellatedControl,
                    Button,
                    owner_resolved_control_role(),
                    CreatorIntent::AddCell(coord),
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
    action: CreatorIntent,
    color: Color,
    selected: bool,
) {
    let text = text.into();
    parent
        .spawn((row_button(text.clone(), 200.0), action))
        .insert((
            OwnColors,
            BackgroundColor(brighten(color, if selected { 0.26 } else { 0.0 })),
            BorderColor::all(if selected { ACCENT } else { EDGE }),
        ))
        .with_child(label(assets, text));
}

fn spawn_spell_tab(
    body: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    session: &CreatorScreenView,
    store: &CreatorLibraryView,
    elements: Option<&ElementCatalog>,
    spell_book: Option<&SpellBook>,
    _spell_file: Option<&SpellFile>,
    _presets: Option<&CreationPresetCatalog>,
) {
    let Some(saved) = &session.spell else {
        body.spawn(blurb(assets, "No spell draft."));
        return;
    };
    let issues = session.spell_issues.clone();

    body.spawn((
        CreatorBodyPanel::Sidebar {
            width: 300.0,
            compact_row: 2,
        },
        panel(),
    ))
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
            left.spawn(panel())
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
                                CreatorIntent::CycleRequirement(index),
                                84.0,
                            );
                            action_button(
                                row,
                                assets,
                                "−",
                                CreatorIntent::AdjustRequirement(index, -1),
                                42.0,
                            );
                            action_button(
                                row,
                                assets,
                                "+",
                                CreatorIntent::AdjustRequirement(index, 1),
                                42.0,
                            );
                            action_button(
                                row,
                                assets,
                                "Remove",
                                CreatorIntent::RemoveRequirement(index),
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
                CreatorIntent::AddRequirement,
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
                CreatorIntent::SetEnchantment(false),
                !enchantment,
                104.0,
            );
            segmented_button(
                controls,
                assets,
                "Enchantment",
                CreatorIntent::SetEnchantment(true),
                enchantment,
                120.0,
            );
            let single = saved.spell.targeting.shape == TargetShape::Single;
            segmented_button(
                controls,
                assets,
                "Self",
                CreatorIntent::SetSingleTarget(false),
                !single,
                72.0,
            );
            segmented_button(
                controls,
                assets,
                "Single",
                CreatorIntent::SetSingleTarget(true),
                single,
                82.0,
            );
            if single {
                action_button(
                    controls,
                    assets,
                    "Range −",
                    CreatorIntent::AdjustRange(-1),
                    84.0,
                );
                action_button(
                    controls,
                    assets,
                    "Range +",
                    CreatorIntent::AdjustRange(1),
                    84.0,
                );
            }
            if enchantment {
                action_button(
                    controls,
                    assets,
                    "Defense −",
                    CreatorIntent::AdjustDefense(-1),
                    100.0,
                );
                action_button(
                    controls,
                    assets,
                    "Defense +",
                    CreatorIntent::AdjustDefense(1),
                    100.0,
                );
            }
        });
    });

    body.spawn((CreatorBodyPanel::Main, panel()))
        .insert(Node {
            min_width: Val::Px(0.0),
            min_height: Val::Px(0.0),
            flex_grow: 1.0,
            overflow: Overflow::scroll_y(),
            ..panel_node()
        })
        .with_children(|form| {
            name_input(form, assets, &saved.name, CreatorNameField::Spell);
            let summary = SpellBuildSummary::from_saved(saved, elements);
            form.spawn(heading(assets, "ordered effects"));
            form.spawn(label(assets, summary.sentence.clone()));
            for (index, effect) in saved.spell.effects.iter().enumerate() {
                let effect_text = effect_summary(effect);
                form.spawn(panel())
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
                                CreatorIntent::MoveEffect(index, -1),
                                44.0,
                            );
                            action_button(
                                row,
                                assets,
                                "→",
                                CreatorIntent::MoveEffect(index, 1),
                                44.0,
                            );
                            action_button(
                                row,
                                assets,
                                "Value −",
                                CreatorIntent::AdjustEffect(index, -1),
                                76.0,
                            );
                            action_button(
                                row,
                                assets,
                                "Value +",
                                CreatorIntent::AdjustEffect(index, 1),
                                76.0,
                            );
                            action_button(
                                row,
                                assets,
                                "Remove",
                                CreatorIntent::RemoveEffect(index),
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
                    ("+ Disable", CreatorEffectKind::Disable),
                    ("+ Burn", CreatorEffectKind::Burn),
                    ("+ Restore", CreatorEffectKind::Restore),
                    ("+ Reveal", CreatorEffectKind::Reveal),
                ] {
                    action_button(row, assets, name, CreatorIntent::AddEffect(kind), 105.0);
                }
            });
            form.spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(8.0),
                margin: UiRect::top(Val::Px(8.0)),
                ..default()
            })
            .with_children(|actions| {
                action_button(actions, assets, "Save", CreatorIntent::SaveSpell, 110.0);
                action_button(
                    actions,
                    assets,
                    "Duplicate",
                    CreatorIntent::DuplicateSpell,
                    220.0,
                );
                action_button(
                    actions,
                    assets,
                    if session.confirm_delete {
                        "Confirm Delete"
                    } else {
                        "Delete"
                    },
                    CreatorIntent::DeleteSpell,
                    140.0,
                );
            });
        });

    body.spawn((
        CreatorBodyPanel::Sidebar {
            width: 320.0,
            compact_row: 3,
        },
        panel(),
    ))
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
    action: CreatorIntent,
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
