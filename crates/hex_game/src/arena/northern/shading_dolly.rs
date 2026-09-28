//! Review-only continuous camera sweep; no gameplay motion or frame-time claim.
use super::*;
use bevy::camera::{visibility::VisibilitySystems, CameraUpdateSystems};
use bevy::diagnostic::FrameCount;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use hex_core::TilePos;
use hex_world_contracts::WorldHex;

const VIEW: &str = "grand-mainland-northwest";
const COUNT: u32 = 16;
const STRIDE: u32 = 16;
const DURATION: u32 = (COUNT - 1) * STRIDE;
const FAR: f32 = 1600.0;
const NEAR: f32 = 250.0;
#[expect(
    clippy::cast_precision_loss,
    reason = "the fixed arena capture width is exactly representable"
)]
const WIDTH: f32 = super::super::WIDTH as f32;
#[expect(
    clippy::cast_precision_loss,
    reason = "the fixed arena capture height is exactly representable"
)]
const HEIGHT: f32 = super::super::HEIGHT as f32;

#[derive(Clone)]
struct Sample {
    point: Vec3,
    step: f32,
    columns: Vec<(i64, i64, i32, u64)>,
}

#[derive(Resource, Default)]
struct Run {
    ready: Option<u32>,
    start: Option<u32>,
    previous: Option<u32>,
    sample: Option<Sample>,
    interest: Option<Vec3>,
    package: Option<u64>,
    backward: Vec3,
    requested: u32,
    completed: u16,
    failed: bool,
}

fn admitted(flag: bool, capture: bool, view: &str, map: ArenaMap) -> bool {
    flag && capture && view == VIEW && map == ArenaMap::GrandV4
}

pub(super) fn install(app: &mut App) {
    // Only the existing capture launch disables Winit and creates an image target.
    // A review build without that launch must never activate this camera driver.
    if std::env::var("HEX_GRAND_SHADING_DOLLY").as_deref() != Ok("1")
        || std::env::var_os("HEX_ARENA_CAPTURE").is_none()
    {
        return;
    }
    app.init_resource::<Run>()
        .add_systems(
            Update,
            drive
                .in_set(ArenaFrame::Present)
                .after(super::camera)
                .before(super::present),
        )
        .add_systems(
            PostUpdate,
            capture
                .after(TransformSystems::Propagate)
                .after(CameraUpdateSystems)
                .after(VisibilitySystems::CheckVisibility),
        );
}

fn settled(first: &mut Option<u32>, ready: bool, frame: u32, wait: u32) -> bool {
    if !ready {
        *first = None;
        return false;
    }
    frame.saturating_sub(*first.get_or_insert(frame)) >= wait.max(4)
}

#[expect(
    clippy::cast_precision_loss,
    reason = "a bounded 240-frame review pass"
)]
fn distance(frame: u32) -> f32 {
    let phase = frame.min(DURATION) as f32 / DURATION as f32;
    FAR + (NEAR - FAR) * (1.0 - (std::f32::consts::TAU * phase).cos()) * 0.5
}

fn perspective() -> Projection {
    Projection::Perspective(PerspectiveProjection {
        fov: 75.0_f32.to_radians(),
        aspect_ratio: WIDTH / HEIGHT,
        near: 0.035,
        far: 24_000.0,
        ..default()
    })
}

fn ready(world: &World) -> bool {
    super::capture_ready(
        VIEW,
        world.resource::<NorthernPresentation>(),
        world.get_resource::<StreamedArena>(),
        world.get_resource::<ArenaRenderStatus>(),
        world.get_resource::<OceanRenderStatus>(),
    )
}

fn fail(world: &mut World, run: &mut Run, error: String) {
    run.failed = true;
    world.resource_mut::<ViewState>().requested = true;
    error!("Grand shading dolly failed: {error}");
    world.write_message(AppExit::error());
}

fn drive(world: &mut World) {
    world.resource_scope(|world, mut run: Mut<Run>| {
        if run.failed {
            return;
        }
        if let Err(error) = advance(world, &mut run) {
            fail(world, &mut run, error);
        }
    });
}

