//! Player-owned observation, remembered landmarks and disclosure-safe HUD facts.

use std::collections::{BTreeMap, BTreeSet};

use bevy_math::Vec3;
use hex_core::arena::{ArenaTerrainView, ArenaVoxelGeometry};

use crate::{Actor, ActorId, ArenaSession, ArenaTuning, ExpeditionRole, Species, Spell, STEP};

/// Camera facts supplied by local input; gameplay decides what may be disclosed.
#[derive(Debug, Clone, Copy)]
pub struct PlayerObservation {
    /// False while paused, unstarted, unfocused or using an observer camera.
    pub active: bool,
    /// Actual collision-retracted camera origin in world units.
    pub origin: Vec3,
    /// Camera's forward direction, normalized by observation admission.
    pub direction: Vec3,
    /// Vertical field of view in radians.
    pub vertical_fov: f32,
    /// Viewport width divided by height.
    pub aspect: f32,
    /// Physical viewport height in pixels.
    pub viewport_height: f32,
}

/// Only these authored encounters and pools produce persistent map markers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LandmarkKind {
    /// One of the three independent Dragon encounters.
    Dragon,
    /// The mountain arena opponent.
    Shadow,
    /// The forest boss.
    Troll,
    /// A single-use healing pool.
    Fountain,
    /// One of the independent lowland Golem encounters.
    Golem,
}

/// Remembered facts, never a live reference to an undisclosed enemy or pool.
#[derive(Debug, Clone, PartialEq)]
pub struct DiscoveredLandmark {
    /// Authored encounter or fountain name.
    pub id: String,
    /// Semantic marker identity.
    pub kind: LandmarkKind,
    /// Last observed position; hidden movement does not update it.
    pub position: Vec3,
    /// A witnessed or player-credited defeat, not remote enemy truth.
    pub defeated: bool,
    /// Last observed pool consumption state; false for encounters.
    pub consumed: bool,
}

/// Coarse health for one currently visible hostile; no exact HP is exported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetHealthSnapshot {
    /// Stable actor identity within this run.
    pub actor_id: ActorId,
    /// Base creature identity for the target label.
    pub species: Species,
    /// Authored role, when present.
    pub role: Option<ExpeditionRole>,
    /// Three above two thirds, two above one third, otherwise one.
    pub health_pips: u8,
}

/// One briefly announced, currently visible coarse enemy-health event.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnemyHealthCueSnapshot {
    /// Stable identity used to place the cue, without exporting hidden actors.
    pub actor_id: ActorId,
    /// Visible world-space point just above the actor's body.
    pub position: Vec3,
    /// Three white, two amber or one red health dots.
    pub health_pips: u8,
}

#[derive(Debug, Default)]
struct HealthAnnouncement {
    shown_band: Option<u8>,
    remaining: f32,
    visible: Option<EnemyHealthCueSnapshot>,
}

/// A short positive-damage confirmation without exact damage or hidden positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HitConfirmationSnapshot {
    /// Monotonically increasing positive player-hit event within this run.
    pub sequence: u64,
}

/// Current ability policy, including active play and held-charge ownership.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpellAvailabilityState {
    /// No living active player, or this held input must first be released.
    Unavailable,
    /// A new press can arm this spell now.
    Ready,
    /// An authoritative cooldown is still running.
    CoolingDown,
    /// This is the player's currently armed held spell.
    Charging,
}

/// Immutable facts for one of the player's three spell cards.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpellAvailability {
    /// Slot identity, in Shield/Fireball/High Jump order.
    pub spell: Spell,
    /// Actual current ability policy.
    pub state: SpellAvailabilityState,
    /// Remaining simulation cooldown seconds, including while paused.
    pub cooldown_remaining: f32,
    /// Current configured duration after player upgrades.
    pub cooldown_total: f32,
    /// Accepted held charge fraction in the inclusive zero-to-one range.
    pub charge_fraction: f32,
}

/// HUD authority; target values are copied only after successful observation.
#[derive(Debug, Clone, PartialEq)]
pub struct CombatFeedbackSnapshot {
    /// Event-driven dots above visible enemies; same-band hits never refresh them.
    pub health_cues: Vec<EnemyHealthCueSnapshot>,
    /// Crosshair target, or a still-visible recent player-hit target.
    pub target: Option<TargetHealthSnapshot>,
    /// Brief positive-damage pulse; terrain, friendly and self hits do not count.
    pub hit: Option<HitConfirmationSnapshot>,
    /// Shield, Fireball and High Jump availability.
    pub spells: [SpellAvailability; 3],
}

