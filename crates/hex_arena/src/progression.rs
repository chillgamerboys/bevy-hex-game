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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpgradeStat {
    /// Increase shield dimensions by one preset.
    ShieldSize,
    /// Increase the unlocked explosion radius by one preset.
    FireballSize,
    /// Increase High Jump rise by half a unit.
    HighJumpHeight,
    /// Increase projectile reference speed by two units per second.
    ProjectileSpeed,
    /// Reduce Shield cooldown by half a second.
    ShieldCooldown,
    /// Reduce Fireball cooldown by a quarter second.
    FireballCooldown,
    /// Reduce High Jump cooldown by half a second.
    HighJumpCooldown,
    /// Increase base Fireball damage by five HP, independently of the forest bonus.
    FireballDamage,
    /// Increase impact impulse by one unit per second.
    FireballKnockback,
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
    player: ArenaTuning,
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
                damage_bonus: 0.0,
                completed: false,
            },
            player: ArenaTuning {
                projectile_speed: 45.0,
                projectile_gravity: 12.0,
                fireball_damage: 15.0,
                fireball_knockback: 12.0,
                fireball_cooldown: 0.5,
                ..Default::default()
            },
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
        let mut value = base.clone();
        let player = &state.player;
        value.shield_size = player.shield_size;
        value.fireball_size = player.fireball_size;
        value.high_jump_height = player.high_jump_height;
        value.projectile_speed = player.projectile_speed;
        value.projectile_gravity = player.projectile_gravity;
        value.shield_cooldown = player.shield_cooldown;
        value.fireball_cooldown = player.fireball_cooldown;
        value.high_jump_cooldown = player.high_jump_cooldown;
        value.fireball_damage = player.fireball_damage + state.snapshot.damage_bonus;
        value.fireball_knockback = player.fireball_knockback;
        value
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
        let player = &mut state.player;
        match stat {
            UpgradeStat::ShieldSize => player.shield_size += 1,
            UpgradeStat::FireballSize => player.fireball_size += 1,
            UpgradeStat::HighJumpHeight => player.high_jump_height += 0.5,
            UpgradeStat::ProjectileSpeed => {
                player.projectile_speed = (player.projectile_speed + 2.0).min(64.0)
            }
            UpgradeStat::ShieldCooldown => {
                player.shield_cooldown = (player.shield_cooldown - 0.5).max(0.5)
            }
            UpgradeStat::FireballCooldown => {
                player.fireball_cooldown = (player.fireball_cooldown - 0.25).max(0.25)
            }
            UpgradeStat::HighJumpCooldown => {
                player.high_jump_cooldown = (player.high_jump_cooldown - 0.5).max(0.5)
            }
            UpgradeStat::FireballDamage => player.fireball_damage += 5.0,
            UpgradeStat::FireballKnockback => player.fireball_knockback += 1.0,
        }
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

    fn can_upgrade(&self, stat: UpgradeStat) -> bool {
        let player = &self.player;
        match stat {
            UpgradeStat::ShieldSize => player.shield_size < 2,
            UpgradeStat::FireballSize => {
                self.snapshot.explosions_unlocked && player.fireball_size < 2
            }
            UpgradeStat::HighJumpHeight => player.high_jump_height < 8.0,
            UpgradeStat::ProjectileSpeed => player.projectile_speed < 64.0,
            UpgradeStat::ShieldCooldown => player.shield_cooldown > 0.5,
            UpgradeStat::FireballCooldown => player.fireball_cooldown > 0.25,
            UpgradeStat::HighJumpCooldown => player.high_jump_cooldown > 0.5,
            UpgradeStat::FireballDamage => player.fireball_damage < 100.0,
            UpgradeStat::FireballKnockback => player.fireball_knockback < 25.0,
        }
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
