//! Actual-package traversal through ArenaInput, drive_simulation and ArenaTick.
//! Only independent starts may relocate; every route uses ordinary held walking.
#![expect(
    clippy::expect_used,
    reason = "The explicit integration runner requires complete fixtures and durable diagnostic receipts, including failures."
)]
use super::*;
use hex_core::HexCoord;

const REACHED: f32 = 2.0;
const STALL_TICKS: u64 = 600;
const ROUTE_LIMIT: Duration = Duration::from_secs(180);
const TOTAL_LIMIT: Duration = Duration::from_secs(1200);

type Point = [f32; 3];
struct Route {
    name: String,
    category: &'static str,
    points: Vec<Point>,
    stacked: bool,
}

#[derive(Clone, Copy, Serialize)]
struct GroundedWalkLimits {
    voxel_height: f32,
    automatic_step_height: f32,
    gravity: f32,
    ground_snap_distance: f32,
    collision_skin: f32,
    tick_seconds: f32,
    maximum_unsupported_descent: f32,
    maximum_airborne_ticks: u64,
}
impl GroundedWalkLimits {
    fn new(voxel_height: f32) -> Self {
        let contract = hex_arena::ground_motion_contract();
        // An ordinary downward step is at most one authored voxel, bounded by
        // the controller's automatic step contract. Only physical contact/snap
        // tolerance is added, never a route-specific drop allowance.
        let maximum_unsupported_descent = voxel_height.min(contract.automatic_step_height)
            + contract.ground_snap_distance
            + contract.collision_skin;
        let fall_seconds =
            (2.0 * f64::from(maximum_unsupported_descent) / f64::from(contract.gravity)).sqrt();
        // The controller starts falling on the tick after walking off support.
        // Round ballistic landing upward to a simulation tick, then allow that
        // one departure/classification tick. Streaming pauses add no ticks.
        let maximum_airborne_ticks = u64::try_from(
            Duration::from_secs_f64(fall_seconds)
                .as_nanos()
                .div_ceil(Duration::from_secs_f32(hex_arena::STEP).as_nanos()),
        )
        .expect("bounded one-voxel fall duration")
            + 1;
        Self {
            voxel_height,
            automatic_step_height: contract.automatic_step_height,
            gravity: contract.gravity,
            ground_snap_distance: contract.ground_snap_distance,
            collision_skin: contract.collision_skin,
            tick_seconds: hex_arena::STEP,
            maximum_unsupported_descent,
            maximum_airborne_ticks,
        }
    }
}

#[derive(Resource, Serialize)]
struct GroundedWalkTrace {
    contract: &'static str,
    limits: GroundedWalkLimits,
    observed_simulation_ticks: u64,
    total_airborne_ticks: u64,
    maximum_airborne_ticks: u64,
    maximum_unsupported_descent: f32,
    completed_airborne_episodes: u64,
    airborne_at_end: bool,
    failure: Option<&'static str>,
    #[serde(skip)]
    last_tick: u64,
    #[serde(skip)]
    previous_height: f32,
    #[serde(skip)]
    departure_height: f32,
    #[serde(skip)]
    airborne_ticks: u64,
}
impl GroundedWalkTrace {
    fn new(limits: GroundedWalkLimits, tick: u64, feet: Vec3) -> Self {
        Self {
            contract: "one-voxel-grounded-walk-v1",
            limits,
            observed_simulation_ticks: 0,
            total_airborne_ticks: 0,
            maximum_airborne_ticks: 0,
            maximum_unsupported_descent: 0.0,
            completed_airborne_episodes: 0,
            airborne_at_end: false,
            failure: None,
            last_tick: tick,
            previous_height: feet.y,
            departure_height: feet.y,
            airborne_ticks: 0,
        }
    }

    fn observe(&mut self, tick: u64, feet: Vec3, grounded: bool) {
        if tick == self.last_tick {
            return;
        }
        if tick.checked_sub(self.last_tick) != Some(1) {
            self.failure
                .get_or_insert("walking observer missed a simulation tick");
            return;
        }
        self.last_tick = tick;
        self.observed_simulation_ticks += 1;
        if !feet.is_finite() {
            self.failure.get_or_insert("nonfinite walking pose");
            return;
        }
        if !self.airborne_at_end {
            self.departure_height = self.previous_height;
        }
        self.departure_height = self.departure_height.max(feet.y);
        // Include the landing sample before resetting an episode. Otherwise a
        // final swept collision could hide most of the unsupported descent.
        self.maximum_unsupported_descent = self
            .maximum_unsupported_descent
            .max(self.departure_height - feet.y);
        if !grounded || self.airborne_at_end {
            self.airborne_ticks += 1;
            self.maximum_airborne_ticks = self.maximum_airborne_ticks.max(self.airborne_ticks);
        }
        if !grounded {
            self.total_airborne_ticks += 1;
        } else {
            self.completed_airborne_episodes += u64::from(self.airborne_at_end);
            self.airborne_ticks = 0;
        }
        self.airborne_at_end = !grounded;
        self.previous_height = feet.y;
        if self.maximum_unsupported_descent > self.limits.maximum_unsupported_descent {
            self.failure
                .get_or_insert("unsupported descent exceeded one ordinary voxel step");
        }
        if self.maximum_airborne_ticks > self.limits.maximum_airborne_ticks {
            self.failure
                .get_or_insert("airborne duration exceeded one ordinary voxel step");
        }
    }
}

