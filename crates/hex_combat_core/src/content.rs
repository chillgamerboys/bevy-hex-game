//! Immutable authored facts required by renderer-free active combat.
//!
//! The Bevy host resolves names and authored catalogs once, then freezes only these
//! serializable values into [`CombatState`](crate::CombatState). Simulation never
//! reaches back into assets, an `AssetServer`, ECS entities, or presentation.

use std::collections::BTreeMap;

use hex_core::{ElementId, SpellId};
use hex_lattice::{Casting, FusionTable, Requirement, SpellTable};
use serde::{Deserialize, Serialize};

/// One resolved mana requirement.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrozenRequirement {
    /// Required element.
    pub element: ElementId,
    /// Mana drawn from its source.
    pub mana: u16,
}

impl From<FrozenRequirement> for Requirement {
    fn from(value: FrozenRequirement) -> Self {
        Self {
            element: value.element,
            mana: value.mana,
        }
    }
}

/// Serializable casting-axis projection.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrozenCasting {
    /// Mana is consumed.
    Evocation,
    /// Mana remains locked and supplies flat defense.
    Enchantment {
        /// Flat incoming-disable prevention.
        defense: u16,
    },
}

impl From<FrozenCasting> for Casting {
    fn from(value: FrozenCasting) -> Self {
        match value {
            FrozenCasting::Evocation => Self::Evocation,
            FrozenCasting::Enchantment { defense } => Self::Enchantment { defense },
        }
    }
}

/// Active-combat target geometry supported by the pure reducer.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrozenTargeting {
    /// The caster's exact occupied surface.
    SelfOnly,
    /// One observed occupied surface in bidirectional body-specific reach.
    Touch,
    /// One observed exact surface within range.
    ExactSurface {
        /// Base horizontal range before high-ground bonus.
        range: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::FrozenTargeting;

    #[test]
    fn touch_targeting_round_trips_as_a_closed_discriminator() {
        let encoded =
            serde_json::to_string(&FrozenTargeting::Touch).expect("touch targeting serializes");
        let decoded: FrozenTargeting =
            serde_json::from_str(&encoded).expect("touch targeting deserializes");

        assert_eq!(decoded, FrozenTargeting::Touch);
        assert_ne!(decoded, FrozenTargeting::ExactSurface { range: 0 });
    }
}

/// One supported active-combat spell effect.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrozenEffect {
    /// Open a defender choice for a flat incoming-disable count.
    DisableHexes {
        /// Raw count before active enchantment prevention.
        count: u16,
    },
    /// Attach one personal-turn Burn.
    Burn {
        /// Number of affected-unit turns.
        turns: u16,
    },
    /// Open a caster-owned restoration decision.
    RestoreHexes {
        /// Maximum disabled cells restored.
        count: u16,
    },
    /// Reveal the complete target lattice.
    Reveal {
        /// Authored divination tier.
        tier: u32,
    },
}

/// One completely resolved spell used by active combat.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct FrozenSpell {
    /// Session-local id frozen with the matching lattice specs.
    pub id: SpellId,
    /// Stable authored name used by commands and evidence.
    pub name: String,
    /// Resolved lattice requirements.
    pub requirements: Vec<FrozenRequirement>,
    /// Mana behavior.
    pub casting: FrozenCasting,
    /// Pure target geometry.
    pub targeting: FrozenTargeting,
    /// Supported effects in authored order.
    pub effects: Vec<FrozenEffect>,
}

/// Complete immutable content projection for one combat.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct FrozenCombatContent {
    spells: BTreeMap<String, FrozenSpell>,
    names: BTreeMap<SpellId, String>,
    fusions: BTreeMap<ElementId, Vec<FrozenRequirement>>,
}

impl FrozenCombatContent {
    /// Builds a validated frozen content projection.
    pub fn new(
        spells: impl IntoIterator<Item = FrozenSpell>,
        fusions: impl IntoIterator<Item = (ElementId, Vec<FrozenRequirement>)>,
    ) -> Result<Self, String> {
        let mut by_name = BTreeMap::new();
        let mut names = BTreeMap::new();
        for spell in spells {
            if spell.name.trim().is_empty() {
                return Err("frozen spell name must not be blank".to_owned());
            }
            if spell.requirements.is_empty() {
                return Err(format!(
                    "frozen spell {:?} must keep at least one requirement",
                    spell.name
                ));
            }
            if spell
                .requirements
                .iter()
                .any(|requirement| requirement.mana == 0)
            {
                return Err(format!(
                    "frozen spell {:?} contains a zero-mana requirement",
                    spell.name
                ));
            }
            let modal_effects = spell
                .effects
                .iter()
                .filter(|effect| {
                    matches!(
                        effect,
                        FrozenEffect::DisableHexes { .. } | FrozenEffect::RestoreHexes { .. }
                    )
                })
                .count();
            if modal_effects > 1 {
                return Err(format!(
                    "frozen spell {:?} would open more than one simultaneous decision",
                    spell.name
                ));
            }
            if names.insert(spell.id, spell.name.clone()).is_some() {
                return Err(format!("duplicate frozen spell id {:?}", spell.id));
            }
            let name = spell.name.clone();
            if by_name.insert(name.clone(), spell).is_some() {
                return Err(format!("duplicate frozen spell name {name:?}"));
            }
        }
        Ok(Self {
            spells: by_name,
            names,
            fusions: fusions.into_iter().collect(),
        })
    }

    /// Finds a spell by its stable authored name.
    #[must_use]
    pub fn spell(&self, name: &str) -> Option<&FrozenSpell> {
        self.spells.get(name)
    }

    /// Finds a stable spell name by frozen session id.
    #[must_use]
    pub fn spell_name(&self, id: SpellId) -> Option<&str> {
        self.names.get(&id).map(String::as_str)
    }

    /// Every spell in lexical stable-name order.
    pub fn spells(&self) -> impl Iterator<Item = &FrozenSpell> {
        self.spells.values()
    }

    /// Whether no active-combat spell facts were frozen.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.spells.is_empty()
    }
}

impl SpellTable for FrozenCombatContent {
    fn requirements(&self, spell: SpellId) -> Vec<Requirement> {
        self.names
            .get(&spell)
            .and_then(|name| self.spells.get(name))
            .map(|spell| {
                spell
                    .requirements
                    .iter()
                    .copied()
                    .map(Requirement::from)
                    .collect()
            })
            .unwrap_or_else(|| {
                vec![Requirement {
                    element: ElementId::default(),
                    mana: u16::MAX,
                }]
            })
    }

    fn casting(&self, spell: SpellId) -> Casting {
        self.names
            .get(&spell)
            .and_then(|name| self.spells.get(name))
            .map_or(Casting::Evocation, |spell| spell.casting.into())
    }
}

impl FusionTable for FrozenCombatContent {
    fn recipe(&self, output: ElementId) -> Option<Vec<Requirement>> {
        self.fusions.get(&output).map(|requirements| {
            requirements
                .iter()
                .copied()
                .map(Requirement::from)
                .collect()
        })
    }
}
