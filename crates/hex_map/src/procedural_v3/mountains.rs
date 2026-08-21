//! Native V3 mountain geometry.
//!
//! Mountains occupy most of their patch with overlapping sharp peak masses. Their
//! outer skirts remain one-level walker terrain, while deliberately steep summit
//! cores are classified from the finished exact traversal graph. Two independently
//! routed, two-wide corridors provide a high pass and a lower bypass.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use hex_assets::RuntimeArtCatalog;
use hex_core::{HexCoord, Level, MapViewHint, SpecialMovementRegion, TilePos};

use super::composition::{compose_single_patch, GeneratedPatchPlan};
use super::layout::{resolve_layout, HexSide, PatchId, ResolvedLayoutPlan, ResolvedPort};
use super::liquid::{LiquidBodyId, LiquidBodyPlan, LiquidFlowState, LiquidNode, LiquidPlan};
use super::patch::{PatchBuildMode, PatchRecipeContext};
use super::routing::vertex_disjoint_paths;
use super::seam::{shape_walker_seams, validate_patch_walker_seams};
use super::seed::SeedStream;
use super::selection::{
    run_recipe, CandidateAttemptError, CandidateContext, FallbackContext, RepairOutcome, V3Recipe,
    ValidatedWorldSelection, WorldValidation,
};
use super::traversal::OrdinaryGraph;
use super::vegetation::{
    append_landform_vegetation, validate_landform_vegetation, LandformVegetationDomain,
    LandformVegetationSet,
};
use super::volume::{
    FillMaterialRole, LevelInterval, NonSolidFill, SolidMass, SolidMaterialRole, SurfaceAccess,
    SurfaceMetadata, VolumeColumn, VolumeElement, VolumePlan,
};
use super::world::{
    FeaturePlan, GeneratedWorldPlan, InteriorPlan, ProtectedFeatureRoute, StructurePlan,
    WorldIssueCode, WorldValidationIssue,
};
use super::V3GenerationError;
use crate::settings::{
    ProceduralV3Settings, V3EnvironmentSettings, V3LayoutSettings, V3MountainsSettings,
    V3RecipeSettings,
};

const PARTY_START: &str = "party_start";
const HOSTILE_START: &str = "hostile_start";
const CONFLICT_CENTER: &str = "conflict_center";
const HIGH_PASS: &str = "high_pass";
const LOWER_BYPASS: &str = "lower_bypass";
const LOW_BYPASS_ANCHOR: &str = "low_bypass";
const STREAM_SOURCE_OVERLOOK: &str = "stream_source_overlook";
const STREAM_FALL_OVERLOOK: &str = "stream_fall_overlook";
const MOUNTAIN_FALL_HEIGHT: Level = 3;
const STREAM_OVERLOOK_RADIUS: u32 = 3;
const MOUNTAIN_TREE_TARGET: usize = 2;

/// Deterministic measurements for one admitted Mountains plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MountainsMetrics {
    pub(crate) ordinary_surfaces: u32,
    pub(crate) special_surfaces: u32,
    pub(crate) mountain_surfaces: u32,
    pub(crate) mountain_coverage_percent: u32,
    pub(crate) accessible_mountain_surfaces: u32,
    pub(crate) reachable_elevation_levels: u32,
    pub(crate) relief: Level,
    pub(crate) peak_count: u8,
    pub(crate) cliff_edges: u32,
    pub(crate) high_pass_steps: u32,
    pub(crate) lower_bypass_steps: u32,
    pub(crate) tree_roots: u32,
}

#[derive(Debug)]
struct MountainsRecipe {
    level_height: f32,
    layout: ResolvedLayoutPlan,
    settings: V3MountainsSettings,
    vegetation: LandformVegetationSet,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct MountainStreams<'a> {
    orientation: SeedStream<'a>,
    peaks: SeedStream<'a>,
    trees: SeedStream<'a>,
}

#[derive(Debug)]
struct MountainStream {
    nodes: BTreeMap<TilePos, LiquidNode>,
    source: TilePos,
    fall: TilePos,
    peak: HexCoord,
}

/// Runs the common eight-candidate selector for one native V3 Mountains world.
pub(crate) fn generate(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV3Settings,
    seed: u64,
    catalog: &RuntimeArtCatalog,
) -> Result<ValidatedWorldSelection<MountainsMetrics>, V3GenerationError> {
    if !level_height.is_finite() || level_height <= 0.0 {
        return Err(V3GenerationError::RecipeContract(
            "Mountains level height must be positive and finite".to_owned(),
        ));
    }
    let mountain_settings = recipe_settings(settings)?;
    let layout = resolve_layout(grid_radius, settings)
        .map_err(|error| V3GenerationError::RecipeContract(error.to_string()))?;
    let vegetation =
        LandformVegetationSet::resolve(catalog, V3EnvironmentSettings::Frozen, "Mountains")
            .map_err(V3GenerationError::RecipeContract)?;
    run_recipe(
        &MountainsRecipe {
            level_height,
            layout,
            settings: mountain_settings.clone(),
            vegetation,
        },
        settings,
        grid_radius,
        seed,
    )
}

impl V3Recipe for MountainsRecipe {
    type Settings = ProceduralV3Settings;
    type Metrics = MountainsMetrics;
    type Score = (u32, u32, u32, u8);

    fn construct(
        &self,
        context: CandidateContext,
        _settings: &Self::Settings,
    ) -> Result<GeneratedWorldPlan, CandidateAttemptError> {
        let patch = PatchRecipeContext::resolve(&self.layout, PatchId(0))
            .map_err(CandidateAttemptError::Fatal)?;
        let fragment = construct_patch_with_objects(
            patch,
            &self.settings,
            self.level_height,
            PatchBuildMode::Candidate {
                world_seed: context.seed,
                candidate: context.candidate,
            },
            &self.vegetation,
        )
        .map_err(CandidateAttemptError::Rejected)?;
        compose_single_patch(self.layout.clone(), fragment).map_err(|error| {
            CandidateAttemptError::Fatal(V3GenerationError::RecipeContract(format!(
                "Mountains single-patch composition failed: {error:?}"
            )))
        })
    }

    fn validate(
        &self,
        _settings: &Self::Settings,
        plan: &GeneratedWorldPlan,
    ) -> WorldValidation<Self::Metrics> {
        validate_mountains(plan, &self.settings, &self.vegetation)
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
            metrics.mountain_coverage_percent.abs_diff(62),
            metrics.relief.abs_diff(self.settings.relief),
            u32::MAX.saturating_sub(metrics.cliff_edges),
            candidate,
        )
    }

    fn canonical_fallback(
        &self,
        context: FallbackContext,
        _settings: &Self::Settings,
    ) -> Result<GeneratedWorldPlan, V3GenerationError> {
        if context.grid_radius != self.layout.grid_radius {
            return Err(V3GenerationError::RecipeContract(
                "Mountains fallback radius disagrees with its resolved layout".to_owned(),
            ));
        }
        let patch = PatchRecipeContext::resolve(&self.layout, PatchId(0))?;
        let fragment = construct_patch_with_objects(
            patch,
            &self.settings,
            self.level_height,
            PatchBuildMode::CanonicalFallback,
            &self.vegetation,
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
                "Mountains fallback composition failed: {error:?}"
            ))
        })
    }
}

fn recipe_settings(
    settings: &ProceduralV3Settings,
) -> Result<&V3MountainsSettings, V3GenerationError> {
    let V3LayoutSettings::Single(patch) = &settings.layout else {
        return Err(V3GenerationError::RecipeUnavailable("Ring7"));
    };
    let V3RecipeSettings::Mountains(mountains) = &patch.recipe else {
        return Err(V3GenerationError::RecipeUnavailable(recipe_name(
            &patch.recipe,
        )));
    };
    if patch.environment != V3EnvironmentSettings::Frozen {
        return Err(V3GenerationError::RecipeContract(
            "Mountains requires the Frozen environment".to_owned(),
        ));
    }
    if !patch.overlays.is_empty() {
        return Err(V3GenerationError::RecipeContract(
            "Mountains overlays are not implemented yet".to_owned(),
        ));
    }
    Ok(mountains)
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
        V3RecipeSettings::Volcano(_) => "Volcano",
        V3RecipeSettings::DeepForest(_) => "DeepForest",
        V3RecipeSettings::Prairie(_) => "Prairie",
        V3RecipeSettings::ShallowSea(_) => "ShallowSea",
        V3RecipeSettings::Beach(_) => "Beach",
        V3RecipeSettings::Shore(_) => "Shore",
        V3RecipeSettings::DeepMountain(_) => "DeepMountain",
        V3RecipeSettings::CrystalAscent(_) => "CrystalAscent",
        V3RecipeSettings::DesertTransition(_) => "DesertTransition",
        V3RecipeSettings::DesertPlain(_) => "DesertPlain",
        V3RecipeSettings::Dunes(_) => "Dunes",
        V3RecipeSettings::Oasis(_) => "Oasis",
    }
}

pub(crate) fn construct_patch_with_catalog(
    patch: PatchRecipeContext<'_>,
    settings: &V3MountainsSettings,
    level_height: f32,
    mode: PatchBuildMode,
    catalog: &RuntimeArtCatalog,
) -> Result<GeneratedPatchPlan, Vec<WorldValidationIssue>> {
    let vegetation =
        LandformVegetationSet::resolve(catalog, V3EnvironmentSettings::Frozen, "Mountains")
            .map_err(|error| vec![recipe_issue(error)])?;
    construct_patch_with_objects(patch, settings, level_height, mode, &vegetation)
}

fn construct_patch_with_objects(
    patch: PatchRecipeContext<'_>,
    settings: &V3MountainsSettings,
    level_height: f32,
    mode: PatchBuildMode,
    vegetation: &LandformVegetationSet,
) -> Result<GeneratedPatchPlan, Vec<WorldValidationIssue>> {
    let streams = mode.seed_streams(&patch);
    construct_patch_with_streams(
        patch,
        settings,
        level_height,
        streams.map(|streams| MountainStreams {
            orientation: streams.stage("mountains.orientation"),
            peaks: streams.stage("mountains.peaks"),
            trees: streams.stage("mountains.vegetation.trees"),
        }),
        vegetation,
    )
}

