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
const LEGACY_MIN_CLEARANCE: Level = 8;
const MAX_ELEVATED_UNDERBODY_BUDGET: Level = 12;
const MAX_ELEVATED_RELIEF_CAP: u32 = 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SkyVerticalBudget {
    underbody: Level,
    relief: u32,
}

fn vertical_budget(min_clearance: Level) -> SkyVerticalBudget {
    if min_clearance <= LEGACY_MIN_CLEARANCE {
        return SkyVerticalBudget {
            underbody: 1,
            relief: 2,
        };
    }
    let extra = min_clearance.saturating_sub(LEGACY_MIN_CLEARANCE);
    SkyVerticalBudget {
        underbody: 1_i32
            .saturating_add(extra / 2)
            .min(MAX_ELEVATED_UNDERBODY_BUDGET),
        relief: 2_u32
            .saturating_add(u32::try_from(extra / 4).unwrap_or(u32::MAX))
            .min(MAX_ELEVATED_RELIEF_CAP),
    }
}

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
    island_bodies: Vec<BTreeSet<HexCoord>>,
    island_cells: BTreeSet<HexCoord>,
    bridge_rows: Vec<Vec<[HexCoord; 2]>>,
    upper_cells: BTreeSet<HexCoord>,
    upper_surfaces: BTreeMap<HexCoord, Level>,
    upper_bottoms: BTreeMap<HexCoord, Level>,
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
    let elevated = settings.min_clearance > LEGACY_MIN_CLEARANCE;
    let vertical_budget = vertical_budget(settings.min_clearance);
    let bridge_level = lowest_upper
        .checked_add(vertical_budget.underbody)
        .ok_or_else(|| CandidateAttemptError::rejected("upper island level overflowed"))?;
    let upper_region = next_special_region(&volume);
    let reliefs = if elevated {
        landing_distance_reliefs(&layout, vertical_budget.relief)?
    } else {
        island_depths(&layout.island_cells)
            .into_iter()
            .map(|(coord, depth)| (coord, depth.min(2)))
            .collect()
    };
    let mut upper_surfaces = BTreeMap::new();
    let mut upper_bottoms = BTreeMap::new();

    for coord in &layout.upper_cells {
        let is_bridge = layout.bridge_cells.contains(coord) && !layout.island_cells.contains(coord);
        let relief = reliefs.get(coord).copied().unwrap_or(0);
        let upper_surface = bridge_level.saturating_add_unsigned(relief);
        let underside_depth = if is_bridge {
            0
        } else if elevated {
            tapered_underbody_depth(
                relief,
                context
                    .streams
                    .stage("sky.shape.underside")
                    .sample_coord(*coord, 0),
                vertical_budget.underbody,
            )
        } else {
            1
        };
        let upper_bottom = if is_bridge {
            upper_surface
        } else {
            bridge_level.saturating_sub(underside_depth)
        };
        let column = volume.columns.get_mut(coord).ok_or_else(|| {
            CandidateAttemptError::rejected(format!(
                "upper layout escaped the map footprint at {coord:?}"
            ))
        })?;
        append_upper_mass(
            &mut column.elements,
            bridge_level,
            upper_surface,
            underside_depth,
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
        upper_bottoms.insert(*coord, upper_bottom);
    }
    let highest_upper = upper_surfaces
        .values()
        .copied()
        .max()
        .unwrap_or(bridge_level);
    let lowest_upper = upper_bottoms
        .values()
        .copied()
        .min()
        .unwrap_or(bridge_level);
    volume.view_hint = sky_view_hint(
        context.grid_radius,
        recipe.level_height,
        highest_ground,
        lowest_upper,
        highest_upper,
    )?;

    Ok(RecipePlan {
        volume,
        metadata: SkyMetadata {
            upper_region,
            bridge_level,
            primary_centres: layout.primary_centres,
            satellite_centres: layout.satellite_centres,
            island_bodies: layout.island_bodies,
            island_cells: layout.island_cells,
            bridge_rows: layout.bridge_rows,
            upper_cells: layout.upper_cells,
            upper_surfaces,
            upper_bottoms,
            ground_repair_actions: recipe.ground.plan.metadata.repair_actions.clone(),
        },
    })
}

