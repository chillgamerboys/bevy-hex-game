//! Native V3 rolling-hills geometry.
//!
//! Height cones are one-Lipschitz, so the generated ordinary surface remains
//! walker-connected by construction. Shared-edge approaches clamp those cones to
//! the resolved seam datum without a post-generation blend pass.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use hex_assets::RuntimeArtCatalog;
use hex_core::{HexCoord, Level, MapViewHint, TilePos};

use super::composition::{compose_single_patch, GeneratedPatchPlan};
use super::layout::{resolve_layout, HexSide, PatchId, ResolvedEdgeId, ResolvedLayoutPlan};
use super::liquid::{LiquidBodyId, LiquidBodyPlan, LiquidFlowState, LiquidNode, LiquidPlan};
use super::local_frame::LocalPatchFrame;
use super::patch::{PatchBuildMode, PatchRecipeContext};
use super::seam::{shape_walker_seams, validate_patch_walker_seams, WalkerSeamShape};
use super::seed::SeedStream;
use super::selection::{
    run_recipe, CandidateAttemptError, CandidateContext, FallbackContext, RepairOutcome, V3Recipe,
    ValidatedWorldSelection, WorldValidation,
};
use super::traversal::OrdinaryGraph;
use super::vegetation::{
    append_landform_vegetation, landform_vegetation_metrics, validate_landform_vegetation,
    LandformVegetationDomain, LandformVegetationSet,
};
use super::volume::{
    FillMaterialRole, LevelInterval, NonSolidFill, SolidMass, SolidMaterialRole, SurfaceAccess,
    SurfaceMetadata, VolumeColumn, VolumeElement, VolumePlan,
};
use super::world::{
    FeatureKind, FeaturePlan, GeneratedWorldPlan, InteriorPlan, PlannedStructure,
    ProtectedFeatureRoute, StructureId, StructureKind, StructurePlan, WorldIssueCode,
    WorldValidationIssue,
};
use super::V3GenerationError;
use crate::settings::{
    ProceduralV3Settings, V3EnvironmentSettings, V3HillsSettings, V3LayoutSettings,
    V3RecipeSettings,
};

const PARTY_START: &str = "party_start";
const HOSTILE_START: &str = "hostile_start";
const CONFLICT_CENTER: &str = "conflict_center";
const BRIDGE_ANCHOR: &str = "bridge";
const BRIDGE_ROUTE: &str = "bridge_crossing";
const FORD_ROUTE: &str = "alternate_crossing";
const RIVER_HALF_WIDTH: i32 = 1;
const CROSSING_HALF_LENGTH: i32 = 2;
const APPROACH_HALF_LENGTH: i32 = 3;
const RIVER_DEPTH: Level = 3;
const SMALL_FALL_HEIGHT: Level = 3;
const FROZEN_ICE_CAP_TARGET: usize = 5;
const HILLS_TREE_TARGET: usize = 3;
const HILLS_GRASS_PERCENT: usize = 70;

/// Deterministic measurements for one admitted Hills plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HillsMetrics {
    pub(crate) ordinary_surfaces: u32,
    pub(crate) reachable_elevation_levels: u32,
    pub(crate) relief: Level,
    pub(crate) critical_route_steps: u32,
    pub(crate) hill_centres: u32,
    pub(crate) barrier_cells: u32,
    pub(crate) bridge_surfaces: u32,
    pub(crate) alternate_crossing_surfaces: u32,
    pub(crate) tree_roots: u32,
    pub(crate) grass_roots: u32,
    pub(crate) valley_grass_percent: u32,
}

#[derive(Debug)]
struct HillsRecipe {
    level_height: f32,
    layout: ResolvedLayoutPlan,
    settings: V3HillsSettings,
    environment: V3EnvironmentSettings,
    vegetation: Option<LandformVegetationSet>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct HillsStreams<'a> {
    pub(super) orientation: SeedStream<'a>,
    pub(super) centres: SeedStream<'a>,
    pub(super) trees: SeedStream<'a>,
    pub(super) grass: SeedStream<'a>,
}

/// Runs the common eight-candidate selector for one native V3 Hills world.
pub(crate) fn generate(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV3Settings,
    seed: u64,
    catalog: &RuntimeArtCatalog,
) -> Result<ValidatedWorldSelection<HillsMetrics>, V3GenerationError> {
    if !level_height.is_finite() || level_height <= 0.0 {
        return Err(V3GenerationError::RecipeContract(
            "Hills level height must be positive and finite".to_owned(),
        ));
    }
    let (hills, environment) = recipe_settings(settings)?;
    let layout = resolve_layout(grid_radius, settings)
        .map_err(|error| V3GenerationError::RecipeContract(error.to_string()))?;
    let vegetation = if matches!(
        environment,
        V3EnvironmentSettings::TemperateGrassland | V3EnvironmentSettings::Frozen
    ) {
        Some(
            LandformVegetationSet::resolve(catalog, environment, "Hills")
                .map_err(V3GenerationError::RecipeContract)?,
        )
    } else {
        None
    };
    run_recipe(
        &HillsRecipe {
            level_height,
            layout,
            settings: hills.clone(),
            environment,
            vegetation,
        },
        settings,
        grid_radius,
        seed,
    )
}

impl V3Recipe for HillsRecipe {
    type Settings = ProceduralV3Settings;
    type Metrics = HillsMetrics;
    type Score = (u32, u32, u8);

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
            self.environment,
            self.level_height,
            PatchBuildMode::Candidate {
                world_seed: context.seed,
                candidate: context.candidate,
            },
            self.vegetation.as_ref(),
        )
        .map_err(CandidateAttemptError::Rejected)?;
        compose_single_patch(self.layout.clone(), fragment).map_err(|error| {
            CandidateAttemptError::Fatal(V3GenerationError::RecipeContract(format!(
                "Hills single-patch composition failed: {error:?}"
            )))
        })
    }

    fn validate(
        &self,
        _settings: &Self::Settings,
        plan: &GeneratedWorldPlan,
    ) -> WorldValidation<Self::Metrics> {
        validate_hills(
            plan,
            &self.settings,
            self.environment,
            self.vegetation.as_ref(),
        )
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
            metrics.relief.abs_diff(self.settings.max_relief),
            metrics
                .hill_centres
                .abs_diff(u32::from(self.settings.hills_per_bank).saturating_mul(2)),
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
                "Hills fallback radius disagrees with its resolved layout".to_owned(),
            ));
        }
        let patch = PatchRecipeContext::resolve(&self.layout, PatchId(0))?;
        let fragment = construct_patch_with_objects(
            patch,
            &self.settings,
            self.environment,
            self.level_height,
            PatchBuildMode::CanonicalFallback,
            self.vegetation.as_ref(),
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
                "Hills fallback composition failed: {error:?}"
            ))
        })
    }
}