fn advance(world: &mut World, run: &mut Run) -> Result<(), String> {
    let state = world.resource::<ViewState>();
    if !admitted(
        true,
        state.capture.is_some(),
        &state.capture_view,
        world.resource::<ArenaSelection>().map,
    ) {
        return Err("requires Grand V4, the northwest view and a windowless capture target".into());
    }
    let settle = state.capture_settle_frames;
    // Suppress ordinary one-shot capture before it can run in ArenaFrame::Capture.
    world.resource_mut::<ViewState>().requested = true;
    let frame = world.resource::<FrameCount>().0;
    if run.previous.is_some_and(|previous| frame <= previous) {
        return Err("render frame count did not advance".into());
    }
    run.previous = Some(frame);
    if run.start.is_none() {
        if frame > 3000 {
            return Err("publication never settled within 3000 render frames".into());
        }
        if !settled(&mut run.ready, ready(world), frame, settle) {
            return Ok(());
        }
        let sample = canonical_sample(world)?;
        let pose = world
            .resource::<NorthernPresentation>()
            .capture
            .ok_or("missing northwest fixture")?;
        run.backward = pose.camera.rotation * Vec3::Z;
        run.interest = Some(pose.interest);
        run.package = Some(
            world
                .resource::<StreamedArena>()
                .overview
                .package_fingerprint,
        );
        run.sample = Some(sample);
        run.start = Some(frame);
    }
    if !ready(world) {
        return Err("publication readiness changed during the continuous sweep".into());
    }
    let original = run.interest.ok_or("missing fixed interest")?;
    let interest = world.resource::<ArenaStreamInterest>();
    if !interest.position.abs_diff_eq(original, 0.001)
        || interest.velocity.length_squared() > 0.000001
    {
        return Err("ordinary northwest streaming interest moved".into());
    }
    if run.package
        != Some(
            world
                .resource::<StreamedArena>()
                .overview
                .package_fingerprint,
        )
    {
        return Err("package changed during sweep".into());
    }
    let elapsed = frame.saturating_sub(run.start.ok_or("missing start frame")?);
    if elapsed > DURATION + 600 {
        return Err("screenshot readbacks did not complete".into());
    }
    let sample = run.sample.as_ref().ok_or("missing admitted sample")?;
    let eye = sample.point + run.backward * distance(elapsed);
    let mut cameras =
        world.query_filtered::<(&mut Transform, &mut Projection), With<ArenaCamera>>();
    let (mut camera, mut projection) = cameras.single_mut(world).map_err(|e| e.to_string())?;
    *camera = Transform::from_translation(eye).looking_at(sample.point, Vec3::Y);
    *projection = perspective();
    Ok(())
}

#[expect(
    clippy::cast_precision_loss,
    reason = "validated finite Grand voxel levels"
)]
fn canonical_sample(world: &World) -> Result<Sample, String> {
    let streamed = world.resource::<StreamedArena>();
    let terrain = world.resource::<ArenaTerrainView>();
    let geometry = *world.resource::<ArenaVoxelGeometry>();
    let center = WorldHex::new(455, -370);
    let mut columns = Vec::new();
    if terrain
        .package_identity
        .as_ref()
        .is_none_or(|id| id.manifest_fingerprint != streamed.overview.package_fingerprint)
    {
        return Err("public and canonical package identities differ".into());
    }
    for (q, r) in [(0, 0), (1, 0), (0, 1), (-1, 1), (-1, 0), (0, -1), (1, -1)] {
        let at = center
            .checked_add(WorldHex::new(q, r))
            .map_err(|e| e.to_string())?;
        let coord = HexCoord::from_axial(
            i32::try_from(at.q).map_err(|e| e.to_string())?,
            i32::try_from(at.r).map_err(|e| e.to_string())?,
        );
        if terrain
            .residency
            .as_ref()
            .is_none_or(|r| r.at(coord, geometry) != ArenaAvailability::Ready)
        {
            return Err(format!("sample terrain is not loaded at {at:?}"));
        }
        let product = streamed
            .runtime
            .resident_chunk(at.chunk())
            .ok_or("sample canonical chunk is absent")?;
        let column = product
            .package
            .columns
            .iter()
            .find(|c| c.position == at)
            .ok_or("sample column is absent")?;
        let mut top = None;
        for run in &column.runs {
            if top.is_some_and(|end| end != run.bottom)
                || !streamed
                    .overview
                    .materials
                    .iter()
                    .any(|m| m.id == run.material && m.solid && m.color.last() == Some(&255))
            {
                return Err("sample is not one contiguous opaque terrain column".into());
            }
            top = Some(run.top);
        }
        let top = top.ok_or("empty sample column")?;
        let height = top as f32 * streamed.overview.level_height;
        let live = terrain
            .columns
            .get(&coord)
            .and_then(|spans| spans.iter().map(|s| s.top_level).max())
            .ok_or("sample public terrain is absent")?;
        if (geometry.top(TilePos::new(coord, live)) - height).abs() > 0.001
            || height <= streamed.overview.sea_level
            || terrain.liquids.iter().any(|s| s.bottom.coord == coord)
            || terrain
                .object_columns
                .get(&coord)
                .is_some_and(|spans| !spans.is_empty())
        {
            return Err(
                "sample public terrain differs from the dry unobstructed canonical column".into(),
            );
        }
        columns.push((at.q, at.r, top, product.revision));
    }
    let &(_, _, own_top, _) = columns.first().ok_or("missing central sample")?;
    let height = own_top as f32 * streamed.overview.level_height;
    let step = columns
        .iter()
        .fold(streamed.overview.level_height, |v, &(_, _, top, _)| {
            v.max((own_top - top).unsigned_abs() as f32 * streamed.overview.level_height)
        });
    if step > streamed.overview.spacing {
        return Err("sample is a full cliff beyond the shader's physical eligibility cap".into());
    }
    let point = Vec3::new(467.839_26, height, -554.713_13);
    if HexCoord::from_world(point) != HexCoord::from_axial(455, -370) {
        return Err("sample target is outside its canonical hex".into());
    }
    Ok(Sample {
        point,
        step,
        columns,
    })
}

