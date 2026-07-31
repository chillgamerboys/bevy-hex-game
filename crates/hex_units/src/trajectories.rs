//! Exact material trajectories through the stacked hex voxel grid.
//!
//! This module deliberately works only in [`hex_core::TilePos`] integer space. It does not
//! consult rendered spans, transforms, level height, or headroom. A straight segment
//! uses an inclusive supercover: every voxel whose closed prism touches the segment is
//! returned, including face, edge, and corner grazes. That conservative boundary rule
//! makes obstruction stable and direction-independent.

use hex_assets::Trajectory;
use hex_core::{HexCoord, Level, TilePos};

use crate::{KnownTerrainOccupancy, TerrainOccupancy};

/// Resolves the selected surface into the endpoint a trajectory actually reaches.
///
/// Ordinary spells travel to the body/air voxel above their selected surface. A
/// construction spell instead travels to the material surface that authorizes
/// placement, then validates its separate creation volume above that anchor.
#[must_use]
pub const fn trajectory_destination(selected_surface: TilePos, creates_terrain: bool) -> TilePos {
    if creates_terrain {
        selected_surface
    } else {
        selected_surface.above()
    }
}

/// Every voxel touched by the centre-to-centre segment, including both endpoints.
///
/// Results are sorted and deduplicated. Intersection is evaluated with exact rational
/// bounds over the three horizontal cube coordinates and the vertical level, so no
/// floating-point rounding or directional nudge chooses one side of a tie.
#[must_use]
pub fn supercover(source: TilePos, destination: TilePos) -> Vec<TilePos> {
    let q_min = source
        .coord
        .x()
        .min(destination.coord.x())
        .saturating_sub(1);
    let q_max = source
        .coord
        .x()
        .max(destination.coord.x())
        .saturating_add(1);
    let r_min = source
        .coord
        .y()
        .min(destination.coord.y())
        .saturating_sub(1);
    let r_max = source
        .coord
        .y()
        .max(destination.coord.y())
        .saturating_add(1);
    let level_min = source.level.min(destination.level).saturating_sub(1);
    let level_max = source.level.max(destination.level).saturating_add(1);

    let start = [
        i64::from(source.coord.x()),
        i64::from(source.coord.y()),
        i64::from(source.coord.z()),
        i64::from(source.level),
    ];
    let end = [
        i64::from(destination.coord.x()),
        i64::from(destination.coord.y()),
        i64::from(destination.coord.z()),
        i64::from(destination.level),
    ];

    let mut touched = Vec::new();
    for q in q_min..=q_max {
        for r in r_min..=r_max {
            let coord = HexCoord::from_axial(q, r);
            for level in level_min..=level_max {
                let centre = [
                    i64::from(coord.x()),
                    i64::from(coord.y()),
                    i64::from(coord.z()),
                    i64::from(level),
                ];
                if segment_touches_closed_voxel(start, end, centre) {
                    touched.push(TilePos::new(coord, level));
                }
            }
        }
    }
    touched.sort_unstable();
    touched.dedup();
    touched
}

/// Intervening voxels a spell trajectory must keep free of material.
///
/// The true source and destination are excluded: a caster's own launch voxel and the
/// selected material surface authorize the cast rather than blocking it. An arc's
/// deterministic apex is not an endpoint and therefore remains obstructable.
#[must_use]
pub fn trajectory_voxels(
    trajectory: Trajectory,
    source: TilePos,
    destination: TilePos,
) -> Vec<TilePos> {
    let mut touched = match trajectory {
        Trajectory::Direct => supercover(source, destination),
        Trajectory::Arc { rise } => {
            let apex = arc_apex(source, destination, rise);
            let mut voxels = supercover(source, apex);
            voxels.extend(supercover(apex, destination));
            voxels
        }
        Trajectory::None => return Vec::new(),
    };
    touched.retain(|&pos| pos != source && pos != destination);
    touched.sort_unstable();
    touched.dedup();
    touched
}

/// Whether exact published material occupancy leaves this trajectory clear.
#[must_use]
pub fn trajectory_is_clear(
    trajectory: Trajectory,
    source: TilePos,
    destination: TilePos,
    terrain: &TerrainOccupancy,
) -> bool {
    trajectory_voxels(trajectory, source, destination)
        .into_iter()
        .all(|pos| !terrain.contains(pos))
}

