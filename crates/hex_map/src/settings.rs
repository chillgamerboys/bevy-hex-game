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

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use bevy::platform::collections::HashSet;
use bevy::prelude::*;
use hex_assets::{RegisterSettings, CONFIG_EXTENSIONS};
use hex_core::{HexCoord, Level};
use serde::de::{Error as _, IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};

pub(crate) const MAX_PROCEDURAL_LEVEL: Level = 128;
const SKY_UPPER_VERTICAL_BUDGET: Level = 20;
pub(crate) const MAX_WALKER_PORT_COUNT: u8 = 4;
pub(crate) const MAX_SEAM_PORT_WIDTH: u32 = 4;
/// Exact region count of the V3 two-rings composite.
pub const V3_RING19_REGION_COUNT: usize = 19;
const RING19_RADIUS: u32 = 55;
const RING19_RECIPE_VALIDATION_RADIUS: u32 = 40;
const TWO_RINGS_REGIONS: [(V3EnvironmentSettings, &str, u8); V3_RING19_REGION_COUNT] = [
    (V3EnvironmentSettings::TemperateGrassland, "Hills", 0),
    (V3EnvironmentSettings::Frozen, "Hills", 0),
    (V3EnvironmentSettings::TemperateGrassland, "Forest", 4),
    (V3EnvironmentSettings::TemperateGrassland, "Prairie", 0),
    (V3EnvironmentSettings::TemperateGrassland, "Hills", 0),
    (V3EnvironmentSettings::TemperateGrassland, "Waterfall", 0),
    (V3EnvironmentSettings::TemperateGrassland, "Waterfall", 5),
    (V3EnvironmentSettings::TemperateGrassland, "SkyIslands", 0),
    (V3EnvironmentSettings::TemperateGrassland, "DeepForest", 0),
    (V3EnvironmentSettings::TemperateGrassland, "DeepForest", 0),
    (V3EnvironmentSettings::TemperateGrassland, "Forest", 4),
    (V3EnvironmentSettings::TemperateGrassland, "Prairie", 0),
    (V3EnvironmentSettings::TemperateGrassland, "Waterfall", 5),
    (V3EnvironmentSettings::TemperateGrassland, "Fort", 0),
    (V3EnvironmentSettings::Rocky, "Caves", 0),
    (V3EnvironmentSettings::Volcanic, "Volcano", 3),
    (V3EnvironmentSettings::Frozen, "Mountains", 0),
    (V3EnvironmentSettings::Frozen, "Mountains", 0),
    (V3EnvironmentSettings::Frozen, "Mountains", 0),
];
const TWO_RINGS_INTERNAL_HYDROLOGY: [(u8, u8, u32, Level); 8] = [
    (16, 5, 3, 29),
    (5, 0, 3, 16),
    (17, 6, 3, 29),
    (6, 0, 3, 16),
    (18, 1, 3, 16),
    (1, 0, 3, 16),
    (0, 4, 3, 16),
    (4, 12, 3, 16),
];
const TWO_RINGS_BOUNDARY_HYDROLOGY: [(u8, Ring19BoundarySide, u32, Level); 2] = [
    (12, Ring19BoundarySide::SouthEast, 3, 3),
    (15, Ring19BoundarySide::West, 3, 14),
];
const CUBE_NEIGHBORS: [(i32, i32, i32); 6] = [
    (1, -1, 0),
    (1, 0, -1),
    (0, 1, -1),
    (-1, 1, 0),
    (-1, 0, 1),
    (0, -1, 1),
];

/// Orders a bounded one-dimensional seam, rejecting branches, cycles, and gaps.
///
/// Settings validation and resolved-layout validation deliberately share this
/// predicate so an authored mask cannot pass one boundary and fail the other.
pub(crate) fn ordered_simple_seam_lanes(
    lanes: &BTreeSet<(HexCoord, HexCoord)>,
) -> Option<Vec<(HexCoord, HexCoord)>> {
    if lanes.is_empty() {
        return None;
    }
    let by_first: BTreeMap<_, _> = lanes.iter().copied().collect();
    if by_first.len() != lanes.len()
        || lanes
            .iter()
            .map(|(_, second)| *second)
            .collect::<BTreeSet<_>>()
            .len()
            != lanes.len()
    {
        return None;
    }
    if lanes.len() == 1 {
        return Some(lanes.iter().copied().collect());
    }

    let neighbors = |first: HexCoord| {
        first
            .neighbors()
            .into_iter()
            .filter(|candidate| {
                by_first.get(candidate).is_some_and(|candidate_second| {
                    by_first
                        .get(&first)
                        .is_some_and(|second| second.distance(*candidate_second) == 1)
                })
            })
            .collect::<Vec<_>>()
    };
    let endpoints: Vec<_> = by_first
        .keys()
        .copied()
        .filter(|first| neighbors(*first).len() == 1)
        .collect();
    if endpoints.len() != 2
        || by_first
            .keys()
            .any(|first| !matches!(neighbors(*first).len(), 1 | 2))
    {
        return None;
    }

    let mut ordered = Vec::with_capacity(lanes.len());
    let mut previous = None;
    let mut current = *endpoints.first()?;
    loop {
        ordered.push((current, *by_first.get(&current)?));
        let next = neighbors(current)
            .into_iter()
            .filter(|neighbor| Some(*neighbor) != previous)
            .min();
        let Some(next) = next else {
            break;
        };
        previous = Some(current);
        current = next;
        if ordered.len() > lanes.len() {
            return None;
        }
    }
    (ordered.len() == lanes.len()).then_some(ordered)
}

/// Checks that every seam lane retains its own full-depth inward corridor.
pub(crate) fn seam_approaches_are_independent(
    boundary: impl IntoIterator<Item = HexCoord>,
    mask: &BTreeSet<HexCoord>,
    inward: impl Fn(HexCoord) -> HexCoord + Copy,
    depth: u32,
) -> bool {
    let mut occupied = BTreeSet::new();
    for boundary_cell in boundary {
        let mut cell = boundary_cell;
        for _ in 0..depth {
            if !mask.contains(&cell) || !occupied.insert(cell) {
                return false;
            }
            cell = inward(cell);
        }
    }
    true
}

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
#[expect(
    clippy::large_enum_variant,
    reason = "Bevy 0.19 reflection cannot derive these designer settings through Box"
)]
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
/// be generated. V3 composes one focused patch or a fixed seven- or nineteen-patch
/// world behind a separate wire contract.
#[expect(
    clippy::large_enum_variant,
    reason = "Bevy 0.19 reflection cannot derive these designer settings through Box"
)]
#[derive(Reflect, Debug, Clone, PartialEq)]
pub enum ProceduralSettings {
    /// The frozen landform/environment/tactical generator.
    V1(ProceduralV1Settings),
    /// The volume-based recipe generator.
    V2(ProceduralV2Settings),
    /// The patch-based semantic world generator.
    V3(ProceduralV3Settings),
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

/// Parameters for the retained V1 sky-island landform.
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

/// Parameters for the retained V1 linked-island topology.
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
    /// Number of sharp peaks distributed through the range.
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

/// Settings shared by every V3 world.
#[derive(Reflect, Debug, Clone, PartialEq, Eq)]
pub struct ProceduralV3Settings {
    /// One focused recipe or a validated composite layout.
    pub layout: V3LayoutSettings,
}

/// Designer-facing V3 world layouts.
#[expect(
    clippy::large_enum_variant,
    reason = "Bevy 0.19 reflection cannot derive these designer settings through Box"
)]
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Deserialize)]
pub enum V3LayoutSettings {
    /// One recipe over one connected footprint.
    Single(PatchSpec),
    /// A central Hills patch and the fixed clockwise six-recipe ring.
    Ring7(V3Ring7Settings),
    /// A central region followed by the clockwise first and second rings.
    Ring19(V3Ring19Settings),
}

/// The fixed V3 seven-region roster.
///
/// Outer regions are named by recipe rather than by direction so scenario and
/// diagnostic data remains stable if presentation orientation changes. The
/// clockwise order starts at north-east: Mountains, Waterfall, Forest, Fort,
/// Caves, then Sky Islands.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct V3Ring7Settings {
    /// Central Hills patch.
    pub center: PatchSpec,
    /// North-east Mountains patch.
    pub mountains: PatchSpec,
    /// East Waterfall patch.
    pub waterfall: PatchSpec,
    /// South-east Forest patch.
    pub forest: PatchSpec,
    /// South-west Fort patch.
    pub fort: PatchSpec,
    /// West Caves patch.
    pub caves: PatchSpec,
    /// North-west Sky Islands patch.
    pub sky_islands: PatchSpec,
}

/// Settings for the fixed nineteen-slot two-rings layout.
///
/// Region order is semantic: centre, six first-ring slots clockwise, then
/// twelve second-ring slots clockwise. Masks, reciprocal seams, and world
/// boundaries are derived by the layout resolver rather than authored here.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct V3Ring19Settings {
    /// Exactly nineteen regions in the fixed semantic slot order.
    pub regions: Vec<Ring19RegionSettings>,
    /// Default contract for every reciprocal internal seam.
    ///
    /// Its liquid setting must be `Dry`; explicit liquid connections below
    /// replace that field on their selected seams.
    pub seam_defaults: SharedEdgeSettings,
    /// Directed liquid handoffs between adjacent region slots.
    #[serde(default)]
    pub liquid_connections: Vec<Ring19LiquidConnectionSettings>,
    /// Directed liquid exits through exact outer boundary sides.
    #[serde(default)]
    pub boundary_outlets: Vec<Ring19BoundaryOutletSettings>,
}

/// One semantic region in the fixed Ring19 slot order.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ring19RegionSettings {
    /// Material and climate family.
    pub environment: V3EnvironmentSettings,
    /// Geometry and topology recipe.
    pub recipe: V3RecipeSettings,
    /// Independently named optional semantic passes.
    #[serde(default)]
    pub overlays: Vec<NamedOverlaySettings>,
    /// Clockwise sixty-degree turns applied in the region-local frame.
    pub rotation_turns: u8,
}

/// One directed liquid handoff between adjacent Ring19 regions.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ring19LiquidConnectionSettings {
    /// Upstream slot in the fixed Ring19 order.
    pub source_region: u8,
    /// Downstream slot in the fixed Ring19 order.
    pub sink_region: u8,
    /// Exact reciprocal seam width.
    pub width: u32,
    /// Exact liquid level shared by both sides of the seam.
    pub level: Level,
}

/// One directed liquid outlet through the Ring19 world boundary.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ring19BoundaryOutletSettings {
    /// Upstream slot in the fixed Ring19 order.
    pub source_region: u8,
    /// Exact outer side used by the liquid.
    pub side: Ring19BoundarySide,
    /// Exact boundary outlet width.
    pub width: u32,
    /// Exact liquid level at the terminal boundary lanes.
    pub level: Level,
}

/// Clockwise side names used by Ring19 boundary outlets.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
pub enum Ring19BoundarySide {
    /// East.
    East,
    /// South-east.
    SouthEast,
    /// South-west.
    SouthWest,
    /// West.
    West,
    /// North-west.
    NorthWest,
    /// North-east.
    NorthEast,
}

/// One V3 patch before its mask and seams are resolved by the world planner.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchSpec {
    /// Material and climate family.
    pub environment: V3EnvironmentSettings,
    /// Geometry and topology recipe.
    pub recipe: V3RecipeSettings,
    /// Independently named optional semantic passes.
    #[serde(default)]
    pub overlays: Vec<NamedOverlaySettings>,
    /// Horizontal columns owned by this patch.
    pub mask: PatchMaskSettings,
    /// Contracts for all six boundaries.
    pub edges: PatchEdgesSettings,
}

/// V3 material and climate families.
///
/// This is separate from both earlier environment enums so V3 additions cannot
/// affect frozen V1/V2 match spaces or hashes.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum V3EnvironmentSettings {
    /// Grass, dirt, exposed stone, gravel, and water.
    TemperateGrassland,
    /// Snow and ice over stone.
    Frozen,
    /// Basalt separated by non-solid lava.
    Volcanic,
    /// Stone, gravel, and dirt suited to underground spaces.
    Rocky,
}

/// V3 geometry recipes.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Deserialize)]
pub enum V3RecipeSettings {
    /// Connected, moderately elevated terrain.
    Hills(V3HillsSettings),
    /// Hills ground plus a flight-gated upper island layer.
    SkyIslands(V3SkyIslandsSettings),
    /// A broad, sharp mountain range.
    Mountains(V3MountainsSettings),
    /// A playable surface above an underground chamber network.
    Caves(V3CavesSettings),
    /// Directed water descending from an inlet to an outlet.
    Waterfall(V3WaterfallSettings),
    /// A wooded region beside open prairie.
    Forest(V3ForestSettings),
    /// A static worked-stone defensive structure.
    Fort(V3FortSettings),
    /// An off-centre crater massif with descending lava.
    Volcano(V3VolcanoSettings),
    /// Dense woodland across a complete patch.
    DeepForest(V3DeepForestSettings),
    /// Open rolling grassland without trees or an authored road.
    Prairie(V3PrairieSettings),
}

/// V3 Hills parameters, intentionally independent from the frozen V2 payload.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct V3HillsSettings {
    /// Base surface level of the valley and crossing approaches.
    pub valley_level: Level,
    /// Maximum height above the valley.
    pub max_relief: Level,
    /// Number of hill centres placed on each side of the barrier.
    pub hills_per_bank: u8,
}

/// V3 Sky Islands parameters, intentionally independent from the V2 payload.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct V3SkyIslandsSettings {
    /// Ground finalized before any upper-layer stream is sampled.
    pub ground: V3HillsSettings,
    /// Completely empty levels required below an island mass.
    pub min_clearance: Level,
    /// Target percentage of patch columns covered by the upper layer.
    pub upper_coverage_percent: u8,
}

/// V3 Mountains parameters, intentionally independent from the V2 payload.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct V3MountainsSettings {
    /// Surface level away from the range.
    pub base_level: Level,
    /// Difference between the base and the tallest peak.
    pub relief: Level,
    /// Number of sharp peaks distributed through the range.
    pub peak_count: u8,
}

