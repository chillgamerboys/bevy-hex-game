//! A generic RON asset loader, and the plumbing that turns a loaded settings
//! file into a Bevy [`Resource`].
//!
//! One loader serves every settings type. Adding a new one is a type, a `.ron`
//! file, and a single `load_settings` call — no per-type loader boilerplate,
//! which matters because the whole point of this pipeline is that adding
//! designer-facing config should be cheap.

use std::any::TypeId;
use std::marker::PhantomData;

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoadFailedEvent, AssetLoader, LoadContext};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use serde::Deserialize;
use thiserror::Error;

/// Everything that can go wrong loading a settings file.
#[derive(Debug, Error)]
pub enum RonLoaderError {
    /// The file could not be read.
    #[error("could not read settings file: {0}")]
    Io(#[from] std::io::Error),
    /// The file was read but is not valid RON, or does not match the type.
    ///
    /// Carries the line and column, because the people editing these files are
    /// not necessarily going to be reading a Rust backtrace.
    #[error("invalid RON: {0}")]
    Ron(#[from] ron::error::SpannedError),
}

/// Loads any `Deserialize` asset from a RON file.
#[derive(TypePath)]
pub struct RonAssetLoader<T: TypePath + Send + Sync> {
    extensions: &'static [&'static str],
    _marker: PhantomData<T>,
}

impl<T: TypePath + Send + Sync> RonAssetLoader<T> {
    /// Builds a loader claiming the given file extensions.
    pub fn new(extensions: &'static [&'static str]) -> Self {
        Self {
            extensions,
            _marker: PhantomData,
        }
    }
}

impl<T> AssetLoader for RonAssetLoader<T>
where
    T: Asset + TypePath + for<'de> Deserialize<'de>,
{
    type Asset = T;
    type Settings = ();
    type Error = RonLoaderError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        _load_context: &mut LoadContext<'_>,
    ) -> Result<T, RonLoaderError> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        Ok(ron::de::from_bytes(&bytes)?)
    }

    fn extensions(&self) -> &[&str] {
        self.extensions
    }
}

/// Holds the handle for a settings file while it loads.
///
/// `arrived` records that the **file** has been parsed, which is not the same question
/// as whether the resource exists. Counting the resource instead meant that anything
/// inserting a settings value before the loader saw the file left the count one short
/// forever — a loading screen that never advances, with nothing in the log to say why.
#[derive(Resource)]
struct SettingsHandle<T: Asset> {
    handle: Handle<T>,
    arrived: bool,
}

/// Tracks how many settings files have been registered and how many have arrived.
///
/// Exists so the loading screen can ask "is everything ready?" without naming a
/// single settings type. That matters for separation: `MapSettings` lives in
/// `hex_map`, and a loading screen that named it would drag a dependency on the map
/// into the binary's screen code — and break the moment the map's settings were
/// renamed.
#[derive(Resource, Debug, Default)]
pub struct SettingsRegistry {
    registered: usize,
    loaded: usize,
    /// Settings chosen at runtime that have not arrived yet, by type.
    ///
    /// A set rather than a counter, because two requests for the same type in one
    /// frame would take a counter to two, get decremented once, and leave the loading
    /// screen up forever. The name rides along purely so a stuck load can say *what*
    /// it is stuck on.
    pending: HashMap<TypeId, &'static str>,
}

impl SettingsRegistry {
    /// Whether every settings file — registered up front or chosen at runtime — has
    /// been parsed and inserted.
    #[must_use]
    pub fn all_loaded(&self) -> bool {
        self.loaded >= self.registered && self.pending.is_empty()
    }