fn pixels_between(matrix: Mat4, a: Vec3, b: Vec3) -> Result<f32, String> {
    let a = matrix * a.extend(1.0);
    let b = matrix * b.extend(1.0);
    if !a.is_finite() || !b.is_finite() || a.w <= 0.0 || b.w <= 0.0 {
        return Err("sample projection is behind the camera or nonfinite".into());
    }
    Ok(
        ((a.truncate().truncate() / a.w - b.truncate().truncate() / b.w)
            * Vec2::new(WIDTH, HEIGHT)
            * 0.5)
            .length(),
    )
}

fn pixels(matrix: Mat4, point: Vec3, step: f32) -> Result<([f32; 3], f32, f32, f32), String> {
    let corners = [
        Vec3::Z,
        Vec3::new(0.866_025_4, 0.0, 0.5),
        Vec3::new(0.866_025_4, 0.0, -0.5),
    ];
    let pairs = corners.map(|v| pixels_between(matrix, point - v, point + v));
    let [a, b, c] = pairs;
    let pairs = [a?, b?, c?];
    let vertical = pixels_between(matrix, point, point + Vec3::Y * step)?;
    let extent = pairs.into_iter().fold(vertical, f32::max);
    let t = ((extent - 1.0) / 2.0).clamp(0.0, 1.0);
    Ok((pairs, vertical, extent, 1.0 - t * t * (3.0 - 2.0 * t)))
}

fn capture(world: &mut World) {
    world.resource_scope(|world, mut run: Mut<Run>| {
        if run.failed || run.start.is_none() || run.requested == COUNT {
            return;
        }
        if let Err(error) = request(world, &mut run) {
            fail(world, &mut run, error);
        }
    });
}

