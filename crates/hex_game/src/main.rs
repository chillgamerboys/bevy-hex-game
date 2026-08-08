//! Thin platform launcher for Hex Game.

#![cfg_attr(bevy_lint, feature(register_tool), register_tool(bevy))]
#![cfg_attr(
    not(any(feature = "dev", feature = "map-review")),
    windows_subsystem = "windows"
)]

use bevy::prelude::AppExit;

fn main() -> AppExit {
    // Chain rather than replace: console builds keep the default stderr report,
    // and the windowed Windows release gets the panic into the log file.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        bevy::log::error!("panic: {info}");
        default_hook(info);
    }));
    hex_game::run()
}
