//! Patch-local semantic plans and checked whole-world composition for V3.
//!
//! Recipes construct one [`GeneratedPatchPlan`] against an exact resolved mask.
//! Composition namespaces every map-local identity, merges complete fragments, and
//! only then admits the unchanged strict [`GeneratedWorldPlan`] validator.

use std::collections::{BTreeMap, BTreeSet};

use hex_core::{HexCoord, InteriorRegionId, MapViewHint, SpecialMovementRegion, TilePos};

use super::layout::{
    HexSide, LayoutKind, PatchId, ResolvedEdgeId, ResolvedEdgeReference, ResolvedLayoutPlan,
    ResolvedLiquidPort, ResolvedPatch,
};
use super::liquid::{LiquidBodyId, LiquidBodyPlan, LiquidFlowState, LiquidNode, LiquidPlan};
use super::volume::{FillMaterialRole, SurfaceAccess, VolumeElement, VolumePlan};
use super::world::GeneratedWorldPlan;
use super::world::{
    FeatureId, FeaturePlan, InteriorPlan, LightId, PlannedGameplayLight, StructureId,
    StructurePlan, WorldIssueCode, WorldValidationIssue,
};

const PATCH_NAMESPACE_BITS: u32 = 4;
const LOCAL_ID_BITS: u32 = u32::BITS - PATCH_NAMESPACE_BITS;
const MAX_PATCH_ID: u32 = (1 << PATCH_NAMESPACE_BITS) - 1;
const MAX_LOCAL_ID: u32 = (1 << LOCAL_ID_BITS) - 1;

/// Complete semantic output owned by exactly one resolved V3 patch.
#[derive(Debug, Clone)]
pub(crate) struct GeneratedPatchPlan {
    pub(crate) patch_id: PatchId,
    pub(crate) volume: VolumePlan,
    pub(crate) liquids: LiquidPlan,
    pub(crate) features: FeaturePlan,
    pub(crate) structures: StructurePlan,
    pub(crate) blockers: BTreeSet<TilePos>,
    pub(crate) lights: BTreeMap<LightId, PlannedGameplayLight>,
    pub(crate) biome_regions: BTreeMap<TilePos, hex_core::BiomeRegionId>,
    pub(crate) interiors: InteriorPlan,
    pub(crate) anchors: BTreeMap<String, TilePos>,
    pub(crate) view_hint: MapViewHint,
}

impl GeneratedPatchPlan {
    /// Checks exact patch ownership and all recipe-independent semantic contracts.
    ///
    /// The whole resolved layout remains authoritative, but shared seams are
    /// projected to world boundaries for local validation. Cross-patch contracts
    /// are checked only after every fragment has been composed.
    #[must_use]
    pub(crate) fn validate_against(
        &self,
        layout: &ResolvedLayoutPlan,
    ) -> Vec<PatchValidationIssue> {
        let mut issues = Vec::new();
        if let Err(error) = layout.validate() {
            issues.extend(error.issues().iter().map(|issue| {
                PatchValidationIssue::new(self.patch_id, PatchIssueCode::Layout, issue.to_string())
            }));
        }

        let Some(resolved_patch) = layout.patches.get(&self.patch_id) else {
            issues.push(PatchValidationIssue::new(
                self.patch_id,
                PatchIssueCode::MissingPatch,
                format!("resolved layout does not contain patch {}", self.patch_id.0),
            ));
            return issues;
        };
        if self.volume.mask != resolved_patch.mask {
            let missing = resolved_patch
                .mask
                .difference(&self.volume.mask)
                .copied()
                .collect::<Vec<_>>();
            let extra = self
                .volume
                .mask
                .difference(&resolved_patch.mask)
                .copied()
                .collect::<Vec<_>>();
            issues.push(PatchValidationIssue::new(
                self.patch_id,
                PatchIssueCode::Mask,
                format!(
                    "semantic patch mask does not exactly match resolved ownership \
                     (missing {missing:?}, extra {extra:?})"
                ),
            ));
            return issues;
        }

        let isolated = self.isolated_world_unchecked(layout, resolved_patch);
        issues.extend(isolated.validate().into_iter().map(|issue| {
            PatchValidationIssue::new(
                self.patch_id,
                PatchIssueCode::Semantic(issue.code),
                issue.detail,
            )
        }));
        issues
    }

    pub(crate) fn isolated_world(
        &self,
        layout: &ResolvedLayoutPlan,
    ) -> Result<GeneratedWorldPlan, Vec<PatchValidationIssue>> {
        let issues = self.validate_against(layout);
        if !issues.is_empty() {
            return Err(issues);
        }
        let Some(resolved_patch) = layout.patches.get(&self.patch_id) else {
            return Err(vec![PatchValidationIssue::new(
                self.patch_id,
                PatchIssueCode::MissingPatch,
                format!("resolved layout does not contain patch {}", self.patch_id.0),
            )]);
        };
        Ok(self.isolated_world_unchecked(layout, resolved_patch))
    }

    fn isolated_world_unchecked(
        &self,
        layout: &ResolvedLayoutPlan,
        resolved_patch: &ResolvedPatch,
    ) -> GeneratedWorldPlan {
        let edges = HexSide::ALL
            .into_iter()
            .map(|side| (side, ResolvedEdgeReference::WorldBoundary))
            .collect();
        let isolated_layout = ResolvedLayoutPlan {
            kind: LayoutKind::Single,
            grid_radius: layout.grid_radius,
            footprint: resolved_patch.mask.clone(),
            patches: BTreeMap::from([(
                self.patch_id,
                ResolvedPatch {
                    biome_region: resolved_patch.biome_region,
                    mask: resolved_patch.mask.clone(),
                    edges,
                },
            )]),
            shared_edges: BTreeMap::new(),
        };
        GeneratedWorldPlan {
            layout: isolated_layout,
            volume: self.volume.clone(),
            liquids: self.liquids.clone(),
            features: self.features.clone(),
            structures: self.structures.clone(),
            blockers: self.blockers.clone(),
            lights: self.lights.clone(),
            biome_regions: self.biome_regions.clone(),
            interiors: self.interiors.clone(),
            anchors: self.anchors.clone(),
            view_hint: self.view_hint,
        }
    }

