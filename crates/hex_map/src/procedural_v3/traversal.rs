//! Recipe-independent ordinary traversal over a semantic V3 volume.
//!
//! The graph is keyed by exact [`TilePos`] values. A ground floor and a floating or
//! underground floor may therefore coexist at one [`HexCoord`] without either
//! replacing the other.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use hex_core::{HexCoord, TilePos, TraversalEndpoint, TraversalProfile};

use super::volume::{SurfaceAccess, VolumePlan};

/// Deterministic graph of surfaces labelled for ordinary walker traversal.
///
/// Declared ordinary surfaces remain nodes even when malformed headroom or a cliff
/// leaves them isolated. Recipe validators can then detect the disconnected surface
/// instead of silently losing it. Exact feature blockers are the exception: callers
/// may omit them from the graph when validating routes around generated obstacles.
#[derive(Debug)]
pub(crate) struct OrdinaryGraph {
    neighbors: BTreeMap<TilePos, Vec<TilePos>>,
}

impl OrdinaryGraph {
    /// Builds a graph, optionally removing the supplied exact blocked surfaces.
    ///
    /// Blocking one level in a stacked column does not remove any other level at
    /// the same horizontal coordinate.
    #[must_use]
    pub(crate) fn from_volume(volume: &VolumePlan, blocked: Option<&BTreeSet<TilePos>>) -> Self {
        let mut positions_by_coord = BTreeMap::<HexCoord, Vec<TilePos>>::new();
        for (position, metadata) in &volume.surfaces {
            if metadata.access != SurfaceAccess::Ordinary
                || blocked.is_some_and(|blocked| blocked.contains(position))
            {
                continue;
            }
            positions_by_coord
                .entry(position.coord)
                .or_default()
                .push(*position);
        }
        for positions in positions_by_coord.values_mut() {
            positions.sort_unstable();
        }

        let endpoints: BTreeMap<_, _> = positions_by_coord
            .values()
            .flatten()
            .copied()
            .map(|position| {
                let headroom = volume.surface_headroom(position).unwrap_or_default();
                (position, TraversalEndpoint::new(position, true, headroom))
            })
            .collect();
        let mut neighbors: BTreeMap<_, Vec<_>> = endpoints
            .keys()
            .copied()
            .map(|position| (position, Vec::new()))
            .collect();

        for (coord, from_positions) in &positions_by_coord {
            for neighbor_coord in coord.neighbors() {
                if neighbor_coord <= *coord {
                    continue;
                }
                let Some(to_positions) = positions_by_coord.get(&neighbor_coord) else {
                    continue;
                };
                for from in from_positions {
                    for to in to_positions {
                        let Some(from_endpoint) = endpoints.get(from).copied() else {
                            continue;
                        };
                        let Some(to_endpoint) = endpoints.get(to).copied() else {
                            continue;
                        };
                        if TraversalProfile::WALKER.admits_transition(from_endpoint, to_endpoint)
                            && TraversalProfile::WALKER
                                .admits_transition(to_endpoint, from_endpoint)
                        {
                            if let Some(from_neighbors) = neighbors.get_mut(from) {
                                from_neighbors.push(*to);
                            }
                            if let Some(to_neighbors) = neighbors.get_mut(to) {
                                to_neighbors.push(*from);
                            }
                        }
                    }
                }
            }
        }
        for adjacent in neighbors.values_mut() {
            adjacent.sort_unstable();
        }

        Self { neighbors }
    }

