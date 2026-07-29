//! Native V3 rolling-hills geometry.
//!
//! Height cones are one-Lipschitz, so the generated ordinary surface remains
//! walker-connected by construction. Shared-edge approaches clamp those cones to
//! the resolved seam datum without a post-generation blend pass.

use std::collections::{BTreeMap, BTreeSet};

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
}

#[derive(Debug)]
struct HillsRecipe {
    level_height: f32,
    layout: ResolvedLayoutPlan,
    settings: V3HillsSettings,
    environment: V3EnvironmentSettings,
}

/// Runs the common eight-candidate selector for one native V3 Hills world.
pub(crate) fn generate(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV3Settings,
    seed: u64,
) -> Result<ValidatedWorldSelection<HillsMetrics>, V3GenerationError> {
    if !level_height.is_finite() || level_height <= 0.0 {
        return Err(V3GenerationError::RecipeContract(
            "Hills level height must be positive and finite".to_owned(),
        ));
    }
    let (hills, environment) = recipe_settings(settings)?;
    let layout = resolve_layout(grid_radius, settings)
        .map_err(|error| V3GenerationError::RecipeContract(error.to_string()))?;
    run_recipe(
        &HillsRecipe {
            level_height,
            layout,
            settings: hills.clone(),
            environment,
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
        let fragment = construct_patch(
            patch,
            &self.settings,
            self.environment,
            self.level_height,
            PatchBuildMode::Candidate {
                world_seed: context.seed,
                candidate: context.candidate,
            },
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
        validate_hills(plan, &self.settings)
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
        let fragment = construct_patch(
            patch,
            &self.settings,
            self.environment,
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
    }
}

pub(crate) fn construct_patch(
    patch: PatchRecipeContext<'_>,
    settings: &V3HillsSettings,
    environment: V3EnvironmentSettings,
    level_height: f32,
    mode: PatchBuildMode,
) -> Result<GeneratedPatchPlan, Vec<WorldValidationIssue>> {
    let streams = mode.seed_streams(&patch);
    construct_patch_with_streams(
        patch,
        settings,
        environment,
        level_height,
        streams.map(|streams| {
            (
                streams.stage("hills.orientation"),
                streams.stage("hills.centres"),
            )
        }),
    )
}

pub(crate) fn construct_patch_with_streams(
    patch: PatchRecipeContext<'_>,
    settings: &V3HillsSettings,
    environment: V3EnvironmentSettings,
    level_height: f32,
    streams: Option<(SeedStream<'_>, SeedStream<'_>)>,
) -> Result<GeneratedPatchPlan, Vec<WorldValidationIssue>> {
    let frame = LocalPatchFrame::resolve(patch.mask(), patch.layout().kind, patch.grid_radius())
        .map_err(|error| vec![recipe_issue(format!("Hills local frame failed: {error}"))])?;
    let local_mask = frame.local_mask(patch.mask()).map_err(|error| {
        vec![recipe_issue(format!(
            "Hills local mask conversion failed: {error}"
        ))]
    })?;
    let requested_orientation = streams.map_or(0, |(orientation, _)| {
        u8::try_from(orientation.sample(0) % 6).unwrap_or_default()
    });
    let centre_stream = streams.map(|(_, centres)| centres);
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
                        &liquid_sources,
                    )
                    .ok()
                    .filter(|nodes| {
                        liquid_ports
                            .iter()
                            .all(|port| port.accepts_nodes(nodes, river_level))
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
    let crossings = local_crossings.into_world(frame)?;
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
            let (column, bed, crossing) =
                river_column(*coord, settings.valley_level, environment, bridge, ford);
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
    let features = FeaturePlan {
        by_id: BTreeMap::new(),
        protected_routes: BTreeMap::from([
            (BRIDGE_ROUTE.to_owned(), bridge_route),
            (FORD_ROUTE.to_owned(), ford_route),
        ]),
        clearings: BTreeMap::new(),
    };
    let liquid_material = if environment == V3EnvironmentSettings::Volcanic {
        FillMaterialRole::Lava
    } else {
        FillMaterialRole::Water
    };
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
        .collect::<Result<_, Vec<WorldValidationIssue>>>()?;
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
        blockers: BTreeSet::new(),
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
        })
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
    })
}

