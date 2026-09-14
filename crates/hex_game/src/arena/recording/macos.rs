//! Safe, main-thread removal of the native shortcut that bypasses finalization.

use bevy::ecs::system::NonSendMarker;
use bevy::prelude::*;
use bevy::winit::EventLoopProxyWrapper;
use objc2::{sel, MainThreadMarker};
use objc2_app_kit::{NSApplication, NSMenu};

use super::super::ViewState;

pub(super) fn install(app: &mut App) {
    app.add_systems(Startup, remove_native_terminate);
}

fn remove_native_terminate(
    _main_thread: NonSendMarker,
    native_loop: Option<Res<EventLoopProxyWrapper>>,
    view: Res<ViewState>,
) {
    // Winit creates its menu before NewEvents, hence before Bevy's Startup.
    // Logical tests and image-target captures never initialize native AppKit.
    if native_loop.is_none() || view.capture.is_some() {
        return;
    }
    let Some(main_thread) = MainThreadMarker::new() else {
        error!("Cannot install graceful native quit away from the main thread");
        return;
    };
    if let Some(menu) = NSApplication::sharedApplication(main_thread).mainMenu() {
        remove_terminate_items(&menu);
    }
}

fn remove_terminate_items(menu: &NSMenu) {
    // Retain a snapshot before mutation; keep About, Services, Hide and other
    // native actions. The game's Super+Q and Esc Quit share Recorder control.
    for item in menu.itemArray().to_vec() {
        if item.action() == Some(sel!(terminate:)) {
            menu.removeItem(&item);
        } else if let Some(submenu) = item.submenu() {
            remove_terminate_items(&submenu);
        }
    }
}