fn recipe_settings(
    settings: &ProceduralV3Settings,
) -> Result<(&V3HillsSettings, V3EnvironmentSettings), V3GenerationError> {
    let V3LayoutSettings::Single(patch) = &settings.layout else {
        return Err(V3GenerationError::RecipeUnavailable("Ring7"));
    };
    let V3RecipeSettings::Hills(hills) = &patch.recipe else {
        return Err(V3GenerationError::RecipeUnavailable(recipe_name(
            &patch.recipe,
        )));
    };
    if patch.environment == V3EnvironmentSettings::Rocky {
        return Err(V3GenerationError::RecipeContract(
            "Hills does not support the Rocky environment".to_owned(),
        ));
    }
    if !patch.overlays.is_empty() {
        return Err(V3GenerationError::RecipeContract(
            "Hills overlays are not implemented yet".to_owned(),
        ));
    }
    Ok((hills, patch.environment))
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

pub(crate) fn construct_patch_with_catalog(
    patch: PatchRecipeContext<'_>,
    settings: &V3HillsSettings,
    environment: V3EnvironmentSettings,
    level_height: f32,
    mode: PatchBuildMode,
    catalog: &RuntimeArtCatalog,
) -> Result<GeneratedPatchPlan, Vec<WorldValidationIssue>> {
    let vegetation = if matches!(
        environment,
        V3EnvironmentSettings::TemperateGrassland | V3EnvironmentSettings::Frozen
    ) {
        Some(
            LandformVegetationSet::resolve(catalog, environment, "Hills")
                .map_err(|error| vec![recipe_issue(error)])?,
        )
    } else {
        None
    };
    construct_patch_with_objects(
        patch,
        settings,
        environment,
        level_height,
        mode,
        vegetation.as_ref(),
    )
}

fn construct_patch_with_objects(
    patch: PatchRecipeContext<'_>,
    settings: &V3HillsSettings,
    environment: V3EnvironmentSettings,
    level_height: f32,
    mode: PatchBuildMode,
    vegetation: Option<&LandformVegetationSet>,
) -> Result<GeneratedPatchPlan, Vec<WorldValidationIssue>> {
    let streams = mode.seed_streams(&patch);
    construct_patch_with_streams(
        patch,
        settings,
        environment,
        level_height,
        streams.map(|streams| HillsStreams {
            orientation: streams.stage("hills.orientation"),
            centres: streams.stage("hills.centres"),
            trees: streams.stage("hills.vegetation.trees"),
            grass: streams.stage("hills.vegetation.grass"),
        }),
        vegetation,
    )
}

pub(crate) fn construct_patch_with_streams(
    patch: PatchRecipeContext<'_>,
    settings: &V3HillsSettings,
    environment: V3EnvironmentSettings,
    level_height: f32,
    streams: Option<HillsStreams<'_>>,
    vegetation: Option<&LandformVegetationSet>,
) -> Result<GeneratedPatchPlan, Vec<WorldValidationIssue>> {
    let frame = LocalPatchFrame::resolve(patch.mask(), patch.layout().kind, patch.grid_radius())
        .map_err(|error| vec![recipe_issue(format!("Hills local frame failed: {error}"))])?;
    let local_mask = frame.local_mask(patch.mask()).map_err(|error| {
        vec![recipe_issue(format!(
            "Hills local mask conversion failed: {error}"
        ))]
    })?;
    let requested_orientation = streams.map_or(0, |streams| {
        u8::try_from(streams.orientation.sample(0) % 6).unwrap_or_default()
    });
    let centre_stream = streams.map(|streams| streams.centres);
    let protected = patch
        .protected_approaches()
        .into_iter()
        .map(|coord| frame.to_local(coord))
        .collect::<Result<BTreeSet<_>, _>>()
        .map_err(|error| {
            vec![recipe_issue(format!(
                "Hills seam approach conversion failed: {error}"
            ))]
        })?;
    let liquid_ports = local_liquid_ports(&patch, frame)?;
    let liquid_approaches = liquid_ports
        .iter()
        .flat_map(|port| port.approach.iter().copied())
        .collect::<BTreeSet<_>>();
    let liquid_sources = liquid_ports
        .iter()
        .filter(|port| port.source)
        .flat_map(|port| port.boundary.iter().copied())
        .collect::<BTreeSet<_>>();
    let river_exclusions = protected
        .difference(&liquid_approaches)
        .copied()
        .collect::<BTreeSet<_>>();
    let lateral_offsets: &[i32] = if patch.layout().kind == super::layout::LayoutKind::Single {
        &[0]
    } else {
        &[0, -4, 4, -6, 6, -2, 2]
    };
    let river_level = settings.valley_level.saturating_sub(1);
    let single_patch_fall =
        (patch.layout().kind == super::layout::LayoutKind::Single).then_some(SMALL_FALL_HEIGHT);
    let confluence = liquid_ports.iter().filter(|port| !port.source).count() > 1;
    let (orientation, river_offset, local_crossings, local_river_nodes) = if confluence {
        confluence_hydrology(
            &local_mask,
            frame.scale(),
            requested_orientation,
            lateral_offsets,
            river_level,
            &liquid_ports,
            &river_exclusions,
        )?
    } else {
        (0..6)
            .flat_map(|turn| {
                lateral_offsets.iter().copied().map(move |river_offset| {
                    (requested_orientation.saturating_add(turn) % 6, river_offset)
                })
            })
            .filter_map(|(orientation, river_offset)| {
                crossing_geometry(&local_mask, orientation, frame.scale(), river_offset)
                    .ok()
                    .filter(|crossings| {
                        crossings.river.is_disjoint(&river_exclusions)
                            && liquid_ports
                                .iter()
                                .all(|port| port.approach.is_subset(&crossings.river))
                    })
                    .and_then(|crossings| {
                        river_nodes(
                            &crossings.river,
                            orientation,
                            river_offset,
                            river_level,
                            single_patch_fall.map(|height| (crossings.fall_target_y, height)),
                            &liquid_sources,
                        )
                        .ok()
                        .filter(|nodes| {
                            liquid_ports
                                .iter()
                                .all(|port| port.accepts_nodes(nodes, river_level))
                                && (liquid_sources.is_empty()
                                    || all_nodes_drain_to_sources(nodes, &liquid_sources))
                        })
                        .map(|nodes| (orientation, river_offset, crossings, nodes))
                    })
            })
            .next()
            .ok_or_else(|| {
                vec![recipe_issue(
                    "Hills cannot align its liquid barrier with the resolved liquid and walker seams",
                )]
            })?
    };
    let mut excluded = protected.clone();
    excluded.extend(local_crossings.protected.iter().copied());
    if confluence {
        excluded.extend(local_crossings.river.iter().copied());
    }
    let centres = select_hill_centres(
        &local_mask,
        settings.hills_per_bank,
        orientation,
        river_offset,
        centre_stream,
        &excluded,
    )?;
    let mut surface_by_local = BTreeMap::new();
    for coord in &local_mask {
        let rise = centres
            .iter()
            .map(|centre| {
                settings
                    .max_relief
                    .saturating_sub(i32::try_from(centre.distance(*coord)).unwrap_or(i32::MAX))
                    .max(0)
            })
            .max()
            .unwrap_or_default();
        surface_by_local.insert(*coord, settings.valley_level.saturating_add(rise));
    }
    for coord in &local_crossings.protected {
        if let Some(level) = surface_by_local.get_mut(coord) {
            *level = settings.valley_level;
        }
    }
    let authored_surface_by_local = surface_by_local.clone();
    fit_protected_routes(
        &local_crossings.protected,
        settings.valley_level,
        &mut surface_by_local,
    );
    if settings.max_relief >= 10 {
        let shelf_exclusions = excluded
            .union(&local_crossings.river)
            .copied()
            .collect::<BTreeSet<_>>();
        add_irregular_shelves(
            &centres,
            &local_mask,
            &shelf_exclusions,
            &local_crossings,
            settings.valley_level,
            &mut surface_by_local,
        );
    }
    let local_ice_caps = if environment == V3EnvironmentSettings::Frozen {
        frozen_ice_caps(&local_crossings, orientation, river_offset)?
    } else {
        BTreeSet::new()
    };
    let crossings = local_crossings.into_world(frame)?;
    let ice_caps = local_ice_caps
        .into_iter()
        .map(|coord| frame.to_world(coord))
        .collect::<Result<BTreeSet<_>, _>>()
        .map_err(|error| {
            vec![recipe_issue(format!(
                "Hills frozen ice-cap conversion failed: {error}"
            ))]
        })?;
    let river_nodes = local_river_nodes
        .into_iter()
        .map(|(position, node)| {
            let position = frame.position_to_world(position).map_err(|error| {
                vec![recipe_issue(format!(
                    "Hills river position conversion failed: {error}"
                ))]
            })?;
            let downstream = node
                .downstream
                .map(|downstream| frame.position_to_world(downstream))
                .transpose()
                .map_err(|error| {
                    vec![recipe_issue(format!(
                        "Hills river downstream conversion failed: {error}"
                    ))]
                })?;
            Ok((
                position,
                LiquidNode {
                    state: node.state,
                    downstream,
                },
            ))
        })
        .collect::<Result<BTreeMap<_, _>, Vec<WorldValidationIssue>>>()?;
    let river_nodes_by_coord = river_nodes
        .iter()
        .map(|(position, node)| (position.coord, (*position, *node)))
        .collect::<BTreeMap<_, _>>();
    let mut surface_by_coord = surface_by_local
        .iter()
        .map(|(coord, level)| {
            frame
                .to_world(*coord)
                .map(|world| (world, *level))
                .map_err(|error| {
                    vec![recipe_issue(format!(
                        "Hills surface conversion failed: {error}"
                    ))]
                })
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let seam_shape = shape_walker_seams(&patch, &mut surface_by_coord)?;
    if confluence && settings.max_relief >= 10 {
        let authored_surface_by_coord = authored_surface_by_local
            .into_iter()
            .map(|(coord, level)| {
                frame
                    .to_world(coord)
                    .map(|world| (world, level))
                    .map_err(|error| {
                        vec![recipe_issue(format!(
                            "Hills authored surface conversion failed: {error}"
                        ))]
                    })
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        let frozen_approaches = protected
            .into_iter()
            .map(|coord| frame.to_world(coord).map_err(recipe_issue))
            .collect::<Result<BTreeSet<_>, _>>()
            .map_err(|issue| vec![issue])?;
        restore_connected_relief(
            &authored_surface_by_coord,
            &crossings,
            settings.valley_level,
            &seam_shape,
            &frozen_approaches,
            &mut surface_by_coord,
        );
    }

    let mut columns = BTreeMap::new();
    let mut surfaces = BTreeMap::new();
    let mut ordinary_by_coord = BTreeMap::new();
    for (coord, level) in &surface_by_coord {
        let bridge = crossings.bridge_deck.contains(coord);
        let ford = crossings.ford_deck.contains(coord) || crossings.auxiliary_fords.contains(coord);
        if crossings.river.contains(coord) {
            let Some((liquid_position, liquid_node)) = river_nodes_by_coord.get(coord).copied()
            else {
                return Err(vec![recipe_issue(format!(
                    "Hills river cell {coord:?} has no liquid node"
                ))]);
            };
            let (column, bed, crossing, ice_cap) = river_column(
                *coord,
                settings.valley_level,
                environment,
                bridge,
                ford,
                ice_caps.contains(coord),
                liquid_position,
                liquid_node,
            );
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
                ordinary_by_coord.insert(*coord, crossing);
            }
            if let Some(ice_cap) = ice_cap {
                surfaces.insert(
                    ice_cap,
                    SurfaceMetadata {
                        access: SurfaceAccess::NonStandable,
                        interior: None,
                    },
                );
            }
        } else {
            let mut column = land_column(*level, environment);
            if ford {
                set_land_surface_material(&mut column, causeway_material(environment));
            }
            let surface = if bridge {
                let deck = TilePos::new(*coord, settings.valley_level.saturating_add(1));
                column.elements.push(VolumeElement::Solid(SolidMass {
                    levels: LevelInterval::new(deck.level, deck.level.saturating_add(1)),
                    material: SolidMaterialRole::Metal,
                    cutaway_for: None,
                }));
                deck
            } else {
                TilePos::new(*coord, *level)
            };
            columns.insert(*coord, column);
            surfaces.insert(
                surface,
                SurfaceMetadata {
                    access: SurfaceAccess::Ordinary,
                    interior: None,
                },
            );
            ordinary_by_coord.insert(*coord, surface);
        }
    }
    let mut volume = VolumePlan {
        mask: patch.mask().clone(),
        columns,
        surfaces,
    };
    seam_shape.apply(&mut volume)?;
    ordinary_by_coord.retain(|_, position| {
        volume
            .surfaces
            .get(position)
            .is_some_and(|metadata| metadata.access == SurfaceAccess::Ordinary)
    });
    let (party_coord, hostile_coord) =
        opposing_landings(ordinary_by_coord.keys().copied(), frame, orientation)?;
    let conflict_coord = crossings
        .bridge_centerline
        .get(crossings.bridge_centerline.len() / 2)
        .copied()
        .ok_or_else(|| vec![recipe_issue("Hills bridge has no conflict landing")])?;
    let alternate_coord = crossings
        .ford_centerline
        .get(crossings.ford_centerline.len() / 2)
        .copied()
        .ok_or_else(|| {
            vec![recipe_issue(
                "Hills alternate crossing has no review landing",
            )]
        })?;
    let exact = |coord| ordinary_by_coord.get(&coord).copied();
    let anchors = BTreeMap::from([
        (
            PARTY_START.to_owned(),
            exact(party_coord)
                .ok_or_else(|| vec![recipe_issue("Hills party landing is missing")])?,
        ),
        (
            HOSTILE_START.to_owned(),
            exact(hostile_coord)
                .ok_or_else(|| vec![recipe_issue("Hills hostile landing is missing")])?,
        ),
        (
            CONFLICT_CENTER.to_owned(),
            exact(conflict_coord)
                .ok_or_else(|| vec![recipe_issue("Hills conflict landing is missing")])?,
        ),
        (
            BRIDGE_ANCHOR.to_owned(),
            exact(conflict_coord)
                .ok_or_else(|| vec![recipe_issue("Hills bridge landing is missing")])?,
        ),
        (
            FORD_ROUTE.to_owned(),
            exact(alternate_coord)
                .ok_or_else(|| vec![recipe_issue("Hills alternate landing is missing")])?,
        ),
    ]);
    let bridge_route = route_membership(
        &crossings.bridge_centerline,
        &crossings.bridge_route,
        &ordinary_by_coord,
    )?;
    let ford_route = route_membership(
        &crossings.ford_centerline,
        &crossings.ford_route,
        &ordinary_by_coord,
    )?;
    let mut features = FeaturePlan {
        by_id: BTreeMap::new(),
        protected_routes: BTreeMap::from([
            (BRIDGE_ROUTE.to_owned(), bridge_route),
            (FORD_ROUTE.to_owned(), ford_route),
        ]),
        clearings: BTreeMap::new(),
    };
    let mut blockers = BTreeSet::new();
    if let Some(vegetation) = vegetation {
        let mut reserved = crossings.river.clone();
        for coord in features
            .protected_routes
            .values()
            .flat_map(|route| route.surfaces.iter().map(|surface| surface.coord))
            .chain(anchors.values().map(|anchor| anchor.coord))
            .chain(patch.protected_approaches())
        {
            reserved.extend(coord.within_radius(2));
        }
        let valley_candidates = ordinary_by_coord
            .iter()
            .filter_map(|(coord, position)| {
                (position.level
                    <= vegetation_valley_ceiling(settings, patch.layout().kind.is_composite()))
                .then_some(*coord)
            })
            .collect::<BTreeSet<_>>();
        let eligible_valley = valley_candidates
            .difference(&reserved)
            .copied()
            .collect::<BTreeSet<_>>();
        let eligible_dry = ordinary_by_coord
            .keys()
            .filter(|coord| !reserved.contains(coord))
            .copied()
            .collect::<BTreeSet<_>>();
        let grass_target =
            hills_grass_target(eligible_valley.len()).map_err(|error| vec![recipe_issue(error)])?;
        append_landform_vegetation(
            if environment == V3EnvironmentSettings::Frozen {
                "Frozen Hills"
            } else {
                "Hills"
            },
            vegetation,
            &ordinary_by_coord,
            &eligible_dry,
            &eligible_valley,
            &reserved,
            HILLS_TREE_TARGET,
            grass_target,
            streams.map(|streams| streams.trees),
            streams.map(|streams| streams.grass),
            &mut features,
            &mut blockers,
        )
        .map_err(|error| vec![recipe_issue(error)])?;
    }
    let liquid_material = if environment == V3EnvironmentSettings::Volcanic {
        FillMaterialRole::Lava
    } else {
        FillMaterialRole::Water
    };
    let biome_regions = volume
        .surfaces
        .keys()
        .copied()
        .map(|surface| (surface, patch.biome_region()))
        .collect();
    let local_levels = surface_by_coord
        .iter()
        .map(|(coord, level)| {
            frame
                .to_local(*coord)
                .map(|local| (local, *level))
                .map_err(|error| {
                    vec![recipe_issue(format!(
                        "Hills camera conversion failed: {error}"
                    ))]
                })
        })
        .collect::<Result<_, Vec<WorldValidationIssue>>>()?;
    let view_hint = frame.view_hint_to_world(hills_view_hint(
        &local_mask,
        &local_levels,
        frame.scale(),
        level_height,
    )?);

    let fragment = GeneratedPatchPlan {
        patch_id: patch.id,
        volume,
        liquids: LiquidPlan {
            bodies: BTreeMap::from([(
                LiquidBodyId(0),
                LiquidBodyPlan {
                    material: liquid_material,
                    nodes: river_nodes,
                },
            )]),
        },
        features,
        structures: StructurePlan {
            by_id: BTreeMap::from([(
                StructureId(0),
                PlannedStructure {
                    kind: StructureKind::Bridge,
                    voxels: crossings
                        .bridge_deck
                        .iter()
                        .map(|coord| TilePos::new(*coord, settings.valley_level.saturating_add(1)))
                        .collect(),
                },
            )]),
        },
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
                    "Hills patch {:?} failed {:?}: {}",
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

#[derive(Debug, Clone)]
struct LocalLiquidPort {
    edge: ResolvedEdgeId,
    side: HexSide,
    source: bool,
    boundary: BTreeSet<HexCoord>,
    approach: BTreeSet<HexCoord>,
    minimum_level: Level,
    maximum_level: Level,
}

impl LocalLiquidPort {
    fn accepts_nodes(&self, nodes: &BTreeMap<TilePos, LiquidNode>, level: Level) -> bool {
        self.boundary.iter().all(|coord| {
            let Some(node) = nodes.get(&TilePos::new(*coord, level)) else {
                return false;
            };
            !self.source || (node.state == LiquidFlowState::Still && node.downstream.is_none())
        })
    }
}

fn local_liquid_ports(
    patch: &PatchRecipeContext<'_>,
    frame: LocalPatchFrame,
) -> Result<Vec<LocalLiquidPort>, Vec<WorldValidationIssue>> {
    patch
        .shared_edges()
        .filter_map(|edge| {
            edge.liquid_port().map(|(source, port)| {
                (
                    edge.id,
                    edge.side,
                    source,
                    port,
                    edge.minimum_level(),
                    edge.maximum_level(),
                )
            })
        })
        .map(|(edge, side, source, port, minimum_level, maximum_level)| {
            let boundary = port
                .lanes
                .iter()
                .map(|(coord, _)| frame.to_local(*coord))
                .collect::<Result<BTreeSet<_>, _>>();
            let approach = port
                .first_approach
                .iter()
                .copied()
                .map(|coord| frame.to_local(coord))
                .collect::<Result<BTreeSet<_>, _>>();
            Ok(LocalLiquidPort {
                edge,
                side,
                source,
                boundary: boundary.map_err(|error| {
                    vec![recipe_issue(format!(
                        "Hills liquid boundary conversion failed: {error}"
                    ))]
                })?,
                approach: approach.map_err(|error| {
                    vec![recipe_issue(format!(
                        "Hills liquid approach conversion failed: {error}"
                    ))]
                })?,
                minimum_level,
                maximum_level,
            })
        })
        .collect()
}

#[derive(Debug)]
struct CrossingGeometry {
    river: BTreeSet<HexCoord>,
    bridge_deck: BTreeSet<HexCoord>,
    ford_deck: BTreeSet<HexCoord>,
    auxiliary_fords: BTreeSet<HexCoord>,
    bridge_centerline: Vec<HexCoord>,
    bridge_route: BTreeSet<HexCoord>,
    ford_centerline: Vec<HexCoord>,
    ford_route: BTreeSet<HexCoord>,
    protected: BTreeSet<HexCoord>,
    fall_target_y: i32,
}

impl CrossingGeometry {
    fn into_world(self, frame: LocalPatchFrame) -> Result<Self, Vec<WorldValidationIssue>> {
        let convert_set = |coords: BTreeSet<HexCoord>| {
            coords
                .into_iter()
                .map(|coord| frame.to_world(coord))
                .collect::<Result<BTreeSet<_>, _>>()
                .map_err(|error| {
                    vec![recipe_issue(format!(
                        "Hills crossing conversion failed: {error}"
                    ))]
                })
        };
        let convert_route = |coords: Vec<HexCoord>| {
            coords
                .into_iter()
                .map(|coord| frame.to_world(coord))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| {
                    vec![recipe_issue(format!(
                        "Hills route conversion failed: {error}"
                    ))]
                })
        };
        Ok(Self {
            river: convert_set(self.river)?,
            bridge_deck: convert_set(self.bridge_deck)?,
            ford_deck: convert_set(self.ford_deck)?,
            auxiliary_fords: convert_set(self.auxiliary_fords)?,
            bridge_centerline: convert_route(self.bridge_centerline)?,
            bridge_route: convert_set(self.bridge_route)?,
            ford_centerline: convert_route(self.ford_centerline)?,
            ford_route: convert_set(self.ford_route)?,
            protected: convert_set(self.protected)?,
            fall_target_y: self.fall_target_y,
        })
    }
}

fn frozen_ice_caps(
    crossings: &CrossingGeometry,
    orientation: u8,
    river_offset: i32,
) -> Result<BTreeSet<HexCoord>, Vec<WorldValidationIssue>> {
    let minimum_y = crossings
        .river
        .iter()
        .map(|coord| unrotate(*coord, orientation).y())
        .min()
        .unwrap_or_default();
    let maximum_y = crossings
        .river
        .iter()
        .map(|coord| unrotate(*coord, orientation).y())
        .max()
        .unwrap_or_default();
    let upstream_limit = minimum_y.saturating_add(maximum_y.saturating_sub(minimum_y).max(0) / 3);
    let mut candidates = crossings
        .river
        .iter()
        .copied()
        .filter(|coord| {
            let local = unrotate(*coord, orientation);
            local.x().saturating_sub(river_offset).abs() == RIVER_HALF_WIDTH
                && local.y() <= upstream_limit
                && !crossings.protected.contains(coord)
        })
        .collect::<Vec<_>>();
    candidates.sort_unstable_by_key(|coord| {
        let local = unrotate(*coord, orientation);
        (local.y(), local.x(), *coord)
    });
    let mut selected = BTreeSet::new();
    for candidate in candidates {
        if selected
            .iter()
            .all(|ice: &HexCoord| ice.distance(candidate) > 1)
        {
            selected.insert(candidate);
            if selected.len() == FROZEN_ICE_CAP_TARGET {
                break;
            }
        }
    }
    if selected.len() < 3 {
        Err(vec![recipe_issue(format!(
            "Frozen Hills can place only {} isolated upstream shoreline ice caps",
            selected.len()
        ))])
    } else {
        Ok(selected)
    }
}

fn crossing_geometry(
    mask: &BTreeSet<HexCoord>,
    orientation: u8,
    grid_radius: u32,
    river_offset: i32,
) -> Result<CrossingGeometry, Vec<WorldValidationIssue>> {
    let radius = i32::try_from(grid_radius)
        .map_err(|error| vec![recipe_issue(format!("Hills radius exceeds i32: {error}"))])?;
    let ford_y = radius / 2;
    let river: BTreeSet<_> = mask
        .iter()
        .copied()
        .filter(|coord| {
            unrotate(*coord, orientation)
                .x()
                .saturating_sub(river_offset)
                .abs()
                <= RIVER_HALF_WIDTH
        })
        .collect();
    let route = |first_y: i32| {
        [first_y, first_y.saturating_add(1)]
            .into_iter()
            .flat_map(|y| {
                (-APPROACH_HALF_LENGTH..=APPROACH_HALF_LENGTH).map(move |x| {
                    rotate(
                        HexCoord::from_axial(x.saturating_add(river_offset), y),
                        orientation,
                    )
                })
            })
            .collect::<BTreeSet<_>>()
    };
    let deck = |first_y: i32| {
        [first_y, first_y.saturating_add(1)]
            .into_iter()
            .flat_map(|y| {
                (-CROSSING_HALF_LENGTH..=CROSSING_HALF_LENGTH).map(move |x| {
                    rotate(
                        HexCoord::from_axial(x.saturating_add(river_offset), y),
                        orientation,
                    )
                })
            })
            .collect::<BTreeSet<_>>()
    };
    let centerline = |y: i32| {
        (-APPROACH_HALF_LENGTH..=APPROACH_HALF_LENGTH)
            .map(|x| {
                rotate(
                    HexCoord::from_axial(x.saturating_add(river_offset), y),
                    orientation,
                )
            })
            .collect::<Vec<_>>()
    };
    let bridge_route = route(0);
    let bridge_deck = deck(0);
    let ford_route = route(ford_y);
    let ford_deck = deck(ford_y);
    let protected: BTreeSet<_> = bridge_route.union(&ford_route).copied().collect();
    for (name, coordinates) in [
        ("river", &river),
        ("bridge route", &bridge_route),
        ("bridge deck", &bridge_deck),
        ("ford route", &ford_route),
        ("ford deck", &ford_deck),
    ] {
        if coordinates.is_empty() || !coordinates.is_subset(mask) {
            return Err(vec![recipe_issue(format!(
                "Hills patch cannot fit its complete {name}"
            ))]);
        }
    }
    Ok(CrossingGeometry {
        river,
        bridge_deck,
        ford_deck,
        auxiliary_fords: BTreeSet::new(),
        bridge_centerline: centerline(0),
        bridge_route,
        ford_centerline: centerline(ford_y),
        ford_route,
        protected,
        fall_target_y: (ford_y / 2).max(1),
    })
}

fn confluence_hydrology(
    mask: &BTreeSet<HexCoord>,
    grid_radius: u32,
    requested_orientation: u8,
    lateral_offsets: &[i32],
    river_level: Level,
    ports: &[LocalLiquidPort],
    river_exclusions: &BTreeSet<HexCoord>,
) -> Result<(u8, i32, CrossingGeometry, BTreeMap<TilePos, LiquidNode>), Vec<WorldValidationIssue>> {
    let (outlet, main_inlet) = confluence_main_ports(ports)?;
    let inlets = ports.iter().filter(|port| !port.source).collect::<Vec<_>>();
    if ports
        .iter()
        .any(|port| river_level < port.minimum_level || river_level > port.maximum_level)
    {
        return Err(vec![recipe_issue(format!(
            "Hills confluence level {river_level} leaves a resolved liquid elevation band"
        ))]);
    }
    for (index, first) in ports.iter().enumerate() {
        for second in ports.iter().skip(index.saturating_add(1)) {
            if !first.approach.is_disjoint(&second.approach) {
                return Err(vec![recipe_issue(format!(
                    "Hills liquid approaches for edges {:?} and {:?} overlap",
                    first.edge, second.edge
                ))]);
            }
        }
    }
    for turn in 0..6 {
        let orientation = requested_orientation.saturating_add(turn) % 6;
        for river_offset in lateral_offsets {
            let Ok(mut crossings) =
                crossing_geometry(mask, orientation, grid_radius, *river_offset)
            else {
                continue;
            };
            if !crossings.river.is_disjoint(river_exclusions)
                || !outlet.approach.is_subset(&crossings.river)
                || !main_inlet.approach.is_subset(&crossings.river)
            {
                continue;
            }
            let Ok(branch_ford_candidates) = attach_confluence_branches(
                mask,
                ports,
                river_exclusions,
                orientation,
                &mut crossings,
            ) else {
                continue;
            };
            let Ok(branch_fords) = necessary_auxiliary_fords(
                mask,
                &crossings.river,
                &crossings.bridge_deck,
                &crossings.ford_deck,
                &branch_ford_candidates,
            ) else {
                continue;
            };
            crossings.auxiliary_fords = branch_fords.clone();
            crossings.protected.extend(branch_fords);
            let Ok(nodes) = confluence_river_nodes(&crossings.river, river_level, &outlet.boundary)
            else {
                continue;
            };
            if !ports
                .iter()
                .all(|port| port.accepts_nodes(&nodes, river_level))
                || inlets.iter().any(|port| {
                    port.boundary.iter().any(|coord| {
                        nodes
                            .get(&TilePos::new(*coord, river_level))
                            .is_none_or(|node| node.downstream.is_none())
                    })
                })
                || !all_nodes_drain_to_sources(&nodes, &outlet.boundary)
                || !has_flow_merge(&nodes)
            {
                continue;
            }
            return Ok((orientation, *river_offset, crossings, nodes));
        }
    }

    Err(vec![recipe_issue(
        "Hills cannot route its resolved inlets into one deterministic outlet",
    )])
}

fn confluence_main_ports(
    ports: &[LocalLiquidPort],
) -> Result<(&LocalLiquidPort, &LocalLiquidPort), Vec<WorldValidationIssue>> {
    let outlets = ports.iter().filter(|port| port.source).collect::<Vec<_>>();
    let inlets = ports.iter().filter(|port| !port.source).collect::<Vec<_>>();
    if outlets.len() != 1 || inlets.len() < 2 {
        return Err(vec![recipe_issue(format!(
            "Hills confluence requires at least two inlets and exactly one outlet; found {} inlets and {} outlets",
            inlets.len(),
            outlets.len()
        ))]);
    }
    let Some(outlet) = outlets.first().copied() else {
        return Err(vec![recipe_issue(
            "Hills confluence has no resolved outlet",
        )]);
    };
    let opposite_side = outlet.side.opposite();
    let Some(main_inlet) = inlets
        .iter()
        .copied()
        .find(|inlet| inlet.side == opposite_side)
    else {
        return Err(vec![recipe_issue(format!(
            "Hills confluence requires an inlet opposite its {:?} outlet to preserve the complete three-wide two-crossing bank barrier",
            outlet.side
        ))]);
    };
    Ok((outlet, main_inlet))
}

fn attach_confluence_branches(
    mask: &BTreeSet<HexCoord>,
    ports: &[LocalLiquidPort],
    river_exclusions: &BTreeSet<HexCoord>,
    orientation: u8,
    crossings: &mut CrossingGeometry,
) -> Result<BTreeSet<HexCoord>, Vec<WorldValidationIssue>> {
    let mut inlets = ports.iter().filter(|port| !port.source).collect::<Vec<_>>();
    inlets.sort_unstable_by_key(|port| port.edge);
    let all_approaches = ports
        .iter()
        .flat_map(|port| port.approach.iter().copied())
        .collect::<BTreeSet<_>>();
    let crossing_exclusions = crossings
        .protected
        .difference(&crossings.river)
        .copied()
        .collect::<BTreeSet<_>>();
    let main_trunk = crossings.river.clone();
    let crossing_rows = crossings
        .bridge_deck
        .union(&crossings.ford_deck)
        .filter(|coord| main_trunk.contains(coord))
        .map(|coord| unrotate(*coord, orientation).y())
        .collect::<BTreeSet<_>>();
    let Some(first_crossing_row) = crossing_rows.first().copied() else {
        return Err(vec![recipe_issue(
            "Hills confluence bridge and passage do not intersect the main trunk",
        )]);
    };
    let Some(last_crossing_row) = crossing_rows.last().copied() else {
        return Err(vec![recipe_issue(
            "Hills confluence bridge and passage do not span the main trunk",
        )]);
    };
    let safe_trunk = main_trunk
        .iter()
        .copied()
        .filter(|coord| {
            let row = unrotate(*coord, orientation).y();
            row < first_crossing_row || row > last_crossing_row
        })
        .collect::<BTreeSet<_>>();
    if safe_trunk.is_empty() {
        return Err(vec![recipe_issue(
            "Hills confluence has no safe main-trunk attachment outside its two crossings",
        )]);
    }
    let mut river = main_trunk.clone();
    let mut branch_fords = BTreeSet::new();

    for inlet in inlets {
        if inlet.approach.is_subset(&main_trunk) {
            continue;
        }
        if !inlet.approach.is_subset(mask)
            || !inlet.approach.is_disjoint(river_exclusions)
            || !inlet.approach.is_disjoint(&crossing_exclusions)
        {
            return Err(vec![recipe_issue(format!(
                "Hills inlet {:?} leaves its available routing domain",
                inlet.edge
            ))]);
        }
        let other_approaches = all_approaches
            .difference(&inlet.approach)
            .copied()
            .collect::<BTreeSet<_>>();
        let existing_branches = river
            .difference(&main_trunk)
            .copied()
            .collect::<BTreeSet<_>>();
        let forbidden = river_exclusions
            .union(&crossing_exclusions)
            .copied()
            .chain(other_approaches)
            .chain(existing_branches)
            .collect::<BTreeSet<_>>();
        let path = shortest_branch_path(mask, &inlet.approach, &safe_trunk, &forbidden)?;
        let new_spine = path
            .iter()
            .copied()
            .filter(|coord| !main_trunk.contains(coord) && !inlet.approach.contains(coord))
            .collect::<BTreeSet<_>>();
        river.extend(inlet.approach.iter().copied());
        river.extend(path);
        branch_fords.extend(new_spine);
    }
    crossings.river = river;
    Ok(branch_fords)
}

fn necessary_auxiliary_fords(
    mask: &BTreeSet<HexCoord>,
    river: &BTreeSet<HexCoord>,
    bridge_deck: &BTreeSet<HexCoord>,
    ford_deck: &BTreeSet<HexCoord>,
    candidates: &BTreeSet<HexCoord>,
) -> Result<BTreeSet<HexCoord>, Vec<WorldValidationIssue>> {
    let dry = mask.difference(river).copied().collect::<BTreeSet<_>>();
    let mut component_by_coord = BTreeMap::new();
    let mut component_count = 0_usize;
    for start in &dry {
        if component_by_coord.contains_key(start) {
            continue;
        }
        let mut reachable = BTreeSet::from([*start]);
        let mut frontier = VecDeque::from([*start]);
        while let Some(coord) = frontier.pop_front() {
            for neighbor in coord.neighbors() {
                if dry.contains(&neighbor) && reachable.insert(neighbor) {
                    frontier.push_back(neighbor);
                }
            }
        }
        for coord in reachable {
            component_by_coord.insert(coord, component_count);
        }
        component_count = component_count.saturating_add(1);
    }
    let bridge_banks = horizontal_authority_banks(bridge_deck, river, &component_by_coord);
    let ford_banks = horizontal_authority_banks(ford_deck, river, &component_by_coord);
    let ordered_bridge_banks = bridge_banks.iter().copied().collect::<Vec<_>>();
    let [bridge_first, bridge_second] = ordered_bridge_banks.as_slice() else {
        return Err(vec![recipe_issue(
            "Hills confluence bridge does not join exactly two dry banks",
        )]);
    };
    if ford_banks != bridge_banks {
        return Err(vec![recipe_issue(
            "Hills confluence main crossings do not join the same two dry banks",
        )]);
    }
    let mut group_by_component = (0..component_count)
        .map(|component| (component, component))
        .collect::<BTreeMap<_, _>>();
    merge_bank_groups(&mut group_by_component, *bridge_first, *bridge_second);
    let mut selected = BTreeSet::new();
    for candidate in candidates {
        let banks =
            horizontal_authority_banks(&BTreeSet::from([*candidate]), river, &component_by_coord);
        let banks = banks.into_iter().collect::<Vec<_>>();
        let [first, second] = banks.as_slice() else {
            continue;
        };
        let first_group = group_by_component.get(first).copied();
        let second_group = group_by_component.get(second).copied();
        if first_group != second_group {
            selected.insert(*candidate);
            merge_bank_groups(&mut group_by_component, *first, *second);
        }
    }
    if group_by_component
        .values()
        .copied()
        .collect::<BTreeSet<_>>()
        .len()
        != 1
    {
        return Err(vec![recipe_issue(
            "Hills cannot connect every tributary bank with deterministic auxiliary crossings",
        )]);
    }
    Ok(selected)
}

fn horizontal_authority_banks(
    authority: &BTreeSet<HexCoord>,
    river: &BTreeSet<HexCoord>,
    component_by_coord: &BTreeMap<HexCoord, usize>,
) -> BTreeSet<usize> {
    authority
        .intersection(river)
        .flat_map(|coord| coord.neighbors())
        .filter_map(|neighbor| component_by_coord.get(&neighbor).copied())
        .collect()
}

fn merge_bank_groups(groups: &mut BTreeMap<usize, usize>, first: usize, second: usize) {
    let Some(first_group) = groups.get(&first).copied() else {
        return;
    };
    let Some(second_group) = groups.get(&second).copied() else {
        return;
    };
    if first_group == second_group {
        return;
    }
    for group in groups.values_mut() {
        if *group == second_group {
            *group = first_group;
        }
    }
}

fn rederive_confluence_auxiliary_crossings(
    plan: &GeneratedWorldPlan,
    ports: &[LocalLiquidPort],
    protected_approaches: &BTreeSet<HexCoord>,
    scale: u32,
    valley_level: Level,
) -> Result<BTreeSet<TilePos>, Vec<WorldValidationIssue>> {
    let Some(bridge) = plan.features.protected_routes.get(BRIDGE_ROUTE) else {
        return Err(vec![recipe_issue(
            "Hills cannot rederive tributary crossings without its main bridge route",
        )]);
    };
    let Some(alternate) = plan.features.protected_routes.get(FORD_ROUTE) else {
        return Err(vec![recipe_issue(
            "Hills cannot rederive tributary crossings without its alternate route",
        )]);
    };
    let bridge_coords = bridge
        .surfaces
        .iter()
        .map(|surface| surface.coord)
        .collect::<BTreeSet<_>>();
    let alternate_coords = alternate
        .surfaces
        .iter()
        .map(|surface| surface.coord)
        .collect::<BTreeSet<_>>();
    let fill_coords = plan
        .volume
        .fill_runs_by_top()
        .keys()
        .map(|position| position.coord)
        .collect::<BTreeSet<_>>();
    let liquid_approaches = ports
        .iter()
        .flat_map(|port| port.approach.iter().copied())
        .collect::<BTreeSet<_>>();
    let river_exclusions = protected_approaches
        .difference(&liquid_approaches)
        .copied()
        .collect::<BTreeSet<_>>();
    let (outlet, main_inlet) = confluence_main_ports(ports)?;
    let mut matches = BTreeSet::new();

    for orientation in 0..6 {
        for river_offset in [0, -4, 4, -6, 6, -2, 2] {
            let Ok(mut geometry) =
                crossing_geometry(&plan.layout.footprint, orientation, scale, river_offset)
            else {
                continue;
            };
            if geometry.bridge_route != bridge_coords
                || geometry.ford_route != alternate_coords
                || !geometry.river.is_disjoint(&river_exclusions)
                || !outlet.approach.is_subset(&geometry.river)
                || !main_inlet.approach.is_subset(&geometry.river)
            {
                continue;
            }
            let Ok(auxiliary_candidates) = attach_confluence_branches(
                &plan.layout.footprint,
                ports,
                &river_exclusions,
                orientation,
                &mut geometry,
            ) else {
                continue;
            };
            let Ok(auxiliary) = necessary_auxiliary_fords(
                &plan.layout.footprint,
                &geometry.river,
                &geometry.bridge_deck,
                &geometry.ford_deck,
                &auxiliary_candidates,
            ) else {
                continue;
            };
            if geometry.river == fill_coords {
                matches.insert(
                    auxiliary
                        .into_iter()
                        .map(|coord| TilePos::new(coord, valley_level))
                        .collect::<BTreeSet<_>>(),
                );
            }
        }
    }

    match matches.into_iter().collect::<Vec<_>>().as_slice() {
        [expected] => Ok(expected.clone()),
        [] => Err(vec![recipe_issue(
            "Hills cannot rederive auxiliary tributary crossings from its exact liquid branches",
        )]),
        alternatives => Err(vec![recipe_issue(format!(
            "Hills auxiliary tributary crossing authority is ambiguous across {} exact geometries",
            alternatives.len()
        ))]),
    }
}

fn shortest_branch_path(
    mask: &BTreeSet<HexCoord>,
    starts: &BTreeSet<HexCoord>,
    targets: &BTreeSet<HexCoord>,
    forbidden: &BTreeSet<HexCoord>,
) -> Result<Vec<HexCoord>, Vec<WorldValidationIssue>> {
    let mut frontier = starts
        .iter()
        .copied()
        .filter(|coord| mask.contains(coord))
        .collect::<BTreeSet<_>>();
    let mut parents = frontier
        .iter()
        .copied()
        .map(|coord| (coord, None))
        .collect::<BTreeMap<_, _>>();
    let target = loop {
        if let Some(target) = frontier
            .iter()
            .copied()
            .find(|coord| targets.contains(coord))
        {
            break target;
        }
        let mut next = BTreeSet::new();
        for coord in &frontier {
            let mut neighbors = coord.neighbors();
            neighbors.sort_unstable();
            for neighbor in neighbors {
                if !mask.contains(&neighbor)
                    || (forbidden.contains(&neighbor) && !targets.contains(&neighbor))
                    || parents.contains_key(&neighbor)
                {
                    continue;
                }
                parents.insert(neighbor, Some(*coord));
                next.insert(neighbor);
            }
        }
        if next.is_empty() {
            return Err(vec![recipe_issue(
                "Hills cannot connect a liquid inlet to its confluence trunk",
            )]);
        }
        frontier = next;
    };
    let mut path = Vec::new();
    let mut current = target;
    loop {
        path.push(current);
        let Some(parent) = parents.get(&current).copied().flatten() else {
            break;
        };
        current = parent;
    }
    path.reverse();
    Ok(path)
}

fn confluence_river_nodes(
    river: &BTreeSet<HexCoord>,
    level: Level,
    outlets: &BTreeSet<HexCoord>,
) -> Result<BTreeMap<TilePos, LiquidNode>, Vec<WorldValidationIssue>> {
    if outlets.is_empty() || !outlets.is_subset(river) {
        return Err(vec![recipe_issue(
            "Hills confluence outlet leaves the planned river",
        )]);
    }
    let mut distances = outlets
        .iter()
        .copied()
        .map(|coord| (coord, 0_u32))
        .collect::<BTreeMap<_, _>>();
    let mut frontier = outlets.clone();
    while !frontier.is_empty() {
        let mut next = BTreeSet::new();
        for coord in &frontier {
            let distance = distances.get(coord).copied().unwrap_or_default();
            let mut neighbors = coord.neighbors();
            neighbors.sort_unstable();
            for neighbor in neighbors {
                if river.contains(&neighbor) && !distances.contains_key(&neighbor) {
                    distances.insert(neighbor, distance.saturating_add(1));
                    next.insert(neighbor);
                }
            }
        }
        frontier = next;
    }
    if distances.len() != river.len() {
        return Err(vec![recipe_issue(
            "Hills confluence river is not one connected body",
        )]);
    }

    river
        .iter()
        .copied()
        .map(|coord| {
            let distance = distances.get(&coord).copied().unwrap_or_default();
            let downstream = if distance == 0 {
                None
            } else {
                coord
                    .neighbors()
                    .into_iter()
                    .filter_map(|neighbor| {
                        distances
                            .get(&neighbor)
                            .copied()
                            .filter(|next| next.saturating_add(1) == distance)
                            .map(|next| (next, neighbor))
                    })
                    .min()
                    .map(|(_, neighbor)| TilePos::new(neighbor, level))
            };
            if distance > 0 && downstream.is_none() {
                return Err(vec![recipe_issue(format!(
                    "Hills confluence node {coord:?} has no downhill successor"
                ))]);
            }
            Ok((
                TilePos::new(coord, level),
                LiquidNode {
                    state: if downstream.is_some() {
                        LiquidFlowState::Current
                    } else {
                        LiquidFlowState::Still
                    },
                    downstream,
                },
            ))
        })
        .collect()
}

fn has_flow_merge(nodes: &BTreeMap<TilePos, LiquidNode>) -> bool {
    let mut incoming = BTreeMap::<TilePos, usize>::new();
    for downstream in nodes.values().filter_map(|node| node.downstream) {
        *incoming.entry(downstream).or_default() += 1;
    }
    incoming.values().any(|count| *count > 1)
}

fn river_column(
    coord: HexCoord,
    valley: Level,
    environment: V3EnvironmentSettings,
    bridge: bool,
    ford: bool,
    ice_cap: bool,
    liquid_position: TilePos,
    liquid_node: LiquidNode,
) -> (VolumeColumn, TilePos, Option<TilePos>, Option<TilePos>) {
    let (bed_level, fill_bottom) = if liquid_node.state == LiquidFlowState::Fall {
        liquid_node.downstream.map_or_else(
            || {
                (
                    liquid_position
                        .level
                        .saturating_sub(RIVER_DEPTH.saturating_sub(1)),
                    liquid_position.level.saturating_sub(1),
                )
            },
            |downstream| (downstream.level.saturating_sub(1), downstream.level),
        )
    } else {
        (
            liquid_position
                .level
                .saturating_sub(RIVER_DEPTH.saturating_sub(1)),
            liquid_position.level.saturating_sub(1),
        )
    };
    let core = if environment == V3EnvironmentSettings::Volcanic {
        SolidMaterialRole::Basalt
    } else {
        SolidMaterialRole::Stone
    };
    let fill = if environment == V3EnvironmentSettings::Volcanic {
        FillMaterialRole::Lava
    } else {
        FillMaterialRole::Water
    };
    let mut elements = vec![
        VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(0, 1),
            material: SolidMaterialRole::Bedrock,
            cutaway_for: None,
        }),
        VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(1, bed_level),
            material: core,
            cutaway_for: None,
        }),
        VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(bed_level, bed_level.saturating_add(1)),
            material: SolidMaterialRole::Gravel,
            cutaway_for: None,
        }),
        VolumeElement::Fill(NonSolidFill {
            levels: LevelInterval::new(fill_bottom, liquid_position.level.saturating_add(1)),
            material: fill,
        }),
    ];
    let crossing = if ford {
        let surface = TilePos::new(coord, valley);
        let support_bottom = liquid_position.level.saturating_add(1);
        elements.push(VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(support_bottom, surface.level.saturating_add(1)),
            material: causeway_material(environment),
            cutaway_for: None,
        }));
        Some(surface)
    } else if bridge {
        let surface = TilePos::new(coord, valley.saturating_add(1));
        elements.push(VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(surface.level, surface.level.saturating_add(1)),
            material: SolidMaterialRole::Metal,
            cutaway_for: None,
        }));
        Some(surface)
    } else {
        None
    };
    let ice_cap = ice_cap.then(|| {
        let surface = TilePos::new(coord, liquid_position.level.saturating_add(1));
        elements.push(VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(surface.level, surface.level.saturating_add(1)),
            material: SolidMaterialRole::Ice,
            cutaway_for: None,
        }));
        surface
    });
    (
        VolumeColumn { elements },
        TilePos::new(coord, bed_level),
        crossing,
        ice_cap,
    )
}