fn observe_grounded_walk(session: Res<ArenaSession>, mut trace: ResMut<GroundedWalkTrace>) {
    if let Some(player) = session.actors.iter().find(|actor| actor.id == 0) {
        trace.observe(session.tick, player.feet, player.grounded);
    } else {
        trace
            .failure
            .get_or_insert("walking observer lost the human actor");
    }
}

fn install_grounded_walk_observer(app: &mut App) {
    app.add_systems(
        ArenaTick,
        observe_grounded_walk
            .after(hex_core::arena::ArenaSystems::Simulate)
            .run_if(resource_exists::<GroundedWalkTrace>),
    );
}

fn downward_step_fixture(drop_levels: i32) -> App {
    use hex_core::arena::{
        ArenaExpeditionSites, ArenaPackageIdentity, ArenaResidency, ArenaSolidSpan,
    };
    use hex_core::{ElementId, SubstanceId};

    let geometry = ArenaVoxelGeometry {
        level_height: 0.35,
        radius: 16,
        min_level: -16,
        ..default()
    };
    let start = HexCoord::from_axial(-3, 0);
    let chunks = (-2..=2)
        .flat_map(|q| (-2..=2).map(move |r| (q, r)))
        .collect();
    let mut terrain = ArenaTerrainView {
        revision: 1,
        selection: ArenaSelection {
            map: ArenaMap::GrandV4,
            ..default()
        },
        spawns: [
            start.to_world(geometry.top(TilePos::new(start, 0))),
            Vec3::ZERO,
        ],
        residency: Some(ArenaResidency {
            ready: chunks,
            catalogue: (-2..=2)
                .flat_map(|q| (-2..=2).map(move |r| (q, r)))
                .collect(),
        }),
        expedition: Some(ArenaExpeditionSites::default()),
        package_identity: Some(ArenaPackageIdentity {
            world_id: "grounded-walk-oracle-fixture".into(),
            manifest_fingerprint: 1,
            sites_fingerprint: None,
        }),
        ..default()
    };
    for coord in HexCoord::ORIGIN.within_radius(16) {
        terrain.columns.insert(
            coord,
            vec![ArenaSolidSpan {
                bottom: TilePos::new(coord, -16),
                top_level: if coord.to_world(0.0).x < 0.0 {
                    0
                } else {
                    -drop_levels
                },
                substance: SubstanceId(1),
            }],
        );
    }
    let mut app = App::new();
    app.insert_resource(terrain)
        .insert_resource(geometry)
        .insert_resource(ArenaMaterials {
            stone: SubstanceId(1),
            reinforced_stone: None,
            bedrock: SubstanceId(2),
            grass: SubstanceId(3),
            dirt: SubstanceId(4),
            fire: ElementId(1),
        })
        .insert_resource(ArenaReset::default())
        .add_plugins(hex_arena::plugin);
    for _ in 0..20 {
        app.world_mut().run_schedule(ArenaTick);
        app.world_mut().resource_mut::<ArenaSession>().bot_enabled = false;
    }
    assert!(
        human(app.world()).grounded,
        "fixture starts on actual support"
    );
    let tick = app.world().resource::<ArenaSession>().tick;
    let feet = human(app.world()).feet;
    app.insert_resource(GroundedWalkTrace::new(
        GroundedWalkLimits::new(geometry.level_height),
        tick,
        feet,
    ));
    install_grounded_walk_observer(&mut app);
    app
}

