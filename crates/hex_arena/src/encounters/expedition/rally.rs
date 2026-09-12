//! Frozen authored travel orders; no player observations enter this state.

use super::*;
use std::collections::{BTreeSet, VecDeque};

#[cfg(test)]
#[path = "rally_reentry_tests.rs"]
mod reentry_tests;

/// Read-only evidence of issued rally orders, without actor or target positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct ExpeditionRallySnapshot {
    /// The first positive player hit has been handled, including a fatal opening hit.
    pub triggered: bool,
    /// The Troll is alive and its rally remains in effect.
    pub active: bool,
    /// Distinct forest parties receiving the initial order.
    pub ordered_parties: usize,
    /// Living forest minions receiving the initial order.
    pub ordered_actors: usize,
    /// Ordered minions still alive while the rally is active.
    pub living_ordered_actors: usize,
    /// Living ordered minions that have not reached their gathering position.
    pub remaining_travellers: usize,
}

const REENTRY_TICKS_PER_FRAME: usize = 256;
const REENTRY_TICKS_PER_ACTOR: usize = 64;
const REENTRY_CANDIDATES_PER_UPDATE: usize = 4;
const REENTRY_RETRY_TICKS: u64 = 60;
const DISPLACED_DISTANCE: f32 = 6.0;

#[derive(Debug, Default)]
struct Order {
    cursor: usize,
    joined: bool,
    segment_origin: Option<Vec3>,
    combat_interrupted: bool,
    retry_tick: u64,
    candidate_offset: usize,
    probe: Option<ReentryProbe>,
}

#[derive(Debug)]
struct ReentryProbe {
    cursor: usize,
    target: Vec3,
    start: Vec3,
    actor: Actor,
    revision: u64,
    ticks: usize,
    stalled: usize,
}

#[derive(Debug)]
pub(in crate::encounters) struct Control {
    troll: Option<ActorId>,
    started: bool,
    active: bool,
    paths: BTreeMap<PartyId, Vec<Vec3>>,
    cursors: BTreeMap<ActorId, Order>,
    next_reentry_actor: ActorId,
    gathering: Vec<Vec3>,
    ordered_parties: usize,
    ordered_actors: usize,
}

impl Control {
    pub(super) fn new(
        sites: &ArenaExpeditionSites,
        actors: &[Actor],
        geometry: ArenaVoxelGeometry,
    ) -> Self {
        let destination = sites.encounters.get("forest_troll");
        let mut paths = BTreeMap::new();
        for (index, (name, roles)) in roster().into_iter().enumerate() {
            if !roles.into_iter().any(ExpeditionRole::is_forest_minion) {
                continue;
            }
            let entry = sites
                .encounters
                .get(&name)
                .and_then(|site| site.rally_entry.as_deref());
            let target = destination.and_then(|site| site.rally_entry.as_deref());
            if let Some(supports) = entry
                .zip(target)
                .and_then(|(from, to)| route(sites, from, to))
            {
                if let Ok(party) = PartyId::try_from(index) {
                    paths.insert(
                        party,
                        supports
                            .into_iter()
                            .map(|p| p.coord.to_world(geometry.top(p) + SKIN))
                            .collect(),
                    );
                }
            }
        }
        let gathering = destination
            .into_iter()
            .flat_map(|site| &site.deployment.surfaces)
            .map(|p| p.coord.to_world(geometry.top(*p) + SKIN))
            .collect();
        Self {
            troll: actors
                .iter()
                .find(|a| a.expedition_role() == Some(ExpeditionRole::Troll))
                .map(|a| a.id),
            started: false,
            active: false,
            paths,
            cursors: BTreeMap::new(),
            next_reentry_actor: 0,
            gathering,
            ordered_parties: 0,
            ordered_actors: 0,
        }
    }
}

