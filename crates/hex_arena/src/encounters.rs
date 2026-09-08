//! Authored multi-party encounters, isolated from the accepted Duel decision loop.

use crate::*;
use bevy_math::Vec3;
use hex_core::arena::{ArenaEncounter, ArenaMap};
use hex_core::{HexCoord, TilePos};
use std::collections::BTreeMap;

mod abilities;
mod brain;
#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy)]
struct Knowledge {
    point: Vec3,
    velocity: Vec3,
    tick: u64,
    direct: bool,
    cue_kind: Option<CombatCueKind>,
}

#[derive(Debug)]
struct PartyRuntime {
    snapshot: PartySnapshot,
    knowledge: Option<Knowledge>,
    last_sight: u64,
    last_cue_id: Option<u64>,
    leash: f32,
    search: f32,
}

#[derive(Debug, Default)]
pub(crate) struct EncounterState {
    pub initialized: bool,
    spawn_failed: bool,
    pub parties: Vec<PartySnapshot>,
    runtime: Vec<PartyRuntime>,
    brains: BTreeMap<ActorId, brain::Brain>,
    pub barriers: Vec<BarrierSnapshot>,
    pub auras: Vec<AuraSnapshot>,
    pub stats: BTreeMap<ActorId, ActorCombatStats>,
    pub ability_counts: BTreeMap<ActorId, [u32; 7]>,
    next_barrier: u64,
    human_seen_tick: u64,
}

impl ArenaSession {
    /// Active direct-attack barriers; transparent to sight, movement and cameras.
    #[must_use]
    pub fn barriers(&self) -> &[BarrierSnapshot] {
        &self.encounter.barriers
    }
    /// Current owner-centered timed support fields.
    #[must_use]
    pub fn auras(&self) -> &[AuraSnapshot] {
        &self.encounter.auras
    }
    /// Stable party states; intended for typed encounter evidence and selection UI.
    #[must_use]
    pub fn parties(&self) -> &[PartySnapshot] {
        &self.encounter.parties
    }
    /// Encounter counts without hidden actor positions.
    #[must_use]
    pub fn encounter_summary(&self) -> EncounterSummary {
        EncounterSummary {
            enabled: self.encounter.initialized,
            living_enemies: self
                .actors
                .iter()
                .filter(|a| a.id != 0 && a.hp > 0.0)
                .count(),
            active_parties: self
                .encounter
                .parties
                .iter()
                .filter(|p| p.phase == PartyPhase::Active)
                .count(),
            dormant_parties: self
                .encounter
                .parties
                .iter()
                .filter(|p| p.phase == PartyPhase::Dormant)
                .count(),
            returning_parties: self
                .encounter
                .parties
                .iter()
                .filter(|p| p.phase == PartyPhase::Returning)
                .count(),
            defeated_parties: self
                .encounter
                .parties
                .iter()
                .filter(|p| p.phase == PartyPhase::Cleared)
                .count(),
        }
    }