    /// Records that a runtime-chosen file for `T` is on its way.
    ///
    /// Call this *before* anything can observe [`Self::all_loaded`] in the same frame,
    /// or the gate can pass while the previous choice is still installed.
    pub fn mark_pending<T: 'static>(&mut self) {
        self.pending
            .insert(TypeId::of::<T>(), std::any::type_name::<T>());
    }

    /// Records that the runtime-chosen file for `T` has arrived.
    pub fn clear_pending<T: 'static>(&mut self) {
        self.pending.remove(&TypeId::of::<T>());
    }

    /// What is still being waited on, for a diagnostic when a load will not finish.
    #[must_use]
    pub fn pending_names(&self) -> Vec<&'static str> {
        let mut names: Vec<&'static str> = self.pending.values().copied().collect();
        names.sort_unstable();
        names
    }

    /// Loading progress in the range 0.0 to 1.0, for a progress bar.
    #[must_use]
    #[expect(
        clippy::cast_precision_loss,
        reason = "counts of settings files, which number in the single digits"
    )]
    pub fn progress(&self) -> f32 {
        let outstanding = self.registered + self.pending.len();
        if outstanding == 0 {
            return 1.0;
        }
        self.loaded as f32 / outstanding as f32
    }
}

/// Registers a settings type as a RON asset, without loading anything.
pub trait RegisterSettings {
    /// Makes `T` loadable from RON. Idempotent, and safe to call from more than one
    /// plugin.
    ///
    /// **The guard is not a nicety.** `init_asset::<T>()` is not idempotent — it
    /// inserts a fresh `Assets<T>` over the live one and registers a second handle
    /// provider, so every handle minted before the second call points into storage
    /// that no longer exists. `Assets::get` then returns [`None`] forever and the
    /// loading screen hangs with an empty log.
    fn register_settings<T>(&mut self, extensions: &'static [&'static str]) -> &mut Self
    where
        T: Asset + Resource + Clone + for<'de> Deserialize<'de>;
}

impl RegisterSettings for App {
    fn register_settings<T>(&mut self, extensions: &'static [&'static str]) -> &mut Self
    where
        T: Asset + Resource + Clone + for<'de> Deserialize<'de>,
    {
        if self.world().contains_resource::<Assets<T>>() {
            return self;
        }
        self.init_asset::<T>();
        self.register_asset_loader(RonAssetLoader::<T>::new(extensions));
        self.init_resource::<SettingsRegistry>();
        self
    }
}

/// Registers settings types loaded from RON.
pub trait LoadSettings {
    /// Registers `T` as a RON asset, starts loading `path`, and inserts the
    /// deserialized value as a [`Resource`] once it arrives.
    ///
    /// The resource is deliberately absent until the file has loaded, so a system
    /// reading it before it is ready fails loudly rather than silently using a
    /// default that does not match what the designer wrote. The loading screen is
    /// what guarantees it is present before gameplay starts.
    fn load_settings<T>(
        &mut self,
        path: &'static str,
        extensions: &'static [&'static str],
    ) -> &mut Self
    where
        T: Asset + Resource + Clone + for<'de> Deserialize<'de>;
}

impl LoadSettings for App {
    fn load_settings<T>(
        &mut self,
        path: &'static str,
        extensions: &'static [&'static str],
    ) -> &mut Self
    where
        T: Asset + Resource + Clone + for<'de> Deserialize<'de>,
    {
        self.register_settings::<T>(extensions);
        self.world_mut()
            .resource_mut::<SettingsRegistry>()
            .registered += 1;

        self.add_systems(
            PreStartup,
            move |mut commands: Commands, asset_server: Res<AssetServer>| {
                commands.insert_resource(SettingsHandle {
                    handle: asset_server.load::<T>(path),
                    arrived: false,
                });
            },
        );

        // Runs until the asset lands, then inserts it and stops doing work. Also
        // re-inserts on change, which is what makes hot-reloading settings work
        // under the `dev` feature.
        self.add_systems(Update, insert_settings::<T>);

        self
    }
}

