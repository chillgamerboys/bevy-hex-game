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
    choose_settings, Encounter, LightingSettings, Scenario, SelectSettings, SettingsRegistry,
    CONFIG_EXTENSIONS,
};
use hex_core::{
    GameplaySetup, GameplaySetupFailure, InteriorRegions, MapAnchors, MapViewHint, ResolvedMapSeed,
    Screen, SimSeeds, SpecialMovementRegions, TerrainReady,
};
use hex_map::{MapSettings, TerrainSettings};
use hex_units::Faction;
use hex_world::TimeOfDay;

pub(super) fn plugin(app: &mut App) {
    // `select_settings` rather than `load_settings`: there is no world file to load
    // until somebody has picked a scenario. It shares the registration `hex_map`
    // already did, which is idempotent, so plugin order does not matter here.
    app.register_type::<ResolvedMapSeed>()
        .register_type::<SimSeeds>()
        .register_type::<GameplaySetupFailure>()
        .select_settings::<MapSettings>(CONFIG_EXTENSIONS);
    // Lighting is chosen the same way, so a scenario brings its own sky and sun.
    // `hex_assets` no longer loads `lighting.ron` at startup -- two mechanisms writing
    // one resource is the collision `hex_map` already had.
    app.select_settings::<LightingSettings>(CONFIG_EXTENSIONS);
    // And so is the encounter: one file per roster, chosen by path. A *directory* of
    // encounters is never loaded at once, because a scenario needs exactly one of them
    // — which is what keeps the one-path-one-type settings loader untouched.
    app.select_settings::<Encounter>(CONFIG_EXTENSIONS);
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

/// Exact launch input retained for deterministic defeat retry.
#[derive(Resource, Debug, Clone)]
pub(super) struct ActiveScenario(pub(super) ScenarioToLoad);

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

/// Asks for the three files the chosen scenario names: its world, its sky, its roster.
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
    commands.remove_resource::<SimSeeds>();
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
        commands.remove_resource::<Encounter>();
        commands.insert_resource(GameplaySetupFailure::new(
            "Loading started without a selected scenario.",
        ));
        next.set(Screen::Title);
        error!("loading entered without a clicked scenario; returning to the title screen");
        return;
    };
    let scenario = pending.scenario.clone();
    let resolved_seed = pending.resolved_seed;
    commands.insert_resource(ActiveScenario(ScenarioToLoad {
        scenario: scenario.clone(),
        resolved_seed,
    }));
    commands.remove_resource::<ScenarioToLoad>();

    if let Some(seed) = resolved_seed {
        info!("starting scenario: {} (seed {})", scenario.name, seed.0);
        commands.insert_resource(seed);
    } else {
        info!("starting scenario: {}", scenario.name);
    }
    commands.insert_resource(sim_seeds_for(&scenario.name, resolved_seed));
    commands.insert_resource(ScenarioTimeOverride(scenario.starting_time_hours));
    choose_settings::<MapSettings>(&mut commands, &asset_server, &mut registry, &scenario.world);
    choose_settings::<LightingSettings>(
        &mut commands,
        &asset_server,
        &mut registry,
        &scenario.lighting,
    );
    choose_settings::<Encounter>(
        &mut commands,
        &asset_server,
        &mut registry,
        &scenario.encounter,
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
    encounter: Option<Res<Encounter>>,
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
    let (Some(map), Some(encounter), Some(lighting), Some(time_override)) =
        (map, encounter, lighting, time_override)
    else {
        return;
    };
    let inputs_changed = map.is_changed()
        || encounter.is_changed()
        || lighting.is_changed()
        || time_override.is_changed()
        || seed.as_ref().is_some_and(|seed| seed.is_changed());
    if status.is_some() && !inputs_changed {
        return;
    }

    let contract_error = scenario_contract_error(&map, &encounter, seed.as_deref())
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

/// Whether the chosen encounter can be placed on the world the scenario named.
///
/// Two files, each valid alone: an encounter cannot see whether its terrain is generated,
/// and a world cannot see who is standing on it. Every entry is checked rather than a
/// side at a time — one authored coordinate in an otherwise anchored roster is the same
/// bug, and it would otherwise only surface as one unit missing from the fight.
fn scenario_contract_error(
    map: &MapSettings,
    encounter: &Encounter,
    seed: Option<&ResolvedMapSeed>,
) -> Option<String> {
    match &map.terrain {
        TerrainSettings::Procedural(_) => {
            if seed.is_none() {
                return Some("procedural terrain has no resolved generation seed".to_owned());
            }
            for unit in encounter.entries() {
                if !unit.placement.is_generated() {
                    return Some(format!(
                        "the {} {:?} is placed on an authored coordinate, but procedural terrain \
                         must use a map anchor",
                        unit.faction.label(),
                        unit.archetype
                    ));
                }
            }
        }
        TerrainSettings::Showcase(_) | TerrainSettings::Perlin(_) => {
            if seed.is_some() {
                return Some(
                    "authored terrain must not receive a scenario generation seed".to_owned(),
                );
            }
            for unit in encounter.entries() {
                if let Some(anchor) = unit.placement.anchor() {
                    return Some(format!(
                        "the {} {:?} uses map anchor {anchor:?}, but authored terrain publishes \
                         none and requires fixed placements",
                        unit.faction.label(),
                        unit.archetype
                    ));
                }
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
    encounter: Option<Res<Encounter>>,
    units: Query<&Faction>,
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
        .or_else(|| roster_shortfall(encounter.as_deref(), &units));

    let Some(reason) = reason else { return };
    if failure.is_none() {
        commands.insert_resource(GameplaySetupFailure::new(reason.clone()));
    }
    error!("gameplay setup failed: {reason}");
    next.set(Screen::Title);
}

/// Whether every side the encounter rosters actually stands on the map.
///
/// This replaced "exactly one player and exactly one enemy", which was true of the
/// two-coordinate scaffold and is not a fact about a roster: an encounter may field four
/// player units, three hostiles, or two hostile groups holding different ground. What
/// still has to hold is that each side the encounter rosters *arrived in full* — the
/// count is compared per faction rather than in total, so three hostiles standing in for
/// a missing party member does not add up to a valid setup.
///
/// `hex_units` names the entry and the reason when a placement fails, and it is the
/// better message. This is the backstop for a placement that goes missing without one.
fn roster_shortfall(encounter: Option<&Encounter>, units: &Query<&Faction>) -> Option<String> {
    let Some(encounter) = encounter else {
        return Some("Gameplay started with no encounter to spawn.".to_owned());
    };

    for faction in encounter.factions() {
        let rostered = encounter.unit_count(faction);
        let standing = units
            .iter()
            .filter(|spawned| **spawned == Faction::from(faction))
            .count();
        if standing != rostered {
            let plural = if rostered == 1 { "unit" } else { "units" };
            return Some(format!(
                "Encounter {:?} rosters {rostered} {} {plural}, but {standing} of them stand on \
                 the map.",
                encounter.name,
                faction.label()
            ));
        }
    }
    None
}

fn clear_session_resources(mut commands: Commands) {
    commands.remove_resource::<ResolvedMapSeed>();
    commands.remove_resource::<SimSeeds>();
    commands.remove_resource::<ScenarioTimeOverride>();
    commands.remove_resource::<TimeOfDay>();
    commands.remove_resource::<ActiveScenario>();
}

/// Derives the session's sim seeds from what already determines the world.
///
/// The world seed folds the scenario's name with the resolved map seed (or a
/// fixed constant for authored maps), so the same launch always deals the same
/// seeds — a replay's precondition. The three streams are decorrelated splits
/// of that one value; see [`SimSeeds`] for why nothing reads them yet.
fn sim_seeds_for(name: &str, resolved: Option<ResolvedMapSeed>) -> SimSeeds {
    // FNV-1a over the name: tiny, stable, and dependency-free — this is an
    // identity fold, not a quality hash.
    let mut folded: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in name.bytes() {
        folded ^= u64::from(byte);
        folded = folded.wrapping_mul(0x0000_0100_0000_01B3);
    }
    let base = folded ^ resolved.map_or(0xA076_1D64_78BD_642F, |seed| seed.0);

    // splitmix64 finalizer to decorrelate the three streams.
    let mix = |mut value: u64| {
        value = value.wrapping_add(0x9E37_79B9_7F4A_7C15);
        value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        value ^ (value >> 31)
    };
    SimSeeds {
        world: mix(base),
        ai_flavor: mix(base.wrapping_add(1)),
        cosmetic: mix(base.wrapping_add(2)),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Instant;

    use bevy::app::PluginsState;
    use bevy::asset::AssetPlugin;
    use bevy::prelude::*;
    use bevy::state::app::StatesPlugin;
    use bevy::MinimalPlugins;
    use hex_assets::{
        AiProfileCatalog, ArtPalette, CombatSettings, ContentIndex, CubeCoord, ElementCatalog,
        ElementFile, Encounter, EncounterFaction, EncounterPlacement, FormationCatalog,
        FormationCenter, GameAssets, LatticeFile, LatticeLibrary, LightingSettings,
        PerceptionSettings, PlayerSettings, Roster, RosterEntry, ScenarioLibrary, SettingsRegistry,
        SpellBook, SpellFile, SubstanceFile, SubstanceTable,
    };
    use hex_combat::{
        AiDecisionTraces, CombatSummary, EncounterOutcome, EncounterResolution, TurnOrder,
        MAX_AI_DECISION_TRACES, MAX_COMBAT_SUMMARY_DETAILS,
    };
    use hex_core::{
        AppSystems, Busy, CommandQueue, ControlOwner, ExteriorIllumination, GameCommand,
        GameplayLight, GameplaySetup, GameplaySetupFailure, Headroom, HexCoord, HexGrid, HexSpan,
        HexTile, IlluminationLevel, InteriorRegions, IssuedCommand, KnowledgeState, LatticeCoord,
        LocalMapKnowledge, MapAnchorId, MapAnchors, MapViewHint, Mode, PartyFormation,
        PausableSystems, Pause, PendingDecision, PerceptionSystems, PlayerSeat, ResolvedMapSeed,
        Screen, SpecialMovementRegion, SpecialMovementRegions, SubstanceId, TerrainReady, TilePos,
        TraversalBlockers, Turn, UnitId,
    };
    use hex_lattice::{LatticeSpec, LatticeState};
    use hex_map::{GenerationReport, MapSettings, TerrainSettings, VoxelMap};
    use hex_perception::{FactionMapKnowledge, ResolvedIllumination};
    use hex_units::{
        either_in_reach, plan_formation_move, Body, Downed, Enemy, Faction, Footing,
        FormationMember, Player, Reach, StandsOn,
    };
    use hex_world::TimeOfDay;

    use super::{
        clear_session_resources, finalize_gameplay_setup, initialize_time_of_day,
        scenario_contract_error, validate_gameplay_lighting_contract, validate_loaded_scenario,
        ActiveScenario, ScenarioTimeOverride, ScenarioToLoad,
    };

    fn library() -> ScenarioLibrary {
        ron::from_str(include_str!("../../../assets/config/scenarios.ron"))
            .expect("the shipped scenarios should parse")
    }

    fn assets_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets")
    }

    /// The encounter a scenario names, read off disk.
    ///
    /// The whole point of the path is that this crate is the first layer allowed to open
    /// both files, so the cross-file contract is checked here and nowhere lower.
    fn encounter_of(scenario: &super::Scenario) -> Encounter {
        let path = assets_dir().join(&scenario.encounter);
        let text = fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!(
                "scenario {:?} names encounter {:?}, which could not be read: {error}",
                scenario.name, scenario.encounter
            )
        });
        ron::from_str(&text).unwrap_or_else(|error| {
            panic!(
                "scenario {:?} names an encounter that does not parse: {error}",
                scenario.name
            )
        })
    }

    /// A two-unit encounter built in Rust, for the contract cases that need a roster the
    /// shipped content deliberately does not contain.
    fn duel(player: EncounterPlacement, hostile: EncounterPlacement) -> Encounter {
        let side = |faction, placement, archetype: &str| Roster {
            faction,
            placement,
            units: vec![RosterEntry {
                archetype: archetype.to_owned(),
                placement: None,
                ai_profile: None,
                ai_group: None,
            }],
        };
        Encounter {
            name: "Test Duel".to_owned(),
            rosters: vec![
                side(EncounterFaction::Player, player, "hedge-mage"),
                side(EncounterFaction::Hostile, hostile, "raider"),
            ],
        }
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

    /// Every encounter a scenario names exists, parses, and rosters both sides.
    ///
    /// Same reasoning as the world and lighting checks — the path is a plain string, so
    /// a typo would otherwise be a loading screen that hangs for the one scenario nobody
    /// clicked. `Encounter`'s `Deserialize` runs `validate()`, so this also proves the
    /// roster is *placeable* in the ways a single file can be judged: no empty roster, no
    /// coordinate that is not a hex, no two units sharing one exact surface.
    #[test]
    fn every_scenario_names_an_encounter_that_exists_and_parses() {
        for scenario in &library().scenarios {
            let encounter = encounter_of(scenario);
            assert!(
                encounter.unit_count(EncounterFaction::Player) >= 1,
                "scenario {:?} rosters no player units",
                scenario.name
            );
            assert!(
                encounter.unit_count(EncounterFaction::Hostile) >= 1,
                "scenario {:?} rosters nobody to fight",
                scenario.name
            );
            for unit in encounter.entries() {
                assert!(
                    !unit.archetype.is_empty(),
                    "scenario {:?} rosters a unit with no archetype",
                    scenario.name
                );
            }
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
                scenario_contract_error(&world, &encounter_of(scenario), seed.as_ref()),
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
        app.insert_resource(encounter_of(&entry));
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

        assert!(scenario_contract_error(&world, &encounter_of(&entry), None)
            .is_some_and(|error| error.contains("no resolved")));

        let authored = duel(
            EncounterPlacement::Fixed(CubeCoord { x: 0, y: 0, z: 0 }),
            EncounterPlacement::Fixed(CubeCoord { x: 1, y: -1, z: 0 }),
        );
        assert!(
            scenario_contract_error(&world, &authored, Some(&ResolvedMapSeed(1)))
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
        // Recipe-specific names, and a formation on one side: a formation is generated
        // exactly when its centre is, so it satisfies the same contract as a bare anchor.
        let placements = duel(
            EncounterPlacement::Anchor("surface_entrance".to_owned()),
            EncounterPlacement::Formation {
                center: FormationCenter::Anchor("deep_chamber".to_owned()),
                spread: 2,
            },
        );

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
        let authored = duel(
            EncounterPlacement::Fixed(CubeCoord { x: 0, y: 0, z: 0 }),
            EncounterPlacement::Fixed(CubeCoord { x: 1, y: -1, z: 0 }),
        );
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin));
        app.init_state::<Screen>();
        app.insert_resource(SettingsRegistry::default());
        app.insert_resource(world);
        app.insert_resource(authored);
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
                GameplaySetup::Perception,
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
                GameplaySetup::Perception,
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

    /// A finalizer harness: an encounter rostering `rostered` units a side, with
    /// `spawned` of each actually standing on the map.
    ///
    /// The counts are separate because the check is exactly the gap between them. It used
    /// to be "exactly one player and exactly one enemy", which is a fact about the
    /// scaffold rather than about a roster — a party of four was structurally invalid.
    fn finalizer_app(
        terrain_ready: bool,
        rostered: usize,
        spawned: usize,
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
                GameplaySetup::Perception,
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
        let side = |faction| Roster {
            faction,
            placement: EncounterPlacement::Formation {
                center: FormationCenter::Anchor("party_start".to_owned()),
                spread: 2,
            },
            units: (0..rostered)
                .map(|_| RosterEntry {
                    archetype: "hedge-mage".to_owned(),
                    placement: None,
                    ai_profile: None,
                    ai_group: None,
                })
                .collect(),
        };
        app.insert_resource(Encounter {
            name: "Finalizer".to_owned(),
            rosters: vec![
                side(EncounterFaction::Player),
                side(EncounterFaction::Hostile),
            ],
        });
        for _ in 0..spawned {
            app.world_mut().spawn((Player, Faction::Player));
            app.world_mut().spawn((Enemy, Faction::Hostile));
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

    /// A roster that arrives short is a setup failure naming the shortfall.
    ///
    /// Four rostered, three standing: the count that matters is per side and against the
    /// roster, so the old "exactly one" check would have called this a valid setup.
    #[test]
    fn finalizer_returns_to_title_when_a_rostered_unit_is_missing() {
        let mut app = finalizer_app(true, 4, 3, None);

        enter_gameplay_and_settle(&mut app);

        assert_eq!(
            *app.world().resource::<State<Screen>>().get(),
            Screen::Title
        );
        let reason = &app.world().resource::<GameplaySetupFailure>().reason;
        assert!(
            reason.contains("rosters 4 player units, but 3"),
            "the failure should say how many are missing from which side: {reason}"
        );
    }

    /// And a full roster of four a side is a valid setup, which the retired check made
    /// structurally impossible.
    #[test]
    fn finalizer_accepts_a_party_larger_than_one() {
        let mut app = finalizer_app(true, 4, 4, None);

        enter_gameplay_and_settle(&mut app);

        assert_eq!(
            *app.world().resource::<State<Screen>>().get(),
            Screen::Gameplay,
            "a four-unit roster is a valid encounter"
        );
        assert!(!app.world().contains_resource::<GameplaySetupFailure>());
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

    /// And every rostered unit starts inside the world it is placed on.
    ///
    /// Not a formality. An authored coordinate outside the grid radius has no surface
    /// under it, which now fails setup and sends the player back to the title screen —
    /// so a scenario nobody has clicked yet would be broken with nothing to say so.
    ///
    /// Every entry, not one per side: a roster can be wrong about its fourth unit.
    #[test]
    fn every_unit_starts_inside_its_own_world() {
        for scenario in &library().scenarios {
            let text = fs::read_to_string(assets_dir().join(&scenario.world))
                .expect("the world file should exist");
            let world: MapSettings = ron::from_str(&text).expect("the world should parse");
            let encounter = encounter_of(scenario);

            for unit in encounter.entries() {
                let who = format!("{} {:?}", unit.faction.label(), unit.archetype);
                if let Some(coord) = unit.placement.fixed_coord() {
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
                if let Some(anchor) = unit.placement.anchor() {
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

    /// The showcase starts with one unit at each end of its defining crossing.
    ///
    /// A formation of one stands exactly on its centre, so this is still an assertion
    /// about two precise hexes — which is what the scenario is for.
    #[test]
    fn the_crossing_starts_units_at_opposite_bridge_landings() {
        let library = library();
        let crossing = library
            .scenarios
            .iter()
            .find(|scenario| scenario.name == "The Crossing")
            .expect("the shipped library should contain The Crossing");
        let encounter = encounter_of(crossing);
        let landings: Vec<Option<CubeCoord>> = encounter
            .entries()
            .map(|unit| unit.placement.fixed_coord())
            .collect();

        assert_eq!(
            landings,
            vec![
                Some(CubeCoord { x: 0, y: 4, z: -4 }),
                Some(CubeCoord { x: 0, y: -4, z: 4 }),
            ]
        );
    }

    /// The integrated trial keeps both complete parties stable and outside engagement
    /// range so formation editing and the bridge approach remain player decisions.
    #[test]
    fn party_trial_starts_matching_stable_parties_apart() {
        let library = library();
        let trial = library
            .scenarios
            .iter()
            .find(|scenario| scenario.name == "Party Trial")
            .expect("the shipped library should contain Party Trial");
        let encounter = encounter_of(trial);

        assert_eq!(encounter.rosters.len(), 2);
        assert_eq!(
            encounter
                .rosters
                .iter()
                .map(|roster| roster.faction)
                .collect::<Vec<_>>(),
            vec![EncounterFaction::Player, EncounterFaction::Hostile]
        );
        for roster in &encounter.rosters {
            assert_eq!(
                roster
                    .units
                    .iter()
                    .map(|unit| unit.archetype.as_str())
                    .collect::<Vec<_>>(),
                vec!["hedge-mage", "raider", "wolf"]
            );
            let EncounterPlacement::Formation { spread, .. } = roster.placement else {
                panic!("Party Trial rosters must use formation placement");
            };
            assert_eq!(spread, 2);
        }

        let centres = encounter
            .rosters
            .iter()
            .map(|roster| match roster.placement {
                EncounterPlacement::Formation {
                    center: FormationCenter::Fixed(coord),
                    ..
                } => coord,
                _ => unreachable!("checked above"),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            centres,
            vec![
                CubeCoord { x: 0, y: 8, z: -8 },
                CubeCoord { x: 0, y: -8, z: 8 },
            ]
        );
        let [first, second] = centres.as_slice() else {
            panic!("Party Trial should have exactly two roster centres");
        };
        let separation = (first.x - second.x)
            .abs()
            .max((first.y - second.y).abs())
            .max((first.z - second.z).abs());
        assert!(
            separation > 4,
            "Party Trial must begin beyond engagement range"
        );
    }

    /// Automated combat UI walks use minimal flat fixtures instead of making ability
    /// assertions depend on the Crossing's routing and six-unit initiative.
    #[test]
    fn focused_ui_trials_are_flat_and_roster_only_the_roles_they_need() {
        let library = library();
        let ability = library
            .scenarios
            .iter()
            .find(|scenario| scenario.name == "Ability Lab")
            .expect("the shipped library should contain Ability Lab");
        let mirror = library
            .scenarios
            .iter()
            .find(|scenario| scenario.name == "Raider Mirror")
            .expect("the shipped library should contain Raider Mirror");

        assert_eq!(ability.world, "config/worlds/flat-combat.ron");
        assert_eq!(mirror.world, ability.world);
        let world_path = assets_dir().join(&ability.world);
        let world_text = fs::read_to_string(&world_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", world_path.display()));
        let world: MapSettings =
            ron::from_str(&world_text).expect("the flat combat world should parse");
        let TerrainSettings::Perlin(perlin) = world.terrain else {
            panic!("the flat combat fixture must not carry authored terrain features");
        };
        assert!(
            perlin.steps.is_empty(),
            "an empty height recipe is level everywhere"
        );

        let ability = encounter_of(ability);
        assert_eq!(ability.unit_count(EncounterFaction::Player), 2);
        assert_eq!(ability.unit_count(EncounterFaction::Hostile), 1);
        assert_eq!(
            ability
                .entries()
                .map(|unit| (unit.faction, unit.archetype))
                .collect::<Vec<_>>(),
            vec![
                (EncounterFaction::Player, "hedge-mage"),
                (EncounterFaction::Player, "wolf"),
                (EncounterFaction::Hostile, "raider"),
            ]
        );

        let mirror = encounter_of(mirror);
        assert_eq!(mirror.unit_count(EncounterFaction::Player), 1);
        assert_eq!(mirror.unit_count(EncounterFaction::Hostile), 1);
        assert_eq!(
            mirror
                .entries()
                .map(|unit| (unit.faction, unit.archetype))
                .collect::<Vec<_>>(),
            vec![
                (EncounterFaction::Player, "raider"),
                (EncounterFaction::Hostile, "raider"),
            ]
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
        let stale_encounter = library()
            .scenarios
            .first()
            .map(encounter_of)
            .expect("the shipped library should not be empty");
        app.insert_resource(stale_encounter);
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
            !app.world().contains_resource::<hex_assets::Encounter>(),
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
            .get_resource::<hex_assets::Encounter>()
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
            .get_resource::<hex_assets::Encounter>()
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
            first_units, second_units,
            "both scenarios produced the same encounter"
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
        let active = app.world().resource::<ActiveScenario>();
        let entries = library();
        let selected = entries
            .scenarios
            .get(procedural_index)
            .expect("the selected scenario still exists");
        assert_eq!(active.0.scenario.name, selected.name);
        assert_eq!(active.0.scenario.world, selected.world);
        assert_eq!(active.0.scenario.encounter, selected.encounter);
        assert_eq!(active.0.resolved_seed, Some(ResolvedMapSeed(configured)));
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
        procedural_gameplay_app_with_combat(scenario_name, false)
    }

    fn shipped_combat_content(
        substances: &SubstanceTable,
    ) -> (
        ElementCatalog,
        SpellBook,
        ContentIndex,
        LatticeLibrary,
        AiProfileCatalog,
        FormationCatalog,
    ) {
        let elements_file: ElementFile =
            ron::from_str(include_str!("../../../assets/config/elements.ron"))
                .expect("the shipped elements should deserialize");
        let spells_file: SpellFile =
            ron::from_str(include_str!("../../../assets/config/spells.ron"))
                .expect("the shipped spells should deserialize");
        let lattices_file: LatticeFile =
            ron::from_str(include_str!("../../../assets/config/lattices.ron"))
                .expect("the shipped lattices should deserialize");
        let profiles: AiProfileCatalog =
            ron::from_str(include_str!("../../../assets/config/ai_profiles.ron"))
                .expect("the shipped AI profiles should deserialize");
        let formations: FormationCatalog =
            ron::from_str(include_str!("../../../assets/config/formations.ron"))
                .expect("the shipped formations should deserialize");
        let elements = ElementCatalog::from_file(&elements_file);
        let spells = SpellBook::from_file(&spells_file);
        let index = ContentIndex::build(&elements, &spells, substances)
            .expect("the shipped combat content should cross-resolve");
        let lattices = LatticeLibrary::build(&lattices_file, &elements, &spells)
            .expect("the shipped lattices should resolve");
        (elements, spells, index, lattices, profiles, formations)
    }

    fn procedural_gameplay_app_with_combat(scenario_name: &str, with_combat: bool) -> App {
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
        let palette: ArtPalette = ron::from_str(include_str!("../../../assets/art/palette.ron"))
            .expect("the shipped art palette should deserialize");
        let seed = entry.generation_seed.map(ResolvedMapSeed);

        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            StatesPlugin,
            bevy::input::InputPlugin,
        ));
        app.init_asset::<Mesh>();
        app.init_asset::<StandardMaterial>();
        app.init_state::<Screen>();
        app.add_sub_state::<Mode>();
        app.add_sub_state::<Pause>();
        app.configure_sets(
            Update,
            (
                AppSystems::TickTimers,
                AppSystems::RecordInput,
                AppSystems::Update,
            )
                .chain(),
        );
        app.configure_sets(Update, PausableSystems.run_if(in_state(Pause(false))));
        app.configure_sets(
            Update,
            (
                PerceptionSystems::PublishAmbient,
                PerceptionSystems::ResolveIllumination,
                PerceptionSystems::ResolveObservation,
                PerceptionSystems::PublishKnowledge,
                PerceptionSystems::ApplyPresentation,
            )
                .chain()
                .in_set(AppSystems::Update),
        );
        app.configure_sets(
            OnEnter(Screen::Gameplay),
            (
                GameplaySetup::Resources,
                GameplaySetup::Terrain,
                GameplaySetup::Actors,
                GameplaySetup::Perception,
                GameplaySetup::View,
                GameplaySetup::Finalize,
            )
                .chain(),
        );
        app.configure_sets(
            OnEnter(Screen::Gameplay),
            (
                PerceptionSystems::PublishAmbient,
                PerceptionSystems::ResolveIllumination,
                PerceptionSystems::ResolveObservation,
                PerceptionSystems::PublishKnowledge,
                PerceptionSystems::ApplyPresentation,
            )
                .chain()
                .in_set(GameplaySetup::Perception),
        );
        app.insert_resource(GameAssets {
            hex_tile: Handle::default(),
            player_pieces: [Handle::default(), Handle::default()],
        });
        let substances = SubstanceTable::from_file(&substances, &palette)
            .expect("the shipped substances should resolve through the shipped palette");
        app.insert_resource(substances.clone());
        app.insert_resource(PerceptionSettings::default());
        app.insert_resource(ExteriorIllumination::new(IlluminationLevel::Bright));
        app.insert_resource(player);
        app.insert_resource(palette);
        app.insert_resource(encounter_of(&entry));
        app.insert_resource(world);
        if let Some(seed) = seed {
            app.insert_resource(seed);
        }
        app.add_plugins((
            hex_map::plugin,
            hex_units::movement::plugin,
            hex_perception::plugin,
        ));
        hex_units::units::plugin(&mut app);
        if with_combat {
            let combat: CombatSettings =
                ron::from_str(include_str!("../../../assets/config/combat.ron"))
                    .expect("the shipped combat settings should deserialize");
            let (elements, spells, index, lattices, profiles, formations) =
                shipped_combat_content(&substances);
            app.insert_resource(combat);
            app.insert_resource(elements);
            app.insert_resource(spells);
            app.insert_resource(index);
            app.insert_resource(lattices);
            app.insert_resource(profiles);
            app.insert_resource(formations);
            app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(100),
            ));
            app.add_plugins((hex_anim::plugin, hex_combat::plugin));
        }
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

    #[derive(Debug, PartialEq, Eq)]
    struct PartyTrialReplay {
        player_stream: Vec<IssuedCommand>,
        summary: CombatSummary,
        turn_order: Vec<UnitId>,
        current: Option<UnitId>,
        round: u32,
        positions: Vec<(UnitId, TilePos)>,
    }

    fn footing_for(app: &mut App, body: Body) -> Footing {
        let substances = app.world().resource::<SubstanceTable>().clone();
        let world = app.world_mut();
        let mut tiles =
            world.query_filtered::<(&TilePos, &HexSpan, &SubstanceId, &Headroom), With<HexTile>>();
        Footing::from_tiles(
            tiles.iter(world),
            &substances,
            body,
            world.get_resource::<TraversalBlockers>(),
        )
    }

    fn party_trial_move(app: &mut App) -> GameCommand {
        let formation = app.world().resource::<PartyFormation>().clone();
        let formations = app.world().resource::<FormationCatalog>();
        let preset = formations
            .get(&formation.preset)
            .expect("Party Trial should start with a resolved formation")
            .clone();
        let anchor_slot = preset
            .anchor()
            .expect("the shipped formation should have an anchor");
        let anchor = formation
            .assignments
            .iter()
            .find_map(|(&unit, &slot)| (slot == anchor_slot).then_some(unit))
            .expect("the party formation should assign its anchor");

        let mut facts = {
            let world = app.world_mut();
            let mut players = world.query_filtered::<(&UnitId, &StandsOn, &Body), With<Player>>();
            players
                .iter(world)
                .map(|(unit, standing, body)| (*unit, standing.0, *body))
                .collect::<Vec<_>>()
        };
        facts.sort_by_key(|(unit, ..)| *unit);
        let (_, anchor_standing, anchor_body) = facts
            .iter()
            .find(|(unit, ..)| *unit == anchor)
            .copied()
            .expect("the formation anchor should be a live player");
        let anchor_footing = Arc::new(footing_for(app, anchor_body));
        let destination = anchor_footing
            .at_coord(HexCoord::from_axial(0, -4))
            .iter()
            .max_by_key(|standing| standing.pos.level)
            .copied()
            .expect("Party Trial should expose the far bridge landing");
        let anchor_path = Reach::from(anchor_standing, &anchor_footing, None)
            .path_to(destination.pos)
            .expect("the party anchor should have a complete crossing route");
        let mut footing_by_body = vec![(anchor_body, Arc::clone(&anchor_footing))];
        for (_, _, body) in &facts {
            if footing_by_body
                .iter()
                .all(|(cached_body, _)| cached_body != body)
            {
                footing_by_body.push((*body, Arc::new(footing_for(app, *body))));
            }
        }
        let members = facts
            .into_iter()
            .map(|(unit, standing, body)| FormationMember {
                unit,
                standing,
                footing: Arc::clone(
                    &footing_by_body
                        .iter()
                        .find(|(cached_body, _)| *cached_body == body)
                        .expect("every member body should have a footing projection")
                        .1,
                ),
            })
            .collect();
        let plan = plan_formation_move(&preset, &formation, &anchor_path, members)
            .expect("the Party Trial party should compress across the bridge");
        GameCommand::MoveParty {
            anchor,
            paths: plan.paths,
        }
    }

    fn queue_player_command(app: &mut App, stream: &mut Vec<IssuedCommand>, command: GameCommand) {
        let issued = IssuedCommand {
            seat: PlayerSeat(0),
            command,
        };
        stream.push(issued.clone());
        app.world_mut().resource_mut::<CommandQueue>().push(issued);
    }

    fn player_decision_command(app: &mut App) -> Option<GameCommand> {
        let pending = app.world().resource::<PendingDecision>().clone();
        let (decider, target, count, restoring) = match pending {
            PendingDecision::None => return None,
            PendingDecision::ChooseDisables { decider, count, .. } => {
                (decider, decider, count, false)
            }
            PendingDecision::ChooseRestores {
                decider,
                target,
                count,
            } => (decider, target, count, true),
        };
        let player_owns_decision = {
            let world = app.world_mut();
            let mut owners = world.query::<(&UnitId, &ControlOwner)>();
            owners
                .iter(world)
                .any(|(unit, owner)| *unit == decider && owner.0 == PlayerSeat(0))
        };
        if !player_owns_decision {
            return None;
        }
        let mut cells = {
            let world = app.world_mut();
            let mut lattices = world.query::<(&UnitId, &LatticeSpec, &LatticeState)>();
            let (_, spec, state) = lattices.iter(world).find(|(unit, ..)| **unit == target)?;
            spec.cells()
                .filter(|(cell, _)| state.is_disabled(*cell) == restoring)
                .map(|(cell, _)| cell)
                .collect::<Vec<LatticeCoord>>()
        };
        cells.sort_unstable();
        cells.truncate(usize::from(count));
        Some(if restoring {
            GameCommand::ChooseRestores {
                unit: decider,
                target,
                cells,
            }
        } else {
            GameCommand::ChooseDisables {
                unit: decider,
                cells,
            }
        })
    }

    fn finish_presentations(app: &mut App) {
        let moving = {
            let world = app.world_mut();
            let mut moving = world.query_filtered::<Entity, With<hex_anim::Transformation>>();
            moving.iter(world).collect::<Vec<_>>()
        };
        for entity in moving {
            app.world_mut()
                .entity_mut(entity)
                .remove::<hex_anim::Transformation>();
        }
    }

    fn player_turn_command(app: &mut App, actor: UnitId) -> Option<GameCommand> {
        let (standing, body, turn) = {
            let world = app.world_mut();
            let mut actors = world.query_filtered::<
                (&UnitId, &StandsOn, &Body, &Turn),
                (With<Player>, Without<Downed>, Without<Busy>),
            >();
            let (_, standing, body, turn) =
                actors.iter(world).find(|(unit, ..)| **unit == actor)?;
            (standing.0, *body, *turn)
        };
        let mut hostiles = {
            let world = app.world_mut();
            let mut targets =
                world.query_filtered::<(&UnitId, &StandsOn), (With<Enemy>, Without<Downed>)>();
            targets
                .iter(world)
                .map(|(unit, standing)| (*unit, standing.0))
                .collect::<Vec<_>>()
        };
        hostiles.sort_by_key(|(unit, ..)| *unit);
        let footing = footing_for(app, body);
        if !turn.acted {
            if let Some((target, _)) = hostiles.iter().find(|(_, target)| {
                standing.pos.coord.distance(target.pos.coord) == 1
                    && (footing.admits_step(standing.pos, target.pos)
                        || footing.admits_step(target.pos, standing.pos))
            }) {
                return Some(GameCommand::Strike {
                    unit: actor,
                    target: *target,
                });
            }
        }
        if turn.movement_left == 0 {
            return Some(GameCommand::EndTurn { unit: actor });
        }

        let occupied = {
            let world = app.world_mut();
            let mut units = world.query::<&StandsOn>();
            units
                .iter(world)
                .map(|standing| standing.0.pos)
                .collect::<Vec<_>>()
        };
        let reach = Reach::from(standing, &footing, None);
        let route = hostiles
            .iter()
            .flat_map(|(target, target_standing)| {
                footing
                    .standings()
                    .into_iter()
                    .filter(|candidate| {
                        candidate.pos.coord.distance(target_standing.pos.coord) == 1
                            && (footing.admits_step(candidate.pos, target_standing.pos)
                                || footing.admits_step(target_standing.pos, candidate.pos))
                            && (candidate.pos == standing.pos || !occupied.contains(&candidate.pos))
                    })
                    .filter_map(|candidate| {
                        reach
                            .path_to(candidate.pos)
                            .map(|path| (*target, candidate.pos, path))
                    })
            })
            .min_by_key(|(target, destination, path)| (path.len(), *target, *destination))
            .map(|(_, _, mut path)| {
                path.truncate(
                    usize::try_from(turn.movement_left)
                        .unwrap_or(usize::MAX)
                        .saturating_add(1),
                );
                path
            });
        if let Some(path) = route.filter(|path| path.len() > 1) {
            return Some(GameCommand::MoveAlong {
                unit: actor,
                path: path.into_iter().map(|step| step.pos).collect(),
            });
        }
        Some(GameCommand::EndTurn { unit: actor })
    }

    fn run_party_trial_replay() -> PartyTrialReplay {
        let mut app = procedural_gameplay_app_with_combat("Party Trial", true);
        enter_screen(&mut app, Screen::Gameplay);
        assert_eq!(
            *app.world().resource::<State<Mode>>().get(),
            Mode::Exploring,
            "Party Trial should leave room for formation travel"
        );

        let mut player_stream = Vec::new();
        let crossing = party_trial_move(&mut app);
        queue_player_command(&mut app, &mut player_stream, crossing);

        for _ in 0..4_000 {
            finish_presentations(&mut app);
            if app
                .world()
                .resource::<EncounterResolution>()
                .outcome()
                .is_some()
            {
                app.update();
                break;
            }
            if app.world().resource::<CommandQueue>().is_empty() {
                if let Some(answer) = player_decision_command(&mut app) {
                    queue_player_command(&mut app, &mut player_stream, answer);
                } else {
                    let current = app.world().resource::<TurnOrder>().current();
                    if let Some(command) =
                        current.and_then(|current| player_turn_command(&mut app, current))
                    {
                        queue_player_command(&mut app, &mut player_stream, command);
                    }
                }
            }
            app.update();
        }
        let outcome = app.world().resource::<EncounterResolution>().outcome();
        assert_eq!(
            outcome,
            Some(EncounterOutcome::Defeat),
            "the deterministic player policy should reach defeat; mode={:?}, pending={:?}, \
             round={}, moves={}, casts={}, strikes={}, downings={}",
            app.world().resource::<State<Mode>>().get(),
            app.world().resource::<PendingDecision>(),
            app.world().resource::<TurnOrder>().round,
            app.world().resource::<CombatSummary>().moves,
            app.world().resource::<CombatSummary>().casts,
            app.world().resource::<CombatSummary>().strikes,
            app.world().resource::<CombatSummary>().downings,
        );
        assert!(
            app.world().resource::<CommandQueue>().is_empty(),
            "the resolved Party Trial left an undrained command"
        );
        assert!(
            app.world().resource::<AiDecisionTraces>().entries.len() <= MAX_AI_DECISION_TRACES,
            "the live inspection window exceeded its bound"
        );

        let summary = app.world().resource::<CombatSummary>().clone();
        assert!(
            summary.ai_selections.len() <= MAX_COMBAT_SUMMARY_DETAILS,
            "the retained AI-decision window exceeded its bound"
        );
        assert!(
            summary.events.len() <= MAX_COMBAT_SUMMARY_DETAILS,
            "the retained combat-event window exceeded its bound"
        );
        let order = app.world().resource::<TurnOrder>();
        let turn_order = order.order().to_vec();
        let current = order.current();
        let round = order.round;
        let mut positions = {
            let world = app.world_mut();
            let mut units = world.query::<(&UnitId, &StandsOn)>();
            units
                .iter(world)
                .map(|(unit, standing)| (*unit, standing.0.pos))
                .collect::<Vec<_>>()
        };
        positions.sort_by_key(|(unit, ..)| *unit);
        PartyTrialReplay {
            player_stream,
            summary,
            turn_order,
            current,
            round,
            positions,
        }
    }

    /// Runs the shipped 3v3 scenario twice from its authored state.
    ///
    /// `CombatSummary::ai_selections` carries each exact observation, canonical legal
    /// set/fingerprint, selected route/command, and profile/algorithm dispatch. The
    /// remaining fields cover the player command stream, structured events, final
    /// positions, turn order, and outcome. Equality here is therefore the integrated
    /// replay contract rather than a second, weaker simulation snapshot.
    #[test]
    fn party_trial_replays_identically_end_to_end() {
        let first = run_party_trial_replay();
        assert!(
            matches!(
                first.player_stream.first().map(|issued| &issued.command),
                Some(GameCommand::MoveParty { paths, .. })
                    if paths.iter().all(|path| path.path.len() > 2)
            ),
            "the replay stream should contain exact full-party crossing routes"
        );
        assert!(
            first
                .summary
                .ai_selections
                .iter()
                .any(|trace| matches!(trace.command, Some(GameCommand::Cast { .. }))),
            "the baseline hostile party should select a cast"
        );
        assert!(first.summary.rounds > 0);
        assert!(first.summary.downings >= 3);
        assert_eq!(first.summary.outcome, Some(EncounterOutcome::Defeat));
        assert_eq!(
            first,
            run_party_trial_replay(),
            "the same Party Trial stream diverged"
        );
    }

    #[test]
    #[ignore = "manual release-mode 100-run Party Trial deterministic soak"]
    fn party_trial_one_hundred_run_soak_is_deterministic() {
        let started = Instant::now();
        let expected = run_party_trial_replay();
        for run in 1..100 {
            assert_eq!(
                run_party_trial_replay(),
                expected,
                "Party Trial run {} diverged from the reference",
                run + 1
            );
        }
        eprintln!(
            "PARTY_TRIAL_SOAK runs=100 elapsed_ms={} outcome={:?} rounds={} \
             ai_count={} ai_fingerprint={} event_count={} event_fingerprint={} \
             retained_ai={} retained_events={}",
            started.elapsed().as_millis(),
            expected.summary.outcome,
            expected.summary.rounds,
            expected.summary.ai_selection_count,
            expected.summary.ai_selection_fingerprint,
            expected.summary.event_count,
            expected.summary.event_fingerprint,
            expected.summary.ai_selections.len(),
            expected.summary.events.len(),
        );
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
        assert_eq!(
            app.world()
                .resource::<LocalMapKnowledge>()
                .state(first_party),
            KnowledgeState::Observed,
            "the real terrain and actor plugins should feed initial player knowledge"
        );
        assert_eq!(
            app.world()
                .resource::<FactionMapKnowledge>()
                .faction(hex_units::Faction::Player)
                .state(first_hostile),
            KnowledgeState::Observed,
            "bright-map faction knowledge should include the hostile anchor"
        );
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
        assert!(!app.world().contains_resource::<LocalMapKnowledge>());
        assert!(!app.world().contains_resource::<FactionMapKnowledge>());
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
            app.world()
                .resource::<LocalMapKnowledge>()
                .state(first_party),
            KnowledgeState::Observed,
            "re-entry should rebuild initial player knowledge"
        );
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
        // Point the hostile roster at an anchor the generator does not publish. The
        // whole roster must fail rather than the map coming up one unit short.
        {
            let mut encounter = app.world_mut().resource_mut::<Encounter>();
            let hostile = encounter
                .rosters
                .iter_mut()
                .find(|roster| roster.faction == EncounterFaction::Hostile)
                .expect("the shipped encounter should roster a hostile side");
            hostile.placement = EncounterPlacement::Anchor("missing_enemy_anchor".to_owned());
        }

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
    fn every_additional_procedural_scenario_loads_terrain_anchors_and_actors() {
        for scenario_name in [
            "Frozen Hills",
            "Volcanic Hills",
            "Sky Islands",
            "Mountains",
            "Caves",
            "Waterfall",
            "Forest",
        ] {
            let scenario = library()
                .scenarios
                .into_iter()
                .find(|scenario| scenario.name == scenario_name)
                .expect("the procedural scenario should be shipped");
            let configured_seed = scenario
                .generation_seed
                .expect("the procedural scenario should have a configured seed");
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
            let encounter = encounter_of(&scenario);
            let anchors = app.world().resource::<MapAnchors>();
            for required in encounter
                .entries()
                .filter_map(|unit| unit.placement.anchor())
            {
                assert!(
                    anchors.get(&MapAnchorId::from(required)).is_some(),
                    "{scenario_name} omitted {required}"
                );
            }
            let recipe_anchors: &[&str] = match scenario_name {
                "Mountains" => &["conflict_center", "high_pass", "low_bypass"],
                "Caves" => &["conflict_center", "cave_entrance", "deep_chamber"],
                "Waterfall" => &["fall_overlook", "basin_overlook"],
                "Forest" => &["forest_clearing", "prairie_overlook"],
                _ => &["conflict_center", "bridge", "alternate_crossing"],
            };
            for required in recipe_anchors {
                assert!(
                    anchors.get(&MapAnchorId::from(*required)).is_some(),
                    "{scenario_name} omitted recipe anchor {required}"
                );
            }
            let special_regions = app.world().resource::<SpecialMovementRegions>();
            match scenario_name {
                "Sky Islands" => assert!(
                    !special_regions.is_empty(),
                    "Sky Islands dropped its flight-gated upper layer"
                ),
                "Mountains" => {}
                "Waterfall" => {
                    assert_eq!(
                        special_regions.len(),
                        6,
                        "Waterfall dropped a radius-12 mid-cliff shelf"
                    );
                    assert!(
                        special_regions.iter().all(|(position, region)| {
                            position.level == 21 && region == SpecialMovementRegion(0)
                        }),
                        "Waterfall changed its exact mid-cliff shelf contract"
                    );
                }
                _ => assert!(
                    special_regions.is_empty(),
                    "{scenario_name} introduced an unexpected optional region"
                ),
            }
            let interiors = app.world().resource::<InteriorRegions>();
            if scenario_name == "Caves" {
                assert!(
                    interiors.surfaces().next().is_some(),
                    "Caves dropped its exact interior floors"
                );
                assert!(
                    interiors.roof_voxels().next().is_some(),
                    "Caves dropped its exact cutaway roofs"
                );
            } else {
                assert!(
                    interiors.is_empty(),
                    "{scenario_name} introduced unexpected interior metadata"
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

    #[test]
    fn shipped_v3_cave_lights_resolve_inside_the_exact_generated_domain() {
        let mut app = procedural_gameplay_app("Caves");
        enter_screen(&mut app, Screen::Gameplay);

        let anchors = app.world().resource::<MapAnchors>();
        let entrance = anchors
            .get(&MapAnchorId::from("cave_entrance"))
            .expect("Caves should publish cave_entrance");
        let deep_chamber = anchors
            .get(&MapAnchorId::from("deep_chamber"))
            .expect("Caves should publish deep_chamber");
        let interiors = app.world().resource::<InteriorRegions>().clone();
        let illumination = app.world().resource::<ResolvedIllumination>().clone();
        let generated_lights = {
            let world = app.world_mut();
            let mut query = world.query::<(&TilePos, &GameplayLight)>();
            query
                .iter(world)
                .map(|(position, light)| (*position, *light))
                .collect::<Vec<_>>()
        };

        assert!(!generated_lights.is_empty());
        assert!(generated_lights.iter().all(|(position, light)| {
            interiors.get(*position).is_some()
                && light.level == IlluminationLevel::Bright
                && (4..=7).contains(&light.radius)
        }));
        for required in [entrance, deep_chamber] {
            let resolved = illumination
                .get(required)
                .expect("required cave floor should be in the resolved perception frame");
            assert_eq!(resolved.level, IlluminationLevel::Bright);
            assert_eq!(
                Some(resolved.domain),
                interiors.get(required).map(hex_core::LightDomain::Interior)
            );
        }
        assert!(
            interiors.surfaces().any(|(position, _region)| {
                illumination
                    .get(position)
                    .is_some_and(|resolved| resolved.level == IlluminationLevel::Dark)
            }),
            "the generated cave should preserve at least one dark optional floor"
        );
    }

    /// The shipped cave is only playable if the ECS terrain, command funnel, and
    /// combat loop agree with the semantic cave validator across the complete entry
    /// route. Premature combat freezes free exploration and made a valid ramp look
    /// like broken movement when the old deep anchor could target through the rock.
    #[test]
    fn shipped_cave_entrance_is_live_walkable_before_combat_can_begin() {
        let mut app = procedural_gameplay_app_with_combat("Caves", true);
        enter_screen(&mut app, Screen::Gameplay);

        let anchors = app.world().resource::<MapAnchors>();
        let party_position = anchors
            .get(&MapAnchorId::from("party_start"))
            .expect("Caves should publish party_start");
        let hostile_position = anchors
            .get(&MapAnchorId::from("hostile_start"))
            .expect("Caves should publish hostile_start");
        let conflict_position = anchors
            .get(&MapAnchorId::from("conflict_center"))
            .expect("Caves should publish conflict_center");
        let (body, player_unit) = {
            let world = app.world_mut();
            let mut players = world.query_filtered::<(&Body, &UnitId), With<Player>>();
            let (body, unit) = players
                .single(world)
                .expect("Caves should spawn exactly one identified player");
            (*body, *unit)
        };

        let footing = {
            let world = app.world_mut();
            let mut tiles = world.query_filtered::<(
                &TilePos,
                &hex_core::HexSpan,
                &hex_core::SubstanceId,
                &hex_core::Headroom,
            ), With<hex_core::HexTile>>();
            Footing::from_tiles(
                tiles.iter(world),
                world.resource::<SubstanceTable>(),
                body,
                None,
            )
        };
        let party = footing
            .at(party_position)
            .expect("the shipped player anchor should be live footing");
        let hostile = footing
            .at(hostile_position)
            .expect("the shipped hostile anchor should be live footing");
        let from_party = Reach::from(party, &footing, None);
        let approach = from_party
            .path_to(conflict_position)
            .expect("party cannot traverse the complete cave entry connector");
        let conflict = *approach
            .last()
            .expect("the route to the conflict anchor should not be empty");
        let to_conflict = Reach::from(conflict, &footing, None);
        assert!(
            app.world()
                .resource::<InteriorRegions>()
                .get(conflict.pos)
                .is_some(),
            "the entry route never reached a covered cave floor"
        );
        assert!(
            to_conflict.cost(party.pos).is_some(),
            "party cannot walk back from the cave entry connector"
        );

        // Cover both lanes, not only the deterministic shortest path chosen for the
        // command below. A two-step detour admits the parallel ribbon while excluding
        // the deeper chamber network.
        let shortest_steps = from_party
            .cost(conflict.pos)
            .expect("the conflict anchor should have a forward cost");
        let entry_envelope: Vec<_> = {
            let interiors = app.world().resource::<InteriorRegions>();
            from_party
                .surfaces()
                .filter(|surface| interiors.get(surface.pos).is_some())
                .filter(|surface| {
                    from_party
                        .cost(surface.pos)
                        .zip(to_conflict.cost(surface.pos))
                        .is_some_and(|(from_start, to_end)| {
                            from_start.saturating_add(to_end) <= shortest_steps.saturating_add(2)
                        })
                })
                .collect()
        };
        assert!(
            entry_envelope
                .iter()
                .any(|surface| !approach.contains(surface)),
            "the cave safety envelope did not include the parallel entrance lane"
        );
        let combat = app.world().resource::<CombatSettings>();
        for surface in &entry_envelope {
            assert!(
                !either_in_reach(
                    surface.pos,
                    hostile.pos,
                    combat.engage_range,
                    combat.levels_per_bonus_range,
                ),
                "hostile at {:?} can start combat through rock while the party is still on \
                 entrance surface {:?}",
                hostile.pos,
                surface.pos
            );
        }

        // Remove presentation timing so the headless app reconciles every route
        // waypoint on its next update. Engagement still consumes those exact
        // MovementCrossings, which is the production path that exposed this bug.
        app.world_mut().remove_resource::<PlayerSettings>();
        let walk = |app: &mut App, path: Vec<TilePos>| {
            app.world_mut()
                .resource_mut::<CommandQueue>()
                .push(IssuedCommand {
                    seat: PlayerSeat(0),
                    command: GameCommand::MoveAlong {
                        unit: player_unit,
                        path,
                    },
                });
            for _ in 0..4 {
                app.update();
            }
        };

        walk(
            &mut app,
            approach.iter().map(|surface| surface.pos).collect(),
        );
        assert_eq!(standing_pos::<Player>(&mut app), Some(conflict_position));
        assert_eq!(
            *app.world().resource::<State<Mode>>().get(),
            Mode::Exploring
        );

        walk(
            &mut app,
            approach.iter().rev().map(|surface| surface.pos).collect(),
        );
        assert_eq!(standing_pos::<Player>(&mut app), Some(party_position));
        assert_eq!(
            *app.world().resource::<State<Mode>>().get(),
            Mode::Exploring
        );
    }

    /// Sim seeds ride the same install path as the map seed, and the same
    /// launch always deals the same seeds — the precondition for replays.
    #[test]
    fn sim_seeds_install_deterministically_while_loading() {
        let procedural_index = library()
            .scenarios
            .iter()
            .position(|scenario| scenario.generation_seed.is_some())
            .expect("the shipped library should contain a generated scenario");

        let mut app = test_app();
        choose(&mut app, procedural_index);
        use super::SimSeeds;

        let first = *app
            .world()
            .get_resource::<SimSeeds>()
            .expect("loading should install the sim seeds");

        let mut relaunch = test_app();
        choose(&mut relaunch, procedural_index);
        let second = *relaunch
            .world()
            .get_resource::<SimSeeds>()
            .expect("loading should install the sim seeds");

        assert_eq!(first, second, "the same launch must deal the same seeds");
        assert_ne!(
            first.world, first.ai_flavor,
            "the three streams must be decorrelated"
        );
        assert_ne!(first.ai_flavor, first.cosmetic);
    }

    /// Different scenarios must not share a seed by accident.
    #[test]
    fn different_scenarios_deal_different_sim_seeds() {
        use super::sim_seeds_for;

        let seeds_a = sim_seeds_for("The Crossing", None);
        let seeds_b = sim_seeds_for("Procedural Hills", None);
        assert_ne!(seeds_a, seeds_b);

        let reseeded = sim_seeds_for("Procedural Hills", Some(ResolvedMapSeed(42)));
        assert_ne!(seeds_b, reseeded, "the resolved map seed feeds the fold");
    }
}
