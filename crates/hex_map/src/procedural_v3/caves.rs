//! Native V3 cave geometry and logical cave lighting.
//!
//! The rocky surface and underground network are planned together. Tunnel air is
//! implicit between an exact floor and an exact cutaway roof; local lights are
//! gameplay semantics rooted on those floor surfaces, not renderer measurements.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use hex_core::{HexCoord, IlluminationLevel, InteriorRegionId, MapViewHint, TilePos};

use super::composition::{compose_single_patch, GeneratedPatchPlan};
use super::layout::{resolve_layout, PatchId, ResolvedLayoutPlan};
use super::liquid::{LiquidBodyId, LiquidBodyPlan, LiquidFlowState, LiquidNode, LiquidPlan};
use super::patch::{PatchBuildMode, PatchRecipeContext};
use super::seam::{shape_walker_seams, validate_patch_walker_seams, WalkerSeamShape};
use super::seed::SeedStream;
use super::selection::{
    run_recipe, CandidateAttemptError, CandidateContext, FallbackContext, RepairOutcome, V3Recipe,
    ValidatedWorldSelection, WorldValidation,
};
use super::traversal::OrdinaryGraph;
use super::volume::{
    FillMaterialRole, LevelInterval, NonSolidFill, SolidMass, SolidMaterialRole, SurfaceAccess,
    SurfaceMetadata, VolumeColumn, VolumeElement, VolumePlan,
};
use super::world::{
    CaveCrystalKind, CaveCrystalPresentation, CaveCrystalSiteKind, FeaturePlan, GeneratedWorldPlan,
    InteriorPlan, LightId, PlannedGameplayLight, PlannedInterior, PlannedLightPresentation,
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
const SURFACE_WATER_LEVEL: i32 = 14;
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
    light_kinds: SeedStream<'a>,
    light_rotations: SeedStream<'a>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct CrystalLightSite {
    origin: TilePos,
    kind: CaveCrystalSiteKind,
}

#[derive(Debug, Clone, Copy)]
struct PatchFrame {
    center: HexCoord,
    scale: i32,
    max_entrance_inset: i32,
}

