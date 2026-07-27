//! Exact positional metadata shared by map generation and gameplay.
//!
//! V3 generator plans remain private to `hex_map`. These small resources are the
//! stable consequences gameplay is allowed to consume.

use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::prelude::*;
use bevy_reflect::prelude::*;

use crate::TilePos;

/// Exact surfaces blocked by generated non-terrain features.
///
/// Terrain standability remains described by tile components and traversal
/// predicates. This resource covers blockers such as a tree rooted on an otherwise
/// standable surface.
#[derive(Resource, Debug, Default, Clone)]
pub struct TraversalBlockers {
    surfaces: BTreeSet<TilePos>,
}

impl TraversalBlockers {
    /// Creates an empty blocker collection.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Marks an exact surface as blocked.
    ///
    /// Returns whether the surface was newly inserted.
    pub fn insert(&mut self, pos: TilePos) -> bool {
        self.surfaces.insert(pos)
    }

    /// Removes a blocker from an exact surface.
    ///
    /// Returns whether the surface was present.
    pub fn remove(&mut self, pos: TilePos) -> bool {
        self.surfaces.remove(&pos)
    }

    /// Whether an exact surface is blocked by a generated feature.
    #[must_use]
    pub fn contains(&self, pos: TilePos) -> bool {
        self.surfaces.contains(&pos)
    }

    /// Iterates over blocked exact surfaces in position order.
    pub fn iter(&self) -> impl Iterator<Item = TilePos> + '_ {
        self.surfaces.iter().copied()
    }

    /// Number of blocked exact surfaces.
    #[must_use]
    pub fn len(&self) -> usize {
        self.surfaces.len()
    }

    /// Whether no exact surface is blocked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.surfaces.is_empty()
    }
}

/// Stable map-local identity of one generated biome region.
///
/// Numeric identities are deterministic only within one generated world. Recipe and
/// environment names remain generator-owned metadata.
#[derive(Reflect, Debug, Default, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BiomeRegionId(pub u32);

/// Biome membership of generated exact surfaces.
///
/// Membership is keyed by [`TilePos`] so ground and a sky island in one horizontal
/// column may belong to different regions. The generator replaces this resource with
/// each map.
#[derive(Resource, Debug, Default, Clone)]
pub struct BiomeRegions {
    by_surface: BTreeMap<TilePos, BiomeRegionId>,
}

impl BiomeRegions {
    /// Creates an empty biome membership map.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds or replaces one exact surface's biome membership.
    pub fn insert(&mut self, pos: TilePos, region: BiomeRegionId) -> Option<BiomeRegionId> {
        self.by_surface.insert(pos, region)
    }

    /// Removes biome membership from one exact surface.
    pub fn remove(&mut self, pos: TilePos) -> Option<BiomeRegionId> {
        self.by_surface.remove(&pos)
    }

    /// Biome region containing one exact surface.
    #[must_use]
    pub fn get(&self, pos: TilePos) -> Option<BiomeRegionId> {
        self.by_surface.get(&pos).copied()
    }

    /// Iterates over exact surfaces and their biome regions in position order.
    pub fn iter(&self) -> impl Iterator<Item = (TilePos, BiomeRegionId)> + '_ {
        self.by_surface
            .iter()
            .map(|(position, region)| (*position, *region))
    }

    /// Number of exact surfaces with published biome membership.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_surface.len()
    }

    /// Whether no exact surface has published biome membership.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_surface.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HexCoord;

    #[test]
    fn blockers_distinguish_stacked_surfaces() {
        let lower = TilePos::new(HexCoord::ORIGIN, 5);
        let upper = TilePos::new(HexCoord::ORIGIN, 15);
        let mut blockers = TraversalBlockers::new();
        assert!(blockers.insert(lower));
        assert!(blockers.contains(lower));
        assert!(!blockers.contains(upper));
        assert!(blockers.remove(lower));
        assert!(blockers.is_empty());
    }

    #[test]
    fn biome_membership_distinguishes_stacked_surfaces() {
        let lower = TilePos::new(HexCoord::ORIGIN, 5);
        let upper = TilePos::new(HexCoord::ORIGIN, 15);
        let mut regions = BiomeRegions::new();
        regions.insert(lower, BiomeRegionId(1));
        regions.insert(upper, BiomeRegionId(2));

        assert_eq!(regions.get(lower), Some(BiomeRegionId(1)));
        assert_eq!(regions.get(upper), Some(BiomeRegionId(2)));
        assert_eq!(regions.len(), 2);
    }
}
