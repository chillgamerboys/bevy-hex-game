//! Bounded escape routes using ordinary movement and the shared High Jump ability.

use super::*;
use hex_core::{HexCoord, TilePos};
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};

/// Limits for optional Shadow crater recovery through shared movement abilities.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EscapeTuning {
    /// Permit validated walking, normal jumping and High Jump exits.
    pub enabled: bool,
    /// Time without vertical or exit progress before starting recovery.
    pub stuck_seconds: f32,
    /// Maximum horizontal extent of the local search.
    pub radius: f32,
    /// Minimum interval between complete planning passes.
    pub replan_seconds: f32,
    /// Maximum simulated duration of one movement candidate.
    pub rollout_seconds: f32,
    /// Maximum duration of one recovery attempt.
    pub attempt_seconds: f32,
}

impl Default for EscapeTuning {
    fn default() -> Self {
        Self {
            enabled: true,
            stuck_seconds: 0.4,
            radius: 6.0,
            replan_seconds: 0.5,
            rollout_seconds: 2.0,
            attempt_seconds: 6.0,
        }
    }
}

impl EscapeTuning {
    /// Reject nonfinite and unbounded local search values.
    pub fn validate(&self) -> Result<(), String> {
        for (name, value, min, max) in [
            ("stuck_seconds", self.stuck_seconds, 0.2, 2.0),
            ("radius", self.radius, 2.0, 6.0),
            ("replan_seconds", self.replan_seconds, 0.5, 3.0),
            ("rollout_seconds", self.rollout_seconds, 0.25, 2.0),
            ("attempt_seconds", self.attempt_seconds, 1.0, 6.0),
        ] {
            if !value.is_finite() || !(min..=max).contains(&value) {
                return Err(format!(
                    "bot.escape.{name} must be finite and in {min}..={max}."
                ));
            }
        }
        Ok(())
    }
}

const DIRECTIONS: [Vec3; 6] = [
    Vec3::X,
    Vec3::new(0.5, 0.0, 0.866_025_4),
    Vec3::new(-0.5, 0.0, 0.866_025_4),
    Vec3::NEG_X,
    Vec3::new(-0.5, 0.0, -0.866_025_4),
    Vec3::new(0.5, 0.0, -0.866_025_4),
];

#[derive(Debug)]
struct Travel {
    target: Vec3,
    jump: bool,
    boost: bool,
    remaining: u16,
    expected: Actor,
    revision: Option<u64>,
}

#[derive(Debug)]
struct Search {
    body: Actor,
    revision: u64,
    candidate: usize,
    rims: [Option<Vec3>; 6],
}

#[derive(Debug, Default)]
pub(super) struct EscapeRecovery {
    suspected: Option<(u64, Vec3)>,
    started: Option<u64>,
    next_probe: u64,
    next_plan: u64,
    search: Option<Search>,
    route: Option<Travel>,
    suppressed: Option<(Vec3, u64)>,
    suppression_revision: Option<u64>,
    suppression_capability: Option<(bool, u32)>,
    reason: &'static str,
    pub rollout_ticks: u64,
    pub attempts: u32,
    pub escapes: u32,
}

