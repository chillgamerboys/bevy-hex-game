use bevy::input::mouse::MouseWheel;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy::transform::TransformSystems;
use bevy::window::{CursorMoved, PrimaryWindow};

use hex_assets::{to_color, CameraSettings, ResolvedLighting, Rgb};
use hex_core::{AppSystems, CameraFocusTarget, GameplaySetup, MapViewHint, Screen};

use crate::{
    sky_material::{SkyMaterial, SkyParams},
    LightingSystems,
};

/// Sky-dome radius, in world units. Comfortably inside the camera's default
/// 1000-unit far plane and far outside the configured zoom range plus the terrain.
const SKY_DOME_RADIUS: f32 = 500.0;

/// Marks the sky-dome entity so `follow_camera` can pin it to the camera.
#[derive(Component, Reflect)]
#[reflect(Component)]
pub(crate) struct SkyDome;

/// Registers the pan/orbit camera and the procedural sky.
pub fn plugin(app: &mut App) {
    app.register_type::<PanOrbitCamera>()
        .register_type::<CameraMode>()
        .register_type::<MapViewHint>()
        .register_type::<SkyDome>()
        .init_resource::<CameraMode>()
        .init_resource::<SavedMapCamera>()
        // Spawned once at startup rather than per screen: it is the render target
        // the UI screens draw through, and the sky behind them.
        .add_systems(Startup, spawn_camera)
        .add_systems(
            OnEnter(Screen::Gameplay),
            (reset_camera_mode, frame_gameplay_camera)
                .chain()
                .in_set(GameplaySetup::View),
        )
        // **The sky belongs to the world, not to the menus.** Hidden outside gameplay,
        // so the title screen is the flat `sky_color` that `apply_ambient` already
        // puts in `ClearColor` rather than a view of a dome the player cannot move.
        //
        // Visibility rather than despawn and respawn: the dome carries a material
        // handle built once in `spawn_camera`, and rebuilding it per screen would
        // churn an asset to change one bool.
        .add_systems(OnEnter(Screen::Gameplay), show_sky)
        .add_systems(OnExit(Screen::Gameplay), hide_sky)
        // Only the material push depends on the settings; the dome has to follow the
        // camera every frame regardless.
        .add_systems(
            Update,
            apply_sky_material
                .in_set(LightingSystems::Apply)
                .run_if(resource_exists_and_changed::<ResolvedLighting>),
        )
        // Camera control is gameplay-only, so dragging over a menu does not
        // silently move the world behind it.
        .add_systems(
            Update,
            (
                orbit_camera,
                pan_camera.run_if(map_camera_active),
                toggle_camera_mode,
            )
                .chain()
                .in_set(AppSystems::RecordInput)
                .run_if(in_state(Screen::Gameplay)),
        )
        // Unit animation writes its Transform in Update. Following in PostUpdate
        // observes that final position without coupling this presentation crate to
        // hex_anim or hex_units, then updates GlobalTransform in the same frame.
        .add_systems(
            PostUpdate,
            (follow_character_camera, follow_camera)
                .chain()
                .before(TransformSystems::Propagate)
                .run_if(in_state(Screen::Gameplay)),
        );
}

/// Which perspective currently controls the gameplay camera.
#[derive(Resource, Reflect, Debug, Default, Clone, Copy, PartialEq, Eq)]
#[reflect(Resource)]
pub enum CameraMode {
    /// Free pan/orbit view framed around the complete map.
    #[default]
    Map,
    /// Close orbit whose focus follows the selected character.
    Character,
}

/// Tags an entity as capable of panning and orbiting.
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct PanOrbitCamera {
    /// The point the camera orbits around. Updated automatically when panning.
    pub focus: Vec3,
    /// Distance from the focus point, in world units. Clamped by camera settings.
    pub radius: f32,
}

impl Default for PanOrbitCamera {
    fn default() -> Self {
        PanOrbitCamera {
            focus: Vec3::ZERO,
            radius: 5.0,
        }
    }
}

#[derive(Debug, Clone)]
struct CameraPose {
    transform: Transform,
    focus: Vec3,
    radius: f32,
}

impl CameraPose {
    fn capture(transform: &Transform, camera: &PanOrbitCamera) -> Self {
        Self {
            transform: *transform,
            focus: camera.focus,
            radius: camera.radius,
        }
    }

    fn restore(self, transform: &mut Transform, camera: &mut PanOrbitCamera) {
        *transform = self.transform;
        camera.focus = self.focus;
        camera.radius = self.radius;
    }
}

#[derive(Resource, Debug, Default)]
struct SavedMapCamera(Option<CameraPose>);

/// Spawn the game camera and the procedural sky dome.
fn spawn_camera(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut sky_materials: ResMut<Assets<SkyMaterial>>,
) {
    let translation = Vec3::new(0., 20., 10.0);
    let radius = translation.length();

    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(translation).looking_at(Vec3::ZERO, Vec3::Y),
        PanOrbitCamera {
            radius,
            ..Default::default()
        },
        Name::new("Game Camera"),
    ));

    // A unit sphere scaled to the dome radius, rendered from the inside (see
    // `SkyMaterial::specialize`). `follow_camera` keeps the camera at its centre.
    // Built with placeholder params; `apply_sky_material` fills them once settings
    // load. Not a shadow caster — a 500-unit sphere would shadow the whole map.
    commands.spawn((
        Mesh3d(meshes.add(Sphere::new(1.0).mesh().uv(64, 48))),
        MeshMaterial3d(sky_materials.add(SkyMaterial {
            params: default_sky_params(),
        })),
        Transform::from_scale(Vec3::splat(SKY_DOME_RADIUS)),
        // Hidden until gameplay. Splash and title both precede the first
        // `OnEnter(Gameplay)`, so spawning visible would show the dome on the very
        // screens this is keeping it off — and only on the first run, which is the
        // worst kind of bug to notice.
        Visibility::Hidden,
        NotShadowCaster,
        // `MeshPickingPlugin` raycasts every `Mesh3d` by default, and the dome's
        // bounding box permanently contains the camera, so the cheap AABB rejection
        // never fires and every pointer move would walk its several thousand
        // triangles. Backface culling means it reports no hit anyway.
        Pickable::IGNORE,
        SkyDome,
        Name::new("Sky Dome"),
    ));
}

