//! Force-launched projectiles, exact impact arbitration, and radial spell effects.

use std::collections::BTreeSet;

use bevy_math::Vec3;
use hex_core::arena::{ArenaMaterials, ArenaTerrainView, ArenaVoxelGeometry};
use hex_core::{HexCoord, TerrainBatchId, TerrainEdit, TerrainImpact, TilePos};

use crate::collision::{voxel_overlaps_body, CollisionWorld, SKIN};
use crate::{
    Actor, ArenaSession, ArenaTuning, CommandsOut, Preview, Projectile, Spell, VisualEffect,
    BODY_HEIGHT, BODY_RADIUS, STEP,
};

const PROJECTILE_RADIUS: f32 = 0.06;
const MAX_FLIGHT_SECONDS: f32 = 5.0;
const EMERGENCE_SECONDS: f32 = 0.18;

#[derive(Debug, Clone)]
pub(crate) struct ShotParameters {
    gravity: f32,
    radius: f32,
    damage: f32,
    knockback: f32,
    terrain_power: u8,
    wall_dimensions: (i32, i32),
    direction: Vec3,
}

#[derive(Debug)]
pub(crate) struct PendingWall {
    voxels: Vec<TilePos>,
    age: f32,
    center: Vec3,
}

#[derive(Debug, Clone, Copy)]
struct Impact {
    point: Vec3,
    normal: Vec3,
    terrain: bool,
}

fn projectile(actor: &Actor, spell: Spell, tuning: &ArenaTuning, id: u64) -> Projectile {
    Projectile {
        id,
        owner: actor.id,
        position: actor.eye(),
        previous_position: actor.eye(),
        velocity: actor.aim * tuning.projectile_speed,
        spell,
        age: 0.0,
        parameters: ShotParameters {
            gravity: tuning.projectile_gravity,
            radius: tuning.fireball_radius(),
            damage: tuning.fireball_damage,
            knockback: tuning.fireball_knockback,
            terrain_power: tuning.terrain_power,
            wall_dimensions: tuning.shield_dimensions(),
            direction: actor.aim,
        },
        owner_cleared: false,
    }
}

/// Analytic constant-gravity displacement, shared by preview and actual shots.
fn displacement(velocity: Vec3, gravity: f32) -> Vec3 {
    velocity * STEP - Vec3::Y * (0.5 * gravity * STEP * STEP)
}

fn capsule_distance(point: Vec3, feet: Vec3) -> f32 {
    let low = feet.y + BODY_RADIUS;
    let high = feet.y + BODY_HEIGHT - BODY_RADIUS;
    let axis = Vec3::new(feet.x, point.y.clamp(low, high), feet.z);
    (point.distance(axis) - BODY_RADIUS).max(0.0)
}

/// Earliest segment contact with the actor's vertical capsule, expanded by the
/// projectile radius. Both endcap quadratics and the finite cylinder participate.
fn sweep_actor(start: Vec3, delta: Vec3, feet: Vec3) -> Option<f32> {
    sweep_capsule(start, delta, feet, PROJECTILE_RADIUS)
}

fn sweep_capsule(start: Vec3, delta: Vec3, feet: Vec3, extra_radius: f32) -> Option<f32> {
    let radius = BODY_RADIUS + extra_radius;
    let low = feet.y + BODY_RADIUS;
    let high = feet.y + BODY_HEIGHT - BODY_RADIUS;
    let axis_point = Vec3::new(feet.x, start.y.clamp(low, high), feet.z);
    if start.distance_squared(axis_point) <= radius * radius {
        return Some(0.0);
    }
    let mut result: Option<f32> = None;
    let local = start - feet;
    let a = delta.x * delta.x + delta.z * delta.z;
    let b = 2.0 * (local.x * delta.x + local.z * delta.z);
    let c = local.x * local.x + local.z * local.z - radius * radius;
    for t in quadratic_roots(a, b, c).into_iter().flatten() {
        let y = start.y + delta.y * t;
        if (0.0..=1.0).contains(&t) && (low..=high).contains(&y) {
            result = Some(result.map_or(t, |old| old.min(t)));
        }
    }
    for y in [low, high] {
        let relative = start - Vec3::new(feet.x, y, feet.z);
        for t in quadratic_roots(
            delta.length_squared(),
            2.0 * relative.dot(delta),
            relative.length_squared() - radius * radius,
        )
        .into_iter()
        .flatten()
        {
            if (0.0..=1.0).contains(&t) {
                result = Some(result.map_or(t, |old| old.min(t)));
            }
        }
    }
    result
}