fn construct_patch_with_streams(
    patch: PatchRecipeContext<'_>,
    settings: &V3MountainsSettings,
    level_height: f32,
    streams: Option<MountainStreams<'_>>,
    vegetation: &LandformVegetationSet,
) -> Result<GeneratedPatchPlan, Vec<WorldValidationIssue>> {
    let mask = patch.mask().clone();
    let orientation = streams.map_or(0, |streams| {
        u8::try_from(streams.orientation.sample(0) % 6).unwrap_or_default()
    });
    let landing_candidates = seam_landing_candidates(&patch);
    let closed_boundary = closed_boundary_coords(&patch, &landing_candidates);
    let route_mask: BTreeSet<_> = mask.difference(&closed_boundary).copied().collect();
    let liquid_feeder_authority = if patch.layout().kind == super::layout::LayoutKind::Ring19 {
        mountain_liquid_feeder_authority(&patch)?
    } else {
        BTreeSet::new()
    };
    let walker_seam_approaches = patch.walker_protected_approaches();
    let liquid_route_reservation = liquid_feeder_authority
        .iter()
        .flat_map(|coord| coord.within_radius(1))
        .filter(|coord| patch.mask().contains(coord) && !walker_seam_approaches.contains(coord))
        .collect::<BTreeSet<_>>();
    let route_control_mask = route_mask
        .difference(&liquid_route_reservation)
        .copied()
        .collect::<BTreeSet<_>>();
    let (party_coord, hostile_coord) = opposing_landings(&landing_candidates, orientation)?;
    let (high_control, bypass_control) =
        route_controls(&route_control_mask, party_coord, hostile_coord, orientation)?;
    let high_centerline = route_via(
        &route_mask,
        party_coord,
        high_control,
        hostile_coord,
        &liquid_route_reservation,
    )
    .ok_or_else(|| vec![recipe_issue("Mountains could not route its high pass")])?;
    let landing_overlap: usize = if patch.layout().kind.is_composite() {
        1
    } else {
        2
    };
    let high_interior: BTreeSet<_> = high_centerline
        .iter()
        .copied()
        .skip(landing_overlap)
        .take(
            high_centerline
                .len()
                .saturating_sub(landing_overlap.saturating_mul(2)),
        )
        .collect();
    let high_footprint = two_wide_footprint(&route_mask, &high_centerline, orientation, true);
    let bypass_controls = if patch.layout().kind.is_composite() {
        ordered_bypass_controls(
            &route_control_mask,
            party_coord,
            hostile_coord,
            orientation,
            high_control,
        )
    } else {
        vec![bypass_control]
    };
    let bypass_centerline = bypass_controls
        .into_iter()
        .filter(|control| !high_interior.contains(control))
        .find_map(|control| {
            let forbidden = high_interior
                .union(&liquid_route_reservation)
                .copied()
                .collect::<BTreeSet<_>>();
            route_via(&route_mask, party_coord, control, hostile_coord, &forbidden).filter(
                |route| {
                    route
                        .iter()
                        .filter(|coord| high_centerline.contains(coord))
                        .count()
                        <= 2
                },
            )
        })
        .ok_or_else(|| vec![recipe_issue("Mountains could not route its lower bypass")])?;
    let bypass_footprint = two_wide_footprint(&route_mask, &bypass_centerline, orientation, false);
    let route_cells: BTreeSet<_> = high_footprint.union(&bypass_footprint).copied().collect();
    if patch.layout().kind == super::layout::LayoutKind::Ring19
        && !route_cells.is_disjoint(&liquid_feeder_authority)
    {
        return Err(vec![recipe_issue(
            "Mountains protected walker routes overlap the complete liquid feeder authority",
        )]);
    }
    let seam_approaches = patch.protected_approaches();
    let seam_buffer = mask
        .iter()
        .copied()
        .filter(|coord| {
            seam_approaches
                .iter()
                .any(|approach| approach.distance(*coord) <= 3)
        })
        .collect::<BTreeSet<_>>();
    let excluded: BTreeSet<_> = route_cells.union(&seam_buffer).copied().collect();
    let peaks = select_peaks(
        &mask,
        usize::from(settings.peak_count),
        &excluded,
        streams.map(|streams| streams.peaks),
    )?;

    let mut surface_by_coord = BTreeMap::new();
    for coord in &mask {
        let rise = peaks
            .iter()
            .map(|peak| mountain_rise(*peak, *coord, settings.relief))
            .max()
            .unwrap_or_default();
        surface_by_coord.insert(*coord, settings.base_level.saturating_add(rise));
    }
    apply_route(
        &mut surface_by_coord,
        &bypass_centerline,
        &bypass_footprint,
        settings.base_level,
        4,
    );
    apply_route(
        &mut surface_by_coord,
        &high_centerline,
        &high_footprint,
        settings.base_level,
        settings.relief / 2,
    );
    if patch.layout().kind == super::layout::LayoutKind::Ring19 {
        reconcile_ring19_route_levels(
            &mut surface_by_coord,
            &high_centerline,
            &bypass_centerline,
            &route_cells,
            settings.base_level,
            settings.relief / 2,
            4,
        )
        .map_err(|error| vec![recipe_issue(error)])?;
    }
    let authored_surface_by_coord = surface_by_coord.clone();
    let seam_shape = shape_walker_seams(&patch, &mut surface_by_coord)?;
    for (coord, authored_level) in authored_surface_by_coord {
        let in_peak_core = peaks.iter().any(|peak| peak.distance(coord) <= 2);
        if in_peak_core && !seam_approaches.contains(&coord) && !route_cells.contains(&coord) {
            surface_by_coord.insert(coord, authored_level);
        }
    }
    if patch.layout().kind == super::layout::LayoutKind::Ring19 {
        fit_ring19_liquid_source_corridors(&patch, &mut surface_by_coord)?;
    }
    let mountain_streams = build_mountain_streams(
        &patch,
        &mut surface_by_coord,
        &peaks,
        &route_cells,
        settings.base_level,
        settings.relief,
    )?;
    let stream_nodes_by_coord = mountain_streams
        .iter()
        .enumerate()
        .flat_map(|(body, stream)| {
            stream
                .nodes
                .iter()
                .map(move |(position, node)| (position.coord, (body, *position, *node)))
        })
        .collect::<BTreeMap<_, _>>();

    let mut columns = BTreeMap::new();
    let mut surfaces = BTreeMap::new();
    for (coord, level) in &surface_by_coord {
        if let Some((_, position, node)) = stream_nodes_by_coord.get(coord).copied() {
            let crossing = route_cells.contains(coord).then_some(*level);
            let (column, bed, crossing) = mountain_stream_column(position, node, crossing);
            columns.insert(*coord, column);
            surfaces.insert(
                bed,
                SurfaceMetadata {
                    access: SurfaceAccess::NonStandable,
                    interior: None,
                },
            );
            if let Some(crossing) = crossing {
                surfaces.insert(
                    crossing,
                    SurfaceMetadata {
                        access: SurfaceAccess::Ordinary,
                        interior: None,
                    },
                );
            }
        } else {
            let exposed = is_exposed_stone(*coord, *level, &surface_by_coord, settings);
            columns.insert(*coord, mountain_column(*level, exposed));
            surfaces.insert(
                TilePos::new(*coord, *level),
                SurfaceMetadata {
                    access: SurfaceAccess::Ordinary,
                    interior: None,
                },
            );
        }
    }
    let mut volume = VolumePlan {
        mask: mask.clone(),
        columns,
        surfaces,
    };
    seam_shape.apply(&mut volume)?;
    let party = exact_position(&surface_by_coord, party_coord)?;
    let hostile = exact_position(&surface_by_coord, hostile_coord)?;
    let initial_graph = OrdinaryGraph::from_volume(&volume, None);
    let reachable = initial_graph.distances_from(party);
    if patch.layout().kind == super::layout::LayoutKind::Ring19 {
        let unreachable_route = route_cells
            .iter()
            .filter_map(|coord| {
                let position = exact_position(&surface_by_coord, *coord).ok()?;
                (!reachable.contains_key(&position)).then_some(position)
            })
            .collect::<Vec<_>>();
        if let Some(position) = unreachable_route.first() {
            let neighbors = position
                .coord
                .neighbors()
                .into_iter()
                .map(|coord| {
                    (
                        coord,
                        surface_by_coord.get(&coord).copied(),
                        high_centerline.contains(&coord),
                        bypass_centerline.contains(&coord),
                        high_footprint.contains(&coord),
                        bypass_footprint.contains(&coord),
                        stream_nodes_by_coord.contains_key(&coord),
                    )
                })
                .collect::<Vec<_>>();
            return Err(vec![recipe_issue(format!(
                "Mountains Ring19 construction leaves {} protected route cells outside the \
                 ordinary network; first \
                 {position:?} (high center {}, bypass center {}, high footprint {}, bypass \
                 footprint {}), neighbors {neighbors:?}",
                unreachable_route.len(),
                high_centerline.contains(&position.coord),
                bypass_centerline.contains(&position.coord),
                high_footprint.contains(&position.coord),
                bypass_footprint.contains(&position.coord),
            ))]);
        }
    }
    let special_region = SpecialMovementRegion(0);
    for (position, metadata) in &mut volume.surfaces {
        if metadata.access == SurfaceAccess::Ordinary && !reachable.contains_key(position) {
            metadata.access = SurfaceAccess::SpecialMovement(special_region);
        }
    }

    let high_route = route_membership(&high_centerline, &high_footprint, &surface_by_coord)?;
    let bypass_route = route_membership(&bypass_centerline, &bypass_footprint, &surface_by_coord)?;
    let conflict_coord = high_centerline
        .get(high_centerline.len() / 2)
        .copied()
        .ok_or_else(|| vec![recipe_issue("Mountains high pass has no midpoint")])?;
    let bypass_coord = bypass_centerline
        .get(bypass_centerline.len() / 2)
        .copied()
        .ok_or_else(|| vec![recipe_issue("Mountains lower bypass has no midpoint")])?;
    let mut anchors = BTreeMap::from([
        (PARTY_START.to_owned(), party),
        (HOSTILE_START.to_owned(), hostile),
        (
            CONFLICT_CENTER.to_owned(),
            exact_position(&surface_by_coord, conflict_coord)?,
        ),
        (
            HIGH_PASS.to_owned(),
            exact_position(&surface_by_coord, conflict_coord)?,
        ),
        (
            LOW_BYPASS_ANCHOR.to_owned(),
            exact_position(&surface_by_coord, bypass_coord)?,
        ),
    ]);
    if let Some(stream) = mountain_streams.first() {
        anchors.insert(
            STREAM_SOURCE_OVERLOOK.to_owned(),
            stream_overlook(&reachable, stream.source)?,
        );
        anchors.insert(
            STREAM_FALL_OVERLOOK.to_owned(),
            stream_overlook(&reachable, stream.fall)?,
        );
    }
    let mut features = FeaturePlan {
        by_id: BTreeMap::new(),
        protected_routes: BTreeMap::from([
            (HIGH_PASS.to_owned(), high_route),
            (LOWER_BYPASS.to_owned(), bypass_route),
        ]),
        clearings: BTreeMap::new(),
    };
    let ordinary_surfaces = volume
        .surfaces
        .iter()
        .filter_map(|(position, metadata)| {
            (metadata.access == SurfaceAccess::Ordinary).then_some((position.coord, *position))
        })
        .collect::<BTreeMap<_, _>>();
    let mut vegetation_reserved = mountain_streams
        .iter()
        .flat_map(|stream| stream.nodes.keys().map(|position| position.coord))
        .collect::<BTreeSet<_>>();
    vegetation_reserved.extend(
        features
            .protected_routes
            .values()
            .flat_map(|route| route.surfaces.iter().map(|surface| surface.coord))
            .chain(anchors.values().map(|anchor| anchor.coord))
            .chain(patch.protected_approaches()),
    );
    let tree_candidates = ordinary_surfaces
        .keys()
        .filter(|coord| !vegetation_reserved.contains(coord))
        .copied()
        .collect::<BTreeSet<_>>();
    let mut blockers = BTreeSet::new();
    append_landform_vegetation(
        "Mountains",
        vegetation,
        &ordinary_surfaces,
        &tree_candidates,
        &BTreeSet::new(),
        &vegetation_reserved,
        MOUNTAIN_TREE_TARGET,
        0,
        streams.map(|streams| streams.trees),
        None,
        &mut features,
        &mut blockers,
    )
    .map_err(|error| vec![recipe_issue(error)])?;
    let biome_regions = volume
        .surfaces
        .keys()
        .copied()
        .map(|surface| (surface, patch.biome_region()))
        .collect();
    let view_hint =
        mountain_view_hint(&mask, &surface_by_coord, patch.grid_radius(), level_height)?;

    let fragment = GeneratedPatchPlan {
        patch_id: patch.id,
        volume,
        liquids: LiquidPlan {
            bodies: mountain_streams
                .into_iter()
                .enumerate()
                .map(|(index, stream)| {
                    (
                        LiquidBodyId(u32::try_from(index).unwrap_or(u32::MAX)),
                        LiquidBodyPlan {
                            material: FillMaterialRole::Water,
                            nodes: stream.nodes,
                        },
                    )
                })
                .collect(),
        },
        features,
        structures: StructurePlan::default(),
        blockers,
        lights: BTreeMap::new(),
        biome_regions,
        interiors: InteriorPlan::default(),
        anchors,
        view_hint,
    };
    let mut issues = validate_patch_walker_seams(&patch, &fragment.volume);
    issues.extend(
        fragment
            .validate_against(patch.layout())
            .into_iter()
            .map(|issue| {
                recipe_issue(format!(
                    "Mountains patch {:?} failed {:?}: {}",
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

fn seam_landing_candidates(patch: &PatchRecipeContext<'_>) -> BTreeSet<HexCoord> {
    let protected = patch.walker_protected_approaches();
    if protected.is_empty() {
        patch.mask().clone()
    } else {
        protected
    }
}

fn closed_boundary_coords(
    patch: &PatchRecipeContext<'_>,
    open: &BTreeSet<HexCoord>,
) -> BTreeSet<HexCoord> {
    patch
        .shared_edges()
        .flat_map(|edge| edge.boundary_pairs().into_iter().map(|(local, _)| local))
        .filter(|coord| !open.contains(coord))
        .collect()
}

fn mountain_liquid_source_approaches(patch: &PatchRecipeContext<'_>) -> BTreeSet<HexCoord> {
    patch
        .shared_edges()
        .filter_map(|edge| edge.liquid_port())
        .filter(|liquid| liquid.is_source)
        .flat_map(|liquid| liquid.port.first_approach)
        .collect()
}

fn mountain_liquid_feeder_authority(
    patch: &PatchRecipeContext<'_>,
) -> Result<BTreeSet<HexCoord>, Vec<WorldValidationIssue>> {
    let exact = mountain_liquid_source_approaches(patch);
    let walker = patch.walker_protected_approaches();
    if !exact.is_disjoint(&walker) {
        return Err(vec![recipe_issue(
            "Mountains liquid source approach overlaps a walker seam approach",
        )]);
    }
    let frame = patch.local_frame().map_err(|error| {
        vec![recipe_issue(format!(
            "Mountains liquid feeder frame failed: {error}"
        ))]
    })?;
    let outlets = patch
        .shared_edges()
        .filter_map(|edge| edge.liquid_port())
        .filter(|liquid| liquid.is_source)
        .flat_map(|liquid| liquid.port.lanes.into_iter().map(|(local, _)| local))
        .collect::<BTreeSet<_>>();
    let maximum_depth = exact
        .iter()
        .map(|coord| {
            outlets
                .iter()
                .map(|outlet| coord.distance(*outlet))
                .min()
                .unwrap_or_default()
        })
        .max()
        .unwrap_or_default();
    let mut authority = exact.clone();
    for inward in exact.iter().copied().filter(|coord| {
        outlets
            .iter()
            .map(|outlet| coord.distance(*outlet))
            .min()
            .unwrap_or_default()
            == maximum_depth
    }) {
        authority.extend(inward.line_between(frame.center()));
    }
    Ok(authority
        .into_iter()
        .filter(|coord| patch.mask().contains(coord) && !walker.contains(coord))
        .collect())
}

fn opposing_landings(
    mask: &BTreeSet<HexCoord>,
    orientation: u8,
) -> Result<(HexCoord, HexCoord), Vec<WorldValidationIssue>> {
    let party = mask
        .iter()
        .copied()
        .min_by_key(|coord| (axis_value(*coord, orientation), *coord));
    let hostile = mask
        .iter()
        .copied()
        .max_by_key(|coord| (axis_value(*coord, orientation), *coord));
    match (party, hostile) {
        (Some(party), Some(hostile)) if party != hostile => Ok((party, hostile)),
        _ => Err(vec![recipe_issue(
            "Mountains patch cannot fit opposing landings",
        )]),
    }
}

fn route_controls(
    mask: &BTreeSet<HexCoord>,
    start: HexCoord,
    end: HexCoord,
    orientation: u8,
) -> Result<(HexCoord, HexCoord), Vec<WorldValidationIssue>> {
    let start_axis = axis_value(start, orientation);
    let end_axis = axis_value(end, orientation);
    let midpoint = start_axis.saturating_add(end_axis) / 2;
    let half_band = start_axis.abs_diff(end_axis).saturating_div(5).max(1);
    let side_turn = orientation.saturating_add(2) % 6;
    let candidates: Vec<_> = mask
        .iter()
        .copied()
        .filter(|coord| axis_value(*coord, orientation).abs_diff(midpoint) <= half_band)
        .filter(|coord| coord.distance(start) >= 3 && coord.distance(end) >= 3)
        .collect();
    let high = candidates
        .iter()
        .copied()
        .max_by_key(|coord| (axis_value(*coord, side_turn), *coord));
    let bypass = candidates
        .iter()
        .copied()
        .min_by_key(|coord| (axis_value(*coord, side_turn), *coord));
    match (high, bypass) {
        (Some(high), Some(bypass)) if high != bypass => Ok((high, bypass)),
        _ => Err(vec![recipe_issue(
            "Mountains patch cannot separate high-pass and bypass controls",
        )]),
    }
}

fn ordered_bypass_controls(
    mask: &BTreeSet<HexCoord>,
    start: HexCoord,
    end: HexCoord,
    orientation: u8,
    high_control: HexCoord,
) -> Vec<HexCoord> {
    let start_axis = axis_value(start, orientation);
    let end_axis = axis_value(end, orientation);
    let midpoint = start_axis.saturating_add(end_axis) / 2;
    let half_band = start_axis.abs_diff(end_axis).saturating_div(5).max(1);
    let side_turn = orientation.saturating_add(2) % 6;
    let mut candidates = mask
        .iter()
        .copied()
        .filter(|coord| axis_value(*coord, orientation).abs_diff(midpoint) <= half_band)
        .filter(|coord| coord.distance(start) >= 3 && coord.distance(end) >= 3)
        .filter(|coord| *coord != high_control)
        .collect::<Vec<_>>();
    candidates.sort_by_key(|coord| (axis_value(*coord, side_turn), *coord));
    candidates
}

fn route_via(
    mask: &BTreeSet<HexCoord>,
    start: HexCoord,
    control: HexCoord,
    end: HexCoord,
    forbidden: &BTreeSet<HexCoord>,
) -> Option<Vec<HexCoord>> {
    let first = shortest_path(mask, start, control, forbidden)?;
    let mut second_forbidden = forbidden.clone();
    second_forbidden.extend(first.iter().copied().filter(|coord| *coord != control));
    let second = shortest_path(mask, control, end, &second_forbidden)?;
    let mut combined = first;
    combined.extend(second.into_iter().skip(1));
    (combined.iter().copied().collect::<BTreeSet<_>>().len() == combined.len()).then_some(combined)
}

fn shortest_path(
    mask: &BTreeSet<HexCoord>,
    start: HexCoord,
    end: HexCoord,
    forbidden: &BTreeSet<HexCoord>,
) -> Option<Vec<HexCoord>> {
    if !mask.contains(&start) || !mask.contains(&end) {
        return None;
    }
    let mut previous = BTreeMap::<HexCoord, Option<HexCoord>>::from([(start, None)]);
    let mut frontier = VecDeque::from([start]);
    while let Some(current) = frontier.pop_front() {
        if current == end {
            break;
        }
        let mut neighbors = current.neighbors();
        neighbors.sort_by_key(|coord| (coord.distance(end), *coord));
        for neighbor in neighbors {
            if !mask.contains(&neighbor)
                || (forbidden.contains(&neighbor) && neighbor != end)
                || previous.contains_key(&neighbor)
            {
                continue;
            }
            previous.insert(neighbor, Some(current));
            frontier.push_back(neighbor);
        }
    }
    if !previous.contains_key(&end) {
        return None;
    }
    let mut reversed = vec![end];
    let mut current = end;
    while current != start {
        let predecessor = previous.get(&current).copied().flatten()?;
        reversed.push(predecessor);
        current = predecessor;
    }
    reversed.reverse();
    Some(reversed)
}

fn fit_ring19_liquid_source_corridors(
    patch: &PatchRecipeContext<'_>,
    levels: &mut BTreeMap<HexCoord, Level>,
) -> Result<(), Vec<WorldValidationIssue>> {
    for edge in patch.shared_edges() {
        let Some(liquid) = edge.liquid_port() else {
            continue;
        };
        if !liquid.is_source {
            continue;
        }
        let water_level = match liquid.elevation {
            super::layout::ResolvedLiquidElevation::EdgeBand => edge
                .preferred_level()
                .saturating_sub(1)
                .max(edge.contract.elevation.min)
                .min(edge.contract.elevation.max),
            super::layout::ResolvedLiquidElevation::Exact(level) => level,
        };
        let minimum_surface = water_level.saturating_add(1);
        let corridor = liquid
            .port
            .first_approach
            .iter()
            .copied()
            .chain(liquid.port.lanes.iter().map(|(local, _)| *local))
            .collect::<BTreeSet<_>>();
        for coord in corridor {
            let Some(surface) = levels.get_mut(&coord) else {
                return Err(vec![recipe_issue(format!(
                    "Mountains liquid source corridor leaves its patch at {coord:?}"
                ))]);
            };
            *surface = (*surface).max(minimum_surface);
        }
    }
    Ok(())
}

fn build_mountain_streams(
    patch: &PatchRecipeContext<'_>,
    levels: &mut BTreeMap<HexCoord, Level>,
    peaks: &[HexCoord],
    route_cells: &BTreeSet<HexCoord>,
    base_level: Level,
    relief: Level,
) -> Result<Vec<MountainStream>, Vec<WorldValidationIssue>> {
    let boundary = patch
        .mask()
        .iter()
        .copied()
        .filter(|coord| {
            coord
                .neighbors()
                .into_iter()
                .any(|neighbor| !patch.mask().contains(&neighbor))
        })
        .collect::<BTreeSet<_>>();
    if patch.layout().kind == super::layout::LayoutKind::Single {
        let mut outlets = boundary
            .iter()
            .copied()
            .filter(|coord| !route_cells.contains(coord))
            .collect::<Vec<_>>();
        outlets.sort_unstable_by_key(|coord| {
            (
                Reverse(
                    peaks
                        .iter()
                        .map(|peak| peak.distance(*coord))
                        .min()
                        .unwrap_or_default(),
                ),
                levels.get(coord).copied().unwrap_or(Level::MAX),
                *coord,
            )
        });
        let outlet_level = base_level.saturating_sub(1);
        for outlet in outlets.into_iter().take(12) {
            if let Some(stream) = build_mountain_stream(
                patch.mask(),
                &*levels,
                peaks,
                route_cells,
                &BTreeSet::new(),
                &boundary,
                outlet,
                outlet_level,
                base_level,
                relief,
                None,
                false,
            ) {
                return Ok(vec![stream]);
            }
        }
        return Err(vec![recipe_issue(
            "Mountains could not route a peak-fed stream to the world boundary",
        )]);
    }

    let mut outgoing = Vec::new();
    for edge in patch.shared_edges() {
        let Some(liquid) = edge.liquid_port() else {
            continue;
        };
        if !liquid.is_source {
            return Err(vec![recipe_issue(
                "Mountains supports source/outlet liquid contracts, not incoming liquid",
            )]);
        }
        let endpoint_level = match liquid.elevation {
            super::layout::ResolvedLiquidElevation::EdgeBand => edge
                .preferred_level()
                .saturating_sub(1)
                .max(edge.contract.elevation.min)
                .min(edge.contract.elevation.max),
            super::layout::ResolvedLiquidElevation::Exact(level) => level,
        };
        outgoing.push((edge.side, liquid.port, endpoint_level));
    }
    if outgoing.is_empty() {
        return Ok(Vec::new());
    }
    let [(side, port, endpoint_level)] = outgoing.as_slice() else {
        return Err(vec![recipe_issue(format!(
            "Mountains has {} directed liquid outlets; expected one coherent corridor",
            outgoing.len()
        ))]);
    };
    if patch.layout().kind == super::layout::LayoutKind::Ring19
        && *endpoint_level > base_level.saturating_add(4)
    {
        return build_joint_ring19_mountain_streams(
            patch,
            levels,
            peaks,
            route_cells,
            *side,
            port,
            *endpoint_level,
            base_level,
            relief,
        )
        .map_err(|error| vec![recipe_issue(error)]);
    }
    let mut outlets = port
        .lanes
        .iter()
        .map(|(local, _)| (*local, *endpoint_level))
        .collect::<Vec<_>>();
    if patch.layout().kind == super::layout::LayoutKind::Ring19 {
        let coordinates = outlets.iter().map(|(coord, _)| *coord).collect::<Vec<_>>();
        outlets.sort_unstable_by_key(|(coord, _)| {
            (
                coordinates
                    .iter()
                    .map(|other| coord.distance(*other))
                    .sum::<u32>(),
                *coord,
            )
        });
    } else {
        outlets.sort_unstable();
    }
    let reserved_outlets = outlets
        .iter()
        .map(|(coord, _)| *coord)
        .collect::<BTreeSet<_>>();
    let mut used = BTreeSet::new();
    let mut streams = Vec::new();
    for (outlet, outlet_level) in outlets {
        let other_outlets = reserved_outlets
            .iter()
            .copied()
            .filter(|coord| *coord != outlet)
            .collect::<BTreeSet<_>>();
        let mut reserved = used.union(&other_outlets).copied().collect::<BTreeSet<_>>();
        if patch.layout().kind == super::layout::LayoutKind::Ring19 {
            reserved.extend(patch.walker_protected_approaches());
            reserved.extend(boundary.iter().copied());
            reserved.remove(&outlet);
        }
        let mut stream = build_mountain_stream(
            patch.mask(),
            &*levels,
            peaks,
            route_cells,
            &reserved,
            &boundary,
            outlet,
            outlet_level,
            base_level,
            relief,
            streams.first().map(|stream: &MountainStream| stream.peak),
            patch.layout().kind == super::layout::LayoutKind::Ring19,
        );
        let mut fitted_failure = None;
        if stream.is_none() && patch.layout().kind == super::layout::LayoutKind::Ring19 {
            match build_fitted_ring19_mountain_stream(
                patch,
                levels,
                peaks,
                route_cells,
                &reserved,
                &boundary,
                outlet,
                outlet_level,
                base_level,
                relief,
                streams.first().map(|stream: &MountainStream| stream.peak),
            ) {
                Ok(fitted) => stream = Some(fitted),
                Err(error) => fitted_failure = Some(error),
            }
        }
        let Some(stream) = stream else {
            let approach_levels = port
                .first_approach
                .iter()
                .map(|coord| (*coord, levels.get(coord).copied()))
                .collect::<Vec<_>>();
            let planned = streams
                .iter()
                .map(|stream: &MountainStream| {
                    (
                        stream.source,
                        stream.peak,
                        stream.nodes.len(),
                        stream
                            .nodes
                            .keys()
                            .map(|position| position.coord)
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>();
            return Err(vec![recipe_issue(format!(
                "Mountains could not route a peak-fed stream to liquid outlet {outlet:?} at \
                 level {outlet_level} below surface {:?}; approach levels \
                 {approach_levels:?}; fitted routing {:?}; planned {planned:?}",
                levels.get(&outlet),
                fitted_failure,
            ))]);
        };
        used.extend(stream.nodes.keys().map(|position| position.coord));
        streams.push(stream);
    }
    Ok(streams)
}

#[expect(
    clippy::too_many_arguments,
    reason = "the Ring19 joint solve keeps each immutable seam and terrain contract explicit"
)]
fn build_joint_ring19_mountain_streams(
    patch: &PatchRecipeContext<'_>,
    levels: &mut BTreeMap<HexCoord, Level>,
    peaks: &[HexCoord],
    route_cells: &BTreeSet<HexCoord>,
    side: HexSide,
    port: &ResolvedPort,
    outlet_level: Level,
    base_level: Level,
    relief: Level,
) -> Result<Vec<MountainStream>, String> {
    let tails = ring19_mountain_approach_tails(side, port).ok_or_else(|| {
        "Ring19 Mountains could not partition its exact liquid approach into lane tails".to_owned()
    })?;
    let terminals = tails.keys().copied().collect::<BTreeSet<_>>();
    let walker_protected = patch.walker_protected_approaches();
    if !route_cells.is_disjoint(&port.first_approach)
        || !walker_protected.is_disjoint(&port.first_approach)
    {
        return Err(
            "Ring19 Mountains liquid approach overlaps protected walker terrain".to_owned(),
        );
    }

    let boundary = patch
        .mask()
        .iter()
        .copied()
        .filter(|coord| {
            coord
                .neighbors()
                .into_iter()
                .any(|neighbor| !patch.mask().contains(&neighbor))
        })
        .collect::<BTreeSet<_>>();
    let mut forbidden = route_cells
        .union(&walker_protected)
        .copied()
        .chain(boundary.iter().copied())
        .chain(peaks.iter().copied())
        .chain(port.first_approach.iter().copied())
        .collect::<BTreeSet<_>>();
    for terminal in &terminals {
        forbidden.remove(terminal);
    }

    let source_minimum = base_level.saturating_add(relief / 3);
    let source_land_minimum = base_level.saturating_add(relief / 2);
    let mut ordered_peaks = peaks.to_vec();
    ordered_peaks.sort_unstable_by_key(|peak| {
        (
            terminals
                .iter()
                .map(|terminal| peak.distance(*terminal))
                .min()
                .unwrap_or(u32::MAX),
            *peak,
        )
    });

    let mut attempted_groups = 0_usize;
    for peak in ordered_peaks {
        let mut candidates = levels
            .iter()
            .filter_map(|(coord, level)| {
                let nearest = peaks
                    .iter()
                    .copied()
                    .min_by_key(|candidate| (candidate.distance(*coord), *candidate));
                (!forbidden.contains(coord)
                    && nearest == Some(peak)
                    && (1..=2).contains(&peak.distance(*coord))
                    && *level >= source_land_minimum)
                    .then_some(*coord)
            })
            .collect::<Vec<_>>();
        candidates.sort_unstable_by_key(|coord| {
            (
                terminals
                    .iter()
                    .map(|terminal| coord.distance(*terminal))
                    .min()
                    .unwrap_or(u32::MAX),
                Reverse(levels.get(coord).copied().unwrap_or_default()),
                *coord,
            )
        });
        let mut source_groups = connected_source_triples(&candidates);
        source_groups.sort_by_key(|sources| {
            (
                sources
                    .iter()
                    .map(|source| {
                        terminals
                            .iter()
                            .map(|terminal| source.distance(*terminal))
                            .min()
                            .unwrap_or(u32::MAX)
                    })
                    .sum::<u32>(),
                sources
                    .iter()
                    .map(|source| Reverse(levels.get(source).copied().unwrap_or_default()))
                    .collect::<Vec<_>>(),
                sources.clone(),
            )
        });

        'source_groups: for sources in source_groups {
            attempted_groups = attempted_groups.saturating_add(1);
            let Some(ingress_paths) =
                vertex_disjoint_mountain_paths(patch.mask(), &forbidden, &sources, &terminals)
            else {
                continue;
            };
            let mut full_paths = Vec::new();
            for mut path in ingress_paths {
                let Some(terminal) = path.last().copied() else {
                    continue 'source_groups;
                };
                let Some(tail) = tails.get(&terminal) else {
                    continue 'source_groups;
                };
                path.extend(tail.iter().copied().skip(1));
                full_paths.push(path);
            }
            full_paths.sort_by_key(|path| path.last().copied());
            let corridor = full_paths
                .iter()
                .flat_map(|path| path.iter().copied())
                .collect::<BTreeSet<_>>();
            if full_paths.len() != tails.len()
                || !corridor.is_disjoint(route_cells)
                || !horizontal_coords_connected(&corridor)
            {
                continue;
            }

            let mut fitted = Vec::new();
            for path in full_paths {
                let Some(water_levels) =
                    fitted_mountain_water_profile(&path, outlet_level, source_minimum)
                else {
                    continue 'source_groups;
                };
                let Some(stream) = mountain_stream_from_profile(&path, &water_levels, peak) else {
                    continue 'source_groups;
                };
                fitted.push((path, water_levels, stream));
            }
            if fitted.len() != tails.len()
                || fitted
                    .iter()
                    .flat_map(|(path, _, _)| path)
                    .any(|coord| !levels.contains_key(coord))
            {
                continue;
            }
            for (path, water_levels, _) in &fitted {
                for (coord, water_level) in path.iter().copied().zip(water_levels) {
                    let Some(surface) = levels.get_mut(&coord) else {
                        return Err(format!(
                            "Ring19 Mountains joint stream leaves its patch at {coord:?}"
                        ));
                    };
                    *surface = (*surface).max(water_level.saturating_add(1));
                }
            }
            return Ok(fitted.into_iter().map(|(_, _, stream)| stream).collect());
        }
    }

    Err(format!(
        "Ring19 Mountains could not jointly route {} liquid lanes from one summit after \
         {attempted_groups} connected source-group attempts",
        tails.len()
    ))
}

fn ring19_mountain_approach_tails(
    side: HexSide,
    port: &ResolvedPort,
) -> Option<BTreeMap<HexCoord, Vec<HexCoord>>> {
    let inward = side.opposite();
    let mut covered = BTreeSet::new();
    let mut tails = BTreeMap::new();
    for (boundary, _) in &port.lanes {
        if !port.first_approach.contains(boundary) {
            return None;
        }
        let mut outward_to_inward = Vec::new();
        let mut current = *boundary;
        loop {
            if !port.first_approach.contains(&current) {
                break;
            }
            if !covered.insert(current) {
                return None;
            }
            outward_to_inward.push(current);
            current = inward.neighbor(current);
        }
        outward_to_inward.reverse();
        let terminal = outward_to_inward.first().copied()?;
        if tails.insert(terminal, outward_to_inward).is_some() {
            return None;
        }
    }
    (covered == port.first_approach).then_some(tails)
}

fn connected_source_triples(candidates: &[HexCoord]) -> Vec<BTreeSet<HexCoord>> {
    let mut triples = Vec::new();
    for (first_index, first) in candidates.iter().copied().enumerate() {
        for (second_index, second) in candidates
            .iter()
            .copied()
            .enumerate()
            .skip(first_index.saturating_add(1))
        {
            for third in candidates
                .iter()
                .copied()
                .skip(second_index.saturating_add(1))
            {
                let sources = BTreeSet::from([first, second, third]);
                if horizontal_coords_connected(&sources) {
                    triples.push(sources);
                }
            }
        }
    }
    triples
}

fn vertex_disjoint_mountain_paths(
    mask: &BTreeSet<HexCoord>,
    forbidden: &BTreeSet<HexCoord>,
    sources: &BTreeSet<HexCoord>,
    terminals: &BTreeSet<HexCoord>,
) -> Option<Vec<Vec<HexCoord>>> {
    if sources.len() != terminals.len()
        || !sources.is_disjoint(terminals)
        || sources
            .iter()
            .chain(terminals)
            .any(|coord| !mask.contains(coord) || forbidden.contains(coord))
    {
        return None;
    }
    let allowed = mask.difference(forbidden).copied().collect::<BTreeSet<_>>();
    let starts = sources.iter().copied().collect::<Vec<_>>();
    let targets = terminals.iter().copied().collect::<Vec<_>>();
    let mut paths = vertex_disjoint_paths(&allowed, &starts, &targets)?;
    paths.sort_by_key(|path| path.last().copied());
    Some(paths)
}

fn mountain_stream_from_profile(
    path: &[HexCoord],
    water_levels: &[Level],
    peak: HexCoord,
) -> Option<MountainStream> {
    if path.len() != water_levels.len() {
        return None;
    }
    let positions = path
        .iter()
        .copied()
        .zip(water_levels.iter().copied())
        .map(|(coord, level)| TilePos::new(coord, level))
        .collect::<Vec<_>>();
    let nodes = positions
        .iter()
        .copied()
        .enumerate()
        .map(|(index, position)| {
            let downstream = positions.get(index.saturating_add(1)).copied();
            let state = downstream.map_or(LiquidFlowState::Still, |downstream| {
                match position.level.saturating_sub(downstream.level) {
                    0 => LiquidFlowState::Current,
                    1 => LiquidFlowState::Rapid,
                    _ => LiquidFlowState::Fall,
                }
            });
            (position, LiquidNode { state, downstream })
        })
        .collect::<BTreeMap<_, _>>();
    let source = positions.first().copied()?;
    let fall = positions.windows(2).find_map(|pair| {
        let [from, to] = pair else {
            return None;
        };
        (from.level.saturating_sub(to.level) == MOUNTAIN_FALL_HEIGHT).then_some(*from)
    })?;
    Some(MountainStream {
        nodes,
        source,
        fall,
        peak,
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "Ring19 source fitting keeps every immutable routing contract explicit"
)]
fn build_fitted_ring19_mountain_stream(
    patch: &PatchRecipeContext<'_>,
    levels: &mut BTreeMap<HexCoord, Level>,
    peaks: &[HexCoord],
    route_cells: &BTreeSet<HexCoord>,
    reserved: &BTreeSet<HexCoord>,
    boundary: &BTreeSet<HexCoord>,
    outlet: HexCoord,
    outlet_level: Level,
    base_level: Level,
    relief: Level,
    required_peak: Option<HexCoord>,
) -> Result<MountainStream, String> {
    let source_minimum = base_level.saturating_add(relief / 3);
    let source_land_minimum = base_level.saturating_add(relief / 2);
    let minimum_edges = source_minimum
        .saturating_sub(outlet_level)
        .saturating_sub(MOUNTAIN_FALL_HEIGHT)
        .max(0)
        .saturating_add(1);
    let minimum_edges = u32::try_from(minimum_edges).unwrap_or(u32::MAX);
    let walker_protected = patch.walker_protected_approaches();
    let feeder_line = patch
        .local_frame()
        .map_err(|error| format!("cannot resolve Ring19 feeder frame: {error}"))?
        .center()
        .line_between(outlet);
    let mut sources = levels
        .iter()
        .filter_map(|(coord, level)| {
            let peak = peaks
                .iter()
                .copied()
                .min_by_key(|peak| (peak.distance(*coord), *peak))?;
            let peak_distance = peak.distance(*coord);
            (!route_cells.contains(coord)
                && !reserved.contains(coord)
                && !boundary.contains(coord)
                && !walker_protected.contains(coord)
                && !peaks.contains(coord)
                && required_peak.is_none_or(|required| peak == required)
                && (1..=2).contains(&peak_distance)
                && *level >= source_land_minimum
                && coord.distance(outlet) >= minimum_edges)
                .then_some((*coord, *level, peak, peak_distance))
        })
        .collect::<Vec<_>>();
    sources.sort_unstable_by_key(|(coord, level, peak, peak_distance)| {
        (
            feeder_line
                .iter()
                .map(|feeder| coord.distance(*feeder))
                .min()
                .unwrap_or_default(),
            *peak_distance,
            Reverse(*level),
            u32::MAX.saturating_sub(coord.distance(outlet)),
            *peak,
            *coord,
        )
    });
    let source_count = sources.len();

    let low_route_cells = route_cells
        .iter()
        .copied()
        .filter(|coord| levels.get(coord).is_none_or(|level| *level <= outlet_level))
        .collect::<BTreeSet<_>>();
    let mut forbidden = reserved
        .union(&low_route_cells)
        .copied()
        .chain(boundary.iter().copied())
        .chain(walker_protected.iter().copied())
        .chain(peaks.iter().copied())
        .collect::<BTreeSet<_>>();
    forbidden.remove(&outlet);
    let mut path_count = 0_usize;
    let mut profile_count = 0_usize;
    for (source, _land_level, peak, _peak_distance) in sources.into_iter().take(24) {
        let Some(path) = shortest_path(patch.mask(), source, outlet, &forbidden) else {
            continue;
        };
        path_count = path_count.saturating_add(1);
        let Some(water_levels) = fitted_mountain_water_profile_around_routes(
            &path,
            levels,
            route_cells,
            outlet_level,
            source_minimum,
        ) else {
            continue;
        };
        if path
            .iter()
            .copied()
            .zip(&water_levels)
            .any(|(coord, water_level)| {
                route_cells.contains(&coord)
                    && levels
                        .get(&coord)
                        .is_none_or(|surface| *surface <= *water_level)
            })
        {
            continue;
        }
        profile_count = profile_count.saturating_add(1);
        for (coord, water_level) in path.iter().copied().zip(&water_levels) {
            let Some(surface) = levels.get_mut(&coord) else {
                return Err(format!(
                    "selected source route leaves the surface plan at {coord:?}"
                ));
            };
            *surface = (*surface).max(water_level.saturating_add(1));
        }
        let positions = path
            .iter()
            .copied()
            .zip(water_levels)
            .map(|(coord, level)| TilePos::new(coord, level))
            .collect::<Vec<_>>();
        let nodes = positions
            .iter()
            .copied()
            .enumerate()
            .map(|(index, position)| {
                let downstream = positions.get(index.saturating_add(1)).copied();
                let state = downstream.map_or(LiquidFlowState::Still, |downstream| match position
                    .level
                    .saturating_sub(downstream.level)
                {
                    0 => LiquidFlowState::Current,
                    1 => LiquidFlowState::Rapid,
                    _ => LiquidFlowState::Fall,
                });
                (position, LiquidNode { state, downstream })
            })
            .collect::<BTreeMap<_, _>>();
        let Some(source) = positions.first().copied() else {
            continue;
        };
        let Some(fall) = positions.windows(2).find_map(|pair| {
            let [from, to] = pair else {
                return None;
            };
            (from.level.saturating_sub(to.level) == MOUNTAIN_FALL_HEIGHT).then_some(*from)
        }) else {
            continue;
        };
        return Ok(MountainStream {
            nodes,
            source,
            fall,
            peak,
        });
    }
    Err(format!(
        "no fitted path from {source_count} eligible near-peak sources (paths {path_count}, \
         profiles {profile_count}, forbidden {}, boundary {}, low routes {}/{})",
        forbidden.len(),
        boundary.len(),
        low_route_cells.len(),
        route_cells.len()
    ))
}

fn fitted_mountain_water_profile(
    path: &[HexCoord],
    outlet_level: Level,
    source_minimum: Level,
) -> Option<Vec<Level>> {
    let edges = path.len().checked_sub(1)?;
    let source_level = source_minimum.max(outlet_level.saturating_add(MOUNTAIN_FALL_HEIGHT));
    let gradual_drop = source_level
        .saturating_sub(outlet_level)
        .saturating_sub(MOUNTAIN_FALL_HEIGHT);
    let gradual_drop = usize::try_from(gradual_drop).ok()?;
    if edges < gradual_drop.saturating_add(1) {
        return None;
    }
    let fall_edge = edges / 2;
    let mut drops = vec![0; edges];
    *drops.get_mut(fall_edge)? = MOUNTAIN_FALL_HEIGHT;
    let mut remaining = gradual_drop;
    for index in (0..edges).rev() {
        if index == fall_edge || remaining == 0 {
            continue;
        }
        *drops.get_mut(index)? = 1;
        remaining = remaining.saturating_sub(1);
    }
    if remaining != 0 {
        return None;
    }
    let mut levels = vec![source_level];
    for drop in drops {
        levels.push(levels.last().copied()?.saturating_sub(drop));
    }
    (levels.last().copied() == Some(outlet_level)).then_some(levels)
}

fn fitted_mountain_water_profile_around_routes(
    path: &[HexCoord],
    surface_levels: &BTreeMap<HexCoord, Level>,
    route_cells: &BTreeSet<HexCoord>,
    outlet_level: Level,
    source_minimum: Level,
) -> Option<Vec<Level>> {
    let outlet = *path.last()?;
    if outlet_level > surface_levels.get(&outlet)?.saturating_sub(1) {
        return None;
    }

    let mut profiles = BTreeMap::<(Level, Option<usize>), Vec<Level>>::from([(
        (outlet_level, None),
        vec![outlet_level],
    )]);
    for coord in path.iter().rev().skip(1) {
        let cap = if route_cells.contains(coord) {
            surface_levels.get(coord)?.saturating_sub(1)
        } else {
            source_minimum
        };
        let mut upstream = BTreeMap::<(Level, Option<usize>), Vec<Level>>::new();
        for ((downstream_level, fall_index), tail) in profiles {
            for drop in [0, 1, MOUNTAIN_FALL_HEIGHT] {
                if drop == MOUNTAIN_FALL_HEIGHT && fall_index.is_some() {
                    continue;
                }
                let level = downstream_level.saturating_add(drop);
                if level > cap {
                    continue;
                }
                let mut profile = Vec::with_capacity(tail.len().saturating_add(1));
                profile.push(level);
                profile.extend(tail.iter().copied());
                let fall_index = if drop == MOUNTAIN_FALL_HEIGHT {
                    Some(0)
                } else {
                    fall_index.map(|index| index.saturating_add(1))
                };
                let key = (level, fall_index);
                match upstream.entry(key) {
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        entry.insert(profile);
                    }
                    std::collections::btree_map::Entry::Occupied(mut entry) => {
                        if profile < *entry.get() {
                            entry.insert(profile);
                        }
                    }
                }
            }
        }
        profiles = upstream;
        if profiles.is_empty() {
            return None;
        }
    }
    let midpoint = path.len().saturating_sub(1) / 2;
    profiles
        .into_iter()
        .filter_map(|((source, fall_index), profile)| {
            let fall_index = fall_index?;
            (source >= source_minimum
                && path
                    .get(fall_index)
                    .zip(path.get(fall_index.saturating_add(1)))
                    .is_some_and(|(fall_source, landing)| {
                        !route_cells.contains(fall_source) && !route_cells.contains(landing)
                    }))
            .then_some(profile)
        })
        .min_by_key(|profile| {
            (
                profile_fall_index(profile).abs_diff(midpoint),
                profile.clone(),
            )
        })
}

#[expect(
    clippy::too_many_arguments,
    reason = "stream routing keeps each immutable geometry contract explicit"
)]
fn build_mountain_stream(
    mask: &BTreeSet<HexCoord>,
    levels: &BTreeMap<HexCoord, Level>,
    peaks: &[HexCoord],
    route_cells: &BTreeSet<HexCoord>,
    reserved: &BTreeSet<HexCoord>,
    boundary: &BTreeSet<HexCoord>,
    outlet: HexCoord,
    outlet_level: Level,
    base_level: Level,
    relief: Level,
    required_peak: Option<HexCoord>,
    search_feasible_routes: bool,
) -> Option<MountainStream> {
    let source_minimum = base_level.saturating_add(relief / 3);
    let source_land_minimum = base_level.saturating_add(relief / 2);
    let minimum_edges = source_minimum
        .saturating_sub(outlet_level)
        .saturating_sub(MOUNTAIN_FALL_HEIGHT)
        .max(0)
        .saturating_add(1);
    let minimum_edges = u32::try_from(minimum_edges).unwrap_or(u32::MAX);
    let mut sources = levels
        .iter()
        .filter_map(|(coord, level)| {
            let peak = peaks
                .iter()
                .copied()
                .min_by_key(|peak| (peak.distance(*coord), *peak))?;
            let peak_distance = peak.distance(*coord);
            (!route_cells.contains(coord)
                && !reserved.contains(coord)
                && !boundary.contains(coord)
                && !peaks.contains(coord)
                && required_peak.is_none_or(|required| peak == required)
                && (1..=2).contains(&peak_distance)
                && *level >= source_land_minimum
                && coord.distance(outlet) >= minimum_edges)
                .then_some((*coord, *level, peak, peak_distance))
        })
        .collect::<Vec<_>>();
    sources.sort_unstable_by_key(|(coord, level, peak, peak_distance)| {
        (
            *peak_distance,
            Reverse(*level),
            u32::MAX.saturating_sub(coord.distance(outlet)),
            *peak,
            *coord,
        )
    });
    let mut forbidden = reserved.clone();
    forbidden.extend(peaks.iter().copied());
    forbidden.remove(&outlet);
    for (source, _land_level, peak, _peak_distance) in sources.into_iter().take(24) {
        let routed = shortest_path(mask, source, outlet, &forbidden)
            .and_then(|path| {
                mountain_water_profile(&path, levels, outlet_level, source_minimum)
                    .map(|water_levels| (path, water_levels))
            })
            .or_else(|| {
                search_feasible_routes.then(|| {
                    feasible_shortest_mountain_route(
                        mask,
                        levels,
                        source,
                        outlet,
                        &forbidden,
                        outlet_level,
                        source_minimum,
                    )
                })?
            });
        let Some((path, water_levels)) = routed else {
            continue;
        };
        let positions = path
            .iter()
            .copied()
            .zip(water_levels)
            .map(|(coord, level)| TilePos::new(coord, level))
            .collect::<Vec<_>>();
        let nodes = positions
            .iter()
            .copied()
            .enumerate()
            .map(|(index, position)| {
                let downstream = positions.get(index.saturating_add(1)).copied();
                let state = downstream.map_or(LiquidFlowState::Still, |downstream| match position
                    .level
                    .saturating_sub(downstream.level)
                {
                    0 => LiquidFlowState::Current,
                    1 => LiquidFlowState::Rapid,
                    _ => LiquidFlowState::Fall,
                });
                (position, LiquidNode { state, downstream })
            })
            .collect::<BTreeMap<_, _>>();
        let source = positions.first().copied()?;
        let fall = positions.windows(2).find_map(|pair| {
            let [from, to] = pair else {
                return None;
            };
            (from.level.saturating_sub(to.level) == MOUNTAIN_FALL_HEIGHT).then_some(*from)
        })?;
        return Some(MountainStream {
            nodes,
            source,
            fall,
            peak,
        });
    }
    None
}

fn mountain_water_profile(
    path: &[HexCoord],
    levels: &BTreeMap<HexCoord, Level>,
    outlet_level: Level,
    source_minimum: Level,
) -> Option<Vec<Level>> {
    let outlet = *path.last()?;
    let outlet_cap = levels.get(&outlet)?.saturating_sub(1);
    if outlet_level > outlet_cap {
        return None;
    }
    let mut profiles =
        BTreeMap::<(Level, bool), Vec<Level>>::from([((outlet_level, false), vec![outlet_level])]);
    for coord in path.iter().rev().skip(1) {
        let cap = levels.get(coord)?.saturating_sub(1);
        let mut upstream = BTreeMap::<(Level, bool), Vec<Level>>::new();
        for ((downstream_level, used_fall), tail) in profiles {
            for drop in [0, 1, MOUNTAIN_FALL_HEIGHT] {
                if drop == MOUNTAIN_FALL_HEIGHT && used_fall {
                    continue;
                }
                let level = downstream_level.saturating_add(drop);
                if level > cap {
                    continue;
                }
                let mut profile = Vec::with_capacity(tail.len().saturating_add(1));
                profile.push(level);
                profile.extend(tail.iter().copied());
                let key = (level, used_fall || drop == MOUNTAIN_FALL_HEIGHT);
                match upstream.entry(key) {
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        entry.insert(profile);
                    }
                    std::collections::btree_map::Entry::Occupied(mut entry) => {
                        if profile_fall_index(&profile) < profile_fall_index(entry.get())
                            || (profile_fall_index(&profile) == profile_fall_index(entry.get())
                                && profile < *entry.get())
                        {
                            entry.insert(profile);
                        }
                    }
                }
            }
        }
        profiles = upstream;
        if profiles.is_empty() {
            return None;
        }
    }
    profiles
        .into_iter()
        .filter_map(|((source, used_fall), profile)| {
            (used_fall && source >= source_minimum).then_some(profile)
        })
        .min_by_key(|profile| {
            (
                Reverse(profile.first().copied().unwrap_or_default()),
                profile_fall_index(profile),
                profile.clone(),
            )
        })
}

fn feasible_shortest_mountain_route(
    mask: &BTreeSet<HexCoord>,
    levels: &BTreeMap<HexCoord, Level>,
    source: HexCoord,
    outlet: HexCoord,
    forbidden: &BTreeSet<HexCoord>,
    outlet_level: Level,
    source_minimum: Level,
) -> Option<(Vec<HexCoord>, Vec<Level>)> {
    let mut distances = BTreeMap::from([(outlet, 0_u32)]);
    let mut frontier = VecDeque::from([outlet]);
    while let Some(current) = frontier.pop_front() {
        let next_distance = distances.get(&current).copied()?.saturating_add(1);
        for neighbor in current.neighbors() {
            if !mask.contains(&neighbor)
                || (forbidden.contains(&neighbor) && neighbor != source)
                || distances.contains_key(&neighbor)
            {
                continue;
            }
            distances.insert(neighbor, next_distance);
            frontier.push_back(neighbor);
        }
    }
    let source_distance = distances.get(&source).copied()?;
    let outlet_cap = levels.get(&outlet)?.saturating_sub(1);
    if outlet_level > outlet_cap {
        return None;
    }

    type WaterState = (Level, bool);
    let mut states = BTreeMap::<HexCoord, BTreeSet<WaterState>>::from([(
        outlet,
        BTreeSet::from([(outlet_level, false)]),
    )]);
    let mut downstream = BTreeMap::<(HexCoord, Level, bool), (HexCoord, Level, bool)>::new();
    for distance in 1..=source_distance {
        let coordinates = distances
            .iter()
            .filter_map(|(coord, candidate_distance)| {
                (*candidate_distance == distance).then_some(*coord)
            })
            .collect::<Vec<_>>();
        for coord in coordinates {
            let cap = levels.get(&coord)?.saturating_sub(1);
            for next in coord.neighbors().into_iter().filter(|neighbor| {
                distances.get(neighbor).copied() == Some(distance.saturating_sub(1))
            }) {
                let next_states = states.get(&next).cloned().unwrap_or_default();
                for (next_level, used_fall) in next_states {
                    for drop in [0, 1, MOUNTAIN_FALL_HEIGHT] {
                        if drop == MOUNTAIN_FALL_HEIGHT && used_fall {
                            continue;
                        }
                        let level = next_level.saturating_add(drop);
                        if level > cap {
                            continue;
                        }
                        let state = (level, used_fall || drop == MOUNTAIN_FALL_HEIGHT);
                        if states.entry(coord).or_default().insert(state) {
                            downstream
                                .insert((coord, state.0, state.1), (next, next_level, used_fall));
                        }
                    }
                }
            }
        }
    }

    let candidates = states
        .get(&source)?
        .iter()
        .copied()
        .filter(|(level, used_fall)| *used_fall && *level >= source_minimum)
        .collect::<Vec<_>>();
    candidates
        .into_iter()
        .filter_map(|(level, used_fall)| {
            let mut coordinates = vec![source];
            let mut water_levels = vec![level];
            let mut state = (source, level, used_fall);
            while state.0 != outlet {
                state = downstream.get(&state).copied()?;
                coordinates.push(state.0);
                water_levels.push(state.1);
            }
            (coordinates.iter().copied().collect::<BTreeSet<_>>().len() == coordinates.len())
                .then_some((coordinates, water_levels))
        })
        .min_by_key(|(coordinates, water_levels)| {
            (
                Reverse(water_levels.first().copied().unwrap_or_default()),
                profile_fall_index(water_levels),
                water_levels.clone(),
                coordinates.clone(),
            )
        })
}

fn profile_fall_index(profile: &[Level]) -> usize {
    profile
        .windows(2)
        .position(|pair| {
            pair.first()
                .zip(pair.get(1))
                .is_some_and(|(from, to)| from.saturating_sub(*to) == MOUNTAIN_FALL_HEIGHT)
        })
        .unwrap_or(usize::MAX)
}

fn two_wide_footprint(
    mask: &BTreeSet<HexCoord>,
    centerline: &[HexCoord],
    orientation: u8,
    positive_side: bool,
) -> BTreeSet<HexCoord> {
    let centerline_set: BTreeSet<_> = centerline.iter().copied().collect();
    let side_turn = orientation.saturating_add(2) % 6;
    let mut footprint = centerline_set.clone();
    for center in centerline {
        let shoulder = if positive_side {
            center
                .neighbors()
                .into_iter()
                .filter(|coord| mask.contains(coord) && !centerline_set.contains(coord))
                .max_by_key(|coord| (axis_value(*coord, side_turn), *coord))
        } else {
            center
                .neighbors()
                .into_iter()
                .filter(|coord| mask.contains(coord) && !centerline_set.contains(coord))
                .min_by_key(|coord| (axis_value(*coord, side_turn), *coord))
        };
        if let Some(shoulder) = shoulder {
            footprint.insert(shoulder);
        }
    }
    footprint
}

fn select_peaks(
    mask: &BTreeSet<HexCoord>,
    count: usize,
    excluded: &BTreeSet<HexCoord>,
    stream: Option<SeedStream<'_>>,
) -> Result<Vec<HexCoord>, Vec<WorldValidationIssue>> {
    let mut candidates: Vec<_> = mask
        .iter()
        .copied()
        .filter(|coord| !excluded.contains(coord))
        .filter(|coord| {
            coord
                .neighbors()
                .into_iter()
                .all(|next| mask.contains(&next))
        })
        .collect();
    candidates.sort_by_key(|coord| {
        (
            stream.map_or(coord.distance(HexCoord::ORIGIN).into(), |stream| {
                stream.sample_coord(*coord, 0)
            }),
            *coord,
        )
    });
    let mut peaks = Vec::new();
    for separation in [4_u32, 3, 2] {
        for candidate in &candidates {
            if peaks.contains(candidate)
                || peaks
                    .iter()
                    .any(|peak: &HexCoord| peak.distance(*candidate) < separation)
            {
                continue;
            }
            peaks.push(*candidate);
            if peaks.len() == count {
                return Ok(peaks);
            }
        }
    }
    Err(vec![recipe_issue(format!(
        "Mountains patch placed {} separated peaks; expected {count}",
        peaks.len()
    ))])
}

fn mountain_rise(peak: HexCoord, coord: HexCoord, relief: Level) -> Level {
    let distance = i32::try_from(peak.distance(coord)).unwrap_or(i32::MAX);
    let sharp_core = relief.saturating_sub(distance.saturating_mul(8)).max(0);
    let outer_radius = relief.saturating_div(3).saturating_add(4);
    let walker_skirt = outer_radius.saturating_sub(distance).clamp(0, 12);
    sharp_core.max(walker_skirt)
}

fn apply_route(
    levels: &mut BTreeMap<HexCoord, Level>,
    centerline: &[HexCoord],
    footprint: &BTreeSet<HexCoord>,
    base: Level,
    rise_cap: Level,
) {
    if centerline.is_empty() {
        return;
    }
    let center_levels = route_center_levels(centerline, base, rise_cap);
    for (coord, level) in &center_levels {
        levels.insert(*coord, *level);
    }
    for coord in footprint {
        if centerline.contains(coord) {
            continue;
        }
        if let Some((_, level)) = center_levels
            .iter()
            .min_by_key(|(center, _)| (center.distance(*coord), *center))
        {
            levels.insert(*coord, *level);
        }
    }
}

fn route_center_levels(
    centerline: &[HexCoord],
    base: Level,
    rise_cap: Level,
) -> BTreeMap<HexCoord, Level> {
    let last = centerline.len().saturating_sub(1);
    centerline
        .iter()
        .enumerate()
        .map(|(index, coord)| {
            let from_start = i32::try_from(index).unwrap_or(i32::MAX);
            let from_end = i32::try_from(last.saturating_sub(index)).unwrap_or(i32::MAX);
            (
                *coord,
                base.saturating_add(from_start.min(from_end).min(rise_cap)),
            )
        })
        .collect()
}

fn reconcile_ring19_route_levels(
    levels: &mut BTreeMap<HexCoord, Level>,
    high_centerline: &[HexCoord],
    bypass_centerline: &[HexCoord],
    route_cells: &BTreeSet<HexCoord>,
    base: Level,
    high_rise: Level,
    bypass_rise: Level,
) -> Result<(), String> {
    let high = route_center_levels(high_centerline, base, high_rise);
    let bypass = route_center_levels(bypass_centerline, base, bypass_rise);
    let mut centers = BTreeMap::new();
    for coord in high.keys().chain(bypass.keys()).copied() {
        let level = match (high.get(&coord).copied(), bypass.get(&coord).copied()) {
            (Some(high_level), Some(bypass_level)) if high_level != bypass_level => {
                return Err(format!(
                    "Ring19 Mountains routes assign incompatible levels {high_level} and \
                     {bypass_level} to shared centerline cell {coord:?}"
                ));
            }
            (Some(high_level), Some(_)) | (Some(high_level), None) => high_level,
            (None, Some(bypass_level)) => bypass_level,
            (None, None) => continue,
        };
        centers.insert(coord, level);
        levels.insert(coord, level);
    }
    for coord in route_cells
        .iter()
        .copied()
        .filter(|coord| !centers.contains_key(coord))
    {
        let current = levels.get(&coord).copied().unwrap_or_default();
        let Some((_, shoulder_level)) = centers
            .iter()
            .filter(|(center, _)| center.distance(coord) == 1)
            .min_by_key(|(center, level)| (current.abs_diff(**level), **center))
        else {
            return Err(format!(
                "Ring19 Mountains route shoulder {coord:?} has no adjacent centerline"
            ));
        };
        levels.insert(coord, *shoulder_level);
    }
    Ok(())
}

fn is_exposed_stone(
    coord: HexCoord,
    level: Level,
    levels: &BTreeMap<HexCoord, Level>,
    settings: &V3MountainsSettings,
) -> bool {
    let snow_capped = level
        >= settings
            .base_level
            .saturating_add(settings.relief.saturating_mul(3) / 4);
    let steep = coord.neighbors().into_iter().any(|neighbor| {
        levels
            .get(&neighbor)
            .is_some_and(|other| level.abs_diff(*other) >= 2)
    });
    steep && !snow_capped
}

fn mountain_column(surface: Level, exposed: bool) -> VolumeColumn {
    let mut elements = vec![VolumeElement::Solid(SolidMass {
        levels: LevelInterval::new(0, 1),
        material: SolidMaterialRole::Bedrock,
        cutaway_for: None,
    })];
    if exposed {
        elements.push(VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(1, surface.saturating_add(1)),
            material: SolidMaterialRole::Stone,
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
            material: SolidMaterialRole::Snow,
            cutaway_for: None,
        }));
    }
    VolumeColumn { elements }
}

fn mountain_stream_column(
    position: TilePos,
    node: LiquidNode,
    crossing_level: Option<Level>,
) -> (VolumeColumn, TilePos, Option<TilePos>) {
    let (bed_level, fill_bottom) = if node.state == LiquidFlowState::Fall {
        node.downstream.map_or_else(
            || {
                (
                    position.level.saturating_sub(2),
                    position.level.saturating_sub(1),
                )
            },
            |downstream| (downstream.level.saturating_sub(1), downstream.level),
        )
    } else {
        (
            position.level.saturating_sub(2),
            position.level.saturating_sub(1),
        )
    };
    let mut elements = vec![
        VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(0, 1),
            material: SolidMaterialRole::Bedrock,
            cutaway_for: None,
        }),
        VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(1, bed_level),
            material: SolidMaterialRole::Stone,
            cutaway_for: None,
        }),
        VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(bed_level, bed_level.saturating_add(1)),
            material: SolidMaterialRole::Gravel,
            cutaway_for: None,
        }),
        VolumeElement::Fill(NonSolidFill {
            levels: LevelInterval::new(fill_bottom, position.level.saturating_add(1)),
            material: FillMaterialRole::Water,
        }),
    ];
    if let Some(crossing_level) = crossing_level {
        elements.push(VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(crossing_level, crossing_level.saturating_add(1)),
            material: SolidMaterialRole::Metal,
            cutaway_for: None,
        }));
        (
            VolumeColumn { elements },
            TilePos::new(position.coord, bed_level),
            Some(TilePos::new(position.coord, crossing_level)),
        )
    } else {
        (
            VolumeColumn { elements },
            TilePos::new(position.coord, bed_level),
            None,
        )
    }
}

