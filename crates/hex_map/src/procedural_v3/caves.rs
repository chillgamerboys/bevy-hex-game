//! Native V3 cave geometry and logical cave lighting.
//!
//! The rocky surface and underground network are planned together. Tunnel air is
//! implicit between an exact floor and an exact cutaway roof; local lights are
//! gameplay semantics rooted on those floor surfaces, not renderer measurements.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use hex_core::{HexCoord, IlluminationLevel, InteriorRegionId, MapViewHint, TilePos};

use super::layout::{resolve_layout, LayoutKind, PatchId, ResolvedLayoutPlan};
use super::seed::{SeedStream, SeedStreams};
use super::selection::{
    run_recipe, CandidateAttemptError, CandidateContext, FallbackContext, RepairOutcome, V3Recipe,
    ValidatedWorldSelection, WorldValidation,
};
use super::traversal::OrdinaryGraph;
use super::volume::{
    LevelInterval, SolidMass, SolidMaterialRole, SurfaceAccess, SurfaceMetadata, VolumeColumn,
    VolumeElement, VolumePlan,
};
use super::world::{
    FeaturePlan, GeneratedWorldPlan, InteriorPlan, LightId, PlannedGameplayLight, PlannedInterior,
    StructurePlan, WorldIssueCode, WorldValidationIssue,
};
use super::V3GenerationError;
use crate::settings::{
    ProceduralV3Settings, V3CavesSettings, V3EnvironmentSettings, V3LayoutSettings,
    V3OverlaySettings, V3RecipeSettings,
};

const CORRIDOR_CLEARANCE: i32 = 3;
const CHAMBER_CLEARANCE: i32 = 4;
const MIN_ROOF_THICKNESS: i32 = 3;
const PARTY_START: &str = "party_start";
const HOSTILE_START: &str = "hostile_start";
const CONFLICT_CENTER: &str = "conflict_center";
const CAVE_ENTRANCE: &str = "cave_entrance";
const DEEP_CHAMBER: &str = "deep_chamber";

/// Recipe metrics retained by selection and the public generation report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CavesMetrics {
    pub(crate) chamber_count: u32,
    pub(crate) covered_floors: u32,
    pub(crate) critical_floors: u32,
    pub(crate) optional_dark_floors: u32,
    pub(crate) gameplay_lights: u32,
    pub(crate) minimum_roof_thickness: i32,
    pub(crate) minimum_clearance: i32,
    pub(crate) maximum_clearance: i32,
    pub(crate) surface_relief: u32,
    pub(crate) floor_relief: u32,
    pub(crate) entrance_steps: u32,
    pub(crate) critical_route_steps: u32,
    pub(crate) reachable_surfaces: u32,
    pub(crate) reachable_elevation_levels: u32,
    pub(crate) gravel_surface_percent: u32,
}

#[derive(Debug)]
struct CavesRecipe {
    level_height: f32,
    layout: ResolvedLayoutPlan,
    settings: V3CavesSettings,
    #[cfg(test)]
    reject_candidates: bool,
}

#[derive(Debug, Clone, Copy)]
struct CaveStreams<'a> {
    orientation: SeedStream<'a>,
    floors: SeedStream<'a>,
    clearances: SeedStream<'a>,
    surface: SeedStream<'a>,
    materials: SeedStream<'a>,
    lights: SeedStream<'a>,
}

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
struct CaveTopology {
    frame: PatchFrame,
    chamber_centres: Vec<HexCoord>,
    entrance: CaveRoute,
    floor_levels: BTreeMap<HexCoord, i32>,
    clearances: BTreeMap<HexCoord, i32>,
    critical_coords: BTreeSet<HexCoord>,
    optional_coords: BTreeSet<HexCoord>,
    deepest_critical: usize,
}

#[derive(Debug, Clone, Copy)]
struct PatchFrame {
    center: HexCoord,
    scale: i32,
}

/// Runs the common eight-candidate selector for one V3 Caves world.
pub(crate) fn generate(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV3Settings,
    seed: u64,
) -> Result<ValidatedWorldSelection<CavesMetrics>, V3GenerationError> {
    if !level_height.is_finite() || level_height <= 0.0 {
        return Err(V3GenerationError::RecipeContract(
            "Caves level height must be positive and finite".to_owned(),
        ));
    }
    let cave_settings = validate_recipe_settings(settings)?;
    let layout = resolve_layout(grid_radius, settings)
        .map_err(|error| V3GenerationError::RecipeContract(error.to_string()))?;
    let patch = CavePatch::new(&layout, PatchId(0))?;
    patch.validate_capacity(cave_settings)?;
    run_recipe(
        &CavesRecipe {
            level_height,
            layout,
            settings: cave_settings.clone(),
            #[cfg(test)]
            reject_candidates: false,
        },
        settings,
        grid_radius,
        seed,
    )
}

impl V3Recipe for CavesRecipe {
    type Settings = ProceduralV3Settings;
    type Metrics = CavesMetrics;
    type Score = (u32, u32, u32, u32, u8);

    fn construct(
        &self,
        context: CandidateContext,
        settings: &Self::Settings,
    ) -> Result<GeneratedWorldPlan, CandidateAttemptError> {
        #[cfg(test)]
        if self.reject_candidates {
            return Err(CandidateAttemptError::Rejected(vec![recipe_issue(
                "forced cave candidate rejection",
            )]));
        }

        validate_recipe_settings(settings).map_err(CandidateAttemptError::Fatal)?;
        if context.grid_radius != self.layout.grid_radius {
            return Err(CandidateAttemptError::Fatal(
                V3GenerationError::RecipeContract(
                    "Caves candidate radius disagrees with its resolved layout".to_owned(),
                ),
            ));
        }
        let streams = SeedStreams::new(context.seed, context.candidate, PatchId(0).0);
        let streams = CaveStreams {
            orientation: streams.stage("caves.orientation"),
            floors: streams.stage("caves.floors"),
            clearances: streams.stage("caves.clearances"),
            surface: streams.stage("caves.surface"),
            materials: streams.stage("caves.materials"),
            lights: streams.stage("caves.lights"),
        };
        construct_plan(
            self.layout.clone(),
            &self.settings,
            Some(streams),
            self.level_height,
        )
        .map_err(CandidateAttemptError::Rejected)
    }

    fn validate(
        &self,
        _settings: &Self::Settings,
        plan: &GeneratedWorldPlan,
    ) -> WorldValidation<Self::Metrics> {
        validate_caves(plan, &self.settings)
    }

    fn repair(
        &self,
        _context: CandidateContext,
        _settings: &Self::Settings,
        _plan: &mut GeneratedWorldPlan,
        _round: u8,
        _issues: &[WorldValidationIssue],
    ) -> Result<RepairOutcome, CandidateAttemptError> {
        Ok(RepairOutcome::NoChange)
    }

    fn score(
        &self,
        _settings: &Self::Settings,
        metrics: &Self::Metrics,
        candidate: u8,
    ) -> Self::Score {
        (
            metrics.optional_dark_floors.abs_diff(8),
            metrics.gameplay_lights.abs_diff(6),
            metrics.gravel_surface_percent.abs_diff(24),
            metrics.surface_relief.abs_diff(3),
            candidate,
        )
    }

