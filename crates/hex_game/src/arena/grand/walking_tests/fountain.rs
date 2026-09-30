//! One actual garden approach through ordinary movement and the shipping observer/map systems.
use super::*;
use crate::arena::ux::package_observation;
use hex_arena::LandmarkKind;

const FOUNTAIN: &str = "garden_fountain";
const HOLD_TICKS: u64 = 90;

#[derive(Resource)]
struct FountainGaze(Vec3);

#[derive(Resource, Default)]
struct FirstDiscovery(Option<serde_json::Value>);

#[derive(Resource)]
struct RepeatContact {
    frames: u64,
    all_inside: bool,
}

// A point on the human body's vertical center line strictly inside an actual
// published fountain liquid voxel is a sufficient contact witness. This uses
// public world voxel facts; it does not reproduce the capsule overlap algorithm.
fn fountain_contact(world: &World) -> bool {
    let player = human(world);
    let geometry = *world.resource::<ArenaVoxelGeometry>();
    let view = world.resource::<ArenaTerrainView>();
    let point = player.feet + Vec3::Y * player.body_dimensions().y * 0.1;
    let Some(at) = geometry.voxel_at(point) else {
        return false;
    };
    view.expedition
        .as_ref()
        .and_then(|s| s.fountains.get(FOUNTAIN))
        .is_some_and(|pool| pool.cells.contains(&at))
        && view.liquids.iter().any(|liquid| {
            liquid.bottom.coord == at.coord
                && liquid.bottom.level <= at.level
                && at.level <= liquid.top_level
        })
}

pub(super) fn record_frame(world: &mut World) {
    if world.contains_resource::<RepeatContact>() {
        let inside = fountain_contact(world);
        let mut contact = world.resource_mut::<RepeatContact>();
        contact.frames += 1;
        contact.all_inside &= inside;
    }
    if !world
        .get_resource::<FirstDiscovery>()
        .is_some_and(|r| r.0.is_none())
    {
        return;
    }
    let Some(marker) = known(world) else { return };
    let Some((origin, direction)) = package_observation::camera(world) else {
        return;
    };
    let center = world.resource::<FountainGaze>().0;
    let snapshot = serde_json::json!({
        "tick":world.resource::<ArenaSession>().tick,"marker":marker,
        "camera_origin":origin.to_array(),"camera_direction":direction.to_array(),
        "ray_terrain_ready":sight_resident(world,origin,center),
        "production_sight_clear":world.resource::<ArenaSession>().observation_sight_clear(origin,center),
        "actual_map_symbols":package_observation::map_symbols(world,Some(marker.position)),
    });
    world.resource_mut::<FirstDiscovery>().0 = Some(snapshot);
}

fn aim_fountain(
    gaze: Option<Res<FountainGaze>>,
    session: Res<ArenaSession>,
    mut view: ResMut<ViewState>,
) {
    let Some(gaze) = gaze else { return };
    let Some(player) = session
        .actors
        .iter()
        .find(|a| Some(a.id) == session.human_actor_id())
    else {
        return;
    };
    let direction = (gaze.0 - player.eye()).normalize_or_zero();
    view.yaw = (-direction.x).atan2(-direction.z);
    view.pitch = direction.y.clamp(-1.0, 1.0).asin();
}

fn known(world: &World) -> Option<hex_arena::DiscoveredLandmark> {
    world
        .resource::<ArenaSession>()
        .discovered_landmarks()
        .into_iter()
        .find(|m| m.id == FOUNTAIN && m.kind == LandmarkKind::Fountain)
}

fn consumed(world: &World) -> Option<bool> {
    world
        .resource::<ArenaSession>()
        .expedition_progress()?
        .fountains
        .into_iter()
        .find(|f| f.name == FOUNTAIN)
        .map(|f| f.consumed)
}

