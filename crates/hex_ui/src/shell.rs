use bevy::input_focus::tab_navigation::TabGroup;
use bevy::prelude::*;
use hex_assets::{to_color, MenuSettings};
use hex_core::Screen;

const FALLBACK_BACKGROUND: Color = Color::srgb(0.10, 0.11, 0.14);

#[derive(Component)]
struct MenuBackground;

/// Marks an entity as belonging to a screen so one generic system can remove it.
#[derive(Component, Debug, Clone, Copy)]
pub struct DespawnOnExit(pub Screen);

pub(super) fn plugin(app: &mut App) {
    app.add_systems(Update, paint_menu_background);
}

/// Returns a system that despawns every entity owned by the given screen.
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

/// Opaque full-screen root for a menu-like screen.
#[must_use]
pub fn screen_root(screen: Screen, name: &'static str) -> impl Bundle {
    (
        Name::new(name),
        screen_root_node(),
        BackgroundColor(FALLBACK_BACKGROUND),
        MenuBackground,
        TabGroup::new(0),
        DespawnOnExit(screen),
    )
}

/// Responsive layout portion of [`screen_root`].
#[must_use]
pub fn screen_root_node() -> Node {
    Node {
        width: Val::Percent(100.0),
        height: Val::Percent(100.0),
        align_items: AlignItems::Center,
        justify_content: JustifyContent::Center,
        flex_direction: FlexDirection::Column,
        row_gap: Val::Px(16.0),
        overflow: Overflow::clip_y(),
        ..default()
    }
}

/// Full-screen modal overlay that dims the underlying surface.
#[must_use]
pub fn overlay_root(name: &'static str) -> impl Bundle {
    (
        Name::new(name),
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(12.0),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.68)),
        TabGroup::modal(),
        crate::focus::ModalFocusScope,
        GlobalZIndex(10),
    )
}

fn paint_menu_background(
    settings: Option<Res<MenuSettings>>,
    mut panels: Query<&mut BackgroundColor, With<MenuBackground>>,
) {
    let Some(settings) = settings else { return };
    let wanted = to_color(settings.background);
    for mut panel in &mut panels {
        if panel.0 != wanted {
            panel.0 = wanted;
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::{state::app::StatesPlugin, MinimalPlugins};

    use super::*;

    #[test]
    fn screens_are_opaque_before_authored_settings_arrive() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin));
        let entity = app
            .world_mut()
            .spawn(screen_root(Screen::Title, "Test Screen"))
            .id();
        assert_eq!(
            app.world()
                .entity(entity)
                .get::<BackgroundColor>()
                .map(|background| background.0),
            Some(FALLBACK_BACKGROUND)
        );
    }
}
