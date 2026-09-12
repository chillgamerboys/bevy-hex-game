//! Read-only expedition state and finite healing through admitted liquid geometry.

use super::*;
use crate::hex_prisms::HexPrism;
use crate::SKIN;
use bevy_math::Vec3;
use hex_core::arena::{ArenaExpeditionSites, ArenaTerrainView, ArenaVoxelGeometry};
use hex_core::TilePos;

/// One authored milestone's player reward, separate from immediate kill XP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub enum ExpeditionReward {
    /// Troll's separate +25 Fireball damage bonus.
    TrollDamage,
    /// Standard explosions after defeating the three Dragons.
    DragonExplosions,
    /// Shadow's +25 maximum HP without any current-HP healing.
    ShadowVitality,
}

impl ExpeditionReward {
    const ALL: [Self; 3] = [
        Self::TrollDamage,
        Self::DragonExplosions,
        Self::ShadowVitality,
    ];

    const fn role(self) -> ExpeditionRole {
        match self {
            Self::TrollDamage => ExpeditionRole::Troll,
            Self::DragonExplosions => ExpeditionRole::Dragon,
            Self::ShadowVitality => ExpeditionRole::MountainShadow,
        }
    }
}

const ORB_HEIGHT: f32 = 0.6;
const PICKUP_DISTANCE: f32 = 1.6;

#[derive(Debug, Default)]
pub(super) struct RewardState {
    position: Option<Vec3>,
    collected: bool,
    settlement_revision: Option<u64>,
}

impl ProgressState {
    fn milestone_defeated(&self, reward: ExpeditionReward) -> bool {
        let mut members = self
            .roster
            .iter()
            .filter(|(_, entry)| entry.role == Some(reward.role()))
            .peekable();
        members.peek().is_some() && members.all(|(id, _)| self.defeated.contains(id))
    }

    fn milestone_origin(&self, reward: ExpeditionReward) -> Option<Vec3> {
        self.death_order.iter().rev().find_map(|id| {
            self.roster
                .get(id)
                .filter(|entry| entry.role == Some(reward.role()))
                .and_then(|_| self.death_positions.get(id).copied())
        })
    }
}

/// Milestone defeat, orb availability and collection are distinct facts.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct MilestoneSnapshot {
    /// Stable gameplay reward identity.
    pub reward: ExpeditionReward,
    /// Every registered enemy for this milestone has been defeated.
    pub defeated: bool,
    /// Position of an actual available reward orb; never a hidden enemy location.
    pub available_position: Option<[f32; 3]>,
    /// The player has collected and received this reward exactly once.
    pub collected: bool,
}

/// Pool identity and consumption for joining to public world presentation facts.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FountainSnapshot {
    /// Canonical authored fountain name; no hidden location is disclosed here.
    pub name: String,
    /// The pool has already spent its one healing use.
    pub consumed: bool,
}

/// Run-local expedition projection. This snapshot grants no mutation authority.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ExpeditionSnapshot {
    /// All registered enemies, including Troll, Dragons and Shadow.
    pub enemies_total: usize,
    /// Registered enemy deaths, including uncredited deaths.
    pub enemies_defeated: usize,
    /// Registered forest minions (Goblins and Shamans), excluding Troll.
    pub forest_total: usize,
    /// Registered forest minion deaths.
    pub forest_defeated: usize,
    /// Registered Dragon deaths.
    pub dragons_defeated: usize,
    /// Stable Troll, Dragon and Shadow milestone ordering.
    pub milestones: [MilestoneSnapshot; 3],
    /// Canonically ordered pool identities and consumption; not map markers.
    pub fountains: Vec<FountainSnapshot>,
}

#[derive(Debug)]
pub(super) struct FountainState {
    cells: BTreeSet<TilePos>,
    consumed: bool,
}

