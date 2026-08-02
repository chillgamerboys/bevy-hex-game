//! Settings application adapter. Rendering belongs to `hex_ui`.

use bevy::prelude::*;
use hex_core::{
    BindingConflict, BindingEditError, InputAction, InputBindings, KeyChord, KeyModifiers, Screen,
};
use hex_ui::{
    SettingsIntent, SettingsModalView, SettingsTab, UiBindingRow, UiIntent, UiSetting,
    UiSettingRow, UiSettingsView, UiSystems,
};

use crate::preferences::{PreferencesDirty, PreferencesNotice, UserPreferences};

#[derive(Resource, Debug, Default)]
struct SettingsSession {
    tab: SettingsTab,
    capture: Option<InputAction>,
    conflict: Option<BindingConflict>,
    confirm_restore_all: bool,
}

impl SettingsSession {
    fn dismiss_modal(&mut self) -> bool {
        let dismissed =
            self.capture.is_some() || self.conflict.is_some() || self.confirm_restore_all;
        self.capture = None;
        self.conflict = None;
        self.confirm_restore_all = false;
        dismissed
    }
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<SettingsSession>()
        .add_systems(OnEnter(Screen::Settings), reset_settings_session)
        .add_systems(
            PreUpdate,
            capture_next_key
                .in_set(UiSystems::CaptureInput)
                .after(bevy::input::InputSystems)
                .run_if(in_state(Screen::Settings)),
        )
        .add_systems(
            Update,
            (
                handle_settings.after(UiSystems::EmitIntents),
                publish_settings_view,
            )
                .chain()
                .run_if(in_state(Screen::Settings)),
        );
}

fn reset_settings_session(mut session: ResMut<SettingsSession>) {
    *session = SettingsSession::default();
}

fn capture_next_key(
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut session: ResMut<SettingsSession>,
    mut preferences: ResMut<UserPreferences>,
    mut dirty: ResMut<PreferencesDirty>,
    mut notice: ResMut<PreferencesNotice>,
) {
    if session.capture.is_none() {
        if (session.conflict.is_some() || session.confirm_restore_all)
            && keys.just_pressed(KeyCode::Escape)
        {
            consume_just_pressed(&mut keys);
            session.dismiss_modal();
        }
        return;
    }

    if keys.just_pressed(KeyCode::Escape) {
        consume_just_pressed(&mut keys);
        session.capture = None;
        return;
    }
    let Some(chord) = captured_chord(&keys) else {
        return;
    };
    consume_just_pressed(&mut keys);

    let Some(action) = session.capture.take() else {
        return;
    };
    let mut resolved = InputBindings::from_overrides(preferences.binding_overrides.clone());
    match resolved.assign(action, chord) {
        Ok(()) => store_bindings(&mut preferences, &mut dirty, &resolved),
        Err(BindingEditError::Conflict(conflict)) => session.conflict = Some(conflict),
        Err(error) => notice.0 = Some(format!("Binding was refused: {error}")),
    }
}

fn captured_chord(input: &ButtonInput<KeyCode>) -> Option<KeyChord> {
    let modifiers = KeyModifiers::from_input(input);
    input
        .get_just_pressed()
        .copied()
        .filter_map(|key| KeyChord::new(key, modifiers))
        .min_by_key(|chord| chord.key)
}

fn consume_just_pressed(input: &mut ButtonInput<KeyCode>) {
    let pressed = input.get_just_pressed().copied().collect::<Vec<_>>();
    for key in pressed {
        let _ = input.clear_just_pressed(key);
    }
}

fn handle_settings(
    mut intents: MessageReader<UiIntent>,
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<InputBindings>,
    mut session: ResMut<SettingsSession>,
    mut preferences: ResMut<UserPreferences>,
    mut dirty: ResMut<PreferencesDirty>,
    mut notice: ResMut<PreferencesNotice>,
    mut next: ResMut<NextState<Screen>>,
) {
    if bindings.just_pressed(&keys, InputAction::Cancel) && !session.dismiss_modal() {
        next.set(Screen::Title);
    }
    for intent in intents.read() {
        let UiIntent::Settings(intent) = intent else {
            continue;
        };
        match *intent {
            SettingsIntent::SelectTab(tab) => session.tab = tab,
            SettingsIntent::Adjust(setting) => {
                adjust_general_setting(&mut preferences, setting);
                dirty.0 = true;
            }
            SettingsIntent::BeginCapture(action) => {
                if action.metadata().rebindable && action_is_available(action) {
                    session.capture = Some(action);
                    session.conflict = None;
                    session.confirm_restore_all = false;
                }
            }
            SettingsIntent::CancelCapture => session.capture = None,
            SettingsIntent::SwapConflict => {
                let Some(conflict) = session.conflict.take() else {
                    continue;
                };
                let mut resolved =
                    InputBindings::from_overrides(preferences.binding_overrides.clone());
                match resolved.swap(conflict.requested, conflict.existing) {
                    Ok(()) => store_bindings(&mut preferences, &mut dirty, &resolved),
                    Err(error) => {
                        notice.0 = Some(format!("Bindings could not be swapped: {error}"))
                    }
                }
            }
            SettingsIntent::CancelConflict => session.conflict = None,
            SettingsIntent::RestoreBinding(action) => {
                if !action.metadata().rebindable || !action_is_available(action) {
                    continue;
                }
                let mut resolved =
                    InputBindings::from_overrides(preferences.binding_overrides.clone());
                resolved.restore(action);
                store_bindings(&mut preferences, &mut dirty, &resolved);
            }
            SettingsIntent::RequestRestoreAll => {
                if !preferences.binding_overrides.is_empty() {
                    session.confirm_restore_all = true;
                    session.capture = None;
                    session.conflict = None;
                }
            }
            SettingsIntent::ConfirmRestoreAll => {
                if session.confirm_restore_all {
                    session.confirm_restore_all = false;
                    let resolved = InputBindings::default();
                    store_bindings(&mut preferences, &mut dirty, &resolved);
                }
            }
            SettingsIntent::CancelRestoreAll => session.confirm_restore_all = false,
            SettingsIntent::Back => {
                if !session.dismiss_modal() {
                    next.set(Screen::Title);
                }
            }
        }
    }
}

