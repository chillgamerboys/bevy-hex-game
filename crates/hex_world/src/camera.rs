use std::collections::{BTreeMap, BTreeSet};

use bevy::input::mouse::MouseWheel;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy::transform::TransformSystems;
use bevy::window::{CursorMoved, PrimaryWindow};

use hex_assets::{to_color, CameraSettings, ResolvedLighting, Rgb};
use hex_core::{
    config::HEX_CIRCUMRADIUS, AppSystems, CameraFocusTarget, GameplaySetup, HexSpan, HexTile,
    InputAction, InputBindings, MapViewHint, Screen, TilePos,
};

use crate::{
    sky_material::{SkyMaterial, SkyParams},
    LightingSystems,
};

/// Sky-dome radius, in world units. Comfortably inside the camera's default
/// 1000-unit far plane and far outside the configured zoom range plus the terrain.
const SKY_DOME_RADIUS: f32 = 500.0;

/// Nearby yaw offsets considered when terrain leaves no room for the preferred
/// minimum Character-camera radius.
///
/// Thirty-degree steps cover both hex faces and corners. Positive yaw wins an
/// otherwise exact tie, so a symmetric enclosure cannot make the camera alternate
/// between two equally good directions.
const CHARACTER_CAMERA_YAW_OFFSETS: [f32; 11] = [
    std::f32::consts::FRAC_PI_6,
    -std::f32::consts::FRAC_PI_6,
    std::f32::consts::FRAC_PI_3,
    -std::f32::consts::FRAC_PI_3,
    std::f32::consts::FRAC_PI_2,
    -std::f32::consts::FRAC_PI_2,
    2.0 * std::f32::consts::FRAC_PI_3,
    -2.0 * std::f32::consts::FRAC_PI_3,
    5.0 * std::f32::consts::FRAC_PI_6,
    -5.0 * std::f32::consts::FRAC_PI_6,
    std::f32::consts::PI,
];

/// Clearances within this world-unit tolerance are the same deterministic tie.
const CHARACTER_CAMERA_CLEARANCE_TIE_EPSILON: f32 = 1e-5;

/// Marks the sky-dome entity so `follow_camera` can pin it to the camera.
#[derive(Component, Reflect)]
#[reflect(Component)]
pub(crate) struct SkyDome;

/// Same-frame ordering for the public terrain projection and camera transforms.
///
/// Review tooling uses [`Self::FollowCharacter`] to establish an initial pose
/// before Character-mode collision resolves it. The set carries presentation
/// ordering only; it does not expose terrain ownership or gameplay visibility.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CameraSystems {
    /// Refresh the cached public `HexTile`/`TilePos`/`HexSpan` projection.
    RefreshObstructions,
    /// Follow the selected character and keep the camera outside terrain.
    FollowCharacter,
    /// Pin camera-owned presentation, such as the sky dome, to the final pose.
    FollowPresentation,
}

/// Registers the pan/orbit camera and the procedural sky.
pub fn plugin(app: &mut App) {
    app.register_type::<PanOrbitCamera>()
        .register_type::<CameraMode>()
        .register_type::<MapViewHint>()
        .register_type::<SkyDome>()
        .init_resource::<CameraMode>()
        .init_resource::<SavedMapCamera>()
        .init_resource::<CameraObstructionIndex>()
        .init_resource::<CharacterCameraCollision>()
        .init_resource::<InputBindings>()
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
        .add_systems(
            OnExit(Screen::Gameplay),
            (hide_sky, clear_camera_obstruction_index),
        )
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
            (
                refresh_camera_obstruction_index.in_set(CameraSystems::RefreshObstructions),
                follow_character_camera.in_set(CameraSystems::FollowCharacter),
                follow_camera.in_set(CameraSystems::FollowPresentation),
            )
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

/// One rendered terrain run in the camera's public obstruction projection.
#[derive(Debug, Clone, Copy)]
struct CameraObstruction {
    center: Vec2,
    span: HexSpan,
}

/// Cached presentation-only projection of public terrain geometry.
///
/// It intentionally contains no map-private storage or gameplay visibility facts.
#[derive(Resource, Debug, Default)]
struct CameraObstructionIndex {
    spans_by_coord: BTreeMap<hex_core::HexCoord, Vec<HexSpan>>,
    initialized: bool,
    rebuilds: u64,
}

/// Transient radius used only while Character mode avoids terrain.
///
/// The camera's `Transform` retains any deterministic no-room yaw chosen by
/// collision resolution. Keeping direction in the actual pose means subsequent
/// frames and manual orbit input share one authority instead of fighting a second
/// cached angle.
#[derive(Resource, Debug, Default)]
struct CharacterCameraCollision {
    effective_radius: Option<f32>,
}

#[derive(Debug, Clone, Copy)]
struct CameraClearance {
    radius: f32,
    obstructed: bool,
}

#[derive(Debug, Clone, Copy)]
struct CharacterCameraPath {
    yaw: f32,
    direction: Vec3,
    clearance: CameraClearance,
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

fn reset_camera_mode(
    mut mode: ResMut<CameraMode>,
    mut saved: ResMut<SavedMapCamera>,
    mut collision: ResMut<CharacterCameraCollision>,
) {
    *mode = CameraMode::Map;
    saved.0 = None;
    collision.effective_radius = None;
}

fn map_camera_active(mode: Res<CameraMode>) -> bool {
    *mode == CameraMode::Map
}

fn pitch_limits(mode: CameraMode, settings: &CameraSettings) -> (f32, f32) {
    match mode {
        CameraMode::Map => (settings.min_pitch, settings.max_pitch),
        CameraMode::Character => (settings.character_min_pitch, settings.character_max_pitch),
    }
}

/// Snaps between the current free-map pose and a close orbit around the selected unit.
fn toggle_camera_mode(
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<InputBindings>,
    settings: Res<CameraSettings>,
    mut mode: ResMut<CameraMode>,
    mut saved: ResMut<SavedMapCamera>,
    mut collision: ResMut<CharacterCameraCollision>,
    targets: Query<&Transform, (With<CameraFocusTarget>, Without<PanOrbitCamera>)>,
    mut cameras: Query<(&mut Transform, &mut PanOrbitCamera), Without<CameraFocusTarget>>,
) {
    if !bindings.just_pressed(&keys, InputAction::ToggleCamera) {
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
                settings.character_min_pitch,
                settings.character_max_pitch,
            );

            camera.focus = target.translation + Vec3::Y * settings.character_focus_height;
            camera.radius = settings.character_radius;
            collision.effective_radius = Some(camera.radius);
            transform.translation = camera.focus
                + Mat3::from_quat(transform.rotation).mul_vec3(Vec3::new(0.0, 0.0, camera.radius));
            *mode = CameraMode::Character;
        }
        CameraMode::Character => {
            if let Some(pose) = saved.0.take() {
                pose.restore(&mut transform, &mut camera);
            }
            collision.effective_radius = None;
            *mode = CameraMode::Map;
        }
    }
}

