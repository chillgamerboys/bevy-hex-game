//! Primary title navigation and the separate development-scenario catalog.
//! Application code supplies immutable projections and handles typed intents.

use bevy::prelude::*;
use bevy::ui::InteractionDisabled;
use bevy::ui_widgets::{ControlOrientation, ScrollArea, Scrollbar, ScrollbarThumb};
use hex_core::Screen;

use crate::{
    blurb, button, despawn_screen, display, fine, heading, label, panel, screen_root,
    stacked_row_button, supporting_text_role, ResolvedUiMetrics, ResumeView, ScenarioBrowserIntent,
    ScenarioBrowserView, TitleIntent, TitleScenarioView, TitleView, UiAssets, UiIntent, UiSystems,
    UiViewportClass, ACCENT_EDGE, BLURB_SIZE, DANGER,
};

const TITLE_ACTIONS_MAX_WIDTH: f32 = 960.0;
const SCENARIO_DECK_MAX_WIDTH: f32 = 1_160.0;
const SURFACE_GAP: f32 = 12.0;

#[derive(Component)]
struct TitleSurface;

#[derive(Component)]
struct TitleActions;

#[derive(Component)]
struct TitleActionDetail;

#[derive(Component)]
struct TitleControl(TitleIntent);

#[derive(Component)]
struct ScenarioSurface;

#[derive(Component)]
struct ScenarioCatalogViewport;

#[derive(Component)]
struct ScenarioIntroduction;

#[derive(Component)]
struct ScenarioDeck;

#[derive(Component)]
struct ScenarioColumn;

#[derive(Component)]
struct ScenarioColumnHeading;

#[derive(Component)]
struct ScenarioControl(ScenarioBrowserIntent);

pub(super) fn plugin(app: &mut App) {
    app.add_systems(OnEnter(Screen::Title), spawn_title)
        .add_systems(
            Update,
            (refresh_title, apply_title_layout)
                .chain()
                .in_set(UiSystems::Render)
                .run_if(in_state(Screen::Title)),
        )
        .add_systems(
            Update,
            emit_title_intents
                .in_set(UiSystems::EmitIntents)
                .run_if(in_state(Screen::Title)),
        )
        .add_systems(OnExit(Screen::Title), despawn_screen(Screen::Title))
        .add_systems(OnEnter(Screen::Scenarios), spawn_scenarios)
        .add_systems(
            Update,
            (refresh_scenarios, apply_scenario_layout)
                .chain()
                .in_set(UiSystems::Render)
                .run_if(in_state(Screen::Scenarios)),
        )
        .add_systems(
            Update,
            emit_scenario_intents
                .in_set(UiSystems::EmitIntents)
                .run_if(in_state(Screen::Scenarios)),
        )
        .add_systems(OnExit(Screen::Scenarios), despawn_screen(Screen::Scenarios));
}

fn spawn_title(
    mut commands: Commands,
    assets: Res<UiAssets>,
    view: Res<TitleView>,
    resume: Res<ResumeView>,
) {
    commands
        .spawn(screen_root(Screen::Title, "Title Screen"))
        .insert(TitleSurface)
        .with_children(|root| render_title(root, &assets, &view, &resume));
}

fn refresh_title(
    view: Res<TitleView>,
    resume: Res<ResumeView>,
    assets: Res<UiAssets>,
    roots: Query<Entity, With<TitleSurface>>,
    mut commands: Commands,
) {
    if !view.is_changed() && !resume.is_changed() {
        return;
    }
    for root in &roots {
        commands.entity(root).despawn_related::<Children>();
        commands
            .entity(root)
            .with_children(|root| render_title(root, &assets, &view, &resume));
    }
}

