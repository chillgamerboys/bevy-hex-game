//! Storage-independent checks of the production HUD action routing.
use super::*;
use bevy::window::PrimaryWindow;

fn menu_fixture() -> App {
    let mut app = App::new();
    app.insert_resource(ViewState {
        started: false,
        paused: true,
        capture: None,
        ..default()
    })
    .insert_resource(ArenaSelection {
        map: ArenaMap::GrandV4,
        ..default()
    })
    .insert_resource(State {
        available: true,
        token: Some(CheckpointToken {
            generation: 17,
            fingerprint: 91,
        }),
        ..default()
    })
    .init_resource::<ArenaReset>()
    .init_resource::<ArenaTuning>()
    .init_resource::<hex_arena::ArenaBattleSetup>()
    .init_resource::<ArenaInput>()
    .init_resource::<ArenaSession>()
    .add_message::<AppExit>();
    app.world_mut().spawn((
        Window {
            focused: true,
            ..default()
        },
        PrimaryWindow,
    ));
    app
}

#[test]
fn grand_menu_buttons_queue_exact_manager_requests_without_exiting_or_resetting() {
    let mut app = menu_fixture();
    let token = app.world().resource::<State>().token;
    let generation = app.world().resource::<ArenaReset>().generation;
    // Start and Continue share the primary action; the manager chooses from the
    // durable slot's availability. Neither button may bypass that manager.
    for available in [false, true] {
        app.world_mut().resource_mut::<State>().available = available;
        press_menu_action(&mut app, hud::Action::Start);
        assert!(matches!(
            app.world().resource::<State>().request,
            Some(Request::Primary)
        ));
        assert!(!app.world().resource::<ViewState>().started);
        assert!(app.world().resource::<ViewState>().paused);
    }
    for action in [
        hud::Action::NewRun,
        hud::Action::ConfirmNew,
        hud::Action::CancelNew,
        hud::Action::Quit,
    ] {
        app.world_mut().resource_mut::<State>().request = None;
        press_menu_action(&mut app, action);
        assert!(matches!(
            (action, app.world().resource::<State>().request),
            (hud::Action::NewRun, Some(Request::NewRun))
                | (hud::Action::ConfirmNew, Some(Request::ConfirmNew))
                | (hud::Action::CancelNew, Some(Request::CancelNew))
                | (hud::Action::Quit, Some(Request::SaveQuit))
        ));
        assert_eq!(app.world().resource::<State>().token, token);
        assert!(app.world().resource::<State>().available);
        assert_eq!(app.world().resource::<ArenaReset>().generation, generation);
        assert!(app.world().resource::<ViewState>().paused);
        assert!(app.world().resource::<Messages<AppExit>>().is_empty());
    }
}

#[test]
fn grand_restart_requires_confirmation_and_resume_waits_for_cancel() {
    let mut app = menu_fixture();
    app.world_mut().resource_mut::<ViewState>().started = true;
    let token = app.world().resource::<State>().token;
    let generation = app.world().resource::<ArenaReset>().generation;
    press_menu_action(&mut app, hud::Action::Restart);
    assert!(app.world().resource::<State>().confirmation);
    assert!(app
        .world()
        .resource::<State>()
        .status
        .contains("Confirm New Run or Cancel"));
    assert_eq!(app.world().resource::<ArenaReset>().generation, generation);
    press_menu_action(&mut app, hud::Action::Resume);
    assert!(app.world().resource::<ViewState>().paused);
    assert_eq!(app.world().resource::<State>().token, token);

    press_menu_action(&mut app, hud::Action::CancelNew);
    // This resource-only fixture has no tuning or map plugins, so this manager
    // branch cannot open storage, stream terrain, or advance gameplay.
    update(app.world_mut());
    assert!(!app.world().resource::<State>().confirmation);
    assert!(app.world().resource::<State>().available);
    assert_eq!(app.world().resource::<State>().token, token);
    assert_eq!(app.world().resource::<ArenaReset>().generation, generation);
    press_menu_action(&mut app, hud::Action::Resume);
    assert!(!app.world().resource::<ViewState>().paused);
    assert!(app.world().resource::<ViewState>().started);
    assert!(app.world().resource::<Messages<AppExit>>().is_empty());
}

