//! Actual-package traversal through ArenaInput, drive_simulation and ArenaTick.
//! Only independent starts may relocate; every route uses ordinary held walking.
#![expect(
    clippy::expect_used,
    reason = "The explicit integration runner requires complete fixtures and durable diagnostic receipts, including failures."
)]
use super::*;
use hex_core::HexCoord;

#[path = "walking_tests/fountain.rs"]
mod fountain;

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
struct WaypointReach {
    horizontal_distance: f32,
    vertical_distance: Option<f32>,
    require_grounded: bool,
}
impl WaypointReach {
    fn new(stacked: bool, body: Vec3, voxel_height: f32) -> Self {
        Self {
            // Authored points name individual supports. A two-unit radius can
            // skip a neighboring hex and cut a stair corner across several
            // risers, despite every published support being traversable.
            horizontal_distance: if stacked {
                body.x.min(body.z) * 0.5
            } else {
                REACHED
            },
            vertical_distance: stacked.then_some(voxel_height * 0.5),
            require_grounded: stacked,
        }
    }

    fn reached(self, feet: Vec3, target: Vec3, grounded: bool) -> bool {
        feet.with_y(0.0).distance(target.with_y(0.0)) <= self.horizontal_distance
            && self
                .vertical_distance
                .is_none_or(|tolerance| (feet.y - target.y).abs() <= tolerance)
            && (!self.require_grounded || grounded)
    }
}

#[test]
fn authored_waypoint_requires_the_stair_support_instead_of_skipping_it() {
    let reach = WaypointReach::new(true, Vec3::new(0.5, 1.2, 0.5), 0.35);
    let feet = Vec3::new(1030.5632, 348.9501, -417.70718);
    let intermediate = Vec3::new(1032.302, 349.30, -417.0);
    // The former two-unit / 0.8-height check accepted this actual garden
    // intermediate before the actor climbed it, then aimed at a higher tread.
    assert!(feet.with_y(0.0).distance(intermediate.with_y(0.0)) < REACHED);
    assert!((feet.y - intermediate.y).abs() < 0.8);
    assert!(!reach.reached(feet, intermediate, true));
    assert!(!reach.reached(intermediate - Vec3::Y * 0.35, intermediate, true));
    assert!(!reach.reached(intermediate + Vec3::X, intermediate, true));
    assert!(!reach.reached(intermediate, intermediate, false));
    assert!(reach.reached(intermediate + Vec3::Y * 0.0001, intermediate, true));
}

