//! Bounded creature travel using the production body, including confirmed landings.

use super::*;
use bevy_math::Quat;

/// Frozen pre-recovery movement for the accepted Shadow's patrol/home phases.
/// Active Shadow combat already returns through its unchanged Bot controller.
#[derive(Debug)]
pub(super) struct ShadowTravel {
    direction: Vec3,
    decided: u64,
    revision: Option<u64>,
    last_feet: Vec3,
}

impl ShadowTravel {
    pub fn new(home: Vec3) -> Self {
        Self {
            direction: Vec3::ZERO,
            decided: 0,
            revision: None,
            last_feet: home,
        }
    }

    pub fn travel(
        &mut self,
        actor: &Actor,
        desired: Vec3,
        flight: bool,
        stop: bool,
        world: &CollisionWorld,
        view: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        tuning: &EncounterTuning,
        tick: u64,
    ) -> Vec3 {
        if self.revision != world.revision
            || tick.saturating_sub(self.decided) >= 24
            || actor.feet.distance(self.last_feet) > 2.0
        {
            self.direction = Vec3::ZERO;
            if desired.length_squared() >= 0.001 {
                let mut best = f32::NEG_INFINITY;
                for angle in [0.0, 0.65, -0.65, 1.3, -1.3] {
                    let direction = Quat::from_rotation_y(angle) * desired;
                    let mut body = actor.clone();
                    let mut safe = true;
                    for _ in 0..12 {
                        motion::tick(&mut body, direction, true, false, flight, world, tuning);
                        if !shapes::clear(world, &body, body.feet, body.body_yaw)
                            || (!flight
                                && (!dry(&body, view, geometry)
                                    || body.feet.y < actor.feet.y - 0.45))
                        {
                            safe = false;
                            break;
                        }
                    }
                    let moved = body.feet - actor.feet;
                    let score = moved.dot(desired.normalize_or_zero()) - 0.05 * angle.abs();
                    if safe && moved.length() > 0.02 && score > best {
                        best = score;
                        self.direction = direction;
                    }
                }
            }
            self.decided = tick + u64::from(actor.id % 4);
            self.revision = world.revision;
            self.last_feet = actor.feet;
        }
        if stop {
            self.direction = Vec3::ZERO;
        }
        self.direction
    }
}

#[derive(Debug)]
struct JumpRoute {
    direction: Vec3,
    expected: Actor,
    remaining: u16,
    revision: Option<u64>,
    airborne: bool,
}

#[derive(Debug, Default)]
pub(super) struct Steering {
    direction: Vec3,
    next_decision: u64,
    next_recovery: u64,
    progress_feet: Vec3,
    progress_tick: u64,
    initialized: bool,
    revision: Option<u64>,
    jump: Option<JumpRoute>,
}

impl Steering {
    pub fn jumping(&self) -> bool {
        self.jump.is_some()
    }

    pub fn blocked_ticks(&self, tick: u64) -> u64 {
        tick.saturating_sub(self.progress_tick)
    }

    pub fn hold(&mut self, actor: &Actor, tick: u64) {
        self.direction = Vec3::ZERO;
        self.next_decision = tick;
        self.progress_tick = tick;
        self.progress_feet = actor.feet;
        self.jump = None;
    }