/// Keeps a close orbit centred on the selected unit's rendered position.
fn follow_character_camera(
    mut mode: ResMut<CameraMode>,
    mut saved: ResMut<SavedMapCamera>,
    settings: Res<CameraSettings>,
    time: Res<Time>,
    obstruction_index: Res<CameraObstructionIndex>,
    mut collision: ResMut<CharacterCameraCollision>,
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
        collision.effective_radius = None;
        *mode = CameraMode::Map;
        return;
    };

    let wanted_focus = target.translation + Vec3::Y * settings.character_focus_height;
    if wanted_focus.distance_squared(camera.focus) > f32::EPSILON {
        camera.focus = wanted_focus;
    }

    let eye_direction = transform.rotation * Vec3::Z;
    let path = obstruction_index.character_path(
        wanted_focus,
        eye_direction,
        camera.radius,
        settings.character_collision_margin,
        settings.character_min_effective_radius,
    );
    if path.yaw.abs() > f32::EPSILON {
        transform.rotation = Quat::from_rotation_y(path.yaw) * transform.rotation;
    }
    let previous = collision.effective_radius.unwrap_or(camera.radius);
    let hysteresis = settings.character_collision_margin * 0.25;
    let effective = resolve_effective_radius(
        previous,
        path.clearance.radius,
        path.clearance.obstructed,
        hysteresis,
        settings.character_restoration_speed,
        time.delta_secs(),
    );
    if collision
        .effective_radius
        .is_none_or(|radius| (radius - effective).abs() > f32::EPSILON)
    {
        collision.effective_radius = Some(effective);
    }

    let wanted_eye = wanted_focus + path.direction * effective;
    if transform.translation.distance_squared(wanted_eye) > f32::EPSILON {
        transform.translation = wanted_eye;
    }
}

fn resolve_effective_radius(
    previous: f32,
    safe_radius: f32,
    obstructed: bool,
    hysteresis: f32,
    restoration_speed: f32,
    delta_seconds: f32,
) -> f32 {
    if safe_radius < previous {
        safe_radius
    } else if !obstructed || safe_radius - previous > hysteresis {
        (previous + restoration_speed * delta_seconds).min(safe_radius)
    } else {
        previous
    }
}

impl CameraObstructionIndex {
    fn character_path(
        &self,
        focus: Vec3,
        direction: Vec3,
        desired_radius: f32,
        margin: f32,
        preferred_minimum_radius: f32,
    ) -> CharacterCameraPath {
        let direction = direction.normalize_or_zero();
        let current = self.safe_radius(focus, direction, desired_radius, margin);
        let mut best = CharacterCameraPath {
            yaw: 0.0,
            direction,
            clearance: current,
        };
        if current.radius >= preferred_minimum_radius || direction.length_squared() <= f32::EPSILON
        {
            return best;
        }

        for yaw in CHARACTER_CAMERA_YAW_OFFSETS {
            let candidate_direction = Quat::from_rotation_y(yaw) * direction;
            let clearance = self.safe_radius(focus, candidate_direction, desired_radius, margin);
            let candidate = CharacterCameraPath {
                yaw,
                direction: candidate_direction,
                clearance,
            };
            if clearance.radius >= preferred_minimum_radius {
                return candidate;
            }
            if clearance.radius > best.clearance.radius + CHARACTER_CAMERA_CLEARANCE_TIE_EPSILON {
                best = candidate;
            }
        }
        best
    }

