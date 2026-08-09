//! Designer-facing sight settings loaded from `assets/config/perception.ron`.
//!
//! The authoritative perception systems consume [`hex_core::SightProfile`]. This
//! module keeps the RON representation explicit and independently validates every
//! named playtest preset before converting it to that shared runtime contract.

use bevy::prelude::*;
use hex_core::{SightBand, SightProfile};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer};

/// `assets/config/perception.ron` — gameplay sight radii.
#[derive(Asset, Resource, Reflect, Debug, Clone, PartialEq, Eq)]
#[reflect(Resource)]
pub struct PerceptionSettings {
    /// Named playtest preset selected for this run.
    pub active: SightPreset,
    /// Broad sight intended for large-map tactical review.
    pub expansive: SightRanges,
    /// Intermediate sight range used for comparison captures.
    pub focused: SightRanges,
    /// Restrained sight range used for comparison captures.
    pub tight: SightRanges,
}

impl PerceptionSettings {
    /// Returns the selected runtime sight profile.
    #[must_use]
    pub fn active_profile(&self) -> SightProfile {
        self.profile(self.active)
    }

    /// Resolves any named preset into the shared runtime sight contract.
    #[must_use]
    pub fn profile(&self, preset: SightPreset) -> SightProfile {
        let ranges = match preset {
            SightPreset::Expansive => self.expansive,
            SightPreset::Focused => self.focused,
            SightPreset::Tight => self.tight,
        };
        SightProfile {
            bright: ranges.bright.into(),
            dim: ranges.dim.into(),
            dark: ranges.dark.into(),
        }
    }

    /// Rejects sight settings that would make illumination tiers contradictory.
    pub fn validate(&self) -> Result<(), String> {
        for (name, ranges) in [
            ("expansive", self.expansive),
            ("focused", self.focused),
            ("tight", self.tight),
        ] {
            ranges.validate(name)?;
        }
        self.expansive
            .validate_covers("expansive", self.focused, "focused")?;
        self.focused
            .validate_covers("focused", self.tight, "tight")?;
        Ok(())
    }
}

impl Default for PerceptionSettings {
    /// The shipped `perception.ron` values, for headless tests that do not load assets.
    fn default() -> Self {
        Self {
            active: SightPreset::Expansive,
            expansive: SightRanges::new(36, 12),
            focused: SightRanges::new(24, 8),
            tight: SightRanges::new(18, 6),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnvalidatedPerceptionSettings {
    active: SightPreset,
    expansive: SightRanges,
    focused: SightRanges,
    tight: SightRanges,
}

impl<'de> Deserialize<'de> for PerceptionSettings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = UnvalidatedPerceptionSettings::deserialize(deserializer)?;
        let settings = Self {
            active: raw.active,
            expansive: raw.expansive,
            focused: raw.focused,
            tight: raw.tight,
        };
        settings.validate().map_err(D::Error::custom)?;
        Ok(settings)
    }
}

/// Named sight-range presets available for playtest comparison.
#[derive(Reflect, Deserialize, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum SightPreset {
    /// Bright 36, dim 12, dark 1.
    #[default]
    Expansive,
    /// Bright 24, dim 8, dark 1.
    Focused,
    /// Bright 18, dim 6, dark 1.
    Tight,
}

/// Upper-dome radii for every illumination tier.
#[derive(Reflect, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SightRanges {
    /// Limits under direct sunlight or equally strong local light.
    pub bright: SightBandSettings,
    /// Limits under moonlight or equally weak local light.
    pub dim: SightBandSettings,
    /// Radius-one awareness in absolute darkness.
    pub dark: SightBandSettings,
}

impl SightRanges {
    const fn new(bright: u32, dim: u32) -> Self {
        Self {
            bright: SightBandSettings::new(bright),
            dim: SightBandSettings::new(dim),
            dark: SightBandSettings::new(1),
        }
    }

