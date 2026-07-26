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

use bevy::platform::collections::HashSet;
use bevy::prelude::*;
use hex_assets::{RegisterSettings, CONFIG_EXTENSIONS};
use hex_core::{HexCoord, Level};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer};

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