/// Stable breadth-first traversal over authored edges. Each edge retains its
/// complete ordered supports, including reverse traversal and stacked altitude.
fn route(sites: &ArenaExpeditionSites, from: &str, to: &str) -> Option<Vec<TilePos>> {
    let mut queue = VecDeque::from([from.to_owned()]);
    let mut reached = BTreeSet::from([from.to_owned()]);
    let mut parent = BTreeMap::<String, (String, Vec<TilePos>)>::new();
    while let Some(node) = queue.pop_front() {
        if node == to {
            break;
        }
        for edge in sites.routes.values() {
            let (next, mut supports) = if edge.from == node {
                (edge.to.as_str(), edge.supports.clone())
            } else if edge.to == node {
                let mut reversed = edge.supports.clone();
                reversed.reverse();
                (edge.from.as_str(), reversed)
            } else {
                continue;
            };
            if supports.is_empty() || !reached.insert(next.to_owned()) {
                continue;
            }
            // Shared junction supports are emitted only once after concatenation.
            supports.dedup();
            parent.insert(next.to_owned(), (node.clone(), supports));
            queue.push_back(next.to_owned());
        }
    }
    if !reached.contains(to) {
        return None;
    }
    let mut segments = Vec::new();
    let mut node = to;
    while node != from {
        let (previous, supports) = parent.get(node)?;
        segments.push(supports.as_slice());
        node = previous;
    }
    let mut path = vec![*sites.route_nodes.get(from)?];
    for supports in segments.into_iter().rev() {
        path.extend_from_slice(supports);
    }
    path.dedup();
    Some(path)
}

impl ArenaSession {
    /// Actual issued rally counts; does not disclose hidden player or enemy positions.
    #[must_use]
    pub fn expedition_rally_status(&self) -> Option<ExpeditionRallySnapshot> {
        let control = self.encounter.expedition.as_ref()?;
        let living: Vec<_> = self
            .actors
            .iter()
            .filter(|a| control.active && a.hp > 0.0 && control.cursors.contains_key(&a.id))
            .collect();
        let remaining_travellers = living
            .iter()
            .filter(|actor| {
                let path = actor.party.and_then(|party| control.paths.get(&party));
                let order = control.cursors.get(&actor.id);
                order.is_none_or(|order| !order.joined)
                    || path.is_some_and(|path| order.is_none_or(|order| order.cursor < path.len()))
                    || control.gathering.is_empty()
                    || control
                        .gathering
                        .get(usize::from(actor.id) % control.gathering.len())
                        .is_none_or(|goal| actor.feet.distance(*goal) > 0.8)
            })
            .count();
        Some(ExpeditionRallySnapshot {
            triggered: control.started,
            active: control.active,
            ordered_parties: control.ordered_parties,
            ordered_actors: control.ordered_actors,
            living_ordered_actors: living.len(),
            remaining_travellers,
        })
    }

    pub(in crate::encounters) fn rally_on_player_damage(
        &mut self,
        owner: ActorId,
        victim: ActorId,
    ) {
        if self.human_actor_id() != Some(owner) {
            return;
        }
        let Some(control) = self.encounter.expedition.as_mut() else {
            return;
        };
        if control.started || control.troll != Some(victim) {
            return;
        }
        control.started = true;
        control.active = self.actors.iter().any(|a| a.id == victim && a.hp > 0.0);
        if !control.active {
            return;
        }
        for actor in self.actors.iter().filter(|a| {
            a.hp > 0.0
                && a.expedition_role()
                    .is_some_and(ExpeditionRole::is_forest_minion)
        }) {
            if actor
                .party
                .is_some_and(|party| control.paths.contains_key(&party))
            {
                control.cursors.insert(actor.id, Order::default());
            }
        }
        control.ordered_actors = control.cursors.len();
        control.ordered_parties = self
            .actors
            .iter()
            .filter(|a| control.cursors.contains_key(&a.id))
            .filter_map(|a| a.party)
            .collect::<BTreeSet<_>>()
            .len();
        self.notice = "The Troll calls every surviving forest party to the ancient grove!".into();
    }

    pub(in crate::encounters) fn cancel_dead_rally(&mut self) {
        let Some(control) = self.encounter.expedition.as_mut() else {
            return;
        };
        if control.active
            && !self
                .actors
                .iter()
                .any(|a| Some(a.id) == control.troll && a.hp > 0.0)
        {
            control.active = false;
            control.cursors.clear();
            for brain in self.encounter.brains.values_mut() {
                brain.rally_goal = None;
            }
        }
    }