    fn namespace(mut self, namespace_names: bool) -> Result<Self, WorldCompositionError> {
        let patch = self.patch_id;

        for column in self.volume.columns.values_mut() {
            for element in &mut column.elements {
                let VolumeElement::Solid(mass) = element else {
                    continue;
                };
                if let Some(region) = mass.cutaway_for {
                    mass.cutaway_for = Some(InteriorRegionId(namespace_numeric(
                        patch,
                        region.0,
                        NamespaceKind::Interior,
                    )?));
                }
            }
        }
        for metadata in self.volume.surfaces.values_mut() {
            if let Some(region) = metadata.interior {
                metadata.interior = Some(InteriorRegionId(namespace_numeric(
                    patch,
                    region.0,
                    NamespaceKind::Interior,
                )?));
            }
            if let SurfaceAccess::SpecialMovement(region) = metadata.access {
                metadata.access = SurfaceAccess::SpecialMovement(SpecialMovementRegion(
                    namespace_numeric(patch, region.0, NamespaceKind::SpecialMovement)?,
                ));
            }
        }

        self.liquids.bodies = std::mem::take(&mut self.liquids.bodies)
            .into_iter()
            .map(|(id, body)| {
                namespace_numeric(patch, id.0, NamespaceKind::Liquid)
                    .map(|id| (LiquidBodyId(id), body))
            })
            .collect::<Result<_, _>>()?;
        self.features.by_id = std::mem::take(&mut self.features.by_id)
            .into_iter()
            .map(|(id, feature)| {
                namespace_numeric(patch, id.0, NamespaceKind::Feature)
                    .map(|id| (FeatureId(id), feature))
            })
            .collect::<Result<_, _>>()?;
        if namespace_names {
            self.features.protected_routes =
                namespace_named_map(patch, std::mem::take(&mut self.features.protected_routes));
            self.features.clearings =
                namespace_named_map(patch, std::mem::take(&mut self.features.clearings));
        }
        self.structures.by_id = std::mem::take(&mut self.structures.by_id)
            .into_iter()
            .map(|(id, structure)| {
                namespace_numeric(patch, id.0, NamespaceKind::Structure)
                    .map(|id| (StructureId(id), structure))
            })
            .collect::<Result<_, _>>()?;
        self.lights = std::mem::take(&mut self.lights)
            .into_iter()
            .map(|(id, light)| {
                namespace_numeric(patch, id.0, NamespaceKind::Light).map(|id| (LightId(id), light))
            })
            .collect::<Result<_, _>>()?;
        self.interiors.by_id = std::mem::take(&mut self.interiors.by_id)
            .into_iter()
            .map(|(id, interior)| {
                namespace_numeric(patch, id.0, NamespaceKind::Interior)
                    .map(|id| (InteriorRegionId(id), interior))
            })
            .collect::<Result<_, _>>()?;
        if namespace_names {
            self.anchors = namespace_named_map(patch, std::mem::take(&mut self.anchors));
        }
        Ok(self)
    }
}

fn namespace_named_map<T>(patch: PatchId, values: BTreeMap<String, T>) -> BTreeMap<String, T> {
    values
        .into_iter()
        .map(|(name, value)| (namespace_name(patch, &name), value))
        .collect()
}

fn namespace_name(patch: PatchId, local: &str) -> String {
    format!("{}_{}", patch_slug(patch), local)
}

fn patch_slug(patch: PatchId) -> String {
    match patch.0 {
        0 => "center".to_owned(),
        1 => "mountains".to_owned(),
        2 => "waterfall".to_owned(),
        3 => "forest".to_owned(),
        4 => "fort".to_owned(),
        5 => "caves".to_owned(),
        6 => "sky_islands".to_owned(),
        id => format!("patch_{id}"),
    }
}

fn namespace_numeric(
    patch: PatchId,
    local: u32,
    kind: NamespaceKind,
) -> Result<u32, WorldCompositionError> {
    if patch.0 > MAX_PATCH_ID || local > MAX_LOCAL_ID {
        return Err(WorldCompositionError::NamespaceOverflow { patch, kind, local });
    }
    Ok((patch.0 << LOCAL_ID_BITS) | local)
}

/// Recipe-independent category for a patch-local validation issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PatchIssueCode {
    Layout,
    MissingPatch,
    Mask,
    Semantic(WorldIssueCode),
}

/// One exact patch contract violation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PatchValidationIssue {
    pub(crate) patch: PatchId,
    pub(crate) code: PatchIssueCode,
    pub(crate) detail: String,
}

impl PatchValidationIssue {
    fn new(patch: PatchId, code: PatchIssueCode, detail: impl Into<String>) -> Self {
        Self {
            patch,
            code,
            detail: detail.into(),
        }
    }
}

/// Semantic ID family whose local value is namespaced during composition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NamespaceKind {
    Liquid,
    Feature,
    Structure,
    Light,
    Interior,
    SpecialMovement,
}

/// Stable local anchor reference used to publish a world-level canonical alias.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PatchAnchorRef {
    pub(crate) patch: PatchId,
    pub(crate) local_name: String,
}

/// Inputs owned by the whole-world planner rather than any individual recipe.
#[derive(Debug, Clone)]
pub(crate) struct WorldCompositionSettings {
    pub(crate) canonical_anchors: BTreeMap<String, PatchAnchorRef>,
    pub(crate) view_hint: MapViewHint,
}

/// Collision category reported before a partially merged world can escape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CollisionKind {
    Column,
    Surface,
    Liquid,
    Feature,
    FeatureMembership,
    Structure,
    Blocker,
    Light,
    Biome,
    Interior,
    Anchor,
}

/// Why complete patch fragments could not form one strict whole-world plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorldCompositionError {
    InvalidLayout(Vec<String>),
    MissingFragment(PatchId),
    DuplicateFragment(PatchId),
    UnexpectedFragment(PatchId),
    InvalidPatch {
        patch: PatchId,
        issues: Vec<PatchValidationIssue>,
    },
    LiquidSeam {
        edge: ResolvedEdgeId,
        issue: LiquidSeamIssue,
    },
    NamespaceOverflow {
        patch: PatchId,
        kind: NamespaceKind,
        local: u32,
    },
    Collision {
        kind: CollisionKind,
        key: String,
    },
    InvalidCanonicalAnchorName(String),
    MissingCanonicalAnchor {
        alias: String,
        patch: PatchId,
        local_name: String,
    },
    FinalValidation(Vec<WorldValidationIssue>),
}

/// Exact reason a declared directed liquid seam could not be stitched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LiquidSeamIssue {
    MissingEndpoint {
        patch: PatchId,
        coord: hex_core::HexCoord,
    },
    AmbiguousEndpoint {
        patch: PatchId,
        coord: hex_core::HexCoord,
        positions: Vec<TilePos>,
    },
    MaterialMismatch {
        source: FillMaterialRole,
        sink: FillMaterialRole,
    },
    SourceAlreadyFlows {
        position: TilePos,
        downstream: TilePos,
    },
    SourceIsNotStill {
        position: TilePos,
        state: LiquidFlowState,
    },
    Uphill {
        source: TilePos,
        sink: TilePos,
    },
    ExcessiveDrop {
        source: TilePos,
        sink: TilePos,
    },
    MissingMergedBody(LiquidBodyId),
    MissingMergedNode(TilePos),
}