#[derive(Debug, Clone)]
struct CaveLiquidSink {
    body: LiquidBodyId,
    top_level: i32,
    boundary: BTreeSet<HexCoord>,
    coordinates: BTreeSet<HexCoord>,
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
    let patch = PatchRecipeContext::resolve(&layout, PatchId(0))?;
    validate_patch_capacity(&patch, cave_settings)?;
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
        let patch = PatchRecipeContext::resolve(&self.layout, PatchId(0))
            .map_err(CandidateAttemptError::Fatal)?;
        let fragment = construct_patch(
            patch,
            &self.settings,
            self.level_height,
            PatchBuildMode::Candidate {
                world_seed: context.seed,
                candidate: context.candidate,
            },
        )
        .map_err(CandidateAttemptError::Rejected)?;
        compose_single_patch(self.layout.clone(), fragment).map_err(|error| {
            CandidateAttemptError::Fatal(V3GenerationError::RecipeContract(format!(
                "Caves single-patch composition failed: {error:?}"
            )))
        })
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
        let patch = PatchRecipeContext::resolve(&self.layout, PatchId(0))?;
        let fragment = construct_patch(
            patch,
            &self.settings,
            self.level_height,
            PatchBuildMode::CanonicalFallback,
        )
        .map_err(|issues| {
            V3GenerationError::RecipeContract(
                issues
                    .into_iter()
                    .map(|issue| issue.detail)
                    .collect::<Vec<_>>()
                    .join("; "),
            )
        })?;
        compose_single_patch(self.layout.clone(), fragment).map_err(|error| {
            V3GenerationError::RecipeContract(format!(
                "Caves fallback composition failed: {error:?}"
            ))
        })
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

fn validate_patch_capacity(
    patch: &PatchRecipeContext<'_>,
    settings: &V3CavesSettings,
) -> Result<(), V3GenerationError> {
    let topology_mask = cave_topology_mask(patch);
    let frame = context_patch_frame(patch, &topology_mask).map_err(recipe_issues_to_error)?;
    let orientations: &[u8] = if patch.layout().kind == super::layout::LayoutKind::Single {
        &[0]
    } else {
        &[0, 1, 2, 3, 4, 5]
    };
    let mut rejected = Vec::new();
    let topology = orientations
        .iter()
        .find_map(|orientation| {
            match build_topology(&topology_mask, frame, *orientation, settings, None) {
                Ok(topology) => Some(topology),
                Err(issues) => {
                    rejected.push(format!(
                        "orientation {orientation}: {}",
                        issues
                            .into_iter()
                            .map(|issue| issue.detail)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                    None
                }
            }
        })
        .ok_or_else(|| {
            V3GenerationError::RecipeContract(format!(
                "Caves footprint cannot fit the configured chamber network ({})",
                rejected.join("; ")
            ))
        })?;
    if topology.chamber_centres.len() != usize::from(settings.chamber_count) {
        return Err(V3GenerationError::RecipeContract(
            "Caves footprint cannot fit the configured chamber network".to_owned(),
        ));
    }
    Ok(())
}

pub(crate) fn construct_patch(
    patch: PatchRecipeContext<'_>,
    settings: &V3CavesSettings,
    level_height: f32,
    mode: PatchBuildMode,
) -> Result<GeneratedPatchPlan, Vec<WorldValidationIssue>> {
    let streams = mode.seed_streams(&patch);
    construct_patch_with_streams(
        patch,
        settings,
        streams.map(|streams| CaveStreams {
            orientation: streams.stage("caves.orientation"),
            floors: streams.stage("caves.floors"),
            clearances: streams.stage("caves.clearances"),
            surface: streams.stage("caves.surface"),
            materials: streams.stage("caves.materials"),
            lights: streams.stage("caves.lights"),
            light_kinds: streams.stage("caves.lights.visual.kind"),
            light_rotations: streams.stage("caves.lights.visual.rotation"),
        }),
        level_height,
    )
}

fn construct_patch_with_streams(
    patch: PatchRecipeContext<'_>,
    settings: &V3CavesSettings,
    streams: Option<CaveStreams<'_>>,
    level_height: f32,
) -> Result<GeneratedPatchPlan, Vec<WorldValidationIssue>> {
    validate_patch_capacity(&patch, settings)
        .map_err(|error| vec![recipe_issue(error.to_string())])?;
    let liquid_sinks = cave_liquid_sinks(&patch)?;
    let liquid_coordinates = liquid_sinks
        .iter()
        .flat_map(|sink| sink.coordinates.iter().copied())
        .collect::<BTreeSet<_>>();
    let mask = patch.mask().clone();
    let biome_region = patch.biome_region();
    let topology_mask = cave_topology_mask(&patch);
    let frame = context_patch_frame(&patch, &topology_mask)?;
    let requested_orientation = streams.map_or(0, |streams| {
        u8::try_from(streams.orientation.sample(0) % 6).unwrap_or_default()
    });
    let (orientation, topology, surface_heights, seam_shape) = compatible_patch_geometry(
        &patch,
        &topology_mask,
        frame,
        requested_orientation,
        settings,
        streams,
        &liquid_coordinates,
    )?;
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
    let interior = InteriorRegionId(1);

    let mut columns = BTreeMap::new();
    let mut surfaces = BTreeMap::new();
    let mut interior_floors = BTreeSet::new();
    let mut entrances = BTreeSet::new();
    let mut roof_voxels = BTreeSet::new();
    let mut surface_by_coord = BTreeMap::new();
    let liquid_levels = liquid_sinks
        .iter()
        .flat_map(|sink| {
            sink.coordinates
                .iter()
                .copied()
                .map(move |coord| (coord, sink.top_level))
        })
        .collect::<BTreeMap<_, _>>();
    for coord in &mask {
        let surface_level = surface_heights.get(coord).copied().ok_or_else(|| {
            vec![recipe_issue(format!(
                "Caves surface plan omitted coordinate {coord:?}"
            ))]
        })?;
        if let Some(water_level) = liquid_levels.get(coord).copied() {
            let (column, bed) = surface_water_column(*coord, water_level);
            columns.insert(*coord, column);
            surfaces.insert(
                bed,
                SurfaceMetadata {
                    access: SurfaceAccess::NonStandable,
                    interior: None,
                },
            );
            surface_by_coord.insert(*coord, bed);
        } else if let Some(ramp_level) = ramp_levels.get(coord).copied() {
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
    let mut volume = VolumePlan {
        mask: mask.clone(),
        columns,
        surfaces,
    };
    seam_shape.apply(&mut volume)?;

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
    let mut protected_positions = anchors.values().copied().collect::<BTreeSet<_>>();
    protected_positions.extend(
        patch
            .protected_approaches()
            .into_iter()
            .map(|coord| TilePos::new(coord, 0)),
    );
    let lights = plan_lights(
        &volume,
        &critical_targets,
        &optional_targets,
        &protected_positions,
        streams,
    )?;
    carve_crystal_alcoves(
        &mut volume,
        &mut interior_floors,
        &mut entrances,
        &mut roof_voxels,
        interior,
        &lights,
    )?;
    let biome_regions = volume
        .surfaces
        .keys()
        .copied()
        .map(|surface| (surface, biome_region))
        .collect();
    let view_hint = cave_view_hint(
        patch.grid_radius(),
        level_height,
        settings.surface_level,
        topology.frame,
        orientation,
    )?;

    let fragment = GeneratedPatchPlan {
        patch_id: patch.id,
        volume,
        liquids: LiquidPlan {
            bodies: liquid_sinks
                .iter()
                .map(|sink| {
                    (
                        sink.body,
                        LiquidBodyPlan {
                            material: FillMaterialRole::Water,
                            nodes: sink
                                .coordinates
                                .iter()
                                .copied()
                                .map(|coord| {
                                    (
                                        TilePos::new(coord, sink.top_level),
                                        LiquidNode {
                                            state: LiquidFlowState::Still,
                                            downstream: None,
                                        },
                                    )
                                })
                                .collect(),
                        },
                    )
                })
                .collect(),
        },
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
    };
    let mut issues = validate_patch_walker_seams(&patch, &fragment.volume);
    issues.extend(validate_cave_liquid_sinks(&patch, &fragment, &liquid_sinks));
    issues.extend(
        fragment
            .validate_against(patch.layout())
            .into_iter()
            .map(|issue| {
                recipe_issue(format!(
                    "Caves patch {:?} failed {:?}: {}",
                    issue.patch, issue.code, issue.detail
                ))
            }),
    );
    if issues.is_empty() {
        Ok(fragment)
    } else {
        Err(issues)
    }
}

fn compatible_patch_geometry(
    patch: &PatchRecipeContext<'_>,
    topology_mask: &BTreeSet<HexCoord>,
    frame: PatchFrame,
    requested_orientation: u8,
    settings: &V3CavesSettings,
    streams: Option<CaveStreams<'_>>,
    liquid_coordinates: &BTreeSet<HexCoord>,
) -> Result<(u8, CaveTopology, BTreeMap<HexCoord, i32>, WalkerSeamShape), Vec<WorldValidationIssue>>
{
    let mut last_issues = Vec::new();
    let orientation_offsets: &[u8] = if patch.layout().kind == super::layout::LayoutKind::Single {
        &[0]
    } else {
        &[0, 1, 2, 3, 4, 5]
    };
    for offset in orientation_offsets {
        let orientation = requested_orientation.saturating_add(*offset) % 6;
        let topology = match build_topology(topology_mask, frame, orientation, settings, streams) {
            Ok(topology) => topology,
            Err(issues) => {
                last_issues = issues;
                continue;
            }
        };
        let mut surface_heights =
            match build_surface_heights(patch.mask(), settings, &topology, streams) {
                Ok(heights) => heights,
                Err(issues) => {
                    last_issues = issues;
                    continue;
                }
            };
        let seam_shape = match shape_walker_seams(patch, &mut surface_heights) {
            Ok(shape) => shape,
            Err(issues) => {
                last_issues = issues;
                continue;
            }
        };
        let ramp_conflicts = topology.entrance.rows.iter().flatten().any(|position| {
            seam_shape.is_boundary(position.coord)
                || seam_shape
                    .required_surface(position.coord)
                    .is_some_and(|required| required != *position)
        });
        let underground_seam_crossing = topology
            .floor_levels
            .keys()
            .any(|coord| seam_shape.is_boundary(*coord));
        let underground_liquid_overlap = topology
            .floor_levels
            .keys()
            .chain(
                topology
                    .entrance
                    .rows
                    .iter()
                    .flatten()
                    .map(|position| &position.coord),
            )
            .any(|coord| liquid_coordinates.contains(coord));
        if ramp_conflicts || underground_seam_crossing || underground_liquid_overlap {
            last_issues = vec![recipe_issue(format!(
                "Caves orientation {orientation} overlaps a protected shared seam or liquid sink"
            ))];
            continue;
        }
        return Ok((orientation, topology, surface_heights, seam_shape));
    }
    if last_issues.is_empty() {
        last_issues.push(recipe_issue(
            "Caves found no entrance orientation compatible with its shared seams",
        ));
    }
    Err(last_issues)
}

fn cave_liquid_sinks(
    patch: &PatchRecipeContext<'_>,
) -> Result<Vec<CaveLiquidSink>, Vec<WorldValidationIssue>> {
    let mut sinks = Vec::new();
    let mut occupied = BTreeSet::new();
    for edge in patch.shared_edges() {
        let Some((is_source, port)) = edge.liquid_port() else {
            continue;
        };
        if is_source {
            return Err(vec![recipe_issue(format!(
                "Caves patch {:?} cannot source directed liquid edge {:?}",
                patch.id, edge.id
            ))]);
        }

        let top_level =
            SURFACE_WATER_LEVEL.clamp(edge.contract.elevation.min, edge.contract.elevation.max);
        let boundary = port
            .lanes
            .iter()
            .map(|(local, _neighbor)| *local)
            .collect::<BTreeSet<_>>();
        let coordinates = boundary
            .iter()
            .copied()
            .chain(port.first_approach.iter().copied())
            .collect::<BTreeSet<_>>();
        if boundary.len() != port.lanes.len()
            || coordinates.is_empty()
            || coordinates
                .iter()
                .any(|coord| !patch.mask().contains(coord))
            || !connected_coords(&coordinates)
        {
            return Err(vec![recipe_issue(format!(
                "Caves incoming liquid edge {:?} does not form one connected in-patch sink",
                edge.id
            ))]);
        }
        if coordinates.iter().any(|coord| !occupied.insert(*coord)) {
            return Err(vec![recipe_issue(
                "Caves incoming liquid sink footprints overlap",
            )]);
        }
        sinks.push(CaveLiquidSink {
            body: LiquidBodyId(u32::try_from(sinks.len()).unwrap_or(u32::MAX)),
            top_level,
            boundary,
            coordinates,
        });
    }
    Ok(sinks)
}

fn validate_cave_liquid_sinks(
    patch: &PatchRecipeContext<'_>,
    fragment: &GeneratedPatchPlan,
    expected: &[CaveLiquidSink],
) -> Vec<WorldValidationIssue> {
    let mut issues = Vec::new();
    if patch.layout().kind == super::layout::LayoutKind::Single && !expected.is_empty() {
        issues.push(recipe_issue(
            "Single Caves must not declare a directed liquid sink",
        ));
    }
    if fragment.liquids.bodies.len() != expected.len() {
        issues.push(recipe_issue(format!(
            "Caves has {} liquid bodies, expected {} incoming sink bodies",
            fragment.liquids.bodies.len(),
            expected.len()
        )));
    }

    for sink in expected {
        let Some(body) = fragment.liquids.bodies.get(&sink.body) else {
            issues.push(recipe_issue(format!(
                "Caves is missing incoming liquid body {:?}",
                sink.body
            )));
            continue;
        };
        let expected_nodes = sink
            .coordinates
            .iter()
            .copied()
            .map(|coord| TilePos::new(coord, sink.top_level))
            .collect::<BTreeSet<_>>();
        let actual_nodes = body.nodes.keys().copied().collect::<BTreeSet<_>>();
        if body.material != FillMaterialRole::Water || actual_nodes != expected_nodes {
            issues.push(recipe_issue(format!(
                "Caves liquid body {:?} does not exactly cover its incoming port and approach",
                sink.body
            )));
        }
        if body
            .nodes
            .values()
            .any(|node| node.state != LiquidFlowState::Still || node.downstream.is_some())
        {
            issues.push(recipe_issue(format!(
                "Caves liquid body {:?} must be a terminal still-water sink",
                sink.body
            )));
        }
        for coord in &sink.boundary {
            if !body
                .nodes
                .contains_key(&TilePos::new(*coord, sink.top_level))
            {
                issues.push(recipe_issue(format!(
                    "Caves liquid body {:?} omits boundary endpoint {coord:?}",
                    sink.body
                )));
            }
        }
        for position in expected_nodes {
            let bed = TilePos::new(position.coord, position.level.saturating_sub(1));
            let bed_is_non_standable = fragment.volume.surfaces.get(&bed).is_some_and(|metadata| {
                metadata.access == SurfaceAccess::NonStandable && metadata.interior.is_none()
            });
            let valid_column = fragment
                .volume
                .columns
                .get(&position.coord)
                .is_some_and(|column| {
                    let gravel_bed = column.elements.iter().any(|element| {
                        matches!(
                            element,
                            VolumeElement::Solid(SolidMass {
                                levels,
                                material: SolidMaterialRole::Gravel,
                                cutaway_for: None,
                            }) if *levels
                                == LevelInterval::new(
                                    position.level.saturating_sub(1),
                                    position.level
                                )
                        )
                    });
                    let exact_fill = column.elements.iter().any(|element| {
                        matches!(
                            element,
                            VolumeElement::Fill(NonSolidFill {
                                levels,
                                material: FillMaterialRole::Water,
                            }) if *levels
                                == LevelInterval::new(
                                    position.level,
                                    position.level.saturating_add(1)
                                )
                        )
                    });
                    gravel_bed && exact_fill
                });
            let overlaps_interior = fragment.interiors.by_id.values().any(|interior| {
                interior
                    .floors
                    .iter()
                    .any(|floor| floor.coord == position.coord)
            });
            if !bed_is_non_standable || !valid_column || overlaps_interior {
                issues.push(recipe_issue(format!(
                    "Caves liquid sink {position:?} lacks its exact exterior water-bed contract"
                )));
            }
        }
    }
    issues
}

fn connected_coords(coordinates: &BTreeSet<HexCoord>) -> bool {
    let Some(start) = coordinates.first().copied() else {
        return false;
    };
    let mut reached = BTreeSet::from([start]);
    let mut frontier = VecDeque::from([start]);
    while let Some(coord) = frontier.pop_front() {
        for neighbor in coord.neighbors() {
            if coordinates.contains(&neighbor) && reached.insert(neighbor) {
                frontier.push_back(neighbor);
            }
        }
    }
    reached.len() == coordinates.len()
}

fn build_topology(
    mask: &BTreeSet<HexCoord>,
    frame: PatchFrame,
    orientation: u8,
    settings: &V3CavesSettings,
    streams: Option<CaveStreams<'_>>,
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
        let Some((parent_center, child_center, parent_level, child_level)) = chamber_centres
            .get(parent)
            .zip(chamber_centres.get(child))
            .zip(chamber_levels.get(parent))
            .zip(chamber_levels.get(child))
            .map(
                |(((parent_center, child_center), parent_level), child_level)| {
                    (*parent_center, *child_center, *parent_level, *child_level)
                },
            )
        else {
            return Err(vec![recipe_issue(
                "Caves chamber tree references a missing chamber",
            )]);
        };
        routes.push(paired_route(
            mask,
            parent_center,
            child_center,
            parent_level,
            child_level,
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
                .and_then(|start| {
                    chamber_centres
                        .get(*index)
                        .map(|center| start.coord.distance(*center))
                })
                .unwrap_or_default()
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
    Ok(PatchFrame {
        center,
        scale,
        max_entrance_inset: 0,
    })
}

fn cave_topology_mask(patch: &PatchRecipeContext<'_>) -> BTreeSet<HexCoord> {
    let shared_boundary: BTreeSet<_> = patch
        .shared_edges()
        .flat_map(|edge| edge.boundary_pairs().into_iter().map(|(local, _)| local))
        .collect();
    patch.mask().difference(&shared_boundary).copied().collect()
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

fn context_patch_frame(
    patch: &PatchRecipeContext<'_>,
    mask: &BTreeSet<HexCoord>,
) -> Result<PatchFrame, Vec<WorldValidationIssue>> {
    let mut frame = patch_frame(mask)?;
    if patch.layout().kind == super::layout::LayoutKind::Ring7 {
        frame.max_entrance_inset = 4;
    }
    Ok(frame)
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
        let rises = stream.is_none_or(|stream| {
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
    for inset in 0..=frame.max_entrance_inset {
        let start = frame.scale.saturating_neg().saturating_add(inset);
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
            .all(|position| mask.contains(&position.coord))
        {
            return Ok(CaveRoute { rows });
        }
    }
    Err(vec![recipe_issue(
        "Caves entrance ramp escaped the patch mask",
    )])
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
    protected: &BTreeSet<TilePos>,
    streams: Option<CaveStreams<'_>>,
) -> Result<BTreeMap<LightId, PlannedGameplayLight>, Vec<WorldValidationIssue>> {
    let mut candidates = crystal_light_candidates(volume, critical, protected);
    candidates.sort_by_key(|site| {
        (
            streams.map_or_else(
                || fallback_light_priority(site.origin),
                |streams| {
                    streams.lights.sample_coord(
                        site.origin.coord,
                        u64::try_from(site.origin.level)
                            .unwrap_or(u64::MAX)
                            .wrapping_add(u64::from(crystal_site_kind_tag(site.kind)) << 56),
                    )
                },
            ),
            *site,
        )
    });
    let mut uncovered = critical.clone();
    let mut lights = BTreeMap::new();
    while !uncovered.is_empty() {
        let selected = candidates
            .iter()
            .copied()
            .enumerate()
            .filter_map(|(index, site)| {
                if lights.values().any(|light: &PlannedGameplayLight| {
                    light.origin.coord.distance(site.origin.coord) <= 2
                }) {
                    return None;
                }
                let radius = 4_u32.saturating_add(streams.map_or_else(
                    || u32::try_from(index % 4).unwrap_or_default(),
                    |streams| {
                        u32::try_from(
                            streams.lights.sample(
                                10_000_u64.saturating_add(u64::try_from(index).unwrap_or(u64::MAX)),
                            ) % 4,
                        )
                        .unwrap_or_default()
                    },
                ));
                let coverage = uncovered
                    .iter()
                    .filter(|target| illuminated(site.origin, radius, **target))
                    .count();
                let optional_cost = optional
                    .iter()
                    .filter(|target| illuminated(site.origin, radius, **target))
                    .count();
                (coverage > 0).then_some((std::cmp::Reverse(coverage), optional_cost, site, radius))
            })
            .min();
        let Some((_, _, site, radius)) = selected else {
            return Err(vec![recipe_issue(
                "Caves cannot cover its required route with local lights",
            )]);
        };
        let id = LightId(u32::try_from(lights.len()).unwrap_or(u32::MAX));
        let supported = supported_crystal_kinds(volume, site, protected);
        let salt = u64::from(id.0)
            .wrapping_shl(32)
            .wrapping_add(u64::from(
                u32::try_from(site.origin.level).unwrap_or(u32::MAX),
            ))
            .wrapping_add(u64::from(crystal_site_kind_tag(site.kind)) << 56);
        let kind_sample = streams.map_or_else(
            || fallback_crystal_sample(site.origin, id, 0),
            |streams| streams.light_kinds.sample_coord(site.origin.coord, salt),
        );
        let kind_index = usize::try_from(
            kind_sample % u64::try_from(supported.len()).unwrap_or(u64::MAX).max(1),
        )
        .unwrap_or_default();
        let kind = supported.get(kind_index).copied().ok_or_else(|| {
            vec![recipe_issue(
                "Caves selected a light site without a supported crystal kind",
            )]
        })?;
        let rotation_sample = streams.map_or_else(
            || fallback_crystal_sample(site.origin, id, 1),
            |streams| {
                streams
                    .light_rotations
                    .sample_coord(site.origin.coord, salt)
            },
        );
        let rotation = u8::try_from(rotation_sample % 6).unwrap_or_default();
        lights.insert(
            id,
            PlannedGameplayLight {
                origin: site.origin,
                level: IlluminationLevel::Bright,
                radius,
                presentation: Some(PlannedLightPresentation::CaveCrystal(
                    CaveCrystalPresentation {
                        kind,
                        site: site.kind,
                        rotation,
                    },
                )),
            },
        );
        uncovered.retain(|target| !illuminated(site.origin, radius, *target));
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

fn crystal_light_candidates(
    volume: &VolumePlan,
    critical: &BTreeSet<TilePos>,
    protected: &BTreeSet<TilePos>,
) -> Vec<CrystalLightSite> {
    let mut candidates = BTreeSet::new();
    let highest_critical_level = critical.iter().map(|position| position.level).max();
    for target in critical {
        for coord in target.coord.within_radius(2) {
            let origin = TilePos::new(coord, target.level);
            let is_open_entrance = volume
                .surfaces
                .iter()
                .any(|(position, metadata)| *position == origin && metadata.interior.is_some())
                && volume.surfaces.iter().all(|(position, metadata)| {
                    position.coord != coord || metadata.interior.is_some()
                });
            if is_open_entrance {
                continue;
            }
            let existing_interior: Vec<_> = volume
                .surfaces
                .iter()
                .filter(|(position, metadata)| {
                    position.coord.distance(coord) <= 1 && metadata.interior.is_some()
                })
                .map(|(position, _metadata)| *position)
                .collect();
            if existing_interior
                .iter()
                .any(|position| position.level != origin.level)
            {
                continue;
            }
            let connects_to_interior = volume.surfaces.iter().any(|(position, metadata)| {
                position.coord.distance(coord) <= 2
                    && position.level == origin.level
                    && metadata.interior.is_some()
            });
            let site = CrystalLightSite {
                origin,
                kind: CaveCrystalSiteKind::InteriorAlcove,
            };
            if connects_to_interior && !supported_crystal_kinds(volume, site, protected).is_empty()
            {
                candidates.insert(site);
            }

            // The entrance spends every row on a one-level descent. Reserve a
            // connected side landing near its top instead of flattening that ramp.
            if highest_critical_level.is_none_or(|highest| target.level < highest.saturating_sub(2))
            {
                continue;
            }
            let entrance_site = CrystalLightSite {
                origin,
                kind: CaveCrystalSiteKind::EntranceLanding,
            };
            if connects_to_interior
                && !supported_crystal_kinds(volume, entrance_site, protected).is_empty()
            {
                candidates.insert(entrance_site);
            }
        }
    }
    candidates.into_iter().collect()
}

fn supported_crystal_kinds(
    volume: &VolumePlan,
    site: CrystalLightSite,
    protected: &BTreeSet<TilePos>,
) -> Vec<CaveCrystalKind> {
    [
        CaveCrystalKind::LowCluster,
        CaveCrystalKind::Branched,
        CaveCrystalKind::Spire,
    ]
    .into_iter()
    .filter(|kind| crystal_site_supports(volume, site, crystal_alcove_height(*kind), protected))
    .collect()
}

const fn crystal_alcove_height(kind: CaveCrystalKind) -> i32 {
    let height = kind.height();
    if height < CORRIDOR_CLEARANCE {
        CORRIDOR_CLEARANCE
    } else {
        height
    }
}

fn volume_occupied(volume: &VolumePlan, position: TilePos) -> bool {
    volume.columns.get(&position.coord).is_none_or(|column| {
        column.elements.iter().any(|element| {
            let levels = match element {
                VolumeElement::Solid(mass) => mass.levels,
                VolumeElement::Fill(fill) => fill.levels,
            };
            levels.bottom <= position.level && position.level < levels.top
        })
    })
}

fn crystal_site_supports(
    volume: &VolumePlan,
    site: CrystalLightSite,
    height: i32,
    protected: &BTreeSet<TilePos>,
) -> bool {
    if !(2..=4).contains(&height)
        || protected
            .iter()
            .any(|position| position.coord.distance(site.origin.coord) <= 1)
    {
        return false;
    }
    let clear_top = site.origin.level.saturating_add(1).saturating_add(height);
    site.origin.coord.within_radius(1).into_iter().all(|coord| {
        let Some(column) = volume.columns.get(&coord) else {
            return false;
        };
        if column
            .elements
            .iter()
            .any(|element| matches!(element, VolumeElement::Fill(_)))
        {
            return false;
        }
        if site.kind == CaveCrystalSiteKind::EntranceLanding {
            return true;
        }

        let outer_surface = volume
            .surfaces
            .iter()
            .filter(|(position, metadata)| position.coord == coord && metadata.interior.is_none())
            .map(|(position, _metadata)| position.level)
            .max();
        let Some(outer_surface) = outer_surface else {
            return false;
        };
        let retained_roof: Vec<_> = column
            .elements
            .iter()
            .filter_map(|element| {
                let VolumeElement::Solid(mass) = element else {
                    return None;
                };
                let bottom = mass.levels.bottom.max(clear_top);
                (bottom < mass.levels.top).then_some(LevelInterval::new(bottom, mass.levels.top))
            })
            .collect();
        let roof_contains_surface = retained_roof
            .iter()
            .any(|levels| levels.bottom <= outer_surface && outer_surface < levels.top);
        let roof_bottom = retained_roof
            .iter()
            .map(|levels| levels.bottom)
            .min()
            .unwrap_or(i32::MAX);
        roof_contains_surface
            && outer_surface.saturating_sub(roof_bottom).saturating_add(1) >= MIN_ROOF_THICKNESS
    })
}

fn carve_crystal_alcoves(
    volume: &mut VolumePlan,
    interior_floors: &mut BTreeSet<TilePos>,
    entrances: &mut BTreeSet<TilePos>,
    roof_voxels: &mut BTreeSet<TilePos>,
    interior: InteriorRegionId,
    lights: &BTreeMap<LightId, PlannedGameplayLight>,
) -> Result<(), Vec<WorldValidationIssue>> {
    for (id, light) in lights {
        let Some(PlannedLightPresentation::CaveCrystal(crystal)) = light.presentation else {
            return Err(vec![recipe_issue(format!(
                "Caves light {id:?} has no crystal presentation"
            ))]);
        };
        if crystal.rotation >= 6 {
            return Err(vec![recipe_issue(format!(
                "Caves light {id:?} has invalid crystal rotation {}",
                crystal.rotation
            ))]);
        }

        let opens_to_sky = crystal.site == CaveCrystalSiteKind::EntranceLanding;
        for coord in light.origin.coord.within_radius(1) {
            let Some(column) = volume.columns.get(&coord).cloned() else {
                return Err(vec![recipe_issue(format!(
                    "Caves light {id:?} crystal alcove escaped the patch at {coord:?}"
                ))]);
            };
            let was_entrance = entrances.iter().any(|position| position.coord == coord);
            let carved = carve_crystal_alcove_column(
                &column,
                light.origin.level,
                crystal_alcove_height(crystal.kind),
                interior,
                opens_to_sky,
            )?;
            volume.columns.insert(coord, carved);

            volume.surfaces.retain(|position, metadata| {
                position.coord != coord || (!opens_to_sky && metadata.interior != Some(interior))
            });
            interior_floors.retain(|position| position.coord != coord);
            entrances.retain(|position| position.coord != coord);
            roof_voxels.retain(|position| position.coord != coord);

            let floor = TilePos::new(coord, light.origin.level);
            volume
                .surfaces
                .insert(floor, ordinary_surface(Some(interior)));
            interior_floors.insert(floor);
            if was_entrance || opens_to_sky {
                entrances.insert(floor);
            }
            let Some(carved) = volume.columns.get(&coord) else {
                return Err(vec![recipe_issue(
                    "Caves lost a crystal alcove column while rebuilding it",
                )]);
            };
            for element in &carved.elements {
                let VolumeElement::Solid(mass) = element else {
                    continue;
                };
                if mass.cutaway_for == Some(interior) {
                    roof_voxels.extend(
                        (mass.levels.bottom..mass.levels.top)
                            .map(|level| TilePos::new(coord, level)),
                    );
                }
            }
        }
    }
    Ok(())
}

fn carve_crystal_alcove_column(
    column: &VolumeColumn,
    floor: i32,
    height: i32,
    interior: InteriorRegionId,
    opens_to_sky: bool,
) -> Result<VolumeColumn, Vec<WorldValidationIssue>> {
    let clear_top = floor.saturating_add(1).saturating_add(height);
    let mut elements = vec![
        solid(0, 1, SolidMaterialRole::Bedrock, None),
        solid(1, floor, SolidMaterialRole::Stone, None),
        solid(
            floor,
            floor.saturating_add(1),
            SolidMaterialRole::Gravel,
            None,
        ),
    ];
    for element in &column.elements {
        match element {
            VolumeElement::Solid(mass) => {
                if opens_to_sky {
                    continue;
                }
                let bottom = mass.levels.bottom.max(clear_top);
                if bottom < mass.levels.top {
                    elements.push(solid(
                        bottom,
                        mass.levels.top,
                        mass.material,
                        Some(interior),
                    ));
                }
            }
            VolumeElement::Fill(_) => {
                return Err(vec![recipe_issue(
                    "Caves cannot carve a crystal alcove through a non-solid fill",
                )]);
            }
        }
    }
    Ok(VolumeColumn { elements })
}

pub(crate) fn validate_caves(
    plan: &GeneratedWorldPlan,
    settings: &V3CavesSettings,
) -> WorldValidation<CavesMetrics> {
    let mut issues = plan.validate();
    if plan.layout.kind == super::layout::LayoutKind::Single && !plan.liquids.bodies.is_empty() {
        issues.push(recipe_issue("Single Caves must not contain liquids"));
    }
    if !plan.features.by_id.is_empty()
        || !plan.structures.by_id.is_empty()
        || !plan.blockers.is_empty()
    {
        issues.push(recipe_issue(
            "Caves must not contain surface features, structures, or blockers",
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
            plan.volume.surfaces.get(surface).is_some_and(|metadata| {
                metadata.interior.is_none() && metadata.access == SurfaceAccess::Ordinary
            })
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

    let light_origins: Vec<_> = plan.lights.values().map(|light| light.origin).collect();
    if light_origins.iter().enumerate().any(|(index, origin)| {
        light_origins
            .iter()
            .skip(index.saturating_add(1))
            .any(|other| origin.coord.distance(other.coord) <= 2)
    }) {
        issues.push(recipe_issue(
            "Caves crystal alcoves overlap instead of remaining distinct landmarks",
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
        if plan
            .anchors
            .values()
            .any(|anchor| anchor.coord.distance(light.origin.coord) <= 1)
        {
            issues.push(recipe_issue(format!(
                "Caves light {id:?} crystal footprint overlaps a protected actor anchor"
            )));
        }
        let Some(PlannedLightPresentation::CaveCrystal(crystal)) = light.presentation else {
            issues.push(recipe_issue(format!(
                "Caves light {id:?} has no cave-crystal presentation"
            )));
            continue;
        };
        if crystal.rotation >= 6 {
            issues.push(recipe_issue(format!(
                "Caves light {id:?} crystal rotation {} is outside 0..6",
                crystal.rotation
            )));
        }
        let alcove_height = crystal_alcove_height(crystal.kind);
        let mut invalid_floor = false;
        let mut invalid_site_geometry = false;
        let mut occupied_visual = false;
        for coord in light.origin.coord.within_radius(1) {
            let floor = TilePos::new(coord, light.origin.level);
            if plan
                .volume
                .surfaces
                .get(&floor)
                .and_then(|metadata| metadata.interior)
                != Some(*region)
                || !interior.floors.contains(&floor)
                || plan
                    .volume
                    .surface_headroom(floor)
                    .is_none_or(|headroom| headroom.0 < alcove_height)
            {
                invalid_floor = true;
            }
            occupied_visual |= (1..=alcove_height).any(|offset| {
                volume_occupied(
                    &plan.volume,
                    TilePos::new(coord, light.origin.level.saturating_add(offset)),
                )
            });
            match crystal.site {
                CaveCrystalSiteKind::EntranceLanding => {
                    invalid_site_geometry |= !interior.entrances.contains(&floor)
                        || plan.volume.surfaces.iter().any(|(position, metadata)| {
                            position.coord == coord && metadata.interior.is_none()
                        })
                        || interior
                            .roof_voxels
                            .iter()
                            .any(|position| position.coord == coord);
                }
                CaveCrystalSiteKind::InteriorAlcove => {
                    let outer_surface = plan
                        .volume
                        .surfaces
                        .iter()
                        .filter(|(position, metadata)| {
                            position.coord == coord && metadata.interior.is_none()
                        })
                        .map(|(position, _metadata)| position.level)
                        .max();
                    let roof_bottom = plan
                        .volume
                        .columns
                        .get(&coord)
                        .into_iter()
                        .flat_map(|column| &column.elements)
                        .filter_map(|element| match element {
                            VolumeElement::Solid(mass) if mass.cutaway_for == Some(*region) => {
                                Some(mass.levels.bottom)
                            }
                            VolumeElement::Solid(_) | VolumeElement::Fill(_) => None,
                        })
                        .min();
                    invalid_site_geometry |=
                        outer_surface
                            .zip(roof_bottom)
                            .is_none_or(|(outer, bottom)| {
                                bottom
                                    < light
                                        .origin
                                        .level
                                        .saturating_add(1)
                                        .saturating_add(alcove_height)
                                    || outer.saturating_sub(bottom).saturating_add(1)
                                        < MIN_ROOF_THICKNESS
                                    || !interior.roof_voxels.contains(&TilePos::new(coord, bottom))
                            });
                }
            }
        }
        if invalid_floor {
            issues.push(recipe_issue(format!(
                "Caves light {id:?} does not own a flat radius-one crystal alcove with {alcove_height} clear levels"
            )));
        }
        if occupied_visual {
            issues.push(recipe_issue(format!(
                "Caves light {id:?} crystal visual envelope intersects semantic occupancy"
            )));
        }
        if invalid_site_geometry {
            issues.push(recipe_issue(format!(
                "Caves light {id:?} does not satisfy its exact {:?} floor and roof contract",
                crystal.site
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
    let patch = plan
        .layout
        .patches
        .iter()
        .next()
        .map(|(_patch_id, patch)| patch)
        .ok_or_else(|| recipe_issue("Caves validation cannot find its isolated patch"))?;
    let frame = patch_frame(&patch.mask).map_err(|issues| {
        issues
            .into_iter()
            .next()
            .unwrap_or_else(|| recipe_issue("Caves validation cannot resolve its patch frame"))
    })?;
    let topology = (0..6)
        .filter_map(|orientation| {
            build_topology(&patch.mask, frame, orientation, settings, None).ok()
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

fn surface_water_column(coord: HexCoord, water_level: i32) -> (VolumeColumn, TilePos) {
    let bed_level = water_level.saturating_sub(1);
    (
        VolumeColumn {
            elements: vec![
                solid(0, 1, SolidMaterialRole::Bedrock, None),
                solid(1, bed_level, SolidMaterialRole::Stone, None),
                solid(bed_level, water_level, SolidMaterialRole::Gravel, None),
                VolumeElement::Fill(NonSolidFill {
                    levels: LevelInterval::new(water_level, water_level.saturating_add(1)),
                    material: FillMaterialRole::Water,
                }),
            ],
        },
        TilePos::new(coord, bed_level),
    )
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

const fn crystal_site_kind_tag(kind: CaveCrystalSiteKind) -> u8 {
    match kind {
        CaveCrystalSiteKind::InteriorAlcove => 0,
        CaveCrystalSiteKind::EntranceLanding => 1,
    }
}

fn fallback_crystal_sample(position: TilePos, id: LightId, domain: u8) -> u64 {
    let [x, y, z] = position.coord.to_cubic_array();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"caves.crystal");
    bytes.push(domain);
    bytes.extend_from_slice(&id.0.to_le_bytes());
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
        EdgeElevationSettings, EdgeLiquidPortSettings, EdgeLiquidSettings,
        PatchEdgeContractSettings, PatchEdgesSettings, PatchMaskSettings, PatchSpec,
        SharedEdgeSettings, V3ForestSettings, V3FortSettings, V3HillsSettings, V3MountainsSettings,
        V3Ring7Settings, V3SkyIslandsSettings, V3WaterfallSettings, WalkerPortSettings,
    };
    use crate::terrain::TerrainPalette;
    use hex_core::SubstanceId;

    use super::super::layout::HexSide;

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
    const WORKED_STONE: SubstanceId = SubstanceId(12);

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

    fn cave_inlet_ring_settings() -> ProceduralV3Settings {
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
                    surface_level: 16,
                    cave_floor_level: 7,
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
        for (first, first_side, second, second_side) in [
            (0, HexSide::NorthEast, 1, HexSide::SouthWest),
            (0, HexSide::East, 2, HexSide::West),
            (0, HexSide::SouthEast, 3, HexSide::NorthWest),
            (0, HexSide::SouthWest, 4, HexSide::NorthEast),
            (0, HexSide::West, 5, HexSide::East),
            (0, HexSide::NorthWest, 6, HexSide::SouthEast),
            (1, HexSide::SouthEast, 2, HexSide::NorthWest),
            (2, HexSide::SouthWest, 3, HexSide::NorthEast),
            (3, HexSide::West, 4, HexSide::East),
            (4, HexSide::NorthWest, 5, HexSide::SouthEast),
            (5, HexSide::NorthEast, 6, HexSide::SouthWest),
            (6, HexSide::East, 1, HexSide::West),
        ] {
            set_ring_edge(
                ring_patch_mut(&mut ring, first),
                first_side,
                ring_shared(EdgeLiquidSettings::Dry),
            );
            set_ring_edge(
                ring_patch_mut(&mut ring, second),
                second_side,
                ring_shared(EdgeLiquidSettings::Dry),
            );
        }
        set_ring_edge(
            &mut ring.center,
            HexSide::West,
            ring_shared(EdgeLiquidSettings::Outlet(EdgeLiquidPortSettings {
                width: 3,
            })),
        );
        set_ring_edge(
            &mut ring.caves,
            HexSide::East,
            ring_shared(EdgeLiquidSettings::Inlet(EdgeLiquidPortSettings {
                width: 3,
            })),
        );
        ProceduralV3Settings {
            layout: V3LayoutSettings::Ring7(ring),
        }
    }

    fn generated_patch(environment: V3EnvironmentSettings, recipe: V3RecipeSettings) -> PatchSpec {
        PatchSpec {
            environment,
            recipe,
            overlays: Vec::new(),
            mask: PatchMaskSettings::GeneratedRegion,
            edges: world_edges(),
        }
    }

    fn ring_patch_mut(ring: &mut V3Ring7Settings, id: u32) -> &mut PatchSpec {
        match id {
            0 => &mut ring.center,
            1 => &mut ring.mountains,
            2 => &mut ring.waterfall,
            3 => &mut ring.forest,
            4 => &mut ring.fort,
            5 => &mut ring.caves,
            6 => &mut ring.sky_islands,
            _ => unreachable!("fixed Ring7 patch id"),
        }
    }

    fn set_ring_edge(patch: &mut PatchSpec, side: HexSide, contract: PatchEdgeContractSettings) {
        *match side {
            HexSide::East => &mut patch.edges.east,
            HexSide::SouthEast => &mut patch.edges.south_east,
            HexSide::SouthWest => &mut patch.edges.south_west,
            HexSide::West => &mut patch.edges.west,
            HexSide::NorthWest => &mut patch.edges.north_west,
            HexSide::NorthEast => &mut patch.edges.north_east,
        } = contract;
    }

    fn ring_shared(liquid: EdgeLiquidSettings) -> PatchEdgeContractSettings {
        PatchEdgeContractSettings::Shared(SharedEdgeSettings {
            elevation: EdgeElevationSettings {
                preferred: 15,
                min: 14,
                max: 16,
            },
            walker: WalkerPortSettings { count: 2, width: 2 },
            liquid,
            approach_depth: 3,
        })
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
            worked_stone: WORKED_STONE,
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
    fn single_caves_remain_liquid_free() {
        for seed in [0, 33, 445, 736_283_041] {
            let selected =
                generate(12, 0.4, &settings(), seed).expect("Single Caves should remain valid");
            assert!(
                selected.validated.plan.liquids.bodies.is_empty(),
                "Single seed {seed} unexpectedly materialized surface water"
            );
        }
    }

    #[test]
    fn ring_cave_sink_exactly_covers_its_inlet_and_approach() {
        let settings = cave_inlet_ring_settings();
        let layout = resolve_layout(33, &settings).expect("liquid Ring7 fixture should resolve");
        let patch =
            PatchRecipeContext::resolve(&layout, PatchId(5)).expect("Caves patch should resolve");
        let expected = cave_liquid_sinks(&patch).expect("Caves inlet should resolve");
        let [expected_sink] = expected.as_slice() else {
            panic!("Caves should resolve exactly one incoming liquid sink");
        };
        assert_eq!(expected_sink.top_level, SURFACE_WATER_LEVEL);
        assert_eq!(expected_sink.boundary.len(), 3);

        let caves = match &settings.layout {
            V3LayoutSettings::Ring7(ring) => match &ring.caves.recipe {
                V3RecipeSettings::Caves(caves) => caves,
                _ => unreachable!(),
            },
            _ => unreachable!(),
        };
        for mode in [
            PatchBuildMode::Candidate {
                world_seed: 0,
                candidate: 2,
            },
            PatchBuildMode::CanonicalFallback,
        ] {
            let fragment = construct_patch(patch, caves, 0.4, mode)
                .expect("Caves should fit around its exact incoming liquid sink");
            assert_eq!(fragment.validate_against(&layout), Vec::new());
            assert_eq!(
                validate_cave_liquid_sinks(&patch, &fragment, &expected),
                Vec::new()
            );

            let body = fragment
                .liquids
                .bodies
                .get(&LiquidBodyId(0))
                .expect("Caves should publish one sink body");
            assert_eq!(
                body.nodes.keys().copied().collect::<BTreeSet<_>>(),
                expected_sink
                    .coordinates
                    .iter()
                    .copied()
                    .map(|coord| TilePos::new(coord, SURFACE_WATER_LEVEL))
                    .collect()
            );
            assert!(expected_sink.boundary.iter().all(|coord| {
                body.nodes
                    .contains_key(&TilePos::new(*coord, SURFACE_WATER_LEVEL))
            }));
            assert!(fragment.interiors.by_id.values().all(|interior| {
                interior
                    .floors
                    .iter()
                    .all(|floor| !expected_sink.coordinates.contains(&floor.coord))
            }));
        }
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
    fn validator_rejects_crystal_floor_roof_and_visual_volume_corruption() {
        let selected = generate(12, 0.4, &settings(), 33).expect("Caves should generate");
        let caves_settings = match &settings().layout {
            V3LayoutSettings::Single(patch) => match &patch.recipe {
                V3RecipeSettings::Caves(caves) => caves.clone(),
                _ => unreachable!(),
            },
            _ => unreachable!(),
        };
        let region = *selected
            .validated
            .plan
            .interiors
            .by_id
            .keys()
            .next()
            .expect("Caves should publish one interior");
        let interior_origin = selected
            .validated
            .plan
            .lights
            .values()
            .find_map(|light| match light.presentation {
                Some(PlannedLightPresentation::CaveCrystal(crystal))
                    if crystal.site == CaveCrystalSiteKind::InteriorAlcove =>
                {
                    Some(light.origin)
                }
                Some(PlannedLightPresentation::CaveCrystal(_)) | None => None,
            })
            .expect("reviewed Caves should contain an interior crystal alcove");
        let entrance_origin = selected
            .validated
            .plan
            .lights
            .values()
            .find_map(|light| match light.presentation {
                Some(PlannedLightPresentation::CaveCrystal(crystal))
                    if crystal.site == CaveCrystalSiteKind::EntranceLanding =>
                {
                    Some(light.origin)
                }
                Some(PlannedLightPresentation::CaveCrystal(_)) | None => None,
            })
            .expect("reviewed Caves should contain an entrance crystal landing");

        let mut missing_floor = selected.validated.plan.clone();
        let removed_floor = TilePos::new(
            interior_origin
                .coord
                .within_radius(1)
                .into_iter()
                .find(|coord| *coord != interior_origin.coord)
                .expect("a radius-one alcove has neighboring cells"),
            interior_origin.level,
        );
        missing_floor
            .interiors
            .by_id
            .get_mut(&region)
            .expect("Caves should retain its interior")
            .floors
            .remove(&removed_floor);
        let WorldValidation::Invalid(floor_issues) =
            validate_caves(&missing_floor, &caves_settings)
        else {
            panic!("a crystal alcove with missing interior membership must fail");
        };
        assert!(floor_issues
            .iter()
            .any(|issue| issue.detail.contains("flat radius-one crystal alcove")));

        let mut missing_roof = selected.validated.plan.clone();
        let roof = missing_roof
            .interiors
            .by_id
            .get(&region)
            .and_then(|interior| {
                interior
                    .roof_voxels
                    .iter()
                    .copied()
                    .find(|position| position.coord == interior_origin.coord)
            })
            .expect("an interior alcove should retain a roof");
        missing_roof
            .interiors
            .by_id
            .get_mut(&region)
            .expect("Caves should retain its interior")
            .roof_voxels
            .remove(&roof);
        let WorldValidation::Invalid(roof_issues) = validate_caves(&missing_roof, &caves_settings)
        else {
            panic!("a crystal alcove with missing roof membership must fail");
        };
        assert!(roof_issues
            .iter()
            .any(|issue| issue.detail.contains("floor and roof contract")));

        let mut occupied_visual = selected.validated.plan;
        occupied_visual
            .volume
            .columns
            .get_mut(&entrance_origin.coord)
            .expect("an entrance landing should retain its column")
            .elements
            .push(solid(
                entrance_origin.level.saturating_add(1),
                entrance_origin.level.saturating_add(2),
                SolidMaterialRole::Stone,
                None,
            ));
        let WorldValidation::Invalid(volume_issues) =
            validate_caves(&occupied_visual, &caves_settings)
        else {
            panic!("a crystal intersecting semantic occupancy must fail");
        };
        assert!(volume_issues
            .iter()
            .any(|issue| issue.detail.contains("visual envelope")));
    }

    #[test]
    fn fixed_seed_corpus_remains_valid_without_fallbacks() {
        for seed in [0, 1, 17, 33, 41, 42, 178, 445, 808, 2_026, 736_283_041] {
            let selected =
                generate(12, 0.4, &settings(), seed).expect("fixed Caves seed should generate");
            assert!(
                !selected.used_fallback,
                "seed {seed} used fallback: {:?}",
                selected.notes
            );
            assert!(selected.metrics.minimum_clearance >= CORRIDOR_CLEARANCE);
            assert!(selected.metrics.maximum_clearance >= CHAMBER_CLEARANCE);
            assert!(selected.metrics.optional_dark_floors > 0);
        }
    }

    #[test]
    fn reviewed_crystal_layouts_match_the_v3_goldens() {
        let expected: [(u64, Option<u8>, u64, u64); 4] = [
            (
                33,
                Some(1),
                18_085_428_821_256_931_804,
                16_715_970_756_191_823_297,
            ),
            (
                178,
                Some(4),
                15_578_452_451_260_576_352,
                15_852_936_182_682_831_065,
            ),
            (
                445,
                Some(5),
                13_372_267_763_965_264_527,
                1_525_721_581_912_758_674,
            ),
            (
                736_283_041,
                Some(2),
                11_318_601_618_626_144_040,
                16_767_723_275_272_645_400,
            ),
        ];
        let mut crystal_kinds = BTreeSet::new();
        let mut site_kinds = BTreeSet::new();
        for (seed, candidate, semantic_fingerprint, map_fingerprint) in expected {
            let selected =
                generate(12, 0.4, &settings(), seed).expect("reviewed Caves seed should generate");
            let build =
                super::super::build(12, 0.4, &settings(), seed, &palette(), &is_solid, None)
                    .expect("reviewed Caves seed should materialize");
            assert_eq!(selected.selected_candidate, candidate);
            assert_eq!(
                selected.validated.semantic_fingerprint,
                semantic_fingerprint
            );
            assert_eq!(build.report.map_fingerprint, map_fingerprint);
            for light in selected.validated.plan.lights.values() {
                let Some(PlannedLightPresentation::CaveCrystal(crystal)) = light.presentation
                else {
                    panic!("reviewed light must reserve a crystal presentation");
                };
                crystal_kinds.insert(crystal.kind);
                site_kinds.insert(crystal.site);
            }
        }
        assert_eq!(
            crystal_kinds,
            BTreeSet::from([
                CaveCrystalKind::LowCluster,
                CaveCrystalKind::Branched,
                CaveCrystalKind::Spire,
            ])
        );
        assert_eq!(
            site_kinds,
            BTreeSet::from([
                CaveCrystalSiteKind::InteriorAlcove,
                CaveCrystalSiteKind::EntranceLanding,
            ])
        );
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
            let warmup = super::super::build(
                radius,
                0.4,
                &settings(),
                u64::MAX,
                &palette,
                &is_solid,
                None,
            )
            .expect("warm-up Caves should build");
            std::hint::black_box(warmup);

            let mut samples = Vec::new();
            for seed in 0..12 {
                let started = std::time::Instant::now();
                let build =
                    super::super::build(radius, 0.4, &settings(), seed, &palette, &is_solid, None)
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
