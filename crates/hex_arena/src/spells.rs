//! Force-launched projectiles, exact impact arbitration, and radial spell effects.

use std::collections::BTreeSet;

use bevy_math::Vec3;
use hex_core::arena::{ArenaMaterials, ArenaTerrainView, ArenaVoxelGeometry};
use hex_core::{HexCoord, TerrainBatchId, TerrainEdit, TerrainImpact, TilePos};

use crate::collision::{CollisionWorld, SKIN};
use crate::{
    Actor, ArenaSession, ArenaTuning, CombatCueKind, CommandsOut, Preview, Projectile, Spell,
    VisualEffect, BODY_HEIGHT, BODY_RADIUS, STEP,
};
#[cfg(test)]
use hex_core::arena::ARENA_MAX_LEVEL;

const PROJECTILE_RADIUS: f32 = 0.06;
pub(super) const MAX_FLIGHT_SECONDS: f32 = 8.0;
pub(super) const EMERGENCE_SECONDS: f32 = 0.18;

#[derive(Debug, Clone)]
pub(crate) struct ShotParameters {
    gravity: f32,
    radius: f32,
    damage: f32,
    knockback: f32,
    terrain_power: u8,
    wall_dimensions: (i32, i32),
    shield_push: f32,
    direction: Vec3,
    team: crate::TeamId,
    min_y: f32,
}

#[derive(Debug)]
pub(crate) struct PendingWall {
    pub(super) voxels: Vec<TilePos>,
    pub(super) age: f32,
    candidates: Vec<TilePos>,
    center: Vec3,
}

#[derive(Debug, Clone, Copy)]
struct Impact {
    point: Vec3,
    normal: Vec3,
    actor: Option<u8>,
    barrier: Option<u64>,
}

fn projectile(
    actor: &Actor,
    spell: Spell,
    tuning: &ArenaTuning,
    id: u64,
    launch_speed: f32,
) -> Projectile {
    Projectile {
        id,
        owner: actor.id,
        position: actor.eye(),
        previous_position: actor.eye(),
        velocity: actor.aim * launch_speed,
        spell,
        age: 0.0,
        parameters: ShotParameters {
            gravity: tuning.projectile_gravity,
            radius: tuning.fireball_radius(),
            damage: if actor.species == crate::Species::Shaman {
                tuning.encounters.shaman_fireball_damage * actor.damage_multiplier
            } else {
                tuning.fireball_damage * actor.damage_multiplier
            },
            knockback: tuning.fireball_knockback,
            terrain_power: tuning.terrain_power,
            wall_dimensions: tuning.shield_dimensions(),
            shield_push: tuning.shield_push,
            direction: actor.aim,
            team: actor.team,
            min_y: -10.0,
        },
        owner_cleared: false,
    }
}

/// Analytic constant-gravity displacement, shared by preview and actual shots.
fn displacement(velocity: Vec3, gravity: f32) -> Vec3 {
    velocity * STEP - Vec3::Y * (0.5 * gravity * STEP * STEP)
}

fn flight_active(shot: &Projectile) -> bool {
    shot.age + STEP * 0.01 < MAX_FLIGHT_SECONDS && shot.position.y > shot.parameters.min_y
}

pub(super) fn capsule_distance(point: Vec3, feet: Vec3) -> f32 {
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
    sweep_capsule_dimensions(start, delta, feet, extra_radius, BODY_HEIGHT, BODY_RADIUS)
}

