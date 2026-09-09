//! Finite attack authority. Barriers never enter the terrain edit stream.

use super::*;
use hex_core::{TerrainDamageKind, TerrainImpact};
use std::collections::BTreeSet;

#[path = "golem.rs"]
mod golem;

pub(super) use golem::golem_swipe_blocked;

pub(super) fn index(kind: CreatureAbility) -> usize {
    kind.index()
}

#[derive(Debug)]
pub(super) struct Cast {
    kind: CreatureAbility,
    age: f32,
    windup: f32,
    duration: f32,
    pulses: u8,
    direction: Vec3,
    team: TeamId,
    multiplier: f32,
    actor_damage: BTreeMap<ActorId, f32>,
    voxels: BTreeSet<TilePos>,
    barrier_damage: BTreeMap<u64, f32>,
    laser: Option<LaserTrack>,
}

#[derive(Debug, Clone, Copy)]
struct LaserTrack {
    target: ActorId,
    point: Vec3,
    velocity: Vec3,
    tick: u64,
    tracking: bool,
}

impl LaserTrack {
    fn observed(observed: targeting::ObservedTarget) -> Self {
        Self {
            target: observed.body.id,
            point: observed.sight_point,
            velocity: observed.body.velocity,
            tick: observed.tick,
            tracking: true,
        }
    }
}

impl Cast {
    pub(super) fn direction(&self) -> Vec3 {
        self.direction
    }

    pub(super) fn tracks_breath(&self) -> bool {
        self.kind == CreatureAbility::FireCone
    }

    pub(super) fn tracks_ember(&self) -> bool {
        self.kind == CreatureAbility::WispEmber && self.pulses == 0
    }

    fn tracks_boulder(&self) -> bool {
        self.kind == CreatureAbility::WormBoulder && self.pulses == 0
    }

    pub(super) fn tracks_laser(&self, _tuning: &EncounterTuning) -> bool {
        self.kind == CreatureAbility::GolemLaser
    }

    pub(super) fn laser_target(&self) -> Option<ActorId> {
        self.laser.map(|track| track.target)
    }

    pub(super) fn preparing_laser(&self) -> bool {
        self.kind == CreatureAbility::GolemLaser && self.age < self.windup
    }

    fn track_laser(
        &mut self,
        actor: &Actor,
        observed: Option<targeting::ObservedTarget>,
        tick: u64,
        tuning: &EncounterTuning,
    ) {
        let Some(track) = &mut self.laser else {
            return;
        };
        let current = observed.filter(|sample| {
            sample.body.id == track.target
                && sample.tick >= track.tick
                && sample.tick <= tick
                && tick.saturating_sub(sample.tick) < 12
        });
        track.tracking = current.is_some();
        if let Some(sample) = current {
            *track = LaserTrack::observed(sample);
        }
        // This cast has a finite duration. Extrapolate its own frozen sample
        // through that duration rather than a shorter projectile-forecast cap.
        let point = track.point + track.velocity * elapsed(tick, track.tick);
        let bearing = point - (actor.feet + Vec3::Y * 1.4);
        let mouth = shapes::golem_mouth(actor, bearing);
        let desired = (point - mouth).normalize_or(self.direction);
        let angle = self.direction.dot(desired).clamp(-1.0, 1.0).acos();
        let limit = tuning.golem_laser_turn_speed * STEP;
        self.direction = if angle <= limit {
            desired
        } else {
            let turn = bevy_math::Quat::from_rotation_arc(self.direction, desired);
            (bevy_math::Quat::IDENTITY.slerp(turn, limit / angle) * self.direction)
                .normalize_or(self.direction)
        };
    }

    fn track_breath(&mut self, actor: &Actor, max_turn: f32) {
        if !self.tracks_breath() {
            return;
        }
        // The body has already performed its collision-safe bounded yaw turn.
        // Pitch follows the observation-limited aim at the same angular limit;
        // the emitted cone always comes out of the actual forward-facing mouth.
        let pitch = self.direction.y.clamp(-1.0, 1.0).asin();
        let desired = actor.aim.y.clamp(-1.0, 1.0).asin();
        let pitch = pitch + (desired - pitch).clamp(-max_turn, max_turn);
        self.direction = actor.body_rotation() * Vec3::NEG_Z * pitch.cos() + Vec3::Y * pitch.sin();
    }
}