/// Sky parameters used for the one frame or two before `LightingSettings` loads.
///
/// Written in linear RGB, because that is what the shader consumes — unlike
/// [`sky_params`], which converts the designer-facing sRGB values. Deliberately close
/// to the shipped sky rather than an alarming colour: the loading screen already
/// blocks on settings, so this is only ever seen briefly, and a garish placeholder
/// would be the more visible bug.
fn default_sky_params() -> SkyParams {
    SkyParams {
        horizon_color: Vec3::new(0.5, 0.6, 0.7),
        cloud_coverage: 0.0,
        zenith_color: Vec3::new(0.2, 0.35, 0.6),
        hex_scale: 8.0,
        cloud_color: Vec3::new(0.9, 0.9, 0.92),
        cloud_softness: 0.1,
        cloud_roundness: 0.5,
        cloud_noise: 0.0,
        sun_direction: Vec3::Y,
        celestial_bodies_enabled: 0.0,
        sun_disc_color: Vec3::ONE,
        sun_angular_radius_radians: 0.0,
        moon_direction: Vec3::NEG_Y,
        moon_angular_radius_radians: 0.0,
        moon_disc_color: Vec3::ONE,
        sun_halo_width_radians: 0.0,
        lower_glow_direction: Vec3::NEG_Y,
        moon_halo_width_radians: 0.0,
        lower_glow_color: Vec3::ZERO,
        sun_halo_strength: 0.0,
        moon_halo_strength: 0.0,
        lower_glow_angular_radius_radians: 0.0,
        lower_glow_strength: 0.0,
        _padding: 0.0,
    }
}

/// Applies generated map framing, or the designer-authored fallback, on every entry.
fn frame_gameplay_camera(
    settings: Res<CameraSettings>,
    hint: Option<Res<MapViewHint>>,
    cameras: Query<(&mut Transform, &mut PanOrbitCamera)>,
) {
    let to_vec3 = |(x, y, z)| Vec3::new(x, y, z);
    let fallback_eye = to_vec3(settings.gameplay_eye);
    let fallback_focus = to_vec3(settings.gameplay_focus);
    let (eye, focus, what) = match hint.as_deref() {
        Some(hint) if hint.is_valid() => (
            to_vec3(hint.eye),
            to_vec3(hint.focus),
            "generated map view hint",
        ),
        Some(_) => {
            warn!("generated map view hint must contain finite, distinct points; using camera.ron");
            (
                fallback_eye,
                fallback_focus,
                "gameplay_eye and gameplay_focus",
            )
        }
        None => (
            fallback_eye,
            fallback_focus,
            "gameplay_eye and gameplay_focus",
        ),
    };
    frame_camera(cameras, eye, focus, what);
}

fn reset_camera_mode(mut mode: ResMut<CameraMode>, mut saved: ResMut<SavedMapCamera>) {
    *mode = CameraMode::Map;
    saved.0 = None;
}

fn map_camera_active(mode: Res<CameraMode>) -> bool {
    *mode == CameraMode::Map
}

/// Snaps between the current free-map pose and a close orbit around the selected unit.
fn toggle_camera_mode(
    keys: Res<ButtonInput<KeyCode>>,
    settings: Res<CameraSettings>,
    mut mode: ResMut<CameraMode>,
    mut saved: ResMut<SavedMapCamera>,
    targets: Query<&Transform, (With<CameraFocusTarget>, Without<PanOrbitCamera>)>,
    mut cameras: Query<(&mut Transform, &mut PanOrbitCamera), Without<CameraFocusTarget>>,
) {
    if !keys.just_pressed(KeyCode::KeyC) {
        return;
    }

    let Ok((mut transform, mut camera)) = cameras.single_mut() else {
        return;
    };
    match *mode {
        CameraMode::Map => {
            let Ok(target) = targets.single() else {
                warn!("cannot enter character camera without exactly one selected focus target");
                return;
            };
            saved.0 = Some(CameraPose::capture(&transform, &camera));

            let wanted_pitch = settings.character_pitch * std::f32::consts::FRAC_PI_2;
            let pitch_delta = wanted_pitch - downward_pitch(transform.rotation);
            transform.rotation = apply_pitch_delta(
                transform.rotation,
                pitch_delta,
                settings.min_pitch,
                settings.max_pitch,
            );

            camera.focus = target.translation + Vec3::Y * settings.character_focus_height;
            camera.radius = settings.character_radius;
            transform.translation = camera.focus
                + Mat3::from_quat(transform.rotation).mul_vec3(Vec3::new(0.0, 0.0, camera.radius));
            *mode = CameraMode::Character;
        }
        CameraMode::Character => {
            if let Some(pose) = saved.0.take() {
                pose.restore(&mut transform, &mut camera);
            }
            *mode = CameraMode::Map;
        }
    }
}

