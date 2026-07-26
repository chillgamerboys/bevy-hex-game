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
//! permanent. See `docs/GAMEPLAY_LOOP.md`.)

use bevy::prelude::*;

/// Which columns a piece may step between.
pub mod movement;
/// Hex-specific movement along a route of columns.
pub mod pathing;
/// The units themselves: the player, enemies, and click-to-move.
pub mod units;

pub use movement::{route, Body, Footing, Standing, MAX_STEP};
pub use pathing::HexPathingLine;
pub use units::{Enemy, Faction, Player, StandsOn};

/// Adds every unit system.
///
/// `hex_anim` is added here rather than by the binary because this crate is its only
/// consumer: the animation engine is a dependency of movement, not a peer of it.
pub fn plugin(app: &mut App) {
    app.add_plugins((hex_anim::plugin, movement::plugin, units::plugin));
}
