//! Pause overlay.
//!
//! Exists because a pause with no visible state is indistinguishable from a
//! crash: the world simply stops responding and nothing explains why. Anything
//! that suspends the game needs to say so on screen.

use bevy::prelude::*;
use hex_core::Pause;

use crate::save::ResumeNotice;
use crate::screens::combat_lab::CombatLabSession;

use super::overlay_root;
use super::widgets::{blurb, display, UiAssets};

pub(super) fn plugin(app: &mut App) {
    app.add_systems(OnEnter(Pause(true)), spawn_pause_menu);
    app.add_systems(Update, update_resume_notice.run_if(in_state(Pause(true))));
    app.add_systems(OnExit(Pause(true)), despawn_pause_menu);
}

#[derive(Component)]
struct PauseMenu;

#[derive(Component)]
struct ResumeNoticeText;

fn spawn_pause_menu(
    mut commands: Commands,
    assets: Res<UiAssets>,
    lab: Option<Res<CombatLabSession>>,
) {
    let hint = if lab.is_some() {
        "ESC to resume   ·   F5 save disabled for Combat Lab   ·   BACKSPACE to return"
    } else {
        "ESC to resume   ·   F5 to save exploration   ·   BACKSPACE to title"
    };
    commands
        .spawn((overlay_root("Pause Menu"), PauseMenu))
        .with_children(|parent| {
            parent.spawn(display(&assets, "Paused"));
            parent.spawn(blurb(&assets, hint));
            parent.spawn((ResumeNoticeText, blurb(&assets, "")));
        });
}

fn update_resume_notice(
    notice: Option<Res<ResumeNotice>>,
    mut text: Query<&mut Text, With<ResumeNoticeText>>,
) {
    let Some(notice) = notice else { return };
    if !notice.is_changed() {
        return;
    }
    for mut text in &mut text {
        text.0 = notice.0.clone().unwrap_or_default();
    }
}

fn despawn_pause_menu(mut commands: Commands, menus: Query<Entity, With<PauseMenu>>) {
    for entity in &menus {
        commands.entity(entity).despawn();
    }
}
