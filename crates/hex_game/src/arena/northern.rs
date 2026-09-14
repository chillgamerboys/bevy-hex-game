//! Northern map presentation consumes compact world facts and gameplay flight state.
use super::{environment::UnderwaterTint, ArenaCamera, ArenaFrame, ViewState};
use bevy::camera::ScalingMode;
use bevy::core_pipeline::prepass::DepthPrepass;
use bevy::prelude::*;
use hex_arena::ArenaSession;
use hex_core::arena::{
    ArenaAvailability, ArenaMap, ArenaRenderStatus, ArenaReset, ArenaSelection,
    ArenaStreamInterest, ArenaTerrainView, ArenaVoxelGeometry,
};
use hex_core::ocean::{OceanEnvironmentView, OceanSimulationTime, OceanWindProfile};
use hex_core::HexCoord;
use hex_map::arena::streamed::StreamedArena;
use hex_map::ocean::{
    sample_local_surface, sample_surface, OceanBathymetry, OceanBoundaryColumn, OceanFrame,
    OceanNearBoundary, OceanRenderStatus, OceanSurfaceAdapter, OceanSurfaceProfile,
};
use hex_world::battle_sky::{BattleSkyFrame, BattleSkyProfile};
use std::sync::Arc;

#[derive(Clone, Copy)]
struct CapturePose {
    camera: Transform,
    interest: Vec3,
    overview_height: Option<f32>,
}

#[derive(Resource, Default)]
pub(super) struct NorthernPresentation {
    package: Option<u64>,
    generation: Option<u64>,
    boundary_center: Option<HexCoord>,
    boundary_revision: Option<u64>,
    capture: Option<CapturePose>,
    capture_view: String,
    enabled: bool,
    boat_fixture_tick: Option<u64>,
}

#[derive(Component)]
struct FlightCue;

