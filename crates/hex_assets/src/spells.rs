//! Spells, loaded from `assets/config/spells.ron`.
//!
//! A spell is defined by its **requirements** (an element multiset drawn from adjacent
//! gems, whose length is the tier — at most six, a full ring), its **casting axis**
//! (evocation vs. enchantment), its **mana axis** (fixed vs. variable), whether it is
//! **co-castable**, its **targeting**, and a list of **effects** from a closed enum.
//!
//! # "Ritual" is derived, not stored
//!
//! The design names two independent axes and observes that "ritual" is the corner
//! where both hold: variable mana *and* co-castable. Rather than store a third flag
//! that can disagree with the two it summarises, [`Spell::is_ritual`] derives it.
//!
//! # Effects are a closed enum, never a script
//!
//! [`Effect`] is a fixed vocabulary of primitives (audit §8). A closed enum can be
//! bounds-checked at parse and makes runtime failure unrepresentable — the whole
//! reason there is no scripting engine. Extension is one variant plus one match arm.
//! These effects are *applied* downstream (hex_combat, when casting lands); this crate
//! only parses and validates them.
//!
//! Element and substance references are by **name**; resolving them against the
//! element and substance tables is [`ContentIndex`](crate::ContentIndex)'s job.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use hex_core::{Screen, SpellId};
use serde::Deserialize;

use crate::{LoadSettings, CONFIG_EXTENSIONS};

/// One gem a spell requires: a distinct adjacent source of `element` contributing
/// `mana`. Mirrors `hex_lattice::Requirement` so the future `SpellTable` mapping is
/// direct.
#[derive(Reflect, Debug, Clone, Deserialize)]
pub struct GemRequirement {
    /// The element the adjacent gem (or live fusion output) must provide, by name.
    pub element: String,
    /// How much mana that gem contributes to the cast.
    pub mana: u16,
}

/// How a spell spends the mana it draws. Mirrors `hex_lattice::Casting`.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum CastingAxis {
    /// Drains and consumes the mana; its cost is throughput, recovered by channelling.
    Evocation,
    /// Ties the drawn mana up for as long as the enchantment lasts, lost if it breaks.
    Enchantment {
        /// Flat reduction applied to incoming disable counts while active. Zero for a
        /// non-defensive enchantment.
        defense: u16,
    },
}

/// Whether a spell draws a fixed amount of mana or a variable amount for a varied
/// effect.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum ManaAxis {
    /// A binary spell: it fires at full strength or not at all.
    Fixed,
    /// A variable spell: it scales with the mana it is given.
    Variable,
}

/// The shape a spell's targeting covers. Pure data; `hex_units::targeting` resolves the
/// geometry at cast time (a later ticket).
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum TargetShape {
    /// Cast on the caster itself; `range` is 0.
    SelfCast,
    /// A single target surface.
    Single,
    /// A line out from the caster.
    Line,
    /// An area around the target.
    Blast,
}

/// Where a spell can be cast, reusing `hex_units::targeting`'s height-advantage
/// geometry at cast time. Pure data here.
#[derive(Reflect, Debug, Clone, Copy, Deserialize)]
pub struct TargetingSpec {
    /// Base range in hexes, before any high-ground bonus.
    pub range: u8,
    /// The shape the spell covers.
    pub shape: TargetShape,
    /// Whether an unobstructed line of sight to the target is required.
    pub needs_los: bool,
}

/// One primitive effect a spell applies when it resolves. A closed vocabulary
/// (audit §8) — extension is one variant here plus one match arm where effects are
/// applied.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Deserialize)]
pub enum Effect {
    /// Disable a number of the target's hexes. `targeted` chooses specific hexes
    /// rather than a flat count.
    DisableHexes {
        /// How many hexes to disable.
        count: u8,
        /// Whether the caster picks which hexes, rather than an arbitrary count.
        targeted: bool,
    },
    /// Burn locked mana out of the target.
    Burn {
        /// How much mana to burn.
        amount: u16,
    },
    /// Restore a number of the target's disabled hexes.
    RestoreHexes {
        /// How many hexes to restore.
        count: u8,
    },
    /// Reduce incoming disable counts by a flat amount (a one-shot ward, distinct from
    /// an enchantment's persistent [`CastingAxis::Enchantment`] defense).
    ModifyIncomingDisables {
        /// The flat reduction.
        amount: u16,
    },
    /// Reveal part of the target's hidden lattice; how much scales with `tier`.
    Reveal {
        /// The divination tier — how much of the lattice becomes visible.
        tier: u8,
    },
    /// Light an area, lifting fog around the caster.
    Illuminate {
        /// Radius lit, in hexes.
        radius: u8,
    },
    /// Replace terrain at the target with a named substance.
    SetTerrain {
        /// The substance to place, by name (resolved against the substance table).
        substance: String,
    },
    /// Clear the terrain voxel at the target (turning it to air).
    ClearTerrain,
    /// Conjure a wall of a named substance.
    SpawnWall {
        /// The substance the wall is made of, by name.
        substance: String,
    },
    /// Push the target a number of hexes away.
    Displace {
        /// How many hexes to push.
        distance: u8,
    },
}

