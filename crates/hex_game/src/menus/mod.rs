//! Menus drawn over the top of a screen, rather than replacing it.
//!
//! Distinct from [`crate::screens`]: a screen owns the whole frame and controls
//! what state the game is in, whereas a menu overlays one. The pause menu leaves
//! the world rendered behind it.
//!
//! Scaffolding. Layout and transitions are real; the option lists wait for the
//! design doc.

use bevy::prelude::*;

pub(crate) mod lattice_view;
mod pause;

pub(super) fn plugin(app: &mut App) {
    app.add_plugins(pause::plugin);
    app.add_systems(Update, lattice_view::paint_interactions);
}
