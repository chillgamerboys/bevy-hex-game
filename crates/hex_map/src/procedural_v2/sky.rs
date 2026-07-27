//! Layered sky islands built above a finalized V2 Hills ground plan.
//!
//! The ground selection is completed before any `sky.*` stream is sampled. Every
//! candidate clones that immutable semantic ground, appends an independent upper
//! network, and passes the complete volume through the common V2 validator.

use std::collections::BTreeSet;

use hex_core::{HexCoord, Level, MapViewHint, SpecialMovementRegion, SubstanceId, TilePos};

use super::hills::{self, HillsMetadata, HillsMetrics};
use super::recipe::{
    materialize_selection, run_recipe, CandidateAttemptError, CandidateContext, FallbackContext,
    MaterializedSelection, RecipePlan, RecipeSelection, RecipeValidation, RepairOutcome,
    ReportMetrics, V2Recipe, ValidationContext,
};
use super::volume::{
    LevelInterval, SolidMass, SolidMaterialRole, SurfaceAccess, SurfaceMetadata, VolumeElement,
};
use super::V2GenerationError;
use crate::procedural::TacticalMetrics;
use crate::settings::{
    LayeredSkyIslandsSettings, ProceduralV2Settings, V2EnvironmentSettings, V2RecipeSettings,
};
use crate::terrain::TerrainPalette;

const PRIMARY_ISLAND_COUNT: usize = 3;
const UPPER_REGION_OFFSET: u32 = 1;

/// Measurements used to validate and rank one upper-layer candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SkyMetrics {
    tactical: TacticalMetrics,
    upper_columns: u32,
    coverage_percent: u32,
    satellite_count: u8,
    bridge_count: u8,
}

impl ReportMetrics for SkyMetrics {
    fn tactical(&self) -> TacticalMetrics {
        self.tactical
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SkyMetadata {
    upper_region: SpecialMovementRegion,
    upper_surface: Level,
    primary_centres: [HexCoord; PRIMARY_ISLAND_COUNT],
    satellite_centres: Vec<HexCoord>,
    island_cells: BTreeSet<HexCoord>,
    bridge_lanes: Vec<[BTreeSet<HexCoord>; 2]>,
    upper_cells: BTreeSet<HexCoord>,
    ground_repair_actions: Vec<String>,
}

struct LayeredSkyRecipe<'a> {
    ground: &'a RecipeSelection<HillsMetadata, HillsMetrics>,
    environment: V2EnvironmentSettings,
    level_height: f32,
}

/// Generates a finalized Hills ground and layers a separately selected upper network.
pub(crate) fn build(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV2Settings,
    seed: u64,
    palette: &TerrainPalette,
    is_solid: &dyn Fn(SubstanceId) -> bool,
) -> Result<MaterializedSelection<SkyMetadata, SkyMetrics>, V2GenerationError> {
    let V2RecipeSettings::LayeredSkyIslands(sky_settings) = &settings.recipe else {
        return Err(V2GenerationError::RecipeUnavailable("LayeredSkyIslands"));
    };
    if !level_height.is_finite() || level_height <= 0.0 {
        return Err(V2GenerationError::RecipeContract(
            "LayeredSkyIslands level height must be positive and finite".to_owned(),
        ));
    }

    let ground_settings = ProceduralV2Settings {
        environment: settings.environment,
        recipe: V2RecipeSettings::Hills(sky_settings.ground.clone()),
    };
    let ground = hills::select(
        grid_radius,
        level_height,
        &ground_settings,
        seed,
        palette,
        is_solid,
    )?
    .into_unvalidated();
    let recipe = LayeredSkyRecipe {
        ground: &ground,
        environment: settings.environment,
        level_height,
    };
    let mut selection = run_recipe(&recipe, sky_settings, grid_radius, seed)?;
    selection.prepend_diagnostics(
        format!(
            "ground selected candidate {:?}; fallback={}",
            ground.selected_candidate, ground.used_fallback
        ),
        ground.used_fallback,
    );
    materialize_selection(selection, palette, is_solid)
}

impl V2Recipe for LayeredSkyRecipe<'_> {
    type Settings = LayeredSkyIslandsSettings;
    type Metadata = SkyMetadata;
    type Metrics = SkyMetrics;
    type Score = (u32, u32, u8);

