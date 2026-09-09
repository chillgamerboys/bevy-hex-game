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
    if std::env::args().any(|arg| arg == "--arena") {
        #[cfg(feature = "arena-prototype")]
        return hex_game::arena::run();
        #[cfg(not(feature = "arena-prototype"))]
        {
            #[expect(
                clippy::print_stderr,
                reason = "report an unavailable launch capability before logging is initialized"
            )]
            {
                eprintln!("Battle Mode is unavailable in this build. Enable arena-prototype or use cargo battle.");
            }
            return AppExit::error();
        }
    }
    hex_game::run()
}
