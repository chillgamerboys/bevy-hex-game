use bevy::prelude::*;
use bevy::core_pipeline::Skybox;
use bevy::input::mouse::MouseWheel;
use bevy::render::render_resource::{TextureViewDescriptor, TextureViewDimension};
use bevy::window::{CursorMoved, PrimaryWindow};

use crate::plugins::world_3d::config::*;

const SKYBOX_PATH: &str = "textures/sky_boxes/Ryfjallet_cubemap.png";

pub struct CameraPlugin;
impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<PanOrbitCamera>()
            .add_systems(Startup, spawn_camera)
            .add_systems(Update, (reinterpret_skybox_when_loaded, orbit_camera, pan_camera));
    }
}


/// Tags an entity as capable of panning and orbiting.
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct PanOrbitCamera {
    /// The "focus point" to orbit around. It is automatically updated when panning the camera
    pub focus: Vec3,
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

/// Tracks whether the stacked-2D skybox PNG has been reinterpreted as a cubemap yet.
#[derive(Component)]
struct SkyboxNeedsReinterpret;

/// Spawn the game camera with a built-in cubemap skybox.
fn spawn_camera(mut commands: Commands, asset_server: Res<AssetServer>) {
    let translation = Vec3::new(0., 20., 10.0);
    let radius = translation.length();

    let skybox_handle: Handle<Image> = asset_server.load(SKYBOX_PATH);

    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(translation).looking_at(Vec3::ZERO, Vec3::Y),
        PanOrbitCamera {
            radius,
            ..Default::default()
        },
        Name::new("Game Camera"),
        Skybox {
            image: skybox_handle,
            brightness: SKYBOX_BRIGHTNESS,
            ..default()
        },
        SkyboxNeedsReinterpret,
    ));
}

/// PNGs do not carry cubemap metadata, so they load as a single stacked 2D texture.
/// Once the asset finishes loading, reinterpret it as a cube array texture.
fn reinterpret_skybox_when_loaded(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    cameras: Query<(Entity, &Skybox), With<SkyboxNeedsReinterpret>>,
) {
    for (entity, skybox) in cameras.iter() {
        let Some(image) = images.get_mut(&skybox.image) else { continue };
        if image.texture_descriptor.array_layer_count() == 1 {
            image
                .reinterpret_stacked_2d_as_array(image.height() / image.width())
                .expect("skybox PNG should be a vertical stack of cube faces");
            image.texture_view_descriptor = Some(TextureViewDescriptor {
                dimension: Some(TextureViewDimension::Cube),
                ..default()
            });
        }
        commands.entity(entity).remove::<SkyboxNeedsReinterpret>();
    }
}

// Camera Pan using WASD
fn pan_camera(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
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

        let mut change = velocity * time.delta_secs() * CAMERA_SPEED;
        // scale velocity with zoom radius
        change *= camera.radius + CAMERA_SPEED_OFFSET;

        transform.translation += change;
        camera.focus += change;
    }
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
            let pitch = Quat::from_rotation_x(-delta_y);
            transform.rotation = yaw * transform.rotation; // rotate around global y axis
            transform.rotation *= pitch; // rotate around local x axis

            // assert pitch limits
            let mut tilt = (transform.rotation * Vec3::Y).y;
            let below_board = (transform.rotation * Vec3::Z).y < 0.0;
            if below_board {tilt = 2. - tilt;}
            let mut adjustment = 0.0;
            if tilt < MIN_PITCH {
                adjustment = MIN_PITCH - tilt;
            } else if tilt > MAX_PITCH {
                adjustment = MAX_PITCH - tilt;
            } //TODO: max down tilt is a little buggy
            let adjustment = Quat::from_rotation_x(adjustment);
            transform.rotation *= adjustment;
        } else if scroll.abs() > 0.0 {
            any = true;
            pan_orbit.radius -= scroll * pan_orbit.radius * 0.2;
            // dont allow zoom to reach zero or you get stuck
            pan_orbit.radius = f32::max(pan_orbit.radius, MAX_ZOOM_IN);
            pan_orbit.radius = f32::min(pan_orbit.radius, MAX_ZOOM_OUT);
        }

        if any {
            let rot_matrix = Mat3::from_quat(transform.rotation);
            transform.translation = pan_orbit.focus + rot_matrix.mul_vec3(Vec3::new(0.0, 0.0, pan_orbit.radius));
        }
    }
}

fn get_primary_window_size(windows: &Query<&Window, With<PrimaryWindow>>) -> Vec2 {
    let window = windows.single().expect("expected exactly one primary window");
    Vec2::new(window.width(), window.height())
}
