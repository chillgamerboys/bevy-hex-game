//! Explicit world/gameplay contracts for the isolated real-time arena experiment.

use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::prelude::*;
use bevy_ecs::schedule::ScheduleLabel;
use bevy_math::Vec3;

use crate::{ElementId, HexCoord, SubstanceId, TilePos};

/// Highest editable or solid voxel level in the isolated arena, inclusive.
///
/// World mutation enforces this storage bound. Terrain-creating gameplay must
/// filter candidate cells against the same bound before emitting edits. Shields
/// intentionally admit partial footprints when terrain, bounds, or bodies clip them.
pub const ARENA_MAX_LEVEL: i32 = 128;

/// Deterministic world recipes admitted by the isolated combat experiment.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ArenaMap {
    /// Accepted two-actor regression arena.
    #[default]
    Duel,
    /// Compact authored fort.
    Fort,
    /// Three separate encounters in the authored seven-region world.
    SevenRegions,
}

/// Composition of the compact Fort encounter.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ArenaEncounter {
    /// One small dragon.
    #[default]
    Dragon,
    /// Five melee goblins.
    Goblins,
    /// One shaman and three goblins.
    ShamanParty,
    /// One unchanged shadow player.
    Shadow,
}

/// Requested recipe, committed by world publication before simulation consumes it.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ArenaSelection {
    /// Resident map recipe.
    pub map: ArenaMap,
    /// Fort composition; ignored by Duel and Seven Regions.
    pub encounter: ArenaEncounter,
}

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

/// World-published dimensions; upper faces are `level * level_height + vertical_offset`.
#[derive(Resource, Debug, Clone, Copy)]
pub struct ArenaVoxelGeometry {
    /// Vertical thickness of one voxel in world units.
    pub level_height: f32,
    /// Playable axial radius, inclusive.
    pub radius: u32,
    /// Offset added to `level * level_height` to obtain a voxel's upper face.
    pub vertical_offset: f32,
    /// Lowest resident/editable level, inclusive.
    pub min_level: i32,
    /// Highest resident/editable level, inclusive.
    pub max_level: i32,
}

impl Default for ArenaVoxelGeometry {
    fn default() -> Self {
        Self {
            level_height: 0.4,
            radius: 12,
            vertical_offset: 0.0,
            min_level: 0,
            max_level: ARENA_MAX_LEVEL,
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
        pos.level as f32 * self.level_height + self.vertical_offset
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
            level: ((point.y - self.vertical_offset) / self.level_height).ceil() as i32,
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
        let middle = HexCoord::from_world(center);
        // A conservative axial window bounds candidate columns. Exact Euclidean
        // center distance below retains the original spherical selection contract.
        #[expect(
            clippy::cast_possible_truncation,
            reason = "finite bounded blast radius"
        )]
        let reach = radius.ceil().min(10_000.0) as i32 + 2;
        let bound = i32::try_from(self.radius).unwrap_or(i32::MAX);
        let mut result = Vec::new();
        for q in middle.x().saturating_sub(reach).max(-bound)
            ..=middle.x().saturating_add(reach).min(bound)
        {
            for r in middle.y().saturating_sub(reach).max(-bound)
                ..=middle.y().saturating_add(reach).min(bound)
            {
                let coord = HexCoord::from_axial(q, r);
                let start = TilePos::new(coord, i32::MIN);
                let end = TilePos::new(coord, i32::MAX);
                for (pos, _) in view.voxels.range(start..=end) {
                    if self.center(*pos).distance_squared(center) <= radius * radius {
                        result.push(*pos);
                    }
                }
            }
        }
        result.sort_unstable();
        result
    }
}

/// Exact contiguous material run; top and bottom levels are inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArenaSolidSpan {
    /// Lowest voxel and horizontal identity.
    pub bottom: TilePos,
    /// Highest voxel in the same column.
    pub top_level: i32,
    /// World-owned material identity.
    pub substance: SubstanceId,
}

