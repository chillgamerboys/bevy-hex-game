//! Native V3 rolling-hills geometry.
//!
//! Height cones are one-Lipschitz, so the generated ordinary surface remains
//! walker-connected by construction. Shared-edge approaches clamp those cones to
//! the resolved seam datum without a post-generation blend pass.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use hex_assets::RuntimeArtCatalog;
use hex_core::{HexCoord, Level, MapViewHint, TilePos};

use super::composition::{compose_single_patch, GeneratedPatchPlan};
use super::layout::{resolve_layout, PatchId, ResolvedLayoutPlan};
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
    let (orientation, river_offset, local_crossings, local_river_nodes) = (0..6)
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
        })?;
    let mut excluded = protected;
    excluded.extend(local_crossings.protected.iter().copied());
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

    let mut columns = BTreeMap::new();
    let mut surfaces = BTreeMap::new();
    let mut ordinary_by_coord = BTreeMap::new();
    for (coord, level) in &surface_by_coord {
        let bridge = crossings.bridge_deck.contains(coord);
        let ford = crossings.ford_deck.contains(coord);
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

#[derive(Debug)]
struct LocalLiquidPort {
    source: bool,
    boundary: BTreeSet<HexCoord>,
    approach: BTreeSet<HexCoord>,
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
        .filter_map(|edge| edge.liquid_port())
        .map(|(source, port)| {
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
            })
        })
        .collect()
}

#[derive(Debug)]
struct CrossingGeometry {
    river: BTreeSet<HexCoord>,
    bridge_deck: BTreeSet<HexCoord>,
    ford_deck: BTreeSet<HexCoord>,
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
        bridge_centerline: centerline(0),
        bridge_route,
        ford_centerline: centerline(ford_y),
        ford_route,
        protected,
        fall_target_y: (ford_y / 2).max(1),
    })
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
                } else if crossings.ford_deck.contains(coord) {
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
        &BTreeSet::new(),
    )
}

pub(crate) fn validate_patch(
    patch: PatchRecipeContext<'_>,
    fragment: &GeneratedPatchPlan,
    settings: &V3HillsSettings,
    environment: V3EnvironmentSettings,
    catalog: &RuntimeArtCatalog,
) -> WorldValidation<HillsMetrics> {
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
        Ok(plan) => validate_hills_inner(
            &plan,
            settings,
            environment,
            vegetation.as_ref(),
            false,
            patch.layout().kind.is_composite(),
            &protected_approaches,
        ),
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
    let protected_surfaces = bridge
        .surfaces
        .union(&alternate.surfaces)
        .copied()
        .collect::<BTreeSet<_>>();
    let (two_level_cliffs, three_level_cliffs) =
        cliff_transition_counts(&ordinary, &protected_surfaces);
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

    let fill_coords: BTreeSet<_> = plan
        .volume
        .fill_runs_by_top()
        .keys()
        .map(|position| position.coord)
        .collect();
    let matching_orientation = (0..6).find(|orientation| {
        crossing_geometry(
            &plan.layout.footprint,
            *orientation,
            plan.layout.grid_radius,
            0,
        )
        .is_ok_and(|geometry| geometry.river == fill_coords)
    });
    if matching_orientation.is_none() {
        issues.push(recipe_issue(
            "Hills liquid does not form the exact three-wide edge-to-edge barrier",
        ));
    }
    validate_barrier_surfaces(plan, &fill_coords, bridge, alternate, &mut issues);
    validate_alternate_support(plan, &fill_coords, alternate, &mut issues);
    let frozen = ordinary
        .positions()
        .any(|position| solid_material_at(&plan.volume, position) == Some(SolidMaterialRole::Snow));
    validate_frozen_ice_caps(plan, frozen, &protected_surfaces, &mut issues);
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
    issues: &mut Vec<WorldValidationIssue>,
) {
    let declared_crossing_coords: BTreeSet<_> = bridge
        .surfaces
        .union(&alternate.surfaces)
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
