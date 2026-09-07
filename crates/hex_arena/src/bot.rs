//! Small reactive opponent: visible targets, three spell choices, and local strafing.

use bevy_math::{Vec2, Vec3};
use hex_core::arena::{ArenaTerrainView, ArenaVoxelGeometry};

use crate::collision::{CollisionWorld, SKIN};
use crate::spells::{preview_actor, MAX_FLIGHT_SECONDS};
use crate::{Actor, ActorIntent, ArenaTuning, Projectile, Spell, BODY_HEIGHT, BODY_RADIUS, STEP};

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
}

impl Default for Bot {
    fn default() -> Self {
        Self {
            seed: 0x6A09_E667,
            think_ticks: 0,
            release_ticks: 60,
            strafe_ticks: 0,
            strafe: 1.0,
            travel: Vec3::ZERO,
            aim: None,
            last_hp: 100.0,
        }
    }
}

impl Bot {
    fn random(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        f32::from(u16::try_from(self.seed & 0xffff).unwrap_or(0)) / 65535.0
    }

    pub(super) fn intent(
        &mut self,
        actors: &[Actor],
        projectiles: &[Projectile],
        collision: &CollisionWorld,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        tuning: &ArenaTuning,
    ) -> ActorIntent {
        let (Some(human), Some(bot)) = (
            actors.iter().find(|a| a.id == 0 && a.hp > 0.0),
            actors.iter().find(|a| a.id == 1 && a.hp > 0.0),
        ) else {
            return ActorIntent::default();
        };
        self.think_ticks = self.think_ticks.saturating_sub(1);
        self.release_ticks = self.release_ticks.saturating_sub(1);
        self.strafe_ticks = self.strafe_ticks.saturating_sub(1);
        let mut selected = None;
        if self.think_ticks == 0 {
            self.think_ticks = 24; // Five decisions/second, not perfect per-tick tracking.
            let visible = [human.center(), human.eye()]
                .into_iter()
                .any(|point| visible_from(collision, bot.eye(), point));
            if self.strafe_ticks == 0 {
                self.strafe_ticks = 120;
                self.strafe = if self.random() < 0.5 { -1.0 } else { 1.0 };
                self.travel = Vec3::new(self.random() * 2.0 - 1.0, 0.0, self.random() * 2.0 - 1.0)
                    .normalize_or_zero();
            }
            let hurt = bot.hp + 0.1 < self.last_hp;
            self.last_hp = bot.hp;
            if visible {
                let toward = (human.center() - bot.center())
                    .with_y(0.0)
                    .normalize_or_zero();
                let distance = bot.center().distance(human.center());
                let approach = if distance > 12.0 {
                    0.9
                } else if distance < tuning.fireball_radius() + 2.0 {
                    -1.0
                } else {
                    0.0
                };
                self.travel = (toward * approach + toward.cross(Vec3::Y) * self.strafe * 0.7)
                    .normalize_or_zero();
                self.aim = Some((human.center() - bot.eye()).normalize_or_zero());
                if self.release_ticks == 0 {
                    let ready = |spell: Spell| {
                        bot.cooldowns
                            .get(spell.index())
                            .is_some_and(|seconds| *seconds <= STEP)
                    };
                    if distance < tuning.blast_radius() * 0.8 && ready(Spell::AreaBlast) {
                        selected = Some(Spell::AreaBlast);
                    } else {
                        let threatened = projectiles.iter().any(|shot| {
                            let to_bot = bot.center() - shot.position;
                            shot.owner != bot.id
                                && shot.spell == Spell::Fireball
                                && to_bot.length() < 14.0
                                && shot
                                    .velocity
                                    .normalize_or_zero()
                                    .dot(to_bot.normalize_or_zero())
                                    > 0.92
                                && visible_from(collision, bot.eye(), shot.position)
                        });
                        if distance > 5.0
                            && (hurt || bot.hp <= 55.0 || threatened)
                            && ready(Spell::Shield)
                        {
                            for offset in [3.0, 4.5] {
                                let landing = bot.feet + toward * offset + Vec3::Y * 0.06;
                                if let Some((aim, _)) = ballistic_aim(bot.eye(), landing, tuning) {
                                    let mut caster = bot.clone();
                                    caster.aim = aim;
                                    caster.selected = Spell::Shield;
                                    if preview_actor(
                                        &caster, actors, collision, world, geometry, tuning,
                                    )
                                    .valid
                                    {
                                        self.aim = Some(aim);
                                        selected = Some(Spell::Shield);
                                        break;
                                    }
                                }
                            }
                        }
                        if selected.is_none()
                            && distance > tuning.fireball_radius() + 0.8
                            && ready(Spell::Fireball)
                        {
                            let target = human.center();
                            if let Some((_, time)) = ballistic_aim(bot.eye(), target, tuning) {
                                // One small lead estimate; no intercept search or knowledge behind cover.
                                let velocity = ((human.feet - human.previous_feet) / STEP)
                                    .with_y(0.0)
                                    .clamp_length_max(7.0);
                                let noise =
                                    Vec3::new(self.random() - 0.5, 0.0, self.random() - 0.5) * 0.32;
                                let target = target + velocity * time.min(0.6) + noise;
                                if let Some((aim, _)) = ballistic_aim(bot.eye(), target, tuning) {
                                    let mut caster = bot.clone();
                                    caster.aim = aim;
                                    caster.selected = Spell::Fireball;
                                    let forecast = preview_actor(
                                        &caster, actors, collision, world, geometry, tuning,
                                    );
                                    let safe = forecast.impact.is_none_or(|point| {
                                        point.distance(bot.center())
                                            > tuning.fireball_radius() + 0.5
                                    });
                                    if safe {
                                        self.aim = Some(aim);
                                        selected = Some(Spell::Fireball);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        if selected.is_some() {
            self.release_ticks = 42; // Brief shared reaction gap between different spells.
        }
        let aim = self.aim.unwrap_or(bot.aim);
        let forward = aim.with_y(0.0).normalize_or(Vec3::NEG_Z);
        // Keep the current pose on a cast tick so the checked launch/footprint is
        // not invalidated by our own voluntary movement. Impulses still apply.
        let travel = if selected.is_some() {
            Vec3::ZERO
        } else {
            safe_travel(bot, self.travel, collision)
        };
        ActorIntent {
            movement: Vec2::new(travel.dot(forward.cross(Vec3::Y)), travel.dot(forward)),
            aim,
            cast: selected.is_some(),
            selected,
            ..Default::default()
        }
    }
}

fn visible_from(collision: &CollisionWorld, origin: Vec3, target: Vec3) -> bool {
    collision
        .sweep_sphere(origin, target - origin, 0.0)
        .is_none()
}

fn safe_travel(bot: &Actor, desired: Vec3, collision: &CollisionWorld) -> Vec3 {
    if !bot.grounded {
        return desired;
    }
    let side = desired.cross(Vec3::Y);
    [desired, side, -side, -desired]
        .into_iter()
        .find(|travel| {
            let raised = bot.feet + Vec3::Y * (0.4 + SKIN * 2.0);
            collision
                .sweep(raised, *travel * 0.8, BODY_HEIGHT, BODY_RADIUS)
                .is_none()
                && collision
                    .ground(raised + *travel * 0.8, BODY_HEIGHT, BODY_RADIUS, 1.3)
                    .is_some()
        })
        .unwrap_or(Vec3::ZERO)
}

/// Low ballistic arc at the real launch speed, including vertical targets.
fn ballistic_aim(origin: Vec3, target: Vec3, tuning: &ArenaTuning) -> Option<(Vec3, f32)> {
    let delta = target - origin;
    let speed = tuning.projectile_speed;
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
