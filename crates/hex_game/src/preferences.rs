//! Persistent display preferences and replaceable audio/input seams.

use bevy::prelude::*;
use bevy::window::{MonitorSelection, PresentMode, WindowMode};
use hex_assets::{DisplaySettings, PresentModeSetting};
use hex_core::input::InputBindingOverrides;
use hex_core::InputBindings;
use hex_gameplay_model::HudComponentPreferences;
use serde::{Deserialize, Serialize};

use crate::storage::{read, write_atomic, StoragePaths};

const PREFERENCES_VERSION: u32 = 3;

/// Persisted frame-presentation choice.
#[derive(Serialize, Deserialize, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FramePresentation {
    /// Vsync capped to the display.
    #[default]
    Vsync,
    /// Prefer uncapped presentation.
    NoVsync,
    /// Prefer low-latency mailbox presentation where supported.
    Mailbox,
}

impl FramePresentation {
    pub(crate) const fn next(self) -> Self {
        match self {
            Self::Vsync => Self::NoVsync,
            Self::NoVsync => Self::Mailbox,
            Self::Mailbox => Self::Vsync,
        }
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Vsync => "Vsync",
            Self::NoVsync => "No Vsync",
            Self::Mailbox => "Mailbox",
        }
    }
}

impl From<FramePresentation> for PresentMode {
    fn from(value: FramePresentation) -> Self {
        match value {
            FramePresentation::Vsync => Self::AutoVsync,
            FramePresentation::NoVsync => Self::AutoNoVsync,
            FramePresentation::Mailbox => Self::Mailbox,
        }
    }
}

/// Disposable preferences file; compatibility is intentionally version-bound.
#[derive(Resource, Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct UserPreferences {
    version: u32,
    pub(crate) fullscreen: bool,
    pub(crate) window_width: u32,
    pub(crate) window_height: u32,
    pub(crate) presentation: FramePresentation,
    #[serde(default)]
    pub(crate) ui_scale: hex_ui::UiScaleMode,
    pub(crate) master_volume: f32,
    pub(crate) music_volume: f32,
    pub(crate) effects_volume: f32,
    pub(crate) ui_volume: f32,
    #[serde(default)]
    pub(crate) binding_overrides: InputBindingOverrides,
    #[serde(default)]
    pub(crate) hud_visibility: HudComponentPreferences,
}

impl Default for UserPreferences {
    fn default() -> Self {
        Self {
            version: PREFERENCES_VERSION,
            fullscreen: false,
            window_width: 1280,
            window_height: 720,
            presentation: FramePresentation::Vsync,
            ui_scale: hex_ui::UiScaleMode::Auto,
            master_volume: 1.0,
            music_volume: 0.8,
            effects_volume: 0.8,
            ui_volume: 0.8,
            binding_overrides: InputBindingOverrides::default(),
            hud_visibility: HudComponentPreferences::default(),
        }
    }
}

impl UserPreferences {
    fn upgrade(mut self) -> Result<Self, String> {
        match self.version {
            1 => {
                self.version = PREFERENCES_VERSION;
                self.ui_scale = hex_ui::UiScaleMode::Auto;
                Ok(self)
            }
            2 => {
                self.version = PREFERENCES_VERSION;
                Ok(self)
            }
            PREFERENCES_VERSION => Ok(self),
            version => Err(format!(
                "preferences version {version} is incompatible with {PREFERENCES_VERSION}"
            )),
        }
    }

    fn validate(&self) -> Result<(), String> {
        if self.version != PREFERENCES_VERSION {
            return Err(format!(
                "preferences version {} is incompatible with {}",
                self.version, PREFERENCES_VERSION
            ));
        }
        if !(960..=3840).contains(&self.window_width) || !(540..=2160).contains(&self.window_height)
        {
            return Err("window size is outside the supported pre-alpha range".to_owned());
        }
        for (name, volume) in [
            ("master_volume", self.master_volume),
            ("music_volume", self.music_volume),
            ("effects_volume", self.effects_volume),
            ("ui_volume", self.ui_volume),
        ] {
            if !volume.is_finite() || !(0.0..=1.0).contains(&volume) {
                return Err(format!("{name} must be finite and in 0.0..=1.0"));
            }
        }
        self.binding_overrides
            .validate()
            .map_err(|error| format!("input bindings are invalid: {error}"))?;
        Ok(())
    }
}

/// Runtime projection consumed by a future audio backend.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq)]
pub(crate) struct AudioBusVolumes {
    pub(crate) music: f32,
    pub(crate) effects: f32,
    pub(crate) ui: f32,
}

/// Last persistence result shown on the settings screen.
#[derive(Resource, Debug, Default, Clone)]
pub(crate) struct PreferencesNotice(pub(crate) Option<String>);

