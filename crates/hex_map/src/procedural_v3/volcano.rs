//! Native V3 Volcano recipe.
//!
//! The massif, lava topology, elevated crossing, and stair approaches are planned
//! together. Lava never becomes traversable footing, and the bridge remains the
//! sole ordinary crossing through the three-wide massif-to-boundary barrier.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use hex_core::{HexCoord, Level, MapViewHint, TilePos};

use super::composition::{compose_single_patch, GeneratedPatchPlan};
use super::layout::{resolve_layout, PatchId, ResolvedLayoutPlan};
use super::liquid::{LiquidBodyId, LiquidBodyPlan, LiquidFlowState, LiquidNode, LiquidPlan};
use super::local_frame::LocalPatchFrame;
use super::patch::{PatchBuildMode, PatchRecipeContext};
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
    let frame = LocalPatchFrame::resolve(patch.mask(), patch.layout().kind, patch.grid_radius())
        .map_err(|error| vec![recipe_issue(format!("Volcano local frame failed: {error}"))])?;
    let local_mask = frame.local_mask(patch.mask()).map_err(|error| {
        vec![recipe_issue(format!(
            "Volcano local mask conversion failed: {error}"
        ))]
    })?;
    let orientation = streams.map_or(0, |streams| {
        u8::try_from(streams.orientation.sample(0) % 6).unwrap_or_default()
    });
    let geometry = plan_geometry(
        &local_mask,
        frame.scale(),
        settings,
        orientation,
        streams.map(|streams| streams.massif),
    )?;
    let geometry = geometry_to_world(geometry, frame)?;
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

        let level = geometry
            .surfaces
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
    let volume = VolumePlan {
        mask: patch.mask().clone(),
        columns,
        surfaces,
    };
    let biome_regions = volume
        .surfaces
        .keys()
        .copied()
        .map(|surface| (surface, patch.biome_region()))
        .collect();
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
    let view_hint = frame.view_hint_to_world(volcano_view_hint(
        frame.scale(),
        settings.base_level.saturating_add(settings.summit_relief),
        level_height,
    )?);
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
        anchors: geometry.anchors,
        view_hint,
    };
    let issues = fragment
        .validate_against(patch.layout())
        .into_iter()
        .map(|issue| {
            recipe_issue(format!(
                "Volcano patch {:?} failed {:?}: {}",
                issue.patch, issue.code, issue.detail
            ))
        })
        .collect::<Vec<_>>();
    if issues.is_empty() {
        Ok(fragment)
    } else {
        Err(issues)
    }
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

    let east_delta = rotate(HexCoord::from_axial(1, 0), orientation);
    let mut lanes = Vec::new();
    for lane in LAVA_LANES {
        let mut path = Vec::new();
        let mut coord = rotate(HexCoord::from_axial(crater_x, lane), orientation);
        while mask.contains(&coord) {
            path.push(coord);
            let next = shift(coord, east_delta);
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

pub(crate) fn validate_volcano(
    plan: &GeneratedWorldPlan,
    settings: &V3VolcanoSettings,
) -> WorldValidation<VolcanoMetrics> {
    let mut issues = plan.validate();
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
    let bridge_clearance = bridge
        .voxels
        .iter()
        .filter_map(|surface| {
            lava.nodes
                .keys()
                .find(|lava| lava.coord == surface.coord)
                .map(|lava| surface.level.saturating_sub(lava.level))
        })
        .min()
        .unwrap_or_default();
    if bridge_clearance < settings.bridge_clearance {
        issues.push(recipe_issue(format!(
            "Volcano bridge clearance is {bridge_clearance}; expected at least {}",
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
    if ordinary
        .reachable_avoiding(party, &bridge.voxels)
        .contains(&hostile)
    {
        issues.push(recipe_issue(
            "Volcano retains an ordinary ford or dry passage around its sole bridge",
        ));
    }
    if plan
        .structures
        .by_id
        .values()
        .filter(|structure| structure.kind == StructureKind::Stair)
        .count()
        != 1
    {
        issues.push(recipe_issue(
            "Volcano requires one exact paired stair structure",
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
        assert_eq!(first.metrics, second.metrics);
        assert!(!first.used_fallback, "{:#?}", first.notes);
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
        }
    }
}
