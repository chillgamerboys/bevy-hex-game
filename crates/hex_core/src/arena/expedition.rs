//! Passive world facts for authored arena expeditions.
//!
//! Names are stable content identities, never actor ids or gameplay profiles.
//! World publishes a complete validated snapshot with the terrain revision.
//! Gameplay still validates each physical actor pose against current collision;
//! a route or supporting surface is not a movement or spawn authorization.

use std::collections::{BTreeMap, BTreeSet};

use crate::TilePos;

use super::ArenaDeploymentRegion;

/// Finite authored geometry for encounters, travel and consumable pool locations.
///
/// The producer rejects dangling references, empty sites/volumes, unsupported
/// surfaces, invalid route endpoints and non-traversable route ribbons before
/// publishing. Consumers must not reconstruct these facts from visual meshes.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ArenaExpeditionSites {
    /// Stable named encounter areas. Species, counts and difficulty are gameplay facts.
    pub encounters: BTreeMap<String, ArenaEncounterSite>,
    /// Named junctions and encounter entrances on exact supporting voxels.
    pub route_nodes: BTreeMap<String, TilePos>,
    /// Bidirectional authored route edges joining the named nodes.
    pub routes: BTreeMap<String, ArenaExpeditionRoute>,
    /// Named liquid volumes; healing amount and consumption belong to gameplay.
    pub fountains: BTreeMap<String, ArenaFountainVolume>,
}

/// Authored candidate surfaces for one complete encounter party.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArenaEncounterSite {
    /// Preferred support and finite allowed spawn surfaces. The gameplay owner
    /// admits a complete non-overlapping roster or rejects the encounter setup.
    pub deployment: ArenaDeploymentRegion,
    /// Optional route node used to enter the authored forest travel network.
    /// Does not grant knowledge of the player or order an actor to move.
    pub rally_entry: Option<String>,
}

/// One bidirectional path with world-validated grade and clearance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArenaExpeditionRoute {
    /// Stable start junction key in `route_nodes`.
    pub from: String,
    /// Stable end junction key in `route_nodes`.
    pub to: String,
    /// Positive clear height above every supporting voxel, in world voxel levels.
    /// Both columns of each step also retain this aperture above the higher support.
    /// This is a geometry guarantee; gameplay still validates each actor's body.
    pub clearance_levels: u32,
    /// Ordered supporting voxels, including both endpoint nodes. Adjacent entries
    /// are adjoining steps; reverse this sequence for the opposite direction.
    pub supports: Vec<TilePos>,
    /// Complete finite walkable ribbon, including the centerline. Validation uses
    /// the entire width, so a valid centerline cannot hide blocked shoulders.
    pub ribbon: BTreeSet<TilePos>,
}

/// Exact non-solid water cells occupied by one authored fountain pool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArenaFountainVolume {
    /// Liquid voxel identities, not supporting floors or visual light geometry.
    /// Gameplay tests physical body overlap and owns the one-use healing state.
    pub cells: BTreeSet<TilePos>,
}