    fn construct(
        &self,
        context: CandidateContext,
        settings: &Self::Settings,
    ) -> Result<RecipePlan<Self::Metadata>, CandidateAttemptError> {
        layered_plan(self, context, settings, false)
    }

    fn validate(
        &self,
        _context: ValidationContext,
        settings: &Self::Settings,
        plan: &RecipePlan<Self::Metadata>,
    ) -> RecipeValidation<Self::Metrics> {
        validate_layered_plan(self, settings, plan)
    }

    fn repair(
        &self,
        _context: CandidateContext,
        _settings: &Self::Settings,
        _plan: &mut RecipePlan<Self::Metadata>,
        _round: u8,
        _issues: &[String],
    ) -> Result<RepairOutcome, CandidateAttemptError> {
        Ok(RepairOutcome::NoChange)
    }

    fn score(
        &self,
        settings: &Self::Settings,
        metrics: &Self::Metrics,
        candidate: u8,
    ) -> Self::Score {
        (
            metrics
                .coverage_percent
                .abs_diff(u32::from(settings.upper_coverage_percent)),
            metrics.upper_columns,
            candidate,
        )
    }

    fn preexisting_repair_actions(&self, plan: &RecipePlan<Self::Metadata>) -> Vec<String> {
        plan.metadata.ground_repair_actions.clone()
    }

    fn canonical_fallback(
        &self,
        context: FallbackContext,
        settings: &Self::Settings,
    ) -> Result<RecipePlan<Self::Metadata>, V2GenerationError> {
        layered_plan(
            self,
            CandidateContext {
                grid_radius: context.grid_radius,
                candidate: 0,
                streams: super::seed::SeedStreams::new(0, 0),
            },
            settings,
            true,
        )
        .map_err(|error| match error {
            CandidateAttemptError::Rejected(issues) => V2GenerationError::InvalidFallback(issues),
            CandidateAttemptError::Fatal(error) => error,
        })
    }
}

fn layered_plan(
    recipe: &LayeredSkyRecipe<'_>,
    context: CandidateContext,
    settings: &LayeredSkyIslandsSettings,
    fallback: bool,
) -> Result<RecipePlan<SkyMetadata>, CandidateAttemptError> {
    let mut volume = recipe.ground.plan.volume.clone();
    let layout = choose_layout(
        context,
        settings,
        &recipe.ground.plan.metadata.topology.protected_approaches,
        fallback,
    )?;
    let highest_ground = volume
        .surfaces
        .keys()
        .map(|surface| surface.level)
        .max()
        .ok_or_else(|| CandidateAttemptError::rejected("finalized Hills ground has no surfaces"))?;
    let lowest_upper = highest_ground
        .checked_add(1)
        .and_then(|level| level.checked_add(settings.min_clearance))
        .ok_or_else(|| CandidateAttemptError::rejected("upper clearance level overflowed"))?;
    let upper_surface = lowest_upper
        .checked_add(3)
        .ok_or_else(|| CandidateAttemptError::rejected("upper island level overflowed"))?;
    let upper_region = next_special_region(&volume);

    for coord in &layout.upper_cells {
        let is_bridge = layout.bridge_cells.contains(coord) && !layout.island_cells.contains(coord);
        let column = volume.columns.get_mut(coord).ok_or_else(|| {
            CandidateAttemptError::rejected(format!(
                "upper layout escaped the map footprint at {coord:?}"
            ))
        })?;
        let distance = layout
            .all_centres()
            .map(|centre| centre.distance(*coord))
            .min()
            .unwrap_or(u32::MAX);
        append_upper_mass(
            &mut column.elements,
            upper_surface,
            distance,
            is_bridge,
            recipe.environment,
            context
                .streams
                .stage("sky.materials")
                .sample_coord(*coord, 0),
        );
        volume.surfaces.insert(
            TilePos::new(*coord, upper_surface),
            SurfaceMetadata {
                access: SurfaceAccess::SpecialMovement(upper_region),
                interior: None,
            },
        );
    }
    volume.view_hint = sky_view_hint(
        context.grid_radius,
        recipe.level_height,
        highest_ground,
        upper_surface,
    )?;

    Ok(RecipePlan {
        volume,
        metadata: SkyMetadata {
            upper_region,
            upper_surface,
            primary_centres: layout.primary_centres,
            satellite_centres: layout.satellite_centres,
            island_cells: layout.island_cells,
            bridge_lanes: layout.bridge_lanes,
            upper_cells: layout.upper_cells,
            ground_repair_actions: recipe.ground.plan.metadata.repair_actions.clone(),
        },
    })
}

