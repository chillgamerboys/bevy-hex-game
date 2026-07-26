//! Turning the chosen scenario into the settings the world is built from.
//!
//! This is the one place that can do it. A scenario names its world by **path**,
//! because `hex_assets` cannot mention a terrain type without inverting the crate
//! graph — and `hex_units`, which places the pieces, cannot see `hex_map` at all. The
//! binary sees everything, so the string becomes a `Handle<MapSettings>` here and
//! nowhere else.
//!
//! # Why `OnEnter(Screen::Loading)`
//!
//! State transitions run **before** the same frame's `Update`, so marking the world
//! file as pending here happens before anything can ask whether loading has finished.
//! Move this into `Update` and the gate in `PostUpdate` can pass on the same frame with
//! the *previous* scenario's terrain still installed — a wrong-map bug that renders
//! perfectly and logs nothing.

use bevy::prelude::*;
use hex_assets::{
    choose_settings, ScenarioLibrary, SelectSettings, SelectedScenario, SettingsRegistry,
    CONFIG_EXTENSIONS,
};
use hex_core::Screen;
use hex_map::MapSettings;

pub(super) fn plugin(app: &mut App) {
    // `select_settings` rather than `load_settings`: there is no world file to load
    // until somebody has picked a scenario. It shares the registration `hex_map`
    // already did, which is idempotent, so plugin order does not matter here.
    app.select_settings::<MapSettings>(CONFIG_EXTENSIONS);
    app.init_resource::<SelectedScenario>();
    app.register_type::<SelectedScenario>();
    app.add_systems(OnEnter(Screen::Loading), apply_selected_scenario);
}

