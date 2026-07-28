//! Standalone 3D viewport presentation and pointer translation.
//!
//! This module deliberately consumes complete, resolved snapshots. It knows nothing
//! about project files, undo history, editor tools, or mutable object drafts. The UI
//! translates those concerns into [`ViewportContent`] and consumes the picking
//! messages published here.

use std::collections::{BTreeMap, BTreeSet};

use bevy::gltf::GltfAssetLabel;
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::light::{GlobalAmbientLight, NotShadowCaster};
use bevy::picking::events::{Click, Move, Out, Over, Pointer};
use bevy::picking::mesh_picking::MeshPickingPlugin;
use bevy::picking::pointer::PointerButton;
use bevy::prelude::*;
use bevy::window::{CursorMoved, PrimaryWindow};
use hex_assets::{
    EffectPart, LocalAxialCoord, LocalVoxelCoord, ObjectPart, PlantPart, PropPart, SrgbColor,
    VoxelStyleId, VoxelSurfaceMode, MAX_OBJECT_RADIUS,
};
use hex_core::config::{HEX_CIRCUMRADIUS, HEX_SMALL_DIAMETER};

/// World-space height of one authored voxel level.
pub const DEFAULT_LEVEL_HEIGHT: f32 = 0.4;

const HEX_MESH: &str = "meshes/hex.glb";
const DEFAULT_GRID_RADIUS: u8 = 6;
const GUIDE_THICKNESS: f32 = 0.012;
const GRID_LINE_LIFT: f32 = 0.008;
const MIN_CAMERA_RADIUS: f32 = 2.5;
const MAX_CAMERA_RADIUS: f32 = 120.0;
const MIN_CAMERA_PITCH: f32 = 0.02;
const MAX_CAMERA_PITCH: f32 = std::f32::consts::FRAC_PI_2;
const ORBIT_SENSITIVITY: f32 = 1.0;
const PAN_SCREEN_SCALE: f32 = 2.0;
const ZOOM_SENSITIVITY: f32 = 0.12;

/// Which Workshop view the 3D viewport is presenting.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ViewportMode {
    /// One material/style voxel on a minimal guide.
    #[default]
    StylePreview,
    /// A complete authored object over its active-level grid.
    Object,
}

/// Deterministic lighting used to inspect authored colours and surfaces.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ViewportPreviewRig {
    /// Neutral key light, soft ambient fill, and shadows.
    #[default]
    Neutral,
    /// Low cool fill for checking emission and silhouette readability.
    Dark,
    /// Uniform material colour with lighting disabled.
    Unlit,
}

/// Whether viewport pointer and camera input should be accepted this frame.
///
/// The UI sets this to `false` whenever egui wants pointer or keyboard input. Picking
/// observers and camera controls both honor it, so clicking a panel cannot edit or
/// orbit the scene behind that panel.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewportInputEnabled(pub bool);

impl Default for ViewportInputEnabled {
    fn default() -> Self {
        Self(true)
    }
}

/// A resolved emissive treatment ready for viewport rendering.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewportEmission {
    /// Emitted sRGB colour.
    pub color: SrgbColor,
    /// Finite nonnegative intensity from the validated style catalog.
    pub strength: f32,
}

/// A palette-resolved voxel style ready for viewport rendering.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewportStyle {
    /// Base sRGB colour.
    pub color: SrgbColor,
    /// Renderer treatment authored by the style.
    pub surface_mode: VoxelSurfaceMode,
    /// Validated opacity in `0.0 < opacity <= 1.0`.
    pub opacity: f32,
    /// Optional palette-resolved emission.
    pub emission: Option<ViewportEmission>,
}

/// One rendered object-local voxel.
///
/// This is both snapshot data and the component attached to its viewport entity.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct RenderedVoxel {
    /// Exact object-local cell.
    pub position: LocalVoxelCoord,
    /// Stable style id resolved through [`ViewportContent::styles`].
    pub style: VoxelStyleId,
}

/// Complete presentation snapshot consumed by the viewport.
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct ViewportContent {
    /// Occupied cells to draw.
    pub voxels: Vec<RenderedVoxel>,
    /// Resolved shared materials, keyed by stable style id.
    pub styles: BTreeMap<VoxelStyleId, ViewportStyle>,
    /// Horizontal radius of the object authoring guide.
    pub grid_radius: u8,
    /// Level on which empty placement guides are pickable.
    pub active_level: i32,
    /// Whether only voxels on [`Self::active_level`] are drawn.
    pub isolate_active_level: bool,
    /// Whether the ground and grid guide are drawn.
    pub show_grid: bool,
    /// Exact selected occupied cells.
    pub selected_cells: BTreeSet<LocalVoxelCoord>,
    /// Semantic roles keyed by occupied cell.
    pub semantic_parts: BTreeMap<LocalVoxelCoord, ObjectPart>,
    /// Exact horizontal blocker columns.
    pub blocker_columns: BTreeSet<LocalAxialCoord>,
    /// Exact occupied canopy-cutaway cells.
    pub canopy_cells: BTreeSet<LocalVoxelCoord>,
    /// Whether semantic-role rings are drawn.
    pub show_semantic_overlay: bool,
    /// Whether blocker-column rings are drawn.
    pub show_blocker_overlay: bool,
    /// Whether canopy-cell rings are drawn.
    pub show_canopy_overlay: bool,
}

impl Default for ViewportContent {
    fn default() -> Self {
        Self {
            voxels: Vec::new(),
            styles: BTreeMap::new(),
            grid_radius: DEFAULT_GRID_RADIUS,
            active_level: 0,
            isolate_active_level: false,
            show_grid: true,
            selected_cells: BTreeSet::new(),
            semantic_parts: BTreeMap::new(),
            blocker_columns: BTreeSet::new(),
            canopy_cells: BTreeSet::new(),
            show_semantic_overlay: false,
            show_blocker_overlay: false,
            show_canopy_overlay: false,
        }
    }
}

