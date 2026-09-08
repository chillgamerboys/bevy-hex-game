//! Golem finite sphere and locked continuous beam, using ordinary damage authority.

use super::*;

#[derive(Clone, Copy)]
enum Contact {
    Actor(ActorId),
    Barrier(u64),
    Terrain(crate::collision::Hit),
}

impl ArenaSession {
    fn golem_trace(
        &self,
        owner: &Actor,
        cast: &Cast,
        geometry: ArenaVoxelGeometry,
        tuning: &ArenaTuning,
    ) -> (BeamSnapshot, Option<Contact>) {
        let origin = shapes::golem_mouth(owner, cast.direction);
        let delta = world_end(origin, cast.direction, geometry) - origin;
        let radius = tuning.encounters.golem_laser_radius;
        let obstruction = self.collision.attack_sweep(origin, delta, radius);
        let mut fraction = obstruction.map_or(1.0, |(hit, _)| hit.fraction);
        let mut contact =
            obstruction.map(|(hit, id)| id.map_or(Contact::Terrain(hit), Contact::Barrier));
        for actor in &self.actors {
            if actor.hp <= 0.0 || actor.id == owner.id || actor.team == cast.team {
                continue;
            }
            if let Some(hit) = shapes::sweep_actor(origin, delta, actor, true, radius) {
                if hit.fraction < fraction {
                    fraction = hit.fraction;
                    contact = Some(Contact::Actor(actor.id));
                }
            }
        }
        (
            BeamSnapshot {
                origin,
                direction: cast.direction,
                end: origin + delta * fraction,
                radius,
                locked: !cast.tracks_laser(&tuning.encounters),
            },
            contact,
        )
    }

    pub(super) fn golem_beam(
        &self,
        owner: &Actor,
        cast: &Cast,
        geometry: ArenaVoxelGeometry,
        tuning: &ArenaTuning,
    ) -> BeamSnapshot {
        self.golem_trace(owner, cast, geometry, tuning).0
    }

    pub(super) fn golem_laser_tick(
        &mut self,
        owner: &Actor,
        cast: &mut Cast,
        dt: f32,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        materials: ArenaMaterials,
        tuning: &ArenaTuning,
        out: &mut CommandsOut,
    ) {
        let (beam, contact) = self.golem_trace(owner, cast, geometry, tuning);
        let cap = tuning.encounters.golem_laser_damage * cast.multiplier;
        let pulse = cap * dt / cast.duration;
        match contact {
            Some(Contact::Actor(id)) => {
                let previous = cast.actor_damage.get(&id).copied().unwrap_or(0.0);
                let admitted = pulse.min((cap - previous).max(0.0));
                let removed = self
                    .actors
                    .iter_mut()
                    .find(|a| a.id == id)
                    .map_or(0.0, |actor| {
                        let removed = admitted.min(actor.hp);
                        actor.hp -= removed;
                        removed
                    });
                cast.actor_damage.insert(id, previous + admitted);
                if removed > 0.0 {
                    self.record_damage(owner.id, id, removed);
                    if previous <= 0.0 {
                        self.combat_cue_from(owner.id, cast.team, beam.end, CombatCueKind::Impact);
                    }
                }
            }
            Some(Contact::Barrier(id)) => {
                let previous = cast.barrier_damage.get(&id).copied().unwrap_or(0.0);
                let admitted = pulse.min((cap - previous).max(0.0));
                if let Some(barrier) = self.encounter.barriers.iter_mut().find(|b| b.id == id) {
                    barrier.hp = (barrier.hp - admitted).max(0.0);
                }
                cast.barrier_damage.insert(id, previous + admitted);
                self.collision.sync_barriers(&self.encounter.barriers);
            }
            Some(Contact::Terrain(hit)) => {
                // The beam endpoint is the swept sphere center. Move onto the
                // physical struck face before selecting the authoritative voxel.
                let point = beam.end - hit.normal * (beam.radius + SKIN * 4.0);
                let pos = if hit.normal.length_squared() < SKIN * SKIN {
                    // A starting sphere overlap has no face normal. Its center
                    // can still be in free air: recover the actual intersected
                    // voxel volume rather than selecting that empty center cell.
                    let extent =
                        (1.0 + geometry.level_height * geometry.level_height * 0.25).sqrt();
                    geometry
                        .sphere(world, beam.end, beam.radius + extent)
                        .into_iter()
                        .filter_map(|pos| {
                            let distance =
                                closest_voxel_point(beam.end, pos, geometry).distance(beam.end);
                            (distance <= beam.radius + SKIN).then_some((pos, distance))
                        })
                        .min_by(|(a, da), (b, db)| da.total_cmp(db).then_with(|| a.cmp(b)))
                        .map(|(pos, _)| pos)
                } else {
                    geometry.voxel_at(point)
                };
                if let Some(pos) = pos {
                    if world.voxels.contains_key(&pos) && cast.voxels.insert(pos) {
                        self.golem_terrain(
                            vec![pos],
                            TerrainDamageKind::Elemental(materials.fire),
                            tuning.encounters.golem_laser_terrain_power,
                            out,
                        );
                        self.combat_cue_from(owner.id, cast.team, beam.end, CombatCueKind::Impact);
                    }
                }
            }
            None => {}
        }
    }