#[test]
fn authored_waypoint_does_not_change_cross_country_arrival() {
    let reach = WaypointReach::new(false, Vec3::new(0.5, 1.2, 0.5), 0.35);
    assert!(reach.reached(Vec3::new(1.9, 12.0, 0.0), Vec3::ZERO, false));
    assert!(!reach.reached(Vec3::new(2.1, 0.0, 0.0), Vec3::ZERO, true));
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

const WATER_CROSSING: &str = "grand-lake-foothill-water-crossing";

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum CrossingMode {
    Walking,
    Swimming,
}

#[derive(Clone, Copy)]
struct CrossingSample {
    tick: u64,
    feet: Vec3,
    swimming: bool,
    grounded: bool,
    terrain_ready: bool,
    solid_body_clear: bool,
    wet_depth: Option<f32>,
    oxygen: f32,
}

#[derive(Resource, Serialize)]
struct MixedCrossingTrace {
    contract: &'static str,
    observed_simulation_ticks: u64,
    swimming_ticks: u64,
    wet_swimming_ticks: u64,
    dry_ticks: u64,
    entry_count: u64,
    exit_count: u64,
    swimming_distance: f32,
    maximum_sampled_water_depth: f32,
    minimum_oxygen: f32,
    all_terrain_ready: bool,
    all_solid_bodies_clear: bool,
    swimming_at_end: bool,
    transitions: Vec<serde_json::Value>,
    dry_segments: Vec<GroundedWalkTrace>,
    current_dry_segment: Option<GroundedWalkTrace>,
    failure: Option<&'static str>,
    #[serde(skip)]
    limits: GroundedWalkLimits,
    #[serde(skip)]
    previous: CrossingSample,
}

impl MixedCrossingTrace {
    fn new(limits: GroundedWalkLimits, sample: CrossingSample) -> Self {
        Self {
            contract: "ordinary-walk-swim-walk-v1",
            observed_simulation_ticks: 0,
            swimming_ticks: 0,
            wet_swimming_ticks: 0,
            dry_ticks: 0,
            entry_count: 0,
            exit_count: 0,
            swimming_distance: 0.0,
            maximum_sampled_water_depth: 0.0,
            minimum_oxygen: sample.oxygen,
            all_terrain_ready: sample.terrain_ready,
            all_solid_bodies_clear: sample.solid_body_clear,
            swimming_at_end: sample.swimming,
            transitions: vec![
                serde_json::json!({"tick":sample.tick,"mode":CrossingMode::Walking,"feet":sample.feet.to_array()}),
            ],
            dry_segments: Vec::new(),
            current_dry_segment: Some(GroundedWalkTrace::new(limits, sample.tick, sample.feet)),
            failure: None,
            limits,
            previous: sample,
        }
    }

    fn observe(&mut self, sample: CrossingSample) {
        if sample.tick == self.previous.tick {
            return;
        }
        if sample.tick.checked_sub(self.previous.tick) != Some(1) {
            self.failure
                .get_or_insert("mixed observer missed a simulation tick");
            return;
        }
        self.observed_simulation_ticks += 1;
        self.all_terrain_ready &= sample.terrain_ready;
        self.all_solid_bodies_clear &= sample.solid_body_clear;
        self.minimum_oxygen = self.minimum_oxygen.min(sample.oxygen);
        if !sample.feet.is_finite() || !sample.oxygen.is_finite() {
            self.failure
                .get_or_insert("nonfinite crossing pose or oxygen");
        }
        if !sample.terrain_ready {
            self.failure
                .get_or_insert("completed crossing tick used unavailable local terrain");
        }
        if !sample.solid_body_clear {
            self.failure
                .get_or_insert("crossing body intersected solid geometry");
        }
        if sample.swimming {
            self.swimming_ticks += 1;
            if let Some(depth) = sample.wet_depth {
                self.wet_swimming_ticks += 1;
                self.maximum_sampled_water_depth = self.maximum_sampled_water_depth.max(depth);
                if self.previous.swimming && self.previous.wet_depth.is_some() {
                    self.swimming_distance += sample
                        .feet
                        .with_y(0.0)
                        .distance(self.previous.feet.with_y(0.0));
                }
            }
            if !self.previous.swimming {
                self.entry_count += 1;
                // Include the transition sample in the preceding dry fall.
                // Water activation must never erase a cliff-sized entry drop.
                if let Some(mut dry) = self.current_dry_segment.take() {
                    dry.observe(sample.tick, sample.feet, true);
                    if let Some(failure) = dry.failure {
                        self.failure.get_or_insert(failure);
                    }
                    self.dry_segments.push(dry);
                }
            }
        } else {
            self.dry_ticks += 1;
            if self.previous.swimming {
                self.exit_count += 1;
                self.current_dry_segment = Some(GroundedWalkTrace::new(
                    self.limits,
                    self.previous.tick,
                    self.previous.feet,
                ));
            }
            if let Some(dry) = &mut self.current_dry_segment {
                dry.observe(sample.tick, sample.feet, sample.grounded);
                if let Some(failure) = dry.failure {
                    self.failure.get_or_insert(failure);
                }
            }
        }
        if sample.swimming != self.previous.swimming {
            self.transitions.push(serde_json::json!({
                "tick":sample.tick,"mode":if sample.swimming {CrossingMode::Swimming}else{CrossingMode::Walking},
                "feet":sample.feet.to_array(),"grounded":sample.grounded,"terrain_ready":sample.terrain_ready,
                "solid_body_clear":sample.solid_body_clear,"wet_depth":sample.wet_depth,"oxygen":sample.oxygen,
            }));
        }
        self.swimming_at_end = sample.swimming;
        self.previous = sample;
    }

    fn completion_error(&self, body_width: f32) -> Option<&'static str> {
        self.failure.or_else(|| {
            if self.entry_count == 0 || self.exit_count == 0 || self.swimming_at_end {
                Some("crossing did not complete real walking/swimming/walking transitions")
            } else if self.wet_swimming_ticks == 0 || self.swimming_distance < body_width {
                Some("crossing did not swim through at least one body width of sampled water")
            } else if self
                .current_dry_segment
                .as_ref()
                .is_none_or(|dry| dry.airborne_at_end)
            {
                Some("crossing endpoint was not grounded after swimming")
            } else {
                None
            }
        })
    }
}

