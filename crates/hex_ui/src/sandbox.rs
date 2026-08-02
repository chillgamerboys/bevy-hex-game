//! Full-screen Sandbox composition routes.

use bevy::input_focus::InputFocus;
use bevy::prelude::*;
use bevy::ui::InteractionDisabled;
use bevy::ui_widgets::ScrollArea;
use hex_core::Screen;
use hex_gameplay_model::{SandboxRoute, SandboxSide, SandboxSlotIndex};

use crate::{
    blurb, despawn_screen, display, fine, heading, label, panel, panel_node, row_button,
    screen_root, screen_root_node, screen_title, ResolvedUiMetrics, SandboxIntent,
    SandboxLatticeCellKind, SandboxLatticeCellView, SandboxMapView, SandboxRosterSlotView,
    SandboxView, UiAssets, UiIntent, UiSystems, UiTextMustFit, UiViewportClass, DANGER,
    FUSION_COLOR,
};

#[derive(Component)]
struct SandboxSurface;

#[derive(Component)]
struct SandboxContent;

#[derive(Component)]
struct SandboxOverviewDeck;

#[derive(Component)]
struct SandboxRosterGrid;

#[derive(Component)]
struct SandboxPickerLayout;

#[derive(Component, Clone, Copy)]
enum SandboxResponsiveNode {
    OverviewDeck,
    OverviewCard,
    RosterGrid,
    RosterSlot,
    PickerLayout,
    CharacterList,
    CharacterPreview,
}

#[derive(Component)]
struct SandboxControl(SandboxIntent);

pub(super) fn plugin(app: &mut App) {
    app.add_systems(OnEnter(Screen::Sandbox), spawn)
        .add_systems(
            Update,
            (refresh, apply_layout)
                .chain()
                .in_set(UiSystems::Render)
                .run_if(in_state(Screen::Sandbox)),
        )
        .add_systems(
            Update,
            emit_intents
                .in_set(UiSystems::EmitIntents)
                .run_if(in_state(Screen::Sandbox)),
        )
        .add_systems(OnExit(Screen::Sandbox), despawn_screen(Screen::Sandbox));
}

fn spawn(
    mut commands: Commands,
    assets: Res<UiAssets>,
    asset_server: Res<AssetServer>,
    view: Res<SandboxView>,
) {
    commands
        .spawn((screen_root(Screen::Sandbox, "Sandbox"), SandboxSurface))
        .insert(Node {
            padding: UiRect::all(Val::Px(24.0)),
            justify_content: JustifyContent::FlexStart,
            overflow: Overflow::clip_y(),
            ..screen_root_node()
        })
        .with_children(|root| render(root, &assets, &asset_server, &view));
}

fn refresh(
    view: Res<SandboxView>,
    assets: Res<UiAssets>,
    asset_server: Res<AssetServer>,
    roots: Query<Entity, With<SandboxSurface>>,
    parents: Query<&ChildOf>,
    names: Query<&Name>,
    mut focus: ResMut<InputFocus>,
    mut focus_refreshes: ResMut<crate::focus::FocusRefreshRequests>,
    mut commands: Commands,
) {
    if !view.is_changed() {
        return;
    }
    for root in &roots {
        crate::focus::begin_route_refresh(root, &mut focus, &parents, &names, &mut focus_refreshes);
        commands.entity(root).despawn_related::<Children>();
        commands
            .entity(root)
            .with_children(|root| render(root, &assets, &asset_server, &view));
    }
}

