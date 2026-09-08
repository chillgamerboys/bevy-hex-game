//! Finite attack authority. Barriers never enter the terrain edit stream.

use super::*;
use hex_core::{TerrainDamageKind, TerrainImpact};
use std::collections::BTreeSet;

pub(super) fn index(kind: CreatureAbility) -> usize {
    match kind {
        CreatureAbility::Fireball => 0,
        CreatureAbility::Shield => 1,
        CreatureAbility::FireCone => 2,
        CreatureAbility::Bite => 3,
        CreatureAbility::Swipe => 4,
        CreatureAbility::Barrier => 5,
        CreatureAbility::Aura => 6,
    }
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
}

impl Cast {
    pub(super) fn direction(&self) -> Vec3 {
        self.direction
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
    pub(super) fn advance_support(&mut self, tuning: &ArenaTuning) {
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
            if actor.id == 0
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
            cast.age += STEP;
            let (range, angle) = match cast.kind {
                CreatureAbility::FireCone => (c.breath_range, c.breath_angle.to_radians() * 0.5),
                CreatureAbility::Bite => (c.bite_range, c.bite_angle.to_radians() * 0.5),
                CreatureAbility::Swipe => (c.swipe_range, c.swipe_angle.to_radians() * 0.5),
                CreatureAbility::Aura => (c.aura_radius, std::f32::consts::PI),
                CreatureAbility::Barrier => (c.barrier_distance, 0.0),
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
            if let Some(a) = self.actors.iter_mut().find(|a| a.id == *id) {
                a.attack = Some(AttackSnapshot {
                    kind: cast.kind,
                    phase,
                    origin: actor.eye(),
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
                        .or_insert([0; 7])
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
                    _ => {}
                }
            }
            if cast.age < cast.windup + cast.duration + 0.15 {
                brain.active = Some(cast);
            } else if let Some(a) = self.actors.iter_mut().find(|a| a.id == *id) {
                a.attack = None;
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
                    || delta.normalize().dot(direction) >= half_angle.cos())
        };
        // Keep the same obstruction snapshot for the whole pulse: destroying a
        // barrier cannot also hit a body behind that barrier in the same pulse.
        let mut damage_events = Vec::new();
        for actor in &mut self.actors {
            if actor.hp <= 0.0 || actor.id == owner.id || actor.team == cast.team {
                continue;
            }
            let Some(point) = body_cone_contact(origin, direction, range, half_angle, actor) else {
                continue;
            };
            if self
                .collision
                .attack_sweep(origin, point - origin, 0.0)
                .is_some_and(|(h, _)| h.fraction < 1.0 - SKIN)
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
            let distance = (barrier.center - origin).length();
            let ray = (barrier.center - origin).normalize_or(direction) * range;
            let Some(hit) = shapes::sweep_barrier(barrier, origin, ray, 0.0) else {
                continue;
            };
            let contact = origin + ray * hit.fraction;
            if distance > range + barrier.width || !in_cone(contact) {
                continue;
            }
            if !self
                .collision
                .attack_sweep(origin, contact - origin, 0.0)
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
            let center = geometry.center(pos);
            let delta = center - origin;
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

fn closest_body_point(origin: Vec3, actor: &Actor) -> Vec3 {
    if actor.species == Species::Dragon {
        let inverse = actor.body_rotation().inverse();
        let local = inverse * (origin - actor.center());
        actor.center()
            + actor.body_rotation() * local.clamp(-actor.dimensions * 0.5, actor.dimensions * 0.5)
    } else {
        let radius = actor.dimensions.x * 0.5;
        let axis = Vec3::new(
            actor.feet.x,
            origin.y.clamp(
                actor.feet.y + radius,
                actor.feet.y + actor.dimensions.y - radius,
            ),
            actor.feet.z,
        );
        axis + (origin - axis).normalize_or_zero() * radius.min(origin.distance(axis))
    }
}

// Alternating projections between two convex physical volumes handles a flank
// clipping the cone even when the body's closest point to the mouth is outside.
fn body_cone_contact(
    origin: Vec3,
    direction: Vec3,
    range: f32,
    angle: f32,
    actor: &Actor,
) -> Option<Vec3> {
    let mut point = closest_body_point(origin, actor);
    for _ in 0..32 {
        let delta = point - origin;
        let distance = delta.length();
        let axial = delta.dot(direction);
        let radial = delta - direction * axial;
        let cone = if distance <= range && (distance < SKIN || axial / distance >= angle.cos()) {
            point
        } else if distance > range && axial / distance >= angle.cos() {
            origin + delta * (range / distance)
        } else {
            let edge = direction * angle.cos() + radial.normalize_or(Vec3::X) * angle.sin();
            origin + edge * delta.dot(edge).clamp(0.0, range)
        };
        let body = closest_body_point(cone, actor);
        if body.distance_squared(cone) < SKIN * SKIN {
            return Some(cone);
        }
        point = body;
    }
    None
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