    fn safe_radius(
        &self,
        focus: Vec3,
        direction: Vec3,
        desired_radius: f32,
        margin: f32,
    ) -> CameraClearance {
        if !focus.is_finite()
            || !direction.is_finite()
            || !desired_radius.is_finite()
            || desired_radius <= 0.0
        {
            return CameraClearance {
                radius: desired_radius,
                obstructed: false,
            };
        }
        let direction = direction.normalize_or_zero();
        if direction.length_squared() <= f32::EPSILON {
            return CameraClearance {
                radius: desired_radius,
                obstructed: false,
            };
        }
        let end = focus + direction * desired_radius;
        let candidate_coords = hex_core::HexCoord::from_world(focus)
            .line_between(hex_core::HexCoord::from_world(end))
            .into_iter()
            .flat_map(|coord| coord.within_radius(1))
            .collect::<BTreeSet<_>>();
        let hit = candidate_coords
            .into_iter()
            .filter_map(|coord| self.spans_by_coord.get(&coord).map(|spans| (coord, spans)))
            .flat_map(|(coord, spans)| {
                let center = coord.to_world(0.0).xz();
                spans
                    .iter()
                    .copied()
                    .map(move |span| CameraObstruction { center, span })
            })
            .filter_map(|obstruction| {
                obstruction.first_hit_distance(focus, direction, desired_radius)
            })
            .min_by(f32::total_cmp);
        hit.map_or(
            CameraClearance {
                radius: desired_radius,
                obstructed: false,
            },
            |distance| CameraClearance {
                // The minimum is a preferred usable distance, never permission to
                // cross the closest hit. A no-room ray is handled by bounded yaw
                // search above; a complete enclosure falls back to the best true
                // margin-safe clearance, even when that is below the preference.
                radius: (distance - margin).max(0.0).min(desired_radius),
                obstructed: true,
            },
        )
    }
}

impl CameraObstruction {
    fn first_hit_distance(self, origin: Vec3, direction: Vec3, maximum: f32) -> Option<f32> {
        let horizontal = circle_interval(
            origin.xz(),
            direction.xz(),
            self.center,
            HEX_CIRCUMRADIUS,
            maximum,
        )?;
        let vertical = axis_interval(
            origin.y,
            direction.y,
            self.span.bottom,
            self.span.top,
            maximum,
        )?;
        let enter = horizontal.0.max(vertical.0);
        let exit = horizontal.1.min(vertical.1);
        (enter <= exit).then_some(enter)
    }
}

fn circle_interval(
    origin: Vec2,
    direction: Vec2,
    center: Vec2,
    radius: f32,
    maximum: f32,
) -> Option<(f32, f32)> {
    let offset = origin - center;
    let quadratic = direction.length_squared();
    if quadratic <= f32::EPSILON {
        return (offset.length_squared() <= radius * radius).then_some((0.0, maximum));
    }
    let linear = offset.dot(direction);
    let constant = offset.length_squared() - radius * radius;
    let discriminant = linear * linear - quadratic * constant;
    if discriminant < 0.0 {
        return None;
    }
    let root = discriminant.sqrt();
    let enter = ((-linear - root) / quadratic).max(0.0);
    let exit = ((-linear + root) / quadratic).min(maximum);
    (enter <= exit).then_some((enter, exit))
}

fn axis_interval(
    origin: f32,
    direction: f32,
    minimum: f32,
    maximum_value: f32,
    maximum_distance: f32,
) -> Option<(f32, f32)> {
    if direction.abs() <= f32::EPSILON {
        return (minimum <= origin && origin <= maximum_value).then_some((0.0, maximum_distance));
    }
    let first = (minimum - origin) / direction;
    let second = (maximum_value - origin) / direction;
    let enter = first.min(second).max(0.0);
    let exit = first.max(second).min(maximum_distance);
    (enter <= exit).then_some((enter, exit))
}

fn refresh_camera_obstruction_index(
    mut index: ResMut<CameraObstructionIndex>,
    tiles: Query<(&TilePos, &HexSpan), With<HexTile>>,
    changed_tiles: Query<
        (),
        (
            With<HexTile>,
            Or<(Added<HexTile>, Changed<TilePos>, Changed<HexSpan>)>,
        ),
    >,
    mut removed_tiles: RemovedComponents<HexTile>,
) {
    let removed = removed_tiles.read().next().is_some();
    if index.initialized && !removed && changed_tiles.is_empty() {
        return;
    }

    let mut spans_by_coord = BTreeMap::<_, Vec<_>>::new();
    for (position, span) in &tiles {
        spans_by_coord
            .entry(position.coord)
            .or_default()
            .push(*span);
    }
    for spans in spans_by_coord.values_mut() {
        spans.sort_by(|first, second| {
            first
                .bottom
                .total_cmp(&second.bottom)
                .then_with(|| first.top.total_cmp(&second.top))
        });
    }
    index.spans_by_coord = spans_by_coord;
    index.initialized = true;
    index.rebuilds = index.rebuilds.saturating_add(1);
}