fn causeway_material(environment: V3EnvironmentSettings) -> SolidMaterialRole {
    if environment == V3EnvironmentSettings::Volcanic {
        SolidMaterialRole::Basalt
    } else {
        SolidMaterialRole::Gravel
    }
}

fn set_land_surface_material(column: &mut VolumeColumn, material: SolidMaterialRole) {
    if let Some(VolumeElement::Solid(surface)) = column.elements.last_mut() {
        surface.material = material;
    }
}

fn route_membership(
    centerline: &[HexCoord],
    route: &BTreeSet<HexCoord>,
    surfaces: &BTreeMap<HexCoord, TilePos>,
) -> Result<ProtectedFeatureRoute, Vec<WorldValidationIssue>> {
    let exact = |coord: HexCoord| {
        surfaces.get(&coord).copied().ok_or_else(|| {
            vec![recipe_issue(format!(
                "Hills crossing route has no ordinary surface at {coord:?}"
            ))]
        })
    };
    Ok(ProtectedFeatureRoute {
        centerline: centerline
            .iter()
            .copied()
            .map(exact)
            .collect::<Result<_, _>>()?,
        surfaces: route.iter().copied().map(exact).collect::<Result<_, _>>()?,
    })
}

fn river_nodes(
    river: &BTreeSet<HexCoord>,
    orientation: u8,
    river_offset: i32,
    top_level: Level,
    fall: Option<(i32, Level)>,
    terminal_sources: &BTreeSet<HexCoord>,
) -> Result<BTreeMap<TilePos, LiquidNode>, Vec<WorldValidationIssue>> {
    let mut nodes = BTreeMap::new();
    for coord in river {
        let local = unrotate(*coord, orientation);
        let level = fall.map_or(top_level, |(target_y, height)| {
            if local.y() >= target_y {
                top_level.saturating_sub(height)
            } else {
                top_level
            }
        });
        let forward = rotate(
            HexCoord::from_axial(local.x(), local.y().saturating_add(1)),
            orientation,
        );
        let downstream_coord = if terminal_sources.contains(coord) {
            None
        } else if river.contains(&forward) {
            Some(forward)
        } else if local.x() != river_offset {
            let merge = rotate(HexCoord::from_axial(river_offset, local.y()), orientation);
            if !river.contains(&merge) || coord.distance(merge) != 1 {
                return Err(vec![recipe_issue(format!(
                    "Hills river lane cannot merge at boundary cell {coord:?}"
                ))]);
            }
            Some(merge)
        } else {
            None
        };
        let downstream = downstream_coord.map(|next| {
            let next_local = unrotate(next, orientation);
            let next_level = fall.map_or(top_level, |(target_y, height)| {
                if next_local.y() >= target_y {
                    top_level.saturating_sub(height)
                } else {
                    top_level
                }
            });
            TilePos::new(next, next_level)
        });
        nodes.insert(
            TilePos::new(*coord, level),
            LiquidNode {
                state: match downstream {
                    Some(next) if next.level < level => LiquidFlowState::Fall,
                    Some(_) => LiquidFlowState::Current,
                    None => LiquidFlowState::Still,
                },
                downstream,
            },
        );
    }
    Ok(nodes)
}

