//! Northern map presentation consumes compact world facts and gameplay flight state.
use super::{environment::UnderwaterTint, ArenaCamera, ArenaFrame, ViewState};
use bevy::prelude::*;
use hex_arena::ArenaSession;
use hex_core::arena::{
    ArenaAvailability, ArenaMap, ArenaRenderStatus, ArenaReset, ArenaSelection,
    ArenaStreamInterest, ArenaTerrainView, ArenaVoxelGeometry,
};
use hex_core::HexCoord;
use hex_map::arena::streamed::StreamedArena;
use hex_map::ocean::{
    sample_surface, OceanBathymetry, OceanBoundaryColumn, OceanFrame, OceanNearBoundary,
    OceanRenderStatus, OceanSurfaceProfile,
};
use hex_world::battle_sky::{BattleSkyFrame, BattleSkyProfile};

#[derive(Clone, Copy)]
struct CapturePose {
    camera: Transform,
    interest: Vec3,
}

#[derive(Resource, Default)]
struct NorthernPresentation {
    package: Option<u64>,
    generation: Option<u64>,
    phase: f32,
    boundary_center: Option<HexCoord>,
    boundary_revision: Option<u64>,
    capture: Option<CapturePose>,
    capture_view: String,
    enabled: bool,
}

#[derive(Component)]
struct FlightCue;

pub(super) fn install(app: &mut App) {
    hex_map::ocean::install(app);
    app.init_resource::<NorthernPresentation>()
        .add_systems(Startup, spawn_cue)
        .add_systems(Update, configure.before(ArenaFrame::Input))
        .add_systems(
            Update,
            interest.in_set(ArenaFrame::Input).after(super::input),
        )
        .add_systems(
            Update,
            (camera, present, cue)
                .chain()
                .in_set(ArenaFrame::Present)
                .after(super::environment::present),
        );
}

pub(super) fn sun_direction() -> Vec3 {
    let elevation = 18.0_f32.to_radians();
    let azimuth = 76.057_f32.to_radians();
    Vec3::new(
        azimuth.sin() * elevation.cos(),
        elevation.sin(),
        azimuth.cos() * elevation.cos(),
    )
}

pub(super) fn controls_text() -> &'static str {
    "F: open/fold exploration flight · WASD + mouse: steer · Space/Ctrl: rise/drop · Shift: fast flight\nG: momentum glider · Fireball and Shield work in flight · High Jump returns to gravity"
}

pub(super) fn fixture_view(view: &str) -> bool {
    matches!(
        view,
        "northern-overview"
            | "northern-bay"
            | "northern-settlement"
            | "northern-summit"
            | "northern-waterline"
            | "northern-underwater"
    )
}

/// Static publication readiness, independent of native motion and control feel.
pub(super) fn capture_ready(
    view: &str,
    streamed: Option<&StreamedArena>,
    terrain: Option<&ArenaRenderStatus>,
    ocean: Option<&OceanRenderStatus>,
) -> bool {
    if !fixture_view(view) {
        return true;
    }
    let Some(streamed) = streamed else {
        return false;
    };
    let counts = streamed.runtime.counts();
    streamed.failure.is_none()
        && counts.resident_chunks > 0
        && counts.in_flight_jobs == 0
        && counts.queued_chunks == 0
        && terrain.is_some_and(|status| status.pending_chunks == 0)
        && ocean.is_some_and(|status| {
            status.ready
                && status.error.is_none()
                && status.bathymetry_revision == Some(streamed.overview.package_fingerprint)
        })
}

