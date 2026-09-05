//! Shared ordinary-walker contracts for V3 patch seams.
//!
//! Patch recipes shape their local approach surfaces before voxelization. The
//! complete world validator later proves that only the exact declared lanes cross
//! between patches.

use std::collections::{BTreeMap, BTreeSet};

use hex_core::{HexCoord, Level, SpecialMovementRegion, TilePos, TraversalProfile};

use crate::settings::MAX_SEAM_PORT_WIDTH;

use super::layout::{LayoutKind, PatchId, ResolvedEdgeContract, ResolvedEdgeId};
use super::patch::PatchRecipeContext;
use super::traversal::OrdinaryGraph;
use super::volume::{SurfaceAccess, VolumePlan};
use super::world::{GeneratedWorldPlan, WorldIssueCode, WorldValidationIssue};

const LEGACY_SEAM_CLOSURE_REGION_BASE: u32 = 0x0ffe_0000;
const LEGACY_SEAM_CLOSURE_REGION_LIMIT: u32 = LEGACY_SEAM_CLOSURE_REGION_BASE + 0x1_0000;
const RING19_SEAM_CLOSURE_REGION_BASE: u32 = 0x07fe_0000;
const RING19_SEAM_CLOSURE_REGION_LIMIT: u32 = RING19_SEAM_CLOSURE_REGION_BASE + 0x1_0000;
const MACRO_SEAM_CLOSURE_REGION_BASE: u32 = 0x03fe_0000;
const MACRO_SEAM_CLOSURE_REGION_LIMIT: u32 = MACRO_SEAM_CLOSURE_REGION_BASE + 0x1_0000;

/// Whether an access marker is an exact shared-seam closure rather than a
/// recipe-owned special-movement region.
#[must_use]
pub(crate) const fn is_seam_closure_access(access: SurfaceAccess) -> bool {
    matches!(
        access,
        SurfaceAccess::SpecialMovement(region)
            if (region.0 >= LEGACY_SEAM_CLOSURE_REGION_BASE
                && region.0 < LEGACY_SEAM_CLOSURE_REGION_LIMIT)
                || (region.0 >= RING19_SEAM_CLOSURE_REGION_BASE
                    && region.0 < RING19_SEAM_CLOSURE_REGION_LIMIT)
                || (region.0 >= MACRO_SEAM_CLOSURE_REGION_BASE
                    && region.0 < MACRO_SEAM_CLOSURE_REGION_LIMIT)
    )
}

const fn seam_closure_region(kind: LayoutKind, edge: ResolvedEdgeId) -> SpecialMovementRegion {
    let base = match kind {
        LayoutKind::Single | LayoutKind::Ring7 => LEGACY_SEAM_CLOSURE_REGION_BASE,
        LayoutKind::Ring19 => RING19_SEAM_CLOSURE_REGION_BASE,
        LayoutKind::Macro => MACRO_SEAM_CLOSURE_REGION_BASE,
        LayoutKind::Schematic => MACRO_SEAM_CLOSURE_REGION_BASE,
    };
    SpecialMovementRegion(base.saturating_add(edge.0))
}

/// Exact local consequences of shaping one patch's shared walker seams.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct WalkerSeamShape {
    boundary_regions: BTreeMap<HexCoord, SpecialMovementRegion>,
    open_surfaces: BTreeSet<TilePos>,
}

impl WalkerSeamShape {
    /// Exact ordinary surface required at this protected approach coordinate.
    #[must_use]
    pub(crate) fn required_surface(&self, coord: HexCoord) -> Option<TilePos> {
        self.open_surfaces
            .iter()
            .find(|surface| surface.coord == coord)
            .copied()
    }

    /// Whether a column participates in any shared boundary.
    #[must_use]
    pub(crate) fn is_boundary(&self, coord: HexCoord) -> bool {
        self.boundary_regions.contains_key(&coord)
    }