impl ViewportContent {
    /// Replaces the occupied-cell snapshot in deterministic position/style order.
    pub fn set_voxels(&mut self, mut voxels: Vec<RenderedVoxel>) {
        voxels.sort_by(|left, right| {
            left.position
                .cmp(&right.position)
                .then_with(|| left.style.cmp(&right.style))
        });
        self.voxels = voxels;
    }

    /// Replaces the complete resolved style table.
    pub fn set_styles(&mut self, styles: BTreeMap<VoxelStyleId, ViewportStyle>) {
        self.styles = styles;
    }
}

/// Requests replacement of the complete viewport snapshot.
///
/// Multiple updates in one frame collapse to the last message.
#[derive(Message, Debug, Clone)]
pub struct ViewportContentUpdate {
    /// Replacement snapshot.
    pub content: ViewportContent,
}

impl ViewportContentUpdate {
    /// Creates a complete snapshot replacement.
    #[must_use]
    pub const fn new(content: ViewportContent) -> Self {
        Self { content }
    }
}

/// Fixed camera orientations offered by the viewport toolbar.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum CameraSnap {
    /// Oblique three-quarter authoring view.
    #[default]
    Perspective,
    /// Straight down along world Y.
    Top,
    /// Straight along negative world Z.
    Front,
    /// Straight along negative world X.
    Side,
}

/// Requests one fixed camera orientation while preserving focus and zoom.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct CameraSnapRequest(pub CameraSnap);

/// Requests camera framing around the currently visible content.
#[derive(Message, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FrameViewportRequest;

/// Whether a picking hit came from content or an empty placement guide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewportPickSource {
    /// An occupied authored voxel.
    Voxel,
    /// An empty-cell guide on the active level.
    Grid,
}

/// Exact viewport face currently addressed by the pointer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewportFaceTarget {
    /// Picked render entity.
    pub entity: Entity,
    /// Object-local cell associated with the mesh.
    pub cell: LocalVoxelCoord,
    /// Whether this was occupied content or an active-level guide.
    pub source: ViewportPickSource,
    /// World-space hit point reported by mesh picking.
    pub world_position: Vec3,
    /// Normalized world-space face normal reported by mesh picking.
    pub normal: Vec3,
}

/// Current hover target, or `None` while the pointer is outside the viewport content.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq)]
pub struct HoveredFaceTarget(pub Option<ViewportFaceTarget>);

/// Reports every actual change to [`HoveredFaceTarget`].
#[derive(Message, Debug, Clone, Copy, PartialEq)]
pub struct ViewportHoverChanged(pub Option<ViewportFaceTarget>);

/// Reports a click on an occupied voxel or active-level grid cell.
#[derive(Message, Debug, Clone, Copy, PartialEq)]
pub struct ViewportFaceClicked {
    /// Pointer button that completed the click.
    pub button: PointerButton,
    /// Exact picked face.
    pub target: ViewportFaceTarget,
}

/// Orbit state attached to the standalone viewport camera.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct ViewportCamera {
    /// World-space orbit focus.
    pub focus: Vec3,
    /// Distance from the focus.
    pub radius: f32,
    /// Horizontal rotation around world Y, in radians.
    pub yaw: f32,
    /// Angle above the horizon, in radians.
    pub pitch: f32,
}

impl Default for ViewportCamera {
    fn default() -> Self {
        Self {
            focus: Vec3::new(0.0, 2.4, 0.0),
            radius: 15.0,
            yaw: std::f32::consts::FRAC_PI_4,
            pitch: 0.62,
        }
    }
}

impl ViewportCamera {
    /// Applies a fixed orientation without changing framing.
    pub fn snap(&mut self, snap: CameraSnap) {
        let (yaw, pitch) = snap_angles(snap);
        self.yaw = yaw;
        self.pitch = pitch;
    }

    /// Produces the camera transform represented by this orbit state.
    #[must_use]
    pub fn transform(self) -> Transform {
        camera_transform(self)
    }
}

/// Public ordering points for application/UI adapters around viewport work.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ViewportSystems {
    /// Apply complete snapshots and preview-light changes.
    Reconcile,
    /// Consume camera requests and pointer input.
    Input,
    /// Draw transient grid and hover guides.
    Present,
}

#[derive(Component)]
struct ViewportManaged;

#[derive(Component, Debug, Clone, Copy)]
struct ViewportPickCell {
    cell: LocalVoxelCoord,
    source: ViewportPickSource,
}

#[derive(Component)]
struct ViewportKeyLight;

#[derive(Resource)]
struct ViewportRenderAssets {
    hex_mesh: Handle<Mesh>,
    guide_material: Handle<StandardMaterial>,
    missing_material: Handle<StandardMaterial>,
    content_materials: BTreeMap<VoxelStyleId, CachedViewportMaterial>,
}

struct CachedViewportMaterial {
    source: ViewportStyle,
    rig: ViewportPreviewRig,
    handle: Handle<StandardMaterial>,
}

#[derive(Resource, Default)]
struct ViewportSceneCache {
    voxels: BTreeMap<LocalVoxelCoord, CachedVoxelEntity>,
    grid: BTreeMap<LocalVoxelCoord, Entity>,
}