fn render_title(
    root: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    view: &TitleView,
    resume: &ResumeView,
) {
    root.spawn(display(assets, "Hex Game"));
    if let Some(reason) = &view.setup_failure {
        root.spawn((
            Name::new("Gameplay Setup Failure"),
            Text::new(reason.clone()),
            supporting_text_role(),
            TextFont {
                font: assets.body.clone().into(),
                ..TextFont::from_font_size(BLURB_SIZE)
            },
            TextColor(DANGER),
            Node {
                max_width: Val::Px(900.0),
                ..default()
            },
        ));
    }
    root.spawn((Name::new("Primary Routes"), TitleActions, panel()))
        .insert(Node {
            width: Val::Percent(94.0),
            max_width: Val::Px(TITLE_ACTIONS_MAX_WIDTH),
            display: Display::Grid,
            grid_template_columns: RepeatedGridTrack::flex(2, 1.0),
            column_gap: Val::Px(SURFACE_GAP),
            row_gap: Val::Px(SURFACE_GAP),
            flex_shrink: 0.0,
            padding: UiRect::all(Val::Px(18.0)),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(10.0)),
            ..default()
        })
        .with_children(|actions| {
            for (name, supporting, intent, enabled) in [
                (
                    "Continue",
                    resume.message.as_str(),
                    TitleIntent::Continue,
                    resume.available,
                ),
                (
                    "New Game",
                    "Begin the integrated Party Trial scenario.",
                    TitleIntent::NewGame,
                    true,
                ),
                (
                    "Character Creator",
                    "Build and revise characters and their lattices.",
                    TitleIntent::CharacterCreator,
                    true,
                ),
                (
                    "Spell Creator",
                    "Build and revise spells for character loadouts.",
                    TitleIntent::SpellCreator,
                    true,
                ),
                (
                    "Combat Lab",
                    "Compose a sandbox or launch a deterministic fixture.",
                    TitleIntent::CombatLab,
                    true,
                ),
                (
                    "Map Scenarios",
                    "Browse map and world presentation scenarios.",
                    TitleIntent::MapScenarios,
                    true,
                ),
                (
                    "Demos",
                    "Browse focused gameplay demonstrations.",
                    TitleIntent::Demos,
                    true,
                ),
                (
                    "Settings",
                    "Display, readable UI scale, presentation, and volume.",
                    TitleIntent::Settings,
                    true,
                ),
                ("Quit", "Exit the pre-alpha build.", TitleIntent::Quit, true),
            ] {
                spawn_title_action(actions, assets, name, supporting, intent, enabled);
            }
        });
    root.spawn(fine(assets, concat!("v", env!("CARGO_PKG_VERSION"))));
}

fn spawn_title_action(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    name: &str,
    supporting: &str,
    intent: TitleIntent,
    enabled: bool,
) {
    let mut action = parent.spawn((
        button(name.to_owned()),
        TitleControl(intent),
        crate::UiVisibilityRequirement::Immediate,
    ));
    action.insert(Node {
        width: Val::Percent(100.0),
        min_height: Val::Px(58.0),
        flex_shrink: 0.0,
        padding: UiRect::axes(Val::Px(14.0), Val::Px(9.0)),
        flex_direction: FlexDirection::Column,
        align_items: AlignItems::FlexStart,
        row_gap: Val::Px(2.0),
        ..default()
    });
    action.with_children(|button| {
        button.spawn(label(assets, name.to_owned()));
        button.spawn((TitleActionDetail, blurb(assets, supporting.to_owned())));
    });
    if !enabled {
        action.insert(InteractionDisabled);
    }
}

fn apply_title_layout(
    metrics: Res<ResolvedUiMetrics>,
    added: Query<(), Added<TitleActions>>,
    mut actions: Query<&mut Node, With<TitleActions>>,
    mut details: Query<&mut Node, (With<TitleActionDetail>, Without<TitleActions>)>,
) {
    if !metrics.is_changed() && added.is_empty() {
        return;
    }
    let compact = metrics.viewport == UiViewportClass::Compact;
    let primary_labels_only = compact || metrics.content_scale >= 1.75;
    let single_column = compact && metrics.logical_size.x < 720.0;
    for mut node in &mut actions {
        node.grid_template_columns = if single_column {
            RepeatedGridTrack::flex(1, 1.0)
        } else {
            RepeatedGridTrack::flex(2, 1.0)
        };
        let gap = Val::Px(if compact { 6.0 } else { SURFACE_GAP });
        node.column_gap = gap;
        node.row_gap = gap;
        node.padding = UiRect::all(Val::Px(if compact { 10.0 } else { 18.0 }));
    }
    for mut node in &mut details {
        node.display = if primary_labels_only {
            Display::None
        } else {
            Display::Flex
        };
    }
}

fn spawn_scenarios(mut commands: Commands, assets: Res<UiAssets>, view: Res<ScenarioBrowserView>) {
    commands
        .spawn((
            screen_root(
                Screen::Scenarios,
                match view.kind {
                    crate::ScenarioBrowserKind::MapScenarios => "Map Scenarios Screen",
                    crate::ScenarioBrowserKind::Demos => "Demos Screen",
                },
            ),
            ScenarioSurface,
        ))
        .insert(Node {
            // Put the first display line inside a real content inset. A margin
            // on the text node alone does not protect Cinzel's ascender
            // overhang from the render-target edge on Compact Retina canvases.
            padding: UiRect {
                left: Val::Px(14.0),
                right: Val::Px(14.0),
                top: Val::Px(72.0),
                bottom: Val::Px(14.0),
            },
            justify_content: JustifyContent::FlexStart,
            overflow: Overflow::clip_y(),
            ..crate::screen_root_node()
        })
        .with_children(|root| render_scenarios(root, &assets, &view));
}