fn exact_position(
    levels: &BTreeMap<HexCoord, Level>,
    coord: HexCoord,
) -> Result<TilePos, Vec<WorldValidationIssue>> {
    levels
        .get(&coord)
        .copied()
        .map(|level| TilePos::new(coord, level))
        .ok_or_else(|| {
            vec![recipe_issue(format!(
                "Mountains surface is missing at {coord:?}"
            ))]
        })
}

fn stream_overlook(
    reachable: &BTreeMap<TilePos, u32>,
    target: TilePos,
) -> Result<TilePos, Vec<WorldValidationIssue>> {
    reachable
        .keys()
        .copied()
        .filter(|position| position.coord.distance(target.coord) <= STREAM_OVERLOOK_RADIUS)
        .min_by_key(|position| {
            (
                position.coord.distance(target.coord),
                position.level.abs_diff(target.level),
                *position,
            )
        })
        .ok_or_else(|| {
            vec![recipe_issue(format!(
                "Mountains has no reachable review footing within {STREAM_OVERLOOK_RADIUS} columns of stream feature {target:?}"
            ))]
        })
}

fn route_membership(
    centerline: &[HexCoord],
    footprint: &BTreeSet<HexCoord>,
    levels: &BTreeMap<HexCoord, Level>,
) -> Result<ProtectedFeatureRoute, Vec<WorldValidationIssue>> {
    let centerline = centerline
        .iter()
        .copied()
        .map(|coord| exact_position(levels, coord))
        .collect::<Result<Vec<_>, _>>()?;
    let surfaces = footprint
        .iter()
        .copied()
        .map(|coord| exact_position(levels, coord))
        .collect::<Result<BTreeSet<_>, _>>()?;
    Ok(ProtectedFeatureRoute {
        centerline,
        surfaces,
    })
}

