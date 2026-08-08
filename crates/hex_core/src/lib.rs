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
pub mod authored_occupancy;
pub mod commands;
pub mod config;
#[cfg(feature = "test-support")]
pub mod deterministic_fixture;
pub mod effects;
pub mod elements;
pub mod faction;
pub mod formation;
pub mod hex;
pub mod input;
pub mod lattice_ids;
pub mod occupancy;
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
    AppSystems, GameplayPhase, GameplaySetup, GameplaySystems, Mode, PausableSystems, Pause,
    RoundElapsed, Screen, Turn,
};
pub use authored_occupancy::{AuthoredObjectVoxelRun, AuthoredObjectVoxelRuns};
pub use commands::{Busy, CommandQueue, GameCommand, IssuedCommand, PendingDecision};
#[cfg(feature = "test-support")]
pub use deterministic_fixture::{
    deterministic_fixture, DeterministicFixtureDefinition, DeterministicFixtureInitialState,
    DeterministicRosterEntry, DeterministicRosterPlacement, DETERMINISTIC_FIXTURES,
};
pub use effects::{EffectEnd, EffectId, EffectPayload, PersistentEffect};
pub use elements::{ElementId, SpellId};
pub use faction::Faction;
pub use formation::{
    FormationError, FormationPreset, FormationSlot, PartyFormation, PartyMovementMode, PartyPath,
    MAX_FORMATION_SLOTS, MIN_FORMATION_SLOTS,
};
pub use hex::{HexCoord, HexGrid, HexSpan, HexTile, Sextant};
pub use input::{
    BindingConflict, BindingEditError, BindingRestoreOutcome, InputAction, InputActionInventory,
    InputActionMetadata, InputBindingOverrides, InputBindings, InputCategory, InputContext,
    KeyChord, KeyModifiers,
};
pub use lattice_ids::{EnchantId, LatticeCoord};
pub use occupancy::{OccupancyBlock, UnitOccupancy};
pub use perception::{
    upper_dome_contains, ExactGridPoint, ExteriorIllumination, GameplayLight, IlluminationLevel,
    KnowledgeExpiry, KnowledgeSource, KnowledgeState, KnownTraversal, LightDomain,
    LocalMapKnowledge, PerceptionSystems, SightBand, SightProfile,
};
pub use presentation::{
    CanopyOccluder, PresentationOcclusion, PresentationOcclusionReason, PresentationSystems,
    TargetReticleRequest, TreeFadeAmount, TreeOccluder, WorldMarkerSuppression,
};
pub use setup::GameplaySetupFailure;
pub use spatial::{BiomeRegionId, BiomeRegions, TraversalBlockers};
pub use terrain::{
    CutawayOccluder, InteriorRegionId, InteriorRegions, MapAnchorId, MapAnchors, MapViewHint,
    ResolvedMapSeed, SpecialMovementRegion, SpecialMovementRegions, TerrainReady,
};
pub use terrain_impact::{
    is_terrain_toughness, DamagedVoxels, TerrainBatchId, TerrainImpact, TerrainImpactDisposition,
    TerrainImpactOutcome, TerrainImpactRejection, TerrainImpactResult, TerrainSystems,
    TerrainVoxelHealth, TerrainVoxelOutcome, MAX_TERRAIN_TOUGHNESS,
};
pub use traversal::{TraversalEndpoint, TraversalProfile};
pub use unit_ids::{ControlOwner, PlayerSeat, SimSeeds, UnitId};
pub use view::{
    CameraFocusTarget, CenterInspectionCamera, InspectionCameraSubject, ZoomSensitivityOverride,
};
pub use voxel::{Headroom, Level, RunBottom, SubstanceId, TerrainEdit, TilePos, MAX_HEADROOM};
