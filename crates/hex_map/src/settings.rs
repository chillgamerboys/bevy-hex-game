//! Designer-facing map settings, loaded from a world file such as
//! `assets/config/world.ron`.
//!
//! These live here rather than in `hex_assets` so that the map's settings, its
//! generation, and its rendering are all in the crate the map is owned in. Only the
//! *loader* is shared: `hex_assets` handles RON parsing and hot reload for every
//! settings type in the game.
//!
//! **Which file is chosen at runtime, not here.** Each scenario names its own world, so
//! this registers the type as loadable and the binary asks for a path once the player
//! has picked one. `world.ron` is still the world the first scenario uses and still the
//! file to edit while trying terrain out.

use std::collections::BTreeSet;

use bevy::platform::collections::HashSet;
use bevy::prelude::*;
use hex_assets::{RegisterSettings, CONFIG_EXTENSIONS};
use hex_core::{HexCoord, Level};
use serde::de::{Error as _, IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};

pub(crate) const MAX_PROCEDURAL_LEVEL: Level = 128;
const SKY_UPPER_VERTICAL_BUDGET: Level = 8;

/// Registers map settings as loadable from RON.
pub fn plugin(app: &mut App) {
    app.register_type::<MapSettings>();
    app.register_settings::<MapSettings>(CONFIG_EXTENSIONS);
}

/// `assets/config/world.ron`: grid shape and terrain generation.
#[derive(Asset, Resource, Reflect, Debug, Clone, PartialEq)]
#[reflect(Resource)]
pub struct MapSettings {
    /// Tiles from the centre to the edge. The grid holds `3r² + 3r + 1` tiles.
    pub grid_radius: u32,
    /// World height of one voxel level.
    pub level_height: f32,
    /// Terrain preset and its parameters.
    pub terrain: TerrainSettings,
}

impl MapSettings {
    /// Checks the relationships that make a terrain preset constructible.
    ///
    /// Deserialization calls this before a settings asset can replace the active
    /// one. It is also public so tools which construct settings directly can report
    /// the same designer-facing errors before starting a world.
    pub fn validate(&self) -> Result<(), String> {
        if !self.level_height.is_finite() || self.level_height <= 0.0 {
            return Err("level_height must be positive and finite".to_owned());
        }

        match &self.terrain {
            TerrainSettings::Showcase(showcase) => showcase.validate(self.grid_radius),
            TerrainSettings::Perlin(_) => Ok(()),
            TerrainSettings::Procedural(procedural) => procedural.validate(self.grid_radius),
        }
    }
}

#[derive(Deserialize)]
struct UnvalidatedMapSettings {
    grid_radius: u32,
    level_height: f32,
    terrain: TerrainSettings,
}

impl<'de> Deserialize<'de> for MapSettings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = UnvalidatedMapSettings::deserialize(deserializer)?;
        let settings = Self {
            grid_radius: raw.grid_radius,
            level_height: raw.level_height,
            terrain: raw.terrain,
        };
        settings.validate().map_err(D::Error::custom)?;
        Ok(settings)
    }
}

/// Available terrain presets.
#[derive(Reflect, Debug, Clone, PartialEq, Deserialize)]
pub enum TerrainSettings {
    /// The deterministic valley, river, bridge, and mountain test map.
    Showcase(ShowcaseSettings),
    /// The original fractal Perlin terrain.
    Perlin(PerlinSettings),
    /// A versioned, seeded map assembled from landform, environment, and tactics.
    Procedural(ProceduralSettings),
}

/// Versioned settings for the semantic-first procedural generator.
///
/// V1 remains a frozen compatibility contract. V2 gives each topology an honest
/// recipe payload instead of allowing landform/tactical combinations which cannot
/// be generated.
#[derive(Reflect, Debug, Clone, PartialEq)]
pub enum ProceduralSettings {
    /// The frozen landform/environment/tactical generator.
    V1(ProceduralV1Settings),
    /// The volume-based recipe generator.
    V2(ProceduralV2Settings),
}

/// Frozen V1 procedural settings.
///
/// These fields intentionally retain their original types and validation. The
/// custom [`Deserialize`] implementation for [`ProceduralSettings`] preserves the
/// existing flat RON shape rather than requiring a `V1((...))` wrapper.
#[derive(Reflect, Debug, Clone, PartialEq)]
pub struct ProceduralV1Settings {
    /// Large-scale terrain geometry.
    pub landform: LandformSettings,
    /// Surface and hazard materials.
    pub environment: EnvironmentSettings,
    /// Required routes, anchors, and tactical structure.
    pub tactical: TacticalSettings,
}

/// Geometry recipes available to the procedural generator.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Deserialize)]
pub enum LandformSettings {
    /// Connected, moderately elevated terrain on both sides of a valley.
    Hills(HillsSettings),
    /// Floating land masses joined by a required bridge network.
    SkyIslands(SkyIslandsSettings),
}

/// Parameters for connected hills.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct HillsSettings {
    /// Base surface level of the river valley and crossing approaches.
    pub valley_level: Level,
    /// Maximum height above the valley. V1 supports values from one through eight.
    pub max_relief: Level,
    /// Number of hill centres placed on each side of the barrier.
    pub hills_per_bank: u8,
}

/// Parameters for the structural sky-island probe.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SkyIslandsSettings {
    /// Surface level of the critical island chain.
    pub surface_level: Level,
    /// Radius of each required island.
    pub island_radius: u32,
}

/// Material recipes independent of terrain geometry.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum EnvironmentSettings {
    /// Grass, dirt, exposed stone, gravel, and water.
    TemperateGrassland,
    /// Snow and ice over the hills, with water remaining an impassable hazard.
    Frozen,
    /// Basalt terrain separated by non-solid lava.
    Volcanic,
}

/// Tactical topology recipes.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Deserialize)]
pub enum TacticalSettings {
    /// An edge-to-edge hazard with a bridge and a separated alternate crossing.
    Crossing(CrossingSettings),
    /// A critical chain of floating islands plus optional flight-only islands.
    LinkedIslands(LinkedIslandsSettings),
}

/// Parameters for the two-route crossing recipe.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CrossingSettings {
    /// Cells expanded around the centreline. One produces a three-wide barrier.
    pub barrier_half_width: u32,
    /// Surface level of the channel bed.
    pub bed_level: Level,
    /// Lowest occupied hazard voxel.
    pub hazard_bottom: Level,
    /// Highest occupied hazard voxel, inclusive.
    pub hazard_top: Level,
    /// Level occupied by the one-voxel bridge deck.
    pub bridge_level: Level,
}

/// Parameters for the linked-island structural probe.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LinkedIslandsSettings {
    /// Width of every required bridge in cells.
    pub bridge_width: u32,
}

/// Settings shared by every V2 recipe.
#[derive(Reflect, Debug, Clone, PartialEq)]
pub struct ProceduralV2Settings {
    /// Material family applied after the recipe has finalized its geometry.
    pub environment: V2EnvironmentSettings,
    /// Geometry, topology, validation, and repair contract.
    pub recipe: V2RecipeSettings,
}

