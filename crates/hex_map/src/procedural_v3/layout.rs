//! Resolved V3 patch masks and shared edge contracts.
//!
//! Settings describe each side from a designer's perspective. Resolution assigns
//! stable patch and edge identities, creates exact disjoint masks, and stores each
//! internal seam once so neighboring recipes cannot generate competing borders.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

use hex_core::{BiomeRegionId, HexCoord, Level};

use crate::settings::{
    EdgeLiquidSettings, PatchEdgeContractSettings, PatchEdgesSettings, PatchMaskSettings,
    PatchSpec, ProceduralV3Settings, SharedEdgeSettings, V3LayoutSettings, V3Ring7Settings,
};

const RING_RADIUS: u32 = 33;
const RING_PATCH_OFFSET: i32 = 22;

/// Stable complete-world layout family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LayoutKind {
    Single,
    Ring7,
}

/// Stable semantic patch slot. IDs also namespace V3 seed streams.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct PatchId(pub(crate) u32);

/// Stable identity of one resolved shared edge.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ResolvedEdgeId(pub(crate) u32);

/// Clockwise side names used by settings, layout, flow, and fingerprints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum HexSide {
    East,
    SouthEast,
    SouthWest,
    West,
    NorthWest,
    NorthEast,
}

impl HexSide {
    pub(crate) const ALL: [Self; 6] = [
        Self::East,
        Self::SouthEast,
        Self::SouthWest,
        Self::West,
        Self::NorthWest,
        Self::NorthEast,
    ];

    #[must_use]
    pub(crate) const fn opposite(self) -> Self {
        match self {
            Self::East => Self::West,
            Self::SouthEast => Self::NorthWest,
            Self::SouthWest => Self::NorthEast,
            Self::West => Self::East,
            Self::NorthWest => Self::SouthEast,
            Self::NorthEast => Self::SouthWest,
        }
    }

    #[must_use]
    pub(crate) const fn neighbor(self, coord: HexCoord) -> HexCoord {
        let [x, y, z] = coord.to_cubic_array();
        let [dx, dy, dz] = match self {
            Self::East => [1, 0, -1],
            Self::SouthEast => [0, 1, -1],
            Self::SouthWest => [-1, 1, 0],
            Self::West => [-1, 0, 1],
            Self::NorthWest => [0, -1, 1],
            Self::NorthEast => [1, -1, 0],
        };
        HexCoord::new_cubic(x + dx, y + dy, z + dz)
    }
}

/// One patch-side reference to either the outside world or a shared seam.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResolvedEdgeReference {
    WorldBoundary,
    Shared(ResolvedEdgeId),
}

/// Resolved vertical seam band.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResolvedElevationBand {
    pub(crate) preferred: Level,
    pub(crate) min: Level,
    pub(crate) max: Level,
}

/// One exact aperture and its protected approach cells on both sides of a seam.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ResolvedPort {
    /// Contiguous adjacent pairs, oriented from the edge's first patch to its second.
    pub(crate) lanes: BTreeSet<(HexCoord, HexCoord)>,
    pub(crate) first_approach: BTreeSet<HexCoord>,
    pub(crate) second_approach: BTreeSet<HexCoord>,
}

/// Exact ordinary route ports on one shared edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedWalkerPorts {
    pub(crate) ports: Vec<ResolvedPort>,
}

/// One directed hydrology relationship, stored once for both patches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ResolvedLiquidPort {
    Dry,
    Directed {
        source: PatchId,
        sink: PatchId,
        port: ResolvedPort,
    },
}

/// Exact resolved seam shared by two patches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedEdgeContract {
    pub(crate) first: (PatchId, HexSide),
    pub(crate) second: (PatchId, HexSide),
    pub(crate) elevation: ResolvedElevationBand,
    pub(crate) walker: ResolvedWalkerPorts,
    pub(crate) liquid: ResolvedLiquidPort,
    pub(crate) approach_depth: u32,
    /// Every pair of adjacent columns across the two masks, oriented first-to-second.
    pub(crate) boundary_pairs: BTreeSet<(HexCoord, HexCoord)>,
    /// Exact cells recipes must preserve while approaching the seam.
    pub(crate) protected_approaches: BTreeMap<PatchId, BTreeSet<HexCoord>>,
}

/// One resolved patch in the complete layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedPatch {
    pub(crate) biome_region: BiomeRegionId,
    pub(crate) mask: BTreeSet<HexCoord>,
    pub(crate) edges: BTreeMap<HexSide, ResolvedEdgeReference>,
}

/// Complete mask and seam topology consumed by candidate recipes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedLayoutPlan {
    pub(crate) kind: LayoutKind,
    pub(crate) grid_radius: u32,
    pub(crate) footprint: BTreeSet<HexCoord>,
    pub(crate) patches: BTreeMap<PatchId, ResolvedPatch>,
    pub(crate) shared_edges: BTreeMap<ResolvedEdgeId, ResolvedEdgeContract>,
}

