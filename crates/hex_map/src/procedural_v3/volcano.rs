//! Native V3 Volcano recipe.
//!
//! The massif, lava topology, elevated crossing, and stair approaches are planned
//! together. Lava never becomes traversable footing, and the bridge remains the
//! sole ordinary crossing through the three-wide massif-to-boundary barrier.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use hex_core::{HexCoord, Level, MapViewHint, TilePos};

use super::composition::{compose_single_patch, GeneratedPatchPlan};
use super::layout::{
    resolve_layout, HexSide, PatchId, ResolvedBoundaryLiquidOutlet, ResolvedLayoutPlan,
};
use super::liquid::{LiquidBodyId, LiquidBodyPlan, LiquidFlowState, LiquidNode, LiquidPlan};
use super::local_frame::LocalPatchFrame;
use super::patch::{PatchBuildMode, PatchRecipeContext};
use super::seam::{shape_walker_seams, validate_patch_walker_seams};
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
    FeaturePlan, GeneratedWorldPlan, InteriorPlan, PlannedStructure, ProtectedFeatureRoute,
    StructureId, StructureKind, StructurePlan, WorldIssueCode, WorldValidationIssue,
};
use super::V3GenerationError;
use crate::settings::{
    ProceduralV3Settings, V3EnvironmentSettings, V3LayoutSettings, V3RecipeSettings,
    V3VolcanoSettings,
};

const PARTY_START: &str = "party_start";
const HOSTILE_START: &str = "hostile_start";
const CONFLICT_CENTER: &str = "conflict_center";
const BRIDGE_ANCHOR: &str = "bridge";
const CRATER_OVERLOOK: &str = "crater_overlook";
const BRIDGE_ROUTE: &str = "bridge_route";
const LAVA_LANES: [i32; 3] = [-1, 0, 1];
const BRIDGE_FLOW_ROWS: [i32; 2] = [0, 1];
const CRATER_DEPTH: Level = 4;
const DESCENT_STAGES: usize = 4;

/// Exact deterministic measurements of one admitted Volcano plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VolcanoMetrics {
    pub(crate) ordinary_surfaces: u32,
    pub(crate) reachable_elevation_levels: u32,
    pub(crate) summit_relief: Level,
    pub(crate) massif_surfaces: u32,
    pub(crate) massif_coverage_percent: u32,
    pub(crate) lava_nodes: u32,
    pub(crate) fall_nodes: u32,
    pub(crate) maximum_fall_height: Level,
    pub(crate) bridge_surfaces: u32,
    pub(crate) bridge_clearance: Level,
    pub(crate) critical_route_steps: u32,
}

struct VolcanoRecipe {
    level_height: f32,
    layout: ResolvedLayoutPlan,
    settings: V3VolcanoSettings,
    #[cfg(test)]
    reject_candidates: bool,
}

#[derive(Clone, Copy)]
struct VolcanoStreams<'a> {
    orientation: SeedStream<'a>,
    massif: SeedStream<'a>,
}

#[derive(Debug)]
struct VolcanoGeometry {
    massif: BTreeSet<HexCoord>,
    surfaces: BTreeMap<HexCoord, Level>,
    lava: BTreeMap<TilePos, LiquidNode>,
    bridge: BTreeSet<TilePos>,
    stairs: BTreeSet<TilePos>,
    route: ProtectedFeatureRoute,
    anchors: BTreeMap<String, TilePos>,
    low_lava_level: Level,
}

pub(crate) fn generate(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV3Settings,
    seed: u64,
) -> Result<ValidatedWorldSelection<VolcanoMetrics>, V3GenerationError> {
    generate_inner(grid_radius, level_height, settings, seed, false)
}

fn generate_inner(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV3Settings,
    seed: u64,
    reject_candidates: bool,
) -> Result<ValidatedWorldSelection<VolcanoMetrics>, V3GenerationError> {
    #[cfg(not(test))]
    let _ = reject_candidates;
    if !level_height.is_finite() || level_height <= 0.0 {
        return Err(V3GenerationError::RecipeContract(
            "Volcano level height must be positive and finite".to_owned(),
        ));
    }
    let volcano = recipe_settings(settings)?.clone();
    let layout = resolve_layout(grid_radius, settings)
        .map_err(|error| V3GenerationError::RecipeContract(error.to_string()))?;
    run_recipe(
        &VolcanoRecipe {
            level_height,
            layout,
            settings: volcano,
            #[cfg(test)]
            reject_candidates,
        },
        settings,
        grid_radius,
        seed,
    )
}

impl V3Recipe for VolcanoRecipe {
    type Settings = ProceduralV3Settings;
    type Metrics = VolcanoMetrics;
    type Score = (u32, Reverse<Level>, u32, u8);

    fn construct(
        &self,
        context: CandidateContext,
        _settings: &Self::Settings,
    ) -> Result<GeneratedWorldPlan, CandidateAttemptError> {
        #[cfg(test)]
        if self.reject_candidates {
            return Err(CandidateAttemptError::Rejected(vec![recipe_issue(
                "forced Volcano candidate rejection",
            )]));
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
                "Volcano single-patch composition failed: {error:?}"
            )))
        })
    }

    fn validate(
        &self,
        _settings: &Self::Settings,
        plan: &GeneratedWorldPlan,
    ) -> WorldValidation<Self::Metrics> {
        validate_volcano(plan, &self.settings)
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
            metrics
                .massif_coverage_percent
                .abs_diff(u32::from(self.settings.massif_coverage_percent)),
            Reverse(metrics.maximum_fall_height),
            metrics.critical_route_steps,
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
                "Volcano fallback radius disagrees with its resolved layout".to_owned(),
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
                "Volcano fallback composition failed: {error:?}"
            ))
        })
    }
}

