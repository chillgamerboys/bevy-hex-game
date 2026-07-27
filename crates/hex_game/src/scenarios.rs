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
    choose_settings, LightingSettings, Scenario, ScenarioPlacement, ScenarioSettings,
    SelectSettings, SettingsRegistry, CONFIG_EXTENSIONS,
};
use hex_core::{
    GameplaySetup, GameplaySetupFailure, InteriorRegions, MapAnchors, MapViewHint, ResolvedMapSeed,
    Screen, SpecialMovementRegions, TerrainReady,
};
use hex_map::{MapSettings, TerrainSettings};
use hex_units::{Enemy, Player};
use hex_world::TimeOfDay;

pub(super) fn plugin(app: &mut App) {
    // `select_settings` rather than `load_settings`: there is no world file to load
    // until somebody has picked a scenario. It shares the registration `hex_map`
    // already did, which is idempotent, so plugin order does not matter here.
    app.register_type::<ResolvedMapSeed>()
        .register_type::<GameplaySetupFailure>()
        .select_settings::<MapSettings>(CONFIG_EXTENSIONS);
    // Lighting is chosen the same way, so a scenario brings its own sky and sun.
    // `hex_assets` no longer loads `lighting.ron` at startup -- two mechanisms writing
    // one resource is the collision `hex_map` already had.
    app.select_settings::<LightingSettings>(CONFIG_EXTENSIONS);
    app.add_systems(OnEnter(Screen::Loading), apply_selected_scenario)
        .add_systems(
            OnEnter(Screen::Gameplay),
            (
                initialize_time_of_day.in_set(GameplaySetup::Resources),
                finalize_gameplay_setup.in_set(GameplaySetup::Finalize),
            ),
        )
        .add_systems(
            Update,
            validate_gameplay_lighting_contract.run_if(in_state(Screen::Gameplay)),
        )
        .add_systems(OnExit(Screen::Gameplay), clear_session_resources);
}

/// The exact scenario and resolved seed whose button was clicked.
///
/// The library can hot-reload between the title-screen click and the next frame's
/// `OnEnter(Loading)`. Carrying the entry itself keeps a reorder or removal from
/// changing what that click means; carrying the seed prevents a later reroll from
/// changing an already-started load.
#[derive(Resource, Debug, Clone)]
pub(super) struct ScenarioToLoad {
    pub(super) scenario: Scenario,
    pub(super) resolved_seed: Option<ResolvedMapSeed>,
}

/// The selected scenario's authored hour, snapshotted before its lighting loads.
///
/// Keeping this separate from [`TimeOfDay`] lets the loading contract distinguish an
/// absent override (use the cycle default) from an explicit hour. The latter is invalid
/// for a static lighting profile.
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub(crate) struct ScenarioTimeOverride(Option<f32>);

/// Result of checking the selected scenario against its loaded world.
///
/// The loading gate accepts only `Ready`. Keeping `Invalid` distinct prevents a bad
/// hot reload from logging every frame while the state transition returns to title.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ScenarioContractStatus {
    /// Seed and placements agree with the loaded terrain preset.
    Ready,
    /// The scenario and world cannot safely enter gameplay together.
    Invalid,
}

/// Asks for the chosen scenario's world, and installs its unit placements.
fn apply_selected_scenario(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut registry: ResMut<SettingsRegistry>,
    pending: Option<Res<ScenarioToLoad>>,
    mut next: ResMut<NextState<Screen>>,
) {
    // These resources describe the previous generated world. Clearing them before the
    // new settings request prevents a failed generation from reusing old anchors or a
    // stale readiness marker.
    commands.remove_resource::<MapAnchors>();
    commands.remove_resource::<SpecialMovementRegions>();
    commands.remove_resource::<InteriorRegions>();
    commands.remove_resource::<MapViewHint>();
    commands.remove_resource::<TerrainReady>();
    commands.remove_resource::<ResolvedMapSeed>();
    commands.remove_resource::<TimeOfDay>();
    commands.remove_resource::<ScenarioTimeOverride>();
    commands.remove_resource::<GameplaySetupFailure>();
    commands.remove_resource::<ScenarioContractStatus>();

    let Some(pending) = pending else {
        // A direct state transition (for example from the inspector) must not let the
        // loading gate reuse a previous scenario or enter gameplay without settings.
        // State changes requested from OnEnter are applied before the PostUpdate
        // readiness gate, so returning to title is sufficient and leaves the registry
        // truthful.
        commands.remove_resource::<ScenarioSettings>();
        commands.insert_resource(GameplaySetupFailure::new(
            "Loading started without a selected scenario.",
        ));
        next.set(Screen::Title);
        error!("loading entered without a clicked scenario; returning to the title screen");
        return;
    };
    let scenario = pending.scenario.clone();
    let resolved_seed = pending.resolved_seed;
    commands.remove_resource::<ScenarioToLoad>();

    if let Some(seed) = resolved_seed {
        info!("starting scenario: {} (seed {})", scenario.name, seed.0);
        commands.insert_resource(seed);
    } else {
        info!("starting scenario: {}", scenario.name);
    }
    commands.insert_resource(scenario.units.clone());
    commands.insert_resource(ScenarioTimeOverride(scenario.starting_time_hours));
    choose_settings::<MapSettings>(&mut commands, &asset_server, &mut registry, &scenario.world);
    choose_settings::<LightingSettings>(
        &mut commands,
        &asset_server,
        &mut registry,
        &scenario.lighting,
    );
}

