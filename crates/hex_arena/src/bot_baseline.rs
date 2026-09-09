//! Frozen 8d20e13 bot for paired, test-only comparisons using current spell physics.

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
    charging_fireball: bool,
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
            charging_fireball: false,
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

    /// Discard a planned release without resetting the deterministic movement seed.
    pub(super) fn cancel_charge(&mut self) {
        self.charging_fireball = false;
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
            self.cancel_charge();
            return ActorIntent::default();
        };
        self.think_ticks = self.think_ticks.saturating_sub(1);
        self.release_ticks = self.release_ticks.saturating_sub(1);
        self.strafe_ticks = self.strafe_ticks.saturating_sub(1);
        if self.charging_fireball
            && bot
                .charge()
                .is_none_or(|charge| charge.spell != Spell::Fireball)
        {
            self.cancel_charge();
        }
        let mut selected = None;
        let mut cast_pressed = false;
        let mut cast_released = false;
        let mut cast_held = self.charging_fireball;
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
                if self.release_ticks == 0 && !self.charging_fireball {
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
                            let speed = tuning.launch_speed(0.0);
                            for offset in [3.0, 4.5] {
                                let landing = bot.feet + toward * offset + Vec3::Y * 0.06;
                                if let Some((aim, _)) =
                                    ballistic_aim(bot.eye(), landing, tuning, speed)
                                {
                                    let mut caster = bot.clone();
                                    caster.aim = aim;
                                    caster.selected = Spell::Shield;
                                    if preview_actor(
                                        &caster, actors, collision, world, geometry, tuning, speed,
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
                        if selected.is_none() && ready(Spell::Fireball) {
                            let speed = tuning.launch_speed(tuning.reference_charge_seconds());
                            if let Some(aim) = self.fireball_aim(
                                bot, human, actors, collision, world, geometry, tuning, speed,
                            ) {
                                self.aim = Some(aim);
                                selected = Some(Spell::Fireball);
                            }
                        }
                    }
                }
            }
        }
        if let Some(spell) = selected {
            cast_pressed = true;
            if spell == Spell::Fireball {
                self.charging_fireball = true;
                cast_held = true;
            } else {
                cast_released = true; // Shield and Area Blast use an ordinary quick tap.
            }
        }
        if let Some(charge) = bot.charge().filter(|charge| {
            self.charging_fireball
                && charge.spell == Spell::Fireball
                && charge.elapsed >= tuning.reference_charge_seconds()
        }) {
            // Release checks use the current body positions and actual accumulated
            // charge. They never track or release at a target behind opaque terrain.
            let visible = [human.center(), human.eye()]
                .into_iter()
                .any(|point| visible_from(collision, bot.eye(), point));
            if visible {
                if let Some(aim) = self.fireball_aim(
                    bot,
                    human,
                    actors,
                    collision,
                    world,
                    geometry,
                    tuning,
                    tuning.launch_speed(charge.elapsed),
                ) {
                    self.aim = Some(aim);
                    cast_released = true;
                }
            }
            // Missing held input cancels an unsafe charge through actor authority.
            cast_held = false;
            self.cancel_charge();
        }
        if cast_released {
            self.release_ticks = 42; // Brief shared reaction gap between different spells.
        }
        let aim = self.aim.unwrap_or(bot.aim);
        let forward = aim.with_y(0.0).normalize_or(Vec3::NEG_Z);
        // Keep the checked launch pose on release; ordinary movement continues while charging.
        let travel = if cast_released {
            Vec3::ZERO
        } else {
            safe_travel(bot, self.travel, collision)
        };
        ActorIntent {
            movement: Vec2::new(travel.dot(forward.cross(Vec3::Y)), travel.dot(forward)),
            aim,
            cast_pressed,
            cast_released,
            cast_held,
            selected,
            ..Default::default()
        }
    }

    fn fireball_aim(
        &mut self,
        bot: &Actor,
        human: &Actor,
        actors: &[Actor],
        collision: &CollisionWorld,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        tuning: &ArenaTuning,
        speed: f32,
    ) -> Option<Vec3> {
        if bot.center().distance(human.center()) <= tuning.fireball_radius() + 0.8 {
            return None;
        }
        let target = human.center();
        let (_, time) = ballistic_aim(bot.eye(), target, tuning, speed)?;
        // One small lead estimate; no intercept search or knowledge behind cover.
        let velocity = ((human.feet - human.previous_feet) / STEP)
            .with_y(0.0)
            .clamp_length_max(7.0);
        let noise = Vec3::new(self.random() - 0.5, 0.0, self.random() - 0.5) * 0.32;
        let target = target + velocity * time.min(0.6) + noise;
        let (aim, _) = ballistic_aim(bot.eye(), target, tuning, speed)?;
        let mut caster = bot.clone();
        caster.aim = aim;
        caster.selected = Spell::Fireball;
        let forecast = preview_actor(&caster, actors, collision, world, geometry, tuning, speed);
        forecast
            .impact
            .is_none_or(|point| point.distance(bot.center()) > tuning.fireball_radius() + 0.5)
            .then_some(aim)
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
fn ballistic_aim(
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