impl Effect {
    /// The substance name this effect references, if any — the cross-file reference
    /// [`ContentIndex`](crate::ContentIndex) must resolve.
    #[must_use]
    pub fn substance(&self) -> Option<&str> {
        match self {
            Self::SetTerrain { substance } | Self::SpawnWall { substance } => Some(substance),
            _ => None,
        }
    }
}

/// A single spell definition, before element/substance names are resolved.
#[derive(Reflect, Debug, Clone, Deserialize)]
pub struct Spell {
    /// The adjacent gems this spell draws on; its length is the tier (≤ 6).
    pub requirements: Vec<GemRequirement>,
    /// How the spell spends its mana.
    pub casting: CastingAxis,
    /// Whether the spell draws fixed or variable mana.
    pub mana: ManaAxis,
    /// Whether this spell can be cast alongside another in the same action.
    pub co_castable: bool,
    /// Where the spell can be cast.
    pub targeting: TargetingSpec,
    /// What the spell does when it resolves.
    pub effects: Vec<Effect>,
}

impl Spell {
    /// The spell's tier: how many adjacent gems it requires.
    ///
    /// Saturates at [`u8::MAX`], but [`SpellFile::validate`] caps tier at six, so a
    /// spell that parsed is always in range.
    #[must_use]
    pub fn tier(&self) -> u8 {
        u8::try_from(self.requirements.len()).unwrap_or(u8::MAX)
    }

    /// Whether this spell is a **ritual** — the design's name for the corner where a
    /// spell is both variable-mana and co-castable. Derived, never stored.
    #[must_use]
    pub fn is_ritual(&self) -> bool {
        matches!(self.mana, ManaAxis::Variable) && self.co_castable
    }
}

/// The largest tier a spell can have: a full ring of six adjacent gems.
const MAX_TIER: usize = 6;

/// The raw file, before names are turned into ids.
///
/// `Deserialize` is hand-written (via `UnvalidatedSpellFile`) so tier bounds, mana
/// amounts and effect fields are checked at parse: an invalid `spells.ron` fails to
/// load and the previous valid [`SpellBook`] stays active.
#[derive(Asset, Resource, Reflect, Debug, Clone)]
#[reflect(Resource)]
pub struct SpellFile {
    /// Spells by name.
    pub spells: HashMap<String, Spell>,
}

/// The same shape as [`SpellFile`] with a derived `Deserialize` and no validation.
#[derive(Deserialize)]
struct UnvalidatedSpellFile {
    spells: HashMap<String, Spell>,
}

impl<'de> Deserialize<'de> for SpellFile {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = UnvalidatedSpellFile::deserialize(deserializer)?;
        let file = Self { spells: raw.spells };
        file.validate().map_err(serde::de::Error::custom)?;
        Ok(file)
    }
}

impl SpellFile {
    /// Checks every intra-file invariant. Cross-file references (an element in
    /// `requirements`, a substance in an effect) are [`ContentIndex`](crate::ContentIndex)'s
    /// job — a single file cannot see the others.
    pub fn validate(&self) -> Result<(), String> {
        for (name, spell) in &self.spells {
            let tier = spell.requirements.len();
            if tier == 0 {
                return Err(format!(
                    "spell '{name}' requires no gems; tier must be at least 1"
                ));
            }
            if tier > MAX_TIER {
                return Err(format!(
                    "spell '{name}' has tier {tier}; the maximum is {MAX_TIER} (a full ring)"
                ));
            }
            for requirement in &spell.requirements {
                if requirement.mana == 0 {
                    return Err(format!(
                        "spell '{name}' draws 0 mana from a '{}' gem",
                        requirement.element
                    ));
                }
            }
            validate_effects(name, spell)?;
        }
        Ok(())
    }
}

