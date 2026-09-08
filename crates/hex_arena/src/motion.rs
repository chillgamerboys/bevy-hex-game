//! Profile movement with one unchanged M01 path and a swept low flying body.

use crate::collision::{CollisionWorld, SKIN};
use crate::controller::GroundProfile;
use crate::shapes;
use crate::{Actor, EncounterTuning, Species, STEP};
use bevy_math::Vec3;

#[cfg(test)]
#[path = "wisp_motion_tests.rs"]
mod wisp_tests;

pub(crate) fn tick(
    actor: &mut Actor,
    direction: Vec3,
    run: bool,
    jump: bool,
    flight: bool,
    world: &CollisionWorld,
    tuning: &EncounterTuning,
) {
    if actor.species == Species::Wisp {
        wisp_tick(actor, direction, world, tuning);
        return;
    }
    actor.previous_yaw = actor.body_yaw;
    if actor.species == Species::Golem {
        golem_tick(actor, direction, world, tuning);
        return;
    }
    if actor.species != Species::Dragon {
        actor.body_yaw = (-actor.aim.x).atan2(-actor.aim.z);
        let profile = match actor.species {
            Species::Goblin => GroundProfile {
                height: actor.dimensions.y,
                radius: actor.dimensions.x * 0.5,
                walk: tuning.goblin_walk,
                run: tuning.goblin_run,
            },
            Species::Shaman => GroundProfile {
                walk: tuning.shaman_walk,
                run: tuning.shaman_run,
                ..Default::default()
            },
            _ => GroundProfile::default(),
        };
        actor
            .body
            .tick_profile(&mut actor.feet, direction, run, jump, world, profile);
        actor.grounded = actor.body.grounded;
        return;
    }
    if !shapes::clear(world, actor, actor.feet, actor.body_yaw) {
        return;
    }
    let forward = if direction.with_y(0.0).length_squared() > 0.01 {
        direction
    } else {
        actor.aim
    };
    let yaw = (-forward.x).atan2(-forward.z);
    shapes::turn(world, actor, yaw, tuning.dragon_turn_speed * STEP);
    actor.body.step_rise = 0.0;
    let floor = shapes::ground(world, actor, actor.feet, SKIN * 8.0);
    if flight {
        if !actor.flying {
            actor.body.impulse_velocity.y += actor.body.vertical_velocity;
            actor.body.vertical_velocity = 0.0;
        }
        actor.flying = true;
        actor.body.grounded = false;
        let delta = (direction.clamp_length_max(1.0) * tuning.dragon_flight_speed
            + actor.body.impulse_velocity)
            * STEP;
        let (feet, contacts) = shapes::slide(world, actor, actor.feet, delta);
        actor.feet = feet;
        for normal in contacts {
            actor.body.impulse_velocity -=
                normal * actor.body.impulse_velocity.dot(normal).min(0.0);
        }
        actor.body.impulse_velocity *= (-3.0 * STEP).exp();
    } else {
        if actor.flying {
            actor.body.vertical_velocity += actor.body.impulse_velocity.y;
            actor.body.impulse_velocity.y = 0.0;
        }
        actor.flying = false;
        actor.body.vertical_velocity += actor.body.impulse_velocity.y;
        actor.body.impulse_velocity.y = 0.0;
        actor.body.grounded = floor.is_some() && actor.body.vertical_velocity <= 0.0;
        if actor.body.grounded {
            if let Some(feet) = floor {
                actor.feet = feet;
            }
            actor.body.vertical_velocity = 0.0;
        }
        let horizontal = direction.with_y(0.0).normalize_or_zero() * tuning.dragon_ground_speed
            + actor.body.impulse_velocity;
        let vertical = if actor.body.grounded {
            0.0
        } else {
            let y = actor.body.vertical_velocity * STEP - 0.5 * 17.333_334 * STEP * STEP;
            actor.body.vertical_velocity -= 17.333_334 * STEP;
            y
        };
        let original = actor.feet;
        let (mut feet, mut contacts) = shapes::slide(
            world,
            actor,
            original,
            horizontal * STEP + Vec3::Y * vertical,
        );
        if actor.body.grounded
            && (feet - original).with_y(0.0).length_squared() + SKIN * SKIN
                < (horizontal * STEP).length_squared()
        {
            let rise = Vec3::Y * (0.4 + SKIN * 2.0);
            let allowed =
                shapes::sweep(world, actor, original, rise).map_or(rise, |h| rise * h.fraction);
            let raised = original + allowed;
            if shapes::clear(world, actor, raised, actor.body_yaw) {
                let (across, accepted) = shapes::slide(world, actor, raised, horizontal * STEP);
                if (across - raised).with_y(0.0).length_squared()
                    > (feet - original).with_y(0.0).length_squared() + SKIN * SKIN
                {
                    if let Some(land) = shapes::ground(world, actor, across, allowed.y + SKIN * 4.0)
                    {
                        if shapes::clear(world, actor, land, actor.body_yaw) {
                            feet = land;
                            contacts = accepted;
                            actor.body.step_rise = (land.y - original.y).max(0.0);
                        }
                    }
                }
            }
        }
        actor.feet = feet;
        for normal in contacts {
            actor.body.impulse_velocity -=
                normal * actor.body.impulse_velocity.dot(normal).min(0.0);
            if normal.y.abs() > 0.5 && actor.body.vertical_velocity * normal.y < 0.0 {
                actor.body.vertical_velocity = 0.0;
                actor.body.grounded = normal.y > 0.5;
            }
        }
        actor.body.impulse_velocity *= (-3.0 * STEP).exp();
    }
    actor.grounded = actor.body.grounded;
}

