//! Main menu: pick a scenario and start it.
//!
//! One button per scenario in `assets/config/scenarios.ron`, so adding a map to the
//! menu is a content change rather than a code one.
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
use hex_assets::{Scenario, ScenarioLibrary};
use hex_core::{GameplaySetupFailure, ResolvedMapSeed, Screen};

use crate::menus::widgets::{
    blurb, button, display, divider, fine, heading, label, small_button, UiAssets, ACCENT_EDGE,
    BLURB_SIZE,
};
use crate::scenarios::ScenarioToLoad;

use super::{despawn_screen, screen_root};

/// The scenario table and the static controls below it share this alignment.
const SCENARIO_LIST_WIDTH: f32 = 564.0;
/// Keeps the menu compact on tall windows while flexing down on the default 720p view.
const SCENARIO_LIST_MAX_HEIGHT: f32 = 600.0;

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<SessionSeeds>();
    app.add_systems(OnEnter(Screen::Title), spawn_title);
    app.add_systems(
        Update,
        (
            rebuild_scenario_list,
            reroll_scenario_seed,
            start_chosen_scenario,
            open_lattice_demo,
            handle_input,
        )
            .chain()
            .run_if(in_state(Screen::Title)),
    );
    app.add_systems(OnExit(Screen::Title), despawn_screen(Screen::Title));
}

/// The node the scenario buttons hang off.
#[derive(Component)]
struct ScenarioList;

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