fn exercise_downward_step(drop_levels: i32, pause_in_air: bool) -> App {
    let mut app = downward_step_fixture(drop_levels);
    let mut paused = false;
    for _ in 0..360 {
        app.world_mut().resource_mut::<ArenaInput>().human = ActorIntent {
            aim: Vec3::X,
            movement: if human(app.world()).feet.x < 4.0 {
                Vec2::Y
            } else {
                Vec2::ZERO
            },
            ..default()
        };
        app.world_mut().run_schedule(ArenaTick);
        if pause_in_air && !paused && !human(app.world()).grounded {
            let tick = app.world().resource::<ArenaSession>().tick;
            let before =
                serde_json::to_value(app.world().resource::<GroundedWalkTrace>()).expect("trace");
            {
                let mut terrain = app.world_mut().resource_mut::<ArenaTerrainView>();
                terrain
                    .residency
                    .as_mut()
                    .expect("finite fixture")
                    .ready
                    .clear();
                terrain.revision += 1;
                terrain.full_rebuild = true;
            }
            for _ in 0..30 {
                app.world_mut().run_schedule(ArenaTick);
            }
            assert_eq!(
                app.world().resource::<ArenaSession>().tick,
                tick,
                "streaming holds actual simulation"
            );
            assert_eq!(
                serde_json::to_value(app.world().resource::<GroundedWalkTrace>())
                    .expect("paused trace"),
                before
            );
            {
                let mut terrain = app.world_mut().resource_mut::<ArenaTerrainView>();
                let residency = terrain.residency.as_mut().expect("finite fixture");
                residency.ready = residency.catalogue.clone();
                terrain.revision += 1;
                terrain.full_rebuild = true;
            }
            paused = true;
        }
    }
    assert!(
        human(app.world()).feet.x >= 3.9,
        "controller crossed the fixture ledge"
    );
    assert!(
        human(app.world()).grounded,
        "include actual landing and settling"
    );
    assert!(
        !pause_in_air || paused,
        "exercise a real airborne streaming pause"
    );
    app
}

#[test]
fn grounded_walk_accepts_one_voxel_step_and_excludes_streaming_waits() {
    let app = exercise_downward_step(1, true);
    let trace = app.world().resource::<GroundedWalkTrace>();
    assert!(
        trace.failure.is_none(),
        "{trace_failure:?}",
        trace_failure = trace.failure
    );
    assert!(trace.completed_airborne_episodes > 0);
    assert!(trace.maximum_unsupported_descent > 0.3);
    assert!(trace.maximum_unsupported_descent <= trace.limits.maximum_unsupported_descent);
    assert!(trace.maximum_airborne_ticks <= trace.limits.maximum_airborne_ticks);
    assert_eq!(trace.observed_simulation_ticks, 360);
}

#[test]
fn grounded_walk_rejects_a_large_fall_even_when_it_lands_safely() {
    let app = exercise_downward_step(4, false);
    let trace = app.world().resource::<GroundedWalkTrace>();
    assert!(
        trace.failure.is_some(),
        "a dry safe landing must not approve a four-voxel fall"
    );
    assert!(trace.maximum_unsupported_descent > trace.limits.maximum_unsupported_descent);
    assert!(trace.maximum_airborne_ticks > trace.limits.maximum_airborne_ticks);
    assert!(trace.completed_airborne_episodes > 0);
    assert!(!trace.airborne_at_end);
}

#[test]
fn grounded_walk_counts_the_landing_sample_and_endpoint_settling() {
    let mut trace = GroundedWalkTrace::new(GroundedWalkLimits::new(0.35), 0, Vec3::Y);
    trace.observe(1, Vec3::Y * 0.8, false);
    assert!(trace.failure.is_none());
    trace.observe(2, Vec3::Y * 0.3, true);
    assert!(
        trace.failure.is_some(),
        "the landing tick must retain the full fall"
    );
    assert!((trace.maximum_unsupported_descent - 0.7).abs() < 0.00001);
    assert_eq!(trace.maximum_airborne_ticks, 2);
}

fn routes(world: &World) -> Vec<Route> {
    let overview = &world.resource::<StreamedArena>().overview;
    let anchor = |name: &str| Vec3::from_array(*overview.anchors.get(name).expect("review anchor"));
    let mut routes = Vec::new();
    // Deliberately independent cross-country samples: these broad crossings do
    // not follow the producer's graded route centerlines or search terrain.
    let forest = anchor("forest");
    for (name, offset) in [
        ("forest_north", Vec3::NEG_Z * 90.0),
        ("forest_south", Vec3::Z * 90.0),
        ("forest_east", Vec3::X * 90.0),
        ("forest_west", Vec3::NEG_X * 90.0),
    ] {
        routes.push(Route {
            name: name.into(),
            category: "cross_country",
            points: vec![forest.to_array(), (forest + offset).to_array()],
            stacked: false,
        });
    }
    let bank = overview
        .review_cameras
        .get("grand-river-exit")
        .expect("river bank camera");
    let riverbank = Vec3::from_array(bank.eye);
    let across = (Vec3::from_array(bank.target) - riverbank)
        .with_y(0.0)
        .normalize_or(Vec3::X);
    for (name, side) in [
        ("river_bank_escape", across),
        ("river_bank_along", across.cross(Vec3::Y)),
    ] {
        routes.push(Route {
            name: name.into(),
            category: "bank_escape",
            points: vec![riverbank.to_array(), (riverbank + side * 85.0).to_array()],
            stacked: false,
        });
    }
    let bank = overview
        .review_cameras
        .get("grand-valley-lake-bank")
        .expect("valley bank camera");
    let valley = Vec3::from_array(bank.eye);
    routes.push(Route {
        name: "valley_crossing".into(),
        category: "cross_country",
        points: vec![
            valley.to_array(),
            (valley + Vec3::new(-90.0, 0.0, 70.0)).to_array(),
        ],
        stacked: false,
    });
    // Fixed geographical samples span the changed mountain feet. These use
    // published review frames, not a terrain search for favorable paths. The
    // ordinary controller still decides each step; the full-region graph and
    // ground-level review remain separate evidence for broader usability.
    for name in [
        "grand-west-foothill-crossing",
        "grand-west-foothill-uphill",
        "grand-lake-foothill-crossing",
        "grand-lake-foothill-uphill",
    ] {
        let frame = overview
            .review_cameras
            .get(name)
            .expect("world-published foothill review frame");
        routes.push(Route {
            name: name.into(),
            category: "foothill_cross_country",
            points: vec![frame.eye, frame.target],
            stacked: false,
        });
    }
    let view = world.resource::<ArenaTerrainView>();
    let geometry = *world.resource::<ArenaVoxelGeometry>();
    let sites = view
        .expedition
        .as_ref()
        .expect("Grand exact route publication");
    assert!(
        !sites.routes.is_empty(),
        "r02 must publish its actual stacked route supports"
    );
    for (name, route) in &sites.routes {
        // Keeping every support preserves turns and stacked stair levels.
        // Heights participate in waypoint completion, so crossing a spiral's
        // plan projection on the wrong floor cannot count as an ascent.
        let points = route
            .supports
            .iter()
            .map(|support| support.coord.to_world(geometry.top(*support)).to_array())
            .collect();
        routes.push(Route {
            name: name.clone(),
            category: "authored_connection",
            points,
            stacked: true,
        });
    }
    routes
}

