//! Layered sky islands built above a finalized V2 Hills ground plan.
//!
//! The ground selection is completed before any `sky.*` stream is sampled. Every
//! candidate clones that immutable semantic ground, appends an independent upper
//! network, and passes the complete volume through the common V2 validator.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, VecDeque};

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
    bridge_level: Level,
    primary_centres: [HexCoord; PRIMARY_ISLAND_COUNT],
    satellite_centres: Vec<HexCoord>,
    island_cells: BTreeSet<HexCoord>,
    bridge_rows: Vec<Vec<[HexCoord; 2]>>,
    upper_cells: BTreeSet<HexCoord>,
    upper_surfaces: BTreeMap<HexCoord, Level>,
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
            "candidate ground: selected {:?}; fallback={}",
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
    let topology = &recipe.ground.plan.metadata.topology;
    let route_exclusions = overhead_route_exclusions(topology);
    let layout = choose_layout(
        context,
        settings,
        &topology.protected_approaches,
        &route_exclusions,
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
    let bridge_level = lowest_upper
        .checked_add(1)
        .ok_or_else(|| CandidateAttemptError::rejected("upper island level overflowed"))?;
    let upper_region = next_special_region(&volume);
    let island_depths = island_depths(&layout.island_cells);
    let mut upper_surfaces = BTreeMap::new();

    for coord in &layout.upper_cells {
        let is_bridge = layout.bridge_cells.contains(coord) && !layout.island_cells.contains(coord);
        let relief = island_depths.get(coord).copied().unwrap_or(0).min(2);
        let upper_surface = bridge_level.saturating_add_unsigned(relief);
        let column = volume.columns.get_mut(coord).ok_or_else(|| {
            CandidateAttemptError::rejected(format!(
                "upper layout escaped the map footprint at {coord:?}"
            ))
        })?;
        append_upper_mass(
            &mut column.elements,
            bridge_level,
            upper_surface,
            relief,
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
        upper_surfaces.insert(*coord, upper_surface);
    }
    let highest_upper = upper_surfaces
        .values()
        .copied()
        .max()
        .unwrap_or(bridge_level);
    volume.view_hint = sky_view_hint(
        context.grid_radius,
        recipe.level_height,
        highest_ground,
        highest_upper,
    )?;

    Ok(RecipePlan {
        volume,
        metadata: SkyMetadata {
            upper_region,
            bridge_level,
            primary_centres: layout.primary_centres,
            satellite_centres: layout.satellite_centres,
            island_cells: layout.island_cells,
            bridge_rows: layout.bridge_rows,
            upper_cells: layout.upper_cells,
            upper_surfaces,
            ground_repair_actions: recipe.ground.plan.metadata.repair_actions.clone(),
        },
    })
}

struct SkyLayout {
    primary_centres: [HexCoord; PRIMARY_ISLAND_COUNT],
    satellite_centres: Vec<HexCoord>,
    island_cells: BTreeSet<HexCoord>,
    bridge_cells: BTreeSet<HexCoord>,
    bridge_rows: Vec<Vec<[HexCoord; 2]>>,
    upper_cells: BTreeSet<HexCoord>,
}

fn overhead_route_exclusions(topology: &crate::procedural::V1HillsTopology) -> BTreeSet<HexCoord> {
    let mut exclusions: BTreeSet<_> = topology
        .protected_approaches
        .difference(&topology.barrier)
        .copied()
        .collect();
    exclusions.extend(
        topology
            .bridge
            .iter()
            .chain(&topology.alternate_crossing)
            .map(|position| position.coord),
    );
    exclusions
}

fn choose_layout(
    context: CandidateContext,
    settings: &LayeredSkyIslandsSettings,
    protected_approaches: &BTreeSet<HexCoord>,
    route_exclusions: &BTreeSet<HexCoord>,
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
    let proposed_primary = [
        rotate_third(HexCoord::from_axial(-extent, 0), orientation),
        rotate_third(HexCoord::from_axial(extent, -extent), orientation),
        rotate_third(HexCoord::from_axial(0, extent), orientation),
    ];
    let [first_proposed, middle_proposed, last_proposed] = proposed_primary;
    let mut chosen = Vec::new();
    let first_primary = nearest_clear_centre(
        context.grid_radius,
        first_proposed,
        protected_approaches,
        &chosen,
    )?;
    chosen.push(first_primary);
    let middle_primary = nearest_clear_centre(
        context.grid_radius,
        middle_proposed,
        protected_approaches,
        &chosen,
    )?;
    chosen.push(middle_primary);
    let last_primary = nearest_clear_centre(
        context.grid_radius,
        last_proposed,
        protected_approaches,
        &chosen,
    )?;
    chosen.push(last_primary);
    let primary_centres = [first_primary, middle_primary, last_primary];
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
        rotate_third(HexCoord::from_axial(0, -extent), orientation),
        rotate_third(HexCoord::from_axial(extent, 0), orientation),
    ];
    let mut satellite_centres = Vec::new();
    for proposed in satellite_candidates.into_iter().take(satellite_count) {
        let centre =
            nearest_clear_centre(context.grid_radius, proposed, protected_approaches, &chosen)?;
        chosen.push(centre);
        satellite_centres.push(centre);
    }
    let bridge_rows = bridge_tree(
        context.grid_radius,
        primary_centres,
        &satellite_centres,
        route_exclusions,
    )?;

    let target_columns = footprint_size(context.grid_radius)
        .saturating_mul(u32::from(settings.upper_coverage_percent))
        .saturating_add(50)
        / 100;
    let max_island_radius = context.grid_radius.saturating_div(3).max(2);
    let mut layouts = Vec::new();
    for primary_radius in 1..=max_island_radius {
        let satellite_radius = primary_radius.saturating_div(2).max(1);
        let layout = match build_layout(
            context.grid_radius,
            primary_centres,
            &satellite_centres,
            primary_radius,
            satellite_radius,
            protected_approaches,
            &bridge_rows,
        ) {
            Ok(layout) => layout,
            Err(CandidateAttemptError::Rejected(_issues)) => continue,
            Err(CandidateAttemptError::Fatal(error)) => {
                return Err(CandidateAttemptError::Fatal(error));
            }
        };
        layouts.push(layout);
    }
    layouts
        .into_iter()
        .min_by_key(|layout| {
            (
                u32::try_from(layout.upper_cells.len())
                    .unwrap_or(u32::MAX)
                    .abs_diff(target_columns),
                layout.upper_cells.len(),
            )
        })
        .ok_or_else(|| CandidateAttemptError::rejected("no upper layout could be constructed"))
}

