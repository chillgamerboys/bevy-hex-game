//! Grand expedition progression. World-authored sites never decide reward policy.
use crate::{Actor, ActorIntent, ArenaSession, Species, SKIN, STEP};
use bevy_math::Vec3;
use hex_core::arena::{ArenaTerrainView, ArenaVoxelGeometry};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

mod checkpoint;
mod finite;
pub use checkpoint::GrandCheckpointIdentity;
#[cfg(test)]
mod tests;

/// Stable elemental reward identities, independent of acquisition order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ShrineId {
    /// Stronger Fireballs and radial explosions.
    Fire,
    /// Faster projectiles and improved gliding.
    Air,
    /// Faster swimming and sailing.
    Water,
    /// Stronger newly created stone and faster running, never health.
    Earth,
    /// Larger spells and greater jump height.
    Plant,
}
impl ShrineId {
    /// Stable UI and verification order.
    pub const ALL: [Self; 5] = [Self::Fire, Self::Air, Self::Water, Self::Earth, Self::Plant];
    /// World-owned supported shrine anchor key.
    #[must_use]
    pub const fn anchor(self) -> &'static str {
        match self {
            Self::Fire => "shrine_fire",
            Self::Air => "shrine_air",
            Self::Water => "shrine_water",
            Self::Earth => "shrine_earth",
            Self::Plant => "shrine_plant",
        }
    }
}

/// Designer-facing scalar bonuses; changes never alter the accepted base movement.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrandTuning {
    /// Fire damage multiplier.
    pub fire_damage: f32,
    /// Fire blast-radius multiplier.
    pub fire_size: f32,
    /// Both projectile launch-speed multiplier.
    pub air_projectiles: f32,
    /// Glider speed and turning multiplier; wind influence remains unchanged.
    pub air_glider: f32,
    /// Swimming-speed multiplier.
    pub water_swim: f32,
    /// Sailing drive and maximum-speed multiplier.
    pub water_boat: f32,
    /// Ordinary ground-speed multiplier.
    pub earth_run: f32,
    /// Spell-footprint multiplier.
    pub plant_size: f32,
    /// Ordinary and High Jump height multiplier.
    pub plant_jump: f32,
}
impl Default for GrandTuning {
    fn default() -> Self {
        Self {
            fire_damage: 1.25,
            fire_size: 1.15,
            air_projectiles: 1.15,
            air_glider: 1.10,
            water_swim: 1.25,
            water_boat: 1.15,
            earth_run: 1.15,
            plant_size: 1.15,
            plant_jump: 1.25,
        }
    }
}
impl GrandTuning {
    /// Reject malformed or extreme player modifiers before changing a live run.
    pub fn validate(&self) -> Result<(), String> {
        if [
            self.fire_damage,
            self.fire_size,
            self.air_projectiles,
            self.air_glider,
            self.water_swim,
            self.water_boat,
            self.earth_run,
            self.plant_size,
            self.plant_jump,
        ]
        .into_iter()
        .all(|v| v.is_finite() && (1.0..=2.0).contains(&v))
        {
            Ok(())
        } else {
            Err("Grand shrine bonuses must be finite multipliers in 1..=2".into())
        }
    }
}

/// Read-only cumulative progression, suitable for the shrine and teleport HUD.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GrandProgressSnapshot {
    /// Each collected shrine exactly once, in stable order.
    pub shrines: Vec<ShrineId>,
    /// Shrines physically discovered, independent of claiming their blessing.
    pub discovered_shrines: Vec<ShrineId>,
    /// Shadow defeat grants the short teleport, never a Forest health reward.
    pub teleport_unlocked: bool,
    /// Remaining simulation seconds before another teleport.
    pub teleport_cooldown: f32,
    /// Last activated shrine, or the initial start before any shrine.
    pub respawn_anchor: Option<ShrineId>,
    /// Number of death recoveries during this run.
    pub deaths: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct GrandState {
    pub(super) acquired: BTreeSet<ShrineId>,
    pub(super) discovered: BTreeSet<ShrineId>,
    pub(super) last_shrine: Option<ShrineId>,
    pub(super) start: Vec3,
    pub(super) teleport_unlocked: bool,
    pub(super) teleport_cooldown: f32,
    pub(super) deaths: u32,
    pub(crate) admitted: BTreeSet<String>,
    pub(crate) sites: std::collections::BTreeMap<String, Vec3>,
    pub(crate) respawn_interest: Option<Vec3>,
    pub(crate) tuning: GrandTuning,
}
impl GrandState {
    pub(crate) fn new(start: Vec3) -> Self {
        Self {
            acquired: BTreeSet::new(),
            discovered: BTreeSet::new(),
            last_shrine: None,
            start,
            teleport_unlocked: false,
            teleport_cooldown: 0.0,
            deaths: 0,
            admitted: BTreeSet::new(),
            sites: Default::default(),
            respawn_interest: None,
            tuning: GrandTuning::default(),
        }
    }
    pub(crate) fn has(&self, shrine: ShrineId) -> bool {
        self.acquired.contains(&shrine)
    }
}