    fn canonical_fallback(
        &self,
        context: FallbackContext,
        settings: &Self::Settings,
    ) -> Result<GeneratedWorldPlan, V3GenerationError> {
        validate_recipe_settings(settings)?;
        if context.grid_radius != self.layout.grid_radius {
            return Err(V3GenerationError::RecipeContract(
                "Caves fallback radius disagrees with its resolved layout".to_owned(),
            ));
        }
        construct_plan(self.layout.clone(), &self.settings, None, self.level_height).map_err(
            |issues| {
                V3GenerationError::RecipeContract(
                    issues
                        .into_iter()
                        .map(|issue| issue.detail)
                        .collect::<Vec<_>>()
                        .join("; "),
                )
            },
        )
    }
}

fn validate_recipe_settings(
    settings: &ProceduralV3Settings,
) -> Result<&V3CavesSettings, V3GenerationError> {
    let V3LayoutSettings::Single(patch) = &settings.layout else {
        return Err(V3GenerationError::RecipeUnavailable("Ring7"));
    };
    if patch.environment != V3EnvironmentSettings::Rocky {
        return Err(V3GenerationError::RecipeContract(
            "Caves requires the Rocky environment".to_owned(),
        ));
    }
    let V3RecipeSettings::Caves(caves) = &patch.recipe else {
        return Err(V3GenerationError::RecipeUnavailable(recipe_name(
            &patch.recipe,
        )));
    };
    if patch
        .overlays
        .iter()
        .any(|overlay| overlay.kind != V3OverlaySettings::Lighting)
    {
        return Err(V3GenerationError::RecipeContract(
            "Caves currently accepts only Lighting overlays".to_owned(),
        ));
    }
    Ok(caves)
}

struct CavePatch<'a> {
    layout: &'a ResolvedLayoutPlan,
    patch_id: PatchId,
    mask: &'a BTreeSet<HexCoord>,
}

impl<'a> CavePatch<'a> {
    fn new(layout: &'a ResolvedLayoutPlan, patch_id: PatchId) -> Result<Self, V3GenerationError> {
        let patch = layout.patches.get(&patch_id).ok_or_else(|| {
            V3GenerationError::RecipeContract(format!("Caves layout has no patch {patch_id:?}"))
        })?;
        Ok(Self {
            layout,
            patch_id,
            mask: &patch.mask,
        })
    }

    fn validate_capacity(&self, settings: &V3CavesSettings) -> Result<(), V3GenerationError> {
        if self.layout.kind != LayoutKind::Single {
            return Err(V3GenerationError::RecipeUnavailable("Ring7"));
        }
        let frame = patch_frame(self.mask).map_err(recipe_issues_to_error)?;
        let topology = build_topology(self.mask, frame, 0, settings, None, self.patch_id)
            .map_err(recipe_issues_to_error)?;
        if topology.chamber_centres.len() != usize::from(settings.chamber_count) {
            return Err(V3GenerationError::RecipeContract(
                "Caves footprint cannot fit the configured chamber network".to_owned(),
            ));
        }
        Ok(())
    }
}

fn construct_plan(
    layout: ResolvedLayoutPlan,
    settings: &V3CavesSettings,
    streams: Option<CaveStreams<'_>>,
    level_height: f32,
) -> Result<GeneratedWorldPlan, Vec<WorldValidationIssue>> {
    let patch_id = PatchId(0);
    let patch = layout
        .patches
        .get(&patch_id)
        .ok_or_else(|| vec![recipe_issue("Single Caves layout has no patch zero")])?;
    let mask = patch.mask.clone();
    let biome_region = patch.biome_region;
    let frame = patch_frame(&mask)?;
    let orientation = streams.map_or(0, |streams| {
        u8::try_from(streams.orientation.sample(0) % 6).unwrap_or_default()
    });
    let topology = build_topology(&mask, frame, orientation, settings, streams, patch_id)?;
    let surface_heights = build_surface_heights(&mask, settings, &topology, streams)?;
    let ramp_levels: BTreeMap<_, _> = topology
        .entrance
        .rows
        .iter()
        .flatten()
        .map(|position| (position.coord, position.level))
        .collect();
    let covered: BTreeSet<_> = topology
        .floor_levels
        .keys()
        .copied()
        .filter(|coord| !ramp_levels.contains_key(coord))
        .collect();
    let interior = InteriorRegionId(patch_id.0.saturating_add(1));

    let mut columns = BTreeMap::new();
    let mut surfaces = BTreeMap::new();
    let mut interior_floors = BTreeSet::new();
    let mut entrances = BTreeSet::new();
    let mut roof_voxels = BTreeSet::new();
    let mut surface_by_coord = BTreeMap::new();
    for coord in &mask {
        let surface_level = surface_heights.get(coord).copied().ok_or_else(|| {
            vec![recipe_issue(format!(
                "Caves surface plan omitted coordinate {coord:?}"
            ))]
        })?;
        if let Some(ramp_level) = ramp_levels.get(coord).copied() {
            let position = TilePos::new(*coord, ramp_level);
            columns.insert(*coord, entrance_column(ramp_level));
            surfaces.insert(position, ordinary_surface(Some(interior)));
            surface_by_coord.insert(*coord, position);
            interior_floors.insert(position);
            entrances.insert(position);
        } else if covered.contains(coord) {
            let floor =
                topology.floor_levels.get(coord).copied().ok_or_else(|| {
                    vec![recipe_issue("Caves covered coordinate has no floor level")]
                })?;
            let clearance =
                topology.clearances.get(coord).copied().ok_or_else(|| {
                    vec![recipe_issue("Caves covered coordinate has no clearance")]
                })?;
            let roof_bottom = floor.saturating_add(1).saturating_add(clearance);
            columns.insert(
                *coord,
                covered_column(floor, roof_bottom, surface_level, interior),
            );
            let floor_position = TilePos::new(*coord, floor);
            let surface_position = TilePos::new(*coord, surface_level);
            surfaces.insert(floor_position, ordinary_surface(Some(interior)));
            surfaces.insert(surface_position, ordinary_surface(None));
            surface_by_coord.insert(*coord, surface_position);
            interior_floors.insert(floor_position);
            for level in roof_bottom..=surface_level {
                roof_voxels.insert(TilePos::new(*coord, level));
            }
        } else {
            let gravel = streams.map_or_else(
                || fallback_gravel(*coord),
                |streams| {
                    streams
                        .materials
                        .sample_coord(coarse_coord(*coord), 0)
                        .is_multiple_of(4)
                },
            );
            columns.insert(*coord, rocky_column(surface_level, gravel));
            let position = TilePos::new(*coord, surface_level);
            surfaces.insert(position, ordinary_surface(None));
            surface_by_coord.insert(*coord, position);
        }
    }
    let volume = VolumePlan {
        mask: mask.clone(),
        columns,
        surfaces,
    };

    let party = topology
        .entrance
        .rows
        .first()
        .and_then(|row| row.first())
        .copied()
        .ok_or_else(|| vec![recipe_issue("Caves entrance has no landing")])?;
    let hostile_coord = topology
        .chamber_centres
        .get(topology.deepest_critical)
        .copied()
        .ok_or_else(|| vec![recipe_issue("Caves has no deepest critical chamber")])?;
    let hostile = interior_floor_at(&volume, hostile_coord)
        .ok_or_else(|| vec![recipe_issue("Caves deepest chamber has no exact floor")])?;
    let root_coord = topology
        .chamber_centres
        .first()
        .copied()
        .ok_or_else(|| vec![recipe_issue("Caves has no root chamber")])?;
    let conflict = interior_floor_at(&volume, root_coord)
        .ok_or_else(|| vec![recipe_issue("Caves root chamber has no exact floor")])?;
    let anchors = BTreeMap::from([
        (PARTY_START.to_owned(), party),
        (HOSTILE_START.to_owned(), hostile),
        (CONFLICT_CENTER.to_owned(), conflict),
        (CAVE_ENTRANCE.to_owned(), party),
        (DEEP_CHAMBER.to_owned(), hostile),
    ]);

    let critical_targets = exact_interior_positions(&volume, &topology.critical_coords);
    let optional_targets = exact_interior_positions(&volume, &topology.optional_coords);
    let lights = plan_lights(
        &volume,
        &critical_targets,
        &optional_targets,
        streams.map(|streams| streams.lights),
    )?;
    let biome_regions = volume
        .surfaces
        .keys()
        .copied()
        .map(|surface| (surface, biome_region))
        .collect();
    let view_hint = cave_view_hint(
        layout.grid_radius,
        level_height,
        settings.surface_level,
        topology.frame,
        orientation,
    )?;

    Ok(GeneratedWorldPlan {
        layout,
        volume,
        liquids: Default::default(),
        features: FeaturePlan::default(),
        structures: StructurePlan::default(),
        blockers: BTreeSet::new(),
        lights,
        biome_regions,
        interiors: InteriorPlan {
            by_id: BTreeMap::from([(
                interior,
                PlannedInterior {
                    floors: interior_floors,
                    entrances,
                    roof_voxels,
                },
            )]),
        },
        anchors,
        view_hint,
    })
}

