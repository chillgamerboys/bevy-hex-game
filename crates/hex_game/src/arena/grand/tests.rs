//! Opt-in, separate-process validation of the real application checkpoint boundary.
//! `tools/grand_verify.py` supplies a real package and isolated storage for each case.
use super::super::hud;
use super::*;
use bevy::ecs::system::RunSystemOnce;
use hex_arena::{ActorIntent, ShrineId, Spell};
use hex_core::arena::{ArenaAvailability, ArenaMaterials};
use hex_core::ocean::OceanEnvironmentView;
use hex_core::{
    DamagedVoxels, TerrainBatchId, TerrainDamageKind, TerrainEdit, TerrainImpact, TilePos,
};
use hex_map::arena::streamed::StreamedArena;
use hex_map::ocean::{OceanBathymetry, OceanSurfaceAdapter, OceanSurfaceProfile};
use hex_test_app::HeadlessAppBuilder;
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};

#[path = "admission_tests.rs"]
mod admission_tests;
#[path = "circuit_tests.rs"]
mod circuit_tests;
#[path = "sailing_tests.rs"]
mod sailing_tests;
#[path = "ui_flow_tests.rs"]
mod ui_flow_tests;
#[cfg(feature = "test-support")]
#[path = "walking_tests.rs"]
mod walking_tests;

#[derive(Debug, Serialize, Deserialize)]
struct Receipt {
    mode: String,
    write_pid: u32,
    tick: u64,
    environment_seconds_bits: u64,
    player_position: [f32; 3],
    projectiles: usize,
    carved: TilePos,
    boundary_carved: TilePos,
    damaged: TilePos,
    remaining: u8,
    maximum: u8,
    world_records: BTreeMap<String, u64>,
}

fn fixture() -> App {
    let mut builder = HeadlessAppBuilder::new()
        .with_minimal_plugins()
        .with_fixed_step(Duration::from_secs_f64(1.0 / 60.0));
    builder
        .app_mut()
        .insert_resource(ArenaSelection {
            map: ArenaMap::GrandV4,
            ..default()
        })
        .insert_resource(ViewState {
            started: false,
            paused: true,
            capture: None,
            ..default()
        })
        .add_message::<AppExit>()
        .configure_sets(
            Update,
            (ArenaFrame::Input, ArenaFrame::Tick, ArenaFrame::Present).chain(),
        )
        .add_plugins((hex_map::arena::plugin, hex_arena::plugin))
        .add_systems(
            Update,
            super::super::northern::interest.in_set(ArenaFrame::Input),
        )
        .add_systems(
            Update,
            super::super::drive_simulation.in_set(ArenaFrame::Tick),
        );
    install(builder.app_mut());
    let mut app = builder.build();
    pump_until(&mut app, "initial package and gameplay", |world| {
        world.resource::<ArenaSession>().is_grand_run()
            && world.resource::<State>().identity.is_some()
            && world.resource::<State>().tuning.is_some()
    });
    app.world_mut().resource_mut::<ArenaSession>().bot_enabled = false;
    publish_ocean(app.world_mut());
    app
}

#[expect(
    clippy::expect_used,
    reason = "A real menu action must run through the production HUD system or fail this UX acceptance test."
)]
fn press_menu_action(app: &mut App, action: hud::Action) {
    let button = app.world_mut().spawn((Interaction::Pressed, action)).id();
    app.world_mut()
        .run_system_once(hud::buttons)
        .expect("Grand HUD button system");
    app.world_mut().despawn(button);
}