/// Whether faction-authorized known material leaves this trajectory clear.
///
/// Presentation, target cycling, and AI use this optimistic projection. Full world
/// occupancy remains exclusive to the authoritative command application boundary.
#[must_use]
pub fn known_trajectory_is_clear(
    trajectory: Trajectory,
    source: TilePos,
    destination: TilePos,
    terrain: &KnownTerrainOccupancy,
) -> bool {
    trajectory_voxels(trajectory, source, destination)
        .into_iter()
        .all(|pos| !terrain.contains(pos))
}

/// Chooses a source/destination-symmetric horizontal midpoint and raises it.
fn arc_apex(source: TilePos, destination: TilePos, rise: u8) -> TilePos {
    let horizontal_source = TilePos::new(source.coord, 0);
    let horizontal_destination = TilePos::new(destination.coord, 0);
    let coord = supercover(horizontal_source, horizontal_destination)
        .into_iter()
        .map(|pos| pos.coord)
        .min_by_key(|coord| {
            let from_source = coord.distance(source.coord);
            let from_destination = coord.distance(destination.coord);
            (
                from_source.max(from_destination),
                from_source.saturating_add(from_destination),
                coord.x(),
                coord.y(),
            )
        })
        .unwrap_or(source.coord);
    TilePos::new(
        coord,
        source
            .level
            .max(destination.level)
            .saturating_add(Level::from(rise)),
    )
}

/// Exact line/closed-cell intersection in scaled cube-plus-level coordinates.
fn segment_touches_closed_voxel(start: [i64; 4], end: [i64; 4], centre: [i64; 4]) -> bool {
    let mut lower = Rational::new(0, 1);
    let mut upper = Rational::new(1, 1);

    for ((start, end), centre) in start.into_iter().zip(end).zip(centre) {
        // Scaling centres by two turns the closed half-voxel constraint into
        // `-1 <= a + b*t <= 1` without fractions.
        let a = 2 * (start - centre);
        let b = 2 * (end - start);
        if b == 0 {
            if !(-1..=1).contains(&a) {
                return false;
            }
            continue;
        }

        let first = Rational::new(-1 - a, b);
        let second = Rational::new(1 - a, b);
        let dimension_lower = first.min(second);
        let dimension_upper = first.max(second);
        lower = lower.max(dimension_lower);
        upper = upper.min(dimension_upper);
        if lower > upper {
            return false;
        }
    }
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rational {
    numerator: i64,
    denominator: i64,
}

impl Rational {
    fn new(mut numerator: i64, mut denominator: i64) -> Self {
        debug_assert_ne!(denominator, 0);
        if denominator < 0 {
            numerator = -numerator;
            denominator = -denominator;
        }
        Self {
            numerator,
            denominator,
        }
    }
}

impl Ord for Rational {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (i128::from(self.numerator) * i128::from(other.denominator))
            .cmp(&(i128::from(other.numerator) * i128::from(self.denominator)))
    }
}

impl PartialOrd for Rational {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use hex_core::RunBottom;

    use super::*;

    fn at(q: i32, r: i32, level: i32) -> TilePos {
        TilePos::new(HexCoord::from_axial(q, r), level)
    }

    fn occupied(voxels: impl IntoIterator<Item = TilePos>) -> TerrainOccupancy {
        TerrainOccupancy::from_runs(voxels.into_iter().map(|pos| (pos, RunBottom(pos.level))))
            .expect("single-voxel runs are valid")
    }

    #[test]
    fn hidden_world_occupancy_cannot_change_authorized_trajectory_legality() {
        let source = at(0, 0, 2);
        let destination = at(3, 0, 2);
        let hidden = at(1, 0, 2);
        let clear_world = TerrainOccupancy::default();
        let blocked_world = occupied([hidden]);
        let same_knowledge = KnownTerrainOccupancy::default();

        assert!(trajectory_is_clear(
            Trajectory::Direct,
            source,
            destination,
            &clear_world,
        ));
        assert!(!trajectory_is_clear(
            Trajectory::Direct,
            source,
            destination,
            &blocked_world,
        ));
        assert!(known_trajectory_is_clear(
            Trajectory::Direct,
            source,
            destination,
            &same_knowledge,
        ));
    }

    #[test]
    fn direct_supercover_is_symmetric_in_all_six_sextants() {
        let source = at(0, 0, 2);
        for sextant in hex_core::Sextant::ALL {
            let mut coord = source.coord;
            for _ in 0..4 {
                coord = coord.neighbor(sextant);
            }
            let destination = TilePos::new(coord, 4);
            let forward = supercover(source, destination);
            let reverse = supercover(destination, source);
            assert_eq!(forward, reverse, "{source:?} -> {destination:?}");
            assert!(forward.contains(&source));
            assert!(forward.contains(&destination));
        }
    }

