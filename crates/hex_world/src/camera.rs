use bevy::core_pipeline::Skybox;
use bevy::input::mouse::MouseWheel;
use bevy::prelude::*;
use bevy::render::render_resource::{TextureViewDescriptor, TextureViewDimension};
use bevy::window::{CursorMoved, PrimaryWindow};

use hex_assets::{CameraSettings, GameAssets, LightingSettings};
use hex_core::{AppSystems, Screen};

/// Registers the pan/orbit camera and the skybox.
pub fn plugin(app: &mut App) {
    app.register_type::<PanOrbitCamera>()
        // Spawned once at startup rather than per screen: it is the render target
        // the UI screens draw through, and the skybox behind them.
        .add_systems(Startup, spawn_camera)
        .add_systems(OnEnter(Screen::Gameplay), frame_gameplay_camera)
        .add_systems(Update, apply_skybox_brightness.in_set(AppSystems::Update))
        .add_systems(
            Update,
            reinterpret_skybox_when_loaded.in_set(AppSystems::Update),
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

/// Spawn the game camera with a built-in cubemap skybox.
fn spawn_camera(mut commands: Commands, assets: Res<GameAssets>) {
    let translation = Vec3::new(0., 20., 10.0);
    let radius = translation.length();

    let skybox_handle = assets.skybox.clone();

    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(translation).looking_at(Vec3::ZERO, Vec3::Y),
        PanOrbitCamera {
            radius,
            ..Default::default()
        },
        Name::new("Game Camera"),
        // Brightness is set by `apply_skybox_brightness` once settings load; the
        // camera itself has to exist from startup as the UI's render target.
        Skybox {
            image: Some(skybox_handle),
            ..default()
        },
    ));
}

/// Restores the designer-authored full-map view whenever gameplay begins.
fn frame_gameplay_camera(
    settings: Res<CameraSettings>,
    mut cameras: Query<(&mut Transform, &mut PanOrbitCamera)>,
) {
    let (eye_x, eye_y, eye_z) = settings.gameplay_eye;
    let (focus_x, focus_y, focus_z) = settings.gameplay_focus;
    let eye = Vec3::new(eye_x, eye_y, eye_z);
    let focus = Vec3::new(focus_x, focus_y, focus_z);
    let offset = eye - focus;

    if !eye.is_finite() || !focus.is_finite() || offset.length_squared() <= f32::EPSILON {
        warn!("camera.ron: gameplay_eye and gameplay_focus must be finite, distinct points");
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

/// Applies skybox brightness from settings, and keeps it in step if the file is
/// edited while running.
fn apply_skybox_brightness(
    settings: Option<Res<LightingSettings>>,
    mut skyboxes: Query<&mut Skybox>,
) {
    let Some(settings) = settings else { return };
    if !settings.is_changed() {
        return;
    }
    for mut skybox in &mut skyboxes {
        skybox.brightness = settings.skybox_brightness;
    }
}

/// PNGs do not carry cubemap metadata, so they load as a single stacked 2D texture.
/// Reinterpret it as a cube array texture the moment the image finishes loading.
///
/// Driven by `AssetEvent` rather than by polling a marker component: the load happens
/// once, but the old query-every-frame version kept scanning for the rest of the run.
/// Reacting to the event means the body only executes on the frame the asset lands.
fn reinterpret_skybox_when_loaded(
    mut asset_events: MessageReader<AssetEvent<Image>>,
    mut images: ResMut<Assets<Image>>,
    cameras: Query<&Skybox>,
) {
    for event in asset_events.read() {
        let AssetEvent::LoadedWithDependencies { id } = event else {
            continue;
        };

        // Images load for all sorts of reasons; only touch one a camera uses as a skybox.
        let is_skybox = cameras.iter().any(|skybox| {
            skybox
                .image
                .as_ref()
                .is_some_and(|handle| handle.id() == *id)
        });
        if !is_skybox {
            continue;
        }

        let Some(mut image) = images.get_mut(*id) else {
            continue;
        };
        if image.texture_descriptor.array_layer_count() == 1 {
            // Bind the layer count first: `Assets::get_mut` hands back a change-detection
            // `AssetMut` wrapper, so reading `image` inside the call's argument list would
            // overlap with the mutable borrow the call itself takes.
            let layers = image.height() / image.width();
            #[expect(
                clippy::expect_used,
                reason = "the skybox asset ships with the game; a PNG that is not a \
                          vertical stack of six square faces is a broken build, and \
                          failing loudly beats rendering a black sky with no \
                          explanation"
            )]
            image
                .reinterpret_stacked_2d_as_array(layers)
                .expect("skybox PNG should be a vertical stack of cube faces");
            image.texture_view_descriptor = Some(TextureViewDescriptor {
                dimension: Some(TextureViewDimension::Cube),
                ..default()
            });
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

#[cfg(test)]
mod tests {
    use bevy::state::app::StatesPlugin;

    use super::*;

    fn camera_settings() -> CameraSettings {
        CameraSettings {
            gameplay_eye: (0.0, 44.0, 38.0),
            gameplay_focus: (0.0, 6.0, 0.0),
            pan_speed: 0.4,
            pan_speed_offset: 10.0,
            min_pitch: 0.25,
            max_pitch: 0.95,
            min_zoom: 5.0,
            max_zoom: 60.0,
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
        let eye = Vec3::new(0.0, 44.0, 38.0);
        let focus = Vec3::new(0.0, 6.0, 0.0);

        assert!(transform.translation.distance(eye) < 1e-5);
        assert!(camera.focus.distance(focus) < 1e-5);
        assert!((camera.radius - eye.distance(focus)).abs() < 1e-5);
        let forward = transform.forward().as_vec3();
        assert!(forward.dot((focus - eye).normalize()) > 0.9999);
    }

    #[test]
    fn gameplay_entry_frames_the_map_every_time() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin));
        app.init_state::<Screen>();
        app.insert_resource(camera_settings());
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

        enter(&mut app, Screen::Title);
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
    }
}
