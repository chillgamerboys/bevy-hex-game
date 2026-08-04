//! Recipe-facing view of one resolved V3 patch.
//!
//! Layout resolution owns masks and shared-edge contracts. Recipes consume this
//! projection so they cannot reinterpret a seam or accidentally use another patch's
//! seed namespace.

use std::collections::BTreeSet;

use hex_core::{BiomeRegionId, HexCoord, Level};

use super::layout::{
    ring19_patch_center, HexSide, LayoutKind, PatchId, ResolvedBoundaryLiquidOutlet,
    ResolvedEdgeContract, ResolvedEdgeId, ResolvedEdgeReference, ResolvedLayoutPlan,
    ResolvedLiquidElevation, ResolvedLiquidPort, ResolvedPatch, ResolvedPort,
    RING19_LOCAL_FRAME_SCALE,
};
use super::local_frame::LocalPatchFrame;
use super::seed::SeedStreams;
use super::V3GenerationError;

/// Deterministic construction mode shared by every patch-ready V3 recipe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PatchBuildMode {
    /// One member of the complete world's eight-candidate selection run.
    Candidate { world_seed: u64, candidate: u8 },
    /// Separately authored fallback with no seed or candidate state.
    CanonicalFallback,
}

impl PatchBuildMode {
    /// Candidate streams namespaced by the resolved patch, or none for fallback.
    #[must_use]
    pub(crate) fn seed_streams(self, patch: &PatchRecipeContext<'_>) -> Option<SeedStreams> {
        match self {
            Self::Candidate {
                world_seed,
                candidate,
            } => Some(patch.seed_streams(world_seed, candidate)),
            Self::CanonicalFallback => None,
        }
    }
}

/// One shared edge as seen from a particular patch.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PatchSharedEdge<'a> {
    pub(crate) id: ResolvedEdgeId,
    pub(crate) side: HexSide,
    pub(crate) contract: &'a ResolvedEdgeContract,
    patch_is_first: bool,
}

/// One directed liquid seam projected into a patch's local orientation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PatchLiquidPort {
    pub(crate) is_source: bool,
    pub(crate) port: ResolvedPort,
    pub(crate) elevation: ResolvedLiquidElevation,
}

/// One still-water seam projected into a patch's local orientation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PatchStandingWaterPort {
    pub(crate) port: ResolvedPort,
    pub(crate) elevation: ResolvedLiquidElevation,
}

impl<'a> PatchSharedEdge<'a> {
    /// Preferred surface level agreed by both neighboring patches.
    #[must_use]
    pub(crate) const fn preferred_level(&self) -> Level {
        self.contract.elevation.preferred
    }

    /// Lowest liquid or surface level admitted by this shared edge.
    #[must_use]
    pub(crate) const fn minimum_level(&self) -> Level {
        self.contract.elevation.min
    }

    /// Highest liquid or surface level admitted by this shared edge.
    #[must_use]
    pub(crate) const fn maximum_level(&self) -> Level {
        self.contract.elevation.max
    }

    /// Exact cells on this side which local decoration and hazards must preserve.
    #[must_use]
    pub(crate) fn protected_approaches(&self) -> &'a BTreeSet<HexCoord> {
        let patch = if self.patch_is_first {
            self.contract.first.0
        } else {
            self.contract.second.0
        };
        match self.contract.protected_approaches.get(&patch) {
            Some(coords) => coords,
            None => empty_coords(),
        }
    }

    /// Exact cells reserved only for this edge's ordinary-walker apertures.
    #[must_use]
    pub(crate) fn walker_protected_approaches(&self) -> BTreeSet<HexCoord> {
        self.contract
            .walker
            .ports
            .iter()
            .flat_map(|port| {
                if self.patch_is_first {
                    port.first_approach.iter()
                } else {
                    port.second_approach.iter()
                }
            })
            .copied()
            .collect()
    }

    /// Walker apertures oriented from this patch to its neighbor.
    #[must_use]
    pub(crate) fn walker_ports(&self) -> Vec<ResolvedPort> {
        self.contract
            .walker
            .ports
            .iter()
            .map(|port| orient_port(port, self.patch_is_first))
            .collect()
    }

    /// Every adjacent cross-patch pair, oriented outward from this patch.
    #[must_use]
    pub(crate) fn boundary_pairs(&self) -> BTreeSet<(HexCoord, HexCoord)> {
        if self.patch_is_first {
            self.contract.boundary_pairs.clone()
        } else {
            self.contract
                .boundary_pairs
                .iter()
                .map(|(first, second)| (*second, *first))
                .collect()
        }
    }

    /// Directed liquid aperture when this patch is the source or sink.
    #[must_use]
    pub(crate) fn liquid_port(&self) -> Option<PatchLiquidPort> {
        let ResolvedLiquidPort::Directed {
            source,
            port,
            elevation,
            ..
        } = &self.contract.liquid
        else {
            return None;
        };
        let patch = if self.patch_is_first {
            self.contract.first.0
        } else {
            self.contract.second.0
        };
        Some(PatchLiquidPort {
            is_source: *source == patch,
            port: orient_port(port, self.patch_is_first),
            elevation: *elevation,
        })
    }

    /// Standing-water aperture when this seam joins two level still bodies.
    #[must_use]
    pub(crate) fn standing_water_port(&self) -> Option<PatchStandingWaterPort> {
        let ResolvedLiquidPort::Standing { port, elevation } = &self.contract.liquid else {
            return None;
        };
        Some(PatchStandingWaterPort {
            port: orient_port(port, self.patch_is_first),
            elevation: *elevation,
        })
    }
}