impl EscapeRecovery {
    pub fn reason(&self) -> &'static str {
        self.reason
    }

    /// Reset movement plans on pause; no cast state is owned or changed here.
    pub fn cancel_charge(&mut self) {
        self.search = None;
        self.route = None;
    }

    /// Supplies movement and jump edges only; the caller retains combat input.
    pub fn intent(
        &mut self,
        bot: &Actor,
        collision: &CollisionWorld,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        tuning: &ArenaTuning,
        tick: u64,
    ) -> Option<ActorIntent> {
        let policy = &tuning.bot.escape;
        if !policy.enabled || bot.hp <= 0.0 {
            self.finish(false);
            return None;
        }
        if self.started.is_none() {
            if tick < self.next_probe {
                return None;
            }
            self.next_probe = tick + 12;
            if let Some((at, signature)) = self.suppressed {
                let unchanged = self.suppression_revision == Some(world.revision)
                    || signature == local_signature(world, geometry, at, policy.radius);
                self.suppression_revision = Some(world.revision);
                let same_capability = self.suppression_capability
                    == Some((
                        ready(bot, Spell::HighJump),
                        tuning.high_jump_height.to_bits(),
                    ));
                if unchanged
                    && same_capability
                    && at.with_y(0.0).distance(bot.feet.with_y(0.0)) < 2.0
                {
                    return None;
                }
                self.suppressed = None;
            }
            let floor = depression_floor(bot, collision, policy.radius)?;
            let rims = nearby_rims(bot, floor, collision, world, geometry, policy.radius);
            if rims.iter().flatten().count() < 4 {
                self.suspected = None;
                return None;
            }
            let falling =
                !bot.grounded && bot.impulse_velocity().y < -0.5 && bot.feet.y > floor.y + 0.5;
            if !bot.grounded && !falling {
                return None;
            }
            let (since, at) = *self.suspected.get_or_insert((tick, bot.feet));
            if bot.grounded
                && (bot.feet.y > at.y + 0.3
                    || bot.feet.with_y(0.0).distance(at.with_y(0.0)) > policy.radius)
            {
                self.suspected = Some((tick, bot.feet));
                return None;
            }
            if !falling && tick.saturating_sub(since) < u64::from(ticks(policy.stuck_seconds)) {
                return None;
            }
            self.started = Some(tick);
            self.attempts += 1;
            self.next_plan = tick + u64::from(ticks(policy.replan_seconds));
            self.search = Some(Search {
                body: bot.clone(),
                revision: world.revision,
                candidate: if falling { 12 } else { 0 },
                rims,
            });
            self.reason = "escape_search";
        }
        if self.started.is_some_and(|start| {
            tick.saturating_sub(start) >= u64::from(ticks(policy.attempt_seconds))
        }) {
            self.fail(bot, world, geometry, tuning);
            return None;
        }
        if let Some(route) = self.route.take() {
            let matches = route.revision == collision.revision
                && route.expected.feet.distance(bot.feet) < 0.12
                && route
                    .expected
                    .impulse_velocity()
                    .distance(bot.impulse_velocity())
                    < 0.25;
            if matches && route.remaining == 0 && bot.grounded {
                self.escapes += 1;
                self.finish(true);
                return None;
            }
            if matches && route.remaining > 0 {
                return Some(self.advance_route(bot, route, collision, world, geometry, tuning));
            }
            self.reason = "escape_revalidate";
            self.search = None;
        }
        if let Some(mut search) = self.search.take() {
            // Falling changes the start every tick; each candidate forecasts from
            // the actual body while retaining the same bounded directional pass.
            let moved = bot.grounded && search.body.feet.distance(bot.feet) > 0.12;
            if search.revision == world.revision && !moved {
                let index = search.candidate;
                search.candidate += 1;
                if let Some(route) = self.test_route(
                    bot,
                    index,
                    search.rims.get(index % 6).copied().flatten(),
                    collision,
                    world,
                    geometry,
                    tuning,
                ) {
                    self.reason = if route.boost {
                        "escape_high_jump"
                    } else if route.jump {
                        "escape_jump"
                    } else {
                        "escape_walk"
                    };
                    return Some(
                        self.advance_route(bot, route, collision, world, geometry, tuning),
                    );
                }
                if search.candidate >= 18 {
                    self.fail(bot, world, geometry, tuning);
                    return None;
                }
                self.search = Some(search);
                return Some(neutral(bot));
            }
            self.reason = "escape_revalidate";
        }
        if tick >= self.next_plan {
            self.next_plan = tick + u64::from(ticks(policy.replan_seconds));
            if let Some(floor) = depression_floor(bot, collision, policy.radius) {
                self.search = Some(Search {
                    body: bot.clone(),
                    revision: world.revision,
                    candidate: if bot.grounded { 0 } else { 12 },
                    rims: nearby_rims(bot, floor, collision, world, geometry, policy.radius),
                });
            }
        }
        Some(neutral(bot))
    }

    fn finish(&mut self, escaped: bool) {
        self.started = None;
        self.suspected = None;
        self.cancel_charge();
        self.reason = if escaped {
            "escape_complete"
        } else {
            "escape_off"
        };
    }

    fn fail(
        &mut self,
        bot: &Actor,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        tuning: &ArenaTuning,
    ) {
        self.suppressed = Some((
            bot.feet,
            local_signature(world, geometry, bot.feet, tuning.bot.escape.radius),
        ));
        self.suppression_revision = Some(world.revision);
        self.suppression_capability = Some((
            ready(bot, Spell::HighJump),
            tuning.high_jump_height.to_bits(),
        ));
        self.finish(false);
        self.reason = "escape_no_safe_route";
    }

    fn test_route(
        &mut self,
        bot: &Actor,
        index: usize,
        rim: Option<Vec3>,
        collision: &CollisionWorld,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        tuning: &ArenaTuning,
    ) -> Option<Travel> {
        let direction = *DIRECTIONS.get(index % 6)?;
        let jump = (6..12).contains(&index);
        let boost = index >= 12;
        if (!boost && !bot.grounded) || (boost && !ready(bot, Spell::HighJump)) {
            return None;
        }
        let target = if index < 6 {
            let endpoint = bot.feet + direction * (tuning.bot.escape.radius - 0.2) + Vec3::Y * 0.3;
            collision.ground(endpoint, BODY_HEIGHT, BODY_RADIUS, 0.6)?
        } else {
            rim?
        };
        let mut next = bot.clone();
        if boost {
            next.body.boost(tuning.high_jump_height);
            next.grounded = false;
        }
        let duration = ticks(tuning.bot.escape.rollout_seconds);
        for step in 0..duration {
            let travel = steer(next.feet, target);
            crate::motion::tick(
                &mut next,
                travel,
                true,
                jump && step == 0,
                false,
                collision,
                &tuning.encounters,
            );
            self.rollout_ticks += 1;
            if !safe(&next, collision, world, geometry)
                || next.feet.with_y(0.0).distance(bot.feet.with_y(0.0)) > tuning.bot.escape.radius
            {
                return None;
            }
            if next.grounded && step > 2 {
                if next.feet.with_y(0.0).distance(target.with_y(0.0)) < 0.3
                    && (next.feet.y - target.y).abs() < 0.2
                {
                    return Some(Travel {
                        target,
                        jump,
                        boost,
                        remaining: step + 1,
                        expected: bot.clone(),
                        revision: collision.revision,
                    });
                }
                if jump || boost {
                    return None;
                }
            }
        }
        None
    }

    fn advance_route(
        &mut self,
        bot: &Actor,
        mut route: Travel,
        collision: &CollisionWorld,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        tuning: &ArenaTuning,
    ) -> ActorIntent {
        let mut next = bot.clone();
        let direction = steer(bot.feet, route.target);
        if route.boost {
            next.body.boost(tuning.high_jump_height);
            next.grounded = false;
        }
        crate::motion::tick(
            &mut next,
            direction,
            true,
            route.jump,
            false,
            collision,
            &tuning.encounters,
        );
        if !safe(&next, collision, world, geometry) {
            self.reason = "escape_route_blocked";
            return neutral(bot);
        }
        let mut intent = neutral(bot);
        let forward = bot.aim.with_y(0.0).normalize_or(Vec3::NEG_Z);
        intent.movement = Vec2::new(
            direction.dot(forward.cross(Vec3::Y)),
            direction.dot(forward),
        );
        intent.run = true;
        intent.jump = route.jump;
        intent.high_jump = route.boost;
        route.jump = false;
        route.boost = false;
        route.remaining = route.remaining.saturating_sub(1);
        route.expected = next;
        self.route = Some(route);
        intent
    }
}