    fn initialize_encounter(
        &mut self,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        tuning: &ArenaTuning,
    ) {
        let c = &tuning.encounters;
        self.encounter = EncounterState {
            initialized: true,
            ..Default::default()
        };
        let human = Actor::spawn(
            0,
            world.spawns.first().copied().unwrap_or(Vec3::ZERO),
            Vec3::NEG_Z,
        );
        self.actors = vec![human];
        let specs: Vec<(Vec3, Vec<Species>)> = if world.selection.map == ArenaMap::SevenRegions {
            [
                ("mountains_high_pass", vec![Species::Dragon]),
                (
                    "fort_fort_courtyard",
                    vec![
                        Species::Shaman,
                        Species::Goblin,
                        Species::Goblin,
                        Species::Goblin,
                    ],
                ),
                ("caves_cave_entrance", vec![Species::Goblin; 5]),
            ]
            .into_iter()
            .filter_map(|(name, roster)| {
                world.anchors.get(name).copied().map(|home| (home, roster))
            })
            .collect()
        } else {
            let roster = match world.selection.encounter {
                ArenaEncounter::Dragon => vec![Species::Dragon],
                ArenaEncounter::Goblins => vec![Species::Goblin; 5],
                ArenaEncounter::ShamanParty => vec![
                    Species::Shaman,
                    Species::Goblin,
                    Species::Goblin,
                    Species::Goblin,
                ],
                ArenaEncounter::Shadow => vec![Species::Shadow],
            };
            vec![(world.spawns.get(1).copied().unwrap_or(Vec3::ZERO), roster)]
        };
        for (index, (home, roster)) in specs.into_iter().enumerate() {
            let Ok(party) = u16::try_from(index) else {
                continue;
            };
            let leader = roster.first().copied().unwrap_or(Species::Goblin);
            let (leash, search) = match leader {
                Species::Dragon => (c.dragon_leash, c.dragon_search),
                Species::Shadow => (c.shadow_leash, c.shadow_search),
                _ => (c.ground_leash, c.ground_search),
            };
            let mut living = 0;
            for species in roster {
                let Ok(id) = u8::try_from(self.actors.len()) else {
                    break;
                };
                let player = self.actors.first().map_or(Vec3::ZERO, |a| a.feet);
                let mut actor = Actor::spawn(id, home, (player - home).normalize_or(Vec3::NEG_Z));
                actor.species = species;
                actor.party = Some(party);
                match species {
                    Species::Dragon => {
                        actor.max_hp = c.dragon_hp;
                        actor.dimensions =
                            Vec3::new(c.dragon_width, c.dragon_height, c.dragon_length);
                    }
                    Species::Goblin => {
                        actor.max_hp = c.goblin_hp;
                        actor.dimensions = Vec3::new(
                            c.goblin_radius * 2.0,
                            c.goblin_height,
                            c.goblin_radius * 2.0,
                        );
                    }
                    Species::Shaman => actor.max_hp = c.shaman_hp,
                    _ => {}
                }
                actor.hp = actor.max_hp;
                if let Some(feet) =
                    safe_spawn(&actor, home, &self.actors, &self.collision, world, geometry)
                {
                    actor.feet = feet;
                    actor.previous_feet = feet;
                    actor.body.grounded = true;
                    actor.grounded = true;
                    self.encounter
                        .brains
                        .insert(id, brain::Brain::new(id, feet));
                    self.encounter.stats.insert(id, ActorCombatStats::default());
                    self.actors.push(actor);
                    living += 1;
                } else {
                    self.encounter.spawn_failed = true;
                    self.notice = format!(
                        "No safe dry spawn for {species:?}; reset or choose another encounter."
                    );
                }
            }
            self.encounter.runtime.push(PartyRuntime {
                snapshot: PartySnapshot {
                    id: party,
                    phase: if living > 0 {
                        PartyPhase::Dormant
                    } else {
                        PartyPhase::Cleared
                    },
                    home,
                    living,
                },
                knowledge: None,
                last_sight: 0,
                last_cue_id: None,
                leash,
                search,
            });
        }
        if self.encounter.runtime.is_empty() {
            self.encounter.spawn_failed = true;
            self.notice = "The published world is missing required encounter anchors.".into();
        }
        // World authors the starting region; gameplay checks the actual player
        // body and group exclusion before admitting a nearby dry fallback.
        if let Some(mut human) = self.actors.first().cloned() {
            let far = |feet: Vec3| {
                self.actors
                    .iter()
                    .filter(|a| a.id != 0 && a.hp > 0.0)
                    .all(|a| feet.distance(a.feet) > c.activation_radius + 1.0)
            };
            if !shapes::clear(&self.collision, &human, human.feet, human.body_yaw)
                || !dry(&human, world, geometry)
                || !far(human.feet)
            {
                let mut candidates = Vec::new();
                for coord in HexCoord::from_world(human.feet).within_radius(10) {
                    let desired = coord.to_world(human.feet.y);
                    if far(desired) {
                        candidates.push(desired);
                    }
                }
                candidates.sort_by(|a, b| {
                    a.distance_squared(human.feet)
                        .total_cmp(&b.distance_squared(human.feet))
                });
                if let Some(feet) = candidates.into_iter().find_map(|p| {
                    safe_spawn(
                        &human,
                        p,
                        self.actors.get(1..).unwrap_or(&[]),
                        &self.collision,
                        world,
                        geometry,
                    )
                    .filter(|p| far(*p))
                }) {
                    human.feet = feet;
                    human.previous_feet = feet;
                    if let Some(slot) = self.actors.first_mut() {
                        *slot = human;
                    }
                } else {
                    self.encounter.spawn_failed = true;
                    self.notice =
                        "No safe dry player approach beyond encounter activation range.".into();
                }
            }
        }
        if let Some(home) = self.encounter.runtime.first().map(|p| p.snapshot.home) {
            if let Some(human) = self.actors.first_mut() {
                human.aim = (home - human.eye()).normalize_or(Vec3::NEG_Z);
            }
        }
        self.publish_parties();
    }