fn refresh_scenarios(
    view: Res<ScenarioBrowserView>,
    assets: Res<UiAssets>,
    roots: Query<Entity, With<ScenarioSurface>>,
    mut commands: Commands,
) {
    if !view.is_changed() {
        return;
    }
    for root in &roots {
        commands.entity(root).despawn_related::<Children>();
        commands
            .entity(root)
            .with_children(|root| render_scenarios(root, &assets, &view));
    }
}

fn render_scenarios(
    root: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    view: &ScenarioBrowserView,
) {
    root.spawn((
        Name::new("Scenario Screen Title"),
        display(assets, view.kind.title()),
        Node {
            flex_shrink: 0.0,
            ..default()
        },
        crate::UiVisibilityRequirement::Immediate,
    ));
    root.spawn((
        Name::new("Scenario Screen Introduction"),
        ScenarioIntroduction,
        blurb(assets, match view.kind {
            crate::ScenarioBrowserKind::MapScenarios => {
                "Map and world presentation scenarios. New Game remains the canonical game route."
            }
            crate::ScenarioBrowserKind::Demos => {
                "Focused gameplay demonstrations. New Game remains the canonical game route."
            }
        }),
        Node {
            flex_shrink: 0.0,
            ..default()
        },
        crate::UiVisibilityRequirement::Immediate,
    ));
    root.spawn((
        Name::new("Scenario Catalog Frame"),
        Node {
            width: Val::Percent(100.0),
            min_height: Val::Px(0.0),
            flex_grow: 1.0,
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(8.0),
            ..default()
        },
    ))
    .with_children(|frame| {
        let viewport = frame
            .spawn((
                Name::new("Scenario Catalog Viewport"),
                ScenarioCatalogViewport,
                ScrollArea,
                ScrollPosition::default(),
                Node {
                    min_width: Val::Px(0.0),
                    min_height: Val::Px(0.0),
                    flex_grow: 1.0,
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::FlexStart,
                    overflow: Overflow::scroll_y(),
                    ..default()
                },
            ))
            .with_children(|viewport| {
                viewport
                    .spawn((
                        Name::new("Scenario Catalog"),
                        ScenarioDeck,
                        Node {
                            width: Val::Percent(96.0),
                            max_width: Val::Px(SCENARIO_DECK_MAX_WIDTH),
                            flex_shrink: 0.0,
                            align_self: AlignSelf::Center,
                            display: Display::Grid,
                            grid_template_columns: RepeatedGridTrack::flex(2, 1.0),
                            column_gap: Val::Px(SURFACE_GAP),
                            row_gap: Val::Px(SURFACE_GAP),
                            align_items: AlignItems::Start,
                            ..default()
                        },
                    ))
                    .with_children(|deck| {
                        spawn_scenario_column(
                            deck,
                            assets,
                            view.kind.title(),
                            view.scenarios.iter(),
                        );
                    });
            })
            .id();
        if view.kind == crate::ScenarioBrowserKind::MapScenarios {
            frame
                .spawn((
                    Name::new("Scenario Catalog Scrollbar"),
                    Scrollbar::new(viewport, ControlOrientation::Vertical, 36.0),
                    Node {
                        width: Val::Px(18.0),
                        height: Val::Percent(100.0),
                        flex_shrink: 0.0,
                        border: UiRect::all(Val::Px(1.0)),
                        border_radius: BorderRadius::all(Val::Px(9.0)),
                        ..default()
                    },
                    BorderColor::all(ACCENT_EDGE),
                    BackgroundColor(Color::srgba(0.02, 0.03, 0.045, 0.96)),
                ))
                .with_child((
                    Name::new("Scenario Catalog Scrollbar Thumb"),
                    ScrollbarThumb {
                        border_radius: BorderRadius::all(Val::Px(8.0)),
                        border: UiRect::all(Val::Px(1.0)),
                    },
                    BorderColor::all(ACCENT_EDGE),
                    BackgroundColor(crate::ACCENT),
                ));
        }
    });
    root.spawn((
        button("Back"),
        ScenarioControl(ScenarioBrowserIntent::Back),
        crate::UiVisibilityRequirement::Immediate,
    ))
    .with_child(label(assets, "Back to title"));
}

fn spawn_scenario_column<'a>(
    deck: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    title: &'static str,
    entries: impl Iterator<Item = &'a TitleScenarioView>,
) {
    deck.spawn((
        Name::new(format!("{title} Scenario Column")),
        ScenarioColumn,
        panel(),
    ))
    .insert(Node {
        min_width: Val::Px(0.0),
        display: Display::Grid,
        grid_template_columns: RepeatedGridTrack::flex(2, 1.0),
        column_gap: Val::Px(10.0),
        row_gap: Val::Px(10.0),
        padding: UiRect::all(Val::Px(14.0)),
        border: UiRect::all(Val::Px(1.0)),
        border_radius: BorderRadius::all(Val::Px(10.0)),
        ..default()
    })
    .with_children(|column| {
        column.spawn((
            ScenarioColumnHeading,
            heading(assets, title),
            Node {
                grid_column: GridPlacement::span(2),
                ..default()
            },
        ));
        for entry in entries {
            spawn_scenario_card(column, assets, entry);
        }
    });
}

