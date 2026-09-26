//! Destruction and loading at the real Grand death-recovery boundary.
use super::*;
use bevy_math::Vec2;
use hex_core::arena::{
    ArenaExpeditionSites, ArenaMap, ArenaMaterials, ArenaResidency, ArenaSelection, ArenaSolidSpan,
};
use hex_core::ocean::{
    OceanEnvironmentSampler, OceanEnvironmentView, OceanSurfaceSample, OceanWaterColumn,
    OceanWindProfile,
};
use hex_core::{ElementId, HexCoord, SubstanceId, TilePos};
use std::sync::Arc;

#[derive(Debug)]
struct DryEnvironment;
impl OceanEnvironmentSampler for DryEnvironment {
    fn surface_at(&self, _: Vec2, _: f32, _: OceanWaterColumn) -> Option<OceanSurfaceSample> {
        None
    }
}

fn tick(session: &mut ArenaSession, world: &ArenaTerrainView, geometry: ArenaVoxelGeometry) {
    session.advance(
        ActorIntent::default(),
        world,
        geometry,
        ArenaMaterials {
            stone: SubstanceId(1),
            reinforced_stone: Some(SubstanceId(6)),
            bedrock: SubstanceId(2),
            grass: SubstanceId(3),
            dirt: SubstanceId(4),
            fire: ElementId(1),
        },
        &crate::ArenaTuning::default(),
    );
}

fn fixture() -> (ArenaSession, ArenaTerrainView, ArenaVoxelGeometry, Vec3) {
    let geometry = ArenaVoxelGeometry {
        level_height: 1.0,
        radius: 64,
        ..Default::default()
    };
    let chunks: BTreeSet<_> = (-5..=5)
        .flat_map(|q| (-5..=5).map(move |r| (q, r)))
        .collect();
    let shrine = HexCoord::from_axial(32, 0).to_world(20.0 + SKIN);
    let world = ArenaTerrainView {
        revision: 1,
        selection: ArenaSelection {
            map: ArenaMap::GrandV4,
            ..Default::default()
        },
        spawns: [Vec3::Y * (20.0 + SKIN), Vec3::ZERO],
        columns: HexCoord::ORIGIN
            .within_radius(64)
            .into_iter()
            .map(|coord| {
                (
                    coord,
                    vec![ArenaSolidSpan {
                        bottom: TilePos::new(coord, 0),
                        top_level: 20,
                        substance: SubstanceId(1),
                    }],
                )
            })
            .collect(),
        residency: Some(ArenaResidency {
            catalogue: chunks.clone(),
            ready: chunks,
        }),
        expedition: Some(ArenaExpeditionSites::default()),
        anchors: [(ShrineId::Earth.anchor().into(), shrine)].into(),
        ..Default::default()
    };
    let mut session = ArenaSession {
        bot_enabled: false,
        ..Default::default()
    };
    session.reset(4, &world, geometry);
    session.ocean_environment = Some(OceanEnvironmentView {
        package_fingerprint: 7,
        sampler: Arc::new(DryEnvironment),
        wind: OceanWindProfile::default(),
    });
    tick(&mut session, &world, geometry);
    let actor = session.actors.first_mut().expect("Grand player");
    actor.feet = shrine;
    actor.previous_feet = shrine;
    session.advance_grand(
        ActorIntent {
            interact: true,
            ..Default::default()
        },
        &world,
        geometry,
    );
    assert_eq!(
        session
            .grand_progress()
            .expect("Grand progress")
            .respawn_anchor,
        Some(ShrineId::Earth)
    );
    (session, world, geometry, shrine)
}

fn destroy_support(world: &mut ArenaTerrainView, center: Vec3, radius: u32, lower_to: Option<i32>) {
    for coord in HexCoord::from_world(center).within_radius(radius) {
        if let Some(top_level) = lower_to {
            world.columns.insert(
                coord,
                vec![ArenaSolidSpan {
                    bottom: TilePos::new(coord, 0),
                    top_level,
                    substance: SubstanceId(1),
                }],
            );
        } else {
            world.columns.remove(&coord);
        }
    }
    world.revision += 1;
    world.full_rebuild = true;
}

fn die(session: &mut ArenaSession) {
    let player = session.actors.first_mut().expect("Grand player");
    player.hp = 0.0;
    player.feet = Vec3::new(90.0, -30.0, 0.0);
}