    /// Number of ordinary, non-blocked exact surfaces.
    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.neighbors.len()
    }

    /// Whether an exact surface is a node in this graph.
    #[must_use]
    pub(crate) fn contains(&self, position: TilePos) -> bool {
        self.neighbors.contains_key(&position)
    }

    /// Every exact graph node in deterministic position order.
    pub(crate) fn positions(&self) -> impl Iterator<Item = TilePos> + '_ {
        self.neighbors.keys().copied()
    }

    /// Walker-admitted neighbors of one exact surface in deterministic order.
    #[must_use]
    pub(crate) fn neighbors(&self, position: TilePos) -> &[TilePos] {
        self.neighbors.get(&position).map_or(&[], Vec::as_slice)
    }

    /// Whether two exact nodes share one symmetric walker transition.
    #[must_use]
    pub(crate) fn admits(&self, from: TilePos, to: TilePos) -> bool {
        self.neighbors(from).binary_search(&to).is_ok()
    }

    /// Minimum walker distance from `start` to every reachable exact surface.
    #[must_use]
    pub(crate) fn distances_from(&self, start: TilePos) -> BTreeMap<TilePos, u32> {
        if !self.contains(start) {
            return BTreeMap::new();
        }

        let mut distances = BTreeMap::from([(start, 0_u32)]);
        let mut frontier = VecDeque::from([start]);
        while let Some(position) = frontier.pop_front() {
            let Some(distance) = distances.get(&position).copied() else {
                continue;
            };
            for neighbor in self.neighbors(position) {
                if distances.contains_key(neighbor) {
                    continue;
                }
                distances.insert(*neighbor, distance.saturating_add(1));
                frontier.push_back(*neighbor);
            }
        }
        distances
    }

    /// Every surface reachable without traversing any exact blocked node.
    #[must_use]
    pub(crate) fn reachable_avoiding(
        &self,
        start: TilePos,
        blocked: &BTreeSet<TilePos>,
    ) -> BTreeSet<TilePos> {
        if !self.contains(start) || blocked.contains(&start) {
            return BTreeSet::new();
        }

        let mut reachable = BTreeSet::from([start]);
        let mut frontier = VecDeque::from([start]);
        while let Some(position) = frontier.pop_front() {
            for neighbor in self.neighbors(position) {
                if !blocked.contains(neighbor) && reachable.insert(*neighbor) {
                    frontier.push_back(*neighbor);
                }
            }
        }
        reachable
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::procedural_v3::volume::{
        LevelInterval, SolidMass, SolidMaterialRole, SurfaceMetadata, VolumeColumn, VolumeElement,
    };

    fn surface() -> SurfaceMetadata {
        SurfaceMetadata {
            access: SurfaceAccess::Ordinary,
            interior: None,
        }
    }

    fn stone(bottom: i32, top: i32) -> VolumeElement {
        VolumeElement::Solid(SolidMass {
            levels: LevelInterval::new(bottom, top),
            material: SolidMaterialRole::Stone,
            cutaway_for: None,
        })
    }

    fn volume(
        columns: impl IntoIterator<Item = (HexCoord, Vec<VolumeElement>, Vec<i32>)>,
    ) -> VolumePlan {
        let columns: Vec<_> = columns.into_iter().collect();
        let mut plan = VolumePlan::new(columns.iter().map(|(coord, _, _)| *coord).collect());
        for (coord, elements, surface_levels) in columns {
            let previous = plan.columns.insert(coord, VolumeColumn { elements });
            assert!(previous.is_some());
            for level in surface_levels {
                assert!(plan
                    .surfaces
                    .insert(TilePos::new(coord, level), surface())
                    .is_none());
            }
        }
        plan
    }

    fn east_of(coord: HexCoord) -> HexCoord {
        coord
            .neighbors()
            .into_iter()
            .max()
            .expect("a hex has six neighbors")
    }

    #[test]
    fn stacked_surfaces_remain_exact_and_connect_at_matching_levels() {
        let west = HexCoord::ORIGIN;
        let east = east_of(west);
        let plan = volume([
            (west, vec![stone(0, 5), stone(10, 11)], vec![4, 10]),
            (east, vec![stone(0, 5), stone(10, 11)], vec![4, 10]),
        ]);
        let graph = OrdinaryGraph::from_volume(&plan, None);
        let west_ground = TilePos::new(west, 4);
        let west_upper = TilePos::new(west, 10);
        let east_ground = TilePos::new(east, 4);
        let east_upper = TilePos::new(east, 10);

        assert_eq!(graph.len(), 4);
        assert_eq!(
            graph
                .positions()
                .filter(|position| position.coord == west)
                .collect::<Vec<_>>(),
            [west_ground, west_upper]
        );
        assert!(graph.admits(west_ground, east_ground));
        assert!(graph.admits(east_ground, west_ground));
        assert!(graph.admits(west_upper, east_upper));
        assert!(!graph.admits(west_ground, east_upper));
    }

    #[test]
    fn blocker_removes_only_its_exact_stacked_surface() {
        let west = HexCoord::ORIGIN;
        let east = east_of(west);
        let plan = volume([
            (west, vec![stone(0, 5), stone(10, 11)], vec![4, 10]),
            (east, vec![stone(0, 5), stone(10, 11)], vec![4, 10]),
        ]);
        let blocked_upper = TilePos::new(west, 10);
        let blocked = BTreeSet::from([blocked_upper]);
        let graph = OrdinaryGraph::from_volume(&plan, Some(&blocked));

        assert!(!graph.contains(blocked_upper));
        assert!(graph.contains(TilePos::new(west, 4)));
        assert!(graph.admits(TilePos::new(west, 4), TilePos::new(east, 4)));
        assert!(!graph.admits(blocked_upper, TilePos::new(east, 10)));
    }

    #[test]
    fn insufficient_endpoint_headroom_is_retained_but_disconnected() {
        let west = HexCoord::ORIGIN;
        let east = east_of(west);
        let plan = volume([
            (west, vec![stone(0, 5), stone(7, 8)], vec![4]),
            (east, vec![stone(0, 5), stone(6, 7)], vec![4]),
        ]);
        let graph = OrdinaryGraph::from_volume(&plan, None);
        let west_floor = TilePos::new(west, 4);
        let east_floor = TilePos::new(east, 4);

        assert!(graph.contains(east_floor));
        assert!(graph.neighbors(east_floor).is_empty());
        assert!(!graph.admits(west_floor, east_floor));
    }

    #[test]
    fn two_level_cliff_has_no_walker_edge() {
        let west = HexCoord::ORIGIN;
        let east = east_of(west);
        let plan = volume([
            (west, vec![stone(0, 5)], vec![4]),
            (east, vec![stone(0, 7)], vec![6]),
        ]);
        let graph = OrdinaryGraph::from_volume(&plan, None);

        assert!(!graph.admits(TilePos::new(west, 4), TilePos::new(east, 6)));
        assert!(!graph.admits(TilePos::new(east, 6), TilePos::new(west, 4)));
    }

    #[test]
    fn neighbors_distances_and_reachability_are_deterministic() {
        let first = HexCoord::ORIGIN;
        let second = east_of(first);
        let third = east_of(second);
        let plan = volume([
            (third, vec![stone(0, 5)], vec![4]),
            (first, vec![stone(0, 5)], vec![4]),
            (second, vec![stone(0, 5)], vec![4]),
        ]);
        let graph = OrdinaryGraph::from_volume(&plan, None);
        let first = TilePos::new(first, 4);
        let second = TilePos::new(second, 4);
        let third = TilePos::new(third, 4);

        assert_eq!(graph.neighbors(second), &[first, third]);
        assert_eq!(
            graph.distances_from(first),
            BTreeMap::from([(first, 0), (second, 1), (third, 2)])
        );
        assert_eq!(
            graph.reachable_avoiding(first, &BTreeSet::from([second])),
            BTreeSet::from([first])
        );
        assert!(graph
            .distances_from(TilePos::new(first.coord, 99))
            .is_empty());
        assert_eq!(graph.len(), 3);
    }
}
