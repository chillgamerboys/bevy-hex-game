//! Short body-controller rollout from admitted observations, never live input.

use bevy_math::Vec3;

use crate::collision::CollisionWorld;
use crate::controller::{Body, GroundProfile};
use crate::STEP;

#[derive(Debug)]
pub(crate) struct ForecastMotion {
    pub id: u8,
    initial: Vec3,
    samples: Vec<(f32, Vec3)>,
}

impl ForecastMotion {
    pub fn human(
        id: u8,
        feet: Vec3,
        velocity: Vec3,
        prediction: f32,
        collision: &CollisionWorld,
    ) -> Self {
        let mut result = Self {
            id,
            initial: feet,
            samples: vec![(0.0, feet)],
        };
        let horizon = prediction.clamp(0.0, 0.5);
        let horizontal = velocity.with_y(0.0);
        let speed = horizontal.length();
        let profile = GroundProfile {
            walk: speed,
            run: speed,
            ..Default::default()
        };
        let mut body = Body::default();
        body.vertical_velocity = velocity.y;
        let mut feet = feet;
        let mut elapsed = 0.0;
        while elapsed < horizon {
            let previous = feet;
            // No inferred button presses, buffered jumps or hidden momentum.
            body.tick_profile(&mut feet, horizontal, false, false, collision, profile);
            let next = (elapsed + STEP).min(horizon);
            result
                .samples
                .push((next, previous.lerp(feet, (next - elapsed) / STEP)));
            elapsed = next;
        }
        result
    }

    pub fn feet_at(&self, seconds: f32) -> Vec3 {
        let seconds = seconds.max(0.0);
        for pair in self.samples.windows(2) {
            let [(start, from), (end, to)] = pair else {
                continue;
            };
            if seconds <= *end {
                return from.lerp(*to, (seconds - start) / (end - start));
            }
        }
        self.samples.last().map_or(self.initial, |sample| sample.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::arena::{ArenaTerrainView, ArenaVoxelGeometry};
    use hex_core::{HexCoord, SubstanceId, TilePos};

    fn floor(ceiling: bool) -> CollisionWorld {
        let mut view = ArenaTerrainView::default();
        for coord in HexCoord::ORIGIN.within_radius(10) {
            view.voxels.insert(TilePos::new(coord, 0), SubstanceId(1));
            if ceiling {
                view.voxels.insert(TilePos::new(coord, 4), SubstanceId(1));
            }
        }
        let mut world = CollisionWorld::default();
        world.refresh(&view, ArenaVoxelGeometry::default());
        world
    }

    #[test]
    fn shadow_jump_prediction_matches_rising_apex_falling_and_landing_controller() {
        let world = floor(false);
        for (feet, vertical) in [(0.3, 5.0), (1.3, 0.0), (0.5, -4.0)] {
            let start = Vec3::Y * feet;
            let path = ForecastMotion::human(0, start, Vec3::new(4.5, vertical, 0.0), 0.5, &world);
            let mut body = Body::default();
            body.vertical_velocity = vertical;
            let mut actual = start;
            let mut time = 0.0;
            for _ in 0..60 {
                body.tick(&mut actual, Vec3::X, false, false, &world);
                time += STEP;
                assert!(path.feet_at(time).distance(actual) < 0.0001);
            }
            assert_eq!(path.feet_at(0.5), path.feet_at(8.0));
            if vertical < 0.0 {
                assert!(actual.y.abs() < 0.01, "falling target must land");
            } else {
                assert!(actual.y < feet + vertical * 0.5 + 0.01);
            }
        }
    }

    #[test]
    fn shadow_jump_prediction_hits_ceiling_and_does_not_predict_another_jump() {
        let world = floor(true);
        let path = ForecastMotion::human(0, Vec3::Y * 0.2, Vec3::Y * 6.0, 3.0, &world);
        assert!(path.samples.iter().all(|(_, feet)| world.clear(
            *feet,
            crate::BODY_HEIGHT,
            crate::BODY_RADIUS
        )));
        assert!(
            path.feet_at(0.5).y < 0.05,
            "ceiling contact must stop upward motion"
        );
        let grounded = ForecastMotion::human(0, path.feet_at(0.5), Vec3::ZERO, 0.5, &world);
        assert!(grounded.feet_at(0.5).y < 0.05);
        let observed_next_jump =
            ForecastMotion::human(0, Vec3::Y * 0.05, Vec3::Y * 6.0, 0.5, &world);
        assert!(observed_next_jump.feet_at(0.05).y > grounded.feet_at(0.05).y + 0.1);
    }
    #[test]
    fn shadow_jump_forecast_matches_live_projectile_and_controller_sweeps() {
        use super::super::{advance_shot, forecast_spell_with_motion, projectile, ForecastBody};
        use crate::{Actor, ArenaTuning, Spell, BODY_HEIGHT};

        let collision = floor(false);
        let tuning = ArenaTuning::default();
        let mut caster = Actor::spawn(1, Vec3::new(-4.0, 0.0, 0.0), Vec3::X);
        let mut target = Actor::spawn(0, Vec3::new(4.0, 0.3, 0.0), Vec3::NEG_X);
        let velocity = Vec3::Y * 5.0;
        let motion = ForecastMotion::human(0, target.feet, velocity, 0.5, &collision);
        let speed = 20.0;
        let mut time = 0.4;
        for _ in 0..3 {
            (caster.aim, time) = crate::bot::ballistic_aim(
                caster.eye(),
                motion.feet_at(time) + Vec3::Y * (BODY_HEIGHT * 0.5),
                &tuning,
                speed,
            )
            .expect("reachable predicted human");
        }
        let forecast = forecast_spell_with_motion(
            &caster,
            &[ForecastBody::human(0, target.feet, velocity, 0.5)],
            Some(&motion),
            &collision,
            &ArenaTerrainView::default(),
            ArenaVoxelGeometry::default(),
            &tuning,
            speed,
        )
        .impact
        .expect("forecast impact");
        assert_eq!(forecast.actor, Some(0));
        let mut body = Body::default();
        body.vertical_velocity = velocity.y;
        let mut shot = projectile(&caster, Spell::Fireball, &tuning, 0, speed);
        for _ in 0..60 {
            target.previous_feet = target.feet;
            body.tick(&mut target.feet, Vec3::ZERO, false, false, &collision);
            if let Some(hit) = advance_shot(
                &mut shot,
                &collision,
                &[caster.clone(), target.clone()],
                false,
            ) {
                assert_eq!(hit.actor, forecast.actor);
                assert!(hit.point.distance(forecast.point) < 0.0001);
                assert!((shot.age - forecast.time).abs() < STEP * 0.01);
                return;
            }
        }
        panic!("live shot missed predicted interception");
    }
}
