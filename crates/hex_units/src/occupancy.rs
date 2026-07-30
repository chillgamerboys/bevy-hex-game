//! Canonical exact-surface unit occupancy.
//!
//! Terrain answers where a body could stand. This projection answers where another
//! body already stands. Keeping those facts separate lets movement, combat, AI,
//! formations, and deployment share one `TilePos` rule without teaching gameplay
//! anything about map storage.

use std::collections::{BTreeMap, BTreeSet};

use hex_core::{PartyPath, TilePos, UnitId};
use serde::{Deserialize, Serialize};

/// Why a route conflicts with another body.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum OccupancyBlock {
    /// The route would finish on another body.
    Destination {
        /// Contested exact surface.
        position: TilePos,
        /// Stable identity already standing there.
        occupant: UnitId,
    },
    /// An intermediate step would pass through another body.
    Route {
        /// Blocked exact surface.
        position: TilePos,
        /// Stable identity already standing there.
        occupant: UnitId,
    },
}

/// Stable projection of bodies onto exact terrain surfaces.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnitOccupancy {
    by_position: BTreeMap<TilePos, BTreeSet<UnitId>>,
}

impl UnitOccupancy {
    /// Builds occupancy from stable unit/surface pairs.
    #[must_use]
    pub fn from_positions(positions: impl IntoIterator<Item = (UnitId, TilePos)>) -> Self {
        let mut occupancy = Self::default();
        for (unit, position) in positions {
            occupancy
                .by_position
                .entry(position)
                .or_default()
                .insert(unit);
        }
        occupancy
    }

    /// The lowest stable occupant other than `except`, if one is present.
    #[must_use]
    pub fn occupant(&self, position: TilePos, except: Option<UnitId>) -> Option<UnitId> {
        self.by_position
            .get(&position)
            .into_iter()
            .flatten()
            .copied()
            .find(|unit| Some(*unit) != except)
    }

    /// Whether another body occupies `position`.
    #[must_use]
    pub fn is_occupied(&self, position: TilePos, except: Option<UnitId>) -> bool {
        self.occupant(position, except).is_some()
    }

    /// Whether any exact surface currently contains more than one body.
    #[must_use]
    pub fn has_overlaps(&self) -> bool {
        self.by_position
            .values()
            .any(|occupants| occupants.len() > 1)
    }

    /// Checks every step after the route origin and distinguishes pass-through from
    /// an occupied endpoint.
    pub fn validate_route(&self, path: &[TilePos], mover: UnitId) -> Result<(), OccupancyBlock> {
        let Some(last) = path.last().copied() else {
            return Ok(());
        };
        for &position in path.iter().skip(1) {
            let Some(occupant) = self.occupant(position, Some(mover)) else {
                continue;
            };
            return Err(if position == last {
                OccupancyBlock::Destination { position, occupant }
            } else {
                OccupancyBlock::Route { position, occupant }
            });
        }
        Ok(())
    }

