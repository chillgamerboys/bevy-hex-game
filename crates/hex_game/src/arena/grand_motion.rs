//! Opt-in temporal evidence: one admitted start, then ordinary Grand walking.
//! Each request records its render pose/time; asynchronous readback never pauses movement.
use super::*;
use bevy::camera::visibility::VisibilitySystems;
use hex_core::arena::{ArenaAvailability, ArenaRenderStatus};
use hex_core::{HexCoord, TilePos};
use hex_map::arena::streamed::StreamedArena;
use std::time::{Duration, Instant};

const COUNT: u32 = 24;
const STRIDE_TICKS: u64 = 48;
const MAX_WALK_FRAMES: u32 = 900;

#[derive(Clone, Copy)]
struct Route {
    name: &'static str,
    start: Vec3,
    end: Vec3,
    river: bool,
}
fn route(view: &str) -> Option<Route> {
    let (name, a, b, reverse, river) = match view {
        "grand-motion-forest-forward" => {
            ("forest-forward", [-240., 220.], [-240., 175.], false, false)
        }
        "grand-motion-forest-reverse" => {
            ("forest-reverse", [-240., 220.], [-240., 175.], true, false)
        }
        "grand-motion-river-forward" => ("river-forward", [-18., 235.], [-56., 260.], false, true),
        "grand-motion-river-reverse" => ("river-reverse", [-18., 235.], [-56., 260.], true, true),
        _ => return None,
    };
    let [ax, az] = a;
    let [bx, bz] = b;
    let (start, end) = (Vec3::new(ax, 200., az), Vec3::new(bx, 200., bz));
    Some(Route {
        name,
        start: if reverse { end } else { start },
        end: if reverse { start } else { end },
        river,
    })
}
pub(super) fn is_view(view: &str) -> bool {
    view.starts_with("grand-motion-")
}