/// Explicit user edits awaiting one atomic write.
#[derive(Resource, Debug, Default)]
pub(crate) struct PreferencesDirty(pub(crate) bool);

/// Whether a valid local preference file takes precedence over authored defaults.
#[derive(Resource, Debug, Default)]
struct PreferencesOrigin {
    persisted: bool,
}

pub(crate) fn plugin(app: &mut App) {
    app.init_resource::<StoragePaths>()
        .init_resource::<UserPreferences>()
        .init_resource::<InputBindings>()
        .init_resource::<AudioBusVolumes>()
        .init_resource::<PreferencesNotice>()
        .init_resource::<PreferencesDirty>()
        .init_resource::<PreferencesOrigin>()
        .add_systems(Startup, load_preferences)
        .add_systems(
            Update,
            (
                adopt_authored_presentation,
                apply_preferences,
                persist_changed_preferences,
            )
                .chain(),
        );
}

fn load_preferences(
    paths: Res<StoragePaths>,
    mut preferences: ResMut<UserPreferences>,
    mut notice: ResMut<PreferencesNotice>,
    mut dirty: ResMut<PreferencesDirty>,
    mut origin: ResMut<PreferencesOrigin>,
) {
    match read(&paths.preferences) {
        Ok(text) => match ron::from_str::<UserPreferences>(&text)
            .map_err(|error| error.to_string())
            .and_then(|loaded| {
                let mut loaded = loaded.upgrade()?;
                let repaired_development_binding = loaded
                    .binding_overrides
                    .reconcile_active_development_conflicts()
                    .map_err(|error| format!("input bindings are invalid: {error}"))?;
                loaded.validate()?;
                Ok((loaded, repaired_development_binding))
            }) {
            Ok((loaded, repaired_development_binding)) => {
                *preferences = loaded;
                origin.persisted = true;
                dirty.0 = repaired_development_binding;
            }
            Err(reason) => {
                notice.0 = Some(format!(
                    "Stored settings were incompatible and defaults were restored: {reason}"
                ));
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            notice.0 = Some(format!("Settings could not be read: {error}"));
        }
    }
}

fn adopt_authored_presentation(
    display: Option<Res<DisplaySettings>>,
    origin: Res<PreferencesOrigin>,
    mut preferences: ResMut<UserPreferences>,
) {
    let Some(display) = display else { return };
    if origin.persisted || !display.is_changed() {
        return;
    }
    preferences.presentation = match display.present_mode {
        PresentModeSetting::Vsync => FramePresentation::Vsync,
        PresentModeSetting::NoVsync => FramePresentation::NoVsync,
        PresentModeSetting::Mailbox => FramePresentation::Mailbox,
    };
}

#[expect(
    clippy::cast_precision_loss,
    reason = "validated logical window dimensions are at most 3840×2160 and therefore exactly representable as f32"
)]
fn apply_preferences(
    preferences: Res<UserPreferences>,
    mut buses: ResMut<AudioBusVolumes>,
    mut bindings: ResMut<InputBindings>,
    mut hud: Option<ResMut<hex_gameplay_model::HudState>>,
    mut ui_scale: ResMut<hex_ui::UiScalePreference>,
    mut windows: Query<&mut Window>,
) {
    if !preferences.is_changed() {
        return;
    }
    buses.music = preferences.master_volume * preferences.music_volume;
    buses.effects = preferences.master_volume * preferences.effects_volume;
    buses.ui = preferences.master_volume * preferences.ui_volume;
    *bindings = InputBindings::from_overrides(preferences.binding_overrides.clone());
    if let Some(hud) = hud.as_deref_mut() {
        hud.replace_preferences(preferences.hud_visibility);
    }
    ui_scale.0 = preferences.ui_scale;
    for mut window in &mut windows {
        window.mode = if preferences.fullscreen {
            WindowMode::BorderlessFullscreen(MonitorSelection::Current)
        } else {
            WindowMode::Windowed
        };
        if !preferences.fullscreen {
            window.resolution.set(
                preferences.window_width as f32,
                preferences.window_height as f32,
            );
        }
        window.present_mode = preferences.presentation.into();
    }
}

