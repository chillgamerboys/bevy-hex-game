//! Title and development-scenario application adapters.
//!
//! These systems publish immutable facts to `hex_ui` and apply typed intents. They
//! contain no Bevy UI nodes or presentation-owned navigation state.

use std::time::{SystemTime, UNIX_EPOCH};

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use hex_assets::{Scenario, ScenarioLibrary};
use hex_core::{GameplaySetupFailure, InputAction, InputBindings, ResolvedMapSeed, Screen};
use hex_ui::{
    ScenarioBrowserIntent, ScenarioBrowserView, TitleIntent, TitleScenarioView, TitleView,
    UiIntent, UiSystems,
};

use crate::scenarios::ScenarioToLoad;

use super::creator::CreatorEntryRequest;

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<InputBindings>()
        .init_resource::<SessionSeeds>()
        .add_systems(
            Update,
            handle_title_intents
                .after(UiSystems::EmitIntents)
                .run_if(in_state(Screen::Title)),
        )
        .add_systems(
            Update,
            (publish_title_view, handle_title_input).run_if(in_state(Screen::Title)),
        )
        .add_systems(
            Update,
            handle_scenario_intents
                .after(UiSystems::EmitIntents)
                .run_if(in_state(Screen::Scenarios)),
        )
        .add_systems(
            Update,
            (publish_scenario_view, handle_scenario_input).run_if(in_state(Screen::Scenarios)),
        );
}

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

fn publish_title_view(failure: Option<Res<GameplaySetupFailure>>, mut view: ResMut<TitleView>) {
    let next = TitleView {
        setup_failure: failure.as_deref().map(|failure| failure.reason.clone()),
    };
    if view.setup_failure != next.setup_failure {
        *view = next;
    }
}