fn render(
    root: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    asset_server: &AssetServer,
    view: &SandboxView,
) {
    let title = match view.route {
        SandboxRoute::Overview => "Hex / Sandbox".to_owned(),
        SandboxRoute::MapBrowser => "Hex / Sandbox / Maps".to_owned(),
        SandboxRoute::MapDetail => "Hex / Sandbox / Map Confirmation".to_owned(),
        SandboxRoute::Roster(side) => format!("Hex / Sandbox / {side}"),
        SandboxRoute::CharacterPicker { side, slot } => {
            format!("Hex / Sandbox / {side} / Slot {slot}")
        }
    };
    root.spawn((screen_title(assets, title), UiTextMustFit));
    if let Some(notice) = &view.notice {
        root.spawn(blurb(assets, notice.clone()));
    }
    root.spawn((
        Name::new("Sandbox Content"),
        SandboxContent,
        ScrollArea,
        ScrollPosition::default(),
        Node {
            width: Val::Percent(100.0),
            min_width: Val::Px(0.0),
            min_height: Val::Px(0.0),
            flex_grow: 1.0,
            overflow: Overflow::scroll_y(),
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::FlexStart,
            align_items: AlignItems::Center,
            padding: UiRect::axes(Val::Px(8.0), Val::Px(10.0)),
            ..default()
        },
    ))
    .with_children(|content| match view.route {
        SandboxRoute::Overview => render_overview(content, assets, view),
        SandboxRoute::MapBrowser => render_map_browser(content, assets, asset_server, view),
        SandboxRoute::MapDetail => render_map_detail(content, assets, asset_server, view),
        SandboxRoute::Roster(side) => render_roster(content, assets, side, view),
        SandboxRoute::CharacterPicker { side, slot } => {
            render_character_picker(content, assets, side, slot, view)
        }
    });
    render_footer(root, assets, view);
}

fn render_overview(content: &mut ChildSpawnerCommands, assets: &UiAssets, view: &SandboxView) {
    content.spawn((display(assets, "Sandbox"), UiTextMustFit));
    content.spawn(blurb(
        assets,
        "Choose a map and up to six characters on each side, then deploy them.",
    ));
    content
        .spawn((
            Name::new("Sandbox Overview Cards"),
            SandboxOverviewDeck,
            SandboxResponsiveNode::OverviewDeck,
            Node {
                width: Val::Percent(96.0),
                max_width: Val::Px(1_360.0),
                min_width: Val::Px(0.0),
                flex_shrink: 0.0,
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Stretch,
                justify_content: JustifyContent::Center,
                column_gap: Val::Px(18.0),
                row_gap: Val::Px(18.0),
                ..default()
            },
        ))
        .with_children(|deck| {
            overview_map_card(deck, assets, view.map.as_ref());
            overview_roster_card(deck, assets, SandboxSide::Party, &view.party);
            overview_roster_card(deck, assets, SandboxSide::Enemies, &view.enemies);
        });
}

fn overview_map_card(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    map: Option<&SandboxMapView>,
) {
    parent
        .spawn((
            row_button("Choose Sandbox map", 320.0),
            SandboxControl(SandboxIntent::OpenMapBrowser),
            SandboxResponsiveNode::OverviewCard,
            crate::UiVisibilityRequirement::Scrollable,
        ))
        .insert(Node {
            width: Val::Percent(31.0),
            min_width: Val::Px(44.0),
            min_height: Val::Px(270.0),
            flex_grow: 1.0,
            flex_shrink: 1.0,
            align_items: AlignItems::Stretch,
            justify_content: JustifyContent::FlexStart,
            ..flexible_row_button_node(270.0)
        })
        .with_children(|card| {
            card.spawn(heading(assets, "Map"));
            if let Some(map) = map {
                card.spawn(label(assets, map.name.clone()));
                card.spawn(fine(assets, seed_label(map.resolved_seed)));
                card.spawn(blurb(assets, map.description.clone()));
            } else {
                card.spawn(blurb(assets, "Select a Map"));
            }
            card.spawn(fine(assets, "Change Map →"));
        });
}

