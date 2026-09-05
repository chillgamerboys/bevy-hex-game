//! V4 world-package composition root.
//!
//! This explicit explorer exercises authoring products, residency, existing terrain
//! meshes and exact gameplay motion. It does not install frozen V3 scenario/save
//! plugins or claim to implement encounter scheduling. The map provider, motion
//! consumer and disposable presentation remain separately owned.

mod atlas;
mod queue;

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
use hex_map::v4::{RenderOrigin, ResidentRun, TerrainPresenter};
use hex_units::v4::{plan_route, ContinuousStep, RouteResult, SearchLimits};
use hex_world_contracts::{
    ChunkId, QueryResult, ResidencyRequest, VoxelEdit, VoxelPosition, WorldEditTransaction,
    WorldHex, WorldQuery,
};
use hex_world_runtime::{FileChunkSource, IoLimits, RuntimeConfig, WorldRuntime};
use serde::Serialize;

use queue::MeshQueue;

const LEVEL_HEIGHT: f32 = 0.35;

#[derive(Resource, Clone)]
struct Options {
    package: PathBuf,
    save: Option<PathBuf>,
    capture: Option<PathBuf>,
    focus: Option<VoxelPosition>,
    radius: u32,
    frames: u64,
    azimuth: f32,
    view: String,
}