pub(super) fn aim_from_camera(
    session: &ArenaSession,
    actor_id: u8,
    origin: Vec3,
    direction: Vec3,
) -> Vec3 {
    let Some(actor) = session.actors.iter().find(|a| a.id == actor_id) else {
        return direction.normalize_or_zero();
    };
    if !origin.is_finite() || !direction.is_finite() || direction.length_squared() < 0.0001 {
        return actor.aim;
    }
    let delta = direction.normalize() * 80.0;
    let mut fraction = session
        .collision
        .sweep_sphere(origin, delta, 0.0)
        .map_or(1.0, |h| h.fraction);
    for target in session
        .actors
        .iter()
        .filter(|a| a.id != actor_id && a.hp > 0.0)
    {
        if let Some(hit) = sweep_capsule(origin, delta, target.feet, 0.0) {
            fraction = fraction.min(hit);
        }
    }
    let aim = (origin + delta * fraction - actor.eye()).normalize_or_zero();
    if aim.length_squared() > 0.5 {
        aim
    } else {
        actor.aim
    }
}

fn quadratic_roots(a: f32, b: f32, c: f32) -> [Option<f32>; 2] {
    if a.abs() < f32::EPSILON {
        return [None, None];
    }
    let discriminant = b * b - 4.0 * a * c;
    if discriminant < 0.0 {
        return [None, None];
    }
    let root = discriminant.sqrt();
    [Some((-b - root) / (2.0 * a)), Some((-b + root) / (2.0 * a))]
}

fn advance_shot(
    shot: &mut Projectile,
    collision: &CollisionWorld,
    actors: &[Actor],
    predict: bool,
) -> Option<Impact> {
    shot.previous_position = shot.position;
    let delta = displacement(shot.velocity, shot.parameters.gravity);
    let mut hit = collision
        .sweep_sphere(shot.position, delta, PROJECTILE_RADIUS)
        .map(|hit| (hit.fraction, hit.normal, true));
    for actor in actors.iter().filter(|a| a.hp > 0.0) {
        if actor.id == shot.owner && !shot.owner_cleared {
            if capsule_distance(shot.position, actor.feet) > PROJECTILE_RADIUS + SKIN {
                shot.owner_cleared = true;
            } else {
                continue;
            }
        }
        let previous_feet = if predict {
            actor.feet
        } else {
            actor.previous_feet
        };
        // Sweep in the moving body's frame, avoiding missed fast cross-traffic.
        let relative_delta = delta - (actor.feet - previous_feet);
        if let Some(fraction) = sweep_actor(shot.position, relative_delta, previous_feet) {
            // Exact ties favor terrain, preserving a closed wall's blocker.
            if hit.is_none_or(|(old, _, _)| fraction < old) {
                let point = shot.position + delta * fraction;
                let center = previous_feet
                    + (actor.feet - previous_feet) * fraction
                    + Vec3::Y * (BODY_HEIGHT * 0.5);
                hit = Some((fraction, (point - center).normalize_or_zero(), false));
            }
        }
    }
    shot.age += STEP;
    shot.velocity -= Vec3::Y * (shot.parameters.gravity * STEP);
    if let Some((fraction, normal, terrain)) = hit {
        shot.position += delta * fraction;
        Some(Impact {
            point: shot.position,
            normal,
            terrain,
        })
    } else {
        shot.position += delta;
        None
    }
}

fn wall_volume(
    impact: Impact,
    direction: Vec3,
    dimensions: (i32, i32),
    world: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    actors: &[Actor],
) -> Vec<TilePos> {
    if !impact.terrain || impact.normal.y < 0.5 {
        return Vec::new();
    }
    // Sphere contact is above the upper face; sample narrowly inside that voxel.
    let Some(support) =
        geometry.voxel_at(impact.point - impact.normal * (PROJECTILE_RADIUS + SKIN * 2.0))
    else {
        return Vec::new();
    };
    let tangent = Vec3::new(direction.z, 0.0, -direction.x).normalize_or_zero();
    let candidates = [(1, 0), (0, 1), (-1, 1), (-1, 0), (0, -1), (1, -1)];
    let Some((dq, dr)) = candidates.into_iter().max_by(|(aq, ar), (bq, br)| {
        HexCoord::from_axial(*aq, *ar)
            .to_world(0.0)
            .dot(tangent)
            .total_cmp(&HexCoord::from_axial(*bq, *br).to_world(0.0).dot(tangent))
    }) else {
        return Vec::new();
    };
    let (width, height) = dimensions;
    let mut volume = BTreeSet::new();
    for offset in -(width / 2)..=width / 2 {
        let coord = HexCoord::from_axial(
            support.coord.x() + dq * offset,
            support.coord.y() + dr * offset,
        );
        if !geometry.contains_column(coord)
            || !world
                .voxels
                .contains_key(&TilePos::new(coord, support.level))
        {
            return Vec::new();
        }
        for rise in 1..=height {
            volume.insert(TilePos::new(coord, support.level + rise));
        }
    }
    let volume: Vec<_> = volume.into_iter().collect();
    if wall_is_clear(&volume, world, geometry, actors) {
        volume
    } else {
        Vec::new()
    }
}

