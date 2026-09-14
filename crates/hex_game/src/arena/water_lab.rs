//! Native controls and evidence views for the finite water experiment.
use super::*;
use hex_core::ocean::{OceanEnvironmentView, OceanSimulationTime, OceanWindProfile};
use hex_core::water_lab::{LabStart, LabStyle, LabWave, LabWind, WaterLabSettings, WATER_LAB_ID};
use hex_map::water_lab::{LabSurface, WaterLabFrame};
use std::sync::Arc;

#[derive(Resource)]
struct LabUi {
    visible: bool,
    capture_staged: bool,
}
impl Default for LabUi {
    fn default() -> Self {
        Self {
            visible: std::env::var_os("HEX_ARENA_CAPTURE").is_none(),
            capture_staged: false,
        }
    }
}
#[derive(Component)]
struct Panel;
#[derive(Component)]
struct Details;

pub(super) fn install(app: &mut App) {
    hex_map::water_lab::install(app);
    let mut settings = WaterLabSettings::default();
    if let Ok(value) = std::env::var("HEX_WATER_LAB_WAVE") {
        settings.wave = match value.as_str() {
            "flat" => LabWave::Flat,
            "swell" => LabWave::Swell,
            "crossing" => LabWave::Crossing,
            _ => LabWave::Regular,
        };
    }
    if let Ok(value) = std::env::var("HEX_WATER_LAB_STYLE") {
        settings.style = match value.as_str() {
            "depth" => LabStyle::Depth,
            "crests" => LabStyle::Crests,
            _ => LabStyle::Patterns,
        };
    }
    if let Ok(value) = std::env::var("HEX_WATER_LAB_WIND") {
        settings.wind = match value.as_str() {
            "calm" => LabWind::Calm,
            "strong" => LabWind::Strong,
            "gusts" => LabWind::Gusts,
            "turning" => LabWind::Turning,
            "shelter" => LabWind::Shelter,
            _ => LabWind::Steady,
        };
    }
    if let Some(phase) = std::env::var("HEX_WATER_LAB_PHASE")
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|value| value.is_finite())
    {
        settings.frozen_phase = Some(phase.rem_euclid(900.0));
    }
    if let Some(scale) = std::env::var("HEX_WATER_LAB_GLIDER_WIND")
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
    {
        settings.glider_wind_scale = scale;
    }
    app.insert_resource(settings);
    app.init_resource::<LabUi>()
        .add_systems(Startup, spawn)
        .add_systems(Update, capture_motion.in_set(ArenaFrame::Capture))
        .add_systems(
            Update,
            (controls, configure)
                .chain()
                .after(ArenaFrame::Input)
                .before(ArenaFrame::Tick),
        )
        .add_systems(
            Update,
            (stage_capture, present)
                .chain()
                .in_set(ArenaFrame::Present)
                .after(super::worm_capture::camera)
                .after(super::environment::present),
        )
        .add_systems(
            Update,
            water_tint.in_set(ArenaFrame::Present).after(present),
        );
}

/// Typed comparison facts persisted alongside ordinary arena capture state.
pub(super) fn snapshot(settings: &WaterLabSettings, frame: &WaterLabFrame) -> serde_json::Value {
    serde_json::json!({"enabled":frame.enabled,"wave":format!("{:?}",settings.wave),"style":format!("{:?}",settings.style),"wind":format!("{:?}",settings.wind),"glider_wind_scale":settings.glider_wind_scale,"wave_phase":settings.phase(frame.seconds),"frozen":settings.frozen_phase.is_some(),"whole_voxel_height":0.4,"maximum_columns":3283})
}

