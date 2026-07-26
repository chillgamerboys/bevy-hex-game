//! Designer-facing map settings, loaded from `assets/config/world.ron`.
//!
//! These live here rather than in `hex_assets` so that the map's settings, its
//! generation, and its rendering are all in the crate the map is owned in. Only the
//! *loader* is shared — `hex_assets::LoadSettings` handles RON parsing and
//! hot-reload for every settings type in the game.
//!
//! Adding a new map setting is a field here plus a line in the RON file. Nothing
//! outside this crate needs to know.

use bevy::prelude::*;
use hex_assets::{LoadSettings, CONFIG_EXTENSIONS};
use serde::Deserialize;

/// Registers map settings for loading.
pub fn plugin(app: &mut App) {
    app.register_type::<MapSettings>();
    app.load_settings::<MapSettings>("config/world.ron", CONFIG_EXTENSIONS);
}

/// `assets/config/world.ron` — grid shape and terrain generation.
#[derive(Asset, Resource, Reflect, Debug, Clone, Deserialize)]
#[reflect(Resource)]
pub struct MapSettings {
    /// Tiles from the centre to the edge. The grid holds `3r² + 3r + 1` tiles, so
    /// this grows quadratically: 20 is 1261 tiles, 40 would be 4921.
    pub grid_radius: u32,
    /// World height of one voxel level.
    ///
    /// The tile mesh is exactly one unit tall, so a run of *n* levels renders at
    /// `scale.y = n * level_height` and columns stack seamlessly at any value.
    ///
    /// Lower values give flatter, more terraced terrain; raising it toward 1.0 gives
    /// chunkier cells that read better once they are being dug into.
    pub level_height: f32,
    /// Terrain generation.
    pub terrain: TerrainSettings,
}

/// Perlin terrain configuration.
#[derive(Reflect, Debug, Clone, Deserialize)]
pub struct TerrainSettings {
    /// Fixed seed for reproducible worlds, or `None` to randomise per launch.
    pub seed: Option<u64>,
    /// Octaves of noise, summed. More steps with higher frequencies and smaller
    /// magnitudes give finer detail on top of the broad shape.
    pub steps: Vec<PerlinStepSettings>,
}

/// One octave of Perlin noise.
#[derive(Reflect, Debug, Clone, Deserialize)]
pub struct PerlinStepSettings {
    /// Noise frequency along x. Higher is bumpier.
    pub x_freq: f32,
    /// Noise frequency along y. Higher is bumpier.
    pub y_freq: f32,
    /// How much height this octave contributes.
    pub magnitude: f32,
}
