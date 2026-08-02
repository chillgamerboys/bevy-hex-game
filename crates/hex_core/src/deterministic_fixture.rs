//! Immutable deterministic fixture truth shared by test and review tooling.
//!
//! Simulation cases and typed test launch requests use the same stable identities for
//! every overlapping claim. Shipping UI does not compile this module.

/// One exact roster entry in a deterministic fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeterministicRosterEntry {
    /// Stable shipped archetype key.
    Shipped(&'static str),
    /// Stable id in the test-only Creator library.
    Creator(u64),
}

/// Exact authored placement used by a deterministic fixture roster.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeterministicRosterPlacement {
    /// Place one unit at an exact axial coordinate.
    Fixed {
        /// Cube x coordinate.
        x: i32,
        /// Cube y coordinate.
        y: i32,
        /// Cube z coordinate.
        z: i32,
    },
    /// Place an ordered roster around an exact center.
    Formation {
        /// Cube x coordinate.
        x: i32,
        /// Cube y coordinate.
        y: i32,
        /// Cube z coordinate.
        z: i32,
        /// Maximum formation spread.
        spread: u32,
    },
}

/// Optional exact state applied after a deterministic fixture spawns its actors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeterministicFixtureInitialState {
    /// Depleted, locked, damaged, and downed lattice states for Channel coverage.
    ChannelAttrition,
}

/// One immutable deterministic experiment definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeterministicFixtureDefinition {
    /// Stable machine identity used by walks and simulations.
    pub id: &'static str,
    /// Human-facing experiment name.
    pub name: &'static str,
    /// Searchable behavior tags.
    pub tags: &'static str,
    /// Behavioral claim the fixture must construct and verify.
    pub description: &'static str,
    /// Shipped scenario used by the interactive fixture.
    pub scenario: &'static str,
    /// Sandbox map catalog identity used by typed test launch requests.
    pub sandbox_map: &'static str,
    /// Stable map and seed description.
    pub map_seed: &'static str,
    /// Exact roster description.
    pub roster: &'static str,
    /// Exact Party entries in stable launch order.
    pub party: &'static [DeterministicRosterEntry],
    /// Exact Enemy entries in stable launch order.
    pub enemies: &'static [DeterministicRosterEntry],
    /// Exact Party placement.
    pub party_placement: DeterministicRosterPlacement,
    /// Exact Enemy placement.
    pub enemy_placement: DeterministicRosterPlacement,
    /// Optional exact post-spawn state.
    pub initial_state: Option<DeterministicFixtureInitialState>,
    /// Whether the fixture exposes Shipped, Tactical, and Custom runs.
    pub profile_matrix: bool,
    /// Whether a renderer-free simulation case directly owns the same claim.
    pub simulated: bool,
}