impl EncounterState {
    pub(crate) fn cancel_charges(&mut self) {
        for brain in self.brains.values_mut() {
            brain.cancel_charge();
        }
    }
}

impl ArenaSession {
    pub(super) fn advance_support(&mut self, tuning: &ArenaTuning, map: ArenaMap) {
        let c = &tuning.encounters;
        for a in &mut self.encounter.auras {
            a.remaining -= STEP;
        }
        self.encounter.auras.retain(|a| {
            a.remaining > 0.0
                && self
                    .actors
                    .iter()
                    .any(|owner| owner.id == a.owner && owner.hp > 0.0)
        });
        for aura in &mut self.encounter.auras {
            if let Some(owner) = self.actors.iter().find(|a| a.id == aura.owner) {
                aura.center = owner.center();
            }
        }
        self.refresh_support_buffs(tuning);
        let human_id = self.human_actor_id();
        for actor in &mut self.actors {
            if actor.hp <= 0.0 {
                continue;
            }
            let buff = actor.damage_multiplier > 1.0;
            if buff {
                actor.damage_multiplier = c.aura_damage_multiplier;
                actor.hp = (actor.hp + c.aura_heal * STEP).min(actor.max_hp);
            }
            if actor.species == Species::Dragon
                && actor
                    .last_damage_tick
                    .is_some_and(|t| elapsed(self.tick, t) >= c.dragon_regen_delay)
            {
                actor.hp = (actor.hp + c.dragon_regen_rate * STEP).min(actor.max_hp);
            }
            if map != ArenaMap::Duel
                && Some(actor.id) == human_id
                && elapsed(self.tick, actor.last_activity_tick) >= c.human_regen_delay
                && elapsed(self.tick, self.encounter.human_seen_tick) >= c.human_unseen_delay
            {
                actor.hp = (actor.hp + c.human_regen_rate * STEP).min(actor.max_hp);
            }
        }
    }

    pub(super) fn begin_ability(
        &mut self,
        brains: &mut BTreeMap<ActorId, brain::Brain>,
        id: ActorId,
        request: brain::Request,
        tuning: &ArenaTuning,
    ) {
        self.refresh_support_buffs(tuning);
        let Some(actor) = self.actors.iter().find(|a| a.id == id && a.hp > 0.0) else {
            return;
        };
        let Some(brain) = brains.get_mut(&id) else {
            return;
        };
        if !brain.ready(request.kind) {
            return;
        }
        let c = &tuning.encounters;
        let (windup, duration, cooldown) = match request.kind {
            CreatureAbility::FireCone => (c.breath_windup, c.breath_seconds, c.breath_cooldown),
            CreatureAbility::Bite => (c.bite_windup, 0.15, c.bite_cooldown),
            CreatureAbility::Swipe => (c.swipe_windup, 0.15, c.swipe_cooldown),
            CreatureAbility::Barrier => (0.0, 0.15, c.barrier_cooldown),
            CreatureAbility::Aura => (c.aura_windup, 0.2, c.aura_cooldown),
            CreatureAbility::GolemSlam => (c.golem_slam_windup, 0.15, c.golem_slam_cooldown),
            CreatureAbility::GolemSwipe => (c.golem_swipe_windup, 0.15, c.golem_swipe_cooldown),
            CreatureAbility::WispEmber => (c.wisp_ember_windup, 0.15, c.wisp_ember_cooldown),
            CreatureAbility::WormBoulder
                if actor
                    .worm()
                    .is_some_and(|s| s.exposed && s.phase == WormPhase::Exposed) =>
            {
                (c.worm_boulder_windup, 0.15, c.worm_boulder_cooldown)
            }
            CreatureAbility::GolemLaser => (
                c.golem_laser_charge,
                c.golem_laser_seconds,
                c.golem_laser_cooldown,
            ),
            _ => return,
        };
        if let Some(cd) = brain.cooldowns.get_mut(index(request.kind)) {
            *cd = cooldown;
        }
        brain.active = Some(Cast {
            kind: request.kind,
            age: 0.0,
            windup,
            duration,
            pulses: 0,
            direction: request.aim.normalize_or(actor.aim),
            team: actor.team,
            multiplier: actor.damage_multiplier,
            actor_damage: BTreeMap::new(),
            voxels: BTreeSet::new(),
            barrier_damage: BTreeMap::new(),
            laser: (request.kind == CreatureAbility::GolemLaser)
                .then(|| brain.golem_observation().map(LaserTrack::observed))
                .flatten(),
        });
    }