    fn publish_parties(&mut self) {
        for p in &mut self.encounter.runtime {
            p.snapshot.living = self
                .actors
                .iter()
                .filter(|a| a.party == Some(p.snapshot.id) && a.hp > 0.0)
                .count();
            if p.snapshot.living == 0 {
                p.snapshot.phase = PartyPhase::Cleared;
                p.knowledge = None;
            }
        }
        self.encounter.parties = self.encounter.runtime.iter().map(|p| p.snapshot).collect();
    }

    pub(super) fn wake_encounter_damage(&mut self, owner: ActorId, victim: ActorId, amount: f32) {
        if !self.encounter.initialized || owner != 0 || amount <= 0.0 {
            return;
        }
        let Some(actor) = self.actors.iter().find(|a| a.id == victim) else {
            return;
        };
        let point = (actor.center() / 2.0).round() * 2.0;
        if let Some(p) = self
            .encounter
            .runtime
            .iter_mut()
            .find(|p| Some(p.snapshot.id) == actor.party)
        {
            if p.snapshot.phase == PartyPhase::Dormant {
                p.snapshot.phase = PartyPhase::Active;
                p.knowledge = Some(Knowledge {
                    point,
                    velocity: Vec3::ZERO,
                    tick: self.tick,
                    direct: false,
                    cue_kind: None,
                });
                p.last_sight = self.tick;
            }
        }
    }

    fn observe_parties(&mut self, tuning: &ArenaTuning) {
        let Some(human) = self.actors.iter().find(|a| a.id == 0 && a.hp > 0.0) else {
            return;
        };
        for p in &mut self.encounter.runtime {
            if p.snapshot.phase == PartyPhase::Cleared {
                continue;
            }
            // Stagger groups while retaining the full ten-Hz sensing cadence.
            if !(self.tick + u64::from(p.snapshot.id) * 4).is_multiple_of(12) {
                continue;
            }
            let visible = self
                .actors
                .iter()
                .filter(|a| a.party == Some(p.snapshot.id) && a.hp > 0.0)
                .any(|a| {
                    (p.snapshot.phase != PartyPhase::Dormant
                        || a.center().distance(human.center())
                            <= tuning.encounters.activation_radius)
                        && [human.center(), human.eye()]
                            .into_iter()
                            .any(|target| self.collision.sight_clear(a.eye(), target))
                });
            if visible {
                let velocity = p
                    .knowledge
                    .filter(|k| k.direct && self.tick > k.tick && self.tick - k.tick <= 24)
                    .map_or(Vec3::ZERO, |k| {
                        ((human.feet - k.point) / elapsed(self.tick, k.tick)).clamp_length_max(9.0)
                    });
                p.knowledge = Some(Knowledge {
                    point: human.feet,
                    velocity,
                    tick: self.tick,
                    direct: true,
                    cue_kind: None,
                });
                p.last_sight = self.tick;
                if p.snapshot.phase == PartyPhase::Dormant {
                    p.snapshot.phase = PartyPhase::Active;
                }
                if p.snapshot.phase == PartyPhase::Active {
                    self.encounter.human_seen_tick = self.tick;
                }
            }
            // Consume the finite event stream once per party, even while dormant
            // or returning. Sound cannot become a delayed activation or refresh
            // the direct-sight search clock. A same-tick sighting wins over cues.
            let previous_cue = p.last_cue_id;
            if let Some(latest) = self.combat_cues.last() {
                p.last_cue_id = Some(latest.id);
            }
            if p.snapshot.phase == PartyPhase::Active {
                if let Some(cue) = self.combat_cues.iter().rev().find(|cue| {
                    cue.owner == 0
                        && previous_cue.is_none_or(|id| cue.id > id)
                        && self.tick.saturating_sub(cue.tick) <= 120
                        && p.knowledge
                            .is_none_or(|k| cue.tick > k.tick || (cue.tick == k.tick && !k.direct))
                        && self
                            .actors
                            .iter()
                            .filter(|a| a.hp > 0.0 && a.party == Some(p.snapshot.id))
                            .any(|a| a.center().distance(cue.position) <= tuning.bot.cue_radius)
                }) {
                    p.knowledge = Some(Knowledge {
                        point: cue.position,
                        velocity: Vec3::ZERO,
                        tick: cue.tick,
                        direct: false,
                        cue_kind: Some(cue.kind),
                    });
                }
            }
            if p.snapshot.phase == PartyPhase::Active {
                let exceeded = self
                    .actors
                    .iter()
                    .filter(|a| a.party == Some(p.snapshot.id) && a.hp > 0.0)
                    .any(|a| a.feet.distance(p.snapshot.home) > p.leash);
                if exceeded || elapsed(self.tick, p.last_sight) > p.search {
                    p.snapshot.phase = PartyPhase::Returning;
                }
            } else if p.snapshot.phase == PartyPhase::Returning {
                let returned = self
                    .actors
                    .iter()
                    .filter(|a| a.party == Some(p.snapshot.id) && a.hp > 0.0)
                    .all(|a| a.feet.with_y(0.0).distance(p.snapshot.home.with_y(0.0)) < 4.0);
                if returned {
                    p.snapshot.phase = PartyPhase::Dormant;
                    p.knowledge = None;
                }
            }
        }
    }

