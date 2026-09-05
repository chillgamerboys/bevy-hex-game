//! Gameplay adapters for the V4 world. No map provider or renderer is imported.
//!
//! Exact transitions reuse the established traversal predicate in a tiny translated
//! coordinate frame. World coordinates and route identities remain exact i64 values.
//! Both turn consumers and a continuously interpolated motion controller use these
//! queries; action-point spending and encounter scheduling stay with the caller.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};

use hex_core::{Headroom, HexCoord, TilePos, TraversalEndpoint, TraversalProfile};
use hex_world_contracts::{ChunkId, QueryResult, Surface, VoxelPosition, WorldQuery};

/// Operational search bounds, independent of total catalogue size.
#[derive(Debug, Clone, Copy)]
pub struct SearchLimits {
    /// Maximum settled surfaces in one synchronous query.
    pub nodes: usize,
    /// Maximum horizontal distance from the start.
    pub radius: u32,
    /// Maximum number of ordinary steps in an accepted route.
    pub steps: u32,
}

impl Default for SearchLimits {
    fn default() -> Self {
        Self {
            nodes: 8192,
            radius: 64,
            steps: 256,
        }
    }
}

/// A route and the exact resident revisions on which its legality depends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldRoute {
    /// Includes the starting and final surfaces; never collapses stacked positions.
    pub waypoints: Vec<VoxelPosition>,
    /// Pin these partitions before accepting the route; release pins on completion.
    pub revisions: BTreeMap<ChunkId, u64>,
}

impl WorldRoute {
    /// Check the revision proof again after the caller acquires residency pins.
    pub fn is_current(&self, query: &impl WorldQuery) -> bool {
        !self.waypoints.is_empty()
            && self
                .revisions
                .iter()
                .all(|(chunk, revision)| query.revision(*chunk) == Some(*revision))
    }
}

/// A bounded planning result; missing terrain cannot become an empty shortcut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteResult {
    /// Complete legal route through available facts.
    Ready(WorldRoute),
    /// Relevant exact facts are not available. The caller may request and retry.
    Pending(BTreeSet<ChunkId>),
    /// No route exists inside the fully explored operational envelope.
    NoRoute,
    /// An explicit work limit was reached; this is not a proof of unreachability.
    LimitReached,
    /// Invalid bounds, coordinate arithmetic, or inconsistent query facts.
    Invalid(String),
}

/// Query one exact supporting surface without changing stacked identity.
pub fn supporting_surface(
    query: &impl WorldQuery,
    position: VoxelPosition,
) -> QueryResult<Option<Surface>> {
    match query.surfaces(position.column) {
        QueryResult::Ready(surfaces) => QueryResult::Ready(
            surfaces
                .into_iter()
                .find(|surface| surface.position == position),
        ),
        QueryResult::Unloaded(chunk) => QueryResult::Unloaded(chunk),
        QueryResult::OutsideWorld => QueryResult::OutsideWorld,
    }
}

/// Apply the existing complete traversal rule to exact V4 surface facts.
pub fn admits_transition(profile: TraversalProfile, from: &Surface, to: &Surface) -> bool {
    if from.position.column.checked_distance(to.position.column) != Ok(1) {
        return false;
    }
    let delta = i64::from(to.position.level) - i64::from(from.position.level);
    if delta > i64::from(profile.max_climb) || -delta > i64::from(profile.max_drop) {
        return false;
    }
    // Translate vertical levels as well as horizontal coordinates. Absolute i32
    // heights near an endpoint must not overflow the legacy positional predicate.
    let Ok(delta) = i32::try_from(delta) else {
        return false;
    };
    let clearance = |surface: &Surface| {
        Headroom(
            surface
                .headroom
                .and_then(|value| i32::try_from(value).ok())
                .unwrap_or(i32::MAX),
        )
    };
    let neighbor = HexCoord::new_cubic(1, 0, -1);
    profile.admits_transition(
        TraversalEndpoint::new(TilePos::new(HexCoord::ORIGIN, 0), true, clearance(from)),
        TraversalEndpoint::new(TilePos::new(neighbor, delta), true, clearance(to)),
    )
}

