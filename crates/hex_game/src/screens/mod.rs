//! Application adapters for the screens rendered by `hex_ui`.

use bevy::prelude::*;
use hex_core::Screen;

pub(crate) mod combat_lab;
mod creator;
pub(crate) mod gameplay;
mod lattice_demo;
mod loading;
mod settings;
mod splash;
pub(crate) mod title;

pub use hex_ui::{despawn_screen, screen_root, screen_root_node};

pub(super) fn plugin(app: &mut App) {
    app.init_state::<Screen>();
    app.register_type::<Screen>();
    app.add_plugins((
        splash::plugin,
        title::plugin,
        settings::plugin,
        creator::plugin,
        combat_lab::plugin,
        lattice_demo::plugin,
        loading::plugin,
        gameplay::plugin,
    ));
}
