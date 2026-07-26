//! Gameplay: the player, input handling, picking, and movement.
//!
//! This crate must not depend on `hex_world`. Anything the two need to share
//! belongs in `hex_core`.

use bevy::prelude::*;

/// Which columns a piece may step between.
pub mod movement;
/// Hex-specific movement along a route of columns.
pub mod pathing;
/// The player piece and click-to-move.
pub mod player;

pub use movement::{route, Body, Footing, Standing, MAX_STEP};
pub use player::Player;

/// Adds every gameplay system.
///
/// `hex_anim` is added here rather than by the binary because this crate is its only
/// consumer: the animation engine is a dependency of movement, not a peer of it.
pub fn plugin(app: &mut App) {
    app.add_plugins((hex_anim::plugin, player::plugin));
}