/// Validate a single leg immediately before a turn or continuous controller moves.
pub fn query_step(
    query: &impl WorldQuery,
    profile: TraversalProfile,
    from: VoxelPosition,
    to: VoxelPosition,
) -> QueryResult<bool> {
    let from = match supporting_surface(query, from) {
        QueryResult::Ready(Some(surface)) => surface,
        QueryResult::Ready(None) => return QueryResult::Ready(false),
        QueryResult::Unloaded(chunk) => return QueryResult::Unloaded(chunk),
        QueryResult::OutsideWorld => return QueryResult::OutsideWorld,
    };
    match supporting_surface(query, to) {
        QueryResult::Ready(Some(to)) => QueryResult::Ready(admits_transition(profile, &from, &to)),
        QueryResult::Ready(None) => QueryResult::Ready(false),
        QueryResult::Unloaded(chunk) => QueryResult::Unloaded(chunk),
        QueryResult::OutsideWorld => QueryResult::OutsideWorld,
    }
}

/// Destination-specific deterministic A*, bounded by work and local distance.
pub fn plan_route(
    query: &impl WorldQuery,
    profile: TraversalProfile,
    start: VoxelPosition,
    goal: VoxelPosition,
    limits: SearchLimits,
) -> RouteResult {
    if limits.nodes == 0
        || limits.radius == 0
        || limits.steps == 0
        || profile.levels_tall <= 0
        || profile.max_climb < 0
        || profile.max_drop < 0
    {
        return RouteResult::Invalid(
            "positive query limits and a valid traversal profile are required".to_owned(),
        );
    }
    match start.column.checked_distance(goal.column) {
        Ok(distance) if distance > u64::from(limits.radius) => return RouteResult::LimitReached,
        Err(error) => return RouteResult::Invalid(error.to_string()),
        _ => {}
    }
    let first = match supporting_surface(query, start) {
        QueryResult::Ready(Some(surface))
            if surface.headroom.is_none_or(|clearance| {
                clearance >= u32::try_from(profile.levels_tall).unwrap_or(u32::MAX)
            }) =>
        {
            surface
        }
        QueryResult::Unloaded(chunk) => return RouteResult::Pending(BTreeSet::from([chunk])),
        _ => return RouteResult::NoRoute,
    };
    let mut frontier = BinaryHeap::from([Reverse((0_u64, 0_u32, start))]);
    let mut best = BTreeMap::from([(start, 0_u32)]);
    let mut facts = BTreeMap::from([(start, first)]);
    let mut predecessor = BTreeMap::new();
    let mut pending = BTreeSet::new();
    let mut settled = 0_usize;
    let mut bounded = false;
    while let Some(Reverse((_score, cost, position))) = frontier.pop() {
        if best.get(&position) != Some(&cost) {
            continue;
        }
        if position == goal {
            return reconstruct(query, start, goal, &predecessor);
        }
        if settled >= limits.nodes {
            return RouteResult::LimitReached;
        }
        settled += 1;
        if cost >= limits.steps {
            bounded = true;
            continue;
        }
        let Some(from) = facts.get(&position).cloned() else {
            return RouteResult::Invalid("search lost its exact surface facts".to_owned());
        };
        let neighbors = match position.column.neighbors() {
            Ok(value) => value,
            Err(error) => return RouteResult::Invalid(error.to_string()),
        };
        for column in neighbors {
            match start.column.checked_distance(column) {
                Ok(distance) if distance > u64::from(limits.radius) => {
                    bounded = true;
                    continue;
                }
                Err(error) => return RouteResult::Invalid(error.to_string()),
                _ => {}
            }
            let surfaces = match query.surfaces(column) {
                QueryResult::Ready(surfaces) => surfaces,
                QueryResult::Unloaded(chunk) => {
                    pending.insert(chunk);
                    continue;
                }
                QueryResult::OutsideWorld => continue,
            };
            for to in surfaces {
                if !admits_transition(profile, &from, &to) {
                    continue;
                }
                let next = cost + 1;
                if best
                    .get(&to.position)
                    .is_some_and(|previous| *previous <= next)
                {
                    continue;
                }
                let heuristic = match to.position.column.checked_distance(goal.column) {
                    Ok(value) => value,
                    Err(error) => return RouteResult::Invalid(error.to_string()),
                };
                best.insert(to.position, next);
                predecessor.insert(to.position, position);
                frontier.push(Reverse((
                    heuristic.saturating_add(u64::from(next)),
                    next,
                    to.position,
                )));
                facts.insert(to.position, to);
            }
        }
    }
    if !pending.is_empty() {
        RouteResult::Pending(pending)
    } else if bounded {
        RouteResult::LimitReached
    } else {
        RouteResult::NoRoute
    }
}