fn wall_is_clear(
    volume: &[TilePos],
    world: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    actors: &[Actor],
) -> bool {
    if volume.is_empty() {
        return false;
    }
    let mut bottoms = BTreeSet::new();
    for pos in volume {
        if !geometry.contains_column(pos.coord)
            || world.voxels.contains_key(pos)
            || actors.iter().filter(|a| a.hp > 0.0).any(|a| {
                voxel_overlaps_body(
                    *pos,
                    geometry,
                    a.feet,
                    BODY_HEIGHT,
                    BODY_RADIUS + SKIN * 4.0,
                )
            })
        {
            return false;
        }
        let below = TilePos::new(pos.coord, pos.level - 1);
        if volume.binary_search(&below).is_err() {
            bottoms.insert(below);
        }
    }
    bottoms
        .into_iter()
        .all(|pos| world.voxels.contains_key(&pos))
}

impl ArenaSession {
    pub(super) fn release(
        &mut self,
        owner: u8,
        spell: Spell,
        tuning: &ArenaTuning,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        materials: ArenaMaterials,
        out: &mut CommandsOut,
    ) {
        let Some(actor) = self.actors.iter().find(|a| a.id == owner) else {
            return;
        };
        if spell == Spell::AreaBlast {
            self.explode(
                actor.center(),
                owner,
                spell,
                tuning.blast_radius(),
                tuning.blast_damage,
                tuning.blast_knockback,
                tuning.terrain_power,
                world,
                geometry,
                materials,
                out,
            );
        } else {
            let shot = projectile(actor, spell, tuning, self.next_projectile);
            self.next_projectile += 1;
            self.projectiles.push(shot);
        }
    }

    pub(super) fn advance_projectiles(
        &mut self,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        materials: ArenaMaterials,
        out: &mut CommandsOut,
    ) {
        let mut survivors = Vec::new();
        for mut shot in std::mem::take(&mut self.projectiles) {
            if let Some(impact) = advance_shot(&mut shot, &self.collision, &self.actors, false) {
                if shot.spell == Spell::Shield {
                    let volume = wall_volume(
                        impact,
                        shot.parameters.direction,
                        shot.parameters.wall_dimensions,
                        world,
                        geometry,
                        &self.actors,
                    );
                    if volume.is_empty() {
                        self.notice = "Shield fizzled: needs a clear, supported footprint.".into();
                    } else {
                        self.pending_walls.push(PendingWall {
                            voxels: volume,
                            age: 0.0,
                            center: impact.point,
                        });
                        self.effects.push(VisualEffect {
                            center: impact.point,
                            radius: 0.4,
                            age: 0.0,
                            lifetime: EMERGENCE_SECONDS,
                            kind: Spell::Shield,
                        });
                    }
                } else {
                    self.explode(
                        impact.point,
                        shot.owner,
                        shot.spell,
                        shot.parameters.radius,
                        shot.parameters.damage,
                        shot.parameters.knockback,
                        shot.parameters.terrain_power,
                        world,
                        geometry,
                        materials,
                        out,
                    );
                }
            } else if shot.age < MAX_FLIGHT_SECONDS && shot.position.y > -10.0 {
                survivors.push(shot);
            } else if shot.spell == Spell::Shield {
                self.notice = "Shield fizzled beyond the arena.".into();
            }
        }
        self.projectiles = survivors;
    }

    pub(super) fn advance_walls(
        &mut self,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        materials: ArenaMaterials,
        out: &mut CommandsOut,
    ) {
        let mut survivors = Vec::new();
        let mut reserved = BTreeSet::new();
        for mut wall in std::mem::take(&mut self.pending_walls) {
            wall.age += STEP;
            if !wall_is_clear(&wall.voxels, world, geometry, &self.actors)
                || wall.voxels.iter().any(|p| reserved.contains(p))
            {
                self.notice = "Shield fizzled: its emerging footprint became blocked.".into();
                continue;
            }
            reserved.extend(wall.voxels.iter().copied());
            if wall.age + STEP * 0.01 >= EMERGENCE_SECONDS {
                for pos in wall.voxels {
                    out.edits.push(TerrainEdit::Set {
                        pos,
                        substance: materials.stone,
                    });
                }
                self.shields_raised += 1;
                self.effects.push(VisualEffect {
                    center: wall.center,
                    radius: 0.65,
                    age: 0.0,
                    lifetime: 0.25,
                    kind: Spell::Shield,
                });
            } else {
                survivors.push(wall);
            }
        }
        self.pending_walls = survivors;
    }

