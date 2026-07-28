//! What stands on the map when a scenario starts, and where.
//!
//! An **encounter** is a roster: one entry per unit, each naming an archetype and
//! resolving to exactly one surface. It replaces the two-coordinate scaffold that could
//! only ever say "one player here, one enemy there".
//!
//! # Why a file per encounter, named by path
//!
//! A scenario names its encounter the same way it already names its world and its
//! lighting: by asset path, turned into a handle by `hex_game`. Nothing here loads a
//! *directory* of encounters, because a scenario needs exactly one of them — so the
//! one-path-one-type settings loader carries this unchanged, and an encounter hot-reloads
//! like every other settings file.
//!
//! # Placement is a surface, never a coordinate
//!
//! Two surfaces stacked in one column are separate places, so an encounter cannot say
//! where a unit stands by naming a coordinate alone. [`EncounterPlacement::Anchor`]
//! names one exact surface the generator published; [`EncounterPlacement::Fixed`] names
//! an authored coordinate and takes the lowest surface there that the body fits on —
//! the ground, rather than a bridge built over it. Resolution needs the live map and so
//! belongs to `hex_units`; this crate only carries what the designer wrote.
//!
//! # What an encounter deliberately does not carry
//!
//! - **No rewards, loot, or victory conditions.** Nothing yet knows what a fight should
//!   yield, and a field invented now would be wrong in a way content would then depend
//!   on.
//! - **No lattices.** An entry names its archetype as a string and nothing resolves it.
//!   That is the seam: when `lattices.ron` lands, one archetype-to-lattice lookup goes
//!   inside the spawn loop rather than into per-unit spawn code.
//! - **No triggers, quests, or dialogue.** Those are fields added to this schema later,
//!   not gaps in it today.

use bevy::prelude::*;
use serde::{de::Error as _, Deserialize, Deserializer};

use crate::settings::CubeCoord;

/// `assets/config/encounters/*.ron` — one encounter: who is on the map, and where.
///
/// Chosen per scenario, so it is a [`Resource`] as well as an [`Asset`]: the active
/// encounter is what `hex_units` spawns from.
#[derive(Asset, Resource, Reflect, Debug, Clone, PartialEq, Eq)]
#[reflect(Resource)]
pub struct Encounter {
    /// What this encounter is called, for logs and setup failures.
    pub name: String,
    /// The sides, in the order their units spawn.
    ///
    /// A list rather than one entry per faction, so two hostile groups holding
    /// different ground are expressible without a second mechanism.
    pub rosters: Vec<Roster>,
}

/// One side of an encounter: its units, and where they come in.
#[derive(Reflect, Debug, Clone, PartialEq, Eq)]
pub struct Roster {
    /// Which side these units fight for.
    pub faction: EncounterFaction,
    /// Where the units come in, unless an entry names its own placement.
    pub placement: EncounterPlacement,
    /// The units, in the order they spawn.
    pub units: Vec<RosterEntry>,
}

/// One unit in a roster.
#[derive(Reflect, Debug, Clone, PartialEq, Eq)]
pub struct RosterEntry {
    /// What kind of unit this is, by name.
    ///
    /// Resolved to nothing today. It is the key an archetype's lattice will be
    /// looked up by, which is why it is named now rather than when lattices land.
    pub archetype: String,
    /// This unit's own placement, overriding its roster's.
    ///
    /// For the unit that has to be somewhere exact — the sentry on the bridge — while
    /// the rest of its side comes in as a formation.
    pub placement: Option<EncounterPlacement>,
}

/// Which side a rostered unit is on.
///
/// Mirrors `hex_units::Faction` rather than using it: `hex_units` depends on this
/// crate, so naming its types here would invert the crate graph. The conversion lives
/// with the spawn path.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
pub enum EncounterFaction {
    /// The party the player controls.
    Player,
    /// Everything that wants the party dead.
    Hostile,
}

impl EncounterFaction {
    /// The name to use in a designer-facing message.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Player => "player",
            Self::Hostile => "hostile",
        }
    }
}

