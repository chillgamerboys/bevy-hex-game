//! Observation-limited Strong opponent with ordinary charged casts and local peeks.

use bevy_math::{Vec2, Vec3};
use hex_core::arena::{ArenaTerrainView, ArenaVoxelGeometry};
use serde::Serialize;

use crate::collision::{CollisionWorld, SKIN};
use crate::spells::{
    capsule_distance, forecast_spell, ForecastBody, EMERGENCE_SECONDS, MAX_FLIGHT_SECONDS,
};
use crate::{
    Actor, ActorIntent, ArenaTuning, CombatCue, CombatCueKind, Projectile, Spell, BODY_HEIGHT,
    BODY_RADIUS, STEP,
};

mod navigation;
use navigation::Route;

/// Read-only evidence for bot intent and bounded planning work.
#[derive(Debug, Clone, Serialize)]
pub struct BotDebugSnapshot {
    /// Current decision mode.
    pub mode: &'static str,
    /// Last position established by actual sight.
    pub last_seen: Option<[f32; 3]>,
    /// Seconds since the last sighting.
    pub memory_age: Option<f32>,
    /// Last audible, already-coarsened combat position.
    pub cue_position: Option<[f32; 3]>,
    /// Whether the remembered cue came from a release or an impact.
    pub cue_kind: Option<&'static str>,
    /// Whether the bot is maintaining an ordinary Fireball hold.
    pub charging: bool,
    /// Number of remaining local movement waypoints.
    pub route_len: usize,
    /// Committed direction around cover, or zero before a route.
    pub flank_side: i8,
    /// Admitted speculative releases during this round.
    pub blind_shots: u32,
    /// Uncertainty around the currently remembered target, in world units.
    pub uncertainty: f32,
    /// Charge duration currently planned, in simulation seconds.
    pub planned_charge: f32,
    /// Reason for the most recent release or cancellation.
    pub last_release_reason: &'static str,
    /// Number of five-Hz decisions made.
    pub decisions: u64,
    /// Number of shared swept spell forecasts requested.
    pub forecasts: u64,
    /// Total simulated body ticks used by local route planning.
    pub route_rollout_ticks: u64,
    /// Body ticks spent checking immediate walking and sprinting support.
    pub movement_rollout_ticks: u64,
}

#[derive(Debug, Clone, Copy)]
struct Memory {
    feet: Vec3,
    velocity: Vec3,
    tick: u64,
}

#[derive(Debug, Clone, Copy)]
struct CueMemory {
    position: Vec3,
    tick: u64,
    kind: CombatCueKind,
}

#[derive(Debug, Clone, Copy)]
struct Belief {
    feet: Vec3,
    velocity: Vec3,
    visible: bool,
    uncertainty: f32,
    profile: Option<crate::targeting::ObservedTarget>,
}

impl Belief {
    fn center(self) -> Vec3 {
        self.feet
            + Vec3::Y
                * (self
                    .profile
                    .map_or(BODY_HEIGHT, |target| target.body.dimensions.y)
                    * 0.5)
    }

    fn distance(self, point: Vec3, time: f32, prediction: f32) -> f32 {
        self.profile.map_or_else(
            || capsule_distance(point, self.feet + self.velocity * time.min(prediction)),
            |mut target| {
                target.body.feet = self.feet;
                target.body.velocity = self.velocity;
                target.distance(point, time)
            },
        )
    }
}

#[derive(Debug)]
struct BattlePerception {
    search: Vec3,
    seen: Vec<crate::targeting::ObservedTarget>,
    target: Option<crate::targeting::ObservedTarget>,
}

#[derive(Debug, Clone, Copy)]
struct Threat {
    position: Vec3,
    velocity: Vec3,
    arrival: f32,
}

/// Only the sensing facade may construct an observation from live opponent state.
#[derive(Debug, Default, Clone, Copy)]
struct Observation {
    target: Option<Memory>,
    threat: Option<Threat>,
}

#[derive(Debug)]
pub(super) struct Bot {
    seed: u32,
    think_ticks: u16,
    release_ticks: u16,
    strafe_ticks: u16,
    strafe: f32,
    travel: Vec3,
    aim: Option<Vec3>,
    last_hp: f32,
    charging_fireball: bool,
    memory: Option<Memory>,
    cue: Option<CueMemory>,
    last_cue_id: Option<u64>,
    sense_ticks: u16,
    observation: Observation,
    loss_tick: Option<u64>,
    blind_used: bool,
    tick: u64,
    route: Route,
    ambush_until: u64,
    ambush_episode: Option<u64>,
    dodge_until: u64,
    dodge: Vec3,
    pending_tap: Option<(Spell, Vec3, bool)>,
    run: bool,
    error: Vec3,
    mode: &'static str,
    last_release_reason: &'static str,
    planned_charge: f32,
    uncertainty: f32,
    decisions: u64,
    forecasts: u64,
    route_rollout_ticks: u64,
    blind_shots: u32,
    safe_at: u64,
    safe_direction: Vec3,
    safe_requested: Vec3,
    safe_revision: Option<u64>,
    movement_rollout_ticks: u64,
    safe_expected: Option<(Vec3, Vec3)>,
    safe_run: bool,
    battle: Option<BattlePerception>,
}

