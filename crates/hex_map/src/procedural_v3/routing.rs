//! Deterministic routing primitives shared by procedural V3 recipes.
//!
//! The semantic recipes use this module only when several exact corridors must be
//! selected together. Vertex splitting gives every horizontal coordinate unit
//! capacity, so a successful solve cannot hide a shared cell between two routes.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use hex_core::HexCoord;

use super::layout::HexSide;

#[derive(Debug, Clone, Copy)]
struct UnitFlowEdge {
    to: usize,
    reverse: usize,
    capacity: u8,
    initial_capacity: u8,
}

#[derive(Debug)]
struct UnitFlowNetwork {
    adjacency: Vec<Vec<UnitFlowEdge>>,
}

impl UnitFlowNetwork {
    fn new(node_count: usize) -> Self {
        Self {
            adjacency: vec![Vec::new(); node_count],
        }
    }

    fn add_edge(&mut self, from: usize, to: usize) -> Option<()> {
        let reverse_from = self.adjacency.get(to)?.len();
        let reverse_to = self.adjacency.get(from)?.len();
        self.adjacency.get_mut(from)?.push(UnitFlowEdge {
            to,
            reverse: reverse_from,
            capacity: 1,
            initial_capacity: 1,
        });
        self.adjacency.get_mut(to)?.push(UnitFlowEdge {
            to: from,
            reverse: reverse_to,
            capacity: 0,
            initial_capacity: 0,
        });
        Some(())
    }

    fn augment_one(&mut self, source: usize, sink: usize) -> Option<()> {
        let mut predecessor = vec![None; self.adjacency.len()];
        let mut frontier = VecDeque::from([source]);
        *predecessor.get_mut(source)? = Some((source, usize::MAX));
        while let Some(node) = frontier.pop_front() {
            if node == sink {
                break;
            }
            for (edge_index, edge) in self.adjacency.get(node)?.iter().enumerate() {
                if edge.capacity == 0 || predecessor.get(edge.to).is_some_and(Option::is_some) {
                    continue;
                }
                *predecessor.get_mut(edge.to)? = Some((node, edge_index));
                frontier.push_back(edge.to);
            }
        }
        if predecessor.get(sink)?.is_none() {
            return None;
        }
        let mut node = sink;
        while node != source {
            let (previous, edge_index) = predecessor.get(node).copied().flatten()?;
            let reverse = self.adjacency.get(previous)?.get(edge_index)?.reverse;
            self.adjacency
                .get_mut(previous)?
                .get_mut(edge_index)?
                .capacity = 0;
            self.adjacency.get_mut(node)?.get_mut(reverse)?.capacity = 1;
            node = previous;
        }
        Some(())
    }

    fn carries_flow(&self, from: usize, to: usize) -> bool {
        self.adjacency.get(from).is_some_and(|edges| {
            edges.iter().any(|edge| {
                edge.to == to && edge.initial_capacity > 0 && edge.capacity < edge.initial_capacity
            })
        })
    }
}

/// Finds one deterministic set of pairwise vertex-disjoint paths.
///
/// Starts and targets are matched by the flow rather than their input positions.
/// The returned paths remain ordered by `starts`, and each path ends at exactly one
/// unique target.
pub(super) fn vertex_disjoint_paths(
    allowed: &BTreeSet<HexCoord>,
    starts: &[HexCoord],
    targets: &[HexCoord],
) -> Option<Vec<Vec<HexCoord>>> {
    if starts.len() != targets.len()
        || starts.is_empty()
        || starts.iter().any(|start| !allowed.contains(start))
        || targets.iter().any(|target| !allowed.contains(target))
    {
        return None;
    }
    let cells = allowed.iter().copied().collect::<Vec<_>>();
    let indices = cells
        .iter()
        .copied()
        .enumerate()
        .map(|(index, coord)| (coord, index))
        .collect::<BTreeMap<_, _>>();
    let source = cells.len().saturating_mul(2);
    let sink = source.saturating_add(1);
    let mut network = UnitFlowNetwork::new(sink.saturating_add(1));
    for (index, coord) in cells.iter().copied().enumerate() {
        let input = index.saturating_mul(2);
        let output = input.saturating_add(1);
        network.add_edge(input, output)?;
        for side in HexSide::ALL {
            let Some(neighbor_index) = indices.get(&side.neighbor(coord)).copied() else {
                continue;
            };
            network.add_edge(output, neighbor_index.saturating_mul(2))?;
        }
    }
    for start in starts {
        network.add_edge(source, indices.get(start)?.saturating_mul(2))?;
    }
    for target in targets {
        network.add_edge(
            indices.get(target)?.saturating_mul(2).saturating_add(1),
            sink,
        )?;
    }
    for _ in 0..starts.len() {
        network.augment_one(source, sink)?;
    }

    let target_set = targets.iter().copied().collect::<BTreeSet<_>>();
    let mut paths = Vec::with_capacity(starts.len());
    for start in starts {
        let mut coord = *start;
        let mut path = vec![coord];
        let mut visited = BTreeSet::from([coord]);
        loop {
            let index = *indices.get(&coord)?;
            let output = index.saturating_mul(2).saturating_add(1);
            if target_set.contains(&coord) && network.carries_flow(output, sink) {
                break;
            }
            let next = HexSide::ALL.into_iter().find_map(|side| {
                let neighbor = side.neighbor(coord);
                let neighbor_index = indices.get(&neighbor).copied()?;
                network
                    .carries_flow(output, neighbor_index.saturating_mul(2))
                    .then_some(neighbor)
            })?;
            if !visited.insert(next) {
                return None;
            }
            path.push(next);
            coord = next;
        }
        paths.push(path);
    }
    Some(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_three_vertex_disjoint_corridors_deterministically() {
        let allowed = (-3..=3)
            .flat_map(|x| (-3..=3).map(move |y| HexCoord::from_axial(x, y)))
            .collect::<BTreeSet<_>>();
        let starts = [
            HexCoord::from_axial(-3, -1),
            HexCoord::from_axial(-3, 0),
            HexCoord::from_axial(-3, 1),
        ];
        let targets = [
            HexCoord::from_axial(3, -1),
            HexCoord::from_axial(3, 0),
            HexCoord::from_axial(3, 1),
        ];

        let first =
            vertex_disjoint_paths(&allowed, &starts, &targets).expect("three lanes should fit");
        let second =
            vertex_disjoint_paths(&allowed, &starts, &targets).expect("solve should repeat");

        assert_eq!(first, second);
        let mut occupied = BTreeSet::new();
        for path in &first {
            assert!(path
                .windows(2)
                .all(|pair| { matches!(pair, [from, to] if from.distance(*to) == 1) }));
            assert!(path.iter().all(|coord| occupied.insert(*coord)));
        }
        assert_eq!(
            first
                .iter()
                .filter_map(|path| path.last().copied())
                .collect::<BTreeSet<_>>(),
            targets.into_iter().collect()
        );
    }
}
