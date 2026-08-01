//! Settings application adapter. Rendering belongs to `hex_ui`.

use bevy::prelude::*;
use hex_core::{InputAction, InputBindings, Screen};
use hex_ui::{UiIntent, UiSetting, UiSettingRow, UiSettingsView};

use crate::preferences::{PreferencesDirty, PreferencesNotice, UserPreferences};

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        (handle_settings, publish_settings_view)
            .chain()
            .run_if(in_state(Screen::Settings)),
    );
}

fn handle_settings(
    mut intents: MessageReader<UiIntent>,
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<InputBindings>,
    mut preferences: ResMut<UserPreferences>,
    mut dirty: ResMut<PreferencesDirty>,
    mut next: ResMut<NextState<Screen>>,
) {
    if bindings.just_pressed(&keys, InputAction::Cancel) {
        next.set(Screen::Title);
    }
    for intent in intents.read() {
        match intent {
            UiIntent::AdjustSetting(setting) => {
                match setting {
                    UiSetting::Fullscreen => preferences.fullscreen = !preferences.fullscreen,
                    UiSetting::WindowSize => {
                        (preferences.window_width, preferences.window_height) =
                            match (preferences.window_width, preferences.window_height) {
                                (1280, 720) => (1600, 900),
                                (1600, 900) => (1920, 1080),
                                _ => (1280, 720),
                            };
                    }
                    UiSetting::Presentation => {
                        preferences.presentation = preferences.presentation.next();
                    }
                    UiSetting::UiScale => preferences.ui_scale = preferences.ui_scale.next(),
                    UiSetting::MasterVolume => cycle_volume(&mut preferences.master_volume),
                    UiSetting::MusicVolume => cycle_volume(&mut preferences.music_volume),
                    UiSetting::EffectsVolume => cycle_volume(&mut preferences.effects_volume),
                    UiSetting::UiVolume => cycle_volume(&mut preferences.ui_volume),
                }
                dirty.0 = true;
            }
            UiIntent::Back => next.set(Screen::Title),
            _ => {}
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

fn publish_settings_view(
    preferences: Res<UserPreferences>,
    notice: Res<PreferencesNotice>,
    mut view: ResMut<UiSettingsView>,
) {
    let next = UiSettingsView {
        rows: vec![
            UiSettingRow {
                setting: UiSetting::Fullscreen,
                label: "Fullscreen".to_owned(),
                value: if preferences.fullscreen { "On" } else { "Off" }.to_owned(),
            },
            UiSettingRow {
                setting: UiSetting::WindowSize,
                label: "Window size".to_owned(),
                value: format!(
                    "{} × {}",
                    preferences.window_width, preferences.window_height
                ),
            },
            UiSettingRow {
                setting: UiSetting::Presentation,
                label: "Presentation".to_owned(),
                value: preferences.presentation.label().to_owned(),
            },
            UiSettingRow {
                setting: UiSetting::UiScale,
                label: "UI scale".to_owned(),
                value: preferences.ui_scale.label().to_owned(),
            },
            UiSettingRow {
                setting: UiSetting::MasterVolume,
                label: "Master volume".to_owned(),
                value: percent(preferences.master_volume),
            },
            UiSettingRow {
                setting: UiSetting::MusicVolume,
                label: "Music volume".to_owned(),
                value: percent(preferences.music_volume),
            },
            UiSettingRow {
                setting: UiSetting::EffectsVolume,
                label: "Effects volume".to_owned(),
                value: percent(preferences.effects_volume),
            },
            UiSettingRow {
                setting: UiSetting::UiVolume,
                label: "UI volume".to_owned(),
                value: percent(preferences.ui_volume),
            },
        ],
        notice: notice.0.clone(),
    };
    if *view != next {
        *view = next;
    }
}

fn percent(value: f32) -> String {
    format!("{:.0}%", value * 100.0)
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