fn reconstruct(
    query: &impl WorldQuery,
    start: VoxelPosition,
    goal: VoxelPosition,
    predecessor: &BTreeMap<VoxelPosition, VoxelPosition>,
) -> RouteResult {
    let mut waypoints = vec![goal];
    let mut current = goal;
    while current != start {
        let Some(previous) = predecessor.get(&current) else {
            return RouteResult::Invalid("incomplete predecessor chain".to_owned());
        };
        waypoints.push(*previous);
        current = *previous;
    }
    waypoints.reverse();
    let mut revisions = BTreeMap::new();
    for position in &waypoints {
        let chunk = position.column.chunk();
        let Some(revision) = query.revision(chunk) else {
            return RouteResult::Pending(BTreeSet::from([chunk]));
        };
        revisions.insert(chunk, revision);
    }
    RouteResult::Ready(WorldRoute {
        waypoints,
        revisions,
    })
}

/// Continuous progress on one already admitted ordinary step.
///
/// This small locomotion adapter models step-assisted hex movement. It is not a
/// general rigid-body solver. Every frame rechecks exact support and aperture;
/// animation interpolates the returned fraction and owns no second terrain model.
#[derive(Debug, Clone, Copy)]
pub struct ContinuousStep {
    /// Exact starting support.
    pub from: VoxelPosition,
    /// Exact destination support.
    pub to: VoxelPosition,
    /// Progress through the leg, in the inclusive interval zero to one.
    pub fraction: f64,
}

