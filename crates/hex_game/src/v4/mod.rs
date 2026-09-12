//! V4 world-package composition root.
//!
//! This explicit explorer exercises authoring products, residency, existing terrain
//! meshes and exact gameplay motion. It does not install frozen V3 scenario/save
//! plugins or claim to implement encounter scheduling. The map provider, motion
//! consumer and disposable presentation remain separately owned.

mod art;
mod atlas;
mod gpu_completion;
mod knowledge;
mod object_edit;
mod prefetch;
mod queue;
mod state;
mod walk;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use bevy::camera::RenderTarget;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::picking::mesh_picking::MeshPickingPlugin;
use bevy::picking::pointer::PointerButton;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::winit::WinitPlugin;
use hex_core::{TerrainRenderBatch, TraversalProfile};
use hex_map::v4::{PresentationLimits, RenderOrigin, ResidentRun, TerrainPresenter};
use hex_units::v4::{plan_route, ContinuousStep, RouteResult, SearchLimits};
use hex_world_contracts::{
    ChunkId, QueryResult, ResidencyRequest, VoxelEdit, VoxelPosition, WorldEditTransaction,
    WorldHex, WorldManifest, WorldQuery,
};
use hex_world_runtime::{FileChunkSource, IoLimits, RuntimeConfig, WorldRuntime};
use serde::Serialize;

use knowledge::WorldKnowledge;
use queue::MeshQueue;

const LEVEL_HEIGHT: f32 = 0.35;

#[derive(Resource, Clone)]
struct Options {
    package: PathBuf,
    save: Option<PathBuf>,
    capture: Option<PathBuf>,
    focus: Option<VoxelPosition>,
    radius: u32,
    parties: usize,
    frames: u64,
    settle_frames: u64,
    azimuth: f32,
    view: String,
    walk: Option<PathBuf>,
}

impl Options {
    fn parse() -> Result<Self, String> {
        Self::parse_arguments(std::env::args().skip(1))
    }

    fn parse_arguments(arguments: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut args = arguments.into_iter();
        let mut values = BTreeMap::new();
        while let Some(flag) = args.next() {
            if ![
                "--world",
                "--save",
                "--capture",
                "--focus",
                "--radius",
                "--parties",
                "--frames",
                "--settle-frames",
                "--azimuth",
                "--view",
                "--walk",
            ]
            .contains(&flag.as_str())
            {
                return Err(format!(
                    "unknown option {flag}; use --world PACKAGE [--save DIRECTORY] [--capture FRAME.png] [--focus q,r,level] [--view orbit|top|first|atlas]"
                ));
            }
            let value = args
                .next()
                .ok_or_else(|| format!("missing value for {flag}"))?;
            if values.insert(flag.clone(), value).is_some() {
                return Err(format!("duplicate option {flag}"));
            }
        }
        let package = values
            .get("--world")
            .map(PathBuf::from)
            .ok_or("--world PACKAGE is required")?;
        let save = values.get("--save").map(PathBuf::from);
        let capture = values.get("--capture").map(PathBuf::from);
        if capture
            .as_ref()
            .is_some_and(|path| path.exists() || path.with_extension("json").exists())
        {
            return Err("capture or receipt already exists; use a fresh output path".into());
        }
        let walk = values.get("--walk").map(PathBuf::from);
        if walk.is_some() && capture.is_none() {
            return Err("automated walks require an explicit windowless --capture output".into());
        }
        let focus = values
            .get("--focus")
            .map(|value| -> Result<VoxelPosition, String> {
                let coordinates = value
                    .split(',')
                    .map(str::parse::<i64>)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| error.to_string())?;
                let [q, r, level] = coordinates.as_slice() else {
                    return Err("focus must be q,r,level".into());
                };
                Ok(VoxelPosition {
                    column: WorldHex::new(*q, *r),
                    level: i32::try_from(*level).map_err(|error| error.to_string())?,
                })
            })
            .transpose()?;
        let radius = values
            .get("--radius")
            .map_or(Ok(56), |value| value.parse::<u32>())
            .map_err(|error| error.to_string())?;
        let parties = values
            .get("--parties")
            .map_or(Ok(2), |value| value.parse::<usize>())
            .map_err(|error| error.to_string())?;
        let frames = values
            .get("--frames")
            .map_or(Ok(3600), |value| value.parse::<u64>())
            .map_err(|error| error.to_string())?;
        let settle_frames = values
            .get("--settle-frames")
            .map_or(Ok(120), |value| value.parse::<u64>())
            .map_err(|error| error.to_string())?;
        let azimuth = values
            .get("--azimuth")
            .map_or(Ok(35.0), |value| value.parse::<f32>())
            .map_err(|error| error.to_string())?;
        let view = values
            .get("--view")
            .cloned()
            .unwrap_or_else(|| "orbit".into());
        if !(16..=224).contains(&radius)
            || !(1..=7).contains(&parties)
            || (radius > 96 && (capture.is_none() || parties != 1))
            || !(1..=100_000).contains(&frames)
            || !(12..=10_000).contains(&settle_frames)
            || frames <= settle_frames
            || !azimuth.is_finite()
            || !["orbit", "top", "first", "atlas"].contains(&view.as_str())
        {
            return Err("invalid radius, party count, frames, settle frames, azimuth, or view; settle frames must be 12..10000 below the frame deadline; radius 97..224 requires a windowless capture with one party".into());
        }
        Ok(Self {
            package,
            save,
            capture,
            focus,
            radius,
            parties,
            frames,
            settle_frames,
            azimuth,
            view,
            walk,
        })
    }
}

/// Region entries are useful initial placements, never party ownership identities.
/// Extra parties use explicitly declared usable anchors, not arbitrary scenic peaks.
fn party_spawn_points(
    manifest: &WorldManifest,
    count: usize,
) -> Result<Vec<VoxelPosition>, String> {
    if !(1..=7).contains(&count) {
        return Err("party count must be one to seven".into());
    }
    let mut points = Vec::new();
    for region in manifest.regions.iter().take(count) {
        let entry = manifest
            .features
            .iter()
            .find(|feature| {
                feature.region_id == region.id && feature.kind == "entry" && feature.asset.is_none()
            })
            .ok_or_else(|| format!("region {} has no declared entry anchor", region.id))?;
        if points
            .iter()
            .any(|point: &VoxelPosition| point.column == entry.anchor.column)
        {
            return Err("declared region entries overlap another party's initial column".into());
        }
        points.push(entry.anchor);
    }
    while points.len() < count {
        let mut candidates = Vec::new();
        for feature in &manifest.features {
            if feature.asset.is_some()
                || !["entry", "transit", "gameplay-anchor"].contains(&feature.kind.as_str())
                || points
                    .iter()
                    .any(|point| point.column == feature.anchor.column)
            {
                continue;
            }
            let separation = points
                .iter()
                .map(|point| point.column.checked_distance(feature.anchor.column))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?
                .into_iter()
                .min()
                .unwrap_or(0);
            candidates.push((separation, feature));
        }
        candidates.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.id.cmp(&b.1.id)));
        let Some((_, feature)) = candidates.first() else {
            return Err(format!(
                "requested {count} parties but the world has only {} distinct declared usable spawn anchors; author more transit or gameplay anchors",
                points.len()
            ));
        };
        points.push(feature.anchor);
    }
    Ok(points)
}

