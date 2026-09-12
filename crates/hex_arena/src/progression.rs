//! Run-local Forest–Massif progression and player-only spell tuning.

use std::collections::{BTreeMap, BTreeSet};

mod expedition;
pub use expedition::{ExpeditionReward, ExpeditionSnapshot, FountainSnapshot, MilestoneSnapshot};

use crate::{ActorId, ArenaSession, ArenaTuning, ExpeditionRole, Species};

/// Impact behavior frozen when an ordinary Fireball is released.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum FireballMode {
    /// Damage only the exact struck body, barrier, or terrain voxel.
    ContactOnly,
    /// Apply one ordinary radial explosion, without extra contact damage.
    #[default]
    Explosive,
}

/// Beneficial player upgrades available in the Forest–Massif pause menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum UpgradeStat {
    /// Advance one exact Shield footprint rank.
    ShieldSize,
    /// Add 0.15 units to an unlocked explosion radius.
    FireballSize,
    /// Multiply High Jump rise by 1.15, up to eight units.
    HighJumpHeight,
    /// Multiply Fireball reference speed by 1.10.
    ProjectileSpeed,
    /// Multiply Shield reference speed by 1.10.
    ShieldProjectileSpeed,
    /// Multiply ordinary walking speed by 1.10.
    WalkingSpeed,
    /// Multiply Shield cooldown by 0.85.
    ShieldCooldown,
    /// Multiply Fireball cooldown by 0.85.
    FireballCooldown,
    /// Multiply High Jump cooldown by 0.85.
    HighJumpCooldown,
    /// Multiply rewarded base Fireball damage by 1.15.
    FireballDamage,
    /// Multiply impact impulse by 1.15.
    FireballKnockback,
}

impl UpgradeStat {
    /// Stable presentation order; gravity has no purchase.
    pub const ALL: [Self; 11] = [
        Self::WalkingSpeed,
        Self::FireballDamage,
        Self::ProjectileSpeed,
        Self::ShieldProjectileSpeed,
        Self::FireballKnockback,
        Self::FireballCooldown,
        Self::ShieldCooldown,
        Self::HighJumpCooldown,
        Self::HighJumpHeight,
        Self::FireballSize,
        Self::ShieldSize,
    ];

    /// Maximum bankable purchases for this field.
    #[must_use]
    pub const fn max_ranks(self) -> u8 {
        match self {
            Self::WalkingSpeed | Self::ShieldSize => 4,
            _ => 5,
        }
    }
}

/// A gameplay-derived menu value, before or after exactly one purchase.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UpgradeValue {
    /// A speed, damage, time, radius, or height in its ordinary units.
    Scalar(f32),
    /// Width in columns and height in voxel levels.
    Dimensions(i32, i32),
}

/// Read-only rank and next-purchase values; unavailable purchases have no after value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UpgradePreview {
    /// Purchased ranks for this field.
    pub rank: u8,
    /// Maximum permitted rank.
    pub max_ranks: u8,
    /// Current effective value, including collected rewards.
    pub before: UpgradeValue,
    /// Next effective value, absent for a capped or locked field.
    pub after: Option<UpgradeValue>,
}

/// Effective expedition-only spell geometry and movement, separate from legacy presets.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerSpellProfile {
    /// Shield seed reference speed, independent of Fireball speed.
    pub shield_projectile_speed: f32,
    /// Exact width and height, including milestone bonuses.
    pub shield_dimensions: (i32, i32),
    /// Exact unlocked radial blast size.
    pub fireball_radius: f32,
    /// Ordinary walking speed, excluding impulses and gliding.
    pub walking_speed: f32,
}

/// Read-only player progress, with no hidden enemy positions or party activity.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct ProgressSnapshot {
    /// Current player level, starting at one.
    pub level: u32,
    /// XP earned toward the next level.
    pub xp: u32,
    /// Additional XP required for this complete level.
    pub xp_to_next: u32,
    /// Total credited kill XP this run.
    pub total_xp: u32,
    /// Level rewards that have not been spent.
    pub available_upgrades: u32,
    /// Defeated registered forest minions (Goblins/Shamans), excluding Troll.
    pub forest_defeated: usize,
    /// Defeated members of the three-Dragon roster.
    pub dragons_defeated: usize,
    /// All registered forest minions have been defeated.
    pub forest_cleared: bool,
    /// Player explosions are unlocked (pickup required in an expedition).
    pub explosions_unlocked: bool,
    /// Collected Wisp reward permits a charging-only trajectory guide.
    pub fireball_guide_unlocked: bool,
    /// Separate permanent damage reward (Troll pickup in an expedition).
    pub damage_bonus: f32,
    /// Every registered authored enemy has been defeated.
    pub completed: bool,
}

