use super::*;
use crate::*;
use hex_core::arena::{ArenaMap, ArenaSelection, ArenaSolidSpan, ArenaStaticSpan};
use hex_core::{ElementId, HexCoord, SubstanceId};

struct Fixture {
    session: ArenaSession,
    world: ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    materials: ArenaMaterials,
    tuning: ArenaTuning,
}

fn fixture() -> Fixture {
    let geometry = ArenaVoxelGeometry {
        radius: 80,
        ..Default::default()
    };
    let material = SubstanceId(1);
    let columns = HexCoord::ORIGIN.within_radius(80);
    let anchors = [
        ("party_start", -65),
        ("hostile_start", -45),
        ("forest_outer_a", -45),
        ("forest_outer_b", -30),
        ("forest_middle", -15),
        ("forest_deep_a", 0),
        ("forest_deep_b", 15),
        ("dragon_lower", 30),
        ("dragon_middle", 45),
        ("dragon_upper", 60),
    ]
    .into_iter()
    .map(|(name, q)| (name.to_owned(), HexCoord::from_axial(q, 0).to_world(SKIN)))
    .collect::<BTreeMap<_, _>>();
    let world = ArenaTerrainView {
        revision: 1,
        selection: ArenaSelection {
            map: ArenaMap::ForestMassif,
            ..Default::default()
        },
        spawns: [
            *anchors.get("party_start").expect("start"),
            *anchors.get("hostile_start").expect("enemy"),
        ],
        anchors,
        voxels: columns
            .iter()
            .map(|coord| (TilePos::new(*coord, 0), material))
            .collect(),
        columns: columns
            .iter()
            .map(|coord| {
                (
                    *coord,
                    vec![ArenaSolidSpan {
                        bottom: TilePos::new(*coord, 0),
                        top_level: 0,
                        substance: material,
                    }],
                )
            })
            .collect(),
        ..Default::default()
    };
    let materials = ArenaMaterials {
        stone: material,
        grass: material,
        dirt: material,
        bedrock: SubstanceId(2),
        fire: ElementId(1),
    };
    let tuning = ArenaTuning::default();
    let mut session = ArenaSession {
        bot_enabled: false,
        ..Default::default()
    };
    session.reset(0, &world, geometry);
    session.advance(ActorIntent::default(), &world, geometry, materials, &tuning);
    assert_eq!(session.actors.len(), 26, "{}", session.notice);
    Fixture {
        session,
        world,
        geometry,
        materials,
        tuning,
    }
}

impl Fixture {
    fn kill(&mut self, id: ActorId, credit: bool) {
        let actor = self
            .session
            .actors
            .iter_mut()
            .find(|actor| actor.id == id)
            .expect("actor");
        let amount = actor.hp;
        actor.hp = 0.0;
        if credit {
            self.session.record_damage(0, id, amount);
        }
        self.session.reconcile_progression();
    }

    fn kill_species(&mut self, species: &[Species]) {
        let ids: Vec<_> = self
            .session
            .actors
            .iter()
            .filter(|actor| species.contains(&actor.species))
            .map(|actor| actor.id)
            .collect();
        for id in ids {
            self.kill(id, true);
        }
    }

    fn launch(&mut self, owner: ActorId, aim: Vec3) {
        self.session
            .actors
            .iter_mut()
            .find(|actor| actor.id == owner)
            .expect("owner")
            .aim = aim;
        self.session.release(
            owner,
            Spell::Fireball,
            &self.tuning,
            45.0,
            &self.world,
            self.geometry,
            self.materials,
            &mut CommandsOut::default(),
        );
    }

    fn resolve(&mut self) -> CommandsOut {
        let mut out = CommandsOut::default();
        for _ in 0..960 {
            self.session
                .advance_projectiles(&self.world, self.geometry, self.materials, &mut out);
            if self.session.projectiles.is_empty() {
                break;
            }
        }
        assert!(self.session.projectiles.is_empty());
        out
    }

    fn place_targets(&mut self) {
        for (id, feet) in [
            (0, Vec3::new(-5.0, SKIN, -30.0)),
            (1, Vec3::new(-1.0, SKIN, -30.0)),
            (2, Vec3::new(-1.0, SKIN, -29.0)),
        ] {
            let actor = self
                .session
                .actors
                .iter_mut()
                .find(|actor| actor.id == id)
                .expect("actor");
            actor.feet = feet;
            actor.previous_feet = feet;
        }
    }
}

