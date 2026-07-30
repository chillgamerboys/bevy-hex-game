//! Runtime acceptance gallery for tracked voxel objects.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use bevy::camera::RenderTarget;
use bevy::light::GlobalAmbientLight;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use hex_assets::{
    ConnectivityPolicy, EffectPart, HexObjectRotation, LocalVoxelCoord, ObjectAssetId,
    ObjectBlueprint, ObjectBounds, ObjectCatalogFile, ObjectCategory, ObjectInstance, ObjectPart,
    ObjectPlacement, PaletteSwatch, RuntimeArtCatalog, SrgbColor, SwatchId, VoxelEmission,
    VoxelStyle, VoxelStyleId, VoxelSurfaceMode, OBJECT_BLUEPRINT_SCHEMA_VERSION,
};
use hex_core::{HexCoord, TilePos};
use hex_objects::ObjectRenderChunk;

const DEFAULT_OBJECT_ID: &str = "plant/small-broadleaf";
const LEVEL_HEIGHT: f32 = 0.4;
const OBJECT_ENVIRONMENT: &str = "HEX_OBJECT_GALLERY_OBJECT";
const RIG_ENVIRONMENT: &str = "HEX_OBJECT_GALLERY_RIG";
const CAPTURE_ENVIRONMENT: &str = "HEX_OBJECT_GALLERY_CAPTURE";
const FIXTURES_ENVIRONMENT: &str = "HEX_OBJECT_GALLERY_MATERIAL_FIXTURES";
const MATERIAL_FIXTURE_ID: &str = "effect/runtime-material-fixture";
const CAPTURE_WIDTH: u32 = 1600;
const CAPTURE_HEIGHT: u32 = 900;

#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
enum GalleryRig {
    Neutral,
    Dark,
}

impl GalleryRig {
    fn from_environment() -> Self {
        match std::env::var(RIG_ENVIRONMENT) {
            Ok(value) if value.eq_ignore_ascii_case("dark") => Self::Dark,
            Ok(value) if value.eq_ignore_ascii_case("neutral") => Self::Neutral,
            Ok(value) => {
                warn!(
                    "{RIG_ENVIRONMENT}='{value}' is invalid; expected 'neutral' or 'dark', \
                     using neutral"
                );
                Self::Neutral
            }
            Err(_) => Self::Neutral,
        }
    }
}

#[derive(Resource)]
struct GallerySpawned;

#[derive(Resource)]
struct GalleryOptions {
    material_fixtures: bool,
    object_id: String,
}

#[derive(Component)]
struct GalleryCamera;

#[derive(Resource)]
struct GalleryCapture {
    path: PathBuf,
    target: Handle<Image>,
    settled_frames: u8,
    requested: bool,
}

fn main() -> AppExit {
    let rig = GalleryRig::from_environment();
    let object_id =
        std::env::var(OBJECT_ENVIRONMENT).unwrap_or_else(|_| DEFAULT_OBJECT_ID.to_owned());
    App::new()
        .insert_resource(rig)
        .insert_resource(GalleryOptions {
            material_fixtures: std::env::var_os(FIXTURES_ENVIRONMENT).is_some(),
            object_id,
        })
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: format!("Object Gallery / {rig:?}"),
                resolution: (1600, 900).into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins((hex_assets::plugin, hex_objects::plugin))
        .add_systems(Startup, setup_gallery)
        .add_systems(
            Update,
            (install_material_fixture, spawn_rotations, request_capture).chain(),
        )
        .run()
}