struct SkyLayout {
    primary_centres: [HexCoord; PRIMARY_ISLAND_COUNT],
    satellite_centres: Vec<HexCoord>,
    island_bodies: Vec<BTreeSet<HexCoord>>,
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
        let constructed = if settings.min_clearance > LEGACY_MIN_CLEARANCE {
            build_elevated_layout(
                context,
                primary_centres,
                &satellite_centres,
                primary_radius,
                satellite_radius,
                protected_approaches,
                &bridge_rows,
            )
        } else {
            build_layout(
                context.grid_radius,
                primary_centres,
                &satellite_centres,
                primary_radius,
                satellite_radius,
                protected_approaches,
                &bridge_rows,
            )
        };
        let layout = match constructed {
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
    build_layout_with_radii(IslandLayoutInputs {
        grid_radius,
        primary_centres,
        satellite_centres,
        primary_radii: [primary_radius; PRIMARY_ISLAND_COUNT],
        satellite_radii: vec![satellite_radius; satellite_centres.len()],
        protected_approaches,
        bridge_rows,
        edge_stream: None,
    })
}

fn build_elevated_layout(
    context: CandidateContext,
    primary_centres: [HexCoord; PRIMARY_ISLAND_COUNT],
    satellite_centres: &[HexCoord],
    primary_radius: u32,
    satellite_radius: u32,
    protected_approaches: &BTreeSet<HexCoord>,
    bridge_rows: &[Vec<[HexCoord; 2]>],
) -> Result<SkyLayout, CandidateAttemptError> {
    let primary_radii = varied_primary_radii(
        primary_radius,
        context.streams.stage("sky.shape.radii").sample(0),
    );
    let satellite_radii =
        varied_satellite_radii(satellite_centres.len(), satellite_radius, context);
    build_layout_with_radii(IslandLayoutInputs {
        grid_radius: context.grid_radius,
        primary_centres,
        satellite_centres,
        primary_radii,
        satellite_radii,
        protected_approaches,
        bridge_rows,
        edge_stream: Some(context.streams.stage("sky.shape.edge")),
    })
}

struct IslandLayoutInputs<'a> {
    grid_radius: u32,
    primary_centres: [HexCoord; PRIMARY_ISLAND_COUNT],
    satellite_centres: &'a [HexCoord],
    primary_radii: [u32; PRIMARY_ISLAND_COUNT],
    satellite_radii: Vec<u32>,
    protected_approaches: &'a BTreeSet<HexCoord>,
    bridge_rows: &'a [Vec<[HexCoord; 2]>],
    edge_stream: Option<super::seed::SeedStream<'a>>,
}