/// Indestructible authored-object occupancy, separate from terrain HP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArenaStaticSpan {
    /// Lowest occupied voxel and horizontal identity.
    pub bottom: TilePos,
    /// Highest occupied voxel in the same column.
    pub top_level: i32,
    /// Whether actors collide with this volume.
    pub blocks_movement: bool,
    /// Whether direct attacks collide with this volume.
    pub blocks_projectiles: bool,
    /// Whether terrain-blocked sight is occluded by this volume.
    pub blocks_sight: bool,
}

/// Authored finite supporting surfaces admitted for one spectator deployment side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArenaDeploymentRegion {
    /// Preferred supporting voxel, always included in `surfaces`.
    pub preferred: TilePos,
    /// Candidate supporting voxels, not guaranteed free body poses or reservations.
    /// Gameplay validates the whole roster against current published geometry.
    pub surfaces: BTreeSet<TilePos>,
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
    /// Recipe belonging to this publication, never an uncommitted menu choice.
    pub selection: ArenaSelection,
    /// Authored region and encounter sites in the published world coordinate space.
    pub anchors: BTreeMap<String, Vec3>,
    /// Optional authored spectator sides, in roster order. No gameplay-generated
    /// search may escape these surfaces onto unrelated floors or rooftops.
    pub battle_deployment: Option<[ArenaDeploymentRegion; 2]>,
    /// Compact solid runs, indexed by column for incremental collision refresh.
    pub columns: BTreeMap<HexCoord, Vec<ArenaSolidSpan>>,
    /// Columns changed by this revision. Consumers missing a revision rebuild fully.
    pub dirty_columns: BTreeSet<HexCoord>,
    /// A reset or recipe change requires rebuilding every collision column.
    pub full_rebuild: bool,
    /// Static object query geometry; not terrain and never a terrain damage target.
    pub static_spans: Vec<ArenaStaticSpan>,
    /// Non-solid liquid volumes used for dry spawn and route validation.
    pub liquids: Vec<ArenaSolidSpan>,
    /// Inclusive protected edit-level intervals per column, including authored
    /// object supports and liquid topology. Placement previews use the same facts.
    pub edit_protected: BTreeMap<HexCoord, Vec<(i32, i32)>>,
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

    #[test]
    fn authored_offset_round_trips_without_changing_voxel_identity() {
        let geometry = ArenaVoxelGeometry {
            vertical_offset: 0.4,
            ..Default::default()
        };
        for level in 0..32 {
            let voxel = TilePos::new(HexCoord::from_axial(2, -3), level);
            assert_eq!(geometry.voxel_at(geometry.center(voxel)), Some(voxel));
            assert!(
                (geometry.top(voxel) - (ArenaVoxelGeometry::default().top(voxel) + 0.4)).abs()
                    < 0.00001
            );
        }
    }

    #[test]
    fn bounded_sphere_matches_full_euclidean_selection() {
        let geometry = ArenaVoxelGeometry::default();
        let view = ArenaTerrainView {
            voxels: HexCoord::ORIGIN
                .within_radius(12)
                .into_iter()
                .flat_map(|coord| {
                    (0..20).map(move |level| (TilePos::new(coord, level), SubstanceId(1)))
                })
                .collect(),
            ..Default::default()
        };
        for center in [
            Vec3::ZERO,
            Vec3::new(7.3, 4.6, -2.8),
            Vec3::new(-15.0, 3.0, 9.0),
        ] {
            for radius in [1.5, 2.5, 4.0, 5.5] {
                let expected: Vec<_> = view
                    .voxels
                    .keys()
                    .copied()
                    .filter(|voxel| {
                        geometry.center(*voxel).distance_squared(center) <= radius * radius
                    })
                    .collect();
                assert_eq!(geometry.sphere(&view, center, radius), expected);
            }
        }
    }
}
