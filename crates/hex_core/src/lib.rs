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
pub mod commands;
pub mod config;
pub mod effects;
pub mod elements;
pub mod formation;
pub mod hex;
pub mod input;
pub mod lattice_ids;
pub mod perception;
pub mod presentation;
pub mod setup;
pub mod spatial;
pub mod terrain;
pub mod terrain_impact;
pub mod traversal;
pub mod unit_ids;
pub mod view;
pub mod voxel;

pub use app::{
    AppSystems, GameplaySetup, GameplaySystems, Mode, PausableSystems, Pause, RoundElapsed, Screen,
    Turn,
};
pub use commands::{Busy, CommandQueue, GameCommand, IssuedCommand, PendingDecision};
pub use effects::{EffectEnd, EffectId, EffectPayload, PersistentEffect};
pub use elements::{ElementId, SpellId};
pub use formation::{
    FormationError, FormationPreset, FormationSlot, PartyFormation, PartyMovementMode, PartyPath,
    MAX_FORMATION_SLOTS, MIN_FORMATION_SLOTS,
};
pub use hex::{HexCoord, HexGrid, HexSpan, HexTile, Sextant};
pub use input::{InputAction, InputBindings};
pub use lattice_ids::{EnchantId, LatticeCoord};
pub use perception::{
    ExteriorIllumination, GameplayLight, IlluminationLevel, KnowledgeExpiry, KnowledgeSource,
    KnowledgeState, KnownTraversal, LightDomain, LocalMapKnowledge, PerceptionSystems, SightBand,
    SightProfile,
};
pub use presentation::{CanopyOccluder, PresentationOcclusion, PresentationOcclusionReason};
pub use setup::GameplaySetupFailure;
pub use spatial::{BiomeRegionId, BiomeRegions, TraversalBlockers};
pub use terrain::{
    CutawayOccluder, InteriorRegionId, InteriorRegions, MapAnchorId, MapAnchors, MapViewHint,
    ResolvedMapSeed, SpecialMovementRegion, SpecialMovementRegions, TerrainReady,
};
pub use terrain_impact::{
    TerrainBatchId, TerrainImpact, TerrainImpactDisposition, TerrainImpactOutcome,
    TerrainVoxelOutcome,
};
pub use traversal::{TraversalEndpoint, TraversalProfile};
pub use unit_ids::{ControlOwner, PlayerSeat, SimSeeds, UnitId};
pub use view::CameraFocusTarget;
pub use voxel::{Headroom, Level, SubstanceId, TerrainEdit, TilePos, MAX_HEADROOM};
