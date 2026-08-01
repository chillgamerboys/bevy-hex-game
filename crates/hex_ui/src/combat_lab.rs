//! Combat Lab setup and saved-report presentation.

use bevy::input_focus::tab_navigation::TabIndex;
use bevy::prelude::*;
use bevy::text::EditableText;
use bevy::ui::InteractionDisabled;
use bevy::ui_widgets::ScrollArea;
use hex_assets::{
    CombatLabMapCatalog, CombatLabMapDefinition, CombatLabRegionCenter, CombatRuleField,
    CombatRulesPreset, CombatRulesProfile, CombatSettings, CreationCellKind, CreationPresetCatalog,
    ElementCatalog, PresetAudience, SavedCharacter, SpellBook,
};
use hex_core::{Screen, COMBAT_LAB_FIXTURES};
use hex_gameplay_model::{
    LabTab, RosterChoice as ModelRosterChoice, SandboxStep, MAX_COMBAT_LAB_ROSTER,
};

use crate::{
    blurb, body_text_role, compact_glyph_role, display, element_color, fine, heading, label, panel,
    panel_node, responsive_control_role, row_button, screen_root, screen_root_node, short_name,
    CharacterBuildSummary, CombatLabIntent, CombatLabReportField, CombatLabReportsView,
    CombatLabRulesVariant, CombatLabScreenView, CreatorLibraryView, ResolvedUiMetrics, UiAssets,
    UiIntent, UiSystems, UiViewportClass, DANGER, FUSION_COLOR, LABEL,
};

const MAX_ROSTER: usize = MAX_COMBAT_LAB_ROSTER;
type RosterChoice = ModelRosterChoice<hex_assets::CustomCharacterId>;

#[derive(Component)]
struct LabRoot;

#[derive(Component)]
struct LabTabs;

#[derive(Component)]
struct LabResponsiveBody;

#[derive(Component)]
struct LabInnerScroll;

#[derive(Component, Debug, Clone, Copy)]
enum LabBodyPanel {
    Sidebar(f32),
    Main,
}

#[derive(Component)]
struct FixtureFilter;

#[derive(Component)]
struct FixtureCard {
    #[cfg(any(test, feature = "test-support"))]
    id: &'static str,
    searchable: String,
}

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        (
            render,
            apply_lab_screen_layout,
            emit_actions.in_set(UiSystems::EmitIntents),
            emit_text_changes.in_set(UiSystems::EmitIntents),
        )
            .run_if(in_state(Screen::CombatLab)),
    );
}

fn render(
    mut commands: Commands,
    roots: Query<Entity, With<LabRoot>>,
    view: Res<CombatLabScreenView>,
    assets: Res<UiAssets>,
    asset_server: Res<AssetServer>,
) {
    if !view.is_changed() {
        return;
    }
    for root in &roots {
        commands.entity(root).despawn();
    }
    if !view.active {
        return;
    }
    spawn_lab_ui(
        &mut commands,
        &assets,
        &view,
        &view.library,
        view.elements.as_ref(),
        view.spells.as_ref(),
        view.presets.as_ref(),
        view.maps.as_ref(),
        view.combat.as_ref(),
        &view.reports,
        &asset_server,
    );
}

fn emit_actions(
    clicked: Query<(&Interaction, &CombatLabIntent), Changed<Interaction>>,
    mut intents: MessageWriter<UiIntent>,
) {
    for (interaction, action) in &clicked {
        if *interaction == Interaction::Pressed {
            intents.write(UiIntent::CombatLab(action.clone()));
        }
    }
}

fn emit_text_changes(
    filters: Query<&EditableText, (Changed<EditableText>, With<FixtureFilter>)>,
    report_fields: Query<(&EditableText, &CombatLabReportField), Changed<EditableText>>,
    mut fixture_cards: Query<(&FixtureCard, &mut Node)>,
    mut intents: MessageWriter<UiIntent>,
) {
    for value in &filters {
        let filter = value.value().to_string().to_lowercase();
        for (card, mut node) in &mut fixture_cards {
            node.display = if filter.is_empty() || card.searchable.contains(&filter) {
                Display::Flex
            } else {
                Display::None
            };
        }
        intents.write(UiIntent::CombatLab(CombatLabIntent::SetFixtureFilter(
            value.value().to_string(),
        )));
    }
    for (value, field) in &report_fields {
        intents.write(UiIntent::CombatLab(CombatLabIntent::SetReportField(
            *field,
            value.value().to_string(),
        )));
    }
}