impl Default for Bot {
    fn default() -> Self {
        Self {
            seed: 0x6A09_E667,
            think_ticks: 0,
            release_ticks: 0,
            strafe_ticks: 0,
            strafe: 1.0,
            travel: Vec3::ZERO,
            aim: None,
            last_hp: 100.0,
            charging_fireball: false,
            memory: None,
            cue: None,
            last_cue_id: None,
            sense_ticks: 0,
            observation: Observation::default(),
            loss_tick: None,
            blind_used: false,
            tick: 0,
            route: Route::default(),
            ambush_until: 0,
            ambush_episode: None,
            dodge_until: 0,
            dodge: Vec3::ZERO,
            pending_tap: None,
            run: false,
            error: Vec3::ZERO,
            mode: "search",
            last_release_reason: "none",
            planned_charge: 0.0,
            uncertainty: 0.0,
            decisions: 0,
            forecasts: 0,
            route_rollout_ticks: 0,
            blind_shots: 0,
            safe_at: 0,
            safe_direction: Vec3::ZERO,
            safe_requested: Vec3::ZERO,
            safe_revision: None,
            movement_rollout_ticks: 0,
            safe_expected: None,
            safe_run: false,
            battle: None,
        }
    }
}

impl Bot {
    pub(crate) fn with_seed(seed: u32) -> Self {
        Self {
            seed: seed.max(1),
            ..Self::default()
        }
    }

    pub(crate) fn intent_battle(
        &mut self,
        id: u8,
        search: Vec3,
        actors: &[Actor],
        projectiles: &[Projectile],
        collision: &CollisionWorld,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        tuning: &ArenaTuning,
        cues: &[CombatCue],
        tick: u64,
    ) -> ActorIntent {
        self.battle
            .get_or_insert(BattlePerception {
                search,
                seen: Vec::new(),
                target: None,
            })
            .search = search;
        self.intent_for(
            id,
            actors,
            projectiles,
            collision,
            world,
            geometry,
            tuning,
            cues,
            tick,
        )
    }

    fn random(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        f32::from(u16::try_from(self.seed & 0xffff).unwrap_or(0)) / 65535.0
    }

    pub(super) fn debug(&self) -> BotDebugSnapshot {
        BotDebugSnapshot {
            mode: self.mode,
            last_seen: self.memory.map(|m| m.feet.to_array()),
            memory_age: self.memory.map(|m| age(self.tick, m.tick)),
            cue_position: self.cue.map(|c| c.position.to_array()),
            cue_kind: self.cue.map(|c| match c.kind {
                CombatCueKind::Release => "release",
                CombatCueKind::Impact => "impact",
            }),
            charging: self.charging_fireball,
            route_len: self.route.points.len(),
            flank_side: self.route.side,
            blind_shots: self.blind_shots,
            uncertainty: self.uncertainty,
            planned_charge: self.planned_charge,
            last_release_reason: self.last_release_reason,
            decisions: self.decisions,
            forecasts: self.forecasts,
            route_rollout_ticks: self.route_rollout_ticks,
            movement_rollout_ticks: self.movement_rollout_ticks,
        }
    }

    /// Cancel the plan without reseeding or spending a spell cooldown.
    pub(super) fn cancel_charge(&mut self) {
        self.charging_fireball = false;
        self.pending_tap = None;
        self.ambush_until = 0;
        self.last_release_reason = "cancelled";
    }

    pub(super) fn intent(
        &mut self,
        actors: &[Actor],
        projectiles: &[Projectile],
        collision: &CollisionWorld,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        tuning: &ArenaTuning,
        cues: &[CombatCue],
        tick: u64,
    ) -> ActorIntent {
        self.intent_for(
            1,
            actors,
            projectiles,
            collision,
            world,
            geometry,
            tuning,
            cues,
            tick,
        )
    }

