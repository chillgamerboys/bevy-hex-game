//! The screens the player moves through, and the transitions between them.
//!
//! Each screen owns its own entities and despawns them on exit, using the
//! [`DespawnOnExit`] pattern: everything a screen spawns is tagged, and one
//! generic system clears the tag's entities when the state changes. That keeps
//! teardown from being a per-screen checklist somebody forgets to update.

use bevy::prelude::*;
use hex_assets::{to_color, MenuSettings};
use hex_core::Screen;

mod gameplay;
mod lattice_demo;
mod loading;
mod settings;
mod splash;
pub(crate) mod title;

pub(super) fn plugin(app: &mut App) {
    app.init_state::<Screen>();
    app.register_type::<Screen>();

    app.add_plugins((
        splash::plugin,
        title::plugin,
        settings::plugin,
        lattice_demo::plugin,
        loading::plugin,
        gameplay::plugin,
    ));

    app.add_systems(Update, paint_menu_background);
}

/// Marks a screen's backing panel, so its colour can be kept in step with settings.
#[derive(Component)]
struct MenuBackground;

/// The colour a menu screen is drawn in before `menu.ron` has parsed.
///
/// **Not a nicety.** `Screen::Splash` is the *default* state, so it renders on the very
/// first frame — before any RON has been read. Something has to be on screen, and the
/// alternative to a constant is Bevy's default clear colour showing through.
///
/// Kept close to the shipped value so the swap is invisible. `default_sky_params` in
/// `hex_world` exists for the same reason and says the same thing: a placeholder that
/// looks wrong is a bug report, not a diagnostic.
const FALLBACK_BACKGROUND: Color = Color::srgb(0.10, 0.11, 0.14);

/// Keeps the menu backdrop in step with `menu.ron`, including on hot reload.
fn paint_menu_background(
    settings: Option<Res<MenuSettings>>,
    mut panels: Query<&mut BackgroundColor, With<MenuBackground>>,
) {
    let Some(settings) = settings else { return };
    let wanted = to_color(settings.background);
    for mut panel in &mut panels {
        // Repainted whenever the settings change *or* a screen is newly spawned, which
        // is why this is not gated on `is_changed()` alone: a screen entered after the
        // last change would otherwise keep the fallback for ever.
        if panel.0 != wanted {
            panel.0 = wanted;
        }
    }
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
///
/// **Opaque**, which is what keeps the menus out of the world. Being a solid panel
/// rather than a chosen `ClearColor` means it does not matter what the 3D scene is
/// doing behind it, or when: a scenario's lighting arriving mid-load repaints
/// `ClearColor` under the loading screen, and nobody sees it.
///
/// Splash, title and loading all use this. The gameplay HUD deliberately does not —
/// there the world is the point.
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
        BackgroundColor(FALLBACK_BACKGROUND),
        MenuBackground,
        DespawnOnExit(screen),
    )
}

#[cfg(test)]
mod tests {
    use bevy::state::app::StatesPlugin;
    use bevy::MinimalPlugins;
    use hex_assets::MenuSettings;

    use super::*;

    /// A screen root on its own, with the repaint system the plugin registers.
    fn painted_app(settings: Option<MenuSettings>) -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin));
        app.init_state::<Screen>();
        if let Some(settings) = settings {
            app.insert_resource(settings);
        }
        app.add_systems(Update, paint_menu_background);
        app.world_mut()
            .spawn(screen_root(Screen::Title, "Test Screen"));
        app
    }

    fn background(app: &mut App) -> Option<Color> {
        let mut panels = app
            .world_mut()
            .query_filtered::<&BackgroundColor, With<MenuBackground>>();
        panels.iter(app.world()).next().map(|panel| panel.0)
    }

    /// A menu screen is opaque, and takes its colour from `menu.ron`.
    ///
    /// Opaque is the point rather than a detail: it is what stops the menus caring what
    /// the 3D scene is doing behind them. A scenario's lighting arriving mid-load
    /// repaints `ClearColor` while the loading screen is still up, and nobody sees it.
    #[test]
    fn a_menu_screen_takes_its_colour_from_settings() {
        let mut app = painted_app(Some(MenuSettings {
            background: (0.5, 0.25, 0.75),
        }));
        app.update();

        assert_eq!(
            background(&mut app),
            Some(Color::srgb(0.5, 0.25, 0.75)),
            "the screen ignored menu.ron"
        );
    }

    /// And renders regardless when the settings have not arrived.
    ///
    /// `Screen::Splash` is the **default** state, so a menu screen exists on the very
    /// first frame — before any RON has been read. Without a fallback the screen is
    /// whatever Bevy's clear colour happens to be, and this project has already shipped
    /// one crash from assuming a settings resource was there on this screen.
    #[test]
    fn a_menu_screen_renders_before_its_settings_load() {
        let mut app = painted_app(None);
        app.update();

        assert_eq!(
            background(&mut app),
            Some(FALLBACK_BACKGROUND),
            "a screen with no settings yet should still be opaque"
        );
    }

    /// A screen entered after the settings last changed still gets painted.
    ///
    /// The reason `paint_menu_background` is not gated on `is_changed()`: the title
    /// screen is spawned and despawned repeatedly, and `menu.ron` changes once. A
    /// change-driven version paints the first screen and leaves every later one on the
    /// fallback — which looks correct, because the fallback matches the shipped value.
    #[test]
    fn a_screen_spawned_later_is_painted_too() {
        let mut app = painted_app(Some(MenuSettings {
            background: (0.5, 0.25, 0.75),
        }));
        app.update();
        app.update();

        let later = app
            .world_mut()
            .spawn(screen_root(Screen::Loading, "Later Screen"))
            .id();
        app.update();

        let painted = app
            .world()
            .entity(later)
            .get::<BackgroundColor>()
            .map(|panel| panel.0);
        assert_eq!(
            painted,
            Some(Color::srgb(0.5, 0.25, 0.75)),
            "a screen spawned after the settings settled kept the fallback"
        );
    }
}
