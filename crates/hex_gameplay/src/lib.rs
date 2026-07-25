//! Gameplay: the player, input handling, picking, and movement.
//!
//! This crate must not depend on `hex_world`. Anything the two need to share
//! belongs in `hex_core`.

use bevy::prelude::*;

pub mod animation;
pub mod pathing;
pub mod player;

pub use player::Player;

/// Adds every gameplay system.
pub fn plugin(app: &mut App) {
    app.add_plugins((animation::plugin, player::plugin));
}