    pub fn travel(
        &mut self,
        actor: &Actor,
        desired: Vec3,
        flight: bool,
        run: bool,
        world: &CollisionWorld,
        view: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        tuning: &EncounterTuning,
        tick: u64,
    ) -> (Vec3, bool) {
        let progress = actor.feet - self.progress_feet;
        if !self.initialized
            || progress.dot(desired.normalize_or_zero()) > 0.15
            || (!flight && progress.y > 0.15)
        {
            self.progress_tick = tick;
            self.progress_feet = actor.feet;
            if !self.initialized {
                self.next_recovery = tick + 48 + u64::from(actor.id % 12) * 5;
            }
            self.initialized = true;
        }
        if let Some(mut route) = self.jump.take() {
            let matches = route.revision == world.revision
                && actor.feet.distance(route.expected.feet) < 0.15
                && actor
                    .body
                    .impulse_velocity
                    .distance(route.expected.body.impulse_velocity)
                    < 0.2
                && (actor.body.vertical_velocity - route.expected.body.vertical_velocity).abs()
                    < 0.2;
            if matches && (!route.airborne || !actor.grounded) && route.remaining > 0 {
                let mut next = actor.clone();
                motion::tick(
                    &mut next,
                    route.direction,
                    true,
                    false,
                    false,
                    world,
                    tuning,
                );
                if volume_safe(&next, world, view, geometry) {
                    route.airborne |= !next.grounded;
                    route.expected = next;
                    route.remaining -= 1;
                    let direction = route.direction;
                    self.jump = Some(route);
                    return (direction, false);
                }
            }
            self.next_decision = tick;
        }
        if desired.length_squared() < 0.001 {
            self.hold(actor, tick);
            return (Vec3::ZERO, false);
        }
        if tick >= self.next_decision || self.revision != world.revision {
            self.direction = steer(actor, desired, flight, run, world, view, geometry, tuning);
            self.next_decision = tick + 24;
            self.revision = world.revision;
        }
        if !flight
            && run
            && matches!(actor.species, Species::Goblin | Species::Shaman)
            && actor.grounded
            && self.blocked_ticks(tick) >= 48
            && tick >= self.next_recovery
        {
            self.next_recovery = tick + 60 + u64::from(actor.id % 5);
            let descent = descent_route(actor, desired, world, view, geometry, tuning);
            let route = descent
                .map(|(direction, duration)| (direction, duration, false))
                .or_else(|| {
                    jump_route(actor, desired, world, view, geometry, tuning)
                        .map(|(direction, duration)| (direction, duration, true))
                });
            if let Some((direction, duration, jump)) = route {
                let mut next = actor.clone();
                motion::tick(&mut next, direction, true, jump, false, world, tuning);
                let airborne = !next.grounded;
                self.jump = Some(JumpRoute {
                    direction,
                    expected: next,
                    remaining: duration.saturating_sub(1),
                    revision: world.revision,
                    airborne,
                });
                return (direction, jump);
            }
            // A supported sideways/backward excursion can reveal the next
            // reachable intermediate ledge. It never teleports or alters terrain.
            self.direction = detour(actor, desired, run, world, view, geometry, tuning);
            self.next_decision = tick + 48;
        }
        // Recheck the actual next step after impulse, separation, terrain change,
        // or a cached direction reaching a cliff. A short fall is not support.
        let mut next = actor.clone();
        motion::tick(&mut next, self.direction, run, false, flight, world, tuning);
        if !(if flight {
            flight_step_safe(actor, &next, world, view, geometry)
        } else {
            volume_safe(&next, world, view, geometry) && supported(&next, world)
        }) {
            self.direction = Vec3::ZERO;
            self.next_decision = tick;
        }
        (self.direction, false)
    }
}

pub(super) fn contained(actor: &Actor, geometry: ArenaVoxelGeometry) -> bool {
    if matches!(actor.species, Species::Wisp | Species::Worm) {
        let low = geometry.top(TilePos::new(HexCoord::ORIGIN, geometry.min_level))
            - geometry.level_height;
        let high = geometry.top(TilePos::new(HexCoord::ORIGIN, geometry.max_level));
        if actor.feet.y < low || actor.feet.y + actor.dimensions.y > high {
            return false;
        }
    }
    if matches!(
        actor.species,
        Species::Golem | Species::Wisp | Species::Worm
    ) {
        return shapes::compound_contained(actor, geometry);
    }
    // The convex hull of resident column centers lies inside the scalloped
    // hex union. Constraining the complete body to this conservative interior
    // also rejects an edge crossing outside between two resident corners.
    let half = actor.dimensions * 0.5;
    let axes = [
        Vec3::Z,
        Vec3::new(0.866_025_4, 0.0, 0.5),
        Vec3::new(0.866_025_4, 0.0, -0.5),
    ];
    let limit = f64::from(geometry.radius) * 1.5;
    [-1.0, 1.0].into_iter().all(|x| {
        [-1.0, 1.0].into_iter().all(|z| {
            let point = actor.feet + actor.body_rotation() * Vec3::new(x * half.x, 0.0, z * half.z);
            axes.into_iter()
                .all(|axis| f64::from(point.dot(axis).abs()) <= limit)
        })
    })
}

