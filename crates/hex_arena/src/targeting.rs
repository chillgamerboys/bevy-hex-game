//! Bounded hostile sensing. Decisions retain copies, never hidden actor references.

use bevy_math::Vec3;

use crate::collision::CollisionWorld;
use crate::spells::ForecastBody;
use crate::{shapes, Actor, STEP};

#[derive(Debug, Clone, Copy)]
pub(crate) struct ObservedTarget {
    pub body: ForecastBody,
    pub tick: u64,
}

impl ObservedTarget {
    pub fn center(self) -> Vec3 {
        self.body.feet + Vec3::Y * (self.body.dimensions.y * 0.5)
    }

    pub fn distance(self, point: Vec3, seconds: f32) -> f32 {
        let mut body = Actor::spawn(self.body.id, self.body.feet, Vec3::NEG_Z);
        body.species = self.body.species;
        body.dimensions = self.body.dimensions;
        body.body_yaw = self.body.yaw;
        let time = seconds.min(self.body.predict_seconds);
        body.feet += self.body.velocity * time;
        body.body_yaw += self.body.yaw_velocity * time;
        shapes::distance(point, &body)
    }
}

/// Only successful sight admission copies a hostile's physical profile and pose.
/// Velocity comes from dated observations of the same ID, not live motion state.
pub(crate) fn observe(
    observer: &Actor,
    actors: &[Actor],
    previous: &[ObservedTarget],
    collision: &CollisionWorld,
    tick: u64,
    prediction: f32,
) -> Vec<ObservedTarget> {
    let mut visible: Vec<_> = actors
        .iter()
        .filter(|a| a.hp > 0.0 && a.team != observer.team)
        .filter(|a| {
            [a.center(), a.eye()]
                .into_iter()
                .any(|point| collision.sight_clear(observer.eye(), point))
        })
        .map(|actor| {
            let old = previous
                .iter()
                .find(|old| old.body.id == actor.id && tick > old.tick && tick - old.tick <= 25);
            let velocity = old.map_or(Vec3::ZERO, |old| {
                let seconds = f32::from(u16::try_from(tick - old.tick).unwrap_or(25)) * STEP;
                ((actor.feet - old.body.feet) / seconds).clamp_length_max(9.0)
            });
            let yaw_velocity = old.map_or(0.0, |old| {
                let seconds = f32::from(u16::try_from(tick - old.tick).unwrap_or(25)) * STEP;
                let change = (actor.body_yaw - old.body.yaw + std::f32::consts::PI)
                    .rem_euclid(std::f32::consts::TAU)
                    - std::f32::consts::PI;
                (change / seconds).clamp(-6.0, 6.0)
            });
            ObservedTarget {
                body: ForecastBody {
                    id: actor.id,
                    feet: actor.feet,
                    velocity,
                    predict_seconds: prediction,
                    species: actor.species,
                    team: actor.team,
                    dimensions: actor.dimensions,
                    yaw: actor.body_yaw,
                    yaw_velocity,
                },
                tick,
            }
        })
        .collect();
    visible.sort_by(|a, b| {
        a.center()
            .distance_squared(observer.center())
            .total_cmp(&b.center().distance_squared(observer.center()))
            .then_with(|| a.body.id.cmp(&b.body.id))
    });
    visible
}
