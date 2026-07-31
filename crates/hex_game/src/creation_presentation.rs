//! Read-only presentation projections shared by the Creator and Combat Lab.
//!
//! These summaries are deliberately derived from creator blueprints instead of stored.
//! A saved record therefore cannot claim to do something its lattice or spells no
//! longer provide, and every Wave 6 screen uses the same wording.

use hex_assets::{
    creator_character_issues, creator_spell_issues, CastingAxis, CreationCellKind,
    CreationLibraryFile, Effect, ElementCatalog, SavedCharacter, SavedSpell, Spell, SpellBook,
    SpellReference, TargetShape,
};

/// Compact, factual description of one spell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SpellBuildSummary {
    pub(crate) name: String,
    pub(crate) requirements: Vec<String>,
    pub(crate) casting: String,
    pub(crate) targeting: String,
    pub(crate) effects: Vec<String>,
    pub(crate) sentence: String,
    pub(crate) issues: Vec<String>,
}

/// Compact, factual description of one character.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CharacterBuildSummary {
    pub(crate) name: String,
    pub(crate) cells: usize,
    pub(crate) attunement: Vec<String>,
    pub(crate) spells: Vec<SpellBuildSummary>,
    pub(crate) capabilities: Vec<String>,
    pub(crate) issues: Vec<String>,
}

impl SpellBuildSummary {
    pub(crate) fn from_saved(saved: &SavedSpell, elements: Option<&ElementCatalog>) -> Self {
        let issues = elements.map_or_else(
            || vec!["Element catalog is still loading.".to_owned()],
            |elements| creator_spell_issues(saved, elements),
        );
        Self::from_spell(saved.name.clone(), &saved.spell, issues)
    }

    pub(crate) fn from_spell(name: impl Into<String>, spell: &Spell, issues: Vec<String>) -> Self {
        let requirements = spell
            .requirements
            .iter()
            .map(|requirement| format!("{} ×{}", requirement.element, requirement.mana))
            .collect();
        let casting = match spell.casting {
            CastingAxis::Evocation => "Evocation".to_owned(),
            CastingAxis::Enchantment { defense } => {
                format!("Enchantment · defense {defense}")
            }
        };
        let targeting = format!(
            "{} · range {}",
            match spell.targeting.shape {
                TargetShape::SelfCast => "Self",
                TargetShape::Single => "Single target",
                _ => "Unsupported target",
            },
            spell.targeting.range
        );
        let effects = spell.effects.iter().map(effect_summary).collect::<Vec<_>>();
        let outcome = if effects.is_empty() {
            match spell.casting {
                CastingAxis::Enchantment { defense } if defense > 0 => {
                    format!("Defense {defense}")
                }
                _ => "No delivered effect".to_owned(),
            }
        } else {
            effects.join(", then ")
        };
        let sentence = format!("{targeting} · {outcome}");
        Self {
            name: name.into(),
            requirements,
            casting,
            targeting,
            effects,
            sentence,
            issues,
        }
    }
}