    pub(crate) fn intent_for(
        &mut self,
        id: u8,
        actors: &[Actor],
        projectiles: &[Projectile],
        collision: &CollisionWorld,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        tuning: &ArenaTuning,
        cues: &[CombatCue],
        tick: u64,
    ) -> ActorIntent {
        let Some(bot) = actors.iter().find(|a| a.id == id && a.hp > 0.0) else {
            self.cancel_charge();
            return ActorIntent::default();
        };
        self.tick = tick;
        self.think_ticks = self.think_ticks.saturating_sub(1);
        self.sense_ticks = self.sense_ticks.saturating_sub(1);
        self.release_ticks = self.release_ticks.saturating_sub(1);
        self.strafe_ticks = self.strafe_ticks.saturating_sub(1);
        if self.charging_fireball && bot.charge().is_none_or(|c| c.spell != Spell::Fireball) {
            self.charging_fireball = false;
        }
        if self.sense_ticks == 0 || self.pending_tap.is_some() {
            self.sense_ticks = 12;
            // This facade alone reads live opponents. A failed LOS query copies
            // no position, velocity, aim, charge, or cooldown into the brain.
            let mut switched = false;
            let target = if let Some(battle) = &mut self.battle {
                battle.seen = crate::targeting::observe(
                    bot,
                    actors,
                    &battle.seen,
                    collision,
                    tick,
                    tuning.bot.prediction_seconds,
                );
                let selected = battle.seen.first().copied();
                if let Some(selected) = selected {
                    switched = battle
                        .target
                        .is_some_and(|old| old.body.id != selected.body.id);
                    battle.target = Some(selected);
                }
                selected.map(|seen| Memory {
                    feet: seen.body.feet,
                    velocity: seen.body.velocity,
                    tick,
                })
            } else {
                actors
                    .iter()
                    .find(|a| a.id == 0 && a.hp > 0.0)
                    .filter(|a| {
                        [a.center(), a.eye()]
                            .into_iter()
                            .any(|p| visible_from(collision, bot.eye(), p))
                    })
                    .map(|a| {
                        let velocity = self
                            .memory
                            .filter(|m| age(tick, m.tick) <= 0.21 && tick > m.tick)
                            .map_or(Vec3::ZERO, |m| {
                                ((a.feet - m.feet) / age(tick, m.tick)).clamp_length_max(9.0)
                            });
                        Memory {
                            feet: a.feet,
                            velocity,
                            tick,
                        }
                    })
            };
            if switched {
                self.memory = None;
                self.cue = None;
                self.loss_tick = None;
                self.blind_used = false;
                self.observation.target = None;
                self.route.clear();
                self.cancel_charge();
                self.aim = None;
                self.think_ticks = 0;
            }
            let threat = projectiles
                .iter()
                .filter(|s| s.owner != bot.id && s.spell == Spell::Fireball)
                .filter(|s| s.source_team() != bot.team)
                .filter(|s| {
                    s.position.distance(bot.center()) < 18.0
                        && visible_from(collision, bot.eye(), s.position)
                })
                .filter_map(|s| {
                    let toward = bot.center() - s.position;
                    let speed = s.velocity.length();
                    let closing = s
                        .velocity
                        .normalize_or_zero()
                        .dot(toward.normalize_or_zero());
                    (speed > SKIN && closing > 0.75).then_some(Threat {
                        position: s.position,
                        velocity: s.velocity,
                        arrival: toward.length() / speed,
                    })
                })
                .min_by(|a, b| a.arrival.total_cmp(&b.arrival));
            for cue in cues {
                if self.last_cue_id.is_some_and(|old| cue.id <= old) {
                    continue;
                }
                self.last_cue_id = Some(cue.id);
                if cue.owner != bot.id
                    && cue.team != bot.team
                    && age(tick, cue.tick) <= 1.0
                    && cue.position.distance(bot.center()) <= tuning.bot.cue_radius
                {
                    self.cue = Some(CueMemory {
                        position: cue.position,
                        tick: cue.tick,
                        kind: cue.kind,
                    });
                }
            }
            if target.is_some() && self.observation.target.is_none() {
                self.think_ticks = 0;
                self.ambush_until = 0;
            }
            if target.is_some() {
                self.memory = target;
                self.loss_tick = None;
                self.blind_used = false;
            } else if self.observation.target.is_some() {
                self.loss_tick = Some(tick);
                self.blind_used = false;
            }
            self.observation = Observation { target, threat };
            if switched && bot.charge().is_some() {
                return ActorIntent {
                    aim: bot.aim,
                    ..Default::default()
                };
            }
            if let Some(threat) = threat {
                self.ambush_until = 0;
                let side = threat
                    .velocity
                    .with_y(0.0)
                    .normalize_or_zero()
                    .cross(Vec3::Y)
                    * self.strafe;
                self.dodge = (side
                    + (bot.center() - threat.position)
                        .with_y(0.0)
                        .normalize_or_zero()
                        * 0.25)
                    .normalize_or_zero();
                self.dodge_until = self.tick + 54;
            }
        }
        let mut action = ActorIntent::default();
        if let Some((spell, aim, needs_threat)) = self.pending_tap.take() {
            if ready(bot, spell) {
                let belief = self.belief(self.observation.target.is_some(), collision, tuning);
                let aim = match spell {
                    Spell::AreaBlast => self
                        .observation
                        .target
                        .filter(|target| {
                            self.battle.as_ref().and_then(|b| b.target).map_or_else(
                                || capsule_distance(bot.center(), target.feet),
                                |target| target.distance(bot.center(), 0.0),
                            ) < tuning.blast_radius() * 0.8
                        })
                        .map(|_| aim),
                    Spell::Shield if !needs_threat || self.observation.threat.is_some() => self
                        .shield_aim(
                            bot,
                            aim.with_y(0.0).normalize_or_zero(),
                            belief,
                            self.observation.threat,
                            collision,
                            world,
                            geometry,
                            tuning,
                        ),
                    _ => None,
                };
                if let Some(aim) = aim {
                    self.aim = Some(aim);
                    action = self.tap(spell, tuning);
                }
            }
        } else if self.think_ticks == 0 {
            self.think_ticks = 24;
            action = self.decide(bot, self.observation, collision, world, geometry, tuning);
        }
        if self.charging_fireball && !action.cast_released {
            action.cast_held = true;
        }
        if action.cast_released {
            self.travel = Vec3::ZERO;
        }
        let aim = self.aim.unwrap_or(bot.aim);
        let forward = aim.with_y(0.0).normalize_or(Vec3::NEG_Z);
        let displaced = self.safe_expected.is_some_and(|(feet, impulse)| {
            feet.distance(bot.feet) > 0.03 || impulse.distance(bot.impulse_velocity()) > 0.1
        });
        if displaced {
            self.route.clear();
        }
        self.route.invalidate(bot, collision);
        let route = if action.cast_released || tick < self.ambush_until {
            self.route.hold(bot, tick);
            None
        } else if tick < self.dodge_until {
            self.route.clear();
            None
        } else {
            self.route.travel(bot, tick, &tuning.bot)
        };
        let travel = if action.cast_released || tick < self.ambush_until {
            Vec3::ZERO
        } else if tick < self.dodge_until {
            self.dodge
        } else {
            route.unwrap_or(self.travel)
        };
        let run = (self.run || tick < self.dodge_until) && route.is_none();
        if self.safe_at == 0
            || tick.saturating_sub(self.safe_at) >= 12
            || displaced
            || self.safe_run != run
            || self.safe_revision != collision.revision
            || travel.dot(self.safe_requested) < 0.75
        {
            let (direction, work) = safe_travel(bot, travel, collision, run);
            self.safe_direction = direction;
            self.safe_requested = travel;
            self.safe_at = tick;
            self.safe_revision = collision.revision;
            self.safe_run = run;
            self.movement_rollout_ticks += work;
        }
        let travel = if travel.length_squared() < SKIN {
            Vec3::ZERO
        } else {
            self.safe_direction
        };
        if route.is_some_and(|planned| planned.distance(travel) > 0.05) {
            self.route.clear();
        }
        let mut next_body = bot.body.clone();
        let mut next_feet = bot.feet;
        next_body.tick(&mut next_feet, travel, run, false, collision);
        self.movement_rollout_ticks += 1;
        self.safe_expected = Some((
            next_feet,
            next_body.impulse_velocity + Vec3::Y * next_body.vertical_velocity,
        ));
        action.movement = Vec2::new(travel.dot(forward.cross(Vec3::Y)), travel.dot(forward));
        action.aim = aim;
        action.run = run;
        action
    }