    pub(super) fn advance_abilities(
        &mut self,
        brains: &mut BTreeMap<ActorId, brain::Brain>,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        materials: ArenaMaterials,
        tuning: &ArenaTuning,
        out: &mut CommandsOut,
    ) {
        let c = &tuning.encounters;
        for (id, brain) in brains {
            for cd in &mut brain.cooldowns {
                *cd = (*cd - STEP).max(0.0);
            }
            let Some(mut cast) = brain.active.take() else {
                continue;
            };
            let Some(actor) = self
                .actors
                .iter()
                .find(|a| a.id == *id && a.hp > 0.0)
                .cloned()
            else {
                continue;
            };
            if cast.tracks_boulder()
                && !actor
                    .worm()
                    .is_some_and(|s| s.exposed && s.phase == WormPhase::Exposed)
            {
                continue;
            }
            cast.track_breath(&actor, c.dragon_turn_speed * STEP);
            if cast.tracks_laser(c) {
                cast.track_laser(&actor, brain.golem_observation(), self.tick, c);
            }
            if cast.tracks_ember() || cast.tracks_boulder() {
                cast.direction = actor.aim;
            }
            cast.age += STEP;
            if cast.tracks_ember() && cast.age + STEP * 0.01 >= cast.windup {
                let Some(aim) = brain.ember_release_aim(
                    &actor,
                    &self.collision,
                    world,
                    geometry,
                    tuning,
                    materials,
                    self.tick,
                ) else {
                    continue;
                };
                cast.direction = aim;
            }
            if cast.tracks_boulder() && cast.age + STEP * 0.01 >= cast.windup {
                let Some(aim) = self.worm_release_aim(&actor, world, geometry, tuning) else {
                    continue;
                };
                cast.direction = aim;
            }
            let (range, angle) = match cast.kind {
                CreatureAbility::FireCone => (c.breath_range, c.breath_angle.to_radians() * 0.5),
                CreatureAbility::Bite => (c.bite_range, c.bite_angle.to_radians() * 0.5),
                CreatureAbility::Swipe => (c.swipe_range, c.swipe_angle.to_radians() * 0.5),
                CreatureAbility::Aura => (c.aura_radius, std::f32::consts::PI),
                CreatureAbility::Barrier => (c.barrier_distance, 0.0),
                CreatureAbility::GolemSlam => (c.golem_slam_range, std::f32::consts::PI),
                CreatureAbility::GolemSwipe => (c.golem_swipe_range, std::f32::consts::FRAC_PI_2),
                CreatureAbility::WispEmber => (c.wisp_preferred_max, 0.0),
                CreatureAbility::WormBoulder => (
                    c.worm_boulder_speed * c.worm_boulder_speed / c.worm_boulder_gravity,
                    0.0,
                ),
                _ => (0.0, 0.0),
            };
            let phase = if cast.age < cast.windup {
                AttackPhase::Windup
            } else if cast.age < cast.windup + cast.duration {
                AttackPhase::Active
            } else {
                AttackPhase::Recovery
            };
            let progress = match phase {
                AttackPhase::Windup => cast.age / cast.windup.max(STEP),
                AttackPhase::Active => (cast.age - cast.windup) / cast.duration,
                AttackPhase::Recovery => (cast.age - cast.windup - cast.duration) / 0.15,
            }
            .clamp(0.0, 1.0);
            let beam = (cast.kind == CreatureAbility::GolemLaser)
                .then(|| self.golem_beam(&actor, &cast, geometry, tuning));
            let origin = if matches!(
                cast.kind,
                CreatureAbility::GolemSlam | CreatureAbility::GolemSwipe
            ) {
                actor.center()
            } else {
                beam.map_or(actor.eye(), |b| b.origin)
            };
            let range = beam.map_or(range, |b| b.origin.distance(b.end));
            if let Some(a) = self.actors.iter_mut().find(|a| a.id == *id) {
                if cast.kind == CreatureAbility::GolemLaser {
                    a.aim = cast.direction;
                }
                a.beam = beam;
                a.attack = Some(AttackSnapshot {
                    kind: cast.kind,
                    phase,
                    origin,
                    direction: cast.direction,
                    range,
                    half_angle: angle,
                    progress,
                });
            }
            let wanted = if cast.age + STEP * 0.01 < cast.windup {
                0
            } else if cast.kind == CreatureAbility::FireCone {
                if cast.age < cast.windup + cast.duration / 3.0 {
                    1
                } else if cast.age < cast.windup + cast.duration * 2.0 / 3.0 {
                    2
                } else {
                    3
                }
            } else {
                1
            };
            while cast.pulses < wanted {
                cast.pulses += 1;
                if cast.pulses == 1 {
                    self.refresh_support_buffs(tuning);
                    cast.multiplier = self
                        .actors
                        .iter()
                        .find(|a| a.id == *id && a.hp > 0.0)
                        .map_or(1.0, |a| a.damage_multiplier);
                    if let Some(count) = self
                        .encounter
                        .ability_counts
                        .entry(*id)
                        .or_insert([0; CREATURE_ABILITY_COUNT])
                        .get_mut(index(cast.kind))
                    {
                        *count += 1;
                    }
                    self.combat_cue(*id, actor.eye(), CombatCueKind::Release);
                    if let Some(a) = self.actors.iter_mut().find(|a| a.id == *id) {
                        a.last_activity_tick = self.tick;
                    }
                }
                match cast.kind {
                    CreatureAbility::Barrier => {
                        let normal = cast.direction.with_y(0.0).normalize_or(Vec3::NEG_Z);
                        self.encounter.barriers.push(BarrierSnapshot {
                            id: self.encounter.next_barrier,
                            owner: *id,
                            center: actor.eye() + normal * c.barrier_distance,
                            normal,
                            width: c.barrier_width,
                            height: c.barrier_height,
                            hp: c.barrier_hp,
                            max_hp: c.barrier_hp,
                            remaining: c.barrier_seconds,
                            lifetime: c.barrier_seconds,
                        });
                        self.encounter.next_barrier += 1;
                        self.collision.sync_barriers(&self.encounter.barriers);
                    }
                    CreatureAbility::Aura => self.encounter.auras.push(AuraSnapshot {
                        owner: *id,
                        center: actor.center(),
                        radius: c.aura_radius,
                        remaining: c.aura_seconds,
                        lifetime: c.aura_seconds,
                    }),
                    CreatureAbility::FireCone | CreatureAbility::Bite | CreatureAbility::Swipe => {
                        self.direct_pulse(
                            &actor, &mut cast, range, angle, world, geometry, materials, tuning,
                            out,
                        )
                    }
                    CreatureAbility::GolemSlam => {
                        self.golem_slam(&actor, &mut cast, world, geometry, tuning, out)
                    }
                    CreatureAbility::GolemSwipe => {
                        self.golem_swipe(&actor, &mut cast, world, geometry, materials, tuning, out)
                    }
                    CreatureAbility::WormBoulder => {
                        let mut spec = worm::boulder_spec(c);
                        spec.damage *= cast.multiplier;
                        self.release_creature_projectile(&actor, cast.direction, spec);
                    }
                    CreatureAbility::WispEmber => {
                        let mut spec = wisp::ember_spec(c, materials);
                        spec.damage *= cast.multiplier;
                        self.release_creature_projectile(&actor, cast.direction, spec);
                        brain.ember_released();
                    }
                    _ => {}
                }
            }
            if cast.kind == CreatureAbility::GolemLaser {
                let previous = (cast.age - STEP - cast.windup).clamp(0.0, cast.duration);
                let current = (cast.age - cast.windup).clamp(0.0, cast.duration);
                if current > previous {
                    self.golem_laser_tick(
                        &actor,
                        &mut cast,
                        current - previous,
                        world,
                        geometry,
                        materials,
                        tuning,
                        out,
                    );
                }
            }
            if cast.age < cast.windup + cast.duration + 0.15 {
                brain.active = Some(cast);
            } else if let Some(a) = self.actors.iter_mut().find(|a| a.id == *id) {
                a.attack = None;
                a.beam = None;
            }
        }
        for actor in &mut self.actors {
            if actor.hp <= 0.0 {
                actor.attack = None;
                actor.beam = None;
            }
        }
        self.encounter
            .barriers
            .retain(|b| b.hp > 0.0 && b.remaining > 0.0);
        self.collision.sync_barriers(&self.encounter.barriers);
    }

