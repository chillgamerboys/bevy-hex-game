//! Immutable Combat Lab fixture truth shared by simulation and presentation.
//!
//! Widgets may filter and render these definitions, but they do not own their names,
//! claims, roster sizes, maps, or supported profile matrix. The deterministic
//! simulation cases use the same stable identities for every overlapping claim.

/// One immutable Combat Lab experiment definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CombatLabFixtureDefinition {
    /// Stable machine identity used by walks, reports, and simulations.
    pub id: &'static str,
    /// Human-facing experiment name.
    pub name: &'static str,
    /// Searchable behavior tags.
    pub tags: &'static str,
    /// Behavioral claim the fixture must construct and verify.
    pub description: &'static str,
    /// Shipped scenario used by the interactive fixture.
    pub scenario: &'static str,
    /// Combat Lab map catalog identity used by Copy to Sandbox.
    pub sandbox_map: &'static str,
    /// Stable map and seed description.
    pub map_seed: &'static str,
    /// Exact roster description.
    pub roster: &'static str,
    /// Number of player-controlled combatants.
    pub player_count: u8,
    /// Number of hostile combatants.
    pub hostile_count: u8,
    /// Whether the fixture exposes Shipped, Tactical, and Custom runs.
    pub profile_matrix: bool,
    /// Whether a renderer-free simulation case directly owns the same claim.
    pub simulated: bool,
}

/// Immutable Combat Lab fixtures in stable selector order.
pub const COMBAT_LAB_FIXTURES: [CombatLabFixtureDefinition; 7] = [
    CombatLabFixtureDefinition {
        id: "ability-lab",
        name: "Ability Lab",
        tags: "aiming reveal restore revival",
        description: "A flat 2v1 for aiming, friendly damage, reveal, restoration, and revival.",
        scenario: "Ability Lab",
        sandbox_map: "flat-arena",
        map_seed: "Flat Arena · authored",
        roster: "2 Player · 1 Hostile",
        player_count: 2,
        hostile_count: 1,
        profile_matrix: false,
        simulated: true,
    },
    CombatLabFixtureDefinition {
        id: "raider-mirror",
        name: "Raider Mirror",
        tags: "identity defense enchantment",
        description: "Same archetype on both sides, with deterministic defensive enchantments.",
        scenario: "Raider Mirror",
        sandbox_map: "flat-arena",
        map_seed: "Flat Arena · authored",
        roster: "1 Player Raider · 1 Hostile Raider",
        player_count: 1,
        hostile_count: 1,
        profile_matrix: false,
        simulated: false,
    },
    CombatLabFixtureDefinition {
        id: "creator-spell-matrix",
        name: "Creator Spell Matrix",
        tags: "creator disable burn reveal restore defense",
        description: "Creator-format spell delivery against the flat deterministic roster.",
        scenario: "Ability Lab",
        sandbox_map: "flat-arena",
        map_seed: "Flat Arena · authored",
        roster: "Fixture Caster · Fixture Target",
        player_count: 1,
        hostile_count: 1,
        profile_matrix: false,
        simulated: true,
    },
    CombatLabFixtureDefinition {
        id: "creator-roster-matrix",
        name: "Creator Roster Matrix",
        tags: "creator roster selection ordering",
        description: "Mixed roster selection, stable unit ordering, and multi-unit combat.",
        scenario: "Ability Lab",
        sandbox_map: "flat-arena",
        map_seed: "Flat Arena · authored",
        roster: "2 Player · 2 Hostile creator records",
        player_count: 2,
        hostile_count: 2,
        profile_matrix: false,
        simulated: false,
    },
    CombatLabFixtureDefinition {
        id: "occupancy-matrix",
        name: "Occupancy Matrix",
        tags: "occupancy chokepoint endpoint route stacked interruption ai",
        description: "Party Trial on the authored Crossing for human/AI chokepoints, exact endpoints, route reservations, stacked bridge surfaces, and movement interruption.",
        scenario: "Party Trial",
        sandbox_map: "the-crossing",
        map_seed: "The Crossing · authored",
        roster: "3 Player · 3 Hostile",
        player_count: 3,
        hostile_count: 3,
        profile_matrix: false,
        simulated: true,
    },
    CombatLabFixtureDefinition {
        id: "channel-attrition",
        name: "Channel Attrition",
        tags: "channel mana disabled enchantment full repeated ai downed",
        description: "Ability Lab's deterministic lattices for depleted/full mana, disabled cells, enchantment locks, repeated Channel, AI selection, and downed refusal.",
        scenario: "Ability Lab",
        sandbox_map: "flat-arena",
        map_seed: "Flat Arena · authored",
        roster: "3 Player · 3 Hostile · preloaded lattice states",
        player_count: 3,
        hostile_count: 3,
        profile_matrix: false,
        simulated: true,
    },
    CombatLabFixtureDefinition {
        id: "tempo-matrix",
        name: "Tempo Matrix",
        tags: "tempo profile shipped tactical custom party",
        description: "The frozen 3v3 Party Trial baseline used repeatedly under Shipped, Tactical two-step, and bounded Custom profiles.",
        scenario: "Party Trial",
        sandbox_map: "the-crossing",
        map_seed: "The Crossing · authored",
        roster: "3 Player · 3 Hostile",
        player_count: 3,
        hostile_count: 3,
        profile_matrix: true,
        simulated: true,
    },
];

/// Finds immutable fixture truth by stable identity.
#[must_use]
pub fn combat_lab_fixture(id: &str) -> Option<&'static CombatLabFixtureDefinition> {
    COMBAT_LAB_FIXTURES.iter().find(|fixture| fixture.id == id)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn fixture_ids_are_unique_and_descriptions_are_behavioral() {
        let mut ids = BTreeSet::new();
        for fixture in COMBAT_LAB_FIXTURES {
            assert!(ids.insert(fixture.id));
            assert!(!fixture.description.trim().is_empty());
            assert!(fixture.player_count > 0);
            assert!(fixture.hostile_count > 0);
        }
    }
}
