//! Shared vocabulary and pure domain logic for the game.
//!
//! This crate is the bottom of the dependency graph: everything else may depend
//! on it, and it depends on nothing in the workspace. It carries no rendering
//! and builds no [`App`](bevy_ecs), which is what keeps it fast to compile and
//! testable without a GPU.
//!
//! Types shared between `hex_world` (presentation) and `hex_units` (rules)
//! live here. Those two crates must not depend on each other, so this is where
//! their common language belongs.

pub mod app;
pub mod config;
pub mod hex;
pub mod setup;
pub mod terrain;
pub mod traversal;
pub mod voxel;

pub use app::{AppSystems, GameplaySetup, Mode, PausableSystems, Pause, Screen, Turn};
pub use hex::{HexCoord, HexGrid, HexSpan, HexTile};
pub use setup::GameplaySetupFailure;
pub use terrain::{
    CutawayOccluder, InteriorRegionId, InteriorRegions, MapAnchorId, MapAnchors, MapViewHint,
    ResolvedMapSeed, SpecialMovementRegion, SpecialMovementRegions, TerrainReady,
};
pub use traversal::{TraversalEndpoint, TraversalProfile};
pub use voxel::{Headroom, Level, SubstanceId, TerrainEdit, TilePos, MAX_HEADROOM};