    fn belief(
        &self,
        visible: bool,
        collision: &CollisionWorld,
        tuning: &ArenaTuning,
    ) -> Option<Belief> {
        let memory = self
            .memory
            .filter(|m| age(self.tick, m.tick) <= tuning.bot.memory_seconds);
        let cue = self
            .cue
            .filter(|c| age(self.tick, c.tick) <= tuning.bot.cue_memory_seconds);
        if let Some(m) = memory.filter(|m| visible || cue.is_none_or(|c| c.tick <= m.tick)) {
            let elapsed = age(self.tick, m.tick);
            return Some(Belief {
                feet: m.feet
                    + if visible {
                        Vec3::ZERO
                    } else {
                        m.velocity * elapsed.min(tuning.bot.prediction_seconds)
                    },
                velocity: if visible { m.velocity } else { Vec3::ZERO },
                visible,
                uncertainty: if visible { 0.1 } else { 0.4 + elapsed * 1.5 },
                profile: self.battle.as_ref().and_then(|b| b.target),
            });
        }
        cue.map(|c| {
            let feet = collision
                .ground(c.position + Vec3::Y * 0.8, BODY_HEIGHT, BODY_RADIUS, 3.0)
                .unwrap_or(c.position - Vec3::Y * (BODY_HEIGHT * 0.5));
            Belief {
                feet,
                velocity: Vec3::ZERO,
                visible: false,
                uncertainty: 1.5,
                profile: None,
            }
        })
    }