impl ArenaSession {
    /// Grand identity does not imply Forest objectives or an enemy-free session.
    #[must_use]
    pub const fn is_grand_run(&self) -> bool {
        self.grand.is_some()
    }
    /// Current Grand shrine and teleport facts; no hidden enemy state is disclosed.
    #[must_use]
    pub fn grand_progress(&self) -> Option<GrandProgressSnapshot> {
        self.grand.as_ref().map(|g| GrandProgressSnapshot {
            shrines: g.acquired.iter().copied().collect(),
            discovered_shrines: g.discovered.iter().copied().collect(),
            teleport_unlocked: g.teleport_unlocked,
            teleport_cooldown: g.teleport_cooldown,
            respawn_anchor: g.last_shrine,
            deaths: g.deaths,
        })
    }
    /// Install validated authored shrine tuning while retaining all acquired rewards.
    pub fn configure_grand(&mut self, tuning: GrandTuning) -> Result<(), String> {
        tuning.validate()?;
        self.configured_grand_tuning = tuning.clone();
        if let Some(g) = &mut self.grand {
            g.tuning = tuning;
        }
        Ok(())
    }
    /// Actor-owned collision interests; the world decides exact bounded residency.
    pub fn grand_actor_interests(&self) -> Vec<Vec3> {
        if self.grand.is_none() {
            return Vec::new();
        }
        let Some(player) = self.actors.iter().find(|a| a.id == 0) else {
            return Vec::new();
        };
        let Some(g) = &self.grand else {
            return Vec::new();
        };
        let mut interests: Vec<_> = self
            .actors
            .iter()
            .filter(|a| a.hp > 0.0)
            .map(|a| a.feet)
            .collect();
        interests.extend(self.projectiles.iter().map(|p| p.position));
        interests.extend(self.pending_walls.iter().map(|w| w.center));
        interests.extend(g.respawn_interest);
        interests.extend(
            g.sites
                .values()
                .filter(|p| p.distance(player.feet) < 100.0)
                .copied(),
        );
        interests.extend(
            self.encounter
                .dormant
                .values()
                .filter(|p| p.near(player.feet))
                .flat_map(|p| p.actors.iter().filter(|a| a.hp > 0.0).map(|a| a.feet)),
        );
        interests
    }
    pub(crate) fn earth_construction(&self, owner: crate::ActorId) -> bool {
        Some(owner) == self.human_actor_id()
            && self.grand.as_ref().is_some_and(|g| g.has(ShrineId::Earth))
    }
    pub(crate) fn advance_grand(
        &mut self,
        intent: ActorIntent,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
    ) {
        let Some(mut grand) = self.grand.take() else {
            return;
        };
        grand.teleport_cooldown = (grand.teleport_cooldown - STEP).max(0.0);
        grand.teleport_unlocked |= self
            .actors
            .iter()
            .chain(
                self.encounter
                    .dormant
                    .values()
                    .flat_map(|p| p.actors.iter()),
            )
            .any(|a| a.species == Species::Shadow && a.hp <= 0.0);
        let Some(player_index) = self.actors.iter().position(|a| a.id == 0) else {
            self.grand = Some(grand);
            return;
        };
        if self.actors.get(player_index).is_some_and(|a| a.hp <= 0.0) {
            let desired = grand
                .last_shrine
                .and_then(|s| world.anchors.get(s.anchor()).copied())
                .unwrap_or(grand.start);
            grand.respawn_interest = Some(desired);
            let template = self.actors.get(player_index).cloned();
            if let Some(mut player) = template {
                if !self.collision.needs_terrain(
                    desired,
                    Vec3::ZERO,
                    player.dimensions.y,
                    player.dimensions.x * 0.5,
                ) {
                    if let Some(feet) = crate::encounters::safe_spawn(
                        &player,
                        desired,
                        &self.actors,
                        &self.collision,
                        world,
                        geometry,
                    ) {
                        let max_hp = player.max_hp;
                        player = Actor::spawn(0, feet, player.aim);
                        player.configure_expedition_player();
                        player.max_hp = max_hp;
                        player.hp = max_hp;
                        player.free_flight = Some(Default::default());
                        player.marine = Some(Default::default());
                        if let Some(m) = &mut player.marine {
                            m.lab = true;
                            m.glider_wind_scale = 0.65;
                        }
                        if let Some(slot) = self.actors.get_mut(player_index) {
                            *slot = player;
                        }
                        grand.respawn_interest = None;
                        grand.deaths = grand.deaths.saturating_add(1);
                        self.outcome = None;
                        self.notice = "Returned to your last shrine. Progress and the changed world are retained.".into();
                    }
                }
            }
        }
        if let Some(player) = self.actors.get(player_index).filter(|a| a.hp > 0.0) {
            for shrine in ShrineId::ALL {
                let Some(position) = world.anchors.get(shrine.anchor()).copied() else {
                    continue;
                };
                if player.feet.distance(position) <= 2.4
                    && self
                        .collision
                        .sight_clear(player.eye(), position + Vec3::Y * 0.5)
                    && !self.collision.needs_terrain(
                        position,
                        Vec3::ZERO,
                        player.dimensions.y,
                        player.dimensions.x * 0.5,
                    )
                {
                    grand.discovered.insert(shrine);
                    if intent.interact {
                        if grand.acquired.insert(shrine) {
                            self.notice = format!("{shrine:?} shrine: blessing acquired.");
                        }
                        grand.last_shrine = Some(shrine);
                    } else if !grand.acquired.contains(&shrine) {
                        self.notice = format!("Press R to activate the {shrine:?} shrine.");
                    }
                }
            }
        }
        if intent.teleport && grand.teleport_unlocked && grand.teleport_cooldown <= STEP * 0.01 {
            if let Some(player) = self.actors.get(player_index) {
                if let Some(target) =
                    teleport_target(player, &self.actors, &self.collision, world, geometry)
                {
                    if let Some(player) = self.actors.get_mut(player_index) {
                        player.feet = target;
                        player.previous_feet = target;
                        player.body = Default::default();
                        player.body.grounded = true;
                        player.grounded = true;
                        player.clear_glider();
                        player.cancel_charge();
                        grand.teleport_cooldown = 6.0;
                    }
                } else {
                    self.notice = "Teleport needs visible, clear ground within 12 units.".into();
                }
            }
        }
        if let Some(player) = self.actors.get_mut(player_index) {
            player.jump_scale = if grand.has(ShrineId::Plant) {
                grand.tuning.plant_jump
            } else {
                1.0
            };
            player.glider_scale = if grand.has(ShrineId::Air) {
                grand.tuning.air_glider
            } else {
                1.0
            };
            player.swim_scale = if grand.has(ShrineId::Water) {
                grand.tuning.water_swim
            } else {
                1.0
            };
            player.boat_scale = if grand.has(ShrineId::Water) {
                grand.tuning.water_boat
            } else {
                1.0
            };
        }
        self.grand = Some(grand);
    }
}

