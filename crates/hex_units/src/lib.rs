//! Units: the things that stand on the map, and the rules for where they may stand.
//!
//! A unit is a body at a position. That is all this crate claims — it deliberately
//! says nothing about turns, initiative or combat, which live a layer up in
//! `hex_combat`, and nothing about how a unit is *drawn* moving, which lives a layer
//! down in `hex_anim`.
//!
//! This crate must not depend on `hex_world` or `hex_map`. Anything they need to
//! share belongs in `hex_core`; Cargo is what enforces it.
//!
//! # What a unit is not, yet
//!
//! The design gives every character and enemy a **lattice** — a hex grid of gems,
//! fusions and spells that is simultaneously its stat block, its ability list and its
//! health. None of that exists here. A unit is currently a [`Body`] and a position,
//! and anything written against it should expect to gain a lattice rather than assume
//! a unit is only a body.
//!
//! (The design notes call that grid a "core". It is called a **lattice** in this
//! codebase, because `hex_core` is already a crate and the collision would be
//! permanent. See `docs/systems/combat.md`.)

use bevy::prelude::*;

/// Exact occupancy projected from opt-in authored non-terrain objects.
pub mod authored_object_occupancy;
/// Planning exact, rotating, compressed whole-party routes.
pub mod formation;
/// Which surfaces a piece may step between.
pub mod movement;
/// Compatibility path for shared exact-surface occupancy vocabulary.
pub mod occupancy {
    pub use hex_core::{OccupancyBlock, UnitOccupancy};
}
/// Hex-specific movement along a route of surfaces.
pub mod pathing;
/// Showing a piece where it can go before it goes there.
pub mod selection;
/// Who can be reached from where, and what height is worth.
pub mod targeting;
/// Spell-created terrain placement and legality.
pub mod terrain_creation;
/// Exact material occupancy derived from published integer run bounds.
pub mod terrain_occupancy;
/// Deterministic landing choices after terrain withdraws a unit's support.
pub mod terrain_reconciliation;
/// Exact material obstruction along spell trajectories.
pub mod trajectories;
/// The units themselves: the player, enemies, and click-to-move.
pub mod units;
/// Turning a spell's shape into the exact voxels it reaches.
pub mod volumes;

pub use authored_object_occupancy::{
    AuthoredObjectOccupancy, AuthoredObjectOccupancySystems, InvalidAuthoredObjectRun,
};
pub use formation::{
    plan_formation_move, plan_formation_move_with_occupancy, rotated, FormationMember,
    FormationPlan, FormationPlanError,
};
pub use hex_core::{Faction, OccupancyBlock, UnitOccupancy};
pub use movement::{
    route, route_with_occupancy, Body, Footing, MovementCrossings, MovementSystems, Reach, Standing,
};
pub use pathing::HexPathingLine;
pub use selection::{
    HoveredSurface, PathOverlay, RangeOverlay, Selected, TargetReticle, TerrainRevision, UnitRing,
};
pub use targeting::{either_in_reach, high_ground_bonus, in_reach};
pub use terrain_creation::{
    resolve_creation_volume, validate_creation_volume, CreationBody, TerrainCreationBlock,
};
pub use terrain_occupancy::{
    InvalidTerrainRun, KnownTerrainOccupancy, TerrainOccupancy, TerrainOccupancySystems,
};
pub use terrain_reconciliation::{plan_unsupported_actor_landing, NoLanding};
pub use trajectories::{
    authored_object_sight_segment_is_clear, known_trajectory_is_clear, sight_segment_is_clear,
    supercover, terrain_and_authored_object_sight_is_clear, terrain_sight_is_clear,
    trajectory_destination, trajectory_is_clear, trajectory_voxels,
};
pub use units::{
    Archetype, Downed, Enemy, MovingTo, Party, Player, StandsOn, StopMovingAt, UnitAllocator,
    UnitRegistry,
};
// `volumes` is deliberately not re-exported here. Its names are bare verbs —
// `line`, `column`, `path`, `resolve` — that only read correctly qualified, and
// flattening them into the crate root would put them one collision away from
// anything else this list ever grows.

/// Adds every unit system.
///
/// `hex_anim` is added here rather than by the binary because this crate is its only
/// consumer: the animation engine is a dependency of movement, not a peer of it.
pub fn plugin(app: &mut App) {
    app.add_plugins((
        hex_anim::plugin,
        terrain_occupancy::plugin,
        authored_object_occupancy::plugin,
        movement::plugin,
        units::plugin,
        selection::plugin,
    ));
}