impl ResolvedLayoutPlan {
    /// Rechecks exact coverage, connectivity, references, and shared seam geometry.
    pub(crate) fn validate(&self) -> Result<(), LayoutValidationError> {
        let mut issues = Vec::new();
        if !(12..=40).contains(&self.grid_radius) {
            issues.push(LayoutIssue::UnsupportedRadius(self.grid_radius));
        }
        if self.footprint.is_empty() || !connected(&self.footprint) {
            issues.push(LayoutIssue::InvalidFootprint);
        }
        if self
            .footprint
            .iter()
            .any(|coord| HexCoord::ORIGIN.distance(*coord) > self.grid_radius)
        {
            issues.push(LayoutIssue::FootprintOutOfBounds);
        }

        let expected_patch_count = match self.kind {
            LayoutKind::Single => 1,
            LayoutKind::Ring7 => 7,
        };
        if self.patches.len() != expected_patch_count {
            issues.push(LayoutIssue::PatchCount {
                expected: expected_patch_count,
                actual: self.patches.len(),
            });
        }
        if self.kind == LayoutKind::Ring7
            && (self.grid_radius != RING_RADIUS
                || self.footprint
                    != HexCoord::ORIGIN
                        .within_radius(RING_RADIUS)
                        .into_iter()
                        .collect())
        {
            issues.push(LayoutIssue::InvalidRingFootprint);
        }

        let mut covered = BTreeSet::new();
        let mut biome_regions = BTreeSet::new();
        for (id, patch) in &self.patches {
            if patch.mask.is_empty() || !connected(&patch.mask) {
                issues.push(LayoutIssue::DisconnectedPatch(*id));
            }
            if !patch.mask.is_subset(&self.footprint) {
                issues.push(LayoutIssue::PatchOutsideFootprint(*id));
            }
            for coord in &patch.mask {
                if !covered.insert(*coord) {
                    issues.push(LayoutIssue::OverlappingPatch(*id, *coord));
                }
            }
            if !biome_regions.insert(patch.biome_region) {
                issues.push(LayoutIssue::DuplicateBiomeRegion(patch.biome_region));
            }
            if patch.edges.len() != HexSide::ALL.len()
                || HexSide::ALL
                    .iter()
                    .any(|side| !patch.edges.contains_key(side))
            {
                issues.push(LayoutIssue::IncompletePatchEdges(*id));
            }
        }
        if covered != self.footprint {
            issues.push(LayoutIssue::IncompleteCoverage);
        }

        let mut references = BTreeMap::<ResolvedEdgeId, Vec<(PatchId, HexSide)>>::new();
        for (patch_id, patch) in &self.patches {
            for (side, reference) in &patch.edges {
                if let ResolvedEdgeReference::Shared(edge) = reference {
                    references
                        .entry(*edge)
                        .or_default()
                        .push((*patch_id, *side));
                }
            }
        }
        for (edge_id, edge) in &self.shared_edges {
            let mut expected = vec![edge.first, edge.second];
            expected.sort_unstable();
            let mut actual = references.remove(edge_id).unwrap_or_default();
            actual.sort_unstable();
            if actual != expected {
                issues.push(LayoutIssue::SharedReferenceMismatch(*edge_id));
            }
            validate_resolved_edge(*edge_id, edge, &self.patches, &mut issues);
        }
        for edge_id in references.into_keys() {
            issues.push(LayoutIssue::MissingSharedEdge(edge_id));
        }

        if issues.is_empty() {
            Ok(())
        } else {
            Err(LayoutValidationError { issues })
        }
    }
}

/// Resolves strict V3 settings into exact masks and shared edge objects.
pub(crate) fn resolve_layout(
    grid_radius: u32,
    settings: &ProceduralV3Settings,
) -> Result<ResolvedLayoutPlan, LayoutValidationError> {
    let resolved = match &settings.layout {
        V3LayoutSettings::Single(patch) => resolve_single(grid_radius, patch)?,
        V3LayoutSettings::Ring7(ring) => resolve_ring(grid_radius, ring)?,
    };
    resolved.validate()?;
    Ok(resolved)
}

fn resolve_single(
    grid_radius: u32,
    patch: &PatchSpec,
) -> Result<ResolvedLayoutPlan, LayoutValidationError> {
    let mask = match &patch.mask {
        PatchMaskSettings::WholeWorld => HexCoord::ORIGIN
            .within_radius(grid_radius)
            .into_iter()
            .collect(),
        PatchMaskSettings::Explicit(coords) => explicit_mask(coords, grid_radius)?,
        PatchMaskSettings::GeneratedRegion => {
            return Err(LayoutValidationError::one(LayoutIssue::GeneratedSingleMask));
        }
    };
    let mut edges = BTreeMap::new();
    for side in HexSide::ALL {
        if !matches!(
            edge_setting(&patch.edges, side),
            PatchEdgeContractSettings::WorldBoundary
        ) {
            return Err(LayoutValidationError::one(LayoutIssue::SharedSingleEdge(
                side,
            )));
        }
        edges.insert(side, ResolvedEdgeReference::WorldBoundary);
    }
    let id = PatchId(0);
    let patches = BTreeMap::from([(
        id,
        ResolvedPatch {
            biome_region: BiomeRegionId(0),
            mask: mask.clone(),
            edges,
        },
    )]);
    Ok(ResolvedLayoutPlan {
        kind: LayoutKind::Single,
        grid_radius,
        footprint: mask,
        patches,
        shared_edges: BTreeMap::new(),
    })
}

fn resolve_ring(
    grid_radius: u32,
    ring: &V3Ring7Settings,
) -> Result<ResolvedLayoutPlan, LayoutValidationError> {
    if grid_radius != RING_RADIUS {
        return Err(LayoutValidationError::one(LayoutIssue::InvalidRingRadius(
            grid_radius,
        )));
    }
    let specs = ring_specs(ring);
    let all_generated = specs
        .iter()
        .all(|(_, patch)| matches!(patch.mask, PatchMaskSettings::GeneratedRegion));
    let all_explicit = specs
        .iter()
        .all(|(_, patch)| matches!(patch.mask, PatchMaskSettings::Explicit(_)));
    if !all_generated && !all_explicit {
        return Err(LayoutValidationError::one(LayoutIssue::MixedRingMasks));
    }

    let footprint: BTreeSet<_> = HexCoord::ORIGIN
        .within_radius(RING_RADIUS)
        .into_iter()
        .collect();
    let masks = if all_generated {
        generated_ring_masks(&footprint)
    } else {
        let mut masks = BTreeMap::new();
        for (id, patch) in specs {
            let PatchMaskSettings::Explicit(coords) = &patch.mask else {
                unreachable!("the Ring7 mask mode was established as explicit");
            };
            masks.insert(id, explicit_mask(coords, grid_radius)?);
        }
        masks
    };

    let mut patches = BTreeMap::new();
    for (id, _) in specs {
        let edges = HexSide::ALL
            .into_iter()
            .map(|side| (side, ResolvedEdgeReference::WorldBoundary))
            .collect();
        patches.insert(
            id,
            ResolvedPatch {
                biome_region: BiomeRegionId(id.0),
                mask: masks.get(&id).cloned().unwrap_or_default(),
                edges,
            },
        );
    }

    let seams = ring_seams();
    let mut shared_edges = BTreeMap::new();
    for (index, (first_id, first_side, second_id, second_side)) in seams.into_iter().enumerate() {
        let edge_id = ResolvedEdgeId(u32::try_from(index).unwrap_or(u32::MAX));
        let first_settings = edge_setting(&spec_for(ring, first_id).edges, first_side);
        let second_settings = edge_setting(&spec_for(ring, second_id).edges, second_side);
        let contract = resolve_shared_edge(
            first_id,
            first_side,
            first_settings,
            second_id,
            second_side,
            second_settings,
            &masks,
        )?;
        shared_edges.insert(edge_id, contract);
        let Some(first_patch) = patches.get_mut(&first_id) else {
            return Err(LayoutValidationError::one(LayoutIssue::MissingEdgePatch(
                edge_id, first_id,
            )));
        };
        first_patch
            .edges
            .insert(first_side, ResolvedEdgeReference::Shared(edge_id));
        let Some(second_patch) = patches.get_mut(&second_id) else {
            return Err(LayoutValidationError::one(LayoutIssue::MissingEdgePatch(
                edge_id, second_id,
            )));
        };
        second_patch
            .edges
            .insert(second_side, ResolvedEdgeReference::Shared(edge_id));
    }

    for (id, patch) in specs {
        for side in HexSide::ALL {
            let resolved = patches
                .get(&id)
                .and_then(|resolved| resolved.edges.get(&side))
                .copied()
                .unwrap_or(ResolvedEdgeReference::WorldBoundary);
            let authored = edge_setting(&patch.edges, side);
            match (resolved, authored) {
                (
                    ResolvedEdgeReference::WorldBoundary,
                    PatchEdgeContractSettings::WorldBoundary,
                )
                | (ResolvedEdgeReference::Shared(_), PatchEdgeContractSettings::Shared(_)) => {}
                _ => {
                    return Err(LayoutValidationError::one(LayoutIssue::UnexpectedEdgeKind(
                        id, side,
                    )));
                }
            }
        }
    }

    Ok(ResolvedLayoutPlan {
        kind: LayoutKind::Ring7,
        grid_radius,
        footprint,
        patches,
        shared_edges,
    })
}