    /// Projects one surface access value through the resolved seam closure.
    #[must_use]
    pub(crate) fn access_for(&self, surface: TilePos, access: SurfaceAccess) -> SurfaceAccess {
        let Some(region) = self.boundary_regions.get(&surface.coord).copied() else {
            return access;
        };
        if !self.open_surfaces.contains(&surface) && access == SurfaceAccess::Ordinary {
            SurfaceAccess::SpecialMovement(region)
        } else {
            access
        }
    }

    /// Projects every non-aperture boundary surface into special movement.
    pub(crate) fn apply(&self, volume: &mut VolumePlan) -> Result<(), Vec<WorldValidationIssue>> {
        let mut issues = Vec::new();
        for (surface, metadata) in &mut volume.surfaces {
            metadata.access = self.access_for(*surface, metadata.access);
        }
        for surface in &self.open_surfaces {
            if !matches!(
                volume.surfaces.get(surface).map(|metadata| metadata.access),
                Some(SurfaceAccess::Ordinary)
            ) {
                let actual = volume
                    .surfaces
                    .iter()
                    .filter(|(candidate, _)| candidate.coord == surface.coord)
                    .map(|(candidate, metadata)| (*candidate, metadata.access))
                    .collect::<Vec<_>>();
                issues.push(seam_issue(format!(
                    "declared seam aperture has no exact ordinary surface at {surface:?}; actual surfaces at the coordinate: {actual:?}"
                )));
            }
        }
        if issues.is_empty() {
            Ok(())
        } else {
            Err(issues)
        }
    }
}

/// Fits exact two-wide walker ports to their shared datum and identifies closures.
pub(crate) fn shape_walker_seams(
    patch: &PatchRecipeContext<'_>,
    levels: &mut BTreeMap<HexCoord, Level>,
) -> Result<WalkerSeamShape, Vec<WorldValidationIssue>> {
    let mut issues = Vec::new();
    let mut open_coords = BTreeSet::new();
    let mut open_surfaces = BTreeSet::new();
    let mut edges = Vec::new();

    for edge in patch.shared_edges() {
        let ports = edge.walker_ports();
        if !valid_walker_contract(
            edge.contract.walker.count,
            edge.contract.walker.width,
            &ports,
        ) {
            issues.push(seam_issue(format!(
                "patch {} seam {:?} must declare exact two-wide walker ports",
                patch.id.0, edge.id
            )));
            continue;
        }
        for port in &ports {
            open_coords.extend(port.first_approach.iter().copied());
        }
        edges.push((edge, ports));
    }
    if !issues.is_empty() {
        return Err(issues);
    }

    let mut fixed_macro_approaches = BTreeMap::<HexCoord, Level>::new();
    if patch.layout().kind == LayoutKind::Macro {
        for (edge, ports) in &edges {
            let preferred = edge.preferred_level();
            for coord in ports
                .iter()
                .flat_map(|port| port.first_approach.iter().copied())
            {
                if fixed_macro_approaches
                    .insert(coord, preferred)
                    .is_some_and(|existing| existing != preferred)
                {
                    issues.push(seam_issue(format!(
                        "patch {} Macro route approaches require conflicting levels at {coord:?}",
                        patch.id.0
                    )));
                }
            }
        }
    }
    if !issues.is_empty() {
        return Err(issues);
    }

    for (edge, ports) in &edges {
        let preferred = edge.preferred_level();
        let approaches: BTreeSet<_> = ports
            .iter()
            .flat_map(|port| port.first_approach.iter().copied())
            .collect();
        for (coord, level) in levels.iter_mut() {
            if fixed_macro_approaches.contains_key(coord) {
                continue;
            }
            let distance = approaches
                .iter()
                .map(|approach| approach.distance(*coord))
                .min()
                .unwrap_or(u32::MAX);
            let distance = i32::try_from(distance).unwrap_or(i32::MAX);
            *level = (*level)
                .min(preferred.saturating_add(distance))
                .max(preferred.saturating_sub(distance));
        }
        for coord in approaches {
            match levels.get_mut(&coord) {
                Some(level) => {
                    *level = preferred;
                    open_surfaces.insert(TilePos::new(coord, preferred));
                }
                None => issues.push(seam_issue(format!(
                    "patch {} seam {:?} approach {coord:?} is outside its level plan",
                    patch.id.0, edge.id
                ))),
            }
        }
    }
    if !issues.is_empty() {
        return Err(issues);
    }

    let mut boundary_regions = BTreeMap::new();
    for (edge, _) in edges {
        let region = seam_closure_region(patch.layout().kind, edge.id);
        for (local, _) in edge.boundary_pairs() {
            if !levels.contains_key(&local) {
                issues.push(seam_issue(format!(
                    "patch {} seam {:?} boundary {local:?} is outside its level plan",
                    patch.id.0, edge.id
                )));
                continue;
            }
            boundary_regions
                .entry(local)
                .and_modify(|existing| {
                    if region < *existing {
                        *existing = region;
                    }
                })
                .or_insert(region);
        }
    }

    if issues.is_empty() {
        debug_assert!(open_surfaces
            .iter()
            .all(|surface| open_coords.contains(&surface.coord)));
        Ok(WalkerSeamShape {
            boundary_regions,
            open_surfaces,
        })
    } else {
        Err(issues)
    }
}

