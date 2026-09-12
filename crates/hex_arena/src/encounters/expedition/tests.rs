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
                format!("{prefix}_spring_{i:02}"),
                ArenaFountainVolume {
                    cells: [TilePos::new(HexCoord::from_axial(i, 70), 1)].into(),
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
                sites.fountains.remove("forest_spring_01");
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