fn overview_roster_card(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    side: SandboxSide,
    slots: &[SandboxRosterSlotView],
) {
    let count = slots.iter().filter(|slot| slot.character.is_some()).count();
    parent
        .spawn((
            row_button(format!("Choose {side} characters"), 320.0),
            SandboxControl(SandboxIntent::OpenRoster(side)),
            SandboxResponsiveNode::OverviewCard,
            crate::UiVisibilityRequirement::Scrollable,
        ))
        .insert(Node {
            width: Val::Percent(31.0),
            min_width: Val::Px(44.0),
            min_height: Val::Px(270.0),
            flex_grow: 1.0,
            flex_shrink: 1.0,
            align_items: AlignItems::Stretch,
            justify_content: JustifyContent::FlexStart,
            ..flexible_row_button_node(270.0)
        })
        .with_children(|card| {
            card.spawn(heading(assets, side.to_string()));
            card.spawn(label(assets, format!("{count} / 6 characters")));
            for slot in slots.iter().filter_map(|slot| slot.character.as_ref()) {
                card.spawn(fine(assets, slot.name.clone()));
            }
            card.spawn(fine(assets, format!("Edit {side} →")));
        });
}

fn render_map_browser(
    content: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    asset_server: &AssetServer,
    view: &SandboxView,
) {
    content.spawn(heading(assets, "Select a Map"));
    content.spawn(blurb(
        assets,
        "Inspect a catalog map before replacing the committed Sandbox map.",
    ));
    content
        .spawn((
            Name::new("Sandbox Map Catalog"),
            Node {
                width: Val::Percent(96.0),
                max_width: Val::Px(1_080.0),
                min_width: Val::Px(0.0),
                flex_shrink: 0.0,
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Stretch,
                row_gap: Val::Px(10.0),
                ..default()
            },
        ))
        .with_children(|list| {
            for map in &view.maps {
                list.spawn((
                    row_button(format!("Inspect {}", map.name), 520.0),
                    SandboxControl(SandboxIntent::SelectMap(map.id.clone())),
                    crate::UiVisibilityRequirement::Scrollable,
                ))
                .insert(Node {
                    width: Val::Percent(100.0),
                    min_width: Val::Px(44.0),
                    min_height: Val::Px(116.0),
                    flex_shrink: 1.0,
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(16.0),
                    ..flexible_row_button_node(116.0)
                })
                .with_children(|row| {
                    row.spawn((
                        Name::new(format!("{} Preview", map.name)),
                        ImageNode::new(asset_server.load(map.preview.clone())),
                        Node {
                            width: Val::Px(168.0),
                            height: Val::Px(92.0),
                            flex_shrink: 0.0,
                            ..default()
                        },
                    ));
                    row.spawn(Node {
                        min_width: Val::Px(0.0),
                        flex_grow: 1.0,
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(4.0),
                        ..default()
                    })
                    .with_children(|copy| {
                        copy.spawn(heading(assets, map.name.clone()));
                        copy.spawn(blurb(assets, map.description.clone()));
                    });
                });
            }
            let mut coming_soon = list.spawn((
                row_button("Create New Map — Coming Soon", 520.0),
                InteractionDisabled,
                crate::UiVisibilityRequirement::Scrollable,
            ));
            coming_soon.with_children(|button| {
                button.spawn(label(assets, "Create New Map"));
                button.spawn(fine(assets, "Coming Soon"));
            });
        });
}

fn render_map_detail(
    content: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    asset_server: &AssetServer,
    view: &SandboxView,
) {
    let Some(map) = view.pending_map.as_ref() else {
        content.spawn(blurb(assets, "No pending map is available."));
        return;
    };
    content
        .spawn((Name::new("Sandbox Map Confirmation"), panel()))
        .insert(Node {
            width: Val::Percent(96.0),
            max_width: Val::Px(1_180.0),
            min_width: Val::Px(0.0),
            flex_shrink: 0.0,
            align_items: AlignItems::Stretch,
            ..panel_node()
        })
        .with_children(|panel| {
            panel.spawn(heading(assets, map.name.clone()));
            panel.spawn((
                Name::new(format!("{} Large Preview", map.name)),
                ImageNode::new(asset_server.load(map.preview.clone())),
                Node {
                    width: Val::Percent(100.0),
                    max_width: Val::Px(960.0),
                    min_height: Val::Px(240.0),
                    height: Val::Vh(48.0),
                    max_height: Val::Px(540.0),
                    align_self: AlignSelf::Center,
                    ..default()
                },
            ));
            panel.spawn(blurb(assets, map.description.clone()));
            panel.spawn(label(assets, seed_label(map.resolved_seed)));
        });
}