    #[test]
    fn arc_apex_and_supercover_are_source_destination_symmetric() {
        let source = at(-2, 1, 1);
        let destination = at(3, -2, 4);
        assert_eq!(
            trajectory_voxels(Trajectory::Arc { rise: 3 }, source, destination),
            trajectory_voxels(Trajectory::Arc { rise: 3 }, destination, source)
        );
    }

    #[test]
    fn vertical_and_mixed_segments_include_every_conservative_graze() {
        assert_eq!(
            supercover(at(0, 0, 0), at(0, 0, 2)),
            vec![at(0, 0, 0), at(0, 0, 1), at(0, 0, 2)]
        );

        let mixed = supercover(at(0, 0, 0), at(2, -1, 2));
        assert!(mixed.contains(&at(0, 0, 0)));
        assert!(mixed.contains(&at(2, -1, 2)));
        assert_eq!(mixed, supercover(at(2, -1, 2), at(0, 0, 0)));
    }

    #[test]
    fn face_edge_and_corner_ties_include_both_sides() {
        let diagonal = supercover(at(0, 0, 0), at(2, 1, 0));
        assert!(diagonal.contains(&at(1, 0, 0)));
        assert!(diagonal.contains(&at(1, 1, 0)));

        let rising = supercover(at(0, 0, 0), at(2, 0, 2));
        assert!(rising.contains(&at(1, 0, 1)));
        assert!(rising.contains(&at(1, 0, 0)));
        assert!(rising.contains(&at(1, 0, 2)));
    }

    #[test]
    fn endpoints_authorize_but_an_intervening_wall_blocks() {
        let source = at(0, 0, 1);
        let destination = at(3, 0, 1);
        let endpoints = occupied([source, destination]);
        assert!(trajectory_is_clear(
            Trajectory::Direct,
            source,
            destination,
            &endpoints
        ));

        let wall = occupied([at(1, 0, 1)]);
        assert!(!trajectory_is_clear(
            Trajectory::Direct,
            source,
            destination,
            &wall
        ));
    }

    #[test]
    fn an_arc_clears_a_bridge_while_a_direct_shot_hits_it() {
        let source = at(0, 0, 2);
        let destination = at(4, 0, 2);
        let bridge = occupied([at(2, 0, 2), at(2, -1, 2)]);

        assert!(!trajectory_is_clear(
            Trajectory::Direct,
            source,
            destination,
            &bridge
        ));
        assert!(trajectory_is_clear(
            Trajectory::Arc { rise: 3 },
            source,
            destination,
            &bridge
        ));
    }

    #[test]
    fn direct_under_a_bridge_stays_in_the_gap() {
        let source = at(0, 0, 2);
        let destination = at(4, 0, 2);
        let bridge = occupied([at(2, 0, 5), at(2, -1, 5)]);
        assert!(trajectory_is_clear(
            Trajectory::Direct,
            source,
            destination,
            &bridge
        ));
    }

    #[test]
    fn a_cave_ceiling_blocks_the_arc_apex() {
        let source = at(0, 0, 1);
        let destination = at(4, 0, 1);
        let ceiling = occupied([at(2, 0, 4)]);
        assert!(!trajectory_is_clear(
            Trajectory::Arc { rise: 3 },
            source,
            destination,
            &ceiling
        ));
    }

    #[test]
    fn none_deliberately_bypasses_material() {
        let source = at(0, 0, 1);
        let destination = at(3, 0, 1);
        let wall = occupied([at(1, 0, 1), at(2, 0, 1)]);
        assert!(trajectory_is_clear(
            Trajectory::None,
            source,
            destination,
            &wall
        ));
        assert!(trajectory_voxels(Trajectory::None, source, destination).is_empty());
    }

    #[test]
    fn ordinary_and_creation_casts_resolve_distinct_endpoint_voxels() {
        let surface = at(2, -1, 4);
        assert_eq!(trajectory_destination(surface, false), at(2, -1, 5));
        assert_eq!(trajectory_destination(surface, true), surface);
    }

    #[test]
    fn flat_ground_does_not_block_an_ordinary_level_shot() {
        let standing = at(0, 0, 1);
        let target_surface = at(3, 0, 1);
        let floor = occupied((0..=3).map(|q| at(q, 0, 1)));

        assert!(trajectory_is_clear(
            Trajectory::Direct,
            standing.above(),
            trajectory_destination(target_surface, false),
            &floor,
        ));
    }
}