#[derive(Resource)]
pub(super) struct Run {
    route: Route,
    began: Instant,
    placed: Option<u64>,
    start: Option<(u64, u32, Vec3)>,
    written: u32,
    requested: u32,
    last_tick: u64,
    last_clock: f64,
    last_feet: Option<Vec3>,
    relocations: u32,
}
impl Run {
    pub(super) fn loading_interest(&self) -> Option<Vec3> {
        self.placed.is_none().then_some(self.route.start)
    }
}
pub(super) fn install(app: &mut App, state: &ViewState, map: ArenaMap) -> Result<(), String> {
    if state.capture.is_none() || !is_view(&state.capture_view) {
        return Ok(());
    }
    if map != ArenaMap::GrandV4 || !cfg!(feature = "test-support") {
        return Err("Grand temporal capture requires Grand V4 and the test-support feature".into());
    }
    let route = route(&state.capture_view).ok_or("Unknown Grand temporal route")?;
    app.insert_resource(Run {
        route,
        began: Instant::now(),
        placed: None,
        start: None,
        written: 0,
        requested: 0,
        last_tick: 0,
        last_clock: 0.,
        last_feet: None,
        relocations: 0,
    })
    // Streamed mesh/proxy handoff is published before transform propagation.
    // Record after it and after the ordinary Update camera, before render extraction.
    .add_systems(
        PostUpdate,
        capture
            .after(TransformSystems::Propagate)
            .after(VisibilitySystems::CheckVisibility),
    );
    Ok(())
}
fn ready(view: &ArenaTerrainView, geometry: ArenaVoxelGeometry, point: Vec3) -> bool {
    view.residency.as_ref().is_some_and(|r| {
        HexCoord::from_world(point)
            .within_radius(3)
            .into_iter()
            .all(|c| r.at(c, geometry) == ArenaAvailability::Ready)
    })
}
fn body_valid(
    session: &ArenaSession,
    view: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
) -> bool {
    #[cfg(feature = "test-support")]
    {
        session
            .human_actor_id()
            .is_some_and(|id| session.actor_volume_valid(id, view, geometry))
    }
    #[cfg(not(feature = "test-support"))]
    {
        let _ = (session, view, geometry);
        false
    }
}
fn supported(
    session: &ArenaSession,
    view: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
) -> bool {
    #[cfg(feature = "test-support")]
    {
        session
            .human_actor_id()
            .is_some_and(|id| session.actor_pose_valid(id, view, geometry))
    }
    #[cfg(not(feature = "test-support"))]
    {
        let _ = (session, view, geometry);
        false
    }
}
fn start_surface(world: &World, desired: Vec3) -> Option<Vec3> {
    let view = world.resource::<ArenaTerrainView>();
    let geometry = *world.resource::<ArenaVoxelGeometry>();
    let session = world.resource::<ArenaSession>();
    let actor = session
        .actors
        .iter()
        .find(|a| Some(a.id) == session.human_actor_id())?;
    let mut candidates = HexCoord::from_world(desired).within_radius(2);
    candidates.sort_by(|a, b| {
        a.to_world(0.)
            .distance_squared(desired.with_y(0.))
            .total_cmp(&b.to_world(0.).distance_squared(desired.with_y(0.)))
    });
    candidates.into_iter().find_map(|coord| {
        let top = view
            .columns
            .get(&coord)?
            .iter()
            .map(|s| s.top_level)
            .max()?;
        let feet = coord.to_world(geometry.top(TilePos::new(coord, top)) + 0.02);
        if !ready(view, geometry, feet) {
            return None;
        }
        let high = geometry.voxel_at(feet + Vec3::Y * actor.body_dimensions().y)?;
        if (top + 1..=high.level).any(|level| {
            view.solid_at(TilePos::new(coord, level)).is_some()
                || view.liquids.iter().any(|s| {
                    s.bottom.coord == coord && (s.bottom.level..=s.top_level).contains(&level)
                })
        }) {
            return None;
        }
        Some(feet)
    })
}
fn walking_intent(route: Route) -> ActorIntent {
    let direction = (route.end - route.start)
        .with_y(0.)
        .normalize_or(Vec3::NEG_Z);
    let across = Vec3::new(0.55, 0., 0.835);
    let forward = (direction
        + if route.river {
            across * 0.75
        } else {
            Vec3::ZERO
        })
    .normalize_or(direction);
    ActorIntent {
        aim: (forward - Vec3::Y * 0.12).normalize_or(forward),
        movement: Vec2::new(
            direction.dot(forward.cross(Vec3::Y)),
            direction.dot(forward),
        ),
        ..default()
    }
}
pub(super) fn input(world: &mut World) -> Result<ActorIntent, String> {
    if !world.contains_resource::<Run>() {
        return Err("Missing temporal capture state".into());
    }
    world.resource_scope(|world, mut run: Mut<Run>| {
        if run.began.elapsed() > Duration::from_secs(240) {
            return Err("Temporal route exceeded four-minute watchdog".into());
        }
        if run.written == COUNT {
            return Ok(ActorIntent::default());
        }
        if !world.resource::<ArenaSession>().is_grand_run() {
            return Ok(ActorIntent::default());
        }
        if run.placed.is_none() {
            let Some(feet) = start_surface(world, run.route.start) else {
                return Ok(ActorIntent::default());
            };
            let tick = world.resource::<ArenaSession>().tick;
            let mut session = world.resource_mut::<ArenaSession>();
            let id = session.human_actor_id().ok_or("Grand has no human")?;
            let actor = session
                .actors
                .iter_mut()
                .find(|a| a.id == id)
                .ok_or("Human body is missing")?;
            actor.feet = feet;
            actor.previous_feet = feet;
            actor.grounded = false;
            actor.aim = walking_intent(run.route).aim;
            run.placed = Some(tick);
            run.relocations += 1;
            return Ok(ActorIntent::default());
        }
        let session = world.resource::<ArenaSession>();
        let actor = session
            .actors
            .iter()
            .find(|a| Some(a.id) == session.human_actor_id())
            .ok_or("Human body is missing")?;
        let view = world.resource::<ArenaTerrainView>();
        let geometry = *world.resource::<ArenaVoxelGeometry>();
        if actor.hp <= 0.
            || session
                .grand_progress()
                .is_some_and(|p| p.deaths > 0 || !p.shrines.is_empty() || p.teleport_unlocked)
            || actor.boat().is_some_and(|b| b.active)
            || actor.glider().is_some_and(|g| g.open)
            || actor.free_flight().is_some_and(|f| f.active)
        {
            return Err("Temporal walking lost its ordinary living unupgraded human".into());
        }
        if run.start.is_none() {
            if session.tick > run.placed.unwrap_or(session.tick) + 12
                && ready(view, geometry, actor.feet)
                && supported(session, view, geometry)
                && world
                    .get_resource::<ArenaRenderStatus>()
                    .is_some_and(|s| s.pending_chunks == 0)
            {
                run.start = Some((
                    session.tick,
                    world.resource::<ViewState>().frames,
                    actor.feet,
                ));
                run.last_tick = session.tick;
                run.last_clock = session.ocean_time().seconds;
                world.resource_mut::<ViewState>().third_person = false;
            }
            return Ok(ActorIntent::default());
        }
        let (_, frame, _) = run.start.ok_or("Missing route start")?;
        if world.resource::<ViewState>().frames.saturating_sub(frame) > MAX_WALK_FRAMES {
            return Err(format!(
                "Temporal route exceeded {MAX_WALK_FRAMES} render frames at {:?}: {}",
                actor.feet, session.notice
            ));
        }
        if ready(view, geometry, actor.feet) && !body_valid(session, view, geometry) {
            return Err(format!("Invalid walking body at {:?}", actor.feet));
        }
        Ok(walking_intent(run.route))
    })
}
fn capture(world: &mut World) {
    if let Err(error) = capture_inner(world) {
        error!("Grand motion failed: {error}");
        world.write_message(AppExit::error());
    }
}
fn capture_inner(world: &mut World) -> Result<(), String> {
    world.resource_scope(|world, mut run: Mut<Run>| {
        if run.requested == COUNT {
            return Ok(());
        }
        let Some((start_tick, start_frame, start_feet)) = run.start else {
            return Ok(());
        };
        let session = world.resource::<ArenaSession>();
        if session.tick < run.last_tick + STRIDE_TICKS {
            return Ok(());
        }
        let view = world.resource::<ArenaTerrainView>();
        let geometry = *world.resource::<ArenaVoxelGeometry>();
        let actor = session.actors.iter()
            .find(|a| Some(a.id) == session.human_actor_id())
            .ok_or("Human disappeared")?;
        if !ready(view, geometry, actor.feet) {
            return Ok(());
        }
        if !body_valid(session, view, geometry) {
            return Err("Captured body is not valid".into());
        }
        let clock = session.ocean_time();
        let tick = session.tick;
        let feet = actor.feet;
        if clock.seconds <= run.last_clock
            || run.last_feet.is_some_and(|old| old.with_y(0.).distance(feet.with_y(0.)) < 0.15)
        {
            return Err(format!("Temporal frame has no real clock/movement progress at {feet:?}"));
        }
        let body = actor.body_dimensions();
        let eye = actor.eye();
        let grounded = actor.grounded;
        let identity = view.package_identity.clone().ok_or("Package identity missing")?;
        let terrain_revision = view.revision;
        let mut named = world.query::<(&Name, &Visibility, Option<&ViewVisibility>)>();
        let mut hidden = Vec::new();
        let mut visible = Vec::new();
        let mut in_view = Vec::new();
        for (name, visibility, frustum) in named.iter(world) {
            if let Some(id) = name.as_str().strip_prefix("Grand distant grand/tree/") {
                if frustum.is_some_and(|visible| visible.get()) {
                    in_view.push(id.to_owned());
                }
                if *visibility == Visibility::Hidden {
                    hidden.push(id.to_owned());
                } else {
                    visible.push(id.to_owned());
                }
            }
        }
        hidden.sort();
        visible.sort();
        in_view.sort();
        let mut cameras = world.query_filtered::<&Transform, With<ArenaCamera>>();
        let camera = cameras.single(world).map_err(|e| e.to_string())?;
        let camera_position = camera.translation.to_array();
        let camera_rotation = camera.rotation.to_array();
        let ocean = world.resource::<hex_map::ocean::OceanFrame>().time();
        if (ocean.seconds - clock.seconds).abs() > 1e-6 {
            return Err("Rendered ocean/river clock differs from simulation".into());
        }
        let streamed = world.resource::<StreamedArena>();
        let counts = streamed.runtime.counts();
        if let Some(error) = &streamed.failure {
            return Err(error.clone());
        }
        let state = world.resource::<ViewState>();
        let path = state.capture.as_ref().ok_or("Missing image destination")?;
        let target = state.image.clone().ok_or("Missing image target")?;
        let index = run.requested;
        let stem = path.file_stem().ok_or("Image destination has no filename")?.to_string_lossy();
        let path = path.with_file_name(format!("{stem}-{index:02}.png"));
        if path.exists() || path.with_extension("json").exists() {
            return Err(format!("Temporal output already exists: {}", path.display()));
        }
        let coord = HexCoord::from_world(feet);
        let direction = (run.route.end - run.route.start).with_y(0.).normalize_or(Vec3::NEG_Z);
        let mut receipt = serde_json::json!({
            "route": run.route.name, "index": index, "frame": state.frames,
            "tick": tick, "start_tick": start_tick, "start_frame": start_frame,
            "start_feet": start_feet.to_array(), "start_requested": run.route.start.to_array(),
            "ocean_time_seconds": clock.seconds, "ocean_generation": clock.generation,
            "rendered_ocean_time_seconds": ocean.seconds, "river_phase": clock.seconds.rem_euclid(4.) / 4.,
            "feet": feet.to_array(), "eye": eye.to_array(), "body_dimensions": body.to_array(),
            "body_valid": true, "grounded": grounded, "relocations": run.relocations,
            "package_identity": identity, "progress_units": (feet - start_feet).dot(direction),
            "direction": direction.to_array(),
            "camera": {"position": camera_position, "rotation": camera_rotation},
            "camera_mode": "ordinary-first-person"
        });
        let presentation = serde_json::json!({
            "input": {"movement": walking_intent(run.route).movement.to_array(), "aim": walking_intent(run.route).aim.to_array()},
            "chunk": [coord.x().div_euclid(16), coord.y().div_euclid(16)],
            "terrain_revision": terrain_revision, "resident_chunks": counts.resident_chunks,
            "pending_chunks": world.resource::<ArenaRenderStatus>().pending_chunks,
            "forest_proxy_visible": visible, "forest_proxy_hidden": hidden, "forest_proxy_in_view": in_view,
            "evidence": "CONTINUOUS_WINDOWLESS_ORDINARY_WALK; human control feel and taste pending"
        });
        receipt.as_object_mut().ok_or("Receipt is not an object")?
            .extend(presentation.as_object().ok_or("Presentation facts are not an object")?.clone());
        run.last_tick = tick;
        run.last_clock = clock.seconds;
        run.last_feet = Some(feet);
        run.requested += 1;
        world.spawn(Screenshot::image(target)).observe(move |
            captured: On<ScreenshotCaptured>, mut run: ResMut<Run>, state: Res<ViewState>,
            session: Res<ArenaSession>, mut exit: MessageWriter<AppExit>
        | {
            let mut receipt = receipt.clone();
            receipt["readback_completion"] = serde_json::json!({"frame": state.frames, "tick": session.tick});
            let result = (|| -> Result<(), String> {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                captured.image.clone().try_into_dynamic().map_err(|e| e.to_string())?
                    .save(&path).map_err(|e| e.to_string())?;
                std::fs::write(path.with_extension("json"),
                    serde_json::to_vec_pretty(&receipt).map_err(|e| e.to_string())?
                ).map_err(|e| e.to_string())?;
                Ok(())
            })();
            match result {
                Ok(()) => {
                    run.written += 1;
                    if run.written == COUNT {
                        exit.write(AppExit::Success);
                    }
                },
                Err(error) => {
                    error!("Grand motion write failed: {error}");
                    exit.write(AppExit::error());
                },
            }
        });
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn temporal_views_keep_ordinary_first_person_visibility_and_capture_opt_in() {
        for view in ["grand-motion-forest-forward", "grand-motion-river-reverse"] {
            let state = ViewState {
                capture: Some("fixture.png".into()),
                capture_view: view.into(),
                third_person: false,
                ..default()
            };
            assert!(!state.external_camera());
            assert!(!super::super::northern::fixture_view(view));
        }
        let mut app = App::new();
        assert!(install(&mut app, &ViewState::default(), ArenaMap::GrandV4).is_ok());
        assert!(!app.world().contains_resource::<Run>());
    }

    #[test]
    #[expect(
        clippy::expect_used,
        reason = "Named route fixtures and bounded stride must remain present."
    )]
    fn paired_routes_cross_chunk_boundaries_and_use_only_ordinary_input() {
        for (forward, reverse) in [
            ("grand-motion-forest-forward", "grand-motion-forest-reverse"),
            ("grand-motion-river-forward", "grand-motion-river-reverse"),
        ] {
            let a = route(forward).expect("route");
            let b = route(reverse).expect("route");
            assert_eq!(a.start, b.end);
            assert_eq!(a.end, b.start);
            let start = HexCoord::from_world(a.start);
            let end = HexCoord::from_world(a.end);
            assert_ne!(
                (start.x().div_euclid(16), start.y().div_euclid(16)),
                (end.x().div_euclid(16), end.y().div_euclid(16))
            );
            for route in [a, b] {
                let input = walking_intent(route);
                assert!(
                    !input.jump
                        && !input.run
                        && !input.high_jump
                        && !input.flight_toggle
                        && !input.boat_toggle
                        && !input.teleport
                );
                let forward = input.aim.with_y(0.).normalize();
                let actual = forward * input.movement.y + forward.cross(Vec3::Y) * input.movement.x;
                assert!(actual.dot((route.end - route.start).normalize()) > 0.999);
            }
        }
        assert!((COUNT - 1) * u32::try_from(STRIDE_TICKS).expect("bounded") >= 4 * 120);
    }
}