fn apply_lab_screen_layout(
    metrics: Res<ResolvedUiMetrics>,
    added_bodies: Query<(), Added<LabResponsiveBody>>,
    mut roots: Query<
        &mut Node,
        (
            With<LabRoot>,
            Without<LabTabs>,
            Without<LabResponsiveBody>,
            Without<LabBodyPanel>,
            Without<LabInnerScroll>,
        ),
    >,
    mut tabs: Query<
        &mut Node,
        (
            With<LabTabs>,
            Without<LabRoot>,
            Without<LabResponsiveBody>,
            Without<LabBodyPanel>,
            Without<LabInnerScroll>,
        ),
    >,
    mut bodies: Query<&mut Node, (With<LabResponsiveBody>, Without<LabRoot>, Without<LabTabs>)>,
    mut panels: Query<
        (&LabBodyPanel, &mut Node),
        (
            Without<LabResponsiveBody>,
            Without<LabRoot>,
            Without<LabTabs>,
            Without<LabInnerScroll>,
        ),
    >,
    mut inner_scrolls: Query<
        &mut Node,
        (
            With<LabInnerScroll>,
            Without<LabResponsiveBody>,
            Without<LabBodyPanel>,
            Without<LabRoot>,
            Without<LabTabs>,
        ),
    >,
    mut controls: Query<
        &mut Node,
        (
            Or<(With<CombatLabIntent>, With<InteractionDisabled>)>,
            Without<LabRoot>,
            Without<LabTabs>,
            Without<LabResponsiveBody>,
            Without<LabBodyPanel>,
            Without<LabInnerScroll>,
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
    for mut node in &mut tabs {
        node.flex_wrap = if compact {
            FlexWrap::Wrap
        } else {
            FlexWrap::NoWrap
        };
        node.row_gap = if compact { Val::Px(6.0) } else { Val::ZERO };
    }
    for mut node in &mut bodies {
        node.flex_direction = if compact {
            FlexDirection::Column
        } else {
            FlexDirection::Row
        };
        node.overflow = if compact {
            Overflow::visible()
        } else {
            Overflow::default()
        };
        node.height = if compact { Val::Auto } else { Val::Px(0.0) };
        node.flex_grow = if compact { 0.0 } else { 1.0 };
    }
    for (role, mut node) in &mut panels {
        node.width = if compact {
            Val::Percent(100.0)
        } else {
            match role {
                LabBodyPanel::Sidebar(width) => Val::Px(*width),
                LabBodyPanel::Main => Val::Auto,
            }
        };
        node.height = Val::Auto;
    }
    for mut node in &mut inner_scrolls {
        node.flex_grow = if compact { 0.0 } else { 1.0 };
        node.overflow = if compact {
            Overflow::visible()
        } else {
            Overflow::scroll_y()
        };
    }
    for mut node in &mut controls {
        node.max_width = if compact {
            Val::Percent(100.0)
        } else {
            Val::Auto
        };
        node.min_width = if compact { Val::Px(0.0) } else { Val::Auto };
    }
}

fn spawn_lab_ui(
    commands: &mut Commands,
    assets: &UiAssets,
    state: &CombatLabScreenView,
    store: &CreatorLibraryView,
    elements: Option<&ElementCatalog>,
    spells: Option<&SpellBook>,
    presets: Option<&CreationPresetCatalog>,
    maps: Option<&CombatLabMapCatalog>,
    combat: Option<&CombatSettings>,
    reports: &CombatLabReportsView,
    asset_server: &AssetServer,
) {
    commands
        .spawn((
            screen_root(Screen::CombatLab, "Combat Lab Screen"),
            LabRoot,
            ScrollArea,
            ScrollPosition::default(),
        ))
        .insert(Node {
            padding: UiRect::all(Val::Px(18.0)),
            justify_content: JustifyContent::FlexStart,
            ..screen_root_node()
        })
        .with_children(|root| {
            root.spawn(display(assets, "Combat Lab"));
            root.spawn((
                LabTabs,
                Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(8.0),
                    ..default()
                },
            ))
            .with_children(|tabs| {
                lab_button(
                    tabs,
                    assets,
                    "Sandbox",
                    CombatLabIntent::Tab(LabTab::Sandbox),
                    170.0,
                );
                lab_button(
                    tabs,
                    assets,
                    "Fixed Fixtures",
                    CombatLabIntent::Tab(LabTab::Fixtures),
                    170.0,
                );
                lab_button(
                    tabs,
                    assets,
                    "Saved Reports",
                    CombatLabIntent::Tab(LabTab::Reports),
                    170.0,
                );
                lab_button(tabs, assets, "Back", CombatLabIntent::Back, 100.0);
            });
            if !state.notice.is_empty() {
                root.spawn(blurb(assets, state.notice.clone()));
            }
            match state.tab {
                LabTab::Sandbox => {
                    spawn_sandbox_progress(root, assets, state.sandbox_step);
                    match state.sandbox_step {
                        SandboxStep::Map => {
                            spawn_map_setup(root, assets, state, maps, asset_server);
                        }
                        SandboxStep::Rosters => spawn_sandbox_setup(
                            root,
                            assets,
                            state,
                            store,
                            elements,
                            spells,
                            presets,
                            maps,
                            asset_server,
                        ),
                        SandboxStep::Rules => {
                            spawn_rules_setup(root, assets, state, maps, combat);
                        }
                    }
                    spawn_sandbox_footer(root, assets, state, maps, combat);
                }
                LabTab::Fixtures => spawn_fixture_selector(root, assets, state),
                LabTab::Reports => spawn_saved_reports(root, assets, state, reports),
            }
        });
}

fn lab_button(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    text: impl Into<String>,
    action: CombatLabIntent,
    width: f32,
) {
    let text = text.into();
    parent
        .spawn((row_button(text.clone(), width), action))
        .with_child(label(assets, text));
}

fn disabled_lab_button(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    text: impl Into<String>,
    reason: impl Into<String>,
    width: f32,
) {
    let text = text.into();
    parent
        .spawn((row_button(text.clone(), width), InteractionDisabled))
        .insert(BorderColor::all(Color::srgba(0.35, 0.36, 0.39, 0.72)))
        .with_children(|button| {
            button.spawn(label(assets, text));
            button.spawn(fine(assets, reason.into()));
        });
}

fn spawn_sandbox_footer(
    root: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    state: &CombatLabScreenView,
    maps: Option<&CombatLabMapCatalog>,
    shipped: Option<&CombatSettings>,
) {
    root.spawn((
        Name::new("Combat Lab Step Actions"),
        Node {
            width: Val::Percent(96.0),
            min_height: Val::Px(52.0),
            flex_shrink: 0.0,
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::Wrap,
            justify_content: JustifyContent::FlexEnd,
            align_items: AlignItems::Center,
            column_gap: Val::Px(8.0),
            row_gap: Val::Px(6.0),
            ..default()
        },
    ))
    .with_children(|footer| match state.sandbox_step {
        SandboxStep::Map => {
            if maps.is_some_and(|catalog| catalog.get(&state.map).is_some()) {
                lab_button(
                    footer,
                    assets,
                    "Continue to Rosters",
                    CombatLabIntent::ShowSandboxStep(SandboxStep::Rosters),
                    220.0,
                );
            } else {
                disabled_lab_button(
                    footer,
                    assets,
                    "Continue to Rosters",
                    "Choose a loaded map",
                    220.0,
                );
            }
        }
        SandboxStep::Rosters => {
            lab_button(
                footer,
                assets,
                "Back to Map",
                CombatLabIntent::ShowSandboxStep(SandboxStep::Map),
                170.0,
            );
            let ready = !state.players.is_empty()
                && !state.hostiles.is_empty()
                && state.players.len() <= MAX_ROSTER
                && state.hostiles.len() <= MAX_ROSTER;
            if ready {
                lab_button(
                    footer,
                    assets,
                    "Continue to Rules",
                    CombatLabIntent::ShowSandboxStep(SandboxStep::Rules),
                    230.0,
                );
            } else {
                disabled_lab_button(
                    footer,
                    assets,
                    "Continue to Rules",
                    "Each side needs 1–6 Map-ready characters",
                    300.0,
                );
            }
        }
        SandboxStep::Rules => {
            lab_button(
                footer,
                assets,
                "Back to Rosters",
                CombatLabIntent::ShowSandboxStep(SandboxStep::Rosters),
                190.0,
            );
            if shipped.is_some() {
                lab_button(
                    footer,
                    assets,
                    "Load Map & Deploy",
                    CombatLabIntent::PrepareDeployment,
                    210.0,
                );
            } else {
                disabled_lab_button(
                    footer,
                    assets,
                    "Load Map & Deploy",
                    "Combat rules are still loading",
                    250.0,
                );
            }
        }
    });
}