struct CachedVoxelEntity {
    entity: Entity,
    style: VoxelStyleId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum VoxelSceneAction {
    Despawn(LocalVoxelCoord),
    Spawn(RenderedVoxel),
    Restyle(RenderedVoxel),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CameraDragMode {
    Orbit,
    Pan,
}

#[derive(Default)]
struct CameraDrag {
    mode: Option<CameraDragMode>,
    last_cursor: Option<Vec2>,
}

/// Registers viewport rendering, camera controls, lighting, and picking translation.
pub fn plugin(app: &mut App) {
    if !app.is_plugin_added::<MeshPickingPlugin>() {
        app.add_plugins(MeshPickingPlugin);
    }

    app.init_resource::<ViewportMode>()
        .init_resource::<ViewportPreviewRig>()
        .init_resource::<ViewportInputEnabled>()
        .init_resource::<ViewportContent>()
        .init_resource::<ViewportSceneCache>()
        .init_resource::<HoveredFaceTarget>()
        .insert_resource(GlobalAmbientLight::default())
        .insert_resource(ClearColor(Color::srgb(0.035, 0.04, 0.045)))
        .add_message::<ViewportContentUpdate>()
        .add_message::<CameraSnapRequest>()
        .add_message::<FrameViewportRequest>()
        .add_message::<ViewportHoverChanged>()
        .add_message::<ViewportFaceClicked>()
        .configure_sets(
            Update,
            (
                ViewportSystems::Reconcile,
                ViewportSystems::Input,
                ViewportSystems::Present,
            )
                .chain(),
        )
        .add_systems(Startup, spawn_viewport)
        .add_systems(
            Update,
            (apply_content_updates, rebuild_viewport, apply_preview_rig)
                .chain()
                .in_set(ViewportSystems::Reconcile),
        )
        .add_systems(
            Update,
            (
                handle_camera_requests,
                control_camera,
                clear_hover_when_input_disabled,
            )
                .in_set(ViewportSystems::Input),
        )
        .add_systems(Update, draw_guides.in_set(ViewportSystems::Present))
        .add_observer(on_pointer_over)
        .add_observer(on_pointer_move)
        .add_observer(on_pointer_out)
        .add_observer(on_pointer_click);
}

fn spawn_viewport(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let hex_mesh = asset_server.load(
        GltfAssetLabel::Primitive {
            mesh: 0,
            primitive: 0,
        }
        .from_asset(HEX_MESH),
    );
    let guide_material = materials.add(StandardMaterial {
        base_color: Color::srgba(0.32, 0.36, 0.40, 0.10),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        perceptual_roughness: 1.0,
        ..default()
    });
    let missing_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.92, 0.12, 0.52),
        unlit: true,
        ..default()
    });
    commands.insert_resource(ViewportRenderAssets {
        hex_mesh,
        guide_material,
        missing_material,
        content_materials: BTreeMap::new(),
    });

    let camera = ViewportCamera::default();
    commands.spawn((
        Camera3d::default(),
        camera.transform(),
        camera,
        Name::new("Asset Workshop Camera"),
    ));
    commands.spawn((
        DirectionalLight {
            color: Color::srgb(1.0, 0.96, 0.90),
            illuminance: 8_500.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(9.0, 14.0, 7.0).looking_at(Vec3::ZERO, Vec3::Y),
        ViewportKeyLight,
        Name::new("Asset Workshop Key Light"),
    ));
}

fn apply_content_updates(
    mut updates: MessageReader<ViewportContentUpdate>,
    mut content: ResMut<ViewportContent>,
) {
    let mut replacement = None;
    for update in updates.read() {
        replacement = Some(update.content.clone());
    }
    if let Some(replacement) = replacement {
        *content = replacement;
    }
}

fn rebuild_viewport(
    mut commands: Commands,
    content: Res<ViewportContent>,
    mode: Res<ViewportMode>,
    rig: Res<ViewportPreviewRig>,
    assets: Option<ResMut<ViewportRenderAssets>>,
    mut scene: ResMut<ViewportSceneCache>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if !content.is_changed() && !mode.is_changed() && !rig.is_changed() {
        return;
    }
    let Some(mut assets) = assets else {
        return;
    };

    let stale_materials: Vec<_> = assets
        .content_materials
        .keys()
        .filter(|id| !content.styles.contains_key(*id))
        .cloned()
        .collect();
    let mut rebind_styles = BTreeSet::new();
    for id in stale_materials {
        if let Some(cached) = assets.content_materials.remove(&id) {
            drop(materials.remove(cached.handle.id()));
            rebind_styles.insert(id);
        }
    }
    for (id, style) in &content.styles {
        match assets.content_materials.get_mut(id) {
            Some(cached) if cached.source == *style && cached.rig == *rig => {}
            Some(cached) => {
                let reused_handle =
                    if let Some(material) = materials.get_mut_untracked(&cached.handle) {
                        *material = material_for(*style, *rig);
                        true
                    } else {
                        false
                    };
                if !reused_handle {
                    cached.handle = materials.add(material_for(*style, *rig));
                    rebind_styles.insert(id.clone());
                }
                cached.source = *style;
                cached.rig = *rig;
            }
            None => {
                rebind_styles.insert(id.clone());
                assets.content_materials.insert(
                    id.clone(),
                    CachedViewportMaterial {
                        source: *style,
                        rig: *rig,
                        handle: materials.add(material_for(*style, *rig)),
                    },
                );
            }
        }
    }

    let desired_voxels: BTreeMap<_, _> = content
        .voxels
        .iter()
        .filter(|voxel| {
            !content.isolate_active_level
                || *mode == ViewportMode::StylePreview
                || voxel.position.level == content.active_level
        })
        .map(|voxel| (voxel.position, voxel.style.clone()))
        .collect();
    let actions = plan_voxel_reconciliation(&scene.voxels, &desired_voxels, &rebind_styles);
    let mut missing_styles = BTreeSet::new();
    for action in actions {
        match action {
            VoxelSceneAction::Despawn(position) => {
                if let Some(cached) = scene.voxels.remove(&position) {
                    commands.entity(cached.entity).despawn();
                }
            }
            VoxelSceneAction::Spawn(voxel) => {
                let material = material_handle(&assets, &voxel.style, &mut missing_styles);
                let entity = commands
                    .spawn((
                        Mesh3d(assets.hex_mesh.clone()),
                        MeshMaterial3d(material),
                        voxel_transform(voxel.position),
                        voxel.clone(),
                        ViewportPickCell {
                            cell: voxel.position,
                            source: ViewportPickSource::Voxel,
                        },
                        ViewportManaged,
                        Name::new(voxel_name(voxel.position)),
                    ))
                    .id();
                scene.voxels.insert(
                    voxel.position,
                    CachedVoxelEntity {
                        entity,
                        style: voxel.style,
                    },
                );
            }
            VoxelSceneAction::Restyle(voxel) => {
                let material = material_handle(&assets, &voxel.style, &mut missing_styles);
                if let Some(cached) = scene.voxels.get_mut(&voxel.position) {
                    commands.entity(cached.entity).insert((
                        MeshMaterial3d(material),
                        voxel.clone(),
                        Name::new(voxel_name(voxel.position)),
                    ));
                    cached.style = voxel.style;
                }
            }
        }
    }
    for id in missing_styles {
        warn!("viewport content references missing style '{id}'");
    }

    let desired_grid = desired_grid_cells(&content, *mode, &desired_voxels);
    let stale_grid: Vec<_> = scene
        .grid
        .keys()
        .filter(|cell| !desired_grid.contains(*cell))
        .copied()
        .collect();
    for cell in stale_grid {
        if let Some(entity) = scene.grid.remove(&cell) {
            commands.entity(entity).despawn();
        }
    }
    for cell in desired_grid {
        if scene.grid.contains_key(&cell) {
            continue;
        }
        let entity = commands
            .spawn((
                Mesh3d(assets.hex_mesh.clone()),
                MeshMaterial3d(assets.guide_material.clone()),
                guide_transform(cell),
                NotShadowCaster,
                ViewportPickCell {
                    cell,
                    source: ViewportPickSource::Grid,
                },
                ViewportManaged,
                Name::new(format!("Grid ({}, {}, {})", cell.q, cell.r, cell.level)),
            ))
            .id();
        scene.grid.insert(cell, entity);
    }
}