    fn decide(
        &mut self,
        bot: &Actor,
        observation: Observation,
        collision: &CollisionWorld,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        tuning: &ArenaTuning,
    ) -> ActorIntent {
        self.decisions += 1;
        let visible = observation.target.is_some();
        if self
            .memory
            .is_some_and(|m| age(self.tick, m.tick) > tuning.bot.memory_seconds)
        {
            self.memory = None;
        }
        if self
            .cue
            .is_some_and(|c| age(self.tick, c.tick) > tuning.bot.cue_memory_seconds)
        {
            self.cue = None;
        }
        let belief = self.belief(visible, collision, tuning);
        self.uncertainty = belief.map_or(0.0, |b| b.uncertainty);
        let hurt = bot.hp + 0.1 < self.last_hp;
        self.last_hp = bot.hp;
        if self.strafe_ticks == 0 {
            self.strafe_ticks = 120;
            self.strafe = if self.random() < 0.5 { -1.0 } else { 1.0 };
            if belief.is_none() {
                self.travel = Vec3::new(self.random() * 2.0 - 1.0, 0.0, self.random() * 2.0 - 1.0)
                    .normalize_or_zero();
            }
        }
        self.run = false;
        if let Some(threat) = observation.threat {
            self.ambush_until = 0;
            let side = threat
                .velocity
                .with_y(0.0)
                .normalize_or_zero()
                .cross(Vec3::Y)
                * self.strafe;
            self.dodge = (side
                + (bot.center() - threat.position)
                    .with_y(0.0)
                    .normalize_or_zero()
                    * 0.25)
                .normalize_or_zero();
            self.dodge_until = self.tick + 54;
        }
        // A visible incoming projectile can justify defense even when its
        // owner has never been seen and there is no target memory.
        if let Some(threat) = observation.threat {
            if self.release_ticks == 0 && ready(bot, Spell::Shield) {
                let direction = (threat.position - bot.center())
                    .with_y(0.0)
                    .normalize_or_zero();
                if let Some(aim) = self.shield_aim(
                    bot,
                    direction,
                    belief,
                    Some(threat),
                    collision,
                    world,
                    geometry,
                    tuning,
                ) {
                    return self.replace_with_tap(bot, Spell::Shield, aim, tuning);
                }
            }
        }
        let Some(belief) = belief else {
            self.mode = "search";
            self.route.clear();
            self.ambush_until = 0;
            self.cancel_charge();
            if let Some(battle) = &self.battle {
                self.travel = (battle.search - bot.feet).with_y(0.0).normalize_or_zero();
                self.run = true;
            }
            return ActorIntent::default();
        };
        let toward = (belief.center() - bot.center())
            .with_y(0.0)
            .normalize_or_zero();
        let distance = bot.center().distance(belief.center());
        if visible {
            self.route.clear();
            self.ambush_until = 0;
            self.ambush_episode = None;
            let approach = if distance > tuning.bot.preferred_distance {
                0.9
            } else if distance
                < tuning
                    .bot
                    .minimum_distance
                    .max(tuning.fireball_radius() + 0.8)
            {
                -1.0
            } else {
                0.0
            };
            self.travel =
                (toward * approach + toward.cross(Vec3::Y) * self.strafe * 0.7).normalize_or_zero();
            self.run = distance > tuning.bot.sprint_distance || observation.threat.is_some();
            self.mode = "engage";
        } else {
            self.travel = toward;
            self.mode = "prepare";
            if tuning.bot.flank_enabled && !visible_from(collision, bot.eye(), belief.center()) {
                let side = if self.strafe < 0.0 { -1 } else { 1 };
                self.route_rollout_ticks += self.route.plan(
                    bot,
                    belief.center(),
                    collision,
                    self.tick,
                    &tuning.bot,
                    side,
                );
                if !self.route.points.is_empty() {
                    self.mode = "flank";
                }
            } else {
                self.route.clear();
            }
            if self.charging_fireball
                && !self.route.points.is_empty()
                && self.ambush_episode != self.memory.map(|m| m.tick)
            {
                self.ambush_episode = self.memory.map(|m| m.tick);
                if self.random() < tuning.bot.ambush_chance {
                    let duration = tuning.bot.ambush_min_seconds
                        + self.random()
                            * (tuning.bot.ambush_max_seconds - tuning.bot.ambush_min_seconds);
                    self.ambush_until = self.tick + u64::from(ticks(duration));
                }
            }
            if self.tick < self.ambush_until {
                self.mode = "ambush";
            }
        }
        self.aim = Some((belief.center() - bot.eye()).normalize_or(bot.aim));
        // Close defense and visible projectile danger are independent of seeing
        // the projectile owner. Changing a held spell uses a neutral tick first.
        let close_blast =
            visible && distance < tuning.blast_radius() * 0.8 && ready(bot, Spell::AreaBlast);
        let defensive = distance > 5.0 && hurt && ready(bot, Spell::Shield);
        if self.release_ticks == 0 && close_blast {
            return self.replace_with_tap(bot, Spell::AreaBlast, bot.aim, tuning);
        }
        if self.release_ticks == 0 && defensive {
            if let Some(aim) = self.shield_aim(
                bot,
                toward,
                Some(belief),
                None,
                collision,
                world,
                geometry,
                tuning,
            ) {
                return self.replace_with_tap(bot, Spell::Shield, aim, tuning);
            }
        }
        if self.release_ticks > 0 {
            return ActorIntent::default();
        }
        let current = bot
            .charge()
            .filter(|c| c.spell == Spell::Fireball)
            .map_or(0.0, |c| c.elapsed);
        let armed = bot.charge().is_some_and(|c| c.spell == Spell::Fireball);
        if !armed && (!ready(bot, Spell::Fireball) || distance <= tuning.fireball_radius() + 0.8) {
            return ActorIntent::default();
        }
        if !armed {
            self.error = Vec3::new(self.random() * 2.0 - 1.0, 0.0, self.random() * 2.0 - 1.0)
                * tuning.bot.aim_error;
        }
        // Hearing can prepare a hold and guide searching, but can never create
        // a firing budget. Speculative aiming uses the last DIRECT sight only.
        let blind = !belief.visible;
        let shot_belief = if !blind {
            Some(belief)
        } else {
            self.memory
                .filter(|m| {
                    tuning.bot.blind_fire_enabled
                        && self.loss_tick.is_some()
                        && !self.blind_used
                        && age(self.tick, m.tick) <= tuning.bot.blind_fire_max_age
                })
                .map(|m| {
                    let elapsed = age(self.tick, m.tick);
                    Belief {
                        feet: m.feet + m.velocity * elapsed.min(tuning.bot.prediction_seconds),
                        velocity: Vec3::ZERO,
                        visible: false,
                        uncertainty: 0.4 + elapsed * 1.5,
                        profile: self.battle.as_ref().and_then(|b| b.target),
                    }
                })
        };
        let choice = shot_belief.and_then(|target| {
            self.best_shot(bot, target, current, collision, world, geometry, tuning)
        });
        self.planned_charge = choice.map_or(tuning.charge_seconds, |(_, elapsed, _)| elapsed);
        if let Some((aim, elapsed, _)) = choice {
            self.aim = Some(aim);
            if elapsed <= current + STEP * 0.01 && self.tick >= self.ambush_until {
                self.charging_fireball = false;
                self.last_release_reason = if blind {
                    "limited blind splash"
                } else {
                    "observed hit or splash"
                };
                self.release_ticks = ticks(tuning.bot.reaction_seconds);
                if blind {
                    self.blind_used = true;
                    self.blind_shots += 1;
                }
                return ActorIntent {
                    selected: Some(Spell::Fireball),
                    cast_pressed: !armed,
                    cast_released: true,
                    ..Default::default()
                };
            }
        }
        self.charging_fireball = true;
        if !armed {
            return ActorIntent {
                selected: Some(Spell::Fireball),
                cast_pressed: true,
                cast_held: true,
                ..Default::default()
            };
        }
        ActorIntent::default()
    }

