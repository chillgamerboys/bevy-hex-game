//! Hex coordinates and the marker components that identify grid entities.
//!
//! # Cube coordinates
//!
//! Game code addresses the grid in **cube coordinates** — three axes `x`, `y`, `z`
//! with the invariant `x + y + z == 0`. Cube space is what makes hex algorithms
//! read naturally: distance is half the sum of absolute component differences,
//! rotation is a component rotation, reflection a component swap.
//!
//! Storage is the axial pair `(x, y)`, with `z` derived as `-x - y`. That is not a
//! compromise. Because the three coordinates must always sum to zero, storing all
//! three would make it possible to represent a value that is not a hex at all.
//! [`HexCoord::new_cubic`] enforces the invariant at the boundary, and everything
//! past that point is correct by construction.
//!
//! # Relationship to `hexx`
//!
//! Hex-space algorithms are delegated to [`hexx`], which has already solved the
//! fiddly ones — line drawing and ranges today, A*, field-of-view and
//! field-of-movement when gameplay needs them.
//!
//! `hexx` is depended on **without its Bevy features**, so it pins only `glam` and
//! can never gate a Bevy upgrade. Every `hexx` entry point used here takes and
//! returns plain scalars and arrays, so its `glam` version never meets Bevy's.
//! Hex↔world conversion stays ours, because it has to account for the height map
//! and the dimensions of `hex.glb`.

use bevy_ecs::prelude::*;
use bevy_math::Vec3;
use bevy_reflect::prelude::*;
use hexx::Hex;

use crate::config::HEX_CIRCUMRADIUS;
use crate::terrain::HeightMap;

/// A position on the hex grid, in cube coordinates.
///
/// See the [module documentation](self) for why only two of the three cube
/// coordinates are stored.
#[derive(Component, Reflect, Default, Debug, Copy, Clone, PartialEq, Eq, Hash)]
#[reflect(Component)]
pub struct HexCoord {
    x: i32,
    y: i32,
}

impl HexCoord {
    /// The centre of the grid.
    pub const ORIGIN: Self = Self { x: 0, y: 0 };

    /// Builds a coordinate from cube coordinates.
    ///
    /// # Panics
    ///
    /// Panics unless `x + y + z == 0`. Use [`Self::try_new_cubic`] where the
    /// inputs are not already known to describe a valid hex.
    #[must_use]
    pub const fn new_cubic(x: i32, y: i32, z: i32) -> Self {
        assert!(x + y + z == 0, "cube coordinates must sum to zero");
        Self { x, y }
    }

    /// Builds a coordinate from cube coordinates, or [`None`] if they do not sum
    /// to zero.
    #[must_use]
    pub const fn try_new_cubic(x: i32, y: i32, z: i32) -> Option<Self> {
        if x + y + z == 0 {
            Some(Self { x, y })
        } else {
            None
        }
    }

    /// Builds a coordinate from the axial pair.
    ///
    /// Prefer [`Self::new_cubic`] in game code; this exists for serialization and
    /// for interop with code that speaks axial.
    #[must_use]
    pub const fn from_axial(x: i32, y: i32) -> Self {
        Self { x, y }
    }

    /// The `x` cube coordinate (also called `q`).
    #[must_use]
    pub const fn x(self) -> i32 {
        self.x
    }

    /// The `y` cube coordinate (also called `r`).
    #[must_use]
    pub const fn y(self) -> i32 {
        self.y
    }

    /// The `z` cube coordinate (also called `s`), derived as `-x - y`.
    #[must_use]
    pub const fn z(self) -> i32 {
        -self.x - self.y
    }

    /// All three cube coordinates, in `[x, y, z]` order.
    #[must_use]
    pub const fn to_cubic_array(self) -> [i32; 3] {
        [self.x, self.y, self.z()]
    }

    /// Position of this tile's centre in world space.
    ///
    /// Pass a height map to sit the position on top of the tile; without one the
    /// `y` component is zero.
    #[must_use]
    pub fn to_world(&self, map: Option<&HeightMap>) -> Vec3 {
        let x = HEX_CIRCUMRADIUS * f32::sqrt(3.0) * ((self.x as f32) + (self.y as f32) / 2.0);
        let y = map.map_or(0., |map| map.get_world_height(*self));
        let z = HEX_CIRCUMRADIUS * (3.0 / 2.0) * (self.y as f32);
        Vec3 { x, y, z }
    }

    /// The tile containing a world-space position. The `y` component is ignored.
    #[must_use]
    pub fn from_world(world_coord: Vec3) -> Self {
        let x = (f32::sqrt(3.0) * world_coord.x - world_coord.z) / 3.0 / HEX_CIRCUMRADIUS;
        let y = ((2.0 / 3.0) * world_coord.z) / HEX_CIRCUMRADIUS;
        Self::from_floating([x, y])
    }

    /// Rounds fractional hex-space coordinates to the nearest tile.
    #[must_use]
    pub fn from_floating(coords: [f32; 2]) -> Self {
        Self::from_hex(Hex::round(coords))
    }

    /// A stable byte representation, used as hash input by the terrain generators.
    #[must_use]
    pub const fn to_bytes(self) -> [u8; 8] {
        let x = self.x.to_ne_bytes();
        let y = self.y.to_ne_bytes();
        [x[0], x[1], x[2], x[3], y[0], y[1], y[2], y[3]]
    }