fn render_roster(
    content: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    side: SandboxSide,
    view: &SandboxView,
) {
    content.spawn(heading(assets, format!("Select {side} Characters")));
    content.spawn(blurb(
        assets,
        format!("Choose up to six {side} characters. Empty slots remain sparse."),
    ));
    let slots = side_slots(view, side);
    content
        .spawn((
            Name::new(format!("{side} Slots")),
            SandboxRosterGrid,
            SandboxResponsiveNode::RosterGrid,
            Node {
                width: Val::Percent(96.0),
                max_width: Val::Px(1_220.0),
                min_width: Val::Px(0.0),
                flex_shrink: 0.0,
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Stretch,
                column_gap: Val::Px(16.0),
                row_gap: Val::Px(16.0),
                ..default()
            },
        ))
        .with_children(|grid| {
            for slot in slots {
                roster_slot(grid, assets, side, slot);
            }
        });
}

fn roster_slot(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    side: SandboxSide,
    slot: &SandboxRosterSlotView,
) {
    parent
        .spawn((
            row_button(format!("{side} slot {}", slot.slot), 460.0),
            SandboxControl(SandboxIntent::OpenCharacterPicker {
                side,
                slot: slot.slot,
            }),
            SandboxResponsiveNode::RosterSlot,
            crate::UiVisibilityRequirement::Scrollable,
        ))
        .insert(Node {
            width: Val::Percent(47.0),
            min_width: Val::Px(44.0),
            min_height: Val::Px(154.0),
            flex_shrink: 0.0,
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::FlexStart,
            column_gap: Val::Px(12.0),
            ..flexible_row_button_node(154.0)
        })
        .with_children(|card| {
            if let Some(character) = &slot.character {
                spawn_mini_lattice(card, assets, &character.cells);
                card.spawn(Node {
                    min_width: Val::Px(0.0),
                    flex_grow: 1.0,
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(5.0),
                    ..default()
                })
                .with_children(|copy| {
                    copy.spawn(heading(assets, character.name.clone()));
                    copy.spawn(fine(assets, character.lattice.clone()));
                    if let Some(reason) = &character.blocked {
                        copy.spawn(fine(assets, format!("NOT MAP-READY · {reason}")))
                            .insert(TextColor(DANGER));
                    }
                    copy.spawn(fine(assets, format!("Slot {} · Change →", slot.slot)));
                });
            } else {
                card.spawn(heading(assets, format!("Slot {}", slot.slot)));
                card.spawn(blurb(assets, "Select a Character"));
            }
        });
}