fn sweep_capsule_dimensions(
    start: Vec3,
    delta: Vec3,
    feet: Vec3,
    extra_radius: f32,
    height: f32,
    body_radius: f32,
) -> Option<f32> {
    let radius = body_radius + extra_radius;
    let low = feet.y + body_radius;
    let high = feet.y + height - body_radius;
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
        if let Some(hit) = if target.species == crate::Species::Dragon {
            crate::shapes::sweep_dragon(origin, delta, target, true, 0.0).map(|h| h.fraction)
        } else {
            sweep_capsule_dimensions(
                origin,
                delta,
                target.feet,
                0.0,
                target.dimensions.y,
                target.dimensions.x * 0.5,
            )
        } {
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
        .attack_sweep(shot.position, delta, PROJECTILE_RADIUS)
        .map(|(hit, barrier)| (hit.fraction, hit.normal, None, barrier));
    for actor in actors.iter().filter(|a| a.hp > 0.0) {
        if actor.id != shot.owner && actor.team == shot.parameters.team {
            continue;
        }
        if actor.id == shot.owner && !shot.owner_cleared {
            if crate::shapes::distance(shot.position, actor) > PROJECTILE_RADIUS + SKIN {
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
        let body_hit = if actor.species == crate::Species::Dragon {
            crate::shapes::sweep_dragon(shot.position, delta, actor, predict, PROJECTILE_RADIUS)
                .map(|h| h.fraction)
        } else if matches!(
            actor.species,
            crate::Species::Human | crate::Species::Shadow | crate::Species::Shaman
        ) {
            sweep_actor(shot.position, relative_delta, previous_feet)
        } else {
            sweep_capsule_dimensions(
                shot.position,
                relative_delta,
                previous_feet,
                PROJECTILE_RADIUS,
                actor.dimensions.y,
                actor.dimensions.x * 0.5,
            )
        };
        if let Some(fraction) = body_hit {
            // Exact ties favor terrain, preserving a closed wall's blocker.
            if hit.is_none_or(|(old, _, _, _)| fraction < old) {
                let point = shot.position + delta * fraction;
                let feet = previous_feet + (actor.feet - previous_feet) * fraction;
                let radius = actor.dimensions.x * 0.5;
                let axis = if actor.species == crate::Species::Dragon {
                    let center = feet + Vec3::Y * actor.dimensions.y * 0.5;
                    let local = actor.body_rotation().inverse() * (point - center);
                    center
                        + actor.body_rotation()
                            * local.clamp(-actor.dimensions * 0.5, actor.dimensions * 0.5)
                } else {
                    Vec3::new(
                        feet.x,
                        point
                            .y
                            .clamp(feet.y + radius, feet.y + actor.dimensions.y - radius),
                        feet.z,
                    )
                };
                hit = Some((
                    fraction,
                    (point - axis).normalize_or_zero(),
                    Some(actor.id),
                    None,
                ));
            }
        }
    }
    shot.age += STEP;
    shot.velocity -= Vec3::Y * (shot.parameters.gravity * STEP);
    if let Some((fraction, normal, actor, barrier)) = hit {
        shot.position += delta * fraction;
        Some(Impact {
            point: shot.position,
            normal,
            actor,
            barrier,
        })
    } else {
        shot.position += delta;
        None
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "the finite anchor level is bounded before integer conversion"
)]
fn wall_candidates(
    impact: Impact,
    direction: Vec3,
    dimensions: (i32, i32),
    geometry: ArenaVoxelGeometry,
) -> Vec<TilePos> {
    // Impact.point is the projectile center. Anchor at its physical contact,
    // choosing the free side of a hex face and the level above an exact top face.
    let contact = impact.point - impact.normal * PROJECTILE_RADIUS;
    let horizontal_normal = Vec3::new(impact.normal.x, 0.0, impact.normal.z);
    let sample = contact + horizontal_normal * (SKIN * 2.0) + Vec3::Y * (SKIN * 2.0);
    if !sample.is_finite()
        || sample.abs().max_element() > 10_000.0
        || !geometry.level_height.is_finite()
        || geometry.level_height <= 0.0
    {
        return Vec::new();
    }
    let level = ((sample.y - geometry.vertical_offset) / geometry.level_height).ceil();
    if !(-100_000.0..=100_000.0).contains(&level) {
        return Vec::new();
    }
    // The anchor may be outside the arena while part of its width reaches in.
    // Admit bounds per candidate cell, not for the anchor as a whole.
    let base = TilePos::new(HexCoord::from_world(sample), level as i32);
    let tangent = Vec3::new(direction.z, 0.0, -direction.x).normalize_or_zero();
    let directions = [(1, 0), (0, 1), (-1, 1), (-1, 0), (0, -1), (1, -1)];
    let Some((dq, dr)) = directions.into_iter().max_by(|(aq, ar), (bq, br)| {
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
        let coord =
            HexCoord::from_axial(base.coord.x() + dq * offset, base.coord.y() + dr * offset);
        for rise in 0..height {
            volume.insert(TilePos::new(coord, base.level + rise));
        }
    }
    volume.into_iter().collect()
}

fn available_wall_voxels(
    candidates: &[TilePos],
    world: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    actors: &[Actor],
    reserved: &BTreeSet<TilePos>,
) -> Vec<TilePos> {
    candidates
        .iter()
        .copied()
        .filter(|pos| {
            geometry.contains_column(pos.coord)
                && (geometry.min_level..=geometry.max_level).contains(&pos.level)
                && !world.voxels.contains_key(pos)
                && !world.edit_protected.get(&pos.coord).is_some_and(|ranges| {
                    ranges
                        .iter()
                        .any(|(low, high)| (*low..=*high).contains(&pos.level))
                })
                && !world.static_spans.iter().any(|v| {
                    v.bottom.coord == pos.coord
                        && (v.bottom.level..=v.top_level).contains(&pos.level)
                })
                && !reserved.contains(pos)
                && !actors
                    .iter()
                    .filter(|actor| actor.hp > 0.0)
                    .any(|actor| crate::shapes::voxel_overlap(*pos, geometry, actor))
        })
        .collect()
}

fn wall_volume(
    impact: Impact,
    direction: Vec3,
    dimensions: (i32, i32),
    world: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    actors: &[Actor],
) -> Vec<TilePos> {
    available_wall_voxels(
        &wall_candidates(impact, direction, dimensions, geometry),
        world,
        geometry,
        actors,
        &BTreeSet::new(),
    )
}

impl ArenaSession {
    pub(super) fn release(
        &mut self,
        owner: u8,
        spell: Spell,
        tuning: &ArenaTuning,
        launch_speed: f32,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        materials: ArenaMaterials,
        out: &mut CommandsOut,
    ) {
        if self.encounter.initialized {
            self.refresh_support_buffs(tuning);
        }
        let Some(actor) = self.actors.iter().find(|a| a.id == owner).cloned() else {
            return;
        };
        self.combat_cue(owner, actor.eye(), CombatCueKind::Release);
        self.record_cast(owner, spell);
        if spell == Spell::AreaBlast {
            self.explode(
                actor.center(),
                owner,
                actor.team,
                spell,
                tuning.blast_radius(),
                tuning.blast_damage * actor.damage_multiplier,
                tuning.blast_knockback,
                tuning.terrain_power,
                world,
                geometry,
                materials,
                out,
            );
        } else {
            let mut shot = projectile(&actor, spell, tuning, self.next_projectile, launch_speed);
            shot.parameters.min_y = self.collision.min_y.min(-10.0);
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
                self.combat_cue(shot.owner, impact.point, CombatCueKind::Impact);
                if let Some(id) = impact.barrier {
                    if shot.spell == Spell::Fireball {
                        if let Some(barrier) =
                            self.encounter.barriers.iter_mut().find(|b| b.id == id)
                        {
                            barrier.hp = (barrier.hp - shot.parameters.damage).max(0.0);
                        }
                        self.collision.sync_barriers(&self.encounter.barriers);
                    } else {
                        continue;
                    }
                }
                if shot.spell == Spell::Shield {
                    if let Some(actor) = impact
                        .actor
                        .and_then(|id| self.actors.iter_mut().find(|actor| actor.id == id))
                    {
                        let incoming = Vec3::new(
                            shot.parameters.direction.x,
                            0.0,
                            shot.parameters.direction.z,
                        );
                        let away = Vec3::new(-impact.normal.x, 0.0, -impact.normal.z);
                        let direction = if incoming.length_squared() > SKIN * SKIN {
                            incoming.normalize()
                        } else if away.length_squared() > SKIN * SKIN {
                            away.normalize()
                        } else {
                            Vec3::X
                        };
                        // Once per seed hit, without HP damage or teleporting.
                        // Ordinary swept movement resolves the impulse next tick.
                        actor.body.impulse_velocity += direction * shot.parameters.shield_push;
                    }
                    let candidates = wall_candidates(
                        impact,
                        shot.parameters.direction,
                        shot.parameters.wall_dimensions,
                        geometry,
                    );
                    if candidates.is_empty() {
                        self.shield_no_room_notice();
                    } else {
                        let voxels = available_wall_voxels(
                            &candidates,
                            world,
                            geometry,
                            &self.actors,
                            &BTreeSet::new(),
                        );
                        self.pending_walls.push(PendingWall {
                            voxels,
                            age: 0.0,
                            candidates,
                            center: impact.point - impact.normal * PROJECTILE_RADIUS,
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
                        shot.parameters.team,
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
            } else if flight_active(&shot) {
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
            wall.voxels =
                available_wall_voxels(&wall.candidates, world, geometry, &self.actors, &reserved);
            reserved.extend(wall.voxels.iter().copied());
            // Announced explosions settle before creation. Recheck the fixed
            // mask against the resulting occupancy, including actors that have
            // moved during emergence; a blocked cell never cancels its neighbors.
            if wall.age + STEP * 0.01 >= EMERGENCE_SECONDS && self.pending_impacts.is_empty() {
                if wall.voxels.is_empty() {
                    self.shield_no_room_notice();
                    continue;
                }
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
        owner_team: crate::TeamId,
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
        let mut damage_events = Vec::new();
        let mut useful_fireball = false;
        for actor in &mut self.actors {
            if actor.hp <= 0.0
                || (spell == Spell::AreaBlast && actor.id == owner)
                || (actor.id != owner && actor.team == owner_team)
            {
                continue;
            }
            let distance = crate::shapes::distance(center, actor);
            if distance >= radius {
                continue;
            }
            // No LOS query: radial effects intentionally pass through cover.
            let falloff = (1.0 - distance / radius).clamp(0.0, 1.0);
            useful_fireball |= actor.id != owner && damage > 0.0 && falloff >= 0.4;
            let removed = actor.hp.min(damage * falloff);
            actor.hp -= removed;
            damage_events.push((actor.id, removed));
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
        for (victim, removed) in damage_events {
            self.record_damage(owner, victim, removed);
        }
        if spell == Spell::Fireball {
            self.record_fireball_impact(owner, useful_fireball);
        }
        let volume = geometry.sphere(world, center, radius);
        if !volume.is_empty() {
            let impact = TerrainImpact {
                batch: TerrainBatchId(self.next_impact),
                volume,
                kind: hex_core::TerrainDamageKind::Elemental(materials.fire),
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
    preview_actor(
        actor,
        &session.actors,
        &session.collision,
        world,
        geometry,
        tuning,
        tuning.launch_speed(actor.charge().map_or(0.0, |charge| charge.elapsed)),
    )
}

/// Shared stationary-body forecast for human assistance and bot shot admission.
pub(super) fn preview_actor(
    actor: &Actor,
    actors: &[Actor],
    collision: &CollisionWorld,
    world: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    tuning: &ArenaTuning,
    launch_speed: f32,
) -> Preview {
    if actor.selected == Spell::AreaBlast {
        return Preview {
            impact: Some(actor.center()),
            valid: true,
            ..Default::default()
        };
    }
    let mut shot = projectile(actor, actor.selected, tuning, 0, launch_speed);
    shot.parameters.min_y = collision.min_y.min(-10.0);
    let mut result = Preview {
        points: vec![shot.position],
        ..Default::default()
    };
    let mut tick = 0_u16;
    while flight_active(&shot) {
        let impact = advance_shot(&mut shot, collision, actors, true);
        if tick.is_multiple_of(4) || impact.is_some() {
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
                    actors,
                );
                result.valid = !result.wall_voxels.is_empty();
            } else {
                result.valid = true;
            }
            break;
        }
        tick += 1;
    }
    result
}

/// Deliberate observed body or memory hypothesis, never a reference to hidden state.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ForecastBody {
    pub id: u8,
    pub feet: Vec3,
    pub velocity: Vec3,
    pub predict_seconds: f32,
    pub species: crate::Species,
    pub team: crate::TeamId,
    pub dimensions: Vec3,
    pub yaw: f32,
    pub yaw_velocity: f32,
}
impl ForecastBody {
    pub fn human(id: u8, feet: Vec3, velocity: Vec3, predict_seconds: f32) -> Self {
        Self {
            id,
            feet,
            velocity,
            predict_seconds,
            species: crate::Species::Human,
            team: u8::from(id != 0),
            dimensions: Vec3::new(BODY_RADIUS * 2.0, BODY_HEIGHT, BODY_RADIUS * 2.0),
            yaw: 0.0,
            yaw_velocity: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ForecastImpact {
    pub point: Vec3,
    pub actor: Option<u8>,
    pub barrier: Option<u64>,
    pub time: f32,
}

#[derive(Debug, Default)]
pub(crate) struct SpellForecast {
    pub impact: Option<ForecastImpact>,
    pub wall_voxels: Vec<TilePos>,
}

/// The production sweep with explicitly supplied knowledge and bounded motion lead.
/// The caster is always included so owner clearance and self-hit rules stay intact.
pub(crate) fn forecast_spell(
    caster: &Actor,
    observed: &[ForecastBody],
    collision: &CollisionWorld,
    world: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    tuning: &ArenaTuning,
    launch_speed: f32,
) -> SpellForecast {
    if caster.selected == Spell::AreaBlast {
        return SpellForecast {
            impact: Some(ForecastImpact {
                point: caster.center(),
                actor: None,
                barrier: None,
                time: 0.0,
            }),
            ..Default::default()
        };
    }
    let mut bodies = vec![caster.clone()];
    bodies.extend(
        observed
            .iter()
            .filter(|body| body.id != caster.id)
            .map(|fact| {
                let mut body = Actor::spawn(fact.id, fact.feet, Vec3::NEG_Z);
                body.species = fact.species;
                body.team = fact.team;
                body.dimensions = fact.dimensions;
                body.body_yaw = fact.yaw;
                body.previous_yaw = fact.yaw;
                body
            }),
    );
    if let Some(owner) = bodies.first_mut() {
        owner.previous_feet = owner.feet;
    }
    let mut shot = projectile(caster, caster.selected, tuning, 0, launch_speed);
    shot.parameters.min_y = collision.min_y.min(-10.0);
    while flight_active(&shot) {
        for body in bodies.iter_mut().skip(1) {
            if let Some(fact) = observed.iter().find(|fact| fact.id == body.id) {
                body.previous_feet = body.feet;
                body.previous_yaw = body.body_yaw;
                body.body_yaw = fact.yaw
                    + fact.yaw_velocity * (shot.age + STEP).min(fact.predict_seconds.max(0.0));
                body.feet = fact.feet
                    + fact.velocity * (shot.age + STEP).min(fact.predict_seconds.max(0.0));
            }
        }
        if let Some(hit) = advance_shot(&mut shot, collision, &bodies, false) {
            return SpellForecast {
                impact: Some(ForecastImpact {
                    point: hit.point,
                    actor: hit.actor,
                    barrier: hit.barrier,
                    time: shot.age,
                }),
                wall_voxels: if caster.selected == Spell::Shield {
                    wall_volume(
                        hit,
                        shot.parameters.direction,
                        shot.parameters.wall_dimensions,
                        world,
                        geometry,
                        &bodies,
                    )
                } else {
                    Vec::new()
                },
            };
        }
    }
    SpellForecast::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collision::voxel_overlaps_body;

    #[test]
    fn ballistic_step_matches_analytic_parabola_and_downhill_has_more_reach() {
        let tuning = ArenaTuning::default();
        let actor = Actor::spawn(0, Vec3::ZERO, Vec3::X);
        let mut shot = projectile(&actor, Spell::Fireball, &tuning, 0, tuning.projectile_speed);
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

    #[test]
    fn nearest_contact_wins_between_terrain_and_actors_in_either_order() {
        let geometry = ArenaVoxelGeometry::default();
        let view = ArenaTerrainView {
            voxels: (0..=5)
                .map(|level| {
                    (
                        TilePos::new(HexCoord::from_axial(2, 0), level),
                        hex_core::SubstanceId(1),
                    )
                })
                .collect(),
            ..Default::default()
        };
        let mut world = CollisionWorld::default();
        world.refresh(&view, geometry);
        let shooter = Actor::spawn(0, Vec3::ZERO, Vec3::X);
        let behind = Actor::spawn(1, Vec3::X * 5.0, Vec3::NEG_X);
        let mut shot = projectile(&shooter, Spell::Fireball, &ArenaTuning::default(), 0, 32.0);
        shot.velocity = Vec3::X * 1200.0;
        let contact = advance_shot(&mut shot, &world, std::slice::from_ref(&behind), false)
            .expect("the wall must be hit before the distant body");
        assert!(contact.actor.is_none() && contact.point.x < 3.0);
        let near = Actor::spawn(2, Vec3::X * 1.5, Vec3::NEG_X);
        let mut shot = projectile(&shooter, Spell::Fireball, &ArenaTuning::default(), 0, 32.0);
        shot.velocity = Vec3::X * 1200.0;
        let contact = advance_shot(&mut shot, &world, &[behind, near], false).expect("nearer body");
        assert!(contact.actor == Some(2) && contact.point.x < 1.5);
    }

    #[test]
    fn relative_sweep_hits_an_actor_crossing_between_tick_endpoints() {
        let shooter = Actor::spawn(0, Vec3::ZERO, Vec3::X);
        let mut target = Actor::spawn(1, Vec3::new(5.0, 0.0, 2.0), Vec3::NEG_X);
        target.previous_feet = Vec3::new(5.0, 0.0, -2.0);
        let mut shot = projectile(&shooter, Spell::Fireball, &ArenaTuning::default(), 0, 32.0);
        shot.velocity = Vec3::X * 1200.0;
        let contact = advance_shot(&mut shot, &CollisionWorld::default(), &[target], false)
            .expect("moving actor crosses the ray");
        assert!(contact.actor == Some(1) && (4.0..5.0).contains(&contact.point.x));
    }

    fn shield_impact(contact: Vec3, normal: Vec3) -> Impact {
        Impact {
            point: contact + normal * PROJECTILE_RADIUS,
            normal,
            actor: None,
            barrier: None,
        }
    }

    fn shield_materials() -> ArenaMaterials {
        ArenaMaterials {
            stone: hex_core::SubstanceId(1),
            bedrock: hex_core::SubstanceId(2),
            grass: hex_core::SubstanceId(3),
            dirt: hex_core::SubstanceId(4),
            fire: hex_core::ElementId(1),
        }
    }

    fn staged_wall(candidates: Vec<TilePos>) -> PendingWall {
        PendingWall {
            voxels: candidates.clone(),
            candidates,
            age: EMERGENCE_SECONDS - STEP,
            center: Vec3::ZERO,
        }
    }

    #[test]
    fn shields_accept_floor_wall_ceiling_and_actor_contact_at_the_same_height() {
        let geometry = ArenaVoxelGeometry::default();
        let world = ArenaTerrainView::default();
        let contact = Vec3::Y * 1.2;
        for normal in [Vec3::Y, Vec3::NEG_Y, Vec3::X] {
            let impact = shield_impact(contact, normal);
            let volume = wall_volume(impact, Vec3::X, (3, 4), &world, geometry, &[]);
            assert_eq!(volume.len(), 12);
            assert!(volume.iter().all(|pos| (4..=7).contains(&pos.level)));
            let actor_hit = wall_volume(
                Impact {
                    actor: Some(1),
                    barrier: None,
                    ..impact
                },
                Vec3::X,
                (3, 4),
                &world,
                geometry,
                &[],
            );
            assert_eq!(actor_hit, volume);
        }
    }

    #[test]
    fn wall_face_contact_anchors_on_the_free_neighbor_and_floor_preserves_height() {
        let geometry = ArenaVoxelGeometry::default();
        let mut world = ArenaTerrainView::default();
        world
            .voxels
            .insert(TilePos::new(HexCoord::ORIGIN, 4), shield_materials().stone);
        let face = hex_core::config::HEX_SMALL_DIAMETER * 0.5;
        let volume = wall_volume(
            shield_impact(Vec3::new(face, 1.4, 0.0), Vec3::X),
            Vec3::NEG_X,
            (1, 3),
            &world,
            geometry,
            &[],
        );
        assert_eq!(volume.len(), 3);
        assert!(volume
            .iter()
            .all(|pos| pos.coord == HexCoord::from_axial(1, 0)));
        let floor = wall_volume(
            shield_impact(Vec3::ZERO, Vec3::Y),
            Vec3::X,
            (5, 5),
            &world,
            geometry,
            &[],
        );
        assert_eq!(floor.len(), 24, "only the pre-existing cell is skipped");
        assert!(floor.iter().all(|pos| (1..=5).contains(&pos.level)));
    }

    #[test]
    fn shield_skips_only_occupied_and_out_of_bounds_cells_without_support() {
        let geometry = ArenaVoxelGeometry::default();
        let impact = shield_impact(Vec3::ZERO, Vec3::Y);
        let mut world = ArenaTerrainView::default();
        let complete = wall_volume(impact, Vec3::X, (3, 4), &world, geometry, &[]);
        let occupied = *complete.first().expect("wall candidate");
        world.voxels.insert(occupied, shield_materials().dirt);
        let partial = wall_volume(impact, Vec3::X, (3, 4), &world, geometry, &[]);
        assert_eq!(partial.len(), 11);
        assert!(!partial.contains(&occupied));
        let top = geometry.top(TilePos::new(HexCoord::ORIGIN, ARENA_MAX_LEVEL - 2));
        let clipped = wall_volume(
            shield_impact(Vec3::Y * top, Vec3::Y),
            Vec3::X,
            (3, 4),
            &world,
            geometry,
            &[],
        );
        assert_eq!(clipped.len(), 6);
        assert!(clipped.iter().all(|pos| pos.level <= ARENA_MAX_LEVEL));
        let edge = HexCoord::from_axial(12, 0).to_world(0.0);
        let clipped = wall_volume(
            shield_impact(edge, Vec3::Y),
            Vec3::X,
            (7, 4),
            &world,
            geometry,
            &[],
        );
        assert!(!clipped.is_empty() && clipped.len() < 28);
        assert!(clipped
            .iter()
            .all(|pos| geometry.contains_column(pos.coord)));
    }

    #[test]
    fn shield_anchor_outside_arena_keeps_cells_crossing_back_inside() {
        let geometry = ArenaVoxelGeometry::default();
        let world = ArenaTerrainView::default();
        let contact = HexCoord::from_axial(13, 0).to_world(0.0);
        let volume = wall_volume(
            shield_impact(contact, Vec3::Y),
            Vec3::Z,
            (7, 4),
            &world,
            geometry,
            &[],
        );
        assert_eq!(volume.len(), 12, "three of seven columns enter the arena");
        assert!(volume.iter().all(|pos| geometry.contains_column(pos.coord)));
        assert!(volume
            .iter()
            .any(|pos| pos.coord == HexCoord::from_axial(12, 0)));
    }

    #[test]
    fn pinned_actor_and_new_terrain_leave_gaps_in_a_mature_wall_without_overwrites() {
        let geometry = ArenaVoxelGeometry::default();
        let mut world = ArenaTerrainView::default();
        let candidates = wall_candidates(
            shield_impact(Vec3::ZERO, Vec3::Y),
            Vec3::X,
            (3, 4),
            geometry,
        );
        let occupied = *candidates.first().expect("wall cell");
        let materials = shield_materials();
        world.voxels.insert(occupied, materials.stone);
        let feet = Vec3::Y * SKIN;
        let mut session = ArenaSession {
            actors: vec![Actor::spawn(0, feet, Vec3::X)],
            pending_walls: vec![staged_wall(candidates)],
            ..Default::default()
        };
        let mut out = CommandsOut::default();
        session.advance_walls(&world, geometry, materials, &mut out);
        assert!(!out.edits.is_empty() && out.edits.len() < 12);
        assert!(out.edits.iter().all(|edit| edit.pos() != occupied
            && !voxel_overlaps_body(
                edit.pos(),
                geometry,
                feet,
                BODY_HEIGHT,
                BODY_RADIUS + SKIN * 4.0
            )));
        let actor = session.actors.first().expect("pinned actor");
        assert_eq!(actor.feet, feet);
        assert_eq!(
            actor.body.impulse_velocity,
            Vec3::ZERO,
            "formation does not keep pushing"
        );
        assert_eq!(session.shields_raised, 1);
        assert!(session.pending_walls.is_empty());
    }

    #[test]
    fn overlapping_shields_reserve_cells_deterministically_without_cancelling_neighbors() {
        let geometry = ArenaVoxelGeometry::default();
        let world = ArenaTerrainView::default();
        let a = TilePos::new(HexCoord::ORIGIN, 1);
        let b = a.above();
        let c = b.above();
        let mut session = ArenaSession {
            pending_walls: vec![staged_wall(vec![a, b]), staged_wall(vec![b, c])],
            ..Default::default()
        };
        let mut out = CommandsOut::default();
        session.advance_walls(&world, geometry, shield_materials(), &mut out);
        assert_eq!(
            out.edits.iter().map(TerrainEdit::pos).collect::<Vec<_>>(),
            vec![a, b, c]
        );
        assert_eq!(session.shields_raised, 2);
    }

    #[test]
    fn emergence_rechecks_fixed_candidates_after_pending_impact_settles() {
        let geometry = ArenaVoxelGeometry::default();
        let mut world = ArenaTerrainView::default();
        let materials = shield_materials();
        let a = TilePos::new(HexCoord::ORIGIN, 1);
        let b = a.above();
        world.voxels.insert(a, materials.stone);
        let impact = TerrainImpact {
            batch: TerrainBatchId(9),
            volume: vec![a],
            kind: hex_core::TerrainDamageKind::Elemental(materials.fire),
            power: 2,
        };
        let mut session = ArenaSession {
            pending_walls: vec![staged_wall(vec![a, b])],
            pending_impacts: [(impact.batch, impact)].into_iter().collect(),
            ..Default::default()
        };
        let mut out = CommandsOut::default();
        session.advance_walls(&world, geometry, materials, &mut out);
        assert!(out.edits.is_empty());
        assert_eq!(
            session.pending_walls.first().expect("waiting wall").voxels,
            vec![b]
        );
        // The settled world frees one original candidate and occupies the other.
        world.voxels.remove(&a);
        world.voxels.insert(b, materials.dirt);
        session.pending_impacts.clear();
        session.advance_walls(&world, geometry, materials, &mut out);
        assert_eq!(
            out.edits.iter().map(TerrainEdit::pos).collect::<Vec<_>>(),
            vec![a]
        );
        assert!(session.pending_walls.is_empty());
    }
}