#[derive(Debug, Clone, Copy)]
struct RosterEntry {
    species: Species,
    role: Option<ExpeditionRole>,
}

impl RosterEntry {
    fn is_minion(self) -> bool {
        match self.role {
            Some(role) => matches!(
                role,
                ExpeditionRole::BabyGoblin | ExpeditionRole::Goblin | ExpeditionRole::Shaman
            ),
            None => matches!(self.species, Species::Goblin | Species::Shaman),
        }
    }

    fn is_dragon(self) -> bool {
        self.role.map_or(self.species == Species::Dragon, |role| {
            role == ExpeditionRole::Dragon
        })
    }

    fn xp(self) -> u32 {
        match self.role {
            Some(ExpeditionRole::Troll) => 50,
            Some(ExpeditionRole::MountainShadow) => 100,
            Some(ExpeditionRole::BabyGoblin | ExpeditionRole::Goblin) => 1,
            Some(ExpeditionRole::Shaman) => 5,
            Some(ExpeditionRole::Dragon) => 20,
            Some(ExpeditionRole::PlainWisp) => 3,
            Some(ExpeditionRole::PlainGolem) => 25,
            None => match self.species {
                Species::Goblin => 1,
                Species::Shaman => 5,
                Species::Dragon => 20,
                _ => 0,
            },
        }
    }
}

#[derive(Debug)]
pub(crate) struct ProgressState {
    snapshot: ProgressSnapshot,
    ranks: BTreeMap<UpgradeStat, u8>,
    fireball_speed_bonus: f32,
    shield_speed_bonus: f32,
    shield_dimension_bonus: i32,
    roster: BTreeMap<ActorId, RosterEntry>,
    expedition: bool,
    fountains: BTreeMap<String, expedition::FountainState>,
    rewards: BTreeMap<ExpeditionReward, expedition::RewardState>,
    settlement_supports: BTreeSet<hex_core::TilePos>,
    death_positions: BTreeMap<ActorId, bevy_math::Vec3>,
    death_order: Vec<ActorId>,
    defeated: BTreeSet<ActorId>,
    hits: BTreeMap<ActorId, u64>,
}

impl Default for ProgressState {
    fn default() -> Self {
        Self {
            snapshot: ProgressSnapshot {
                level: 1,
                xp: 0,
                xp_to_next: 10,
                total_xp: 0,
                available_upgrades: 0,
                forest_defeated: 0,
                dragons_defeated: 0,
                forest_cleared: false,
                explosions_unlocked: false,
                fireball_guide_unlocked: false,
                damage_bonus: 0.0,
                completed: false,
            },
            ranks: BTreeMap::new(),
            fireball_speed_bonus: 0.0,
            shield_speed_bonus: 0.0,
            shield_dimension_bonus: 0,
            roster: BTreeMap::new(),
            expedition: false,
            fountains: BTreeMap::new(),
            rewards: BTreeMap::new(),
            settlement_supports: BTreeSet::new(),
            death_positions: BTreeMap::new(),
            death_order: Vec::new(),
            defeated: BTreeSet::new(),
            hits: BTreeMap::new(),
        }
    }
}

impl ArenaSession {
    /// Whether the reset-accepted session is the authored Forest–Massif player run.
    #[must_use]
    pub fn is_forest_run(&self) -> bool {
        self.progression.is_some()
    }

    /// Run progress; other maps and spectators have no progression.
    #[must_use]
    pub fn progress(&self) -> Option<ProgressSnapshot> {
        self.progression.as_ref().map(|state| state.snapshot)
    }

    /// All authored enemies are defeated; movement and exploration may continue.
    #[must_use]
    pub fn completed_run(&self) -> bool {
        self.progress().is_some_and(|progress| progress.completed)
    }

    /// Effective player spell tuning, keeping the supplied enemy configuration unchanged.
    #[must_use]
    pub fn player_tuning(&self, base: &ArenaTuning) -> ArenaTuning {
        let Some(state) = &self.progression else {
            return base.clone();
        };
        state.tuning(base)
    }

    /// Current ordinary expedition walking speed; legacy movement remains separately tuned.
    #[must_use]
    pub fn player_walking_speed(&self) -> f32 {
        self.progression
            .as_ref()
            .map_or(4.5, |state| state.walking_speed())
    }