fn nearest_clear_centre(
    grid_radius: u32,
    proposed: HexCoord,
    blocked: &BTreeSet<HexCoord>,
    chosen: &[HexCoord],
) -> Result<HexCoord, CandidateAttemptError> {
    let minimum_separation = grid_radius.saturating_div(2).max(5);
    let target_clearance = grid_radius.saturating_div(5).max(2);
    for clearance in (1..=target_clearance).rev() {
        let maximum_distance = grid_radius.saturating_sub(clearance);
        let forbidden: BTreeSet<_> = blocked
            .iter()
            .flat_map(|coord| coord.within_radius(clearance))
            .collect();
        let centre = HexCoord::ORIGIN
            .within_radius(maximum_distance)
            .into_iter()
            .filter(|candidate| {
                !forbidden.contains(candidate)
                    && chosen
                        .iter()
                        .all(|other| other.distance(*candidate) >= minimum_separation)
            })
            .min_by_key(|candidate| (proposed.distance(*candidate), *candidate));
        if let Some(centre) = centre {
            return Ok(centre);
        }
    }
    Err(CandidateAttemptError::rejected(
        "protected ground approaches leave no separated sky island centre",
    ))
}

fn build_layout(
    grid_radius: u32,
    primary_centres: [HexCoord; PRIMARY_ISLAND_COUNT],
    satellite_centres: &[HexCoord],
    primary_radius: u32,
    satellite_radius: u32,
    protected_approaches: &BTreeSet<HexCoord>,
    bridge_rows: &[Vec<[HexCoord; 2]>],
) -> Result<SkyLayout, CandidateAttemptError> {
    let mut island_cells = BTreeSet::new();
    for centre in primary_centres {
        let footprint: BTreeSet<_> = centre
            .within_radius(primary_radius)
            .into_iter()
            .filter(|coord| HexCoord::ORIGIN.distance(*coord) <= grid_radius)
            .collect();
        if !footprint.is_disjoint(protected_approaches) {
            return Err(CandidateAttemptError::rejected(
                "primary island footprint overlaps a protected ground approach",
            ));
        }
        island_cells.extend(footprint);
    }
    for centre in satellite_centres {
        let footprint: BTreeSet<_> = centre
            .within_radius(satellite_radius)
            .into_iter()
            .filter(|coord| HexCoord::ORIGIN.distance(*coord) <= grid_radius)
            .collect();
        if !footprint.is_disjoint(protected_approaches) {
            return Err(CandidateAttemptError::rejected(
                "satellite footprint overlaps a protected ground approach",
            ));
        }
        island_cells.extend(footprint);
    }

    let bridge_cells = bridge_rows
        .iter()
        .flatten()
        .flat_map(|row| row.iter().copied())
        .collect();
    let upper_cells = island_cells.union(&bridge_cells).copied().collect();
    Ok(SkyLayout {
        primary_centres,
        satellite_centres: satellite_centres.to_vec(),
        island_cells,
        bridge_cells,
        bridge_rows: bridge_rows.to_vec(),
        upper_cells,
    })
}