struct SkyLayout {
    primary_centres: [HexCoord; PRIMARY_ISLAND_COUNT],
    satellite_centres: Vec<HexCoord>,
    island_cells: BTreeSet<HexCoord>,
    bridge_cells: BTreeSet<HexCoord>,
    bridge_lanes: Vec<[BTreeSet<HexCoord>; 2]>,
    upper_cells: BTreeSet<HexCoord>,
}

impl SkyLayout {
    fn all_centres(&self) -> impl Iterator<Item = HexCoord> + '_ {
        self.primary_centres
            .iter()
            .copied()
            .chain(self.satellite_centres.iter().copied())
    }
}

fn choose_layout(
    context: CandidateContext,
    settings: &LayeredSkyIslandsSettings,
    protected_approaches: &BTreeSet<HexCoord>,
    fallback: bool,
) -> Result<SkyLayout, CandidateAttemptError> {
    let radius = i32::try_from(context.grid_radius)
        .map_err(|_out_of_range| CandidateAttemptError::rejected("grid radius is too large"))?;
    let orientation = if fallback {
        0
    } else {
        u8::try_from(context.streams.stage("sky.layout.orientation").sample(0) % 3)
            .unwrap_or_default()
    };
    let extent = (radius / 2).max(5);
    let primary_centres = [
        rotate_third(HexCoord::from_axial(-extent, extent / 3), orientation),
        rotate_third(HexCoord::from_axial(0, -extent / 2), orientation),
        rotate_third(HexCoord::from_axial(extent, -extent / 3), orientation),
    ];
    let satellite_count = if fallback
        || context
            .streams
            .stage("sky.layout.satellites")
            .sample(0)
            .is_multiple_of(2)
    {
        2
    } else {
        1
    };
    let satellite_candidates = [
        rotate_third(HexCoord::from_axial(0, extent), orientation),
        rotate_third(HexCoord::from_axial(-extent / 2, extent), orientation),
    ];
    let satellite_centres: Vec<_> = satellite_candidates
        .into_iter()
        .take(satellite_count)
        .collect();

    let target_columns = footprint_size(context.grid_radius)
        .saturating_mul(u32::from(settings.upper_coverage_percent))
        .saturating_add(50)
        / 100;
    let max_island_radius = context.grid_radius.saturating_div(3).max(2);
    let mut layouts = Vec::new();
    for primary_radius in 1..=max_island_radius {
        let satellite_radius = primary_radius.saturating_div(2).max(1);
        let layout = build_layout(
            context.grid_radius,
            primary_centres,
            &satellite_centres,
            primary_radius,
            satellite_radius,
            protected_approaches,
        )?;
        layouts.push(layout);
    }
    layouts
        .into_iter()
        .min_by_key(|layout| {
            (
                u32::try_from(layout.island_cells.len())
                    .unwrap_or(u32::MAX)
                    .abs_diff(target_columns),
                layout.island_cells.len(),
            )
        })
        .ok_or_else(|| CandidateAttemptError::rejected("no upper layout could be constructed"))
}