fn pump_until(app: &mut App, reason: &str, ready: impl Fn(&World) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        app.update();
        if let Some(streamed) = app.world().get_resource::<StreamedArena>() {
            assert!(
                streamed.failure.is_none(),
                "stream failure while {reason}: {:?}",
                streamed.failure
            );
        }
        if ready(app.world()) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timeout while {reason}: {}",
            app.world().resource::<State>().status
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[expect(
    clippy::expect_used,
    reason = "The actual-package fixture requires valid authored ocean geometry before any marine state can be verified."
)]
fn publish_ocean(world: &mut World) {
    let map = Arc::clone(&world.resource::<StreamedArena>().overview);
    let geometry = *world.resource::<ArenaVoxelGeometry>();
    let bath = OceanBathymetry {
        revision: map.package_fingerprint,
        origin_xz: Vec2::from_array(map.origin_xz),
        spacing: map.spacing,
        width: map.width,
        height: map.height,
        bed_heights: map.bed_heights.clone(),
        ..default()
    }
    .with_shore_shelter(map.sea_level, 80.0)
    .expect("actual package bathymetry");
    let wind = super::super::northern::prevailing_wind(
        world.resource::<StreamedArena>(),
        ArenaMap::GrandV4,
    );
    let profile = OceanSurfaceProfile::regular_voxels(map.sea_level, geometry.level_height)
        .with_wind_response(wind);
    let field = hex_map::water_lab::WindField::for_region(
        world.resource::<ArenaTerrainView>(),
        geometry,
        map.sea_level,
        None,
    );
    world.insert_resource(OceanEnvironmentView {
        package_fingerprint: map.package_fingerprint,
        sampler: Arc::new(
            OceanSurfaceAdapter::new(profile, bath)
                .expect("ocean snapshot")
                .with_wind_field(field),
        ),
        wind,
    });
}

fn step(app: &mut App, intent: ActorIntent) {
    // Real PreUpdate transfers last-tick announcements to the authoritative inbox.
    app.update();
    before_frame(app.world_mut());
    app.world_mut().resource_mut::<ArenaInput>().human = intent;
    app.world_mut().run_schedule(ArenaTick);
}

#[expect(
    clippy::expect_used,
    reason = "The integration fixture must contain the production human actor before relocating its streaming interest."
)]
fn relocate(app: &mut App, position: Vec3) {
    app.world_mut().resource_mut::<ViewState>().pause();
    {
        let mut session = app.world_mut().resource_mut::<ArenaSession>();
        let player = session
            .actors
            .iter_mut()
            .find(|actor| actor.id == 0)
            .expect("real human");
        player.feet = position;
        player.previous_feet = position;
        player.grounded = false;
    }
    app.world_mut()
        .resource_mut::<ArenaStreamInterest>()
        .position = position;
    pump_until(app, "fixture destination", |world| {
        hex_map::arena::checkpoint::restore_ready(world, position)
    });
}