fn all_nodes_drain_to_sources(
    nodes: &BTreeMap<TilePos, LiquidNode>,
    sources: &BTreeSet<HexCoord>,
) -> bool {
    let terminals = sources
        .iter()
        .flat_map(|coord| {
            nodes
                .keys()
                .copied()
                .filter(move |position| position.coord == *coord)
        })
        .collect::<BTreeSet<_>>();
    nodes.keys().copied().all(|start| {
        let mut current = start;
        let mut visited = BTreeSet::new();
        loop {
            if !visited.insert(current) {
                return false;
            }
            let Some(node) = nodes.get(&current) else {
                return false;
            };
            let Some(next) = node.downstream else {
                return terminals.contains(&current);
            };
            current = next;
        }
    })
}

fn select_hill_centres(
    mask: &BTreeSet<HexCoord>,
    per_bank: u8,
    orientation: u8,
    river_offset: i32,
    stream: Option<SeedStream<'_>>,
    excluded: &BTreeSet<HexCoord>,
) -> Result<Vec<HexCoord>, Vec<WorldValidationIssue>> {
    let mut selected = Vec::new();
    for bank in [-1_i32, 1_i32] {
        let mut candidates: Vec<_> = mask
            .iter()
            .copied()
            .filter(|coord| {
                unrotate(*coord, orientation)
                    .x()
                    .saturating_sub(river_offset)
                    .signum()
                    == bank
            })
            .filter(|coord| !excluded.contains(coord))
            .collect();
        candidates.sort_by_key(|coord| {
            (
                stream.map_or(0, |stream| {
                    stream.sample_coord(*coord, bank.unsigned_abs().into())
                }),
                *coord,
            )
        });
        for coord in candidates {
            if selected
                .iter()
                .all(|centre: &HexCoord| centre.distance(coord) >= 3)
            {
                selected.push(coord);
                if selected
                    .iter()
                    .filter(|centre| {
                        unrotate(**centre, orientation)
                            .x()
                            .saturating_sub(river_offset)
                            .signum()
                            == bank
                    })
                    .count()
                    == usize::from(per_bank)
                {
                    break;
                }
            }
        }
    }
    let expected = usize::from(per_bank).saturating_mul(2);
    if selected.len() != expected {
        return Err(vec![recipe_issue(format!(
            "Hills patch placed {} separated centres; expected {expected}",
            selected.len()
        ))]);
    }
    Ok(selected)
}

fn fit_protected_routes(
    protected: &BTreeSet<HexCoord>,
    preferred: Level,
    levels: &mut BTreeMap<HexCoord, Level>,
) {
    for (coord, level) in levels {
        let distance = protected
            .iter()
            .map(|protected| protected.distance(*coord))
            .min()
            .unwrap_or(u32::MAX);
        let distance = i32::try_from(distance).unwrap_or(i32::MAX);
        *level = (*level).min(preferred.saturating_add(distance));
    }
}

fn restore_connected_relief(
    authored: &BTreeMap<HexCoord, Level>,
    crossings: &CrossingGeometry,
    valley: Level,
    seam_shape: &WalkerSeamShape,
    frozen_approaches: &BTreeSet<HexCoord>,
    levels: &mut BTreeMap<HexCoord, Level>,
) {
    loop {
        let candidates = authored
            .iter()
            .filter_map(|(coord, target)| {
                if frozen_approaches.contains(coord)
                    || seam_shape.is_boundary(*coord)
                    || seam_shape.required_surface(*coord).is_some()
                {
                    return None;
                }
                levels
                    .get(coord)
                    .copied()
                    .filter(|current| current < target)
                    .map(|current| (*coord, current, *target))
            })
            .collect::<Vec<_>>();
        let mut changed = false;
        for (coord, current, target) in candidates {
            levels.insert(coord, current.saturating_add(1).min(target));
            if hills_shaped_levels_connected(levels, crossings, valley, seam_shape) {
                changed = true;
            } else {
                levels.insert(coord, current);
            }
        }
        if !changed {
            break;
        }
    }
}

fn hills_shaped_levels_connected(
    levels: &BTreeMap<HexCoord, Level>,
    crossings: &CrossingGeometry,
    valley: Level,
    seam_shape: &WalkerSeamShape,
) -> bool {
    let ordinary_by_coord = levels
        .iter()
        .filter_map(|(coord, level)| {
            let position = if crossings.river.contains(coord) {
                if crossings.bridge_deck.contains(coord) {
                    Some(TilePos::new(*coord, valley.saturating_add(1)))
                } else if crossings.ford_deck.contains(coord)
                    || crossings.auxiliary_fords.contains(coord)
                {
                    Some(TilePos::new(*coord, valley))
                } else {
                    None
                }
            } else {
                Some(TilePos::new(*coord, *level))
            }?;
            (seam_shape.access_for(position, SurfaceAccess::Ordinary) == SurfaceAccess::Ordinary)
                .then_some((*coord, position.level))
        })
        .collect::<BTreeMap<_, _>>();
    let Some(start) = ordinary_by_coord.keys().next().copied() else {
        return false;
    };
    let mut visited = BTreeSet::from([start]);
    let mut frontier = VecDeque::from([start]);
    while let Some(coord) = frontier.pop_front() {
        let Some(level) = ordinary_by_coord.get(&coord).copied() else {
            continue;
        };
        for neighbor in coord.neighbors() {
            let Some(neighbor_level) = ordinary_by_coord.get(&neighbor).copied() else {
                continue;
            };
            if level.abs_diff(neighbor_level) <= 1 && visited.insert(neighbor) {
                frontier.push_back(neighbor);
            }
        }
    }
    visited.len() == ordinary_by_coord.len()
        && confluence_bank_authority_is_valid(&ordinary_by_coord, crossings)
}

