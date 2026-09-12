use super::*;
use hex_core::arena::{
    ArenaDeploymentRegion, ArenaEncounterSite, ArenaFountainVolume, ArenaSelection,
};
use hex_core::{ElementId, SubstanceId};

fn fixture() -> (
    ArenaSession,
    ArenaTerrainView,
    ArenaVoxelGeometry,
    ArenaMaterials,
    ArenaTuning,
) {
    let geometry = ArenaVoxelGeometry {
        radius: 187,
        ..Default::default()
    };
    let player = HexCoord::from_axial(0, 80);
    let mut view = ArenaTerrainView {
        selection: ArenaSelection {
            map: ArenaMap::ForestMassif,
            encounter: ArenaEncounter::Goblins,
        },
        spawns: [player.to_world(SKIN), Vec3::ZERO],
        ..Default::default()
    };
    for coord in player.within_radius(2) {
        view.voxels.insert(TilePos::new(coord, 0), SubstanceId(1));
    }
    let mut sites = ArenaExpeditionSites::default();
    for (index, (name, _)) in roster().into_iter().enumerate() {
        let index = i32::try_from(index).expect("bounded fixture roster");
        let center = HexCoord::from_axial(-72 + (index % 5) * 36, -90 + (index / 5) * 36);
        let surfaces = center
            .within_radius(4)
            .into_iter()
            .map(|coord| TilePos::new(coord, 0))
            .collect();
        let deployment = ArenaDeploymentRegion {
            preferred: TilePos::new(center, 0),
            surfaces,
        };
        for surface in &deployment.surfaces {
            view.voxels.insert(*surface, SubstanceId(1));
        }
        sites.encounters.insert(
            name,
            ArenaEncounterSite {
                deployment,
                rally_entry: None,
            },
        );
    }
    for (prefix, count) in [("forest", 4), ("mountain", 2)] {
        for i in 1..=count {
            sites.fountains.insert(
                format!("{prefix}_fountain_{i:02}"),
                ArenaFountainVolume {
                    cells: [TilePos::new(
                        HexCoord::from_axial(if prefix == "forest" { i } else { i + 10 }, 70),
                        1,
                    )]
                    .into(),
                },
            );
        }
    }
    view.expedition = Some(sites);
    let materials = ArenaMaterials {
        stone: SubstanceId(1),
        grass: SubstanceId(2),
        dirt: SubstanceId(3),
        bedrock: SubstanceId(4),
        fire: ElementId(1),
    };
    let tuning = ArenaTuning::default();
    let mut session = ArenaSession {
        bot_enabled: false,
        ..Default::default()
    };
    session.reset(0, &view, geometry);
    (session, view, geometry, materials, tuning)
}

#[test]
fn expedition_admits_exact_named_roster_and_actual_supported_profiles() {
    let (mut session, view, geometry, materials, tuning) = fixture();
    session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
    assert_eq!(session.actors.len(), 115, "{}", session.notice);
    assert_eq!(session.parties().len(), 19);
    for (role, count) in [
        (ExpeditionRole::BabyGoblin, 15),
        (ExpeditionRole::Goblin, 92),
        (ExpeditionRole::Shaman, 2),
        (ExpeditionRole::Troll, 1),
        (ExpeditionRole::Dragon, 3),
        (ExpeditionRole::MountainShadow, 1),
    ] {
        assert_eq!(
            session
                .actors
                .iter()
                .filter(|a| a.expedition_role() == Some(role))
                .count(),
            count
        );
    }
    for actor in &session.actors {
        assert!(
            session.actor_pose_valid(actor.id, &view, geometry),
            "{:?}",
            actor.expedition_role()
        );
        if actor.id == 0 {
            continue;
        }
        let role = actor.expedition_role().expect("authored identity");
        assert_eq!(actor.species, role.species());
        let profile = actor.expedition_tuning(&tuning);
        match role {
            ExpeditionRole::BabyGoblin => {
                assert_eq!(actor.hp.to_bits(), 30.0_f32.to_bits());
                assert_eq!(profile.encounters.goblin_run.to_bits(), 5.0_f32.to_bits());
                assert_eq!(profile.encounters.swipe_damage.to_bits(), 8.0_f32.to_bits());
            }
            ExpeditionRole::Troll => {
                assert_eq!(actor.hp.to_bits(), 600.0_f32.to_bits());
                assert!((actor.body_dimensions().y - 2.4).abs() < 0.001);
                assert!((actor.eye().y - actor.feet.y - 1.86).abs() < 0.001);
            }
            ExpeditionRole::MountainShadow => {
                assert_eq!(actor.hp.to_bits(), 125.0_f32.to_bits());
                assert_eq!(profile.fireball_damage.to_bits(), 30.0_f32.to_bits());
            }
            _ => {}
        }
    }
}