/// V2 material families.
///
/// This is deliberately separate from [`EnvironmentSettings`] so adding a V2
/// environment can never change the frozen V1 match space or its fingerprints.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum V2EnvironmentSettings {
    /// Grass, dirt, exposed stone, gravel, and water.
    TemperateGrassland,
    /// Snow and ice over stone.
    Frozen,
    /// Basalt separated by non-solid lava.
    Volcanic,
    /// Stone, gravel, and dirt suited to underground spaces.
    Rocky,
}

/// Geometry recipes supported by the V2 volume generator.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Deserialize)]
pub enum V2RecipeSettings {
    /// V1-compatible hills represented by the V2 volume model.
    Hills(V2HillsSettings),
    /// A finalized Hills ground map with flight-gated islands above it.
    LayeredSkyIslands(LayeredSkyIslandsSettings),
    /// A sharp ridge, peaks, a high pass, and a lower bypass.
    Mountains(MountainsSettings),
    /// A playable surface above one underground chamber network.
    Caves(CavesSettings),
}

/// V2 Hills parameters.
///
/// These are the three genuine degrees of freedom from V1 Hills. Hazard width,
/// crossing width, and all crossing levels are derived by
/// [`Self::derived_crossing`] so every deserializable V2 Hills recipe is
/// structurally consistent.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct V2HillsSettings {
    /// Base surface level of the river valley and crossing approaches.
    pub valley_level: Level,
    /// Maximum height above the valley.
    pub max_relief: Level,
    /// Number of hill centres placed on each side of the barrier.
    pub hills_per_bank: u8,
}

/// Derived V2 Hills crossing invariants.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq)]
pub struct DerivedHillsCrossing {
    /// Cells expanded around the centreline. One creates a three-wide hazard.
    pub hazard_half_width: u32,
    /// Width of the bridge and alternate crossing in cells.
    pub crossing_width: u32,
    /// Surface level of the channel bed.
    pub bed_level: Level,
    /// Lowest occupied hazard voxel.
    pub hazard_bottom: Level,
    /// Highest occupied hazard voxel, inclusive.
    pub hazard_top: Level,
    /// Level occupied by the one-voxel bridge deck.
    pub bridge_level: Level,
}

/// Parameters for Hills ground with a separate upper island layer.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayeredSkyIslandsSettings {
    /// The ground recipe, finalized before any sky stream is sampled.
    pub ground: V2HillsSettings,
    /// Completely empty levels required between local ground and an island mass.
    pub min_clearance: Level,
    /// Target percentage of map columns covered by the upper layer.
    pub upper_coverage_percent: u8,
}

/// Parameters for the sharp mountain recipe.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MountainsSettings {
    /// Surface level away from the ridge.
    pub base_level: Level,
    /// Difference between the base and the tallest peak.
    pub relief: Level,
    /// Number of sharp peaks distributed along the main ridge.
    pub peak_count: u8,
}

/// Parameters for the surface-and-underground cave recipe.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CavesSettings {
    /// Typical surface level before modest rocky variation.
    pub surface_level: Level,
    /// Typical floor level of the underground stratum.
    pub cave_floor_level: Level,
    /// Target number of rooted chambers in the main network.
    pub chamber_count: u8,
}

struct ProceduralSettingsWire {
    generator_version: u32,
    landform: Option<LandformSettings>,
    environment: Option<ProceduralEnvironmentWire>,
    tactical: Option<TacticalSettings>,
    recipe: Option<V2RecipeSettings>,
    unknown_fields: BTreeSet<String>,
}

#[derive(Deserialize)]
enum ProceduralEnvironmentWire {
    TemperateGrassland,
    Frozen,
    Volcanic,
    Rocky,
}

enum ProceduralSettingsField {
    GeneratorVersion,
    Landform,
    Environment,
    Tactical,
    Recipe,
    Unknown(String),
}

impl<'de> Deserialize<'de> for ProceduralSettingsField {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct FieldVisitor;

        impl Visitor<'_> for FieldVisitor {
            type Value = ProceduralSettingsField;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a procedural settings field")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(match value {
                    "generator_version" => ProceduralSettingsField::GeneratorVersion,
                    "landform" => ProceduralSettingsField::Landform,
                    "environment" => ProceduralSettingsField::Environment,
                    "tactical" => ProceduralSettingsField::Tactical,
                    "recipe" => ProceduralSettingsField::Recipe,
                    _ => ProceduralSettingsField::Unknown(value.to_owned()),
                })
            }
        }

        deserializer.deserialize_identifier(FieldVisitor)
    }
}

impl<'de> Deserialize<'de> for ProceduralSettingsWire {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct WireVisitor;

        impl<'de> Visitor<'de> for WireVisitor {
            type Value = ProceduralSettingsWire;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("procedural settings")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut generator_version = None;
                let mut landform = None;
                let mut environment = None;
                let mut tactical = None;
                let mut recipe = None;
                let mut unknown_fields = BTreeSet::new();

                while let Some(field) = map.next_key()? {
                    match field {
                        ProceduralSettingsField::GeneratorVersion => {
                            if generator_version.is_some() {
                                return Err(A::Error::duplicate_field("generator_version"));
                            }
                            generator_version = Some(map.next_value()?);
                        }
                        ProceduralSettingsField::Landform => {
                            if landform.is_some() {
                                return Err(A::Error::duplicate_field("landform"));
                            }
                            landform = Some(map.next_value()?);
                        }
                        ProceduralSettingsField::Environment => {
                            if environment.is_some() {
                                return Err(A::Error::duplicate_field("environment"));
                            }
                            environment = Some(map.next_value()?);
                        }
                        ProceduralSettingsField::Tactical => {
                            if tactical.is_some() {
                                return Err(A::Error::duplicate_field("tactical"));
                            }
                            tactical = Some(map.next_value()?);
                        }
                        ProceduralSettingsField::Recipe => {
                            if recipe.is_some() {
                                return Err(A::Error::duplicate_field("recipe"));
                            }
                            recipe = Some(map.next_value()?);
                        }
                        ProceduralSettingsField::Unknown(field) => {
                            map.next_value::<IgnoredAny>()?;
                            unknown_fields.insert(field);
                        }
                    }
                }

                Ok(ProceduralSettingsWire {
                    generator_version: generator_version
                        .ok_or_else(|| A::Error::missing_field("generator_version"))?,
                    landform,
                    environment,
                    tactical,
                    recipe,
                    unknown_fields,
                })
            }
        }

        const FIELDS: &[&str] = &[
            "generator_version",
            "landform",
            "environment",
            "tactical",
            "recipe",
        ];
        deserializer.deserialize_struct("ProceduralSettingsWire", FIELDS, WireVisitor)
    }
}

