use super::*;
use crate::{plugin, ArenaBattleSetup, ArenaInput, ArenaOutcome, ArenaSession, ArenaTuning};
use hex_core::arena::{
    ArenaMap, ArenaMaterials, ArenaPackageIdentity, ArenaReset, ArenaResidency, ArenaSelection,
    ArenaSolidSpan, ArenaTick,
};
use hex_core::ocean::{OceanEnvironmentSampler, OceanSurfaceSample, OceanWindProfile};
use hex_core::SubstanceId;
use std::{collections::BTreeSet, sync::Arc};

fn session_app() -> bevy_app::App {
    let (_, _, mut terrain, geometry, environment) = fixture();
    let land = HexCoord::from_axial(-10, 0);
    let dry = land.within_radius(3);
    terrain
        .liquids
        .retain(|span| !dry.contains(&span.bottom.coord));
    for coord in dry {
        terrain
            .voxels
            .insert(TilePos::new(coord, 0), SubstanceId(1));
    }
    terrain.spawns = [land.to_world(crate::collision::SKIN), Vec3::ZERO];
    terrain.package_identity = Some(ArenaPackageIdentity {
        world_id: "test-ocean".into(),
        manifest_fingerprint: 7,
        sites_fingerprint: None,
    });
    let mut app = bevy_app::App::new();
    app.insert_resource(terrain)
        .insert_resource(geometry)
        .insert_resource(ArenaMaterials {
            stone: SubstanceId(1),
            grass: SubstanceId(3),
            dirt: SubstanceId(4),
            bedrock: SubstanceId(5),
            fire: hex_core::ElementId(1),
        })
        .insert_resource(ArenaReset::default())
        .insert_resource(environment)
        .add_plugins(plugin);
    app.world_mut().run_schedule(ArenaTick);
    app
}

#[test]
fn fixed_schedule_consumes_boat_edges_casts_aboard_and_publishes_one_clock() {
    let mut app = session_app();
    {
        let mut session = app.world_mut().resource_mut::<ArenaSession>();
        assert!(session.encounter.initialized);
        let player = session.actors.first_mut().expect("player");
        player.feet = Vec3::Y * DECK;
        player.previous_feet = player.feet;
        player.grounded = false;
        player.body.grounded = false;
    }
    app.world_mut().resource_mut::<ArenaInput>().human = toggle();
    app.world_mut().run_schedule(ArenaTick);
    assert!(
        app.world()
            .resource::<ArenaSession>()
            .actors
            .first()
            .expect("player")
            .boat()
            .expect("boat")
            .active
    );
    assert!(!app.world().resource::<ArenaInput>().human.boat_toggle);
    app.world_mut().resource_mut::<ArenaInput>().human = ActorIntent {
        aim: Vec3::Y,
        cast_pressed: true,
        cast_released: true,
        ..Default::default()
    };
    app.world_mut().run_schedule(ArenaTick);
    let session = app.world().resource::<ArenaSession>();
    assert_eq!(session.projectiles.len(), 1);
    assert!(
        session
            .actors
            .first()
            .expect("player")
            .boat()
            .expect("boat")
            .active
    );
    assert_eq!(
        *app.world().resource::<OceanSimulationTime>(),
        session.ocean_time()
    );
    assert!(session.ocean_time().seconds > 0.0);
    let player = session.actors.first().expect("player");
    let visible_surface = 0.2 * session.ocean_time().phase_seconds().sin();
    // Northern uses the accepted eased float: it follows a rising surface without snapping.
    assert!(player.marine.as_ref().expect("marine").lab);
    assert!(player.feet.y > DECK && player.feet.y < visible_surface + DECK);
    assert!((player.eye().y - player.feet.y - 1.02).abs() < 0.0001);
}

