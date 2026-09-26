//! Lazy atomic admission of the finite Grand roster from world-published sites.
use super::*;

const SITES: &[(&str, Species, usize)] = &[
    ("grand_goblin_01", Species::Goblin, 8),
    ("grand_goblin_02", Species::Goblin, 12),
    ("grand_goblin_03", Species::Goblin, 18),
    ("grand_goblin_04", Species::Goblin, 20),
    ("grand_goblin_05", Species::Goblin, 22),
    ("grand_goblin_06", Species::Goblin, 24),
    ("grand_shaman_01", Species::Shaman, 3),
    ("grand_dragon_01", Species::Dragon, 3),
    ("grand_golem_01", Species::Golem, 3),
    ("grand_wisp_01", Species::Wisp, 10),
    ("grand_worm_01", Species::Worm, 1),
    ("grand_worm_02", Species::Worm, 1),
    ("grand_worm_03", Species::Worm, 1),
    ("grand_shadow_tunnel", Species::Shadow, 1),
];

impl ArenaSession {
    pub(super) fn initialize_grand(
        &mut self,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
    ) {
        if world.expedition.is_none() {
            self.notice = "Grand expedition sites are loading.".into();
            return;
        }
        self.actors.truncate(1);
        if let Some(player) = self.actors.first_mut() {
            player.configure_expedition_player();
            player.free_flight.get_or_insert_with(Default::default);
            let marine = player.marine.get_or_insert_with(Default::default);
            marine.lab = true;
            marine.glider_wind_scale = 0.65;
        }
        self.initialize_exploration(world, geometry);
        if self.encounter.initialized {
            // Grant only ordinary XP/fountains. Forest milestone rewards stay absent.
            if let Some(state) = &mut self.progression {
                state.expedition = true;
            }
            if let Some(sites) = &world.expedition {
                self.register_expedition_sites(sites);
            }
        }
    }

    pub(super) fn admit_grand_parties(
        &mut self,
        world: &ArenaTerrainView,
        geometry: ArenaVoxelGeometry,
        tuning: &ArenaTuning,
    ) {
        let Some(sites) = &world.expedition else {
            return;
        };
        for (ordinal, &(name, leader, count)) in SITES.iter().enumerate() {
            if self
                .grand
                .as_ref()
                .is_none_or(|g| g.admitted.contains(name))
            {
                continue;
            }
            let Some(site) = sites.encounters.get(name) else {
                continue;
            };
            let region = &site.deployment;
            let home = region
                .preferred
                .coord
                .to_world(geometry.top(region.preferred) + SKIN);
            if let Some(g) = &mut self.grand {
                g.sites.insert(name.into(), home);
            }
            if self
                .actors
                .iter()
                .find(|a| a.id == 0)
                .is_none_or(|a| a.feet.distance(home) >= 100.0)
            {
                continue;
            }
            if region.surfaces.is_empty()
                || region.surfaces.iter().any(|pos| {
                    world.residency.as_ref().is_some_and(|r| {
                        r.at(pos.coord, geometry) != hex_core::arena::ArenaAvailability::Ready
                    })
                })
            {
                continue;
            }
            let Ok(party) = u16::try_from(ordinal) else {
                continue;
            };
            let home = region
                .preferred
                .coord
                .to_world(geometry.top(region.preferred) + SKIN);
            let mut admitted = Vec::new();
            for slot in 0..count {
                let Ok(id) = ActorId::try_from(ordinal * 32 + slot + 1) else {
                    break;
                };
                let (species, role) = roster_profile(leader, slot);
                let mut actor = Actor::spawn(id, home, Vec3::NEG_Z);
                if let Some(role) = role {
                    actor.configure_expedition(role, &tuning.encounters);
                } else {
                    actor.configure_species(species, &tuning.encounters);
                }
                actor.party = Some(party);
                let mut others = self.actors.clone();
                others.extend(admitted.iter().cloned());
                let feet = if species == Species::Worm {
                    worm::deployment_pose(
                        &mut actor,
                        region,
                        &others,
                        &self.collision,
                        world,
                        geometry,
                    )
                } else if species == Species::Wisp {
                    battle_runtime::flying_deployment_pose(
                        &actor,
                        region,
                        &others,
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
                        &others,
                        &self.collision,
                        world,
                        geometry,
                    )
                };
                let Some(feet) = feet else {
                    break;
                };
                if self.collision.needs_terrain(
                    feet,
                    Vec3::ZERO,
                    actor.dimensions.y,
                    actor.dimensions.x.max(actor.dimensions.z),
                ) {
                    break;
                }
                actor.feet = feet;
                actor.previous_feet = feet;
                actor.flying = species == Species::Wisp;
                actor.grounded = !actor.flying;
                actor.body.grounded = !actor.flying;
                admitted.push(actor);
            }
            if admitted.len() != count {
                continue;
            }
            for (slot, actor) in admitted.iter().enumerate() {
                let mut control = brain::Brain::new(actor.id, actor.feet);
                if actor.species == Species::Wisp {
                    control.configure_wisp_opening(
                        slot,
                        count,
                        tuning.encounters.wisp_initial_volley_spread,
                    );
                }
                self.encounter.brains.insert(actor.id, control);
                self.encounter.stats.insert(actor.id, Default::default());
                self.player_knowledge.register_actor(actor, name);
            }
            self.actors.extend(admitted);
            self.encounter.runtime.push(PartyRuntime {
                snapshot: PartySnapshot {
                    id: party,
                    phase: PartyPhase::Dormant,
                    home,
                    living: count,
                },
                knowledge: None,
                last_sight: self.tick,
                last_cue_id: None,
                leash: if leader == Species::Dragon {
                    tuning.encounters.dragon_leash
                } else {
                    tuning.encounters.ground_leash
                },
                search: if leader == Species::Dragon {
                    tuning.encounters.dragon_search
                } else {
                    tuning.encounters.ground_search
                },
                battle_search: None,
            });
            if let Some(g) = &mut self.grand {
                g.admitted.insert(name.into());
            }
            self.register_forest_roster();
            if let Some(p) = &mut self.progression {
                p.expedition = true;
            }
        }
        self.publish_parties();
    }