fn publish_scenario_view(
    library: Option<Res<ScenarioLibrary>>,
    seeds: Res<SessionSeeds>,
    mut view: ResMut<ScenarioBrowserView>,
    mut last_projection: Local<Option<String>>,
) {
    let scenarios = library
        .as_deref()
        .map(|library| {
            library
                .visible_scenarios()
                .map(|scenario| TitleScenarioView {
                    scenario: scenario.clone(),
                    resolved_seed: seeds.resolved(scenario),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let projection = format!("{scenarios:?}");
    if last_projection.as_ref() == Some(&projection) {
        return;
    }
    *last_projection = Some(projection);
    *view = ScenarioBrowserView { scenarios };
}

fn handle_title_intents(
    mut intents: MessageReader<UiIntent>,
    library: Option<Res<ScenarioLibrary>>,
    seeds: Res<SessionSeeds>,
    mut commands: Commands,
    mut next: ResMut<NextState<Screen>>,
    mut exit: MessageWriter<AppExit>,
) {
    for intent in intents.read() {
        let UiIntent::Title(intent) = intent else {
            continue;
        };
        match intent {
            TitleIntent::Continue => {}
            TitleIntent::NewGame => {
                let Some(library) = library.as_deref() else {
                    warn!("New Game was pressed before the scenario library loaded");
                    continue;
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
                    continue;
                };
                launch_scenario(&mut commands, &mut next, &seeds, scenario.clone());
            }
            TitleIntent::Creators => {
                commands.insert_resource(CreatorEntryRequest::CharacterLibrary);
                next.set(Screen::CharacterCreator);
            }
            TitleIntent::CombatLab => next.set(Screen::CombatLab),
            TitleIntent::Scenarios => next.set(Screen::Scenarios),
            TitleIntent::Settings => next.set(Screen::Settings),
            TitleIntent::Quit => {
                exit.write(AppExit::Success);
            }
        }
    }
}

fn handle_scenario_intents(
    mut intents: MessageReader<UiIntent>,
    mut seeds: ResMut<SessionSeeds>,
    mut commands: Commands,
    mut next: ResMut<NextState<Screen>>,
) {
    for intent in intents.read() {
        let UiIntent::Scenarios(intent) = intent else {
            continue;
        };
        match intent {
            ScenarioBrowserIntent::Start(scenario) => {
                launch_scenario(&mut commands, &mut next, &seeds, scenario.clone());
            }
            ScenarioBrowserIntent::Reroll(scenario) => {
                if let Some(seed) = seeds.reroll(scenario) {
                    info!("rerolled scenario seed: {} -> {}", scenario.name, seed);
                }
            }
            ScenarioBrowserIntent::Back => next.set(Screen::Title),
        }
    }
}

fn launch_scenario(
    commands: &mut Commands,
    next: &mut NextState<Screen>,
    seeds: &SessionSeeds,
    scenario: Scenario,
) {
    commands.remove_resource::<crate::save::PendingResume>();
    let resolved_seed = seeds.resolved(&scenario).map(ResolvedMapSeed);
    commands.insert_resource(ScenarioToLoad {
        scenario,
        resolved_seed,
        encounter_override: None,
    });
    next.set(Screen::Loading);
}

fn handle_title_input(
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<InputBindings>,
    mut exit: MessageWriter<AppExit>,
) {
    if bindings.just_pressed(&keys, InputAction::Cancel) {
        exit.write(AppExit::Success);
    }
}

fn handle_scenario_input(
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<InputBindings>,
    mut next: ResMut<NextState<Screen>>,
) {
    if bindings.just_pressed(&keys, InputAction::Cancel) {
        next.set(Screen::Title);
    }
}

#[cfg(test)]
mod tests {
    use bevy::state::app::StatesPlugin;
    use bevy::MinimalPlugins;
    use hex_assets::ScenarioCategory;

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

    fn test_app_with(library: Option<ScenarioLibrary>, screen: Screen) -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin, bevy::input::InputPlugin))
            .init_state::<Screen>()
            .init_resource::<TitleView>()
            .init_resource::<ScenarioBrowserView>()
            .add_message::<UiIntent>();
        if let Some(library) = library {
            app.insert_resource(library);
        }
        plugin(&mut app);
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(screen);
        app.update();
        app.update();
        app
    }

    fn send_title(app: &mut App, intent: TitleIntent) {
        app.world_mut().write_message(UiIntent::Title(intent));
        app.update();
    }

    fn send_scenario(app: &mut App, intent: ScenarioBrowserIntent) {
        app.world_mut().write_message(UiIntent::Scenarios(intent));
        app.update();
    }

    #[test]
    fn scenario_projection_lists_visible_maps_and_demos() {
        let app = test_app_with(
            Some(ScenarioLibrary {
                default_game: "Default".to_owned(),
                scenarios: vec![
                    in_category("Default", ScenarioCategory::Demo),
                    scenario("Map One"),
                    in_category("Focused Demo", ScenarioCategory::Demo),
                    scenario("Map Two"),
                ],
            }),
            Screen::Scenarios,
        );
        let names = app
            .world()
            .resource::<ScenarioBrowserView>()
            .scenarios
            .iter()
            .map(|entry| entry.scenario.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, ["Map One", "Focused Demo", "Map Two"]);
    }

    #[test]
    fn a_scenario_library_that_arrives_late_updates_the_catalog() {
        let mut app = test_app_with(None, Screen::Scenarios);
        assert!(app
            .world()
            .resource::<ScenarioBrowserView>()
            .scenarios
            .is_empty());

        app.insert_resource(library());
        app.update();

        assert_eq!(
            app.world()
                .resource::<ScenarioBrowserView>()
                .scenarios
                .len(),
            1
        );
    }

    #[test]
    fn new_game_resolves_the_hidden_default_independently() {
        let mut app = test_app_with(Some(library()), Screen::Title);
        send_title(&mut app, TitleIntent::NewGame);
        assert_eq!(
            app.world().resource::<ScenarioToLoad>().scenario.name,
            "Default"
        );
    }

    #[test]
    fn primary_routes_remain_typed_navigation() {
        for (intent, expected) in [
            (TitleIntent::Creators, Screen::CharacterCreator),
            (TitleIntent::CombatLab, Screen::CombatLab),
            (TitleIntent::Scenarios, Screen::Scenarios),
            (TitleIntent::Settings, Screen::Settings),
        ] {
            let mut app = test_app_with(Some(library()), Screen::Title);
            send_title(&mut app, intent);
            assert!(matches!(
                app.world().resource::<NextState<Screen>>(),
                NextState::Pending(screen) if *screen == expected
            ));
        }
    }

    #[test]
    fn an_in_flight_card_keeps_its_exact_scenario_snapshot() {
        let mut app = test_app_with(Some(library()), Screen::Scenarios);
        let clicked = scenario("Clicked Before Reload");
        app.insert_resource(library());
        send_scenario(&mut app, ScenarioBrowserIntent::Start(clicked));
        assert_eq!(
            app.world().resource::<ScenarioToLoad>().scenario.name,
            "Clicked Before Reload"
        );
    }

    #[test]
    fn rerolled_seed_is_session_only_and_used_by_launch() {
        let configured = 42;
        let generated = seeded_scenario("Generated", configured);
        let mut app = test_app_with(Some(library()), Screen::Scenarios);
        app.insert_resource(SessionSeeds {
            overrides: HashMap::default(),
            entropy: 7,
        });

        send_scenario(&mut app, ScenarioBrowserIntent::Reroll(generated.clone()));
        let rerolled = app
            .world()
            .resource::<SessionSeeds>()
            .resolved(&generated)
            .expect("generated scenario has a seed");
        assert_ne!(rerolled, configured);

        send_scenario(&mut app, ScenarioBrowserIntent::Start(generated));
        assert_eq!(
            app.world().resource::<ScenarioToLoad>().resolved_seed,
            Some(ResolvedMapSeed(rerolled))
        );
    }
}
