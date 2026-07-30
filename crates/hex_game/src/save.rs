//! One atomic, build-bound exploration resume slot.

use std::collections::{BTreeMap, BTreeSet};

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::ui::InteractionDisabled;
use hex_assets::{Scenario, ScenarioLibrary};
use hex_combat::EncounterResolution;
use hex_core::{
    Busy, CommandQueue, GameplaySetup, GameplaySetupFailure, HexSpan, HexTile, InputAction,
    InputBindings, Mode, PartyFormation, Pause, PendingDecision, ResolvedMapSeed, Screen, TilePos,
    UnitId,
};
use hex_lattice::LatticeState;
use hex_map::{MapSettings, TerrainSettings};
use hex_units::{Downed, Faction, MovingTo, Selected, Standing, StandsOn};
use serde::{Deserialize, Serialize};

use crate::scenarios::{ActiveScenario, ScenarioToLoad};
use crate::screens::title::{ContinueStatusText, ContinuesGame};
use crate::storage::{read, write_atomic, StoragePaths};

const RESUME_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
struct ResumeFile {
    format_version: u32,
    build_version: String,
    scenario_name: String,
    scenario_digest: u64,
    resolved_seed: Option<u64>,
    generator_version: Option<u32>,
    formation: PartyFormation,
    selected: Option<UnitId>,
    units: Vec<UnitResume>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
struct UnitResume {
    id: UnitId,
    faction: Faction,
    position: TilePos,
    lattice: Option<LatticeState>,
    downed: bool,
}

#[derive(Resource, Debug, Clone, Default)]
enum ResumeStatus {
    #[default]
    Missing,
    Available(ResumeFile),
    Invalid(String),
}

/// Latest save feedback shown while paused.
#[derive(Resource, Debug, Default, Clone)]
pub(crate) struct ResumeNotice(pub(crate) Option<String>);

#[derive(Resource, Debug, Clone)]
pub(crate) struct PendingResume(ResumeFile);

pub(crate) fn plugin(app: &mut App) {
    app.init_resource::<StoragePaths>()
        .init_resource::<ResumeStatus>()
        .init_resource::<ResumeNotice>()
        .add_systems(Startup, load_resume)
        .add_systems(
            Update,
            (sync_continue_button, continue_game, save_exploration),
        )
        .add_systems(
            OnEnter(Screen::Gameplay),
            restore_pending_resume.in_set(GameplaySetup::Restore),
        )
        .add_systems(OnEnter(Screen::Title), clear_abandoned_resume);
}

fn clear_abandoned_resume(mut commands: Commands, pending: Option<Res<PendingResume>>) {
    if pending.is_some() {
        commands.remove_resource::<PendingResume>();
    }
}

fn load_resume(paths: Res<StoragePaths>, mut status: ResMut<ResumeStatus>) {
    *status = match read(&paths.resume) {
        Ok(text) => match ron::from_str::<ResumeFile>(&text)
            .map_err(|error| format!("Resume data could not be parsed: {error}"))
            .and_then(|resume| {
                validate_resume(&resume)?;
                Ok(resume)
            }) {
            Ok(resume) => ResumeStatus::Available(resume),
            Err(reason) => ResumeStatus::Invalid(reason),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => ResumeStatus::Missing,
        Err(error) => ResumeStatus::Invalid(format!("Resume data could not be read: {error}")),
    };
}

fn validate_resume(resume: &ResumeFile) -> Result<(), String> {
    if resume.format_version != RESUME_VERSION {
        return Err(format!(
            "Resume format {} is incompatible with {}.",
            resume.format_version, RESUME_VERSION
        ));
    }
    if resume.build_version != build_identity() {
        return Err(format!(
            "Resume build {} does not match this build {}.",
            resume.build_version,
            build_identity()
        ));
    }
    if resume.scenario_name.trim().is_empty() {
        return Err("Resume data has no scenario name.".to_owned());
    }
    if resume.units.is_empty() {
        return Err("Resume data has no units.".to_owned());
    }
    let unique: BTreeSet<UnitId> = resume.units.iter().map(|unit| unit.id).collect();
    if unique.len() != resume.units.len() {
        return Err("Resume data repeats a unit id.".to_owned());
    }
    if resume
        .selected
        .is_some_and(|selected| !unique.contains(&selected))
    {
        return Err("Resume data selects a unit that is not present.".to_owned());
    }
    Ok(())
}

fn sync_continue_button(
    status: Res<ResumeStatus>,
    mut commands: Commands,
    buttons: Query<Entity, With<ContinuesGame>>,
    mut text: Query<&mut Text, With<ContinueStatusText>>,
) {
    if !status.is_changed() && buttons.is_empty() {
        return;
    }
    let (enabled, message) = match status.as_ref() {
        ResumeStatus::Missing => (false, "No exploration resume has been saved.".to_owned()),
        ResumeStatus::Available(resume) => (
            true,
            format!(
                "Resume {} from its last explicit save.",
                resume.scenario_name
            ),
        ),
        ResumeStatus::Invalid(reason) => (false, reason.clone()),
    };
    for button in &buttons {
        if enabled {
            commands.entity(button).remove::<InteractionDisabled>();
        } else {
            commands.entity(button).insert(InteractionDisabled);
        }
    }
    for mut text in &mut text {
        text.0.clone_from(&message);
    }
}

fn continue_game(
    clicked: Query<&Interaction, (Changed<Interaction>, With<ContinuesGame>)>,
    mut status: ResMut<ResumeStatus>,
    library: Option<Res<ScenarioLibrary>>,
    mut commands: Commands,
    mut next: ResMut<NextState<Screen>>,
) {
    if !clicked
        .iter()
        .any(|interaction| *interaction == Interaction::Pressed)
    {
        return;
    }
    let ResumeStatus::Available(resume) = status.as_ref() else {
        return;
    };
    let resume = resume.clone();
    let Some(library) = library else {
        return;
    };
    let Some(scenario) = library
        .scenarios
        .iter()
        .find(|scenario| scenario.name == resume.scenario_name)
    else {
        let reason = format!(
            "The saved scenario {:?} is no longer available.",
            resume.scenario_name
        );
        commands.insert_resource(GameplaySetupFailure::new(reason.clone()));
        *status = ResumeStatus::Invalid(reason);
        return;
    };
    if scenario_digest(scenario) != resume.scenario_digest {
        let reason = format!(
            "The saved scenario {:?} changed and cannot be resumed.",
            resume.scenario_name
        );
        commands.insert_resource(GameplaySetupFailure::new(reason.clone()));
        *status = ResumeStatus::Invalid(reason);
        return;
    }
    if scenario.generation_seed.is_some() != resume.resolved_seed.is_some() {
        let reason = format!(
            "The saved seed contract for {:?} is incompatible.",
            resume.scenario_name
        );
        commands.insert_resource(GameplaySetupFailure::new(reason.clone()));
        *status = ResumeStatus::Invalid(reason);
        return;
    }
    commands.insert_resource(ScenarioToLoad {
        scenario: scenario.clone(),
        resolved_seed: resume.resolved_seed.map(ResolvedMapSeed),
        encounter_override: None,
    });
    commands.insert_resource(PendingResume(resume));
    next.set(Screen::Loading);
}

#[derive(SystemParam)]
struct SaveWorld<'w, 's> {
    screen: Res<'w, State<Screen>>,
    mode: Option<Res<'w, State<Mode>>>,
    pause: Option<Res<'w, State<Pause>>>,
    queue: Res<'w, CommandQueue>,
    pending: Res<'w, PendingDecision>,
    resolution: Res<'w, EncounterResolution>,
    active: Option<Res<'w, ActiveScenario>>,
    lab_session: Option<Res<'w, crate::screens::combat_lab::CombatLabSession>>,
    map: Option<Res<'w, MapSettings>>,
    formation: Res<'w, PartyFormation>,
    moving: Query<'w, 's, (), Or<(With<MovingTo>, With<Busy>)>>,
    units: Query<
        'w,
        's,
        (
            &'static UnitId,
            &'static Faction,
            &'static StandsOn,
            Option<&'static LatticeState>,
            Has<Downed>,
            Has<Selected>,
        ),
    >,
}

fn save_exploration(
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<InputBindings>,
    world: SaveWorld,
    paths: Res<StoragePaths>,
    mut status: ResMut<ResumeStatus>,
    mut notice: ResMut<ResumeNotice>,
) {
    if *world.screen.get() != Screen::Gameplay || !bindings.just_pressed(&keys, InputAction::Save) {
        return;
    }
    if world.lab_session.is_some() {
        notice.0 =
            Some("Resume not saved: Combat Lab sessions are temporary test fixtures.".to_owned());
        return;
    }
    let safe = world
        .mode
        .as_deref()
        .is_some_and(|mode| *mode.get() == Mode::Exploring)
        && world
            .pause
            .as_deref()
            .is_some_and(|pause| *pause.get() == Pause(true))
        && world.queue.is_empty()
        && !world.pending.is_open()
        && !world.resolution.is_resolved()
        && world.moving.is_empty();
    if !safe {
        notice.0 = Some(
            "Resume not saved: pause during safe exploration with no movement or decision pending."
                .to_owned(),
        );
        return;
    }
    let (Some(active), Some(map)) = (world.active, world.map) else {
        notice.0 = Some("Resume not saved: scenario setup is incomplete.".to_owned());
        return;
    };

    let mut snapshots: Vec<UnitResume> = world
        .units
        .iter()
        .map(|(id, faction, standing, lattice, downed, _)| UnitResume {
            id: *id,
            faction: *faction,
            position: standing.0.pos,
            lattice: lattice.cloned(),
            downed,
        })
        .collect();
    snapshots.sort_by_key(|unit| unit.id);
    let selected = world
        .units
        .iter()
        .find_map(|(id, _, _, _, _, selected)| selected.then_some(*id));
    let resume = ResumeFile {
        format_version: RESUME_VERSION,
        build_version: build_identity().to_owned(),
        scenario_name: active.0.scenario.name.clone(),
        scenario_digest: scenario_digest(&active.0.scenario),
        resolved_seed: active.0.resolved_seed.map(|seed| seed.0),
        generator_version: generator_version(&map),
        formation: world.formation.as_ref().clone(),
        selected,
        units: snapshots,
    };
    if let Err(reason) = validate_resume(&resume) {
        notice.0 = Some(format!("Resume not saved: {reason}"));
        return;
    }
    let serialized = match ron::ser::to_string_pretty(&resume, ron::ser::PrettyConfig::new()) {
        Ok(serialized) => serialized,
        Err(error) => {
            notice.0 = Some(format!("Resume could not be encoded: {error}"));
            return;
        }
    };
    match write_atomic(&paths.resume, &serialized) {
        Ok(()) => {
            *status = ResumeStatus::Available(resume);
            notice.0 = Some("Exploration resume saved.".to_owned());
        }
        Err(error) => {
            notice.0 = Some(format!("Resume could not be saved: {error}"));
        }
    }
}

fn restore_pending_resume(
    mut commands: Commands,
    pending: Option<Res<PendingResume>>,
    map: Res<MapSettings>,
    tiles: Query<(&TilePos, &HexSpan), With<HexTile>>,
    mut units: Query<(
        Entity,
        &UnitId,
        &Faction,
        &mut StandsOn,
        &mut Transform,
        Option<&mut LatticeState>,
        Has<Downed>,
        Has<Selected>,
    )>,
    mut formation: ResMut<PartyFormation>,
    mut next: ResMut<NextState<Screen>>,
) {
    let Some(pending) = pending else { return };
    let resume = &pending.0;
    if generator_version(&map) != resume.generator_version {
        fail_restore(
            &mut commands,
            &mut next,
            format!(
                "The saved generator version {:?} does not match {:?}.",
                resume.generator_version,
                generator_version(&map)
            ),
        );
        return;
    }

    let saved: BTreeMap<UnitId, &UnitResume> =
        resume.units.iter().map(|unit| (unit.id, unit)).collect();
    if saved.len() != units.iter().count() {
        fail_restore(
            &mut commands,
            &mut next,
            "The saved roster no longer matches this scenario.".to_owned(),
        );
        return;
    }

    for (entity, id, faction, mut standing, mut transform, lattice, downed, selected) in &mut units
    {
        let Some(snapshot) = saved.get(id) else {
            fail_restore(
                &mut commands,
                &mut next,
                format!("The saved roster is missing unit {}.", id.0),
            );
            return;
        };
        if snapshot.faction != *faction {
            fail_restore(
                &mut commands,
                &mut next,
                format!("The saved faction for unit {} changed.", id.0),
            );
            return;
        }
        let Some((_, span)) = tiles
            .iter()
            .find(|(position, _)| **position == snapshot.position)
        else {
            fail_restore(
                &mut commands,
                &mut next,
                format!("The saved position for unit {} no longer exists.", id.0),
            );
            return;
        };
        standing.0 = Standing {
            pos: snapshot.position,
            span: *span,
        };
        transform.translation = standing.0.world_position();
        match (snapshot.lattice.as_ref(), lattice) {
            (Some(saved), Some(mut current)) => *current = saved.clone(),
            (None, None) => {}
            _ => {
                fail_restore(
                    &mut commands,
                    &mut next,
                    format!("The saved lattice for unit {} no longer matches.", id.0),
                );
                return;
            }
        }
        if snapshot.downed && !downed {
            commands.entity(entity).insert(Downed);
        } else if !snapshot.downed && downed {
            commands.entity(entity).remove::<Downed>();
        }
        let should_select = resume.selected == Some(*id);
        if should_select && !selected {
            commands.entity(entity).insert(Selected);
        } else if !should_select && selected {
            commands.entity(entity).remove::<Selected>();
        }
    }
    *formation = resume.formation.clone();
    commands.remove_resource::<PendingResume>();
    info!("restored exploration resume for {}", resume.scenario_name);
}

fn fail_restore(commands: &mut Commands, next: &mut NextState<Screen>, reason: String) {
    commands.insert_resource(GameplaySetupFailure::new(format!(
        "The exploration resume is incompatible: {reason}"
    )));
    commands.insert_resource(ResumeStatus::Invalid(reason));
    commands.remove_resource::<PendingResume>();
    next.set(Screen::Title);
}

fn generator_version(map: &MapSettings) -> Option<u32> {
    match &map.terrain {
        TerrainSettings::Procedural(settings) => Some(settings.generator_version()),
        TerrainSettings::Showcase(_) | TerrainSettings::Perlin(_) => None,
    }
}

fn scenario_digest(scenario: &Scenario) -> u64 {
    let mut digest = 0xcbf2_9ce4_8422_2325_u64;
    let mut fold = |bytes: &[u8]| {
        for byte in bytes {
            digest ^= u64::from(*byte);
            digest = digest.wrapping_mul(0x0000_0100_0000_01B3);
        }
        digest ^= 0xff;
        digest = digest.wrapping_mul(0x0000_0100_0000_01B3);
    };
    fold(build_identity().as_bytes());
    fold(scenario.name.as_bytes());
    fold(scenario.world.as_bytes());
    fold(scenario.lighting.as_bytes());
    fold(scenario.encounter.as_bytes());
    if let Some(seed) = scenario.generation_seed {
        fold(&seed.to_le_bytes());
    }
    if let Some(hours) = scenario.starting_time_hours {
        fold(&hours.to_bits().to_le_bytes());
    }
    // Coarse by design: this is a disposable slot, so any shipped gameplay or world
    // content change refuses it instead of pretending to migrate semantic ids.
    for content in [
        include_str!("../../../assets/config/scenarios.ron"),
        include_str!("../../../assets/config/formations.ron"),
        include_str!("../../../assets/config/lattices.ron"),
        include_str!("../../../assets/config/elements.ron"),
        include_str!("../../../assets/config/spells.ron"),
        include_str!("../../../assets/config/ai_profiles.ron"),
        include_str!("../../../assets/config/combat.ron"),
        include_str!("../../../assets/config/perception.ron"),
        include_str!("../../../assets/config/player.ron"),
        include_str!("../../../assets/config/substances.ron"),
        include_str!("../../../assets/config/lighting.ron"),
        include_str!("../../../assets/config/lighting/overcast.ron"),
        include_str!("../../../assets/config/world.ron"),
        include_str!("../../../assets/config/worlds/flat-combat.ron"),
        include_str!("../../../assets/config/worlds/procedural-caves.ron"),
        include_str!("../../../assets/config/worlds/procedural-forest.ron"),
        include_str!("../../../assets/config/worlds/procedural-frozen.ron"),
        include_str!("../../../assets/config/worlds/procedural-hills.ron"),
        include_str!("../../../assets/config/worlds/procedural-mountains.ron"),
        include_str!("../../../assets/config/worlds/procedural-prairie.ron"),
        include_str!("../../../assets/config/worlds/procedural-sky-islands.ron"),
        include_str!("../../../assets/config/worlds/procedural-volcanic.ron"),
        include_str!("../../../assets/config/worlds/procedural-waterfall.ron"),
        include_str!("../../../assets/config/worlds/rolling-hills.ron"),
        include_str!("../../../assets/config/encounters/ability-lab.ron"),
        include_str!("../../../assets/config/encounters/anchored-skirmish.ron"),
        include_str!("../../../assets/config/encounters/bridge-crossing.ron"),
        include_str!("../../../assets/config/encounters/open-ground.ron"),
        include_str!("../../../assets/config/encounters/party-trial.ron"),
        include_str!("../../../assets/config/encounters/raider-mirror.ron"),
    ] {
        fold(content.as_bytes());
    }
    digest
}

fn build_identity() -> &'static str {
    option_env!("HEX_GAME_BUILD_ID").unwrap_or(env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use hex_assets::ScenarioCategory;

    use super::*;

    fn scenario() -> Scenario {
        Scenario {
            name: "Party Trial".to_owned(),
            category: ScenarioCategory::Demo,
            blurb: "Integrated.".to_owned(),
            world: "config/world.ron".to_owned(),
            lighting: "config/lighting.ron".to_owned(),
            generation_seed: None,
            starting_time_hours: None,
            encounter: "config/encounters/party-trial.ron".to_owned(),
        }
    }

    fn resume() -> ResumeFile {
        ResumeFile {
            format_version: RESUME_VERSION,
            build_version: build_identity().to_owned(),
            scenario_name: "Party Trial".to_owned(),
            scenario_digest: scenario_digest(&scenario()),
            resolved_seed: None,
            generator_version: None,
            formation: PartyFormation::default(),
            selected: Some(UnitId(0)),
            units: vec![UnitResume {
                id: UnitId(0),
                faction: Faction::Player,
                position: TilePos::ORIGIN,
                lattice: None,
                downed: false,
            }],
        }
    }

    #[test]
    fn resume_round_trip_is_stable() {
        let original = resume();
        let text = ron::ser::to_string_pretty(&original, ron::ser::PrettyConfig::new())
            .expect("resume should encode");
        let decoded: ResumeFile = ron::from_str(&text).expect("resume should decode");
        assert_eq!(decoded, original);
        assert_eq!(validate_resume(&decoded), Ok(()));
    }

    #[test]
    fn corrupt_identity_and_build_drift_are_refused() {
        let mut duplicate = resume();
        let repeated = duplicate
            .units
            .first()
            .expect("fixture has one unit")
            .clone();
        duplicate.units.push(repeated);
        assert!(validate_resume(&duplicate).is_err());

        let mut old_build = resume();
        old_build.build_version = "old".to_owned();
        assert!(validate_resume(&old_build).is_err());
    }

    #[test]
    fn scenario_changes_invalidate_the_digest() {
        let original = scenario();
        let mut changed = original.clone();
        changed.encounter = "config/encounters/other.ron".to_owned();
        assert_ne!(scenario_digest(&original), scenario_digest(&changed));
    }
}