/// Where a rostered unit starts.
///
/// [`Self::Fixed`] and [`Self::Anchor`] each place **one** unit on one surface;
/// [`Self::Formation`] is the form that places a group.
// `Formation` is the only struct variant in the schema, so it is the only place a
// misspelling can land in a field name rather than a variant name. Without this, a
// stray `radius: 4` beside `spread` parses and does nothing at all.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum EncounterPlacement {
    /// An exact coordinate on an authored map, whose landmarks never move.
    ///
    /// The three components must sum to zero — see [`CubeCoord`].
    Fixed(CubeCoord),
    /// One exact surface, resolved from `hex_core::MapAnchors` by name.
    ///
    /// What generated maps use: rerolling a seed moves the useful ground, and the
    /// generator republishes the anchor rather than the designer rewriting coordinates.
    Anchor(String),
    /// A centre, and how far from it this side may spread.
    ///
    /// **The formation anchor.** A party of four placed at one anchor would otherwise
    /// have to stack on one voxel, which is not a position the rest of the game can
    /// express. Instead each unit takes the next free surface *walkable* from the
    /// centre, closest first — so a formation never crosses a chasm to find room, never
    /// puts two units in one place, and only uses surfaces the body actually fits on.
    ///
    /// A named spawn zone is this: the anchor names it, `spread` bounds it, and the
    /// fill order is deterministic (walking distance, then position), so the same
    /// encounter on the same map always deals the same surfaces.
    Formation {
        /// The surface the formation gathers around. Its first unit stands here.
        center: FormationCenter,
        /// How many walking steps from the centre a unit may be placed.
        ///
        /// Steps, not hexes: a surface two steps away is two moves away, which is
        /// what "next to the party" means on terrain with cliffs and bridges in it.
        spread: u32,
    },
}

impl EncounterPlacement {
    /// Whether this placement resolves through a generated map anchor.
    ///
    /// The authored/generated contract is checked per scenario against its world, and
    /// a formation is generated exactly when its centre is.
    #[must_use]
    pub fn is_generated(&self) -> bool {
        match self {
            Self::Fixed(_) => false,
            Self::Anchor(_) => true,
            Self::Formation { center, .. } => matches!(center, FormationCenter::Anchor(_)),
        }
    }

    /// Whether this placement occupies exactly one named surface.
    ///
    /// Two units sharing one of these is a designer error rather than a crowd, and it
    /// is rejected when the file parses.
    #[must_use]
    pub fn is_exact(&self) -> bool {
        matches!(self, Self::Fixed(_) | Self::Anchor(_))
    }

    /// The anchor name this placement needs, if it needs one.
    #[must_use]
    pub fn anchor(&self) -> Option<&str> {
        match self {
            Self::Fixed(_) => None,
            Self::Anchor(name) => Some(name.as_str()),
            Self::Formation { center, .. } => match center {
                FormationCenter::Fixed(_) => None,
                FormationCenter::Anchor(name) => Some(name.as_str()),
            },
        }
    }

    /// The authored coordinate this placement is built on, if it has one.
    #[must_use]
    pub fn fixed_coord(&self) -> Option<CubeCoord> {
        match self {
            Self::Fixed(coord) => Some(*coord),
            Self::Anchor(_) => None,
            Self::Formation { center, .. } => match center {
                FormationCenter::Fixed(coord) => Some(*coord),
                FormationCenter::Anchor(_) => None,
            },
        }
    }
}

/// The surface a formation gathers around.
///
/// The same two forms as a single placement, deliberately: an authored map centres a
/// formation on a coordinate, a generated one on a published anchor, and neither should
/// need a different vocabulary because a group rather than one unit is arriving.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum FormationCenter {
    /// An exact coordinate on an authored map.
    Fixed(CubeCoord),
    /// A surface published by the generator.
    Anchor(String),
}