    pub(in crate::encounters) fn advance_rally(
        &mut self,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        tuning: &ArenaTuning,
    ) {
        self.cancel_dead_rally();
        let Some(control) = self.encounter.expedition.as_mut() else {
            return;
        };
        if !control.active {
            return;
        }
        control
            .cursors
            .retain(|id, _| self.actors.iter().any(|a| a.id == *id && a.hp > 0.0));
        let mut ids: Vec<_> = control.cursors.keys().copied().collect();
        let start = ids.partition_point(|id| *id < control.next_reentry_actor);
        ids.rotate_left(start);
        let mut remaining = REENTRY_TICKS_PER_FRAME;
        for id in ids {
            let Some(actor) = self.actors.iter().find(|a| a.id == id) else {
                continue;
            };
            let Some(path) = actor.party.and_then(|party| control.paths.get(&party)) else {
                continue;
            };
            let gathering = (!control.gathering.is_empty())
                .then(|| {
                    control
                        .gathering
                        .get(usize::from(id) % control.gathering.len())
                        .copied()
                })
                .flatten();
            let Some(order) = control.cursors.get_mut(&id) else {
                continue;
            };
            let fighting = self
                .encounter
                .runtime
                .iter()
                .find(|party| Some(party.snapshot.id) == actor.party)
                .is_some_and(|party| {
                    party.snapshot.phase == PartyPhase::Active && party.knowledge.is_some()
                });
            let resumed = order.combat_interrupted && !fighting;
            order.combat_interrupted = fighting;
            let goal = path.get(order.cursor).copied().or(gathering);
            let displaced = order.segment_origin.zip(goal).is_some_and(|(start, end)| {
                let segment = end - start;
                let fraction = ((actor.feet - start).dot(segment)
                    / segment.length_squared().max(0.001))
                .clamp(0.0, 1.0);
                actor.feet.distance(start + segment * fraction) > DISPLACED_DISTANCE
            });
            if resumed || (order.joined && displaced) {
                order.joined = false;
                order.candidate_offset = 0;
                order.probe = None;
                order.retry_tick = self.tick;
            }
            if !order.joined && !fighting && remaining > 0 && self.tick >= order.retry_tick {
                let allowance = remaining.min(REENTRY_TICKS_PER_ACTOR);
                let spent = reenter(
                    order,
                    actor,
                    path,
                    gathering,
                    &self.collision,
                    world,
                    geometry,
                    tuning,
                    self.tick,
                    allowance,
                );
                remaining -= spent;
                control.next_reentry_actor = id.saturating_add(1);
            }
            if order.joined {
                while path.get(order.cursor).is_some_and(|point| {
                    actor.feet.with_y(0.0).distance(point.with_y(0.0)) <= 0.8
                        && (actor.feet.y - point.y).abs() <= 0.45
                }) {
                    order.segment_origin = path.get(order.cursor).copied();
                    order.cursor += 1;
                }
            }
            if let Some(brain) = self.encounter.brains.get_mut(&id) {
                // Holding an unresolved call prevents a kited survivor returning
                // home while the bounded physical probe runs. Admitted combat
                // still overrides this own-position travel goal in Brain::intent.
                brain.rally_goal = if order.joined {
                    path.get(order.cursor).copied().or(gathering)
                } else {
                    Some(actor.feet)
                };
            }
        }
    }
}