/// Merges complete patch fragments into one strict whole-world semantic plan.
pub(crate) fn compose_world(
    layout: ResolvedLayoutPlan,
    fragments: Vec<GeneratedPatchPlan>,
    settings: WorldCompositionSettings,
) -> Result<GeneratedWorldPlan, WorldCompositionError> {
    if let Err(error) = layout.validate() {
        return Err(WorldCompositionError::InvalidLayout(
            error.issues().iter().map(ToString::to_string).collect(),
        ));
    }
    let namespace_names = layout.kind != LayoutKind::Single;
    let mut by_patch = BTreeMap::new();
    for fragment in fragments {
        let patch = fragment.patch_id;
        if !layout.patches.contains_key(&patch) {
            return Err(WorldCompositionError::UnexpectedFragment(patch));
        }
        if by_patch.insert(patch, fragment).is_some() {
            return Err(WorldCompositionError::DuplicateFragment(patch));
        }
    }
    for patch in layout.patches.keys().copied() {
        if !by_patch.contains_key(&patch) {
            return Err(WorldCompositionError::MissingFragment(patch));
        }
    }
    for (patch, fragment) in &by_patch {
        let issues = fragment.validate_against(&layout);
        if !issues.is_empty() {
            return Err(WorldCompositionError::InvalidPatch {
                patch: *patch,
                issues,
            });
        }
    }

    let mut volume = VolumePlan {
        mask: layout.footprint.clone(),
        columns: BTreeMap::new(),
        surfaces: BTreeMap::new(),
    };
    let mut liquids = LiquidPlan::default();
    let mut features = FeaturePlan::default();
    let mut structures = StructurePlan::default();
    let mut blockers = BTreeSet::new();
    let mut lights = BTreeMap::new();
    let mut biome_regions = BTreeMap::new();
    let mut interiors = InteriorPlan::default();
    let mut anchors = BTreeMap::new();

    for fragment in by_patch.into_values() {
        let fragment = fragment.namespace(namespace_names)?;
        merge_map(
            &mut volume.columns,
            fragment.volume.columns,
            CollisionKind::Column,
        )?;
        merge_map(
            &mut volume.surfaces,
            fragment.volume.surfaces,
            CollisionKind::Surface,
        )?;
        merge_map(
            &mut liquids.bodies,
            fragment.liquids.bodies,
            CollisionKind::Liquid,
        )?;
        merge_map(
            &mut features.by_id,
            fragment.features.by_id,
            CollisionKind::Feature,
        )?;
        merge_feature_memberships(
            &mut features,
            fragment.features.protected_routes,
            fragment.features.clearings,
        )?;
        merge_map(
            &mut structures.by_id,
            fragment.structures.by_id,
            CollisionKind::Structure,
        )?;
        merge_set(&mut blockers, fragment.blockers, CollisionKind::Blocker)?;
        merge_map(&mut lights, fragment.lights, CollisionKind::Light)?;
        merge_map(
            &mut biome_regions,
            fragment.biome_regions,
            CollisionKind::Biome,
        )?;
        merge_map(
            &mut interiors.by_id,
            fragment.interiors.by_id,
            CollisionKind::Interior,
        )?;
        merge_map(&mut anchors, fragment.anchors, CollisionKind::Anchor)?;
    }
    stitch_directed_liquid_seams(&layout, &mut liquids)?;

    for (alias, target) in settings.canonical_anchors {
        if !super::world::valid_stable_name(&alias) {
            return Err(WorldCompositionError::InvalidCanonicalAnchorName(alias));
        }
        let namespaced = namespace_name(target.patch, &target.local_name);
        let Some(position) = anchors.get(&namespaced).copied() else {
            return Err(WorldCompositionError::MissingCanonicalAnchor {
                alias,
                patch: target.patch,
                local_name: target.local_name,
            });
        };
        insert_unique(&mut anchors, alias, position, CollisionKind::Anchor)?;
    }

    let world = GeneratedWorldPlan {
        layout,
        volume,
        liquids,
        features,
        structures,
        blockers,
        lights,
        biome_regions,
        interiors,
        anchors,
        view_hint: settings.view_hint,
    };
    let issues = world.validate();
    if issues.is_empty() {
        Ok(world)
    } else {
        Err(WorldCompositionError::FinalValidation(issues))
    }
}

#[derive(Debug, Clone, Copy)]
struct LiquidEndpoint {
    body: LiquidBodyId,
    position: TilePos,
    node: LiquidNode,
    material: FillMaterialRole,
}

#[derive(Debug, Clone, Copy)]
struct LiquidSeamLink {
    edge: ResolvedEdgeId,
    source: LiquidEndpoint,
    sink: LiquidEndpoint,
}

fn stitch_directed_liquid_seams(
    layout: &ResolvedLayoutPlan,
    liquids: &mut LiquidPlan,
) -> Result<(), WorldCompositionError> {
    let mut links = Vec::new();
    for (edge_id, edge) in &layout.shared_edges {
        let ResolvedLiquidPort::Directed { source, sink, port } = &edge.liquid else {
            continue;
        };
        let source_is_first = *source == edge.first.0 && *sink == edge.second.0;
        let source_is_second = *source == edge.second.0 && *sink == edge.first.0;
        if !source_is_first && !source_is_second {
            continue;
        }

        for (first_coord, second_coord) in &port.lanes {
            let (source_coord, sink_coord) = if source_is_first {
                (*first_coord, *second_coord)
            } else {
                (*second_coord, *first_coord)
            };
            let source_endpoint = unique_liquid_endpoint(
                *edge_id,
                *source,
                source_coord,
                edge.elevation.min,
                edge.elevation.max,
                liquids,
            )?;
            let sink_endpoint = unique_liquid_endpoint(
                *edge_id,
                *sink,
                sink_coord,
                edge.elevation.min,
                edge.elevation.max,
                liquids,
            )?;
            validate_liquid_link(*edge_id, source_endpoint, sink_endpoint)?;
            links.push(LiquidSeamLink {
                edge: *edge_id,
                source: source_endpoint,
                sink: sink_endpoint,
            });
        }
    }
    let Some(first_link) = links.first() else {
        return Ok(());
    };
    let first_edge = first_link.edge;

    let mut parents: BTreeMap<_, _> = liquids
        .bodies
        .keys()
        .copied()
        .map(|body| (body, body))
        .collect();
    for link in &links {
        union_liquid_bodies(&mut parents, link.source.body, link.sink.body);
    }
    let roots: BTreeMap<_, _> = parents
        .keys()
        .copied()
        .map(|body| (body, liquid_body_root(&parents, body)))
        .collect();

    let mut merged = BTreeMap::<LiquidBodyId, LiquidBodyPlan>::new();
    for (body_id, body) in std::mem::take(&mut liquids.bodies) {
        let Some(root) = roots.get(&body_id).copied() else {
            return Err(WorldCompositionError::LiquidSeam {
                edge: first_edge,
                issue: LiquidSeamIssue::MissingMergedBody(body_id),
            });
        };
        if let Some(destination) = merged.get_mut(&root) {
            debug_assert_eq!(
                destination.material, body.material,
                "directly linked liquid materials were validated before union"
            );
            for (position, node) in body.nodes {
                let replaced = destination.nodes.insert(position, node);
                debug_assert!(
                    replaced.is_none(),
                    "validated disjoint patch masks cannot contribute duplicate liquid nodes"
                );
            }
        } else {
            merged.insert(root, body);
        }
    }
    liquids.bodies = merged;

    for link in links {
        let Some(body_id) = roots.get(&link.source.body).copied() else {
            return Err(WorldCompositionError::LiquidSeam {
                edge: link.edge,
                issue: LiquidSeamIssue::MissingMergedBody(link.source.body),
            });
        };
        let Some(body) = liquids.bodies.get_mut(&body_id) else {
            return Err(WorldCompositionError::LiquidSeam {
                edge: link.edge,
                issue: LiquidSeamIssue::MissingMergedBody(body_id),
            });
        };
        let Some(source) = body.nodes.get_mut(&link.source.position) else {
            return Err(WorldCompositionError::LiquidSeam {
                edge: link.edge,
                issue: LiquidSeamIssue::MissingMergedNode(link.source.position),
            });
        };
        source.state = LiquidFlowState::Current;
        source.downstream = Some(link.sink.position);
    }
    Ok(())
}

