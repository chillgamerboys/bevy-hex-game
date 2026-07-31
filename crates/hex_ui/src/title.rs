//! Title-screen rendering. Application code supplies immutable scenario snapshots and
//! handles the typed intents emitted here.

use bevy::prelude::*;
use bevy::ui::InteractionDisabled;
use bevy::ui_widgets::ScrollArea;
use hex_assets::ScenarioCategory;
use hex_core::Screen;

use crate::{
    blurb, button, despawn_screen, display, fine, heading, label, panel, screen_root,
    stacked_row_button, ResolvedUiMetrics, ResumeView, TitleIntent, TitleScenarioView, TitleView,
    UiAssets, UiIntent, UiSystems, UiViewportClass, ACCENT_EDGE, BLURB_SIZE, DANGER,
};

const CATEGORY_DECK_MAX_WIDTH: f32 = 1_500.0;
const CATEGORY_GAP: f32 = 16.0;

#[derive(Component)]
struct TitleSurface;

#[derive(Component)]
struct CategoryDeck;

#[derive(Component)]
struct CategoryColumn;

#[derive(Component)]
struct TitleControl(TitleIntent);

pub(super) fn plugin(app: &mut App) {
    app.add_systems(OnEnter(Screen::Title), spawn_title)
        .add_systems(
            Update,
            (refresh_title, apply_title_layout)
                .chain()
                .run_if(in_state(Screen::Title)),
        )
        .add_systems(
            Update,
            emit_title_intents
                .in_set(UiSystems::EmitIntents)
                .run_if(in_state(Screen::Title)),
        )
        .add_systems(OnExit(Screen::Title), despawn_screen(Screen::Title));
}

fn spawn_title(
    mut commands: Commands,
    assets: Res<UiAssets>,
    view: Res<TitleView>,
    resume: Res<ResumeView>,
) {
    commands
        .spawn((screen_root(Screen::Title, "Title Screen"), TitleSurface))
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
            TextFont {
                font: assets.body.clone().into(),
                ..TextFont::from_font_size(BLURB_SIZE)
            },
            TextColor(DANGER),
            Node {
                max_width: Val::Px(1_100.0),
                ..default()
            },
        ));
    }
    root.spawn((
        Name::new("Scenario Category Deck"),
        CategoryDeck,
        Node {
            width: Val::Percent(96.0),
            max_width: Val::Px(CATEGORY_DECK_MAX_WIDTH),
            min_height: Val::Px(0.0),
            flex_basis: Val::Px(0.0),
            flex_grow: 1.0,
            flex_shrink: 1.0,
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(CATEGORY_GAP),
            align_items: AlignItems::Stretch,
            ..default()
        },
    ))
    .with_children(|deck| {
        spawn_category_column(
            deck,
            assets,
            "maps",
            view.scenarios
                .iter()
                .filter(|entry| entry.scenario.category == ScenarioCategory::Map),
            false,
        );
        spawn_category_column(
            deck,
            assets,
            "demos",
            view.scenarios
                .iter()
                .filter(|entry| entry.scenario.category == ScenarioCategory::Demo),
            true,
        );
        spawn_action_column(deck, assets, resume);
    });
    root.spawn((
        Node {
            margin: UiRect::bottom(Val::Px(4.0)),
            ..default()
        },
        children![blurb(
            assets,
            "New Game starts Party Trial   ·   development fixtures stay available",
        )],
    ));
    root.spawn((
        Name::new("Version"),
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(12.0),
            bottom: Val::Px(8.0),
            ..default()
        },
        Pickable::IGNORE,
        children![fine(assets, concat!("v", env!("CARGO_PKG_VERSION")))],
    ));
}

