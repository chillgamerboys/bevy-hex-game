//! Selectable exploration formations loaded from `config/formations.ron`.

use std::collections::BTreeSet;

use bevy::prelude::*;
use hex_core::FormationPreset;
use serde::{de::Error as _, Deserialize, Deserializer};

use crate::{LoadSettings, CONFIG_EXTENSIONS};

/// Validated formation presets in stable authored order.
#[derive(Asset, Resource, Reflect, Debug, Clone, PartialEq, Eq)]
#[reflect(Resource)]
pub struct FormationCatalog {
    /// Presets offered by the party formation UI.
    pub presets: Vec<FormationPreset>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnvalidatedFormationCatalog {
    presets: Vec<FormationPreset>,
}

impl FormationCatalog {
    /// Finds a preset by its stable content name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&FormationPreset> {
        self.presets.iter().find(|preset| preset.name == name)
    }

    /// Validates every preset and unique stable name.
    pub fn validate(&self) -> Result<(), String> {
        if self.presets.is_empty() {
            return Err("formations.ron must define at least one preset".to_owned());
        }
        let mut names = BTreeSet::new();
        for preset in &self.presets {
            preset
                .validate()
                .map_err(|error| format!("formation {:?}: {error:?}", preset.name))?;
            if !names.insert(preset.name.as_str()) {
                return Err(format!(
                    "formations.ron defines {:?} more than once",
                    preset.name
                ));
            }
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for FormationCatalog {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = UnvalidatedFormationCatalog::deserialize(deserializer)?;
        let catalog = Self {
            presets: raw.presets,
        };
        catalog.validate().map_err(D::Error::custom)?;
        Ok(catalog)
    }
}

/// Registers formation content without adding runtime party behavior.
pub fn plugin(app: &mut App) {
    app.register_type::<FormationCatalog>()
        .load_settings::<FormationCatalog>("config/formations.ron", CONFIG_EXTENSIONS);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_shape_parses_and_is_connected() {
        let catalog: FormationCatalog = ron::from_str(
            r#"(
                presets: [(
                    name: "Pair",
                    slots: [
                        (offset: (q: 0, r: 0), anchor: true),
                        (offset: (q: -1, r: 0), anchor: false),
                    ],
                )],
            )"#,
        )
        .expect("valid formation content");
        assert!(catalog.get("Pair").is_some());
    }
}