fn unique_liquid_endpoint(
    edge: ResolvedEdgeId,
    patch: PatchId,
    coord: HexCoord,
    min_level: i32,
    max_level: i32,
    liquids: &LiquidPlan,
) -> Result<LiquidEndpoint, WorldCompositionError> {
    let mut candidates = Vec::new();
    for (body_id, body) in &liquids.bodies {
        candidates.extend(body.nodes.iter().filter_map(|(position, node)| {
            (position.coord == coord && min_level <= position.level && position.level <= max_level)
                .then_some(LiquidEndpoint {
                    body: *body_id,
                    position: *position,
                    node: *node,
                    material: body.material,
                })
        }));
    }
    match candidates.as_slice() {
        [endpoint] => Ok(*endpoint),
        [] => Err(WorldCompositionError::LiquidSeam {
            edge,
            issue: LiquidSeamIssue::MissingEndpoint { patch, coord },
        }),
        _ => Err(WorldCompositionError::LiquidSeam {
            edge,
            issue: LiquidSeamIssue::AmbiguousEndpoint {
                patch,
                coord,
                positions: candidates
                    .into_iter()
                    .map(|endpoint| endpoint.position)
                    .collect(),
            },
        }),
    }
}

fn validate_liquid_link(
    edge: ResolvedEdgeId,
    source: LiquidEndpoint,
    sink: LiquidEndpoint,
) -> Result<(), WorldCompositionError> {
    let fail = |issue| WorldCompositionError::LiquidSeam { edge, issue };
    if source.material != sink.material {
        return Err(fail(LiquidSeamIssue::MaterialMismatch {
            source: source.material,
            sink: sink.material,
        }));
    }
    if let Some(downstream) = source.node.downstream {
        return Err(fail(LiquidSeamIssue::SourceAlreadyFlows {
            position: source.position,
            downstream,
        }));
    }
    if source.node.state != LiquidFlowState::Still {
        return Err(fail(LiquidSeamIssue::SourceIsNotStill {
            position: source.position,
            state: source.node.state,
        }));
    }
    if sink.position.level > source.position.level {
        return Err(fail(LiquidSeamIssue::Uphill {
            source: source.position,
            sink: sink.position,
        }));
    }
    if source.position.level.saturating_sub(sink.position.level) > 1 {
        return Err(fail(LiquidSeamIssue::ExcessiveDrop {
            source: source.position,
            sink: sink.position,
        }));
    }
    Ok(())
}

fn union_liquid_bodies(
    parents: &mut BTreeMap<LiquidBodyId, LiquidBodyId>,
    first: LiquidBodyId,
    second: LiquidBodyId,
) {
    let first = liquid_body_root(parents, first);
    let second = liquid_body_root(parents, second);
    if first == second {
        return;
    }
    let (root, child) = if first < second {
        (first, second)
    } else {
        (second, first)
    };
    parents.insert(child, root);
}

fn liquid_body_root(
    parents: &BTreeMap<LiquidBodyId, LiquidBodyId>,
    body: LiquidBodyId,
) -> LiquidBodyId {
    let mut current = body;
    while let Some(parent) = parents.get(&current).copied() {
        if parent == current {
            break;
        }
        current = parent;
    }
    current
}

/// Admits one patch through the same checked composer without changing Single names.
pub(crate) fn compose_single_patch(
    layout: ResolvedLayoutPlan,
    fragment: GeneratedPatchPlan,
) -> Result<GeneratedWorldPlan, WorldCompositionError> {
    if layout.kind != LayoutKind::Single {
        return Err(WorldCompositionError::InvalidLayout(vec![
            "single-patch composition requires a Single resolved layout".to_owned(),
        ]));
    }
    let view_hint = fragment.view_hint;
    compose_world(
        layout,
        vec![fragment],
        WorldCompositionSettings {
            canonical_anchors: BTreeMap::new(),
            view_hint,
        },
    )
}

fn merge_feature_memberships(
    destination: &mut FeaturePlan,
    routes: BTreeMap<String, super::world::ProtectedFeatureRoute>,
    clearings: BTreeMap<String, super::world::FeatureClearing>,
) -> Result<(), WorldCompositionError> {
    for (name, route) in routes {
        if destination.clearings.contains_key(&name) {
            return collision(CollisionKind::FeatureMembership, &name);
        }
        insert_unique(
            &mut destination.protected_routes,
            name,
            route,
            CollisionKind::FeatureMembership,
        )?;
    }
    for (name, clearing) in clearings {
        if destination.protected_routes.contains_key(&name) {
            return collision(CollisionKind::FeatureMembership, &name);
        }
        insert_unique(
            &mut destination.clearings,
            name,
            clearing,
            CollisionKind::FeatureMembership,
        )?;
    }
    Ok(())
}

fn merge_map<K, V>(
    destination: &mut BTreeMap<K, V>,
    source: BTreeMap<K, V>,
    kind: CollisionKind,
) -> Result<(), WorldCompositionError>
where
    K: Ord + std::fmt::Debug,
{
    for (key, value) in source {
        insert_unique(destination, key, value, kind)?;
    }
    Ok(())
}

fn insert_unique<K, V>(
    destination: &mut BTreeMap<K, V>,
    key: K,
    value: V,
    kind: CollisionKind,
) -> Result<(), WorldCompositionError>
where
    K: Ord + std::fmt::Debug,
{
    if destination.contains_key(&key) {
        return collision(kind, &key);
    }
    destination.insert(key, value);
    Ok(())
}

fn merge_set<T>(
    destination: &mut BTreeSet<T>,
    source: BTreeSet<T>,
    kind: CollisionKind,
) -> Result<(), WorldCompositionError>
where
    T: Ord + std::fmt::Debug,
{
    for value in source {
        if destination.contains(&value) {
            return collision(kind, &value);
        }
        destination.insert(value);
    }
    Ok(())
}

fn collision(kind: CollisionKind, key: &impl std::fmt::Debug) -> Result<(), WorldCompositionError> {
    Err(WorldCompositionError::Collision {
        kind,
        key: format!("{key:?}"),
    })
}

#[cfg(test)]
mod tests {
    use hex_core::{BiomeRegionId, HexCoord, IlluminationLevel};

