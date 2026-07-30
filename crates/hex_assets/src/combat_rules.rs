//! Frozen, session-local Combat Lab rule profiles.
//!
//! Authored [`CombatSettings`] remain the shipped source of truth. A profile copies
//! only the six numeric seams the Lab is allowed to tune, validates them against
//! explicit bounds, and produces an effective session value without writing
//! `combat.ron`.

use bevy::prelude::Reflect;
use serde::{Deserialize, Serialize};

use crate::CombatSettings;

/// Current serialized Combat Lab rules-profile schema.
pub const COMBAT_RULES_PROFILE_VERSION: u16 = 1;

/// Named profile identity shown by Combat Lab.
#[derive(Reflect, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CombatRulesPreset {
    /// The exact values loaded from shipped `combat.ron`.
    Shipped,
    /// The shipped rules with movement reduced to two steps per turn.
    TacticalTwoStep,
    /// A bounded player-edited profile.
    Custom,
}

/// One numeric seam exposed by Combat Lab.
#[derive(Reflect, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CombatRuleField {
    /// Movement budget granted at the start of each turn.
    MovementPerTurn,
    /// Raw lattice cells disabled by a basic strike.
    StrikeDisables,
    /// Horizontal range at which combat begins.
    EngageRange,
    /// Extra separation required before combat ends.
    DisengageMargin,
    /// Elevation levels required for one bonus spell-range hex.
    LevelsPerBonusRange,
    /// Further round rollovers a tier of Reveal survives.
    RevealDuration,
}

impl CombatRuleField {
    /// Every exposed field in stable UI and serialization order.
    pub const ALL: [Self; 6] = [
        Self::MovementPerTurn,
        Self::StrikeDisables,
        Self::EngageRange,
        Self::DisengageMargin,
        Self::LevelsPerBonusRange,
        Self::RevealDuration,
    ];

    /// Short player-facing label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::MovementPerTurn => "Movement per turn",
            Self::StrikeDisables => "Strike disables",
            Self::EngageRange => "Engage range",
            Self::DisengageMargin => "Disengage margin",
            Self::LevelsPerBonusRange => "Levels per bonus range",
            Self::RevealDuration => "Reveal duration",
        }
    }

    /// Plain-language mechanical meaning.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::MovementPerTurn => "How many surface steps a unit may spend each turn.",
            Self::StrikeDisables => {
                "How many lattice cells a basic strike threatens before prevention."
            }
            Self::EngageRange => "How close opposing units must be before the game enters combat.",
            Self::DisengageMargin => {
                "How much farther than engage range opponents must separate to leave combat."
            }
            Self::LevelsPerBonusRange => {
                "How many levels of high ground grant one extra hex of spell range."
            }
            Self::RevealDuration => {
                "How many further round rollovers each Reveal tier remains known."
            }
        }
    }

    /// Inclusive supported numeric range.
    #[must_use]
    pub const fn bounds(self) -> CombatRuleBounds {
        match self {
            Self::MovementPerTurn => CombatRuleBounds { min: 1, max: 12 },
            Self::StrikeDisables => CombatRuleBounds { min: 1, max: 12 },
            Self::EngageRange => CombatRuleBounds { min: 1, max: 24 },
            Self::DisengageMargin => CombatRuleBounds { min: 1, max: 12 },
            Self::LevelsPerBonusRange => CombatRuleBounds { min: 1, max: 32 },
            Self::RevealDuration => CombatRuleBounds { min: 1, max: 12 },
        }
    }
}

/// Inclusive numeric bounds for one exposed rule.
#[derive(Reflect, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct CombatRuleBounds {
    /// Smallest accepted value.
    pub min: u32,
    /// Largest accepted value.
    pub max: u32,
}

impl CombatRuleBounds {
    /// Whether `value` is admitted by this range.
    #[must_use]
    pub const fn contains(self, value: u32) -> bool {
        value >= self.min && value <= self.max
    }
}

/// One labelled difference between a selected profile and shipped settings.
#[derive(Reflect, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct CombatRuleChange {
    /// Changed rule.
    pub field: CombatRuleField,
    /// Value from shipped `combat.ron`.
    pub shipped: u32,
    /// Selected session value.
    pub selected: u32,
}