impl ArenaSession {
    /// Settle actual reward orbs before collecting them. World changes can remove
    /// their footing, so retry pending/invalid settlement only on a new revision.
    pub(crate) fn advance_milestones(
        &mut self,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
    ) {
        let Some(state) = self.progression.as_ref().filter(|state| state.expedition) else {
            return;
        };
        let pending: Vec<_> = ExpeditionReward::ALL
            .into_iter()
            .filter_map(|reward| {
                let stored = state.rewards.get(&reward)?;
                (!stored.collected
                    && state.milestone_defeated(reward)
                    && stored.settlement_revision != Some(world.revision))
                .then(|| {
                    state
                        .milestone_origin(reward)
                        .map(|origin| (reward, origin, stored.position))
                })
                .flatten()
            })
            .collect();
        for (reward, origin, previous) in pending {
            let position = previous
                .filter(|position| {
                    self.reward_standing_pose(*position - Vec3::Y * ORB_HEIGHT, world, geometry)
                })
                .or_else(|| self.settle_reward(origin, world, geometry));
            if let Some(stored) = self
                .progression
                .as_mut()
                .and_then(|state| state.rewards.get_mut(&reward))
            {
                stored.position = position;
                stored.settlement_revision = Some(world.revision);
            }
        }
        let Some(human) = self.human_actor_id() else {
            return;
        };
        let Some(player) = self
            .actors
            .iter()
            .find(|actor| actor.id == human && actor.hp > 0.0)
        else {
            return;
        };
        let Some(state) = self.progression.as_ref() else {
            return;
        };
        let collected: Vec<_> = state
            .rewards
            .iter()
            .filter_map(|(reward, stored)| {
                let position = stored.position?;
                (!stored.collected
                    && player.center().distance(position) <= PICKUP_DISTANCE
                    && self.collision.sight_clear(player.eye(), position)
                    && self
                        .collision
                        .sweep_sphere(player.eye(), position - player.eye(), 0.05)
                        .is_none())
                .then_some(*reward)
            })
            .collect();
        let mut notices = Vec::new();
        for reward in collected {
            let Some(state) = self.progression.as_mut() else {
                continue;
            };
            let Some(stored) = state
                .rewards
                .get_mut(&reward)
                .filter(|stored| !stored.collected)
            else {
                continue;
            };
            stored.collected = true;
            stored.position = None;
            match reward {
                ExpeditionReward::TrollDamage => {
                    state.snapshot.damage_bonus += 25.0;
                    notices.push("Troll reward: Fireball damage +25.");
                }
                ExpeditionReward::DragonExplosions => {
                    state.snapshot.explosions_unlocked = true;
                    notices.push("Dragon reward: Fireball explosions unlocked.");
                }
                ExpeditionReward::ShadowVitality => {
                    if let Some(player) = self.actors.iter_mut().find(|actor| actor.id == human) {
                        player.max_hp += 25.0;
                    }
                    notices.push("Shadow reward: maximum health +25.");
                }
            }
        }
        if !notices.is_empty() {
            self.notice = notices.join(" ");
        }
    }

    fn settle_reward(
        &self,
        origin: Vec3,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
    ) -> Option<Vec3> {
        let state = self.progression.as_ref()?;
        let player = self.actors.iter().find(|actor| actor.expedition_player)?;
        let origin = if origin.is_finite() {
            origin
        } else {
            world.spawns.first().copied()?
        };
        let mut candidates: Vec<_> = state
            .settlement_supports
            .iter()
            .map(|support| support.coord.to_world(geometry.top(*support) + SKIN))
            .collect();
        // Preserve exact placement when the death lies above a reachable authored
        // surface. A tree crown or other isolated new platform must not trap the
        // reward; those deaths settle on the nearest valid authored route or shelf.
        let drop = geometry.top(TilePos::new(hex_core::HexCoord::ORIGIN, geometry.max_level))
            - geometry.top(TilePos::new(hex_core::HexCoord::ORIGIN, geometry.min_level))
            + geometry.level_height;
        if let Some(ground) = self
            .collision
            .ground(
                origin + Vec3::Y * SKIN * 8.0,
                player.dimensions.y,
                player.dimensions.x * 0.5,
                drop.max(1.0),
            )
            .filter(|ground| {
                let support = geometry.voxel_at(*ground - Vec3::Y * SKIN * 2.0);
                support.is_some_and(|support| state.settlement_supports.contains(&support))
            })
        {
            candidates.push(ground);
        }
        candidates.sort_by(|a, b| {
            a.distance_squared(origin)
                .total_cmp(&b.distance_squared(origin))
                .then_with(|| a.x.total_cmp(&b.x))
                .then_with(|| a.y.total_cmp(&b.y))
                .then_with(|| a.z.total_cmp(&b.z))
        });
        candidates
            .into_iter()
            .find(|feet| self.reward_standing_pose(*feet, world, geometry))
            .map(|feet| feet + Vec3::Y * ORB_HEIGHT)
    }

