//! Atomic admission of the complete gameplay-authored expedition roster.

use super::*;
use hex_core::arena::ArenaExpeditionSites;

mod confinement;
pub(super) use confinement::ShadowArena;
mod rally;
pub(super) use rally::Control;
pub use rally::ExpeditionRallySnapshot;

pub(crate) const CAMP_COUNTS: [usize; 14] = [3, 3, 3, 3, 3, 5, 5, 5, 9, 9, 11, 13, 15, 20];

fn roster() -> Vec<(String, Vec<ExpeditionRole>)> {
    let mut result: Vec<_> = CAMP_COUNTS
        .into_iter()
        .enumerate()
        .map(|(index, count)| {
            let role = if index < 5 {
                ExpeditionRole::BabyGoblin
            } else {
                ExpeditionRole::Goblin
            };
            let mut members = vec![role; count];
            if index == 11 || index == 12 {
                members.insert(0, ExpeditionRole::Shaman);
            }
            (format!("forest_camp_{:02}", index + 1), members)
        })
        .collect();
    result.extend([
        ("forest_troll".into(), vec![ExpeditionRole::Troll]),
        ("dragon_lower".into(), vec![ExpeditionRole::Dragon]),
        ("dragon_middle".into(), vec![ExpeditionRole::Dragon]),
        ("dragon_upper".into(), vec![ExpeditionRole::Dragon]),
        (
            "mountain_shadow".into(),
            vec![ExpeditionRole::MountainShadow],
        ),
    ]);
    for index in 1..=3 {
        result.push((
            format!("plain_golem_{index:02}"),
            vec![ExpeditionRole::PlainGolem],
        ));
    }
    for (index, count) in [3, 3, 4].into_iter().enumerate() {
        result.push((
            format!("plain_wisp_{:02}", index + 1),
            vec![ExpeditionRole::PlainWisp; count],
        ));
    }
    result
}

impl ArenaSession {
    pub(crate) fn reward_standing_pose(
        &self,
        feet: Vec3,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
    ) -> bool {
        let mut body = Actor::spawn(0, feet, Vec3::NEG_Z);
        body.configure_expedition_player();
        feet.is_finite()
            && steering::contained(&body, geometry)
            && shapes::clear(&self.collision, &body, feet, body.body_yaw)
            && dry(&body, world, geometry)
            && shapes::ground(&self.collision, &body, feet, SKIN * 8.0).is_some()
    }