fn has_pending_activity(session: &Session) -> bool {
    session.edit_requested.is_some()
        || session.object_edit_requested.is_some()
        || session.object_removal.is_some()
        || session.cancel_object_edit_requested
        || session.save_requested
        || session.step_requested
        || session.actors.iter().any(|actor| {
            actor.motion.is_some()
                || actor.requested.is_some()
                || (!actor.turn_steps && !actor.route.is_empty())
        })
}

#[derive(Resource)]
struct ResidentWorld(WorldRuntime);

struct ExplorerActor {
    id: String,
    column: WorldHex,
    standing: Option<VoxelPosition>,
    requested_level: Option<i32>,
    entity: Option<Entity>,
    route: VecDeque<VoxelPosition>,
    motion: Option<ContinuousStep>,
    pinned: bool,
    turn_steps: bool,
    requested: Option<MoveRequest>,
    planning_pinned: bool,
}

struct MoveRequest {
    goal: VoxelPosition,
    waiting: BTreeSet<ChunkId>,
}

struct EditRequest {
    position: VoxelPosition,
    observed_revision: u64,
}

#[derive(Resource)]
struct Session {
    actors: Vec<ExplorerActor>,
    selected: usize,
    edit_requested: Option<EditRequest>,
    object_edit_requested: Option<object_edit::ObjectSelection>,
    object_removal: Option<object_edit::ObjectRemoval>,
    cancel_object_edit_requested: bool,
    successful_object_edits: u64,
    save_requested: bool,
    successful_saves: u64,
    gameplay_revision: u64,
    interests: Vec<ResidencyRequest>,
    desired: BTreeSet<ChunkId>,
    status: String,
    error: Option<String>,
    frames: u64,
    settled_frames: u64,
    step_requested: bool,
    yaw: f32,
    pitch: f32,
    distance: f32,
    target: Option<Handle<Image>>,
    capture_requested: bool,
    started: Instant,
    frame_milliseconds: Vec<f64>,
    rebase_milliseconds: Vec<f64>,
}

#[derive(Component)]
struct ExplorerCamera;
#[derive(Component)]
struct ExplorerHud;

#[derive(Resource)]
struct ExplorerArt {
    hex: Handle<Mesh>,
    pieces: [Handle<Mesh>; 2],
    material: Handle<StandardMaterial>,
}

/// Parse explicit explorer options and run the V4 world independently of V3.
pub fn launch() -> AppExit {
    match Options::parse().and_then(build_app) {
        Ok(mut app) => app.run(),
        Err(error) => {
            let _written = writeln!(std::io::stderr().lock(), "hex_v4: {error}");
            AppExit::error()
        }
    }
}

fn build_app(options: Options) -> Result<App, String> {
    let source = FileChunkSource::open_workspace(&options.package, IoLimits::default())
        .map_err(|error| error.to_string())?;
    let mut runtime = WorldRuntime::new(
        Arc::new(source),
        RuntimeConfig {
            max_resident_chunks: 768,
            max_in_flight_jobs: 2,
            max_publications_per_pump: 2,
            ..default()
        },
    )
    .map_err(|error| error.to_string())?;
    if let Some(save) = &options.save {
        if save.join("current.ron").exists() {
            runtime
                .restore_save(save, IoLimits::default())
                .map_err(|error| error.to_string())?;
        }
    }
    let mut actors = party_spawn_points(runtime.manifest(), options.parties)?
        .into_iter()
        .enumerate()
        .map(|(index, position)| ExplorerActor {
            id: format!("party/{index}"),
            column: position.column,
            standing: None,
            requested_level: Some(position.level),
            entity: None,
            route: VecDeque::new(),
            motion: None,
            pinned: false,
            turn_steps: false,
            requested: None,
            planning_pinned: false,
        })
        .collect::<Vec<_>>();
    let (selected, gameplay_revision) = if options.save.is_some() {
        state::restore(&runtime, &mut actors)?
    } else {
        (0, 0)
    };
    let first = actors.get_mut(selected).ok_or("world has no regions")?;
    if let Some(focus) = options.focus {
        first.column = focus.column;
        first.requested_level = Some(focus.level);
    }
    let origin = RenderOrigin {
        column: first
            .column
            .chunk()
            .origin()
            .map_err(|error| error.to_string())?,
        level: first.requested_level.unwrap_or(0),
    };
    let presenter = TerrainPresenter::with_limits(
        runtime.manifest(),
        origin,
        LEVEL_HEIGHT,
        PresentationLimits {
            max_resident_chunks: 768,
            ..default()
        },
    )
    .map_err(|error| error.to_string())?;
    let knowledge = WorldKnowledge::open(&runtime, options.save.as_deref())?;
    let mut app = App::new();
    let plugins = DefaultPlugins.set(WindowPlugin {
        exit_condition: if options.capture.is_some() {
            bevy::window::ExitCondition::DontExit
        } else {
            bevy::window::ExitCondition::OnAllClosed
        },
        primary_window: if options.capture.is_some() {
            None
        } else {
            Some(Window {
                title: "Hex V4 - World Explorer".into(),
                resolution: (1440, 900).into(),
                ..default()
            })
        },
        ..default()
    });
    if options.capture.is_some() {
        app.add_plugins(plugins.disable::<WinitPlugin>());
        app.add_plugins(bevy::app::ScheduleRunnerPlugin::run_loop(
            std::time::Duration::ZERO,
        ));
        gpu_completion::install(&mut app)?;
    } else {
        app.add_plugins(plugins);
    }
    app.add_plugins(MeshPickingPlugin);
    hex_objects::v4::transparency_plugin(&mut app);
    app.insert_resource(ClearColor(Color::srgb(0.085, 0.12, 0.15)));
    app.insert_resource(GlobalAmbientLight {
        color: Color::WHITE,
        brightness: 450.0,
        ..default()
    });
    app.insert_resource(Session {
        actors,
        selected,
        edit_requested: None,
        object_edit_requested: None,
        object_removal: None,
        cancel_object_edit_requested: false,
        successful_object_edits: 0,
        save_requested: false,
        successful_saves: 0,
        gameplay_revision,
        interests: Vec::new(),
        desired: BTreeSet::new(),
        status: "Select terrain to plan a route".into(),
        error: None,
        frames: 0,
        settled_frames: 0,
        step_requested: false,
        yaw: options.azimuth.to_radians(),
        pitch: 0.7,
        distance: 50.0,
        target: None,
        capture_requested: false,
        started: Instant::now(),
        frame_milliseconds: Vec::new(),
        rebase_milliseconds: Vec::new(),
    });
    if let Some(path) = &options.walk {
        app.insert_resource(walk::WalkHarness::load(path)?);
    }
    app.insert_resource(options);
    app.insert_resource(ResidentWorld(runtime));
    app.insert_resource(knowledge);
    app.insert_resource(presenter);
    app.init_resource::<MeshQueue>();
    app.init_resource::<atlas::AtlasState>();
    app.add_systems(Startup, (setup, atlas::setup).chain());
    app.add_systems(
        Update,
        (input, tick, update_view, atlas::update, capture).chain(),
    );
    app.add_observer(on_click);
    Ok(app)
}

fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    asset_server: Res<AssetServer>,
    options: Res<Options>,
    mut session: ResMut<Session>,
) {
    // The same shipped player primitives as GameAssets, without installing V3's
    // settings, scenario, roster or complete-world bootstrap.
    let piece = |mesh| {
        asset_server.load(
            bevy::gltf::GltfAssetLabel::Primitive { mesh, primitive: 0 }
                .from_asset("meshes/pieces.glb"),
        )
    };
    commands.insert_resource(ExplorerArt {
        hex: asset_server.load(
            bevy::gltf::GltfAssetLabel::Primitive {
                mesh: 0,
                primitive: 0,
            }
            .from_asset("meshes/hex.glb"),
        ),
        pieces: [piece(0), piece(1)],
        material: materials.add(Color::srgb(0.93, 0.68, 0.22)),
    });
    let mut camera = commands.spawn((
        Camera3d::default(),
        IsDefaultUiCamera,
        Transform::from_xyz(30.0, 50.0, 45.0).looking_at(Vec3::ZERO, Vec3::Y),
        ExplorerCamera,
    ));
    if options.capture.is_some() {
        let target = images.add(Image::new_target_texture(
            1920,
            1080,
            TextureFormat::Rgba8UnormSrgb,
            None,
        ));
        camera.insert(RenderTarget::Image(target.clone().into()));
        session.target = Some(target);
    }
    commands.spawn((
        DirectionalLight {
            illuminance: 12_000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.9, -0.5, 0.0)),
    ));
    commands.spawn((
        Text::new("Loading V4 world..."),
        TextFont {
            font_size: FontSize::Px(17.0),
            ..default()
        },
        TextColor(Color::WHITE),
        Node {
            position_type: PositionType::Absolute,
            left: px(18),
            top: px(16),
            padding: UiRect::all(px(12)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.02, 0.035, 0.05, 0.88)),
        ExplorerHud,
    ));
}

fn input(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut session: ResMut<Session>,
    atlas: Res<atlas::AtlasState>,
) {
    if keys.just_pressed(KeyCode::Tab) && session.actors.len() > 1 {
        session.selected = (session.selected + 1) % session.actors.len();
    }
    if keys.just_pressed(KeyCode::KeyT) {
        let selected = session.selected;
        if let Some(actor) = session.actors.get_mut(selected) {
            actor.turn_steps = !actor.turn_steps;
        }
    }
    if keys.just_pressed(KeyCode::Space) {
        session.step_requested = true;
    }
    if keys.just_pressed(KeyCode::KeyS) {
        session.save_requested = true;
    }
    if keys.just_pressed(KeyCode::Escape) {
        session.cancel_object_edit_requested = true;
    }
    if mouse.pressed(MouseButton::Right) && !atlas.visible {
        session.yaw -= motion.delta.x * 0.005;
        session.pitch = (session.pitch + motion.delta.y * 0.004).clamp(0.1, 1.45);
    }
    if !atlas.visible {
        session.distance = (session.distance * (-scroll.delta.y * 0.1).exp()).clamp(8.0, 180.0);
    }
}

fn on_click(
    event: On<Pointer<Click>>,
    batches: Query<&TerrainRenderBatch>,
    runs: Query<&ResidentRun>,
    parts: Query<&hex_objects::v4::ResidentObjectPart>,
    queue: Res<MeshQueue>,
    presenter: Res<TerrainPresenter>,
    keys: Res<ButtonInput<KeyCode>>,
    mut session: ResMut<Session>,
    atlas: Res<atlas::AtlasState>,
    runtime: Res<ResidentWorld>,
) {
    if atlas.visible || event.event.button != PointerButton::Primary {
        return;
    }
    let Some(hit) = event.event.hit.position else {
        return;
    };
    let normal = event.event.hit.normal;
    let selected = if let Ok(batch) = batches.get(event.event_target()) {
        batch
            .resolve_hit(hit, normal)
            .and_then(|entity| runs.get(entity).ok())
            .and_then(|run| {
                presenter
                    .receipts()
                    .find(|receipt| receipt.coordinate == run.position.column.chunk())
                    .map(|receipt| {
                        (
                            run.position,
                            clicked_voxel(run, hit, normal, presenter.origin()),
                            receipt.revision,
                        )
                    })
            })
    } else if let Ok(part) = parts.get(event.event_target()) {
        queue
            .art
            .as_ref()
            .and_then(|art| art.object(&part.id))
            .and_then(|object| {
                object_hit(object, part.clip, hit, normal, presenter.origin())
                    .map(|(surface, voxel)| (surface, voxel, part.revision))
            })
    } else {
        None
    };
    let Some((surface, voxel, revision)) = selected else {
        return;
    };
    if keys.pressed(KeyCode::KeyD) {
        if session.object_removal.is_some() || session.object_edit_requested.is_some() {
            session.status = "An object edit is pending; Escape cancels it".into();
            return;
        }
        let selections = match object_edit::selections_at(&runtime.0, voxel, revision) {
            Ok(selections) => selections,
            Err(error) => {
                session.status = format!("Edit refused: {error}");
                return;
            }
        };
        let preferred = parts
            .get(event.event_target())
            .ok()
            .map(|part| part.id.as_str());
        let selection = preferred
            .and_then(|id| {
                selections
                    .iter()
                    .find(|selection| selection.object_id == id)
            })
            .or_else(|| selections.first());
        if let Some(selection) = selection {
            session.object_edit_requested = Some(selection.clone());
            return;
        }
        session.edit_requested = Some(EditRequest {
            position: voxel,
            observed_revision: revision,
        });
    } else {
        let selected = session.selected;
        if let Some(actor) = session.actors.get_mut(selected) {
            actor.requested = Some(MoveRequest {
                goal: surface,
                waiting: BTreeSet::new(),
            });
        }
    }
}

/// Recover exact identity from a bounded local hit and the authority's intervals.
#[expect(
    clippy::cast_possible_truncation,
    reason = "finite local height is bounded before conversion"
)]
fn object_hit(
    object: &hex_world_contracts::ObjectInstance,
    clip: Option<ChunkId>,
    hit: Vec3,
    normal: Option<Vec3>,
    origin: RenderOrigin,
) -> Option<(VoxelPosition, VoxelPosition)> {
    let inside = hit - normal.unwrap_or(Vec3::Y) * (LEVEL_HEIGHT * 0.001);
    let level = f64::from(inside.y) / f64::from(LEVEL_HEIGHT);
    if !inside.is_finite() || inside.x.abs().max(inside.z.abs()) > 8192.0 || level.abs() > 4096.0 {
        return None;
    }
    let position = origin
        .global_voxel(hex_core::TilePos::new(
            hex_core::HexCoord::from_world(inside),
            level.floor() as i32,
        ))
        .ok()?;
    if clip.is_some_and(|clip| clip != position.column.chunk()) {
        return None;
    }
    let column = object
        .occupancy
        .binary_search_by_key(&position.column, |column| column.position)
        .ok()
        .and_then(|index| object.occupancy.get(index))?;
    let run = column
        .runs
        .iter()
        .find(|run| run.bottom <= position.level && position.level < run.top)?;
    Some((
        VoxelPosition {
            column: position.column,
            level: run.top.checked_sub(1)?,
        },
        position,
    ))
}

