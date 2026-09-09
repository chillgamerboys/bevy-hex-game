//! Shallow burrowing authority: every motion is checked against current published earth.

use super::*;

#[cfg(test)]
#[path = "worm_tests.rs"]
mod tests;
use crate::bot::ballistic_aim_with_gravity;
use crate::spells::{forecast_creature_projectile, CreatureProjectileSpec};
use crate::worm_body::WormBodyState;
use crate::worm_geometry::{self, Admission, BurrowContext, PrismPose};
use bevy_math::Quat;
use hex_core::arena::{
    ArenaBurrowMaterials, ArenaBurrowOutcome, ArenaBurrowRequest, ArenaDeploymentRegion,
};
use hex_core::TerrainDamageKind;

/// Species-specific earth sensing supplies a position, never a shot forecast.
#[derive(Debug, Clone, Copy)]
struct BurrowTarget {
    id: ActorId,
    point: Vec3,
    tick: u64,
}

#[derive(Debug)]
pub(super) struct Controller {
    heading: f32,
    lift: f32,
    surface: f32,
    phase: WormPhase,
    phase_time: f32,
    blocked: f32,
    next_sense: u64,
    seen: Vec<targeting::ObservedTarget>,
    target: Option<Knowledge>,
    goal: Vec3,
    search_step: u8,
    committed_direction: Option<Vec3>,
    committed_goal: Vec3,
    burrow_target: Option<BurrowTarget>,
    preferred_threat: Option<ActorId>,
    pursuit_active: bool,
    attempted_this_exposure: bool,
}

impl Controller {
    fn new(actor: &Actor) -> Self {
        Self {
            heading: actor.body_yaw,
            lift: 0.0,
            surface: actor.feet.y - SKIN,
            phase: WormPhase::Emerging,
            phase_time: 0.0,
            blocked: 0.0,
            next_sense: 0,
            seen: Vec::new(),
            target: None,
            goal: actor.feet,
            search_step: actor.id % 4,
            committed_direction: None,
            committed_goal: actor.feet,
            burrow_target: None,
            preferred_threat: None,
            pursuit_active: false,
            attempted_this_exposure: false,
        }
    }

    fn phase(&mut self, next: WormPhase) {
        self.phase = next;
        self.phase_time = 0.0;
        if next != WormPhase::Travel {
            self.burrow_target = None;
            self.committed_direction = None;
        }
        if next == WormPhase::Exposed {
            self.attempted_this_exposure = false;
        }
        if next == WormPhase::Diving {
            self.target = None;
            self.seen.clear();
        }
    }

    pub(super) fn pursuing(&self) -> bool {
        self.pursuit_active
    }

    pub(super) fn hostile_damage(&mut self, owner: ActorId) {
        self.preferred_threat = Some(owner);
        self.pursuit_active = true;
        if self.phase != WormPhase::Travel {
            self.phase(WormPhase::Diving);
        }
    }

    fn physically_buried(
        &self,
        actor: &Actor,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        depth: u8,
    ) -> bool {
        self.lift <= SKIN
            && pose(actor).is_some_and(|body| {
                body.parts.iter().all(|part| part.offset.y.abs() <= SKIN)
                    && travel_depth(body, self.surface, world, geometry, depth)
            })
    }

    fn sense_underground(&mut self, actor: &Actor, actors: &[Actor], tick: u64) {
        let target = actors
            .iter()
            .filter(|a| a.hp > 0.0 && a.team != actor.team)
            .min_by(|a, b| {
                (Some(b.id) == self.preferred_threat)
                    .cmp(&(Some(a.id) == self.preferred_threat))
                    .then_with(|| {
                        a.feet
                            .distance_squared(actor.feet)
                            .total_cmp(&b.feet.distance_squared(actor.feet))
                    })
                    .then_with(|| a.id.cmp(&b.id))
            });
        self.burrow_target = target.map(|target| BurrowTarget {
            id: target.id,
            point: target.feet,
            tick,
        });
        if target.is_none() {
            self.pursuit_active = false;
            self.preferred_threat = None;
        }
    }

