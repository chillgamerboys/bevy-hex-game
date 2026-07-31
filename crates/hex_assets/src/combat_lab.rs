//! Versioned map and deployment-region content for the human Combat Lab.

use std::collections::BTreeSet;

use bevy::prelude::*;
use serde::{de::Error as _, Deserialize, Deserializer};

use crate::{CubeCoord, LoadSettings, CONFIG_EXTENSIONS};

/// Current on-disk schema for `config/combat_lab_maps.ron`.
pub const COMBAT_LAB_MAP_SCHEMA_VERSION: u32 = 2;

/// Curated maps offered by the transient Combat Lab Sandbox.
#[derive(Asset, Resource, Reflect, Debug, Clone, PartialEq, Eq)]
#[reflect(Resource)]
pub struct CombatLabMapCatalog {
    /// Versioned independently from the user's creation library.
    pub schema_version: u32,
    /// Stable authored display order.
    pub maps: Vec<CombatLabMapDefinition>,
}

/// One fixed-seed map and the two regions in which rosters may deploy.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CombatLabMapDefinition {
    /// Stable machine identifier stored in launch data and tests.
    pub id: String,
    /// Player-facing map name.
    pub display_name: String,
    /// Concise tactical description shown beside the renderer-generated preview.
    pub description: String,
    /// Mechanic and terrain labels used to scan the map shelf.
    pub tags: Vec<String>,
    /// Asset path to a deterministic preview captured from the shipped renderer.
    pub preview: String,
    /// Stable scenario name whose world and lighting are loaded.
    pub scenario: String,
    /// Exact seed used when the selected scenario is generated.
    pub fixed_seed: Option<u64>,
    /// Legal region for human-controlled units.
    pub player_region: CombatLabDeploymentRegion,
    /// Legal region for baseline-AI units.
    pub hostile_region: CombatLabDeploymentRegion,
}

/// A bounded legal surface region resolved after terrain exists.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CombatLabDeploymentRegion {
    /// Authored coordinate or map-generated anchor at the center.
    pub center: CombatLabRegionCenter,
    /// Footing path-cost radius from the resolved center.
    pub radius: u32,
}

/// Ways packaged content may resolve the center of a deployment region.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Deserialize)]
pub enum CombatLabRegionCenter {
    /// Exact authored horizontal coordinate; terrain resolves its top surface.
    Fixed(CubeCoord),
    /// Named map anchor that already includes exact elevation.
    Anchor(String),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnvalidatedCatalog {
    schema_version: u32,
    maps: Vec<CombatLabMapDefinition>,
}

impl CombatLabMapCatalog {
    /// Finds a map by stable machine identifier.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&CombatLabMapDefinition> {
        self.maps.iter().find(|map| map.id == id)
    }

    /// Validates stable IDs, schema, centers, and bounded deployment radii.
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != COMBAT_LAB_MAP_SCHEMA_VERSION {
            return Err(format!(
                "unsupported Combat Lab map schema {}; expected {}",
                self.schema_version, COMBAT_LAB_MAP_SCHEMA_VERSION
            ));
        }
        if self.maps.is_empty() {
            return Err("Combat Lab map catalog must contain at least one map".to_owned());
        }
        let mut ids = BTreeSet::new();
        for map in &self.maps {
            if map.id.trim().is_empty()
                || map.display_name.trim().is_empty()
                || map.description.trim().is_empty()
                || map.scenario.trim().is_empty()
                || map.preview.trim().is_empty()
            {
                return Err(
                    "Combat Lab map IDs, names, descriptions, previews, and scenarios cannot be blank"
                        .to_owned(),
                );
            }
            if map.tags.is_empty() || map.tags.iter().any(|tag| tag.trim().is_empty()) {
                return Err(format!(
                    "Combat Lab map {:?} needs at least one non-blank tag",
                    map.id
                ));
            }
            if !ids.insert(map.id.as_str()) {
                return Err(format!("duplicate Combat Lab map ID {:?}", map.id));
            }
            for (side, region) in [
                ("Player", &map.player_region),
                ("Hostile", &map.hostile_region),
            ] {
                if !(1..=12).contains(&region.radius) {
                    return Err(format!(
                        "{} {side} deployment radius {} must be in 1..=12",
                        map.id, region.radius
                    ));
                }
                if let CombatLabRegionCenter::Fixed(coord) = &region.center {
                    if coord.x + coord.y + coord.z != 0 {
                        return Err(format!(
                            "{} {side} deployment coordinate must sum to zero",
                            map.id
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for CombatLabMapCatalog {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = UnvalidatedCatalog::deserialize(deserializer)?;
        let catalog = Self {
            schema_version: raw.schema_version,
            maps: raw.maps,
        };
        catalog.validate().map_err(D::Error::custom)?;
        Ok(catalog)
    }
}

/// Registers packaged Combat Lab map content.
pub fn plugin(app: &mut App) {
    app.register_type::<CombatLabMapCatalog>()
        .load_settings::<CombatLabMapCatalog>("config/combat_lab_maps.ron", CONFIG_EXTENSIONS);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_catalog_parses_and_has_stable_maps() {
        let catalog: CombatLabMapCatalog =
            ron::from_str(include_str!("../../../assets/config/combat_lab_maps.ron"))
                .expect("shipped Combat Lab maps parse");
        assert_eq!(catalog.schema_version, COMBAT_LAB_MAP_SCHEMA_VERSION);
        assert!(catalog.get("flat-arena").is_some());
        assert!(catalog.get("the-crossing").is_some());
        assert!(catalog.get("procedural-hills").is_some());
        assert!(catalog.get("forest").is_some());
        let prairie = catalog
            .get("prairie")
            .expect("Prairie should remain selectable in Combat Lab");
        assert_eq!(prairie.scenario, "Prairie");
        assert_eq!(prairie.fixed_seed, Some(1_592_598_566));
        assert_eq!(prairie.preview, "ui/combat-lab/prairie.png");
        let deep_forest = catalog
            .get("deep-forest")
            .expect("Deep Forest should remain selectable in Combat Lab");
        assert_eq!(deep_forest.scenario, "Deep Forest");
        assert_eq!(deep_forest.fixed_seed, Some(1_592_598_566));
        assert_eq!(deep_forest.preview, "ui/combat-lab/deep-forest.png");
        assert!(catalog.get("fort").is_some());
        assert!(catalog.get("seven-regions").is_some());
        let two_rings = catalog
            .get("two-rings")
            .expect("Two Rings should be selectable in Combat Lab");
        assert_eq!(two_rings.scenario, "Two Rings");
        assert_eq!(two_rings.fixed_seed, Some(1_592_598_566));
        assert_eq!(two_rings.preview, "ui/combat-lab/two-rings.png");
        assert_eq!(catalog.maps.len(), 16);

        let scenarios: crate::ScenarioLibrary =
            ron::from_str(include_str!("../../../assets/config/scenarios.ron"))
                .expect("shipped scenarios parse");
        for scenario in scenarios
            .scenarios
            .iter()
            .filter(|scenario| scenario.category == crate::ScenarioCategory::Map)
        {
            assert!(
                catalog.maps.iter().any(|map| map.scenario == scenario.name),
                "Map scenario {:?} is missing from Combat Lab",
                scenario.name
            );
        }

        let asset_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
        for map in &catalog.maps {
            assert!(
                asset_root.join(&map.preview).is_file(),
                "preview for {:?} does not exist at {:?}",
                map.id,
                map.preview
            );
        }
    }
}