#[test]
fn expedition_missing_extra_or_unfit_site_refuses_the_entire_roster() {
    for failure in 0..4 {
        let (mut session, mut view, geometry, materials, tuning) = fixture();
        let sites = view.expedition.as_mut().expect("sites");
        match failure {
            0 => {
                sites.encounters.remove("forest_camp_01");
            }
            1 => {
                let site = sites
                    .encounters
                    .get("forest_camp_01")
                    .expect("site")
                    .clone();
                sites.encounters.insert("unknown_camp".into(), site);
            }
            2 => {
                let site = sites.encounters.get_mut("forest_camp_14").expect("site");
                site.deployment.surfaces = [site.deployment.preferred].into();
            }
            _ => {
                sites.fountains.remove("forest_fountain_01");
            }
        }
        session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
        assert!(session.actors.is_empty());
        assert!(session.parties().is_empty());
        assert!(matches!(
            session.battle_result,
            Some(BattleResult::InvalidSetup(_))
        ));
    }
}

#[test]
fn expedition_profiles_drive_real_movement() {
    let (mut session, view, geometry, materials, tuning) = fixture();
    session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
    let mut baby = session
        .actors
        .iter()
        .find(|a| a.expedition_role() == Some(ExpeditionRole::BabyGoblin))
        .expect("baby")
        .clone();
    let mut adult = baby.clone();
    adult.configure_expedition(ExpeditionRole::Goblin, &tuning.encounters);
    let empty = CollisionWorld::default();
    let baby_tuning = baby.expedition_tuning(&tuning);
    let start = baby.feet;
    for _ in 0..120 {
        motion::tick(
            &mut baby,
            Vec3::X,
            true,
            false,
            false,
            &empty,
            &baby_tuning.encounters,
        );
        motion::tick(
            &mut adult,
            Vec3::X,
            true,
            false,
            false,
            &empty,
            &tuning.encounters,
        );
    }
    assert!((baby.feet.x - start.x - 5.0).abs() < 0.02);
    assert!((adult.feet.x - start.x - 6.0).abs() < 0.02);
}

#[test]
fn expedition_player_moves_five_percent_faster_without_stacking_on_reset() {
    let (mut session, view, geometry, materials, tuning) = fixture();
    for generation in [0, 1] {
        session.reset(generation, &view, geometry);
        session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
        let player = session.actors.first().expect("player").clone();
        assert!(session
            .actors
            .iter()
            .skip(1)
            .all(|actor| !actor.expedition_player));
        for run in [false, true] {
            for (mut actor, expected) in [
                (player.clone(), 4.725),
                (Actor::spawn(0, player.feet, Vec3::NEG_Z), 4.5),
                (Actor::spawn(1, player.feet, Vec3::NEG_Z), 4.5),
            ] {
                let start = actor.feet.x;
                for _ in 0..120 {
                    motion::tick(
                        &mut actor,
                        Vec3::X,
                        run,
                        false,
                        false,
                        &CollisionWorld::default(),
                        &tuning.encounters,
                    );
                }
                assert!((actor.feet.x - start - expected).abs() < 0.02);
            }
        }
    }
}

#[test]
fn forest_player_never_regenerates_without_a_fountain() {
    let (mut session, view, geometry, materials, tuning) = fixture();
    session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
    let player = session.actors.first_mut().expect("player");
    player.hp = 10.0;
    session.tick = 10_000;
    session.advance_support(&tuning, ArenaMap::ForestMassif);
    assert_eq!(
        session.actors.first().expect("player").hp.to_bits(),
        10.0_f32.to_bits()
    );
}

#[test]
fn baby_and_troll_swipes_apply_their_own_damage_and_cooldown() {
    for (role, damage, cooldown) in [
        (ExpeditionRole::BabyGoblin, 8.0_f32, 1.4_f32),
        (ExpeditionRole::Troll, 24.0_f32, 1.8_f32),
    ] {
        let (mut session, view, geometry, materials, tuning) = fixture();
        session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
        let source = session
            .actors
            .iter()
            .find(|a| a.expedition_role() == Some(role))
            .expect("source")
            .clone();
        session.actors.retain(|a| a.id == 0 || a.id == source.id);
        let player = session.actors.first_mut().expect("player");
        player.feet = source.feet + Vec3::X;
        player.previous_feet = player.feet;
        let aim = (player.center() - source.eye()).normalize();
        let mut brains = std::mem::take(&mut session.encounter.brains);
        brains.retain(|id, _| *id == source.id);
        session.begin_ability(
            &mut brains,
            source.id,
            brain::Request {
                kind: CreatureAbility::Swipe,
                aim,
            },
            &tuning,
        );
        let brain = brains.get(&source.id).expect("brain");
        assert_eq!(
            brain
                .cooldowns
                .get(CreatureAbility::Swipe.index())
                .expect("cooldown")
                .to_bits(),
            cooldown.to_bits()
        );
        for _ in 0..90 {
            session.advance_abilities(
                &mut brains,
                &view,
                geometry,
                materials,
                &tuning,
                &mut CommandsOut::default(),
            );
        }
        assert_eq!(
            session.actors.first().expect("player").hp.to_bits(),
            (100.0 - damage).to_bits()
        );
    }
}

