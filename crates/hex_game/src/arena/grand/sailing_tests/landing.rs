//! Required ordinary boat-to-shore leg, separate from the sailing clock.
use super::*;

pub(super) fn disembark(
    app: &mut App,
    window: Entity,
    target: Vec3,
    peaks: &mut SailingHighwater,
) -> Result<serde_json::Value, String> {
    let start = player(app.world())?.feet;
    if !player(app.world())?.boat().is_some_and(|boat| boat.active) {
        return Err("shore arrival must begin in the actually sailed boat".into());
    }
    let start_tick = app.world().resource::<ArenaSession>().tick;
    let start_seconds = app.world().resource::<ArenaSession>().ocean_time().seconds;
    let deadline = Instant::now() + Duration::from_secs(90);
    {
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.release(KeyCode::KeyS);
        keys.press(KeyCode::KeyB);
        keys.press(KeyCode::KeyW);
    }
    let mut first_toggle = None;
    let mut swimming_observed = false;
    let mut settling_since = None;
    let mut transitions = Vec::new();
    let mut previous_mode = "boat";
    loop {
        let before_tick = app.world().resource::<ArenaSession>().tick;
        steer(app, window, target)?;
        app.update();
        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.release(KeyCode::KeyB);
            keys.clear();
        }
        unchanged_progress(app.world())?;
        observe(app, peaks)?;
        let session = app.world().resource::<ArenaSession>();
        let tick = session.tick;
        let elapsed = session.ocean_time().seconds - start_seconds;
        let actor = player(app.world())?;
        let active_boat = actor.boat().is_some_and(|boat| boat.active);
        if first_toggle.is_none() {
            let pending = app.world().resource::<ArenaInput>().human.boat_toggle;
            first_toggle = Some(serde_json::json!({
                "tick_before":before_tick,"tick_after":tick,
                "boat_active":active_boat,"pending_input_edge":pending,
            }));
            if active_boat && (tick > before_tick || !pending) {
                return Err("single disembark B was refused or consumed".into());
            }
        }
        if tick == before_tick {
            if Instant::now() >= deadline {
                return Err("shore arrival stalled while terrain loaded".into());
            }
            std::thread::sleep(Duration::from_millis(2));
            continue;
        }
        let swimming = actor.swimming().is_some_and(|swim| swim.active);
        swimming_observed |= swimming;
        if active_boat || !session.actor_solid_volume_valid(0) {
            return Err("shore arrival retained the boat or intersected solid terrain".into());
        }
        let view = app.world().resource::<ArenaTerrainView>();
        let geometry = *app.world().resource::<ArenaVoxelGeometry>();
        let ready = view.residency.as_ref().is_some_and(|r| {
            r.at(HexCoord::from_world(actor.feet), geometry) == ArenaAvailability::Ready
        });
        if !ready {
            return Err("shore arrival advanced over unavailable terrain".into());
        }
        let mode = if swimming {
            "swimming"
        } else if actor.grounded {
            "walking"
        } else {
            "airborne"
        };
        if mode != previous_mode {
            transitions
                .push(serde_json::json!({"mode":mode,"tick":tick,"feet":actor.feet.to_array()}));
            previous_mode = mode;
        }
        let remaining = actor.feet.with_y(0.).distance(target.with_y(0.));
        let vertical_error = (actor.feet.y - target.y).abs();
        let vertical_tolerance = geometry.level_height + 0.001;
        let grounded_arrival = remaining <= ARRIVAL_RADIUS
            && vertical_error <= vertical_tolerance
            && actor.grounded
            && !swimming
            && session.actor_pose_valid(0, view, geometry)
            && matches!(
                surface(app.world(), actor.feet),
                OceanSurfaceState::ReadyDry
            );
        if grounded_arrival {
            let since = *settling_since.get_or_insert(tick);
            if tick.saturating_sub(since) >= 40 {
                if !swimming_observed {
                    return Err("boat-to-shore leg did not exercise ordinary swimming".into());
                }
                return Ok(serde_json::json!({
                    "status":"PASS","start":start.to_array(),"target":target.to_array(),
                    "end":actor.feet.to_array(),"remaining":remaining,
                    "vertical_error":vertical_error,"vertical_tolerance":vertical_tolerance,
                    "voxel_height":geometry.level_height,
                    "simulation_ticks":tick-start_tick,"elapsed_simulation_seconds":elapsed,
                    "settling_ticks":tick-since,"grounded":true,"support_valid":true,
                    "solid_body_clear":true,"all_observed_terrain_ready":true,
                    "swimming_observed":swimming_observed,"boat_active":false,
                    "first_b_frame":first_toggle,"transitions":transitions,
                }));
            }
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .release(KeyCode::KeyW);
        } else {
            settling_since = None;
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::KeyW);
        }
        if elapsed >= 60. || Instant::now() >= deadline {
            return Err(format!(
                "shore arrival did not reach dry supported landing; remaining={remaining:.3}"
            ));
        }
    }
}
