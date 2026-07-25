//! A generic RON asset loader, and the plumbing that turns a loaded settings
//! file into a Bevy [`Resource`].
//!
//! One loader serves every settings type. Adding a new one is a type, a `.ron`
//! file, and a single `load_settings` call — no per-type loader boilerplate,
//! which matters because the whole point of this pipeline is that adding
//! designer-facing config should be cheap.

use std::marker::PhantomData;

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext};
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
#[derive(Resource)]
struct SettingsHandle<T: Asset>(Handle<T>);

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
        self.init_asset::<T>();
        self.register_asset_loader(RonAssetLoader::<T>::new(extensions));

        self.add_systems(
            PreStartup,
            move |mut commands: Commands, asset_server: Res<AssetServer>| {
                commands.insert_resource(SettingsHandle(asset_server.load::<T>(path)));
            },
        );

        // Runs until the asset lands, then inserts it and stops doing work. Also
        // re-inserts on change, which is what makes hot-reloading settings work
        // under the `dev_native` feature.
        self.add_systems(Update, insert_settings::<T>);

        self
    }
}

fn insert_settings<T: Asset + Resource + Clone>(
    mut commands: Commands,
    handle: Option<Res<SettingsHandle<T>>>,
    assets: Res<Assets<T>>,
    mut events: MessageReader<AssetEvent<T>>,
    current: Option<Res<T>>,
) {
    let Some(handle) = handle else { return };

    // Insert once when it first arrives...
    if current.is_none() {
        if let Some(value) = assets.get(&handle.0) {
            commands.insert_resource(value.clone());
        }
        return;
    }

    // ...and again whenever the file changes on disk.
    for event in events.read() {
        if event.is_modified(&handle.0) {
            if let Some(value) = assets.get(&handle.0) {
                commands.insert_resource(value.clone());
            }
        }
    }
}