impl<'de> Deserialize<'de> for ProceduralSettings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ProceduralSettingsWire::deserialize(deserializer)?;
        match wire.generator_version {
            1 => {
                let landform = wire
                    .landform
                    .ok_or_else(|| D::Error::custom("procedural V1 requires the landform field"))?;
                let tactical = wire
                    .tactical
                    .ok_or_else(|| D::Error::custom("procedural V1 requires the tactical field"))?;
                if wire.recipe.is_some() {
                    return Err(D::Error::custom(
                        "procedural V1 does not accept the V2 recipe field",
                    ));
                }
                let environment = match wire.environment.ok_or_else(|| {
                    D::Error::custom("procedural V1 requires the environment field")
                })? {
                    ProceduralEnvironmentWire::TemperateGrassland => {
                        EnvironmentSettings::TemperateGrassland
                    }
                    ProceduralEnvironmentWire::Frozen => EnvironmentSettings::Frozen,
                    ProceduralEnvironmentWire::Volcanic => EnvironmentSettings::Volcanic,
                    ProceduralEnvironmentWire::Rocky => {
                        return Err(D::Error::custom(
                            "procedural V1 does not support the Rocky environment",
                        ));
                    }
                };
                Ok(Self::V1(ProceduralV1Settings {
                    landform,
                    environment,
                    tactical,
                }))
            }
            2 => {
                if let Some(field) = wire.unknown_fields.iter().next() {
                    return Err(D::Error::custom(format!(
                        "procedural V2 contains unknown field {field:?}"
                    )));
                }
                if wire.landform.is_some() || wire.tactical.is_some() {
                    return Err(D::Error::custom(
                        "procedural V2 uses recipe instead of landform and tactical fields",
                    ));
                }
                let recipe = wire
                    .recipe
                    .ok_or_else(|| D::Error::custom("procedural V2 requires the recipe field"))?;
                let environment = match wire.environment.ok_or_else(|| {
                    D::Error::custom("procedural V2 requires the environment field")
                })? {
                    ProceduralEnvironmentWire::TemperateGrassland => {
                        V2EnvironmentSettings::TemperateGrassland
                    }
                    ProceduralEnvironmentWire::Frozen => V2EnvironmentSettings::Frozen,
                    ProceduralEnvironmentWire::Volcanic => V2EnvironmentSettings::Volcanic,
                    ProceduralEnvironmentWire::Rocky => V2EnvironmentSettings::Rocky,
                };
                Ok(Self::V2(ProceduralV2Settings {
                    environment,
                    recipe,
                }))
            }
            version => Err(D::Error::custom(format!(
                "unsupported procedural generator_version {version}; expected 1 or 2"
            ))),
        }
    }
}

impl ProceduralSettings {
    /// Algorithm contract used to reproduce saved seeds.
    #[must_use]
    pub const fn generator_version(&self) -> u32 {
        match self {
            Self::V1(_) => 1,
            Self::V2(_) => 2,
        }
    }

    fn validate(&self, grid_radius: u32) -> Result<(), String> {
        match self {
            Self::V1(settings) => settings.validate(grid_radius),
            Self::V2(settings) => settings.validate(grid_radius),
        }
    }
}

impl ProceduralV1Settings {
    fn validate(&self, grid_radius: u32) -> Result<(), String> {
        match (&self.landform, &self.tactical) {
            (LandformSettings::Hills(hills), TacticalSettings::Crossing(crossing)) => {
                hills.validate(grid_radius)?;
                crossing.validate(hills.valley_level)?;
            }
            (LandformSettings::SkyIslands(islands), TacticalSettings::LinkedIslands(linked)) => {
                islands.validate(grid_radius)?;
                linked.validate()?;
            }
            (LandformSettings::Hills(_), TacticalSettings::LinkedIslands(_)) => {
                return Err("Hills landform requires the Crossing tactical recipe".to_owned());
            }
            (LandformSettings::SkyIslands(_), TacticalSettings::Crossing(_)) => {
                return Err(
                    "SkyIslands landform requires the LinkedIslands tactical recipe".to_owned(),
                );
            }
        }
        Ok(())
    }
}

impl ProceduralV2Settings {
    fn validate(&self, grid_radius: u32) -> Result<(), String> {
        if !(12..=40).contains(&grid_radius) {
            return Err("procedural V2 requires grid_radius from 12 through 40".to_owned());
        }

        match (&self.recipe, self.environment) {
            (
                V2RecipeSettings::Hills(hills),
                V2EnvironmentSettings::TemperateGrassland
                | V2EnvironmentSettings::Frozen
                | V2EnvironmentSettings::Volcanic,
            ) => hills.validate(grid_radius),
            (V2RecipeSettings::Hills(_), V2EnvironmentSettings::Rocky) => {
                Err("V2 Hills does not support the Rocky environment".to_owned())
            }
            (
                V2RecipeSettings::LayeredSkyIslands(islands),
                V2EnvironmentSettings::TemperateGrassland | V2EnvironmentSettings::Frozen,
            ) => islands.validate(grid_radius),
            (
                V2RecipeSettings::LayeredSkyIslands(_),
                V2EnvironmentSettings::Volcanic | V2EnvironmentSettings::Rocky,
            ) => Err("V2 LayeredSkyIslands requires TemperateGrassland or Frozen".to_owned()),
            (V2RecipeSettings::Mountains(mountains), V2EnvironmentSettings::Frozen) => {
                mountains.validate(grid_radius)
            }
            (V2RecipeSettings::Mountains(_), _) => {
                Err("V2 Mountains requires the Frozen environment".to_owned())
            }
            (V2RecipeSettings::Caves(caves), V2EnvironmentSettings::Rocky) => {
                caves.validate(grid_radius)
            }
            (V2RecipeSettings::Caves(_), _) => {
                Err("V2 Caves requires the Rocky environment".to_owned())
            }
        }
    }
}

impl V2HillsSettings {
    /// Derives invariants shared by the river, bridge, and alternate crossing.
    pub fn derived_crossing(&self) -> Result<DerivedHillsCrossing, String> {
        let bed_level = self
            .valley_level
            .checked_sub(3)
            .ok_or_else(|| "V2 Hills valley_level is too low for its hazard".to_owned())?;
        let hazard_bottom = bed_level
            .checked_add(1)
            .ok_or_else(|| "V2 Hills crossing level relationship overflows Level".to_owned())?;
        let hazard_top = bed_level
            .checked_add(2)
            .ok_or_else(|| "V2 Hills crossing level relationship overflows Level".to_owned())?;
        let bridge_level = self
            .valley_level
            .checked_add(1)
            .ok_or_else(|| "V2 Hills bridge level relationship overflows Level".to_owned())?;

        Ok(DerivedHillsCrossing {
            hazard_half_width: 1,
            crossing_width: 2,
            bed_level,
            hazard_bottom,
            hazard_top,
            bridge_level,
        })
    }