/// Checks one patch's exact local endpoint, approach, and closure contract.
#[must_use]
pub(crate) fn validate_patch_walker_seams(
    patch: &PatchRecipeContext<'_>,
    volume: &VolumePlan,
) -> Vec<WorldValidationIssue> {
    let mut issues = Vec::new();
    let ordinary = OrdinaryGraph::from_volume(volume, None);

    for edge in patch.shared_edges() {
        let ports = edge.walker_ports();
        if !valid_walker_contract(
            edge.contract.walker.count,
            edge.contract.walker.width,
            &ports,
        ) {
            issues.push(seam_issue(format!(
                "patch {} seam {:?} does not retain exact two-wide walker ports",
                patch.id.0, edge.id
            )));
            continue;
        }
        for port in &ports {
            let mut landings = BTreeSet::new();
            for (local, _) in &port.lanes {
                let position = TilePos::new(*local, edge.preferred_level());
                validate_local_surface(patch.id, edge.id, position, volume, &ordinary, &mut issues);
                landings.insert(position);
            }
            for coord in &port.first_approach {
                let position = TilePos::new(*coord, edge.preferred_level());
                validate_local_surface(patch.id, edge.id, position, volume, &ordinary, &mut issues);
                if !landings
                    .iter()
                    .any(|landing| ordinary.distances_from(*landing).contains_key(&position))
                {
                    issues.push(seam_issue(format!(
                        "patch {} seam {:?} approach {position:?} is disconnected from its port",
                        patch.id.0, edge.id
                    )));
                }
            }
        }
    }

    let mut expected_closures = BTreeMap::<HexCoord, SpecialMovementRegion>::new();
    for edge in patch.shared_edges() {
        let region = seam_closure_region(patch.layout().kind, edge.id);
        for (local, _) in edge.boundary_pairs() {
            expected_closures
                .entry(local)
                .and_modify(|existing| {
                    if region < *existing {
                        *existing = region;
                    }
                })
                .or_insert(region);
        }
    }
    for edge in patch.shared_edges() {
        let edge_open = edge
            .walker_ports()
            .into_iter()
            .flat_map(|port| port.first_approach)
            .map(|coord| TilePos::new(coord, edge.preferred_level()))
            .collect::<BTreeSet<_>>();
        for (local, _) in edge.boundary_pairs() {
            for (surface, metadata) in volume
                .surfaces
                .iter()
                .filter(|(surface, _)| surface.coord == local)
            {
                if metadata.access == SurfaceAccess::Ordinary && !edge_open.contains(surface) {
                    issues.push(seam_issue(format!(
                        "patch {} seam {:?} leaves undeclared boundary surface {surface:?} ordinary",
                        patch.id.0, edge.id
                    )));
                }
            }
        }
    }
    for (coord, expected_region) in expected_closures {
        let contains_fill = volume.columns.get(&coord).is_some_and(|column| {
            column
                .elements
                .iter()
                .any(|element| matches!(element, super::volume::VolumeElement::Fill(_)))
        });
        for (surface, metadata) in volume
            .surfaces
            .iter()
            .filter(|(surface, _)| surface.coord == coord)
        {
            if metadata.access == SurfaceAccess::Ordinary
                || (metadata.access == SurfaceAccess::NonStandable && contains_fill)
            {
                continue;
            }
            if metadata.access != SurfaceAccess::SpecialMovement(expected_region) {
                issues.push(seam_issue(format!(
                    "patch {} shared boundary surface {surface:?} uses {:?}, expected exact seam \
                     closure {:?}",
                    patch.id.0, metadata.access, expected_region
                )));
            }
        }
    }
    issues
}

