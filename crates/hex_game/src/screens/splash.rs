//! Brief hold on startup, then straight to the title screen.
//!
//! Exists as a real state rather than a cosmetic flourish: it gives the window
//! and renderer a frame or two to settle before anything is measured or shown,
//! and it is the conventional place to put a studio logo later.

use bevy::prelude::*;
use hex_core::Screen;

const SPLASH_SECONDS: f32 = 0.8;

pub(super) fn plugin(app: &mut App) {
    app.add_systems(OnEnter(Screen::Splash), start_splash_timer);
    app.add_systems(Update, advance_to_title.run_if(in_state(Screen::Splash)));
}

#[derive(Resource)]
struct SplashTimer(Timer);

fn start_splash_timer(mut commands: Commands) {
    commands.insert_resource(SplashTimer(Timer::from_seconds(
        SPLASH_SECONDS,
        TimerMode::Once,
    )));
}

fn advance_to_title(
    time: Res<Time>,
    mut timer: ResMut<SplashTimer>,
    mut next: ResMut<NextState<Screen>>,
) {
    if timer.0.tick(time.delta()).just_finished() {
        next.set(Screen::Title);
    }
}
