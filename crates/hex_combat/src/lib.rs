//! The gameplay loop: real time until something happens, then turns.
//!
//! The game plays like Baldur's Gate 3 — you walk around freely, and the moment a
//! hostile is close enough the world starts taking turns. There is **one map** and one
//! set of units either way. [`Mode`](hex_core::Mode) is the switch.
//!
//! # What is provisional
//!
//! Almost all of it. The design has not settled **initiative**, **action economy** or
//! **fight length**, and this crate needs an answer to all three to run at all. So it
//! picks the cheapest defensible one, says so, and puts the numbers in
//! `assets/config/combat.ron` where they are obviously knobs rather than decisions:
//!
//! - **Initiative** is a component with a fixed value, ordered high-to-low. The design
//!   proposes deriving it from lattice size, which would also solve boss action
//!   economy by giving a large lattice several slots in the order. Not built, because
//!   lattices do not exist.
//! - **A turn** is a movement budget and one action. The design's current preference
//!   is free movement of one or two hexes plus one action; this is that, with the
//!   budget exposed so it can be tried.
//! - **Nothing deals damage.** Damage disables lattice hexes, and there are no
//!   lattices, so an attack here is an animation and a log line. Building a stand-in
//!   damage model would bake in the numbers the design explicitly has not chosen.
//!
//! **No randomness**, which is not provisional — the design is explicit that
//! uncertainty comes from hidden information rather than dice. Ties in initiative
//! break by entity index, so the same units always produce the same order.

use bevy::prelude::*;

/// What an enemy does with its turn. A placeholder, and says so.
mod ai;
/// Whose turn it is, and what they have left.
pub mod turns;

pub use hex_core::Turn;
pub use turns::{Initiative, TurnOrder};

/// Adds the combat loop.
pub fn plugin(app: &mut App) {
    app.add_plugins((turns::plugin, ai::plugin));
}
