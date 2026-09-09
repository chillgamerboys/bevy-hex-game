//! Two short wall-following templates, validated by the real M01 body controller.

use super::*;

#[derive(Debug, Default)]
pub(super) struct Route {
    pub points: Vec<Vec3>,
    directions: Vec<Vec3>,
    pub side: i8,
    pub planned_at: u64,
    pub committed_until: u64,
    pub revision: Option<u64>,
    progress_at: u64,
    progress_feet: Vec3,
    failed_sides: u8,
    retry_after: u64,
}

impl Route {
    pub fn clear(&mut self) {
        self.points.clear();
        self.directions.clear();
    }

    pub fn invalidate(&mut self, bot: &Actor, collision: &CollisionWorld) {
        if self.revision != collision.revision
            || !bot.grounded
            || bot.impulse_velocity().length() > 1.0
        {
            self.clear();
        }
    }

    pub fn hold(&mut self, bot: &Actor, tick: u64) {
        self.progress_at = tick;
        self.progress_feet = bot.feet;
    }

    pub fn travel(&mut self, bot: &Actor, tick: u64, tuning: &crate::BotTuning) -> Option<Vec3> {
        while self.points.first().is_some_and(|point| {
            (point.with_y(0.0) - bot.feet.with_y(0.0)).length() < tuning.waypoint_distance
        }) {
            self.points.remove(0);
            self.directions.remove(0);
        }
        if self.points.is_empty() {
            return None;
        }
        if tick.saturating_sub(self.progress_at) >= u64::from(ticks(tuning.stuck_seconds)) {
            let progress = (bot.feet - self.progress_feet).with_y(0.0).length();
            self.progress_at = tick;
            self.progress_feet = bot.feet;
            if progress < tuning.stuck_distance {
                self.clear();
                self.side = -self.side;
                self.failed_sides += 1;
                self.retry_after = tick + if self.failed_sides >= 2 { 120 } else { 1 };
                self.committed_until = tick + u64::from(ticks(tuning.flank_commit_seconds));
                return None;
            }
            self.failed_sides = 0;
        }
        self.directions.first().copied()
    }

    pub fn plan(
        &mut self,
        bot: &Actor,
        target: Vec3,
        collision: &CollisionWorld,
        tick: u64,
        tuning: &crate::BotTuning,
        tie_side: i8,
    ) -> u64 {
        self.invalidate(bot, collision);
        if !bot.grounded || tick < self.retry_after {
            return 0;
        }
        let interval = ticks(tuning.flank_replan_seconds);
        if !self.points.is_empty()
            && tick.saturating_sub(self.planned_at) < u64::from(interval)
            && self.revision == collision.revision
        {
            return 0;
        }
        // Terrain changes invalidate a route but cannot defeat the CPU budget.
        if tick.saturating_sub(self.planned_at) < u64::from(interval) && self.planned_at != 0 {
            if self.revision != collision.revision {
                self.clear();
            }
            return 0;
        }
        self.planned_at = tick;
        self.revision = collision.revision;
        let mut work = 0;
        let mut choices = Vec::new();
        for side in [-1_i8, 1] {
            if tick < self.committed_until && self.side != 0 && side != self.side {
                continue;
            }
            let mut body = bot.body.clone();
            let mut feet = bot.feet;
            let mut points = Vec::new();
            let mut directions = Vec::new();
            let mut peek = false;
            for _ in 0..4 {
                let eye = feet + Vec3::Y * crate::EYE_HEIGHT;
                let Some(hit) = collision.sweep_sphere(eye, target - eye, 0.0) else {
                    peek = true;
                    break;
                };
                let normal = hit.normal.with_y(0.0).normalize_or_zero();
                if normal.length_squared() < 0.5 {
                    break;
                }
                let tangent = normal.cross(Vec3::Y) * f32::from(side);
                let direction = (tangent - normal * 0.2).normalize_or_zero();
                let start = feet;
                let mut supported = true;
                for step in 0..60 {
                    body.tick(&mut feet, direction, false, false, collision);
                    work += 1;
                    if step % 6 == 0 || step == 59 {
                        supported &= collision.clear(feet, BODY_HEIGHT, BODY_RADIUS)
                            && collision
                                .ground(
                                    feet + Vec3::Y * (SKIN * 2.0),
                                    BODY_HEIGHT,
                                    BODY_RADIUS,
                                    0.42,
                                )
                                .is_some();
                        if !supported {
                            break;
                        }
                    }
                }
                if !supported || (feet - start).with_y(0.0).length() < 0.25 {
                    break;
                }
                points.push(feet);
                directions.push(direction);
                if visible_from(collision, feet + Vec3::Y * crate::EYE_HEIGHT, target) {
                    peek = true;
                    break;
                }
            }
            if !points.is_empty() {
                choices.push((side, points, peek, directions));
            }
        }
        choices.sort_by(|a, b| {
            b.2.cmp(&a.2)
                .then_with(|| {
                    if a.2 {
                        a.1.len().cmp(&b.1.len())
                    } else {
                        // A partial flank should approach the remembered goal,
                        // not win merely by accumulating more lateral travel.
                        let distance = |points: &[Vec3]| {
                            points.last().map_or(f32::INFINITY, |point| {
                                (*point - target).with_y(0.0).length_squared()
                            })
                        };
                        distance(&a.1).total_cmp(&distance(&b.1))
                    }
                })
                .then_with(|| (a.0 != tie_side).cmp(&(b.0 != tie_side)))
        });
        if let Some((side, points, _, directions)) = choices.into_iter().next() {
            self.side = side;
            self.points = points;
            self.directions = directions;
            self.committed_until = tick + u64::from(ticks(tuning.flank_commit_seconds));
            self.progress_at = tick;
            self.progress_feet = bot.feet;
        } else {
            self.clear();
        }
        work
    }
}
