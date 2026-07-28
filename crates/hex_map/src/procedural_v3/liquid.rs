//! Map-private directed liquid topology for procedural generator V3.
//!
//! Occupancy remains in [`VolumePlan`]. This layer assigns every non-solid fill
//! run to exactly one stable liquid body and describes flow between exact stacked
//! run tops. Runtime crates receive only the later presentation projection.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

use hex_core::TilePos;

use super::volume::{FillMaterialRole, NonSolidFill, VolumePlan};

/// Stable map-local identity of one connected liquid body.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct LiquidBodyId(pub(crate) u32);

/// Authored presentation class and exact successor of one fill run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LiquidNode {
    pub(crate) state: LiquidFlowState,
    pub(crate) downstream: Option<TilePos>,
}

/// Deterministic presentation class for one liquid run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum LiquidFlowState {
    Still,
    Current,
    Rapid,
    Fall,
}

/// One material-homogeneous connected liquid graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LiquidBodyPlan {
    pub(crate) material: FillMaterialRole,
    pub(crate) nodes: BTreeMap<TilePos, LiquidNode>,
}

/// Complete directed liquid metadata, separate from occupied volume.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct LiquidPlan {
    pub(crate) bodies: BTreeMap<LiquidBodyId, LiquidBodyPlan>,
}

impl LiquidPlan {
    /// Cross-checks exact fill ownership, flow geometry, and graph integrity.
    #[must_use]
    pub(crate) fn validate(&self, volume: &VolumePlan) -> Vec<LiquidIssue> {
        let fill_runs = volume.fill_runs_by_top();
        let mut issues = Vec::new();
        let mut owners = BTreeMap::new();

        for (body_id, body) in &self.bodies {
            if body.nodes.is_empty() {
                issues.push(LiquidIssue::EmptyBody(*body_id));
                continue;
            }
            for position in body.nodes.keys().copied() {
                if let Some(first) = owners.insert(position, *body_id) {
                    issues.push(LiquidIssue::DuplicateNode {
                        position,
                        first,
                        second: *body_id,
                    });
                }
                match fill_runs.get(&position) {
                    None => issues.push(LiquidIssue::NodeWithoutFill {
                        body: *body_id,
                        position,
                    }),
                    Some(fill) if fill.material != body.material => {
                        issues.push(LiquidIssue::MaterialMismatch {
                            body: *body_id,
                            position,
                            expected: fill.material,
                            actual: body.material,
                        });
                    }
                    Some(_) => {}
                }
            }
        }

        for position in fill_runs.keys() {
            if !owners.contains_key(position) {
                issues.push(LiquidIssue::MissingNode(*position));
            }
        }

        for (body_id, body) in &self.bodies {
            validate_body_nodes(*body_id, body, &fill_runs, &mut issues);
            validate_body_graph(*body_id, body, &mut issues);
        }
        issues
    }
}

fn validate_body_nodes(
    body_id: LiquidBodyId,
    body: &LiquidBodyPlan,
    fill_runs: &BTreeMap<TilePos, NonSolidFill>,
    issues: &mut Vec<LiquidIssue>,
) {
    for (position, node) in &body.nodes {
        let Some(downstream) = node.downstream else {
            if node.state != LiquidFlowState::Still {
                issues.push(LiquidIssue::MovingTerminal {
                    body: body_id,
                    position: *position,
                    state: node.state,
                });
            }
            continue;
        };

        if downstream == *position {
            issues.push(LiquidIssue::SelfLoop {
                body: body_id,
                position: *position,
            });
        }
        if !body.nodes.contains_key(&downstream) {
            issues.push(LiquidIssue::DownstreamOutsideBody {
                body: body_id,
                position: *position,
                downstream,
            });
            continue;
        }
        if !position.coord.neighbors().contains(&downstream.coord) {
            issues.push(LiquidIssue::NonAdjacentDownstream {
                body: body_id,
                position: *position,
                downstream,
            });
        }
        if downstream.level > position.level {
            issues.push(LiquidIssue::UphillFlow {
                body: body_id,
                position: *position,
                downstream,
            });
            continue;
        }

        let drop = position.level.saturating_sub(downstream.level);
        match node.state {
            LiquidFlowState::Still | LiquidFlowState::Current | LiquidFlowState::Rapid => {
                if drop > 1 {
                    issues.push(LiquidIssue::NonFallDrop {
                        body: body_id,
                        position: *position,
                        downstream,
                        state: node.state,
                    });
                }
            }
            LiquidFlowState::Fall => {
                if drop < 2 {
                    issues.push(LiquidIssue::ShallowFall {
                        body: body_id,
                        position: *position,
                        downstream,
                    });
                } else if fill_runs
                    .get(position)
                    .is_some_and(|fill| fill.levels.bottom > downstream.level.saturating_add(1))
                {
                    issues.push(LiquidIssue::DiscontinuousFall {
                        body: body_id,
                        position: *position,
                        downstream,
                    });
                }
            }
        }
    }
}