    use super::*;
    use crate::procedural_v3::layout::resolve_layout;
    use crate::procedural_v3::liquid::{LiquidBodyPlan, LiquidFlowState, LiquidNode};
    use crate::procedural_v3::patch::PatchRecipeContext;
    use crate::procedural_v3::seam::shape_walker_seams;
    use crate::procedural_v3::volume::{
        FillMaterialRole, LevelInterval, NonSolidFill, SolidMass, SolidMaterialRole,
        SurfaceMetadata, VolumeColumn,
    };
    use crate::procedural_v3::world::{
        FeatureClearing, FeatureKind, PlannedFeature, PlannedInterior, PlannedStructure,
        ProtectedFeatureRoute, StructureKind,
    };
    use crate::settings::{
        EdgeElevationSettings, EdgeLiquidPortSettings, EdgeLiquidSettings,
        PatchEdgeContractSettings, PatchEdgesSettings, PatchMaskSettings, PatchSpec,
        ProceduralV3Settings, SharedEdgeSettings, V3CavesSettings, V3EnvironmentSettings,
        V3ForestSettings, V3FortSettings, V3HillsSettings, V3LayoutSettings, V3MountainsSettings,
        V3RecipeSettings, V3Ring7Settings, V3SkyIslandsSettings, V3WaterfallSettings,
        WalkerPortSettings,
    };

    const TEST_VIEW: MapViewHint = MapViewHint::new((0.0, 40.0, 40.0), (0.0, 5.0, 0.0));

    #[test]
    fn patch_validation_requires_exact_resolved_mask_and_biome_membership() {
        let layout = ring7_layout();
        let mut wrong_mask = complete_patch(&layout, PatchId(0));
        let removed = *wrong_mask
            .volume
            .mask
            .iter()
            .next()
            .expect("the generated patch is not empty");
        wrong_mask.volume.mask.remove(&removed);
        let issues = wrong_mask.validate_against(&layout);
        assert!(issues
            .iter()
            .any(|issue| issue.code == PatchIssueCode::Mask));

        let mut wrong_biome = complete_patch(&layout, PatchId(0));
        let position = *wrong_biome
            .biome_regions
            .keys()
            .next()
            .expect("the generated patch has a surface");
        wrong_biome
            .biome_regions
            .insert(position, BiomeRegionId(99));
        let issues = wrong_biome.validate_against(&layout);
        assert!(issues
            .iter()
            .any(|issue| { issue.code == PatchIssueCode::Semantic(WorldIssueCode::Biome) }));
    }

    #[test]
    fn composition_requires_exactly_one_fragment_for_every_resolved_patch() {
        let layout = ring7_layout();
        let mut fragments = complete_fragments(&layout);
        let missing = fragments.pop().expect("the fixture has seven fragments");
        assert_eq!(
            compose_world(layout.clone(), fragments, composition_settings())
                .expect_err("the missing fragment must fail"),
            WorldCompositionError::MissingFragment(missing.patch_id)
        );

        let mut fragments = complete_fragments(&layout);
        let duplicate = fragments
            .first()
            .expect("the fixture has a center fragment")
            .clone();
        fragments.push(duplicate);
        assert_eq!(
            compose_world(layout.clone(), fragments, composition_settings())
                .expect_err("the duplicate fragment must fail"),
            WorldCompositionError::DuplicateFragment(PatchId(0))
        );

        let mut fragments = complete_fragments(&layout);
        let mut unexpected = fragments
            .first()
            .expect("the fixture has a center fragment")
            .clone();
        unexpected.patch_id = PatchId(9);
        *fragments
            .first_mut()
            .expect("the fixture has a center fragment") = unexpected;
        assert_eq!(
            compose_world(layout, fragments, composition_settings())
                .expect_err("the unexpected fragment must fail"),
            WorldCompositionError::UnexpectedFragment(PatchId(9))
        );
    }

    #[test]
    fn composition_namespaces_every_identity_and_local_name_deterministically() {
        let layout = ring7_layout();
        let world = compose_world(
            layout.clone(),
            complete_fragments(&layout),
            composition_settings(),
        )
        .expect("complete dry Ring7 fragments should compose");

        assert_eq!(world.liquids.bodies.len(), 7);
        assert_eq!(world.features.by_id.len(), 7);
        assert_eq!(world.structures.by_id.len(), 7);
        assert_eq!(world.lights.len(), 7);
        assert_eq!(world.interiors.by_id.len(), 7);
        assert_eq!(world.blockers.len(), 7);
        assert_eq!(world.volume.columns.len(), layout.footprint.len());
        assert_eq!(world.biome_regions.len(), world.volume.surfaces.len());

        for patch in layout.patches.keys().copied() {
            let prefix = patch.0 << LOCAL_ID_BITS;
            assert!(world.liquids.bodies.contains_key(&LiquidBodyId(prefix)));
            assert!(world.features.by_id.contains_key(&FeatureId(prefix)));
            assert!(world.structures.by_id.contains_key(&StructureId(prefix)));
            assert!(world.lights.contains_key(&LightId(prefix)));
            assert!(world
                .interiors
                .by_id
                .contains_key(&InteriorRegionId(prefix | 3)));
            assert!(world
                .features
                .protected_routes
                .contains_key(&namespace_name(patch, "main_route")));
            assert!(world
                .features
                .clearings
                .contains_key(&namespace_name(patch, "main_clearing")));
            assert!(world
                .anchors
                .contains_key(&namespace_name(patch, "party_start")));
        }
        assert_eq!(
            world.anchors.get("party_start"),
            world.anchors.get("center_party_start")
        );
        assert_eq!(world.validate(), Vec::new());
    }

    #[test]
    fn interior_and_special_region_remaps_update_every_transitive_reference() {
        let layout = ring7_layout();
        let world = compose_world(
            layout.clone(),
            complete_fragments(&layout),
            composition_settings(),
        )
        .expect("complete dry Ring7 fragments should compose");
        let patch = PatchId(5);
        let interior = InteriorRegionId((patch.0 << LOCAL_ID_BITS) | 3);
        let special = SpecialMovementRegion((patch.0 << LOCAL_ID_BITS) | 2);
        let plan = world
            .interiors
            .by_id
            .get(&interior)
            .expect("the cave-like fixture interior should be namespaced");

        for floor in &plan.floors {
            assert_eq!(
                world
                    .volume
                    .surfaces
                    .get(floor)
                    .and_then(|metadata| metadata.interior),
                Some(interior)
            );
        }
        for roof in &plan.roof_voxels {
            assert!(world
                .volume
                .columns
                .get(&roof.coord)
                .expect("the roof column exists")
                .elements
                .iter()
                .any(|element| matches!(
                    element,
                    VolumeElement::Solid(mass)
                        if mass.cutaway_for == Some(interior)
                            && mass.levels.bottom <= roof.level
                            && roof.level < mass.levels.top
                )));
        }
        assert!(world
            .volume
            .surfaces
            .values()
            .any(|metadata| { metadata.access == SurfaceAccess::SpecialMovement(special) }));
    }