#[test]
fn authored_roster_is_exact_and_incomplete_world_refuses_whole_run() {
    let mut f = fixture();
    assert_eq!(
        f.session
            .parties()
            .iter()
            .map(|p| p.living)
            .collect::<Vec<_>>(),
        [2, 3, 4, 6, 7, 1, 1, 1]
    );
    for (species, count) in [
        (Species::Goblin, 20),
        (Species::Shaman, 2),
        (Species::Dragon, 3),
    ] {
        assert_eq!(
            f.session
                .actors
                .iter()
                .filter(|a| a.species == species)
                .count(),
            count
        );
    }
    f.world.anchors.remove("forest_deep_b");
    f.session.reset(1, &f.world, f.geometry);
    f.session.advance(
        ActorIntent::default(),
        &f.world,
        f.geometry,
        f.materials,
        &f.tuning,
    );
    assert!(f.session.is_finished() && f.session.actors.is_empty());
    assert!(f.session.notice.contains("forest_deep_b"));
}

#[test]
fn forest_and_dragon_rewards_are_separate_once_only_and_clear_still_moves() {
    let mut f = fixture();
    f.kill_species(&[Species::Goblin]);
    assert!(!f.session.progress().expect("progress").forest_cleared);
    f.kill_species(&[Species::Shaman]);
    let p = f.session.progress().expect("progress");
    assert_eq!(p.forest_defeated, 22);
    assert!(p.forest_cleared && !p.explosions_unlocked);
    assert!((f.session.player_tuning(&f.tuning).fireball_damage - 40.0).abs() < SKIN);
    f.kill_species(&[Species::Dragon]);
    let p = f.session.progress().expect("progress");
    assert!(p.completed && p.explosions_unlocked);
    assert_eq!(
        (p.total_xp, p.level, p.xp, p.available_upgrades),
        (90, 5, 8, 4)
    );
    f.session.reconcile_progression();
    assert_eq!(f.session.progress(), Some(p));
    let feet = f.session.actors.first().expect("player").feet;
    for _ in 0..20 {
        f.session.advance(
            ActorIntent {
                movement: Vec2::Y,
                ..Default::default()
            },
            &f.world,
            f.geometry,
            f.materials,
            &f.tuning,
        );
    }
    assert!(!f.session.is_finished() && f.session.completed_run());
    assert!(
        f.session
            .actors
            .first()
            .expect("player")
            .feet
            .distance(feet)
            > 0.1
    );
    f.session.reset(1, &f.world, f.geometry);
    let fresh = f.session.progress().expect("progress");
    assert_eq!(
        (fresh.level, fresh.total_xp, fresh.available_upgrades),
        (1, 0, 0)
    );
    assert!(!fresh.completed && !fresh.explosions_unlocked && !fresh.forest_cleared);
}

#[test]
fn dragon_first_completion_waits_for_every_forest_enemy_and_keeps_casting() {
    let mut f = fixture();
    let dragons: Vec<_> = f
        .session
        .actors
        .iter()
        .filter(|actor| actor.species == Species::Dragon)
        .map(|actor| actor.id)
        .collect();
    for id in dragons.iter().take(2) {
        f.kill(*id, true);
    }
    assert_eq!(f.session.progress().expect("progress").dragons_defeated, 2);
    assert!(!f.session.progress().expect("progress").explosions_unlocked);
    f.kill(*dragons.last().expect("third Dragon"), true);
    let unlocked = f.session.progress().expect("progress");
    assert!(unlocked.explosions_unlocked && !unlocked.forest_cleared && !unlocked.completed);
    assert_eq!(unlocked.forest_defeated, 0);
    assert!((f.session.player_tuning(&f.tuning).fireball_damage - 15.0).abs() < SKIN);
    f.kill_species(&[Species::Goblin]);
    assert!(!f.session.completed_run());
    f.kill_species(&[Species::Shaman]);
    let completed = f.session.progress().expect("progress");
    assert!(completed.completed && completed.forest_cleared && completed.explosions_unlocked);
    assert_eq!(
        (completed.total_xp, completed.level, completed.xp),
        (90, 5, 8)
    );
    assert!((f.session.player_tuning(&f.tuning).fireball_damage - 40.0).abs() < SKIN);
    f.session.advance(
        ActorIntent {
            aim: Vec3::Y,
            selected: Some(Spell::Fireball),
            cast_pressed: true,
            cast_released: true,
            ..Default::default()
        },
        &f.world,
        f.geometry,
        f.materials,
        &f.tuning,
    );
    assert!(!f.session.is_finished() && f.session.outcome.is_none());
    assert_eq!(f.session.progress(), Some(completed));
    assert_eq!(
        f.session
            .projectiles
            .first()
            .expect("victory exploration cast")
            .fireball_mode(),
        FireballMode::Explosive
    );
}