fn confluence_bank_authority_is_valid(
    ordinary_by_coord: &BTreeMap<HexCoord, Level>,
    crossings: &CrossingGeometry,
) -> bool {
    let bridge = crossings
        .bridge_deck
        .intersection(&crossings.river)
        .copied()
        .collect::<BTreeSet<_>>();
    let alternate = crossings
        .ford_deck
        .intersection(&crossings.river)
        .copied()
        .collect::<BTreeSet<_>>();
    let wet = bridge
        .union(&alternate)
        .copied()
        .chain(crossings.auxiliary_fords.iter().copied())
        .collect::<BTreeSet<_>>();
    let mut component_by_coord = BTreeMap::new();
    let mut component_count = 0_usize;
    for start in ordinary_by_coord
        .keys()
        .copied()
        .filter(|coord| !wet.contains(coord))
    {
        if component_by_coord.contains_key(&start) {
            continue;
        }
        let mut reachable = BTreeSet::from([start]);
        let mut frontier = VecDeque::from([start]);
        while let Some(coord) = frontier.pop_front() {
            let Some(level) = ordinary_by_coord.get(&coord).copied() else {
                continue;
            };
            for neighbor in coord.neighbors() {
                let Some(neighbor_level) = ordinary_by_coord.get(&neighbor).copied() else {
                    continue;
                };
                if !wet.contains(&neighbor)
                    && level.abs_diff(neighbor_level) <= 1
                    && reachable.insert(neighbor)
                {
                    frontier.push_back(neighbor);
                }
            }
        }
        for coord in reachable {
            component_by_coord.insert(coord, component_count);
        }
        component_count = component_count.saturating_add(1);
    }
    let authorities = [bridge, alternate]
        .into_iter()
        .chain(
            crossings
                .auxiliary_fords
                .iter()
                .copied()
                .map(|coord| BTreeSet::from([coord])),
        )
        .collect::<Vec<_>>();
    let mut edges = Vec::with_capacity(authorities.len());
    for authority in authorities {
        let banks = authority
            .iter()
            .flat_map(|coord| coord.neighbors())
            .filter(|coord| !wet.contains(coord))
            .filter_map(|coord| component_by_coord.get(&coord).copied())
            .collect::<BTreeSet<_>>();
        let banks = banks.into_iter().collect::<Vec<_>>();
        let [first, second] = banks.as_slice() else {
            return false;
        };
        edges.push((*first, *second));
    }
    bank_graph_is_connected(component_count, &edges, &BTreeSet::new())
        && bank_graph_is_connected(component_count, &edges, &BTreeSet::from([0]))
        && bank_graph_is_connected(component_count, &edges, &BTreeSet::from([1]))
        && !bank_graph_is_connected(component_count, &edges, &BTreeSet::from([0, 1]))
        && (2..edges.len()).all(|index| {
            !bank_graph_is_connected(component_count, &edges, &BTreeSet::from([index]))
        })
}

fn add_irregular_shelves(
    centres: &[HexCoord],
    mask: &BTreeSet<HexCoord>,
    excluded: &BTreeSet<HexCoord>,
    crossings: &CrossingGeometry,
    valley: Level,
    levels: &mut BTreeMap<HexCoord, Level>,
) {
    let direction = HexCoord::from_axial(1, 0);
    let mut occupied = BTreeSet::new();
    for centre in centres {
        let direction_offset = centre
            .x()
            .saturating_mul(17)
            .saturating_add(centre.y().saturating_mul(31))
            .rem_euclid(6);
        for turn in 0..6_u8 {
            let turns = u8::try_from(direction_offset)
                .unwrap_or_default()
                .saturating_add(turn)
                % 6;
            let delta = rotate(direction, turns);
            let inner = shift(*centre, delta);
            let outer = shift(inner, delta);
            let beyond = shift(outer, delta);
            if [inner, outer, beyond]
                .into_iter()
                .any(|coord| !mask.contains(&coord) || excluded.contains(&coord))
                || occupied.contains(&inner)
                || occupied.contains(&outer)
            {
                continue;
            }
            let Some(peak_level) = levels.get(centre).copied() else {
                break;
            };
            let Some(beyond_level) = levels.get(&beyond).copied() else {
                continue;
            };
            if peak_level.saturating_sub(beyond_level) < 3 {
                continue;
            }
            let Some(previous_inner) = levels.get(&inner).copied() else {
                continue;
            };
            let Some(previous_outer) = levels.get(&outer).copied() else {
                continue;
            };
            levels.insert(inner, peak_level);
            levels.insert(outer, peak_level);
            if !hills_levels_connected(levels, mask, crossings, valley) {
                levels.insert(inner, previous_inner);
                levels.insert(outer, previous_outer);
                continue;
            }
            occupied.insert(inner);
            occupied.insert(outer);
            break;
        }
    }
}

fn hills_levels_connected(
    levels: &BTreeMap<HexCoord, Level>,
    mask: &BTreeSet<HexCoord>,
    crossings: &CrossingGeometry,
    valley: Level,
) -> bool {
    let ordinary_by_coord = mask
        .iter()
        .filter_map(|coord| {
            if crossings.river.contains(coord) {
                if crossings.bridge_deck.contains(coord) {
                    Some((*coord, valley.saturating_add(1)))
                } else if crossings.ford_deck.contains(coord)
                    || crossings.auxiliary_fords.contains(coord)
                {
                    Some((*coord, valley))
                } else {
                    None
                }
            } else {
                levels.get(coord).copied().map(|level| (*coord, level))
            }
        })
        .collect::<BTreeMap<_, _>>();
    let Some(start) = ordinary_by_coord.keys().next().copied() else {
        return false;
    };
    let mut visited = BTreeSet::from([start]);
    let mut frontier = VecDeque::from([start]);
    while let Some(coord) = frontier.pop_front() {
        let Some(level) = ordinary_by_coord.get(&coord).copied() else {
            continue;
        };
        for neighbor in coord.neighbors() {
            let Some(neighbor_level) = ordinary_by_coord.get(&neighbor).copied() else {
                continue;
            };
            if level.abs_diff(neighbor_level) <= 1 && visited.insert(neighbor) {
                frontier.push_back(neighbor);
            }
        }
    }
    visited.len() == ordinary_by_coord.len()
}

fn opposing_landings(
    coords: impl IntoIterator<Item = HexCoord>,
    frame: LocalPatchFrame,
    orientation: u8,
) -> Result<(HexCoord, HexCoord), Vec<WorldValidationIssue>> {
    let coords = coords
        .into_iter()
        .map(|coord| frame.to_local(coord).map(|local| (coord, local)))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            vec![recipe_issue(format!(
                "Hills landing conversion failed: {error}"
            ))]
        })?;
    let party = coords
        .iter()
        .copied()
        .min_by_key(|(coord, local)| (unrotate(*local, orientation).x(), *coord))
        .map(|(coord, _)| coord);
    let hostile = coords
        .iter()
        .copied()
        .max_by_key(|(coord, local)| (unrotate(*local, orientation).x(), *coord))
        .map(|(coord, _)| coord);
    match (party, hostile) {
        (Some(party), Some(hostile)) if party != hostile => Ok((party, hostile)),
        _ => Err(vec![recipe_issue(
            "Hills patch cannot fit two opposing actor landings",
        )]),
    }
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

fn land_column(surface: Level, environment: V3EnvironmentSettings) -> VolumeColumn {
    let surface_material = match environment {
        V3EnvironmentSettings::TemperateGrassland => SolidMaterialRole::Grass,
        V3EnvironmentSettings::Frozen => SolidMaterialRole::Snow,
        V3EnvironmentSettings::Volcanic => SolidMaterialRole::Basalt,
        V3EnvironmentSettings::Rocky => SolidMaterialRole::Stone,
    };
    let core_material = if environment == V3EnvironmentSettings::Volcanic {
        SolidMaterialRole::Basalt
    } else {
        SolidMaterialRole::Stone
    };
    VolumeColumn {
        elements: vec![
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(0, 1),
                material: SolidMaterialRole::Bedrock,
                cutaway_for: None,
            }),
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(1, surface.saturating_sub(3)),
                material: core_material,
                cutaway_for: None,
            }),
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(surface.saturating_sub(3), surface),
                material: SolidMaterialRole::Dirt,
                cutaway_for: None,
            }),
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(surface, surface.saturating_add(1)),
                material: surface_material,
                cutaway_for: None,
            }),
        ],
    }
}

fn validate_hills(
    plan: &GeneratedWorldPlan,
    settings: &V3HillsSettings,
    environment: V3EnvironmentSettings,
    vegetation: Option<&LandformVegetationSet>,
) -> WorldValidation<HillsMetrics> {
    validate_hills_inner(
        plan,
        settings,
        environment,
        vegetation,
        true,
        false,
        false,
        None,
        &BTreeSet::new(),
    )
}

fn validate_resolved_liquid_ports(
    patch: &PatchRecipeContext<'_>,
    fragment: &GeneratedPatchPlan,
    confluence: bool,
) -> Vec<WorldValidationIssue> {
    let nodes = fragment
        .liquids
        .bodies
        .values()
        .flat_map(|body| body.nodes.iter().map(|(position, node)| (*position, *node)))
        .collect::<BTreeMap<_, _>>();
    let wet_coords = nodes
        .keys()
        .map(|position| position.coord)
        .collect::<BTreeSet<_>>();
    let mut liquid_approaches = BTreeSet::new();
    let mut declared_boundary = BTreeSet::new();
    let mut shared_boundary = BTreeSet::new();
    let mut inlet_positions = BTreeSet::new();
    let mut outlet_positions = BTreeSet::new();
    let mut inlet_count = 0_usize;
    let mut outlet_count = 0_usize;
    let mut issues = Vec::new();

    for edge in patch.shared_edges() {
        shared_boundary.extend(edge.boundary_pairs().into_iter().map(|(inside, _)| inside));
        let Some((source, port)) = edge.liquid_port() else {
            continue;
        };
        if source {
            outlet_count = outlet_count.saturating_add(1);
        } else {
            inlet_count = inlet_count.saturating_add(1);
        }
        liquid_approaches.extend(port.first_approach.iter().copied());
        declared_boundary.extend(port.lanes.iter().map(|(inside, _)| *inside));
        for coord in &port.first_approach {
            let matching = nodes
                .iter()
                .filter(|(position, _)| {
                    position.coord == *coord
                        && edge.minimum_level() <= position.level
                        && position.level <= edge.maximum_level()
                })
                .count();
            if matching != 1 {
                issues.push(recipe_issue(format!(
                    "Hills liquid approach {coord:?} on edge {:?} has {matching} nodes inside its elevation band",
                    edge.id
                )));
            }
        }
        for coord in port.lanes.iter().map(|(inside, _)| *inside) {
            let matching = nodes
                .iter()
                .filter_map(|(position, node)| {
                    (position.coord == coord
                        && edge.minimum_level() <= position.level
                        && position.level <= edge.maximum_level())
                    .then_some((*position, *node))
                })
                .collect::<Vec<_>>();
            let [(position, node)] = matching.as_slice() else {
                issues.push(recipe_issue(format!(
                    "Hills liquid boundary {coord:?} on edge {:?} does not have one exact endpoint",
                    edge.id
                )));
                continue;
            };
            if source {
                outlet_positions.insert(*position);
                if node.state != LiquidFlowState::Still || node.downstream.is_some() {
                    issues.push(recipe_issue(format!(
                        "Hills outlet endpoint {position:?} must be Still before composition"
                    )));
                }
            } else {
                inlet_positions.insert(*position);
                if node.downstream.is_none() {
                    issues.push(recipe_issue(format!(
                        "Hills inlet endpoint {position:?} does not flow into the patch"
                    )));
                }
            }
        }
    }

    let walker_only = patch
        .protected_approaches()
        .difference(&liquid_approaches)
        .copied()
        .collect::<BTreeSet<_>>();
    if !walker_only.is_disjoint(&wet_coords) {
        issues.push(recipe_issue(
            "Hills confluence overlaps a non-liquid protected seam approach",
        ));
    }
    let undeclared_boundary = shared_boundary
        .intersection(&wet_coords)
        .copied()
        .filter(|coord| !declared_boundary.contains(coord))
        .collect::<BTreeSet<_>>();
    if !undeclared_boundary.is_empty() {
        issues.push(recipe_issue(format!(
            "Hills river reaches undeclared shared-boundary cells {undeclared_boundary:?}"
        )));
    }

    if confluence {
        if inlet_count < 2 || outlet_count != 1 {
            issues.push(recipe_issue(format!(
                "Hills confluence has {inlet_count} inlet edges and {outlet_count} outlet edges"
            )));
        }
        if !has_flow_merge(&nodes) {
            issues.push(recipe_issue(
                "Hills multi-inlet river has no actual flow merge",
            ));
        }
        for inlet in inlet_positions {
            if !liquid_path_reaches(&nodes, inlet, |position| {
                outlet_positions.contains(&position)
            }) {
                issues.push(recipe_issue(format!(
                    "Hills inlet {inlet:?} does not drain to the resolved outlet"
                )));
            }
        }
        if nodes.keys().copied().any(|start| {
            !liquid_path_reaches(&nodes, start, |position| {
                outlet_positions.contains(&position)
            })
        }) {
            issues.push(recipe_issue(
                "Hills confluence contains an internal terminal or cycle",
            ));
        }
    }
    issues
}

pub(crate) fn validate_patch(
    patch: PatchRecipeContext<'_>,
    fragment: &GeneratedPatchPlan,
    settings: &V3HillsSettings,
    environment: V3EnvironmentSettings,
    catalog: &RuntimeArtCatalog,
) -> WorldValidation<HillsMetrics> {
    let liquid_ports = match local_liquid_ports(
        &patch,
        match LocalPatchFrame::resolve(patch.mask(), patch.layout().kind, patch.grid_radius()) {
            Ok(frame) => frame,
            Err(error) => {
                return WorldValidation::Invalid(vec![recipe_issue(format!(
                    "Hills validation frame failed: {error}"
                ))]);
            }
        },
    ) {
        Ok(ports) => ports,
        Err(issues) => return WorldValidation::Invalid(issues),
    };
    let confluence = liquid_ports.iter().filter(|port| !port.source).count() > 1;
    if confluence {
        match confluence_main_ports(&liquid_ports) {
            Ok(_) => {}
            Err(issues) => return WorldValidation::Invalid(issues),
        }
    }
    let liquid_issues = validate_resolved_liquid_ports(&patch, fragment, confluence);
    if !liquid_issues.is_empty() {
        return WorldValidation::Invalid(liquid_issues);
    }
    let vegetation = if matches!(
        environment,
        V3EnvironmentSettings::TemperateGrassland | V3EnvironmentSettings::Frozen
    ) {
        match LandformVegetationSet::resolve(catalog, environment, "Hills") {
            Ok(vegetation) => Some(vegetation),
            Err(error) => return WorldValidation::Invalid(vec![recipe_issue(error)]),
        }
    } else {
        None
    };
    let frame =
        match LocalPatchFrame::resolve(patch.mask(), patch.layout().kind, patch.grid_radius()) {
            Ok(frame) => frame,
            Err(error) => {
                return WorldValidation::Invalid(vec![recipe_issue(format!(
                    "Hills validation frame failed: {error}"
                ))]);
            }
        };
    let protected_approaches = match patch
        .protected_approaches()
        .into_iter()
        .map(|coord| frame.to_local(coord).map_err(recipe_issue))
        .collect::<Result<BTreeSet<_>, _>>()
    {
        Ok(protected) => protected,
        Err(issue) => return WorldValidation::Invalid(vec![issue]),
    };
    match frame.canonical_local_world(fragment) {
        Ok(plan) => {
            let expected_auxiliary = if confluence {
                match rederive_confluence_auxiliary_crossings(
                    &plan,
                    &liquid_ports,
                    &protected_approaches,
                    frame.scale(),
                    settings.valley_level,
                ) {
                    Ok(expected) => Some(expected),
                    Err(issues) => return WorldValidation::Invalid(issues),
                }
            } else {
                None
            };
            validate_hills_inner(
                &plan,
                settings,
                environment,
                vegetation.as_ref(),
                false,
                patch.layout().kind.is_composite(),
                confluence,
                expected_auxiliary.as_ref(),
                &protected_approaches,
            )
        }
        Err(error) => WorldValidation::Invalid(vec![recipe_issue(format!(
            "Hills validation projection failed: {error}"
        ))]),
    }
}

