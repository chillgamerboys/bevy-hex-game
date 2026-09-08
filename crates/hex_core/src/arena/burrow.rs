//! Bounded arena-only burrow requests and source-correlated world admission.

use std::collections::BTreeSet;

use bevy_ecs::prelude::{Message, Resource};

use crate::{SubstanceId, TerrainVoxelHealth, TilePos};

/// Maximum announced swept cells in one atomic burrow proposal.
pub const MAX_ARENA_BURROW_CELLS: usize = 64;

/// World-owned material admission. Destination dirt comes from `ArenaMaterials`.
#[derive(Debug, Clone, Default, Resource)]
pub struct ArenaBurrowMaterials {
    /// Solid, diggable substances with finite world-owned toughness.
    pub eligible: BTreeSet<SubstanceId>,
}

/// Bounded proposed conversion of the cells touched by one continuous body step.
#[derive(Debug, Clone, Message)]
pub struct ArenaBurrowRequest {
    /// Session generation; old proposals cannot mutate a reset map.
    pub generation: u64,
    /// Stable source identity within that generation.
    pub actor: u8,
    /// Strictly increasing source sequence, including rejected proposals.
    pub sequence: u64,
    /// Exact canonical swept cells, including air; no duplicates or empty volume.
    pub volume: Vec<TilePos>,
}

impl ArenaBurrowRequest {
    /// First envelope failure, before live generation, material or occupancy checks.
    #[must_use]
    pub fn structural_rejection(&self) -> Option<ArenaBurrowRejection> {
        if self.volume.is_empty() {
            Some(ArenaBurrowRejection::EmptyVolume)
        } else if self.volume.len() > MAX_ARENA_BURROW_CELLS {
            Some(ArenaBurrowRejection::TooManyCells)
        } else if !self
            .volume
            .iter()
            .zip(self.volume.iter().skip(1))
            .all(|(a, b)| a < b)
        {
            Some(ArenaBurrowRejection::NonCanonicalVolume)
        } else {
            None
        }
    }
}

/// One material conversion, retaining remaining HP without repairing damaged earth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArenaBurrowChange {
    /// Exact changed world cell.
    pub position: TilePos,
    /// Previous substance; destination is always the published dirt.
    pub before: SubstanceId,
    /// Current world ledger health immediately before this conversion.
    pub health_before: TerrainVoxelHealth,
    /// Health after capping remaining HP at the dirt maximum.
    pub health_after: TerrainVoxelHealth,
}

/// Atomic admission result; air and unchanged dirt produce no change records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArenaBurrowResult {
    /// Entire requested volume admitted; actual material changes are canonical.
    Accepted {
        /// Actual material changes; unchanged dirt and air have no records.
        changed: Vec<ArenaBurrowChange>,
    },
    /// No requested cell was changed by this proposal.
    Rejected {
        /// First canonical offending cell, or none for an invalid request envelope.
        position: Option<TilePos>,
        /// Exact admission failure.
        reason: ArenaBurrowRejection,
    },
}

/// Source-correlated outcome published before gameplay can advance the proposal.
#[derive(Debug, Clone, Message)]
pub struct ArenaBurrowOutcome {
    /// Generation copied from the request.
    pub generation: u64,
    /// Actor copied from the request.
    pub actor: u8,
    /// Proposal sequence copied from the request.
    pub sequence: u64,
    /// Atomic world result.
    pub result: ArenaBurrowResult,
}

/// Narrow burrow failures, independent of ordinary elemental or physical impacts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArenaBurrowRejection {
    /// No cells were announced.
    EmptyVolume,
    /// More than the bounded per-proposal capacity was announced.
    TooManyCells,
    /// Cell identities are unsorted or repeated.
    NonCanonicalVolume,
    /// The world has already accepted another reset generation.
    StaleGeneration,
    /// A sequence was already consumed, or precedes the highest consumed value.
    ReusedSequence,
    /// A cell is outside the current horizontal or vertical world bounds.
    OutsideWorld,
    /// A current edit-protected interval includes this cell, including protected air.
    Protected,
    /// Non-solid liquid occupies this cell.
    Liquid,
    /// An authored static object occupies this cell regardless of ordinary query masks.
    StaticObject,
    /// A solid cell is not admitted by the published burrow material policy.
    IneligibleMaterial,
    /// The world content cannot supply valid dirt conversion and HP semantics.
    InvalidDestination,
}
