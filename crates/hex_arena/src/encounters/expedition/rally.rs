//! Frozen authored travel orders; no player observations enter this state.

use super::*;
use std::collections::{BTreeSet, VecDeque};

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

#[derive(Debug)]
pub(in crate::encounters) struct Control {
    troll: Option<ActorId>,
    started: bool,
    active: bool,
    paths: BTreeMap<PartyId, Vec<Vec3>>,
    cursors: BTreeMap<ActorId, usize>,
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
                let cursor = control.cursors.get(&actor.id).copied().unwrap_or(0);
                path.is_some_and(|path| cursor < path.len())
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
                control.cursors.insert(actor.id, 0);
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
        if self.encounter.expedition.as_ref().is_some_and(|control| {
            control.active
                && !self
                    .actors
                    .iter()
                    .any(|a| Some(a.id) == control.troll && a.hp > 0.0)
        }) {
            self.advance_rally();
        }
    }

    pub(in crate::encounters) fn advance_rally(&mut self) {
        let Some(control) = self.encounter.expedition.as_mut() else {
            return;
        };
        if !control.active {
            return;
        }
        if !self
            .actors
            .iter()
            .any(|a| Some(a.id) == control.troll && a.hp > 0.0)
        {
            control.active = false;
            control.cursors.clear();
            for brain in self.encounter.brains.values_mut() {
                brain.rally_goal = None;
            }
            return;
        }
        control
            .cursors
            .retain(|id, _| self.actors.iter().any(|a| a.id == *id && a.hp > 0.0));
        for (id, cursor) in &mut control.cursors {
            let Some(actor) = self.actors.iter().find(|a| a.id == *id) else {
                continue;
            };
            let Some(path) = actor.party.and_then(|party| control.paths.get(&party)) else {
                continue;
            };
            while path.get(*cursor).is_some_and(|point| {
                actor.feet.with_y(0.0).distance(point.with_y(0.0)) <= 0.8
                    && (actor.feet.y - point.y).abs() <= 0.45
            }) {
                *cursor += 1;
            }
            let gathering = (!control.gathering.is_empty())
                .then(|| {
                    control
                        .gathering
                        .get(usize::from(*id) % control.gathering.len())
                        .copied()
                })
                .flatten();
            if let Some(brain) = self.encounter.brains.get_mut(id) {
                brain.rally_goal = path.get(*cursor).copied().or(gathering);
            }
        }
    }
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