    pub(super) fn initialize_expedition(
        &mut self,
        sites: &ArenaExpeditionSites,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        tuning: &ArenaTuning,
    ) -> Result<(), String> {
        let specs = roster();
        if sites.encounters.len() != specs.len()
            || specs
                .iter()
                .any(|(name, _)| !sites.encounters.contains_key(name))
        {
            return Err(
                "Expedition requires fourteen camps, Troll, three Dragons, Shadow, three Golems and three Wisp packs."
                    .into(),
            );
        }
        let fountains: Vec<_> = (1..=4)
            .map(|i| format!("forest_fountain_{i:02}"))
            .chain((1..=2).map(|i| format!("mountain_fountain_{i:02}")))
            .collect();
        if sites.fountains.len() != fountains.len()
            || fountains.iter().any(|name| {
                sites
                    .fountains
                    .get(name)
                    .is_none_or(|pool| pool.cells.is_empty())
            })
        {
            return Err("Expedition requires six complete named fountain pools.".into());
        }
        let mut human = Actor::spawn(
            0,
            world.spawns.first().copied().unwrap_or(Vec3::ZERO),
            Vec3::NEG_Z,
        );
        human.configure_expedition_player();
        if !human.feet.is_finite()
            || !shapes::clear(&self.collision, &human, human.feet, human.body_yaw)
            || !dry(&human, world, geometry)
            || shapes::ground(&self.collision, &human, human.feet, SKIN * 8.0).is_none()
        {
            return Err(
                "Expedition bridge spawn does not support the complete player body.".into(),
            );
        }
        let mut actors = vec![human];
        let mut landmarks = Vec::new();
        let mut encounter = EncounterState {
            initialized: true,
            ..Default::default()
        };
        for (index, (name, roles)) in specs.into_iter().enumerate() {
            let site = sites
                .encounters
                .get(&name)
                .ok_or_else(|| format!("Missing expedition encounter {name}."))?;
            let region = &site.deployment;
            if region.surfaces.is_empty() || !region.surfaces.contains(&region.preferred) {
                return Err(format!("Encounter {name} has no complete deployment area."));
            }
            let party = u16::try_from(index)
                .map_err(|error| format!("Expedition party capacity exceeded: {error}."))?;
            let home = region
                .preferred
                .coord
                .to_world(geometry.top(region.preferred) + SKIN);
            let leader = roles.first().copied().ok_or("Empty expedition party.")?;
            let count = roles.len();
            for (slot, role) in roles.into_iter().enumerate() {
                let id = u8::try_from(actors.len())
                    .map_err(|error| format!("Expedition actor capacity exceeded: {error}."))?;
                let mut actor = Actor::spawn(id, home, Vec3::NEG_Z);
                actor.configure_expedition(role, &tuning.encounters);
                if name == "dragon_upper" {
                    actor.configure_summit_dragon();
                }
                actor.party = Some(party);
                let feet = if role == ExpeditionRole::PlainWisp {
                    battle_runtime::flying_deployment_pose(
                        &actor,
                        region,
                        &actors,
                        &self.collision,
                        world,
                        geometry,
                        tuning,
                    )
                    .map(|(feet, layer)| {
                        actor.flight_layer = Some(layer);
                        feet
                    })
                } else {
                    battle_runtime::deployment_pose(
                        &actor,
                        region,
                        &actors,
                        &self.collision,
                        world,
                        geometry,
                    )
                }
                .ok_or_else(|| format!("No complete supported pose for {role:?} in {name}."))?;
                actor.feet = feet;
                actor.previous_feet = feet;
                actor.flying = role == ExpeditionRole::PlainWisp;
                actor.body.grounded = !actor.flying;
                actor.grounded = !actor.flying;
                landmarks.push((id, name.clone()));
                let mut brain = brain::Brain::new(id, feet);
                if role == ExpeditionRole::PlainWisp {
                    brain.configure_wisp_opening(
                        slot,
                        count,
                        tuning.encounters.wisp_initial_volley_spread,
                    );
                }
                encounter.brains.insert(id, brain);
                encounter.stats.insert(id, ActorCombatStats::default());
                actors.push(actor);
            }
            let c = &tuning.encounters;
            let (leash, search) = match leader {
                ExpeditionRole::Dragon => (c.dragon_leash, c.dragon_search),
                ExpeditionRole::MountainShadow => (9.0, c.shadow_search),
                _ => (c.ground_leash, c.ground_search),
            };
            encounter.runtime.push(PartyRuntime {
                snapshot: PartySnapshot {
                    id: party,
                    phase: PartyPhase::Dormant,
                    home,
                    living: count,
                },
                knowledge: None,
                last_sight: 0,
                last_cue_id: None,
                leash,
                search,
                battle_search: None,
            });
        }
        let player_feet = actors.first().map_or(Vec3::ZERO, |actor| actor.feet);
        if actors.iter().skip(1).any(|actor| {
            actor.feet.distance(player_feet) <= tuning.encounters.activation_radius + 1.0
        }) {
            return Err("Expedition bridge spawn is inside an enemy activation range.".into());
        }
        if let Some(home) = encounter.runtime.first().map(|party| party.snapshot.home) {
            if let Some(player) = actors.first_mut() {
                player.aim = (home - player.eye()).normalize_or(Vec3::NEG_Z);
            }
        }
        encounter.expedition = Some(Control::new(sites, &actors, geometry));
        encounter.shadow_arena = ShadowArena::new(sites, &actors);
        self.actors = actors;
        for (id, name) in landmarks {
            if let Some(actor) = self.actors.iter().find(|actor| actor.id == id) {
                self.player_knowledge.register_actor(actor, &name);
            }
        }
        self.encounter = encounter;
        self.register_forest_roster();
        self.register_expedition_sites(sites);
        self.publish_parties();
        Ok(())
    }
}

#[cfg(test)]
mod tests;