fn build_layout(
    grid_radius: u32,
    primary_centres: [HexCoord; PRIMARY_ISLAND_COUNT],
    satellite_centres: &[HexCoord],
    primary_radius: u32,
    satellite_radius: u32,
    protected_approaches: &BTreeSet<HexCoord>,
) -> Result<SkyLayout, CandidateAttemptError> {
    let mut island_cells = BTreeSet::new();
    for centre in primary_centres {
        island_cells.extend(
            centre
                .within_radius(primary_radius)
                .into_iter()
                .filter(|coord| {
                    HexCoord::ORIGIN.distance(*coord) <= grid_radius
                        && !protected_approaches.contains(coord)
                }),
        );
    }
    for centre in satellite_centres {
        island_cells.extend(
            centre
                .within_radius(satellite_radius)
                .into_iter()
                .filter(|coord| {
                    HexCoord::ORIGIN.distance(*coord) <= grid_radius
                        && !protected_approaches.contains(coord)
                }),
        );
    }

    let [first_primary, middle_primary, last_primary] = primary_centres;
    let mut connections = vec![
        (first_primary, middle_primary),
        (middle_primary, last_primary),
    ];
    if let Some(first) = satellite_centres.first().copied() {
        connections.push((first, middle_primary));
    }
    if let Some(second) = satellite_centres.get(1).copied() {
        connections.push((second, first_primary));
    }

    let mut bridge_cells = BTreeSet::new();
    let mut bridge_lanes = Vec::new();
    for (start, end) in connections {
        let lanes = bridge_between(grid_radius, start, end)?;
        let [first_lane, second_lane] = &lanes;
        bridge_cells.extend(first_lane.iter().copied());
        bridge_cells.extend(second_lane.iter().copied());
        bridge_lanes.push(lanes);
    }
    let upper_cells = island_cells.union(&bridge_cells).copied().collect();
    Ok(SkyLayout {
        primary_centres,
        satellite_centres: satellite_centres.to_vec(),
        island_cells,
        bridge_cells,
        bridge_lanes,
        upper_cells,
    })
}

fn bridge_between(
    grid_radius: u32,
    start: HexCoord,
    end: HexCoord,
) -> Result<[BTreeSet<HexCoord>; 2], CandidateAttemptError> {
    let line = start.line_between(end);
    let line_cells: BTreeSet<_> = line.iter().copied().collect();
    let midpoint = line
        .get(line.len().saturating_div(2))
        .copied()
        .ok_or_else(|| CandidateAttemptError::rejected("sky bridge has no centreline"))?;
    let lane_index = midpoint
        .neighbors()
        .into_iter()
        .enumerate()
        .find_map(|(index, neighbor)| (!line_cells.contains(&neighbor)).then_some(index))
        .ok_or_else(|| CandidateAttemptError::rejected("sky bridge has no parallel lane"))?;
    let first: BTreeSet<_> = line
        .iter()
        .copied()
        .filter(|coord| HexCoord::ORIGIN.distance(*coord) <= grid_radius)
        .collect();
    let second: BTreeSet<_> = line
        .iter()
        .filter_map(|coord| coord.neighbors().get(lane_index).copied())
        .filter(|coord| HexCoord::ORIGIN.distance(*coord) <= grid_radius)
        .collect();
    if first.len() != line.len() || second.len() != line.len() {
        return Err(CandidateAttemptError::rejected(
            "two-wide sky bridge escaped the map footprint",
        ));
    }
    Ok([first, second])
}

fn append_upper_mass(
    elements: &mut Vec<VolumeElement>,
    surface: Level,
    distance: u32,
    bridge: bool,
    environment: V2EnvironmentSettings,
    material_sample: u64,
) {
    if bridge {
        elements.push(VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(surface, surface.saturating_add(1)),
            material: SolidMaterialRole::Metal,
            cutaway_for: None,
        }));
        return;
    }

    let thickness = 4_i32.saturating_sub(i32::try_from(distance.min(2)).unwrap_or(2));
    let bottom = surface.saturating_add(1).saturating_sub(thickness);
    let top_material = match environment {
        V2EnvironmentSettings::TemperateGrassland => SolidMaterialRole::Grass,
        V2EnvironmentSettings::Frozen if material_sample.is_multiple_of(11) => {
            SolidMaterialRole::Ice
        }
        V2EnvironmentSettings::Frozen => SolidMaterialRole::Snow,
        V2EnvironmentSettings::Volcanic | V2EnvironmentSettings::Rocky => SolidMaterialRole::Stone,
    };
    let top = LevelInterval::new(surface, surface.saturating_add(1));
    if bottom < surface {
        elements.push(VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(bottom, surface),
            material: SolidMaterialRole::Stone,
            cutaway_for: None,
        }));
    }
    elements.push(VolumeElement::Solid(SolidMass {
        levels: top,
        material: top_material,
        cutaway_for: None,
    }));
}