    pub(super) fn advance_encounter(
        &mut self,
        human: ActorIntent,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        materials: ArenaMaterials,
        tuning: &ArenaTuning,
    ) -> CommandsOut {
        if !self.encounter.initialized {
            self.initialize_encounter(world, geometry, tuning);
        }
        if self.encounter.spawn_failed {
            return CommandsOut::default();
        }
        self.begin_simulation_tick();
        let mut out = CommandsOut::default();
        for b in &mut self.encounter.barriers {
            b.remaining -= STEP;
        }
        self.encounter
            .barriers
            .retain(|b| b.remaining > 0.0 && b.hp > 0.0);
        self.collision.sync_barriers(&self.encounter.barriers);
        self.observe_parties(tuning);
        let mut brains = std::mem::take(&mut self.encounter.brains);
        let mut intents = BTreeMap::new();
        let mut plans = Vec::new();
        if self.bot_enabled {
            for (id, brain) in &mut brains {
                let Some(actor) = self.actors.iter().find(|a| a.id == *id && a.hp > 0.0) else {
                    continue;
                };
                let Some(p) = self
                    .encounter
                    .runtime
                    .iter()
                    .find(|p| Some(p.snapshot.id) == actor.party)
                else {
                    continue;
                };
                let (intent, request) = brain.intent(
                    actor,
                    p,
                    &self.actors,
                    &self.projectiles,
                    &self.combat_cues,
                    &self.collision,
                    world,
                    geometry,
                    tuning,
                    self.tick,
                );
                intents.insert(*id, intent);
                if let Some(request) = request {
                    plans.push((*id, request));
                }
            }
        }
        let mut casts = Vec::new();
        for actor in &mut self.actors {
            actor.previous_feet = actor.feet;
            actor.previous_yaw = actor.body_yaw;
            if actor.hp <= 0.0 {
                actor.attack = None;
                actor.cancel_charge();
                continue;
            }
            let intent = if actor.id == 0 {
                human
            } else {
                intents.get(&actor.id).map_or(
                    ActorIntent {
                        aim: actor.aim,
                        ..Default::default()
                    },
                    |i| i.input,
                )
            };
            if intent.aim.is_finite() && intent.aim.length_squared() > 0.0001 {
                actor.aim = intent.aim.normalize();
            }
            if let Some(spell) = intent.selected {
                if spell != actor.selected && actor.charge.is_some() {
                    actor.cancel_charge();
                }
                actor.selected = spell;
            }
            for cd in &mut actor.cooldowns {
                *cd = (*cd - STEP).max(0.0);
            }
            let forward = actor.aim.with_y(0.0).normalize_or(Vec3::NEG_Z);
            let direction = if actor.id == 0 {
                forward * intent.movement.y + forward.cross(Vec3::Y) * intent.movement.x
            } else {
                intents.get(&actor.id).map_or(Vec3::ZERO, |i| i.direction)
            };
            let flight = intents.get(&actor.id).is_some_and(|i| i.flight);
            motion::tick(
                actor,
                direction,
                intent.run,
                intent.jump,
                flight,
                &self.collision,
                &tuning.encounters,
            );
            if actor.feet.y < self.collision.min_y + 2.0 || !actor.feet.is_finite() {
                actor.hp = 0.0;
                actor.cancel_charge();
            }
            if actor.hp > 0.0 {
                if let Some((spell, speed)) = actor.casting(intent, tuning) {
                    casts.push((actor.id, spell, speed));
                }
            }
        }
        separate_many(&mut self.actors, &self.collision);
        self.advance_projectiles(world, geometry, materials, &mut out);
        self.advance_support(tuning);
        // Existing incoming damage resolves before simultaneous new releases.
        casts.retain(|(id, _, _)| self.actors.iter().any(|a| a.id == *id && a.hp > 0.0));
        for (id, spell, speed) in casts {
            if let Some(actor) = self.actors.iter_mut().find(|a| a.id == id) {
                let cooldown = if actor.species == Species::Shaman {
                    match spell {
                        Spell::Shield => tuning.encounters.shaman_shield_cooldown,
                        _ => tuning.encounters.shaman_fireball_cooldown,
                    }
                } else {
                    tuning.cooldown(spell)
                };
                if let Some(cd) = actor.cooldowns.get_mut(spell.index()) {
                    *cd = cooldown;
                }
            }
            self.release(
                id, spell, tuning, speed, world, geometry, materials, &mut out,
            );
        }
        for (id, request) in plans {
            self.begin_ability(&mut brains, id, request, tuning);
        }
        self.advance_abilities(&mut brains, world, geometry, materials, tuning, &mut out);
        self.encounter.brains = brains;
        self.advance_walls(world, geometry, materials, &mut out);
        self.publish_parties();
        let human_alive = self.actors.iter().any(|a| a.id == 0 && a.hp > 0.0);
        let enemy = self
            .actors
            .iter()
            .find(|a| a.id != 0 && a.hp > 0.0)
            .map(|a| a.id);
        self.outcome = match (human_alive, enemy) {
            (true, None) => Some(ArenaOutcome::Winner(0)),
            (false, Some(id)) => Some(ArenaOutcome::Winner(id)),
            (false, None) => Some(ArenaOutcome::Draw),
            _ => None,
        };
        if self.outcome.is_some() {
            self.cancel_charges();
            self.projectiles.clear();
            self.pending_walls.clear();
            self.encounter.auras.clear();
        }
        out
    }
}

