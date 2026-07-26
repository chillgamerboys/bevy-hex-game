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
    choose_settings, LightingSettings, Scenario, SelectSettings, SettingsRegistry,
    CONFIG_EXTENSIONS,
};
use hex_core::Screen;
use hex_map::MapSettings;

pub(super) fn plugin(app: &mut App) {
    // `select_settings` rather than `load_settings`: there is no world file to load
    // until somebody has picked a scenario. It shares the registration `hex_map`
    // already did, which is idempotent, so plugin order does not matter here.
    app.select_settings::<MapSettings>(CONFIG_EXTENSIONS);
    // Lighting is chosen the same way, so a scenario brings its own sky and sun.
    // `hex_assets` no longer loads `lighting.ron` at startup -- two mechanisms writing
    // one resource is the collision `hex_map` already had.
    app.select_settings::<LightingSettings>(CONFIG_EXTENSIONS);
    app.add_systems(OnEnter(Screen::Loading), apply_selected_scenario);
}

/// The exact scenario whose button was clicked.
///
/// The library can hot-reload between the title-screen click and the next frame's
/// `OnEnter(Loading)`. Carrying the entry itself keeps a reorder or removal from
/// changing what that click means.
#[derive(Resource, Debug, Clone)]
pub(super) struct ScenarioToLoad(pub(super) Scenario);