fn validate_layered_plan(
    recipe: &LayeredSkyRecipe<'_>,
    settings: &LayeredSkyIslandsSettings,
    plan: &RecipePlan<SkyMetadata>,
) -> RecipeValidation<SkyMetrics> {
    let mut issues = Vec::new();
    let metadata = &plan.metadata;
    if metadata.primary_centres.len() != PRIMARY_ISLAND_COUNT {
        issues.push("upper layer does not contain exactly three primary islands".to_owned());
    }
    if !(1..=2).contains(&metadata.satellite_centres.len()) {
        issues.push("upper layer must contain one or two satellites".to_owned());
    }
    if metadata.bridge_lanes.len()
        != PRIMARY_ISLAND_COUNT
            .saturating_sub(1)
            .saturating_add(metadata.satellite_centres.len())
    {
        issues.push("upper layer bridge count does not match its island tree".to_owned());
    }
    if metadata.bridge_lanes.iter().any(|lanes| {
        let [first, second] = lanes;
        first.is_empty()
            || first.len() != second.len()
            || !first
                .iter()
                .all(|coord| metadata.upper_cells.contains(coord))
            || !second
                .iter()
                .all(|coord| metadata.upper_cells.contains(coord))
    }) {
        issues.push("an upper bridge is not a complete two-wide route".to_owned());
    }
    if !ground_is_unchanged(&recipe.ground.plan.volume, &plan.volume) {
        issues.push("upper construction changed finalized Hills ground semantics".to_owned());
    }
    if !metadata
        .island_cells
        .is_disjoint(&recipe.ground.plan.metadata.topology.protected_approaches)
    {
        issues.push("an island mass covers a protected Hills approach".to_owned());
    }

    let upper_columns = u32::try_from(metadata.island_cells.len()).unwrap_or(u32::MAX);
    let total_columns = footprint_size(plan.volume.grid_radius);
    let coverage_percent = upper_columns.saturating_mul(100) / total_columns.max(1);
    if !(15..=25).contains(&coverage_percent) {
        issues.push(format!(
            "upper coverage is {coverage_percent}%; expected 15% through 25%"
        ));
    }
    let expected_region_surfaces: BTreeSet<_> = metadata
        .upper_cells
        .iter()
        .copied()
        .map(|coord| TilePos::new(coord, metadata.upper_surface))
        .collect();
    let actual_region_surfaces: BTreeSet<_> = plan
        .volume
        .surfaces
        .iter()
        .filter_map(|(position, surface)| {
            (surface.access == SurfaceAccess::SpecialMovement(metadata.upper_region))
                .then_some(*position)
        })
        .collect();
    if actual_region_surfaces != expected_region_surfaces {
        issues.push("upper special-movement membership is not exact".to_owned());
    }

    for coord in &metadata.upper_cells {
        let Some(ground_column) = recipe.ground.plan.volume.columns.get(coord) else {
            issues.push(format!("upper column {coord:?} has no ground counterpart"));
            continue;
        };
        let Some(upper_column) = plan.volume.columns.get(coord) else {
            issues.push(format!("upper column {coord:?} is missing"));
            continue;
        };
        let ground_top = ground_column
            .elements
            .iter()
            .filter_map(|element| match element {
                VolumeElement::Solid(mass) => Some(mass.levels.top),
                VolumeElement::Fill(_) => None,
            })
            .max()
            .unwrap_or(0);
        let upper_bottom = upper_column
            .elements
            .iter()
            .filter_map(|element| match element {
                VolumeElement::Solid(mass) if mass.levels.top > ground_top => {
                    Some(mass.levels.bottom)
                }
                VolumeElement::Solid(_) | VolumeElement::Fill(_) => None,
            })
            .min()
            .unwrap_or(Level::MAX);
        if upper_bottom.saturating_sub(ground_top) < settings.min_clearance {
            issues.push(format!(
                "upper column {coord:?} has less than {} empty levels",
                settings.min_clearance
            ));
        }
    }

    if issues.is_empty() {
        RecipeValidation::valid(SkyMetrics {
            tactical: recipe.ground.plan.metadata.metrics.tactical,
            upper_columns,
            coverage_percent,
            satellite_count: u8::try_from(metadata.satellite_centres.len()).unwrap_or(u8::MAX),
            bridge_count: u8::try_from(metadata.bridge_lanes.len()).unwrap_or(u8::MAX),
        })
    } else {
        RecipeValidation::invalid(issues)
    }
}