fn map_button(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    map: &CombatLabMapDefinition,
    selected: bool,
) {
    let text = if selected {
        format!("SELECTED · {}", map.display_name)
    } else {
        map.display_name.clone()
    };
    parent
        .spawn((
            row_button(map.display_name.clone(), 280.0),
            CombatLabIntent::SelectMap(map.id.clone()),
        ))
        .insert(BorderColor::all(if selected {
            Color::srgba(0.49, 0.68, 0.86, 1.0)
        } else {
            Color::srgba(0.26, 0.29, 0.34, 0.9)
        }))
        .with_child(label(assets, text));
}

fn map_seed_label(map: &CombatLabMapDefinition) -> String {
    map.fixed_seed.map_or_else(
        || "Authored / embedded seed".to_owned(),
        |seed| format!("Seed {seed}"),
    )
}

fn deployment_summary(map: &CombatLabMapDefinition) -> String {
    format!(
        "Deploy P {} r{} · H {} r{}",
        region_center_label(&map.player_region.center),
        map.player_region.radius,
        region_center_label(&map.hostile_region.center),
        map.hostile_region.radius,
    )
}

fn region_center_label(center: &CombatLabRegionCenter) -> String {
    match center {
        CombatLabRegionCenter::Fixed(coord) => {
            format!("({},{},{})", coord.x, coord.y, coord.z)
        }
        CombatLabRegionCenter::Anchor(anchor) => format!("@{anchor}"),
    }
}

fn spawn_sandbox_progress(
    root: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    current: SandboxStep,
) {
    root.spawn(Node {
        flex_direction: FlexDirection::Row,
        column_gap: Val::Px(10.0),
        ..default()
    })
    .with_children(|progress| {
        for (step, text) in [
            (SandboxStep::Map, "1 · MAP"),
            (SandboxStep::Rosters, "2 · ROSTERS"),
            (SandboxStep::Rules, "3 · RULES"),
        ] {
            let status = if step == current { "ACTIVE" } else { "STEP" };
            progress
                .spawn(fine(assets, format!("{status} · {text}")))
                .insert(TextColor(if step == current {
                    Color::srgba(0.93, 0.79, 0.46, 1.0)
                } else {
                    Color::srgba(0.67, 0.71, 0.77, 1.0)
                }));
        }
        progress.spawn(fine(assets, "4 · DEPLOY"));
    });
}

fn spawn_map_setup(
    root: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    state: &CombatLabScreenView,
    maps: Option<&CombatLabMapCatalog>,
    asset_server: &AssetServer,
) {
    root.spawn((
        LabResponsiveBody,
        ScrollArea,
        Node {
            width: Val::Percent(96.0),
            min_height: Val::Px(0.0),
            flex_grow: 1.0,
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(12.0),
            ..default()
        },
    ))
    .with_children(|body| {
        body.spawn((panel(), LabBodyPanel::Sidebar(360.0)))
            .insert(Node {
                width: Val::Px(360.0),
                min_height: Val::Px(0.0),
                ..panel_node()
            })
            .with_children(|list| {
                list.spawn(heading(assets, "1 · choose map"));
                list.spawn(blurb(
                    assets,
                    "The selected map and resolved seed are frozen into every run and report.",
                ));
                list.spawn((
                    LabInnerScroll,
                    ScrollArea,
                    Node {
                        min_height: Val::Px(0.0),
                        flex_grow: 1.0,
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(6.0),
                        overflow: Overflow::scroll_y(),
                        ..default()
                    },
                ))
                .with_children(|buttons| {
                    if let Some(maps) = maps {
                        for map in &maps.maps {
                            map_button(buttons, assets, map, state.map == map.id);
                        }
                    }
                });
            });
        body.spawn((panel(), LabBodyPanel::Main))
            .insert(Node {
                min_width: Val::Px(0.0),
                min_height: Val::Px(0.0),
                flex_grow: 1.0,
                ..panel_node()
            })
            .with_children(|preview| {
                preview.spawn(heading(assets, "frozen map preview"));
                if let Some(record) = maps.and_then(|catalog| catalog.get(&state.map)) {
                    preview.spawn((
                        Name::new(format!("Map Preview: {}", record.display_name)),
                        ImageNode::new(asset_server.load(record.preview.clone())),
                        Node {
                            width: Val::Percent(100.0),
                            max_width: Val::Px(720.0),
                            height: Val::Px(360.0),
                            border: UiRect::all(Val::Px(1.0)),
                            ..default()
                        },
                        BorderColor::all(Color::srgba(0.49, 0.68, 0.86, 0.85)),
                    ));
                    preview.spawn(heading(assets, record.display_name.clone()));
                    preview.spawn(fine(assets, record.tags.join("  ·  ")));
                    preview.spawn(blurb(assets, record.description.clone()));
                    preview.spawn(fine(
                        assets,
                        format!(
                            "{}  ·  {}",
                            map_seed_label(record),
                            deployment_summary(record)
                        ),
                    ));
                } else {
                    preview
                        .spawn(blurb(assets, "The packaged map catalog is still loading."))
                        .insert(TextColor(DANGER));
                }
            });
    });
}