fn recipe_settings(
    settings: &ProceduralV3Settings,
) -> Result<&V3VolcanoSettings, V3GenerationError> {
    let V3LayoutSettings::Single(patch) = &settings.layout else {
        return Err(V3GenerationError::RecipeUnavailable("Ring19"));
    };
    let V3RecipeSettings::Volcano(volcano) = &patch.recipe else {
        return Err(V3GenerationError::RecipeUnavailable(recipe_name(
            &patch.recipe,
        )));
    };
    if patch.environment != V3EnvironmentSettings::Volcanic {
        return Err(V3GenerationError::RecipeContract(
            "Volcano requires the Volcanic environment".to_owned(),
        ));
    }
    if !patch.overlays.is_empty() {
        return Err(V3GenerationError::RecipeContract(
            "Volcano overlays are not implemented yet".to_owned(),
        ));
    }
    Ok(volcano)
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

pub(crate) fn construct_patch(
    patch: PatchRecipeContext<'_>,
    settings: &V3VolcanoSettings,
    level_height: f32,
    mode: PatchBuildMode,
) -> Result<GeneratedPatchPlan, Vec<WorldValidationIssue>> {
    let streams = mode.seed_streams(&patch);
    construct_patch_with_streams(
        patch,
        settings,
        level_height,
        streams.map(|streams| VolcanoStreams {
            orientation: streams.stage("volcano.orientation"),
            massif: streams.stage("volcano.massif"),
        }),
    )
}

fn construct_patch_with_streams(
    patch: PatchRecipeContext<'_>,
    settings: &V3VolcanoSettings,
    level_height: f32,
    streams: Option<VolcanoStreams<'_>>,
) -> Result<GeneratedPatchPlan, Vec<WorldValidationIssue>> {
    let frame = patch
        .local_frame_with_rotation(0)
        .map_err(|error| vec![recipe_issue(format!("Volcano local frame failed: {error}"))])?;
    let local_mask = frame.local_mask(patch.mask()).map_err(|error| {
        vec![recipe_issue(format!(
            "Volcano local mask conversion failed: {error}"
        ))]
    })?;
    let seeded_orientation = streams.map_or(0, |streams| {
        u8::try_from(streams.orientation.sample(0) % 6).unwrap_or_default()
    });
    let orientation = volcano_orientation(&patch, seeded_orientation)?;
    let geometry = plan_geometry(
        &local_mask,
        frame.scale(),
        settings,
        orientation,
        streams.map(|streams| streams.massif),
    )?;
    let geometry = geometry_to_world(geometry, frame)?;
    let lava_coords = geometry
        .lava
        .keys()
        .map(|position| position.coord)
        .collect::<BTreeSet<_>>();
    let walker_approaches = patch
        .shared_edges()
        .flat_map(|edge| edge.walker_ports())
        .flat_map(|port| port.first_approach)
        .collect::<BTreeSet<_>>();
    if !lava_coords.is_disjoint(&walker_approaches) {
        return Err(vec![recipe_issue(
            "Volcano lava overlaps a resolved walker seam approach",
        )]);
    }
    let mut surface_levels = geometry.surfaces.clone();
    let seam_shape = shape_walker_seams(&patch, &mut surface_levels)?;
    for coord in &geometry.massif {
        if walker_approaches.contains(coord) {
            continue;
        }
        if let Some(authored) = geometry.surfaces.get(coord).copied() {
            surface_levels.insert(*coord, authored);
        }
    }
    let lava_by_coord = geometry
        .lava
        .iter()
        .map(|(position, node)| (position.coord, (*position, *node)))
        .collect::<BTreeMap<_, _>>();
    let bridge_by_coord = geometry
        .bridge
        .iter()
        .map(|position| (position.coord, *position))
        .collect::<BTreeMap<_, _>>();
    let stairs_by_coord = geometry
        .stairs
        .iter()
        .map(|position| (position.coord, *position))
        .collect::<BTreeMap<_, _>>();

    let mut columns = BTreeMap::new();
    let mut surfaces = BTreeMap::new();
    for coord in patch.mask() {
        if let Some((lava_position, node)) = lava_by_coord.get(coord).copied() {
            let (mut column, bed) = lava_column(lava_position, node);
            surfaces.insert(
                bed,
                SurfaceMetadata {
                    access: SurfaceAccess::NonStandable,
                    interior: None,
                },
            );
            if let Some(bridge) = bridge_by_coord.get(coord).copied() {
                column.elements.push(VolumeElement::Solid(SolidMass {
                    levels: LevelInterval::new(bridge.level, bridge.level.saturating_add(1)),
                    material: SolidMaterialRole::Metal,
                    cutaway_for: None,
                }));
                surfaces.insert(
                    bridge,
                    SurfaceMetadata {
                        access: SurfaceAccess::Ordinary,
                        interior: None,
                    },
                );
            }
            columns.insert(*coord, column);
            continue;
        }

        let level = surface_levels
            .get(coord)
            .copied()
            .unwrap_or(settings.base_level);
        let mut column = basalt_column(level);
        let ground = TilePos::new(*coord, level);
        let mut surface = ground;
        let mut access = SurfaceAccess::Ordinary;
        if let Some(stair) = stairs_by_coord.get(coord).copied() {
            if stair.level > ground.level {
                column.elements.push(VolumeElement::Solid(SolidMass {
                    levels: LevelInterval::new(
                        ground.level.saturating_add(1),
                        stair.level.saturating_add(1),
                    ),
                    material: SolidMaterialRole::WorkedStone,
                    cutaway_for: None,
                }));
            }
            surface = stair;
            access = SurfaceAccess::Ordinary;
        }
        columns.insert(*coord, column);
        surfaces.insert(
            surface,
            SurfaceMetadata {
                access,
                interior: None,
            },
        );
    }
    let mut volume = VolumePlan {
        mask: patch.mask().clone(),
        columns,
        surfaces,
    };
    seam_shape.apply(&mut volume)?;
    let biome_regions = volume
        .surfaces
        .keys()
        .copied()
        .map(|surface| (surface, patch.biome_region()))
        .collect();
    let mut anchors = geometry.anchors;
    for anchor in anchors.values_mut() {
        if volume
            .surfaces
            .get(anchor)
            .is_some_and(|metadata| metadata.access == SurfaceAccess::Ordinary)
        {
            continue;
        }
        let Some(projected) = volume
            .surfaces
            .iter()
            .filter_map(|(surface, metadata)| {
                (surface.coord == anchor.coord && metadata.access == SurfaceAccess::Ordinary)
                    .then_some(*surface)
            })
            .max_by_key(|surface| surface.level)
        else {
            return Err(vec![recipe_issue(format!(
                "Volcano anchor at {:?} has no ordinary surface after seam shaping",
                anchor.coord
            ))]);
        };
        *anchor = projected;
    }
    let structures = StructurePlan {
        by_id: BTreeMap::from([
            (
                StructureId(0),
                PlannedStructure {
                    kind: StructureKind::Bridge,
                    voxels: geometry.bridge.clone(),
                },
            ),
            (
                StructureId(1),
                PlannedStructure {
                    kind: StructureKind::Stair,
                    voxels: geometry.stairs.clone(),
                },
            ),
        ]),
    };
    let view_hint = frame.view_hint_rotated_to_world(
        volcano_view_hint(
            frame.scale(),
            settings.base_level.saturating_add(settings.summit_relief),
            level_height,
        )?,
        orientation,
    );
    let fragment = GeneratedPatchPlan {
        patch_id: patch.id,
        volume,
        liquids: LiquidPlan {
            bodies: BTreeMap::from([(
                LiquidBodyId(0),
                LiquidBodyPlan {
                    material: FillMaterialRole::Lava,
                    nodes: geometry.lava,
                },
            )]),
        },
        features: FeaturePlan {
            by_id: BTreeMap::new(),
            protected_routes: BTreeMap::from([(BRIDGE_ROUTE.to_owned(), geometry.route)]),
            clearings: BTreeMap::new(),
        },
        structures,
        blockers: BTreeSet::new(),
        lights: BTreeMap::new(),
        biome_regions,
        interiors: InteriorPlan::default(),
        anchors,
        view_hint,
    };
    match validate_patch(patch, &fragment, settings) {
        WorldValidation::Valid(_) => Ok(fragment),
        WorldValidation::Invalid(issues) => Err(issues),
    }
}

fn volcano_orientation(
    patch: &PatchRecipeContext<'_>,
    seeded_orientation: u8,
) -> Result<u8, Vec<WorldValidationIssue>> {
    if !patch.layout().kind.is_composite() {
        return Ok(seeded_orientation);
    }
    if !patch.is_world_boundary(HexSide::West) {
        return Err(vec![recipe_issue(
            "composite Volcano lava requires a western world-boundary outlet",
        )]);
    }
    if patch
        .shared_edges()
        .any(|edge| edge.liquid_port().is_some())
    {
        return Err(vec![recipe_issue(
            "composite Volcano lava must remain separate from stitched liquid ports",
        )]);
    }
    ring19_volcano_outlet(patch).map_err(|error| vec![recipe_issue(error)])?;

    // Local lava advances along +x. Three turns map that axis to world-West,
    // making every terminal lane exit the required outer boundary.
    if patch.layout().kind == super::layout::LayoutKind::Ring19 && patch.rotation_turns() != 3 {
        return Err(vec![recipe_issue(format!(
            "Ring19 Volcano rotation_turns {} must match its western outlet orientation 3",
            patch.rotation_turns()
        ))]);
    }
    Ok(3)
}

fn ring19_volcano_outlet<'layout>(
    patch: &PatchRecipeContext<'layout>,
) -> Result<Option<&'layout ResolvedBoundaryLiquidOutlet>, String> {
    if patch.layout().kind != super::layout::LayoutKind::Ring19 {
        return Ok(None);
    }
    let outlets = patch.boundary_liquid_outlets().collect::<Vec<_>>();
    let [outlet] = outlets.as_slice() else {
        return Err(format!(
            "Ring19 Volcano requires exactly one resolved boundary outlet; found {}",
            outlets.len()
        ));
    };
    if outlet.side != HexSide::West || outlet.lanes.len() != LAVA_LANES.len() {
        return Err(
            "Ring19 Volcano requires one exact three-lane western boundary outlet".to_owned(),
        );
    }
    Ok(Some(outlet))
}