#[test]
#[expect(
    clippy::float_cmp,
    reason = "terminal zeroes and restart constants must be preserved exactly"
)]
fn oxygen_death_freezes_the_empty_session_until_restart_refills_the_player() {
    let mut app = session_app();
    let [spawn, _] = app.world().resource::<ArenaTerrainView>().spawns;
    let underwater = Vec3::Y * -2.0;
    {
        let mut session = app.world_mut().resource_mut::<ArenaSession>();
        assert!(session.encounter.initialized);
        assert!(!session.is_finished());
        assert_eq!(session.actors.len(), 1);
        let player = session.actors.first_mut().expect("explorer");
        player.feet = underwater;
        player.previous_feet = underwater;
        player.grounded = false;
        player.body.grounded = false;
        player.body.vertical_velocity = -1.0;
        player.body.control_velocity = Vec3::X;
        player.body.airborne_momentum = Some(Vec3::X);
        player.hp = 5.0 * STEP; // Less than one actual drowning tick.
        let marine = player.marine.as_mut().expect("marine");
        marine.swim.oxygen_seconds = STEP * 0.5;
        marine.swim.active = true;
        marine.velocity = Vec3::X;
    }
    app.world_mut().run_schedule(ArenaTick);
    let death_clock = *app.world().resource::<OceanSimulationTime>();
    {
        let session = app.world().resource::<ArenaSession>();
        assert_eq!(session.outcome, Some(ArenaOutcome::Draw));
        assert!(session.is_finished());
        let player = session.actors.first().expect("explorer");
        assert_eq!(player.hp, 0.0);
        assert_eq!(player.feet, underwater);
        let swim = player.swimming().expect("swim");
        assert_eq!(swim.oxygen_seconds, 0.0);
        assert!(swim.submerged);
        assert!(!swim.active);
        assert!(!player.boat().expect("boat").active);
        assert!(!player.glider().expect("glider").open);
        assert!(!player.free_flight().expect("flight").active);
        assert_eq!(player.marine.as_ref().expect("marine").velocity, Vec3::ZERO);
        assert_eq!(player.body.vertical_velocity, 0.0);
        assert_eq!(player.body.control_velocity, Vec3::ZERO);
        assert_eq!(player.body.airborne_momentum, None);
    }
    // Esc/defeat cancels casts without refilling oxygen. Even extra fixed ticks
    // with queued travel/cast edges cannot resume a terminal session.
    app.world_mut()
        .resource_mut::<ArenaSession>()
        .cancel_charges();
    for _ in 0..3 {
        app.world_mut().resource_mut::<ArenaInput>().human = ActorIntent {
            movement: Vec2::ONE,
            jump: true,
            high_jump: true,
            glider_toggle: true,
            flight_toggle: true,
            boat_toggle: true,
            flight_vertical: 1.0,
            cast_pressed: true,
            cast_released: true,
            ..Default::default()
        };
        app.world_mut().run_schedule(ArenaTick);
        let session = app.world().resource::<ArenaSession>();
        let player = session.actors.first().expect("explorer");
        assert_eq!(session.outcome, Some(ArenaOutcome::Draw));
        assert_eq!(session.ocean_time(), death_clock);
        assert_eq!(player.feet, underwater);
        assert_eq!(player.hp, 0.0);
        assert_eq!(player.swimming().expect("swim").oxygen_seconds, 0.0);
        assert!(session.projectiles.is_empty());
        assert_eq!(*app.world().resource::<OceanSimulationTime>(), death_clock);
    }
    // Restart uses the production generation path, including clearing queued edges.
    app.world_mut().resource_mut::<ArenaInput>().human = ActorIntent {
        jump: true,
        high_jump: true,
        glider_toggle: true,
        flight_toggle: true,
        boat_toggle: true,
        cast_pressed: true,
        cast_released: true,
        ..Default::default()
    };
    app.world_mut().resource_mut::<ArenaReset>().generation += 1;
    app.world_mut().run_schedule(ArenaTick);
    let session = app.world().resource::<ArenaSession>();
    assert!(!session.is_finished());
    assert_eq!(session.outcome, None);
    assert_eq!(session.actors.len(), 1);
    let player = session.actors.first().expect("explorer");
    assert!((player.feet - spawn).length() < 0.001);
    assert_eq!(player.hp, 100.0);
    let swim = player.swimming().expect("swim");
    assert_eq!(swim.oxygen_seconds, OXYGEN);
    assert!(!swim.submerged && !swim.active);
    assert!(!player.boat().expect("boat").active);
    assert!(!player.glider().expect("glider").open);
    assert!(!player.free_flight().expect("flight").active);
    assert_eq!(session.ocean_time().generation, death_clock.generation + 1);
    assert!(session.ocean_time().seconds < death_clock.seconds);
    assert_eq!(
        *app.world().resource::<OceanSimulationTime>(),
        session.ocean_time()
    );
}

#[derive(Debug)]
struct FlatSea;
impl OceanEnvironmentSampler for FlatSea {
    fn surface_at(
        &self,
        _xz: Vec2,
        phase: f32,
        column: OceanWaterColumn,
    ) -> Option<OceanSurfaceSample> {
        Some(OceanSurfaceSample {
            height: column.mean_height + 0.2 * phase.sin(),
            normal: Vec3::Y,
            vertical_velocity: 0.2 * phase.cos(),
            mean_height: column.mean_height,
            bed_height: column.bed_height,
            water_id: column.water_id,
        })
    }
}

fn fixture() -> (
    Actor,
    CollisionWorld,
    ArenaTerrainView,
    ArenaVoxelGeometry,
    OceanEnvironmentView,
) {
    let geometry = ArenaVoxelGeometry {
        level_height: 1.0,
        radius: 100,
        min_level: -30,
        max_level: 100,
        ..Default::default()
    };
    let chunks: BTreeSet<_> = (-3..=3)
        .flat_map(|q| (-3..=3).map(move |r| (q, r)))
        .collect();
    let mut terrain = ArenaTerrainView {
        revision: 1,
        selection: ArenaSelection {
            map: ArenaMap::NorthernArchipelago,
            ..Default::default()
        },
        residency: Some(ArenaResidency {
            catalogue: chunks.clone(),
            ready: chunks,
        }),
        liquids: HexCoord::ORIGIN
            .within_radius(35)
            .into_iter()
            .map(|coord| ArenaSolidSpan {
                bottom: TilePos::new(coord, -20),
                top_level: 0,
                substance: SubstanceId(2),
            })
            .collect(),
        ..Default::default()
    };
    terrain.liquids.sort_by_key(|span| span.bottom);
    let mut world = CollisionWorld::default();
    world.refresh(&terrain, geometry);
    let mut actor = Actor::spawn(0, Vec3::Y * DECK, Vec3::X);
    actor.configure_expedition_player();
    actor.free_flight = Some(Default::default());
    actor.marine = Some(Default::default());
    let environment = OceanEnvironmentView {
        package_fingerprint: 7,
        sampler: Arc::new(FlatSea),
        wind: OceanWindProfile::default(),
    };
    (actor, world, terrain, geometry, environment)
}