fn neutral(bot: &Actor) -> ActorIntent {
    ActorIntent {
        aim: bot.aim,
        ..Default::default()
    }
}

fn steer(feet: Vec3, target: Vec3) -> Vec3 {
    let delta = (target - feet).with_y(0.0);
    if delta.length_squared() < 0.15 * 0.15 {
        Vec3::ZERO
    } else {
        delta.normalize_or_zero()
    }
}

fn depression_floor(bot: &Actor, collision: &CollisionWorld, radius: f32) -> Option<Vec3> {
    if bot.grounded {
        Some(bot.feet)
    } else {
        collision.ground(bot.feet, BODY_HEIGHT, BODY_RADIUS, radius)
    }
}

fn safe(
    body: &Actor,
    collision: &CollisionWorld,
    view: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
) -> bool {
    collision.clear(body.feet, BODY_HEIGHT, BODY_RADIUS)
        && [
            Vec3::ZERO,
            Vec3::X * BODY_RADIUS,
            Vec3::NEG_X * BODY_RADIUS,
            Vec3::Z * BODY_RADIUS,
            Vec3::NEG_Z * BODY_RADIUS,
        ]
        .into_iter()
        .all(|offset| {
            let coord = HexCoord::from_world(body.feet + offset);
            geometry.contains_column(coord)
                && !view.liquids.iter().any(|span| {
                    span.bottom.coord == coord
                        && geometry.top(TilePos::new(coord, span.top_level)) > body.feet.y + SKIN
                        && geometry.top(span.bottom) - geometry.level_height
                            < body.feet.y + BODY_HEIGHT
                })
        })
}

fn nearby_rims(
    bot: &Actor,
    floor: Vec3,
    collision: &CollisionWorld,
    view: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    radius: f32,
) -> [Option<Vec3>; 6] {
    DIRECTIONS.map(|direction| {
        let mut best: Option<Vec3> = None;
        for fraction in [0.2, 0.35, 0.5, 0.7, 0.85, 1.0] {
            let point = floor + direction * (radius * fraction) + Vec3::Y * radius;
            let Some(footing) = collision.ground(point, BODY_HEIGHT, BODY_RADIUS, radius - 0.3)
            else {
                continue;
            };
            let mut body = bot.clone();
            body.feet = footing;
            if footing.y > floor.y + 0.3
                && safe(&body, collision, view, geometry)
                && best.is_none_or(|old| footing.y < old.y - SKIN)
            {
                best = Some(footing);
            }
        }
        best
    })
}

fn local_signature(
    view: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    center: Vec3,
    radius: f32,
) -> u64 {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    for pos in geometry.sphere(view, center, radius + 2.0) {
        if let Some(substance) = view.voxels.get(&pos) {
            pos.hash(&mut hash);
            substance.hash(&mut hash);
        }
    }
    hash.finish()
}

#[cfg(test)]
mod tests;