impl ContinuousStep {
    /// Advance in real time only while the same exact transition remains legal.
    pub fn advance(
        &mut self,
        query: &impl WorldQuery,
        profile: TraversalProfile,
        elapsed_seconds: f64,
        steps_per_second: f64,
    ) -> QueryResult<bool> {
        if !elapsed_seconds.is_finite()
            || elapsed_seconds < 0.0
            || !steps_per_second.is_finite()
            || steps_per_second <= 0.0
            || !self.fraction.is_finite()
            || !(0.0..=1.0).contains(&self.fraction)
        {
            return QueryResult::Ready(false);
        }
        match query_step(query, profile, self.from, self.to) {
            QueryResult::Ready(true) => {
                self.fraction = (self.fraction + elapsed_seconds * steps_per_second).min(1.0);
                QueryResult::Ready(true)
            }
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_world_contracts::WorldHex;

    #[derive(Default)]
    struct Fixture {
        columns: BTreeMap<WorldHex, QueryResult<Vec<Surface>>>,
        revisions: BTreeMap<ChunkId, u64>,
    }

    impl Fixture {
        fn insert(&mut self, surface: Surface) {
            self.revisions
                .entry(surface.position.column.chunk())
                .or_insert(0);
            let entry = self
                .columns
                .entry(surface.position.column)
                .or_insert_with(|| QueryResult::Ready(Vec::new()));
            if let QueryResult::Ready(surfaces) = entry {
                surfaces.push(surface);
            }
        }
    }

    impl WorldQuery for Fixture {
        fn voxel(&self, position: VoxelPosition) -> QueryResult<Option<String>> {
            match self.columns.get(&position.column) {
                Some(QueryResult::Ready(_)) => QueryResult::Ready(None),
                Some(QueryResult::Unloaded(chunk)) => QueryResult::Unloaded(*chunk),
                _ => QueryResult::OutsideWorld,
            }
        }
        fn surfaces(&self, column: WorldHex) -> QueryResult<Vec<Surface>> {
            self.columns
                .get(&column)
                .cloned()
                .unwrap_or(QueryResult::OutsideWorld)
        }
        fn revision(&self, chunk: ChunkId) -> Option<u64> {
            self.revisions.get(&chunk).copied()
        }
    }

    fn surface(q: i64, level: i32, headroom: Option<u32>) -> Surface {
        Surface {
            position: VoxelPosition {
                column: WorldHex::new(q, 0),
                level,
            },
            material: "stone".to_owned(),
            headroom,
        }
    }

    #[test]
    fn exact_predicate_retains_lateral_aperture_and_stacked_rejection() {
        let low = surface(0, 10, Some(2));
        let high = surface(1, 11, Some(2));
        assert!(!admits_transition(TraversalProfile::WALKER, &low, &high));
        assert!(admits_transition(
            TraversalProfile::WALKER,
            &surface(0, 10, Some(3)),
            &high
        ));
        assert!(!admits_transition(
            TraversalProfile::WALKER,
            &low,
            &surface(0, 11, None)
        ));
        assert!(!admits_transition(
            TraversalProfile::WALKER,
            &low,
            &surface(1, 13, None)
        ));
    }

    #[test]
    fn huge_world_coordinates_and_absolute_levels_never_enter_legacy_math() {
        let q = 9_000_000_000_000_000_000;
        assert!(admits_transition(
            TraversalProfile::WALKER,
            &surface(q, i32::MAX - 1, None),
            &surface(q + 1, i32::MAX, None)
        ));
        assert!(admits_transition(
            TraversalProfile::WALKER,
            &surface(q, i32::MIN, None),
            &surface(q + 1, i32::MIN + 1, None)
        ));
    }

    #[test]
    fn deterministic_route_crosses_negative_chunk_seam_with_revision_proof() {
        let mut fixture = Fixture::default();
        for q in -3..=3 {
            fixture.insert(surface(q, 4, None));
        }
        let start = surface(-3, 4, None).position;
        let goal = surface(3, 4, None).position;
        let planned = plan_route(
            &fixture,
            TraversalProfile::WALKER,
            start,
            goal,
            SearchLimits::default(),
        );
        let RouteResult::Ready(route) = planned else {
            panic!("expected route, got {planned:?}");
        };
        assert_eq!(route.waypoints.len(), 7);
        assert_eq!(route.revisions.len(), 2);
        assert!(route.is_current(&fixture));
        fixture.revisions.insert(goal.column.chunk(), 1);
        assert!(!route.is_current(&fixture));
    }

    #[test]
    fn unavailable_column_blocks_route_and_continuous_progress() {
        let from = surface(15, 4, None);
        let to = surface(16, 4, None);
        let mut fixture = Fixture::default();
        fixture.insert(from.clone());
        fixture.columns.insert(
            to.position.column,
            QueryResult::Unloaded(to.position.column.chunk()),
        );
        assert!(matches!(
            plan_route(
                &fixture,
                TraversalProfile::WALKER,
                from.position,
                to.position,
                SearchLimits::default()
            ),
            RouteResult::Pending(_)
        ));
        let mut step = ContinuousStep {
            from: from.position,
            to: to.position,
            fraction: 0.25,
        };
        assert!(matches!(
            step.advance(&fixture, TraversalProfile::WALKER, 0.1, 3.0),
            QueryResult::Unloaded(_)
        ));
        assert!((step.fraction - 0.25).abs() < f64::EPSILON);
        fixture.insert(to);
        // Publishing replaces the unloaded state rather than appending into it.
        fixture.columns.insert(
            step.to.column,
            QueryResult::Ready(vec![surface(16, 4, None)]),
        );
        assert_eq!(
            step.advance(&fixture, TraversalProfile::WALKER, 0.1, 3.0),
            QueryResult::Ready(true)
        );
        assert!((step.fraction - 0.55).abs() < f64::EPSILON);
    }

    #[test]
    fn removing_support_mid_motion_never_advances_actor() {
        let mut fixture = Fixture::default();
        let from = surface(0, 5, None);
        let to = surface(1, 5, None);
        fixture.insert(from.clone());
        fixture.insert(to.clone());
        let mut step = ContinuousStep {
            from: from.position,
            to: to.position,
            fraction: 0.4,
        };
        fixture
            .columns
            .insert(from.position.column, QueryResult::Ready(Vec::new()));
        assert_eq!(
            step.advance(&fixture, TraversalProfile::WALKER, 0.2, 5.0),
            QueryResult::Ready(false)
        );
        assert!((step.fraction - 0.4).abs() < f64::EPSILON);
    }

    #[test]
    fn work_limits_are_not_reported_as_complete_routes_or_unreachability() {
        let mut fixture = Fixture::default();
        for q in 0..=5 {
            fixture.insert(surface(q, 4, None));
        }
        let limits = SearchLimits {
            nodes: 2,
            radius: 10,
            steps: 10,
        };
        assert_eq!(
            plan_route(
                &fixture,
                TraversalProfile::WALKER,
                surface(0, 4, None).position,
                surface(5, 4, None).position,
                limits
            ),
            RouteResult::LimitReached
        );
        let invalid = SearchLimits { nodes: 0, ..limits };
        assert!(matches!(
            plan_route(
                &fixture,
                TraversalProfile::WALKER,
                surface(0, 4, None).position,
                surface(5, 4, None).position,
                invalid
            ),
            RouteResult::Invalid(_)
        ));
    }

    #[test]
    fn bridge_stack_does_not_teleport_to_ground() {
        let mut fixture = Fixture::default();
        for q in 0..=2 {
            fixture.insert(surface(q, 2, Some(4)));
            fixture.insert(surface(q, 7, None));
        }
        assert_eq!(
            plan_route(
                &fixture,
                TraversalProfile::WALKER,
                surface(0, 7, None).position,
                surface(2, 2, Some(4)).position,
                SearchLimits::default()
            ),
            RouteResult::NoRoute
        );
    }
}