fn plan_voxel_reconciliation(
    existing: &BTreeMap<LocalVoxelCoord, CachedVoxelEntity>,
    desired: &BTreeMap<LocalVoxelCoord, VoxelStyleId>,
    rebind_styles: &BTreeSet<VoxelStyleId>,
) -> Vec<VoxelSceneAction> {
    let mut actions = Vec::new();
    for position in existing.keys() {
        if !desired.contains_key(position) {
            actions.push(VoxelSceneAction::Despawn(*position));
        }
    }
    for (position, style) in desired {
        match existing.get(position) {
            None => actions.push(VoxelSceneAction::Spawn(RenderedVoxel {
                position: *position,
                style: style.clone(),
            })),
            Some(cached) if cached.style != *style || rebind_styles.contains(style) => {
                actions.push(VoxelSceneAction::Restyle(RenderedVoxel {
                    position: *position,
                    style: style.clone(),
                }));
            }
            Some(_) => {}
        }
    }
    actions
}

fn desired_grid_cells(
    content: &ViewportContent,
    mode: ViewportMode,
    occupied: &BTreeMap<LocalVoxelCoord, VoxelStyleId>,
) -> BTreeSet<LocalVoxelCoord> {
    if !content.show_grid {
        return BTreeSet::new();
    }
    let radius = match mode {
        ViewportMode::StylePreview => 0,
        ViewportMode::Object => content.grid_radius.min(MAX_OBJECT_RADIUS),
    };
    axial_cells(radius)
        .into_iter()
        .map(|axial| LocalVoxelCoord::new(axial.q, axial.r, content.active_level))
        .filter(|cell| !occupied.contains_key(cell))
        .collect()
}

fn material_handle(
    assets: &ViewportRenderAssets,
    style: &VoxelStyleId,
    missing_styles: &mut BTreeSet<VoxelStyleId>,
) -> Handle<StandardMaterial> {
    assets
        .content_materials
        .get(style)
        .map(|cached| cached.handle.clone())
        .unwrap_or_else(|| {
            missing_styles.insert(style.clone());
            assets.missing_material.clone()
        })
}

fn voxel_name(position: LocalVoxelCoord) -> String {
    format!("Voxel ({}, {}, {})", position.q, position.r, position.level)
}

fn material_for(style: ViewportStyle, rig: ViewportPreviewRig) -> StandardMaterial {
    let opacity = if style.opacity.is_finite() {
        style.opacity.clamp(f32::EPSILON, 1.0)
    } else {
        1.0
    };
    let alpha_mode = match style.surface_mode {
        VoxelSurfaceMode::Opaque => AlphaMode::Opaque,
        VoxelSurfaceMode::Cutout => AlphaMode::AlphaToCoverage,
        VoxelSurfaceMode::Translucent => AlphaMode::Blend,
        VoxelSurfaceMode::Additive => AlphaMode::Add,
    };
    let alpha = if style.surface_mode == VoxelSurfaceMode::Opaque {
        1.0
    } else {
        opacity
    };
    let emission = style.emission.map_or(LinearRgba::BLACK, |emission| {
        let strength = if emission.strength.is_finite() {
            emission.strength.max(0.0)
        } else {
            0.0
        };
        let linear = Color::srgb(
            emission.color.red(),
            emission.color.green(),
            emission.color.blue(),
        )
        .to_linear();
        LinearRgba::new(
            linear.red * strength,
            linear.green * strength,
            linear.blue * strength,
            1.0,
        )
    });

    StandardMaterial {
        base_color: Color::srgba(
            style.color.red(),
            style.color.green(),
            style.color.blue(),
            alpha,
        ),
        emissive: emission,
        alpha_mode,
        perceptual_roughness: 0.82,
        metallic: 0.0,
        unlit: rig == ViewportPreviewRig::Unlit,
        ..default()
    }
}

fn apply_preview_rig(
    rig: Res<ViewportPreviewRig>,
    mut ambient: ResMut<GlobalAmbientLight>,
    mut clear: ResMut<ClearColor>,
    mut lights: Query<(&mut DirectionalLight, &mut Visibility), With<ViewportKeyLight>>,
) {
    if !rig.is_changed() {
        return;
    }
    let Ok((mut light, mut visibility)) = lights.single_mut() else {
        return;
    };

    match *rig {
        ViewportPreviewRig::Neutral => {
            ambient.color = Color::srgb(0.72, 0.77, 0.84);
            ambient.brightness = 260.0;
            light.color = Color::srgb(1.0, 0.96, 0.90);
            light.illuminance = 8_500.0;
            light.shadow_maps_enabled = true;
            *visibility = Visibility::Inherited;
            clear.0 = Color::srgb(0.035, 0.04, 0.045);
        }
        ViewportPreviewRig::Dark => {
            ambient.color = Color::srgb(0.20, 0.28, 0.46);
            ambient.brightness = 12.0;
            light.color = Color::srgb(0.34, 0.46, 0.72);
            light.illuminance = 280.0;
            light.shadow_maps_enabled = false;
            *visibility = Visibility::Inherited;
            clear.0 = Color::srgb(0.004, 0.006, 0.012);
        }
        ViewportPreviewRig::Unlit => {
            ambient.brightness = 0.0;
            light.illuminance = 0.0;
            light.shadow_maps_enabled = false;
            *visibility = Visibility::Hidden;
            clear.0 = Color::srgb(0.075, 0.075, 0.075);
        }
    }
}

