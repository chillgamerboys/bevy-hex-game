//! Collision-aware exploration flight and safe streamed-terrain admission.

use bevy_math::{Vec2, Vec3};
use hex_core::arena::{ArenaStreamInterest, ArenaTerrainView, ArenaVoxelGeometry};

use crate::collision::{slide_with_contacts, CollisionWorld, SKIN};
use crate::{Actor, ActorIntent, ArenaSession, Species, STEP};

const SPEED: f32 = 80.0;
const FAST_SPEED: f32 = 160.0;
const EXIT_SPEED: f32 = 32.0;

/// Read-only exploration controller and loading facts. No render frame drives physics.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FreeFlightSnapshot {
    /// F has enabled powered exploration flight; G gliding is a separate mode.
    pub active: bool,
    /// Accepted velocity after collision clipping, in world units per second.
    pub velocity: Vec3,
    /// Requested travel is waiting for exact terrain; the body has not advanced.
    pub loading: bool,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct FreeFlightState {
    pub(crate) active: bool,
    velocity: Vec3,
    requested: Vec3,
    loading: bool,
}

impl Actor {
    /// Exploration-only flight state; ordinary battle actors return `None`.
    #[must_use]
    pub fn free_flight(&self) -> Option<FreeFlightSnapshot> {
        self.free_flight.as_ref().map(|state| {
            if self.hp <= 0.0 {
                FreeFlightSnapshot::default()
            } else {
                FreeFlightSnapshot {
                    active: state.active,
                    velocity: state.velocity,
                    loading: state.loading,
                }
            }
        })
    }
}

impl ArenaSession {
    /// Whether the accepted map is an enemy-free exploration run.
    #[must_use]
    pub const fn is_exploration(&self) -> bool {
        self.exploration
    }

    /// The world's ahead-of-travel interest, including blocked travel requests.
    #[must_use]
    pub fn stream_interest(&self) -> Option<ArenaStreamInterest> {
        let actor = self
            .actors
            .iter()
            .find(|actor| Some(actor.id) == self.human_actor_id() && actor.hp > 0.0)?;
        let state = actor.free_flight.as_ref()?;
        Some(ArenaStreamInterest {
            position: actor.feet,
            velocity: state.requested,
        })
    }

    pub(crate) fn initialize_exploration(
        &mut self,
        view: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
    ) {
        let Some(actor) = self.actors.first_mut() else {
            return;
        };
        let waiting = self.collision.needs_terrain(
            actor.feet,
            Vec3::ZERO,
            actor.dimensions.y,
            actor.dimensions.x * 0.5,
        );
        if let Some(state) = &mut actor.free_flight {
            state.loading = waiting;
        }
        if waiting {
            return;
        }
        if !crate::shapes::clear(&self.collision, actor, actor.feet, actor.body_yaw)
            || !crate::encounters::dry(actor, view, geometry)
            || crate::shapes::ground(&self.collision, actor, actor.feet, SKIN * 8.0).is_none()
        {
            self.notice =
                "Exploration requires a complete, supported dry starting position.".into();
            self.encounter.spawn_failed = true;
            return;
        }
        self.encounter.initialized = true;
    }
}

/// Mode changes are independent of casting; spells work while powered flight stays active.
pub(crate) fn prepare(actor: &mut Actor, intent: ActorIntent) {
    if actor.free_flight.is_none() || actor.species != Species::Human {
        return;
    }
    if actor.hp <= 0.0 {
        actor.clear_glider();
        return;
    }
    let active = actor
        .free_flight
        .as_ref()
        .is_some_and(|flight| flight.active);
    // High Jump returns to gravity so its ordinary ballistic boost remains useful.
    if !intent.flight_toggle && !(intent.high_jump && active) {
        return;
    }
    if active {
        let velocity = actor
            .free_flight
            .as_ref()
            .map_or(Vec3::ZERO, |flight| flight.velocity)
            .clamp_length_max(EXIT_SPEED);
        actor.body.airborne_momentum = Some(velocity.with_y(0.0));
        actor.body.control_velocity = velocity.with_y(0.0);
        actor.body.vertical_velocity = velocity.y;
        actor.body.impulse_velocity = Vec3::ZERO;
        if let Some(state) = &mut actor.free_flight {
            state.active = false;
            state.velocity = velocity;
        }
    } else {
        crate::glider::fold(actor);
        let velocity = actor.body.control_velocity
            + actor.body.impulse_velocity
            + Vec3::Y * actor.body.vertical_velocity;
        if let Some(state) = &mut actor.free_flight {
            state.active = true;
            state.velocity = velocity;
        }
        actor.body.control_velocity = Vec3::ZERO;
        actor.body.airborne_momentum = None;
        actor.body.impulse_velocity = Vec3::ZERO;
        actor.body.vertical_velocity = 0.0;
        actor.grounded = false;
        actor.body.grounded = false;
    }
}