fn context<'a>(
    terrain: &'a ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    environment: &'a OceanEnvironmentView,
) -> MarineWorld<'a> {
    MarineWorld {
        terrain,
        geometry,
        environment: Some(environment),
        time: OceanSimulationTime::default(),
    }
}

fn toggle() -> ActorIntent {
    ActorIntent {
        boat_toggle: true,
        ..Default::default()
    }
}

#[test]
fn water_lab_floats_two_thirds_deep_and_toggles_boat_without_ratchet() {
    #[derive(Debug)]
    struct SteppedSea;
    impl OceanEnvironmentSampler for SteppedSea {
        fn surface_at(
            &self,
            _at: Vec2,
            phase: f32,
            column: OceanWaterColumn,
        ) -> Option<OceanSurfaceSample> {
            Some(OceanSurfaceSample {
                height: column.mean_height + 0.4 * phase.sin().round(),
                normal: Vec3::Y,
                vertical_velocity: 0.0,
                mean_height: column.mean_height,
                bed_height: column.bed_height,
                water_id: column.water_id,
            })
        }
    }
    let mut app = session_app();
    {
        let mut terrain = app.world_mut().resource_mut::<ArenaTerrainView>();
        terrain.selection.map = ArenaMap::WaterLab;
        terrain.package_identity = None;
        terrain.anchors.insert("water_lab_swim".into(), Vec3::ZERO);
    }
    {
        let mut environment = app.world_mut().resource_mut::<OceanEnvironmentView>();
        environment.package_fingerprint = hex_core::water_lab::WATER_LAB_ID;
        environment.sampler = Arc::new(SteppedSea);
        environment.wind.speed = 0.0;
    }
    app.world_mut().resource_mut::<ArenaReset>().generation += 1;
    app.world_mut().run_schedule(ArenaTick);
    let terrain = app.world().resource::<ArenaTerrainView>().clone();
    let geometry = *app.world().resource::<ArenaVoxelGeometry>();
    assert!(app
        .world_mut()
        .resource_mut::<ArenaSession>()
        .reset_water_lab_pose(hex_core::water_lab::LabStart::Swim, &terrain, geometry));
    for _ in 0..120 {
        app.world_mut().run_schedule(ArenaTick);
    }
    for _ in 0..20 {
        let actor = app
            .world()
            .resource::<ArenaSession>()
            .actors
            .first()
            .unwrap();
        let surface = 0.4
            * app
                .world()
                .resource::<OceanSimulationTime>()
                .phase_seconds()
                .sin()
                .round();
        assert!((actor.feet.y + actor.dimensions.y * (2.0 / 3.0) - surface).abs() < 0.401);
        app.world_mut().resource_mut::<ArenaInput>().human = toggle();
        app.world_mut().run_schedule(ArenaTick);
        assert!(
            app.world()
                .resource::<ArenaSession>()
                .actors
                .first()
                .unwrap()
                .boat()
                .unwrap()
                .active
        );
        app.world_mut().resource_mut::<ArenaInput>().human = toggle();
        app.world_mut().run_schedule(ArenaTick);
        for _ in 0..120 {
            app.world_mut().run_schedule(ArenaTick);
        }
        let actor = app
            .world()
            .resource::<ArenaSession>()
            .actors
            .first()
            .unwrap();
        assert!(!actor.boat().unwrap().active);
        let surface = 0.4
            * app
                .world()
                .resource::<OceanSimulationTime>()
                .phase_seconds()
                .sin()
                .round();
        assert!((actor.feet.y + 0.8 - surface).abs() < 0.401);
        assert!(body_velocity(actor).length() < 0.001);
    }
}

#[test]
fn water_lab_swimming_speed_is_below_walk_and_wind_scale_is_glider_only() {
    let (mut actor, world, terrain, geometry, environment) = fixture();
    let sea = context(&terrain, geometry, &environment);
    actor.marine.as_mut().unwrap().lab = true;
    actor.marine.as_mut().unwrap().glider_wind_scale = 0.45;
    actor.feet.y = -0.8;
    for _ in 0..360 {
        tick_or_wait(
            &mut actor,
            ActorIntent {
                movement: Vec2::Y,
                ..Default::default()
            },
            &sea,
            &world,
        );
    }
    assert!((body_velocity(&actor).with_y(0.0).length() - actor.walking_speed * 0.85).abs() < 0.01);
    prepare(&mut actor, ActorIntent::default(), &sea, &world);
    assert!((actor.glider.wind.length() / sea.wind(actor.feet).length() - 0.45).abs() < 0.00001);
    assert!(prepare(&mut actor, toggle(), &sea, &world).is_none());
    assert!((actor.boat().unwrap().wind.length() - sea.wind(actor.feet).length()).abs() < 0.00001);
}