/// Validates facts which live in separate RON files once both have arrived.
///
/// `hex_assets` cannot inspect `MapSettings` without inverting the crate graph, so
/// deserializing either file alone cannot catch a procedural world paired with
/// authored placements or a missing seed. The binary is the first layer allowed to
/// see both.
pub(crate) fn validate_loaded_scenario(
    mut commands: Commands,
    registry: Res<SettingsRegistry>,
    map: Option<Res<MapSettings>>,
    scenario: Option<Res<ScenarioSettings>>,
    lighting: Option<Res<LightingSettings>>,
    time_override: Option<Res<ScenarioTimeOverride>>,
    seed: Option<Res<ResolvedMapSeed>>,
    status: Option<Res<ScenarioContractStatus>>,
    mut next: ResMut<NextState<Screen>>,
) {
    if !registry.all_loaded()
        || status
            .as_deref()
            .is_some_and(|status| *status == ScenarioContractStatus::Invalid)
    {
        return;
    }
    let (Some(map), Some(scenario), Some(lighting), Some(time_override)) =
        (map, scenario, lighting, time_override)
    else {
        return;
    };
    let inputs_changed = map.is_changed()
        || scenario.is_changed()
        || lighting.is_changed()
        || time_override.is_changed()
        || seed.as_ref().is_some_and(|seed| seed.is_changed());
    if status.is_some() && !inputs_changed {
        return;
    }

    let contract_error = scenario_contract_error(&map, &scenario, seed.as_deref())
        .or_else(|| lighting.resolve(time_override.0).err());
    if let Some(reason) = contract_error {
        error!("selected scenario is incompatible with its world: {reason}");
        commands.insert_resource(ScenarioContractStatus::Invalid);
        commands.insert_resource(GameplaySetupFailure::new(format!(
            "The selected scenario is incompatible with its world: {reason}."
        )));
        next.set(Screen::Title);
    } else {
        commands.insert_resource(ScenarioContractStatus::Ready);
    }
}

/// Resolves the scenario/profile default before any presentation system needs it.
fn initialize_time_of_day(
    mut commands: Commands,
    lighting: Res<LightingSettings>,
    time_override: Res<ScenarioTimeOverride>,
    mut next: ResMut<NextState<Screen>>,
) {
    match lighting.resolve(time_override.0) {
        Ok(resolved) => match resolved.time_hours {
            Some(hours) => {
                commands.insert_resource(TimeOfDay { hours });
            }
            None => {
                commands.remove_resource::<TimeOfDay>();
            }
        },
        Err(reason) => {
            error!("could not initialize scenario time of day: {reason}");
            commands.insert_resource(GameplaySetupFailure::new(format!(
                "The selected scenario cannot initialize its lighting: {reason}."
            )));
            next.set(Screen::Title);
        }
    }
}

/// Rechecks the cross-asset time contract when lighting hot-reloads in gameplay.
///
/// A static lighting file is valid by itself, but cannot replace a cycle while the
/// active scenario owns an explicit hour. Returning to title preserves the authored
/// scenario contract instead of silently discarding its time.
fn validate_gameplay_lighting_contract(
    mut commands: Commands,
    lighting: Res<LightingSettings>,
    time_override: Res<ScenarioTimeOverride>,
    mut next: ResMut<NextState<Screen>>,
) {
    if !lighting.is_changed() {
        return;
    }
    let Err(reason) = lighting.resolve(time_override.0) else {
        return;
    };

    error!("active scenario is incompatible with reloaded lighting: {reason}");
    commands.insert_resource(GameplaySetupFailure::new(format!(
        "The active scenario is incompatible with reloaded lighting: {reason}."
    )));
    next.set(Screen::Title);
}

fn scenario_contract_error(
    map: &MapSettings,
    scenario: &ScenarioSettings,
    seed: Option<&ResolvedMapSeed>,
) -> Option<String> {
    match &map.terrain {
        TerrainSettings::Procedural(_) => {
            if seed.is_none() {
                return Some("procedural terrain has no resolved generation seed".to_owned());
            }
            for (who, placement) in [("player", &scenario.player), ("enemy", &scenario.enemy)] {
                if !matches!(placement, ScenarioPlacement::Anchor(_)) {
                    return Some(format!("procedural {who} placement must use a map anchor"));
                }
            }
        }
        TerrainSettings::Showcase(_) | TerrainSettings::Perlin(_) => {
            if seed.is_some() {
                return Some(
                    "authored terrain must not receive a scenario generation seed".to_owned(),
                );
            }
            if !matches!(scenario.player, ScenarioPlacement::Fixed(_))
                || !matches!(scenario.enemy, ScenarioPlacement::Fixed(_))
            {
                return Some("authored terrain requires Fixed unit placements".to_owned());
            }
        }
    }
    None
}