fn bridge_tree(
    grid_radius: u32,
    primary_centres: [HexCoord; PRIMARY_ISLAND_COUNT],
    satellite_centres: &[HexCoord],
    route_exclusions: &BTreeSet<HexCoord>,
) -> Result<Vec<Vec<[HexCoord; 2]>>, CandidateAttemptError> {
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
    connections
        .into_iter()
        .map(|(start, end)| bridge_between(grid_radius, start, end, route_exclusions))
        .collect()
}

fn bridge_between(
    grid_radius: u32,
    start: HexCoord,
    end: HexCoord,
    protected_approaches: &BTreeSet<HexCoord>,
) -> Result<Vec<[HexCoord; 2]>, CandidateAttemptError> {
    paired_route(grid_radius, start, end, protected_approaches)
}

fn paired_route(
    grid_radius: u32,
    start: HexCoord,
    end: HexCoord,
    blocked: &BTreeSet<HexCoord>,
) -> Result<Vec<[HexCoord; 2]>, CandidateAttemptError> {
    if blocked.contains(&start) || blocked.contains(&end) {
        return Err(CandidateAttemptError::rejected(
            "sky island centre overlaps a protected ground approach",
        ));
    }

    let direct = start.line_between(end);
    if let Some(rows) = paired_rows_for_centerline(&direct, grid_radius, blocked) {
        return Ok(rows);
    }
    if let Some(centerline) = shortest_centerline(grid_radius, start, end, blocked) {
        if let Some(rows) = paired_rows_for_centerline(&centerline, grid_radius, blocked) {
            return Ok(rows);
        }
    }

    search_paired_route(grid_radius, start, end, blocked)
}

fn paired_rows_for_centerline(
    centerline: &[HexCoord],
    grid_radius: u32,
    blocked: &BTreeSet<HexCoord>,
) -> Option<Vec<[HexCoord; 2]>> {
    if centerline.is_empty()
        || centerline
            .iter()
            .any(|coord| !route_cell_is_open(*coord, grid_radius, blocked))
    {
        return None;
    }

    let mut layers = Vec::<BTreeMap<HexCoord, Option<HexCoord>>>::new();
    for centre in centerline {
        let previous = layers.last();
        let mut layer = BTreeMap::new();
        for candidate in centre.neighbors() {
            if !route_cell_is_open(candidate, grid_radius, blocked) {
                continue;
            }
            let predecessor = match previous {
                None => Some(None),
                Some(previous) => previous
                    .keys()
                    .find(|before| **before == candidate || before.distance(candidate) == 1)
                    .copied()
                    .map(Some),
            };
            if let Some(predecessor) = predecessor {
                layer.insert(candidate, predecessor);
            }
        }
        if layer.is_empty() {
            return None;
        }
        layers.push(layer);
    }

    let mut current = layers.last()?.keys().next().copied()?;
    let mut second_reversed = Vec::with_capacity(layers.len());
    for layer in layers.iter().rev() {
        second_reversed.push(current);
        let previous = layer.get(&current).copied()?;
        let Some(previous) = previous else {
            break;
        };
        current = previous;
    }
    if second_reversed.len() != layers.len() {
        return None;
    }
    second_reversed.reverse();
    Some(
        centerline
            .iter()
            .copied()
            .zip(second_reversed)
            .map(|(first, second)| [first, second])
            .collect(),
    )
}

