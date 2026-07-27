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
    chamber_footprints: Vec<BTreeSet<HexCoord>>,
    tree_edges: Vec<(usize, usize)>,
    corridor_routes: Vec<CaveRoute>,
    entrance_ramp: CaveRoute,
    covered_cells: BTreeSet<HexCoord>,
    chamber_cells: BTreeSet<HexCoord>,
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
    type Score = (u8, Level, u32, u32, u32, u8);

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
        (
            metrics.chamber_count.abs_diff(settings.chamber_count),
            Level::try_from(metrics.minimum_roof_thickness.abs_diff(3)).unwrap_or(Level::MAX),
            u32::from(metrics.branch_nodes).abs_diff(2),
            metrics.cave_coverage_percent.abs_diff(18),
            metrics.tactical.environment_signature_percent.abs_diff(25),
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
    for (parent, child) in &tree_edges {
        let (Some(start), Some(end)) = (
            chamber_centres.get(*parent).copied(),
            chamber_centres.get(*child).copied(),
        ) else {
            return Err(CandidateAttemptError::rejected(
                "cave tree edge references a missing chamber",
            ));
        };
        corridor_routes.push(paired_route(
            context.grid_radius,
            start,
            end,
            settings.cave_floor_level,
        )?);
    }

    let mut chamber_footprints: Vec<_> = chamber_centres
        .iter()
        .map(|centre| {
            centre
                .within_radius(1)
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

    let surface_heights = surface_heights(context, settings, orientation, &entrance_ramp)?;
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
            let clearance = if chamber_cells.contains(coord) {
                CHAMBER_CLEARANCE
            } else {
                CORRIDOR_CLEARANCE
            };
            let roof_bottom = settings
                .cave_floor_level
                .saturating_add(1)
                .saturating_add(clearance);
            roof_bottoms.insert(*coord, roof_bottom);
            columns.insert(
                *coord,
                covered_column(settings.cave_floor_level, roof_bottom, *surface_level),
            );
            let floor = TilePos::new(*coord, settings.cave_floor_level);
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
                LevelInterval::new(settings.cave_floor_level.saturating_add(1), roof_bottom),
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
    let hostile_position = TilePos::new(deepest_chamber, settings.cave_floor_level);
    let conflict_position = TilePos::new(
        chamber_centres.first().copied().unwrap_or(HexCoord::ORIGIN),
        settings.cave_floor_level,
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
            chamber_footprints,
            tree_edges,
            corridor_routes,
            entrance_ramp,
            covered_cells,
            chamber_cells,
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

fn paired_route(
    grid_radius: u32,
    start: HexCoord,
    end: HexCoord,
    floor_level: Level,
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
                    .find(|before| **before == candidate || before.distance(candidate) == 1)
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
    let rows = centerline
        .into_iter()
        .zip(second_reversed)
        .map(|(first, second)| {
            [
                TilePos::new(first, floor_level),
                TilePos::new(second, floor_level),
            ]
        })
        .collect();
    Ok(CaveRoute { rows })
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

fn surface_heights(
    context: CandidateContext,
    settings: &CavesSettings,
    orientation: u8,
    entrance: &CaveRoute,
) -> Result<BTreeMap<HexCoord, Level>, CandidateAttemptError> {
    let radius = i32::try_from(context.grid_radius)
        .map_err(|_out_of_range| CandidateAttemptError::rejected("cave radius is too large"))?;
    let rise_cap = 17_i32.saturating_sub(settings.surface_level).min(2);
    let mound_centres = [
        from_local(-radius / 3, 0, orientation),
        from_local(radius / 3, -radius / 3, orientation),
    ];
    let landing: Vec<_> = entrance
        .rows
        .first()
        .into_iter()
        .flatten()
        .map(|position| position.coord)
        .collect();
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
    if min_surface < 14
        || max_surface > 17
        || min_surface < settings.surface_level
        || max_surface.saturating_sub(min_surface) > 2
    {
        issues.push(format!(
            "rocky cave surface is {min_surface}..={max_surface}; expected a modest level 14..=17 surface"
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
    if metadata
        .corridor_routes
        .iter()
        .any(|route| !valid_flat_route(route, settings.cave_floor_level))
    {
        issues.push("a cave corridor is not a contiguous two-wide flat route".to_owned());
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
        .covered_cells
        .iter()
        .map(|coord| TilePos::new(*coord, settings.cave_floor_level))
        .collect();
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
        let clearance = if metadata.chamber_cells.contains(coord) {
            CHAMBER_CLEARANCE
        } else {
            CORRIDOR_CLEARANCE
        };
        let expected_bottom = settings
            .cave_floor_level
            .saturating_add(1)
            .saturating_add(clearance);
        if metadata.roof_bottoms.get(coord).copied() != Some(expected_bottom)
            || interior.clear_air.get(coord).copied()
                != Some(LevelInterval::new(
                    settings.cave_floor_level.saturating_add(1),
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
            settings.cave_floor_level,
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
            BTreeSet::from([
                TilePos::new(*coord, settings.cave_floor_level),
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
    let expected_hostile = Some(TilePos::new(
        metadata.deepest_chamber,
        settings.cave_floor_level,
    ));
    if party != expected_party || plan.volume.anchors.get(CAVE_ENTRANCE).copied() != expected_party
    {
        issues.push("cave party and entrance anchors are not ramp-derived".to_owned());
    }
    if hostile != expected_hostile
        || plan.volume.anchors.get(DEEP_CHAMBER).copied() != expected_hostile
    {
        issues.push("cave hostile and deep-chamber anchors are not depth-derived".to_owned());
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

    let ordinary_count = plan.volume.surfaces.len();
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
            reachable_surfaces: u32::try_from(ordinary_count).unwrap_or(u32::MAX),
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
    !route.rows.is_empty()
        && route.rows.iter().all(|row| {
            matches!(row, [first, second]
                if first.coord.distance(second.coord) == 1
                    && first.level == floor
                    && second.level == floor)
        })
        && route.rows.windows(2).all(|pair| {
            matches!(pair, [before, after]
                if before[0].coord.distance(after[0].coord) == 1
                    && before[1].coord.distance(after[1].coord) <= 1)
        })
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
    fn shipped_caves_are_deterministic_connected_and_exactly_tagged() {
        let settings = settings(15, 8, 7);
        let first = build(12, 0.4, &settings, CAVE_SEED, &palette(), &is_solid)
            .expect("the shipped Caves seed should generate");
        let second = build(12, 0.4, &settings, CAVE_SEED, &palette(), &is_solid)
            .expect("the repeated Caves seed should generate");

        assert_eq!(first.map_fingerprint, second.map_fingerprint);
        assert_eq!(first.selected_candidate, second.selected_candidate);
        assert_eq!(first.map.len(), 469);
        assert_eq!(first.candidates_evaluated, 8);
        assert!(!first.used_fallback);
        assert!(first.special_regions.is_empty());
        assert!(!first.interiors.is_empty());
        assert!(first.interiors.has_roof_voxels());
        assert_eq!(first.metadata.chamber_centres.len(), 7);
        assert_eq!(first.metrics.chamber_count, 7);
        assert_eq!(first.metrics.minimum_roof_thickness, 3);
        assert_eq!(first.metrics.entrance_steps, 7);
        assert!(first.metrics.covered_floors > 0);
        assert!(first.anchors.get(&PARTY_START.into()).is_some());
        assert!(first.anchors.get(&HOSTILE_START.into()).is_some());
        assert!(first.anchors.get(&CAVE_ENTRANCE.into()).is_some());
        assert!(first.anchors.get(&DEEP_CHAMBER.into()).is_some());
    }

    #[test]
    fn supported_radii_counts_and_vertical_extremes_generate_valid_caves() {
        for radius in [12, 20, 40] {
            for chamber_count in 6..=8 {
                let generated = build(
                    radius,
                    0.4,
                    &settings(15, 8, chamber_count),
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
        let settings = settings(15, 8, 7);
        for seed in [0, 1, 505, 808, CAVE_SEED, u64::MAX] {
            let first = build(12, 0.4, &settings, seed, &palette(), &is_solid)
                .unwrap_or_else(|error| panic!("seed {seed} should generate: {error}"));
            let second = build(12, 0.4, &settings, seed, &palette(), &is_solid)
                .unwrap_or_else(|error| panic!("repeated seed {seed} should generate: {error}"));

            assert!(
                !first.used_fallback,
                "seed {seed} unexpectedly used fallback"
            );
            assert!(first.valid_candidates > 0);
            assert!(first.repair_actions.is_empty());
            assert_eq!(first.map_fingerprint, second.map_fingerprint);
            assert_eq!(first.selected_candidate, second.selected_candidate);
        }
    }

    #[test]
    fn canonical_fallback_is_valid_and_deterministic() {
        let recipe = CavesRecipe { level_height: 0.4 };
        let settings = cave_settings(15, 8, 7);
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
        assert!(matches!(
            validate_plan(&settings, &first),
            RecipeValidation::Valid(_)
        ));
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
    #[ignore = "manual release/debug generator benchmark"]
    fn cave_radius_benchmark_tracks_the_radius_40_target() {
        let settings = settings(15, 8, 7);
        let palette = palette();
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
            eprintln!("Caves radius {radius}: median={median}us");
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
            "Caves radius 40 median was {radius_40_median}us; target is {target_micros}us"
        );
    }
}
