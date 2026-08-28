//! Recipe-independent ordinary traversal over a semantic V3 volume.
//!
//! The graph is keyed by exact [`TilePos`] values. A ground floor and a floating or
//! underground floor may therefore coexist at one [`HexCoord`] without either
//! replacing the other.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use hex_core::{HexCoord, Level, TilePos, TraversalEndpoint, TraversalProfile};

use super::volume::{SurfaceAccess, VolumePlan};

/// Whether one exact surface would be retained as a node by
/// [`OrdinaryGraph::from_volume`].
///
/// This narrow predicate lets authored-route validators inspect a handful of
/// exact positions without rebuilding the complete 105k-column graph. Ordinary
/// nodes intentionally remain present even with zero headroom; walker admission
/// is decided by [`ordinary_transition_is_admitted`].
pub(crate) fn ordinary_surface_is_node(
    volume: &VolumePlan,
    blocked: Option<&BTreeSet<TilePos>>,
    position: TilePos,
) -> bool {
    volume
        .surfaces
        .get(&position)
        .is_some_and(|metadata| metadata.access == SurfaceAccess::Ordinary)
        && blocked.is_none_or(|blocked| !blocked.contains(&position))
}

/// Whether the complete ordinary graph would publish one symmetric edge.
///
/// The implementation deliberately mirrors [`OrdinaryGraph::from_volume`]:
/// exact ordinary/nonblocked endpoints, adjacent horizontal columns, projected
/// headroom with the same fail-closed default, and walker admission in both
/// directions.
pub(crate) fn ordinary_transition_is_admitted(
    volume: &VolumePlan,
    blocked: Option<&BTreeSet<TilePos>>,
    from: TilePos,
    to: TilePos,
) -> bool {
    if from.coord.distance(to.coord) != 1
        || !ordinary_surface_is_node(volume, blocked, from)
        || !ordinary_surface_is_node(volume, blocked, to)
    {
        return false;
    }
    let from_endpoint = TraversalEndpoint::new(
        from,
        true,
        volume.surface_headroom(from).unwrap_or_default(),
    );
    let to_endpoint =
        TraversalEndpoint::new(to, true, volume.surface_headroom(to).unwrap_or_default());
    TraversalProfile::WALKER.admits_transition(from_endpoint, to_endpoint)
        && TraversalProfile::WALKER.admits_transition(to_endpoint, from_endpoint)
}

/// Deterministic graph of surfaces labelled for ordinary walker traversal.
///
/// Declared ordinary surfaces remain nodes even when malformed headroom or a cliff
/// leaves them isolated. Recipe validators can then detect the disconnected surface
/// instead of silently losing it. Exact feature blockers are the exception: callers
/// may omit them from the graph when validating routes around generated obstacles.
#[derive(Debug)]
pub(crate) struct OrdinaryGraph {
    positions_by_coord: BTreeMap<HexCoord, Vec<TilePos>>,
    endpoints: BTreeMap<TilePos, TraversalEndpoint>,
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