    pub(super) fn golem_slam(
        &mut self,
        owner: &Actor,
        cast: &mut Cast,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        tuning: &ArenaTuning,
        out: &mut CommandsOut,
    ) {
        let center = owner.center();
        let c = &tuning.encounters;
        let mut events = Vec::new();
        for actor in &mut self.actors {
            if actor.hp <= 0.0
                || actor.id == owner.id
                || actor.team == cast.team
                || shapes::distance(center, actor) > c.golem_slam_range + SKIN
            {
                continue;
            }
            let cap = c.golem_slam_damage * cast.multiplier;
            let previous = cast.actor_damage.get(&actor.id).copied().unwrap_or(0.0);
            let removed = (cap - previous).max(0.0).min(actor.hp);
            actor.hp -= removed;
            cast.actor_damage.insert(actor.id, cap);
            if removed > 0.0 {
                let away = (actor.center() - center).normalize_or(Vec3::Y);
                actor.body.impulse_velocity +=
                    (away + Vec3::Y * 0.35).normalize_or(Vec3::Y) * c.golem_slam_knockback;
                actor.body.grounded = false;
                actor.grounded = false;
                events.push((actor.id, removed));
            }
        }
        for (id, removed) in events {
            self.record_damage(owner.id, id, removed);
        }
        // Sphere intersection uses voxel volume, so a face inside the radius
        // is admitted even if its center lies outside. Cover does not clip splash.
        let extent = (1.0 + geometry.level_height * geometry.level_height * 0.25).sqrt();
        let volume = geometry
            .sphere(world, center, c.golem_slam_range + extent)
            .into_iter()
            .filter(|pos| {
                closest_voxel_point(center, *pos, geometry).distance(center)
                    <= c.golem_slam_range + SKIN
            })
            .filter(|pos| cast.voxels.insert(*pos))
            .collect::<Vec<_>>();
        if !volume.is_empty() {
            self.golem_terrain(
                volume,
                TerrainDamageKind::Physical,
                c.golem_slam_terrain_power,
                out,
            );
        }
        self.effects.push(VisualEffect {
            center,
            radius: c.golem_slam_range,
            age: 0.0,
            lifetime: 0.45,
            kind: Spell::AreaBlast,
        });
        self.combat_cue_from(owner.id, cast.team, center, CombatCueKind::Impact);
    }

    fn golem_terrain(
        &mut self,
        volume: Vec<TilePos>,
        kind: TerrainDamageKind,
        power: u8,
        out: &mut CommandsOut,
    ) {
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
}

// Traverse the finite native-hex footprint. A laser ends at the real resident
// boundary (or vertical limit), not an arbitrary combat range or padded AABB.
fn world_end(origin: Vec3, direction: Vec3, geometry: ArenaVoxelGeometry) -> Vec3 {
    let mut coord = HexCoord::from_world(origin);
    let bottom = geometry.top(TilePos::new(coord, geometry.min_level)) - geometry.level_height;
    let top = geometry.top(TilePos::new(coord, geometry.max_level));
    if !geometry.contains_column(coord) || origin.y < bottom || origin.y > top {
        return origin;
    }
    let vertical = if direction.y > SKIN {
        (top - origin.y) / direction.y
    } else if direction.y < -SKIN {
        (bottom - origin.y) / direction.y
    } else {
        f32::INFINITY
    };
    let normals = [
        Vec3::X,
        Vec3::new(0.5, 0.0, 0.866_025_4),
        Vec3::new(-0.5, 0.0, 0.866_025_4),
    ];
    let mut traveled = 0.0;
    for _ in 0..geometry.radius.saturating_mul(6).saturating_add(12) {
        let local = origin - coord.to_world(0.0);
        let exit = normals
            .into_iter()
            .flat_map(|n| [n, -n])
            .filter_map(|normal| {
                let rate = normal.dot(direction);
                (rate > SKIN).then(|| (0.866_025_4 - normal.dot(local)) / rate)
            })
            .filter(|t| *t > traveled + SKIN * 0.1)
            .min_by(f32::total_cmp)
            .unwrap_or(f32::INFINITY);
        if vertical <= exit {
            return origin + direction * vertical;
        }
        if !exit.is_finite() {
            return origin;
        }
        let next = HexCoord::from_world(origin + direction * (exit + SKIN * 8.0));
        if next == coord || !geometry.contains_column(next) {
            return origin + direction * exit;
        }
        coord = next;
        traveled = exit;
    }
    origin + direction * traveled
}

#[cfg(test)]
#[path = "golem_tests.rs"]
mod tests;
