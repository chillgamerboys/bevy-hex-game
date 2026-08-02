//! Pure terrain construction.
//!
//! This module turns settings and resolved substance ids into a [`VoxelMap`].
//! It has no ECS parameters and spawns no entities, so terrain shape can be tested
//! directly while [`crate::grid`] stays focused on lifecycle and rendering.

use bevy::platform::collections::{HashMap, HashSet};

use hex_assets::SubstanceTable;
use hex_core::{HexCoord, Level, SubstanceId, TilePos};

use crate::generator::{HeightMap, PerlinGenerator, PerlinStep};
use crate::settings::{CubeCoord, MapSettings, PerlinSettings, ShowcaseSettings, TerrainSettings};
use crate::voxel::{Column, VoxelMap};

const SHOWCASE_TOPSOIL_LEVELS: Level = 3;

/// Substance ids available to terrain construction.
///
/// Fields unused by the selected preset are left as air. This keeps authored and
/// Perlin worlds independent of materials used only by procedural environments.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TerrainPalette {
    pub(super) bedrock: SubstanceId,
    pub(super) stone: SubstanceId,
    pub(super) dirt: SubstanceId,
    pub(super) grass: SubstanceId,
    pub(super) gravel: SubstanceId,
    pub(super) water: SubstanceId,
    pub(super) metal: SubstanceId,
    pub(super) worked_stone: SubstanceId,
    pub(super) limestone: SubstanceId,
    pub(super) slate: SubstanceId,
    pub(super) timber: SubstanceId,
    pub(super) terracotta: SubstanceId,
    pub(super) snow: SubstanceId,
    pub(super) ice: SubstanceId,
    pub(super) basalt: SubstanceId,
    pub(super) lava: SubstanceId,
}

impl TerrainPalette {
    pub(crate) fn for_terrain(
        table: &SubstanceTable,
        terrain: &TerrainSettings,
    ) -> Result<Self, String> {
        let id = |name: &str| {
            table
                .id(name)
                .ok_or_else(|| format!("substances.ron is missing required material \"{name}\""))
        };
        let optional = |name: &str| table.id(name).unwrap_or(SubstanceId::AIR);
        let mut palette = Self {
            bedrock: id("bedrock")?,
            stone: id("stone")?,
            dirt: id("dirt")?,
            grass: id("grass")?,
            gravel: optional("gravel"),
            water: optional("water"),
            metal: optional("metal"),
            worked_stone: optional("worked_stone"),
            limestone: optional("limestone"),
            slate: optional("slate"),
            timber: optional("timber"),
            terracotta: optional("terracotta"),
            snow: optional("snow"),
            ice: optional("ice"),
            basalt: optional("basalt"),
            lava: optional("lava"),
        };

        match terrain {
            TerrainSettings::Perlin(_) => {}
            TerrainSettings::Showcase(_) => {
                palette.gravel = id("gravel")?;
                palette.water = id("water")?;
                palette.metal = id("metal")?;
            }
            TerrainSettings::Procedural(_) => {
                // Procedural candidates and their canonical fallback share one
                // validator. Require the complete procedural vocabulary so a missing
                // environment material cannot be mistaken for air during validation.
                palette.gravel = id("gravel")?;
                palette.water = id("water")?;
                palette.metal = id("metal")?;
                palette.worked_stone = id("worked_stone")?;
                palette.limestone = id("limestone")?;
                palette.slate = id("slate")?;
                palette.timber = id("timber")?;
                palette.terracotta = id("terracotta")?;
                palette.snow = id("snow")?;
                palette.ice = id("ice")?;
                palette.basalt = id("basalt")?;
                palette.lava = id("lava")?;
            }
        }

        Ok(palette)
    }
}

/// Builds an authored or Perlin map without procedural runtime inputs.
///
/// Procedural terrain requires a resolved seed, the published walker, and the live
/// substance predicate, so it must go through `procedural::build`.
#[must_use]
pub(crate) fn build_non_procedural_map(
    settings: &MapSettings,
    palette: &TerrainPalette,
) -> Option<VoxelMap> {
    match &settings.terrain {
        TerrainSettings::Showcase(showcase) => {
            Some(build_showcase(settings.grid_radius, showcase, palette))
        }
        TerrainSettings::Perlin(perlin) => {
            Some(build_perlin(settings.grid_radius, perlin, palette))
        }
        TerrainSettings::Procedural(_) => None,
    }
}

