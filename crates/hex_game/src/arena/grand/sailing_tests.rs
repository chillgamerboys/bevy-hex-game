//! Opt-in measurement of the authored Grand crossing through normal input and physics.
use super::*;
use bevy::{
    input::mouse::{MouseButtonInput, MouseWheel},
    window::{CursorMoved, CursorOptions, PrimaryWindow},
};
use hex_core::{ocean::OceanSurfaceState, ocean::OceanWaterColumn, HexCoord};
use hex_map::v4::ResidentChunk;

const ARRIVAL_RADIUS: f32 = 2.0;
const SIMULATION_LIMIT_SECONDS: f64 = 120.0;
const WALL_LIMIT: Duration = Duration::from_secs(300);

#[derive(Default, Serialize)]
struct SailingHighwater {
    resident_chunks: usize,
    finite_sources: usize,
    detailed_chunks: usize,
    in_flight_jobs: usize,
    queued_chunks: usize,
    pinned_chunks: usize,
}

fn player(world: &World) -> Result<&hex_arena::Actor, String> {
    world
        .resource::<ArenaSession>()
        .actors
        .iter()
        .find(|actor| actor.id == 0)
        .ok_or_else(|| "Grand human missing".into())
}

fn surface(world: &World, position: Vec3) -> OceanSurfaceState {
    let view = world.resource::<ArenaTerrainView>();
    let geometry = *world.resource::<ArenaVoxelGeometry>();
    let coord = HexCoord::from_world(position);
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
    world.resource::<OceanEnvironmentView>().sample(
        Vec2::new(position.x, position.z),
        world.resource::<ArenaSession>().ocean_time(),
        availability,
        column,
    )
}

fn observe(app: &mut App, peaks: &mut SailingHighwater) -> Result<(), String> {
    let streamed = app.world().resource::<StreamedArena>();
    if let Some(error) = &streamed.failure {
        return Err(format!("streaming failure: {error}"));
    }
    let counts = streamed.runtime.counts();
    let finite = streamed.edits.resident_source_count();
    peaks.resident_chunks = peaks.resident_chunks.max(counts.resident_chunks);
    peaks.finite_sources = peaks.finite_sources.max(finite);
    peaks.in_flight_jobs = peaks.in_flight_jobs.max(counts.in_flight_jobs);
    peaks.queued_chunks = peaks.queued_chunks.max(counts.queued_chunks);
    peaks.pinned_chunks = peaks.pinned_chunks.max(counts.pinned_chunks);
    if counts.resident_chunks > 512 || streamed.peak_resident > 512 || counts.in_flight_jobs > 2 {
        return Err("streaming residency or worker budget exceeded".into());
    }
    if finite != counts.resident_chunks {
        return Err("finite source residency disagrees with runtime".into());
    }
    let mut detailed = app.world_mut().query::<&ResidentChunk>();
    let count = detailed.iter(app.world()).count();
    peaks.detailed_chunks = peaks.detailed_chunks.max(count);
    if count > 256 {
        return Err("detailed publication budget exceeded".into());
    }
    Ok(())
}

fn input_surface(app: &mut App) -> Entity {
    app.init_resource::<super::super::super::northern::NorthernPresentation>()
        .init_resource::<OceanBathymetry>()
        .init_resource::<OceanSurfaceProfile>()
        .init_resource::<hex_world::battle_sky::BattleSkyProfile>()
        .add_systems(
            Update,
            super::super::super::northern::configure.before(ArenaFrame::Input),
        )
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ButtonInput<MouseButton>>()
        .add_message::<MouseButtonInput>()
        .add_message::<CursorMoved>()
        .add_message::<MouseWheel>()
        .add_systems(
            Update,
            super::super::super::input
                .in_set(ArenaFrame::Input)
                .before(super::super::super::northern::interest),
        );
    // A logical ECS window is enough for the production input adapter. No Winit,
    // native window, cursor warp or screen capture is installed by this fixture.
    app.world_mut()
        .spawn((
            Window {
                focused: true,
                ..default()
            },
            CursorOptions::default(),
            PrimaryWindow,
        ))
        .id()
}