#[expect(
    clippy::expect_used,
    reason = "Missing authored actors, shrine sites or acknowledged terrain damage must fail this persistence fixture immediately."
)]
fn prepare_progress_and_damage(app: &mut App) -> (TilePos, TilePos, u8, u8) {
    // Admit a real authored party; never inject enemy IDs or incomplete rosters.
    let geometry = *app.world().resource::<ArenaVoxelGeometry>();
    let deployment = app
        .world()
        .resource::<ArenaTerrainView>()
        .expedition
        .as_ref()
        .expect("authored expedition")
        .encounters
        .get("grand_goblin_01")
        .expect("authored goblin party")
        .deployment
        .clone();
    let site = deployment.preferred;
    relocate(app, site.coord.to_world(geometry.top(site) + 0.02));
    // relocate guarantees the player's immediate support only. Party admission
    // requires the entire authored disk, which streams asynchronously.
    pump_until(app, "complete authored goblin deployment", |world| {
        let terrain = world.resource::<ArenaTerrainView>();
        let residency = terrain.residency.as_ref().expect("finite residency");
        deployment
            .surfaces
            .iter()
            .all(|position| residency.at(position.coord, geometry) == ArenaAvailability::Ready)
    });
    for _ in 0..12 {
        step(app, ActorIntent::default());
    }
    let session = app.world().resource::<ArenaSession>();
    let terrain = app.world().resource::<ArenaTerrainView>();
    assert!(
        session.actors.iter().any(|actor| actor.id != 0),
        "real party admission after complete residency: notice={}, tick={}, preferred_solid={}, compact_columns={}, explicit_voxels={}",
        session.notice,
        session.tick,
        terrain.solid_at(site).is_some(),
        terrain.columns.len(),
        terrain.voxels.len(),
    );
    if let Some(enemy) = app
        .world_mut()
        .resource_mut::<ArenaSession>()
        .actors
        .iter_mut()
        .find(|actor| actor.id != 0)
    {
        enemy.hp = (enemy.max_hp - 3.0).max(1.0);
    }

    let shrine = *app
        .world()
        .resource::<ArenaTerrainView>()
        .anchors
        .get(ShrineId::Earth.anchor())
        .expect("Earth shrine");
    relocate(app, shrine);
    step(
        app,
        ActorIntent {
            interact: true,
            ..default()
        },
    );
    assert!(app
        .world()
        .resource::<ArenaSession>()
        .grand_progress()
        .expect("Grand")
        .shrines
        .contains(&ShrineId::Earth));

    let materials = *app.world().resource::<ArenaMaterials>();
    let substances = app.world().resource::<hex_assets::SubstanceTable>();
    let view = app.world().resource::<ArenaTerrainView>();
    let candidates: Vec<_> = view
        .columns
        .values()
        .flatten()
        .filter(|span| {
            span.substance != materials.bedrock
                && substances
                    .toughness(span.substance)
                    .is_some_and(|health| health > 1)
        })
        .filter(|span| span.top_level > span.bottom.level)
        .map(|span| TilePos::new(span.bottom.coord, span.top_level - 1))
        .take(2)
        .collect();
    let mut candidates = candidates.into_iter();
    let carved = candidates.next().expect("first destructible terrain cell");
    let damaged = candidates.next().expect("second destructible terrain cell");
    app.world_mut()
        .write_message(TerrainEdit::Clear { pos: carved });
    app.world_mut().write_message(TerrainImpact {
        batch: TerrainBatchId(9_000_001),
        volume: vec![damaged],
        kind: TerrainDamageKind::Physical,
        power: 1,
    });
    app.update();
    settle(app.world_mut()).expect("real map mutation boundary");
    assert!(app
        .world()
        .resource::<ArenaTerrainView>()
        .solid_at(carved)
        .is_none());
    let health = app
        .world()
        .resource::<DamagedVoxels>()
        .get(damaged)
        .expect("partial terrain health");
    assert!(health.remaining < health.maximum);
    (carved, damaged, health.remaining, health.maximum)
}