    fn tap(&mut self, spell: Spell, tuning: &ArenaTuning) -> ActorIntent {
        self.charging_fireball = false;
        self.release_ticks = ticks(tuning.bot.reaction_seconds);
        self.last_release_reason = if spell == Spell::Shield {
            "defensive shield"
        } else {
            "close blast"
        };
        ActorIntent {
            selected: Some(spell),
            cast_pressed: true,
            cast_released: true,
            ..Default::default()
        }
    }

    fn replace_with_tap(
        &mut self,
        bot: &Actor,
        spell: Spell,
        aim: Vec3,
        tuning: &ArenaTuning,
    ) -> ActorIntent {
        self.ambush_until = 0;
        if bot.charge().is_some() {
            self.charging_fireball = false;
            self.pending_tap = Some((spell, aim, self.observation.threat.is_some()));
            self.last_release_reason = "interrupt charge";
            ActorIntent::default()
        } else {
            self.aim = Some(aim);
            self.tap(spell, tuning)
        }
    }

    fn shield_aim(
        &mut self,
        bot: &Actor,
        direction: Vec3,
        belief: Option<Belief>,
        threat: Option<Threat>,
        collision: &CollisionWorld,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        tuning: &ArenaTuning,
    ) -> Option<Vec3> {
        let speed = tuning.launch_speed(0.0);
        let observations = belief.map_or_else(Vec::new, |b| self.forecast_bodies(b, tuning));
        for offset in [4.5, 6.0] {
            let landing = bot.feet + direction * offset + Vec3::Y * 0.06;
            let Some((aim, _)) = ballistic_aim(bot.eye(), landing, tuning, speed) else {
                continue;
            };
            let mut caster = bot.clone();
            caster.aim = aim;
            caster.selected = Spell::Shield;
            self.forecasts += 1;
            let forecast = forecast_spell(
                &caster,
                &observations,
                collision,
                world,
                geometry,
                tuning,
                speed,
            );
            if forecast.wall_voxels.len() >= 3
                && forecast.impact.is_some_and(|i| {
                    let enough_time = threat.is_none_or(|t| {
                        let normal = direction.with_y(0.0).normalize_or_zero();
                        let closing = t.velocity.dot(normal);
                        if closing >= -SKIN {
                            return false;
                        }
                        let time = (i.point - t.position).dot(normal) / closing;
                        let crossing = t.position + t.velocity * time
                            - Vec3::Y * (0.5 * tuning.projectile_gravity * time * time);
                        time > i.time + EMERGENCE_SECONDS + STEP
                            && forecast.wall_voxels.iter().any(|pos| {
                                crate::collision::voxel_overlaps_body(
                                    *pos,
                                    geometry,
                                    crossing - Vec3::Y * 0.06,
                                    0.12,
                                    0.06,
                                )
                            })
                    });
                    enough_time
                        && capsule_distance(i.point, bot.feet) > tuning.fireball_radius() + 0.2
                })
            {
                return Some(aim);
            }
        }
        None
    }

