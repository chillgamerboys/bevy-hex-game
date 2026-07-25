//! Hex coordinates and the marker components that identify grid entities.

use std::cmp::{max, min};

use bevy_ecs::prelude::*;
use bevy_math::Vec3;
use bevy_reflect::prelude::*;

use crate::config::HEX_CIRCUMRADIUS;
use crate::terrain::HeightMap;

#[derive(Component, Reflect, Default, Debug, Copy, Clone, PartialEq, Eq, Hash)]
#[reflect(Component)]
/// Coordinates in axial space
/// see: https://www.redblobgames.com/grids/hexagons/#coordinates-axial
/// HexCoord(q, r)
pub struct HexCoord(pub i32, pub i32);

impl HexCoord {
    /// see: https://www.redblobgames.com/grids/hexagons/#hex-to-pixel-axial
    ///
    /// Optionally provide height map to get position at the top of the tile.
    /// Otherwise just set height to 0
    pub fn to_world(&self, map: Option<&HeightMap>) -> Vec3 {
        let x = HEX_CIRCUMRADIUS * f32::sqrt(3.0) * ((self.0 as f32) + (self.1 as f32) / 2.0);
        let y = if let Some(map) = map {
            map.get_world_height(*self)
        } else {
            0.
        };
        let z = HEX_CIRCUMRADIUS * (3.0 / 2.0) * (self.1 as f32);
        Vec3 { x, y, z }
    }

    /// Uses just x and z componeents of world coord to convert to hexcoord
    /// See: https://www.redblobgames.com/grids/hexagons/#pixel-to-hex
    pub fn from_world(world_coord: Vec3) -> HexCoord {
        // first convert to hex space
        let x = (f32::sqrt(3.0) * world_coord.x - world_coord.z) / 3.0 / HEX_CIRCUMRADIUS;
        let y = ((2.0 / 3.0) * world_coord.z) / HEX_CIRCUMRADIUS;
        // then round it to the nearest hex coord
        HexCoord::from_floating((x, y))
    }

    /// Round floating point hex space coords to integer hexcoord
    /// see: https://www.redblobgames.com/grids/hexagons/#rounding
    pub fn from_floating((fx, fy): (f32, f32)) -> HexCoord {
        let mut x = fx.round();
        let mut y = fy.round();
        let rem_x = fx - x;
        let rem_y = fy - y;
        if rem_x.abs() >= rem_y.abs() {
            x += (rem_x + 0.5 * rem_y).round();
        } else {
            y += (rem_y + 0.5 * rem_x).round();
        }
        HexCoord(x as i32, y as i32)
    }

    pub fn to_bytes(self) -> [u8; 8] {
        let x: [u8; 4] = self.0.to_ne_bytes();
        let y: [u8; 4] = self.1.to_ne_bytes();
        [x[0], x[1], x[2], x[3], y[0], y[1], y[2], y[3]]
    }

    /// Distance in hex space to other coord.
    /// See: https://www.redblobgames.com/grids/hexagons/#distances-axial
    pub fn distance(&self, other: HexCoord) -> u64 {
        ((self.0 - other.0).abs()
            + (self.0 + self.1 - other.0 - other.1).abs()
            + (self.1 - other.1).abs()) as u64
            / 2
    }

    /// Gets the hexcoords that draw a straight line between self and other
    /// See: https://www.redblobgames.com/grids/hexagons/#line-drawing
    pub fn line_between(&self, other: HexCoord) -> Vec<HexCoord> {
        let start_world = self.to_world(None);
        let end_world = other.to_world(None);

        let dist = self.distance(other);
        let mut results = Vec::new();
        for point in 0..=dist {
            let inter_world = start_world.lerp(end_world, (point as f32) / (dist as f32));
            let inter_hex = HexCoord::from_world(inter_world);
            results.push(inter_hex);
        }
        results
    }

    /// returns all the hex coords that are
    /// within radius number of tiles
    pub fn within_radius(&self, radius: i32) -> Vec<HexCoord> {
        let mut within = Vec::new();
        for x in -radius..radius + 1 {
            for y in max(-radius, (-x) - radius)..min(radius, (-x) + radius) + 1 {
                within.push(HexCoord(x + self.0, y + self.1));
            }
        }
        within
    }
}

/// Marks the parent entity that owns every spawned tile.
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct HexGrid;

/// Marks a single tile of the grid. Lives here rather than in `hex_world` so
/// that gameplay can query tiles without depending on the presentation crate.
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct HexTile;
