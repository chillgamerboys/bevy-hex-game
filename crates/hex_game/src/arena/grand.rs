//! Grand's single resume slot. Owners stage their records before play is enabled.
use std::{path::PathBuf, thread::JoinHandle};

use bevy::prelude::*;
use hex_arena::{ArenaInput, ArenaSession, ArenaTuning, GrandCheckpointIdentity, GrandTuning};
use hex_core::arena::{
    ArenaActorStreamInterests, ArenaBurrowOutcome, ArenaMap, ArenaReset, ArenaSelection,
    ArenaStreamInterest, ArenaSystems, ArenaTerrainView, ArenaTick, ArenaVoxelGeometry,
};
use hex_core::{ocean::OceanSimulationTime, TerrainImpactOutcome};
use hex_world_runtime::{
    CancellationToken, CheckpointIdentity, CheckpointLimits, CheckpointToken, OwnerRecord,
    SessionCheckpoint, SessionCheckpointStore,
};
use serde::{Deserialize, Serialize};

use super::{ArenaFrame, ViewState};

#[cfg(all(test, feature = "test-support"))]
mod tests;

// Bump when durable gameplay semantics change, even if the RON content is unchanged.
const CONTENT_VERSION: u32 = 1;
const AUTOSAVE_TICKS: u64 = 30 * 120;
const APP_FORMAT: &str = "grand-app-v1";
const GAMEPLAY_FORMAT: &str = "grand-gameplay-v1";

#[derive(Clone, Copy)]
pub(super) enum Request {
    Primary,
    NewRun,
    ConfirmNew,
    CancelNew,
    SaveQuit,
}

#[derive(Resource, Default)]
pub(super) struct State {
    pub confirmation: bool,
    pub available: bool,
    pub status: String,
    pub biome_notice: String,
    pub request: Option<Request>,
    identity: Option<CheckpointIdentity>,
    content_revision: String,
    token: Option<CheckpointToken>,
    configured_generation: Option<u64>,
    rejected_generation: Option<u64>,
    tuning: Option<GrandTuning>,
    refused: bool,
    active: bool,
    last_saved_tick: u64,
    last_reward_key: String,
    force_save: bool,
    quit_after_save: bool,
    writing: Option<JoinHandle<Result<CheckpointToken, String>>>,
    restoring: Option<Restoring>,
    biome: Option<&'static str>,
    biome_candidate: Option<(&'static str, u64)>,
    biome_notice_until: u64,
}

struct Restoring {
    session: ArenaSession,
    app: AppCheckpoint,
    position: Vec3,
    started: std::time::Instant,
}

impl State {
    pub fn busy(&self) -> bool {
        self.writing.is_some() || self.restoring.is_some()
    }