fn crossing_sample(world: &World) -> CrossingSample {
    use hex_core::ocean::{OceanSurfaceState, OceanWaterColumn};
    let player = human(world);
    let session = world.resource::<ArenaSession>();
    let view = world.resource::<ArenaTerrainView>();
    let geometry = *world.resource::<ArenaVoxelGeometry>();
    let coord = HexCoord::from_world(player.feet);
    let availability = view
        .residency
        .as_ref()
        .map_or(ArenaAvailability::Unloaded, |r| r.at(coord, geometry));
    let column = view
        .liquids
        .iter()
        .filter(|span| span.bottom.coord == coord)
        .max_by_key(|span| span.top_level)
        .map(|span| OceanWaterColumn {
            mean_height: geometry.top(TilePos::new(coord, span.top_level)),
            bed_height: geometry.top(span.bottom) - geometry.level_height,
            water_id: span.substance,
        });
    let water = world.resource::<OceanEnvironmentView>().sample(
        Vec2::new(player.feet.x, player.feet.z),
        session.ocean_time(),
        availability,
        column,
    );
    let wet_depth = match water {
        OceanSurfaceState::ReadyWet(surface) => Some(surface.mean_height - surface.bed_height),
        _ => None,
    };
    let swim = player.swimming().expect("Grand swimming snapshot");
    CrossingSample {
        tick: session.tick,
        feet: player.feet,
        swimming: swim.active,
        grounded: player.grounded,
        terrain_ready: ready(world, player.feet),
        solid_body_clear: session.actor_solid_volume_valid(0),
        wet_depth,
        oxygen: swim.oxygen_seconds,
    }
}

fn observe_mixed_crossing(world: &mut World) {
    let sample = crossing_sample(world);
    world.resource_mut::<MixedCrossingTrace>().observe(sample);
}

fn install_mixed_crossing_observer(app: &mut App) {
    app.add_systems(
        ArenaTick,
        observe_mixed_crossing
            .after(hex_core::arena::ArenaSystems::Simulate)
            .run_if(resource_exists::<MixedCrossingTrace>),
    );
}

#[test]
fn mixed_crossing_keeps_the_dry_entry_fall_and_ignores_zero_tick_waits() {
    let limits = GroundedWalkLimits::new(0.35);
    let initial = CrossingSample {
        tick: 10,
        feet: Vec3::Y,
        swimming: false,
        grounded: true,
        terrain_ready: true,
        solid_body_clear: true,
        wet_depth: None,
        oxygen: 90.0,
    };
    let mut trace = MixedCrossingTrace::new(limits, initial);
    trace.observe(CrossingSample {
        feet: Vec3::Y * 0.2,
        ..initial
    });
    assert_eq!(trace.observed_simulation_ticks, 0);
    trace.observe(CrossingSample {
        tick: 11,
        feet: Vec3::Y * 0.2,
        swimming: true,
        grounded: false,
        wet_depth: Some(3.0),
        ..initial
    });
    assert_eq!(trace.entry_count, 1);
    assert!(
        trace.failure.is_some(),
        "swim activation cannot erase an excessive entry fall"
    );
}

