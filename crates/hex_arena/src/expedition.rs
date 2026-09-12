//! Gameplay-owned authored identities and physical/combat profile overrides.

use crate::{Actor, ArenaTuning, EncounterTuning, Species};
use bevy_math::Vec3;
use std::borrow::Cow;

/// Stable authored identity independent of the shared creature shape/AI family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum ExpeditionRole {
    /// Slower, weaker Goblin in one of the five outskirts groups.
    BabyGoblin,
    /// Ordinary adult forest Goblin.
    Goblin,
    /// Party-local forest support caster.
    Shaman,
    /// Large forest boss with its own physical and combat profile.
    Troll,
    /// One of the three independent mountain encounters.
    Dragon,
    /// Fixed-strength spell opponent in the side arena.
    MountainShadow,
}

impl ExpeditionRole {
    pub(crate) const fn species(self) -> Species {
        match self {
            Self::BabyGoblin | Self::Goblin | Self::Troll => Species::Goblin,
            Self::Shaman => Species::Shaman,
            Self::Dragon => Species::Dragon,
            Self::MountainShadow => Species::Shadow,
        }
    }
}

impl Actor {
    /// Authored expedition identity; absent for legacy worlds and ordinary battles.
    #[must_use]
    pub const fn expedition_role(&self) -> Option<ExpeditionRole> {
        self.expedition_role
    }

    pub(crate) fn configure_expedition(&mut self, role: ExpeditionRole, tuning: &EncounterTuning) {
        self.configure_species(role.species(), tuning);
        self.expedition_role = Some(role);
        match role {
            ExpeditionRole::BabyGoblin => self.max_hp = 30.0,
            ExpeditionRole::Troll => {
                self.max_hp = 600.0;
                self.dimensions = Vec3::new(1.4, crate::BODY_HEIGHT * 3.0, 1.4);
            }
            ExpeditionRole::MountainShadow => self.max_hp = 125.0,
            _ => {}
        }
        self.hp = self.max_hp;
    }

    // The returned lifetime belongs only to base, not self. Callers may still move
    // the actor while consuming its frozen profile through ordinary controllers.
    pub(crate) fn expedition_tuning<'a>(&self, base: &'a ArenaTuning) -> Cow<'a, ArenaTuning> {
        if !matches!(
            self.expedition_role,
            Some(
                ExpeditionRole::BabyGoblin | ExpeditionRole::Troll | ExpeditionRole::MountainShadow
            )
        ) {
            return Cow::Borrowed(base);
        }
        let mut tuning = base.clone();
        match self.expedition_role {
            Some(ExpeditionRole::BabyGoblin) => {
                tuning.encounters.goblin_walk = 3.0;
                tuning.encounters.goblin_run = 5.0;
                tuning.encounters.swipe_damage = 8.0;
                tuning.encounters.swipe_cooldown = 1.4;
            }
            Some(ExpeditionRole::Troll) => {
                tuning.encounters.goblin_walk = 3.0;
                tuning.encounters.goblin_run = 3.0;
                tuning.encounters.swipe_damage = 24.0;
                tuning.encounters.swipe_range = 2.5;
                tuning.encounters.swipe_cooldown = 1.8;
                tuning.encounters.swipe_windup = 0.45;
                tuning.fireball_damage = 35.0;
                tuning.fireball_cooldown = 2.5;
            }
            Some(ExpeditionRole::MountainShadow) => {
                tuning.fireball_damage = 30.0;
                tuning.fireball_cooldown = 0.75;
                tuning.projectile_speed = 45.0;
                tuning.projectile_gravity = 12.0;
                tuning.fireball_size = 1;
            }
            _ => {}
        }
        Cow::Owned(tuning)
    }
}