#[derive(Debug, Default)]
pub(crate) struct PlayerKnowledge {
    active: bool,
    sample_elapsed: f32,
    dwell: BTreeMap<String, u8>,
    landmarks: BTreeMap<String, DiscoveredLandmark>,
    actors: BTreeMap<ActorId, (String, LandmarkKind)>,
    credited_defeats: BTreeSet<ActorId>,
    visible_last_sample: BTreeSet<ActorId>,
    target: Option<TargetHealthSnapshot>,
    last_hit: Option<(ActorId, f32)>,
    hit_sequence: u64,
    health_announcements: BTreeMap<ActorId, HealthAnnouncement>,
    pending_first_hits: BTreeSet<ActorId>,
}

impl PlayerKnowledge {
    pub(crate) fn register_actor(&mut self, actor: &Actor, name: &str) {
        let kind = match actor.expedition_role() {
            Some(ExpeditionRole::Dragon) => LandmarkKind::Dragon,
            Some(ExpeditionRole::Troll) => LandmarkKind::Troll,
            Some(ExpeditionRole::MountainShadow) => LandmarkKind::Shadow,
            Some(ExpeditionRole::PlainGolem) => LandmarkKind::Golem,
            _ => return,
        };
        self.actors.insert(actor.id, (name.into(), kind));
    }

    pub(crate) fn credited_defeat(&mut self, actor: ActorId) {
        self.credited_defeats.insert(actor);
        if let Some((name, _)) = self.actors.get(&actor) {
            if let Some(known) = self.landmarks.get_mut(name) {
                known.defeated = true;
            }
        }
    }

    fn admit(&mut self, landmark: DiscoveredLandmark) {
        let dwell = self.dwell.entry(landmark.id.clone()).or_default();
        *dwell = dwell.saturating_add(1).min(5);
        if *dwell >= 5 || self.landmarks.contains_key(&landmark.id) {
            self.landmarks.insert(landmark.id.clone(), landmark);
        }
    }
}

