//! Application adapters for the screens rendered by `hex_ui`.

use bevy::prelude::*;
use hex_core::Screen;

mod creator;
pub(crate) use creator::CreatorSandboxReturn;
pub(crate) mod gameplay;
mod lattice_demo;
mod loading;
mod main_menu;
pub(crate) mod sandbox;
mod settings;
mod splash;

pub use hex_ui::despawn_screen;

pub(super) fn plugin(app: &mut App) {
    app.init_state::<Screen>();
    app.register_type::<Screen>();
    app.add_plugins((
        hex_multiplayer::MultiplayerPlugin,
        crate::multiplayer_gameplay::plugin,
    ));
    app.add_plugins((
        splash::plugin,
        main_menu::plugin,
        settings::plugin,
        creator::plugin,
        sandbox::plugin,
        lattice_demo::plugin,
        loading::plugin,
        gameplay::plugin,
    ));
}