fn render_character_picker(
    content: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    side: SandboxSide,
    slot: SandboxSlotIndex,
    view: &SandboxView,
) {
    content.spawn(heading(
        assets,
        format!("Select a Character for {side} Slot {slot}"),
    ));
    content
        .spawn((
            Name::new("Sandbox Character Picker"),
            SandboxPickerLayout,
            SandboxResponsiveNode::PickerLayout,
            Node {
                width: Val::Percent(96.0),
                max_width: Val::Px(1_320.0),
                min_width: Val::Px(0.0),
                flex_shrink: 0.0,
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Stretch,
                column_gap: Val::Px(18.0),
                row_gap: Val::Px(18.0),
                ..default()
            },
        ))
        .with_children(|layout| {
            layout
                .spawn((
                    Name::new("Character List"),
                    SandboxResponsiveNode::CharacterList,
                    panel(),
                ))
                .insert(Node {
                    width: Val::Percent(44.0),
                    min_width: Val::Px(0.0),
                    flex_shrink: 1.0,
                    align_items: AlignItems::Stretch,
                    row_gap: Val::Px(8.0),
                    ..panel_node()
                })
                .with_children(|list| {
                    list.spawn(heading(assets, "Characters"));
                    for character in &view.characters {
                        let title = if character.selected {
                            format!("Selected · {}", character.name)
                        } else {
                            character.name.clone()
                        };
                        list.spawn((
                            row_button(format!("Preview {}", character.name), 300.0),
                            SandboxControl(SandboxIntent::PreviewCharacter(
                                character.character.clone(),
                            )),
                            crate::UiVisibilityRequirement::Scrollable,
                        ))
                        .insert(Node {
                            width: Val::Percent(100.0),
                            min_width: Val::Px(44.0),
                            // Long readiness summaries wrap at enlarged semantic
                            // scales. The shared content scroll owns the resulting
                            // height; shrinking the row clips its interactive label.
                            flex_shrink: 0.0,
                            align_items: AlignItems::Stretch,
                            ..flexible_row_button_node(48.0)
                        })
                        .with_children(|row| {
                            row.spawn(label(assets, title));
                            row.spawn(fine(assets, character.lattice.clone()));
                        });
                    }
                });
            layout
                .spawn((
                    Name::new("Character Lattice Preview"),
                    SandboxResponsiveNode::CharacterPreview,
                    panel(),
                ))
                .insert(Node {
                    width: Val::Auto,
                    min_width: Val::Px(0.0),
                    flex_grow: 1.0,
                    flex_shrink: 1.0,
                    min_height: Val::Px(360.0),
                    align_items: AlignItems::Center,
                    ..panel_node()
                })
                .with_children(|preview| {
                    preview.spawn(heading(assets, "Lattice Preview"));
                    if let Some(character) = &view.preview {
                        preview.spawn(label(assets, character.name.clone()));
                        spawn_large_lattice(preview, assets, &character.cells);
                        preview.spawn(blurb(assets, character.lattice.clone()));
                        if let Some(reason) = &character.blocked {
                            preview
                                .spawn(fine(assets, format!("NOT MAP-READY · {reason}")))
                                .insert(TextColor(DANGER));
                        }
                    } else {
                        preview.spawn(blurb(assets, "Choose a character to preview."));
                    }
                });
        });
}

fn render_footer(root: &mut ChildSpawnerCommands, assets: &UiAssets, view: &SandboxView) {
    root.spawn((
        Name::new("Sandbox Actions"),
        Node {
            width: Val::Percent(100.0),
            min_height: Val::Px(56.0),
            flex_shrink: 0.0,
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::Wrap,
            justify_content: JustifyContent::FlexEnd,
            align_items: AlignItems::Center,
            column_gap: Val::Px(10.0),
            row_gap: Val::Px(8.0),
            ..default()
        },
    ))
    .with_children(|footer| match view.route {
        SandboxRoute::Overview => {
            control_button(footer, assets, "Back", SandboxIntent::Back, true);
            let blocker = view.start_blocker.as_ref();
            control_button(
                footer,
                assets,
                "Start Sandbox",
                SandboxIntent::StartSandbox,
                blocker.is_none(),
            );
            if let Some(blocker) = blocker {
                footer
                    .spawn(fine(assets, blocker.message()))
                    .insert(TextColor(DANGER));
            }
        }
        SandboxRoute::MapBrowser => {
            control_button(footer, assets, "Back", SandboxIntent::Back, true);
        }
        SandboxRoute::MapDetail => {
            control_button(footer, assets, "Back", SandboxIntent::Back, true);
            if view
                .pending_map
                .as_ref()
                .is_some_and(|map| map.can_regenerate)
            {
                control_button(
                    footer,
                    assets,
                    "Regenerate Seed",
                    SandboxIntent::RegenerateMap,
                    true,
                );
            }
            control_button(
                footer,
                assets,
                "Use Map",
                SandboxIntent::UseMap,
                view.pending_map.is_some(),
            );
        }
        SandboxRoute::Roster(_) => {
            control_button(footer, assets, "Back", SandboxIntent::Back, true);
        }
        SandboxRoute::CharacterPicker { side, slot } => {
            control_button(footer, assets, "Back", SandboxIntent::Back, true);
            if side_slots(view, side)
                .iter()
                .find(|entry| entry.slot == slot)
                .is_some_and(|entry| entry.character.is_some())
            {
                control_button(
                    footer,
                    assets,
                    "Clear Slot",
                    SandboxIntent::ClearSlot { side, slot },
                    true,
                );
            }
            control_button(
                footer,
                assets,
                "Create a New Character",
                SandboxIntent::CreateCharacter,
                true,
            );
            control_button(
                footer,
                assets,
                "Use Character",
                SandboxIntent::UseCharacter,
                view.preview.is_some(),
            );
        }
    });
}