/// The button that opens the lattice ruleset demo.
#[derive(Component)]
struct OpensLatticeDemo;

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
            parent.spawn((
                Node {
                    margin: UiRect::bottom(Val::Px(10.0)),
                    ..default()
                },
                children![heading(&assets, "scenarios")],
            ));
            if let Some(reason) = failure_reason {
                parent.spawn((
                    Name::new("Gameplay Setup Failure"),
                    SetupFailureNotice,
                    Text::new(reason),
                    TextFont {
                        font: assets.body.clone().into(),
                        ..TextFont::from_font_size(BLURB_SIZE)
                    },
                    TextColor(Color::srgb(0.95, 0.45, 0.40)),
                    Node {
                        max_width: Val::Px(720.0),
                        ..default()
                    },
                ));
            }
            parent
                .spawn((
                    Name::new("Scenario List"),
                    ScenarioList,
                    ScrollArea,
                    Node {
                        width: Val::Px(SCENARIO_LIST_WIDTH),
                        min_height: Val::Px(0.0),
                        max_height: Val::Px(SCENARIO_LIST_MAX_HEIGHT),
                        flex_basis: Val::Px(0.0),
                        flex_grow: 1.0,
                        flex_shrink: 1.0,
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(8.0),
                        align_items: AlignItems::Center,
                        overflow: Overflow::scroll_y(),
                        ..default()
                    },
                ))
                .with_children(|list| {
                    list.spawn((ListPlaceholder, blurb(&assets, "loading scenarios...")));
                });
            parent.spawn(divider(SCENARIO_LIST_WIDTH));
            parent
                .spawn((button("Lattice Demo"), OpensLatticeDemo))
                .insert(BorderColor::all(ACCENT_EDGE))
                .with_children(|entry| {
                    entry.spawn(label(&assets, "Lattice Demo"));
                    entry.spawn(blurb(
                        &assets,
                        "Poke the magic ruleset: cast, channel, strike, break enchantments.",
                    ));
                });
            parent.spawn((
                Node {
                    margin: UiRect::top(Val::Px(8.0)),
                    ..default()
                },
                children![blurb(&assets, "click a scenario to play   ·   ESC to quit")],
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

/// Fills the list once the library is known, and again if it changes on disk.
fn rebuild_scenario_list(
    mut commands: Commands,
    library: Option<Res<ScenarioLibrary>>,
    lists: Query<Entity, With<ScenarioList>>,
    placeholders: Query<Entity, With<ListPlaceholder>>,
    existing: Query<Entity, With<ScenarioEntry>>,
    clicked_starts: Query<&Interaction, (Changed<Interaction>, With<StartsScenario>)>,
    clicked_rerolls: Query<&Interaction, (Changed<Interaction>, With<RerollsScenario>)>,
    seeds: Res<SessionSeeds>,
    assets: Res<UiAssets>,
) {
    let Some(library) = library else { return };
    let Ok(list) = lists.single() else { return };

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

    for scenario in &library.scenarios {
        // A fixed-width row with a permanent right-hand slot: rows with and
        // without a seed control keep identical left edges. The first walk
        // photograph showed the zig-zag the old optional slot produced.
        let row = commands
            .spawn((
                Name::new("Scenario Entry"),
                ScenarioEntry,
                Node {
                    width: Val::Px(SCENARIO_LIST_WIDTH),
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(12.0),
                    align_items: AlignItems::Center,
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
            .with_children(|entry| {
                entry.spawn(label(&assets, scenario.name.clone()));
                entry.spawn(blurb(&assets, scenario.blurb.clone()));
            })
            .id();
        commands.entity(row).add_child(launch);

        let slot = if let Some(seed) = seed {
            commands
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
                .id()
        } else {
            commands
                .spawn((
                    Name::new("Seed Slot Spacer"),
                    Node {
                        width: Val::Px(132.0),
                        ..default()
                    },
                ))
                .id()
        };
        commands.entity(row).add_child(slot);

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
            commands.insert_resource(ScenarioToLoad {
                scenario: starts.scenario.clone(),
                resolved_seed: seeds.resolved(&starts.scenario).map(ResolvedMapSeed),
            });
            next.set(Screen::Loading);
        }
    }
}

/// Opens the lattice ruleset demo when its button is pressed.
fn open_lattice_demo(
    clicked: Query<&Interaction, (Changed<Interaction>, With<OpensLatticeDemo>)>,
    mut next: ResMut<NextState<Screen>>,
) {
    for interaction in &clicked {
        if *interaction == Interaction::Pressed {
            next.set(Screen::LatticeDemo);
        }
    }
}

fn handle_input(keys: Res<ButtonInput<KeyCode>>, mut exit: MessageWriter<AppExit>) {
    if keys.just_pressed(KeyCode::Escape) {
        exit.write(AppExit::Success);
    }
}

#[cfg(test)]
mod tests {
    use bevy::state::app::StatesPlugin;
    use bevy::MinimalPlugins;
    use hex_assets::{CubeCoord, Scenario, ScenarioPlacement, ScenarioSettings};

    use super::*;

    fn at(x: i32, y: i32, z: i32) -> CubeCoord {
        CubeCoord { x, y, z }
    }

    fn scenario(name: &str, enemy: CubeCoord) -> Scenario {
        Scenario {
            name: name.to_owned(),
            blurb: "A map.".to_owned(),
            world: "config/world.ron".to_owned(),
            lighting: "config/lighting.ron".to_owned(),
            generation_seed: None,
            starting_time_hours: None,
            units: ScenarioSettings {
                player: ScenarioPlacement::Fixed(at(0, 0, 0)),
                enemy: ScenarioPlacement::Fixed(enemy),
            },
        }
    }

    fn seeded_scenario(name: &str, seed: u64) -> Scenario {
        Scenario {
            generation_seed: Some(seed),
            ..scenario(name, at(1, -1, 0))
        }
    }

    fn library() -> ScenarioLibrary {
        ScenarioLibrary {
            scenarios: vec![scenario("First", at(1, -1, 0))],
        }
    }

    fn two_scenario_library() -> ScenarioLibrary {
        ScenarioLibrary {
            scenarios: vec![
                scenario("First", at(1, -1, 0)),
                scenario("Second", at(2, -2, 0)),
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

    fn assert_scrollable_scenario_list(app: &mut App) {
        let world = app.world_mut();
        let mut lists = world.query_filtered::<
            (&Node, Option<&ScrollPosition>),
            (With<ScenarioList>, With<ScrollArea>),
        >();
        let (node, scroll) = lists
            .single(world)
            .expect("the title should have exactly one scrollable scenario list");

        assert_eq!(node.width, Val::Px(SCENARIO_LIST_WIDTH));
        assert_eq!(node.min_height, Val::Px(0.0));
        assert_eq!(node.max_height, Val::Px(SCENARIO_LIST_MAX_HEIGHT));
        assert_eq!(node.flex_basis, Val::Px(0.0));
        assert!((node.flex_grow - 1.0).abs() <= f32::EPSILON);
        assert_eq!(node.overflow.y, OverflowAxis::Scroll);
        assert!(
            scroll.is_some(),
            "ScrollArea should require the ScrollPosition its wheel observer updates"
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

    /// The demo button is part of the static title layout and switches screens.
    #[test]
    fn the_lattice_demo_button_opens_the_demo_screen() {
        let mut app = test_app();
        go_to(&mut app, Screen::Title);

        let world = app.world_mut();
        let mut buttons = world.query_filtered::<Entity, With<OpensLatticeDemo>>();
        let button = buttons
            .single(world)
            .expect("the title screen should offer exactly one demo button");
        app.world_mut()
            .entity_mut(button)
            .insert(Interaction::Pressed);
        app.update();
        app.update();

        assert_eq!(
            *app.world().resource::<State<Screen>>().get(),
            Screen::LatticeDemo,
            "pressing the demo button should enter the demo screen"
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
        assert_eq!(
            library.scenarios.len(),
            11,
            "update the title-screen coverage when the shipped scenario count changes"
        );
        let expected_rows = library.scenarios.len();
        let mut app = test_app_with(library);

        go_to(&mut app, Screen::Title);

        assert_eq!(scenario_entries(&mut app), expected_rows);
        assert_eq!(buttons(&mut app), expected_rows);
        assert_scrollable_scenario_list(&mut app);
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
        assert_scrollable_scenario_list(&mut app);

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
        assert_scrollable_scenario_list(&mut app);
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
            scenarios: vec![
                scenario("Authored", at(1, -1, 0)),
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
            scenarios: vec![scenario.clone()],
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
                .first()
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
