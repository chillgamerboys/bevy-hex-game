//! Native V2 cave geometry.
//!
//! Caves retain a playable rocky surface while carving one exact underground
//! stratum. An open two-wide ramp joins both layers; roofed chamber and corridor
//! columns publish exact interior and cutaway metadata.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use bevy::platform::collections::HashMap;
use hex_core::{
    Headroom, HexCoord, InteriorRegionId, Level, MapViewHint, SubstanceId, TilePos,
    TraversalEndpoint, TraversalProfile, MAX_HEADROOM,
};

use super::recipe::{
    materialize_selection, run_recipe, CandidateAttemptError, CandidateContext, FallbackContext,
    MaterializedSelection, RecipePlan, RecipeValidation, RepairOutcome, ReportMetrics, V2Recipe,
    ValidationContext,
};
use super::volume::{
    InteriorVolume, LevelInterval, SolidMass, SolidMaterialRole, SurfaceAccess, SurfaceMetadata,
    TerrainVolumePlan, VolumeColumn, VolumeElement,
};
use super::V2GenerationError;
use crate::procedural::TacticalMetrics;
use crate::settings::{
    CavesSettings, ProceduralV2Settings, V2EnvironmentSettings, V2RecipeSettings,
};
use crate::terrain::TerrainPalette;

const INTERIOR: InteriorRegionId = InteriorRegionId(1);
const CORRIDOR_CLEARANCE: Level = 3;
const CHAMBER_CLEARANCE: Level = 4;
const PARTY_START: &str = "party_start";
const HOSTILE_START: &str = "hostile_start";
const CONFLICT_CENTER: &str = "conflict_center";
const CAVE_ENTRANCE: &str = "cave_entrance";
const DEEP_CHAMBER: &str = "deep_chamber";

#[derive(Debug, Clone)]
struct CaveRoute {
    rows: Vec<[TilePos; 2]>,
}