fn control_button(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    name: impl Into<String>,
    intent: SandboxIntent,
    enabled: bool,
) {
    let name = name.into();
    let mut button = parent.spawn((
        row_button(name.clone(), 144.0),
        SandboxControl(intent),
        crate::UiVisibilityRequirement::Immediate,
    ));
    button.with_child(label(assets, name));
    if !enabled {
        button.insert(InteractionDisabled);
    }
}

/// Baseline row-button geometry for controls that participate in flexible route layouts.
///
/// These controls intentionally relax the stock row button's content-width floor so a
/// compact route cannot force horizontal overflow. The shared padding, border, radius,
/// and 44 px target floor remain intact.
fn flexible_row_button_node(min_height: f32) -> Node {
    Node {
        width: Val::Auto,
        min_width: Val::Px(44.0),
        height: Val::Auto,
        min_height: Val::Px(min_height.max(44.0)),
        flex_shrink: 0.0,
        align_items: AlignItems::Center,
        justify_content: JustifyContent::Center,
        flex_direction: FlexDirection::Column,
        row_gap: Val::Px(2.0),
        padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
        border: UiRect::all(Val::Px(1.0)),
        border_radius: BorderRadius::all(Val::Px(6.0)),
        ..default()
    }
}

fn side_slots(view: &SandboxView, side: SandboxSide) -> &[SandboxRosterSlotView] {
    match side {
        SandboxSide::Party => &view.party,
        SandboxSide::Enemies => &view.enemies,
    }
}

fn seed_label(seed: Option<u64>) -> String {
    seed.map_or_else(|| "Authored".to_owned(), |seed| format!("Seed {seed}"))
}

pub(crate) fn spawn_mini_lattice(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    cells: &[SandboxLatticeCellView],
) {
    spawn_lattice(parent, assets, cells, 19.0, 98.0, 84.0);
}

fn spawn_large_lattice(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    cells: &[SandboxLatticeCellView],
) {
    spawn_lattice(parent, assets, cells, 34.0, 360.0, 280.0);
}

#[expect(
    clippy::cast_precision_loss,
    reason = "Creator lattice coordinates are schema-bounded and presentation uses pixels"
)]
fn spawn_lattice(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    cells: &[SandboxLatticeCellView],
    width: f32,
    canvas_width: f32,
    canvas_height: f32,
) {
    let height = width * 1.15;
    parent
        .spawn((
            Name::new("Character Lattice Preview"),
            Node {
                width: Val::Px(canvas_width),
                height: Val::Px(canvas_height),
                position_type: PositionType::Relative,
                flex_shrink: 0.0,
                ..default()
            },
        ))
        .with_children(|canvas| {
            for cell in cells {
                let x = canvas_width * 0.45 + (cell.q as f32 + cell.r as f32 * 0.5) * width * 0.88;
                let y = canvas_height * 0.39 + cell.r as f32 * height * 0.74;
                let color = match cell.kind {
                    SandboxLatticeCellKind::Gem => Color::srgb(0.16, 0.45, 0.52),
                    SandboxLatticeCellKind::Fusion => FUSION_COLOR,
                    SandboxLatticeCellKind::Spell => Color::srgba(0.86, 0.80, 0.62, 0.94),
                    SandboxLatticeCellKind::Blank => Color::srgba(0.36, 0.38, 0.42, 0.88),
                };
                canvas
                    .spawn((
                        ImageNode::new(assets.hex_cell.clone()).with_color(color),
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(x),
                            top: Val::Px(y),
                            width: Val::Px(width),
                            height: Val::Px(height),
                            align_items: AlignItems::Center,
                            justify_content: JustifyContent::Center,
                            ..default()
                        },
                    ))
                    .with_child((
                        Text::new(cell.label.clone()),
                        TextFont {
                            font: assets.body.clone().into(),
                            font_size: FontSize::Px((width * 0.34).max(8.0)),
                            ..default()
                        },
                        TextColor(Color::BLACK),
                    ));
            }
        });
}

