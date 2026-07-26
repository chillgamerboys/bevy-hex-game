use bevy::input::mouse::MouseWheel;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy::window::{CursorMoved, PrimaryWindow};

use hex_assets::{to_color, CameraSettings, LightingSettings, Rgb};
use hex_core::{AppSystems, Screen};

use crate::sky_material::{SkyMaterial, SkyParams};

/// Sky-dome radius, in world units. Comfortably inside the camera's default
/// 1000-unit far plane and far outside `max_zoom` (50) plus the terrain extent.
const SKY_DOME_RADIUS: f32 = 500.0;

/// Marks the sky-dome entity so `follow_camera` can pin it to the camera.
#[derive(Component)]
struct SkyDome;

/// Registers the pan/orbit camera and the procedural sky.
pub fn plugin(app: &mut App) {
    app.register_type::<PanOrbitCamera>()
        // Spawned once at startup rather than per screen: it is the render target
        // the UI screens draw through, and the sky behind them.
        .add_systems(Startup, spawn_camera)
        .add_systems(
            Update,
            (apply_sky_material, follow_camera).in_set(AppSystems::Update),
        )
        // Camera control is gameplay-only, so dragging over a menu does not
        // silently move the world behind it.
        .add_systems(
            Update,
            (orbit_camera, pan_camera)
                .in_set(AppSystems::RecordInput)
                .run_if(in_state(Screen::Gameplay)),
        );
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
        Mesh3d(meshes.add(Sphere::new(1.0).mesh().uv(32, 18))),
        MeshMaterial3d(sky_materials.add(SkyMaterial {
            params: default_sky_params(),
        })),
        Transform::from_scale(Vec3::splat(SKY_DOME_RADIUS)),
        NotShadowCaster,
        SkyDome,
        Name::new("Sky Dome"),
    ));
}

/// Placeholder sky parameters used until `LightingSettings` loads. Muted values so a
/// stalled settings load is obviously wrong rather than looking intentional.
fn default_sky_params() -> SkyParams {
    SkyParams {
        horizon_color: Vec3::new(0.5, 0.6, 0.7),
        cloud_coverage: 0.0,
        zenith_color: Vec3::new(0.2, 0.35, 0.6),
        hex_scale: 8.0,
        cloud_color: Vec3::new(0.9, 0.9, 0.92),
        cloud_softness: 0.1,
    }
}

/// Build sky parameters from settings. `to_color(..).to_linear()` converts the
/// designer-facing sRGB tuples into the linear RGB the shader expects.
fn sky_params(settings: &LightingSettings) -> SkyParams {
    let lin = |rgb: Rgb| {
        let c = to_color(rgb).to_linear();
        Vec3::new(c.red, c.green, c.blue)
    };
    SkyParams {
        horizon_color: lin(settings.sky_color),
        cloud_coverage: settings.cloud_coverage,
        zenith_color: lin(settings.zenith_color),
        hex_scale: settings.hex_cloud_scale,
        cloud_color: lin(settings.cloud_color),
        cloud_softness: settings.cloud_softness,
    }
}

/// Push sky settings into the dome material, on load and on every hot reload.
fn apply_sky_material(
    settings: Option<Res<LightingSettings>>,
    domes: Query<&MeshMaterial3d<SkyMaterial>, With<SkyDome>>,
    mut materials: ResMut<Assets<SkyMaterial>>,
) {
    let Some(settings) = settings else {
        return;
    };
    if !settings.is_changed() {
        return;
    }
    for handle in &domes {
        if let Some(mut material) = materials.get_mut(&handle.0) {
            material.params = sky_params(&settings);
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
        dome.translation = cam.translation;
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
            let pitch = Quat::from_rotation_x(-delta_y);
            transform.rotation = yaw * transform.rotation; // rotate around global y axis
            transform.rotation *= pitch; // rotate around local x axis

            // assert pitch limits
            let mut tilt = (transform.rotation * Vec3::Y).y;
            let below_board = (transform.rotation * Vec3::Z).y < 0.0;
            if below_board {
                tilt = 2. - tilt;
            }
            let mut adjustment = 0.0;
            if tilt < settings.min_pitch {
                adjustment = settings.min_pitch - tilt;
            } else if tilt > settings.max_pitch {
                adjustment = settings.max_pitch - tilt;
            } //TODO: max down tilt is a little buggy
            let adjustment = Quat::from_rotation_x(adjustment);
            transform.rotation *= adjustment;
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