fn validate_mountains(
    plan: &GeneratedWorldPlan,
    settings: &V3MountainsSettings,
    vegetation: &LandformVegetationSet,
) -> WorldValidation<MountainsMetrics> {
    validate_mountains_inner(
        plan,
        settings,
        vegetation,
        settings.base_level.saturating_add(settings.relief / 2),
        1,
        &BTreeSet::new(),
    )
}

pub(crate) fn validate_patch(
    patch: PatchRecipeContext<'_>,
    fragment: &GeneratedPatchPlan,
    settings: &V3MountainsSettings,
    catalog: &RuntimeArtCatalog,
) -> WorldValidation<MountainsMetrics> {
    let vegetation =
        match LandformVegetationSet::resolve(catalog, V3EnvironmentSettings::Frozen, "Mountains") {
            Ok(vegetation) => vegetation,
            Err(error) => return WorldValidation::Invalid(vec![recipe_issue(error)]),
        };
    let seam_approaches = patch.walker_protected_approaches();
    let available_rise = fragment
        .features
        .protected_routes
        .get(HIGH_PASS)
        .and_then(|route| {
            route
                .centerline
                .iter()
                .map(|position| {
                    seam_approaches
                        .iter()
                        .map(|approach| approach.distance(position.coord))
                        .min()
                        .unwrap_or(u32::MAX)
                })
                .max()
        })
        .and_then(|rise| i32::try_from(rise).ok())
        .unwrap_or(settings.relief / 2)
        .min(settings.relief / 2);
    let frame = match patch.local_frame() {
        Ok(frame) => frame,
        Err(error) => {
            return WorldValidation::Invalid(vec![recipe_issue(format!(
                "Mountains validation frame failed: {error}"
            ))]);
        }
    };
    let mut local = match frame.canonical_local_world(fragment) {
        Ok(plan) => plan,
        Err(error) => {
            return WorldValidation::Invalid(vec![recipe_issue(format!(
                "Mountains validation projection failed: {error}"
            ))]);
        }
    };
    local.layout.grid_radius = local
        .layout
        .footprint
        .iter()
        .map(|coord| HexCoord::ORIGIN.distance(*coord))
        .max()
        .unwrap_or_default();
    let protected_approaches = match seam_approaches
        .into_iter()
        .map(|coord| frame.to_local(coord).map_err(recipe_issue))
        .collect::<Result<BTreeSet<_>, _>>()
    {
        Ok(protected) => protected,
        Err(issue) => return WorldValidation::Invalid(vec![issue]),
    };
    validate_mountains_inner(
        &local,
        settings,
        &vegetation,
        settings.base_level.saturating_add(available_rise),
        patch
            .shared_edges()
            .filter_map(|edge| {
                edge.liquid_port()
                    .and_then(|liquid| liquid.is_source.then_some(liquid.port.lanes.len()))
            })
            .sum(),
        &protected_approaches,
    )
}