impl Options {
    fn parse() -> Result<Self, String> {
        let mut args = std::env::args().skip(1);
        let mut values = BTreeMap::new();
        while let Some(flag) = args.next() {
            if ![
                "--world",
                "--save",
                "--capture",
                "--focus",
                "--radius",
                "--frames",
                "--azimuth",
                "--view",
            ]
            .contains(&flag.as_str())
            {
                return Err(format!("unknown option {flag}; use --world PACKAGE [--save DIRECTORY] [--capture FRAME.png] [--focus q,r,level] [--view orbit|top|first|atlas]"));
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
        let focus = values
            .get("--focus")
            .map(|value| {
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
        let frames = values
            .get("--frames")
            .map_or(Ok(3600), |value| value.parse::<u64>())
            .map_err(|error| error.to_string())?;
        let azimuth = values
            .get("--azimuth")
            .map_or(Ok(35.0), |value| value.parse::<f32>())
            .map_err(|error| error.to_string())?;
        let view = values
            .get("--view")
            .cloned()
            .unwrap_or_else(|| "orbit".into());
        if !(16..=96).contains(&radius)
            || !(1..=100_000).contains(&frames)
            || !azimuth.is_finite()
            || !["orbit", "top", "first", "atlas"].contains(&view.as_str())
        {
            return Err("invalid radius (16..96), frames, azimuth, or view".into());
        }
        Ok(Self {
            package,
            save,
            capture,
            focus,
            radius,
            frames,
            azimuth,
            view,
        })
    }
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
    save_requested: bool,
    interests: Vec<(WorldHex, bool)>,
    rendered: BTreeMap<ChunkId, (u64, u64)>,
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
    let source = FileChunkSource::open(options.package.join("manifest.ron"), IoLimits::default())
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
    let mut actors = runtime
        .manifest()
        .regions
        .iter()
        .take(2)
        .map(|region| {
            let entry = runtime
                .manifest()
                .features
                .iter()
                .find(|feature| feature.region_id == region.id && feature.kind == "entry")
                .ok_or_else(|| format!("region {} has no declared entry anchor", region.id))?;
            Ok(ExplorerActor {
                id: format!("party/{}", region.id),
                column: entry.anchor.column,
                standing: None,
                requested_level: Some(entry.anchor.level),
                entity: None,
                route: VecDeque::new(),
                motion: None,
                pinned: false,
                turn_steps: false,
                requested: None,
                planning_pinned: false,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let first = actors.first_mut().ok_or("world has no regions")?;
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
    let presenter = TerrainPresenter::new(runtime.manifest(), origin, LEVEL_HEIGHT)
        .map_err(|error| error.to_string())?;
    let mut app = App::new();
    let plugins = DefaultPlugins.set(WindowPlugin {
        primary_window: if options.capture.is_some() {
            None
        } else {
            Some(Window {
                title: "Hex V4 — World Explorer".into(),
                resolution: (1440, 900).into(),
                ..default()
            })
        },
        ..default()
    });
    if options.capture.is_some() {
        app.add_plugins(plugins.disable::<WinitPlugin>());
        app.add_plugins(bevy::app::ScheduleRunnerPlugin::run_loop(
            std::time::Duration::from_secs_f64(1.0 / 60.0),
        ));
    } else {
        app.add_plugins(plugins);
    }
    app.add_plugins(MeshPickingPlugin);
    app.insert_resource(ClearColor(Color::srgb(0.085, 0.12, 0.15)));
    app.insert_resource(GlobalAmbientLight {
        color: Color::WHITE,
        brightness: 450.0,
        ..default()
    });
    app.insert_resource(Session {
        actors,
        selected: 0,
        edit_requested: None,
        save_requested: false,
        interests: Vec::new(),
        rendered: BTreeMap::new(),
        desired: BTreeSet::new(),
        status: "Loading nearby terrain…".into(),
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
    app.insert_resource(options);
    app.insert_resource(ResidentWorld(runtime));
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
            shadows_enabled: true,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.9, -0.5, 0.0)),
    ));
    commands.spawn((
        Text::new("Loading V4 world…"),
        TextFont {
            font_size: 17.0,
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
    presenter: Res<TerrainPresenter>,
    keys: Res<ButtonInput<KeyCode>>,
    mut session: ResMut<Session>,
    atlas: Res<atlas::AtlasState>,
) {
    if atlas.visible {
        return;
    }
    if event.event.button != PointerButton::Primary {
        return;
    }
    let Ok(batch) = batches.get(event.event_target()) else {
        return;
    };
    let Some(hit) = event.event.hit.position else {
        return;
    };
    let Some(entity) = batch.resolve_hit(hit, event.event.hit.normal) else {
        return;
    };
    if let Ok(run) = runs.get(entity) {
        if keys.pressed(KeyCode::KeyD) {
            if let Some(receipt) = presenter
                .receipts()
                .find(|receipt| receipt.coordinate == run.position.column.chunk())
            {
                session.edit_requested = Some(EditRequest {
                    position: clicked_voxel(run, hit, event.event.hit.normal, presenter.origin()),
                    observed_revision: receipt.revision,
                });
            }
        } else {
            let selected = session.selected;
            if let Some(actor) = session.actors.get_mut(selected) {
                actor.requested = Some(MoveRequest {
                    goal: run.position,
                    waiting: BTreeSet::new(),
                });
            }
        }
    }
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
    let elapsed = world.resource::<Time>().delta_secs_f64();
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
                        session.error = Some(error);
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
    let interests = session
        .actors
        .iter()
        .map(|actor| (actor.column, actor.motion.is_some()))
        .collect::<Vec<_>>();
    if interests != session.interests {
        runtime
            .set_interests(
                session
                    .actors
                    .iter()
                    .map(|actor| ResidencyRequest {
                        id: actor.id.clone(),
                        center: actor.column,
                        radius: options.radius + 16,
                        retention_radius: options.radius + 32,
                        priority: 10,
                    })
                    .collect(),
            )
            .map_err(|error| error.to_string())?;
        session.interests = interests;
    }
    let updates = runtime.pump();
    if let Some(failure) = updates.failures.first() {
        return Err(format!("chunk {:?}: {}", failure.coordinate, failure.error));
    }
    for coordinate in updates.removed {
        let _removed = presenter.remove(world, coordinate);
        session.rendered.remove(&coordinate);
        queue.forget(coordinate);
    }
    let art_ready = {
        let art = world.resource::<ExplorerArt>();
        let server = world.resource::<AssetServer>();
        for handle in &art.pieces {
            if let Some(bevy::asset::LoadState::Failed(error)) = server.get_load_state(handle.id())
            {
                return Err(format!("stock player asset failed: {error}"));
            }
        }
        art.pieces
            .iter()
            .all(|handle| world.resource::<Assets<Mesh>>().contains(handle.id()))
    };
    for actor in &mut session.actors {
        if actor.standing.is_none() {
            if let QueryResult::Ready(surfaces) = runtime.surfaces(actor.column) {
                let surface = surfaces
                    .into_iter()
                    .filter(|surface| surface.headroom.is_none_or(|clearance| clearance >= 2))
                    .filter(|surface| {
                        actor
                            .requested_level
                            .is_none_or(|level| surface.position.level == level)
                    })
                    .last()
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
    move_actors(session, runtime, elapsed)?;
    edit_and_save(options, session, runtime)?;
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
    let retired = session
        .rendered
        .keys()
        .filter(|coordinate| !desired.contains(coordinate))
        .copied()
        .collect::<Vec<_>>();
    for coordinate in retired {
        let _removed = presenter.remove(world, coordinate);
        session.rendered.remove(&coordinate);
        queue.forget(coordinate);
    }
    let level = selected
        .standing
        .map_or(selected.requested_level.unwrap_or(0), |position| {
            position.level
        });
    if presenter
        .origin()
        .column
        .checked_distance(center)
        .map_err(|error| error.to_string())?
        > 128
        || (i64::from(presenter.origin().level) - i64::from(level)).unsigned_abs() > 1024
    {
        // Retain current nearby roots through an atomic origin change. Canceled
        // worker outputs carry the old epoch and cannot replace these meshes.
        queue.cancel();
        let started = Instant::now();
        presenter
            .rebase(
                world,
                RenderOrigin {
                    column: center.chunk().origin().map_err(|error| error.to_string())?,
                    level,
                },
            )
            .map_err(|error| error.to_string())?;
        session
            .rebase_milliseconds
            .push(started.elapsed().as_secs_f64() * 1000.0);
        session.rendered = presenter
            .receipts()
            .map(|receipt| (receipt.coordinate, (receipt.revision, receipt.fingerprint)))
            .collect();
    }
    for product in runtime
        .resident_chunks()
        .filter(|product| desired.contains(&product.coordinate))
    {
        let signature = (product.revision, product.package.fingerprint);
        if session.rendered.get(&product.coordinate) != Some(&signature) {
            session.rendered.insert(product.coordinate, signature);
            queue.enqueue(product);
            session.settled_frames = 0;
        }
    }
    session.desired = desired;
    queue.tick(world, runtime, presenter, &session.desired)?;
    let origin = presenter.origin();
    for actor in &session.actors {
        if let (Some(entity), Some(position)) = (actor.entity, actor.standing) {
            let visible = position
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
) -> Result<(), String> {
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
                runtime.apply_transaction_durable(&transaction, save, IoLimits::default())
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
    if session.save_requested {
        session.save_requested = false;
        session.status = if let Some(save) = &options.save {
            match runtime.save(save, IoLimits::default()) {
                Ok(()) => "Terrain partitions saved".into(),
                Err(error) => format!("Save failed: {error}"),
            }
        } else {
            "This session uses temporary edits; launch with --save DIRECTORY to persist terrain"
                .into()
        };
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
    mut cameras: Query<&mut Transform, With<ExplorerCamera>>,
    mut hud: Query<&mut Text, With<ExplorerHud>>,
) {
    if let Ok(mut text) = hud.single_mut() {
        let counts = runtime.0.counts();
        **text = format!("V4 World Explorer · {} regions\n{}\n{} resident chunks · {} visible · {} loading\nClick to walk · D+click dig · S save terrain · Tab switch party\nRight-drag orbit · Scroll zoom · T: {} · Space: advance one step", runtime.0.manifest().regions.len(), session.error.as_ref().unwrap_or(&session.status), counts.resident_chunks, session.desired.len(), counts.queued_chunks + counts.in_flight_jobs, if session.actors.get(session.selected).is_some_and(|actor| actor.turn_steps) { "step mode" } else { "continuous mode" });
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
            130.0
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
    elapsed_seconds: f64,
    resident_chunks: usize,
    rendered_chunks: usize,
    mesh_publications: u64,
    discarded_mesh_jobs: u64,
    local_queue_peak: usize,
    rendered_vertices: usize,
    frame_samples_ms: Vec<f64>,
    rebase_samples_ms: Vec<f64>,
    static_review: &'static str,
    native_motion: &'static str,
}

fn capture(
    mut commands: Commands,
    options: Res<Options>,
    mut session: ResMut<Session>,
    runtime: Res<ResidentWorld>,
    presenter: Res<TerrainPresenter>,
    queue: Res<MeshQueue>,
    mut exit: MessageWriter<AppExit>,
) {
    if let Some(error) = &session.error {
        error!("V4 world failed: {error}");
        if options.capture.is_some() {
            exit.write(AppExit::error());
        }
        return;
    }
    if session.capture_requested || session.settled_frames < 12 {
        return;
    }
    let (Some(output), Some(target)) = (options.capture.clone(), session.target.clone()) else {
        return;
    };
    let receipt = CaptureReceipt {
        package: options.package.display().to_string(),
        world_fingerprint: format!("{:016x}", runtime.0.manifest().fingerprint),
        frames: session.frames,
        elapsed_seconds: session.started.elapsed().as_secs_f64(),
        resident_chunks: runtime.0.counts().resident_chunks,
        rendered_chunks: presenter.receipts().count(),
        mesh_publications: queue.published,
        discarded_mesh_jobs: queue.discarded,
        local_queue_peak: queue.peak_pending,
        rendered_vertices: presenter.receipts().map(|receipt| receipt.vertices).sum(),
        frame_samples_ms: session.frame_milliseconds.clone(),
        rebase_samples_ms: session.rebase_milliseconds.clone(),
        static_review: "UNREVIEWED",
        native_motion: "HUMAN-MOTION-PENDING",
    };
    let receipt_bytes = match serde_json::to_vec_pretty(&receipt) {
        Ok(bytes) => bytes,
        Err(error) => {
            session.error = Some(error.to_string());
            return;
        }
    };
    commands.spawn(Screenshot::image(target)).observe(
        move |captured: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| {
            let result = crate::capture::write_png(&captured.image, &output).and_then(|stats| {
                if stats.brightest <= 8 || !stats.has_coverage {
                    return Err("capture is black or lacks visual coverage".into());
                }
                std::fs::write(output.with_extension("json"), &receipt_bytes)
                    .map_err(|error| error.to_string())
            });
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