#[test]
fn grand_respawn_uses_safe_deep_crater_floor_and_retains_blessings() {
    let (mut session, mut world, geometry, shrine) = fixture();
    destroy_support(&mut world, shrine, 12, Some(8));
    die(&mut session);
    tick(&mut session, &world, geometry);
    let player = session.actors.first().unwrap();
    assert!(
        player.hp > 0.0,
        "removed shrine support cannot strand the dead player"
    );
    assert!((player.feet - shrine.with_y(8.0 + SKIN)).length() < 0.01);
    assert!(session.earth_construction(0));
    assert_eq!(session.grand_progress().unwrap().deaths, 1);
    assert_eq!(
        world
            .columns
            .get(&HexCoord::from_world(shrine))
            .expect("the crater floor remains published")
            .first()
            .unwrap()
            .top_level,
        8
    );
}

#[test]
fn grand_respawn_waits_for_unloaded_shrine_instead_of_abandoning_it() {
    let (mut session, mut world, geometry, shrine) = fixture();
    let saved = world.residency.as_ref().unwrap().ready.clone();
    world
        .residency
        .as_mut()
        .unwrap()
        .ready
        .retain(|(q, _)| *q < 1);
    world.revision += 1;
    world.full_rebuild = true;
    die(&mut session);
    tick(&mut session, &world, geometry);
    assert_eq!(
        session.actors.first().unwrap().hp.to_bits(),
        0.0_f32.to_bits()
    );
    assert!(session.grand_actor_interests().contains(&shrine));
    world.residency.as_mut().unwrap().ready = saved;
    world.revision += 1;
    tick(&mut session, &world, geometry);
    assert!(session.actors.first().unwrap().hp > 0.0);
    assert!((session.actors.first().unwrap().feet - shrine).length() < 0.01);
}

#[test]
fn grand_recovery_wait_freezes_combat_clocks_until_one_safe_return_tick() {
    for fallback in [false, true] {
        let (mut session, mut world, geometry, shrine) = fixture();
        let ready = world.residency.as_ref().unwrap().ready.clone();
        let start = *world.spawns.first().unwrap();
        let enemy_feet = if fallback {
            shrine + Vec3::X * 25.0
        } else {
            start + Vec3::X * 5.0
        } + Vec3::Y * 2.0;
        let mut enemy = Actor::spawn(7, enemy_feet, Vec3::Y);
        enemy.cooldowns = [2.0; 3];
        session.actors.push(enemy);
        session.release(
            7,
            crate::Spell::Fireball,
            &crate::ArenaTuning::default(),
            10.0,
            &world,
            geometry,
            ArenaMaterials {
                stone: SubstanceId(1),
                reinforced_stone: Some(SubstanceId(6)),
                bedrock: SubstanceId(2),
                grass: SubstanceId(3),
                dirt: SubstanceId(4),
                fire: ElementId(1),
            },
            &mut crate::CommandsOut::default(),
        );
        session.record_high_jump(7, enemy_feet);
        session.grand.as_mut().unwrap().teleport_cooldown = 3.0;
        if fallback {
            destroy_support(&mut world, shrine, 12, None);
        }
        world.residency.as_mut().unwrap().ready.retain(
            |(q, _)| {
                if fallback {
                    *q >= 1
                } else {
                    *q < 1
                }
            },
        );
        world.revision += 1;
        world.full_rebuild = true;
        die(&mut session);
        let clock = session.ocean_time();
        let saved_tick = session.tick;
        let combat = ron::ser::to_string(&(
            session.actors.iter().find(|actor| actor.id == 7).unwrap(),
            &session.projectiles,
            &session.effects,
            &session.combat_cues,
        ))
        .unwrap();
        for _ in 0..4 {
            tick(&mut session, &world, geometry);
            assert_eq!(session.tick, saved_tick);
            assert_eq!(session.ocean_time(), clock);
            assert_eq!(
                session
                    .grand_progress()
                    .unwrap()
                    .teleport_cooldown
                    .to_bits(),
                3.0_f32.to_bits()
            );
            assert_eq!(
                ron::ser::to_string(&(
                    session.actors.iter().find(|actor| actor.id == 7).unwrap(),
                    &session.projectiles,
                    &session.effects,
                    &session.combat_cues,
                ))
                .unwrap(),
                combat,
                "recovery loading must not move enemies or age combat and effects"
            );
            assert_eq!(session.grand_progress().unwrap().deaths, 0);
            let grand = session.grand.as_ref().unwrap();
            assert_eq!(
                grand.respawn_stage,
                if fallback {
                    RespawnStage::Start
                } else {
                    RespawnStage::Shrine
                }
            );
            assert_eq!(
                grand.respawn_interest,
                Some(if fallback { start } else { shrine })
            );
        }
        // Destination metadata may progress to the starting beach while the
        // simulation is frozen. Admitting it resumes exactly one ordinary tick.
        world.residency.as_mut().unwrap().ready = ready;
        world.revision += 1;
        tick(&mut session, &world, geometry);
        assert_eq!(session.tick, saved_tick + 1);
        assert!(session.ocean_time().seconds > clock.seconds);
        assert_eq!(session.grand_progress().unwrap().deaths, 1);
        assert_eq!(
            session.grand_progress().unwrap().respawn_anchor,
            Some(ShrineId::Earth)
        );
        assert!(session.earth_construction(0));
        assert!(session.actors.first().unwrap().hp > 0.0);
        assert!(
            session
                .actors
                .first()
                .unwrap()
                .feet
                .distance(if fallback { start } else { shrine })
                < 0.01
        );
        let enemy = session.actors.iter().find(|actor| actor.id == 7).unwrap();
        assert!(enemy.cooldowns.iter().all(|cooldown| *cooldown < 2.0));
        assert!(session.projectiles.first().unwrap().age > 0.0);
        assert!(session.effects.first().unwrap().age > 0.0);
        assert!(session.grand_progress().unwrap().teleport_cooldown < 3.0);
    }
}