impl CaveRoute {
    fn coords(&self) -> BTreeSet<HexCoord> {
        self.rows
            .iter()
            .flat_map(|row| row.iter().map(|position| position.coord))
            .collect()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CavesMetadata {
    orientation: u8,
    surface_heights: BTreeMap<HexCoord, Level>,
    surface_materials: BTreeMap<HexCoord, SolidMaterialRole>,
    chamber_centres: Vec<HexCoord>,
    chamber_floor_levels: Vec<Level>,
    chamber_footprints: Vec<BTreeSet<HexCoord>>,
    tree_edges: Vec<(usize, usize)>,
    extra_edges: Vec<(usize, usize)>,
    corridor_routes: Vec<CaveRoute>,
    entrance_ramp: CaveRoute,
    covered_cells: BTreeSet<HexCoord>,
    chamber_cells: BTreeSet<HexCoord>,
    floor_levels: BTreeMap<HexCoord, Level>,
    clearances: BTreeMap<HexCoord, Level>,
    roof_bottoms: BTreeMap<HexCoord, Level>,
    deepest_chamber: HexCoord,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CavesMetrics {
    tactical: TacticalMetrics,
    chamber_count: u8,
    branch_nodes: u8,
    covered_floors: u32,
    minimum_roof_thickness: Level,
    entrance_steps: u32,
    cave_coverage_percent: u32,
    floor_relief: Level,
    clearance_relief: Level,
    surface_relief: Level,
    extra_links: u8,
}

impl ReportMetrics for CavesMetrics {
    fn tactical(&self) -> TacticalMetrics {
        self.tactical
    }
}

struct CavesRecipe {
    level_height: f32,
}

pub(crate) fn build(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV2Settings,
    seed: u64,
    palette: &TerrainPalette,
    is_solid: &dyn Fn(SubstanceId) -> bool,
) -> Result<MaterializedSelection<CavesMetadata, CavesMetrics>, V2GenerationError> {
    let V2RecipeSettings::Caves(cave_settings) = &settings.recipe else {
        return Err(V2GenerationError::RecipeUnavailable("Caves"));
    };
    if settings.environment != V2EnvironmentSettings::Rocky {
        return Err(V2GenerationError::RecipeUnavailable(
            "Caves with non-Rocky environment",
        ));
    }
    if !level_height.is_finite() || level_height <= 0.0 {
        return Err(V2GenerationError::RecipeContract(
            "Caves level height must be positive and finite".to_owned(),
        ));
    }

    let recipe = CavesRecipe { level_height };
    let selection = run_recipe(&recipe, cave_settings, grid_radius, seed)?;
    materialize_selection(selection, palette, is_solid)
}

impl V2Recipe for CavesRecipe {
    type Settings = CavesSettings;
    type Metadata = CavesMetadata;
    type Metrics = CavesMetrics;
    type Score = (u8, Level, u32, u32, u32, u32, u32, u32, u8, u8);

    fn construct(
        &self,
        context: CandidateContext,
        settings: &Self::Settings,
    ) -> Result<RecipePlan<Self::Metadata>, CandidateAttemptError> {
        construct_plan(self, context, settings, false)
    }

    fn validate(
        &self,
        _context: ValidationContext,
        settings: &Self::Settings,
        plan: &RecipePlan<Self::Metadata>,
    ) -> RecipeValidation<Self::Metrics> {
        validate_plan(settings, plan)
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
        let complexity = settings.chamber_count.saturating_sub(8);
        let target_branch_nodes = if complexity == 0 {
            2
        } else {
            3_u32.saturating_add(u32::from(complexity / 3))
        };
        let target_coverage = 18_u32.saturating_add(u32::from(complexity).saturating_mul(3));
        (
            metrics.chamber_count.abs_diff(settings.chamber_count),
            Level::try_from(metrics.minimum_roof_thickness.abs_diff(3)).unwrap_or(Level::MAX),
            u32::from(metrics.branch_nodes).abs_diff(target_branch_nodes),
            metrics.cave_coverage_percent.abs_diff(target_coverage),
            metrics.tactical.environment_signature_percent.abs_diff(25),
            metrics
                .floor_relief
                .abs_diff(extended_floor_relief(settings)),
            metrics
                .clearance_relief
                .abs_diff(extended_clearance_extra(settings).saturating_add(1)),
            metrics
                .surface_relief
                .abs_diff(extended_surface_relief(settings)),
            metrics
                .extra_links
                .abs_diff(extended_extra_link_count(settings)),
            candidate,
        )
    }

    fn canonical_fallback(
        &self,
        context: FallbackContext,
        settings: &Self::Settings,
    ) -> Result<RecipePlan<Self::Metadata>, V2GenerationError> {
        construct_plan(
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

fn construct_plan(
    recipe: &CavesRecipe,
    context: CandidateContext,
    settings: &CavesSettings,
    fallback: bool,
) -> Result<RecipePlan<CavesMetadata>, CandidateAttemptError> {
    let radius = i32::try_from(context.grid_radius)
        .map_err(|_out_of_range| CandidateAttemptError::rejected("cave radius is too large"))?;
    let orientation = if fallback {
        0
    } else {
        u8::try_from(context.streams.stage("caves.orientation").sample(0) % 6).unwrap_or_default()
    };
    let entrance_ramp = entrance_ramp(radius, orientation, settings)?;
    let (chamber_centres, tree_edges) =
        chamber_tree(radius, orientation, settings, context, fallback)?;
    let chamber_floor_levels =
        chamber_floor_levels(settings, &chamber_centres, &tree_edges, context, fallback)?;
    let extra_edges = extra_edges(settings, &chamber_centres, &tree_edges, context, fallback)?;

    let mut corridor_routes = Vec::new();
    let ramp_end = entrance_ramp
        .rows
        .last()
        .and_then(|row| row.first())
        .copied()
        .ok_or_else(|| CandidateAttemptError::rejected("cave entrance ramp is empty"))?;
    corridor_routes.push(connector_route(
        radius,
        orientation,
        ramp_end,
        settings.cave_floor_level,
    )?);
    for (parent, child) in tree_edges.iter().chain(&extra_edges) {
        let (Some(start), Some(end)) = (
            chamber_centres.get(*parent).copied(),
            chamber_centres.get(*child).copied(),
        ) else {
            return Err(CandidateAttemptError::rejected(
                "cave tree edge references a missing chamber",
            ));
        };
        let (Some(start_level), Some(end_level)) = (
            chamber_floor_levels.get(*parent).copied(),
            chamber_floor_levels.get(*child).copied(),
        ) else {
            return Err(CandidateAttemptError::rejected(
                "cave edge references a missing chamber floor",
            ));
        };
        corridor_routes.push(paired_route(
            context.grid_radius,
            start,
            end,
            start_level,
            end_level,
            settings.chamber_count > 8,
        )?);
    }

    let mut chamber_footprints: Vec<_> = chamber_centres
        .iter()
        .enumerate()
        .map(|(index, centre)| {
            let footprint_radius = u32::from(settings.chamber_count > 8 && index == 0);
            centre
                .within_radius(footprint_radius.saturating_add(1))
                .into_iter()
                .filter(|coord| HexCoord::ORIGIN.distance(*coord) <= context.grid_radius)
                .collect::<BTreeSet<_>>()
        })
        .collect();
    let mut network_cells: BTreeSet<_> = chamber_footprints
        .iter()
        .flat_map(|footprint| footprint.iter().copied())
        .collect();
    network_cells.extend(
        corridor_routes
            .iter()
            .flat_map(|route| route.coords().into_iter()),
    );
    let cave_distances = coordinate_distances(ramp_end.coord, &network_cells);
    let (deepest_index, deepest_chamber) = chamber_centres
        .iter()
        .copied()
        .enumerate()
        .filter(|(index, _centre)| *index > 0)
        .max_by_key(|(_index, centre)| {
            (
                cave_distances.get(centre).copied().unwrap_or_default(),
                *centre,
            )
        })
        .ok_or_else(|| CandidateAttemptError::rejected("cave has no deep chamber"))?;
    let deepest_footprint: BTreeSet<_> = deepest_chamber
        .within_radius(2)
        .into_iter()
        .filter(|coord| HexCoord::ORIGIN.distance(*coord) <= context.grid_radius)
        .collect();
    let Some(footprint) = chamber_footprints.get_mut(deepest_index) else {
        return Err(CandidateAttemptError::rejected(
            "deep cave chamber escaped its metadata",
        ));
    };
    *footprint = deepest_footprint;
    network_cells = chamber_footprints
        .iter()
        .flat_map(|footprint| footprint.iter().copied())
        .chain(
            corridor_routes
                .iter()
                .flat_map(|route| route.coords().into_iter()),
        )
        .collect();

    let floor_levels = reconcile_floor_levels(
        settings,
        &chamber_footprints,
        &chamber_floor_levels,
        &mut corridor_routes,
        &entrance_ramp,
    )?;
    let ramp_levels: BTreeMap<_, _> = entrance_ramp
        .rows
        .iter()
        .flatten()
        .map(|position| (position.coord, position.level))
        .collect();
    let ramp_cells: BTreeSet<_> = ramp_levels.keys().copied().collect();
    let covered_cells: BTreeSet<_> = network_cells.difference(&ramp_cells).copied().collect();
    let chamber_cells: BTreeSet<_> = chamber_footprints
        .iter()
        .flat_map(|footprint| footprint.iter().copied())
        .collect();
    let clearances = cave_clearances(
        settings,
        context,
        fallback,
        &corridor_routes,
        &chamber_footprints,
        deepest_index,
        &covered_cells,
    );

    let surface_heights = surface_heights(
        context,
        settings,
        orientation,
        &entrance_ramp,
        &chamber_centres,
        &floor_levels,
        &clearances,
    )?;
    let mut surface_materials = BTreeMap::new();
    let mut roof_bottoms = BTreeMap::new();
    let mut columns = BTreeMap::new();
    let mut surfaces = BTreeMap::new();
    let mut floors = BTreeSet::new();
    let mut entrances = BTreeSet::new();
    let mut clear_air = BTreeMap::new();

    for (coord, surface_level) in &surface_heights {
        let surface_material = rocky_surface_material(*coord, context, fallback);
        surface_materials.insert(*coord, surface_material);
        if let Some(ramp_level) = ramp_levels.get(coord).copied() {
            columns.insert(*coord, entrance_column(ramp_level));
            let position = TilePos::new(*coord, ramp_level);
            surfaces.insert(
                position,
                SurfaceMetadata {
                    access: SurfaceAccess::Ordinary,
                    interior: Some(INTERIOR),
                },
            );
            entrances.insert(position);
            clear_air.insert(
                *coord,
                LevelInterval::new(
                    ramp_level.saturating_add(1),
                    ramp_level
                        .saturating_add(5)
                        .max(surface_level.saturating_add(2)),
                ),
            );
        } else if covered_cells.contains(coord) {
            let floor_level = floor_levels.get(coord).copied().ok_or_else(|| {
                CandidateAttemptError::rejected("an excavated cave cell has no floor level")
            })?;
            let clearance = clearances.get(coord).copied().ok_or_else(|| {
                CandidateAttemptError::rejected("an excavated cave cell has no clearance")
            })?;
            let roof_bottom = floor_level.saturating_add(1).saturating_add(clearance);
            roof_bottoms.insert(*coord, roof_bottom);
            columns.insert(
                *coord,
                covered_column(floor_level, roof_bottom, *surface_level),
            );
            let floor = TilePos::new(*coord, floor_level);
            surfaces.insert(
                floor,
                SurfaceMetadata {
                    access: SurfaceAccess::Ordinary,
                    interior: Some(INTERIOR),
                },
            );
            surfaces.insert(
                TilePos::new(*coord, *surface_level),
                SurfaceMetadata {
                    access: SurfaceAccess::Ordinary,
                    interior: None,
                },
            );
            floors.insert(floor);
            clear_air.insert(
                *coord,
                LevelInterval::new(floor_level.saturating_add(1), roof_bottom),
            );
        } else {
            columns.insert(*coord, rocky_column(*surface_level, surface_material));
            surfaces.insert(
                TilePos::new(*coord, *surface_level),
                SurfaceMetadata {
                    access: SurfaceAccess::Ordinary,
                    interior: None,
                },
            );
        }
    }

    let party_position = entrance_ramp
        .rows
        .first()
        .and_then(|row| row.first())
        .copied()
        .ok_or_else(|| CandidateAttemptError::rejected("cave entrance has no landing"))?;
    let entry_approach = cave_entry_approach(&entrance_ramp, &corridor_routes);
    let hostile_position = safest_deep_chamber_anchor(
        chamber_footprints
            .get(deepest_index)
            .ok_or_else(|| CandidateAttemptError::rejected("deep cave footprint is missing"))?,
        &floor_levels,
        &entry_approach,
    )
    .ok_or_else(|| CandidateAttemptError::rejected("deep cave chamber has no floor"))?;
    let conflict_coord = chamber_centres.first().copied().unwrap_or(HexCoord::ORIGIN);
    let conflict_position = TilePos::new(
        conflict_coord,
        floor_levels
            .get(&conflict_coord)
            .copied()
            .unwrap_or(settings.cave_floor_level),
    );
    let anchors = BTreeMap::from([
        (PARTY_START.to_owned(), party_position),
        (HOSTILE_START.to_owned(), hostile_position),
        (CONFLICT_CENTER.to_owned(), conflict_position),
        (CAVE_ENTRANCE.to_owned(), party_position),
        (DEEP_CHAMBER.to_owned(), hostile_position),
    ]);
    let view_hint = cave_view_hint(
        context.grid_radius,
        recipe.level_height,
        settings.surface_level,
        orientation,
    )?;

    Ok(RecipePlan {
        volume: TerrainVolumePlan {
            grid_radius: context.grid_radius,
            columns,
            surfaces,
            anchors,
            interiors: BTreeMap::from([(
                INTERIOR,
                InteriorVolume {
                    floors,
                    entrances,
                    clear_air,
                },
            )]),
            view_hint,
        },
        metadata: CavesMetadata {
            orientation,
            surface_heights,
            surface_materials,
            chamber_centres,
            chamber_floor_levels,
            chamber_footprints,
            tree_edges,
            extra_edges,
            corridor_routes,
            entrance_ramp,
            covered_cells,
            chamber_cells,
            floor_levels,
            clearances,
            roof_bottoms,
            deepest_chamber,
        },
    })
}

fn entrance_ramp(
    radius: i32,
    orientation: u8,
    settings: &CavesSettings,
) -> Result<CaveRoute, CandidateAttemptError> {
    let descent = settings
        .surface_level
        .checked_sub(settings.cave_floor_level)
        .ok_or_else(|| CandidateAttemptError::rejected("cave entrance descent underflowed"))?;
    let rows = (0..=descent)
        .map(|step| {
            let local_y = (-radius).saturating_add(step);
            let level = settings.surface_level.saturating_sub(step);
            [
                TilePos::new(from_local(0, local_y, orientation), level),
                TilePos::new(from_local(1, local_y, orientation), level),
            ]
        })
        .collect();
    Ok(CaveRoute { rows })
}

fn connector_route(
    radius: i32,
    orientation: u8,
    ramp_end: TilePos,
    floor_level: Level,
) -> Result<CaveRoute, CandidateAttemptError> {
    let local_end = to_local(ramp_end.coord, orientation);
    if local_end.x() != 0 || local_end.y() > 0 {
        return Err(CandidateAttemptError::rejected(
            "cave entrance endpoint cannot connect to the root",
        ));
    }
    let rows: Vec<_> = (local_end.y()..=0)
        .map(|local_y| {
            [
                TilePos::new(from_local(0, local_y, orientation), floor_level),
                TilePos::new(from_local(1, local_y, orientation), floor_level),
            ]
        })
        .collect();
    if rows.iter().flatten().any(|position| {
        HexCoord::ORIGIN.distance(position.coord) > u32::try_from(radius).unwrap_or(u32::MAX)
    }) {
        return Err(CandidateAttemptError::rejected(
            "cave entrance connector escaped the map",
        ));
    }
    Ok(CaveRoute { rows })
}

fn chamber_tree(
    radius: i32,
    orientation: u8,
    settings: &CavesSettings,
    context: CandidateContext,
    fallback: bool,
) -> Result<(Vec<HexCoord>, Vec<(usize, usize)>), CandidateAttemptError> {
    if settings.chamber_count > 8 {
        return extended_chamber_tree(radius, orientation, settings);
    }

    let outer = (radius / 2).max(8).min(radius.saturating_sub(2));
    let inner = (outer / 2).max(3);
    let root = from_local(0, 0, orientation);
    let left_junction = from_local(-inner, inner, orientation);
    let right_junction = from_local(inner, 0, orientation);
    let leaf_pool = [
        (from_local(-outer, outer, orientation), 1_usize),
        (from_local(-outer / 2, outer, orientation), 1_usize),
        (
            from_local(0, outer, orientation),
            if fallback
                || context
                    .streams
                    .stage("caves.layout")
                    .sample(0)
                    .is_multiple_of(2)
            {
                1
            } else {
                2
            },
        ),
        (from_local(outer / 2, outer / 2, orientation), 2_usize),
        (from_local(outer, 0, orientation), 2_usize),
    ];
    let selected: &[usize] = match settings.chamber_count {
        6 => &[0, 2, 4],
        7 => &[0, 1, 3, 4],
        8 => &[0, 1, 2, 3, 4],
        _ => {
            return Err(CandidateAttemptError::rejected(
                "unsupported cave chamber count",
            ));
        }
    };

    let mut centres = vec![root, left_junction, right_junction];
    let mut edges = vec![(0, 1), (0, 2)];
    for selected_index in selected {
        let Some((centre, parent)) = leaf_pool.get(*selected_index).copied() else {
            return Err(CandidateAttemptError::rejected(
                "cave leaf selection escaped its pool",
            ));
        };
        let child = centres.len();
        centres.push(centre);
        edges.push((parent, child));
    }
    if centres
        .iter()
        .any(|centre| HexCoord::ORIGIN.distance(*centre) > u32::try_from(radius).unwrap_or(0))
    {
        return Err(CandidateAttemptError::rejected(
            "cave chamber centre escaped the map",
        ));
    }
    Ok((centres, edges))
}

fn extended_chamber_tree(
    radius: i32,
    orientation: u8,
    settings: &CavesSettings,
) -> Result<(Vec<HexCoord>, Vec<(usize, usize)>), CandidateAttemptError> {
    if !(9..=12).contains(&settings.chamber_count) {
        return Err(CandidateAttemptError::rejected(
            "unsupported expanded cave chamber count",
        ));
    }

    // The authored normalized slots keep the minimum-radius fallback reliable while
    // scaling the network across larger maps. Rotation still comes from the candidate
    // stream, so the template has six deterministic presentations.
    let slots = [
        (0, 0),
        (-4, 4),
        (4, 0),
        (0, 7),
        (-9, 9),
        (-5, 9),
        (0, 10),
        (5, 5),
        (9, 0),
        (-9, 5),
        (9, -5),
        (5, -9),
    ];
    let edges = [
        (0, 1),
        (0, 2),
        (0, 3),
        (1, 4),
        (1, 5),
        (3, 6),
        (3, 7),
        (2, 8),
        (1, 9),
        (2, 10),
        (2, 11),
    ];
    let count = usize::from(settings.chamber_count);
    let centres: Vec<_> = slots
        .into_iter()
        .take(count)
        .map(|(x, y)| {
            from_local(
                scale_extended_coord(x, radius),
                scale_extended_coord(y, radius),
                orientation,
            )
        })
        .collect();
    let tree_edges: Vec<_> = edges
        .into_iter()
        .filter(|(_parent, child)| *child < count)
        .collect();
    if centres.len() != count
        || tree_edges.len() != count.saturating_sub(1)
        || centres
            .iter()
            .any(|centre| HexCoord::ORIGIN.distance(*centre) > radius.unsigned_abs())
    {
        return Err(CandidateAttemptError::rejected(
            "expanded cave template escaped its map or tree",
        ));
    }
    Ok((centres, tree_edges))
}

fn scale_extended_coord(value: i32, radius: i32) -> i32 {
    value.saturating_mul(radius) / 12
}

fn extended_floor_relief(settings: &CavesSettings) -> Level {
    i32::from(settings.chamber_count.saturating_sub(8).min(2))
}

fn extended_clearance_extra(settings: &CavesSettings) -> Level {
    i32::from(settings.chamber_count.saturating_sub(8).min(3))
}

fn extended_surface_relief(settings: &CavesSettings) -> Level {
    if settings.chamber_count <= 8 {
        2
    } else {
        i32::from(settings.chamber_count.saturating_sub(6).min(5))
    }
}

const fn extended_extra_link_count(settings: &CavesSettings) -> u8 {
    settings.chamber_count.saturating_sub(8) / 2
}

fn chamber_floor_levels(
    settings: &CavesSettings,
    centres: &[HexCoord],
    _tree_edges: &[(usize, usize)],
    context: CandidateContext,
    fallback: bool,
) -> Result<Vec<Level>, CandidateAttemptError> {
    let relief = extended_floor_relief(settings);
    let mut levels = vec![settings.cave_floor_level; centres.len()];
    if relief == 0 {
        return Ok(levels);
    }

    let stream = context.streams.stage("caves.extended.floors");
    for (index, level) in levels.iter_mut().enumerate().skip(1) {
        let offset = if index <= 3 {
            1.min(relief)
        } else if fallback {
            Level::try_from(index).unwrap_or(Level::MAX) % relief.saturating_add(1)
        } else {
            Level::try_from(
                stream.sample(u64::try_from(index).unwrap_or(u64::MAX))
                    % u64::try_from(relief.saturating_add(1))
                        .unwrap_or(u64::MAX)
                        .max(1),
            )
            .unwrap_or_default()
        };
        *level = settings.cave_floor_level.saturating_add(offset);
    }
    let Some(last) = levels.last_mut() else {
        return Err(CandidateAttemptError::rejected(
            "expanded cave has no chamber floors",
        ));
    };
    *last = settings.cave_floor_level.saturating_add(relief);
    Ok(levels)
}

fn extra_edges(
    settings: &CavesSettings,
    centres: &[HexCoord],
    tree_edges: &[(usize, usize)],
    context: CandidateContext,
    fallback: bool,
) -> Result<Vec<(usize, usize)>, CandidateAttemptError> {
    let requested = usize::from(extended_extra_link_count(settings));
    if requested == 0 {
        return Ok(Vec::new());
    }

    let count = centres.len();
    let mut eligible: Vec<_> = [
        (4_usize, 5_usize),
        (6, 7),
        (7, 8),
        (4, 9),
        (8, 10),
        (10, 11),
    ]
    .into_iter()
    .filter(|(first, second)| {
        *first < count
            && *second < count
            && !tree_edges.iter().any(|edge| {
                let normalized = (edge.0.min(edge.1), edge.0.max(edge.1));
                normalized == (*first.min(second), *first.max(second))
            })
    })
    .collect();
    if eligible.len() < requested {
        return Err(CandidateAttemptError::rejected(
            "expanded cave has too few eligible extra links",
        ));
    }
    let offset = if fallback {
        0
    } else {
        usize::try_from(
            context.streams.stage("caves.extended.links").sample(0)
                % u64::try_from(eligible.len()).unwrap_or(u64::MAX).max(1),
        )
        .unwrap_or_default()
    };
    eligible.rotate_left(offset);
    eligible.truncate(requested);
    eligible.sort_unstable();
    Ok(eligible)
}

fn reconcile_floor_levels(
    settings: &CavesSettings,
    footprints: &[BTreeSet<HexCoord>],
    chamber_levels: &[Level],
    routes: &mut [CaveRoute],
    entrance: &CaveRoute,
) -> Result<BTreeMap<HexCoord, Level>, CandidateAttemptError> {
    let mut floors = BTreeMap::new();
    for (footprint, level) in footprints.iter().zip(chamber_levels) {
        for coord in footprint {
            if floors.insert(*coord, *level).is_some() {
                return Err(CandidateAttemptError::rejected(
                    "expanded cave chamber footprints overlap",
                ));
            }
        }
    }

    for route in routes.iter_mut() {
        for row in &mut route.rows {
            let existing: BTreeSet<_> = row
                .iter()
                .filter_map(|position| floors.get(&position.coord).copied())
                .collect();
            if existing.len() > 1 {
                return Err(CandidateAttemptError::rejected(
                    "a two-wide cave row intersects incompatible floor terraces",
                ));
            }
            let level = existing.first().copied().unwrap_or_else(|| {
                row.first()
                    .map_or(settings.cave_floor_level, |pos| pos.level)
            });
            for position in row {
                position.level = level;
                floors.entry(position.coord).or_insert(level);
            }
        }
    }
    for (route_index, route) in routes.iter_mut().enumerate() {
        for position in route.rows.iter_mut().flatten() {
            if let Some(level) = floors.get(&position.coord).copied() {
                position.level = level;
            }
        }
        if !valid_sloped_route(route) {
            return Err(CandidateAttemptError::rejected(format!(
                "cave floor reconciliation made route {route_index} unwalkable"
            )));
        }
    }

    for coord in entrance.coords() {
        floors.remove(&coord);
    }
    if settings.chamber_count <= 8
        && floors
            .values()
            .any(|level| *level != settings.cave_floor_level)
    {
        return Err(CandidateAttemptError::rejected(
            "legacy cave floor levels changed",
        ));
    }
    Ok(floors)
}

fn cave_clearances(
    settings: &CavesSettings,
    context: CandidateContext,
    fallback: bool,
    routes: &[CaveRoute],
    footprints: &[BTreeSet<HexCoord>],
    deepest_index: usize,
    covered: &BTreeSet<HexCoord>,
) -> BTreeMap<HexCoord, Level> {
    let mut clearances: BTreeMap<_, _> = covered
        .iter()
        .map(|coord| (*coord, CORRIDOR_CLEARANCE))
        .collect();
    if settings.chamber_count > 8 {
        let phase = if fallback {
            0
        } else {
            context
                .streams
                .stage("caves.extended.corridor_heights")
                .sample(0)
                % 2
        };
        for (index, route) in routes.iter().enumerate().skip(1) {
            let clearance = CORRIDOR_CLEARANCE.saturating_add(
                if (u64::try_from(index).unwrap_or(u64::MAX) + phase) % 2 != 0 {
                    1
                } else {
                    0
                },
            );
            for coord in route.coords() {
                if let Some(existing) = clearances.get_mut(&coord) {
                    *existing = (*existing).max(clearance);
                }
            }
        }
    }

    let extra = extended_clearance_extra(settings);
    let stream = context.streams.stage("caves.extended.chamber_heights");
    for (index, footprint) in footprints.iter().enumerate() {
        let clearance = if extra == 0 || index == 0 {
            CHAMBER_CLEARANCE
        } else if index == deepest_index {
            CHAMBER_CLEARANCE.saturating_add(extra)
        } else {
            let sampled = if fallback {
                Level::try_from(index).unwrap_or(Level::MAX) % extra.saturating_add(1)
            } else {
                Level::try_from(
                    stream.sample(u64::try_from(index).unwrap_or(u64::MAX))
                        % u64::try_from(extra.saturating_add(1))
                            .unwrap_or(u64::MAX)
                            .max(1),
                )
                .unwrap_or_default()
            };
            CHAMBER_CLEARANCE.saturating_add(sampled)
        };
        for coord in footprint {
            if let Some(existing) = clearances.get_mut(coord) {
                *existing = clearance;
            }
        }
    }
    clearances
}

fn paired_route(
    grid_radius: u32,
    start: HexCoord,
    end: HexCoord,
    start_level: Level,
    end_level: Level,
    require_advancing_lane: bool,
) -> Result<CaveRoute, CandidateAttemptError> {
    let centerline = start.line_between(end);
    let centerline_cells: BTreeSet<_> = centerline.iter().copied().collect();
    let mut layers = Vec::<BTreeMap<HexCoord, Option<HexCoord>>>::new();
    for centre in &centerline {
        let previous = layers.last();
        let mut layer = BTreeMap::new();
        for candidate in centre.neighbors() {
            if HexCoord::ORIGIN.distance(candidate) > grid_radius
                || centerline_cells.contains(&candidate)
            {
                continue;
            }
            let predecessor = match previous {
                None => Some(None),
                Some(previous) => previous
                    .keys()
                    .find(|before| {
                        before.distance(candidate) == 1
                            || (!require_advancing_lane && **before == candidate)
                    })
                    .copied()
                    .map(Some),
            };
            if let Some(predecessor) = predecessor {
                layer.insert(candidate, predecessor);
            }
        }
        if layer.is_empty() {
            return Err(CandidateAttemptError::rejected(
                "a cave corridor cannot maintain two lanes",
            ));
        }
        layers.push(layer);
    }

    let Some(mut current) = layers.last().and_then(|layer| layer.keys().next()).copied() else {
        return Err(CandidateAttemptError::rejected(
            "a cave corridor has no second lane",
        ));
    };
    let mut second_reversed = Vec::with_capacity(layers.len());
    for layer in layers.iter().rev() {
        second_reversed.push(current);
        let Some(previous) = layer.get(&current).copied().flatten() else {
            break;
        };
        current = previous;
    }
    if second_reversed.len() != layers.len() {
        return Err(CandidateAttemptError::rejected(
            "a cave corridor has incomplete lane metadata",
        ));
    }
    second_reversed.reverse();
    let transitions = centerline.len().saturating_sub(1);
    let rows = centerline
        .into_iter()
        .zip(second_reversed)
        .enumerate()
        .map(|(index, (first, second))| {
            let level = interpolated_level(start_level, end_level, index, transitions);
            [TilePos::new(first, level), TilePos::new(second, level)]
        })
        .collect();
    Ok(CaveRoute { rows })
}

fn interpolated_level(start: Level, end: Level, index: usize, transitions: usize) -> Level {
    if transitions == 0 || start == end {
        return start;
    }
    let span = start.abs_diff(end);
    let progressed = span.saturating_mul(u32::try_from(index).unwrap_or(u32::MAX))
        / u32::try_from(transitions).unwrap_or(u32::MAX).max(1);
    let progressed = Level::try_from(progressed).unwrap_or(Level::MAX);
    if end > start {
        start.saturating_add(progressed)
    } else {
        start.saturating_sub(progressed)
    }
}

fn coordinate_distances(start: HexCoord, allowed: &BTreeSet<HexCoord>) -> BTreeMap<HexCoord, u32> {
    let mut distances = BTreeMap::from([(start, 0_u32)]);
    let mut frontier = VecDeque::from([start]);
    while let Some(coord) = frontier.pop_front() {
        let steps = distances.get(&coord).copied().unwrap_or(u32::MAX);
        for neighbor in coord.neighbors() {
            if allowed.contains(&neighbor) && !distances.contains_key(&neighbor) {
                distances.insert(neighbor, steps.saturating_add(1));
                frontier.push_back(neighbor);
            }
        }
    }
    distances
}

fn cave_entry_approach(entrance: &CaveRoute, corridor_routes: &[CaveRoute]) -> Vec<TilePos> {
    entrance
        .rows
        .iter()
        .chain(
            corridor_routes
                .first()
                .into_iter()
                .flat_map(|route| route.rows.iter()),
        )
        .flatten()
        .copied()
        .collect()
}

fn safest_deep_chamber_anchor(
    footprint: &BTreeSet<HexCoord>,
    floor_levels: &BTreeMap<HexCoord, Level>,
    entry_approach: &[TilePos],
) -> Option<TilePos> {
    // Combat does not have wall-aware line of sight yet. Maximise geometric
    // separation from the complete entrance route without coupling deterministic map
    // construction to mutable combat settings; the shipped scenario regression
    // checks the selected floor against the actual loaded policy.
    footprint
        .iter()
        .filter_map(|coord| {
            floor_levels
                .get(coord)
                .copied()
                .map(|level| TilePos::new(*coord, level))
        })
        .max_by_key(|position| {
            (
                entry_approach
                    .iter()
                    .map(|approach| approach.coord.distance(position.coord))
                    .min()
                    .unwrap_or(u32::MAX),
                position.coord,
            )
        })
}

fn surface_heights(
    context: CandidateContext,
    settings: &CavesSettings,
    orientation: u8,
    entrance: &CaveRoute,
    chamber_centres: &[HexCoord],
    floor_levels: &BTreeMap<HexCoord, Level>,
    clearances: &BTreeMap<HexCoord, Level>,
) -> Result<BTreeMap<HexCoord, Level>, CandidateAttemptError> {
    let radius = i32::try_from(context.grid_radius)
        .map_err(|_out_of_range| CandidateAttemptError::rejected("cave radius is too large"))?;
    let landing: Vec<_> = entrance
        .rows
        .first()
        .into_iter()
        .flatten()
        .map(|position| position.coord)
        .collect();
    if settings.chamber_count <= 8 {
        let rise_cap = 17_i32.saturating_sub(settings.surface_level).min(2);
        let mound_centres = [
            from_local(-radius / 3, 0, orientation),
            from_local(radius / 3, -radius / 3, orientation),
        ];
        let mut heights = BTreeMap::new();
        for coord in HexCoord::ORIGIN.within_radius(context.grid_radius) {
            let rise = mound_centres
                .iter()
                .map(|centre| {
                    rise_cap
                        .saturating_sub_unsigned(centre.distance(coord) / 3)
                        .max(0)
                })
                .max()
                .unwrap_or_default();
            let mut height = settings.surface_level.saturating_add(rise);
            let landing_distance = landing
                .iter()
                .map(|landing| landing.distance(coord))
                .min()
                .unwrap_or_default();
            height = height.min(
                settings
                    .surface_level
                    .saturating_add_unsigned(landing_distance.min(2)),
            );
            heights.insert(coord, height);
        }
        return Ok(heights);
    }

    let relief = extended_surface_relief(settings);
    let stream = context.streams.stage("caves.extended.surface");
    let feature_count = usize::from(settings.chamber_count.saturating_sub(5).min(6));
    let mut feature_centres: Vec<_> = chamber_centres.iter().copied().enumerate().collect();
    feature_centres.sort_by_key(|(index, centre)| {
        (
            stream.sample(u64::try_from(*index).unwrap_or(u64::MAX)),
            *centre,
        )
    });
    feature_centres.truncate(feature_count);
    if let Some(farthest) = chamber_centres.iter().copied().max_by_key(|centre| {
        landing
            .iter()
            .map(|landing| landing.distance(*centre))
            .min()
            .unwrap_or_default()
    }) {
        if let Some(feature) = feature_centres
            .iter_mut()
            .find(|(_index, centre)| *centre == farthest)
        {
            feature.0 = usize::MAX;
        } else {
            if let Some(last) = feature_centres.last_mut() {
                *last = (usize::MAX, farthest);
            }
        }
        feature_centres.sort_by_key(|(index, centre)| (*index, *centre));
    }

    let mut heights = BTreeMap::new();
    for coord in HexCoord::ORIGIN.within_radius(context.grid_radius) {
        let rise = feature_centres
            .iter()
            .enumerate()
            .map(|(feature, (index, centre))| {
                let peak = if *index == usize::MAX {
                    relief
                } else {
                    let variable = relief.saturating_sub(1).max(1);
                    2_i32
                        .saturating_add(
                            Level::try_from(
                                stream.sample(
                                    u64::try_from(*index)
                                        .unwrap_or(u64::MAX)
                                        .saturating_add(100),
                                ) % u64::try_from(variable).unwrap_or(1).max(1),
                            )
                            .unwrap_or_default(),
                        )
                        .min(relief)
                };
                let width = 2_u32.saturating_add(
                    u32::try_from(
                        stream.sample(u64::try_from(feature).unwrap_or(u64::MAX) + 200) % 2,
                    )
                    .unwrap_or_default(),
                );
                peak.saturating_sub_unsigned(centre.distance(coord) / width)
                    .max(0)
            })
            .max()
            .unwrap_or_default();
        let mut height = settings.surface_level.saturating_add(rise);
        let landing_distance = landing
            .iter()
            .map(|landing| landing.distance(coord))
            .min()
            .unwrap_or_default();
        height = height.min(
            settings
                .surface_level
                .saturating_add_unsigned(landing_distance),
        );
        heights.insert(coord, height);
    }

    let max_surface = settings.surface_level.saturating_add(relief);
    let landing_distances: BTreeMap<_, _> = heights
        .keys()
        .copied()
        .map(|coord| {
            let distance = landing
                .iter()
                .map(|landing| landing.distance(coord))
                .min()
                .unwrap_or_default();
            (coord, distance)
        })
        .collect();
    let mut frontier = VecDeque::new();
    for (coord, floor) in floor_levels {
        let Some(clearance) = clearances.get(coord).copied() else {
            return Err(CandidateAttemptError::rejected(
                "a cave floor has no surface-clearance contract",
            ));
        };
        let required = floor.saturating_add(clearance).saturating_add(3);
        let landing_cap = landing_distances
            .get(coord)
            .copied()
            .map_or(max_surface, |distance| {
                settings.surface_level.saturating_add_unsigned(distance)
            });
        if required > max_surface || required > landing_cap {
            return Err(CandidateAttemptError::rejected(
                "expanded cave roof cannot fit below its surface relief",
            ));
        }
        let Some(height) = heights.get_mut(coord) else {
            return Err(CandidateAttemptError::rejected(
                "expanded cave floor escaped the surface",
            ));
        };
        if *height < required {
            *height = required;
            frontier.push_back(*coord);
        }
    }

    while let Some(coord) = frontier.pop_front() {
        let Some(height) = heights.get(&coord).copied() else {
            continue;
        };
        let needed = height.saturating_sub(1);
        for neighbor in coord.neighbors() {
            let Some(neighbor_height) = heights.get_mut(&neighbor) else {
                continue;
            };
            if *neighbor_height >= needed {
                continue;
            }
            let landing_cap = landing_distances
                .get(&neighbor)
                .copied()
                .map_or(max_surface, |distance| {
                    settings.surface_level.saturating_add_unsigned(distance)
                });
            if needed > max_surface || needed > landing_cap {
                return Err(CandidateAttemptError::rejected(
                    "expanded cave roof projection blocks the entrance landing",
                ));
            }
            *neighbor_height = needed;
            frontier.push_back(neighbor);
        }
    }
    Ok(heights)
}

fn rocky_surface_material(
    coord: HexCoord,
    context: CandidateContext,
    fallback: bool,
) -> SolidMaterialRole {
    let gravel = if fallback {
        coord
            .x()
            .saturating_add(coord.y().saturating_mul(2))
            .rem_euclid(7)
            == 0
    } else {
        context
            .streams
            .stage("caves.materials")
            .sample_coord(
                HexCoord::from_axial(coord.x().div_euclid(3), coord.y().div_euclid(3)),
                0,
            )
            .is_multiple_of(5)
    };
    if gravel {
        SolidMaterialRole::Gravel
    } else {
        SolidMaterialRole::Stone
    }
}

fn rocky_column(surface: Level, material: SolidMaterialRole) -> VolumeColumn {
    let mut elements = vec![VolumeElement::Solid(SolidMass {
        levels: LevelInterval::new(0, 1),
        material: SolidMaterialRole::Bedrock,
        cutaway_for: None,
    })];
    if material == SolidMaterialRole::Stone {
        elements.push(VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(1, surface.saturating_add(1)),
            material,
            cutaway_for: None,
        }));
    } else {
        elements.push(VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(1, surface),
            material: SolidMaterialRole::Stone,
            cutaway_for: None,
        }));
        elements.push(VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(surface, surface.saturating_add(1)),
            material,
            cutaway_for: None,
        }));
    }
    VolumeColumn { elements }
}