    fn validate(&self, grid_radius: u32) -> Result<(), String> {
        if !(12..=40).contains(&grid_radius) {
            return Err("procedural V2 Hills requires grid_radius from 12 through 40".to_owned());
        }
        if self.valley_level < 5 {
            return Err("V2 Hills valley_level must leave room for bedrock and strata".to_owned());
        }
        if !(1..=8).contains(&self.max_relief) {
            return Err("V2 Hills max_relief must be between 1 and 8".to_owned());
        }
        let Some(highest_surface) = self.valley_level.checked_add(self.max_relief) else {
            return Err("V2 Hills level relationship overflows Level".to_owned());
        };
        if highest_surface > MAX_PROCEDURAL_LEVEL {
            return Err(format!(
                "V2 Hills surfaces cannot exceed level {MAX_PROCEDURAL_LEVEL}"
            ));
        }
        if !(1..=6).contains(&self.hills_per_bank) {
            return Err("V2 Hills hills_per_bank must be between 1 and 6".to_owned());
        }
        self.derived_crossing()?;
        Ok(())
    }
}

impl LayeredSkyIslandsSettings {
    fn validate(&self, grid_radius: u32) -> Result<(), String> {
        self.ground.validate(grid_radius)?;
        if self.min_clearance < 8 {
            return Err("LayeredSkyIslands min_clearance must be at least 8".to_owned());
        }
        if !(15..=25).contains(&self.upper_coverage_percent) {
            return Err(
                "LayeredSkyIslands upper_coverage_percent must be between 15 and 25".to_owned(),
            );
        }

        let highest_ground = self
            .ground
            .valley_level
            .checked_add(self.ground.max_relief)
            .ok_or_else(|| "LayeredSkyIslands ground relationship overflows Level".to_owned())?;
        let highest_reserved = highest_ground
            .checked_add(self.min_clearance)
            .and_then(|level| level.checked_add(SKY_UPPER_VERTICAL_BUDGET))
            .ok_or_else(|| "LayeredSkyIslands level relationship overflows Level".to_owned())?;
        if highest_reserved > MAX_PROCEDURAL_LEVEL {
            return Err(format!(
                "LayeredSkyIslands reserved volume cannot exceed level {MAX_PROCEDURAL_LEVEL}"
            ));
        }
        Ok(())
    }
}

impl MountainsSettings {
    fn validate(&self, grid_radius: u32) -> Result<(), String> {
        if !(12..=40).contains(&grid_radius) {
            return Err(
                "procedural V2 Mountains requires grid_radius from 12 through 40".to_owned(),
            );
        }
        if self.base_level < 5 {
            return Err("Mountains base_level must leave room for bedrock and strata".to_owned());
        }
        if !(14..=16).contains(&self.relief) {
            return Err("Mountains relief must be between 14 and 16".to_owned());
        }
        if !(3..=5).contains(&self.peak_count) {
            return Err("Mountains peak_count must be between 3 and 5".to_owned());
        }
        let Some(highest_surface) = self.base_level.checked_add(self.relief) else {
            return Err("Mountains level relationship overflows Level".to_owned());
        };
        if highest_surface > MAX_PROCEDURAL_LEVEL {
            return Err(format!(
                "Mountains surfaces cannot exceed level {MAX_PROCEDURAL_LEVEL}"
            ));
        }
        Ok(())
    }
}

impl CavesSettings {
    fn validate(&self, grid_radius: u32) -> Result<(), String> {
        if !(12..=40).contains(&grid_radius) {
            return Err("procedural V2 Caves requires grid_radius from 12 through 40".to_owned());
        }
        if !(14..=17).contains(&self.surface_level) {
            return Err("Caves surface_level must be between 14 and 17".to_owned());
        }
        if !(6..=8).contains(&self.cave_floor_level) {
            return Err("Caves cave_floor_level must be between 6 and 8".to_owned());
        }
        if !(6..=8).contains(&self.chamber_count) {
            return Err("Caves chamber_count must be between 6 and 8".to_owned());
        }
        let Some(vertical_space) = self.surface_level.checked_sub(self.cave_floor_level) else {
            return Err("Caves cave_floor_level must be below the surface".to_owned());
        };
        if vertical_space < 7 {
            return Err(
                "Caves need four clear chamber levels below at least three roof levels".to_owned(),
            );
        }
        Ok(())
    }
}

impl HillsSettings {
    fn validate(&self, grid_radius: u32) -> Result<(), String> {
        if !(12..=40).contains(&grid_radius) {
            return Err("procedural Hills requires grid_radius from 12 through 40".to_owned());
        }
        if self.valley_level < 5 {
            return Err("Hills valley_level must leave room for bedrock and strata".to_owned());
        }
        if !(1..=8).contains(&self.max_relief) {
            return Err("Hills max_relief must be between 1 and 8".to_owned());
        }
        let Some(highest_surface) = self.valley_level.checked_add(self.max_relief) else {
            return Err("Hills level relationship overflows Level".to_owned());
        };
        if highest_surface > MAX_PROCEDURAL_LEVEL {
            return Err(format!(
                "Hills surfaces cannot exceed level {MAX_PROCEDURAL_LEVEL}"
            ));
        }
        if !(1..=6).contains(&self.hills_per_bank) {
            return Err("Hills hills_per_bank must be between 1 and 6".to_owned());
        }
        Ok(())
    }
}

impl SkyIslandsSettings {
    fn validate(&self, grid_radius: u32) -> Result<(), String> {
        if !(12..=40).contains(&grid_radius) {
            return Err("procedural SkyIslands requires grid_radius from 12 through 40".to_owned());
        }
        if self.surface_level < 8 {
            return Err("SkyIslands surface_level must be at least 8".to_owned());
        }
        let Some(highest_surface) = self.surface_level.checked_add(4) else {
            return Err("SkyIslands level relationship overflows Level".to_owned());
        };
        if highest_surface > MAX_PROCEDURAL_LEVEL {
            return Err(format!(
                "SkyIslands surfaces cannot exceed level {MAX_PROCEDURAL_LEVEL}"
            ));
        }
        if !(2..=4).contains(&self.island_radius) {
            return Err("SkyIslands island_radius must be between 2 and 4".to_owned());
        }
        Ok(())
    }
}

impl CrossingSettings {
    fn validate(&self, valley_level: Level) -> Result<(), String> {
        if self.barrier_half_width != 1 {
            return Err("procedural Crossing barrier_half_width must be 1".to_owned());
        }
        if self.bed_level < 1 {
            return Err("Crossing bed_level must remain above bedrock".to_owned());
        }
        if self.bed_level > MAX_PROCEDURAL_LEVEL {
            return Err(format!(
                "Crossing levels cannot exceed {MAX_PROCEDURAL_LEVEL}"
            ));
        }
        let Some(expected_hazard_bottom) = self.bed_level.checked_add(1) else {
            return Err("Crossing bed relationship overflows Level".to_owned());
        };
        if self.hazard_bottom != expected_hazard_bottom {
            return Err("Crossing hazard_bottom must sit directly above its bed".to_owned());
        }
        if self.hazard_top < self.hazard_bottom
            || self.hazard_top >= valley_level
            || self.hazard_top > MAX_PROCEDURAL_LEVEL
        {
            return Err("Crossing hazard levels must lie between the bed and valley".to_owned());
        }
        let Some(expected_bridge_level) = valley_level.checked_add(1) else {
            return Err("Crossing bridge relationship overflows Level".to_owned());
        };
        if self.bridge_level != expected_bridge_level || self.bridge_level > MAX_PROCEDURAL_LEVEL {
            return Err("Crossing bridge_level must be exactly one above the valley".to_owned());
        }
        Ok(())
    }
}