fn capture_motion(
    mut commands: Commands,
    state: Res<ViewState>,
    settings: Res<WaterLabSettings>,
    frame: Res<WaterLabFrame>,
    session: Res<ArenaSession>,
    cameras: Query<&Transform, With<ArenaCamera>>,
) {
    if !frame.enabled
        || !state.capture_view.starts_with("water-lab-motion")
        || !(30..=122).contains(&state.frames)
        || !(state.frames - 30).is_multiple_of(4)
    {
        return;
    }
    let (Some(base), Some(target)) = (&state.capture, state.image.clone()) else {
        return;
    };
    let index = (state.frames - 30) / 4;
    let path = base.with_file_name(format!(
        "{}-{index:02}.png",
        base.file_stem().unwrap_or_default().to_string_lossy()
    ));
    let receipt = serde_json::json!({"lab":snapshot(&settings,&frame),"frame":state.frames,"tick":session.tick,"camera":cameras.single().ok().map(|pose|pose.translation.to_array()),"player_feet":session.actors.first().map(|actor|actor.feet.to_array()),"evidence":"CONTINUOUS_WINDOWLESS_SEQUENCE; user control feel pending"});
    commands.spawn(Screenshot::image(target)).observe(
        move |captured: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| {
            let result = (|| -> Result<(), String> {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                }
                captured
                    .image
                    .clone()
                    .try_into_dynamic()
                    .map_err(|error| error.to_string())?
                    .save(&path)
                    .map_err(|error| error.to_string())?;
                std::fs::write(
                    path.with_extension("json"),
                    serde_json::to_string_pretty(&receipt).map_err(|error| error.to_string())?,
                )
                .map_err(|error| error.to_string())?;
                Ok(())
            })();
            match result {
                Ok(()) if index == 23 => {
                    info!("Water lab motion sequence complete: {}", path.display());
                    exit.write(AppExit::Success);
                }
                Ok(()) => {}
                Err(error) => {
                    error!("Water lab motion capture failed: {error}");
                    exit.write(AppExit::error());
                }
            }
        },
    );
}

fn water_tint(
    frame: Res<WaterLabFrame>,
    view: Res<ArenaTerrainView>,
    geometry: Res<ArenaVoxelGeometry>,
    environment: Option<Res<OceanEnvironmentView>>,
    mut sky: ResMut<hex_world::battle_sky::BattleSkyFrame>,
    mut cameras: Query<(&Transform, &mut DistanceFog), With<ArenaCamera>>,
    mut overlays: Query<
        (&mut Node, &mut BackgroundColor),
        With<super::environment::UnderwaterTint>,
    >,
) {
    if !frame.enabled {
        return;
    }
    let mut underwater = false;
    for (camera, mut fog) in &mut cameras {
        let coord = hex_core::HexCoord::from_world(camera.translation);
        if let Some(span) = view.liquids.iter().find(|span| span.bottom.coord == coord) {
            let column = hex_core::ocean::OceanWaterColumn {
                mean_height: geometry.top(hex_core::TilePos::new(coord, span.top_level)),
                bed_height: geometry.top(span.bottom) - geometry.level_height,
                water_id: span.substance,
            };
            if let Some(surface) = environment.as_ref().and_then(|env| {
                env.sampler.surface_at(
                    Vec2::new(camera.translation.x, camera.translation.z),
                    frame.seconds,
                    column,
                )
            }) {
                underwater = camera.translation.y < surface.height
                    && camera.translation.y > surface.bed_height;
            }
        }
        fog.color = if underwater {
            Color::srgb(0.04, 0.28, 0.35)
        } else {
            Color::srgb(0.62, 0.72, 0.82)
        };
        fog.falloff = FogFalloff::Exponential {
            density: if underwater { 0.10 } else { 0.0003 },
        };
    }
    sky.underwater_color = underwater.then_some(Vec3::new(0.04, 0.28, 0.35));
    for (mut node, mut color) in &mut overlays {
        node.display = if underwater {
            Display::Flex
        } else {
            Display::None
        };
        color.0 = Color::srgba(0.02, 0.20, 0.27, 0.18);
    }
}

fn spawn(mut commands: Commands) {
    commands
        .spawn((
            Panel,
            Node {
                position_type: PositionType::Absolute,
                right: px(18),
                top: px(75),
                width: px(325),
                padding: UiRect::all(px(14)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.025, 0.055, 0.075, 0.92)),
            GlobalZIndex(30),
            Pickable::IGNORE,
        ))
        .with_children(|parent| {
            parent.spawn((
                Details,
                Text::new(""),
                TextFont {
                    font_size: FontSize::Px(16.0),
                    ..default()
                },
                TextColor(Color::srgb(0.85, 0.95, 0.96)),
            ));
        });
}