    pub(super) fn intent(
        &mut self,
        actor: &Actor,
        party: &PartyRuntime,
        actors: &[Actor],
        brain: &mut brain::Brain,
        collision: &CollisionWorld,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        tuning: &ArenaTuning,
        tick: u64,
    ) -> (brain::MotionIntent, Option<brain::Request>) {
        let c = &tuning.encounters;
        self.phase_time += STEP;
        self.pursuit_active |= party.snapshot.phase == PartyPhase::Active;
        let buried = self.physically_buried(actor, world, geometry, c.worm_depth_levels);
        if self.phase == WormPhase::Travel && !buried {
            self.phase(WormPhase::Diving);
        } else if self.phase == WormPhase::Diving && buried {
            self.phase(WormPhase::Travel);
            self.next_sense = tick;
        }
        let exposed = actor.worm().is_some_and(|s| s.exposed);
        let can_see = exposed && matches!(self.phase, WormPhase::Emerging | WormPhase::Exposed);
        if buried && self.phase == WormPhase::Travel && self.pursuit_active {
            if tick >= self.next_sense {
                self.sense_underground(actor, actors, tick);
                self.next_sense = tick.saturating_add(12);
            }
        } else {
            self.burrow_target = None;
        }
        if can_see && tick >= self.next_sense {
            self.seen = targeting::observe(
                actor,
                actors,
                &self.seen,
                collision,
                tick,
                tuning.bot.prediction_seconds,
            );
            self.next_sense = tick.saturating_add(12);
        }
        if !can_see {
            self.seen.clear();
        }
        self.target = can_see
            .then(|| {
                self.seen
                    .iter()
                    .copied()
                    .filter(|s| elapsed(tick, s.tick) <= 0.2)
                    .find(|s| collision.sight_clear(actor.eye(), s.sight_point))
            })
            .flatten()
            .map(|observed| Knowledge {
                point: observed.body.feet,
                velocity: observed.body.velocity,
                tick: observed.tick,
                direct: true,
                cue_kind: None,
                observed: Some(observed),
            });
        if self.phase == WormPhase::Emerging && exposed {
            self.phase(WormPhase::Exposed);
        }
        // Earth sensing chooses where to surface; it never authorizes a shot.
        // Distant targets are approached underground instead of stopping every
        // 1.5 seconds to stare at an impossible ballistic range.
        let approach_ready = self.burrow_target.is_none_or(|target| {
            let mouth = actor
                .eye()
                .with_y(self.surface + geometry.level_height * 1.5 + SKIN);
            ballistic_aim_with_gravity(
                mouth,
                target.point,
                c.worm_boulder_gravity,
                c.worm_boulder_speed,
            )
            .is_some()
        });
        if self.phase == WormPhase::Travel
            && buried
            && approach_ready
            && (self.phase_time >= c.worm_surface_interval || self.blocked >= 0.4)
        {
            self.phase(WormPhase::Emerging);
            self.next_sense = tick;
        }
        let request = if self.pursuit_active
            && self.phase == WormPhase::Exposed
            && !self.attempted_this_exposure
            && brain.ready(CreatureAbility::WormBoulder)
        {
            self.target
                .and_then(|seen| release_aim(actor, seen, collision, world, geometry, tuning, tick))
                .map(|aim| brain::Request {
                    kind: CreatureAbility::WormBoulder,
                    aim,
                })
        } else {
            None
        };
        if request.is_some() {
            self.attempted_this_exposure = true;
        }
        if self.phase == WormPhase::Exposed
            && brain.active.is_none()
            && request.is_none()
            && (self.attempted_this_exposure || self.phase_time >= c.worm_exposed_watch)
        {
            self.phase(WormPhase::Diving);
        }
        if self.phase == WormPhase::Diving {
            // This brain belongs only to a Worm. A canceled windup keeps its
            // normal cooldown; released projectiles keep their frozen payload.
            brain.active = None;
        }
        let known = self.target.or_else(|| {
            party
                .knowledge
                .filter(|k| elapsed(tick, k.tick) <= party.search)
        });
        self.goal = if !self.pursuit_active {
            party.snapshot.home
        } else if let Some(target) = self
            .burrow_target
            .filter(|target| elapsed(tick, target.tick) <= 0.2)
        {
            self.preferred_threat = Some(target.id);
            target.point
        } else if let Some(known) = known {
            known
                .observed
                .map_or(known.point, targeting::ObservedTarget::center)
        } else if let Some(search) = party.battle_search {
            let offsets = [Vec3::X, Vec3::Z, Vec3::NEG_X, Vec3::NEG_Z];
            let point = search
                + offsets
                    .get(usize::from(self.search_step % 4))
                    .copied()
                    .unwrap_or(Vec3::ZERO)
                    * 3.0;
            if actor.feet.with_y(0.0).distance(point.with_y(0.0)) < 1.0 {
                self.search_step = self.search_step.wrapping_add(1);
            }
            point
        } else {
            party.snapshot.home
        };
        if self.goal.distance(self.committed_goal) > 2.0 || self.blocked > 0.2 {
            self.committed_direction = None;
        }
        let direction = if self.phase == WormPhase::Travel && buried {
            self.committed_direction
                .unwrap_or_else(|| (self.goal - actor.feet).with_y(0.0).normalize_or_zero())
        } else {
            Vec3::ZERO
        };
        let aim = request.map_or_else(
            || {
                self.target.map_or(actor.aim, |k| {
                    (k.observed
                        .map_or(k.point, targeting::ObservedTarget::center)
                        - actor.eye())
                    .normalize_or(actor.aim)
                })
            },
            |r| r.aim,
        );
        (
            brain::MotionIntent {
                input: ActorIntent {
                    aim,
                    ..Default::default()
                },
                direction,
                flight: false,
                lunge: false,
            },
            request,
        )
    }
}