/// Checks a spell's effects have sane fields and that the spell does *something*.
fn validate_effects(name: &str, spell: &Spell) -> Result<(), String> {
    for effect in &spell.effects {
        let zero = |field: &str| format!("spell '{name}' effect {field} must be at least 1");
        match effect {
            Effect::DisableHexes { count, .. } if *count == 0 => {
                return Err(zero("DisableHexes.count"));
            }
            Effect::Burn { amount } if *amount == 0 => return Err(zero("Burn.amount")),
            Effect::RestoreHexes { count } if *count == 0 => {
                return Err(zero("RestoreHexes.count"));
            }
            Effect::ModifyIncomingDisables { amount } if *amount == 0 => {
                return Err(zero("ModifyIncomingDisables.amount"));
            }
            Effect::Reveal { tier } if *tier == 0 => return Err(zero("Reveal.tier")),
            Effect::Illuminate { radius } if *radius == 0 => return Err(zero("Illuminate.radius")),
            Effect::Displace { distance } if *distance == 0 => {
                return Err(zero("Displace.distance"));
            }
            Effect::SetTerrain { substance } | Effect::SpawnWall { substance }
                if substance.is_empty() =>
            {
                return Err(format!("spell '{name}' names an empty substance"));
            }
            _ => {}
        }
    }

    // A spell must do something: at least one effect, or a defensive enchantment whose
    // whole point is the disable reduction it carries in its casting axis.
    let is_defensive = matches!(spell.casting, CastingAxis::Enchantment { defense } if defense > 0);
    if spell.effects.is_empty() && !is_defensive {
        return Err(format!(
            "spell '{name}' has no effects and no defensive enchantment — it would do nothing"
        ));
    }
    Ok(())
}

/// Spells indexed by the [`SpellId`] assigned from sorted names.
#[derive(Resource, Reflect, Debug, Clone, Default)]
#[reflect(Resource)]
pub struct SpellBook {
    /// Names indexed by id; `by_id[i]` is the name of `SpellId(i)`.
    by_id: Vec<String>,
    #[reflect(ignore)]
    by_name: HashMap<String, SpellId>,
    #[reflect(ignore)]
    spells: HashMap<SpellId, Spell>,
}

impl SpellBook {
    /// The id a name maps to, or [`None`] if there is no such spell.
    #[must_use]
    pub fn id(&self, name: &str) -> Option<SpellId> {
        self.by_name.get(name).copied()
    }

    /// The name of a spell, for logs and content resolution.
    #[must_use]
    pub fn name(&self, id: SpellId) -> Option<&str> {
        self.by_id.get(id.0 as usize).map(String::as_str)
    }

    /// The definition of a spell, or [`None`] if the id is not in the book.
    #[must_use]
    pub fn spell(&self, id: SpellId) -> Option<&Spell> {
        self.spells.get(&id)
    }

    /// Every spell, in id order — for content resolution and the dev-feature dump.
    pub fn iter(&self) -> impl Iterator<Item = (SpellId, &str, &Spell)> + '_ {
        self.by_id.iter().enumerate().filter_map(|(index, name)| {
            let id = SpellId(u16::try_from(index).unwrap_or(u16::MAX));
            self.spells.get(&id).map(|spell| (id, name.as_str(), spell))
        })
    }

    /// How many spells the book holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    /// Whether the book is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// Builds a book from a loaded file, assigning ids from sorted names.
    #[must_use]
    pub fn from_file(file: &SpellFile) -> Self {
        let mut by_id: Vec<String> = file.spells.keys().cloned().collect();
        by_id.sort();

        let mut by_name: HashMap<String, SpellId> = HashMap::default();
        let mut spells: HashMap<SpellId, Spell> = HashMap::default();
        for (index, name) in by_id.iter().enumerate() {
            let id = SpellId(u16::try_from(index).unwrap_or(u16::MAX));
            by_name.insert(name.clone(), id);
            if let Some(spell) = file.spells.get(name) {
                spells.insert(id, spell.clone());
            }
        }

        Self {
            by_id,
            by_name,
            spells,
        }
    }
}

/// Registers the spell book for loading.
pub fn plugin(app: &mut App) {
    app.register_type::<SpellBook>();
    app.load_settings::<SpellFile>("config/spells.ron", CONFIG_EXTENSIONS);
    register_book_builder(app);
}

/// Rebuilds the book when the file loads or hot-reloads, but never during gameplay.
fn register_book_builder(app: &mut App) {
    app.add_systems(
        Update,
        build_spellbook.run_if(not(in_state(Screen::Gameplay))),
    );
}