fn spawn_action_column(deck: &mut ChildSpawnerCommands, assets: &UiAssets, resume: &ResumeView) {
    deck.spawn((Name::new("Actions Column"), CategoryColumn, panel()))
        .insert(category_column_node())
        .with_children(|column| {
            column.spawn(heading(assets, "actions"));
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
                    "Settings",
                    "Display, readable UI scale, presentation, and volume.",
                    TitleIntent::Settings,
                    true,
                ),
                ("Quit", "Exit the pre-alpha build.", TitleIntent::Quit, true),
            ] {
                spawn_card_button(column, assets, name, supporting, intent, enabled);
            }
        });
}

fn spawn_category_column<'a>(
    deck: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    title: &'static str,
    entries: impl Iterator<Item = &'a TitleScenarioView>,
    include_tools: bool,
) {
    deck.spawn((
        Name::new(format!("{title} Scenario Column")),
        CategoryColumn,
        panel(),
    ))
    .insert(category_column_node())
    .with_children(|column| {
        column.spawn(heading(assets, title));
        column
            .spawn((
                Name::new(format!("{title} Scenario List")),
                ScrollArea,
                Node {
                    width: Val::Percent(100.0),
                    min_height: Val::Px(0.0),
                    flex_basis: Val::Px(0.0),
                    flex_grow: 1.0,
                    flex_shrink: 1.0,
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(8.0),
                    align_items: AlignItems::Stretch,
                    overflow: Overflow::scroll_y(),
                    ..default()
                },
            ))
            .with_children(|list| {
                if include_tools {
                    spawn_tool_cards(list, assets);
                }
                for entry in entries {
                    spawn_scenario_card(list, assets, entry);
                }
            });
    });
}

fn category_column_node() -> Node {
    Node {
        min_width: Val::Px(0.0),
        height: Val::Percent(100.0),
        flex_basis: Val::Px(0.0),
        flex_grow: 1.0,
        flex_shrink: 1.0,
        flex_direction: FlexDirection::Column,
        row_gap: Val::Px(10.0),
        padding: UiRect::all(Val::Px(14.0)),
        border: UiRect::all(Val::Px(1.0)),
        border_radius: BorderRadius::all(Val::Px(10.0)),
        ..default()
    }
}

fn card_node() -> Node {
    Node {
        width: Val::Percent(100.0),
        padding: UiRect::axes(Val::Px(14.0), Val::Px(11.0)),
        flex_direction: FlexDirection::Column,
        row_gap: Val::Px(4.0),
        border: UiRect::all(Val::Px(1.0)),
        border_radius: BorderRadius::all(Val::Px(6.0)),
        ..default()
    }
}

fn spawn_tool_cards(list: &mut ChildSpawnerCommands, assets: &UiAssets) {
    list.spawn((
        Name::new("Creator and Lab Entries"),
        Node {
            width: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(8.0),
            ..default()
        },
    ))
    .with_children(|tools| {
        for (name, supporting, intent) in [
            (
                "Character Creator",
                "Build, save, revise, duplicate, and test character lattices.",
                TitleIntent::CharacterCreator,
            ),
            (
                "Spell Creator",
                "Compose, validate, save, revise, and duplicate combat spells.",
                TitleIntent::SpellCreator,
            ),
            (
                "Combat Lab",
                "Compose a sandbox or launch a deterministic combat fixture.",
                TitleIntent::CombatLab,
            ),
        ] {
            spawn_card_button(tools, assets, name, supporting, intent, true);
        }
    });
}