fn validate_hills_inner(
    plan: &GeneratedWorldPlan,
    settings: &V3HillsSettings,
    environment: V3EnvironmentSettings,
    vegetation_objects: Option<&LandformVegetationSet>,
    validate_common: bool,
    composite_layout: bool,
    confluence: bool,
    expected_auxiliary_crossings: Option<&BTreeSet<TilePos>>,
    additional_vegetation_protected: &BTreeSet<HexCoord>,
) -> WorldValidation<HillsMetrics> {
    let mut issues = if validate_common {
        plan.validate()
    } else {
        Vec::new()
    };
    let ordinary = OrdinaryGraph::from_volume(&plan.volume, Some(&plan.blockers));
    let Some(party) = plan.anchors.get(PARTY_START).copied() else {
        issues.push(recipe_issue("Hills is missing party_start"));
        return WorldValidation::Invalid(issues);
    };
    let Some(hostile) = plan.anchors.get(HOSTILE_START).copied() else {
        issues.push(recipe_issue("Hills is missing hostile_start"));
        return WorldValidation::Invalid(issues);
    };
    let distances = ordinary.distances_from(party);
    if distances.len() != ordinary.len() {
        issues.push(recipe_issue(
            "Hills ordinary surfaces are not one walker-connected network",
        ));
    }
    let Some(critical_route_steps) = distances.get(&hostile).copied() else {
        issues.push(recipe_issue(
            "Hills actor anchors are not connected by ordinary movement",
        ));
        return WorldValidation::Invalid(issues);
    };
    let levels: BTreeSet<_> = ordinary
        .positions()
        .map(|position| position.level)
        .collect();
    let min = levels.iter().next().copied().unwrap_or_default();
    let max = levels.iter().next_back().copied().unwrap_or_default();
    let relief = max.saturating_sub(min);
    if relief > settings.max_relief {
        issues.push(recipe_issue(format!(
            "Hills relief {relief} exceeds configured maximum {}",
            settings.max_relief
        )));
    }
    let Some(bridge) = plan.features.protected_routes.get(BRIDGE_ROUTE) else {
        issues.push(recipe_issue("Hills is missing its direct bridge route"));
        return WorldValidation::Invalid(issues);
    };
    let Some(alternate) = plan.features.protected_routes.get(FORD_ROUTE) else {
        issues.push(recipe_issue(
            "Hills is missing its alternate crossing route",
        ));
        return WorldValidation::Invalid(issues);
    };
    let fill_coords: BTreeSet<_> = plan
        .volume
        .fill_runs_by_top()
        .keys()
        .map(|position| position.coord)
        .collect();
    let main_crossing_surfaces = bridge
        .surfaces
        .union(&alternate.surfaces)
        .copied()
        .collect::<BTreeSet<_>>();
    let auxiliary_crossings = plan
        .volume
        .surfaces
        .iter()
        .filter_map(|(position, metadata)| {
            (metadata.access == SurfaceAccess::Ordinary
                && fill_coords.contains(&position.coord)
                && !main_crossing_surfaces.contains(position))
            .then_some(*position)
        })
        .collect::<BTreeSet<_>>();
    let protected_surfaces = main_crossing_surfaces.clone();
    let all_crossing_surfaces = protected_surfaces
        .iter()
        .copied()
        .chain(auxiliary_crossings.iter().copied())
        .collect::<BTreeSet<_>>();
    let (two_level_cliffs, three_level_cliffs) =
        cliff_transition_counts(&ordinary, &all_crossing_surfaces);
    if settings.max_relief >= 10 {
        if relief < 10 {
            issues.push(recipe_issue(format!(
                "Hills reachable relief is {relief}; revised terrain requires at least 10"
            )));
        }
        if two_level_cliffs < 3 {
            issues.push(recipe_issue(format!(
                "Hills has {two_level_cliffs} non-route two-level cliff transitions; expected at least 3"
            )));
        }
        if three_level_cliffs == 0 {
            issues.push(recipe_issue(
                "Hills has no non-route cliff transition of at least three levels",
            ));
        }
    }
    validate_crossing_route("bridge", bridge, &ordinary, &mut issues);
    validate_crossing_route("alternate crossing", alternate, &ordinary, &mut issues);
    if !bridge.surfaces.is_disjoint(&alternate.surfaces) {
        issues.push(recipe_issue(
            "Hills direct and alternate crossing footprints overlap",
        ));
    }

    let validation_offsets: &[i32] = if composite_layout {
        &[0, -4, 4, -6, 6, -2, 2]
    } else {
        &[0]
    };
    let candidate_crossings = (0..6)
        .flat_map(|orientation| {
            validation_offsets
                .iter()
                .copied()
                .filter_map(move |offset| {
                    crossing_geometry(
                        &plan.layout.footprint,
                        orientation,
                        plan.layout.grid_radius,
                        offset,
                    )
                    .ok()
                })
        })
        .collect::<Vec<_>>();
    let exact_barrier = candidate_crossings
        .iter()
        .any(|geometry| geometry.river == fill_coords);
    let contains_main_trunk = candidate_crossings
        .iter()
        .any(|geometry| geometry.river.is_subset(&fill_coords));
    if !exact_barrier && !(confluence && contains_main_trunk) {
        issues.push(recipe_issue(if confluence {
            "Hills confluence does not preserve one complete three-wide crossing trunk"
        } else {
            "Hills liquid does not form the exact three-wide edge-to-edge barrier"
        }));
    }
    validate_auxiliary_crossings(
        plan,
        &fill_coords,
        &auxiliary_crossings,
        expected_auxiliary_crossings,
        environment,
        &mut issues,
    );
    validate_barrier_surfaces(
        plan,
        &fill_coords,
        bridge,
        alternate,
        &auxiliary_crossings,
        &mut issues,
    );
    validate_alternate_support(plan, &fill_coords, alternate, &mut issues);
    let frozen = ordinary
        .positions()
        .any(|position| solid_material_at(&plan.volume, position) == Some(SolidMaterialRole::Snow));
    validate_frozen_ice_caps(plan, frozen, &all_crossing_surfaces, &mut issues);
    validate_small_fall(plan, validate_common, &mut issues);
    let mut vegetation_reserved = fill_coords.clone();
    for coord in protected_surfaces
        .iter()
        .map(|surface| surface.coord)
        .chain(plan.anchors.values().map(|anchor| anchor.coord))
        .chain(
            plan.layout
                .shared_edges
                .values()
                .flat_map(|edge| edge.protected_approaches.values())
                .flatten()
                .copied()
                .chain(additional_vegetation_protected.iter().copied()),
        )
    {
        vegetation_reserved.extend(coord.within_radius(2));
    }
    let ordinary_surfaces = plan
        .volume
        .surfaces
        .iter()
        .filter_map(|(position, metadata)| {
            (metadata.access == SurfaceAccess::Ordinary).then_some((position.coord, *position))
        })
        .collect::<BTreeMap<_, _>>();
    let eligible_valley = ordinary_surfaces
        .values()
        .filter(|position| {
            position.level <= vegetation_valley_ceiling(settings, composite_layout)
                && !vegetation_reserved.contains(&position.coord)
        })
        .map(|position| position.coord)
        .collect::<BTreeSet<_>>();
    let eligible_dry = ordinary_surfaces
        .keys()
        .filter(|coord| !vegetation_reserved.contains(coord))
        .copied()
        .collect::<BTreeSet<_>>();
    let recipe_name = if environment == V3EnvironmentSettings::Frozen {
        "Frozen Hills"
    } else {
        "Hills"
    };
    let vegetation = if let Some(objects) = vegetation_objects {
        let no_nonvegetation_blockers = BTreeSet::new();
        match validate_landform_vegetation(
            recipe_name,
            objects,
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
        }
    } else {
        if !plan.blockers.is_empty() {
            issues.push(recipe_issue(
                "Hills without authored vegetation has undeclared blocker authority",
            ));
        }
        landform_vegetation_metrics(recipe_name, environment, plan.features.by_id.values())
            .unwrap_or_else(|error| {
                issues.push(recipe_issue(error));
                super::vegetation::LandformVegetationMetrics { trees: 0, grass: 0 }
            })
    };
    if matches!(
        environment,
        V3EnvironmentSettings::TemperateGrassland | V3EnvironmentSettings::Frozen
    ) && !(2..=5).contains(&vegetation.trees)
    {
        issues.push(recipe_issue(format!(
            "Hills has {} authored trees; expected 2 through 5",
            vegetation.trees
        )));
    }
    if plan
        .features
        .by_id
        .values()
        .any(|feature| match feature.kind {
            FeatureKind::Tree => !eligible_dry.contains(&feature.root.coord),
            FeatureKind::TallGrass => !eligible_valley.contains(&feature.root.coord),
            FeatureKind::CaveVegetation => true,
        })
    {
        issues.push(recipe_issue(
            "Hills authored vegetation leaves eligible dry terrain, the grass valley, or its protected clearances",
        ));
    }
    let valley_grass_percent = percent(vegetation.grass, eligible_valley.len());
    if matches!(
        environment,
        V3EnvironmentSettings::TemperateGrassland | V3EnvironmentSettings::Frozen
    ) && !(65..=80).contains(&valley_grass_percent)
    {
        issues.push(recipe_issue(format!(
            "Hills covers {valley_grass_percent}% of eligible valley surfaces with grass \
             ({}/{}); expected 65 through 80%",
            vegetation.grass,
            eligible_valley.len()
        )));
    }

    let bridge_barrier: BTreeSet<_> = bridge
        .surfaces
        .iter()
        .copied()
        .filter(|surface| fill_coords.contains(&surface.coord))
        .collect();
    let alternate_barrier: BTreeSet<_> = alternate
        .surfaces
        .iter()
        .copied()
        .filter(|surface| fill_coords.contains(&surface.coord))
        .collect();
    if bridge_barrier.is_empty() || alternate_barrier.is_empty() {
        issues.push(recipe_issue(
            "Hills crossing footprints do not span the liquid barrier",
        ));
    } else if confluence {
        validate_confluence_crossing_authority(
            &ordinary,
            &bridge_barrier,
            &alternate_barrier,
            &auxiliary_crossings,
            &mut issues,
        );
    } else {
        if !ordinary
            .reachable_avoiding(party, &alternate_barrier)
            .contains(&hostile)
        {
            issues.push(recipe_issue(
                "Hills direct bridge is not an independent bank-to-bank route",
            ));
        }
        if !ordinary
            .reachable_avoiding(party, &bridge_barrier)
            .contains(&hostile)
        {
            issues.push(recipe_issue(
                "Hills alternate crossing is not an independent bank-to-bank route",
            ));
        }
        let both_crossings: BTreeSet<_> =
            bridge_barrier.union(&alternate_barrier).copied().collect();
        if ordinary
            .reachable_avoiding(party, &both_crossings)
            .contains(&hostile)
        {
            issues.push(recipe_issue(
                "Hills banks remain connected after both declared crossings are removed",
            ));
        }
    }

    let bridge_structures: Vec<_> = plan
        .structures
        .by_id
        .values()
        .filter(|structure| structure.kind == StructureKind::Bridge)
        .collect();
    let bridge_is_valid = matches!(
        bridge_structures.as_slice(),
        [structure]
            if structure.voxels.iter().all(|position| {
                solid_material_at(&plan.volume, *position) == Some(SolidMaterialRole::Metal)
            })
    );
    if !bridge_is_valid {
        issues.push(recipe_issue(
            "Hills must contain exactly one all-metal bridge structure",
        ));
    }
    validate_shared_approaches(plan, &ordinary, &mut issues);
    if !issues.is_empty() {
        return WorldValidation::Invalid(issues);
    }
    WorldValidation::Valid(HillsMetrics {
        ordinary_surfaces: count_u32(ordinary.len()),
        reachable_elevation_levels: count_u32(levels.len()),
        relief,
        critical_route_steps,
        hill_centres: u32::from(settings.hills_per_bank).saturating_mul(2),
        barrier_cells: count_u32(fill_coords.len()),
        bridge_surfaces: count_u32(bridge.surfaces.len()),
        alternate_crossing_surfaces: count_u32(alternate.surfaces.len()),
        tree_roots: count_u32(vegetation.trees),
        grass_roots: count_u32(vegetation.grass),
        valley_grass_percent,
    })
}

fn validate_confluence_crossing_authority(
    ordinary: &OrdinaryGraph,
    bridge: &BTreeSet<TilePos>,
    alternate: &BTreeSet<TilePos>,
    auxiliary: &BTreeSet<TilePos>,
    issues: &mut Vec<WorldValidationIssue>,
) {
    let wet_ordinary = bridge
        .union(alternate)
        .copied()
        .chain(auxiliary.iter().copied())
        .collect::<BTreeSet<_>>();
    let mut component_by_surface = BTreeMap::new();
    let mut component_count = 0_usize;
    for start in ordinary
        .positions()
        .filter(|position| !wet_ordinary.contains(position))
    {
        if component_by_surface.contains_key(&start) {
            continue;
        }
        let reachable = ordinary.reachable_avoiding(start, &wet_ordinary);
        for surface in reachable {
            component_by_surface.insert(surface, component_count);
        }
        component_count = component_count.saturating_add(1);
    }

    let mut authorities = vec![
        ("bridge", bridge.clone()),
        ("alternate crossing", alternate.clone()),
    ];
    authorities.extend(
        auxiliary
            .iter()
            .copied()
            .map(|surface| ("auxiliary tributary crossing", BTreeSet::from([surface]))),
    );
    let mut bank_edges = Vec::with_capacity(authorities.len());
    for (name, authority) in &authorities {
        let Some(start) = authority.first().copied() else {
            issues.push(recipe_issue(format!(
                "Hills {name} has no liquid-barrier authority"
            )));
            return;
        };
        let mut authority_reachable = BTreeSet::from([start]);
        let mut authority_frontier = VecDeque::from([start]);
        while let Some(surface) = authority_frontier.pop_front() {
            for neighbor in ordinary.neighbors(surface) {
                if authority.contains(neighbor) && authority_reachable.insert(*neighbor) {
                    authority_frontier.push_back(*neighbor);
                }
            }
        }
        if !authority.is_subset(&authority_reachable) {
            issues.push(recipe_issue(format!(
                "Hills {name} liquid-barrier authority is not internally walkable"
            )));
        }
        let banks = authority
            .iter()
            .flat_map(|surface| ordinary.neighbors(*surface))
            .filter(|neighbor| !wet_ordinary.contains(neighbor))
            .filter_map(|neighbor| component_by_surface.get(neighbor).copied())
            .collect::<BTreeSet<_>>();
        let banks = banks.into_iter().collect::<Vec<_>>();
        let [first, second] = banks.as_slice() else {
            issues.push(recipe_issue(format!(
                "Hills {name} must join exactly two distinct dry-bank components; found {}",
                banks.len()
            )));
            return;
        };
        bank_edges.push((*first, *second));
    }

    if !bank_graph_is_connected(component_count, &bank_edges, &BTreeSet::new()) {
        issues.push(recipe_issue(
            "Hills declared confluence crossings do not connect every dry-bank component",
        ));
        return;
    }
    for (index, name) in ["bridge", "alternate crossing"].into_iter().enumerate() {
        if !bank_graph_is_connected(component_count, &bank_edges, &BTreeSet::from([index])) {
            issues.push(recipe_issue(format!(
                "Hills {name} is not independently redundant in the confluence bank graph"
            )));
        }
    }
    for index in 2..bank_edges.len() {
        if bank_graph_is_connected(component_count, &bank_edges, &BTreeSet::from([index])) {
            issues.push(recipe_issue(
                "Hills auxiliary tributary crossing is not a necessary dry-bank connection",
            ));
        }
    }
    if bank_graph_is_connected(
        component_count,
        &bank_edges,
        &BTreeSet::from([0_usize, 1_usize]),
    ) {
        issues.push(recipe_issue(
            "Hills confluence banks remain connected after both main crossings are removed",
        ));
    }
}