    /// Effective before/after values for one rank, independent of the current point balance.
    #[must_use]
    pub fn upgrade_preview(&self, stat: UpgradeStat) -> Option<UpgradePreview> {
        let state = self.progression.as_ref()?;
        let rank = state.rank(stat);
        Some(UpgradePreview {
            rank,
            max_ranks: stat.max_ranks(),
            before: state.upgrade_value(stat, rank),
            after: state
                .can_upgrade(stat)
                .then(|| state.upgrade_value(stat, rank + 1)),
        })
    }

    /// Whether one available level reward can improve this field now.
    #[must_use]
    pub fn can_upgrade(&self, stat: UpgradeStat) -> bool {
        self.progression
            .as_ref()
            .is_some_and(|state| state.snapshot.available_upgrades > 0 && state.can_upgrade(stat))
    }

    /// Spend exactly one level reward; rejected or capped choices change nothing.
    pub fn spend_upgrade(&mut self, stat: UpgradeStat) -> bool {
        if !self.can_upgrade(stat) {
            return false;
        }
        let Some(state) = &mut self.progression else {
            return false;
        };
        *state.ranks.entry(stat).or_default() += 1;
        state.snapshot.available_upgrades -= 1;
        true
    }

    pub(crate) fn player_fireball_mode(&self) -> FireballMode {
        if self.progress().is_some_and(|p| !p.explosions_unlocked) {
            FireballMode::ContactOnly
        } else {
            FireballMode::Explosive
        }
    }

    pub(crate) fn register_forest_roster(&mut self) {
        if let Some(state) = &mut self.progression {
            state.roster = self
                .actors
                .iter()
                .filter(|a| a.id != 0)
                .map(|actor| {
                    (
                        actor.id,
                        RosterEntry {
                            species: actor.species,
                            role: actor.expedition_role(),
                        },
                    )
                })
                .collect();
            state.expedition = state.roster.values().any(|entry| entry.role.is_some());
        }
    }

    pub(crate) fn record_player_hit(&mut self, owner: ActorId, victim: ActorId) {
        if self.human_actor_id() != Some(owner) || owner == victim {
            return;
        }
        let hostile = self
            .actors
            .iter()
            .find(|a| a.id == owner)
            .zip(self.actors.iter().find(|a| a.id == victim))
            .is_some_and(|(a, b)| a.team != b.team);
        if hostile {
            if let Some(state) = &mut self.progression {
                state.hits.insert(victim, self.tick);
            }
        }
    }

    pub(crate) fn reconcile_progression(&mut self) {
        let Some(state) = &mut self.progression else {
            return;
        };
        for actor in self.actors.iter().filter(|actor| actor.hp <= 0.0) {
            let Some(entry) = state.roster.get(&actor.id).copied() else {
                continue;
            };
            if !state.defeated.insert(actor.id) {
                continue;
            }
            state.death_positions.insert(actor.id, actor.feet);
            state.death_order.push(actor.id);
            if entry.is_minion() {
                state.snapshot.forest_defeated += 1;
            }
            if entry.is_dragon() {
                state.snapshot.dragons_defeated += 1;
            }
            if state
                .hits
                .get(&actor.id)
                .is_some_and(|tick| self.tick.saturating_sub(*tick) <= 1200)
            {
                let xp = entry.xp();
                self.player_knowledge.credited_defeat(actor.id);
                state.snapshot.xp += xp;
                state.snapshot.total_xp += xp;
                while state.snapshot.xp >= state.snapshot.xp_to_next {
                    state.snapshot.xp -= state.snapshot.xp_to_next;
                    state.snapshot.level += 1;
                    state.snapshot.available_upgrades += 1;
                    state.snapshot.xp_to_next = level_threshold(state.snapshot.level);
                }
            }
        }
        let minions = state.minion_total();
        state.snapshot.forest_cleared = minions > 0 && state.snapshot.forest_defeated == minions;
        if !state.expedition {
            // Legacy packages retain automatic clear rewards. Expedition pickup
            // authority will mutate these fields separately; deaths never do.
            state.snapshot.explosions_unlocked = state.snapshot.dragons_defeated == 3;
            state.snapshot.damage_bonus = if state.snapshot.forest_cleared {
                25.0
            } else {
                0.0
            };
        }
        state.snapshot.completed =
            !state.roster.is_empty() && state.defeated.len() == state.roster.len();
    }
}

impl ProgressState {
    fn minion_total(&self) -> usize {
        self.roster
            .values()
            .filter(|entry| entry.is_minion())
            .count()
    }

