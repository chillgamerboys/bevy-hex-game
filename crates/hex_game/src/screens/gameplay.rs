//! The game itself.
//!
//! Owns the pause toggle, the route back to the title screen, and the HUD. Terrain,
//! environment, and the units are spawned by `hex_map`, `hex_world`, and `hex_units`
//! on `OnEnter(Screen::Gameplay)`; this module deliberately does not reach into any
//! of them. It reads `Mode` and `TurnOrder` to describe what is happening, and
//! writes neither.

use bevy::prelude::*;
use hex_combat::{Turn, TurnOrder};
use hex_core::{Mode, PausableSystems, Pause, Screen};
use hex_lattice::{LatticeSpec, LatticeState};
use hex_units::Player;

use super::{despawn_screen, DespawnOnExit};
use crate::menus::widgets::UiAssets;

pub(super) fn plugin(app: &mut App) {
    app.add_sub_state::<Pause>();
    app.register_type::<Pause>();
    // A second sub-state of `Screen::Gameplay`, independent of `Pause`. Both are
    // computed from the screen, so pausing does not destroy the mode.
    app.add_sub_state::<Mode>();
    app.register_type::<Mode>();

    app.add_systems(Update, handle_input.run_if(in_state(Screen::Gameplay)));
    app.add_systems(Update, update_hud.run_if(in_state(Screen::Gameplay)));
    // Pausable, because the system that acts on the flag is. `mirror_truth` runs in
    // `PausableSystems`, so a toggle that kept firing while paused would set the
    // resource with nothing to carry it out — leaving the store holding a full reveal
    // the flag says is off, which is the stale-and-authoritative state its own doc
    // calls worse than no reveal at all.
    #[cfg(feature = "dev")]
    app.add_systems(
        Update,
        toggle_reveal_all
            .in_set(PausableSystems)
            .run_if(in_state(Screen::Gameplay)),
    );
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
fn spawn_hud(mut commands: Commands, assets: Res<UiAssets>) {
    commands
        .spawn((
            Name::new("Gameplay HUD"),
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(12.0),
                left: Val::Px(12.0),
                right: Val::Px(12.0),
                padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                border_radius: BorderRadius::all(Val::Px(4.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.03, 0.04, 0.05, 0.78)),
            // Without this the HUD swallows clicks on any tile behind it, and
            // click-to-move silently stops working along the bottom edge.
            Pickable::IGNORE,
            DespawnOnExit(Screen::Gameplay),
        ))
        .with_children(|parent| {
            parent.spawn((
                HudText,
                Text::new(exploring_hint()),
                TextFont {
                    font: assets.body.clone().into(),
                    ..TextFont::from_font_size(14.0)
                },
                TextColor(Color::srgb(0.94, 0.94, 0.94)),
                Pickable::IGNORE,
            ));
        });
}

fn exploring_hint() -> String {
    "EXPLORING   ·   click a tile to move   ·   right-drag to orbit   ·   \
     WASD to pan   ·   scroll to zoom   ·   ESC to pause"
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
    acting: Query<(Has<Player>, &Turn)>,
    party: Query<(&LatticeSpec, &LatticeState), With<Player>>,
    mut hud: Query<&mut Text, With<HudText>>,
) {
    let Ok(mut text) = hud.single_mut() else {
        return;
    };

    let wanted = match mode.get() {
        Mode::Exploring => exploring_hint(),
        Mode::Combat => {
            // How much movement is left, spelled out. Without it a click refused for
            // being out of range is indistinguishable from a click that did not
            // register — which is precisely the complaint the tinted range answers,
            // and the number is what confirms the tint rather than merely repeating it.
            let whose = match acting.single() {
                Ok((true, turn)) => format!("your turn, {} to move", turn.movement_left),
                Ok((false, _)) => "enemy turn".to_owned(),
                Err(_) => "…".to_owned(),
            };
            format!(
                "COMBAT   ·   round {}   ·   {}{}   ·   SPACE to end turn   \
                 ·   ESC to pause",
                order.round + 1,
                whose,
                lattice_readout(&party)
            )
        }
    };

    if text.0 != wanted {
        text.0 = wanted;
    }
}

/// Flips the dev reveal-all toggle, so a designer can see the truth behind the
/// fog while playing.
///
/// Behind the `dev` feature deliberately: the shipped build has no key that
/// exposes hidden information, and hidden information is the game's source of
/// uncertainty rather than dice. `K` for knowledge — `Escape`, `Backspace`,
/// `Space`, `C`, `Enter` and `WASD` are all taken.
///
/// The resource is initialised by `hex_combat`'s plugin, which the binary always
/// adds, so this cannot be the observer-on-the-title-screen crash: it is a
/// system, it is gated on the gameplay screen, and its parameter always resolves.
///
/// Logs the new state because the reveal has no presentation of its own yet: units now
/// carry lattices, so the store fills, but nothing draws a hostile one. A silent key
/// that appears to do nothing is indistinguishable from a broken one.
#[cfg(feature = "dev")]
fn toggle_reveal_all(keys: Res<ButtonInput<KeyCode>>, mut reveal: ResMut<hex_combat::RevealAll>) {
    if keys.just_pressed(KeyCode::KeyK) {
        reveal.0 = !reveal.0;
        info!("reveal-all {}", if reveal.0 { "on" } else { "off" });
    }
}

/// Your party's hexes still standing, out of the hexes it started with.
///
/// A **sum across the party**, not one unit's lattice, and the label says "party" for
/// that reason: every shipped encounter fields one player unit today, so an unqualified
/// count would read correctly by accident and then quietly become an aggregate the first
/// time somebody adds a second line to a roster — which the encounter files openly invite.
/// Per-unit readouts are the casting-UX ticket's, not this one's.
///
/// Read straight off the components rather than through
/// [`FactionKnowledge`](hex_combat::FactionKnowledge): a faction's knowledge of *itself*
/// is not the question that store answers. It exists to gate what you know about a
/// **hostile** lattice, where seeing a unit reveals nothing about its contents — and
/// routing your own hexes through it would either need a self-view nothing publishes, or
/// teach the next reader that `view()` is how you look at anything, which is exactly the
/// confusion the two-channel split exists to prevent.
///
/// Empty while nothing carries a lattice, so a party of one inert unit reads exactly as
/// it did before this — no readout rather than a zero.
fn lattice_readout(party: &Query<(&LatticeSpec, &LatticeState), With<Player>>) -> String {
    let mut live = 0_usize;
    let mut total = 0_usize;
    for (spec, state) in party {
        // The spec is what says which cells exist; the state only says which of them
        // are down. Counting the state alone would miss every cell that has never been
        // touched, which early in a fight is all of them.
        for (coord, _) in spec.cells() {
            total += 1;
            if !state.is_disabled(coord) {
                live += 1;
            }
        }
    }
    if total == 0 {
        return String::new();
    }
    format!("   ·   party {live}/{total} hexes")
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

#[cfg(test)]
mod tests {
    use bevy::MinimalPlugins;

    use super::*;

    /// Every layer of the full-width HUD must let world picks pass through.
    ///
    /// Pickability is per entity, so ignoring only the backing node still leaves its
    /// text able to swallow tile clicks.
    #[test]
    fn gameplay_hud_does_not_block_tile_clicks() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(UiAssets {
            display: Handle::default(),
            body: Handle::default(),
            hex_cell: Handle::default(),
        });
        app.add_systems(Startup, spawn_hud);
        app.update();

        let mut roots = app
            .world_mut()
            .query_filtered::<&Pickable, With<BackgroundColor>>();
        assert!(
            roots
                .iter(app.world())
                .any(|pickable| *pickable == Pickable::IGNORE),
            "the HUD backing node blocks world picks"
        );

        let mut labels = app.world_mut().query_filtered::<&Pickable, With<HudText>>();
        assert_eq!(
            labels.iter(app.world()).next(),
            Some(&Pickable::IGNORE),
            "the HUD text blocks world picks"
        );
    }
}