fn store_bindings(
    preferences: &mut UserPreferences,
    dirty: &mut PreferencesDirty,
    bindings: &InputBindings,
) {
    if preferences.binding_overrides != *bindings.overrides() {
        preferences.binding_overrides = bindings.overrides().clone();
        dirty.0 = true;
    }
}

fn adjust_general_setting(preferences: &mut UserPreferences, setting: UiSetting) {
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
        UiSetting::Presentation => preferences.presentation = preferences.presentation.next(),
        UiSetting::UiScale => preferences.ui_scale = preferences.ui_scale.next(),
        UiSetting::MasterVolume => cycle_volume(&mut preferences.master_volume),
        UiSetting::MusicVolume => cycle_volume(&mut preferences.music_volume),
        UiSetting::EffectsVolume => cycle_volume(&mut preferences.effects_volume),
        UiSetting::UiVolume => cycle_volume(&mut preferences.ui_volume),
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
    session: Res<SettingsSession>,
    notice: Res<PreferencesNotice>,
    mut view: ResMut<UiSettingsView>,
) {
    let next = project_settings_view(&preferences, &session, notice.0.clone());
    if *view != next {
        *view = next;
    }
}

fn project_settings_view(
    preferences: &UserPreferences,
    session: &SettingsSession,
    notice: Option<String>,
) -> UiSettingsView {
    let resolved = InputBindings::from_overrides(preferences.binding_overrides.clone());
    let rows = if session.tab == SettingsTab::General {
        general_rows(preferences)
    } else {
        Vec::new()
    };
    let bindings = session
        .tab
        .input_category()
        .map_or_else(Vec::new, |category| {
            InputAction::ALL
                .into_iter()
                .filter(|action| {
                    action.metadata().category == category && action_is_available(*action)
                })
                .map(|action| {
                    let metadata = action.metadata();
                    UiBindingRow {
                        action,
                        label: metadata.label.to_owned(),
                        chord: resolved.chord(action).label(),
                        rebindable: metadata.rebindable,
                        overridden: preferences.binding_overrides.get(action).is_some(),
                    }
                })
                .collect()
        });
    let modal = if let Some(action) = session.capture {
        Some(SettingsModalView::Capture {
            action,
            label: action.metadata().label.to_owned(),
        })
    } else if let Some(conflict) = session.conflict {
        Some(SettingsModalView::Conflict {
            requested: conflict.requested.metadata().label.to_owned(),
            existing: conflict.existing.metadata().label.to_owned(),
            chord: conflict.chord.label(),
        })
    } else if session.confirm_restore_all {
        Some(SettingsModalView::ConfirmRestoreAll)
    } else {
        None
    };
    UiSettingsView {
        tab: session.tab,
        rows,
        bindings,
        can_restore_all: !preferences.binding_overrides.is_empty(),
        modal,
        notice,
    }
}

fn action_is_available(action: InputAction) -> bool {
    action != InputAction::RevealAll || cfg!(feature = "dev")
}

fn general_rows(preferences: &UserPreferences) -> Vec<UiSettingRow> {
    vec![
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
    ]
}