impl LinkedIslandsSettings {
    fn validate(&self) -> Result<(), String> {
        if self.bridge_width != 2 {
            return Err("LinkedIslands bridge_width must be 2".to_owned());
        }
        Ok(())
    }
}

/// Perlin terrain configuration.
#[derive(Reflect, Debug, Clone, PartialEq, Deserialize)]
pub struct PerlinSettings {
    /// Fixed seed for reproducible worlds, or `None` to randomise per launch.
    pub seed: Option<u64>,
    /// Octaves of noise, summed.
    pub steps: Vec<PerlinStepSettings>,
}

/// One octave of Perlin noise.
#[derive(Reflect, Debug, Clone, PartialEq, Deserialize)]
pub struct PerlinStepSettings {
    /// Noise frequency along x. Higher is bumpier.
    pub x_freq: f32,
    /// Noise frequency along y. Higher is bumpier.
    pub y_freq: f32,
    /// How much height this octave contributes.
    pub magnitude: f32,
}

/// A cube coordinate as written in map RON: `(x: 0, y: 0, z: 0)`.
///
/// The three axes must sum to zero. Validation converts this designer-facing
/// representation into the engine's axial [`HexCoord`].
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct CubeCoord {
    /// East-west axis.
    pub x: i32,
    /// North-east to south-west axis.
    pub y: i32,
    /// North-west to south-east axis. Always `-x - y`.
    pub z: i32,
}

/// Parameters for the deterministic showcase terrain.
#[derive(Reflect, Debug, Clone, PartialEq, Deserialize)]
pub struct ShowcaseSettings {
    /// Surface level of the low ground beside the river.
    pub valley_level: Level,
    /// Highest surface level on the gently terraced side.
    pub gentle_max_level: Level,
    /// Number of hexes travelled before the gentle side rises another level.
    pub gentle_terrace_width: u32,
    /// River course and depth.
    pub river: RiverSettings,
    /// Metal crossing over the river.
    pub bridge: BridgeSettings,
    /// Steep peak on the far side of the valley.
    pub mountain: MountainSettings,
    /// Ordered, adjacent coordinates of the climbable mountain trail.
    pub switchback: Vec<CubeCoord>,
}

/// River course and vertical profile.
#[derive(Reflect, Debug, Clone, PartialEq, Deserialize)]
pub struct RiverSettings {
    /// Waypoints joined into the centre line of the river.
    pub path: Vec<CubeCoord>,
    /// Hexes added on either side of the centre line. One makes a three-wide river.
    pub half_width: u32,
    /// Level of the gravel riverbed.
    pub bed_level: Level,
    /// Lowest water voxel level.
    pub water_bottom: Level,
    /// Highest water voxel level, inclusive.
    pub water_top: Level,
}

/// Straight, two-lane bridge parameters.
#[derive(Reflect, Debug, Clone, PartialEq, Deserialize)]
pub struct BridgeSettings {
    /// Axial x coordinates of the bridge lanes.
    pub lane_xs: Vec<i32>,
    /// Inclusive axial y coordinate of the mountain-side landing.
    pub y_min: i32,
    /// Inclusive axial y coordinate of the gentle-side landing.
    pub y_max: i32,
    /// Level occupied by the one-voxel-thick deck.
    pub deck_level: Level,
}

/// Mountain shape parameters.
#[derive(Reflect, Debug, Clone, PartialEq, Deserialize)]
pub struct MountainSettings {
    /// Centre and highest coordinate of the peak.
    pub summit: CubeCoord,
    /// Valley floor used outside the cone.
    pub base_level: Level,
    /// Surface level at the summit.
    pub peak_level: Level,
    /// Levels lost for every hex travelled away from the summit.
    pub falloff_per_hex: Level,
    /// Voxel levels at and above this elevation are exposed stone.
    pub exposed_stone_level: Level,
}

impl ShowcaseSettings {
    fn validate(&self, grid_radius: u32) -> Result<(), String> {
        if grid_radius < 12 {
            return Err("Showcase terrain requires grid_radius of at least 12".to_owned());
        }
        if self.valley_level < 4 {
            return Err("Showcase valley_level must leave room for bedrock and strata".to_owned());
        }
        if self.gentle_max_level < self.valley_level {
            return Err("gentle_max_level cannot be below valley_level".to_owned());
        }
        if self.gentle_terrace_width == 0 {
            return Err("gentle_terrace_width must be positive".to_owned());
        }

        self.validate_river(grid_radius)?;
        self.validate_bridge(grid_radius)?;
        self.validate_mountain(grid_radius)?;
        self.validate_switchback(grid_radius)?;
        Ok(())
    }

    fn validate_river(&self, grid_radius: u32) -> Result<(), String> {
        if self.river.path.len() < 2 {
            return Err("river.path needs at least two waypoints".to_owned());
        }
        if self.river.half_width != 1 {
            return Err("Showcase river.half_width must be 1".to_owned());
        }
        for (index, coord) in self.river.path.iter().copied().enumerate() {
            checked_coord(coord, grid_radius, &format!("river.path[{index}]"))?;
        }

        let Some(first) = self.river.path.first().copied() else {
            return Err("river.path needs a first waypoint".to_owned());
        };
        let Some(last) = self.river.path.last().copied() else {
            return Err("river.path needs a last waypoint".to_owned());
        };
        if cube_distance(first) != u64::from(grid_radius)
            || cube_distance(last) != u64::from(grid_radius)
        {
            return Err("river.path must start and end on the map boundary".to_owned());
        }

        if self.river.bed_level < 1 {
            return Err("river bed must remain above bedrock".to_owned());
        }
        let expected_water_bottom = self
            .river
            .bed_level
            .checked_add(1)
            .ok_or_else(|| "river bed level is too high".to_owned())?;
        if self.river.water_bottom != expected_water_bottom {
            return Err("water_bottom must sit directly above the riverbed".to_owned());
        }
        if self.river.water_top < self.river.water_bottom {
            return Err("water_top cannot be below water_bottom".to_owned());
        }
        if self.river.water_top >= self.valley_level {
            return Err("river water must remain below the valley banks".to_owned());
        }
        Ok(())
    }