#[expect(
    clippy::expect_used,
    clippy::panic,
    reason = "The fixture requires a valid authored pose for its selected movement case and must reject unsupported externally selected modes."
)]
fn prepare_mode(app: &mut App, mode: &str) {
    let start = app
        .world()
        .resource::<ArenaTerrainView>()
        .spawns
        .first()
        .copied()
        .expect("authored player spawn");
    match mode {
        "land" => {
            relocate(app, start);
            for _ in 0..12 {
                step(app, ActorIntent::default());
            }
        }
        "air" => {
            relocate(app, start + Vec3::Y * 35.0);
        }
        "boat" => {
            let map = Arc::clone(&app.world().resource::<StreamedArena>().overview);
            let width = usize::try_from(map.width).expect("validated overview width");
            let [origin_x, origin_z] = map.origin_xz;
            let coarse = map
                .bed_heights
                .iter()
                .enumerate()
                .filter(|(_, height)| **height < map.sea_level - 5.0)
                .map(|(index, _)| {
                    let x = f32::from(u16::try_from(index % width).expect("bounded overview x"));
                    let z = f32::from(u16::try_from(index / width).expect("bounded overview z"));
                    Vec3::new(
                        origin_x + x * map.spacing,
                        map.sea_level,
                        origin_z + z * map.spacing,
                    )
                })
                .min_by(|a, b| {
                    a.distance_squared(start)
                        .total_cmp(&b.distance_squared(start))
                })
                .expect("deep ocean grid point");
            relocate(app, coarse);
            let geometry = *app.world().resource::<ArenaVoxelGeometry>();
            let view = app.world().resource::<ArenaTerrainView>();
            let water = view
                .liquids
                .iter()
                .filter(|span| {
                    (geometry.top(TilePos::new(span.bottom.coord, span.top_level)) - map.sea_level)
                        .abs()
                        < 0.01
                        && u16::try_from(span.top_level - span.bottom.level)
                            .is_ok_and(|depth| f32::from(depth) * geometry.level_height > 3.0)
                })
                .map(|span| span.bottom.coord.to_world(map.sea_level - 0.2))
                .min_by(|a, b| {
                    a.distance_squared(coarse)
                        .total_cmp(&b.distance_squared(coarse))
                })
                .expect("admitted deep ocean column");
            relocate(app, water);
            publish_ocean(app.world_mut());
            step(
                app,
                ActorIntent {
                    boat_toggle: true,
                    aim: Vec3::X,
                    ..default()
                },
            );
            for _ in 0..12 {
                step(
                    app,
                    ActorIntent {
                        movement: Vec2::Y,
                        aim: Vec3::X,
                        ..default()
                    },
                );
            }
        }
        _ => panic!("unsupported verification mode"),
    }
    if mode != "air" {
        assert_mode(app.world(), mode);
    }
    let mut session = app.world_mut().resource_mut::<ArenaSession>();
    let player = session
        .actors
        .iter_mut()
        .find(|actor| actor.id == 0)
        .expect("human");
    player.hp = player.max_hp - 7.0;
}

#[expect(
    clippy::expect_used,
    reason = "Movement acceptance requires the production human actor to exist."
)]
fn assert_mode(world: &World, mode: &str) {
    let player = world
        .resource::<ArenaSession>()
        .actors
        .iter()
        .find(|actor| actor.id == 0)
        .expect("human");
    match mode {
        "land" => assert!(player.grounded, "land save must have grounded support"),
        "boat" => assert!(
            player
                .boat()
                .is_some_and(|boat| boat.active && boat.velocity.length() > 0.0),
            "boat save must retain active moving boat"
        ),
        "air" => assert!(
            player
                .glider()
                .is_some_and(|glider| glider.open && glider.velocity.length() > 0.0),
            "air save must retain the open momentum glider"
        ),
        _ => unreachable!(),
    }
}

#[expect(
    clippy::expect_used,
    reason = "An owner export failure must fail the complete-state equality assertion rather than omit a partition."
)]
fn world_records(world: &World) -> BTreeMap<String, u64> {
    hex_map::arena::checkpoint::export_records(world)
        .expect("world export")
        .map(|record| {
            let record = record.expect("partition export");
            (record.key, xxhash_rust::xxh3::xxh3_64(&record.bytes))
        })
        .collect()
}