fn spawn_rules_setup(
    root: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    state: &CombatLabScreenView,
    maps: Option<&CombatLabMapCatalog>,
    shipped: Option<&CombatSettings>,
) {
    let Some(shipped) = shipped else {
        root.spawn(blurb(assets, "Shipped combat rules are still loading."))
            .insert(TextColor(DANGER));
        return;
    };
    let profile = state
        .rules
        .clone()
        .unwrap_or_else(|| CombatRulesProfile::shipped(shipped));
    let changes = profile.changed_from_shipped(shipped);
    root.spawn((
        LabResponsiveBody,
        ScrollArea,
        Node {
            width: Val::Percent(96.0),
            min_height: Val::Px(0.0),
            flex_grow: 1.0,
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(12.0),
            ..default()
        },
    ))
    .with_children(|body| {
        body.spawn((panel(), LabBodyPanel::Sidebar(285.0)))
            .insert(Node {
                width: Val::Px(285.0),
                ..panel_node()
            })
            .with_children(|summary| {
                summary.spawn(heading(assets, "frozen run summary"));
                let map = maps
                    .and_then(|catalog| catalog.get(&state.map))
                    .map_or("Loading map", |map| map.display_name.as_str());
                summary.spawn(blurb(
                    assets,
                    format!(
                        "{map}\nPlayer {} · Hostile {}\n{} field{} changed from shipped",
                        state.players.len(),
                        state.hostiles.len(),
                        changes.len(),
                        if changes.len() == 1 { "" } else { "s" }
                    ),
                ));
                for change in &changes {
                    summary.spawn(fine(
                        assets,
                        format!(
                            "CHANGED · {} {} → {}",
                            change.field.label(),
                            change.shipped,
                            change.selected
                        ),
                    ));
                }
            });
        body.spawn((panel(), LabBodyPanel::Main))
            .insert(Node {
                min_width: Val::Px(0.0),
                min_height: Val::Px(0.0),
                flex_grow: 1.0,
                ..panel_node()
            })
            .with_children(|rules| {
                rules.spawn(heading(assets, "3 · rules profile"));
                rules
                    .spawn(Node {
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(7.0),
                        ..default()
                    })
                    .with_children(|presets| {
                        for (preset, text) in [
                            (CombatRulesPreset::Shipped, "Shipped"),
                            (CombatRulesPreset::TacticalTwoStep, "Tactical two-step"),
                            (CombatRulesPreset::Custom, "Custom"),
                        ] {
                            let selected = profile.preset == preset;
                            lab_button(
                                presets,
                                assets,
                                if selected {
                                    format!("SELECTED · {text}")
                                } else {
                                    text.to_owned()
                                },
                                CombatLabIntent::SelectRulesPreset(preset),
                                205.0,
                            );
                        }
                    });
                rules
                    .spawn((
                        LabInnerScroll,
                        ScrollArea,
                        Node {
                            min_height: Val::Px(0.0),
                            flex_grow: 1.0,
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px(5.0),
                            overflow: Overflow::scroll_y(),
                            ..default()
                        },
                    ))
                    .with_children(|fields| {
                        for field in CombatRuleField::ALL {
                            let bounds = field.bounds();
                            let value = profile.value(field);
                            let shipped_value = CombatRulesProfile::shipped(shipped).value(field);
                            fields
                                .spawn(panel())
                                .insert(Node {
                                    width: Val::Percent(100.0),
                                    ..panel_node()
                                })
                                .with_children(|row| {
                                    row.spawn(heading(assets, field.label()));
                                    row.spawn(fine(assets, field.description()));
                                    row.spawn(Node {
                                        flex_direction: FlexDirection::Row,
                                        align_items: AlignItems::Center,
                                        column_gap: Val::Px(7.0),
                                        ..default()
                                    })
                                    .with_children(
                                        |stepper| {
                                            lab_button(
                                                stepper,
                                                assets,
                                                "−",
                                                CombatLabIntent::AdjustRule(field, -1),
                                                46.0,
                                            );
                                            stepper.spawn(label(assets, value.to_string()));
                                            lab_button(
                                                stepper,
                                                assets,
                                                "+",
                                                CombatLabIntent::AdjustRule(field, 1),
                                                46.0,
                                            );
                                            stepper.spawn(fine(
                                                assets,
                                                format!(
                                                    "VALID {}–{} · {}",
                                                    bounds.min,
                                                    bounds.max,
                                                    if value == shipped_value {
                                                        "SHIPPED".to_owned()
                                                    } else {
                                                        format!(
                                                            "CHANGED {} → {}",
                                                            shipped_value, value
                                                        )
                                                    }
                                                ),
                                            ));
                                        },
                                    );
                                });
                        }
                    });
                rules
                    .spawn(Node {
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(7.0),
                        ..default()
                    })
                    .with_children(|actions| {
                        lab_button(
                            actions,
                            assets,
                            "Reset to Shipped",
                            CombatLabIntent::ResetRules,
                            180.0,
                        );
                    });
            });
    });
}

fn spawn_sandbox_setup(
    root: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    state: &CombatLabScreenView,
    store: &CreatorLibraryView,
    elements: Option<&ElementCatalog>,
    spells: Option<&SpellBook>,
    presets: Option<&CreationPresetCatalog>,
    maps: Option<&CombatLabMapCatalog>,
    asset_server: &AssetServer,
) {
    root.spawn((
        LabResponsiveBody,
        ScrollArea,
        Node {
            width: Val::Percent(96.0),
            height: Val::Px(0.0),
            min_height: Val::Px(0.0),
            flex_grow: 1.0,
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(12.0),
            ..default()
        },
    ))
    .with_children(|body| {
        body.spawn((panel(), LabBodyPanel::Sidebar(320.0)))
            .insert(Node {
                width: Val::Px(320.0),
                min_height: Val::Px(0.0),
                ..panel_node()
            })
            .with_children(|map_panel| {
                map_panel.spawn(heading(assets, "selected map"));
                if let Some(record) = maps.and_then(|catalog| catalog.get(&state.map)) {
                    map_panel.spawn((
                        Name::new(format!("Map Preview: {}", record.display_name)),
                        ImageNode::new(asset_server.load(record.preview.clone())),
                        Node {
                            width: Val::Px(280.0),
                            height: Val::Px(158.0),
                            border: UiRect::all(Val::Px(1.0)),
                            ..default()
                        },
                        BorderColor::all(Color::srgba(0.49, 0.68, 0.86, 0.85)),
                    ));
                    map_panel.spawn(heading(assets, record.display_name.clone()));
                    map_panel.spawn(fine(assets, record.tags.join("  ·  ")));
                    map_panel.spawn(blurb(assets, record.description.clone()));
                    map_panel.spawn(fine(
                        assets,
                        format!(
                            "{}  ·  {}",
                            map_seed_label(record),
                            deployment_summary(record)
                        ),
                    ));
                } else {
                    map_panel
                        .spawn(blurb(assets, "The packaged map catalog is still loading."))
                        .insert(TextColor(DANGER));
                }
            });

        body.spawn((panel(), LabBodyPanel::Main))
            .insert(Node {
                min_width: Val::Px(0.0),
                min_height: Val::Px(0.0),
                flex_grow: 1.0,
                ..panel_node()
            })
            .with_children(|rosters| {
                let map_label = maps
                    .and_then(|catalog| catalog.get(&state.map))
                    .map_or("Loading map", |record| record.display_name.as_str());
                rosters.spawn(heading(assets, format!("{map_label} · rosters")));
                rosters
                    .spawn(Node {
                        min_height: Val::Px(0.0),
                        flex_grow: 1.0,
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(12.0),
                        ..default()
                    })
                    .with_children(|columns| {
                        spawn_roster_column(
                            columns,
                            assets,
                            "Player",
                            &state.players,
                            true,
                            store,
                            elements,
                            spells,
                            presets,
                            &state.map_ready_choices,
                        );
                        spawn_roster_column(
                            columns,
                            assets,
                            "Hostile · baseline AI",
                            &state.hostiles,
                            false,
                            store,
                            elements,
                            spells,
                            presets,
                            &state.map_ready_choices,
                        );
                    });
                if state.players.is_empty()
                    || state.hostiles.is_empty()
                    || state.players.len() > MAX_ROSTER
                    || state.hostiles.len() > MAX_ROSTER
                {
                    rosters
                        .spawn(blurb(assets, "Each side needs 1–6 Map-ready characters."))
                        .insert(TextColor(DANGER));
                }
            });
    });
}