    fn direct_pulse(
        &mut self,
        owner: &Actor,
        cast: &mut Cast,
        range: f32,
        half_angle: f32,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        materials: ArenaMaterials,
        tuning: &ArenaTuning,
        out: &mut CommandsOut,
    ) {
        let c = &tuning.encounters;
        let (damage, power, kind) = match cast.kind {
            CreatureAbility::FireCone => (
                c.breath_damage,
                c.breath_terrain_power,
                TerrainDamageKind::Elemental(materials.fire),
            ),
            CreatureAbility::Bite => (
                c.bite_damage,
                c.bite_terrain_power,
                TerrainDamageKind::Physical,
            ),
            _ => (
                c.swipe_damage,
                c.swipe_terrain_power,
                TerrainDamageKind::Physical,
            ),
        };
        let origin = owner.eye();
        let direction = cast.direction;
        let in_cone = |point: Vec3| {
            let delta = point - origin;
            delta.length() <= range + SKIN
                && (delta.length_squared() < SKIN * SKIN
                    || delta.normalize().dot(direction) + SKIN >= half_angle.cos())
        };
        // Keep the same obstruction snapshot for the whole pulse: destroying a
        // barrier cannot also hit a body behind that barrier in the same pulse.
        let mut damage_events = Vec::new();
        for actor in &mut self.actors {
            if actor.hp <= 0.0 || actor.id == owner.id || actor.team == cast.team {
                continue;
            }
            if shapes::exposed_cone_contact(actor, origin, direction, range, half_angle, |point| {
                !self
                    .collision
                    .attack_sweep(origin, point - origin, 0.0)
                    .is_some_and(|(h, _)| h.fraction < 1.0 - SKIN)
            })
            .is_none()
            {
                continue;
            }
            let cap = damage * cast.multiplier;
            let previous = cast.actor_damage.get(&actor.id).copied().unwrap_or(0.0);
            let pulse = if cast.kind == CreatureAbility::FireCone {
                cap / 3.0
            } else {
                cap
            };
            let admitted = pulse.min((cap - previous).max(0.0));
            let removed = admitted.min(actor.hp);
            actor.hp -= removed;
            cast.actor_damage.insert(actor.id, previous + admitted);
            if removed > 0.0 {
                damage_events.push((actor.id, removed));
            }
        }
        for (victim, damage) in damage_events {
            self.record_damage(owner.id, victim, damage);
        }
        for barrier in &mut self.encounter.barriers {
            let rotation =
                bevy_math::Quat::from_rotation_y((-barrier.normal.x).atan2(-barrier.normal.z));
            let half = Vec3::new(barrier.width * 0.5, barrier.height * 0.5, 0.025);
            let Some(contact) =
                shapes::volume_cone_contact(origin, direction, range, half_angle, |point| {
                    closest_box_point(point, barrier.center, rotation, half)
                })
            else {
                continue;
            };
            let ray =
                (contact - origin) + ((contact - origin).normalize_or(direction) * SKIN * 4.0);
            if !self
                .collision
                .attack_sweep(origin, ray, 0.0)
                .is_some_and(|(_, id)| id == Some(barrier.id))
            {
                continue;
            }
            let cap = damage * cast.multiplier;
            let previous = cast.barrier_damage.get(&barrier.id).copied().unwrap_or(0.0);
            let pulse = if cast.kind == CreatureAbility::FireCone {
                cap / 3.0
            } else {
                cap
            };
            let admitted = pulse.min((cap - previous).max(0.0));
            barrier.hp = (barrier.hp - admitted).max(0.0);
            cast.barrier_damage.insert(barrier.id, previous + admitted);
        }
        let mut volume = Vec::new();
        for pos in geometry.sphere(
            world,
            origin,
            range + (1.0 + geometry.level_height * geometry.level_height * 0.25).sqrt(),
        ) {
            if cast.voxels.contains(&pos) {
                continue;
            }
            let Some(point) =
                shapes::volume_cone_contact(origin, direction, range, half_angle, |point| {
                    closest_voxel_point(point, pos, geometry)
                })
            else {
                continue;
            };
            let delta = (point - origin) + (point - origin).normalize_or(direction) * SKIN * 4.0;
            let Some((hit, barrier)) = self.collision.attack_sweep(origin, delta, 0.0) else {
                continue;
            };
            if barrier.is_some() {
                continue;
            }
            let contact = origin + delta * hit.fraction;
            if in_cone(contact)
                && geometry.voxel_at(contact + delta.normalize_or_zero() * SKIN * 2.0) == Some(pos)
            {
                volume.push(pos);
                cast.voxels.insert(pos);
            }
        }
        if !volume.is_empty() {
            let impact = TerrainImpact {
                batch: TerrainBatchId(self.next_impact),
                volume,
                kind,
                power,
            };
            self.next_impact += 1;
            self.pending_impacts.insert(impact.batch, impact.clone());
            out.impacts.push(impact);
        }
        self.encounter
            .barriers
            .retain(|b| b.hp > 0.0 && b.remaining > 0.0);
        self.collision.sync_barriers(&self.encounter.barriers);
    }
}