fn handle_camera_requests(
    mut snaps: MessageReader<CameraSnapRequest>,
    mut frames: MessageReader<FrameViewportRequest>,
    content: Res<ViewportContent>,
    mode: Res<ViewportMode>,
    mut cameras: Query<(&mut ViewportCamera, &mut Transform)>,
) {
    let mut requested_snap = None;
    for request in snaps.read() {
        requested_snap = Some(request.0);
    }
    let frame_requested = frames.read().next().is_some();
    if requested_snap.is_none() && !frame_requested {
        return;
    }

    let Ok((mut camera, mut transform)) = cameras.single_mut() else {
        return;
    };
    if frame_requested {
        let (focus, radius) = frame_for(&content, *mode);
        camera.focus = focus;
        camera.radius = radius;
    }
    if let Some(snap) = requested_snap {
        camera.snap(snap);
    }
    *transform = camera.transform();
}

fn control_camera(
    windows: Query<&Window, With<PrimaryWindow>>,
    input_enabled: Res<ViewportInputEnabled>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut cursor_events: MessageReader<CursorMoved>,
    mut wheel_events: MessageReader<MouseWheel>,
    mut drag: Local<CameraDrag>,
    mut cameras: Query<(&mut ViewportCamera, &mut Transform)>,
) {
    let cursor_positions: Vec<Vec2> = cursor_events.read().map(|event| event.position).collect();
    let scroll: f32 = wheel_events
        .read()
        .map(|event| match event.unit {
            MouseScrollUnit::Line => event.y,
            MouseScrollUnit::Pixel => event.y / 48.0,
        })
        .sum();
    let Ok(window) = windows.single() else {
        drag.mode = None;
        drag.last_cursor = None;
        return;
    };
    if !input_enabled.0 {
        drag.mode = None;
        drag.last_cursor = None;
        return;
    }

    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    let space = keys.pressed(KeyCode::Space);
    let wanted_mode = if buttons.pressed(MouseButton::Middle)
        || (space && buttons.pressed(MouseButton::Left))
        || (shift && buttons.pressed(MouseButton::Right))
    {
        Some(CameraDragMode::Pan)
    } else if buttons.pressed(MouseButton::Right) {
        Some(CameraDragMode::Orbit)
    } else {
        None
    };
    if drag.mode != wanted_mode {
        drag.mode = wanted_mode;
        drag.last_cursor = window.cursor_position();
    }

    let mut pointer_delta = Vec2::ZERO;
    if wanted_mode.is_some() {
        for position in cursor_positions {
            if let Some(previous) = drag.last_cursor {
                pointer_delta += position - previous;
            }
            drag.last_cursor = Some(position);
        }
    } else {
        drag.last_cursor = None;
    }

    if pointer_delta == Vec2::ZERO && scroll.abs() <= f32::EPSILON {
        return;
    }
    let Ok((mut camera, mut transform)) = cameras.single_mut() else {
        return;
    };

    match wanted_mode {
        Some(CameraDragMode::Orbit) => {
            let size = Vec2::new(window.width().max(1.0), window.height().max(1.0));
            camera.yaw -= pointer_delta.x / size.x * std::f32::consts::TAU * ORBIT_SENSITIVITY;
            camera.pitch += pointer_delta.y / size.y * std::f32::consts::PI * ORBIT_SENSITIVITY;
            camera.pitch = camera.pitch.clamp(MIN_CAMERA_PITCH, MAX_CAMERA_PITCH);
        }
        Some(CameraDragMode::Pan) => {
            let rotation = camera.transform().rotation;
            let right = rotation * Vec3::X;
            let up = rotation * Vec3::Y;
            let world_per_pixel = camera.radius / window.height().max(1.0) * PAN_SCREEN_SCALE;
            camera.focus += (-right * pointer_delta.x + up * pointer_delta.y) * world_per_pixel;
        }
        _ => {}
    }

    if scroll.abs() > f32::EPSILON {
        camera.radius *= (-scroll * ZOOM_SENSITIVITY).exp();
        camera.radius = camera.radius.clamp(MIN_CAMERA_RADIUS, MAX_CAMERA_RADIUS);
    }
    *transform = camera.transform();
}

fn clear_hover_when_input_disabled(
    input: Res<ViewportInputEnabled>,
    mut hovered: ResMut<HoveredFaceTarget>,
    mut changes: MessageWriter<ViewportHoverChanged>,
) {
    if input.0 || hovered.0.is_none() {
        return;
    }
    hovered.0 = None;
    changes.write(ViewportHoverChanged(None));
}

fn on_pointer_over(
    event: On<Pointer<Over>>,
    input: Res<ViewportInputEnabled>,
    cells: Query<&ViewportPickCell>,
    mut hovered: ResMut<HoveredFaceTarget>,
    mut changes: MessageWriter<ViewportHoverChanged>,
) {
    if !input.0 {
        return;
    }
    let Ok(cell) = cells.get(event.event_target()) else {
        return;
    };
    set_hover(
        target_from_hit(
            event.event_target(),
            *cell,
            event.event.hit.position,
            event.event.hit.normal,
        ),
        &mut hovered,
        &mut changes,
    );
}