#[expect(
    clippy::cast_precision_loss,
    reason = "bounded local encounter tick durations"
)]
fn elapsed(tick: u64, since: u64) -> f32 {
    tick.saturating_sub(since) as f32 * STEP
}

fn dry(actor: &Actor, view: &ArenaTerrainView, geometry: ArenaVoxelGeometry) -> bool {
    !view.liquids.iter().any(|run| {
        let bottom = geometry.top(run.bottom) - geometry.level_height;
        let top = geometry.top(TilePos::new(run.bottom.coord, run.top_level));
        actor.feet.y < top - SKIN
            && actor.feet.y + actor.dimensions.y > bottom + SKIN
            && run.bottom.coord.to_world(actor.feet.y).distance(actor.feet)
                < actor.dimensions.x.max(actor.dimensions.z) * 0.5 + 1.0
    })
}

fn safe_spawn(
    actor: &Actor,
    desired: Vec3,
    others: &[Actor],
    collision: &CollisionWorld,
    view: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
) -> Option<Vec3> {
    let mut candidates = Vec::new();
    for coord in HexCoord::from_world(desired).within_radius(6) {
        if !geometry.contains_column(coord) {
            continue;
        }
        if let Some(runs) = view.columns.get(&coord) {
            for run in runs {
                candidates
                    .push(coord.to_world(geometry.top(TilePos::new(coord, run.top_level)) + SKIN));
            }
        } else {
            for (pos, _) in view
                .voxels
                .range(TilePos::new(coord, i32::MIN)..=TilePos::new(coord, i32::MAX))
            {
                candidates.push(coord.to_world(geometry.top(*pos) + SKIN));
            }
        }
    }
    candidates.sort_by(|a, b| {
        a.distance_squared(desired)
            .total_cmp(&b.distance_squared(desired))
    });
    candidates.into_iter().find(|feet| {
        if (feet.y - desired.y).abs() > 4.0 {
            return false;
        }
        let mut test = actor.clone();
        test.feet = *feet;
        shapes::clear(collision, &test, *feet, test.body_yaw)
            && dry(&test, view, geometry)
            && shapes::ground(collision, &test, *feet, SKIN * 8.0).is_some()
            && others
                .iter()
                .filter(|a| a.hp > 0.0)
                .all(|a| body_overlap(&test, a).is_none())
    })
}