#[test]
fn bought_damage_stacks_with_forest_reward_even_at_normal_upgrade_cap() {
    let mut f = fixture();
    let first_ten: Vec<_> = f
        .session
        .actors
        .iter()
        .filter(|actor| actor.species == Species::Goblin)
        .take(10)
        .map(|actor| actor.id)
        .collect();
    for id in first_ten {
        f.kill(id, true);
    }
    assert!(f.session.spend_upgrade(UpgradeStat::FireballDamage));
    assert!((f.session.player_tuning(&f.tuning).fireball_damage - 20.0).abs() < SKIN);
    assert!(!f.session.progress().expect("progress").forest_cleared);
    // This one-map roster cannot fund seventeen damage purchases. Extra credits
    // explicitly exercise the normal stat ceiling; purchases still use the API.
    f.session
        .progression
        .as_mut()
        .expect("state")
        .snapshot
        .available_upgrades = 20;
    for _ in 0..16 {
        assert!(f.session.spend_upgrade(UpgradeStat::FireballDamage));
    }
    assert!((f.session.player_tuning(&f.tuning).fireball_damage - 100.0).abs() < SKIN);
    let capped = f.session.progress().expect("progress");
    assert!(!f.session.spend_upgrade(UpgradeStat::FireballDamage));
    assert_eq!(f.session.progress(), Some(capped));
    f.kill_species(&[Species::Goblin, Species::Shaman]);
    let rewarded = f.session.progress().expect("progress");
    assert!(rewarded.forest_cleared && !rewarded.explosions_unlocked);
    assert!((rewarded.damage_bonus - 25.0).abs() < SKIN);
    assert!((f.session.player_tuning(&f.tuning).fireball_damage - 125.0).abs() < SKIN);
    assert!(!f.session.can_upgrade(UpgradeStat::FireballDamage));
    assert!(!f.session.spend_upgrade(UpgradeStat::FireballDamage));
    f.session.reconcile_progression();
    assert_eq!(f.session.progress(), Some(rewarded));
    assert!((f.session.player_tuning(&f.tuning).fireball_damage - 125.0).abs() < SKIN);
}

#[test]
fn xp_credits_recent_knockback_but_not_unrelated_or_expired_deaths() {
    let mut f = fixture();
    f.session.record_player_hit(0, 1);
    f.session.tick += 1200;
    f.kill(1, false);
    assert_eq!(f.session.progress().expect("progress").total_xp, 1);
    f.session.record_player_hit(0, 2);
    f.session.tick += 1201;
    f.kill(2, false);
    f.kill(3, false);
    f.kill(1, false);
    assert_eq!(f.session.progress().expect("progress").total_xp, 1);
    assert_eq!(f.session.progress().expect("progress").forest_defeated, 3);
    assert_eq!(
        (1..=5).map(level_threshold).collect::<Vec<_>>(),
        [10, 15, 23, 34, 51]
    );
}