#[test]
fn shore_step_is_not_replayed_by_boat_swimming_or_unloaded_water_ticks() {
    let (mut stepped, world, terrain, geometry, environment) = fixture();
    let mut shore = ArenaTerrainView {
        voxels: HexCoord::ORIGIN
            .within_radius(3)
            .into_iter()
            .map(|coord| (TilePos::new(coord, 0), SubstanceId(1)))
            .collect(),
        ..Default::default()
    };
    shore
        .voxels
        .insert(TilePos::new(HexCoord::from_axial(1, 0), 1), SubstanceId(1));
    let mut shore_world = CollisionWorld::default();
    shore_world.refresh(
        &shore,
        ArenaVoxelGeometry {
            level_height: 0.35,
            ..Default::default()
        },
    );
    stepped.feet = Vec3::ZERO;
    for _ in 0..90 {
        stepped.body.tick_profile(
            &mut stepped.feet,
            Vec3::X,
            false,
            false,
            &shore_world,
            crate::controller::GroundProfile {
                height: stepped.dimensions.y,
                radius: stepped.dimensions.x * 0.5,
                ..Default::default()
            },
        );
        if stepped.step_rise_this_tick() > 0.3 {
            break;
        }
    }
    assert!((stepped.step_rise_this_tick() - 0.35).abs() < 0.001);

    // Preserve the actual controller event while placing each transition at
    // its relevant ocean fixture. No test invents a step_rise value.
    for mode in ["boat", "swimming", "unloaded"] {
        let mut actor = stepped.clone();
        actor.feet = Vec3::Y * if mode == "boat" { DECK } else { -2.0 };
        actor.previous_feet = actor.feet;
        actor.grounded = false;
        actor.body.grounded = false;
        let mut admitted = terrain.clone();
        if mode == "unloaded" {
            admitted
                .residency
                .as_mut()
                .expect("streamed ocean")
                .ready
                .clear();
        }
        let sea = context(&admitted, geometry, &environment);
        if mode == "boat" {
            assert!(prepare(&mut actor, toggle(), &sea, &world).is_none());
            assert!(actor.boat().expect("boat").active);
        }
        for _ in 0..3 {
            assert!(tick_or_wait(
                &mut actor,
                ActorIntent::default(),
                &sea,
                &world
            ));
            assert!(
                actor.step_rise_this_tick().abs() < f32::EPSILON,
                "{mode} replayed the ground step"
            );
        }
        if mode == "swimming" {
            assert!(actor.swimming().expect("swim").active);
        } else if mode == "unloaded" {
            assert!(actor.free_flight().expect("flight").loading);
            assert!((actor.feet.y + 2.0).abs() < f32::EPSILON);
        }
    }
}

#[test]
fn transition_boat_deployment_cannot_reverse_motion_toward_the_camera() {
    let (mut actor, world, terrain, geometry, environment) = fixture();
    let sea = context(&terrain, geometry, &environment);
    actor.aim = Vec3::NEG_X;
    actor.body.control_velocity = Vec3::X * 12.0;
    assert!(prepare(&mut actor, toggle(), &sea, &world).is_none());
    assert!(actor.boat().expect("boat").heading.dot(Vec3::X) > 0.999);
    boat_tick(
        &mut actor,
        ActorIntent {
            movement: Vec2::Y,
            ..Default::default()
        },
        &sea,
        &world,
    );
    assert!(actor.boat().expect("boat").velocity.x > 11.0);
    assert!(actor.boat().expect("boat").heading.dot(Vec3::X) > 0.999);
}

#[test]
fn transition_cooldown_rejected_high_jump_cannot_bypass_swimming_drag() {
    let mut velocities = Vec::new();
    for press in [false, true] {
        let (mut actor, _, terrain, geometry, environment) = fixture();
        actor.feet.y = -5.0;
        actor.body.vertical_velocity = 10.0;
        let state = actor.marine.as_mut().expect("marine");
        state.swim.active = true;
        state.velocity = Vec3::Y * 10.0;
        if let Some(cooldown) = actor.cooldowns.get_mut(crate::Spell::HighJump.index()) {
            *cooldown = 1.0;
        }
        let mut session = ArenaSession::default();
        session.reset_with_setup(1, &terrain, geometry, &ArenaBattleSetup::default());
        session.actors = vec![actor];
        session.encounter.initialized = true;
        session.ocean_environment = Some(environment);
        let materials = hex_core::arena::ArenaMaterials {
            stone: SubstanceId(1),
            grass: SubstanceId(3),
            dirt: SubstanceId(4),
            bedrock: SubstanceId(5),
            fire: hex_core::ElementId(1),
        };
        session.advance(
            ActorIntent {
                high_jump: press,
                ..Default::default()
            },
            &terrain,
            geometry,
            materials,
            &ArenaTuning::default(),
        );
        velocities.push(
            session
                .actors
                .first()
                .expect("player")
                .body
                .vertical_velocity,
        );
    }
    assert!(velocities.iter().all(|velocity| *velocity < 9.9));
    assert!(velocities
        .first()
        .zip(velocities.last())
        .is_some_and(|(normal, rejected)| (normal - rejected).abs() < 0.0001));
}

#[test]
fn portable_boat_toggle_preserves_speed_and_never_ratchets_altitude() {
    let (mut actor, world, terrain, geometry, environment) = fixture();
    let sea = context(&terrain, geometry, &environment);
    actor.body.control_velocity = Vec3::X * 5.0;
    let feet = actor.feet;
    for _ in 0..20 {
        assert!(prepare(&mut actor, toggle(), &sea, &world).is_none());
        assert!(actor.boat().is_some_and(|boat| boat.active));
        assert!((actor.boat().expect("boat").velocity.length() - 5.0).abs() < 0.0001);
        assert!(prepare(&mut actor, toggle(), &sea, &world).is_none());
        assert!(!actor.boat().expect("boat").active);
        assert!(body_velocity(&actor).distance(Vec3::X * 5.0) < 0.0001);
        assert!(actor.feet.distance(feet) < 0.0001);
    }
}