pub(super) fn boulder_spec(c: &EncounterTuning) -> CreatureProjectileSpec {
    CreatureProjectileSpec {
        ability: CreatureAbility::WormBoulder,
        appearance: ProjectileAppearance::Boulder,
        speed: c.worm_boulder_speed,
        gravity: c.worm_boulder_gravity,
        collision_radius: c.worm_boulder_collision_radius,
        splash_radius: c.worm_boulder_radius,
        damage: c.worm_boulder_damage,
        knockback: c.worm_boulder_knockback,
        terrain_kind: TerrainDamageKind::Physical,
        terrain_power: c.worm_boulder_terrain_power,
    }
}

fn release_aim(
    actor: &Actor,
    seen: Knowledge,
    collision: &CollisionWorld,
    world: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    tuning: &ArenaTuning,
    tick: u64,
) -> Option<Vec3> {
    if !actor.worm().is_some_and(|s| s.exposed) || !seen.direct || elapsed(tick, seen.tick) > 0.2 {
        return None;
    }
    let observed = seen.observed?;
    if !collision.sight_clear(actor.eye(), observed.sight_point) {
        return None;
    }
    let mut fact = observed.body;
    let age = elapsed(tick, seen.tick).min(fact.predict_seconds);
    fact.feet += fact.velocity * age;
    fact.predict_seconds = (fact.predict_seconds - age).max(0.0);
    let spec = boulder_spec(&tuning.encounters);
    let (_, time) =
        ballistic_aim_with_gravity(actor.eye(), fact.center(), spec.gravity, spec.speed)?;
    let (aim, _) = ballistic_aim_with_gravity(
        actor.eye(),
        fact.center() + fact.velocity * time.min(fact.predict_seconds),
        spec.gravity,
        spec.speed,
    )?;
    let hit = forecast_creature_projectile(actor, aim, spec, &[fact], collision, world, geometry)
        .impact?;
    let target = targeting::ObservedTarget {
        body: fact,
        ..observed
    };
    (hit.barrier.is_none() && target.distance(hit.point, hit.time) <= spec.splash_radius * 0.6)
        .then_some(aim)
}

fn pose(actor: &Actor) -> Option<PrismPose> {
    Some(PrismPose {
        feet: actor.feet,
        parts: actor.body_prism_snapshot()?,
    })
}

fn surface_at(
    view: &ArenaTerrainView,
    coord: HexCoord,
    reference: f32,
    geometry: ArenaVoxelGeometry,
) -> Option<TilePos> {
    let center = geometry.voxel_at(coord.to_world(reference))?.level;
    (center - 2..=center + 2)
        .filter(|level| *level >= geometry.min_level && *level <= geometry.max_level)
        .map(|level| TilePos::new(coord, level))
        .filter(|p| {
            view.voxels.contains_key(p)
                && !view.voxels.contains_key(&TilePos::new(coord, p.level + 1))
        })
        .min_by(|a, b| {
            (geometry.top(*a) - reference)
                .abs()
                .total_cmp(&(geometry.top(*b) - reference).abs())
        })
}

fn head_supports(
    body: PrismPose,
    reference: f32,
    view: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
) -> Option<Vec<TilePos>> {
    let head_base = body.feet.y + body.parts.iter().next()?.offset.y;
    let ceiling = reference.max(head_base) + SKIN;
    worm_geometry::head_columns(body)?
        .into_iter()
        .map(|coord| {
            // Exposure may remain valid above a newly deep crater even when
            // the anchored tail prevents whole-body settling. Search only this
            // actual head column and the finite resident levels. Never select
            // an unrelated upper roof above both the old band and current head.
            view.voxels
                .range(
                    TilePos::new(coord, geometry.min_level)
                        ..=TilePos::new(coord, geometry.max_level),
                )
                .rev()
                .find_map(|(pos, _)| {
                    (geometry.top(*pos) <= ceiling
                        && !view
                            .voxels
                            .contains_key(&TilePos::new(coord, pos.level + 1)))
                    .then_some(*pos)
                })
        })
        .collect()
}