#[test]
fn grand_respawn_falls_back_to_start_when_loaded_shrine_area_is_removed() {
    let (mut session, mut world, geometry, shrine) = fixture();
    destroy_support(&mut world, shrine, 12, None);
    die(&mut session);
    for _ in 0..3 {
        tick(&mut session, &world, geometry);
    }
    assert!(
        session.actors.first().unwrap().hp > 0.0,
        "fully destroyed shrine area needs a bounded fallback"
    );
    assert!(
        (session.actors.first().unwrap().feet - *world.spawns.first().unwrap()).length() < 0.01
    );
    assert_eq!(
        session.grand_progress().unwrap().respawn_anchor,
        Some(ShrineId::Earth)
    );
    assert!(session.earth_construction(0));
    assert!(!world.columns.contains_key(&HexCoord::from_world(shrine)));
}

#[test]
fn grand_respawn_prefers_bounded_nearby_ground_before_the_start() {
    let (mut session, mut world, geometry, shrine) = fixture();
    destroy_support(&mut world, shrine, 6, None);
    die(&mut session);
    tick(&mut session, &world, geometry);
    let player = session.actors.first().unwrap();
    assert!(player.hp > 0.0);
    let distance = HexCoord::from_world(player.feet).distance(HexCoord::from_world(shrine));
    assert!((7..=12).contains(&distance));
    assert!(crate::shapes::clear(
        &session.collision,
        player,
        player.feet,
        player.body_yaw
    ));
    assert!(crate::shapes::ground(&session.collision, player, player.feet, SKIN * 8.0).is_some());
    assert_eq!(
        session.grand.as_ref().unwrap().respawn_stage,
        RespawnStage::Shrine
    );
    assert_eq!(
        session.grand_progress().unwrap().respawn_anchor,
        Some(ShrineId::Earth)
    );
}

#[test]
fn grand_respawn_pending_fallback_survives_checkpoint_and_waits_for_start() {
    let (mut session, mut world, geometry, shrine) = fixture();
    destroy_support(&mut world, shrine, 12, None);
    let ready = world.residency.as_ref().unwrap().ready.clone();
    world
        .residency
        .as_mut()
        .unwrap()
        .ready
        .retain(|(q, _)| *q >= 1);
    world.revision += 1;
    die(&mut session);
    tick(&mut session, &world, geometry);
    tick(&mut session, &world, geometry);
    let start = *world.spawns.first().unwrap();
    assert_eq!(
        session.grand.as_ref().unwrap().respawn_stage,
        RespawnStage::Start
    );
    assert_eq!(
        session.actors.first().unwrap().hp.to_bits(),
        0.0_f32.to_bits()
    );
    assert!(session.grand_actor_interests().contains(&start));
    assert!(!session.grand_actor_interests().contains(&shrine));
    assert!(session.notice.contains("Loading safe ground"));
    let identity = GrandCheckpointIdentity {
        world_id: "respawn-test".into(),
        content_revision: "1".into(),
    };
    let bytes = session.encode_grand_checkpoint(&identity).unwrap();
    assert_eq!(
        ArenaSession::grand_checkpoint_position(&bytes, &identity).unwrap(),
        start
    );
    let environment = session.ocean_environment.clone();
    let mut restored =
        ArenaSession::decode_grand_checkpoint(&bytes, &identity, &world, geometry, 5).unwrap();
    restored.ocean_environment = environment;
    tick(&mut restored, &world, geometry);
    assert_eq!(
        restored.actors.first().unwrap().hp.to_bits(),
        0.0_f32.to_bits()
    );
    assert_eq!(
        restored.grand.as_ref().unwrap().respawn_stage,
        RespawnStage::Start
    );
    world.residency.as_mut().unwrap().ready = ready;
    world.revision += 1;
    tick(&mut restored, &world, geometry);
    assert!(restored.actors.first().unwrap().hp > 0.0);
    assert!((restored.actors.first().unwrap().feet - start).length() < 0.01);
    assert!(restored.earth_construction(0));
    assert_eq!(restored.grand_progress().unwrap().deaths, 1);
    assert_eq!(
        restored.grand_progress().unwrap().respawn_anchor,
        Some(ShrineId::Earth)
    );
    assert!(!world.columns.contains_key(&HexCoord::from_world(shrine)));
}