fn spawn_card_button(
    parent: &mut ChildSpawnerCommands,
    assets: &UiAssets,
    name: &str,
    supporting: &str,
    intent: TitleIntent,
    enabled: bool,
) {
    let mut entity = parent.spawn((button(name.to_owned()), TitleControl(intent)));
    entity
        .insert(card_node())
        .insert(BorderColor::all(ACCENT_EDGE))
        .with_children(|button| {
            button.spawn(label(assets, name.to_owned()));
            button.spawn((
                blurb(assets, supporting.to_owned()),
                Node {
                    width: Val::Percent(100.0),
                    ..default()
                },
            ));
        });
    if !enabled {
        entity.insert(InteractionDisabled);
    }
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
            row_gap: Val::Px(6.0),
            align_items: AlignItems::Stretch,
            ..default()
        },
    ))
    .with_children(|row| {
        spawn_card_button(
            row,
            assets,
            &entry.scenario.name,
            &entry.scenario.blurb,
            TitleIntent::StartScenario(entry.scenario.clone()),
            true,
        );
        if let Some(seed) = entry.resolved_seed {
            row.spawn((
                stacked_row_button(format!("Reroll {}", entry.scenario.name), 148.0),
                TitleControl(TitleIntent::RerollScenario(entry.scenario.clone())),
            ))
            .with_children(|control| {
                control.spawn(blurb(assets, "reroll"));
                control.spawn(fine(assets, format!("seed {seed}")));
            });
        }
    });
}

fn apply_title_layout(
    metrics: Res<ResolvedUiMetrics>,
    added_decks: Query<(), Added<CategoryDeck>>,
    added_columns: Query<(), Added<CategoryColumn>>,
    mut decks: Query<&mut Node, With<CategoryDeck>>,
    mut columns: Query<&mut Node, (With<CategoryColumn>, Without<CategoryDeck>)>,
) {
    if !metrics.is_changed() && added_decks.is_empty() && added_columns.is_empty() {
        return;
    }
    let compact = metrics.viewport == UiViewportClass::Compact;
    for mut node in &mut decks {
        node.flex_direction = if compact {
            FlexDirection::Column
        } else {
            FlexDirection::Row
        };
        node.overflow = if compact {
            Overflow::scroll_y()
        } else {
            Overflow::default()
        };
    }
    for mut node in &mut columns {
        node.width = if compact {
            Val::Percent(100.0)
        } else {
            Val::Auto
        };
        node.height = if compact {
            Val::Auto
        } else {
            Val::Percent(100.0)
        };
        node.flex_basis = if compact { Val::Auto } else { Val::Px(0.0) };
        node.flex_grow = if compact { 0.0 } else { 1.0 };
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

#[cfg(test)]
mod tests {
    use bevy::{state::app::StatesPlugin, MinimalPlugins};
    use hex_assets::Scenario;

    use super::*;

    fn scenario(name: &str, category: ScenarioCategory) -> Scenario {
        Scenario {
            name: name.to_owned(),
            category,
            blurb: "A focused scenario.".to_owned(),
            world: "config/world.ron".to_owned(),
            lighting: "config/lighting.ron".to_owned(),
            generation_seed: None,
            starting_time_hours: None,
            encounter: "config/encounters/open-ground.ron".to_owned(),
        }
    }

    #[test]
    fn title_renderer_groups_cards_and_keeps_required_routes_named() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin));
        app.init_state::<Screen>()
            .insert_resource(UiAssets {
                display: Handle::default(),
                body: Handle::default(),
                hex_cell: Handle::default(),
            })
            .init_resource::<ResolvedUiMetrics>()
            .insert_resource(TitleView {
                scenarios: vec![TitleScenarioView {
                    scenario: scenario("Map One", ScenarioCategory::Map),
                    resolved_seed: None,
                }],
                setup_failure: None,
            })
            .init_resource::<ResumeView>()
            .add_message::<UiIntent>();
        plugin(&mut app);
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Title);
        app.update();
        app.update();

        let names = app
            .world_mut()
            .query::<&Name>()
            .iter(app.world())
            .map(|name| name.as_str().to_owned())
            .collect::<Vec<_>>();
        for expected in [
            "Map One",
            "Character Creator",
            "Spell Creator",
            "Combat Lab",
            "Continue",
            "New Game",
            "Settings",
        ] {
            assert!(
                names.iter().any(|name| name == expected),
                "missing {expected}"
            );
        }
    }
}