fn setup_gallery(
    mut commands: Commands,
    rig: Res<GalleryRig>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let (ambient_color, ambient_brightness, light_color, illuminance, clear_color) = match *rig {
        GalleryRig::Neutral => (
            Color::srgb(0.72, 0.77, 0.84),
            260.0,
            Color::srgb(1.0, 0.96, 0.90),
            8_500.0,
            Color::srgb(0.035, 0.04, 0.045),
        ),
        GalleryRig::Dark => (
            Color::srgb(0.20, 0.28, 0.46),
            12.0,
            Color::srgb(0.34, 0.46, 0.72),
            280.0,
            Color::srgb(0.004, 0.006, 0.012),
        ),
    };
    commands.insert_resource(GlobalAmbientLight {
        color: ambient_color,
        brightness: ambient_brightness,
        ..default()
    });
    commands.insert_resource(ClearColor(clear_color));
    commands.spawn((
        DirectionalLight {
            color: light_color,
            illuminance,
            shadow_maps_enabled: *rig == GalleryRig::Neutral,
            ..default()
        },
        Transform::from_xyz(9.0, 14.0, 7.0).looking_at(Vec3::ZERO, Vec3::Y),
        Name::new("Object Gallery Key Light"),
    ));
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(48.0, 48.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.19, 0.21, 0.20),
            perceptual_roughness: 0.96,
            ..default()
        })),
        Name::new("Object Gallery Ground"),
    ));
    commands.spawn((
        Camera3d::default(),
        gallery_camera_transform(4, 0, 5),
        GalleryCamera,
        Name::new("Object Gallery Camera"),
    ));

    if let Some(path) = std::env::var_os(CAPTURE_ENVIRONMENT).map(PathBuf::from) {
        let target = images.add(Image::new_target_texture(
            CAPTURE_WIDTH,
            CAPTURE_HEIGHT,
            TextureFormat::Rgba8UnormSrgb,
            None,
        ));
        commands.spawn((
            Camera3d::default(),
            Camera {
                order: -1,
                ..default()
            },
            RenderTarget::Image(target.clone().into()),
            gallery_camera_transform(4, 0, 5),
            GalleryCamera,
            Name::new("Object Gallery Capture Camera"),
        ));
        commands.insert_resource(GalleryCapture {
            path,
            target,
            settled_frames: 0,
            requested: false,
        });
    }
}

fn gallery_camera_transform(ring_radius: u8, object_radius: u8, object_height: u8) -> Transform {
    let object_height = f32::from(object_height) * LEVEL_HEIGHT;
    let focus = Vec3::new(0.0, object_height * 0.5, 0.0);
    let horizontal_extent = f32::from(ring_radius) + f32::from(object_radius);
    let distance = (horizontal_extent * 2.5 + object_height * 1.4).max(22.0);
    Transform::from_xyz(0.0, focus.y + distance * 0.55, distance).looking_at(focus, Vec3::Y)
}

fn install_material_fixture(
    options: Res<GalleryOptions>,
    catalog: Option<ResMut<RuntimeArtCatalog>>,
    mut exit: MessageWriter<AppExit>,
) {
    if !options.material_fixtures {
        return;
    }
    let Some(mut catalog) = catalog else {
        return;
    };
    let Ok(fixture_id) = ObjectAssetId::new(MATERIAL_FIXTURE_ID) else {
        error!("material fixture id violates the stable-id contract");
        exit.write(AppExit::error());
        return;
    };
    if catalog.object(&fixture_id).is_some() {
        return;
    }
    match catalog_with_material_fixture(&catalog, fixture_id) {
        Ok(augmented) => *catalog = augmented,
        Err(error) => {
            error!("cannot build transient runtime material fixture: {error}");
            exit.write(AppExit::error());
        }
    }
}