/// Stable recipe inputs for one patch of a Single, Ring7, or Ring19 layout.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PatchRecipeContext<'a> {
    pub(crate) id: PatchId,
    pub(crate) patch: &'a ResolvedPatch,
    layout: &'a ResolvedLayoutPlan,
}

impl<'a> PatchRecipeContext<'a> {
    /// Resolves one patch without allowing recipes to fall back to patch zero.
    pub(crate) fn resolve(
        layout: &'a ResolvedLayoutPlan,
        id: PatchId,
    ) -> Result<Self, V3GenerationError> {
        let patch = layout.patches.get(&id).ok_or_else(|| {
            V3GenerationError::RecipeContract(format!(
                "resolved layout does not contain requested patch {}",
                id.0
            ))
        })?;
        Ok(Self { id, patch, layout })
    }

    /// Exact horizontal columns owned by this recipe.
    #[must_use]
    pub(crate) const fn mask(&self) -> &BTreeSet<HexCoord> {
        &self.patch.mask
    }

    /// Stable biome membership published for every generated surface.
    #[must_use]
    pub(crate) const fn biome_region(&self) -> BiomeRegionId {
        self.patch.biome_region
    }

    /// Clockwise turns applied to recipe-local authored geometry.
    #[must_use]
    pub(crate) const fn rotation_turns(&self) -> u8 {
        self.patch.rotation_turns
    }

    /// Stable recipe-local frame, resolved in constant time for fixed Ring19 slots.
    pub(crate) fn local_frame(&self) -> Result<LocalPatchFrame, V3GenerationError> {
        self.local_frame_with_rotation(self.rotation_turns())
    }

    /// Stable recipe-local frame with an explicit recipe-owned orientation.
    pub(crate) fn local_frame_with_rotation(
        &self,
        rotation: u8,
    ) -> Result<LocalPatchFrame, V3GenerationError> {
        if self.layout.kind == LayoutKind::Ring19 {
            let center = ring19_patch_center(self.id).ok_or_else(|| {
                V3GenerationError::RecipeContract(format!(
                    "Ring19 patch {} has no fixed local-frame centre",
                    self.id.0
                ))
            })?;
            if !self.patch.mask.contains(&center) {
                return Err(V3GenerationError::RecipeContract(format!(
                    "Ring19 patch {} does not contain its fixed local-frame centre {center:?}",
                    self.id.0
                )));
            }
            return Ok(LocalPatchFrame::from_resolved_ring19(
                center,
                RING19_LOCAL_FRAME_SCALE,
                rotation,
            ));
        }
        LocalPatchFrame::resolve_rotated(
            &self.patch.mask,
            self.layout.kind,
            self.layout.grid_radius,
            rotation,
        )
        .map_err(V3GenerationError::RecipeContract)
    }

    /// All shared edges in clockwise side order.
    pub(crate) fn shared_edges(&self) -> impl Iterator<Item = PatchSharedEdge<'a>> + '_ {
        let mut projected = Vec::new();
        if self.layout.kind == LayoutKind::Macro {
            for (edge_id, contract) in &self.layout.shared_edges {
                let (side, patch_is_first) = if contract.first.0 == self.id {
                    (contract.first.1, true)
                } else if contract.second.0 == self.id {
                    (contract.second.1, false)
                } else {
                    continue;
                };
                projected.push(PatchSharedEdge {
                    id: *edge_id,
                    side,
                    contract,
                    patch_is_first,
                });
            }
        } else {
            for side in HexSide::ALL {
                let Some(ResolvedEdgeReference::Shared(edge_id)) = self.patch.edges.get(&side)
                else {
                    continue;
                };
                let Some(contract) = self.layout.shared_edges.get(edge_id) else {
                    continue;
                };
                projected.push(PatchSharedEdge {
                    id: *edge_id,
                    side,
                    contract,
                    patch_is_first: contract.first == (self.id, side),
                });
            }
        }
        projected.into_iter()
    }

