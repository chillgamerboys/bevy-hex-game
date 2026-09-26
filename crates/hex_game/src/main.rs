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
    // A pre-opened file remains usable while thread-local tracing is being torn
    // down. Calling tracing here can panic again during Winit window destruction.
    let panic_log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(std::env::temp_dir().join("hex-game-panic.log"))
        .ok()
        .map(std::sync::Mutex::new);
    std::panic::set_hook(Box::new(move |info| {
        use std::io::Write;
        if let Some(log) = &panic_log {
            if let Ok(mut file) = log.try_lock() {
                let timestamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |duration| duration.as_secs());
                let _ = writeln!(file, "{timestamp} pid={} panic: {info}", std::process::id());
                let _ = file.flush();
            }
        }
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
                eprintln!(
                    "Battle Mode is unavailable in this build. Enable arena-prototype or use cargo battle."
                );
            }
            return AppExit::error();
        }
    }
    hex_game::run()
}