fn body_overlap(a: &Actor, b: &Actor) -> Option<Vec3> {
    if a.feet.y >= b.feet.y + b.dimensions.y || b.feet.y >= a.feet.y + a.dimensions.y {
        return None;
    }
    let delta = (a.center() - b.center()).with_y(0.0);
    let mut axes = vec![Vec3::X, Vec3::Z];
    for actor in [a, b] {
        if actor.species == Species::Dragon {
            axes.extend([
                actor.body_rotation() * Vec3::X,
                actor.body_rotation() * Vec3::Z,
            ]);
        }
    }
    if delta.length_squared() > SKIN * SKIN {
        axes.push(delta.normalize());
    }
    let extent = |actor: &Actor, axis: Vec3| {
        if actor.species == Species::Dragon {
            (actor.body_rotation() * Vec3::X).dot(axis).abs() * actor.dimensions.x * 0.5
                + (actor.body_rotation() * Vec3::Z).dot(axis).abs() * actor.dimensions.z * 0.5
        } else {
            actor.dimensions.x * 0.5
        }
    };
    let mut best = None;
    let mut depth = f32::INFINITY;
    for axis in axes {
        let overlap = extent(a, axis) + extent(b, axis) - delta.dot(axis).abs();
        if overlap <= 0.0 {
            return None;
        }
        if overlap < depth {
            depth = overlap;
            best = Some(if delta.dot(axis) < 0.0 { -axis } else { axis });
        }
    }
    best.map(|axis| axis * (depth * 0.5 + SKIN))
}

fn separate_many(actors: &mut [Actor], world: &CollisionWorld) {
    for _ in 0..4 {
        let mut changed = false;
        for i in 0..actors.len() {
            let (left, right) = actors.split_at_mut(i + 1);
            let Some(a) = left.last_mut() else {
                continue;
            };
            if a.hp <= 0.0 {
                continue;
            }
            for b in right.iter_mut().filter(|a| a.hp > 0.0) {
                if let Some(push) = body_overlap(a, b) {
                    let fa = shapes::slide(world, a, a.feet, push).0;
                    let fb = shapes::slide(world, b, b.feet, -push).0;
                    changed |= fa.distance_squared(a.feet) > SKIN * SKIN
                        || fb.distance_squared(b.feet) > SKIN * SKIN;
                    a.feet = fa;
                    b.feet = fb;
                }
            }
        }
        if !changed {
            break;
        }
    }
}