#[expect(
    clippy::expect_used,
    reason = "This acceptance phase must produce and reopen a complete real application save or fail with the precise missing boundary."
)]
fn write_phase(app: &mut App, mode: &str, root: &std::path::Path) {
    assert!(
        !slot_path().exists(),
        "writer fixture must start in isolated empty storage"
    );
    let initial_generation = app.world().resource::<ArenaReset>().generation;
    press_menu_action(app, hud::Action::NewRun);
    update(app.world_mut());
    assert!(app.world().resource::<State>().confirmation);
    assert!(app.world().resource::<ViewState>().paused);
    assert_eq!(
        app.world().resource::<ArenaReset>().generation,
        initial_generation
    );
    assert!(
        !slot_path().exists(),
        "requesting New Run cannot create a slot"
    );
    press_menu_action(app, hud::Action::CancelNew);
    update(app.world_mut());
    assert!(!app.world().resource::<State>().confirmation);
    press_menu_action(app, hud::Action::ConfirmNew);
    update(app.world_mut());
    assert_eq!(
        app.world().resource::<ArenaReset>().generation,
        initial_generation
    );
    assert!(
        !slot_path().exists(),
        "cancelled or unsolicited confirmation cannot reset storage"
    );
    assert!(!app.world().resource::<State>().active);

    press_menu_action(app, hud::Action::NewRun);
    update(app.world_mut());
    press_menu_action(app, hud::Action::ConfirmNew);
    update(app.world_mut());
    assert!(!app.world().resource::<State>().confirmation);
    assert!(app.world().resource::<State>().active);
    assert!(app.world().resource::<ViewState>().started);
    assert_eq!(
        app.world().resource::<ArenaReset>().generation,
        initial_generation + 1
    );
    app.world_mut().resource_mut::<ViewState>().pause();
    pump_until(app, "confirmed New Run initial autosave", |world| {
        !world.resource::<State>().busy()
    });
    assert!(
        app.world().resource::<State>().available,
        "{}",
        app.world().resource::<State>().status
    );
    {
        let mut state = app.world_mut().resource_mut::<State>();
        state.active = false; // fixture setup is explicitly outside ordinary autosave policy
        state.force_save = false;
    }
    let (carved, damaged, remaining, maximum) = prepare_progress_and_damage(app);
    prepare_mode(app, mode);
    // Release a real projectile. Land and boat also preserve a held charge;
    // airborne saving opens the real glider after casting, because casting folds it.
    let shot = Vec3::new(1.0, 0.7, 0.0).normalize();
    step(
        app,
        ActorIntent {
            aim: shot,
            selected: Some(Spell::Shield),
            cast_pressed: true,
            cast_held: true,
            ..default()
        },
    );
    step(
        app,
        ActorIntent {
            aim: shot,
            cast_released: true,
            ..default()
        },
    );
    if mode == "air" {
        app.world_mut()
            .resource_scope(|world, mut session: Mut<ArenaSession>| {
                let feet = session
                    .actors
                    .iter()
                    .find(|actor| actor.id == 0)
                    .expect("human")
                    .feet;
                assert!(
                    session.start_exploration_glide(
                        feet,
                        Vec3::X,
                        world.resource::<ArenaTerrainView>(),
                        *world.resource::<ArenaVoxelGeometry>()
                    ),
                    "real glider admission at airborne pose"
                );
            });
    } else {
        step(
            app,
            ActorIntent {
                aim: shot,
                selected: Some(Spell::Fireball),
                cast_pressed: true,
                cast_held: true,
                ..default()
            },
        );
        step(
            app,
            ActorIntent {
                aim: shot,
                cast_held: true,
                ..default()
            },
        );
    }
    assert_mode(app.world(), mode);
    assert!(
        !app.world()
            .resource::<ArenaSession>()
            .projectiles
            .is_empty(),
        "active projectile at save"
    );
    assert!(
        mode == "air"
            || app
                .world()
                .resource::<ArenaSession>()
                .actors
                .first()
                .expect("human")
                .charge()
                .is_some(),
        "held charge at land/boat save"
    );
    // More than sixty hours exposes drift if the app validates against an exact
    // f64 1/120 instead of the gameplay clock's actual f32 fixed step.
    app.world_mut().resource_mut::<ArenaSession>().tick = 25_920_001;
    let clock = app.world().resource::<ArenaSession>().ocean_time();
    app.world_mut().insert_resource(clock);
    {
        let mut view = app.world_mut().resource_mut::<ViewState>();
        view.yaw = 0.63;
        view.pitch = -0.21;
        view.third_person = true;
    }
    let boundary_carved = app
        .world()
        .resource::<ArenaTerrainView>()
        .columns
        .values()
        .flatten()
        .map(|span| TilePos::new(span.bottom.coord, span.top_level))
        .find(|position| *position != carved && *position != damaged)
        .expect("loaded solid for current-frame edit");
    // Announce after the last PreUpdate, exactly as a final gameplay tick does.
    // The application save boundary must drain this without another gameplay tick.
    app.world_mut().write_message(TerrainEdit::Clear {
        pos: boundary_carved,
    });
    app.world_mut().resource_mut::<State>().active = true;
    press_menu_action(app, hud::Action::Quit);
    assert!(matches!(
        app.world().resource::<State>().request,
        Some(Request::SaveQuit)
    ));
    assert!(app.world().resource::<Messages<AppExit>>().is_empty());
    update(app.world_mut());
    assert!(app.world().resource::<ViewState>().paused);
    assert!(
        app.world().resource::<State>().busy(),
        "Save & Quit must wait for its save worker"
    );
    assert!(
        app.world().resource::<Messages<AppExit>>().is_empty(),
        "no exit before durable save completion"
    );
    assert!(
        app.world()
            .resource::<ArenaTerrainView>()
            .solid_at(boundary_carved)
            .is_none(),
        "save must include terrain operations emitted in the current frame"
    );
    pump_until(app, "atomic save worker", |world| {
        !world.resource::<State>().busy()
    });
    assert!(
        app.world().resource::<State>().available,
        "{}",
        app.world().resource::<State>().status
    );
    let exits = app
        .world_mut()
        .resource_mut::<Messages<AppExit>>()
        .drain()
        .collect::<Vec<_>>();
    assert!(
        matches!(exits.as_slice(), [AppExit::Success]),
        "Save & Quit exits once after the durable save"
    );
    let session = app.world().resource::<ArenaSession>();
    let player = session
        .actors
        .iter()
        .find(|actor| actor.id == 0)
        .expect("human");
    let receipt = Receipt {
        mode: mode.into(),
        write_pid: std::process::id(),
        tick: session.tick,
        environment_seconds_bits: clock.seconds.to_bits(),
        player_position: player.feet.to_array(),
        projectiles: session.projectiles.len(),
        carved,
        boundary_carved,
        damaged,
        remaining,
        maximum,
        world_records: world_records(app.world()),
    };
    std::fs::write(
        root.join("expected.json"),
        serde_json::to_vec_pretty(&receipt).expect("receipt"),
    )
    .expect("write receipt");
    // Reopen every owner record from disk, rather than trusting the completed worker.
    let saved = store(app.world().resource::<State>())
        .expect("store")
        .load()
        .expect("head")
        .expect("saved slot");
    saved
        .verify_all(&CancellationToken::default())
        .expect("all owner bodies durable");
}