fn persist_changed_preferences(
    preferences: Res<UserPreferences>,
    paths: Res<StoragePaths>,
    mut dirty: ResMut<PreferencesDirty>,
    mut notice: ResMut<PreferencesNotice>,
    mut origin: ResMut<PreferencesOrigin>,
) {
    if !dirty.0 {
        return;
    }
    dirty.0 = false;
    let serialized =
        match ron::ser::to_string_pretty(preferences.as_ref(), ron::ser::PrettyConfig::new()) {
            Ok(serialized) => serialized,
            Err(error) => {
                notice.0 = Some(format!("Settings could not be encoded: {error}"));
                return;
            }
        };
    match write_atomic(&paths.preferences, &serialized) {
        Ok(()) => {
            origin.persisted = true;
            notice.0 = Some("Settings saved.".to_owned());
        }
        Err(error) => notice.0 = Some(format!("Settings could not be saved: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::input::KeyChord;
    use hex_core::InputAction;

    #[test]
    fn invalid_volumes_and_versions_are_refused() {
        let preferences = UserPreferences {
            master_volume: f32::NAN,
            ..default()
        };
        assert!(preferences.validate().is_err());

        let preferences = UserPreferences {
            version: PREFERENCES_VERSION + 1,
            ..default()
        };
        assert!(preferences.validate().is_err());
    }

    #[test]
    fn version_one_preferences_upgrade_without_losing_values() {
        let text = r#"(
            version: 1,
            fullscreen: true,
            window_width: 1600,
            window_height: 900,
            presentation: Mailbox,
            master_volume: 0.5,
            music_volume: 0.4,
            effects_volume: 0.3,
            ui_volume: 0.2,
        )"#;
        let loaded = ron::from_str::<UserPreferences>(text)
            .expect("v1 shape remains readable")
            .upgrade()
            .expect("v1 upgrades");
        assert_eq!(loaded.version, PREFERENCES_VERSION);
        assert!(loaded.fullscreen);
        assert_eq!(loaded.window_width, 1600);
        assert_eq!(loaded.ui_scale, hex_ui::UiScaleMode::Auto);
        assert!(loaded.binding_overrides.is_empty());
        assert_eq!(loaded.hud_visibility, HudComponentPreferences::default());
        assert!((loaded.master_volume - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn version_two_ui_scale_survives_migration_and_new_fields_default() {
        let text = r#"(
            version: 2,
            fullscreen: false,
            window_width: 1920,
            window_height: 1080,
            presentation: Vsync,
            ui_scale: Percent200,
            master_volume: 0.9,
            music_volume: 0.8,
            effects_volume: 0.7,
            ui_volume: 0.6,
        )"#;
        let restarted = ron::from_str::<UserPreferences>(text)
            .expect("v2 shape remains readable")
            .upgrade()
            .expect("v2 preferences upgrade");
        assert_eq!(restarted.ui_scale, hex_ui::UiScaleMode::Percent200);
        assert_eq!(restarted.version, PREFERENCES_VERSION);
        assert!(restarted.binding_overrides.is_empty());
        assert_eq!(restarted.hud_visibility, HudComponentPreferences::default());
    }

    #[test]
    fn version_three_round_trip_persists_only_binding_overrides_and_hud_preferences() {
        let mut bindings = InputBindings::default();
        bindings
            .assign(InputAction::ToggleParty, KeyChord::plain(KeyCode::KeyY))
            .expect("unused binding is accepted");
        let preferences = UserPreferences {
            binding_overrides: bindings.overrides().clone(),
            hud_visibility: HudComponentPreferences {
                party: false,
                initiative: true,
                activity: true,
                action_bar: false,
            },
            ..default()
        };
        let encoded = ron::ser::to_string(&preferences).expect("v3 preferences encode");
        assert!(encoded.contains("ToggleParty"));
        assert!(
            !encoded.contains("ToggleCamera"),
            "defaults are not persisted"
        );

        let restarted = ron::from_str::<UserPreferences>(&encoded)
            .expect("v3 preferences decode")
            .upgrade()
            .expect("v3 preferences remain current");
        restarted.validate().expect("round trip remains valid");
        assert_eq!(restarted, preferences);
        assert_eq!(
            InputBindings::from_overrides(restarted.binding_overrides)
                .chord(InputAction::ToggleParty),
            KeyChord::plain(KeyCode::KeyY)
        );
    }

    #[test]
    fn hud_visibility_defaults_match_the_minimal_gameplay_contract() {
        assert_eq!(
            HudComponentPreferences::default(),
            HudComponentPreferences {
                party: true,
                initiative: true,
                activity: false,
                action_bar: true,
            }
        );
    }

    #[test]
    fn audio_buses_apply_master_volume() {
        let preferences = UserPreferences {
            master_volume: 0.5,
            music_volume: 0.8,
            effects_volume: 0.6,
            ui_volume: 0.4,
            ..default()
        };
        let buses = AudioBusVolumes {
            music: preferences.master_volume * preferences.music_volume,
            effects: preferences.master_volume * preferences.effects_volume,
            ui: preferences.master_volume * preferences.ui_volume,
        };
        assert!((buses.music - 0.4).abs() < f32::EPSILON);
        assert!((buses.effects - 0.3).abs() < f32::EPSILON);
        assert!((buses.ui - 0.2).abs() < f32::EPSILON);
    }
}