fn shortest_centerline(
    grid_radius: u32,
    start: HexCoord,
    end: HexCoord,
    blocked: &BTreeSet<HexCoord>,
) -> Option<Vec<HexCoord>> {
    let mut parent = BTreeMap::new();
    let mut reached = BTreeSet::from([start]);
    let mut frontier = VecDeque::from([start]);
    while let Some(coord) = frontier.pop_front() {
        if coord == end {
            break;
        }
        for neighbor in coord.neighbors() {
            if !route_cell_is_open(neighbor, grid_radius, blocked) || !reached.insert(neighbor) {
                continue;
            }
            parent.insert(neighbor, coord);
            frontier.push_back(neighbor);
        }
    }
    if !reached.contains(&end) {
        return None;
    }
    let mut current = end;
    let mut reversed = vec![current];
    while current != start {
        current = parent.get(&current).copied()?;
        reversed.push(current);
    }
    reversed.reverse();
    Some(reversed)
}

fn search_paired_route(
    grid_radius: u32,
    start: HexCoord,
    end: HexCoord,
    blocked: &BTreeSet<HexCoord>,
) -> Result<Vec<[HexCoord; 2]>, CandidateAttemptError> {
    let mut frontier = BinaryHeap::new();
    let mut best_steps = BTreeMap::<[HexCoord; 2], u32>::new();
    let mut parent = BTreeMap::<[HexCoord; 2], [HexCoord; 2]>::new();
    for neighbor in start.neighbors() {
        if !route_cell_is_open(neighbor, grid_radius, blocked) {
            continue;
        }
        for row in [[start, neighbor], [neighbor, start]] {
            let estimate = row_distance_to(row, end);
            best_steps.insert(row, 0);
            frontier.push(Reverse((estimate, 0_u32, row)));
        }
    }

    let mut goal = None;
    while let Some(Reverse((_estimate, steps, row))) = frontier.pop() {
        if best_steps.get(&row).is_none_or(|best| steps != *best) {
            continue;
        }
        if row.contains(&end) {
            goal = Some(row);
            break;
        }

        let next_steps = steps.saturating_add(1);
        for next in next_bridge_rows(row, grid_radius, blocked) {
            if best_steps
                .get(&next)
                .is_some_and(|known| *known <= next_steps)
            {
                continue;
            }
            best_steps.insert(next, next_steps);
            parent.insert(next, row);
            let estimate = next_steps.saturating_add(row_distance_to(next, end));
            frontier.push(Reverse((estimate, next_steps, next)));
        }
    }

    let Some(mut current) = goal else {
        return Err(CandidateAttemptError::rejected(
            "protected ground approaches block a two-wide upper bridge",
        ));
    };
    let mut reversed = vec![current];
    while !current.contains(&start) {
        let Some(previous) = parent.get(&current).copied() else {
            return Err(CandidateAttemptError::rejected(
                "upper bridge route has incomplete parent metadata",
            ));
        };
        reversed.push(previous);
        current = previous;
    }
    reversed.reverse();
    Ok(reversed)
}