fn teleport_target(
    player: &Actor,
    actors: &[Actor],
    collision: &crate::collision::CollisionWorld,
    world: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
) -> Option<Vec3> {
    if player.hp <= 0.0 || !player.aim.is_finite() {
        return None;
    }
    let origin = player.eye();
    let aim = player.aim.normalize_or(Vec3::NEG_Z);
    // Search only actual ray-visible support; gaps may be crossed, walls may not.
    let end = origin + aim * 12.0;
    let hit = collision.sweep_sphere(origin, end - origin, 0.01)?;
    if hit.normal.y < 0.5 {
        return None;
    }
    let contact = origin + (end - origin) * hit.fraction + Vec3::Y * SKIN * 4.0;
    let feet = collision.ground(
        contact + Vec3::Y * 0.1,
        player.dimensions.y,
        player.dimensions.x * 0.5,
        0.2,
    )?;
    if player.feet.distance(feet) > 12.0
        || collision.needs_terrain(origin, feet - origin, 0.05, 0.01)
        || collision.needs_terrain(
            feet,
            Vec3::ZERO,
            player.dimensions.y,
            player.dimensions.x * 0.5,
        )
    {
        return None;
    }
    let mut probe = player.clone();
    probe.feet = feet;
    (crate::shapes::clear(collision, &probe, feet, probe.body_yaw)
        && crate::encounters::dry(&probe, world, geometry)
        && actors
            .iter()
            .filter(|a| a.id != player.id && a.hp > 0.0)
            .all(|a| crate::encounters::body_overlap(&probe, a).is_none()))
    .then_some(feet)
}
