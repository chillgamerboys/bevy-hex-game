//! Recipe-facing view of one resolved V3 patch.
//!
//! Layout resolution owns masks and shared-edge contracts. Recipes consume this
//! projection so they cannot reinterpret a seam or accidentally use another patch's
//! seed namespace.

use std::collections::BTreeSet;

use hex_core::{BiomeRegionId, HexCoord, Level};

use super::layout::{
    HexSide, PatchId, ResolvedEdgeContract, ResolvedEdgeId, ResolvedEdgeReference,
    ResolvedLayoutPlan, ResolvedLiquidPort, ResolvedPatch, ResolvedPort,
};
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

impl<'a> PatchSharedEdge<'a> {
    /// Preferred surface level agreed by both neighboring patches.
    #[must_use]
    pub(crate) const fn preferred_level(&self) -> Level {
        self.contract.elevation.preferred
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
    pub(crate) fn liquid_port(&self) -> Option<(bool, ResolvedPort)> {
        let ResolvedLiquidPort::Directed { source, port, .. } = &self.contract.liquid else {
            return None;
        };
        let patch = if self.patch_is_first {
            self.contract.first.0
        } else {
            self.contract.second.0
        };
        Some((*source == patch, orient_port(port, self.patch_is_first)))
    }
}

/// Stable recipe inputs for one patch of either a Single or Ring7 layout.
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

    /// All shared edges in clockwise side order.
    pub(crate) fn shared_edges(&self) -> impl Iterator<Item = PatchSharedEdge<'a>> + '_ {
        HexSide::ALL.into_iter().filter_map(|side| {
            let ResolvedEdgeReference::Shared(edge_id) = self.patch.edges.get(&side)? else {
                return None;
            };
            let contract = self.layout.shared_edges.get(edge_id)?;
            let patch_is_first = contract.first == (self.id, side);
            Some(PatchSharedEdge {
                id: *edge_id,
                side,
                contract,
                patch_is_first,
            })
        })
    }

    /// Union of exact approach cells which recipe-local features must not occupy.
    #[must_use]
    pub(crate) fn protected_approaches(&self) -> BTreeSet<HexCoord> {
        let mut protected = BTreeSet::new();
        for edge in self.shared_edges() {
            protected.extend(edge.protected_approaches().iter().copied());
        }
        protected
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
                        mask: BTreeSet::from([first_coord]),
                        edges: first_edges,
                    },
                ),
                (
                    PatchId(5),
                    ResolvedPatch {
                        biome_region: BiomeRegionId(5),
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
    fn missing_patch_is_an_explicit_contract_failure() {
        let layout = two_patch_layout();
        let error =
            PatchRecipeContext::resolve(&layout, PatchId(99)).expect_err("missing patch fails");
        assert!(error.to_string().contains("requested patch 99"));
    }
}
