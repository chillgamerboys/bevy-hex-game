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

use std::sync::Arc;

use bevy::prelude::*;
use hex_assets::{
    choose_settings, Encounter, EncounterPlacement, FormationCatalog, LightingSettings, Scenario,
    SelectSettings, SettingsRegistry, SubstanceTable, CONFIG_EXTENSIONS,
};
use hex_core::{
    GameplaySetup, GameplaySetupFailure, Headroom, HexCoord, HexSpan, HexTile, InteriorRegions,
    MapAnchorId, MapAnchors, MapViewHint, PartyFormation, ResolvedMapSeed, Screen, Sextant,
    SimSeeds, SpecialMovementRegions, SubstanceId, TerrainReady, TilePos, TraversalBlockers,
    UnitId,
};
use hex_map::{MapSettings, TerrainSettings};
use hex_units::{
    plan_formation_move_with_occupancy, route_with_occupancy, Body, Faction, Footing,
    FormationMember, Player, StandsOn, UnitOccupancy,
};
use hex_world::TimeOfDay;

use crate::screens::CreatorSandboxReturn;

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
                stage_crystal_ascent_showcase_party
                    .after(GameplaySetup::Actors)
                    .before(GameplaySetup::Restore),
                stage_crystal_mountain_showcase_party
                    .after(GameplaySetup::Actors)
                    .before(GameplaySetup::Restore),
                finalize_gameplay_setup.in_set(GameplaySetup::Finalize),
            ),
        )
        .add_systems(
            Update,
            validate_gameplay_lighting_contract.run_if(in_state(Screen::Gameplay)),
        )
        .add_systems(OnExit(Screen::Gameplay), clear_session_resources);
}

const CRYSTAL_ASCENT_SHOWCASE: &str = "Crystal Ascent Showcase";
const CRYSTAL_ASCENT_SCENARIO: &str = "Crystal Ascent";
const CRYSTAL_ASCENT_SITE_RADIUS: u32 = 32;
const CRYSTAL_ASCENT_LOWER_ENTRY: &str = "crystal_ascent.lower_entry";
const CRYSTAL_ASCENT_BOTTOM_CHAMBER: &str = "crystal_ascent.bottom_chamber";
const CRYSTAL_MOUNTAIN_SHOWCASE: &str = "Crystal Mountain Showcase";
const CRYSTAL_MOUNTAIN_RADIUS: u32 = 77;
const CRYSTAL_MOUNTAIN_FOOT_APRON: &str = "crystal_mountain.foot_apron";
const CRYSTAL_MOUNTAIN_TUNNEL_MOUTH: &str = "crystal_mountain.tunnel_mouth";

/// Stages the shipped Crystal Ascent party on its exact exterior terminal.
///
/// Encounter formations deliberately express a *region*, not an exact multi-unit
/// footprint, and their schema has no travel-facing field. That is right for ordinary
/// generated maps, but this showcase promises something narrower: three stable party
/// members on the landmark's four-wide apron, looking through the entrance. Resolve
/// that one launch after generic actors exist and before save restoration/perception.
/// A pending save therefore remains authoritative, while a fresh launch never exposes
/// the generic formation's temporary positions to gameplay systems.
///
/// Other non-combat maps reuse the same encounter roster, so the active scenario is
/// part of this adapter's authority gate. Only Crystal Ascent owns landmark-specific
/// apron staging and its fail-closed anchor checks.
fn stage_crystal_ascent_showcase_party(
    mut commands: Commands,
    active: Option<Res<ActiveScenario>>,
    encounter: Option<Res<Encounter>>,
    anchors: Option<Res<MapAnchors>>,
    interiors: Option<Res<InteriorRegions>>,
    table: Option<Res<SubstanceTable>>,
    blockers: Option<Res<TraversalBlockers>>,
    failure: Option<Res<GameplaySetupFailure>>,
    mut formation: Option<ResMut<PartyFormation>>,
    players: Query<(Entity, &UnitId, &Body), With<Player>>,
    tiles: Query<(&TilePos, &HexSpan, &SubstanceId, &Headroom), With<HexTile>>,
) {
    let Some(active) = active else { return };
    let Some(encounter) = encounter else { return };
    if active.0.scenario.name != CRYSTAL_ASCENT_SCENARIO
        || encounter.name != CRYSTAL_ASCENT_SHOWCASE
        || failure.is_some()
    {
        return;
    }

    let result = (|| {
        let anchors = anchors
            .as_deref()
            .ok_or("the generated map published no anchors")?;
        let interiors = interiors
            .as_deref()
            .ok_or("the generated map published no interior regions")?;
        let table = table
            .as_deref()
            .ok_or("the substance table is unavailable")?;
        let formation = formation
            .as_deref_mut()
            .ok_or("party formation state is unavailable")?;
        let lower_entry = anchors
            .get(&MapAnchorId::from(CRYSTAL_ASCENT_LOWER_ENTRY))
            .ok_or("the lower-entry anchor is missing")?;
        let bottom_chamber = anchors
            .get(&MapAnchorId::from(CRYSTAL_ASCENT_BOTTOM_CHAMBER))
            .ok_or("the bottom-chamber anchor is missing")?;

        let mut members = players
            .iter()
            .map(|(entity, unit, body)| (entity, *unit, *body))
            .collect::<Vec<_>>();
        members.sort_by_key(|(_, unit, _)| *unit);
        if members.len() != 3 {
            return Err("the standard showcase party did not spawn exactly three members");
        }
        let body = members
            .first()
            .map(|(_, _, body)| *body)
            .ok_or("the standard showcase party is empty")?;
        if members
            .iter()
            .any(|(_, _, member_body)| *member_body != body)
        {
            return Err("the standard showcase party no longer shares one staging footprint");
        }

        let footing = Footing::from_tiles(tiles.iter(), table, body, blockers.as_deref());
        let mut apron = lower_entry
            .coord
            .within_radius(2)
            .into_iter()
            .filter(|coord| coord.distance(HexCoord::ORIGIN) == CRYSTAL_ASCENT_SITE_RADIUS)
            .map(|coord| TilePos::new(coord, lower_entry.level))
            .filter(|position| interiors.get(*position).is_none())
            .filter_map(|position| footing.at(position))
            .collect::<Vec<_>>();
        apron
            .sort_by_key(|standing| (standing.pos.coord.distance(lower_entry.coord), standing.pos));
        apron.dedup_by_key(|standing| standing.pos);
        if apron.len() != 4 || apron.iter().all(|standing| standing.pos != lower_entry) {
            return Err("the lower exterior terminal is not an exact four-wide standable apron");
        }

        let facing = Sextant::ALL
            .into_iter()
            .min_by_key(|direction| {
                (
                    lower_entry
                        .coord
                        .neighbor(*direction)
                        .distance(bottom_chamber.coord),
                    *direction,
                )
            })
            .ok_or("the entrance has no inward-facing sextant")?;
        for ((entity, _, _), standing) in members.into_iter().zip(apron) {
            commands.entity(entity).insert((
                StandsOn(standing),
                Transform::from_translation(standing.world_position()),
            ));
        }
        formation.facing = facing;
        Ok::<(), &'static str>(())
    })();

    if let Err(detail) = result {
        let reason = format!("Crystal Ascent showcase staging failed: {detail}.");
        error!("{reason}");
        commands.insert_resource(GameplaySetupFailure::new(reason));
    }
}

/// Stages the Crystal Mountain party in a group-safe exterior footprint.
///
/// Nearest-first generic staging can give three individually valid cells whose routes
/// conflict when the default Compact group enters the boundary mouth. Keep the anchor
/// exact, enumerate only nearby exterior footing, and retain the first deterministic
/// placement/facing whose production formation planner proves atomic. The camera walk
/// switches to Solo only after that ordinary Group move completes.
fn stage_crystal_mountain_showcase_party(
    mut commands: Commands,
    encounter: Option<Res<Encounter>>,
    anchors: Option<Res<MapAnchors>>,
    interiors: Option<Res<InteriorRegions>>,
    table: Option<Res<SubstanceTable>>,
    formations: Option<Res<FormationCatalog>>,
    blockers: Option<Res<TraversalBlockers>>,
    failure: Option<Res<GameplaySetupFailure>>,
    mut formation: Option<ResMut<PartyFormation>>,
    players: Query<(Entity, &UnitId, &Body), With<Player>>,
    tiles: Query<(&TilePos, &HexSpan, &SubstanceId, &Headroom), With<HexTile>>,
) {
    let Some(encounter) = encounter else { return };
    if encounter.name != CRYSTAL_MOUNTAIN_SHOWCASE || failure.is_some() {
        return;
    }

    let result = (|| {
        let anchors = anchors
            .as_deref()
            .ok_or("the generated map published no anchors")?;
        let interiors = interiors
            .as_deref()
            .ok_or("the generated map published no interior regions")?;
        let table = table
            .as_deref()
            .ok_or("the substance table is unavailable")?;
        let formations = formations
            .as_deref()
            .ok_or("the formation catalog is unavailable")?;
        let formation = formation
            .as_deref_mut()
            .ok_or("party formation state is unavailable")?;
        let foot_apron = anchors
            .get(&MapAnchorId::from(CRYSTAL_MOUNTAIN_FOOT_APRON))
            .ok_or("the Crystal Mountain foot-apron anchor is missing")?;
        let tunnel_mouth = anchors
            .get(&MapAnchorId::from(CRYSTAL_MOUNTAIN_TUNNEL_MOUTH))
            .ok_or("the Crystal Mountain tunnel-mouth anchor is missing")?;

        let mut members = players
            .iter()
            .map(|(entity, unit, body)| (entity, *unit, *body))
            .collect::<Vec<_>>();
        members.sort_by_key(|(_, unit, _)| *unit);
        if members.len() != 3 {
            return Err("the standard showcase party did not spawn exactly three members");
        }
        let body = members
            .first()
            .map(|(_, _, body)| *body)
            .ok_or("the standard showcase party is empty")?;
        if members
            .iter()
            .any(|(_, _, member_body)| *member_body != body)
        {
            return Err("the standard showcase party no longer shares one staging footprint");
        }

        let footing = Arc::new(Footing::from_tiles(
            tiles.iter(),
            table,
            body,
            blockers.as_deref(),
        ));
        let selected = footing
            .at(foot_apron)
            .ok_or("the foot-apron anchor is not standable")?;
        let selected_unit = members
            .first()
            .map(|(_, unit, _)| *unit)
            .ok_or("the standard showcase party is empty")?;
        let preset = formations
            .get(&formation.preset)
            .ok_or("the selected formation preset is unavailable")?;
        let anchor_slot = preset
            .anchor()
            .ok_or("the selected formation preset has no anchor")?;
        let anchor = formation
            .assignments
            .iter()
            .find_map(|(&unit, &slot)| (slot == anchor_slot).then_some(unit))
            .ok_or("the selected formation has no assigned anchor")?;
        if anchor != selected_unit {
            return Err("the stable foot-apron explorer is not the formation anchor");
        }
        let destination = footing
            .at(tunnel_mouth)
            .ok_or("the tunnel-mouth anchor is not standable")?;
        let external_occupancy = UnitOccupancy::default();
        let anchor_path =
            route_with_occupancy(selected, destination, &footing, &external_occupancy, anchor)
                .ok_or("the formation anchor cannot route into the tunnel mouth")?;

        let mut candidates = foot_apron
            .coord
            .within_radius(4)
            .into_iter()
            .filter(|coord| coord.distance(HexCoord::ORIGIN) == CRYSTAL_MOUNTAIN_RADIUS)
            .map(|coord| TilePos::new(coord, foot_apron.level))
            .filter(|position| *position != foot_apron)
            .filter(|position| interiors.get(*position).is_none())
            .filter_map(|position| footing.at(position))
            .collect::<Vec<_>>();
        candidates.sort_by_key(|standing| standing.pos);
        candidates.dedup_by_key(|standing| standing.pos);

        let mut directions = Sextant::ALL;
        directions.sort_by_key(|direction| {
            (
                foot_apron
                    .coord
                    .neighbor(*direction)
                    .distance(tunnel_mouth.coord),
                *direction,
            )
        });
        let mut accepted = None;
        'placements: for direction in directions {
            for (second_index, second) in candidates.iter().enumerate() {
                for (third_index, third) in candidates.iter().enumerate() {
                    if second_index == third_index {
                        continue;
                    }
                    let staged = [selected, *second, *third];
                    let mut candidate_formation = formation.clone();
                    candidate_formation.facing = direction;
                    let planned_members = members
                        .iter()
                        .zip(staged)
                        .map(|((_, unit, _), standing)| FormationMember {
                            unit: *unit,
                            standing,
                            footing: Arc::clone(&footing),
                        })
                        .collect::<Vec<_>>();
                    if plan_formation_move_with_occupancy(
                        preset,
                        &candidate_formation,
                        &anchor_path,
                        planned_members,
                        &external_occupancy,
                    )
                    .is_ok()
                    {
                        accepted = Some((direction, staged));
                        break 'placements;
                    }
                }
            }
        }
        let Some((facing, staged)) = accepted else {
            return Err("no nearby exterior staging footprint can enter the mouth atomically");
        };

        for ((entity, _, _), standing) in members.into_iter().zip(staged) {
            commands.entity(entity).insert((
                StandsOn(standing),
                Transform::from_translation(standing.world_position()),
            ));
        }
        formation.facing = facing;
        Ok::<(), &'static str>(())
    })();

    if let Err(detail) = result {
        let reason = format!("Crystal Mountain showcase staging failed: {detail}.");
        error!("{reason}");
        commands.insert_resource(GameplaySetupFailure::new(reason));
    }
}