fn validate_mountains_inner(
    plan: &GeneratedWorldPlan,
    settings: &V3MountainsSettings,
    vegetation_objects: &LandformVegetationSet,
    required_saddle: Level,
    expected_streams: usize,
    additional_vegetation_protected: &BTreeSet<HexCoord>,
) -> WorldValidation<MountainsMetrics> {
    let mut issues = plan.validate();
    validate_mountain_streams(plan, settings, expected_streams, &mut issues);
    let ordinary = OrdinaryGraph::from_volume(&plan.volume, Some(&plan.blockers));
    let Some(party) = plan.anchors.get(PARTY_START).copied() else {
        issues.push(recipe_issue("Mountains is missing party_start"));
        return WorldValidation::Invalid(issues);
    };
    let Some(hostile) = plan.anchors.get(HOSTILE_START).copied() else {
        issues.push(recipe_issue("Mountains is missing hostile_start"));
        return WorldValidation::Invalid(issues);
    };
    let distances = ordinary.distances_from(party);
    let Some(lower_bypass_steps) = distances.get(&hostile).copied() else {
        issues.push(recipe_issue(
            "Mountains actor anchors are not connected by ordinary movement",
        ));
        return WorldValidation::Invalid(issues);
    };
    let Some(high_pass) = plan.features.protected_routes.get(HIGH_PASS) else {
        issues.push(recipe_issue("Mountains is missing its high pass"));
        return WorldValidation::Invalid(issues);
    };
    let Some(lower_bypass) = plan.features.protected_routes.get(LOWER_BYPASS) else {
        issues.push(recipe_issue("Mountains is missing its lower bypass"));
        return WorldValidation::Invalid(issues);
    };
    validate_route(HIGH_PASS, high_pass, &ordinary, &mut issues);
    validate_route(LOWER_BYPASS, lower_bypass, &ordinary, &mut issues);
    let shared_centerline = high_pass
        .centerline
        .iter()
        .filter(|position| lower_bypass.centerline.contains(position))
        .count();
    if shared_centerline > 2 {
        issues.push(recipe_issue(format!(
            "Mountains routes share {shared_centerline} centerline surfaces; only their landings may overlap"
        )));
    }
    let high_peak = high_pass
        .centerline
        .iter()
        .map(|position| position.level)
        .max()
        .unwrap_or_default();
    if high_peak < required_saddle {
        issues.push(recipe_issue(format!(
            "Mountains high pass reaches {high_peak}; expected saddle level {required_saddle}"
        )));
    }
    let bypass_peak = lower_bypass
        .centerline
        .iter()
        .map(|position| position.level)
        .max()
        .unwrap_or_default();
    if bypass_peak > settings.base_level.saturating_add(4) {
        issues.push(recipe_issue(
            "Mountains lower bypass rises above four levels",
        ));
    }

    let all_positions: Vec<_> = plan.volume.surfaces.keys().copied().collect();
    let mountain_surfaces = all_positions
        .iter()
        .filter(|position| position.level > settings.base_level)
        .count();
    let mountain_coverage_percent = percent(mountain_surfaces, all_positions.len());
    if mountain_coverage_percent < 52 {
        issues.push(recipe_issue(format!(
            "Mountains coverage is {mountain_coverage_percent}%; expected at least 52%"
        )));
    }
    let accessible_mountain_surfaces = distances
        .keys()
        .filter(|position| position.level > settings.base_level)
        .count();
    let standable_mountain_surfaces = plan
        .volume
        .surfaces
        .iter()
        .filter(|(position, metadata)| {
            position.level > settings.base_level && metadata.access != SurfaceAccess::NonStandable
        })
        .count();
    let accessible_mountain_percent =
        percent(accessible_mountain_surfaces, standable_mountain_surfaces);
    if accessible_mountain_percent < 60 {
        issues.push(recipe_issue(format!(
            "Mountains exposes {accessible_mountain_surfaces} of {standable_mountain_surfaces} \
             raised standable surfaces ({accessible_mountain_percent}%) to ordinary movement; \
             expected at least 60%"
        )));
    }
    let max_level = all_positions
        .iter()
        .map(|position| position.level)
        .max()
        .unwrap_or_default();
    let relief = max_level.saturating_sub(settings.base_level);
    if relief != settings.relief {
        issues.push(recipe_issue(format!(
            "Mountains relief is {relief}; expected {}",
            settings.relief
        )));
    }
    let realized_peaks = all_positions
        .iter()
        .filter(|position| position.level == settings.base_level.saturating_add(settings.relief))
        .count();
    if realized_peaks < usize::from(settings.peak_count) {
        issues.push(recipe_issue(format!(
            "Mountains realizes {realized_peaks} summit cells; expected at least {}",
            settings.peak_count
        )));
    }
    let snow_cap_level = settings
        .base_level
        .saturating_add(settings.relief.saturating_mul(3) / 4);
    if all_positions
        .iter()
        .filter(|position| position.level >= snow_cap_level)
        .any(|position| {
            surface_material_at(&plan.volume, *position) != Some(SolidMaterialRole::Snow)
        })
    {
        issues.push(recipe_issue(
            "Mountains highest elevation band is not completely snow-capped",
        ));
    }
    let cliff_edges = count_cliff_edges(&plan.volume);
    if cliff_edges == 0 {
        issues.push(recipe_issue("Mountains contains no deliberate cliff edges"));
    }
    validate_shared_approaches(plan, &ordinary, &mut issues);
    let mut vegetation_reserved = plan
        .liquids
        .bodies
        .values()
        .flat_map(|body| body.nodes.keys().map(|position| position.coord))
        .collect::<BTreeSet<_>>();
    vegetation_reserved.extend(
        plan.features
            .protected_routes
            .values()
            .flat_map(|route| route.surfaces.iter().map(|surface| surface.coord))
            .chain(plan.anchors.values().map(|anchor| anchor.coord))
            .chain(additional_vegetation_protected.iter().copied()),
    );
    let ordinary_surfaces = plan
        .volume
        .surfaces
        .iter()
        .filter_map(|(position, metadata)| {
            (metadata.access == SurfaceAccess::Ordinary).then_some((position.coord, *position))
        })
        .collect::<BTreeMap<_, _>>();
    let no_nonvegetation_blockers = BTreeSet::new();
    let vegetation = match validate_landform_vegetation(
        "Mountains",
        vegetation_objects,
        &[LandformVegetationDomain {
            surfaces: &ordinary_surfaces,
            reserved: &vegetation_reserved,
        }],
        &plan.features,
        &no_nonvegetation_blockers,
        &plan.blockers,
    ) {
        Ok(metrics) => metrics,
        Err(errors) => {
            issues.extend(errors.into_iter().map(recipe_issue));
            super::vegetation::LandformVegetationMetrics { trees: 0, grass: 0 }
        }
    };
    if !(1..=2).contains(&vegetation.trees) || vegetation.grass != 0 {
        issues.push(recipe_issue(format!(
            "Mountains has {} frozen trees and {} grass tufts; expected 1 through 2 trees and no grass",
            vegetation.trees, vegetation.grass
        )));
    }
    if plan
        .features
        .by_id
        .values()
        .any(|feature| vegetation_reserved.contains(&feature.root.coord))
    {
        issues.push(recipe_issue(
            "Mountains frozen trees leave dry terrain or a protected route, anchor, seam, or stream clearance",
        ));
    }
    if !issues.is_empty() {
        return WorldValidation::Invalid(issues);
    }
    let reachable_levels: BTreeSet<_> = distances.keys().map(|position| position.level).collect();
    let special_surfaces = plan
        .volume
        .surfaces
        .values()
        .filter(|metadata| matches!(metadata.access, SurfaceAccess::SpecialMovement(_)))
        .count();
    WorldValidation::Valid(MountainsMetrics {
        ordinary_surfaces: count_u32(ordinary.len()),
        special_surfaces: count_u32(special_surfaces),
        mountain_surfaces: count_u32(mountain_surfaces),
        mountain_coverage_percent,
        accessible_mountain_surfaces: count_u32(accessible_mountain_surfaces),
        reachable_elevation_levels: count_u32(reachable_levels.len()),
        relief,
        peak_count: settings.peak_count,
        cliff_edges,
        high_pass_steps: count_u32(high_pass.centerline.len().saturating_sub(1)),
        lower_bypass_steps,
        tree_roots: count_u32(vegetation.trees),
    })
}

