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
/// What stands on the map when a scenario starts.
pub mod encounter;
/// Who each of them is: archetype lattices, resolved from content.
pub mod lattices;
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
pub use content_index::{ContentError, ContentIndex, ContentTables};
pub use elements::{ElementCatalog, ElementFile, FusionInput};
pub use encounter::{
    Encounter, EncounterFaction, EncounterPlacement, FormationCenter, Roster, RosterEntry,
    RosteredUnit,
};
pub use lattices::{
    Archetype, AxialPair, LatticeError, LatticeFile, LatticeLibrary, UnvalidatedArchetype,
    UnvalidatedCell, UnvalidatedEntry,
};
pub use loader::{
    choose_settings, LoadSettings, RegisterSettings, SelectSettings, SettingsRegistry,
};
pub use object_blueprint::{
    ConnectivityPolicy, EffectPart, LocalAxialCoord, LocalVoxelCoord, ObjectBlueprint,
    ObjectBounds, ObjectCategory, ObjectPart, ObjectPlacement, PlantPart, PropPart,
    MAX_OBJECT_HEIGHT, MAX_OBJECT_RADIUS, MAX_OBJECT_VOXELS, OBJECT_BLUEPRINT_SCHEMA_VERSION,
};
pub use scenario::{Scenario, ScenarioCategory, ScenarioLibrary};
pub use settings::{
    to_color, ActionEconomy, CameraSettings, CelestialBody, CelestialCycleSettings,
    ChannellingTrickle, CombatSettings, CubeCoord, DisplaySettings, InitiativePolicy,
    LightingKeyframe, LightingProfile, LightingSettings, MenuSettings, PlayerSettings,
    PresentModeSetting, ResolvedLighting, Rgb, RoutPolicy,
};
pub use spells::{
    CastingAxis, Effect, GemRequirement, ManaAxis, Spell, SpellBook, SpellFile, TargetShape,
    TargetingSpec, VoxelOffset,
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
        .register_type::<Encounter>()
        .register_type::<EncounterPlacement>()
        .register_type::<ScenarioLibrary>();

    app.add_plugins(substances::plugin);
    app.add_plugins(elements::plugin);
    app.add_plugins(spells::plugin);
    app.add_plugins(content_index::plugin);
    app.add_plugins(lattices::plugin);

    // Two types are deliberately **not** loaded from a fixed file here.
    //
    // `Encounter` is the resource `spawn_units` reads, but which file it comes from is
    // whichever the chosen scenario named, so the library is what gets loaded and the
    // encounter is selected out of it.
    //
    // `LightingSettings` is chosen the same way, by `hex_game::scenarios` — a scenario
    // names its own sky. Loading either here as well would run both `insert_settings`
    // and `apply_settings_choice` against one resource, and hold the loading screen open
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
