use super::*;
use crate::{ArenaBattleSetup, ArenaTuning};
use hex_core::arena::{ArenaMap, ArenaResidency, ArenaSelection, ArenaSolidSpan};
use hex_core::ocean::{OceanEnvironmentSampler, OceanSurfaceSample, OceanWindProfile};
use hex_core::SubstanceId;
use std::{collections::BTreeSet, sync::Arc};

#[test]
fn fixed_schedule_consumes_boat_edges_casts_aboard_and_publishes_one_clock() {
    use crate::{plugin, ArenaInput, ArenaSession};
    use hex_core::arena::{ArenaMaterials, ArenaPackageIdentity, ArenaReset, ArenaTick};
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
    assert!((player.feet.y - visible_surface - DECK).abs() < 0.0001);
    assert!((player.eye().y - visible_surface - DECK - 1.02).abs() < 0.0001);
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
fn sail_accelerates_with_wind_and_loses_momentum_across_or_against_it() {
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
    assert!(aligned > 12.0 && across < 12.0 && against < across);
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