fn build_layout_with_radii(
    inputs: IslandLayoutInputs<'_>,
) -> Result<SkyLayout, CandidateAttemptError> {
    let IslandLayoutInputs {
        grid_radius,
        primary_centres,
        satellite_centres,
        primary_radii,
        satellite_radii,
        protected_approaches,
        bridge_rows,
        edge_stream,
    } = inputs;
    let bridge_cells = bridge_rows
        .iter()
        .flatten()
        .flat_map(|row| row.iter().copied())
        .collect::<BTreeSet<_>>();
    let bodies = primary_centres
        .into_iter()
        .zip(primary_radii)
        .map(|(centre, radius)| (centre, radius, "primary"))
        .chain(
            satellite_centres
                .iter()
                .copied()
                .zip(satellite_radii)
                .map(|(centre, radius)| (centre, radius, "satellite")),
        );
    let mut island_bodies = Vec::new();
    let mut island_cells = BTreeSet::new();
    let elevated = edge_stream.is_some();
    for (index, (centre, requested_radius, label)) in bodies.enumerate() {
        let mut radius = requested_radius;
        let footprint = loop {
            let mut footprint: BTreeSet<_> = centre
                .within_radius(radius)
                .into_iter()
                .filter(|coord| HexCoord::ORIGIN.distance(*coord) <= grid_radius)
                .collect();
            if let Some(edge_stream) = edge_stream {
                footprint.retain(|coord| {
                    centre.distance(*coord) < radius
                        || bridge_cells.contains(coord)
                        || !edge_stream
                            .sample_coord(*coord, u64::try_from(index).unwrap_or(u64::MAX))
                            .is_multiple_of(5)
                });
            }
            let overlaps_protected = !footprint.is_disjoint(protected_approaches);
            let touches_another = footprint.iter().any(|coord| {
                island_cells.contains(coord)
                    || coord
                        .neighbors()
                        .into_iter()
                        .any(|neighbor| island_cells.contains(&neighbor))
            });
            if !overlaps_protected && (!elevated || !touches_another) {
                break footprint;
            }
            if elevated && radius > 1 {
                radius = radius.saturating_sub(1);
                continue;
            }
            let reason = if overlaps_protected {
                format!("{label} island footprint overlaps a protected ground approach")
            } else {
                "island footprints touch before their bridges are added".to_owned()
            };
            return Err(CandidateAttemptError::rejected(reason));
        };
        if footprint.is_empty() || !footprint.contains(&centre) {
            return Err(CandidateAttemptError::rejected(format!(
                "{label} island footprint lost its centre"
            )));
        }
        island_cells.extend(&footprint);
        island_bodies.push(footprint);
    }

    let upper_cells = island_cells.union(&bridge_cells).copied().collect();
    Ok(SkyLayout {
        primary_centres,
        satellite_centres: satellite_centres.to_vec(),
        island_bodies,
        island_cells,
        bridge_cells,
        bridge_rows: bridge_rows.to_vec(),
        upper_cells,
    })
}

fn varied_primary_radii(nominal: u32, sample: u64) -> [u32; PRIMARY_ISLAND_COUNT] {
    let radii = [
        nominal.saturating_add(1),
        nominal,
        nominal.saturating_sub(1).max(1),
    ];
    match sample % 3 {
        0 => radii,
        1 => [radii[2], radii[0], radii[1]],
        _ => [radii[1], radii[2], radii[0]],
    }
}

