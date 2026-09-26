//! Actual-package traversal through ArenaInput, drive_simulation and ArenaTick.
//! Only independent starts may relocate; every route uses ordinary held walking.
#![expect(
    clippy::expect_used,
    clippy::print_stdout,
    reason = "The explicit integration runner requires complete fixtures and durable diagnostic receipts, including failures."
)]
use super::*;
use hex_core::HexCoord;

const REACHED: f32 = 2.0;
const STALL_TICKS: u64 = 600;
const ROUTE_LIMIT: Duration = Duration::from_secs(180);
const TOTAL_LIMIT: Duration = Duration::from_secs(1200);

type Point = [f32; 2];
struct Route {
    name: &'static str,
    category: &'static str,
    points: &'static [Point],
}

// Coordinates describe independently selected broad areas and the authored
// mountain shoulders. They do not reconstruct the producer's height function.
const ROUTES: &[Route] = &[
    Route {
        name: "river_west_escape",
        category: "bank_escape",
        points: &[[-73., 270.], [-230., 270.]],
    },
    Route {
        name: "river_east_escape",
        category: "bank_escape",
        points: &[[-21., 270.], [130., 270.]],
    },
    Route {
        name: "island_landing_and_ascent",
        category: "island",
        points: &[
            [-1099., 465.],
            [-1118., 475.],
            [-1131., 502.],
            [-1184., 510.],
            [-1238., 484.],
            [-1240., 431.],
            [-1207., 395.],
            [-1161., 406.],
            [-1145., 439.],
            [-1170., 455.],
        ],
    },
    Route {
        name: "western_hill_north",
        category: "cross_country",
        points: &[[-240., 220.], [-240., 140.]],
    },
    Route {
        name: "western_hill_south",
        category: "cross_country",
        points: &[[-240., 220.], [-240., 300.]],
    },
    Route {
        name: "western_hill_west",
        category: "cross_country",
        points: &[[-240., 220.], [-320., 220.]],
    },
    Route {
        name: "western_hill_east",
        category: "cross_country",
        points: &[[-240., 220.], [-160., 220.]],
    },
    Route {
        name: "eastern_valley_diagonal",
        category: "cross_country",
        points: &[[480., 90.], [600., 200.], [400., 310.]],
    },
    Route {
        name: "garden_ascent",
        category: "mountain",
        points: &[
            [490., -110.],
            [505., -185.],
            [500., -310.],
            [425., -405.],
            [470., -525.],
            [330., -535.],
            [275., -490.],
        ],
    },
    Route {
        name: "western_massif_ascent",
        category: "mountain",
        points: &[
            [-170., -115.],
            [-400., -190.],
            [-555., -300.],
            [-550., -430.],
            [-525., -565.],
            [-485., -610.],
            [-400., -610.],
            [-400., -565.],
        ],
    },
    Route {
        name: "crystal_shoulder_ascent",
        category: "mountain",
        points: &[
            [-105., -618.],
            [10., -665.],
            [55., -535.],
            [-45., -450.],
            [-179., -518.],
            [-260., -625.],
            [-400., -610.],
            [-400., -565.],
        ],
    },
];

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
fn clear_surface(world: &World, coord: HexCoord) -> Option<Vec3> {
    let view = world.resource::<ArenaTerrainView>();
    let geometry = *world.resource::<ArenaVoxelGeometry>();
    let level = view
        .columns
        .get(&coord)?
        .iter()
        .map(|s| s.top_level)
        .max()?;
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

fn start_route(app: &mut App, point: Point) -> Result<Vec3, String> {
    app.world_mut().resource_mut::<ViewState>().pause();
    // A paused interest load is setup, not movement evidence. Its Y is only an
    // interest coordinate; exact supporting terrain selects the eventual start.
    let [x, z] = point;
    let interest = Vec3::new(x, 200., z);
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
        .find_map(|c| clear_surface(app.world(), c))
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

fn object_ahead(world: &World, direction: Vec3) -> bool {
    let player = human(world);
    let geometry = *world.resource::<ArenaVoxelGeometry>();
    let view = world.resource::<ArenaTerrainView>();
    let side = direction.cross(Vec3::Y);
    let radius = player.body_dimensions().x * 0.5;
    [0.3, 0.6, 1.2, 2.4].into_iter().any(|distance| {
        [-radius, 0.0, radius].into_iter().any(|offset| {
            let point = player.feet + direction * distance + side * offset;
            let coord = HexCoord::from_world(point);
            view.object_columns.get(&coord).is_some_and(|spans| {
                spans.iter().any(|s| {
                    let low = geometry.top(s.bottom) - geometry.level_height;
                    let high = geometry.top(TilePos::new(coord, s.top_level));
                    high > player.feet.y + 0.05 && low < player.feet.y + player.body_dimensions().y
                })
            })
        })
    })
}

fn block_reason(world: &World) -> Option<&'static str> {
    let player = human(world);
    let session = world.resource::<ArenaSession>();
    let progress = session.grand_progress().expect("Grand progress");
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
    let first = *route.points.first().expect("nonempty route");
    let start = match start_route(app, first) {
        Ok(start) => start,
        Err(error) => {
            return serde_json::json!({"name":route.name,"category":route.category,"status":"FAIL","phase":"setup","error":error});
        }
    };
    let first_tick = app.world().resource::<ArenaSession>().tick;
    let deadline = (began + ROUTE_LIMIT).min(total_deadline);
    let mut distance = 0.0_f32;
    let mut previous = start;
    let mut samples = vec![serde_json::json!({"tick":first_tick,"feet":start.to_array()})];
    let mut detours = 0_u32;
    let mut failed = None;
    let mut completed = 0;
    let mut maximum_step = 0.0_f32;
    let mut sample_tick = first_tick;
    for (index, point) in route.points.iter().skip(1).enumerate() {
        let &[x, z] = point;
        let target = Vec3::new(x, 0., z);
        let segment_start = human(app.world()).feet;
        let segment_distance = segment_start.with_y(0.).distance(target);
        let tick_budget = u64::try_from(
            Duration::from_secs_f32(segment_distance / 3.0 + 20.0).as_millis() * 120 / 1000,
        )
        .expect("bounded authored route duration");
        let segment_tick = app.world().resource::<ArenaSession>().tick;
        let mut best = segment_distance;
        let mut progress_tick = segment_tick;
        let mut detour = None::<(Vec3, Vec3, u64)>;
        loop {
            let feet = human(app.world()).feet;
            let tick = app.world().resource::<ArenaSession>().tick;
            let remaining = feet.with_y(0.).distance(target);
            if remaining <= REACHED {
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
            let direct = (target - feet.with_y(0.)).normalize_or_zero();
            let direction = if let Some((origin, direction, chosen_tick)) =
                detour.filter(|(origin, _, chosen_tick)| {
                    let traveled = feet.with_y(0.).distance(origin.with_y(0.));
                    traveled < 2.0 && (traveled > 0.15 || tick.saturating_sub(*chosen_tick) < 30)
                }) {
                detour = Some((origin, direction, chosen_tick));
                direction
            } else {
                detour = None;
                if object_ahead(app.world(), direct) {
                    // Only small local object avoidance; terrain slope is never
                    // searched around. Broad cross-country probes remain broad.
                    let choice = [0.55, -0.55, 1.05, -1.05, 1.57, -1.57, 2.1, -2.1]
                        .into_iter()
                        .map(|yaw| Quat::from_rotation_y(yaw) * direct)
                        .find(|d| !object_ahead(app.world(), *d));
                    choice.map_or(direct, |direction| {
                        detours += 1;
                        detour = Some((feet, direction, tick));
                        direction
                    })
                } else {
                    direct
                }
            };
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
        let [x, z] = *route.points.last().expect("nonempty route");
        let remaining = human(world).feet.with_y(0.0).distance(Vec3::new(x, 0.0, z));
        if failed.is_none() && remaining > REACHED {
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
        "object_detours":detours,"completed_segments":completed,"required_segments":route.points.len()-1,
        "waypoints":route.points,"samples":samples,
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
    if let Some(name) = &selected {
        assert!(
            ROUTES.iter().any(|r| r.name == name),
            "unknown route {name}"
        );
    }
    let started = Instant::now();
    let deadline = started + TOTAL_LIMIT;
    let mut app = fixture();
    let identity = app
        .world()
        .resource::<ArenaTerrainView>()
        .package_identity
        .clone()
        .expect("package identity");
    let mut results = Vec::new();
    for route in ROUTES
        .iter()
        .filter(|r| selected.as_ref().is_none_or(|name| name == r.name))
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
            "kind":"grand-ordinary-walking-v1","status":if results.len()==ROUTES.len() && pass {"PASS"}else if pass {"PARTIAL"}else{"FAIL"},
            "scope":"Real package and production ArenaInput/drive_simulation/ArenaTick. Relocations only at independent route starts. No jump, flight, glider, boat, teleport, spells, upgrades or controller changes. AI decisions disabled; body/static-object collision retained. Local object steering only; no terrain path search. Physical walking evidence, not native input/control feel.",
            "package":package,"identity":identity,"selected_route":selected,"expected_routes":ROUTES.len(),
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
