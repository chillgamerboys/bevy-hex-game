//! Main menu: start or continue the default game, or launch a development fixture.
//!
//! `assets/config/scenarios.ron` names one default game and categorizes every
//! development scenario. The default is launched through New Game and never appears
//! beside focused fixtures.
//!
//! # The list is rebuilt, not spawned once
//!
//! `Screen::Title` is reached on a **wall-clock timer** from the splash screen, not on
//! a load gate, so it is genuinely reachable before `scenarios.ron` has finished
//! parsing. A system that spawned the buttons on entry would show an empty menu on a
//! cold disk and never correct itself. Rebuilding when the library appears or changes
//! covers that, hot reload, and nothing else needing to know the difference.
//!
//! For the same reason the library is taken as `Option<Res<_>>` everywhere here. A
//! plain `Res<T>` on a resource that does not exist yet is a panic, and this project
//! has already shipped that crash once on this very screen.

use std::time::{SystemTime, UNIX_EPOCH};

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::ui_widgets::ScrollArea;
use hex_assets::{Scenario, ScenarioCategory, ScenarioLibrary};
use hex_core::{GameplaySetupFailure, InputAction, InputBindings, ResolvedMapSeed, Screen};

use crate::menus::widgets::{
    blurb, button, display, fine, heading, label, small_button, UiAssets, ACCENT_EDGE, BLURB_SIZE,
    DANGER,
};
use crate::scenarios::ScenarioToLoad;

use super::creator::CreatorEntryRequest;
use super::{despawn_screen, screen_root};

/// Stops the three columns becoming unreadably wide on an ultrawide display.
const CATEGORY_DECK_MAX_WIDTH: f32 = 1_500.0;
/// The horizontal breathing room between framed category lanes.
const CATEGORY_GAP: f32 = 16.0;

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<InputBindings>();
    app.init_resource::<SessionSeeds>();
    app.add_systems(OnEnter(Screen::Title), spawn_title);
    app.add_systems(
        Update,
        (
            rebuild_scenario_list,
            reroll_scenario_seed,
            start_chosen_scenario,
            start_new_game,
            open_character_creator,
            open_spell_creator,
            open_combat_lab,
            open_settings,
            quit_game,
            handle_input,
        )
            .chain()
            .run_if(in_state(Screen::Title)),
    );
    app.add_systems(OnExit(Screen::Title), despawn_screen(Screen::Title));
}

/// The node the three framed category lanes hang off.
#[derive(Component)]
struct CategoryDeck;

/// One framed title-screen lane.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
struct ScenarioColumn(ScenarioCategory);

/// The stable application-action lane beside development scenarios.
#[derive(Component)]
struct ActionColumn;

/// The independently scrollable list inside a category lane.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
struct ScenarioList(ScenarioCategory);

/// One row in the list, including its optional seed control.
#[derive(Component)]
struct ScenarioEntry;

/// A button carrying the exact scenario it will start.
#[derive(Component)]
struct StartsScenario {
    scenario: Scenario,
}

/// A secondary button that gives a generated scenario a new session seed.
#[derive(Component)]
struct RerollsScenario {
    scenario: Scenario,
}

/// The button that opens saved character authoring.
#[derive(Component)]
struct OpensCharacterCreator;

/// The button that opens saved spell authoring.
#[derive(Component)]
struct OpensSpellCreator;

/// The button that opens sandbox and deterministic combat fixtures.
#[derive(Component)]
struct OpensCombatLab;

/// Starts the library's independently resolved default scenario.
#[derive(Component)]
struct StartsNewGame;

/// Opens the pre-alpha settings surface.
#[derive(Component)]
struct OpensSettings;

/// Exits the application from the Actions lane.
#[derive(Component)]
struct QuitsGame;

/// Stable Continue hook, activated by the resume scaffold.
#[derive(Component)]
pub(crate) struct ContinuesGame;

/// Supporting text updated by the resume scaffold.
#[derive(Component)]
pub(crate) struct ContinueStatusText;

/// The static mechanics demo card, which is not reconciled from scenario content.
#[derive(Component)]
struct StaticDemoEntry;

/// The line that stands in for the list until the library has loaded.
#[derive(Component)]
struct ListPlaceholder;

/// Player-visible reason the previous scenario could not finish setup.
#[derive(Component)]
struct SetupFailureNotice;

/// Seed overrides that live only for this process.
///
/// Keys include both display name and world path so a hot reload cannot accidentally
/// transfer a reroll to a different scenario that reused one of them.
#[derive(Resource)]
struct SessionSeeds {
    overrides: HashMap<String, u64>,
    entropy: u64,
}

impl Default for SessionSeeds {
    fn default() -> Self {
        let entropy = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0xA076_1D64_78BD_642F, |elapsed| {
                elapsed.as_secs() ^ u64::from(elapsed.subsec_nanos()).rotate_left(32)
            });
        Self {
            overrides: HashMap::default(),
            entropy,
        }
    }
}