#[expect(
    clippy::too_many_arguments,
    reason = "one roster column presents both packaged and saved choices"
)]
fn spawn_roster_column(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    title: &str,
    roster: &[RosterChoice],
    player: bool,
    store: &CreatorLibraryView,
    elements: Option<&ElementCatalog>,
    spells: Option<&SpellBook>,
    presets: Option<&CreationPresetCatalog>,
    map_ready_choices: &[RosterChoice],
) {
    parent
        .spawn(panel())
        .insert(Node {
            min_width: Val::Px(0.0),
            min_height: Val::Px(0.0),
            flex_grow: 1.0,
            ..panel_node()
        })
        .with_children(|column| {
            column.spawn(heading(assets, title));
            column
                .spawn((
                    LabInnerScroll,
                    ScrollArea,
                    Node {
                        min_height: Val::Px(0.0),
                        flex_grow: 1.0,
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(7.0),
                        overflow: Overflow::scroll_y(),
                        ..default()
                    },
                ))
                .with_children(|list| {
                    for (index, choice) in roster.iter().enumerate() {
                        let (up, down, remove) = if player {
                            (
                                CombatLabIntent::MovePlayer(index, -1),
                                CombatLabIntent::MovePlayer(index, 1),
                                CombatLabIntent::RemovePlayer(index),
                            )
                        } else {
                            (
                                CombatLabIntent::MoveHostile(index, -1),
                                CombatLabIntent::MoveHostile(index, 1),
                                CombatLabIntent::RemoveHostile(index),
                            )
                        };
                        spawn_build_card(
                            list,
                            assets,
                            choice,
                            store,
                            presets,
                            elements,
                            spells,
                            Some((index + 1, up, down, remove)),
                            None,
                            map_ready_choices,
                        );
                    }
                    if roster.len() < MAX_ROSTER {
                        list.spawn(fine(assets, "ADD TEMPLATE"));
                        for template in ["wolf", "raider", "hedge-mage"] {
                            spawn_build_card(
                                list,
                                assets,
                                &RosterChoice::Template(template.to_owned()),
                                store,
                                presets,
                                elements,
                                spells,
                                None,
                                Some(if player {
                                    CombatLabIntent::AddPlayerTemplate(template.to_owned())
                                } else {
                                    CombatLabIntent::AddHostileTemplate(template.to_owned())
                                }),
                                map_ready_choices,
                            );
                        }
                        list.spawn(fine(assets, "ADD SAVED CHARACTER"));
                        for character in &store.file.characters {
                            let ready =
                                map_ready_choices.contains(&RosterChoice::Custom(character.id));
                            spawn_build_card(
                                list,
                                assets,
                                &RosterChoice::Custom(character.id),
                                store,
                                presets,
                                elements,
                                spells,
                                None,
                                ready.then_some(if player {
                                    CombatLabIntent::AddPlayerCustom(character.id)
                                } else {
                                    CombatLabIntent::AddHostileCustom(character.id)
                                }),
                                map_ready_choices,
                            );
                        }
                    }
                });
        });
}

#[expect(
    clippy::too_many_arguments,
    reason = "the shared roster card renders a complete frozen build projection"
)]
fn spawn_build_card(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    choice: &RosterChoice,
    store: &CreatorLibraryView,
    presets: Option<&CreationPresetCatalog>,
    elements: Option<&ElementCatalog>,
    spells: Option<&SpellBook>,
    roster_actions: Option<(usize, CombatLabIntent, CombatLabIntent, CombatLabIntent)>,
    add_action: Option<CombatLabIntent>,
    map_ready_choices: &[RosterChoice],
) {
    let Some((character, library, source)) = choice_record(choice, store, presets) else {
        parent
            .spawn(blurb(
                assets,
                format!("Missing record: {}", choice_name(choice, store)),
            ))
            .insert(TextColor(DANGER));
        return;
    };
    let summary = CharacterBuildSummary::from_saved(&character, &library, elements, spells);
    let ready = summary.ready()
        && match choice {
            RosterChoice::Template(_) => true,
            RosterChoice::Custom(_) | RosterChoice::Packaged(_) => {
                map_ready_choices.contains(choice)
            }
        };
    parent
        .spawn(panel())
        .insert((
            Node {
                width: Val::Percent(100.0),
                padding: UiRect::all(Val::Px(8.0)),
                row_gap: Val::Px(3.0),
                ..panel_node()
            },
            BorderColor::all(if ready {
                Color::srgba(0.93, 0.79, 0.46, 0.42)
            } else {
                Color::srgba(0.94, 0.36, 0.30, 0.65)
            }),
        ))
        .with_children(|card| {
            card.spawn(Node {
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(9.0),
                ..default()
            })
            .with_children(|top| {
                spawn_mini_lattice(top, assets, &character, elements);
                top.spawn(Node {
                    min_width: Val::Px(0.0),
                    flex_grow: 1.0,
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(2.0),
                    ..default()
                })
                .with_children(|text| {
                    text.spawn(heading(
                        assets,
                        roster_actions.as_ref().map_or_else(
                            || summary.name.clone(),
                            |(slot, _, _, _)| format!("{slot}. {}", summary.name),
                        ),
                    ));
                    text.spawn(fine(
                        assets,
                        format!("{source} · {}", summary.compact_line()),
                    ));
                    if !summary.attunement.is_empty() {
                        text.spawn(fine(
                            assets,
                            format!("Attunement / channel · {}", summary.attunement.join(", ")),
                        ));
                    }
                    for spell in &summary.spells {
                        text.spawn(fine(assets, format!("{} · {}", spell.name, spell.sentence)));
                    }
                });
            });
            if !ready {
                let reason = if summary.issues.is_empty() {
                    "Needs at least one supported, fresh-cast spell.".to_owned()
                } else {
                    summary.issues.join(" · ")
                };
                card.spawn(fine(assets, format!("BLOCKED · {reason}")))
                    .insert(TextColor(DANGER));
            }
            card.spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(4.0),
                ..default()
            })
            .with_children(|actions| {
                if let Some((_, up, down, remove)) = roster_actions {
                    lab_button(actions, assets, "↑", up, 42.0);
                    lab_button(actions, assets, "↓", down, 42.0);
                    lab_button(actions, assets, "Remove", remove, 78.0);
                } else if let Some(add) = add_action {
                    lab_button(actions, assets, "Add to roster", add, 132.0);
                } else if !ready {
                    if let RosterChoice::Custom(id) = choice {
                        lab_button(
                            actions,
                            assets,
                            "Edit in Creator",
                            CombatLabIntent::EditCustom(*id),
                            142.0,
                        );
                    }
                }
            });
        });
}