/// The exact scenario and resolved seed frozen by its typed launch owner.
///
/// The library can hot-reload between a Campaign, Sandbox, save, retry, review, or
/// test request and the next frame's `OnEnter(Loading)`. Carrying the entry itself
/// keeps a reorder or removal from changing that request; carrying the seed prevents
/// a later regeneration from changing an already-started load.
#[derive(Resource, Debug, Clone, PartialEq)]
pub(super) struct ScenarioToLoad {
    pub(super) scenario: Scenario,
    pub(super) resolved_seed: Option<ResolvedMapSeed>,
    /// Frozen creator/fixture roster. Normal authored scenarios leave this absent.
    pub(super) encounter_override: Option<Encounter>,
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
    creator_return: Option<Res<CreatorSandboxReturn>>,
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
        next.set(setup_failure_destination(creator_return.is_some()));
        error!("loading entered without a typed launch request; returning to Main Menu");
        return;
    };
    let scenario = pending.scenario.clone();
    let resolved_seed = pending.resolved_seed;
    commands.insert_resource(ActiveScenario(ScenarioToLoad {
        scenario: scenario.clone(),
        resolved_seed,
        encounter_override: pending.encounter_override.clone(),
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
    if let Some(encounter) = pending.encounter_override.clone() {
        commands.insert_resource(encounter);
    } else {
        choose_settings::<Encounter>(
            &mut commands,
            &asset_server,
            &mut registry,
            &scenario.encounter,
        );
    }
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
    active: Option<Res<ActiveScenario>>,
    status: Option<Res<ScenarioContractStatus>>,
    creator_return: Option<Res<CreatorSandboxReturn>>,
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

    let allow_resolved_surfaces = active
        .as_deref()
        .is_some_and(|active| active.0.encounter_override.is_some());
    let contract_error = scenario_contract_error_for_launch(
        &map,
        &encounter,
        seed.as_deref(),
        allow_resolved_surfaces,
    )
    .or_else(|| lighting.resolve(time_override.0).err());
    if let Some(reason) = contract_error {
        error!("selected scenario is incompatible with its world: {reason}");
        commands.insert_resource(ScenarioContractStatus::Invalid);
        commands.insert_resource(GameplaySetupFailure::new(format!(
            "The selected scenario is incompatible with its world: {reason}."
        )));
        next.set(setup_failure_destination(creator_return.is_some()));
    } else {
        commands.insert_resource(ScenarioContractStatus::Ready);
    }
}

/// Resolves the scenario/profile default before any presentation system needs it.
fn initialize_time_of_day(
    mut commands: Commands,
    lighting: Res<LightingSettings>,
    time_override: Res<ScenarioTimeOverride>,
    creator_return: Option<Res<CreatorSandboxReturn>>,
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
            next.set(setup_failure_destination(creator_return.is_some()));
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
    creator_return: Option<Res<CreatorSandboxReturn>>,
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
    next.set(setup_failure_destination(creator_return.is_some()));
}

/// Whether the chosen encounter can be placed on the world the scenario named.
///
/// Two files, each valid alone: an encounter cannot see whether its terrain is generated,
/// and a world cannot see who is standing on it. Every entry is checked rather than a
/// side at a time — one authored coordinate in an otherwise anchored roster is the same
/// bug, and it would otherwise only surface as one unit missing from the fight.
#[cfg(test)]
fn scenario_contract_error(
    map: &MapSettings,
    encounter: &Encounter,
    seed: Option<&ResolvedMapSeed>,
) -> Option<String> {
    scenario_contract_error_for_launch(map, encounter, seed, false)
}

fn scenario_contract_error_for_launch(
    map: &MapSettings,
    encounter: &Encounter,
    seed: Option<&ResolvedMapSeed>,
    allow_resolved_surfaces: bool,
) -> Option<String> {
    match &map.terrain {
        TerrainSettings::Procedural(_) => {
            if seed.is_none() {
                return Some("procedural terrain has no resolved generation seed".to_owned());
            }
            for unit in encounter.entries() {
                let resolved_override = allow_resolved_surfaces
                    && matches!(unit.placement, EncounterPlacement::Surface(_));
                if !unit.placement.is_generated() && !resolved_override {
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
    creator_return: Option<Res<CreatorSandboxReturn>>,
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
    next.set(setup_failure_destination(creator_return.is_some()));
}

fn setup_failure_destination(has_creator_return: bool) -> Screen {
    if has_creator_return {
        Screen::Sandbox
    } else {
        Screen::Title
    }
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
        let standing = units.iter().filter(|spawned| **spawned == faction).count();
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
pub(crate) mod tests {
    use std::collections::{BTreeMap, BTreeSet};
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
        ObjectBlueprint, ObjectCatalogFile, ObjectInstance, PerceptionSettings, PlayerSettings,
        Roster, RosterEntry, RuntimeArtCatalog, ScenarioLibrary, SettingsRegistry, SpellBook,
        SpellFile, SubstanceFile, SubstanceTable, VoxelStyleCatalog,
    };
    use hex_combat::{
        AiDecisionTraces, CombatSummary, EncounterResolution, TurnOrder, MAX_AI_DECISION_TRACES,
        MAX_COMBAT_SUMMARY_DETAILS,
    };
    use hex_core::{
        AppSystems, AuthoredObjectVoxelRun, AuthoredObjectVoxelRuns, Busy, CommandQueue,
        ControlOwner, ExteriorIllumination, GameCommand, GameplayLight, GameplaySetup,
        GameplaySetupFailure, Headroom, HexCoord, HexGrid, HexSpan, HexTile, IlluminationLevel,
        InteriorRegions, IssuedCommand, KnowledgeState, LatticeCoord, LightDomain,
        LocalMapKnowledge, MapAnchorId, MapAnchors, MapViewHint, Mode, PartyFormation,
        PartyMovementMode, PausableSystems, Pause, PendingDecision, PerceptionSystems, PlayerSeat,
        ResolvedMapSeed, Screen, Sextant, SpecialMovementRegion, SpecialMovementRegions,
        SubstanceId, TerrainReady, TilePos, TraversalBlockers, TraversalProfile, Turn, UnitId,
    };
    use hex_lattice::{LatticeSpec, LatticeState};
    use hex_map::{
        GenerationReport, MapSettings, ProceduralRecipeMetrics, TerrainSettings, VoxelMap,
    };
    use hex_perception::{
        can_observe, can_observe_with_authored_objects, FactionMapKnowledge, ResolvedIllumination,
    };
    use hex_units::{
        either_in_reach, plan_formation_move, plan_formation_move_with_occupancy,
        route_with_occupancy, Archetype, AuthoredObjectOccupancy, Body, Downed, Enemy, Faction,
        Footing, FormationMember, Player, Reach, StandsOn, TerrainOccupancy, UnitOccupancy,
    };
    use hex_world::TimeOfDay;

    #[cfg(feature = "test-support")]
    use hex_assets::CameraSettings;
    #[cfg(feature = "test-support")]
    use hex_core::{
        CameraFocusTarget, CutawayOccluder, PresentationOcclusion, PresentationOcclusionReason,
        RunBottom, TreeOccluder,
    };
    #[cfg(feature = "test-support")]
    use hex_units::Standing;

    use super::{
        clear_session_resources, finalize_gameplay_setup, initialize_time_of_day,
        scenario_contract_error, scenario_contract_error_for_launch, setup_failure_destination,
        stage_crystal_ascent_showcase_party, stage_crystal_mountain_showcase_party,
        validate_gameplay_lighting_contract, validate_loaded_scenario, ActiveScenario,
        ScenarioContractStatus, ScenarioTimeOverride, ScenarioToLoad,
    };

    fn library() -> ScenarioLibrary {
        ron::from_str(include_str!("../../../assets/config/scenarios.ron"))
            .expect("the shipped scenarios should parse")
    }

    #[test]
    fn setup_failures_preserve_a_creator_origin_via_sandbox() {
        assert_eq!(setup_failure_destination(false), Screen::Title);
        assert_eq!(setup_failure_destination(true), Screen::Sandbox);
    }

    fn assets_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets")
    }

    fn runtime_art_catalog(palette: &ArtPalette) -> RuntimeArtCatalog {
        let styles: VoxelStyleCatalog =
            ron::from_str(include_str!("../../../assets/art/voxel_styles.ron"))
                .expect("the shipped voxel styles should deserialize");
        let manifest: ObjectCatalogFile =
            ron::from_str(include_str!("../../../assets/art/object_catalog.ron"))
                .expect("the shipped object catalog should deserialize");
        let mut objects = BTreeMap::new();
        for id in manifest.ids() {
            let path = assets_dir()
                .join("art/objects")
                .join(format!("{}.ron", id.as_str()));
            let source = fs::read_to_string(&path).unwrap_or_else(|error| {
                panic!(
                    "object catalog entry '{}' could not be read from {}: {error}",
                    id.as_str(),
                    path.display()
                )
            });
            let blueprint: ObjectBlueprint = ron::from_str(&source).unwrap_or_else(|error| {
                panic!(
                    "object catalog entry '{}' does not deserialize: {error}",
                    id.as_str()
                )
            });
            let previous = objects.insert(blueprint.id.clone(), blueprint);
            assert!(
                previous.is_none(),
                "object catalog resolved duplicate blueprint '{}'",
                id.as_str()
            );
        }
        RuntimeArtCatalog::from_sources(palette, &styles, &manifest, objects)
            .expect("the shipped runtime art graph should resolve")
    }

    /// Independently projects every occupied cell in a shipped blueprint into the
    /// exact world columns of its runtime instance. The production Crystal Ascent
    /// adapter compacts the same source cells before publishing them; keeping this
    /// derivation in the acceptance test prevents a self-fulfilling component check.
    fn expected_object_occupancy(
        blueprint: &ObjectBlueprint,
        instance: &ObjectInstance,
    ) -> AuthoredObjectOccupancy {
        let runs = blueprint.placements.iter().map(|placement| {
            let rotated = instance
                .rotation()
                .rotate_voxel(placement.position, blueprint.origin)
                .expect("shipped object coordinates should rotate without overflow");
            let q = instance
                .origin()
                .coord
                .x()
                .checked_add(
                    rotated
                        .q
                        .checked_sub(blueprint.origin.q)
                        .expect("shipped local q offset should be exact"),
                )
                .expect("shipped world q should be exact");
            let r = instance
                .origin()
                .coord
                .y()
                .checked_add(
                    rotated
                        .r
                        .checked_sub(blueprint.origin.r)
                        .expect("shipped local r offset should be exact"),
                )
                .expect("shipped world r should be exact");
            let level = instance
                .origin()
                .level
                .checked_add(
                    rotated
                        .level
                        .checked_sub(blueprint.origin.level)
                        .expect("shipped local level offset should be exact"),
                )
                .expect("shipped world level should be exact");
            let position = TilePos::new(HexCoord::from_axial(q, r), level);
            AuthoredObjectVoxelRun::new(position, position.level)
        });
        AuthoredObjectOccupancy::from_runs(runs)
            .expect("the shipped object's projected cells should form valid runs")
    }

    /// Proves and returns the live heart instance, source runs, and exact authority.
    pub(crate) fn crystal_heart_occupancy_snapshot(
        app: &mut App,
    ) -> (
        ObjectInstance,
        AuthoredObjectVoxelRuns,
        AuthoredObjectOccupancy,
    ) {
        let (instance, published_runs) = {
            let world = app.world_mut();
            let mut objects = world.query::<(&ObjectInstance, &AuthoredObjectVoxelRuns)>();
            let mut hearts = objects.iter(world).filter(|(instance, _)| {
                instance.object_id().as_str() == "prop/crystal-cathedral-heart"
            });
            let (instance, runs) = hearts
                .next()
                .expect("Crystal Ascent should publish one occupied cathedral heart");
            assert!(
                hearts.next().is_none(),
                "Crystal Ascent published two hearts"
            );
            (instance.clone(), runs.clone())
        };
        let blueprint = app
            .world()
            .resource::<RuntimeArtCatalog>()
            .object(instance.object_id())
            .expect("the live heart should resolve through the shipped art catalog");
        let expected = expected_object_occupancy(blueprint, &instance);
        let published = AuthoredObjectOccupancy::from_runs(published_runs.iter())
            .expect("the heart component should contain valid compact runs");
        assert_eq!(
            published, expected,
            "heart runs diverged from its blueprint"
        );
        assert_eq!(
            app.world().resource::<AuthoredObjectOccupancy>(),
            &expected,
            "authoritative occupancy diverged from the heart component"
        );
        let published_cells = published_runs
            .iter()
            .map(|run| {
                usize::try_from(run.top.level.saturating_sub(run.bottom).saturating_add(1))
                    .expect("shipped heart run length should fit usize")
            })
            .sum::<usize>();
        assert_eq!(
            published_cells,
            blueprint.placements.len(),
            "heart compaction lost or invented an authored structural cell"
        );
        (instance, published_runs, expected)
    }

    /// Finds one chamber pair blocked only by the shipped heart's seven-ray volume.
    pub(crate) fn crystal_heart_blocked_sight_pair(
        app: &App,
        heart: TilePos,
    ) -> (TilePos, TilePos) {
        let illumination = app.world().resource::<ResolvedIllumination>();
        let terrain = app.world().resource::<TerrainOccupancy>();
        let authored = app.world().resource::<AuthoredObjectOccupancy>();
        let profile = app
            .world()
            .resource::<PerceptionSettings>()
            .active_profile();
        let candidates = illumination
            .iter()
            .map(|(position, _)| position)
            .filter(|position| {
                position.level == heart.level.saturating_sub(1)
                    && (5..=7).contains(&position.coord.distance(heart.coord))
            })
            .collect::<Vec<_>>();
        for observer in candidates.iter().copied() {
            for target in candidates.iter().copied() {
                if observer == target {
                    continue;
                }
                if can_observe(observer, target, illumination, profile, terrain)
                    && !can_observe_with_authored_objects(
                        observer,
                        target,
                        illumination,
                        profile,
                        terrain,
                        authored,
                    )
                {
                    return (observer, target);
                }
            }
        }
        panic!("the shipped heart should block at least one otherwise-clear chamber sight bundle");
    }

    #[cfg(feature = "test-support")]
    fn crystal_mountain_presentation_app() -> App {
        let mut app = unfinished_procedural_gameplay_app("Crystal Mountain", false);
        let settings: CameraSettings =
            ron::from_str(include_str!("../../../assets/config/camera.ron"))
                .expect("the shipped camera settings should deserialize");
        settings
            .validate()
            .expect("the shipped camera settings should remain valid");
        app.insert_resource(settings);
        app.add_plugins(bevy::window::WindowPlugin {
            primary_window: None,
            ..default()
        });
        app.add_plugins(bevy::transform::TransformPlugin);
        app.add_plugins((
            hex_world::test_support::headless_camera_plugin,
            crate::fog::plugin,
        ));
        hex_world::install_full_cutaway_review_override(&mut app);
        finish_test_app(app)
    }

    #[cfg(feature = "test-support")]
    fn focus_crystal_mountain_interior(app: &mut App) -> TilePos {
        let target = app
            .world()
            .resource::<MapAnchors>()
            .get(&MapAnchorId::from("crystal_mountain.midpoint"))
            .expect("Crystal Mountain should publish its tunnel midpoint");
        let span = {
            let world = app.world_mut();
            let mut surfaces =
                world.query_filtered::<(&TilePos, &HexSpan, &Headroom), With<HexTile>>();
            surfaces
                .iter(world)
                .find(|(position, _, headroom)| **position == target && headroom.0 > 0)
                .map(|(_, span, _)| *span)
                .expect("the tunnel midpoint should be an exposed published surface")
        };
        let standing = Standing { pos: target, span };
        {
            let world = app.world_mut();
            let player = {
                let mut players = world.query_filtered::<(Entity, &UnitId), With<Player>>();
                players
                    .iter(world)
                    .min_by_key(|(_, unit)| **unit)
                    .map(|(entity, _)| entity)
                    .expect("Crystal Mountain should retain its player party")
            };
            world.entity_mut(player).insert((
                StandsOn(standing),
                Transform::from_translation(standing.world_position()),
                CameraFocusTarget::new(target),
            ));
        }
        app.update();

        let world = app.world_mut();
        let mut focused = world.query_filtered::<&CameraFocusTarget, With<Player>>();
        assert_eq!(
            focused
                .single(world)
                .expect("the review fixture should publish one exact camera focus")
                .surface,
            target,
            "review cutaway must consume the relocated actor's exact interior surface"
        );
        target
    }

    #[cfg(feature = "test-support")]
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct CrystalMountainRuntimeSnapshot {
        map_fingerprint: u64,
        terrain: TerrainOccupancy,
        authored: AuthoredObjectOccupancy,
        blocked_sight_pair: (TilePos, TilePos),
        fog_surfaces: BTreeSet<TilePos>,
        cutaway_roofs: BTreeSet<TilePos>,
        cutaway_tree_roots: BTreeSet<TilePos>,
    }

    #[cfg(feature = "test-support")]
    fn crystal_mountain_runtime_snapshot(app: &mut App) -> CrystalMountainRuntimeSnapshot {
        let target = focus_crystal_mountain_interior(app);
        let expected_terrain = {
            let world = app.world_mut();
            let mut runs = world.query_filtered::<(&TilePos, &RunBottom), With<HexTile>>();
            TerrainOccupancy::from_runs(
                runs.iter(world)
                    .map(|(position, bottom)| (*position, *bottom)),
            )
            .expect("Crystal Mountain should publish only valid terrain runs")
        };
        let terrain = app.world().resource::<TerrainOccupancy>().clone();
        assert_eq!(
            terrain, expected_terrain,
            "runtime terrain authority must derive from every composed material run"
        );
        assert!(terrain.contains(target));

        let (heart, _, authored) = crystal_heart_occupancy_snapshot(app);
        let blocked_sight_pair = crystal_heart_blocked_sight_pair(app, heart.origin());
        let active_region = app
            .world()
            .resource::<InteriorRegions>()
            .get(target)
            .expect("the tunnel midpoint should belong to the combined interior");
        let projected_regions = {
            let world = app.world_mut();
            let mut cutaways = world.query::<&CutawayOccluder>();
            cutaways
                .iter(world)
                .map(|cutaway| cutaway.0)
                .collect::<Vec<_>>()
        };
        assert!(
            projected_regions.contains(&active_region),
            "the combined interior omitted its exact rendered roof projection: active={active_region:?}, projected={projected_regions:?}"
        );
        let cutaway_roofs = {
            let world = app.world_mut();
            let mut roofs = world.query_filtered::<(
                &TilePos,
                &CutawayOccluder,
                &PresentationOcclusion,
                Option<&Visibility>,
            ), With<HexTile>>();
            roofs
                .iter(world)
                .filter(|(_, cutaway, _, _)| cutaway.0 == active_region)
                .map(|(position, _, occlusion, visibility)| {
                    assert!(occlusion.contains(PresentationOcclusionReason::InteriorCutaway));
                    assert_eq!(visibility, Some(&Visibility::Hidden));
                    *position
                })
                .collect::<BTreeSet<_>>()
        };
        assert!(
            !cutaway_roofs.is_empty(),
            "the combined tunnel/ascent interior should hide authored roof runs"
        );
        let interior_regions = app.world().resource::<InteriorRegions>().clone();
        let cutaway_tree_roots = {
            let world = app.world_mut();
            let mut trees = world.query::<(
                &TreeOccluder,
                Option<&PresentationOcclusion>,
                Option<&Visibility>,
            )>();
            let mut all_roots = BTreeSet::new();
            let mut hidden_roots = BTreeSet::new();
            for (tree, occlusion, visibility) in trees.iter(world) {
                all_roots.insert(tree.0);
                if interior_regions.roof_region(tree.0) == Some(active_region) {
                    assert!(occlusion.is_some_and(|occlusion| {
                        occlusion.contains(PresentationOcclusionReason::InteriorCutaway)
                    }));
                    assert_eq!(visibility, Some(&Visibility::Hidden));
                    hidden_roots.insert(tree.0);
                }
            }
            assert!(
                !all_roots.is_empty(),
                "Crystal Mountain should retain its generated tree roots"
            );
            hidden_roots
        };
        assert_eq!(
            app.world().resource::<LocalMapKnowledge>().state(target),
            KnowledgeState::Observed,
            "relocating the real observer should rebuild local visibility at the tunnel midpoint"
        );
        let fog_surfaces = crate::fog::fog_overlay_positions(app.world_mut());
        assert!(!fog_surfaces.is_empty());
        assert!(
            !fog_surfaces.contains(&target),
            "an observed tunnel midpoint must not retain a fog cap"
        );

        CrystalMountainRuntimeSnapshot {
            map_fingerprint: app.world().resource::<GenerationReport>().map_fingerprint,
            terrain,
            authored,
            blocked_sight_pair,
            fog_surfaces,
            cutaway_roofs,
            cutaway_tree_roots,
        }
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

    /// Every encounter a scenario names exists, parses, and rosters its intended sides.
    ///
    /// Same reasoning as the world and lighting checks — the path is a plain string, so
    /// a typo would otherwise be a loading screen that hangs for the one scenario nobody
    /// clicked. `Encounter`'s `Deserialize` runs `validate()`, so this also proves the
    /// roster is *placeable* in the ways a single file can be judged: no empty roster, no
    /// coordinate that is not a hex, no two units sharing one exact surface. The two
    /// Crystal traversal showcases and the three island review maps are approved
    /// non-combat maps; every other scenario must still provide somebody to fight.
    #[test]
    fn every_scenario_names_an_encounter_that_exists_and_parses() {
        for scenario in &library().scenarios {
            let encounter = encounter_of(scenario);
            assert!(
                encounter.unit_count(EncounterFaction::Player) >= 1,
                "scenario {:?} rosters no player units",
                scenario.name
            );
            let hostile_count = encounter.unit_count(EncounterFaction::Hostile);
            if matches!(
                scenario.name.as_str(),
                "Crystal Ascent"
                    | "Crystal Mountain"
                    | "Sandy Islets"
                    | "Wooded Island"
                    | "Ocean Archipelagoes"
            ) {
                assert_eq!(
                    hostile_count, 0,
                    "the non-combat map showcases should remain non-combat"
                );
            } else {
                assert!(
                    hostile_count >= 1,
                    "scenario {:?} rosters nobody to fight",
                    scenario.name
                );
            }
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
    fn procedural_retry_accepts_typed_resolved_surface_overrides_only() {
        let entry = library()
            .scenarios
            .into_iter()
            .find(|scenario| scenario.generation_seed.is_some())
            .expect("the shipped library should include procedural terrain");
        let world_text = fs::read_to_string(assets_dir().join(&entry.world))
            .expect("the procedural world should be readable");
        let world: MapSettings =
            ron::from_str(&world_text).expect("the procedural world should deserialize");
        let exact = duel(
            EncounterPlacement::Surface(TilePos::new(HexCoord::ORIGIN, 2)),
            EncounterPlacement::Surface(TilePos::new(HexCoord::from_axial(1, -1), 3)),
        );
        let seed = ResolvedMapSeed(1);

        assert!(scenario_contract_error(&world, &exact, Some(&seed))
            .is_some_and(|error| error.contains("map anchor")));
        assert_eq!(
            scenario_contract_error_for_launch(&world, &exact, Some(&seed), true),
            None
        );

        let lighting_text = fs::read_to_string(assets_dir().join(&entry.lighting))
            .expect("the procedural lighting should be readable");
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin));
        app.init_state::<Screen>();
        app.insert_resource(SettingsRegistry::default());
        app.insert_resource(world);
        app.insert_resource(exact.clone());
        app.insert_resource(
            ron::from_str::<LightingSettings>(&lighting_text)
                .expect("the procedural lighting should deserialize"),
        );
        app.insert_resource(ScenarioTimeOverride(entry.starting_time_hours));
        app.insert_resource(seed);
        app.insert_resource(ActiveScenario(ScenarioToLoad {
            scenario: entry,
            resolved_seed: Some(seed),
            encounter_override: Some(exact),
        }));
        app.add_systems(Update, validate_loaded_scenario);

        app.update();

        assert_eq!(
            *app.world().resource::<ScenarioContractStatus>(),
            ScenarioContractStatus::Ready
        );
        assert!(!app.world().contains_resource::<GameplaySetupFailure>());
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
    /// under it, which now fails setup and sends the player back to Main Menu —
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

    /// Crystal Ascent is a traversal-and-lighting showcase, not an encounter arena.
    /// Its content freezes the public 144-level recipe and starts the complete shipped
    /// party together at the lower apron without inventing a dummy hostile.
    #[test]
    fn crystal_ascent_showcase_freezes_its_world_and_non_combat_party() {
        let library = library();
        let scenario = library
            .scenarios
            .iter()
            .find(|scenario| scenario.name == "Crystal Ascent")
            .expect("the shipped library should contain Crystal Ascent");

        assert_eq!(
            scenario.world,
            "config/worlds/procedural-crystal-ascent.ron"
        );
        assert_eq!(scenario.generation_seed, Some(1_592_598_566));

        let world_text = fs::read_to_string(assets_dir().join(&scenario.world))
            .expect("the Crystal Ascent world should be readable");
        let world: MapSettings =
            ron::from_str(&world_text).expect("the Crystal Ascent world should parse");
        assert_eq!(world.grid_radius, 40);
        let TerrainSettings::Procedural(hex_map::ProceduralSettings::V3(v3)) = &world.terrain
        else {
            panic!("Crystal Ascent must remain a V3 procedural world");
        };
        let hex_map::V3LayoutSettings::Single(patch) = &v3.layout else {
            panic!("Crystal Ascent must remain a standalone Single patch");
        };
        assert_eq!(
            patch.environment,
            hex_map::V3EnvironmentSettings::TemperateGrassland
        );
        let hex_map::V3RecipeSettings::CrystalAscent(settings) = &patch.recipe else {
            panic!("Crystal Ascent must use its dedicated recipe");
        };
        assert_eq!(settings.base_level, 6);
        assert_eq!(settings.rise_levels, 144);

        let encounter = encounter_of(scenario);
        assert_eq!(encounter.rosters.len(), 1);
        let roster = encounter
            .rosters
            .first()
            .expect("Crystal Ascent should retain its player roster");
        assert_eq!(roster.faction, EncounterFaction::Player);
        assert_eq!(
            roster
                .units
                .iter()
                .map(|unit| unit.archetype.as_str())
                .collect::<Vec<_>>(),
            vec!["hedge-mage", "raider", "wolf"]
        );
        assert_eq!(encounter.unit_count(EncounterFaction::Hostile), 0);
        assert_eq!(
            roster.placement,
            EncounterPlacement::Formation {
                center: FormationCenter::Anchor("party_start".to_owned()),
                spread: 2,
            }
        );
    }

    /// The selectable Crystal Mountain content freezes the intended Macro roster and
    /// launches the standard party from the stable world-owned foot anchor. Exact
    /// tunnel and route behavior remains generator-owned and is proved in map tests.
    #[test]
    fn crystal_mountain_showcase_names_its_macro_world_and_non_combat_party() {
        let library = library();
        let scenario = library
            .scenarios
            .iter()
            .find(|scenario| scenario.name == "Crystal Mountain")
            .expect("the shipped library should contain Crystal Mountain");

        assert_eq!(
            scenario.world,
            "config/worlds/procedural-crystal-mountain.ron"
        );
        assert_eq!(
            scenario.encounter,
            "config/encounters/crystal-mountain-showcase.ron"
        );
        assert_eq!(scenario.generation_seed, Some(1_592_598_566));

        let world_text = fs::read_to_string(assets_dir().join(&scenario.world))
            .expect("the Crystal Mountain world should be readable");
        let world: MapSettings =
            ron::from_str(&world_text).expect("the Crystal Mountain world should parse");
        assert_eq!(world.grid_radius, 77);
        let TerrainSettings::Procedural(hex_map::ProceduralSettings::V3(v3)) = &world.terrain
        else {
            panic!("Crystal Mountain must remain a V3 procedural world");
        };
        let hex_map::V3LayoutSettings::Macro(layout) = &v3.layout else {
            panic!("Crystal Mountain must remain a Macro world");
        };
        assert_eq!(layout.macro_radius, 3);
        assert_eq!(
            layout
                .instances
                .iter()
                .map(|instance| (instance.name.as_str(), instance.cells.len()))
                .collect::<Vec<_>>(),
            vec![
                ("crystal-ascent", 7),
                ("summit-forest", 5),
                ("inner-mountain", 7),
                ("outer-mountain", 18),
            ]
        );
        let landmark = layout
            .instances
            .first()
            .expect("Crystal Mountain should retain its central landmark instance");
        assert_eq!(landmark.rotation_turns, 0);
        let hex_map::V3RecipeSettings::CrystalAscent(settings) = &landmark.recipe else {
            panic!("Crystal Mountain's central instance must use CrystalAscent");
        };
        assert_eq!(settings.base_level, 6);
        assert_eq!(settings.rise_levels, 144);
        assert!(layout.critical_route.is_empty());

        let encounter = encounter_of(scenario);
        assert_eq!(encounter.name, "Crystal Mountain Showcase");
        assert_eq!(encounter.rosters.len(), 1);
        let roster = encounter
            .rosters
            .first()
            .expect("Crystal Mountain should retain its player roster");
        assert_eq!(roster.faction, EncounterFaction::Player);
        assert_eq!(
            roster
                .units
                .iter()
                .map(|unit| unit.archetype.as_str())
                .collect::<Vec<_>>(),
            vec!["hedge-mage", "raider", "wolf"]
        );
        assert_eq!(encounter.unit_count(EncounterFaction::Hostile), 0);
        assert_eq!(
            roster.placement,
            EncounterPlacement::Formation {
                center: FormationCenter::Anchor("crystal_mountain.foot_apron".to_owned()),
                spread: 2,
            }
        );
    }

    #[test]
    fn crystal_mountain_showcase_stages_a_clear_default_group_route_from_the_outer_apron() {
        let mut app = procedural_gameplay_app("Crystal Mountain");
        enter_screen(&mut app, Screen::Gameplay);

        assert!(
            !app.world().contains_resource::<GameplaySetupFailure>(),
            "exact outer-apron staging failed: {:?}",
            app.world()
                .get_resource::<GameplaySetupFailure>()
                .map(|failure| failure.reason.as_str())
        );
        let (foot_apron, tunnel_mouth) = {
            let anchors = app.world().resource::<MapAnchors>();
            (
                anchors
                    .get(&MapAnchorId::from("crystal_mountain.foot_apron"))
                    .expect("Crystal Mountain should publish its exterior apron"),
                anchors
                    .get(&MapAnchorId::from("crystal_mountain.tunnel_mouth"))
                    .expect("Crystal Mountain should publish its roofed threshold"),
            )
        };
        let mut party = {
            let world = app.world_mut();
            let mut players =
                world.query_filtered::<(&UnitId, &StandsOn, &Transform), With<Player>>();
            players
                .iter(world)
                .map(|(unit, standing, transform)| {
                    assert_eq!(transform.translation, standing.0.world_position());
                    (*unit, standing.0.pos)
                })
                .collect::<Vec<_>>()
        };
        party.sort_by_key(|(unit, _)| *unit);
        assert_eq!(party.len(), 3);
        assert_eq!(
            party.first().map(|(_, position)| *position),
            Some(foot_apron)
        );
        let occupied = party
            .iter()
            .map(|(_, position)| *position)
            .collect::<BTreeSet<_>>();
        assert_eq!(occupied.len(), party.len());
        assert_eq!(
            party
                .iter()
                .map(|(_, position)| *position)
                .collect::<Vec<_>>(),
            vec![
                foot_apron,
                TilePos::new(HexCoord::from_axial(-77, 0), 6),
                TilePos::new(HexCoord::from_axial(-77, 4), 6),
            ],
            "the fresh party should retain its stable group-safe exterior footprint"
        );
        let interiors = app.world().resource::<InteriorRegions>();
        assert!(party.iter().all(|(_, position)| {
            position.level == foot_apron.level
                && position.coord.distance(HexCoord::ORIGIN) <= 77
                && interiors.get(*position).is_none()
        }));

        let formation = app.world().resource::<PartyFormation>();
        let inward = TilePos::new(
            foot_apron.coord.neighbor(formation.facing),
            foot_apron.level,
        );
        assert!(
            inward.coord.distance(tunnel_mouth.coord)
                < foot_apron.coord.distance(tunnel_mouth.coord),
            "the staged party must face inward toward the tunnel"
        );
        let inward_is_standable = {
            let world = app.world_mut();
            let mut surfaces = world.query_filtered::<(&TilePos, &Headroom), With<HexTile>>();
            surfaces
                .iter(world)
                .any(|(position, headroom)| *position == inward && headroom.0 > 0)
        };
        assert!(inward_is_standable);

        let (selected, body) = {
            let world = app.world_mut();
            let mut players = world.query_filtered::<(&UnitId, &Body, &StandsOn), With<Player>>();
            players
                .iter(world)
                .min_by_key(|(unit, _, _)| **unit)
                .map(|(unit, body, _)| (*unit, *body))
                .expect("the standard party should retain its selected explorer")
        };
        let table = app.world().resource::<SubstanceTable>().clone();
        let blockers = app.world().resource::<TraversalBlockers>().clone();
        let authored = app.world().resource::<AuthoredObjectOccupancy>().clone();
        let footing = Arc::new({
            let world = app.world_mut();
            let mut tiles = world
                .query_filtered::<(&TilePos, &HexSpan, &SubstanceId, &Headroom), With<HexTile>>();
            Footing::from_tiles_with_object_occupancy(
                tiles.iter(world),
                &table,
                body,
                Some(&blockers),
                &authored,
            )
        });
        let destination = footing
            .at(tunnel_mouth)
            .expect("the roofed tunnel threshold should remain standable");

        let formation = app.world().resource::<PartyFormation>().clone();
        let formations = app.world().resource::<FormationCatalog>();
        let preset = formations
            .get(&formation.preset)
            .expect("fresh gameplay should resolve the shipped formation preset")
            .clone();
        let anchor_slot = preset
            .anchor()
            .expect("the shipped formation preset should have one anchor");
        let anchor = formation
            .assignments
            .iter()
            .find_map(|(&unit, &slot)| (slot == anchor_slot).then_some(unit))
            .expect("the staged standard party should retain its formation anchor");
        assert_eq!(anchor, selected);
        let members = {
            let world = app.world_mut();
            let mut players = world.query_filtered::<(&UnitId, &StandsOn), With<Player>>();
            players
                .iter(world)
                .map(|(unit, standing)| FormationMember {
                    unit: *unit,
                    standing: standing.0,
                    footing: Arc::clone(&footing),
                })
                .collect::<Vec<_>>()
        };
        let anchor_standing = members
            .iter()
            .find_map(|member| (member.unit == anchor).then_some(member.standing))
            .expect("the formation anchor should be one of the staged players");
        let external_occupancy = UnitOccupancy::default();
        let anchor_route = route_with_occupancy(
            anchor_standing,
            destination,
            &footing,
            &external_occupancy,
            anchor,
        )
        .expect("the default Group anchor should route to the tunnel mouth");
        let group_plan = plan_formation_move_with_occupancy(
            &preset,
            &formation,
            &anchor_route,
            members,
            &external_occupancy,
        )
        .expect("the default Group party should compress through the four-wide mouth");
        assert_eq!(group_plan.paths.len(), party.len());
        assert!(group_plan.paths.iter().all(|path| path.path.len() > 1));
        let anchor_path = group_plan
            .paths
            .iter()
            .find(|path| path.member == anchor)
            .expect("the group plan should retain its anchor path");
        assert_eq!(anchor_path.path.first().copied(), Some(foot_apron));
        assert_eq!(anchor_path.path.last().copied(), Some(tunnel_mouth));
    }

    fn restore_staging_override_fixture(
        mut commands: Commands,
        mut formation: ResMut<PartyFormation>,
        players: Query<(Entity, &UnitId, &StandsOn), With<Player>>,
    ) {
        let mut staged = players
            .iter()
            .map(|(entity, unit, standing)| (entity, *unit, standing.0))
            .collect::<Vec<_>>();
        staged.sort_by_key(|(_, unit, _)| *unit);
        let restored = staged
            .iter()
            .map(|(_, _, standing)| *standing)
            .cycle()
            .skip(1)
            .take(staged.len())
            .collect::<Vec<_>>();
        for ((entity, _, _), standing) in staged.into_iter().zip(restored) {
            commands.entity(entity).insert((
                StandsOn(standing),
                Transform::from_translation(standing.world_position()),
            ));
        }
        formation.facing = Sextant::D;
        formation.mode = PartyMovementMode::Solo;
    }

    #[test]
    fn crystal_mountain_restore_remains_authoritative_over_fresh_showcase_staging() {
        let mut app = procedural_gameplay_app("Crystal Mountain");
        app.add_systems(
            OnEnter(Screen::Gameplay),
            restore_staging_override_fixture.in_set(GameplaySetup::Restore),
        );
        enter_screen(&mut app, Screen::Gameplay);

        assert!(
            !app.world().contains_resource::<GameplaySetupFailure>(),
            "the restore fixture should retain a valid composed setup"
        );
        let mut party = {
            let world = app.world_mut();
            let mut players =
                world.query_filtered::<(&UnitId, &StandsOn, &Transform), With<Player>>();
            players
                .iter(world)
                .map(|(unit, standing, transform)| {
                    assert_eq!(transform.translation, standing.0.world_position());
                    (*unit, standing.0.pos)
                })
                .collect::<Vec<_>>()
        };
        party.sort_by_key(|(unit, _)| *unit);
        assert_eq!(
            party,
            vec![
                (UnitId(0), TilePos::new(HexCoord::from_axial(-77, 0), 6)),
                (UnitId(1), TilePos::new(HexCoord::from_axial(-77, 4), 6)),
                (UnitId(2), TilePos::new(HexCoord::from_axial(-77, 3), 6)),
            ],
            "Restore must overwrite every fresh staging position"
        );
        let formation = app.world().resource::<PartyFormation>();
        assert_eq!(formation.facing, Sextant::D);
        assert_eq!(formation.mode, PartyMovementMode::Solo);
    }

    /// The generic encounter formation owns a spawn *region*. This landmark's terminal
    /// is narrower: freeze the exact fresh-launch staging that the scenario adapter
    /// publishes before perception sees any party member.
    #[test]
    fn crystal_ascent_showcase_stages_every_party_member_on_the_exact_apron() {
        let mut app = procedural_gameplay_app("Crystal Ascent");
        enter_screen(&mut app, Screen::Gameplay);

        assert!(
            !app.world().contains_resource::<GameplaySetupFailure>(),
            "exact apron staging failed: {:?}",
            app.world()
                .get_resource::<GameplaySetupFailure>()
                .map(|failure| failure.reason.as_str())
        );
        let mut party = {
            let world = app.world_mut();
            let mut players = world
                .query_filtered::<(&UnitId, &Archetype, &StandsOn, &Transform), With<Player>>();
            players
                .iter(world)
                .map(|(unit, archetype, standing, transform)| {
                    assert_eq!(
                        transform.translation,
                        standing.0.world_position(),
                        "{unit:?} presentation did not follow its authoritative staging surface"
                    );
                    (*unit, archetype.0.clone(), standing.0.pos)
                })
                .collect::<Vec<_>>()
        };
        party.sort_by_key(|(unit, ..)| *unit);
        assert_eq!(
            party,
            vec![
                (
                    UnitId(0),
                    "hedge-mage".to_owned(),
                    TilePos::new(HexCoord::new_cubic(-17, -15, 32), 6),
                ),
                (
                    UnitId(1),
                    "raider".to_owned(),
                    TilePos::new(HexCoord::new_cubic(-18, -14, 32), 6),
                ),
                (
                    UnitId(2),
                    "wolf".to_owned(),
                    TilePos::new(HexCoord::new_cubic(-16, -16, 32), 6),
                ),
            ],
            "the stable roster must occupy three exact cells of the four-wide lower apron"
        );
        let interiors = app.world().resource::<InteriorRegions>();
        assert!(party.iter().all(|(_, _, position)| {
            position.coord.distance(HexCoord::ORIGIN) == 32 && interiors.get(*position).is_none()
        }));

        let formation = app.world().resource::<PartyFormation>();
        assert_eq!(formation.facing, Sextant::A);
        let lower_entry = app
            .world()
            .resource::<MapAnchors>()
            .get(&MapAnchorId::from("crystal_ascent.lower_entry"))
            .expect("the showcase should publish its lower entry");
        let chamber = app
            .world()
            .resource::<MapAnchors>()
            .get(&MapAnchorId::from("crystal_ascent.bottom_chamber"))
            .expect("the showcase should publish its bottom chamber");
        assert!(
            lower_entry
                .coord
                .neighbor(formation.facing)
                .distance(chamber.coord)
                < lower_entry.coord.distance(chamber.coord),
            "the party's initial travel-facing must point inward through the entrance"
        );
    }

    #[test]
    fn non_crystal_scenarios_keep_generic_formation_staging() {
        let mut app = procedural_gameplay_app("Procedural Hills");
        enter_screen(&mut app, Screen::Gameplay);

        assert!(
            !app.world().contains_resource::<GameplaySetupFailure>(),
            "the Crystal Ascent adapter interfered with another scenario"
        );
        let party_start = app
            .world()
            .resource::<MapAnchors>()
            .get(&MapAnchorId::from("party_start"))
            .expect("Procedural Hills should publish party_start");
        let mut party = {
            let world = app.world_mut();
            let mut players = world.query_filtered::<(&UnitId, &StandsOn), With<Player>>();
            players
                .iter(world)
                .map(|(unit, standing)| (*unit, standing.0.pos))
                .collect::<Vec<_>>()
        };
        party.sort_by_key(|(unit, _)| *unit);
        assert_eq!(party.len(), 1);
        assert_eq!(party.first().copied(), Some((UnitId(0), party_start)));
    }

    #[test]
    fn crystal_ascent_showcase_builds_the_complete_runtime_contract() {
        let mut app = procedural_gameplay_app("Crystal Ascent");
        enter_screen(&mut app, Screen::Gameplay);

        assert!(
            app.world().contains_resource::<TerrainReady>(),
            "Crystal Ascent did not finish terrain generation: {:?}",
            app.world()
                .get_resource::<GameplaySetupFailure>()
                .map(|failure| failure.reason.as_str())
        );
        let report = app.world().resource::<GenerationReport>();
        let Some(ProceduralRecipeMetrics::CrystalAscent(metrics)) = &report.recipe_metrics else {
            panic!(
                "Crystal Ascent published unexpected metrics: {:?}",
                report.recipe_metrics
            );
        };
        assert_eq!(metrics.circuits, 3);
        assert_eq!(metrics.flights, 18);
        assert_eq!(metrics.landings, 18);
        assert_eq!(metrics.crystal_fixtures, 19);
        assert_eq!(metrics.gameplay_lights, 38);
        assert_eq!(metrics.rise_levels, 144);
        assert!(metrics.minimum_stair_headroom >= 4);
        assert!(metrics.critical_route_steps > 144);

        let anchors = app.world().resource::<MapAnchors>();
        for name in [
            "crystal_ascent.lower_entry",
            "crystal_ascent.bottom_chamber",
            "crystal_ascent.corner_landing",
            "crystal_ascent.upper_exit",
        ] {
            assert!(
                anchors.get(&MapAnchorId::from(name)).is_some(),
                "Crystal Ascent omitted {name}"
            );
        }
        let lower_entry = anchors
            .get(&MapAnchorId::from("crystal_ascent.lower_entry"))
            .expect("Crystal Ascent should publish its lower entry");
        let bottom_chamber = anchors
            .get(&MapAnchorId::from("crystal_ascent.bottom_chamber"))
            .expect("Crystal Ascent should publish its bottom chamber");
        let mid_flight = anchors
            .get(&MapAnchorId::from("crystal_ascent.mid_flight"))
            .expect("Crystal Ascent should publish its deterministic mid-flight review point");
        let corner_landing = anchors
            .get(&MapAnchorId::from("crystal_ascent.corner_landing"))
            .expect("Crystal Ascent should publish its deterministic corner-landing review point");
        let upper_contraction = anchors
            .get(&MapAnchorId::from("crystal_ascent.upper_contraction"))
            .expect("Crystal Ascent should publish its upper-contraction review point");
        let upper_exit = anchors
            .get(&MapAnchorId::from("crystal_ascent.upper_exit"))
            .expect("Crystal Ascent should publish its upper exit");
        let illumination = app.world().resource::<ResolvedIllumination>();
        for exterior in [lower_entry, upper_exit] {
            let resolved = illumination
                .get(exterior)
                .unwrap_or_else(|| panic!("missing resolved illumination at {exterior:?}"));
            assert_eq!(resolved.domain, LightDomain::Exterior);
            assert_eq!(resolved.level, IlluminationLevel::Bright);
        }
        for interior in [
            bottom_chamber,
            mid_flight,
            corner_landing,
            upper_contraction,
        ] {
            let resolved = illumination
                .get(interior)
                .unwrap_or_else(|| panic!("missing resolved illumination at {interior:?}"));
            assert!(matches!(resolved.domain, LightDomain::Interior(_)));
            assert!(
                resolved.level >= IlluminationLevel::Dim,
                "required interior review point {interior:?} resolved {:?}",
                resolved.level
            );
        }
        assert!(
            app.world()
                .resource::<InteriorRegions>()
                .surfaces()
                .next()
                .is_some(),
            "Crystal Ascent did not publish its dark interior domain"
        );
        let gameplay_lights = {
            let world = app.world_mut();
            let mut lights = world.query::<&GameplayLight>();
            lights
                .iter(world)
                .map(|light| (light.level, light.radius))
                .collect::<Vec<_>>()
        };
        assert_eq!(gameplay_lights.len(), 38);
        assert_eq!(
            gameplay_lights
                .iter()
                .filter(|light| **light == (IlluminationLevel::Bright, 4))
                .count(),
            18
        );
        assert_eq!(
            gameplay_lights
                .iter()
                .filter(|light| **light == (IlluminationLevel::Dim, 18))
                .count(),
            18
        );
        assert_eq!(
            gameplay_lights
                .iter()
                .filter(|light| **light == (IlluminationLevel::Bright, 8))
                .count(),
            1
        );
        assert_eq!(
            gameplay_lights
                .iter()
                .filter(|light| **light == (IlluminationLevel::Dim, 24))
                .count(),
            1
        );
        let physical_lights = app
            .world_mut()
            .query::<&PointLight>()
            .iter(app.world())
            .collect::<Vec<_>>();
        assert_eq!(
            physical_lights.len(),
            22,
            "the 18 landing crystals and four heart emitters should each publish once"
        );
        assert!(physical_lights.iter().all(|light| {
            (light.intensity - 4_500.0).abs() <= f32::EPSILON
                && (light.range - 4.5).abs() <= f32::EPSILON
                && !light.shadow_maps_enabled
                && !light.contact_shadows_enabled
        }));
        let crystal_objects = {
            let world = app.world_mut();
            let mut objects = world.query::<&ObjectInstance>();
            objects
                .iter(world)
                .filter(|instance| instance.object_id().as_str().starts_with("prop/crystal-"))
                .map(|instance| {
                    (
                        instance.object_id().as_str().to_owned(),
                        TilePos::new(
                            instance.origin().coord,
                            instance.origin().level.saturating_sub(1),
                        ),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            crystal_objects
                .iter()
                .filter(|(id, _)| id == "prop/crystal-cathedral-heart")
                .count(),
            1,
            "the visual heart and its authoritative occupancy must share one root"
        );
        assert_eq!(
            crystal_objects.len(),
            19,
            "the heart and all 18 landing crystals should be visible fixtures"
        );
        let illumination = app.world().resource::<ResolvedIllumination>();
        for (_, floor) in &crystal_objects {
            let resolved = illumination
                .get(*floor)
                .unwrap_or_else(|| panic!("missing fixture illumination at {floor:?}"));
            assert_eq!(resolved.level, IlluminationLevel::Bright);
            assert!(matches!(resolved.domain, LightDomain::Interior(_)));
        }
        assert_eq!(
            app.world_mut()
                .query::<&AuthoredObjectVoxelRuns>()
                .iter(app.world())
                .count(),
            1,
            "only the cathedral heart should opt into authored occupancy"
        );
        let (first_heart, first_runs, first_occupancy) = crystal_heart_occupancy_snapshot(&mut app);
        let first_occupancy_fingerprint = first_occupancy.fingerprint();
        assert_ne!(first_occupancy_fingerprint, 0);
        let heart_support = TilePos::new(
            first_heart.origin().coord,
            first_heart.origin().level.saturating_sub(1),
        );
        assert!(first_occupancy.blocks_standing_body(heart_support, TraversalProfile::WALKER));
        assert!(app
            .world()
            .resource::<TraversalBlockers>()
            .contains(heart_support));
        let first_blocked_pair = crystal_heart_blocked_sight_pair(&app, first_heart.origin());
        assert_eq!(
            app.world_mut()
                .query_filtered::<Entity, With<Player>>()
                .iter(app.world())
                .count(),
            3
        );
        assert!(standing_pos::<Enemy>(&mut app).is_none());

        enter_screen(&mut app, Screen::Title);
        assert!(
            !app.world().contains_resource::<AuthoredObjectOccupancy>(),
            "Title must not retain gameplay occupancy authority"
        );
        assert_eq!(
            app.world_mut()
                .query::<&AuthoredObjectVoxelRuns>()
                .iter(app.world())
                .count(),
            0,
            "Title must not retain the generated heart source"
        );

        enter_screen(&mut app, Screen::Gameplay);
        assert!(app.world().contains_resource::<TerrainReady>());
        let (second_heart, second_runs, second_occupancy) =
            crystal_heart_occupancy_snapshot(&mut app);
        assert_eq!(second_heart, first_heart);
        assert_eq!(second_runs, first_runs);
        assert_eq!(second_occupancy, first_occupancy);
        assert_eq!(second_occupancy.fingerprint(), first_occupancy_fingerprint);
        assert_eq!(
            crystal_heart_blocked_sight_pair(&app, second_heart.origin()),
            first_blocked_pair,
            "re-entry must rebuild identical seven-ray obstruction"
        );
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn crystal_mountain_rebuilds_visibility_fog_and_cutaway_from_composed_authority() {
        let mut app = crystal_mountain_presentation_app();
        enter_screen(&mut app, Screen::Gameplay);
        assert!(
            app.world().contains_resource::<TerrainReady>(),
            "Crystal Mountain setup failed: {:?}",
            app.world()
                .get_resource::<GameplaySetupFailure>()
                .map(|failure| failure.reason.as_str())
        );
        let first = crystal_mountain_runtime_snapshot(&mut app);

        enter_screen(&mut app, Screen::Title);
        assert!(!app.world().contains_resource::<VoxelMap>());
        assert!(!app.world().contains_resource::<TerrainOccupancy>());
        assert!(!app.world().contains_resource::<AuthoredObjectOccupancy>());
        assert!(!app.world().contains_resource::<ResolvedIllumination>());
        assert!(!app.world().contains_resource::<LocalMapKnowledge>());
        assert!(!app.world().contains_resource::<FactionMapKnowledge>());
        assert!(crate::fog::fog_overlay_positions(app.world_mut()).is_empty());
        assert_eq!(
            app.world_mut()
                .query_filtered::<Entity, With<HexGrid>>()
                .iter(app.world())
                .count(),
            0,
            "Crystal Mountain left a rendered grid after teardown"
        );
        assert_eq!(
            app.world_mut()
                .query::<&CutawayOccluder>()
                .iter(app.world())
                .count(),
            0,
            "Crystal Mountain left cutaway-tagged roof entities after teardown"
        );
        assert_eq!(
            app.world_mut()
                .query::<&TreeOccluder>()
                .iter(app.world())
                .count(),
            0,
            "Crystal Mountain left generated tree roots after teardown"
        );
        {
            let world = app.world_mut();
            let mut occlusions = world.query::<&PresentationOcclusion>();
            assert!(occlusions.iter(world).all(|occlusion| {
                !occlusion.contains(PresentationOcclusionReason::Fog)
                    && !occlusion.contains(PresentationOcclusionReason::InteriorCutaway)
            }));
        }

        enter_screen(&mut app, Screen::Gameplay);
        assert!(app.world().contains_resource::<TerrainReady>());
        assert!(!app.world().contains_resource::<GameplaySetupFailure>());
        let second = crystal_mountain_runtime_snapshot(&mut app);
        assert_eq!(
            second, first,
            "Crystal Mountain re-entry rebuilt stale or divergent visibility presentation state"
        );
    }

    #[test]
    #[ignore = "manual release/debug Crystal Ascent end-to-end boundary-rise benchmark"]
    fn crystal_ascent_boundary_rises_track_materialization_perception_and_entity_counts() {
        for rise_levels in [100, 144, 200] {
            let mut app = procedural_gameplay_app("Crystal Ascent");
            {
                let mut map = app.world_mut().resource_mut::<MapSettings>();
                let TerrainSettings::Procedural(hex_map::ProceduralSettings::V3(v3)) =
                    &mut map.terrain
                else {
                    panic!("Crystal Ascent benchmark should retain V3 settings");
                };
                let hex_map::V3LayoutSettings::Single(patch) = &mut v3.layout else {
                    panic!("Crystal Ascent benchmark should retain its Single patch");
                };
                let hex_map::V3RecipeSettings::CrystalAscent(settings) = &mut patch.recipe else {
                    panic!("Crystal Ascent benchmark should retain its recipe");
                };
                settings.rise_levels = rise_levels;
                map.validate()
                    .unwrap_or_else(|error| panic!("rise {rise_levels} must validate: {error}"));
            }

            let started = Instant::now();
            enter_screen(&mut app, Screen::Gameplay);
            let setup_elapsed = started.elapsed();
            assert!(
                app.world().contains_resource::<TerrainReady>(),
                "rise {rise_levels} setup failed: {:?}",
                app.world()
                    .get_resource::<GameplaySetupFailure>()
                    .map(|failure| failure.reason.as_str())
            );

            let (columns, material_runs) = {
                let map = app.world().resource::<VoxelMap>();
                (
                    map.columns().count(),
                    map.columns()
                        .map(|(_, column)| hex_map::runs(column).len())
                        .sum::<usize>(),
                )
            };
            let (tile_entities, total_entities, object_instances, point_lights) = {
                let world = app.world_mut();
                let tile_entities = world
                    .query_filtered::<Entity, With<HexTile>>()
                    .iter(world)
                    .count();
                let object_instances = world.query::<&ObjectInstance>().iter(world).count();
                let point_lights = world.query::<&PointLight>().iter(world).count();
                (
                    tile_entities,
                    world.iter_entities().count(),
                    object_instances,
                    point_lights,
                )
            };
            let illumination_surfaces = app.world().resource::<ResolvedIllumination>().len();
            let perception = *app
                .world()
                .resource::<hex_perception::PerceptionRuntimeStats>();
            let report = app.world().resource::<GenerationReport>();
            let Some(ProceduralRecipeMetrics::CrystalAscent(metrics)) = &report.recipe_metrics
            else {
                panic!("rise {rise_levels} omitted Crystal Ascent metrics");
            };
            assert_eq!(metrics.rise_levels, rise_levels);
            assert_eq!(columns, 4_921);
            assert_eq!(object_instances, 61);
            assert_eq!(point_lights, 22);
            assert!(
                illumination_surfaces >= metrics.ordinary_surfaces as usize,
                "resolved illumination must include every ordinary route surface"
            );
            assert!((1..=2).contains(&perception.illumination_resolutions));
            assert!((1..=2).contains(&perception.observation_resolutions));
            eprintln!(
                "CRYSTAL_ASCENT_RUNTIME rise={rise_levels} setup={setup_elapsed:?} \
                 generation_us={} columns={columns} material_runs={material_runs} \
                 tile_entities={tile_entities} total_entities={total_entities} \
                 ordinary_surfaces={} illumination_surfaces={illumination_surfaces} \
                 object_instances={object_instances} point_lights={point_lights}",
                report.elapsed_micros, metrics.ordinary_surfaces,
            );

            enter_screen(&mut app, Screen::Title);
            assert!(!app.world().contains_resource::<VoxelMap>());
            assert!(!app.world().contains_resource::<ResolvedIllumination>());
            assert!(!app.world().contains_resource::<AuthoredObjectOccupancy>());
        }
    }

    #[derive(Debug)]
    struct MacroRuntimeProfile {
        scenario_name: &'static str,
        setup_elapsed: std::time::Duration,
        generation_and_materialization_micros: u64,
        columns: usize,
        material_runs: usize,
        tile_entities: usize,
        total_entities: usize,
        object_instances: usize,
        point_lights: usize,
        illumination_surfaces: usize,
        illumination_resolutions: u64,
        observation_resolutions: u64,
    }

    fn macro_runtime_profile(scenario_name: &'static str) -> MacroRuntimeProfile {
        let mut app = procedural_gameplay_app(scenario_name);
        let started = Instant::now();
        enter_screen(&mut app, Screen::Gameplay);
        let setup_elapsed = started.elapsed();
        assert!(
            app.world().contains_resource::<TerrainReady>(),
            "{scenario_name} setup failed: {:?}",
            app.world()
                .get_resource::<GameplaySetupFailure>()
                .map(|failure| failure.reason.as_str())
        );

        let report = app.world().resource::<GenerationReport>().clone();
        let (columns, material_runs) = {
            let map = app.world().resource::<VoxelMap>();
            (
                map.columns().count(),
                map.columns()
                    .map(|(_, column)| hex_map::runs(column).len())
                    .sum::<usize>(),
            )
        };
        let illumination_surfaces = app.world().resource::<ResolvedIllumination>().len();
        let perception = *app
            .world()
            .resource::<hex_perception::PerceptionRuntimeStats>();
        let (tile_entities, total_entities, object_instances, point_lights) = {
            let world = app.world_mut();
            let tile_entities = world
                .query_filtered::<Entity, With<HexTile>>()
                .iter(world)
                .count();
            let object_instances = world.query::<&ObjectInstance>().iter(world).count();
            let point_lights = world.query::<&PointLight>().iter(world).count();
            (
                tile_entities,
                world.iter_entities().count(),
                object_instances,
                point_lights,
            )
        };

        enter_screen(&mut app, Screen::Title);
        assert!(!app.world().contains_resource::<VoxelMap>());
        assert!(!app.world().contains_resource::<ResolvedIllumination>());
        assert!(!app.world().contains_resource::<AuthoredObjectOccupancy>());

        MacroRuntimeProfile {
            scenario_name,
            setup_elapsed,
            generation_and_materialization_micros: report.elapsed_micros,
            columns,
            material_runs,
            tile_entities,
            total_entities,
            object_instances,
            point_lights,
            illumination_surfaces,
            illumination_resolutions: perception.illumination_resolutions,
            observation_resolutions: perception.observation_resolutions,
        }
    }

    #[test]
    #[ignore = "manual release-mode Mountain Range/Crystal Mountain runtime comparison"]
    fn crystal_mountain_runtime_profile_compares_materialization_and_entities_to_mountain_range() {
        let mountain_range = macro_runtime_profile("Mountain Range");
        let crystal_mountain = macro_runtime_profile("Crystal Mountain");

        for profile in [&mountain_range, &crystal_mountain] {
            assert_eq!(profile.columns, 18_019);
            assert!(profile.setup_elapsed > std::time::Duration::ZERO);
            assert!(profile.generation_and_materialization_micros > 0);
            assert!(profile.material_runs >= profile.columns);
            assert!(profile.tile_entities >= profile.material_runs);
            assert!(profile.total_entities >= profile.tile_entities);
            assert!(profile.illumination_surfaces > 0);
            assert!((1..=2).contains(&profile.illumination_resolutions));
            assert!((1..=2).contains(&profile.observation_resolutions));
            eprintln!(
                "MACRO_RUNTIME scenario={:?} setup={:?} generation_and_materialization_us={} \
                 columns={} material_runs={} tile_entities={} total_entities={} \
                 object_instances={} point_lights={} illumination_surfaces={} \
                 illumination_resolutions={} observation_resolutions={}",
                profile.scenario_name,
                profile.setup_elapsed,
                profile.generation_and_materialization_micros,
                profile.columns,
                profile.material_runs,
                profile.tile_entities,
                profile.total_entities,
                profile.object_instances,
                profile.point_lights,
                profile.illumination_surfaces,
                profile.illumination_resolutions,
                profile.observation_resolutions,
            );
        }
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

    /// Loading is only valid after a typed launch owner has supplied a snapshot.
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
            encounter_override: None,
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

        // Back to Main Menu, exactly as BACKSPACE does, then issue the other launch.
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

    /// The seed frozen by the typed launch request is installed for map generation.
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

    fn finish_test_app(mut app: App) -> App {
        while app.plugins_state() != PluginsState::Cleaned {
            app.finish();
            app.cleanup();
        }
        app
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

    /// Builds one shipped scenario through the production map and unit plugins.
    pub(crate) fn procedural_gameplay_app_with_combat(
        scenario_name: &str,
        with_combat: bool,
    ) -> App {
        finish_test_app(unfinished_procedural_gameplay_app(
            scenario_name,
            with_combat,
        ))
    }

    fn unfinished_procedural_gameplay_app(scenario_name: &str, with_combat: bool) -> App {
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
        let art_catalog = runtime_art_catalog(&palette);
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
                GameplaySetup::Restore,
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
        app.insert_resource(art_catalog);
        app.insert_resource(palette);
        app.insert_resource(encounter_of(&entry));
        app.insert_resource(ActiveScenario(ScenarioToLoad {
            scenario: entry.clone(),
            resolved_seed: seed,
            encounter_override: None,
        }));
        app.insert_resource(world);
        let formations: FormationCatalog =
            ron::from_str(include_str!("../../../assets/config/formations.ron"))
                .expect("the shipped formation catalog should deserialize");
        app.insert_resource(formations);
        if let Some(seed) = seed {
            app.insert_resource(seed);
        }
        app.add_plugins((
            hex_map::plugin,
            hex_units::terrain_occupancy::plugin,
            hex_units::authored_object_occupancy::plugin,
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
            (
                stage_crystal_ascent_showcase_party
                    .after(GameplaySetup::Actors)
                    .before(GameplaySetup::Restore),
                stage_crystal_mountain_showcase_party
                    .after(GameplaySetup::Actors)
                    .before(GameplaySetup::Restore),
                finalize_gameplay_setup.in_set(GameplaySetup::Finalize),
            ),
        );

        app
    }

    /// Applies a real state transition and lets its entry schedule settle.
    pub(crate) fn enter_screen(app: &mut App, screen: Screen) {
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
        stalled: bool,
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

        let occupancy = {
            let world = app.world_mut();
            let mut units = world.query::<(&UnitId, &StandsOn)>();
            UnitOccupancy::from_positions(
                units
                    .iter(world)
                    .map(|(unit, standing)| (*unit, standing.0.pos)),
            )
        };
        let reach = Reach::with_occupancy(standing, &footing, None, &occupancy, actor);
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
                            && !occupancy.is_occupied(candidate.pos, Some(actor))
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

        let mut last_progress = (0, 0, 0, 0, 0, 0);
        let mut last_progress_round = 0;
        let mut stalled = false;
        let mut frames_executed = 0;
        for _ in 0..4_000 {
            frames_executed += 1;
            if app
                .world()
                .resource::<EncounterResolution>()
                .outcome()
                .is_some()
            {
                app.update();
                break;
            }
            let (round, progress) = {
                let summary = app.world().resource::<CombatSummary>();
                (
                    app.world().resource::<TurnOrder>().round,
                    (
                        summary.moves,
                        summary.strikes,
                        summary.applied_disables,
                        summary.restored_cells,
                        summary.downings,
                        summary.revivals,
                    ),
                )
            };
            if progress != last_progress {
                last_progress = progress;
                last_progress_round = round;
            } else if round.saturating_sub(last_progress_round) >= 25 {
                stalled = true;
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
        let bound_diagnostic = {
            let moving = {
                let world = app.world_mut();
                let mut moving = world.query_filtered::<&UnitId, With<hex_units::MovingTo>>();
                moving.iter(world).copied().collect::<Vec<_>>()
            };
            let busy = {
                let world = app.world_mut();
                let mut busy = world.query_filtered::<&UnitId, With<hex_core::Busy>>();
                busy.iter(world).copied().collect::<Vec<_>>()
            };
            let authority = hex_combat::authority_snapshot(app.world()).map(|state| {
                (
                    state.current(),
                    state
                        .units
                        .values()
                        .map(|actor| {
                            (
                                actor.id,
                                actor.faction,
                                actor.turn,
                                actor.busy,
                                actor.motion.is_some(),
                                actor.downed,
                                actor.position,
                            )
                        })
                        .collect::<Vec<_>>(),
                    state
                        .events
                        .iter()
                        .rev()
                        .take(5)
                        .cloned()
                        .collect::<Vec<_>>(),
                )
            });
            let player_turns = {
                let world = app.world_mut();
                let mut players = world.query_filtered::<
                    (&UnitId, Option<&Turn>, Has<hex_core::Busy>, Has<Downed>),
                    With<Player>,
                >();
                players
                    .iter(world)
                    .map(|(id, turn, busy, downed)| (*id, turn.copied(), busy, downed))
                    .collect::<Vec<_>>()
            };
            let order = app.world().resource::<TurnOrder>();
            let summary = app.world().resource::<CombatSummary>();
            format!(
                "frames={frames_executed} round={} current={:?} progress={:?} \
                 last_progress_round={last_progress_round} pending={:?} queue_empty={} mode={:?} \
                 pause={:?} moving={moving:?} busy={busy:?} authority={authority:?} \
                 player_turns={player_turns:?} \
                 virtual_delta={:?} virtual_elapsed={:?}",
                order.round,
                order.current(),
                (
                    summary.moves,
                    summary.strikes,
                    summary.applied_disables,
                    summary.restored_cells,
                    summary.downings,
                    summary.revivals,
                ),
                app.world().resource::<PendingDecision>(),
                app.world().resource::<CommandQueue>().is_empty(),
                app.world().resource::<State<Mode>>().get(),
                app.world().get_resource::<State<Pause>>().map(State::get),
                app.world()
                    .resource::<bevy::time::Time<bevy::time::Virtual>>()
                    .delta(),
                app.world()
                    .resource::<bevy::time::Time<bevy::time::Virtual>>()
                    .elapsed(),
            )
        };
        assert!(
            outcome.is_some() || stalled,
            "the deterministic player policy neither resolved nor reached the bounded \
             no-progress gate: {bound_diagnostic}"
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
            stalled,
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
    /// positions, turn order, outcome, and the bounded no-progress gate required now
    /// that downed bodies can hold the Crossing's chokepoints. Equality here is
    /// therefore the integrated replay contract rather than a second, weaker
    /// simulation snapshot.
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
        assert!(
            first.summary.downings >= 1,
            "the replay should make damage progress before resolving or reaching its \
             bounded terrain-obstructed stalemate"
        );
        assert!(
            first.summary.outcome.is_some() || first.stalled,
            "the run should resolve or reproduce the bounded chokepoint stalemate"
        );
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
        let first_hostile_knowledge = app
            .world()
            .resource::<FactionMapKnowledge>()
            .faction(hex_units::Faction::Player)
            .state(first_hostile);
        assert_eq!(
            first_hostile_knowledge,
            KnowledgeState::Unknown,
            "bright range must not reveal the hostile anchor through intervening terrain"
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
            app.world()
                .resource::<FactionMapKnowledge>()
                .faction(hex_units::Faction::Player)
                .state(first_hostile),
            first_hostile_knowledge,
            "re-entry should reproduce obstruction-aware hostile visibility"
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

    fn assert_vegetation_scenario_reenters_with_the_same_art(scenario_name: &str) {
        fn object_snapshot(app: &mut App) -> Vec<(String, TilePos, u32, u8)> {
            let world = app.world_mut();
            let mut objects = world.query::<&ObjectInstance>();
            let mut snapshot = objects
                .iter(world)
                .map(|instance| {
                    (
                        instance.object_id().as_str().to_owned(),
                        instance.origin(),
                        instance.level_height().to_bits(),
                        instance.rotation().steps(),
                    )
                })
                .collect::<Vec<_>>();
            snapshot.sort_unstable();
            snapshot
        }

        let mut app = procedural_gameplay_app(scenario_name);
        let art_fingerprint = app
            .world()
            .resource::<RuntimeArtCatalog>()
            .combined_fingerprint();
        enter_screen(&mut app, Screen::Gameplay);

        assert!(app.world().contains_resource::<TerrainReady>());
        let first_map_fingerprint = app.world().resource::<GenerationReport>().map_fingerprint;
        let first_party = app
            .world()
            .resource::<MapAnchors>()
            .get(&MapAnchorId::from("party_start"))
            .unwrap_or_else(|| panic!("{scenario_name} should publish party_start"));
        let first_hostile = app
            .world()
            .resource::<MapAnchors>()
            .get(&MapAnchorId::from("hostile_start"))
            .unwrap_or_else(|| panic!("{scenario_name} should publish hostile_start"));
        let first_features = object_snapshot(&mut app);
        assert!(
            !first_features.is_empty(),
            "{scenario_name} should publish authored object instances"
        );

        enter_screen(&mut app, Screen::Title);
        assert!(!app.world().contains_resource::<VoxelMap>());
        assert_eq!(
            app.world()
                .resource::<RuntimeArtCatalog>()
                .combined_fingerprint(),
            art_fingerprint,
            "gameplay teardown should retain the accepted global art graph"
        );
        assert_eq!(
            object_snapshot(&mut app),
            Vec::new(),
            "{scenario_name} teardown left authored feature instances alive"
        );

        enter_screen(&mut app, Screen::Gameplay);
        assert!(app.world().contains_resource::<TerrainReady>());
        assert_eq!(
            app.world().resource::<GenerationReport>().map_fingerprint,
            first_map_fingerprint
        );
        assert_eq!(standing_pos::<Player>(&mut app), Some(first_party));
        assert_eq!(standing_pos::<Enemy>(&mut app), Some(first_hostile));
        assert_eq!(
            object_snapshot(&mut app),
            first_features,
            "{scenario_name} re-entry changed its exact authored feature placement"
        );
    }

    #[test]
    fn forest_reenters_with_the_same_authored_features_and_art_graph() {
        assert_vegetation_scenario_reenters_with_the_same_art("Forest");
    }

    #[test]
    fn deep_forest_reenters_with_the_same_authored_features_and_art_graph() {
        assert_vegetation_scenario_reenters_with_the_same_art("Deep Forest");
    }

    #[test]
    fn prairie_reenters_with_the_same_authored_features_and_art_graph() {
        assert_vegetation_scenario_reenters_with_the_same_art("Prairie");
    }

    #[test]
    fn island_showcases_freeze_their_shipped_profiles_and_standard_party() {
        for (name, world_path, radius) in [
            (
                "Sandy Islets",
                "config/worlds/procedural-sandy-islets.ron",
                24,
            ),
            (
                "Wooded Island",
                "config/worlds/procedural-wooded-island.ron",
                40,
            ),
            (
                "Ocean Archipelagoes",
                "config/worlds/procedural-ocean-archipelagoes.ron",
                77,
            ),
        ] {
            let scenario = library()
                .scenarios
                .into_iter()
                .find(|scenario| scenario.name == name)
                .unwrap_or_else(|| panic!("the shipped library should contain {name}"));
            assert_eq!(scenario.world, world_path);
            assert_eq!(scenario.generation_seed, Some(1_592_598_566));
            assert_eq!(scenario.encounter, "config/encounters/island-showcase.ron");

            let world_text = fs::read_to_string(assets_dir().join(world_path))
                .unwrap_or_else(|error| panic!("cannot read {name} world: {error}"));
            let world: MapSettings = ron::from_str(&world_text)
                .unwrap_or_else(|error| panic!("cannot parse {name} world: {error}"));
            assert_eq!(world.grid_radius, radius);
            let TerrainSettings::Procedural(hex_map::ProceduralSettings::V3(v3)) = &world.terrain
            else {
                panic!("{name} should remain a V3 procedural world");
            };
            match (name, &v3.layout) {
                ("Sandy Islets", hex_map::V3LayoutSettings::Single(patch)) => {
                    assert_eq!(patch.environment, hex_map::V3EnvironmentSettings::Coastal);
                    assert!(patch.overlays.is_empty());
                    assert!(matches!(
                        &patch.mask,
                        hex_map::PatchMaskSettings::WholeWorld
                    ));
                    let hex_map::V3RecipeSettings::SandyIslets(settings) = &patch.recipe else {
                        panic!("Sandy Islets changed recipe");
                    };
                    assert_eq!(settings.sea_level, 8);
                    assert_eq!(settings.land_coverage_percent, 32);
                    assert_eq!(settings.islet_count, 5);
                    assert_eq!(settings.max_relief, 3);
                }
                ("Wooded Island", hex_map::V3LayoutSettings::Single(patch)) => {
                    assert_eq!(patch.environment, hex_map::V3EnvironmentSettings::Coastal);
                    assert!(patch.overlays.is_empty());
                    assert!(matches!(
                        &patch.mask,
                        hex_map::PatchMaskSettings::WholeWorld
                    ));
                    let hex_map::V3RecipeSettings::WoodedIsland(settings) = &patch.recipe else {
                        panic!("Wooded Island changed recipe");
                    };
                    assert_eq!(settings.sea_level, 8);
                    assert_eq!(settings.land_coverage_percent, 65);
                    assert_eq!(settings.max_relief, 6);
                    assert_eq!(settings.tree_coverage_percent, 25);
                }
                ("Ocean Archipelagoes", hex_map::V3LayoutSettings::Macro(layout)) => {
                    assert_eq!(layout.macro_radius, 3);
                    assert_eq!(layout.instances.len(), 6);
                    assert_eq!(layout.liquid_connections.len(), 10);
                    assert_eq!(layout.walker_connections.len(), 1);
                }
                _ => panic!("{name} changed its shipped layout kind"),
            }

            let encounter = encounter_of(&scenario);
            assert_eq!(encounter.name, "Island Showcase");
            assert_eq!(encounter.unit_count(EncounterFaction::Player), 3);
            assert_eq!(encounter.unit_count(EncounterFaction::Hostile), 0);
            assert_eq!(encounter.rosters.len(), 1);
            let roster = &encounter.rosters[0];
            assert_eq!(roster.faction, EncounterFaction::Player);
            assert_eq!(
                roster.placement,
                EncounterPlacement::Formation {
                    center: FormationCenter::Anchor("party_start".to_owned()),
                    spread: 1,
                }
            );
            assert_eq!(
                roster
                    .units
                    .iter()
                    .map(|unit| unit.archetype.as_str())
                    .collect::<Vec<_>>(),
                ["hedge-mage", "raider", "wolf"]
            );
            assert!(roster.units.iter().all(|unit| unit.placement.is_none()));
        }
    }

    fn assert_island_scenario_reenters_with_same_world(
        scenario_name: &str,
        required_anchors: &[&str],
    ) {
        let mut app = procedural_gameplay_app(scenario_name);
        enter_screen(&mut app, Screen::Gameplay);

        assert!(app.world().contains_resource::<TerrainReady>());
        let first_fingerprint = app.world().resource::<GenerationReport>().map_fingerprint;
        let first_terrain_occupancy = app.world().resource::<TerrainOccupancy>().clone();
        let first_blockers = app
            .world()
            .resource::<TraversalBlockers>()
            .iter()
            .collect::<Vec<_>>();
        let first_objects = island_object_snapshot(&mut app);
        let first_anchors = {
            let anchors = app.world().resource::<MapAnchors>();
            required_anchors
                .iter()
                .map(|name| {
                    (
                        *name,
                        anchors
                            .get(&MapAnchorId::from(*name))
                            .unwrap_or_else(|| panic!("{scenario_name} omitted {name}")),
                    )
                })
                .collect::<Vec<_>>()
        };
        let first_party = app
            .world()
            .resource::<MapAnchors>()
            .get(&MapAnchorId::from("party_start"))
            .expect("an island world should publish party_start");
        assert_eq!(standing_pos::<Player>(&mut app), Some(first_party));
        assert_eq!(
            player_count(&mut app),
            3,
            "{scenario_name} changed its party"
        );
        assert!(
            standing_pos::<Enemy>(&mut app).is_none(),
            "{scenario_name} should remain a non-combat review world"
        );

        enter_screen(&mut app, Screen::Title);
        assert!(!app.world().contains_resource::<VoxelMap>());
        assert!(!app.world().contains_resource::<MapAnchors>());
        assert!(!app.world().contains_resource::<GenerationReport>());
        assert!(!app.world().contains_resource::<TerrainReady>());
        assert!(!app.world().contains_resource::<TerrainOccupancy>());
        assert!(!app.world().contains_resource::<TraversalBlockers>());
        assert!(standing_pos::<Player>(&mut app).is_none());
        assert!(standing_pos::<Enemy>(&mut app).is_none());
        assert_eq!(player_count(&mut app), 0);
        assert!(island_object_snapshot(&mut app).is_empty());
        assert_eq!(
            app.world_mut()
                .query_filtered::<Entity, With<HexGrid>>()
                .iter(app.world())
                .count(),
            0,
            "{scenario_name} teardown left a rendered grid alive"
        );

        enter_screen(&mut app, Screen::Gameplay);
        assert!(app.world().contains_resource::<TerrainReady>());
        assert_eq!(
            app.world().resource::<GenerationReport>().map_fingerprint,
            first_fingerprint,
            "{scenario_name} changed fingerprint after re-entry"
        );
        let second_anchors = app.world().resource::<MapAnchors>();
        for (name, expected) in first_anchors {
            assert_eq!(
                second_anchors.get(&MapAnchorId::from(name)),
                Some(expected),
                "{scenario_name} changed anchor {name} after re-entry"
            );
        }
        assert_eq!(standing_pos::<Player>(&mut app), Some(first_party));
        assert_eq!(
            player_count(&mut app),
            3,
            "{scenario_name} changed its party"
        );
        assert!(standing_pos::<Enemy>(&mut app).is_none());
        assert_eq!(
            app.world().resource::<TerrainOccupancy>(),
            &first_terrain_occupancy,
            "{scenario_name} rebuilt different terrain occupancy"
        );
        assert_eq!(
            app.world()
                .resource::<TraversalBlockers>()
                .iter()
                .collect::<Vec<_>>(),
            first_blockers,
            "{scenario_name} rebuilt different generated blockers"
        );
        assert_eq!(
            island_object_snapshot(&mut app),
            first_objects,
            "{scenario_name} rebuilt different generated objects"
        );
        assert_eq!(
            app.world_mut()
                .query_filtered::<Entity, With<HexGrid>>()
                .iter(app.world())
                .count(),
            1,
            "{scenario_name} re-entry duplicated the rendered grid"
        );
    }

    fn player_count(app: &mut App) -> usize {
        let world = app.world_mut();
        let mut players = world.query_filtered::<Entity, With<Player>>();
        players.iter(world).count()
    }

    fn island_object_snapshot(app: &mut App) -> Vec<(String, TilePos, u8)> {
        let world = app.world_mut();
        let mut objects = world.query::<&ObjectInstance>();
        let mut snapshot = objects
            .iter(world)
            .map(|instance| {
                (
                    instance.object_id().as_str().to_owned(),
                    instance.origin(),
                    instance.rotation().steps(),
                )
            })
            .collect::<Vec<_>>();
        snapshot.sort_unstable();
        snapshot
    }

    #[test]
    fn island_worlds_reenter_with_same_fingerprint_anchors_and_actors() {
        let cases: &[(&str, &[&str])] = &[
            (
                "Sandy Islets",
                &[
                    "party_start",
                    "hostile_start",
                    "sandy_islets_primary_overlook",
                    "sandy_islets_channel_overlook",
                ],
            ),
            (
                "Wooded Island",
                &[
                    "party_start",
                    "hostile_start",
                    "wooded_island_beach",
                    "wooded_island_clearing",
                    "wooded_island_ridge",
                ],
            ),
            (
                "Ocean Archipelagoes",
                &[
                    "party_start",
                    "hostile_start",
                    "macro_route_end",
                    "archipelago.home_beach",
                    "archipelago.channel_overlook",
                    "archipelago.home_ridge",
                ],
            ),
        ];
        for (scenario_name, required_anchors) in cases {
            assert_island_scenario_reenters_with_same_world(scenario_name, required_anchors);
        }
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
            "failed setup left generated terrain alive on Main Menu"
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
            "Deep Forest",
            "Prairie",
            "Two Rings",
            "Mountain Range",
            "Desert Transition",
            "Desert Plain",
            "Dunes",
            "Desert Oasis Rings",
            "Sandy Islets",
            "Wooded Island",
            "Ocean Archipelagoes",
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
                "{scenario_name} did not finish terrain generation: {:?}",
                app.world()
                    .get_resource::<GameplaySetupFailure>()
                    .map(|failure| failure.reason.as_str())
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
            match (scenario_name, report.recipe_metrics.as_ref()) {
                ("Deep Forest", Some(ProceduralRecipeMetrics::DeepForest(metrics))) => {
                    assert!(metrics.tree_roots > 0, "Deep Forest did not publish trees");
                    assert_eq!(metrics.clearing_count, 3);
                    assert!(
                        (28..=32).contains(&metrics.blocker_coverage_percent),
                        "Deep Forest blocker coverage left its approved band: {}%",
                        metrics.blocker_coverage_percent
                    );
                }
                ("Prairie", Some(ProceduralRecipeMetrics::Prairie(metrics))) => {
                    assert!(metrics.grass_roots > 0, "Prairie did not publish grass");
                    assert!(
                        (65..=75).contains(&metrics.grass_coverage_percent),
                        "Prairie grass coverage left its approved band: {}%",
                        metrics.grass_coverage_percent
                    );
                }
                ("Desert Transition", Some(ProceduralRecipeMetrics::DesertTransition(metrics))) => {
                    assert!(metrics.grass_surfaces > 0);
                    assert!(metrics.transition_surfaces > 0);
                    assert!(metrics.sand_surfaces > 0);
                    assert!(metrics.critical_route_steps > 0);
                }
                ("Desert Plain", Some(ProceduralRecipeMetrics::DesertPlain(metrics))) => {
                    assert_eq!(metrics.sand_surface_percent, 100);
                    assert!(metrics.critical_route_steps > 0);
                }
                ("Dunes", Some(ProceduralRecipeMetrics::Dunes(metrics))) => {
                    assert_eq!(metrics.ridge_count, 5);
                    assert_eq!(metrics.ridge_height, 6);
                    assert!(metrics.crest_surfaces > 0);
                    assert!(metrics.trough_surfaces > 0);
                    assert!(metrics.critical_route_steps > 0);
                }
                ("Sandy Islets", Some(ProceduralRecipeMetrics::SandyIslets(metrics))) => {
                    assert_eq!(metrics.world_columns, 1_801);
                    assert_eq!(metrics.land_components, 5);
                    assert!(metrics.land_surfaces > 0);
                    assert!(metrics.water_cells > 0);
                    assert!(metrics.primary_reachable_surfaces > 0);
                    assert!(metrics.critical_route_steps > 0);
                }
                ("Wooded Island", Some(ProceduralRecipeMetrics::WoodedIsland(metrics))) => {
                    assert_eq!(metrics.world_columns, 4_921);
                    assert!(metrics.land_surfaces > 0);
                    assert!(metrics.water_cells > 0);
                    assert!(metrics.grass_interior_surfaces > 0);
                    assert!(metrics.tree_roots > 0);
                    assert!(metrics.reachable_surfaces > 0);
                    assert!(metrics.critical_route_steps > 0);
                }
                (
                    "Ocean Archipelagoes",
                    Some(ProceduralRecipeMetrics::OceanArchipelago(metrics)),
                ) => {
                    assert_eq!(metrics.world_columns, 18_019);
                    assert_eq!(metrics.macro_cells, 37);
                    assert_eq!(metrics.biome_regions, 6);
                    assert_eq!(metrics.standing_water_seams, 10);
                    assert_eq!(metrics.dry_components, 7);
                    assert_eq!(metrics.scenic_dry_components, 6);
                    assert!(metrics.liquid_cells > 0);
                    assert!(metrics.reachable_surfaces > 0);
                    assert!(metrics.critical_route_steps > 0);
                    assert!(metrics.tree_roots > 0);
                }
                ("Two Rings", Some(ProceduralRecipeMetrics::Ring19(metrics))) => {
                    assert_eq!(metrics.world_columns, 9_241);
                    assert_eq!(metrics.biome_regions, 19);
                    assert_eq!(metrics.reciprocal_seams, 42);
                    assert_eq!(metrics.redundant_regions, 19);
                }
                ("Desert Oasis Rings", Some(ProceduralRecipeMetrics::Ring19(metrics))) => {
                    assert_eq!(metrics.world_columns, 9_241);
                    assert_eq!(metrics.biome_regions, 19);
                    assert_eq!(metrics.reciprocal_seams, 42);
                    assert_eq!(metrics.redundant_regions, 19);
                    assert_eq!(metrics.directed_liquid_seams, 0);
                    assert_eq!(metrics.boundary_liquid_outlets, 0);
                    assert!(metrics.liquid_cells > 0);
                    assert!(metrics.feature_instances >= 12);
                }
                ("Mountain Range", Some(ProceduralRecipeMetrics::MountainRange(metrics))) => {
                    assert_eq!(metrics.world_columns, 18_019);
                    assert_eq!(metrics.macro_cells, 37);
                    assert_eq!(metrics.biome_regions, 30);
                    assert_eq!(metrics.outer_macro_sides, 42);
                    assert!(metrics.critical_route_steps > 0);
                    assert!(metrics.standing_water_seams > 0);
                    assert!(metrics.directed_liquid_seams > 0);
                    assert!((92..=104).contains(&metrics.summit_level));
                    assert!(metrics.high_massif_surfaces >= 100);
                }
                (
                    "Deep Forest"
                    | "Prairie"
                    | "Desert Transition"
                    | "Desert Plain"
                    | "Dunes"
                    | "Sandy Islets"
                    | "Wooded Island"
                    | "Ocean Archipelagoes"
                    | "Two Rings"
                    | "Desert Oasis Rings"
                    | "Mountain Range",
                    metrics,
                ) => {
                    panic!("{scenario_name} published unexpected metrics: {metrics:?}");
                }
                _ => {}
            }
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
                "Volcanic Hills" => &["conflict_center", "bridge", "crater_overlook"],
                "Mountains" => &["conflict_center", "high_pass", "low_bypass"],
                "Caves" => &["conflict_center", "cave_entrance", "deep_chamber"],
                "Waterfall" => &["fall_overlook", "basin_overlook"],
                "Forest" => &["forest_clearing", "prairie_overlook"],
                "Deep Forest" => &["deep_forest_clearing"],
                "Prairie" => &["prairie_overlook"],
                "Desert Transition" => &["transition_center", "grass_overlook", "sand_overlook"],
                "Desert Plain" => &["desert_plain_overlook"],
                "Dunes" => &["dune_crest", "dune_trough"],
                "Desert Oasis Rings" => &[
                    "oasis_overlook",
                    "inner_dune_crest",
                    "outer_dune_crest",
                    "desert_plain_overlook",
                ],
                "Sandy Islets" => &[
                    "sandy_islets_primary_overlook",
                    "sandy_islets_channel_overlook",
                ],
                "Wooded Island" => &[
                    "wooded_island_beach",
                    "wooded_island_clearing",
                    "wooded_island_ridge",
                ],
                "Ocean Archipelagoes" => &[
                    "macro_route_end",
                    "archipelago.home_beach",
                    "archipelago.channel_overlook",
                    "archipelago.home_ridge",
                ],
                "Two Rings" => &[
                    "center_conflict_center",
                    "mountains_a_stream_source_overlook",
                    "caves_deep_chamber",
                    "mountain_waterfall_overlook",
                    "confluence_overlook",
                    "vegetation_gradient_overlook",
                    "fort_outlet_overlook",
                ],
                "Mountain Range" => &[
                    "beach_review",
                    "coast_review",
                    "inland_review",
                    "foothill_review",
                    "massif_front_review",
                    "deep_mountain_base",
                    "deep_mountain_review",
                ],
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
                "Sky Islands" | "Two Rings" => assert!(
                    !special_regions.is_empty(),
                    "{scenario_name} dropped its flight-gated upper layer"
                ),
                "Desert Oasis Rings" => assert!(
                    !special_regions.is_empty(),
                    "Desert Oasis Rings dropped its closed non-route seams"
                ),
                "Mountain Range" => assert!(
                    !special_regions.is_empty(),
                    "Mountain Range dropped its closed non-route macro seams"
                ),
                // Remote dry components are deliberately scenic. Whether Macro represents
                // their closed seams as ordinary disconnection or special movement metadata
                // is validated by the composition contract, not this lifecycle smoke test.
                "Ocean Archipelagoes" => {}
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
            if matches!(scenario_name, "Caves" | "Two Rings") {
                assert!(
                    interiors.surfaces().next().is_some(),
                    "{scenario_name} dropped its exact interior floors"
                );
                assert!(
                    interiors.roof_voxels().next().is_some(),
                    "{scenario_name} dropped its exact cutaway roofs"
                );
            } else {
                assert!(
                    interiors.is_empty(),
                    "{scenario_name} introduced unexpected interior metadata"
                );
            }
            assert!(standing_pos::<Player>(&mut app).is_some());
            if matches!(
                scenario_name,
                "Sandy Islets" | "Wooded Island" | "Ocean Archipelagoes"
            ) {
                assert!(
                    standing_pos::<Enemy>(&mut app).is_none(),
                    "{scenario_name} should remain a non-combat review world"
                );
            } else {
                assert!(standing_pos::<Enemy>(&mut app).is_some());
            }
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

    #[cfg(feature = "test-support")]
    #[test]
    #[ignore = "manual release-mode shipped large-map Character-camera timing diagnostic"]
    fn shipped_large_maps_character_collision_release_timing() {
        for (scenario_name, expected_columns, budget) in [
            ("Two Rings", 9_241, std::time::Duration::from_millis(1)),
            (
                "Mountain Range",
                18_019,
                std::time::Duration::from_millis(2),
            ),
            (
                "Crystal Mountain",
                18_019,
                std::time::Duration::from_millis(2),
            ),
            (
                "Ocean Archipelagoes",
                18_019,
                std::time::Duration::from_millis(2),
            ),
        ] {
            let mut app = procedural_gameplay_app(scenario_name);
            enter_screen(&mut app, Screen::Gameplay);
            assert!(
                app.world().contains_resource::<TerrainReady>(),
                "{scenario_name} did not finish terrain generation: {:?}",
                app.world()
                    .get_resource::<GameplaySetupFailure>()
                    .map(|failure| failure.reason.as_str())
            );

            let settings: hex_assets::CameraSettings =
                ron::from_str(include_str!("../../../assets/config/camera.ron"))
                    .expect("the shipped camera settings should deserialize");
            let mut supports = app
                .world()
                .resource::<MapAnchors>()
                .iter()
                .map(|(_id, position)| position)
                .collect::<Vec<_>>();
            supports.sort_unstable();
            supports.dedup();
            let projection = {
                let world = app.world_mut();
                let mut tiles = world.query_filtered::<(&TilePos, &HexSpan), With<HexTile>>();
                tiles
                    .iter(world)
                    .map(|(position, span)| (*position, *span))
                    .collect::<Vec<_>>()
            };

            let profile = hex_world::camera::test_support::profile_character_collision(
                &projection,
                &supports,
                &settings,
                10_000,
            )
            .expect("the shipped public terrain projection should support camera diagnostics");

            assert_eq!(
                profile.columns, expected_columns,
                "the camera diagnostic must use every shipped {scenario_name} column"
            );
            assert!(
                profile.spans >= profile.columns,
                "each public column should publish at least one exact material run"
            );
            assert_eq!(profile.supports, supports.len());
            assert_ne!(
                profile.result_checksum, 0,
                "the timed collision results must remain observable"
            );
            eprintln!(
                "shipped {scenario_name} Character collision diagnostic (release): \
             columns={}, spans={}, supports={}, queries={}, index_build={:?}, \
             index_rebuild_p95={:?}, index_rebuild_worst={:?}, query_p95={:?}, \
             query_worst={:?}",
                profile.columns,
                profile.spans,
                profile.supports,
                profile.queries,
                profile.index_build,
                profile.index_rebuild_p95,
                profile.index_rebuild_worst,
                profile.query_p95,
                profile.query_worst,
            );
            assert!(
                profile.query_p95 < budget,
                "shipped {scenario_name} Character collision p95 {:?} breached the {budget:?} release budget",
                profile.query_p95
            );
        }
    }

    #[test]
    fn volcanic_hills_scenario_uses_the_native_volcano_contract() {
        let mut app = procedural_gameplay_app("Volcanic Hills");
        enter_screen(&mut app, Screen::Gameplay);

        assert!(app.world().contains_resource::<TerrainReady>());
        let report = app.world().resource::<GenerationReport>();
        assert_eq!(
            report.semantic_plan_fingerprint,
            Some(6_901_546_631_227_104_688)
        );
        assert_eq!(report.map_fingerprint, 7_940_527_797_927_330_083);
        assert_eq!(report.valid_candidates, 8);
        let Some(ProceduralRecipeMetrics::Volcano(metrics)) = &report.recipe_metrics else {
            panic!(
                "Volcanic Hills published the wrong recipe metrics: {:?}",
                report.recipe_metrics
            );
        };
        assert_eq!(metrics.summit_relief, 20);
        assert_eq!(metrics.bridge_clearance, 4);

        let anchors = app.world().resource::<MapAnchors>();
        for required in [
            "party_start",
            "hostile_start",
            "conflict_center",
            "bridge",
            "crater_overlook",
        ] {
            assert!(
                anchors.get(&MapAnchorId::from(required)).is_some(),
                "Volcanic Hills omitted {required}"
            );
        }
        assert!(
            anchors
                .get(&MapAnchorId::from("alternate_crossing"))
                .is_none(),
            "the native Volcano resurrected the removed cooled crossing"
        );
        assert!(standing_pos::<Player>(&mut app).is_some());
        assert!(standing_pos::<Enemy>(&mut app).is_some());
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