/// Versioned rules frozen into one Sandbox or fixture launch.
#[derive(Reflect, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CombatRulesProfile {
    /// Serialized schema version.
    pub version: u16,
    /// Named preset or bounded custom identity.
    pub preset: CombatRulesPreset,
    /// Movement budget granted at the start of each turn.
    pub movement_per_turn: u32,
    /// Raw lattice cells disabled by a basic strike.
    pub strike_disables: u16,
    /// Horizontal range at which combat begins.
    pub engage_range: u32,
    /// Extra separation required before combat ends.
    pub disengage_margin: u32,
    /// Elevation levels required for one bonus spell-range hex.
    pub levels_per_bonus_range: u32,
    /// Further round rollovers a tier of Reveal survives.
    pub reveal_duration: u32,
}

impl CombatRulesProfile {
    /// Captures the six exposed values from shipped settings.
    #[must_use]
    pub const fn shipped(settings: &CombatSettings) -> Self {
        Self {
            version: COMBAT_RULES_PROFILE_VERSION,
            preset: CombatRulesPreset::Shipped,
            movement_per_turn: settings.movement_per_turn,
            strike_disables: settings.strike_disables,
            engage_range: settings.engage_range,
            disengage_margin: settings.disengage_margin,
            levels_per_bonus_range: settings.levels_per_bonus_range,
            reveal_duration: settings.divination_rounds_per_tier,
        }
    }

    /// Builds the tactical preset from shipped settings, changing only movement.
    #[must_use]
    pub const fn tactical_two_step(settings: &CombatSettings) -> Self {
        Self {
            preset: CombatRulesPreset::TacticalTwoStep,
            movement_per_turn: 2,
            ..Self::shipped(settings)
        }
    }

    /// Converts the current numeric selection to an editable custom profile.
    #[must_use]
    pub fn custom_from(profile: &Self) -> Self {
        let mut custom = profile.clone();
        custom.preset = CombatRulesPreset::Custom;
        custom
    }

    /// Returns one field's value as the common UI numeric type.
    #[must_use]
    pub const fn value(&self, field: CombatRuleField) -> u32 {
        match field {
            CombatRuleField::MovementPerTurn => self.movement_per_turn,
            CombatRuleField::StrikeDisables => self.strike_disables as u32,
            CombatRuleField::EngageRange => self.engage_range,
            CombatRuleField::DisengageMargin => self.disengage_margin,
            CombatRuleField::LevelsPerBonusRange => self.levels_per_bonus_range,
            CombatRuleField::RevealDuration => self.reveal_duration,
        }
    }

    /// Sets one custom field, refusing values outside its published range.
    pub fn set_custom(&mut self, field: CombatRuleField, value: u32) -> Result<(), String> {
        validate_value(field, value)?;
        self.preset = CombatRulesPreset::Custom;
        match field {
            CombatRuleField::MovementPerTurn => self.movement_per_turn = value,
            CombatRuleField::StrikeDisables => {
                self.strike_disables = u16::try_from(value).map_err(|error| {
                    format!("{} does not fit its storage type: {error}", field.label())
                })?;
            }
            CombatRuleField::EngageRange => self.engage_range = value,
            CombatRuleField::DisengageMargin => self.disengage_margin = value,
            CombatRuleField::LevelsPerBonusRange => self.levels_per_bonus_range = value,
            CombatRuleField::RevealDuration => self.reveal_duration = value,
        }
        Ok(())
    }

    /// Validates schema, bounds, and named-preset identity against shipped settings.
    pub fn validate(&self, shipped: &CombatSettings) -> Result<(), String> {
        if self.version != COMBAT_RULES_PROFILE_VERSION {
            return Err(format!(
                "combat rules profile version {} is unsupported; expected {}",
                self.version, COMBAT_RULES_PROFILE_VERSION
            ));
        }
        for field in CombatRuleField::ALL {
            validate_value(field, self.value(field))?;
        }
        let expected = match self.preset {
            CombatRulesPreset::Shipped => Some(Self::shipped(shipped)),
            CombatRulesPreset::TacticalTwoStep => Some(Self::tactical_two_step(shipped)),
            CombatRulesPreset::Custom => None,
        };
        if let Some(expected) = expected {
            for field in CombatRuleField::ALL {
                if self.value(field) != expected.value(field) {
                    return Err(format!(
                        "{} preset must keep {} at {}, found {}",
                        preset_label(self.preset),
                        field.label(),
                        expected.value(field),
                        self.value(field)
                    ));
                }
            }
        }
        Ok(())
    }

    /// Projects every numeric difference from shipped settings in stable field order.
    #[must_use]
    pub fn changed_from_shipped(&self, shipped: &CombatSettings) -> Vec<CombatRuleChange> {
        let shipped_profile = Self::shipped(shipped);
        CombatRuleField::ALL
            .into_iter()
            .filter_map(|field| {
                let selected = self.value(field);
                let shipped = shipped_profile.value(field);
                (selected != shipped).then_some(CombatRuleChange {
                    field,
                    shipped,
                    selected,
                })
            })
            .collect()
    }