fn spawn_scenario_card(
    list: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    entry: &TitleScenarioView,
) {
    list.spawn((
        Name::new(format!("Scenario Entry: {}", entry.scenario.name)),
        Node {
            width: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::FlexStart,
            row_gap: Val::Px(6.0),
            ..default()
        },
    ))
    .with_children(|row| {
        row.spawn((
            button(entry.scenario.name.clone()),
            ScenarioControl(ScenarioBrowserIntent::Start(entry.scenario.clone())),
            crate::UiVisibilityRequirement::Scrollable,
        ))
        .insert(BorderColor::all(ACCENT_EDGE))
        .insert(Node {
            width: Val::Percent(100.0),
            min_width: Val::Px(0.0),
            min_height: Val::Px(58.0),
            padding: UiRect::axes(Val::Px(14.0), Val::Px(8.0)),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::FlexStart,
            row_gap: Val::Px(3.0),
            ..default()
        })
        .with_children(|button| {
            button.spawn(label(assets, entry.scenario.name.clone()));
            button.spawn(blurb(assets, entry.scenario.blurb.clone()));
        });
        if let Some(seed) = entry.resolved_seed {
            row.spawn((
                stacked_row_button(format!("Reroll {}", entry.scenario.name), 220.0),
                ScenarioControl(ScenarioBrowserIntent::Reroll(entry.scenario.clone())),
                crate::UiVisibilityRequirement::Scrollable,
            ))
            .with_children(|control| {
                control.spawn(blurb(assets, "reroll"));
                control.spawn(fine(assets, format!("seed {seed}")));
            });
        }
    });
}

fn apply_scenario_layout(
    metrics: Res<ResolvedUiMetrics>,
    added: Query<(), Added<ScenarioDeck>>,
    mut decks: Query<
        &mut Node,
        (
            With<ScenarioDeck>,
            Without<ScenarioColumn>,
            Without<ScenarioColumnHeading>,
            Without<ScenarioIntroduction>,
        ),
    >,
    mut columns: Query<
        &mut Node,
        (
            With<ScenarioColumn>,
            Without<ScenarioDeck>,
            Without<ScenarioColumnHeading>,
            Without<ScenarioIntroduction>,
        ),
    >,
    mut headings: Query<
        &mut Node,
        (
            With<ScenarioColumnHeading>,
            Without<ScenarioDeck>,
            Without<ScenarioColumn>,
            Without<ScenarioIntroduction>,
        ),
    >,
    mut introductions: Query<
        &mut Node,
        (
            With<ScenarioIntroduction>,
            Without<ScenarioDeck>,
            Without<ScenarioColumn>,
            Without<ScenarioColumnHeading>,
        ),
    >,
) {
    if !metrics.is_changed() && added.is_empty() {
        return;
    }
    let compact = metrics.viewport == UiViewportClass::Compact;
    for mut node in &mut introductions {
        node.display = if compact && metrics.content_scale >= 1.5 {
            Display::None
        } else {
            Display::Flex
        };
    }
    for mut node in &mut decks {
        node.grid_template_columns = RepeatedGridTrack::flex(1, 1.0);
    }
    for mut node in &mut columns {
        node.grid_template_columns = RepeatedGridTrack::flex(if compact { 1 } else { 2 }, 1.0);
    }
    for mut node in &mut headings {
        node.grid_column = GridPlacement::span(if compact { 1 } else { 2 });
    }
}

fn emit_title_intents(
    controls: Query<(&Interaction, &TitleControl), Changed<Interaction>>,
    mut intents: MessageWriter<UiIntent>,
) {
    for (interaction, control) in &controls {
        if *interaction == Interaction::Pressed {
            intents.write(UiIntent::Title(control.0.clone()));
        }
    }
}

fn emit_scenario_intents(
    controls: Query<(&Interaction, &ScenarioControl), Changed<Interaction>>,
    mut intents: MessageWriter<UiIntent>,
) {
    for (interaction, control) in &controls {
        if *interaction == Interaction::Pressed {
            intents.write(UiIntent::Scenarios(control.0.clone()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_exposes_every_primary_route_without_scenario_cards() {
        let intents = [
            TitleIntent::Continue,
            TitleIntent::NewGame,
            TitleIntent::CharacterCreator,
            TitleIntent::SpellCreator,
            TitleIntent::CombatLab,
            TitleIntent::MapScenarios,
            TitleIntent::Demos,
            TitleIntent::Settings,
            TitleIntent::Quit,
        ];
        assert_eq!(intents.len(), 9);
    }
}