fn clear_camera_obstruction_index(
    mut index: ResMut<CameraObstructionIndex>,
    mut collision: ResMut<CharacterCameraCollision>,
) {
    if index.initialized || !index.spans_by_coord.is_empty() {
        index.spans_by_coord.clear();
        index.initialized = false;
    }
    if collision.effective_radius.is_some() {
        collision.effective_radius = None;
    }
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
    bindings: Res<InputBindings>,
    time: Res<Time>,
    settings: Res<CameraSettings>,
    mut query: Query<(&mut Transform, &mut PanOrbitCamera)>,
) {
    if ![
        InputAction::CameraForward,
        InputAction::CameraBackward,
        InputAction::CameraLeft,
        InputAction::CameraRight,
    ]
    .into_iter()
    .any(|action| bindings.pressed(&keys, action))
    {
        return;
    }

    for (mut transform, mut camera) in query.iter_mut() {
        let mut velocity = Vec3::ZERO;
        let local_z = transform.local_z();
        let forward = -Vec3::new(local_z.x, 0., local_z.z);
        let right = Vec3::new(local_z.z, 0., -local_z.x);

        if bindings.pressed(&keys, InputAction::CameraForward) {
            velocity += forward;
        }
        if bindings.pressed(&keys, InputAction::CameraBackward) {
            velocity -= forward;
        }
        if bindings.pressed(&keys, InputAction::CameraLeft) {
            velocity -= right;
        }
        if bindings.pressed(&keys, InputAction::CameraRight) {
            velocity += right;
        }

        velocity = velocity.normalize_or_zero();
        if velocity.length_squared() <= f32::EPSILON {
            continue;
        }

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
    mode: Res<CameraMode>,
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
            let (min_pitch, max_pitch) = pitch_limits(*mode, &settings);
            transform.rotation =
                apply_pitch_delta(transform.rotation, delta_y, min_pitch, max_pitch);
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
    use std::time::{Duration, Instant};

    use super::*;
    use hex_test_app::HeadlessAppBuilder;

    #[derive(Resource, Default)]
    struct CameraChangeCounts {
        transforms: usize,
        controls: usize,
    }

    fn count_camera_changes(
        cameras: Query<(Ref<Transform>, Ref<PanOrbitCamera>)>,
        mut counts: ResMut<CameraChangeCounts>,
    ) {
        for (transform, controls) in &cameras {
            counts.transforms += usize::from(transform.is_changed());
            counts.controls += usize::from(controls.is_changed());
        }
    }

    fn camera_settings() -> CameraSettings {
        CameraSettings {
            gameplay_eye: (0.0, 48.0, 42.0),
            gameplay_focus: (0.0, 6.0, 0.0),
            character_focus_height: 0.4,
            character_radius: 7.0,
            character_collision_margin: 0.35,
            character_min_effective_radius: 1.5,
            character_restoration_speed: 8.0,
            character_pitch: 0.3,
            character_min_pitch: 0.05,
            character_max_pitch: 0.95,
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

    fn production_scale_obstruction_fixture() -> (CameraObstructionIndex, Vec3) {
        let obstruction_coord = hex_core::HexCoord::from_axial(0, 1);
        let direction = obstruction_coord.to_world(0.0).normalize();
        let mut spans_by_coord = hex_core::HexCoord::ORIGIN
            .within_radius(55)
            .into_iter()
            .map(|coord| (coord, vec![HexSpan::new(0.0, 0.4)]))
            .collect::<BTreeMap<_, _>>();
        spans_by_coord.insert(obstruction_coord, vec![HexSpan::new(0.0, 2.0)]);
        assert_eq!(
            spans_by_coord.len(),
            9_241,
            "the fixture must retain the exact Two Rings column count"
        );
        (
            CameraObstructionIndex {
                spans_by_coord,
                initialized: true,
                rebuilds: 1,
            },
            direction,
        )
    }

    fn timed_character_queries(iterations: usize) -> (CharacterCameraPath, Vec<Duration>) {
        let (index, direction) = production_scale_obstruction_fixture();
        let expected = index.character_path(Vec3::Y, direction, 7.0, 0.35, 1.5);
        let mut timings = Vec::with_capacity(iterations);
        for _ in 0..iterations {
            let started = Instant::now();
            let path = index.character_path(Vec3::Y, direction, 7.0, 0.35, 1.5);
            timings.push(started.elapsed());
            assert!((path.yaw - expected.yaw).abs() < f32::EPSILON);
            assert!((path.clearance.radius - expected.clearance.radius).abs() < f32::EPSILON);
            assert!(path.direction.dot(expected.direction) > 0.9999);
        }
        assert_eq!(index.rebuilds, 1);
        (expected, timings)
    }

    fn timing_percentile(timings: &mut [Duration], percentile: usize) -> Duration {
        timings.sort_unstable();
        let rank = timings
            .len()
            .saturating_mul(percentile)
            .div_ceil(100)
            .saturating_sub(1);
        timings.get(rank).copied().unwrap_or_default()
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
    fn one_hundred_idle_frames_do_not_republish_camera_components() {
        let mut builder = HeadlessAppBuilder::new().with_minimal_plugins();
        builder
            .app_mut()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<InputBindings>()
            .init_resource::<CameraChangeCounts>()
            .insert_resource(camera_settings())
            .add_systems(Update, (pan_camera, count_camera_changes).chain());
        let mut app = builder.build();
        app.world_mut().spawn((
            Transform::from_xyz(0.0, 20.0, 10.0),
            PanOrbitCamera::default(),
        ));

        app.update();
        *app.world_mut().resource_mut::<CameraChangeCounts>() = CameraChangeCounts::default();
        for _ in 0..100 {
            app.update();
        }

        let counts = app.world().resource::<CameraChangeCounts>();
        assert_eq!(counts.transforms, 0);
        assert_eq!(counts.controls, 0);
    }

    #[test]
    fn public_tile_spans_bound_the_nearest_safe_character_radius() {
        let obstruction_coord = hex_core::HexCoord::from_axial(-1, 2);
        let index = CameraObstructionIndex {
            spans_by_coord: BTreeMap::from([(obstruction_coord, vec![HexSpan::new(0.0, 2.0)])]),
            initialized: true,
            rebuilds: 1,
        };

        let clearance = index.safe_radius(Vec3::new(0.0, 1.0, 0.0), Vec3::Z, 7.0, 0.35);

        assert!(clearance.obstructed);
        assert!((clearance.radius - 1.65).abs() < 1e-5);
        let clear = index.safe_radius(Vec3::new(0.0, 3.0, 0.0), Vec3::Z, 7.0, 0.35);
        assert!(!clear.obstructed);
        assert!(
            (clear.radius - 7.0).abs() < f32::EPSILON,
            "a vertically disjoint run must not obstruct the view segment"
        );
    }

    #[test]
    fn nearer_hit_is_never_overridden_by_preferred_minimum_radius() {
        let obstruction_coord = hex_core::HexCoord::from_axial(0, 1);
        let direction = obstruction_coord.to_world(0.0).normalize();
        let index = CameraObstructionIndex {
            spans_by_coord: BTreeMap::from([(obstruction_coord, vec![HexSpan::new(0.0, 2.0)])]),
            initialized: true,
            rebuilds: 1,
        };

        let clearance = index.safe_radius(Vec3::Y, direction, 7.0, 0.35);

        assert!(clearance.obstructed);
        assert!(
            clearance.radius < 1.5,
            "the actual margin-safe clearance must win over the preferred minimum"
        );
        assert!((clearance.radius - (3.0_f32.sqrt() - 1.35)).abs() < 1e-5);
    }

    #[test]
    fn no_room_path_chooses_the_nearest_clear_yaw_with_a_stable_tie_break() {
        let obstruction_coord = hex_core::HexCoord::from_axial(0, 1);
        let direction = obstruction_coord.to_world(0.0).normalize();
        let index = CameraObstructionIndex {
            spans_by_coord: BTreeMap::from([(obstruction_coord, vec![HexSpan::new(0.0, 2.0)])]),
            initialized: true,
            rebuilds: 1,
        };

        let path = index.character_path(Vec3::Y, direction, 7.0, 0.35, 1.5);

        assert!(
            (path.yaw - std::f32::consts::FRAC_PI_3).abs() < f32::EPSILON,
            "positive sixty degrees must win the equal-clearance yaw tie"
        );
        assert!(!path.clearance.obstructed);
        assert!((path.clearance.radius - 7.0).abs() < f32::EPSILON);
        let expected_direction = Quat::from_rotation_y(std::f32::consts::FRAC_PI_3) * direction;
        assert!(path.direction.dot(expected_direction) > 0.9999);
    }

    #[test]
    fn enclosed_path_uses_best_true_clearance_without_penetration() {
        let origin = hex_core::HexCoord::ORIGIN;
        let spans_by_coord = origin
            .neighbors()
            .into_iter()
            .map(|coord| (coord, vec![HexSpan::new(0.0, 2.0)]))
            .collect();
        let direction = hex_core::HexCoord::from_axial(0, 1)
            .to_world(0.0)
            .normalize();
        let index = CameraObstructionIndex {
            spans_by_coord,
            initialized: true,
            rebuilds: 1,
        };

        let path = index.character_path(Vec3::Y, direction, 7.0, 0.35, 1.5);
        let verified = index.safe_radius(Vec3::Y, path.direction, 7.0, 0.35);

        assert!(path.clearance.obstructed);
        assert!(
            path.clearance.radius < 1.5,
            "a full enclosure cannot honestly retain the preferred minimum"
        );
        assert!((path.clearance.radius - verified.radius).abs() < f32::EPSILON);
        assert!(
            (path.yaw - std::f32::consts::FRAC_PI_6).abs() < f32::EPSILON,
            "the first equally best between-column yaw should remain stable"
        );
    }

    #[test]
    fn production_scale_obstruction_queries_are_deterministic() {
        let (path, mut timings) = timed_character_queries(100);
        let p95 = timing_percentile(&mut timings, 95);
        let worst = timings.last().copied().unwrap_or_default();

        assert!(
            (path.yaw - std::f32::consts::FRAC_PI_3).abs() < f32::EPSILON,
            "the production-scale index must preserve the focused yaw result"
        );
        eprintln!(
            "9,241-column Character collision diagnostic (debug): p95={p95:?}, worst={worst:?}"
        );
    }

    #[test]
    #[ignore = "manual release-mode 9,241-column Character-camera timing diagnostic"]
    fn production_scale_character_collision_release_timing() {
        let (_path, mut timings) = timed_character_queries(10_000);
        let p95 = timing_percentile(&mut timings, 95);
        let worst = timings.last().copied().unwrap_or_default();

        eprintln!(
            "9,241-column Character collision diagnostic (release): p95={p95:?}, worst={worst:?}"
        );
        assert!(
            p95 < Duration::from_millis(1),
            "9,241-column Character collision p95 {p95:?} breached the 1 ms release budget"
        );
    }

    #[test]
    fn collision_snaps_inward_and_restores_with_hysteresis() {
        assert!(
            (resolve_effective_radius(6.0, 2.5, true, 0.1, 8.0, 0.25) - 2.5).abs() < f32::EPSILON,
            "new obstructions must shorten the camera immediately"
        );
        assert!(
            (resolve_effective_radius(2.5, 2.55, true, 0.1, 8.0, 0.25) - 2.5).abs() < f32::EPSILON,
            "sub-margin clearance changes must not jitter the camera"
        );
        assert!(
            (resolve_effective_radius(2.5, 7.0, false, 0.1, 8.0, 0.25) - 4.5).abs() < f32::EPSILON,
            "a cleared obstruction should restore at the authored rate"
        );
    }

    #[test]
    fn ten_thousand_stable_frames_do_not_rebuild_or_republish_the_character_camera() {
        let settings = camera_settings();
        let focus = Vec3::Y * settings.character_focus_height;
        let rotation =
            Quat::from_rotation_x(-settings.character_pitch * std::f32::consts::FRAC_PI_2);
        let eye = focus + rotation * Vec3::Z * settings.character_radius;
        let mut builder = HeadlessAppBuilder::new().with_minimal_plugins();
        builder
            .app_mut()
            .insert_resource(settings.clone())
            .insert_resource(CameraMode::Character)
            .init_resource::<SavedMapCamera>()
            .init_resource::<CameraObstructionIndex>()
            .insert_resource(CharacterCameraCollision {
                effective_radius: Some(settings.character_radius),
            })
            .init_resource::<CameraChangeCounts>()
            .add_systems(
                PostUpdate,
                (
                    refresh_camera_obstruction_index,
                    follow_character_camera,
                    count_camera_changes,
                )
                    .chain(),
            );
        let mut app = builder.build();
        app.world_mut().spawn((
            Transform::from_translation(eye).with_rotation(rotation),
            PanOrbitCamera {
                focus,
                radius: settings.character_radius,
            },
        ));
        app.world_mut().spawn((
            Transform::from_translation(Vec3::ZERO),
            CameraFocusTarget::new(TilePos::ORIGIN),
        ));
        let tile = app
            .world_mut()
            .spawn((
                HexTile,
                TilePos::new(hex_core::HexCoord::from_axial(20, 20), 0),
                HexSpan::new(0.0, 0.4),
            ))
            .id();

        app.update();
        assert_eq!(app.world().resource::<CameraObstructionIndex>().rebuilds, 1);
        *app.world_mut().resource_mut::<CameraChangeCounts>() = CameraChangeCounts::default();
        for _ in 0..10_000 {
            app.update();
        }

        let index = app.world().resource::<CameraObstructionIndex>();
        assert_eq!(index.rebuilds, 1);
        let counts = app.world().resource::<CameraChangeCounts>();
        assert_eq!(counts.transforms, 0);
        assert_eq!(counts.controls, 0);

        app.world_mut()
            .entity_mut(tile)
            .insert(HexSpan::new(0.0, 0.8));
        app.update();
        assert_eq!(app.world().resource::<CameraObstructionIndex>().rebuilds, 2);
        app.world_mut().entity_mut(tile).despawn();
        app.update();
        let index = app.world().resource::<CameraObstructionIndex>();
        assert_eq!(index.rebuilds, 3);
        assert!(index.spans_by_coord.is_empty());
    }

    #[test]
    fn resolved_no_room_yaw_stays_stable_for_ten_thousand_frames() {
        let settings = camera_settings();
        let focus = Vec3::Y * settings.character_focus_height;
        let obstruction_coord = hex_core::HexCoord::from_axial(0, 1);
        let initial_direction = obstruction_coord.to_world(0.0).normalize();
        let eye = focus + initial_direction * settings.character_radius;
        let rotation = Transform::from_translation(eye)
            .looking_at(focus, Vec3::Y)
            .rotation;
        let mut builder = HeadlessAppBuilder::new().with_minimal_plugins();
        builder
            .app_mut()
            .insert_resource(settings.clone())
            .insert_resource(CameraMode::Character)
            .init_resource::<SavedMapCamera>()
            .init_resource::<CameraObstructionIndex>()
            .insert_resource(CharacterCameraCollision {
                effective_radius: Some(settings.character_radius),
            })
            .init_resource::<CameraChangeCounts>()
            .add_systems(
                PostUpdate,
                (
                    refresh_camera_obstruction_index,
                    follow_character_camera,
                    count_camera_changes,
                )
                    .chain(),
            );
        let mut app = builder.build();
        let camera = app
            .world_mut()
            .spawn((
                Transform::from_translation(eye).with_rotation(rotation),
                PanOrbitCamera {
                    focus,
                    radius: settings.character_radius,
                },
            ))
            .id();
        app.world_mut().spawn((
            Transform::from_translation(Vec3::ZERO),
            CameraFocusTarget::new(TilePos::ORIGIN),
        ));
        app.world_mut().spawn((
            HexTile,
            TilePos::new(obstruction_coord, 0),
            HexSpan::new(0.0, 2.0),
        ));

        app.update();
        let settled_direction = app
            .world()
            .entity(camera)
            .get::<Transform>()
            .expect("the camera should keep its transform")
            .rotation
            * Vec3::Z;
        assert!(
            settled_direction.dot(initial_direction) < 0.75,
            "the no-room ray should choose a nearby clear yaw"
        );
        assert_eq!(app.world().resource::<CameraObstructionIndex>().rebuilds, 1);
        *app.world_mut().resource_mut::<CameraChangeCounts>() = CameraChangeCounts::default();

        for _ in 0..10_000 {
            app.update();
        }

        assert_eq!(app.world().resource::<CameraObstructionIndex>().rebuilds, 1);
        let counts = app.world().resource::<CameraChangeCounts>();
        assert_eq!(counts.transforms, 0);
        assert_eq!(counts.controls, 0);
        let final_direction = app
            .world()
            .entity(camera)
            .get::<Transform>()
            .expect("the camera should keep its transform")
            .rotation
            * Vec3::Z;
        assert!(final_direction.dot(settled_direction) > 0.9999);
    }

    #[test]
    fn character_pitch_can_orbit_near_the_horizon() {
        let min_pitch = 0.05;
        let max_pitch = 0.95;
        let rotation = apply_pitch_delta(
            rotation_at_pitch(0.3 * std::f32::consts::FRAC_PI_2),
            -10.0,
            min_pitch,
            max_pitch,
        );

        assert_pitch(rotation, min_pitch * std::f32::consts::FRAC_PI_2);
        assert!(
            min_pitch < camera_settings().min_pitch,
            "the close camera should tilt closer to the horizon than the map camera"
        );
        assert_eq!(
            pitch_limits(CameraMode::Map, &camera_settings()),
            (0.25, 0.95)
        );
        assert_eq!(
            pitch_limits(CameraMode::Character, &camera_settings()),
            (min_pitch, max_pitch)
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
        let mut builder = HeadlessAppBuilder::new()
            .with_minimal_plugins()
            .with_state_plugin();
        builder.app_mut().init_state::<Screen>();
        builder.app_mut().insert_resource(camera_settings());
        builder.app_mut().init_resource::<CameraMode>();
        builder.app_mut().init_resource::<SavedMapCamera>();
        builder
            .app_mut()
            .init_resource::<CharacterCameraCollision>();
        builder.app_mut().add_systems(
            OnEnter(Screen::Gameplay),
            (reset_camera_mode, frame_gameplay_camera).chain(),
        );
        let mut app = builder.build();
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
        let mut builder = HeadlessAppBuilder::new()
            .with_minimal_plugins()
            .with_state_plugin();
        builder.app_mut().init_state::<Screen>();
        builder.app_mut().insert_resource(camera_settings());
        builder.app_mut().configure_sets(
            OnEnter(Screen::Gameplay),
            (
                GameplaySetup::Resources,
                GameplaySetup::Terrain,
                GameplaySetup::Actors,
                GameplaySetup::Perception,
                GameplaySetup::View,
                GameplaySetup::Finalize,
            )
                .chain(),
        );
        builder.app_mut().add_systems(
            OnEnter(Screen::Gameplay),
            publish_generated_view.in_set(GameplaySetup::Resources),
        );
        builder.app_mut().add_systems(
            OnEnter(Screen::Gameplay),
            frame_gameplay_camera.in_set(GameplaySetup::View),
        );
        let mut app = builder.build();
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
        let mut builder = HeadlessAppBuilder::new()
            .with_minimal_plugins()
            .with_state_plugin();
        builder.app_mut().init_state::<Screen>();
        builder.app_mut().insert_resource(camera_settings());
        builder
            .app_mut()
            .insert_resource(MapViewHint::new((0.0, 0.0, 0.0), (0.0, 0.0, 0.0)));
        builder
            .app_mut()
            .add_systems(OnEnter(Screen::Gameplay), frame_gameplay_camera);
        let mut app = builder.build();
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
        let mut builder = HeadlessAppBuilder::new().with_minimal_plugins();
        builder.app_mut().insert_resource(camera_settings());
        builder
            .app_mut()
            .insert_resource(ButtonInput::<KeyCode>::default());
        builder.app_mut().init_resource::<CameraMode>();
        builder.app_mut().init_resource::<SavedMapCamera>();
        builder.app_mut().init_resource::<CameraObstructionIndex>();
        builder
            .app_mut()
            .init_resource::<CharacterCameraCollision>();
        builder.app_mut().init_resource::<InputBindings>();
        builder.app_mut().add_systems(Update, toggle_camera_mode);
        builder
            .app_mut()
            .add_systems(PostUpdate, follow_character_camera);

        let eye = Vec3::new(0.0, 48.0, 42.0);
        let focus = Vec3::new(0.0, 6.0, 0.0);
        let camera = builder
            .app_mut()
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
            builder
                .app_mut()
                .world_mut()
                .spawn((
                    Transform::from_translation(translation),
                    CameraFocusTarget::new(hex_core::TilePos::ORIGIN),
                ))
                .id()
        });
        (builder.build(), camera, target)
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
    fn character_camera_shortens_before_a_public_terrain_run() {
        let target = Vec3::new(3.0, 2.0, -1.0);
        let (mut app, camera, _) = prototype_camera_app(Some(target));

        toggle_camera(&mut app);
        let unobstructed = camera_pose(&app, camera);
        let direction = (unobstructed.0.translation - unobstructed.1).normalize();
        toggle_camera(&mut app);

        let obstacle_center = unobstructed.1 + direction * 4.0;
        let obstacle_coord = hex_core::HexCoord::from_world(obstacle_center);
        let lower = unobstructed.1.y.min(obstacle_center.y) - 1.0;
        let upper = unobstructed.1.y.max(obstacle_center.y) + 1.0;
        *app.world_mut().resource_mut::<CameraObstructionIndex>() = CameraObstructionIndex {
            spans_by_coord: BTreeMap::from([(obstacle_coord, vec![HexSpan::new(lower, upper)])]),
            initialized: true,
            rebuilds: 1,
        };

        toggle_camera(&mut app);

        let shortened = camera_pose(&app, camera);
        let effective = app
            .world()
            .resource::<CharacterCameraCollision>()
            .effective_radius
            .expect("Character mode should retain an effective radius");
        assert!(effective < camera_settings().character_radius);
        assert!(
            (shortened.0.translation.distance(shortened.1) - effective).abs() < 1e-5,
            "the rendered eye must use the collision-limited radius"
        );
        assert!(
            (shortened.2 - camera_settings().character_radius).abs() < f32::EPSILON,
            "collision must not overwrite the player's requested orbit radius"
        );
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
            CameraFocusTarget::new(hex_core::TilePos::ORIGIN),
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
        let mut builder = HeadlessAppBuilder::new();
        builder.app_mut().insert_resource(camera_settings());
        builder
            .app_mut()
            .insert_resource(ButtonInput::<KeyCode>::default());
        let mut time = Time::<()>::default();
        time.advance_by(Duration::from_secs(1));
        builder.app_mut().insert_resource(time);
        builder.app_mut().init_resource::<CameraMode>();
        builder.app_mut().init_resource::<InputBindings>();
        builder
            .app_mut()
            .add_systems(Update, pan_camera.run_if(map_camera_active));
        let mut app = builder.build();
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
        let mut builder = HeadlessAppBuilder::new()
            .with_minimal_plugins()
            .with_asset_plugin()
            .with_state_plugin()
            .with_input();
        builder
            .app_mut()
            .add_plugins(bevy::window::WindowPlugin::default());
        builder.app_mut().init_asset::<Mesh>();
        builder.app_mut().add_plugins(crate::sky_material::plugin);
        builder.app_mut().init_state::<Screen>();
        builder.app_mut().insert_resource(camera_settings());
        builder.app_mut().add_plugins(super::plugin);
        let mut app = builder.build();
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
        let mut builder = HeadlessAppBuilder::new()
            .with_minimal_plugins()
            .with_asset_plugin()
            .with_state_plugin()
            .with_input();
        builder
            .app_mut()
            .add_plugins(bevy::window::WindowPlugin::default());
        builder.app_mut().init_asset::<Mesh>();
        builder.app_mut().add_plugins(crate::sky_material::plugin);
        builder.app_mut().init_state::<Screen>();
        // No `CameraSettings` on purpose.
        builder.app_mut().add_plugins(super::plugin);
        let mut app = builder.build();

        enter(&mut app, Screen::Title);
    }
}