    fn best_shot(
        &mut self,
        bot: &Actor,
        belief: Belief,
        current: f32,
        collision: &CollisionWorld,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        tuning: &ArenaTuning,
    ) -> Option<(Vec3, f32, f32)> {
        let mut best: Option<(Vec3, f32, f32)> = None;
        let mut examined = Vec::new();
        for elapsed in [
            0.0,
            tuning.charge_seconds * 0.25,
            tuning.charge_seconds * 0.5,
            tuning.charge_seconds * 0.75,
            tuning.charge_seconds,
            current,
        ] {
            let elapsed = elapsed.max(current);
            if examined
                .iter()
                .any(|e: &f32| (*e - elapsed).abs() < STEP * 0.01)
            {
                continue;
            }
            examined.push(elapsed);
            if let Some((aim, time)) = self.fireball_aim(
                bot,
                belief,
                collision,
                world,
                geometry,
                tuning,
                tuning.launch_speed(elapsed),
            ) {
                let cost = (elapsed - current) + time;
                if best.is_none_or(|(_, old_elapsed, old_time)| {
                    cost < old_elapsed - current + old_time
                }) {
                    best = Some((aim, elapsed, time));
                }
            }
        }
        best
    }

    fn fireball_aim(
        &mut self,
        bot: &Actor,
        belief: Belief,
        collision: &CollisionWorld,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        tuning: &ArenaTuning,
        speed: f32,
    ) -> Option<(Vec3, f32)> {
        if bot.center().distance(belief.center()) <= tuning.fireball_radius() + 0.8 {
            return None;
        }
        let observations = self.forecast_bodies(belief, tuning);
        let (_, lead_time) = ballistic_aim(bot.eye(), belief.center(), tuning, speed)?;
        let lead = belief.velocity * lead_time.min(tuning.bot.prediction_seconds);
        for vertical in [
            belief
                .profile
                .map_or(BODY_HEIGHT, |target| target.body.dimensions.y)
                * 0.5,
            0.06,
        ] {
            let uncertainty_error = if belief.visible {
                Vec3::ZERO
            } else {
                self.error.normalize_or_zero() * belief.uncertainty.min(1.0)
            };
            let target = belief.feet + Vec3::Y * vertical + lead + self.error + uncertainty_error;
            let Some((aim, _)) = ballistic_aim(bot.eye(), target, tuning, speed) else {
                continue;
            };
            let mut caster = bot.clone();
            caster.aim = aim;
            caster.selected = Spell::Fireball;
            self.forecasts += 1;
            let forecast = forecast_spell(
                &caster,
                &observations,
                collision,
                world,
                geometry,
                tuning,
                speed,
            );
            let Some(impact) = forecast.impact else {
                continue;
            };
            let target_distance =
                belief.distance(impact.point, impact.time, tuning.bot.prediction_seconds);
            if capsule_distance(impact.point, bot.feet) > tuning.fireball_radius() + 0.5
                && target_distance <= tuning.fireball_radius() * 0.6
                && (belief.visible || (impact.actor.is_none() && impact.barrier.is_none()))
            {
                return Some((aim, impact.time));
            }
        }
        None
    }