fn ground_is_unchanged(
    ground: &super::volume::TerrainVolumePlan,
    layered: &super::volume::TerrainVolumePlan,
) -> bool {
    ground.anchors == layered.anchors
        && ground.interiors == layered.interiors
        && ground.surfaces.iter().all(|(position, metadata)| {
            layered
                .surfaces
                .get(position)
                .is_some_and(|actual| actual == metadata)
        })
        && ground.columns.iter().all(|(coord, column)| {
            layered.columns.get(coord).is_some_and(|actual| {
                actual.elements.len() >= column.elements.len()
                    && actual
                        .elements
                        .iter()
                        .zip(&column.elements)
                        .all(|(layered_element, ground_element)| layered_element == ground_element)
            })
        })
}

fn next_special_region(volume: &super::volume::TerrainVolumePlan) -> SpecialMovementRegion {
    let highest = volume
        .surfaces
        .values()
        .filter_map(|surface| match surface.access {
            SurfaceAccess::SpecialMovement(region) => Some(region.0),
            SurfaceAccess::Ordinary | SurfaceAccess::NonStandable => None,
        })
        .max()
        .unwrap_or(0);
    SpecialMovementRegion(highest.saturating_add(UPPER_REGION_OFFSET))
}

fn sky_view_hint(
    grid_radius: u32,
    level_height: f32,
    ground_surface: Level,
    upper_surface: Level,
) -> Result<MapViewHint, CandidateAttemptError> {
    let radius = u16::try_from(grid_radius)
        .map_err(|_out_of_range| CandidateAttemptError::rejected("sky radius is too large"))?;
    let ground = i16::try_from(ground_surface)
        .map_err(|_out_of_range| CandidateAttemptError::rejected("ground level is too large"))?;
    let upper = i16::try_from(upper_surface)
        .map_err(|_out_of_range| CandidateAttemptError::rejected("upper level is too large"))?;
    let focus_level = f32::from(ground.saturating_add(upper)) * 0.5;
    let focus_height = focus_level * level_height;
    let frame =
        (f32::from(radius) * 4.2).max(f32::from(upper.saturating_sub(ground)) * level_height * 3.0);
    Ok(MapViewHint::new(
        (0.0, focus_height + frame, frame),
        (0.0, focus_height, 0.0),
    ))
}

const fn rotate_third(coord: HexCoord, turns: u8) -> HexCoord {
    match turns % 3 {
        0 => coord,
        1 => HexCoord::from_axial(coord.z(), coord.x()),
        _ => HexCoord::from_axial(coord.y(), coord.z()),
    }
}

const fn footprint_size(radius: u32) -> u32 {
    1_u32.saturating_add(3_u32.saturating_mul(radius.saturating_mul(radius.saturating_add(1))))
}

#[cfg(test)]
mod tests {
    use hex_core::{MapAnchorId, SpecialMovementRegions};

    use super::*;
    use crate::settings::V2HillsSettings;

    const BEDROCK: SubstanceId = SubstanceId(1);
    const STONE: SubstanceId = SubstanceId(2);
    const DIRT: SubstanceId = SubstanceId(3);
    const GRASS: SubstanceId = SubstanceId(4);
    const GRAVEL: SubstanceId = SubstanceId(5);
    const WATER: SubstanceId = SubstanceId(6);
    const METAL: SubstanceId = SubstanceId(7);
    const SNOW: SubstanceId = SubstanceId(8);
    const ICE: SubstanceId = SubstanceId(9);
    const BASALT: SubstanceId = SubstanceId(10);
    const LAVA: SubstanceId = SubstanceId(11);
    const SKY_SEED: u64 = 94_445_606;

    fn palette() -> TerrainPalette {
        TerrainPalette {
            bedrock: BEDROCK,
            stone: STONE,
            dirt: DIRT,
            grass: GRASS,
            gravel: GRAVEL,
            water: WATER,
            metal: METAL,
            snow: SNOW,
            ice: ICE,
            basalt: BASALT,
            lava: LAVA,
        }
    }

    fn is_solid(substance: SubstanceId) -> bool {
        !matches!(substance, SubstanceId::AIR | WATER | LAVA)
    }