fn band(
    body: PrismPose,
    reference: f32,
    view: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    depth: u8,
) -> Option<f32> {
    let supports: Option<Vec<_>> = worm_geometry::footprint_columns(body)?
        .into_iter()
        .map(|coord| surface_at(view, coord, reference, geometry))
        .collect();
    let mut low = f32::INFINITY;
    let mut high = f32::NEG_INFINITY;
    for surface in supports? {
        low = low.min(geometry.top(surface));
        high = high.max(geometry.top(surface));
    }
    (high - low <= f32::from(depth.saturating_sub(1)) * geometry.level_height + SKIN)
        .then_some(high)
}

fn travel_depth(
    body: PrismPose,
    reference: f32,
    world: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    depth: u8,
) -> bool {
    let Some(columns) = worm_geometry::footprint_columns(body) else {
        return false;
    };
    columns.into_iter().all(|coord| {
        surface_at(world, coord, reference, geometry).is_some_and(|surface| {
            let distance = geometry.top(surface) - body.feet.y;
            distance >= geometry.level_height - SKIN * 2.0
                && distance <= f32::from(depth) * geometry.level_height + SKIN * 2.0
        })
    })
}

fn refresh_exposure(
    actor: &mut Actor,
    reference: f32,
    phase: WormPhase,
    world: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
) {
    let clearance = pose(actor).and_then(|body| {
        let supports = head_supports(body, reference, world, geometry)?;
        worm_geometry::head_clearance(body, &supports, world, geometry)
    });
    if let Some(body) = &mut actor.worm {
        body.snapshot = WormSnapshot {
            phase,
            head_index: 0,
            head_clearance: clearance.unwrap_or(0.0),
            exposed: clearance.is_some_and(|gap| gap + SKIN * 0.5 >= geometry.level_height),
        };
    }
}

impl ArenaSession {
    pub(crate) fn install_burrow_policy(&mut self, policy: &ArenaBurrowMaterials) {
        if self.burrow_policy.eligible != policy.eligible {
            self.burrow_policy = policy.clone();
        }
    }

    pub(crate) fn accept_burrow_outcome(&mut self, outcome: &ArenaBurrowOutcome) {
        if self.generation != Some(outcome.generation)
            || !self
                .actors
                .iter()
                .any(|a| a.id == outcome.actor && a.hp > 0.0)
            || !self
                .pending_burrows
                .get(&outcome.actor)
                .is_some_and(|pending| pending.sequence == outcome.sequence)
        {
            return;
        }
        // Acknowledgement only frees the slot. The next pose is recomputed and
        // fully checked against current dirt, bodies, intent and terrain.
        self.pending_burrows.remove(&outcome.actor);
    }

    pub(super) fn prepare_worms(&mut self, world: &ArenaTerrainView, geometry: ArenaVoxelGeometry) {
        if !self.actors.iter().any(|a| a.species == Species::Worm) {
            return;
        }
        self.burrow_query.refresh(world);
        self.encounter
            .worms
            .retain(|id, _| self.actors.iter().any(|a| a.id == *id && a.hp > 0.0));
        for actor in self
            .actors
            .iter_mut()
            .filter(|a| a.species == Species::Worm && a.hp > 0.0)
        {
            let controller = self
                .encounter
                .worms
                .entry(actor.id)
                .or_insert_with(|| Controller::new(actor));
            refresh_exposure(actor, controller.surface, controller.phase, world, geometry);
        }
    }

    pub(super) fn refresh_worm_heads(
        &mut self,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
    ) {
        for actor in self
            .actors
            .iter_mut()
            .filter(|a| a.species == Species::Worm && a.hp > 0.0)
        {
            if let Some(controller) = self.encounter.worms.get(&actor.id) {
                refresh_exposure(actor, controller.surface, controller.phase, world, geometry);
            }
        }
    }