#[expect(
    clippy::expect_used,
    reason = "This fresh-process acceptance phase must restore every required owner and write its verification receipt or fail immediately."
)]
fn read_phase(app: &mut App, mode: &str, root: &std::path::Path) {
    let receipt: Receipt =
        serde_json::from_slice(&std::fs::read(root.join("expected.json")).expect("writer receipt"))
            .expect("receipt format");
    assert_ne!(
        receipt.write_pid,
        std::process::id(),
        "resume must run in a fresh process"
    );
    assert_eq!(receipt.mode, mode);
    let snapshot = store(app.world().resource::<State>())
        .expect("store")
        .load()
        .expect("head")
        .expect("saved slot");
    let identity = gameplay_identity(app.world().resource::<State>()).expect("gameplay identity");
    let bytes = snapshot
        .record("gameplay", "session", GAMEPLAY_FORMAT)
        .expect("gameplay record")
        .expect("body");
    let before_token = snapshot.token();
    let initial_generation = app.world().resource::<ArenaReset>().generation;
    press_menu_action(app, hud::Action::NewRun);
    update(app.world_mut());
    assert!(app.world().resource::<State>().confirmation);
    press_menu_action(app, hud::Action::CancelNew);
    update(app.world_mut());
    assert!(!app.world().resource::<State>().confirmation);
    assert!(app.world().resource::<State>().available);
    assert_eq!(app.world().resource::<State>().token, Some(before_token));
    assert_eq!(
        app.world().resource::<ArenaReset>().generation,
        initial_generation
    );
    assert_eq!(
        store(app.world().resource::<State>())
            .expect("store after Cancel")
            .load()
            .expect("head after Cancel")
            .expect("retained resume")
            .token(),
        before_token,
        "Cancel must retain the exact durable resume slot"
    );
    press_menu_action(app, hud::Action::Start);
    assert!(matches!(
        app.world().resource::<State>().request,
        Some(Request::Primary)
    ));
    update(app.world_mut());
    pump_until(
        app,
        "application adoption after collision residency",
        |world| world.resource::<State>().restoring.is_none(),
    );
    assert_eq!(app.world().resource::<State>().status, "Expedition resumed");
    assert_mode(app.world(), mode);
    let expected = ArenaSession::decode_grand_checkpoint(
        &bytes,
        &identity,
        app.world().resource::<ArenaTerrainView>(),
        *app.world().resource::<ArenaVoxelGeometry>(),
        app.world().resource::<ArenaReset>().generation,
    )
    .expect("expected owner state at new generation");
    let actual = app.world().resource::<ArenaSession>();
    assert_eq!(
        actual
            .encode_grand_checkpoint(&identity)
            .expect("restored full state"),
        expected
            .encode_grand_checkpoint(&identity)
            .expect("expected full state"),
        "all durable actors, AI, projectiles, rewards and clocks must round-trip exactly"
    );
    assert_eq!(actual.tick, receipt.tick);
    assert_eq!(actual.projectiles.len(), receipt.projectiles);
    assert!(actual
        .actors
        .first()
        .expect("human")
        .feet
        .abs_diff_eq(Vec3::from_array(receipt.player_position), 0.00001));
    assert_eq!(
        app.world()
            .resource::<OceanSimulationTime>()
            .seconds
            .to_bits(),
        receipt.environment_seconds_bits
    );
    assert_eq!(
        world_records(app.world()),
        receipt.world_records,
        "sparse edits, damage, counters and ledgers"
    );
    assert_eq!(app.world().resource::<State>().token, Some(before_token));
    assert!((app.world().resource::<ViewState>().yaw - 0.63).abs() < 0.00001);
    assert!((app.world().resource::<ViewState>().pitch + 0.21).abs() < 0.00001);
    assert!(app.world().resource::<ViewState>().third_person);
    app.world_mut().resource_mut::<ViewState>().pause();
    // The real host republishes its read-only environment from the newly admitted
    // local terrain before play; the fixture has no renderer/ocean update plugin.
    publish_ocean(app.world_mut());
    step(
        app,
        ActorIntent {
            cast_held: mode != "air",
            aim: Vec3::X,
            ..default()
        },
    );
    assert_eq!(
        app.world().resource::<ArenaSession>().tick,
        receipt.tick + 1,
        "the first resumed gameplay tick must continue the saved clock"
    );
    assert!(app
        .world()
        .resource::<ArenaSession>()
        .actors
        .iter()
        .all(|actor| actor.feet.is_finite()));
    assert_mode(app.world(), mode);
    // Revisit the edited column after the full state comparison: unloaded air
    // must never be accepted as evidence that a carved cell stayed empty.
    let geometry = *app.world().resource::<ArenaVoxelGeometry>();
    relocate(
        app,
        receipt
            .carved
            .coord
            .to_world(geometry.top(receipt.carved) + 20.0),
    );
    let terrain = app.world().resource::<ArenaTerrainView>();
    assert_eq!(
        terrain
            .residency
            .as_ref()
            .expect("finite residency")
            .at(receipt.carved.coord, geometry),
        ArenaAvailability::Ready
    );
    assert!(terrain.solid_at(receipt.carved).is_none());
    relocate(
        app,
        receipt
            .damaged
            .coord
            .to_world(geometry.top(receipt.damaged) + 20.0),
    );
    let terrain = app.world().resource::<ArenaTerrainView>();
    assert_eq!(
        terrain
            .residency
            .as_ref()
            .expect("finite residency")
            .at(receipt.damaged.coord, geometry),
        ArenaAvailability::Ready
    );
    assert!(terrain.solid_at(receipt.damaged).is_some());
    let health = app
        .world()
        .resource::<DamagedVoxels>()
        .get(receipt.damaged)
        .expect("restored partial HP");
    assert_eq!(
        (health.remaining, health.maximum),
        (receipt.remaining, receipt.maximum)
    );
    relocate(
        app,
        receipt
            .boundary_carved
            .coord
            .to_world(geometry.top(receipt.boundary_carved) + 20.0),
    );
    let terrain = app.world().resource::<ArenaTerrainView>();
    assert_eq!(
        terrain
            .residency
            .as_ref()
            .expect("finite residency")
            .at(receipt.boundary_carved.coord, geometry),
        ArenaAvailability::Ready
    );
    assert!(
        terrain.solid_at(receipt.boundary_carved).is_none(),
        "the last-frame edit survived a fresh process"
    );
    let result = serde_json::json!({ "mode": mode, "write_pid": receipt.write_pid, "read_pid": std::process::id(),
        "tick": receipt.tick, "projectiles": receipt.projectiles, "world_records": receipt.world_records.len(),
        "full_owner_state_equal": true, "partial_health_retained": true, "carve_revisited": true,
        "environment_seconds_bits": receipt.environment_seconds_bits,
        "environment_seconds": f64::from_bits(receipt.environment_seconds_bits) });
    std::fs::write(
        root.join("verified.json"),
        serde_json::to_vec_pretty(&result).expect("result"),
    )
    .expect("write result");
}