fn build_perlin(grid_radius: u32, settings: &PerlinSettings, palette: &TerrainPalette) -> VoxelMap {
    let steps = settings
        .steps
        .iter()
        .map(|step| PerlinStep::new(step.x_freq, step.y_freq, step.magnitude))
        .collect();
    let generator = PerlinGenerator::new(steps, settings.seed);
    let heights = HeightMap::new(generator, grid_radius);

    let mut map = VoxelMap::new();
    for coord in HexCoord::ORIGIN.within_radius(grid_radius) {
        map.insert_column(
            coord,
            perlin_column_for(heights.surface_level(coord), palette),
        );
    }
    map
}

/// The original Perlin strata, kept byte-for-byte equivalent to the previous
/// generator: bedrock, stone, two dirt levels, then grass.
fn perlin_column_for(surface: Level, palette: &TerrainPalette) -> Column {
    let mut column = Column::new();
    for level in 0..=surface {
        let substance = if level == 0 {
            palette.bedrock
        } else if level == surface {
            palette.grass
        } else if level + 2 >= surface {
            palette.dirt
        } else {
            palette.stone
        };
        column.set(level, substance);
    }
    column
}

fn build_showcase(
    grid_radius: u32,
    settings: &ShowcaseSettings,
    palette: &TerrainPalette,
) -> VoxelMap {
    let river_centres = river_centres(settings);
    let river_cells = expanded_river(&river_centres, settings.river.half_width, grid_radius);
    let trail_levels = switchback_levels(settings);
    let summit = coord(settings.mountain.summit);

    let mut map = VoxelMap::new();
    for coord in HexCoord::ORIGIN.within_radius(grid_radius) {
        let (river_distance, nearest_centre) = nearest_river(coord, &river_centres);

        let column = if river_cells.contains(&coord) {
            river_column(settings, palette)
        } else if let Some(level) = trail_levels.get(&coord).copied() {
            trail_column(level, settings, palette)
        } else if coord.y() >= nearest_centre.y() {
            let surface = gentle_surface(river_distance, settings);
            showcase_column_for(surface, palette.grass, palette)
        } else {
            let surface = mountain_surface(coord, summit, settings);
            mountain_column(surface, settings, palette)
        };
        map.insert_column(coord, column);
    }

    for x in &settings.bridge.lane_xs {
        for y in settings.bridge.y_min..=settings.bridge.y_max {
            map.set(
                TilePos::new(HexCoord::from_axial(*x, y), settings.bridge.deck_level),
                palette.metal,
            );
        }
    }

    map
}

fn river_centres(settings: &ShowcaseSettings) -> Vec<HexCoord> {
    let mut centres = Vec::new();
    let mut seen: HashSet<HexCoord> = HashSet::default();

    for pair in settings.river.path.windows(2) {
        let [start, end] = pair else { continue };
        for coord in coord(*start).line_between(coord(*end)) {
            if seen.insert(coord) {
                centres.push(coord);
            }
        }
    }
    centres
}

fn expanded_river(centres: &[HexCoord], half_width: u32, grid_radius: u32) -> HashSet<HexCoord> {
    let mut cells = HashSet::default();
    for centre in centres {
        for coord in centre.within_radius(half_width) {
            if HexCoord::ORIGIN.distance(coord) <= grid_radius {
                cells.insert(coord);
            }
        }
    }
    cells
}

fn switchback_levels(settings: &ShowcaseSettings) -> HashMap<HexCoord, Level> {
    let mut levels = HashMap::default();
    let transitions =
        i64::try_from(settings.switchback.len().saturating_sub(1)).unwrap_or(i64::MAX);
    let rise = i64::from(settings.mountain.peak_level - settings.bridge.deck_level);

    for (index, raw) in settings.switchback.iter().copied().enumerate() {
        let progress = i64::try_from(index).unwrap_or(i64::MAX);
        let offset = if transitions == 0 {
            0
        } else {
            rise.saturating_mul(progress) / transitions
        };
        let level = i64::from(settings.bridge.deck_level).saturating_add(offset);
        let level = Level::try_from(level).unwrap_or(Level::MAX);
        levels.insert(coord(raw), level);
    }
    levels
}

