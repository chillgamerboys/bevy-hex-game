//! Exploration sailing and swimming over world-owned admitted water surfaces.

use bevy_math::{Quat, Vec2, Vec3};
use hex_core::arena::{ArenaAvailability, ArenaTerrainView, ArenaVoxelGeometry};
use hex_core::ocean::{
    OceanEnvironmentView, OceanSimulationTime, OceanSurfaceState, OceanWaterColumn,
};
use hex_core::{HexCoord, TilePos};

use crate::collision::{slide_with_contacts, CollisionWorld};
use crate::{Actor, ActorIntent, ArenaSession, STEP};

const OXYGEN: f32 = 90.0;
const DECK: f32 = 0.35;
const BOAT_SPEED: f32 = 24.0;
const HULL_RADIUS: f32 = 0.65;
const HULL_HEIGHT: f32 = 0.65;

/// Authoritative boat pose and motion; presentation adds no player displacement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoatSnapshot {
    /// Whether the portable boat is deployed.
    pub active: bool,
    /// Unit horizontal heading of the hull.
    pub heading: Vec3,
    /// Ground velocity in world units/s, including the surface's vertical motion.
    pub velocity: Vec3,
    /// Sampled upward water normal for the hull's visual orientation.
    pub surface_normal: Vec3,
    /// Current horizontal wind, in world units/s.
    pub wind: Vec3,
}

impl Default for BoatSnapshot {
    fn default() -> Self {
        Self {
            active: false,
            heading: Vec3::NEG_Z,
            velocity: Vec3::ZERO,
            surface_normal: Vec3::Y,
            wind: Vec3::ZERO,
        }
    }
}

/// Physical swimming and breathing facts, independent of camera mode.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SwimSnapshot {
    /// Whether swimming currently controls the body.
    pub active: bool,
    /// Remaining breathable reserve, measured in simulation seconds.
    pub oxygen_seconds: f32,
    /// Full reserve; a surfaced player refills this in six seconds.
    pub oxygen_capacity_seconds: f32,
    /// The physical eye is beneath the sampled visible water surface.
    pub submerged: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct MarineState {
    boat: BoatSnapshot,
    swim: SwimSnapshot,
    velocity: Vec3,
}

impl Default for MarineState {
    fn default() -> Self {
        Self {
            boat: BoatSnapshot::default(),
            swim: SwimSnapshot {
                active: false,
                oxygen_seconds: OXYGEN,
                oxygen_capacity_seconds: OXYGEN,
                submerged: false,
            },
            velocity: Vec3::ZERO,
        }
    }
}

impl MarineState {
    pub(crate) fn stop_on_death(&mut self) {
        self.boat = BoatSnapshot::default();
        self.velocity = Vec3::ZERO;
        self.swim.active = false;
        // Preserve the last breathing facts for the terminal HUD. Only Restart
        // constructs a fresh reserve; pausing or repeated cleanup cannot refill it.
    }
}

impl Actor {
    /// Exploration boat facts; legacy maps and enemies return `None`.
    #[must_use]
    pub fn boat(&self) -> Option<BoatSnapshot> {
        self.marine.as_ref().map(|state| state.boat)
    }

    /// Exploration swimming and oxygen facts; these never follow a spectator camera.
    #[must_use]
    pub fn swimming(&self) -> Option<SwimSnapshot> {
        self.marine.as_ref().map(|state| state.swim)
    }
}

impl ArenaSession {
    /// Completed fixed-step time shared by ocean physics and presentation.
    #[must_use]
    pub fn ocean_time(&self) -> OceanSimulationTime {
        OceanSimulationTime::from_fixed_tick(
            self.generation.unwrap_or(0),
            self.tick,
            f64::from(STEP),
        )
    }
}

pub(crate) struct MarineWorld<'a> {
    pub terrain: &'a ArenaTerrainView,
    pub geometry: ArenaVoxelGeometry,
    pub environment: Option<&'a OceanEnvironmentView>,
    pub time: OceanSimulationTime,
}