#[test]
#[ignore = "tools/grand_verify.py launches writer and reader with a real Grand package and isolated storage"]
fn process_checkpoint_child() {
    let phase = std::env::var("HEX_GRAND_VERIFY_PHASE").expect("explicit child phase");
    let mode = std::env::var("HEX_GRAND_VERIFY_MODE").expect("explicit movement case");
    let root =
        PathBuf::from(std::env::var_os("HEX_GAME_DATA_DIR").expect("isolated data directory"));
    assert!(
        root.join("grand-verification-only").is_file(),
        "refuse real user data"
    );
    assert!(
        std::env::var_os("HEX_GRAND_WORLD").is_some(),
        "require an explicit actual package"
    );
    let mut app = fixture();
    match phase.as_str() {
        "write" => write_phase(&mut app, &mode, &root),
        "read" => read_phase(&mut app, &mode, &root),
        _ => panic!("unsupported child phase"),
    }
}

#[test]
fn companion_streaming_preserves_checkpoint_identity_and_rejects_incomplete_reads() {
    let bytes = vec![37_u8; 40_001];
    let length = u64::try_from(bytes.len()).expect("bounded fixture");
    let name = "grand-overview.ron";
    let mut prior = xxhash_rust::xxh3::Xxh3::new();
    prior.update(name.as_bytes());
    prior.update(&length.to_le_bytes());
    prior.update(&bytes);
    let mut current = xxhash_rust::xxh3::Xxh3::new();
    hash_companion(&mut current, name, bytes.as_slice(), length, length)
        .expect("complete companion");
    assert_eq!(current.digest(), prior.digest());
    for (declared, cap) in [
        (length + 1, length + 1),
        (length - 1, length),
        (length, length - 1),
    ] {
        assert!(hash_companion(
            &mut xxhash_rust::xxh3::Xxh3::new(),
            name,
            bytes.as_slice(),
            declared,
            cap
        )
        .is_err());
    }
}