    /// Whether this exact patch side exits the complete resolved world.
    #[must_use]
    pub(crate) fn is_world_boundary(&self, side: HexSide) -> bool {
        if self.layout.kind == LayoutKind::Macro {
            return self
                .patch
                .mask
                .iter()
                .any(|coord| !self.layout.footprint.contains(&side.neighbor(*coord)));
        }
        matches!(
            self.patch.edges.get(&side),
            Some(ResolvedEdgeReference::WorldBoundary)
        )
    }

    /// Union of exact approach cells which recipe-local features must not occupy.
    #[must_use]
    pub(crate) fn protected_approaches(&self) -> BTreeSet<HexCoord> {
        let mut protected = BTreeSet::new();
        for edge in self.shared_edges() {
            protected.extend(edge.protected_approaches().iter().copied());
        }
        for outlet in self.boundary_liquid_outlets() {
            protected.extend(outlet.inward_approach.iter().copied());
        }
        protected
    }

    /// Union of approach cells reserved only for ordinary-walker seam apertures.
    #[must_use]
    pub(crate) fn walker_protected_approaches(&self) -> BTreeSet<HexCoord> {
        self.shared_edges()
            .flat_map(|edge| edge.walker_protected_approaches())
            .collect()
    }

    /// Exact complete-world boundary liquid outlets owned by this patch.
    pub(crate) fn boundary_liquid_outlets(
        &self,
    ) -> impl Iterator<Item = &'a ResolvedBoundaryLiquidOutlet> + '_ {
        self.layout
            .boundary_liquid_outlets
            .values()
            .filter(move |outlet| outlet.source == self.id)
    }

    /// Candidate streams namespaced by the stable patch slot.
    #[must_use]
    pub(crate) fn seed_streams(&self, seed: u64, candidate: u8) -> SeedStreams {
        SeedStreams::new(seed, candidate, self.id.0)
    }

    /// Complete resolved layout which owns this patch.
    #[must_use]
    pub(crate) const fn layout(&self) -> &'a ResolvedLayoutPlan {
        self.layout
    }

    /// Whole-world radius used for semantic and camera scaling.
    #[must_use]
    pub(crate) const fn grid_radius(&self) -> u32 {
        self.layout.grid_radius
    }
}

fn orient_port(port: &ResolvedPort, patch_is_first: bool) -> ResolvedPort {
    if patch_is_first {
        return port.clone();
    }
    ResolvedPort {
        lanes: port
            .lanes
            .iter()
            .map(|(first, second)| (*second, *first))
            .collect(),
        first_approach: port.second_approach.clone(),
        second_approach: port.first_approach.clone(),
    }
}