    /// Validates whole-party routes as one atomic movement.
    ///
    /// Members may follow through surfaces vacated by another member. Paths are
    /// continuous animations rather than lock-step waypoint turns, so vector indexes
    /// are not a shared clock. Endpoints must remain unique and two members may never
    /// traverse the same edge in opposite directions.
    pub fn validate_group_routes(paths: &[PartyPath]) -> Result<(), OccupancyBlock> {
        let mut destinations = Self::default();
        for path in paths {
            let Some(&destination) = path.path.last() else {
                continue;
            };
            if let Some(occupant) = destinations.occupant(destination, None) {
                return Err(OccupancyBlock::Destination {
                    position: destination,
                    occupant,
                });
            }
            destinations.relocate(path.member, destination);
        }

        for (index, path) in paths.iter().enumerate() {
            for other in paths.iter().skip(index + 1) {
                for edge in path.path.windows(2) {
                    let [from, to] = edge else {
                        continue;
                    };
                    if other
                        .path
                        .windows(2)
                        .any(|other_edge| other_edge == [*to, *from])
                    {
                        return Err(OccupancyBlock::Route {
                            position: *to,
                            occupant: path.member,
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// Returns a projection without the named units.
    #[must_use]
    pub fn without(&self, units: impl IntoIterator<Item = UnitId>) -> Self {
        let excluded: BTreeSet<_> = units.into_iter().collect();
        Self::from_positions(self.by_position.iter().flat_map(|(&position, occupants)| {
            occupants
                .iter()
                .copied()
                .filter(|unit| !excluded.contains(unit))
                .map(move |unit| (unit, position))
        }))
    }

    /// Moves one stable body to a reserved endpoint in a derived projection.
    pub fn relocate(&mut self, unit: UnitId, destination: TilePos) {
        self.by_position.retain(|_, occupants| {
            occupants.remove(&unit);
            !occupants.is_empty()
        });
        self.by_position
            .entry(destination)
            .or_default()
            .insert(unit);
    }

    /// Deterministic change key for cached previews.
    #[must_use]
    pub fn fingerprint(&self) -> u64 {
        let mut fingerprint = 14_695_981_039_346_656_037u64;
        for (position, occupants) in &self.by_position {
            for bytes in [
                position.coord.x().to_le_bytes(),
                position.coord.y().to_le_bytes(),
                position.level.to_le_bytes(),
            ] {
                for byte in bytes {
                    fingerprint ^= u64::from(byte);
                    fingerprint = fingerprint.wrapping_mul(1_099_511_628_211);
                }
            }
            for unit in occupants {
                for byte in unit.0.to_le_bytes() {
                    fingerprint ^= u64::from(byte);
                    fingerprint = fingerprint.wrapping_mul(1_099_511_628_211);
                }
            }
        }
        fingerprint
    }
}

#[cfg(test)]
mod tests {
    use hex_core::HexCoord;

    use super::*;

    #[test]
    fn elevation_is_part_of_occupancy_identity() {
        let low = TilePos::new(HexCoord::ORIGIN, 2);
        let high = TilePos::new(HexCoord::ORIGIN, 7);
        let occupancy = UnitOccupancy::from_positions([(UnitId(1), high)]);
        assert!(!occupancy.is_occupied(low, None));
        assert!(occupancy.is_occupied(high, None));
    }

    #[test]
    fn routes_distinguish_pass_through_from_occupied_endpoints() {
        let start = TilePos::new(HexCoord::ORIGIN, 1);
        let middle = TilePos::new(HexCoord::from_axial(1, 0), 1);
        let end = TilePos::new(HexCoord::from_axial(2, 0), 1);
        let mover = UnitId(1);
        let blocker = UnitId(2);

        let middle_occupied = UnitOccupancy::from_positions([(mover, start), (blocker, middle)]);
        assert_eq!(
            middle_occupied.validate_route(&[start, middle, end], mover),
            Err(OccupancyBlock::Route {
                position: middle,
                occupant: blocker,
            })
        );

        let end_occupied = UnitOccupancy::from_positions([(mover, start), (blocker, end)]);
        assert_eq!(
            end_occupied.validate_route(&[start, middle, end], mover),
            Err(OccupancyBlock::Destination {
                position: end,
                occupant: blocker,
            })
        );
    }

    #[test]
    fn relocation_replaces_the_units_previous_surface() {
        let start = TilePos::new(HexCoord::ORIGIN, 1);
        let end = TilePos::new(HexCoord::from_axial(1, 0), 1);
        let mut occupancy = UnitOccupancy::from_positions([(UnitId(1), start)]);
        occupancy.relocate(UnitId(1), end);
        assert!(!occupancy.is_occupied(start, None));
        assert_eq!(occupancy.occupant(end, None), Some(UnitId(1)));
    }

    #[test]
    fn group_routes_allow_trailing_but_reject_overlap_and_swaps() {
        let a = TilePos::new(HexCoord::ORIGIN, 1);
        let b = TilePos::new(HexCoord::from_axial(1, 0), 1);
        let c = TilePos::new(HexCoord::from_axial(2, 0), 1);
        let trailing = [
            PartyPath {
                member: UnitId(1),
                path: vec![b, c],
            },
            PartyPath {
                member: UnitId(2),
                path: vec![a, b],
            },
        ];
        assert_eq!(UnitOccupancy::validate_group_routes(&trailing), Ok(()));

        let overlap = [
            PartyPath {
                member: UnitId(1),
                path: vec![a, c],
            },
            PartyPath {
                member: UnitId(2),
                path: vec![b, c],
            },
        ];
        assert!(matches!(
            UnitOccupancy::validate_group_routes(&overlap),
            Err(OccupancyBlock::Destination { position, .. }) if position == c
        ));

        let swap = [
            PartyPath {
                member: UnitId(1),
                path: vec![a, b],
            },
            PartyPath {
                member: UnitId(2),
                path: vec![b, a],
            },
        ];
        assert!(matches!(
            UnitOccupancy::validate_group_routes(&swap),
            Err(OccupancyBlock::Route { .. })
        ));
    }
}