/// V3 Caves parameters, intentionally independent from the V2 payload.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct V3CavesSettings {
    /// Typical surface level before rocky variation.
    pub surface_level: Level,
    /// Typical floor level of the underground stratum.
    pub cave_floor_level: Level,
    /// Target number of rooted chambers in the main network.
    pub chamber_count: u8,
}

/// V3 Volcano parameters.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct V3VolcanoSettings {
    /// Surface level away from the volcanic massif.
    pub base_level: Level,
    /// Difference between the base and the crater rim.
    pub summit_relief: Level,
    /// Target percentage of patch columns occupied by the massif.
    pub massif_coverage_percent: u8,
    /// Minimum vertical clearance between lava and the bridge deck.
    pub bridge_clearance: Level,
}

/// V3 Deep Forest parameters.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct V3DeepForestSettings {
    /// Base surface level before rolling variation.
    pub base_level: Level,
    /// Maximum rolling-ground relief.
    pub max_relief: Level,
    /// Target percentage of ordinary surfaces covered by blocking roots.
    pub blocker_coverage_percent: u8,
    /// Exact count of protected irregular clearings.
    pub clearing_count: u8,
}

/// V3 Prairie parameters.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct V3PrairieSettings {
    /// Base surface level before rolling variation.
    pub base_level: Level,
    /// Maximum rolling-ground relief.
    pub max_relief: Level,
    /// Target percentage of eligible surfaces carrying nonblocking grass.
    pub grass_coverage_percent: u8,
}

/// Reserved Waterfall recipe payload.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct V3WaterfallSettings;

/// Reserved Forest recipe payload.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct V3ForestSettings;

/// Reserved Fort recipe payload.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct V3FortSettings;

/// A stable name and semantic overlay kind.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamedOverlaySettings {
    /// Lowercase stable identifier used by seed streams and diagnostics.
    pub name: String,
    /// Semantic layer contributed by this pass.
    pub kind: V3OverlaySettings,
}

/// Semantic overlay families reserved by the V3 foundation.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum V3OverlaySettings {
    /// Directed or still liquid topology.
    Liquid,
    /// Trees, tall grass, and future surface features.
    Vegetation,
    /// Bridges, walls, stairs, and other built geometry.
    Structure,
    /// Public local gameplay light placements.
    Lighting,
}

/// Horizontal columns assigned to a V3 patch.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Deserialize)]
pub enum PatchMaskSettings {
    /// The complete world footprint. Valid only for `Single`.
    WholeWorld,
    /// A deterministic region resolved by a composite world planner.
    GeneratedRegion,
    /// An authored connected set of cube coordinates.
    Explicit(Vec<CubeCoord>),
}

/// Named contracts for all six sides of a patch.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchEdgesSettings {
    /// East boundary.
    pub east: PatchEdgeContractSettings,
    /// South-east boundary.
    pub south_east: PatchEdgeContractSettings,
    /// South-west boundary.
    pub south_west: PatchEdgeContractSettings,
    /// West boundary.
    pub west: PatchEdgeContractSettings,
    /// North-west boundary.
    pub north_west: PatchEdgeContractSettings,
    /// North-east boundary.
    pub north_east: PatchEdgeContractSettings,
}

/// Whether an edge meets the world boundary or another patch.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Deserialize)]
pub enum PatchEdgeContractSettings {
    /// No neighboring patch exists.
    WorldBoundary,
    /// Both neighboring patches consume one reciprocal semantic seam.
    Shared(SharedEdgeSettings),
}

/// Settings both sides of an internal patch seam must agree on.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedEdgeSettings {
    /// Permitted elevation range and preferred seam datum.
    pub elevation: EdgeElevationSettings,
    /// Ordinary-walker ports required through the seam.
    pub walker: WalkerPortSettings,
    /// Directed liquid handoff, or an explicitly dry seam.
    pub liquid: EdgeLiquidSettings,
    /// Cells on both sides reserved from recipe-local decoration.
    pub approach_depth: u32,
}

/// Vertical contract for one shared edge.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EdgeElevationSettings {
    /// Target surface level at the seam.
    pub preferred: Level,
    /// Lowest level the neighboring recipes may resolve.
    pub min: Level,
    /// Highest level the neighboring recipes may resolve.
    pub max: Level,
}

/// Ordinary-walker route ports on one shared edge.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WalkerPortSettings {
    /// Number of separated route ports.
    pub count: u8,
    /// Width of every route port in horizontal cells.
    pub width: u32,
}

/// Liquid flow crossing a shared edge.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum EdgeLiquidSettings {
    /// No liquid may cross this edge.
    Dry,
    /// Liquid enters this patch from its neighbor.
    Inlet(EdgeLiquidPortSettings),
    /// Liquid exits this patch into its neighbor.
    Outlet(EdgeLiquidPortSettings),
}

/// Width shared by a reciprocal liquid inlet/outlet pair.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EdgeLiquidPortSettings {
    /// Width of the contiguous liquid port in horizontal cells.
    pub width: u32,
}