fn next_bridge_rows(
    row: [HexCoord; 2],
    grid_radius: u32,
    blocked: &BTreeSet<HexCoord>,
) -> Vec<[HexCoord; 2]> {
    let [first, second] = row;
    let mut first_steps = Vec::with_capacity(7);
    first_steps.push(first);
    first_steps.extend(first.neighbors());
    let mut second_steps = Vec::with_capacity(7);
    second_steps.push(second);
    second_steps.extend(second.neighbors());

    let mut rows = BTreeSet::new();
    for next_first in &first_steps {
        if !route_cell_is_open(*next_first, grid_radius, blocked) {
            continue;
        }
        for next_second in &second_steps {
            let next = [*next_first, *next_second];
            if next == row
                || next_first == next_second
                || next_first.distance(*next_second) != 1
                || !route_cell_is_open(*next_second, grid_radius, blocked)
            {
                continue;
            }
            rows.insert(next);
        }
    }
    rows.into_iter().collect()
}

fn route_cell_is_open(coord: HexCoord, grid_radius: u32, blocked: &BTreeSet<HexCoord>) -> bool {
    HexCoord::ORIGIN.distance(coord) <= grid_radius && !blocked.contains(&coord)
}

fn row_distance_to(row: [HexCoord; 2], target: HexCoord) -> u32 {
    row.into_iter()
        .map(|coord| coord.distance(target))
        .min()
        .unwrap_or(u32::MAX)
}

fn append_upper_mass(
    elements: &mut Vec<VolumeElement>,
    bridge_level: Level,
    surface: Level,
    relief: u32,
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

    let thickness = 2_i32.saturating_add_unsigned(relief);
    let bottom = surface.saturating_add(1).saturating_sub(thickness);
    debug_assert_eq!(bottom, bridge_level.saturating_sub(1));
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

fn island_depths(islands: &BTreeSet<HexCoord>) -> BTreeMap<HexCoord, u32> {
    let boundary: BTreeSet<_> = islands
        .iter()
        .copied()
        .filter(|coord| {
            coord
                .neighbors()
                .into_iter()
                .any(|neighbor| !islands.contains(&neighbor))
        })
        .collect();
    let mut depths: BTreeMap<HexCoord, u32> =
        boundary.iter().copied().map(|coord| (coord, 0)).collect();
    let mut frontier: VecDeque<_> = boundary.into_iter().collect();
    while let Some(coord) = frontier.pop_front() {
        let depth = depths.get(&coord).copied().unwrap_or(0);
        for neighbor in coord.neighbors() {
            if !islands.contains(&neighbor) || depths.contains_key(&neighbor) {
                continue;
            }
            depths.insert(neighbor, depth.saturating_add(1));
            frontier.push_back(neighbor);
        }
    }
    depths
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
    if metadata.bridge_rows.len()
        != PRIMARY_ISLAND_COUNT
            .saturating_sub(1)
            .saturating_add(metadata.satellite_centres.len())
    {
        issues.push("upper layer bridge count does not match its island tree".to_owned());
    }
    if metadata
        .bridge_rows
        .iter()
        .any(|rows| !valid_bridge_rows(rows, &metadata.upper_cells))
    {
        issues.push("an upper bridge is not a complete two-wide route".to_owned());
    }
    if !ground_is_unchanged(&recipe.ground.plan.volume, &plan.volume) {
        issues.push("upper construction changed finalized Hills ground semantics".to_owned());
    }
    if metadata.island_cells.is_empty() || !metadata.island_cells.is_subset(&metadata.upper_cells) {
        issues.push("island footprint is empty or escapes the upper network".to_owned());
    }
    let expected_island_components =
        PRIMARY_ISLAND_COUNT.saturating_add(metadata.satellite_centres.len());
    if connected_component_count(&metadata.island_cells) != expected_island_components {
        issues.push(format!(
            "island footprint does not contain {expected_island_components} distinct bodies"
        ));
    }
    if !metadata
        .island_cells
        .is_disjoint(&recipe.ground.plan.metadata.topology.protected_approaches)
    {
        issues.push("an island mass covers the river or a protected Hills approach".to_owned());
    }
    let bridge_cells: BTreeSet<_> = metadata
        .bridge_rows
        .iter()
        .flatten()
        .flat_map(|row| row.iter().copied())
        .collect();
    if !bridge_cells.is_disjoint(&overhead_route_exclusions(
        &recipe.ground.plan.metadata.topology,
    )) {
        issues.push("an upper bridge covers a protected ground landing or crossing".to_owned());
    }

    let upper_columns = u32::try_from(metadata.upper_cells.len()).unwrap_or(u32::MAX);
    let total_columns = footprint_size(plan.volume.grid_radius);
    let coverage_percent = upper_columns.saturating_mul(100) / total_columns.max(1);
    if !(15..=25).contains(&coverage_percent) {
        issues.push(format!(
            "upper coverage is {coverage_percent}%; expected 15% through 25%"
        ));
    }
    let expected_region_surfaces: BTreeSet<_> = metadata
        .upper_surfaces
        .iter()
        .map(|(coord, level)| TilePos::new(*coord, *level))
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
    if metadata.upper_surfaces.len() != metadata.upper_cells.len()
        || !metadata
            .upper_surfaces
            .keys()
            .all(|coord| metadata.upper_cells.contains(coord))
    {
        issues.push("upper surface levels do not match the upper footprint".to_owned());
    }
    if metadata.upper_surfaces.values().any(|surface| {
        !(metadata.bridge_level..=metadata.bridge_level.saturating_add(2)).contains(surface)
    }) {
        issues.push("upper island relief exceeds the supported zero-to-two range".to_owned());
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
            bridge_count: u8::try_from(metadata.bridge_rows.len()).unwrap_or(u8::MAX),
        })
    } else {
        RecipeValidation::invalid(issues)
    }
}