fn empty_coords() -> &'static BTreeSet<HexCoord> {
    static EMPTY: std::sync::OnceLock<BTreeSet<HexCoord>> = std::sync::OnceLock::new();
    EMPTY.get_or_init(BTreeSet::new)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use hex_core::{BiomeRegionId, HexCoord};

    use super::*;
    use crate::procedural_v3::layout::{
        LayoutKind, ResolvedEdgeId, ResolvedElevationBand, ResolvedWalkerPorts,
    };

    fn two_patch_layout() -> ResolvedLayoutPlan {
        let first_coord = HexCoord::ORIGIN;
        let second_coord = HexCoord::from_axial(1, 0);
        let edge_id = ResolvedEdgeId(7);
        let mut first_edges = BTreeMap::from_iter(
            HexSide::ALL.map(|side| (side, ResolvedEdgeReference::WorldBoundary)),
        );
        let mut second_edges = BTreeMap::from_iter(
            HexSide::ALL.map(|side| (side, ResolvedEdgeReference::WorldBoundary)),
        );
        first_edges.insert(HexSide::East, ResolvedEdgeReference::Shared(edge_id));
        second_edges.insert(HexSide::West, ResolvedEdgeReference::Shared(edge_id));
        let port = ResolvedPort {
            lanes: BTreeSet::from([(first_coord, second_coord)]),
            first_approach: BTreeSet::from([first_coord]),
            second_approach: BTreeSet::from([second_coord]),
        };
        ResolvedLayoutPlan {
            kind: LayoutKind::Ring7,
            grid_radius: 33,
            footprint: BTreeSet::from([first_coord, second_coord]),
            patches: BTreeMap::from([
                (
                    PatchId(2),
                    ResolvedPatch {
                        biome_region: BiomeRegionId(2),
                        rotation_turns: 0,
                        mask: BTreeSet::from([first_coord]),
                        edges: first_edges,
                    },
                ),
                (
                    PatchId(5),
                    ResolvedPatch {
                        biome_region: BiomeRegionId(5),
                        rotation_turns: 0,
                        mask: BTreeSet::from([second_coord]),
                        edges: second_edges,
                    },
                ),
            ]),
            shared_edges: BTreeMap::from([(
                edge_id,
                ResolvedEdgeContract {
                    first: (PatchId(2), HexSide::East),
                    second: (PatchId(5), HexSide::West),
                    elevation: ResolvedElevationBand {
                        preferred: 15,
                        min: 14,
                        max: 16,
                    },
                    walker: ResolvedWalkerPorts {
                        count: 1,
                        width: 1,
                        ports: vec![port],
                    },
                    liquid: ResolvedLiquidPort::Dry,
                    approach_depth: 1,
                    boundary_pairs: BTreeSet::from([(first_coord, second_coord)]),
                    protected_approaches: BTreeMap::from([
                        (PatchId(2), BTreeSet::from([first_coord])),
                        (PatchId(5), BTreeSet::from([second_coord])),
                    ]),
                },
            )]),
            boundary_liquid_outlets: BTreeMap::new(),
        }
    }

    #[test]
    fn patch_context_orients_shared_ports_and_namespaces_streams() {
        let layout = two_patch_layout();
        let first = PatchRecipeContext::resolve(&layout, PatchId(2)).expect("first patch");
        let second = PatchRecipeContext::resolve(&layout, PatchId(5)).expect("second patch");
        let first_edge = first.shared_edges().next().expect("first edge");
        let second_edge = second.shared_edges().next().expect("second edge");

        assert_eq!(first_edge.side, HexSide::East);
        assert_eq!(second_edge.side, HexSide::West);
        assert_eq!(first_edge.preferred_level(), 15);
        assert_eq!(
            first_edge
                .walker_ports()
                .first()
                .expect("first patch exposes one walker port")
                .lanes,
            BTreeSet::from([(HexCoord::ORIGIN, HexCoord::from_axial(1, 0))])
        );
        assert_eq!(
            second_edge
                .walker_ports()
                .first()
                .expect("second patch exposes one walker port")
                .lanes,
            BTreeSet::from([(HexCoord::from_axial(1, 0), HexCoord::ORIGIN)])
        );
        assert_ne!(
            first.seed_streams(44, 3).stage("landform").sample(9),
            second.seed_streams(44, 3).stage("landform").sample(9)
        );
    }

    #[test]
    fn boundary_liquid_approaches_join_full_but_not_walker_only_reservations() {
        let mut layout = two_patch_layout();
        layout.kind = LayoutKind::Ring19;
        let boundary_approach = HexCoord::from_axial(-1, 0);
        layout.boundary_liquid_outlets.insert(
            (PatchId(2), HexSide::West),
            ResolvedBoundaryLiquidOutlet {
                source: PatchId(2),
                side: HexSide::West,
                lanes: BTreeSet::from([(
                    HexCoord::ORIGIN,
                    HexSide::West.neighbor(HexCoord::ORIGIN),
                )]),
                inward_approach: BTreeSet::from([boundary_approach]),
                approach_depth: 1,
                level: 3,
            },
        );
        let context = PatchRecipeContext::resolve(&layout, PatchId(2)).expect("first patch");

        assert_eq!(
            context.walker_protected_approaches(),
            BTreeSet::from([HexCoord::ORIGIN])
        );
        assert_eq!(
            context.protected_approaches(),
            BTreeSet::from([HexCoord::ORIGIN, boundary_approach])
        );
        assert_eq!(context.boundary_liquid_outlets().count(), 1);
    }

    #[test]
    fn missing_patch_is_an_explicit_contract_failure() {
        let layout = two_patch_layout();
        let error =
            PatchRecipeContext::resolve(&layout, PatchId(99)).expect_err("missing patch fails");
        assert!(error.to_string().contains("requested patch 99"));
    }
}