fn nearest_river(coord: HexCoord, centres: &[HexCoord]) -> (u32, HexCoord) {
    centres
        .iter()
        .copied()
        .map(|centre| (coord.distance(centre), centre))
        .min_by_key(|(distance, centre)| (*distance, centre.x(), centre.y()))
        .unwrap_or((0, HexCoord::ORIGIN))
}

fn gentle_surface(river_distance: u32, settings: &ShowcaseSettings) -> Level {
    if river_distance <= 2 {
        return settings.valley_level;
    }

    let rise = 1 + (river_distance - 3) / settings.gentle_terrace_width;
    let rise = Level::try_from(rise).unwrap_or(Level::MAX);
    settings
        .valley_level
        .saturating_add(rise)
        .min(settings.gentle_max_level)
}

fn mountain_surface(coord: HexCoord, summit: HexCoord, settings: &ShowcaseSettings) -> Level {
    let distance = Level::try_from(coord.distance(summit)).unwrap_or(Level::MAX);
    let drop = settings.mountain.falloff_per_hex.saturating_mul(distance);
    settings
        .mountain
        .peak_level
        .saturating_sub(drop)
        .max(settings.mountain.base_level)
}

fn river_column(settings: &ShowcaseSettings, palette: &TerrainPalette) -> Column {
    let mut column = showcase_column_for(settings.river.bed_level, palette.gravel, palette);
    for level in settings.river.water_bottom..=settings.river.water_top {
        column.set(level, palette.water);
    }
    column
}

fn mountain_column(
    surface: Level,
    settings: &ShowcaseSettings,
    palette: &TerrainPalette,
) -> Column {
    let mut column = showcase_column_for(surface, palette.grass, palette);
    if surface >= settings.mountain.exposed_stone_level {
        for level in settings.mountain.exposed_stone_level..=surface {
            column.set(level, palette.stone);
        }
    }
    column
}

fn trail_column(surface: Level, settings: &ShowcaseSettings, palette: &TerrainPalette) -> Column {
    let mut column = showcase_column_for(surface, palette.gravel, palette);
    if surface > settings.mountain.exposed_stone_level {
        for level in settings.mountain.exposed_stone_level..surface {
            column.set(level, palette.stone);
        }
    }
    column
}

/// Showcase strata: bedrock, stone core, three dirt levels, and a chosen surface.
fn showcase_column_for(
    surface: Level,
    surface_substance: SubstanceId,
    palette: &TerrainPalette,
) -> Column {
    let mut column = Column::new();
    for level in 0..=surface {
        let substance = if level == 0 {
            palette.bedrock
        } else if level == surface {
            surface_substance
        } else if level >= surface.saturating_sub(SHOWCASE_TOPSOIL_LEVELS) {
            palette.dirt
        } else {
            palette.stone
        };
        column.set(level, substance);
    }
    column
}