    #[test]
    fn canonical_aliases_are_explicit_validated_and_collision_checked() {
        let layout = ring7_layout();
        let fragments = complete_fragments(&layout);
        let mut invalid = composition_settings();
        invalid.canonical_anchors =
            BTreeMap::from([("Party Start".to_owned(), center_anchor_ref())]);
        assert_eq!(
            compose_world(layout.clone(), fragments.clone(), invalid)
                .expect_err("the invalid alias must fail"),
            WorldCompositionError::InvalidCanonicalAnchorName("Party Start".to_owned())
        );

        let mut missing = composition_settings();
        missing.canonical_anchors = BTreeMap::from([(
            "party_start".to_owned(),
            PatchAnchorRef {
                patch: PatchId(0),
                local_name: "missing".to_owned(),
            },
        )]);
        assert_eq!(
            compose_world(layout.clone(), fragments.clone(), missing)
                .expect_err("the missing local anchor must fail"),
            WorldCompositionError::MissingCanonicalAnchor {
                alias: "party_start".to_owned(),
                patch: PatchId(0),
                local_name: "missing".to_owned(),
            }
        );

        let mut collision = composition_settings();
        collision.canonical_anchors =
            BTreeMap::from([("center_party_start".to_owned(), center_anchor_ref())]);
        assert!(matches!(
            compose_world(layout, fragments, collision),
            Err(WorldCompositionError::Collision {
                kind: CollisionKind::Anchor,
                ..
            })
        ));
    }

    #[test]
    fn every_checked_merge_family_reports_duplicate_keys() {
        for kind in [
            CollisionKind::Column,
            CollisionKind::Surface,
            CollisionKind::Liquid,
            CollisionKind::Feature,
            CollisionKind::FeatureMembership,
            CollisionKind::Structure,
            CollisionKind::Light,
            CollisionKind::Biome,
            CollisionKind::Interior,
            CollisionKind::Anchor,
        ] {
            let mut destination = BTreeMap::from([(1_u32, 1_u32)]);
            let error = insert_unique(&mut destination, 1, 2, kind)
                .expect_err("duplicate keys must never overwrite prior semantic data");
            assert!(matches!(
                error,
                WorldCompositionError::Collision {
                    kind: actual,
                    ..
                } if actual == kind
            ));
        }

        let error = merge_set(
            &mut BTreeSet::from([TilePos::new(HexCoord::ORIGIN, 0)]),
            BTreeSet::from([TilePos::new(HexCoord::ORIGIN, 0)]),
            CollisionKind::Blocker,
        )
        .expect_err("duplicate blocker positions must be rejected");
        assert!(matches!(
            error,
            WorldCompositionError::Collision {
                kind: CollisionKind::Blocker,
                ..
            }
        ));
    }

    #[test]
    fn directed_liquid_seams_merge_bodies_and_realize_every_lane() {
        let layout = directed_ring7_layout();
        let (edge_id, edge) = layout
            .shared_edges
            .iter()
            .find_map(|(id, edge)| {
                matches!(edge.liquid, ResolvedLiquidPort::Directed { .. }).then_some((*id, edge))
            })
            .expect("the fixture has one directed seam");
        let world = compose_world(
            layout.clone(),
            directed_fragments(&layout),
            composition_settings(),
        )
        .expect("exact directed endpoints should compose");
        let ResolvedLiquidPort::Directed { source, port, .. } = &edge.liquid else {
            panic!("the selected seam should be directed");
        };
        let source_is_first = *source == edge.first.0;

        for (first, second) in &port.lanes {
            let (source_coord, sink_coord) = if source_is_first {
                (*first, *second)
            } else {
                (*second, *first)
            };
            let crossing = world
                .liquids
                .bodies
                .values()
                .find_map(|body| {
                    body.nodes.iter().find_map(|(position, node)| {
                        (position.coord == source_coord
                            && node
                                .downstream
                                .is_some_and(|target| target.coord == sink_coord))
                        .then_some((*position, *node, body))
                    })
                })
                .expect("every declared lane should have one directed successor");
            assert_eq!(crossing.1.state, LiquidFlowState::Current);
            assert!(crossing
                .2
                .nodes
                .contains_key(&crossing.1.downstream.expect("crossing has a target")));
        }
        assert_eq!(world.validate(), Vec::new(), "edge {edge_id:?}");
    }

    #[test]
    fn directed_liquid_seams_reject_missing_and_mismatched_endpoints() {
        let layout = directed_ring7_layout();
        let (edge_id, edge) = layout
            .shared_edges
            .iter()
            .find_map(|(id, edge)| {
                matches!(edge.liquid, ResolvedLiquidPort::Directed { .. }).then_some((*id, edge))
            })
            .expect("the fixture has one directed seam");
        let ResolvedLiquidPort::Directed { source, sink, port } = &edge.liquid else {
            panic!("the selected seam should be directed");
        };
        let first_lane = *port.lanes.first().expect("the directed port has lanes");
        let source_coord = if *source == edge.first.0 {
            first_lane.0
        } else {
            first_lane.1
        };
        assert!(matches!(
            compose_world(
                layout.clone(),
                complete_fragments(&layout),
                composition_settings()
            ),
            Err(WorldCompositionError::LiquidSeam {
                edge,
                issue: LiquidSeamIssue::MissingEndpoint { patch, coord },
            }) if edge == edge_id && patch == *source && coord == source_coord
        ));

        let mut fragments = directed_fragments(&layout);
        let sink_fragment = fragments
            .iter_mut()
            .find(|fragment| fragment.patch_id == *sink)
            .expect("the sink fragment exists");
        for body in sink_fragment.liquids.bodies.values_mut() {
            body.material = FillMaterialRole::Lava;
        }
        for column in sink_fragment.volume.columns.values_mut() {
            for element in &mut column.elements {
                if let VolumeElement::Fill(fill) = element {
                    fill.material = FillMaterialRole::Lava;
                }
            }
        }
        assert!(matches!(
            compose_world(layout, fragments, composition_settings()),
            Err(WorldCompositionError::LiquidSeam {
                edge,
                issue: LiquidSeamIssue::MaterialMismatch {
                    source: FillMaterialRole::Water,
                    sink: FillMaterialRole::Lava,
                },
            }) if edge == edge_id
        ));
    }

    #[test]
    fn namespace_bounds_reject_aliasing_instead_of_truncating_ids() {
        assert_eq!(
            namespace_numeric(PatchId(MAX_PATCH_ID + 1), 0, NamespaceKind::Feature),
            Err(WorldCompositionError::NamespaceOverflow {
                patch: PatchId(MAX_PATCH_ID + 1),
                kind: NamespaceKind::Feature,
                local: 0,
            })
        );
        assert_eq!(
            namespace_numeric(PatchId(0), MAX_LOCAL_ID + 1, NamespaceKind::Structure),
            Err(WorldCompositionError::NamespaceOverflow {
                patch: PatchId(0),
                kind: NamespaceKind::Structure,
                local: MAX_LOCAL_ID + 1,
            })
        );
    }

    #[test]
    fn final_whole_world_validation_remains_authoritative() {
        let layout = ring7_layout();
        let settings = WorldCompositionSettings {
            canonical_anchors: BTreeMap::from([("party_start".to_owned(), center_anchor_ref())]),
            view_hint: MapViewHint::new((f32::NAN, 0.0, 0.0), (0.0, 0.0, 0.0)),
        };
        let error = compose_world(layout.clone(), complete_fragments(&layout), settings)
            .expect_err("an invalid whole-world view must fail final validation");
        assert!(matches!(
            error,
            WorldCompositionError::FinalValidation(issues)
                if issues.iter().any(|issue| issue.code == WorldIssueCode::View)
        ));
    }

