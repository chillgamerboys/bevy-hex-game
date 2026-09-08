//! Small creature controllers. Only this sensing facade reads live human state.

use super::*;
use crate::bot::ballistic_aim;
use crate::spells::{forecast_spell, ForecastBody};

#[derive(Debug, Clone, Copy)]
pub(super) struct Request {
    pub kind: CreatureAbility,
    pub aim: Vec3,
}
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct MotionIntent {
    pub input: ActorIntent,
    pub direction: Vec3,
    pub flight: bool,
}
#[derive(Debug)]
pub(super) struct Brain {
    pub active: Option<super::abilities::Cast>,
    pub cooldowns: [f32; 7],
    shadow: Bot,
    home: Vec3,
    seed: u32,
    steering: steering::Steering,
    shadow_travel: steering::ShadowTravel,
    retreat_goal: Option<Vec3>,
    retreat_reconsider: u64,
    flight_recovery: Option<(Vec3, u64)>,
    shooting_angle: bool,
    next_shot_probe: u64,
    pub decision: Option<CreatureDecisionSnapshot>,
    spell_gap: f32,
    error: Vec3,
    was_visible: bool,
    reaction_since: u64,
    battle_seen: Vec<targeting::ObservedTarget>,
    battle_sense_tick: Option<u64>,
    battle_target: Option<ActorId>,
}