fn build_topology(
    mask: &BTreeSet<HexCoord>,
    frame: PatchFrame,
    orientation: u8,
    settings: &V3CavesSettings,
    streams: Option<CaveStreams<'_>>,
    _patch_id: PatchId,
) -> Result<CaveTopology, Vec<WorldValidationIssue>> {
    let entrance = entrance_ramp(mask, frame, orientation, settings)?;
    let chamber_centres = chamber_centres(mask, frame, orientation, settings.chamber_count)?;
    let tree_edges = chamber_tree_edges(chamber_centres.len());
    let chamber_levels = chamber_floor_levels(
        settings,
        chamber_centres.len(),
        streams.map(|streams| streams.floors),
    );
    let chamber_footprints = chamber_centres
        .iter()
        .enumerate()
        .map(|(index, center)| {
            let radius = if index == 0 || index + 1 == chamber_centres.len() {
                2
            } else {
                1 + u32::from(index % 4 == 0)
            };
            center
                .within_radius(radius)
                .into_iter()
                .filter(|coord| mask.contains(coord))
                .collect::<BTreeSet<_>>()
        })
        .collect::<Vec<_>>();
    if chamber_footprints
        .iter()
        .enumerate()
        .any(|(index, footprint)| {
            footprint.is_empty()
                || chamber_footprints
                    .iter()
                    .skip(index.saturating_add(1))
                    .any(|other| !footprint.is_disjoint(other))
        })
    {
        return Err(vec![recipe_issue(
            "Caves chamber footprints overlap or escaped the patch mask",
        )]);
    }

    let ramp_end = entrance
        .rows
        .last()
        .and_then(|row| row.first())
        .copied()
        .ok_or_else(|| vec![recipe_issue("Caves entrance ramp is empty")])?;
    let root = *chamber_centres
        .first()
        .ok_or_else(|| vec![recipe_issue("Caves has no root chamber")])?;
    let mut routes = vec![paired_route(
        mask,
        ramp_end.coord,
        root,
        settings.cave_floor_level,
        settings.cave_floor_level,
    )?];
    for (parent, child) in tree_edges {
        routes.push(paired_route(
            mask,
            chamber_centres[parent],
            chamber_centres[child],
            chamber_levels[parent],
            chamber_levels[child],
        )?);
    }

    let floor_levels = reconcile_floor_levels(
        settings,
        &chamber_footprints,
        &chamber_levels,
        &mut routes,
        &entrance,
    )?;
    let ramp_coords = entrance.coords();
    let covered_coords: BTreeSet<_> = floor_levels
        .keys()
        .copied()
        .filter(|coord| !ramp_coords.contains(coord))
        .collect();
    let clearances = cave_clearances(
        settings,
        &chamber_footprints,
        &routes,
        &covered_coords,
        &floor_levels,
        streams.map(|streams| streams.clearances),
    );
    let optional_count = usize::from(settings.chamber_count >= 9).saturating_add(1);
    let critical_count = chamber_centres.len().saturating_sub(optional_count).max(1);
    let critical_coords: BTreeSet<_> = routes
        .iter()
        .take(critical_count)
        .flat_map(CaveRoute::coords)
        .chain(
            chamber_footprints
                .iter()
                .take(critical_count)
                .flat_map(|footprint| footprint.iter().copied()),
        )
        .chain(ramp_coords.iter().copied())
        .collect();
    let optional_coords: BTreeSet<_> = chamber_footprints
        .iter()
        .skip(critical_count)
        .flat_map(|footprint| footprint.iter().copied())
        .filter(|coord| !critical_coords.contains(coord))
        .collect();
    let deepest_critical = (0..critical_count)
        .max_by_key(|index| {
            entrance
                .rows
                .first()
                .and_then(|row| row.first())
                .map_or(0, |start| start.coord.distance(chamber_centres[*index]))
        })
        .unwrap_or_default();

    Ok(CaveTopology {
        frame,
        chamber_centres,
        entrance,
        floor_levels,
        clearances,
        critical_coords,
        optional_coords,
        deepest_critical,
    })
}

fn patch_frame(mask: &BTreeSet<HexCoord>) -> Result<PatchFrame, Vec<WorldValidationIssue>> {
    let center = mask
        .iter()
        .copied()
        .min_by_key(|candidate| {
            let max_distance = mask
                .iter()
                .map(|coord| candidate.distance(*coord))
                .max()
                .unwrap_or_default();
            let total_distance: u64 = mask
                .iter()
                .map(|coord| u64::from(candidate.distance(*coord)))
                .sum();
            (max_distance, total_distance, *candidate)
        })
        .ok_or_else(|| vec![recipe_issue("Caves patch mask is empty")])?;
    let max_distance = mask
        .iter()
        .map(|coord| center.distance(*coord))
        .max()
        .unwrap_or_default();
    let scale = i32::try_from(max_distance.min(12)).map_err(|error| {
        vec![recipe_issue(format!(
            "Caves patch scale is invalid: {error}"
        ))]
    })?;
    if scale < 10 {
        return Err(vec![recipe_issue(
            "Caves patch needs an effective radius of at least ten",
        )]);
    }
    Ok(PatchFrame { center, scale })
}