    pub fn request_new(&mut self) {
        if !self.busy() {
            self.confirmation = true;
            self.status = "Begin a new expedition? Your current resume will be replaced. Confirm New Run or Cancel.".into();
        }
    }
}

#[derive(Resource)]
struct Settling;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AppCheckpoint {
    content_revision: String,
    sites_fingerprint: u64,
    tick: u64,
    environment_seconds: f64,
    yaw: f32,
    pitch: f32,
    third_person: bool,
}

pub(super) fn install(app: &mut App) {
    app.init_resource::<State>()
        .init_resource::<ArenaActorStreamInterests>()
        .configure_sets(
            ArenaTick,
            ArenaSystems::Simulate.run_if(not(resource_exists::<Settling>)),
        )
        .add_systems(
            Update,
            before_frame
                .before(ArenaFrame::Tick)
                .after(ArenaFrame::Input),
        )
        .add_systems(
            Update,
            update.after(ArenaFrame::Tick).before(ArenaFrame::Present),
        );
}

fn slot_path() -> PathBuf {
    let storage = crate::storage::StoragePaths::default();
    storage.preferences.with_file_name("grand-v4-resume")
}

fn store(state: &State) -> Result<SessionCheckpointStore, String> {
    SessionCheckpointStore::new(
        slot_path(),
        state.identity.clone().ok_or("Grand package is not ready")?,
        CheckpointLimits::default(),
    )
    .map_err(|error| error.to_string())
}

fn gameplay_identity(state: &State) -> Result<GrandCheckpointIdentity, String> {
    Ok(GrandCheckpointIdentity {
        world_id: state
            .identity
            .as_ref()
            .ok_or("Grand package is not ready")?
            .world_id
            .clone(),
        content_revision: state.content_revision.clone(),
    })
}

fn content() -> Result<(GrandTuning, ArenaTuning, String), String> {
    let root = bevy::asset::io::file::FileAssetReader::get_base_path().join("assets/config");
    let mut digest = xxhash_rust::xxh3::Xxh3::new();
    let mut grand = None;
    let mut arena = None;
    for name in [
        "arena.ron",
        "arena/grand-v4.ron",
        "substances.ron",
        "terrain_damage.ron",
        "elements.ron",
    ] {
        let path = root.join(name);
        let bytes = std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        digest.update(name.as_bytes());
        digest.update(
            &u64::try_from(bytes.len())
                .map_err(|error| error.to_string())?
                .to_le_bytes(),
        );
        digest.update(&bytes);
        if name == "arena/grand-v4.ron" {
            grand = Some(
                ron::de::from_bytes(&bytes).map_err(|error| format!("Grand tuning: {error}"))?,
            );
        } else if name == "arena.ron" {
            let tuning: ArenaTuning =
                ron::de::from_bytes(&bytes).map_err(|error| format!("Arena tuning: {error}"))?;
            tuning.validate()?;
            arena = Some(tuning);
        }
    }
    let package = hex_map::arena::streamed::package_path_for(ArenaMap::GrandV4);
    for name in ["grand-overview.ron", "arena-sites.ron", "grand-biomes.ron"] {
        let path = package.join(name);
        let bytes = std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        digest.update(name.as_bytes());
        digest.update(
            &u64::try_from(bytes.len())
                .map_err(|error| error.to_string())?
                .to_le_bytes(),
        );
        digest.update(&bytes);
    }
    Ok((
        grand.ok_or("Grand tuning is missing")?,
        arena.ok_or("Arena tuning is missing")?,
        format!("grand-{CONTENT_VERSION}-{:016x}", digest.digest()),
    ))
}

fn before_frame(world: &mut World) {
    if world.resource::<ArenaSelection>().map != ArenaMap::GrandV4 {
        world
            .resource_mut::<ArenaActorStreamInterests>()
            .positions
            .clear();
        return;
    }
    let generation = world.resource::<ArenaReset>().generation;
    if world.resource::<State>().rejected_generation == Some(generation)
        || world
            .resource::<ArenaTerrainView>()
            .package_identity
            .is_none()
    {
        return;
    }
    if world.resource::<State>().configured_generation != Some(generation) {
        let result = content().and_then(|(tuning, arena, revision)| {
            world
                .resource_mut::<ArenaSession>()
                .configure_grand(tuning.clone())?;
            world.insert_resource(arena);
            let mut state = world.resource_mut::<State>();
            state.tuning = Some(tuning);
            state.content_revision = revision;
            state.configured_generation = Some(generation);
            Ok(())
        });
        if let Err(error) = result {
            world.resource_mut::<State>().rejected_generation = Some(generation);
            fail(world, format!("Grand content could not be loaded: {error}"));
            return;
        }
    }
    let (positions, destination) = world.resource::<State>().restoring.as_ref().map_or_else(
        || {
            (
                world.resource::<ArenaSession>().grand_actor_interests(),
                None,
            )
        },
        |restore| {
            (
                restore.session.grand_actor_interests(),
                Some(restore.position),
            )
        },
    );
    world.resource_mut::<ArenaActorStreamInterests>().positions = positions;
    if let Some(position) = destination {
        world.resource_mut::<ArenaStreamInterest>().position = position;
        world.resource_mut::<ViewState>().pause();
    }
}

fn fail(world: &mut World, message: String) {
    error!("{message}");
    world.resource_mut::<ViewState>().pause();
    let mut state = world.resource_mut::<State>();
    state.status = message;
    state.quit_after_save = false;
}

fn read_app(
    snapshot: &SessionCheckpoint,
    state: &State,
    sites: u64,
) -> Result<AppCheckpoint, String> {
    let bytes = snapshot
        .record("app", "session", APP_FORMAT)
        .map_err(|error| error.to_string())?
        .ok_or("Save is missing its application record")?;
    let app: AppCheckpoint = ron::de::from_bytes(&bytes).map_err(|error| error.to_string())?;
    if app.content_revision != state.content_revision || app.sites_fingerprint != sites {
        return Err("This save belongs to different gameplay content or authored sites. It has been preserved.".into());
    }
    if !app.yaw.is_finite()
        || !app.pitch.is_finite()
        || app.pitch.abs() > 1.49
        || !app.environment_seconds.is_finite()
        || app.environment_seconds < 0.0
        || app.tick > 9_007_199_254_740_992
        || (app.environment_seconds
            - OceanSimulationTime::from_fixed_tick(0, app.tick, f64::from(hex_arena::STEP)).seconds)
            .abs()
            > 0.01
    {
        return Err("Save camera or environment clock is invalid".into());
    }
    Ok(app)
}

fn inspect_slot(world: &mut World) -> Result<(), String> {
    if world.resource::<State>().tuning.is_none() {
        return Ok(());
    }
    let Some(package) = world
        .resource::<ArenaTerrainView>()
        .package_identity
        .clone()
    else {
        return Ok(());
    };
    if world
        .resource::<State>()
        .identity
        .as_ref()
        .is_some_and(|identity| identity.manifest_fingerprint == package.manifest_fingerprint)
    {
        return Ok(());
    }
    let mut state = world.resource_mut::<State>();
    state.identity = Some(CheckpointIdentity {
        world_id: package.world_id,
        manifest_fingerprint: package.manifest_fingerprint,
        content_version: CONTENT_VERSION,
    });
    state.available = false;
    state.token = None;
    state.refused = false;
    match store(&state)?.load() {
        Ok(Some(snapshot)) => {
            // Validate the mandatory content identity before offering Continue.
            read_app(
                &snapshot,
                &state,
                package
                    .sites_fingerprint
                    .ok_or("Grand has no authored site identity")?,
            )?;
            state.token = Some(snapshot.token());
            state.available = true;
            state.status = "Continue your saved expedition, or choose New Run.".into();
        }
        Ok(None) => state.status = "No saved expedition. Choose New Run to begin.".into(),
        Err(error) => {
            state.refused = true;
            return Err(format!(
                "Cannot continue: {error}. The save has been preserved. New Run archives it before starting."
            ));
        }
    }
    Ok(())
}

fn reward_key(session: &ArenaSession) -> String {
    session
        .grand_progress()
        .map_or_else(String::new, |progress| {
            format!(
                "{:?}/{}/{:?}/{}/{:?}/{:?}",
                progress.shrines,
                progress.teleport_unlocked,
                progress.respawn_anchor,
                progress.deaths,
                progress.discovered_shrines,
                session.progress(),
            )
        })
}

fn update_biome(world: &mut World) {
    if !world.resource::<State>().active {
        return;
    }
    let session = world.resource::<ArenaSession>();
    let tick = session.tick;
    let entered = session
        .actors
        .iter()
        .find(|actor| actor.id == 0 && actor.hp > 0.0)
        .and_then(|actor| {
            world
                .get_resource::<hex_map::arena::streamed::StreamedArena>()
                .and_then(|map| map.biome_at(actor.feet))
        });
    let mut state = world.resource_mut::<State>();
    if tick >= state.biome_notice_until {
        state.biome_notice.clear();
    }
    let Some(entered) = entered else {
        return;
    };
    if state.biome == Some(entered) {
        state.biome_candidate = None;
    } else if let Some((candidate, since)) = state.biome_candidate {
        if candidate != entered {
            state.biome_candidate = Some((entered, tick));
        } else if tick.saturating_sub(since) >= 60 {
            state.biome = Some(entered);
            state.biome_candidate = None;
            state.biome_notice = entered.into();
            state.biome_notice_until = tick.saturating_add(360);
        }
    } else {
        state.biome_candidate = Some((entered, tick));
    }
}

fn update(world: &mut World) {
    if world.resource::<ArenaSelection>().map != ArenaMap::GrandV4
        || world.resource::<ViewState>().capture.is_some()
    {
        return;
    }
    update_biome(world);
    if let Err(error) = inspect_slot(world) {
        fail(world, error);
    }
    finish_write(world);
    if world.resource::<State>().restoring.is_some() {
        finish_restore(world);
        return;
    }
    let request = world.resource_mut::<State>().request.take();
    if let Some(request) = request {
        if world.resource::<State>().busy() {
            world.resource_mut::<State>().request = Some(request);
        } else {
            match request {
                Request::Primary if world.resource::<State>().available => {
                    if let Err(error) = begin_restore(world) {
                        fail(
                            world,
                            format!("Cannot continue: {error}. Your save is preserved."),
                        );
                    }
                }
                Request::Primary | Request::NewRun => world.resource_mut::<State>().request_new(),
                Request::CancelNew => {
                    let mut state = world.resource_mut::<State>();
                    state.confirmation = false;
                    state.status.clear();
                }
                Request::ConfirmNew => {
                    if world.resource::<State>().confirmation {
                        if let Err(error) = new_run(world) {
                            fail(world, error);
                        }
                    }
                }
                Request::SaveQuit => {
                    if world.resource::<State>().active {
                        world.resource_mut::<ViewState>().pause();
                        let mut state = world.resource_mut::<State>();
                        state.force_save = true;
                        state.quit_after_save = true;
                    } else {
                        quit(world);
                    }
                }
            }
        }
    }
    let session = world.resource::<ArenaSession>();
    let tick = session.tick;
    let reward = reward_key(session);
    let state = world.resource::<State>();
    if state.active
        && !state.busy()
        && session.is_grand_run()
        && (state.force_save
            || tick.saturating_sub(state.last_saved_tick) >= AUTOSAVE_TICKS
            || reward != state.last_reward_key)
    {
        if let Err(error) = begin_save(world, reward) {
            fail(
                world,
                format!("Save failed: {error}. The previous complete save is intact."),
            );
        }
    }
}

fn settle(world: &mut World) -> Result<(), String> {
    world.insert_resource(Settling);
    world.run_schedule(ArenaTick);
    world.remove_resource::<Settling>();
    let terrain = world
        .resource_mut::<Messages<TerrainImpactOutcome>>()
        .drain()
        .collect::<Vec<_>>();
    let burrows = world
        .resource_mut::<Messages<ArenaBurrowOutcome>>()
        .drain()
        .collect::<Vec<_>>();
    world
        .resource_mut::<ArenaSession>()
        .settle_checkpoint_outcomes(&terrain, &burrows)
}

fn begin_save(world: &mut World, reward: String) -> Result<(), String> {
    settle(world)?;
    let state = world.resource::<State>();
    let store = store(state)?;
    let expected = state.token;
    let gameplay = world
        .resource::<ArenaSession>()
        .encode_grand_checkpoint(&gameplay_identity(state)?)?;
    let view = world.resource::<ViewState>();
    let app = AppCheckpoint {
        content_revision: state.content_revision.clone(),
        sites_fingerprint: world
            .resource::<ArenaTerrainView>()
            .package_identity
            .as_ref()
            .and_then(|identity| identity.sites_fingerprint)
            .ok_or("Grand sites identity is missing")?,
        tick: world.resource::<ArenaSession>().tick,
        environment_seconds: world.resource::<OceanSimulationTime>().seconds,
        yaw: view.yaw,
        pitch: view.pitch,
        third_person: view.third_person,
    };
    let app_bytes = ron::ser::to_string(&app)
        .map_err(|error| error.to_string())?
        .into_bytes();
    let map = hex_map::arena::checkpoint::snapshot_records(world)?;
    let records = [
        OwnerRecord {
            owner: "app".into(),
            key: "session".into(),
            format: APP_FORMAT.into(),
            bytes: app_bytes,
        },
        OwnerRecord {
            owner: "gameplay".into(),
            key: "session".into(),
            format: GAMEPLAY_FORMAT.into(),
            bytes: gameplay,
        },
    ];
    let worker = std::thread::Builder::new()
        .name("grand-save".into())
        .spawn(move || {
            store
                .commit(
                    expected,
                    map.into_records().chain(records.into_iter().map(Ok)),
                    &CancellationToken::default(),
                )
                .map(|snapshot| snapshot.token())
                .map_err(|error| error.to_string())
        })
        .map_err(|error| error.to_string())?;
    let tick = world.resource::<ArenaSession>().tick;
    let mut state = world.resource_mut::<State>();
    state.writing = Some(worker);
    state.last_saved_tick = tick;
    state.last_reward_key = reward;
    state.force_save = false;
    state.status = "Saving expedition…".into();
    Ok(())
}

fn finish_write(world: &mut World) {
    if !world
        .resource::<State>()
        .writing
        .as_ref()
        .is_some_and(JoinHandle::is_finished)
    {
        return;
    }
    let Some(worker) = world.resource_mut::<State>().writing.take() else {
        return;
    };
    match worker
        .join()
        .unwrap_or_else(|_| Err("Save worker stopped unexpectedly".into()))
    {
        Ok(token) => {
            let mut state = world.resource_mut::<State>();
            state.token = Some(token);
            state.available = true;
            state.status = "Expedition saved".into();
            if state.quit_after_save {
                quit(world);
            }
        }
        Err(error) => fail(
            world,
            format!("Save failed: {error}. The previous complete save is intact."),
        ),
    }
}

fn begin_restore(world: &mut World) -> Result<(), String> {
    let state = world.resource::<State>();
    let snapshot = store(state)?
        .load()
        .map_err(|error| error.to_string())?
        .ok_or("Resume slot no longer exists")?;
    snapshot
        .verify_all(&CancellationToken::default())
        .map_err(|error| error.to_string())?;
    let sites = world
        .resource::<ArenaTerrainView>()
        .package_identity
        .as_ref()
        .and_then(|identity| identity.sites_fingerprint)
        .ok_or("Grand sites identity is missing")?;
    let app = read_app(&snapshot, state, sites)?;
    let bytes = snapshot
        .record("gameplay", "session", GAMEPLAY_FORMAT)
        .map_err(|error| error.to_string())?
        .ok_or("Save is missing gameplay state")?;
    let session = ArenaSession::decode_grand_checkpoint(
        &bytes,
        &gameplay_identity(state)?,
        world.resource::<ArenaTerrainView>(),
        *world.resource::<ArenaVoxelGeometry>(),
        world.resource::<ArenaReset>().generation,
    )?;
    if session.tick != app.tick {
        return Err("Saved owners disagree on the simulation tick".into());
    }
    let position = ArenaSession::grand_checkpoint_position(&bytes, &gameplay_identity(state)?)?;
    let staged_map = hex_map::arena::checkpoint::stage_restore(world, &snapshot)?;
    world.resource_mut::<ViewState>().pause();
    hex_map::arena::checkpoint::commit_staged(world, staged_map, position)?;
    world.resource_mut::<ArenaStreamInterest>().position = position;
    let mut state = world.resource_mut::<State>();
    state.token = Some(snapshot.token());
    state.restoring = Some(Restoring {
        session,
        app,
        position,
        started: std::time::Instant::now(),
    });
    state.status = "Loading the saved location…".into();
    Ok(())
}

fn finish_restore(world: &mut World) {
    let Some(position) = world
        .resource::<State>()
        .restoring
        .as_ref()
        .map(|restore| restore.position)
    else {
        return;
    };
    world.resource_mut::<ArenaStreamInterest>().position = position;
    if world
        .resource::<State>()
        .restoring
        .as_ref()
        .is_some_and(|restore| restore.started.elapsed().as_secs() > 60)
    {
        world.resource_mut::<State>().restoring = None;
        fail(world, "Saved terrain did not become ready within 60 seconds. Play remains paused; your save is preserved.".into());
        return;
    }
    if !hex_map::arena::checkpoint::restore_ready(world, position) {
        return;
    }
    let Some(mut restore) = world.resource_mut::<State>().restoring.take() else {
        return;
    };
    let generation = world.resource::<ArenaReset>().generation;
    let result = restore
        .session
        .rebind_grand_terrain(
            world.resource::<ArenaTerrainView>(),
            *world.resource::<ArenaVoxelGeometry>(),
            generation,
        )
        .and_then(|()| {
            restore.session.validate_grand_burrow_poses(
                world.resource::<ArenaTerrainView>(),
                *world.resource::<ArenaVoxelGeometry>(),
                *world.resource::<hex_core::arena::ArenaMaterials>(),
                world.resource::<hex_core::arena::ArenaBurrowMaterials>(),
            )
        });
    if let Err(error) = result {
        if error == "Grand checkpoint destination terrain is not ready" {
            let mut state = world.resource_mut::<State>();
            state.status = "Loading saved encounters…".into();
            state.restoring = Some(restore);
        } else {
            fail(
                world,
                format!("Cannot restore: {error}. Play remains paused; your save is preserved."),
            );
        }
        return;
    }
    let key = reward_key(&restore.session);
    world.insert_resource(restore.session);
    world.insert_resource(OceanSimulationTime {
        generation,
        seconds: restore.app.environment_seconds,
    });
    world.resource_mut::<ArenaInput>().human = Default::default();
    {
        let mut view = world.resource_mut::<ViewState>();
        view.yaw = restore.app.yaw;
        view.pitch = restore.app.pitch;
        view.third_person = restore.app.third_person;
        view.initialized = true;
        view.begin_play();
    }
    let mut state = world.resource_mut::<State>();
    state.active = true;
    state.biome = None;
    state.biome_candidate = None;
    state.biome_notice.clear();
    state.last_saved_tick = restore.app.tick;
    state.last_reward_key = key;
    state.status = "Expedition resumed".into();
}

fn new_run(world: &mut World) -> Result<(), String> {
    // Preserve the old head and immutable bodies under the store's writer lock.
    let tuning = world
        .resource::<State>()
        .tuning
        .clone()
        .ok_or("Grand tuning unavailable")?;
    let expected = store(world.resource::<State>())?
        .archive_current_head()
        .map_err(|error| format!("Could not preserve the previous run: {error}"))?;
    let generation = world.resource::<ArenaReset>().generation.saturating_add(1);
    world.resource_mut::<ArenaReset>().generation = generation;
    world.resource_mut::<ViewState>().prepare_round();
    world.resource_mut::<ArenaInput>().human = Default::default();
    world.run_schedule(ArenaTick);
    world
        .resource_mut::<ArenaSession>()
        .configure_grand(tuning)?;
    world.resource_mut::<ViewState>().begin_play();
    let mut state = world.resource_mut::<State>();
    state.confirmation = false;
    state.token = expected;
    state.available = false;
    state.refused = false;
    state.active = true;
    state.biome = None;
    state.biome_candidate = None;
    state.biome_notice.clear();
    state.force_save = true;
    state.quit_after_save = false;
    state.last_saved_tick = 0;
    state.status = "New expedition started".into();
    Ok(())
}

fn quit(world: &mut World) {
    if let Some(mut recorder) = world.get_resource_mut::<super::recording::Recorder>() {
        recorder.request_quit();
    } else {
        world.write_message(AppExit::Success);
    }
}
