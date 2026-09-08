//! Small creature controllers. Only this sensing facade reads live human state.

use super::*;
use crate::bot::ballistic_aim;
use crate::spells::{forecast_spell, ForecastBody};
use bevy_math::Quat;

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
    travel: Vec3,
    decided: u64,
    revision: Option<u64>,
    last_feet: Vec3,
    spell_gap: f32,
    error: Vec3,
    was_visible: bool,
    reaction_since: u64,
}

impl Brain {
    pub fn new(id: u8, home: Vec3) -> Self {
        Self {
            active: None,
            cooldowns: [0.0; 7],
            shadow: Bot::default(),
            home,
            seed: 0x9175_BAFF ^ (u32::from(id) * 1973),
            travel: Vec3::ZERO,
            decided: 0,
            revision: None,
            last_feet: home,
            spell_gap: 0.0,
            error: Vec3::ZERO,
            was_visible: false,
            reaction_since: 0,
        }
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
        let sight = actors
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
            });
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
                && actors
                    .iter()
                    .find(|a| a.id == s.owner)
                    .is_some_and(|a| a.team != actor.team)
                && s.position.distance(actor.center()) < 9.0
                && (actor.center() - s.position).dot(s.velocity) > 0.0
                && collision.sight_clear(actor.eye(), s.position)
        });
        let hurt = actor
            .last_damage_tick
            .is_some_and(|t| elapsed(tick, t) < 1.0);
        if actor.species == Species::Shadow && party.snapshot.phase == PartyPhase::Active {
            let input = self.shadow.intent_for(
                actor.id, actors, shots, collision, world, geometry, tuning, cues, tick,
            );
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
        let target = known.map(|k| k.point + Vec3::Y * 0.4);
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
            _ => target.unwrap_or(self.home),
        };
        if actor.species == Species::Dragon {
            flight = party.snapshot.phase == PartyPhase::Dormant
                || (party.snapshot.phase == PartyPhase::Active && sight.is_none())
                || retreat;
            if retreat {
                if let Some(target) = target {
                    goal = actor.feet
                        + (actor.center() - target).with_y(0.0).normalize_or(Vec3::Z) * 5.0;
                }
            }
            if flight {
                let probe = actor.feet + Vec3::Y * 4.0;
                let floor =
                    shapes::ground(collision, actor, probe, 12.0).map_or(self.home.y, |p| p.y);
                goal.y = floor + c.dragon_cruise_height;
            }
            if (hurt || threatened) && target.is_some() && self.ready(CreatureAbility::Barrier) {
                request = Some(Request {
                    kind: CreatureAbility::Barrier,
                    aim: input.aim,
                });
            } else if !retreat && sight.is_some() {
                let distance = actor.eye().distance(target.unwrap_or(actor.eye()));
                let facing = (actor.body_rotation() * Vec3::NEG_Z)
                    .dot(input.aim.with_y(0.0).normalize_or(Vec3::NEG_Z));
                if distance <= c.bite_range + 0.2
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
                if distance < c.bite_range * 0.7 {
                    goal = actor.feet;
                }
            }
        } else if actor.species == Species::Goblin {
            if let Some(target) = target {
                let toward = (target - actor.center()).with_y(0.0).normalize_or_zero();
                goal += toward.cross(Vec3::Y) * (f32::from(actor.id % 3) - 1.0) * 0.65;
                if sight.is_some()
                    && actor.eye().distance(target) <= c.swipe_range + 0.2
                    && self.ready(CreatureAbility::Swipe)
                {
                    request = Some(Request {
                        kind: CreatureAbility::Swipe,
                        aim: input.aim,
                    });
                }
                if actor.eye().distance(target) < c.swipe_range * 0.6 {
                    goal = actor.feet;
                }
            }
        } else if actor.species == Species::Shaman {
            if let Some(target) = target {
                goal = target + (actor.center() - target).with_y(0.0).normalize_or(Vec3::Z) * 8.0;
                if actor.center().distance(target) < 5.0 {
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
                    || (party.snapshot.phase == PartyPhase::Active && eligible.len() >= 2))
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
        let revise = self.revision != collision.revision
            || tick.saturating_sub(self.decided) >= 24
            || actor.feet.distance(self.last_feet) > 2.0;
        if revise {
            self.travel = steer(actor, desired, flight, collision, world, geometry, c);
            self.decided = tick + u64::from(actor.id % 4);
            self.revision = collision.revision;
            self.last_feet = actor.feet;
        }
        if let Some(active) = &self.active {
            input.aim = active.direction();
        }
        if self.active.is_some() || request.is_some() || input.cast_released {
            self.travel = Vec3::ZERO;
        }
        input.run = party.snapshot.phase != PartyPhase::Dormant;
        (
            MotionIntent {
                input,
                direction: self.travel,
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
        let speed = tuning.launch_speed(actor.charge()?.elapsed);
        let target = seen.point + Vec3::Y * 0.4;
        let (_, time) = ballistic_aim(actor.eye(), target, tuning, speed)?;
        let (aim, _) = ballistic_aim(
            actor.eye(),
            target + seen.velocity * time.min(0.5),
            tuning,
            speed,
        )?;
        let mut caster = actor.clone();
        caster.selected = Spell::Fireball;
        caster.aim = (aim + self.error).normalize_or(aim);
        let forecast = forecast_spell(
            &caster,
            &[ForecastBody::human(0, seen.point, seen.velocity, 0.5)],
            collision,
            world,
            geometry,
            tuning,
            speed,
        );
        let impact = forecast.impact?;
        (shapes::distance(impact.point, actor) > tuning.fireball_radius() + 0.2)
            .then_some(caster.aim)
    }
}

fn steer(
    actor: &Actor,
    desired: Vec3,
    flight: bool,
    world: &CollisionWorld,
    view: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    tuning: &EncounterTuning,
) -> Vec3 {
    if desired.length_squared() < 0.001 {
        return Vec3::ZERO;
    }
    // Four short production-controller rollouts. This preserves stacked support,
    // exact step and turn limits without introducing a coordinate-only nav graph.
    let mut best = (f32::NEG_INFINITY, Vec3::ZERO);
    for angle in [0.0, 0.65, -0.65, 1.3, -1.3] {
        let direction = Quat::from_rotation_y(angle) * desired;
        let mut body = actor.clone();
        let mut safe = true;
        for _ in 0..12 {
            motion::tick(&mut body, direction, true, false, flight, world, tuning);
            if !shapes::clear(world, &body, body.feet, body.body_yaw)
                || (!flight && (!dry(&body, view, geometry) || body.feet.y < actor.feet.y - 0.45))
            {
                safe = false;
                break;
            }
        }
        if safe {
            let moved = body.feet - actor.feet;
            let score = moved.dot(desired.normalize_or_zero()) - 0.05 * angle.abs();
            if moved.length() > 0.02 && score > best.0 {
                best = (score, direction);
            }
        }
    }
    best.1
}