fn on_pointer_move(
    event: On<Pointer<Move>>,
    input: Res<ViewportInputEnabled>,
    cells: Query<&ViewportPickCell>,
    mut hovered: ResMut<HoveredFaceTarget>,
    mut changes: MessageWriter<ViewportHoverChanged>,
) {
    if !input.0 {
        return;
    }
    let Ok(cell) = cells.get(event.event_target()) else {
        return;
    };
    set_hover(
        target_from_hit(
            event.event_target(),
            *cell,
            event.event.hit.position,
            event.event.hit.normal,
        ),
        &mut hovered,
        &mut changes,
    );
}

fn on_pointer_out(
    event: On<Pointer<Out>>,
    cells: Query<&ViewportPickCell>,
    mut hovered: ResMut<HoveredFaceTarget>,
    mut changes: MessageWriter<ViewportHoverChanged>,
) {
    if cells.get(event.event_target()).is_err()
        || hovered
            .0
            .is_none_or(|target| target.entity != event.event_target())
    {
        return;
    }
    hovered.0 = None;
    changes.write(ViewportHoverChanged(None));
}

fn on_pointer_click(
    event: On<Pointer<Click>>,
    input: Res<ViewportInputEnabled>,
    cells: Query<&ViewportPickCell>,
    mut clicks: MessageWriter<ViewportFaceClicked>,
) {
    if !input.0 {
        return;
    }
    let Ok(cell) = cells.get(event.event_target()) else {
        return;
    };
    let Some(target) = target_from_hit(
        event.event_target(),
        *cell,
        event.event.hit.position,
        event.event.hit.normal,
    ) else {
        return;
    };
    clicks.write(ViewportFaceClicked {
        button: event.event.button,
        target,
    });
}

fn set_hover(
    target: Option<ViewportFaceTarget>,
    hovered: &mut HoveredFaceTarget,
    changes: &mut MessageWriter<ViewportHoverChanged>,
) {
    if hovered.0 == target {
        return;
    }
    hovered.0 = target;
    changes.write(ViewportHoverChanged(target));
}

fn target_from_hit(
    entity: Entity,
    cell: ViewportPickCell,
    world_position: Option<Vec3>,
    normal: Option<Vec3>,
) -> Option<ViewportFaceTarget> {
    let world_position = world_position?;
    let fallback_normal = match cell.source {
        ViewportPickSource::Voxel => Vec3::ZERO,
        ViewportPickSource::Grid => Vec3::Y,
    };
    let normal = normal.unwrap_or(fallback_normal).normalize_or_zero();
    Some(ViewportFaceTarget {
        entity,
        cell: cell.cell,
        source: cell.source,
        world_position,
        normal,
    })
}

fn draw_guides(
    content: Res<ViewportContent>,
    mode: Res<ViewportMode>,
    hovered: Res<HoveredFaceTarget>,
    mut gizmos: Gizmos,
) {
    if content.show_grid {
        let radius = match *mode {
            ViewportMode::StylePreview => 0,
            ViewportMode::Object => content.grid_radius.min(MAX_OBJECT_RADIUS),
        };
        let y = grid_plane_y(content.active_level) + GRID_LINE_LIFT;
        for axial in axial_cells(radius) {
            draw_hex_ring(
                &mut gizmos,
                axial_world_center(axial, y),
                Color::srgba(0.72, 0.76, 0.80, 0.26),
            );
        }
    }

    for position in &content.selected_cells {
        if cell_is_visible(&content, *position) {
            draw_hex_ring(
                &mut gizmos,
                overlay_ring_center(*position, 0.026),
                Color::srgb(1.0, 0.82, 0.24),
            );
        }
    }
    if content.show_semantic_overlay {
        for (position, part) in &content.semantic_parts {
            if cell_is_visible(&content, *position) {
                draw_hex_ring(
                    &mut gizmos,
                    overlay_ring_center(*position, 0.042),
                    semantic_color(*part),
                );
            }
        }
    }
    if content.show_canopy_overlay {
        for position in &content.canopy_cells {
            if cell_is_visible(&content, *position) {
                draw_hex_ring(
                    &mut gizmos,
                    overlay_ring_center(*position, 0.060),
                    Color::srgb(0.20, 0.86, 0.92),
                );
            }
        }
    }
    if content.show_blocker_overlay {
        for column in &content.blocker_columns {
            draw_hex_ring(
                &mut gizmos,
                axial_world_center(*column, GRID_LINE_LIFT + 0.020),
                Color::srgb(0.96, 0.24, 0.22),
            );
        }
    }

    let Some(target) = hovered.0 else {
        return;
    };
    let base_y = level_floor(target.cell.level);
    let center = voxel_world_center(target.cell);
    match target.source {
        ViewportPickSource::Voxel => {
            draw_hex_ring(
                &mut gizmos,
                Vec3::new(center.x, base_y + GRID_LINE_LIFT, center.z),
                Color::srgb(0.98, 0.78, 0.30),
            );
            draw_hex_ring(
                &mut gizmos,
                Vec3::new(
                    center.x,
                    base_y + DEFAULT_LEVEL_HEIGHT + GRID_LINE_LIFT,
                    center.z,
                ),
                Color::srgb(0.98, 0.78, 0.30),
            );
        }
        ViewportPickSource::Grid => {
            draw_hex_ring(
                &mut gizmos,
                Vec3::new(center.x, base_y + GRID_LINE_LIFT, center.z),
                Color::srgb(0.98, 0.78, 0.30),
            );
        }
    }
    if target.normal != Vec3::ZERO {
        gizmos.line(
            target.world_position,
            target.world_position + target.normal * 0.55,
            Color::srgb(0.98, 0.48, 0.24),
        );
    }
}

fn cell_is_visible(content: &ViewportContent, position: LocalVoxelCoord) -> bool {
    !content.isolate_active_level || position.level == content.active_level
}

fn overlay_ring_center(position: LocalVoxelCoord, lift: f32) -> Vec3 {
    let center = voxel_world_center(position);
    Vec3::new(
        center.x,
        level_floor(position.level) + DEFAULT_LEVEL_HEIGHT + lift,
        center.z,
    )
}