fn entrance_column(surface: Level) -> VolumeColumn {
    VolumeColumn {
        elements: vec![
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(0, 1),
                material: SolidMaterialRole::Bedrock,
                cutaway_for: None,
            }),
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(1, surface),
                material: SolidMaterialRole::Stone,
                cutaway_for: None,
            }),
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(surface, surface.saturating_add(1)),
                material: SolidMaterialRole::Gravel,
                cutaway_for: None,
            }),
        ],
    }
}

fn covered_column(floor: Level, roof_bottom: Level, surface: Level) -> VolumeColumn {
    VolumeColumn {
        elements: vec![
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(0, 1),
                material: SolidMaterialRole::Bedrock,
                cutaway_for: None,
            }),
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(1, floor),
                material: SolidMaterialRole::Stone,
                cutaway_for: None,
            }),
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(floor, floor.saturating_add(1)),
                material: SolidMaterialRole::Gravel,
                cutaway_for: None,
            }),
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(roof_bottom, surface.saturating_add(1)),
                material: SolidMaterialRole::Stone,
                cutaway_for: Some(INTERIOR),
            }),
        ],
    }
}

fn validate_plan(
    settings: &CavesSettings,
    plan: &RecipePlan<CavesMetadata>,
) -> RecipeValidation<CavesMetrics> {
    let metadata = &plan.metadata;
    let mut issues = Vec::new();
    let min_surface = metadata
        .surface_heights
        .values()
        .copied()
        .min()
        .unwrap_or(settings.surface_level);
    let max_surface = metadata
        .surface_heights
        .values()
        .copied()
        .max()
        .unwrap_or(settings.surface_level);
    let expected_surface_relief = extended_surface_relief(settings);
    let invalid_surface = if settings.chamber_count <= 8 {
        min_surface < 14
            || max_surface > 17
            || min_surface < settings.surface_level
            || max_surface.saturating_sub(min_surface) > 2
    } else {
        min_surface != settings.surface_level
            || max_surface
                > settings
                    .surface_level
                    .saturating_add(expected_surface_relief)
            || max_surface.saturating_sub(min_surface) != expected_surface_relief
    };
    if invalid_surface {
        issues.push(format!(
            "rocky cave surface is {min_surface}..={max_surface}; expected relief {expected_surface_relief} from level {}",
            settings.surface_level
        ));
    }
    if metadata.surface_heights.len() != plan.volume.columns.len()
        || metadata.surface_materials.len() != metadata.surface_heights.len()
    {
        issues.push("cave surface metadata does not cover the complete footprint".to_owned());
    }
    let surface_height_lookup: HashMap<_, _> = metadata
        .surface_heights
        .iter()
        .map(|(coord, level)| (*coord, *level))
        .collect();
    if metadata.surface_heights.iter().any(|(coord, level)| {
        coord.neighbors().into_iter().any(|neighbor| {
            surface_height_lookup
                .get(&neighbor)
                .is_some_and(|other| level.abs_diff(*other) > 1)
        })
    }) {
        issues.push("rocky cave surface contains a non-walkable height transition".to_owned());
    }

    let unique_centres: BTreeSet<_> = metadata.chamber_centres.iter().copied().collect();
    if metadata.chamber_centres.len() != usize::from(settings.chamber_count)
        || unique_centres.len() != metadata.chamber_centres.len()
    {
        issues.push("cave chamber count or centre uniqueness is invalid".to_owned());
    }
    if metadata.chamber_footprints.len() != metadata.chamber_centres.len()
        || metadata
            .chamber_centres
            .iter()
            .zip(&metadata.chamber_footprints)
            .any(|(centre, footprint)| !footprint.contains(centre))
    {
        issues.push("a cave chamber footprint does not contain its centre".to_owned());
    }
    for (index, footprint) in metadata.chamber_footprints.iter().enumerate() {
        if metadata
            .chamber_footprints
            .iter()
            .skip(index.saturating_add(1))
            .any(|other| !footprint.is_disjoint(other))
        {
            issues.push("cave chamber footprints overlap".to_owned());
            break;
        }
    }
    let exact_chamber_cells: BTreeSet<_> = metadata
        .chamber_footprints
        .iter()
        .flat_map(|footprint| footprint.iter().copied())
        .collect();
    if metadata.chamber_cells != exact_chamber_cells {
        issues.push("cave chamber cell metadata is stale".to_owned());
    }
    let branch_nodes = validate_tree(
        metadata.chamber_centres.len(),
        &metadata.tree_edges,
        &mut issues,
    );
    validate_extra_edges(
        metadata.chamber_centres.len(),
        &metadata.tree_edges,
        &metadata.extra_edges,
        extended_extra_link_count(settings),
        &mut issues,
    );
    if metadata.chamber_floor_levels.len() != metadata.chamber_centres.len()
        || metadata
            .chamber_centres
            .iter()
            .zip(&metadata.chamber_floor_levels)
            .any(|(centre, level)| metadata.floor_levels.get(centre).copied() != Some(*level))
    {
        issues.push("cave chamber floor metadata is incomplete or stale".to_owned());
    }
    let min_floor = metadata
        .floor_levels
        .values()
        .copied()
        .min()
        .unwrap_or(settings.cave_floor_level);
    let max_floor = metadata
        .floor_levels
        .values()
        .copied()
        .max()
        .unwrap_or(settings.cave_floor_level);
    if min_floor != settings.cave_floor_level
        || max_floor.saturating_sub(min_floor) != extended_floor_relief(settings)
    {
        issues.push(format!(
            "cave floor range {min_floor}..={max_floor} does not match its derived relief"
        ));
    }

    if !valid_ramp(
        &metadata.entrance_ramp,
        settings.surface_level,
        settings.cave_floor_level,
    ) {
        issues.push("cave entrance is not a contiguous two-wide one-level ramp".to_owned());
    }
    let expected_edge = i32::try_from(plan.volume.grid_radius)
        .ok()
        .map(|radius| HexCoord::from_axial(0, -radius));
    let actual_edge = metadata
        .entrance_ramp
        .rows
        .first()
        .and_then(|row| row.first())
        .map(|position| to_local(position.coord, metadata.orientation));
    if actual_edge != expected_edge {
        issues.push("cave entrance does not begin at its oriented map edge".to_owned());
    }
    let expected_routes = 1_usize
        .saturating_add(metadata.tree_edges.len())
        .saturating_add(metadata.extra_edges.len());
    if metadata.corridor_routes.len() != expected_routes
        || metadata.corridor_routes.iter().any(|route| {
            if settings.chamber_count <= 8 {
                !valid_flat_route(route, settings.cave_floor_level)
            } else {
                !valid_sloped_route(route)
            }
        })
    {
        issues.push(
            "a cave corridor is not a contiguous two-wide flat route or valid sloped route"
                .to_owned(),
        );
    }
    if !routes_match_declared_edges(metadata) {
        issues.push("cave corridor routes do not match their declared graph edges".to_owned());
    }
    let exact_entrance_floors: BTreeMap<_, _> = metadata
        .entrance_ramp
        .rows
        .iter()
        .flatten()
        .map(|position| (position.coord, position.level))
        .collect();
    if metadata.corridor_routes.iter().any(|route| {
        !route_uses_exact_authored_floors(
            route,
            &metadata.floor_levels,
            &exact_entrance_floors,
            &plan.volume,
        )
    }) {
        issues.push("a cave corridor route does not use exact authored ordinary floors".to_owned());
    }
    if metadata
        .corridor_routes
        .iter()
        .any(|route| !route_admits_walker(route, &plan.volume))
    {
        issues.push("a cave corridor lacks walker clearance across a transition".to_owned());
    }
    let ramp_cells = metadata.entrance_ramp.coords();
    if metadata.corridor_routes.iter().any(|route| {
        route
            .coords()
            .into_iter()
            .any(|coord| !metadata.covered_cells.contains(&coord) && !ramp_cells.contains(&coord))
    }) {
        issues.push("a cave corridor is missing from the excavated network".to_owned());
    }

    let expected_floors: BTreeSet<_> = metadata
        .floor_levels
        .iter()
        .map(|(coord, level)| TilePos::new(*coord, *level))
        .collect();
    if metadata.floor_levels.len() != metadata.covered_cells.len()
        || metadata
            .floor_levels
            .keys()
            .any(|coord| !metadata.covered_cells.contains(coord))
    {
        issues.push("cave floor metadata does not exactly match covered cells".to_owned());
    }
    let expected_entrances: BTreeSet<_> = metadata
        .entrance_ramp
        .rows
        .iter()
        .flatten()
        .copied()
        .collect();
    let Some(interior) = plan.volume.interiors.get(&INTERIOR) else {
        issues.push("cave volume has no exact interior metadata".to_owned());
        return RecipeValidation::invalid(issues);
    };
    if plan.volume.interiors.len() != 1
        || interior.floors != expected_floors
        || interior.entrances != expected_entrances
    {
        issues.push("cave interior floor or entrance membership is not exact".to_owned());
    }
    let expected_clear_coords: BTreeSet<_> =
        metadata.covered_cells.union(&ramp_cells).copied().collect();
    if interior.clear_air.len() != expected_clear_coords.len()
        || interior
            .clear_air
            .keys()
            .any(|coord| !expected_clear_coords.contains(coord))
    {
        issues.push("cave clear-air metadata does not match the excavated network".to_owned());
    }

    let mut minimum_roof_thickness = Level::MAX;
    for coord in &metadata.covered_cells {
        let Some(floor_level) = metadata.floor_levels.get(coord).copied() else {
            issues.push(format!("cave cell {coord:?} has no exact floor level"));
            break;
        };
        let Some(clearance) = metadata.clearances.get(coord).copied() else {
            issues.push(format!("cave cell {coord:?} has no exact clearance"));
            break;
        };
        let minimum_clearance = if metadata.chamber_cells.contains(coord) {
            CHAMBER_CLEARANCE
        } else {
            CORRIDOR_CLEARANCE
        };
        let maximum_clearance =
            minimum_clearance.saturating_add(if metadata.chamber_cells.contains(coord) {
                extended_clearance_extra(settings)
            } else {
                if settings.chamber_count > 8 {
                    1
                } else {
                    0
                }
            });
        if !(minimum_clearance..=maximum_clearance).contains(&clearance) {
            issues.push(format!(
                "cave cell {coord:?} clearance {clearance} is outside its derived range"
            ));
            break;
        }
        let expected_bottom = floor_level.saturating_add(1).saturating_add(clearance);
        if metadata.roof_bottoms.get(coord).copied() != Some(expected_bottom)
            || interior.clear_air.get(coord).copied()
                != Some(LevelInterval::new(
                    floor_level.saturating_add(1),
                    expected_bottom,
                ))
        {
            issues.push(format!(
                "cave cell {coord:?} does not preserve its authored clearance"
            ));
            break;
        }
        let Some(surface) = metadata.surface_heights.get(coord).copied() else {
            issues.push(format!("cave cell {coord:?} has no surface roof"));
            break;
        };
        let roof_thickness = surface.saturating_add(1).saturating_sub(expected_bottom);
        minimum_roof_thickness = minimum_roof_thickness.min(roof_thickness);
        if roof_thickness < 3 {
            issues.push(format!(
                "cave cell {coord:?} has only {roof_thickness} roof levels"
            ));
            break;
        }
        if !valid_covered_column(
            plan.volume.columns.get(coord),
            floor_level,
            expected_bottom,
            surface,
        ) {
            issues.push(format!(
                "cave cell {coord:?} has malformed floor or cutaway roof strata"
            ));
            break;
        }
    }
    if metadata.roof_bottoms.len() != metadata.covered_cells.len()
        || metadata
            .roof_bottoms
            .keys()
            .any(|coord| !metadata.covered_cells.contains(coord))
    {
        issues.push("cave roof metadata does not exactly match covered cells".to_owned());
    }
    if metadata.clearances.len() != metadata.covered_cells.len()
        || metadata
            .clearances
            .keys()
            .any(|coord| !metadata.covered_cells.contains(coord))
    {
        issues.push("cave clearance metadata does not exactly match covered cells".to_owned());
    }

    let surfaces_by_coord = surfaces_by_coord(&plan.volume.surfaces);
    for (coord, surface_level) in &metadata.surface_heights {
        let actual = surfaces_by_coord.get(coord).cloned().unwrap_or_default();
        let expected = if let Some(ramp_level) = metadata
            .entrance_ramp
            .rows
            .iter()
            .flatten()
            .find(|position| position.coord == *coord)
            .map(|position| position.level)
        {
            BTreeSet::from([TilePos::new(*coord, ramp_level)])
        } else if metadata.covered_cells.contains(coord) {
            let floor_level = metadata
                .floor_levels
                .get(coord)
                .copied()
                .unwrap_or(settings.cave_floor_level);
            BTreeSet::from([
                TilePos::new(*coord, floor_level),
                TilePos::new(*coord, *surface_level),
            ])
        } else {
            BTreeSet::from([TilePos::new(*coord, *surface_level)])
        };
        if actual != expected {
            issues.push(format!(
                "cave column {coord:?} exposes accidental or missing footing"
            ));
            break;
        }
    }
    if plan
        .volume
        .surfaces
        .values()
        .any(|surface| surface.access != SurfaceAccess::Ordinary)
    {
        issues.push("caves contain a non-ordinary authored surface".to_owned());
    }
    if plan
        .volume
        .columns
        .values()
        .flat_map(|column| &column.elements)
        .any(|element| {
            !matches!(
                element,
                VolumeElement::Solid(SolidMass {
                    material: SolidMaterialRole::Bedrock
                        | SolidMaterialRole::Stone
                        | SolidMaterialRole::Gravel,
                    ..
                })
            )
        })
    {
        issues.push("caves contain a fill or non-Rocky material".to_owned());
    }

    let party = plan.volume.anchors.get(PARTY_START).copied();
    let hostile = plan.volume.anchors.get(HOSTILE_START).copied();
    let expected_party = metadata
        .entrance_ramp
        .rows
        .first()
        .and_then(|row| row.first())
        .copied();
    let entry_approach = cave_entry_approach(&metadata.entrance_ramp, &metadata.corridor_routes);
    let expected_hostile = metadata
        .chamber_centres
        .iter()
        .position(|centre| *centre == metadata.deepest_chamber)
        .and_then(|index| metadata.chamber_footprints.get(index))
        .and_then(|footprint| {
            safest_deep_chamber_anchor(footprint, &metadata.floor_levels, &entry_approach)
        });
    if party != expected_party || plan.volume.anchors.get(CAVE_ENTRANCE).copied() != expected_party
    {
        issues.push("cave party and entrance anchors are not ramp-derived".to_owned());
    }
    if hostile != expected_hostile
        || plan.volume.anchors.get(DEEP_CHAMBER).copied() != expected_hostile
    {
        issues.push(
            "cave hostile and deep-chamber anchors are not the safest floor in the deepest chamber"
                .to_owned(),
        );
    }
    if !metadata.chamber_centres.contains(&metadata.deepest_chamber) {
        issues.push("deep cave anchor is not a chamber centre".to_owned());
    }

    let (distances, route_steps) = if let (Some(party), Some(hostile)) = (party, hostile) {
        let distances = traversal_distances(&plan.volume, party);
        let route_steps = distances.get(&hostile).copied();
        if route_steps.is_none() {
            issues.push("the deep chamber is unreachable from the cave entrance".to_owned());
        }
        (distances, route_steps)
    } else {
        issues.push("cave actor anchors are missing".to_owned());
        (HashMap::new(), None)
    };
    if interior
        .floors
        .iter()
        .any(|floor| !distances.contains_key(floor))
    {
        issues.push("the cave contains an interior floor unreachable from its entrance".to_owned());
    }
    if metadata
        .chamber_centres
        .iter()
        .zip(&metadata.chamber_floor_levels)
        .map(|(coord, level)| TilePos::new(*coord, *level))
        .any(|centre| !distances.contains_key(&centre))
    {
        issues.push("the cave contains a chamber centre unreachable from its entrance".to_owned());
    }

    let ramp_end = metadata
        .entrance_ramp
        .rows
        .last()
        .and_then(|row| row.first())
        .map(|position| position.coord);
    if let Some(ramp_end) = ramp_end {
        let cave_network: BTreeSet<_> =
            metadata.covered_cells.union(&ramp_cells).copied().collect();
        let cave_distances = coordinate_distances(ramp_end, &cave_network);
        let farthest = metadata
            .chamber_centres
            .iter()
            .copied()
            .filter(|centre| *centre != HexCoord::ORIGIN)
            .max_by_key(|centre| {
                (
                    cave_distances.get(centre).copied().unwrap_or_default(),
                    *centre,
                )
            });
        if farthest != Some(metadata.deepest_chamber) {
            issues
                .push("the deep chamber is not the farthest chamber in the cave graph".to_owned());
        }
    }

    if !issues.is_empty() {
        return RecipeValidation::invalid(issues);
    }

    let reachable_elevation_levels = u32::try_from(
        distances
            .keys()
            .map(|position| position.level)
            .collect::<BTreeSet<_>>()
            .len(),
    )
    .unwrap_or(u32::MAX);
    let gravel = metadata
        .surface_materials
        .values()
        .filter(|material| **material == SolidMaterialRole::Gravel)
        .count();
    let environment_signature_percent = u32::try_from(gravel)
        .unwrap_or(u32::MAX)
        .saturating_mul(100)
        / u32::try_from(metadata.surface_materials.len())
            .unwrap_or(u32::MAX)
            .max(1);
    let cave_coverage_percent = u32::try_from(metadata.covered_cells.len())
        .unwrap_or(u32::MAX)
        .saturating_mul(100)
        / u32::try_from(metadata.surface_heights.len())
            .unwrap_or(u32::MAX)
            .max(1);
    let entrance_steps =
        u32::try_from(metadata.entrance_ramp.rows.len().saturating_sub(1)).unwrap_or(u32::MAX);
    let minimum_clearance = metadata
        .clearances
        .values()
        .copied()
        .min()
        .unwrap_or(CORRIDOR_CLEARANCE);
    let maximum_clearance = metadata
        .clearances
        .values()
        .copied()
        .max()
        .unwrap_or(CHAMBER_CLEARANCE);
    let spawn_height_difference = party
        .zip(hostile)
        .map(|(party, hostile)| {
            Level::try_from(party.level.abs_diff(hostile.level)).unwrap_or(Level::MAX)
        })
        .unwrap_or_default();
    let metrics = CavesMetrics {
        tactical: TacticalMetrics {
            relief: max_surface.saturating_sub(settings.cave_floor_level),
            barrier_cells: 0,
            critical_route_steps: route_steps.unwrap_or(u32::MAX),
            spawn_height_difference,
            bank_high_ground_difference: 0,
            reachable_surfaces: u32::try_from(distances.len()).unwrap_or(u32::MAX),
            reachable_elevation_levels,
            alternate_detour_percent: 0,
            river_sinuosity_percent: 0,
            environment_signature_percent,
        },
        chamber_count: u8::try_from(metadata.chamber_centres.len()).unwrap_or(u8::MAX),
        branch_nodes,
        covered_floors: u32::try_from(metadata.covered_cells.len()).unwrap_or(u32::MAX),
        minimum_roof_thickness,
        entrance_steps,
        cave_coverage_percent,
        floor_relief: max_floor.saturating_sub(min_floor),
        clearance_relief: maximum_clearance.saturating_sub(minimum_clearance),
        surface_relief: max_surface.saturating_sub(min_surface),
        extra_links: u8::try_from(metadata.extra_edges.len()).unwrap_or(u8::MAX),
    };
    RecipeValidation::valid(metrics)
}

