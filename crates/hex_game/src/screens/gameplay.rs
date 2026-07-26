//! The game itself.
//!
//! Owns the pause toggle, the route back to the title screen, and the HUD. The world
//! and the units are spawned by `hex_map`, `hex_world` and `hex_units` on
//! `OnEnter(Screen::Gameplay)`; this module deliberately does not reach into any of
//! them. It reads `Mode` and `TurnOrder` to describe what is happening, and writes
//! neither.

use bevy::prelude::*;
use hex_combat::{Turn, TurnOrder};
use hex_core::{Mode, Pause, Screen};
use hex_units::Player;

use super::{despawn_screen, DespawnOnExit};

pub(super) fn plugin(app: &mut App) {
    app.add_sub_state::<Pause>();
    app.register_type::<Pause>();
    // A second sub-state of `Screen::Gameplay`, independent of `Pause`. Both are
    // computed from the screen, so pausing does not destroy the mode.
    app.add_sub_state::<Mode>();
    app.register_type::<Mode>();

    app.add_systems(Update, handle_input.run_if(in_state(Screen::Gameplay)));
    app.add_systems(Update, update_hud.run_if(in_state(Screen::Gameplay)));
    app.add_systems(
        OnEnter(Screen::Gameplay),
        (reset_pause, reset_mode, spawn_hud),
    );
    app.add_systems(OnExit(Screen::Gameplay), despawn_screen(Screen::Gameplay));
}

/// Marks the HUD line so it can be rewritten as the mode changes.
#[derive(Component)]
struct HudText;

/// Controls are otherwise undiscoverable — there is no manual and no tutorial.
fn spawn_hud(mut commands: Commands) {
    commands
        .spawn((
            Name::new("Gameplay HUD"),
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(12.0),
                left: Val::Px(12.0),
                ..default()
            },
            // Without this the HUD swallows clicks on any tile behind it, and
            // click-to-move silently stops working in the bottom-left corner.
            Pickable::IGNORE,
            DespawnOnExit(Screen::Gameplay),
        ))
        .with_children(|parent| {
            parent.spawn((
                HudText,
                Text::new(exploring_hint()),
                TextFont::from_font_size(14.0),
                TextColor(Color::srgba(0.9, 0.9, 0.9, 0.7)),
            ));
        });
}

fn exploring_hint() -> String {
    "EXPLORING    -    click a tile to move    -    right-drag to orbit    -    \
     WASD to pan    -    scroll to zoom    -    ESC to pause"
        .to_owned()
}

/// Rewrites the hint line to say what the game is doing and whose turn it is.
///
/// The turn order is the only readout a player gets — there is no combat log and no
/// unit portraits — so it has to say enough to act on: which mode, which round, and
/// whether this is your go.
fn update_hud(
    mode: Res<State<Mode>>,
    order: Res<TurnOrder>,
    acting: Query<Has<Player>, With<Turn>>,
    mut hud: Query<&mut Text, With<HudText>>,
) {
    let Ok(mut text) = hud.single_mut() else {
        return;
    };

    let wanted = match mode.get() {
        Mode::Exploring => exploring_hint(),
        Mode::Combat => {
            let whose = match acting.single() {
                Ok(true) => "your turn",
                Ok(false) => "enemy turn",
                Err(_) => "…",
            };
            format!(
                "COMBAT    -    round {}    -    {}    -    SPACE to end turn    \
                 -    ESC to pause",
                order.round + 1,
                whose
            )
        }
    };

    if text.0 != wanted {
        text.0 = wanted;
    }
}

/// Entering gameplay always starts unpaused, so a pause left set from a previous
/// session cannot leak into a new one.
fn reset_pause(mut next: ResMut<NextState<Pause>>) {
    next.set(Pause(false));
}

/// And always starts out of combat, for the same reason.
fn reset_mode(mut next: ResMut<NextState<Mode>>) {
    next.set(Mode::Exploring);
}

fn handle_input(
    keys: Res<ButtonInput<KeyCode>>,
    pause: Res<State<Pause>>,
    mut next_pause: ResMut<NextState<Pause>>,
    mut next_screen: ResMut<NextState<Screen>>,
) {
    if keys.just_pressed(KeyCode::Escape) {
        next_pause.set(Pause(!pause.get().0));
    }
    // Backspace rather than Escape, which is taken by pause.
    if keys.just_pressed(KeyCode::Backspace) {
        next_screen.set(Screen::Title);
    }
}