fn validate_body_graph(
    body_id: LiquidBodyId,
    body: &LiquidBodyPlan,
    issues: &mut Vec<LiquidIssue>,
) {
    if body.nodes.is_empty() {
        return;
    }

    let mut reverse = body
        .nodes
        .keys()
        .copied()
        .map(|position| (position, BTreeSet::new()))
        .collect::<BTreeMap<_, _>>();
    let mut indegree = body
        .nodes
        .keys()
        .copied()
        .map(|position| (position, 0_usize))
        .collect::<BTreeMap<_, _>>();
    for (position, node) in &body.nodes {
        let Some(downstream) = node
            .downstream
            .filter(|downstream| body.nodes.contains_key(downstream))
        else {
            continue;
        };
        reverse.entry(downstream).or_default().insert(*position);
        if let Some(count) = indegree.get_mut(&downstream) {
            *count = count.saturating_add(1);
        }
    }

    let mut ready: VecDeque<_> = indegree
        .iter()
        .filter_map(|(position, count)| (*count == 0).then_some(*position))
        .collect();
    let mut visited = 0_usize;
    while let Some(position) = ready.pop_front() {
        visited = visited.saturating_add(1);
        let Some(downstream) = body
            .nodes
            .get(&position)
            .and_then(|node| node.downstream)
            .filter(|downstream| body.nodes.contains_key(downstream))
        else {
            continue;
        };
        let Some(count) = indegree.get_mut(&downstream) else {
            continue;
        };
        *count = count.saturating_sub(1);
        if *count == 0 {
            ready.push_back(downstream);
        }
    }
    if visited != body.nodes.len() {
        issues.push(LiquidIssue::Cycle(body_id));
    }

    let mut terminates = BTreeSet::new();
    let mut frontier: VecDeque<_> = body
        .nodes
        .iter()
        .filter_map(|(position, node)| {
            (node.state == LiquidFlowState::Still && node.downstream.is_none()).then_some(*position)
        })
        .collect();
    while let Some(position) = frontier.pop_front() {
        if !terminates.insert(position) {
            continue;
        }
        if let Some(upstream) = reverse.get(&position) {
            frontier.extend(upstream.iter().copied());
        }
    }
    for (position, node) in &body.nodes {
        if node.state != LiquidFlowState::Still && !terminates.contains(position) {
            issues.push(LiquidIssue::MovingChainDoesNotTerminate {
                body: body_id,
                position: *position,
            });
        }
    }

    let Some(start) = body
        .nodes
        .first_key_value()
        .map(|(position, _node)| *position)
    else {
        return;
    };
    let mut connected = BTreeSet::from([start]);
    let mut frontier = VecDeque::from([start]);
    while let Some(position) = frontier.pop_front() {
        let same_level_neighbors = position
            .coord
            .neighbors()
            .map(|coord| TilePos::new(coord, position.level));
        for neighbor in same_level_neighbors {
            if body.nodes.contains_key(&neighbor) && connected.insert(neighbor) {
                frontier.push_back(neighbor);
            }
        }
        if let Some(downstream) = body
            .nodes
            .get(&position)
            .and_then(|node| node.downstream)
            .filter(|downstream| body.nodes.contains_key(downstream))
        {
            if connected.insert(downstream) {
                frontier.push_back(downstream);
            }
        }
        if let Some(upstream) = reverse.get(&position) {
            for neighbor in upstream {
                if connected.insert(*neighbor) {
                    frontier.push_back(*neighbor);
                }
            }
        }
    }
    if connected.len() != body.nodes.len() {
        issues.push(LiquidIssue::DisconnectedBody(body_id));
    }
}