fn spawn_rotations(
    mut commands: Commands,
    catalog: Option<Res<RuntimeArtCatalog>>,
    spawned: Option<Res<GallerySpawned>>,
    options: Res<GalleryOptions>,
    mut cameras: Query<&mut Transform, With<GalleryCamera>>,
    mut exit: MessageWriter<AppExit>,
) {
    if spawned.is_some() {
        return;
    }
    let Some(catalog) = catalog else {
        return;
    };
    let Ok(object_id) = ObjectAssetId::new(options.object_id.clone()) else {
        error!(
            "gallery object id '{}' violates the stable-id contract",
            options.object_id
        );
        exit.write(AppExit::error());
        return;
    };
    let Some(blueprint) = catalog.object(&object_id) else {
        error!(
            "runtime art catalog does not contain gallery object '{}'",
            object_id.as_str()
        );
        exit.write(AppExit::error());
        return;
    };

    let ring_radius = 4_u8.saturating_add(blueprint.bounds.radius.saturating_mul(2));
    let axial_radius = i32::from(ring_radius);
    let coordinates = [
        HexCoord::from_axial(0, -axial_radius),
        HexCoord::from_axial(axial_radius, -axial_radius),
        HexCoord::from_axial(axial_radius, 0),
        HexCoord::from_axial(0, axial_radius),
        HexCoord::from_axial(-axial_radius, axial_radius),
        HexCoord::from_axial(-axial_radius, 0),
    ];
    for (steps, coord) in (0_u8..6).zip(coordinates) {
        let Ok(rotation) = HexObjectRotation::new(steps) else {
            error!("gallery rotation {steps} violates the six-way rotation contract");
            exit.write(AppExit::error());
            return;
        };
        let Ok(instance) = ObjectInstance::new(
            object_id.clone(),
            TilePos::new(coord, 0),
            LEVEL_HEIGHT,
            rotation,
        ) else {
            error!("gallery object instance {steps} is invalid");
            exit.write(AppExit::error());
            return;
        };
        commands.spawn((
            instance,
            Name::new(format!("{} / rotation {steps}", blueprint.display_name)),
        ));
    }
    let camera_transform = gallery_camera_transform(
        ring_radius,
        blueprint.bounds.radius,
        blueprint.bounds.height,
    );
    for mut transform in &mut cameras {
        *transform = camera_transform;
    }
    if options.material_fixtures {
        let Ok(fixture_id) = ObjectAssetId::new(MATERIAL_FIXTURE_ID) else {
            error!("material fixture id violates the stable-id contract");
            exit.write(AppExit::error());
            return;
        };
        if catalog.object(&fixture_id).is_none() {
            return;
        }
        let Ok(instance) = ObjectInstance::new(
            fixture_id,
            TilePos::new(HexCoord::ORIGIN, 0),
            LEVEL_HEIGHT,
            HexObjectRotation::default(),
        ) else {
            error!("runtime material fixture instance is invalid");
            exit.write(AppExit::error());
            return;
        };
        commands.spawn((instance, Name::new("Runtime Material Fixture")));
    }
    commands.insert_resource(GallerySpawned);
}