#[test]
fn grand_respawn_reports_terminal_failure_without_resetting_run() {
    let (mut session, mut world, geometry, shrine) = fixture();
    let start = *world.spawns.first().unwrap();
    destroy_support(&mut world, shrine, 12, None);
    destroy_support(&mut world, start, 12, None);
    die(&mut session);
    for _ in 0..3 {
        tick(&mut session, &world, geometry);
    }
    assert_eq!(
        session.grand.as_ref().unwrap().respawn_stage,
        RespawnStage::Failed
    );
    assert!(session.notice.contains("No safe ground remains"));
    assert!(session.notice.contains("New Run"));
    assert!(session.earth_construction(0));
    assert_eq!(session.grand_progress().unwrap().deaths, 0);
    let identity = GrandCheckpointIdentity {
        world_id: "respawn-test".into(),
        content_revision: "1".into(),
    };
    let bytes = session.encode_grand_checkpoint(&identity).unwrap();
    let mut restored =
        ArenaSession::decode_grand_checkpoint(&bytes, &identity, &world, geometry, 5).unwrap();
    tick(&mut restored, &world, geometry);
    assert_eq!(
        restored.actors.first().unwrap().hp.to_bits(),
        0.0_f32.to_bits()
    );
    assert!(restored.notice.contains("No safe ground remains"));
    assert!(restored.earth_construction(0));
    assert!(
        restored.outcome.is_none(),
        "the dedicated failure state stays saveable"
    );
}

#[test]
fn grand_respawn_does_not_use_implicit_submerged_crater_support() {
    #[derive(Debug)]
    struct Ocean;
    impl OceanEnvironmentSampler for Ocean {
        fn inundation_column_at(&self, _: Vec2) -> Option<OceanWaterColumn> {
            Some(OceanWaterColumn {
                mean_height: 16.0,
                bed_height: 0.0,
                water_id: SubstanceId(5),
            })
        }
        fn surface_at(
            &self,
            _: Vec2,
            _: f32,
            column: OceanWaterColumn,
        ) -> Option<OceanSurfaceSample> {
            Some(OceanSurfaceSample {
                height: 16.0,
                normal: Vec3::Y,
                vertical_velocity: 0.0,
                mean_height: column.mean_height,
                bed_height: column.bed_height,
                water_id: column.water_id,
            })
        }
    }
    let (mut session, mut world, geometry, shrine) = fixture();
    destroy_support(&mut world, shrine, 12, Some(8));
    session.ocean_environment.as_mut().unwrap().sampler = Arc::new(Ocean);
    die(&mut session);
    for _ in 0..3 {
        tick(&mut session, &world, geometry);
    }
    assert!(
        world.liquids.is_empty(),
        "implicit ocean is absent from compact inland liquid runs"
    );
    let player = session.actors.first().unwrap();
    assert!(player.hp > 0.0);
    assert!((player.feet - *world.spawns.first().unwrap()).length() < 0.01);
    assert!(player.feet.y > 16.0);
}

#[test]
fn grand_respawn_stage_defaults_for_existing_checkpoint_and_rejects_live_fallback() {
    let (mut session, world, geometry, _) = fixture();
    let identity = GrandCheckpointIdentity {
        world_id: "respawn-test".into(),
        content_revision: "1".into(),
    };
    let bytes = session.encode_grand_checkpoint(&identity).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.contains("respawn_stage:Shrine,"));
    let old = text.replace("respawn_stage:Shrine,", "");
    let restored =
        ArenaSession::decode_grand_checkpoint(old.as_bytes(), &identity, &world, geometry, 5)
            .unwrap();
    assert_eq!(
        restored.grand.as_ref().unwrap().respawn_stage,
        RespawnStage::Shrine
    );
    let grand = session.grand.as_mut().unwrap();
    grand.respawn_stage = RespawnStage::Start;
    grand.respawn_interest = Some(grand.start);
    assert!(session
        .encode_grand_checkpoint(&identity)
        .unwrap_err()
        .contains("inconsistent death-recovery"));
}