fn percent(value: f32) -> String {
    format!("{:.0}%", value * 100.0)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use bevy::ecs::system::RunSystemOnce;

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

    #[test]
    fn direct_binding_tabs_project_every_action_in_one_category() {
        let preferences = UserPreferences::default();
        let projected = SettingsTab::ALL
            .into_iter()
            .flat_map(|tab| {
                project_settings_view(&preferences, &SettingsSession { tab, ..default() }, None)
                    .bindings
            })
            .map(|row| row.action)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            projected,
            InputAction::ALL
                .into_iter()
                .filter(|action| action_is_available(*action))
                .collect::<BTreeSet<_>>()
        );
    }

    #[test]
    fn fixed_actions_remain_visible_but_cannot_start_capture() {
        let view = project_settings_view(
            &UserPreferences::default(),
            &SettingsSession {
                tab: SettingsTab::System,
                ..default()
            },
            None,
        );
        let cancel = view
            .bindings
            .iter()
            .find(|row| row.action == InputAction::Cancel)
            .expect("fixed Back action is projected");
        assert!(!cancel.rebindable);
        assert_eq!(cancel.chord, "Esc");
    }

    #[test]
    fn capture_ignores_pure_modifiers_and_preserves_modifiers_on_a_primary_key() {
        let mut input = ButtonInput::default();
        input.press(KeyCode::ShiftLeft);
        assert_eq!(captured_chord(&input), None);

        input.press(KeyCode::KeyY);
        let chord = captured_chord(&input).expect("modified primary key is captured");
        assert_eq!(chord.label(), "Shift+Y");
    }

    #[test]
    fn capture_consumes_the_raw_key_before_other_input_handlers() {
        let mut world = World::new();
        world.init_resource::<SettingsSession>();
        world.init_resource::<UserPreferences>();
        world.init_resource::<PreferencesDirty>();
        world.init_resource::<PreferencesNotice>();
        world.init_resource::<ButtonInput<KeyCode>>();
        world.resource_mut::<SettingsSession>().capture = Some(InputAction::ToggleParty);
        world
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyY);

        world
            .run_system_once(capture_next_key)
            .expect("capture system has all required resources");

        assert!(!world
            .resource::<ButtonInput<KeyCode>>()
            .just_pressed(KeyCode::KeyY));
        assert_eq!(
            world
                .resource::<UserPreferences>()
                .binding_overrides
                .get(InputAction::ToggleParty),
            Some(KeyChord::plain(KeyCode::KeyY))
        );
        assert!(world.resource::<PreferencesDirty>().0);
    }

    #[test]
    fn capture_consumes_every_simultaneous_key_so_none_leak_to_focus() {
        let mut world = World::new();
        world.init_resource::<SettingsSession>();
        world.init_resource::<UserPreferences>();
        world.init_resource::<PreferencesDirty>();
        world.init_resource::<PreferencesNotice>();
        world.init_resource::<ButtonInput<KeyCode>>();
        world.resource_mut::<SettingsSession>().capture = Some(InputAction::ToggleParty);
        {
            let mut input = world.resource_mut::<ButtonInput<KeyCode>>();
            input.press(KeyCode::KeyY);
            input.press(KeyCode::Space);
        }

        world
            .run_system_once(capture_next_key)
            .expect("capture system has all required resources");

        assert!(world
            .resource::<ButtonInput<KeyCode>>()
            .get_just_pressed()
            .next()
            .is_none());
    }

    #[test]
    fn escape_wins_a_simultaneous_capture_batch() {
        let mut world = World::new();
        world.init_resource::<SettingsSession>();
        world.init_resource::<UserPreferences>();
        world.init_resource::<PreferencesDirty>();
        world.init_resource::<PreferencesNotice>();
        world.init_resource::<ButtonInput<KeyCode>>();
        world.resource_mut::<SettingsSession>().capture = Some(InputAction::ToggleParty);
        {
            let mut input = world.resource_mut::<ButtonInput<KeyCode>>();
            input.press(KeyCode::KeyY);
            input.press(KeyCode::Escape);
        }

        world
            .run_system_once(capture_next_key)
            .expect("capture system has all required resources");

        assert!(world.resource::<SettingsSession>().capture.is_none());
        assert!(world
            .resource::<UserPreferences>()
            .binding_overrides
            .is_empty());
        assert!(world
            .resource::<ButtonInput<KeyCode>>()
            .get_just_pressed()
            .next()
            .is_none());
    }

    #[test]
    fn escape_cancels_capture_without_changing_a_binding() {
        let mut world = World::new();
        world.init_resource::<SettingsSession>();
        world.init_resource::<UserPreferences>();
        world.init_resource::<PreferencesDirty>();
        world.init_resource::<PreferencesNotice>();
        world.init_resource::<ButtonInput<KeyCode>>();
        world.resource_mut::<SettingsSession>().capture = Some(InputAction::ToggleParty);
        world
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Escape);

        world
            .run_system_once(capture_next_key)
            .expect("capture system has all required resources");

        assert!(world.resource::<SettingsSession>().capture.is_none());
        assert!(world
            .resource::<UserPreferences>()
            .binding_overrides
            .is_empty());
        assert!(!world.resource::<PreferencesDirty>().0);
        assert!(!world
            .resource::<ButtonInput<KeyCode>>()
            .just_pressed(KeyCode::Escape));
    }
}