impl SessionSeeds {
    fn resolved(&self, scenario: &Scenario) -> Option<u64> {
        scenario.generation_seed.map(|configured| {
            self.overrides
                .get(&scenario_seed_key(scenario))
                .copied()
                .unwrap_or(configured)
        })
    }

    fn reroll(&mut self, scenario: &Scenario) -> Option<u64> {
        let current = self.resolved(scenario)?;
        self.entropy = mixed_seed(
            self.entropy
                .wrapping_add(0x9E37_79B9_7F4A_7C15)
                .wrapping_add(current),
        );
        let candidate = if self.entropy == current {
            current.wrapping_add(1)
        } else {
            self.entropy
        };
        self.overrides
            .insert(scenario_seed_key(scenario), candidate);
        Some(candidate)
    }
}

fn scenario_seed_key(scenario: &Scenario) -> String {
    format!("{}\0{}", scenario.name, scenario.world)
}

fn mixed_seed(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
}

fn spawn_title(
    mut commands: Commands,
    failure: Option<Res<GameplaySetupFailure>>,
    assets: Res<UiAssets>,
) {
    let failure_reason = failure.as_deref().map(|failure| failure.reason.clone());
    commands
        .spawn(screen_root(Screen::Title, "Title Screen"))
        .with_children(|parent| {
            parent.spawn(display(&assets, "Hex Game"));
            if let Some(reason) = failure_reason {
                parent.spawn((
                    Name::new("Gameplay Setup Failure"),
                    SetupFailureNotice,
                    Text::new(reason),
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
            parent
                .spawn((
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
                    for category in [ScenarioCategory::Map, ScenarioCategory::Demo] {
                        spawn_category_column(deck, category, &assets);
                    }
                    spawn_action_column(deck, &assets);
                });
            parent.spawn((
                Node {
                    margin: UiRect::bottom(Val::Px(4.0)),
                    ..default()
                },
                children![blurb(
                    &assets,
                    "New Game starts Party Trial   ·   development fixtures stay available",
                )],
            ));
            // The version a bug report needs, where a screenshot catches it.
            // Absolute so it rides the corner instead of the menu column.
            parent.spawn((
                Name::new("Version"),
                Node {
                    position_type: PositionType::Absolute,
                    right: Val::Px(12.0),
                    bottom: Val::Px(8.0),
                    ..default()
                },
                Pickable::IGNORE,
                children![fine(&assets, concat!("v", env!("CARGO_PKG_VERSION")))],
            ));
        });
}

fn category_label(category: ScenarioCategory) -> &'static str {
    match category {
        ScenarioCategory::Map => "maps",
        ScenarioCategory::Demo => "demos",
    }
}

fn spawn_action_column(deck: &mut ChildSpawnerCommands, assets: &UiAssets) {
    deck.spawn((
        Name::new("Actions Column"),
        ActionColumn,
        crate::menus::widgets::panel(),
    ))
    .insert(Node {
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
    })
    .with_children(|column| {
        column.spawn(heading(assets, "actions"));
        for (name, blurb_text, action) in [
            (
                "Continue",
                "Resume the last explicitly saved exploration state.",
                0_u8,
            ),
            (
                "New Game",
                "Begin the integrated Party Trial scenario.",
                1_u8,
            ),
            (
                "Settings",
                "Display, presentation, and volume scaffolding.",
                2_u8,
            ),
            ("Quit", "Exit the pre-alpha build.", 3_u8),
        ] {
            let mut entity = column.spawn(button(name));
            entity
                .insert(scenario_card_node())
                .insert(BorderColor::all(ACCENT_EDGE))
                .with_children(|button| {
                    button.spawn(label(assets, name));
                    let mut supporting = button.spawn((
                        blurb(assets, blurb_text),
                        Node {
                            width: Val::Percent(100.0),
                            ..default()
                        },
                    ));
                    if action == 0 {
                        supporting.insert(ContinueStatusText);
                    }
                });
            match action {
                0 => {
                    entity.insert(ContinuesGame);
                }
                1 => {
                    entity.insert(StartsNewGame);
                }
                2 => {
                    entity.insert(OpensSettings);
                }
                _ => {
                    entity.insert(QuitsGame);
                }
            }
        }
    });
}

fn spawn_category_column(
    deck: &mut ChildSpawnerCommands,
    category: ScenarioCategory,
    assets: &UiAssets,
) {
    deck.spawn((
        Name::new(format!("{} Scenario Column", category_label(category))),
        ScenarioColumn(category),
        crate::menus::widgets::panel(),
    ))
    .insert(Node {
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
    })
    .with_children(|column| {
        column.spawn(heading(assets, category_label(category)));
        column
            .spawn((
                Name::new(format!("{} Scenario List", category_label(category))),
                ScenarioList(category),
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
                list.spawn((ListPlaceholder, blurb(assets, "loading scenarios...")));
                if category == ScenarioCategory::Demo {
                    spawn_wave_six_demo_cards(list, assets);
                }
            });
    });
}

fn scenario_card_node() -> Node {
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

fn spawn_wave_six_demo_cards(list: &mut ChildSpawnerCommands, assets: &UiAssets) {
    list.spawn((
        Name::new("Static Wave 6 Demo Entries"),
        StaticDemoEntry,
        Node {
            width: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(8.0),
            ..default()
        },
    ))
    .with_children(|entry| {
        entry
            .spawn((button("Character Creator"), OpensCharacterCreator))
            .insert(scenario_card_node())
            .insert(BorderColor::all(ACCENT_EDGE))
            .with_children(|button| {
                button.spawn(label(assets, "Character Creator"));
                button.spawn((
                    blurb(
                        assets,
                        "Build, save, revise, duplicate, and test character lattices.",
                    ),
                    Node {
                        width: Val::Percent(100.0),
                        ..default()
                    },
                ));
            });
        entry
            .spawn((button("Spell Creator"), OpensSpellCreator))
            .insert(scenario_card_node())
            .insert(BorderColor::all(ACCENT_EDGE))
            .with_children(|button| {
                button.spawn(label(assets, "Spell Creator"));
                button.spawn((
                    blurb(
                        assets,
                        "Compose, validate, save, revise, and duplicate combat spells.",
                    ),
                    Node {
                        width: Val::Percent(100.0),
                        ..default()
                    },
                ));
            });
        entry
            .spawn((button("Combat Lab"), OpensCombatLab))
            .insert(scenario_card_node())
            .insert(BorderColor::all(ACCENT_EDGE))
            .with_children(|button| {
                button.spawn(label(assets, "Combat Lab"));
                button.spawn((
                    blurb(
                        assets,
                        "Compose a sandbox or launch a deterministic combat fixture.",
                    ),
                    Node {
                        width: Val::Percent(100.0),
                        ..default()
                    },
                ));
            });
    });
}

/// Fills both development lists once the library is known, and again if it changes.
fn rebuild_scenario_list(
    mut commands: Commands,
    library: Option<Res<ScenarioLibrary>>,
    lists: Query<(Entity, &ScenarioList)>,
    placeholders: Query<Entity, With<ListPlaceholder>>,
    existing: Query<Entity, With<ScenarioEntry>>,
    clicked_starts: Query<&Interaction, (Changed<Interaction>, With<StartsScenario>)>,
    clicked_rerolls: Query<&Interaction, (Changed<Interaction>, With<RerollsScenario>)>,
    seeds: Res<SessionSeeds>,
    assets: Res<UiAssets>,
) {
    let Some(library) = library else { return };
    if lists.iter().count() != 2 {
        return;
    }

    // A hot reload and a pointer press can land in the same frame. The button carries
    // the exact scenario snapshot, so let its click system consume it before rebuilding
    // the row. Starting leaves this screen anyway; rerolling marks the seed resource
    // changed and causes a rebuild on the next frame.
    let click_in_flight = clicked_starts
        .iter()
        .chain(clicked_rerolls.iter())
        .any(|interaction| *interaction == Interaction::Pressed);
    if click_in_flight {
        return;
    }

    // **Reconciled from what is on screen, not from a change event.** The first version
    // rebuilt only when the library was added or changed, which is true exactly once
    // per run — so coming back from gameplay spawned a fresh, empty list that nothing
    // ever filled, and the menu sat on "loading scenarios…" for the rest of the
    // session. Asking "are the buttons missing?" covers screen re-entry, a library that
    // arrives late, and a hot reload, without needing to tell them apart.
    //
    // The guard still matters: without it this would rebuild every frame and a button
    // would never hold a hover long enough to show one.
    if !library.is_changed() && !seeds.is_changed() && !existing.is_empty() {
        return;
    }

    for stale in existing.iter().chain(placeholders.iter()) {
        commands.entity(stale).despawn();
    }

    for scenario in library.visible_scenarios() {
        // Focused combat demos scale behind Combat Lab's fixture selector in Wave 6;
        // only world-first scenarios remain direct, data-backed title cards.
        if scenario.category == ScenarioCategory::Demo {
            continue;
        }
        let Some(list) = lists
            .iter()
            .find_map(|(entity, list)| (list.0 == scenario.category).then_some(entity))
        else {
            // Closed category vocabulary plus three columns makes this unreachable,
            // but refusing to drop content silently is the important invariant.
            warn!(
                "title screen has no {:?} column for scenario {:?}",
                scenario.category, scenario.name
            );
            continue;
        };

        let row = commands
            .spawn((
                Name::new(format!("Scenario Entry: {}", scenario.name)),
                ScenarioEntry,
                Node {
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(6.0),
                    align_items: AlignItems::Stretch,
                    ..default()
                },
            ))
            .id();

        let seed = seeds.resolved(scenario);
        let launch = commands
            .spawn((
                button("Scenario"),
                StartsScenario {
                    scenario: scenario.clone(),
                },
            ))
            .insert(scenario_card_node())
            .with_children(|entry| {
                entry.spawn(label(&assets, scenario.name.clone()));
                entry.spawn((
                    blurb(&assets, scenario.blurb.clone()),
                    Node {
                        width: Val::Percent(100.0),
                        ..default()
                    },
                ));
            })
            .id();
        commands.entity(row).add_child(launch);

        if let Some(seed) = seed {
            let reroll = commands
                .spawn((
                    small_button("Reroll Seed"),
                    RerollsScenario {
                        scenario: scenario.clone(),
                    },
                ))
                .with_children(|control| {
                    control.spawn(blurb(&assets, "reroll"));
                    control.spawn(fine(&assets, format!("seed {seed}")));
                })
                .id();
            commands.entity(row).add_child(reroll);
        }

        commands.entity(list).add_child(row);
    }
}

/// Replaces only this session's seed; configuration remains untouched.
fn reroll_scenario_seed(
    mut seeds: ResMut<SessionSeeds>,
    clicked: Query<(&Interaction, &RerollsScenario), Changed<Interaction>>,
) {
    for (interaction, rerolls) in &clicked {
        if *interaction == Interaction::Pressed {
            if let Some(seed) = seeds.reroll(&rerolls.scenario) {
                info!(
                    "rerolled scenario seed: {} -> {}",
                    rerolls.scenario.name, seed
                );
            }
        }
    }
}

/// Starts whichever scenario was clicked.
fn start_chosen_scenario(
    mut commands: Commands,
    clicked: Query<(&Interaction, &StartsScenario), Changed<Interaction>>,
    seeds: Res<SessionSeeds>,
    mut next: ResMut<NextState<Screen>>,
) {
    for (interaction, starts) in &clicked {
        if *interaction == Interaction::Pressed {
            commands.remove_resource::<crate::save::PendingResume>();
            commands.insert_resource(ScenarioToLoad {
                scenario: starts.scenario.clone(),
                resolved_seed: seeds.resolved(&starts.scenario).map(ResolvedMapSeed),
                encounter_override: None,
            });
            next.set(Screen::Loading);
        }
    }
}

/// Starts the one integrated scenario independently from the development lanes.
fn start_new_game(
    mut commands: Commands,
    clicked: Query<&Interaction, (Changed<Interaction>, With<StartsNewGame>)>,
    library: Option<Res<ScenarioLibrary>>,
    seeds: Res<SessionSeeds>,
    mut next: ResMut<NextState<Screen>>,
) {
    if !clicked
        .iter()
        .any(|interaction| *interaction == Interaction::Pressed)
    {
        return;
    }
    let Some(library) = library else {
        warn!("New Game was pressed before the scenario library loaded");
        return;
    };
    let Some(scenario) = library.default_scenario() else {
        error!(
            "default_game {:?} does not resolve to a scenario",
            library.default_game
        );
        commands.insert_resource(GameplaySetupFailure::new(format!(
            "The configured default game {:?} does not exist.",
            library.default_game
        )));
        return;
    };
    commands.remove_resource::<crate::save::PendingResume>();
    commands.insert_resource(ScenarioToLoad {
        scenario: scenario.clone(),
        resolved_seed: seeds.resolved(scenario).map(ResolvedMapSeed),
        encounter_override: None,
    });
    next.set(Screen::Loading);
}

/// Opens the lattice ruleset demo when its button is pressed.
fn open_character_creator(
    mut commands: Commands,
    clicked: Query<&Interaction, (Changed<Interaction>, With<OpensCharacterCreator>)>,
    mut next: ResMut<NextState<Screen>>,
) {
    for interaction in &clicked {
        if *interaction == Interaction::Pressed {
            commands.insert_resource(CreatorEntryRequest::CharacterLibrary);
            next.set(Screen::CharacterCreator);
        }
    }
}

fn open_spell_creator(
    mut commands: Commands,
    clicked: Query<&Interaction, (Changed<Interaction>, With<OpensSpellCreator>)>,
    mut next: ResMut<NextState<Screen>>,
) {
    for interaction in &clicked {
        if *interaction == Interaction::Pressed {
            commands.insert_resource(CreatorEntryRequest::SpellLibrary);
            next.set(Screen::SpellCreator);
        }
    }
}

fn open_combat_lab(
    clicked: Query<&Interaction, (Changed<Interaction>, With<OpensCombatLab>)>,
    mut next: ResMut<NextState<Screen>>,
) {
    for interaction in &clicked {
        if *interaction == Interaction::Pressed {
            next.set(Screen::CombatLab);
        }
    }
}

fn open_settings(
    clicked: Query<&Interaction, (Changed<Interaction>, With<OpensSettings>)>,
    mut next: ResMut<NextState<Screen>>,
) {
    if clicked
        .iter()
        .any(|interaction| *interaction == Interaction::Pressed)
    {
        next.set(Screen::Settings);
    }
}

fn quit_game(
    clicked: Query<&Interaction, (Changed<Interaction>, With<QuitsGame>)>,
    mut exit: MessageWriter<AppExit>,
) {
    if clicked
        .iter()
        .any(|interaction| *interaction == Interaction::Pressed)
    {
        exit.write(AppExit::Success);
    }
}

fn handle_input(
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<InputBindings>,
    mut exit: MessageWriter<AppExit>,
) {
    if bindings.just_pressed(&keys, InputAction::Cancel) {
        exit.write(AppExit::Success);
    }
}

#[cfg(test)]
mod tests {
    use bevy::state::app::StatesPlugin;
    use bevy::MinimalPlugins;
    use hex_assets::{Scenario, ScenarioCategory};

    use super::*;

    fn scenario(name: &str) -> Scenario {
        Scenario {
            name: name.to_owned(),
            category: ScenarioCategory::Map,
            blurb: "A map.".to_owned(),
            world: "config/world.ron".to_owned(),
            lighting: "config/lighting.ron".to_owned(),
            generation_seed: None,
            starting_time_hours: None,
            encounter: "config/encounters/bridge-crossing.ron".to_owned(),
        }
    }

    fn seeded_scenario(name: &str, seed: u64) -> Scenario {
        Scenario {
            generation_seed: Some(seed),
            encounter: "config/encounters/anchored-skirmish.ron".to_owned(),
            ..scenario(name)
        }
    }

    fn in_category(name: &str, category: ScenarioCategory) -> Scenario {
        Scenario {
            category,
            ..scenario(name)
        }
    }

    fn library() -> ScenarioLibrary {
        ScenarioLibrary {
            default_game: "Default".to_owned(),
            scenarios: vec![
                in_category("Default", ScenarioCategory::Demo),
                scenario("First"),
            ],
        }
    }

    fn two_scenario_library() -> ScenarioLibrary {
        ScenarioLibrary {
            default_game: "Default".to_owned(),
            scenarios: vec![
                in_category("Default", ScenarioCategory::Demo),
                scenario("First"),
                scenario("Second"),
            ],
        }
    }

    fn shipped_library() -> ScenarioLibrary {
        ron::from_str(include_str!("../../../../assets/config/scenarios.ron"))
            .expect("the shipped scenario library should parse")
    }

    fn test_app_with(library: ScenarioLibrary) -> App {
        let mut app = App::new();
        // `InputPlugin` because `handle_input` reads `ButtonInput<KeyCode>`, which
        // `MinimalPlugins` does not provide -- and a missing system parameter is a
        // panic rather than a skipped system.
        app.add_plugins((MinimalPlugins, StatesPlugin, bevy::input::InputPlugin));
        app.init_state::<Screen>();
        app.insert_resource(library);
        // Default handles stand in for the real fonts; layout logic under test
        // does not care what the glyphs would rasterize from.
        app.insert_resource(UiAssets {
            display: Handle::default(),
            body: Handle::default(),
            hex_cell: Handle::default(),
        });
        app.add_plugins(super::plugin);
        app
    }

    fn test_app() -> App {
        test_app_with(library())
    }

    fn go_to(app: &mut App, screen: Screen) {
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(screen);
        app.update();
        app.update();
    }

    fn buttons(app: &mut App) -> usize {
        let mut query = app
            .world_mut()
            .query_filtered::<Entity, With<StartsScenario>>();
        query.iter(app.world()).count()
    }

    fn scenario_entries(app: &mut App) -> usize {
        let mut query = app
            .world_mut()
            .query_filtered::<Entity, With<ScenarioEntry>>();
        query.iter(app.world()).count()
    }

    fn assert_scrollable_scenario_lists(app: &mut App) {
        let world = app.world_mut();
        let mut lists = world.query_filtered::<
            (&ScenarioList, &Node, Option<&ScrollPosition>),
            (With<ScenarioList>, With<ScrollArea>),
        >();
        let mut categories = Vec::new();
        for (list, node, scroll) in lists.iter(world) {
            categories.push(list.0);
            assert_eq!(node.width, Val::Percent(100.0));
            assert_eq!(node.min_height, Val::Px(0.0));
            assert_eq!(node.flex_basis, Val::Px(0.0));
            assert!((node.flex_grow - 1.0).abs() <= f32::EPSILON);
            assert_eq!(node.overflow.y, OverflowAxis::Scroll);
            assert!(
                scroll.is_some(),
                "{:?} needs its own ScrollPosition for independent wheel input",
                list.0
            );
        }
        categories.sort_by_key(|category| match category {
            ScenarioCategory::Map => 0,
            ScenarioCategory::Demo => 1,
        });
        assert_eq!(
            categories,
            vec![ScenarioCategory::Map, ScenarioCategory::Demo],
            "every closed category needs one independently scrollable lane"
        );
    }

    fn button_named(app: &mut App, name: &str) -> Entity {
        let world = app.world_mut();
        let mut query = world.query::<(Entity, &StartsScenario)>();
        query
            .iter(world)
            .find_map(|(entity, starts)| (starts.scenario.name == name).then_some(entity))
            .expect("the requested scenario button should exist")
    }

    fn button_category(app: &mut App, button: Entity) -> ScenarioCategory {
        let world = app.world();
        let entry = world
            .get::<ChildOf>(button)
            .expect("a scenario button belongs to its entry")
            .parent();
        let list = world
            .get::<ChildOf>(entry)
            .expect("a scenario entry belongs directly to a list")
            .parent();
        world
            .get::<ScenarioList>(list)
            .expect("the entry parent is a category list")
            .0
    }

    fn rendered_names(app: &mut App, category: ScenarioCategory) -> Vec<String> {
        let list = {
            let world = app.world_mut();
            let mut lists = world.query::<(Entity, &ScenarioList)>();
            lists
                .iter(world)
                .find_map(|(entity, list)| (list.0 == category).then_some(entity))
                .expect("the requested category list exists")
        };
        let world = app.world();
        world
            .get::<Children>(list)
            .into_iter()
            .flatten()
            .filter_map(|entry| world.get::<Children>(*entry))
            .flat_map(|children| children.iter())
            .filter_map(|button| world.get::<StartsScenario>(button))
            .map(|starts| starts.scenario.name.clone())
            .collect()
    }

    fn reroll_button_named(app: &mut App, name: &str) -> Entity {
        let world = app.world_mut();
        let mut query = world.query::<(Entity, &RerollsScenario)>();
        query
            .iter(world)
            .find_map(|(entity, rerolls)| (rerolls.scenario.name == name).then_some(entity))
            .expect("the requested scenario should have a reroll button")
    }

    fn has_text(app: &mut App, wanted: &str) -> bool {
        let world = app.world_mut();
        let mut query = world.query::<&Text>();
        query.iter(world).any(|text| text.0.contains(wanted))
    }

    #[test]
    fn new_game_resolves_the_hidden_default_independently() {
        let mut app = test_app();
        go_to(&mut app, Screen::Title);

        assert_eq!(
            rendered_names(&mut app, ScenarioCategory::Demo),
            Vec::<String>::new(),
            "the default leaked into the visible demo lane"
        );
        let new_game = {
            let world = app.world_mut();
            let mut query = world.query_filtered::<Entity, With<StartsNewGame>>();
            query
                .single(world)
                .expect("the Actions lane should have one New Game button")
        };
        app.world_mut()
            .entity_mut(new_game)
            .insert(Interaction::Pressed);
        app.update();

        let pending = app.world().resource::<ScenarioToLoad>();
        assert_eq!(pending.scenario.name, "Default");
        assert!(matches!(
            app.world().resource::<NextState<Screen>>(),
            NextState::Pending(Screen::Loading)
        ));
    }

    /// Character authoring is a static Demo-lane entry and switches screens.
    #[test]
    fn the_creator_button_opens_the_creator_screen() {
        let mut app = test_app();
        go_to(&mut app, Screen::Title);

        let world = app.world_mut();
        let mut buttons = world.query_filtered::<Entity, With<OpensCharacterCreator>>();
        let button = buttons
            .single(world)
            .expect("the title screen should offer exactly one creator button");
        app.world_mut()
            .entity_mut(button)
            .insert(Interaction::Pressed);
        app.update();
        app.update();

        assert_eq!(
            *app.world().resource::<State<Screen>>().get(),
            Screen::CharacterCreator,
            "pressing the creator button should enter the creator screen"
        );
    }

    #[test]
    fn the_spell_creator_button_opens_the_spell_creator_screen() {
        let mut app = test_app();
        go_to(&mut app, Screen::Title);

        let world = app.world_mut();
        let mut buttons = world.query_filtered::<Entity, With<OpensSpellCreator>>();
        let button = buttons
            .single(world)
            .expect("the title screen should offer exactly one spell creator button");
        app.world_mut()
            .entity_mut(button)
            .insert(Interaction::Pressed);
        app.update();
        app.update();

        assert_eq!(
            *app.world().resource::<State<Screen>>().get(),
            Screen::SpellCreator,
            "pressing the spell creator button should enter its dedicated screen"
        );
    }

    #[test]
    fn every_development_category_renders_scenarios_and_demo_keeps_its_static_card() {
        let mut app = test_app_with(ScenarioLibrary {
            default_game: "Default".to_owned(),
            scenarios: vec![
                in_category("Default", ScenarioCategory::Demo),
                in_category("Map One", ScenarioCategory::Map),
                in_category("Demo One", ScenarioCategory::Demo),
                in_category("Map Two", ScenarioCategory::Map),
            ],
        });
        go_to(&mut app, Screen::Title);

        for (name, category) in [
            ("Map One", ScenarioCategory::Map),
            ("Map Two", ScenarioCategory::Map),
        ] {
            let button = button_named(&mut app, name);
            assert_eq!(
                button_category(&mut app, button),
                category,
                "{name} disappeared from its authored lane"
            );
            let node = app
                .world()
                .get::<Node>(button)
                .expect("the launch card has layout");
            assert_eq!(node.width, Val::Percent(100.0));
            assert_eq!(
                node.height,
                Val::Auto,
                "cards grow with wrapped blurbs instead of clipping to a fixed height"
            );
        }
        assert_eq!(
            rendered_names(&mut app, ScenarioCategory::Map),
            vec!["Map One".to_owned(), "Map Two".to_owned()],
            "filtering into a lane must preserve source-file order"
        );

        let world = app.world_mut();
        let mut wave_six_wrappers = world.query::<(&Name, &Node)>();
        let (_, wrapper) = wave_six_wrappers
            .iter(world)
            .find(|(name, _)| name.as_str() == "Static Wave 6 Demo Entries")
            .expect("the three Wave 6 entries share one Demo-lane wrapper");
        assert_eq!(
            wrapper.flex_direction,
            FlexDirection::Column,
            "both creators and Combat Lab must stack vertically at full width"
        );
        let mut static_entries = world.query_filtered::<Entity, With<StaticDemoEntry>>();
        assert_eq!(
            static_entries.iter(world).count(),
            1,
            "scenario-backed demos must not replace the static rules demo"
        );
        let mut demo_buttons = world.query_filtered::<(&Name, Entity), Or<(
            With<OpensCharacterCreator>,
            With<OpensSpellCreator>,
            With<OpensCombatLab>,
        )>>();
        let buttons: Vec<_> = demo_buttons.iter(world).collect();
        assert_eq!(buttons.len(), 3);
        let static_button = buttons
            .iter()
            .find_map(|(name, entity)| (name.as_str() == "Character Creator").then_some(*entity))
            .expect("the static character creator entry exists");

        let static_entry = world
            .get::<ChildOf>(static_button)
            .expect("the static button belongs to its card")
            .parent();
        let demo_list = world
            .get::<ChildOf>(static_entry)
            .expect("the static card belongs to a category list")
            .parent();
        assert_eq!(
            world.get::<ScenarioList>(demo_list),
            Some(&ScenarioList(ScenarioCategory::Demo))
        );
    }

    #[test]
    fn gameplay_setup_failure_is_visible_on_the_title_screen() {
        let mut app = test_app();
        app.insert_resource(GameplaySetupFailure::new(
            "The selected map could not publish a party anchor.",
        ));

        go_to(&mut app, Screen::Title);

        assert!(has_text(
            &mut app,
            "The selected map could not publish a party anchor."
        ));
        let mut notices = app
            .world_mut()
            .query_filtered::<Entity, With<SetupFailureNotice>>();
        assert_eq!(notices.iter(app.world()).count(), 1);
    }

    /// The real library is large enough to need a viewport at the default window height.
    /// Every row must still be built; scrolling changes presentation, not content.
    #[test]
    fn shipped_scenarios_build_all_rows_inside_a_scroll_area() {
        let library = shipped_library();
        let expected_rows = library
            .visible_scenarios()
            .filter(|scenario| scenario.category == ScenarioCategory::Map)
            .count();
        let mut app = test_app_with(library);

        go_to(&mut app, Screen::Title);

        assert_eq!(scenario_entries(&mut app), expected_rows);
        assert_eq!(buttons(&mut app), expected_rows);
        assert_scrollable_scenario_lists(&mut app);
        let category = ScenarioCategory::Map;
        let expected: Vec<String> = app
            .world()
            .resource::<ScenarioLibrary>()
            .visible_scenarios()
            .filter(|scenario| scenario.category == category)
            .map(|scenario| scenario.name.clone())
            .collect();
        assert_eq!(
            rendered_names(&mut app, category),
            expected,
            "every non-default shipped entry should appear exactly once in its authored lane"
        );
        assert!(rendered_names(&mut app, ScenarioCategory::Demo).is_empty());
    }

    /// The menu still has its scenarios when you come back to it.
    ///
    /// Reported from play: quitting to the title left it stuck on "loading scenarios…"
    /// for the rest of the session. The list was rebuilt only when the library was
    /// *added or changed*, which happens exactly once per run — so the second visit
    /// spawned a fresh, empty list and nothing ever filled it.
    ///
    /// The fix is to reconcile from what is on screen rather than from a change event,
    /// which is the same lesson the turn ring taught: state, not edges.
    #[test]
    fn returning_to_the_title_screen_rebuilds_the_list() {
        let mut app = test_app();

        go_to(&mut app, Screen::Title);
        assert_eq!(
            buttons(&mut app),
            1,
            "the first visit should list a scenario"
        );
        assert_scrollable_scenario_lists(&mut app);

        go_to(&mut app, Screen::Gameplay);
        assert_eq!(
            buttons(&mut app),
            0,
            "leaving the title screen should take its buttons with it"
        );

        go_to(&mut app, Screen::Title);
        assert_eq!(
            buttons(&mut app),
            1,
            "coming back left the menu empty — the list was never repopulated"
        );
        assert_scrollable_scenario_lists(&mut app);
    }

    /// A library that arrives after the screen does still gets listed.
    ///
    /// `Screen::Title` is reached on a wall-clock timer rather than a load gate, so on
    /// a cold disk the menu really can exist before `scenarios.ron` has parsed.
    #[test]
    fn a_library_that_arrives_late_still_gets_listed() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin, bevy::input::InputPlugin));
        app.init_state::<Screen>();
        app.insert_resource(UiAssets {
            display: Handle::default(),
            body: Handle::default(),
            hex_cell: Handle::default(),
        });
        app.add_plugins(super::plugin);

        go_to(&mut app, Screen::Title);
        assert_eq!(buttons(&mut app), 0, "nothing to list yet");

        app.insert_resource(library());
        app.update();
        app.update();

        assert_eq!(buttons(&mut app), 1, "the library arrived and was ignored");
    }

    /// A click means the entry that was drawn, even if a hot reload has reordered the
    /// library before that click is processed.
    #[test]
    fn reordering_the_library_cannot_change_an_in_flight_click() {
        let mut app = test_app_with(two_scenario_library());
        go_to(&mut app, Screen::Title);
        let first_button = button_named(&mut app, "First");

        let mut reordered = two_scenario_library();
        reordered.scenarios.reverse();
        app.insert_resource(reordered);
        app.world_mut()
            .entity_mut(first_button)
            .insert(Interaction::Pressed);
        app.update();

        let pending = app.world().resource::<ScenarioToLoad>();
        assert_eq!(
            pending.scenario.name, "First",
            "the old First button was reinterpreted through the reordered library"
        );
    }

    /// Removing an entry cannot invalidate a click that already landed on its button.
    #[test]
    fn removing_the_clicked_entry_cannot_invalidate_an_in_flight_click() {
        let mut app = test_app_with(two_scenario_library());
        go_to(&mut app, Screen::Title);
        let second_button = button_named(&mut app, "Second");

        app.insert_resource(library());
        app.world_mut()
            .entity_mut(second_button)
            .insert(Interaction::Pressed);
        app.update();

        let pending = app.world().resource::<ScenarioToLoad>();
        assert_eq!(
            pending.scenario.name, "Second",
            "removing Second from the library discarded the click on its existing button"
        );
    }

    /// Authored scenarios have no meaningless seed control, while generated scenarios
    /// show one and snapshot their configured seed when started.
    #[test]
    fn only_seeded_scenarios_offer_rerolls_and_capture_the_seed() {
        let mut app = test_app_with(ScenarioLibrary {
            default_game: "Default".to_owned(),
            scenarios: vec![
                in_category("Default", ScenarioCategory::Demo),
                scenario("Authored"),
                seeded_scenario("Generated", 42),
            ],
        });
        go_to(&mut app, Screen::Title);

        let world = app.world_mut();
        let mut rerolls = world.query::<&RerollsScenario>();
        let names: Vec<String> = rerolls
            .iter(world)
            .map(|entry| entry.scenario.name.clone())
            .collect();
        assert_eq!(names, vec!["Generated".to_owned()]);
        assert!(
            has_text(&mut app, "seed 42"),
            "the configured seed is not visible beside the generated scenario"
        );

        let generated = button_named(&mut app, "Generated");
        app.world_mut()
            .entity_mut(generated)
            .insert(Interaction::Pressed);
        app.update();

        let pending = app.world().resource::<ScenarioToLoad>();
        assert_eq!(pending.resolved_seed, Some(ResolvedMapSeed(42)));
    }

    /// A reroll survives title-screen re-entry but never changes the scenario asset.
    #[test]
    fn rerolled_seed_is_session_only_and_used_by_the_next_click() {
        let configured = 42;
        let scenario = seeded_scenario("Generated", configured);
        let mut app = test_app_with(ScenarioLibrary {
            default_game: "Default".to_owned(),
            scenarios: vec![
                in_category("Default", ScenarioCategory::Demo),
                scenario.clone(),
            ],
        });
        // Stable entropy makes the assertion deterministic without changing production
        // behaviour, where the resource is initialized from wall-clock time.
        app.insert_resource(SessionSeeds {
            overrides: HashMap::default(),
            entropy: 7,
        });
        go_to(&mut app, Screen::Title);

        let reroll = reroll_button_named(&mut app, "Generated");
        app.world_mut()
            .entity_mut(reroll)
            .insert(Interaction::Pressed);
        app.update();

        let rerolled = app
            .world()
            .resource::<SessionSeeds>()
            .resolved(&scenario)
            .expect("the generated scenario should have a seed");
        assert_ne!(rerolled, configured);
        app.update();
        assert!(
            has_text(&mut app, &format!("seed {rerolled}")),
            "the reroll control did not display the replacement seed"
        );
        assert_eq!(
            app.world()
                .resource::<ScenarioLibrary>()
                .scenarios
                .iter()
                .find(|entry| entry.name == "Generated")
                .and_then(|entry| entry.generation_seed),
            Some(configured),
            "rerolling modified the loaded scenario configuration"
        );

        go_to(&mut app, Screen::Gameplay);
        go_to(&mut app, Screen::Title);
        let generated = button_named(&mut app, "Generated");
        app.world_mut()
            .entity_mut(generated)
            .insert(Interaction::Pressed);
        app.update();

        let pending = app.world().resource::<ScenarioToLoad>();
        assert_eq!(pending.resolved_seed, Some(ResolvedMapSeed(rerolled)));
    }
}