/// Checks exact declared transitions and rejects every other ordinary seam crossing.
pub(crate) fn validate_world_walker_seams(
    plan: &GeneratedWorldPlan,
    issues: &mut Vec<WorldValidationIssue>,
) {
    if plan.layout.shared_edges.is_empty() {
        return;
    }

    let ordinary = OrdinaryGraph::from_volume(&plan.volume, Some(&plan.blockers));
    for (edge_id, edge) in &plan.layout.shared_edges {
        if !valid_walker_contract(edge.walker.count, edge.walker.width, &edge.walker.ports) {
            issues.push(seam_issue(format!(
                "shared seam {edge_id:?} does not retain its exact declared walker-port width"
            )));
            continue;
        }
        let declared_lanes: BTreeSet<_> = edge
            .walker
            .ports
            .iter()
            .flat_map(|port| port.lanes.iter().copied())
            .collect();
        let admitted_pairs = admitted_walker_boundary_pairs(edge);

        for (first, second) in &declared_lanes {
            let first = TilePos::new(*first, edge.elevation.preferred);
            let second = TilePos::new(*second, edge.elevation.preferred);
            validate_world_surface(*edge_id, first, &plan.volume, &ordinary, issues);
            validate_world_surface(*edge_id, second, &plan.volume, &ordinary, issues);
            if !ordinary.admits(first, second) || !ordinary.admits(second, first) {
                issues.push(seam_issue(format!(
                    "shared seam {edge_id:?} exact lane {first:?} -> {second:?} lacks a symmetric walker aperture"
                )));
            }
        }

        for port in &edge.walker.ports {
            for coord in port.first_approach.iter().chain(&port.second_approach) {
                validate_world_surface(
                    *edge_id,
                    TilePos::new(*coord, edge.elevation.preferred),
                    &plan.volume,
                    &ordinary,
                    issues,
                );
            }
        }
        for patch in edge.protected_approaches.keys() {
            if !plan.layout.patches.contains_key(patch) {
                issues.push(seam_issue(format!(
                    "shared seam {edge_id:?} protects an unknown patch {}",
                    patch.0
                )));
            }
        }

        for (first_coord, second_coord) in &edge.boundary_pairs {
            let first = ordinary
                .positions()
                .filter(|surface| surface.coord == *first_coord)
                .collect::<Vec<_>>();
            let second = ordinary
                .positions()
                .filter(|surface| surface.coord == *second_coord)
                .collect::<Vec<_>>();
            for first in &first {
                for second in &second {
                    if ordinary.admits(*first, *second) || ordinary.admits(*second, *first) {
                        let declared_exact = admitted_pairs
                            .contains(&(*first_coord, *second_coord))
                            && first.level == edge.elevation.preferred
                            && second.level == edge.elevation.preferred;
                        let declared_spanning =
                            plan.features.protected_routes.values().any(|route| {
                                route.surfaces.contains(first) && route.surfaces.contains(second)
                            });
                        if !declared_exact && !declared_spanning {
                            issues.push(seam_issue(format!(
                                "shared seam {edge_id:?} admits undeclared crossing {first:?} -> {second:?}"
                            )));
                        }
                    }
                }
            }
        }
    }
}