/// Incremental controller proof: at most the global/per-actor tick budget, with
/// round-robin actor service. A blocked candidate gets a bounded next attempt;
/// no whole-forest burst of thirty-second route simulations occurs in one tick.
fn reenter(
    order: &mut Order,
    actor: &Actor,
    path: &[Vec3],
    gathering: Option<Vec3>,
    collision: &CollisionWorld,
    world: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    tuning: &ArenaTuning,
    tick: u64,
    allowance: usize,
) -> usize {
    if order.probe.as_ref().is_some_and(|probe| {
        probe.revision != world.revision || probe.start.distance(actor.feet) > 0.1
    }) {
        order.probe = None;
        order.candidate_offset = 0;
    }
    let mut candidates: Vec<_> = path
        .iter()
        .copied()
        .enumerate()
        .skip(order.cursor)
        .chain(gathering.map(|point| (path.len(), point)))
        .collect();
    candidates.sort_by(|(ia, a), (ib, b)| {
        a.distance_squared(actor.feet)
            .total_cmp(&b.distance_squared(actor.feet))
            .then_with(|| ib.cmp(ia))
    });
    let mut spent = 0;
    let mut attempted = 0;
    while spent < allowance && attempted < REENTRY_CANDIDATES_PER_UPDATE {
        if order.probe.is_none() {
            let Some((cursor, target)) = candidates.get(order.candidate_offset).copied() else {
                order.candidate_offset = 0;
                order.retry_tick = tick + REENTRY_RETRY_TICKS;
                break;
            };
            order.probe = Some(ReentryProbe {
                cursor,
                target,
                start: actor.feet,
                actor: actor.clone(),
                revision: world.revision,
                ticks: 0,
                stalled: 0,
            });
            spent += 1; // Candidate validation consumes budget even at zero distance.
        }
        let Some(probe) = order.probe.as_mut() else {
            break;
        };
        let profile = actor.expedition_tuning(tuning);
        let mut valid = steering::contained(&probe.actor, geometry)
            && shapes::clear(
                collision,
                &probe.actor,
                probe.actor.feet,
                probe.actor.body_yaw,
            )
            && dry(&probe.actor, world, geometry)
            && shapes::ground(collision, &probe.actor, probe.actor.feet, 0.45).is_some();
        while valid
            && spent < allowance
            && probe.ticks < 3600
            && (probe
                .actor
                .feet
                .with_y(0.0)
                .distance(probe.target.with_y(0.0))
                > 0.3
                || !probe.actor.grounded
                || shapes::ground(collision, &probe.actor, probe.actor.feet, 0.05).is_none())
        {
            let before = probe.actor.feet;
            let settling = before.with_y(0.0).distance(probe.target.with_y(0.0)) <= 0.3;
            // A downhill entry can arrive horizontally before touching its tread.
            // Finish through neutral controller ticks, charged to the same bounded
            // proof budget and retained across frames if this allowance runs out.
            let direction = if settling {
                Vec3::ZERO
            } else {
                (probe.target - before).with_y(0.0).normalize_or_zero()
            };
            motion::tick(
                &mut probe.actor,
                direction,
                true,
                false,
                false,
                collision,
                &profile.encounters,
            );
            spent += 1;
            probe.ticks += 1;
            probe.stalled =
                if !settling && (probe.actor.feet - before).with_y(0.0).dot(direction) < 0.001 {
                    probe.stalled + 1
                } else {
                    0
                };
            valid = probe.stalled < 24
                && steering::contained(&probe.actor, geometry)
                && shapes::clear(
                    collision,
                    &probe.actor,
                    probe.actor.feet,
                    probe.actor.body_yaw,
                )
                && dry(&probe.actor, world, geometry)
                && probe.actor.feet.y >= before.y - 0.45
                && shapes::ground(collision, &probe.actor, probe.actor.feet, 0.45).is_some();
        }
        let arrived = probe
            .actor
            .feet
            .with_y(0.0)
            .distance(probe.target.with_y(0.0))
            <= 0.3;
        if valid
            && arrived
            && probe.actor.grounded
            && (probe.actor.feet.y - probe.target.y).abs() <= 0.45
            && shapes::ground(collision, &probe.actor, probe.actor.feet, 0.05).is_some()
        {
            order.cursor = probe.cursor;
            order.joined = true;
            order.segment_origin = Some(actor.feet);
            order.probe = None;
            break;
        }
        if !valid
            || (arrived && (probe.actor.feet.y - probe.target.y).abs() > 0.45)
            || probe.ticks >= 3600
        {
            order.probe = None;
            order.candidate_offset += 1;
            attempted += 1;
        } else {
            break;
        }
    }
    if !order.joined && order.probe.is_none() && attempted >= REENTRY_CANDIDATES_PER_UPDATE {
        order.retry_tick = tick + REENTRY_RETRY_TICKS;
    }
    spent
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::arena::ArenaExpeditionRoute;

    #[test]
    fn routes_preserve_winding_supports_altitude_and_reverse_edges() {
        let points = [
            TilePos::new(HexCoord::from_axial(0, 0), 2),
            TilePos::new(HexCoord::from_axial(1, 0), 3),
            TilePos::new(HexCoord::from_axial(1, 1), 4),
            TilePos::new(HexCoord::from_axial(2, 1), 5),
        ];
        let mut sites = ArenaExpeditionSites::default();
        for (name, pos) in [
            ("camp", points.first()),
            ("bend", points.get(2)),
            ("boss", points.last()),
        ] {
            sites.route_nodes.insert(name.into(), *pos.expect("point"));
        }
        for (name, from, to, supports) in [
            (
                "01",
                "camp",
                "bend",
                points.get(..3).expect("first edge").to_vec(),
            ),
            (
                "02",
                "boss",
                "bend",
                points
                    .get(2..)
                    .expect("reverse edge")
                    .iter()
                    .rev()
                    .copied()
                    .collect(),
            ),
        ] {
            sites.routes.insert(
                name.into(),
                ArenaExpeditionRoute {
                    from: from.into(),
                    to: to.into(),
                    clearance_levels: 8,
                    ribbon: supports.iter().copied().collect(),
                    supports,
                },
            );
        }
        assert_eq!(route(&sites, "camp", "boss"), Some(points.to_vec()));
        assert_eq!(
            route(&sites, "boss", "camp"),
            Some(points.into_iter().rev().collect())
        );
        assert!(route(&sites, "missing", "boss").is_none());
    }
}
