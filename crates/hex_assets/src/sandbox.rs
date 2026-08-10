//! Versioned Sandbox map content with hidden actor-staging compatibility metadata.

use std::collections::BTreeSet;

use bevy::prelude::*;
use serde::{de::Error as _, Deserialize, Deserializer};

use crate::{CubeCoord, LoadSettings, CONFIG_EXTENSIONS};

/// Current on-disk schema for `config/sandbox_maps.ron`.
pub const SANDBOX_MAP_SCHEMA_VERSION: u32 = 2;

/// Curated maps offered by Sandbox.
#[derive(Asset, Resource, Reflect, Debug, Clone, PartialEq, Eq)]
#[reflect(Resource)]
pub struct SandboxMapCatalog {
    /// Versioned independently from the user's creation library.
    pub schema_version: u32,
    /// Stable authored display order.
    pub maps: Vec<SandboxMapDefinition>,
}

/// One fixed-seed map and the two compatibility regions used to stage hidden actors.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxMapDefinition {
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
    /// Hidden staging region for human-controlled units; manual placement ignores it.
    pub player_region: SandboxDeploymentRegion,
    /// Hidden staging region for baseline-AI units; manual placement ignores it.
    pub hostile_region: SandboxDeploymentRegion,
}

/// A bounded hidden actor-staging region resolved after terrain exists.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxDeploymentRegion {
    /// Authored coordinate or map-generated anchor at the center.
    pub center: SandboxRegionCenter,
    /// Footing path-cost radius from the resolved center.
    pub radius: u32,
}

/// Ways packaged content may resolve the center of an actor-staging region.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Deserialize)]
pub enum SandboxRegionCenter {
    /// Exact authored horizontal coordinate; terrain resolves its top surface.
    Fixed(CubeCoord),
    /// Named map anchor that already includes exact elevation.
    Anchor(String),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnvalidatedCatalog {
    schema_version: u32,
    maps: Vec<SandboxMapDefinition>,
}

impl SandboxMapCatalog {
    /// Finds a map by stable machine identifier.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&SandboxMapDefinition> {
        self.maps.iter().find(|map| map.id == id)
    }

    /// Validates stable IDs, schema, centers, and bounded staging radii.
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != SANDBOX_MAP_SCHEMA_VERSION {
            return Err(format!(
                "unsupported Sandbox map schema {}; expected {}",
                self.schema_version, SANDBOX_MAP_SCHEMA_VERSION
            ));
        }
        if self.maps.is_empty() {
            return Err("Sandbox map catalog must contain at least one map".to_owned());
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
                    "Sandbox map IDs, names, descriptions, previews, and scenarios cannot be blank"
                        .to_owned(),
                );
            }
            if map.tags.is_empty() || map.tags.iter().any(|tag| tag.trim().is_empty()) {
                return Err(format!(
                    "Sandbox map {:?} needs at least one non-blank tag",
                    map.id
                ));
            }
            if !ids.insert(map.id.as_str()) {
                return Err(format!("duplicate Sandbox map ID {:?}", map.id));
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
                if let SandboxRegionCenter::Fixed(coord) = &region.center {
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

impl<'de> Deserialize<'de> for SandboxMapCatalog {
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

/// Registers packaged Sandbox map content.
pub fn plugin(app: &mut App) {
    app.register_type::<SandboxMapCatalog>()
        .load_settings::<SandboxMapCatalog>("config/sandbox_maps.ron", CONFIG_EXTENSIONS);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_catalog_parses_and_has_stable_maps() {
        let catalog: SandboxMapCatalog =
            ron::from_str(include_str!("../../../assets/config/sandbox_maps.ron"))
                .expect("shipped Sandbox maps parse");
        assert_eq!(catalog.schema_version, SANDBOX_MAP_SCHEMA_VERSION);
        assert!(catalog.get("flat-arena").is_some());
        assert!(catalog.get("the-crossing").is_some());
        assert!(catalog.get("procedural-hills").is_some());
        assert!(catalog.get("forest").is_some());
        let prairie = catalog
            .get("prairie")
            .expect("Prairie should remain selectable in Sandbox");
        assert_eq!(prairie.scenario, "Prairie");
        assert_eq!(prairie.fixed_seed, Some(1_592_598_566));
        assert_eq!(prairie.preview, "ui/sandbox/prairie.png");
        let deep_forest = catalog
            .get("deep-forest")
            .expect("Deep Forest should remain selectable in Sandbox");
        assert_eq!(deep_forest.scenario, "Deep Forest");
        assert_eq!(deep_forest.fixed_seed, Some(1_592_598_566));
        assert_eq!(deep_forest.preview, "ui/sandbox/deep-forest.png");
        assert!(catalog.get("fort").is_some());
        let crystal_ascent = catalog
            .get("crystal-ascent")
            .expect("Crystal Ascent should be selectable in Sandbox");
        assert_eq!(crystal_ascent.scenario, "Crystal Ascent");
        assert_eq!(crystal_ascent.fixed_seed, Some(1_592_598_566));
        assert_eq!(crystal_ascent.preview, "ui/sandbox/crystal-ascent.png");
        assert_eq!(
            crystal_ascent.player_region.center,
            SandboxRegionCenter::Anchor("crystal_ascent.lower_entry".to_owned())
        );
        assert_eq!(
            crystal_ascent.hostile_region.center,
            SandboxRegionCenter::Anchor("crystal_ascent.upper_exit".to_owned())
        );
        assert!(catalog.get("seven-regions").is_some());
        let two_rings = catalog
            .get("two-rings")
            .expect("Two Rings should be selectable in Sandbox");
        assert_eq!(two_rings.scenario, "Two Rings");
        assert_eq!(two_rings.fixed_seed, Some(1_592_598_566));
        assert_eq!(two_rings.preview, "ui/sandbox/two-rings.png");
        let mountain_range = catalog
            .get("mountain-range")
            .expect("Mountain Range should be selectable in Sandbox");
        assert_eq!(mountain_range.scenario, "Mountain Range");
        assert_eq!(mountain_range.fixed_seed, Some(129_704_046));
        assert_eq!(mountain_range.preview, "ui/sandbox/mountain-range.png");
        assert_eq!(catalog.maps.len(), 18);

        let scenarios: crate::ScenarioLibrary =
            ron::from_str(include_str!("../../../assets/config/scenarios.ron"))
                .expect("shipped scenarios parse");
        for map in &catalog.maps {
            assert!(
                scenarios
                    .scenarios
                    .iter()
                    .any(|scenario| scenario.name == map.scenario),
                "Sandbox map {:?} names an unavailable scenario {:?}",
                map.id,
                map.scenario
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
