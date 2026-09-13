//! Player-only momentum flight. Opening changes the controller, never velocity.

use bevy_math::{Quat, Vec3};
use hex_core::arena::{ArenaTerrainView, ArenaVoxelGeometry};

use crate::collision::{slide_with_contacts, CollisionWorld, SKIN};
use crate::controller::{GroundProfile, GRAVITY};
use crate::{Actor, ActorIntent, Species, STEP};

const MAX_SPEED: f32 = 32.0;
const FULL_LIFT_SPEED: f32 = 8.0;
const STALL_SPEED: f32 = 4.0;

/// Authoritative read-only flight facts for the canopy and compact HUD cue.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GliderSnapshot {
    /// Whether the player is currently controlling an open glider.
    pub open: bool,
    /// Persistent world-space motion; neither walking nor Dragon flight input.
    pub velocity: Vec3,
    /// Speed relative to the wind, in world units per second.
    pub airspeed: f32,
    /// Zero with full lift, one at or below the distinct stall speed.
    pub stall_fraction: f32,
    /// Current bounded flight heading for cosmetic canopy orientation.
    pub direction: Vec3,
}

#[derive(Clone, Debug)]
pub(crate) struct GliderState {
    pub(crate) open: bool,
    velocity: Vec3,
    look: Vec3,
    heading: Vec3,
    pub(crate) wind: Vec3,
}

impl Default for GliderState {
    fn default() -> Self {
        Self {
            open: false,
            velocity: Vec3::ZERO,
            look: Vec3::NEG_Z,
            heading: Vec3::NEG_Z,
            wind: Vec3::ZERO,
        }
    }
}

impl GliderState {
    pub(crate) fn snapshot(&self) -> GliderSnapshot {
        let airspeed = (self.velocity - self.wind).length();
        GliderSnapshot {
            open: self.open,
            velocity: self.velocity,
            airspeed,
            stall_fraction: 1.0 - lift(airspeed),
            direction: self.heading,
        }
    }
}

fn lift(speed: f32) -> f32 {
    ((speed - STALL_SPEED) / (FULL_LIFT_SPEED - STALL_SPEED)).clamp(0.0, 1.0)
}

fn clamped_look(look: Vec3) -> Vec3 {
    let normalized = look.normalize_or(Vec3::NEG_Z);
    let pitch = normalized
        .y
        .asin()
        .clamp(-60_f32.to_radians(), 30_f32.to_radians());
    let horizontal = normalized.with_y(0.0).normalize_or(Vec3::NEG_Z);
    horizontal * pitch.cos() + Vec3::Y * pitch.sin()
}

fn turn_towards(current: Vec3, target: Vec3, limit: f32) -> Vec3 {
    let dot = current.dot(target).clamp(-1.0, 1.0);
    let angle = dot.acos();
    if angle <= limit || angle < 0.0001 {
        target
    } else {
        Quat::IDENTITY.slerp(Quat::from_rotation_arc(current, target), limit / angle) * current
    }
}

/// Tuning at 20 units/s: level -0.58, dive45 +11.68, climb30 -14.25 units/s².
fn acceleration(speed: f32, direction: Vec3) -> f32 {
    -GRAVITY * direction.y - 10.0 * direction.y.max(0.0) - 0.22 - 0.0009 * speed * speed
}

fn velocity_step(state: &mut GliderState) {
    // Keep persistent ground velocity, applying flight forces in the moving air.
    // Changing wind or toggling the canopy supplies no instantaneous velocity.
    state.velocity -= state.wind;
    let speed = state.velocity.length();
    let turn_degrees = 100.0 - 45.0 * (speed / MAX_SPEED).clamp(0.0, 1.0);
    // Heading stays bounded even when opening during a vertical fall or boost.
    state.heading = turn_towards(state.heading, state.look, turn_degrees.to_radians() * STEP);
    state.heading = clamped_look(state.heading);
    let current = state.velocity.normalize_or(state.heading);
    let direction = turn_towards(
        current,
        state.heading,
        turn_degrees.to_radians() * STEP * lift(speed),
    );
    let glide_acceleration =
        acceleration(speed, direction) + GRAVITY * direction.y * (1.0 - lift(speed));
    let next_speed = (speed + glide_acceleration * STEP).max(0.0);
    state.velocity = direction * next_speed;
    // Lost lift means real downward acceleration, not an artificial fold or
    // a forward launch kick. Diving recovers speed and then steering authority.
    state.velocity.y -= GRAVITY * (1.0 - lift(speed)) * STEP;
    state.velocity = state.velocity.clamp_length_max(MAX_SPEED);
    state.velocity += state.wind;
}

