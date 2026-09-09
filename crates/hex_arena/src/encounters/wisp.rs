//! Slow hovering ranged profile. All attack inputs are admitted observations.

use super::*;
use crate::bot::ballistic_aim_with_gravity;
use crate::spells::{forecast_creature_projectile, CreatureProjectileSpec};
use bevy_math::Quat;
use hex_core::TerrainDamageKind;

/// An own-sighting copy; cover pressure never receives a live hidden body.
#[derive(Debug, Clone, Copy)]
pub(super) enum EmberTarget {
    Visible(Knowledge),
    Cover(Knowledge),
}

pub(super) fn ember_spec(c: &EncounterTuning, materials: ArenaMaterials) -> CreatureProjectileSpec {
    CreatureProjectileSpec {
        ability: CreatureAbility::WispEmber,
        appearance: ProjectileAppearance::Ember,
        speed: c.wisp_ember_speed,
        gravity: c.wisp_ember_gravity,
        collision_radius: c.wisp_ember_collision_radius,
        splash_radius: c.wisp_ember_radius,
        damage: c.wisp_ember_damage,
        knockback: c.wisp_ember_knockback,
        terrain_kind: TerrainDamageKind::Elemental(materials.fire),
        terrain_power: c.wisp_ember_terrain_power,
    }
}

/// Recheck after caster motion and incoming hits, using only a dated sight copy.
/// The exact production projectile sweep admits the shot and its self-splash.
pub(super) fn release_aim(
    actor: &Actor,
    target: EmberTarget,
    collision: &CollisionWorld,
    world: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    tuning: &ArenaTuning,
    materials: ArenaMaterials,
    tick: u64,
) -> Option<Vec3> {
    let c = &tuning.encounters;
    let (seen, cover) = match target {
        EmberTarget::Visible(seen) => (seen, false),
        EmberTarget::Cover(seen) => (seen, true),
    };
    let age = elapsed(tick, seen.tick);
    if !seen.direct || age > if cover { c.wisp_memory_seconds } else { 0.2 } {
        return None;
    }
    let sight_point = seen
        .observed
        .map_or(seen.point + Vec3::Y * 0.4, |o| o.sight_point);
    if actor.eye().distance(sight_point) > c.wisp_preferred_max {
        return None;
    }
    if cover {
        // A finite shot chips the intervening terrain, never a guessed live body.
        // Once this line opens, seek/reacquire instead of firing into empty space.
        if collision.sight_clear(actor.eye(), sight_point) {
            return None;
        }
        let spec = ember_spec(c, materials);
        let (aim, _) =
            ballistic_aim_with_gravity(actor.eye(), sight_point, spec.gravity, spec.speed)?;
        let hit = forecast_creature_projectile(actor, aim, spec, &[], collision, world, geometry)
            .impact?;
        return (hit.actor.is_none()
            && hit.barrier.is_none()
            && shapes::distance(hit.point, actor) > spec.splash_radius + 0.1
            && hit.point.distance(actor.eye())
                < sight_point.distance(actor.eye()) + spec.splash_radius)
            .then_some(aim);
    }
    if !collision.sight_clear(actor.eye(), sight_point) {
        return None;
    }
    let mut fact = seen.observed?.body;
    let advance = age.min(fact.predict_seconds);
    fact.feet += fact.velocity * advance;
    fact.yaw += fact.yaw_velocity * advance;
    fact.predict_seconds = (fact.predict_seconds - advance).max(0.0);
    let spec = ember_spec(c, materials);
    let center = fact.center();
    let (_, time) = ballistic_aim_with_gravity(actor.eye(), center, spec.gravity, spec.speed)?;
    let point = center + fact.velocity * time.min(fact.predict_seconds);
    let (aim, _) = ballistic_aim_with_gravity(actor.eye(), point, spec.gravity, spec.speed)?;
    let forecast =
        forecast_creature_projectile(actor, aim, spec, &[fact], collision, world, geometry);
    let hit = forecast.impact?;
    if hit.barrier.is_some() || shapes::distance(hit.point, actor) <= spec.splash_radius + 0.1 {
        return None;
    }
    let target = targeting::ObservedTarget {
        body: fact,
        tick,
        sight_point,
    };
    (target.distance(hit.point, hit.time) <= spec.splash_radius * 0.6).then_some(aim)
}

/// Five short terrain-only candidate headings, refreshed at the behavior cadence.
/// The range band is a preference: close visible shots are legal and useful.
pub(super) fn positioning_goal(
    actor: &Actor,
    target: Option<Vec3>,
    search: Vec3,
    collision: &CollisionWorld,
    world: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    tuning: &EncounterTuning,
) -> Vec3 {
    let heading = target
        .map_or(search - actor.feet, |point| {
            let delta = point - actor.center();
            let range = delta.with_y(0.0).length();
            if range < tuning.wisp_preferred_min {
                -delta.with_y(0.0)
            } else if range > tuning.wisp_preferred_max {
                delta.with_y(0.0)
            } else {
                Vec3::ZERO
            }
        })
        .with_y(0.0)
        .normalize_or_zero();
    if heading.length_squared() > SKIN * SKIN {
        for angle in [0.0, 0.6, -0.6, 1.2, -1.2] {
            let desired = actor.feet + Quat::from_rotation_y(angle) * heading * 3.0;
            if let Some(goal) =
                steering::flight_goal(actor, desired, collision, world, geometry, tuning)
            {
                return goal;
            }
        }
    }
    steering::flight_goal(actor, actor.feet, collision, world, geometry, tuning)
        .unwrap_or(actor.feet)
}

/// Nearby allied public poses separate requested travel, never teleport bodies.
/// Different reserved layers do not pull each other toward a shared altitude.
pub(super) fn spaced_direction(actor: &Actor, actors: &[Actor], desired: Vec3) -> Vec3 {
    let mut direction = desired.clamp_length_max(1.0);
    for ally in actors
        .iter()
        .filter(|a| a.id != actor.id && a.team == actor.team && a.hp > 0.0)
    {
        let separation = actor.center() - ally.center();
        if separation.y.abs() >= (actor.dimensions.y + ally.dimensions.y) * 0.5 + 0.1 {
            continue;
        }
        let flat = separation.with_y(0.0);
        let distance = flat.length();
        if distance < 2.2 && distance > SKIN {
            direction += flat / distance * (2.2 - distance) / 2.2;
        }
    }
    direction.clamp_length_max(1.0)
}