/// Completes cross-crate gameplay construction or returns visibly to the title.
///
/// Map and unit systems publish the detailed reason when they can. The structural
/// checks here are defense in depth for a future setup system that forgets to do so.
fn finalize_gameplay_setup(
    mut commands: Commands,
    failure: Option<Res<GameplaySetupFailure>>,
    terrain_ready: Option<Res<TerrainReady>>,
    players: Query<(), With<Player>>,
    enemies: Query<(), With<Enemy>>,
    mut next: ResMut<NextState<Screen>>,
) {
    let reason = failure
        .as_deref()
        .map(|failure| failure.reason.clone())
        .or_else(|| {
            terrain_ready
                .is_none()
                .then(|| "The selected scenario could not build valid terrain.".to_owned())
        })
        .or_else(|| {
            (players.iter().count() != 1)
                .then(|| "The selected scenario did not create exactly one player.".to_owned())
        })
        .or_else(|| {
            (enemies.iter().count() != 1)
                .then(|| "The selected scenario did not create exactly one enemy.".to_owned())
        });

    let Some(reason) = reason else { return };
    if failure.is_none() {
        commands.insert_resource(GameplaySetupFailure::new(reason.clone()));
    }
    error!("gameplay setup failed: {reason}");
    next.set(Screen::Title);
}