impl Encounter {
    /// Every rostered unit, in spawn order, with the placement it resolves through.
    ///
    /// One iterator rather than nested loops in every consumer: spawn order *is* the
    /// declaration order, and that is what makes the unit ids a function of the
    /// encounter rather than of the run.
    pub fn entries(&self) -> impl Iterator<Item = RosteredUnit<'_>> + '_ {
        self.rosters.iter().flat_map(|roster| {
            roster.units.iter().map(move |entry| RosteredUnit {
                faction: roster.faction,
                archetype: entry.archetype.as_str(),
                placement: entry.placement.as_ref().unwrap_or(&roster.placement),
            })
        })
    }

    /// How many units this encounter rosters for one side.
    #[must_use]
    pub fn unit_count(&self, faction: EncounterFaction) -> usize {
        self.rosters
            .iter()
            .filter(|roster| roster.faction == faction)
            .map(|roster| roster.units.len())
            .sum()
    }

    /// The sides this encounter rosters units for, in declaration order.
    pub fn factions(&self) -> impl Iterator<Item = EncounterFaction> + '_ {
        let mut seen = Vec::new();
        self.rosters.iter().filter_map(move |roster| {
            if seen.contains(&roster.faction) {
                return None;
            }
            seen.push(roster.faction);
            Some(roster.faction)
        })
    }

    /// Checks what a single encounter file can be wrong about on its own.
    ///
    /// Deserialization runs this, so a malformed encounter stops on the loading screen
    /// with a reason rather than reaching the spawn loop. What it cannot see is the map:
    /// whether an anchor exists, and whether anything can be stood on, are answered
    /// against live terrain in `hex_units`.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("an encounter needs a name".to_owned());
        }
        if self.rosters.is_empty() {
            return Err(format!(
                "encounter {:?} has no rosters, so it would start an empty map",
                self.name
            ));
        }

        // Across the whole encounter, not per roster: two sides sent to one anchor is the
        // same mistake as two units of one side, and the file is the earliest place that
        // can see either.
        let mut exact: Vec<(&EncounterPlacement, &'static str)> = Vec::new();
        for roster in &self.rosters {
            let side = roster.faction.label();
            if roster.units.is_empty() {
                return Err(format!(
                    "encounter {:?}: the {side} roster has no units",
                    self.name
                ));
            }
            roster.placement.validate().map_err(|reason| {
                format!("encounter {:?}: the {side} roster {reason}", self.name)
            })?;

            for entry in &roster.units {
                if entry.archetype.trim().is_empty() {
                    return Err(format!(
                        "encounter {:?}: a {side} unit has no archetype",
                        self.name
                    ));
                }
                let placement = entry.placement.as_ref().unwrap_or(&roster.placement);
                placement.validate().map_err(|reason| {
                    format!(
                        "encounter {:?}: {side} unit {:?} {reason}",
                        self.name, entry.archetype
                    )
                })?;

                // An exact placement holds one unit. Two units sharing one is not a
                // crowd to be resolved at spawn time, it is a file to be fixed — and
                // the fix is a formation, so the message says so.
                if placement.is_exact() {
                    if let Some((_, other)) = exact.iter().find(|(taken, _)| *taken == placement) {
                        return Err(format!(
                            "encounter {:?}: the {side} unit {:?} and a {other} unit share the \
                             placement {placement:?}, which holds exactly one; use \
                             Formation(center: …, spread: …) for a group",
                            self.name, entry.archetype
                        ));
                    }
                    exact.push((placement, side));
                }
            }
        }

        // Last, so a malformed roster is reported as what it is rather than as this.
        //
        // The scaffold this replaced asserted exactly one player, which was a claim
        // about a *count*, and retiring it took the *existence* guarantee with it. Only
        // the count deserved to go: an encounter of hostiles alone passes every check
        // above, enters gameplay, and leaves nothing to select, nothing to command and
        // no camera target — a screen that renders perfectly and cannot be played, with
        // no log line anywhere.
        if !self
            .rosters
            .iter()
            .any(|roster| roster.faction == EncounterFaction::Player)
        {
            return Err(format!(
                "encounter {:?} rosters no player side, so it would start a map with \
                 nothing to command",
                self.name
            ));
        }
        Ok(())
    }
}