fn input_velocity(intent: ActorIntent, aim: Vec3) -> Vec3 {
    let look = if intent.glider_look.is_finite() {
        intent.glider_look.normalize_or(aim)
    } else {
        aim
    };
    let yaw = look.with_y(0.0).normalize_or(Vec3::NEG_Z);
    let axes = if intent.movement.is_finite() {
        intent.movement.clamp_length_max(1.0)
    } else {
        Vec2::ZERO
    };
    let vertical = if intent.flight_vertical.is_finite() {
        intent.flight_vertical.clamp(-1.0, 1.0)
    } else {
        0.0
    };
    let direction = look * axes.y + yaw.cross(Vec3::Y) * axes.x + Vec3::Y * vertical;
    direction.clamp_length_max(1.0)
        * if intent.flight_fast {
            FAST_SPEED
        } else {
            SPEED
        }
}

fn ordinary_request(actor: &Actor, intent: ActorIntent) -> Vec3 {
    if actor.glider.open {
        return actor.glider.snapshot().velocity
            + actor.body.impulse_velocity
            + Vec3::Y * actor.body.vertical_velocity;
    }
    let axes = if intent.movement.is_finite() {
        intent.movement.clamp_length_max(1.0)
    } else {
        Vec2::ZERO
    };
    let forward = actor.aim.with_y(0.0).normalize_or(Vec3::NEG_Z);
    let requested = (forward * axes.y + forward.cross(Vec3::Y) * axes.x).normalize_or_zero()
        * actor.walking_speed;
    actor.body.airborne_momentum.unwrap_or(requested)
        + actor.body.impulse_velocity
        + Vec3::Y * (actor.body.vertical_velocity - crate::controller::GRAVITY * STEP)
}

/// Handle powered flight or suspend ordinary motion before unadmitted terrain.
/// Returning false delegates to the unchanged walking/gliding controller.
pub(crate) fn tick_or_wait(actor: &mut Actor, intent: ActorIntent, world: &CollisionWorld) -> bool {
    let Some(state) = actor.free_flight.as_ref() else {
        return false;
    };
    let active = state.active;
    let velocity = if active {
        input_velocity(intent, actor.aim) + actor.body.impulse_velocity
    } else {
        ordinary_request(actor, intent)
    };
    let loading = world.needs_terrain(
        actor.feet,
        velocity * STEP,
        actor.dimensions.y,
        actor.dimensions.x * 0.5,
    );
    if let Some(state) = &mut actor.free_flight {
        state.requested = velocity;
        state.loading = loading;
        state.velocity = if loading { Vec3::ZERO } else { velocity };
    }
    if loading {
        return true;
    }
    if !active {
        return false;
    }
    actor.body.step_rise = 0.0;
    if !world.clear(actor.feet, actor.dimensions.y, actor.dimensions.x * 0.5) {
        return true;
    }
    let (feet, contacts) = slide_with_contacts(
        world,
        actor.feet,
        velocity * STEP,
        actor.dimensions.y,
        actor.dimensions.x * 0.5,
    );
    actor.feet = feet;
    actor.body_yaw = (-actor.aim.x).atan2(-actor.aim.z);
    actor.grounded = false;
    actor.body.grounded = false;
    let mut accepted = velocity;
    for normal in contacts {
        accepted -= normal * accepted.dot(normal).min(0.0);
        actor.body.impulse_velocity -= normal * actor.body.impulse_velocity.dot(normal).min(0.0);
    }
    actor.body.impulse_velocity *= (-3.0 * STEP).exp();
    if let Some(state) = &mut actor.free_flight {
        state.velocity = accepted;
    }
    true
}

/// Publish accepted walking/gliding motion separately from the streaming request.
pub(crate) fn finish(actor: &mut Actor) {
    if let Some(state) = &mut actor.free_flight {
        if !state.active && !state.loading {
            state.velocity = (actor.feet - actor.previous_feet) / STEP;
        }
    }
}

#[cfg(test)]
mod tests;
