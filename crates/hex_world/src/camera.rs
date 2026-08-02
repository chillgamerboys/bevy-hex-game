use std::collections::{BTreeMap, BTreeSet};

use bevy::input::mouse::MouseWheel;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy::transform::TransformSystems;
use bevy::window::{CursorMoved, PrimaryWindow};

use hex_assets::{to_color, CameraSettings, ResolvedLighting, Rgb};
use hex_core::{
    config::HEX_CIRCUMRADIUS, AppSystems, CameraFocusTarget, CenterInspectionCamera, GameplaySetup,
    HexSpan, HexTile, InputAction, InputBindings, InspectionCameraSubject, MapViewHint, Screen,
    TilePos, UnitId,
};

use crate::{
    sky_material::{SkyMaterial, SkyParams},
    LightingSystems,
};

/// Sky-dome radius, in world units. Comfortably inside the camera's default
/// 1000-unit far plane and far outside the configured zoom range plus the terrain.
const SKY_DOME_RADIUS: f32 = 500.0;

/// Distance from a unit-hex centre to any one of its six faces.
const HEX_FACE_DISTANCE: f32 = HEX_CIRCUMRADIUS * 0.866_025_4;
/// Lets ordinary upward free-look keep the character near the lower third of the
/// view before unusual-angle assistance starts lowering and retracting the boom.
const CHARACTER_UPWARD_COMPOSITION_ALLOWANCE: f32 = std::f32::consts::PI / 12.0;
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
        .register_type::<InspectionCameraSubject>()
        .register_type::<CenterInspectionCamera>()
        .register_type::<MapViewHint>()
        .register_type::<SkyDome>()
        .add_message::<CenterInspectionCamera>()
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
                center_inspection_camera,
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
    position: TilePos,
    center: Vec2,
    span: HexSpan,
}

/// One exact public terrain run retained by the cached camera index.
#[derive(Debug, Clone, Copy)]
struct IndexedCameraSpan {
    position: TilePos,
    span: HexSpan,
}

/// Cached presentation-only projection of public terrain geometry.
///
/// It intentionally contains no map-private storage or gameplay visibility facts.
#[derive(Resource, Debug, Default)]
struct CameraObstructionIndex {
    spans_by_coord: BTreeMap<hex_core::HexCoord, Vec<IndexedCameraSpan>>,
    initialized: bool,
    rebuilds: u64,
}

/// Transient pose used only while Character mode avoids terrain.
///
/// Player-authored rotation and zoom remain on the camera components. Collision may
/// only shorten the effective boom radius, then restore it after stable clearance.
#[derive(Resource, Debug, Default)]
struct CharacterCameraCollision {
    target: Option<Entity>,
    effective_radius: Option<f32>,
    last_desired_radius: Option<f32>,
    outward_clear_for_seconds: f32,
}

impl CharacterCameraCollision {
    fn begin_target(&mut self, target: Entity, desired_radius: f32) {
        self.target = Some(target);
        self.effective_radius = Some(desired_radius);
        self.last_desired_radius = Some(desired_radius);
        self.outward_clear_for_seconds = 0.0;
    }

    fn clear(&mut self) {
        self.target = None;
        self.effective_radius = None;
        self.last_desired_radius = None;
        self.outward_clear_for_seconds = 0.0;
    }
}

#[derive(Debug, Clone, Copy)]
struct CameraClearance {
    radius: f32,
    obstructed: bool,
}

/// Copy-only presentation target resolved from shared projections.
#[derive(Debug, Clone, Copy)]
struct ResolvedCameraTarget {
    entity: Entity,
    translation: Vec3,
    surface: TilePos,
}