/// Pick a voxel just inside the hit face, preserving the observed interval.
#[expect(
    clippy::cast_possible_truncation,
    reason = "finite local hit height is clamped to the exact i32 interval"
)]
fn clicked_voxel(
    run: &ResidentRun,
    hit: Vec3,
    normal: Option<Vec3>,
    origin: RenderOrigin,
) -> VoxelPosition {
    let inside = hit - normal.unwrap_or(Vec3::Y) * (LEVEL_HEIGHT * 0.001);
    let local = f64::from(inside.y) / f64::from(LEVEL_HEIGHT);
    let level = if local.is_finite() {
        (local.floor() + f64::from(origin.level))
            .clamp(f64::from(run.bottom), f64::from(run.top - 1)) as i32
    } else {
        run.position.level
    };
    VoxelPosition {
        column: run.position.column,
        level,
    }
}

fn tick(world: &mut World) {
    let elapsed = world.resource::<Time<Real>>().delta_secs_f64();
    let options = world.resource::<Options>().clone();
    world.resource_scope(|world, mut session: Mut<Session>| {
        session.frames += 1;
        if session.frame_milliseconds.len() < 100_000 {
            session.frame_milliseconds.push(elapsed * 1000.0);
        }
        if session.error.is_some() {
            return;
        }
        world.resource_scope(|world, mut resident: Mut<ResidentWorld>| {
            world.resource_scope(|world, mut presenter: Mut<TerrainPresenter>| {
                world.resource_scope(|world, mut queue: Mut<MeshQueue>| {
                    if let Err(error) = advance(
                        world,
                        &options,
                        &mut session,
                        &mut resident.0,
                        &mut presenter,
                        &mut queue,
                        elapsed,
                    ) {
                        session.object_edit_requested = None;
                        session.cancel_object_edit_requested = false;
                        let cleanup = session
                            .object_removal
                            .take()
                            .map(|mut removal| removal.cancel(&mut resident.0))
                            .transpose();
                        session.error = Some(match cleanup {
                            Ok(_) => error,
                            Err(cleanup) => {
                                format!("{error}; object pin cleanup failed: {cleanup}")
                            }
                        });
                    }
                });
            });
        });
    });
}

fn advance(
    world: &mut World,
    options: &Options,
    session: &mut Session,
    runtime: &mut WorldRuntime,
    presenter: &mut TerrainPresenter,
    queue: &mut MeshQueue,
    elapsed: f64,
) -> Result<(), String> {
    let counts = runtime.counts();
    let mut interests = session
        .actors
        .iter()
        .map(|actor| ResidencyRequest {
            id: actor.id.clone(),
            center: actor.column,
            radius: options.radius + 16,
            retention_radius: options.radius + 32,
            priority: 10,
        })
        .collect::<Vec<_>>();
    for actor in &session.actors {
        if let Some(request) = prefetch::ahead(
            &actor.id,
            actor
                .motion
                .into_iter()
                .map(|motion| motion.to)
                .chain(actor.route.iter().copied()),
            runtime.load_timing().ema_milliseconds,
            counts.queued_chunks.saturating_add(counts.in_flight_jobs),
            2,
        ) {
            interests.push(request);
        }
    }
    if interests != session.interests {
        runtime
            .set_interests(interests.clone())
            .map_err(|error| error.to_string())?;
        session.interests = interests;
    }
    let updates = runtime.pump();
    if let Some(failure) = updates.failures.first() {
        return Err(format!("chunk {:?}: {}", failure.coordinate, failure.error));
    }
    // Runtime removal does not delete presentation immediately. The mesh queue
    // restores adjacent boundary faces and retires the root atomically.
    let art_ready = {
        let art = world.resource::<ExplorerArt>();
        let server = world.resource::<AssetServer>();
        for handle in art.pieces.iter().chain(std::iter::once(&art.hex)) {
            if let Some(bevy::asset::LoadState::Failed(error)) = server.get_load_state(handle.id())
            {
                return Err(format!("stock player asset failed: {error}"));
            }
        }
        art.pieces
            .iter()
            .all(|handle| world.resource::<Assets<Mesh>>().contains(handle.id()))
    };
    if queue.art.is_none() {
        let handle = &world.resource::<ExplorerArt>().hex;
        if let Some(mesh) = world.resource::<Assets<Mesh>>().get(handle) {
            queue.art = Some(art::StockArt::load(mesh.clone())?);
        }
    }
    if let Some(art) = &mut queue.art {
        art.refresh(runtime)?;
    }
    for actor in &mut session.actors {
        if actor.standing.is_none() {
            if let QueryResult::Ready(surfaces) = runtime.surfaces(actor.column) {
                let surface = surfaces
                    .into_iter()
                    .rfind(|surface| {
                        surface.headroom.is_none_or(|clearance| clearance >= 2)
                            && actor
                                .requested_level
                                .is_none_or(|level| surface.position.level == level)
                    })
                    .ok_or_else(|| {
                        format!(
                            "{} has no valid initial support at {:?}",
                            actor.id, actor.column
                        )
                    })?;
                actor.standing = Some(surface.position);
            }
        }
        if actor.standing.is_some() && actor.entity.is_none() && art_ready {
            actor.entity = Some(spawn_actor(world));
        }
    }
    if world.contains_resource::<walk::WalkHarness>() {
        world.resource_scope(|_world, mut harness: Mut<walk::WalkHarness>| {
            harness.tick(session, runtime)
        })?;
    }
    move_actors(session, runtime, elapsed)?;
    edit_and_save(options, session, runtime, false)?;
    let knowledge_idle = world.resource_scope(|_world, mut knowledge: Mut<WorldKnowledge>| {
        knowledge.tick(session, runtime)?;
        Ok::<bool, String>(knowledge.idle())
    })?;
    if knowledge_idle {
        edit_and_save(options, session, runtime, true)?;
    }
    let selected = session
        .actors
        .get(session.selected)
        .ok_or("selected party is missing")?;
    let center = selected.column;
    let desired = runtime
        .resident_chunks()
        .filter(|product| {
            product.coordinate.origin().is_ok_and(|origin| {
                origin
                    .checked_distance(center)
                    .is_ok_and(|distance| distance <= u64::from(options.radius + 24))
            })
        })
        .map(|product| product.coordinate)
        .collect::<BTreeSet<_>>();
    let level = selected
        .standing
        .map_or(selected.requested_level.unwrap_or(0), |position| {
            position.level
        });
    let needs_rebase = presenter
        .origin()
        .column
        .checked_distance(center)
        .map_err(|error| error.to_string())?
        > 128
        || (i64::from(presenter.origin().level) - i64::from(level)).unsigned_abs() > 1024;
    let draining = needs_rebase
        && presenter
            .receipts()
            .any(|receipt| !desired.contains(&receipt.coordinate));
    if needs_rebase && !draining {
        // Retain current nearby roots through an atomic origin change. Canceled
        // worker outputs carry the old epoch and cannot replace these meshes.
        queue.cancel();
        let started = Instant::now();
        let new_origin = RenderOrigin {
            column: center.chunk().origin().map_err(|error| error.to_string())?,
            level,
        };
        if let Some(art) = &queue.art {
            art.validate_rebase(world, new_origin)?;
        }
        // Terrain prepares every replacement before changing anything. Its
        // publication touches only terrain-owned roots/assets, so the validated
        // art transforms remain admissible throughout this exclusive operation.
        presenter
            .rebase(world, new_origin)
            .map_err(|error| error.to_string())?;
        if let Some(art) = &mut queue.art {
            art.rebase(world, new_origin)?;
        }
        session
            .rebase_milliseconds
            .push(started.elapsed().as_secs_f64() * 1000.0);
        // Rebase preserves each published source and exact halo. The queue's
        // accepted source stamps therefore remain valid; its epoch rejects work
        // prepared at the previous render origin.
    }
    session.desired = desired;
    queue.tick(world, runtime, presenter, &session.desired, !draining)?;
    let origin = presenter.origin();
    for actor in &session.actors {
        if let (Some(entity), Some(position)) = (actor.entity, actor.standing) {
            let visible = !draining
                && position
                    .column
                    .checked_distance(center)
                    .is_ok_and(|distance| distance <= u64::from(options.radius));
            if let Ok(mut entity) = world.get_entity_mut(entity) {
                entity.insert(if visible {
                    Visibility::Visible
                } else {
                    Visibility::Hidden
                });
                if visible {
                    let at = actor_render_position(actor, origin)?;
                    entity.insert(Transform::from_translation(at));
                }
            }
        }
    }
    if queue.is_idle()
        && runtime.counts().in_flight_jobs == 0
        && runtime.counts().queued_chunks == 0
        && knowledge_idle
        && !has_pending_activity(session)
        && world
            .get_resource::<walk::WalkHarness>()
            .is_none_or(walk::WalkHarness::completed)
        && session
            .actors
            .iter()
            .all(|actor| actor.standing.is_some() && actor.entity.is_some())
    {
        session.settled_frames += 1;
    } else {
        session.settled_frames = 0;
    }
    if session.frames > options.frames && options.capture.is_some() {
        return Err(
            "windowless capture timed out before publication and screenshot completion".into(),
        );
    }
    Ok(())
}

