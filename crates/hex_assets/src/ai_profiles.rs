//! Data-authored AI profile dispatch loaded from `config/ai_profiles.ron`.

use std::collections::BTreeSet;

use bevy::prelude::*;
use hex_ai::{AiProfile, AiProfileId};
use serde::{de::Error as _, Deserialize, Deserializer};

use crate::{LoadSettings, CONFIG_EXTENSIONS};

/// Validated AI profiles in stable authored order.
#[derive(Asset, Resource, Reflect, Debug, Clone, PartialEq, Eq)]
#[reflect(Resource)]
pub struct AiProfileCatalog {
    /// Profiles available to archetype defaults and encounter overrides.
    pub profiles: Vec<AiProfile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnvalidatedAiProfileCatalog {
    profiles: Vec<AiProfile>,
}

impl AiProfileCatalog {
    /// Finds a profile by stable identity.
    #[must_use]
    pub fn get(&self, id: &AiProfileId) -> Option<&AiProfile> {
        self.profiles.iter().find(|profile| &profile.id == id)
    }

    /// Validates nonempty, unique profile and algorithm identities.
    pub fn validate(&self) -> Result<(), String> {
        if self.profiles.is_empty() {
            return Err("ai_profiles.ron must define at least one profile".to_owned());
        }
        let mut ids = BTreeSet::new();
        for profile in &self.profiles {
            if profile.id.0.trim().is_empty() {
                return Err("an AI profile has an empty id".to_owned());
            }
            if profile.algorithm.0.trim().is_empty() {
                return Err(format!(
                    "AI profile {:?} has an empty algorithm id",
                    profile.id.0
                ));
            }
            if !ids.insert(profile.id.clone()) {
                return Err(format!(
                    "ai_profiles.ron defines {:?} more than once",
                    profile.id.0
                ));
            }
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for AiProfileCatalog {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = UnvalidatedAiProfileCatalog::deserialize(deserializer)?;
        let catalog = Self {
            profiles: raw.profiles,
        };
        catalog.validate().map_err(D::Error::custom)?;
        Ok(catalog)
    }
}

/// Registers profile content without adding runtime algorithm dispatch.
pub fn plugin(app: &mut App) {
    app.register_type::<AiProfileCatalog>()
        .load_settings::<AiProfileCatalog>("config/ai_profiles.ron", CONFIG_EXTENSIONS);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_profile_parses() {
        let catalog: AiProfileCatalog = ron::from_str(
            r#"(
                profiles: [(
                    id: ("baseline"),
                    algorithm: ("baseline-v1"),
                )],
            )"#,
        )
        .expect("valid profile content");
        assert!(catalog.get(&AiProfileId("baseline".to_owned())).is_some());
    }
}