fn validate_tree(chamber_count: usize, edges: &[(usize, usize)], issues: &mut Vec<String>) -> u8 {
    let mut adjacency = vec![Vec::new(); chamber_count];
    let mut valid_edges = true;
    for (parent, child) in edges {
        let Some(parent_neighbors) = adjacency.get_mut(*parent) else {
            valid_edges = false;
            continue;
        };
        parent_neighbors.push(*child);
        let Some(child_neighbors) = adjacency.get_mut(*child) else {
            valid_edges = false;
            continue;
        };
        child_neighbors.push(*parent);
    }
    let mut reached = BTreeSet::from([0_usize]);
    let mut frontier = VecDeque::from([0_usize]);
    while let Some(chamber) = frontier.pop_front() {
        let Some(neighbors) = adjacency.get(chamber) else {
            continue;
        };
        for neighbor in neighbors {
            if reached.insert(*neighbor) {
                frontier.push_back(*neighbor);
            }
        }
    }
    if !valid_edges
        || edges.len() != chamber_count.saturating_sub(1)
        || reached.len() != chamber_count
    {
        issues.push("cave chamber graph is not one rooted acyclic tree".to_owned());
    }
    let branch_nodes = u8::try_from(
        adjacency
            .iter()
            .filter(|neighbors| neighbors.len() >= 3)
            .count(),
    )
    .unwrap_or(u8::MAX);
    if branch_nodes == 0 {
        issues.push("cave chamber tree has no branching junction".to_owned());
    }
    branch_nodes
}