fn require(
    report: &mut serde_json::Value,
    name: &str,
    pass: bool,
    facts: serde_json::Value,
) -> Result<(), String> {
    report["assertions"][name] =
        serde_json::json!({"status":if pass {"PASS"}else{"FAIL"},"facts":facts});
    if pass {
        Ok(())
    } else {
        Err(format!("fountain assertion failed: {name}"))
    }
}

fn hold(app: &mut App, ticks: u64, deadline: Instant) -> Result<(), String> {
    let start = app.world().resource::<ArenaSession>().tick;
    while app
        .world()
        .resource::<ArenaSession>()
        .tick
        .saturating_sub(start)
        < ticks
    {
        if Instant::now() >= deadline {
            return Err("fountain hold exceeded wall deadline".into());
        }
        frame(app, Vec3::ZERO);
        if !ready(app.world(), human(app.world()).feet) {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    Ok(())
}

fn sight_resident(world: &World, origin: Vec3, target: Vec3) -> bool {
    let view = world.resource::<ArenaTerrainView>();
    let geometry = *world.resource::<ArenaVoxelGeometry>();
    let Some(residency) = view.residency.as_ref() else {
        return false;
    };
    // A sub-hex sampling interval plus each sample's full immediate ring covers
    // the camera-to-water segment. Missing terrain may never count as occlusion.
    let distance = origin.distance(target);
    if !distance.is_finite() || distance > 100.0 {
        return false;
    }
    (0..=400_u16).all(|step| {
        let point = origin.lerp(target, f32::from(step) / 400.0);
        HexCoord::from_world(point)
            .within_radius(1)
            .into_iter()
            .all(|at| residency.at(at, geometry) == ArenaAvailability::Ready)
    })
}

fn normal_body(world: &World) -> Result<(), String> {
    let session = world.resource::<ArenaSession>();
    let player = human(world);
    let progress = session
        .grand_progress()
        .ok_or("Grand progression missing")?;
    if player.hp <= 0.0 || progress.deaths != 0 {
        return Err("death/respawn during fountain approach".into());
    }
    if !progress.shrines.is_empty()
        || progress.teleport_unlocked
        || player.boat().is_some_and(|b| b.active)
        || player.glider().is_some_and(|g| g.open)
        || player.free_flight().is_some_and(|f| f.active)
    {
        return Err("fountain approach acquired an upgrade or nonwalking movement".into());
    }
    if !ready(world, player.feet) {
        return Err("fountain approach lost ready terrain".into());
    }
    if !session.actor_solid_volume_valid(0) {
        return Err("fountain approach entered solid geometry".into());
    }
    if let Some(failure) = world
        .get_resource::<GroundedWalkTrace>()
        .and_then(|t| t.failure)
    {
        return Err(format!("fountain approach grounding: {failure}"));
    }
    Ok(())
}

fn approach(
    app: &mut App,
    target: Vec3,
    report: &mut serde_json::Value,
    phase: &str,
    deadline: Instant,
    reached: impl Fn(&World) -> bool,
) -> Result<(), String> {
    let start = human(app.world()).feet;
    let first_tick = app.world().resource::<ArenaSession>().tick;
    let mut follower = ObjectFollower::default();
    let mut best = start.with_y(0.0).distance(target.with_y(0.0));
    let mut progress_tick = first_tick;
    let mut samples = Vec::new();
    let result = loop {
        let world = app.world();
        let tick = world.resource::<ArenaSession>().tick;
        let feet = human(world).feet;
        if let Err(error) = normal_body(world) {
            break Err(error);
        }
        if reached(world) {
            break Ok(());
        }
        let remaining = feet.with_y(0.0).distance(target.with_y(0.0));
        if remaining < best - 0.3 {
            best = remaining;
            progress_tick = tick;
        }
        if tick.saturating_sub(progress_tick) > STALL_TICKS
            || tick.saturating_sub(first_tick) > 7200
            || Instant::now() >= deadline
        {
            break Err(
                "ordinary fountain approach stalled or exceeded its bounded deadline".into(),
            );
        }
        if samples
            .last()
            .and_then(|s: &serde_json::Value| s["tick"].as_u64())
            .is_none_or(|last| tick >= last + 120)
        {
            samples.push(serde_json::json!({"tick":tick,"feet":feet.to_array(),"remaining":remaining,"known":known(world).is_some(),"consumed":consumed(world)}));
        }
        let objects = LocalObjects::observe(world);
        let direction = follower.steer(feet, target.with_y(0.0), |heading, reach| {
            objects.blocked(heading, reach)
        });
        // Stop at the exact water target if discovery still needs its normal
        // dwell, rather than walking past the basin while waiting for knowledge.
        frame(
            app,
            if remaining < human(app.world()).body_dimensions().x * 0.5 {
                Vec3::ZERO
            } else {
                direction
            },
        );
    };
    report[phase] = serde_json::json!({
        "status":if result.is_ok(){"PASS"}else{"FAIL"},"error":result.as_ref().err(),
        "target":target.to_array(),"start":start.to_array(),"simulation_ticks":app.world().resource::<ArenaSession>().tick-first_tick,
        "samples":samples,"endpoint":body_state(app.world()),"grounding":app.world().get_resource::<GroundedWalkTrace>(),
        "grounding_scope":"continuous from garden court through reveal and consumption; no reset at discovery or injury setup",
        "path_source":"direct destination selected from actual published fountain liquid cells; ordinary local object steering, no added authored route or relocation",
    });
    result
}

fn run(app: &mut App, report: &mut serde_json::Value) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(300);
    let route = routes(app.world())
        .into_iter()
        .find(|r| r.name == "garden_ascent")
        .ok_or("missing garden_ascent")?;
    let geometry = *app.world().resource::<ArenaVoxelGeometry>();
    let view = app.world().resource::<ArenaTerrainView>();
    let pool = view
        .expedition
        .as_ref()
        .and_then(|s| s.fountains.get(FOUNTAIN))
        .ok_or("missing published garden_fountain")?;
    let mut tops = BTreeMap::new();
    for cell in &pool.cells {
        tops.entry(cell.coord)
            .and_modify(|at: &mut TilePos| {
                if cell.level > at.level {
                    *at = *cell;
                }
            })
            .or_insert(*cell);
    }
    let points: Vec<Vec3> = tops
        .values()
        .map(|at| at.coord.to_world(geometry.top(*at) + 0.01))
        .collect();
    let first = *points.first().ok_or("empty fountain liquid volume")?;
    let (min, max) = points
        .iter()
        .fold((first, first), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
    let midpoint = (min + max) * 0.5;
    let center = *points
        .iter()
        .min_by(|a, b| {
            a.distance_squared(midpoint)
                .total_cmp(&b.distance_squared(midpoint))
        })
        .ok_or("no central fountain water")?;
    report["published_fountain"] = serde_json::json!({"id":FOUNTAIN,"liquid_voxels":pool.cells.len(),"surface_points":points.iter().map(|p|p.to_array()).collect::<Vec<_>>(),"central_water":center.to_array()});
    let route_start = *route.points.first().ok_or("empty garden ascent")?;
    let start = start_route(app, route_start, route.stacked)?;
    package_observation::install(app)?;
    app.insert_resource(FountainGaze(center))
        .init_resource::<FirstDiscovery>()
        .add_systems(Update, aim_fountain.in_set(ArenaFrame::Input));
    // First-person is an ordinary camera mode; the shipping observer still
    // derives its actual origin through camera_origin and the collision cache.
    app.world_mut().resource_mut::<ViewState>().third_person = false;
    let hidden_tick = app.world().resource::<ArenaSession>().tick;
    hold(app, HOLD_TICKS, deadline)?;
    let (origin, direction) =
        package_observation::camera(app.world()).ok_or("missing player camera")?;
    let resident = sight_resident(app.world(), origin, center);
    let clear = app
        .world()
        .resource::<ArenaSession>()
        .observation_sight_clear(origin, center);
    let undiscovered = known(app.world()).is_none();
    let symbols = package_observation::map_symbols(app.world_mut(), None);
    let no_leak = !symbols.is_empty() && !symbols.iter().any(|s| s == "+" || s == "○");
    let mut failures = Vec::new();
    for result in [
        require(
            report,
            "occlusion_before_reveal",
            resident && !clear,
            serde_json::json!({"ray_terrain_ready":resident,"production_sight_clear":clear,"camera_origin":origin.to_array(),"camera_direction":direction.to_array(),"held_simulation_ticks":app.world().resource::<ArenaSession>().tick-hidden_tick}),
        ),
        require(
            report,
            "undiscovered_before_reveal",
            undiscovered,
            serde_json::json!({"discovered":known(app.world())}),
        ),
        require(
            report,
            "no_undiscovered_map_marker",
            no_leak,
            serde_json::json!({"actual_map_symbols":symbols}),
        ),
    ] {
        if let Err(error) = result {
            failures.push(error);
        }
    }
    if !failures.is_empty() {
        return Err(failures.join("; "));
    }
    install_grounded_walk_observer(app);
    let ascent = traverse_started_route(app, &route, deadline, false, start, Instant::now());
    let ascent_pass = ascent["status"] == "PASS";
    report["garden_ascent"] = ascent;
    if !ascent_pass {
        return Err("unchanged ordinary garden ascent failed".into());
    }
    app.world_mut().resource_mut::<ViewState>().paused = false;
    let feet = human(app.world()).feet;
    // Discovery can occur during an ordinary step. Keep that entire airborne
    // episode across the reveal/consumption boundary instead of restarting its
    // fall budget when the test applies its explicit injured-health setup.
    app.insert_resource(GroundedWalkTrace::new(
        GroundedWalkLimits::new(app.world().resource::<ArenaVoxelGeometry>().level_height),
        app.world().resource::<ArenaSession>().tick,
        feet,
    ));
    let target = *points
        .iter()
        .min_by(|a, b| {
            a.with_y(0.0)
                .distance_squared(feet.with_y(0.0))
                .total_cmp(&b.with_y(0.0).distance_squared(feet.with_y(0.0)))
        })
        .ok_or("no published fountain destination")?;
    approach(app, target, report, "reveal_approach", deadline, |w| {
        known(w).is_some()
    })?;
    let marker = known(app.world()).ok_or("fountain not discovered")?;
    let discovery = app.world().resource::<FirstDiscovery>().0.clone();
    let observed_clear = discovery
        .as_ref()
        .is_some_and(|d| d["ray_terrain_ready"] == true && d["production_sight_clear"] == true);
    let symbols = package_observation::map_symbols(app.world_mut(), Some(marker.position));
    require(
        report,
        "visible_acquisition",
        observed_clear && !marker.consumed && consumed(app.world()) == Some(false),
        serde_json::json!({"marker":marker,"progress_consumed":consumed(app.world()),"first_discovery":discovery}),
    )?;
    require(
        report,
        "discovered_map_marker",
        symbols.iter().filter(|s| *s == "+").count() == 1 && !symbols.iter().any(|s| s == "○"),
        serde_json::json!({"symbols_at_discovered_position":symbols}),
    )?;
    let max_hp = human(app.world()).max_hp;
    let injured = (max_hp - 50.0).max(1.0);
    app.world_mut()
        .resource_mut::<ArenaSession>()
        .actors
        .iter_mut()
        .find(|a| a.id == 0)
        .ok_or("missing human")?
        .hp = injured;
    report["health_setup"] =
        serde_json::json!({"before":max_hp,"injured":injured,"position_changed":false});
    approach(app, target, report, "consumption_approach", deadline, |w| {
        consumed(w) == Some(true) && fountain_contact(w)
    })?;
    let healed = human(app.world()).hp;
    let expected = (injured + 40.0).min(max_hp);
    require(
        report,
        "production_consumption",
        consumed(app.world()) == Some(true) && (healed - expected).abs() < 0.0001,
        serde_json::json!({"before_hp":injured,"after_hp":healed,"expected_hp":expected,"maximum_hp":max_hp,"progress_consumed":consumed(app.world())}),
    )?;
    let marker = known(app.world()).ok_or("discovered fountain disappeared")?;
    let symbols = package_observation::map_symbols(app.world_mut(), Some(marker.position));
    require(
        report,
        "spent_map_marker",
        marker.consumed
            && symbols.iter().filter(|s| *s == "○").count() == 1
            && !symbols.iter().any(|s| s == "+"),
        serde_json::json!({"marker":marker,"symbols_at_discovered_position":symbols}),
    )?;
    let reinjured = (healed - 10.0).max(1.0);
    app.world_mut()
        .resource_mut::<ArenaSession>()
        .actors
        .iter_mut()
        .find(|a| a.id == 0)
        .ok_or("missing human")?
        .hp = reinjured;
    app.world_mut().remove_resource::<FountainGaze>();
    app.world_mut().resource_mut::<ViewState>().yaw += std::f32::consts::PI;
    app.insert_resource(RepeatContact {
        frames: 0,
        all_inside: true,
    });
    let duplicate_tick = app.world().resource::<ArenaSession>().tick;
    hold(app, HOLD_TICKS, deadline)?;
    let after = human(app.world()).hp;
    let contact = app.world().resource::<RepeatContact>();
    require(
        report,
        "no_duplicate_heal",
        (after - reinjured).abs() < 0.0001
            && consumed(app.world()) == Some(true)
            && known(app.world()).is_some_and(|m| m.consumed)
            && contact.frames > 0
            && contact.all_inside,
        serde_json::json!({"reinjured_hp":reinjured,"after_hp":after,"held_simulation_ticks":app.world().resource::<ArenaSession>().tick-duplicate_tick,"looking_away":true,"observed_contact_frames":contact.frames,"all_observed_frames_inside_fountain":contact.all_inside,"endpoint":body_state(app.world())}),
    )?;
    Ok(())
}

#[test]
#[ignore = "requires explicit actual Grand package and isolated marked HEX_GAME_DATA_DIR"]
fn actual_grand_fountain_observation_and_use() {
    let data = PathBuf::from(std::env::var_os("HEX_GAME_DATA_DIR").expect("isolated storage"));
    assert!(
        data.join("grand-verification-only").is_file(),
        "refuse real user data"
    );
    assert!(
        !data.join("fountain.json").exists(),
        "use fresh fountain evidence output"
    );
    let package =
        PathBuf::from(std::env::var_os("HEX_GRAND_WORLD").expect("explicit actual package"));
    let mut report = serde_json::json!({"kind":"grand-fountain-observation-v1","status":"INCOMPLETE","package":package,"assertions":{},
        "scope":"Actual garden_ascent and published fountain liquid cells; ordinary production controller, collision, observer and map dots. One independent starting setup only, no relocation afterward. HP setup is explicit; no discovered/consumed fact injection. No native visibility or control-feel claim."});
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut app = fixture();
        report["identity"] =
            serde_json::to_value(&app.world().resource::<ArenaTerrainView>().package_identity)
                .expect("package identity");
        let result = run(&mut app, &mut report);
        report["final_body"] = body_state(app.world());
        result
    }))
    .unwrap_or_else(|payload| {
        Err(payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
            .unwrap_or_else(|| "fountain fixture panic".into()))
    });
    report["status"] = if result.is_ok() { "PASS" } else { "FAIL" }.into();
    report["error"] = serde_json::to_value(result.as_ref().err()).expect("fountain error");
    std::fs::write(
        data.join("fountain.json"),
        serde_json::to_vec_pretty(&report).expect("fountain receipt"),
    )
    .expect("write fountain receipt");
    println!("GRAND_FOUNTAIN_RESULT {report}");
    assert!(result.is_ok(), "{}", result.err().unwrap_or_default());
}
