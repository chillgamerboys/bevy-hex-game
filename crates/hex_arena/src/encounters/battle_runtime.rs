//! Reset-time spectator admission and whole-team results, using the normal authority.

use super::*;
use hex_core::arena::ArenaDeploymentRegion;

impl ArenaSession {
    pub(super) fn observe_battle_parties(&mut self, tuning: &ArenaTuning) {
        for party in &mut self.encounter.runtime {
            if party.snapshot.phase == PartyPhase::Cleared
                || !(self.tick + u64::from(party.snapshot.id) * 4).is_multiple_of(12)
            {
                continue;
            }
            let members: Vec<_> = self
                .actors
                .iter()
                .filter(|a| a.hp > 0.0 && a.party == Some(party.snapshot.id))
                .collect();
            let Some(member) = members.first() else {
                continue;
            };
            let previous = self
                .encounter
                .battle_seen
                .get(&party.snapshot.id)
                .map_or(&[][..], Vec::as_slice);
            let mut seen = BTreeMap::new();
            for observer in &members {
                for target in targeting::observe(
                    observer,
                    &self.actors,
                    previous,
                    &self.collision,
                    self.tick,
                    tuning.bot.prediction_seconds,
                ) {
                    seen.insert(target.body.id, target);
                }
            }
            let mut seen: Vec<_> = seen.into_values().collect();
            let distance = |target: &targeting::ObservedTarget| {
                members
                    .iter()
                    .map(|a| a.center().distance_squared(target.center()))
                    .fold(f32::INFINITY, f32::min)
            };
            seen.sort_by(|a, b| {
                distance(a)
                    .total_cmp(&distance(b))
                    .then_with(|| a.body.id.cmp(&b.body.id))
            });
            if let Some(target) = seen.first().copied() {
                party.knowledge = Some(Knowledge {
                    point: target.body.feet,
                    velocity: target.body.velocity,
                    tick: self.tick,
                    direct: true,
                    cue_kind: None,
                    observed: Some(target),
                });
                party.last_sight = self.tick;
            }
            self.encounter.battle_seen.insert(party.snapshot.id, seen);
            let previous_cue = party.last_cue_id;
            if let Some(last) = self.combat_cues.last() {
                party.last_cue_id = Some(last.id);
            }
            if let Some(cue) = self.combat_cues.iter().rev().find(|cue| {
                cue.team != member.team
                    && previous_cue.is_none_or(|id| cue.id > id)
                    && self.tick.saturating_sub(cue.tick) <= 120
                    && party.knowledge.is_none_or(|k| cue.tick > k.tick)
                    && members
                        .iter()
                        .any(|a| a.center().distance(cue.position) <= tuning.bot.cue_radius)
            }) {
                party.knowledge = Some(Knowledge {
                    point: cue.position,
                    velocity: Vec3::ZERO,
                    tick: cue.tick,
                    direct: false,
                    cue_kind: Some(cue.kind),
                    observed: None,
                });
            }
            if party
                .knowledge
                .is_some_and(|k| elapsed(self.tick, k.tick) > party.search)
            {
                party.knowledge = None;
            }
        }
    }

    /// The admitted player identity; spectators have no body, HP, or cast input.
    #[must_use]
    pub fn human_actor_id(&self) -> Option<ActorId> {
        (self.accepted_battle.control == ArenaControl::Player).then_some(0)
    }