    fn request_burrow(&mut self, actor: ActorId, volume: Vec<TilePos>, out: &mut CommandsOut) {
        if self.pending_burrows.contains_key(&actor) {
            return;
        }
        let Some(generation) = self.generation else {
            return;
        };
        let next = self.next_burrow.entry(actor).or_default();
        let Some(sequence) = next.checked_add(1) else {
            return;
        };
        *next = sequence;
        let request = ArenaBurrowRequest {
            generation,
            actor,
            sequence,
            volume,
        };
        if request.structural_rejection().is_some() {
            return;
        }
        self.pending_burrows.insert(actor, request.clone());
        out.burrows.push(request);
    }

    pub(super) fn worm_release_aim(
        &self,
        actor: &Actor,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        tuning: &ArenaTuning,
    ) -> Option<Vec3> {
        let controller = self.encounter.worms.get(&actor.id)?;
        if controller.phase != WormPhase::Exposed {
            return None;
        }
        let mut current = actor.clone();
        refresh_exposure(
            &mut current,
            controller.surface,
            controller.phase,
            world,
            geometry,
        );
        release_aim(
            &current,
            controller.target?,
            &self.collision,
            world,
            geometry,
            tuning,
            self.tick,
        )
    }

    /// Earth locomotion owns depth while embedded or supported. Once the entire
    /// body is in unsupported air, gravity settles it through air only; falling
    /// cannot authorize conversion or reuse a pending burrow proposal.
    fn settle_airborne_worm(
        &self,
        actor: &mut Actor,
        control: &mut Controller,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        materials: ArenaMaterials,
        depth: u8,
    ) -> bool {
        let Some(before) = pose(actor) else {
            return false;
        };
        if !self
            .burrow_query
            .above_ground_clear(before, world, geometry)
            || shapes::ground(&self.collision, actor, actor.feet, SKIN * 8.0).is_some()
        {
            return false;
        }
        let gravity = 17.333_334; // The existing M01 gravity, without jump control.
        let fall = actor.body.vertical_velocity * STEP - 0.5 * gravity * STEP * STEP;
        actor.body.vertical_velocity -= gravity * STEP;
        let mut delta = actor.body.impulse_velocity * STEP + Vec3::Y * fall;
        let context = BurrowContext {
            world,
            policy: &self.burrow_policy,
            dirt: materials.dirt,
            bodies: &self.actors,
            owner: actor.id,
            geometry,
        };
        let clear = |delta: Vec3, fraction: f32| {
            let after = PrismPose {
                feet: before.feet + delta * fraction,
                ..before
            };
            worm_geometry::swept_prism_cells(before, after, geometry).is_ok_and(|cells| {
                cells.iter().all(|cell| {
                    world
                        .voxels
                        .get(cell)
                        .is_none_or(|material| *material == hex_core::SubstanceId::AIR)
                }) && matches!(
                    self.burrow_query.admit(before, after, &context),
                    Admission::Clear
                )
            })
        };
        let admitted_fraction = |delta| {
            if clear(delta, 1.0) {
                return 1.0;
            }
            // Find the last fully admitted pose on this single linear path.
            // Twelve subdivisions stop within ordinary collision skin for a
            // normal gravity step, including all-mask objects and live bodies.
            let mut low = 0.0;
            let mut high = 1.0;
            for _ in 0..12 {
                let middle = (low + high) * 0.5;
                if clear(delta, middle) {
                    low = middle;
                } else {
                    high = middle;
                }
            }
            low
        };
        let mut fraction = admitted_fraction(delta);
        if fraction < 1.0 && delta.y < 0.0 && delta.with_y(0.0).length_squared() > SKIN * SKIN {
            // A sideways impulse against a wall cannot cancel gravity. Choose
            // one vertical alternative instead of committing two legs; retained
            // horizontal momentum remains independently swept on later ticks.
            let vertical = Vec3::Y * delta.y;
            let vertical_fraction = admitted_fraction(vertical);
            if vertical_fraction > fraction {
                delta = vertical;
                fraction = vertical_fraction;
            }
        }
        if fraction < 1.0 {
            fraction = (fraction - SKIN / delta.length().max(SKIN)).max(0.0);
        }
        actor.feet += delta * fraction;
        if fraction < 1.0 && delta.y < 0.0 {
            actor.body.vertical_velocity = 0.0;
        }
        actor.body.impulse_velocity *= (-3.0 * STEP).exp();
        actor.grounded = false;
        actor.body.grounded = false;
        actor.flying = false;
        if shapes::ground(&self.collision, actor, actor.feet, SKIN * 8.0).is_some() {
            actor.body.vertical_velocity = 0.0;
            if let Some(surface) = pose(actor)
                .and_then(|body| band(body, actor.feet.y, world, geometry, depth))
                .filter(|surface| *surface <= actor.feet.y + SKIN)
            {
                control.surface = surface;
            }
        }
        control.blocked = 0.0;
        refresh_exposure(actor, control.surface, control.phase, world, geometry);
        true
    }