fn human(world: &World) -> &hex_arena::Actor {
    world
        .resource::<ArenaSession>()
        .actors
        .iter()
        .find(|a| a.id == 0)
        .expect("real Grand human")
}

fn ready(world: &World, point: Vec3) -> bool {
    let view = world.resource::<ArenaTerrainView>();
    let geometry = *world.resource::<ArenaVoxelGeometry>();
    view.residency.as_ref().is_some_and(|residency| {
        HexCoord::from_world(point)
            .within_radius(3)
            .into_iter()
            .all(|coord| residency.at(coord, geometry) == ArenaAvailability::Ready)
    })
}

fn body_state(world: &World) -> serde_json::Value {
    let player = human(world);
    let view = world.resource::<ArenaTerrainView>();
    let geometry = *world.resource::<ArenaVoxelGeometry>();
    let coord = HexCoord::from_world(player.feet);
    let nearby: Vec<_> = coord.within_radius(1).into_iter().map(|c| serde_json::json!({
        "q": c.x(), "r": c.y(),
        "terrain": view.columns.get(&c).map(|spans| spans.iter().map(|s| [s.bottom.level,s.top_level]).collect::<Vec<_>>()),
        "objects": view.object_columns.get(&c).map(|spans| spans.iter().map(|s| [s.bottom.level,s.top_level]).collect::<Vec<_>>()),
        "liquids": view.liquids.iter().filter(|s| s.bottom.coord == c).map(|s| [s.bottom.level,s.top_level]).collect::<Vec<_>>(),
        "ready": view.residency.as_ref().map(|r| r.at(c, geometry) == ArenaAvailability::Ready),
    })).collect();
    serde_json::json!({
        "feet": player.feet.to_array(), "grounded": player.grounded,
        "body_dimensions": player.body_dimensions().to_array(), "hp": player.hp,
        "impulse": player.impulse_velocity().to_array(), "step_rise": player.step_rise_this_tick(),
        "notice": world.resource::<ArenaSession>().notice,
        "volume_valid": world.resource::<ArenaSession>().actor_volume_valid(0, view, geometry),
        "support_valid": world.resource::<ArenaSession>().actor_pose_valid(0, view, geometry),
        "nearby_columns": nearby,
        "primary_interest": world.resource::<ArenaStreamInterest>().position.to_array(),
        "actor_interests": world.resource::<ArenaSession>().grand_actor_interests().iter().map(|p| p.to_array()).collect::<Vec<_>>(),
        "encounter_readiness": view.expedition.as_ref().map(|sites| sites.encounters.iter().map(|(name,site)| {
            let pos = site.deployment.preferred;
            let preferred = pos.coord.to_world(geometry.top(pos));
            let ready = site.deployment.surfaces.iter().filter(|p| view.residency.as_ref().is_some_and(|r|r.at(p.coord,geometry)==ArenaAvailability::Ready)).count();
            serde_json::json!({"name":name,"preferred":preferred.to_array(),"player_distance":preferred.distance(player.feet),"ready_surfaces":ready,"total_surfaces":site.deployment.surfaces.len()})
        }).collect::<Vec<_>>()),
    })
}