    fn rank(&self, stat: UpgradeStat) -> u8 {
        self.ranks.get(&stat).copied().unwrap_or(0)
    }

    fn walking_speed(&self) -> f32 {
        4.725 * 1.10_f32.powi(i32::from(self.rank(UpgradeStat::WalkingSpeed)))
    }

    fn upgrade_value(&self, stat: UpgradeStat, rank: u8) -> UpgradeValue {
        let r = i32::from(rank);
        UpgradeValue::Scalar(match stat {
            UpgradeStat::WalkingSpeed => 4.725 * 1.10_f32.powi(r),
            UpgradeStat::FireballDamage => (15.0 + self.snapshot.damage_bonus) * 1.15_f32.powi(r),
            UpgradeStat::ProjectileSpeed => (45.0 + self.fireball_speed_bonus) * 1.10_f32.powi(r),
            UpgradeStat::ShieldProjectileSpeed => {
                (45.0 + self.shield_speed_bonus) * 1.10_f32.powi(r)
            }
            UpgradeStat::FireballKnockback => 12.0 * 1.15_f32.powi(r),
            UpgradeStat::FireballCooldown => 0.5 * 0.85_f32.powi(r),
            UpgradeStat::ShieldCooldown => 5.0 * 0.85_f32.powi(r),
            UpgradeStat::HighJumpCooldown => 7.0 * 0.85_f32.powi(r),
            UpgradeStat::HighJumpHeight => (4.0 * 1.15_f32.powi(r)).min(8.0),
            UpgradeStat::FireballSize => 2.5 + 0.15 * f32::from(rank),
            UpgradeStat::ShieldSize => {
                let (width, height) = match rank {
                    1 => (6, 5),
                    2 => (6, 6),
                    3 => (7, 6),
                    4.. => (7, 7),
                    _ => (5, 5),
                };
                return UpgradeValue::Dimensions(
                    width + self.shield_dimension_bonus,
                    height + self.shield_dimension_bonus,
                );
            }
        })
    }

    fn scalar(&self, stat: UpgradeStat) -> f32 {
        match self.upgrade_value(stat, self.rank(stat)) {
            UpgradeValue::Scalar(value) => value,
            UpgradeValue::Dimensions(_, _) => 0.0,
        }
    }

    fn tuning(&self, base: &ArenaTuning) -> ArenaTuning {
        let mut value = base.clone();
        value.shield_size = 1;
        value.fireball_size = 1;
        value.high_jump_height = self.scalar(UpgradeStat::HighJumpHeight);
        value.projectile_speed = self.scalar(UpgradeStat::ProjectileSpeed);
        value.projectile_gravity = 12.0;
        value.shield_cooldown = self.scalar(UpgradeStat::ShieldCooldown);
        value.fireball_cooldown = self.scalar(UpgradeStat::FireballCooldown);
        value.high_jump_cooldown = self.scalar(UpgradeStat::HighJumpCooldown);
        value.fireball_damage = self.scalar(UpgradeStat::FireballDamage);
        value.fireball_knockback = self.scalar(UpgradeStat::FireballKnockback);
        let shield_dimensions =
            match self.upgrade_value(UpgradeStat::ShieldSize, self.rank(UpgradeStat::ShieldSize)) {
                UpgradeValue::Dimensions(width, height) => (width, height),
                UpgradeValue::Scalar(_) => (5, 5),
            };
        value.player_profile = Some(PlayerSpellProfile {
            shield_projectile_speed: self.scalar(UpgradeStat::ShieldProjectileSpeed),
            shield_dimensions,
            fireball_radius: self.scalar(UpgradeStat::FireballSize),
            walking_speed: self.walking_speed(),
        });
        value
    }

    fn can_upgrade(&self, stat: UpgradeStat) -> bool {
        self.rank(stat) < stat.max_ranks()
            && (stat != UpgradeStat::FireballSize || self.snapshot.explosions_unlocked)
    }
}

fn level_threshold(level: u32) -> u32 {
    // Compute ceil(10 * (3/2)^(level-1)) without cumulative per-level rounding.
    let exponent = level.saturating_sub(1);
    let numerator = 3_u64
        .checked_pow(exponent)
        .and_then(|value| value.checked_mul(10));
    let denominator = 2_u64.checked_pow(exponent);
    numerator
        .zip(denominator)
        .and_then(|(n, d)| u32::try_from(n.div_ceil(d)).ok())
        .unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests;
