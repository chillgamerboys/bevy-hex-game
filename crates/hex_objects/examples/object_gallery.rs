//! Runtime acceptance gallery for tracked voxel objects.

use std::path::{Path, PathBuf};

use bevy::camera::RenderTarget;
use bevy::light::GlobalAmbientLight;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use hex_assets::{HexObjectRotation, ObjectAssetId, ObjectInstance, RuntimeArtCatalog};
use hex_core::{HexCoord, TilePos};
use hex_objects::ObjectRenderChunk;

const OBJECT_ID: &str = "plant/small-broadleaf";
const LEVEL_HEIGHT: f32 = 0.4;
const RIG_ENVIRONMENT: &str = "HEX_OBJECT_GALLERY_RIG";
const CAPTURE_ENVIRONMENT: &str = "HEX_OBJECT_GALLERY_CAPTURE";
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
struct GalleryCapture {
    path: PathBuf,
    target: Handle<Image>,
    settled_frames: u8,
    requested: bool,
}

fn main() -> AppExit {
    let rig = GalleryRig::from_environment();
    App::new()
        .insert_resource(rig)
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
        .add_systems(Update, (spawn_rotations, request_capture).chain())
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
        Mesh3d(meshes.add(Plane3d::default().mesh().size(24.0, 24.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.19, 0.21, 0.20),
            perceptual_roughness: 0.96,
            ..default()
        })),
        Name::new("Object Gallery Ground"),
    ));
    commands.spawn((
        Camera3d::default(),
        gallery_camera_transform(),
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
            gallery_camera_transform(),
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

fn gallery_camera_transform() -> Transform {
    Transform::from_xyz(0.0, 10.5, 17.0).looking_at(Vec3::new(0.0, 1.1, 0.0), Vec3::Y)
}

fn spawn_rotations(
    mut commands: Commands,
    catalog: Option<Res<RuntimeArtCatalog>>,
    spawned: Option<Res<GallerySpawned>>,
    mut exit: MessageWriter<AppExit>,
) {
    if spawned.is_some() {
        return;
    }
    let Some(catalog) = catalog else {
        return;
    };
    let Ok(object_id) = ObjectAssetId::new(OBJECT_ID) else {
        error!("gallery object id '{OBJECT_ID}' violates the stable-id contract");
        exit.write(AppExit::error());
        return;
    };
    if catalog.object(&object_id).is_none() {
        error!("runtime art catalog does not contain gallery object '{OBJECT_ID}'");
        exit.write(AppExit::error());
        return;
    }

    let coordinates = [
        HexCoord::from_axial(0, -4),
        HexCoord::from_axial(4, -4),
        HexCoord::from_axial(4, 0),
        HexCoord::from_axial(0, 4),
        HexCoord::from_axial(-4, 4),
        HexCoord::from_axial(-4, 0),
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
            Name::new(format!("Small Broadleaf / rotation {steps}")),
        ));
    }
    commands.insert_resource(GallerySpawned);
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