fn plan_geometry(
    mask: &BTreeSet<HexCoord>,
    scale: u32,
    settings: &V3VolcanoSettings,
    orientation: u8,
    massif_stream: Option<SeedStream<'_>>,
) -> Result<VolcanoGeometry, Vec<WorldValidationIssue>> {
    let radius = i32::try_from(scale)
        .map_err(|error| vec![recipe_issue(format!("Volcano radius exceeds i32: {error}"))])?;
    let crater_x = -(radius / 2);
    let crater = rotate(HexCoord::from_axial(crater_x, 0), orientation);
    if !mask.contains(&crater) {
        return Err(vec![recipe_issue(
            "Volcano cannot place its off-centre crater inside the patch",
        )]);
    }
    let west_delta = rotate(HexCoord::from_axial(-1, 0), orientation);
    let mut mandatory_massif = BTreeSet::new();
    for lane in LAVA_LANES {
        let mut coord = rotate(HexCoord::from_axial(crater_x, lane), orientation);
        while mask.contains(&coord) {
            mandatory_massif.insert(coord);
            let next = shift(coord, west_delta);
            if !mask.contains(&next) {
                break;
            }
            coord = next;
        }
    }
    let massif_target = mask
        .len()
        .saturating_mul(usize::from(settings.massif_coverage_percent))
        / 100;
    if mandatory_massif.len() > massif_target {
        return Err(vec![recipe_issue(format!(
            "Volcano's boundary-rooted massif requires {} cells, exceeding target {massif_target}",
            mandatory_massif.len()
        ))]);
    }
    let mut candidates = mask
        .difference(&mandatory_massif)
        .copied()
        .filter(|coord| {
            let local = unrotate(*coord, orientation);
            !BRIDGE_FLOW_ROWS.contains(&local.x())
        })
        .collect::<Vec<_>>();
    candidates.sort_unstable_by_key(|coord| {
        (
            crater.distance(*coord),
            massif_stream.map_or(0, |stream| stream.sample_coord(*coord, 0)),
            *coord,
        )
    });
    let mut massif = mandatory_massif;
    massif.extend(
        candidates
            .into_iter()
            .take(massif_target.saturating_sub(massif.len())),
    );
    if massif.len() != massif_target {
        return Err(vec![recipe_issue(format!(
            "Volcano selected {} massif cells; expected {massif_target}",
            massif.len()
        ))]);
    }
    let massif_radius = massif
        .iter()
        .map(|coord| crater.distance(*coord))
        .max()
        .unwrap_or(1)
        .max(1);
    let mut surfaces = mask
        .iter()
        .copied()
        .map(|coord| (coord, settings.base_level))
        .collect::<BTreeMap<_, _>>();
    for coord in &massif {
        let distance = crater.distance(*coord);
        let rise = massif_rise(distance, massif_radius, settings.summit_relief);
        surfaces.insert(*coord, settings.base_level.saturating_add(rise));
    }

    let lanes = lava_lane_paths(mask, scale, orientation)?;

    let low_lava_level = settings
        .base_level
        .saturating_add(3)
        .saturating_sub(settings.bridge_clearance)
        .max(3);
    let bridge_level = low_lava_level.saturating_add(settings.bridge_clearance);
    let stair_steps = bridge_level.saturating_sub(settings.base_level);
    let maximum_stair_steps = radius.saturating_sub(2).max(1);
    if stair_steps > maximum_stair_steps {
        return Err(vec![recipe_issue(format!(
            "Volcano bridge requires {stair_steps} stair levels, but radius {scale} admits at most {maximum_stair_steps}"
        ))]);
    }
    let crater_lava_level = settings
        .base_level
        .saturating_add(settings.summit_relief)
        .saturating_sub(CRATER_DEPTH);
    if crater_lava_level <= low_lava_level {
        return Err(vec![recipe_issue(
            "Volcano crater lava must begin above its boundary flow",
        )]);
    }

    let mut lava = BTreeMap::new();
    for path in &lanes {
        let bridge_index = path
            .iter()
            .position(|coord| unrotate(*coord, orientation).x() == 0)
            .ok_or_else(|| vec![recipe_issue("Volcano lava misses its bridge longitude")])?;
        let levels = lava_levels(path.len(), bridge_index, crater_lava_level, low_lava_level);
        for (index, coord) in path.iter().copied().enumerate() {
            let Some(level) = levels.get(index).copied() else {
                return Err(vec![recipe_issue(
                    "Volcano lava level projection is incomplete",
                )]);
            };
            let downstream = path.get(index.saturating_add(1)).and_then(|next| {
                levels
                    .get(index.saturating_add(1))
                    .copied()
                    .map(|next_level| TilePos::new(*next, next_level))
            });
            let state = match downstream {
                None => LiquidFlowState::Still,
                Some(next) if level.saturating_sub(next.level) >= 2 => LiquidFlowState::Fall,
                Some(_) if massif.contains(&coord) => LiquidFlowState::Rapid,
                Some(_) => LiquidFlowState::Current,
            };
            lava.insert(TilePos::new(coord, level), LiquidNode { state, downstream });
        }
    }

    let mut bridge = BTreeSet::new();
    for x in BRIDGE_FLOW_ROWS {
        for y in LAVA_LANES {
            let coord = rotate(HexCoord::from_axial(x, y), orientation);
            let Some((lava_position, _node)) = lava
                .iter()
                .find(|(position, _node)| position.coord == coord)
            else {
                return Err(vec![recipe_issue(format!(
                    "Volcano bridge has no lava below {coord:?}"
                ))]);
            };
            if lava_position.level != low_lava_level {
                return Err(vec![recipe_issue(
                    "Volcano bridge is not downstream of every lava fall",
                )]);
            }
            bridge.insert(TilePos::new(coord, bridge_level));
        }
    }

    let mut stairs = BTreeSet::new();
    let mut route_surfaces = bridge.clone();
    let mut centerline = Vec::new();
    let route_extent = stair_steps.saturating_add(1);
    for signed in -route_extent..=route_extent {
        let magnitude = signed.abs();
        let level = if magnitude <= 1 {
            bridge_level
        } else if magnitude <= stair_steps {
            bridge_level.saturating_sub(magnitude.saturating_sub(1))
        } else {
            settings.base_level
        };
        for x in BRIDGE_FLOW_ROWS {
            let coord = rotate(HexCoord::from_axial(x, signed), orientation);
            if !mask.contains(&coord) || lava.keys().any(|position| position.coord == coord) {
                if magnitude <= 1 {
                    let surface = TilePos::new(coord, bridge_level);
                    if !bridge.contains(&surface) {
                        return Err(vec![recipe_issue(
                            "Volcano bridge route leaves its exact deck",
                        )]);
                    }
                    route_surfaces.insert(surface);
                    if x == 0 {
                        centerline.push(surface);
                    }
                    continue;
                }
                return Err(vec![recipe_issue(
                    "Volcano stair approach leaves dry terrain",
                )]);
            }
            let surface = TilePos::new(coord, level);
            route_surfaces.insert(surface);
            if magnitude > 1 && magnitude <= stair_steps {
                stairs.insert(surface);
            }
            if x == 0 {
                centerline.push(surface);
            }
        }
    }
    centerline.sort_unstable_by_key(|position| unrotate(position.coord, orientation).y());
    let party = centerline
        .first()
        .copied()
        .ok_or_else(|| vec![recipe_issue("Volcano bridge route has no party landing")])?;
    let hostile = centerline
        .last()
        .copied()
        .ok_or_else(|| vec![recipe_issue("Volcano bridge route has no hostile landing")])?;
    let bridge_anchor = centerline
        .iter()
        .copied()
        .find(|position| unrotate(position.coord, orientation).y() == 0)
        .ok_or_else(|| vec![recipe_issue("Volcano bridge route has no center")])?;
    let crater_overlook_coord = rotate(HexCoord::from_axial(crater_x, 2), orientation);
    let crater_overlook = surfaces
        .get(&crater_overlook_coord)
        .copied()
        .map(|level| TilePos::new(crater_overlook_coord, level))
        .ok_or_else(|| vec![recipe_issue("Volcano crater overlook leaves the massif")])?;
    let anchors = BTreeMap::from([
        (PARTY_START.to_owned(), party),
        (HOSTILE_START.to_owned(), hostile),
        (CONFLICT_CENTER.to_owned(), bridge_anchor),
        (BRIDGE_ANCHOR.to_owned(), bridge_anchor),
        (CRATER_OVERLOOK.to_owned(), crater_overlook),
    ]);
    Ok(VolcanoGeometry {
        massif,
        surfaces,
        lava,
        bridge,
        stairs,
        route: ProtectedFeatureRoute {
            centerline,
            surfaces: route_surfaces,
        },
        anchors,
        low_lava_level,
    })
}

fn lava_lane_paths(
    mask: &BTreeSet<HexCoord>,
    scale: u32,
    orientation: u8,
) -> Result<Vec<Vec<HexCoord>>, Vec<WorldValidationIssue>> {
    let radius = i32::try_from(scale)
        .map_err(|error| vec![recipe_issue(format!("Volcano radius exceeds i32: {error}"))])?;
    let crater_x = -(radius / 2);
    let flow_delta = rotate(HexCoord::from_axial(1, 0), orientation);
    let mut lanes = Vec::with_capacity(LAVA_LANES.len());
    for lane in LAVA_LANES {
        let mut path = Vec::new();
        let mut coord = rotate(HexCoord::from_axial(crater_x, lane), orientation);
        while mask.contains(&coord) {
            path.push(coord);
            let next = shift(coord, flow_delta);
            if !mask.contains(&next) {
                break;
            }
            coord = next;
        }
        if path.len() < 8 {
            return Err(vec![recipe_issue(
                "Volcano cannot route three lava lanes from crater to boundary",
            )]);
        }
        lanes.push(path);
    }
    Ok(lanes)
}

fn geometry_to_world(
    geometry: VolcanoGeometry,
    frame: LocalPatchFrame,
) -> Result<VolcanoGeometry, Vec<WorldValidationIssue>> {
    let convert_coord = |coord| {
        frame
            .to_world(coord)
            .map_err(|error| vec![recipe_issue(format!("Volcano conversion failed: {error}"))])
    };
    let convert_position = |position: TilePos| {
        frame.position_to_world(position).map_err(|error| {
            vec![recipe_issue(format!(
                "Volcano position conversion failed: {error}"
            ))]
        })
    };
    let massif = geometry
        .massif
        .into_iter()
        .map(convert_coord)
        .collect::<Result<_, _>>()?;
    let surfaces = geometry
        .surfaces
        .into_iter()
        .map(|(coord, level)| convert_coord(coord).map(|coord| (coord, level)))
        .collect::<Result<_, _>>()?;
    let lava = geometry
        .lava
        .into_iter()
        .map(|(position, node)| {
            let position = convert_position(position)?;
            let downstream = node.downstream.map(convert_position).transpose()?;
            Ok((
                position,
                LiquidNode {
                    state: node.state,
                    downstream,
                },
            ))
        })
        .collect::<Result<_, Vec<WorldValidationIssue>>>()?;
    let bridge = geometry
        .bridge
        .into_iter()
        .map(convert_position)
        .collect::<Result<_, _>>()?;
    let stairs = geometry
        .stairs
        .into_iter()
        .map(convert_position)
        .collect::<Result<_, _>>()?;
    let route = ProtectedFeatureRoute {
        centerline: geometry
            .route
            .centerline
            .into_iter()
            .map(convert_position)
            .collect::<Result<_, _>>()?,
        surfaces: geometry
            .route
            .surfaces
            .into_iter()
            .map(convert_position)
            .collect::<Result<_, _>>()?,
    };
    let anchors = geometry
        .anchors
        .into_iter()
        .map(|(name, position)| convert_position(position).map(|position| (name, position)))
        .collect::<Result<_, _>>()?;
    Ok(VolcanoGeometry {
        massif,
        surfaces,
        lava,
        bridge,
        stairs,
        route,
        anchors,
        low_lava_level: geometry.low_lava_level,
    })
}