fn bank_graph_is_connected(
    component_count: usize,
    edges: &[(usize, usize)],
    removed_edges: &BTreeSet<usize>,
) -> bool {
    if component_count == 0 {
        return false;
    }
    let mut reachable = BTreeSet::from([0_usize]);
    let mut frontier = VecDeque::from([0_usize]);
    while let Some(component) = frontier.pop_front() {
        for (index, (first, second)) in edges.iter().copied().enumerate() {
            if removed_edges.contains(&index) {
                continue;
            }
            let neighbor = if first == component {
                Some(second)
            } else if second == component {
                Some(first)
            } else {
                None
            };
            match neighbor {
                Some(neighbor) if reachable.insert(neighbor) => frontier.push_back(neighbor),
                _ => {}
            }
        }
    }
    reachable.len() == component_count
}

fn validate_small_fall(
    plan: &GeneratedWorldPlan,
    required: bool,
    issues: &mut Vec<WorldValidationIssue>,
) {
    let fall_nodes = plan
        .liquids
        .bodies
        .values()
        .flat_map(|body| body.nodes.iter())
        .filter_map(|(position, node)| {
            (node.state == LiquidFlowState::Fall).then_some((*position, node.downstream))
        })
        .collect::<Vec<_>>();
    if !required {
        if !fall_nodes.is_empty() {
            issues.push(recipe_issue(
                "Composite Hills must preserve its level cross-patch liquid contract",
            ));
        }
        return;
    }
    if fall_nodes.len() != 3 {
        issues.push(recipe_issue(format!(
            "Hills requires one contiguous three-wide small fall, found {} nodes",
            fall_nodes.len()
        )));
        return;
    }
    let drops = fall_nodes
        .iter()
        .filter_map(|(position, downstream)| {
            downstream.map(|downstream| position.level.saturating_sub(downstream.level))
        })
        .collect::<BTreeSet<_>>();
    if drops != BTreeSet::from([SMALL_FALL_HEIGHT]) {
        issues.push(recipe_issue(format!(
            "Hills small fall must descend {SMALL_FALL_HEIGHT} levels in every lane"
        )));
    }
    let coords = fall_nodes
        .iter()
        .map(|(position, _)| position.coord)
        .collect::<BTreeSet<_>>();
    if coords.iter().any(|coord| {
        !coord
            .neighbors()
            .into_iter()
            .any(|neighbor| coords.contains(&neighbor))
    }) {
        issues.push(recipe_issue(
            "Hills small fall curtain is not horizontally contiguous",
        ));
    }
    let Some(bridge) = plan.features.protected_routes.get(BRIDGE_ROUTE) else {
        return;
    };
    let Some(alternate) = plan.features.protected_routes.get(FORD_ROUTE) else {
        return;
    };
    let bridge_coords = bridge
        .surfaces
        .iter()
        .map(|position| position.coord)
        .collect::<BTreeSet<_>>();
    let alternate_coords = alternate
        .surfaces
        .iter()
        .map(|position| position.coord)
        .collect::<BTreeSet<_>>();
    let liquid_nodes = plan
        .liquids
        .bodies
        .values()
        .flat_map(|body| body.nodes.iter().map(|(position, node)| (*position, *node)))
        .collect::<BTreeMap<_, _>>();
    let fall_is_between_crossings = fall_nodes.iter().all(|(fall, _)| {
        liquid_nodes
            .keys()
            .copied()
            .filter(|position| bridge_coords.contains(&position.coord))
            .any(|bridge| liquid_path_reaches(&liquid_nodes, bridge, |position| position == *fall))
            && liquid_path_reaches(&liquid_nodes, *fall, |position| {
                alternate_coords.contains(&position.coord)
            })
    });
    if !fall_is_between_crossings {
        issues.push(recipe_issue(
            "Hills small fall must remain topologically between the bridge and alternate passage",
        ));
    }
}

fn liquid_path_reaches(
    nodes: &BTreeMap<TilePos, LiquidNode>,
    start: TilePos,
    mut target: impl FnMut(TilePos) -> bool,
) -> bool {
    let mut current = Some(start);
    let mut visited = BTreeSet::new();
    while let Some(position) = current {
        if target(position) {
            return true;
        }
        if !visited.insert(position) {
            return false;
        }
        current = nodes.get(&position).and_then(|node| node.downstream);
    }
    false
}

fn validate_frozen_ice_caps(
    plan: &GeneratedWorldPlan,
    frozen: bool,
    protected: &BTreeSet<TilePos>,
    issues: &mut Vec<WorldValidationIssue>,
) {
    let ice_caps = plan
        .volume
        .surfaces
        .iter()
        .filter_map(|(position, metadata)| {
            (solid_material_at(&plan.volume, *position) == Some(SolidMaterialRole::Ice))
                .then_some((*position, *metadata))
        })
        .collect::<Vec<_>>();
    if !frozen {
        if !ice_caps.is_empty() {
            issues.push(recipe_issue(
                "Non-frozen Hills contains frozen shoreline ice caps",
            ));
        }
        return;
    }
    if !(3..=7).contains(&ice_caps.len()) {
        issues.push(recipe_issue(format!(
            "Frozen Hills has {} shoreline ice caps; expected 3 through 7",
            ice_caps.len()
        )));
    }
    if ice_caps
        .iter()
        .any(|(_, metadata)| metadata.access != SurfaceAccess::NonStandable)
    {
        issues.push(recipe_issue(
            "Frozen Hills ice caps must be visible nonstandable solids outside every movement network",
        ));
    }
    let liquid_coords = plan
        .volume
        .fill_runs_by_top()
        .keys()
        .map(|position| position.coord)
        .collect::<BTreeSet<_>>();
    if ice_caps
        .iter()
        .any(|(position, _)| !liquid_coords.contains(&position.coord))
    {
        issues.push(recipe_issue(
            "Frozen Hills ice caps must sit over authored river cells",
        ));
    }
    if ice_caps
        .iter()
        .any(|(position, _)| protected.iter().any(|route| route.coord == position.coord))
    {
        issues.push(recipe_issue(
            "Frozen Hills ice caps overlap a protected river crossing",
        ));
    }
    for (index, (first, _)) in ice_caps.iter().enumerate() {
        if ice_caps
            .iter()
            .skip(index.saturating_add(1))
            .any(|(second, _)| first.coord.distance(second.coord) <= 1)
        {
            issues.push(recipe_issue(
                "Frozen Hills shoreline ice caps must remain isolated",
            ));
            break;
        }
    }
    let Some(bridge) = plan.features.protected_routes.get(BRIDGE_ROUTE) else {
        return;
    };
    let bridge_coords = bridge
        .surfaces
        .iter()
        .map(|position| position.coord)
        .collect::<BTreeSet<_>>();
    let liquid_nodes = plan
        .liquids
        .bodies
        .values()
        .flat_map(|body| body.nodes.iter().map(|(position, node)| (*position, *node)))
        .collect::<BTreeMap<_, _>>();
    let caps_are_upstream_shoreline = ice_caps.iter().all(|(ice, _)| {
        let shoreline = ice.coord.neighbors().into_iter().any(|neighbor| {
            plan.layout.footprint.contains(&neighbor) && !liquid_coords.contains(&neighbor)
        });
        let upstream = liquid_nodes
            .keys()
            .copied()
            .filter(|position| position.coord == ice.coord)
            .any(|position| {
                liquid_path_reaches(&liquid_nodes, position, |downstream| {
                    bridge_coords.contains(&downstream.coord)
                })
            });
        shoreline && upstream
    });
    if !caps_are_upstream_shoreline {
        issues.push(recipe_issue(
            "Frozen Hills ice caps must remain on the upstream shoreline lanes",
        ));
    }
    let flowing_lanes = plan
        .liquids
        .bodies
        .values()
        .flat_map(|body| body.nodes.iter())
        .filter(|(position, node)| {
            bridge_coords.contains(&position.coord) && node.downstream.is_some()
        })
        .map(|(position, _)| position.coord)
        .collect::<BTreeSet<_>>();
    if flowing_lanes.len() < 2 {
        issues.push(recipe_issue(
            "Frozen Hills must preserve at least two flowing river lanes",
        ));
    }
}

fn cliff_transition_counts(
    ordinary: &OrdinaryGraph,
    protected: &BTreeSet<TilePos>,
) -> (usize, usize) {
    let by_coord = ordinary
        .positions()
        .map(|position| (position.coord, position))
        .collect::<BTreeMap<_, _>>();
    let mut two_level = 0_usize;
    let mut three_level = 0_usize;
    for (coord, position) in &by_coord {
        for neighbor_coord in coord.neighbors() {
            if neighbor_coord <= *coord {
                continue;
            }
            let Some(neighbor) = by_coord.get(&neighbor_coord).copied() else {
                continue;
            };
            if protected.contains(position) || protected.contains(&neighbor) {
                continue;
            }
            match position.level.abs_diff(neighbor.level) {
                2 => two_level = two_level.saturating_add(1),
                3.. => three_level = three_level.saturating_add(1),
                _ => {}
            }
        }
    }
    (two_level, three_level)
}

fn validate_crossing_route(
    name: &str,
    route: &ProtectedFeatureRoute,
    ordinary: &OrdinaryGraph,
    issues: &mut Vec<WorldValidationIssue>,
) {
    if route.surfaces.len() < route.centerline.len().saturating_mul(2) {
        issues.push(recipe_issue(format!(
            "Hills {name} is not two ordinary surfaces wide"
        )));
    }
    for pair in route.centerline.windows(2) {
        if !matches!(pair, [from, to] if ordinary.admits(*from, *to)) {
            issues.push(recipe_issue(format!(
                "Hills {name} centerline contains an illegal walker transition"
            )));
        }
    }
    for center in &route.centerline {
        if !route
            .surfaces
            .iter()
            .any(|surface| surface.coord != center.coord && ordinary.admits(*center, *surface))
        {
            issues.push(recipe_issue(format!(
                "Hills {name} has no walkable second lane at {center:?}"
            )));
        }
    }
}

fn validate_barrier_surfaces(
    plan: &GeneratedWorldPlan,
    fill_coords: &BTreeSet<HexCoord>,
    bridge: &ProtectedFeatureRoute,
    alternate: &ProtectedFeatureRoute,
    auxiliary_crossings: &BTreeSet<TilePos>,
    issues: &mut Vec<WorldValidationIssue>,
) {
    let declared_crossing_coords: BTreeSet<_> = bridge
        .surfaces
        .union(&alternate.surfaces)
        .copied()
        .chain(auxiliary_crossings.iter().copied())
        .map(|surface| surface.coord)
        .collect();
    for coord in fill_coords {
        let ordinary_surfaces: Vec<_> = plan
            .volume
            .surfaces
            .iter()
            .filter_map(|(surface, metadata)| {
                (surface.coord == *coord && metadata.access == SurfaceAccess::Ordinary)
                    .then_some(*surface)
            })
            .collect();
        if declared_crossing_coords.contains(coord) {
            if ordinary_surfaces.len() != 1 {
                issues.push(recipe_issue(format!(
                    "Hills declared crossing cell {coord:?} has {} ordinary surfaces",
                    ordinary_surfaces.len()
                )));
            }
        } else if !ordinary_surfaces.is_empty() {
            issues.push(recipe_issue(format!(
                "Hills liquid barrier is accidentally standable at {coord:?}"
            )));
        }
    }
}

fn validate_auxiliary_crossings(
    plan: &GeneratedWorldPlan,
    fill_coords: &BTreeSet<HexCoord>,
    auxiliary_crossings: &BTreeSet<TilePos>,
    expected_crossings: Option<&BTreeSet<TilePos>>,
    environment: V3EnvironmentSettings,
    issues: &mut Vec<WorldValidationIssue>,
) {
    let expected_count = expected_crossings.map_or(0, BTreeSet::len);
    if auxiliary_crossings.len() != expected_count {
        issues.push(recipe_issue(format!(
            "Hills confluence has {} auxiliary tributary crossings; expected {expected_count}",
            auxiliary_crossings.len()
        )));
    }
    if expected_crossings.is_some_and(|expected| expected != auxiliary_crossings) {
        issues.push(recipe_issue(
            "Hills auxiliary tributary crossings do not match their rederived liquid-branch authority",
        ));
    }
    for surface in auxiliary_crossings {
        if !plan.biome_regions.contains_key(surface) {
            issues.push(recipe_issue(format!(
                "Hills auxiliary tributary crossing {surface:?} is missing exact biome-region membership"
            )));
        }
        if !fill_coords.contains(&surface.coord) {
            issues.push(recipe_issue(format!(
                "Hills auxiliary tributary crossing {surface:?} leaves the liquid barrier"
            )));
        }
        if solid_material_at(&plan.volume, *surface) != Some(causeway_material(environment)) {
            issues.push(recipe_issue(format!(
                "Hills auxiliary tributary crossing {surface:?} does not use the exact causeway material"
            )));
        }
        if !surface_has_contiguous_support_from_fill(&plan.volume, *surface) {
            issues.push(recipe_issue(format!(
                "Hills auxiliary tributary crossing has an unsupported vertical gap below {surface:?}"
            )));
        }
    }
}

fn validate_alternate_support(
    plan: &GeneratedWorldPlan,
    fill_coords: &BTreeSet<HexCoord>,
    alternate: &ProtectedFeatureRoute,
    issues: &mut Vec<WorldValidationIssue>,
) {
    for surface in alternate
        .surfaces
        .iter()
        .filter(|surface| fill_coords.contains(&surface.coord))
    {
        if !surface_has_contiguous_support_from_fill(&plan.volume, *surface) {
            issues.push(recipe_issue(format!(
                "Hills alternate passage has an unsupported vertical gap below {surface:?}"
            )));
        }
    }
}

fn surface_has_contiguous_support_from_fill(volume: &VolumePlan, surface: TilePos) -> bool {
    let Some(column) = volume.columns.get(&surface.coord) else {
        return false;
    };
    let Some(fill_top) = column
        .elements
        .iter()
        .filter_map(|element| {
            let VolumeElement::Fill(fill) = element else {
                return None;
            };
            Some(fill.levels.top)
        })
        .max()
    else {
        return false;
    };
    fill_top <= surface.level
        && (fill_top..=surface.level).all(|level| {
            column.elements.iter().any(|element| {
                let VolumeElement::Solid(mass) = element else {
                    return false;
                };
                mass.levels.bottom <= level && level < mass.levels.top
            })
        })
}