pub(crate) fn fold(actor: &mut Actor) {
    if !actor.glider.open {
        return;
    }
    let velocity = actor.glider.velocity
        + actor.body.impulse_velocity
        + Vec3::Y * actor.body.vertical_velocity;
    actor.glider.open = false;
    actor.glider.velocity = velocity;
    actor.body.airborne_momentum = Some(velocity.with_y(0.0));
    actor.body.control_velocity = velocity.with_y(0.0);
    actor.body.vertical_velocity = velocity.y;
    actor.body.impulse_velocity = Vec3::ZERO;
}

/// Apply G/casting transitions before High Jump can modify ballistic velocity.
pub(crate) fn prepare(
    actor: &mut Actor,
    intent: ActorIntent,
    world: &CollisionWorld,
    view: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
) {
    if !actor.expedition_player
        || actor.species != Species::Human
        || actor.boat().is_some_and(|boat| boat.active)
        || actor
            .free_flight
            .as_ref()
            .is_some_and(|flight| flight.active)
    {
        return;
    }
    if actor.hp <= 0.0 {
        actor.clear_glider();
        return;
    }
    if intent.glider_look.is_finite() && intent.glider_look.length_squared() > 0.0001 {
        actor.glider.look = clamped_look(intent.glider_look);
    }
    let casting =
        intent.cast_pressed || intent.cast_held || actor.charge.is_some() || intent.high_jump;
    if casting {
        fold(actor);
        return;
    }
    if !actor.glider.open && !intent.glider_toggle {
        return;
    }
    if !crate::encounters::dry(actor, view, geometry) {
        fold(actor);
        return;
    }
    if !intent.glider_toggle {
        return;
    }
    if actor.glider.open {
        fold(actor);
    } else if world.clear(actor.feet, actor.dimensions.y, actor.dimensions.x * 0.5) {
        actor.glider.velocity = actor.body.control_velocity
            + actor.body.impulse_velocity
            + Vec3::Y * actor.body.vertical_velocity;
        actor.glider.heading = clamped_look(actor.glider.velocity.normalize_or(actor.glider.look));
        actor.glider.open = true;
        if !actor.grounded {
            actor.body.control_velocity = Vec3::ZERO;
            actor.body.airborne_momentum = None;
            actor.body.vertical_velocity = 0.0;
            actor.body.impulse_velocity = Vec3::ZERO;
        }
    }
}

/// The same complete physical player body is swept against all solid geometry.
pub(crate) fn tick(actor: &mut Actor, world: &CollisionWorld, profile: GroundProfile) {
    actor.body.step_rise = 0.0;
    if !world.clear(actor.feet, profile.height, profile.radius) {
        actor.glider.velocity = Vec3::ZERO;
        fold(actor);
        return;
    }
    actor.glider.velocity += actor.body.impulse_velocity + Vec3::Y * actor.body.vertical_velocity;
    actor.body.impulse_velocity = Vec3::ZERO;
    actor.body.vertical_velocity = 0.0;
    velocity_step(&mut actor.glider);
    let (feet, contacts) = slide_with_contacts(
        world,
        actor.feet,
        actor.glider.velocity * STEP,
        profile.height,
        profile.radius,
    );
    actor.feet = feet;
    actor.grounded = false;
    actor.body.grounded = false;
    if !contacts.is_empty() {
        let mut blocking = false;
        for normal in contacts {
            actor.glider.velocity -= normal * actor.glider.velocity.dot(normal).min(0.0);
            if normal.y > 0.5 {
                actor.grounded = true;
                actor.body.grounded = true;
            } else {
                blocking = true;
            }
        }
        if blocking {
            fold(actor);
        }
    } else if actor.glider.velocity.y <= 0.0 {
        if let Some(feet) = world.ground(actor.feet, profile.height, profile.radius, SKIN * 8.0) {
            actor.feet = feet;
            actor.glider.velocity.y = 0.0;
            actor.grounded = true;
            actor.body.grounded = true;
        }
    }
}

/// Water retains its existing physics; crossing it just folds the canopy.
pub(crate) fn finish(actor: &mut Actor, view: &ArenaTerrainView, geometry: ArenaVoxelGeometry) {
    if actor.hp <= 0.0 {
        actor.clear_glider();
    } else if actor.glider.open && !crate::encounters::dry(actor, view, geometry) {
        fold(actor);
    } else if actor.glider.open && actor.grounded {
        actor.glider.velocity = actor.body.control_velocity
            + actor.body.impulse_velocity
            + Vec3::Y * actor.body.vertical_velocity;
    }
}

#[cfg(test)]
mod tests;