fn validate_mountain_streams(
    plan: &GeneratedWorldPlan,
    settings: &V3MountainsSettings,
    expected_streams: usize,
    issues: &mut Vec<WorldValidationIssue>,
) {
    if plan.liquids.bodies.len() != expected_streams {
        issues.push(recipe_issue(format!(
            "Mountains has {} peak-fed stream bodies; expected {expected_streams}",
            plan.liquids.bodies.len()
        )));
        return;
    }
    let summit_level = settings.base_level.saturating_add(settings.relief);
    let summits = plan
        .volume
        .surfaces
        .keys()
        .filter_map(|position| (position.level == summit_level).then_some(position.coord))
        .collect::<BTreeSet<_>>();
    let mut sources = Vec::new();
    let mut falls = Vec::new();
    let mut corridor = BTreeSet::new();
    for (body_id, body) in &plan.liquids.bodies {
        corridor.extend(body.nodes.keys().map(|position| position.coord));
        if body.material != FillMaterialRole::Water {
            issues.push(recipe_issue(format!(
                "Mountains stream {body_id:?} is not water"
            )));
        }
        let fall_nodes = body
            .nodes
            .iter()
            .filter(|(_, node)| node.state == LiquidFlowState::Fall)
            .collect::<Vec<_>>();
        if fall_nodes.len() != 1 {
            issues.push(recipe_issue(format!(
                "Mountains stream {body_id:?} has {} waterfall nodes; expected one",
                fall_nodes.len()
            )));
        } else if fall_nodes.first().is_some_and(|(position, node)| {
            node.downstream.is_none_or(|downstream| {
                position.level.saturating_sub(downstream.level) != MOUNTAIN_FALL_HEIGHT
            })
        }) {
            issues.push(recipe_issue(format!(
                "Mountains stream {body_id:?} does not contain the exact {MOUNTAIN_FALL_HEIGHT}-level fall"
            )));
        } else if let Some((position, _node)) = fall_nodes.first() {
            falls.push(**position);
        }
        let terminals = body
            .nodes
            .iter()
            .filter_map(|(position, node)| {
                (node.state == LiquidFlowState::Still && node.downstream.is_none())
                    .then_some(*position)
            })
            .collect::<Vec<_>>();
        if terminals.len() != 1 {
            issues.push(recipe_issue(format!(
                "Mountains stream {body_id:?} has {} boundary outlets; expected one",
                terminals.len()
            )));
        } else if terminals.first().is_some_and(|terminal| {
            terminal
                .coord
                .neighbors()
                .into_iter()
                .all(|neighbor| plan.layout.footprint.contains(&neighbor))
        }) {
            issues.push(recipe_issue(format!(
                "Mountains stream {body_id:?} does not terminate at a patch boundary"
            )));
        }
        let downstream_targets = body
            .nodes
            .values()
            .filter_map(|node| node.downstream)
            .collect::<BTreeSet<_>>();
        let body_sources = body
            .nodes
            .keys()
            .filter(|position| !downstream_targets.contains(position))
            .copied()
            .collect::<Vec<_>>();
        if body_sources.len() != 1 {
            issues.push(recipe_issue(format!(
                "Mountains stream {body_id:?} has {} spring sources; expected one",
                body_sources.len()
            )));
        } else if let Some(source) = body_sources.first().copied() {
            sources.push(source);
            if !is_peak_fed_source(source, &summits, settings) {
                issues.push(recipe_issue(format!(
                    "Mountains stream {body_id:?} spring source {source:?} is not in the near-peak high band"
                )));
            }
        }
        for (position, node) in &body.nodes {
            if node
                .downstream
                .is_some_and(|downstream| downstream.level > position.level)
            {
                issues.push(recipe_issue(format!(
                    "Mountains stream {body_id:?} flows uphill from {position:?}"
                )));
            }
        }
    }
    if expected_streams > 1 && !horizontal_coords_connected(&corridor) {
        issues.push(recipe_issue(
            "Mountains multi-lane outlet does not form one coherent horizontal stream corridor",
        ));
    }
    let source_peaks = sources
        .iter()
        .filter_map(|source| nearest_summit(source.coord, &summits))
        .collect::<BTreeSet<_>>();
    if expected_streams > 1 && source_peaks.len() != 1 {
        issues.push(recipe_issue(format!(
            "Mountains multi-lane corridor draws from {} summit groups; expected one",
            source_peaks.len()
        )));
    }
    if expected_streams > 0 {
        validate_stream_overlook(
            plan,
            STREAM_SOURCE_OVERLOOK,
            &sources,
            STREAM_OVERLOOK_RADIUS,
            issues,
        );
        validate_stream_overlook(
            plan,
            STREAM_FALL_OVERLOOK,
            &falls,
            STREAM_OVERLOOK_RADIUS,
            issues,
        );
    }
}