impl MarineWorld<'_> {
    fn sample(&self, point: Vec3) -> OceanSurfaceState {
        let coord = HexCoord::from_world(point);
        let availability = self.terrain.residency.as_ref().map_or_else(
            || {
                if self.geometry.contains_column(coord) {
                    ArenaAvailability::Ready
                } else {
                    ArenaAvailability::OutsideWorld
                }
            },
            |residency| residency.at(coord, self.geometry),
        );
        let Some(environment) = self.environment else {
            return OceanSurfaceState::Unloaded;
        };
        // Northern publication guarantees sorted liquid runs. No cached admission
        // or whole-ocean scan: seek this column in the current exact publication.
        let start = self
            .terrain
            .liquids
            .partition_point(|span| span.bottom.coord < coord);
        let column = self
            .terrain
            .liquids
            .get(start..)
            .and_then(|runs| {
                runs.iter()
                    .take_while(|span| span.bottom.coord == coord)
                    .last()
            })
            .map(|span| OceanWaterColumn {
                mean_height: self.geometry.top(TilePos::new(coord, span.top_level)),
                bed_height: self.geometry.top(span.bottom) - self.geometry.level_height,
                water_id: span.substance,
            });
        environment.sample(Vec2::new(point.x, point.z), self.time, availability, column)
    }

    fn wind(&self) -> Vec3 {
        self.environment.map_or(Vec3::ZERO, |environment| {
            let velocity = environment.wind.velocity_at(self.time);
            Vec3::new(velocity.x, 0.0, velocity.y)
        })
    }
}

fn body_velocity(actor: &Actor) -> Vec3 {
    actor.body.control_velocity
        + actor.body.impulse_velocity
        + Vec3::Y * actor.body.vertical_velocity
}

fn transfer_velocity(actor: &mut Actor, velocity: Vec3) {
    actor.body.control_velocity = velocity.with_y(0.0);
    actor.body.airborne_momentum = Some(velocity.with_y(0.0));
    actor.body.vertical_velocity = velocity.y;
    actor.body.impulse_velocity = Vec3::ZERO;
}

fn fold_boat(actor: &mut Actor) {
    let velocity = actor
        .marine
        .as_ref()
        .filter(|state| state.boat.active)
        .map(|state| state.boat.velocity);
    if let Some(velocity) = velocity {
        let velocity =
            velocity + actor.body.impulse_velocity + Vec3::Y * actor.body.vertical_velocity;
        if let Some(state) = &mut actor.marine {
            state.boat.active = false;
            state.velocity = velocity;
        }
        transfer_velocity(actor, velocity);
    }
}

fn hull_positions(feet: Vec3, heading: Vec3) -> [Vec3; 3] {
    [-0.8, 0.0, 0.8].map(|offset| feet + heading * offset - Vec3::Y * (DECK + 0.25))
}

fn boat_clear(world: &CollisionWorld, feet: Vec3, heading: Vec3, actor: &Actor) -> bool {
    world.clear(feet, actor.dimensions.y, actor.dimensions.x * 0.5)
        && hull_positions(feet, heading)
            .into_iter()
            .all(|p| world.clear(p, HULL_HEIGHT, HULL_RADIUS))
}

/// Mode transitions happen before free flight, glider and High Jump preparation.
pub(crate) fn prepare(
    actor: &mut Actor,
    intent: ActorIntent,
    sea: &MarineWorld<'_>,
    world: &CollisionWorld,
) -> Option<&'static str> {
    actor.glider.wind = sea.wind();
    actor.marine.as_ref()?;
    if actor.hp <= 0.0 {
        return None;
    }
    if intent.flight_toggle
        || intent.high_jump
        || actor.body.impulse_velocity.length_squared() > 0.0001
    {
        fold_boat(actor);
    }
    if sea.environment.is_none() {
        fold_boat(actor);
        return intent.boat_toggle.then_some("The ocean is still loading.");
    }
    if !intent.boat_toggle {
        return None;
    }
    if actor.marine.as_ref().is_some_and(|state| state.boat.active) {
        fold_boat(actor);
        return None;
    }
    if intent.flight_toggle
        || intent.high_jump
        || actor.free_flight.as_ref().is_some_and(|state| state.active)
    {
        return Some("Fold free flight before deploying the boat near water.");
    }
    let approach = actor.aim.with_y(0.0).normalize_or(Vec3::NEG_Z);
    let incoming = if actor.glider.open {
        actor.glider.snapshot().velocity
            + actor.body.impulse_velocity
            + Vec3::Y * actor.body.vertical_velocity
    } else {
        body_velocity(actor)
    }
    .with_y(0.0)
    .clamp_length_max(BOAT_SPEED);
    // Mounting cannot redirect existing travel toward a backward camera view.
    let heading = incoming.normalize_or(approach);
    for distance in [0.0, 0.6, 1.2] {
        let candidate = actor.feet + approach * distance;
        let OceanSurfaceState::ReadyWet(surface) = sea.sample(candidate) else {
            continue;
        };
        if (actor.feet.y - surface.height).abs() > 1.5
            || surface.mean_height - surface.bed_height < 0.7
        {
            continue;
        }
        let feet = candidate.with_y(surface.height + DECK);
        let delta = feet - actor.feet;
        if world.needs_terrain(actor.feet, delta, actor.dimensions.y, actor.dimensions.x * 0.5)
            || world.sweep(actor.feet, delta, actor.dimensions.y, actor.dimensions.x * 0.5).is_some()
            || !boat_clear(world, feet, heading, actor)
            || hull_positions(feet, heading).into_iter().any(|point| !matches!(sea.sample(point), OceanSurfaceState::ReadyWet(s) if s.mean_height - s.bed_height >= 0.7))
        { continue; }
        crate::glider::fold(actor);
        let velocity = incoming;
        if let Some(state) = &mut actor.marine {
            state.boat = BoatSnapshot {
                active: true,
                heading,
                velocity,
                surface_normal: surface.normal,
                wind: sea.wind(),
            };
            state.swim.active = false;
            state.velocity = velocity;
        }
        actor.feet = feet;
        transfer_velocity(actor, Vec3::ZERO);
        actor.grounded = false;
        actor.body.grounded = false;
        return None;
    }
    Some("Deploy the boat beside clear, sufficiently deep ocean water.")
}