const fn semantic_color(part: ObjectPart) -> Color {
    match part {
        ObjectPart::Plant(PlantPart::Root) => Color::srgb(0.78, 0.30, 0.20),
        ObjectPart::Plant(PlantPart::Trunk) => Color::srgb(0.94, 0.53, 0.20),
        ObjectPart::Plant(PlantPart::Branch) => Color::srgb(0.92, 0.73, 0.24),
        ObjectPart::Plant(PlantPart::Foliage) => Color::srgb(0.30, 0.84, 0.38),
        ObjectPart::Plant(PlantPart::Accent) => Color::srgb(0.96, 0.36, 0.68),
        ObjectPart::Effect(EffectPart::Core) => Color::srgb(0.26, 0.90, 0.96),
        ObjectPart::Effect(EffectPart::Trail) => Color::srgb(0.26, 0.52, 0.98),
        ObjectPart::Effect(EffectPart::Accent) => Color::srgb(0.72, 0.40, 0.96),
        ObjectPart::Prop(PropPart::Structure) => Color::srgb(0.74, 0.78, 0.82),
        ObjectPart::Prop(PropPart::Detail) => Color::srgb(0.96, 0.72, 0.28),
    }
}

fn draw_hex_ring(gizmos: &mut Gizmos, center: Vec3, color: Color) {
    let inner = 0.5 * HEX_SMALL_DIAMETER;
    let points = [
        center + Vec3::new(0.0, 0.0, HEX_CIRCUMRADIUS),
        center + Vec3::new(inner, 0.0, 0.5 * HEX_CIRCUMRADIUS),
        center + Vec3::new(inner, 0.0, -0.5 * HEX_CIRCUMRADIUS),
        center + Vec3::new(0.0, 0.0, -HEX_CIRCUMRADIUS),
        center + Vec3::new(-inner, 0.0, -0.5 * HEX_CIRCUMRADIUS),
        center + Vec3::new(-inner, 0.0, 0.5 * HEX_CIRCUMRADIUS),
    ];
    for (from, to) in points
        .iter()
        .copied()
        .zip(points.iter().copied().cycle().skip(1))
        .take(points.len())
    {
        gizmos.line(from, to, color);
    }
}

fn axial_cells(radius: u8) -> Vec<LocalAxialCoord> {
    let radius = i32::from(radius);
    let mut cells = Vec::new();
    for q in -radius..=radius {
        let min_r = (-radius).max(-q - radius);
        let max_r = radius.min(-q + radius);
        for r in min_r..=max_r {
            cells.push(LocalAxialCoord::new(q, r));
        }
    }
    cells
}

#[expect(
    clippy::cast_precision_loss,
    reason = "validated object-local coordinates are bounded to a tiny authoring canvas"
)]
fn axial_world_center(position: LocalAxialCoord, y: f32) -> Vec3 {
    let q = position.q as f32;
    let r = position.r as f32;
    Vec3::new(
        3.0f32.sqrt() * (q + 0.5 * r) * HEX_CIRCUMRADIUS,
        y,
        1.5 * r * HEX_CIRCUMRADIUS,
    )
}

#[expect(
    clippy::cast_precision_loss,
    reason = "validated object levels are bounded to a 64-level authoring canvas"
)]
fn level_floor(level: i32) -> f32 {
    level as f32 * DEFAULT_LEVEL_HEIGHT
}

/// Returns the world-space center of one object-local voxel.
#[must_use]
pub fn voxel_world_center(position: LocalVoxelCoord) -> Vec3 {
    axial_world_center(
        position.axial(),
        level_floor(position.level) + 0.5 * DEFAULT_LEVEL_HEIGHT,
    )
}

/// Returns the render transform for one object-local voxel.
#[must_use]
pub fn voxel_transform(position: LocalVoxelCoord) -> Transform {
    Transform {
        translation: voxel_world_center(position),
        scale: Vec3::new(1.0, DEFAULT_LEVEL_HEIGHT, 1.0),
        ..default()
    }
}

fn guide_transform(position: LocalVoxelCoord) -> Transform {
    Transform {
        translation: axial_world_center(
            position.axial(),
            level_floor(position.level) - 0.5 * GUIDE_THICKNESS,
        ),
        scale: Vec3::new(1.0, GUIDE_THICKNESS, 1.0),
        ..default()
    }
}

fn grid_plane_y(level: i32) -> f32 {
    level_floor(level)
}

fn snap_angles(snap: CameraSnap) -> (f32, f32) {
    match snap {
        CameraSnap::Perspective => (std::f32::consts::FRAC_PI_4, 0.62),
        CameraSnap::Top => (0.0, std::f32::consts::FRAC_PI_2),
        CameraSnap::Front => (0.0, 0.0),
        CameraSnap::Side => (std::f32::consts::FRAC_PI_2, 0.0),
    }
}

fn camera_transform(camera: ViewportCamera) -> Transform {
    let horizontal = camera.pitch.cos();
    let offset = Vec3::new(
        camera.yaw.sin() * horizontal,
        camera.pitch.sin(),
        camera.yaw.cos() * horizontal,
    ) * camera.radius;
    let eye = camera.focus + offset;
    let up = if horizontal.abs() <= 1e-4 {
        Vec3::Z
    } else {
        Vec3::Y
    };
    Transform::from_translation(eye).looking_at(camera.focus, up)
}

fn frame_for(content: &ViewportContent, mode: ViewportMode) -> (Vec3, f32) {
    if mode == ViewportMode::StylePreview {
        return (Vec3::new(0.0, 0.5 * DEFAULT_LEVEL_HEIGHT, 0.0), 5.5);
    }

    frame_object_positions(
        content
            .voxels
            .iter()
            .filter(|voxel| {
                !content.isolate_active_level || voxel.position.level == content.active_level
            })
            .map(|voxel| voxel.position),
    )
    .unwrap_or_else(|| {
        let radius = f32::from(content.grid_radius.min(MAX_OBJECT_RADIUS));
        (
            Vec3::new(0.0, grid_plane_y(content.active_level), 0.0),
            (radius * HEX_SMALL_DIAMETER + 4.0).clamp(5.5, MAX_CAMERA_RADIUS),
        )
    })
}