pub(super) fn install(app: &mut App) {
    hex_map::ocean::install(app);
    app.init_resource::<NorthernPresentation>()
        .add_systems(Startup, spawn_cue)
        .add_systems(
            Update,
            (configure, ocean_depth).chain().before(ArenaFrame::Input),
        )
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

// Only ocean presentation needs opaque scene depth for underwater sight length.
// Leaving Northern restores the existing Forest/Duel/Fort camera pipeline.
fn ocean_depth(
    mut commands: Commands,
    selection: Res<ArenaSelection>,
    cameras: Query<(Entity, Has<DepthPrepass>), With<ArenaCamera>>,
) {
    let enabled = selection.map == ArenaMap::NorthernArchipelago;
    for (entity, present) in &cameras {
        if enabled && !present {
            commands.entity(entity).insert(DepthPrepass);
        } else if !enabled && present {
            commands.entity(entity).remove::<DepthPrepass>();
        }
    }
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
    "M: minimap · V: wind direction/speed (arrow points downwind)\nF: open/fold exploration flight · WASD + mouse: steer · Space/Ctrl: rise/drop · Shift: fast flight\nB: deploy/fold sailboat near water · W: sail/paddle · A/D: steer · S: brake\nSwimming: Space rises, Ctrl dives · 90 seconds of oxygen\nG: momentum glider · Fireball and Shield work in flight · High Jump returns to gravity"
}

pub(super) fn fixture_view(view: &str) -> bool {
    matches!(
        view,
        "northern-overview"
            | "northern-bay"
            | "northern-bay-flat"
            | "northern-settlement"
            | "northern-summit"
            | "northern-waterline"
            | "northern-underwater"
            | "northern-boat"
    )
}

/// Static publication readiness, independent of native motion and control feel.
pub(super) fn capture_ready(
    view: &str,
    presentation: &NorthernPresentation,
    streamed: Option<&StreamedArena>,
    terrain: Option<&ArenaRenderStatus>,
    ocean: Option<&OceanRenderStatus>,
) -> bool {
    if !fixture_view(view) {
        return true;
    }
    if presentation.capture.is_none() {
        return false;
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
            "phase_seconds": value.phase_seconds,
        })),
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "Atomic map presentation setup joins immutable publication with four presentation resources."
)]
fn configure(
    mut commands: Commands,
    environment: Option<Res<OceanEnvironmentView>>,
    selection: Res<ArenaSelection>,
    reset: Res<ArenaReset>,
    state: Res<ViewState>,
    session: Res<ArenaSession>,
    streamed: Option<Res<StreamedArena>>,
    terrain: Res<ArenaTerrainView>,
    geometry: Res<ArenaVoxelGeometry>,
    mut cache: ResMut<NorthernPresentation>,
    mut bath: ResMut<OceanBathymetry>,
    mut profile: ResMut<OceanSurfaceProfile>,
    mut sky_profile: ResMut<BattleSkyProfile>,
    mut exit: MessageWriter<AppExit>,
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
        if environment.is_some() {
            commands.remove_resource::<OceanEnvironmentView>();
        }
        cache.capture = None;
        return;
    }
    let Some(streamed) = streamed else {
        return;
    };
    let map = &streamed.overview;
    if cache.package != Some(map.package_fingerprint) {
        let prepared = OceanBathymetry {
            revision: map.package_fingerprint,
            origin_xz: Vec2::from_array(map.origin_xz),
            spacing: map.spacing,
            width: map.width,
            height: map.height,
            bed_heights: map.bed_heights.clone(),
            ..default()
        }
        .with_shore_shelter(map.sea_level, 80.0);
        match prepared {
            Ok(prepared) => *bath = prepared,
            Err(error) => {
                error!("Northern shoreline presentation: {error}");
                exit.write(AppExit::error());
                return;
            }
        }
        *profile = OceanSurfaceProfile {
            mean_sea_level: map.sea_level,
            ..default()
        };
        // Capture-only baseline retains identical water membership and depth absorption.
        if state.capture.is_some() && state.capture_view == "northern-bay-flat" {
            for wave in &mut profile.waves {
                wave.amplitude = 0.0;
            }
        }
        cache.package = Some(map.package_fingerprint);
        cache.boundary_center = None;
        cache.capture = None;
    }
    if environment
        .as_ref()
        .is_none_or(|environment| environment.package_fingerprint != map.package_fingerprint)
    {
        match OceanSurfaceAdapter::new(profile.clone(), bath.clone()) {
            Ok(adapter) => {
                commands.insert_resource(OceanEnvironmentView {
                    package_fingerprint: map.package_fingerprint,
                    sampler: Arc::new(adapter),
                    wind: OceanWindProfile::default(),
                });
            }
            Err(error) => {
                error!("Northern ocean environment: {error}");
                exit.write(AppExit::error());
                return;
            }
        }
    }
    if cache.generation != Some(reset.generation) {
        cache.generation = Some(reset.generation);
        cache.boundary_center = None;
    }
    if state.capture.is_some() && fixture_view(&state.capture_view) {
        if cache.capture.is_none() || cache.capture_view != state.capture_view {
            cache.capture = capture_pose(
                &state.capture_view,
                &session,
                &streamed,
                &profile,
                &bath,
                &terrain,
                *geometry,
            );
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

fn camera(
    cache: Res<NorthernPresentation>,
    mut cameras: Query<(&mut Transform, &mut Projection), With<ArenaCamera>>,
) {
    if let Some(capture) = cache.capture {
        if let Ok((mut camera, mut projection)) = cameras.single_mut() {
            *camera = capture.camera;
            if let Some(height) = capture.overview_height {
                if !matches!(&*projection, Projection::Orthographic(_)) {
                    *projection = Projection::Orthographic(OrthographicProjection {
                        scaling_mode: ScalingMode::FixedVertical {
                            viewport_height: height,
                        },
                        near: 0.035,
                        far: 24_000.0,
                        ..OrthographicProjection::default_3d()
                    });
                }
            } else if matches!(&*projection, Projection::Orthographic(_)) {
                *projection = Projection::Perspective(PerspectiveProjection {
                    fov: 75.0_f32.to_radians(),
                    near: 0.035,
                    far: 24_000.0,
                    ..default()
                });
            }
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "Camera presentation joins exact local water, the visual ocean, sky and HUD-safe tint."
)]
fn present(
    time: Res<OceanSimulationTime>,
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
    // Explicit frozen phase zero matches the existing arena capture contract.
    frame.phase_seconds = if state.capture.is_some() && state.capture_view != "northern-boat" {
        0.0
    } else {
        time.phase_seconds()
    };
    sky.enabled = true;
    sky.sun_direction = sun_direction();
    #[expect(
        clippy::cast_possible_truncation,
        reason = "Cloud translation uses the same elapsed run time; f32 precision is ample for its slow drift."
    )]
    let cloud_seconds = time.seconds as f32;
    sky.cloud_phase = if state.capture.is_some() {
        0.0
    } else {
        cloud_seconds
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
            &boundary,
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
    sky.underwater_color = water.map(|color| {
        let color = color.to_linear();
        Vec3::new(color.red, color.green, color.blue)
    });
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
    boundary: &OceanNearBoundary,
    camera: Vec3,
    phase: f32,
) -> Option<Color> {
    let sample = sample_local_surface(
        profile,
        bath,
        boundary,
        Vec2::new(camera.x, camera.z),
        phase,
    )?;
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
                .is_none_or(|r| r.at(coord, geometry) != ArenaAvailability::Ready)
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
    session: &ArenaSession,
    streamed: &StreamedArena,
    profile: &OceanSurfaceProfile,
    bath: &OceanBathymetry,
    terrain: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
) -> Option<CapturePose> {
    let map = &streamed.overview;
    let spawn = Vec3::from_array(map.player_spawn);
    if view == "northern-overview" {
        return Some(overview_pose(
            Vec2::from_array(map.origin_xz),
            Vec2::new(
                map.width.saturating_sub(1) as f32,
                map.height.saturating_sub(1) as f32,
            ) * map.spacing,
            map.sea_level,
            map.bed_heights
                .iter()
                .copied()
                .fold(map.sea_level, f32::max),
            spawn,
        ));
    }
    let anchor = |name: &str| map.anchors.get(name).copied().map(Vec3::from_array);
    let bay = anchor("bay")?.with_y(map.sea_level);
    let (position, target, interest) = match view {
        "northern-boat" => {
            let actor = session
                .human_actor_id()
                .and_then(|id| session.actors.iter().find(|actor| actor.id == id))?;
            let boat = actor.boat().filter(|boat| boat.active)?;
            let side = boat.heading.cross(Vec3::Y);
            (
                actor.feet - boat.heading * 5.5 - side * 4.0 + Vec3::Y * 3.0,
                actor.feet + Vec3::Y * 0.8,
                actor.feet,
            )
        }
        "northern-bay" | "northern-bay-flat" => (spawn + Vec3::Y * 6.0, bay + Vec3::Y * 5.0, spawn),
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
            let site = waterline_site(terrain, geometry, spawn, bay, map.sea_level)?;
            let surface = sample_surface(profile, bath, Vec2::new(site.x, site.z), 0.0)
                .map_or(map.sea_level, |sample| sample.height);
            let eye = site.with_y(
                surface
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
        overview_height: None,
    })
}

// A coarse height sample can misclassify a steep coast. Wait for actual admitted
// liquid intervals so the underwater fixture cannot start inside the seabed.
fn waterline_site(
    terrain: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    spawn: Vec3,
    bay: Vec3,
    sea: f32,
) -> Option<Vec3> {
    (0_u16..=100)
        .map(|step| spawn.lerp(bay, f32::from(step) / 100.0))
        .find(|point| {
            let coord = HexCoord::from_world(*point);
            let first = terrain
                .liquids
                .partition_point(|span| span.bottom.coord < coord);
            terrain
                .liquids
                .iter()
                .skip(first)
                .take_while(|span| span.bottom.coord == coord)
                .any(|span| {
                    let top = geometry.top(hex_core::TilePos::new(coord, span.top_level));
                    let bottom = geometry.top(span.bottom) - geometry.level_height;
                    (top - sea).abs() < 0.01 && top - bottom >= 5.0
                })
        })
}

/// Fit the complete finite footprint, including its highest visible terrain, with
/// a ten-percent frame margin. Orthographic overview avoids the old distant wide
/// lens shrinking all three clusters; other captures retain the gameplay lens.
fn overview_pose(origin: Vec2, extent: Vec2, sea: f32, summit: f32, interest: Vec3) -> CapturePose {
    let center = origin + extent * 0.5;
    let target = Vec3::new(center.x, (sea + summit) * 0.5, center.y);
    let back = Vec3::new(0.0, 1.65, 1.0).normalize();
    let up = back.cross(Vec3::X);
    let corners = overview_corners(origin, extent, sea, summit);
    let mut half_width: f32 = 0.0;
    let mut half_height: f32 = 0.0;
    let mut near_depth: f32 = 0.0;
    for corner in corners {
        let local = corner - target;
        half_width = half_width.max(local.x.abs());
        half_height = half_height.max(local.dot(up).abs());
        near_depth = near_depth.max(local.dot(back));
    }
    // The windowless arena canvas is fixed at 1600 × 900.
    let aspect = 16.0 / 9.0;
    let height = 2.0 * half_height.max(half_width / aspect) / 0.9;
    CapturePose {
        camera: Transform::from_translation(target + back * (near_depth + 800.0))
            .looking_at(target, Vec3::Y),
        interest,
        overview_height: Some(height),
    }
}

fn overview_corners(origin: Vec2, extent: Vec2, sea: f32, summit: f32) -> [Vec3; 8] {
    std::array::from_fn(|index| {
        Vec3::new(
            origin.x + if index & 1 == 0 { 0.0 } else { extent.x },
            if index & 2 == 0 { sea } else { summit },
            origin.y + if index & 4 == 0 { 0.0 } else { extent.y },
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waterline_fixture_waits_for_deep_admitted_water() {
        use hex_core::arena::ArenaSolidSpan;
        let geometry = ArenaVoxelGeometry {
            level_height: 1.0,
            vertical_offset: 1.0,
            ..default()
        };
        let spawn = HexCoord::ORIGIN.to_world(10.0);
        let deep = HexCoord::from_axial(2, 0);
        let bay = deep.to_world(10.0);
        let mut terrain = ArenaTerrainView::default();
        assert!(waterline_site(&terrain, geometry, spawn, bay, 10.0).is_none());
        terrain.liquids.push(ArenaSolidSpan {
            bottom: hex_core::TilePos::new(HexCoord::from_axial(1, 0), 8),
            top_level: 9,
            substance: hex_core::SubstanceId::AIR,
        });
        assert!(waterline_site(&terrain, geometry, spawn, bay, 10.0).is_none());
        terrain.liquids.push(ArenaSolidSpan {
            bottom: hex_core::TilePos::new(deep, 2),
            top_level: 9,
            substance: hex_core::SubstanceId::AIR,
        });
        let site = waterline_site(&terrain, geometry, spawn, bay, 10.0)
            .expect("the admitted deep interval is suitable for both water views");
        assert_eq!(HexCoord::from_world(site), deep);
    }

    #[test]
    fn overview_fits_complete_published_bounds_with_margin() {
        let origin = Vec2::new(-1216.0, -1056.0);
        let extent = Vec2::new(2432.0, 2112.0);
        let interest = Vec3::new(-645.0, 161.7, -100.5);
        let pose = overview_pose(origin, extent, 140.0, 442.75, interest);
        let half_height = pose.overview_height.expect("orthographic overview") * 0.5;
        let half_width = half_height * (16.0 / 9.0);
        for corner in overview_corners(origin, extent, 140.0, 442.75) {
            let local = pose.camera.rotation.inverse() * (corner - pose.camera.translation);
            assert!(local.x.abs() <= half_width * 0.901);
            assert!(local.y.abs() <= half_height * 0.901);
            assert!(local.z < -0.035 && local.z > -24_000.0);
        }
        // The landscape occupies useful image width while all world edges remain included.
        assert!(1528.0 / (2.0 * half_width) > 0.38);
        assert_eq!(pose.interest, interest);
    }
}

/// Explicit windowless fixture: move the player to admitted water and press B.
/// The normal controller owns deployment; this is presentation staging, not travel evidence.
pub(super) fn stage_boat_capture(world: &mut World, view: &str) -> Result<(), String> {
    use hex_core::ocean::{OceanSurfaceState, OceanWaterColumn};
    if view != "northern-boat" || boat_capture_ready(world.resource::<ArenaSession>(), view) {
        return Ok(());
    }
    let tick = world.resource::<ArenaSession>().tick;
    if tick < 2 {
        return Ok(());
    }
    if let Some(attempt) = world.resource::<NorthernPresentation>().boat_fixture_tick {
        if tick > attempt + 2 {
            return Err(format!(
                "Boat fixture did not deploy: {}",
                world.resource::<ArenaSession>().notice
            ));
        }
        return Ok(());
    }
    let Some(streamed) = world.get_resource::<StreamedArena>() else {
        return Ok(());
    };
    let Some(environment) = world.get_resource::<OceanEnvironmentView>() else {
        return Ok(());
    };
    let map = &streamed.overview;
    let Some(bay) = map.anchors.get("bay").copied().map(Vec3::from_array) else {
        return Err("Boat fixture requires the authored bay".into());
    };
    let spawn = Vec3::from_array(map.player_spawn);
    let terrain = world.resource::<ArenaTerrainView>();
    let geometry = *world.resource::<ArenaVoxelGeometry>();
    let Some(first_water) = waterline_site(terrain, geometry, spawn, bay, map.sea_level) else {
        return Ok(());
    };
    let site = first_water.lerp(bay, 0.35);
    let coord = HexCoord::from_world(site);
    let available = terrain
        .residency
        .as_ref()
        .map_or(ArenaAvailability::OutsideWorld, |residency| {
            residency.at(coord, geometry)
        });
    let start = terrain
        .liquids
        .partition_point(|span| span.bottom.coord < coord);
    let column = terrain
        .liquids
        .iter()
        .skip(start)
        .take_while(|span| span.bottom.coord == coord)
        .last()
        .map(|span| OceanWaterColumn {
            mean_height: geometry.top(hex_core::TilePos::new(coord, span.top_level)),
            bed_height: geometry.top(span.bottom) - geometry.level_height,
            water_id: span.substance,
        });
    let OceanSurfaceState::ReadyWet(surface) = environment.sample(
        Vec2::new(site.x, site.z),
        world.resource::<ArenaSession>().ocean_time(),
        available,
        column,
    ) else {
        return Ok(());
    };
    let feet = site.with_y(surface.height + 0.2);
    let heading = (bay - site).with_y(0.0).normalize_or(Vec3::NEG_Z);
    let mut session = world.resource_mut::<ArenaSession>();
    let human = session
        .human_actor_id()
        .ok_or("Boat fixture requires a player")?;
    let actor = session
        .actors
        .iter_mut()
        .find(|actor| actor.id == human)
        .ok_or("Missing boat fixture player")?;
    actor.feet = feet;
    actor.previous_feet = feet;
    actor.aim = heading;
    let intent = hex_arena::ActorIntent {
        boat_toggle: true,
        aim: heading,
        ..default()
    };
    world.resource_mut::<hex_arena::ArenaInput>().human = intent;
    if let Some((_, recorded)) = world.resource_mut::<ViewState>().capture_inputs.last_mut() {
        *recorded = intent;
    }
    world
        .resource_mut::<NorthernPresentation>()
        .boat_fixture_tick = Some(tick);
    Ok(())
}

pub(super) fn boat_capture_ready(session: &ArenaSession, view: &str) -> bool {
    view == "northern-boat"
        && session
            .human_actor_id()
            .and_then(|id| session.actors.iter().find(|actor| actor.id == id))
            .and_then(hex_arena::Actor::boat)
            .is_some_and(|boat| boat.active)
}