fn steer(app: &mut App, window: Entity, target: Vec3) -> Result<(), String> {
    let direction = (target - player(app.world())?.feet)
        .with_y(0.0)
        .normalize_or_zero();
    let target_yaw = (-direction.x).atan2(-direction.z);
    let yaw = app.world().resource::<ViewState>().yaw;
    let error = (target_yaw - yaw).sin().atan2((target_yaw - yaw).cos());
    let logical = app
        .world()
        .get::<Window>(window)
        .ok_or("input window missing")?;
    let center = Vec2::new(logical.width() * 0.5, logical.height() * 0.5);
    // At most 0.1 radians of camera yaw each frame, through the same cursor
    // displacement path used by native input. Hull turn rate remains authoritative.
    let displacement = Vec2::new((-error / 0.0025).clamp(-40.0, 40.0), 0.0);
    app.world_mut().write_message(CursorMoved {
        window,
        position: center + displacement,
        delta: Some(displacement),
    });
    Ok(())
}

fn unchanged_progress(world: &World) -> Result<(), String> {
    let session = world.resource::<ArenaSession>();
    let progress = session.grand_progress().ok_or("Grand progress missing")?;
    let actor = player(world)?;
    if progress.deaths != 0 || actor.hp <= 0.0 {
        return Err("player died or respawned during the crossing".into());
    }
    if !progress.shrines.is_empty() || progress.teleport_unlocked {
        return Err("crossing acquired an upgrade".into());
    }
    if actor.glider().is_some_and(|g| g.open) || actor.free_flight().is_some_and(|f| f.active) {
        return Err("nonboat flight activated".into());
    }
    Ok(())
}