    pub(super) fn move_worms(
        &mut self,
        intents: &BTreeMap<ActorId, brain::MotionIntent>,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        materials: ArenaMaterials,
        tuning: &ArenaTuning,
        out: &mut CommandsOut,
    ) {
        let ids: Vec<_> = self
            .actors
            .iter()
            .filter(|a| a.species == Species::Worm && a.hp > 0.0)
            .map(|a| a.id)
            .collect();
        for id in ids {
            let Some(mut control) = self.encounter.worms.remove(&id) else {
                continue;
            };
            let Some(mut actor) = self.actors.iter().find(|a| a.id == id).cloned() else {
                continue;
            };
            let Some(before) = pose(&actor) else {
                continue;
            };
            let c = &tuning.encounters;
            if self.settle_airborne_worm(
                &mut actor,
                &mut control,
                world,
                geometry,
                materials,
                c.worm_depth_levels,
            ) {
                if let Some(slot) = self.actors.iter_mut().find(|a| a.id == id) {
                    *slot = actor;
                }
                self.encounter.worms.insert(id, control);
                continue;
            }
            actor.body.impulse_velocity.y += actor.body.vertical_velocity;
            actor.body.vertical_velocity = 0.0;
            let impulse = actor.body.impulse_velocity * STEP;
            let wanted = if control.phase == WormPhase::Travel
                && control.physically_buried(&actor, world, geometry, c.worm_depth_levels)
            {
                intents.get(&id).map_or(Vec3::ZERO, |i| i.direction)
            } else {
                Vec3::ZERO
            };
            let trial = (self.tick + u64::from(id) * 3).is_multiple_of(20);
            let turns: &[f32] = if trial && control.blocked > STEP {
                &[
                    0.0,
                    std::f32::consts::FRAC_PI_3,
                    -std::f32::consts::FRAC_PI_3,
                    std::f32::consts::FRAC_PI_3 * 2.0,
                    -std::f32::consts::FRAC_PI_3 * 2.0,
                ]
            } else {
                &[0.0]
            };
            let mut accepted = None;
            for angle in turns {
                let direction = Quat::from_rotation_y(*angle) * wanted;
                let target_heading = if direction.length_squared() > SKIN * SKIN {
                    (-direction.x).atan2(-direction.z)
                } else {
                    control.heading
                };
                let change = (target_heading - control.heading + std::f32::consts::PI)
                    .rem_euclid(std::f32::consts::TAU)
                    - std::f32::consts::PI;
                let mut heading = control.heading
                    + change.clamp(-c.worm_turn_speed * STEP, c.worm_turn_speed * STEP);
                let mut feet = actor.feet + direction.clamp_length_max(1.0) * c.worm_speed * STEP;
                let Some(proposed_parts) =
                    WormBodyState::parts_at(c.worm_segments, heading, control.lift)
                else {
                    continue;
                };
                let candidate = PrismPose {
                    feet: feet + impulse,
                    parts: proposed_parts,
                };
                let travel_band = band(
                    candidate,
                    control.surface,
                    world,
                    geometry,
                    c.worm_depth_levels,
                );
                let surface = if matches!(control.phase, WormPhase::Emerging | WormPhase::Exposed)
                    || (control.phase == WormPhase::Diving && travel_band.is_none())
                {
                    // A stationary head rise need not invent a common travel
                    // band beneath a tail spanning an already existing hole.
                    // The head still needs actual local support, and the whole
                    // component sweep still requires current world admission.
                    head_supports(candidate, control.surface, world, geometry).and_then(
                        |supports| {
                            supports
                                .into_iter()
                                .map(|p| geometry.top(p))
                                .max_by(f32::total_cmp)
                        },
                    )
                } else {
                    travel_band
                };
                let Some(surface) = surface else {
                    continue;
                };
                let buried =
                    surface - f32::from(c.worm_depth_levels) * geometry.level_height + SKIN;
                let (height, desired_lift) = match control.phase {
                    WormPhase::Diving if travel_band.is_none() => (actor.feet.y, 0.0),
                    WormPhase::Travel | WormPhase::Diving => (buried, 0.0),
                    WormPhase::Emerging | WormPhase::Exposed => (
                        actor.feet.y,
                        (surface + geometry.level_height + SKIN - actor.feet.y).max(0.0),
                    ),
                };
                feet.y +=
                    (height - feet.y).clamp(-c.worm_rise_speed * STEP, c.worm_rise_speed * STEP);
                let lift = control.lift
                    + (desired_lift - control.lift)
                        .clamp(-c.worm_rise_speed * STEP, c.worm_rise_speed * STEP);
                let Some(mut parts) = WormBodyState::parts_at(c.worm_segments, heading, lift)
                else {
                    continue;
                };
                if control.phase == WormPhase::Travel
                    && !travel_depth(
                        PrismPose { feet, parts },
                        surface,
                        world,
                        geometry,
                        c.worm_depth_levels,
                    )
                {
                    // Ease to a new band's height at the old footprint before
                    // allowing a turn/translation to cross its depth boundary.
                    feet.x = actor.feet.x;
                    feet.z = actor.feet.z;
                    heading = control.heading;
                    parts = before.parts;
                    if !travel_depth(
                        PrismPose { feet, parts },
                        control.surface,
                        world,
                        geometry,
                        c.worm_depth_levels,
                    ) {
                        continue;
                    }
                }
                // Voluntary movement, spine turn, lift and independent impulse
                // describe ONE linear component path for this simulation tick.
                let after = PrismPose {
                    feet: feet + impulse,
                    parts,
                };
                let context = BurrowContext {
                    world,
                    policy: &self.burrow_policy,
                    dirt: materials.dirt,
                    bodies: &self.actors,
                    owner: id,
                    geometry,
                };
                match self.burrow_query.admit(before, after, &context) {
                    Admission::Clear => {
                        accepted =
                            Some((after, heading, lift, surface, direction, angle.abs() > SKIN));
                        break;
                    }
                    Admission::NeedsConversion(volume) => self.request_burrow(id, volume, out),
                    Admission::Blocked(_reason) => {}
                }
            }
            // If the voluntary proposal is blocked, the still-independent
            // impulse can propose its own single before→after path, never a
            // second leg after already committing voluntary movement.
            if accepted.is_none() && impulse.length_squared() > SKIN * SKIN {
                let after = PrismPose {
                    feet: actor.feet + impulse,
                    ..before
                };
                let context = BurrowContext {
                    world,
                    policy: &self.burrow_policy,
                    dirt: materials.dirt,
                    bodies: &self.actors,
                    owner: id,
                    geometry,
                };
                match self.burrow_query.admit(before, after, &context) {
                    Admission::Clear => {
                        accepted = Some((
                            after,
                            control.heading,
                            control.lift,
                            control.surface,
                            Vec3::ZERO,
                            false,
                        ))
                    }
                    Admission::NeedsConversion(volume) => self.request_burrow(id, volume, out),
                    Admission::Blocked(_reason) => {}
                }
            }
            if let Some((after, heading, lift, surface, direction, alternate)) = accepted {
                if let Some(body) = &mut actor.worm {
                    if body.update(after.parts).is_some() {
                        actor.dimensions = body.bounds_max - body.bounds_min;
                        let progress = actor.feet.distance(after.feet) > SKIN
                            || (control.lift - lift).abs() > SKIN;
                        control.blocked = if progress {
                            0.0
                        } else {
                            control.blocked + STEP
                        };
                        control.heading = heading;
                        control.lift = lift;
                        control.surface = surface;
                        if alternate {
                            control.committed_direction = Some(direction);
                            control.committed_goal = control.goal;
                        }
                        actor.feet = after.feet;
                        actor.body_yaw = heading;
                    }
                }
            } else {
                control.blocked += STEP;
            }
            actor.body.impulse_velocity *= (-3.0 * STEP).exp();
            actor.grounded = false;
            actor.body.grounded = false;
            actor.flying = false;
            refresh_exposure(&mut actor, control.surface, control.phase, world, geometry);
            if let Some(slot) = self.actors.iter_mut().find(|a| a.id == id) {
                *slot = actor;
            }
            self.encounter.worms.insert(id, control);
        }
    }