/// Setup clearance at a hex center using the actual production body height;
/// final acceptance uses the gameplay owner's exact body/ground hook.
fn clear_surface(world: &World, coord: HexCoord, desired_height: Option<f32>) -> Option<Vec3> {
    let view = world.resource::<ArenaTerrainView>();
    let geometry = *world.resource::<ArenaVoxelGeometry>();
    let level = view
        .columns
        .get(&coord)?
        .iter()
        .map(|s| s.top_level)
        .min_by(|a, b| {
            desired_height.map_or_else(
                || b.cmp(a),
                |height| {
                    (geometry.top(TilePos::new(coord, *a)) - height)
                        .abs()
                        .total_cmp(&(geometry.top(TilePos::new(coord, *b)) - height).abs())
                },
            )
        })?;
    let feet = coord.to_world(geometry.top(TilePos::new(coord, level)) + 0.02);
    if !ready(world, feet) {
        return None;
    }
    let height = human(world).body_dimensions().y;
    for fraction in [0., 0.125, 0.25, 0.375, 0.5, 0.625, 0.75, 0.875, 1.] {
        let pos = geometry.voxel_at(feet + Vec3::Y * height * fraction)?;
        if view.solid_at(pos).is_some()
            || view.liquids.iter().any(|s| {
                s.bottom.coord == coord && (s.bottom.level..=s.top_level).contains(&pos.level)
            })
        {
            return None;
        }
    }
    Some(feet)
}

fn frame(app: &mut App, direction: Vec3) {
    app.world_mut().resource_mut::<ArenaInput>().human = ActorIntent {
        aim: if direction.length_squared() > 0.0 {
            direction
        } else {
            human(app.world()).aim
        },
        movement: if direction.length_squared() > 0.0 {
            Vec2::Y
        } else {
            Vec2::ZERO
        },
        ..default()
    };
    // The fixture installs the production drive_simulation in Update. It alone
    // runs ArenaTick; there is no direct controller call or position correction.
    app.update();
}