/// Per-actor counters for the selected map encounter.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EncounterActorStats {
    /// Stable actor identifier.
    pub id: ActorId,
    /// Authored physical/ability profile.
    pub species: Species,
    /// Exact spell and HP accounting.
    pub combat: ActorCombatStats,
    /// Fireball, Shield, FireCone, Bite, Swipe, Barrier, Aura ability activations.
    pub abilities: [u32; 7],
}
impl ArenaSession {
    /// Map combat accounting in stable actor order, with no decision-state mutation.
    #[must_use]
    pub fn encounter_stats(&self) -> Vec<EncounterActorStats> {
        self.actors
            .iter()
            .map(|a| EncounterActorStats {
                id: a.id,
                species: a.species,
                combat: self.encounter.stats.get(&a.id).copied().unwrap_or_default(),
                abilities: self
                    .encounter
                    .ability_counts
                    .get(&a.id)
                    .copied()
                    .unwrap_or([0; 7]),
            })
            .collect()
    }

    /// Typed validation of a living actor's full physical pose and dry support.
    /// Flying bodies require collision clearance and no liquid intersection.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn actor_pose_valid(
        &self,
        id: ActorId,
        view: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
    ) -> bool {
        self.actors.iter().find(|a| a.id == id).is_some_and(|a| {
            shapes::clear(&self.collision, a, a.feet, a.body_yaw)
                && dry(a, view, geometry)
                && (a.flying || shapes::ground(&self.collision, a, a.feet, 0.05).is_some())
        })
    }

    /// Clone an actor and continuously drive authored waypoints through the actual
    /// movement controller. At most 3,600 ticks and 32 waypoints; no live mutation.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn probe_dry_route(
        &self,
        id: ActorId,
        waypoints: &[Vec3],
        view: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        tuning: &ArenaTuning,
    ) -> bool {
        if waypoints.len() > 32 {
            return false;
        }
        let Some(mut actor) = self.actors.iter().find(|a| a.id == id).cloned() else {
            return false;
        };
        let mut ticks = 0;
        for point in waypoints {
            while actor.feet.with_y(0.0).distance(point.with_y(0.0)) > 0.3 {
                if ticks >= 3600 {
                    return false;
                }
                let before = actor.feet;
                let direction = (point - actor.feet).with_y(0.0).normalize_or_zero();
                motion::tick(
                    &mut actor,
                    direction,
                    true,
                    false,
                    false,
                    &self.collision,
                    &tuning.encounters,
                );
                ticks += 1;
                if !shapes::clear(&self.collision, &actor, actor.feet, actor.body_yaw)
                    || !dry(&actor, view, geometry)
                    || actor.feet.y < before.y - 0.45
                    || shapes::ground(&self.collision, &actor, actor.feet, 0.45).is_none()
                {
                    return false;
                }
            }
            if (actor.feet.y - point.y).abs() > 0.45 {
                return false;
            }
        }
        shapes::ground(&self.collision, &actor, actor.feet, 0.05).is_some()
    }
}

impl ArenaSession {
    /// Find a synthetic combat target area without changing combat or AI state.
    /// Reuses an existing visible area during a visit; otherwise tries sixteen
    /// role-appropriate directions/ranges through the authoritative pose query.
    /// Ordinary play never calls this explicit performance-fixture helper.
    #[must_use]
    pub fn synthetic_combat_target_pose(
        &self,
        actor: ActorId,
        representative: ActorId,
        previous: Option<Vec3>,
        view: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
    ) -> Option<Vec3> {
        if let Some(point) = previous {
            if let Some(feet) =
                self.visible_supported_actor_pose(actor, representative, point, view, geometry)
            {
                return Some(feet);
            }
        }
        let human = self.actors.iter().find(|a| a.id == actor)?;
        let target = self.actors.iter().find(|a| a.id == representative)?;
        let distances = match target.species {
            Species::Dragon => [2.5, 1.5],
            Species::Goblin => [1.1, 2.2],
            _ => [8.0, 5.0],
        };
        for distance in distances {
            for turn in [0.0, 0.25, -0.25, 0.5, -0.5, 0.75, -0.75, 1.0] {
                let forward = bevy_math::Quat::from_rotation_y(turn * std::f32::consts::PI)
                    * target.body_rotation()
                    * Vec3::NEG_Z;
                let desired =
                    target.eye() + forward * distance - Vec3::Y * (human.body_dimensions().y * 0.5);
                if let Some(feet) = self.visible_supported_actor_pose(
                    actor,
                    representative,
                    desired,
                    view,
                    geometry,
                ) {
                    return Some(feet);
                }
            }
        }
        None
    }