    fn validate(self, name: &str) -> Result<(), String> {
        for (tier, band) in [
            ("bright", self.bright),
            ("dim", self.dim),
            ("dark", self.dark),
        ] {
            if band.radius == 0 {
                return Err(format!(
                    "perception.ron: {name}.{tier}.radius must be at least 1"
                ));
            }
        }
        if self.dark != SightBandSettings::new(1) {
            return Err(format!(
                "perception.ron: {name}.dark.radius must be exactly 1"
            ));
        }
        if self.bright.radius < self.dim.radius || self.dim.radius < self.dark.radius {
            return Err(format!(
                "perception.ron: {name} sight radius must satisfy bright >= dim >= dark"
            ));
        }
        Ok(())
    }

    fn validate_covers(
        self,
        name: &str,
        narrower: Self,
        narrower_name: &str,
    ) -> Result<(), String> {
        for (tier, broader, narrower) in [
            ("bright", self.bright, narrower.bright),
            ("dim", self.dim, narrower.dim),
            ("dark", self.dark, narrower.dark),
        ] {
            if broader.radius < narrower.radius {
                return Err(format!(
                    "perception.ron: {name}.{tier}.radius must be at least \
                     {narrower_name}.{tier}.radius"
                ));
            }
        }
        Ok(())
    }
}

/// Authoring representation of one upper-dome sight radius.
#[derive(Reflect, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SightBandSettings {
    /// Inclusive radius in hex/voxel units.
    pub radius: u32,
}

impl SightBandSettings {
    /// Creates one authoring sight band.
    #[must_use]
    pub const fn new(radius: u32) -> Self {
        Self { radius }
    }
}