/// Keeps a close orbit centred on the selected unit's rendered position.
fn follow_character_camera(
    mut mode: ResMut<CameraMode>,
    mut saved: ResMut<SavedMapCamera>,
    settings: Res<CameraSettings>,
    targets: Query<&Transform, (With<CameraFocusTarget>, Without<PanOrbitCamera>)>,
    mut cameras: Query<(&mut Transform, &mut PanOrbitCamera), Without<CameraFocusTarget>>,
) {
    if *mode != CameraMode::Character {
        return;
    }
    let Ok((mut transform, mut camera)) = cameras.single_mut() else {
        return;
    };
    let Ok(target) = targets.single() else {
        if let Some(pose) = saved.0.take() {
            pose.restore(&mut transform, &mut camera);
        }
        *mode = CameraMode::Map;
        return;
    };

    let wanted_focus = target.translation + Vec3::Y * settings.character_focus_height;
    let change = wanted_focus - camera.focus;
    if change.length_squared() <= f32::EPSILON {
        return;
    }
    camera.focus = wanted_focus;
    transform.translation += change;
}

/// Reveals the sky when the world does.
fn show_sky(domes: Query<&mut Visibility, With<SkyDome>>) {
    set_sky(domes, Visibility::Visible);
}

/// And hides it again on the way back to the menus.
///
/// The title screen used to inherit wherever the player had orbited to before quitting,
/// so the same menu appeared at a different angle every time. The first fix pointed the
/// camera somewhere fixed, which only chose *which* sky to look at; not drawing it is
/// the answer that leaves nothing to choose.
fn hide_sky(domes: Query<&mut Visibility, With<SkyDome>>) {
    set_sky(domes, Visibility::Hidden);
}