fn volume_safe(
    actor: &Actor,
    world: &CollisionWorld,
    view: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
) -> bool {
    contained(actor, geometry)
        && shapes::clear(world, actor, actor.feet, actor.body_yaw)
        && dry(actor, view, geometry)
}

fn supported(actor: &Actor, world: &CollisionWorld) -> bool {
    shapes::ground(world, actor, actor.feet, 0.4 + SKIN * 8.0).is_some()
}

fn flight_supported(actor: &Actor, world: &CollisionWorld) -> bool {
    shapes::ground(world, actor, actor.feet + Vec3::Y * 4.0, 16.0).is_some()
}

fn flight_step_safe(
    previous: &Actor,
    next: &Actor,
    world: &CollisionWorld,
    view: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
) -> bool {
    if contained(previous, geometry) {
        return volume_safe(next, world, view, geometry) && flight_supported(next, world);
    }
    // Real knockback is not erased by an AI boundary. Once pushed outside,
    // permit clear inward progress even before the complete long body reenters.
    shapes::clear(world, next, next.feet, next.body_yaw)
        && dry(next, view, geometry)
        && (contained(next, geometry)
            || next.feet.with_y(0.0).length_squared() < previous.feet.with_y(0.0).length_squared())
}

pub(super) fn flight_goal(
    actor: &Actor,
    desired: Vec3,
    world: &CollisionWorld,
    view: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    tuning: &EncounterTuning,
) -> Option<Vec3> {
    let mut body = actor.clone();
    let floor = shapes::ground(world, &body, desired + Vec3::Y * 8.0, 20.0)?;
    if actor.species == Species::Wisp {
        let cruise = tuning.wisp_cruise_height
            + f32::from(actor.flight_layer.unwrap_or(0)) * tuning.wisp_layer_spacing;
        // The same layer follows local support. A low ceiling admits a lower
        // physically clear altitude, without changing collision or melee rules.
        return (0_u8..5).find_map(|step| {
            body.feet = floor + Vec3::Y * (cruise - f32::from(step) * 0.8).max(0.4);
            volume_safe(&body, world, view, geometry).then_some(body.feet)
        });
    }
    body.feet = floor + Vec3::Y * tuning.dragon_cruise_height;
    volume_safe(&body, world, view, geometry).then_some(body.feet)
}

pub(super) fn retreat_goal(
    actor: &Actor,
    threat: Vec3,
    world: &CollisionWorld,
    view: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    tuning: &EncounterTuning,
) -> Option<Vec3> {
    let away = (actor.center() - threat).with_y(0.0).normalize_or(Vec3::Z);
    [0.0, 0.8, -0.8, 1.6, -1.6, std::f32::consts::PI]
        .into_iter()
        .filter_map(|angle| {
            flight_goal(
                actor,
                actor.feet + Quat::from_rotation_y(angle) * away * 5.0,
                world,
                view,
                geometry,
                tuning,
            )
        })
        .max_by(|a, b| {
            let score = |p: Vec3| {
                p.distance(threat)
                    + if world.sight_clear(threat, p + Vec3::Y * 0.2) {
                        0.0
                    } else {
                        3.0
                    }
            };
            score(*a).total_cmp(&score(*b))
        })
}

fn steer(
    actor: &Actor,
    desired: Vec3,
    flight: bool,
    run: bool,
    world: &CollisionWorld,
    view: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    tuning: &EncounterTuning,
) -> Vec3 {
    let mut best = (f32::NEG_INFINITY, Vec3::ZERO);
    for angle in [0.0, 0.65, -0.65, 1.3, -1.3] {
        let direction = Quat::from_rotation_y(angle) * desired;
        if let Some(body) = walk_route(
            actor, direction, flight, 12, run, world, view, geometry, tuning,
        ) {
            let moved = body.feet - actor.feet;
            let score = moved.dot(desired.normalize_or_zero()) - 0.05 * angle.abs();
            if moved.length() > 0.02 && score > best.0 {
                best = (score, direction);
            }
        }
    }
    best.1
}