impl From<SightBandSettings> for SightBand {
    fn from(value: SightBandSettings) -> Self {
        Self::new(value.radius)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    use bevy::asset::{AssetLoadFailedEvent, AssetPlugin};
    use hex_test_app::HeadlessAppBuilder;

    use crate::loader::LoadSettings;

    use super::*;

    const PERCEPTION_RON: &str = include_str!("../../../assets/config/perception.ron");

    #[test]
    fn shipped_settings_define_all_three_profiles() {
        let settings: PerceptionSettings =
            ron::from_str(PERCEPTION_RON).expect("the shipped perception settings should parse");

        assert_eq!(settings, PerceptionSettings::default());
        assert_eq!(settings.active, SightPreset::Expansive);
        assert_eq!(
            settings.profile(SightPreset::Expansive),
            SightProfile::DEFAULT
        );
        assert_eq!(
            settings.profile(SightPreset::Focused),
            SightProfile {
                bright: SightBand::new(24),
                dim: SightBand::new(8),
                dark: SightBand::new(1),
            }
        );
        assert_eq!(
            settings.profile(SightPreset::Tight),
            SightProfile {
                bright: SightBand::new(18),
                dim: SightBand::new(6),
                dark: SightBand::new(1),
            }
        );
    }

    #[test]
    fn active_preset_selects_the_runtime_profile() {
        for (name, expected) in [("Expansive", 36), ("Focused", 24), ("Tight", 18)] {
            let authored =
                PERCEPTION_RON.replacen("active: Expansive", &format!("active: {name}"), 1);
            let settings: PerceptionSettings =
                ron::from_str(&authored).expect("every named preset should parse");
            assert_eq!(settings.active_profile().bright.radius, expected);
        }
    }

    #[test]
    fn invalid_profiles_are_rejected_during_deserialization() {
        for (needle, replacement, expected) in [
            (
                "bright: (radius: 36)",
                "bright: (radius: 0)",
                "expansive.bright.radius",
            ),
            (
                "dark: (radius: 1)",
                "dark: (radius: 2)",
                "expansive.dark.radius",
            ),
            (
                "bright: (radius: 24)",
                "bright: (radius: 37)",
                "expansive.bright.radius",
            ),
            ("dim: (radius: 6)", "dim: (radius: 9)", "focused.dim.radius"),
        ] {
            let invalid = PERCEPTION_RON.replacen(needle, replacement, 1);
            assert_ne!(
                invalid, PERCEPTION_RON,
                "the fixture no longer contains {needle:?}"
            );
            let error = ron::from_str::<PerceptionSettings>(&invalid)
                .expect_err("invalid sight settings should fail deserialization");
            assert!(
                error.to_string().contains(expected),
                "{replacement:?} returned an unrelated error: {error}"
            );
        }
    }

    #[test]
    fn unknown_fields_are_rejected_at_every_schema_level() {
        for (needle, replacement, unknown) in [
            (
                "active: Expansive,",
                "active: Expansive,\n    activee: Focused,",
                "activee",
            ),
            (
                "expansive: (",
                "expansive: (\n        distance: 36,",
                "distance",
            ),
            (
                "bright: (radius: 36)",
                "bright: (radius: 36, diagonal: 36)",
                "diagonal",
            ),
        ] {
            let invalid = PERCEPTION_RON.replacen(needle, replacement, 1);
            let error = ron::from_str::<PerceptionSettings>(&invalid)
                .expect_err("unknown fields must fail loudly");
            assert!(
                error.to_string().contains(unknown),
                "unknown field {unknown:?} returned an unrelated error: {error}"
            );
        }
    }

    #[derive(Resource, Default)]
    struct SawPerceptionLoadFailure(bool);

    fn record_perception_load_failure(
        mut failures: MessageReader<AssetLoadFailedEvent<PerceptionSettings>>,
        mut saw_failure: ResMut<SawPerceptionLoadFailure>,
    ) {
        if failures.read().next().is_some() {
            saw_failure.0 = true;
        }
    }

    fn update_until(app: &mut App, mut predicate: impl FnMut(&World) -> bool) -> bool {
        for _ in 0..600 {
            app.update();
            if predicate(app.world()) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        false
    }

    static TEMP_ASSET_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TempAssetRoot(PathBuf);

    impl TempAssetRoot {
        fn new() -> std::io::Result<Self> {
            let sequence = TEMP_ASSET_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "hex-assets-perception-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir_all(&path)?;
            Ok(Self(path))
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempAssetRoot {
        fn drop(&mut self) {
            if let Err(error) = fs::remove_dir_all(&self.0) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    warn!(
                        "could not remove test asset directory {:?}: {error}",
                        self.0
                    );
                }
            }
        }
    }

    #[test]
    fn invalid_hot_reload_keeps_previous_perception_settings_and_recovers() {
        let root = TempAssetRoot::new().expect("the temporary asset directory should be created");
        let perception_path = root.path().join("perception.ron");
        fs::write(&perception_path, PERCEPTION_RON)
            .expect("the valid perception fixture should be written");

        let mut builder = HeadlessAppBuilder::new()
            .with_minimal_plugins()
            .with_asset_plugin_config(AssetPlugin {
                file_path: root.path().to_string_lossy().into_owned(),
                ..default()
            });
        builder
            .app_mut()
            .load_settings::<PerceptionSettings>("perception.ron", &["ron"]);
        builder
            .app_mut()
            .init_resource::<SawPerceptionLoadFailure>();
        builder
            .app_mut()
            .add_systems(Update, record_perception_load_failure);
        let mut app = builder.build();

        assert!(
            update_until(&mut app, |world| world
                .contains_resource::<PerceptionSettings>()),
            "the valid perception fixture did not load"
        );
        let previous = app.world().resource::<PerceptionSettings>().clone();

        let invalid = PERCEPTION_RON.replacen("bright: (radius: 36)", "bright: (radius: 0)", 1);
        fs::write(&perception_path, invalid)
            .expect("the invalid perception edit should be written");
        app.world()
            .resource::<AssetServer>()
            .reload("perception.ron");

        assert!(
            update_until(&mut app, |world| {
                world.resource::<SawPerceptionLoadFailure>().0
            }),
            "the invalid reload did not report an asset failure"
        );
        assert_eq!(
            app.world().resource::<PerceptionSettings>(),
            &previous,
            "an invalid reload replaced the last valid resource"
        );

        let recovered = PERCEPTION_RON.replacen("active: Expansive", "active: Focused", 1);
        fs::write(&perception_path, recovered)
            .expect("the corrected perception edit should be written");
        app.world()
            .resource::<AssetServer>()
            .reload("perception.ron");

        assert!(
            update_until(&mut app, |world| {
                world.resource::<PerceptionSettings>().active == SightPreset::Focused
            }),
            "a valid edit after an invalid reload did not replace the resource"
        );
    }
}