fn set_sky(mut domes: Query<&mut Visibility, With<SkyDome>>, wanted: Visibility) {
    for mut visibility in &mut domes {
        // Guarded for the same reason `follow_camera` guards its write: assigning
        // through `Mut` marks the component changed even when the value is identical,
        // and visibility changes propagate to children.
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
}

/// Points every camera at `focus` from `eye`.
///
/// `what` names the settings being applied, so a bad edit says which pair to look at.
fn frame_camera(
    mut cameras: Query<(&mut Transform, &mut PanOrbitCamera)>,
    eye: Vec3,
    focus: Vec3,
    what: &str,
) {
    let offset = eye - focus;

    if !eye.is_finite() || !focus.is_finite() || offset.length_squared() <= f32::EPSILON {
        warn!("camera.ron: {what} must be finite, distinct points");
        return;
    }

    // Looking straight along the usual up vector is degenerate. The shipped frame
    // is oblique, but choosing a fallback keeps a live settings edit recoverable.
    let direction = offset.normalize();
    let up = if direction.cross(Vec3::Y).length_squared() <= f32::EPSILON {
        Vec3::Z
    } else {
        Vec3::Y
    };

    for (mut transform, mut camera) in &mut cameras {
        transform.translation = eye;
        transform.look_at(focus, up);
        camera.focus = focus;
        camera.radius = offset.length();
    }
}

/// Build sky parameters from settings. `to_color(..).to_linear()` converts the
/// designer-facing sRGB tuples into the linear RGB the shader expects.
pub(crate) fn sky_params(lighting: &ResolvedLighting) -> SkyParams {
    let lin = |rgb: Rgb| {
        let c = to_color(rgb).to_linear();
        Vec3::new(c.red, c.green, c.blue)
    };
    SkyParams {
        horizon_color: lin(lighting.sky_color),
        cloud_coverage: lighting.cloud_coverage,
        zenith_color: lin(lighting.zenith_color),
        hex_scale: lighting.hex_cloud_scale,
        cloud_color: lin(lighting.cloud_color),
        cloud_softness: lighting.cloud_softness,
        cloud_roundness: lighting.cloud_roundness,
        cloud_noise: lighting.cloud_noise,
        sun_direction: lighting.sun_direction,
        celestial_bodies_enabled: if lighting.key_body.is_some() {
            1.0
        } else {
            0.0
        },
        sun_disc_color: lin(lighting.sun_disc_color),
        sun_angular_radius_radians: 0.5 * lighting.sun_angular_diameter_degrees.to_radians(),
        moon_direction: -lighting.sun_direction,
        moon_angular_radius_radians: 0.5 * lighting.moon_angular_diameter_degrees.to_radians(),
        moon_disc_color: lin(lighting.moon_disc_color),
        sun_halo_width_radians: lighting.sun_halo_width_degrees.to_radians(),
        lower_glow_direction: lighting.lower_glow_direction,
        moon_halo_width_radians: lighting.moon_halo_width_degrees.to_radians(),
        lower_glow_color: lin(lighting.lower_glow_color),
        sun_halo_strength: lighting.sun_halo_strength,
        moon_halo_strength: lighting.moon_halo_strength,
        lower_glow_angular_radius_radians: lighting.lower_glow_angular_radius_degrees.to_radians(),
        lower_glow_strength: lighting.lower_glow_strength,
        _padding: 0.0,
    }
}

/// Push a resolved lighting frame into the dome material.
pub(crate) fn apply_sky_material(
    lighting: Res<ResolvedLighting>,
    domes: Query<&MeshMaterial3d<SkyMaterial>, With<SkyDome>>,
    mut materials: ResMut<Assets<SkyMaterial>>,
) {
    for handle in &domes {
        if let Some(mut material) = materials.get_mut(&handle.0) {
            material.params = sky_params(&lighting);
        }
    }
}

/// Keep the dome centred on the camera so the camera never reaches its far wall.
fn follow_camera(
    camera: Query<&Transform, (With<PanOrbitCamera>, Without<SkyDome>)>,
    mut domes: Query<&mut Transform, With<SkyDome>>,
) {
    let Ok(cam) = camera.single() else {
        return;
    };
    for mut dome in &mut domes {
        // Guarded because writing through `Mut` marks the transform changed even when
        // the value is identical, which would re-propagate and re-extract the dome
        // every frame on a still camera — including on the menu screens.
        if dome.translation.distance_squared(cam.translation) > f32::EPSILON {
            dome.translation = cam.translation;
        }
    }
}

// Camera Pan using WASD
fn pan_camera(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    settings: Res<CameraSettings>,
    mut query: Query<(&mut Transform, &mut PanOrbitCamera)>,
) {
    for (mut transform, mut camera) in query.iter_mut() {
        let mut velocity = Vec3::ZERO;
        let local_z = transform.local_z();
        let forward = -Vec3::new(local_z.x, 0., local_z.z);
        let right = Vec3::new(local_z.z, 0., -local_z.x);

        for key in keys.get_pressed() {
            match key {
                KeyCode::KeyW => velocity += forward,
                KeyCode::KeyS => velocity -= forward,
                KeyCode::KeyA => velocity -= right,
                KeyCode::KeyD => velocity += right,
                _ => (),
            }
        }

        velocity = velocity.normalize_or_zero();

        let mut change = velocity * time.delta_secs() * settings.pan_speed;
        // scale velocity with zoom radius
        change *= camera.radius + settings.pan_speed_offset;

        transform.translation += change;
        camera.focus += change;
    }
}

/// Applies one vertical drag while keeping the camera inside its configured pitch arc.
///
/// Pitch is measured as the signed angle downward from the horizon. The settings store
/// the limits as fractions of a quarter-turn, so `0.0` is level and `1.0` is straight
/// down. Integrating the scalar angle before building the quaternion avoids losing
/// which side of straight down a large cursor movement crossed.
fn apply_pitch_delta(rotation: Quat, downward_delta: f32, min_pitch: f32, max_pitch: f32) -> Quat {
    if !downward_delta.is_finite()
        || !min_pitch.is_finite()
        || !max_pitch.is_finite()
        || !(0.0..=1.0).contains(&min_pitch)
        || !(0.0..=1.0).contains(&max_pitch)
        || min_pitch > max_pitch
    {
        return rotation;
    }

    let current = downward_pitch(rotation);
    if !current.is_finite() {
        return rotation;
    }

    let min_angle = min_pitch * std::f32::consts::FRAC_PI_2;
    let max_angle = max_pitch * std::f32::consts::FRAC_PI_2;
    let wanted = current + downward_delta;
    let clamped = wanted.max(min_angle).min(max_angle);

    // A negative local-X rotation pitches the camera downward, so moving from
    // `current` to `clamped` uses their difference in this order.
    rotation * Quat::from_rotation_x(current - clamped)
}

/// Signed angle downward from the horizon for a camera rotation.
fn downward_pitch(rotation: Quat) -> f32 {
    let forward_y = (rotation * Vec3::NEG_Z).y;
    let up_y = (rotation * Vec3::Y).y;
    (-forward_y).atan2(up_y)
}

/// Pan the camera with WASD, zoom with scroll wheel, orbit with right mouse drag.
///
/// Uses `CursorMoved` rather than raw `MouseMotion` because Wayland (and therefore
/// WSL2's default WSLg session) does not deliver `MouseMotion` events while a button
/// is held. `CursorMoved` is button-state-independent on every backend we care about.
fn orbit_camera(
    windows: Query<&Window, With<PrimaryWindow>>,
    mut ev_cursor: MessageReader<CursorMoved>,
    mut ev_scroll: MessageReader<MouseWheel>,
    input_mouse: Res<ButtonInput<MouseButton>>,
    settings: Res<CameraSettings>,
    mut last_cursor: Local<Option<Vec2>>,
    mut query: Query<(&mut PanOrbitCamera, &mut Transform)>,
) {
    let orbit_button = MouseButton::Right;
    let pressed = input_mouse.pressed(orbit_button);

    let mut rotation_move = Vec2::ZERO;
    let mut scroll = 0.0;

    if pressed {
        // Initialize the baseline on the first frame of the press so we don't
        // get a huge jump from wherever the cursor was last frame.
        if last_cursor.is_none() {
            *last_cursor = windows.single().ok().and_then(|w| w.cursor_position());
        }
        for ev in ev_cursor.read() {
            if let Some(prev) = *last_cursor {
                rotation_move += ev.position - prev;
            }
            *last_cursor = Some(ev.position);
        }
    } else {
        // Drop accumulated events so the next press starts clean.
        ev_cursor.clear();
        *last_cursor = None;
    }

    for ev in ev_scroll.read() {
        scroll += ev.y;
    }

    for (mut pan_orbit, mut transform) in query.iter_mut() {
        let mut any = false;
        if rotation_move.length_squared() > 0.0 {
            any = true;
            let window = get_primary_window_size(&windows);
            let delta_x = rotation_move.x / window.x * std::f32::consts::PI * 2.0;
            let delta_y = rotation_move.y / window.y * std::f32::consts::PI;
            let yaw = Quat::from_rotation_y(-delta_x);
            transform.rotation = yaw * transform.rotation; // rotate around global y axis
            transform.rotation = apply_pitch_delta(
                transform.rotation,
                delta_y,
                settings.min_pitch,
                settings.max_pitch,
            );
        } else if scroll.abs() > 0.0 {
            any = true;
            pan_orbit.radius -= scroll * pan_orbit.radius * settings.zoom_sensitivity;
            // dont allow zoom to reach zero or you get stuck
            pan_orbit.radius = f32::max(pan_orbit.radius, settings.min_zoom);
            pan_orbit.radius = f32::min(pan_orbit.radius, settings.max_zoom);
        }

        if any {
            let rot_matrix = Mat3::from_quat(transform.rotation);
            transform.translation =
                pan_orbit.focus + rot_matrix.mul_vec3(Vec3::new(0.0, 0.0, pan_orbit.radius));
        }
    }
}

fn get_primary_window_size(windows: &Query<&Window, With<PrimaryWindow>>) -> Vec2 {
    #[expect(
        clippy::expect_used,
        reason = "the primary window is created by DefaultPlugins before any system \
                  runs; its absence means the app is not running at all"
    )]
    let window = windows
        .single()
        .expect("expected exactly one primary window");
    Vec2::new(window.width(), window.height())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bevy::asset::AssetPlugin;
    use bevy::state::app::StatesPlugin;

    use super::*;

    fn camera_settings() -> CameraSettings {
        CameraSettings {
            gameplay_eye: (0.0, 48.0, 42.0),
            gameplay_focus: (0.0, 6.0, 0.0),
            character_focus_height: 0.4,
            character_radius: 7.0,
            character_pitch: 0.3,
            pan_speed: 0.4,
            pan_speed_offset: 10.0,
            min_pitch: 0.25,
            max_pitch: 0.95,
            min_zoom: 5.0,
            max_zoom: 70.0,
            zoom_sensitivity: 0.2,
        }
    }

    fn enter(app: &mut App, screen: Screen) {
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(screen);
        app.update();
    }

    fn assert_full_map_frame(app: &App, entity: Entity) {
        assert_camera_frame(
            app,
            entity,
            Vec3::new(0.0, 48.0, 42.0),
            Vec3::new(0.0, 6.0, 0.0),
        );
    }

    fn assert_camera_frame(app: &App, entity: Entity, eye: Vec3, focus: Vec3) {
        let transform = app
            .world()
            .entity(entity)
            .get::<Transform>()
            .expect("the camera should have a transform");
        let camera = app
            .world()
            .entity(entity)
            .get::<PanOrbitCamera>()
            .expect("the camera should have pan/orbit state");

        assert!(transform.translation.distance(eye) < 1e-5);
        assert!(camera.focus.distance(focus) < 1e-5);
        assert!((camera.radius - eye.distance(focus)).abs() < 1e-5);
        let forward = transform.forward().as_vec3();
        assert!(forward.dot((focus - eye).normalize()) > 0.9999);
    }

    fn publish_generated_view(mut commands: Commands) {
        commands.insert_resource(MapViewHint::new((12.0, 36.0, -18.0), (2.0, 5.0, -1.0)));
    }

    fn rotation_at_pitch(angle: f32) -> Quat {
        Quat::from_rotation_x(-angle)
    }

    fn assert_pitch(rotation: Quat, expected: f32) {
        let actual = downward_pitch(rotation);
        assert!(
            (actual - expected).abs() < 1e-5,
            "expected pitch {expected}, got {actual}"
        );
    }

    #[test]
    fn pitch_delta_clamps_to_angular_limits() {
        let min_pitch = 0.25;
        let max_pitch = 0.95;
        let min_angle = min_pitch * std::f32::consts::FRAC_PI_2;
        let max_angle = max_pitch * std::f32::consts::FRAC_PI_2;
        let middle = 0.5 * std::f32::consts::FRAC_PI_2;

        assert_pitch(
            apply_pitch_delta(rotation_at_pitch(middle), -10.0, min_pitch, max_pitch),
            min_angle,
        );
        assert_pitch(
            apply_pitch_delta(rotation_at_pitch(middle), 10.0, min_pitch, max_pitch),
            max_angle,
        );
        assert_pitch(
            apply_pitch_delta(rotation_at_pitch(middle), 0.1, min_pitch, max_pitch),
            middle + 0.1,
        );
    }

    #[test]
    fn pitch_delta_cannot_flip_across_straight_down() {
        let min_pitch = 0.25;
        let max_pitch = 0.95;
        let middle = 0.5 * std::f32::consts::FRAC_PI_2;

        // One full-window cursor jump produces a PI-radian delta. Applying that raw
        // before measuring the tilt used to cross straight down and leave the camera
        // inverted instead of at its lower-looking limit.
        let rotation = apply_pitch_delta(
            rotation_at_pitch(middle),
            std::f32::consts::PI,
            min_pitch,
            max_pitch,
        );

        assert_pitch(rotation, max_pitch * std::f32::consts::FRAC_PI_2);
        assert!((rotation * Vec3::Y).y > 0.0, "the camera ended upside down");
    }

    #[test]
    fn pitch_delta_preserves_yaw() {
        let yaw = 1.1;
        let middle = 0.5 * std::f32::consts::FRAC_PI_2;
        let before = Quat::from_rotation_y(yaw) * rotation_at_pitch(middle);
        let after = apply_pitch_delta(before, 0.2, 0.25, 0.95);
        let before_heading = (before * Vec3::NEG_Z).xz().normalize();
        let after_heading = (after * Vec3::NEG_Z).xz().normalize();

        assert!(
            before_heading.dot(after_heading) > 0.9999,
            "local pitch changed the camera's yaw"
        );
    }

    #[test]
    fn gameplay_entry_frames_the_map_every_time() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin));
        app.init_state::<Screen>();
        app.insert_resource(camera_settings());
        app.init_resource::<CameraMode>();
        app.init_resource::<SavedMapCamera>();
        app.add_systems(
            OnEnter(Screen::Gameplay),
            (reset_camera_mode, frame_gameplay_camera).chain(),
        );
        let entity = app
            .world_mut()
            .spawn((
                Transform::from_xyz(1.0, 2.0, 3.0),
                PanOrbitCamera::default(),
            ))
            .id();

        enter(&mut app, Screen::Gameplay);
        assert_full_map_frame(&app, entity);

        enter(&mut app, Screen::Title);
        *app.world_mut().resource_mut::<CameraMode>() = CameraMode::Character;
        let saved_pose = CameraPose::capture(
            app.world()
                .entity(entity)
                .get::<Transform>()
                .expect("the camera should have a transform"),
            app.world()
                .entity(entity)
                .get::<PanOrbitCamera>()
                .expect("the camera should have pan/orbit state"),
        );
        app.world_mut().resource_mut::<SavedMapCamera>().0 = Some(saved_pose);
        {
            let mut entity_mut = app.world_mut().entity_mut(entity);
            entity_mut
                .get_mut::<Transform>()
                .expect("the camera should have a transform")
                .translation = Vec3::splat(-50.0);
            let mut camera = entity_mut
                .get_mut::<PanOrbitCamera>()
                .expect("the camera should have pan/orbit state");
            camera.focus = Vec3::splat(20.0);
            camera.radius = 2.0;
        }

        enter(&mut app, Screen::Gameplay);
        assert_full_map_frame(&app, entity);
        assert_eq!(*app.world().resource::<CameraMode>(), CameraMode::Map);
        assert!(app.world().resource::<SavedMapCamera>().0.is_none());
    }

    #[test]
    fn generated_view_published_in_resources_wins_in_view() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin));
        app.init_state::<Screen>();
        app.insert_resource(camera_settings());
        app.configure_sets(
            OnEnter(Screen::Gameplay),
            (
                GameplaySetup::Resources,
                GameplaySetup::Terrain,
                GameplaySetup::Actors,
                GameplaySetup::View,
                GameplaySetup::Finalize,
            )
                .chain(),
        );
        app.add_systems(
            OnEnter(Screen::Gameplay),
            publish_generated_view.in_set(GameplaySetup::Resources),
        );
        app.add_systems(
            OnEnter(Screen::Gameplay),
            frame_gameplay_camera.in_set(GameplaySetup::View),
        );
        let entity = app
            .world_mut()
            .spawn((
                Transform::from_xyz(1.0, 2.0, 3.0),
                PanOrbitCamera::default(),
            ))
            .id();

        enter(&mut app, Screen::Gameplay);

        assert_camera_frame(
            &app,
            entity,
            Vec3::new(12.0, 36.0, -18.0),
            Vec3::new(2.0, 5.0, -1.0),
        );
    }

    #[test]
    fn invalid_generated_view_uses_camera_settings() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin));
        app.init_state::<Screen>();
        app.insert_resource(camera_settings());
        app.insert_resource(MapViewHint::new((0.0, 0.0, 0.0), (0.0, 0.0, 0.0)));
        app.add_systems(OnEnter(Screen::Gameplay), frame_gameplay_camera);
        let entity = app
            .world_mut()
            .spawn((
                Transform::from_xyz(1.0, 2.0, 3.0),
                PanOrbitCamera::default(),
            ))
            .id();

        enter(&mut app, Screen::Gameplay);

        assert_full_map_frame(&app, entity);
    }

    fn prototype_camera_app(target: Option<Vec3>) -> (App, Entity, Option<Entity>) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(camera_settings());
        app.insert_resource(ButtonInput::<KeyCode>::default());
        app.init_resource::<CameraMode>();
        app.init_resource::<SavedMapCamera>();
        app.add_systems(Update, toggle_camera_mode);
        app.add_systems(PostUpdate, follow_character_camera);

        let eye = Vec3::new(0.0, 48.0, 42.0);
        let focus = Vec3::new(0.0, 6.0, 0.0);
        let camera = app
            .world_mut()
            .spawn((
                Transform::from_translation(eye).looking_at(focus, Vec3::Y),
                PanOrbitCamera {
                    focus,
                    radius: eye.distance(focus),
                },
            ))
            .id();
        let target = target.map(|translation| {
            app.world_mut()
                .spawn((Transform::from_translation(translation), CameraFocusTarget))
                .id()
        });
        (app, camera, target)
    }

    fn toggle_camera(app: &mut App) {
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyC);
        app.update();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .reset(KeyCode::KeyC);
    }

    fn camera_pose(app: &App, entity: Entity) -> (Transform, Vec3, f32) {
        let entity = app.world().entity(entity);
        let transform = *entity
            .get::<Transform>()
            .expect("the camera should have a transform");
        let camera = entity
            .get::<PanOrbitCamera>()
            .expect("the camera should have pan/orbit state");
        (transform, camera.focus, camera.radius)
    }

    #[test]
    fn character_camera_snaps_close_and_restores_the_exact_map_pose() {
        let target = Vec3::new(3.0, 2.0, -1.0);
        let (mut app, camera, _) = prototype_camera_app(Some(target));
        let original = camera_pose(&app, camera);
        let original_heading = (original.0.rotation * Vec3::NEG_Z).xz().normalize();

        toggle_camera(&mut app);

        assert_eq!(*app.world().resource::<CameraMode>(), CameraMode::Character);
        let close = camera_pose(&app, camera);
        let expected_focus = target + Vec3::Y * camera_settings().character_focus_height;
        assert!(close.1.distance(expected_focus) < 1e-5);
        assert!((close.2 - camera_settings().character_radius).abs() < f32::EPSILON);
        assert_pitch(
            close.0.rotation,
            camera_settings().character_pitch * std::f32::consts::FRAC_PI_2,
        );
        let close_heading = (close.0.rotation * Vec3::NEG_Z).xz().normalize();
        assert!(original_heading.dot(close_heading) > 0.9999);
        assert!(
            close
                .0
                .forward()
                .as_vec3()
                .dot((expected_focus - close.0.translation).normalize())
                > 0.9999
        );

        {
            let mut entity = app.world_mut().entity_mut(camera);
            entity
                .get_mut::<Transform>()
                .expect("the camera should have a transform")
                .translation = Vec3::splat(-20.0);
            let mut orbit = entity
                .get_mut::<PanOrbitCamera>()
                .expect("the camera should have orbit state");
            orbit.focus = Vec3::splat(8.0);
            orbit.radius = 5.5;
        }
        toggle_camera(&mut app);

        assert_eq!(*app.world().resource::<CameraMode>(), CameraMode::Map);
        let restored = camera_pose(&app, camera);
        assert_eq!(restored.0, original.0);
        assert_eq!(restored.1, original.1);
        assert!((restored.2 - original.2).abs() < f32::EPSILON);
    }

    #[test]
    fn character_camera_follows_movement_and_a_new_focus_target() {
        let start = Vec3::new(2.0, 1.0, -3.0);
        let (mut app, camera, target) = prototype_camera_app(Some(start));
        let target = target.expect("the fixture should spawn a target");
        toggle_camera(&mut app);
        let before = camera_pose(&app, camera);

        let movement = Vec3::new(1.5, 0.4, -2.0);
        app.world_mut()
            .entity_mut(target)
            .get_mut::<Transform>()
            .expect("the target should have a transform")
            .translation += movement;
        app.update();

        let moved = camera_pose(&app, camera);
        assert!(
            moved
                .0
                .translation
                .distance(before.0.translation + movement)
                < 1e-5
        );
        assert!(moved.1.distance(before.1 + movement) < 1e-5);
        assert!((moved.2 - before.2).abs() < f32::EPSILON);
        assert_eq!(moved.0.rotation, before.0.rotation);

        app.world_mut()
            .entity_mut(target)
            .remove::<CameraFocusTarget>();
        let replacement_position = Vec3::new(-4.0, 3.0, 6.0);
        app.world_mut().spawn((
            Transform::from_translation(replacement_position),
            CameraFocusTarget,
        ));
        let eye_offset = moved.0.translation - moved.1;
        app.update();

        let retargeted = camera_pose(&app, camera);
        let expected_focus =
            replacement_position + Vec3::Y * camera_settings().character_focus_height;
        assert!(retargeted.1.distance(expected_focus) < 1e-5);
        assert!(
            retargeted
                .0
                .translation
                .distance(expected_focus + eye_offset)
                < 1e-5
        );
    }

    fn move_focus_target_in_update(mut targets: Query<&mut Transform, With<CameraFocusTarget>>) {
        for mut target in &mut targets {
            target.translation += Vec3::new(1.5, 0.4, -2.0);
        }
    }

    #[test]
    fn post_update_follow_observes_target_movement_from_the_same_frame() {
        let (mut app, camera, _) = prototype_camera_app(Some(Vec3::new(2.0, 1.0, -3.0)));
        toggle_camera(&mut app);
        let before = camera_pose(&app, camera);
        let movement = Vec3::new(1.5, 0.4, -2.0);
        app.add_systems(Update, move_focus_target_in_update);

        app.update();

        let after = camera_pose(&app, camera);
        assert!(
            after
                .0
                .translation
                .distance(before.0.translation + movement)
                < 1e-5
        );
        assert!(after.1.distance(before.1 + movement) < 1e-5);
    }

    #[test]
    fn wasd_pan_runs_only_in_map_mode() {
        let mut app = App::new();
        app.insert_resource(camera_settings());
        app.insert_resource(ButtonInput::<KeyCode>::default());
        let mut time = Time::<()>::default();
        time.advance_by(Duration::from_secs(1));
        app.insert_resource(time);
        app.init_resource::<CameraMode>();
        app.add_systems(Update, pan_camera.run_if(map_camera_active));
        let camera = app
            .world_mut()
            .spawn((
                Transform::from_xyz(0.0, 7.0, 7.0).looking_at(Vec3::ZERO, Vec3::Y),
                PanOrbitCamera {
                    focus: Vec3::ZERO,
                    radius: 10.0,
                },
            ))
            .id();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyW);
        *app.world_mut().resource_mut::<CameraMode>() = CameraMode::Character;
        let before = camera_pose(&app, camera);

        app.update();

        let character = camera_pose(&app, camera);
        assert_eq!(character.0, before.0);
        assert_eq!(character.1, before.1);

        *app.world_mut().resource_mut::<CameraMode>() = CameraMode::Map;
        app.update();

        let map = camera_pose(&app, camera);
        assert_ne!(map.0.translation, before.0.translation);
        assert_ne!(map.1, before.1);
        assert_eq!(
            map.0.translation - before.0.translation,
            map.1 - before.1,
            "panning should translate the eye and focus together"
        );
    }

    #[test]
    fn missing_focus_target_leaves_the_map_camera_unchanged() {
        let (mut app, camera, _) = prototype_camera_app(None);
        let before = camera_pose(&app, camera);

        toggle_camera(&mut app);

        assert_eq!(*app.world().resource::<CameraMode>(), CameraMode::Map);
        let after = camera_pose(&app, camera);
        assert_eq!(after.0, before.0);
        assert_eq!(after.1, before.1);
        assert!((after.2 - before.2).abs() < f32::EPSILON);
        assert!(app.world().resource::<SavedMapCamera>().0.is_none());
    }

    #[test]
    fn losing_the_focus_target_restores_the_saved_map_pose() {
        let (mut app, camera, target) = prototype_camera_app(Some(Vec3::new(2.0, 1.0, -3.0)));
        let target = target.expect("the fixture should spawn a target");
        let map_pose = camera_pose(&app, camera);
        toggle_camera(&mut app);

        app.world_mut().entity_mut(target).despawn();
        app.update();

        assert_eq!(*app.world().resource::<CameraMode>(), CameraMode::Map);
        let restored = camera_pose(&app, camera);
        assert_eq!(restored.0, map_pose.0);
        assert_eq!(restored.1, map_pose.1);
        assert!((restored.2 - map_pose.2).abs() < f32::EPSILON);
        assert!(app.world().resource::<SavedMapCamera>().0.is_none());
    }

    /// The sky is drawn in the world and nowhere else.
    ///
    /// Reported from play as the menu appearing "at a random zoom of the scenario".
    /// The first fix pointed the camera somewhere fixed, which only chose *which* sky
    /// to look at; not drawing it at all leaves nothing to choose, and the menu is the
    /// flat `ClearColor` instead.
    ///
    /// **Drives the whole plugin, not the system.** The predecessor of this test
    /// registered its system by hand, so it proved a function worked and said nothing
    /// about whether anything called it — and "nothing called it" was the entire defect
    /// it had been written for.
    #[test]
    fn the_sky_belongs_to_gameplay() {
        let mut app = sky_app();

        // Before any gameplay at all. Splash and title both precede the first
        // `OnEnter(Gameplay)`, so this is the one pass where the dome has never been
        // shown — and the only one a first-run bug would show up in.
        assert_eq!(
            dome_visibility(&mut app),
            Some(Visibility::Hidden),
            "the dome was visible before gameplay had ever started"
        );

        enter(&mut app, Screen::Gameplay);
        assert_eq!(
            dome_visibility(&mut app),
            Some(Visibility::Visible),
            "the world has no sky"
        );

        enter(&mut app, Screen::Title);
        assert_eq!(
            dome_visibility(&mut app),
            Some(Visibility::Hidden),
            "the sky followed the player back to the menu"
        );

        // Round again, because the bug this replaces was specifically about returning.
        enter(&mut app, Screen::Gameplay);
        assert_eq!(dome_visibility(&mut app), Some(Visibility::Visible));
        enter(&mut app, Screen::Title);
        assert_eq!(dome_visibility(&mut app), Some(Visibility::Hidden));
    }

    /// An app running the real camera plugin, with everything that plugin declares.
    ///
    /// It also carries orbit, pan and the sky material, so this has to supply what
    /// those ask for even though they never run here: input for mouse and keyboard,
    /// windowing for `CursorMoved`, assets for the dome mesh and its material. A
    /// missing message or resource is a panic, not a skipped system.
    fn sky_app() -> App {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            StatesPlugin,
            bevy::input::InputPlugin,
            bevy::window::WindowPlugin::default(),
        ));
        app.init_asset::<Mesh>();
        app.add_plugins(crate::sky_material::plugin);
        app.init_state::<Screen>();
        app.insert_resource(camera_settings());
        app.add_plugins(super::plugin);
        // `spawn_camera` is on `Startup`, so the dome does not exist until a frame has
        // run.
        app.update();
        app
    }

    fn dome_visibility(app: &mut App) -> Option<Visibility> {
        let mut domes = app
            .world_mut()
            .query_filtered::<&Visibility, With<SkyDome>>();
        domes.iter(app.world()).next().copied()
    }

    /// Reaching the title screen before `camera.ron` has parsed must not take the game
    /// down.
    ///
    /// The title screen arrives on a wall-clock timer rather than a load gate, so it
    /// really is reachable before the settings exist. This project has shipped that
    /// crash once already, on this very screen.
    #[test]
    fn the_title_screen_survives_missing_settings() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            StatesPlugin,
            bevy::input::InputPlugin,
            bevy::window::WindowPlugin::default(),
        ));
        app.init_asset::<Mesh>();
        app.add_plugins(crate::sky_material::plugin);
        app.init_state::<Screen>();
        // No `CameraSettings` on purpose.
        app.add_plugins(super::plugin);

        enter(&mut app, Screen::Title);
    }
}
