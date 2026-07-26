//! Asset handles and load tracking.
//!
//! Every asset the game needs is requested once, here, at startup, and handed out
//! as a [`GameAssets`] resource. Two reasons that matters beyond tidiness:
//!
//! - The loading screen needs a single place to ask "is everything ready yet?".
//!   Previously each system called `asset_server.load()` at spawn time, so the
//!   first frames rendered with meshes that had not arrived and nothing could
//!   tell how far along loading was.
//! - Asset paths stop being string literals scattered through gameplay and
//!   presentation code. When settings move into RON, this is the seam those paths
//!   move behind.

use bevy::asset::{LoadState, UntypedAssetId};
use bevy::gltf::GltfAssetLabel;
use bevy::prelude::*;

pub mod loader;
pub mod settings;
pub mod substances;

pub use loader::{LoadSettings, SettingsRegistry};
pub use settings::{
    to_color, CameraSettings, DisplaySettings, LightingSettings, PlayerSettings,
    PresentModeSetting, Rgb,
};
pub use substances::{Substance, SubstanceFile, SubstanceTable};

const HEX_MESH: &str = "meshes/hex.glb";
const PIECES_MESH: &str = "meshes/pieces.glb";
const SKYBOX: &str = "textures/sky_boxes/Ryfjallet_cubemap.png";

/// RON files carry a `.ron` extension, but are matched here by their full
/// double extension so an ordinary `.ron` elsewhere is not claimed by the wrong
/// loader.
pub const CONFIG_EXTENSIONS: &[&str] = &["ron"];

/// Registers asset loading and the settings shared across the game.
pub fn plugin(app: &mut App) {
    app.add_systems(PreStartup, load_assets);

    app.register_type::<CameraSettings>()
        .register_type::<LightingSettings>()
        .register_type::<PlayerSettings>()
        .register_type::<DisplaySettings>();

    app.add_plugins(substances::plugin);

    app.load_settings::<CameraSettings>("config/camera.ron", CONFIG_EXTENSIONS)
        .load_settings::<LightingSettings>("config/lighting.ron", CONFIG_EXTENSIONS)
        .load_settings::<PlayerSettings>("config/player.ron", CONFIG_EXTENSIONS)
        .load_settings::<DisplaySettings>("config/display.ron", CONFIG_EXTENSIONS);
}

/// Handles to everything the game loads from disk.
#[derive(Resource, Debug, Clone)]
pub struct GameAssets {
    /// The single tile mesh, instanced across the whole grid.
    pub hex_tile: Handle<Mesh>,
    /// The two primitives making up the player piece.
    pub player_pieces: [Handle<Mesh>; 2],
    /// Cubemap for the sky, stored as a vertically stacked 2D PNG.
    pub skybox: Handle<Image>,
}

impl GameAssets {
    /// Whether every asset has finished loading, including its dependencies.
    ///
    /// Returns `true` on failure as well as success: a missing asset is already
    /// reported as an error by the asset server, and blocking the loading screen
    /// forever on top of that turns a visible problem into a hang.
    pub fn is_ready(&self, asset_server: &AssetServer) -> bool {
        self.handle_ids()
            .into_iter()
            .all(|id| match asset_server.get_load_state(id) {
                Some(LoadState::Loaded) | Some(LoadState::Failed(_)) | None => true,
                Some(LoadState::NotLoaded) | Some(LoadState::Loading) => false,
            })
    }

    fn handle_ids(&self) -> [UntypedAssetId; 4] {
        [
            self.hex_tile.id().untyped(),
            self.player_pieces[0].id().untyped(),
            self.player_pieces[1].id().untyped(),
            self.skybox.id().untyped(),
        ]
    }
}

fn load_assets(mut commands: Commands, asset_server: Res<AssetServer>) {
    let primitive = |path: &str, mesh: usize| {
        asset_server
            .load(GltfAssetLabel::Primitive { mesh, primitive: 0 }.from_asset(path.to_owned()))
    };

    commands.insert_resource(GameAssets {
        hex_tile: primitive(HEX_MESH, 0),
        player_pieces: [primitive(PIECES_MESH, 0), primitive(PIECES_MESH, 1)],
        skybox: asset_server.load(SKYBOX),
    });
}
