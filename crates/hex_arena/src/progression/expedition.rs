//! Read-only expedition state and finite healing through admitted liquid geometry.

use super::*;
use crate::hex_prisms::HexPrism;
use crate::SKIN;
use hex_core::arena::{ArenaExpeditionSites, ArenaTerrainView, ArenaVoxelGeometry};
use hex_core::TilePos;

/// One authored milestone's player reward, separate from immediate kill XP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum ExpeditionReward {
    /// Troll's separate +25 Fireball damage bonus.
    TrollDamage,
    /// Standard explosions after defeating the three Dragons.
    DragonExplosions,
    /// Shadow's +25 maximum HP without any current-HP healing.
    ShadowVitality,
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
    /// Snapshot of an admitted expedition player run; absent for legacy packages.
    #[must_use]
    pub fn expedition_progress(&self) -> Option<ExpeditionSnapshot> {
        let state = self.progression.as_ref().filter(|state| state.expedition)?;
        let milestone = |reward, role| {
            let members: Vec<_> = state
                .roster
                .iter()
                .filter(|(_, entry)| entry.role == Some(role))
                .map(|(id, _)| *id)
                .collect();
            MilestoneSnapshot {
                reward,
                defeated: !members.is_empty()
                    && members.iter().all(|id| state.defeated.contains(id)),
                // Orb spawning/collection is a separate next slice. A defeated
                // milestone must not imply that an orb exists or was collected.
                available_position: None,
                collected: false,
            }
        };
        Some(ExpeditionSnapshot {
            enemies_total: state.roster.len(),
            enemies_defeated: state.defeated.len(),
            forest_total: state.minion_total(),
            forest_defeated: state.snapshot.forest_defeated,
            dragons_defeated: state.snapshot.dragons_defeated,
            milestones: [
                milestone(ExpeditionReward::TrollDamage, ExpeditionRole::Troll),
                milestone(ExpeditionReward::DragonExplosions, ExpeditionRole::Dragon),
                milestone(
                    ExpeditionReward::ShadowVitality,
                    ExpeditionRole::MountainShadow,
                ),
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

    pub(crate) fn register_expedition_fountains(&mut self, sites: &ArenaExpeditionSites) {
        let Some(state) = self.progression.as_mut().filter(|state| state.expedition) else {
            return;
        };
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