fn is_peak_fed_source(
    source: TilePos,
    summits: &BTreeSet<HexCoord>,
    settings: &V3MountainsSettings,
) -> bool {
    source.level >= settings.base_level.saturating_add(settings.relief / 3)
        && summits
            .iter()
            .any(|summit| summit.distance(source.coord) <= 2)
}

fn nearest_summit(source: HexCoord, summits: &BTreeSet<HexCoord>) -> Option<HexCoord> {
    summits
        .iter()
        .copied()
        .min_by_key(|summit| (summit.distance(source), *summit))
}

fn horizontal_coords_connected(coords: &BTreeSet<HexCoord>) -> bool {
    let Some(start) = coords.first().copied() else {
        return true;
    };
    let mut reached = BTreeSet::from([start]);
    let mut frontier = VecDeque::from([start]);
    while let Some(current) = frontier.pop_front() {
        for neighbor in current.neighbors() {
            if coords.contains(&neighbor) && reached.insert(neighbor) {
                frontier.push_back(neighbor);
            }
        }
    }
    reached.len() == coords.len()
}

fn validate_stream_overlook(
    plan: &GeneratedWorldPlan,
    name: &str,
    targets: &[TilePos],
    radius: u32,
    issues: &mut Vec<WorldValidationIssue>,
) {
    let Some(anchor) = plan.anchors.get(name).copied() else {
        issues.push(recipe_issue(format!(
            "Mountains is missing required stream review anchor {name:?}"
        )));
        return;
    };
    if !targets
        .iter()
        .any(|target| target.coord.distance(anchor.coord) <= radius)
    {
        issues.push(recipe_issue(format!(
            "Mountains stream review anchor {name:?} at {anchor:?} is farther than {radius} columns from its feature"
        )));
    }
}