fn valid_bridge_rows(rows: &[[HexCoord; 2]], upper_cells: &BTreeSet<HexCoord>) -> bool {
    !rows.is_empty()
        && rows.iter().all(|row| {
            let [first, second] = *row;
            first != second
                && first.distance(second) == 1
                && upper_cells.contains(&first)
                && upper_cells.contains(&second)
        })
        && rows.windows(2).all(|pair| {
            let [before, after] = pair else {
                return false;
            };
            let [before_first, before_second] = *before;
            let [after_first, after_second] = *after;
            (before_first == after_first || before_first.distance(after_first) == 1)
                && (before_second == after_second || before_second.distance(after_second) == 1)
                && before != after
        })
}

fn connected_component_count(coords: &BTreeSet<HexCoord>) -> usize {
    let mut remaining = coords.clone();
    let mut components = 0_usize;
    while let Some(start) = remaining.pop_first() {
        components = components.saturating_add(1);
        let mut frontier = VecDeque::from([start]);
        while let Some(coord) = frontier.pop_front() {
            for neighbor in coord.neighbors() {
                if remaining.remove(&neighbor) {
                    frontier.push_back(neighbor);
                }
            }
        }
    }
    components
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
    use std::time::Instant;

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
        assert_eq!(sky.selected_candidate, Some(3));
        assert_eq!(sky.map_fingerprint, 4_313_975_567_675_515_163);
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
            .filter(|(position, _region)| position.level < sky.metadata.bridge_level)
            .collect();
        assert_eq!(layered_ground_regions, ground_regions);
        assert!(sky.interiors.is_empty());
        assert!((15..=25).contains(&sky.metrics.coverage_percent));
        assert_eq!(sky.metadata.primary_centres.len(), 3);
        assert!((1..=2).contains(&sky.metadata.satellite_centres.len()));
        assert!(!sky.metadata.bridge_rows.is_empty());
        assert_eq!(
            connected_component_count(&sky.metadata.island_cells),
            PRIMARY_ISLAND_COUNT + sky.metadata.satellite_centres.len()
        );
        assert!(sky
            .metadata
            .bridge_rows
            .iter()
            .all(|rows| valid_bridge_rows(rows, &sky.metadata.upper_cells)));
        assert!(sky.metadata.upper_cells.iter().all(|coord| {
            let level = sky
                .metadata
                .upper_surfaces
                .get(coord)
                .copied()
                .expect("every upper coordinate should have one exact surface");
            sky.special_regions.get(TilePos::new(*coord, level)) == Some(sky.metadata.upper_region)
        }));
        let bridge_only: BTreeSet<_> = sky
            .metadata
            .bridge_rows
            .iter()
            .flatten()
            .flat_map(|row| row.iter().copied())
            .filter(|coord| !sky.metadata.island_cells.contains(coord))
            .collect();
        assert!(!bridge_only.is_empty());
        assert!(bridge_only.iter().all(|coord| {
            let surface = sky
                .metadata
                .upper_surfaces
                .get(coord)
                .copied()
                .expect("every bridge coordinate should have an upper surface");
            surface == sky.metadata.bridge_level
                && sky.map.get(TilePos::new(*coord, surface)) == METAL
                && sky
                    .map
                    .get(TilePos::new(*coord, surface.saturating_add(1)))
                    .is_air()
        }));
        assert!(sky.metadata.island_cells.iter().all(|coord| {
            let surface = sky
                .metadata
                .upper_surfaces
                .get(coord)
                .copied()
                .expect("every island coordinate should have an upper surface");
            sky.map.get(TilePos::new(*coord, surface)) == GRASS
        }));
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
            .unwrap_or_else(|error| {
                panic!("Frozen layered sky radius {radius} should generate: {error}")
            });
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

    #[test]
    fn fixed_seed_corpus_is_valid_without_fallback() {
        for environment in [
            V2EnvironmentSettings::TemperateGrassland,
            V2EnvironmentSettings::Frozen,
        ] {
            for seed in [0, 1, 505, 808, 20_260_726, SKY_SEED, u64::MAX] {
                let generated = build(12, 0.4, &settings(environment), seed, &palette(), &is_solid)
                    .unwrap_or_else(|error| {
                        panic!("{environment:?} seed {seed} should generate: {error}")
                    });

                assert!(
                    !generated.used_fallback,
                    "{environment:?} seed {seed} unexpectedly used the canonical fallback"
                );
                assert!(generated.valid_candidates > 0);
                assert!(generated.repair_actions.len() <= 4);
                assert_eq!(
                    connected_component_count(&generated.metadata.island_cells),
                    PRIMARY_ISLAND_COUNT + generated.metadata.satellite_centres.len()
                );
                assert!((15..=25).contains(&generated.metrics.coverage_percent));
            }
        }
    }

    #[test]
    fn canonical_upper_layout_validates_over_representative_finalized_ground() {
        let palette = palette();
        let sky_settings = settings(V2EnvironmentSettings::TemperateGrassland);
        let V2RecipeSettings::LayeredSkyIslands(layered_settings) = &sky_settings.recipe else {
            unreachable!("the helper always constructs layered sky settings")
        };

        for seed in [0, 20_260_726, SKY_SEED, u64::MAX] {
            let finalized_ground = hills::select(
                12,
                0.4,
                &ground_settings(V2EnvironmentSettings::TemperateGrassland),
                seed,
                &palette,
                &is_solid,
            )
            .expect("the representative Hills ground should select")
            .into_unvalidated();
            let recipe = LayeredSkyRecipe {
                ground: &finalized_ground,
                environment: V2EnvironmentSettings::TemperateGrassland,
                level_height: 0.4,
            };
            let fallback = recipe
                .canonical_fallback(FallbackContext { grid_radius: 12 }, layered_settings)
                .unwrap_or_else(|error| {
                    panic!("seed {seed} canonical upper layout should construct: {error}")
                });

            fallback
                .volume
                .validate()
                .expect("the canonical upper layout should pass common volume validation");
            assert!(matches!(
                validate_layered_plan(&recipe, layered_settings, &fallback),
                RecipeValidation::Valid(_)
            ));
        }
    }

    #[test]
    fn recipe_validation_rejects_corrupt_upper_semantics() {
        let palette = palette();
        let sky_settings = settings(V2EnvironmentSettings::TemperateGrassland);
        let V2RecipeSettings::LayeredSkyIslands(layered_settings) = &sky_settings.recipe else {
            unreachable!("the helper always constructs layered sky settings")
        };
        let finalized_ground = hills::select(
            12,
            0.4,
            &ground_settings(V2EnvironmentSettings::TemperateGrassland),
            SKY_SEED,
            &palette,
            &is_solid,
        )
        .expect("the Hills ground should select")
        .into_unvalidated();
        let recipe = LayeredSkyRecipe {
            ground: &finalized_ground,
            environment: V2EnvironmentSettings::TemperateGrassland,
            level_height: 0.4,
        };
        let context = CandidateContext {
            grid_radius: 12,
            candidate: 0,
            streams: super::super::seed::SeedStreams::new(SKY_SEED, 0),
        };
        let valid = layered_plan(&recipe, context, layered_settings, false)
            .expect("the fixed candidate should construct");

        let mut broken_lane = valid.clone();
        let first_row = broken_lane
            .metadata
            .bridge_rows
            .first_mut()
            .and_then(|rows| rows.first_mut())
            .expect("the valid candidate should have a bridge row");
        let [first, _second] = *first_row;
        *first_row = [first, first];
        assert!(validation_issues(&recipe, layered_settings, &broken_lane)
            .iter()
            .any(|issue| issue.contains("two-wide route")));

        let mut missing_membership = valid.clone();
        let (&upper_coord, &upper_level) = missing_membership
            .metadata
            .upper_surfaces
            .iter()
            .next()
            .expect("the candidate should have upper surfaces");
        missing_membership
            .volume
            .surfaces
            .remove(&TilePos::new(upper_coord, upper_level));
        assert!(
            validation_issues(&recipe, layered_settings, &missing_membership)
                .iter()
                .any(|issue| issue.contains("membership is not exact"))
        );

        let mut changed_ground = valid;
        changed_ground
            .volume
            .anchors
            .remove(crate::procedural::PARTY_START);
        assert!(
            validation_issues(&recipe, layered_settings, &changed_ground)
                .iter()
                .any(|issue| issue.contains("ground semantics"))
        );
    }

    fn validation_issues(
        recipe: &LayeredSkyRecipe<'_>,
        settings: &LayeredSkyIslandsSettings,
        plan: &RecipePlan<SkyMetadata>,
    ) -> Vec<String> {
        match validate_layered_plan(recipe, settings, plan) {
            RecipeValidation::Valid(_metrics) => Vec::new(),
            RecipeValidation::Invalid(issues) => issues,
        }
    }

    #[test]
    #[ignore = "10,000 seeds are a manual stress corpus"]
    fn ten_thousand_seed_corpus_has_less_than_one_percent_fallbacks() {
        let mut fallback_count = 0_u32;
        for seed in 0..10_000 {
            let generated = build(
                12,
                0.4,
                &settings(V2EnvironmentSettings::TemperateGrassland),
                seed,
                &palette(),
                &is_solid,
            )
            .unwrap_or_else(|error| panic!("seed {seed} should generate: {error}"));
            fallback_count += u32::from(generated.used_fallback);
        }
        assert!(
            fallback_count < 100,
            "{fallback_count} of 10,000 maps used fallback"
        );
    }

    #[test]
    #[ignore = "manual release/debug generator benchmark"]
    fn layered_sky_radius_benchmark_tracks_the_radius_40_target() {
        let palette = palette();
        let settings = settings(V2EnvironmentSettings::TemperateGrassland);
        let mut radius_40_median = 0_u128;
        for radius in [12, 20, 40] {
            let mut samples = Vec::new();
            for seed in 0..8 {
                let started = Instant::now();
                let generated = build(radius, 0.4, &settings, seed, &palette, &is_solid)
                    .expect("the benchmark map should generate");
                samples.push(started.elapsed().as_micros());
                std::hint::black_box(generated);
            }
            samples.sort_unstable();
            let median = samples
                .get(samples.len() / 2)
                .copied()
                .expect("the benchmark always records eight samples");
            eprintln!("Layered Sky radius {radius}: median={median}us");
            if radius == 40 {
                radius_40_median = median;
            }
        }

        let target_micros = if cfg!(debug_assertions) {
            250_000
        } else {
            50_000
        };
        assert!(
            radius_40_median < target_micros,
            "Layered Sky radius 40 median was {radius_40_median}us; target is {target_micros}us"
        );
    }
}