fn walk_route(
    actor: &Actor,
    direction: Vec3,
    flight: bool,
    ticks: u16,
    run: bool,
    world: &CollisionWorld,
    view: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    tuning: &EncounterTuning,
) -> Option<Actor> {
    let mut body = actor.clone();
    for _ in 0..ticks {
        let previous = body.clone();
        motion::tick(&mut body, direction, run, false, flight, world, tuning);
        if !(if flight {
            flight_step_safe(&previous, &body, world, view, geometry)
        } else {
            volume_safe(&body, world, view, geometry) && supported(&body, world)
        }) {
            return None;
        }
    }
    Some(body)
}

fn descent_route(
    actor: &Actor,
    desired: Vec3,
    world: &CollisionWorld,
    view: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    tuning: &EncounterTuning,
) -> Option<(Vec3, u16)> {
    let forward = desired.with_y(0.0).normalize_or_zero();
    // A downward ledge needs a proved landing, not an upward jump. Three
    // directions share the existing staggered recovery deadline and 120-tick cap.
    for angle in [0.0, 0.65, -0.65] {
        let direction = Quat::from_rotation_y(angle) * forward;
        let mut body = actor.clone();
        let mut airborne = false;
        for tick in 0_u16..120 {
            motion::tick(&mut body, direction, true, false, false, world, tuning);
            if !volume_safe(&body, world, view, geometry) {
                break;
            }
            airborne |= !body.grounded;
            if airborne && body.grounded {
                let moved = body.feet - actor.feet;
                if moved.y < -0.4 - SKIN * 8.0
                    && moved.dot(forward) > 0.35
                    && supported(&body, world)
                {
                    return Some((direction, tick + 1));
                }
                break;
            }
        }
    }
    None
}

fn jump_route(
    actor: &Actor,
    desired: Vec3,
    world: &CollisionWorld,
    view: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    tuning: &EncounterTuning,
) -> Option<(Vec3, u16)> {
    let forward = desired.with_y(0.0).normalize_or_zero();
    let mut best = None;
    for angle in [0.0, 0.65, -0.65, 1.3, -1.3, std::f32::consts::PI] {
        let direction = Quat::from_rotation_y(angle) * forward;
        let mut body = actor.clone();
        for tick in 0_u16..120 {
            motion::tick(&mut body, direction, true, tick == 0, false, world, tuning);
            if !volume_safe(&body, world, view, geometry) {
                break;
            }
            if tick > 2 && body.grounded {
                let moved = body.feet - actor.feet;
                let score = moved.dot(forward) + moved.y * 2.0 - angle.abs() * 0.1;
                if moved.with_y(0.0).length() > 0.35
                    && moved.y >= -0.4 - SKIN * 8.0
                    && supported(&body, world)
                    && best.is_none_or(|(s, _, _)| score > s)
                {
                    best = Some((score, direction, tick + 1));
                }
                break;
            }
        }
    }
    best.map(|(_, direction, duration)| (direction, duration))
}

fn detour(
    actor: &Actor,
    desired: Vec3,
    run: bool,
    world: &CollisionWorld,
    view: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    tuning: &EncounterTuning,
) -> Vec3 {
    let mut best = (f32::NEG_INFINITY, Vec3::ZERO);
    for angle in [
        std::f32::consts::FRAC_PI_2,
        -std::f32::consts::FRAC_PI_2,
        std::f32::consts::PI,
    ] {
        let direction = Quat::from_rotation_y(angle) * desired.with_y(0.0).normalize_or_zero();
        if let Some(body) = walk_route(
            actor, direction, false, 48, run, world, view, geometry, tuning,
        ) {
            let moved = body.feet - actor.feet;
            let score = moved.length() + moved.dot(desired.normalize_or_zero()) * 0.25;
            if moved.length() > 0.3 && score > best.0 {
                best = (score, direction);
            }
        }
    }
    best.1
}