    fn forecast_bodies(&self, belief: Belief, tuning: &ArenaTuning) -> Vec<ForecastBody> {
        if let Some(battle) = &self.battle {
            if belief.visible {
                battle.seen.iter().map(|seen| seen.body).collect()
            } else {
                Vec::new()
            }
        } else {
            forecast_bodies(belief, tuning)
        }
    }
}

fn forecast_bodies(belief: Belief, tuning: &ArenaTuning) -> Vec<ForecastBody> {
    // A memory/cue is an aiming hypothesis, never a phantom collision body.
    if belief.visible {
        vec![ForecastBody::human(
            0,
            belief.feet,
            belief.velocity,
            tuning.bot.prediction_seconds,
        )]
    } else {
        Vec::new()
    }
}

fn age(now: u64, then: u64) -> f32 {
    f32::from(u16::try_from(now.saturating_sub(then)).unwrap_or(u16::MAX)) * STEP
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "validated finite durations are clamped to the u16 tick budget"
)]
fn ticks(seconds: f32) -> u16 {
    (seconds / STEP).ceil().clamp(0.0, f32::from(u16::MAX)) as u16
}

fn ready(bot: &Actor, spell: Spell) -> bool {
    bot.cooldowns.get(spell.index()).is_some_and(|c| *c <= STEP)
}

fn visible_from(collision: &CollisionWorld, origin: Vec3, target: Vec3) -> bool {
    collision.sight_clear(origin, target)
}

fn safe_travel(bot: &Actor, desired: Vec3, collision: &CollisionWorld, run: bool) -> (Vec3, u64) {
    if desired.length_squared() < SKIN || !bot.grounded {
        return (desired, 0);
    }
    let side = desired.cross(Vec3::Y);
    let mut work = 0;
    for direction in [desired, side, -side, -desired] {
        let mut body = bot.body.clone();
        let mut feet = bot.feet;
        let mut supported = true;
        for _ in 0..12 {
            body.tick(&mut feet, direction, run, false, collision);
            work += 1;
            if !collision.clear(feet, BODY_HEIGHT, BODY_RADIUS)
                || collision
                    .ground(
                        feet + Vec3::Y * (SKIN * 2.0),
                        BODY_HEIGHT,
                        BODY_RADIUS,
                        0.42,
                    )
                    .is_none()
            {
                supported = false;
                break;
            }
        }
        if supported && (feet - bot.feet).with_y(0.0).length() > 0.02 {
            return (direction, work);
        }
    }
    (Vec3::ZERO, work)
}

/// Low ballistic arc at the shared launch speed, including vertical targets.
pub(crate) fn ballistic_aim(
    origin: Vec3,
    target: Vec3,
    tuning: &ArenaTuning,
    speed: f32,
) -> Option<(Vec3, f32)> {
    let delta = target - origin;
    let gravity = tuning.projectile_gravity;
    if !delta.is_finite() || delta.length_squared() < SKIN * SKIN || speed <= 0.0 || gravity <= 0.0
    {
        return None;
    }
    let b = speed * speed - gravity * delta.y;
    let discriminant = b * b - gravity * gravity * delta.length_squared();
    if discriminant < 0.0 {
        return None;
    }
    let denominator = b + discriminant.sqrt();
    if denominator <= 0.0 {
        return None;
    }
    let time = (2.0 * delta.length_squared() / denominator).sqrt();
    if !time.is_finite() || time > MAX_FLIGHT_SECONDS || time <= 0.0 {
        return None;
    }
    let aim = (delta + Vec3::Y * (0.5 * gravity * time * time)) / (speed * time);
    aim.is_finite().then_some((aim.normalize_or_zero(), time))
}

#[cfg(test)]
mod tests;
