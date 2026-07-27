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

/// Which surfaces a piece may step between.
pub mod movement;
/// Hex-specific movement along a route of surfaces.
pub mod pathing;
/// Showing a piece where it can go before it goes there.
pub mod selection;
/// Who can be reached from where, and what height is worth.
pub mod targeting;
/// The units themselves: the player, enemies, and click-to-move.
pub mod units;

pub use movement::{route, Body, Footing, MovementCrossings, MovementSystems, Reach, Standing};
pub use pathing::HexPathingLine;
pub use selection::{
    HoveredSurface, PathOverlay, RangeOverlay, Selected, TerrainRevision, UnitRing,
};
pub use targeting::{either_in_reach, high_ground_bonus, in_reach, LEVELS_PER_BONUS_RANGE};
pub use units::{
    Enemy, Faction, MovingTo, Party, Player, StandsOn, StopMovingAt, UnitAllocator, UnitRegistry,
};

/// Adds every unit system.
///
/// `hex_anim` is added here rather than by the binary because this crate is its only
/// consumer: the animation engine is a dependency of movement, not a peer of it.
pub fn plugin(app: &mut App) {
    app.add_plugins((
        hex_anim::plugin,
        movement::plugin,
        units::plugin,
        selection::plugin,
    ));
}