fn breathing(actor: &mut Actor, sample: OceanSurfaceState) {
    let eye = actor.eye();
    let Some(state) = &mut actor.marine else {
        return;
    };
    state.swim.submerged = match sample {
        OceanSurfaceState::ReadyWet(surface) => {
            eye.y < surface.height - 0.08 && eye.y > surface.bed_height
        }
        OceanSurfaceState::ReadyDry => false,
        OceanSurfaceState::Unloaded | OceanSurfaceState::OutsideWorld => return,
    };
    if state.swim.submerged {
        state.swim.oxygen_seconds = (state.swim.oxygen_seconds - STEP).max(0.0);
        if state.swim.oxygen_seconds <= 0.0 {
            actor.hp = (actor.hp - 10.0 * STEP).max(0.0);
        }
    } else {
        state.swim.oxygen_seconds = (state.swim.oxygen_seconds + OXYGEN / 6.0 * STEP).min(OXYGEN);
    }
}

fn telemetry(actor: &mut Actor, requested: Vec3, loading: bool) {
    if let Some(flight) = &mut actor.free_flight {
        flight.requested = requested;
        flight.loading = loading;
    }
}

fn boat_velocity(boat: BoatSnapshot, intent: ActorIntent, aim: Vec3) -> (Vec3, Vec3) {
    let speed = boat.velocity.with_y(0.0).length();
    let turn = (70.0 - 35.0 * (speed / BOAT_SPEED)).to_radians() * STEP;
    let target = if intent.movement.y > 0.0 {
        aim.with_y(0.0).normalize_or(boat.heading)
    } else {
        boat.heading
    };
    let signed = boat.heading.cross(target).y.atan2(boat.heading.dot(target));
    let heading = Quat::from_rotation_y(
        signed.clamp(-turn, turn) - intent.movement.x.clamp(-1.0, 1.0) * turn,
    ) * boat.heading;
    let alignment = heading.dot(boat.wind.normalize_or_zero());
    let mut acceleration = 3.0 * alignment.max(0.0) * boat.wind.length() / 10.0
        - 0.25
        - 0.006 * speed * speed
        - 2.0 * (-alignment).max(0.0);
    if intent.movement.y > 0.0 && speed < 4.0 {
        acceleration = acceleration.max(1.5);
    }
    if intent.movement.y < 0.0 {
        acceleration -= 8.0;
    }
    (
        heading,
        heading * (speed + acceleration * STEP).clamp(0.0, BOAT_SPEED),
    )
}

fn boat_tick(
    actor: &mut Actor,
    intent: ActorIntent,
    sea: &MarineWorld<'_>,
    world: &CollisionWorld,
) {
    let Some(state) = actor.marine.as_ref() else {
        return;
    };
    let boat = BoatSnapshot {
        wind: sea.wind(),
        ..state.boat
    };
    let (heading, velocity) = boat_velocity(boat, intent, actor.aim);
    let next = actor.feet + velocity * STEP;
    let surface = match sea.sample(next) {
        OceanSurfaceState::ReadyWet(surface) => surface,
        OceanSurfaceState::Unloaded => {
            telemetry(actor, velocity, true);
            return;
        }
        OceanSurfaceState::ReadyDry | OceanSurfaceState::OutsideWorld => {
            if let Some(state) = &mut actor.marine {
                state.boat.velocity = Vec3::ZERO;
            }
            telemetry(actor, velocity, false);
            return;
        }
    };
    let desired = next.with_y(surface.height + DECK);
    let delta = desired - actor.feet;
    let mut fraction = 1.0_f32;
    let mut loading = world.needs_terrain(
        actor.feet,
        delta,
        actor.dimensions.y,
        actor.dimensions.x * 0.5,
    );
    for point in hull_positions(actor.feet, heading) {
        loading |= world.needs_terrain(point, delta, HULL_HEIGHT, HULL_RADIUS);
        match sea.sample(point + delta) {
            OceanSurfaceState::Unloaded => loading = true,
            OceanSurfaceState::ReadyWet(s) if s.mean_height - s.bed_height >= 0.7 => {}
            _ => fraction = 0.0,
        }
        if let Some(hit) = world.sweep(point, delta, HULL_HEIGHT, HULL_RADIUS) {
            fraction = fraction.min(hit.fraction);
        }
    }
    telemetry(actor, velocity, loading);
    if loading {
        return;
    }
    if let Some(hit) = world.sweep(
        actor.feet,
        delta,
        actor.dimensions.y,
        actor.dimensions.x * 0.5,
    ) {
        fraction = fraction.min(hit.fraction);
    }
    if !boat_clear(world, actor.feet, heading, actor) {
        if let Some(state) = &mut actor.marine {
            state.boat.velocity = Vec3::ZERO;
        }
        return;
    }
    actor.feet += delta * fraction;
    actor.body_yaw = (-heading.x).atan2(-heading.z);
    if let Some(state) = &mut actor.marine {
        state.boat.heading = heading;
        state.boat.velocity = if fraction < 1.0 {
            Vec3::ZERO
        } else {
            velocity.with_y(surface.vertical_velocity)
        };
        state.boat.surface_normal = surface.normal;
        state.boat.wind = boat.wind;
        state.velocity = state.boat.velocity;
    }
}