/// One deterministic map-private liquid contract violation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LiquidIssue {
    EmptyBody(LiquidBodyId),
    MissingNode(TilePos),
    NodeWithoutFill {
        body: LiquidBodyId,
        position: TilePos,
    },
    DuplicateNode {
        position: TilePos,
        first: LiquidBodyId,
        second: LiquidBodyId,
    },
    MaterialMismatch {
        body: LiquidBodyId,
        position: TilePos,
        expected: FillMaterialRole,
        actual: FillMaterialRole,
    },
    DownstreamOutsideBody {
        body: LiquidBodyId,
        position: TilePos,
        downstream: TilePos,
    },
    SelfLoop {
        body: LiquidBodyId,
        position: TilePos,
    },
    NonAdjacentDownstream {
        body: LiquidBodyId,
        position: TilePos,
        downstream: TilePos,
    },
    UphillFlow {
        body: LiquidBodyId,
        position: TilePos,
        downstream: TilePos,
    },
    NonFallDrop {
        body: LiquidBodyId,
        position: TilePos,
        downstream: TilePos,
        state: LiquidFlowState,
    },
    ShallowFall {
        body: LiquidBodyId,
        position: TilePos,
        downstream: TilePos,
    },
    DiscontinuousFall {
        body: LiquidBodyId,
        position: TilePos,
        downstream: TilePos,
    },
    MovingTerminal {
        body: LiquidBodyId,
        position: TilePos,
        state: LiquidFlowState,
    },
    Cycle(LiquidBodyId),
    MovingChainDoesNotTerminate {
        body: LiquidBodyId,
        position: TilePos,
    },
    DisconnectedBody(LiquidBodyId),
}

impl fmt::Display for LiquidIssue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

#[cfg(test)]
mod tests {
    use hex_core::HexCoord;

    use super::*;
    use crate::procedural_v3::volume::{
        LevelInterval, NonSolidFill, SolidMass, SolidMaterialRole, VolumeElement,
    };

    fn coord(x: i32, y: i32, z: i32) -> HexCoord {
        HexCoord::new_cubic(x, y, z)
    }

    fn volume(runs: &[(HexCoord, i32, i32, FillMaterialRole)]) -> VolumePlan {
        let mask = runs.iter().map(|(coord, ..)| *coord).collect();
        let mut volume = VolumePlan::new(mask);
        for (coord, bottom, top, material) in runs {
            volume
                .columns
                .entry(*coord)
                .or_default()
                .elements
                .push(VolumeElement::Fill(NonSolidFill {
                    levels: LevelInterval::new(*bottom, *top),
                    material: *material,
                }));
        }
        volume
    }

    fn body(
        material: FillMaterialRole,
        nodes: impl IntoIterator<Item = (TilePos, LiquidNode)>,
    ) -> LiquidPlan {
        LiquidPlan {
            bodies: BTreeMap::from([(
                LiquidBodyId(7),
                LiquidBodyPlan {
                    material,
                    nodes: nodes.into_iter().collect(),
                },
            )]),
        }
    }

    fn node(state: LiquidFlowState, downstream: Option<TilePos>) -> LiquidNode {
        LiquidNode { state, downstream }
    }

    fn has(issues: &[LiquidIssue], predicate: impl Fn(&LiquidIssue) -> bool) -> bool {
        issues.iter().any(predicate)
    }

    #[test]
    fn flat_flow_and_same_level_still_water_form_valid_bodies() {
        let origin = coord(0, 0, 0);
        let east = coord(1, 0, -1);
        let far_east = coord(2, 0, -2);
        let volume = volume(&[
            (origin, 1, 2, FillMaterialRole::Water),
            (east, 1, 2, FillMaterialRole::Water),
            (far_east, 1, 2, FillMaterialRole::Water),
        ]);
        let first = TilePos::new(origin, 1);
        let second = TilePos::new(east, 1);
        let last = TilePos::new(far_east, 1);
        let flow = body(
            FillMaterialRole::Water,
            [
                (first, node(LiquidFlowState::Current, Some(second))),
                (second, node(LiquidFlowState::Rapid, Some(last))),
                (last, node(LiquidFlowState::Still, None)),
            ],
        );
        assert_eq!(flow.validate(&volume), Vec::new());

        let pond = body(
            FillMaterialRole::Water,
            [
                (first, node(LiquidFlowState::Still, None)),
                (second, node(LiquidFlowState::Still, None)),
                (last, node(LiquidFlowState::Still, None)),
            ],
        );
        assert_eq!(pond.validate(&volume), Vec::new());
    }