#[test]
fn mixed_crossing_requires_real_wet_movement_and_dry_exit() {
    let initial = CrossingSample {
        tick: 1,
        feet: Vec3::Y,
        swimming: false,
        grounded: true,
        terrain_ready: true,
        solid_body_clear: true,
        wet_depth: None,
        oxygen: 90.0,
    };
    let mut trace = MixedCrossingTrace::new(GroundedWalkLimits::new(0.35), initial);
    trace.observe(CrossingSample { tick: 2, ..initial });
    assert!(trace.completion_error(0.5).is_some());
    trace.observe(CrossingSample {
        tick: 3,
        swimming: true,
        grounded: false,
        wet_depth: Some(3.0),
        ..initial
    });
    assert!(trace.completion_error(0.5).is_some());
    trace.observe(CrossingSample {
        tick: 4,
        feet: Vec3::new(0.6, 1.0, 0.0),
        swimming: true,
        grounded: false,
        wet_depth: Some(3.0),
        ..initial
    });
    trace.observe(CrossingSample {
        tick: 5,
        feet: Vec3::new(0.6, 1.0, 0.0),
        ..initial
    });
    assert!(trace.completion_error(0.5).is_none());
    assert_eq!(
        trace.observed_simulation_ticks,
        trace.dry_ticks + trace.swimming_ticks
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
    fountain::record_frame(app.world_mut());
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
    blocked_side_reversals: u32,
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
        let clear_heading = |side| {
            [2.4, 0.75].into_iter().find_map(|reach| {
                angles
                    .into_iter()
                    .map(|angle| Quat::from_rotation_y(angle * side) * direct)
                    .find(|direction| !blocked(*direction, reach))
            })
        };
        self.heading = if let Some(direction) = clear_heading(sign) {
            direction
        } else if let Some(direction) = clear_heading(-sign) {
            // A cluster can close the entire retained half-circle. Reverse
            // only at that dead end, then retain the new side normally.
            self.side = Some(-sign);
            self.blocked_side_reversals += 1;
            direction
        } else {
            Vec3::ZERO
        };
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

#[test]
fn object_follower_reverses_only_after_its_entire_side_is_blocked() {
    let mut follower = ObjectFollower::default();
    let target = Vec3::X * 40.0;
    let first = follower.steer(Vec3::ZERO, target, |d, _| d.z > -0.6);
    assert!(first.z < -0.6);
    let retained = follower.side;
    let still_clear = follower.steer(Vec3::ZERO, target, |d, _| d.z.abs() < 0.2);
    assert!(still_clear.z < -0.2);
    assert_eq!(follower.side, retained);
    assert_eq!(follower.blocked_side_reversals, 0);

    let reversed = follower.steer(Vec3::ZERO, target, |d, _| d.z < 0.6);
    assert!(reversed.z > 0.6, "use the available opposite half-circle");
    assert_eq!(follower.side, retained.map(|sign| -sign));
    assert_eq!(follower.blocked_side_reversals, 1);
    assert_eq!(follower.entries, 1, "this is the same obstacle detour");

    let held = follower.steer(Vec3::ZERO, target, |d, _| d.z.abs() < 0.2);
    assert!(held.z > 0.2);
    assert_eq!(follower.blocked_side_reversals, 1);
}

#[test]
fn object_follower_stops_when_both_sides_are_blocked() {
    let mut follower = ObjectFollower::default();
    let heading = follower.steer(Vec3::ZERO, Vec3::X * 40.0, |_, _| true);
    assert_eq!(heading, Vec3::ZERO);
    assert_eq!(follower.blocked_side_reversals, 0);
}

fn block_reason(world: &World) -> Option<&'static str> {
    if let Some(failure) = world
        .get_resource::<MixedCrossingTrace>()
        .and_then(|trace| trace.failure)
    {
        return Some(failure);
    }
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
    let volume_valid = if world.contains_resource::<MixedCrossingTrace>() {
        session.actor_solid_volume_valid(0)
    } else {
        session.actor_volume_valid(0, view, *world.resource::<ArenaVoxelGeometry>())
    };
    if ready(world, player.feet) && !volume_valid {
        return Some("player entered solid or liquid volume");
    }
    None
}

fn walk_route(app: &mut App, route: &Route, total_deadline: Instant) -> serde_json::Value {
    traverse_route(app, route, total_deadline, false)
}

fn traverse_route(
    app: &mut App,
    route: &Route,
    total_deadline: Instant,
    mixed: bool,
) -> serde_json::Value {
    let began = Instant::now();
    if mixed {
        install_mixed_crossing_observer(app);
    } else {
        install_grounded_walk_observer(app);
    }
    let first = *route.points.first().expect("nonempty route");
    let start = match start_route(app, first, route.stacked) {
        Ok(start) => start,
        Err(error) => {
            return serde_json::json!({"name":route.name,"category":route.category,"status":"FAIL","phase":"setup","error":error});
        }
    };
    traverse_started_route(app, route, total_deadline, mixed, start, began)
}

// A focused observation acceptance can hold at its one initial setup, then walk
// this unchanged route without performing another relocation to the first point.
fn traverse_started_route(
    app: &mut App,
    route: &Route,
    total_deadline: Instant,
    mixed: bool,
    start: Vec3,
    began: Instant,
) -> serde_json::Value {
    let first_tick = app.world().resource::<ArenaSession>().tick;
    let limits = GroundedWalkLimits::new(app.world().resource::<ArenaVoxelGeometry>().level_height);
    let waypoint_reach = WaypointReach::new(
        route.stacked,
        human(app.world()).body_dimensions(),
        limits.voxel_height,
    );
    if mixed {
        let sample = crossing_sample(app.world());
        app.insert_resource(MixedCrossingTrace::new(limits, sample));
    } else {
        app.insert_resource(GroundedWalkTrace::new(limits, first_tick, start));
    }
    let deadline = (began + ROUTE_LIMIT).min(total_deadline);
    let mut distance = 0.0_f32;
    let mut previous = start;
    let mut samples = vec![serde_json::json!({"tick":first_tick,"feet":start.to_array()})];
    let mut waypoint_arrivals = Vec::new();
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
            if waypoint_reach.reached(feet, authored, human(app.world()).grounded) {
                if route.stacked {
                    waypoint_arrivals.push(serde_json::json!({
                        "segment": index, "tick": tick, "feet": feet.to_array(),
                        "target": point, "grounded": human(app.world()).grounded,
                    }));
                }
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
                    "blocked_side_reversals":follower.blocked_side_reversals,
                    "event":if follower.side != previous_side { match (previous_side, follower.side) { (Some(_), Some(_)) => "blocked-side-reversal", (_, Some(_)) => "begin", (_, None) => "clear" } } else { "follow" },
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
        if failed.is_none()
            && world
                .get_resource::<GroundedWalkTrace>()
                .is_some_and(|trace| trace.airborne_at_end)
        {
            failed = Some("route endpoint remained airborne after settling".into());
        }
        if failed.is_none() {
            if let Some(trace) = world.get_resource::<MixedCrossingTrace>() {
                failed = trace
                    .completion_error(human(world).body_dimensions().x)
                    .map(str::to_string);
            }
        }
        let target = Vec3::from_array(*route.points.last().expect("nonempty route"));
        let remaining = human(world).feet.with_y(0.0).distance(target.with_y(0.0));
        if failed.is_none()
            && !waypoint_reach.reached(human(world).feet, target, human(world).grounded)
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
        "waypoint_reach":waypoint_reach,
        "waypoint_arrivals":waypoint_arrivals,
        "simulation_ticks":app.world().resource::<ArenaSession>().tick-first_tick,
        "wall_seconds":began.elapsed().as_secs_f64(),"distance":distance,"maximum_auto_step":maximum_step,
        "object_detours":detours,"object_steering_trace":steering_trace,"completed_segments":completed,"required_segments":route.points.len()-1,
        "waypoints":route.points,"samples":samples,"grounding":app.world().get_resource::<GroundedWalkTrace>(),
        "mixed_crossing":app.world().get_resource::<MixedCrossingTrace>(),
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
            "kind":"grand-ordinary-walking-r04","status":if results.len()==routes.len() && pass {"PASS"}else if pass {"PARTIAL"}else{"FAIL"},
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

#[test]
#[ignore = "requires explicit actual Grand package and isolated HEX_GAME_DATA_DIR with grand-verification-only marker"]
fn actual_grand_lake_walk_swim_walk() {
    let data =
        PathBuf::from(std::env::var_os("HEX_GAME_DATA_DIR").expect("isolated data directory"));
    assert!(
        data.join("grand-verification-only").is_file(),
        "refuse real user data"
    );
    assert!(
        !data.join("crossing.json").exists(),
        "use fresh crossing output"
    );
    let package =
        PathBuf::from(std::env::var_os("HEX_GRAND_WORLD").expect("explicit actual package"));
    let selected = std::env::var("HEX_GRAND_CROSSING_ROUTE").ok();
    assert!(
        selected.as_ref().is_none_or(|name| name == WATER_CROSSING),
        "unknown water crossing"
    );
    let mut app = fixture();
    // The original failed dry probe remains historical evidence. Its SAME
    // published endpoints now name a separate mixed-mode claim; no terrain
    // search, endpoint shortening or dry-fall exception creates a passing path.
    let frame = app
        .world()
        .resource::<StreamedArena>()
        .overview
        .review_cameras
        .get("grand-lake-foothill-crossing")
        .expect("unchanged lake crossing frame")
        .clone();
    let points = vec![frame.eye, frame.target];
    let route = Route {
        name: WATER_CROSSING.into(),
        category: "walk_swim_walk",
        points: points.clone(),
        stacked: false,
    };
    let identity = app
        .world()
        .resource::<ArenaTerrainView>()
        .package_identity
        .clone()
        .expect("package identity");
    let began = Instant::now();
    let attempt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        traverse_route(&mut app, &route, began + ROUTE_LIMIT, true)
    }));
    let result = attempt.unwrap_or_else(|payload| {
        let error = payload.downcast_ref::<String>().cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|text| (*text).to_string()))
            .unwrap_or_else(|| "non-string crossing fixture panic".to_string());
        serde_json::json!({"name":WATER_CROSSING,"status":"FAIL","phase":"fixture_or_route_panic","error":error})
    });
    let pass = result.get("status").and_then(|v| v.as_str()) == Some("PASS");
    let receipt = serde_json::json!({
        "kind":"grand-mixed-water-crossing-v1",
        "status":if !pass {"FAIL"} else if selected.is_some() {"PARTIAL"} else {"PASS"},
        "scope":"Same original lake-crossing endpoints through production ArenaInput/drive_simulation/ArenaTick. Actual walking/swimming/walking snapshots, loaded terrain and solid body clearance every completed tick; unchanged one-voxel dry fall limits include entry. No jump, flight, glider, boat, teleport, spells, upgrades or mid-route relocation. Forty completed dry settling ticks. This is mixed movement, never a dry-walking or native-feel verdict.",
        "package":package,"identity":identity,"selected_route":selected,
        "source_frame":"grand-lake-foothill-crossing","authored_endpoints":points,
        "wall_seconds":began.elapsed().as_secs_f64(),"route":result,
    });
    std::fs::write(
        data.join("crossing.json"),
        serde_json::to_vec_pretty(&receipt).expect("crossing receipt"),
    )
    .expect("write crossing receipt");
    println!("GRAND_MIXED_CROSSING {receipt}");
    assert!(
        pass,
        "mixed crossing failed; see {}",
        data.join("crossing.json").display()
    );
}