fn validate_extra_edges(
    chamber_count: usize,
    tree_edges: &[(usize, usize)],
    extra_edges: &[(usize, usize)],
    expected_count: u8,
    issues: &mut Vec<String>,
) {
    let tree: BTreeSet<_> = tree_edges
        .iter()
        .map(|(first, second)| ((*first).min(*second), (*first).max(*second)))
        .collect();
    let mut unique = BTreeSet::new();
    let valid = extra_edges.iter().all(|(first, second)| {
        let edge = ((*first).min(*second), (*first).max(*second));
        *first < chamber_count
            && *second < chamber_count
            && first != second
            && !tree.contains(&edge)
            && unique.insert(edge)
    });
    if !valid || extra_edges.len() != usize::from(expected_count) {
        issues.push("cave extra-link metadata is invalid or stale".to_owned());
    }
}

fn routes_match_declared_edges(metadata: &CavesMetadata) -> bool {
    let mut routes = metadata.corridor_routes.iter();
    let (Some(connector), Some(entrance_landing)) =
        (routes.next(), metadata.entrance_ramp.rows.last().copied())
    else {
        return false;
    };
    if connector.rows.first().copied() != Some(entrance_landing)
        || !connector.rows.last().is_some_and(|row| {
            row_lands_in_chamber(
                row,
                metadata.chamber_centres.first().copied(),
                metadata.chamber_floor_levels.first().copied(),
                metadata.chamber_footprints.first(),
            )
        })
    {
        return false;
    }

    for (route, (parent, child)) in
        routes.zip(metadata.tree_edges.iter().chain(&metadata.extra_edges))
    {
        let Some(first) = route.rows.first() else {
            return false;
        };
        let Some(last) = route.rows.last() else {
            return false;
        };
        if !row_lands_in_chamber(
            first,
            metadata.chamber_centres.get(*parent).copied(),
            metadata.chamber_floor_levels.get(*parent).copied(),
            metadata.chamber_footprints.get(*parent),
        ) || !row_lands_in_chamber(
            last,
            metadata.chamber_centres.get(*child).copied(),
            metadata.chamber_floor_levels.get(*child).copied(),
            metadata.chamber_footprints.get(*child),
        ) {
            return false;
        }
    }

    metadata.corridor_routes.len()
        == 1_usize
            .saturating_add(metadata.tree_edges.len())
            .saturating_add(metadata.extra_edges.len())
}