impl ArenaSession {
    /// Admit camera observations at most ten times per second; inactive samples
    /// clear transient feedback/dwell but preserve discovered facts across death/pause.
    pub fn observe_player(
        &mut self,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        observation: PlayerObservation,
        dt: f32,
    ) {
        let player = self
            .human_actor_id()
            .and_then(|id| self.actors.iter().find(|a| a.id == id));
        let active =
            observation.valid() && !self.is_finished() && player.is_some_and(|a| a.hp > 0.0);
        self.player_knowledge.active = active;
        if !active {
            self.player_knowledge.sample_elapsed = 0.0;
            self.player_knowledge.dwell.clear();
            self.player_knowledge.visible_last_sample.clear();
            self.player_knowledge.target = None;
            self.player_knowledge.pending_first_hits.clear();
            for announcement in self.player_knowledge.health_announcements.values_mut() {
                announcement.visible = None;
            }
            return;
        }
        let dt = if dt.is_finite() {
            dt.clamp(0.0, 0.25)
        } else {
            0.0
        };
        if let Some((_, elapsed)) = &mut self.player_knowledge.last_hit {
            *elapsed += dt;
        }
        for announcement in self.player_knowledge.health_announcements.values_mut() {
            announcement.remaining = (announcement.remaining - dt).max(0.0);
        }
        // First-hit visibility uses this frame's actual camera, not a potentially
        // 100ms-old discovery sample. Hidden first hits are consumed without a cue.
        let live: BTreeSet<_> = self
            .actors
            .iter()
            .filter(|actor| actor.hp > 0.0)
            .map(|actor| actor.id)
            .collect();
        self.player_knowledge
            .health_announcements
            .retain(|id, _| live.contains(id));
        let pending = std::mem::take(&mut self.player_knowledge.pending_first_hits);
        if !pending.is_empty() {
            self.collision.refresh(world, geometry);
        }
        let direction = observation.direction.normalize();
        let right = direction.cross(Vec3::Y).normalize_or(Vec3::X);
        let up = right.cross(direction).normalize();
        for id in pending {
            let Some(actor) = self
                .actors
                .iter()
                .find(|actor| actor.id == id && actor.hp > 0.0)
            else {
                continue;
            };
            let rotation = actor.body_rotation().inverse();
            let diameter = (rotation * right)
                .abs()
                .dot(actor.dimensions)
                .max((rotation * up).abs().dot(actor.dimensions));
            let visible = [actor.center(), actor.eye()].into_iter().any(|point| {
                observation.contains(point, diameter)
                    && self.collision.sight_clear(observation.origin, point)
            });
            if visible {
                let pips = target_snapshot(actor).health_pips;
                self.player_knowledge
                    .health_announcements
                    .entry(id)
                    .or_insert(HealthAnnouncement {
                        shown_band: Some(pips),
                        remaining: 1.0,
                        visible: Some(EnemyHealthCueSnapshot {
                            actor_id: id,
                            position: actor.center() + Vec3::Y * (actor.dimensions.y * 0.5 + 0.35),
                            health_pips: pips,
                        }),
                    });
            }
        }
        // Refresh the handful of active announcements every rendered frame.
        // Landmark acquisition remains10Hz; moving bodies and occluders cannot
        // leave a live cue behind between those samples.
        if self
            .player_knowledge
            .health_announcements
            .values()
            .any(|cue| cue.remaining > 0.0)
        {
            self.collision.refresh(world, geometry);
        }
        for (id, announcement) in &mut self.player_knowledge.health_announcements {
            if announcement.remaining <= 0.0 {
                continue;
            }
            let Some(actor) = self.actors.iter().find(|actor| actor.id == *id) else {
                continue;
            };
            let rotation = actor.body_rotation().inverse();
            let diameter = (rotation * right)
                .abs()
                .dot(actor.dimensions)
                .max((rotation * up).abs().dot(actor.dimensions));
            let visible = [actor.center(), actor.eye()].into_iter().any(|point| {
                observation.contains(point, diameter)
                    && self.collision.sight_clear(observation.origin, point)
            });
            announcement.visible = visible.then(|| {
                let band = target_snapshot(actor).health_pips;
                if announcement.shown_band != Some(band) {
                    announcement.shown_band = Some(band);
                    announcement.remaining = 1.0;
                }
                EnemyHealthCueSnapshot {
                    actor_id: *id,
                    position: actor.center() + Vec3::Y * (actor.dimensions.y * 0.5 + 0.35),
                    health_pips: band,
                }
            });
        }
        self.player_knowledge.sample_elapsed += dt;
        if self.player_knowledge.sample_elapsed + 0.00001 < 0.1 {
            return;
        }
        // Never turn one stalled frame into several purported distinct sightings.
        self.player_knowledge.sample_elapsed =
            (self.player_knowledge.sample_elapsed - 0.1).max(0.0) % 0.1;
        self.collision.refresh(world, geometry);
        let team = player.map_or(0, |p| p.team);
        self.player_knowledge.health_announcements.retain(|id, _| {
            self.actors
                .iter()
                .any(|actor| actor.id == *id && actor.hp > 0.0)
        });
        for announcement in self.player_knowledge.health_announcements.values_mut() {
            announcement.visible = None;
        }
        let mut seen = BTreeSet::new();
        let mut visible = BTreeMap::new();
        let direction = observation.direction.normalize();
        let delta = direction * 1600.0;
        let right = direction.cross(Vec3::Y).normalize_or(Vec3::X);
        let up = right.cross(direction).normalize();
        let mut nearest = 1.0;
        let mut aimed = None;
        for actor in self.actors.iter().filter(|a| a.team != team) {
            let rotation = actor.body_rotation().inverse();
            let diameter = (rotation * right)
                .abs()
                .dot(actor.dimensions)
                .max((rotation * up).abs().dot(actor.dimensions));
            let sighted = [actor.center(), actor.eye()].into_iter().any(|point| {
                observation.contains(point, diameter)
                    && self.collision.sight_clear(observation.origin, point)
            });
            if !sighted {
                continue;
            }
            if let Some((id, kind)) = self.player_knowledge.actors.get(&actor.id).cloned() {
                let known = self.player_knowledge.landmarks.get(&id).cloned();
                let witnessed_death = actor.hp <= 0.0
                    && self
                        .player_knowledge
                        .visible_last_sample
                        .contains(&actor.id);
                // Corpses are not rendered. Only an already known encounter can
                // publish a witnessed disappearance, never discover a hidden corpse.
                let central_sighting =
                    observation.landmark_contains(actor.center(), diameter, kind)
                        && self
                            .collision
                            .sight_clear(observation.origin, actor.center());
                if central_sighting && (actor.hp > 0.0 || witnessed_death && known.is_some()) {
                    seen.insert(id.clone());
                    self.player_knowledge.admit(DiscoveredLandmark {
                        id,
                        kind,
                        position: actor.feet,
                        defeated: witnessed_death
                            || self.player_knowledge.credited_defeats.contains(&actor.id),
                        consumed: false,
                    });
                }
            }
            if actor.hp <= 0.0 {
                continue;
            }
            let target = target_snapshot(actor);
            visible.insert(actor.id, target);
            if let Some(announcement) = self
                .player_knowledge
                .health_announcements
                .get_mut(&actor.id)
            {
                if announcement.shown_band != Some(target.health_pips) {
                    announcement.shown_band = Some(target.health_pips);
                    announcement.remaining = 1.0;
                }
                announcement.visible = Some(EnemyHealthCueSnapshot {
                    actor_id: actor.id,
                    position: actor.center() + Vec3::Y * (actor.dimensions.y * 0.5 + 0.35),
                    health_pips: target.health_pips,
                });
            }
            if let Some(hit) =
                crate::shapes::sweep_actor(observation.origin, delta, actor, false, 0.0)
            {
                if hit.fraction < nearest {
                    nearest = hit.fraction;
                    aimed = Some(actor.id);
                }
            }
        }
        // A terrain/barrier ray cannot change the result when no visible living
        // actor intersects the reticle. Keep the original full-range query when
        // it can matter, including strict ties and projectile-only blockers.
        let aimed = aimed.filter(|_| {
            self.collision
                .attack_sweep(observation.origin, delta, 0.0)
                .is_none_or(|(hit, _)| nearest < hit.fraction)
        });
        if let Some(sites) = world
            .expedition
            .as_ref()
            .filter(|_| self.expedition_progress().is_some())
        {
            let states = self.expedition_progress();
            for (id, pool) in &sites.fountains {
                let mut tops = BTreeMap::new();
                for cell in &pool.cells {
                    tops.entry(cell.coord)
                        .and_modify(|top: &mut hex_core::TilePos| {
                            if cell.level > top.level {
                                *top = *cell;
                            }
                        })
                        .or_insert(*cell);
                }
                let points: Vec<_> = tops
                    .values()
                    .map(|at| at.coord.to_world(geometry.top(*at) + 0.01))
                    .collect();
                let Some(first) = points.first().copied() else {
                    continue;
                };
                let (min, max) = points
                    .iter()
                    .fold((first, first), |(min, max), p| (min.min(*p), max.max(*p)));
                let center = (min + max) * 0.5;
                let point = points
                    .iter()
                    .min_by(|a, b| {
                        a.distance_squared(center)
                            .total_cmp(&b.distance_squared(center))
                    })
                    .copied()
                    .unwrap_or(first);
                // A central cell-sized patch must itself be visible: the extent of
                // the whole pool cannot make a distant sliver count as observation.
                let sighted = observation.landmark_contains(point, 1.0, LandmarkKind::Fountain)
                    && self.collision.sight_clear(observation.origin, point);
                if sighted {
                    seen.insert(id.clone());
                    self.player_knowledge.admit(DiscoveredLandmark {
                        id: id.clone(),
                        kind: LandmarkKind::Fountain,
                        position: point,
                        defeated: false,
                        consumed: states
                            .as_ref()
                            .and_then(|s| s.fountains.iter().find(|f| &f.name == id))
                            .is_some_and(|f| f.consumed),
                    });
                }
            }
        }
        self.player_knowledge
            .dwell
            .retain(|id, _| seen.contains(id));
        self.player_knowledge.visible_last_sample = visible.keys().copied().collect();
        let fallback = self
            .player_knowledge
            .last_hit
            .filter(|(_, age)| *age <= 2.0)
            .map(|(id, _)| id);
        self.player_knowledge.target = aimed.or(fallback).and_then(|id| visible.get(&id).copied());
    }

