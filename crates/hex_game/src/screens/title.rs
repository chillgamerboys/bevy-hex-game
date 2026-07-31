//! Title-screen application adapter.
//!
//! This module publishes immutable scenario facts to `hex_ui` and applies typed
//! title intents. It contains no Bevy UI nodes or presentation state.

use std::time::{SystemTime, UNIX_EPOCH};

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use hex_assets::{Scenario, ScenarioCategory, ScenarioLibrary};
use hex_core::{GameplaySetupFailure, InputAction, InputBindings, ResolvedMapSeed, Screen};
use hex_ui::{TitleIntent, TitleScenarioView, TitleView, UiIntent, UiSystems};

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
            (publish_title_view, handle_input).run_if(in_state(Screen::Title)),
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

fn publish_title_view(
    library: Option<Res<ScenarioLibrary>>,
    failure: Option<Res<GameplaySetupFailure>>,
    seeds: Res<SessionSeeds>,
    mut view: ResMut<TitleView>,
    mut last_projection: Local<Option<String>>,
) {
    let scenarios = library
        .as_deref()
        .map(|library| {
            library
                .visible_scenarios()
                // Focused mechanics demos live behind Combat Lab. Only world-first
                // scenarios remain direct data-backed title cards.
                .filter(|scenario| scenario.category == ScenarioCategory::Map)
                .map(|scenario| TitleScenarioView {
                    scenario: scenario.clone(),
                    resolved_seed: seeds.resolved(scenario),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let setup_failure = failure.as_deref().map(|failure| failure.reason.clone());
    let projection = format!("{scenarios:?}|{setup_failure:?}");
    if last_projection.as_ref() == Some(&projection) {
        return;
    }
    *last_projection = Some(projection);
    *view = TitleView {
        scenarios,
        setup_failure,
    };
}

fn handle_title_intents(
    mut intents: MessageReader<UiIntent>,
    library: Option<Res<ScenarioLibrary>>,
    mut seeds: ResMut<SessionSeeds>,
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
            TitleIntent::StartScenario(scenario) => {
                launch_scenario(&mut commands, &mut next, &seeds, scenario.clone());
            }
            TitleIntent::RerollScenario(scenario) => {
                if let Some(seed) = seeds.reroll(scenario) {
                    info!("rerolled scenario seed: {} -> {}", scenario.name, seed);
                }
            }
            TitleIntent::CharacterCreator => {
                commands.insert_resource(CreatorEntryRequest::CharacterLibrary);
                next.set(Screen::CharacterCreator);
            }
            TitleIntent::SpellCreator => {
                commands.insert_resource(CreatorEntryRequest::SpellLibrary);
                next.set(Screen::SpellCreator);
            }
            TitleIntent::CombatLab => next.set(Screen::CombatLab),
            TitleIntent::Settings => next.set(Screen::Settings),
            TitleIntent::Quit => {
                exit.write(AppExit::Success);
            }
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

    fn test_app_with(library: Option<ScenarioLibrary>) -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin, bevy::input::InputPlugin))
            .init_state::<Screen>()
            .init_resource::<TitleView>()
            .add_message::<UiIntent>();
        if let Some(library) = library {
            app.insert_resource(library);
        }
        plugin(&mut app);
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Title);
        app.update();
        app.update();
        app
    }

    fn send(app: &mut App, intent: TitleIntent) {
        app.world_mut().write_message(UiIntent::Title(intent));
        app.update();
    }

    #[test]
    fn projection_lists_only_world_first_development_scenarios() {
        let app = test_app_with(Some(ScenarioLibrary {
            default_game: "Default".to_owned(),
            scenarios: vec![
                in_category("Default", ScenarioCategory::Demo),
                scenario("Map One"),
                in_category("Focused Demo", ScenarioCategory::Demo),
                scenario("Map Two"),
            ],
        }));
        let names = app
            .world()
            .resource::<TitleView>()
            .scenarios
            .iter()
            .map(|entry| entry.scenario.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, ["Map One", "Map Two"]);
    }

    #[test]
    fn a_library_that_arrives_late_updates_the_projection() {
        let mut app = test_app_with(None);
        assert!(app.world().resource::<TitleView>().scenarios.is_empty());
        app.insert_resource(library());
        app.update();
        assert_eq!(app.world().resource::<TitleView>().scenarios.len(), 1);
    }

    #[test]
    fn new_game_resolves_the_hidden_default_independently() {
        let mut app = test_app_with(Some(library()));
        send(&mut app, TitleIntent::NewGame);
        let pending = app.world().resource::<ScenarioToLoad>();
        assert_eq!(pending.scenario.name, "Default");
        assert!(matches!(
            app.world().resource::<NextState<Screen>>(),
            NextState::Pending(Screen::Loading)
        ));
    }

    #[test]
    fn creator_and_lab_routes_remain_typed_navigation() {
        for (intent, expected) in [
            (TitleIntent::CharacterCreator, Screen::CharacterCreator),
            (TitleIntent::SpellCreator, Screen::SpellCreator),
            (TitleIntent::CombatLab, Screen::CombatLab),
            (TitleIntent::Settings, Screen::Settings),
        ] {
            let mut app = test_app_with(Some(library()));
            send(&mut app, intent);
            assert!(matches!(
                app.world().resource::<NextState<Screen>>(),
                NextState::Pending(screen) if *screen == expected
            ));
        }
    }

    #[test]
    fn an_in_flight_card_keeps_its_exact_scenario_snapshot() {
        let mut app = test_app_with(Some(ScenarioLibrary {
            default_game: "Default".to_owned(),
            scenarios: vec![
                in_category("Default", ScenarioCategory::Demo),
                scenario("First"),
                scenario("Second"),
            ],
        }));
        let clicked = app
            .world()
            .resource::<ScenarioLibrary>()
            .scenarios
            .get(1)
            .expect("First is the second authored entry")
            .clone();
        app.insert_resource(library());
        send(&mut app, TitleIntent::StartScenario(clicked));
        assert_eq!(
            app.world().resource::<ScenarioToLoad>().scenario.name,
            "First"
        );
    }

    #[test]
    fn rerolled_seed_is_session_only_and_used_by_launch() {
        let configured = 42;
        let generated = seeded_scenario("Generated", configured);
        let mut app = test_app_with(Some(ScenarioLibrary {
            default_game: "Default".to_owned(),
            scenarios: vec![
                in_category("Default", ScenarioCategory::Demo),
                generated.clone(),
            ],
        }));
        app.insert_resource(SessionSeeds {
            overrides: HashMap::default(),
            entropy: 7,
        });

        send(&mut app, TitleIntent::RerollScenario(generated.clone()));
        let rerolled = app
            .world()
            .resource::<SessionSeeds>()
            .resolved(&generated)
            .expect("generated scenario has a seed");
        assert_ne!(rerolled, configured);
        assert_eq!(
            app.world()
                .resource::<ScenarioLibrary>()
                .scenarios
                .get(1)
                .and_then(|scenario| scenario.generation_seed),
            Some(configured)
        );

        send(&mut app, TitleIntent::StartScenario(generated));
        assert_eq!(
            app.world().resource::<ScenarioToLoad>().resolved_seed,
            Some(ResolvedMapSeed(rerolled))
        );
    }
}