    /// Produces effective session settings without mutating authored content.
    pub fn effective_settings(&self, shipped: &CombatSettings) -> Result<CombatSettings, String> {
        self.validate(shipped)?;
        let mut effective = shipped.clone();
        effective.movement_per_turn = self.movement_per_turn;
        effective.strike_disables = self.strike_disables;
        effective.engage_range = self.engage_range;
        effective.disengage_margin = self.disengage_margin;
        effective.levels_per_bonus_range = self.levels_per_bonus_range;
        effective.divination_rounds_per_tier = self.reveal_duration;
        effective.validate()?;
        Ok(effective)
    }
}

fn validate_value(field: CombatRuleField, value: u32) -> Result<(), String> {
    let bounds = field.bounds();
    if bounds.contains(value) {
        Ok(())
    } else {
        Err(format!(
            "{} must be in {}..={}, found {}",
            field.label(),
            bounds.min,
            bounds.max,
            value
        ))
    }
}

const fn preset_label(preset: CombatRulesPreset) -> &'static str {
    match preset {
        CombatRulesPreset::Shipped => "Shipped",
        CombatRulesPreset::TacticalTwoStep => "Tactical two-step",
        CombatRulesPreset::Custom => "Custom",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_and_tactical_presets_have_exact_identity() {
        let settings = CombatSettings::default();
        let shipped = CombatRulesProfile::shipped(&settings);
        let tactical = CombatRulesProfile::tactical_two_step(&settings);

        assert_eq!(shipped.validate(&settings), Ok(()));
        assert_eq!(tactical.validate(&settings), Ok(()));
        assert!(shipped.changed_from_shipped(&settings).is_empty());
        assert_eq!(
            tactical.changed_from_shipped(&settings),
            vec![CombatRuleChange {
                field: CombatRuleField::MovementPerTurn,
                shipped: settings.movement_per_turn,
                selected: 2,
            }]
        );
    }

    #[test]
    fn every_field_accepts_both_bounds_and_refuses_values_outside() {
        let settings = CombatSettings::default();
        for field in CombatRuleField::ALL {
            let bounds = field.bounds();
            let mut profile =
                CombatRulesProfile::custom_from(&CombatRulesProfile::shipped(&settings));
            assert_eq!(profile.set_custom(field, bounds.min), Ok(()));
            assert_eq!(profile.validate(&settings), Ok(()));
            assert_eq!(profile.set_custom(field, bounds.max), Ok(()));
            assert_eq!(profile.validate(&settings), Ok(()));
            assert!(profile
                .set_custom(field, bounds.min.saturating_sub(1))
                .is_err());
            assert!(profile
                .set_custom(field, bounds.max.saturating_add(1))
                .is_err());
        }
    }

    #[test]
    fn named_preset_cannot_smuggle_changed_values() {
        let settings = CombatSettings::default();
        let mut profile = CombatRulesProfile::shipped(&settings);
        profile.movement_per_turn = profile.movement_per_turn.saturating_add(1);
        assert!(profile.validate(&settings).is_err());
    }

    #[test]
    fn profile_round_trip_preserves_version_and_values() {
        let settings = CombatSettings::default();
        let profile = CombatRulesProfile::tactical_two_step(&settings);
        let encoded = ron::to_string(&profile).expect("serialize");
        let decoded: CombatRulesProfile = ron::from_str(&encoded).expect("deserialize");
        assert_eq!(decoded, profile);
    }

    #[test]
    fn effective_settings_change_only_exposed_fields() {
        let settings = CombatSettings::default();
        let mut custom = CombatRulesProfile::custom_from(&CombatRulesProfile::shipped(&settings));
        custom
            .set_custom(CombatRuleField::MovementPerTurn, 3)
            .expect("valid");
        custom
            .set_custom(CombatRuleField::RevealDuration, 2)
            .expect("valid");
        let effective = custom.effective_settings(&settings).expect("valid profile");

        assert_eq!(effective.movement_per_turn, 3);
        assert_eq!(effective.divination_rounds_per_tier, 2);
        assert_eq!(effective.default_initiative, settings.default_initiative);
        assert_eq!(effective.initiative_policy, settings.initiative_policy);
        assert_eq!(effective.action_economy, settings.action_economy);
        assert_eq!(effective.channelling_trickle, settings.channelling_trickle);
        assert_eq!(effective.rout_policy, settings.rout_policy);
    }
}
