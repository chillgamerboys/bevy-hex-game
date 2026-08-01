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
/// their own string-backed representation and convert through [`Self::new`], so the id
/// keeps a single construction path and is not itself pinned to an on-disk format.
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

/// Stable identity of one generated interior network.
///
/// The number is deterministic only within one map. Exact floor and roof-voxel
/// memberships live in [`InteriorRegions`].
#[derive(Reflect, Debug, Default, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InteriorRegionId(pub u32);

/// Marks one rendered terrain run segment as a roof review tooling may cut away.
///
/// Ordinary gameplay keeps the roof visible. This component is a projection for
/// explicit capture-tool queries. The exact positional source of truth remains the
/// roof voxels in [`InteriorRegions`], so rebuilding or splitting terrain runs cannot
/// change which generated material the metadata names.
#[derive(Component, Reflect, Debug, Default, Clone, Copy, PartialEq, Eq)]
#[reflect(Component)]
pub struct CutawayOccluder(pub InteriorRegionId);

/// Exact floor and roof-voxel memberships for generated interior networks.
///
/// Both collections use [`TilePos`] because caves may exist beneath another standable
/// surface in the same column. Interior floors determine when an actor is inside or
/// entering a region. Roof voxels are the persistent source from which `hex_map`
/// projects cutaway components onto its transient rendered runs.
#[derive(Resource, Debug, Default, Clone)]
pub struct InteriorRegions {
    by_surface: HashMap<TilePos, InteriorRegionId>,
    by_roof_voxel: HashMap<TilePos, InteriorRegionId>,
}

impl InteriorRegions {
    /// Creates empty interior metadata.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds or replaces one exact interior floor.
    pub fn insert_surface(
        &mut self,
        pos: TilePos,
        region: InteriorRegionId,
    ) -> Option<InteriorRegionId> {
        self.by_surface.insert(pos, region)
    }

    /// Adds or replaces one exact voxel of authored cutaway roof.
    pub fn insert_roof_voxel(
        &mut self,
        pos: TilePos,
        region: InteriorRegionId,
    ) -> Option<InteriorRegionId> {
        self.by_roof_voxel.insert(pos, region)
    }

    /// Removes one exact voxel from the authored cutaway roof.
    pub fn remove_roof_voxel(&mut self, pos: TilePos) -> Option<InteriorRegionId> {
        self.by_roof_voxel.remove(&pos)
    }

    /// Region containing an exact interior floor.
    #[must_use]
    pub fn get(&self, pos: TilePos) -> Option<InteriorRegionId> {
        self.by_surface.get(&pos).copied()
    }

    /// Region whose authored opaque roof contains the exact voxel at `pos`.
    #[must_use]
    pub fn roof_region(&self, pos: TilePos) -> Option<InteriorRegionId> {
        self.by_roof_voxel.get(&pos).copied()
    }

    /// Every exact interior floor and its region, in unspecified order.
    pub fn surfaces(&self) -> impl Iterator<Item = (TilePos, InteriorRegionId)> + '_ {
        self.by_surface
            .iter()
            .map(|(position, region)| (*position, *region))
    }

    /// Every exact cutaway roof voxel and its region, in unspecified order.
    pub fn roof_voxels(&self) -> impl Iterator<Item = (TilePos, InteriorRegionId)> + '_ {
        self.by_roof_voxel
            .iter()
            .map(|(position, region)| (*position, *region))
    }

    /// Keeps only interior floors accepted by `keep`.
    pub fn retain_surfaces(&mut self, mut keep: impl FnMut(TilePos, InteriorRegionId) -> bool) {
        self.by_surface
            .retain(|position, region| keep(*position, *region));
    }

    /// Keeps only cutaway roof voxels accepted by `keep`.
    pub fn retain_roof_voxels(&mut self, mut keep: impl FnMut(TilePos, InteriorRegionId) -> bool) {
        self.by_roof_voxel
            .retain(|position, region| keep(*position, *region));
    }

    /// Whether any exact cutaway roof voxels are present.
    #[must_use]
    pub fn has_roof_voxels(&self) -> bool {
        !self.by_roof_voxel.is_empty()
    }

    /// Whether the active map has no interior floors or cutaway roof voxels.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_surface.is_empty() && self.by_roof_voxel.is_empty()
    }
}