fn solid_material_at(volume: &VolumePlan, position: TilePos) -> Option<SolidMaterialRole> {
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

fn validate_shared_approaches(
    plan: &GeneratedWorldPlan,
    ordinary: &OrdinaryGraph,
    issues: &mut Vec<WorldValidationIssue>,
) {
    for edge in plan.layout.shared_edges.values() {
        for (patch, approaches) in &edge.protected_approaches {
            for coord in approaches {
                let Some(position) = plan
                    .volume
                    .surfaces
                    .keys()
                    .find(|position| position.coord == *coord)
                    .copied()
                else {
                    issues.push(recipe_issue(format!(
                        "Hills patch {} has no seam approach surface at {coord:?}",
                        patch.0
                    )));
                    continue;
                };
                if position.level != edge.elevation.preferred {
                    issues.push(recipe_issue(format!(
                        "Hills seam approach {position:?} does not use preferred level {}",
                        edge.elevation.preferred
                    )));
                }
                if !ordinary.contains(position) {
                    issues.push(recipe_issue(format!(
                        "Hills seam approach {position:?} is not ordinary footing"
                    )));
                }
            }
        }
    }
}

fn hills_view_hint(
    mask: &BTreeSet<HexCoord>,
    levels: &BTreeMap<HexCoord, Level>,
    grid_radius: u32,
    level_height: f32,
) -> Result<MapViewHint, Vec<WorldValidationIssue>> {
    let focus_level = if levels.is_empty() {
        0.0
    } else {
        let total = levels.values().try_fold(0.0_f32, |sum, level| {
            i16::try_from(*level)
                .map(|level| sum + f32::from(level))
                .map_err(|error| {
                    vec![recipe_issue(format!(
                        "Hills camera level does not fit inside i16: {error}"
                    ))]
                })
        })?;
        let count = f32::from(u16::try_from(levels.len()).map_err(|error| {
            vec![recipe_issue(format!(
                "Hills camera footprint does not fit inside u16: {error}"
            ))]
        })?);
        (total / count) * level_height
    };
    let radius = u16::try_from(grid_radius)
        .map_err(|error| vec![recipe_issue(format!("Hills camera radius: {error}"))])?;
    let frame = f32::from(radius).mul_add(3.2, 8.0);
    let center = mask
        .iter()
        .copied()
        .min_by_key(|coord| coord.distance(HexCoord::ORIGIN))
        .unwrap_or(HexCoord::ORIGIN);
    let horizontal_offset = f32::from(i16::try_from(center.x()).map_err(|error| {
        vec![recipe_issue(format!(
            "Hills camera coordinate does not fit inside i16: {error}"
        ))]
    })?) * 0.5;
    Ok(MapViewHint::new(
        (horizontal_offset, focus_level + frame * 0.75, frame * 0.8),
        (horizontal_offset, focus_level, 0.0),
    ))
}

fn recipe_issue(detail: impl Into<String>) -> WorldValidationIssue {
    WorldValidationIssue::new(WorldIssueCode::Recipe("hills"), detail)
}

fn count_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn percent(part: usize, total: usize) -> u32 {
    count_u32(part)
        .saturating_mul(100)
        .checked_div(count_u32(total))
        .unwrap_or_default()
}

fn hills_grass_target(eligible: usize) -> Result<usize, String> {
    if eligible == 0 {
        return Err("Hills has no eligible valley surface for authored grass".to_owned());
    }
    let minimum = eligible.saturating_mul(65).div_ceil(100);
    let maximum = eligible.saturating_mul(80) / 100;
    if minimum > maximum {
        return Err(format!(
            "Hills cannot realize integral 65-80% grass coverage across {eligible} eligible valley surfaces"
        ));
    }
    Ok(eligible
        .saturating_mul(HILLS_GRASS_PERCENT)
        .div_ceil(100)
        .clamp(minimum, maximum))
}

pub(super) fn vegetation_valley_ceiling(
    settings: &V3HillsSettings,
    composite_layout: bool,
) -> Level {
    let lower_band = if composite_layout {
        settings.max_relief.saturating_div(2)
    } else {
        settings.max_relief.saturating_div(3)
    };
    settings.valley_level.saturating_add(lower_band.max(2))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{
        PatchEdgeContractSettings, PatchEdgesSettings, PatchMaskSettings, PatchSpec,
    };

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
            layout: V3LayoutSettings::Single(PatchSpec {
                environment: V3EnvironmentSettings::TemperateGrassland,
                recipe: V3RecipeSettings::Hills(V3HillsSettings {
                    valley_level: 15,
                    max_relief: 8,
                    hills_per_bank: 3,
                }),
                overlays: Vec::new(),
                mask: PatchMaskSettings::WholeWorld,
                edges: world_boundaries(),
            }),
        }
    }

    #[test]
    fn native_hills_are_deterministic_connected_and_stratified() {
        let settings = settings();
        let first = generate(
            12,
            0.4,
            &settings,
            883,
            super::super::vegetation::tests::runtime_art_catalog(),
        )
        .expect("valid Hills");
        let second = generate(
            12,
            0.4,
            &settings,
            883,
            super::super::vegetation::tests::runtime_art_catalog(),
        )
        .expect("same valid Hills");
        assert_eq!(
            first.validated.semantic_fingerprint,
            second.validated.semantic_fingerprint
        );
        assert_eq!(first.metrics.ordinary_surfaces, 405);
        assert_eq!(first.metrics.hill_centres, 6);
        assert_eq!(first.metrics.barrier_cells, 73);
        assert_eq!(first.metrics.bridge_surfaces, 14);
        assert_eq!(first.metrics.alternate_crossing_surfaces, 14);
        assert!(first.metrics.relief <= 8);
        assert!(first.metrics.reachable_elevation_levels >= 2);
        assert_eq!(first.metrics.tree_roots, 3);
        assert!((65..=80).contains(&first.metrics.valley_grass_percent));
        assert!(first.metrics.grass_roots > 0);

        let plan = &first.validated.plan;
        assert_eq!(plan.volume.columns.len(), 469);
        assert!(plan.volume.mask.iter().all(|coord| {
            solid_material_at(&plan.volume, TilePos::new(*coord, 0))
                == Some(SolidMaterialRole::Bedrock)
        }));
        assert!(plan
            .liquids
            .bodies
            .values()
            .all(|body| body.material == FillMaterialRole::Water));
        let fall_nodes = plan
            .liquids
            .bodies
            .values()
            .flat_map(|body| body.nodes.values())
            .filter(|node| node.state == LiquidFlowState::Fall)
            .count();
        assert_eq!(fall_nodes, 3);
        assert_eq!(
            plan.structures
                .by_id
                .values()
                .filter(|structure| structure.kind == StructureKind::Bridge)
                .count(),
            1
        );
        let ordinary = OrdinaryGraph::from_volume(&plan.volume, Some(&plan.blockers));
        let alternate = plan
            .features
            .protected_routes
            .get(FORD_ROUTE)
            .expect("Hills should retain the alternate passage");
        assert!(alternate
            .surfaces
            .iter()
            .filter(|surface| {
                plan.volume
                    .fill_runs_by_top()
                    .keys()
                    .any(|fill| fill.coord == surface.coord)
            })
            .all(|surface| surface_has_contiguous_support_from_fill(&plan.volume, *surface)));
        assert!(alternate
            .centerline
            .windows(2)
            .all(|pair| matches!(pair, [from, to] if ordinary.admits(*from, *to))));
    }

    #[test]
    fn frozen_retains_water_and_volcanic_uses_lava_and_basalt() {
        let mut frozen = settings();
        let V3LayoutSettings::Single(frozen_patch) = &mut frozen.layout else {
            unreachable!("test uses Single")
        };
        frozen_patch.environment = V3EnvironmentSettings::Frozen;
        let frozen = generate(
            12,
            0.4,
            &frozen,
            91,
            super::super::vegetation::tests::runtime_art_catalog(),
        )
        .expect("valid Frozen Hills");
        assert_eq!(frozen.metrics.tree_roots, 3);
        assert!((65..=80).contains(&frozen.metrics.valley_grass_percent));
        assert!(frozen
            .validated
            .plan
            .features
            .by_id
            .values()
            .all(|feature| feature.object_id.as_str().contains("snowy-")));
        assert!(frozen
            .validated
            .plan
            .liquids
            .bodies
            .values()
            .all(|body| body.material == FillMaterialRole::Water));
        let frozen_ice_caps = frozen
            .validated
            .plan
            .volume
            .surfaces
            .iter()
            .filter(|(position, metadata)| {
                solid_material_at(&frozen.validated.plan.volume, **position)
                    == Some(SolidMaterialRole::Ice)
                    && metadata.access == SurfaceAccess::NonStandable
            })
            .count();
        assert_eq!(frozen_ice_caps, FROZEN_ICE_CAP_TARGET);
        assert!(frozen
            .validated
            .plan
            .volume
            .surfaces
            .values()
            .all(|metadata| !matches!(metadata.access, SurfaceAccess::SpecialMovement(_))));

        let mut volcanic = settings();
        let V3LayoutSettings::Single(volcanic_patch) = &mut volcanic.layout else {
            unreachable!("test uses Single")
        };
        volcanic_patch.environment = V3EnvironmentSettings::Volcanic;
        let volcanic = generate(
            12,
            0.4,
            &volcanic,
            91,
            super::super::vegetation::tests::runtime_art_catalog(),
        )
        .expect("valid Volcanic Hills");
        let plan = &volcanic.validated.plan;
        assert!(plan
            .liquids
            .bodies
            .values()
            .all(|body| body.material == FillMaterialRole::Lava));
        let ford = plan
            .features
            .protected_routes
            .get(FORD_ROUTE)
            .expect("alternate crossing");
        let liquid_coords: BTreeSet<_> = plan
            .volume
            .fill_runs_by_top()
            .keys()
            .map(|position| position.coord)
            .collect();
        assert!(ford
            .centerline
            .iter()
            .filter(|surface| liquid_coords.contains(&surface.coord))
            .all(|surface| solid_material_at(&plan.volume, *surface)
                == Some(SolidMaterialRole::Basalt)));
    }

    #[test]
    fn revised_hills_reach_ten_levels_with_irregular_non_route_cliffs() {
        for environment in [
            V3EnvironmentSettings::TemperateGrassland,
            V3EnvironmentSettings::Frozen,
        ] {
            let mut revised = settings();
            let V3LayoutSettings::Single(patch) = &mut revised.layout else {
                unreachable!("test uses Single")
            };
            patch.environment = environment;
            let V3RecipeSettings::Hills(hills) = &mut patch.recipe else {
                unreachable!("test uses Hills")
            };
            hills.max_relief = 12;
            let selected = generate(
                12,
                0.4,
                &revised,
                1_592_598_566,
                super::super::vegetation::tests::runtime_art_catalog(),
            )
            .expect("revised Hills should generate");
            assert!(selected.metrics.relief >= 10);
            let plan = &selected.validated.plan;
            let ordinary = OrdinaryGraph::from_volume(&plan.volume, Some(&plan.blockers));
            let protected = plan
                .features
                .protected_routes
                .values()
                .flat_map(|route| route.surfaces.iter().copied())
                .collect();
            let (two_level, three_level) = cliff_transition_counts(&ordinary, &protected);
            assert!(two_level >= 3, "two-level cliff transitions: {two_level}");
            assert!(
                three_level >= 1,
                "three-level cliff transitions: {three_level}"
            );
        }
    }

    #[test]
    fn validator_reprojects_complete_tree_volume_after_root_and_blocker_corruption() {
        let mut revised = settings();
        let V3LayoutSettings::Single(patch) = &mut revised.layout else {
            unreachable!("test uses Single")
        };
        let V3RecipeSettings::Hills(hills) = &mut patch.recipe else {
            unreachable!("test uses Hills")
        };
        hills.max_relief = 12;
        let hills = hills.clone();
        let catalog = super::super::vegetation::tests::runtime_art_catalog();
        let selected = generate(12, 0.4, &revised, 1_592_598_566, catalog)
            .expect("revised Hills should generate");
        let mut corrupted = selected.validated.plan;
        let target = TilePos::new(HexCoord::from_axial(-12, 3), 21);
        assert_eq!(
            corrupted
                .volume
                .surfaces
                .get(&target)
                .map(|metadata| metadata.access),
            Some(SurfaceAccess::Ordinary),
            "the corruption must retain a superficially valid ordinary root"
        );
        let feature_id = corrupted
            .features
            .by_id
            .iter()
            .find_map(|(id, feature)| (feature.kind == FeatureKind::Tree).then_some(*id))
            .expect("Hills should contain one tree feature");
        let old_blockers = corrupted
            .features
            .by_id
            .get(&feature_id)
            .expect("selected tree should remain present")
            .blocker_footprint
            .clone();
        for blocker in old_blockers {
            corrupted.blockers.remove(&blocker);
        }
        let feature = corrupted
            .features
            .by_id
            .get_mut(&feature_id)
            .expect("selected tree should remain mutable");
        feature.root = target;
        feature.object_id =
            hex_assets::ObjectAssetId::new(super::super::vegetation::SMALL_BROADLEAF_ID)
                .expect("tracked object id");
        feature.rotation = hex_assets::HexObjectRotation::new(0).expect("zero rotation");
        feature.blocker_footprint = BTreeSet::from([target]);
        corrupted.blockers.insert(target);

        let vegetation = LandformVegetationSet::resolve(
            catalog,
            V3EnvironmentSettings::TemperateGrassland,
            "Hills",
        )
        .expect("accepted temperate vegetation");
        let WorldValidation::Invalid(issues) = validate_hills(
            &corrupted,
            &hills,
            V3EnvironmentSettings::TemperateGrassland,
            Some(&vegetation),
        ) else {
            panic!("a tree whose complete authored volume leaves valid support was accepted");
        };
        assert!(
            issues.iter().any(|issue| {
                issue.detail.contains("plant/small-broadleaf")
                    && (issue.detail.contains("leaves or intersects")
                        || issue.detail.contains("reserved column"))
            }),
            "missing independently projected authored-volume issue: {issues:#?}"
        );
    }

    #[test]
    fn revised_hills_pr_corpus_validates_128_seeds_and_named_regressions() {
        let mut seeds = (0_u64..128).collect::<BTreeSet<_>>();
        seeds.insert(1_592_598_566);
        for environment in [
            V3EnvironmentSettings::TemperateGrassland,
            V3EnvironmentSettings::Frozen,
        ] {
            let mut revised = settings();
            let V3LayoutSettings::Single(patch) = &mut revised.layout else {
                unreachable!("test uses Single")
            };
            patch.environment = environment;
            let V3RecipeSettings::Hills(hills) = &mut patch.recipe else {
                unreachable!("test uses Hills")
            };
            hills.max_relief = 12;
            let mut fallback_seeds = Vec::new();
            for seed in &seeds {
                let selected = generate(
                    12,
                    0.4,
                    &revised,
                    *seed,
                    super::super::vegetation::tests::runtime_art_catalog(),
                )
                .unwrap_or_else(|error| panic!("{environment:?} Hills seed {seed}: {error}"));
                if selected.used_fallback {
                    fallback_seeds.push(*seed);
                }
            }
            assert!(
                fallback_seeds.len().saturating_mul(100) < seeds.len(),
                "{}/{} {environment:?} Hills seeds used fallback: {fallback_seeds:?}",
                fallback_seeds.len(),
                seeds.len()
            );
        }
    }

    #[test]
    fn rocky_hills_fail_instead_of_fabricating_a_plan() {
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