    pub(super) fn separate_worms(
        &mut self,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        materials: ArenaMaterials,
        out: &mut CommandsOut,
    ) {
        for left in 0..self.actors.len() {
            for right in left + 1..self.actors.len() {
                let Some(mut a) = self.actors.get(left).filter(|a| a.hp > 0.0).cloned() else {
                    continue;
                };
                let Some(mut b) = self.actors.get(right).filter(|b| b.hp > 0.0).cloned() else {
                    continue;
                };
                if a.species != Species::Worm && b.species != Species::Worm {
                    continue;
                }
                let Some(push) = body_overlap(&a, &b) else {
                    continue;
                };
                let mut blocked = false;
                let others: Vec<_> = self
                    .actors
                    .iter()
                    .filter(|p| p.id != a.id && p.id != b.id)
                    .cloned()
                    .collect();
                for (actor, delta) in [(&mut a, push), (&mut b, -push)] {
                    if actor.species == Species::Worm {
                        let Some(parts) = actor.worm.as_ref().map(|b| b.previous) else {
                            blocked = true;
                            break;
                        };
                        let before = PrismPose {
                            feet: actor.previous_feet,
                            parts,
                        };
                        let Some(current) = pose(actor) else {
                            blocked = true;
                            break;
                        };
                        let after = PrismPose {
                            feet: actor.feet + delta,
                            ..current
                        };
                        let context = BurrowContext {
                            world,
                            policy: &self.burrow_policy,
                            dirt: materials.dirt,
                            bodies: &others,
                            owner: actor.id,
                            geometry,
                        };
                        match self.burrow_query.admit(before, after, &context) {
                            Admission::Clear => actor.feet = after.feet,
                            Admission::NeedsConversion(volume) => {
                                self.request_burrow(actor.id, volume, out);
                                blocked = true;
                            }
                            Admission::Blocked(_reason) => blocked = true,
                        }
                    } else {
                        actor.feet = shapes::slide(&self.collision, actor, actor.feet, delta).0;
                    }
                }
                if blocked
                    || body_overlap(&a, &b).is_some()
                    || others.iter().any(|other| {
                        other.hp > 0.0
                            && (body_overlap(&a, other).is_some()
                                || body_overlap(&b, other).is_some())
                    })
                {
                    continue;
                }
                if let Some(slot) = self.actors.get_mut(left) {
                    *slot = a;
                }
                if let Some(slot) = self.actors.get_mut(right) {
                    *slot = b;
                }
            }
        }
    }
}