fn insert_settings<T: Asset + Resource + Clone>(
    mut commands: Commands,
    handle: Option<ResMut<SettingsHandle<T>>>,
    assets: Res<Assets<T>>,
    mut events: MessageReader<AssetEvent<T>>,
    mut registry: ResMut<SettingsRegistry>,
) {
    let Some(mut handle) = handle else { return };

    // Insert once when the file first arrives...
    //
    // Keyed on whether the *file* has landed, not on whether the resource is absent.
    // The latter reads the same until something else inserts the resource first, and
    // then `loaded` never reaches `registered` and the loading screen stays up with
    // nothing to explain it.
    if !handle.arrived {
        if let Some(value) = assets.get(&handle.handle) {
            commands.insert_resource(value.clone());
            handle.arrived = true;
            registry.loaded += 1;
        }
        return;
    }

    // ...and again whenever the file changes on disk.
    for event in events.read() {
        if event.is_modified(&handle.handle) {
            if let Some(value) = assets.get(&handle.handle) {
                commands.insert_resource(value.clone());
            }
        }
    }
}

/// A settings file chosen while the game is running, rather than at startup.
///
/// The handle is the only strong reference, so replacing this resource drops the
/// previous asset and a later choice of the same path re-reads it from disk. That is
/// correct, and it is also why "the file is already loaded" is never a safe assumption
/// here.
#[derive(Resource)]
pub struct SettingsChoice<T: Asset> {
    handle: Handle<T>,
    /// Only for diagnostics — an asset path is not recoverable from a handle.
    path: String,
    applied: bool,
}

/// Registers the machinery for choosing `T`'s file at runtime.
pub trait SelectSettings {
    /// Makes `T` selectable with [`choose_settings`].
    ///
    /// Deliberately does **not** count towards `registered`: nothing is being loaded
    /// yet, and a type that is only ever chosen at runtime would otherwise hold the
    /// loading screen up before anyone had chosen anything.
    fn select_settings<T>(&mut self, extensions: &'static [&'static str]) -> &mut Self
    where
        T: Asset + Resource + Clone + for<'de> Deserialize<'de>;
}

impl SelectSettings for App {
    fn select_settings<T>(&mut self, extensions: &'static [&'static str]) -> &mut Self
    where
        T: Asset + Resource + Clone + for<'de> Deserialize<'de>,
    {
        self.register_settings::<T>(extensions);
        self.add_systems(Update, apply_settings_choice::<T>);
        self
    }
}

/// Asks for `T` to be loaded from `path`, replacing whatever it was loaded from.
///
/// The resource does not change until the file arrives; until then
/// [`SettingsRegistry::all_loaded`] is false, which is what holds the loading screen.
pub fn choose_settings<T: Asset + Resource + Clone>(
    commands: &mut Commands,
    asset_server: &AssetServer,
    registry: &mut SettingsRegistry,
    path: &str,
) {
    // Marked immediately rather than through `Commands`, so nothing can observe
    // `all_loaded()` between the request and the wait.
    registry.mark_pending::<T>();
    commands.insert_resource(SettingsChoice::<T> {
        handle: asset_server.load::<T>(path.to_owned()),
        path: path.to_owned(),
        applied: false,
    });
}