fn resolve_shared_edge(
    first_id: PatchId,
    first_side: HexSide,
    first: &PatchEdgeContractSettings,
    second_id: PatchId,
    second_side: HexSide,
    second: &PatchEdgeContractSettings,
    masks: &BTreeMap<PatchId, BTreeSet<HexCoord>>,
) -> Result<ResolvedEdgeContract, LayoutValidationError> {
    let (PatchEdgeContractSettings::Shared(first), PatchEdgeContractSettings::Shared(second)) =
        (first, second)
    else {
        return Err(LayoutValidationError::one(LayoutIssue::UnpairedSharedEdge(
            first_id,
            first_side,
            second_id,
            second_side,
        )));
    };
    if first.elevation != second.elevation
        || first.walker != second.walker
        || first.approach_depth != second.approach_depth
    {
        return Err(LayoutValidationError::one(
            LayoutIssue::MismatchedSharedEdge(first_id, second_id),
        ));
    }
    let liquid_request = resolve_liquid_request(first_id, first, second_id, second)?;
    let first_mask = masks.get(&first_id).cloned().unwrap_or_default();
    let second_mask = masks.get(&second_id).cloned().unwrap_or_default();
    let boundary_pairs = boundary_pairs(&first_mask, &second_mask);
    if boundary_pairs.is_empty() {
        return Err(LayoutValidationError::one(
            LayoutIssue::NonAdjacentSharedPatches(first_id, second_id),
        ));
    }
    let oriented_pairs: BTreeSet<_> = boundary_pairs
        .iter()
        .copied()
        .filter(|(first, second)| first_side.neighbor(*first) == *second)
        .collect();
    let mut requests =
        vec![PortRequest::Walker(first.walker.width); usize::from(first.walker.count)];
    if let LiquidRequest::Directed { width, .. } = liquid_request {
        requests.push(PortRequest::Liquid(width));
    }
    let selected = select_ports(
        &requests,
        &oriented_pairs,
        &first_mask,
        &second_mask,
        first_side,
        second_side,
        first.approach_depth,
    )
    .ok_or_else(|| {
        LayoutValidationError::one(LayoutIssue::InsufficientPortCapacity(first_id, second_id))
    })?;
    let mut walker_ports = Vec::with_capacity(usize::from(first.walker.count));
    let mut liquid_port = None;
    for (request, port) in requests.into_iter().zip(selected) {
        match request {
            PortRequest::Walker(_) => walker_ports.push(port),
            PortRequest::Liquid(_) => liquid_port = Some(port),
        }
    }
    let liquid = match liquid_request {
        LiquidRequest::Dry => ResolvedLiquidPort::Dry,
        LiquidRequest::Directed { source, sink, .. } => ResolvedLiquidPort::Directed {
            source,
            sink,
            port: liquid_port.expect("a directed request selects one liquid port"),
        },
    };
    let mut first_approaches = BTreeSet::new();
    let mut second_approaches = BTreeSet::new();
    for port in walker_ports
        .iter()
        .chain(liquid_port_ref(&liquid).into_iter())
    {
        first_approaches.extend(port.first_approach.iter().copied());
        second_approaches.extend(port.second_approach.iter().copied());
    }
    let protected_approaches =
        BTreeMap::from([(first_id, first_approaches), (second_id, second_approaches)]);
    Ok(ResolvedEdgeContract {
        first: (first_id, first_side),
        second: (second_id, second_side),
        elevation: ResolvedElevationBand {
            preferred: first.elevation.preferred,
            min: first.elevation.min,
            max: first.elevation.max,
        },
        walker: ResolvedWalkerPorts {
            ports: walker_ports,
        },
        liquid,
        approach_depth: first.approach_depth,
        boundary_pairs,
        protected_approaches,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LiquidRequest {
    Dry,
    Directed {
        source: PatchId,
        sink: PatchId,
        width: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PortRequest {
    Walker(u32),
    Liquid(u32),
}

impl PortRequest {
    const fn width(self) -> u32 {
        match self {
            Self::Walker(width) | Self::Liquid(width) => width,
        }
    }
}

fn resolve_liquid_request(
    first_id: PatchId,
    first: &SharedEdgeSettings,
    second_id: PatchId,
    second: &SharedEdgeSettings,
) -> Result<LiquidRequest, LayoutValidationError> {
    match (first.liquid, second.liquid) {
        (EdgeLiquidSettings::Dry, EdgeLiquidSettings::Dry) => Ok(LiquidRequest::Dry),
        (EdgeLiquidSettings::Outlet(first), EdgeLiquidSettings::Inlet(second))
            if first.width == second.width =>
        {
            Ok(LiquidRequest::Directed {
                source: first_id,
                sink: second_id,
                width: first.width,
            })
        }
        (EdgeLiquidSettings::Inlet(first), EdgeLiquidSettings::Outlet(second))
            if first.width == second.width =>
        {
            Ok(LiquidRequest::Directed {
                source: second_id,
                sink: first_id,
                width: first.width,
            })
        }
        _ => Err(LayoutValidationError::one(LayoutIssue::MismatchedLiquid(
            first_id, second_id,
        ))),
    }
}

fn liquid_port_ref(liquid: &ResolvedLiquidPort) -> Option<&ResolvedPort> {
    match liquid {
        ResolvedLiquidPort::Dry => None,
        ResolvedLiquidPort::Directed { port, .. } => Some(port),
    }
}

fn select_ports(
    requests: &[PortRequest],
    boundary_pairs: &BTreeSet<(HexCoord, HexCoord)>,
    first_mask: &BTreeSet<HexCoord>,
    second_mask: &BTreeSet<HexCoord>,
    first_side: HexSide,
    second_side: HexSide,
    approach_depth: u32,
) -> Option<Vec<ResolvedPort>> {
    if requests.is_empty() {
        return Some(Vec::new());
    }
    let required_lanes = requests
        .iter()
        .try_fold(0_u32, |total, request| total.checked_add(request.width()))?;
    if usize::try_from(required_lanes).ok()? > boundary_pairs.len() {
        return None;
    }

    let mut candidates = BTreeMap::<u32, Vec<ResolvedPort>>::new();
    for width in requests.iter().map(|request| request.width()) {
        candidates.entry(width).or_insert_with(|| {
            port_candidates(
                boundary_pairs,
                first_mask,
                second_mask,
                first_side,
                second_side,
                width,
                approach_depth,
            )
        });
    }
    if requests
        .iter()
        .any(|request| candidates.get(&request.width()).is_none_or(Vec::is_empty))
    {
        return None;
    }

    let mut selected = Vec::with_capacity(requests.len());
    let seam_leaves = seam_leaves(boundary_pairs);
    if choose_ports(requests, &candidates, &seam_leaves, 0, &mut selected) {
        Some(selected)
    } else {
        None
    }
}

fn choose_ports(
    requests: &[PortRequest],
    candidates: &BTreeMap<u32, Vec<ResolvedPort>>,
    seam_leaves: &BTreeSet<HexCoord>,
    request_index: usize,
    selected: &mut Vec<ResolvedPort>,
) -> bool {
    let Some(request) = requests.get(request_index) else {
        return true;
    };
    let Some(options) = candidates.get(&request.width()) else {
        return false;
    };
    let mut ordered: Vec<_> = options.iter().collect();
    ordered.sort_by(|first, second| {
        port_option_score(second, selected, seam_leaves)
            .cmp(&port_option_score(first, selected, seam_leaves))
            .then_with(|| first.cmp(second))
    });
    for candidate in ordered {
        if selected
            .iter()
            .all(|existing| ports_are_disjoint(existing, candidate))
        {
            selected.push(candidate.clone());
            if choose_ports(
                requests,
                candidates,
                seam_leaves,
                request_index + 1,
                selected,
            ) {
                return true;
            }
            selected.pop();
        }
    }
    false
}

fn seam_leaves(boundary_pairs: &BTreeSet<(HexCoord, HexCoord)>) -> BTreeSet<HexCoord> {
    let cells: BTreeSet<_> = boundary_pairs.iter().map(|(first, _)| *first).collect();
    cells
        .iter()
        .copied()
        .filter(|cell| {
            cell.neighbors()
                .into_iter()
                .filter(|neighbor| cells.contains(neighbor))
                .count()
                <= 1
        })
        .collect()
}

fn port_option_score(
    candidate: &ResolvedPort,
    selected: &[ResolvedPort],
    seam_leaves: &BTreeSet<HexCoord>,
) -> (u32, u32) {
    let candidate_cells: Vec<_> = candidate.lanes.iter().map(|(first, _)| *first).collect();
    let margin = seam_leaves
        .iter()
        .map(|leaf| {
            candidate_cells
                .iter()
                .map(|cell| leaf.distance(*cell))
                .min()
                .unwrap_or(0)
        })
        .min()
        .unwrap_or(0);
    let separation = selected
        .iter()
        .flat_map(|port| port.lanes.iter().map(|(first, _)| *first))
        .flat_map(|existing| {
            candidate_cells
                .iter()
                .map(move |cell| existing.distance(*cell))
        })
        .min()
        .unwrap_or(margin);
    (separation, margin)
}

fn ports_are_disjoint(first: &ResolvedPort, second: &ResolvedPort) -> bool {
    first.lanes.is_disjoint(&second.lanes)
        && first.lanes.iter().map(|(coord, _)| *coord).all(|coord| {
            second
                .lanes
                .iter()
                .all(|(other, _)| coord.distance(*other) > 1)
        })
        && first.lanes.iter().map(|(_, coord)| *coord).all(|coord| {
            second
                .lanes
                .iter()
                .all(|(_, other)| coord.distance(*other) > 1)
        })
        && first.first_approach.is_disjoint(&second.first_approach)
        && first.second_approach.is_disjoint(&second.second_approach)
}

fn port_candidates(
    boundary_pairs: &BTreeSet<(HexCoord, HexCoord)>,
    first_mask: &BTreeSet<HexCoord>,
    second_mask: &BTreeSet<HexCoord>,
    first_side: HexSide,
    second_side: HexSide,
    width: u32,
    approach_depth: u32,
) -> Vec<ResolvedPort> {
    let Ok(width) = usize::try_from(width) else {
        return Vec::new();
    };
    if width == 0 || width > boundary_pairs.len() {
        return Vec::new();
    }
    let lanes: Vec<_> = boundary_pairs.iter().copied().collect();
    let lane_by_first: BTreeMap<_, _> = lanes.iter().copied().collect();
    let mut lane_sets = BTreeSet::new();
    for lane in &lanes {
        let mut path = vec![*lane];
        let mut visited = BTreeSet::from([lane.0]);
        collect_lane_paths(
            *lane,
            width,
            &lane_by_first,
            &mut path,
            &mut visited,
            &mut lane_sets,
        );
    }
    lane_sets
        .into_iter()
        .filter_map(|lanes: BTreeSet<_>| {
            let first_boundary = lanes.iter().map(|(coord, _)| *coord).collect();
            let second_boundary = lanes.iter().map(|(_, coord)| *coord).collect();
            Some(ResolvedPort {
                lanes,
                first_approach: approach_corridor(
                    &first_boundary,
                    first_mask,
                    first_side.opposite(),
                    approach_depth,
                )?,
                second_approach: approach_corridor(
                    &second_boundary,
                    second_mask,
                    second_side.opposite(),
                    approach_depth,
                )?,
            })
        })
        .collect()
}

fn collect_lane_paths(
    current: (HexCoord, HexCoord),
    width: usize,
    lane_by_first: &BTreeMap<HexCoord, HexCoord>,
    path: &mut Vec<(HexCoord, HexCoord)>,
    visited: &mut BTreeSet<HexCoord>,
    results: &mut BTreeSet<BTreeSet<(HexCoord, HexCoord)>>,
) {
    if path.len() == width {
        results.insert(path.iter().copied().collect());
        return;
    }
    for first_neighbor in current.0.neighbors() {
        let Some(second_neighbor) = lane_by_first.get(&first_neighbor).copied() else {
            continue;
        };
        if current.1.distance(second_neighbor) != 1 || !visited.insert(first_neighbor) {
            continue;
        }
        path.push((first_neighbor, second_neighbor));
        collect_lane_paths(
            (first_neighbor, second_neighbor),
            width,
            lane_by_first,
            path,
            visited,
            results,
        );
        path.pop();
        visited.remove(&first_neighbor);
    }
}

fn generated_ring_masks(footprint: &BTreeSet<HexCoord>) -> BTreeMap<PatchId, BTreeSet<HexCoord>> {
    let centers = ring_centers();
    let mut masks = centers
        .keys()
        .copied()
        .map(|id| (id, BTreeSet::new()))
        .collect::<BTreeMap<_, _>>();
    for coord in footprint {
        let id = centers
            .iter()
            .min_by_key(|(id, center)| (coord.distance(**center), **id))
            .map(|(id, _)| *id)
            .unwrap_or(PatchId(0));
        masks.entry(id).or_default().insert(*coord);
    }
    masks
}

fn ring_centers() -> BTreeMap<PatchId, HexCoord> {
    BTreeMap::from([
        (PatchId(0), HexCoord::ORIGIN),
        (
            PatchId(1),
            HexCoord::new_cubic(RING_PATCH_OFFSET, -RING_PATCH_OFFSET, 0),
        ),
        (
            PatchId(2),
            HexCoord::new_cubic(RING_PATCH_OFFSET, 0, -RING_PATCH_OFFSET),
        ),
        (
            PatchId(3),
            HexCoord::new_cubic(0, RING_PATCH_OFFSET, -RING_PATCH_OFFSET),
        ),
        (
            PatchId(4),
            HexCoord::new_cubic(-RING_PATCH_OFFSET, RING_PATCH_OFFSET, 0),
        ),
        (
            PatchId(5),
            HexCoord::new_cubic(-RING_PATCH_OFFSET, 0, RING_PATCH_OFFSET),
        ),
        (
            PatchId(6),
            HexCoord::new_cubic(0, -RING_PATCH_OFFSET, RING_PATCH_OFFSET),
        ),
    ])
}

fn ring_seams() -> [(PatchId, HexSide, PatchId, HexSide); 12] {
    [
        (
            PatchId(0),
            HexSide::NorthEast,
            PatchId(1),
            HexSide::SouthWest,
        ),
        (PatchId(0), HexSide::East, PatchId(2), HexSide::West),
        (
            PatchId(0),
            HexSide::SouthEast,
            PatchId(3),
            HexSide::NorthWest,
        ),
        (
            PatchId(0),
            HexSide::SouthWest,
            PatchId(4),
            HexSide::NorthEast,
        ),
        (PatchId(0), HexSide::West, PatchId(5), HexSide::East),
        (
            PatchId(0),
            HexSide::NorthWest,
            PatchId(6),
            HexSide::SouthEast,
        ),
        (
            PatchId(1),
            HexSide::SouthEast,
            PatchId(2),
            HexSide::NorthWest,
        ),
        (
            PatchId(2),
            HexSide::SouthWest,
            PatchId(3),
            HexSide::NorthEast,
        ),
        (PatchId(3), HexSide::West, PatchId(4), HexSide::East),
        (
            PatchId(4),
            HexSide::NorthWest,
            PatchId(5),
            HexSide::SouthEast,
        ),
        (
            PatchId(5),
            HexSide::NorthEast,
            PatchId(6),
            HexSide::SouthWest,
        ),
        (PatchId(6), HexSide::East, PatchId(1), HexSide::West),
    ]
}

fn ring_specs(ring: &V3Ring7Settings) -> [(PatchId, &PatchSpec); 7] {
    [
        (PatchId(0), &ring.center),
        (PatchId(1), &ring.mountains),
        (PatchId(2), &ring.waterfall),
        (PatchId(3), &ring.forest),
        (PatchId(4), &ring.fort),
        (PatchId(5), &ring.caves),
        (PatchId(6), &ring.sky_islands),
    ]
}

fn spec_for(ring: &V3Ring7Settings, id: PatchId) -> &PatchSpec {
    match id.0 {
        0 => &ring.center,
        1 => &ring.mountains,
        2 => &ring.waterfall,
        3 => &ring.forest,
        4 => &ring.fort,
        5 => &ring.caves,
        6 => &ring.sky_islands,
        _ => unreachable!("fixed Ring7 patch id"),
    }
}

fn edge_setting(edges: &PatchEdgesSettings, side: HexSide) -> &PatchEdgeContractSettings {
    match side {
        HexSide::East => &edges.east,
        HexSide::SouthEast => &edges.south_east,
        HexSide::SouthWest => &edges.south_west,
        HexSide::West => &edges.west,
        HexSide::NorthWest => &edges.north_west,
        HexSide::NorthEast => &edges.north_east,
    }
}

fn explicit_mask(
    coords: &[crate::settings::CubeCoord],
    grid_radius: u32,
) -> Result<BTreeSet<HexCoord>, LayoutValidationError> {
    let mut mask = BTreeSet::new();
    for raw in coords {
        let Some(coord) = HexCoord::try_new_cubic(raw.x, raw.y, raw.z) else {
            return Err(LayoutValidationError::one(LayoutIssue::InvalidCube(
                raw.x, raw.y, raw.z,
            )));
        };
        if HexCoord::ORIGIN.distance(coord) > grid_radius {
            return Err(LayoutValidationError::one(LayoutIssue::MaskOutOfBounds(
                coord,
            )));
        }
        if !mask.insert(coord) {
            return Err(LayoutValidationError::one(LayoutIssue::DuplicateMaskCoord(
                coord,
            )));
        }
    }
    if mask.is_empty() || !connected(&mask) {
        return Err(LayoutValidationError::one(
            LayoutIssue::DisconnectedExplicitMask,
        ));
    }
    Ok(mask)
}

fn boundary_pairs(
    first: &BTreeSet<HexCoord>,
    second: &BTreeSet<HexCoord>,
) -> BTreeSet<(HexCoord, HexCoord)> {
    first
        .iter()
        .flat_map(|coord| {
            coord
                .neighbors()
                .into_iter()
                .filter(|neighbor| second.contains(neighbor))
                .map(|neighbor| (*coord, neighbor))
        })
        .collect()
}

fn approach_corridor(
    boundary: &BTreeSet<HexCoord>,
    mask: &BTreeSet<HexCoord>,
    inward: HexSide,
    depth: u32,
) -> Option<BTreeSet<HexCoord>> {
    if depth == 0 {
        return Some(BTreeSet::new());
    }
    let mut corridor = BTreeSet::new();
    for boundary_cell in boundary {
        let mut cell = *boundary_cell;
        for _ in 0..depth {
            if !mask.contains(&cell) {
                return None;
            }
            corridor.insert(cell);
            cell = inward.neighbor(cell);
        }
    }
    Some(corridor)
}

fn connected(mask: &BTreeSet<HexCoord>) -> bool {
    let Some(start) = mask.first().copied() else {
        return false;
    };
    let mut reached = BTreeSet::from([start]);
    let mut frontier = VecDeque::from([start]);
    while let Some(coord) = frontier.pop_front() {
        for neighbor in coord.neighbors() {
            if mask.contains(&neighbor) && reached.insert(neighbor) {
                frontier.push_back(neighbor);
            }
        }
    }
    reached.len() == mask.len()
}

fn validate_resolved_edge(
    id: ResolvedEdgeId,
    edge: &ResolvedEdgeContract,
    patches: &BTreeMap<PatchId, ResolvedPatch>,
    issues: &mut Vec<LayoutIssue>,
) {
    let Some(first) = patches.get(&edge.first.0) else {
        issues.push(LayoutIssue::MissingEdgePatch(id, edge.first.0));
        return;
    };
    let Some(second) = patches.get(&edge.second.0) else {
        issues.push(LayoutIssue::MissingEdgePatch(id, edge.second.0));
        return;
    };
    let expected_pairs = boundary_pairs(&first.mask, &second.mask);
    if edge.boundary_pairs.is_empty() || edge.boundary_pairs != expected_pairs {
        issues.push(LayoutIssue::InvalidBoundaryPairs(id));
    }
    let all_ports: Vec<_> = edge
        .walker
        .ports
        .iter()
        .chain(liquid_port_ref(&edge.liquid))
        .collect();
    let ports_valid = all_ports
        .iter()
        .all(|port| valid_resolved_port(port, edge, &first.mask, &second.mask, &expected_pairs))
        && all_ports.iter().enumerate().all(|(index, port)| {
            all_ports
                .iter()
                .skip(index + 1)
                .all(|other| ports_are_disjoint(port, other))
        });
    let expected_first_approach = all_ports
        .iter()
        .flat_map(|port| port.first_approach.iter().copied())
        .collect();
    let expected_second_approach = all_ports
        .iter()
        .flat_map(|port| port.second_approach.iter().copied())
        .collect();
    let expected_approaches = BTreeMap::from([
        (edge.first.0, expected_first_approach),
        (edge.second.0, expected_second_approach),
    ]);
    if edge.protected_approaches != expected_approaches {
        for patch in [edge.first.0, edge.second.0] {
            issues.push(LayoutIssue::InvalidProtectedApproach(id, patch));
        }
    }
    if edge.first.1.opposite() != edge.second.1
        || edge.elevation.min > edge.elevation.preferred
        || edge.elevation.preferred > edge.elevation.max
        || edge.walker.ports.iter().any(|port| port.lanes.len() < 2)
        || !ports_valid
    {
        issues.push(LayoutIssue::InvalidResolvedContract(id));
    }
    if let ResolvedLiquidPort::Directed { source, sink, port } = &edge.liquid {
        let endpoints = BTreeSet::from([edge.first.0, edge.second.0]);
        if source == sink
            || !endpoints.contains(source)
            || !endpoints.contains(sink)
            || port.lanes.len() < 2
        {
            issues.push(LayoutIssue::InvalidResolvedContract(id));
        }
    }
}

fn valid_resolved_port(
    port: &ResolvedPort,
    edge: &ResolvedEdgeContract,
    first_mask: &BTreeSet<HexCoord>,
    second_mask: &BTreeSet<HexCoord>,
    boundary_pairs: &BTreeSet<(HexCoord, HexCoord)>,
) -> bool {
    if port.lanes.is_empty()
        || !port.lanes.is_subset(boundary_pairs)
        || port
            .lanes
            .iter()
            .any(|(first, second)| edge.first.1.neighbor(*first) != *second)
    {
        return false;
    }
    let first_boundary: BTreeSet<_> = port.lanes.iter().map(|(coord, _)| *coord).collect();
    let second_boundary: BTreeSet<_> = port.lanes.iter().map(|(_, coord)| *coord).collect();
    first_boundary.len() == port.lanes.len()
        && second_boundary.len() == port.lanes.len()
        && connected(&first_boundary)
        && connected(&second_boundary)
        && approach_corridor(
            &first_boundary,
            first_mask,
            edge.first.1.opposite(),
            edge.approach_depth,
        )
        .is_some_and(|expected| port.first_approach == expected)
        && approach_corridor(
            &second_boundary,
            second_mask,
            edge.second.1.opposite(),
            edge.approach_depth,
        )
        .is_some_and(|expected| port.second_approach == expected)
}

/// One deterministic resolved-layout contract failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LayoutIssue {
    UnsupportedRadius(u32),
    InvalidFootprint,
    FootprintOutOfBounds,
    InvalidRingRadius(u32),
    InvalidRingFootprint,
    PatchCount { expected: usize, actual: usize },
    DisconnectedPatch(PatchId),
    PatchOutsideFootprint(PatchId),
    OverlappingPatch(PatchId, HexCoord),
    DuplicateBiomeRegion(BiomeRegionId),
    IncompletePatchEdges(PatchId),
    IncompleteCoverage,
    SharedReferenceMismatch(ResolvedEdgeId),
    MissingSharedEdge(ResolvedEdgeId),
    MissingEdgePatch(ResolvedEdgeId, PatchId),
    InvalidBoundaryPairs(ResolvedEdgeId),
    InvalidProtectedApproach(ResolvedEdgeId, PatchId),
    InvalidResolvedContract(ResolvedEdgeId),
    GeneratedSingleMask,
    SharedSingleEdge(HexSide),
    MixedRingMasks,
    InvalidCube(i32, i32, i32),
    MaskOutOfBounds(HexCoord),
    DuplicateMaskCoord(HexCoord),
    DisconnectedExplicitMask,
    UnpairedSharedEdge(PatchId, HexSide, PatchId, HexSide),
    MismatchedSharedEdge(PatchId, PatchId),
    MismatchedLiquid(PatchId, PatchId),
    NonAdjacentSharedPatches(PatchId, PatchId),
    InsufficientPortCapacity(PatchId, PatchId),
    UnexpectedEdgeKind(PatchId, HexSide),
}

impl fmt::Display for LayoutIssue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

/// All layout failures found in one validation or resolution pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LayoutValidationError {
    issues: Vec<LayoutIssue>,
}

impl LayoutValidationError {
    fn one(issue: LayoutIssue) -> Self {
        Self {
            issues: vec![issue],
        }
    }

    #[must_use]
    pub(crate) fn issues(&self) -> &[LayoutIssue] {
        &self.issues
    }
}

impl fmt::Display for LayoutValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let details = self
            .issues
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ");
        write!(formatter, "invalid V3 resolved layout: {details}")
    }
}

impl std::error::Error for LayoutValidationError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{
        EdgeElevationSettings, EdgeLiquidPortSettings, PatchEdgesSettings, V3CavesSettings,
        V3EnvironmentSettings, V3ForestSettings, V3FortSettings, V3HillsSettings,
        V3MountainsSettings, V3RecipeSettings, V3SkyIslandsSettings, V3WaterfallSettings,
        WalkerPortSettings,
    };

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

    fn shared(liquid: EdgeLiquidSettings) -> PatchEdgeContractSettings {
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

    fn patch(environment: V3EnvironmentSettings, recipe: V3RecipeSettings) -> PatchSpec {
        PatchSpec {
            environment,
            recipe,
            overlays: Vec::new(),
            mask: PatchMaskSettings::GeneratedRegion,
            edges: world_edges(),
        }
    }

    fn set_edge(edges: &mut PatchEdgesSettings, side: HexSide, value: PatchEdgeContractSettings) {
        *match side {
            HexSide::East => &mut edges.east,
            HexSide::SouthEast => &mut edges.south_east,
            HexSide::SouthWest => &mut edges.south_west,
            HexSide::West => &mut edges.west,
            HexSide::NorthWest => &mut edges.north_west,
            HexSide::NorthEast => &mut edges.north_east,
        } = value;
    }

    fn ring_settings() -> ProceduralV3Settings {
        let mut ring = V3Ring7Settings {
            center: patch(
                V3EnvironmentSettings::TemperateGrassland,
                V3RecipeSettings::Hills(V3HillsSettings {
                    valley_level: 15,
                    max_relief: 8,
                    hills_per_bank: 3,
                }),
            ),
            mountains: patch(
                V3EnvironmentSettings::Frozen,
                V3RecipeSettings::Mountains(V3MountainsSettings {
                    base_level: 15,
                    relief: 18,
                    peak_count: 5,
                }),
            ),
            waterfall: patch(
                V3EnvironmentSettings::TemperateGrassland,
                V3RecipeSettings::Waterfall(V3WaterfallSettings),
            ),
            forest: patch(
                V3EnvironmentSettings::TemperateGrassland,
                V3RecipeSettings::Forest(V3ForestSettings),
            ),
            fort: patch(
                V3EnvironmentSettings::TemperateGrassland,
                V3RecipeSettings::Fort(V3FortSettings),
            ),
            caves: patch(
                V3EnvironmentSettings::Rocky,
                V3RecipeSettings::Caves(V3CavesSettings {
                    surface_level: 15,
                    cave_floor_level: 7,
                    chamber_count: 8,
                }),
            ),
            sky_islands: patch(
                V3EnvironmentSettings::TemperateGrassland,
                V3RecipeSettings::SkyIslands(V3SkyIslandsSettings {
                    ground: V3HillsSettings {
                        valley_level: 15,
                        max_relief: 8,
                        hills_per_bank: 3,
                    },
                    min_clearance: 12,
                    upper_coverage_percent: 20,
                }),
            ),
        };
        for (first, first_side, second, second_side) in ring_seams() {
            set_edge(
                &mut spec_mut(&mut ring, first).edges,
                first_side,
                shared(EdgeLiquidSettings::Dry),
            );
            set_edge(
                &mut spec_mut(&mut ring, second).edges,
                second_side,
                shared(EdgeLiquidSettings::Dry),
            );
        }
        ProceduralV3Settings {
            layout: V3LayoutSettings::Ring7(ring),
        }
    }

    fn spec_mut(ring: &mut V3Ring7Settings, id: PatchId) -> &mut PatchSpec {
        match id.0 {
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

    #[test]
    fn single_whole_world_resolves_exactly_once() {
        let settings = ProceduralV3Settings {
            layout: V3LayoutSettings::Single(PatchSpec {
                environment: V3EnvironmentSettings::TemperateGrassland,
                recipe: V3RecipeSettings::Hills(V3HillsSettings {
                    valley_level: 15,
                    max_relief: 8,
                    hills_per_bank: 3,
                }),
                overlays: Vec::new(),
                mask: PatchMaskSettings::WholeWorld,
                edges: world_edges(),
            }),
        };
        let resolved = resolve_layout(12, &settings).expect("valid Single layout");

        assert_eq!(resolved.footprint.len(), 469);
        assert_eq!(resolved.patches.len(), 1);
        assert!(resolved.shared_edges.is_empty());
        assert!(resolved.validate().is_ok());
    }

    #[test]
    fn generated_ring_is_disjoint_connected_and_exactly_covered() {
        let resolved = resolve_layout(33, &ring_settings()).expect("valid Ring7 layout");

        assert_eq!(resolved.footprint.len(), 3_367);
        assert_eq!(resolved.patches.len(), 7);
        assert_eq!(resolved.shared_edges.len(), 12);
        assert!(resolved
            .patches
            .values()
            .all(|patch| connected(&patch.mask)));
        assert!(resolved
            .shared_edges
            .values()
            .all(|edge| !edge.boundary_pairs.is_empty()));
    }

    #[test]
    fn generated_ring_pins_clean_lane_capacity_for_every_seam() {
        let resolved = resolve_layout(33, &ring_settings()).expect("valid Ring7 layout");
        let capacities = resolved
            .shared_edges
            .values()
            .map(|edge| {
                let oriented = edge
                    .boundary_pairs
                    .iter()
                    .filter(|(first, second)| edge.first.1.neighbor(*first) == *second)
                    .count();
                (edge.boundary_pairs.len(), oriented)
            })
            .fold(BTreeMap::<_, usize>::new(), |mut counts, capacity| {
                *counts.entry(capacity).or_default() += 1;
                counts
            });

        assert_eq!(capacities, BTreeMap::from([((29, 15), 6), ((37, 19), 6)]));
    }

    #[test]
    fn shared_edges_are_one_object_with_two_references() {
        let resolved = resolve_layout(33, &ring_settings()).expect("valid Ring7 layout");

        for (edge_id, edge) in &resolved.shared_edges {
            let references = resolved
                .patches
                .values()
                .flat_map(|patch| patch.edges.values())
                .filter(|reference| **reference == ResolvedEdgeReference::Shared(*edge_id))
                .count();
            assert_eq!(references, 2);
            assert_eq!(edge.first.1.opposite(), edge.second.1);
            assert_eq!(edge.protected_approaches.len(), 2);
            assert_eq!(edge.walker.ports.len(), 2);
            for port in &edge.walker.ports {
                assert_eq!(port.lanes.len(), 2);
                assert_eq!(port.first_approach.len(), 6);
                assert_eq!(port.second_approach.len(), 6);
                assert!(port
                    .lanes
                    .iter()
                    .all(|(first, second)| edge.first.1.neighbor(*first) == *second));
            }
            assert!(ports_are_disjoint(
                &edge.walker.ports[0],
                &edge.walker.ports[1]
            ));
        }
    }

    #[test]
    fn liquid_direction_is_resolved_once() {
        let mut settings = ring_settings();
        let V3LayoutSettings::Ring7(ring) = &mut settings.layout else {
            unreachable!();
        };
        let outlet = EdgeLiquidSettings::Outlet(EdgeLiquidPortSettings { width: 3 });
        let inlet = EdgeLiquidSettings::Inlet(EdgeLiquidPortSettings { width: 3 });
        set_edge(&mut ring.center.edges, HexSide::East, shared(outlet));
        set_edge(&mut ring.waterfall.edges, HexSide::West, shared(inlet));
        let resolved = resolve_layout(33, &settings).expect("reciprocal liquid seam");
        let Some(edge) = resolved.shared_edges.get(&ResolvedEdgeId(1)) else {
            panic!("center/waterfall seam should resolve");
        };
        let ResolvedLiquidPort::Directed { source, sink, port } = &edge.liquid else {
            panic!("the reciprocal liquid seam should resolve one directed port")
        };
        assert_eq!((*source, *sink), (PatchId(0), PatchId(2)));
        assert_eq!(port.lanes.len(), 3);
        assert_eq!(port.first_approach.len(), 9);
        assert_eq!(port.second_approach.len(), 9);
        assert!(edge
            .walker
            .ports
            .iter()
            .all(|walker| ports_are_disjoint(walker, port)));
        assert!(resolved.validate().is_ok());
    }

    #[test]
    fn exact_port_resolution_is_deterministic() {
        let settings = ring_settings();
        let first = resolve_layout(33, &settings).expect("valid Ring7 layout");
        let second = resolve_layout(33, &settings).expect("valid Ring7 layout");

        assert_eq!(first, second);
    }

    #[test]
    fn insufficient_clean_lane_capacity_fails_resolution() {
        let mut settings = ring_settings();
        let V3LayoutSettings::Ring7(ring) = &mut settings.layout else {
            unreachable!();
        };
        for edge in [&mut ring.center.edges.east, &mut ring.waterfall.edges.west] {
            let PatchEdgeContractSettings::Shared(shared) = edge else {
                unreachable!();
            };
            shared.walker.width = 8;
        }

        let error = resolve_layout(33, &settings)
            .expect_err("two eight-wide ports cannot fit a fifteen-lane seam");
        assert!(error
            .issues()
            .contains(&LayoutIssue::InsufficientPortCapacity(
                PatchId(0),
                PatchId(2)
            )));
    }

    #[test]
    fn resolved_port_corruption_is_rejected() {
        let mut resolved = resolve_layout(33, &ring_settings()).expect("valid Ring7 layout");
        let edge = resolved
            .shared_edges
            .get_mut(&ResolvedEdgeId(0))
            .expect("the first fixed seam exists");
        let lane = edge.walker.ports[0]
            .lanes
            .first()
            .copied()
            .expect("walker ports are non-empty");
        edge.walker.ports[0].lanes.remove(&lane);

        let error = resolved
            .validate()
            .expect_err("a one-lane resolved walker port is corrupt");
        assert!(error
            .issues()
            .contains(&LayoutIssue::InvalidResolvedContract(ResolvedEdgeId(0))));
    }

    #[test]
    fn mismatched_shared_contract_is_rejected() {
        let mut settings = ring_settings();
        let V3LayoutSettings::Ring7(ring) = &mut settings.layout else {
            unreachable!();
        };
        let PatchEdgeContractSettings::Shared(edge) = &mut ring.center.edges.east else {
            unreachable!();
        };
        edge.elevation.preferred = 16;
        assert!(resolve_layout(33, &settings).is_err());
    }
}