#[test]
fn sail_accelerates_downwind_and_across_but_coasts_into_headwind() {
    let base = BoatSnapshot {
        active: true,
        heading: Vec3::X,
        velocity: Vec3::X * 12.0,
        ..Default::default()
    };
    let velocities = [Vec3::X * 10.0, Vec3::Z * 10.0, Vec3::NEG_X * 10.0].map(|wind| {
        boat_velocity(
            BoatSnapshot { wind, ..base },
            ActorIntent::default(),
            Vec3::X,
        )
        .1
        .length()
    });
    let [aligned, across, against] = velocities;
    assert!(aligned > 12.0 && across > aligned && against < 12.0);
    let slow = BoatSnapshot {
        velocity: Vec3::ZERO,
        wind: Vec3::NEG_X * 10.0,
        ..base
    };
    assert!(
        boat_velocity(
            slow,
            ActorIntent {
                movement: Vec2::Y,
                ..Default::default()
            },
            Vec3::X
        )
        .1
        .length()
            > 0.0
    );
}

#[test]
fn full_hull_and_player_stop_at_solid_walls() {
    let (mut actor, mut world, mut terrain, geometry, environment) = fixture();
    for coord in HexCoord::from_axial(3, 0).within_radius(1) {
        for level in -1..=4 {
            terrain
                .voxels
                .insert(TilePos::new(coord, level), SubstanceId(1));
        }
    }
    terrain.revision += 1;
    world.refresh(&terrain, geometry);
    let sea = context(&terrain, geometry, &environment);
    assert!(prepare(&mut actor, toggle(), &sea, &world).is_none());
    actor.marine.as_mut().expect("marine").boat.velocity = Vec3::X * 24.0;
    for _ in 0..180 {
        boat_tick(&mut actor, ActorIntent::default(), &sea, &world);
    }
    assert!(boat_clear(
        &world,
        actor.feet,
        actor.boat().expect("boat").heading,
        &actor
    ));
    assert!(actor.feet.x < HexCoord::from_axial(3, 0).to_world(0.0).x);
}

#[test]
fn unloaded_water_holds_last_pose_and_preserves_requested_prefetch() {
    let (mut actor, mut world, mut terrain, geometry, environment) = fixture();
    assert!(prepare(
        &mut actor,
        toggle(),
        &context(&terrain, geometry, &environment),
        &world
    )
    .is_none());
    actor.marine.as_mut().expect("marine").boat.velocity = Vec3::X * 12.0;
    let feet = actor.feet;
    terrain.residency.as_mut().expect("residency").ready.clear();
    terrain.revision += 1;
    world.refresh(&terrain, geometry);
    boat_tick(
        &mut actor,
        ActorIntent::default(),
        &context(&terrain, geometry, &environment),
        &world,
    );
    assert!(actor.feet.distance(feet) < 0.0001);
    assert!(actor.free_flight().expect("flight").loading);
    assert!(
        actor
            .free_flight
            .as_ref()
            .expect("flight")
            .requested
            .length()
            > 10.0
    );
}

#[test]
fn oxygen_uses_physical_eye_then_refills_without_healing() {
    let (mut actor, _, terrain, geometry, environment) = fixture();
    let sea = context(&terrain, geometry, &environment);
    actor.feet.y = -2.0;
    actor.marine.as_mut().expect("marine").swim.oxygen_seconds = 0.01;
    for _ in 0..3 {
        let sample = sea.sample(actor.feet);
        breathing(&mut actor, sample);
    }
    assert!(actor.swimming().expect("swim").submerged);
    assert!(actor.hp < 100.0);
    let hp = actor.hp;
    actor.feet.y = -0.8; // feet remain wet, the physical eye is above the surface.
    for _ in 0..720 {
        let sample = sea.sample(actor.feet);
        breathing(&mut actor, sample);
    }
    assert!(!actor.swimming().expect("swim").submerged);
    assert!((actor.swimming().expect("swim").oxygen_seconds - 90.0).abs() < 0.001);
    assert!((actor.hp - hp).abs() < 0.0001);
}

#[test]
fn swim_vertical_controls_and_high_jump_keep_actual_body_collision() {
    let (mut actor, world, terrain, geometry, environment) = fixture();
    let sea = context(&terrain, geometry, &environment);
    actor.feet.y = -5.0;
    for _ in 0..120 {
        assert!(tick_or_wait(
            &mut actor,
            ActorIntent {
                flight_vertical: 1.0,
                ..Default::default()
            },
            &sea,
            &world
        ));
    }
    let upper = actor.feet.y;
    assert!(upper > -4.0);
    for _ in 0..180 {
        assert!(tick_or_wait(
            &mut actor,
            ActorIntent {
                flight_vertical: -1.0,
                ..Default::default()
            },
            &sea,
            &world
        ));
    }
    assert!(actor.feet.y < upper - 1.0);
    assert!(actor.high_jump(&ArenaTuning::default()));
    assert!(tick_or_wait(
        &mut actor,
        ActorIntent {
            high_jump: true,
            ..Default::default()
        },
        &sea,
        &world
    ));
    assert!(body_velocity(&actor).y > 2.5);
}