    fn explode(
        &mut self,
        center: Vec3,
        owner: u8,
        spell: Spell,
        radius: f32,
        damage: f32,
        knockback: f32,
        power: u8,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        materials: ArenaMaterials,
        out: &mut CommandsOut,
    ) {
        for actor in &mut self.actors {
            if actor.hp <= 0.0 || (spell == Spell::AreaBlast && actor.id == owner) {
                continue;
            }
            let distance = capsule_distance(center, actor.feet);
            if distance >= radius {
                continue;
            }
            // No LOS query: radial effects intentionally pass through cover.
            let falloff = (1.0 - distance / radius).clamp(0.0, 1.0);
            actor.hp = (actor.hp - damage * falloff).max(0.0);
            let away = actor.center() - center;
            let outward = if away.length_squared() > SKIN * SKIN {
                away.normalize()
            } else {
                Vec3::Y
            };
            let impulse = (outward + Vec3::Y * 0.35).normalize_or_zero() * knockback * falloff;
            actor.body.impulse_velocity += impulse;
            if impulse.y > 0.0 {
                actor.body.grounded = false;
            }
        }
        let volume = geometry.sphere(world, center, radius);
        if !volume.is_empty() {
            let impact = TerrainImpact {
                batch: TerrainBatchId(self.next_impact),
                volume,
                element: materials.fire,
                power,
            };
            self.next_impact += 1;
            self.pending_impacts.insert(impact.batch, impact.clone());
            out.impacts.push(impact);
        }
        self.effects.push(VisualEffect {
            center,
            radius,
            age: 0.0,
            lifetime: 0.45,
            kind: spell,
        });
    }
}

pub(super) fn preview(
    session: &ArenaSession,
    world: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    tuning: &ArenaTuning,
) -> Preview {
    let Some(actor) = session.actors.iter().find(|a| a.id == 0) else {
        return Preview::default();
    };
    if actor.selected == Spell::AreaBlast {
        return Preview {
            impact: Some(actor.center()),
            valid: true,
            ..Default::default()
        };
    }
    let mut shot = projectile(actor, actor.selected, tuning, 0);
    let mut result = Preview {
        points: vec![shot.position],
        ..Default::default()
    };
    for tick in 0..600 {
        let impact = advance_shot(&mut shot, &session.collision, &session.actors, true);
        if tick % 4 == 0 || impact.is_some() {
            result.points.push(shot.position);
        }
        if let Some(impact) = impact {
            result.impact = Some(impact.point);
            if shot.spell == Spell::Shield {
                result.wall_voxels = wall_volume(
                    impact,
                    shot.parameters.direction,
                    shot.parameters.wall_dimensions,
                    world,
                    geometry,
                    &session.actors,
                );
                result.valid = !result.wall_voxels.is_empty();
            } else {
                result.valid = true;
            }
            break;
        }
        if shot.position.y < -10.0 {
            break;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ballistic_step_matches_analytic_parabola_and_downhill_has_more_reach() {
        let tuning = ArenaTuning::default();
        let actor = Actor::spawn(0, Vec3::ZERO, Vec3::X);
        let mut shot = projectile(&actor, Spell::Fireball, &tuning, 0);
        let start = shot.position;
        for _ in 0..120 {
            assert!(advance_shot(&mut shot, &CollisionWorld::default(), &[], false).is_none());
        }
        assert!((shot.position.x - start.x - 32.0).abs() < 0.001);
        assert!((shot.position.y - start.y + 6.0).abs() < 0.001);
        let reach = |height: f32| {
            (2.0 * height / tuning.projectile_gravity).sqrt() * tuning.projectile_speed
        };
        assert!(reach(5.0) > reach(1.0) * 2.0);
    }

    #[test]
    fn capsule_sweep_detects_thin_fast_and_vertical_hits_without_box_corners() {
        let hit = sweep_actor(Vec3::new(-10.0, 0.4, 0.0), Vec3::X * 20.0, Vec3::ZERO)
            .expect("fast body hit");
        assert!((hit - (10.0 - BODY_RADIUS - PROJECTILE_RADIUS) / 20.0).abs() < 0.0001);
        assert!(sweep_actor(Vec3::new(0.29, 0.79, -2.0), Vec3::Z * 4.0, Vec3::ZERO).is_none());
        assert!(sweep_actor(Vec3::Y * 10.0, Vec3::NEG_Y * 20.0, Vec3::ZERO).is_some());
    }
}
