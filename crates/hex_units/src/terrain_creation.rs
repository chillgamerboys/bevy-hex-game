//! Pure legality for spell-created terrain.
//!
//! A cast selects an observed material surface, but creation begins in the first
//! voxel above it. The world still validates every low-level edit; this module owns
//! the gameplay checks answerable from published exact occupancy and unit bodies
//! before a cast is paid for.

use hex_assets::TargetShape;
use hex_core::{Sextant, TilePos, UnitId};

use crate::{volumes, Body, TerrainOccupancy};

/// One exact body projection used while validating terrain creation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreationBody {
    /// Stable unit identity, retained for diagnostics and focused tests.
    pub unit: UnitId,
    /// Exact material surface supporting the body.
    pub support: TilePos,
    /// Body geometry occupying the levels above `support`.
    pub body: Body,
}

/// Why a proposed terrain-creation volume is unsafe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerrainCreationBlock {
    /// A proposed voxel already contains material.
    Material {
        /// Occupied voxel.
        pos: TilePos,
    },
    /// A proposed voxel is the exact surface supporting a unit.
    UnitSupport {
        /// Supporting voxel.
        pos: TilePos,
        /// Supported unit.
        unit: UnitId,
    },
    /// A proposed voxel intersects a unit's body.
    UnitBody {
        /// Intersected voxel.
        pos: TilePos,
        /// Intersected unit.
        unit: UnitId,
    },
}

/// Resolves an authored shape for terrain creation.
///
/// `selected_surface` remains the observed authorization and range anchor. Material
/// begins at `selected_surface.above()`, so a two-level column creates two complete
/// voxels above the floor rather than replacing the floor and raising only one level.
#[must_use]
pub fn resolve_creation_volume(
    shape: &TargetShape,
    caster: TilePos,
    selected_surface: TilePos,
    facing: Option<Sextant>,
) -> Option<Vec<TilePos>> {
    volumes::resolve(shape, caster, selected_surface.above(), facing)
}

/// Validates a complete terrain-creation volume without partially admitting it.
///
/// Unit support and body checks run before material occupancy so a unit standing on
/// an otherwise malformed surface is still protected explicitly. Callers expose only
/// a generic refusal to faction-facing presentation; the detailed block is for
/// authoritative diagnostics and tests.
pub fn validate_creation_volume(
    volume: &[TilePos],
    terrain: &TerrainOccupancy,
    bodies: impl IntoIterator<Item = CreationBody>,
) -> Result<(), TerrainCreationBlock> {
    let bodies: Vec<_> = bodies.into_iter().collect();

    for &pos in volume {
        for body in &bodies {
            if pos == body.support {
                return Err(TerrainCreationBlock::UnitSupport {
                    pos,
                    unit: body.unit,
                });
            }
            let levels_tall = body.body.traversal_profile().levels_tall;
            if levels_tall > 0
                && pos.coord == body.support.coord
                && pos.level > body.support.level
                && pos.level <= body.support.level.saturating_add(levels_tall)
            {
                return Err(TerrainCreationBlock::UnitBody {
                    pos,
                    unit: body.unit,
                });
            }
        }
        if terrain.contains(pos) {
            return Err(TerrainCreationBlock::Material { pos });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use hex_core::{HexCoord, RunBottom, TraversalProfile};

    use super::*;

    fn at(q: i32, r: i32, level: i32) -> TilePos {
        TilePos::new(HexCoord::from_axial(q, r), level)
    }

    #[test]
    fn a_two_level_column_starts_wholly_above_the_selected_floor() {
        let floor = at(2, -1, 4);
        assert_eq!(
            resolve_creation_volume(&TargetShape::Column { height: 2 }, at(0, 0, 4), floor, None,),
            Some(vec![at(2, -1, 5), at(2, -1, 6)])
        );
    }

    #[test]
    fn a_selected_lower_stack_never_jumps_to_the_highest_run() {
        let lower = at(0, 0, 2);
        assert_eq!(
            resolve_creation_volume(&TargetShape::Single, at(1, 0, 2), lower, None),
            Some(vec![at(0, 0, 3)])
        );
    }

    #[test]
    fn existing_material_blocks_the_complete_creation_volume() {
        let terrain =
            TerrainOccupancy::from_runs([(at(0, 0, 2), RunBottom(0)), (at(0, 0, 7), RunBottom(6))])
                .expect("stacked fixture");
        let volume = vec![at(0, 0, 3), at(0, 0, 4), at(0, 0, 6)];

        assert_eq!(
            validate_creation_volume(&volume, &terrain, []),
            Err(TerrainCreationBlock::Material { pos: at(0, 0, 6) })
        );
    }

    #[test]
    fn a_body_and_its_support_are_both_protected() {
        let terrain = TerrainOccupancy::default();
        let unit = CreationBody {
            unit: UnitId(9),
            support: at(0, 0, 4),
            body: Body::new(TraversalProfile::WALKER),
        };

        assert_eq!(
            validate_creation_volume(&[at(0, 0, 4)], &terrain, [unit]),
            Err(TerrainCreationBlock::UnitSupport {
                pos: at(0, 0, 4),
                unit: UnitId(9),
            })
        );
        assert_eq!(
            validate_creation_volume(&[at(0, 0, 5)], &terrain, [unit]),
            Err(TerrainCreationBlock::UnitBody {
                pos: at(0, 0, 5),
                unit: UnitId(9),
            })
        );
        assert_eq!(
            validate_creation_volume(&[at(0, 0, 6)], &terrain, [unit]),
            Err(TerrainCreationBlock::UnitBody {
                pos: at(0, 0, 6),
                unit: UnitId(9),
            })
        );
        assert_eq!(
            validate_creation_volume(&[at(0, 0, 7)], &terrain, [unit]),
            Ok(())
        );
    }
}