fn admitted_walker_boundary_pairs(edge: &ResolvedEdgeContract) -> BTreeSet<(HexCoord, HexCoord)> {
    edge.boundary_pairs
        .iter()
        .copied()
        .filter(|(first, second)| {
            edge.walker.ports.iter().any(|port| {
                port.lanes.iter().any(|(coord, _)| coord == first)
                    && port.lanes.iter().any(|(_, coord)| coord == second)
            })
        })
        .collect()
}

fn validate_local_surface(
    patch: PatchId,
    edge: super::layout::ResolvedEdgeId,
    position: TilePos,
    volume: &VolumePlan,
    ordinary: &OrdinaryGraph,
    issues: &mut Vec<WorldValidationIssue>,
) {
    if !matches!(
        volume
            .surfaces
            .get(&position)
            .map(|metadata| metadata.access),
        Some(SurfaceAccess::Ordinary)
    ) {
        issues.push(seam_issue(format!(
            "patch {} seam {edge:?} requires exact ordinary surface {position:?}",
            patch.0
        )));
        return;
    }
    if volume
        .surface_headroom(position)
        .is_none_or(|headroom| headroom.0 < TraversalProfile::WALKER.levels_tall)
    {
        issues.push(seam_issue(format!(
            "patch {} seam {edge:?} surface {position:?} lacks two-level headroom",
            patch.0
        )));
    }
    if !ordinary.contains(position) {
        issues.push(seam_issue(format!(
            "patch {} seam {edge:?} surface {position:?} is absent from ordinary traversal",
            patch.0
        )));
    }
}

fn validate_world_surface(
    edge: super::layout::ResolvedEdgeId,
    position: TilePos,
    volume: &VolumePlan,
    ordinary: &OrdinaryGraph,
    issues: &mut Vec<WorldValidationIssue>,
) {
    if !matches!(
        volume
            .surfaces
            .get(&position)
            .map(|metadata| metadata.access),
        Some(SurfaceAccess::Ordinary)
    ) {
        issues.push(seam_issue(format!(
            "shared seam {edge:?} requires exact ordinary surface {position:?}"
        )));
        return;
    }
    if volume
        .surface_headroom(position)
        .is_none_or(|headroom| headroom.0 < TraversalProfile::WALKER.levels_tall)
    {
        issues.push(seam_issue(format!(
            "shared seam {edge:?} surface {position:?} lacks two-level headroom"
        )));
    }
    if !ordinary.contains(position) {
        issues.push(seam_issue(format!(
            "shared seam {edge:?} surface {position:?} is blocked from ordinary traversal"
        )));
    }
}

fn seam_issue(detail: impl Into<String>) -> WorldValidationIssue {
    WorldValidationIssue::new(WorldIssueCode::Traversal, detail)
}

fn valid_walker_contract(count: u8, width: u32, ports: &[super::layout::ResolvedPort]) -> bool {
    if count == 0 {
        return width == 0 && ports.is_empty();
    }
    (2..=MAX_SEAM_PORT_WIDTH).contains(&width)
        && usize::from(count) == ports.len()
        && ports
            .iter()
            .all(|port| port.lanes.len() == usize::try_from(width).unwrap_or(usize::MAX))
}

#[cfg(test)]
mod tests {
    use hex_core::{BiomeRegionId, MapViewHint};

    use super::*;
    use crate::procedural_v3::layout::{
        HexSide, ResolvedEdgeReference, ResolvedElevationBand, ResolvedLayoutPlan,
        ResolvedLiquidPort, ResolvedPatch, ResolvedWalkerPorts,
    };
    use crate::procedural_v3::liquid::LiquidPlan;
    use crate::procedural_v3::world::{FeaturePlan, InteriorPlan, StructurePlan};