#[test]
fn pause_preserves_marine_state_and_reset_or_legacy_map_removes_it() {
    let (mut actor, world, terrain, geometry, environment) = fixture();
    let sea = context(&terrain, geometry, &environment);
    assert!(prepare(&mut actor, toggle(), &sea, &world).is_none());
    actor.marine.as_mut().expect("marine").swim.oxygen_seconds = 42.0;
    let boat = actor.boat();
    let swim = actor.swimming();
    actor.cancel_charge(); // Pause's command must not clear travel or reserve.
    assert_eq!(actor.boat(), boat);
    assert_eq!(actor.swimming(), swim);
    let mut session = ArenaSession {
        actors: vec![actor],
        ..Default::default()
    };
    session.reset_with_setup(8, &terrain, geometry, &ArenaBattleSetup::default());
    let restarted = session.actors.first().expect("explorer");
    assert!(!restarted.boat().expect("boat").active);
    assert!((restarted.swimming().expect("swim").oxygen_seconds - 90.0).abs() < 0.0001);
    assert!(session.ocean_time().seconds.abs() < f64::EPSILON);
    let forest = ArenaTerrainView {
        selection: ArenaSelection {
            map: ArenaMap::ForestMassif,
            ..Default::default()
        },
        ..terrain
    };
    session.reset_with_setup(9, &forest, geometry, &ArenaBattleSetup::default());
    assert!(session
        .actors
        .iter()
        .all(|actor| actor.boat().is_none() && actor.swimming().is_none()));
}

#[test]
fn lab_space_and_hands_free_follow_identical_extreme_steps_and_release_dive() {
    #[derive(Debug)]
    struct Storm;
    impl OceanEnvironmentSampler for Storm {
        fn surface_at(
            &self,
            _: Vec2,
            phase: f32,
            column: OceanWaterColumn,
        ) -> Option<OceanSurfaceSample> {
            Some(OceanSurfaceSample {
                height: column.mean_height + (phase.sin() * 12.0).round() * 0.4,
                normal: Vec3::Y,
                vertical_velocity: 0.0,
                mean_height: column.mean_height,
                bed_height: column.bed_height,
                water_id: column.water_id,
            })
        }
    }
    let (mut idle, world, terrain, geometry, mut environment) = fixture();
    environment.sampler = Arc::new(Storm);
    idle.feet.y = -0.8;
    idle.marine.as_mut().unwrap().lab = true;
    let mut space = idle.clone();
    for tick in 0..720 {
        let sea = MarineWorld {
            time: OceanSimulationTime::from_fixed_tick(1, tick, f64::from(STEP)),
            ..context(&terrain, geometry, &environment)
        };
        for (actor, vertical) in [(&mut idle, 0.0), (&mut space, 1.0)] {
            let before = actor.feet.y;
            assert!(tick_or_wait(
                actor,
                ActorIntent {
                    flight_vertical: vertical,
                    ..Default::default()
                },
                &sea,
                &world
            ));
            let OceanSurfaceState::ReadyWet(surface) = sea.sample(actor.feet) else {
                panic!("expected sea");
            };
            assert!((actor.feet.y + 0.8 - surface.height).abs() < 0.8);
            assert!((actor.feet.y - before).abs() <= 6.0 * STEP + 0.00001);
        }
        assert!(idle.feet.distance(space.feet) < 0.0001);
    }
    let sea = context(&terrain, geometry, &environment);
    idle.feet.y = -0.8;
    for _ in 0..90 {
        tick_or_wait(
            &mut idle,
            ActorIntent {
                flight_vertical: -1.0,
                ..Default::default()
            },
            &sea,
            &world,
        );
    }
    assert!(idle.eye().y < 0.0, "Ctrl still dives");
    let submerged_y = idle.feet.y;
    tick_or_wait(&mut idle, ActorIntent::default(), &sea, &world);
    assert!(idle.feet.y > submerged_y && idle.feet.y < -0.81);
    for _ in 0..120 {
        tick_or_wait(&mut idle, ActorIntent::default(), &sea, &world);
    }
    // Collision sweeps discard sub-skin displacement near the target.
    assert!((idle.feet.y + 0.8).abs() < 0.002);
}

#[test]
fn lab_boat_uses_current_depth_over_a_temporarily_flooded_shelf() {
    #[derive(Debug)]
    struct Shelf(f32);
    impl OceanEnvironmentSampler for Shelf {
        fn inundation_column_at(&self, _: Vec2) -> Option<OceanWaterColumn> {
            Some(OceanWaterColumn {
                mean_height: 0.0,
                bed_height: 0.4,
                water_id: SubstanceId(2),
            })
        }
        fn surface_at(
            &self,
            _: Vec2,
            _: f32,
            column: OceanWaterColumn,
        ) -> Option<OceanSurfaceSample> {
            Some(OceanSurfaceSample {
                height: self.0,
                normal: Vec3::Y,
                vertical_velocity: 0.0,
                mean_height: column.mean_height,
                bed_height: column.bed_height,
                water_id: column.water_id,
            })
        }
    }
    let (mut actor, mut world, mut terrain, geometry, mut env) = fixture();
    terrain.liquids.clear();
    let geometry = ArenaVoxelGeometry {
        level_height: 0.4,
        ..geometry
    };
    terrain.voxels = HexCoord::ORIGIN
        .within_radius(3)
        .into_iter()
        .map(|coord| (TilePos::new(coord, 1), SubstanceId(1)))
        .collect();
    terrain.revision += 1;
    world.refresh(&terrain, geometry);
    actor.feet.y = 1.2;
    actor.marine.as_mut().unwrap().lab = true;
    env.sampler = Arc::new(Shelf(2.0));
    let sea = context(&terrain, geometry, &env);
    assert!(prepare(&mut actor, toggle(), &sea, &world).is_none());
    assert!(actor.boat().unwrap().active);
    assert!(prepare(&mut actor, toggle(), &sea, &world).is_none());
    env.sampler = Arc::new(Shelf(0.5));
    actor.feet.y = 0.4;
    assert!(prepare(
        &mut actor,
        toggle(),
        &context(&terrain, geometry, &env),
        &world
    )
    .is_some());
    assert!(!actor.boat().unwrap().active);
}