fn defeat(session: &mut ArenaSession, id: ActorId, credit: bool) {
    if credit {
        session.record_player_hit(0, id);
    }
    session
        .actors
        .iter_mut()
        .find(|actor| actor.id == id)
        .expect("registered enemy")
        .hp = 0.0;
    session.reconcile_progression();
}

#[test]
fn expedition_all_credited_kills_reach_level_eight_without_automatic_rewards() {
    let (mut session, view, geometry, materials, tuning) = fixture();
    session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
    let ids: Vec<_> = session
        .actors
        .iter()
        .skip(1)
        .map(|actor| actor.id)
        .collect();
    for id in ids.iter().take(ids.len() - 1) {
        defeat(&mut session, *id, true);
    }
    assert!(!session.completed_run());
    let last = *ids.last().expect("final enemy");
    defeat(&mut session, last, true);
    let progress = session.progress().expect("progress");
    assert_eq!(
        (
            progress.total_xp,
            progress.level,
            progress.xp,
            progress.available_upgrades
        ),
        (327, 8, 4, 7)
    );
    assert_eq!(progress.forest_defeated, 109);
    assert_eq!(progress.dragons_defeated, 3);
    assert!(progress.forest_cleared && progress.completed);
    assert!(!progress.explosions_unlocked);
    assert_eq!(progress.damage_bonus.to_bits(), 0.0_f32.to_bits());
    assert!(!session.can_upgrade(UpgradeStat::FireballSize));
    defeat(&mut session, last, true);
    assert_eq!(session.progress(), Some(progress));
    session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
    assert!(
        !session.is_finished(),
        "victory must leave exploration active"
    );
}

#[test]
fn expedition_uncredited_and_expired_deaths_count_once_without_xp_or_troll_minion_credit() {
    let (mut session, view, geometry, materials, tuning) = fixture();
    session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
    let troll = session
        .actors
        .iter()
        .find(|a| a.expedition_role() == Some(ExpeditionRole::Troll))
        .expect("troll")
        .id;
    let shadow = session
        .actors
        .iter()
        .find(|a| a.expedition_role() == Some(ExpeditionRole::MountainShadow))
        .expect("shadow")
        .id;
    session.record_player_hit(0, troll);
    session.tick += 1200;
    defeat(&mut session, troll, false);
    assert_eq!(session.progress().expect("progress").total_xp, 50);
    assert_eq!(session.progress().expect("progress").forest_defeated, 0);
    session.record_player_hit(0, shadow);
    session.tick += 1201;
    defeat(&mut session, shadow, false);
    assert_eq!(session.progress().expect("progress").total_xp, 50);
    let ids: Vec<_> = session
        .actors
        .iter()
        .skip(1)
        .map(|actor| actor.id)
        .collect();
    for id in ids {
        defeat(&mut session, id, false);
    }
    let progress = session.progress().expect("progress");
    assert_eq!(progress.total_xp, 50);
    assert_eq!(progress.forest_defeated, 109);
    assert!(progress.completed);
    session.reset(1, &view, geometry);
    session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
    let reset = session.progress().expect("reset progress");
    assert_eq!(
        (reset.total_xp, reset.level, reset.xp, reset.forest_defeated),
        (0, 1, 0, 0)
    );
    assert!(!reset.completed && !reset.explosions_unlocked);
}

fn pool(view: &mut ArenaTerrainView, geometry: ArenaVoxelGeometry) -> (TilePos, Vec3) {
    let cell = *view
        .expedition
        .as_ref()
        .expect("sites")
        .fountains
        .get("forest_fountain_01")
        .expect("pool")
        .cells
        .first()
        .expect("water cell");
    let floor = TilePos::new(cell.coord, cell.level - 1);
    view.voxels.insert(floor, SubstanceId(1));
    view.liquids.push(hex_core::arena::ArenaSolidSpan {
        bottom: cell,
        top_level: cell.level,
        substance: SubstanceId(5),
    });
    (cell, cell.coord.to_world(geometry.top(floor) + SKIN))
}