/// Finite head/heading trials over the world's elongated surfaces. The returned
/// pose is above ground; the first dive still requires ordinary conversion ack.
pub(super) fn deployment_pose(
    actor: &mut Actor,
    region: &ArenaDeploymentRegion,
    others: &[Actor],
    collision: &CollisionWorld,
    view: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
) -> Option<Vec3> {
    let count = u8::try_from(actor.body_hex_prisms().count()).ok()?;
    let mut query = worm_geometry::BurrowQuery::default();
    query.refresh(view);
    let mut surfaces: Vec<_> = region.surfaces.iter().copied().collect();
    surfaces.sort_by_key(|p| (p.coord.distance(region.preferred.coord), *p));
    let width = hex_core::config::HEX_SMALL_DIAMETER;
    let directions = [
        Vec3::X * width,
        Vec3::new(width * 0.5, 0.0, 1.5),
        Vec3::new(-width * 0.5, 0.0, 1.5),
        Vec3::NEG_X * width,
        Vec3::new(-width * 0.5, 0.0, -1.5),
        Vec3::new(width * 0.5, 0.0, -1.5),
    ];
    for surface in surfaces {
        if !view.voxels.contains_key(&surface) {
            continue;
        }
        for behind in directions {
            let heading = behind.x.atan2(behind.z);
            let parts = WormBodyState::parts_at(count, heading, 0.0)?;
            let feet = surface.coord.to_world(geometry.top(surface) + SKIN);
            let body = PrismPose { feet, parts };
            let footprint = worm_geometry::footprint_columns(body)?;
            if !footprint.iter().all(|coord| {
                let support = TilePos::new(*coord, surface.level);
                region.surfaces.contains(&support) && view.voxels.contains_key(&support)
            }) || !worm_geometry::contained(body, geometry)
            {
                continue;
            }
            let mut candidate = actor.clone();
            candidate.set_observed_prisms(parts)?;
            candidate.feet = feet;
            candidate.previous_feet = feet;
            candidate.body_yaw = heading;
            candidate.previous_yaw = heading;
            if !query.above_ground_clear(body, view, geometry)
                || !shapes::clear(collision, &candidate, feet, heading)
                || !dry(&candidate, view, geometry)
                || others
                    .iter()
                    .any(|other| other.hp > 0.0 && body_overlap(&candidate, other).is_some())
            {
                continue;
            }
            *actor = candidate;
            return Some(feet);
        }
    }
    None
}