#[test]
fn spend_is_beneficial_capped_locked_and_does_not_change_enemy_tuning() {
    let mut f = fixture();
    assert!(!f.session.spend_upgrade(UpgradeStat::FireballDamage));
    f.kill_species(&[Species::Goblin]);
    let before = f.session.progress().expect("progress");
    assert!(!f.session.spend_upgrade(UpgradeStat::FireballSize));
    assert_eq!(f.session.progress(), Some(before));
    assert!(f.session.spend_upgrade(UpgradeStat::FireballCooldown));
    assert!(!f.session.can_upgrade(UpgradeStat::FireballCooldown));
    let player = f.session.player_tuning(&f.tuning);
    assert!((player.fireball_cooldown - 0.25).abs() < SKIN);
    assert!((player.projectile_speed - 45.0).abs() < SKIN);
    assert!((player.projectile_gravity - 12.0).abs() < SKIN);
    assert!((f.tuning.projectile_speed - 32.0).abs() < SKIN);
    assert!((f.tuning.fireball_cooldown - 1.25).abs() < SKIN);
    f.kill_species(&[Species::Shaman, Species::Dragon]);
    assert!(f.session.spend_upgrade(UpgradeStat::FireballDamage));
    assert!((f.session.player_tuning(&f.tuning).fireball_damage - 45.0).abs() < SKIN);
    assert!(f.session.spend_upgrade(UpgradeStat::FireballSize));
    assert!(!f.session.can_upgrade(UpgradeStat::FireballSize));
    assert!((f.session.player_tuning(&f.tuning).fireball_radius() - 3.5).abs() < SKIN);
}

#[test]
fn contact_hits_one_body_and_freezes_before_dragon_unlock() {
    let mut f = fixture();
    f.place_targets();
    f.launch(0, Vec3::X);
    assert_eq!(
        f.session.projectiles.first().expect("shot").fireball_mode(),
        FireballMode::ContactOnly
    );
    f.kill_species(&[Species::Dragon]);
    assert!(f.session.progress().expect("progress").explosions_unlocked);
    assert!(f.session.spend_upgrade(UpgradeStat::FireballDamage));
    assert!(f.resolve().impacts.is_empty());
    assert!(f.session.effects.iter().any(|effect| {
        effect.kind == crate::VisualEffectKind::FireballContact && effect.radius <= 0.16
    }));
    assert!(
        f.session
            .effects
            .iter()
            .all(|effect| effect.kind != crate::VisualEffectKind::Fireball)
    );
    assert!((f.session.actors.get(1).expect("target").hp - 35.0).abs() < SKIN);
    assert!((f.session.actors.get(2).expect("nearby").hp - 50.0).abs() < SKIN);
    f.launch(0, Vec3::X);
    assert_eq!(
        f.session.projectiles.first().expect("shot").fireball_mode(),
        FireballMode::Explosive
    );
    f.resolve();
    let target = f.session.actors.get(1).expect("target");
    assert!(
        target.hp >= 15.0 && target.hp < 16.0,
        "one radial contribution: {}",
        target.hp
    );
    assert!(f.session.actors.get(2).expect("nearby").hp < 50.0);
}

#[test]
fn contact_terrain_damages_exactly_one_voxel_and_barrier_does_not_splash() {
    let mut f = fixture();
    f.place_targets();
    f.launch(0, Vec3::NEG_Y);
    let out = f.resolve();
    assert_eq!(out.impacts.len(), 1);
    assert_eq!(out.impacts.first().expect("impact").volume.len(), 1);
    assert!((f.session.actors.first().expect("player").hp - 100.0).abs() < SKIN);
    f.session.encounter.barriers.push(BarrierSnapshot {
        id: 1,
        owner: 1,
        center: Vec3::new(-3.0, 0.8, -30.0),
        normal: Vec3::X,
        width: 4.0,
        height: 2.0,
        hp: 60.0,
        max_hp: 60.0,
        remaining: 4.0,
        lifetime: 4.0,
    });
    f.session
        .collision
        .sync_barriers(&f.session.encounter.barriers);
    f.launch(0, Vec3::X);
    assert!(f.resolve().impacts.is_empty());
    assert!((f.session.barriers().first().expect("barrier").hp - 45.0).abs() < SKIN);
    assert!((f.session.actors.get(1).expect("target").hp - 50.0).abs() < SKIN);
}