#[expect(
    clippy::too_many_arguments,
    reason = "Lab-only input joins normal session controls with explicit comparison settings and reset anchors."
)]
fn controls(
    keys: Res<ButtonInput<KeyCode>>,
    windows: Query<&Window>,
    selection: Res<ArenaSelection>,
    terrain: Res<ArenaTerrainView>,
    geometry: Res<ArenaVoxelGeometry>,
    mut settings: ResMut<WaterLabSettings>,
    mut ui: ResMut<LabUi>,
    mut state: ResMut<ViewState>,
    mut session: ResMut<ArenaSession>,
    mut input: ResMut<ArenaInput>,
) {
    if selection.map != ArenaMap::WaterLab || windows.iter().any(|window| !window.focused) {
        return;
    }
    if keys.just_pressed(KeyCode::F1) {
        ui.visible = !ui.visible;
    }
    if keys.just_pressed(KeyCode::F2) {
        settings.wave = match settings.wave {
            LabWave::Flat => LabWave::Regular,
            LabWave::Regular => LabWave::Swell,
            LabWave::Swell => LabWave::Crossing,
            LabWave::Crossing => LabWave::Flat,
        };
    }
    if keys.just_pressed(KeyCode::F3) {
        settings.style = match settings.style {
            LabStyle::Depth => LabStyle::Crests,
            LabStyle::Crests => LabStyle::Patterns,
            LabStyle::Patterns => LabStyle::Depth,
        };
    }
    if keys.just_pressed(KeyCode::F4) {
        settings.wind = match settings.wind {
            LabWind::Calm => LabWind::Steady,
            LabWind::Steady => LabWind::Strong,
            LabWind::Strong => LabWind::Gusts,
            LabWind::Gusts => LabWind::Turning,
            LabWind::Turning => LabWind::Shelter,
            LabWind::Shelter => LabWind::Calm,
        };
    }
    if keys.just_pressed(KeyCode::F5) {
        settings.glider_wind_scale = if settings.glider_wind_scale > 0.9 {
            0.65
        } else if settings.glider_wind_scale > 0.5 {
            0.45
        } else {
            1.0
        };
    }
    let phase = session.ocean_time().phase_seconds();
    if keys.just_pressed(KeyCode::F6) {
        if let Some(frozen) = settings.frozen_phase.take() {
            settings.phase_origin = (phase - frozen).rem_euclid(900.0);
        } else {
            settings.frozen_phase = Some(settings.phase(phase));
        }
    }
    if keys.just_pressed(KeyCode::F7) {
        settings.phase_origin = phase;
        if settings.frozen_phase.is_some() {
            settings.frozen_phase = Some(0.0);
        }
    }
    let reset = [
        (KeyCode::F8, LabStart::Shore),
        (KeyCode::F9, LabStart::Swim),
        (KeyCode::F10, LabStart::Boat),
        (KeyCode::F11, LabStart::Glider),
    ]
    .into_iter()
    .find(|(key, _)| keys.just_pressed(*key))
    .map(|(_, start)| start);
    let outside = session.actors.first().is_some_and(|actor| {
        !hex_map::water_lab::contains(hex_core::HexCoord::from_world(actor.feet))
            || actor.feet.y < -5.0
    });
    if let Some(start) = reset.or(outside.then_some(LabStart::Shore)) {
        if session.reset_water_lab_pose(start, &terrain, *geometry) {
            state.yaw = if start == LabStart::Glider {
                0.0
            } else {
                -std::f32::consts::FRAC_PI_2
            };
            state.pitch = 0.0;
            state.step_offset = 0.0;
            let heading = if start == LabStart::Glider {
                Vec3::NEG_Z
            } else {
                Vec3::X
            };
            input.human = ActorIntent {
                aim: heading,
                glider_look: heading,
                ..default()
            };
        }
    }
}

fn configure(
    mut commands: Commands,
    selection: Res<ArenaSelection>,
    settings: Res<WaterLabSettings>,
    geometry: Res<ArenaVoxelGeometry>,
    environment: Option<Res<OceanEnvironmentView>>,
    mut previous: Local<Option<WaterLabSettings>>,
) {
    if selection.map != ArenaMap::WaterLab {
        *previous = None;
        return;
    }
    if *previous == Some(*settings)
        && environment
            .as_ref()
            .is_some_and(|env| env.package_fingerprint == WATER_LAB_ID)
    {
        return;
    }
    commands.insert_resource(OceanEnvironmentView {
        package_fingerprint: WATER_LAB_ID,
        sampler: Arc::new(LabSurface {
            settings: *settings,
            level_height: geometry.level_height,
        }),
        wind: OceanWindProfile::default(),
    });
    *previous = Some(*settings);
}