/// Prefers one disclosure-authorized inspection subject, then falls back to the
/// gameplay-owned selection projection used before HUD inspection existed.
///
/// Multiple inspection subjects are a malformed adapter publication. Failing closed
/// to selection prevents query iteration order from choosing which disclosed unit the
/// camera follows.
fn resolve_camera_target<'a>(
    inspections: impl Iterator<
        Item = (
            Entity,
            &'a UnitId,
            &'a Transform,
            &'a InspectionCameraSubject,
        ),
    >,
    selections: impl Iterator<Item = (Entity, &'a Transform, &'a CameraFocusTarget)>,
) -> Option<ResolvedCameraTarget> {
    let mut authorized = inspections.filter_map(|(entity, unit, transform, subject)| {
        (*unit == subject.unit).then_some(ResolvedCameraTarget {
            entity,
            translation: transform.translation,
            surface: subject.surface,
        })
    });
    let inspected = authorized.next();
    if inspected.is_some() && authorized.next().is_none() {
        return inspected;
    }

    let mut selected = selections.map(|(entity, transform, target)| ResolvedCameraTarget {
        entity,
        translation: transform.translation,
        surface: target.surface,
    });
    let target = selected.next();
    if target.is_some() && selected.next().is_none() {
        target
    } else {
        None
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
    collision.clear();
}

fn map_camera_active(mode: Res<CameraMode>) -> bool {
    *mode == CameraMode::Map
}

fn pitch_limits(mode: CameraMode, settings: &CameraSettings) -> (f32, f32) {
    match mode {
        CameraMode::Map => (settings.min_pitch, settings.max_pitch),
        CameraMode::Character => (-1.0, 1.0),
    }
}

/// Applies explicit HUD inspection requests to the free Map camera exactly once.
///
/// The request is accepted only when exactly one entity carries a matching,
/// identity-consistent [`InspectionCameraSubject`]. Character mode drains and ignores
/// center messages because its normal follow pass already tracks that subject.
fn center_inspection_camera(
    mut requests: MessageReader<CenterInspectionCamera>,
    mode: Res<CameraMode>,
    settings: Res<CameraSettings>,
    subjects: Query<(&UnitId, &Transform, &InspectionCameraSubject), Without<PanOrbitCamera>>,
    mut cameras: Query<(&mut Transform, &mut PanOrbitCamera), With<PanOrbitCamera>>,
) {
    for request in requests.read() {
        if *mode != CameraMode::Map {
            continue;
        }
        let mut matches = subjects
            .iter()
            .filter(|(unit, _, subject)| **unit == request.unit && subject.unit == request.unit);
        let Some((_, target, _)) = matches.next() else {
            continue;
        };
        if matches.next().is_some() {
            continue;
        }
        let Ok((mut transform, mut camera)) = cameras.single_mut() else {
            continue;
        };
        let wanted_focus = target.translation + Vec3::Y * settings.character_focus_height;
        let translation = wanted_focus - camera.focus;
        if translation.length_squared() <= f32::EPSILON {
            continue;
        }
        transform.translation += translation;
        camera.focus = wanted_focus;
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
    inspected: Query<
        (Entity, &UnitId, &Transform, &InspectionCameraSubject),
        Without<PanOrbitCamera>,
    >,
    selected: Query<(Entity, &Transform, &CameraFocusTarget), Without<PanOrbitCamera>>,
    mut cameras: Query<(&mut Transform, &mut PanOrbitCamera), With<PanOrbitCamera>>,
) {
    if !bindings.just_pressed(&keys, InputAction::ToggleCamera) {
        return;
    }

    let Ok((mut transform, mut camera)) = cameras.single_mut() else {
        return;
    };
    match *mode {
        CameraMode::Map => {
            let Some(target) = resolve_camera_target(inspected.iter(), selected.iter()) else {
                warn!(
                    "cannot enter character camera without exactly one inspection or selection target"
                );
                return;
            };
            saved.0 = Some(CameraPose::capture(&transform, &camera));

            let wanted_pitch = settings.character_pitch * std::f32::consts::FRAC_PI_2;
            let pitch_delta = wanted_pitch - downward_pitch(transform.rotation);
            transform.rotation = apply_pitch_delta(transform.rotation, pitch_delta, -1.0, 1.0);

            camera.focus = target.translation + Vec3::Y * settings.character_focus_height;
            camera.radius = settings.character_radius;
            collision.begin_target(target.entity, camera.radius);
            transform.translation = camera.focus
                + Mat3::from_quat(transform.rotation).mul_vec3(Vec3::new(0.0, 0.0, camera.radius));
            *mode = CameraMode::Character;
        }
        CameraMode::Character => {
            if let Some(pose) = saved.0.take() {
                pose.restore(&mut transform, &mut camera);
            }
            collision.clear();
            *mode = CameraMode::Map;
        }
    }
}

/// Keeps a close orbit centred on the inspected or selected unit's rendered position.
fn follow_character_camera(
    mut mode: ResMut<CameraMode>,
    mut saved: ResMut<SavedMapCamera>,
    settings: Res<CameraSettings>,
    time: Res<Time>,
    obstruction_index: Res<CameraObstructionIndex>,
    mut collision: ResMut<CharacterCameraCollision>,
    inspected: Query<
        (Entity, &UnitId, &Transform, &InspectionCameraSubject),
        Without<PanOrbitCamera>,
    >,
    selected: Query<(Entity, &Transform, &CameraFocusTarget), Without<PanOrbitCamera>>,
    mut cameras: Query<(&mut Transform, &mut PanOrbitCamera), With<PanOrbitCamera>>,
) {
    if *mode != CameraMode::Character {
        return;
    }
    let Ok((mut transform, mut camera)) = cameras.single_mut() else {
        return;
    };
    let Some(target) = resolve_camera_target(inspected.iter(), selected.iter()) else {
        if let Some(pose) = saved.0.take() {
            pose.restore(&mut transform, &mut camera);
        }
        collision.clear();
        *mode = CameraMode::Map;
        return;
    };

    // Collision history belongs to one selected unit. A new target must resolve its
    // own corridor immediately instead of inheriting the prior target's close boom or
    // release timer.
    if collision.target != Some(target.entity) {
        collision.begin_target(target.entity, camera.radius);
    }

    let wanted_focus = target.translation + Vec3::Y * settings.character_focus_height;
    if wanted_focus.distance_squared(camera.focus) > f32::EPSILON {
        camera.focus = wanted_focus;
    }

    // Rotation is exclusively player-authored. Collision follows the deterministic
    // placement ray derived from that look and can shorten only its distance; it
    // never changes where the player looks.
    let direction = character_boom_direction(transform.rotation);
    let clearance = obstruction_index.safe_radius(
        wanted_focus,
        target.surface,
        direction,
        camera.radius,
        settings.character_probe_radius,
        settings.character_collision_margin,
    );
    let previous = collision.effective_radius.unwrap_or(camera.radius);
    let desired_changed = collision
        .last_desired_radius
        .is_none_or(|radius| (radius - camera.radius).abs() > f32::EPSILON);
    let effective = resolve_effective_radius(
        previous,
        clearance.radius,
        clearance.obstructed,
        desired_changed,
        settings.character_collision_release_delay,
        settings.character_restoration_speed,
        time.delta_secs(),
        &mut collision.outward_clear_for_seconds,
    );
    if collision
        .effective_radius
        .is_none_or(|radius| (radius - effective).abs() > f32::EPSILON)
    {
        collision.effective_radius = Some(effective);
    }
    if collision
        .last_desired_radius
        .is_none_or(|radius| (radius - camera.radius).abs() > f32::EPSILON)
    {
        collision.last_desired_radius = Some(camera.radius);
    }

    let wanted_eye = wanted_focus + direction * effective;
    if transform.translation.distance_squared(wanted_eye) > f32::EPSILON {
        transform.translation = wanted_eye;
    }
}

fn resolve_effective_radius(
    previous: f32,
    safe_radius: f32,
    obstructed: bool,
    desired_changed: bool,
    release_delay: f32,
    restoration_speed: f32,
    delta_seconds: f32,
    outward_clear_for_seconds: &mut f32,
) -> f32 {
    if safe_radius < previous {
        *outward_clear_for_seconds = 0.0;
        return safe_radius;
    }
    // Partial clearance is still an obstruction. Waiting until the complete desired
    // boom is clear prevents adjacent faces from accumulating one release timer and
    // producing an outward/inward "breath" while the character walks past them.
    if obstructed {
        *outward_clear_for_seconds = 0.0;
        return previous;
    }
    if desired_changed {
        *outward_clear_for_seconds = 0.0;
        return safe_radius;
    }
    if safe_radius - previous <= f32::EPSILON {
        *outward_clear_for_seconds = 0.0;
        return previous;
    }

    *outward_clear_for_seconds += delta_seconds.max(0.0);
    if *outward_clear_for_seconds + f32::EPSILON < release_delay {
        return previous;
    }

    (previous + restoration_speed * delta_seconds.max(0.0)).min(safe_radius)
}

/// Returns the placement ray for the Character camera without changing its look.
///
/// Downward and level views use the ordinary orbit ray. For upward free-look, the
/// camera may lag the authored pitch by at most fifteen degrees. This keeps shallow
/// upward input from driving a long third-person boom into the supporting floor,
/// while steeper placement rays track the authored pitch with that fixed offset and
/// progressively meet the support-limited close view. The camera's rotation remains
/// exactly player-authored throughout.
fn character_boom_direction(rotation: Quat) -> Vec3 {
    let desired_pitch = downward_pitch(rotation);
    if !desired_pitch.is_finite() {
        return rotation * Vec3::Z;
    }
    let placement_pitch = if desired_pitch < 0.0 {
        (desired_pitch + CHARACTER_UPWARD_COMPOSITION_ALLOWANCE).min(0.0)
    } else {
        desired_pitch
    };
    let right = rotation * Vec3::X;
    let mut horizontal_back = right.cross(Vec3::Y).normalize_or_zero();
    if horizontal_back.length_squared() <= f32::EPSILON {
        let backward = rotation * Vec3::Z;
        horizontal_back = Vec3::new(backward.x, 0.0, backward.z).normalize_or_zero();
    }
    if horizontal_back.length_squared() <= f32::EPSILON {
        horizontal_back = Vec3::Z;
    }
    horizontal_back * placement_pitch.cos() + Vec3::Y * placement_pitch.sin()
}

impl CameraObstructionIndex {
    fn rebuild(&mut self, tiles: impl IntoIterator<Item = (TilePos, HexSpan)>) {
        let mut spans_by_coord = BTreeMap::<_, Vec<_>>::new();
        for (position, span) in tiles {
            spans_by_coord
                .entry(position.coord)
                .or_default()
                .push(IndexedCameraSpan { position, span });
        }
        for spans in spans_by_coord.values_mut() {
            spans.sort_by(|first, second| {
                first
                    .span
                    .bottom
                    .total_cmp(&second.span.bottom)
                    .then_with(|| first.span.top.total_cmp(&second.span.top))
                    .then_with(|| first.position.cmp(&second.position))
            });
        }
        self.spans_by_coord = spans_by_coord;
        self.initialized = true;
        self.rebuilds = self.rebuilds.saturating_add(1);
    }

    fn safe_radius(
        &self,
        focus: Vec3,
        support: TilePos,
        direction: Vec3,
        desired_radius: f32,
        probe_radius: f32,
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
        if !probe_radius.is_finite()
            || probe_radius <= 0.0
            || probe_radius > CameraSettings::MAX_CHARACTER_PROBE_RADIUS
        {
            // Validated settings never reach this branch. An external adapter or
            // test that bypasses validation fails closed instead of silently using
            // an undersized spatial candidate set and tunnelling through terrain.
            return CameraClearance {
                radius: 0.0,
                obstructed: true,
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
            .flat_map(|coord| coord.within_radius(camera_probe_candidate_radius(probe_radius)))
            .collect::<BTreeSet<_>>();
        let hit = candidate_coords
            .into_iter()
            .filter_map(|coord| self.spans_by_coord.get(&coord).map(|spans| (coord, spans)))
            .flat_map(|(coord, spans)| {
                let center = coord.to_world(0.0).xz();
                spans.iter().copied().map(move |indexed| CameraObstruction {
                    position: indexed.position,
                    center,
                    span: indexed.span,
                })
            })
            .filter_map(|obstruction| {
                obstruction.first_hit_distance(
                    focus,
                    support,
                    direction,
                    desired_radius,
                    probe_radius,
                )
            })
            .min_by(f32::total_cmp);
        hit.map_or(
            CameraClearance {
                radius: desired_radius,
                obstructed: false,
            },
            |distance| CameraClearance {
                // The margin is a preferred separation, never permission to cross
                // the closest hit. A tight enclosure may honestly retract to zero.
                radius: (distance - margin).max(0.0).min(desired_radius),
                obstructed: true,
            },
        )
    }
}

fn camera_probe_candidate_radius(probe_radius: f32) -> u32 {
    let mut radius = 1_u32;
    let mut covered = HEX_FACE_DISTANCE;
    while probe_radius >= covered {
        radius = radius.saturating_add(1);
        covered += HEX_FACE_DISTANCE;
    }
    radius
}

impl CameraObstruction {
    fn first_hit_distance(
        self,
        origin: Vec3,
        support: TilePos,
        direction: Vec3,
        maximum: f32,
        probe_radius: f32,
    ) -> Option<f32> {
        let local_origin = origin.xz() - self.center;
        let horizontal_limit = HEX_FACE_DISTANCE + probe_radius;
        let mut enter: f32 = 0.0;
        let mut exit = maximum;
        for normal in [
            Vec2::X,
            Vec2::new(0.5, 0.866_025_4),
            Vec2::new(-0.5, 0.866_025_4),
        ] {
            let interval = axis_interval(
                local_origin.dot(normal),
                direction.xz().dot(normal),
                -horizontal_limit,
                horizontal_limit,
                maximum,
            )?;
            enter = enter.max(interval.0);
            exit = exit.min(interval.1);
        }
        let vertical = axis_interval(
            origin.y,
            direction.y,
            self.span.bottom - probe_radius,
            self.span.top + probe_radius,
            maximum,
        )?;
        enter = enter.max(vertical.0);
        exit = exit.min(vertical.1);
        if enter > exit {
            return None;
        }
        if enter > f32::EPSILON {
            return Some(enter);
        }

        // A local run whose actual top is at or below the focus is floor-like while
        // an upward sweep leaves it. "Local" is deliberately limited to the exact
        // support and its ordinary one-step neighborhood; unrelated stacked runs
        // remain obstructions even when an unusually wide probe reaches them. The
        // probe expansion may overlap a local floor at distance zero even though the
        // focus itself is not inside material. That happens while a unit interpolates
        // onto a one-level-higher neighbor: the authoritative support remains the
        // previous surface until the leg ends, while the smooth focus is already
        // above the destination's real top.
        //
        // Ignore only that zero-entry, monotonically exiting overlap. A wall or roof
        // whose real top remains above the focus, as well as every positive-distance
        // hit, still obstructs. Validated settings also keep the probe radius no
        // larger than the focus height, so an ordinary floor cannot contain the
        // camera's target point.
        let local_floor = self.position.coord.distance(support.coord) <= 1
            && self.position.level.abs_diff(support.level) <= 1
            && self.span.top <= origin.y + f32::EPSILON;
        let exits_supporting_floor = local_floor && direction.y >= -f32::EPSILON;
        (!exits_supporting_floor).then_some(0.0)
    }
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
    // Drain the complete batch. Reading only the first removal leaves the cursor
    // behind, which would turn one large terrain replacement into one full index
    // rebuild per frame until every stale message had been consumed.
    let removed = removed_tiles.read().count() > 0;
    if index.initialized && !removed && changed_tiles.is_empty() {
        return;
    }

    index.rebuild(tiles.iter().map(|(position, span)| (*position, *span)));
}

fn clear_camera_obstruction_index(
    mut index: ResMut<CameraObstructionIndex>,
    mut collision: ResMut<CharacterCameraCollision>,
) {
    if index.initialized || !index.spans_by_coord.is_empty() {
        index.spans_by_coord.clear();
        index.initialized = false;
    }
    if collision.effective_radius.is_some()
        || collision.last_desired_radius.is_some()
        || collision.outward_clear_for_seconds > 0.0
    {
        collision.clear();
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
/// the limits as fractions of a quarter-turn, so `-1.0` is straight up, `0.0` is
/// level, and `1.0` is straight down. Integrating the scalar angle before building
/// the quaternion avoids losing which side of a vertical pole a large cursor movement
/// crossed.
fn apply_pitch_delta(rotation: Quat, downward_delta: f32, min_pitch: f32, max_pitch: f32) -> Quat {
    if !downward_delta.is_finite()
        || !min_pitch.is_finite()
        || !max_pitch.is_finite()
        || !(-1.0..=1.0).contains(&min_pitch)
        || !(-1.0..=1.0).contains(&max_pitch)
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

#[cfg(any(test, feature = "test-support"))]
fn rotation_with_pitch(rotation: Quat, pitch: f32) -> Quat {
    let wanted = pitch * std::f32::consts::FRAC_PI_2;
    apply_pitch_delta(rotation, wanted - downward_pitch(rotation), -1.0, 1.0)
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
            let base_rotation = transform.rotation;
            transform.rotation = yaw * base_rotation; // rotate around global y axis
            let (min_pitch, max_pitch) = pitch_limits(*mode, &settings);
            transform.rotation =
                apply_pitch_delta(transform.rotation, delta_y, min_pitch, max_pitch);
        }
        if scroll.abs() > 0.0 {
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

#[cfg(feature = "test-support")]
pub mod test_support {
    //! Read-only timing diagnostics for composition tests.
    //!
    //! Callers supply the same public `TilePos`/`HexSpan` projection that the
    //! production camera observes. This module never accepts map-private storage.

    use std::collections::{BTreeMap, BTreeSet};
    use std::time::{Duration, Instant};

    use bevy::prelude::{Quat, Vec3};
    use hex_assets::CameraSettings;
    use hex_core::{HexSpan, TilePos};

    use super::{character_boom_direction, rotation_with_pitch, CameraObstructionIndex};

    const INDEX_REBUILD_SAMPLES: usize = 32;

    /// Timings and coverage facts from one Character-camera collision diagnostic.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct CharacterCollisionProfile {
        /// Unique hex columns retained by the public-terrain index.
        pub columns: usize,
        /// Exact material runs retained across those columns.
        pub spans: usize,
        /// Unique exact support surfaces sampled at six yaw angles each.
        pub supports: usize,
        /// Number of steady collision queries timed.
        pub queries: usize,
        /// First construction of the camera obstruction index.
        pub index_build: Duration,
        /// Ninety-fifth percentile of repeated full index rebuilds.
        pub index_rebuild_p95: Duration,
        /// Slowest repeated full index rebuild.
        pub index_rebuild_worst: Duration,
        /// Ninety-fifth percentile of steady Character collision queries.
        pub query_p95: Duration,
        /// Slowest steady Character collision query.
        pub query_worst: Duration,
        /// Rolling result identity that keeps every timed query observable.
        pub result_checksum: u64,
    }

    #[derive(Debug, Clone, Copy)]
    struct DiagnosticQuery {
        focus: Vec3,
        support: TilePos,
        desired_rotation: Quat,
    }

    /// Profiles the production index and Character collision algorithm over a
    /// caller-owned public terrain projection.
    ///
    /// `supports` should be representative exact surfaces, such as the shipped
    /// map's published anchors. Every unique support is sampled at six yaw angles.
    /// Generation, rendering, and projection collection are deliberately outside
    /// the timed regions.
    pub fn profile_character_collision(
        projection: &[(TilePos, HexSpan)],
        supports: &[TilePos],
        settings: &CameraSettings,
        query_count: usize,
    ) -> Result<CharacterCollisionProfile, String> {
        settings.validate()?;
        if projection.is_empty() {
            return Err("camera diagnostic requires at least one public terrain run".to_owned());
        }
        if query_count == 0 {
            return Err("camera diagnostic requires at least one collision query".to_owned());
        }

        let spans_by_position = projection
            .iter()
            .copied()
            .collect::<BTreeMap<TilePos, HexSpan>>();
        if spans_by_position.len() != projection.len() {
            return Err("camera diagnostic received duplicate public TilePos entries".to_owned());
        }
        let canonical_supports = supports.iter().copied().collect::<BTreeSet<_>>();
        if canonical_supports.is_empty() {
            return Err("camera diagnostic requires at least one exact support".to_owned());
        }

        let mut samples = Vec::with_capacity(canonical_supports.len().saturating_mul(6));
        for support in &canonical_supports {
            let span = spans_by_position.get(support).ok_or_else(|| {
                format!(
                    "camera diagnostic support {support:?} is absent from the public projection"
                )
            })?;
            let focus =
                support.coord.to_world(span.top) + Vec3::Y * settings.character_focus_height;
            for turn in 0_u8..6 {
                let yaw = f32::from(turn) * std::f32::consts::TAU / 6.0;
                samples.push(DiagnosticQuery {
                    focus,
                    support: *support,
                    desired_rotation: rotation_with_pitch(
                        Quat::from_rotation_y(yaw),
                        settings.character_pitch,
                    ),
                });
            }
        }

        let started = Instant::now();
        let mut index = CameraObstructionIndex::default();
        index.rebuild(projection.iter().copied());
        let index_build = started.elapsed();
        let columns = index.spans_by_coord.len();

        let mut rebuild_timings = Vec::with_capacity(INDEX_REBUILD_SAMPLES);
        for _ in 0..INDEX_REBUILD_SAMPLES {
            let started = Instant::now();
            index.rebuild(projection.iter().copied());
            rebuild_timings.push(started.elapsed());
        }
        let index_rebuild_p95 = percentile(&mut rebuild_timings, 95);
        let index_rebuild_worst = rebuild_timings.last().copied().unwrap_or_default();

        let mut query_timings = Vec::with_capacity(query_count);
        let mut result_checksum = 0_u64;
        for sample in samples.iter().cycle().take(query_count) {
            let started = Instant::now();
            let clearance = index.safe_radius(
                sample.focus,
                sample.support,
                character_boom_direction(sample.desired_rotation),
                settings.character_radius,
                settings.character_probe_radius,
                settings.character_collision_margin,
            );
            query_timings.push(started.elapsed());
            result_checksum = result_checksum.rotate_left(9)
                ^ u64::from(clearance.radius.to_bits())
                ^ (u64::from(clearance.obstructed) << 63);
        }
        let query_p95 = percentile(&mut query_timings, 95);
        let query_worst = query_timings.last().copied().unwrap_or_default();

        Ok(CharacterCollisionProfile {
            columns,
            spans: projection.len(),
            supports: canonical_supports.len(),
            queries: query_count,
            index_build,
            index_rebuild_p95,
            index_rebuild_worst,
            query_p95,
            query_worst,
            result_checksum,
        })
    }

    fn percentile(timings: &mut [Duration], percentile: usize) -> Duration {
        timings.sort_unstable();
        let rank = timings
            .len()
            .saturating_mul(percentile)
            .div_ceil(100)
            .saturating_sub(1);
        timings.get(rank).copied().unwrap_or_default()
    }
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
            character_probe_radius: 0.1,
            character_collision_margin: 0.35,
            character_restoration_speed: 8.0,
            character_collision_release_delay: 0.2,
            character_self_hide_radius: 1.0,
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

    fn rotation_facing(horizontal_direction: Vec3, pitch: f32) -> Quat {
        let base = Transform::from_translation(horizontal_direction)
            .looking_at(Vec3::ZERO, Vec3::Y)
            .rotation;
        rotation_with_pitch(base, pitch)
    }

    fn assert_pitch(rotation: Quat, expected: f32) {
        let actual = downward_pitch(rotation);
        assert!(
            (actual - expected).abs() < 1e-5,
            "expected pitch {expected}, got {actual}"
        );
    }

    fn indexed_span(coord: hex_core::HexCoord, span: HexSpan) -> IndexedCameraSpan {
        IndexedCameraSpan {
            position: TilePos::new(coord, 0),
            span,
        }
    }

    fn flat_radius_55_obstruction_fixture() -> (CameraObstructionIndex, Vec3) {
        let obstruction_coord = hex_core::HexCoord::from_axial(0, 1);
        let direction = obstruction_coord.to_world(0.0).normalize();
        let mut spans_by_coord = hex_core::HexCoord::ORIGIN
            .within_radius(55)
            .into_iter()
            .map(|coord| (coord, vec![indexed_span(coord, HexSpan::new(0.0, 0.4))]))
            .collect::<BTreeMap<_, _>>();
        spans_by_coord.insert(
            obstruction_coord,
            vec![indexed_span(obstruction_coord, HexSpan::new(0.0, 2.0))],
        );
        assert_eq!(
            spans_by_coord.len(),
            9_241,
            "a flat radius-55 fixture must contain 9,241 columns"
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

    fn timed_character_queries(iterations: usize) -> (CameraClearance, Vec<Duration>) {
        let (index, direction) = flat_radius_55_obstruction_fixture();
        let desired_rotation = Transform::from_translation(direction)
            .looking_at(Vec3::ZERO, Vec3::Y)
            .rotation;
        let expected = index.safe_radius(
            Vec3::Y,
            TilePos::ORIGIN,
            desired_rotation * Vec3::Z,
            7.0,
            0.4,
            0.35,
        );
        let mut timings = Vec::with_capacity(iterations);
        for _ in 0..iterations {
            let started = Instant::now();
            let clearance = index.safe_radius(
                Vec3::Y,
                TilePos::ORIGIN,
                desired_rotation * Vec3::Z,
                7.0,
                0.4,
                0.35,
            );
            timings.push(started.elapsed());
            assert_eq!(clearance.radius.to_bits(), expected.radius.to_bits());
            assert_eq!(clearance.obstructed, expected.obstructed);
        }
        assert_eq!(index.rebuilds, 1);
        (expected, timings)
    }

    fn resolve_test_radius(
        previous: f32,
        safe_radius: f32,
        obstructed: bool,
        desired_changed: bool,
        delta_seconds: f32,
        outward_clear_for_seconds: &mut f32,
    ) -> f32 {
        resolve_effective_radius(
            previous,
            safe_radius,
            obstructed,
            desired_changed,
            0.2,
            8.0,
            delta_seconds,
            outward_clear_for_seconds,
        )
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
    fn character_pitch_spans_straight_up_through_straight_down() {
        let middle = rotation_at_pitch(0.0);
        let straight_up = apply_pitch_delta(middle, -10.0, -1.0, 1.0);
        let straight_down = apply_pitch_delta(middle, 10.0, -1.0, 1.0);
        let upward_interior = apply_pitch_delta(middle, -0.4, -1.0, 1.0);

        assert_pitch(straight_up, -std::f32::consts::FRAC_PI_2);
        assert!(
            (straight_up * Vec3::NEG_Z).dot(Vec3::Y) > 0.99999,
            "the upper Character limit must look straight up"
        );
        assert_pitch(straight_down, std::f32::consts::FRAC_PI_2);
        assert!(
            (straight_down * Vec3::NEG_Z).dot(Vec3::NEG_Y) > 0.99999,
            "the lower Character limit must look straight down"
        );
        assert_pitch(upward_interior, -0.4);
        assert_eq!(
            pitch_limits(CameraMode::Map, &camera_settings()),
            (0.25, 0.95),
            "Map mode keeps its tactical downward-only arc"
        );
        assert_eq!(
            pitch_limits(CameraMode::Character, &camera_settings()),
            (-1.0, 1.0),
            "Character mode must expose the full vertical look arc"
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
            spans_by_coord: BTreeMap::from([(
                obstruction_coord,
                vec![indexed_span(obstruction_coord, HexSpan::new(0.0, 1.0))],
            )]),
            initialized: true,
            rebuilds: 1,
        };

        let clearance = index.safe_radius(
            Vec3::new(0.0, 1.0, 0.0),
            TilePos::ORIGIN,
            Vec3::Z,
            7.0,
            0.4,
            0.35,
        );

        assert!(clearance.obstructed);
        assert!(clearance.radius > 0.0 && clearance.radius < 1.65);
        let clear = index.safe_radius(
            Vec3::new(0.0, 3.0, 0.0),
            TilePos::ORIGIN,
            Vec3::Z,
            7.0,
            0.4,
            0.35,
        );
        assert!(!clear.obstructed);
        assert!(
            (clear.radius - 7.0).abs() < f32::EPSILON,
            "a vertically disjoint run must not obstruct the view segment"
        );
    }

    #[test]
    fn swept_probe_is_conservative_at_prism_faces_and_corners() {
        let obstruction = CameraObstruction {
            position: TilePos::new(hex_core::HexCoord::ORIGIN, 1),
            center: Vec2::ZERO,
            span: HexSpan::new(0.0, 1.0),
        };
        let probe = 0.4;
        let face_hit = obstruction
            .first_hit_distance(
                Vec3::new(-3.0, 0.5, 0.0),
                TilePos::ORIGIN,
                Vec3::X,
                7.0,
                probe,
            )
            .expect("the expanded flat face should be hit");
        assert!((face_hit - (3.0 - HEX_FACE_DISTANCE - probe)).abs() < 1e-5);

        let corner_hit = obstruction
            .first_hit_distance(
                Vec3::new(0.0, 0.5, -3.0),
                TilePos::ORIGIN,
                Vec3::Z,
                7.0,
                probe,
            )
            .expect("the expanded point-facing corner should be hit");
        let expected_corner = 3.0 - (HEX_FACE_DISTANCE + probe) / 0.866_025_4;
        assert!((corner_hit - expected_corner).abs() < 1e-5);
        assert!(
            obstruction
                .first_hit_distance(
                    Vec3::new(HEX_FACE_DISTANCE + probe + 0.01, 0.5, -3.0),
                    TilePos::ORIGIN,
                    Vec3::Z,
                    7.0,
                    probe,
                )
                .is_none(),
            "a parallel sweep outside the expanded face must remain clear"
        );
    }

    #[test]
    fn a_wide_valid_probe_expands_spatial_candidates_beyond_one_ring() {
        let obstruction_coord = hex_core::HexCoord::from_axial(0, 2);
        let index = CameraObstructionIndex {
            spans_by_coord: BTreeMap::from([(
                obstruction_coord,
                vec![indexed_span(obstruction_coord, HexSpan::new(0.0, 1.0))],
            )]),
            initialized: true,
            rebuilds: 1,
        };
        let direction = hex_core::HexCoord::from_axial(1, 0)
            .to_world(0.0)
            .normalize();
        let focus = Vec3::new(0.0, 0.5, 0.0);

        let narrow = index.safe_radius(focus, TilePos::ORIGIN, direction, 7.0, 0.4, 0.0);
        assert!(!narrow.obstructed);
        let wide = index.safe_radius(
            focus,
            TilePos::ORIGIN,
            direction,
            7.0,
            CameraSettings::MAX_CHARACTER_PROBE_RADIUS,
            0.0,
        );
        assert!(wide.obstructed);
        assert!(wide.radius < 7.0);
    }

    #[test]
    fn probe_exits_floor_like_zero_entry_but_not_walls_or_stacked_geometry() {
        let support = CameraObstruction {
            position: TilePos::ORIGIN,
            center: Vec2::ZERO,
            span: HexSpan::new(-0.4, 0.0),
        };
        let direction = Vec3::new(0.0, 0.5, 1.0).normalize();
        assert!(
            support
                .first_hit_distance(Vec3::Y * 0.4, TilePos::ORIGIN, direction, 7.0, 0.4)
                .is_none(),
            "the focus probe starts on and exits the selected unit's support"
        );

        let coplanar_floor = CameraObstruction {
            position: TilePos::new(hex_core::HexCoord::ORIGIN, -1),
            center: Vec2::ZERO,
            span: HexSpan::new(-0.4, 0.0),
        };
        assert!(
            coplanar_floor
                .first_hit_distance(Vec3::Y * 0.4, TilePos::ORIGIN, direction, 7.0, 0.4)
                .is_none(),
            "a coplanar floor tangent to the probe must not cage it"
        );
        let raised_step = CameraObstruction {
            span: HexSpan::new(-0.4, 0.4),
            ..coplanar_floor
        };
        assert!(
            raised_step
                .first_hit_distance(Vec3::Y * 0.4, TilePos::ORIGIN, direction, 7.0, 0.4)
                .is_none(),
            "a floor whose real top reaches the focus must not become a probe-expansion wall"
        );
        let wall_above_focus = CameraObstruction {
            span: HexSpan::new(-0.4, 0.8),
            ..coplanar_floor
        };
        assert_eq!(
            wall_above_focus.first_hit_distance(
                Vec3::Y * 0.4,
                TilePos::ORIGIN,
                direction,
                7.0,
                0.4,
            ),
            Some(0.0),
            "a different run extending above the focus remains an immediate obstruction"
        );
        let unrelated_stacked_floor = CameraObstruction {
            position: TilePos::new(hex_core::HexCoord::ORIGIN, -2),
            span: HexSpan::new(-0.4, 0.0),
            ..coplanar_floor
        };
        assert_eq!(
            unrelated_stacked_floor.first_hit_distance(
                Vec3::Y * 0.4,
                TilePos::ORIGIN,
                direction,
                7.0,
                0.4,
            ),
            Some(0.0),
            "an unrelated stacked run must not inherit the local-floor exception"
        );

        for containing in [
            CameraObstruction {
                position: TilePos::new(hex_core::HexCoord::ORIGIN, 1),
                center: Vec2::ZERO,
                span: HexSpan::new(-0.4, 2.0),
            },
            CameraObstruction {
                position: TilePos::new(hex_core::HexCoord::ORIGIN, 2),
                center: Vec2::ZERO,
                span: HexSpan::new(0.2, 1.0),
            },
        ] {
            assert_eq!(
                containing.first_hit_distance(Vec3::ZERO, TilePos::ORIGIN, direction, 7.0, 0.4,),
                Some(0.0),
                "a containing wall or ceiling must be an immediate hit"
            );
        }

        let bridge = CameraObstruction {
            position: TilePos::new(hex_core::HexCoord::ORIGIN, 5),
            center: Vec2::ZERO,
            span: HexSpan::new(1.5, 2.5),
        };
        assert!(
            bridge
                .first_hit_distance(
                    Vec3::new(-3.0, 1.0, 0.0),
                    TilePos::ORIGIN,
                    Vec3::X,
                    7.0,
                    0.4,
                )
                .is_none(),
            "a vertically disjoint bridge must not block a camera below it"
        );
        assert!(
            bridge
                .first_hit_distance(
                    Vec3::new(-3.0, 1.2, 0.0),
                    TilePos::ORIGIN,
                    Vec3::X,
                    7.0,
                    0.4,
                )
                .is_some(),
            "the near-plane probe must catch the expanded bridge underside"
        );
    }

    #[test]
    fn straight_up_look_retracts_against_support_without_reauthoring_rotation() {
        let settings = camera_settings();
        let authored = apply_pitch_delta(
            Quat::from_rotation_y(0.7),
            -std::f32::consts::FRAC_PI_2,
            -1.0,
            1.0,
        );
        let index = CameraObstructionIndex {
            spans_by_coord: BTreeMap::from([(
                hex_core::HexCoord::ORIGIN,
                vec![indexed_span(
                    hex_core::HexCoord::ORIGIN,
                    HexSpan::new(-0.4, 0.0),
                )],
            )]),
            initialized: true,
            rebuilds: 1,
        };
        let clearance = index.safe_radius(
            Vec3::Y * settings.character_focus_height,
            TilePos::ORIGIN,
            character_boom_direction(authored),
            settings.character_radius,
            settings.character_probe_radius,
            settings.character_collision_margin,
        );

        assert!(clearance.obstructed);
        assert!(clearance.radius.abs() < f32::EPSILON);
        assert!((authored * Vec3::NEG_Z).dot(Vec3::Y) > 0.99999);
        assert!((authored * Vec3::X).dot(Quat::from_rotation_y(0.7) * Vec3::X) > 0.99999);
    }

    #[test]
    fn shallow_upward_look_keeps_the_full_open_ground_boom() {
        let settings = camera_settings();
        let authored = rotation_at_pitch(-10.0_f32.to_radians());
        let boom_direction = character_boom_direction(authored);
        let index = CameraObstructionIndex {
            spans_by_coord: BTreeMap::from([(
                hex_core::HexCoord::ORIGIN,
                vec![indexed_span(
                    hex_core::HexCoord::ORIGIN,
                    HexSpan::new(-0.4, 0.0),
                )],
            )]),
            initialized: true,
            rebuilds: 1,
        };

        let clearance = index.safe_radius(
            Vec3::Y * settings.character_focus_height,
            TilePos::ORIGIN,
            boom_direction,
            settings.character_radius,
            settings.character_probe_radius,
            settings.character_collision_margin,
        );

        assert!(boom_direction.y.abs() < 1e-5);
        assert!(!clearance.obstructed);
        assert!((clearance.radius - settings.character_radius).abs() < f32::EPSILON);
        assert_pitch(authored, -10.0_f32.to_radians());
    }

    #[test]
    fn upward_placement_is_continuous_and_never_lags_authored_look_by_more_than_fifteen_degrees() {
        let cases = [
            (0.0_f32, 0.0_f32),
            (-10.0, 0.0),
            (-15.0, 0.0),
            (-30.0, -15.0),
            (-90.0, -75.0),
        ];
        let mut previous_placement = f32::INFINITY;

        for (authored_degrees, expected_placement_degrees) in cases {
            let authored_pitch = authored_degrees.to_radians();
            let direction = character_boom_direction(rotation_at_pitch(authored_pitch));
            let placement_pitch = direction.y.clamp(-1.0, 1.0).asin();
            let expected = expected_placement_degrees.to_radians();

            assert!((placement_pitch - expected).abs() < 1e-5);
            assert!(placement_pitch <= previous_placement + 1e-5);
            assert!(placement_pitch >= authored_pitch - 1e-5);
            assert!(
                placement_pitch - authored_pitch <= CHARACTER_UPWARD_COMPOSITION_ALLOWANCE + 1e-5
            );
            previous_placement = placement_pitch;
        }
    }

    #[test]
    fn interpolated_uphill_focus_exits_the_destination_floor_without_collapsing() {
        let from = hex_core::HexCoord::ORIGIN;
        let to = hex_core::HexCoord::from_axial(0, 1);
        let level_height = 0.4;
        let progress = 0.5;
        let feet = from.to_world(0.0).lerp(to.to_world(level_height), progress);
        let focus = feet + Vec3::Y * level_height;
        let support = TilePos::new(from, 0);
        let destination = TilePos::new(to, 1);
        let index = CameraObstructionIndex {
            spans_by_coord: BTreeMap::from([
                (
                    from,
                    vec![IndexedCameraSpan {
                        position: support,
                        span: HexSpan::new(-level_height, 0.0),
                    }],
                ),
                (
                    to,
                    vec![IndexedCameraSpan {
                        position: destination,
                        span: HexSpan::new(-level_height, level_height),
                    }],
                ),
            ]),
            initialized: true,
            rebuilds: 1,
        };
        let direction = Vec3::new(0.0, 0.5, 1.0).normalize();

        let clearance = index.safe_radius(focus, support, direction, 7.0, 0.4, 0.35);

        assert!(!clearance.obstructed);
        assert!((clearance.radius - 7.0).abs() < f32::EPSILON);

        let wall = CameraObstructionIndex {
            spans_by_coord: BTreeMap::from([(
                to,
                vec![IndexedCameraSpan {
                    position: TilePos::new(to, 2),
                    span: HexSpan::new(-level_height, focus.y + 0.1),
                }],
            )]),
            initialized: true,
            rebuilds: 1,
        };
        let blocked = wall.safe_radius(focus, support, direction, 7.0, 0.4, 0.35);
        assert!(blocked.obstructed);
        assert!(blocked.radius.abs() < f32::EPSILON);
    }

    #[test]
    fn nearer_hit_sets_the_exact_margin_safe_radius() {
        let obstruction_coord = hex_core::HexCoord::from_axial(0, 1);
        let direction = obstruction_coord.to_world(0.0).normalize();
        let index = CameraObstructionIndex {
            spans_by_coord: BTreeMap::from([(
                obstruction_coord,
                vec![indexed_span(obstruction_coord, HexSpan::new(0.0, 1.0))],
            )]),
            initialized: true,
            rebuilds: 1,
        };

        let clearance = index.safe_radius(Vec3::Y, TilePos::ORIGIN, direction, 7.0, 0.4, 0.35);

        assert!(clearance.obstructed);
        assert!(
            clearance.radius < 1.5,
            "the nearest hit should determine the actual margin-safe clearance"
        );
        let hit = CameraObstruction {
            position: TilePos::new(obstruction_coord, 0),
            center: obstruction_coord.to_world(0.0).xz(),
            span: HexSpan::new(0.0, 2.0),
        }
        .first_hit_distance(Vec3::Y, TilePos::ORIGIN, direction, 7.0, 0.4)
        .expect("the expanded prism should be hit");
        assert!((clearance.radius - (hit - 0.35)).abs() < 1e-5);
    }

    #[test]
    fn flat_radius_55_obstruction_queries_are_deterministic() {
        let (clearance, mut timings) = timed_character_queries(100);
        let p95 = timing_percentile(&mut timings, 95);
        let worst = timings.last().copied().unwrap_or_default();

        assert!(clearance.radius.is_finite());
        assert!(clearance.obstructed);
        eprintln!(
            "synthetic flat radius-55 Character collision diagnostic (debug): \
             p95={p95:?}, worst={worst:?}"
        );
    }

    #[test]
    #[ignore = "manual release-mode synthetic radius-55 Character-camera timing diagnostic"]
    fn flat_radius_55_character_collision_release_timing() {
        let (_clearance, mut timings) = timed_character_queries(10_000);
        let p95 = timing_percentile(&mut timings, 95);
        let worst = timings.last().copied().unwrap_or_default();

        eprintln!(
            "synthetic flat radius-55 Character collision diagnostic (release): \
             p95={p95:?}, worst={worst:?}"
        );
        assert!(
            p95 < Duration::from_millis(1),
            "synthetic flat radius-55 Character collision p95 {p95:?} breached the 1 ms release budget"
        );
    }

    #[test]
    fn collision_retracts_immediately_then_recovers_after_stable_clearance() {
        let mut clear_for = 0.0;
        let retracted = resolve_test_radius(6.0, 2.5, true, false, 0.1, &mut clear_for);
        assert!((retracted - 2.5).abs() < f32::EPSILON);
        assert!(clear_for.abs() < f32::EPSILON);

        let held = resolve_test_radius(retracted, 7.0, false, false, 0.1, &mut clear_for);
        assert!((held - retracted).abs() < f32::EPSILON);
        assert!((clear_for - 0.1).abs() < f32::EPSILON);

        let mut radii = vec![held];
        while *radii
            .last()
            .expect("the recovery sequence starts populated")
            < 7.0
        {
            let previous = *radii.last().expect("the recovery sequence stays populated");
            let next = resolve_test_radius(previous, 7.0, false, false, 0.1, &mut clear_for);
            assert!(next + f32::EPSILON >= previous, "recovery reversed inward");
            assert!(next - previous <= 0.8 + 1e-5);
            assert!(next <= 7.0);
            radii.push(next);
        }
        assert!(
            radii
                .get(1)
                .is_some_and(|radius| (*radius - 3.3).abs() < 1e-5),
            "recovery should advance by exactly the configured 0.8-unit step"
        );
        assert!((radii.last().copied().unwrap_or_default() - 7.0).abs() < f32::EPSILON);
    }

    #[test]
    fn collision_recovery_never_snaps_past_the_configured_rate() {
        let mut clear_for = 0.2;
        let mut effective = 2.35;

        while effective < 7.0 {
            let previous = effective;
            effective = resolve_test_radius(previous, 7.0, false, false, 0.1, &mut clear_for);
            assert!(effective + f32::EPSILON >= previous);
            assert!(
                effective - previous <= 0.8 + 1e-5,
                "the final recovery frame must not add an unbounded hysteresis snap"
            );
        }

        assert!((effective - 7.0).abs() < f32::EPSILON);
    }

    #[test]
    fn worsening_clearance_resets_the_release_delay() {
        let mut clear_for = 0.0;
        let mut effective = 2.5;

        effective = resolve_test_radius(effective, 7.0, false, false, 0.1, &mut clear_for);
        assert!((effective - 2.5).abs() < f32::EPSILON);
        assert!((clear_for - 0.1).abs() < f32::EPSILON);

        effective = resolve_test_radius(effective, 3.0, true, false, 0.1, &mut clear_for);
        assert!(
            (effective - 2.5).abs() < f32::EPSILON,
            "partial blocked clearance must not start an outward/inward camera breath"
        );
        assert!(clear_for.abs() < f32::EPSILON);

        effective = resolve_test_radius(effective, 7.0, false, false, 0.1, &mut clear_for);
        assert!((effective - 2.5).abs() < f32::EPSILON);
        effective = resolve_test_radius(effective, 7.0, false, false, 0.1, &mut clear_for);
        assert!((effective - 3.3).abs() < 1e-5);
    }

    #[test]
    fn obstruction_chatter_never_moves_the_camera_outward() {
        let mut clear_for = 0.0;
        let mut effective = 2.5;
        let mut observed = Vec::new();
        for safe_radius in [2.55, 2.48, 2.53] {
            effective =
                resolve_test_radius(effective, safe_radius, true, false, 0.1, &mut clear_for);
            observed.push(effective);
        }
        assert_eq!(observed, vec![2.5, 2.48, 2.48]);
        assert!(clear_for.abs() < f32::EPSILON);

        effective = resolve_test_radius(effective, 7.0, false, false, 0.1, &mut clear_for);
        assert!((effective - 2.48).abs() < f32::EPSILON);
        effective = resolve_test_radius(effective, 7.0, false, false, 0.1, &mut clear_for);
        assert!(effective > 2.48);
        let interrupted = resolve_test_radius(effective, 2.2, true, false, 0.1, &mut clear_for);
        assert!((interrupted - 2.2).abs() < f32::EPSILON);
        assert!(clear_for.abs() < f32::EPSILON);
    }

    #[test]
    fn a_player_zoom_change_takes_effect_without_collision_lag() {
        let mut clear_for = 0.1;
        let zoomed = resolve_test_radius(2.5, 5.6, false, true, 0.1, &mut clear_for);
        assert!((zoomed - 5.6).abs() < f32::EPSILON);
        assert!(clear_for.abs() < f32::EPSILON);

        let collision_limited = resolve_test_radius(5.6, 3.0, true, true, 0.1, &mut clear_for);
        assert!((collision_limited - 3.0).abs() < f32::EPSILON);
        assert!(clear_for.abs() < f32::EPSILON);
    }

    #[test]
    fn player_zoom_never_releases_an_improved_but_still_blocked_radius() {
        let mut clear_for = 0.1;
        let effective = resolve_test_radius(2.5, 3.0, true, true, 0.1, &mut clear_for);

        assert!(
            (effective - 2.5).abs() < f32::EPSILON,
            "a desired-zoom change must not turn partial blocked clearance into an outward pop"
        );
        assert!(clear_for.abs() < f32::EPSILON);
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
                ..default()
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
    fn a_large_removal_batch_rebuilds_the_obstruction_index_only_once() {
        let mut builder = HeadlessAppBuilder::new().with_minimal_plugins();
        builder
            .app_mut()
            .init_resource::<CameraObstructionIndex>()
            .add_systems(PostUpdate, refresh_camera_obstruction_index);
        let mut app = builder.build();
        let tiles = (0..128)
            .map(|q| {
                app.world_mut()
                    .spawn((
                        HexTile,
                        TilePos::new(hex_core::HexCoord::from_axial(q, 0), 0),
                        HexSpan::new(0.0, 1.0),
                    ))
                    .id()
            })
            .collect::<Vec<_>>();

        app.update();
        assert_eq!(app.world().resource::<CameraObstructionIndex>().rebuilds, 1);
        for tile in tiles {
            app.world_mut().entity_mut(tile).despawn();
        }

        app.update();
        let index = app.world().resource::<CameraObstructionIndex>();
        assert_eq!(index.rebuilds, 2);
        assert!(index.spans_by_coord.is_empty());

        app.update();
        assert_eq!(app.world().resource::<CameraObstructionIndex>().rebuilds, 2);
    }

    #[test]
    fn obstructed_character_camera_preserves_player_rotation_for_ten_thousand_frames() {
        let settings = camera_settings();
        let focus = Vec3::Y * settings.character_focus_height;
        let obstruction_coord = hex_core::HexCoord::from_axial(0, 1);
        let initial_direction = obstruction_coord.to_world(0.0).normalize();
        let authored_rotation = rotation_facing(initial_direction, 0.0);
        let eye = focus + authored_rotation * Vec3::Z * settings.character_radius;
        let mut builder = HeadlessAppBuilder::new().with_minimal_plugins();
        builder
            .app_mut()
            .insert_resource(settings.clone())
            .insert_resource(CameraMode::Character)
            .init_resource::<SavedMapCamera>()
            .init_resource::<CameraObstructionIndex>()
            .insert_resource(CharacterCameraCollision {
                effective_radius: Some(settings.character_radius),
                ..default()
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
                Transform::from_translation(eye).with_rotation(authored_rotation),
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
        let settled = app
            .world()
            .entity(camera)
            .get::<Transform>()
            .expect("the camera should keep its transform");
        assert!(
            settled.rotation.dot(authored_rotation).abs() > 0.999999,
            "collision must not author either pitch or yaw"
        );
        assert!(
            settled.translation.distance(focus) < settings.character_radius,
            "the obstruction should be handled by radius retraction"
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
        let final_rotation = app
            .world()
            .entity(camera)
            .get::<Transform>()
            .expect("the camera should keep its transform")
            .rotation;
        assert!(final_rotation.dot(authored_rotation).abs() > 0.999999);
    }

    #[test]
    fn pitch_delta_cannot_flip_across_either_vertical_pole() {
        for (delta, expected) in [
            (-std::f32::consts::PI, -std::f32::consts::FRAC_PI_2),
            (std::f32::consts::PI, std::f32::consts::FRAC_PI_2),
        ] {
            let rotation = apply_pitch_delta(rotation_at_pitch(0.0), delta, -1.0, 1.0);
            assert!(rotation.is_finite());
            assert!((rotation.length() - 1.0).abs() < 1e-5);
            assert_pitch(rotation, expected);
            assert!(
                downward_pitch(rotation).abs() <= std::f32::consts::FRAC_PI_2 + 1e-5,
                "a large drag crossed a vertical pole and inverted the camera"
            );
        }
    }

    #[test]
    fn pitch_delta_preserves_yaw_through_both_vertical_poles() {
        let yaw = 1.1;
        let before = Quat::from_rotation_y(yaw) * rotation_at_pitch(0.0);
        let expected_right = before * Vec3::X;

        for delta in [-10.0, 10.0] {
            let at_pole = apply_pitch_delta(before, delta, -1.0, 1.0);
            assert!(
                (at_pole * Vec3::X).dot(expected_right) > 0.9999,
                "pitching to a pole discarded the player-authored yaw"
            );

            let yaw_delta = 0.7;
            let turned_at_pole = Quat::from_rotation_y(yaw_delta) * at_pole;
            let away_from_pole = apply_pitch_delta(
                turned_at_pole,
                if delta.is_sign_negative() { 0.2 } else { -0.2 },
                -1.0,
                1.0,
            );
            let wanted_right = Quat::from_rotation_y(yaw_delta) * expected_right;
            assert!(
                (away_from_pole * Vec3::X).dot(wanted_right) > 0.9999,
                "yaw authored at a pole did not reappear after pitching away"
            );
        }
    }

    #[test]
    fn ordinary_right_drag_authors_yaw_and_clamps_character_pitch() {
        let mut builder = HeadlessAppBuilder::new()
            .with_minimal_plugins()
            .with_input();
        builder.app_mut().add_message::<CursorMoved>();
        builder.app_mut().insert_resource(camera_settings());
        builder.app_mut().insert_resource(CameraMode::Character);
        builder
            .app_mut()
            .init_resource::<CharacterCameraCollision>();
        builder.app_mut().add_systems(Update, orbit_camera);
        let window = builder
            .app_mut()
            .world_mut()
            .spawn((
                Window {
                    resolution: bevy::window::WindowResolution::new(1_200, 800),
                    ..default()
                },
                PrimaryWindow,
            ))
            .id();
        let initial_rotation =
            Quat::from_rotation_y(0.4) * rotation_at_pitch(0.3 * std::f32::consts::FRAC_PI_2);
        let focus = Vec3::ZERO;
        let radius = 7.0;
        let camera = builder
            .app_mut()
            .world_mut()
            .spawn((
                PanOrbitCamera { focus, radius },
                Transform {
                    translation: focus
                        + Mat3::from_quat(initial_rotation).mul_vec3(Vec3::new(0.0, 0.0, radius)),
                    rotation: initial_rotation,
                    ..default()
                },
            ))
            .id();
        let mut app = builder.build();

        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Right);
        app.world_mut().write_message(CursorMoved {
            window,
            position: Vec2::new(600.0, 400.0),
            delta: None,
        });
        app.update();
        app.world_mut().write_message(CursorMoved {
            window,
            position: Vec2::new(200.0, 800.0),
            delta: Some(Vec2::new(-400.0, 400.0)),
        });
        app.update();

        let transform = app
            .world()
            .entity(camera)
            .get::<Transform>()
            .expect("the ordinary orbit target should retain its transform");
        let initial_right = initial_rotation * Vec3::X;
        let authored_right = transform.rotation * Vec3::X;
        assert!(
            (initial_right.dot(authored_right) + 0.5).abs() < 1e-4,
            "one-third-turn right drag should author a 120-degree yaw"
        );
        assert_pitch(transform.rotation, std::f32::consts::FRAC_PI_2);
        assert!(
            (transform.rotation * Vec3::NEG_Z).dot(Vec3::NEG_Y) > 0.99999,
            "the downward gesture should reach straight down without inverting"
        );
        let authored_radius = app
            .world()
            .entity(camera)
            .get::<PanOrbitCamera>()
            .expect("orbit state should remain present")
            .radius;
        assert!(
            (authored_radius - radius).abs() < f32::EPSILON,
            "an azimuth gesture must not mutate desired zoom"
        );

        app.world_mut().write_message(CursorMoved {
            window,
            position: Vec2::new(200.0, -800.0),
            delta: Some(Vec2::new(0.0, -1_600.0)),
        });
        app.update();
        let upward = app
            .world()
            .entity(camera)
            .get::<Transform>()
            .expect("the ordinary orbit target should retain its transform");
        assert_pitch(upward.rotation, -std::f32::consts::FRAC_PI_2);
        assert!(
            (upward.rotation * Vec3::NEG_Z).dot(Vec3::Y) > 0.99999,
            "the upward gesture should reach straight up"
        );
    }

    #[test]
    fn right_drag_and_wheel_author_rotation_and_zoom_in_the_same_frame() {
        let mut builder = HeadlessAppBuilder::new()
            .with_minimal_plugins()
            .with_input();
        builder
            .app_mut()
            .add_message::<CursorMoved>()
            .add_message::<MouseWheel>()
            .insert_resource(camera_settings())
            .insert_resource(CameraMode::Character)
            .init_resource::<CharacterCameraCollision>()
            .add_systems(Update, orbit_camera);
        let window = builder
            .app_mut()
            .world_mut()
            .spawn((
                Window {
                    resolution: bevy::window::WindowResolution::new(1_200, 800),
                    ..default()
                },
                PrimaryWindow,
            ))
            .id();
        let initial_rotation = Quat::from_rotation_y(0.4) * rotation_at_pitch(0.0);
        let focus = Vec3::ZERO;
        let radius = 7.0;
        let camera = builder
            .app_mut()
            .world_mut()
            .spawn((
                PanOrbitCamera { focus, radius },
                Transform {
                    translation: focus + initial_rotation * Vec3::Z * radius,
                    rotation: initial_rotation,
                    ..default()
                },
            ))
            .id();
        let mut app = builder.build();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Right);
        app.world_mut().write_message(CursorMoved {
            window,
            position: Vec2::new(600.0, 400.0),
            delta: None,
        });
        app.update();

        let before = *app
            .world()
            .entity(camera)
            .get::<Transform>()
            .expect("the orbit target should retain its transform");
        app.world_mut().write_message(CursorMoved {
            window,
            position: Vec2::new(480.0, 480.0),
            delta: Some(Vec2::new(-120.0, 80.0)),
        });
        app.world_mut().write_message(MouseWheel {
            unit: bevy::input::mouse::MouseScrollUnit::Line,
            x: 0.0,
            y: 1.0,
            window,
            phase: bevy::input::touch::TouchPhase::Moved,
        });
        app.update();

        let entity = app.world().entity(camera);
        let transform = entity
            .get::<Transform>()
            .expect("the orbit target should retain its transform");
        let orbit = entity
            .get::<PanOrbitCamera>()
            .expect("the orbit target should retain its controls");
        assert!(transform.rotation.dot(before.rotation).abs() < 0.9999);
        assert!((orbit.radius - 5.6).abs() < 1e-5);
        let wanted_eye = orbit.focus + transform.rotation * Vec3::Z * orbit.radius;
        assert!(transform.translation.distance(wanted_eye) < 1e-5);
    }

    #[test]
    fn orbit_input_survives_the_same_frame_collision_pass() {
        let (mut app, camera, _) = prototype_camera_app(Some(Vec3::ZERO));
        app.insert_resource(ButtonInput::<MouseButton>::default())
            .add_message::<CursorMoved>()
            .add_message::<MouseWheel>()
            .add_systems(Update, orbit_camera);
        let window = app
            .world_mut()
            .spawn((
                Window {
                    resolution: bevy::window::WindowResolution::new(1_200, 800),
                    ..default()
                },
                PrimaryWindow,
            ))
            .id();
        toggle_camera(&mut app);

        *app.world_mut().resource_mut::<CameraObstructionIndex>() = CameraObstructionIndex {
            spans_by_coord: BTreeMap::from([(
                hex_core::HexCoord::ORIGIN,
                vec![indexed_span(
                    hex_core::HexCoord::ORIGIN,
                    HexSpan::new(-1.0, 20.0),
                )],
            )]),
            initialized: true,
            rebuilds: 1,
        };
        app.update();
        let blocked = camera_pose(&app, camera);
        assert!(
            blocked.0.translation.distance(blocked.1) < blocked.2,
            "the fixture must begin with active collision retraction"
        );

        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Right);
        app.world_mut().write_message(CursorMoved {
            window,
            position: Vec2::new(600.0, 400.0),
            delta: None,
        });
        app.update();
        let baseline = camera_pose(&app, camera).0.rotation;

        app.world_mut().write_message(CursorMoved {
            window,
            position: Vec2::new(720.0, -800.0),
            delta: Some(Vec2::new(120.0, -1_200.0)),
        });
        app.world_mut().write_message(MouseWheel {
            unit: bevy::input::mouse::MouseScrollUnit::Line,
            x: 0.0,
            y: 1.0,
            window,
            phase: bevy::input::touch::TouchPhase::Moved,
        });
        app.update();
        let yaw = Quat::from_rotation_y(-0.2 * std::f32::consts::PI);
        let expected = apply_pitch_delta(yaw * baseline, -1.5 * std::f32::consts::PI, -1.0, 1.0);
        let authored = camera_pose(&app, camera);
        assert!(authored.0.rotation.dot(expected).abs() > 0.999999);
        assert!(
            (authored.0.rotation * Vec3::NEG_Z).dot(Vec3::Y) > 0.99999,
            "collision must not prevent a straight-up look"
        );
        assert!((authored.2 - 5.6).abs() < 1e-5);
        assert!(
            authored.0.translation.distance(authored.1) < f32::EPSILON,
            "the unusual-angle obstruction should retract position without changing look direction"
        );

        for frame in 0..100 {
            app.update();
            let held = camera_pose(&app, camera);
            assert!(
                held.0.rotation.dot(expected).abs() > 0.999999,
                "blocked follow rewrote player rotation on frame {frame}"
            );
        }
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
    fn map_inspection_centers_once_without_mutating_gameplay_authority() {
        let mut builder = HeadlessAppBuilder::new().with_minimal_plugins();
        builder
            .app_mut()
            .insert_resource(camera_settings())
            .init_resource::<CameraMode>()
            .add_message::<CenterInspectionCamera>()
            .add_systems(Update, center_inspection_camera);
        let eye = Vec3::new(0.0, 14.0, 12.0);
        let camera = builder
            .app_mut()
            .world_mut()
            .spawn((
                Transform::from_translation(eye).looking_at(Vec3::ZERO, Vec3::Y),
                PanOrbitCamera {
                    focus: Vec3::ZERO,
                    radius: eye.length(),
                },
            ))
            .id();
        let selected_surface = TilePos::new(hex_core::HexCoord::from_axial(-2, 1), 4);
        let selected = builder
            .app_mut()
            .world_mut()
            .spawn((
                UnitId(1),
                Transform::from_xyz(-3.0, 2.0, 1.0),
                CameraFocusTarget::new(selected_surface),
                hex_core::Turn {
                    movement_left: 3,
                    acted: false,
                },
            ))
            .id();
        let inspected_id = UnitId(2);
        let inspected_surface = TilePos::new(hex_core::HexCoord::from_axial(3, -2), 9);
        let inspected_position = Vec3::new(7.0, 3.0, -5.0);
        let inspected = builder
            .app_mut()
            .world_mut()
            .spawn((
                inspected_id,
                Transform::from_translation(inspected_position),
                InspectionCameraSubject::new(inspected_id, inspected_surface),
            ))
            .id();
        let mut app = builder.build();
        let before = camera_pose(&app, camera);

        app.world_mut()
            .write_message(CenterInspectionCamera::new(inspected_id));
        app.update();

        let centered = camera_pose(&app, camera);
        let wanted_focus = inspected_position + Vec3::Y * camera_settings().character_focus_height;
        let offset = wanted_focus - before.1;
        assert!(centered.1.distance(wanted_focus) < 1e-5);
        assert!(
            centered
                .0
                .translation
                .distance(before.0.translation + offset)
                < 1e-5
        );
        assert_eq!(centered.0.rotation, before.0.rotation);
        assert!((centered.2 - before.2).abs() < f32::EPSILON);

        let movement = Vec3::new(5.0, 0.5, 2.0);
        app.world_mut()
            .entity_mut(inspected)
            .get_mut::<Transform>()
            .expect("the inspected unit should have a transform")
            .translation += movement;
        app.update();
        let held = camera_pose(&app, camera);
        assert_eq!(held.0, centered.0);
        assert_eq!(held.1, centered.1);

        app.world_mut()
            .write_message(CenterInspectionCamera::new(inspected_id));
        app.update();
        let recentered = camera_pose(&app, camera);
        assert!(recentered.1.distance(wanted_focus + movement) < 1e-5);

        app.world_mut()
            .write_message(CenterInspectionCamera::new(UnitId(99)));
        app.update();
        let refused = camera_pose(&app, camera);
        assert_eq!(refused.0, recentered.0);
        assert_eq!(refused.1, recentered.1);

        let selected_entity = app.world().entity(selected);
        assert_eq!(
            selected_entity
                .get::<CameraFocusTarget>()
                .map(|target| target.surface),
            Some(selected_surface)
        );
        assert_eq!(
            selected_entity
                .get::<hex_core::Turn>()
                .map(|turn| (turn.movement_left, turn.acted)),
            Some((3, false))
        );
        assert!(
            !app.world()
                .entity(inspected)
                .contains::<CameraFocusTarget>(),
            "presentation inspection must not become gameplay selection authority"
        );
    }

    #[test]
    fn character_camera_follows_authorized_inspection_then_falls_back_to_selection() {
        let selected_position = Vec3::new(-4.0, 1.0, 2.0);
        let (mut app, camera, selected) = prototype_camera_app(Some(selected_position));
        let selected = selected.expect("the fixture should retain its selected target");
        app.world_mut().entity_mut(selected).insert(hex_core::Turn {
            movement_left: 2,
            acted: false,
        });
        let inspected_id = UnitId(27);
        let inspected_position = Vec3::new(8.0, 2.0, -6.0);
        let inspected = app
            .world_mut()
            .spawn((
                inspected_id,
                Transform::from_translation(inspected_position),
                InspectionCameraSubject::new(inspected_id, TilePos::ORIGIN),
            ))
            .id();

        toggle_camera(&mut app);

        assert_eq!(*app.world().resource::<CameraMode>(), CameraMode::Character);
        let inspected_pose = camera_pose(&app, camera);
        let inspected_focus =
            inspected_position + Vec3::Y * camera_settings().character_focus_height;
        assert!(inspected_pose.1.distance(inspected_focus) < 1e-5);

        let movement = Vec3::new(1.25, 0.4, -0.75);
        app.world_mut()
            .entity_mut(inspected)
            .get_mut::<Transform>()
            .expect("the inspected unit should retain its transform")
            .translation += movement;
        app.update();
        assert!(
            camera_pose(&app, camera)
                .1
                .distance(inspected_focus + movement)
                < 1e-5
        );

        app.world_mut()
            .entity_mut(inspected)
            .remove::<InspectionCameraSubject>();
        app.update();
        let fallback = camera_pose(&app, camera);
        let selected_focus = selected_position + Vec3::Y * camera_settings().character_focus_height;
        assert!(fallback.1.distance(selected_focus) < 1e-5);
        assert_eq!(*app.world().resource::<CameraMode>(), CameraMode::Character);

        let selected_entity = app.world().entity(selected);
        assert!(selected_entity.contains::<CameraFocusTarget>());
        assert_eq!(
            selected_entity
                .get::<hex_core::Turn>()
                .map(|turn| (turn.movement_left, turn.acted)),
            Some((2, false))
        );
    }

    fn install_tall_camera_wall(app: &mut App, focus: Vec3, direction: Vec3, distance: f32) {
        let coord = hex_core::HexCoord::from_world(focus + direction * distance);
        *app.world_mut().resource_mut::<CameraObstructionIndex>() = CameraObstructionIndex {
            spans_by_coord: BTreeMap::from([(
                coord,
                vec![indexed_span(coord, HexSpan::new(-10.0, 20.0))],
            )]),
            initialized: true,
            rebuilds: 1,
        };
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
    fn character_camera_retracts_before_public_terrain_without_rotating() {
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
            spans_by_coord: BTreeMap::from([(
                obstacle_coord,
                vec![indexed_span(obstacle_coord, HexSpan::new(lower, upper))],
            )]),
            initialized: true,
            rebuilds: 1,
        };

        toggle_camera(&mut app);

        let shortened = camera_pose(&app, camera);
        let collision = app.world().resource::<CharacterCameraCollision>();
        let effective = collision
            .effective_radius
            .expect("Character mode should retain an effective radius");
        assert!(
            effective < camera_settings().character_radius,
            "an obstruction must retract the camera along the authored boom"
        );
        assert!(
            (shortened.0.translation.distance(shortened.1) - effective).abs() < 1e-5,
            "the rendered eye must use the collision-limited radius"
        );
        assert!(
            (shortened.2 - camera_settings().character_radius).abs() < f32::EPSILON,
            "collision must not overwrite the player's requested orbit radius"
        );
        assert!(
            unobstructed.0.rotation.dot(shortened.0.rotation).abs() > 0.999999,
            "collision must preserve the complete player-authored rotation"
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

    #[test]
    fn a_clear_new_focus_target_does_not_inherit_the_old_targets_retracted_boom() {
        let (mut app, camera, old_target) = prototype_camera_app(Some(Vec3::ZERO));
        let old_target = old_target.expect("the fixture should spawn a target");
        toggle_camera(&mut app);
        let initial = camera_pose(&app, camera);
        let direction = character_boom_direction(initial.0.rotation);
        install_tall_camera_wall(&mut app, initial.1, direction, 2.0);
        app.update();
        let retracted = camera_pose(&app, camera);
        let old_effective_radius = retracted.0.translation.distance(retracted.1);
        assert!(old_effective_radius < retracted.2 - 1e-4);

        app.world_mut()
            .entity_mut(old_target)
            .remove::<CameraFocusTarget>();
        app.world_mut()
            .resource_mut::<CameraObstructionIndex>()
            .spans_by_coord
            .clear();
        let replacement_position = Vec3::new(40.0, 1.0, 0.0);
        let replacement_surface =
            TilePos::new(hex_core::HexCoord::from_world(replacement_position), 0);
        app.world_mut().spawn((
            Transform::from_translation(replacement_position),
            CameraFocusTarget::new(replacement_surface),
        ));

        app.update();

        let retargeted = camera_pose(&app, camera);
        assert!(
            (retargeted.0.translation.distance(retargeted.1) - retargeted.2).abs() < 1e-5,
            "a clear new target must begin at the player's desired boom, not recover from the old target"
        );
    }

    #[test]
    fn a_blocked_new_focus_target_resolves_its_own_safe_boom_immediately() {
        let (mut app, camera, old_target) = prototype_camera_app(Some(Vec3::ZERO));
        let old_target = old_target.expect("the fixture should spawn a target");
        toggle_camera(&mut app);
        let initial = camera_pose(&app, camera);
        let direction = character_boom_direction(initial.0.rotation);
        install_tall_camera_wall(&mut app, initial.1, direction, 2.0);
        app.update();
        let retracted = camera_pose(&app, camera);
        let old_effective_radius = retracted.0.translation.distance(retracted.1);
        assert!(old_effective_radius < retracted.2 - 1e-4);

        app.world_mut()
            .entity_mut(old_target)
            .remove::<CameraFocusTarget>();
        let replacement_position = Vec3::new(40.0, 1.0, 0.0);
        let replacement_surface =
            TilePos::new(hex_core::HexCoord::from_world(replacement_position), 0);
        let replacement_focus =
            replacement_position + Vec3::Y * camera_settings().character_focus_height;
        install_tall_camera_wall(&mut app, replacement_focus, direction, 4.0);
        let settings = camera_settings();
        let expected = app
            .world()
            .resource::<CameraObstructionIndex>()
            .safe_radius(
                replacement_focus,
                replacement_surface,
                direction,
                settings.character_radius,
                settings.character_probe_radius,
                settings.character_collision_margin,
            );
        assert!(expected.obstructed);
        assert!(expected.radius > old_effective_radius + 1e-4);
        app.world_mut().spawn((
            Transform::from_translation(replacement_position),
            CameraFocusTarget::new(replacement_surface),
        ));

        app.update();

        let retargeted = camera_pose(&app, camera);
        assert!(
            (retargeted.0.translation.distance(retargeted.1) - expected.radius).abs() < 1e-5,
            "a blocked new target must resolve its own clearance instead of inheriting the old boom"
        );
    }

    #[test]
    fn unobstructed_motion_preserves_the_exact_player_authored_composition() {
        let start = Vec3::new(2.0, 1.0, -3.0);
        let (mut app, camera, target) = prototype_camera_app(Some(start));
        let target = target.expect("the fixture should spawn a target");
        toggle_camera(&mut app);
        let initial = camera_pose(&app, camera);
        let authored_rotation = initial.0.rotation;
        let desired_radius = camera_settings().character_radius;
        let step = Vec3::new(0.017, 0.003, -0.011);
        let mut previous_eye = initial.0.translation;

        for frame in 0..120 {
            let target_translation = {
                let mut target = app.world_mut().entity_mut(target);
                let mut transform = target
                    .get_mut::<Transform>()
                    .expect("the target should have a transform");
                transform.translation += step;
                transform.translation
            };
            app.update();

            let pose = camera_pose(&app, camera);
            let expected_focus =
                target_translation + Vec3::Y * camera_settings().character_focus_height;
            assert!(
                pose.1.distance(expected_focus) < 1e-5,
                "frame {frame} did not follow the selected unit exactly"
            );
            assert!(
                pose.0.rotation.dot(authored_rotation).abs() > 0.999999,
                "frame {frame} changed the player's perspective"
            );
            assert!((pose.2 - desired_radius).abs() < f32::EPSILON);
            assert!(
                (pose.0.translation.distance(expected_focus) - desired_radius).abs() < 1e-5,
                "frame {frame} changed the camera distance in open space"
            );
            assert!(
                pose.0
                    .translation
                    .distance(expected_focus + authored_rotation * Vec3::Z * desired_radius)
                    < 1e-5
            );
            assert!(
                (pose.0.translation - previous_eye).distance(step) < 1e-5,
                "frame {frame} introduced motion unrelated to character movement"
            );
            previous_eye = pose.0.translation;
        }
    }

    #[test]
    fn shallow_upward_free_look_stays_stable_while_the_character_walks() {
        let start = Vec3::new(2.0, 1.0, -3.0);
        let (mut app, camera, target) = prototype_camera_app(Some(start));
        let target = target.expect("the fixture should spawn a target");
        toggle_camera(&mut app);
        let authored_rotation = rotation_at_pitch(-10.0_f32.to_radians());
        app.world_mut()
            .entity_mut(camera)
            .get_mut::<Transform>()
            .expect("the camera should retain its transform")
            .rotation = authored_rotation;
        app.update();

        let desired_radius = camera_settings().character_radius;
        let eye_offset = character_boom_direction(authored_rotation) * desired_radius;
        let step = Vec3::new(0.017, 0.003, -0.011);
        let mut previous_eye = camera_pose(&app, camera).0.translation;
        for frame in 0..120 {
            let target_translation = {
                let mut target = app.world_mut().entity_mut(target);
                let mut transform = target
                    .get_mut::<Transform>()
                    .expect("the target should retain its transform");
                transform.translation += step;
                transform.translation
            };
            app.update();

            let pose = camera_pose(&app, camera);
            let expected_focus =
                target_translation + Vec3::Y * camera_settings().character_focus_height;
            assert!(pose.0.rotation.dot(authored_rotation).abs() > 0.999999);
            assert!((pose.2 - desired_radius).abs() < f32::EPSILON);
            assert!(
                pose.0.translation.distance(expected_focus + eye_offset) < 1e-5,
                "frame {frame} changed the player-controlled shallow-upward composition"
            );
            assert!(
                (pose.0.translation - previous_eye).distance(step) < 1e-5,
                "frame {frame} introduced camera motion unrelated to walking"
            );
            previous_eye = pose.0.translation;
        }
    }

    #[test]
    fn moving_past_partial_clearance_never_recreates_camera_breathing() {
        let start = Vec3::ZERO;
        let (mut app, camera, target) = prototype_camera_app(Some(start));
        let target = target.expect("the fixture should spawn a target");
        toggle_camera(&mut app);
        let initial = camera_pose(&app, camera);
        let authored_rotation = initial.0.rotation;
        let boom = character_boom_direction(authored_rotation);
        let ground_boom = boom.xz().normalize();
        let wall_center = initial.1 + boom * 4.0;
        let wall_coord = hex_core::HexCoord::from_world(wall_center);
        *app.world_mut().resource_mut::<CameraObstructionIndex>() = CameraObstructionIndex {
            spans_by_coord: BTreeMap::from([(
                wall_coord,
                vec![indexed_span(wall_coord, HexSpan::new(-10.0, 20.0))],
            )]),
            initialized: true,
            rebuilds: 1,
        };
        app.update();

        let desired_radius = camera_settings().character_radius;
        let mut previous_radius = camera_pose(&app, camera)
            .0
            .translation
            .distance(camera_pose(&app, camera).1);
        assert!(previous_radius < desired_radius);

        // Walking away from the wall improves its partial clearance, but the wall
        // remains present. The camera must hold instead of following every increase.
        for frame in 0..10 {
            let target_translation = {
                let mut entity = app.world_mut().entity_mut(target);
                let mut transform = entity
                    .get_mut::<Transform>()
                    .expect("the target should retain its transform");
                transform.translation -= Vec3::new(ground_boom.x, 0.0, ground_boom.y) * 0.04;
                transform.translation
            };
            app.update();
            let pose = camera_pose(&app, camera);
            let radius = pose.0.translation.distance(pose.1);
            assert!(
                (radius - previous_radius).abs() < 1e-5,
                "frame {frame} released into still-blocked partial clearance"
            );
            assert!(pose.0.rotation.dot(authored_rotation).abs() > 0.999999);
            assert!((pose.2 - desired_radius).abs() < f32::EPSILON);
            assert!(
                pose.1.distance(
                    target_translation + Vec3::Y * camera_settings().character_focus_height
                ) < 1e-5
            );
        }

        // Reversing past the starting point worsens clearance and may retract only
        // inward. It must never produce an outward/inward alternating sequence.
        let mut retracted_further = false;
        for frame in 0..15 {
            {
                let mut entity = app.world_mut().entity_mut(target);
                entity
                    .get_mut::<Transform>()
                    .expect("the target should retain its transform")
                    .translation += Vec3::new(ground_boom.x, 0.0, ground_boom.y) * 0.05;
            }
            app.update();
            let pose = camera_pose(&app, camera);
            let radius = pose.0.translation.distance(pose.1);
            assert!(
                radius <= previous_radius + 1e-5,
                "frame {frame} moved outward while the wall remained"
            );
            assert!(pose.0.rotation.dot(authored_rotation).abs() > 0.999999);
            assert!((pose.2 - desired_radius).abs() < f32::EPSILON);
            retracted_further |= radius < previous_radius - 1e-5;
            previous_radius = radius;
        }
        assert!(
            retracted_further,
            "worsening wall clearance must produce at least one strict inward retraction"
        );

        app.world_mut()
            .resource_mut::<CameraObstructionIndex>()
            .spans_by_coord
            .clear();
        let held = previous_radius;
        app.update();
        let first_clear = camera_pose(&app, camera);
        assert!((first_clear.0.translation.distance(first_clear.1) - held).abs() < 1e-5);

        let mut restored = held;
        for _ in 0..16 {
            app.update();
            let pose = camera_pose(&app, camera);
            let radius = pose.0.translation.distance(pose.1);
            assert!(radius + 1e-5 >= restored);
            assert!(radius - restored <= 0.8 + 1e-5);
            assert!(pose.0.rotation.dot(authored_rotation).abs() > 0.999999);
            assert!((pose.2 - desired_radius).abs() < f32::EPSILON);
            restored = radius;
            if (restored - desired_radius).abs() < 1e-5 {
                break;
            }
        }
        assert!((restored - desired_radius).abs() < 1e-5);
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

    #[test]
    fn one_hundred_gameplay_lifecycles_leave_one_camera_and_no_collision_state() {
        let mut app = sky_app();

        for cycle in 0..100 {
            enter(&mut app, Screen::Gameplay);
            assert_eq!(*app.world().resource::<CameraMode>(), CameraMode::Map);
            assert!(app.world().resource::<SavedMapCamera>().0.is_none());
            let camera_count = {
                let mut cameras = app
                    .world_mut()
                    .query_filtered::<Entity, With<PanOrbitCamera>>();
                cameras.iter(app.world()).count()
            };
            assert_eq!(
                camera_count, 1,
                "cycle {cycle} duplicated the global camera"
            );
            let dome_count = {
                let mut domes = app.world_mut().query_filtered::<Entity, With<SkyDome>>();
                domes.iter(app.world()).count()
            };
            assert_eq!(dome_count, 1, "cycle {cycle} duplicated the sky dome");

            {
                let mut index = app.world_mut().resource_mut::<CameraObstructionIndex>();
                index.initialized = true;
                index.spans_by_coord.insert(
                    hex_core::HexCoord::ORIGIN,
                    vec![indexed_span(
                        hex_core::HexCoord::ORIGIN,
                        HexSpan::new(0.0, 0.4),
                    )],
                );
            }
            *app.world_mut().resource_mut::<CharacterCameraCollision>() =
                CharacterCameraCollision {
                    target: None,
                    effective_radius: Some(2.0),
                    last_desired_radius: Some(7.0),
                    outward_clear_for_seconds: 0.15,
                };

            enter(&mut app, Screen::Title);
            let index = app.world().resource::<CameraObstructionIndex>();
            assert!(!index.initialized);
            assert!(index.spans_by_coord.is_empty());
            let collision = app.world().resource::<CharacterCameraCollision>();
            assert!(collision.target.is_none());
            assert!(collision.effective_radius.is_none());
            assert!(collision.last_desired_radius.is_none());
            assert!(collision.outward_clear_for_seconds.abs() < f32::EPSILON);
        }
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