#[test]
fn pending_shrine_fallback_restores_start_interest_and_waits_for_ready_adoption() {
    use hex_core::arena::{
        ArenaExpeditionSites, ArenaPackageIdentity, ArenaResidency, ArenaSolidSpan,
    };
    use hex_core::ocean::{
        OceanEnvironmentSampler, OceanSurfaceSample, OceanWaterColumn, OceanWindProfile,
    };
    use hex_core::{ElementId, HexCoord, SubstanceId};

    #[derive(Debug)]
    struct Dry;
    impl OceanEnvironmentSampler for Dry {
        fn surface_at(&self, _: Vec2, _: f32, _: OceanWaterColumn) -> Option<OceanSurfaceSample> {
            None
        }
    }

    // This checks application composition after the map owner has staged its
    // published facts. The separate process/store tests own durable map staging;
    // this small typed world does not pretend to be a compiled Grand package.
    let mut app = menu_fixture();
    let geometry = ArenaVoxelGeometry {
        level_height: 1.0,
        radius: 64,
        ..default()
    };
    let start = Vec3::Y * 20.0001;
    let shrine = HexCoord::from_axial(32, 0).to_world(start.y);
    let catalogue: std::collections::BTreeSet<_> = (-5..=5)
        .flat_map(|q| (-5..=5).map(move |r| (q, r)))
        .collect();
    app.insert_resource(geometry)
        .insert_resource(ArenaTerrainView {
            revision: 1,
            selection: ArenaSelection {
                map: ArenaMap::GrandV4,
                ..default()
            },
            package_identity: Some(ArenaPackageIdentity {
                world_id: "typed-respawn-composition".into(),
                manifest_fingerprint: 7,
                sites_fingerprint: Some(11),
            }),
            spawns: [start, start],
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
                catalogue: catalogue.clone(),
                ready: catalogue.clone(),
            }),
            expedition: Some(ArenaExpeditionSites::default()),
            anchors: [(ShrineId::Earth.anchor().into(), shrine)].into(),
            ..default()
        })
        .insert_resource(ArenaMaterials {
            stone: SubstanceId(1),
            reinforced_stone: Some(SubstanceId(6)),
            bedrock: SubstanceId(2),
            grass: SubstanceId(3),
            dirt: SubstanceId(4),
            fire: ElementId(1),
        })
        .insert_resource(OceanEnvironmentView {
            package_fingerprint: 7,
            sampler: Arc::new(Dry),
            wind: OceanWindProfile::default(),
        })
        .init_resource::<ArenaActorStreamInterests>()
        .init_resource::<ArenaStreamInterest>()
        .add_plugins(hex_arena::plugin);
    app.world_mut().resource_mut::<ArenaSession>().bot_enabled = false;
    app.world_mut().run_schedule(ArenaTick);
    {
        let mut session = app.world_mut().resource_mut::<ArenaSession>();
        let actor = session.actors.first_mut().expect("admitted Grand player");
        actor.feet = shrine;
        actor.previous_feet = shrine;
    }
    app.world_mut().resource_mut::<ArenaInput>().human = ActorIntent {
        interact: true,
        ..default()
    };
    app.world_mut().run_schedule(ArenaTick);
    let progress = app
        .world()
        .resource::<ArenaSession>()
        .grand_progress()
        .expect("claimed Grand shrine");
    assert_eq!(progress.shrines, vec![ShrineId::Earth]);
    assert_eq!(progress.respawn_anchor, Some(ShrineId::Earth));
    {
        let mut terrain = app.world_mut().resource_mut::<ArenaTerrainView>();
        for coord in HexCoord::from_world(shrine).within_radius(12) {
            terrain.columns.remove(&coord);
        }
        terrain
            .residency
            .as_mut()
            .expect("streamed fixture")
            .ready
            .retain(|(q, _)| *q >= 1);
        terrain.revision += 1;
        terrain.full_rebuild = true;
    }
    app.world_mut()
        .resource_mut::<ArenaSession>()
        .actors
        .first_mut()
        .expect("player")
        .hp = 0.0;
    for _ in 0..2 {
        app.world_mut().run_schedule(ArenaTick);
    }
    let generation = app.world().resource::<ArenaReset>().generation;
    {
        let mut state = app.world_mut().resource_mut::<State>();
        state.identity = Some(CheckpointIdentity {
            world_id: "typed-respawn-composition".into(),
            manifest_fingerprint: 7,
            content_version: CONTENT_VERSION,
        });
        state.content_revision = "typed-respawn-composition".into();
        state.configured_generation = Some(generation);
    }
    let identity = gameplay_identity(app.world().resource::<State>()).expect("fixture identity");
    let session = app.world().resource::<ArenaSession>();
    assert_eq!(
        session
            .actors
            .first()
            .expect("waiting dead player")
            .hp
            .to_bits(),
        0.0_f32.to_bits(),
        "fallback must stay pending while start is unloaded: {}",
        session.notice
    );
    assert!(
        session.grand_actor_interests().contains(&start),
        "pending fallback must request the start, notice={:?}, interests={:?}",
        session.notice,
        session.grand_actor_interests()
    );
    assert!(!session.grand_actor_interests().contains(&shrine));
    let bytes = session
        .encode_grand_checkpoint(&identity)
        .expect("pending fallback checkpoint");
    let saved_tick = session.tick;
    let saved_reward = reward_key(session);
    let saved_clock = session.ocean_time().seconds;
    let position = ArenaSession::grand_checkpoint_position(&bytes, &identity)
        .expect("saved recovery destination");
    assert_eq!(position, start);
    let restored = ArenaSession::decode_grand_checkpoint(
        &bytes,
        &identity,
        app.world().resource::<ArenaTerrainView>(),
        geometry,
        generation,
    )
    .expect("staged gameplay owner");
    app.world_mut().insert_resource(ArenaSession::default());
    app.world_mut().resource_mut::<State>().restoring = Some(Restoring {
        session: restored,
        app: AppCheckpoint {
            content_revision: identity.content_revision,
            sites_fingerprint: 11,
            tick: saved_tick,
            environment_seconds: saved_clock,
            yaw: 0.0,
            pitch: 0.0,
            third_person: false,
        },
        position,
        started: Instant::now(),
    });
    before_frame(app.world_mut());
    assert_eq!(
        app.world().resource::<ArenaStreamInterest>().position,
        start
    );
    let interests = &app
        .world()
        .resource::<ArenaActorStreamInterests>()
        .positions;
    assert!(interests.contains(&start));
    assert!(!interests.contains(&shrine));
    finish_restore(app.world_mut());
    assert!(app.world().resource::<State>().restoring.is_some());
    assert!(app.world().resource::<ViewState>().paused);
    assert!(!app.world().resource::<ArenaSession>().is_grand_run());

    // Only the typed world publication makes the destination Ready. Adopting
    // the checkpoint itself must neither simulate a tick nor heal the corpse.
    {
        let mut terrain = app.world_mut().resource_mut::<ArenaTerrainView>();
        terrain.residency.as_mut().expect("streamed fixture").ready = catalogue;
        terrain.revision += 1;
    }
    before_frame(app.world_mut());
    finish_restore(app.world_mut());
    assert!(app.world().resource::<State>().restoring.is_none());
    assert!(app.world().resource::<State>().active);
    assert!(!app.world().resource::<ViewState>().paused);
    let adopted = app.world().resource::<ArenaSession>();
    assert_eq!(adopted.tick, saved_tick);
    assert_eq!(reward_key(adopted), saved_reward);
    assert_eq!(adopted.grand_progress(), Some(progress.clone()));
    assert_eq!(
        adopted.actors.first().expect("dead player").hp.to_bits(),
        0.0_f32.to_bits()
    );
    assert_eq!(
        app.world()
            .resource::<OceanSimulationTime>()
            .seconds
            .to_bits(),
        saved_clock.to_bits()
    );

    app.world_mut().run_schedule(ArenaTick);
    let recovered = app.world().resource::<ArenaSession>();
    let player = recovered.actors.first().expect("recovered player");
    assert!(player.hp > 0.0 && player.feet.distance(start) < 0.01);
    let after = recovered.grand_progress().expect("retained Grand progress");
    assert_eq!(after.shrines, progress.shrines);
    assert_eq!(after.respawn_anchor, progress.respawn_anchor);
    assert_eq!(after.deaths, 1);
    assert!(!app
        .world()
        .resource::<ArenaTerrainView>()
        .columns
        .contains_key(&HexCoord::from_world(shrine)));
}