        Self {
            positions_by_coord,
            endpoints,
            neighbors,
        }
    }

    /// Reprojects only columns whose semantic surfaces changed.
    ///
    /// Surface eligibility and headroom are column-local, and traversal edges
    /// only join horizontally adjacent columns. Removing the old vertices and
    /// every incident edge, then rebuilding those vertices and incident edges,
    /// is therefore exactly equivalent to [`Self::from_volume`] for the updated
    /// volume. The returned set contains every old or new vertex incident to the
    /// repair so callers can update cached reachability without rescanning the
    /// complete graph.
    pub(crate) fn refresh_coords(
        &mut self,
        volume: &VolumePlan,
        blocked: Option<&BTreeSet<TilePos>>,
        changed_coords: impl IntoIterator<Item = HexCoord>,
    ) -> BTreeSet<TilePos> {
        let changed_coords = changed_coords.into_iter().collect::<BTreeSet<_>>();
        let mut affected = BTreeSet::<TilePos>::new();

        for coord in &changed_coords {
            let old_positions = self.positions_by_coord.remove(coord).unwrap_or_default();
            for position in old_positions {
                affected.insert(position);
                self.endpoints.remove(&position);
                if let Some(old_neighbors) = self.neighbors.remove(&position) {
                    for neighbor in old_neighbors {
                        affected.insert(neighbor);
                        if let Some(adjacent) = self.neighbors.get_mut(&neighbor) {
                            if let Ok(index) = adjacent.binary_search(&position) {
                                adjacent.remove(index);
                            }
                        }
                    }
                }
            }
        }

        for coord in &changed_coords {
            let positions = volume
                .surfaces
                .range(TilePos::new(*coord, Level::MIN)..=TilePos::new(*coord, Level::MAX))
                .filter_map(|(position, metadata)| {
                    (metadata.access == SurfaceAccess::Ordinary
                        && blocked.is_none_or(|blocked| !blocked.contains(position)))
                    .then_some(*position)
                })
                .collect::<Vec<_>>();
            for position in &positions {
                let headroom = volume.surface_headroom(*position).unwrap_or_default();
                self.endpoints
                    .insert(*position, TraversalEndpoint::new(*position, true, headroom));
                self.neighbors.insert(*position, Vec::new());
                affected.insert(*position);
            }
            if !positions.is_empty() {
                self.positions_by_coord.insert(*coord, positions);
            }
        }

        let coordinate_pairs = changed_coords
            .iter()
            .flat_map(|coord| {
                coord.neighbors().into_iter().map(move |neighbor| {
                    if *coord <= neighbor {
                        (*coord, neighbor)
                    } else {
                        (neighbor, *coord)
                    }
                })
            })
            .collect::<BTreeSet<_>>();
        for (from_coord, to_coord) in coordinate_pairs {
            let Some(from_positions) = self.positions_by_coord.get(&from_coord) else {
                continue;
            };
            let Some(to_positions) = self.positions_by_coord.get(&to_coord) else {
                continue;
            };
            let admitted = from_positions
                .iter()
                .flat_map(|from| to_positions.iter().map(move |to| (*from, *to)))
                .filter(|(from, to)| {
                    self.endpoints
                        .get(from)
                        .copied()
                        .is_some_and(|from_endpoint| {
                            self.endpoints.get(to).copied().is_some_and(|to_endpoint| {
                                TraversalProfile::WALKER
                                    .admits_transition(from_endpoint, to_endpoint)
                                    && TraversalProfile::WALKER
                                        .admits_transition(to_endpoint, from_endpoint)
                            })
                        })
                })
                .collect::<Vec<_>>();
            for (from, to) in admitted {
                if let Some(from_neighbors) = self.neighbors.get_mut(&from) {
                    match from_neighbors.binary_search(&to) {
                        Ok(_) => {}
                        Err(index) => from_neighbors.insert(index, to),
                    }
                }
                if let Some(to_neighbors) = self.neighbors.get_mut(&to) {
                    match to_neighbors.binary_search(&from) {
                        Ok(_) => {}
                        Err(index) => to_neighbors.insert(index, from),
                    }
                }
                affected.insert(from);
                affected.insert(to);
            }
        }

        affected
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

    fn assert_graphs_equal(actual: &OrdinaryGraph, expected: &OrdinaryGraph) {
        assert_eq!(actual.positions_by_coord, expected.positions_by_coord);
        assert_eq!(actual.endpoints, expected.endpoints);
        assert_eq!(actual.neighbors, expected.neighbors);
    }

    #[test]
    fn route_local_node_and_edge_predicates_match_the_complete_graph() {
        let west = HexCoord::ORIGIN;
        let east = east_of(west);
        let north_east = east_of(east);
        let plan = volume([
            (west, vec![stone(0, 5), stone(10, 11)], vec![4, 10]),
            (east, vec![stone(0, 6), stone(10, 11)], vec![5, 10]),
            (north_east, vec![stone(0, 8)], vec![7]),
        ]);
        let blocked_fixture = BTreeSet::from([TilePos::new(east, 5)]);
        let candidates = plan
            .surfaces
            .keys()
            .copied()
            .chain([TilePos::new(west, 99)])
            .collect::<Vec<_>>();

        for blocked in [None, Some(&blocked_fixture)] {
            let graph = OrdinaryGraph::from_volume(&plan, blocked);
            for position in &candidates {
                assert_eq!(
                    ordinary_surface_is_node(&plan, blocked, *position),
                    graph.contains(*position),
                    "node mismatch at {position:?} with blocked={blocked:?}"
                );
                for neighbor in &candidates {
                    assert_eq!(
                        ordinary_transition_is_admitted(&plan, blocked, *position, *neighbor),
                        graph.admits(*position, *neighbor),
                        "edge mismatch {position:?} -> {neighbor:?} with blocked={blocked:?}"
                    );
                }
            }
        }
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

    #[test]
    fn local_column_refresh_matches_a_complete_graph_rebuild() {
        let root = HexCoord::from_axial(0, 0);
        let edited = HexCoord::from_axial(1, 0);
        let goal = HexCoord::from_axial(2, 0);
        let detour_first = HexCoord::from_axial(0, 1);
        let detour_second = HexCoord::from_axial(1, 1);
        let mut plan = volume([
            (root, vec![stone(0, 5)], vec![4]),
            (edited, vec![stone(0, 5), stone(10, 11)], vec![4, 10]),
            (goal, vec![stone(0, 5)], vec![4]),
            (detour_first, vec![stone(0, 5)], vec![4]),
            (detour_second, vec![stone(0, 5)], vec![4]),
        ]);
        let mut graph = OrdinaryGraph::from_volume(&plan, None);
        let old_ground = TilePos::new(edited, 4);
        let old_upper = TilePos::new(edited, 10);

        plan.columns.insert(
            edited,
            VolumeColumn {
                elements: vec![stone(0, 7)],
            },
        );
        assert!(plan.surfaces.remove(&old_ground).is_some());
        assert!(plan.surfaces.remove(&old_upper).is_some());
        let raised = TilePos::new(edited, 6);
        assert!(plan.surfaces.insert(raised, surface()).is_none());

        let affected = graph.refresh_coords(&plan, None, [edited]);
        let rebuilt = OrdinaryGraph::from_volume(&plan, None);
        assert_graphs_equal(&graph, &rebuilt);
        assert!(affected.contains(&old_ground));
        assert!(affected.contains(&old_upper));
        assert!(affected.contains(&raised));
        assert!(affected.contains(&TilePos::new(root, 4)));
        assert!(affected.contains(&TilePos::new(goal, 4)));

        plan.columns.insert(
            edited,
            VolumeColumn {
                elements: vec![stone(0, 5)],
            },
        );
        assert!(plan.surfaces.remove(&raised).is_some());
        assert!(plan.surfaces.insert(old_ground, surface()).is_none());
        graph.refresh_coords(&plan, None, [edited]);
        assert_graphs_equal(&graph, &OrdinaryGraph::from_volume(&plan, None));
    }
}