fn row_lands_in_chamber(
    row: &[TilePos; 2],
    centre: Option<HexCoord>,
    level: Option<Level>,
    footprint: Option<&BTreeSet<HexCoord>>,
) -> bool {
    let (Some(centre), Some(level), Some(footprint)) = (centre, level, footprint) else {
        return false;
    };
    row[0] == TilePos::new(centre, level)
        && row
            .iter()
            .all(|position| position.level == level && footprint.contains(&position.coord))
}

fn route_uses_exact_authored_floors(
    route: &CaveRoute,
    floors: &BTreeMap<HexCoord, Level>,
    entrance_floors: &BTreeMap<HexCoord, Level>,
    volume: &TerrainVolumePlan,
) -> bool {
    route.rows.iter().flatten().all(|position| {
        floors
            .get(&position.coord)
            .or_else(|| entrance_floors.get(&position.coord))
            .is_some_and(|level| *level == position.level)
            && volume.surfaces.get(position).is_some_and(|surface| {
                surface.access == SurfaceAccess::Ordinary && surface.interior == Some(INTERIOR)
            })
            && semantic_traversal_endpoint(volume, *position)
                .is_some_and(|endpoint| endpoint.is_solid)
    })
}

fn valid_ramp(route: &CaveRoute, top: Level, bottom: Level) -> bool {
    let first = route.rows.first().and_then(|row| row.first());
    let last = route.rows.last().and_then(|row| row.first());
    first.is_some_and(|position| position.level == top)
        && last.is_some_and(|position| position.level == bottom)
        && route.rows.iter().all(|row| {
            matches!(row, [first, second]
                if first.coord.distance(second.coord) == 1 && first.level == second.level)
        })
        && route.rows.windows(2).all(|pair| {
            matches!(pair, [before, after]
                if before[0].coord.distance(after[0].coord) == 1
                    && before[1].coord.distance(after[1].coord) == 1
                    && before[0].level.saturating_sub(after[0].level) == 1
                    && before[1].level.saturating_sub(after[1].level) == 1)
        })
}

fn valid_flat_route(route: &CaveRoute, floor: Level) -> bool {
    valid_sloped_route(route)
        && route
            .rows
            .iter()
            .flatten()
            .all(|position| position.level == floor)
}

fn valid_sloped_route(route: &CaveRoute) -> bool {
    !route.rows.is_empty()
        && route.rows.iter().all(|row| {
            matches!(row, [first, second]
                if first.coord.distance(second.coord) == 1 && first.level == second.level)
        })
        && route.rows.windows(2).all(|pair| {
            matches!(pair, [before, after]
                if before[0].coord.distance(after[0].coord) == 1
                    && before[1].coord.distance(after[1].coord) <= 1
                    && before[0].level.abs_diff(after[0].level) <= 1
                    && before[1].level.abs_diff(after[1].level) <= 1)
        })
}

fn route_admits_walker(route: &CaveRoute, volume: &TerrainVolumePlan) -> bool {
    route.rows.windows(2).all(|pair| {
        matches!(pair, [before, after]
            if semantic_traversal_endpoint(volume, before[0])
                .zip(semantic_traversal_endpoint(volume, after[0])).is_some_and(|(from, to)|
                TraversalProfile::WALKER.admits_transition(from, to)
                    && TraversalProfile::WALKER.admits_transition(to, from))
                && semantic_traversal_endpoint(volume, before[1])
                    .zip(semantic_traversal_endpoint(volume, after[1])).is_some_and(|(from, to)|
                    TraversalProfile::WALKER.admits_transition(from, to)
                        && TraversalProfile::WALKER.admits_transition(to, from)))
    })
}

fn semantic_traversal_endpoint(
    volume: &TerrainVolumePlan,
    position: TilePos,
) -> Option<TraversalEndpoint> {
    let surface = volume.surfaces.get(&position)?;
    if surface.access != SurfaceAccess::Ordinary {
        return None;
    }
    let column = volume.columns.get(&position.coord)?;
    let surface_top = position.level.checked_add(1)?;
    let is_solid = column.elements.iter().any(|element| {
        matches!(
            element,
            VolumeElement::Solid(mass)
                if mass.levels.bottom <= position.level && mass.levels.top == surface_top
        )
    });
    Some(TraversalEndpoint::new(
        position,
        is_solid,
        semantic_headroom(column, position),
    ))
}

fn valid_covered_column(
    column: Option<&VolumeColumn>,
    floor: Level,
    roof_bottom: Level,
    surface: Level,
) -> bool {
    let Some(column) = column else {
        return false;
    };
    let roof = column.elements.iter().find_map(|element| match element {
        VolumeElement::Solid(mass) if mass.cutaway_for == Some(INTERIOR) => Some(*mass),
        _ => None,
    });
    let tagged_count = column
        .elements
        .iter()
        .filter(|element| {
            matches!(
                element,
                VolumeElement::Solid(SolidMass {
                    cutaway_for: Some(INTERIOR),
                    ..
                })
            )
        })
        .count();
    tagged_count == 1
        && roof.is_some_and(|roof| {
            roof.levels == LevelInterval::new(roof_bottom, surface.saturating_add(1))
        })
        && column.elements.iter().all(|element| match element {
            VolumeElement::Solid(mass) if mass.levels.top <= floor.saturating_add(1) => {
                mass.cutaway_for.is_none()
            }
            VolumeElement::Solid(mass) => mass.cutaway_for == Some(INTERIOR),
            VolumeElement::Fill(_) => false,
        })
}

fn surfaces_by_coord(
    surfaces: &BTreeMap<TilePos, SurfaceMetadata>,
) -> BTreeMap<HexCoord, BTreeSet<TilePos>> {
    let mut by_coord = BTreeMap::<HexCoord, BTreeSet<TilePos>>::new();
    for position in surfaces.keys() {
        by_coord
            .entry(position.coord)
            .or_default()
            .insert(*position);
    }
    by_coord
}

fn traversal_distances(volume: &TerrainVolumePlan, start: TilePos) -> HashMap<TilePos, u32> {
    let endpoints: HashMap<_, _> = volume
        .surfaces
        .iter()
        .filter(|(_position, metadata)| metadata.access == SurfaceAccess::Ordinary)
        .filter_map(|(position, _metadata)| {
            volume.columns.get(&position.coord).map(|column| {
                (
                    *position,
                    TraversalEndpoint::new(*position, true, semantic_headroom(column, *position)),
                )
            })
        })
        .collect();
    let mut by_coord = HashMap::<HexCoord, Vec<TilePos>>::new();
    for position in endpoints.keys() {
        by_coord.entry(position.coord).or_default().push(*position);
    }
    let mut distances = HashMap::from([(start, 0_u32)]);
    let mut frontier = VecDeque::from([start]);
    while let Some(from) = frontier.pop_front() {
        let steps = distances.get(&from).copied().unwrap_or(u32::MAX);
        let Some(from_endpoint) = endpoints.get(&from).copied() else {
            continue;
        };
        for neighbor in from.coord.neighbors() {
            let Some(candidates) = by_coord.get(&neighbor) else {
                continue;
            };
            for to in candidates {
                let Some(to_endpoint) = endpoints.get(to).copied() else {
                    continue;
                };
                if !distances.contains_key(to)
                    && TraversalProfile::WALKER.admits_transition(from_endpoint, to_endpoint)
                {
                    distances.insert(*to, steps.saturating_add(1));
                    frontier.push_back(*to);
                }
            }
        }
    }
    distances
}