/// Opaque identity of an optional area that ordinary walking cannot reach.
///
/// The id is deterministic only within one generated map. It groups exact surface
/// positions without naming a generator recipe or promising which future ability can
/// enter the area. [`SpecialMovementRegions`] is the positional source of truth; an
/// ECS component should be added only when a live system has a query-based use for it.
#[derive(Reflect, Debug, Default, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
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

/// Preferred initial camera frame generated from the active map geometry.
///
/// Values are world-space positions because `hex_world` deliberately cannot depend on
/// map settings such as `level_height`. The map converts its semantic hint before
/// publishing this resource. Authored and compatibility maps may omit it and use the
/// designer-authored camera settings instead.
#[derive(Resource, Reflect, Debug, Clone, Copy, PartialEq)]
#[reflect(Resource)]
pub struct MapViewHint {
    /// Initial camera position.
    pub eye: (f32, f32, f32),
    /// Point the camera looks at and orbits around.
    pub focus: (f32, f32, f32),
}

impl MapViewHint {
    /// Creates a world-space camera hint.
    #[must_use]
    pub const fn new(eye: (f32, f32, f32), focus: (f32, f32, f32)) -> Self {
        Self { eye, focus }
    }

    /// Whether the two points describe a finite, non-degenerate camera frame.
    #[must_use]
    pub fn is_valid(self) -> bool {
        let (eye_x, eye_y, eye_z) = self.eye;
        let (focus_x, focus_y, focus_z) = self.focus;
        let offset = (eye_x - focus_x, eye_y - focus_y, eye_z - focus_z);
        [eye_x, eye_y, eye_z, focus_x, focus_y, focus_z]
            .into_iter()
            .all(f32::is_finite)
            && offset
                .0
                .mul_add(offset.0, offset.1.mul_add(offset.1, offset.2 * offset.2))
                > f32::EPSILON
    }
}

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

    #[test]
    fn interior_metadata_distinguishes_floors_and_exact_roof_voxels() {
        let floor = TilePos::new(HexCoord::ORIGIN, 6);
        let roof_bottom = TilePos::new(HexCoord::ORIGIN, 14);
        let roof_middle = TilePos::new(HexCoord::ORIGIN, 15);
        let roof_top = TilePos::new(HexCoord::ORIGIN, 16);
        let region = InteriorRegionId(3);
        let mut interiors = InteriorRegions::new();

        assert_eq!(interiors.insert_surface(floor, region), None);
        for roof in [roof_bottom, roof_middle, roof_top] {
            assert_eq!(interiors.insert_roof_voxel(roof, region), None);
        }
        assert_eq!(interiors.get(floor), Some(region));
        assert_eq!(interiors.get(roof_top), None);
        assert_eq!(interiors.roof_region(roof_bottom), Some(region));
        assert_eq!(interiors.roof_region(roof_middle), Some(region));
        assert_eq!(interiors.roof_region(roof_top), Some(region));
        assert_eq!(interiors.roof_region(floor), None);
        assert_eq!(interiors.roof_voxels().count(), 3);
        assert!(interiors.has_roof_voxels());

        interiors.retain_surfaces(|position, _| position != floor);
        assert_eq!(interiors.remove_roof_voxel(roof_middle), Some(region));
        interiors.retain_roof_voxels(|position, _| position != roof_top);
        assert_eq!(interiors.get(floor), None);
        assert_eq!(interiors.roof_region(roof_bottom), Some(region));
        assert_eq!(interiors.roof_region(roof_middle), None);
        assert_eq!(interiors.roof_region(roof_top), None);
    }

    #[test]
    fn map_view_hint_rejects_invalid_frames() {
        assert!(MapViewHint::new((0.0, 1.0, 0.0), (0.0, 0.0, 0.0)).is_valid());
        assert!(!MapViewHint::new((0.0, 0.0, 0.0), (0.0, 0.0, 0.0)).is_valid());
        assert!(!MapViewHint::new((f32::NAN, f32::NAN, f32::NAN), (0.0, 0.0, 0.0)).is_valid());
    }
}