#[test]
fn lab_boat_follows_steps_gradually_and_keeps_horizontal_sailing() {
    #[derive(Debug)]
    struct StepSea(f32);
    impl OceanEnvironmentSampler for StepSea {
        fn surface_at(
            &self,
            _: Vec2,
            _: f32,
            column: OceanWaterColumn,
        ) -> Option<OceanSurfaceSample> {
            Some(OceanSurfaceSample {
                height: column.mean_height + self.0,
                normal: Vec3::Y,
                vertical_velocity: 0.0,
                mean_height: column.mean_height,
                bed_height: column.bed_height,
                water_id: column.water_id,
            })
        }
    }
    let (mut actor, world, terrain, geometry, mut environment) = fixture();
    actor.marine.as_mut().unwrap().lab = true;
    assert!(prepare(
        &mut actor,
        toggle(),
        &context(&terrain, geometry, &environment),
        &world
    )
    .is_none());
    for height in [0.4, -0.4, 1.2, -1.2] {
        environment.sampler = Arc::new(StepSea(height));
        let sea = context(&terrain, geometry, &environment);
        let initial = actor.feet.y;
        for tick in 0..120 {
            let before = actor.feet;
            let before_boat = actor.boat().unwrap();
            let expected = boat_velocity(
                BoatSnapshot {
                    wind: sea.wind(before),
                    ..before_boat
                },
                ActorIntent::default(),
                actor.aim,
            )
            .1;
            boat_tick(&mut actor, ActorIntent::default(), &sea, &world);
            let delta = actor.feet - before;
            assert!(delta.y.abs() <= 6.0 * STEP + 0.00001);
            assert!((delta.with_y(0.0) - expected.with_y(0.0) * STEP).length() < 0.0001);
            assert!(
                (actor.feet.y - height - DECK).abs() <= (before.y - height - DECK).abs() + 0.00001
            );
            if tick == 0 {
                assert!((actor.feet.y - initial).abs() < (height + DECK - initial).abs() * 0.5);
                let mut folded = actor.clone();
                fold_boat(&mut folded);
                assert!(folded.body.vertical_velocity.abs() < 0.0001);
                assert!(
                    (body_velocity(&folded).with_y(0.0) - expected.with_y(0.0)).length() < 0.0001
                );
            }
        }
        assert!((actor.feet.y - height - DECK).abs() < 0.002);
    }
}

#[test]
fn lab_buoyancy_eases_speed_at_a_step_and_reversal() {
    let mut height = 0.0;
    let mut speed = 0.0;
    for target in [0.4, -0.4] {
        for tick in 0..120 {
            let (rise, next_speed) = buoyancy_step(height, target, speed);
            assert!(
                (next_speed - speed).abs() < 1.6,
                "no instant velocity replacement"
            );
            if tick == 0 {
                assert!(rise.abs() < 0.01, "ease into a new voxel height");
            }
            height += rise;
            speed = next_speed;
        }
        assert!((height - target).abs() < 0.002);
        assert!(speed.abs() < 0.002);
    }
}

#[test]
fn lab_wave_pushes_downhill_on_rise_and_fall_without_driving_the_sail() {
    #[derive(Debug)]
    struct SlopingWave {
        slope: Vec2,
        rise: f32,
        dry_positive_x: bool,
    }
    impl OceanEnvironmentSampler for SlopingWave {
        fn surface_at(
            &self,
            at: Vec2,
            time: f32,
            column: OceanWaterColumn,
        ) -> Option<OceanSurfaceSample> {
            if self.dry_positive_x && at.x > 1.0 {
                return None;
            }
            Some(OceanSurfaceSample {
                height: column.mean_height + self.slope.dot(at) + self.rise * time,
                normal: Vec3::Y,
                vertical_velocity: 0.0,
                mean_height: column.mean_height,
                bed_height: column.bed_height,
                water_id: column.water_id,
            })
        }
    }
    for boat in [false, true] {
        for rise in [-0.8, 0.0, 0.8] {
            for slope in [Vec2::X * 0.4, -Vec2::Y * 0.4, Vec2::ZERO] {
                let (mut actor, world, terrain, geometry, mut environment) = fixture();
                actor.marine.as_mut().unwrap().lab = true;
                environment.wind.speed = 0.0;
                environment.sampler = Arc::new(SlopingWave {
                    slope,
                    rise,
                    dry_positive_x: false,
                });
                let sea = context(&terrain, geometry, &environment);
                if boat {
                    assert!(prepare(&mut actor, toggle(), &sea, &world).is_none());
                } else {
                    actor.feet.y = -0.8;
                    tick_or_wait(&mut actor, ActorIntent::default(), &sea, &world);
                }
                let start = actor.feet;
                for tick in 1..61 {
                    let sea = MarineWorld {
                        time: OceanSimulationTime::from_fixed_tick(1, tick, f64::from(STEP)),
                        ..context(&terrain, geometry, &environment)
                    };
                    assert!(tick_or_wait(
                        &mut actor,
                        ActorIntent::default(),
                        &sea,
                        &world
                    ));
                    let state = actor.marine.as_ref().unwrap();
                    assert!(state.wave_velocity.length() <= 0.80001);
                    if boat {
                        assert!(
                            (state.boat.velocity.with_y(0.0) - state.wave_velocity).length()
                                < 0.0001,
                            "wave drift must not become sailing speed on the next tick"
                        );
                    }
                }
                let displacement = (actor.feet - start).with_y(0.0);
                if rise.abs() > 0.01 && slope.length() > 0.01 {
                    let downhill = -Vec3::new(slope.x, 0.0, slope.y);
                    assert!(
                        displacement.dot(downhill) > 0.005,
                        "push follows the downhill gradient"
                    );
                } else {
                    assert!(
                        displacement.length() < 0.0001,
                        "still/flat water gives no push"
                    );
                }
            }
        }
    }
    let (mut actor, _, terrain, geometry, mut environment) = fixture();
    environment.sampler = Arc::new(SlopingWave {
        slope: Vec2::X,
        rise: 1.0,
        dry_positive_x: true,
    });
    let state = actor.marine.as_mut().unwrap();
    state.lab = true;
    state.wave_height = Some(-0.4);
    let (push, _) = context(&terrain, geometry, &environment).wave_motion(Vec3::ZERO, 0.0, state);
    assert!(
        push.length() < 0.0001,
        "missing water cannot become a downhill cliff"
    );
}