fn catalog_with_material_fixture(
    catalog: &RuntimeArtCatalog,
    fixture_id: ObjectAssetId,
) -> Result<RuntimeArtCatalog, String> {
    let mut palette = catalog.palette().clone();
    let mut styles = catalog.styles().clone();
    for (suffix, display_name, color, surface_mode, opacity, emission) in [
        (
            "opaque",
            "Review Opaque",
            (0.72, 0.28, 0.12),
            VoxelSurfaceMode::Opaque,
            1.0,
            false,
        ),
        (
            "cutout",
            "Review Cutout",
            (0.78, 0.72, 0.16),
            VoxelSurfaceMode::Cutout,
            0.65,
            false,
        ),
        (
            "translucent",
            "Review Translucent",
            (0.12, 0.62, 0.82),
            VoxelSurfaceMode::Translucent,
            0.42,
            false,
        ),
        (
            "additive",
            "Review Additive",
            (0.82, 0.18, 0.62),
            VoxelSurfaceMode::Additive,
            0.75,
            true,
        ),
    ] {
        let swatch_id =
            SwatchId::new(format!("review/{suffix}")).map_err(|error| error.to_string())?;
        let color = SrgbColor::new(color.0, color.1, color.2).map_err(|error| error.to_string())?;
        let swatch = PaletteSwatch::new(
            display_name,
            color,
            BTreeSet::from(["review".to_owned(), "runtime-fixture".to_owned()]),
        )
        .map_err(|error| error.to_string())?;
        palette
            .insert(swatch_id.clone(), swatch)
            .map_err(|error| error.to_string())?;

        let style_id =
            VoxelStyleId::new(format!("review/{suffix}")).map_err(|error| error.to_string())?;
        let emission = emission
            .then(|| VoxelEmission::new(swatch_id.clone(), 3.0))
            .transpose()
            .map_err(|error| error.to_string())?;
        let style = VoxelStyle::new(display_name, swatch_id, surface_mode, opacity, emission)
            .map_err(|error| error.to_string())?;
        styles
            .insert(style_id, style)
            .map_err(|error| error.to_string())?;
    }

    let fixture = ObjectBlueprint {
        schema_version: OBJECT_BLUEPRINT_SCHEMA_VERSION,
        id: fixture_id.clone(),
        display_name: "Runtime Material Fixture".to_owned(),
        category: ObjectCategory::Effect,
        bounds: ObjectBounds {
            radius: 2,
            min_level: 0,
            height: 2,
        },
        connectivity: ConnectivityPolicy::Free,
        origin: LocalVoxelCoord::new(0, 0, 0),
        placements: vec![
            fixture_placement(0, 0, 0, "review/opaque", EffectPart::Core)?,
            fixture_placement(-1, 0, 0, "review/cutout", EffectPart::Trail)?,
            fixture_placement(1, -1, 0, "review/translucent", EffectPart::Accent)?,
            fixture_placement(2, -1, 0, "review/translucent", EffectPart::Accent)?,
            fixture_placement(0, 1, 1, "review/additive", EffectPart::Accent)?,
        ],
        blocker_footprint: Vec::new(),
        canopy_occluders: Vec::new(),
    };
    fixture.validate(&styles)?;

    let mut objects = catalog.objects().clone();
    objects.insert(fixture_id.clone(), fixture);
    let manifest =
        ObjectCatalogFile::new(catalog.manifest().ids().iter().cloned().chain([fixture_id]))
            .map_err(|error| error.to_string())?;
    RuntimeArtCatalog::from_sources(&palette, &styles, &manifest, objects)
        .map_err(|error| error.to_string())
}

fn fixture_placement(
    q: i32,
    r: i32,
    level: i32,
    style: &str,
    part: EffectPart,
) -> Result<ObjectPlacement, String> {
    Ok(ObjectPlacement {
        position: LocalVoxelCoord::new(q, r, level),
        style: VoxelStyleId::new(style).map_err(|error| error.to_string())?,
        part: ObjectPart::Effect(part),
    })
}

fn request_capture(
    mut commands: Commands,
    spawned: Option<Res<GallerySpawned>>,
    chunks: Query<(), With<ObjectRenderChunk>>,
    capture: Option<ResMut<GalleryCapture>>,
) {
    let Some(mut capture) = capture else {
        return;
    };
    if capture.requested || spawned.is_none() || chunks.is_empty() {
        return;
    }
    capture.settled_frames = capture.settled_frames.saturating_add(1);
    if capture.settled_frames < 8 {
        return;
    }

    let output = capture.path.clone();
    commands
        .spawn(Screenshot::image(capture.target.clone()))
        .observe(
            move |captured: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| {
                match save_capture(&captured.image, &output) {
                    Ok(()) => {
                        info!("object gallery capture completed: {}", output.display());
                        exit.write(AppExit::Success);
                    }
                    Err(error) => {
                        error!(
                            "object gallery capture failed for {}: {error}",
                            output.display()
                        );
                        exit.write(AppExit::error());
                    }
                }
            },
        );
    capture.requested = true;
}

fn save_capture(image: &Image, path: &Path) -> Result<(), String> {
    if image.width() != CAPTURE_WIDTH || image.height() != CAPTURE_HEIGHT {
        return Err(format!(
            "renderer returned {}x{}; expected {CAPTURE_WIDTH}x{CAPTURE_HEIGHT}",
            image.width(),
            image.height()
        ));
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create '{}': {error}", parent.display()))?;
    }
    let dynamic = image
        .clone()
        .try_into_dynamic()
        .map_err(|error| format!("cannot convert renderer output: {error}"))?;
    dynamic
        .to_rgb8()
        .save(path)
        .map_err(|error| format!("cannot write PNG: {error}"))
}