fn chamber_centres(
    mask: &BTreeSet<HexCoord>,
    frame: PatchFrame,
    orientation: u8,
    chamber_count: u8,
) -> Result<Vec<HexCoord>, Vec<WorldValidationIssue>> {
    const SLOTS: [(i32, i32); 12] = [
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
    let centres = SLOTS
        .into_iter()
        .take(usize::from(chamber_count))
        .map(|(q, r)| {
            let local = HexCoord::from_axial(
                scale_template(q, frame.scale),
                scale_template(r, frame.scale),
            );
            translate(frame.center, rotate(local, orientation))
        })
        .collect::<Vec<_>>();
    let unique: BTreeSet<_> = centres.iter().copied().collect();
    if centres.len() != usize::from(chamber_count)
        || unique.len() != centres.len()
        || centres.iter().any(|coord| !mask.contains(coord))
    {
        return Err(vec![recipe_issue(
            "Caves chamber template escaped or collapsed inside its patch mask",
        )]);
    }
    Ok(centres)
}

fn chamber_tree_edges(count: usize) -> Vec<(usize, usize)> {
    const EDGES: [(usize, usize); 11] = [
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
    EDGES
        .into_iter()
        .filter(|(_parent, child)| *child < count)
        .collect()
}

fn chamber_floor_levels(
    settings: &V3CavesSettings,
    count: usize,
    stream: Option<SeedStream<'_>>,
) -> Vec<i32> {
    let mut levels = vec![settings.cave_floor_level; count];
    for (parent, child) in chamber_tree_edges(count) {
        let parent_level = levels
            .get(parent)
            .copied()
            .unwrap_or(settings.cave_floor_level);
        let rises = stream.map_or(true, |stream| {
            stream
                .sample(u64::try_from(child).unwrap_or(u64::MAX))
                .is_multiple_of(3)
                || child % 4 == 0
        });
        if let Some(level) = levels.get_mut(child) {
            *level = parent_level.saturating_add(i32::from(rises));
        }
    }
    if let Some((index, level)) = levels.iter_mut().enumerate().last() {
        if index > 3 {
            *level = (*level).max(settings.cave_floor_level.saturating_add(2));
        }
    }
    levels
}

fn entrance_ramp(
    mask: &BTreeSet<HexCoord>,
    frame: PatchFrame,
    orientation: u8,
    settings: &V3CavesSettings,
) -> Result<CaveRoute, Vec<WorldValidationIssue>> {
    let descent = settings
        .surface_level
        .checked_sub(settings.cave_floor_level)
        .ok_or_else(|| vec![recipe_issue("Caves entrance descent underflowed")])?;
    let start = frame.scale.saturating_neg();
    let rows = (0..=descent)
        .map(|step| {
            let y = start.saturating_add(step);
            let level = settings.surface_level.saturating_sub(step);
            [
                TilePos::new(
                    translate(
                        frame.center,
                        rotate(HexCoord::from_axial(0, y), orientation),
                    ),
                    level,
                ),
                TilePos::new(
                    translate(
                        frame.center,
                        rotate(HexCoord::from_axial(1, y), orientation),
                    ),
                    level,
                ),
            ]
        })
        .collect::<Vec<_>>();
    if rows
        .iter()
        .flatten()
        .any(|position| !mask.contains(&position.coord))
    {
        return Err(vec![recipe_issue(
            "Caves entrance ramp escaped the patch mask",
        )]);
    }
    Ok(CaveRoute { rows })
}

fn paired_route(
    mask: &BTreeSet<HexCoord>,
    start: HexCoord,
    end: HexCoord,
    start_level: i32,
    end_level: i32,
) -> Result<CaveRoute, Vec<WorldValidationIssue>> {
    let centerline = start.line_between(end);
    let centerline_set: BTreeSet<_> = centerline.iter().copied().collect();
    let mut layers = Vec::<BTreeMap<HexCoord, Option<HexCoord>>>::new();
    for centre in &centerline {
        let previous = layers.last();
        let mut layer = BTreeMap::new();
        for candidate in centre.neighbors() {
            if !mask.contains(&candidate) || centerline_set.contains(&candidate) {
                continue;
            }
            let predecessor = match previous {
                None => Some(None),
                Some(previous) => previous
                    .keys()
                    .find(|before| before.distance(candidate) <= 1)
                    .copied()
                    .map(Some),
            };
            if let Some(predecessor) = predecessor {
                layer.insert(candidate, predecessor);
            }
        }
        if layer.is_empty() {
            return Err(vec![recipe_issue(
                "Caves corridor cannot retain its second lane",
            )]);
        }
        layers.push(layer);
    }
    let Some(mut current) = layers.last().and_then(|layer| layer.keys().next()).copied() else {
        return Err(vec![recipe_issue("Caves corridor has no second lane")]);
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
        return Err(vec![recipe_issue(
            "Caves corridor second lane is incomplete",
        )]);
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

fn reconcile_floor_levels(
    settings: &V3CavesSettings,
    footprints: &[BTreeSet<HexCoord>],
    chamber_levels: &[i32],
    routes: &mut [CaveRoute],
    entrance: &CaveRoute,
) -> Result<BTreeMap<HexCoord, i32>, Vec<WorldValidationIssue>> {
    let mut floors = BTreeMap::new();
    for (footprint, level) in footprints.iter().zip(chamber_levels) {
        for coord in footprint {
            if floors.insert(*coord, *level).is_some() {
                return Err(vec![recipe_issue("Caves chamber footprints overlap")]);
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
                return Err(vec![recipe_issue(
                    "Caves corridor row intersects incompatible chamber terraces",
                )]);
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
        if route.rows.windows(2).any(|pair| {
            !matches!(pair, [first, second] if first[0].level.abs_diff(second[0].level) <= 1
                && first.iter().all(|from| second.iter().any(|to| from.coord.distance(to.coord) <= 1)))
        }) {
            return Err(vec![recipe_issue(format!(
                "Caves floor reconciliation made corridor {route_index} unwalkable"
            ))]);
        }
    }
    for coord in entrance.coords() {
        floors.remove(&coord);
    }
    Ok(floors)
}

fn cave_clearances(
    settings: &V3CavesSettings,
    footprints: &[BTreeSet<HexCoord>],
    routes: &[CaveRoute],
    covered: &BTreeSet<HexCoord>,
    floor_levels: &BTreeMap<HexCoord, i32>,
    stream: Option<SeedStream<'_>>,
) -> BTreeMap<HexCoord, i32> {
    let mut clearances: BTreeMap<_, _> = covered
        .iter()
        .copied()
        .map(|coord| (coord, CORRIDOR_CLEARANCE))
        .collect();
    for (index, route) in routes.iter().enumerate() {
        let raised = stream.map_or(index % 3 == 1, |stream| {
            stream
                .sample(u64::try_from(index).unwrap_or(u64::MAX))
                .is_multiple_of(3)
        });
        if raised {
            for coord in route.coords() {
                if let Some(clearance) = clearances.get_mut(&coord) {
                    *clearance = 4;
                }
            }
        }
    }
    for (index, footprint) in footprints.iter().enumerate() {
        let floor = footprint
            .iter()
            .filter_map(|coord| floor_levels.get(coord).copied())
            .max()
            .unwrap_or(settings.cave_floor_level);
        let max_clearance = settings
            .surface_level
            .saturating_sub(floor)
            .saturating_sub(MIN_ROOF_THICKNESS)
            .max(CHAMBER_CLEARANCE);
        let extra = stream.map_or_else(
            || i32::try_from(index % 3).unwrap_or_default(),
            |stream| {
                i32::try_from(
                    stream.sample(100_u64.saturating_add(u64::try_from(index).unwrap_or(u64::MAX)))
                        % 3,
                )
                .unwrap_or_default()
            },
        );
        let clearance = CHAMBER_CLEARANCE.saturating_add(extra).min(max_clearance);
        for coord in footprint {
            if let Some(existing) = clearances.get_mut(coord) {
                *existing = (*existing).max(clearance);
            }
        }
    }
    clearances
}

fn build_surface_heights(
    mask: &BTreeSet<HexCoord>,
    settings: &V3CavesSettings,
    topology: &CaveTopology,
    streams: Option<CaveStreams<'_>>,
) -> Result<BTreeMap<HexCoord, i32>, Vec<WorldValidationIssue>> {
    let base = settings.surface_level.saturating_sub(3).max(14);
    let mut mound_centres = topology
        .chamber_centres
        .iter()
        .copied()
        .enumerate()
        .filter(|(index, _)| index % 2 == 0)
        .collect::<Vec<_>>();
    mound_centres.truncate(5);
    let mut heights = BTreeMap::new();
    for coord in mask {
        let rise = mound_centres
            .iter()
            .map(|(index, center)| {
                let peak = streams.map_or_else(
                    || 1 + i32::try_from(index % 3).unwrap_or_default(),
                    |streams| {
                        1 + i32::try_from(
                            streams
                                .surface
                                .sample(u64::try_from(*index).unwrap_or(u64::MAX))
                                % 3,
                        )
                        .unwrap_or_default()
                    },
                );
                peak.saturating_sub_unsigned(center.distance(*coord) / 3)
                    .max(0)
            })
            .max()
            .unwrap_or_default();
        heights.insert(
            *coord,
            base.saturating_add(rise).min(settings.surface_level),
        );
    }

    let mut frontier = VecDeque::new();
    for (coord, floor) in &topology.floor_levels {
        let Some(clearance) = topology.clearances.get(coord).copied() else {
            continue;
        };
        let required = floor
            .saturating_add(clearance)
            .saturating_add(MIN_ROOF_THICKNESS);
        if required > settings.surface_level {
            return Err(vec![recipe_issue(
                "Caves floor and clearance cannot preserve three roof levels",
            )]);
        }
        if let Some(height) = heights.get_mut(coord) {
            if *height < required {
                *height = required;
                frontier.push_back(*coord);
            }
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
            if *neighbor_height < needed {
                *neighbor_height = needed;
                frontier.push_back(neighbor);
            }
        }
    }
    Ok(heights)
}

fn plan_lights(
    volume: &VolumePlan,
    critical: &BTreeSet<TilePos>,
    optional: &BTreeSet<TilePos>,
    stream: Option<SeedStream<'_>>,
) -> Result<BTreeMap<LightId, PlannedGameplayLight>, Vec<WorldValidationIssue>> {
    let mut candidates: Vec<_> = critical.iter().copied().collect();
    candidates.sort_by_key(|position| {
        (
            stream.map_or_else(
                || fallback_light_priority(*position),
                |stream| {
                    stream.sample_coord(
                        position.coord,
                        u64::try_from(position.level).unwrap_or(u64::MAX),
                    )
                },
            ),
            *position,
        )
    });
    let mut uncovered = critical.clone();
    let mut lights = BTreeMap::new();
    while !uncovered.is_empty() {
        let selected = candidates
            .iter()
            .copied()
            .enumerate()
            .filter_map(|(index, origin)| {
                if !volume.surfaces.contains_key(&origin) {
                    return None;
                }
                let radius = 4_u32.saturating_add(stream.map_or_else(
                    || u32::try_from(index % 4).unwrap_or_default(),
                    |stream| {
                        u32::try_from(
                            stream.sample(
                                10_000_u64.saturating_add(u64::try_from(index).unwrap_or(u64::MAX)),
                            ) % 4,
                        )
                        .unwrap_or_default()
                    },
                ));
                let coverage = uncovered
                    .iter()
                    .filter(|target| illuminated(origin, radius, **target))
                    .count();
                let optional_cost = optional
                    .iter()
                    .filter(|target| illuminated(origin, radius, **target))
                    .count();
                (coverage > 0).then_some((
                    std::cmp::Reverse(coverage),
                    optional_cost,
                    origin,
                    radius,
                ))
            })
            .min();
        let Some((_, _, origin, radius)) = selected else {
            return Err(vec![recipe_issue(
                "Caves cannot cover its required route with local lights",
            )]);
        };
        let id = LightId(u32::try_from(lights.len()).unwrap_or(u32::MAX));
        lights.insert(
            id,
            PlannedGameplayLight {
                origin,
                level: IlluminationLevel::Bright,
                radius,
            },
        );
        uncovered.retain(|target| !illuminated(origin, radius, *target));
        if lights.len() > 64 {
            return Err(vec![recipe_issue(
                "Caves light planner exceeded its bounded source count",
            )]);
        }
    }
    if optional.iter().all(|target| {
        lights
            .values()
            .any(|light| illuminated(light.origin, light.radius, *target))
    }) {
        return Err(vec![recipe_issue(
            "Caves lights illuminate every optional branch floor",
        )]);
    }
    Ok(lights)
}

fn validate_caves(
    plan: &GeneratedWorldPlan,
    settings: &V3CavesSettings,
) -> WorldValidation<CavesMetrics> {
    let mut issues = plan.validate();
    if !plan.liquids.bodies.is_empty()
        || !plan.features.by_id.is_empty()
        || !plan.structures.by_id.is_empty()
        || !plan.blockers.is_empty()
    {
        issues.push(recipe_issue(
            "Caves must not contain liquids, surface features, structures, or blockers",
        ));
    }
    let Some((region, interior)) = plan.interiors.by_id.first_key_value() else {
        return WorldValidation::Invalid(vec![recipe_issue("Caves contains no interior network")]);
    };
    if plan.interiors.by_id.len() != 1 {
        issues.push(recipe_issue("Caves must contain exactly one interior"));
    }
    let Some(party) = plan.anchors.get(PARTY_START).copied() else {
        return WorldValidation::Invalid(vec![recipe_issue("Caves has no party anchor")]);
    };
    let Some(hostile) = plan.anchors.get(HOSTILE_START).copied() else {
        return WorldValidation::Invalid(vec![recipe_issue("Caves has no hostile anchor")]);
    };
    for name in [
        PARTY_START,
        HOSTILE_START,
        CONFLICT_CENTER,
        CAVE_ENTRANCE,
        DEEP_CHAMBER,
    ] {
        if !plan.anchors.contains_key(name) {
            issues.push(recipe_issue(format!("Caves is missing anchor {name:?}")));
        }
    }
    if plan.anchors.get(CAVE_ENTRANCE) != Some(&party)
        || plan.anchors.get(DEEP_CHAMBER) != Some(&hostile)
    {
        issues.push(recipe_issue(
            "Caves review anchors disagree with actor anchors",
        ));
    }
    if interior.floors.contains(&party) && interior.floors.contains(&hostile) {
        let graph = OrdinaryGraph::from_volume(&plan.volume, None);
        let distances = graph.distances_from(party);
        if !distances.contains_key(&hostile)
            || interior
                .floors
                .iter()
                .any(|floor| !distances.contains_key(floor))
        {
            issues.push(recipe_issue(
                "Caves interior is not completely walker-connected from the entrance",
            ));
        }
    } else {
        issues.push(recipe_issue(
            "Caves actor anchors are not exact interior floors",
        ));
    }

    let clearances: Vec<_> = interior
        .floors
        .iter()
        .filter(|floor| !interior.entrances.contains(floor))
        .filter_map(|floor| {
            plan.volume
                .surface_headroom(*floor)
                .map(|headroom| headroom.0)
        })
        .collect();
    let minimum_clearance = clearances.iter().copied().min().unwrap_or_default();
    let maximum_clearance = clearances.iter().copied().max().unwrap_or_default();
    if minimum_clearance < CORRIDOR_CLEARANCE || maximum_clearance < CHAMBER_CLEARANCE {
        issues.push(recipe_issue(format!(
            "Caves clearance range {minimum_clearance}..={maximum_clearance} violates corridor/chamber contracts"
        )));
    }
    let roof_thicknesses: Vec<_> = interior
        .roof_voxels
        .iter()
        .filter_map(|voxel| {
            let floor = interior
                .floors
                .iter()
                .find(|floor| floor.coord == voxel.coord)?;
            let roof_bottom = interior
                .roof_voxels
                .iter()
                .filter(|roof| roof.coord == voxel.coord)
                .map(|roof| roof.level)
                .min()?;
            let surface = plan
                .volume
                .surfaces
                .iter()
                .filter(|(surface, metadata)| {
                    surface.coord == floor.coord && metadata.interior.is_none()
                })
                .map(|(surface, _metadata)| surface.level)
                .max()?;
            Some(surface.saturating_sub(roof_bottom).saturating_add(1))
        })
        .collect();
    let minimum_roof_thickness = roof_thicknesses.iter().copied().min().unwrap_or_default();
    if minimum_roof_thickness < MIN_ROOF_THICKNESS {
        issues.push(recipe_issue(format!(
            "Caves minimum roof thickness is {minimum_roof_thickness}"
        )));
    }

    let top_surfaces: BTreeMap<_, _> = plan
        .volume
        .surfaces
        .keys()
        .filter(|surface| {
            plan.volume
                .surfaces
                .get(surface)
                .is_some_and(|metadata| metadata.interior.is_none())
        })
        .map(|surface| (surface.coord, surface.level))
        .collect();
    if top_surfaces
        .values()
        .any(|level| !(14..=settings.surface_level).contains(level))
    {
        issues.push(recipe_issue(
            "Caves rocky surface escaped levels 14 through the configured surface level",
        ));
    }
    if top_surfaces.iter().any(|(coord, level)| {
        coord.neighbors().into_iter().any(|neighbor| {
            top_surfaces
                .get(&neighbor)
                .is_some_and(|other| level.abs_diff(*other) > 1)
        })
    }) {
        issues.push(recipe_issue(
            "Caves rocky surface contains a non-walkable adjacent step",
        ));
    }

    for (id, light) in &plan.lights {
        if light.level != IlluminationLevel::Bright || !(4..=7).contains(&light.radius) {
            issues.push(recipe_issue(format!(
                "Caves light {id:?} is not Bright with radius 4..=7"
            )));
        }
        if plan
            .volume
            .surfaces
            .get(&light.origin)
            .and_then(|metadata| metadata.interior)
            != Some(*region)
        {
            issues.push(recipe_issue(format!(
                "Caves light {id:?} is not rooted inside the cave domain"
            )));
        }
    }
    let (critical, optional) = match cave_target_sets(plan, settings, party) {
        Ok(targets) => targets,
        Err(issue) => {
            issues.push(issue);
            (BTreeSet::new(), BTreeSet::new())
        }
    };
    let uncovered: Vec<_> = critical
        .iter()
        .filter(|target| {
            !plan
                .lights
                .values()
                .any(|light| illuminated(light.origin, light.radius, **target))
        })
        .copied()
        .collect();
    if !uncovered.is_empty() {
        issues.push(recipe_issue(format!(
            "Caves lights leave {} critical floors dark",
            uncovered.len()
        )));
    }
    let optional_dark_floors = optional
        .iter()
        .filter(|floor| {
            !plan
                .lights
                .values()
                .any(|light| illuminated(light.origin, light.radius, **floor))
        })
        .count();
    if optional_dark_floors == 0 {
        issues.push(recipe_issue(
            "Caves has no dark optional branch floor outside the required route",
        ));
    }

    if !issues.is_empty() {
        return WorldValidation::Invalid(issues);
    }

    let graph = OrdinaryGraph::from_volume(&plan.volume, None);
    let distances = graph.distances_from(party);
    let reachable_levels: BTreeSet<_> = distances.keys().map(|position| position.level).collect();
    let floor_min = interior
        .floors
        .iter()
        .map(|position| position.level)
        .min()
        .unwrap_or_default();
    let floor_max = interior
        .floors
        .iter()
        .map(|position| position.level)
        .max()
        .unwrap_or_default();
    let surface_min = top_surfaces.values().copied().min().unwrap_or_default();
    let surface_max = top_surfaces.values().copied().max().unwrap_or_default();
    let gravel = top_surfaces
        .iter()
        .filter(|(coord, level)| {
            surface_material(plan, TilePos::new(**coord, **level))
                == Some(SolidMaterialRole::Gravel)
        })
        .count();
    let entrance_levels: BTreeSet<_> = interior
        .entrances
        .iter()
        .map(|position| position.level)
        .collect();
    let metrics = CavesMetrics {
        chamber_count: u32::from(settings.chamber_count),
        covered_floors: count_u32(
            interior
                .floors
                .len()
                .saturating_sub(interior.entrances.len()),
        ),
        critical_floors: count_u32(critical.len()),
        optional_dark_floors: count_u32(optional_dark_floors),
        gameplay_lights: count_u32(plan.lights.len()),
        minimum_roof_thickness,
        minimum_clearance,
        maximum_clearance,
        surface_relief: surface_min.abs_diff(surface_max),
        floor_relief: floor_min.abs_diff(floor_max),
        entrance_steps: count_u32(entrance_levels.len().saturating_sub(1)),
        critical_route_steps: distances.get(&hostile).copied().unwrap_or(u32::MAX),
        reachable_surfaces: count_u32(distances.len()),
        reachable_elevation_levels: count_u32(reachable_levels.len()),
        gravel_surface_percent: count_u32(gravel)
            .saturating_mul(100)
            .checked_div(count_u32(top_surfaces.len()))
            .unwrap_or_default(),
    };
    WorldValidation::Valid(metrics)
}

fn cave_target_sets(
    plan: &GeneratedWorldPlan,
    settings: &V3CavesSettings,
    party: TilePos,
) -> Result<(BTreeSet<TilePos>, BTreeSet<TilePos>), WorldValidationIssue> {
    let patch_id = PatchId(0);
    let patch = plan
        .layout
        .patches
        .get(&patch_id)
        .ok_or_else(|| recipe_issue("Caves validation cannot find patch zero"))?;
    let frame = patch_frame(&patch.mask).map_err(|issues| {
        issues
            .into_iter()
            .next()
            .unwrap_or_else(|| recipe_issue("Caves validation cannot resolve its patch frame"))
    })?;
    let topology = (0..6)
        .filter_map(|orientation| {
            build_topology(&patch.mask, frame, orientation, settings, None, patch_id).ok()
        })
        .find(|topology| {
            topology
                .entrance
                .rows
                .first()
                .and_then(|row| row.first())
                .copied()
                == Some(party)
        })
        .ok_or_else(|| recipe_issue("Caves validation cannot recover its entrance orientation"))?;
    Ok((
        exact_interior_positions(&plan.volume, &topology.critical_coords),
        exact_interior_positions(&plan.volume, &topology.optional_coords),
    ))
}

fn exact_interior_positions(volume: &VolumePlan, coords: &BTreeSet<HexCoord>) -> BTreeSet<TilePos> {
    volume
        .surfaces
        .iter()
        .filter(|(position, metadata)| {
            coords.contains(&position.coord) && metadata.interior.is_some()
        })
        .map(|(position, _metadata)| *position)
        .collect()
}

fn interior_floor_at(volume: &VolumePlan, coord: HexCoord) -> Option<TilePos> {
    volume.surfaces.iter().find_map(|(position, metadata)| {
        (position.coord == coord && metadata.interior.is_some()).then_some(*position)
    })
}

fn ordinary_surface(interior: Option<InteriorRegionId>) -> SurfaceMetadata {
    SurfaceMetadata {
        access: SurfaceAccess::Ordinary,
        interior,
    }
}

fn rocky_column(surface: i32, gravel: bool) -> VolumeColumn {
    let mut elements = vec![
        solid(0, 1, SolidMaterialRole::Bedrock, None),
        solid(
            1,
            if gravel {
                surface
            } else {
                surface.saturating_add(1)
            },
            SolidMaterialRole::Stone,
            None,
        ),
    ];
    if gravel {
        elements.push(solid(
            surface,
            surface.saturating_add(1),
            SolidMaterialRole::Gravel,
            None,
        ));
    }
    VolumeColumn { elements }
}

fn entrance_column(surface: i32) -> VolumeColumn {
    VolumeColumn {
        elements: vec![
            solid(0, 1, SolidMaterialRole::Bedrock, None),
            solid(1, surface, SolidMaterialRole::Stone, None),
            solid(
                surface,
                surface.saturating_add(1),
                SolidMaterialRole::Gravel,
                None,
            ),
        ],
    }
}

fn covered_column(
    floor: i32,
    roof_bottom: i32,
    surface: i32,
    interior: InteriorRegionId,
) -> VolumeColumn {
    VolumeColumn {
        elements: vec![
            solid(0, 1, SolidMaterialRole::Bedrock, None),
            solid(1, floor, SolidMaterialRole::Stone, None),
            solid(
                floor,
                floor.saturating_add(1),
                SolidMaterialRole::Gravel,
                None,
            ),
            solid(
                roof_bottom,
                surface.saturating_add(1),
                SolidMaterialRole::Stone,
                Some(interior),
            ),
        ],
    }
}

fn solid(
    bottom: i32,
    top: i32,
    material: SolidMaterialRole,
    cutaway_for: Option<InteriorRegionId>,
) -> VolumeElement {
    VolumeElement::Solid(SolidMass {
        levels: LevelInterval::new(bottom, top),
        material,
        cutaway_for,
    })
}

fn surface_material(plan: &GeneratedWorldPlan, surface: TilePos) -> Option<SolidMaterialRole> {
    plan.volume
        .columns
        .get(&surface.coord)?
        .elements
        .iter()
        .find_map(|element| match element {
            VolumeElement::Solid(mass)
                if mass.levels.bottom <= surface.level && surface.level < mass.levels.top =>
            {
                Some(mass.material)
            }
            VolumeElement::Solid(_) | VolumeElement::Fill(_) => None,
        })
}

fn cave_view_hint(
    grid_radius: u32,
    level_height: f32,
    surface_level: i32,
    frame: PatchFrame,
    orientation: u8,
) -> Result<MapViewHint, Vec<WorldValidationIssue>> {
    let direction_coord = translate(
        frame.center,
        rotate(
            HexCoord::from_axial(0, frame.scale.saturating_neg()),
            orientation,
        ),
    );
    let direction = direction_coord.to_world(0.0) - frame.center.to_world(0.0);
    let horizontal = direction
        .x
        .mul_add(direction.x, direction.z * direction.z)
        .sqrt();
    if horizontal <= f32::EPSILON {
        return Err(vec![recipe_issue(
            "Caves camera direction is horizontally degenerate",
        )]);
    }
    let frame_distance =
        (f32::from(u16::try_from(grid_radius).unwrap_or(u16::MAX)) * 4.0).max(42.0);
    let focus_height = f32::from(i16::try_from(surface_level).unwrap_or(i16::MAX)) * level_height;
    let center = frame.center.to_world(focus_height);
    Ok(MapViewHint::new(
        (
            center.x + direction.x / horizontal * frame_distance,
            focus_height + frame_distance,
            center.z + direction.z / horizontal * frame_distance,
        ),
        (center.x, focus_height, center.z),
    ))
}

fn interpolated_level(start: i32, end: i32, index: usize, transitions: usize) -> i32 {
    if transitions == 0 || start == end {
        return start;
    }
    let span = start.abs_diff(end);
    let progressed = span.saturating_mul(u32::try_from(index).unwrap_or(u32::MAX))
        / u32::try_from(transitions).unwrap_or(u32::MAX).max(1);
    let progressed = i32::try_from(progressed).unwrap_or(i32::MAX);
    if end > start {
        start.saturating_add(progressed)
    } else {
        start.saturating_sub(progressed)
    }
}

fn illuminated(origin: TilePos, radius: u32, target: TilePos) -> bool {
    origin.coord.distance(target.coord) <= radius && origin.level.abs_diff(target.level) <= radius
}

fn coarse_coord(coord: HexCoord) -> HexCoord {
    HexCoord::from_axial(coord.x().div_euclid(3), coord.y().div_euclid(3))
}

fn fallback_gravel(coord: HexCoord) -> bool {
    coord
        .x()
        .saturating_add(coord.y().saturating_mul(2))
        .rem_euclid(4)
        == 0
}

fn fallback_light_priority(position: TilePos) -> u64 {
    let [x, y, z] = position.coord.to_cubic_array();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&x.to_le_bytes());
    bytes.extend_from_slice(&y.to_le_bytes());
    bytes.extend_from_slice(&z.to_le_bytes());
    bytes.extend_from_slice(&position.level.to_le_bytes());
    xxhash_rust::xxh3::xxh3_64(&bytes)
}

fn scale_template(value: i32, scale: i32) -> i32 {
    value.saturating_mul(scale) / 12
}

fn translate(origin: HexCoord, offset: HexCoord) -> HexCoord {
    HexCoord::from_axial(
        origin.x().saturating_add(offset.x()),
        origin.y().saturating_add(offset.y()),
    )
}

fn rotate(coord: HexCoord, turns: u8) -> HexCoord {
    let mut rotated = coord;
    for _ in 0..(turns % 6) {
        let [x, y, z] = rotated.to_cubic_array();
        rotated = HexCoord::new_cubic(-z, -x, -y);
    }
    rotated
}

fn count_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn recipe_issue(detail: impl Into<String>) -> WorldValidationIssue {
    WorldValidationIssue::new(WorldIssueCode::Recipe("caves"), detail)
}

fn recipe_issues_to_error(issues: Vec<WorldValidationIssue>) -> V3GenerationError {
    V3GenerationError::RecipeContract(
        issues
            .into_iter()
            .map(|issue| issue.detail)
            .collect::<Vec<_>>()
            .join("; "),
    )
}

const fn recipe_name(recipe: &V3RecipeSettings) -> &'static str {
    match recipe {
        V3RecipeSettings::Hills(_) => "Hills",
        V3RecipeSettings::SkyIslands(_) => "SkyIslands",
        V3RecipeSettings::Mountains(_) => "Mountains",
        V3RecipeSettings::Caves(_) => "Caves",
        V3RecipeSettings::Waterfall(_) => "Waterfall",
        V3RecipeSettings::Forest(_) => "Forest",
        V3RecipeSettings::Fort(_) => "Fort",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{
        PatchEdgeContractSettings, PatchEdgesSettings, PatchMaskSettings, PatchSpec,
    };
    use crate::terrain::TerrainPalette;
    use hex_core::SubstanceId;

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

    fn world_edges() -> PatchEdgesSettings {
        PatchEdgesSettings {
            east: PatchEdgeContractSettings::WorldBoundary,
            south_east: PatchEdgeContractSettings::WorldBoundary,
            south_west: PatchEdgeContractSettings::WorldBoundary,
            west: PatchEdgeContractSettings::WorldBoundary,
            north_west: PatchEdgeContractSettings::WorldBoundary,
            north_east: PatchEdgeContractSettings::WorldBoundary,
        }
    }

    fn settings() -> ProceduralV3Settings {
        ProceduralV3Settings {
            layout: V3LayoutSettings::Single(PatchSpec {
                environment: V3EnvironmentSettings::Rocky,
                recipe: V3RecipeSettings::Caves(V3CavesSettings {
                    surface_level: 17,
                    cave_floor_level: 6,
                    chamber_count: 12,
                }),
                overlays: Vec::new(),
                mask: PatchMaskSettings::WholeWorld,
                edges: world_edges(),
            }),
        }
    }

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

    #[test]
    fn hero_cave_is_native_stacked_volume_with_logical_lights() {
        let selected =
            generate(12, 0.4, &settings(), 736_283_041).expect("V3 Caves should generate");
        assert_eq!(selected.metrics.chamber_count, 12);
        assert!(selected.metrics.covered_floors > 100);
        assert!(selected.metrics.gameplay_lights > 0);
        assert!(selected.metrics.optional_dark_floors > 0);
        assert!(selected.metrics.minimum_clearance >= 3);
        assert!(selected.metrics.maximum_clearance >= 4);
        assert!(selected.metrics.minimum_roof_thickness >= 3);
        assert_eq!(selected.validated.plan.validate(), Vec::new());
    }

    #[test]
    fn named_streams_are_repeatable_and_seed_sensitive() {
        let first = generate(12, 0.4, &settings(), 41).expect("Caves should generate");
        let repeat = generate(12, 0.4, &settings(), 41).expect("Caves should repeat");
        let other = generate(12, 0.4, &settings(), 42).expect("other seed should generate");
        assert_eq!(
            first.validated.semantic_fingerprint,
            repeat.validated.semantic_fingerprint
        );
        assert_ne!(
            first.validated.semantic_fingerprint,
            other.validated.semantic_fingerprint
        );
    }

    #[test]
    fn canonical_fallback_is_independent_and_valid() {
        let settings = settings();
        let layout = resolve_layout(12, &settings).expect("layout should resolve");
        let recipe = CavesRecipe {
            level_height: 0.4,
            layout,
            settings: match &settings.layout {
                V3LayoutSettings::Single(patch) => match &patch.recipe {
                    V3RecipeSettings::Caves(caves) => caves.clone(),
                    _ => unreachable!(),
                },
                _ => unreachable!(),
            },
            reject_candidates: true,
        };
        let selected = run_recipe(&recipe, &settings, 12, 9).expect("fallback should remain valid");
        assert!(selected.used_fallback);
        assert!(selected.selected_candidate.is_none());
        assert!(matches!(
            validate_caves(&selected.validated.plan, &recipe.settings),
            WorldValidation::Valid(_)
        ));
    }

    #[test]
    fn every_critical_floor_is_bright_but_optional_darkness_remains() {
        let selected = generate(12, 0.4, &settings(), 17).expect("Caves should generate");
        let plan = &selected.validated.plan;
        let party = plan
            .anchors
            .get(PARTY_START)
            .copied()
            .expect("Caves should publish party_start");
        let (critical, optional) = cave_target_sets(
            plan,
            &match &settings().layout {
                V3LayoutSettings::Single(patch) => match &patch.recipe {
                    V3RecipeSettings::Caves(caves) => caves.clone(),
                    _ => unreachable!(),
                },
                _ => unreachable!(),
            },
            party,
        )
        .expect("Caves should recover its exact light target sets");
        assert!(critical.iter().all(|target| {
            plan.lights
                .values()
                .any(|light| illuminated(light.origin, light.radius, *target))
        }));
        assert!(optional.iter().any(|target| {
            !plan
                .lights
                .values()
                .any(|light| illuminated(light.origin, light.radius, *target))
        }));
        assert!(selected.metrics.optional_dark_floors > 0);
    }

    #[test]
    fn validator_rejects_a_missing_critical_light_network() {
        let selected = generate(12, 0.4, &settings(), 17).expect("Caves should generate");
        let mut plan = selected.validated.plan;
        plan.lights.clear();

        let WorldValidation::Invalid(issues) = validate_caves(
            &plan,
            match &settings().layout {
                V3LayoutSettings::Single(patch) => match &patch.recipe {
                    V3RecipeSettings::Caves(caves) => caves,
                    _ => unreachable!(),
                },
                _ => unreachable!(),
            },
        ) else {
            panic!("a Caves plan without critical lights must fail");
        };
        assert!(issues
            .iter()
            .any(|issue| issue.detail.contains("critical floors dark")));
    }

    #[test]
    fn fixed_seed_corpus_remains_valid_without_fallbacks() {
        for seed in [0, 1, 17, 41, 42, 808, 2_026, 736_283_041] {
            let selected =
                generate(12, 0.4, &settings(), seed).expect("fixed Caves seed should generate");
            assert!(!selected.used_fallback, "seed {seed} used fallback");
            assert!(selected.metrics.minimum_clearance >= CORRIDOR_CLEARANCE);
            assert!(selected.metrics.maximum_clearance >= CHAMBER_CLEARANCE);
            assert!(selected.metrics.optional_dark_floors > 0);
        }
    }

    #[test]
    #[ignore = "10,000 seeds are a manual V3 Caves stress corpus"]
    fn ten_thousand_seed_corpus_has_less_than_one_percent_fallbacks() {
        let mut fallbacks = 0_u32;
        for seed in 0..10_000 {
            let selected = generate(12, 0.4, &settings(), seed)
                .expect("every final Caves map should be valid");
            fallbacks = fallbacks.saturating_add(u32::from(selected.used_fallback));
        }
        assert!(fallbacks < 100, "fallbacks: {fallbacks}/10000");
    }

    #[test]
    #[ignore = "manual release/debug V3 Caves full-build benchmark"]
    fn caves_full_build_benchmark_tracks_median_and_p95() {
        let budget = if cfg!(debug_assertions) {
            std::time::Duration::from_millis(250)
        } else {
            std::time::Duration::from_millis(50)
        };
        let palette = palette();
        for radius in [12, 20, 40] {
            let warmup =
                super::super::build(radius, 0.4, &settings(), u64::MAX, &palette, &is_solid)
                    .expect("warm-up Caves should build");
            std::hint::black_box(warmup);

            let mut samples = Vec::new();
            for seed in 0..12 {
                let started = std::time::Instant::now();
                let build =
                    super::super::build(radius, 0.4, &settings(), seed, &palette, &is_solid)
                        .expect("benchmark Caves should build");
                assert!(!build.report.used_fallback);
                samples.push(started.elapsed());
                std::hint::black_box(build);
            }
            samples.sort_unstable();
            let median = samples
                .get(samples.len() / 2)
                .copied()
                .expect("the benchmark records twelve samples");
            let p95 = samples
                .last()
                .copied()
                .expect("the benchmark records twelve samples");
            eprintln!("V3 Caves full build radius {radius}: median={median:?} p95={p95:?}");
            assert!(
                median < budget && p95 < budget,
                "radius {radius} median={median:?} p95={p95:?}, budget={budget:?}"
            );
        }
    }
}