/// Asks for the chosen scenario's world, and installs its unit placements.
fn apply_selected_scenario(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut registry: ResMut<SettingsRegistry>,
    pending: Option<Res<ScenarioToLoad>>,
    mut next: ResMut<NextState<Screen>>,
) {
    let Some(pending) = pending else {
        // A direct state transition (for example from the inspector) must not let the
        // loading gate reuse a previous scenario or enter gameplay without settings.
        // State changes requested from OnEnter are applied before the PostUpdate
        // readiness gate, so returning to title is sufficient and leaves the registry
        // truthful.
        next.set(Screen::Title);
        error!("loading entered without a clicked scenario; returning to the title screen");
        return;
    };
    let scenario = pending.0.clone();
    commands.remove_resource::<ScenarioToLoad>();

    info!("starting scenario: {}", scenario.name);
    commands.insert_resource(scenario.units.clone());
    choose_settings::<MapSettings>(&mut commands, &asset_server, &mut registry, &scenario.world);
    choose_settings::<LightingSettings>(
        &mut commands,
        &asset_server,
        &mut registry,
        &scenario.lighting,
    );
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
    use hex_assets::{CubeCoord, LightingSettings, ScenarioLibrary, SettingsRegistry};
    use hex_core::Screen;
    use hex_map::MapSettings;

    use super::ScenarioToLoad;

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

    /// Every lighting file a scenario names exists and parses.
    ///
    /// Same reasoning as the world check: the path is a plain string, so nothing else
    /// can catch a typo. The failure it prevents is a loading screen that hangs — and
    /// only for the one scenario nobody happened to start.
    #[test]
    fn every_scenario_names_lighting_that_exists_and_parses() {
        for scenario in &library().scenarios {
            let path = assets_dir().join(&scenario.lighting);
            let text = fs::read_to_string(&path).unwrap_or_else(|error| {
                panic!(
                    "scenario {:?} names lighting {:?}, which could not be read: {error}",
                    scenario.name, scenario.lighting
                )
            });
            let lighting: Result<LightingSettings, _> = ron::from_str(&text);
            assert!(
                lighting.is_ok(),
                "scenario {:?} names lighting that does not parse: {:?}",
                scenario.name,
                lighting.err()
            );
        }
    }

    /// Every shipped sun is actually above the horizon.
    ///
    /// **The reason this test exists.** `sun_rotation` looks like "height, compass,
    /// roll" and is not: it is an XYZ Euler triple that wraps past 2π, and the vertical
    /// component of the result depends on the first two numbers *together*. The first
    /// alternative lighting file changed both and put the sun 20° **below** the horizon.
    ///
    /// A directional light pointing at the sky lights nothing. The map renders as a
    /// black mass, no system errors, no log line — it is only visible by looking, and
    /// it shipped because nobody did.
    ///
    /// Computed with Bevy's own `Quat`, deliberately: hand-derived Euler maths is what
    /// caused the bug, so a hand-derived check would be the same mistake twice.
    #[test]
    fn every_shipped_sun_is_above_the_horizon() {
        for scenario in &library().scenarios {
            let text = fs::read_to_string(assets_dir().join(&scenario.lighting))
                .expect("the lighting file should exist");
            let lighting: LightingSettings =
                ron::from_str(&text).expect("the lighting should parse");

            let (x, y, z) = lighting.sun_rotation;
            // The direction the light *travels*, which is the transform's forward axis
            // — exactly what `sun_transform` builds in `hex_world::sky`.
            let heading = Quat::from_euler(EulerRot::XYZ, x, y, z) * Vec3::NEG_Z;
            let elevation = (-heading.y).asin().to_degrees();

            assert!(
                heading.y < 0.0,
                "scenario {:?}: {} puts the sun {:.1}° below the horizon, which lights \
                 nothing and renders a black map",
                scenario.name,
                scenario.lighting,
                -elevation
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

    /// The showcase starts with one unit at each end of its defining crossing.
    #[test]
    fn the_crossing_starts_units_at_opposite_bridge_landings() {
        let library = library();
        let crossing = library
            .scenarios
            .iter()
            .find(|scenario| scenario.name == "The Crossing")
            .expect("the shipped library should contain The Crossing");

        assert_eq!(crossing.units.player, CubeCoord { x: 0, y: 4, z: -4 });
        assert_eq!(crossing.units.enemy, CubeCoord { x: 0, y: -4, z: 4 });
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

    fn enter_gameplay_if_registry_is_ready(
        registry: Res<SettingsRegistry>,
        mut next: ResMut<NextState<Screen>>,
    ) {
        if registry.all_loaded() {
            next.set(Screen::Gameplay);
        }
    }

    /// Loading is only valid after a scenario button has supplied a snapshot.
    ///
    /// The loading gate runs in PostUpdate, after the return requested from OnEnter
    /// has already taken effect.
    #[test]
    fn loading_without_a_scenario_snapshot_returns_to_title() {
        let mut app = test_app();
        app.add_systems(
            PostUpdate,
            enter_gameplay_if_registry_is_ready.run_if(in_state(Screen::Loading)),
        );

        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Loading);
        app.update();

        assert_eq!(
            *app.world().resource::<State<Screen>>().get(),
            Screen::Title
        );
        assert!(
            app.world().resource::<SettingsRegistry>().all_loaded(),
            "returning to title left the settings registry falsely pending"
        );
        assert!(
            !app.world()
                .contains_resource::<hex_assets::ScenarioSettings>(),
            "loading without a click reused stale scenario placements"
        );

        assert_ne!(
            *app.world().resource::<State<Screen>>().get(),
            Screen::Gameplay,
            "the loading gate reused a previous world without a scenario click"
        );
    }

    /// Runs frames until the world for the chosen scenario has been installed.
    ///
    /// Bounded, and it fails naming what it was still waiting for. An unbounded loop
    /// here turns a regression into a CI job that hangs for its whole timeout with
    /// nothing to read.
    fn settle(app: &mut App) -> (MapSettings, LightingSettings) {
        for _ in 0..600 {
            app.update();
            if app.world().resource::<SettingsRegistry>().all_loaded() {
                let world = app.world().get_resource::<MapSettings>().cloned();
                let lighting = app.world().get_resource::<LightingSettings>().cloned();
                if let (Some(world), Some(lighting)) = (world, lighting) {
                    return (world, lighting);
                }
            }
        }
        panic!(
            "the scenario never arrived; still waiting on {:?}",
            app.world().resource::<SettingsRegistry>().pending_names()
        );
    }

    fn choose(app: &mut App, index: usize) {
        let scenario = app
            .world()
            .resource::<ScenarioLibrary>()
            .scenarios
            .get(index)
            .cloned()
            .expect("the requested scenario should exist");
        app.insert_resource(ScenarioToLoad(scenario));
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Loading);
        app.update();
    }

    /// Choosing a scenario installs *its* world, *its* lighting and *its* placements.
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
        let (first, first_light) = settle(&mut app);
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
        let (second, second_light) = settle(&mut app);
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
            first_light, second_light,
            "both scenarios produced the same lighting; the sky does not follow the scenario"
        );
        assert_ne!(
            (first_units.enemy.x, first_units.enemy.y),
            (second_units.enemy.x, second_units.enemy.y),
            "both scenarios produced the same placements"
        );
    }
}