fn apply_layout(
    metrics: Res<ResolvedUiMetrics>,
    added: Query<(), Added<SandboxResponsiveNode>>,
    mut nodes: Query<(&SandboxResponsiveNode, &mut Node)>,
) {
    if !metrics.is_changed() && added.is_empty() {
        return;
    }
    let compact = metrics.viewport == UiViewportClass::Compact;
    for (role, mut node) in &mut nodes {
        match role {
            SandboxResponsiveNode::OverviewDeck => {
                node.flex_direction = if compact {
                    FlexDirection::Column
                } else {
                    FlexDirection::Row
                };
                node.align_items = AlignItems::Stretch;
            }
            SandboxResponsiveNode::OverviewCard => {
                node.width = if compact {
                    Val::Percent(100.0)
                } else {
                    Val::Percent(31.0)
                };
                node.flex_grow = if compact { 0.0 } else { 1.0 };
                node.flex_shrink = if compact { 0.0 } else { 1.0 };
            }
            SandboxResponsiveNode::RosterGrid => {
                node.flex_direction = if compact {
                    FlexDirection::Column
                } else {
                    FlexDirection::Row
                };
                node.flex_wrap = if compact {
                    FlexWrap::NoWrap
                } else {
                    FlexWrap::Wrap
                };
            }
            SandboxResponsiveNode::RosterSlot => {
                node.width = if compact {
                    Val::Percent(100.0)
                } else {
                    Val::Percent(47.0)
                };
            }
            SandboxResponsiveNode::PickerLayout => {
                node.flex_direction = if compact {
                    FlexDirection::Column
                } else {
                    FlexDirection::Row
                };
                // The semantic scale enlarges copy without changing Bevy's authored
                // pixel caps. Scale the desktop composition with it so a 4K/200%
                // canvas retains the same usable line lengths as 1080p/100%.
                node.max_width = Val::Px(1_320.0 * metrics.content_scale);
            }
            SandboxResponsiveNode::CharacterList => {
                node.width = if compact {
                    Val::Percent(100.0)
                } else {
                    Val::Percent(44.0)
                };
                node.flex_shrink = if compact { 0.0 } else { 1.0 };
            }
            SandboxResponsiveNode::CharacterPreview => {
                node.width = if compact {
                    Val::Percent(100.0)
                } else {
                    Val::Auto
                };
                node.flex_grow = if compact { 0.0 } else { 1.0 };
                node.flex_shrink = if compact { 0.0 } else { 1.0 };
                node.min_height = Val::Px(360.0 * metrics.content_scale);
            }
        }
    }
}

fn emit_intents(
    controls: Query<(&Interaction, &SandboxControl), Changed<Interaction>>,
    mut intents: MessageWriter<UiIntent>,
) {
    for (interaction, control) in &controls {
        if *interaction == Interaction::Pressed {
            intents.write(UiIntent::Sandbox(control.0.clone()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authored_and_generated_seed_copy_is_unambiguous() {
        assert_eq!(seed_label(None), "Authored");
        assert_eq!(seed_label(Some(42)), "Seed 42");
    }
}
