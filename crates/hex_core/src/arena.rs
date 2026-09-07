//! Explicit world/gameplay contracts for the isolated real-time arena experiment.

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use bevy_ecs::schedule::ScheduleLabel;
use bevy_math::Vec3;

use crate::{ElementId, HexCoord, SubstanceId, TilePos};

/// Fixed-step schedule shared by the arena's world and gameplay producers.
#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArenaTick;

/// World mutations from the previous tick settle before movement or new casts.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum ArenaSystems {
    /// Apply reset, material edits, and correlated damage announcements.
    ApplyTerrain,
    /// Publish complete authoritative occupancy after deferred commands settle.
    PublishTerrain,
    /// Refresh collision, advance actors and projectiles, and publish next effects.
    Simulate,
}

/// World-published physical dimensions; a voxel's upper face is `level * level_height`.
#[derive(Resource, Debug, Clone, Copy)]
pub struct ArenaVoxelGeometry {
    /// Vertical thickness of one voxel in world units.
    pub level_height: f32,
    /// Playable axial radius, inclusive.
    pub radius: u32,
}

impl Default for ArenaVoxelGeometry {
    fn default() -> Self {
        Self {
            level_height: 0.4,
            radius: 12,
        }
    }
}

impl ArenaVoxelGeometry {
    /// Physical upper surface of the exact voxel.
    #[must_use]
    #[expect(
        clippy::cast_precision_loss,
        reason = "the bounded arena uses small integer voxel levels"
    )]
    pub fn top(self, pos: TilePos) -> f32 {
        pos.level as f32 * self.level_height
    }

    /// Physical center of the exact voxel, including its vertical level.
    #[must_use]
    pub fn center(self, pos: TilePos) -> Vec3 {
        pos.coord.to_world(self.top(pos) - self.level_height * 0.5)
    }

    /// Exact voxel containing a finite world-space point; upper faces belong below.
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "arena coordinates are explicitly bounded before conversion"
    )]
    pub fn voxel_at(self, point: Vec3) -> Option<TilePos> {
        if !point.is_finite() || point.y.abs() > 10_000.0 || self.level_height <= 0.0 {
            return None;
        }
        let coord = HexCoord::from_world(point);
        self.contains_column(coord).then_some(TilePos {
            coord,
            level: (point.y / self.level_height).ceil() as i32,
        })
    }

    /// Whether a column belongs to the fixed resident arena footprint.
    #[must_use]
    pub fn contains_column(self, coord: HexCoord) -> bool {
        coord
            .x()
            .unsigned_abs()
            .max(coord.y().unsigned_abs())
            .max(coord.z().unsigned_abs())
            <= self.radius
    }

    /// Canonical exact occupied voxel centers inside a physical Euclidean sphere.
    #[must_use]
    pub fn sphere(self, view: &ArenaTerrainView, center: Vec3, radius: f32) -> Vec<TilePos> {
        if !center.is_finite() || !radius.is_finite() || radius <= 0.0 {
            return Vec::new();
        }
        view.voxels
            .keys()
            .copied()
            .filter(|pos| self.center(*pos).distance_squared(center) <= radius * radius)
            .collect()
    }
}

/// Complete immutable-by-convention occupancy projection; only the map producer writes it.
#[derive(Resource, Debug, Default, Clone)]
pub struct ArenaTerrainView {
    /// Changes on reset or material mutation; partial HP changes do not alter collision.
    pub revision: u64,
    /// Every resident solid voxel keyed by its exact stack-safe identity.
    pub voxels: BTreeMap<TilePos, SubstanceId>,
    /// Human and bot feet positions on valid initial supports.
    pub spawns: [Vec3; 2],
}

/// Accepted material identities published from the same content catalog as damage rules.
#[derive(Resource, Debug, Clone, Copy)]
pub struct ArenaMaterials {
    /// Conjurable stone used for cover and shield walls.
    pub stone: SubstanceId,
    /// Arena's indestructible foundation.
    pub bedrock: SubstanceId,
    /// Upper terrain material.
    pub grass: SubstanceId,
    /// Destructible subsoil.
    pub dirt: SubstanceId,
    /// Damage element shared by both explosive spells.
    pub fire: ElementId,
}

/// Shared reset generation. World and simulation each acknowledge it independently.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct ArenaReset {
    /// Increment to restore the complete authored arena and combat session.
    pub generation: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn physical_round_trip_and_sphere_preserve_stacks() {
        let geometry = ArenaVoxelGeometry::default();
        let lower = TilePos {
            coord: HexCoord::ORIGIN,
            level: 3,
        };
        let upper = TilePos { level: 9, ..lower };
        assert_eq!(geometry.voxel_at(geometry.center(lower)), Some(lower));
        let view = ArenaTerrainView {
            voxels: [(lower, SubstanceId::AIR), (upper, SubstanceId::AIR)].into(),
            ..Default::default()
        };
        assert_eq!(
            geometry.sphere(&view, geometry.center(lower), 0.9),
            vec![lower]
        );
    }
}
