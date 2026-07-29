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

pub mod art_palette;
pub mod content_index;
pub mod elements;
pub mod loader;
pub mod object_blueprint;
/// The scenarios offered on the title screen.
pub mod scenario;
pub mod settings;
pub mod spells;
pub mod substances;

pub use art_palette::{
    ArtContractError, ArtPalette, ObjectAssetId, PaletteSwatch, SrgbColor, SwatchId, SwatchMatch,
    VoxelEmission, VoxelStyle, VoxelStyleCatalog, VoxelStyleId, VoxelSurfaceMode,
    ART_SCHEMA_VERSION, DEFAULT_NEAR_COLOR_THRESHOLD,
};
pub use content_index::{ContentError, ContentIndex};
pub use elements::{ElementCatalog, ElementFile, FusionInput};
pub use loader::{
    choose_settings, LoadSettings, RegisterSettings, SelectSettings, SettingsRegistry,
};
pub use object_blueprint::{
    ConnectivityPolicy, EffectPart, LocalAxialCoord, LocalVoxelCoord, ObjectBlueprint,
    ObjectBounds, ObjectCategory, ObjectPart, ObjectPlacement, PlantPart, PropPart,
    MAX_OBJECT_HEIGHT, MAX_OBJECT_RADIUS, MAX_OBJECT_VOXELS, OBJECT_BLUEPRINT_SCHEMA_VERSION,
};
pub use scenario::{Scenario, ScenarioLibrary};
pub use settings::{
    to_color, ActionEconomy, CameraSettings, CelestialBody, CelestialCycleSettings,
    ChannellingTrickle, CombatSettings, CubeCoord, DisplaySettings, InitiativePolicy,
    LightingKeyframe, LightingProfile, LightingSettings, MenuSettings, PlayerSettings,
    PresentModeSetting, ResolvedLighting, Rgb, RoutPolicy, ScenarioPlacement, ScenarioSettings,
};
pub use spells::{
    CastingAxis, Effect, GemRequirement, ManaAxis, Spell, SpellBook, SpellFile, TargetShape,
    TargetingSpec,
};
pub use substances::{Substance, SubstanceFile, SubstanceTable, SubstanceTableError};

const HEX_MESH: &str = "meshes/hex.glb";
const PIECES_MESH: &str = "meshes/pieces.glb";

/// File extensions claimed by the generic settings loader.
pub const CONFIG_EXTENSIONS: &[&str] = &["ron"];

/// Registers asset loading and the settings shared across the game.
pub fn plugin(app: &mut App) {
    app.add_systems(PreStartup, load_assets);

    app.register_type::<CameraSettings>()
        .register_type::<CombatSettings>()
        .register_type::<LightingSettings>()
        .register_type::<LightingProfile>()
        .register_type::<CelestialCycleSettings>()
        .register_type::<LightingKeyframe>()
        .register_type::<CelestialBody>()
        .register_type::<PlayerSettings>()
        .register_type::<DisplaySettings>()
        .register_type::<MenuSettings>()
        .register_type::<ScenarioPlacement>()
        .register_type::<ScenarioSettings>()
        .register_type::<ScenarioLibrary>();

    app.add_plugins(substances::plugin);
    app.add_plugins(elements::plugin);
    app.add_plugins(spells::plugin);
    app.add_plugins(content_index::plugin);

    // Two types are deliberately **not** loaded from a fixed file here.
    //
    // `ScenarioSettings` is still the resource `spawn_units` reads, but its value now
    // comes from whichever scenario was chosen, so the library is what gets loaded and
    // the placements come out of it.
    //
    // `LightingSettings` is chosen the same way, by `hex_game::scenarios` — a scenario
    // names its own sky. Loading it here as well would run both `insert_settings` and
    // `apply_settings_choice` against one resource, and hold the loading screen open
    // for a file nobody asked for.
    app.load_settings::<CameraSettings>("config/camera.ron", CONFIG_EXTENSIONS)
        .load_settings::<CombatSettings>("config/combat.ron", CONFIG_EXTENSIONS)
        .load_settings::<PlayerSettings>("config/player.ron", CONFIG_EXTENSIONS)
        .load_settings::<DisplaySettings>("config/display.ron", CONFIG_EXTENSIONS)
        .load_settings::<MenuSettings>("config/menu.ron", CONFIG_EXTENSIONS)
        .load_settings::<ScenarioLibrary>("config/scenarios.ron", CONFIG_EXTENSIONS)
        .load_settings::<ArtPalette>("art/palette.ron", CONFIG_EXTENSIONS);
}

/// Handles to everything the game loads from disk.
#[derive(Resource, Debug, Clone)]
pub struct GameAssets {
    /// The single tile mesh, instanced across the whole grid.
    pub hex_tile: Handle<Mesh>,
    /// The two primitives making up the player piece.
    pub player_pieces: [Handle<Mesh>; 2],
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

    fn handle_ids(&self) -> [UntypedAssetId; 3] {
        [
            self.hex_tile.id().untyped(),
            self.player_pieces[0].id().untyped(),
            self.player_pieces[1].id().untyped(),
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
    });
}