    #[test]
    fn empty_plan_exactly_covers_a_volume_without_fill_runs() {
        let volume = VolumePlan::new(BTreeSet::from([coord(0, 0, 0)]));

        assert_eq!(LiquidPlan::default().validate(&volume), Vec::new());
    }

    #[test]
    fn covered_and_stacked_fill_runs_keep_distinct_exact_nodes() {
        let origin = coord(0, 0, 0);
        let mut volume = VolumePlan::new(BTreeSet::from([origin]));
        volume
            .columns
            .get_mut(&origin)
            .expect("the origin is in the mask")
            .elements = vec![
            VolumeElement::Fill(NonSolidFill {
                levels: LevelInterval::new(1, 3),
                material: FillMaterialRole::Water,
            }),
            VolumeElement::Solid(SolidMass {
                levels: LevelInterval::new(3, 4),
                material: SolidMaterialRole::Metal,
                cutaway_for: None,
            }),
            VolumeElement::Fill(NonSolidFill {
                levels: LevelInterval::new(6, 8),
                material: FillMaterialRole::Lava,
            }),
        ];
        let plan = LiquidPlan {
            bodies: BTreeMap::from([
                (
                    LiquidBodyId(1),
                    LiquidBodyPlan {
                        material: FillMaterialRole::Water,
                        nodes: BTreeMap::from([(
                            TilePos::new(origin, 2),
                            node(LiquidFlowState::Still, None),
                        )]),
                    },
                ),
                (
                    LiquidBodyId(2),
                    LiquidBodyPlan {
                        material: FillMaterialRole::Lava,
                        nodes: BTreeMap::from([(
                            TilePos::new(origin, 7),
                            node(LiquidFlowState::Still, None),
                        )]),
                    },
                ),
            ]),
        };

        assert_eq!(plan.validate(&volume), Vec::new());
    }

    #[test]
    fn run_top_ownership_is_exact_and_material_homogeneous() {
        let origin = coord(0, 0, 0);
        let east = coord(1, 0, -1);
        let volume = volume(&[
            (origin, 2, 4, FillMaterialRole::Water),
            (east, 4, 6, FillMaterialRole::Lava),
        ]);
        let water = TilePos::new(origin, 3);
        let lava = TilePos::new(east, 5);
        let extra = TilePos::new(east, 8);
        let mut plan = LiquidPlan {
            bodies: BTreeMap::from([
                (
                    LiquidBodyId(1),
                    LiquidBodyPlan {
                        material: FillMaterialRole::Lava,
                        nodes: BTreeMap::from([
                            (water, node(LiquidFlowState::Still, None)),
                            (extra, node(LiquidFlowState::Still, None)),
                        ]),
                    },
                ),
                (
                    LiquidBodyId(2),
                    LiquidBodyPlan {
                        material: FillMaterialRole::Water,
                        nodes: BTreeMap::from([(water, node(LiquidFlowState::Still, None))]),
                    },
                ),
            ]),
        };
        plan.bodies.insert(
            LiquidBodyId(3),
            LiquidBodyPlan {
                material: FillMaterialRole::Water,
                nodes: BTreeMap::new(),
            },
        );

        let issues = plan.validate(&volume);
        assert!(has(&issues, |issue| matches!(
            issue,
            LiquidIssue::MissingNode(position) if *position == lava
        )));
        assert!(has(&issues, |issue| matches!(
            issue,
            LiquidIssue::NodeWithoutFill { position, .. } if *position == extra
        )));
        assert!(has(&issues, |issue| matches!(
            issue,
            LiquidIssue::DuplicateNode { position, .. } if *position == water
        )));
        assert!(has(&issues, |issue| matches!(
            issue,
            LiquidIssue::MaterialMismatch { position, .. } if *position == water
        )));
        assert!(has(&issues, |issue| matches!(
            issue,
            LiquidIssue::EmptyBody(LiquidBodyId(3))
        )));
    }

