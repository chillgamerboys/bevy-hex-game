//! Shared facts produced while preparing a terrain.
//!
//! `hex_map` owns generation and storage, while scenarios and units need to consume
//! a few of its results without depending on that crate. These resources are the
//! narrow hand-off: stable spawn anchors, the resolved seed, and an explicit signal
//! that terrain construction succeeded.

use std::fmt;

use bevy_ecs::prelude::*;
use bevy_platform::collections::HashMap;
use bevy_reflect::prelude::*;

use crate::TilePos;

/// Stable name of a generated point on a map.
///
/// A string newtype keeps scenario files readable and lets generators introduce
/// recipe-specific anchors without expanding a shared enum. Asset crates deserialize
/// their own string-backed representation and convert through [`Self::new`], keeping
/// serialization dependencies out of this bottom-level domain crate.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MapAnchorId(String);

impl MapAnchorId {
    /// Creates an anchor id from its stable textual name.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// The stable textual name used by settings and diagnostics.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for MapAnchorId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for MapAnchorId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for MapAnchorId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Exact generated surfaces available to scenario placement.
///
/// Anchors use [`TilePos`] rather than a horizontal coordinate because a generated
/// bridge and the ground beneath it may share a hex while remaining unrelated
/// places. The generator replaces this resource for each map.
#[derive(Resource, Debug, Default, Clone)]
pub struct MapAnchors {
    by_id: HashMap<MapAnchorId, TilePos>,
}

impl MapAnchors {
    /// Creates an empty anchor collection.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds or replaces one anchor, returning the previous position if it existed.
    pub fn insert(&mut self, id: MapAnchorId, pos: TilePos) -> Option<TilePos> {
        self.by_id.insert(id, pos)
    }

    /// Resolves an anchor to its exact surface.
    #[must_use]
    pub fn get(&self, id: &MapAnchorId) -> Option<TilePos> {
        self.by_id.get(id).copied()
    }

    /// Every anchor and its exact surface, in unspecified order.
    pub fn iter(&self) -> impl Iterator<Item = (&MapAnchorId, TilePos)> {
        self.by_id.iter().map(|(id, pos)| (id, *pos))
    }

    /// Number of published anchors.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    /// Whether the map published no anchors.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}

impl FromIterator<(MapAnchorId, TilePos)> for MapAnchors {
    fn from_iter<T: IntoIterator<Item = (MapAnchorId, TilePos)>>(anchors: T) -> Self {
        Self {
            by_id: anchors.into_iter().collect(),
        }
    }
}

/// Opaque identity of an optional area that ordinary walking cannot reach.
///
/// The id is deterministic only within one generated map. It groups exact surface
/// positions without naming a generator recipe or promising which future ability can
/// enter the area. Tile entities in such an area carry this component, while
/// [`SpecialMovementRegions`] remains the positional source of truth.
#[derive(Component, Reflect, Debug, Default, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[reflect(Component)]
pub struct SpecialMovementRegion(
    /// Map-local deterministic region number.
    pub u32,
);

/// Exact generated surfaces belonging to optional special-movement regions.
///
/// This is keyed by [`TilePos`] rather than a horizontal coordinate because a bridge
/// and the ground below it can occupy the same column without belonging to the same
/// region. The map replaces this resource whenever it constructs a new terrain.
#[derive(Resource, Debug, Default, Clone)]
pub struct SpecialMovementRegions {
    by_surface: HashMap<TilePos, SpecialMovementRegion>,
}

impl SpecialMovementRegions {
    /// Creates an empty region collection.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds or replaces one exact surface, returning its previous region if present.
    pub fn insert(
        &mut self,
        pos: TilePos,
        region: SpecialMovementRegion,
    ) -> Option<SpecialMovementRegion> {
        self.by_surface.insert(pos, region)
    }

    /// Finds the special-movement region containing an exact surface.
    #[must_use]
    pub fn get(&self, pos: TilePos) -> Option<SpecialMovementRegion> {
        self.by_surface.get(&pos).copied()
    }

    /// Every tagged surface and its map-local region id, in unspecified order.
    pub fn iter(&self) -> impl Iterator<Item = (TilePos, SpecialMovementRegion)> + '_ {
        self.by_surface
            .iter()
            .map(|(position, region)| (*position, *region))
    }

    /// Keeps only memberships accepted by `keep`.
    ///
    /// Terrain edits use this to discard exact surfaces that no longer exist without
    /// inventing new semantic regions after generation.
    pub fn retain(&mut self, mut keep: impl FnMut(TilePos, SpecialMovementRegion) -> bool) {
        self.by_surface
            .retain(|position, region| keep(*position, *region));
    }

    /// Number of tagged surfaces.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_surface.len()
    }

    /// Whether the active map has no special-movement surfaces.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_surface.is_empty()
    }
}

impl FromIterator<(TilePos, SpecialMovementRegion)> for SpecialMovementRegions {
    fn from_iter<T: IntoIterator<Item = (TilePos, SpecialMovementRegion)>>(regions: T) -> Self {
        Self {
            by_surface: regions.into_iter().collect(),
        }
    }
}

/// Seed actually used to generate the active map.
///
/// This is resolved after applying any session reroll override, so diagnostics and
/// reproduction always report the value that reached the generator rather than only
/// the configured default.
#[derive(Resource, Reflect, Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
#[reflect(Resource)]
pub struct ResolvedMapSeed(pub u64);

/// Signals that the active terrain was generated and validated successfully.
///
/// Actors that require terrain should run after the setup stage that inserts this
/// marker, and may require the resource when a failed generation must prevent spawn.
#[derive(Resource, Reflect, Debug, Default, Clone, Copy, PartialEq, Eq)]
#[reflect(Resource)]
pub struct TerrainReady;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HexCoord;

    #[test]
    fn anchors_preserve_the_exact_surface() {
        let id = MapAnchorId::from("party_start");
        let ground = TilePos::new(HexCoord::ORIGIN, 4);
        let bridge = TilePos::new(HexCoord::ORIGIN, 9);
        let mut anchors = MapAnchors::new();

        assert_eq!(anchors.insert(id.clone(), ground), None);
        assert_eq!(anchors.insert(id.clone(), bridge), Some(ground));
        assert_eq!(anchors.get(&id), Some(bridge));
    }

    #[test]
    fn special_regions_distinguish_stacked_surfaces() {
        let ground = TilePos::new(HexCoord::ORIGIN, 4);
        let bridge = TilePos::new(HexCoord::ORIGIN, 9);
        let mut regions = SpecialMovementRegions::new();

        assert_eq!(regions.insert(ground, SpecialMovementRegion(2)), None);
        assert_eq!(regions.insert(bridge, SpecialMovementRegion(7)), None);
        assert_eq!(regions.get(ground), Some(SpecialMovementRegion(2)));
        assert_eq!(regions.get(bridge), Some(SpecialMovementRegion(7)));
        assert_eq!(regions.len(), 2);
    }

    #[test]
    fn special_regions_can_prune_stale_exact_surfaces() {
        let kept = TilePos::new(HexCoord::ORIGIN, 4);
        let removed = TilePos::new(HexCoord::new_cubic(1, -1, 0), 6);
        let mut regions: SpecialMovementRegions = [
            (kept, SpecialMovementRegion(0)),
            (removed, SpecialMovementRegion(1)),
        ]
        .into_iter()
        .collect();

        regions.retain(|position, _| position == kept);

        assert_eq!(regions.get(kept), Some(SpecialMovementRegion(0)));
        assert_eq!(regions.get(removed), None);
    }
}