/// Returns true when boat/swimming owns movement or must wait for exact terrain.
pub(crate) fn tick_or_wait(
    actor: &mut Actor,
    intent: ActorIntent,
    sea: &MarineWorld<'_>,
    world: &CollisionWorld,
) -> bool {
    if actor.marine.is_none() || sea.environment.is_none() {
        return false;
    }
    let sample = sea.sample(actor.feet);
    breathing(actor, sample);
    if actor.hp <= 0.0 {
        actor.clear_glider();
        return true;
    }
    if actor.free_flight.as_ref().is_some_and(|state| state.active) {
        if let Some(state) = &mut actor.marine {
            state.swim.active = false;
        }
        return false;
    }
    if actor.marine.as_ref().is_some_and(|state| state.boat.active) {
        boat_tick(actor, intent, sea, world);
        return true;
    }
    let surface = match sample {
        OceanSurfaceState::ReadyWet(surface)
            if actor.feet.y < surface.mean_height - 0.05
                && actor.feet.y + actor.dimensions.y > surface.bed_height =>
        {
            surface
        }
        OceanSurfaceState::Unloaded => {
            let request = crate::exploration::ordinary_request(actor, intent);
            telemetry(actor, request, true);
            return true;
        }
        _ => {
            if let Some(state) = &mut actor.marine {
                state.swim.active = false;
            }
            return false;
        }
    };
    crate::glider::fold(actor);
    let entering = actor
        .marine
        .as_ref()
        .is_some_and(|state| !state.swim.active);
    let prior = if entering {
        body_velocity(actor)
    } else {
        actor
            .marine
            .as_ref()
            .map_or(Vec3::ZERO, |state| state.velocity)
    };
    let forward = actor.aim.with_y(0.0).normalize_or(Vec3::NEG_Z);
    let direction = (forward * intent.movement.y + forward.cross(Vec3::Y) * intent.movement.x)
        .clamp_length_max(1.0);
    let vertical = intent.flight_vertical.clamp(-1.0, 1.0);
    // A gentle control-only return to breathing depth; no water-volume solver.
    let neutral = ((surface.height - 0.95 - actor.feet.y) * 2.0).clamp(-0.5, 1.2);
    let mut velocity = prior.lerp(
        direction * 3.5
            + Vec3::Y
                * if vertical.abs() > 0.01 {
                    vertical * 2.5
                } else {
                    neutral
                },
        1.0 - (-2.5 * STEP).exp(),
    );
    if intent.high_jump && actor.body.vertical_velocity > 2.5 {
        velocity.y = actor.body.vertical_velocity;
    }
    velocity += actor.body.impulse_velocity;
    let delta = velocity * STEP;
    let loading = world.needs_terrain(
        actor.feet,
        delta,
        actor.dimensions.y,
        actor.dimensions.x * 0.5,
    );
    telemetry(actor, velocity, loading);
    if loading {
        return true;
    }
    let (feet, contacts) = slide_with_contacts(
        world,
        actor.feet,
        delta,
        actor.dimensions.y,
        actor.dimensions.x * 0.5,
    );
    actor.feet = feet;
    for normal in contacts {
        velocity -= normal * velocity.dot(normal).min(0.0);
    }
    transfer_velocity(actor, velocity);
    actor.grounded = false;
    actor.body.grounded = false;
    if let Some(state) = &mut actor.marine {
        state.swim.active = true;
        state.velocity = velocity;
    }
    true
}

#[cfg(test)]
mod tests;