fn varied_satellite_radii(count: usize, nominal: u32, context: CandidateContext) -> Vec<u32> {
    let mut radii = vec![nominal; count];
    if count > 1 && nominal > 1 {
        let smaller = usize::try_from(
            context.streams.stage("sky.shape.satellite_radii").sample(0)
                % u64::try_from(count).unwrap_or(1),
        )
        .unwrap_or_default();
        if let Some(radius) = radii.get_mut(smaller) {
            *radius = radius.saturating_sub(1).max(1);
        }
    }
    radii
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
    underside_depth: Level,
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

    let bottom = bridge_level.saturating_sub(underside_depth);
    debug_assert!(underside_depth > 0);
    debug_assert!(bottom <= surface);
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

fn landing_distance_reliefs(
    layout: &SkyLayout,
    relief_cap: u32,
) -> Result<BTreeMap<HexCoord, u32>, CandidateAttemptError> {
    let mut reliefs = BTreeMap::new();
    let bridge_only: BTreeSet<_> = layout
        .bridge_cells
        .difference(&layout.island_cells)
        .copied()
        .collect();
    for body in &layout.island_bodies {
        let mut landings: Vec<_> = body
            .intersection(&layout.bridge_cells)
            .copied()
            .filter(|coord| {
                coord
                    .neighbors()
                    .into_iter()
                    .any(|neighbor| bridge_only.contains(&neighbor))
            })
            .collect();
        if landings.is_empty() {
            landings.extend(body.intersection(&layout.bridge_cells).copied());
        }
        if landings.is_empty() {
            return Err(CandidateAttemptError::rejected(
                "an elevated island has no bridge landing",
            ));
        }

        let mut distances: BTreeMap<_, _> = landings
            .iter()
            .copied()
            .map(|coord| (coord, 0_u32))
            .collect();
        let mut frontier: VecDeque<_> = landings.into_iter().collect();
        while let Some(coord) = frontier.pop_front() {
            let distance = distances.get(&coord).copied().unwrap_or(0);
            for neighbor in coord.neighbors() {
                if !body.contains(&neighbor) || distances.contains_key(&neighbor) {
                    continue;
                }
                distances.insert(neighbor, distance.saturating_add(1));
                frontier.push_back(neighbor);
            }
        }
        if distances.len() != body.len() {
            return Err(CandidateAttemptError::rejected(
                "an elevated island silhouette is disconnected from its bridge landing",
            ));
        }
        for (coord, distance) in distances {
            if reliefs.insert(coord, distance.min(relief_cap)).is_some() {
                return Err(CandidateAttemptError::rejected(
                    "elevated island bodies overlap",
                ));
            }
        }
    }
    Ok(reliefs)
}

fn tapered_underbody_depth(relief: u32, sample: u64, budget: Level) -> Level {
    let relief = Level::try_from(relief).unwrap_or(Level::MAX);
    budget
        .saturating_div(2)
        .max(1)
        .saturating_add(relief)
        .saturating_add(i32::from(!sample.is_multiple_of(2)))
        .min(budget)
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
    if metadata.island_bodies.len() != expected_island_components {
        issues.push(format!(
            "upper layer records {} island bodies; expected {expected_island_components}",
            metadata.island_bodies.len()
        ));
    }
    let recorded_island_cells: BTreeSet<_> = metadata
        .island_bodies
        .iter()
        .flat_map(|body| body.iter().copied())
        .collect();
    if recorded_island_cells != metadata.island_cells
        || metadata
            .island_bodies
            .iter()
            .any(|body| connected_component_count(body) != 1)
    {
        issues.push("recorded island bodies do not exactly partition the footprint".to_owned());
    }
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
    if metadata.upper_bottoms.len() != metadata.upper_cells.len()
        || !metadata
            .upper_bottoms
            .keys()
            .all(|coord| metadata.upper_cells.contains(coord))
    {
        issues.push("upper mass bottoms do not match the upper footprint".to_owned());
    }
    let vertical_budget = vertical_budget(settings.min_clearance);
    let relief_cap = vertical_budget.relief;
    if metadata.upper_surfaces.values().any(|surface| {
        !(metadata.bridge_level..=metadata.bridge_level.saturating_add_unsigned(relief_cap))
            .contains(surface)
    }) {
        issues.push(format!(
            "upper island relief exceeds the supported zero-to-{relief_cap} range"
        ));
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
        if metadata.upper_bottoms.get(coord).copied() != Some(upper_bottom) {
            issues.push(format!(
                "upper column {coord:?} bottom does not match its exact metadata"
            ));
        }
        if upper_bottom.saturating_sub(ground_top) < settings.min_clearance {
            issues.push(format!(
                "upper column {coord:?} has less than {} empty levels",
                settings.min_clearance
            ));
        }
        let bridge_only = bridge_cells.contains(coord) && !metadata.island_cells.contains(coord);
        let underside_depth = metadata.bridge_level.saturating_sub(upper_bottom);
        if bridge_only
            && (upper_bottom != metadata.bridge_level
                || metadata.upper_surfaces.get(coord).copied() != Some(metadata.bridge_level))
        {
            issues.push(format!(
                "upper bridge column {coord:?} is not a flat one-voxel deck"
            ));
        } else if !bridge_only {
            let maximum_depth = vertical_budget.underbody;
            if !(1..=maximum_depth).contains(&underside_depth) {
                issues.push(format!(
                    "upper island column {coord:?} has unsupported underbody depth \
                     {underside_depth}"
                ));
            }
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
    lowest_upper: Level,
    upper_surface: Level,
) -> Result<MapViewHint, CandidateAttemptError> {
    let radius = u16::try_from(grid_radius)
        .map_err(|_out_of_range| CandidateAttemptError::rejected("sky radius is too large"))?;
    let ground = i16::try_from(ground_surface)
        .map_err(|_out_of_range| CandidateAttemptError::rejected("ground level is too large"))?;
    let upper = i16::try_from(upper_surface)
        .map_err(|_out_of_range| CandidateAttemptError::rejected("upper level is too large"))?;
    let lowest_upper = i16::try_from(lowest_upper)
        .map_err(|_out_of_range| CandidateAttemptError::rejected("upper base is too large"))?;
    let lowest = ground.min(lowest_upper);
    let focus_level = f32::from(lowest.saturating_add(upper)) * 0.5;
    let focus_height = focus_level * level_height;
    let frame =
        (f32::from(radius) * 4.2).max(f32::from(upper.saturating_sub(lowest)) * level_height * 3.0);
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
            worked_stone: SubstanceId(12),
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
        settings_with_geometry(environment, LEGACY_MIN_CLEARANCE, 20)
    }

    fn settings_with_geometry(
        environment: V2EnvironmentSettings,
        min_clearance: Level,
        upper_coverage_percent: u8,
    ) -> ProceduralV2Settings {
        ProceduralV2Settings {
            environment,
            recipe: V2RecipeSettings::LayeredSkyIslands(LayeredSkyIslandsSettings {
                ground: V2HillsSettings {
                    valley_level: 15,
                    max_relief: 8,
                    hills_per_bank: 3,
                },
                min_clearance,
                upper_coverage_percent,
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
    fn legacy_layout_defers_touching_island_rejection_to_validation() {
        let centres = [
            HexCoord::from_axial(-2, 0),
            HexCoord::from_axial(2, 0),
            HexCoord::from_axial(0, 6),
        ];
        let layout = build_layout(12, centres, &[], 2, 1, &BTreeSet::new(), &[])
            .expect("legacy construction should union touching bodies before validation");
        let expected: BTreeSet<_> = centres
            .into_iter()
            .flat_map(|centre| centre.within_radius(2))
            .collect();

        assert_eq!(layout.island_cells, expected);
        assert_eq!(layout.island_bodies.len(), PRIMARY_ISLAND_COUNT);
        assert!(
            connected_component_count(&layout.island_cells) < PRIMARY_ISLAND_COUNT,
            "the fixture must reach the legacy validation-time rejection"
        );
    }

    #[test]
    fn elevated_vertical_budget_is_monotonic_and_bounded() {
        let mut previous = vertical_budget(LEGACY_MIN_CLEARANCE);
        for min_clearance in LEGACY_MIN_CLEARANCE.saturating_add(1)..=128 {
            let budget = vertical_budget(min_clearance);
            assert!(budget.underbody >= previous.underbody);
            assert!(budget.relief >= previous.relief);
            assert!(
                budget.underbody.saturating_add_unsigned(budget.relief) <= 19,
                "one level remains for the ground-to-gap boundary inside the 20-level reservation"
            );
            previous = budget;
        }
    }

    #[test]
    fn legacy_layered_sky_compatibility_map_preserves_finalized_hills_ground() {
        let palette = palette();
        let sky = build(
            12,
            0.4,
            &settings(V2EnvironmentSettings::TemperateGrassland),
            SKY_SEED,
            &palette,
            &is_solid,
        )
        .expect("the legacy layered sky compatibility map should generate");
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
    fn shipped_layered_sky_selection_is_pinned() {
        let generated = build(
            12,
            0.4,
            &settings_with_geometry(V2EnvironmentSettings::TemperateGrassland, 22, 24),
            SKY_SEED,
            &palette(),
            &is_solid,
        )
        .expect("the shipped layered sky selection should generate");

        assert_eq!(generated.selected_candidate, Some(2));
        assert_eq!(generated.map_fingerprint, 13_919_513_730_444_748_723);
        assert_eq!(generated.metrics.coverage_percent, 24);
        assert!(!generated.used_fallback);
    }

    #[test]
    fn elevated_settings_are_deterministic_varied_and_clear_of_ground() {
        let settings_cases = [(14, 18, 4, 3), (18, 21, 6, 4), (22, 24, 8, 5)];
        for (min_clearance, coverage, expected_underbody, expected_relief) in settings_cases {
            assert_eq!(
                vertical_budget(min_clearance),
                SkyVerticalBudget {
                    underbody: expected_underbody,
                    relief: expected_relief,
                }
            );
            let settings = settings_with_geometry(
                V2EnvironmentSettings::TemperateGrassland,
                min_clearance,
                coverage,
            );
            let first = build(12, 0.4, &settings, SKY_SEED, &palette(), &is_solid).unwrap_or_else(
                |error| {
                    panic!(
                        "elevated settings clearance={min_clearance}, coverage={coverage} \
                     should generate: {error}"
                    )
                },
            );
            let second = build(12, 0.4, &settings, SKY_SEED, &palette(), &is_solid)
                .expect("the repeated elevated map should generate");
            let ground = hills::build(
                12,
                0.4,
                &ground_settings(V2EnvironmentSettings::TemperateGrassland),
                SKY_SEED,
                &palette(),
                &is_solid,
            )
            .expect("the matching finalized Hills ground should generate");

            assert_eq!(first.map_fingerprint, second.map_fingerprint);
            assert_eq!(first.selected_candidate, second.selected_candidate);
            assert!(!first.used_fallback);
            assert!((15..=25).contains(&first.metrics.coverage_percent));

            let body_sizes: BTreeSet<_> = first
                .metadata
                .island_bodies
                .iter()
                .map(BTreeSet::len)
                .collect();
            assert!(
                body_sizes.len() >= 2,
                "clearance={min_clearance} produced uniform island silhouettes"
            );
            let body_peaks: BTreeSet<_> = first
                .metadata
                .island_bodies
                .iter()
                .filter_map(|body| {
                    body.iter()
                        .filter_map(|coord| first.metadata.upper_surfaces.get(coord))
                        .max()
                        .copied()
                })
                .collect();
            assert!(
                body_peaks.len() >= 2,
                "clearance={min_clearance} produced uniform island elevations"
            );
            let actual_relief = first
                .metadata
                .upper_surfaces
                .values()
                .map(|surface| surface.saturating_sub(first.metadata.bridge_level))
                .max()
                .unwrap_or_default();
            assert_eq!(
                actual_relief,
                Level::try_from(expected_relief).unwrap_or(Level::MAX)
            );
            let actual_underbody = first
                .metadata
                .upper_bottoms
                .iter()
                .filter(|(coord, _bottom)| first.metadata.island_cells.contains(coord))
                .map(|(_coord, bottom)| first.metadata.bridge_level.saturating_sub(*bottom))
                .max()
                .unwrap_or_default();
            assert_eq!(actual_underbody, expected_underbody);

            for coord in &first.metadata.upper_cells {
                let ground_top = ground
                    .map
                    .column(*coord)
                    .expect("the finalized ground should contain every upper coordinate")
                    .top();
                let upper_bottom = first
                    .metadata
                    .upper_bottoms
                    .get(coord)
                    .copied()
                    .expect("every upper coordinate should publish its exact bottom");
                assert!(
                    upper_bottom.saturating_sub(ground_top) >= min_clearance,
                    "upper mass at {coord:?} violates clearance={min_clearance}"
                );
            }

            let bridge_cells: BTreeSet<_> = first
                .metadata
                .bridge_rows
                .iter()
                .flatten()
                .flat_map(|row| row.iter().copied())
                .filter(|coord| !first.metadata.island_cells.contains(coord))
                .collect();
            assert!(!bridge_cells.is_empty());
            assert!(bridge_cells.iter().all(|coord| {
                first.metadata.upper_surfaces.get(coord) == Some(&first.metadata.bridge_level)
                    && first.metadata.upper_bottoms.get(coord) == Some(&first.metadata.bridge_level)
            }));
            for (name, position) in ground.anchors.iter() {
                assert_eq!(first.anchors.get(name), Some(position));
            }
        }
    }

    #[test]
    fn sky_view_frames_the_full_vertical_volume_without_moving_the_legacy_pose() {
        let legacy_focus_height = f32::from(23_i16.saturating_add(35)) * 0.5 * 0.4;
        let legacy_frame = (12_f32 * 4.2).max(f32::from(35_i16.saturating_sub(23)) * 0.4 * 3.0);
        assert_eq!(
            sky_view_hint(12, 0.4, 23, 32, 35).expect("the legacy bounds should frame"),
            MapViewHint::new(
                (0.0, legacy_focus_height + legacy_frame, legacy_frame),
                (0.0, legacy_focus_height, 0.0)
            )
        );

        let underside_below_ground =
            sky_view_hint(12, 0.4, 60, 40, 70).expect("all finite bounds should frame");
        assert_eq!(underside_below_ground.focus, (0.0, 22.0, 0.0));
        assert!(
            underside_below_ground.eye.1 > underside_below_ground.focus.1,
            "the full ground-to-top volume should remain below the camera"
        );
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
        let corpus = [
            (
                V2EnvironmentSettings::TemperateGrassland,
                0,
                4,
                2_434_454_051_459_768_621,
            ),
            (
                V2EnvironmentSettings::TemperateGrassland,
                1,
                0,
                4_348_857_088_705_690_465,
            ),
            (
                V2EnvironmentSettings::TemperateGrassland,
                505,
                6,
                8_377_939_174_444_709_004,
            ),
            (
                V2EnvironmentSettings::TemperateGrassland,
                808,
                3,
                14_507_719_794_286_862_272,
            ),
            (
                V2EnvironmentSettings::TemperateGrassland,
                20_260_726,
                3,
                10_768_175_029_688_531_214,
            ),
            (
                V2EnvironmentSettings::TemperateGrassland,
                SKY_SEED,
                3,
                4_313_975_567_675_515_163,
            ),
            (
                V2EnvironmentSettings::TemperateGrassland,
                u64::MAX,
                3,
                17_675_506_124_942_439_081,
            ),
            (
                V2EnvironmentSettings::Frozen,
                0,
                4,
                4_504_858_493_484_372_362,
            ),
            (
                V2EnvironmentSettings::Frozen,
                1,
                1,
                16_880_656_360_868_672_559,
            ),
            (
                V2EnvironmentSettings::Frozen,
                505,
                2,
                9_009_212_082_947_791_225,
            ),
            (
                V2EnvironmentSettings::Frozen,
                808,
                3,
                14_474_164_956_362_771_915,
            ),
            (
                V2EnvironmentSettings::Frozen,
                20_260_726,
                3,
                1_242_084_161_769_491_627,
            ),
            (
                V2EnvironmentSettings::Frozen,
                SKY_SEED,
                3,
                1_391_565_746_594_453_391,
            ),
            (
                V2EnvironmentSettings::Frozen,
                u64::MAX,
                3,
                4_287_661_209_375_967_734,
            ),
        ];

        for (environment, seed, expected_candidate, expected_fingerprint) in corpus {
            let generated = build(12, 0.4, &settings(environment), seed, &palette(), &is_solid)
                .unwrap_or_else(|error| {
                    panic!("{environment:?} seed {seed} should generate: {error}")
                });

            assert_eq!(generated.selected_candidate, Some(expected_candidate));
            assert_eq!(generated.map_fingerprint, expected_fingerprint);
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

        let mut stale_bottom = valid.clone();
        let bottom = stale_bottom
            .metadata
            .upper_bottoms
            .values_mut()
            .next()
            .expect("the candidate should record an upper bottom");
        *bottom = bottom.saturating_sub(1);
        assert!(validation_issues(&recipe, layered_settings, &stale_bottom)
            .iter()
            .any(|issue| issue.contains("exact metadata")));

        let mut broken_partition = valid.clone();
        let removed = broken_partition
            .metadata
            .island_bodies
            .first_mut()
            .and_then(BTreeSet::pop_first)
            .expect("the candidate should record a non-empty island body");
        assert!(broken_partition.metadata.island_cells.contains(&removed));
        assert!(
            validation_issues(&recipe, layered_settings, &broken_partition)
                .iter()
                .any(|issue| issue.contains("partition"))
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
        let mut radius_40_median = 0_u128;
        for (label, settings) in [
            (
                "legacy",
                settings(V2EnvironmentSettings::TemperateGrassland),
            ),
            (
                "elevated",
                settings_with_geometry(V2EnvironmentSettings::TemperateGrassland, 22, 24),
            ),
        ] {
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
                eprintln!("Layered Sky {label} radius {radius}: median={median}us");
                if radius == 40 {
                    radius_40_median = radius_40_median.max(median);
                }
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