impl EncounterPlacement {
    /// Checks the placement alone, with no map to resolve it against.
    fn validate(&self) -> Result<(), String> {
        if let Some(coord) = self.fixed_coord() {
            if coord.x + coord.y + coord.z != 0 {
                return Err(format!(
                    "is placed at ({}, {}, {}), whose components do not sum to zero and so are \
                     not a hex",
                    coord.x, coord.y, coord.z
                ));
            }
        }
        if self.anchor().is_some_and(|name| name.trim().is_empty()) {
            return Err("names an empty map anchor".to_owned());
        }
        Ok(())
    }
}

/// One unit as the spawn loop sees it: a side, an archetype, and one placement.
///
/// Borrowed from the encounter rather than cloned out of it, because the spawn loop
/// reads it once and the strings are the designer's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RosteredUnit<'a> {
    /// Which side this unit is on.
    pub faction: EncounterFaction,
    /// What kind of unit it is. Resolved to nothing yet.
    pub archetype: &'a str,
    /// Where it starts: its own placement, or its roster's.
    pub placement: &'a EncounterPlacement,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnvalidatedEncounter {
    name: String,
    rosters: Vec<UnvalidatedRoster>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnvalidatedRoster {
    faction: EncounterFaction,
    placement: EncounterPlacement,
    units: Vec<UnvalidatedEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnvalidatedEntry {
    archetype: String,
    #[serde(default)]
    placement: Option<EncounterPlacement>,
}

impl<'de> Deserialize<'de> for Encounter {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = UnvalidatedEncounter::deserialize(deserializer)?;
        let encounter = Self {
            name: raw.name,
            rosters: raw
                .rosters
                .into_iter()
                .map(|roster| Roster {
                    faction: roster.faction,
                    placement: roster.placement,
                    units: roster
                        .units
                        .into_iter()
                        .map(|entry| RosterEntry {
                            archetype: entry.archetype,
                            placement: entry.placement,
                        })
                        .collect(),
                })
                .collect(),
        };
        encounter.validate().map_err(D::Error::custom)?;
        Ok(encounter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(ron: &str) -> Result<Encounter, ron::error::SpannedError> {
        ron::from_str(ron)
    }

    /// A roster's placement, without indexing — which stays denied in tests.
    fn roster_placement(encounter: &Encounter, index: usize) -> EncounterPlacement {
        encounter
            .rosters
            .get(index)
            .expect("the encounter should have that roster")
            .placement
            .clone()
    }

    fn anchored(name: &str) -> String {
        format!(
            r#"(
                name: "Anchored",
                rosters: [
                    (
                        faction: Player,
                        placement: Anchor("{name}"),
                        units: [(archetype: "hedge-mage")],
                    ),
                ],
            )"#
        )
    }

    #[test]
    fn a_roster_carries_many_units_per_side_in_declaration_order() {
        let encounter = parse(
            r#"(
                name: "Warband",
                rosters: [
                    (
                        faction: Player,
                        placement: Formation(center: Anchor("party_start"), spread: 2),
                        units: [
                            (archetype: "hedge-mage"),
                            (archetype: "raider", placement: Some(Anchor("bridge"))),
                        ],
                    ),
                    (
                        faction: Hostile,
                        placement: Formation(center: Anchor("hostile_start"), spread: 2),
                        units: [(archetype: "wolf"), (archetype: "wolf")],
                    ),
                ],
            )"#,
        )
        .expect("the roster should parse");

        assert_eq!(encounter.unit_count(EncounterFaction::Player), 2);
        assert_eq!(encounter.unit_count(EncounterFaction::Hostile), 2);
        assert_eq!(
            encounter.factions().collect::<Vec<_>>(),
            vec![EncounterFaction::Player, EncounterFaction::Hostile]
        );

        let entries: Vec<_> = encounter.entries().collect();
        let entry = |index: usize| {
            *entries
                .get(index)
                .expect("the roster should deal four entries")
        };
        assert_eq!(entries.len(), 4);
        assert_eq!(entry(0).archetype, "hedge-mage");
        // The unit with no placement of its own inherits its roster's formation.
        assert_eq!(entry(0).placement, &roster_placement(&encounter, 0));
        // The one that overrides keeps its own exact surface.
        assert_eq!(
            entry(1).placement,
            &EncounterPlacement::Anchor("bridge".to_owned())
        );
        assert_eq!(entry(3).faction, EncounterFaction::Hostile);
    }

    #[test]
    fn both_scaffold_placement_forms_still_parse() {
        let fixed = parse(
            r#"(
                name: "Authored",
                rosters: [
                    (
                        faction: Player,
                        placement: Fixed((x: 0, y: 4, z: -4)),
                        units: [(archetype: "hedge-mage")],
                    ),
                ],
            )"#,
        )
        .expect("a fixed placement should parse");
        assert!(!roster_placement(&fixed, 0).is_generated());
        assert!(roster_placement(&fixed, 0).is_exact());

        let anchor = parse(&anchored("party_start")).expect("an anchor placement should parse");
        let placement = roster_placement(&anchor, 0);
        assert!(placement.is_generated());
        assert_eq!(
            placement.anchor(),
            Some("party_start"),
            "the anchor name should survive the round trip"
        );
    }

    #[test]
    fn a_formation_is_generated_exactly_when_its_centre_is() {
        let generated = EncounterPlacement::Formation {
            center: FormationCenter::Anchor("party_start".to_owned()),
            spread: 2,
        };
        let authored = EncounterPlacement::Formation {
            center: FormationCenter::Fixed(CubeCoord { x: 0, y: 4, z: -4 }),
            spread: 2,
        };

        assert!(generated.is_generated());
        assert!(!authored.is_generated());
        // Neither is exact: a formation is the form that holds more than one unit.
        assert!(!generated.is_exact());
        assert!(!authored.is_exact());
    }

    /// A coordinate whose components do not sum to zero is not a hex. It used to warn
    /// and put the unit at the centre of the map; now the file does not parse, which is
    /// the earliest layer that can see it at all.
    #[test]
    fn an_impossible_coordinate_is_rejected_when_the_file_parses() {
        let error = parse(
            r#"(
                name: "Broken",
                rosters: [
                    (
                        faction: Player,
                        placement: Fixed((x: 1, y: 1, z: 1)),
                        units: [(archetype: "hedge-mage")],
                    ),
                ],
            )"#,
        )
        .expect_err("an impossible coordinate should be rejected")
        .to_string();
        assert!(error.contains("sum to zero"), "unexpected error: {error}");
    }

    #[test]
    fn two_units_may_not_share_one_exact_placement() {
        let error = parse(
            r#"(
                name: "Stacked",
                rosters: [
                    (
                        faction: Player,
                        placement: Anchor("party_start"),
                        units: [(archetype: "hedge-mage"), (archetype: "raider")],
                    ),
                ],
            )"#,
        )
        .expect_err("two units on one anchor should be rejected")
        .to_string();
        assert!(error.contains("Formation"), "unexpected error: {error}");
    }

    /// And two *sides* sent to one anchor is the same mistake, which a per-roster check
    /// would have let through to the spawn loop.
    #[test]
    fn two_sides_may_not_share_one_exact_placement() {
        let error = parse(
            r#"(
                name: "Overlap",
                rosters: [
                    (
                        faction: Player,
                        placement: Anchor("bridge"),
                        units: [(archetype: "hedge-mage")],
                    ),
                    (
                        faction: Hostile,
                        placement: Anchor("bridge"),
                        units: [(archetype: "raider")],
                    ),
                ],
            )"#,
        )
        .expect_err("both sides on one anchor should be rejected")
        .to_string();
        assert!(
            error.contains("player") && error.contains("hostile"),
            "the error should name both sides: {error}"
        );
    }

    #[test]
    fn an_encounter_needs_a_name_a_roster_and_units() {
        let empty_rosters = parse(r#"(name: "Empty", rosters: [])"#)
            .expect_err("an encounter with no rosters should be rejected")
            .to_string();
        assert!(empty_rosters.contains("no rosters"));

        let empty_units = parse(
            r#"(
                name: "Silent",
                rosters: [(faction: Hostile, placement: Anchor("hostile_start"), units: [])],
            )"#,
        )
        .expect_err("a roster with no units should be rejected")
        .to_string();
        assert!(empty_units.contains("no units"));

        let unnamed = parse(&anchored("party_start").replace(r#""Anchored""#, r#""  ""#))
            .expect_err("an unnamed encounter should be rejected")
            .to_string();
        assert!(unnamed.contains("needs a name"));
    }

    #[test]
    fn an_empty_anchor_name_is_rejected() {
        let error = parse(&anchored("  "))
            .expect_err("an empty anchor should be rejected")
            .to_string();
        assert!(error.contains("empty map anchor"), "unexpected: {error}");
    }

    #[test]
    fn an_unknown_field_is_rejected_rather_than_ignored() {
        let error = parse(
            r#"(
                name: "Rewards",
                rosters: [
                    (
                        faction: Player,
                        placement: Anchor("party_start"),
                        units: [(archetype: "hedge-mage")],
                    ),
                ],
                loot: "a sword",
            )"#,
        )
        .expect_err("an unknown field should be rejected")
        .to_string();
        assert!(error.contains("loot"), "unexpected error: {error}");
    }

    /// `Formation` is the schema's only struct variant, so it is the only place a
    /// misspelling lands in a field name. The top-level check above does not reach it.
    #[test]
    fn an_unknown_field_inside_a_formation_is_rejected_too() {
        let error = parse(
            r#"(
                name: "Sprawl",
                rosters: [
                    (
                        faction: Player,
                        placement: Formation(center: Anchor("party_start"), spread: 2, radius: 4),
                        units: [(archetype: "hedge-mage")],
                    ),
                ],
            )"#,
        )
        .expect_err("a stray formation field should be rejected")
        .to_string();
        assert!(error.contains("radius"), "unexpected error: {error}");
    }

    /// Retiring "exactly one player" retired the count *and* the existence guarantee.
    /// Only the count deserved to go: hostiles alone is a map with nothing to command.
    #[test]
    fn an_encounter_with_no_player_side_is_rejected() {
        let error = parse(
            r#"(
                name: "Ambush",
                rosters: [
                    (
                        faction: Hostile,
                        placement: Anchor("enemy_start"),
                        units: [(archetype: "raider")],
                    ),
                ],
            )"#,
        )
        .expect_err("an encounter with no player side should be rejected")
        .to_string();
        assert!(
            error.contains("nothing to command"),
            "unexpected error: {error}"
        );
    }

    /// Two hostile groups holding different ground need no second mechanism: a
    /// faction may appear in more than one roster.
    #[test]
    fn a_faction_may_hold_two_positions() {
        let encounter = parse(
            r#"(
                name: "Pincer",
                rosters: [
                    (
                        faction: Hostile,
                        placement: Formation(center: Anchor("bridge"), spread: 1),
                        units: [(archetype: "raider")],
                    ),
                    (
                        faction: Hostile,
                        placement: Formation(center: Anchor("alternate_crossing"), spread: 1),
                        units: [(archetype: "raider")],
                    ),
                    (
                        faction: Player,
                        placement: Formation(center: Anchor("party_start"), spread: 1),
                        units: [(archetype: "hedge-mage")],
                    ),
                ],
            )"#,
        )
        .expect("two rosters for one faction should parse");

        assert_eq!(encounter.unit_count(EncounterFaction::Hostile), 2);
        assert_eq!(
            encounter.factions().collect::<Vec<_>>(),
            vec![EncounterFaction::Hostile, EncounterFaction::Player],
            "a faction rostered twice is still one side"
        );
    }
}