fn spawn_actor(world: &mut World) -> Entity {
    let art = world.resource::<ExplorerArt>();
    let meshes = art.pieces.clone();
    let material = art.material.clone();
    // Shipped king model height is 9.08 source units; preserve its 1.88-level body.
    let scale = LEVEL_HEIGHT * 1.88 / 9.08;
    let transform = Transform {
        translation: Vec3::new(-scale, -scale, -10.0 * scale),
        scale: Vec3::splat(scale),
        ..default()
    };
    let children = meshes
        .into_iter()
        .map(|mesh| {
            world
                .spawn((
                    Mesh3d(mesh),
                    MeshMaterial3d(material.clone()),
                    transform,
                    bevy::picking::Pickable::IGNORE,
                ))
                .id()
        })
        .collect::<Vec<_>>();
    world
        .spawn((Transform::default(), Visibility::default()))
        .add_children(&children)
        .id()
}

fn move_actors(
    session: &mut Session,
    runtime: &mut WorldRuntime,
    elapsed: f64,
) -> Result<(), String> {
    for actor in &mut session.actors {
        if actor.motion.is_some() {
            continue;
        }
        let Some(request) = &actor.requested else {
            continue;
        };
        if request
            .waiting
            .iter()
            .any(|chunk| runtime.revision(*chunk).is_none())
        {
            continue;
        }
        let Some(start) = actor.standing else {
            continue;
        };
        let goal = request.goal;
        match plan_route(
            runtime,
            TraversalProfile::WALKER,
            start,
            goal,
            SearchLimits::default(),
        ) {
            RouteResult::Ready(route) => {
                runtime
                    .pin(
                        format!("motion/{}", actor.id),
                        route.revisions.keys().copied().collect(),
                    )
                    .map_err(|error| error.to_string())?;
                if !route.is_current(runtime) {
                    return Err("route changed while acquiring pins".into());
                }
                actor.pinned = true;
                actor.route = route.waypoints.into_iter().skip(1).collect();
                actor.requested = None;
                session.status = format!("{}: {} steps", actor.id, actor.route.len());
            }
            RouteResult::Pending(waiting) => {
                // Explicitly retain missing planning facts and retry only after
                // they arrive. The runtime applies the same bounded residency budget.
                let previous = actor.requested.as_ref().map(|request| &request.waiting);
                let mut required = previous.cloned().unwrap_or_default();
                required.extend(waiting);
                if required.len() > 256 {
                    actor.requested = None;
                    session.status = "Route exceeds the local planning residency budget".into();
                } else {
                    runtime
                        .pin(format!("planning/{}", actor.id), required.clone())
                        .map_err(|error| error.to_string())?;
                    actor.planning_pinned = true;
                    actor.requested = Some(MoveRequest {
                        goal,
                        waiting: required,
                    });
                    session.status = format!("{} is waiting for exact route terrain", actor.id);
                }
            }
            RouteResult::NoRoute => {
                actor.requested = None;
                session.status = "No legal route through the available terrain".into();
            }
            RouteResult::LimitReached => {
                actor.requested = None;
                session.status =
                    "Destination exceeds this local route query; choose a nearer waypoint".into();
            }
            RouteResult::Invalid(error) => return Err(error),
        }
        if actor.requested.is_none() && actor.planning_pinned {
            runtime
                .unpin(&format!("planning/{}", actor.id))
                .map_err(|error| error.to_string())?;
            actor.planning_pinned = false;
        }
    }
    for (index, actor) in session.actors.iter_mut().enumerate() {
        if actor.motion.is_none()
            && (!actor.turn_steps || (session.step_requested && index == session.selected))
        {
            if let (Some(from), Some(to)) = (actor.standing, actor.route.pop_front()) {
                actor.motion = Some(ContinuousStep {
                    from,
                    to,
                    fraction: 0.0,
                });
            }
        }
        if let Some(motion) = &mut actor.motion {
            match motion.advance(runtime, TraversalProfile::WALKER, elapsed.min(0.1), 4.0) {
                QueryResult::Ready(true) => {
                    if motion.fraction >= 1.0 {
                        actor.standing = Some(motion.to);
                        actor.column = motion.to.column;
                        actor.motion = None;
                    }
                }
                QueryResult::Unloaded(_) => {
                    session.status = "Movement is waiting for exact terrain".into()
                }
                _ => {
                    actor.motion = None;
                    actor.route.clear();
                    session.status = "Route stopped after its terrain changed".into();
                }
            }
        }
        if actor.pinned && actor.route.is_empty() && actor.motion.is_none() {
            runtime
                .unpin(&format!("motion/{}", actor.id))
                .map_err(|error| error.to_string())?;
            actor.pinned = false;
        }
    }
    session.step_requested = false;
    Ok(())
}