/// Turns the loaded file into the indexed book, and rebuilds it on hot-reload.
fn build_spellbook(
    mut commands: Commands,
    file: Option<Res<SpellFile>>,
    book: Option<Res<SpellBook>>,
) {
    let Some(file) = file else { return };
    if !file.is_changed() && book.is_some() {
        return;
    }
    commands.insert_resource(SpellBook::from_file(&file));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn targeting() -> TargetingSpec {
        TargetingSpec {
            range: 3,
            shape: TargetShape::Single,
            needs_los: true,
        }
    }

    fn gem(element: &str, mana: u16) -> GemRequirement {
        GemRequirement {
            element: element.to_owned(),
            mana,
        }
    }

    fn ember() -> Spell {
        Spell {
            requirements: vec![gem("Fire", 1)],
            casting: CastingAxis::Evocation,
            mana: ManaAxis::Fixed,
            co_castable: false,
            targeting: targeting(),
            effects: vec![Effect::DisableHexes {
                count: 1,
                targeted: false,
            }],
        }
    }

    fn test_file() -> SpellFile {
        let mut spells = HashMap::default();
        spells.insert("Ember".to_owned(), ember());
        spells.insert(
            "Flamethrower".to_owned(),
            Spell {
                requirements: vec![gem("Fire", 1), gem("Fire", 1)],
                casting: CastingAxis::Evocation,
                mana: ManaAxis::Variable,
                co_castable: true,
                targeting: TargetingSpec {
                    range: 2,
                    shape: TargetShape::Line,
                    needs_los: true,
                },
                effects: vec![Effect::Burn { amount: 2 }],
            },
        );
        SpellFile { spells }
    }

    fn shipped_file() -> SpellFile {
        ron::from_str(include_str!("../../../assets/config/spells.ron"))
            .expect("the shipped spell file should parse and validate")
    }

    #[test]
    fn shipped_spells_parse() {
        let book = SpellBook::from_file(&shipped_file());
        assert!(book.id("Ember").is_some());
        assert!(book.id("Fireball").is_some());
    }

    /// Every closed-enum effect variant must appear in the shipped content, or the
    /// pipeline is not actually exercised end to end.
    #[test]
    fn shipped_spells_cover_every_effect_variant() {
        let file = shipped_file();
        let mut seen = std::collections::HashSet::new();
        for spell in file.spells.values() {
            for effect in &spell.effects {
                seen.insert(std::mem::discriminant(effect));
            }
        }
        // The ten variants of Effect (ClearTerrain has no fields, so build it directly).
        let all = [
            Effect::DisableHexes {
                count: 1,
                targeted: false,
            },
            Effect::Burn { amount: 1 },
            Effect::RestoreHexes { count: 1 },
            Effect::ModifyIncomingDisables { amount: 1 },
            Effect::Reveal { tier: 1 },
            Effect::Illuminate { radius: 1 },
            Effect::SetTerrain {
                substance: "stone".to_owned(),
            },
            Effect::ClearTerrain,
            Effect::SpawnWall {
                substance: "stone".to_owned(),
            },
            Effect::Displace { distance: 1 },
        ];
        for effect in &all {
            assert!(
                seen.contains(&std::mem::discriminant(effect)),
                "shipped spells never use {effect:?}"
            );
        }
    }

    #[test]
    fn ids_do_not_depend_on_file_order() {
        let first = SpellBook::from_file(&test_file());
        let second = SpellBook::from_file(&test_file());
        for name in ["Ember", "Flamethrower"] {
            assert_eq!(
                first.id(name),
                second.id(name),
                "{name} moved between builds"
            );
        }
    }

    #[test]
    fn ritual_is_variable_and_co_castable() {
        let book = SpellBook::from_file(&test_file());
        let flamethrower = book
            .id("Flamethrower")
            .expect("test file defines Flamethrower");
        let ember = book.id("Ember").expect("test file defines Ember");
        assert!(book.spell(flamethrower).expect("present").is_ritual());
        assert!(
            !book.spell(ember).expect("present").is_ritual(),
            "Ember is a binary evocation"
        );
    }

    #[test]
    fn tier_is_the_requirement_count() {
        assert_eq!(ember().tier(), 1);
    }

    #[test]
    fn validate_rejects_over_tier_spells() {
        let mut file = test_file();
        let mut over = ember();
        over.requirements = std::iter::repeat_with(|| gem("Fire", 1)).take(7).collect();
        file.spells.insert("Inferno".to_owned(), over);
        assert!(file.validate().is_err(), "tier 7 exceeds the six-gem ring");
    }

    #[test]
    fn validate_rejects_a_do_nothing_spell() {
        let mut file = test_file();
        let mut inert = ember();
        inert.effects.clear();
        inert.casting = CastingAxis::Evocation;
        file.spells.insert("Fizzle".to_owned(), inert);
        assert!(
            file.validate().is_err(),
            "an evocation with no effects does nothing"
        );
    }

    #[test]
    fn a_defensive_enchantment_may_have_no_effects() {
        let mut file = test_file();
        let mut shield = ember();
        shield.effects.clear();
        shield.casting = CastingAxis::Enchantment { defense: 2 };
        file.spells.insert("Shield".to_owned(), shield);
        assert!(
            file.validate().is_ok(),
            "a defensive enchantment's point is its defense"
        );
    }

    #[test]
    fn validate_rejects_zero_mana_requirements() {
        let mut file = test_file();
        let mut free = ember();
        free.requirements = vec![gem("Fire", 0)];
        file.spells.insert("Freebie".to_owned(), free);
        assert!(file.validate().is_err(), "a gem must contribute mana");
    }
}