// The one-level body always hovers, including stationary windup and bootstrap
// ticks. Voluntary input never changes its physical yaw or absorbs real impulse.
fn wisp_tick(actor: &mut Actor, direction: Vec3, world: &CollisionWorld, tuning: &EncounterTuning) {
    actor.body_yaw = 0.0;
    actor.previous_yaw = 0.0;
    actor.body.step_rise = 0.0;
    actor.body.impulse_velocity.y += actor.body.vertical_velocity;
    actor.body.vertical_velocity = 0.0;
    actor.flying = true;
    actor.grounded = false;
    actor.body.grounded = false;
    if !shapes::clear(world, actor, actor.feet, 0.0) {
        return;
    }
    let delta = (direction.clamp_length_max(1.0) * tuning.wisp_flight_speed
        + actor.body.impulse_velocity)
        * STEP;
    let (feet, contacts) = shapes::slide(world, actor, actor.feet, delta);
    actor.feet = feet;
    for normal in contacts {
        actor.body.impulse_velocity -= normal * actor.body.impulse_velocity.dot(normal).min(0.0);
    }
    actor.body.impulse_velocity *= (-3.0 * STEP).exp();
}

// Keep the accepted capsule and Dragon paths unchanged. This ground-only path
// sweeps the complete fixed prism union and never admits jump or flight input.
fn golem_tick(
    actor: &mut Actor,
    direction: Vec3,
    world: &CollisionWorld,
    tuning: &EncounterTuning,
) {
    actor.body_yaw = 0.0;
    actor.previous_yaw = 0.0;
    actor.body.step_rise = 0.0;
    if !shapes::clear(world, actor, actor.feet, 0.0) {
        return;
    }
    let floor = shapes::ground(world, actor, actor.feet, SKIN * 8.0);
    actor.flying = false;
    actor.body.vertical_velocity += actor.body.impulse_velocity.y;
    actor.body.impulse_velocity.y = 0.0;
    actor.body.grounded = floor.is_some() && actor.body.vertical_velocity <= 0.0;
    if actor.body.grounded {
        if let Some(feet) = floor {
            actor.feet = feet;
        }
        actor.body.vertical_velocity = 0.0;
    }
    let horizontal = direction.with_y(0.0).normalize_or_zero() * tuning.golem_speed
        + actor.body.impulse_velocity;
    let vertical = if actor.body.grounded {
        0.0
    } else {
        let y = actor.body.vertical_velocity * STEP - 0.5 * 17.333_334 * STEP * STEP;
        actor.body.vertical_velocity -= 17.333_334 * STEP;
        y
    };
    let original = actor.feet;
    let (mut feet, mut contacts) = shapes::slide(
        world,
        actor,
        original,
        horizontal * STEP + Vec3::Y * vertical,
    );
    if actor.body.grounded
        && (feet - original).with_y(0.0).length_squared() + SKIN * SKIN
            < (horizontal * STEP).length_squared()
    {
        let rise = Vec3::Y * (0.4 + SKIN * 2.0);
        let allowed =
            shapes::sweep(world, actor, original, rise).map_or(rise, |h| rise * h.fraction);
        let raised = original + allowed;
        if shapes::clear(world, actor, raised, actor.body_yaw) {
            let (across, accepted) = shapes::slide(world, actor, raised, horizontal * STEP);
            if (across - raised).with_y(0.0).length_squared()
                > (feet - original).with_y(0.0).length_squared() + SKIN * SKIN
            {
                if let Some(land) = shapes::ground(world, actor, across, allowed.y + SKIN * 4.0) {
                    if shapes::clear(world, actor, land, actor.body_yaw) {
                        feet = land;
                        contacts = accepted;
                        actor.body.step_rise = (land.y - original.y).max(0.0);
                    }
                }
            }
        }
    }
    actor.feet = feet;
    for normal in contacts {
        actor.body.impulse_velocity -= normal * actor.body.impulse_velocity.dot(normal).min(0.0);
        if normal.y.abs() > 0.5 && actor.body.vertical_velocity * normal.y < 0.0 {
            actor.body.vertical_velocity = 0.0;
            actor.body.grounded = normal.y > 0.5;
        }
    }
    actor.body.impulse_velocity *= (-3.0 * STEP).exp();
    actor.grounded = actor.body.grounded;
}