    /// Remembered map markers only; no hidden positions or complete authored roster.
    #[must_use]
    pub fn discovered_landmarks(&self) -> Vec<DiscoveredLandmark> {
        self.player_knowledge.landmarks.values().cloned().collect()
    }

    /// Current player HUD facts. Calling this getter never advances discovery.
    #[must_use]
    pub fn combat_feedback(&self, tuning: &ArenaTuning) -> CombatFeedbackSnapshot {
        let actor = self
            .human_actor_id()
            .and_then(|id| self.actors.iter().find(|a| a.id == id));
        let tuning = self.player_tuning(tuning);
        let active = self.player_knowledge.active
            && !self.is_finished()
            && actor.is_some_and(|a| a.hp > 0.0);
        CombatFeedbackSnapshot {
            health_cues: if active {
                self.player_knowledge
                    .health_announcements
                    .values()
                    .filter(|announcement| announcement.remaining > 0.0)
                    .filter_map(|announcement| announcement.visible)
                    .collect()
            } else {
                Vec::new()
            },
            target: active.then_some(self.player_knowledge.target).flatten(),
            hit: (active
                && self
                    .player_knowledge
                    .last_hit
                    .is_some_and(|(_, age)| age <= 0.2))
            .then_some(HitConfirmationSnapshot {
                sequence: self.player_knowledge.hit_sequence,
            }),
            spells: [Spell::Shield, Spell::Fireball, Spell::HighJump].map(|spell| {
                let cooldown = actor
                    .and_then(|a| a.cooldowns.get(spell.index()))
                    .copied()
                    .unwrap_or(0.0);
                let charge = actor.and_then(Actor::charge);
                let charging = charge.filter(|c| c.spell == spell);
                let state = if !active
                    || spell == Spell::HighJump
                        && actor
                            .is_none_or(|a| !matches!(a.species, Species::Human | Species::Shadow))
                {
                    SpellAvailabilityState::Unavailable
                } else if charging.is_some() {
                    SpellAvailabilityState::Charging
                } else if cooldown > STEP * 0.01 {
                    SpellAvailabilityState::CoolingDown
                } else if spell != Spell::HighJump
                    && (charge.is_some() || actor.is_some_and(|a| a.cast_needs_release))
                {
                    SpellAvailabilityState::Unavailable
                } else {
                    SpellAvailabilityState::Ready
                };
                SpellAvailability {
                    spell,
                    state,
                    cooldown_remaining: cooldown.max(0.0),
                    cooldown_total: tuning.cooldown(spell),
                    charge_fraction: charging
                        .map_or(0.0, |c| (c.elapsed / tuning.charge_seconds).clamp(0.0, 1.0)),
                }
            }),
        }
    }

