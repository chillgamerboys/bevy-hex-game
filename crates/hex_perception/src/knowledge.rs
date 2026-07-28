//! Faction-scoped current and remembered world knowledge.

use std::collections::BTreeMap;

use bevy_ecs::prelude::Resource;
use hex_core::{KnowledgeState, LocalMapKnowledge, TilePos, UnitId};
use hex_units::Faction;

use crate::{
    FactionObservation, FactionObservations, ObservedUnit, SurfaceSnapshot, SurfaceSnapshots,
};

/// One remembered or currently observed exact terrain snapshot.
///
/// Unknown positions are absent from [`FactionKnowledge`] entirely, so this type can
/// represent only Remembered or Observed facts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KnownSurface {
    state: KnowledgeState,
    snapshot: SurfaceSnapshot,
}

impl KnownSurface {
    /// Whether this snapshot is remembered or currently observed.
    #[must_use]
    pub const fn state(self) -> KnowledgeState {
        self.state
    }

    /// Exact terrain facts captured at the last observation.
    #[must_use]
    pub const fn snapshot(self) -> SurfaceSnapshot {
        self.snapshot
    }
}

/// Authoritative spatial knowledge held by one faction.
///
/// Terrain remains as an exact last-seen snapshot after sight is lost. Units are
/// current-observation facts only and are cleared before every refresh.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct FactionKnowledge {
    surfaces: BTreeMap<TilePos, KnownSurface>,
    units: BTreeMap<UnitId, ObservedUnit>,
}

impl FactionKnowledge {
    /// Creates empty faction knowledge.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a remembered or observed exact surface.
    #[must_use]
    pub fn surface(&self, pos: TilePos) -> Option<KnownSurface> {
        self.surfaces.get(&pos).copied()
    }

    /// Returns Unknown for an absent surface.
    #[must_use]
    pub fn state(&self, pos: TilePos) -> KnowledgeState {
        self.surface(pos)
            .map_or(KnowledgeState::Unknown, KnownSurface::state)
    }

    /// Iterates over remembered and observed surfaces in exact-position order.
    pub fn surfaces(&self) -> impl Iterator<Item = (TilePos, KnownSurface)> + '_ {
        self.surfaces
            .iter()
            .map(|(position, known)| (*position, *known))
    }

    /// Returns a currently observed unit.
    #[must_use]
    pub fn unit(&self, id: UnitId) -> Option<ObservedUnit> {
        self.units.get(&id).copied()
    }

    /// Iterates over currently observed units in stable identity order.
    pub fn units(&self) -> impl Iterator<Item = (UnitId, ObservedUnit)> + '_ {
        self.units.iter().map(|(id, unit)| (*id, *unit))
    }

    /// Number of remembered or observed surfaces.
    #[must_use]
    pub fn surface_count(&self) -> usize {
        self.surfaces.len()
    }

    /// Number of currently observed units.
    #[must_use]
    pub fn unit_count(&self) -> usize {
        self.units.len()
    }

    /// Whether this faction has never observed any current or remembered fact.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.surfaces.is_empty() && self.units.is_empty()
    }

    fn apply(&mut self, current: &SurfaceSnapshots, observation: &FactionObservation) {
        for known in self.surfaces.values_mut() {
            known.state = KnowledgeState::Remembered;
        }
        self.units.clear();

        for pos in observation.surfaces() {
            if let Some(snapshot) = current.get(pos) {
                self.surfaces.insert(
                    pos,
                    KnownSurface {
                        state: KnowledgeState::Observed,
                        snapshot,
                    },
                );
            } else {
                // The exact old position is in sight and no longer exists. Retaining
                // its snapshot would turn deletion into a permanent phantom surface.
                self.surfaces.remove(&pos);
            }
        }

        self.units.extend(
            observation.units().filter(|(_, unit)| {
                observation.observes(unit.pos) && current.get(unit.pos).is_some()
            }),
        );
    }
}

/// Spatial knowledge for both current factions without ordering [`Faction`].
///
/// The slots stay private and are selected with an exhaustive match. This avoids
/// pretending `Faction` has a meaningful sort order merely to use it as a map key.
#[derive(Resource, Debug, Default, Clone, PartialEq)]
pub struct FactionMapKnowledge {
    player: FactionKnowledge,
    hostile: FactionKnowledge,
}

impl FactionMapKnowledge {
    /// Creates empty knowledge for both factions.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns one faction's authoritative spatial knowledge.
    #[must_use]
    pub const fn faction(&self, faction: Faction) -> &FactionKnowledge {
        match faction {
            Faction::Player => &self.player,
            Faction::Hostile => &self.hostile,
        }
    }

    fn faction_mut(&mut self, faction: Faction) -> &mut FactionKnowledge {
        match faction {
            Faction::Player => &mut self.player,
            Faction::Hostile => &mut self.hostile,
        }
    }