/// Installs a chosen settings file once it has loaded, and on every later change.
fn apply_settings_choice<T: Asset + Resource + Clone>(
    mut commands: Commands,
    choice: Option<ResMut<SettingsChoice<T>>>,
    assets: Res<Assets<T>>,
    mut changes: MessageReader<AssetEvent<T>>,
    mut failures: MessageReader<AssetLoadFailedEvent<T>>,
    mut registry: ResMut<SettingsRegistry>,
) {
    // Drained unconditionally, before any early return. A `MessageReader` whose cursor
    // is left behind replays up to two frames of stale events later — against a handle
    // id that by then may belong to a different file.
    let modified: Vec<AssetId<T>> = changes
        .read()
        .filter_map(|event| match event {
            AssetEvent::Modified { id } => Some(*id),
            _ => None,
        })
        .collect();
    let failed: Vec<(AssetId<T>, String)> = failures
        .read()
        .map(|event| (event.id, event.error.to_string()))
        .collect();

    let Some(mut choice) = choice else { return };

    if !choice.applied {
        // Polled, never driven by `AssetEvent::Added`. Asking for a path whose asset is
        // already resident hands back the cached handle and emits **no event at all**,
        // so an event-driven version works the first time a scenario is chosen and
        // waits forever the second time.
        if let Some(value) = assets.get(&choice.handle) {
            commands.insert_resource(value.clone());
            choice.applied = true;
            registry.clear_pending::<T>();
            return;
        }
        if let Some((_, error)) = failed.iter().find(|(id, _)| *id == choice.handle.id()) {
            // Left pending on purpose. A settings file that will not parse should stop
            // the game on the loading screen with the reason in the log, which is the
            // behaviour the traps table in `CLAUDE.md` already describes.
            error!("{}: {error}", choice.path);
        }
        return;
    }

    if modified.iter().any(|id| *id == choice.handle.id()) {
        if let Some(value) = assets.get(&choice.handle) {
            commands.insert_resource(value.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::AssetPlugin;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::MinimalPlugins;

    /// A settings type that exists only here.
    #[derive(Asset, Resource, Reflect, Debug, Clone, Deserialize, PartialEq)]
    struct Probe(u32);

    /// A path that will never resolve.
    ///
    /// Every test here mints a real handle and then hands the asset over by calling
    /// `Assets::insert` directly. That keeps them synchronous and free of file IO —
    /// waiting on a real load would make them a race with the asset thread.
    const NOWHERE: &str = "config/does-not-exist.ron";

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()));
        app
    }

    /// Hands `Probe` to the asset system as though the file had just parsed.
    fn deliver(app: &mut App, handle: &Handle<Probe>, value: Probe) {
        app.world_mut()
            .resource_mut::<Assets<Probe>>()
            .insert(handle.id(), value)
            .expect("the handle should still be live");
    }

    fn choose(app: &mut App, path: &'static str) {
        app.world_mut()
            .run_system_once(
                move |mut commands: Commands,
                      asset_server: Res<AssetServer>,
                      mut registry: ResMut<SettingsRegistry>| {
                    choose_settings::<Probe>(&mut commands, &asset_server, &mut registry, path);
                },
            )
            .expect("the one-shot system should run");
    }

    fn chosen_handle(app: &mut App) -> Handle<Probe> {
        app.world()
            .resource::<SettingsChoice<Probe>>()
            .handle
            .clone()
    }

    /// A file chosen but not yet arrived holds the loading screen.
    #[test]
    fn a_pending_choice_is_not_loaded() {
        let mut app = test_app();
        app.select_settings::<Probe>(&["ron"]);

        assert!(
            app.world().resource::<SettingsRegistry>().all_loaded(),
            "nothing registered and nothing chosen is trivially ready"
        );

        choose(&mut app, NOWHERE);
        app.update();

        assert!(!app.world().resource::<SettingsRegistry>().all_loaded());
        assert_eq!(
            app.world()
                .resource::<SettingsRegistry>()
                .pending_names()
                .len(),
            1,
            "a stuck load has to be able to say what it is stuck on"
        );
        assert!(app.world().get_resource::<Probe>().is_none());
    }

    /// And stops holding it once the file lands.
    #[test]
    fn an_arrived_choice_becomes_the_resource() {
        let mut app = test_app();
        app.select_settings::<Probe>(&["ron"]);
        choose(&mut app, NOWHERE);
        app.update();

        let handle = chosen_handle(&mut app);
        deliver(&mut app, &handle, Probe(7));
        app.update();

        assert!(app.world().resource::<SettingsRegistry>().all_loaded());
        assert_eq!(app.world().get_resource::<Probe>(), Some(&Probe(7)));
    }

    /// Choosing again re-arms the wait rather than leaving the old value in place.
    ///
    /// The half that matters. An implementation that installs a file once and never
    /// re-chooses passes every first-time test and quietly plays the previous
    /// scenario's map forever after.
    #[test]
    fn choosing_a_second_file_waits_again() {
        let mut app = test_app();
        app.select_settings::<Probe>(&["ron"]);
        choose(&mut app, NOWHERE);
        app.update();
        let first = chosen_handle(&mut app);
        deliver(&mut app, &first, Probe(1));
        app.update();
        assert_eq!(app.world().get_resource::<Probe>(), Some(&Probe(1)));

        choose(&mut app, "config/somewhere-else.ron");
        app.update();
        assert!(
            !app.world().resource::<SettingsRegistry>().all_loaded(),
            "a fresh choice has to hold the gate until it arrives"
        );

        let second = chosen_handle(&mut app);
        deliver(&mut app, &second, Probe(2));
        app.update();
        assert_eq!(app.world().get_resource::<Probe>(), Some(&Probe(2)));
    }

    /// Editing the chosen file while the game runs replaces the resource.
    #[test]
    fn a_chosen_file_still_hot_reloads() {
        let mut app = test_app();
        app.select_settings::<Probe>(&["ron"]);
        choose(&mut app, NOWHERE);
        app.update();
        let handle = chosen_handle(&mut app);
        deliver(&mut app, &handle, Probe(1));
        app.update();

        // Re-inserting into an occupied slot is what the asset system does on a
        // reload, and it queues `Modified` rather than `Added`.
        deliver(&mut app, &handle, Probe(9));

        // **Two frames, and the second is not padding.** `Assets::asset_events` drains
        // the queue in `PostUpdate`, while this runs in `Update` — so a change made
        // during frame N is not readable until frame N+1. First arrival does not pay
        // that cost only because it polls `Assets::get` instead of waiting for an
        // event, which is the same reason choosing an already-loaded file works at all.
        app.update();
        app.update();

        assert_eq!(app.world().get_resource::<Probe>(), Some(&Probe(9)));
    }

    /// Registering the same type twice must not detach the handles already minted.
    ///
    /// `init_asset` is not idempotent: a second call inserts a fresh `Assets<T>` over
    /// the live one, and every existing handle then points at storage that is gone.
    /// `Assets::get` returns [`None`] for ever after and the loading screen hangs with
    /// nothing in the log — so the guard is load-bearing and this is its test.
    #[test]
    fn registering_twice_does_not_orphan_existing_handles() {
        let mut app = test_app();
        app.register_settings::<Probe>(&["ron"]);

        let handle = app.world().resource::<AssetServer>().load::<Probe>(NOWHERE);
        deliver(&mut app, &handle, Probe(3));

        app.register_settings::<Probe>(&["ron"]);

        assert_eq!(
            app.world().resource::<Assets<Probe>>().get(&handle),
            Some(&Probe(3)),
            "the second registration replaced the asset storage"
        );
    }

    /// A settings resource inserted before its file arrives must not stall the gate.
    ///
    /// Named for the symptom because that is all you get: `loaded` used to be counted
    /// only when the resource was *absent*, so anything inserting one early left the
    /// count permanently one short. The game sat on "loading…" with an empty log and
    /// nothing to grep for.
    #[test]
    fn an_early_resource_does_not_hang_the_loading_screen() {
        let mut app = test_app();
        app.load_settings::<Probe>("config/probe.ron", &["ron"]);
        app.finish();
        app.update();

        // Somebody else gets there first — a default, a scenario, a test harness.
        app.insert_resource(Probe(42));
        app.update();

        let handle = app
            .world()
            .get_resource::<SettingsHandle<Probe>>()
            .map(|held| held.handle.clone())
            .expect("PreStartup should have minted a handle");
        deliver(&mut app, &handle, Probe(1));
        app.update();

        assert!(
            app.world().resource::<SettingsRegistry>().all_loaded(),
            "the file arrived, so the gate must open however the resource got there"
        );
    }
}
