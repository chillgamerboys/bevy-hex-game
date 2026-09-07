//! Native, default-off composition for the isolated spell-combat experiment.

mod hud;
mod presentation;
#[cfg(test)]
mod tests;

use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::window::{CursorGrabMode, CursorMoved, CursorOptions, PrimaryWindow};
use bevy::winit::WinitPlugin;
use hex_arena::{ActorIntent, ArenaInput, ArenaSession, ArenaTuning, Spell};
use hex_core::arena::{ArenaReset, ArenaSystems, ArenaTerrainView, ArenaTick};
use std::path::PathBuf;

const WIDTH: u32 = 1600;
const HEIGHT: u32 = 900;
const FIXED_SECONDS: f64 = 1.0 / 120.0;

#[derive(Resource)]
struct ViewState {
    paused: bool,
    third_person: bool,
    yaw: f32,
    pitch: f32,
    initialized: bool,
    previews: [bool; 2],
    suppress_click: bool,
    capture: Option<PathBuf>,
    capture_view: String,
    image: Option<Handle<Image>>,
    frames: u32,
    requested: bool,
    accumulator: f64,
    reset_seen: u64,
    frame_times: Vec<f64>,
    step_offset: f32,
    tick_times: Vec<(u64, bool, f64)>,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            paused: false,
            third_person: false,
            yaw: 0.0,
            pitch: 0.0,
            initialized: false,
            previews: [true, false],
            suppress_click: true,
            capture: std::env::var_os("HEX_ARENA_CAPTURE").map(PathBuf::from),
            capture_view: std::env::var("HEX_ARENA_VIEW").unwrap_or_else(|_| "first".into()),
            image: None,
            frames: 0,
            requested: false,
            accumulator: 0.0,
            reset_seen: 0,
            frame_times: Vec::new(),
            step_offset: 0.0,
            tick_times: Vec::new(),
        }
    }
}

impl ViewState {
    fn external_camera(&self) -> bool {
        self.capture.is_some()
            && !matches!(self.capture_view.as_str(), "first" | "third" | "tuning")
            && !self.capture_view.ends_with("-first")
            && !self.capture_view.ends_with("-third")
    }
}

#[derive(Component)]
struct ArenaCamera;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ArenaFrame {
    Input,
    Tick,
    Present,
    Capture,
}

/// Builds a separate arena application without installing tactical gameplay plugins.
pub fn run() -> AppExit {
    let state = ViewState::default();
    let capture = state.capture.is_some();
    let mut app = App::new();
    let plugins = DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "Hex — Spell Arena".into(),
            resolution: bevy::window::WindowResolution::new(WIDTH, HEIGHT)
                .with_scale_factor_override(1.0),
            visible: !capture,
            focused: !capture,
            present_mode: bevy::window::PresentMode::AutoVsync,
            ..default()
        }),
        ..default()
    });
    if capture {
        app.add_plugins(plugins.disable::<WinitPlugin>())
            .add_plugins(bevy::app::ScheduleRunnerPlugin::run_loop(
                std::time::Duration::from_secs_f64(1.0 / 60.0),
            ));
    } else {
        app.add_plugins(plugins);
    }
    app.insert_resource(state)
        .insert_resource(ClearColor(Color::srgb(0.10, 0.16, 0.22)))
        .insert_resource(GlobalAmbientLight {
            color: Color::srgb(0.77, 0.85, 1.0),
            brightness: 420.0,
            ..default()
        })
        .init_schedule(ArenaTick)
        .configure_sets(
            ArenaTick,
            (
                ArenaSystems::ApplyTerrain,
                ArenaSystems::PublishTerrain,
                ArenaSystems::Simulate,
            )
                .chain(),
        )
        .configure_sets(
            Update,
            (
                ArenaFrame::Input,
                ArenaFrame::Tick,
                ArenaFrame::Present,
                ArenaFrame::Capture,
            )
                .chain(),
        )
        .add_plugins((hex_map::arena::plugin, hex_arena::plugin))
        .add_systems(
            Startup,
            (setup, hud::setup, presentation::setup_effects)
                .chain()
                .after(ArenaSystems::PublishTerrain),
        )
        .add_systems(
            Update,
            (input, hud::buttons).chain().in_set(ArenaFrame::Input),
        )
        .add_systems(Update, drive_simulation.in_set(ArenaFrame::Tick))
        .add_systems(
            Update,
            (
                presentation::actors,
                presentation::camera,
                presentation::effects,
                presentation::solid_effects,
                hud::update,
            )
                .chain()
                .in_set(ArenaFrame::Present),
        )
        .add_systems(Update, capture_frame.in_set(ArenaFrame::Capture));
    app.run()
}

fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut state: ResMut<ViewState>,
    mut tuning: ResMut<ArenaTuning>,
) {
    let config = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/config/arena.ron");
    match std::fs::read_to_string(&config)
        .map_err(|error| error.to_string())
        .and_then(|source| ron::from_str::<ArenaTuning>(&source).map_err(|error| error.to_string()))
    {
        Ok(value) => match value.validate() {
            Ok(()) => *tuning = value,
            Err(error) => warn!("Arena settings rejected; using defaults: {error}"),
        },
        Err(error) => warn!("Arena settings unavailable; using defaults: {error}"),
    }
    if state.capture.is_some() {
        let size = if state.capture_view.ends_with("-compact") {
            0
        } else if state.capture_view.ends_with("-large") {
            2
        } else {
            1
        };
        tuning.shield_size = size;
        tuning.fireball_size = size;
        tuning.blast_size = size;
        state.third_person =
            state.capture_view == "third" || state.capture_view.ends_with("-third");
    }
    let target = if state.capture.is_some() {
        let handle = images.add(Image::new_target_texture(
            WIDTH,
            HEIGHT,
            bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
            None,
        ));
        state.image = Some(handle.clone());
        RenderTarget::Image(handle.into())
    } else {
        RenderTarget::default()
    };
    commands.spawn((
        Camera3d::default(),
        IsDefaultUiCamera,
        Camera {
            clear_color: ClearColorConfig::Default,
            ..default()
        },
        target,
        Projection::Perspective(PerspectiveProjection {
            fov: 75.0_f32.to_radians(),
            near: 0.035,
            far: 180.0,
            ..default()
        }),
        Transform::from_xyz(0.0, 6.0, 15.0).looking_at(Vec3::new(0.0, 3.0, 0.0), Vec3::Y),
        bevy::core_pipeline::tonemapping::Tonemapping::TonyMcMapface,
        Msaa::Sample4,
        ArenaCamera,
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 18_000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(-15.0, 30.0, 18.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    info!("Spell arena initialized; fixed step120Hz, local-only, radius12. Controls WASD mouse Space Shift 1/2/3 LMB C T Escape R.");
}

fn aim(state: &ViewState) -> Vec3 {
    Quat::from_euler(EulerRot::YXZ, state.yaw, state.pitch, 0.0) * Vec3::NEG_Z
}

fn input(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut moved: MessageReader<CursorMoved>,
    mut windows: Query<(&mut Window, &mut CursorOptions), With<PrimaryWindow>>,
    mut state: ResMut<ViewState>,
    session: Res<ArenaSession>,
    mut intent: ResMut<ArenaInput>,
    mut reset: ResMut<ArenaReset>,
) {
    if state.capture.is_some() {
        moved.clear();
        return;
    }
    let Ok((mut window, mut cursor)) = windows.single_mut() else {
        moved.clear();
        return;
    };
    if !state.initialized {
        if let Some(actor) = session.actors.first() {
            state.yaw = (-actor.aim.x).atan2(-actor.aim.z);
            state.pitch = actor.aim.y.clamp(-1.0, 1.0).asin();
            state.initialized = true;
        }
    }
    if !window.focused {
        state.paused = true;
        state.suppress_click = true;
    }
    if keys.just_pressed(KeyCode::Escape) && window.focused {
        state.paused = !state.paused;
        state.suppress_click = true;
    }
    if keys.just_pressed(KeyCode::KeyR) {
        reset.generation = reset.generation.saturating_add(1);
        state.paused = false;
        state.initialized = false;
        state.suppress_click = true;
    }
    if keys.just_pressed(KeyCode::KeyC) {
        state.third_person = !state.third_person;
    }
    if keys.just_pressed(KeyCode::KeyT) {
        let spell = session
            .actors
            .first()
            .map(|actor| actor.selected)
            .unwrap_or(Spell::Shield);
        let slot = match spell {
            Spell::Shield => Some(0),
            Spell::Fireball => Some(1),
            Spell::AreaBlast => None,
        };
        if let Some(enabled) = slot.and_then(|slot| state.previews.get_mut(slot)) {
            *enabled = !*enabled;
        }
    }
    let active = window.focused && !state.paused && session.outcome.is_none();
    cursor.visible = !active;
    // CursorMoved plus recentering preserves the repository's WSLg held-button path.
    // Ignore warp-to-center events, so recentering never contributes camera rotation.
    cursor.grab_mode = CursorGrabMode::None;
    let center = Vec2::new(window.width() * 0.5, window.height() * 0.5);
    let mut displacement = Vec2::ZERO;
    for event in moved.read() {
        if event.position.distance_squared(center) > 0.01 {
            displacement = event.position - center;
        }
    }
    if active {
        if !state.suppress_click {
            state.yaw -= displacement.x * 0.0025;
            state.pitch = (state.pitch - displacement.y * 0.0025).clamp(-1.48, 1.48);
        }
        window.set_cursor_position(Some(center));
    }
    if !active {
        intent.human = ActorIntent::default();
        return;
    }
    let direction = aim(&state);
    intent.human.aim = session.actors.first().map_or(direction, |actor| {
        let eye = actor.eye();
        let camera = camera_origin(&session, &state, eye, direction);
        session.aim_from_camera(actor.id, camera, direction)
    });
    let axis = |positive, negative| {
        f32::from(u8::from(keys.pressed(positive))) - f32::from(u8::from(keys.pressed(negative)))
    };
    intent.human.movement = Vec2::new(
        axis(KeyCode::KeyD, KeyCode::KeyA),
        axis(KeyCode::KeyW, KeyCode::KeyS),
    );
    intent.human.run = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    intent.human.jump |= keys.just_pressed(KeyCode::Space);
    for (key, spell) in [
        (KeyCode::Digit1, Spell::Shield),
        (KeyCode::Digit2, Spell::Fireball),
        (KeyCode::Digit3, Spell::AreaBlast),
    ] {
        if keys.just_pressed(key) {
            intent.human.selected = Some(spell);
        }
    }
    intent.human.cast |= mouse.just_pressed(MouseButton::Left) && !state.suppress_click;
    if !mouse.pressed(MouseButton::Left) {
        state.suppress_click = false;
    }
}

fn drive_simulation(world: &mut World) {
    let delta = world.resource::<Time>().delta_secs_f64();
    let delta_f32 = world.resource::<Time>().delta_secs();
    let generation = world.resource::<ArenaReset>().generation;
    let capture = world.resource::<ViewState>().capture.is_some();
    let (steps, frame, view) = {
        let mut state = world.resource_mut::<ViewState>();
        state.frames = state.frames.saturating_add(1);
        if state.frame_times.len() < 36_000 {
            state.frame_times.push(delta * 1000.0);
        }
        let changed = generation != state.reset_seen;
        state.reset_seen = generation;
        if changed {
            state.step_offset = 0.0;
        } else if !state.paused {
            state.step_offset *= (-12.0 * delta_f32.min(0.1)).exp();
        }
        if state.paused && !changed {
            state.accumulator = 0.0;
            (0, state.frames, state.capture_view.clone())
        } else {
            state.accumulator += if capture { 1.0 / 60.0 } else { delta.min(0.1) };
            let mut steps = 0;
            while state.accumulator + 1.0e-12 >= FIXED_SECONDS && steps < 12 {
                state.accumulator = (state.accumulator - FIXED_SECONDS).max(0.0);
                steps += 1;
            }
            if changed && steps == 0 {
                steps = 1;
            }
            (steps, state.frames, state.capture_view.clone())
        }
    };
    if capture {
        world.resource_mut::<ArenaSession>().bot_enabled = false;
        if frame > 2 {
            let direction = world
                .resource::<ArenaSession>()
                .actors
                .first()
                .map(|a| a.aim)
                .unwrap_or(Vec3::X);
            let mut input = world.resource_mut::<ArenaInput>();
            input.human.aim = if view.starts_with("shield") || view.starts_with("fireball") {
                (Vec3::new(direction.x, -0.2, direction.z)).normalize_or_zero()
            } else {
                direction
            };
            if frame == 6 && view.starts_with("shield") {
                input.human.selected = Some(Spell::Shield);
                input.human.cast = !view.contains("preview");
            }
            if frame == 48 && view.starts_with("fireball") {
                input.human.selected = Some(Spell::Fireball);
                input.human.cast = true;
            }
            if frame == 48 && view.starts_with("blast") {
                input.human.selected = Some(Spell::AreaBlast);
                input.human.cast = true;
            }
        }
        if view == "tuning" && frame == 4 {
            world.resource_mut::<ViewState>().paused = true;
        }
    }
    for _ in 0..steps {
        let before = world.resource::<ArenaTerrainView>().revision;
        let started = std::time::Instant::now();
        world.run_schedule(ArenaTick);
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
        let changed = before != world.resource::<ArenaTerrainView>().revision;
        let tick = world.resource::<ArenaSession>().tick;
        {
            let mut state = world.resource_mut::<ViewState>();
            if state.tick_times.len() < 36_000 {
                state.tick_times.push((tick, changed, elapsed));
            }
        }
        let rise = world
            .resource::<ArenaSession>()
            .actors
            .first()
            .map_or(0.0, |a| a.step_rise_this_tick());
        if rise > 0.0 {
            let mut state = world.resource_mut::<ViewState>();
            state.step_offset = (state.step_offset - rise).max(-0.8);
        }
    }
}

fn camera_origin(session: &ArenaSession, state: &ViewState, eye: Vec3, direction: Vec3) -> Vec3 {
    let smoothed_eye = eye + Vec3::Y * state.step_offset;
    let desired = if state.third_person {
        smoothed_eye - direction * 1.25
            + Vec3::Y * 0.38
            + direction.cross(Vec3::Y).normalize_or_zero() * 0.28
    } else {
        smoothed_eye
    };
    session.camera_position(eye, desired)
}

fn capture_frame(
    mut commands: Commands,
    mut state: ResMut<ViewState>,
    session: Res<ArenaSession>,
    view: Res<ArenaTerrainView>,
) {
    let Some(path) = state.capture.clone() else {
        return;
    };
    let frame =
        if state.capture_view.starts_with("blast") || state.capture_view.starts_with("fireball") {
            57
        } else {
            90
        };
    if state.frames < frame || state.requested {
        return;
    }
    let Some(target) = state.image.clone() else {
        return;
    };
    state.requested = true;
    let receipt = serde_json::json!({"view":state.capture_view,"frame":state.frames,"tick":session.tick,"terrain_revision":view.revision,"voxels":view.voxels.len(),"actors":session.actors.iter().map(|a|serde_json::json!({"id":a.id,"hp":a.hp,"feet":[a.feet.x,a.feet.y,a.feet.z],"cooldowns":a.cooldowns})).collect::<Vec<_>>(),"notice":session.notice,"shields_raised":session.shields_raised,"terrain_outcomes":session.terrain_outcomes,"effects":session.effects.iter().map(|e|serde_json::json!({"spell":e.kind,"radius":e.radius,"age":e.age})).collect::<Vec<_>>(),"frame_ms":state.frame_times,"tick_samples":state.tick_times.iter().map(|(tick,changed,ms)|serde_json::json!({"tick":tick,"terrain_changed":changed,"cpu_ms":ms})).collect::<Vec<_>>(),"width":WIDTH,"height":HEIGHT,"evidence":"STATIC_CAPTURE_UNREVIEWED; logic is recorded separately; native feel pending"});
    commands.spawn(Screenshot::image(target)).observe(
        move |captured: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| {
            let result = (|| -> Result<(), String> {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                captured
                    .image
                    .clone()
                    .try_into_dynamic()
                    .map_err(|e| e.to_string())?
                    .save(&path)
                    .map_err(|e| e.to_string())?;
                std::fs::write(
                    path.with_extension("json"),
                    serde_json::to_string_pretty(&receipt).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
                Ok(())
            })();
            match result {
                Ok(()) => {
                    info!("Arena capture complete: {}", path.display());
                    exit.write(AppExit::Success);
                }
                Err(error) => {
                    error!("Arena capture failed: {error}");
                    exit.write(AppExit::error());
                }
            }
        },
    );
}