fn massif_rise(distance: u32, radius: u32, relief: Level) -> Level {
    if distance == 0 {
        return relief.saturating_sub(CRATER_DEPTH);
    }
    if distance == 1 {
        return relief;
    }
    let numerator =
        i64::from(relief.saturating_sub(1)).saturating_mul(i64::from(distance.saturating_sub(1)));
    let denominator = i64::from(radius.saturating_sub(1).max(1));
    relief
        .saturating_sub(i32::try_from(numerator / denominator).unwrap_or(i32::MAX))
        .max(2)
}

fn lava_levels(length: usize, bridge_index: usize, high: Level, low: Level) -> Vec<Level> {
    let descent = bridge_index.max(1);
    let drop = high.saturating_sub(low);
    (0..length)
        .map(|index| {
            if index >= descent {
                return low;
            }
            let stage = index.saturating_mul(DESCENT_STAGES) / descent;
            let stage = i32::try_from(stage.min(DESCENT_STAGES)).unwrap_or_default();
            let stages = i32::try_from(DESCENT_STAGES).unwrap_or(1).max(1);
            high.saturating_sub(drop.saturating_mul(stage) / stages)
        })
        .collect()
}

fn basalt_column(surface: Level) -> VolumeColumn {
    VolumeColumn {
        elements: vec![
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(0, 1),
                material: SolidMaterialRole::Bedrock,
                cutaway_for: None,
            }),
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(1, surface.saturating_add(1)),
                material: SolidMaterialRole::Basalt,
                cutaway_for: None,
            }),
        ],
    }
}

fn lava_column(position: TilePos, node: LiquidNode) -> (VolumeColumn, TilePos) {
    let fill_bottom = if node.state == LiquidFlowState::Fall {
        node.downstream
            .map_or(position.level.saturating_sub(1), |downstream| {
                downstream.level
            })
    } else {
        position.level.saturating_sub(1)
    };
    let bed_level = fill_bottom.saturating_sub(1).max(1);
    let mut elements = vec![VolumeElement::Solid(SolidMass {
        levels: LevelInterval::new(0, 1),
        material: SolidMaterialRole::Bedrock,
        cutaway_for: None,
    })];
    if bed_level > 1 {
        elements.push(VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(1, bed_level),
            material: SolidMaterialRole::Basalt,
            cutaway_for: None,
        }));
    }
    elements.extend([
        VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(bed_level, bed_level.saturating_add(1)),
            material: SolidMaterialRole::Gravel,
            cutaway_for: None,
        }),
        VolumeElement::Fill(NonSolidFill {
            levels: LevelInterval::new(fill_bottom, position.level.saturating_add(1)),
            material: FillMaterialRole::Lava,
        }),
    ]);
    (
        VolumeColumn { elements },
        TilePos::new(position.coord, bed_level),
    )
}

fn volcano_view_hint(
    radius: u32,
    summit_level: Level,
    level_height: f32,
) -> Result<MapViewHint, Vec<WorldValidationIssue>> {
    let radius = u16::try_from(radius)
        .map(f32::from)
        .map_err(|error| vec![recipe_issue(format!("Volcano radius exceeds u16: {error}"))])?;
    let summit = i16::try_from(summit_level)
        .map(f32::from)
        .map_err(|error| vec![recipe_issue(format!("Volcano summit exceeds i16: {error}"))])?;
    let focus_y = summit * level_height * 0.45;
    let hint = MapViewHint::new(
        (
            radius.mul_add(1.5, 7.0),
            focus_y + radius.mul_add(1.0, 10.0),
            radius.mul_add(1.45, 7.0),
        ),
        (-radius * 0.22, focus_y, 0.0),
    );
    hint.is_valid()
        .then_some(hint)
        .ok_or_else(|| vec![recipe_issue("Volcano camera hint is invalid")])
}

/// Revalidates one Volcano fragment against its resolved composite authority.
pub(crate) fn validate_patch(
    patch: PatchRecipeContext<'_>,
    fragment: &GeneratedPatchPlan,
    settings: &V3VolcanoSettings,
) -> WorldValidation<VolcanoMetrics> {
    let mut issues = fragment
        .validate_against(patch.layout())
        .into_iter()
        .map(|issue| {
            recipe_issue(format!(
                "Volcano patch {:?} failed {:?}: {}",
                issue.patch, issue.code, issue.detail
            ))
        })
        .collect::<Vec<_>>();
    issues.extend(validate_patch_walker_seams(&patch, &fragment.volume));
    issues.extend(validate_composite_outlet(&patch, fragment, settings));
    if !issues.is_empty() {
        return WorldValidation::Invalid(issues);
    }
    let frame = match patch.local_frame() {
        Ok(frame) => frame,
        Err(error) => {
            return WorldValidation::Invalid(vec![recipe_issue(format!(
                "Volcano validation frame failed: {error}"
            ))]);
        }
    };
    match frame.canonical_local_world(fragment) {
        Ok(plan) => validate_volcano_inner(&plan, settings, false),
        Err(error) => WorldValidation::Invalid(vec![recipe_issue(format!(
            "Volcano validation projection failed: {error}"
        ))]),
    }
}

fn validate_composite_outlet(
    patch: &PatchRecipeContext<'_>,
    fragment: &GeneratedPatchPlan,
    settings: &V3VolcanoSettings,
) -> Vec<WorldValidationIssue> {
    if !patch.layout().kind.is_composite() {
        return Vec::new();
    }
    let mut issues = Vec::new();
    if !patch.is_world_boundary(HexSide::West) {
        issues.push(recipe_issue(
            "composite Volcano lava requires a western world-boundary outlet",
        ));
    }
    if patch
        .shared_edges()
        .any(|edge| edge.liquid_port().is_some())
    {
        issues.push(recipe_issue(
            "composite Volcano lava must remain separate from stitched liquid ports",
        ));
    }
    let terminals = fragment
        .liquids
        .bodies
        .values()
        .filter(|body| body.material == FillMaterialRole::Lava)
        .flat_map(|body| {
            body.nodes.iter().filter_map(|(position, node)| {
                node.downstream.is_none().then_some((*position, *node))
            })
        })
        .collect::<Vec<_>>();
    let expected_terminals = match expected_composite_terminal_positions(patch, settings) {
        Ok(expected) => Some(expected),
        Err(expected_issues) => {
            issues.extend(expected_issues);
            None
        }
    };
    if terminals.len() != LAVA_LANES.len() {
        issues.push(recipe_issue(format!(
            "composite Volcano has {} lava terminals; expected exactly {}",
            terminals.len(),
            LAVA_LANES.len()
        )));
    }
    let terminal_positions = terminals
        .iter()
        .map(|(position, _)| *position)
        .collect::<BTreeSet<_>>();
    if expected_terminals
        .as_ref()
        .is_some_and(|expected| terminal_positions != *expected)
    {
        issues.push(recipe_issue(format!(
            "composite Volcano terminals do not match the exact contiguous three-lane western outlet positions (actual {terminal_positions:?}, expected {expected_terminals:?})"
        )));
    }
    match ring19_volcano_outlet(patch) {
        Ok(Some(outlet)) => {
            let declared_terminals = outlet
                .lanes
                .iter()
                .map(|(inside, _)| TilePos::new(*inside, outlet.level))
                .collect::<BTreeSet<_>>();
            if outlet.side != HexSide::West
                || outlet.lanes.len() != LAVA_LANES.len()
                || expected_terminals
                    .as_ref()
                    .is_some_and(|expected| declared_terminals != *expected)
            {
                issues.push(recipe_issue(format!(
                "Ring19 Volcano resolved outlet must exactly equal the recipe's three western terminals (declared {declared_terminals:?}, expected {expected_terminals:?})"
            )));
            }
        }
        Ok(None) => {}
        Err(error) => issues.push(recipe_issue(error)),
    }
    for (position, node) in &terminals {
        if node.state != LiquidFlowState::Still {
            issues.push(recipe_issue(format!(
                "composite Volcano terminal {position:?} is not Still"
            )));
        }
        if patch
            .layout()
            .footprint
            .contains(&HexSide::West.neighbor(position.coord))
        {
            issues.push(recipe_issue(format!(
                "composite Volcano terminal {position:?} does not exit the western world boundary"
            )));
        }
    }
    issues
}