/// Capture receipts describe admitted world facts; they do not claim motion performance.
pub(super) fn snapshot(
    streamed: Option<&StreamedArena>,
    terrain: Option<&ArenaRenderStatus>,
    ocean: Option<&OceanRenderStatus>,
) -> serde_json::Value {
    let Some(world) = streamed else {
        return serde_json::Value::Null;
    };
    let counts = world.runtime.counts();
    serde_json::json!({
        "world_id": world.overview.world_id,
        "source_fingerprint": world.overview.source_fingerprint,
        "package_fingerprint": world.overview.package_fingerprint,
        "islands": world.overview.islands.len(),
        "trees": world.overview.tree_count,
        "buildings": world.overview.building_count,
        "sea_level": world.overview.sea_level,
        "resident_chunks": counts.resident_chunks,
        "peak_resident_chunks": world.peak_resident,
        "queued_chunks": counts.queued_chunks,
        "in_flight_jobs": counts.in_flight_jobs,
        "source_chunks_retained": world.edits.resident_source_count(),
        "last_publication_ms": world.publication_ms,
        "failure": world.failure,
        "terrain_pending": terrain.map(|value| value.pending_chunks),
        "ocean": ocean.map(|value| serde_json::json!({
            "ready": value.ready,
            "surface_vertices": value.surface_vertices,
            "boundary_vertices": value.boundary_vertices,
            "bathymetry_revision": value.bathymetry_revision,
            "error": value.error,
            "phase_seconds": 0.0,
        })),
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "Atomic map presentation setup joins immutable publication with four presentation resources."
)]
fn configure(
    selection: Res<ArenaSelection>,
    reset: Res<ArenaReset>,
    state: Res<ViewState>,
    streamed: Option<Res<StreamedArena>>,
    mut cache: ResMut<NorthernPresentation>,
    mut bath: ResMut<OceanBathymetry>,
    mut profile: ResMut<OceanSurfaceProfile>,
    mut sky_profile: ResMut<BattleSkyProfile>,
) {
    let enabled = selection.map == ArenaMap::NorthernArchipelago;
    if enabled != cache.enabled {
        *sky_profile = if enabled {
            BattleSkyProfile::northern()
        } else {
            BattleSkyProfile::default()
        };
        cache.enabled = enabled;
    }
    if !enabled {
        cache.capture = None;
        return;
    }
    let Some(streamed) = streamed else {
        return;
    };
    let map = &streamed.overview;
    if cache.package != Some(map.package_fingerprint) {
        *bath = OceanBathymetry {
            revision: map.package_fingerprint,
            origin_xz: Vec2::from_array(map.origin_xz),
            spacing: map.spacing,
            width: map.width,
            height: map.height,
            bed_heights: map.bed_heights.clone(),
        };
        *profile = OceanSurfaceProfile {
            mean_sea_level: map.sea_level,
            ..default()
        };
        cache.package = Some(map.package_fingerprint);
        cache.boundary_center = None;
        cache.capture = None;
    }
    if cache.generation != Some(reset.generation) {
        cache.generation = Some(reset.generation);
        cache.phase = 0.0;
        cache.boundary_center = None;
    }
    if state.capture.is_some() && fixture_view(&state.capture_view) {
        if cache.capture.is_none() || cache.capture_view != state.capture_view {
            cache.capture = capture_pose(&state.capture_view, &streamed, &profile, &bath);
            cache.capture_view.clone_from(&state.capture_view);
        }
    } else {
        cache.capture = None;
    }
}

fn interest(
    session: Res<ArenaSession>,
    cache: Res<NorthernPresentation>,
    mut target: ResMut<ArenaStreamInterest>,
) {
    if !cache.enabled {
        return;
    }
    if let Some(capture) = cache.capture {
        // An explicitly windowless composition camera requests fine terrain at
        // its subject, never fabricating movement or discoveries for the actor.
        *target = ArenaStreamInterest {
            position: capture.interest,
            velocity: Vec3::ZERO,
        };
    } else if let Some(interest) = session.stream_interest() {
        *target = interest;
    }
}

fn camera(cache: Res<NorthernPresentation>, mut cameras: Query<&mut Transform, With<ArenaCamera>>) {
    if let Some(capture) = cache.capture {
        if let Ok(mut camera) = cameras.single_mut() {
            *camera = capture.camera;
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "Camera presentation joins exact local water, the visual ocean, sky and HUD-safe tint."
)]
fn present(
    time: Res<Time>,
    state: Res<ViewState>,
    view: Res<ArenaTerrainView>,
    geometry: Res<ArenaVoxelGeometry>,
    mut cache: ResMut<NorthernPresentation>,
    mut sky: ResMut<BattleSkyFrame>,
    profile: Res<OceanSurfaceProfile>,
    bath: Res<OceanBathymetry>,
    mut frame: ResMut<OceanFrame>,
    mut boundary: ResMut<OceanNearBoundary>,
    mut cameras: Query<(&Transform, &mut DistanceFog), With<ArenaCamera>>,
    mut overlays: Query<(&mut Node, &mut BackgroundColor), With<UnderwaterTint>>,
) {
    frame.enabled = cache.enabled && cache.package.is_some();
    if !frame.enabled {
        return;
    }
    if state.started && !state.paused && state.capture.is_none() {
        cache.phase += time.delta_secs();
    }
    // Explicit frozen phase zero matches the existing arena capture contract.
    frame.phase_seconds = if state.capture.is_some() {
        0.0
    } else {
        cache.phase.rem_euclid(900.0)
    };
    sky.enabled = true;
    sky.sun_direction = sun_direction();
    sky.cloud_phase = if state.capture.is_some() {
        0.0
    } else {
        cache.phase
    };
    let mut water = None;
    if let Ok((camera, mut fog)) = cameras.single_mut() {
        frame.camera_position = camera.translation;
        sky.center = camera.translation;
        publish_boundary(
            &view,
            *geometry,
            camera.translation,
            &mut cache,
            &mut boundary,
        );
        water = camera_water(
            &view,
            *geometry,
            &profile,
            &bath,
            camera.translation,
            frame.phase_seconds,
        );
        fog.color = water.unwrap_or(Color::srgb(0.46, 0.59, 0.73));
        fog.falloff = FogFalloff::Exponential {
            density: if water.is_some() { 0.09 } else { 0.00016 },
        };
        fog.directional_light_color = if water.is_some() {
            Color::NONE
        } else {
            Color::srgb(1.0, 0.76, 0.48)
        };
    }
    for (mut node, mut background) in &mut overlays {
        super::ux::set_display(
            &mut node,
            if water.is_some() {
                Display::Flex
            } else {
                Display::None
            },
        );
        background.set_if_neq(BackgroundColor(
            water.map_or(Color::NONE, |color| color.with_alpha(0.20)),
        ));
    }
}

fn camera_water(
    view: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    profile: &OceanSurfaceProfile,
    bath: &OceanBathymetry,
    camera: Vec3,
    phase: f32,
) -> Option<Color> {
    let sample = sample_surface(profile, bath, Vec2::new(camera.x, camera.z), phase)?;
    if camera.y >= sample.height {
        return None;
    }
    let coord = HexCoord::from_world(camera);
    let first = view
        .liquids
        .partition_point(|span| span.bottom.coord < coord);
    let in_volume = view
        .liquids
        .iter()
        .skip(first)
        .take_while(|span| span.bottom.coord == coord)
        .any(|span| {
            let top = geometry.top(hex_core::TilePos::new(coord, span.top_level));
            camera.y >= geometry.top(span.bottom) - geometry.level_height
                && (top - profile.mean_sea_level).abs() < 0.01
        });
    in_volume.then_some(Color::linear_rgb(
        sample.color.x,
        sample.color.y,
        sample.color.z,
    ))
}

fn publish_boundary(
    view: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    camera: Vec3,
    cache: &mut NorthernPresentation,
    boundary: &mut OceanNearBoundary,
) {
    let at = HexCoord::from_world(camera);
    // Moving the bounded window every eight columns avoids a mesh rebuild on
    // each camera nudge while keeping at least twenty units around the eye.
    let center = HexCoord::from_axial(at.x().div_euclid(8) * 8 + 4, at.y().div_euclid(8) * 8 + 4);
    let center_world = center.to_world(0.0);
    let moved = cache.boundary_center != Some(center);
    let revised = cache.boundary_revision != Some(view.revision);
    if !moved && !revised {
        return;
    }
    let local_change = view.full_rebuild
        || view
            .dirty_columns
            .iter()
            .any(|coord| coord.to_world(0.0).distance_squared(center_world) < 36.0 * 36.0);
    cache.boundary_revision = Some(view.revision);
    if !moved && !local_change {
        return;
    }
    cache.boundary_center = Some(center);
    boundary.columns.clear();
    boundary.known_columns.clear();
    for coord in center.within_radius(24) {
        let distance = coord.to_world(0.0).distance_squared(center_world);
        if distance > 34.0 * 34.0
            || view
                .residency
                .as_ref()
                .is_none_or(|r| r.at(coord) != ArenaAvailability::Ready)
        {
            continue;
        }
        // Include a liquid halo beyond known boundary columns: otherwise a
        // clipped-but-known wet neighbor would invent a vertical ocean wall.
        if distance <= 32.0 * 32.0 {
            boundary.known_columns.insert(coord);
        }
        let first = view
            .liquids
            .partition_point(|span| span.bottom.coord < coord);
        for span in view
            .liquids
            .iter()
            .skip(first)
            .take_while(|span| span.bottom.coord == coord)
        {
            boundary.columns.push(OceanBoundaryColumn {
                coordinate: coord,
                bottom: geometry.top(span.bottom) - geometry.level_height,
                top: geometry.top(hex_core::TilePos::new(coord, span.top_level)),
            });
        }
    }
    boundary.revision = boundary.revision.wrapping_add(1);
}

fn spawn_cue(mut commands: Commands) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: px(22),
            bottom: px(152),
            display: Display::None,
            padding: UiRect::all(px(7)),
            ..default()
        },
        super::hud::text("", 22.0, Color::srgb(0.86, 0.94, 1.0)),
        BackgroundColor(Color::srgba(0.025, 0.05, 0.08, 0.85)),
        GlobalZIndex(10),
        Pickable::IGNORE,
        FlightCue,
    ));
}
fn cue(
    state: Res<ViewState>,
    session: Res<ArenaSession>,
    mut labels: Query<(&mut Node, &mut Text), With<FlightCue>>,
) {
    let flight = session
        .actors
        .iter()
        .find(|a| Some(a.id) == session.human_actor_id())
        .and_then(|a| a.free_flight());
    let label = if state.started && !state.paused && state.capture.is_none() {
        flight.map_or_else(String::new, |f| {
            if f.loading {
                "Loading nearby terrain…".into()
            } else if f.active {
                format!("FLIGHT · {:.0} u/s · F to fold", f.velocity.length())
            } else {
                String::new()
            }
        })
    } else {
        String::new()
    };
    for (mut node, mut text) in &mut labels {
        super::ux::set_display(
            &mut node,
            if label.is_empty() {
                Display::None
            } else {
                Display::Flex
            },
        );
        if text.0 != label {
            text.0.clone_from(&label);
        }
    }
}