    #[test]
    fn exact_downstream_geometry_rejects_invalid_steps() {
        let origin = coord(0, 0, 0);
        let east = coord(1, 0, -1);
        let far_east = coord(2, 0, -2);

        let cases = [
            (
                TilePos::new(origin, 4),
                TilePos::new(origin, 4),
                LiquidFlowState::Current,
                "self",
            ),
            (
                TilePos::new(origin, 4),
                TilePos::new(far_east, 4),
                LiquidFlowState::Current,
                "non-adjacent",
            ),
            (
                TilePos::new(origin, 4),
                TilePos::new(east, 5),
                LiquidFlowState::Current,
                "uphill",
            ),
            (
                TilePos::new(origin, 4),
                TilePos::new(east, 2),
                LiquidFlowState::Rapid,
                "non-fall drop",
            ),
            (
                TilePos::new(origin, 4),
                TilePos::new(east, 3),
                LiquidFlowState::Fall,
                "shallow fall",
            ),
        ];

        for (source, target, state, expected) in cases {
            let volume = if source == target {
                volume(&[(
                    source.coord,
                    source.level,
                    source.level + 1,
                    FillMaterialRole::Water,
                )])
            } else {
                volume(&[
                    (
                        source.coord,
                        source.level,
                        source.level + 1,
                        FillMaterialRole::Water,
                    ),
                    (
                        target.coord,
                        target.level,
                        target.level + 1,
                        FillMaterialRole::Water,
                    ),
                ])
            };
            let plan = if source == target {
                body(
                    FillMaterialRole::Water,
                    [(source, node(state, Some(target)))],
                )
            } else {
                body(
                    FillMaterialRole::Water,
                    [
                        (source, node(state, Some(target))),
                        (target, node(LiquidFlowState::Still, None)),
                    ],
                )
            };
            let issues = plan.validate(&volume);
            let matched = match expected {
                "self" => has(&issues, |issue| {
                    matches!(issue, LiquidIssue::SelfLoop { .. })
                }),
                "non-adjacent" => has(&issues, |issue| {
                    matches!(issue, LiquidIssue::NonAdjacentDownstream { .. })
                }),
                "uphill" => has(&issues, |issue| {
                    matches!(issue, LiquidIssue::UphillFlow { .. })
                }),
                "non-fall drop" => has(&issues, |issue| {
                    matches!(issue, LiquidIssue::NonFallDrop { .. })
                }),
                "shallow fall" => has(&issues, |issue| {
                    matches!(issue, LiquidIssue::ShallowFall { .. })
                }),
                _ => false,
            };
            assert!(matched, "{expected} was not reported: {issues:?}");
        }
    }

    #[test]
    fn downstream_must_name_a_node_in_the_same_body() {
        let origin = coord(0, 0, 0);
        let east = coord(1, 0, -1);
        let source = TilePos::new(origin, 3);
        let target = TilePos::new(east, 3);
        let volume = volume(&[
            (origin, 3, 4, FillMaterialRole::Water),
            (east, 3, 4, FillMaterialRole::Water),
        ]);
        let plan = body(
            FillMaterialRole::Water,
            [(source, node(LiquidFlowState::Current, Some(target)))],
        );

        assert!(has(&plan.validate(&volume), |issue| matches!(
            issue,
            LiquidIssue::DownstreamOutsideBody {
                position,
                downstream,
                ..
            } if *position == source && *downstream == target
        )));
    }