    /// Both legacy player rounds and spectator results stop fixed simulation.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.outcome.is_some() || self.battle_result.is_some()
    }

    /// Reset-accepted setup; editing the requested resource cannot relabel a round.
    #[must_use]
    pub fn accepted_battle_setup(&self) -> &ArenaBattleSetup {
        &self.accepted_battle
    }

    /// Exact team accounting in accepted roster order, without mutating the round.
    #[must_use]
    #[expect(
        clippy::cast_precision_loss,
        reason = "bounded local battle tick counts"
    )]
    pub fn battle_summary(&self) -> Option<BattleSummary> {
        (self.accepted_battle.control == ArenaControl::Spectator).then(|| BattleSummary {
            seed: self.accepted_battle.seed,
            ticks: self.tick,
            seconds: self.tick as f32 * STEP,
            teams: self
                .accepted_battle
                .rosters
                .iter()
                .map(|roster| {
                    let initial = self.battle_initial.iter().find(|t| t.team == roster.team);
                    BattleTeamSummary {
                        team: roster.team,
                        initial: initial.map_or(0, |t| t.initial),
                        living: self
                            .actors
                            .iter()
                            .filter(|a| a.team == roster.team && a.hp > 0.0)
                            .count(),
                        hp: self
                            .actors
                            .iter()
                            .filter(|a| a.team == roster.team)
                            .map(|a| a.hp.max(0.0))
                            .sum(),
                        max_hp: initial.map_or(0.0, |t| t.max_hp),
                    }
                })
                .collect(),
            result: self.battle_result.clone(),
        })
    }

    pub(super) fn initialize_battle(
        &mut self,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        tuning: &ArenaTuning,
    ) {
        let setup = self.accepted_battle.clone();
        let admitted = deploy(&setup, world, geometry, tuning, &self.collision);
        self.encounter = EncounterState {
            initialized: true,
            ..Default::default()
        };
        match admitted {
            Ok((actors, parties, brains)) => {
                self.battle_initial = setup
                    .rosters
                    .iter()
                    .map(|roster| {
                        let max_hp = actors
                            .iter()
                            .filter(|a| a.team == roster.team)
                            .map(|a| a.max_hp)
                            .sum();
                        BattleTeamSummary {
                            team: roster.team,
                            initial: roster.actor_count(),
                            living: roster.actor_count(),
                            hp: max_hp,
                            max_hp,
                        }
                    })
                    .collect();
                self.actors = actors;
                self.encounter.runtime = parties;
                self.encounter.brains = brains;
                for actor in &self.actors {
                    self.encounter
                        .stats
                        .insert(actor.id, ActorCombatStats::default());
                }
                self.publish_parties();
            }
            Err(reason) => {
                self.actors.clear();
                self.encounter.spawn_failed = true;
                self.notice.clone_from(&reason);
                self.battle_result = Some(BattleResult::InvalidSetup(reason));
            }
        }
    }

    pub(super) fn finish_battle_tick(&mut self) {
        let living: Vec<_> = self
            .accepted_battle
            .rosters
            .iter()
            .filter(|team| {
                self.actors
                    .iter()
                    .any(|a| a.team == team.team && a.hp > 0.0)
            })
            .map(|team| team.team)
            .collect();
        self.battle_result = match living.as_slice() {
            [] => Some(BattleResult::Draw),
            [winner] => Some(BattleResult::TeamWinner(*winner)),
            _ if self
                .accepted_battle
                .tick_limit
                .is_some_and(|limit| self.tick >= limit) =>
            {
                Some(BattleResult::Timeout)
            }
            _ => None,
        };
    }
}

type Deployment = (
    Vec<Actor>,
    Vec<PartyRuntime>,
    BTreeMap<ActorId, brain::Brain>,
);

fn deploy(
    setup: &ArenaBattleSetup,
    world: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    tuning: &ArenaTuning,
    collision: &CollisionWorld,
) -> Result<Deployment, String> {
    setup
        .validate_for(world.selection.map)
        .map_err(|e| e.to_string())?;
    let regions = world
        .battle_deployment
        .as_ref()
        .ok_or("World has no spectator deployment surfaces")?;
    let mut actors = Vec::new();
    let mut parties = Vec::new();
    let mut brains = BTreeMap::new();
    let mut next_id = 0_u8;
    for (side, roster) in setup.rosters.iter().enumerate() {
        let region = regions.get(side).ok_or("Missing deployment side")?;
        let opposite = regions
            .get(1_usize.saturating_sub(side))
            .ok_or("Missing opposing deployment side")?;
        let home = region
            .preferred
            .coord
            .to_world(geometry.top(region.preferred) + SKIN);
        let search = opposite
            .preferred
            .coord
            .to_world(geometry.top(opposite.preferred) + SKIN);
        let mut pending = Vec::new();
        for members in &roster.parties {
            let party = u16::try_from(parties.len())
                .map_err(|error| format!("Too many parties: {error}"))?;
            for species in members {
                let mut actor =
                    Actor::spawn(next_id, home, (search - home).normalize_or(Vec3::NEG_Z));
                next_id = next_id.checked_add(1).ok_or("Too many actors")?;
                actor.species = *species;
                actor.team = roster.team;
                actor.party = Some(party);
                let c = &tuning.encounters;
                actor.configure_species(*species, c);
                pending.push(actor);
            }
            parties.push(PartyRuntime {
                snapshot: PartySnapshot {
                    id: party,
                    phase: PartyPhase::Active,
                    home,
                    living: members.len(),
                },
                knowledge: None,
                last_sight: 0,
                last_cue_id: None,
                leash: 0.0,
                search: match members.first() {
                    Some(Species::Dragon) => tuning.encounters.dragon_search,
                    Some(Species::Shadow) => tuning.encounters.shadow_search,
                    _ => tuning.encounters.ground_search,
                },
                battle_search: Some(search),
            });
        }
        // Reserve the largest real footprints first, while retaining roster IDs.
        pending.sort_by(|a, b| {
            (a.species == Species::Wisp)
                .cmp(&(b.species == Species::Wisp))
                .then_with(|| {
                    (b.dimensions.x * b.dimensions.z).total_cmp(&(a.dimensions.x * a.dimensions.z))
                })
                .then_with(|| a.id.cmp(&b.id))
        });
        for mut actor in pending {
            let feet = if actor.species == Species::Wisp {
                flying_deployment_pose(&actor, region, &actors, collision, world, geometry, tuning)
                    .map(|(feet, layer)| {
                        actor.flight_layer = Some(layer);
                        feet
                    })
            } else {
                deployment_pose(&actor, region, &actors, collision, world, geometry)
            }
            .ok_or_else(|| {
                format!(
                    "No complete dry deployment for team {} {:?} actor {}",
                    actor.team, actor.species, actor.id
                )
            })?;
            actor.feet = feet;
            actor.previous_feet = feet;
            actor.flying = actor.species == Species::Wisp;
            actor.grounded = !actor.flying;
            actor.body.grounded = !actor.flying;
            brains.insert(
                actor.id,
                brain::Brain::for_battle(actor.id, feet, setup.seed),
            );
            actors.push(actor);
        }
    }
    actors.sort_by_key(|a| a.id);
    Ok((actors, parties, brains))
}