    fn composition_settings() -> WorldCompositionSettings {
        WorldCompositionSettings {
            canonical_anchors: BTreeMap::from([("party_start".to_owned(), center_anchor_ref())]),
            view_hint: TEST_VIEW,
        }
    }

    fn center_anchor_ref() -> PatchAnchorRef {
        PatchAnchorRef {
            patch: PatchId(0),
            local_name: "party_start".to_owned(),
        }
    }

    fn complete_fragments(layout: &ResolvedLayoutPlan) -> Vec<GeneratedPatchPlan> {
        layout
            .patches
            .keys()
            .copied()
            .map(|patch| complete_patch(layout, patch))
            .collect()
    }

    fn directed_fragments(layout: &ResolvedLayoutPlan) -> Vec<GeneratedPatchPlan> {
        let mut fragments = complete_fragments(layout);
        for edge in layout.shared_edges.values() {
            let ResolvedLiquidPort::Directed { source, sink, port } = &edge.liquid else {
                continue;
            };
            let source_is_first = *source == edge.first.0;
            let source_coords =
                port.lanes
                    .iter()
                    .map(|lane| if source_is_first { lane.0 } else { lane.1 });
            let sink_coords = port
                .lanes
                .iter()
                .map(|lane| if source_is_first { lane.1 } else { lane.0 });
            replace_fragment_liquid(
                fragments
                    .iter_mut()
                    .find(|fragment| fragment.patch_id == *source)
                    .expect("the source fragment exists"),
                source_coords,
                edge.elevation.preferred,
            );
            replace_fragment_liquid(
                fragments
                    .iter_mut()
                    .find(|fragment| fragment.patch_id == *sink)
                    .expect("the sink fragment exists"),
                sink_coords,
                edge.elevation.preferred,
            );
        }
        fragments
    }

    fn replace_fragment_liquid(
        fragment: &mut GeneratedPatchPlan,
        coords: impl IntoIterator<Item = HexCoord>,
        level: i32,
    ) {
        for column in fragment.volume.columns.values_mut() {
            column
                .elements
                .retain(|element| !matches!(element, VolumeElement::Fill(_)));
        }
        fragment.liquids = LiquidPlan::default();
        for (index, coord) in coords.into_iter().enumerate() {
            let column = fragment
                .volume
                .columns
                .get_mut(&coord)
                .expect("the endpoint column belongs to the patch");
            column.elements = vec![
                VolumeElement::Solid(SolidMass {
                    levels: LevelInterval::new(0, level),
                    material: SolidMaterialRole::Stone,
                    cutaway_for: None,
                }),
                VolumeElement::Fill(NonSolidFill {
                    levels: LevelInterval::new(level, level + 1),
                    material: FillMaterialRole::Water,
                }),
            ];
            let biome = fragment
                .biome_regions
                .iter()
                .find_map(|(position, biome)| (position.coord == coord).then_some(*biome))
                .expect("the endpoint column has biome membership");
            fragment
                .volume
                .surfaces
                .retain(|position, _| position.coord != coord);
            fragment
                .biome_regions
                .retain(|position, _| position.coord != coord);
            let bed = TilePos::new(coord, level.saturating_sub(1));
            fragment.volume.surfaces.insert(
                bed,
                SurfaceMetadata {
                    access: SurfaceAccess::NonStandable,
                    interior: None,
                },
            );
            fragment.biome_regions.insert(bed, biome);
            let top = TilePos::new(coord, level);
            fragment.liquids.bodies.insert(
                LiquidBodyId(u32::try_from(index).expect("the test port is small")),
                LiquidBodyPlan {
                    material: FillMaterialRole::Water,
                    nodes: BTreeMap::from([(
                        top,
                        LiquidNode {
                            state: LiquidFlowState::Still,
                            downstream: None,
                        },
                    )]),
                },
            );
        }
    }