fn endpoint_liveness(
    app: &mut App,
    peaks: &mut SailingHighwater,
) -> Result<serde_json::Value, String> {
    let began = Instant::now();
    let first_tick = app.world().resource::<ArenaSession>().tick;
    {
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.release(KeyCode::KeyW);
        keys.press(KeyCode::KeyS);
    }
    // Arrival time is captured before this ordinary braking probe. Completing
    // twenty ticks proves the endpoint is not a locally ready global load stall.
    while app
        .world()
        .resource::<ArenaSession>()
        .tick
        .saturating_sub(first_tick)
        < 20
    {
        if began.elapsed() >= Duration::from_secs(20) {
            return Err("arrival could not complete twenty ordinary braking ticks".into());
        }
        let before = app.world().resource::<ArenaSession>().tick;
        app.update();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear();
        unchanged_progress(app.world())?;
        observe(app, peaks)?;
        if !player(app.world())?.boat().is_some_and(|boat| boat.active) {
            return Err("arrival probe lost the deployed boat".into());
        }
        if app.world().resource::<ArenaSession>().tick == before {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    Ok(serde_json::json!({
        "completed_ticks":app.world().resource::<ArenaSession>().tick-first_tick,
        "wall_seconds":began.elapsed().as_secs_f64(),"feet":player(app.world())?.feet.to_array(),
        "boat":player(app.world())?.boat(),
    }))
}

fn measure(app: &mut App) -> Result<serde_json::Value, String> {
    let start_anchor =
        std::env::var("HEX_GRAND_SAIL_START_ANCHOR").unwrap_or_else(|_| "sailing_start".to_owned());
    if !matches!(start_anchor.as_str(), "sailing_start" | "sailing_start_bay") {
        return Err(format!("unsupported sailing start anchor: {start_anchor}"));
    }
    let overview = Arc::clone(&app.world().resource::<StreamedArena>().overview);
    let start = Vec3::from_array(
        *overview
            .anchors
            .get(&start_anchor)
            .ok_or_else(|| format!("{start_anchor} missing"))?,
    );
    let target = Vec3::from_array(
        *overview
            .anchors
            .get("volcano_landing")
            .ok_or("volcano_landing missing")?,
    );
    let direction = (target - start).with_y(0.0).normalize_or_zero();
    let identity = app
        .world()
        .resource::<ArenaTerrainView>()
        .package_identity
        .clone();
    let setup_started = Instant::now();
    // The only relocation is the authored start setup. Its actual ocean sample
    // places a stationary swimmer just under the surface; B must admit the hull.
    relocate(app, start);
    publish_ocean(app.world_mut());
    let OceanSurfaceState::ReadyWet(water) = surface(app.world(), start) else {
        return Err(format!("authored {start_anchor} is not admitted wet ocean"));
    };
    let prepared_launch = start.with_y(water.height - 0.2);
    relocate(app, prepared_launch);
    let window = input_surface(app);
    // Apply the exact production environment publication while still paused.
    app.update();
    {
        let mut view = app.world_mut().resource_mut::<ViewState>();
        view.started = true;
        view.paused = false;
        view.initialized = true;
        view.suppress_click = false;
        view.yaw = (-direction.x).atan2(-direction.z);
        view.pitch = 0.0;
    }
    // Complete real idle simulation before B. A nearby encounter may still need
    // terrain after local restore_ready succeeds; no input edge is retried.
    let idle_tick = app.world().resource::<ArenaSession>().tick;
    let idle_deadline = Instant::now() + Duration::from_secs(30);
    while app.world().resource::<ArenaSession>().tick == idle_tick {
        if Instant::now() >= idle_deadline {
            return Err(format!(
                "launch readiness never advanced an idle tick: {}",
                app.world().resource::<ArenaSession>().notice
            ));
        }
        app.update();
        unchanged_progress(app.world())?;
        if app.world().resource::<ArenaSession>().tick == idle_tick {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    let idle_completed_ticks = app.world().resource::<ArenaSession>().tick - idle_tick;
    let launch = player(app.world())?.feet;
    unchanged_progress(app.world())?;
    let before = player(app.world())?
        .boat()
        .ok_or("portable boat capability missing")?;
    if before.active || before.velocity.length_squared() > 0.0001 {
        return Err("sailing setup was not an undeployed boat at rest".into());
    }
    {
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.press(KeyCode::KeyB);
        keys.press(KeyCode::KeyW);
    }
    let setup_wall_seconds = setup_started.elapsed().as_secs_f64();
    let began = Instant::now();
    let deadline = began + WALL_LIMIT;
    let first_tick = app.world().resource::<ArenaSession>().tick;
    let first_seconds = app.world().resource::<ArenaSession>().ocean_time().seconds;
    let first_environment_seconds = app.world().resource::<OceanSimulationTime>().seconds;
    let mut previous = launch;
    let mut distance = 0.0_f64;
    let mut frames = 0_u64;
    let mut unchanged_tick_frames = 0_u64;
    let mut unchanged_tick_wall_seconds = 0.0_f64;
    let mut stationary_boat_ticks = 0_u64;
    let mut deployment_tick = None;
    let mut last_sample_tick = first_tick;
    let mut peaks = SailingHighwater::default();
    let mut samples = Vec::new();
    let mut failure = None;
    let mut wind_min = f32::INFINITY;
    let mut wind_max = 0.0_f32;
    let mut alignment_min = 1.0_f32;
    let mut departure = None;
    let mut first_b_frame = None;
    loop {
        let tick = app.world().resource::<ArenaSession>().tick;
        let actor = player(app.world())?;
        let remaining = actor.feet.with_y(0.0).distance(target.with_y(0.0));
        if deployment_tick.is_some() && remaining <= ARRIVAL_RADIUS {
            break;
        }
        let elapsed = app.world().resource::<ArenaSession>().ocean_time().seconds - first_seconds;
        if elapsed >= SIMULATION_LIMIT_SECONDS || Instant::now() >= deadline {
            failure = Some("crossing exceeded bounded simulation or wall-clock deadline".into());
            break;
        }
        if let Err(error) = unchanged_progress(app.world()).and_then(|()| observe(app, &mut peaks))
        {
            failure = Some(error);
            break;
        }
        steer(app, window, target)?;
        let frame_started = Instant::now();
        app.update();
        frames += 1;
        let now_tick = app.world().resource::<ArenaSession>().tick;
        // Model one physical press, without retrying a rejected or consumed B.
        // simulate may consume a mode edge even when Grand terrain waits prevent
        // tick advancement, so record that explicitly instead of assuming a latch.
        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.release(KeyCode::KeyB);
            keys.clear();
        }
        if now_tick == tick {
            unchanged_tick_frames += 1;
        }
        let actor = player(app.world())?;
        let boat = actor.boat().ok_or("boat capability lost")?;
        if first_b_frame.is_none() {
            let pending = app.world().resource::<ArenaInput>().human.boat_toggle;
            first_b_frame = Some(serde_json::json!({
                "tick_before":tick,"tick_after":now_tick,"boat_active":boat.active,
                "pending_input_edge":pending,"notice":app.world().resource::<ArenaSession>().notice,
            }));
            if !boat.active && (now_tick > tick || !pending) {
                failure = Some(
                    "the single B press was refused or consumed before boat activation".into(),
                );
                break;
            }
        }
        let traveled = actor.feet.with_y(0.0).distance(previous.with_y(0.0));
        distance += f64::from(traveled);
        previous = actor.feet;
        if boat.active {
            if deployment_tick.is_none() {
                deployment_tick = Some(now_tick);
                departure = Some(
                    serde_json::json!({"tick":now_tick,"feet":actor.feet.to_array(),"boat":boat}),
                );
            }
            wind_min = wind_min.min(boat.wind.length());
            wind_max = wind_max.max(boat.wind.length());
            alignment_min = alignment_min.min(boat.heading.dot(boat.wind.normalize_or_zero()));
            if traveled < 0.00001 {
                stationary_boat_ticks += now_tick.saturating_sub(tick);
            }
        } else if deployment_tick.is_some() || now_tick > first_tick + 12 {
            failure = Some(format!(
                "B did not maintain a deployed boat: {}",
                app.world().resource::<ArenaSession>().notice
            ));
            break;
        }
        if now_tick >= last_sample_tick + 120 || samples.is_empty() {
            last_sample_tick = now_tick;
            samples.push(serde_json::json!({
                "tick":now_tick,"elapsed_seconds":app.world().resource::<ArenaSession>().ocean_time().seconds-first_seconds,
                "feet":actor.feet.to_array(),"remaining":actor.feet.with_y(0.0).distance(target.with_y(0.0)),
                "boat":boat,"notice":app.world().resource::<ArenaSession>().notice,
                "surface":format!("{:?}",surface(app.world(),actor.feet)),
                "primary_interest":app.world().resource::<ArenaStreamInterest>().position.to_array(),
            }));
        }
        // Give normal source workers time to run when this headless fixture is
        // advancing faster than a rendered frame. Simulation dt is unchanged.
        if now_tick == tick
            || app
                .world()
                .resource::<StreamedArena>()
                .runtime
                .counts()
                .in_flight_jobs
                > 0
        {
            std::thread::sleep(Duration::from_millis(2));
        }
        if now_tick == tick {
            unchanged_tick_wall_seconds += frame_started.elapsed().as_secs_f64();
        }
    }
    let arrival_tick = app.world().resource::<ArenaSession>().tick;
    let arrival_seconds = app.world().resource::<ArenaSession>().ocean_time().seconds;
    let arrival_environment_seconds = app.world().resource::<OceanSimulationTime>().seconds;
    let arrival_feet = player(app.world())?.feet;
    let arrival_boat = player(app.world())?.boat();
    let crossing_wall_seconds = began.elapsed().as_secs_f64();
    let endpoint_probe = if failure.is_none() {
        match endpoint_liveness(app, &mut peaks) {
            Ok(probe) => Some(probe),
            Err(error) => {
                failure = Some(error);
                None
            }
        }
    } else {
        None
    };
    app.world_mut().resource_mut::<ViewState>().pause();
    observe(app, &mut peaks)?;
    let session = app.world().resource::<ArenaSession>();
    let elapsed = arrival_seconds - first_seconds;
    let remaining = arrival_feet.with_y(0.0).distance(target.with_y(0.0));
    let summary = serde_json::json!({
        "status":if failure.is_none(){"PASS"}else{"FAIL"},"error":failure,
        "start_anchor":start_anchor,
        "identity":identity,"authored_start":start.to_array(),"authored_target":target.to_array(),
        "prepared_launch":prepared_launch.to_array(),"idle_readiness_ticks":idle_completed_ticks,"first_b_frame":first_b_frame,
        "launch":launch.to_array(),"end":arrival_feet.to_array(),"arrival_radius":ARRIVAL_RADIUS,"remaining":remaining,
        "target_surface":format!("{:?}",surface(app.world(),target)),
        "authored_horizontal_distance":start.with_y(0.0).distance(target.with_y(0.0)),"traveled_horizontal_distance":distance,
        "simulation_ticks":arrival_tick-first_tick,"elapsed_simulation_seconds":elapsed,
        "session_clock_start":first_seconds,"session_clock_end":arrival_seconds,
        "environment_clock_start":first_environment_seconds,"environment_clock_end":arrival_environment_seconds,
        "wind_publisher":"production northern::configure, refreshed on terrain revision and quantized player center",
        "reference_seconds":45.0,"reference_delta_seconds":elapsed-45.0,
        "reference_scope":"The approximately 45-second target applies to the accessible western mainland shore. Starting-bay departure is measured separately and may be longer.",
        "timing_acceptance":"Measurement only: no invented tolerance around the requested approximately 45 seconds.",
    });
    let diagnostics = serde_json::json!({
        "setup_wall_seconds":setup_wall_seconds,"crossing_wall_seconds":crossing_wall_seconds,"endpoint_liveness_probe":endpoint_probe,
        "frames":frames,"unchanged_tick_frames":unchanged_tick_frames,"unchanged_tick_wall_seconds":unchanged_tick_wall_seconds,
        "stationary_boat_ticks":stationary_boat_ticks,"departure":departure,"end_boat":arrival_boat,
        "wind_speed_min":wind_min.is_finite().then_some(wind_min),"wind_speed_max":wind_max,
        "minimum_heading_wind_alignment":alignment_min,"prevailing_wind":{"speed":app.world().resource::<OceanEnvironmentView>().wind.speed,"heading_radians":app.world().resource::<OceanEnvironmentView>().wind.heading_radians},
        "unupgraded":session.grand_progress().is_some_and(|p|p.shrines.is_empty() && !p.teleport_unlocked),
        "source_budget":512,"detail_budget":256,"worker_budget":2,"highwater":peaks,"samples":samples,
    });
    let (serde_json::Value::Object(mut fields), serde_json::Value::Object(diagnostics)) =
        (summary, diagnostics)
    else {
        return Err("sailing receipt sections must be JSON objects".into());
    };
    fields.extend(diagnostics);
    Ok(serde_json::Value::Object(fields))
}

#[test]
#[ignore = "requires explicit Grand package and disposable HEX_GAME_DATA_DIR with grand-verification-only marker"]
fn actual_grand_unupgraded_authored_sailing() {
    let data = PathBuf::from(std::env::var_os("HEX_GAME_DATA_DIR").expect("disposable data root"));
    let package =
        PathBuf::from(std::env::var_os("HEX_GRAND_WORLD").expect("explicit actual package"));
    assert!(
        data.join("grand-verification-only").is_file(),
        "refuse real user data"
    );
    let path = data.join("sailing.json");
    assert!(!path.exists(), "use fresh sailing output");
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut app = fixture();
        measure(&mut app)
    }));
    let measurement = match result {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => {
            serde_json::json!({"status":"FAIL","phase":"setup_or_measurement","error":error})
        }
        Err(payload) => {
            let error = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_string()))
                .unwrap_or_else(|| "non-string fixture panic".into());
            serde_json::json!({"status":"FAIL","phase":"fixture_or_measurement_panic","error":error})
        }
    };
    let passed = measurement
        .get("status")
        .and_then(serde_json::Value::as_str)
        == Some("PASS");
    let receipt = serde_json::json!({
        "kind":"grand-authored-sailing-v1","status":if passed{"PASS"}else{"FAIL"},"package":package,
        "scope":"Actual package, live ocean, shared 9-unit prevailing wind, production keyboard/cursor input, drive_simulation and ArenaTick. One authored-start setup relocation only; no route relocation, velocity injection, upgrades, terrain edits or tuning changes. AI decisions disabled by shared fixture. This proves controller travel, not native control feel or visual orientation.",
        "measurement":measurement,
    });
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&receipt).expect("sailing receipt"),
    )
    .expect("write sailing receipt");
    println!("GRAND_SAILING_RECEIPT {}", path.display());
    assert!(passed, "authored sailing failed; see {}", path.display());
    println!("GRAND_SAILING_PASS {}", path.display());
}
