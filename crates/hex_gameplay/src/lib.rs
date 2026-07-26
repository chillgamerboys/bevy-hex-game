//! Gameplay: the player, input handling, picking, and movement.
//!
//! This crate must not depend on `hex_world`. Anything the two need to share
//! belongs in `hex_core`.

use bevy::prelude::*;

/// Generic transform animation, independent of hexes.
pub mod animation;
/// Which columns a piece may step between.
pub mod movement;
/// Hex-specific movement along a route of columns.
pub mod pathing;
/// The player piece and click-to-move.
pub mod player;

pub use movement::{route, Footing, Standing, MAX_STEP};
pub use player::Player;

/// Adds every gameplay system.
pub fn plugin(app: &mut App) {
    app.add_plugins((animation::plugin, player::plugin));
}