    pub(super) fn grand_waiting_for_actors(&self) -> bool {
        self.grand.is_some()
            && self.actors.iter().filter(|a| a.hp > 0.0).any(|a| {
                self.collision.needs_terrain(
                    a.feet,
                    a.aim * 2.0,
                    a.dimensions.y,
                    a.dimensions.x.max(a.dimensions.z) * 0.5 + 1.0,
                )
            })
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct DormantParty {
    pub(crate) actors: Vec<Actor>,
    pub(crate) projectiles: Vec<Projectile>,
    pub(super) runtime: PartyRuntime,
    brains: BTreeMap<ActorId, brain::Brain>,
    worms: BTreeMap<ActorId, worm::Controller>,
    barriers: Vec<BarrierSnapshot>,
    auras: Vec<AuraSnapshot>,
    walls: Vec<PendingWall>,
    cues: Vec<CombatCue>,
    slept_at: u64,
}
impl DormantParty {
    pub(crate) fn near(&self, player: Vec3) -> bool {
        self.actors.iter().any(|a| a.feet.distance(player) < 100.0)
    }
    pub(crate) fn near_attack(&self, player: Vec3, projectiles: &[Projectile]) -> bool {
        self.near(player)
            || projectiles.iter().filter(|p| p.owner == 0).any(|p| {
                self.actors
                    .iter()
                    .any(|a| a.feet.distance(player) < 210.0 && a.feet.distance(p.position) < 32.0)
            })
    }
}

fn extract<T>(source: &mut Vec<T>, take: impl Fn(&T) -> bool) -> Vec<T> {
    let (selected, retained) = std::mem::take(source).into_iter().partition(take);
    *source = retained;
    selected
}

impl ArenaSession {
    pub(super) fn suspend_grand_parties(&mut self) {
        let Some(player) = self.actors.iter().find(|a| a.id == 0).map(|a| a.feet) else {
            return;
        };
        let parties: Vec<_> = self.encounter.runtime.iter().filter(|p| {
            let members: Vec<_> = self.actors.iter().filter(|a| a.party == Some(p.snapshot.id)).collect();
            !members.is_empty() && members.iter().all(|a| a.feet.distance(player) > 140.0)
                && members.iter().all(|a| !self.pending_burrows.contains_key(&a.id))
                && !self.projectiles.iter().any(|s| s.position.distance(player) < 100.0 && members.iter().any(|a| a.id == s.owner))
                // A player projectile keeps its potential victims active until impact/expiry.
                && !self.projectiles.iter().filter(|s| s.owner == 0).any(|s| members.iter().any(|a| a.feet.distance(s.position) < 32.0))
        }).map(|p| p.snapshot.id).collect();
        for party in parties {
            let Some(index) = self
                .encounter
                .runtime
                .iter()
                .position(|p| p.snapshot.id == party)
            else {
                continue;
            };
            let runtime = self.encounter.runtime.remove(index);
            let actors = extract(&mut self.actors, |a| a.party == Some(party));
            let ids: std::collections::BTreeSet<_> = actors.iter().map(|a| a.id).collect();
            let brains = ids
                .iter()
                .filter_map(|id| self.encounter.brains.remove(id).map(|b| (*id, b)))
                .collect();
            let worms = ids
                .iter()
                .filter_map(|id| self.encounter.worms.remove(id).map(|w| (*id, w)))
                .collect();
            let bundle = DormantParty {
                actors,
                runtime,
                brains,
                worms,
                projectiles: extract(&mut self.projectiles, |p| ids.contains(&p.owner)),
                barriers: extract(&mut self.encounter.barriers, |b| ids.contains(&b.owner)),
                auras: extract(&mut self.encounter.auras, |a| ids.contains(&a.owner)),
                walls: extract(&mut self.pending_walls, |w| ids.contains(&w.owner)),
                cues: extract(&mut self.combat_cues, |c| ids.contains(&c.owner)),
                slept_at: self.tick,
            };
            self.encounter.dormant.insert(party, bundle);
        }
    }

    pub(super) fn wake_grand_parties(&mut self) {
        let Some(player) = self.actors.iter().find(|a| a.id == 0).map(|a| a.feet) else {
            return;
        };
        let ready: Vec<_> = self
            .encounter
            .dormant
            .iter()
            .filter(|(_, p)| {
                p.near_attack(player, &self.projectiles)
                    && p.actors.iter().filter(|a| a.hp > 0.0).all(|a| {
                        !self.collision.needs_terrain(
                            a.feet,
                            a.aim * 2.0,
                            a.dimensions.y,
                            a.dimensions.x.max(a.dimensions.z) * 0.5 + 1.0,
                        )
                    })
            })
            .map(|(id, _)| *id)
            .collect();
        for id in ready {
            let Some(mut p) = self.encounter.dormant.remove(&id) else {
                continue;
            };
            let delta = self.tick.saturating_sub(p.slept_at);
            for a in &mut p.actors {
                a.shift_clock(delta);
            }
            for b in p.brains.values_mut() {
                b.shift_clock(delta);
            }
            for w in p.worms.values_mut() {
                w.shift_clock(delta);
            }
            for c in &mut p.cues {
                c.tick = c.tick.saturating_add(delta);
            }
            p.runtime.last_sight = p.runtime.last_sight.saturating_add(delta);
            if let Some(k) = &mut p.runtime.knowledge {
                k.shift_clock(delta);
            }
            self.shift_grand_credit(&p.actors.iter().map(|a| a.id).collect(), delta);
            self.actors.extend(p.actors);
            self.projectiles.extend(p.projectiles);
            self.encounter.runtime.push(p.runtime);
            self.encounter.brains.extend(p.brains);
            self.encounter.worms.extend(p.worms);
            self.encounter.barriers.extend(p.barriers);
            self.encounter.auras.extend(p.auras);
            self.pending_walls.extend(p.walls);
            self.combat_cues.extend(p.cues);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grand_roster_keeps_existing_population_budget_and_unique_wide_ids() {
        assert_eq!(SITES.iter().map(|s| s.2).sum::<usize>(), 127);
        let ids: std::collections::BTreeSet<_> = SITES
            .iter()
            .enumerate()
            .flat_map(|(i, s)| (0..s.2).map(move |j| i * 32 + j + 1))
            .collect();
        assert_eq!(ids.len(), 127);
        assert!(ids.last().copied().unwrap() > usize::from(u8::MAX));
    }

    #[test]
    fn grand_dormancy_preserves_projectiles_brains_and_remaining_clocks() {
        let world = ArenaTerrainView {
            selection: hex_core::arena::ArenaSelection {
                map: ArenaMap::GrandV4,
                ..Default::default()
            },
            ..Default::default()
        };
        let geometry = ArenaVoxelGeometry::default();
        let mut session = ArenaSession::default();
        session.reset(1, &world, geometry);
        session.actors.truncate(1);
        session.actors[0].feet = Vec3::ZERO;
        let mut enemy = Actor::spawn(417, Vec3::X * 160.0, Vec3::NEG_X);
        enemy.configure_expedition(ExpeditionRole::MountainShadow, &EncounterTuning::default());
        session
            .grand
            .as_mut()
            .unwrap()
            .admitted
            .insert("grand_shadow_tunnel".into());
        enemy.party = Some(13);
        enemy.cooldowns = [0.7; 3];
        enemy.last_damage_tick = Some(10);
        session
            .encounter
            .brains
            .insert(417, brain::Brain::new(417, enemy.feet));
        session.actors.push(enemy);
        session.register_forest_roster();
        session.tick = 100;
        session.encounter.runtime.push(PartyRuntime {
            snapshot: PartySnapshot {
                id: 13,
                phase: PartyPhase::Active,
                home: Vec3::X * 160.0,
                living: 1,
            },
            knowledge: Some(Knowledge {
                point: Vec3::ZERO,
                velocity: Vec3::X,
                tick: 10,
                direct: true,
                cue_kind: None,
                observed: None,
            }),
            last_sight: 10,
            last_cue_id: None,
            leash: 20.0,
            search: 5.0,
            battle_search: None,
        });
        let materials = ArenaMaterials {
            stone: hex_core::SubstanceId(1),
            reinforced_stone: None,
            bedrock: hex_core::SubstanceId(2),
            grass: hex_core::SubstanceId(3),
            dirt: hex_core::SubstanceId(4),
            fire: hex_core::ElementId(1),
        };
        session.release(
            417,
            Spell::Fireball,
            &ArenaTuning::default(),
            10.0,
            &world,
            geometry,
            materials,
            &mut CommandsOut::default(),
        );
        let position = session.projectiles[0].position;
        session.suspend_grand_parties();
        assert_eq!(session.actors.len(), 1);
        assert!(session.projectiles.is_empty());
        assert!(session
            .grand_actor_interests()
            .iter()
            .all(|p| p.distance(Vec3::ZERO) < 100.0));
        let identity = crate::GrandCheckpointIdentity {
            world_id: "grand-test".into(),
            content_revision: "content".into(),
        };
        let bytes = session.encode_grand_checkpoint(&identity).unwrap();
        session =
            ArenaSession::decode_grand_checkpoint(&bytes, &identity, &world, geometry, 1).unwrap();
        session.tick = 1000;
        session.actors[0].feet = Vec3::X * 160.0;
        session.wake_grand_parties();
        assert!(session.encounter.dormant.is_empty());
        assert_eq!(session.actors[1].cooldowns, [0.7; 3]);
        assert_eq!(session.actors[1].last_damage_tick, Some(910));
        assert_eq!(session.projectiles[0].age, 0.0);
        assert_eq!(session.projectiles[0].position, position);
        assert_eq!(session.encounter.runtime[0].knowledge.unwrap().tick, 910);
        assert!(session.encounter.brains.contains_key(&417));
    }
}

impl ArenaSession {
    pub(crate) fn validate_grand_records(&self) -> Result<(), String> {
        let grand = self.grand.as_ref().ok_or("Not a Grand checkpoint")?;
        if grand
            .admitted
            .iter()
            .any(|name| !SITES.iter().any(|site| site.0 == name))
        {
            return Err("Grand checkpoint contains an unknown encounter".into());
        }
        let mut expected = BTreeMap::new();
        let mut expected_parties = std::collections::BTreeSet::new();
        for (ordinal, &(name, leader, count)) in SITES
            .iter()
            .enumerate()
            .filter(|(_, site)| grand.admitted.contains(site.0))
        {
            let party = u16::try_from(ordinal).map_err(|_| "Invalid Grand party identity")?;
            expected_parties.insert(party);
            for slot in 0..count {
                let id = ActorId::try_from(ordinal * 32 + slot + 1)
                    .map_err(|_| "Invalid Grand actor identity")?;
                let (species, role) = roster_profile(leader, slot);
                expected.insert(id, (party, species, role));
            }
        }
        let mut actual = std::collections::BTreeSet::new();
        for actor in self
            .actors
            .iter()
            .chain(
                self.encounter
                    .dormant
                    .values()
                    .flat_map(|p| p.actors.iter()),
            )
            .filter(|a| a.id != 0)
        {
            let Some(&(party, species, role)) = expected.get(&actor.id) else {
                return Err("Grand checkpoint contains an unauthored actor".into());
            };
            if actor.party != Some(party)
                || actor.species != species
                || actor.expedition_role != role
                || !actual.insert(actor.id)
            {
                return Err("Grand checkpoint actor identity/profile is inconsistent".into());
            }
        }
        if !self
            .progression
            .as_ref()
            .is_some_and(|p| p.valid_grand_roster(&actual))
        {
            return Err("Grand checkpoint XP roster is inconsistent".into());
        }
        if actual != expected.keys().copied().collect() {
            return Err("Grand checkpoint is missing encounter members".into());
        }
        let active_ids: std::collections::BTreeSet<_> = self
            .actors
            .iter()
            .filter(|a| a.id != 0)
            .map(|a| a.id)
            .collect();
        if active_ids != self.encounter.brains.keys().copied().collect() {
            return Err("Grand checkpoint active AI ownership is inconsistent".into());
        }
        let mut parties = std::collections::BTreeSet::new();
        for runtime in &self.encounter.runtime {
            if !parties.insert(runtime.snapshot.id)
                || !self
                    .actors
                    .iter()
                    .any(|a| a.party == Some(runtime.snapshot.id))
            {
                return Err("Grand checkpoint active party ownership is inconsistent".into());
            }
        }
        for (&id, party) in &self.encounter.dormant {
            let ids: std::collections::BTreeSet<_> = party.actors.iter().map(|a| a.id).collect();
            if !parties.insert(id)
                || id != party.runtime.snapshot.id
                || party.slept_at > self.tick
                || ids.is_empty()
                || ids != party.brains.keys().copied().collect()
                || party.actors.iter().any(|a| a.party != Some(id))
                || party.projectiles.iter().any(|p| !ids.contains(&p.owner))
                || party.walls.iter().any(|p| !ids.contains(&p.owner))
                || party.barriers.iter().any(|p| !ids.contains(&p.owner))
                || party.auras.iter().any(|p| !ids.contains(&p.owner))
            {
                return Err("Grand checkpoint suspended party ownership is inconsistent".into());
            }
        }
        if parties != expected_parties
            || self
                .projectiles
                .iter()
                .any(|p| p.owner != 0 && !active_ids.contains(&p.owner))
            || self
                .pending_walls
                .iter()
                .any(|p| p.owner != 0 && !active_ids.contains(&p.owner))
        {
            return Err("Grand checkpoint party/attack ownership is inconsistent".into());
        }
        Ok(())
    }
}

fn roster_profile(leader: Species, slot: usize) -> (Species, Option<ExpeditionRole>) {
    let species = if leader == Species::Shaman && slot >= 2 {
        Species::Goblin
    } else {
        leader
    };
    let role = match species {
        Species::Goblin if leader == Species::Shaman => Some(ExpeditionRole::Troll),
        Species::Goblin => Some(ExpeditionRole::Goblin),
        Species::Shaman => Some(ExpeditionRole::Shaman),
        Species::Dragon => Some(ExpeditionRole::Dragon),
        Species::Golem => Some(ExpeditionRole::PlainGolem),
        Species::Wisp => Some(ExpeditionRole::PlainWisp),
        Species::Shadow => Some(ExpeditionRole::MountainShadow),
        _ => None,
    };
    (species, role)
}