fn validate_route(
    name: &str,
    route: &ProtectedFeatureRoute,
    graph: &OrdinaryGraph,
    issues: &mut Vec<WorldValidationIssue>,
) {
    if route.surfaces.len() < route.centerline.len().saturating_add(2) {
        issues.push(recipe_issue(format!(
            "Mountains {name} is not meaningfully two-wide"
        )));
    }
    for pair in route.centerline.windows(2) {
        let Some(from) = pair.first().copied() else {
            continue;
        };
        let Some(to) = pair.get(1).copied() else {
            continue;
        };
        if !graph.admits(from, to) {
            issues.push(recipe_issue(format!(
                "Mountains {name} contains an illegal step {from:?} -> {to:?}"
            )));
        }
    }
    for center in &route.centerline {
        if !route
            .surfaces
            .iter()
            .any(|surface| surface.coord != center.coord && graph.admits(*center, *surface))
        {
            issues.push(recipe_issue(format!(
                "Mountains {name} has no walkable shoulder at {center:?}"
            )));
        }
    }
}

fn validate_shared_approaches(
    plan: &GeneratedWorldPlan,
    ordinary: &OrdinaryGraph,
    issues: &mut Vec<WorldValidationIssue>,
) {
    for edge in plan.layout.shared_edges.values() {
        for approaches in edge.protected_approaches.values() {
            for coord in approaches {
                let surface = plan
                    .volume
                    .surfaces
                    .keys()
                    .find(|surface| surface.coord == *coord)
                    .copied();
                let Some(surface) = surface else {
                    issues.push(recipe_issue(format!(
                        "Mountains has no seam approach at {coord:?}"
                    )));
                    continue;
                };
                if surface.level != edge.elevation.preferred || !ordinary.contains(surface) {
                    issues.push(recipe_issue(format!(
                        "Mountains seam approach {surface:?} is not ordinary footing at preferred level {}",
                        edge.elevation.preferred
                    )));
                }
            }
        }
    }
}

fn count_cliff_edges(volume: &VolumePlan) -> u32 {
    let by_coord: BTreeMap<_, _> = volume
        .surfaces
        .keys()
        .map(|position| (position.coord, position.level))
        .collect();
    let mut cliffs = 0_usize;
    for (coord, level) in &by_coord {
        for neighbor in coord.neighbors() {
            if neighbor > *coord
                && by_coord
                    .get(&neighbor)
                    .is_some_and(|other| level.abs_diff(*other) >= 2)
            {
                cliffs = cliffs.saturating_add(1);
            }
        }
    }
    count_u32(cliffs)
}

fn surface_material_at(volume: &VolumePlan, position: TilePos) -> Option<SolidMaterialRole> {
    volume.columns.get(&position.coord).and_then(|column| {
        column.elements.iter().find_map(|element| {
            let VolumeElement::Solid(mass) = element else {
                return None;
            };
            (mass.levels.bottom <= position.level && position.level < mass.levels.top)
                .then_some(mass.material)
        })
    })
}

fn mountain_view_hint(
    mask: &BTreeSet<HexCoord>,
    levels: &BTreeMap<HexCoord, Level>,
    grid_radius: u32,
    level_height: f32,
) -> Result<MapViewHint, Vec<WorldValidationIssue>> {
    let total = levels.values().try_fold(0.0_f32, |sum, level| {
        i16::try_from(*level)
            .map(|level| sum + f32::from(level))
            .map_err(|error| {
                vec![recipe_issue(format!(
                    "Mountains camera level does not fit inside i16: {error}"
                ))]
            })
    })?;
    let count = f32::from(u16::try_from(levels.len()).map_err(|error| {
        vec![recipe_issue(format!(
            "Mountains camera footprint does not fit inside u16: {error}"
        ))]
    })?);
    let focus_level = (total / count) * level_height;
    let radius = u16::try_from(grid_radius)
        .map_err(|error| vec![recipe_issue(format!("Mountains camera radius: {error}"))])?;
    let frame = f32::from(radius).mul_add(3.5, 12.0);
    let center = mask
        .iter()
        .copied()
        .min_by_key(|coord| coord.distance(HexCoord::ORIGIN))
        .unwrap_or(HexCoord::ORIGIN);
    let horizontal = f32::from(i16::try_from(center.x()).map_err(|error| {
        vec![recipe_issue(format!(
            "Mountains camera coordinate does not fit inside i16: {error}"
        ))]
    })?) * 0.5;
    Ok(MapViewHint::new(
        (horizontal, focus_level + frame * 0.82, frame * 0.78),
        (horizontal, focus_level, 0.0),
    ))
}

fn axis_value(coord: HexCoord, turns: u8) -> i32 {
    let [x, y, z] = rotate(coord, turns).to_cubic_array();
    x.saturating_sub(z).saturating_add(y / 2)
}

fn rotate(coord: HexCoord, turns: u8) -> HexCoord {
    let [mut x, mut y, mut z] = coord.to_cubic_array();
    for _ in 0..turns % 6 {
        (x, y, z) = (-z, -x, -y);
    }
    HexCoord::new_cubic(x, y, z)
}

fn percent(part: usize, total: usize) -> u32 {
    count_u32(part)
        .saturating_mul(100)
        .checked_div(count_u32(total))
        .unwrap_or_default()
}

fn count_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn recipe_issue(detail: impl Into<String>) -> WorldValidationIssue {
    WorldValidationIssue::new(WorldIssueCode::Recipe("mountains"), detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{PatchEdgeContractSettings, PatchEdgesSettings, PatchMaskSettings};

    fn world_boundaries() -> PatchEdgesSettings {
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
            layout: V3LayoutSettings::Single(crate::settings::PatchSpec {
                environment: V3EnvironmentSettings::Frozen,
                recipe: V3RecipeSettings::Mountains(V3MountainsSettings {
                    base_level: 15,
                    relief: 32,
                    peak_count: 5,
                }),
                overlays: Vec::new(),
                mask: PatchMaskSettings::WholeWorld,
                edges: world_boundaries(),
            }),
        }
    }

    #[test]
    fn native_mountains_are_broad_sharp_and_offer_two_routes() {
        let settings = settings();
        let first = generate(
            12,
            0.4,
            &settings,
            5_181,
            super::super::vegetation::tests::runtime_art_catalog(),
        )
        .expect("valid Mountains");
        let second = generate(
            12,
            0.4,
            &settings,
            5_181,
            super::super::vegetation::tests::runtime_art_catalog(),
        )
        .expect("same valid Mountains");

        assert_eq!(
            first.validated.semantic_fingerprint,
            second.validated.semantic_fingerprint
        );
        assert!(first.metrics.mountain_coverage_percent >= 52);
        assert!(first.metrics.accessible_mountain_surfaces > 0);
        assert_eq!(first.metrics.relief, 32);
        assert_eq!(first.metrics.peak_count, 5);
        assert!(first.metrics.cliff_edges > 0);
        assert!(first.metrics.high_pass_steps > 0);
        assert!(first.metrics.lower_bypass_steps > 0);
        assert!(
            percent(
                usize::try_from(first.metrics.accessible_mountain_surfaces).unwrap_or(usize::MAX),
                usize::try_from(first.metrics.mountain_surfaces).unwrap_or(usize::MAX)
            ) >= 60
        );
        let streams = &first.validated.plan.liquids.bodies;
        assert_eq!(streams.len(), 1);
        let stream = streams.values().next().expect("one mountain stream");
        assert_eq!(stream.material, FillMaterialRole::Water);
        assert_eq!(
            stream
                .nodes
                .values()
                .filter(|node| node.state == LiquidFlowState::Fall)
                .count(),
            1
        );
        let downstream = stream
            .nodes
            .values()
            .filter_map(|node| node.downstream)
            .collect::<BTreeSet<_>>();
        let source = stream
            .nodes
            .keys()
            .find(|position| !downstream.contains(position))
            .copied()
            .expect("one exact mountain spring");
        let mountain_settings = match &settings.layout {
            V3LayoutSettings::Single(patch) => match &patch.recipe {
                V3RecipeSettings::Mountains(settings) => settings,
                _ => unreachable!("test uses Mountains"),
            },
            _ => unreachable!("test uses Single"),
        };
        let summits = first
            .validated
            .plan
            .volume
            .surfaces
            .keys()
            .filter_map(|position| {
                (position.level
                    == mountain_settings
                        .base_level
                        .saturating_add(mountain_settings.relief))
                .then_some(position.coord)
            })
            .collect::<BTreeSet<_>>();
        assert!(is_peak_fed_source(source, &summits, mountain_settings));
        for anchor in [STREAM_SOURCE_OVERLOOK, STREAM_FALL_OVERLOOK] {
            assert!(
                first.validated.plan.anchors.contains_key(anchor),
                "missing {anchor}"
            );
        }
    }

    #[test]
    fn low_foothill_spring_corruption_is_rejected() {
        let settings = settings();
        let mountain_settings = match &settings.layout {
            V3LayoutSettings::Single(patch) => match &patch.recipe {
                V3RecipeSettings::Mountains(settings) => settings,
                _ => unreachable!("test uses Mountains"),
            },
            _ => unreachable!("test uses Single"),
        };
        let selected = generate(
            12,
            0.4,
            &settings,
            5_181,
            super::super::vegetation::tests::runtime_art_catalog(),
        )
        .expect("valid Mountains");
        let mut corrupted = selected.validated.plan;
        let body = corrupted
            .liquids
            .bodies
            .values_mut()
            .next()
            .expect("one mountain stream");
        let downstream = body
            .nodes
            .values()
            .filter_map(|node| node.downstream)
            .collect::<BTreeSet<_>>();
        let source = body
            .nodes
            .keys()
            .find(|position| !downstream.contains(position))
            .copied()
            .expect("one mountain spring");
        let source_node = body.nodes.remove(&source).expect("source node");
        let low_source = TilePos::new(source.coord, mountain_settings.base_level.saturating_add(2));
        assert!(body.nodes.insert(low_source, source_node).is_none());
        let vegetation = LandformVegetationSet::resolve(
            super::super::vegetation::tests::runtime_art_catalog(),
            V3EnvironmentSettings::Frozen,
            "Mountains",
        )
        .expect("accepted snowy vegetation");
        let WorldValidation::Invalid(issues) =
            validate_mountains(&corrupted, mountain_settings, &vegetation)
        else {
            panic!("low foothill spring corruption was accepted");
        };
        assert!(
            issues
                .iter()
                .any(|issue| issue.detail.contains("not in the near-peak high band")),
            "missing peak-source invariant issue: {issues:#?}"
        );
    }

    #[test]
    fn multi_lane_stream_corridor_must_be_horizontally_coherent() {
        let origin = HexCoord::ORIGIN;
        let east = origin.neighbors()[0];
        let east_two = east
            .neighbors()
            .into_iter()
            .find(|coord| coord.distance(origin) == 2)
            .expect("outward neighbor");
        assert!(horizontal_coords_connected(&BTreeSet::from([
            origin, east, east_two
        ])));
        assert!(!horizontal_coords_connected(&BTreeSet::from([
            origin,
            HexCoord::new_cubic(3, 0, -3),
        ])));
    }

    #[test]
    fn revised_mountains_validate_supported_radius_coverage() {
        let settings = settings();
        for radius in [12, 20, 40] {
            let selected = generate(
                radius,
                0.4,
                &settings,
                1_592_598_566,
                super::super::vegetation::tests::runtime_art_catalog(),
            )
            .unwrap_or_else(|error| panic!("Mountains radius {radius}: {error}"));
            assert_eq!(selected.metrics.relief, 32);
            assert_eq!(selected.metrics.tree_roots, 2);
            assert!(selected
                .validated
                .plan
                .features
                .by_id
                .values()
                .all(|feature| feature.object_id.as_str().contains("snowy-")));
            assert!(
                percent(
                    usize::try_from(selected.metrics.accessible_mountain_surfaces)
                        .unwrap_or(usize::MAX),
                    usize::try_from(selected.metrics.mountain_surfaces).unwrap_or(usize::MAX)
                ) >= 60
            );
        }
    }

    #[test]
    fn revised_mountains_pr_corpus_validates_128_seeds_and_named_regression() {
        let settings = settings();
        let mut seeds = (0_u64..128).collect::<BTreeSet<_>>();
        seeds.insert(1_592_598_566);
        let mut fallback_seeds = Vec::new();
        for seed in seeds {
            let selected = generate(
                12,
                0.4,
                &settings,
                seed,
                super::super::vegetation::tests::runtime_art_catalog(),
            )
            .unwrap_or_else(|error| panic!("Mountains seed {seed}: {error}"));
            assert_eq!(selected.candidates_evaluated, 8);
            if selected.used_fallback {
                fallback_seeds.push(seed);
            }
        }
        assert!(
            fallback_seeds.is_empty(),
            "revised Mountains used fallback for seeds {fallback_seeds:?}"
        );
    }

    #[test]
    fn non_frozen_mountains_fail_explicitly() {
        let mut settings = settings();
        let V3LayoutSettings::Single(patch) = &mut settings.layout else {
            unreachable!("test uses Single")
        };
        patch.environment = V3EnvironmentSettings::Rocky;
        assert!(generate(
            12,
            0.4,
            &settings,
            1,
            super::super::vegetation::tests::runtime_art_catalog(),
        )
        .is_err());
    }
}