fn semantic_headroom(column: &VolumeColumn, position: TilePos) -> Headroom {
    let from = position.level.saturating_add(1);
    let obstruction = column
        .elements
        .iter()
        .map(element_levels)
        .find(|levels| levels.top > from);
    let clear = match obstruction {
        None => MAX_HEADROOM,
        Some(levels) if levels.bottom <= from => 0,
        Some(levels) => levels.bottom.saturating_sub(from).min(MAX_HEADROOM),
    };
    Headroom(clear)
}

const fn element_levels(element: &VolumeElement) -> LevelInterval {
    match element {
        VolumeElement::Solid(mass) => mass.levels,
        VolumeElement::Fill(fill) => fill.levels,
    }
}

fn cave_view_hint(
    grid_radius: u32,
    level_height: f32,
    surface_level: Level,
    orientation: u8,
) -> Result<MapViewHint, CandidateAttemptError> {
    let radius = i32::try_from(grid_radius)
        .map_err(|_out_of_range| CandidateAttemptError::rejected("cave radius is too large"))?;
    let normal = from_local(0, -radius, orientation).to_world(0.0);
    let horizontal_length = normal.x.mul_add(normal.x, normal.z * normal.z).sqrt();
    if horizontal_length <= f32::EPSILON {
        return Err(CandidateAttemptError::rejected(
            "cave entrance orientation has no view direction",
        ));
    }
    let radius = u16::try_from(grid_radius)
        .map_err(|_out_of_range| CandidateAttemptError::rejected("cave radius is too large"))?;
    let surface = i16::try_from(surface_level)
        .map_err(|_out_of_range| CandidateAttemptError::rejected("cave surface is too high"))?;
    let frame = (f32::from(radius) * 4.0).max(42.0);
    let focus_height = f32::from(surface) * level_height;
    Ok(MapViewHint::new(
        (
            normal.x / horizontal_length * frame,
            focus_height + frame,
            normal.z / horizontal_length * frame,
        ),
        (0.0, focus_height, 0.0),
    ))
}

const fn from_local(local_x: i32, local_y: i32, orientation: u8) -> HexCoord {
    rotate_six(HexCoord::from_axial(local_x, local_y), orientation)
}

const fn to_local(coord: HexCoord, orientation: u8) -> HexCoord {
    rotate_six(coord, (6 - orientation % 6) % 6)
}