fn edit_and_save(
    options: &Options,
    session: &mut Session,
    runtime: &mut WorldRuntime,
    allow_save: bool,
) -> Result<(), String> {
    edit_objects(options, session, runtime)?;
    if let Some(request) = session.edit_requested.take() {
        let position = request.position;
        let protected = session.actors.iter().any(|actor| {
            actor.standing == Some(position)
                || actor
                    .motion
                    .is_some_and(|motion| motion.to == position || motion.from == position)
        });
        if protected {
            session.status = "An active character depends on this support".into();
        } else if runtime.revision(position.column.chunk()).is_some() {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|error| error.to_string())?
                .as_nanos();
            let transaction = WorldEditTransaction {
                id: format!("explorer/{stamp:x}/{}", session.frames),
                expected_revisions: BTreeMap::from([(
                    position.column.chunk(),
                    request.observed_revision,
                )]),
                edits: vec![VoxelEdit {
                    position,
                    material: None,
                }],
            };
            let result = if let Some(save) = &options.save {
                let prepared = state::prepare(runtime, session)?;
                let result = runtime.apply_transaction_durable_with_attachments(
                    &transaction,
                    save,
                    IoLimits::default(),
                    &[prepared.update],
                );
                if result.is_ok() {
                    session.gameplay_revision = prepared.revision;
                }
                result
            } else {
                runtime.apply_transaction(&transaction)
            };
            session.status = match result {
                Ok(_) => "Terrain edit applied to its affected partition".into(),
                Err(error) => format!("Edit refused: {error}"),
            };
        } else {
            session.status = "Edit is waiting for exact terrain".into();
        }
    }
    if session.save_requested && allow_save {
        session.save_requested = false;
        session.status = if let Some(save) = &options.save {
            match state::save(save, runtime, session) {
                Ok(()) => {
                    session.successful_saves += 1;
                    "Terrain and character checkpoint saved".into()
                }
                Err(error) => format!("Save failed: {error}"),
            }
        } else {
            "This session uses temporary edits; launch with --save DIRECTORY to persist terrain"
                .into()
        };
    }
    Ok(())
}

fn edit_objects(
    options: &Options,
    session: &mut Session,
    runtime: &mut WorldRuntime,
) -> Result<(), String> {
    if session.cancel_object_edit_requested {
        session.object_edit_requested = None;
        if let Some(mut removal) = session.object_removal.take() {
            removal.cancel(runtime).map_err(|error| error.to_string())?;
        }
        session.cancel_object_edit_requested = false;
        session.status = "Object edit cancelled".into();
    }
    if let Some(selection) = session.object_edit_requested.take() {
        if session.object_removal.is_some() {
            return Err("a second object command cannot replace a pinned edit".into());
        }
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        match object_edit::ObjectRemoval::begin(
            runtime,
            format!("explorer-object/{stamp:x}/{}", session.frames),
            selection,
            object_edit::RemovalLimits::default(),
        ) {
            Ok(removal) => session.object_removal = Some(removal),
            Err(error) => session.status = format!("Object edit refused: {error}"),
        }
    }
    let levels_tall =
        u32::try_from(TraversalProfile::WALKER.levels_tall).map_err(|error| error.to_string())?;
    let Some(mut removal) = session.object_removal.take() else {
        return Ok(());
    };
    let volumes = session
        .actors
        .iter()
        .flat_map(|actor| {
            actor
                .standing
                .into_iter()
                .chain(
                    actor
                        .motion
                        .into_iter()
                        .flat_map(|motion| [motion.from, motion.to]),
                )
                .chain(actor.route.front().copied())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .map(move |support| object_edit::ActorVolume {
                    actor_id: actor.id.clone(),
                    support,
                    levels_tall,
                })
        })
        .collect::<Vec<_>>();
    match removal.poll(runtime, &volumes) {
        Ok(object_edit::RemovalStatus::Pending(chunks)) => {
            session.status = format!("Object edit is loading {} exact dependencies", chunks.len());
            session.object_removal = Some(removal);
        }
        Ok(object_edit::RemovalStatus::Ready(transaction)) => {
            // The operation retains every dependency until the same-head owner
            // attachment and world delta commit have either succeeded or failed.
            let result = (|| -> Result<(), String> {
                if let Some(save) = &options.save {
                    let prepared = state::prepare(runtime, session)?;
                    runtime
                        .apply_object_transaction_durable_with_attachments(
                            &transaction,
                            save,
                            IoLimits::default(),
                            &[prepared.update],
                        )
                        .map_err(|error| error.to_string())?;
                    session.gameplay_revision = prepared.revision;
                } else {
                    runtime
                        .apply_object_transaction(&transaction)
                        .map_err(|error| error.to_string())?;
                }
                Ok(())
            })();
            removal.cancel(runtime).map_err(|error| error.to_string())?;
            session.status = match result {
                Ok(()) => {
                    session.successful_object_edits = session
                        .successful_object_edits
                        .checked_add(1)
                        .ok_or("object edit counter overflow")?;
                    "Object removed from all affected partitions".into()
                }
                Err(error) => format!("Object edit refused: {error}"),
            };
        }
        Err(error) => {
            removal.cancel(runtime).map_err(|error| error.to_string())?;
            session.status = format!("Object edit refused: {error}");
        }
    }
    Ok(())
}

fn actor_render_position(actor: &ExplorerActor, origin: RenderOrigin) -> Result<Vec3, String> {
    let position = actor.standing.ok_or("actor has no support")?;
    let at = render_position(position, origin)?;
    if let Some(motion) = actor.motion {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "presentation fraction is validated in the unit interval"
        )]
        let fraction = motion.fraction as f32;
        Ok(at.lerp(render_position(motion.to, origin)?, fraction))
    } else {
        Ok(at)
    }
}

fn render_position(position: VoxelPosition, origin: RenderOrigin) -> Result<Vec3, String> {
    let local = origin
        .local_voxel(position)
        .map_err(|error| error.to_string())?;
    #[expect(
        clippy::cast_precision_loss,
        reason = "render origin validates the bounded local level before this conversion"
    )]
    let height = (local.level as f32 + 1.0) * LEVEL_HEIGHT;
    Ok(local.coord.to_world(height))
}