impl ArenaSession {
    pub(crate) fn refresh_support_buffs(&mut self, tuning: &ArenaTuning) {
        self.encounter.auras.retain(|aura| {
            aura.remaining > 0.0 && self.actors.iter().any(|a| a.id == aura.owner && a.hp > 0.0)
        });
        for aura in &mut self.encounter.auras {
            if let Some(owner) = self.actors.iter().find(|a| a.id == aura.owner) {
                aura.center = owner.center();
            }
        }
        let fields: Vec<_> = self
            .encounter
            .auras
            .iter()
            .filter_map(|aura| {
                self.actors
                    .iter()
                    .find(|a| a.id == aura.owner)
                    .map(|owner| (*aura, owner.party, owner.eye()))
            })
            .collect();
        for actor in &mut self.actors {
            let eligible = actor.hp > 0.0
                && fields.iter().any(|(aura, party, eye)| {
                    aura.owner != actor.id
                        && party.is_some()
                        && *party == actor.party
                        && aura.center.distance(actor.center()) <= aura.radius
                        && self.collision.sight_clear(*eye, actor.center())
                });
            actor.damage_multiplier = if eligible {
                tuning.encounters.aura_damage_multiplier
            } else {
                1.0
            };
        }
    }
}

fn closest_box_point(point: Vec3, center: Vec3, rotation: bevy_math::Quat, half: Vec3) -> Vec3 {
    center + rotation * (rotation.inverse() * (point - center)).clamp(-half, half)
}