fn stage_capture(
    mut ui: ResMut<LabUi>,
    mut state: ResMut<ViewState>,
    mut session: ResMut<ArenaSession>,
    view: Res<ArenaTerrainView>,
    geometry: Res<ArenaVoxelGeometry>,
) {
    if view.selection.map != ArenaMap::WaterLab || state.capture.is_none() || ui.capture_staged {
        return;
    }
    let start = if state.capture_view.contains("boat") {
        LabStart::Boat
    } else if state.capture_view.contains("swim") {
        LabStart::Swim
    } else if state.capture_view.contains("glider") {
        LabStart::Glider
    } else {
        LabStart::Shore
    };
    if session.reset_water_lab_pose(start, &view, *geometry) {
        state.yaw = if start == LabStart::Glider {
            0.0
        } else {
            -std::f32::consts::FRAC_PI_2
        };
        state.pitch = -0.12;
        state.third_person = state.capture_view.ends_with("third");
        ui.capture_staged = true;
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "Lab presentation projects current authoritative state and positions a review-only camera."
)]
fn present(
    selection: Res<ArenaSelection>,
    settings: Res<WaterLabSettings>,
    time: Res<OceanSimulationTime>,
    state: Res<ViewState>,
    ui: Res<LabUi>,
    session: Res<ArenaSession>,
    mut frame: ResMut<WaterLabFrame>,
    mut panels: Query<&mut Node, With<Panel>>,
    mut labels: Query<&mut Text, With<Details>>,
    mut cameras: Query<&mut Transform, With<ArenaCamera>>,
    environment: Option<Res<OceanEnvironmentView>>,
) {
    frame.enabled = selection.map == ArenaMap::WaterLab;
    frame.seconds = time.phase_seconds();
    for mut panel in &mut panels {
        panel.display = if frame.enabled && ui.visible {
            Display::Flex
        } else {
            Display::None
        };
    }
    if !frame.enabled {
        return;
    }
    let player = session.actors.first();
    let wind = environment.as_ref().map_or(Vec2::ZERO, |env| {
        env.wind_at(player.map_or(Vec3::ZERO, |actor| actor.feet), *time)
    });
    let speed = player.map_or(0.0, |actor| {
        actor
            .glider()
            .map_or(0.0, |flight| flight.velocity.length())
    });
    for mut label in &mut labels {
        label.0 = format!("WATER LAB   ·   seven small biomes\n\nF2   Waves: {:?}\nF3   Color: {:?}\nF4   Wind: {:?} · {:.1} u/s\nF5   Glider wind: {:.0}%\nF6   Waves: {}\nF7   Restart wave phase\n\nF8   Beach start\nF9   Swimming start\nF10 Boat start\nF11 Glider start\n\nWASD move · B boat · G glider\nF free flight · C camera\nTab pause · F1 hide this panel\n\nGlider speed {:.1} u/s",settings.wave,settings.style,settings.wind,wind.length(),settings.glider_wind_scale * 100.0,if settings.frozen_phase.is_some() { "Frozen" } else { "Running" },speed);
    }
    if state.capture.is_some()
        && state.capture_view.starts_with("water-lab-")
        && !state.capture_view.ends_with("first")
        && !state.capture_view.ends_with("third")
    {
        let (position, target) = match state.capture_view.as_str() {
            view if view.starts_with("water-lab-motion") => {
                let angle = frame.seconds * if view.ends_with("reverse") { -0.6 } else { 0.6 };
                (
                    Vec3::new(6.0 + 28.0 * angle.cos(), 15.0, 28.0 * angle.sin()),
                    Vec3::new(6.0, 8.0, 0.0),
                )
            }
            "water-lab-overview" => (Vec3::new(0.0, 135.0, 55.0), Vec3::new(0.0, 4.0, 0.0)),
            "water-lab-reverse" => (Vec3::new(32.0, 18.0, -20.0), Vec3::new(8.0, 8.0, -5.0)),
            "water-lab-channel" => (Vec3::new(22.0, 14.0, 9.0), Vec3::new(0.0, 8.0, 3.0)),
            _ => (Vec3::new(28.0, 16.0, 21.0), Vec3::new(4.0, 8.0, 0.0)),
        };
        for mut camera in &mut cameras {
            *camera = Transform::from_translation(position).looking_at(target, Vec3::Y);
        }
    }
}