fn choice_record(
    choice: &RosterChoice,
    store: &CreatorLibraryView,
    presets: Option<&CreationPresetCatalog>,
) -> Option<(
    SavedCharacter,
    hex_assets::CreationLibraryFile,
    &'static str,
)> {
    match choice {
        RosterChoice::Custom(id) => store
            .file
            .characters
            .iter()
            .find(|character| character.id == *id)
            .cloned()
            .map(|character| (character, store.file.clone(), "Custom")),
        RosterChoice::Packaged(id) => {
            let presets = presets?;
            let library = presets.library_for(PresetAudience::AutomationFixture);
            library
                .characters
                .iter()
                .find(|character| character.id == *id)
                .cloned()
                .map(|character| (character, library, "Fixture"))
        }
        RosterChoice::Template(name) => {
            let presets = presets?;
            let library = presets.library_for(PresetAudience::HumanTemplate);
            presets
                .characters
                .iter()
                .find(|record| {
                    record.audience == PresetAudience::HumanTemplate
                        && record.key == format!("template-{name}")
                })
                .map(|record| (record.character.clone(), library, "Template"))
        }
    }
}

#[expect(
    clippy::cast_precision_loss,
    reason = "creator coordinates are schema-bounded to 64 cells and miniature layout uses pixels"
)]
fn spawn_mini_lattice(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    character: &SavedCharacter,
    elements: Option<&ElementCatalog>,
) {
    let cell_width = 20.0;
    let cell_height = 23.0;
    parent
        .spawn(Node {
            width: Val::Px(102.0),
            height: Val::Px(76.0),
            position_type: PositionType::Relative,
            flex_shrink: 0.0,
            ..default()
        })
        .with_children(|canvas| {
            for cell in &character.cells {
                let x = 40.0 + (cell.q as f32 + cell.r as f32 * 0.5) * cell_width * 0.88;
                let y = 27.0 + cell.r as f32 * cell_height * 0.74;
                let (color, text) = match &cell.kind {
                    CreationCellKind::Gem(name) => (
                        elements
                            .map(|catalog| element_color(catalog.id(name), catalog))
                            .unwrap_or(Color::srgb(0.16, 0.45, 0.52)),
                        short_name(name),
                    ),
                    CreationCellKind::Fusion(name) => (FUSION_COLOR, short_name(name)),
                    CreationCellKind::Spell(_) => {
                        (Color::srgba(0.86, 0.80, 0.62, 0.94), "S".to_owned())
                    }
                    CreationCellKind::Blank => {
                        (Color::srgba(0.36, 0.38, 0.42, 0.88), "·".to_owned())
                    }
                };
                canvas
                    .spawn((
                        ImageNode::new(assets.hex_cell.clone()).with_color(color),
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(x),
                            top: Val::Px(y),
                            width: Val::Px(cell_width),
                            height: Val::Px(cell_height),
                            align_items: AlignItems::Center,
                            justify_content: JustifyContent::Center,
                            ..default()
                        },
                    ))
                    .with_child((
                        Text::new(text),
                        compact_glyph_role(7.0),
                        TextFont {
                            font: assets.body.clone().into(),
                            ..TextFont::from_font_size(7.0)
                        },
                        TextColor(Color::BLACK),
                    ));
            }
        });
}

fn spawn_fixture_selector(
    root: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    state: &CombatLabScreenView,
) {
    root.spawn(panel())
        .insert(Node {
            width: Val::Percent(88.0),
            min_height: Val::Px(0.0),
            flex_grow: 1.0,
            ..panel_node()
        })
        .with_children(|fixture_panel| {
            fixture_panel.spawn(heading(assets, "fixed deterministic fixtures"));
            fixture_panel.spawn(blurb(
                assets,
                "Immutable map, seed, roster, AI, and placement. Local creations are never read.",
            ));
            fixture_panel.spawn((
                Name::new("Fixture Search"),
                AccessibleLabel::new("Search fixed Combat Lab fixtures"),
                TabIndex(0),
                EditableText {
                    max_characters: Some(48),
                    visible_width: Some(32.0),
                    ..EditableText::new(&state.fixture_filter)
                },
                body_text_role(),
                responsive_control_role(),
                TextFont {
                    font: assets.body.clone().into(),
                    ..TextFont::from_font_size(18.0)
                },
                TextColor(Color::WHITE),
                BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.08)),
                Node {
                    width: Val::Percent(100.0),
                    min_height: Val::Px(44.0),
                    padding: UiRect::all(Val::Px(8.0)),
                    ..default()
                },
                FixtureFilter,
            ));
            fixture_panel
                .spawn((
                    LabInnerScroll,
                    ScrollArea,
                    Node {
                        min_height: Val::Px(0.0),
                        flex_grow: 1.0,
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(8.0),
                        overflow: Overflow::scroll_y(),
                        ..default()
                    },
                ))
                .with_children(|list| {
                    let filter = state.fixture_filter.to_lowercase();
                    for fixture in COMBAT_LAB_FIXTURES {
                        let searchable = format!(
                            "{} {} {} {} {} {}",
                            fixture.id,
                            fixture.name,
                            fixture.tags,
                            fixture.description,
                            fixture.map_seed,
                            fixture.roster
                        )
                        .to_lowercase();
                        let visible = filter.is_empty() || searchable.contains(&filter);
                        list.spawn((
                            panel(),
                            FixtureCard {
                                #[cfg(any(test, feature = "test-support"))]
                                id: fixture.id,
                                searchable,
                            },
                        ))
                        .insert(Node {
                            width: Val::Percent(100.0),
                            display: if visible {
                                Display::Flex
                            } else {
                                Display::None
                            },
                            ..panel_node()
                        })
                        .with_children(|card| {
                            card.spawn(heading(assets, fixture.name));
                            card.spawn(fine(assets, format!("{} · {}", fixture.id, fixture.tags)));
                            card.spawn(fine(
                                assets,
                                format!("{} · {}", fixture.map_seed, fixture.roster),
                            ));
                            card.spawn(blurb(assets, fixture.description));
                            if fixture.profile_matrix {
                                for (variant, label) in [
                                    (CombatLabRulesVariant::Shipped, "Run Shipped"),
                                    (
                                        CombatLabRulesVariant::TacticalTwoStep,
                                        "Run Tactical two-step",
                                    ),
                                    (
                                        CombatLabRulesVariant::CustomThreeStep,
                                        "Run Custom three-step",
                                    ),
                                ] {
                                    lab_button(
                                        card,
                                        assets,
                                        label,
                                        CombatLabIntent::StartFixture(
                                            fixture.id.to_owned(),
                                            variant,
                                        ),
                                        210.0,
                                    );
                                }
                            } else {
                                lab_button(
                                    card,
                                    assets,
                                    "Run Fixture",
                                    CombatLabIntent::StartFixture(
                                        fixture.id.to_owned(),
                                        CombatLabRulesVariant::Shipped,
                                    ),
                                    150.0,
                                );
                            }
                        });
                    }
                });
        });
}

