//! Resolved V3 patch masks and shared edge contracts.
//!
//! Settings describe each side from a designer's perspective. Resolution assigns
//! stable patch and edge identities, creates exact disjoint masks, and stores each
//! internal seam once so neighboring recipes cannot generate competing borders.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::fmt;

use hex_core::{BiomeRegionId, HexCoord, Level};

use crate::settings::{
    ordered_simple_seam_lanes, ring19_region_coord, seam_approaches_are_independent,
    EdgeElevationSettings, EdgeLiquidPortSettings, EdgeLiquidSettings, MacroAccessSettings,
    MacroAxisSettings, MacroBiomeInstanceSettings, MacroBoundarySideSettings, MacroLayoutSettings,
    MacroLiquidConnectionSettings, MacroSpanningFeatureSettings, PatchEdgeContractSettings,
    PatchEdgesSettings, PatchMaskSettings, PatchSpec, ProceduralV3Settings, Ring19BoundarySide,
    SharedEdgeSettings, V3LayoutSettings, V3RecipeSettings, V3Ring19Settings, V3Ring7Settings,
    WalkerPortSettings, MAX_SEAM_PORT_WIDTH, MAX_V3_LEVEL, MAX_WALKER_PORT_COUNT,
    V3_MACRO_CELL_COUNT, V3_RING19_REGION_COUNT, V3_SCHEMATIC_GRID_RADIUS,
};

const RING_RADIUS: u32 = 33;
const RING_PATCH_OFFSET: i32 = 22;
const RING19_RADIUS: u32 = 55;
const MACRO_RADIUS: u32 = 77;
const MACRO_CELL_RADIUS: u32 = 3;
const MACRO_CELL_OFFSET: i32 = 22;
const CRYSTAL_ASCENT_SITE_RADIUS: u32 = 32;
pub(crate) const RING19_LOCAL_FRAME_SCALE: u32 = 12;

/// Stable complete-world layout family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LayoutKind {
    Single,
    Ring7,
    Ring19,
    Macro,
    /// One continuous world whose patches retain only coarse biome ownership.
    Schematic,
}

impl LayoutKind {
    /// Whether this layout combines more than one stable patch namespace.
    #[must_use]
    pub(crate) const fn is_composite(self) -> bool {
        matches!(
            self,
            Self::Ring7 | Self::Ring19 | Self::Macro | Self::Schematic
        )
    }
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
    /// Authored number of distinct ordinary-walker apertures.
    pub(crate) count: u8,
    /// Authored uniform width of every ordinary-walker aperture.
    pub(crate) width: u32,
    pub(crate) ports: Vec<ResolvedPort>,
}

/// One directed hydrology relationship, stored once for both patches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResolvedLiquidElevation {
    /// Legacy Single/Ring7 behavior: liquid may use the shared walker elevation band.
    EdgeBand,
    /// Ring19 behavior: both sides of the liquid seam use one exact level.
    Exact(Level),
}

/// One directed hydrology relationship, stored once for both patches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ResolvedLiquidPort {
    Dry,
    /// One exact still-water continuation with no downstream relationship.
    Standing {
        port: ResolvedPort,
        elevation: ResolvedLiquidElevation,
    },
    Directed {
        source: PatchId,
        sink: PatchId,
        port: ResolvedPort,
        elevation: ResolvedLiquidElevation,
    },
}

/// One exact directed liquid exit through the complete world's outer boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedBoundaryLiquidOutlet {
    pub(crate) source: PatchId,
    pub(crate) side: HexSide,
    /// Exact inside-to-outside lane pairs.
    pub(crate) lanes: BTreeSet<(HexCoord, HexCoord)>,
    /// Exact protected cells leading inward from the boundary lanes.
    pub(crate) inward_approach: BTreeSet<HexCoord>,
    pub(crate) approach_depth: u32,
    pub(crate) level: Level,
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
    /// Clockwise local-frame turns. Single and Ring7 always resolve to zero.
    pub(crate) rotation_turns: u8,
    pub(crate) mask: BTreeSet<HexCoord>,
    pub(crate) edges: BTreeMap<HexSide, ResolvedEdgeReference>,
}

/// Exact resolved vocabulary owned by Macro extensions rather than patch recipes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedMacroContracts {
    pub(crate) walker_connections: Vec<ResolvedMacroWalkerConnection>,
    pub(crate) spanning_features: BTreeMap<String, ResolvedMacroSpanningFeature>,
    pub(crate) anchor_aliases: BTreeMap<String, ResolvedMacroAnchorReference>,
}

/// One explicit surface aperture oriented in authored instance order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedMacroWalkerConnection {
    pub(crate) first: PatchId,
    pub(crate) second: PatchId,
    pub(crate) level: Level,
    pub(crate) port: ResolvedPort,
}

/// One exact whole-world feature after instance and seam resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ResolvedMacroSpanningFeature {
    Tunnel(ResolvedMacroTunnel),
}

/// One exact level tunnel from an outer-world terminal through ordered instance seams.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedMacroTunnel {
    pub(crate) name: String,
    pub(crate) canonical_route: bool,
    pub(crate) instance_route: Vec<PatchId>,
    pub(crate) boundary_terminal: ResolvedMacroBoundaryTerminal,
    pub(crate) destination_anchor: ResolvedMacroAnchorReference,
    pub(crate) floor_level: Level,
    pub(crate) width: u32,
    pub(crate) clearance: u32,
    pub(crate) roof_thickness: u32,
    /// Ordered exact seam apertures, one fewer than `instance_route`.
    pub(crate) seams: Vec<ResolvedMacroSubsurfaceSeam>,
}

/// One exact world-boundary aperture oriented inside-to-outside.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedMacroBoundaryTerminal {
    pub(crate) instance: PatchId,
    pub(crate) side: HexSide,
    pub(crate) lanes: BTreeSet<(HexCoord, HexCoord)>,
    pub(crate) inward_approach: BTreeSet<HexCoord>,
}

/// One exact tunnel seam oriented from the preceding route instance to the next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedMacroSubsurfaceSeam {
    pub(crate) source: PatchId,
    pub(crate) destination: PatchId,
    pub(crate) port: ResolvedPort,
}

/// One instance-local anchor reference after logical-name resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedMacroAnchorReference {
    pub(crate) instance: PatchId,
    pub(crate) anchor: String,
}

/// Complete mask and seam topology consumed by candidate recipes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedLayoutPlan {
    pub(crate) kind: LayoutKind,
    pub(crate) grid_radius: u32,
    pub(crate) footprint: BTreeSet<HexCoord>,
    pub(crate) patches: BTreeMap<PatchId, ResolvedPatch>,
    pub(crate) shared_edges: BTreeMap<ResolvedEdgeId, ResolvedEdgeContract>,
    pub(crate) boundary_liquid_outlets: BTreeMap<(PatchId, HexSide), ResolvedBoundaryLiquidOutlet>,
}

impl ResolvedLayoutPlan {
    /// Rechecks exact coverage, connectivity, references, and shared seam geometry.
    pub(crate) fn validate(&self) -> Result<(), LayoutValidationError> {
        let mut issues = Vec::new();
        let radius_valid = match self.kind {
            LayoutKind::Single => (12..=40).contains(&self.grid_radius),
            LayoutKind::Ring7 => self.grid_radius == RING_RADIUS,
            LayoutKind::Ring19 => self.grid_radius == RING19_RADIUS,
            LayoutKind::Macro => self.grid_radius == MACRO_RADIUS,
            LayoutKind::Schematic => self.grid_radius == V3_SCHEMATIC_GRID_RADIUS,
        };
        if !radius_valid {
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
            LayoutKind::Ring19 => 19,
            LayoutKind::Macro => self.patches.len(),
            LayoutKind::Schematic => hex_schematic::SCHEMATIC_CELL_COUNT,
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
        if self.kind == LayoutKind::Ring19
            && (self.grid_radius != RING19_RADIUS
                || self.footprint
                    != HexCoord::ORIGIN
                        .within_radius(RING19_RADIUS)
                        .into_iter()
                        .collect())
        {
            issues.push(LayoutIssue::InvalidRingFootprint);
        }
        if self.kind == LayoutKind::Macro
            && (self.grid_radius != MACRO_RADIUS
                || self.footprint
                    != HexCoord::ORIGIN
                        .within_radius(MACRO_RADIUS)
                        .into_iter()
                        .collect())
        {
            issues.push(LayoutIssue::InvalidMacroFootprint);
        }
        if self.kind == LayoutKind::Schematic
            && (self.grid_radius != V3_SCHEMATIC_GRID_RADIUS
                || self.footprint
                    != HexCoord::ORIGIN
                        .within_radius(V3_SCHEMATIC_GRID_RADIUS)
                        .into_iter()
                        .collect())
        {
            issues.push(LayoutIssue::InvalidRingFootprint);
        }
        if self.kind == LayoutKind::Macro
            && (self.patches.is_empty() || self.patches.len() > V3_MACRO_CELL_COUNT)
        {
            issues.push(LayoutIssue::InvalidMacroPatchCount(self.patches.len()));
        }
        if self.kind == LayoutKind::Ring19 && self.shared_edges.len() != 42 {
            issues.push(LayoutIssue::SharedEdgeCount {
                expected: 42,
                actual: self.shared_edges.len(),
            });
        }
        if self.kind == LayoutKind::Ring19 {
            let boundary_sides = self
                .patches
                .values()
                .flat_map(|patch| patch.edges.values())
                .filter(|reference| matches!(reference, ResolvedEdgeReference::WorldBoundary))
                .count();
            if boundary_sides != 30 {
                issues.push(LayoutIssue::BoundarySideCount {
                    expected: 30,
                    actual: boundary_sides,
                });
            }
        }

        // Coverage is queried only for membership/cardinality while patches and
        // masks themselves retain canonical BTree iteration. A hash index avoids
        // O(log n) insertion for every one of Grand V3's 105,469 columns without
        // changing which ordered patch/coordinate reports an overlap.
        let mut covered = HashSet::with_capacity(self.footprint.len());
        let mut biome_regions = BTreeSet::new();
        for (id, patch) in &self.patches {
            if self.kind == LayoutKind::Ring19
                && (usize::try_from(id.0).map_or(true, |id| id >= V3_RING19_REGION_COUNT)
                    || patch.biome_region.0 != id.0)
            {
                issues.push(LayoutIssue::InvalidRing19PatchIdentity(
                    *id,
                    patch.biome_region,
                ));
            }
            if self.kind == LayoutKind::Schematic
                && (id.0 != patch.biome_region.0
                    || usize::try_from(id.0)
                        .map_or(true, |id| id >= hex_schematic::SCHEMATIC_CELL_COUNT))
            {
                issues.push(LayoutIssue::InvalidRing19PatchIdentity(
                    *id,
                    patch.biome_region,
                ));
            }
            if patch.rotation_turns > 5
                || (!matches!(
                    self.kind,
                    LayoutKind::Ring19 | LayoutKind::Macro | LayoutKind::Schematic
                ) && patch.rotation_turns != 0)
            {
                issues.push(LayoutIssue::InvalidPatchRotation(*id, patch.rotation_turns));
            }
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
            if self.kind != LayoutKind::Schematic
                && (patch.edges.len() != HexSide::ALL.len()
                    || HexSide::ALL
                        .iter()
                        .any(|side| !patch.edges.contains_key(side)))
            {
                issues.push(LayoutIssue::IncompletePatchEdges(*id));
            }
            if self.kind == LayoutKind::Schematic && !patch.edges.is_empty() {
                issues.push(LayoutIssue::IncompletePatchEdges(*id));
            }
        }
        if covered.len() != self.footprint.len()
            || self.footprint.iter().any(|coord| !covered.contains(coord))
        {
            issues.push(LayoutIssue::IncompleteCoverage);
        }

        let mut references = BTreeMap::<ResolvedEdgeId, Vec<(PatchId, HexSide)>>::new();
        if self.kind != LayoutKind::Macro {
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
        }
        for (edge_id, edge) in &self.shared_edges {
            if self.kind != LayoutKind::Macro {
                let mut expected = vec![edge.first, edge.second];
                expected.sort_unstable();
                let mut actual = references.remove(edge_id).unwrap_or_default();
                actual.sort_unstable();
                if actual != expected {
                    issues.push(LayoutIssue::SharedReferenceMismatch(*edge_id));
                }
            }
            validate_resolved_edge(
                self.kind,
                *edge_id,
                edge,
                &self.patches,
                &self.footprint,
                &mut issues,
            );
        }
        if self.kind == LayoutKind::Macro {
            validate_macro_edge_coverage(self, &mut issues);
        }
        validate_boundary_liquid_outlets(self, &mut issues);
        validate_liquid_graph(self, &mut issues);
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
        V3LayoutSettings::Ring19(ring) => resolve_ring19(grid_radius, ring)?,
        V3LayoutSettings::Macro(macro_layout) => resolve_macro(grid_radius, macro_layout)?,
        V3LayoutSettings::Schematic(schematic) => resolve_schematic(grid_radius, schematic)?,
    };
    resolved.validate()?;
    Ok(resolved)
}

fn resolve_schematic(
    grid_radius: u32,
    settings: &crate::settings::V3SchematicLayoutSettings,
) -> Result<ResolvedLayoutPlan, LayoutValidationError> {
    if grid_radius != V3_SCHEMATIC_GRID_RADIUS {
        return Err(LayoutValidationError::one(LayoutIssue::UnsupportedRadius(
            grid_radius,
        )));
    }
    let pitch = i32::try_from(settings.cell_pitch).map_err(|_error| {
        LayoutValidationError::one(LayoutIssue::UnsupportedRadius(grid_radius))
    })?;
    let footprint: BTreeSet<_> = HexCoord::ORIGIN
        .within_radius(grid_radius)
        .into_iter()
        .collect();
    let coarse = hex_schematic::canonical_coordinates()
        .into_iter()
        .enumerate()
        .map(|(index, coord)| {
            let coarse_coord = HexCoord::from_axial(coord.q(), coord.r());
            let center = HexCoord::from_axial(coord.q() * pitch, coord.r() * pitch);
            (
                PatchId(u32::try_from(index).unwrap_or(u32::MAX)),
                coarse_coord,
                center,
            )
        })
        .collect::<Vec<_>>();
    let coarse_by_coord = coarse
        .iter()
        .map(|(id, coarse_coord, center)| (*coarse_coord, (*id, *center)))
        .collect::<BTreeMap<_, _>>();
    let candidate_offsets = HexCoord::ORIGIN.within_radius(2);
    let mut masks = coarse
        .iter()
        .map(|(id, _, _)| (*id, BTreeSet::new()))
        .collect::<BTreeMap<_, _>>();
    for coord in &footprint {
        let owner = nearest_schematic_owner(*coord, pitch, &coarse_by_coord, &candidate_offsets)
            // The radius-two candidate proof is exhaustively pinned below. Keep
            // the full ordered fallback so malformed future constants fail
            // safely instead of leaving a hole in ownership.
            .or_else(|| {
                coarse
                    .iter()
                    .min_by_key(|(id, _, center)| (center.distance(*coord), id.0))
                    .map(|(id, _, _)| *id)
            })
            .ok_or_else(|| LayoutValidationError::one(LayoutIssue::InvalidFootprint))?;
        masks.entry(owner).or_default().insert(*coord);
    }
    let patches = masks
        .into_iter()
        .map(|(id, mask)| {
            (
                id,
                ResolvedPatch {
                    biome_region: BiomeRegionId(id.0),
                    rotation_turns: 0,
                    mask,
                    edges: BTreeMap::new(),
                },
            )
        })
        .collect();
    Ok(ResolvedLayoutPlan {
        kind: LayoutKind::Schematic,
        grid_radius,
        footprint,
        patches,
        shared_edges: BTreeMap::new(),
        boundary_liquid_outlets: BTreeMap::new(),
    })
}

fn nearest_schematic_owner(
    coord: HexCoord,
    pitch: i32,
    coarse_by_coord: &BTreeMap<HexCoord, (PatchId, HexCoord)>,
    candidate_offsets: &[HexCoord],
) -> Option<PatchId> {
    let base = HexCoord::from_axial(coord.x().div_euclid(pitch), coord.y().div_euclid(pitch));
    candidate_offsets
        .iter()
        .filter_map(|offset| {
            let candidate = HexCoord::from_axial(
                base.x().checked_add(offset.x())?,
                base.y().checked_add(offset.y())?,
            );
            coarse_by_coord.get(&candidate).copied()
        })
        .min_by_key(|(id, center)| (center.distance(coord), id.0))
        .map(|(id, _)| id)
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
            rotation_turns: 0,
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
        boundary_liquid_outlets: BTreeMap::new(),
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
                rotation_turns: 0,
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
        boundary_liquid_outlets: BTreeMap::new(),
    })
}