impl Brain {
    pub fn new(id: u8, home: Vec3) -> Self {
        Self {
            active: None,
            cooldowns: [0.0; 7],
            shadow: Bot::default(),
            home,
            seed: 0x9175_BAFF ^ (u32::from(id) * 1973),
            steering: steering::Steering::default(),
            shadow_travel: steering::ShadowTravel::new(home),
            retreat_goal: None,
            retreat_reconsider: 0,
            flight_recovery: None,
            shooting_angle: false,
            next_shot_probe: 0,
            decision: None,
            spell_gap: 0.0,
            error: Vec3::ZERO,
            was_visible: false,
            reaction_since: 0,
            battle_seen: Vec::new(),
            battle_sense_tick: None,
            battle_target: None,
        }
    }
    pub fn for_battle(id: ActorId, home: Vec3, seed: u64) -> Self {
        let mut brain = Self::new(id, home);
        let mixed =
            seed ^ seed.rotate_left(29) ^ (u64::from(id) + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        brain.seed = u32::try_from(mixed & u64::from(u32::MAX))
            .unwrap_or(1)
            .max(1);
        brain.shadow = Bot::with_seed(brain.seed);
        brain
    }
    pub fn cancel_charge(&mut self) {
        self.shadow.cancel_charge();
        // One idle input tick clears the actor's post-pause release latch.
        self.spell_gap = STEP * 2.0;
    }
    fn random(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        f32::from(u16::try_from(self.seed & 65535).unwrap_or(0)) / 65535.0
    }
    pub fn ready(&self, kind: CreatureAbility) -> bool {
        self.cooldowns
            .get(super::abilities::index(kind))
            .is_some_and(|c| *c <= 0.0)
            && self.active.is_none()
    }

    pub fn intent(
        &mut self,
        actor: &Actor,
        party: &PartyRuntime,
        actors: &[Actor],
        shots: &[Projectile],
        cues: &[CombatCue],
        collision: &CollisionWorld,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        tuning: &ArenaTuning,
        tick: u64,
    ) -> (MotionIntent, Option<Request>) {
        let c = &tuning.encounters;
        self.spell_gap = (self.spell_gap - STEP).max(0.0);
        // Live human data is confined to visibility admission. Downstream plans
        // receive this copy or the party's dated observation, never a hidden pose.
        let battle = party.battle_search.is_some();
        let sight = if battle {
            if self
                .battle_sense_tick
                .is_none_or(|previous| tick.saturating_sub(previous) >= 12)
            {
                self.battle_seen = targeting::observe(
                    actor,
                    actors,
                    &self.battle_seen,
                    collision,
                    tick,
                    tuning.bot.prediction_seconds,
                );
                self.battle_sense_tick = Some(tick);
            }
            self.battle_seen
                .iter()
                .copied()
                .find(|target| collision.sight_clear(actor.eye(), target.sight_point))
                .map(|target| Knowledge {
                    point: target.body.feet,
                    velocity: target.body.velocity,
                    tick: target.tick,
                    direct: true,
                    cue_kind: None,
                    observed: Some(target),
                })
        } else {
            actors
                .iter()
                .find(|a| a.id == 0 && a.hp > 0.0)
                .filter(|target| {
                    party.snapshot.phase != PartyPhase::Dormant
                        && (party.snapshot.phase != PartyPhase::Returning
                            || target.center().distance(actor.eye()) <= 6.0)
                        && [target.eye(), target.center()]
                            .into_iter()
                            .any(|p| collision.sight_clear(actor.eye(), p))
                })
                .map(|target| Knowledge {
                    point: target.feet,
                    velocity: party
                        .knowledge
                        .filter(|k| k.direct)
                        .map_or(Vec3::ZERO, |k| k.velocity),
                    tick,
                    direct: true,
                    cue_kind: None,
                    observed: None,
                })
        };
        let target_id = sight.and_then(|s| s.observed.map(|o| o.body.id));
        if battle && target_id.is_some() && target_id != self.battle_target {
            let replacing = self.battle_target.is_some();
            self.battle_target = target_id;
            self.reaction_since = tick;
            self.error = Vec3::ZERO;
            if replacing && actor.charge().is_some() && actor.species != Species::Shadow {
                self.cancel_charge();
                return (
                    MotionIntent {
                        input: ActorIntent {
                            aim: actor.aim,
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                    None,
                );
            }
        }
        if sight.is_some() && !self.was_visible {
            self.reaction_since = tick;
        }
        self.was_visible = sight.is_some();
        let known = sight.or_else(|| {
            (party.snapshot.phase == PartyPhase::Active)
                .then_some(party.knowledge)
                .flatten()
        });
        let threatened = shots.iter().any(|s| {
            s.spell == Spell::Fireball
                && s.source_team() != actor.team
                && s.position.distance(actor.center()) < 9.0
                && (actor.center() - s.position).dot(s.velocity) > 0.0
                && collision.sight_clear(actor.eye(), s.position)
        });
        let hurt = actor
            .last_damage_tick
            .is_some_and(|t| elapsed(tick, t) < 1.0);
        if actor.species == Species::Shadow && party.snapshot.phase == PartyPhase::Active {
            let input = if let Some(search) = party.battle_search {
                self.shadow.intent_battle(
                    actor.id, search, actors, shots, collision, world, geometry, tuning, cues, tick,
                )
            } else {
                self.shadow.intent_for(
                    actor.id, actors, shots, collision, world, geometry, tuning, cues, tick,
                )
            };
            let aim = input.aim.normalize_or(actor.aim);
            let f = aim.with_y(0.0).normalize_or(Vec3::NEG_Z);
            return (
                MotionIntent {
                    direction: f * input.movement.y + f.cross(Vec3::Y) * input.movement.x,
                    input,
                    flight: false,
                },
                None,
            );
        }
        let mut input = ActorIntent {
            aim: actor.aim,
            ..Default::default()
        };
        let mut request = None;
        let mut flight = false;
        let target = known.map(|k| {
            k.observed
                .map_or(k.point + Vec3::Y * 0.4, targeting::ObservedTarget::center)
        });
        if let Some(target) = target {
            input.aim = (target - actor.eye()).normalize_or(actor.aim);
        }
        let retreat = actor.species == Species::Dragon
            && actor.last_damage_tick.is_some_and(|t| {
                elapsed(tick, t)
                    < if actor.hp < actor.max_hp * 0.5 {
                        c.dragon_hurt_retreat_seconds
                    } else {
                        c.dragon_retreat_seconds
                    }
            });
        let mut goal = match party.snapshot.phase {
            PartyPhase::Returning => self.home,
            PartyPhase::Dormant => {
                self.home
                    + Vec3::new(
                        (f32::from(actor.id) * 1.7).sin(),
                        0.0,
                        (f32::from(actor.id) * 1.7).cos(),
                    ) * 1.5
            }
            _ => target.unwrap_or(party.battle_search.unwrap_or(self.home)),
        };
        if !retreat {
            self.retreat_goal = None;
        }
        if actor.species == Species::Dragon {
            flight = party.snapshot.phase == PartyPhase::Dormant
                || (party.snapshot.phase == PartyPhase::Active && sight.is_none())
                || retreat;
            if retreat {
                self.flight_recovery = None;
                let invalid = self.retreat_goal.is_none_or(|point| {
                    steering::flight_goal(actor, point, collision, world, geometry, c)
                        .is_none_or(|valid| valid.distance(point) > 0.5)
                });
                let reached = self
                    .retreat_goal
                    .is_some_and(|point| actor.feet.distance(point) < 0.6);
                if invalid || (reached && tick >= self.retreat_reconsider) {
                    self.retreat_goal = steering::retreat_goal(
                        actor,
                        target.unwrap_or(actor.feet - actor.aim),
                        collision,
                        world,
                        geometry,
                        c,
                    );
                    self.retreat_reconsider = tick + 120;
                }
                goal = self
                    .retreat_goal
                    .or_else(|| {
                        steering::flight_goal(actor, self.home, collision, world, geometry, c)
                    })
                    .unwrap_or(actor.feet);
            } else {
                let recovery_done = self.flight_recovery.is_some_and(|(point, until)| {
                    tick >= until || actor.feet.distance(point) < 0.4
                });
                if recovery_done {
                    self.flight_recovery = None;
                }
                if sight.is_some()
                    && self.steering.blocked_ticks(tick) >= 48
                    && self.flight_recovery.is_none()
                {
                    if let Some(target) = target {
                        let away = (actor.center() - target).with_y(0.0).normalize_or(Vec3::Z);
                        let approach =
                            target + away * (actor.dimensions.z * 0.5 + c.bite_range * 0.6);
                        self.flight_recovery =
                            steering::flight_goal(actor, approach, collision, world, geometry, c)
                                .map(|point| (point, tick + 240));
                    }
                }
                if let Some((point, _)) = self.flight_recovery {
                    flight = true;
                    goal = point;
                } else if flight {
                    goal = steering::flight_goal(actor, goal, collision, world, geometry, c)
                        .or_else(|| {
                            steering::flight_goal(actor, self.home, collision, world, geometry, c)
                        })
                        .unwrap_or(actor.feet);
                }
            }
            if (hurt || threatened) && target.is_some() && self.ready(CreatureAbility::Barrier) {
                request = Some(Request {
                    kind: CreatureAbility::Barrier,
                    aim: input.aim,
                });
            } else if sight.is_some() {
                let distance = sight.and_then(|seen| seen.observed).map_or_else(
                    || actor.eye().distance(target.unwrap_or(actor.eye())),
                    |seen| seen.distance(actor.eye(), 0.0),
                );
                let facing = (actor.body_rotation() * Vec3::NEG_Z)
                    .dot(input.aim.with_y(0.0).normalize_or(Vec3::NEG_Z));
                // A retreating dragon may stop to turn its physical mouth toward
                // a visible close attacker. Damage still refreshes the retreat
                // timer, and the fixed retreat destination survives this defense.
                let turn_distance = sight.and_then(|seen| seen.observed).map_or_else(
                    || actor.center().distance(target.unwrap_or(actor.center())),
                    |seen| seen.distance(actor.center(), 0.0),
                );
                let mouth_offset = actor.eye().distance(actor.center());
                if retreat
                    && turn_distance <= c.breath_range + mouth_offset
                    && self.ready(CreatureAbility::FireCone)
                {
                    goal = actor.feet;
                }
                if !retreat
                    && distance <= c.bite_range + 0.2
                    && facing >= (c.bite_angle.to_radians() * 0.5).cos()
                    && self.ready(CreatureAbility::Bite)
                {
                    request = Some(Request {
                        kind: CreatureAbility::Bite,
                        aim: input.aim,
                    });
                } else if distance <= c.breath_range
                    && facing >= (c.breath_angle.to_radians() * 0.5).cos()
                    && self.ready(CreatureAbility::FireCone)
                {
                    request = Some(Request {
                        kind: CreatureAbility::FireCone,
                        aim: input.aim,
                    });
                }
                if !retreat
                    && distance < c.bite_range * 0.7
                    && facing >= (c.bite_angle.to_radians() * 0.5).cos()
                    && self.flight_recovery.is_none()
                {
                    goal = actor.feet;
                }
            }
        } else if actor.species == Species::Goblin {
            if let Some(target) = target {
                let toward = (target - actor.center()).with_y(0.0).normalize_or_zero();
                goal += toward.cross(Vec3::Y) * (f32::from(actor.id % 3) - 1.0) * 0.65;
                let distance = sight.and_then(|seen| seen.observed).map_or_else(
                    || actor.eye().distance(target),
                    |seen| seen.distance(actor.eye(), 0.0),
                );
                if sight.is_some()
                    && distance <= c.swipe_range + 0.2
                    && self.ready(CreatureAbility::Swipe)
                {
                    request = Some(Request {
                        kind: CreatureAbility::Swipe,
                        aim: input.aim,
                    });
                }
                if distance < c.swipe_range * 0.6 {
                    goal = actor.feet;
                }
            }
        } else if actor.species == Species::Shaman {
            if tick >= self.next_shot_probe || sight.is_none() {
                self.shooting_angle = sight.is_some_and(|seen| {
                    self.shot_aim_at_speed(
                        actor,
                        seen,
                        collision,
                        world,
                        geometry,
                        tuning,
                        tuning.launch_speed(c.shaman_charge.min(tuning.charge_seconds)),
                        Vec3::ZERO,
                    )
                    .is_some()
                });
                self.next_shot_probe = tick + 24 + u64::from(actor.id % 4);
            }
            if let Some(target) = target {
                // Memory supplies a search destination, not proof of a firing lane.
                // An obstructed shot seeks a new angle instead of parking at 8u.
                if self.shooting_angle && sight.is_some() {
                    let away = (actor.center() - target).with_y(0.0).normalize_or(Vec3::Z);
                    goal = target + away * 8.0;
                    let frontline: Vec<_> = actors
                        .iter()
                        .filter(|ally| {
                            ally.id != actor.id
                                && ally.hp > 0.0
                                && ally.party == actor.party
                                && matches!(ally.species, Species::Goblin | Species::Dragon)
                        })
                        .collect();
                    if !frontline.is_empty() {
                        let count = f32::from(u8::try_from(frontline.len()).unwrap_or(24));
                        let center = frontline.iter().map(|ally| ally.feet).sum::<Vec3>() / count;
                        // Stay behind the fighters, leaving room inside the
                        // existing aura for their lateral melee movement.
                        goal = center + away * (c.aura_radius * 0.75);
                    }
                } else if sight.is_some() && actor.center().distance(target) < 5.0 {
                    goal = actor.feet
                        + (actor.center() - target).with_y(0.0).normalize_or(Vec3::Z) * 4.0;
                }
            }
            let eligible: Vec<_> = actors
                .iter()
                .filter(|a| {
                    a.id != actor.id
                        && a.hp > 0.0
                        && a.party == actor.party
                        && a.center().distance(actor.center()) <= c.aura_radius
                        && collision.sight_clear(actor.eye(), a.center())
                })
                .collect();
            if actor.charge().is_none()
                && self.ready(CreatureAbility::Aura)
                && (eligible.iter().any(|a| a.hp < a.max_hp - 0.1)
                    || (party.snapshot.phase == PartyPhase::Active
                        && eligible
                            .iter()
                            .filter(|ally| {
                                let attacking = ally.attack_state().is_some_and(|attack| {
                                    matches!(
                                        attack.kind,
                                        CreatureAbility::Swipe
                                            | CreatureAbility::Bite
                                            | CreatureAbility::FireCone
                                    )
                                }) || shots.iter().any(|shot| {
                                    shot.owner == ally.id
                                        && shot.spell == Spell::Fireball
                                        && shot.age < 1.0
                                });
                                let reach = match ally.species {
                                    Species::Goblin => c.swipe_range + 0.2,
                                    Species::Dragon => c.breath_range,
                                    _ => 0.0,
                                };
                                let engaged = sight.is_some_and(|seen| {
                                    let point = seen
                                        .observed
                                        .map_or(seen.point + Vec3::Y * 0.4, |o| o.sight_point);
                                    let distance = seen.observed.map_or_else(
                                        || ally.eye().distance(point),
                                        |o| o.distance(ally.eye(), 0.0),
                                    );
                                    distance <= reach && collision.sight_clear(ally.eye(), point)
                                });
                                attacking || engaged
                            })
                            .take(2)
                            .count()
                            >= 2))
            {
                request = Some(Request {
                    kind: CreatureAbility::Aura,
                    aim: input.aim,
                });
            }
            if actor.charge().is_some() {
                input.cast_held = true;
                input.selected = Some(Spell::Fireball);
                if actor.charge().is_some_and(|charge| {
                    charge.elapsed >= c.shaman_charge.min(tuning.charge_seconds)
                }) {
                    if let Some(observation) = sight {
                        if let Some(aim) =
                            self.shot_aim(actor, observation, collision, world, geometry, tuning)
                        {
                            input.aim = aim;
                            input.cast_held = false;
                            input.cast_released = true;
                            self.spell_gap = c.shaman_reaction;
                        } else {
                            input.cast_held = false;
                        }
                    } else {
                        input.cast_held = false;
                    }
                }
            } else if self.active.is_none() && self.spell_gap <= 0.0 && request.is_none() {
                if (hurt || threatened) && actor.cooldowns.first().is_some_and(|cd| *cd <= 0.0) {
                    let direction = input.aim.with_y(0.0).normalize_or(Vec3::NEG_Z);
                    let mut caster = actor.clone();
                    caster.selected = Spell::Shield;
                    caster.aim = (direction - Vec3::Y * 0.45).normalize();
                    let preview = forecast_spell(
                        &caster,
                        &[],
                        collision,
                        world,
                        geometry,
                        tuning,
                        tuning.launch_speed(0.0),
                    );
                    if !preview.wall_voxels.is_empty() {
                        input.selected = Some(Spell::Shield);
                        input.aim = caster.aim;
                        input.cast_pressed = true;
                        input.cast_released = true;
                        self.spell_gap = c.shaman_reaction;
                    }
                }
                if !input.cast_pressed
                    && sight.is_some()
                    && elapsed(tick, self.reaction_since) >= c.shaman_reaction
                    && actor.cooldowns.get(1).is_some_and(|cd| *cd <= 0.0)
                {
                    let spread = tuning.bot.aim_error * c.shaman_spread_multiplier;
                    self.error = Vec3::new(
                        (self.random() * 2.0 - 1.0) * spread,
                        (self.random() * 2.0 - 1.0) * spread * 0.6,
                        (self.random() * 2.0 - 1.0) * spread,
                    );
                    input.selected = Some(Spell::Fireball);
                    input.cast_pressed = true;
                    input.cast_held = true;
                }
            }
        }
        if party.snapshot.phase == PartyPhase::Returning {
            goal = self.home.with_y(if flight { goal.y } else { self.home.y });
            if actor.species == Species::Shadow {
                if sight.is_some() {
                    input = self.shadow.intent_for(
                        actor.id, actors, shots, collision, world, geometry, tuning, cues, tick,
                    );
                } else {
                    self.shadow.cancel_charge();
                    input = ActorIntent {
                        aim: actor.aim,
                        ..Default::default()
                    };
                }
            }
        }
        let desired = goal - actor.feet;
        let moving = desired.with_y(0.0).length()
            > if party.snapshot.phase == PartyPhase::Dormant {
                0.6
            } else {
                0.35
            };
        let desired = if flight {
            desired.clamp_length_max(1.0)
        } else if moving {
            desired.with_y(0.0).normalize_or_zero()
        } else {
            Vec3::ZERO
        };
        if let Some(active) = &self.active {
            // Only admitted own sight can update a breath. Other attacks and a
            // breath whose target is hidden retain their last direction.
            if !active.tracks_breath() || sight.is_none() {
                input.aim = active.direction();
            }
        }
        input.run = party.snapshot.phase != PartyPhase::Dormant || self.steering.jumping();
        let (direction, jump) = if actor.species == Species::Shadow {
            (
                self.shadow_travel.travel(
                    actor,
                    desired,
                    flight,
                    self.active.is_some() || request.is_some() || input.cast_released,
                    collision,
                    world,
                    geometry,
                    c,
                    tick,
                ),
                false,
            )
        } else if self.steering.jumping() {
            // Finish the already validated landing before starting a stationary
            // windup; pausing midair would discard the route's safety proof.
            request = None;
            input.cast_pressed = false;
            input.cast_released = false;
            input.cast_held = actor.charge().is_some();
            self.steering.travel(
                actor, desired, false, input.run, collision, world, geometry, c, tick,
            )
        } else if self.active.is_some() || request.is_some() || input.cast_released {
            self.steering.hold(actor, tick);
            (Vec3::ZERO, false)
        } else {
            self.steering.travel(
                actor, desired, flight, input.run, collision, world, geometry, c, tick,
            )
        };
        input.jump = jump;
        self.decision = Some(CreatureDecisionSnapshot {
            id: actor.id,
            target: sight.and_then(|s| s.observed.map(|o| o.body.id)),
            observation_tick: known.map(|k| k.tick),
            own_sight: sight.is_some(),
            goal: goal.to_array(),
            direction: direction.to_array(),
            flying: flight,
            retreat_seconds: if retreat {
                let duration = if actor.hp < actor.max_hp * 0.5 {
                    c.dragon_hurt_retreat_seconds
                } else {
                    c.dragon_retreat_seconds
                };
                actor
                    .last_damage_tick
                    .map_or(0.0, |since| (duration - elapsed(tick, since)).max(0.0))
            } else {
                0.0
            },
            jump_recovery: self.steering.jumping(),
            blocked_ticks: self.steering.blocked_ticks(tick),
            useful_shot: self.shooting_angle,
        });
        (
            MotionIntent {
                input,
                direction,
                flight,
            },
            request,
        )
    }

    fn shot_aim(
        &self,
        actor: &Actor,
        seen: Knowledge,
        collision: &CollisionWorld,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        tuning: &ArenaTuning,
    ) -> Option<Vec3> {
        self.shot_aim_at_speed(
            actor,
            seen,
            collision,
            world,
            geometry,
            tuning,
            tuning.launch_speed(actor.charge()?.elapsed),
            self.error,
        )
    }

    fn shot_aim_at_speed(
        &self,
        actor: &Actor,
        seen: Knowledge,
        collision: &CollisionWorld,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        tuning: &ArenaTuning,
        speed: f32,
        error: Vec3,
    ) -> Option<Vec3> {
        let target = seen.observed.map_or(
            seen.point + Vec3::Y * 0.4,
            targeting::ObservedTarget::center,
        );
        let (_, time) = ballistic_aim(actor.eye(), target, tuning, speed)?;
        let (aim, _) = ballistic_aim(
            actor.eye(),
            target + seen.velocity * time.min(0.5) + error,
            tuning,
            speed,
        )?;
        let mut caster = actor.clone();
        caster.selected = Spell::Fireball;
        caster.aim = aim;
        let bodies: Vec<_> = if seen.observed.is_some() {
            self.battle_seen
                .iter()
                .filter(|seen| collision.sight_clear(actor.eye(), seen.sight_point))
                .map(|seen| seen.body)
                .collect()
        } else {
            vec![ForecastBody::human(0, seen.point, seen.velocity, 0.5)]
        };
        let forecast = forecast_spell(&caster, &bodies, collision, world, geometry, tuning, speed);
        let impact = forecast.impact?;
        (shapes::distance(impact.point, actor) > tuning.fireball_radius() + 0.2
            && seen.observed.is_none_or(|target| {
                target.distance(impact.point, impact.time) <= tuning.fireball_radius() * 0.8
            }))
        .then_some(caster.aim)
    }
}