    /// Snapshot of an admitted expedition player run; absent for legacy packages.
    #[must_use]
    pub fn expedition_progress(&self) -> Option<ExpeditionSnapshot> {
        let state = self.progression.as_ref().filter(|state| state.expedition)?;
        let milestone = |reward: ExpeditionReward| {
            let stored = state.rewards.get(&reward);
            MilestoneSnapshot {
                reward,
                defeated: state.milestone_defeated(reward),
                available_position: stored
                    .and_then(|reward| reward.position)
                    .map(|point| point.to_array()),
                collected: stored.is_some_and(|reward| reward.collected),
            }
        };
        Some(ExpeditionSnapshot {
            enemies_total: state.roster.len(),
            enemies_defeated: state.defeated.len(),
            forest_total: state.minion_total(),
            forest_defeated: state.snapshot.forest_defeated,
            dragons_defeated: state.snapshot.dragons_defeated,
            milestones: [
                milestone(ExpeditionReward::TrollDamage),
                milestone(ExpeditionReward::DragonExplosions),
                milestone(ExpeditionReward::ShadowVitality),
            ],
            fountains: state
                .fountains
                .iter()
                .map(|(name, fountain)| FountainSnapshot {
                    name: name.clone(),
                    consumed: fountain.consumed,
                })
                .collect(),
        })
    }

    pub(crate) fn register_expedition_sites(&mut self, sites: &ArenaExpeditionSites) {
        let Some(state) = self.progression.as_mut().filter(|state| state.expedition) else {
            return;
        };
        state.settlement_supports = sites
            .encounters
            .values()
            .flat_map(|site| site.deployment.surfaces.iter().copied())
            .chain(
                sites
                    .routes
                    .values()
                    .flat_map(|route| route.ribbon.iter().copied()),
            )
            .collect();
        state.rewards = ExpeditionReward::ALL
            .into_iter()
            .map(|reward| (reward, RewardState::default()))
            .collect();
        state.fountains = sites
            .fountains
            .iter()
            .map(|(name, volume)| {
                (
                    name.clone(),
                    FountainState {
                        cells: volume.cells.clone(),
                        consumed: false,
                    },
                )
            })
            .collect();
    }

    pub(crate) fn advance_fountains(
        &mut self,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
    ) {
        let human = self.human_actor_id();
        let Some(state) = self.progression.as_mut().filter(|state| state.expedition) else {
            return;
        };
        let Some(player) = self
            .actors
            .iter_mut()
            .find(|actor| Some(actor.id) == human && actor.hp > 0.0 && actor.hp < actor.max_hp)
        else {
            return;
        };
        for fountain in state
            .fountains
            .values_mut()
            .filter(|fountain| !fountain.consumed)
        {
            if player.hp >= player.max_hp {
                break;
            }
            let entered = fountain.cells.iter().any(|pos| {
                let top = geometry.top(*pos);
                let Some(volume) = HexPrism::new(
                    pos.coord.to_world(top - geometry.level_height),
                    geometry.level_height,
                ) else {
                    return false;
                };
                volume
                    .overlap_capsule(
                        player.feet,
                        player.dimensions.y,
                        player.dimensions.x * 0.5,
                        SKIN,
                    )
                    .is_some()
                    && world.liquids.iter().any(|liquid| {
                        liquid.bottom.coord == pos.coord
                            && liquid.bottom.level <= pos.level
                            && liquid.top_level >= pos.level
                    })
            });
            if entered {
                player.hp = (player.hp + 40.0).min(player.max_hp);
                fountain.consumed = true;
            }
        }
    }
}