#[test]
fn expedition_fountain_requires_actual_water_body_overlap_and_spends_only_when_wounded() {
    let (mut session, mut view, geometry, materials, tuning) = fixture();
    session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
    let (_, feet) = pool(&mut view, geometry);
    let consumed = |session: &ArenaSession| {
        session
            .expedition_progress()
            .expect("snapshot")
            .fountains
            .iter()
            .filter(|pool| pool.consumed)
            .count()
    };
    session.actors.first_mut().expect("player").feet = feet;
    session.advance_fountains(&view, geometry);
    assert_eq!(consumed(&session), 0, "full HP preserves the pool");

    let player = session.actors.first_mut().expect("player");
    player.hp = 90.0;
    player.feet = feet + Vec3::X * 1.2;
    session.advance_fountains(&view, geometry);
    assert_eq!(
        consumed(&session),
        0,
        "standing outside the actual hex volume cannot heal"
    );
    session.actors.first_mut().expect("player").feet = feet + Vec3::Y;
    session.advance_fountains(&view, geometry);
    assert_eq!(consumed(&session), 0, "above the water is not inside it");
    session.actors.first_mut().expect("player").feet = feet;
    let liquids = std::mem::take(&mut view.liquids);
    session.advance_fountains(&view, geometry);
    assert_eq!(
        consumed(&session),
        0,
        "an authored cell without actual water cannot heal"
    );
    view.liquids = liquids;
    session.advance_fountains(&view, geometry);
    assert_eq!(
        session.actors.first().expect("player").hp.to_bits(),
        100.0_f32.to_bits()
    );
    assert_eq!(consumed(&session), 1);
    session.actors.first_mut().expect("player").hp = 1.0;
    session.advance_fountains(&view, geometry);
    assert_eq!(
        session.actors.first().expect("player").hp.to_bits(),
        1.0_f32.to_bits()
    );
    assert_eq!(view.liquids.len(), 1, "consumption preserves the water");
}

#[test]
fn expedition_fountain_heals_forty_never_revives_and_reset_restores_it() {
    let (mut session, mut view, geometry, materials, tuning) = fixture();
    session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
    let (_, feet) = pool(&mut view, geometry);
    let player = session.actors.first_mut().expect("player");
    player.feet = feet;
    player.hp = 0.0;
    session.advance_fountains(&view, geometry);
    assert!(session
        .expedition_progress()
        .expect("snapshot")
        .fountains
        .iter()
        .all(|f| !f.consumed));
    session.actors.first_mut().expect("player").hp = 10.0;
    session.advance_fountains(&view, geometry);
    assert_eq!(
        session.actors.first().expect("player").hp.to_bits(),
        50.0_f32.to_bits()
    );
    session.reset(1, &view, geometry);
    session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
    assert!(session
        .expedition_progress()
        .expect("snapshot")
        .fountains
        .iter()
        .all(|f| !f.consumed));
    let player = session.actors.first_mut().expect("player");
    player.feet = feet;
    player.hp = 10.0;
    session.advance_fountains(&view, geometry);
    assert_eq!(
        session.actors.first().expect("player").hp.to_bits(),
        50.0_f32.to_bits()
    );
}

#[test]
fn expedition_snapshot_uses_registered_roles_and_does_not_invent_reward_orbs() {
    let (mut session, view, geometry, materials, tuning) = fixture();
    session.advance(ActorIntent::default(), &view, geometry, materials, &tuning);
    let before = session.expedition_progress().expect("snapshot");
    assert_eq!(
        (
            before.enemies_total,
            before.enemies_defeated,
            before.forest_total
        ),
        (114, 0, 109)
    );
    assert_eq!(before.fountains.len(), 6);
    assert!(before
        .milestones
        .iter()
        .all(|m| !m.defeated && !m.collected && m.available_position.is_none()));
    let troll = session
        .actors
        .iter()
        .find(|a| a.expedition_role() == Some(ExpeditionRole::Troll))
        .expect("troll")
        .id;
    // The admitted ledger, not a later live species mutation, defines the reward.
    session
        .actors
        .iter_mut()
        .find(|a| a.id == troll)
        .expect("troll")
        .species = Species::Shadow;
    defeat(&mut session, troll, false);
    let after = session.expedition_progress().expect("snapshot");
    assert_eq!((after.enemies_defeated, after.forest_defeated), (1, 0));
    assert!(
        after
            .milestones
            .iter()
            .find(|m| m.reward == ExpeditionReward::TrollDamage)
            .expect("milestone")
            .defeated
    );
    assert!(after
        .milestones
        .iter()
        .all(|m| !m.collected && m.available_position.is_none()));
}

#[path = "milestone_tests.rs"]
mod milestone_tests;

#[path = "rally_tests.rs"]
mod rally_tests;

#[path = "confinement_tests.rs"]
mod confinement_tests;