    pub(crate) fn confirm_player_damage(&mut self, owner: ActorId, victim: ActorId) {
        if self.human_actor_id() == Some(owner)
            && self
                .actors
                .iter()
                .find(|a| a.id == owner)
                .zip(self.actors.iter().find(|a| a.id == victim))
                .is_some_and(|(a, b)| a.team != b.team)
        {
            self.player_knowledge.hit_sequence =
                self.player_knowledge.hit_sequence.saturating_add(1);
            self.player_knowledge.last_hit = Some((victim, 0.0));
            if self.player_knowledge.active
                && !self
                    .player_knowledge
                    .health_announcements
                    .contains_key(&victim)
            {
                self.player_knowledge.pending_first_hits.insert(victim);
            }
        }
    }
}

impl PlayerObservation {
    fn landmark_contains(self, point: Vec3, diameter: f32, kind: LandmarkKind) -> bool {
        let (range, pixels) = match kind {
            LandmarkKind::Dragon => (120.0, 16.0),
            LandmarkKind::Shadow | LandmarkKind::Troll | LandmarkKind::Golem => (60.0, 16.0),
            LandmarkKind::Fountain => (35.0, 12.0),
        };
        let depth = (point - self.origin).dot(self.direction.normalize());
        self.origin.distance_squared(point) <= range * range
            && self.contains(point, diameter)
            && diameter * 1080.0 / (2.0 * depth * (self.vertical_fov * 0.5).tan()) >= pixels
    }

    fn valid(self) -> bool {
        self.active
            && self.origin.is_finite()
            && self.direction.is_finite()
            && self.direction.length_squared() > 0.0001
            && self.vertical_fov.is_finite()
            && (0.05..3.1).contains(&self.vertical_fov)
            && self.aspect.is_finite()
            && self.aspect > 0.0
            && self.viewport_height.is_finite()
            && self.viewport_height > 0.0
    }

    fn contains(self, point: Vec3, diameter: f32) -> bool {
        let direction = self.direction.normalize();
        let right = direction.cross(Vec3::Y).normalize_or(Vec3::X);
        let up = right.cross(direction).normalize();
        let relative = point - self.origin;
        let depth = relative.dot(direction);
        let tangent = (self.vertical_fov * 0.5).tan();
        depth > 0.035
            && depth <= 1600.0
            && relative.dot(up).abs() <= depth * tangent
            && relative.dot(right).abs() <= depth * tangent * self.aspect
            && diameter * self.viewport_height / (2.0 * depth * tangent) >= 8.0
    }
}

fn target_snapshot(actor: &Actor) -> TargetHealthSnapshot {
    let ratio = actor.hp / actor.max_hp;
    TargetHealthSnapshot {
        actor_id: actor.id,
        species: actor.species,
        role: actor.expedition_role(),
        health_pips: if ratio > 2.0 / 3.0 {
            3
        } else if ratio > 1.0 / 3.0 {
            2
        } else {
            1
        },
    }
}

#[cfg(test)]
mod tests;