    fn complete_patch(layout: &ResolvedLayoutPlan, patch_id: PatchId) -> GeneratedPatchPlan {
        let patch = layout
            .patches
            .get(&patch_id)
            .expect("the fixture patch exists");
        let context =
            PatchRecipeContext::resolve(layout, patch_id).expect("the fixture patch resolves");
        let protected = context.protected_approaches();
        let boundary: BTreeSet<_> = context
            .shared_edges()
            .flat_map(|edge| edge.boundary_pairs().into_iter().map(|(local, _)| local))
            .collect();
        let mut interior_cells = patch
            .mask
            .iter()
            .copied()
            .filter(|coord| {
                coord
                    .neighbors()
                    .iter()
                    .all(|next| patch.mask.contains(next))
                    && !protected.contains(coord)
                    && !boundary.contains(coord)
            })
            .take(7);
        let anchor_coord = interior_cells.next().expect("patch has an interior anchor");
        let tree_coord = interior_cells.next().expect("patch has an interior tree");
        let route_coord = interior_cells.next().expect("patch has an interior route");
        let clearing_coord = interior_cells
            .next()
            .expect("patch has an interior clearing");
        let liquid_coord = interior_cells.next().expect("patch has an interior liquid");
        let cave_coord = interior_cells.next().expect("patch has an interior cave");
        let structure_coord = interior_cells
            .next()
            .expect("patch has an interior structure");

        let mut volume = VolumePlan::new(patch.mask.clone());
        for column in volume.columns.values_mut() {
            column.elements.push(VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(0, 16),
                material: SolidMaterialRole::Stone,
                cutaway_for: None,
            }));
        }
        for coord in &patch.mask {
            volume.surfaces.insert(
                TilePos::new(*coord, 15),
                SurfaceMetadata {
                    access: SurfaceAccess::Ordinary,
                    interior: None,
                },
            );
        }

        let liquid_top = TilePos::new(liquid_coord, 19);
        volume
            .columns
            .get_mut(&liquid_coord)
            .expect("the liquid column exists")
            .elements
            .push(VolumeElement::Fill(NonSolidFill {
                levels: LevelInterval::new(19, 20),
                material: FillMaterialRole::Water,
            }));

        let interior = InteriorRegionId(3);
        let cave_floor = TilePos::new(cave_coord, 0);
        let cave_roof = TilePos::new(cave_coord, 15);
        *volume
            .columns
            .get_mut(&cave_coord)
            .expect("the cave column exists") = VolumeColumn {
            elements: vec![
                VolumeElement::Solid(SolidMass {
                    levels: LevelInterval::new(0, 1),
                    material: SolidMaterialRole::Stone,
                    cutaway_for: None,
                }),
                VolumeElement::Solid(SolidMass {
                    levels: LevelInterval::new(4, 16),
                    material: SolidMaterialRole::Stone,
                    cutaway_for: Some(interior),
                }),
            ],
        };
        volume.surfaces.insert(
            cave_floor,
            SurfaceMetadata {
                access: SurfaceAccess::Ordinary,
                interior: Some(interior),
            },
        );
        volume.surfaces.insert(
            cave_roof,
            SurfaceMetadata {
                access: SurfaceAccess::SpecialMovement(SpecialMovementRegion(2)),
                interior: None,
            },
        );

        let mut levels = patch
            .mask
            .iter()
            .copied()
            .map(|coord| (coord, 15))
            .collect();
        let shape = shape_walker_seams(&context, &mut levels).expect("fixture seams should shape");
        shape
            .apply(&mut volume)
            .expect("fixture seams should project");

        let anchor = TilePos::new(anchor_coord, 15);
        let tree = TilePos::new(tree_coord, 15);
        let route = TilePos::new(route_coord, 15);
        let clearing = TilePos::new(clearing_coord, 15);
        let structure_voxel = TilePos::new(structure_coord, 15);
        let biome_regions = volume
            .surfaces
            .keys()
            .copied()
            .map(|position| (position, patch.biome_region))
            .collect();

        GeneratedPatchPlan {
            patch_id,
            volume,
            liquids: LiquidPlan {
                bodies: BTreeMap::from([(
                    LiquidBodyId(0),
                    LiquidBodyPlan {
                        material: FillMaterialRole::Water,
                        nodes: BTreeMap::from([(
                            liquid_top,
                            LiquidNode {
                                state: LiquidFlowState::Still,
                                downstream: None,
                            },
                        )]),
                    },
                )]),
            },
            features: FeaturePlan {
                by_id: BTreeMap::from([(
                    FeatureId(0),
                    PlannedFeature {
                        root: tree,
                        kind: FeatureKind::Tree,
                        object_id: hex_assets::ObjectAssetId::new("plant/small-broadleaf")
                            .expect("fixture object id should be valid"),
                        rotation: hex_assets::HexObjectRotation::ZERO,
                        blocker_footprint: BTreeSet::from([tree]),
                    },
                )]),
                protected_routes: BTreeMap::from([(
                    "main_route".to_owned(),
                    ProtectedFeatureRoute {
                        centerline: vec![route],
                        surfaces: BTreeSet::from([route]),
                    },
                )]),
                clearings: BTreeMap::from([(
                    "main_clearing".to_owned(),
                    FeatureClearing {
                        surfaces: BTreeSet::from([clearing]),
                    },
                )]),
            },
            structures: StructurePlan {
                by_id: BTreeMap::from([(
                    StructureId(0),
                    PlannedStructure {
                        kind: StructureKind::Bridge,
                        voxels: BTreeSet::from([structure_voxel]),
                    },
                )]),
            },
            blockers: BTreeSet::from([tree]),
            lights: BTreeMap::from([(
                LightId(0),
                PlannedGameplayLight {
                    origin: anchor,
                    level: IlluminationLevel::Bright,
                    radius: 4,
                    presentation: None,
                },
            )]),
            biome_regions,
            interiors: InteriorPlan {
                by_id: BTreeMap::from([(
                    interior,
                    PlannedInterior {
                        floors: BTreeSet::from([cave_floor]),
                        entrances: BTreeSet::from([cave_floor]),
                        roof_voxels: (4..16)
                            .map(|level| TilePos::new(cave_coord, level))
                            .collect(),
                    },
                )]),
            },
            anchors: BTreeMap::from([("party_start".to_owned(), anchor)]),
            view_hint: TEST_VIEW,
        }
    }

    fn ring7_layout() -> ResolvedLayoutPlan {
        let settings = ProceduralV3Settings {
            layout: V3LayoutSettings::Ring7(valid_ring7_settings()),
        };
        resolve_layout(33, &settings).expect("the fixed dry Ring7 settings should resolve")
    }

    fn directed_ring7_layout() -> ResolvedLayoutPlan {
        let mut ring = valid_ring7_settings();
        ring.center.edges.east = shared_edge(EdgeLiquidSettings::Outlet(EdgeLiquidPortSettings {
            width: 2,
        }));
        ring.waterfall.edges.west =
            shared_edge(EdgeLiquidSettings::Inlet(EdgeLiquidPortSettings {
                width: 2,
            }));
        let settings = ProceduralV3Settings {
            layout: V3LayoutSettings::Ring7(ring),
        };
        resolve_layout(33, &settings).expect("the directed Ring7 settings should resolve")
    }

    fn valid_ring7_settings() -> V3Ring7Settings {
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

        let shared = dry_shared_edge();
        ring.center.edges.north_east = shared.clone();
        ring.mountains.edges.south_west = shared.clone();
        ring.center.edges.east = shared.clone();
        ring.waterfall.edges.west = shared.clone();
        ring.center.edges.south_east = shared.clone();
        ring.forest.edges.north_west = shared.clone();
        ring.center.edges.south_west = shared.clone();
        ring.fort.edges.north_east = shared.clone();
        ring.center.edges.west = shared.clone();
        ring.caves.edges.east = shared.clone();
        ring.center.edges.north_west = shared.clone();
        ring.sky_islands.edges.south_east = shared.clone();

        ring.mountains.edges.south_east = shared.clone();
        ring.waterfall.edges.north_west = shared.clone();
        ring.waterfall.edges.south_west = shared.clone();
        ring.forest.edges.north_east = shared.clone();
        ring.forest.edges.west = shared.clone();
        ring.fort.edges.east = shared.clone();
        ring.fort.edges.north_west = shared.clone();
        ring.caves.edges.south_east = shared.clone();
        ring.caves.edges.north_east = shared.clone();
        ring.sky_islands.edges.south_west = shared.clone();
        ring.sky_islands.edges.east = shared.clone();
        ring.mountains.edges.west = shared;
        ring
    }

    fn generated_patch(environment: V3EnvironmentSettings, recipe: V3RecipeSettings) -> PatchSpec {
        PatchSpec {
            environment,
            recipe,
            overlays: Vec::new(),
            mask: PatchMaskSettings::GeneratedRegion,
            edges: world_boundary_edges(),
        }
    }

    fn world_boundary_edges() -> PatchEdgesSettings {
        PatchEdgesSettings {
            east: PatchEdgeContractSettings::WorldBoundary,
            south_east: PatchEdgeContractSettings::WorldBoundary,
            south_west: PatchEdgeContractSettings::WorldBoundary,
            west: PatchEdgeContractSettings::WorldBoundary,
            north_west: PatchEdgeContractSettings::WorldBoundary,
            north_east: PatchEdgeContractSettings::WorldBoundary,
        }
    }

    fn dry_shared_edge() -> PatchEdgeContractSettings {
        shared_edge(EdgeLiquidSettings::Dry)
    }

    fn shared_edge(liquid: EdgeLiquidSettings) -> PatchEdgeContractSettings {
        PatchEdgeContractSettings::Shared(SharedEdgeSettings {
            elevation: EdgeElevationSettings {
                preferred: 15,
                min: 14,
                max: 16,
            },
            walker: WalkerPortSettings { count: 1, width: 2 },
            liquid,
            approach_depth: 2,
        })
    }
}