    /// Builds the compact movement-facing projection for the local player faction.
    #[must_use]
    pub fn player_local_map_knowledge(&self) -> LocalMapKnowledge {
        let mut local = LocalMapKnowledge::new();
        for (_, known) in self.player.surfaces() {
            let snapshot = known.snapshot();
            local.set(
                known.state(),
                snapshot.traversal_endpoint(),
                snapshot.blocked,
            );
        }
        local
    }

    /// Replaces an existing local-player projection with the current knowledge.
    pub fn publish_player_local_map_knowledge(&self, local: &mut LocalMapKnowledge) {
        *local = self.player_local_map_knowledge();
    }
}

/// Applies current observations to both factions' independent knowledge.
///
/// Every previously Observed terrain snapshot first becomes Remembered. Currently
/// observed surfaces then replace it with authoritative current truth. Observed
/// positions absent from current truth are purged, while unseen deletions or edits
/// deliberately leave the old remembered snapshot untouched. Unit knowledge is
/// rebuilt from scratch and therefore never becomes remembered.
pub fn apply_observations(
    knowledge: &mut FactionMapKnowledge,
    current: &SurfaceSnapshots,
    observations: &FactionObservations,
) {
    for faction in [Faction::Player, Faction::Hostile] {
        knowledge
            .faction_mut(faction)
            .apply(current, observations.faction(faction));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::{Headroom, HexCoord, HexSpan, LightDomain, SubstanceId, TraversalEndpoint};

    fn pos(q: i32, level: i32) -> TilePos {
        TilePos::new(HexCoord::from_axial(q, 0), level)
    }

    fn surface(pos: TilePos, substance: u16, blocked: bool) -> SurfaceSnapshot {
        SurfaceSnapshot {
            pos,
            span: HexSpan::new(1.0, 2.0),
            substance: SubstanceId(substance),
            headroom: Headroom(2),
            is_solid: true,
            blocked,
            domain: LightDomain::Exterior,
        }
    }

    fn observations(
        faction: Faction,
        positions: impl IntoIterator<Item = TilePos>,
        units: impl IntoIterator<Item = ObservedUnit>,
    ) -> FactionObservations {
        let mut faction_observation = FactionObservation::new();
        for position in positions {
            faction_observation.insert_surface(position);
        }
        for unit in units {
            faction_observation
                .try_insert_unit(unit)
                .expect("unique unit fixtures");
        }
        FactionObservations::with_faction(faction, faction_observation)
    }

    #[test]
    fn unknown_observed_and_remembered_are_distinct() {
        let visible = pos(0, 5);
        let current =
            SurfaceSnapshots::try_from_iter([surface(visible, 1, false)]).expect("surface");
        let mut knowledge = FactionMapKnowledge::new();
        assert_eq!(
            knowledge.faction(Faction::Player).state(visible),
            KnowledgeState::Unknown
        );

        apply_observations(
            &mut knowledge,
            &current,
            &observations(Faction::Player, [visible], []),
        );
        assert_eq!(
            knowledge.faction(Faction::Player).state(visible),
            KnowledgeState::Observed
        );

        apply_observations(&mut knowledge, &current, &FactionObservations::default());
        assert_eq!(
            knowledge.faction(Faction::Player).state(visible),
            KnowledgeState::Remembered
        );
    }

    #[test]
    fn hidden_edits_and_blocker_changes_do_not_leak_then_reobservation_replaces() {
        let visible = pos(0, 5);
        let original =
            SurfaceSnapshots::try_from_iter([surface(visible, 2, false)]).expect("surface");
        let edited = SurfaceSnapshots::try_from_iter([surface(visible, 9, true)]).expect("surface");
        let mut knowledge = FactionMapKnowledge::new();

        apply_observations(
            &mut knowledge,
            &original,
            &observations(Faction::Player, [visible], []),
        );
        apply_observations(&mut knowledge, &edited, &FactionObservations::default());
        let remembered = knowledge
            .faction(Faction::Player)
            .surface(visible)
            .expect("remembered");
        assert_eq!(remembered.state(), KnowledgeState::Remembered);
        assert_eq!(remembered.snapshot().substance, SubstanceId(2));
        assert!(!remembered.snapshot().blocked);

        apply_observations(
            &mut knowledge,
            &edited,
            &observations(Faction::Player, [visible], []),
        );
        let observed = knowledge
            .faction(Faction::Player)
            .surface(visible)
            .expect("observed");
        assert_eq!(observed.state(), KnowledgeState::Observed);
        assert_eq!(observed.snapshot().substance, SubstanceId(9));
        assert!(observed.snapshot().blocked);
    }

    #[test]
    fn visible_deletion_purges_while_hidden_deletion_remains_remembered() {
        let hidden_deleted = pos(0, 5);
        let visible_deleted = pos(1, 5);
        let original = SurfaceSnapshots::try_from_iter([
            surface(hidden_deleted, 1, false),
            surface(visible_deleted, 1, false),
        ])
        .expect("surfaces");
        let empty = SurfaceSnapshots::default();
        let mut knowledge = FactionMapKnowledge::new();

        apply_observations(
            &mut knowledge,
            &original,
            &observations(Faction::Player, [hidden_deleted, visible_deleted], []),
        );
        apply_observations(
            &mut knowledge,
            &empty,
            &observations(Faction::Player, [visible_deleted], []),
        );

        assert_eq!(
            knowledge.faction(Faction::Player).state(hidden_deleted),
            KnowledgeState::Remembered
        );
        assert_eq!(
            knowledge.faction(Faction::Player).state(visible_deleted),
            KnowledgeState::Unknown
        );
    }

    #[test]
    fn unseen_units_disappear_instead_of_becoming_remembered() {
        let position = pos(0, 5);
        let current =
            SurfaceSnapshots::try_from_iter([surface(position, 1, false)]).expect("surface");
        let unit = ObservedUnit {
            id: UnitId(7),
            faction: Faction::Hostile,
            pos: position,
        };
        let mut knowledge = FactionMapKnowledge::new();

        apply_observations(
            &mut knowledge,
            &current,
            &observations(Faction::Player, [position], [unit]),
        );
        assert_eq!(
            knowledge.faction(Faction::Player).unit(UnitId(7)),
            Some(unit)
        );

        apply_observations(&mut knowledge, &current, &FactionObservations::default());
        assert_eq!(knowledge.faction(Faction::Player).unit(UnitId(7)), None);
        assert_eq!(
            knowledge.faction(Faction::Player).state(position),
            KnowledgeState::Remembered
        );
    }

    #[test]
    fn malformed_observation_cannot_publish_a_unit_without_its_surface() {
        let position = pos(0, 5);
        let current =
            SurfaceSnapshots::try_from_iter([surface(position, 1, false)]).expect("surface");
        let unit = ObservedUnit {
            id: UnitId(7),
            faction: Faction::Hostile,
            pos: position,
        };
        let mut observation = FactionObservation::new();
        observation
            .try_insert_unit(unit)
            .expect("unique unit fixture");
        let observations = FactionObservations::with_faction(Faction::Player, observation);
        let mut knowledge = FactionMapKnowledge::new();

        apply_observations(&mut knowledge, &current, &observations);
        assert_eq!(knowledge.faction(Faction::Player).unit(unit.id), None);
        assert_eq!(
            knowledge.faction(Faction::Player).state(position),
            KnowledgeState::Unknown
        );
    }

    #[test]
    fn hostile_knowledge_never_leaks_into_player_projection() {
        let player_pos = pos(0, 5);
        let hostile_pos = pos(2, 5);
        let current = SurfaceSnapshots::try_from_iter([
            surface(player_pos, 1, false),
            surface(hostile_pos, 1, true),
        ])
        .expect("surfaces");
        let mut knowledge = FactionMapKnowledge::new();

        let player = observations(Faction::Player, [player_pos], []);
        let hostile = observations(Faction::Hostile, [hostile_pos], []);
        let observations = FactionObservations::from_factions(
            player.faction(Faction::Player).clone(),
            hostile.faction(Faction::Hostile).clone(),
        );
        apply_observations(&mut knowledge, &current, &observations);

        let local = knowledge.player_local_map_knowledge();
        assert_eq!(local.state(player_pos), KnowledgeState::Observed);
        assert_eq!(local.state(hostile_pos), KnowledgeState::Unknown);
        assert_eq!(
            local.get(player_pos).map(|known| known.endpoint),
            Some(TraversalEndpoint::new(player_pos, true, Headroom(2)))
        );
        assert!(!local.get(player_pos).expect("known player surface").blocked);
    }

    #[test]
    fn publishing_replaces_stale_local_entries() {
        let old = pos(0, 5);
        let new = pos(1, 5);
        let current = SurfaceSnapshots::try_from_iter([surface(new, 1, false)]).expect("surface");
        let mut local = LocalMapKnowledge::new();
        local.set(
            KnowledgeState::Observed,
            TraversalEndpoint::new(old, true, Headroom(2)),
            false,
        );
        let mut knowledge = FactionMapKnowledge::new();
        apply_observations(
            &mut knowledge,
            &current,
            &observations(Faction::Player, [new], []),
        );

        knowledge.publish_player_local_map_knowledge(&mut local);
        assert_eq!(local.state(old), KnowledgeState::Unknown);
        assert_eq!(local.state(new), KnowledgeState::Observed);
    }
}