/// Asks for the chosen scenario's world, and installs its unit placements.
fn apply_selected_scenario(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut registry: ResMut<SettingsRegistry>,
    library: Option<Res<ScenarioLibrary>>,
    selected: Res<SelectedScenario>,
) {
    let Some(scenario) = library
        .as_deref()
        .and_then(|library| library.scenarios.get(selected.0))
    else {
        // Reachable if `scenarios.ron` hot-reloads shorter than the current selection.
        // Returning without marking anything pending is the important part: leaving a
        // pending entry here would hold the loading screen up for a file nobody asked
        // for, with no way back.
        error!(
            "no scenario at index {} — the world will be whatever was last loaded",
            selected.0
        );
        return;
    };

    info!("starting scenario: {}", scenario.name);
    commands.insert_resource(scenario.units.clone());
    choose_settings::<MapSettings>(&mut commands, &asset_server, &mut registry, &scenario.world);
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use bevy::app::PluginsState;
    use bevy::asset::AssetPlugin;
    use bevy::prelude::*;
    use bevy::state::app::StatesPlugin;
    use bevy::MinimalPlugins;
    use hex_assets::{CubeCoord, ScenarioLibrary, SelectedScenario, SettingsRegistry};
    use hex_core::Screen;
    use hex_map::MapSettings;

    fn library() -> ScenarioLibrary {
        ron::from_str(include_str!("../../../assets/config/scenarios.ron"))
            .expect("the shipped scenarios should parse")
    }

    fn assets_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets")
    }

    /// Cube distance from the centre of the map.
    fn distance_from_centre(coord: CubeCoord) -> u32 {
        let sum = coord.x.abs() + coord.y.abs() + coord.z.abs();
        u32::try_from(sum / 2).unwrap_or(u32::MAX)
    }

    /// Every world a scenario names exists and is a world.
    ///
    /// The path is a plain string, so nothing else can check it: `hex_assets` is not
    /// allowed to know what a map is. A typo would otherwise surface as a game that
    /// sits on the loading screen, and only after someone picked that scenario.
    ///
    /// `MapSettings`'s `Deserialize` runs `validate()`, so this proves each world is
    /// *constructible* rather than merely well-formed RON.
    #[test]
    fn every_scenario_names_a_world_that_exists_and_parses() {
        for scenario in &library().scenarios {
            let path = assets_dir().join(&scenario.world);
            let text = fs::read_to_string(&path).unwrap_or_else(|error| {
                panic!(
                    "scenario {:?} names {:?}, which could not be read: {error}",
                    scenario.name, scenario.world
                )
            });
            let world: Result<MapSettings, _> = ron::from_str(&text);
            assert!(
                world.is_ok(),
                "scenario {:?} names a world that does not parse: {:?}",
                scenario.name,
                world.err()
            );
        }
    }

    /// And every unit starts inside the world it is placed on.
    ///
    /// Not a formality. `coord_from` and `spawn_unit` both warn and fall back to the
    /// centre of the map, so a scenario whose player sits outside its own grid radius
    /// *works* — the piece simply is not where the designer put it, and the only
    /// evidence is a line in the terminal nobody is reading.
    #[test]
    fn every_unit_starts_inside_its_own_world() {
        for scenario in &library().scenarios {
            let text = fs::read_to_string(assets_dir().join(&scenario.world))
                .expect("the world file should exist");
            let world: MapSettings = ron::from_str(&text).expect("the world should parse");

            for (who, coord) in [
                ("player", scenario.units.player),
                ("enemy", scenario.units.enemy),
            ] {
                assert_eq!(
                    coord.x + coord.y + coord.z,
                    0,
                    "scenario {:?}: the {who}'s coordinates do not sum to zero",
                    scenario.name
                );
                assert!(
                    distance_from_centre(coord) <= world.grid_radius,
                    "scenario {:?}: the {who} starts {} hexes out on a map of radius {}",
                    scenario.name,
                    distance_from_centre(coord),
                    world.grid_radius
                );
            }
        }
    }

    /// A harness that can actually reach gameplay, with no renderer.
    ///
    /// `BEVY_ASSET_ROOT` is set for test binaries too, so `config/world.ron` and the
    /// other real world files resolve — this is the one test here that does file IO,
    /// deliberately, because the thing being checked is that a path in a RON file ends
    /// up as terrain.
    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default(), StatesPlugin));
        app.init_state::<Screen>();
        app.add_plugins((hex_map::settings::plugin, super::plugin));
        app.insert_resource(library());
        while app.plugins_state() != PluginsState::Cleaned {
            app.finish();
            app.cleanup();
        }
        app
    }

    /// Runs frames until the world for the chosen scenario has been installed.
    ///
    /// Bounded, and it fails naming what it was still waiting for. An unbounded loop
    /// here turns a regression into a CI job that hangs for its whole timeout with
    /// nothing to read.
    fn settle(app: &mut App) -> MapSettings {
        for _ in 0..600 {
            app.update();
            if app.world().resource::<SettingsRegistry>().all_loaded() {
                if let Some(settings) = app.world().get_resource::<MapSettings>() {
                    return settings.clone();
                }
            }
        }
        panic!(
            "the world never arrived; still waiting on {:?}",
            app.world().resource::<SettingsRegistry>().pending_names()
        );
    }

    fn choose(app: &mut App, index: usize) {
        app.world_mut().resource_mut::<SelectedScenario>().0 = index;
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Loading);
        app.update();
    }

    /// Choosing a scenario installs *its* world and *its* unit placements.
    ///
    /// The second half is the whole test. An implementation that loads a world once and
    /// never re-chooses passes the first half and then plays the first scenario's map
    /// for ever, with the second scenario's units standing on it — which renders
    /// perfectly and logs nothing.
    #[test]
    fn picking_a_different_scenario_changes_the_world() {
        let entries = library().scenarios;
        assert!(
            entries.len() >= 2,
            "this test needs two scenarios to compare"
        );

        let mut app = test_app();

        choose(&mut app, 0);
        let first = settle(&mut app);
        let first_units = app
            .world()
            .get_resource::<hex_assets::ScenarioSettings>()
            .expect("the scenario's placements should be installed")
            .clone();

        // Back to the title screen, exactly as BACKSPACE does, then pick the other one.
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Title);
        app.update();

        choose(&mut app, 1);
        let second = settle(&mut app);
        let second_units = app
            .world()
            .get_resource::<hex_assets::ScenarioSettings>()
            .expect("the scenario's placements should be installed")
            .clone();

        assert_ne!(
            first, second,
            "both scenarios produced the same world, so the choice did nothing"
        );
        assert_ne!(
            (first_units.enemy.x, first_units.enemy.y),
            (second_units.enemy.x, second_units.enemy.y),
            "both scenarios produced the same placements"
        );
    }
}