fn start_route(app: &mut App, point: Point, stacked: bool) -> Result<Vec3, String> {
    app.world_mut().resource_mut::<ViewState>().pause();
    // A paused interest load is setup, not movement evidence. Its Y is only an
    // interest coordinate; exact supporting terrain selects the eventual start.
    let interest = Vec3::from_array(point);
    relocate(app, interest);
    let coord = HexCoord::from_world(interest);
    let mut candidates = coord.within_radius(2);
    candidates.sort_by(|a, b| {
        a.to_world(0.)
            .distance_squared(interest.with_y(0.))
            .total_cmp(&b.to_world(0.).distance_squared(interest.with_y(0.)))
    });
    let feet = candidates
        .into_iter()
        .find_map(|c| clear_surface(app.world(), c, stacked.then_some(interest.y)))
        .ok_or_else(|| {
            format!("no clear real dry starting support within two hexes of {point:?}")
        })?;
    relocate(app, feet);
    {
        let mut view = app.world_mut().resource_mut::<ViewState>();
        view.started = true;
        view.paused = false;
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    let first_tick = app.world().resource::<ArenaSession>().tick;
    loop {
        frame(app, Vec3::ZERO);
        let session = app.world().resource::<ArenaSession>();
        if session.tick > first_tick + 12
            && session.actor_pose_valid(
                0,
                app.world().resource::<ArenaTerrainView>(),
                *app.world().resource::<ArenaVoxelGeometry>(),
            )
        {
            return Ok(human(app.world()).feet);
        }
        if session.tick > first_tick + 240 || Instant::now() >= deadline {
            return Err(format!(
                "ordinary settling failed: {}",
                body_state(app.world())
            ));
        }
    }
}

/// Local object-only sensing. Terrain elevations never choose a direction;
/// the unchanged production controller decides whether each step is legal.
struct LocalObjects {
    feet: Vec3,
    radius: f32,
    occupied: std::collections::BTreeSet<HexCoord>,
}
impl LocalObjects {
    fn observe(world: &World) -> Self {
        let player = human(world);
        let geometry = *world.resource::<ArenaVoxelGeometry>();
        let view = world.resource::<ArenaTerrainView>();
        let occupied = HexCoord::from_world(player.feet)
            .within_radius(4)
            .into_iter()
            .filter(|coord| {
                view.object_columns.get(coord).is_some_and(|spans| {
                    spans.iter().any(|span| {
                        let low = geometry.top(span.bottom) - geometry.level_height;
                        let high = geometry.top(TilePos::new(*coord, span.top_level));
                        high > player.feet.y + 0.05
                            && low < player.feet.y + player.body_dimensions().y
                    })
                })
            })
            .collect();
        Self {
            feet: player.feet,
            radius: player.body_dimensions().x * 0.5 + 0.1,
            occupied,
        }
    }

    fn blocked(&self, direction: Vec3, reach: f32) -> bool {
        let side = direction.cross(Vec3::Y);
        // Dense local body-width samples prevent the old sparse 0.6→1.2→2.4
        // ray from missing a root between samples. This is steering advice,
        // never collision admission or an excuse to alter the body's position.
        let mut distance = 0.15;
        while distance <= reach {
            if [-self.radius, 0.0, self.radius].into_iter().any(|offset| {
                self.occupied.contains(&HexCoord::from_world(
                    self.feet + direction * distance + side * offset,
                ))
            }) {
                return true;
            }
            distance += 0.15;
        }
        false
    }
}

#[derive(Default)]
struct ObjectFollower {
    side: Option<f32>,
    entry_distance: f32,
    entries: u32,
    heading: Vec3,
}
impl ObjectFollower {
    fn steer(&mut self, feet: Vec3, target: Vec3, blocked: impl Fn(Vec3, f32) -> bool) -> Vec3 {
        let direct = (target - feet.with_y(0.0)).normalize_or_zero();
        let remaining = feet.with_y(0.0).distance(target);
        if self.side.is_some() && remaining < self.entry_distance - 1.0 && !blocked(direct, 4.8) {
            self.side = None;
        }
        if self.side.is_none() && !blocked(direct, 2.4) {
            self.heading = direct;
            return direct;
        }
        let angles = [
            0.35,
            0.70,
            1.05,
            1.40,
            1.75,
            2.10,
            2.45,
            2.80,
            std::f32::consts::PI,
        ];
        if self.side.is_none() {
            // Choose once at contact. Alternating the preferred side whenever
            // a two-meter sidestep ends caused the old driver to circle roots.
            let choice = angles.into_iter().find_map(|angle| {
                [1.0, -1.0]
                    .into_iter()
                    .find(|sign| !blocked(Quat::from_rotation_y(angle * sign) * direct, 2.4))
            });
            self.side = Some(choice.unwrap_or(1.0));
            self.entry_distance = remaining;
            self.entries += 1;
        }
        let sign = self.side.unwrap_or(1.0);
        // Reassess the same side every frame instead of blindly holding a
        // direction after a second root has physically stopped it. The shorter
        // probe lets the driver turn inside a narrow local gap; it still reads
        // only objects and moves exclusively through ordinary forward input.
        self.heading = [2.4, 0.75]
            .into_iter()
            .find_map(|reach| {
                angles
                    .into_iter()
                    .map(|angle| Quat::from_rotation_y(angle * sign) * direct)
                    .find(|direction| !blocked(*direction, reach))
            })
            .unwrap_or(Vec3::ZERO);
        self.heading
    }
}

#[test]
fn object_follower_remembers_side_and_rechecks_a_blocked_heading() {
    let mut follower = ObjectFollower::default();
    let target = Vec3::X * 40.0;
    let first = follower.steer(Vec3::ZERO, target, |d, _| d.z > -0.6);
    assert!(first.z < -0.6);
    let side = follower.side;
    let second = follower.steer(Vec3::X, target, |d, _| d.z > -0.9);
    assert!(
        second.z < -0.9,
        "a new obstacle must change the held heading"
    );
    assert_eq!(
        follower.side, side,
        "do not switch sides around the same object"
    );
    assert_eq!(follower.entries, 1);
    let final_heading = follower.steer(Vec3::X * 3.0, target, |_, _| false);
    assert_eq!(final_heading, Vec3::X);
    assert!(follower.side.is_none());
}

fn block_reason(world: &World) -> Option<&'static str> {
    if let Some(failure) = world
        .get_resource::<GroundedWalkTrace>()
        .and_then(|trace| trace.failure)
    {
        return Some(failure);
    }
    let player = human(world);
    let session = world.resource::<ArenaSession>();
    let Some(progress) = session.grand_progress() else {
        return Some("Grand progression missing");
    };
    if player.hp <= 0.0 || progress.deaths > 0 {
        return Some("player died or respawned");
    }
    if !progress.shrines.is_empty() || progress.teleport_unlocked {
        return Some("route acquired an upgrade");
    }
    if player.boat().is_some_and(|b| b.active)
        || player.glider().is_some_and(|g| g.open)
        || player.free_flight().is_some_and(|f| f.active)
    {
        return Some("nonwalking movement activated");
    }
    let view = world.resource::<ArenaTerrainView>();
    if ready(world, player.feet)
        && !session.actor_volume_valid(0, view, *world.resource::<ArenaVoxelGeometry>())
    {
        return Some("player entered solid or liquid volume");
    }
    None
}