impl CharacterBuildSummary {
    pub(crate) fn from_saved(
        character: &SavedCharacter,
        library: &CreationLibraryFile,
        elements: Option<&ElementCatalog>,
        shipped: Option<&SpellBook>,
    ) -> Self {
        let issues = match (elements, shipped) {
            (Some(elements), Some(shipped)) => {
                creator_character_issues(character, library, elements, shipped)
            }
            _ => vec!["Content catalogs are still loading.".to_owned()],
        };
        let attunement = character
            .attunement
            .iter()
            .filter(|(_, capacity)| **capacity > 0)
            .map(|(element, capacity)| {
                let channel = character.channelling.get(element).copied().unwrap_or(0);
                format!("{element} {capacity}/{channel}")
            })
            .collect();
        let spells = character
            .cells
            .iter()
            .filter_map(|cell| match &cell.kind {
                CreationCellKind::Spell(reference) => {
                    resolve_spell(reference, library, elements, shipped)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let mut capabilities = vec!["Strike".to_owned()];
        for spell in &spells {
            for effect in &spell.effects {
                push_unique(&mut capabilities, effect.clone());
            }
            if let Some(defense) = spell.casting.strip_prefix("Enchantment · defense ") {
                if defense != "0" {
                    push_unique(&mut capabilities, format!("Defense {defense}"));
                }
            }
        }
        Self {
            name: character.name.clone(),
            cells: character.cells.len(),
            attunement,
            spells,
            capabilities,
            issues,
        }
    }

    pub(crate) fn ready(&self) -> bool {
        self.issues.is_empty()
    }

    pub(crate) fn compact_line(&self) -> String {
        format!(
            "{} cells · {}",
            self.cells,
            if self.capabilities.is_empty() {
                "No capabilities".to_owned()
            } else {
                self.capabilities.join(" · ")
            }
        )
    }
}

fn resolve_spell(
    reference: &SpellReference,
    library: &CreationLibraryFile,
    elements: Option<&ElementCatalog>,
    shipped: Option<&SpellBook>,
) -> Option<SpellBuildSummary> {
    match reference {
        SpellReference::Shipped(name) => shipped
            .and_then(|book| {
                book.iter()
                    .find(|(_, candidate, _)| candidate == name)
                    .map(|(_, _, spell)| spell)
            })
            .map(|spell| SpellBuildSummary::from_spell(name.clone(), spell, Vec::new())),
        SpellReference::Custom(id) => library
            .spells
            .iter()
            .find(|saved| saved.id == *id)
            .map(|saved| SpellBuildSummary::from_saved(saved, elements)),
    }
}

pub(crate) fn effect_summary(effect: &Effect) -> String {
    match effect {
        Effect::DisableHexes { count, .. } => format!("Disable {count}"),
        Effect::Burn { turns } => format!("Burn {turns} turns"),
        Effect::RestoreHexes { count } => format!("Restore {count}"),
        Effect::Reveal { tier } => format!("Reveal tier {tier}"),
        Effect::ModifyIncomingDisables { amount } => format!("Defense {amount}"),
        _ => "Unsupported effect".to_owned(),
    }
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use hex_assets::{
        CreationCell, CustomCharacterId, CustomSpellId, GemRequirement, ManaAxis, SavedSpell,
        TargetingSpec, Trajectory,
    };

    use super::*;

    fn burn_spell() -> SavedSpell {
        SavedSpell {
            id: CustomSpellId(7),
            name: "Scorch".to_owned(),
            spell: Spell {
                requirements: vec![GemRequirement {
                    element: "Fire".to_owned(),
                    mana: 1,
                }],
                casting: CastingAxis::Evocation,
                mana: ManaAxis::Fixed,
                co_castable: false,
                targeting: TargetingSpec {
                    range: 4,
                    shape: TargetShape::Single,
                    trajectory: Trajectory::None,
                },
                effects: vec![
                    Effect::DisableHexes {
                        count: 1,
                        targeted: false,
                    },
                    Effect::Burn { turns: 2 },
                ],
            },
        }
    }

    #[test]
    fn ordered_effects_read_as_a_sentence() {
        let summary = SpellBuildSummary::from_spell("Scorch", &burn_spell().spell, Vec::new());
        assert_eq!(
            summary.sentence,
            "Single target · range 4 · Disable 1, then Burn 2 turns"
        );
        assert_eq!(summary.effects, ["Disable 1", "Burn 2 turns"]);
    }

    #[test]
    fn character_capabilities_are_derived_not_authored() {
        let spell = burn_spell();
        let library = CreationLibraryFile {
            spells: vec![spell],
            ..CreationLibraryFile::default()
        };
        let character = SavedCharacter {
            id: CustomCharacterId(3),
            name: "Tester".to_owned(),
            cells: vec![CreationCell {
                q: 0,
                r: 0,
                kind: CreationCellKind::Spell(SpellReference::Custom(CustomSpellId(7))),
            }],
            attunement: BTreeMap::from([("Fire".to_owned(), 2)]),
            channelling: BTreeMap::from([("Fire".to_owned(), 1)]),
        };
        let summary = CharacterBuildSummary::from_saved(&character, &library, None, None);
        assert_eq!(
            summary.capabilities,
            ["Strike", "Disable 1", "Burn 2 turns"]
        );
        assert_eq!(summary.attunement, ["Fire 2/1"]);
    }
}