/// Immutable deterministic fixtures in stable test order.
pub const DETERMINISTIC_FIXTURES: [DeterministicFixtureDefinition; 7] = [
    DeterministicFixtureDefinition {
        id: "ability-lab",
        name: "Ability Lab",
        tags: "aiming reveal restore revival",
        description: "A flat 2v1 for aiming, friendly damage, reveal, restoration, and revival.",
        scenario: "Ability Lab",
        sandbox_map: "flat-arena",
        map_seed: "Flat Arena · authored",
        roster: "2 Player · 1 Hostile",
        party: &[
            DeterministicRosterEntry::Shipped("hedge-mage"),
            DeterministicRosterEntry::Shipped("wolf"),
        ],
        enemies: &[DeterministicRosterEntry::Shipped("raider")],
        party_placement: DeterministicRosterPlacement::Formation {
            x: -2,
            y: 0,
            z: 2,
            spread: 1,
        },
        enemy_placement: DeterministicRosterPlacement::Fixed { x: 2, y: 0, z: -2 },
        initial_state: None,
        profile_matrix: false,
        simulated: true,
    },
    DeterministicFixtureDefinition {
        id: "raider-mirror",
        name: "Raider Mirror",
        tags: "identity defense enchantment",
        description: "Same archetype on both sides, with deterministic defensive enchantments.",
        scenario: "Raider Mirror",
        sandbox_map: "flat-arena",
        map_seed: "Flat Arena · authored",
        roster: "1 Player Raider · 1 Hostile Raider",
        party: &[DeterministicRosterEntry::Shipped("raider")],
        enemies: &[DeterministicRosterEntry::Shipped("raider")],
        party_placement: DeterministicRosterPlacement::Fixed { x: -1, y: 0, z: 1 },
        enemy_placement: DeterministicRosterPlacement::Fixed { x: 1, y: 0, z: -1 },
        initial_state: None,
        profile_matrix: false,
        simulated: false,
    },
    DeterministicFixtureDefinition {
        id: "creator-spell-matrix",
        name: "Creator Spell Matrix",
        tags: "creator disable burn reveal restore defense",
        description: "Creator-format spell delivery against the flat deterministic roster.",
        scenario: "Ability Lab",
        sandbox_map: "flat-arena",
        map_seed: "Flat Arena · authored",
        roster: "Fixture Caster · Fixture Target",
        party: &[DeterministicRosterEntry::Creator(1001)],
        enemies: &[DeterministicRosterEntry::Creator(1002)],
        party_placement: DeterministicRosterPlacement::Formation {
            x: -2,
            y: 0,
            z: 2,
            spread: 3,
        },
        enemy_placement: DeterministicRosterPlacement::Formation {
            x: 2,
            y: 0,
            z: -2,
            spread: 3,
        },
        initial_state: None,
        profile_matrix: false,
        simulated: true,
    },
    DeterministicFixtureDefinition {
        id: "creator-roster-matrix",
        name: "Creator Roster Matrix",
        tags: "creator roster selection ordering",
        description: "Mixed roster selection, stable unit ordering, and multi-unit combat.",
        scenario: "Ability Lab",
        sandbox_map: "flat-arena",
        map_seed: "Flat Arena · authored",
        roster: "2 Player · 2 Hostile creator records",
        party: &[
            DeterministicRosterEntry::Creator(1001),
            DeterministicRosterEntry::Creator(1003),
        ],
        enemies: &[
            DeterministicRosterEntry::Creator(1002),
            DeterministicRosterEntry::Creator(1001),
        ],
        party_placement: DeterministicRosterPlacement::Formation {
            x: -2,
            y: 0,
            z: 2,
            spread: 3,
        },
        enemy_placement: DeterministicRosterPlacement::Formation {
            x: 2,
            y: 0,
            z: -2,
            spread: 3,
        },
        initial_state: None,
        profile_matrix: false,
        simulated: false,
    },
    DeterministicFixtureDefinition {
        id: "occupancy-matrix",
        name: "Occupancy Matrix",
        tags: "occupancy chokepoint endpoint route stacked interruption ai",
        description: "Party Trial on the authored Crossing for human/AI chokepoints, exact endpoints, route reservations, stacked bridge surfaces, and movement interruption.",
        scenario: "Party Trial",
        sandbox_map: "the-crossing",
        map_seed: "The Crossing · authored",
        roster: "3 Player · 3 Hostile",
        party: &[
            DeterministicRosterEntry::Shipped("raider"),
            DeterministicRosterEntry::Shipped("wolf"),
            DeterministicRosterEntry::Shipped("raider"),
        ],
        enemies: &[
            DeterministicRosterEntry::Shipped("raider"),
            DeterministicRosterEntry::Shipped("wolf"),
            DeterministicRosterEntry::Shipped("raider"),
        ],
        party_placement: DeterministicRosterPlacement::Formation {
            x: 0,
            y: 2,
            z: -2,
            spread: 2,
        },
        enemy_placement: DeterministicRosterPlacement::Formation {
            x: 0,
            y: -2,
            z: 2,
            spread: 2,
        },
        initial_state: None,
        profile_matrix: false,
        simulated: true,
    },
    DeterministicFixtureDefinition {
        id: "channel-attrition",
        name: "Channel Attrition",
        tags: "channel mana disabled enchantment full repeated ai downed",
        description: "Ability Lab's deterministic lattices for depleted/full mana, disabled cells, enchantment locks, repeated Channel, AI selection, and downed refusal.",
        scenario: "Ability Lab",
        sandbox_map: "flat-arena",
        map_seed: "Flat Arena · authored",
        roster: "3 Player · 3 Hostile · preloaded lattice states",
        party: &[
            DeterministicRosterEntry::Shipped("hedge-mage"),
            DeterministicRosterEntry::Shipped("raider"),
            DeterministicRosterEntry::Shipped("wolf"),
        ],
        enemies: &[
            DeterministicRosterEntry::Shipped("hedge-mage"),
            DeterministicRosterEntry::Shipped("raider"),
            DeterministicRosterEntry::Shipped("wolf"),
        ],
        party_placement: DeterministicRosterPlacement::Formation {
            x: -2,
            y: 0,
            z: 2,
            spread: 2,
        },
        enemy_placement: DeterministicRosterPlacement::Formation {
            x: 2,
            y: 0,
            z: -2,
            spread: 2,
        },
        initial_state: Some(DeterministicFixtureInitialState::ChannelAttrition),
        profile_matrix: false,
        simulated: true,
    },
    DeterministicFixtureDefinition {
        id: "tempo-matrix",
        name: "Tempo Matrix",
        tags: "tempo profile shipped tactical custom party",
        description: "The frozen 3v3 Party Trial baseline used repeatedly under Shipped, Tactical two-step, and bounded Custom profiles.",
        scenario: "Party Trial",
        sandbox_map: "the-crossing",
        map_seed: "The Crossing · authored",
        roster: "3 Player · 3 Hostile",
        party: &[
            DeterministicRosterEntry::Shipped("raider"),
            DeterministicRosterEntry::Shipped("wolf"),
            DeterministicRosterEntry::Shipped("raider"),
        ],
        enemies: &[
            DeterministicRosterEntry::Shipped("raider"),
            DeterministicRosterEntry::Shipped("wolf"),
            DeterministicRosterEntry::Shipped("raider"),
        ],
        party_placement: DeterministicRosterPlacement::Formation {
            x: 0,
            y: 2,
            z: -2,
            spread: 2,
        },
        enemy_placement: DeterministicRosterPlacement::Formation {
            x: 0,
            y: -2,
            z: 2,
            spread: 2,
        },
        initial_state: None,
        profile_matrix: true,
        simulated: true,
    },
];

/// Finds immutable deterministic fixture truth by stable identity.
#[must_use]
pub fn deterministic_fixture(id: &str) -> Option<&'static DeterministicFixtureDefinition> {
    DETERMINISTIC_FIXTURES
        .iter()
        .find(|fixture| fixture.id == id)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn fixture_ids_are_unique_and_descriptions_are_behavioral() {
        let mut ids = BTreeSet::new();
        for fixture in DETERMINISTIC_FIXTURES {
            assert!(ids.insert(fixture.id));
            assert!(!fixture.description.trim().is_empty());
            assert!(!fixture.party.is_empty());
            assert!(!fixture.enemies.is_empty());
        }
    }
}