const fn rotate_six(coord: HexCoord, turns: u8) -> HexCoord {
    match turns % 6 {
        0 => coord,
        1 => HexCoord::from_axial(-coord.z(), -coord.x()),
        2 => HexCoord::from_axial(coord.y(), coord.z()),
        3 => HexCoord::from_axial(-coord.x(), -coord.y()),
        4 => HexCoord::from_axial(coord.z(), coord.x()),
        _ => HexCoord::from_axial(-coord.y(), -coord.z()),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

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
    const CAVE_SEED: u64 = 736_283_041;

    fn palette() -> TerrainPalette {
        TerrainPalette {
            bedrock: BEDROCK,
            stone: STONE,
            dirt: DIRT,
            grass: GRASS,
            gravel: GRAVEL,
            sand: SubstanceId::AIR,
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

    const fn cave_settings(
        surface_level: Level,
        cave_floor_level: Level,
        chamber_count: u8,
    ) -> CavesSettings {
        CavesSettings {
            surface_level,
            cave_floor_level,
            chamber_count,
        }
    }

    fn settings(
        surface_level: Level,
        cave_floor_level: Level,
        chamber_count: u8,
    ) -> ProceduralV2Settings {
        ProceduralV2Settings {
            environment: V2EnvironmentSettings::Rocky,
            recipe: V2RecipeSettings::Caves(cave_settings(
                surface_level,
                cave_floor_level,
                chamber_count,
            )),
        }
    }

    #[test]
    fn legacy_caves_compatibility_map_is_pinned_connected_and_exactly_tagged() {
        let settings = settings(15, 7, 7);
        let first = build(12, 0.4, &settings, CAVE_SEED, &palette(), &is_solid)
            .expect("the legacy Caves compatibility seed should generate");
        let second = build(12, 0.4, &settings, CAVE_SEED, &palette(), &is_solid)
            .expect("the repeated Caves seed should generate");

        assert_eq!(first.map_fingerprint, second.map_fingerprint);
        assert_eq!(first.selected_candidate, second.selected_candidate);
        assert_eq!(first.selected_candidate, Some(2));
        assert_eq!(first.map_fingerprint, 6_154_715_023_132_200_237);
        assert_eq!(first.map.len(), 469);
        assert_eq!(first.candidates_evaluated, 8);
        assert!(!first.used_fallback);
        assert!(first.special_regions.is_empty());
        assert!(!first.interiors.is_empty());
        assert!(first.interiors.has_roof_voxels());
        assert_eq!(first.metadata.chamber_centres.len(), 7);
        assert_eq!(first.metrics.chamber_count, 7);
        assert_eq!(first.metrics.minimum_roof_thickness, 4);
        assert_eq!(first.metrics.entrance_steps, 8);
        assert!(first.metrics.covered_floors > 0);
        assert!(first.anchors.get(&PARTY_START.into()).is_some());
        assert!(first.anchors.get(&HOSTILE_START.into()).is_some());
        assert!(first.anchors.get(&CAVE_ENTRANCE.into()).is_some());
        assert!(first.anchors.get(&DEEP_CHAMBER.into()).is_some());
    }

    #[test]
    fn shipped_cave_selection_is_pinned() {
        let generated = build(
            12,
            0.4,
            &settings(17, 6, 12),
            CAVE_SEED,
            &palette(),
            &is_solid,
        )
        .expect("the shipped Caves selection should generate");

        assert_eq!(generated.selected_candidate, Some(2));
        assert_eq!(generated.map_fingerprint, 832_683_971_217_171_917);
        assert_eq!(generated.metrics.chamber_count, 12);
        assert_eq!(generated.metrics.extra_links, 2);
        assert!(!generated.used_fallback);
        assert_eq!(
            generated.anchors.get(&HOSTILE_START.into()),
            Some(TilePos::new(HexCoord::from_axial(4, 7), 8))
        );
        let entry_approach = cave_entry_approach(
            &generated.metadata.entrance_ramp,
            &generated.metadata.corridor_routes,
        );
        let hostile = generated
            .anchors
            .get(&HOSTILE_START.into())
            .expect("the shipped cave should retain its hostile anchor");
        let old_centre = TilePos::new(generated.metadata.deepest_chamber, 8);
        let minimum_entry_distance = |position: TilePos| {
            entry_approach
                .iter()
                .map(|approach| approach.coord.distance(position.coord))
                .min()
                .unwrap_or_default()
        };
        assert!(minimum_entry_distance(hostile) > minimum_entry_distance(old_centre));
    }

    #[test]
    fn expanded_cave_settings_add_exact_vertical_and_topological_variety() {
        let settings_cases = [
            (16, 7, 9, 1, 2, 3, 0),
            (16, 6, 10, 2, 3, 4, 1),
            (17, 6, 12, 2, 4, 5, 2),
        ];
        for (
            surface,
            floor,
            chamber_count,
            floor_relief,
            clearance_relief,
            surface_relief,
            extra_links,
        ) in settings_cases
        {
            let generated = build(
                12,
                0.4,
                &settings(surface, floor, chamber_count),
                CAVE_SEED,
                &palette(),
                &is_solid,
            )
            .unwrap_or_else(|error| {
                panic!("the {chamber_count}-chamber expanded settings should generate: {error}")
            });

            assert!(!generated.used_fallback);
            assert_eq!(
                generated.metadata.chamber_centres.len(),
                usize::from(chamber_count)
            );
            assert_eq!(generated.metrics.floor_relief, floor_relief);
            assert_eq!(generated.metrics.clearance_relief, clearance_relief);
            assert_eq!(generated.metrics.surface_relief, surface_relief);
            assert_eq!(generated.metrics.extra_links, extra_links);
            assert_ne!(
                generated.metrics.tactical.critical_route_steps,
                u32::MAX,
                "the entrance must reach the generated deep anchor"
            );

            let exact_floor_levels: BTreeSet<_> =
                generated.metadata.floor_levels.values().copied().collect();
            let exact_clearances: BTreeSet<_> =
                generated.metadata.clearances.values().copied().collect();
            assert!(exact_floor_levels.len() >= 2);
            assert!(exact_clearances.len() >= 3);

            for route in &generated.metadata.corridor_routes {
                assert!(valid_sloped_route(route));
                for position in route.rows.iter().flatten() {
                    if !generated.metadata.covered_cells.contains(&position.coord) {
                        continue;
                    }
                    let exact_floor = generated
                        .metadata
                        .floor_levels
                        .get(&position.coord)
                        .copied()
                        .expect("every covered route cell should publish its exact floor");
                    assert_eq!(position.level, exact_floor);
                    assert_eq!(generated.interiors.get(*position), Some(INTERIOR));

                    let roof_bottom = generated
                        .metadata
                        .roof_bottoms
                        .get(&position.coord)
                        .copied()
                        .expect("every covered route should publish a roof bottom");
                    let surface = generated
                        .metadata
                        .surface_heights
                        .get(&position.coord)
                        .copied()
                        .expect("every covered route should remain under the surface");
                    for level in roof_bottom..=surface {
                        assert_eq!(
                            generated
                                .interiors
                                .roof_region(TilePos::new(position.coord, level)),
                            Some(INTERIOR),
                            "every solid roof voxel over a new route must remain cutaway-tagged"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn supported_radii_counts_and_vertical_extremes_generate_valid_caves() {
        let profiles = [
            (15, 8, 6),
            (15, 8, 7),
            (15, 8, 8),
            (16, 7, 9),
            (16, 6, 10),
            (17, 6, 12),
        ];
        for radius in [12, 20, 40] {
            for (surface, floor, chamber_count) in profiles {
                let generated = build(
                    radius,
                    0.4,
                    &settings(surface, floor, chamber_count),
                    u64::from(radius).saturating_mul(100) + u64::from(chamber_count),
                    &palette(),
                    &is_solid,
                )
                .unwrap_or_else(|error| {
                    panic!("radius {radius} with {chamber_count} chambers should generate: {error}")
                });
                assert_eq!(
                    generated.metadata.chamber_centres.len(),
                    usize::from(chamber_count)
                );
                assert!(generated.metrics.minimum_roof_thickness >= 3);
            }
        }

        for (surface, floor) in [(14, 7), (15, 8), (17, 6)] {
            let generated = build(
                12,
                0.4,
                &settings(surface, floor, 7),
                CAVE_SEED,
                &palette(),
                &is_solid,
            )
            .unwrap_or_else(|error| {
                panic!("surface {surface} and floor {floor} should generate: {error}")
            });
            assert_eq!(generated.metrics.entrance_steps, surface.abs_diff(floor));
        }
    }

    #[test]
    fn fixed_seed_corpus_is_valid_deterministic_and_repair_free() {
        let profiles = [(15, 8, 7), (16, 7, 9), (16, 6, 10), (17, 6, 12)];
        for (surface, floor, chamber_count) in profiles {
            let settings = settings(surface, floor, chamber_count);
            for seed in [0, 1, 505, 808, CAVE_SEED, u64::MAX] {
                let first = build(12, 0.4, &settings, seed, &palette(), &is_solid).unwrap_or_else(
                    |error| panic!("{chamber_count}-chamber seed {seed} should generate: {error}"),
                );
                let second = build(12, 0.4, &settings, seed, &palette(), &is_solid).unwrap_or_else(
                    |error| {
                        panic!(
                            "repeated {chamber_count}-chamber seed {seed} should generate: {error}"
                        )
                    },
                );

                assert!(
                    !first.used_fallback,
                    "{chamber_count}-chamber seed {seed} unexpectedly used fallback"
                );
                assert!(first.valid_candidates > 0);
                assert!(first.repair_actions.is_empty());
                assert_eq!(first.map_fingerprint, second.map_fingerprint);
                assert_eq!(first.selected_candidate, second.selected_candidate);
            }
        }
    }

    #[test]
    fn radius_12_pr_corpus_validates_128_cave_seeds_and_named_regressions() {
        let settings = settings(17, 6, 12);
        let mut seeds: BTreeSet<u64> = (0..128).collect();
        seeds.extend([505, 808, CAVE_SEED, u64::MAX]);
        let mut fallbacks = 0_usize;

        for &seed in &seeds {
            let generated = build(12, 0.4, &settings, seed, &palette(), &is_solid)
                .unwrap_or_else(|error| panic!("radius-12 Caves seed {seed}: {error}"));
            fallbacks += usize::from(generated.used_fallback);
        }

        assert!(
            fallbacks.saturating_mul(100) < seeds.len(),
            "{fallbacks}/{} radius-12 Caves seeds used fallback",
            seeds.len()
        );
    }

    #[test]
    fn canonical_fallback_is_valid_and_deterministic() {
        let recipe = CavesRecipe { level_height: 0.4 };
        for settings in [cave_settings(15, 8, 7), cave_settings(17, 6, 12)] {
            let first = recipe
                .canonical_fallback(FallbackContext { grid_radius: 12 }, &settings)
                .expect("the canonical cave fallback should construct");
            let second = recipe
                .canonical_fallback(FallbackContext { grid_radius: 12 }, &settings)
                .expect("the repeated cave fallback should construct");

            first
                .volume
                .validate()
                .expect("the canonical cave fallback volume should validate");
            if let RecipeValidation::Invalid(issues) = validate_plan(&settings, &first) {
                panic!("canonical cave fallback should validate: {issues:?}");
            }
            assert_eq!(first.volume.columns, second.volume.columns);
            assert_eq!(first.volume.surfaces, second.volume.surfaces);
            assert_eq!(first.volume.anchors, second.volume.anchors);
            assert_eq!(first.volume.interiors, second.volume.interiors);
            assert_eq!(
                first.metadata.surface_heights,
                second.metadata.surface_heights
            );
            assert_eq!(
                first.metadata.chamber_centres,
                second.metadata.chamber_centres
            );
        }
    }

    #[test]
    fn validation_rejects_corrupt_cave_semantics() {
        let recipe = CavesRecipe { level_height: 0.4 };
        let settings = cave_settings(15, 8, 7);
        let context = CandidateContext {
            grid_radius: 12,
            candidate: 0,
            streams: super::super::seed::SeedStreams::new(CAVE_SEED, 0),
        };
        let valid = construct_plan(&recipe, context, &settings, false)
            .expect("the fixed cave candidate should construct");

        let mut broken_lane = valid.clone();
        let row = broken_lane
            .metadata
            .corridor_routes
            .first_mut()
            .and_then(|route| route.rows.first_mut())
            .expect("the valid cave should have a corridor row");
        let [first, _second] = *row;
        *row = [first, first];
        assert!(validation_issues(&settings, &broken_lane)
            .iter()
            .any(|issue| issue.contains("two-wide flat route")));

        let mut low_clearance = valid.clone();
        let chamber_coord = low_clearance
            .metadata
            .chamber_cells
            .intersection(&low_clearance.metadata.covered_cells)
            .next()
            .copied()
            .expect("the valid cave should have a covered chamber cell");
        let clear = low_clearance
            .volume
            .interiors
            .get_mut(&INTERIOR)
            .and_then(|interior| interior.clear_air.get_mut(&chamber_coord))
            .expect("the chamber should publish clear air");
        clear.top = clear.top.saturating_sub(1);
        assert!(validation_issues(&settings, &low_clearance)
            .iter()
            .any(|issue| issue.contains("authored clearance")));

        let mut untagged_roof = valid.clone();
        let roof_coord = untagged_roof
            .metadata
            .covered_cells
            .first()
            .copied()
            .expect("the valid cave should have a roof");
        let roof = untagged_roof
            .volume
            .columns
            .get_mut(&roof_coord)
            .and_then(|column| {
                column
                    .elements
                    .iter_mut()
                    .find_map(|element| match element {
                        VolumeElement::Solid(mass) if mass.cutaway_for == Some(INTERIOR) => {
                            Some(mass)
                        }
                        _ => None,
                    })
            })
            .expect("the valid cave should tag its roof");
        roof.cutaway_for = None;
        assert!(validation_issues(&settings, &untagged_roof)
            .iter()
            .any(|issue| issue.contains("cutaway roof strata")));

        let mut duplicate_chamber = valid.clone();
        let first_centre = duplicate_chamber
            .metadata
            .chamber_centres
            .first()
            .copied()
            .expect("the valid cave should have a chamber");
        *duplicate_chamber
            .metadata
            .chamber_centres
            .get_mut(1)
            .expect("the valid cave should have a second chamber") = first_centre;
        assert!(validation_issues(&settings, &duplicate_chamber)
            .iter()
            .any(|issue| issue.contains("centre uniqueness")));

        let mut cyclic_tree = valid.clone();
        cyclic_tree.metadata.tree_edges.push((1, 0));
        assert!(validation_issues(&settings, &cyclic_tree)
            .iter()
            .any(|issue| issue.contains("rooted acyclic tree")));

        let mut accidental_footing = valid;
        let (&coord, &surface) = accidental_footing
            .metadata
            .surface_heights
            .iter()
            .find(|(coord, _surface)| {
                !accidental_footing.metadata.covered_cells.contains(coord)
                    && !accidental_footing
                        .metadata
                        .entrance_ramp
                        .coords()
                        .contains(coord)
            })
            .expect("the valid cave should have an ordinary surface column");
        accidental_footing.volume.surfaces.insert(
            TilePos::new(coord, surface.saturating_sub(1)),
            SurfaceMetadata {
                access: SurfaceAccess::Ordinary,
                interior: None,
            },
        );
        assert!(validation_issues(&settings, &accidental_footing)
            .iter()
            .any(|issue| issue.contains("accidental or missing footing")));
    }

    #[test]
    fn expanded_validation_binds_routes_and_reachability_to_exact_floors() {
        let recipe = CavesRecipe { level_height: 0.4 };
        let settings = cave_settings(17, 6, 12);
        let valid = recipe
            .canonical_fallback(FallbackContext { grid_radius: 12 }, &settings)
            .expect("the expanded cave fallback should construct");
        assert!(validation_issues(&settings, &valid).is_empty());

        let mut unsafe_hostile = valid.clone();
        let unsafe_position = TilePos::new(
            unsafe_hostile.metadata.deepest_chamber,
            unsafe_hostile
                .metadata
                .floor_levels
                .get(&unsafe_hostile.metadata.deepest_chamber)
                .copied()
                .expect("the deepest chamber centre should have an exact floor"),
        );
        unsafe_hostile
            .volume
            .anchors
            .insert(HOSTILE_START.to_owned(), unsafe_position);
        unsafe_hostile
            .volume
            .anchors
            .insert(DEEP_CHAMBER.to_owned(), unsafe_position);
        assert!(validation_issues(&settings, &unsafe_hostile)
            .iter()
            .any(|issue| issue.contains("safest floor")));

        let mut reordered_routes = valid.clone();
        reordered_routes.metadata.corridor_routes.swap(1, 2);
        assert!(validation_issues(&settings, &reordered_routes)
            .iter()
            .any(|issue| issue.contains("declared graph edges")));

        let mut imaginary_route = valid.clone();
        for position in imaginary_route
            .metadata
            .corridor_routes
            .get_mut(1)
            .expect("the expanded cave should have a tree route")
            .rows
            .iter_mut()
            .flatten()
        {
            position.level = position.level.saturating_add(1);
        }
        assert!(validation_issues(&settings, &imaginary_route)
            .iter()
            .any(|issue| issue.contains("exact authored ordinary floors")));

        let mut unreachable_floor = valid.clone();
        let floor = unreachable_floor
            .volume
            .interiors
            .get(&INTERIOR)
            .and_then(|interior| interior.floors.first())
            .copied()
            .expect("the expanded cave should have an interior floor");
        unreachable_floor
            .volume
            .interiors
            .get_mut(&INTERIOR)
            .expect("the expanded cave should retain its interior")
            .floors
            .insert(TilePos::new(floor.coord, floor.level.saturating_add(10)));
        assert!(validation_issues(&settings, &unreachable_floor)
            .iter()
            .any(|issue| issue.contains("interior floor unreachable")));

        let mut unreachable_chamber = valid;
        *unreachable_chamber
            .metadata
            .chamber_floor_levels
            .get_mut(1)
            .expect("the expanded cave should have a second chamber") = 30;
        assert!(validation_issues(&settings, &unreachable_chamber)
            .iter()
            .any(|issue| issue.contains("chamber centre unreachable")));
    }

    fn validation_issues(
        settings: &CavesSettings,
        plan: &RecipePlan<CavesMetadata>,
    ) -> Vec<String> {
        match validate_plan(settings, plan) {
            RecipeValidation::Valid(_metrics) => Vec::new(),
            RecipeValidation::Invalid(issues) => issues,
        }
    }

    #[test]
    fn six_rotations_round_trip_and_preserve_distance() {
        let coord = HexCoord::from_axial(3, -5);
        for orientation in 0..6 {
            let rotated = rotate_six(coord, orientation);
            assert_eq!(to_local(rotated, orientation), coord);
            assert_eq!(
                HexCoord::ORIGIN.distance(rotated),
                HexCoord::ORIGIN.distance(coord)
            );
        }
    }

    #[test]
    #[ignore = "10,000 seeds are a manual stress corpus"]
    fn ten_thousand_seed_corpus_has_less_than_one_percent_fallbacks() {
        let settings = settings(15, 8, 7);
        let mut fallback_count = 0_u32;
        for seed in 0..10_000 {
            let generated = build(12, 0.4, &settings, seed, &palette(), &is_solid)
                .unwrap_or_else(|error| panic!("seed {seed} should generate: {error}"));
            fallback_count += u32::from(generated.used_fallback);
        }
        assert!(
            fallback_count < 100,
            "{fallback_count} of 10,000 maps used fallback"
        );
    }

    #[test]
    #[ignore = "10,000 expanded seeds are a manual stress corpus"]
    fn expanded_ten_thousand_seed_corpus_has_less_than_one_percent_fallbacks() {
        let settings = settings(17, 6, 12);
        let mut fallback_count = 0_u32;
        for seed in 0..10_000 {
            let generated = build(12, 0.4, &settings, seed, &palette(), &is_solid)
                .unwrap_or_else(|error| panic!("expanded seed {seed} should generate: {error}"));
            fallback_count += u32::from(generated.used_fallback);
        }
        assert!(
            fallback_count < 100,
            "{fallback_count} of 10,000 expanded maps used fallback"
        );
    }

    #[test]
    #[ignore = "manual release/debug generator benchmark"]
    fn cave_radius_benchmark_tracks_the_radius_40_target() {
        let palette = palette();
        let target_micros = if cfg!(debug_assertions) {
            250_000
        } else {
            50_000
        };
        for (label, settings) in [
            ("legacy", settings(15, 8, 7)),
            ("expanded", settings(17, 6, 12)),
        ] {
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
                eprintln!("{label} Caves radius {radius}: median={median}us");
                if radius == 40 {
                    radius_40_median = median;
                }
            }
            eprintln!(
                "{label} Caves radius 40 median={radius_40_median}us \
                 target={target_micros}us (trend only)"
            );
        }
    }
}