fn choice_name(choice: &RosterChoice, store: &CreatorLibraryView) -> String {
    match choice {
        RosterChoice::Template(name) => format!("{name} · template"),
        RosterChoice::Custom(id) => store
            .file
            .characters
            .iter()
            .find(|saved| saved.id == *id)
            .map_or_else(|| format!("missing #{}", id.0), |saved| saved.name.clone()),
        RosterChoice::Packaged(id) => format!("fixture character #{}", id.0),
    }
}

fn spawn_saved_reports(
    root: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    _state: &CombatLabScreenView,
    reports: &CombatLabReportsView,
) {
    root.spawn(panel())
        .insert(Node {
            width: Val::Percent(96.0),
            min_height: Val::Px(0.0),
            flex_grow: 1.0,
            ..panel_node()
        })
        .with_children(|history| {
            history.spawn(heading(assets, "explicitly saved local reports"));
            history.spawn(blurb(
                assets,
                "Separate from Creator and Continue · fixed fixtures never consult this history.",
            ));
            if let Some(error) = &reports.error {
                history
                    .spawn(blurb(assets, error.clone()))
                    .insert(TextColor(DANGER));
            }
            if reports.reports.is_empty() {
                history.spawn(blurb(
                    assets,
                    "No saved reports. Finish a Lab run and choose Save Report.",
                ));
                return;
            }
            history
                .spawn((
                    LabInnerScroll,
                    ScrollArea,
                    Node {
                        min_height: Val::Px(0.0),
                        flex_grow: 1.0,
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(7.0),
                        overflow: Overflow::scroll_y(),
                        ..default()
                    },
                ))
                .with_children(|list| {
                    for report in &reports.reports {
                        list.spawn(panel())
                            .insert(Node {
                                width: Val::Percent(100.0),
                                ..panel_node()
                            })
                            .with_children(|card| {
                                card.spawn(heading(assets, report.heading.clone()));
                                card.spawn((
                                    Name::new(format!("Report {} Label", report.id.0)),
                                    AccessibleLabel::new(format!(
                                        "Report {} comparison label",
                                        report.id.0
                                    )),
                                    TabIndex(0),
                                    EditableText {
                                        max_characters: Some(128),
                                        visible_width: Some(32.0),
                                        ..EditableText::new(&report.label)
                                    },
                                    body_text_role(),
                                    responsive_control_role(),
                                    TextFont {
                                        font: assets.body.clone().into(),
                                        ..TextFont::from_font_size(18.0)
                                    },
                                    TextColor(Color::WHITE),
                                    BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.08)),
                                    Node {
                                        width: Val::Percent(100.0),
                                        min_height: Val::Px(44.0),
                                        padding: UiRect::all(Val::Px(7.0)),
                                        ..default()
                                    },
                                    CombatLabReportField::Label(report.id),
                                ));
                                card.spawn((
                                    Name::new(format!("Report {} Notes", report.id.0)),
                                    AccessibleLabel::new(format!("Report {} notes", report.id.0)),
                                    TabIndex(0),
                                    EditableText {
                                        max_characters: Some(2_048),
                                        visible_width: Some(52.0),
                                        ..EditableText::new(&report.notes)
                                    },
                                    body_text_role(),
                                    responsive_control_role(),
                                    TextFont {
                                        font: assets.body.clone().into(),
                                        ..TextFont::from_font_size(18.0)
                                    },
                                    TextColor(LABEL),
                                    BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.05)),
                                    Node {
                                        width: Val::Percent(100.0),
                                        min_height: Val::Px(44.0),
                                        padding: UiRect::all(Val::Px(7.0)),
                                        ..default()
                                    },
                                    CombatLabReportField::Notes(report.id),
                                ));
                                card.spawn(fine(assets, report.metadata.clone()));
                                card.spawn(blurb(assets, report.summary.clone()));
                                lab_button(
                                    card,
                                    assets,
                                    if report.left_selected {
                                        "LEFT · SELECTED"
                                    } else {
                                        "Use as Left"
                                    },
                                    CombatLabIntent::SelectCompareLeft(report.id),
                                    140.0,
                                );
                                lab_button(
                                    card,
                                    assets,
                                    if report.right_selected {
                                        "RIGHT · SELECTED"
                                    } else {
                                        "Use as Right"
                                    },
                                    CombatLabIntent::SelectCompareRight(report.id),
                                    140.0,
                                );
                                if report.pending_delete {
                                    card.spawn(fine(
                                        assets,
                                        "CONFIRM DELETE · this removes only this local report",
                                    ))
                                    .insert(TextColor(DANGER));
                                    lab_button(
                                        card,
                                        assets,
                                        "Confirm Delete",
                                        CombatLabIntent::ConfirmReportDelete(report.id),
                                        160.0,
                                    );
                                    lab_button(
                                        card,
                                        assets,
                                        "Cancel",
                                        CombatLabIntent::CancelReportDelete,
                                        100.0,
                                    );
                                } else {
                                    lab_button(
                                        card,
                                        assets,
                                        "Delete…",
                                        CombatLabIntent::RequestReportDelete(report.id),
                                        110.0,
                                    );
                                }
                            });
                    }
                });
            if let Some(comparison) = &reports.comparison {
                history.spawn(heading(assets, comparison.heading.clone()));
                history.spawn(fine(assets, comparison.frozen.clone()));
                history.spawn(blurb(assets, comparison.deltas.clone()));
            }
        });
}