#[test]
fn protected_static_tree_contact_cannot_damage_nearby_terrain_or_actors() {
    let mut f = fixture();
    let coord = HexCoord::from_world(Vec3::new(-3.0, 0.0, -30.0));
    let root = coord.to_world(SKIN);
    f.world.static_spans.push(ArenaStaticSpan {
        bottom: TilePos::new(coord, 1),
        top_level: 6,
        blocks_movement: true,
        blocks_projectiles: true,
        blocks_sight: true,
    });
    f.world.edit_protected.insert(coord, vec![(0, 6)]);
    f.world.revision += 1;
    f.world.full_rebuild = true;
    f.session.collision.refresh(&f.world, f.geometry);
    for (id, offset) in [
        (0, Vec3::NEG_X * 4.0),
        (1, Vec3::Z * 1.6),
        (2, Vec3::X * 2.2),
    ] {
        let actor = f
            .session
            .actors
            .iter_mut()
            .find(|actor| actor.id == id)
            .expect("actor");
        actor.feet = root + offset;
        actor.previous_feet = actor.feet;
    }
    let health: Vec<_> = f
        .session
        .actors
        .iter()
        .map(|actor| (actor.id, actor.hp))
        .collect();
    f.launch(0, Vec3::X);
    let commands = f.resolve();
    assert!(commands.impacts.is_empty() && commands.edits.is_empty());
    assert_eq!(
        f.session
            .actors
            .iter()
            .map(|actor| (actor.id, actor.hp))
            .collect::<Vec<_>>(),
        health
    );
    let hit = f
        .session
        .effects
        .last()
        .expect("actual tree collision, not an expired miss");
    assert!(hit.center.with_y(root.y).distance(root) < 1.1);
    assert!((hit.radius - 0.16).abs() < SKIN);
    assert!(
        f.session
            .actors
            .get(1)
            .expect("nearby actor")
            .center()
            .distance(hit.center)
            < 2.5
    );
    assert!(
        f.world.voxels.contains_key(&TilePos::new(coord, 0)),
        "protected ground is still present"
    );
}

#[test]
fn shaman_projectile_retains_baseline_payload_and_explosion() {
    let mut f = fixture();
    f.place_targets();
    let owner = f
        .session
        .actors
        .iter()
        .find(|a| a.species == Species::Shaman)
        .expect("shaman")
        .id;
    f.session
        .actors
        .iter_mut()
        .find(|a| a.id == owner)
        .expect("shaman")
        .feet = Vec3::new(-1.0, SKIN, -30.0);
    f.session
        .actors
        .iter_mut()
        .find(|a| a.id == owner)
        .expect("shaman")
        .previous_feet = Vec3::new(-1.0, SKIN, -30.0);
    f.launch(owner, Vec3::NEG_X);
    assert_eq!(
        f.session.projectiles.first().expect("shot").fireball_mode(),
        FireballMode::Explosive
    );
    f.resolve();
    let hp = f.session.actors.first().expect("player").hp;
    assert!(hp < 70.0 && hp > 64.0, "baseline Shaman hit: {hp}");
}

#[test]
fn indexed_liquid_queries_match_full_scan_for_bodies_and_stacked_water() {
    let mut f = fixture();
    f.world.liquids = HexCoord::ORIGIN
        .within_radius(60)
        .into_iter()
        .filter(|coord| coord.x().rem_euclid(3) == 0)
        .flat_map(|coord| {
            [1, 8].map(|level| ArenaSolidSpan {
                bottom: TilePos::new(coord, level),
                top_level: level + 1,
                substance: SubstanceId(3),
            })
        })
        .collect();
    f.world.liquids.sort_by_key(|run| run.bottom);
    for species in [
        Species::Human,
        Species::Goblin,
        Species::Shaman,
        Species::Dragon,
    ] {
        let mut actor = Actor::spawn(0, Vec3::ZERO, Vec3::NEG_Z);
        actor.configure_species(species, &f.tuning.encounters);
        for x in [-110.0, -12.1, -4.2, -1.0, 0.0, 2.6, 12.2, 110.0] {
            for y in [0.0, 0.8, 2.0, 3.6, 5.0] {
                actor.feet = Vec3::new(x, y, 2.2);
                f.world.selection.map = ArenaMap::ForestMassif;
                let indexed = crate::encounters::dry(&actor, &f.world, f.geometry);
                f.world.selection.map = ArenaMap::Fort;
                assert_eq!(
                    indexed,
                    crate::encounters::dry(&actor, &f.world, f.geometry),
                    "{species:?} at {:?}",
                    actor.feet
                );
            }
        }
    }
}