    fn validate_bridge(&self, grid_radius: u32) -> Result<(), String> {
        if self.bridge.lane_xs.len() != 2 {
            return Err("bridge.lane_xs must contain exactly two lanes".to_owned());
        }
        let Some(first_x) = self.bridge.lane_xs.first().copied() else {
            return Err("bridge needs a first lane".to_owned());
        };
        let Some(second_x) = self.bridge.lane_xs.get(1).copied() else {
            return Err("bridge needs a second lane".to_owned());
        };
        if first_x.abs_diff(second_x) != 1 {
            return Err("bridge lanes must be adjacent".to_owned());
        }
        if self.bridge.y_min >= self.bridge.y_max {
            return Err("bridge y_min must be below y_max".to_owned());
        }
        let expected_deck_level = self
            .valley_level
            .checked_add(1)
            .ok_or_else(|| "valley level is too high for a bridge".to_owned())?;
        if self.bridge.deck_level != expected_deck_level {
            return Err("bridge deck must be one level above the valley".to_owned());
        }
        let expected_deck_above_water = self
            .river
            .water_top
            .checked_add(2)
            .ok_or_else(|| "river water level is too high for a bridge".to_owned())?;
        if self.bridge.deck_level != expected_deck_above_water {
            return Err("bridge deck must leave one air level above the water".to_owned());
        }

        for x in &self.bridge.lane_xs {
            checked_axial(*x, self.bridge.y_min, grid_radius, "bridge deck")?;
            checked_axial(*x, self.bridge.y_max, grid_radius, "bridge deck")?;
        }
        Ok(())
    }

    fn validate_mountain(&self, grid_radius: u32) -> Result<(), String> {
        checked_coord(self.mountain.summit, grid_radius, "mountain.summit")?;
        if self.mountain.base_level != self.valley_level {
            return Err("mountain.base_level must match valley_level".to_owned());
        }
        if self.mountain.peak_level <= self.mountain.base_level {
            return Err("mountain.peak_level must be above its base".to_owned());
        }
        if self.mountain.falloff_per_hex <= 0 {
            return Err("mountain.falloff_per_hex must be positive".to_owned());
        }
        if !(self.mountain.base_level..=self.mountain.peak_level)
            .contains(&self.mountain.exposed_stone_level)
        {
            return Err("mountain.exposed_stone_level must lie between base and peak".to_owned());
        }
        Ok(())
    }

    fn validate_switchback(&self, grid_radius: u32) -> Result<(), String> {
        if self.switchback.len() < 2 {
            return Err("switchback needs at least two coordinates".to_owned());
        }

        let mut previous: Option<HexCoord> = None;
        let mut seen: HashSet<HexCoord> = HashSet::default();
        for (index, raw) in self.switchback.iter().copied().enumerate() {
            let coord = checked_coord(raw, grid_radius, &format!("switchback[{index}]"))?;
            if !seen.insert(coord) {
                return Err(format!("switchback[{index}] repeats an earlier coordinate"));
            }
            if previous.is_some_and(|last| last.distance(coord) != 1) {
                return Err(format!(
                    "switchback[{index}] is not adjacent to the previous coordinate"
                ));
            }
            previous = Some(coord);
        }

        let Some(first) = self.switchback.first().copied() else {
            return Err("switchback needs a first coordinate".to_owned());
        };
        if first.y != self.bridge.y_min || !self.bridge.lane_xs.contains(&first.x) {
            return Err("switchback must begin at the mountain-side bridge landing".to_owned());
        }
        if self.switchback.last().copied() != Some(self.mountain.summit) {
            return Err("switchback must finish at mountain.summit".to_owned());
        }

        let rise = self.mountain.peak_level - self.bridge.deck_level;
        let transitions =
            Level::try_from(self.switchback.len().saturating_sub(1)).unwrap_or(Level::MAX);
        if transitions < rise {
            return Err(
                "switchback is too short to climb from the bridge to the peak one level at a time"
                    .to_owned(),
            );
        }
        Ok(())
    }
}

fn checked_coord(raw: CubeCoord, radius: u32, label: &str) -> Result<HexCoord, String> {
    let sum = i64::from(raw.x) + i64::from(raw.y) + i64::from(raw.z);
    if sum != 0 {
        return Err(format!(
            "{label} ({}, {}, {}) must sum to zero",
            raw.x, raw.y, raw.z
        ));
    }
    if cube_distance(raw) > u64::from(radius) {
        return Err(format!("{label} lies outside grid_radius {radius}"));
    }
    Ok(HexCoord::from_axial(raw.x, raw.y))
}

fn checked_axial(x: i32, y: i32, radius: u32, label: &str) -> Result<(), String> {
    let z = -(i64::from(x) + i64::from(y));
    let distance = i64::from(x)
        .unsigned_abs()
        .max(i64::from(y).unsigned_abs())
        .max(z.unsigned_abs());
    if distance > u64::from(radius) {
        return Err(format!(
            "{label} coordinate ({x}, {y}, {z}) lies outside grid_radius {radius}"
        ));
    }
    Ok(())
}