#[expect(
    clippy::cast_precision_loss,
    reason = "The bounded overview sample grid is at most 2048 by 2048."
)]
fn capture_pose(
    view: &str,
    streamed: &StreamedArena,
    profile: &OceanSurfaceProfile,
    bath: &OceanBathymetry,
) -> Option<CapturePose> {
    let map = &streamed.overview;
    let spawn = Vec3::from_array(map.player_spawn);
    let anchor = |name: &str| map.anchors.get(name).copied().map(Vec3::from_array);
    let bay = anchor("bay")?.with_y(map.sea_level);
    let (position, target, interest) = match view {
        "northern-overview" => (
            Vec3::new(1450.0, 1900.0, 1650.0),
            Vec3::new(0.0, map.sea_level + 60.0, 80.0),
            spawn,
        ),
        "northern-bay" => (spawn + Vec3::Y * 6.0, bay + Vec3::Y * 5.0, spawn),
        "northern-settlement" => {
            let site = anchor("settlement")?;
            (
                site + Vec3::new(-65.0, 42.0, 75.0),
                site + Vec3::Y * 4.0,
                site,
            )
        }
        "northern-summit" => {
            let (index, height) = map
                .bed_heights
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))?;
            let width = usize::try_from(map.width).ok()?;
            if width == 0 {
                return None;
            }
            let site = Vec3::new(
                map.origin_xz.first().copied()? + (index % width) as f32 * map.spacing,
                *height,
                map.origin_xz.get(1).copied()? + (index / width) as f32 * map.spacing,
            );
            (site + Vec3::new(70.0, 52.0, 90.0), site, site)
        }
        "northern-waterline" | "northern-underwater" => {
            let site = (0_u16..=100)
                .map(|step| spawn.lerp(bay, f32::from(step) / 100.0))
                .find(|p| {
                    sample_surface(profile, bath, Vec2::new(p.x, p.z), 0.0)
                        .is_some_and(|sample| sample.depth > 3.0)
                })?;
            let eye = site.with_y(
                map.sea_level
                    + if view == "northern-underwater" {
                        -1.2
                    } else {
                        0.7
                    },
            );
            (eye, bay.with_y(map.sea_level + 1.0), site)
        }
        _ => return None,
    };
    Some(CapturePose {
        camera: Transform::from_translation(position).looking_at(target, Vec3::Y),
        interest,
    })
}