struct ProceduralSettingsWire {
    generator_version: u32,
    landform: Option<LandformSettings>,
    environment: Option<ProceduralEnvironmentWire>,
    tactical: Option<TacticalSettings>,
    recipe: Option<V2RecipeSettings>,
    layout: Option<V3LayoutSettings>,
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
    Layout,
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
                    "layout" => ProceduralSettingsField::Layout,
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
                let mut layout = None;
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
                        ProceduralSettingsField::Layout => {
                            if layout.is_some() {
                                return Err(A::Error::duplicate_field("layout"));
                            }
                            layout = Some(map.next_value()?);
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
                    layout,
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
            "layout",
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
                if wire.layout.is_some() {
                    return Err(D::Error::custom(
                        "procedural V1 does not accept the V3 layout field",
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
                if wire.layout.is_some() {
                    return Err(D::Error::custom(
                        "procedural V2 does not accept the V3 layout field",
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
            3 => {
                if let Some(field) = wire.unknown_fields.iter().next() {
                    return Err(D::Error::custom(format!(
                        "procedural V3 contains unknown field {field:?}"
                    )));
                }
                if wire.landform.is_some()
                    || wire.environment.is_some()
                    || wire.tactical.is_some()
                    || wire.recipe.is_some()
                {
                    return Err(D::Error::custom(
                        "procedural V3 uses layout instead of V1/V2 terrain fields",
                    ));
                }
                let layout = wire
                    .layout
                    .ok_or_else(|| D::Error::custom("procedural V3 requires the layout field"))?;
                Ok(Self::V3(ProceduralV3Settings { layout }))
            }
            version => Err(D::Error::custom(format!(
                "unsupported procedural generator_version {version}; expected 1, 2, or 3"
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
            Self::V3(_) => 3,
        }
    }

    fn validate(&self, grid_radius: u32) -> Result<(), String> {
        match self {
            Self::V1(settings) => settings.validate(grid_radius),
            Self::V2(settings) => settings.validate(grid_radius),
            Self::V3(settings) => settings.validate(grid_radius),
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

impl ProceduralV3Settings {
    fn validate(&self, grid_radius: u32) -> Result<(), String> {
        match &self.layout {
            V3LayoutSettings::Single(patch) => {
                if !(12..=40).contains(&grid_radius) {
                    return Err(
                        "procedural V3 Single requires grid_radius from 12 through 40".to_owned(),
                    );
                }
                patch.validate(grid_radius, "V3 Single patch")?;
                match &patch.mask {
                    PatchMaskSettings::WholeWorld | PatchMaskSettings::Explicit(_) => {}
                    PatchMaskSettings::GeneratedRegion => {
                        return Err(
                            "V3 Single mask must be WholeWorld or a connected Explicit mask"
                                .to_owned(),
                        );
                    }
                }
                if !patch.edges.all_world_boundaries() {
                    return Err("V3 Single edges must all be WorldBoundary".to_owned());
                }
                Ok(())
            }
            V3LayoutSettings::Ring7(ring) => {
                if grid_radius != 33 {
                    return Err("procedural V3 Ring7 requires grid_radius exactly 33".to_owned());
                }
                ring.validate(grid_radius)
            }
            V3LayoutSettings::Ring19(ring) => {
                if grid_radius != RING19_RADIUS {
                    return Err(format!(
                        "procedural V3 Ring19 requires grid_radius exactly {RING19_RADIUS}"
                    ));
                }
                ring.validate()
            }
        }
    }
}

impl V3Ring7Settings {
    fn validate(&self, grid_radius: u32) -> Result<(), String> {
        let patches = self.named_patches();
        for (name, patch) in patches {
            patch.validate(grid_radius, &format!("V3 Ring7 {name} patch"))?;
        }

        if !matches!(&self.center.recipe, V3RecipeSettings::Hills(_))
            || !matches!(&self.mountains.recipe, V3RecipeSettings::Mountains(_))
            || !matches!(&self.waterfall.recipe, V3RecipeSettings::Waterfall(_))
            || !matches!(&self.forest.recipe, V3RecipeSettings::Forest(_))
            || !matches!(&self.fort.recipe, V3RecipeSettings::Fort(_))
            || !matches!(&self.caves.recipe, V3RecipeSettings::Caves(_))
            || !matches!(&self.sky_islands.recipe, V3RecipeSettings::SkyIslands(_))
        {
            return Err(
                "V3 Ring7 recipe roster must be center Hills then Mountains, Waterfall, \
                 Forest, Fort, Caves, and SkyIslands clockwise from north-east"
                    .to_owned(),
            );
        }

        self.validate_masks(grid_radius)?;
        self.validate_edges()?;
        self.validate_explicit_seam_geometry()?;
        self.validate_liquid_graph()?;
        if !self.mountains.edges.all_liquids_dry() {
            return Err("V3 Ring7 Mountains edges must all be dry".to_owned());
        }
        Ok(())
    }

    fn named_patches(&self) -> [(&'static str, &PatchSpec); 7] {
        [
            ("center", &self.center),
            ("mountains", &self.mountains),
            ("waterfall", &self.waterfall),
            ("forest", &self.forest),
            ("fort", &self.fort),
            ("caves", &self.caves),
            ("sky_islands", &self.sky_islands),
        ]
    }

    fn validate_masks(&self, grid_radius: u32) -> Result<(), String> {
        let patches = self.named_patches();
        let all_generated = patches
            .iter()
            .all(|(_, patch)| matches!(patch.mask, PatchMaskSettings::GeneratedRegion));
        let all_explicit = patches
            .iter()
            .all(|(_, patch)| matches!(patch.mask, PatchMaskSettings::Explicit(_)));

        if all_generated {
            return Ok(());
        }
        if !all_explicit {
            return Err("V3 Ring7 masks must be all GeneratedRegion or all Explicit".to_owned());
        }

        let mut covered = BTreeSet::new();
        for (name, patch) in patches {
            let PatchMaskSettings::Explicit(coords) = &patch.mask else {
                unreachable!("all Ring7 masks were established as Explicit");
            };
            for coord in coords {
                let key = (coord.x, coord.y, coord.z);
                if !covered.insert(key) {
                    return Err(format!(
                        "V3 Ring7 Explicit masks overlap at ({}, {}, {}) while adding {name}",
                        coord.x, coord.y, coord.z
                    ));
                }
            }
        }

        let radius = u64::from(grid_radius);
        let expected_columns = 3_u64
            .checked_mul(radius)
            .and_then(|value| value.checked_mul(radius))
            .and_then(|value| value.checked_add(3 * radius))
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| "V3 Ring7 footprint size overflows".to_owned())?;
        if u64::try_from(covered.len()).ok() != Some(expected_columns) {
            return Err(format!(
                "V3 Ring7 Explicit masks must cover all {expected_columns} world columns exactly"
            ));
        }
        Ok(())
    }

    fn validate_edges(&self) -> Result<(), String> {
        validate_reciprocal_edge(
            &self.center.edges.north_east,
            &self.mountains.edges.south_west,
            "center north_east / mountains south_west",
        )?;
        validate_reciprocal_edge(
            &self.center.edges.east,
            &self.waterfall.edges.west,
            "center east / waterfall west",
        )?;
        validate_reciprocal_edge(
            &self.center.edges.south_east,
            &self.forest.edges.north_west,
            "center south_east / forest north_west",
        )?;
        validate_reciprocal_edge(
            &self.center.edges.south_west,
            &self.fort.edges.north_east,
            "center south_west / fort north_east",
        )?;
        validate_reciprocal_edge(
            &self.center.edges.west,
            &self.caves.edges.east,
            "center west / caves east",
        )?;
        validate_reciprocal_edge(
            &self.center.edges.north_west,
            &self.sky_islands.edges.south_east,
            "center north_west / sky_islands south_east",
        )?;

        validate_reciprocal_edge(
            &self.mountains.edges.south_east,
            &self.waterfall.edges.north_west,
            "mountains south_east / waterfall north_west",
        )?;
        validate_reciprocal_edge(
            &self.waterfall.edges.south_west,
            &self.forest.edges.north_east,
            "waterfall south_west / forest north_east",
        )?;
        validate_reciprocal_edge(
            &self.forest.edges.west,
            &self.fort.edges.east,
            "forest west / fort east",
        )?;
        validate_reciprocal_edge(
            &self.fort.edges.north_west,
            &self.caves.edges.south_east,
            "fort north_west / caves south_east",
        )?;
        validate_reciprocal_edge(
            &self.caves.edges.north_east,
            &self.sky_islands.edges.south_west,
            "caves north_east / sky_islands south_west",
        )?;
        validate_reciprocal_edge(
            &self.sky_islands.edges.east,
            &self.mountains.edges.west,
            "sky_islands east / mountains west",
        )?;

        for (label, edge) in [
            ("mountains east", &self.mountains.edges.east),
            ("mountains north_west", &self.mountains.edges.north_west),
            ("mountains north_east", &self.mountains.edges.north_east),
            ("waterfall east", &self.waterfall.edges.east),
            ("waterfall south_east", &self.waterfall.edges.south_east),
            ("waterfall north_east", &self.waterfall.edges.north_east),
            ("forest east", &self.forest.edges.east),
            ("forest south_east", &self.forest.edges.south_east),
            ("forest south_west", &self.forest.edges.south_west),
            ("fort south_east", &self.fort.edges.south_east),
            ("fort south_west", &self.fort.edges.south_west),
            ("fort west", &self.fort.edges.west),
            ("caves south_west", &self.caves.edges.south_west),
            ("caves west", &self.caves.edges.west),
            ("caves north_west", &self.caves.edges.north_west),
            ("sky_islands west", &self.sky_islands.edges.west),
            ("sky_islands north_west", &self.sky_islands.edges.north_west),
            ("sky_islands north_east", &self.sky_islands.edges.north_east),
        ] {
            if !matches!(edge, PatchEdgeContractSettings::WorldBoundary) {
                return Err(format!("V3 Ring7 outer edge {label} must be WorldBoundary"));
            }
        }
        Ok(())
    }

    fn validate_explicit_seam_geometry(&self) -> Result<(), String> {
        if !self
            .named_patches()
            .iter()
            .all(|(_, patch)| matches!(patch.mask, PatchMaskSettings::Explicit(_)))
        {
            return Ok(());
        }

        let center = explicit_hex_mask(&self.center.mask, "center")?;
        let mountains = explicit_hex_mask(&self.mountains.mask, "mountains")?;
        let waterfall = explicit_hex_mask(&self.waterfall.mask, "waterfall")?;
        let forest = explicit_hex_mask(&self.forest.mask, "forest")?;
        let fort = explicit_hex_mask(&self.fort.mask, "fort")?;
        let caves = explicit_hex_mask(&self.caves.mask, "caves")?;
        let sky_islands = explicit_hex_mask(&self.sky_islands.mask, "sky_islands")?;

        const EAST: (i32, i32, i32) = (1, 0, -1);
        const SOUTH_EAST: (i32, i32, i32) = (0, 1, -1);
        const SOUTH_WEST: (i32, i32, i32) = (-1, 1, 0);
        const WEST: (i32, i32, i32) = (-1, 0, 1);
        const NORTH_WEST: (i32, i32, i32) = (0, -1, 1);
        const NORTH_EAST: (i32, i32, i32) = (1, -1, 0);

        for (label, first_mask, second_mask, outward, edge) in [
            (
                "center north_east / mountains south_west",
                &center,
                &mountains,
                NORTH_EAST,
                &self.center.edges.north_east,
            ),
            (
                "center east / waterfall west",
                &center,
                &waterfall,
                EAST,
                &self.center.edges.east,
            ),
            (
                "center south_east / forest north_west",
                &center,
                &forest,
                SOUTH_EAST,
                &self.center.edges.south_east,
            ),
            (
                "center south_west / fort north_east",
                &center,
                &fort,
                SOUTH_WEST,
                &self.center.edges.south_west,
            ),
            (
                "center west / caves east",
                &center,
                &caves,
                WEST,
                &self.center.edges.west,
            ),
            (
                "center north_west / sky_islands south_east",
                &center,
                &sky_islands,
                NORTH_WEST,
                &self.center.edges.north_west,
            ),
            (
                "mountains south_east / waterfall north_west",
                &mountains,
                &waterfall,
                SOUTH_EAST,
                &self.mountains.edges.south_east,
            ),
            (
                "waterfall south_west / forest north_east",
                &waterfall,
                &forest,
                SOUTH_WEST,
                &self.waterfall.edges.south_west,
            ),
            (
                "forest west / fort east",
                &forest,
                &fort,
                WEST,
                &self.forest.edges.west,
            ),
            (
                "fort north_west / caves south_east",
                &fort,
                &caves,
                NORTH_WEST,
                &self.fort.edges.north_west,
            ),
            (
                "caves north_east / sky_islands south_west",
                &caves,
                &sky_islands,
                NORTH_EAST,
                &self.caves.edges.north_east,
            ),
            (
                "sky_islands east / mountains west",
                &sky_islands,
                &mountains,
                EAST,
                &self.sky_islands.edges.east,
            ),
        ] {
            let PatchEdgeContractSettings::Shared(shared) = edge else {
                return Err(format!(
                    "V3 Ring7 internal seam {label} must be Shared on both sides"
                ));
            };
            validate_explicit_simple_seam(
                label,
                first_mask,
                second_mask,
                outward,
                shared.approach_depth,
            )?;
        }
        Ok(())
    }

    fn validate_liquid_graph(&self) -> Result<(), String> {
        let seams = [
            (
                0,
                &self.center.edges.north_east,
                1,
                &self.mountains.edges.south_west,
            ),
            (0, &self.center.edges.east, 2, &self.waterfall.edges.west),
            (
                0,
                &self.center.edges.south_east,
                3,
                &self.forest.edges.north_west,
            ),
            (
                0,
                &self.center.edges.south_west,
                4,
                &self.fort.edges.north_east,
            ),
            (0, &self.center.edges.west, 5, &self.caves.edges.east),
            (
                0,
                &self.center.edges.north_west,
                6,
                &self.sky_islands.edges.south_east,
            ),
            (
                1,
                &self.mountains.edges.south_east,
                2,
                &self.waterfall.edges.north_west,
            ),
            (
                2,
                &self.waterfall.edges.south_west,
                3,
                &self.forest.edges.north_east,
            ),
            (3, &self.forest.edges.west, 4, &self.fort.edges.east),
            (
                4,
                &self.fort.edges.north_west,
                5,
                &self.caves.edges.south_east,
            ),
            (
                5,
                &self.caves.edges.north_east,
                6,
                &self.sky_islands.edges.south_west,
            ),
            (
                6,
                &self.sky_islands.edges.east,
                1,
                &self.mountains.edges.west,
            ),
        ];
        let directed = seams
            .into_iter()
            .filter_map(|(first_id, first, second_id, second)| {
                directed_liquid_edge(first_id, first, second_id, second)
            });
        if directed_graph_is_acyclic(7, directed) {
            Ok(())
        } else {
            Err("V3 Ring7 directed liquid contracts must form an acyclic patch graph".to_owned())
        }
    }
}

impl V3Ring19Settings {
    fn validate(&self) -> Result<(), String> {
        self.validate_two_rings_contract()
    }

    fn validate_structure(&self) -> Result<(), String> {
        if self.regions.len() != V3_RING19_REGION_COUNT {
            return Err(format!(
                "V3 Ring19 requires exactly {V3_RING19_REGION_COUNT} regions, got {}",
                self.regions.len()
            ));
        }

        for (index, region) in self.regions.iter().enumerate() {
            if region.rotation_turns > 5 {
                return Err(format!(
                    "V3 Ring19 region {index} rotation_turns must be in 0..=5"
                ));
            }
            validate_v3_recipe(
                &region.recipe,
                region.environment,
                RING19_RECIPE_VALIDATION_RADIUS,
            )
            .map_err(|error| format!("V3 Ring19 region {index}: {error}"))?;
            validate_named_overlays(&region.overlays, &format!("V3 Ring19 region {index}"))?;
        }

        let seam_label = "V3 Ring19 seam_defaults";
        self.seam_defaults.elevation.validate(seam_label)?;
        self.seam_defaults.walker.validate(seam_label)?;
        if !matches!(self.seam_defaults.liquid, EdgeLiquidSettings::Dry) {
            return Err(
                "V3 Ring19 seam_defaults.liquid must be Dry; use liquid_connections".to_owned(),
            );
        }
        if self.seam_defaults.approach_depth > RING19_RADIUS {
            return Err(format!(
                "V3 Ring19 seam_defaults approach_depth cannot exceed {RING19_RADIUS}"
            ));
        }

        let mut occupied_liquid_seams = BTreeSet::new();
        let mut outgoing_regions = BTreeSet::new();
        let mut directed_edges = Vec::new();
        let mut liquid_regions = BTreeSet::new();
        let mut downstream = BTreeMap::new();
        for connection in &self.liquid_connections {
            let source = usize::from(connection.source_region);
            let sink = usize::from(connection.sink_region);
            if source >= V3_RING19_REGION_COUNT || sink >= V3_RING19_REGION_COUNT {
                return Err(format!(
                    "V3 Ring19 liquid connection {} -> {} references a region outside 0..{}",
                    connection.source_region,
                    connection.sink_region,
                    V3_RING19_REGION_COUNT - 1
                ));
            }
            if source == sink {
                return Err(format!(
                    "V3 Ring19 liquid connection {source} -> {sink} cannot target itself"
                ));
            }
            if !ring19_regions_are_adjacent(connection.source_region, connection.sink_region) {
                return Err(format!(
                    "V3 Ring19 liquid connection {source} -> {sink} must use one internal seam"
                ));
            }
            validate_ring19_liquid_width(connection.width, "liquid connection")?;
            validate_ring19_liquid_level(connection.level, "liquid connection")?;
            let seam = if source < sink {
                (source, sink)
            } else {
                (sink, source)
            };
            if !occupied_liquid_seams.insert(seam) {
                return Err(format!(
                    "V3 Ring19 internal seam {seam:?} has more than one liquid connection"
                ));
            }
            if !outgoing_regions.insert(source) {
                return Err(format!(
                    "V3 Ring19 region {source} has more than one liquid outlet"
                ));
            }
            directed_edges.push((source, sink));
            liquid_regions.extend([source, sink]);
            downstream.insert(source, sink);
        }

        let mut boundary_ports = BTreeSet::new();
        let mut boundary_sources = BTreeSet::new();
        for outlet in &self.boundary_outlets {
            let source = usize::from(outlet.source_region);
            if source >= V3_RING19_REGION_COUNT {
                return Err(format!(
                    "V3 Ring19 boundary outlet {} references a region outside 0..{}",
                    outlet.source_region,
                    V3_RING19_REGION_COUNT - 1
                ));
            }
            if !ring19_side_is_world_boundary(outlet.source_region, outlet.side) {
                return Err(format!(
                    "V3 Ring19 boundary outlet region {source} side {:?} is not an outer boundary",
                    outlet.side
                ));
            }
            validate_ring19_liquid_width(outlet.width, "boundary outlet")?;
            validate_ring19_liquid_level(outlet.level, "boundary outlet")?;
            if !boundary_ports.insert((outlet.source_region, outlet.side)) {
                return Err(format!(
                    "V3 Ring19 repeats boundary outlet region {source} side {:?}",
                    outlet.side
                ));
            }
            if !outgoing_regions.insert(source) {
                return Err(format!(
                    "V3 Ring19 region {source} has more than one liquid outlet"
                ));
            }
            boundary_sources.insert(source);
        }
        if self.boundary_outlets.is_empty() {
            return Err("V3 Ring19 requires at least one explicit boundary outlet".to_owned());
        }
        if !directed_graph_is_acyclic(V3_RING19_REGION_COUNT, directed_edges) {
            return Err("V3 Ring19 directed liquid connections must be acyclic".to_owned());
        }
        for origin in liquid_regions {
            let mut current = origin;
            while !boundary_sources.contains(&current) {
                let Some(next) = downstream.get(&current).copied() else {
                    return Err(format!(
                        "V3 Ring19 liquid component containing region {origin} does not terminate at a boundary outlet"
                    ));
                };
                current = next;
            }
        }
        Ok(())
    }

    pub(crate) fn validate_two_rings_contract(&self) -> Result<(), String> {
        self.validate_structure()?;

        for (index, (actual, (environment, recipe, rotation_turns))) in
            self.regions.iter().zip(TWO_RINGS_REGIONS).enumerate()
        {
            if actual.environment != environment
                || ring19_recipe_name(&actual.recipe) != recipe
                || actual.rotation_turns != rotation_turns
            {
                return Err(format!(
                    "V3 Ring19 Two Rings slot {index} requires {environment:?} {recipe} at \
                     rotation {rotation_turns}, got {:?} {} at rotation {}",
                    actual.environment,
                    ring19_recipe_name(&actual.recipe),
                    actual.rotation_turns
                ));
            }
            if index == 14 {
                match actual.overlays.as_slice() {
                    [overlay]
                        if overlay.name == "cave_crystals"
                            && overlay.kind == V3OverlaySettings::Lighting => {}
                    _ => {
                        return Err(
                            "V3 Ring19 Two Rings Caves slot requires exactly the cave_crystals \
                             Lighting overlay"
                                .to_owned(),
                        );
                    }
                }
            } else if !actual.overlays.is_empty() {
                return Err(format!(
                    "V3 Ring19 Two Rings slot {index} does not admit overlays"
                ));
            }
        }

        if self.seam_defaults.elevation
            != (EdgeElevationSettings {
                preferred: 17,
                min: 16,
                max: 18,
            })
            || self.seam_defaults.walker != (WalkerPortSettings { count: 2, width: 2 })
            || !matches!(self.seam_defaults.liquid, EdgeLiquidSettings::Dry)
            || self.seam_defaults.approach_depth != 3
        {
            return Err(
                "V3 Ring19 Two Rings seam defaults require elevation 16..=18 (preferred 17), \
                 two width-two walker ports, Dry liquid, and approach depth three"
                    .to_owned(),
            );
        }

        let actual_internal = self
            .liquid_connections
            .iter()
            .map(|connection| {
                (
                    connection.source_region,
                    connection.sink_region,
                    connection.width,
                    connection.level,
                )
            })
            .collect::<BTreeSet<_>>();
        let expected_internal = TWO_RINGS_INTERNAL_HYDROLOGY
            .into_iter()
            .collect::<BTreeSet<_>>();
        if self.liquid_connections.len() != TWO_RINGS_INTERNAL_HYDROLOGY.len()
            || actual_internal != expected_internal
        {
            return Err(format!(
                "V3 Ring19 Two Rings internal hydrology must be exactly \
                 {expected_internal:?}, got {actual_internal:?}"
            ));
        }

        let actual_boundary = self
            .boundary_outlets
            .iter()
            .map(|outlet| {
                (
                    outlet.source_region,
                    outlet.side,
                    outlet.width,
                    outlet.level,
                )
            })
            .collect::<BTreeSet<_>>();
        let expected_boundary = TWO_RINGS_BOUNDARY_HYDROLOGY
            .into_iter()
            .collect::<BTreeSet<_>>();
        if self.boundary_outlets.len() != TWO_RINGS_BOUNDARY_HYDROLOGY.len()
            || actual_boundary != expected_boundary
        {
            return Err(format!(
                "V3 Ring19 Two Rings boundary hydrology must be exactly \
                 {expected_boundary:?}, got {actual_boundary:?}"
            ));
        }
        Ok(())
    }
}

const fn ring19_recipe_name(recipe: &V3RecipeSettings) -> &'static str {
    match recipe {
        V3RecipeSettings::Hills(_) => "Hills",
        V3RecipeSettings::SkyIslands(_) => "SkyIslands",
        V3RecipeSettings::Mountains(_) => "Mountains",
        V3RecipeSettings::Caves(_) => "Caves",
        V3RecipeSettings::Waterfall(_) => "Waterfall",
        V3RecipeSettings::Forest(_) => "Forest",
        V3RecipeSettings::Fort(_) => "Fort",
        V3RecipeSettings::Volcano(_) => "Volcano",
        V3RecipeSettings::DeepForest(_) => "DeepForest",
        V3RecipeSettings::Prairie(_) => "Prairie",
    }
}

impl PatchSpec {
    fn validate(&self, grid_radius: u32, label: &str) -> Result<(), String> {
        self.validate_recipe(grid_radius)?;
        self.validate_overlays(label)?;
        self.mask.validate(grid_radius, label)?;
        self.edges.validate(grid_radius, label)
    }

    fn validate_recipe(&self, grid_radius: u32) -> Result<(), String> {
        validate_v3_recipe(&self.recipe, self.environment, grid_radius)
    }

    fn validate_overlays(&self, label: &str) -> Result<(), String> {
        validate_named_overlays(&self.overlays, label)
    }
}

fn validate_v3_recipe(
    recipe: &V3RecipeSettings,
    environment: V3EnvironmentSettings,
    grid_radius: u32,
) -> Result<(), String> {
    match (recipe, environment) {
        (
            V3RecipeSettings::Hills(hills),
            V3EnvironmentSettings::TemperateGrassland
            | V3EnvironmentSettings::Frozen
            | V3EnvironmentSettings::Volcanic,
        ) => hills.validate(grid_radius),
        (V3RecipeSettings::Hills(_), V3EnvironmentSettings::Rocky) => {
            Err("V3 Hills does not support the Rocky environment".to_owned())
        }
        (
            V3RecipeSettings::SkyIslands(islands),
            V3EnvironmentSettings::TemperateGrassland | V3EnvironmentSettings::Frozen,
        ) => islands.validate(grid_radius),
        (V3RecipeSettings::SkyIslands(_), _) => {
            Err("V3 SkyIslands requires TemperateGrassland or Frozen".to_owned())
        }
        (V3RecipeSettings::Mountains(mountains), V3EnvironmentSettings::Frozen) => {
            mountains.validate(grid_radius)
        }
        (V3RecipeSettings::Mountains(_), _) => {
            Err("V3 Mountains requires the Frozen environment".to_owned())
        }
        (V3RecipeSettings::Caves(caves), V3EnvironmentSettings::Rocky) => {
            caves.validate(grid_radius)
        }
        (V3RecipeSettings::Caves(_), _) => {
            Err("V3 Caves requires the Rocky environment".to_owned())
        }
        (
            V3RecipeSettings::Waterfall(_)
            | V3RecipeSettings::Forest(_)
            | V3RecipeSettings::Fort(_),
            V3EnvironmentSettings::TemperateGrassland,
        ) => Ok(()),
        (
            V3RecipeSettings::Waterfall(_)
            | V3RecipeSettings::Forest(_)
            | V3RecipeSettings::Fort(_),
            _,
        ) => Err("V3 Waterfall, Forest, and Fort currently require TemperateGrassland".to_owned()),
        (V3RecipeSettings::Volcano(settings), V3EnvironmentSettings::Volcanic) => {
            settings.validate(grid_radius)
        }
        (V3RecipeSettings::Volcano(_), _) => {
            Err("V3 Volcano requires the Volcanic environment".to_owned())
        }
        (V3RecipeSettings::DeepForest(settings), V3EnvironmentSettings::TemperateGrassland) => {
            settings.validate(grid_radius)
        }
        (V3RecipeSettings::DeepForest(_), _) => {
            Err("V3 DeepForest requires the TemperateGrassland environment".to_owned())
        }
        (V3RecipeSettings::Prairie(settings), V3EnvironmentSettings::TemperateGrassland) => {
            settings.validate(grid_radius)
        }
        (V3RecipeSettings::Prairie(_), _) => {
            Err("V3 Prairie requires the TemperateGrassland environment".to_owned())
        }
    }
}

fn validate_named_overlays(overlays: &[NamedOverlaySettings], label: &str) -> Result<(), String> {
    let mut names = BTreeSet::new();
    for overlay in overlays {
        if !is_stable_identifier(&overlay.name) {
            return Err(format!(
                "{label} overlay name {:?} must be a lowercase stable identifier",
                overlay.name
            ));
        }
        if !names.insert(overlay.name.as_str()) {
            return Err(format!(
                "{label} contains duplicate overlay name {:?}",
                overlay.name
            ));
        }
    }
    Ok(())
}

fn validate_ring19_liquid_width(width: u32, label: &str) -> Result<(), String> {
    if !(2..=MAX_SEAM_PORT_WIDTH).contains(&width) {
        return Err(format!(
            "V3 Ring19 {label} width must be between 2 and {MAX_SEAM_PORT_WIDTH}"
        ));
    }
    Ok(())
}

fn validate_ring19_liquid_level(level: Level, label: &str) -> Result<(), String> {
    if !(3..=MAX_PROCEDURAL_LEVEL).contains(&level) {
        return Err(format!(
            "V3 Ring19 {label} level must be between 3 and {MAX_PROCEDURAL_LEVEL}"
        ));
    }
    Ok(())
}

fn ring19_regions_are_adjacent(first: u8, second: u8) -> bool {
    let (Some(first), Some(second)) = (ring19_region_coord(first), ring19_region_coord(second))
    else {
        return false;
    };
    cube_tuple_distance(first, second) == 1
}

fn ring19_side_is_world_boundary(region: u8, side: Ring19BoundarySide) -> bool {
    let Some((x, y, z)) = ring19_region_coord(region) else {
        return false;
    };
    let (dx, dy, dz) = ring19_side_delta(side);
    cube_tuple_radius((x + dx, y + dy, z + dz)) > 2
}

pub(crate) fn ring19_region_coord(region: u8) -> Option<(i32, i32, i32)> {
    const COORDS: [(i32, i32, i32); V3_RING19_REGION_COUNT] = [
        (0, 0, 0),
        (1, -1, 0),
        (1, 0, -1),
        (0, 1, -1),
        (-1, 1, 0),
        (-1, 0, 1),
        (0, -1, 1),
        (2, -2, 0),
        (2, -1, -1),
        (2, 0, -2),
        (1, 1, -2),
        (0, 2, -2),
        (-1, 2, -1),
        (-2, 2, 0),
        (-2, 1, 1),
        (-2, 0, 2),
        (-1, -1, 2),
        (0, -2, 2),
        (1, -2, 1),
    ];
    COORDS.get(usize::from(region)).copied()
}

const fn ring19_side_delta(side: Ring19BoundarySide) -> (i32, i32, i32) {
    match side {
        Ring19BoundarySide::East => (1, 0, -1),
        Ring19BoundarySide::SouthEast => (0, 1, -1),
        Ring19BoundarySide::SouthWest => (-1, 1, 0),
        Ring19BoundarySide::West => (-1, 0, 1),
        Ring19BoundarySide::NorthWest => (0, -1, 1),
        Ring19BoundarySide::NorthEast => (1, -1, 0),
    }
}

fn cube_tuple_distance(first: (i32, i32, i32), second: (i32, i32, i32)) -> u32 {
    let dx = first.0.abs_diff(second.0);
    let dy = first.1.abs_diff(second.1);
    let dz = first.2.abs_diff(second.2);
    dx.max(dy).max(dz)
}

fn cube_tuple_radius(coord: (i32, i32, i32)) -> u32 {
    coord
        .0
        .unsigned_abs()
        .max(coord.1.unsigned_abs())
        .max(coord.2.unsigned_abs())
}

impl PatchMaskSettings {
    fn validate(&self, grid_radius: u32, label: &str) -> Result<(), String> {
        let Self::Explicit(coords) = self else {
            return Ok(());
        };
        if coords.is_empty() {
            return Err(format!("{label} Explicit mask cannot be empty"));
        }

        let mut unique = BTreeSet::new();
        for (index, coord) in coords.iter().copied().enumerate() {
            checked_coord(coord, grid_radius, &format!("{label}.mask[{index}]"))?;
            if !unique.insert((coord.x, coord.y, coord.z)) {
                return Err(format!(
                    "{label} Explicit mask contains duplicate coordinate ({}, {}, {})",
                    coord.x, coord.y, coord.z
                ));
            }
        }

        let Some(start) = unique.first().copied() else {
            return Err(format!("{label} Explicit mask cannot be empty"));
        };
        let mut visited = BTreeSet::from([start]);
        let mut frontier = VecDeque::from([start]);
        while let Some((x, y, z)) = frontier.pop_front() {
            for (dx, dy, dz) in CUBE_NEIGHBORS {
                let neighbor = (x + dx, y + dy, z + dz);
                if unique.contains(&neighbor) && visited.insert(neighbor) {
                    frontier.push_back(neighbor);
                }
            }
        }
        if visited.len() != unique.len() {
            return Err(format!("{label} Explicit mask must be connected"));
        }
        Ok(())
    }
}

fn explicit_hex_mask(mask: &PatchMaskSettings, label: &str) -> Result<BTreeSet<HexCoord>, String> {
    let PatchMaskSettings::Explicit(coords) = mask else {
        return Err(format!("V3 Ring7 {label} mask must be Explicit"));
    };
    coords
        .iter()
        .map(|coord| {
            HexCoord::try_new_cubic(coord.x, coord.y, coord.z).ok_or_else(|| {
                format!(
                    "V3 Ring7 {label} mask contains invalid cube coordinate ({}, {}, {})",
                    coord.x, coord.y, coord.z
                )
            })
        })
        .collect()
}

fn validate_explicit_simple_seam(
    label: &str,
    first_mask: &BTreeSet<HexCoord>,
    second_mask: &BTreeSet<HexCoord>,
    outward: (i32, i32, i32),
    approach_depth: u32,
) -> Result<(), String> {
    let oriented_lanes = first_mask
        .iter()
        .filter_map(|first| {
            let second = shift_hex(*first, outward);
            second_mask.contains(&second).then_some((*first, second))
        })
        .collect();
    let ordered = ordered_simple_seam_lanes(&oriented_lanes).ok_or_else(|| {
        format!("V3 Ring7 Explicit masks for {label} must form one oriented simple contiguous seam")
    })?;
    let inward = (-outward.0, -outward.1, -outward.2);
    let first_approaches = seam_approaches_are_independent(
        ordered.iter().map(|(first, _)| *first),
        first_mask,
        |coord| shift_hex(coord, inward),
        approach_depth,
    );
    let second_approaches = seam_approaches_are_independent(
        ordered.iter().map(|(_, second)| *second),
        second_mask,
        |coord| shift_hex(coord, outward),
        approach_depth,
    );
    if !first_approaches || !second_approaches {
        return Err(format!(
            "V3 Ring7 Explicit masks for {label} must preserve independent depth-{approach_depth} \
             seam approaches"
        ));
    }
    Ok(())
}

fn shift_hex(coord: HexCoord, delta: (i32, i32, i32)) -> HexCoord {
    let [x, y, z] = coord.to_cubic_array();
    HexCoord::new_cubic(x + delta.0, y + delta.1, z + delta.2)
}

impl PatchEdgesSettings {
    fn edges(&self) -> [&PatchEdgeContractSettings; 6] {
        [
            &self.east,
            &self.south_east,
            &self.south_west,
            &self.west,
            &self.north_west,
            &self.north_east,
        ]
    }

    fn validate(&self, grid_radius: u32, label: &str) -> Result<(), String> {
        for (direction, edge) in [
            ("east", &self.east),
            ("south_east", &self.south_east),
            ("south_west", &self.south_west),
            ("west", &self.west),
            ("north_west", &self.north_west),
            ("north_east", &self.north_east),
        ] {
            edge.validate(grid_radius, &format!("{label}.edges.{direction}"))?;
        }
        Ok(())
    }

    fn all_world_boundaries(&self) -> bool {
        self.edges()
            .into_iter()
            .all(|edge| matches!(edge, PatchEdgeContractSettings::WorldBoundary))
    }

    fn all_liquids_dry(&self) -> bool {
        self.edges().into_iter().all(|edge| match edge {
            PatchEdgeContractSettings::WorldBoundary => true,
            PatchEdgeContractSettings::Shared(shared) => {
                matches!(shared.liquid, EdgeLiquidSettings::Dry)
            }
        })
    }
}

impl PatchEdgeContractSettings {
    fn validate(&self, grid_radius: u32, label: &str) -> Result<(), String> {
        let Self::Shared(shared) = self else {
            return Ok(());
        };
        shared.elevation.validate(label)?;
        shared.walker.validate(label)?;
        shared.liquid.validate(label)?;
        if shared.approach_depth > grid_radius {
            return Err(format!(
                "{label} approach_depth cannot exceed grid_radius {grid_radius}"
            ));
        }
        Ok(())
    }
}

impl EdgeElevationSettings {
    fn validate(self, label: &str) -> Result<(), String> {
        if self.min < 0
            || self.min > self.preferred
            || self.preferred > self.max
            || self.max > MAX_PROCEDURAL_LEVEL
        {
            return Err(format!(
                "{label} elevation must satisfy 0 <= min <= preferred <= max <= \
                 {MAX_PROCEDURAL_LEVEL}"
            ));
        }
        Ok(())
    }
}

impl WalkerPortSettings {
    fn validate(self, label: &str) -> Result<(), String> {
        if self.count > MAX_WALKER_PORT_COUNT {
            return Err(format!(
                "{label} walker port count cannot exceed {MAX_WALKER_PORT_COUNT}"
            ));
        }
        if self.count == 0 {
            if self.width != 0 {
                return Err(format!(
                    "{label} walker width must be 0 when no ports are requested"
                ));
            }
        } else {
            if self.width < 2 {
                return Err(format!(
                    "{label} walker ports must be at least two cells wide"
                ));
            }
            if self.width > MAX_SEAM_PORT_WIDTH {
                return Err(format!(
                    "{label} walker port width cannot exceed {MAX_SEAM_PORT_WIDTH}"
                ));
            }
        }
        Ok(())
    }
}

impl EdgeLiquidSettings {
    fn validate(self, label: &str) -> Result<(), String> {
        let (Self::Inlet(port) | Self::Outlet(port)) = self else {
            return Ok(());
        };
        if port.width < 2 {
            return Err(format!(
                "{label} liquid ports must be at least two cells wide"
            ));
        }
        if port.width > MAX_SEAM_PORT_WIDTH {
            return Err(format!(
                "{label} liquid port width cannot exceed {MAX_SEAM_PORT_WIDTH}"
            ));
        }
        Ok(())
    }
}

fn validate_reciprocal_edge(
    first: &PatchEdgeContractSettings,
    second: &PatchEdgeContractSettings,
    label: &str,
) -> Result<(), String> {
    let (PatchEdgeContractSettings::Shared(first), PatchEdgeContractSettings::Shared(second)) =
        (first, second)
    else {
        return Err(format!(
            "V3 Ring7 internal seam {label} must be Shared on both sides"
        ));
    };

    if first.elevation != second.elevation
        || first.walker != second.walker
        || first.approach_depth != second.approach_depth
    {
        return Err(format!(
            "V3 Ring7 internal seam {label} has mismatched shared settings"
        ));
    }

    let liquid_matches = match (first.liquid, second.liquid) {
        (EdgeLiquidSettings::Dry, EdgeLiquidSettings::Dry) => true,
        (EdgeLiquidSettings::Inlet(first), EdgeLiquidSettings::Outlet(second))
        | (EdgeLiquidSettings::Outlet(first), EdgeLiquidSettings::Inlet(second)) => {
            first.width == second.width
        }
        _ => false,
    };
    if !liquid_matches {
        return Err(format!(
            "V3 Ring7 internal seam {label} requires Dry/Dry or reciprocal equal-width \
             Inlet/Outlet liquid settings"
        ));
    }
    Ok(())
}

fn directed_liquid_edge(
    first_id: usize,
    first: &PatchEdgeContractSettings,
    second_id: usize,
    second: &PatchEdgeContractSettings,
) -> Option<(usize, usize)> {
    let (PatchEdgeContractSettings::Shared(first), PatchEdgeContractSettings::Shared(second)) =
        (first, second)
    else {
        return None;
    };
    match (first.liquid, second.liquid) {
        (EdgeLiquidSettings::Outlet(_), EdgeLiquidSettings::Inlet(_)) => {
            Some((first_id, second_id))
        }
        (EdgeLiquidSettings::Inlet(_), EdgeLiquidSettings::Outlet(_)) => {
            Some((second_id, first_id))
        }
        _ => None,
    }
}

fn directed_graph_is_acyclic(
    node_count: usize,
    edges: impl IntoIterator<Item = (usize, usize)>,
) -> bool {
    let mut outgoing = vec![Vec::new(); node_count];
    let mut indegree = vec![0_usize; node_count];
    for (source, sink) in edges {
        let (Some(source_edges), Some(sink_indegree)) =
            (outgoing.get_mut(source), indegree.get_mut(sink))
        else {
            return false;
        };
        source_edges.push(sink);
        *sink_indegree += 1;
    }

    let mut ready: VecDeque<_> = indegree
        .iter()
        .enumerate()
        .filter_map(|(node, degree)| (*degree == 0).then_some(node))
        .collect();
    let mut visited = 0;
    while let Some(node) = ready.pop_front() {
        visited += 1;
        let Some(sinks) = outgoing.get(node) else {
            return false;
        };
        for sink in sinks {
            let Some(degree) = indegree.get_mut(*sink) else {
                return false;
            };
            let Some(next_degree) = degree.checked_sub(1) else {
                return false;
            };
            *degree = next_degree;
            if next_degree == 0 {
                ready.push_back(*sink);
            }
        }
    }
    visited == node_count
}

fn is_stable_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|first| first.is_ascii_lowercase())
        && chars.all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '_' | '-' | '.')
        })
}

impl V3HillsSettings {
    fn validate(&self, grid_radius: u32) -> Result<(), String> {
        if !(12..=40).contains(&grid_radius) {
            return Err("procedural V3 Hills requires grid_radius from 12 through 40".to_owned());
        }
        if self.valley_level < 5 {
            return Err("V3 Hills valley_level must leave room for bedrock and strata".to_owned());
        }
        if !(1..=12).contains(&self.max_relief) {
            return Err("V3 Hills max_relief must be between 1 and 12".to_owned());
        }
        let Some(highest_surface) = self.valley_level.checked_add(self.max_relief) else {
            return Err("V3 Hills level relationship overflows Level".to_owned());
        };
        if highest_surface > MAX_PROCEDURAL_LEVEL {
            return Err(format!(
                "V3 Hills surfaces cannot exceed level {MAX_PROCEDURAL_LEVEL}"
            ));
        }
        if !(1..=6).contains(&self.hills_per_bank) {
            return Err("V3 Hills hills_per_bank must be between 1 and 6".to_owned());
        }
        Ok(())
    }
}

impl V3SkyIslandsSettings {
    fn validate(&self, grid_radius: u32) -> Result<(), String> {
        self.ground.validate(grid_radius)?;
        if self.min_clearance < 8 {
            return Err("V3 SkyIslands min_clearance must be at least 8".to_owned());
        }
        if !(15..=25).contains(&self.upper_coverage_percent) {
            return Err(
                "V3 SkyIslands upper_coverage_percent must be between 15 and 25".to_owned(),
            );
        }
        let highest_ground = self
            .ground
            .valley_level
            .checked_add(self.ground.max_relief)
            .ok_or_else(|| "V3 SkyIslands ground relationship overflows Level".to_owned())?;
        let highest_reserved = highest_ground
            .checked_add(self.min_clearance)
            .and_then(|level| level.checked_add(SKY_UPPER_VERTICAL_BUDGET))
            .ok_or_else(|| "V3 SkyIslands level relationship overflows Level".to_owned())?;
        if highest_reserved > MAX_PROCEDURAL_LEVEL {
            return Err(format!(
                "V3 SkyIslands reserved volume cannot exceed level {MAX_PROCEDURAL_LEVEL}"
            ));
        }
        Ok(())
    }
}

impl V3MountainsSettings {
    fn validate(&self, grid_radius: u32) -> Result<(), String> {
        if !(12..=40).contains(&grid_radius) {
            return Err(
                "procedural V3 Mountains requires grid_radius from 12 through 40".to_owned(),
            );
        }
        if self.base_level < 5 {
            return Err("V3 Mountains base_level must leave room for strata".to_owned());
        }
        if !(14..=32).contains(&self.relief) {
            return Err("V3 Mountains relief must be between 14 and 32".to_owned());
        }
        if !(3..=7).contains(&self.peak_count) {
            return Err("V3 Mountains peak_count must be between 3 and 7".to_owned());
        }
        let Some(highest_surface) = self.base_level.checked_add(self.relief) else {
            return Err("V3 Mountains level relationship overflows Level".to_owned());
        };
        if highest_surface > MAX_PROCEDURAL_LEVEL {
            return Err(format!(
                "V3 Mountains surfaces cannot exceed level {MAX_PROCEDURAL_LEVEL}"
            ));
        }
        Ok(())
    }
}

impl V3CavesSettings {
    fn validate(&self, grid_radius: u32) -> Result<(), String> {
        if !(12..=40).contains(&grid_radius) {
            return Err("procedural V3 Caves requires grid_radius from 12 through 40".to_owned());
        }
        if !(14..=17).contains(&self.surface_level) {
            return Err("V3 Caves surface_level must be between 14 and 17".to_owned());
        }
        if !(6..=8).contains(&self.cave_floor_level) {
            return Err("V3 Caves cave_floor_level must be between 6 and 8".to_owned());
        }
        if !(6..=12).contains(&self.chamber_count) {
            return Err("V3 Caves chamber_count must be between 6 and 12".to_owned());
        }
        let Some(vertical_space) = self.surface_level.checked_sub(self.cave_floor_level) else {
            return Err("V3 Caves cave_floor_level must be below the surface".to_owned());
        };
        if vertical_space < 11 {
            return Err(
                "V3 Caves need +0/+2/+4 floor tiers, four clear chamber levels, and at least \
                 three roof levels"
                    .to_owned(),
            );
        }
        Ok(())
    }
}

impl V3VolcanoSettings {
    fn validate(&self, grid_radius: u32) -> Result<(), String> {
        if !(12..=RING19_RADIUS).contains(&grid_radius) {
            return Err(format!(
                "procedural V3 Volcano requires grid_radius from 12 through {RING19_RADIUS}"
            ));
        }
        if self.base_level < 5 {
            return Err("V3 Volcano base_level must leave room for strata".to_owned());
        }
        if !(20..=48).contains(&self.summit_relief) {
            return Err("V3 Volcano summit_relief must be between 20 and 48".to_owned());
        }
        if !(20..=30).contains(&self.massif_coverage_percent) {
            return Err("V3 Volcano massif_coverage_percent must be between 20 and 30".to_owned());
        }
        if !(4..=12).contains(&self.bridge_clearance) {
            return Err("V3 Volcano bridge_clearance must be between 4 and 12".to_owned());
        }
        let Some(highest_surface) = self.base_level.checked_add(self.summit_relief) else {
            return Err("V3 Volcano level relationship overflows Level".to_owned());
        };
        if highest_surface > MAX_PROCEDURAL_LEVEL {
            return Err(format!(
                "V3 Volcano surfaces cannot exceed level {MAX_PROCEDURAL_LEVEL}"
            ));
        }
        Ok(())
    }
}

impl V3DeepForestSettings {
    fn validate(&self, grid_radius: u32) -> Result<(), String> {
        validate_vegetation_landform(grid_radius, self.base_level, self.max_relief, "DeepForest")?;
        if !(28..=32).contains(&self.blocker_coverage_percent) {
            return Err(
                "V3 DeepForest blocker_coverage_percent must be between 28 and 32".to_owned(),
            );
        }
        if self.clearing_count != 3 {
            return Err("V3 DeepForest clearing_count must be exactly 3".to_owned());
        }
        Ok(())
    }
}

impl V3PrairieSettings {
    fn validate(&self, grid_radius: u32) -> Result<(), String> {
        validate_vegetation_landform(grid_radius, self.base_level, self.max_relief, "Prairie")?;
        if !(65..=75).contains(&self.grass_coverage_percent) {
            return Err("V3 Prairie grass_coverage_percent must be between 65 and 75".to_owned());
        }
        Ok(())
    }
}

fn validate_vegetation_landform(
    grid_radius: u32,
    base_level: Level,
    max_relief: Level,
    recipe: &str,
) -> Result<(), String> {
    if !(12..=RING19_RADIUS).contains(&grid_radius) {
        return Err(format!(
            "procedural V3 {recipe} requires grid_radius from 12 through {RING19_RADIUS}"
        ));
    }
    if base_level < 5 {
        return Err(format!("V3 {recipe} base_level must leave room for strata"));
    }
    if !(1..=12).contains(&max_relief) {
        return Err(format!("V3 {recipe} max_relief must be between 1 and 12"));
    }
    let Some(highest_surface) = base_level.checked_add(max_relief) else {
        return Err(format!("V3 {recipe} level relationship overflows Level"));
    };
    if highest_surface > MAX_PROCEDURAL_LEVEL {
        return Err(format!(
            "V3 {recipe} surfaces cannot exceed level {MAX_PROCEDURAL_LEVEL}"
        ));
    }
    Ok(())
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
        if !(14..=24).contains(&self.relief) {
            return Err("Mountains relief must be between 14 and 24".to_owned());
        }
        if !(3..=7).contains(&self.peak_count) {
            return Err("Mountains peak_count must be between 3 and 7".to_owned());
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
        if !(6..=12).contains(&self.chamber_count) {
            return Err("Caves chamber_count must be between 6 and 12".to_owned());
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
    const V3_FORT_RON: &str = include_str!("../../../assets/config/worlds/procedural-fort.ron");
    const V3_RING7_RON: &str = include_str!("../../../assets/config/worlds/procedural-ring7.ron");
    const V3_RING19_RON: &str =
        include_str!("../../../assets/config/worlds/procedural-two-rings.ron");
    const V1_HILLS_RON: &str = r#"
(
    grid_radius: 12,
    level_height: 0.4,
    terrain: Procedural((
        generator_version: 1,
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
    )),
)
"#;
    const V3_SINGLE_RON: &str = r#"
(
    grid_radius: 12,
    level_height: 0.4,
    terrain: Procedural((
        generator_version: 3,
        layout: Single((
            environment: TemperateGrassland,
            recipe: Hills((
                valley_level: 15,
                max_relief: 8,
                hills_per_bank: 3,
            )),
            overlays: [
                (name: "river.main", kind: Liquid),
                (name: "grassland", kind: Vegetation),
            ],
            mask: WholeWorld,
            edges: (
                east: WorldBoundary,
                south_east: WorldBoundary,
                south_west: WorldBoundary,
                west: WorldBoundary,
                north_west: WorldBoundary,
                north_east: WorldBoundary,
            ),
        )),
    )),
)
"#;

    fn world_boundary_edges() -> PatchEdgesSettings {
        PatchEdgesSettings {
            east: PatchEdgeContractSettings::WorldBoundary,
            south_east: PatchEdgeContractSettings::WorldBoundary,
            south_west: PatchEdgeContractSettings::WorldBoundary,
            west: PatchEdgeContractSettings::WorldBoundary,
            north_west: PatchEdgeContractSettings::WorldBoundary,
            north_east: PatchEdgeContractSettings::WorldBoundary,
        }
    }

    fn dry_shared_edge() -> PatchEdgeContractSettings {
        PatchEdgeContractSettings::Shared(SharedEdgeSettings {
            elevation: EdgeElevationSettings {
                preferred: 15,
                min: 14,
                max: 16,
            },
            walker: WalkerPortSettings { count: 1, width: 2 },
            liquid: EdgeLiquidSettings::Dry,
            approach_depth: 2,
        })
    }

    fn generated_patch(environment: V3EnvironmentSettings, recipe: V3RecipeSettings) -> PatchSpec {
        PatchSpec {
            environment,
            recipe,
            overlays: Vec::new(),
            mask: PatchMaskSettings::GeneratedRegion,
            edges: world_boundary_edges(),
        }
    }

    fn valid_ring7() -> V3Ring7Settings {
        let hills = V3HillsSettings {
            valley_level: 15,
            max_relief: 8,
            hills_per_bank: 3,
        };
        let mut ring = V3Ring7Settings {
            center: generated_patch(
                V3EnvironmentSettings::TemperateGrassland,
                V3RecipeSettings::Hills(hills.clone()),
            ),
            mountains: generated_patch(
                V3EnvironmentSettings::Frozen,
                V3RecipeSettings::Mountains(V3MountainsSettings {
                    base_level: 15,
                    relief: 18,
                    peak_count: 5,
                }),
            ),
            waterfall: generated_patch(
                V3EnvironmentSettings::TemperateGrassland,
                V3RecipeSettings::Waterfall(V3WaterfallSettings),
            ),
            forest: generated_patch(
                V3EnvironmentSettings::TemperateGrassland,
                V3RecipeSettings::Forest(V3ForestSettings),
            ),
            fort: generated_patch(
                V3EnvironmentSettings::TemperateGrassland,
                V3RecipeSettings::Fort(V3FortSettings),
            ),
            caves: generated_patch(
                V3EnvironmentSettings::Rocky,
                V3RecipeSettings::Caves(V3CavesSettings {
                    surface_level: 17,
                    cave_floor_level: 6,
                    chamber_count: 9,
                }),
            ),
            sky_islands: generated_patch(
                V3EnvironmentSettings::TemperateGrassland,
                V3RecipeSettings::SkyIslands(V3SkyIslandsSettings {
                    ground: hills,
                    min_clearance: 14,
                    upper_coverage_percent: 20,
                }),
            ),
        };

        let shared = dry_shared_edge();
        ring.center.edges.north_east = shared.clone();
        ring.mountains.edges.south_west = shared.clone();
        ring.center.edges.east = shared.clone();
        ring.waterfall.edges.west = shared.clone();
        ring.center.edges.south_east = shared.clone();
        ring.forest.edges.north_west = shared.clone();
        ring.center.edges.south_west = shared.clone();
        ring.fort.edges.north_east = shared.clone();
        ring.center.edges.west = shared.clone();
        ring.caves.edges.east = shared.clone();
        ring.center.edges.north_west = shared.clone();
        ring.sky_islands.edges.south_east = shared.clone();

        ring.mountains.edges.south_east = shared.clone();
        ring.waterfall.edges.north_west = shared.clone();
        ring.waterfall.edges.south_west = shared.clone();
        ring.forest.edges.north_east = shared.clone();
        ring.forest.edges.west = shared.clone();
        ring.fort.edges.east = shared.clone();
        ring.fort.edges.north_west = shared.clone();
        ring.caves.edges.south_east = shared.clone();
        ring.caves.edges.north_east = shared.clone();
        ring.sky_islands.edges.south_west = shared.clone();
        ring.sky_islands.edges.east = shared.clone();
        ring.mountains.edges.west = shared;
        ring
    }

    fn valid_ring19() -> V3Ring19Settings {
        let region = Ring19RegionSettings {
            environment: V3EnvironmentSettings::TemperateGrassland,
            recipe: V3RecipeSettings::Hills(V3HillsSettings {
                valley_level: 15,
                max_relief: 8,
                hills_per_bank: 3,
            }),
            overlays: Vec::new(),
            rotation_turns: 0,
        };
        V3Ring19Settings {
            regions: vec![region; V3_RING19_REGION_COUNT],
            seam_defaults: SharedEdgeSettings {
                elevation: EdgeElevationSettings {
                    preferred: 15,
                    min: 14,
                    max: 16,
                },
                walker: WalkerPortSettings { count: 2, width: 2 },
                liquid: EdgeLiquidSettings::Dry,
                approach_depth: 3,
            },
            liquid_connections: vec![Ring19LiquidConnectionSettings {
                source_region: 1,
                sink_region: 7,
                width: 3,
                level: 16,
            }],
            boundary_outlets: vec![Ring19BoundaryOutletSettings {
                source_region: 7,
                side: Ring19BoundarySide::NorthEast,
                width: 3,
                level: 16,
            }],
        }
    }

    fn shipped_ring19_settings() -> MapSettings {
        ron::from_str(V3_RING19_RON).expect("the shipped Two Rings settings should parse")
    }

    fn ring19_mut(settings: &mut MapSettings) -> &mut V3Ring19Settings {
        let TerrainSettings::Procedural(ProceduralSettings::V3(ProceduralV3Settings {
            layout: V3LayoutSettings::Ring19(ring),
        })) = &mut settings.terrain
        else {
            panic!("the shipped Two Rings settings should use V3 Ring19");
        };
        ring
    }

    fn assert_two_rings_contract_rejected(settings: &MapSettings, expected: &str) {
        let TerrainSettings::Procedural(ProceduralSettings::V3(ProceduralV3Settings {
            layout: V3LayoutSettings::Ring19(ring),
        })) = &settings.terrain
        else {
            panic!("the mutated Two Rings settings should retain V3 Ring19");
        };
        ring.validate_structure()
            .expect("the mutation should remain structurally valid");
        let error = settings
            .validate()
            .expect_err("the fixed Two Rings contract should reject the mutation");
        assert!(
            error.contains(expected),
            "expected {expected:?} in validation error {error:?}"
        );
    }

    fn valid_explicit_ring7() -> V3Ring7Settings {
        const RADIUS: u32 = 33;
        const OFFSET: i32 = 22;

        let centers = [
            HexCoord::ORIGIN,
            HexCoord::new_cubic(OFFSET, -OFFSET, 0),
            HexCoord::new_cubic(OFFSET, 0, -OFFSET),
            HexCoord::new_cubic(0, OFFSET, -OFFSET),
            HexCoord::new_cubic(-OFFSET, OFFSET, 0),
            HexCoord::new_cubic(-OFFSET, 0, OFFSET),
            HexCoord::new_cubic(0, -OFFSET, OFFSET),
        ];
        let mut masks: [Vec<CubeCoord>; 7] = std::array::from_fn(|_| Vec::new());
        for coord in HexCoord::ORIGIN.within_radius(RADIUS) {
            let owner = centers
                .iter()
                .enumerate()
                .min_by_key(|(index, center)| (coord.distance(**center), *index))
                .map(|(index, _)| index)
                .expect("the fixed Ring7 has patch centers");
            masks
                .get_mut(owner)
                .expect("a selected patch center has a matching mask")
                .push(CubeCoord {
                    x: coord.x(),
                    y: coord.y(),
                    z: coord.z(),
                });
        }

        let mut ring = valid_ring7();
        for (index, mask) in masks.into_iter().enumerate() {
            let patch = match index {
                0 => &mut ring.center,
                1 => &mut ring.mountains,
                2 => &mut ring.waterfall,
                3 => &mut ring.forest,
                4 => &mut ring.fort,
                5 => &mut ring.caves,
                6 => &mut ring.sky_islands,
                _ => unreachable!("fixed Ring7 patch count"),
            };
            patch.mask = PatchMaskSettings::Explicit(mask);
        }
        ring
    }

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
    fn shipped_procedural_variants_use_the_intended_generator_versions() {
        for (ron, expected_version) in [
            (
                include_str!("../../../assets/config/worlds/procedural-hills.ron"),
                3,
            ),
            (
                include_str!("../../../assets/config/worlds/procedural-frozen.ron"),
                3,
            ),
            (
                include_str!("../../../assets/config/worlds/procedural-volcanic.ron"),
                3,
            ),
            (
                include_str!("../../../assets/config/worlds/procedural-sky-islands.ron"),
                3,
            ),
            (
                include_str!("../../../assets/config/worlds/procedural-mountains.ron"),
                3,
            ),
            (
                include_str!("../../../assets/config/worlds/procedural-caves.ron"),
                3,
            ),
            (
                include_str!("../../../assets/config/worlds/procedural-waterfall.ron"),
                3,
            ),
            (
                include_str!("../../../assets/config/worlds/procedural-forest.ron"),
                3,
            ),
            (
                include_str!("../../../assets/config/worlds/procedural-deep-forest.ron"),
                3,
            ),
            (
                include_str!("../../../assets/config/worlds/procedural-prairie.ron"),
                3,
            ),
            (V3_FORT_RON, 3),
            (V3_RING7_RON, 3),
            (V3_RING19_RON, 3),
        ] {
            let settings: MapSettings =
                ron::from_str(ron).expect("shipped procedural RON should parse");
            let TerrainSettings::Procedural(procedural) = settings.terrain else {
                panic!("the shipped preset should be Procedural")
            };
            assert_eq!(procedural.generator_version(), expected_version);
            assert_eq!(
                matches!(&procedural, ProceduralSettings::V2(_)),
                expected_version == 2
            );
            assert_eq!(
                matches!(&procedural, ProceduralSettings::V3(_)),
                expected_version == 3
            );
        }
    }

    #[test]
    fn shipped_v3_ring7_preserves_exact_shared_contracts() {
        let settings: MapSettings =
            ron::from_str(V3_RING7_RON).expect("the shipped Ring7 RON should parse");
        settings
            .validate()
            .expect("the shipped Ring7 settings should validate");
        assert_eq!(settings.grid_radius, 33);
        assert_eq!(settings.level_height.to_bits(), 0.4_f32.to_bits());

        let TerrainSettings::Procedural(ProceduralSettings::V3(ProceduralV3Settings {
            layout: V3LayoutSettings::Ring7(ring),
        })) = settings.terrain
        else {
            panic!("the shipped composite should use the V3 Ring7 layout")
        };

        let all_edges = [
            ring.center.edges.edges(),
            ring.mountains.edges.edges(),
            ring.waterfall.edges.edges(),
            ring.forest.edges.edges(),
            ring.fort.edges.edges(),
            ring.caves.edges.edges(),
            ring.sky_islands.edges.edges(),
        ];
        let shared = all_edges
            .into_iter()
            .flatten()
            .filter_map(|edge| {
                let PatchEdgeContractSettings::Shared(shared) = edge else {
                    return None;
                };
                Some(shared)
            })
            .collect::<Vec<_>>();
        assert_eq!(shared.len(), 24, "twelve reciprocal internal seams");
        for contract in &shared {
            assert_eq!(
                contract.elevation,
                EdgeElevationSettings {
                    preferred: 15,
                    min: 14,
                    max: 16,
                }
            );
            assert_eq!(contract.walker, WalkerPortSettings { count: 2, width: 2 });
            assert_eq!(contract.approach_depth, 3);
        }

        let PatchEdgeContractSettings::Shared(center_east) = &ring.center.edges.east else {
            panic!("the center east edge should be shared")
        };
        assert_eq!(
            center_east.liquid,
            EdgeLiquidSettings::Inlet(EdgeLiquidPortSettings { width: 3 })
        );
        let PatchEdgeContractSettings::Shared(waterfall_west) = &ring.waterfall.edges.west else {
            panic!("the Waterfall west edge should be shared")
        };
        assert_eq!(
            waterfall_west.liquid,
            EdgeLiquidSettings::Outlet(EdgeLiquidPortSettings { width: 3 })
        );
        let PatchEdgeContractSettings::Shared(center_west) = &ring.center.edges.west else {
            panic!("the center west edge should be shared")
        };
        assert_eq!(
            center_west.liquid,
            EdgeLiquidSettings::Outlet(EdgeLiquidPortSettings { width: 3 })
        );
        let PatchEdgeContractSettings::Shared(caves_east) = &ring.caves.edges.east else {
            panic!("the Caves east edge should be shared")
        };
        assert_eq!(
            caves_east.liquid,
            EdgeLiquidSettings::Inlet(EdgeLiquidPortSettings { width: 3 })
        );

        assert_eq!(
            shared
                .iter()
                .filter(|contract| matches!(contract.liquid, EdgeLiquidSettings::Dry))
                .count(),
            20,
            "every other internal seam should be dry"
        );
        assert!(
            ring.mountains.edges.all_liquids_dry(),
            "Mountains must remain entirely dry"
        );
    }

    #[test]
    fn v3_ring19_structure_enforces_slots_rotation_and_liquid_topology() {
        valid_ring19()
            .validate_structure()
            .expect("the structural Ring19 contract should validate");

        let mut wrong_count = valid_ring19();
        wrong_count.regions.pop();
        assert!(
            wrong_count.validate_structure().is_err(),
            "Ring19 requires exactly nineteen semantic slots"
        );

        let mut wrong_rotation = valid_ring19();
        wrong_rotation
            .regions
            .get_mut(18)
            .expect("the fixture has nineteen regions")
            .rotation_turns = 6;
        assert!(
            wrong_rotation.validate_structure().is_err(),
            "region-local rotation is bounded to six exact turns"
        );

        let mut non_adjacent = valid_ring19();
        non_adjacent
            .liquid_connections
            .first_mut()
            .expect("the fixture has one liquid connection")
            .sink_region = 13;
        assert!(
            non_adjacent.validate_structure().is_err(),
            "liquid handoffs must use an actual internal seam"
        );

        let mut interior_outlet = valid_ring19();
        interior_outlet
            .boundary_outlets
            .first_mut()
            .expect("the fixture has one boundary outlet")
            .side = Ring19BoundarySide::SouthWest;
        assert!(
            interior_outlet.validate_structure().is_err(),
            "a boundary outlet must name an exact outer side"
        );

        for invalid_level in [2, MAX_PROCEDURAL_LEVEL + 1] {
            let mut invalid_connection_level = valid_ring19();
            invalid_connection_level
                .liquid_connections
                .first_mut()
                .expect("the fixture has one liquid connection")
                .level = invalid_level;
            assert!(
                invalid_connection_level.validate_structure().is_err(),
                "internal liquid level {invalid_level} must fail closed"
            );

            let mut invalid_outlet_level = valid_ring19();
            invalid_outlet_level
                .boundary_outlets
                .first_mut()
                .expect("the fixture has one boundary outlet")
                .level = invalid_level;
            assert!(
                invalid_outlet_level.validate_structure().is_err(),
                "boundary liquid level {invalid_level} must fail closed"
            );
        }

        let mut dangling = valid_ring19();
        let outlet = dangling
            .boundary_outlets
            .first_mut()
            .expect("the fixture has one boundary outlet");
        outlet.source_region = 9;
        outlet.side = Ring19BoundarySide::East;
        assert!(
            dangling.validate_structure().is_err(),
            "every internal liquid component must terminate at a boundary outlet"
        );

        let mut boundary_only = valid_ring19();
        boundary_only.liquid_connections.clear();
        boundary_only
            .validate_structure()
            .expect("an independent boundary-only Volcano outlet is valid");

        let mut cycle = valid_ring19();
        cycle.liquid_connections = vec![
            Ring19LiquidConnectionSettings {
                source_region: 0,
                sink_region: 1,
                width: 3,
                level: 16,
            },
            Ring19LiquidConnectionSettings {
                source_region: 1,
                sink_region: 2,
                width: 3,
                level: 16,
            },
            Ring19LiquidConnectionSettings {
                source_region: 2,
                sink_region: 0,
                width: 3,
                level: 16,
            },
        ];
        assert!(
            cycle.validate_structure().is_err(),
            "the directed liquid graph must remain acyclic"
        );
    }

    #[test]
    fn shipped_two_rings_contract_rejects_structurally_valid_drift() {
        let shipped = shipped_ring19_settings();
        shipped
            .validate()
            .expect("the shipped Two Rings settings should validate");
        assert_eq!(shipped.grid_radius, RING19_RADIUS);

        let mut wrong_radius = shipped_ring19_settings();
        wrong_radius.grid_radius = RING19_RADIUS - 1;
        assert_two_rings_contract_rejected(&wrong_radius, "radius exactly 55");

        let mut wrong_recipe = shipped_ring19_settings();
        let forest_recipe = {
            let ring = ring19_mut(&mut wrong_recipe);
            ring.regions
                .get(2)
                .expect("the shipped fixture has slot 2")
                .recipe
                .clone()
        };
        ring19_mut(&mut wrong_recipe)
            .regions
            .get_mut(3)
            .expect("the shipped fixture has slot 3")
            .recipe = forest_recipe;
        assert_two_rings_contract_rejected(&wrong_recipe, "slot 3");

        let mut wrong_rotation = shipped_ring19_settings();
        ring19_mut(&mut wrong_rotation)
            .regions
            .get_mut(2)
            .expect("the shipped fixture has slot 2")
            .rotation_turns = 3;
        assert_two_rings_contract_rejected(&wrong_rotation, "slot 2");

        let mut missing_cave_overlay = shipped_ring19_settings();
        ring19_mut(&mut missing_cave_overlay)
            .regions
            .get_mut(14)
            .expect("the shipped fixture has the Caves slot")
            .overlays
            .clear();
        assert_two_rings_contract_rejected(&missing_cave_overlay, "Caves slot");

        let mut wrong_seam = shipped_ring19_settings();
        ring19_mut(&mut wrong_seam)
            .seam_defaults
            .elevation
            .preferred = 16;
        assert_two_rings_contract_rejected(&wrong_seam, "seam defaults");

        let mut narrow_connection = shipped_ring19_settings();
        ring19_mut(&mut narrow_connection)
            .liquid_connections
            .get_mut(0)
            .expect("the shipped fixture has an internal liquid connection")
            .width = 2;
        assert_two_rings_contract_rejected(&narrow_connection, "internal hydrology");

        let mut missing_connection = shipped_ring19_settings();
        ring19_mut(&mut missing_connection)
            .liquid_connections
            .remove(0);
        assert_two_rings_contract_rejected(&missing_connection, "internal hydrology");

        let mut missing_lava_outlet = shipped_ring19_settings();
        ring19_mut(&mut missing_lava_outlet)
            .boundary_outlets
            .retain(|outlet| outlet.source_region != 15);
        assert_two_rings_contract_rejected(&missing_lava_outlet, "boundary hydrology");
    }

    #[test]
    fn additive_v3_recipe_settings_validate_without_loosening_environments() {
        let volcano = V3RecipeSettings::Volcano(V3VolcanoSettings {
            base_level: 12,
            summit_relief: 24,
            massif_coverage_percent: 25,
            bridge_clearance: 4,
        });
        validate_v3_recipe(&volcano, V3EnvironmentSettings::Volcanic, 20)
            .expect("the canonical Volcano settings should validate");
        assert!(
            validate_v3_recipe(&volcano, V3EnvironmentSettings::Frozen, 20).is_err(),
            "Volcano remains volcanic-only"
        );

        let deep_forest = V3RecipeSettings::DeepForest(V3DeepForestSettings {
            base_level: 12,
            max_relief: 6,
            blocker_coverage_percent: 30,
            clearing_count: 3,
        });
        validate_v3_recipe(&deep_forest, V3EnvironmentSettings::TemperateGrassland, 20)
            .expect("the canonical Deep Forest settings should validate");
        assert!(
            validate_v3_recipe(&deep_forest, V3EnvironmentSettings::Rocky, 20).is_err(),
            "Deep Forest remains temperate-only"
        );

        let prairie = V3RecipeSettings::Prairie(V3PrairieSettings {
            base_level: 12,
            max_relief: 5,
            grass_coverage_percent: 70,
        });
        validate_v3_recipe(&prairie, V3EnvironmentSettings::TemperateGrassland, 20)
            .expect("the canonical Prairie settings should validate");
        assert!(
            validate_v3_recipe(&prairie, V3EnvironmentSettings::Frozen, 20).is_err(),
            "Prairie remains temperate-only"
        );
    }

    #[test]
    fn v1_keeps_its_flat_external_ron_shape() {
        let source = V1_HILLS_RON;
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
        let source = V1_HILLS_RON.replacen(
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
            error.to_string().contains("expected 1, 2, or 3"),
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
    fn v3_single_uses_the_strict_flat_layout_wire() {
        let settings: MapSettings =
            ron::from_str(V3_SINGLE_RON).expect("a valid V3 Single layout should deserialize");
        let TerrainSettings::Procedural(procedural) = settings.terrain else {
            panic!("the parsed preset should be Procedural")
        };
        assert_eq!(procedural.generator_version(), 3);
        let ProceduralSettings::V3(v3) = procedural else {
            panic!("generator_version 3 should dispatch to V3")
        };
        let V3LayoutSettings::Single(patch) = v3.layout else {
            panic!("the parsed V3 layout should be Single")
        };
        assert!(matches!(patch.recipe, V3RecipeSettings::Hills(_)));
        assert_eq!(patch.overlays.len(), 2);

        let wrapped = V3_SINGLE_RON
            .replacen("Procedural((", "Procedural(V3((", 1)
            .replacen("\n    )),\n)", "\n    ))),\n)", 1);
        ron::from_str::<MapSettings>(&wrapped)
            .expect_err("the internal V3 enum must not leak into external RON");
    }

    #[test]
    fn v3_rejects_unknown_and_cross_version_fields() {
        let unknown = V3_SINGLE_RON.replacen(
            "generator_version: 3,",
            "generator_version: 3,\n        typoed_setting: 42,",
            1,
        );
        let error =
            ron::from_str::<MapSettings>(&unknown).expect_err("V3 top-level fields must be strict");
        assert!(
            error.to_string().contains("typoed_setting"),
            "unexpected error: {error}"
        );

        let mixed = V3_SINGLE_RON.replacen(
            "generator_version: 3,",
            "generator_version: 3,\n        environment: TemperateGrassland,",
            1,
        );
        let error =
            ron::from_str::<MapSettings>(&mixed).expect_err("V3 must reject V1/V2 terrain axes");
        assert!(
            error.to_string().contains("uses layout instead"),
            "unexpected error: {error}"
        );

        let nested_unknown = V3_SINGLE_RON.replacen(
            "hills_per_bank: 3,",
            "hills_per_bank: 3,\n                barrier_half_width: 1,",
            1,
        );
        ron::from_str::<MapSettings>(&nested_unknown)
            .expect_err("V3 recipe payloads must reject derived or misspelled fields");
    }

    #[test]
    fn v3_single_validates_radius_mask_edges_and_overlays() {
        for invalid_radius in [11, 41] {
            let source = V3_SINGLE_RON.replacen(
                "grid_radius: 12",
                &format!("grid_radius: {invalid_radius}"),
                1,
            );
            let error = ron::from_str::<MapSettings>(&source)
                .expect_err("V3 Single radius outside 12 through 40 should fail");
            assert!(
                error.to_string().contains("12 through 40"),
                "unexpected error: {error}"
            );
        }

        let generated_mask = V3_SINGLE_RON.replacen("mask: WholeWorld", "mask: GeneratedRegion", 1);
        let error = ron::from_str::<MapSettings>(&generated_mask)
            .expect_err("a Single layout cannot defer its only footprint");
        assert!(
            error
                .to_string()
                .contains("WholeWorld or a connected Explicit"),
            "unexpected error: {error}"
        );

        let shared_edge = V3_SINGLE_RON.replacen(
            "east: WorldBoundary",
            "east: Shared((
                elevation: (preferred: 15, min: 14, max: 16),
                walker: (count: 1, width: 2),
                liquid: Dry,
                approach_depth: 2,
            ))",
            1,
        );
        let error = ron::from_str::<MapSettings>(&shared_edge)
            .expect_err("a Single patch cannot expose an unresolved shared edge");
        assert!(
            error.to_string().contains("all be WorldBoundary"),
            "unexpected error: {error}"
        );

        let duplicate_overlay = V3_SINGLE_RON.replacen(
            "(name: \"grassland\", kind: Vegetation),",
            "(name: \"river.main\", kind: Vegetation),",
            1,
        );
        let error = ron::from_str::<MapSettings>(&duplicate_overlay)
            .expect_err("overlay identifiers must be unique within a patch");
        assert!(
            error.to_string().contains("duplicate overlay"),
            "unexpected error: {error}"
        );

        let unstable_overlay = V3_SINGLE_RON.replacen("\"river.main\"", "\"River main\"", 1);
        let error = ron::from_str::<MapSettings>(&unstable_overlay)
            .expect_err("overlay identifiers must remain stable");
        assert!(
            error.to_string().contains("lowercase stable identifier"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn v3_explicit_masks_are_valid_cube_coordinates_unique_and_connected() {
        let connected = V3_SINGLE_RON.replacen(
            "mask: WholeWorld",
            "mask: Explicit([
                (x: 0, y: 0, z: 0),
                (x: 1, y: -1, z: 0),
                (x: 1, y: 0, z: -1),
            ])",
            1,
        );
        ron::from_str::<MapSettings>(&connected)
            .expect("a connected in-bounds Explicit Single mask should deserialize");

        let disconnected = connected.replacen("(x: 1, y: 0, z: -1)", "(x: 4, y: 0, z: -4)", 1);
        let error = ron::from_str::<MapSettings>(&disconnected)
            .expect_err("disconnected Explicit masks must fail");
        assert!(
            error.to_string().contains("must be connected"),
            "unexpected error: {error}"
        );

        let duplicate = connected.replacen("(x: 1, y: 0, z: -1)", "(x: 1, y: -1, z: 0)", 1);
        let error =
            ron::from_str::<MapSettings>(&duplicate).expect_err("duplicate cells must fail");
        assert!(
            error.to_string().contains("duplicate coordinate"),
            "unexpected error: {error}"
        );

        let invalid_cube = connected.replacen("(x: 1, y: 0, z: -1)", "(x: 1, y: 0, z: 0)", 1);
        let error = ron::from_str::<MapSettings>(&invalid_cube)
            .expect_err("invalid cube coordinates must fail");
        assert!(
            error.to_string().contains("must sum to zero"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn v3_recipe_environment_combinations_are_checked_before_generation() {
        for (recipe, environment, expected) in [
            (
                "Hills((valley_level: 15, max_relief: 8, hills_per_bank: 3))",
                "Rocky",
                "does not support",
            ),
            (
                "SkyIslands((ground: (valley_level: 15, max_relief: 8, hills_per_bank: 3), min_clearance: 14, upper_coverage_percent: 20))",
                "Volcanic",
                "requires TemperateGrassland or Frozen",
            ),
            (
                "Mountains((base_level: 15, relief: 18, peak_count: 5))",
                "TemperateGrassland",
                "requires the Frozen",
            ),
            (
                "Caves((surface_level: 17, cave_floor_level: 6, chamber_count: 9))",
                "Frozen",
                "requires the Rocky",
            ),
            (
                "Waterfall(())",
                "Frozen",
                "currently require TemperateGrassland",
            ),
        ] {
            let source = V3_SINGLE_RON
                .replacen("environment: TemperateGrassland", &format!("environment: {environment}"), 1)
                .replacen(
                    "Hills((\n                valley_level: 15,\n                max_relief: 8,\n                hills_per_bank: 3,\n            ))",
                    recipe,
                    1,
                );
            let error = ron::from_str::<MapSettings>(&source)
                .expect_err("an unsupported V3 recipe/environment pair should fail");
            assert!(
                error.to_string().contains(expected),
                "unexpected error for {recipe}: {error}"
            );
        }
    }

    #[test]
    fn v3_ring7_enforces_roster_radius_masks_and_reciprocal_edges() {
        let settings = MapSettings {
            grid_radius: 33,
            level_height: 0.4,
            terrain: TerrainSettings::Procedural(ProceduralSettings::V3(ProceduralV3Settings {
                layout: V3LayoutSettings::Ring7(valid_ring7()),
            })),
        };
        settings
            .validate()
            .expect("the fixed generated Ring7 contract should validate");

        let mut invalid_radius = settings.clone();
        invalid_radius.grid_radius = 32;
        let error = invalid_radius
            .validate()
            .expect_err("Ring7 must reserve its radius-33 footprint");
        assert!(error.contains("exactly 33"), "unexpected error: {error}");

        let mut mixed_masks = settings.clone();
        let TerrainSettings::Procedural(ProceduralSettings::V3(ProceduralV3Settings {
            layout: V3LayoutSettings::Ring7(ring),
        })) = &mut mixed_masks.terrain
        else {
            panic!("test settings should use V3 Ring7")
        };
        ring.forest.mask = PatchMaskSettings::Explicit(vec![CubeCoord { x: 0, y: 0, z: 0 }]);
        let error = mixed_masks
            .validate()
            .expect_err("Ring7 cannot mix generated and authored masks");
        assert!(
            error
                .to_string()
                .contains("all GeneratedRegion or all Explicit"),
            "unexpected error: {error}"
        );

        let mut mismatched_seam = settings.clone();
        let TerrainSettings::Procedural(ProceduralSettings::V3(ProceduralV3Settings {
            layout: V3LayoutSettings::Ring7(ring),
        })) = &mut mismatched_seam.terrain
        else {
            panic!("test settings should use V3 Ring7")
        };
        let PatchEdgeContractSettings::Shared(shared) = &mut ring.waterfall.edges.west else {
            panic!("the test Ring7 should share center/waterfall")
        };
        shared.approach_depth += 1;
        let error = mismatched_seam
            .validate()
            .expect_err("both sides of a seam must consume the same contract");
        assert!(
            error.to_string().contains("mismatched shared settings"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn v3_ring7_explicit_masks_preserve_fixed_patch_adjacency() {
        let mut ring = valid_explicit_ring7();
        MapSettings {
            grid_radius: 33,
            level_height: 0.4,
            terrain: TerrainSettings::Procedural(ProceduralSettings::V3(ProceduralV3Settings {
                layout: V3LayoutSettings::Ring7(ring.clone()),
            })),
        }
        .validate()
        .expect("generated-equivalent Explicit masks should validate");

        std::mem::swap(&mut ring.mountains.mask, &mut ring.fort.mask);
        let error = MapSettings {
            grid_radius: 33,
            level_height: 0.4,
            terrain: TerrainSettings::Procedural(ProceduralSettings::V3(ProceduralV3Settings {
                layout: V3LayoutSettings::Ring7(ring),
            })),
        }
        .validate()
        .expect_err("rearranged Explicit masks must fail before layout resolution");
        assert!(
            error.contains("must form one oriented simple contiguous seam"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn v3_ring7_rejects_disjoint_explicit_seams_during_settings_validation() {
        let mut ring = valid_explicit_ring7();
        let PatchMaskSettings::Explicit(center) = &mut ring.center.mask else {
            panic!("the test Ring7 center mask should be Explicit");
        };
        let PatchMaskSettings::Explicit(waterfall) = &mut ring.waterfall.mask else {
            panic!("the test Ring7 waterfall mask should be Explicit");
        };
        let protrusion = CubeCoord {
            x: 12,
            y: 0,
            z: -12,
        };
        let index = waterfall
            .iter()
            .position(|coord| *coord == protrusion)
            .expect("the generated-equivalent waterfall mask owns the test cell");
        center.push(waterfall.remove(index));

        let error = MapSettings {
            grid_radius: 33,
            level_height: 0.4,
            terrain: TerrainSettings::Procedural(ProceduralSettings::V3(ProceduralV3Settings {
                layout: V3LayoutSettings::Ring7(ring),
            })),
        }
        .validate()
        .expect_err("a disjoint oriented seam must fail before layout resolution");
        assert!(
            error.contains(
                "center east / waterfall west must form one oriented simple contiguous seam"
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn v3_edge_contracts_validate_apertures_levels_and_liquid_direction() {
        let settings = MapSettings {
            grid_radius: 33,
            level_height: 0.4,
            terrain: TerrainSettings::Procedural(ProceduralSettings::V3(ProceduralV3Settings {
                layout: V3LayoutSettings::Ring7(valid_ring7()),
            })),
        };

        let mut narrow = settings.clone();
        let TerrainSettings::Procedural(ProceduralSettings::V3(ProceduralV3Settings {
            layout: V3LayoutSettings::Ring7(ring),
        })) = &mut narrow.terrain
        else {
            panic!("test settings should use V3 Ring7")
        };
        let PatchEdgeContractSettings::Shared(shared) = &mut ring.center.edges.east else {
            panic!("the test Ring7 center east edge should be shared")
        };
        shared.walker.width = 1;
        let error = narrow
            .validate()
            .expect_err("walker ports narrower than two cells must fail");
        assert!(
            error.to_string().contains("at least two cells wide"),
            "unexpected error: {error}"
        );

        let mut bad_elevation = settings.clone();
        let TerrainSettings::Procedural(ProceduralSettings::V3(ProceduralV3Settings {
            layout: V3LayoutSettings::Ring7(ring),
        })) = &mut bad_elevation.terrain
        else {
            panic!("test settings should use V3 Ring7")
        };
        let PatchEdgeContractSettings::Shared(shared) = &mut ring.center.edges.east else {
            panic!("the test Ring7 center east edge should be shared")
        };
        shared.elevation.min = 17;
        let error = bad_elevation
            .validate()
            .expect_err("an inverted elevation contract must fail");
        assert!(
            error.to_string().contains("min <= preferred <= max"),
            "unexpected error: {error}"
        );

        let mut same_direction = settings;
        let TerrainSettings::Procedural(ProceduralSettings::V3(ProceduralV3Settings {
            layout: V3LayoutSettings::Ring7(ring),
        })) = &mut same_direction.terrain
        else {
            panic!("test settings should use V3 Ring7")
        };
        let outlet = EdgeLiquidSettings::Outlet(EdgeLiquidPortSettings { width: 3 });
        for edge in [&mut ring.center.edges.east, &mut ring.waterfall.edges.west] {
            let PatchEdgeContractSettings::Shared(shared) = edge else {
                panic!("the test Ring7 center/waterfall seam should be shared")
            };
            shared.liquid = outlet;
        }
        let error = same_direction
            .validate()
            .expect_err("a liquid seam cannot have flow leaving both patches");
        assert!(
            error.to_string().contains("reciprocal equal-width"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn v3_port_bounds_cap_resolution_work() {
        WalkerPortSettings {
            count: MAX_WALKER_PORT_COUNT,
            width: MAX_SEAM_PORT_WIDTH,
        }
        .validate("test")
        .expect("the documented maximum walker request should validate");
        EdgeLiquidSettings::Outlet(EdgeLiquidPortSettings {
            width: MAX_SEAM_PORT_WIDTH,
        })
        .validate("test")
        .expect("the documented maximum liquid request should validate");

        let count_error = WalkerPortSettings {
            count: MAX_WALKER_PORT_COUNT + 1,
            width: 2,
        }
        .validate("test")
        .expect_err("unbounded walker port counts must fail");
        assert!(
            count_error.contains("count cannot exceed"),
            "unexpected error: {count_error}"
        );

        let walker_width_error = WalkerPortSettings {
            count: 1,
            width: MAX_SEAM_PORT_WIDTH + 1,
        }
        .validate("test")
        .expect_err("unbounded walker widths must fail");
        assert!(
            walker_width_error.contains("width cannot exceed"),
            "unexpected error: {walker_width_error}"
        );

        let liquid_width_error = EdgeLiquidSettings::Inlet(EdgeLiquidPortSettings {
            width: MAX_SEAM_PORT_WIDTH + 1,
        })
        .validate("test")
        .expect_err("unbounded liquid widths must fail");
        assert!(
            liquid_width_error.contains("width cannot exceed"),
            "unexpected error: {liquid_width_error}"
        );
    }

    #[test]
    fn v3_ring7_rejects_cyclic_patch_liquid_flow() {
        fn set_liquid(edge: &mut PatchEdgeContractSettings, liquid: EdgeLiquidSettings) {
            let PatchEdgeContractSettings::Shared(shared) = edge else {
                panic!("the fixed Ring7 seam should be shared");
            };
            shared.liquid = liquid;
        }

        let mut ring = valid_ring7();
        let outlet = EdgeLiquidSettings::Outlet(EdgeLiquidPortSettings { width: 2 });
        let inlet = EdgeLiquidSettings::Inlet(EdgeLiquidPortSettings { width: 2 });

        set_liquid(&mut ring.center.edges.east, outlet);
        set_liquid(&mut ring.waterfall.edges.west, inlet);
        set_liquid(&mut ring.waterfall.edges.south_west, outlet);
        set_liquid(&mut ring.forest.edges.north_east, inlet);
        set_liquid(&mut ring.forest.edges.north_west, outlet);
        set_liquid(&mut ring.center.edges.south_east, inlet);

        let error = MapSettings {
            grid_radius: 33,
            level_height: 0.4,
            terrain: TerrainSettings::Procedural(ProceduralSettings::V3(ProceduralV3Settings {
                layout: V3LayoutSettings::Ring7(ring),
            })),
        }
        .validate()
        .expect_err("directed patch hydrology must reject cycles");
        assert!(
            error.contains("acyclic patch graph"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn expanded_recipe_settings_validate_without_loosening_their_bounds() {
        for (min_clearance, upper_coverage_percent) in [(14, 18), (18, 21), (22, 24)] {
            let settings = MapSettings {
                grid_radius: 12,
                level_height: 0.4,
                terrain: TerrainSettings::Procedural(ProceduralSettings::V2(
                    ProceduralV2Settings {
                        environment: V2EnvironmentSettings::TemperateGrassland,
                        recipe: V2RecipeSettings::LayeredSkyIslands(LayeredSkyIslandsSettings {
                            ground: V2HillsSettings {
                                valley_level: 15,
                                max_relief: 8,
                                hills_per_bank: 3,
                            },
                            min_clearance,
                            upper_coverage_percent,
                        }),
                    },
                )),
            };
            assert!(
                settings.validate().is_ok(),
                "expanded sky settings {min_clearance}/{upper_coverage_percent} should validate"
            );
        }

        for (relief, peak_count) in [(18, 5), (21, 6), (24, 7)] {
            let settings = MapSettings {
                grid_radius: 12,
                level_height: 0.4,
                terrain: TerrainSettings::Procedural(ProceduralSettings::V2(
                    ProceduralV2Settings {
                        environment: V2EnvironmentSettings::Frozen,
                        recipe: V2RecipeSettings::Mountains(MountainsSettings {
                            base_level: 15,
                            relief,
                            peak_count,
                        }),
                    },
                )),
            };
            assert!(
                settings.validate().is_ok(),
                "expanded mountain settings {relief}/{peak_count} should validate"
            );
        }

        for (surface_level, cave_floor_level, chamber_count) in
            [(16, 7, 9), (16, 6, 10), (17, 6, 12)]
        {
            let settings = MapSettings {
                grid_radius: 12,
                level_height: 0.4,
                terrain: TerrainSettings::Procedural(ProceduralSettings::V2(
                    ProceduralV2Settings {
                        environment: V2EnvironmentSettings::Rocky,
                        recipe: V2RecipeSettings::Caves(CavesSettings {
                            surface_level,
                            cave_floor_level,
                            chamber_count,
                        }),
                    },
                )),
            };
            assert!(
                settings.validate().is_ok(),
                "expanded cave settings {surface_level}/{cave_floor_level}/{chamber_count} should validate"
            );
        }

        for (relief, peak_count) in [(13, 5), (25, 5), (18, 2), (18, 8)] {
            let settings = MountainsSettings {
                base_level: 15,
                relief,
                peak_count,
            };
            assert!(
                settings.validate(12).is_err(),
                "mountain bounds should reject {relief}/{peak_count}"
            );
        }
        for chamber_count in [5, 13] {
            let settings = CavesSettings {
                surface_level: 17,
                cave_floor_level: 6,
                chamber_count,
            };
            assert!(
                settings.validate(12).is_err(),
                "cave bounds should reject {chamber_count} chambers"
            );
        }
    }

    #[test]
    fn v3_caves_reserve_vertical_space_for_all_three_floor_tiers() {
        let valid = V3CavesSettings {
            surface_level: 17,
            cave_floor_level: 6,
            chamber_count: 9,
        };
        valid
            .validate(12)
            .expect("eleven levels should fit +0/+2/+4 floors, clearance, and roof");

        let shallow = V3CavesSettings {
            surface_level: 16,
            ..valid
        };
        let error = shallow
            .validate(12)
            .expect_err("ten levels cannot fit the complete tier contract");
        assert!(
            error.contains("+0/+2/+4 floor tiers"),
            "unexpected error: {error}"
        );
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
        let v1_with_recipe = V1_HILLS_RON.replacen(
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
        let source = V1_HILLS_RON;
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
        let source = V1_HILLS_RON;
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
        let v1_hills = V1_HILLS_RON
            .replacen("valley_level: 15", "valley_level: 125", 1)
            .replacen("bed_level: 12", "bed_level: 122", 1)
            .replacen("hazard_bottom: 13", "hazard_bottom: 123", 1)
            .replacen("hazard_top: 14", "hazard_top: 124", 1)
            .replacen("bridge_level: 16", "bridge_level: 126", 1);
        let v1_error = ron::from_str::<MapSettings>(&v1_hills)
            .expect_err("V1 terrain above the allocation ceiling should fail");
        assert!(
            v1_error.to_string().contains("cannot exceed level 128"),
            "unexpected error: {v1_error}"
        );

        let v2_hills = include_str!("../../../assets/config/worlds/procedural-hills.ron").replacen(
            "valley_level: 15",
            "valley_level: 125",
            1,
        );
        let v2_error = ron::from_str::<MapSettings>(&v2_hills)
            .expect_err("V2 Hills above the allocation ceiling should fail");
        assert!(
            v2_error.to_string().contains("cannot exceed level 128"),
            "unexpected error: {v2_error}"
        );

        let sky = include_str!("../../../assets/config/worlds/procedural-sky-islands.ron")
            .replacen("valley_level: 15", "valley_level: 125", 1);
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