/// Stable visible fixture identities for immutable headless observation.
#[cfg(feature = "test-support")]
pub(crate) fn visible_fixture_ids(world: &mut World) -> Vec<String> {
    let mut query = world.query::<(&FixtureCard, &Node)>();
    query
        .iter(world)
        .filter(|(_, node)| node.display != Display::None)
        .map(|(fixture, _)| fixture.id.to_owned())
        .collect()
}

/// Exercises the production fixture-filter presentation without application state.
#[cfg(feature = "test-support")]
pub(crate) fn observe_fixture_filter(query: &str) -> (Vec<String>, Vec<String>, bool) {
    let mut app = App::new();
    app.add_message::<UiIntent>()
        .add_systems(Update, emit_text_changes);
    let input = app
        .world_mut()
        .spawn((EditableText::new(query), FixtureFilter))
        .id();
    for fixture in COMBAT_LAB_FIXTURES {
        app.world_mut().spawn((
            Node::default(),
            FixtureCard {
                id: fixture.id,
                searchable: format!(
                    "{} {} {} {} {} {}",
                    fixture.id,
                    fixture.name,
                    fixture.tags,
                    fixture.description,
                    fixture.map_seed,
                    fixture.roster
                )
                .to_lowercase(),
            },
        ));
    }
    app.update();
    let mut visible = visible_fixture_ids(app.world_mut());
    visible.sort();

    app.world_mut()
        .entity_mut(input)
        .insert(EditableText::new(""));
    app.update();
    let mut after_clear = visible_fixture_ids(app.world_mut());
    after_clear.sort();
    let input_survived = app.world().get_entity(input).is_ok();
    (visible, after_clear, input_survived)
}

#[cfg(test)]
mod tests {
    use bevy::ecs::world::CommandQueue;
    use hex_gameplay_model::CombatLabReportId;

    use super::*;
    use crate::{CombatLabComparisonView, CombatLabReportCardView};

    fn assets() -> UiAssets {
        UiAssets {
            display: Handle::default(),
            body: Handle::default(),
            hex_cell: Handle::default(),
        }
    }

    #[test]
    fn fixture_selector_keeps_every_card_mounted_for_in_place_filtering() {
        let mut world = World::new();
        let state = CombatLabScreenView {
            tab: LabTab::Fixtures,
            fixture_filter: "tempo".to_owned(),
            ..default()
        };
        let mut queue = CommandQueue::default();
        let mut commands = Commands::new(&mut queue, &world);
        commands.spawn(Node::default()).with_children(|root| {
            spawn_fixture_selector(root, &assets(), &state);
        });
        queue.apply(&mut world);

        let mut cards = world.query::<(&FixtureCard, &Node)>();
        assert_eq!(cards.iter(&world).count(), COMBAT_LAB_FIXTURES.len());
        assert_eq!(
            cards
                .iter(&world)
                .filter(|(_, node)| node.display != Display::None)
                .map(|(card, _)| card.id)
                .collect::<Vec<_>>(),
            ["tempo-matrix"]
        );
    }

    #[test]
    fn rules_step_exposes_every_bounded_control_and_navigation_gate() {
        let mut world = World::new();
        let state = CombatLabScreenView {
            sandbox_step: SandboxStep::Rules,
            ..default()
        };
        let mut queue = CommandQueue::default();
        let mut commands = Commands::new(&mut queue, &world);
        commands.spawn(Node::default()).with_children(|root| {
            spawn_rules_setup(
                root,
                &assets(),
                &state,
                None,
                Some(&CombatSettings::default()),
            );
            spawn_sandbox_footer(
                root,
                &assets(),
                &state,
                None,
                Some(&CombatSettings::default()),
            );
        });
        queue.apply(&mut world);

        let mut presets = 0;
        let mut adjustments = 0;
        let mut resets = 0;
        let mut forwards = 0;
        let mut backs = 0;
        let mut actions = world.query::<&CombatLabIntent>();
        for action in actions.iter(&world) {
            match action {
                CombatLabIntent::SelectRulesPreset(_) => presets += 1,
                CombatLabIntent::AdjustRule(_, _) => adjustments += 1,
                CombatLabIntent::ResetRules => resets += 1,
                CombatLabIntent::PrepareDeployment => forwards += 1,
                CombatLabIntent::ShowSandboxStep(SandboxStep::Rosters) => backs += 1,
                _ => {}
            }
        }
        assert_eq!(presets, 3);
        assert_eq!(adjustments, CombatRuleField::ALL.len() * 2);
        assert_eq!((resets, forwards, backs), (1, 1, 1));
    }

    #[test]
    fn saved_reports_expose_independent_comparison_and_confirmed_delete() {
        let first = CombatLabReportId(1);
        let report = |id, pending_delete| CombatLabReportCardView {
            id,
            heading: format!("REPORT {}", id.0),
            label: String::new(),
            notes: String::new(),
            metadata: "frozen".to_owned(),
            summary: "summary".to_owned(),
            left_selected: id == first,
            right_selected: id != first,
            pending_delete,
        };
        let reports = CombatLabReportsView {
            error: None,
            reports: vec![report(first, true), report(CombatLabReportId(2), false)],
            comparison: Some(CombatLabComparisonView {
                heading: "compare reports 1 vs 2".to_owned(),
                frozen: "frozen left and right".to_owned(),
                deltas: "canonical deltas".to_owned(),
            }),
        };
        let mut world = World::new();
        let mut queue = CommandQueue::default();
        let mut commands = Commands::new(&mut queue, &world);
        commands.spawn(Node::default()).with_children(|root| {
            spawn_saved_reports(root, &assets(), &CombatLabScreenView::default(), &reports);
        });
        queue.apply(&mut world);

        let mut actions = world.query::<&CombatLabIntent>();
        let collected = actions.iter(&world).collect::<Vec<_>>();
        assert_eq!(
            collected
                .iter()
                .filter(|action| matches!(action, CombatLabIntent::SelectCompareLeft(_)))
                .count(),
            2
        );
        assert_eq!(
            collected
                .iter()
                .filter(|action| matches!(action, CombatLabIntent::SelectCompareRight(_)))
                .count(),
            2
        );
        assert!(collected.iter().any(
            |action| matches!(action, CombatLabIntent::ConfirmReportDelete(id) if *id == first)
        ));
        assert!(collected
            .iter()
            .any(|action| matches!(action, CombatLabIntent::CancelReportDelete)));
    }
}
