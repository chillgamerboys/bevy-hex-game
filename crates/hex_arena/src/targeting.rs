//! Bounded hostile sensing. Decisions retain copies, never hidden actor references.

use bevy_math::Vec3;

use crate::collision::CollisionWorld;
use crate::spells::ForecastBody;
use crate::{shapes, Actor, STEP};

#[derive(Debug, Clone, Copy)]
pub(crate) struct ObservedTarget {
    pub body: ForecastBody,
    pub tick: u64,
    /// Endpoint admitted at observation time, never reconstructed from hidden state.
    pub sight_point: Vec3,
}

impl ObservedTarget {
    pub fn center(self) -> Vec3 {
        self.body.center()
    }

    pub fn distance(self, point: Vec3, seconds: f32) -> f32 {
        self.body
            .actor_at(seconds)
            .map_or(f32::INFINITY, |body| shapes::distance(point, &body))
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
    if observer.species == crate::Species::Worm && !observer.worm().is_some_and(|s| s.exposed) {
        return Vec::new();
    }
    let mut visible: Vec<_> = actors
        .iter()
        .filter(|a| a.hp > 0.0 && a.team != observer.team)
        .filter(|a| {
            if a.species == crate::Species::Worm {
                return a.body_hex_prisms().any(|p| {
                    collision.sight_clear(
                        observer.eye(),
                        a.feet + p.offset + Vec3::Y * (p.height * 0.5),
                    )
                });
            }
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
                sight_point: if actor.species == crate::Species::Worm {
                    actor
                        .body_hex_prisms()
                        .map(|p| actor.feet + p.offset + Vec3::Y * (p.height * 0.5))
                        .find(|point| collision.sight_clear(observer.eye(), *point))
                        .unwrap_or(actor.eye())
                } else if collision.sight_clear(observer.eye(), actor.center()) {
                    actor.center()
                } else {
                    actor.eye()
                },
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
                    prisms: actor.body_prism_snapshot(),
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