    fn empty_seam_world() -> GeneratedWorldPlan {
        let coord = HexCoord::ORIGIN;
        let footprint = BTreeSet::from([coord]);
        let edges = HexSide::ALL
            .into_iter()
            .map(|side| (side, ResolvedEdgeReference::WorldBoundary))
            .collect();
        GeneratedWorldPlan {
            source_schematic_fingerprint: None,
            layout: ResolvedLayoutPlan {
                kind: LayoutKind::Single,
                grid_radius: 12,
                footprint: footprint.clone(),
                patches: BTreeMap::from([(
                    PatchId(0),
                    ResolvedPatch {
                        biome_region: BiomeRegionId(0),
                        rotation_turns: 0,
                        mask: footprint.clone(),
                        edges,
                    },
                )]),
                shared_edges: BTreeMap::new(),
                boundary_liquid_outlets: BTreeMap::new(),
            },
            volume: VolumePlan::new(footprint),
            liquids: LiquidPlan::default(),
            features: FeaturePlan::default(),
            structures: StructurePlan::default(),
            blockers: BTreeSet::new(),
            lights: BTreeMap::new(),
            biome_regions: BTreeMap::new(),
            interiors: InteriorPlan::default(),
            anchors: BTreeMap::new(),
            observation_anchors: BTreeMap::new(),
            view_hint: MapViewHint::new((1.0, 4.0, 2.0), (0.0, 0.0, 0.0)),
        }
    }

    #[test]
    fn walker_contract_accepts_exact_explicit_width_four_without_loosening_legacy_bounds() {
        let lanes = [
            (HexCoord::from_axial(0, 0), HexCoord::from_axial(1, 0)),
            (HexCoord::from_axial(0, 1), HexCoord::from_axial(1, 1)),
            (HexCoord::from_axial(0, 2), HexCoord::from_axial(1, 2)),
            (HexCoord::from_axial(0, 3), HexCoord::from_axial(1, 3)),
        ]
        .into_iter()
        .collect();
        let port = super::super::layout::ResolvedPort {
            lanes,
            first_approach: BTreeSet::new(),
            second_approach: BTreeSet::new(),
        };
        assert!(valid_walker_contract(1, 4, std::slice::from_ref(&port)));
        assert!(!valid_walker_contract(1, 3, std::slice::from_ref(&port)));
        assert!(!valid_walker_contract(1, 5, std::slice::from_ref(&port)));
        assert!(valid_walker_contract(0, 0, &[]));
    }

    #[test]
    fn seam_closure_regions_preserve_legacy_ids_and_fit_ring19_local_bits() {
        let edge = ResolvedEdgeId(3);
        let single = seam_closure_region(LayoutKind::Single, edge);
        let ring7 = seam_closure_region(LayoutKind::Ring7, edge);
        let ring19 = seam_closure_region(LayoutKind::Ring19, edge);

        assert_eq!(single, SpecialMovementRegion(0x0ffe_0003));
        assert_eq!(ring7, single);
        assert_eq!(ring19, SpecialMovementRegion(0x07fe_0003));
        assert!(ring19.0 <= 0x07ff_ffff);
        assert!(is_seam_closure_access(SurfaceAccess::SpecialMovement(
            single
        )));
        assert!(is_seam_closure_access(SurfaceAccess::SpecialMovement(
            ring19
        )));
    }

    #[test]
    fn world_walker_validation_skips_only_an_empty_contract_set() {
        let mut plan = empty_seam_world();
        let mut issues = Vec::new();
        validate_world_walker_seams(&plan, &mut issues);
        assert!(issues.is_empty());

        plan.layout.shared_edges.insert(
            ResolvedEdgeId(0),
            ResolvedEdgeContract {
                first: (PatchId(0), HexSide::East),
                second: (PatchId(0), HexSide::West),
                elevation: ResolvedElevationBand {
                    preferred: 0,
                    min: 0,
                    max: 0,
                },
                walker: ResolvedWalkerPorts {
                    count: 1,
                    width: 1,
                    ports: Vec::new(),
                },
                liquid: ResolvedLiquidPort::Dry,
                approach_depth: 1,
                boundary_pairs: BTreeSet::new(),
                protected_approaches: BTreeMap::new(),
            },
        );
        validate_world_walker_seams(&plan, &mut issues);
        assert_eq!(issues.len(), 1);
        assert!(issues
            .first()
            .is_some_and(|issue| issue.detail.contains("exact declared walker-port width")));
    }
}