/// Returns stable focus and radius values for occupied object-local cells.
///
/// `None` represents an empty iterator. Review capture reuses this calculation so
/// its fixed views frame geometry exactly as the interactive viewport does.
pub(crate) fn frame_object_positions(
    positions: impl IntoIterator<Item = LocalVoxelCoord>,
) -> Option<(Vec3, f32)> {
    let mut bounds: Option<(Vec3, Vec3)> = None;
    for position in positions {
        let center = voxel_world_center(position);
        let half = Vec3::new(
            HEX_CIRCUMRADIUS,
            0.5 * DEFAULT_LEVEL_HEIGHT,
            HEX_CIRCUMRADIUS,
        );
        let low = center - half;
        let high = center + half;
        bounds = Some(match bounds {
            Some((minimum, maximum)) => (minimum.min(low), maximum.max(high)),
            None => (low, high),
        });
    }

    bounds.map(|(minimum, maximum)| {
        let focus = 0.5 * (minimum + maximum);
        let half_extent = 0.5 * (maximum - minimum);
        let radius = (2.7 * half_extent.length() + 2.0).clamp(MIN_CAMERA_RADIUS, MAX_CAMERA_RADIUS);
        (focus, radius)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_vec3_close(actual: Vec3, expected: Vec3) {
        assert!(
            actual.distance(expected) < 1e-5,
            "expected {expected:?}, got {actual:?}"
        );
    }

    #[test]
    fn local_hex_voxels_match_game_mesh_spacing_and_level_height() {
        assert_vec3_close(
            voxel_world_center(LocalVoxelCoord::new(0, 0, 0)),
            Vec3::new(0.0, 0.2, 0.0),
        );
        assert_vec3_close(
            voxel_world_center(LocalVoxelCoord::new(1, 0, 0)),
            Vec3::new(3.0f32.sqrt(), 0.2, 0.0),
        );
        assert_vec3_close(
            voxel_world_center(LocalVoxelCoord::new(0, 1, 2)),
            Vec3::new(0.5 * 3.0f32.sqrt(), 1.0, 1.5),
        );
    }

    #[test]
    fn radius_six_guide_contains_the_expected_127_hexes() {
        assert_eq!(axial_cells(6).len(), 127);
        let unique: BTreeSet<_> = axial_cells(6).into_iter().collect();
        assert_eq!(unique.len(), 127);
    }

    #[test]
    fn active_level_grid_omits_occupied_pick_cells() {
        let Ok(style) = VoxelStyleId::new("test/opaque") else {
            unreachable!("fixture style id should be valid")
        };
        let occupied_cell = LocalVoxelCoord::new(0, 0, 3);
        let occupied = BTreeMap::from([(occupied_cell, style)]);
        let content = ViewportContent {
            grid_radius: 1,
            active_level: 3,
            show_grid: true,
            ..default()
        };

        let grid = desired_grid_cells(&content, ViewportMode::Object, &occupied);

        assert_eq!(grid.len(), 6);
        assert!(!grid.contains(&occupied_cell));
    }

    #[test]
    fn max_size_one_cell_edit_plans_one_entity_update() {
        let Ok(original_style) = VoxelStyleId::new("test/original") else {
            unreachable!("fixture style id should be valid")
        };
        let Ok(repainted_style) = VoxelStyleId::new("test/repainted") else {
            unreachable!("fixture style id should be valid")
        };
        let mut desired = BTreeMap::new();
        'levels: for level in 0..64 {
            for axial in axial_cells(MAX_OBJECT_RADIUS) {
                desired.insert(
                    LocalVoxelCoord::new(axial.q, axial.r, level),
                    original_style.clone(),
                );
                if desired.len() == hex_assets::MAX_OBJECT_VOXELS {
                    break 'levels;
                }
            }
        }
        assert_eq!(desired.len(), hex_assets::MAX_OBJECT_VOXELS);

        let existing: BTreeMap<_, _> = desired
            .iter()
            .map(|(position, style)| {
                (
                    *position,
                    CachedVoxelEntity {
                        entity: Entity::PLACEHOLDER,
                        style: style.clone(),
                    },
                )
            })
            .collect();
        assert!(plan_voxel_reconciliation(&existing, &desired, &BTreeSet::new()).is_empty());

        let Some(position) = desired.keys().nth(desired.len() / 2).copied() else {
            unreachable!("max-size fixture should not be empty")
        };
        desired.insert(position, repainted_style.clone());
        assert_eq!(
            plan_voxel_reconciliation(&existing, &desired, &BTreeSet::new()),
            [VoxelSceneAction::Restyle(RenderedVoxel {
                position,
                style: repainted_style,
            })]
        );
    }

    #[test]
    fn camera_snaps_preserve_focus_and_radius_and_look_at_the_focus() {
        let original = ViewportCamera {
            focus: Vec3::new(2.0, 3.0, -4.0),
            radius: 17.0,
            ..default()
        };
        for snap in [
            CameraSnap::Perspective,
            CameraSnap::Top,
            CameraSnap::Front,
            CameraSnap::Side,
        ] {
            let mut camera = original;
            camera.snap(snap);
            let transform = camera.transform();
            assert_vec3_close(camera.focus, original.focus);
            assert!((camera.radius - original.radius).abs() < f32::EPSILON);
            assert!((transform.translation.distance(camera.focus) - camera.radius).abs() < 1e-4);
            let toward_focus = (camera.focus - transform.translation).normalize();
            assert!(transform.forward().as_vec3().dot(toward_focus) > 0.9999);
        }
    }

    #[test]
    fn top_front_and_side_snap_to_exact_axes() {
        let mut camera = ViewportCamera {
            focus: Vec3::ZERO,
            radius: 10.0,
            ..default()
        };

        camera.snap(CameraSnap::Top);
        assert_vec3_close(camera.transform().translation, Vec3::Y * 10.0);
        camera.snap(CameraSnap::Front);
        assert_vec3_close(camera.transform().translation, Vec3::Z * 10.0);
        camera.snap(CameraSnap::Side);
        assert_vec3_close(camera.transform().translation, Vec3::X * 10.0);
    }
}