    #[test]
    fn fall_requires_a_contiguous_source_run_down_to_its_landing() {
        let origin = coord(0, 0, 0);
        let east = coord(1, 0, -1);
        let source = TilePos::new(origin, 10);
        let target = TilePos::new(east, 8);
        let plan = body(
            FillMaterialRole::Water,
            [
                (source, node(LiquidFlowState::Fall, Some(target))),
                (target, node(LiquidFlowState::Still, None)),
            ],
        );

        let contiguous = volume(&[
            (origin, 5, 11, FillMaterialRole::Water),
            (east, 8, 9, FillMaterialRole::Water),
        ]);
        assert_eq!(plan.validate(&contiguous), Vec::new());

        let gap = volume(&[
            (origin, 10, 11, FillMaterialRole::Water),
            (east, 8, 9, FillMaterialRole::Water),
        ]);
        assert!(has(&plan.validate(&gap), |issue| matches!(
            issue,
            LiquidIssue::DiscontinuousFall { .. }
        )));
    }

    #[test]
    fn graph_must_be_acyclic_connected_and_end_in_still_water() {
        let origin = coord(0, 0, 0);
        let east = coord(1, 0, -1);
        let south_east = coord(0, 1, -1);
        let first = TilePos::new(origin, 2);
        let second = TilePos::new(east, 2);
        let third = TilePos::new(south_east, 2);
        let graph_volume = volume(&[
            (origin, 2, 3, FillMaterialRole::Water),
            (east, 2, 3, FillMaterialRole::Water),
            (south_east, 2, 3, FillMaterialRole::Water),
        ]);
        let cycle = body(
            FillMaterialRole::Water,
            [
                (first, node(LiquidFlowState::Current, Some(second))),
                (second, node(LiquidFlowState::Rapid, Some(third))),
                (third, node(LiquidFlowState::Current, Some(first))),
            ],
        );
        let issues = cycle.validate(&graph_volume);
        assert!(has(&issues, |issue| matches!(issue, LiquidIssue::Cycle(_))));
        assert!(has(&issues, |issue| matches!(
            issue,
            LiquidIssue::MovingChainDoesNotTerminate { .. }
        )));

        let moving_terminal = body(
            FillMaterialRole::Water,
            [
                (first, node(LiquidFlowState::Current, Some(second))),
                (second, node(LiquidFlowState::Rapid, None)),
                (third, node(LiquidFlowState::Still, None)),
            ],
        );
        let moving_terminal_issues = moving_terminal.validate(&graph_volume);
        assert!(has(&moving_terminal_issues, |issue| matches!(
            issue,
            LiquidIssue::MovingTerminal { .. }
        )));
        assert!(has(&moving_terminal_issues, |issue| matches!(
            issue,
            LiquidIssue::MovingChainDoesNotTerminate { position, .. }
                if *position == first
        )));

        let remote = coord(4, 0, -4);
        let disconnected_volume = volume(&[
            (origin, 2, 3, FillMaterialRole::Water),
            (remote, 2, 3, FillMaterialRole::Water),
        ]);
        let disconnected = body(
            FillMaterialRole::Water,
            [
                (TilePos::new(origin, 2), node(LiquidFlowState::Still, None)),
                (TilePos::new(remote, 2), node(LiquidFlowState::Still, None)),
            ],
        );
        assert!(has(
            &disconnected.validate(&disconnected_volume),
            |issue| matches!(issue, LiquidIssue::DisconnectedBody(_))
        ));
    }

    #[test]
    fn bodies_admit_merges_and_authored_still_inlets() {
        let west = coord(0, 0, 0);
        let south_west = coord(0, 1, -1);
        let sink_coord = coord(1, 0, -1);
        let downstream_coord = coord(2, 0, -2);
        let west = TilePos::new(west, 4);
        let south_west = TilePos::new(south_west, 4);
        let sink = TilePos::new(sink_coord, 4);
        let downstream = TilePos::new(downstream_coord, 4);
        let volume = volume(&[
            (west.coord, 4, 5, FillMaterialRole::Water),
            (south_west.coord, 4, 5, FillMaterialRole::Water),
            (sink.coord, 4, 5, FillMaterialRole::Water),
            (downstream.coord, 4, 5, FillMaterialRole::Water),
        ]);
        let plan = body(
            FillMaterialRole::Water,
            [
                (west, node(LiquidFlowState::Current, Some(sink))),
                (south_west, node(LiquidFlowState::Rapid, Some(sink))),
                (sink, node(LiquidFlowState::Still, Some(downstream))),
                (downstream, node(LiquidFlowState::Still, None)),
            ],
        );

        assert_eq!(plan.validate(&volume), Vec::new());
    }
}