fn request(world: &mut World, run: &mut Run) -> Result<(), String> {
    let frame = world.resource::<FrameCount>().0;
    let elapsed = frame.saturating_sub(run.start.ok_or("missing start")?);
    if elapsed < run.requested * STRIDE {
        return Ok(());
    }
    if elapsed != run.requested * STRIDE || !ready(world) {
        return Err("missed a fixed capture frame or publication became unready".into());
    }
    let sample = canonical_sample(world)?;
    if run.sample.as_ref().is_none_or(|old| {
        old.columns != sample.columns
            || !old.point.abs_diff_eq(sample.point, 0.001)
            || (old.step - sample.step).abs() > 0.001
    }) {
        return Err("canonical sample changed during capture".into());
    }
    let mut query =
        world.query_filtered::<(&GlobalTransform, &Projection, &Camera), With<ArenaCamera>>();
    let (global, projection, view_camera) = query.single(world).map_err(|e| e.to_string())?;
    let Projection::Perspective(actual_projection) = projection else {
        return Err("camera projection was replaced during sweep".into());
    };
    let viewport = view_camera
        .physical_viewport_size()
        .ok_or("capture viewport is not ready")?;
    if viewport != UVec2::new(super::super::WIDTH, super::super::HEIGHT) {
        return Err("capture viewport differs from the recorded pixel metric".into());
    }
    let projection_receipt = serde_json::json!({"kind":"perspective", "fov_degrees":actual_projection.fov.to_degrees(), "aspect_ratio":actual_projection.aspect_ratio, "near":actual_projection.near, "far":actual_projection.far, "viewport":viewport.to_array()});
    let camera = global.compute_transform();
    let actual_distance = camera.translation.distance(sample.point);
    if (actual_distance - distance(elapsed)).abs() > 0.01
        || (camera.rotation * Vec3::NEG_Z).dot((sample.point - camera.translation).normalize())
            < 0.99999
        || (actual_projection.fov - 75.0_f32.to_radians()).abs() > 0.00001
    {
        return Err("render camera differs from the continuous requested pose".into());
    }
    let clip_from_world = projection.get_clip_from_view() * global.to_matrix().inverse();
    let (pairs, vertical, extent, weight) = pixels(clip_from_world, sample.point, sample.step)?;
    let state = world.resource::<ViewState>();
    let path = state.capture.as_ref().ok_or("missing destination")?;
    let stem = path
        .file_stem()
        .ok_or("missing filename")?
        .to_string_lossy();
    let path = path.with_file_name(format!("{stem}-{:02}.png", run.requested));
    if path.exists() || path.with_extension("json").exists() {
        return Err(format!(
            "refusing to replace prior capture {}",
            path.display()
        ));
    }
    let target = state
        .image
        .clone()
        .ok_or("missing windowless image target")?;
    let streamed = world.resource::<StreamedArena>();
    let counts = streamed.runtime.counts();
    let terrain = world.resource::<ArenaTerrainView>();
    let mut receipt = serde_json::json!({
        "evidence": "CONTINUOUS_WINDOWLESS_CAMERA_ONLY_SHADING_SWEEP; no FPS, gameplay motion or control-feel claim",
        "index": run.requested, "count": COUNT, "render_frame": frame, "arena_frame": state.frames,
        "start_render_frame": run.start, "elapsed_render_frames": elapsed, "duration_frames": DURATION,
        "requested_distance": distance(elapsed), "actual_distance":actual_distance, "camera_position": camera.translation.to_array(),
        "camera_rotation": camera.rotation.to_array(), "clip_from_world": clip_from_world.to_cols_array(),
        "projection": projection_receipt,
        "fixed_interest": run.interest.map(|v| v.to_array()), "target":sample.point.to_array(),
        "canonical_columns_q_r_exclusive_top_revision":sample.columns, "full_step_units":sample.step,
        "corner_pair_pixels":pairs, "full_step_pixels":vertical, "maximum_pixels":extent,
        "shader_weight_if_eligible":weight, "weight_note":"Exact shader screen-size term; actual per-fragment eligibility/color remain renderer facts"
    });
    let publication = serde_json::json!({
        "package_fingerprint":run.package, "package_identity":terrain.package_identity,
        "terrain_revision":terrain.revision, "resident_chunks":counts.resident_chunks,
        "queued_chunks":counts.queued_chunks, "in_flight_jobs":counts.in_flight_jobs,
        "pending_chunks":world.resource::<ArenaRenderStatus>().pending_chunks,
        "ready_since_render_frame":run.ready, "settle_frames":state.capture_settle_frames,
        "readback_pauses_camera":false
    });
    receipt
        .as_object_mut()
        .ok_or("receipt is not an object")?
        .extend(
            publication
                .as_object()
                .ok_or("publication is not an object")?
                .clone(),
        );
    let index = run.requested;
    let requested_frame = frame;
    run.requested += 1;
    world.spawn(Screenshot::image(target)).observe(
        move |captured: On<ScreenshotCaptured>,
              mut run: ResMut<Run>,
              frame: Res<FrameCount>,
              state: Res<ViewState>,
              mut exit: MessageWriter<AppExit>| {
            if run.failed {
                return;
            }
            let mut receipt = receipt.clone();
            let result = (|| -> Result<(), String> {
                if frame.0 < requested_frame { return Err("readback precedes its requested render frame".into()); }
                receipt
                    .as_object_mut()
                    .ok_or("receipt is not an object")?
                    .insert(
                        "readback_completion".into(),
                        serde_json::json!({"render_frame":frame.0,"arena_frame":state.frames,"image_width":captured.image.texture_descriptor.size.width,"image_height":captured.image.texture_descriptor.size.height}),
                    );
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                captured
                    .image
                    .clone()
                    .try_into_dynamic()
                    .map_err(|e| e.to_string())?
                    .save(&path)
                    .map_err(|e| e.to_string())?;
                std::fs::write(
                    path.with_extension("json"),
                    serde_json::to_vec_pretty(&receipt).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
                Ok(())
            })();
            match result {
                Ok(()) => {
                    let requested = run.requested;
                    match complete(&mut run.completed, index, requested) {
                        Ok(true) => {
                            exit.write(AppExit::Success);
                        }
                        Ok(false) => {}
                        Err(error) => {
                            run.failed = true;
                            error!("Grand shading readback failed: {error}");
                            exit.write(AppExit::error());
                        }
                    }
                }
                Err(error) => {
                    run.failed = true;
                    error!("Grand shading dolly write failed: {error}");
                    exit.write(AppExit::error());
                }
            }
        },
    );
    Ok(())
}

// Readbacks may finish out of order. They never advance or pause the camera;
// only all uniquely completed requested images authorize successful exit.
fn complete(completed: &mut u16, index: u32, requested: u32) -> Result<bool, String> {
    if index >= COUNT || index >= requested {
        return Err("unrequested screenshot completion".into());
    }
    let bit = 1_u16.checked_shl(index).ok_or("invalid completion index")?;
    if *completed & bit != 0 {
        return Err("duplicate screenshot completion".into());
    }
    *completed |= bit;
    Ok(requested == COUNT && completed.count_ones() == COUNT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shading_dolly_requires_explicit_windowless_grand_northwest_review() {
        assert!(admitted(true, true, VIEW, ArenaMap::GrandV4));
        assert!(!admitted(false, true, VIEW, ArenaMap::GrandV4));
        assert!(!admitted(true, false, VIEW, ArenaMap::GrandV4));
        assert!(!admitted(true, true, "grand-overview", ArenaMap::GrandV4));
        assert!(!admitted(true, true, VIEW, ArenaMap::NorthernArchipelago));
        let mut first = None;
        assert!(!settled(&mut first, true, 10, 8));
        assert!(!settled(&mut first, false, 17, 8));
        assert!(!settled(&mut first, true, 18, 8));
        assert!(!settled(&mut first, true, 25, 8));
        assert!(settled(&mut first, true, 26, 8));
    }

    #[test]
    fn shading_dolly_readbacks_finish_once_in_any_order_without_driving_motion() {
        let mut completed = 0;
        assert!(complete(&mut completed, 0, 0).is_err());
        for index in (1..COUNT).rev() {
            assert!(!complete(&mut completed, index, COUNT).expect("valid outstanding capture"));
        }
        assert!(complete(&mut completed, 3, COUNT).is_err());
        assert!(complete(&mut completed, 0, COUNT).expect("last unique completion"));
        assert!(complete(&mut completed, COUNT, COUNT).is_err());
    }

    #[test]
    fn shading_dolly_crosses_full_shader_pixel_band_in_both_directions() {
        let backward = Vec3::new(-1.0, 0.7, -1.0).normalize();
        let measure = |frame| {
            let camera = Transform::from_translation(backward * distance(frame))
                .looking_at(Vec3::ZERO, Vec3::Y);
            pixels(
                perspective().get_clip_from_view() * camera.to_matrix().inverse(),
                Vec3::ZERO,
                1.75,
            )
            .expect("front-facing sample")
        };
        let (_, _, far, far_weight) = measure(0);
        let (_, _, near, near_weight) = measure(DURATION / 2);
        assert!(far < 1.0 && far_weight > 0.999);
        assert!(near > 3.0 && near_weight < 0.001);
        assert!((measure(DURATION).2 - far).abs() < 0.0001);
        assert!((distance(0) - FAR).abs() < 0.001);
        assert!((distance(DURATION / 2) - NEAR).abs() < 0.001);
        for frame in 1..DURATION / 2 {
            assert!(distance(frame) < distance(frame - 1));
        }
        let (_, tall, extent, _) = pixels(
            perspective().get_clip_from_view()
                * Transform::from_xyz(0.0, 0.0, 500.0).to_matrix().inverse(),
            Vec3::ZERO,
            20.0,
        )
        .expect("tall step");
        assert!(
            (tall - extent).abs() < 0.0001,
            "whole steps, not thin material bands, govern cliffs"
        );
        assert!(pixels(perspective().get_clip_from_view(), Vec3::Z, 1.0).is_err());
    }
}