fn cube_distance(coord: CubeCoord) -> u64 {
    i64::from(coord.x)
        .unsigned_abs()
        .max(i64::from(coord.y).unsigned_abs())
        .max(i64::from(coord.z).unsigned_abs())
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORLD_RON: &str = include_str!("../../../assets/config/world.ron");

    fn showcase_settings() -> MapSettings {
        ron::from_str(WORLD_RON).expect("the shipped world settings should parse")
    }

    #[test]
    fn shipped_settings_select_the_showcase() {
        let settings = showcase_settings();
        assert_eq!(settings.grid_radius, 12);
        let TerrainSettings::Showcase(showcase) = settings.terrain else {
            panic!("the shipped preset should be Showcase")
        };
        assert_eq!(showcase.valley_level, 15);
        assert_eq!(showcase.mountain.peak_level, 30);
        assert_eq!(showcase.switchback.len(), 24);
    }

    #[test]
    fn perlin_variant_remains_deserializable() {
        let ron = r#"
            (
                grid_radius: 4,
                level_height: 0.4,
                terrain: Perlin((
                    seed: Some(7),
                    steps: [
                        (x_freq: 0.035, y_freq: 0.05, magnitude: 3.0),
                    ],
                )),
            )
        "#;
        let settings: MapSettings = ron::from_str(ron).expect("Perlin RON should remain valid");
        let TerrainSettings::Perlin(perlin) = settings.terrain else {
            panic!("the parsed preset should be Perlin")
        };
        assert_eq!(perlin.seed, Some(7));
        assert_eq!(perlin.steps.len(), 1);
    }

    #[test]
    fn procedural_variants_remain_deserializable() {
        for ron in [
            include_str!("../../../assets/config/worlds/procedural-hills.ron"),
            include_str!("../../../assets/config/worlds/procedural-frozen.ron"),
            include_str!("../../../assets/config/worlds/procedural-volcanic.ron"),
            include_str!("../../../assets/config/worlds/procedural-sky-islands.ron"),
        ] {
            let settings: MapSettings =
                ron::from_str(ron).expect("shipped procedural RON should parse");
            let TerrainSettings::Procedural(procedural) = settings.terrain else {
                panic!("the shipped preset should be Procedural")
            };
            assert_eq!(procedural.generator_version(), 1);
            assert!(matches!(procedural, ProceduralSettings::V1(_)));
        }
    }

    #[test]
    fn v1_keeps_its_flat_external_ron_shape() {
        let source = include_str!("../../../assets/config/worlds/procedural-hills.ron");
        let settings: MapSettings =
            ron::from_str(source).expect("the original flat V1 RON should remain valid");
        let TerrainSettings::Procedural(ProceduralSettings::V1(v1)) = settings.terrain else {
            panic!("generator_version 1 should dispatch to the internal V1 variant")
        };
        assert_eq!(
            v1.landform,
            LandformSettings::Hills(HillsSettings {
                valley_level: 15,
                max_relief: 8,
                hills_per_bank: 3,
            })
        );
        assert_eq!(v1.environment, EnvironmentSettings::TemperateGrassland);

        let wrapped = source.replacen("Procedural((", "Procedural(V1((", 1);
        let wrapped = wrapped.replacen("\n    )),\n)", "\n    ))),\n)", 1);
        ron::from_str::<MapSettings>(&wrapped)
            .expect_err("the internal V1 variant must not leak into external RON");
    }

    #[test]
    fn v1_keeps_legacy_unknown_field_tolerance() {
        let source = include_str!("../../../assets/config/worlds/procedural-hills.ron").replacen(
            "generator_version: 1,",
            "generator_version: 1,\n        legacy_extension: 42,",
            1,
        );
        ron::from_str::<MapSettings>(&source)
            .expect("manual version dispatch must not tighten the frozen V1 wire contract");
    }

    #[test]
    fn v2_rejects_unknown_top_level_fields() {
        let source = r#"
            (
                grid_radius: 12,
                level_height: 0.4,
                terrain: Procedural((
                    generator_version: 2,
                    environment: TemperateGrassland,
                    recipe: Hills((
                        valley_level: 15,
                        max_relief: 8,
                        hills_per_bank: 3,
                    )),
                    typoed_setting: 42,
                )),
            )
        "#;
        let error =
            ron::from_str::<MapSettings>(source).expect_err("V2 top-level fields must be rejected");
        assert!(
            error.to_string().contains("unknown field")
                && error.to_string().contains("typoed_setting"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn unsupported_version_is_reported_without_parsing_a_recipe_shape() {
        let source = r#"
            (
                grid_radius: 12,
                level_height: 0.4,
                terrain: Procedural((
                    generator_version: 99,
                )),
            )
        "#;
        let error = ron::from_str::<MapSettings>(source)
            .expect_err("an unknown generator version must fail during wire dispatch");
        assert!(
            error.to_string().contains("expected 1 or 2"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn v2_recipes_dispatch_from_generator_version() {
        for (recipe, environment) in [
            (
                "Hills((valley_level: 15, max_relief: 8, hills_per_bank: 3))",
                "TemperateGrassland",
            ),
            (
                "LayeredSkyIslands((ground: (valley_level: 15, max_relief: 8, hills_per_bank: 3), min_clearance: 8, upper_coverage_percent: 20))",
                "TemperateGrassland",
            ),
            (
                "Mountains((base_level: 15, relief: 15, peak_count: 4))",
                "Frozen",
            ),
            (
                "Caves((surface_level: 15, cave_floor_level: 8, chamber_count: 7))",
                "Rocky",
            ),
        ] {
            let source = format!(
                "(
                    grid_radius: 12,
                    level_height: 0.4,
                    terrain: Procedural((
                        generator_version: 2,
                        environment: {environment},
                        recipe: {recipe},
                    )),
                )"
            );
            let settings: MapSettings =
                ron::from_str(&source).expect("a valid V2 recipe should deserialize");
            let TerrainSettings::Procedural(procedural) = settings.terrain else {
                panic!("the parsed preset should be Procedural")
            };
            assert_eq!(procedural.generator_version(), 2);
            assert!(matches!(procedural, ProceduralSettings::V2(_)));
        }
    }

    #[test]
    fn v2_hills_derives_fixed_crossing_invariants() {
        let hills = V2HillsSettings {
            valley_level: 15,
            max_relief: 8,
            hills_per_bank: 3,
        };
        assert_eq!(
            hills
                .derived_crossing()
                .expect("a valid valley should derive its crossing"),
            DerivedHillsCrossing {
                hazard_half_width: 1,
                crossing_width: 2,
                bed_level: 12,
                hazard_bottom: 13,
                hazard_top: 14,
                bridge_level: 16,
            }
        );
    }

    #[test]
    fn version_specific_fields_cannot_be_mixed() {
        let v1_with_recipe = include_str!("../../../assets/config/worlds/procedural-hills.ron")
            .replacen(
                "environment: TemperateGrassland,",
                "environment: TemperateGrassland,\n        recipe: Hills((valley_level: 15, max_relief: 8, hills_per_bank: 3)),",
                1,
            );
        let v1_error = ron::from_str::<MapSettings>(&v1_with_recipe)
            .expect_err("V1 must reject a V2 recipe field");
        assert!(
            v1_error
                .to_string()
                .contains("does not accept the V2 recipe"),
            "unexpected error: {v1_error}"
        );

        let v2_with_legacy_axes = r#"
            (
                grid_radius: 12,
                level_height: 0.4,
                terrain: Procedural((
                    generator_version: 2,
                    landform: Hills((
                        valley_level: 15,
                        max_relief: 8,
                        hills_per_bank: 3,
                    )),
                    environment: TemperateGrassland,
                    tactical: Crossing((
                        barrier_half_width: 1,
                        bed_level: 12,
                        hazard_bottom: 13,
                        hazard_top: 14,
                        bridge_level: 16,
                    )),
                    recipe: Hills((
                        valley_level: 15,
                        max_relief: 8,
                        hills_per_bank: 3,
                    )),
                )),
            )
        "#;
        let v2_error = ron::from_str::<MapSettings>(v2_with_legacy_axes)
            .expect_err("V2 must reject V1 landform and tactical fields");
        assert!(
            v2_error.to_string().contains("uses recipe instead"),
            "unexpected error: {v2_error}"
        );
    }

    #[test]
    fn invalid_v2_environment_recipe_combinations_are_rejected() {
        for (recipe, environment, expected) in [
            (
                "Hills((valley_level: 15, max_relief: 8, hills_per_bank: 3))",
                "Rocky",
                "does not support",
            ),
            (
                "LayeredSkyIslands((ground: (valley_level: 15, max_relief: 8, hills_per_bank: 3), min_clearance: 8, upper_coverage_percent: 20))",
                "Volcanic",
                "requires TemperateGrassland or Frozen",
            ),
            (
                "Mountains((base_level: 15, relief: 15, peak_count: 4))",
                "TemperateGrassland",
                "requires the Frozen",
            ),
            (
                "Caves((surface_level: 15, cave_floor_level: 8, chamber_count: 7))",
                "Frozen",
                "requires the Rocky",
            ),
        ] {
            let source = format!(
                "(
                    grid_radius: 12,
                    level_height: 0.4,
                    terrain: Procedural((
                        generator_version: 2,
                        environment: {environment},
                        recipe: {recipe},
                    )),
                )"
            );
            let error = ron::from_str::<MapSettings>(&source)
                .expect_err("an unsupported recipe/environment pair should fail");
            assert!(
                error.to_string().contains(expected),
                "unexpected error for {recipe}: {error}"
            );
        }
    }

    #[test]
    fn v2_recipe_fields_and_vertical_contracts_are_strict() {
        let unknown = r#"
            (
                grid_radius: 12,
                level_height: 0.4,
                terrain: Procedural((
                    generator_version: 2,
                    environment: TemperateGrassland,
                    recipe: Hills((
                        valley_level: 15,
                        max_relief: 8,
                        hills_per_bank: 3,
                        barrier_half_width: 1,
                    )),
                )),
            )
        "#;
        ron::from_str::<MapSettings>(unknown)
            .expect_err("derived V2 invariants must not be accepted as settings");

        let shallow_cave = r#"
            (
                grid_radius: 12,
                level_height: 0.4,
                terrain: Procedural((
                    generator_version: 2,
                    environment: Rocky,
                    recipe: Caves((
                        surface_level: 14,
                        cave_floor_level: 8,
                        chamber_count: 7,
                    )),
                )),
            )
        "#;
        let error = ron::from_str::<MapSettings>(shallow_cave)
            .expect_err("caves must reserve chamber clearance and three roof levels");
        assert!(
            error.to_string().contains("four clear chamber levels"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn procedural_radius_is_bounded_for_v1() {
        let source = include_str!("../../../assets/config/worlds/procedural-hills.ron");
        for invalid_radius in [11, 41] {
            let invalid = source.replacen(
                "grid_radius: 12",
                &format!("grid_radius: {invalid_radius}"),
                1,
            );
            let error = ron::from_str::<MapSettings>(&invalid)
                .expect_err("v1 procedural radius outside 12 through 40 should fail");
            assert!(
                error.to_string().contains("12 through 40"),
                "unexpected error: {error}"
            );
        }
    }

    #[test]
    fn unsupported_landform_tactical_combinations_are_rejected() {
        let source = include_str!("../../../assets/config/worlds/procedural-hills.ron");
        let invalid = source.replacen(
            "tactical: Crossing((\n            barrier_half_width: 1,\n            bed_level: 12,\n            hazard_bottom: 13,\n            hazard_top: 14,\n            bridge_level: 16,\n        ))",
            "tactical: LinkedIslands((bridge_width: 2))",
            1,
        );
        let error = ron::from_str::<MapSettings>(&invalid)
            .expect_err("Hills with LinkedIslands should fail at deserialization");
        assert!(
            error.to_string().contains("requires the Crossing"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn procedural_levels_are_bounded_before_voxel_allocation() {
        let hills = include_str!("../../../assets/config/worlds/procedural-hills.ron")
            .replacen("valley_level: 15", "valley_level: 125", 1)
            .replacen("bed_level: 12", "bed_level: 122", 1)
            .replacen("hazard_bottom: 13", "hazard_bottom: 123", 1)
            .replacen("hazard_top: 14", "hazard_top: 124", 1)
            .replacen("bridge_level: 16", "bridge_level: 126", 1);
        let hills_error = ron::from_str::<MapSettings>(&hills)
            .expect_err("terrain above the v1 allocation ceiling should fail");
        assert!(
            hills_error.to_string().contains("cannot exceed level 128"),
            "unexpected error: {hills_error}"
        );

        let sky = include_str!("../../../assets/config/worlds/procedural-sky-islands.ron")
            .replacen("surface_level: 15", "surface_level: 125", 1);
        let sky_error = ron::from_str::<MapSettings>(&sky)
            .expect_err("sky islands must reserve bounded space for optional relief");
        assert!(
            sky_error.to_string().contains("cannot exceed level 128"),
            "unexpected error: {sky_error}"
        );
    }

    #[test]
    fn invalid_showcase_radius_is_rejected_during_deserialization() {
        let invalid = WORLD_RON.replacen("grid_radius: 12", "grid_radius: 11", 1);
        let error =
            ron::from_str::<MapSettings>(&invalid).expect_err("radius 11 should be rejected");
        assert!(
            error.to_string().contains("at least 12"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn invalid_cube_coordinate_is_rejected() {
        let mut settings = showcase_settings();
        let TerrainSettings::Showcase(showcase) = &mut settings.terrain else {
            panic!("test settings should use Showcase")
        };
        let Some(first) = showcase.river.path.first_mut() else {
            panic!("the river should have a first waypoint")
        };
        first.z += 1;

        let error = settings
            .validate()
            .expect_err("cube coordinates which do not sum to zero must fail");
        assert!(
            error.contains("must sum to zero"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn non_contiguous_switchback_is_rejected() {
        let mut settings = showcase_settings();
        let TerrainSettings::Showcase(showcase) = &mut settings.terrain else {
            panic!("test settings should use Showcase")
        };
        let Some(second) = showcase.switchback.get_mut(1) else {
            panic!("the switchback should have a second coordinate")
        };
        *second = CubeCoord { x: -5, y: -3, z: 8 };

        let error = settings
            .validate()
            .expect_err("a disconnected switchback must fail");
        assert!(error.contains("not adjacent"), "unexpected error: {error}");
    }

    #[test]
    fn switchback_must_be_long_enough_for_the_peak() {
        let mut settings = showcase_settings();
        let TerrainSettings::Showcase(showcase) = &mut settings.terrain else {
            panic!("test settings should use Showcase")
        };
        showcase.mountain.peak_level = 50;

        let error = settings
            .validate()
            .expect_err("a trail which would need multi-level steps must fail");
        assert!(error.contains("too short"), "unexpected error: {error}");
    }

    #[test]
    fn inconsistent_river_depth_is_rejected() {
        let mut settings = showcase_settings();
        let TerrainSettings::Showcase(showcase) = &mut settings.terrain else {
            panic!("test settings should use Showcase")
        };
        showcase.river.water_bottom += 1;

        let error = settings
            .validate()
            .expect_err("water separated from its bed must fail");
        assert!(
            error.contains("directly above"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn overflowing_level_relationships_are_rejected() {
        let mut settings = showcase_settings();
        let TerrainSettings::Showcase(showcase) = &mut settings.terrain else {
            panic!("test settings should use Showcase")
        };
        showcase.river.bed_level = Level::MAX;

        let error = settings
            .validate()
            .expect_err("overflowing level relationships must fail");
        assert!(error.contains("too high"), "unexpected error: {error}");
    }

    #[test]
    fn showcase_river_width_is_bounded() {
        let mut settings = showcase_settings();
        let TerrainSettings::Showcase(showcase) = &mut settings.terrain else {
            panic!("test settings should use Showcase")
        };
        showcase.river.half_width = u32::MAX;

        let error = settings
            .validate()
            .expect_err("a huge river expansion must fail before map construction");
        assert!(error.contains("must be 1"), "unexpected error: {error}");
    }
}