fn update_view(
    session: Res<Session>,
    runtime: Res<ResidentWorld>,
    presenter: Res<TerrainPresenter>,
    options: Res<Options>,
    knowledge: Res<WorldKnowledge>,
    atlas: Res<atlas::AtlasState>,
    mut cameras: Query<&mut Transform, With<ExplorerCamera>>,
    mut hud: Query<(&mut Text, &mut Node), With<ExplorerHud>>,
) {
    if let Ok((mut text, mut node)) = hud.single_mut() {
        let display = if atlas.visible {
            Display::None
        } else {
            Display::Flex
        };
        if node.display != display {
            node.display = display;
        }
        let counts = runtime.0.counts();
        let view = knowledge.selected(&session);
        let updated = format!(
            "V4 World Explorer | {} regions | {}\n{}\n{} resident | {} rendered | {} loading\n{} explored columns | {} known landmarks{}\nClick: walk | D+click: remove | Escape: cancel edit\nS: save | Tab: party | Right-drag: orbit | Scroll: zoom | M: atlas\nT: {} | Space: one step",
            runtime.0.manifest().regions.len(),
            view.map_or("loading party", |view| view.principal.as_str()),
            session.error.as_ref().unwrap_or(&session.status),
            counts.resident_chunks,
            presenter.receipts().count(),
            counts.queued_chunks + counts.in_flight_jobs,
            view.map_or(0, |view| view.discovered_column_count()),
            view.map_or(0, |view| view
                .landmarks
                .values()
                .filter(|landmark| atlas::is_summary_landmark(runtime.0.manifest(), &landmark.id))
                .count()),
            if view.is_some_and(|view| !view.landmark_catalogue_complete) {
                " (nearby restore)"
            } else {
                ""
            },
            if session
                .actors
                .get(session.selected)
                .is_some_and(|actor| actor.turn_steps)
            {
                "step mode"
            } else {
                "continuous mode"
            }
        );
        if **text != updated {
            **text = updated;
        }
    }
    let Some(actor) = session
        .actors
        .get(session.selected)
        .filter(|actor| actor.standing.is_some())
    else {
        return;
    };
    let Ok(focus) = actor_render_position(actor, presenter.origin()) else {
        return;
    };
    let Ok(mut camera) = cameras.single_mut() else {
        return;
    };
    let yaw = session.yaw;
    if options.view == "first" {
        let eye = focus + Vec3::Y * 0.6;
        *camera = Transform::from_translation(eye)
            .looking_at(eye + Vec3::new(yaw.sin(), -0.04, yaw.cos()), Vec3::Y);
    } else {
        let pitch = if options.view == "top" {
            1.48
        } else {
            session.pitch
        };
        let distance = if options.view == "top" {
            (f32::from(u16::try_from(options.radius).unwrap_or(224)) * 4.0).max(130.0)
        } else {
            session.distance
        };
        let offset = Vec3::new(
            yaw.sin() * pitch.cos(),
            pitch.sin(),
            yaw.cos() * pitch.cos(),
        ) * distance;
        *camera = Transform::from_translation(focus + offset).looking_at(focus, Vec3::Y);
    }
}

#[derive(Serialize)]
struct CaptureReceipt {
    package: String,
    world_fingerprint: String,
    frames: u64,
    settled_frames: u64,
    settled_frame_samples_ms: Vec<f64>,
    gpu_completion: gpu_completion::CompletionReceipt,
    process_id: u32,
    captured_unix_seconds: f64,
    live_entities: usize,
    knowledge: knowledge::KnowledgeCounts,
    selected_principal: Option<String>,
    selected_discovered_columns: u64,
    selected_known_features: usize,
    selected_known_landmarks: usize,
    party_count: usize,
    successful_object_edits: u64,
    selected_landmark_catalogue_complete: bool,
    elapsed_seconds: f64,
    resident_chunks: usize,
    rendered_chunks: usize,
    mesh_publications: u64,
    discarded_mesh_jobs: u64,
    local_queue_peak: usize,
    rendered_vertices: usize,
    stock_art_fragments: usize,
    stock_art_variants: usize,
    stock_art_instance_vertices: usize,
    unresolved_stock_art: Vec<String>,
    frame_samples_ms: Vec<f64>,
    rebase_samples_ms: Vec<f64>,
    static_review: &'static str,
    native_motion: &'static str,
    scripted_walk: Option<serde_json::Value>,
}

fn capture(
    mut commands: Commands,
    options: Res<Options>,
    mut session: ResMut<Session>,
    runtime: Res<ResidentWorld>,
    presenter: Res<TerrainPresenter>,
    queue: Res<MeshQueue>,
    knowledge: Res<WorldKnowledge>,
    entities: Query<Entity>,
    walk: Option<Res<walk::WalkHarness>>,
    gpu: Option<Res<gpu_completion::GpuCompletion>>,
    mut exit: MessageWriter<AppExit>,
) {
    let completed_batches = if options.capture.is_some() {
        match gpu
            .as_deref()
            .ok_or_else(|| "windowless capture is missing GPU completion pacing".to_owned())
            .and_then(gpu_completion::GpuCompletion::completed_batches)
        {
            Ok(completed) => completed,
            Err(error) => {
                session.error = Some(error);
                0
            }
        }
    } else {
        0
    };
    if let Some(error) = &session.error {
        error!("V4 world failed: {error}");
        if options.capture.is_some() {
            exit.write(AppExit::error());
        }
        return;
    }
    if session.capture_requested
        || session.settled_frames < options.settle_frames
        || (options.capture.is_some() && completed_batches < options.settle_frames)
        || !knowledge.idle()
        || has_pending_activity(&session)
        || walk.as_ref().is_some_and(|walk| !walk.completed())
    {
        return;
    }
    let (Some(output), Some(target)) = (options.capture.clone(), session.target.clone()) else {
        return;
    };
    let scripted_walk = match walk
        .as_ref()
        .map(|walk| serde_json::to_value(walk.receipts()))
        .transpose()
    {
        Ok(value) => value,
        Err(error) => {
            session.error = Some(error.to_string());
            return;
        }
    };
    let captured_unix_seconds =
        match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            Ok(elapsed) => elapsed.as_secs_f64(),
            Err(error) => {
                session.error = Some(error.to_string());
                return;
            }
        };
    let tail = usize::try_from(session.settled_frames)
        .unwrap_or(session.frame_milliseconds.len())
        .min(session.frame_milliseconds.len());
    let Some(completion) = gpu.as_deref().cloned() else {
        session.error = Some("windowless capture lost GPU completion pacing".into());
        return;
    };
    let gpu_receipt = match completion.receipt() {
        Ok(receipt) => receipt,
        Err(error) => {
            session.error = Some(error);
            return;
        }
    };
    let selected = knowledge.selected(&session);
    let mut receipt = CaptureReceipt {
        package: options.package.display().to_string(),
        world_fingerprint: format!("{:016x}", runtime.0.manifest().fingerprint),
        frames: session.frames,
        settled_frames: session.settled_frames,
        gpu_completion: gpu_receipt,
        settled_frame_samples_ms: session
            .frame_milliseconds
            .iter()
            .skip(session.frame_milliseconds.len().saturating_sub(tail))
            .copied()
            .collect(),
        process_id: std::process::id(),
        captured_unix_seconds,
        live_entities: entities.iter().count(),
        knowledge: knowledge.counts(),
        selected_principal: selected.map(|view| view.principal.clone()),
        selected_discovered_columns: selected.map_or(0, |view| view.discovered_column_count()),
        selected_known_features: selected.map_or(0, |view| view.landmarks.len()),
        selected_known_landmarks: selected.map_or(0, |view| {
            view.landmarks
                .values()
                .filter(|landmark| atlas::is_summary_landmark(runtime.0.manifest(), &landmark.id))
                .count()
        }),
        party_count: session.actors.len(),
        successful_object_edits: session.successful_object_edits,
        selected_landmark_catalogue_complete: selected
            .is_some_and(|view| view.landmark_catalogue_complete),
        elapsed_seconds: session.started.elapsed().as_secs_f64(),
        resident_chunks: runtime.0.counts().resident_chunks,
        rendered_chunks: presenter.receipts().count(),
        mesh_publications: queue.published,
        discarded_mesh_jobs: queue.discarded,
        local_queue_peak: queue.peak_pending,
        rendered_vertices: presenter.receipts().map(|receipt| receipt.vertices).sum(),
        stock_art_fragments: queue.art.as_ref().map_or(0, |art| art.counts().0),
        stock_art_variants: queue.art.as_ref().map_or(0, |art| art.counts().1),
        stock_art_instance_vertices: queue.art.as_ref().map_or(0, |art| art.counts().2),
        unresolved_stock_art: queue
            .art
            .as_ref()
            .map_or_else(Vec::new, |art| art.unresolved()),
        frame_samples_ms: session.frame_milliseconds.clone(),
        rebase_samples_ms: session.rebase_milliseconds.clone(),
        static_review: "UNREVIEWED",
        native_motion: "HUMAN-MOTION-PENDING",
        scripted_walk,
    };
    commands.spawn(Screenshot::image(target)).observe(
        move |captured: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| {
            // Readback is later than the request. A timeout in that render batch
            // must still prevent publication of an authoritative success receipt.
            let result = (|| -> Result<(), String> {
                receipt.gpu_completion = completion.receipt()?;
                let receipt_bytes =
                    serde_json::to_vec_pretty(&receipt).map_err(|error| error.to_string())?;
                let stats = crate::capture::write_png(&captured.image, &output)?;
                if stats.brightest <= 8 || !stats.has_coverage {
                    return Err("capture is black or lacks visual coverage".into());
                }
                atomicwrites::AtomicFile::new(
                    output.with_extension("json"),
                    atomicwrites::DisallowOverwrite,
                )
                .write(|file| {
                    file.write_all(&receipt_bytes)?;
                    file.sync_all()
                })
                .map_err(|error| error.to_string())
            })();
            match result {
                Ok(()) => {
                    info!("V4 capture saved: {}", output.display());
                    exit.write(AppExit::Success);
                }
                Err(error) => {
                    error!("V4 capture failed: {error}");
                    exit.write(AppExit::error());
                }
            }
        },
    );
    session.capture_requested = true;
}