fn closest_voxel_point(point: Vec3, pos: TilePos, geometry: ArenaVoxelGeometry) -> Vec3 {
    let center = geometry.center(pos);
    let local = (point - center).with_y(0.0);
    let y = point
        .y
        .clamp(geometry.top(pos) - geometry.level_height, geometry.top(pos));
    let normals = [
        Vec3::X,
        Vec3::new(0.5, 0.0, 0.866_025_4),
        Vec3::new(-0.5, 0.0, 0.866_025_4),
    ];
    if normals
        .into_iter()
        .all(|n| n.dot(local).abs() <= 0.866_025_4)
    {
        return (center + local).with_y(y);
    }
    let vertices = [
        Vec3::Z,
        Vec3::new(0.866_025_4, 0.0, 0.5),
        Vec3::new(0.866_025_4, 0.0, -0.5),
        Vec3::NEG_Z,
        Vec3::new(-0.866_025_4, 0.0, -0.5),
        Vec3::new(-0.866_025_4, 0.0, 0.5),
    ];
    vertices
        .iter()
        .zip(vertices.iter().cycle().skip(1))
        .take(6)
        .map(|(a, b)| {
            let edge = *b - *a;
            let t = (local - *a).dot(edge) / edge.length_squared();
            (center + *a + edge * t.clamp(0.0, 1.0)).with_y(y)
        })
        .min_by(|a, b| {
            a.distance_squared(point)
                .total_cmp(&b.distance_squared(point))
        })
        .unwrap_or(center.with_y(y))
}

#[cfg(test)]
#[path = "cone_tests.rs"]
mod cone_tests;