fn river_column(
    coord: HexCoord,
    valley: Level,
    environment: V3EnvironmentSettings,
    bridge: bool,
    ford: bool,
) -> (VolumeColumn, TilePos, Option<TilePos>) {
    let bed_level = valley.saturating_sub(RIVER_DEPTH);
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
            levels: LevelInterval::new(valley.saturating_sub(2), valley),
            material: fill,
        }),
    ];
    let crossing = if ford {
        let surface = TilePos::new(coord, valley);
        elements.push(VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(surface.level, surface.level.saturating_add(1)),
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
    (
        VolumeColumn { elements },
        TilePos::new(coord, bed_level),
        crossing,
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
    terminal_sources: &BTreeSet<HexCoord>,
) -> Result<BTreeMap<TilePos, LiquidNode>, Vec<WorldValidationIssue>> {
    let mut nodes = BTreeMap::new();
    for coord in river {
        let local = unrotate(*coord, orientation);
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
        nodes.insert(
            TilePos::new(*coord, top_level),
            LiquidNode {
                state: if downstream_coord.is_some() {
                    LiquidFlowState::Current
                } else {
                    LiquidFlowState::Still
                },
                downstream: downstream_coord.map(|next| TilePos::new(next, top_level)),
            },
        );
    }
    Ok(nodes)
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
) -> WorldValidation<HillsMetrics> {
    let mut issues = plan.validate();
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
    })
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
        let first = generate(12, 0.4, &settings, 883).expect("valid Hills");
        let second = generate(12, 0.4, &settings, 883).expect("same valid Hills");
        assert_eq!(
            first.validated.semantic_fingerprint,
            second.validated.semantic_fingerprint
        );
        assert_eq!(first.metrics.ordinary_surfaces, 408);
        assert_eq!(first.metrics.hill_centres, 6);
        assert_eq!(first.metrics.barrier_cells, 73);
        assert_eq!(first.metrics.bridge_surfaces, 14);
        assert_eq!(first.metrics.alternate_crossing_surfaces, 14);
        assert!(first.metrics.relief <= 8);
        assert!(first.metrics.reachable_elevation_levels >= 2);

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
        assert_eq!(
            plan.structures
                .by_id
                .values()
                .filter(|structure| structure.kind == StructureKind::Bridge)
                .count(),
            1
        );
    }

    #[test]
    fn frozen_retains_water_and_volcanic_uses_lava_and_basalt() {
        let mut frozen = settings();
        let V3LayoutSettings::Single(frozen_patch) = &mut frozen.layout else {
            unreachable!("test uses Single")
        };
        frozen_patch.environment = V3EnvironmentSettings::Frozen;
        let frozen = generate(12, 0.4, &frozen, 91).expect("valid Frozen Hills");
        assert!(frozen
            .validated
            .plan
            .liquids
            .bodies
            .values()
            .all(|body| body.material == FillMaterialRole::Water));

        let mut volcanic = settings();
        let V3LayoutSettings::Single(volcanic_patch) = &mut volcanic.layout else {
            unreachable!("test uses Single")
        };
        volcanic_patch.environment = V3EnvironmentSettings::Volcanic;
        let volcanic = generate(12, 0.4, &volcanic, 91).expect("valid Volcanic Hills");
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
    fn rocky_hills_fail_instead_of_fabricating_a_plan() {
        let mut settings = settings();
        let V3LayoutSettings::Single(patch) = &mut settings.layout else {
            unreachable!("test uses Single")
        };
        patch.environment = V3EnvironmentSettings::Rocky;
        assert!(generate(12, 0.4, &settings, 1).is_err());
    }
}
