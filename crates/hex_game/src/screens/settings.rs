//! Small persistent settings surface for the Wave 5 app shell.

use bevy::prelude::*;
use hex_core::{InputAction, InputBindings, Screen};

use crate::menus::widgets::{blurb, button, display, heading, label, panel, UiAssets};
use crate::preferences::{PreferencesDirty, PreferencesNotice, UserPreferences};

use super::{despawn_screen, screen_root};

pub(super) fn plugin(app: &mut App) {
    app.add_systems(OnEnter(Screen::Settings), spawn_settings)
        .add_systems(
            Update,
            (handle_settings, update_values)
                .chain()
                .run_if(in_state(Screen::Settings)),
        )
        .add_systems(OnExit(Screen::Settings), despawn_screen(Screen::Settings));
}

#[derive(Component, Debug, Clone, Copy)]
enum SettingsAction {
    ToggleFullscreen,
    CycleResolution,
    CyclePresentation,
    CycleMaster,
    CycleMusic,
    CycleEffects,
    CycleUi,
    Back,
}

#[derive(Component, Debug, Clone, Copy)]
struct SettingsValue(SettingsAction);

#[derive(Component)]
struct SettingsNotice;

fn spawn_settings(mut commands: Commands, assets: Res<UiAssets>) {
    commands
        .spawn(screen_root(Screen::Settings, "Settings Screen"))
        .with_children(|root| {
            root.spawn(display(&assets, "Settings"));
            root.spawn(blurb(
                &assets,
                "Pre-alpha preferences persist locally; input bindings are fixed for now.",
            ));
            root.spawn(panel())
                .insert(Node {
                    width: Val::Px(620.0),
                    max_width: Val::Percent(92.0),
                    padding: UiRect::all(Val::Px(18.0)),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(10.0),
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(Val::Px(10.0)),
                    ..default()
                })
                .with_children(|panel| {
                    panel.spawn(heading(&assets, "display and volume"));
                    for (name, action) in [
                        ("Fullscreen", SettingsAction::ToggleFullscreen),
                        ("Window Size", SettingsAction::CycleResolution),
                        ("Presentation", SettingsAction::CyclePresentation),
                        ("Master Volume", SettingsAction::CycleMaster),
                        ("Music Volume", SettingsAction::CycleMusic),
                        ("Effects Volume", SettingsAction::CycleEffects),
                        ("UI Volume", SettingsAction::CycleUi),
                    ] {
                        panel
                            .spawn((button(name), action))
                            .with_child((label(&assets, ""), SettingsValue(action)));
                    }
                    panel
                        .spawn((button("Back"), SettingsAction::Back))
                        .with_child(label(&assets, "Back to title"));
                });
            root.spawn((
                SettingsNotice,
                blurb(&assets, ""),
                Node {
                    max_width: Val::Px(760.0),
                    ..default()
                },
            ));
        });
}

fn handle_settings(
    clicked: Query<(&Interaction, &SettingsAction), Changed<Interaction>>,
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<InputBindings>,
    mut preferences: ResMut<UserPreferences>,
    mut dirty: ResMut<PreferencesDirty>,
    mut next: ResMut<NextState<Screen>>,
) {
    if bindings.just_pressed(&keys, InputAction::Cancel) {
        next.set(Screen::Title);
    }
    for (interaction, action) in &clicked {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match action {
            SettingsAction::ToggleFullscreen => {
                preferences.fullscreen = !preferences.fullscreen;
                dirty.0 = true;
            }
            SettingsAction::CycleResolution => {
                (preferences.window_width, preferences.window_height) =
                    match (preferences.window_width, preferences.window_height) {
                        (1280, 720) => (1600, 900),
                        (1600, 900) => (1920, 1080),
                        _ => (1280, 720),
                    };
                dirty.0 = true;
            }
            SettingsAction::CyclePresentation => {
                preferences.presentation = preferences.presentation.next();
                dirty.0 = true;
            }
            SettingsAction::CycleMaster => {
                cycle_volume(&mut preferences.master_volume);
                dirty.0 = true;
            }
            SettingsAction::CycleMusic => {
                cycle_volume(&mut preferences.music_volume);
                dirty.0 = true;
            }
            SettingsAction::CycleEffects => {
                cycle_volume(&mut preferences.effects_volume);
                dirty.0 = true;
            }
            SettingsAction::CycleUi => {
                cycle_volume(&mut preferences.ui_volume);
                dirty.0 = true;
            }
            SettingsAction::Back => next.set(Screen::Title),
        }
    }
}

fn cycle_volume(volume: &mut f32) {
    *volume = if *volume >= 0.99 {
        0.0
    } else {
        ((*volume * 10.0).round() + 1.0) / 10.0
    };
}

fn update_values(
    preferences: Res<UserPreferences>,
    notice: Res<PreferencesNotice>,
    mut values: Query<(&SettingsValue, &mut Text), Without<SettingsNotice>>,
    mut notices: Query<&mut Text, With<SettingsNotice>>,
) {
    for (value, mut text) in &mut values {
        text.0 = match value.0 {
            SettingsAction::ToggleFullscreen => format!(
                "Fullscreen · {}",
                if preferences.fullscreen { "On" } else { "Off" }
            ),
            SettingsAction::CycleResolution => format!(
                "Window Size · {} × {}",
                preferences.window_width, preferences.window_height
            ),
            SettingsAction::CyclePresentation => {
                format!("Presentation · {}", preferences.presentation.label())
            }
            SettingsAction::CycleMaster => {
                format!("Master Volume · {:.0}%", preferences.master_volume * 100.0)
            }
            SettingsAction::CycleMusic => {
                format!("Music Volume · {:.0}%", preferences.music_volume * 100.0)
            }
            SettingsAction::CycleEffects => format!(
                "Effects Volume · {:.0}%",
                preferences.effects_volume * 100.0
            ),
            SettingsAction::CycleUi => {
                format!("UI Volume · {:.0}%", preferences.ui_volume * 100.0)
            }
            SettingsAction::Back => String::new(),
        };
    }
    for mut text in &mut notices {
        text.0 = notice.0.clone().unwrap_or_default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_cycles_in_ten_percent_steps() {
        let mut volume = 0.8;
        cycle_volume(&mut volume);
        assert!((volume - 0.9).abs() <= f32::EPSILON);
        cycle_volume(&mut volume);
        assert!((volume - 1.0).abs() <= f32::EPSILON);
        cycle_volume(&mut volume);
        assert!(volume.abs() <= f32::EPSILON);
    }
}