    /// Resolve a diagnostic placement near a requested point onto current dry
    /// support, with full body clearance and sight from a living observer.
    /// This read-only query changes no actor or AI state. It checks at most 64
    /// nearby published surfaces and is used by explicit synthetic load fixtures.
    #[must_use]
    pub fn visible_supported_actor_pose(
        &self,
        id: ActorId,
        observer: ActorId,
        desired: Vec3,
        view: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
    ) -> Option<Vec3> {
        if !desired.is_finite() {
            return None;
        }
        let mut actor = self
            .actors
            .iter()
            .find(|a| a.id == id && a.hp > 0.0)?
            .clone();
        let observer = self
            .actors
            .iter()
            .find(|a| a.id == observer && a.hp > 0.0)?;
        let desired_coord = HexCoord::from_world(desired);
        let mut candidates = Vec::new();
        for coord in desired_coord.within_radius(2) {
            if let Some(runs) = view.columns.get(&coord) {
                for run in runs {
                    let height = geometry.top(TilePos::new(coord, run.top_level)) + SKIN;
                    if (height - desired.y).abs() <= 4.0 {
                        if coord == desired_coord {
                            candidates.push(desired.with_y(height));
                        }
                        candidates.push(coord.to_world(height));
                    }
                }
            }
        }
        candidates.sort_by(|a, b| {
            a.distance_squared(desired)
                .total_cmp(&b.distance_squared(desired))
        });
        candidates.into_iter().take(64).find(|feet| {
            actor.feet = *feet;
            shapes::clear(&self.collision, &actor, *feet, actor.body_yaw)
                && dry(&actor, view, geometry)
                && shapes::ground(&self.collision, &actor, *feet, SKIN * 8.0).is_some()
                && self
                    .actors
                    .iter()
                    .filter(|a| a.id != id && a.hp > 0.0)
                    .all(|other| body_overlap(&actor, other).is_none())
                && [actor.center(), actor.eye()]
                    .into_iter()
                    .any(|point| self.collision.sight_clear(observer.eye(), point))
        })
    }

    /// Full body and dry-volume check, allowing an ordinary descending step to be
    /// airborne. Route consumers separately bound support distance and landing.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn actor_volume_valid(
        &self,
        id: ActorId,
        view: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
    ) -> bool {
        self.actors.iter().find(|a| a.id == id).is_some_and(|a| {
            shapes::clear(&self.collision, a, a.feet, a.body_yaw) && dry(a, view, geometry)
        })
    }
}

/// Disclosure-safe party observation trace for local diagnostics, never live
/// hidden actor state. Gameplay HUDs should not create enemy location indicators.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PartyKnowledgeSnapshot {
    /// Stable encounter group.
    pub id: PartyId,
    /// Last directly observed or quantized event position, if any.
    pub point: Option<[f32; 3]>,
    /// Sight, damage, heard-release, heard-impact or none.
    pub source: &'static str,
    /// Tick when the discrete observation was made, never refreshed by memory.
    pub tick: Option<u64>,
}
impl ArenaSession {
    /// Read dated party knowledge without sampling hidden actors or changing AI.
    #[must_use]
    pub fn party_knowledge(&self) -> Vec<PartyKnowledgeSnapshot> {
        self.encounter
            .runtime
            .iter()
            .map(|p| PartyKnowledgeSnapshot {
                id: p.snapshot.id,
                point: p.knowledge.map(|k| k.point.to_array()),
                source: p.knowledge.map_or("none", |k| {
                    if k.direct {
                        "sight"
                    } else {
                        match k.cue_kind {
                            Some(CombatCueKind::Release) => "heard-release",
                            Some(CombatCueKind::Impact) => "heard-impact",
                            None => "damage",
                        }
                    }
                }),
                tick: p.knowledge.map(|k| k.tick),
            })
            .collect()
    }
}