#[cfg(test)]
mod picking_tests {
    use super::*;
    use hex_world_contracts::{ColumnData, ObjectInstance, VoxelRun};

    #[test]
    fn requested_parties_use_distinct_declared_safe_anchors_independent_of_region_count() {
        let (_session, runtime) = walk::tests::fixture();
        let mut manifest = runtime.manifest().clone();
        manifest.regions.truncate(1);
        manifest.features = vec![
            hex_world_contracts::FeatureSummary {
                id: "a/hub".into(),
                region_id: "a".into(),
                kind: "entry".into(),
                anchor: VoxelPosition {
                    column: WorldHex::new(14, 0),
                    level: 2,
                },
                asset: None,
            },
            hex_world_contracts::FeatureSummary {
                id: "a/scenic".into(),
                region_id: "a".into(),
                kind: "observation".into(),
                anchor: VoxelPosition {
                    column: WorldHex::new(17, 0),
                    level: 90,
                },
                asset: None,
            },
            hex_world_contracts::FeatureSummary {
                id: "a/bridge".into(),
                region_id: "a".into(),
                kind: "transit".into(),
                anchor: VoxelPosition {
                    column: WorldHex::new(12, 0),
                    level: 2,
                },
                asset: None,
            },
        ];
        let points = party_spawn_points(&manifest, 2).expect("two exact safe anchors");
        assert_eq!(points.len(), 2);
        assert_eq!(
            points.last().map(|point| point.column),
            Some(WorldHex::new(12, 0))
        );
        assert!(
            party_spawn_points(&manifest, 3).is_err(),
            "scenic peaks do not become spawn fallbacks"
        );
        manifest.features.reverse();
        assert_eq!(
            party_spawn_points(&manifest, 2).expect("metadata order independent"),
            points
        );
    }

    #[test]
    fn capture_settle_budget_is_explicit_bounded_and_smaller_than_deadline() {
        let parse = |settle: &str, frames: &str| {
            Options::parse_arguments(
                [
                    "--world",
                    "fixture",
                    "--settle-frames",
                    settle,
                    "--frames",
                    frames,
                ]
                .map(str::to_owned),
            )
        };
        assert_eq!(
            parse("600", "1200")
                .expect("profiling budget")
                .settle_frames,
            600
        );
        assert!(parse("11", "1200").is_err());
        assert!(parse("10001", "12000").is_err());
        assert!(parse("600", "600").is_err());
        let defaults = Options::parse_arguments(["--world", "fixture"].map(str::to_owned))
            .expect("default options");
        assert_eq!(defaults.settle_frames, 120);
    }

    #[test]
    fn paused_step_route_is_settled_but_active_motion_and_edits_are_not() {
        let (mut session, _runtime) = walk::tests::fixture();
        let actor = session.actors.first_mut().expect("first actor");
        actor.turn_steps = true;
        let goal = VoxelPosition {
            column: WorldHex::new(15, 0),
            level: 2,
        };
        actor.route.push_back(goal);
        assert!(!has_pending_activity(&session));
        session.actors.first_mut().expect("first actor").turn_steps = false;
        assert!(has_pending_activity(&session));
        session
            .actors
            .first_mut()
            .expect("first actor")
            .route
            .clear();
        session.edit_requested = Some(EditRequest {
            position: goal,
            observed_revision: 0,
        });
        assert!(has_pending_activity(&session));
        session.edit_requested = None;
        session.save_requested = true;
        assert!(has_pending_activity(&session));
    }

    #[test]
    fn stock_pick_keeps_exact_large_world_identity_and_rejects_neighbor_or_foreign_fragment() {
        let column = WorldHex::new(9_000_000_000_015, -9_000_000_000_017);
        let object = ObjectInstance {
            id: "test/tree".into(),
            region_id: "test".into(),
            asset: "plant/tall-narrow".into(),
            origin: VoxelPosition { column, level: 40 },
            rotation: 0,
            occupancy: vec![ColumnData {
                position: column,
                runs: vec![VoxelRun {
                    bottom: 40,
                    top: 43,
                    material: "wood".into(),
                }],
            }],
        };
        let origin = RenderOrigin {
            column: column.chunk().origin().expect("chunk origin"),
            level: 40,
        };
        let local = origin.local_hex(column).expect("bounded local coordinate");
        let hit = local.to_world(0.0) + Vec3::Y * (3.0 * LEVEL_HEIGHT);
        let expected = VoxelPosition { column, level: 42 };
        assert_eq!(
            object_hit(&object, Some(column.chunk()), hit, Some(Vec3::Y), origin),
            Some((expected, expected))
        );
        let foreign = WorldHex::new(0, 0).chunk();
        assert!(object_hit(&object, Some(foreign), hit, Some(Vec3::Y), origin).is_none());
        assert!(object_hit(&object, None, hit + Vec3::X * 5.0, Some(Vec3::Y), origin).is_none());
        assert!(object_hit(&object, None, Vec3::splat(f32::NAN), None, origin).is_none());
    }
}