fn walk_route(app: &mut App, route: &Route, total_deadline: Instant) -> serde_json::Value {
    let began = Instant::now();
    install_grounded_walk_observer(app);
    let first = *route.points.first().expect("nonempty route");
    let start = match start_route(app, first, route.stacked) {
        Ok(start) => start,
        Err(error) => {
            return serde_json::json!({"name":route.name,"category":route.category,"status":"FAIL","phase":"setup","error":error});
        }
    };
    let first_tick = app.world().resource::<ArenaSession>().tick;
    let limits = GroundedWalkLimits::new(app.world().resource::<ArenaVoxelGeometry>().level_height);
    app.insert_resource(GroundedWalkTrace::new(limits, first_tick, start));
    let deadline = (began + ROUTE_LIMIT).min(total_deadline);
    let mut distance = 0.0_f32;
    let mut previous = start;
    let mut samples = vec![serde_json::json!({"tick":first_tick,"feet":start.to_array()})];
    let mut detours = 0_u32;
    let mut steering_trace = Vec::new();
    let mut failed = None;
    let mut completed = 0;
    let mut maximum_step = 0.0_f32;
    let mut sample_tick = first_tick;
    for (index, point) in route.points.iter().skip(1).enumerate() {
        let authored = Vec3::from_array(*point);
        let target = authored.with_y(0.0);
        let segment_start = human(app.world()).feet;
        let segment_distance = segment_start.with_y(0.).distance(target);
        let tick_budget = u64::try_from(
            Duration::from_secs_f32(segment_distance / 3.0 + 20.0).as_millis() * 120 / 1000,
        )
        .expect("bounded authored route duration");
        let segment_tick = app.world().resource::<ArenaSession>().tick;
        let mut best = segment_distance;
        let mut progress_tick = segment_tick;
        let mut follower = ObjectFollower::default();
        let mut steering_sample_tick = segment_tick;
        loop {
            let feet = human(app.world()).feet;
            let tick = app.world().resource::<ArenaSession>().tick;
            let remaining = feet.with_y(0.).distance(target);
            if remaining <= REACHED && (!route.stacked || (feet.y - authored.y).abs() <= 0.8) {
                completed += 1;
                break;
            }
            if remaining < best - 0.3 {
                best = remaining;
                progress_tick = tick;
            }
            if let Some(reason) = block_reason(app.world()) {
                failed = Some(reason.to_string());
            }
            if tick.saturating_sub(progress_tick) > STALL_TICKS {
                failed = Some("no forward progress for five simulation seconds".into());
            }
            if tick.saturating_sub(segment_tick) > tick_budget {
                failed = Some("segment exceeded distance-based ordinary walking budget".into());
            }
            if Instant::now() >= deadline {
                failed = Some("bounded wall-clock route deadline".into());
            }
            if failed.is_some() {
                samples.push(serde_json::json!({"segment":index,"target":point,"remaining":remaining,"block":body_state(app.world())}));
                break;
            }
            let objects = LocalObjects::observe(app.world());
            let previous_side = follower.side;
            let previous_entries = follower.entries;
            let direction = follower.steer(feet, target, |heading, reach| {
                objects.blocked(heading, reach)
            });
            detours += follower.entries - previous_entries;
            if follower.side != previous_side
                || (follower.side.is_some() && tick >= steering_sample_tick + 120)
            {
                steering_sample_tick = tick;
                steering_trace.push(serde_json::json!({
                    "tick":tick,"segment":index,"feet":feet.to_array(),"remaining":remaining,
                    "side":follower.side,"heading":direction.to_array(),
                    "entry_distance":follower.entry_distance,"nearby_object_columns":objects.occupied.len(),
                    "direct_blocked":objects.blocked((target-feet.with_y(0.0)).normalize_or_zero(), 2.4),
                    "event":if follower.side != previous_side { if follower.side.is_some() { "begin" } else { "clear" } } else { "follow" },
                }));
            }
            frame(app, direction);
            let player = human(app.world());
            distance += player.feet.with_y(0.).distance(previous.with_y(0.));
            previous = player.feet;
            maximum_step = maximum_step.max(player.step_rise_this_tick());
            if tick >= sample_tick + 600 {
                sample_tick = tick;
                samples.push(
                    serde_json::json!({"tick":tick,"feet":player.feet.to_array(),"segment":index}),
                );
            }
            if !ready(app.world(), player.feet) {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        if failed.is_some() {
            break;
        }
    }
    if failed.is_none() {
        let settle_tick = app.world().resource::<ArenaSession>().tick;
        let settle_deadline = (Instant::now() + Duration::from_secs(20)).min(total_deadline);
        while app
            .world()
            .resource::<ArenaSession>()
            .tick
            .saturating_sub(settle_tick)
            < 40
        {
            if Instant::now() >= settle_deadline {
                failed = Some(
                    "route endpoint could not complete forty settling simulation ticks".into(),
                );
                break;
            }
            let before = app.world().resource::<ArenaSession>().tick;
            frame(app, Vec3::ZERO);
            if let Some(reason) = block_reason(app.world()) {
                failed = Some(reason.to_string());
                break;
            }
            if app.world().resource::<ArenaSession>().tick == before {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        let world = app.world();
        if failed.is_none()
            && !world.resource::<ArenaSession>().actor_pose_valid(
                0,
                world.resource::<ArenaTerrainView>(),
                *world.resource::<ArenaVoxelGeometry>(),
            )
        {
            failed = Some("route endpoint did not settle onto clear dry support".into());
        }
        if failed.is_none() && world.resource::<GroundedWalkTrace>().airborne_at_end {
            failed = Some("route endpoint remained airborne after settling".into());
        }
        let target = Vec3::from_array(*route.points.last().expect("nonempty route"));
        let remaining = human(world).feet.with_y(0.0).distance(target.with_y(0.0));
        if failed.is_none()
            && (remaining > REACHED
                || (route.stacked && (human(world).feet.y - target.y).abs() > 0.8))
        {
            failed = Some("settling carried the player outside the final waypoint".into());
        }
        samples.push(serde_json::json!({
            "endpoint":body_state(world),"remaining":remaining,
            "settling_ticks":world.resource::<ArenaSession>().tick.saturating_sub(settle_tick),
        }));
    }
    app.world_mut().resource_mut::<ViewState>().pause();
    serde_json::json!({
        "name":route.name,"category":route.category,"status":if failed.is_none(){"PASS"}else{"FAIL"},
        "error":failed,"start":start.to_array(),"end":human(app.world()).feet.to_array(),
        "body_dimensions":human(app.world()).body_dimensions().to_array(),
        "simulation_ticks":app.world().resource::<ArenaSession>().tick-first_tick,
        "wall_seconds":began.elapsed().as_secs_f64(),"distance":distance,"maximum_auto_step":maximum_step,
        "object_detours":detours,"object_steering_trace":steering_trace,"completed_segments":completed,"required_segments":route.points.len()-1,
        "waypoints":route.points,"samples":samples,"grounding":app.world().resource::<GroundedWalkTrace>(),
    })
}

#[test]
#[ignore = "requires explicit actual Grand package and isolated HEX_GAME_DATA_DIR with grand-verification-only marker"]
fn actual_grand_ordinary_walking() {
    let data =
        PathBuf::from(std::env::var_os("HEX_GAME_DATA_DIR").expect("isolated data directory"));
    assert!(
        data.join("grand-verification-only").is_file(),
        "refuse real user data"
    );
    let package =
        PathBuf::from(std::env::var_os("HEX_GRAND_WORLD").expect("explicit actual package"));
    assert!(
        !data.join("walking.json").exists(),
        "use fresh acceptance output"
    );
    let selected = std::env::var("HEX_GRAND_WALK_ROUTE").ok();
    let started = Instant::now();
    let deadline = started + TOTAL_LIMIT;
    let mut app = fixture();
    let routes = routes(app.world());
    if let Some(name) = &selected {
        assert!(
            routes.iter().any(|route| &route.name == name),
            "unknown route {name}"
        );
    }
    let identity = app
        .world()
        .resource::<ArenaTerrainView>()
        .package_identity
        .clone()
        .expect("package identity");
    let mut results = Vec::new();
    for route in routes
        .iter()
        .filter(|r| selected.as_ref().is_none_or(|name| name == &r.name))
    {
        // Each start is an independent fixture. A prior failure must not carry
        // a respawn, velocity, HP change or acquired state into the next probe.
        let attempt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if !results.is_empty() {
                app = fixture();
            }
            walk_route(&mut app, route, deadline)
        }));
        let result = attempt.unwrap_or_else(|payload| {
            let message = payload.downcast_ref::<String>().cloned().or_else(||payload.downcast_ref::<&str>().map(|s|(*s).to_string())).unwrap_or_else(||"non-string fixture panic".to_string());
            serde_json::json!({"name":route.name,"category":route.category,"status":"FAIL","phase":"fixture_or_route_panic","error":message})
        });
        println!("GRAND_WALK_ROUTE {result}");
        results.push(result);
        let pass = results
            .iter()
            .all(|r| r.get("status").and_then(|v| v.as_str()) == Some("PASS"));
        let receipt = serde_json::json!({
            "kind":"grand-ordinary-walking-r03","status":if results.len()==routes.len() && pass {"PASS"}else if pass {"PARTIAL"}else{"FAIL"},
            "scope":"Real package and production ArenaInput/drive_simulation/ArenaTick. Relocations only at independent route starts. No jump, flight, glider, boat, teleport, spells, upgrades or controller changes. Every completed simulation tick, including settling, enforces a one-voxel downward-step and ballistic airtime allowance from production physics; streaming holds add no airtime. AI decisions disabled; body/static-object collision retained. Local object steering only; no terrain path search. Physical walking evidence, not native input/control feel.",
            "package":package,"identity":identity,"selected_route":selected,"expected_routes":routes.len(),"route_names":routes.iter().map(|r| &r.name).collect::<Vec<_>>(),
            "wall_seconds":started.elapsed().as_secs_f64(),"routes":results,
        });
        std::fs::write(
            data.join("walking.json"),
            serde_json::to_vec_pretty(&receipt).expect("walking receipt"),
        )
        .expect("write walking receipt");
    }
    assert!(
        results
            .iter()
            .all(|r| r.get("status").and_then(|v| v.as_str()) == Some("PASS")),
        "ordinary walking failed; see {}",
        data.join("walking.json").display()
    );
}