fn expected_composite_terminal_positions(
    patch: &PatchRecipeContext<'_>,
    settings: &V3VolcanoSettings,
) -> Result<BTreeSet<TilePos>, Vec<WorldValidationIssue>> {
    let frame = patch.local_frame_with_rotation(0).map_err(|error| {
        vec![recipe_issue(format!(
            "Volcano outlet validation frame failed: {error}"
        ))]
    })?;
    let local_mask = frame.local_mask(patch.mask()).map_err(|error| {
        vec![recipe_issue(format!(
            "Volcano outlet validation mask failed: {error}"
        ))]
    })?;
    let terminals = lava_lane_paths(&local_mask, frame.scale(), 3)?
        .into_iter()
        .map(|path| {
            path.last().copied().ok_or_else(|| {
                vec![recipe_issue(
                    "Volcano authoritative western lava lane is empty",
                )]
            })
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|coord| {
            frame
                .to_world(coord)
                .map(|coord| {
                    TilePos::new(
                        coord,
                        settings
                            .base_level
                            .saturating_add(3)
                            .saturating_sub(settings.bridge_clearance)
                            .max(3),
                    )
                })
                .map_err(|error| {
                    vec![recipe_issue(format!(
                        "Volcano outlet validation conversion failed: {error}"
                    ))]
                })
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    let terminal_coords = terminals
        .iter()
        .map(|position| position.coord)
        .collect::<BTreeSet<_>>();
    let connected = terminal_coords.first().is_some_and(|start| {
        let mut reachable = BTreeSet::from([*start]);
        let mut frontier = vec![*start];
        while let Some(coord) = frontier.pop() {
            for neighbor in coord.neighbors() {
                if terminal_coords.contains(&neighbor) && reachable.insert(neighbor) {
                    frontier.push(neighbor);
                }
            }
        }
        reachable.len() == terminal_coords.len()
    });
    if terminals.len() != LAVA_LANES.len()
        || !connected
        || terminals.iter().any(|position| {
            patch
                .layout()
                .footprint
                .contains(&HexSide::West.neighbor(position.coord))
        })
    {
        return Err(vec![recipe_issue(
            "Volcano authoritative geometry does not yield three contiguous world-West terminal lanes",
        )]);
    }
    Ok(terminals)
}

fn rederive_bridge_deck(
    lava: &LiquidBodyPlan,
    settings: &V3VolcanoSettings,
) -> Result<BTreeSet<TilePos>, Vec<WorldValidationIssue>> {
    let orientation = lava_flow_orientation(lava)?;
    let low_lava_level = settings
        .base_level
        .saturating_add(3)
        .saturating_sub(settings.bridge_clearance)
        .max(3);
    let bridge_level = low_lava_level.saturating_add(settings.bridge_clearance);
    let mut expected = BTreeSet::new();
    for x in BRIDGE_FLOW_ROWS {
        for y in LAVA_LANES {
            let coord = rotate(HexCoord::from_axial(x, y), orientation);
            let matching = lava
                .nodes
                .keys()
                .filter(|position| position.coord == coord)
                .copied()
                .collect::<Vec<_>>();
            let [lava_position] = matching.as_slice() else {
                return Err(vec![recipe_issue(format!(
                    "Volcano exact bridge coordinate {coord:?} has {} lava nodes; expected one",
                    matching.len()
                ))]);
            };
            if lava_position.level != low_lava_level {
                return Err(vec![recipe_issue(format!(
                    "Volcano exact bridge coordinate {coord:?} has lava at level {}; expected {low_lava_level}",
                    lava_position.level
                ))]);
            }
            expected.insert(TilePos::new(coord, bridge_level));
        }
    }
    Ok(expected)
}

fn lava_flow_orientation(lava: &LiquidBodyPlan) -> Result<u8, Vec<WorldValidationIssue>> {
    let mut orientations = BTreeSet::new();
    for (position, node) in &lava.nodes {
        let Some(downstream) = node.downstream else {
            continue;
        };
        let orientation = (0..6).find(|orientation| {
            shift(
                position.coord,
                rotate(HexCoord::from_axial(1, 0), *orientation),
            ) == downstream.coord
        });
        let Some(orientation) = orientation else {
            return Err(vec![recipe_issue(format!(
                "Volcano lava step {position:?} -> {downstream:?} leaves one exact horizontal flow direction"
            ))]);
        };
        orientations.insert(orientation);
    }
    let orientations = orientations.into_iter().collect::<Vec<_>>();
    let [orientation] = orientations.as_slice() else {
        return Err(vec![recipe_issue(
            "Volcano lava does not have one authoritative flow orientation",
        )]);
    };
    Ok(*orientation)
}

pub(crate) fn validate_volcano(
    plan: &GeneratedWorldPlan,
    settings: &V3VolcanoSettings,
) -> WorldValidation<VolcanoMetrics> {
    validate_volcano_inner(plan, settings, true)
}

fn validate_volcano_inner(
    plan: &GeneratedWorldPlan,
    settings: &V3VolcanoSettings,
    validate_common: bool,
) -> WorldValidation<VolcanoMetrics> {
    let mut issues = if validate_common {
        plan.validate()
    } else {
        Vec::new()
    };
    let ordinary = OrdinaryGraph::from_volume(&plan.volume, Some(&plan.blockers));
    let Some(party) = plan.anchors.get(PARTY_START).copied() else {
        issues.push(recipe_issue("Volcano is missing party_start"));
        return WorldValidation::Invalid(issues);
    };
    let Some(hostile) = plan.anchors.get(HOSTILE_START).copied() else {
        issues.push(recipe_issue("Volcano is missing hostile_start"));
        return WorldValidation::Invalid(issues);
    };
    let distances = ordinary.distances_from(party);
    let Some(critical_route_steps) = distances.get(&hostile).copied() else {
        issues.push(recipe_issue(
            "Volcano actor anchors are not connected through the elevated bridge",
        ));
        return WorldValidation::Invalid(issues);
    };
    let levels = distances
        .keys()
        .map(|position| position.level)
        .collect::<BTreeSet<_>>();

    let lava_bodies = plan
        .liquids
        .bodies
        .values()
        .filter(|body| body.material == FillMaterialRole::Lava)
        .collect::<Vec<_>>();
    let [lava] = lava_bodies.as_slice() else {
        issues.push(recipe_issue(format!(
            "Volcano has {} lava bodies; expected exactly one",
            lava_bodies.len()
        )));
        return WorldValidation::Invalid(issues);
    };
    if plan
        .liquids
        .bodies
        .values()
        .any(|body| body.material != FillMaterialRole::Lava)
    {
        issues.push(recipe_issue("Volcano contains a non-lava liquid body"));
    }
    let fall_drops = lava
        .nodes
        .iter()
        .filter_map(|(position, node)| {
            (node.state == LiquidFlowState::Fall)
                .then_some(
                    node.downstream
                        .map(|next| position.level.saturating_sub(next.level)),
                )
                .flatten()
        })
        .collect::<Vec<_>>();
    let maximum_fall_height = fall_drops.iter().copied().max().unwrap_or_default();
    if fall_drops.is_empty() || maximum_fall_height < 2 {
        issues.push(recipe_issue(
            "Volcano lava never forms a descending fall from the crater",
        ));
    }
    let reverse_targets = lava
        .nodes
        .values()
        .filter_map(|node| node.downstream)
        .collect::<BTreeSet<_>>();
    let crater_sources = lava
        .nodes
        .keys()
        .filter(|position| !reverse_targets.contains(position))
        .filter(|position| {
            position.level
                >= settings
                    .base_level
                    .saturating_add(settings.summit_relief)
                    .saturating_sub(CRATER_DEPTH)
        })
        .count();
    if crater_sources != LAVA_LANES.len() {
        issues.push(recipe_issue(format!(
            "Volcano has {crater_sources} elevated crater sources; expected {}",
            LAVA_LANES.len()
        )));
    }

    let expected_low_lava = settings
        .base_level
        .saturating_add(3)
        .saturating_sub(settings.bridge_clearance)
        .max(3);
    let expected_bridge_level = expected_low_lava.saturating_add(settings.bridge_clearance);
    let expected_bridge = match rederive_bridge_deck(lava, settings) {
        Ok(expected) => Some(expected),
        Err(bridge_issues) => {
            issues.extend(bridge_issues);
            None
        }
    };
    let bridge_structures = plan
        .structures
        .by_id
        .values()
        .filter(|structure| structure.kind == StructureKind::Bridge)
        .collect::<Vec<_>>();
    let [bridge] = bridge_structures.as_slice() else {
        issues.push(recipe_issue(format!(
            "Volcano has {} bridge structures; expected one",
            bridge_structures.len()
        )));
        return WorldValidation::Invalid(issues);
    };
    if bridge.voxels.len() != BRIDGE_FLOW_ROWS.len().saturating_mul(LAVA_LANES.len()) {
        issues.push(recipe_issue(format!(
            "Volcano bridge has {} surfaces; expected {}",
            bridge.voxels.len(),
            BRIDGE_FLOW_ROWS.len().saturating_mul(LAVA_LANES.len())
        )));
    }
    let metal_over_lava = plan
        .volume
        .surfaces
        .iter()
        .filter_map(|(surface, metadata)| {
            (metadata.access == SurfaceAccess::Ordinary
                && lava.nodes.keys().any(|lava| lava.coord == surface.coord)
                && solid_material_at(&plan.volume, *surface) == Some(SolidMaterialRole::Metal))
            .then_some(*surface)
        })
        .collect::<BTreeSet<_>>();
    if let Some(expected_bridge) = &expected_bridge {
        if bridge.voxels != *expected_bridge || metal_over_lava != *expected_bridge {
            issues.push(recipe_issue(format!(
                "Volcano bridge does not match the exact oriented 2-by-3 deck authority (structure {:?}, metal {metal_over_lava:?}, expected {expected_bridge:?})",
                bridge.voxels
            )));
        }
    }
    let bridge_clearances = bridge
        .voxels
        .iter()
        .filter_map(|surface| {
            lava.nodes
                .keys()
                .find(|lava| lava.coord == surface.coord)
                .map(|lava| surface.level.saturating_sub(lava.level))
        })
        .collect::<BTreeSet<_>>();
    let bridge_clearance = bridge_clearances.iter().next().copied().unwrap_or_default();
    if bridge_clearance < settings.bridge_clearance {
        issues.push(recipe_issue(format!(
            "Volcano bridge clearance is {bridge_clearance}; expected at least {}",
            settings.bridge_clearance
        )));
    }
    if bridge_clearances != BTreeSet::from([settings.bridge_clearance]) {
        issues.push(recipe_issue(format!(
            "Volcano bridge deck clearances are {bridge_clearances:?}; expected every cell at {}",
            settings.bridge_clearance
        )));
    }
    let Some(route) = plan.features.protected_routes.get(BRIDGE_ROUTE) else {
        issues.push(recipe_issue(
            "Volcano is missing its protected bridge route",
        ));
        return WorldValidation::Invalid(issues);
    };
    if route
        .centerline
        .windows(2)
        .any(|pair| matches!(pair, [from, to] if !ordinary.admits(*from, *to)))
    {
        issues.push(recipe_issue(
            "Volcano bridge stairs contain a transition taller than one level",
        ));
    }
    let centerline_surfaces = route.centerline.iter().copied().collect::<BTreeSet<_>>();
    let second_lane = route
        .surfaces
        .difference(&centerline_surfaces)
        .copied()
        .collect::<BTreeSet<_>>();
    let expected_stair_steps = expected_bridge_level.saturating_sub(settings.base_level);
    let expected_centerline = usize::try_from(
        expected_stair_steps
            .saturating_add(1)
            .saturating_mul(2)
            .saturating_add(1),
    )
    .unwrap_or(usize::MAX);
    if route.centerline.len() != expected_centerline
        || route.surfaces.len() != expected_centerline.saturating_mul(2)
        || centerline_surfaces.len() != route.centerline.len()
        || !centerline_surfaces.is_subset(&route.surfaces)
        || second_lane.len() != route.centerline.len()
        || !surface_set_connected(&ordinary, &second_lane)
        || route.centerline.iter().any(|center| {
            !second_lane
                .iter()
                .any(|surface| ordinary.admits(*center, *surface))
        })
    {
        issues.push(recipe_issue(
            "Volcano bridge route is not an exact two-wide ordinary stair approach",
        ));
    }
    if ordinary
        .reachable_avoiding(party, &bridge.voxels)
        .contains(&hostile)
    {
        issues.push(recipe_issue(
            "Volcano retains an ordinary ford or dry passage around its sole bridge",
        ));
    }
    let stair_structures = plan
        .structures
        .by_id
        .values()
        .filter(|structure| structure.kind == StructureKind::Stair)
        .collect::<Vec<_>>();
    let expected_stairs = plan
        .volume
        .surfaces
        .iter()
        .filter_map(|(surface, metadata)| {
            (metadata.access == SurfaceAccess::Ordinary
                && solid_material_at(&plan.volume, *surface)
                    == Some(SolidMaterialRole::WorkedStone))
            .then_some(*surface)
        })
        .collect::<BTreeSet<_>>();
    let expected_stair_count =
        usize::try_from(expected_stair_steps.saturating_sub(1).saturating_mul(4))
            .unwrap_or(usize::MAX);
    if expected_stairs.len() != expected_stair_count {
        issues.push(recipe_issue(format!(
            "Volcano rederived {} worked-stone stair surfaces; expected exactly {expected_stair_count}",
            expected_stairs.len(),
        )));
    }
    match stair_structures.as_slice() {
        [stairs] => {
            if stairs.voxels != expected_stairs {
                issues.push(recipe_issue(format!(
                    "Volcano stair structure does not exactly match rederived worked-stone surfaces (structure {:?}, expected {expected_stairs:?})",
                    stairs.voxels
                )));
            }
            if !stairs.voxels.is_subset(&route.surfaces)
                || !stairs.voxels.iter().all(|stair| {
                    bridge.voxels.iter().any(|deck| {
                        ordinary
                            .distances_from(*stair)
                            .get(deck)
                            .is_some_and(|distance| {
                                *distance
                                    <= settings.bridge_clearance.unsigned_abs().saturating_add(2)
                            })
                    })
                })
            {
                issues.push(recipe_issue(
                    "Volcano stair structure leaves its protected two-wide bridge approach",
                ));
            }
        }
        _ => issues.push(recipe_issue(format!(
            "Volcano has {} stair structures; expected exactly one",
            stair_structures.len()
        ))),
    }
    let structure_route = bridge
        .voxels
        .union(&expected_stairs)
        .copied()
        .collect::<BTreeSet<_>>();
    let landings = route
        .surfaces
        .difference(&structure_route)
        .copied()
        .collect::<BTreeSet<_>>();
    if !structure_route.is_subset(&route.surfaces)
        || landings.len() != 4
        || landings.iter().any(|surface| {
            surface.level != settings.base_level
                || solid_material_at(&plan.volume, *surface) != Some(SolidMaterialRole::Basalt)
        })
    {
        issues.push(recipe_issue(
            "Volcano protected route does not exactly equal its deck, paired stairs, and four basalt landings",
        ));
    }

    let lava_coords = lava
        .nodes
        .keys()
        .map(|position| position.coord)
        .collect::<BTreeSet<_>>();
    let dry_massif = plan
        .volume
        .surfaces
        .keys()
        .filter_map(|position| {
            (position.level >= settings.base_level.saturating_add(2)
                && !lava_coords.contains(&position.coord)
                && solid_material_at(&plan.volume, *position) == Some(SolidMaterialRole::Basalt))
            .then_some(position.coord)
        })
        .collect::<BTreeSet<_>>();
    let lava_massif = lava_coords
        .iter()
        .copied()
        .filter(|coord| {
            coord
                .neighbors()
                .into_iter()
                .any(|neighbor| dry_massif.contains(&neighbor))
        })
        .collect::<BTreeSet<_>>();
    let massif_surfaces = dry_massif.union(&lava_massif).count();
    let massif_coverage_percent = percent(massif_surfaces, plan.layout.footprint.len());
    if !(20..=30).contains(&massif_coverage_percent) {
        issues.push(recipe_issue(format!(
            "Volcano massif covers {massif_coverage_percent}% of the patch; expected 20 through 30%"
        )));
    }
    let summit = plan
        .volume
        .surfaces
        .keys()
        .filter_map(|position| {
            (position.level >= settings.base_level.saturating_add(2)
                && solid_material_at(&plan.volume, *position) == Some(SolidMaterialRole::Basalt))
            .then_some(position.level)
        })
        .max()
        .unwrap_or_default();
    let summit_relief = summit.saturating_sub(settings.base_level);
    if summit_relief != settings.summit_relief {
        issues.push(recipe_issue(format!(
            "Volcano summit relief is {summit_relief}; expected {}",
            settings.summit_relief
        )));
    }
    if !issues.is_empty() {
        return WorldValidation::Invalid(issues);
    }
    WorldValidation::Valid(VolcanoMetrics {
        ordinary_surfaces: count_u32(distances.len()),
        reachable_elevation_levels: count_u32(levels.len()),
        summit_relief,
        massif_surfaces: count_u32(massif_surfaces),
        massif_coverage_percent,
        lava_nodes: count_u32(lava.nodes.len()),
        fall_nodes: count_u32(fall_drops.len()),
        maximum_fall_height,
        bridge_surfaces: count_u32(bridge.voxels.len()),
        bridge_clearance,
        critical_route_steps,
    })
}

fn solid_material_at(volume: &VolumePlan, position: TilePos) -> Option<SolidMaterialRole> {
    volume
        .columns
        .get(&position.coord)?
        .elements
        .iter()
        .find_map(|element| match element {
            VolumeElement::Solid(solid)
                if solid.levels.bottom <= position.level && position.level < solid.levels.top =>
            {
                Some(solid.material)
            }
            VolumeElement::Solid(_) | VolumeElement::Fill(_) => None,
        })
}

fn surface_set_connected(graph: &OrdinaryGraph, surfaces: &BTreeSet<TilePos>) -> bool {
    let Some(start) = surfaces.first().copied() else {
        return false;
    };
    let mut visited = BTreeSet::from([start]);
    let mut frontier = vec![start];
    while let Some(current) = frontier.pop() {
        for neighbor in surfaces {
            if !visited.contains(neighbor)
                && graph.admits(current, *neighbor)
                && graph.admits(*neighbor, current)
            {
                visited.insert(*neighbor);
                frontier.push(*neighbor);
            }
        }
    }
    visited.len() == surfaces.len()
}

fn recipe_issue(detail: impl Into<String>) -> WorldValidationIssue {
    WorldValidationIssue::new(WorldIssueCode::Recipe("volcano"), detail)
}

fn percent(part: usize, total: usize) -> u32 {
    if total == 0 {
        return 0;
    }
    count_u32(part)
        .saturating_mul(100)
        .checked_div(count_u32(total))
        .unwrap_or_default()
}

fn count_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn rotate(coord: HexCoord, turns: u8) -> HexCoord {
    let [mut x, mut y, mut z] = coord.to_cubic_array();
    for _ in 0..turns % 6 {
        (x, y, z) = (-z, -x, -y);
    }
    HexCoord::new_cubic(x, y, z)
}

fn unrotate(coord: HexCoord, turns: u8) -> HexCoord {
    rotate(coord, (6_u8.saturating_sub(turns % 6)) % 6)
}

fn shift(coord: HexCoord, delta: HexCoord) -> HexCoord {
    let [x, y, z] = coord.to_cubic_array();
    let [dx, dy, dz] = delta.to_cubic_array();
    HexCoord::new_cubic(x + dx, y + dy, z + dz)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::procedural_v3::layout::{
        LayoutKind, ResolvedEdgeContract, ResolvedEdgeId, ResolvedEdgeReference,
        ResolvedElevationBand, ResolvedLayoutPlan, ResolvedLiquidElevation, ResolvedLiquidPort,
        ResolvedPatch, ResolvedPort, ResolvedWalkerPorts,
    };
    use crate::settings::{
        PatchEdgeContractSettings, PatchEdgesSettings, PatchMaskSettings, PatchSpec,
    };

    const HERO_SEED: u64 = 444_211_238;

    fn settings(radius: u32) -> ProceduralV3Settings {
        let boundary = || PatchEdgeContractSettings::WorldBoundary;
        let _ = radius;
        ProceduralV3Settings {
            layout: V3LayoutSettings::Single(PatchSpec {
                environment: V3EnvironmentSettings::Volcanic,
                recipe: V3RecipeSettings::Volcano(V3VolcanoSettings {
                    base_level: 15,
                    summit_relief: 20,
                    massif_coverage_percent: 25,
                    bridge_clearance: 4,
                }),
                overlays: Vec::new(),
                mask: PatchMaskSettings::WholeWorld,
                edges: PatchEdgesSettings {
                    east: boundary(),
                    south_east: boundary(),
                    south_west: boundary(),
                    west: boundary(),
                    north_west: boundary(),
                    north_east: boundary(),
                },
            }),
        }
    }

    #[test]
    fn native_volcano_is_deterministic_tall_and_bridge_only() {
        let settings = settings(12);
        let first = generate(12, 0.4, &settings, HERO_SEED).expect("valid Volcano");
        let second = generate(12, 0.4, &settings, HERO_SEED).expect("same valid Volcano");
        assert_eq!(
            first.validated.semantic_fingerprint,
            second.validated.semantic_fingerprint
        );
        assert_eq!(
            first.validated.semantic_fingerprint,
            6_901_546_631_227_104_688
        );
        assert_eq!(first.metrics, second.metrics);
        assert!(!first.used_fallback, "{:#?}", first.notes);
        assert_eq!(first.valid_candidates, 8);
        assert_eq!(first.metrics.summit_relief, 20);
        assert!((20..=30).contains(&first.metrics.massif_coverage_percent));
        assert_eq!(first.metrics.bridge_surfaces, 6);
        assert!(first.metrics.bridge_clearance >= 4);
        assert!(first.metrics.fall_nodes >= 3);
        assert!(first.metrics.maximum_fall_height >= 2);
        let plan = &first.validated.plan;
        assert!(plan
            .liquids
            .bodies
            .values()
            .all(|body| body.material == FillMaterialRole::Lava));
        assert_eq!(
            plan.structures
                .by_id
                .values()
                .filter(|structure| structure.kind == StructureKind::Bridge)
                .count(),
            1
        );
        assert_eq!(plan.features.protected_routes.len(), 1);
    }

    #[test]
    fn validator_rejects_moved_and_reshaped_metal_bridge_decks() {
        let settings = settings(12);
        let selected = generate(12, 0.4, &settings, HERO_SEED).expect("valid Volcano");
        let V3LayoutSettings::Single(spec) = &settings.layout else {
            unreachable!("fixture is Single");
        };
        let V3RecipeSettings::Volcano(volcano) = &spec.recipe else {
            unreachable!("fixture is Volcano");
        };
        let lava = selected
            .validated
            .plan
            .liquids
            .bodies
            .values()
            .find(|body| body.material == FillMaterialRole::Lava)
            .expect("lava body");
        let orientation = lava_flow_orientation(lava).expect("one flow orientation");
        let shapes: [(&str, &[(i32, i32)]); 4] = [
            (
                "moved 2-by-3",
                &[(2, -1), (2, 0), (2, 1), (3, -1), (3, 0), (3, 1)],
            ),
            (
                "reshaped",
                &[(2, -1), (2, 0), (2, 1), (3, 0), (3, 1), (4, 1)],
            ),
            ("T", &[(2, -1), (2, 0), (2, 1), (3, 0), (4, 0), (5, 0)]),
            ("1-by-6", &[(2, 0), (3, 0), (4, 0), (5, 0), (6, 0), (7, 0)]),
        ];
        for (name, local_shape) in shapes {
            let coords = local_shape
                .iter()
                .map(|(x, y)| rotate(HexCoord::from_axial(*x, *y), orientation))
                .collect::<BTreeSet<_>>();
            let mut corrupted = selected.validated.plan.clone();
            replace_bridge_deck(&mut corrupted, &coords);
            let WorldValidation::Invalid(issues) = validate_volcano(&corrupted, volcano) else {
                panic!("{name} bridge corruption unexpectedly validated");
            };
            assert!(
                issues.iter().any(|issue| issue
                    .detail
                    .contains("exact oriented 2-by-3 deck authority")),
                "{name} corruption did not reach the exact bridge authority: {issues:?}"
            );
        }
    }

    fn replace_bridge_deck(plan: &mut GeneratedWorldPlan, coords: &BTreeSet<HexCoord>) {
        let bridge_id = plan
            .structures
            .by_id
            .iter()
            .find_map(|(id, structure)| (structure.kind == StructureKind::Bridge).then_some(*id))
            .expect("bridge structure");
        let old_deck = plan
            .structures
            .by_id
            .get(&bridge_id)
            .expect("bridge structure")
            .voxels
            .clone();
        let bridge_level = old_deck
            .first()
            .map(|position| position.level)
            .expect("bridge deck level");
        for surface in &old_deck {
            let metal = plan
                .volume
                .columns
                .get_mut(&surface.coord)
                .expect("old bridge column")
                .elements
                .iter_mut()
                .find_map(|element| match element {
                    VolumeElement::Solid(solid)
                        if solid.material == SolidMaterialRole::Metal
                            && solid.levels.bottom <= surface.level
                            && surface.level < solid.levels.top =>
                    {
                        Some(solid)
                    }
                    _ => None,
                })
                .expect("old bridge metal");
            metal.material = SolidMaterialRole::Basalt;
        }
        let biome = plan
            .biome_regions
            .values()
            .next()
            .copied()
            .expect("Volcano biome");
        let new_deck = coords
            .iter()
            .copied()
            .map(|coord| {
                let position = TilePos::new(coord, bridge_level);
                assert!(!plan.volume.surfaces.contains_key(&position));
                plan.volume
                    .columns
                    .get_mut(&coord)
                    .expect("replacement bridge lava column")
                    .elements
                    .push(VolumeElement::Solid(SolidMass {
                        levels: LevelInterval::new(bridge_level, bridge_level.saturating_add(1)),
                        material: SolidMaterialRole::Metal,
                        cutaway_for: None,
                    }));
                assert!(plan
                    .volume
                    .surfaces
                    .insert(
                        position,
                        SurfaceMetadata {
                            access: SurfaceAccess::Ordinary,
                            interior: None,
                        },
                    )
                    .is_none());
                assert!(plan.biome_regions.insert(position, biome).is_none());
                position
            })
            .collect::<BTreeSet<_>>();
        plan.structures
            .by_id
            .get_mut(&bridge_id)
            .expect("bridge structure")
            .voxels = new_deck;
    }

    #[test]
    fn forced_candidates_use_an_independent_valid_fallback() {
        let settings = settings(12);
        let first = generate_inner(12, 0.4, &settings, 1, true)
            .expect("forced Volcano fallback should validate");
        let second = generate_inner(12, 0.4, &settings, 9_999, true)
            .expect("Volcano fallback should ignore seed state");
        assert!(first.used_fallback);
        assert_eq!(first.candidates_evaluated, 8);
        assert_eq!(
            first.validated.semantic_fingerprint,
            second.validated.semantic_fingerprint
        );
    }

    #[test]
    fn supported_radii_keep_exact_contracts() {
        for radius in [12, 20, 40] {
            let selected = generate(radius, 0.4, &settings(radius), HERO_SEED)
                .unwrap_or_else(|error| panic!("radius {radius} Volcano failed: {error}"));
            assert!(!selected.used_fallback);
            assert_eq!(selected.valid_candidates, 8);
            assert_eq!(selected.metrics.summit_relief, 20);
            assert!((20..=30).contains(&selected.metrics.massif_coverage_percent));
            assert_eq!(selected.metrics.bridge_surfaces, 6);
            assert!(selected.metrics.bridge_clearance >= 4);
        }
    }

    #[test]
    fn radius_12_pr_corpus_validates_128_seeds_and_named_regression() {
        let settings = settings(12);
        let mut seeds = (0..128_u64).collect::<BTreeSet<_>>();
        seeds.insert(HERO_SEED);
        for seed in seeds {
            let selected = generate(12, 0.4, &settings, seed)
                .unwrap_or_else(|error| panic!("Volcano seed {seed}: {error}"));
            assert!(
                !selected.used_fallback,
                "Volcano seed {seed} used fallback: {:#?}",
                selected.notes
            );
            assert_eq!(
                selected.valid_candidates, 8,
                "Volcano seed {seed} rejected an ordinary candidate"
            );
        }
    }

    #[test]
    fn composite_volcano_exits_the_western_boundary_without_a_liquid_seam() {
        let mask = HexCoord::ORIGIN
            .within_radius(12)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let edges = HexSide::ALL
            .into_iter()
            .map(|side| (side, ResolvedEdgeReference::WorldBoundary))
            .collect();
        let layout = ResolvedLayoutPlan {
            kind: LayoutKind::Ring7,
            grid_radius: 33,
            footprint: mask.clone(),
            patches: BTreeMap::from([(
                PatchId(0),
                ResolvedPatch {
                    biome_region: hex_core::BiomeRegionId(0),
                    rotation_turns: 0,
                    mask: mask.clone(),
                    edges,
                },
            )]),
            shared_edges: BTreeMap::new(),
            boundary_liquid_outlets: BTreeMap::new(),
        };
        let patch = PatchRecipeContext::resolve(&layout, PatchId(0)).expect("fixture patch");
        let orientation = volcano_orientation(&patch, 1).expect("western outlet");
        assert_eq!(orientation, 3);

        let fixture = settings(12);
        let V3LayoutSettings::Single(spec) = &fixture.layout else {
            unreachable!("fixture is Single");
        };
        let V3RecipeSettings::Volcano(volcano) = &spec.recipe else {
            unreachable!("fixture is Volcano");
        };
        let geometry =
            plan_geometry(&mask, 12, volcano, orientation, None).expect("valid geometry");
        let terminals = geometry
            .lava
            .iter()
            .filter_map(|(position, node)| node.downstream.is_none().then_some(position.coord))
            .collect::<BTreeSet<_>>();
        assert_eq!(terminals.len(), LAVA_LANES.len());
        assert!(terminals
            .iter()
            .all(|coord| !mask.contains(&HexSide::West.neighbor(*coord))));
    }

    #[test]
    fn ring19_volcano_consumes_the_exact_resolved_western_outlet() {
        let mask = HexCoord::ORIGIN
            .within_radius(12)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let fixture = settings(12);
        let V3LayoutSettings::Single(spec) = &fixture.layout else {
            unreachable!("fixture is Single");
        };
        let V3RecipeSettings::Volcano(volcano) = &spec.recipe else {
            unreachable!("fixture is Volcano");
        };
        let geometry = plan_geometry(&mask, 12, volcano, 3, None).expect("valid geometry");
        let terminals = geometry
            .lava
            .iter()
            .filter_map(|(position, node)| node.downstream.is_none().then_some(*position))
            .collect::<BTreeSet<_>>();
        let level = terminals
            .first()
            .map(|terminal| terminal.level)
            .expect("three terminals");
        assert!(terminals.iter().all(|terminal| terminal.level == level));
        let lanes = terminals
            .iter()
            .map(|terminal| (terminal.coord, HexSide::West.neighbor(terminal.coord)))
            .collect::<BTreeSet<_>>();
        let edges = HexSide::ALL
            .into_iter()
            .map(|side| (side, ResolvedEdgeReference::WorldBoundary))
            .collect();
        let mut layout = ResolvedLayoutPlan {
            kind: LayoutKind::Ring19,
            grid_radius: 55,
            footprint: mask.clone(),
            patches: BTreeMap::from([(
                PatchId(0),
                ResolvedPatch {
                    biome_region: hex_core::BiomeRegionId(0),
                    rotation_turns: 3,
                    mask: mask.clone(),
                    edges,
                },
            )]),
            shared_edges: BTreeMap::new(),
            boundary_liquid_outlets: BTreeMap::from([(
                (PatchId(0), HexSide::West),
                ResolvedBoundaryLiquidOutlet {
                    source: PatchId(0),
                    side: HexSide::West,
                    lanes,
                    inward_approach: terminals.iter().map(|terminal| terminal.coord).collect(),
                    approach_depth: 1,
                    level,
                },
            )]),
        };
        let fragment = GeneratedPatchPlan {
            patch_id: PatchId(0),
            volume: VolumePlan::new(mask),
            liquids: LiquidPlan {
                bodies: BTreeMap::from([(
                    LiquidBodyId(0),
                    LiquidBodyPlan {
                        material: FillMaterialRole::Lava,
                        nodes: geometry.lava,
                    },
                )]),
            },
            features: FeaturePlan::default(),
            structures: StructurePlan::default(),
            blockers: BTreeSet::new(),
            lights: BTreeMap::new(),
            biome_regions: BTreeMap::new(),
            interiors: InteriorPlan::default(),
            anchors: BTreeMap::new(),
            view_hint: MapViewHint::new((1.0, 1.0, 1.0), (0.0, 0.0, 0.0)),
        };
        let patch = PatchRecipeContext::resolve(&layout, PatchId(0)).expect("fixture patch");
        assert_eq!(volcano_orientation(&patch, 1), Ok(3));
        assert_eq!(
            validate_composite_outlet(&patch, &fragment, volcano),
            Vec::new()
        );

        layout
            .boundary_liquid_outlets
            .values_mut()
            .next()
            .expect("one outlet")
            .level = level.saturating_add(1);
        let patch = PatchRecipeContext::resolve(&layout, PatchId(0)).expect("fixture patch");
        assert!(validate_composite_outlet(&patch, &fragment, volcano)
            .iter()
            .any(|issue| issue.detail.contains("must exactly equal")));

        layout
            .boundary_liquid_outlets
            .values_mut()
            .next()
            .expect("one outlet")
            .lanes
            .pop_last();
        let patch = PatchRecipeContext::resolve(&layout, PatchId(0)).expect("fixture patch");
        assert!(volcano_orientation(&patch, 1)
            .expect_err("two lanes must fail")
            .iter()
            .any(|issue| issue.detail.contains("three-lane")));
    }

    #[test]
    fn composite_volcano_rejects_a_non_western_outlet() {
        let mask = HexCoord::ORIGIN
            .within_radius(12)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let mut edges = HexSide::ALL
            .into_iter()
            .map(|side| (side, ResolvedEdgeReference::WorldBoundary))
            .collect::<BTreeMap<_, _>>();
        edges.insert(
            HexSide::West,
            ResolvedEdgeReference::Shared(ResolvedEdgeId(0)),
        );
        let layout = ResolvedLayoutPlan {
            kind: LayoutKind::Ring7,
            grid_radius: 33,
            footprint: mask.clone(),
            patches: BTreeMap::from([(
                PatchId(0),
                ResolvedPatch {
                    biome_region: hex_core::BiomeRegionId(0),
                    rotation_turns: 0,
                    mask,
                    edges,
                },
            )]),
            shared_edges: BTreeMap::new(),
            boundary_liquid_outlets: BTreeMap::new(),
        };
        let patch = PatchRecipeContext::resolve(&layout, PatchId(0)).expect("fixture patch");
        let issues = volcano_orientation(&patch, 1).expect_err("western seam must be rejected");
        assert!(issues
            .iter()
            .any(|issue| issue.detail.contains("western world-boundary outlet")));
    }

    #[test]
    fn composite_volcano_rejects_stitched_lava() {
        let mask = HexCoord::ORIGIN
            .within_radius(12)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let edge_id = ResolvedEdgeId(0);
        let mut edges = HexSide::ALL
            .into_iter()
            .map(|side| (side, ResolvedEdgeReference::WorldBoundary))
            .collect::<BTreeMap<_, _>>();
        edges.insert(HexSide::East, ResolvedEdgeReference::Shared(edge_id));
        let port = ResolvedPort {
            lanes: BTreeSet::new(),
            first_approach: BTreeSet::new(),
            second_approach: BTreeSet::new(),
        };
        let layout = ResolvedLayoutPlan {
            kind: LayoutKind::Ring7,
            grid_radius: 33,
            footprint: mask.clone(),
            patches: BTreeMap::from([(
                PatchId(0),
                ResolvedPatch {
                    biome_region: hex_core::BiomeRegionId(0),
                    rotation_turns: 0,
                    mask,
                    edges,
                },
            )]),
            shared_edges: BTreeMap::from([(
                edge_id,
                ResolvedEdgeContract {
                    first: (PatchId(0), HexSide::East),
                    second: (PatchId(1), HexSide::West),
                    elevation: ResolvedElevationBand {
                        preferred: 15,
                        min: 15,
                        max: 15,
                    },
                    walker: ResolvedWalkerPorts {
                        count: 0,
                        width: 0,
                        ports: Vec::new(),
                    },
                    liquid: ResolvedLiquidPort::Directed {
                        source: PatchId(0),
                        sink: PatchId(1),
                        port,
                        elevation: ResolvedLiquidElevation::EdgeBand,
                    },
                    approach_depth: 0,
                    boundary_pairs: BTreeSet::new(),
                    protected_approaches: BTreeMap::new(),
                },
            )]),
            boundary_liquid_outlets: BTreeMap::new(),
        };
        let patch = PatchRecipeContext::resolve(&layout, PatchId(0)).expect("fixture patch");
        let issues = volcano_orientation(&patch, 1).expect_err("stitched lava must be rejected");
        assert!(issues
            .iter()
            .any(|issue| issue.detail.contains("separate from stitched liquid ports")));
    }
}