const fn coord(raw: CubeCoord) -> HexCoord {
    HexCoord::from_axial(raw.x, raw.y)
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap as StdHashMap, HashSet as StdHashSet, VecDeque};

    use hex_assets::{ArtPalette, SubstanceFile};

    use super::*;

    const BEDROCK: SubstanceId = SubstanceId(1);
    const DIRT: SubstanceId = SubstanceId(2);
    const GRASS: SubstanceId = SubstanceId(3);
    const GRAVEL: SubstanceId = SubstanceId(4);
    const METAL: SubstanceId = SubstanceId(5);
    const STONE: SubstanceId = SubstanceId(6);
    const WATER: SubstanceId = SubstanceId(7);
    const BODY_LEVELS: Level = 2;

    fn palette() -> TerrainPalette {
        TerrainPalette {
            bedrock: BEDROCK,
            stone: STONE,
            dirt: DIRT,
            grass: GRASS,
            gravel: GRAVEL,
            water: WATER,
            metal: METAL,
            worked_stone: SubstanceId(12),
            limestone: SubstanceId(13),
            slate: SubstanceId(14),
            timber: SubstanceId(15),
            terracotta: SubstanceId(16),
            snow: SubstanceId(8),
            ice: SubstanceId(9),
            basalt: SubstanceId(10),
            lava: SubstanceId(11),
        }
    }

    fn settings() -> MapSettings {
        ron::from_str(include_str!("../../../assets/config/world.ron"))
            .expect("the shipped showcase settings should parse")
    }

    fn art_palette() -> ArtPalette {
        ron::from_str(include_str!("../../../assets/art/palette.ron"))
            .expect("the shipped art palette should parse")
    }

    fn showcase(settings: &MapSettings) -> &ShowcaseSettings {
        let TerrainSettings::Showcase(showcase) = &settings.terrain else {
            panic!("test settings should select Showcase")
        };
        showcase
    }

    fn showcase_map() -> (MapSettings, VoxelMap) {
        let settings = settings();
        let map = build_non_procedural_map(&settings, &palette())
            .expect("showcase settings should produce an authored map");
        (settings, map)
    }

    #[test]
    fn authored_presets_do_not_require_procedural_environment_materials() {
        let mut substances: SubstanceFile =
            ron::from_str(include_str!("../../../assets/config/substances.ron"))
                .expect("the shipped substances should parse");
        for environment_material in ["snow", "ice", "basalt", "lava"] {
            substances.substances.remove(environment_material);
        }
        let table = SubstanceTable::from_file(&substances, &art_palette())
            .expect("the remaining substances should resolve through the shipped palette");
        let showcase = settings();

        assert!(
            TerrainPalette::for_terrain(&table, &showcase.terrain).is_ok(),
            "Showcase should not depend on procedural-environment materials"
        );

        let perlin: MapSettings = ron::from_str(include_str!(
            "../../../assets/config/worlds/rolling-hills.ron"
        ))
        .expect("the shipped Perlin settings should parse");
        assert!(
            TerrainPalette::for_terrain(&table, &perlin.terrain).is_ok(),
            "Perlin should not depend on procedural-environment materials"
        );
    }

    #[test]
    fn procedural_presets_reject_an_incomplete_material_vocabulary() {
        let mut substances: SubstanceFile =
            ron::from_str(include_str!("../../../assets/config/substances.ron"))
                .expect("the shipped substances should parse");
        substances.substances.remove("lava");
        let table = SubstanceTable::from_file(&substances, &art_palette())
            .expect("the remaining substances should resolve through the shipped palette");
        let procedural: MapSettings = ron::from_str(include_str!(
            "../../../assets/config/worlds/procedural-hills.ron"
        ))
        .expect("the shipped procedural settings should parse");

        let error = TerrainPalette::for_terrain(&table, &procedural.terrain)
            .expect_err("procedural validation must not alias a missing material to air");
        assert!(error.contains("lava"));
    }

    fn at(x: i32, y: i32, z: i32) -> HexCoord {
        HexCoord::new_cubic(x, y, z)
    }

    #[test]
    fn showcase_has_all_469_bedrock_columns() {
        let (settings, map) = showcase_map();
        assert_eq!(map.len(), 469);

        for coord in HexCoord::ORIGIN.within_radius(settings.grid_radius) {
            let column = map.column(coord).expect("every coordinate needs a column");
            assert_eq!(
                column.get(0),
                BEDROCK,
                "{coord:?} does not start with bedrock"
            );
            assert!(column.top() > 1, "{coord:?} is only bare bedrock");
        }
    }

    #[test]
    fn valley_gentle_side_and_mountain_have_exact_surfaces() {
        let (_, map) = showcase_map();

        assert_eq!(map.surface(at(-4, 2, 2)), Some(15), "valley shelf");
        assert_eq!(map.surface(at(0, 4, -4)), Some(16), "player bank");
        assert_eq!(map.surface(at(0, 12, -12)), Some(19), "gentle cap");
        assert_eq!(map.surface(at(0, -4, 4)), Some(15), "mountain bank");
        assert_eq!(map.surface(at(3, -9, 6)), Some(30), "summit");
    }

    #[test]
    fn ordinary_showcase_columns_have_three_dirt_levels() {
        let (_, map) = showcase_map();
        let coord = at(0, 12, -12);
        let column = map.column(coord).expect("the gentle cap should exist");

        assert_eq!(column.get(0), BEDROCK);
        assert_eq!(column.get(15), STONE);
        for level in 16..=18 {
            assert_eq!(column.get(level), DIRT, "level {level} should be dirt");
        }
        assert_eq!(column.get(19), GRASS);
        assert_eq!(column.get(20), SubstanceId::AIR);
    }

    #[test]
    fn river_and_bridge_follow_the_vertical_contract() {
        let (_, map) = showcase_map();

        let open_river = map
            .column(at(-7, 1, 6))
            .expect("the river waypoint should exist");
        assert_eq!(open_river.get(12), GRAVEL);
        assert_eq!(open_river.get(13), WATER);
        assert_eq!(open_river.get(14), WATER);
        assert_eq!(open_river.get(15), SubstanceId::AIR);
        assert_eq!(open_river.get(16), SubstanceId::AIR);

        let under_bridge = map
            .column(HexCoord::ORIGIN)
            .expect("the bridge centre should exist");
        assert_eq!(under_bridge.get(12), GRAVEL);
        assert_eq!(under_bridge.get(13), WATER);
        assert_eq!(under_bridge.get(14), WATER);
        assert_eq!(under_bridge.get(15), SubstanceId::AIR);
        assert_eq!(under_bridge.get(16), METAL);
        assert_eq!(under_bridge.get(17), SubstanceId::AIR);
    }

    #[test]
    fn bridge_is_two_lanes_and_one_voxel_thick() {
        let (settings, map) = showcase_map();
        let showcase = showcase(&settings);

        for x in &showcase.bridge.lane_xs {
            for y in showcase.bridge.y_min..=showcase.bridge.y_max {
                let column = map
                    .column(HexCoord::from_axial(*x, y))
                    .expect("every bridge coordinate should exist");
                assert_eq!(column.get(showcase.bridge.deck_level), METAL);
                assert_ne!(
                    column.get(showcase.bridge.deck_level + 1),
                    METAL,
                    "the bridge deck must remain one voxel thick"
                );
            }
        }
    }

    #[test]
    fn switchback_is_exact_contiguous_and_climbable() {
        let (settings, map) = showcase_map();
        let showcase = showcase(&settings);
        let transitions = i64::try_from(showcase.switchback.len() - 1)
            .expect("the trail length should fit in i64");
        let rise = i64::from(showcase.mountain.peak_level - showcase.bridge.deck_level);

        let mut previous: Option<(HexCoord, Level)> = None;
        for (index, raw) in showcase.switchback.iter().copied().enumerate() {
            let coord = coord(raw);
            let progress = i64::try_from(index).expect("the trail index should fit in i64");
            let expected = i64::from(showcase.bridge.deck_level) + rise * progress / transitions;
            let expected = Level::try_from(expected).expect("trail levels should fit");
            assert_eq!(
                map.surface(coord),
                Some(expected),
                "wrong trail surface at index {index}"
            );

            let substance = map
                .column(coord)
                .expect("the trail column should exist")
                .get(expected);
            let expected_substance = if index == 0 { METAL } else { GRAVEL };
            assert_eq!(
                substance, expected_substance,
                "wrong trail material at index {index}"
            );

            if let Some((last_coord, last_level)) = previous {
                assert_eq!(last_coord.distance(coord), 1);
                assert!(
                    last_level.abs_diff(expected) <= 1,
                    "trail rises from {last_level} to {expected} at index {index}"
                );
            }
            previous = Some((coord, expected));
        }
    }

    #[test]
    fn high_mountain_is_exposed_stone_beside_the_gravel_summit() {
        let (_, map) = showcase_map();
        let summit = map.column(at(3, -9, 6)).expect("the summit should exist");
        for level in 23..30 {
            assert_eq!(summit.get(level), STONE);
        }
        assert_eq!(summit.get(30), GRAVEL);

        let exposed_face = map
            .column(at(4, -9, 5))
            .expect("the exposed summit face should exist");
        assert_eq!(exposed_face.surface(), Some(27));
        assert_eq!(exposed_face.get(27), STONE);
    }

    #[test]
    fn water_is_one_connected_edge_to_edge_barrier() {
        let (settings, map) = showcase_map();
        let showcase = showcase(&settings);
        let water: StdHashSet<HexCoord> = map
            .columns()
            .filter_map(|(coord, column)| {
                (column.get(showcase.river.water_bottom) == WATER).then_some(coord)
            })
            .collect();

        let start = water
            .iter()
            .next()
            .copied()
            .expect("the showcase should contain water");
        let connected = flood_coords(start, |coord| {
            coord
                .neighbors()
                .into_iter()
                .filter(|neighbor| water.contains(neighbor))
                .collect()
        });
        assert_eq!(
            connected.len(),
            water.len(),
            "the river should be one connected body"
        );

        let first = showcase
            .river
            .path
            .first()
            .copied()
            .map(coord)
            .expect("the river should have a first endpoint");
        let last = showcase
            .river
            .path
            .last()
            .copied()
            .map(coord)
            .expect("the river should have a last endpoint");
        assert!(water.contains(&first));
        assert!(water.contains(&last));
        assert_eq!(HexCoord::ORIGIN.distance(first), settings.grid_radius);
        assert_eq!(HexCoord::ORIGIN.distance(last), settings.grid_radius);
    }

    #[test]
    fn metal_bridge_is_the_only_connection_between_the_banks() {
        let (_, map) = showcase_map();
        let player = at(0, 4, -4);
        let enemy = at(0, -4, 4);

        assert!(
            !banks_connected(&map, player, enemy, false),
            "the river should disconnect the banks when metal is removed"
        );
        assert!(
            banks_connected(&map, player, enemy, true),
            "the bridge should reconnect the banks"
        );
    }

    #[test]
    fn steep_face_contains_cliffs_beside_the_climbable_trail() {
        let (_, map) = showcase_map();
        let summit = at(3, -9, 6);
        let cliff = at(4, -9, 5);
        let summit_level = map.surface(summit).expect("summit surface");
        let cliff_level = map.surface(cliff).expect("adjacent face surface");

        assert_eq!(summit.distance(cliff), 1);
        assert!(
            summit_level.abs_diff(cliff_level) > 1,
            "the direct mountain face should remain a cliff"
        );
    }

    fn flood_coords(
        start: HexCoord,
        neighbors: impl Fn(HexCoord) -> Vec<HexCoord>,
    ) -> StdHashSet<HexCoord> {
        let mut visited = StdHashSet::from([start]);
        let mut queue = VecDeque::from([start]);
        while let Some(coord) = queue.pop_front() {
            for neighbor in neighbors(coord) {
                if visited.insert(neighbor) {
                    queue.push_back(neighbor);
                }
            }
        }
        visited
    }

    fn banks_connected(
        map: &VoxelMap,
        start_coord: HexCoord,
        end_coord: HexCoord,
        include_metal: bool,
    ) -> bool {
        let footing = standable_surfaces(map, include_metal);
        let Some(start_level) = footing
            .get(&start_coord)
            .and_then(|levels| levels.iter().min())
            .copied()
        else {
            return false;
        };
        let Some(end_level) = footing
            .get(&end_coord)
            .and_then(|levels| levels.iter().min())
            .copied()
        else {
            return false;
        };

        let start = (start_coord, start_level);
        let target = (end_coord, end_level);
        let mut visited = StdHashSet::from([start]);
        let mut queue = VecDeque::from([start]);

        while let Some((coord, level)) = queue.pop_front() {
            if (coord, level) == target {
                return true;
            }
            for neighbor in coord.neighbors() {
                let Some(levels) = footing.get(&neighbor) else {
                    continue;
                };
                for next_level in levels {
                    let next = (neighbor, *next_level);
                    if level.abs_diff(*next_level) <= 1 && visited.insert(next) {
                        queue.push_back(next);
                    }
                }
            }
        }
        false
    }

    fn standable_surfaces(map: &VoxelMap, include_metal: bool) -> StdHashMap<HexCoord, Vec<Level>> {
        let mut footing = StdHashMap::new();
        for (coord, column) in map.columns() {
            for level in 0..column.top() {
                let substance = virtual_substance(column, level, include_metal);
                let solid = matches!(substance, BEDROCK | DIRT | GRASS | GRAVEL | STONE)
                    || (include_metal && substance == METAL);
                if !solid {
                    continue;
                }

                let has_headroom = (1..=BODY_LEVELS).all(|offset| {
                    virtual_substance(column, level + offset, include_metal).is_air()
                });
                if has_headroom {
                    footing.entry(coord).or_insert_with(Vec::new).push(level);
                }
            }
        }
        footing
    }

    fn virtual_substance(column: &Column, level: Level, include_metal: bool) -> SubstanceId {
        let substance = column.get(level);
        if !include_metal && substance == METAL {
            SubstanceId::AIR
        } else {
            substance
        }
    }
}