#[test]
fn reaching_and_close_hauled_drive_are_symmetric_and_gradual() {
    let speed_after = |angle: f32| {
        let angle = angle.to_radians();
        let heading = Vec3::new(-angle.cos(), 0.0, angle.sin());
        let mut boat = BoatSnapshot {
            active: true,
            heading,
            velocity: heading * 5.0,
            wind: Vec3::X * 9.0,
            ..Default::default()
        };
        for _ in 0..1200 {
            let (heading, velocity) = boat_velocity(boat, ActorIntent::default(), heading);
            boat.heading = heading;
            boat.velocity = velocity;
        }
        boat.velocity.length()
    };
    let speeds = [30.0, 45.0, 55.0, 65.0, 90.0, 120.0, 180.0].map(speed_after);
    let [head, edge, close, reach, beam, broad, run] = speeds;
    assert!(head < 5.0 && edge > head && close > 5.0);
    assert!(close < reach && reach < beam && broad > run && beam > broad);
    assert!((speed_after(60.0) - speed_after(-60.0)).abs() < 0.0001);
    assert!((speed_after(89.9) - speed_after(90.1)).abs() < 0.01);
    assert!((speed_after(39.9) - speed_after(40.1)).abs() < 0.01);
}

#[derive(Debug)]
struct PositionWind;
impl OceanEnvironmentSampler for PositionWind {
    fn wind_at(
        &self,
        position: Vec3,
        time: OceanSimulationTime,
        profile: OceanWindProfile,
    ) -> Vec2 {
        profile.velocity_at(time) + Vec2::new(position.x, position.y) * 0.1
    }
    fn surface_at(
        &self,
        at: Vec2,
        phase: f32,
        column: OceanWaterColumn,
    ) -> Option<OceanSurfaceSample> {
        FlatSea.surface_at(at, phase, column)
    }
}

#[test]
fn spatial_wind_reaches_boat_and_glider_with_accepted_multiplier() {
    for x in [0.0, 8.0] {
        let (mut actor, world, terrain, geometry, mut environment) = fixture();
        environment.sampler = Arc::new(PositionWind);
        let sea = context(&terrain, geometry, &environment);
        actor.marine.as_mut().expect("marine").lab = true;
        actor.marine.as_mut().expect("marine").glider_wind_scale = 0.65;
        actor.feet = Vec3::new(x, -0.8, 0.0);
        let wind = sea.wind(actor.feet);
        prepare(&mut actor, ActorIntent::default(), &sea, &world);
        assert!((actor.glider.wind - wind * 0.65).length() < 0.00001);
        assert!(prepare(&mut actor, toggle(), &sea, &world).is_none());
        // Deployment samples at the pre-transition feet, before lifting onto the deck.
        assert!((actor.boat().expect("boat").wind - wind).length() < 0.00001);
    }
}

#[test]
fn summit_glide_requires_admitted_clear_terrain_and_preserves_player() {
    let mut app = session_app();
    let mut terrain = app.world().resource::<ArenaTerrainView>().clone();
    let geometry = *app.world().resource::<ArenaVoxelGeometry>();
    let mut session = app.world_mut().resource_mut::<ArenaSession>();
    let actor = session.actors.first().expect("player");
    let id = actor.id;
    let hp = actor.hp;
    let feet = actor.feet + Vec3::Y * 6.0;
    let old = actor.feet;
    let residency = terrain.residency.as_mut().expect("streamed fixture");
    let chunks = std::mem::take(&mut residency.ready);
    terrain.revision += 1;
    assert!(!session.start_exploration_glide(feet, Vec3::X, &terrain, geometry));
    assert_eq!(session.actors.first().expect("player").feet, old);
    terrain.residency.as_mut().expect("residency").ready = chunks;
    terrain.revision += 1;
    assert!(session.start_exploration_glide(feet, Vec3::X, &terrain, geometry));
    let actor = session.actors.first().expect("player");
    assert_eq!(actor.id, id);
    assert_eq!(actor.hp, hp);
    assert_eq!(actor.feet, feet);
    assert!(actor.glider.open);
    assert_eq!(actor.glider.snapshot().velocity, Vec3::X * 12.0);
}