pub(super) fn deployment_pose(
    actor: &Actor,
    region: &ArenaDeploymentRegion,
    others: &[Actor],
    collision: &CollisionWorld,
    view: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
) -> Option<Vec3> {
    let mut surfaces: Vec<_> = region.surfaces.iter().copied().collect();
    surfaces.sort_by_key(|pos| (pos.coord.distance(region.preferred.coord), *pos));
    surfaces.into_iter().find_map(|surface| {
        if !view.voxels.contains_key(&surface) {
            return None;
        }
        let feet = surface.coord.to_world(geometry.top(surface) + SKIN);
        let mut body = actor.clone();
        body.feet = feet;
        if !shapes::clear(collision, &body, feet, body.body_yaw)
            || !dry(&body, view, geometry)
            || shapes::ground(collision, &body, feet, SKIN * 8.0).is_none()
            || others.iter().any(|a| body_overlap(&body, a).is_some())
        {
            return None;
        }
        // Lower the actual body slightly to identify every supporting column.
        body.feet.y -= SKIN * 4.0;
        let footprint_ok = surface.coord.within_radius(4).into_iter().all(|coord| {
            let support = TilePos::new(coord, surface.level);
            !shapes::voxel_overlap(support, geometry, &body)
                || (region.surfaces.contains(&support) && view.voxels.contains_key(&support))
        });
        footprint_ok.then_some(feet)
    })
}

/// Fourteen finite reservations: two flight layers over the seven authored cells.
/// Bodies reserve their actual union, including mixed ground/flying rosters.
pub(super) fn flying_deployment_pose(
    actor: &Actor,
    region: &ArenaDeploymentRegion,
    others: &[Actor],
    collision: &CollisionWorld,
    view: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    tuning: &ArenaTuning,
) -> Option<(Vec3, u8)> {
    let mut surfaces: Vec<_> = region.surfaces.iter().copied().collect();
    surfaces.sort_by_key(|pos| (pos.coord.distance(region.preferred.coord), *pos));
    for layer in 0_u8..2 {
        for surface in &surfaces {
            if !view.voxels.contains_key(surface) {
                continue;
            }
            let ground = surface.coord.to_world(geometry.top(*surface) + SKIN);
            let mut body = actor.clone();
            body.feet = ground - Vec3::Y * SKIN * 4.0;
            if surface.coord.within_radius(2).into_iter().any(|coord| {
                let support = TilePos::new(coord, surface.level);
                shapes::voxel_overlap(support, geometry, &body)
                    && !(region.surfaces.contains(&support) && view.voxels.contains_key(&support))
            }) {
                continue;
            }
            let rise = tuning.encounters.wisp_cruise_height
                + f32::from(layer) * tuning.encounters.wisp_layer_spacing;
            body.feet = ground + Vec3::Y * rise;
            if !shapes::clear(collision, &body, body.feet, 0.0)
                || !dry(&body, view, geometry)
                || !steering::contained(&body, geometry)
                || others
                    .iter()
                    .any(|other| body_overlap(&body, other).is_some())
            {
                continue;
            }
            // No above-roof placement: the complete prism's vertical corridor
            // from its admitted surface must be clear of terrain and props.
            let mut base = body.clone();
            base.feet = ground;
            if !shapes::clear(collision, &base, ground, 0.0)
                || shapes::sweep(collision, &base, ground, Vec3::Y * rise).is_some()
            {
                continue;
            }
            return Some((body.feet, layer));
        }
    }
    None
}