fn clear_session_resources(mut commands: Commands) {
    commands.remove_resource::<ResolvedMapSeed>();
    commands.remove_resource::<ScenarioTimeOverride>();
    commands.remove_resource::<TimeOfDay>();
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
    use hex_assets::{
        CubeCoord, GameAssets, LightingSettings, PlayerSettings, ScenarioLibrary,
        ScenarioPlacement, ScenarioSettings, SettingsRegistry, SubstanceFile, SubstanceTable,
    };
    use hex_core::{
        GameplaySetup, GameplaySetupFailure, HexGrid, InteriorRegions, MapAnchorId, MapAnchors,
        MapViewHint, Mode, Pause, ResolvedMapSeed, Screen, SpecialMovementRegions, TerrainReady,
    };
    use hex_map::{GenerationReport, MapSettings, TerrainSettings, VoxelMap};
    use hex_units::{Enemy, Player, StandsOn};
    use hex_world::TimeOfDay;

    use super::{
        clear_session_resources, finalize_gameplay_setup, initialize_time_of_day,
        scenario_contract_error, validate_gameplay_lighting_contract, validate_loaded_scenario,
        ScenarioTimeOverride, ScenarioToLoad,
    };

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

    #[test]
    fn shipped_piece_is_visually_just_under_two_voxel_levels_tall() {
        // Combined Y bounds of the two king primitives loaded from pieces.glb.
        const PLAYER_MESH_HEIGHT: f32 = 10.039_005 - 0.958_011;

        let player_text = fs::read_to_string(assets_dir().join("config/player.ron"))
            .expect("player settings should be readable");
        let player: PlayerSettings =
            ron::from_str(&player_text).expect("player settings should deserialize");
        let hero = library()
            .scenarios
            .into_iter()
            .find(|scenario| scenario.name == "Procedural Hills")
            .expect("the shipped library should contain the hero scenario");
        let world_text = fs::read_to_string(assets_dir().join(hero.world))
            .expect("hero world should be readable");
        let world: MapSettings = ron::from_str(&world_text).expect("hero world should deserialize");
        let rendered_levels = player.scale * PLAYER_MESH_HEIGHT / world.level_height;

        assert!(
            (1.75..2.0).contains(&rendered_levels),
            "the player renders at {rendered_levels:.3} voxel levels; expected just under two"
        );
    }

    /// Scenario and world files are independently valid assets, but the pair also has
    /// a contract: procedural worlds need generated anchors and a seed; authored
    /// worlds need fixed coordinates and no scenario seed.
    #[test]
    fn every_shipped_scenario_matches_its_world_kind() {
        for scenario in &library().scenarios {
            let path = assets_dir().join(&scenario.world);
            let text = fs::read_to_string(&path).expect("the shipped world should be readable");
            let world: MapSettings =
                ron::from_str(&text).expect("the shipped world should deserialize");
            let seed = scenario.generation_seed.map(ResolvedMapSeed);
            assert_eq!(
                scenario_contract_error(&world, &scenario.units, seed.as_ref()),
                None,
                "scenario {:?} does not match {:?}",
                scenario.name,
                world.terrain
            );
            let lighting_text = fs::read_to_string(assets_dir().join(&scenario.lighting))
                .expect("the shipped lighting should be readable");
            let lighting: LightingSettings =
                ron::from_str(&lighting_text).expect("the shipped lighting should deserialize");
            assert!(
                lighting.resolve(scenario.starting_time_hours).is_ok(),
                "scenario {:?} requests an hour its lighting profile cannot resolve",
                scenario.name
            );
        }
    }

    #[test]
    fn static_lighting_rejects_a_scenario_time_override() {
        let overcast = library()
            .scenarios
            .into_iter()
            .find(|scenario| scenario.lighting.ends_with("overcast.ron"))
            .expect("the shipped library should contain a static overcast scenario");
        let text = fs::read_to_string(assets_dir().join(overcast.lighting))
            .expect("the overcast lighting should be readable");
        let lighting: LightingSettings =
            ron::from_str(&text).expect("the overcast lighting should deserialize");

        assert!(lighting
            .resolve(Some(18.0))
            .is_err_and(|error| error.contains("static lighting")));
        assert!(lighting.resolve(None).is_ok());
    }

    #[test]
    fn loaded_contract_reports_a_static_time_override_as_setup_failure() {
        let entry = library()
            .scenarios
            .into_iter()
            .find(|scenario| scenario.lighting.ends_with("overcast.ron"))
            .expect("the shipped library should contain a static overcast scenario");
        let world_text = fs::read_to_string(assets_dir().join(&entry.world))
            .expect("the static scenario world should be readable");
        let lighting_text = fs::read_to_string(assets_dir().join(&entry.lighting))
            .expect("the static scenario lighting should be readable");

        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin));
        app.init_state::<Screen>();
        app.insert_resource(SettingsRegistry::default());
        app.insert_resource(
            ron::from_str::<MapSettings>(&world_text)
                .expect("the static scenario world should deserialize"),
        );
        app.insert_resource(entry.units);
        app.insert_resource(
            ron::from_str::<LightingSettings>(&lighting_text)
                .expect("the static scenario lighting should deserialize"),
        );
        app.insert_resource(ScenarioTimeOverride(Some(18.0)));
        app.add_systems(Update, validate_loaded_scenario);

        app.update();
        app.update();

        assert!(app
            .world()
            .resource::<GameplaySetupFailure>()
            .reason
            .contains("static lighting"));
        assert_eq!(
            *app.world().resource::<State<Screen>>().get(),
            Screen::Title
        );
    }

    #[test]
    fn gameplay_hot_reload_rejects_static_lighting_with_an_authored_time() {
        let scenarios = library().scenarios;
        let clear = scenarios
            .iter()
            .find(|scenario| scenario.lighting.ends_with("lighting.ron"))
            .expect("the shipped library should contain clear lighting");
        let overcast = scenarios
            .iter()
            .find(|scenario| scenario.lighting.ends_with("overcast.ron"))
            .expect("the shipped library should contain static overcast lighting");
        let read_lighting = |path: &str| {
            let text = fs::read_to_string(assets_dir().join(path))
                .expect("the shipped lighting should be readable");
            ron::from_str::<LightingSettings>(&text)
                .expect("the shipped lighting should deserialize")
        };

        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin));
        app.init_state::<Screen>();
        app.insert_resource(read_lighting(&clear.lighting));
        app.insert_resource(ScenarioTimeOverride(Some(18.5)));
        app.add_systems(
            Update,
            validate_gameplay_lighting_contract.run_if(in_state(Screen::Gameplay)),
        );

        enter_gameplay_and_settle(&mut app);
        app.insert_resource(read_lighting(&overcast.lighting));
        app.update();
        app.update();

        assert_eq!(
            *app.world().resource::<State<Screen>>().get(),
            Screen::Title
        );
        assert!(app
            .world()
            .resource::<GameplaySetupFailure>()
            .reason
            .contains("static lighting"));
    }

    #[test]
    fn procedural_contract_rejects_missing_seed_and_authored_placements() {
        let entry = library()
            .scenarios
            .into_iter()
            .find(|scenario| scenario.generation_seed.is_some())
            .expect("the shipped library should include procedural terrain");
        let path = assets_dir().join(&entry.world);
        let text = fs::read_to_string(path).expect("the procedural world should be readable");
        let world: MapSettings =
            ron::from_str(&text).expect("the procedural world should deserialize");
        assert!(matches!(world.terrain, TerrainSettings::Procedural(_)));

        assert!(scenario_contract_error(&world, &entry.units, None)
            .is_some_and(|error| error.contains("no resolved")));

        let fixed = ScenarioSettings {
            player: ScenarioPlacement::Fixed(CubeCoord { x: 0, y: 0, z: 0 }),
            enemy: ScenarioPlacement::Fixed(CubeCoord { x: 1, y: -1, z: 0 }),
        };
        assert!(
            scenario_contract_error(&world, &fixed, Some(&ResolvedMapSeed(1)))
                .is_some_and(|error| error.contains("map anchor"))
        );
    }

    #[test]
    fn procedural_contract_allows_recipe_specific_anchor_names() {
        let entry = library()
            .scenarios
            .into_iter()
            .find(|scenario| scenario.generation_seed.is_some())
            .expect("the shipped library should include procedural terrain");
        let path = assets_dir().join(&entry.world);
        let text = fs::read_to_string(path).expect("the procedural world should be readable");
        let world: MapSettings =
            ron::from_str(&text).expect("the procedural world should deserialize");
        let placements = ScenarioSettings {
            player: ScenarioPlacement::Anchor("surface_entrance".to_owned()),
            enemy: ScenarioPlacement::Anchor("deep_chamber".to_owned()),
        };

        assert_eq!(
            scenario_contract_error(&world, &placements, Some(&ResolvedMapSeed(1))),
            None
        );
    }

    #[test]
    fn invalid_loaded_contract_publishes_a_visible_failure_reason() {
        let entry = library()
            .scenarios
            .into_iter()
            .find(|scenario| scenario.generation_seed.is_some())
            .expect("the shipped library should include procedural terrain");
        let world_text = fs::read_to_string(assets_dir().join(&entry.world))
            .expect("the procedural world should be readable");
        let world: MapSettings =
            ron::from_str(&world_text).expect("the procedural world should deserialize");
        let fixed = ScenarioSettings {
            player: ScenarioPlacement::Fixed(CubeCoord { x: 0, y: 0, z: 0 }),
            enemy: ScenarioPlacement::Fixed(CubeCoord { x: 1, y: -1, z: 0 }),
        };
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin));
        app.init_state::<Screen>();
        app.insert_resource(SettingsRegistry::default());
        app.insert_resource(world);
        app.insert_resource(fixed);
        let lighting_text = fs::read_to_string(assets_dir().join(&entry.lighting))
            .expect("the procedural lighting should be readable");
        app.insert_resource(
            ron::from_str::<LightingSettings>(&lighting_text)
                .expect("the procedural lighting should deserialize"),
        );
        app.insert_resource(ScenarioTimeOverride(entry.starting_time_hours));
        app.insert_resource(ResolvedMapSeed(1));
        app.add_systems(Update, validate_loaded_scenario);

        app.update();
        app.update();

        assert_eq!(
            *app.world().resource::<State<Screen>>().get(),
            Screen::Title
        );
        assert!(app
            .world()
            .resource::<GameplaySetupFailure>()
            .reason
            .contains("must use a map anchor"));
    }

    #[test]
    fn cyclic_time_initializes_from_override_and_resets_on_reentry() {
        let scenario = library()
            .scenarios
            .into_iter()
            .find(|scenario| {
                let Ok(text) = fs::read_to_string(assets_dir().join(&scenario.lighting)) else {
                    return false;
                };
                ron::from_str::<LightingSettings>(&text)
                    .ok()
                    .is_some_and(|lighting| lighting.default_time_hours().is_some())
            })
            .expect("the shipped library should contain cyclic lighting");
        let text = fs::read_to_string(assets_dir().join(scenario.lighting))
            .expect("the cyclic lighting should be readable");
        let lighting: LightingSettings =
            ron::from_str(&text).expect("the cyclic lighting should deserialize");

        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin));
        app.init_state::<Screen>();
        app.configure_sets(
            OnEnter(Screen::Gameplay),
            (
                GameplaySetup::Resources,
                GameplaySetup::Terrain,
                GameplaySetup::Actors,
                GameplaySetup::View,
                GameplaySetup::Finalize,
            )
                .chain(),
        );
        app.insert_resource(lighting);
        app.insert_resource(ScenarioTimeOverride(Some(18.5)));
        app.add_systems(
            OnEnter(Screen::Gameplay),
            initialize_time_of_day.in_set(GameplaySetup::Resources),
        );
        app.add_systems(OnExit(Screen::Gameplay), clear_session_resources);

        enter_gameplay_and_settle(&mut app);
        assert!(
            (app.world().resource::<TimeOfDay>().hours - 18.5).abs() < f32::EPSILON,
            "the scenario override did not win the profile default"
        );
        app.world_mut().resource_mut::<TimeOfDay>().hours = 3.0;

        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Title);
        app.update();
        app.update();
        assert!(
            !app.world().contains_resource::<TimeOfDay>(),
            "gameplay exit leaked the inspector-edited session hour"
        );

        app.insert_resource(ScenarioTimeOverride(Some(18.5)));
        enter_gameplay_and_settle(&mut app);
        assert!(
            (app.world().resource::<TimeOfDay>().hours - 18.5).abs() < f32::EPSILON,
            "gameplay re-entry did not restore the selected scenario hour"
        );
    }

    #[test]
    fn cyclic_time_uses_the_profile_default_without_an_override() {
        let scenario = library()
            .scenarios
            .into_iter()
            .find(|scenario| scenario.lighting.ends_with("lighting.ron"))
            .expect("the shipped library should contain clear lighting");
        let text = fs::read_to_string(assets_dir().join(scenario.lighting))
            .expect("the clear lighting should be readable");
        let lighting: LightingSettings =
            ron::from_str(&text).expect("the clear lighting should deserialize");
        let expected = lighting
            .default_time_hours()
            .expect("the clear lighting should use a cycle");

        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin));
        app.init_state::<Screen>();
        app.configure_sets(
            OnEnter(Screen::Gameplay),
            (
                GameplaySetup::Resources,
                GameplaySetup::Terrain,
                GameplaySetup::Actors,
                GameplaySetup::View,
                GameplaySetup::Finalize,
            )
                .chain(),
        );
        app.insert_resource(lighting);
        app.insert_resource(ScenarioTimeOverride(None));
        app.add_systems(
            OnEnter(Screen::Gameplay),
            initialize_time_of_day.in_set(GameplaySetup::Resources),
        );

        enter_gameplay_and_settle(&mut app);

        assert!((app.world().resource::<TimeOfDay>().hours - expected).abs() < f32::EPSILON);
    }

    fn finalizer_app(
        terrain_ready: bool,
        player_count: usize,
        enemy_count: usize,
        failure: Option<GameplaySetupFailure>,
    ) -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin));
        app.init_state::<Screen>();
        app.configure_sets(
            OnEnter(Screen::Gameplay),
            (
                GameplaySetup::Resources,
                GameplaySetup::Terrain,
                GameplaySetup::Actors,
                GameplaySetup::View,
                GameplaySetup::Finalize,
            )
                .chain(),
        );
        app.add_systems(
            OnEnter(Screen::Gameplay),
            finalize_gameplay_setup.in_set(GameplaySetup::Finalize),
        );
        if terrain_ready {
            app.insert_resource(TerrainReady);
        }
        if let Some(failure) = failure {
            app.insert_resource(failure);
        }
        for _ in 0..player_count {
            app.world_mut().spawn(Player);
        }
        for _ in 0..enemy_count {
            app.world_mut().spawn(Enemy);
        }
        app
    }

    fn enter_gameplay_and_settle(app: &mut App) {
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Gameplay);
        app.update();
        app.update();
    }

    #[test]
    fn finalizer_returns_to_title_when_a_required_actor_is_missing() {
        let mut app = finalizer_app(true, 0, 1, None);

        enter_gameplay_and_settle(&mut app);

        assert_eq!(
            *app.world().resource::<State<Screen>>().get(),
            Screen::Title
        );
        assert!(app
            .world()
            .resource::<GameplaySetupFailure>()
            .reason
            .contains("exactly one player"));
    }

    #[test]
    fn finalizer_preserves_a_detailed_setup_failure() {
        let expected = "The generated party anchor has no standable surface.";
        let mut app = finalizer_app(true, 1, 1, Some(GameplaySetupFailure::new(expected)));

        enter_gameplay_and_settle(&mut app);

        assert_eq!(
            *app.world().resource::<State<Screen>>().get(),
            Screen::Title
        );
        assert_eq!(
            app.world().resource::<GameplaySetupFailure>().reason,
            expected
        );
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

            for (who, placement) in [
                ("player", &scenario.units.player),
                ("enemy", &scenario.units.enemy),
            ] {
                match placement {
                    ScenarioPlacement::Fixed(coord) => {
                        assert_eq!(
                            coord.x + coord.y + coord.z,
                            0,
                            "scenario {:?}: the {who}'s coordinates do not sum to zero",
                            scenario.name
                        );
                        assert!(
                            distance_from_centre(*coord) <= world.grid_radius,
                            "scenario {:?}: the {who} starts {} hexes out on a map of radius {}",
                            scenario.name,
                            distance_from_centre(*coord),
                            world.grid_radius
                        );
                    }
                    ScenarioPlacement::Anchor(anchor) => {
                        assert!(
                            !anchor.is_empty(),
                            "scenario {:?}: the {who} has an empty generated anchor",
                            scenario.name
                        );
                        assert!(
                            scenario.generation_seed.is_some(),
                            "scenario {:?}: the {who} uses a generated anchor without a seed",
                            scenario.name
                        );
                    }
                }
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

        assert_eq!(
            crossing.units.player,
            ScenarioPlacement::Fixed(CubeCoord { x: 0, y: 4, z: -4 })
        );
        assert_eq!(
            crossing.units.enemy,
            ScenarioPlacement::Fixed(CubeCoord { x: 0, y: -4, z: 4 })
        );
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
        let stale_units = library()
            .scenarios
            .first()
            .map(|scenario| scenario.units.clone())
            .expect("the shipped library should not be empty");
        app.insert_resource(stale_units);
        app.insert_resource(ResolvedMapSeed(99));
        app.insert_resource(TimeOfDay { hours: 3.0 });
        app.insert_resource(SpecialMovementRegions::new());
        app.insert_resource(InteriorRegions::new());
        app.insert_resource(MapViewHint::new((1.0, 2.0, 3.0), (0.0, 0.0, 0.0)));

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
        assert!(
            !app.world().contains_resource::<ResolvedMapSeed>(),
            "loading without a click reused a stale procedural seed"
        );
        assert!(
            !app.world().contains_resource::<TimeOfDay>(),
            "loading without a click reused a stale session hour"
        );
        assert!(
            !app.world().contains_resource::<SpecialMovementRegions>(),
            "loading without a click reused stale generated-region semantics"
        );
        assert!(
            !app.world().contains_resource::<InteriorRegions>(),
            "loading without a click reused stale interior semantics"
        );
        assert!(
            !app.world().contains_resource::<MapViewHint>(),
            "loading without a click reused stale generated framing"
        );
        assert!(app
            .world()
            .resource::<GameplaySetupFailure>()
            .reason
            .contains("without a selected scenario"));

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
        let resolved_seed = scenario.generation_seed.map(ResolvedMapSeed);
        app.insert_resource(ScenarioToLoad {
            scenario,
            resolved_seed,
        });
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
        let mut authored = entries
            .iter()
            .enumerate()
            .filter_map(|(index, scenario)| scenario.generation_seed.is_none().then_some(index));
        let first_index = authored
            .next()
            .expect("this test needs two authored scenarios to compare");
        let second_index = authored
            .next()
            .expect("this test needs two authored scenarios to compare");

        let mut app = test_app();

        choose(&mut app, first_index);
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

        choose(&mut app, second_index);
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
            first_units.enemy, second_units.enemy,
            "both scenarios produced the same placements"
        );
    }

    /// The seed captured by the title-screen click is installed for map generation.
    #[test]
    fn selected_generation_seed_is_installed_while_loading() {
        let procedural_index = library()
            .scenarios
            .iter()
            .position(|scenario| scenario.generation_seed.is_some())
            .expect("the shipped library should contain a generated scenario");
        let mut app = test_app();

        choose(&mut app, procedural_index);

        let configured = library()
            .scenarios
            .get(procedural_index)
            .and_then(|scenario| scenario.generation_seed)
            .expect("the procedural scenario should have a seed");
        assert_eq!(
            app.world().get_resource::<ResolvedMapSeed>(),
            Some(&ResolvedMapSeed(configured))
        );
    }

    /// Selecting an authored map after a generated one cannot leak its old seed.
    #[test]
    fn authored_scenario_clears_a_previous_generation_seed() {
        let entries = library().scenarios;
        let procedural = entries
            .iter()
            .position(|scenario| scenario.generation_seed.is_some())
            .expect("the shipped library should contain a generated scenario");
        let authored = entries
            .iter()
            .position(|scenario| scenario.generation_seed.is_none())
            .expect("the shipped library should contain an authored scenario");
        let mut app = test_app();

        choose(&mut app, procedural);
        assert!(app.world().contains_resource::<ResolvedMapSeed>());
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Title);
        app.update();

        choose(&mut app, authored);
        assert!(
            !app.world().contains_resource::<ResolvedMapSeed>(),
            "the authored map inherited the previous generated map's seed"
        );
    }

    fn procedural_gameplay_app(scenario_name: &str) -> App {
        let entry = library()
            .scenarios
            .into_iter()
            .find(|scenario| scenario.name == scenario_name)
            .unwrap_or_else(|| panic!("the shipped library should contain {scenario_name}"));
        let world_text = fs::read_to_string(assets_dir().join(&entry.world))
            .expect("the hero world should be readable");
        let world: MapSettings =
            ron::from_str(&world_text).expect("the hero world should deserialize");
        let substances: SubstanceFile =
            ron::from_str(include_str!("../../../assets/config/substances.ron"))
                .expect("the shipped substances should deserialize");
        let player: PlayerSettings =
            ron::from_str(include_str!("../../../assets/config/player.ron"))
                .expect("the shipped player settings should deserialize");
        let seed = entry
            .generation_seed
            .map(ResolvedMapSeed)
            .expect("the hero scenario should have a seed");

        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default(), StatesPlugin));
        app.init_asset::<Mesh>();
        app.init_asset::<StandardMaterial>();
        app.init_state::<Screen>();
        app.add_sub_state::<Mode>();
        app.add_sub_state::<Pause>();
        app.configure_sets(
            OnEnter(Screen::Gameplay),
            (
                GameplaySetup::Resources,
                GameplaySetup::Terrain,
                GameplaySetup::Actors,
                GameplaySetup::View,
                GameplaySetup::Finalize,
            )
                .chain(),
        );
        app.insert_resource(GameAssets {
            hex_tile: Handle::default(),
            player_pieces: [Handle::default(), Handle::default()],
        });
        app.insert_resource(SubstanceTable::from_file(&substances));
        app.insert_resource(player);
        app.insert_resource(entry.units);
        app.insert_resource(world);
        app.insert_resource(seed);
        app.add_plugins((hex_map::plugin, hex_units::movement::plugin));
        hex_units::units::plugin(&mut app);
        app.add_systems(
            OnEnter(Screen::Gameplay),
            finalize_gameplay_setup.in_set(GameplaySetup::Finalize),
        );

        while app.plugins_state() != PluginsState::Cleaned {
            app.finish();
            app.cleanup();
        }
        app
    }

    fn enter_screen(app: &mut App, screen: Screen) {
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(screen);
        app.update();
        app.update();
    }

    fn standing_pos<T: Component>(app: &mut App) -> Option<hex_core::TilePos> {
        let world = app.world_mut();
        let mut query = world.query_filtered::<&StandsOn, With<T>>();
        query.iter(world).next().map(|standing| standing.0.pos)
    }

    /// The real map and unit plugins agree on seed, exact anchor surfaces, teardown,
    /// and deterministic re-entry. Unit tests for each subsystem cannot catch a
    /// schedule or resource-contract regression between them.
    #[test]
    fn procedural_world_reenters_with_the_same_fingerprint_and_actor_anchors() {
        let mut app = procedural_gameplay_app("Procedural Hills");
        enter_screen(&mut app, Screen::Gameplay);

        assert!(app.world().contains_resource::<TerrainReady>());
        assert!(
            app.world().contains_resource::<SpecialMovementRegions>(),
            "a ready map should publish its optional-region registry"
        );
        let first_fingerprint = app.world().resource::<GenerationReport>().map_fingerprint;
        let first_party = app
            .world()
            .resource::<MapAnchors>()
            .get(&MapAnchorId::from("party_start"))
            .expect("the map should publish party_start");
        let first_hostile = app
            .world()
            .resource::<MapAnchors>()
            .get(&MapAnchorId::from("hostile_start"))
            .expect("the map should publish hostile_start");
        assert_eq!(standing_pos::<Player>(&mut app), Some(first_party));
        assert_eq!(standing_pos::<Enemy>(&mut app), Some(first_hostile));
        app.insert_resource(InteriorRegions::new());
        app.insert_resource(MapViewHint::new((1.0, 2.0, 3.0), (0.0, 0.0, 0.0)));

        enter_screen(&mut app, Screen::Title);
        assert!(!app.world().contains_resource::<VoxelMap>());
        assert!(!app.world().contains_resource::<MapAnchors>());
        assert!(!app.world().contains_resource::<GenerationReport>());
        assert!(!app.world().contains_resource::<SpecialMovementRegions>());
        assert!(!app.world().contains_resource::<InteriorRegions>());
        assert!(!app.world().contains_resource::<MapViewHint>());
        assert!(!app.world().contains_resource::<TerrainReady>());
        assert_eq!(
            app.world_mut()
                .query_filtered::<Entity, With<HexGrid>>()
                .iter(app.world())
                .count(),
            0
        );
        assert!(standing_pos::<Player>(&mut app).is_none());
        assert!(standing_pos::<Enemy>(&mut app).is_none());

        enter_screen(&mut app, Screen::Gameplay);
        let second_fingerprint = app.world().resource::<GenerationReport>().map_fingerprint;
        assert_eq!(second_fingerprint, first_fingerprint);
        assert!(app.world().contains_resource::<SpecialMovementRegions>());
        assert_eq!(standing_pos::<Player>(&mut app), Some(first_party));
        assert_eq!(standing_pos::<Enemy>(&mut app), Some(first_hostile));
        assert_eq!(
            app.world_mut()
                .query_filtered::<Entity, With<HexGrid>>()
                .iter(app.world())
                .count(),
            1,
            "re-entry duplicated the rendered grid"
        );
    }

    #[test]
    fn missing_generated_enemy_anchor_fails_setup_and_cleans_partial_world() {
        let mut app = procedural_gameplay_app("Procedural Hills");
        app.world_mut().resource_mut::<ScenarioSettings>().enemy =
            ScenarioPlacement::Anchor("missing_enemy_anchor".to_owned());

        enter_screen(&mut app, Screen::Gameplay);

        assert_eq!(
            *app.world().resource::<State<Screen>>().get(),
            Screen::Title
        );
        assert!(app
            .world()
            .resource::<GameplaySetupFailure>()
            .reason
            .contains("missing map anchor"));
        assert!(
            !app.world().contains_resource::<VoxelMap>(),
            "failed setup left generated terrain alive on the title screen"
        );
        assert!(standing_pos::<Player>(&mut app).is_none());
        assert!(standing_pos::<Enemy>(&mut app).is_none());
    }

    #[test]
    fn every_procedural_probe_loads_terrain_anchors_and_actors() {
        for scenario_name in ["Frozen Hills", "Volcanic Hills", "Sky Islands", "Mountains"] {
            let scenario = library()
                .scenarios
                .into_iter()
                .find(|scenario| scenario.name == scenario_name)
                .expect("the procedural probe should be shipped");
            let configured_seed = scenario
                .generation_seed
                .expect("the probe should have a configured seed");
            let mut app = procedural_gameplay_app(scenario_name);
            enter_screen(&mut app, Screen::Gameplay);

            assert!(
                app.world().contains_resource::<TerrainReady>(),
                "{scenario_name} did not finish terrain generation"
            );
            let report = app.world().resource::<GenerationReport>();
            assert_eq!(report.seed, configured_seed);
            assert!(
                report
                    .notes
                    .iter()
                    .all(|note| note.starts_with("candidate ")),
                "{scenario_name} retained a non-candidate diagnostic after successful generation: \
                 {:?}",
                report.notes
            );
            assert!(
                !report.used_fallback,
                "{scenario_name} unexpectedly used its canonical fallback"
            );
            let anchors = app.world().resource::<MapAnchors>();
            for required in [&scenario.units.player, &scenario.units.enemy]
                .into_iter()
                .filter_map(|placement| match placement {
                    ScenarioPlacement::Anchor(name) => Some(name.as_str()),
                    ScenarioPlacement::Fixed(_) => None,
                })
            {
                assert!(
                    anchors.get(&MapAnchorId::from(required)).is_some(),
                    "{scenario_name} omitted {required}"
                );
            }
            let recipe_anchors: &[&str] = if scenario_name == "Mountains" {
                &["conflict_center", "high_pass", "low_bypass"]
            } else {
                &["conflict_center", "bridge", "alternate_crossing"]
            };
            for required in recipe_anchors {
                assert!(
                    anchors.get(&MapAnchorId::from(*required)).is_some(),
                    "{scenario_name} omitted recipe anchor {required}"
                );
            }
            let special_regions = app.world().resource::<SpecialMovementRegions>();
            if matches!(scenario_name, "Sky Islands" | "Mountains") {
                assert!(
                    !special_regions.is_empty(),
                    "{scenario_name} dropped its optional high-ground semantics"
                );
            } else {
                assert!(
                    special_regions.is_empty(),
                    "{scenario_name} introduced an unexpected optional region"
                );
            }
            assert!(standing_pos::<Player>(&mut app).is_some());
            assert!(standing_pos::<Enemy>(&mut app).is_some());
            assert_eq!(
                app.world_mut()
                    .query_filtered::<Entity, With<HexGrid>>()
                    .iter(app.world())
                    .count(),
                1,
                "{scenario_name} did not spawn exactly one rendered grid"
            );
        }
    }
}