    /// Number of tiles between this coordinate and `other`.
    #[must_use]
    pub fn distance(&self, other: Self) -> u32 {
        self.to_hex().unsigned_distance_to(other.to_hex())
    }

    /// The tiles forming a straight line from here to `other`, inclusive of both
    /// ends.
    ///
    /// A zero-length line yields exactly one tile. The previous hand-rolled
    /// implementation divided by the distance, so this case produced `NaN` — which
    /// then went through `as i32` — meaning clicking the tile you were already
    /// standing on generated garbage coordinates.
    #[must_use]
    pub fn line_between(&self, other: Self) -> Vec<Self> {
        self.to_hex()
            .line_to(other.to_hex())
            .map(Self::from_hex)
            .collect()
    }

    /// Every tile within `radius` steps, including this one.
    #[must_use]
    pub fn within_radius(&self, radius: u32) -> Vec<Self> {
        self.to_hex().range(radius).map(Self::from_hex).collect()
    }

    /// The six adjacent tiles.
    #[must_use]
    pub fn neighbors(&self) -> [Self; 6] {
        self.to_hex().all_neighbors().map(Self::from_hex)
    }

    const fn to_hex(self) -> Hex {
        Hex::new(self.x, self.y)
    }

    const fn from_hex(hex: Hex) -> Self {
        Self { x: hex.x, y: hex.y }
    }
}

/// Marks the parent entity that owns every spawned tile.
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct HexGrid;

/// Marks a single tile of the grid. Lives here rather than in `hex_world` so that
/// gameplay can query tiles without depending on the presentation crate.
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct HexTile;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_coordinates_always_sum_to_zero() {
        for x in -20..=20 {
            for y in -20..=20 {
                let coord = HexCoord::from_axial(x, y);
                let [cx, cy, cz] = coord.to_cubic_array();
                assert_eq!(cx + cy + cz, 0, "invariant broken at {coord:?}");
            }
        }
    }

    #[test]
    fn new_cubic_round_trips() {
        let coord = HexCoord::new_cubic(3, 5, -8);
        assert_eq!(coord.x(), 3);
        assert_eq!(coord.y(), 5);
        assert_eq!(coord.z(), -8);
        assert_eq!(coord.to_cubic_array(), [3, 5, -8]);
    }

    #[test]
    fn try_new_cubic_rejects_invalid_triples() {
        assert!(HexCoord::try_new_cubic(1, 1, -2).is_some());
        assert!(HexCoord::try_new_cubic(1, 1, 1).is_none());
    }

    #[test]
    #[should_panic(expected = "cube coordinates must sum to zero")]
    fn new_cubic_panics_on_invalid_triple() {
        let _ = HexCoord::new_cubic(1, 1, 1);
    }

    #[test]
    fn world_round_trips_through_hex_space() {
        for x in -15..=15 {
            for y in -15..=15 {
                let coord = HexCoord::from_axial(x, y);
                let back = HexCoord::from_world(coord.to_world(None));
                assert_eq!(coord, back, "round trip failed for {coord:?}");
            }
        }
    }

    #[test]
    fn distance_to_self_is_zero() {
        assert_eq!(HexCoord::ORIGIN.distance(HexCoord::ORIGIN), 0);
    }

    #[test]
    fn line_length_is_one_more_than_distance() {
        let target = HexCoord::new_cubic(3, -5, 2);
        // The line includes both endpoints, so it is one longer than the distance.
        assert_eq!(
            HexCoord::ORIGIN.line_between(target).len() as u32,
            HexCoord::ORIGIN.distance(target) + 1
        );
    }

    /// Regression test. Clicking the tile you are standing on used to divide by a
    /// zero distance, producing `NaN` coordinates rather than a single tile.
    #[test]
    fn line_to_self_is_a_single_tile() {
        let coord = HexCoord::new_cubic(2, -3, 1);
        assert_eq!(coord.line_between(coord), vec![coord]);
    }

    #[test]
    fn line_endpoints_are_inclusive() {
        let start = HexCoord::ORIGIN;
        let end = HexCoord::new_cubic(5, 0, -5);
        let line = start.line_between(end);
        assert_eq!(line.first(), Some(&start));
        assert_eq!(line.last(), Some(&end));
    }

    #[test]
    fn line_steps_are_contiguous() {
        let line = HexCoord::ORIGIN.line_between(HexCoord::new_cubic(4, -7, 3));
        for pair in line.windows(2) {
            assert_eq!(pair[0].distance(pair[1]), 1, "gap between {pair:?}");
        }
    }

    #[test]
    fn radius_covers_the_expected_tile_count() {
        // A hexagon of radius r holds 3r² + 3r + 1 tiles.
        for radius in 0..=20u32 {
            let expected = 3 * radius * radius + 3 * radius + 1;
            assert_eq!(
                HexCoord::ORIGIN.within_radius(radius).len() as u32,
                expected,
                "wrong tile count at radius {radius}"
            );
        }
    }

    #[test]
    fn every_neighbor_is_one_step_away() {
        let coord = HexCoord::new_cubic(4, -1, -3);
        for neighbor in coord.neighbors() {
            assert_eq!(coord.distance(neighbor), 1);
        }
    }
}