    fn settings(environment: V2EnvironmentSettings) -> ProceduralV2Settings {
        ProceduralV2Settings {
            environment,
            recipe: V2RecipeSettings::LayeredSkyIslands(LayeredSkyIslandsSettings {
                ground: V2HillsSettings {
                    valley_level: 15,
                    max_relief: 8,
                    hills_per_bank: 3,
                },
                min_clearance: 8,
                upper_coverage_percent: 20,
            }),
        }
    }

    fn ground_settings(environment: V2EnvironmentSettings) -> ProceduralV2Settings {
        ProceduralV2Settings {
            environment,
            recipe: V2RecipeSettings::Hills(V2HillsSettings {
                valley_level: 15,
                max_relief: 8,
                hills_per_bank: 3,
            }),
        }
    }

    fn sorted_regions(regions: &SpecialMovementRegions) -> Vec<(TilePos, SpecialMovementRegion)> {
        let mut memberships: Vec<_> = regions.iter().collect();
        memberships.sort_unstable();
        memberships
    }

    #[test]
    fn shipped_layered_sky_map_preserves_finalized_hills_ground() {
        let palette = palette();
        let sky = build(
            12,
            0.4,
            &settings(V2EnvironmentSettings::TemperateGrassland),
            SKY_SEED,
            &palette,
            &is_solid,
        )
        .expect("the shipped layered sky map should generate");
        let ground = hills::build(
            12,
            0.4,
            &ground_settings(V2EnvironmentSettings::TemperateGrassland),
            SKY_SEED,
            &palette,
            &is_solid,
        )
        .expect("the matching Hills ground should generate");

        assert_eq!(sky.map.len(), ground.map.len());
        let mut upper_columns = 0_usize;
        for (coord, ground_column) in ground.map.columns() {
            let layered_column = sky
                .map
                .column(coord)
                .expect("the upper plan must retain every ground column");
            upper_columns += usize::from(layered_column.top() > ground_column.top());
            assert_eq!(
                layered_column
                    .iter()
                    .take(ground_column.iter().len())
                    .collect::<Vec<_>>(),
                ground_column.iter().collect::<Vec<_>>(),
                "ground voxels changed at {coord:?}"
            );
        }
        assert_eq!(upper_columns, sky.metadata.upper_cells.len());
        for (name, position) in ground.anchors.iter() {
            assert_eq!(sky.anchors.get(name), Some(position));
        }

        let ground_regions: BTreeSet<_> = sorted_regions(&ground.special_regions)
            .into_iter()
            .collect();
        let layered_ground_regions: BTreeSet<_> = sorted_regions(&sky.special_regions)
            .into_iter()
            .filter(|(position, _region)| position.level < sky.metadata.upper_surface)
            .collect();
        assert_eq!(layered_ground_regions, ground_regions);
        assert!(sky.interiors.is_empty());
        assert!((15..=25).contains(&sky.metrics.coverage_percent));
        assert_eq!(sky.metadata.primary_centres.len(), 3);
        assert!((1..=2).contains(&sky.metadata.satellite_centres.len()));
        assert!(!sky.metadata.bridge_lanes.is_empty());
        assert!(sky.metadata.upper_cells.iter().all(|coord| sky
            .special_regions
            .get(TilePos::new(*coord, sky.metadata.upper_surface))
            == Some(sky.metadata.upper_region)));
        assert!(sky.anchors.get(&MapAnchorId::from("party_start")).is_some());
    }

    #[test]
    fn layered_sky_is_deterministic_and_scales_across_supported_radii() {
        for radius in [12, 20, 40] {
            let first = build(
                radius,
                0.4,
                &settings(V2EnvironmentSettings::Frozen),
                SKY_SEED,
                &palette(),
                &is_solid,
            )
            .expect("Frozen layered sky should generate");
            let second = build(
                radius,
                0.4,
                &settings(V2EnvironmentSettings::Frozen),
                SKY_SEED,
                &palette(),
                &is_solid,
            )
            .expect("the repeated map should generate");

            assert_eq!(first.map_fingerprint, second.map_fingerprint);
            assert_eq!(first.selected_candidate, second.selected_candidate);
            assert!((15..=25).contains(&first.metrics.coverage_percent));
            assert_eq!(first.candidates_evaluated, 8);
            assert!(!first.special_regions.is_empty());
        }
    }
}
