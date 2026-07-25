//! The screens the player moves through, and the transitions between them.
//!
//! Each screen owns its own entities and despawns them on exit, using the
//! [`DespawnOnExit`] pattern: everything a screen spawns is tagged, and one
//! generic system clears the tag's entities when the state changes. That keeps
//! teardown from being a per-screen checklist somebody forgets to update.

use bevy::prelude::*;
use hex_core::Screen;

mod gameplay;
mod loading;
mod splash;
mod title;

pub(super) fn plugin(app: &mut App) {
    app.init_state::<Screen>();
    app.register_type::<Screen>();

    app.add_plugins((
        splash::plugin,
        title::plugin,
        loading::plugin,
        gameplay::plugin,
    ));
}

/// Marks an entity as belonging to a screen, so it is despawned on exit.
#[derive(Component, Debug, Clone, Copy)]
pub struct DespawnOnExit(pub Screen);

/// Despawns everything tagged for the screen being left.
pub fn despawn_screen(
    screen: Screen,
) -> impl FnMut(Commands, Query<(Entity, &DespawnOnExit)>) + Clone {
    move |mut commands: Commands, query: Query<(Entity, &DespawnOnExit)>| {
        for (entity, tag) in &query {
            if tag.0 == screen {
                commands.entity(entity).despawn();
            }
        }
    }
}

/// A full-screen container, used by the menu-like screens.
pub fn screen_root(screen: Screen, name: &'static str) -> impl Bundle {
    (
        Name::new(name),
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(16.0),
            ..default()
        },
        DespawnOnExit(screen),
    )
}