fn resolve_ring19(
    grid_radius: u32,
    ring: &V3Ring19Settings,
) -> Result<ResolvedLayoutPlan, LayoutValidationError> {
    if grid_radius != RING19_RADIUS {
        return Err(LayoutValidationError::one(LayoutIssue::InvalidRingRadius(
            grid_radius,
        )));
    }
    if ring.regions.len() != V3_RING19_REGION_COUNT {
        return Err(LayoutValidationError::one(LayoutIssue::PatchCount {
            expected: V3_RING19_REGION_COUNT,
            actual: ring.regions.len(),
        }));
    }
    if !matches!(ring.seam_defaults.liquid, EdgeLiquidSettings::Dry) {
        return Err(LayoutValidationError::one(
            LayoutIssue::InvalidRing19Settings("seam_defaults.liquid must remain Dry".to_owned()),
        ));
    }

    let footprint = HexCoord::ORIGIN
        .within_radius(RING19_RADIUS)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let macro_centers = ring19_macro_centers();
    let centers = scaled_centers(&macro_centers, RING_PATCH_OFFSET);
    let masks = generated_nearest_masks(&footprint, &centers);
    let seams = ring19_seams(&macro_centers);
    if seams.len() != 42 {
        return Err(LayoutValidationError::one(
            LayoutIssue::InvalidRing19Settings(format!(
                "fixed radius-two macro layout resolved {} seams instead of 42",
                seams.len()
            )),
        ));
    }

    let mut patches = BTreeMap::new();
    for id in centers.keys().copied() {
        let edges = HexSide::ALL
            .into_iter()
            .map(|side| (side, ResolvedEdgeReference::WorldBoundary))
            .collect();
        patches.insert(
            id,
            ResolvedPatch {
                biome_region: BiomeRegionId(id.0),
                rotation_turns: ring
                    .regions
                    .get(usize::try_from(id.0).unwrap_or(usize::MAX))
                    .map_or(0, |region| region.rotation_turns),
                mask: masks.get(&id).cloned().unwrap_or_default(),
                edges,
            },
        );
    }

    let seam_keys = seams
        .iter()
        .map(|(first, _, second, _)| ordered_patch_pair(*first, *second))
        .collect::<BTreeSet<_>>();
    let mut liquid_connections = BTreeMap::new();
    for connection in &ring.liquid_connections {
        let source = PatchId(u32::from(connection.source_region));
        let sink = PatchId(u32::from(connection.sink_region));
        let key = ordered_patch_pair(source, sink);
        if source == sink
            || !patches.contains_key(&source)
            || !patches.contains_key(&sink)
            || !seam_keys.contains(&key)
        {
            return Err(LayoutValidationError::one(
                LayoutIssue::InvalidRing19Settings(format!(
                    "liquid connection {} -> {} does not name one internal seam",
                    connection.source_region, connection.sink_region
                )),
            ));
        }
        if liquid_connections.insert(key, connection).is_some() {
            return Err(LayoutValidationError::one(
                LayoutIssue::InvalidRing19Settings(format!(
                    "internal seam {} <-> {} has more than one liquid connection",
                    key.0 .0, key.1 .0
                )),
            ));
        }
    }

    let mut shared_edges = BTreeMap::new();
    for (index, (first_id, first_side, second_id, second_side)) in seams.into_iter().enumerate() {
        let edge_id = ResolvedEdgeId(u32::try_from(index).unwrap_or(u32::MAX));
        let key = ordered_patch_pair(first_id, second_id);
        let connection = liquid_connections.get(&key).copied();
        let mut first_settings = ring.seam_defaults.clone();
        let mut second_settings = ring.seam_defaults.clone();
        if let Some(connection) = connection {
            let port = EdgeLiquidPortSettings {
                width: connection.width,
            };
            if PatchId(u32::from(connection.source_region)) == first_id {
                first_settings.liquid = EdgeLiquidSettings::Outlet(port);
                second_settings.liquid = EdgeLiquidSettings::Inlet(port);
            } else {
                first_settings.liquid = EdgeLiquidSettings::Inlet(port);
                second_settings.liquid = EdgeLiquidSettings::Outlet(port);
            }
        }
        let first_authored = PatchEdgeContractSettings::Shared(first_settings);
        let second_authored = PatchEdgeContractSettings::Shared(second_settings);
        let mut contract = resolve_shared_edge(
            first_id,
            first_side,
            &first_authored,
            second_id,
            second_side,
            &second_authored,
            &masks,
        )?;
        if let (Some(connection), ResolvedLiquidPort::Directed { elevation, .. }) =
            (connection, &mut contract.liquid)
        {
            *elevation = ResolvedLiquidElevation::Exact(connection.level);
        }
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

    let mut reserved = BTreeMap::<PatchId, BTreeSet<HexCoord>>::new();
    for edge in shared_edges.values() {
        for (patch, approach) in &edge.protected_approaches {
            reserved
                .entry(*patch)
                .or_default()
                .extend(approach.iter().copied());
        }
    }
    let mut boundary_liquid_outlets = BTreeMap::new();
    let mut authored_outlets = ring.boundary_outlets.iter().collect::<Vec<_>>();
    authored_outlets.sort_unstable();
    for authored in authored_outlets {
        let source = PatchId(u32::from(authored.source_region));
        let side = ring19_boundary_side(authored.side);
        let Some(patch) = patches.get(&source) else {
            return Err(LayoutValidationError::one(
                LayoutIssue::InvalidBoundaryLiquidOutlet(source, side),
            ));
        };
        if !matches!(
            patch.edges.get(&side),
            Some(ResolvedEdgeReference::WorldBoundary)
        ) || boundary_liquid_outlets.contains_key(&(source, side))
        {
            return Err(LayoutValidationError::one(
                LayoutIssue::InvalidBoundaryLiquidOutlet(source, side),
            ));
        }
        let Some(center) = centers.get(&source).copied() else {
            return Err(LayoutValidationError::one(
                LayoutIssue::InvalidBoundaryLiquidOutlet(source, side),
            ));
        };
        let outlet = resolve_boundary_liquid_outlet(
            source,
            side,
            authored.width,
            authored.level,
            ring.seam_defaults.approach_depth,
            center,
            &patch.mask,
            &footprint,
            reserved.get(&source),
        )?;
        reserved
            .entry(source)
            .or_default()
            .extend(outlet.inward_approach.iter().copied());
        boundary_liquid_outlets.insert((source, side), outlet);
    }

    Ok(ResolvedLayoutPlan {
        kind: LayoutKind::Ring19,
        grid_radius,
        footprint,
        patches,
        shared_edges,
        boundary_liquid_outlets,
    })
}

fn resolve_macro(
    grid_radius: u32,
    settings: &MacroLayoutSettings,
) -> Result<ResolvedLayoutPlan, LayoutValidationError> {
    if grid_radius != MACRO_RADIUS || settings.macro_radius != MACRO_CELL_RADIUS {
        return Err(LayoutValidationError::one(LayoutIssue::InvalidMacroRadius(
            grid_radius,
        )));
    }

    let footprint = HexCoord::ORIGIN
        .within_radius(MACRO_RADIUS)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let macro_coords = HexCoord::ORIGIN
        .within_radius(MACRO_CELL_RADIUS)
        .into_iter()
        .collect::<BTreeSet<_>>();
    if macro_coords.len() != V3_MACRO_CELL_COUNT {
        return Err(LayoutValidationError::one(
            LayoutIssue::InvalidMacroAtomicGeometry,
        ));
    }
    let raw_adjacencies = macro_coords
        .iter()
        .map(|coord| {
            coord
                .neighbors()
                .into_iter()
                .filter(|neighbor| macro_coords.contains(neighbor))
                .count()
        })
        .sum::<usize>()
        / 2;
    let outer_sides = macro_coords
        .iter()
        .flat_map(|coord| coord.neighbors())
        .filter(|neighbor| !macro_coords.contains(neighbor))
        .count();
    if raw_adjacencies != 90 || outer_sides != 42 {
        return Err(LayoutValidationError::one(
            LayoutIssue::InvalidMacroAtomicGeometry,
        ));
    }

    let atomic_ids = macro_coords
        .iter()
        .copied()
        .enumerate()
        .map(|(index, coord)| (PatchId(u32::try_from(index).unwrap_or(u32::MAX)), coord))
        .collect::<BTreeMap<_, _>>();
    let atomic_centers = scaled_centers(&atomic_ids, MACRO_CELL_OFFSET);
    let atomic_masks = generated_nearest_masks(&footprint, &atomic_centers);
    let atomic_id_by_coord = atomic_ids
        .iter()
        .map(|(id, coord)| (*coord, *id))
        .collect::<BTreeMap<_, _>>();

    let mut masks = BTreeMap::<PatchId, BTreeSet<HexCoord>>::new();
    let mut patches = BTreeMap::new();
    for (index, instance) in settings.instances.iter().enumerate() {
        let id = PatchId(u32::try_from(index).unwrap_or(u32::MAX));
        let mut mask = BTreeSet::new();
        for raw in &instance.cells {
            let Some(coord) = HexCoord::try_new_cubic(raw.x, raw.y, raw.z) else {
                return Err(LayoutValidationError::one(LayoutIssue::InvalidCube(
                    raw.x, raw.y, raw.z,
                )));
            };
            let Some(atomic_id) = atomic_id_by_coord.get(&coord) else {
                return Err(LayoutValidationError::one(
                    LayoutIssue::InvalidMacroAtomicCell(coord),
                ));
            };
            let Some(atomic_mask) = atomic_masks.get(atomic_id) else {
                return Err(LayoutValidationError::one(
                    LayoutIssue::InvalidMacroAtomicCell(coord),
                ));
            };
            mask.extend(atomic_mask.iter().copied());
        }
        let edges = HexSide::ALL
            .into_iter()
            .map(|side| (side, ResolvedEdgeReference::WorldBoundary))
            .collect();
        masks.insert(id, mask.clone());
        patches.insert(
            id,
            ResolvedPatch {
                biome_region: BiomeRegionId(id.0),
                rotation_turns: instance.rotation_turns,
                mask,
                edges,
            },
        );
    }

    claim_macro_landmark_masks(settings, &footprint, &mut masks, &mut patches)?;

    let named_ids = settings
        .instances
        .iter()
        .enumerate()
        .map(|(index, instance)| {
            (
                instance.name.as_str(),
                PatchId(u32::try_from(index).unwrap_or(u32::MAX)),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut liquids = BTreeMap::new();
    for liquid in &settings.liquid_connections {
        let (first, second) = match liquid {
            MacroLiquidConnectionSettings::Standing {
                first_instance,
                second_instance,
                ..
            } => (first_instance.as_str(), second_instance.as_str()),
            MacroLiquidConnectionSettings::Directed {
                source_instance,
                sink_instance,
                ..
            } => (source_instance.as_str(), sink_instance.as_str()),
        };
        let (Some(first), Some(second)) = (named_ids.get(first), named_ids.get(second)) else {
            return Err(LayoutValidationError::one(
                LayoutIssue::InvalidMacroSettings(
                    "liquid connection references an unknown instance".to_owned(),
                ),
            ));
        };
        liquids.insert(ordered_patch_pair(*first, *second), liquid);
    }
    let critical_walker_pairs = settings
        .critical_route
        .windows(2)
        .filter_map(|pair| {
            let first = named_ids.get(pair.first()?.as_str())?;
            let second = named_ids.get(pair.get(1)?.as_str())?;
            Some(ordered_patch_pair(*first, *second))
        })
        .collect::<BTreeSet<_>>();
    let explicit_walker_pairs = settings
        .walker_connections
        .iter()
        .filter_map(|connection| {
            let first = named_ids.get(connection.first_instance.as_str())?;
            let second = named_ids.get(connection.second_instance.as_str())?;
            Some((
                ordered_patch_pair(*first, *second),
                (connection.width, connection.level),
            ))
        })
        .collect::<BTreeMap<_, _>>();

    let mut shared_edges = BTreeMap::new();
    for (first_index, first_instance) in settings.instances.iter().enumerate() {
        let first_id = PatchId(u32::try_from(first_index).unwrap_or(u32::MAX));
        for (second_index, second_instance) in settings
            .instances
            .iter()
            .enumerate()
            .skip(first_index.saturating_add(1))
        {
            let second_id = PatchId(u32::try_from(second_index).unwrap_or(u32::MAX));
            let all_pairs = boundary_pairs(
                masks.get(&first_id).unwrap_or(&BTreeSet::new()),
                masks.get(&second_id).unwrap_or(&BTreeSet::new()),
            );
            if all_pairs.is_empty() {
                continue;
            }
            let first_side = representative_boundary_side(&all_pairs).ok_or_else(|| {
                LayoutValidationError::one(LayoutIssue::InvalidMacroSettings(
                    "adjacent columns did not resolve to a hex side".to_owned(),
                ))
            })?;
            let second_side = first_side.opposite();
            let pair = ordered_patch_pair(first_id, second_id);
            let liquid = liquids.get(&pair).copied();
            let (walker, exact_walker_level) = explicit_walker_pairs.get(&pair).map_or_else(
                || {
                    let walker = if critical_walker_pairs.contains(&pair) {
                        WalkerPortSettings { count: 1, width: 2 }
                    } else {
                        WalkerPortSettings { count: 0, width: 0 }
                    };
                    (walker, None)
                },
                |(width, level)| {
                    (
                        WalkerPortSettings {
                            count: 1,
                            width: *width,
                        },
                        Some(*level),
                    )
                },
            );
            let (first_authored, second_authored, exact_liquid_level) = macro_shared_settings(
                first_id,
                first_side,
                first_instance,
                second_id,
                second_side,
                second_instance,
                settings.approach_depth,
                walker,
                exact_walker_level,
                liquid,
            );
            let mut contract = resolve_shared_edge_with_pairs(
                first_id,
                first_side,
                &first_authored,
                second_id,
                second_side,
                &second_authored,
                &masks,
                Some(&all_pairs),
            )?;
            if let Some(level) = exact_liquid_level {
                match &mut contract.liquid {
                    ResolvedLiquidPort::Standing { elevation, .. }
                    | ResolvedLiquidPort::Directed { elevation, .. } => {
                        *elevation = ResolvedLiquidElevation::Exact(level);
                    }
                    ResolvedLiquidPort::Dry => {}
                }
            }
            if let Some((width, level)) = explicit_walker_pairs.get(&pair) {
                align_macro_crystal_walker_port(
                    &mut contract,
                    first_id,
                    first_instance,
                    second_id,
                    second_instance,
                    *width,
                    *level,
                    &all_pairs,
                    &masks,
                    &footprint,
                    settings.approach_depth,
                )?;
            }
            let edge_id = ResolvedEdgeId(u32::try_from(shared_edges.len()).unwrap_or(u32::MAX));
            shared_edges.insert(edge_id, contract);
        }
    }

    let resolved = ResolvedLayoutPlan {
        kind: LayoutKind::Macro,
        grid_radius,
        footprint,
        patches,
        shared_edges,
        boundary_liquid_outlets: BTreeMap::new(),
    };
    resolve_macro_contracts(settings, &resolved)?;
    Ok(resolved)
}

#[expect(
    clippy::too_many_arguments,
    reason = "the exact authored port is resolved from both endpoint recipes and final masks"
)]
fn align_macro_crystal_walker_port(
    contract: &mut ResolvedEdgeContract,
    first_id: PatchId,
    first: &MacroBiomeInstanceSettings,
    second_id: PatchId,
    second: &MacroBiomeInstanceSettings,
    width: u32,
    level: Level,
    boundary_pairs: &BTreeSet<(HexCoord, HexCoord)>,
    masks: &BTreeMap<PatchId, BTreeSet<HexCoord>>,
    footprint: &BTreeSet<HexCoord>,
    approach_depth: u32,
) -> Result<(), LayoutValidationError> {
    let crystal = match (&first.recipe, &second.recipe) {
        (V3RecipeSettings::CrystalAscent(settings), _) => Some((first_id, first, settings, true)),
        (_, V3RecipeSettings::CrystalAscent(settings)) => {
            Some((second_id, second, settings, false))
        }
        _ => None,
    };
    let Some((crystal_id, instance, settings, crystal_is_first)) = crystal else {
        return Ok(());
    };
    let summit = settings
        .base_level
        .checked_add(settings.rise_levels)
        .ok_or_else(|| invalid_macro_contract("CrystalAscent summit level overflowed"))?;
    if level != summit {
        return Err(invalid_macro_contract(format!(
            "CrystalAscent explicit walker must meet its upper terminal at level {summit}"
        )));
    }
    let crystal_mask = masks.get(&crystal_id).ok_or_else(|| {
        invalid_macro_contract("CrystalAscent explicit walker has no resolved mask")
    })?;
    let terminal = super::crystal_ascent::macro_upper_terminal_coords(
        crystal_mask,
        instance.rotation_turns,
        summit,
    )
    .map_err(|error| {
        invalid_macro_contract(format!(
            "CrystalAscent upper-terminal projection failed: {error}"
        ))
    })?;
    let authored_approach = super::crystal_ascent::macro_upper_terminal_approach_coords(
        crystal_mask,
        instance.rotation_turns,
        summit,
        approach_depth,
    )
    .map_err(|error| {
        invalid_macro_contract(format!(
            "CrystalAscent upper-terminal approach projection failed: {error}"
        ))
    })?;
    let first_mask = masks.get(&first_id).ok_or_else(|| {
        invalid_macro_contract("explicit walker first instance has no resolved mask")
    })?;
    let second_mask = masks.get(&second_id).ok_or_else(|| {
        invalid_macro_contract("explicit walker second instance has no resolved mask")
    })?;
    let selected = macro_boundary_port_candidates(
        boundary_pairs,
        first_mask,
        second_mask,
        width,
        approach_depth,
        footprint,
    )
    .into_iter()
    .filter(|candidate| {
        let crystal_approach = if crystal_is_first {
            &candidate.port.first_approach
        } else {
            &candidate.port.second_approach
        };
        let crystal_lanes =
            candidate.port.lanes.iter().map(
                |(first, second)| {
                    if crystal_is_first {
                        *first
                    } else {
                        *second
                    }
                },
            );
        terminal.is_subset(crystal_approach)
            && crystal_lanes
                .into_iter()
                .all(|coord| authored_approach.contains(&coord))
            && liquid_port_ref(&contract.liquid)
                .is_none_or(|liquid| ports_are_disjoint(liquid, &candidate.port))
    })
    .map(|candidate| candidate.port)
    .min();
    let Some(mut port) = selected else {
        return Err(invalid_macro_contract(
            "CrystalAscent upper terminal cannot resolve one exact four-wide summit port",
        ));
    };
    if crystal_is_first {
        port.first_approach.clone_from(&authored_approach);
    } else {
        port.second_approach.clone_from(&authored_approach);
    }
    contract.walker.ports = vec![port];
    let mut first_approach = BTreeSet::new();
    let mut second_approach = BTreeSet::new();
    for port in contract
        .walker
        .ports
        .iter()
        .chain(liquid_port_ref(&contract.liquid))
    {
        first_approach.extend(port.first_approach.iter().copied());
        second_approach.extend(port.second_approach.iter().copied());
    }
    contract.protected_approaches =
        BTreeMap::from([(first_id, first_approach), (second_id, second_approach)]);
    Ok(())
}

/// Applies exact authored landmark claims before shared seams are constructed.
///
/// Atomic Macro cells remain the designer-facing ownership graph. Crystal Ascent's
/// authored radius-32 site is slightly larger than the nearest-center union of its
/// central seven cells, so the missing fringe columns are transferred from their
/// current owners here. Doing this before seam resolution keeps every boundary pair,
/// surface port, and subsurface port authoritative for the final masks.
fn claim_macro_landmark_masks(
    settings: &MacroLayoutSettings,
    footprint: &BTreeSet<HexCoord>,
    masks: &mut BTreeMap<PatchId, BTreeSet<HexCoord>>,
    patches: &mut BTreeMap<PatchId, ResolvedPatch>,
) -> Result<(), LayoutValidationError> {
    let crystal_instances = settings
        .instances
        .iter()
        .enumerate()
        .filter(|(_, instance)| matches!(instance.recipe, V3RecipeSettings::CrystalAscent(_)))
        .map(|(index, _)| PatchId(u32::try_from(index).unwrap_or(u32::MAX)))
        .collect::<Vec<_>>();
    let Some(crystal_id) = crystal_instances.first().copied() else {
        return Ok(());
    };
    if crystal_instances.len() != 1 {
        return Err(invalid_macro_contract(
            "Macro supports exactly one CrystalAscent landmark claim",
        ));
    }
    let central_seven = HexCoord::ORIGIN
        .within_radius(1)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let authored_cells = settings
        .instances
        .get(usize::try_from(crystal_id.0).unwrap_or(usize::MAX))
        .map(|instance| {
            instance
                .cells
                .iter()
                .filter_map(|cell| HexCoord::try_new_cubic(cell.x, cell.y, cell.z))
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    if authored_cells != central_seven {
        return Err(invalid_macro_contract(
            "CrystalAscent must own exactly Macro's central seven atomic cells",
        ));
    }

    let claim = HexCoord::ORIGIN
        .within_radius(CRYSTAL_ASCENT_SITE_RADIUS)
        .into_iter()
        .collect::<BTreeSet<_>>();
    if !claim.is_subset(footprint) {
        return Err(invalid_macro_contract(
            "CrystalAscent radius-32 claim falls outside the Macro footprint",
        ));
    }

    let mut owner_by_coord = masks
        .iter()
        .flat_map(|(id, mask)| mask.iter().map(move |coord| (*coord, *id)))
        .collect::<BTreeMap<_, _>>();
    for coord in claim {
        let Some(owner) = owner_by_coord.get(&coord).copied() else {
            return Err(invalid_macro_contract(format!(
                "CrystalAscent claim column {coord:?} has no Macro owner"
            )));
        };
        if owner == crystal_id {
            continue;
        }
        let removed = masks
            .get_mut(&owner)
            .is_some_and(|mask| mask.remove(&coord));
        if !removed {
            return Err(invalid_macro_contract(format!(
                "CrystalAscent claim column {coord:?} could not be removed from {owner:?}"
            )));
        }
        masks.entry(crystal_id).or_default().insert(coord);
        owner_by_coord.insert(coord, crystal_id);
    }

    let mut covered = BTreeSet::new();
    for (id, mask) in &*masks {
        if mask.is_empty() || !connected(mask) {
            return Err(invalid_macro_contract(format!(
                "CrystalAscent radius-32 claim disconnects or empties donor patch {id:?}"
            )));
        }
        for coord in mask {
            if !covered.insert(*coord) {
                return Err(invalid_macro_contract(format!(
                    "CrystalAscent radius-32 claim overlaps column {coord:?}"
                )));
            }
        }
    }
    if covered != *footprint {
        return Err(invalid_macro_contract(
            "CrystalAscent radius-32 claim does not preserve complete Macro coverage",
        ));
    }

    for (id, mask) in &*masks {
        let patch = patches.get_mut(id).ok_or_else(|| {
            invalid_macro_contract(format!(
                "CrystalAscent radius-32 claim has no resolved patch for {id:?}"
            ))
        })?;
        patch.mask.clone_from(mask);
    }
    Ok(())
}

/// Resolves Macro-only surface, subsurface, and anchor contracts against current masks.
///
/// This remains separate from `ResolvedLayoutPlan` so a composed landmark may adjust
/// exact mask ownership and then resolve the same authored contracts again without
/// reconstructing seam geometry in the composition layer.
pub(crate) fn resolve_macro_contracts(
    settings: &MacroLayoutSettings,
    layout: &ResolvedLayoutPlan,
) -> Result<ResolvedMacroContracts, LayoutValidationError> {
    if layout.kind != LayoutKind::Macro {
        return Err(invalid_macro_contract(
            "Macro contracts require a resolved Macro layout",
        ));
    }
    let named_ids = settings
        .instances
        .iter()
        .enumerate()
        .map(|(index, instance)| {
            (
                instance.name.as_str(),
                PatchId(u32::try_from(index).unwrap_or(u32::MAX)),
            )
        })
        .collect::<BTreeMap<_, _>>();

    let mut walker_settings = settings.walker_connections.clone();
    walker_settings.sort_unstable();
    let mut walker_connections = Vec::with_capacity(walker_settings.len());
    for connection in walker_settings {
        let authored_first =
            macro_named_patch(&named_ids, &connection.first_instance, "walker connection")?;
        let authored_second =
            macro_named_patch(&named_ids, &connection.second_instance, "walker connection")?;
        let (first, second) = ordered_patch_pair(authored_first, authored_second);
        let port = resolved_explicit_macro_walker_port(
            layout,
            first,
            second,
            connection.width,
            connection.level,
        )?;
        walker_connections.push(ResolvedMacroWalkerConnection {
            first,
            second,
            level: connection.level,
            port,
        });
    }

    let mut feature_settings = settings.spanning_features.clone();
    feature_settings.sort_unstable();
    let mut spanning_features = BTreeMap::new();
    for feature in feature_settings {
        let MacroSpanningFeatureSettings::Tunnel(tunnel) = feature;
        let boundary_instance = macro_named_patch(
            &named_ids,
            &tunnel.boundary_terminal.instance,
            "tunnel boundary terminal",
        )?;
        let boundary_terminal = resolve_macro_boundary_terminal(
            layout,
            boundary_instance,
            macro_boundary_side(tunnel.boundary_terminal.side),
            tunnel.width,
            settings.approach_depth,
        )?;
        let instance_route = tunnel
            .instance_route
            .iter()
            .map(|name| macro_named_patch(&named_ids, name, "tunnel instance route"))
            .collect::<Result<Vec<_>, _>>()?;
        let destination_anchor = ResolvedMacroAnchorReference {
            instance: macro_named_patch(
                &named_ids,
                &tunnel.destination_anchor.instance,
                "tunnel destination anchor",
            )?,
            anchor: tunnel.destination_anchor.anchor,
        };
        let authored_terminal = resolved_macro_tunnel_destination_terminal(
            settings,
            layout,
            &destination_anchor,
            tunnel.floor_level,
        )?;
        let mut previous = boundary_terminal
            .lanes
            .iter()
            .map(|(inside, _)| *inside)
            .collect::<BTreeSet<_>>();
        let mut seams = Vec::with_capacity(instance_route.len().saturating_sub(1));
        for pair in instance_route.windows(2) {
            let Some(source) = pair.first().copied() else {
                continue;
            };
            let Some(destination) = pair.get(1).copied() else {
                continue;
            };
            let preferred_destination = if destination == destination_anchor.instance {
                authored_terminal.as_ref()
            } else {
                None
            };
            let port = resolve_exact_macro_seam_port(
                layout,
                source,
                destination,
                tunnel.width,
                settings.approach_depth,
                Some(&previous),
                preferred_destination,
            )?;
            previous = port
                .lanes
                .iter()
                .map(|(_, destination)| *destination)
                .collect();
            seams.push(ResolvedMacroSubsurfaceSeam {
                source,
                destination,
                port,
            });
        }
        let name = tunnel.name;
        let resolved = ResolvedMacroSpanningFeature::Tunnel(ResolvedMacroTunnel {
            name: name.clone(),
            canonical_route: tunnel.canonical_route,
            instance_route,
            boundary_terminal,
            destination_anchor,
            floor_level: tunnel.floor_level,
            width: tunnel.width,
            clearance: tunnel.clearance,
            roof_thickness: tunnel.roof_thickness,
            seams,
        });
        if spanning_features.insert(name.clone(), resolved).is_some() {
            return Err(invalid_macro_contract(format!(
                "duplicate resolved spanning-feature name {name:?}"
            )));
        }
    }

    let mut alias_settings = settings.anchor_aliases.clone();
    alias_settings.sort_unstable();
    let mut anchor_aliases = BTreeMap::new();
    for alias in alias_settings {
        let reference = ResolvedMacroAnchorReference {
            instance: macro_named_patch(&named_ids, &alias.instance, "anchor alias")?,
            anchor: alias.anchor,
        };
        if anchor_aliases
            .insert(alias.alias.clone(), reference)
            .is_some()
        {
            return Err(invalid_macro_contract(format!(
                "duplicate resolved anchor alias {:?}",
                alias.alias
            )));
        }
    }

    Ok(ResolvedMacroContracts {
        walker_connections,
        spanning_features,
        anchor_aliases,
    })
}

fn resolved_macro_tunnel_destination_terminal(
    settings: &MacroLayoutSettings,
    layout: &ResolvedLayoutPlan,
    destination: &ResolvedMacroAnchorReference,
    floor_level: Level,
) -> Result<Option<BTreeSet<HexCoord>>, LayoutValidationError> {
    let instance = usize::try_from(destination.instance.0)
        .ok()
        .and_then(|index| settings.instances.get(index))
        .ok_or_else(|| invalid_macro_contract("tunnel destination instance is missing"))?;
    let V3RecipeSettings::CrystalAscent(crystal) = &instance.recipe else {
        return Ok(None);
    };
    if destination.anchor != "crystal_ascent.lower_entry" || floor_level != crystal.base_level {
        return Err(invalid_macro_contract(
            "CrystalAscent tunnel must terminate at crystal_ascent.lower_entry on its base level",
        ));
    }
    let patch = layout
        .patches
        .get(&destination.instance)
        .ok_or_else(|| invalid_macro_contract("CrystalAscent tunnel destination has no patch"))?;
    super::crystal_ascent::macro_lower_terminal_coords(
        &patch.mask,
        patch.rotation_turns,
        crystal.base_level,
    )
    .map(Some)
    .map_err(|error| {
        invalid_macro_contract(format!(
            "CrystalAscent lower-terminal projection failed: {error}"
        ))
    })
}

fn macro_named_patch(
    named_ids: &BTreeMap<&str, PatchId>,
    name: &str,
    label: &str,
) -> Result<PatchId, LayoutValidationError> {
    named_ids.get(name).copied().ok_or_else(|| {
        invalid_macro_contract(format!("{label} references unknown instance {name:?}"))
    })
}

fn invalid_macro_contract(message: impl Into<String>) -> LayoutValidationError {
    LayoutValidationError::one(LayoutIssue::InvalidMacroSettings(message.into()))
}

fn resolved_explicit_macro_walker_port(
    layout: &ResolvedLayoutPlan,
    first: PatchId,
    second: PatchId,
    width: u32,
    level: Level,
) -> Result<ResolvedPort, LayoutValidationError> {
    let pair = ordered_patch_pair(first, second);
    let matching = layout
        .shared_edges
        .values()
        .filter(|edge| ordered_patch_pair(edge.first.0, edge.second.0) == pair)
        .collect::<Vec<_>>();
    let [edge] = matching.as_slice() else {
        return Err(invalid_macro_contract(format!(
            "explicit walker {first:?} <-> {second:?} must resolve exactly one shared edge"
        )));
    };
    if edge.walker.count != 1
        || edge.walker.width != width
        || edge.walker.ports.len() != 1
        || edge.elevation
            != (ResolvedElevationBand {
                preferred: level,
                min: level,
                max: level,
            })
    {
        return Err(invalid_macro_contract(format!(
            "explicit walker {first:?} <-> {second:?} disagrees with its resolved width-{width}, level-{level} shared-edge contract"
        )));
    }
    edge.walker.ports.first().cloned().ok_or_else(|| {
        invalid_macro_contract(format!(
            "explicit walker {first:?} <-> {second:?} has no resolved surface port"
        ))
    })
}

fn resolve_exact_macro_seam_port(
    layout: &ResolvedLayoutPlan,
    source: PatchId,
    destination: PatchId,
    width: u32,
    approach_depth: u32,
    previous: Option<&BTreeSet<HexCoord>>,
    authored_destination: Option<&BTreeSet<HexCoord>>,
) -> Result<ResolvedPort, LayoutValidationError> {
    let source_mask = layout
        .patches
        .get(&source)
        .map(|patch| &patch.mask)
        .ok_or_else(|| invalid_macro_contract(format!("missing source patch {source:?}")))?;
    let destination_mask = layout
        .patches
        .get(&destination)
        .map(|patch| &patch.mask)
        .ok_or_else(|| {
            invalid_macro_contract(format!("missing destination patch {destination:?}"))
        })?;
    let pairs = boundary_pairs(source_mask, destination_mask);
    let candidates = macro_boundary_port_candidates(
        &pairs,
        source_mask,
        destination_mask,
        width,
        approach_depth,
        &layout.footprint,
    );
    let selected = candidates
        .into_iter()
        .filter(|candidate| {
            authored_destination.is_none_or(|terminal| {
                candidate
                    .port
                    .lanes
                    .iter()
                    .map(|(_, destination)| *destination)
                    .collect::<BTreeSet<_>>()
                    == *terminal
            })
        })
        .min_by(|first, second| {
            macro_candidate_route_score(&first.port, previous)
                .cmp(&macro_candidate_route_score(&second.port, previous))
                .then_with(|| first.port.cmp(&second.port))
        });
    selected.map(|candidate| candidate.port).ok_or_else(|| {
        invalid_macro_contract(format!(
            "instances {source:?} and {destination:?} cannot resolve one width-{width} exact seam port"
        ))
    })
}

fn macro_candidate_route_score(
    port: &ResolvedPort,
    previous: Option<&BTreeSet<HexCoord>>,
) -> (u32, u64) {
    let Some(previous) = previous.filter(|points| !points.is_empty()) else {
        let lane_bias = port
            .lanes
            .iter()
            .map(|(source, _)| source.distance(HexCoord::ORIGIN))
            .sum::<u32>();
        return (0, u64::from(lane_bias));
    };
    let distances = port
        .lanes
        .iter()
        .map(|(source, _)| {
            previous
                .iter()
                .map(|prior| source.distance(*prior))
                .min()
                .unwrap_or(u32::MAX)
        })
        .collect::<Vec<_>>();
    (
        distances.iter().copied().max().unwrap_or(u32::MAX),
        distances.iter().map(|distance| u64::from(*distance)).sum(),
    )
}

fn resolve_macro_boundary_terminal(
    layout: &ResolvedLayoutPlan,
    instance: PatchId,
    side: HexSide,
    width: u32,
    approach_depth: u32,
) -> Result<ResolvedMacroBoundaryTerminal, LayoutValidationError> {
    let mask = layout
        .patches
        .get(&instance)
        .map(|patch| &patch.mask)
        .ok_or_else(|| invalid_macro_contract(format!("missing boundary patch {instance:?}")))?;
    let boundary = mask
        .iter()
        .copied()
        .filter_map(|inside| {
            let outside = side.neighbor(inside);
            (!layout.footprint.contains(&outside)).then_some((inside, outside))
        })
        .collect::<BTreeSet<_>>();
    let ordered = ordered_simple_seam_lanes(&boundary).ok_or_else(|| {
        invalid_macro_contract(format!(
            "patch {instance:?} has no simple outer boundary on {side:?}"
        ))
    })?;
    let width = usize::try_from(width).map_err(|error| {
        invalid_macro_contract(format!("boundary width does not fit usize: {error}"))
    })?;
    let mut boundary_anchor = HexCoord::ORIGIN;
    while layout.footprint.contains(&side.neighbor(boundary_anchor)) {
        boundary_anchor = side.neighbor(boundary_anchor);
    }
    let candidate = ordered
        .windows(width)
        .filter_map(|window| {
            let lanes = window.iter().copied().collect::<BTreeSet<_>>();
            let inside = lanes
                .iter()
                .map(|(coord, _)| *coord)
                .collect::<BTreeSet<_>>();
            let inward_approach =
                approach_corridor(&inside, mask, side.opposite(), approach_depth)?;
            if boundary_outlet_touches_other_patch(
                &inside,
                &inward_approach,
                mask,
                &layout.footprint,
            ) {
                return None;
            }
            let maximum_anchor_distance = inside
                .iter()
                .map(|coord| coord.distance(boundary_anchor))
                .max()
                .unwrap_or_default();
            let total_anchor_distance = inside
                .iter()
                .map(|coord| coord.distance(boundary_anchor))
                .sum::<u32>();
            Some((
                maximum_anchor_distance,
                total_anchor_distance,
                lanes,
                inward_approach,
            ))
        })
        .min_by(|first, second| {
            first
                .0
                .cmp(&second.0)
                .then_with(|| first.1.cmp(&second.1))
                .then_with(|| first.2.cmp(&second.2))
        });
    let Some((_, _, lanes, inward_approach)) = candidate else {
        return Err(invalid_macro_contract(format!(
            "patch {instance:?} cannot resolve one width-{width} boundary terminal on {side:?}"
        )));
    };
    Ok(ResolvedMacroBoundaryTerminal {
        instance,
        side,
        lanes,
        inward_approach,
    })
}

const fn macro_boundary_side(side: MacroBoundarySideSettings) -> HexSide {
    match side {
        MacroBoundarySideSettings::East => HexSide::East,
        MacroBoundarySideSettings::SouthEast => HexSide::SouthEast,
        MacroBoundarySideSettings::SouthWest => HexSide::SouthWest,
        MacroBoundarySideSettings::West => HexSide::West,
        MacroBoundarySideSettings::NorthWest => HexSide::NorthWest,
        MacroBoundarySideSettings::NorthEast => HexSide::NorthEast,
    }
}

fn representative_boundary_side(pairs: &BTreeSet<(HexCoord, HexCoord)>) -> Option<HexSide> {
    HexSide::ALL
        .into_iter()
        .map(|side| {
            let count = pairs
                .iter()
                .filter(|(first, second)| side.neighbor(*first) == *second)
                .count();
            (count, Reverse(side))
        })
        .max()
        .and_then(|(count, Reverse(side))| (count > 0).then_some(side))
}

fn macro_shared_settings(
    first_id: PatchId,
    first_side: HexSide,
    first: &MacroBiomeInstanceSettings,
    second_id: PatchId,
    second_side: HexSide,
    second: &MacroBiomeInstanceSettings,
    approach_depth: u32,
    walker: WalkerPortSettings,
    exact_walker_level: Option<Level>,
    liquid: Option<&MacroLiquidConnectionSettings>,
) -> (
    PatchEdgeContractSettings,
    PatchEdgeContractSettings,
    Option<Level>,
) {
    let first_datum = macro_side_datum(first, first_side);
    let second_datum = macro_side_datum(second, second_side);
    let alpine_pair = matches!(
        &first.recipe,
        V3RecipeSettings::Mountains(_) | V3RecipeSettings::DeepMountain(_)
    ) && matches!(
        &second.recipe,
        V3RecipeSettings::Mountains(_) | V3RecipeSettings::DeepMountain(_)
    );
    let shared_alpine_step = if alpine_pair {
        if first.elevation.high == second.elevation.low {
            Some(first.elevation.high)
        } else if second.elevation.high == first.elevation.low {
            Some(second.elevation.high)
        } else {
            None
        }
    } else {
        None
    };
    let deep_mountain_buttress = if alpine_pair && shared_alpine_step.is_none() {
        match (&first.recipe, &second.recipe) {
            (V3RecipeSettings::DeepMountain(_), V3RecipeSettings::Mountains(_)) => {
                Some(first.elevation.low.saturating_add(second.elevation.high) / 2)
            }
            (V3RecipeSettings::Mountains(_), V3RecipeSettings::DeepMountain(_)) => {
                Some(second.elevation.low.saturating_add(first.elevation.high) / 2)
            }
            _ => None,
        }
    } else {
        None
    };
    let preferred = if let Some(exact) = exact_walker_level {
        exact
    } else if let Some(shared_step) = shared_alpine_step {
        shared_step
    } else if let Some(buttress) = deep_mountain_buttress {
        buttress
    } else if walker.count > 0 {
        first_datum.max(second_datum)
    } else {
        (first_datum + second_datum) / 2
    };
    let elevation = exact_walker_level.map_or(
        EdgeElevationSettings {
            preferred,
            min: preferred.saturating_sub(1),
            max: preferred.saturating_add(1).min(MAX_V3_LEVEL),
        },
        |level| EdgeElevationSettings {
            preferred: level,
            min: level,
            max: level,
        },
    );
    let land_connection = walker.count > 0
        && matches!(first.access, MacroAccessSettings::Land)
        && matches!(second.access, MacroAccessSettings::Land);
    let walker = if land_connection {
        walker
    } else {
        WalkerPortSettings { count: 0, width: 0 }
    };
    let (first_liquid, second_liquid, exact_level) = match liquid {
        Some(MacroLiquidConnectionSettings::Standing { width, level, .. }) => {
            let port = EdgeLiquidPortSettings { width: *width };
            (
                EdgeLiquidSettings::Standing(port),
                EdgeLiquidSettings::Standing(port),
                Some(*level),
            )
        }
        Some(MacroLiquidConnectionSettings::Directed {
            source_instance: _,
            sink_instance: _,
            width,
            level,
        }) => {
            let port = EdgeLiquidPortSettings { width: *width };
            let (source, sink) = match liquid {
                Some(MacroLiquidConnectionSettings::Directed {
                    source_instance,
                    sink_instance,
                    ..
                }) => (source_instance.as_str(), sink_instance.as_str()),
                _ => unreachable!("directed liquid arm"),
            };
            let first_is_source = first.name == source && second.name == sink;
            let second_is_source = second.name == source && first.name == sink;
            debug_assert!(first_is_source || second_is_source);
            if first_is_source || (!second_is_source && first_id < second_id) {
                (
                    EdgeLiquidSettings::Outlet(port),
                    EdgeLiquidSettings::Inlet(port),
                    Some(*level),
                )
            } else {
                (
                    EdgeLiquidSettings::Inlet(port),
                    EdgeLiquidSettings::Outlet(port),
                    Some(*level),
                )
            }
        }
        None => (EdgeLiquidSettings::Dry, EdgeLiquidSettings::Dry, None),
    };
    let first_settings = SharedEdgeSettings {
        elevation,
        walker,
        liquid: first_liquid,
        approach_depth,
    };
    let second_settings = SharedEdgeSettings {
        elevation,
        walker,
        liquid: second_liquid,
        approach_depth,
    };
    (
        PatchEdgeContractSettings::Shared(first_settings),
        PatchEdgeContractSettings::Shared(second_settings),
        exact_level,
    )
}

fn macro_side_datum(instance: &MacroBiomeInstanceSettings, side: HexSide) -> Level {
    let high_side = match instance.elevation.grade_axis {
        MacroAxisSettings::East => HexSide::East,
        MacroAxisSettings::SouthEast => HexSide::SouthEast,
        MacroAxisSettings::SouthWest => HexSide::SouthWest,
        MacroAxisSettings::West => HexSide::West,
        MacroAxisSettings::NorthWest => HexSide::NorthWest,
        MacroAxisSettings::NorthEast => HexSide::NorthEast,
    };
    if side == high_side {
        instance.elevation.high
    } else if side == high_side.opposite() {
        instance.elevation.low
    } else {
        (instance.elevation.low + instance.elevation.high) / 2
    }
}

fn split_boundary_components(
    pairs: &BTreeSet<(HexCoord, HexCoord)>,
) -> Vec<BTreeSet<(HexCoord, HexCoord)>> {
    let mut remaining = pairs.clone();
    let mut components = Vec::new();
    while let Some(start) = remaining.first().copied() {
        remaining.remove(&start);
        let mut component = BTreeSet::from([start]);
        let mut pending = VecDeque::from([start]);
        while let Some(current) = pending.pop_front() {
            let adjacent = remaining
                .iter()
                .copied()
                .filter(|candidate| {
                    current.0.distance(candidate.0) == 1 && current.1.distance(candidate.1) == 1
                })
                .collect::<Vec<_>>();
            for candidate in adjacent {
                remaining.remove(&candidate);
                component.insert(candidate);
                pending.push_back(candidate);
            }
        }
        components.push(component);
    }
    components
}

#[expect(
    clippy::too_many_arguments,
    reason = "the exact boundary outlet is defined by its complete resolved contract"
)]
fn resolve_boundary_liquid_outlet(
    source: PatchId,
    side: HexSide,
    width: u32,
    level: Level,
    approach_depth: u32,
    patch_center: HexCoord,
    mask: &BTreeSet<HexCoord>,
    footprint: &BTreeSet<HexCoord>,
    reserved: Option<&BTreeSet<HexCoord>>,
) -> Result<ResolvedBoundaryLiquidOutlet, LayoutValidationError> {
    if !(2..=MAX_SEAM_PORT_WIDTH).contains(&width) {
        return Err(LayoutValidationError::one(
            LayoutIssue::InvalidBoundaryLiquidOutlet(source, side),
        ));
    }
    let boundary = mask
        .iter()
        .copied()
        .filter_map(|inside| {
            let outside = side.neighbor(inside);
            (!footprint.contains(&outside)).then_some((inside, outside))
        })
        .collect::<BTreeSet<_>>();
    let Some(ordered) = ordered_simple_seam_lanes(&boundary) else {
        return Err(LayoutValidationError::one(
            LayoutIssue::InvalidBoundaryLiquidOutlet(source, side),
        ));
    };
    let mut boundary_anchor = patch_center;
    while footprint.contains(&side.neighbor(boundary_anchor)) {
        boundary_anchor = side.neighbor(boundary_anchor);
    }
    let width = usize::try_from(width).map_err(|_error| {
        LayoutValidationError::one(LayoutIssue::InvalidBoundaryLiquidOutlet(source, side))
    })?;
    let candidate = ordered
        .windows(width)
        .filter_map(|window| {
            let lanes = window.iter().copied().collect::<BTreeSet<_>>();
            let inside = lanes
                .iter()
                .map(|(coord, _)| *coord)
                .collect::<BTreeSet<_>>();
            let inward_approach =
                approach_corridor(&inside, mask, side.opposite(), approach_depth)?;
            if boundary_outlet_touches_other_patch(&inside, &inward_approach, mask, footprint)
                || reserved.is_some_and(|reserved| !reserved.is_disjoint(&inward_approach))
            {
                return None;
            }
            let maximum_anchor_distance = inside
                .iter()
                .map(|coord| coord.distance(boundary_anchor))
                .max()
                .unwrap_or_default();
            let total_anchor_distance = inside
                .iter()
                .map(|coord| coord.distance(boundary_anchor))
                .sum::<u32>();
            Some((
                maximum_anchor_distance,
                total_anchor_distance,
                lanes,
                inward_approach,
            ))
        })
        .min_by(|first, second| {
            first
                .0
                .cmp(&second.0)
                .then_with(|| first.1.cmp(&second.1))
                .then_with(|| first.2.cmp(&second.2))
        });
    let Some((_, _, lanes, inward_approach)) = candidate else {
        return Err(LayoutValidationError::one(
            LayoutIssue::InvalidBoundaryLiquidOutlet(source, side),
        ));
    };
    Ok(ResolvedBoundaryLiquidOutlet {
        source,
        side,
        lanes,
        inward_approach,
        approach_depth,
        level,
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
    resolve_shared_edge_with_pairs(
        first_id,
        first_side,
        first,
        second_id,
        second_side,
        second,
        masks,
        None,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "a shared edge is resolved from both endpoint contracts and optional Macro lanes"
)]
fn resolve_shared_edge_with_pairs(
    first_id: PatchId,
    first_side: HexSide,
    first: &PatchEdgeContractSettings,
    second_id: PatchId,
    second_side: HexSide,
    second: &PatchEdgeContractSettings,
    masks: &BTreeMap<PatchId, BTreeSet<HexCoord>>,
    explicit_boundary_pairs: Option<&BTreeSet<(HexCoord, HexCoord)>>,
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
    if first.walker.count > MAX_WALKER_PORT_COUNT
        || (first.walker.count == 0 && first.walker.width != 0)
        || (first.walker.count > 0 && !(2..=MAX_SEAM_PORT_WIDTH).contains(&first.walker.width))
    {
        return Err(LayoutValidationError::one(LayoutIssue::InvalidPortRequest(
            first_id, second_id,
        )));
    }
    let liquid_request = resolve_liquid_request(first_id, first, second_id, second)?;
    if matches!(
        liquid_request,
        LiquidRequest::Directed { width, .. } | LiquidRequest::Standing { width }
            if !(2..=MAX_SEAM_PORT_WIDTH).contains(&width)
                && width != 0
    ) {
        return Err(LayoutValidationError::one(LayoutIssue::InvalidPortRequest(
            first_id, second_id,
        )));
    }
    let first_mask = masks.get(&first_id).cloned().unwrap_or_default();
    let second_mask = masks.get(&second_id).cloned().unwrap_or_default();
    let footprint = masks
        .values()
        .flat_map(|mask| mask.iter().copied())
        .collect::<BTreeSet<_>>();
    let boundary_pairs = explicit_boundary_pairs
        .cloned()
        .unwrap_or_else(|| boundary_pairs(&first_mask, &second_mask));
    if boundary_pairs.is_empty() {
        return Err(LayoutValidationError::one(
            LayoutIssue::NonAdjacentSharedPatches(first_id, second_id),
        ));
    }
    let macro_geometry = explicit_boundary_pairs.is_some();
    let port_pairs = if macro_geometry {
        boundary_pairs.clone()
    } else {
        boundary_pairs
            .iter()
            .copied()
            .filter(|(first, second)| first_side.neighbor(*first) == *second)
            .collect()
    };
    let ordered_lanes = ordered_simple_seam_lanes(&port_pairs);
    if !macro_geometry && ordered_lanes.is_none() {
        return Err(LayoutValidationError::one(
            LayoutIssue::NonSimpleOrientedSeam(first_id, second_id),
        ));
    }
    let requires_inward_approach = first.walker.count > 0
        || matches!(
            liquid_request,
            LiquidRequest::Directed { .. } | LiquidRequest::Standing { width: 1.. }
        );
    if explicit_boundary_pairs.is_none()
        && requires_inward_approach
        && (!lane_approaches_are_independent(
            ordered_lanes
                .as_deref()
                .unwrap_or_default()
                .iter()
                .map(|(first, _)| *first),
            &first_mask,
            first_side.opposite(),
            first.approach_depth,
        ) || !lane_approaches_are_independent(
            ordered_lanes
                .as_deref()
                .unwrap_or_default()
                .iter()
                .map(|(_, second)| *second),
            &second_mask,
            second_side.opposite(),
            first.approach_depth,
        ))
    {
        return Err(LayoutValidationError::one(
            LayoutIssue::AmbiguousPortApproaches(first_id, second_id),
        ));
    }
    let mut requests = Vec::with_capacity(
        usize::from(first.walker.count)
            + usize::from(match liquid_request {
                LiquidRequest::Directed { .. } => true,
                LiquidRequest::Standing { width } => width > 0,
                LiquidRequest::Dry => false,
            }),
    );
    // Ring7 and Ring19 fingerprints select the liquid lane before walker lanes.
    // Macro routes instead reserve their required walker aperture before water.
    if !macro_geometry {
        match liquid_request {
            LiquidRequest::Directed { width, .. } => requests.push(PortRequest::Liquid(width)),
            LiquidRequest::Standing { width } if width > 0 => {
                requests.push(PortRequest::Liquid(width));
            }
            LiquidRequest::Dry | LiquidRequest::Standing { .. } => {}
        }
    }
    requests.extend(std::iter::repeat_n(
        PortRequest::Walker(first.walker.width),
        usize::from(first.walker.count),
    ));
    if macro_geometry {
        match liquid_request {
            LiquidRequest::Directed { width, .. } => requests.push(PortRequest::Liquid(width)),
            LiquidRequest::Standing { width } if width > 0 => {
                requests.push(PortRequest::Liquid(width));
            }
            LiquidRequest::Dry | LiquidRequest::Standing { .. } => {}
        }
    }
    let selected = select_ports(
        &requests,
        &port_pairs,
        &first_mask,
        &second_mask,
        first_side,
        second_side,
        first.approach_depth,
        &footprint,
        macro_geometry,
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
        LiquidRequest::Standing { width } => {
            let mut port = if width == 0 {
                let walker_first_approach = walker_ports
                    .iter()
                    .flat_map(|port| port.first_approach.iter().copied())
                    .collect::<BTreeSet<_>>();
                let walker_second_approach = walker_ports
                    .iter()
                    .flat_map(|port| port.second_approach.iter().copied())
                    .collect::<BTreeSet<_>>();
                let walker_exclusions = port_pairs
                    .iter()
                    .filter(|(first, second)| {
                        walker_first_approach.contains(first)
                            || walker_second_approach.contains(second)
                    })
                    .copied()
                    .collect::<BTreeSet<_>>();
                ResolvedPort {
                    // A full standing-water seam may coexist with an explicit
                    // dry walker aperture. Its complete protected approaches are
                    // exact exclusions, so the ocean remains continuous everywhere
                    // else without requiring liquid endpoints under the causeway.
                    lanes: port_pairs.difference(&walker_exclusions).copied().collect(),
                    first_approach: BTreeSet::new(),
                    second_approach: BTreeSet::new(),
                }
            } else {
                liquid_port.ok_or_else(|| {
                    LayoutValidationError::one(LayoutIssue::InsufficientPortCapacity(
                        first_id, second_id,
                    ))
                })?
            };
            port.first_approach.clear();
            port.second_approach.clear();
            ResolvedLiquidPort::Standing {
                port,
                elevation: ResolvedLiquidElevation::EdgeBand,
            }
        }
        LiquidRequest::Directed { source, sink, .. } => {
            let Some(port) = liquid_port else {
                return Err(LayoutValidationError::one(
                    LayoutIssue::InsufficientPortCapacity(first_id, second_id),
                ));
            };
            ResolvedLiquidPort::Directed {
                source,
                sink,
                port,
                elevation: ResolvedLiquidElevation::EdgeBand,
            }
        }
    };
    let mut first_approaches = BTreeSet::new();
    let mut second_approaches = BTreeSet::new();
    for port in walker_ports.iter().chain(liquid_port_ref(&liquid)) {
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
            count: first.walker.count,
            width: first.walker.width,
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
    Standing {
        width: u32,
    },
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

#[derive(Debug, Clone)]
struct PortCandidate {
    start: usize,
    end: usize,
    port: ResolvedPort,
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
        (EdgeLiquidSettings::Standing(first), EdgeLiquidSettings::Standing(second))
            if first.width == second.width =>
        {
            Ok(LiquidRequest::Standing { width: first.width })
        }
        _ => Err(LayoutValidationError::one(LayoutIssue::MismatchedLiquid(
            first_id, second_id,
        ))),
    }
}

fn liquid_port_ref(liquid: &ResolvedLiquidPort) -> Option<&ResolvedPort> {
    match liquid {
        ResolvedLiquidPort::Dry | ResolvedLiquidPort::Standing { .. } => None,
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
    footprint: &BTreeSet<HexCoord>,
    macro_geometry: bool,
) -> Option<Vec<ResolvedPort>> {
    if requests.is_empty() {
        return Some(Vec::new());
    }
    if requests.len() > usize::from(MAX_WALKER_PORT_COUNT) + 1
        || requests
            .iter()
            .any(|request| !(2..=MAX_SEAM_PORT_WIDTH).contains(&request.width()))
    {
        return None;
    }
    let ordered_lanes = (!macro_geometry)
        .then(|| ordered_simple_seam_lanes(boundary_pairs))
        .flatten();
    let required_lanes = requests
        .iter()
        .try_fold(0_u32, |total, request| total.checked_add(request.width()))?;
    let required_gaps = requests.len().saturating_sub(1);
    if usize::try_from(required_lanes)
        .ok()?
        .checked_add(required_gaps)?
        > if macro_geometry {
            boundary_pairs.len()
        } else {
            ordered_lanes.as_ref()?.len()
        }
    {
        return None;
    }

    let mut candidates = BTreeMap::<u32, Vec<PortCandidate>>::new();
    for width in requests.iter().map(|request| request.width()) {
        candidates.entry(width).or_insert_with(|| {
            if macro_geometry {
                macro_boundary_port_candidates(
                    boundary_pairs,
                    first_mask,
                    second_mask,
                    width,
                    approach_depth,
                    footprint,
                )
            } else {
                port_candidates(
                    ordered_lanes.as_deref().unwrap_or_default(),
                    first_mask,
                    second_mask,
                    first_side,
                    second_side,
                    width,
                    approach_depth,
                    footprint,
                )
            }
        });
    }
    if requests
        .iter()
        .any(|request| candidates.get(&request.width()).is_none_or(Vec::is_empty))
    {
        return None;
    }

    let seam_leaves = seam_leaves(boundary_pairs);
    let mut solutions = Vec::new();
    if let Some(greedy) = score_greedy_ports(requests, &candidates, &seam_leaves, macro_geometry) {
        solutions.push(greedy);
    }
    let mut order: Vec<_> = (0..requests.len()).collect();
    let mut permutations = Vec::new();
    collect_permutations(&mut order, 0, &mut permutations);
    for permutation in permutations {
        if let Some(solution) = earliest_ports_for_order(requests, &candidates, &permutation) {
            solutions.push(solution);
        }
    }
    solutions.into_iter().max_by(|first, second| {
        (macro_geometry.then(|| macro_walker_lane_bias(first, requests)))
            .cmp(&macro_geometry.then(|| macro_walker_lane_bias(second, requests)))
            .then_with(|| {
                port_set_score(first, &seam_leaves).cmp(&port_set_score(second, &seam_leaves))
            })
            .then_with(|| second.cmp(first))
    })
}

fn macro_walker_lane_bias(ports: &[ResolvedPort], requests: &[PortRequest]) -> i64 {
    ports
        .iter()
        .zip(requests)
        .filter(|(_, request)| matches!(request, PortRequest::Walker(_)))
        .flat_map(|(port, _)| port.lanes.iter().map(|(first, _)| i64::from(first.y())))
        .sum()
}

fn score_greedy_ports(
    requests: &[PortRequest],
    candidates: &BTreeMap<u32, Vec<PortCandidate>>,
    seam_leaves: &BTreeSet<HexCoord>,
    macro_geometry: bool,
) -> Option<Vec<ResolvedPort>> {
    let mut selected = Vec::with_capacity(requests.len());
    for request in requests {
        let candidate = candidates
            .get(&request.width())?
            .iter()
            .filter(|candidate| {
                selected
                    .iter()
                    .all(|existing| ports_are_disjoint(existing, &candidate.port))
            })
            .max_by(|first, second| {
                let first_bias = (macro_geometry && matches!(request, PortRequest::Walker(_)))
                    .then(|| macro_port_lane_bias(&first.port));
                let second_bias = (macro_geometry && matches!(request, PortRequest::Walker(_)))
                    .then(|| macro_port_lane_bias(&second.port));
                first_bias
                    .cmp(&second_bias)
                    .then_with(|| {
                        port_option_score(&first.port, &selected, seam_leaves)
                            .cmp(&port_option_score(&second.port, &selected, seam_leaves))
                    })
                    .then_with(|| second.port.cmp(&first.port))
            })?;
        selected.push(candidate.port.clone());
    }
    Some(selected)
}

fn macro_port_lane_bias(port: &ResolvedPort) -> i64 {
    port.lanes
        .iter()
        .map(|(first, _)| i64::from(first.y()))
        .sum()
}

fn collect_permutations(order: &mut [usize], index: usize, results: &mut Vec<Vec<usize>>) {
    if index == order.len() {
        results.push(order.to_vec());
        return;
    }
    for swap_index in index..order.len() {
        order.swap(index, swap_index);
        collect_permutations(order, index + 1, results);
        order.swap(index, swap_index);
    }
}

fn earliest_ports_for_order(
    requests: &[PortRequest],
    candidates: &BTreeMap<u32, Vec<PortCandidate>>,
    order: &[usize],
) -> Option<Vec<ResolvedPort>> {
    let mut selected = vec![None; requests.len()];
    let mut previous_end: Option<usize> = None;
    for request_index in order {
        let request = requests.get(*request_index)?;
        let candidate = candidates.get(&request.width())?.iter().find(|candidate| {
            previous_end.is_none_or(|end| candidate.start > end.saturating_add(1))
        })?;
        previous_end = Some(candidate.end);
        *selected.get_mut(*request_index)? = Some(candidate.port.clone());
    }
    selected.into_iter().collect()
}

fn port_set_score(ports: &[ResolvedPort], seam_leaves: &BTreeSet<HexCoord>) -> (u32, u32) {
    let cells: Vec<Vec<_>> = ports
        .iter()
        .map(|port| port.lanes.iter().map(|(first, _)| *first).collect())
        .collect();
    let separation = cells
        .iter()
        .enumerate()
        .flat_map(|(index, first)| {
            cells.iter().skip(index + 1).flat_map(move |second| {
                first
                    .iter()
                    .flat_map(move |first| second.iter().map(move |second| (*first, *second)))
            })
        })
        .map(|(first, second)| first.distance(second))
        .min()
        .unwrap_or(0);
    let margin = cells
        .iter()
        .flatten()
        .flat_map(|cell| seam_leaves.iter().map(move |leaf| cell.distance(*leaf)))
        .min()
        .unwrap_or(0);
    (separation, margin)
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
    ordered_lanes: &[(HexCoord, HexCoord)],
    first_mask: &BTreeSet<HexCoord>,
    second_mask: &BTreeSet<HexCoord>,
    first_side: HexSide,
    second_side: HexSide,
    width: u32,
    approach_depth: u32,
    footprint: &BTreeSet<HexCoord>,
) -> Vec<PortCandidate> {
    let Ok(width) = usize::try_from(width) else {
        return Vec::new();
    };
    if width == 0 || width > ordered_lanes.len() {
        return Vec::new();
    }
    ordered_lanes
        .windows(width)
        .enumerate()
        .filter_map(|(start, window)| {
            let end = start.saturating_add(width).saturating_sub(1);
            let lanes: BTreeSet<_> = window.iter().copied().collect();
            let first_boundary = lanes
                .iter()
                .map(|(coord, _)| *coord)
                .collect::<BTreeSet<_>>();
            let second_boundary = lanes
                .iter()
                .map(|(_, coord)| *coord)
                .collect::<BTreeSet<_>>();
            if first_boundary
                .iter()
                .any(|coord| lane_touches_third_patch(*coord, first_mask, second_mask, footprint))
                || second_boundary.iter().any(|coord| {
                    lane_touches_third_patch(*coord, second_mask, first_mask, footprint)
                })
            {
                return None;
            }
            let first_approach = approach_corridor(
                &first_boundary,
                first_mask,
                first_side.opposite(),
                approach_depth,
            )?;
            let second_approach = approach_corridor(
                &second_boundary,
                second_mask,
                second_side.opposite(),
                approach_depth,
            )?;
            if approach_touches_other_patch(&first_approach, &first_boundary, first_mask, footprint)
                || approach_touches_other_patch(
                    &second_approach,
                    &second_boundary,
                    second_mask,
                    footprint,
                )
            {
                return None;
            }
            Some(PortCandidate {
                start,
                end,
                port: ResolvedPort {
                    lanes,
                    first_approach,
                    second_approach,
                },
            })
        })
        .collect()
}

fn macro_port_candidates(
    ordered_lanes: &[(HexCoord, HexCoord)],
    first_mask: &BTreeSet<HexCoord>,
    second_mask: &BTreeSet<HexCoord>,
    width: u32,
    approach_depth: u32,
    footprint: &BTreeSet<HexCoord>,
) -> Vec<PortCandidate> {
    let Ok(width) = usize::try_from(width) else {
        return Vec::new();
    };
    if width == 0 || width > ordered_lanes.len() {
        return Vec::new();
    }
    ordered_lanes
        .windows(width)
        .enumerate()
        .filter_map(|(start, window)| {
            let end = start.saturating_add(width).saturating_sub(1);
            let lanes = window.iter().copied().collect::<BTreeSet<_>>();
            let first_boundary = lanes
                .iter()
                .map(|(coord, _)| *coord)
                .collect::<BTreeSet<_>>();
            let second_boundary = lanes
                .iter()
                .map(|(_, coord)| *coord)
                .collect::<BTreeSet<_>>();
            if first_boundary
                .iter()
                .any(|coord| lane_touches_third_patch(*coord, first_mask, second_mask, footprint))
                || second_boundary.iter().any(|coord| {
                    lane_touches_third_patch(*coord, second_mask, first_mask, footprint)
                })
            {
                return None;
            }
            let (first_approach, second_approach) =
                macro_approach_corridors(&lanes, first_mask, second_mask, approach_depth)?;
            if approach_touches_other_patch(&first_approach, &first_boundary, first_mask, footprint)
                || approach_touches_other_patch(
                    &second_approach,
                    &second_boundary,
                    second_mask,
                    footprint,
                )
            {
                return None;
            }
            Some(PortCandidate {
                start,
                end,
                port: ResolvedPort {
                    lanes,
                    first_approach,
                    second_approach,
                },
            })
        })
        .collect()
}

fn macro_boundary_port_candidates(
    boundary_pairs: &BTreeSet<(HexCoord, HexCoord)>,
    first_mask: &BTreeSet<HexCoord>,
    second_mask: &BTreeSet<HexCoord>,
    width: u32,
    approach_depth: u32,
    footprint: &BTreeSet<HexCoord>,
) -> Vec<PortCandidate> {
    let mut candidates = Vec::new();
    let mut offset = 0;
    for component in split_boundary_components(boundary_pairs) {
        let Some(ordered) = ordered_simple_seam_lanes(&component) else {
            continue;
        };
        let component_len = ordered.len();
        let mut component_candidates = macro_port_candidates(
            &ordered,
            first_mask,
            second_mask,
            width,
            approach_depth,
            footprint,
        );
        for candidate in &mut component_candidates {
            candidate.start = candidate.start.saturating_add(offset);
            candidate.end = candidate.end.saturating_add(offset);
        }
        candidates.extend(component_candidates);
        offset = offset.saturating_add(component_len).saturating_add(1);
    }
    candidates
}

fn macro_approach_corridors(
    lanes: &BTreeSet<(HexCoord, HexCoord)>,
    first_mask: &BTreeSet<HexCoord>,
    second_mask: &BTreeSet<HexCoord>,
    depth: u32,
) -> Option<(BTreeSet<HexCoord>, BTreeSet<HexCoord>)> {
    let mut first_approach = BTreeSet::new();
    let mut second_approach = BTreeSet::new();
    for (first, second) in lanes {
        let side = HexSide::ALL
            .into_iter()
            .find(|side| side.neighbor(*first) == *second)?;
        let mut first_cell = *first;
        let mut second_cell = *second;
        for _ in 0..depth {
            if !first_mask.contains(&first_cell) || !second_mask.contains(&second_cell) {
                return None;
            }
            first_approach.insert(first_cell);
            second_approach.insert(second_cell);
            first_cell = side.opposite().neighbor(first_cell);
            second_cell = side.neighbor(second_cell);
        }
    }
    Some((first_approach, second_approach))
}

fn lane_touches_third_patch(
    coord: HexCoord,
    local_mask: &BTreeSet<HexCoord>,
    counterpart_mask: &BTreeSet<HexCoord>,
    footprint: &BTreeSet<HexCoord>,
) -> bool {
    coord.neighbors().into_iter().any(|neighbor| {
        footprint.contains(&neighbor)
            && !local_mask.contains(&neighbor)
            && !counterpart_mask.contains(&neighbor)
    })
}

fn approach_touches_other_patch(
    approach: &BTreeSet<HexCoord>,
    declared_lane: &BTreeSet<HexCoord>,
    local_mask: &BTreeSet<HexCoord>,
    footprint: &BTreeSet<HexCoord>,
) -> bool {
    approach.iter().any(|coord| {
        !declared_lane.contains(coord)
            && coord
                .neighbors()
                .into_iter()
                .any(|neighbor| footprint.contains(&neighbor) && !local_mask.contains(&neighbor))
    })
}

fn boundary_outlet_touches_other_patch(
    inside: &BTreeSet<HexCoord>,
    inward_approach: &BTreeSet<HexCoord>,
    local_mask: &BTreeSet<HexCoord>,
    footprint: &BTreeSet<HexCoord>,
) -> bool {
    inside.iter().any(|coord| {
        coord
            .neighbors()
            .into_iter()
            .any(|neighbor| footprint.contains(&neighbor) && !local_mask.contains(&neighbor))
    }) || approach_touches_other_patch(inward_approach, inside, local_mask, footprint)
}

fn generated_ring_masks(footprint: &BTreeSet<HexCoord>) -> BTreeMap<PatchId, BTreeSet<HexCoord>> {
    let centers = ring_centers();
    generated_nearest_masks(footprint, &centers)
}

fn generated_nearest_masks(
    footprint: &BTreeSet<HexCoord>,
    centers: &BTreeMap<PatchId, HexCoord>,
) -> BTreeMap<PatchId, BTreeSet<HexCoord>> {
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

fn ring19_macro_centers() -> BTreeMap<PatchId, HexCoord> {
    (0..V3_RING19_REGION_COUNT)
        .filter_map(|id| {
            let slot = u8::try_from(id).ok()?;
            let (x, y, z) = ring19_region_coord(slot)?;
            Some((
                PatchId(u32::try_from(id).unwrap_or(u32::MAX)),
                HexCoord::new_cubic(x, y, z),
            ))
        })
        .collect()
}

pub(crate) fn ring19_patch_center(id: PatchId) -> Option<HexCoord> {
    let slot = u8::try_from(id.0).ok()?;
    let (x, y, z) = ring19_region_coord(slot)?;
    Some(HexCoord::new_cubic(
        x * RING_PATCH_OFFSET,
        y * RING_PATCH_OFFSET,
        z * RING_PATCH_OFFSET,
    ))
}

fn scaled_centers(
    centers: &BTreeMap<PatchId, HexCoord>,
    scale: i32,
) -> BTreeMap<PatchId, HexCoord> {
    centers
        .iter()
        .map(|(id, center)| {
            let [x, y, z] = center.to_cubic_array();
            (*id, HexCoord::new_cubic(x * scale, y * scale, z * scale))
        })
        .collect()
}

fn ring19_seams(
    macro_centers: &BTreeMap<PatchId, HexCoord>,
) -> Vec<(PatchId, HexSide, PatchId, HexSide)> {
    let by_center = macro_centers
        .iter()
        .map(|(id, center)| (*center, *id))
        .collect::<BTreeMap<_, _>>();
    let mut seams = Vec::new();
    for (first_id, center) in macro_centers {
        for first_side in HexSide::ALL {
            let Some(second_id) = by_center.get(&first_side.neighbor(*center)).copied() else {
                continue;
            };
            if *first_id < second_id {
                seams.push((*first_id, first_side, second_id, first_side.opposite()));
            }
        }
    }
    seams.sort_unstable_by_key(|(first, _, second, _)| (*first, *second));
    seams
}

const fn ring19_boundary_side(side: Ring19BoundarySide) -> HexSide {
    match side {
        Ring19BoundarySide::East => HexSide::East,
        Ring19BoundarySide::SouthEast => HexSide::SouthEast,
        Ring19BoundarySide::SouthWest => HexSide::SouthWest,
        Ring19BoundarySide::West => HexSide::West,
        Ring19BoundarySide::NorthWest => HexSide::NorthWest,
        Ring19BoundarySide::NorthEast => HexSide::NorthEast,
    }
}

fn ordered_patch_pair(first: PatchId, second: PatchId) -> (PatchId, PatchId) {
    if first < second {
        (first, second)
    } else {
        (second, first)
    }
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

fn lane_approaches_are_independent(
    boundary: impl IntoIterator<Item = HexCoord>,
    mask: &BTreeSet<HexCoord>,
    inward: HexSide,
    depth: u32,
) -> bool {
    seam_approaches_are_independent(boundary, mask, |coord| inward.neighbor(coord), depth)
}

fn connected(mask: &BTreeSet<HexCoord>) -> bool {
    let Some(start) = mask.first().copied() else {
        return false;
    };
    // Connectivity has no ordering semantics. Consume a hash-indexed remainder
    // so each cell and neighbor probe is expected O(1), while the BTree-backed
    // authored mask remains the canonical stored/serialized form.
    let mut remaining = mask.iter().copied().collect::<HashSet<_>>();
    remaining.remove(&start);
    let mut frontier = VecDeque::from([start]);
    while let Some(coord) = frontier.pop_front() {
        for neighbor in coord.neighbors() {
            if remaining.remove(&neighbor) {
                frontier.push_back(neighbor);
            }
        }
    }
    remaining.is_empty()
}

fn validate_resolved_edge(
    kind: LayoutKind,
    id: ResolvedEdgeId,
    edge: &ResolvedEdgeContract,
    patches: &BTreeMap<PatchId, ResolvedPatch>,
    footprint: &BTreeSet<HexCoord>,
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
    let all_pairs = boundary_pairs(&first.mask, &second.mask);
    let boundary_valid = !edge.boundary_pairs.is_empty() && edge.boundary_pairs == all_pairs;
    if !boundary_valid {
        issues.push(LayoutIssue::InvalidBoundaryPairs(id));
    }
    let oriented_pairs = if kind == LayoutKind::Macro {
        edge.boundary_pairs.clone()
    } else {
        edge.boundary_pairs
            .iter()
            .copied()
            .filter(|(first, second)| edge.first.1.neighbor(*first) == *second)
            .collect()
    };
    let requires_inward_approach = kind != LayoutKind::Macro
        && (edge.walker.count > 0 || matches!(edge.liquid, ResolvedLiquidPort::Directed { .. }));
    let seam_geometry_valid = kind == LayoutKind::Macro
        || ordered_simple_seam_lanes(&oriented_pairs).is_some_and(|ordered_lanes| {
            !requires_inward_approach
                || (lane_approaches_are_independent(
                    ordered_lanes.iter().map(|(first, _)| *first),
                    &first.mask,
                    edge.first.1.opposite(),
                    edge.approach_depth,
                ) && lane_approaches_are_independent(
                    ordered_lanes.iter().map(|(_, second)| *second),
                    &second.mask,
                    edge.second.1.opposite(),
                    edge.approach_depth,
                ))
        });
    let directed_port = match &edge.liquid {
        ResolvedLiquidPort::Directed { port, .. } => Some(port),
        ResolvedLiquidPort::Dry | ResolvedLiquidPort::Standing { .. } => None,
    };
    let all_ports: Vec<_> = edge.walker.ports.iter().chain(directed_port).collect();
    let ports_valid = all_ports.iter().all(|port| {
        valid_resolved_port(
            kind,
            port,
            edge,
            &first.mask,
            &second.mask,
            &edge.boundary_pairs,
            footprint,
        )
    }) && all_ports.iter().enumerate().all(|(index, port)| {
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
    let walker_count_matches = usize::from(edge.walker.count) == edge.walker.ports.len();
    let walker_width_matches = if edge.walker.count == 0 {
        edge.walker.width == 0
    } else {
        (2..=MAX_SEAM_PORT_WIDTH).contains(&edge.walker.width)
            && edge
                .walker
                .ports
                .iter()
                .all(|port| u32::try_from(port.lanes.len()).ok() == Some(edge.walker.width))
    };
    if edge.first.1.opposite() != edge.second.1
        || edge.elevation.min < 0
        || edge.elevation.min > edge.elevation.preferred
        || edge.elevation.preferred > edge.elevation.max
        || edge.elevation.max > MAX_V3_LEVEL
        || edge.walker.count > MAX_WALKER_PORT_COUNT
        || !walker_count_matches
        || !walker_width_matches
        || !seam_geometry_valid
        || !ports_valid
    {
        issues.push(LayoutIssue::InvalidResolvedContract(id));
    }
    if let ResolvedLiquidPort::Standing { port, elevation } = &edge.liquid {
        let walker_first_approach = edge
            .walker
            .ports
            .iter()
            .flat_map(|walker| walker.first_approach.iter().copied())
            .collect::<BTreeSet<_>>();
        let walker_second_approach = edge
            .walker
            .ports
            .iter()
            .flat_map(|walker| walker.second_approach.iter().copied())
            .collect::<BTreeSet<_>>();
        let walker_exclusions = edge
            .boundary_pairs
            .iter()
            .filter(|(first, second)| {
                walker_first_approach.contains(first) || walker_second_approach.contains(second)
            })
            .copied()
            .collect::<BTreeSet<_>>();
        let full_except_walker = kind == LayoutKind::Macro
            && !walker_exclusions.is_empty()
            && port.lanes.is_disjoint(&walker_exclusions)
            && port
                .lanes
                .union(&walker_exclusions)
                .copied()
                .collect::<BTreeSet<_>>()
                == edge.boundary_pairs;
        let lane_width_valid = port.lanes == edge.boundary_pairs
            || full_except_walker
            || u32::try_from(port.lanes.len())
                .is_ok_and(|width| (2..=MAX_SEAM_PORT_WIDTH).contains(&width));
        let geometry_valid = (kind == LayoutKind::Macro
            && (port.lanes == edge.boundary_pairs || full_except_walker))
            || ordered_simple_seam_lanes(&port.lanes).is_some();
        let width_valid = lane_width_valid
            && port.lanes.is_subset(&edge.boundary_pairs)
            && geometry_valid
            && port.first_approach.is_empty()
            && port.second_approach.is_empty();
        let elevation_valid = matches!(
            (kind, elevation),
            (LayoutKind::Macro, ResolvedLiquidElevation::Exact(level))
                if (3..=MAX_V3_LEVEL).contains(level)
        );
        if !width_valid || !elevation_valid {
            issues.push(LayoutIssue::InvalidResolvedContract(id));
        }
    } else if let ResolvedLiquidPort::Directed {
        source,
        sink,
        port,
        elevation,
    } = &edge.liquid
    {
        let endpoints = BTreeSet::from([edge.first.0, edge.second.0]);
        let width_valid = u32::try_from(port.lanes.len())
            .is_ok_and(|width| (2..=MAX_SEAM_PORT_WIDTH).contains(&width));
        let elevation_valid = match (kind, elevation) {
            (LayoutKind::Ring19, ResolvedLiquidElevation::Exact(level)) => {
                (3..=MAX_V3_LEVEL).contains(level)
            }
            (LayoutKind::Macro, ResolvedLiquidElevation::Exact(level)) => {
                (3..=MAX_V3_LEVEL).contains(level)
            }
            (LayoutKind::Single | LayoutKind::Ring7, ResolvedLiquidElevation::EdgeBand) => true,
            _ => false,
        };
        if source == sink
            || !endpoints.contains(source)
            || !endpoints.contains(sink)
            || !width_valid
            || !elevation_valid
        {
            issues.push(LayoutIssue::InvalidResolvedContract(id));
        }
    }
}

fn validate_boundary_liquid_outlets(layout: &ResolvedLayoutPlan, issues: &mut Vec<LayoutIssue>) {
    if layout.kind != LayoutKind::Ring19 {
        for (source, side) in layout.boundary_liquid_outlets.keys() {
            issues.push(LayoutIssue::InvalidBoundaryLiquidOutlet(*source, *side));
        }
        return;
    }
    if layout.boundary_liquid_outlets.is_empty() {
        // A completely dry Ring19 is a valid closed world (for example the
        // Desert Oasis profile, whose water is local to its centre patch).
        // A boundary outlet is required only when a resolved internal seam
        // actually transports liquid.
        if layout
            .shared_edges
            .values()
            .any(|edge| !matches!(edge.liquid, ResolvedLiquidPort::Dry))
        {
            issues.push(LayoutIssue::MissingBoundaryLiquidOutlet);
        }
        return;
    }

    let mut reserved = BTreeMap::<PatchId, BTreeSet<HexCoord>>::new();
    for edge in layout.shared_edges.values() {
        for (patch, approach) in &edge.protected_approaches {
            reserved
                .entry(*patch)
                .or_default()
                .extend(approach.iter().copied());
        }
    }

    for ((source, side), outlet) in &layout.boundary_liquid_outlets {
        let Some(patch) = layout.patches.get(source) else {
            issues.push(LayoutIssue::InvalidBoundaryLiquidOutlet(*source, *side));
            continue;
        };
        let inside = outlet
            .lanes
            .iter()
            .map(|(inside, _)| *inside)
            .collect::<BTreeSet<_>>();
        let outside = outlet
            .lanes
            .iter()
            .map(|(_, outside)| *outside)
            .collect::<BTreeSet<_>>();
        let width_valid = u32::try_from(outlet.lanes.len())
            .is_ok_and(|width| (2..=MAX_SEAM_PORT_WIDTH).contains(&width));
        let geometry_valid = outlet.source == *source
            && outlet.side == *side
            && matches!(
                patch.edges.get(side),
                Some(ResolvedEdgeReference::WorldBoundary)
            )
            && width_valid
            && inside.len() == outlet.lanes.len()
            && outside.len() == outlet.lanes.len()
            && outlet.lanes.iter().all(|(inside, outside)| {
                side.neighbor(*inside) == *outside
                    && patch.mask.contains(inside)
                    && !layout.footprint.contains(outside)
            })
            && ordered_simple_seam_lanes(&outlet.lanes).is_some()
            && lane_approaches_are_independent(
                inside.iter().copied(),
                &patch.mask,
                side.opposite(),
                outlet.approach_depth,
            )
            && approach_corridor(&inside, &patch.mask, side.opposite(), outlet.approach_depth)
                .is_some_and(|expected| expected == outlet.inward_approach)
            && !boundary_outlet_touches_other_patch(
                &inside,
                &outlet.inward_approach,
                &patch.mask,
                &layout.footprint,
            )
            && (3..=MAX_V3_LEVEL).contains(&outlet.level)
            && reserved
                .get(source)
                .is_none_or(|existing| existing.is_disjoint(&outlet.inward_approach));
        if !geometry_valid {
            issues.push(LayoutIssue::InvalidBoundaryLiquidOutlet(*source, *side));
            continue;
        }
        reserved
            .entry(*source)
            .or_default()
            .extend(outlet.inward_approach.iter().copied());
    }
}

fn validate_macro_edge_coverage(layout: &ResolvedLayoutPlan, issues: &mut Vec<LayoutIssue>) {
    let ids = layout.patches.keys().copied().collect::<Vec<_>>();
    for (position, first_id) in ids.iter().copied().enumerate() {
        for second_id in ids.iter().copied().skip(position + 1) {
            let Some(first) = layout.patches.get(&first_id) else {
                continue;
            };
            let Some(second) = layout.patches.get(&second_id) else {
                continue;
            };
            let expected = boundary_pairs(&first.mask, &second.mask);
            let mut actual = BTreeSet::new();
            let mut duplicate = false;
            for edge in layout.shared_edges.values().filter(|edge| {
                ordered_patch_pair(edge.first.0, edge.second.0)
                    == ordered_patch_pair(first_id, second_id)
            }) {
                for (edge_first, edge_second) in &edge.boundary_pairs {
                    let pair = if edge.first.0 == first_id {
                        (*edge_first, *edge_second)
                    } else {
                        (*edge_second, *edge_first)
                    };
                    if !actual.insert(pair) {
                        duplicate = true;
                    }
                }
            }
            if duplicate || actual != expected {
                issues.push(LayoutIssue::InvalidMacroEdgeCoverage(first_id, second_id));
            }
        }
    }
}

fn validate_liquid_graph(layout: &ResolvedLayoutPlan, issues: &mut Vec<LayoutIssue>) {
    let mut outgoing = layout
        .patches
        .keys()
        .copied()
        .map(|patch| (patch, BTreeSet::new()))
        .collect::<BTreeMap<_, _>>();
    let mut liquid_regions = BTreeSet::new();
    for edge in layout.shared_edges.values() {
        let ResolvedLiquidPort::Directed { source, sink, .. } = edge.liquid else {
            continue;
        };
        if source == sink
            || !layout.patches.contains_key(&source)
            || !layout.patches.contains_key(&sink)
        {
            continue;
        }
        liquid_regions.extend([source, sink]);
        if let Some(sinks) = outgoing.get_mut(&source) {
            sinks.insert(sink);
        }
    }

    let mut indegree = layout
        .patches
        .keys()
        .copied()
        .map(|patch| (patch, 0_usize))
        .collect::<BTreeMap<_, _>>();
    for sinks in outgoing.values() {
        for sink in sinks {
            if let Some(degree) = indegree.get_mut(sink) {
                *degree += 1;
            }
        }
    }
    let mut ready: VecDeque<_> = indegree
        .iter()
        .filter_map(|(patch, degree)| (*degree == 0).then_some(*patch))
        .collect();
    let mut visited = 0;
    while let Some(patch) = ready.pop_front() {
        visited += 1;
        if let Some(sinks) = outgoing.get(&patch) {
            for sink in sinks {
                if let Some(degree) = indegree.get_mut(sink) {
                    *degree -= 1;
                    if *degree == 0 {
                        ready.push_back(*sink);
                    }
                }
            }
        }
    }
    if visited != layout.patches.len() {
        issues.push(LayoutIssue::CyclicLiquidGraph);
    }

    if layout.kind != LayoutKind::Ring19 {
        return;
    }
    let mut boundary_sources = BTreeSet::new();
    for outlet in layout.boundary_liquid_outlets.values() {
        if !boundary_sources.insert(outlet.source)
            || outgoing
                .get(&outlet.source)
                .is_some_and(|sinks| !sinks.is_empty())
        {
            issues.push(LayoutIssue::MultipleLiquidOutlets(outlet.source));
        }
    }
    for (source, sinks) in &outgoing {
        if sinks.len() > 1 {
            issues.push(LayoutIssue::MultipleLiquidOutlets(*source));
        }
    }
    for origin in liquid_regions {
        let mut current = origin;
        let mut visited = BTreeSet::new();
        while !boundary_sources.contains(&current) {
            if !visited.insert(current) {
                break;
            }
            let Some(next) = outgoing
                .get(&current)
                .and_then(|sinks| sinks.first())
                .copied()
            else {
                issues.push(LayoutIssue::LiquidComponentWithoutBoundary(origin));
                break;
            };
            current = next;
        }
    }
}

fn valid_resolved_port(
    kind: LayoutKind,
    port: &ResolvedPort,
    edge: &ResolvedEdgeContract,
    first_mask: &BTreeSet<HexCoord>,
    second_mask: &BTreeSet<HexCoord>,
    boundary_pairs: &BTreeSet<(HexCoord, HexCoord)>,
    footprint: &BTreeSet<HexCoord>,
) -> bool {
    let lanes_are_oriented = port.lanes.iter().all(|(first, second)| {
        if kind == LayoutKind::Macro {
            HexSide::ALL
                .into_iter()
                .any(|side| side.neighbor(*first) == *second)
        } else {
            edge.first.1.neighbor(*first) == *second
        }
    });
    if port.lanes.is_empty() || !port.lanes.is_subset(boundary_pairs) || !lanes_are_oriented {
        return false;
    }
    let first_boundary: BTreeSet<_> = port.lanes.iter().map(|(coord, _)| *coord).collect();
    let second_boundary: BTreeSet<_> = port.lanes.iter().map(|(_, coord)| *coord).collect();
    let approaches_match = if kind == LayoutKind::Macro {
        macro_approach_corridors(&port.lanes, first_mask, second_mask, edge.approach_depth)
            .is_some_and(|(first, second)| {
                port.first_approach == first && port.second_approach == second
            })
            || (valid_authored_macro_approach(
                &port.first_approach,
                &first_boundary,
                first_mask,
                edge.approach_depth,
            ) && valid_authored_macro_approach(
                &port.second_approach,
                &second_boundary,
                second_mask,
                edge.approach_depth,
            ))
    } else {
        approach_corridor(
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
    };
    first_boundary.len() == port.lanes.len()
        && second_boundary.len() == port.lanes.len()
        && first_boundary
            .iter()
            .all(|coord| !lane_touches_third_patch(*coord, first_mask, second_mask, footprint))
        && second_boundary
            .iter()
            .all(|coord| !lane_touches_third_patch(*coord, second_mask, first_mask, footprint))
        && ordered_simple_seam_lanes(&port.lanes).is_some()
        && approaches_match
        && !approach_touches_other_patch(
            &port.first_approach,
            &first_boundary,
            first_mask,
            footprint,
        )
        && !approach_touches_other_patch(
            &port.second_approach,
            &second_boundary,
            second_mask,
            footprint,
        )
}

fn valid_authored_macro_approach(
    approach: &BTreeSet<HexCoord>,
    boundary: &BTreeSet<HexCoord>,
    mask: &BTreeSet<HexCoord>,
    depth: u32,
) -> bool {
    let expected_len = usize::try_from(depth)
        .ok()
        .and_then(|depth| boundary.len().checked_mul(depth));
    !approach.is_empty()
        && expected_len == Some(approach.len())
        && boundary.is_subset(approach)
        && approach.is_subset(mask)
        && connected(approach)
        && approach.iter().all(|coord| {
            boundary
                .iter()
                .map(|boundary| coord.distance(*boundary))
                .min()
                .is_some_and(|distance| distance < depth)
        })
}

/// One deterministic resolved-layout contract failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LayoutIssue {
    UnsupportedRadius(u32),
    InvalidFootprint,
    FootprintOutOfBounds,
    InvalidRingRadius(u32),
    InvalidRingFootprint,
    InvalidRing19Settings(String),
    InvalidMacroRadius(u32),
    InvalidMacroFootprint,
    InvalidMacroPatchCount(usize),
    InvalidMacroAtomicGeometry,
    InvalidMacroAtomicCell(HexCoord),
    InvalidMacroSettings(String),
    InvalidMacroEdgeCoverage(PatchId, PatchId),
    PatchCount { expected: usize, actual: usize },
    SharedEdgeCount { expected: usize, actual: usize },
    BoundarySideCount { expected: usize, actual: usize },
    DisconnectedPatch(PatchId),
    PatchOutsideFootprint(PatchId),
    OverlappingPatch(PatchId, HexCoord),
    DuplicateBiomeRegion(BiomeRegionId),
    InvalidRing19PatchIdentity(PatchId, BiomeRegionId),
    InvalidPatchRotation(PatchId, u8),
    IncompletePatchEdges(PatchId),
    IncompleteCoverage,
    SharedReferenceMismatch(ResolvedEdgeId),
    MissingSharedEdge(ResolvedEdgeId),
    MissingEdgePatch(ResolvedEdgeId, PatchId),
    InvalidBoundaryPairs(ResolvedEdgeId),
    InvalidProtectedApproach(ResolvedEdgeId, PatchId),
    InvalidResolvedContract(ResolvedEdgeId),
    MissingBoundaryLiquidOutlet,
    InvalidBoundaryLiquidOutlet(PatchId, HexSide),
    MultipleLiquidOutlets(PatchId),
    LiquidComponentWithoutBoundary(PatchId),
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
    NonSimpleOrientedSeam(PatchId, PatchId),
    AmbiguousPortApproaches(PatchId, PatchId),
    InvalidPortRequest(PatchId, PatchId),
    InsufficientPortCapacity(PatchId, PatchId),
    CyclicLiquidGraph,
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
        CubeCoord, EdgeElevationSettings, EdgeLiquidPortSettings, MapSettings, PatchEdgesSettings,
        ProceduralSettings, Ring19BoundaryOutletSettings, Ring19LiquidConnectionSettings,
        Ring19RegionSettings, TerrainSettings, V3CavesSettings, V3CrystalAscentSettings,
        V3EnvironmentSettings, V3ForestSettings, V3FortSettings, V3HillsSettings,
        V3MountainsSettings, V3RecipeSettings, V3Ring19Settings, V3SkyIslandsSettings,
        V3WaterfallSettings, WalkerPortSettings,
    };

    const MOUNTAIN_RANGE_RON: &str =
        include_str!("../../../../assets/config/worlds/procedural-mountain-range.ron");
    const CRYSTAL_MOUNTAIN_RON: &str =
        include_str!("../../../../assets/config/worlds/procedural-crystal-mountain.ron");

    fn mountain_range_settings() -> ProceduralV3Settings {
        let settings: MapSettings =
            ron::from_str(MOUNTAIN_RANGE_RON).expect("shipped Mountain Range settings parse");
        let TerrainSettings::Procedural(ProceduralSettings::V3(settings)) = settings.terrain else {
            panic!("shipped Mountain Range settings should use procedural V3");
        };
        settings
    }

    fn crystal_mountain_settings() -> ProceduralV3Settings {
        let settings: MapSettings =
            ron::from_str(CRYSTAL_MOUNTAIN_RON).expect("shipped Crystal Mountain settings parse");
        let TerrainSettings::Procedural(ProceduralSettings::V3(settings)) = settings.terrain else {
            panic!("shipped Crystal Mountain settings should use procedural V3");
        };
        settings
    }

    #[test]
    fn schematic_local_owner_candidates_match_the_canonical_full_search() {
        let pitch = 22_i32;
        let coarse = hex_schematic::canonical_coordinates()
            .into_iter()
            .enumerate()
            .map(|(index, coord)| {
                let id = PatchId(u32::try_from(index).expect("217 ids fit u32"));
                let coarse_coord = HexCoord::from_axial(coord.q(), coord.r());
                let center = HexCoord::from_axial(coord.q() * pitch, coord.r() * pitch);
                (coarse_coord, (id, center))
            })
            .collect::<BTreeMap<_, _>>();
        let candidate_offsets = HexCoord::ORIGIN.within_radius(2);
        for coord in HexCoord::ORIGIN.within_radius(V3_SCHEMATIC_GRID_RADIUS) {
            let local =
                nearest_schematic_owner(coord, pitch, &coarse, candidate_offsets.as_slice());
            let exhaustive = coarse
                .values()
                .copied()
                .min_by_key(|(id, center)| (center.distance(coord), id.0))
                .map(|(id, _)| id);
            assert_eq!(
                local, exhaustive,
                "radius-two candidate neighborhood changed canonical owner at {coord:?}"
            );
        }
    }

    fn crystal_ascent_macro_settings() -> ProceduralV3Settings {
        let mut settings = mountain_range_settings();
        let V3LayoutSettings::Macro(layout) = &mut settings.layout else {
            unreachable!("Mountain Range helper must return a Macro layout");
        };
        let central_cells = HexCoord::ORIGIN
            .within_radius(1)
            .into_iter()
            .map(|coord| {
                let [x, y, z] = coord.to_cubic_array();
                CubeCoord { x, y, z }
            })
            .collect::<Vec<_>>();
        let central_tuples = central_cells
            .iter()
            .map(|coord| (coord.x, coord.y, coord.z))
            .collect::<BTreeSet<_>>();
        for instance in &mut layout.instances {
            instance
                .cells
                .retain(|cell| !central_tuples.contains(&(cell.x, cell.y, cell.z)));
        }
        layout
            .instances
            .retain(|instance| instance.name == "hills-center" || !instance.cells.is_empty());
        let crystal = layout
            .instances
            .iter_mut()
            .find(|instance| instance.name == "hills-center")
            .expect("the fixture retains the central instance");
        crystal.cells = central_cells;
        crystal.recipe = V3RecipeSettings::CrystalAscent(V3CrystalAscentSettings {
            base_level: 6,
            rise_levels: 144,
        });
        crystal.elevation.low = 6;
        crystal.elevation.high = 150;
        layout.liquid_connections.clear();
        layout.headwaters.clear();
        layout.walker_connections.clear();
        layout.spanning_features.clear();
        layout.anchor_aliases.clear();
        layout.critical_route = vec![
            "hills-lower-outer".to_owned(),
            "mountains-tier1-lower-outer".to_owned(),
        ];
        settings
    }

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
                    surface_level: 17,
                    cave_floor_level: 6,
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

    fn ring19_settings() -> ProceduralV3Settings {
        let region = Ring19RegionSettings {
            environment: V3EnvironmentSettings::TemperateGrassland,
            recipe: V3RecipeSettings::Hills(V3HillsSettings {
                valley_level: 17,
                max_relief: 12,
                hills_per_bank: 3,
            }),
            overlays: Vec::new(),
            rotation_turns: 0,
        };
        let mut regions = vec![region; V3_RING19_REGION_COUNT];
        for (slot, rotation) in [(2, 4), (6, 5), (10, 4), (12, 5), (15, 3)] {
            regions
                .get_mut(slot)
                .expect("fixed Ring19 rotation slot")
                .rotation_turns = rotation;
        }
        let liquid_connections = [
            (16, 5, 29),
            (5, 0, 16),
            (17, 6, 29),
            (6, 0, 16),
            (18, 1, 16),
            (1, 0, 16),
            (0, 4, 16),
            (4, 12, 16),
        ]
        .into_iter()
        .map(
            |(source_region, sink_region, level)| Ring19LiquidConnectionSettings {
                source_region,
                sink_region,
                width: 3,
                level,
            },
        )
        .collect();
        ProceduralV3Settings {
            layout: V3LayoutSettings::Ring19(V3Ring19Settings {
                profile: crate::settings::V3Ring19ProfileSettings::TwoRings,
                regions,
                seam_defaults: SharedEdgeSettings {
                    elevation: EdgeElevationSettings {
                        preferred: 17,
                        min: 16,
                        max: 18,
                    },
                    walker: WalkerPortSettings { count: 2, width: 2 },
                    liquid: EdgeLiquidSettings::Dry,
                    approach_depth: 3,
                },
                liquid_connections,
                boundary_outlets: vec![
                    Ring19BoundaryOutletSettings {
                        source_region: 12,
                        side: Ring19BoundarySide::SouthEast,
                        width: 3,
                        level: 3,
                    },
                    Ring19BoundaryOutletSettings {
                        source_region: 15,
                        side: Ring19BoundarySide::West,
                        width: 3,
                        level: 14,
                    },
                ],
            }),
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

    fn set_liquid_flow(
        ring: &mut V3Ring7Settings,
        source: PatchId,
        source_side: HexSide,
        sink: PatchId,
        sink_side: HexSide,
        width: u32,
    ) {
        set_edge(
            &mut spec_mut(ring, source).edges,
            source_side,
            shared(EdgeLiquidSettings::Outlet(EdgeLiquidPortSettings { width })),
        );
        set_edge(
            &mut spec_mut(ring, sink).edges,
            sink_side,
            shared(EdgeLiquidSettings::Inlet(EdgeLiquidPortSettings { width })),
        );
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
    fn mountain_range_collapses_atomic_cells_into_exact_logical_seams() {
        let settings = mountain_range_settings();
        let first = resolve_layout(MACRO_RADIUS, &settings).expect("valid Mountain Range layout");
        let second =
            resolve_layout(MACRO_RADIUS, &settings).expect("deterministic Mountain Range layout");

        assert_eq!(first, second);
        assert_eq!(first.kind, LayoutKind::Macro);
        assert_eq!(first.grid_radius, 77);
        assert_eq!(first.footprint.len(), 18_019);
        assert_eq!(first.patches.len(), 30);
        assert_eq!(first.shared_edges.len(), 74);
        assert!(first.boundary_liquid_outlets.is_empty());
        assert!(first.validate().is_ok());

        assert_eq!(
            first
                .patches
                .values()
                .map(|patch| patch.mask.len())
                .sum::<usize>(),
            first.footprint.len(),
            "logical instance masks must cover the world exactly once"
        );
        assert_eq!(
            first
                .patches
                .values()
                .map(|patch| patch.biome_region)
                .collect::<BTreeSet<_>>()
                .len(),
            30,
            "every logical instance publishes one opaque biome-region identity"
        );
        assert!(first.patches.values().all(|patch| connected(&patch.mask)));

        let logical_pairs = first
            .shared_edges
            .values()
            .map(|edge| ordered_patch_pair(edge.first.0, edge.second.0))
            .collect::<BTreeSet<_>>();
        assert_eq!(logical_pairs.len(), 74);
        let standing = first
            .shared_edges
            .values()
            .filter_map(|edge| match &edge.liquid {
                ResolvedLiquidPort::Standing { port, elevation } => Some((edge, port, elevation)),
                ResolvedLiquidPort::Dry | ResolvedLiquidPort::Directed { .. } => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(standing.len(), 9);
        for (edge, port, elevation) in standing {
            assert_eq!(*elevation, ResolvedLiquidElevation::Exact(8));
            assert_eq!(
                port.lanes, edge.boundary_pairs,
                "width-zero standing water must resolve the complete broad boundary without a current"
            );
        }
        assert_eq!(
            first
                .shared_edges
                .values()
                .filter(|edge| matches!(edge.liquid, ResolvedLiquidPort::Directed { .. }))
                .count(),
            6
        );
        let V3LayoutSettings::Macro(macro_settings) = &settings.layout else {
            unreachable!("Mountain Range helper must return a Macro layout");
        };
        for edge in first.shared_edges.values() {
            let PatchId(first_id) = edge.first.0;
            let PatchId(second_id) = edge.second.0;
            let first_index = usize::try_from(first_id).expect("patch id fits usize");
            let second_index = usize::try_from(second_id).expect("patch id fits usize");
            let first_instance = macro_settings
                .instances
                .get(first_index)
                .expect("first patch id should name a Macro instance");
            let second_instance = macro_settings
                .instances
                .get(second_index)
                .expect("second patch id should name a Macro instance");
            let touches_deep = [&first_instance.recipe, &second_instance.recipe]
                .into_iter()
                .any(|recipe| matches!(recipe, V3RecipeSettings::DeepMountain(_)));
            if touches_deep {
                let other = if matches!(&first_instance.recipe, V3RecipeSettings::DeepMountain(_)) {
                    second_instance
                } else {
                    first_instance
                };
                let expected = if other.elevation.high == 34 { 41 } else { 48 };
                assert_eq!(
                    edge.elevation.preferred, expected,
                    "Deep Mountain side spurs must meet first-tier Mountains at 41 while its upper front remains at 48"
                );
            }
            let tier_step = (first_instance.elevation.high == 34
                && second_instance.elevation.low == 34)
                || (second_instance.elevation.high == 34 && first_instance.elevation.low == 34);
            if tier_step
                && matches!(&first_instance.recipe, V3RecipeSettings::Mountains(_))
                && matches!(&second_instance.recipe, V3RecipeSettings::Mountains(_))
            {
                assert_eq!(
                    edge.elevation.preferred, 34,
                    "every first-to-second mountain-tier segment must share level 34"
                );
            }
        }
        assert!(first.shared_edges.values().all(|edge| {
            first
                .patches
                .get(&edge.first.0)
                .zip(first.patches.get(&edge.second.0))
                .is_some_and(|(first_patch, second_patch)| {
                    edge.boundary_pairs == boundary_pairs(&first_patch.mask, &second_patch.mask)
                })
        }));

        let segments_by_logical_side = first
            .shared_edges
            .values()
            .flat_map(|edge| [edge.first, edge.second])
            .fold(BTreeMap::<_, usize>::new(), |mut counts, logical_side| {
                *counts.entry(logical_side).or_default() += 1;
                counts
            });
        assert!(
            segments_by_logical_side.values().any(|count| *count > 1),
            "one logical compass side must be able to retain several region-pair segments"
        );
    }

    #[test]
    fn crystal_ascent_claims_radius_32_before_macro_seams_are_resolved() {
        let settings = crystal_ascent_macro_settings();
        let V3LayoutSettings::Macro(macro_settings) = &settings.layout else {
            unreachable!("Crystal Ascent fixture must use Macro");
        };
        let crystal_index = macro_settings
            .instances
            .iter()
            .position(|instance| matches!(instance.recipe, V3RecipeSettings::CrystalAscent(_)))
            .expect("fixture contains Crystal Ascent");
        let crystal_settings = macro_settings
            .instances
            .get(crystal_index)
            .expect("resolved Crystal Ascent index exists");
        assert_eq!(
            crystal_settings
                .cells
                .iter()
                .map(|cell| (cell.x, cell.y, cell.z))
                .collect::<BTreeSet<_>>(),
            HexCoord::ORIGIN
                .within_radius(1)
                .into_iter()
                .map(|coord| {
                    let [x, y, z] = coord.to_cubic_array();
                    (x, y, z)
                })
                .collect(),
            "the logical landmark owns exactly Macro's central seven atomic cells"
        );

        let first = resolve_layout(MACRO_RADIUS, &settings)
            .expect("the radius-32 landmark claim should preserve a valid Macro layout");
        let second = resolve_layout(MACRO_RADIUS, &settings)
            .expect("the radius-32 landmark claim should be deterministic");
        assert_eq!(first, second);
        assert!(first.validate().is_ok());
        assert_eq!(first.footprint.len(), 18_019);
        let crystal_id = PatchId(u32::try_from(crystal_index).expect("fixture index fits u32"));
        let crystal_mask = &first
            .patches
            .get(&crystal_id)
            .expect("Crystal Ascent patch resolves")
            .mask;
        let exact_site = HexCoord::ORIGIN
            .within_radius(CRYSTAL_ASCENT_SITE_RADIUS)
            .into_iter()
            .collect::<BTreeSet<_>>();
        assert!(
            exact_site.is_subset(crystal_mask),
            "every authored radius-32 site column belongs to Crystal Ascent"
        );
        assert!(
            first.patches.values().all(|patch| connected(&patch.mask)),
            "the landmark claim must leave every donor mask connected and nonempty"
        );
        assert_eq!(
            first
                .patches
                .values()
                .map(|patch| patch.mask.len())
                .sum::<usize>(),
            18_019,
            "the landmark claim preserves all 18,019 columns exactly once"
        );
        let covered = first
            .patches
            .values()
            .flat_map(|patch| patch.mask.iter().copied())
            .collect::<BTreeSet<_>>();
        assert_eq!(covered, first.footprint);
        assert!(first.shared_edges.values().all(|edge| {
            let first_patch = first
                .patches
                .get(&edge.first.0)
                .expect("edge first patch exists");
            let second_patch = first
                .patches
                .get(&edge.second.0)
                .expect("edge second patch exists");
            edge.boundary_pairs == boundary_pairs(&first_patch.mask, &second_patch.mask)
        }));

        let mut invalid = settings.clone();
        let V3LayoutSettings::Macro(invalid_macro) = &mut invalid.layout else {
            unreachable!("invalid fixture remains Macro");
        };
        invalid_macro
            .instances
            .iter_mut()
            .find(|instance| matches!(instance.recipe, V3RecipeSettings::CrystalAscent(_)))
            .expect("invalid fixture retains Crystal Ascent")
            .cells
            .pop();
        let error = resolve_layout(MACRO_RADIUS, &invalid)
            .expect_err("a remote or partial atomic claim must fail closed");
        assert!(
            error
                .to_string()
                .contains("must own exactly Macro's central seven atomic cells"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn explicit_macro_walker_reuses_shared_edge_while_tunnel_ports_stay_subsurface() {
        let settings = crystal_mountain_settings();

        let layout = resolve_layout(MACRO_RADIUS, &settings)
            .expect("shipped Crystal Mountain Macro contracts resolve");
        let V3LayoutSettings::Macro(macro_settings) = &settings.layout else {
            unreachable!("Crystal Mountain fixture must remain Macro");
        };
        let contracts = resolve_macro_contracts(macro_settings, &layout)
            .expect("resolved extension contracts match the final masks");
        let walker = contracts
            .walker_connections
            .first()
            .expect("one explicit surface walker resolves");
        assert_eq!(walker.level, 150);
        assert_eq!(walker.port.lanes.len(), 4);
        let matching_edge = layout
            .shared_edges
            .values()
            .find(|edge| {
                ordered_patch_pair(edge.first.0, edge.second.0)
                    == ordered_patch_pair(walker.first, walker.second)
            })
            .expect("the explicit surface walker has one shared edge");
        assert_eq!(matching_edge.walker.count, 1);
        assert_eq!(matching_edge.walker.width, 4);
        assert_eq!(
            matching_edge.elevation,
            ResolvedElevationBand {
                preferred: 150,
                min: 150,
                max: 150,
            }
        );
        assert_eq!(matching_edge.walker.ports.first(), Some(&walker.port));

        let ResolvedMacroSpanningFeature::Tunnel(tunnel) = contracts
            .spanning_features
            .get("crystal_mountain.tunnel")
            .expect("the named tunnel resolves");
        assert_eq!(tunnel.seams.len(), 2);
        assert!(tunnel.seams.iter().all(|seam| seam.port.lanes.len() == 4));
        for seam in &tunnel.seams {
            let edge = layout
                .shared_edges
                .values()
                .find(|edge| {
                    ordered_patch_pair(edge.first.0, edge.second.0)
                        == ordered_patch_pair(seam.source, seam.destination)
                })
                .expect("each tunnel crossing also has a physical instance seam");
            assert!(
                edge.walker.ports.is_empty()
                    || edge.walker.ports.iter().all(|port| port != &seam.port),
                "subsurface tunnel lanes remain separate from ordinary surface ports"
            );
        }
        assert_eq!(tunnel.floor_level, 6);
        assert_eq!(tunnel.width, 4);
        assert_eq!(tunnel.clearance, 6);
        assert_eq!(tunnel.roof_thickness, 3);
        assert_eq!(tunnel.boundary_terminal.lanes.len(), 4);
        assert_eq!(
            tunnel.destination_anchor.anchor,
            "crystal_ascent.lower_entry"
        );
        assert_eq!(
            contracts
                .anchor_aliases
                .get("crystal_mountain.ascent_threshold")
                .expect("stable alias resolves")
                .instance,
            tunnel.destination_anchor.instance
        );
    }

    #[test]
    fn ring19_resolves_exact_masks_topology_hydrology_and_outlets() {
        let settings = ring19_settings();
        let first = resolve_layout(RING19_RADIUS, &settings).expect("fixed Ring19 layout");
        let second = resolve_layout(RING19_RADIUS, &settings).expect("deterministic Ring19 layout");
        assert_eq!(first, second);
        let mut reordered = settings.clone();
        let V3LayoutSettings::Ring19(reordered_ring) = &mut reordered.layout else {
            unreachable!("fixed Ring19 settings");
        };
        reordered_ring.liquid_connections.reverse();
        reordered_ring.boundary_outlets.reverse();
        assert_eq!(
            first,
            resolve_layout(RING19_RADIUS, &reordered)
                .expect("Ring19 graph list order is not semantic")
        );
        assert_eq!(first.footprint.len(), 9_241);
        assert_eq!(first.patches.len(), V3_RING19_REGION_COUNT);
        assert_eq!(first.shared_edges.len(), 42);
        assert_eq!(
            first
                .patches
                .values()
                .map(|patch| patch.mask.len())
                .collect::<Vec<_>>(),
            [
                505, 498, 491, 491, 491, 491, 484, 486, 488, 477, 488, 477, 488, 477, 488, 477,
                488, 477, 479,
            ]
        );
        assert_eq!(
            first
                .patches
                .values()
                .flat_map(|patch| patch.edges.values())
                .filter(|edge| matches!(edge, ResolvedEdgeReference::WorldBoundary))
                .count(),
            30
        );
        assert_eq!(
            first
                .patches
                .values()
                .map(|patch| patch.rotation_turns)
                .collect::<Vec<_>>(),
            [0, 0, 4, 0, 0, 0, 5, 0, 0, 0, 4, 0, 5, 0, 0, 3, 0, 0, 0]
        );
        let actual_seams = first
            .shared_edges
            .iter()
            .map(|(id, edge)| {
                (
                    id.0,
                    (edge.first.0).0,
                    edge.first.1,
                    (edge.second.0).0,
                    edge.second.1,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            actual_seams,
            vec![
                (0, 0, HexSide::NorthEast, 1, HexSide::SouthWest),
                (1, 0, HexSide::East, 2, HexSide::West),
                (2, 0, HexSide::SouthEast, 3, HexSide::NorthWest),
                (3, 0, HexSide::SouthWest, 4, HexSide::NorthEast),
                (4, 0, HexSide::West, 5, HexSide::East),
                (5, 0, HexSide::NorthWest, 6, HexSide::SouthEast),
                (6, 1, HexSide::SouthEast, 2, HexSide::NorthWest),
                (7, 1, HexSide::West, 6, HexSide::East),
                (8, 1, HexSide::NorthEast, 7, HexSide::SouthWest),
                (9, 1, HexSide::East, 8, HexSide::West),
                (10, 1, HexSide::NorthWest, 18, HexSide::SouthEast),
                (11, 2, HexSide::SouthWest, 3, HexSide::NorthEast),
                (12, 2, HexSide::NorthEast, 8, HexSide::SouthWest),
                (13, 2, HexSide::East, 9, HexSide::West),
                (14, 2, HexSide::SouthEast, 10, HexSide::NorthWest),
                (15, 3, HexSide::West, 4, HexSide::East),
                (16, 3, HexSide::East, 10, HexSide::West),
                (17, 3, HexSide::SouthEast, 11, HexSide::NorthWest),
                (18, 3, HexSide::SouthWest, 12, HexSide::NorthEast),
                (19, 4, HexSide::NorthWest, 5, HexSide::SouthEast),
                (20, 4, HexSide::SouthEast, 12, HexSide::NorthWest),
                (21, 4, HexSide::SouthWest, 13, HexSide::NorthEast),
                (22, 4, HexSide::West, 14, HexSide::East),
                (23, 5, HexSide::NorthEast, 6, HexSide::SouthWest),
                (24, 5, HexSide::SouthWest, 14, HexSide::NorthEast),
                (25, 5, HexSide::West, 15, HexSide::East),
                (26, 5, HexSide::NorthWest, 16, HexSide::SouthEast),
                (27, 6, HexSide::West, 16, HexSide::East),
                (28, 6, HexSide::NorthWest, 17, HexSide::SouthEast),
                (29, 6, HexSide::NorthEast, 18, HexSide::SouthWest),
                (30, 7, HexSide::SouthEast, 8, HexSide::NorthWest),
                (31, 7, HexSide::West, 18, HexSide::East),
                (32, 8, HexSide::SouthEast, 9, HexSide::NorthWest),
                (33, 9, HexSide::SouthWest, 10, HexSide::NorthEast),
                (34, 10, HexSide::SouthWest, 11, HexSide::NorthEast),
                (35, 11, HexSide::West, 12, HexSide::East),
                (36, 12, HexSide::West, 13, HexSide::East),
                (37, 13, HexSide::NorthWest, 14, HexSide::SouthEast),
                (38, 14, HexSide::NorthWest, 15, HexSide::SouthEast),
                (39, 15, HexSide::NorthEast, 16, HexSide::SouthWest),
                (40, 16, HexSide::NorthEast, 17, HexSide::SouthWest),
                (41, 17, HexSide::East, 18, HexSide::West),
            ]
        );
        let actual_boundary_sides = first
            .patches
            .iter()
            .flat_map(|(id, patch)| {
                patch.edges.iter().filter_map(move |(side, reference)| {
                    matches!(reference, ResolvedEdgeReference::WorldBoundary)
                        .then_some((id.0, *side))
                })
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(
            actual_boundary_sides,
            BTreeSet::from([
                (7, HexSide::East),
                (7, HexSide::NorthWest),
                (7, HexSide::NorthEast),
                (8, HexSide::East),
                (8, HexSide::NorthEast),
                (9, HexSide::East),
                (9, HexSide::SouthEast),
                (9, HexSide::NorthEast),
                (10, HexSide::East),
                (10, HexSide::SouthEast),
                (11, HexSide::East),
                (11, HexSide::SouthEast),
                (11, HexSide::SouthWest),
                (12, HexSide::SouthEast),
                (12, HexSide::SouthWest),
                (13, HexSide::SouthEast),
                (13, HexSide::SouthWest),
                (13, HexSide::West),
                (14, HexSide::SouthWest),
                (14, HexSide::West),
                (15, HexSide::SouthWest),
                (15, HexSide::West),
                (15, HexSide::NorthWest),
                (16, HexSide::West),
                (16, HexSide::NorthWest),
                (17, HexSide::West),
                (17, HexSide::NorthWest),
                (17, HexSide::NorthEast),
                (18, HexSide::NorthWest),
                (18, HexSide::NorthEast),
            ])
        );

        let actual_liquid = first
            .shared_edges
            .values()
            .filter_map(|edge| {
                let ResolvedLiquidPort::Directed {
                    source,
                    sink,
                    elevation: ResolvedLiquidElevation::Exact(level),
                    ..
                } = edge.liquid
                else {
                    return None;
                };
                Some((source.0, sink.0, level))
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(
            actual_liquid,
            BTreeSet::from([
                (16, 5, 29),
                (5, 0, 16),
                (17, 6, 29),
                (6, 0, 16),
                (18, 1, 16),
                (1, 0, 16),
                (0, 4, 16),
                (4, 12, 16),
            ])
        );

        let water = first
            .boundary_liquid_outlets
            .get(&(PatchId(12), HexSide::SouthEast))
            .expect("slot 12 water outlet");
        assert_eq!(water.level, 3);
        assert_eq!(
            water
                .lanes
                .iter()
                .map(|(inside, _)| *inside)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                HexCoord::new_cubic(-23, 55, -32),
                HexCoord::new_cubic(-22, 55, -33),
                HexCoord::new_cubic(-21, 55, -34),
            ])
        );
        let lava = first
            .boundary_liquid_outlets
            .get(&(PatchId(15), HexSide::West))
            .expect("slot 15 lava outlet");
        assert_eq!(lava.level, 14);
        assert_eq!(
            lava.lanes
                .iter()
                .map(|(inside, _)| *inside)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                HexCoord::new_cubic(-55, 0, 55),
                HexCoord::new_cubic(-55, 1, 54),
                HexCoord::new_cubic(-54, -1, 55),
            ])
        );
        for outlet in first.boundary_liquid_outlets.values() {
            let source_mask = &first
                .patches
                .get(&outlet.source)
                .expect("outlet source patch")
                .mask;
            assert!(outlet.inward_approach.is_subset(source_mask));
            assert!(outlet.lanes.iter().all(|(inside, outside)| {
                source_mask.contains(inside)
                    && !first.footprint.contains(outside)
                    && *outside == outlet.side.neighbor(*inside)
            }));
        }
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
            assert_eq!(edge.walker.count, 2);
            assert_eq!(edge.walker.width, 2);
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
            let [first, second] = edge.walker.ports.as_slice() else {
                panic!("the test seam should contain exactly two walker ports");
            };
            assert!(ports_are_disjoint(first, second));
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
        let ResolvedLiquidPort::Directed {
            source, sink, port, ..
        } = &edge.liquid
        else {
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
            shared.walker.count = 4;
            shared.walker.width = 4;
        }

        let error = resolve_layout(33, &settings)
            .expect_err("four four-wide ports cannot fit a fifteen-lane seam");
        assert!(error
            .issues()
            .contains(&LayoutIssue::InsufficientPortCapacity(
                PatchId(0),
                PatchId(2)
            )));
    }

    #[test]
    fn deleted_resolved_walker_port_is_rejected() {
        let mut resolved = resolve_layout(33, &ring_settings()).expect("valid Ring7 layout");
        let edge = resolved
            .shared_edges
            .get_mut(&ResolvedEdgeId(0))
            .expect("the first fixed seam exists");
        edge.walker.ports.pop();

        let error = resolved
            .validate()
            .expect_err("the exact ports must retain the requested count");
        assert!(error
            .issues()
            .contains(&LayoutIssue::InvalidResolvedContract(ResolvedEdgeId(0))));
    }

    #[test]
    fn nonuniform_resolved_walker_port_is_rejected() {
        let mut resolved = resolve_layout(33, &ring_settings()).expect("valid Ring7 layout");
        let edge = resolved
            .shared_edges
            .get_mut(&ResolvedEdgeId(0))
            .expect("the first fixed seam exists");
        let Some(port) = edge.walker.ports.first_mut() else {
            panic!("the test seam should contain a walker port");
        };
        let Some(lane) = port.lanes.first().copied() else {
            panic!("walker ports should be non-empty");
        };
        port.lanes.remove(&lane);

        let error = resolved
            .validate()
            .expect_err("a one-lane resolved walker port is corrupt");
        assert!(error
            .issues()
            .contains(&LayoutIssue::InvalidResolvedContract(ResolvedEdgeId(0))));
    }

    #[test]
    fn resolved_elevation_bounds_are_rechecked_after_resolution() {
        let resolved = resolve_layout(33, &ring_settings()).expect("valid Ring7 layout");
        let corruptions: [(&str, fn(&mut ResolvedElevationBand)); 2] = [
            ("negative minimum", |elevation| elevation.min = -1),
            ("maximum above the procedural ceiling", |elevation| {
                elevation.max = MAX_V3_LEVEL + 1
            }),
        ];
        for (label, corrupt) in corruptions {
            let mut corrupted = resolved.clone();
            let edge = corrupted
                .shared_edges
                .get_mut(&ResolvedEdgeId(0))
                .expect("the first fixed seam exists");
            corrupt(&mut edge.elevation);

            let error = corrupted
                .validate()
                .expect_err("resolved elevation corruption must fail closed");
            assert!(
                error
                    .issues()
                    .contains(&LayoutIssue::InvalidResolvedContract(ResolvedEdgeId(0))),
                "{label} was not rejected: {error}"
            );
        }
    }

    #[test]
    fn oversized_resolved_liquid_port_is_rejected() {
        let mut settings = ring_settings();
        let V3LayoutSettings::Ring7(ring) = &mut settings.layout else {
            unreachable!();
        };
        set_liquid_flow(
            ring,
            PatchId(0),
            HexSide::East,
            PatchId(2),
            HexSide::West,
            MAX_SEAM_PORT_WIDTH,
        );
        let mut resolved = resolve_layout(33, &settings).expect("bounded liquid port resolves");
        let edge_id = ResolvedEdgeId(1);
        let edge = resolved
            .shared_edges
            .get(&edge_id)
            .expect("center/waterfall seam exists");
        let first_mask = resolved
            .patches
            .get(&edge.first.0)
            .expect("first seam patch exists")
            .mask
            .clone();
        let second_mask = resolved
            .patches
            .get(&edge.second.0)
            .expect("second seam patch exists")
            .mask
            .clone();
        let oriented: BTreeSet<_> = edge
            .boundary_pairs
            .iter()
            .copied()
            .filter(|(first, second)| edge.first.1.neighbor(*first) == *second)
            .collect();
        let ordered =
            ordered_simple_seam_lanes(&oriented).expect("the generated seam is a simple path");
        let wide_port = port_candidates(
            &ordered,
            &first_mask,
            &second_mask,
            edge.first.1,
            edge.second.1,
            MAX_SEAM_PORT_WIDTH + 1,
            edge.approach_depth,
            &resolved.footprint,
        )
        .into_iter()
        .map(|candidate| candidate.port)
        .find(|candidate| {
            edge.walker
                .ports
                .iter()
                .all(|walker| ports_are_disjoint(walker, candidate))
        })
        .expect("the seam has room for an otherwise-valid five-lane corruption");
        let ResolvedLiquidPort::Directed { source, sink, .. } = &edge.liquid else {
            panic!("the test seam should have directed liquid");
        };
        let (source, sink) = (*source, *sink);

        let edge = resolved
            .shared_edges
            .get_mut(&edge_id)
            .expect("center/waterfall seam exists");
        edge.liquid = ResolvedLiquidPort::Directed {
            source,
            sink,
            port: wide_port,
            elevation: ResolvedLiquidElevation::EdgeBand,
        };
        let all_ports: Vec<_> = edge
            .walker
            .ports
            .iter()
            .chain(liquid_port_ref(&edge.liquid))
            .collect();
        edge.protected_approaches = BTreeMap::from([
            (
                edge.first.0,
                all_ports
                    .iter()
                    .flat_map(|port| port.first_approach.iter().copied())
                    .collect(),
            ),
            (
                edge.second.0,
                all_ports
                    .iter()
                    .flat_map(|port| port.second_approach.iter().copied())
                    .collect(),
            ),
        ]);

        let error = resolved
            .validate()
            .expect_err("resolved liquid width must retain the authored upper bound");
        assert!(error
            .issues()
            .contains(&LayoutIssue::InvalidResolvedContract(edge_id)));
    }

    #[test]
    fn cyclic_resolved_liquid_graph_is_rejected() {
        let mut settings = ring_settings();
        let V3LayoutSettings::Ring7(ring) = &mut settings.layout else {
            unreachable!();
        };
        set_liquid_flow(
            ring,
            PatchId(0),
            HexSide::East,
            PatchId(2),
            HexSide::West,
            2,
        );
        set_liquid_flow(
            ring,
            PatchId(2),
            HexSide::SouthWest,
            PatchId(3),
            HexSide::NorthEast,
            2,
        );
        set_liquid_flow(
            ring,
            PatchId(3),
            HexSide::NorthWest,
            PatchId(0),
            HexSide::SouthEast,
            2,
        );

        let error =
            resolve_layout(33, &settings).expect_err("patch-level liquid flow must be acyclic");
        assert!(error.issues().contains(&LayoutIssue::CyclicLiquidGraph));
    }

    #[test]
    fn branching_oriented_lane_graph_is_rejected_before_enumeration() {
        let center = HexCoord::ORIGIN;
        let first_cells = [
            center,
            HexSide::East.neighbor(center),
            HexSide::SouthWest.neighbor(center),
            HexSide::NorthWest.neighbor(center),
        ];
        let lanes = first_cells
            .into_iter()
            .map(|first| (first, HexSide::East.neighbor(first)))
            .collect();

        assert!(
            ordered_simple_seam_lanes(&lanes).is_none(),
            "a branch cannot be treated as a bounded one-dimensional seam"
        );
    }

    #[test]
    fn bounded_port_selection_finds_a_feasible_maximum_request() {
        let lanes: BTreeSet<_> = (-9..=9)
            .map(|offset| {
                let first = HexCoord::new_cubic(0, offset, -offset);
                (first, HexSide::East.neighbor(first))
            })
            .collect();
        let first_mask = lanes
            .iter()
            .map(|(first, _)| *first)
            .collect::<BTreeSet<_>>();
        let second_mask = lanes
            .iter()
            .map(|(_, second)| *second)
            .collect::<BTreeSet<_>>();
        let footprint = first_mask.union(&second_mask).copied().collect();
        let requests =
            vec![PortRequest::Walker(MAX_SEAM_PORT_WIDTH); usize::from(MAX_WALKER_PORT_COUNT)];

        let selected = select_ports(
            &requests,
            &lanes,
            &first_mask,
            &second_mask,
            HexSide::East,
            HexSide::West,
            1,
            &footprint,
            false,
        )
        .expect("four width-four ports exactly fit nineteen lanes with required gaps");
        assert_eq!(selected.len(), usize::from(MAX_WALKER_PORT_COUNT));
        assert!(selected
            .iter()
            .all(|port| port.lanes.len() == usize::try_from(MAX_SEAM_PORT_WIDTH).unwrap()));
        assert!(selected.iter().enumerate().all(|(index, port)| {
            selected
                .iter()
                .skip(index + 1)
                .all(|other| ports_are_disjoint(port, other))
        }));
    }

    #[test]
    fn exact_liquid_elevation_is_ring19_only_and_independent_of_walker_band() {
        let mut settings = ring_settings();
        let V3LayoutSettings::Ring7(ring) = &mut settings.layout else {
            unreachable!("the fixture is Ring7");
        };
        set_liquid_flow(
            ring,
            PatchId(0),
            HexSide::East,
            PatchId(2),
            HexSide::West,
            3,
        );
        let mut resolved = resolve_layout(33, &settings).expect("legacy directed seam resolves");
        let edge_id = ResolvedEdgeId(1);
        let edge = resolved
            .shared_edges
            .get_mut(&edge_id)
            .expect("center/waterfall seam");
        let ResolvedLiquidPort::Directed { elevation, .. } = &mut edge.liquid else {
            unreachable!("the fixture has directed liquid");
        };
        *elevation = ResolvedLiquidElevation::Exact(29);

        assert!(
            resolved.validate().is_err(),
            "Ring7 must reject exact-level liquid authority"
        );
        let edge = resolved
            .shared_edges
            .get(&edge_id)
            .expect("center/waterfall seam");
        let mut issues = Vec::new();
        validate_resolved_edge(
            LayoutKind::Ring19,
            edge_id,
            edge,
            &resolved.patches,
            &resolved.footprint,
            &mut issues,
        );
        assert!(
            issues.is_empty(),
            "Ring19 exact liquid level may sit outside the walker band: {issues:?}"
        );

        let mut legacy = edge.clone();
        let ResolvedLiquidPort::Directed { elevation, .. } = &mut legacy.liquid else {
            unreachable!("the fixture has directed liquid");
        };
        *elevation = ResolvedLiquidElevation::EdgeBand;
        let mut issues = Vec::new();
        validate_resolved_edge(
            LayoutKind::Ring19,
            edge_id,
            &legacy,
            &resolved.patches,
            &resolved.footprint,
            &mut issues,
        );
        assert!(
            issues.contains(&LayoutIssue::InvalidResolvedContract(edge_id)),
            "Ring19 must not fall back to a walker elevation band"
        );
    }

    #[test]
    fn ring7_rejects_injected_boundary_liquid_outlets() {
        let mut resolved = resolve_layout(33, &ring_settings()).expect("valid Ring7 layout");
        let source = PatchId(1);
        let side = HexSide::NorthWest;
        resolved.boundary_liquid_outlets.insert(
            (source, side),
            ResolvedBoundaryLiquidOutlet {
                source,
                side,
                lanes: BTreeSet::new(),
                inward_approach: BTreeSet::new(),
                approach_depth: 1,
                level: 3,
            },
        );

        let error = resolved
            .validate()
            .expect_err("Ring7 must not acquire complete-world boundary outlets");
        assert!(error
            .issues()
            .contains(&LayoutIssue::InvalidBoundaryLiquidOutlet(source, side)));
    }

    fn boundary_liquid_outlet_fixture() -> (ResolvedLayoutPlan, BTreeSet<HexCoord>) {
        let mask = HexCoord::ORIGIN
            .within_radius(2)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let inside = BTreeSet::from([HexCoord::from_axial(2, -1), HexCoord::from_axial(2, 0)]);
        let lanes = inside
            .iter()
            .copied()
            .map(|coord| (coord, HexSide::East.neighbor(coord)))
            .collect::<BTreeSet<_>>();
        let inward_approach = approach_corridor(&inside, &mask, HexSide::West, 2)
            .expect("the exact boundary approach fits");
        let edges = HexSide::ALL
            .into_iter()
            .map(|side| (side, ResolvedEdgeReference::WorldBoundary))
            .collect();
        let outlet = ResolvedBoundaryLiquidOutlet {
            source: PatchId(0),
            side: HexSide::East,
            lanes,
            inward_approach,
            approach_depth: 2,
            level: 16,
        };
        (
            ResolvedLayoutPlan {
                kind: LayoutKind::Ring19,
                grid_radius: 55,
                footprint: mask.clone(),
                patches: BTreeMap::from([(
                    PatchId(0),
                    ResolvedPatch {
                        biome_region: BiomeRegionId(0),
                        rotation_turns: 0,
                        mask,
                        edges,
                    },
                )]),
                shared_edges: BTreeMap::new(),
                boundary_liquid_outlets: BTreeMap::from([((PatchId(0), HexSide::East), outlet)]),
            },
            inside,
        )
    }

    #[test]
    fn boundary_liquid_outlet_geometry_and_reservations_are_exact() {
        let (mut layout, inside) = boundary_liquid_outlet_fixture();
        let mut issues = Vec::new();
        validate_boundary_liquid_outlets(&layout, &mut issues);
        assert!(
            issues.is_empty(),
            "exact outlet should validate: {issues:?}"
        );

        let mut missing = layout.clone();
        missing.boundary_liquid_outlets.clear();
        let mut issues = Vec::new();
        validate_boundary_liquid_outlets(&missing, &mut issues);
        assert!(
            issues.is_empty(),
            "a dry Ring19 does not need a boundary liquid outlet: {issues:?}"
        );

        let mut wet_missing =
            resolve_layout(55, &ring19_settings()).expect("canonical wet Ring19 resolves");
        wet_missing.boundary_liquid_outlets.clear();
        let mut issues = Vec::new();
        validate_boundary_liquid_outlets(&wet_missing, &mut issues);
        assert_eq!(issues, vec![LayoutIssue::MissingBoundaryLiquidOutlet]);

        let mut wrong_level = layout.clone();
        wrong_level
            .boundary_liquid_outlets
            .values_mut()
            .next()
            .expect("one outlet")
            .level = 2;
        let mut issues = Vec::new();
        validate_boundary_liquid_outlets(&wrong_level, &mut issues);
        assert!(issues.contains(&LayoutIssue::InvalidBoundaryLiquidOutlet(
            PatchId(0),
            HexSide::East
        )));

        let reserved_coord = *inside.first().expect("two boundary lanes");
        layout.shared_edges.insert(
            ResolvedEdgeId(99),
            ResolvedEdgeContract {
                first: (PatchId(0), HexSide::East),
                second: (PatchId(1), HexSide::West),
                elevation: ResolvedElevationBand {
                    preferred: 16,
                    min: 15,
                    max: 17,
                },
                walker: ResolvedWalkerPorts {
                    count: 0,
                    width: 0,
                    ports: Vec::new(),
                },
                liquid: ResolvedLiquidPort::Dry,
                approach_depth: 1,
                boundary_pairs: BTreeSet::new(),
                protected_approaches: BTreeMap::from([(
                    PatchId(0),
                    BTreeSet::from([reserved_coord]),
                )]),
            },
        );
        let mut issues = Vec::new();
        validate_boundary_liquid_outlets(&layout, &mut issues);
        assert!(
            issues.contains(&LayoutIssue::InvalidBoundaryLiquidOutlet(
                PatchId(0),
                HexSide::East
            )),
            "boundary liquid reservations must remain disjoint from seam reservations"
        );
    }

    #[test]
    fn boundary_liquid_outlet_lane_cannot_touch_another_patch() {
        let (mut layout, _) = boundary_liquid_outlet_fixture();
        layout.footprint.insert(HexCoord::from_axial(3, -2));

        let mut issues = Vec::new();
        validate_boundary_liquid_outlets(&layout, &mut issues);

        assert!(
            issues.contains(&LayoutIssue::InvalidBoundaryLiquidOutlet(
                PatchId(0),
                HexSide::East
            )),
            "a boundary outlet lane adjacent to another patch must fail revalidation"
        );
    }

    #[test]
    fn boundary_liquid_outlet_approach_cannot_touch_another_patch() {
        let (mut layout, _) = boundary_liquid_outlet_fixture();
        let foreign = HexCoord::from_axial(1, -2);
        assert!(
            layout
                .patches
                .get_mut(&PatchId(0))
                .expect("fixture source patch")
                .mask
                .remove(&foreign),
            "fixture foreign cell starts in the source mask"
        );

        let mut issues = Vec::new();
        validate_boundary_liquid_outlets(&layout, &mut issues);

        assert!(
            issues.contains(&LayoutIssue::InvalidBoundaryLiquidOutlet(
                PatchId(0),
                HexSide::East
            )),
            "a boundary outlet approach adjacent to another patch must fail revalidation"
        );
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
