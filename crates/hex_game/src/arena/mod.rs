//! Native composition for isolated Battle Mode, launched through the menu or `--arena`.

mod encounter;
#[cfg(feature = "test-support")]
pub use encounter::{stress_target_pose, STRESS_VISIT_TICKS};
mod golem;
mod hud;
mod presentation;
mod spectator;
#[cfg(test)]
mod tests;
mod wisp;
mod worm;
mod worm_capture;

use bevy::camera::RenderTarget;
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::window::{CursorGrabMode, CursorMoved, CursorOptions, PrimaryWindow};
use bevy::winit::WinitPlugin;
use hex_arena::{ActorIntent, ArenaInput, ArenaSession, ArenaTuning, Spell};
use hex_core::arena::{
    ArenaEncounter, ArenaMap, ArenaReset, ArenaSelection, ArenaSystems, ArenaTerrainView,
    ArenaTick, ArenaVoxelGeometry,
};
use std::path::PathBuf;

const WIDTH: u32 = 1600;
const HEIGHT: u32 = 900;
const FIXED_SECONDS: f64 = 1.0 / 120.0;

fn launch_selection(map: Option<&str>, encounter: Option<&str>) -> Result<ArenaSelection, String> {
    let map = match map.unwrap_or("fort") {
        "duel" => ArenaMap::Duel,
        "fort" => ArenaMap::Fort,
        "seven-regions" => ArenaMap::SevenRegions,
        value => return Err(format!("Unknown arena map: {value}")),
    };
    let encounter = match encounter.unwrap_or(if map == ArenaMap::Duel {
        "shadow"
    } else {
        "dragon"
    }) {
        "dragon" => ArenaEncounter::Dragon,
        "goblins" => ArenaEncounter::Goblins,
        "shaman-party" => ArenaEncounter::ShamanParty,
        "shadow" => ArenaEncounter::Shadow,
        "golem" | "goblin" | "wisp" | "wisps-2" | "wisps-4" | "wisps-8" | "wisps-12" | "worm"
            if matches!(map, ArenaMap::Fort | ArenaMap::Duel) =>
        {
            ArenaEncounter::Dragon
        }
        "golem" | "goblin" | "wisp" | "wisps-2" | "wisps-4" | "wisps-8" | "wisps-12" | "worm" => {
            return Err("Creature player overrides require Fort or Duel.".into());
        }
        value => return Err(format!("Unknown arena encounter: {value}")),
    };
    Ok(ArenaSelection { map, encounter })
}

fn apply_player_recipe(
    setup: &mut hex_arena::ArenaBattleSetup,
    selection: ArenaSelection,
    encounter: Option<&str>,
) -> Result<(), String> {
    if let Some(recipe) = encounter.and_then(hex_arena::BattlePreset::from_slug) {
        let extended = !hex_arena::BattlePreset::ORIGINAL.contains(&recipe);
        if extended && setup.control != hex_arena::ArenaControl::Player {
            return Err("Spectator creatures use the team roster options.".into());
        }
        if setup.control == hex_arena::ArenaControl::Player
            && (extended
                || (selection.map == ArenaMap::Duel && recipe != hex_arena::BattlePreset::Shadow))
        {
            setup.player_recipe = Some(recipe);
        }
    }
    setup
        .validate_for(selection.map)
        .map_err(|error| error.to_string())
}

fn encounter_preset(encounter: ArenaEncounter) -> hex_arena::BattlePreset {
    use hex_arena::BattlePreset;
    match encounter {
        ArenaEncounter::Dragon => BattlePreset::Dragon,
        ArenaEncounter::Goblins => BattlePreset::Goblins,
        ArenaEncounter::ShamanParty => BattlePreset::ShamanParty,
        ArenaEncounter::Shadow => BattlePreset::Shadow,
    }
}

fn player_preset(
    selection: ArenaSelection,
    setup: &hex_arena::ArenaBattleSetup,
) -> hex_arena::BattlePreset {
    setup.player_recipe.unwrap_or_else(|| {
        if selection.map == ArenaMap::Duel {
            hex_arena::BattlePreset::Shadow
        } else {
            encounter_preset(selection.encounter)
        }
    })
}

fn choose_player_preset(
    selection: &mut ArenaSelection,
    setup: &mut hex_arena::ArenaBattleSetup,
    preset: hex_arena::BattlePreset,
) {
    use hex_arena::BattlePreset;
    selection.encounter = match preset {
        BattlePreset::Shadow => ArenaEncounter::Shadow,
        BattlePreset::Goblins => ArenaEncounter::Goblins,
        BattlePreset::ShamanParty => ArenaEncounter::ShamanParty,
        _ => ArenaEncounter::Dragon,
    };
    setup.player_recipe = if !BattlePreset::ORIGINAL.contains(&preset)
        || (selection.map == ArenaMap::Duel && preset != BattlePreset::Shadow)
    {
        Some(preset)
    } else {
        None
    };
}

fn map_name(map: ArenaMap) -> &'static str {
    match map {
        ArenaMap::Duel => "Duel",
        ArenaMap::Fort => "Fort",
        ArenaMap::SevenRegions => "Seven Regions",
    }
}

fn encounter_name(encounter: ArenaEncounter) -> &'static str {
    match encounter {
        ArenaEncounter::Dragon => "Dragon",
        ArenaEncounter::Goblins => "Goblins",
        ArenaEncounter::ShamanParty => "Shaman party",
        ArenaEncounter::Shadow => "Shadow",
    }
}

#[derive(Resource)]
struct ViewState {
    started: bool,
    paused: bool,
    third_person: bool,
    observer: spectator::ObserverCamera,
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
    simulation_frame_times: Vec<f64>,
    app_started_at: std::time::Instant,
    previous_frame_at: Option<std::time::Instant>,
    frame_wall_intervals: Vec<f64>,
    capture_ready_elapsed_ms: Option<f64>,
    step_offset: f32,
    tick_times: Vec<(u64, bool, f64)>,
    capture_inputs: Vec<(u32, ActorIntent)>,
    capture_fixture_voxels: Vec<hex_core::TilePos>,
    capture_focus: Option<String>,
    capture_event_frame: Option<u32>,
    capture_ready_frame: Option<u32>,
    capture_route_step: usize,
    capture_approach_frame: Option<u32>,
    capture_composition_frame: Option<u32>,
    capture_subjects: Vec<u8>,
    capture_observer_inputs: Vec<spectator::CameraSample>,
    capture_wisp_nominal_hp: Option<f32>,
    capture_wisp_config_loaded: bool,
    capture_wisp_ticks: Vec<wisp::StressTick>,
    capture_wisp_terminal: Option<wisp::TerminalPublication>,
    capture_stress_initialized: bool,
    capture_stress_steps: u32,
    capture_stress_ticks: Vec<encounter::StressTick>,
}

impl Default for ViewState {
    fn default() -> Self {
        let capture = std::env::var_os("HEX_ARENA_CAPTURE").map(PathBuf::from);
        let capture_view = std::env::var("HEX_ARENA_VIEW").unwrap_or_else(|_| "first".into());
        let started =
            capture.is_some() && !matches!(capture_view.as_str(), "start" | "observer-start");
        Self {
            started,
            paused: !started,
            third_person: false,
            observer: spectator::ObserverCamera::default(),
            yaw: 0.0,
            pitch: 0.0,
            initialized: false,
            previews: [true, false],
            suppress_click: true,
            capture,
            capture_view,
            image: None,
            frames: 0,
            requested: false,
            accumulator: 0.0,
            reset_seen: 0,
            frame_times: Vec::new(),
            simulation_frame_times: Vec::new(),
            app_started_at: std::time::Instant::now(),
            previous_frame_at: None,
            frame_wall_intervals: Vec::new(),
            capture_ready_elapsed_ms: None,
            step_offset: 0.0,
            tick_times: Vec::new(),
            capture_inputs: Vec::new(),
            capture_fixture_voxels: Vec::new(),
            capture_focus: std::env::var("HEX_ARENA_FOCUS").ok(),
            capture_event_frame: None,
            capture_ready_frame: None,
            capture_route_step: 0,
            capture_approach_frame: None,
            capture_composition_frame: None,
            capture_subjects: Vec::new(),
            capture_observer_inputs: Vec::new(),
            capture_wisp_nominal_hp: None,
            capture_wisp_config_loaded: false,
            capture_wisp_ticks: Vec::new(),
            capture_wisp_terminal: None,
            capture_stress_initialized: false,
            capture_stress_steps: 0,
            capture_stress_ticks: Vec::new(),
        }
    }
}

impl ViewState {
    fn record_frame_timing(&mut self, now: std::time::Instant, engine_delta: f64) {
        if self.frame_times.len() < 36_000 {
            self.frame_times.push(engine_delta * 1000.0);
        }
        if let Some(previous) = self.previous_frame_at.replace(now) {
            if self.frame_wall_intervals.len() < 36_000 {
                self.frame_wall_intervals
                    .push(now.saturating_duration_since(previous).as_secs_f64() * 1000.0);
            }
        }
    }

    fn begin_play(&mut self) {
        self.started = true;
        self.paused = false;
        self.suppress_click = true;
        self.accumulator = 0.0;
    }

    fn pause(&mut self) {
        self.paused = true;
        self.suppress_click = true;
        self.accumulator = 0.0;
    }

    fn prepare_round(&mut self) {
        self.started = false;
        self.initialized = false;
        self.observer.generation = None;
        self.capture_event_frame = None;
        self.capture_ready_frame = None;
        self.capture_route_step = 0;
        self.capture_approach_frame = None;
        self.capture_composition_frame = None;
        self.capture_subjects.clear();
        self.capture_observer_inputs.clear();
        self.capture_ready_elapsed_ms = None;
        self.capture_stress_initialized = false;
        self.capture_stress_steps = 0;
        self.capture_stress_ticks.clear();
        self.capture_wisp_ticks.clear();
        self.capture_wisp_terminal = None;
        self.pause();
    }

    fn external_camera(&self) -> bool {
        self.capture.is_some()
            && !matches!(
                self.capture_view.as_str(),
                "first" | "third" | "tuning" | "start"
            )
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
#[expect(
    clippy::print_stderr,
    reason = "invalid launch capabilities must be reported before Bevy logging or a window is initialized"
)]
pub fn run() -> AppExit {
    let state = ViewState::default();
    let capture = state.capture.is_some();
    let selection = match launch_selection(
        std::env::var("HEX_ARENA_MAP").ok().as_deref(),
        std::env::var("HEX_ARENA_ENCOUNTER").ok().as_deref(),
    ) {
        Ok(selection) => selection,
        Err(error) => {
            eprintln!("{error}");
            return AppExit::error();
        }
    };
    let control = std::env::var("HEX_ARENA_CONTROL").unwrap_or_else(|_| "player".into());
    if !matches!(control.as_str(), "player" | "spectator") {
        eprintln!("Unknown arena control: {control}");
        return AppExit::error();
    }
    let mut battle = match spectator::launch_setup(
        selection.map,
        control == "spectator",
        std::env::var("HEX_ARENA_TEAM_A").ok().as_deref(),
        std::env::var("HEX_ARENA_TEAM_B").ok().as_deref(),
        std::env::var("HEX_ARENA_BATTLE_SEED").ok().as_deref(),
        std::env::var("HEX_ARENA_BATTLE_TICK_LIMIT").ok().as_deref(),
    ) {
        Ok(setup) => setup,
        Err(error) => {
            eprintln!("{error}");
            return AppExit::error();
        }
    };
    if let Err(error) = apply_player_recipe(
        &mut battle,
        selection,
        std::env::var("HEX_ARENA_ENCOUNTER").ok().as_deref(),
    ) {
        eprintln!("{error}");
        return AppExit::error();
    }
    if let Err(error) =
        wisp::validate_stress_setup(capture, &state.capture_view, selection.map, &battle)
    {
        eprintln!("{error}");
        return AppExit::error();
    }
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
        if let Some(clock) = hex_map::LiquidVisualTime::frozen_at(0.0) {
            app.insert_resource(clock);
        }
        app.add_plugins(plugins.disable::<WinitPlugin>())
            .add_plugins(bevy::app::ScheduleRunnerPlugin::run_loop(
                std::time::Duration::from_secs_f64(1.0 / 60.0),
            ));
    } else {
        app.add_plugins(plugins);
    }
    app.init_resource::<worm_capture::Evidence>()
        .insert_resource(state)
        .insert_resource(selection)
        .insert_resource(battle)
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
        .add_plugins((
            hex_map::arena::plugin,
            hex_arena::plugin,
            hex_objects::plugin,
        ))
        .add_systems(
            Startup,
            (
                setup,
                hud::setup,
                presentation::setup_effects,
                encounter::setup,
                golem::setup,
                wisp::setup,
                worm::setup,
                wisp::configure_capture_lighting,
            )
                .chain()
                .after(ArenaSystems::PublishTerrain),
        )
        .add_systems(
            Update,
            (
                worm_capture::inject_reset_key,
                input,
                hud::buttons,
                sync_cursor,
            )
                .chain()
                .in_set(ArenaFrame::Input),
        )
        .add_systems(Update, drive_simulation.in_set(ArenaFrame::Tick))
        .add_systems(
            Update,
            (worm_capture::observe, worm_capture::progress)
                .chain()
                .after(ArenaFrame::Tick)
                .before(ArenaFrame::Present),
        )
        .add_systems(
            Update,
            (
                presentation::actors,
                worm::update_parts,
                presentation::camera,
                encounter::camera,
                spectator::camera,
                golem::capture_camera,
                wisp::capture_camera,
                worm::capture_camera,
                worm_capture::camera,
                presentation::effects,
                presentation::solid_effects,
                encounter::effects,
                hud::update,
                log_round,
            )
                .chain()
                .in_set(ArenaFrame::Present),
        )
        .add_systems(Update, capture_frame.in_set(ArenaFrame::Capture));
    app.run()
}

fn log_round(session: Res<ArenaSession>, mut logged: Local<bool>) {
    let finished = session.is_finished();
    if finished && !*logged {
        if let Some(battle) = session.battle_summary() {
            info!(battle=?battle, actors=?session.encounter_stats(), "Arena observer battle complete");
        } else if session.encounter_summary().enabled {
            info!(round = ?session.round_summary(), encounter = ?session.encounter_summary(), actors = ?session.encounter_stats(), "Arena encounter complete");
        } else {
            info!(summary = ?session.round_summary(), "Arena round complete");
        }
    }
    *logged = finished;
}

fn setup(
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut images: ResMut<Assets<Image>>,
    mut state: ResMut<ViewState>,
    mut tuning: ResMut<ArenaTuning>,
) {
    commands.insert_resource(hex_assets::GameAssets {
        hex_tile: assets.load(
            bevy::gltf::GltfAssetLabel::Primitive {
                mesh: 0,
                primitive: 0,
            }
            .from_asset("meshes/hex.glb"),
        ),
        player_pieces: [default(), default()],
    });
    // Use the same runtime asset root as meshes and fonts, including packaged games.
    let config =
        bevy::asset::io::file::FileAssetReader::get_base_path().join("assets/config/arena.ron");
    state.capture_wisp_config_loaded = match std::fs::read_to_string(&config)
        .map_err(|error| error.to_string())
        .and_then(|source| ron::from_str::<ArenaTuning>(&source).map_err(|error| error.to_string()))
    {
        Ok(value) => match value.validate() {
            Ok(()) => {
                *tuning = value;
                true
            }
            Err(error) => {
                warn!("Arena settings rejected; using defaults: {error}");
                false
            }
        },
        Err(error) => {
            warn!("Arena settings unavailable; using defaults: {error}");
            false
        }
    };
    state.capture_wisp_nominal_hp =
        wisp::configure_stress_tuning(state.capture.is_some(), &state.capture_view, &mut tuning);
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
            far: 480.0,
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
    info!(
        "Spell arena ready; Enter starts. Escape/Tab pauses and frees the mouse. Hold LMB to charge Shield/Fireball; release LMB to cast any spell. Controls WASD mouse Space Shift 1/2/3 C T R."
    );
}

fn aim(state: &ViewState) -> Vec3 {
    Quat::from_euler(EulerRot::YXZ, state.yaw, state.pitch, 0.0) * Vec3::NEG_Z
}

fn reset_from_input(
    state: &mut ViewState,
    session: &mut ArenaSession,
    intent: &mut ArenaInput,
    reset: &mut ArenaReset,
) {
    session.cancel_charges();
    intent.human = ActorIntent::default();
    reset.generation = reset.generation.saturating_add(1);
    state.prepare_round();
}

fn input(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut moved: MessageReader<CursorMoved>,
    mut wheel: MessageReader<MouseWheel>,
    time: Res<Time>,
    mut windows: Query<(&mut Window, &mut CursorOptions), With<PrimaryWindow>>,
    mut state: ResMut<ViewState>,
    mut session: ResMut<ArenaSession>,
    mut intent: ResMut<ArenaInput>,
    mut reset: ResMut<ArenaReset>,
) {
    if state.capture.is_some() {
        if state.capture_view == "encounter-worm-reset"
            && state.started
            && keys.just_pressed(KeyCode::KeyR)
        {
            reset_from_input(&mut state, &mut session, &mut intent, &mut reset);
        }
        moved.clear();
        wheel.clear();
        return;
    }
    let Ok((mut window, mut cursor)) = windows.single_mut() else {
        moved.clear();
        wheel.clear();
        state.pause();
        intent.human = ActorIntent::default();
        session.cancel_charges();
        return;
    };
    let observing = spectator::active(&session);
    if !state.initialized && !observing {
        if let Some(actor) = session
            .human_actor_id()
            .and_then(|id| session.actors.iter().find(|actor| actor.id == id))
        {
            state.yaw = (-actor.aim.x).atan2(-actor.aim.z);
            state.pitch = actor.aim.y.clamp(-1.0, 1.0).asin();
            state.initialized = true;
        }
    }
    if !window.focused {
        state.pause();
    }
    if window.focused && !state.started && keys.just_pressed(KeyCode::Enter) {
        session.cancel_charges();
        intent.human = ActorIntent::default();
        state.begin_play();
    }
    if window.focused
        && state.started
        && (keys.just_pressed(KeyCode::Escape) || keys.just_pressed(KeyCode::Tab))
    {
        session.cancel_charges();
        intent.human = ActorIntent::default();
        if state.paused && !session.is_finished() {
            state.begin_play();
        } else {
            state.pause();
        }
    }
    if window.focused && state.started && keys.just_pressed(KeyCode::KeyR) {
        reset_from_input(&mut state, &mut session, &mut intent, &mut reset);
    }
    if window.focused
        && state.started
        && !state.paused
        && !observing
        && keys.just_pressed(KeyCode::KeyC)
    {
        state.third_person = !state.third_person;
    }
    let active =
        window.focused && state.started && !state.paused && (observing || !session.is_finished());
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
        if !state.suppress_click && !observing {
            state.yaw -= displacement.x * 0.0025;
            state.pitch = (state.pitch - displacement.y * 0.0025).clamp(-1.48, 1.48);
        }
        window.set_cursor_position(Some(center));
    }
    let scroll = wheel
        .read()
        .map(|event| match event.unit {
            MouseScrollUnit::Line => event.y,
            MouseScrollUnit::Pixel => event.y / 40.0,
        })
        .sum::<f32>();
    if observing {
        intent.human = ActorIntent::default();
        if active && state.observer.generation.is_some() {
            if keys.just_pressed(KeyCode::KeyC) {
                state.observer.toggle_mode();
            }
            let axis = |positive, negative| {
                f32::from(u8::from(keys.pressed(positive)))
                    - f32::from(u8::from(keys.pressed(negative)))
            };
            let movement = Vec3::new(
                axis(KeyCode::KeyD, KeyCode::KeyA),
                axis(KeyCode::KeyE, KeyCode::KeyQ),
                axis(KeyCode::KeyW, KeyCode::KeyS),
            );
            let previous = state.observer.position;
            let look = if state.suppress_click {
                Vec2::ZERO
            } else {
                displacement
            };
            let fast = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
            let desired =
                state
                    .observer
                    .update_pose(look, movement, fast, scroll, time.delta_secs());
            state.observer.position = session.camera_position(previous, desired.translation);
        }
        if !active {
            state.suppress_click = true;
            session.cancel_charges();
        }
        if !mouse.pressed(MouseButton::Left) && active {
            state.suppress_click = false;
        }
        return;
    }
    if !active {
        state.suppress_click = true;
        session.cancel_charges();
        intent.human = ActorIntent::default();
        return;
    }
    let direction = aim(&state);
    let current_aim = session.actors.first().map_or(direction, |actor| {
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
            if session
                .actors
                .first()
                .is_some_and(|actor| actor.selected != spell)
            {
                // Selection cancels the old spell in authority. Discard old queued
                // edges here as well, including when this render frame has no tick.
                let existing_hold = intent.human.cast_pressed
                    || intent.human.cast_held
                    || session
                        .actors
                        .first()
                        .is_some_and(|actor| actor.charge().is_some());
                intent.human.cast_pressed = false;
                intent.human.cast_released = false;
                if existing_hold {
                    state.suppress_click = true;
                }
            }
            intent.human.selected = Some(spell);
        }
    }
    // A queued release owns its release-frame aim until physics consumes it.
    // Selection and menu cancellation clear the edge, restoring current aim.
    if window.focused && state.started && !state.paused && keys.just_pressed(KeyCode::KeyT) {
        let spell = intent
            .human
            .selected
            .or_else(|| session.actors.first().map(|actor| actor.selected))
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
    if !intent.human.cast_released {
        intent.human.aim = current_aim;
    }
    // A press and release can both arrive before the next 120 Hz tick. Preserve
    // both edges while replacing only the current held sample each render frame.
    intent.human.cast_pressed |= mouse.just_pressed(MouseButton::Left) && !state.suppress_click;
    intent.human.cast_released |= mouse.just_released(MouseButton::Left) && !state.suppress_click;
    intent.human.cast_held = mouse.pressed(MouseButton::Left) && !state.suppress_click;
    if !mouse.pressed(MouseButton::Left) {
        state.suppress_click = false;
    }
}

// UI buttons run after input. Synchronize the cursor again so Start/Resume take
// effect in this same frame, and menus never keep a hidden cursor for a frame.
fn sync_cursor(
    state: Res<ViewState>,
    session: Res<ArenaSession>,
    mut windows: Query<(&Window, &mut CursorOptions), With<PrimaryWindow>>,
) {
    for (window, mut cursor) in &mut windows {
        cursor.visible = !window.focused
            || !state.started
            || state.paused
            || (!spectator::active(&session) && session.is_finished());
        cursor.grab_mode = CursorGrabMode::None;
    }
}

fn drive_simulation(world: &mut World) {
    let frame_started = std::time::Instant::now();
    let tick_before_frame = world.resource::<ArenaSession>().tick;
    let delta = world.resource::<Time>().delta_secs_f64();
    let delta_f32 = world.resource::<Time>().delta_secs();
    let generation = world.resource::<ArenaReset>().generation;
    let reset_this_frame = world.resource::<ViewState>().reset_seen != generation;
    let capture = world.resource::<ViewState>().capture.is_some();
    let needs_initialization = world.resource::<ViewState>().frames == 0;
    let (steps, frame, view) = {
        let mut state = world.resource_mut::<ViewState>();
        state.frames = state.frames.saturating_add(1);
        state.record_frame_timing(frame_started, delta);
        let changed = generation != state.reset_seen;
        state.reset_seen = generation;
        if changed {
            state.step_offset = 0.0;
        } else if !state.paused {
            state.step_offset *= (-12.0 * delta_f32.min(0.1)).exp();
        }
        if state.capture_event_frame.is_some() || state.capture_composition_frame.is_some() {
            (0, state.frames, state.capture_view.clone())
        } else if state.paused || !state.started {
            state.accumulator = 0.0;
            (
                u32::from(changed || needs_initialization),
                state.frames,
                state.capture_view.clone(),
            )
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
        let observer = spectator::active(world.resource::<ArenaSession>());
        world.resource_mut::<ArenaSession>().bot_enabled = observer
            || view.starts_with("bot-combat-")
            || view.starts_with("encounter-") && view != "encounter-landmark";
        let direction = world
            .resource::<ArenaSession>()
            .actors
            .first()
            .map_or(Vec3::X, |actor| actor.aim);
        let sample = if observer || encounter::stress_view(&view) {
            ActorIntent::default()
        } else if view.starts_with("encounter-") && view != "encounter-landmark" {
            let mut route_step = world.resource::<ViewState>().capture_route_step;
            let waypoint = encounter::fort_capture_waypoint(
                &view,
                world.resource::<ArenaTerrainView>(),
                *world.resource::<ArenaVoxelGeometry>(),
                world.resource::<ArenaSession>(),
                &mut route_step,
            );
            let action_frame = {
                let mut state = world.resource_mut::<ViewState>();
                state.capture_route_step = route_step;
                if encounter::fort_approach_complete(route_step) {
                    let start = *state.capture_approach_frame.get_or_insert(frame);
                    frame.saturating_sub(start).saturating_add(1)
                } else {
                    frame
                }
            };
            encounter::capture_intent(
                action_frame,
                world.resource::<ArenaSession>(),
                &view,
                waypoint,
            )
        } else {
            capture_intent(frame, &view, world.resource::<ArenaTuning>(), direction)
        };
        world.resource_mut::<ArenaInput>().human = sample;
        world
            .resource_mut::<ViewState>()
            .capture_inputs
            .push((frame, sample));
        if frame == 80 && view.contains("partial-preview") {
            stage_partial_preview(world);
        }
        if matches!(view.as_str(), "tuning" | "observer-paused") && frame == 4 {
            world.resource_mut::<ViewState>().paused = true;
        }
    }
    let frozen = world.resource::<ViewState>().paused || !world.resource::<ViewState>().started;
    let bot_enabled = world.resource::<ArenaSession>().bot_enabled;
    if frozen {
        // A single setup/reset tick publishes the complete world and actors.
        // It must not consume player input or advance the bot's reaction clock.
        world.resource_mut::<ArenaInput>().human = ActorIntent::default();
        world.resource_mut::<ArenaSession>().cancel_charges();
        world.resource_mut::<ArenaSession>().bot_enabled = false;
    }
    for _ in 0..steps {
        let stimulus = if capture && encounter::stress_view(&view) {
            encounter::prepare_stress_tick(world)
        } else {
            None
        };
        let before = world.resource::<ArenaTerrainView>().revision;
        let voxels_before = world.resource::<ArenaTerrainView>().voxels.len();
        let outcomes_before = world.resource::<ArenaSession>().terrain_outcomes;
        let started = std::time::Instant::now();
        world.run_schedule(ArenaTick);
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
        let changed = before != world.resource::<ArenaTerrainView>().revision;
        let tick = world.resource::<ArenaSession>().tick;
        if capture && wisp::stress_view(&view) {
            let load = wisp::stress_load(world.resource::<ArenaSession>());
            let damage_outcome =
                world.resource::<ArenaSession>().terrain_outcomes > outcomes_before;
            let destroyed_voxels =
                voxels_before.saturating_sub(world.resource::<ArenaTerrainView>().voxels.len());
            world
                .resource_mut::<ViewState>()
                .capture_wisp_ticks
                .push(wisp::StressTick {
                    frame,
                    tick,
                    cpu_ms: elapsed,
                    terrain_publication: changed,
                    damage_outcome,
                    destroyed_voxels,
                    load,
                });
        }
        if let Some(stimulus) = stimulus {
            let summary = world.resource::<ArenaSession>().encounter_summary();
            let damage_outcome =
                world.resource::<ArenaSession>().terrain_outcomes > outcomes_before;
            let destroyed_voxels =
                voxels_before.saturating_sub(world.resource::<ArenaTerrainView>().voxels.len());
            let mut state = world.resource_mut::<ViewState>();
            state.capture_stress_steps += 1;
            state.capture_stress_ticks.push(encounter::StressTick {
                frame,
                tick,
                active_parties: summary.active_parties,
                living_enemies: summary.living_enemies,
                terrain_publication: changed,
                damage_outcome,
                destroyed_voxels,
                cpu_ms: elapsed,
                stimulus,
            });
            if state.capture_stress_steps >= 3600 {
                state.capture_event_frame = Some(frame);
                state.accumulator = 0.0;
            }
        }
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
        if capture && spectator::active(world.resource::<ArenaSession>()) {
            let terminal = world.resource::<ArenaSession>().is_finished();
            let reached = match view.as_str() {
                "observer-result" => terminal,
                "observer-performance" => terminal || tick >= 3600,
                "observer-wisp-stress" => terminal || tick >= wisp::STRESS_TICKS,
                "observer-orbit" | "observer-orbit-rear" | "observer-free" => frame >= 120,
                _ => false,
            };
            if reached {
                if terminal {
                    // Publish edits queued by the finishing tick. Gameplay's
                    // terminal guard prevents an extra living simulation tick.
                    let revision = world.resource::<ArenaTerrainView>().revision;
                    let publication_started = std::time::Instant::now();
                    world.run_schedule(ArenaTick);
                    let changed = world.resource::<ArenaTerrainView>().revision != revision;
                    let cpu_ms = publication_started.elapsed().as_secs_f64() * 1000.0;
                    let final_tick = world.resource::<ArenaSession>().tick;
                    let mut state = world.resource_mut::<ViewState>();
                    state.tick_times.push((tick, changed, cpu_ms));
                    if wisp::stress_view(&view) {
                        state.capture_wisp_terminal = Some(wisp::TerminalPublication {
                            tick: final_tick,
                            terrain_publication: changed,
                            cpu_ms,
                        });
                    }
                }
                let mut state = world.resource_mut::<ViewState>();
                state.capture_event_frame = Some(frame);
                state.accumulator = 0.0;
                break;
            }
        }
        let approach_ready = world.resource::<ArenaTerrainView>().selection.map != ArenaMap::Fort
            || encounter::fort_approach_complete(world.resource::<ViewState>().capture_route_step);
        if capture
            && approach_ready
            && encounter::phase_ready(world.resource::<ArenaSession>(), &view)
            && worm::terrain_phase_ready(
                world.resource::<ArenaSession>(),
                &view,
                world.resource::<ArenaTerrainView>(),
                *world.resource::<ArenaVoxelGeometry>(),
            )
        {
            let mut state = world.resource_mut::<ViewState>();
            state.capture_event_frame = Some(frame);
            state.accumulator = 0.0;
            break;
        }
        if capture && world.resource::<ViewState>().capture_stress_steps >= 3600 {
            break;
        }
    }
    if frozen {
        world.resource_mut::<ArenaSession>().bot_enabled = bot_enabled;
    }
    let final_tick = world.resource::<ArenaSession>().tick;
    let ticks_advanced = if reset_this_frame {
        final_tick
    } else {
        final_tick.saturating_sub(tick_before_frame)
    };
    let mut state = world.resource_mut::<ViewState>();
    if state.simulation_frame_times.len() < 36_000 {
        state
            .simulation_frame_times
            .push(f64::from(u32::try_from(ticks_advanced).unwrap_or(0)) * FIXED_SECONDS * 1000.0);
    }
    show_terminal_menu(world);
}

fn show_terminal_menu(world: &mut World) {
    let state = world.resource::<ViewState>();
    if !state.started || state.paused || !world.resource::<ArenaSession>().is_finished() {
        return;
    }
    // Settle terrain queued by the last impact before freezing the round. The
    // terminal gameplay guard prevents another living movement/ability tick.
    world.run_schedule(ArenaTick);
    world.resource_mut::<ViewState>().pause();
    world.resource_mut::<ArenaSession>().cancel_charges();
    world.resource_mut::<ArenaInput>().human = ActorIntent::default();
    let mut cursors = world.query_filtered::<&mut CursorOptions, With<PrimaryWindow>>();
    for mut cursor in cursors.iter_mut(world) {
        cursor.visible = true;
        cursor.grab_mode = CursorGrabMode::None;
    }
}

// Capture fixtures drive the same input edges as native play. Existing explosion
// views release at frame 48; projectile holds approximate the reference speed.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "validated charge duration is positive and at most five seconds"
)]
fn capture_intent(frame: u32, view: &str, tuning: &ArenaTuning, direction: Vec3) -> ActorIntent {
    let spell = if view.starts_with("shield") {
        Some(Spell::Shield)
    } else if view.starts_with("fireball") {
        Some(Spell::Fireball)
    } else if view.starts_with("blast") {
        Some(Spell::AreaBlast)
    } else {
        None
    };
    let reference_frames = (tuning.reference_charge_seconds() * 60.0).round() as u32;
    let hold_review =
        view.contains("charge-") || view.contains("armed") || view.contains("partial-preview");
    let release_frame = if spell == Some(Spell::Shield) {
        6 + reference_frames
    } else {
        48
    };
    let press_frame = if view.contains("charge-partial") {
        capture_frame_index(view)
            .saturating_sub((tuning.charge_seconds * 0.5 * 60.0).round() as u32)
            .saturating_add(1)
    } else if hold_review || spell == Some(Spell::Shield) {
        6
    } else if spell == Some(Spell::AreaBlast) {
        release_frame
    } else {
        release_frame.saturating_sub(reference_frames).max(3)
    };
    let casts = spell.is_some() && (!view.contains("preview") || hold_review);
    ActorIntent {
        aim: if frame > 2 && matches!(spell, Some(Spell::Shield | Spell::Fireball)) {
            Vec3::new(direction.x, -0.2, direction.z).normalize_or_zero()
        } else {
            direction
        },
        selected: (frame == 3).then_some(spell).flatten(),
        cast_pressed: casts && frame == press_frame,
        cast_released: casts && !hold_review && frame == release_frame,
        cast_held: casts && frame >= press_frame && (hold_review || frame < release_frame),
        ..default()
    }
}

fn capture_frame_index(view: &str) -> u32 {
    if view.starts_with("bot-combat-") {
        600
    } else if view.contains("charge-partial") {
        36
    } else if view.contains("charge-full") || view.contains("partial-preview") {
        90
    } else if view.starts_with("blast") || view.starts_with("fireball") {
        57
    } else {
        90
    }
}

fn stage_partial_preview(world: &mut World) {
    let geometry = *world.resource::<hex_core::arena::ArenaVoxelGeometry>();
    let predicted = hex_arena::preview(
        world.resource::<ArenaSession>(),
        world.resource::<ArenaTerrainView>(),
        &geometry,
        world.resource::<ArenaTuning>(),
    );
    let Some(impact) = predicted.impact else {
        return;
    };
    let Some(actor) = world.resource::<ArenaSession>().actors.first() else {
        return;
    };
    let contact_column = hex_core::HexCoord::from_world(impact);
    let visible_side = impact + actor.aim.with_y(0.0).normalize_or_zero();
    // Clip an adjacent column forward of contact, inside both playable views.
    // The nearest outer endpoint can sit beside/behind the first-person camera.
    // Leave the impact column clear so the seed keeps the same anchored footprint.
    let edge = presentation::shield_footprint(&predicted.wall_voxels)
        .into_iter()
        .filter(|pos| pos.coord != contact_column)
        .min_by(|a, b| {
            geometry
                .center(*a)
                .distance_squared(visible_side)
                .total_cmp(&geometry.center(*b).distance_squared(visible_side))
        });
    let Some(edge) = edge else {
        return;
    };
    let stone = world.resource::<hex_core::arena::ArenaMaterials>().stone;
    for pos in [edge, edge.above()] {
        if predicted.wall_voxels.contains(&pos) {
            world.write_message(hex_core::TerrainEdit::Set {
                pos,
                substance: stone,
            });
            world
                .resource_mut::<ViewState>()
                .capture_fixture_voxels
                .push(pos);
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
    tuning: Res<ArenaTuning>,
    geometry: Res<hex_core::arena::ArenaVoxelGeometry>,
    game_assets: Option<Res<hex_assets::GameAssets>>,
    meshes: Res<Assets<Mesh>>,
    objects: Query<&hex_assets::ObjectInstance>,
    chunks: Query<&hex_objects::ObjectRenderChunk>,
    creature_meshes: (
        Query<&Mesh3d, With<golem::GolemPrism>>,
        Query<&Mesh3d, With<wisp::WispPrism>>,
        Query<(&Mesh3d, &worm::WormPart, &Transform), With<worm::WormPrism>>,
        Query<&Mesh3d, With<worm::BoulderVisual>>,
        Query<&Mesh3d, With<wisp::WispWindup>>,
    ),
    cameras: Query<&Transform, With<ArenaCamera>>,
    liquid_clock: Option<Res<hex_map::LiquidVisualTime>>,
    mut exit: MessageWriter<AppExit>,
    lighting: (Res<GlobalAmbientLight>, Query<&DirectionalLight>),
    worm_context: (
        Res<worm_capture::Evidence>,
        Res<ArenaReset>,
        Res<hex_core::arena::ArenaMaterials>,
        Res<hex_core::DamagedVoxels>,
    ),
) {
    let Some(path) = state.capture.clone() else {
        return;
    };
    if state.requested {
        return;
    }
    let (golem_prisms, wisp_prisms, worm_prisms, boulders, wisp_windups) = creature_meshes;
    let object_count = objects.iter().count();
    let chunk_count = chunks.iter().count();
    let needs_objects = !view.static_spans.is_empty() || object_count > 0;
    let authored_assets_ready = !needs_objects
        || (object_count > 0
            && chunk_count > 0
            && game_assets
                .as_ref()
                .is_some_and(|assets| meshes.get(&assets.hex_tile).is_some()));
    let expected_golem_prisms = session
        .actors
        .iter()
        .filter(|actor| actor.species == hex_arena::Species::Golem)
        .map(|actor| actor.body_hex_prisms().count())
        .sum::<usize>();
    let golem_prism_count = golem_prisms.iter().count();
    let golem_assets_ready = golem_prism_count == expected_golem_prisms
        && golem_prisms
            .iter()
            .all(|mesh| meshes.get(&mesh.0).is_some());
    let expected_wisp_prisms = session
        .actors
        .iter()
        .filter(|actor| actor.species == hex_arena::Species::Wisp)
        .map(|actor| actor.body_hex_prisms().count())
        .sum::<usize>();
    let wisp_prism_count = wisp_prisms.iter().count();
    let wisp_windup_count = wisp_windups.iter().count();
    let expected_wisp_windups = session
        .actors
        .iter()
        .filter(|actor| {
            actor.species == hex_arena::Species::Wisp
                && actor.hp > 0.0
                && actor.attack_state().is_some_and(|attack| {
                    attack.kind == hex_arena::CreatureAbility::WispEmber
                        && attack.phase == hex_arena::AttackPhase::Windup
                })
        })
        .count()
        * 6;
    let wisp_assets_ready = expected_wisp_prisms == wisp_prism_count
        && expected_wisp_windups == wisp_windup_count
        && wisp_windups
            .iter()
            .all(|mesh| meshes.get(&mesh.0).is_some())
        && wisp_prisms.iter().all(|mesh| meshes.get(&mesh.0).is_some());
    let expected_worm_prisms = session
        .actors
        .iter()
        .filter(|actor| actor.species == hex_arena::Species::Worm)
        .map(|actor| actor.body_hex_prisms().count())
        .sum::<usize>();
    let worm_prism_count = worm_prisms.iter().count();
    let expected_boulders = session
        .projectiles
        .iter()
        .filter(|shot| shot.appearance() == hex_arena::ProjectileAppearance::Boulder)
        .count();
    let boulder_count = boulders.iter().count();
    let worm_assets_ready = expected_worm_prisms == worm_prism_count
        && worm_prisms
            .iter()
            .all(|(mesh, _, _)| meshes.get(&mesh.0).is_some())
        && expected_boulders == boulder_count
        && boulders.iter().all(|mesh| meshes.get(&mesh.0).is_some());
    if !authored_assets_ready || !golem_assets_ready || !wisp_assets_ready || !worm_assets_ready {
        state.capture_ready_frame = None;
        if state.frames >= 1800 {
            error!(
                "Encounter capture failed: authored objects or creature meshes did not become render-ready"
            );
            state.requested = true;
            exit.write(AppExit::error());
        }
        return;
    }
    let frames = state.frames;
    let ready_frame = *state.capture_ready_frame.get_or_insert(frames);
    if frames >= ready_frame.saturating_add(4) && state.capture_ready_elapsed_ms.is_none() {
        state.capture_ready_elapsed_ms =
            Some(state.app_started_at.elapsed().as_secs_f64() * 1000.0);
    }
    if matches!(
        state.capture_view.as_str(),
        "observer-result"
            | "observer-performance"
            | "observer-wisp-stress"
            | "observer-orbit"
            | "observer-orbit-rear"
            | "observer-free"
    ) && state.capture_event_frame.is_none()
    {
        if frames >= 15_000 {
            error!("Observer capture failed: terminal/performance boundary was not reached");
            state.requested = true;
            exit.write(AppExit::error());
        }
        return;
    }
    if encounter::stress_view(&state.capture_view) && state.capture_event_frame.is_none() {
        if state.frames > 2000 || session.is_finished() {
            error!("Synthetic encounter stress capture ended before 3600 active simulation ticks");
            state.requested = true;
            exit.write(AppExit::error());
        }
        return;
    }
    if encounter::phase_view(&state.capture_view) && state.capture_event_frame.is_none() {
        if state.frames >= 1800 || session.is_finished() {
            error!(
                tick = session.tick,
                approach_waypoint = state.capture_route_step,
                actors = ?session.actors.iter().map(|actor| (actor.id, actor.feet, actor.hp, actor.attack_state(), actor.worm())).collect::<Vec<_>>(),
                parties = ?session.parties(),
                "Encounter capture phase failure state"
            );
            error!(
                "Encounter capture failed: requested phase {} was not reached before the round ended or the 30-second limit",
                state.capture_view
            );
            state.requested = true;
            exit.write(AppExit::error());
        }
        return;
    }
    if spectator::close_view(&state.capture_view) && state.capture_composition_frame.is_none() {
        let subjects = cameras
            .single()
            .ok()
            .map(|camera| spectator::close_subjects(&session, camera))
            .unwrap_or_default();
        if frames >= 110 && spectator::both_teams_visible(&session, &subjects) {
            state.capture_subjects = subjects;
            state.capture_composition_frame = Some(frames);
            state.accumulator = 0.0;
        } else if frames >= 1800 || session.is_finished() {
            error!(tick=session.tick, subjects=?subjects, "Observer close capture failed: both living teams were not visible at useful scale");
            state.requested = true;
            exit.write(AppExit::error());
        }
        return;
    }
    if encounter::composition_view(&state.capture_view, view.selection.map)
        && state.capture_composition_frame.is_none()
    {
        let subjects = if encounter::fort_approach_complete(state.capture_route_step) {
            cameras
                .single()
                .ok()
                .map(|camera| encounter::visible_subjects(&session, camera, &state.capture_view))
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        if !subjects.is_empty() {
            state.capture_subjects = subjects;
            state.capture_composition_frame = Some(frames);
            state.accumulator = 0.0;
        } else if frames >= 1800 || session.is_finished() {
            error!(tick=session.tick, approach_waypoint=state.capture_route_step,
                actors=?session.actors.iter().map(|actor| (actor.id, actor.feet, actor.hp)).collect::<Vec<_>>(),
                "Encounter capture failed: no visible subject after the verified Fort approach");
            state.requested = true;
            exit.write(AppExit::error());
        }
        return;
    }
    let frame = if let Some(frame) = state
        .capture_event_frame
        .or(state.capture_composition_frame)
    {
        frame.saturating_add(4)
    } else if state.capture_view.starts_with("bot-combat-") && session.is_finished() {
        state.frames
    } else {
        capture_frame_index(&state.capture_view)
    };
    if state.frames < frame || state.requested {
        return;
    }
    if state.capture_view == "encounter-landmark"
        && !state
            .capture_focus
            .as_ref()
            .is_some_and(|name| view.anchors.contains_key(name))
    {
        error!("Encounter capture requires a published focus anchor");
        state.requested = true;
        exit.write(AppExit::error());
        return;
    }
    if frames < ready_frame.saturating_add(4) {
        return;
    }
    if liquid_clock
        .as_ref()
        .is_none_or(|clock| !clock.is_frozen() || clock.phase_seconds().abs() > f32::EPSILON)
    {
        error!("Encounter capture requires liquid presentation frozen at phase zero");
        state.requested = true;
        exit.write(AppExit::error());
        return;
    }
    let Some(target) = state.image.clone() else {
        return;
    };
    state.requested = true;
    let predicted = if session.human_actor_id().is_some() {
        hex_arena::preview(&session, &view, &geometry, &tuning)
    } else {
        hex_arena::Preview::default()
    };
    // Keep each macro bounded: a single large object exceeds serde_json's recursive
    // token parser limit as new capture evidence is added.
    let actors = session.actors.iter().map(|actor| {
        let attack = actor.attack_state().map(|attack| serde_json::json!({
            "kind": attack.kind, "phase": attack.phase, "origin": attack.origin.to_array(),
            "direction": attack.direction.to_array(), "range": attack.range,
            "half_angle": attack.half_angle, "progress": attack.progress
        }));
        let charge = actor.charge().map(|charge| serde_json::json!({
            "spell": charge.spell, "elapsed": charge.elapsed,
            "progress": (charge.elapsed / tuning.charge_seconds).clamp(0.0, 1.0),
            "launch_speed": tuning.launch_speed(charge.elapsed)
        }));
        let body_hex_prisms = actor.body_hex_prisms().map(|prism| serde_json::json!({
            "offset": prism.offset.to_array(), "height": prism.height
        })).collect::<Vec<_>>();
        let beam = actor.beam().map(|beam| serde_json::json!({
            "origin": beam.origin.to_array(), "direction": beam.direction.to_array(),
            "end": beam.end.to_array(), "radius": beam.radius, "tracking": beam.tracking
        }));
        serde_json::json!({
            "id": actor.id, "species": actor.species, "team": actor.team,
            "party": actor.party, "hp": actor.hp, "max_hp": actor.max_hp,
            "feet": actor.feet.to_array(), "body_dimensions": actor.body_dimensions().to_array(),
            "body_center": actor.center().to_array(), "worm": actor.worm(),
            "head_earth": worm::head_earth(actor, &view, *geometry),
            "body_rotation": actor.body_rotation().to_array(), "cooldowns": actor.cooldowns,
            "attack": attack, "charge": charge, "body_hex_prisms": body_hex_prisms,
            "idle_mouth": actor.eye().to_array(), "beam": beam, "flying": actor.flying, "grounded": actor.grounded,
            "flight_layer": actor.flight_layer()
        })
    }).collect::<Vec<_>>();
    let projectiles = session.projectiles.iter().map(|projectile|serde_json::json!({
        "id":projectile.id,"owner":projectile.owner,"source_team":projectile.source_team(),
        "position":projectile.position.to_array(),"previous_position":projectile.previous_position.to_array(),
        "velocity":projectile.velocity.to_array(),"age":projectile.age,"appearance":projectile.appearance(),
        "collision_radius":projectile.collision_radius(),"source_ability":projectile.source_ability()
    })).collect::<Vec<_>>();
    let barriers = session.barriers().iter().map(|barrier| serde_json::json!({
        "id": barrier.id, "owner": barrier.owner, "center": barrier.center.to_array(),
        "normal": barrier.normal.to_array(), "width": barrier.width, "height": barrier.height,
        "hp": barrier.hp, "max_hp": barrier.max_hp, "remaining": barrier.remaining,
        "lifetime": barrier.lifetime
    })).collect::<Vec<_>>();
    let auras = session
        .auras()
        .iter()
        .map(|aura| {
            serde_json::json!({
                "owner": aura.owner, "center": aura.center.to_array(), "radius": aura.radius,
                "remaining": aura.remaining, "lifetime": aura.lifetime
            })
        })
        .collect::<Vec<_>>();
    let parties = session.parties().iter().map(|party| serde_json::json!({
        "id": party.id, "phase": party.phase, "home": party.home.to_array(), "living": party.living
    })).collect::<Vec<_>>();
    let capture_inputs = state
        .capture_inputs
        .iter()
        .map(|(frame, input)| {
            serde_json::json!({
                "frame": frame, "selected": input.selected, "movement": input.movement.to_array(),
                "aim": input.aim.to_array(), "run": input.run, "jump": input.jump, "pressed": input.cast_pressed,
                "released": input.cast_released, "held": input.cast_held
            })
        })
        .collect::<Vec<_>>();
    let effects = session
        .effects
        .iter()
        .map(|effect| {
            serde_json::json!({
                "spell": effect.kind, "radius": effect.radius, "age": effect.age, "center": effect.center.to_array()
            })
        })
        .collect::<Vec<_>>();
    let tick_samples = state
        .tick_times
        .iter()
        .map(|(tick, changed, ms)| {
            serde_json::json!({
                "tick": tick, "terrain_changed": changed, "cpu_ms": ms
            })
        })
        .collect::<Vec<_>>();
    let receipt = serde_json::Value::Object([
        ("view", serde_json::json!(state.capture_view)),
        ("started", serde_json::json!(state.started)),
        ("paused", serde_json::json!(state.paused)),
        ("frame", serde_json::json!(state.frames)),
        ("tick", serde_json::json!(session.tick)),
        ("selection", serde_json::json!({"map": map_name(view.selection.map), "encounter": encounter_name(view.selection.encounter)})),
        ("terrain_revision", serde_json::json!(view.revision)),
        ("voxels", serde_json::json!(view.voxels.len())),
        ("static_spans", serde_json::json!(view.static_spans.len())),
        ("authored_objects", serde_json::json!(object_count)),
        ("golem_render_prisms", serde_json::json!(golem_prism_count)),
        ("wisp_render_prisms", serde_json::json!(wisp_prism_count)),
        ("wisp_windup_segments", serde_json::json!(wisp_windup_count)),
        ("worm_render_prisms", serde_json::json!(worm_prism_count)),
        ("boulder_render_count", serde_json::json!(boulder_count)),
        ("worm_render_parts", serde_json::json!(worm_prisms.iter().map(|(_, part, transform)| serde_json::json!({"actor":part.actor_id(),"index":part.segment_index(),"translation":transform.translation.to_array(),"scale":transform.scale.to_array()})).collect::<Vec<_>>())),
        ("worm_capture", worm_context.0.receipt(&view, &worm_context.1, &worm_context.2, &worm_context.3)),
        ("projectiles", serde_json::json!(projectiles)),
        ("lighting", serde_json::json!({"fixture":if wisp::dim_view(&state.capture_view) {"dim-comparison"}else{"ordinary"}, "ambient_brightness":lighting.0.brightness, "directional_illuminance":lighting.1.iter().map(|light|light.illuminance).collect::<Vec<_>>()})),
        ("object_render_chunks", serde_json::json!(chunk_count)),
        ("focus_anchor", serde_json::json!(state.capture_focus)),
        ("phase_reached_frame", serde_json::json!(state.capture_event_frame)),
        ("render_ready_frame", serde_json::json!(state.capture_ready_frame)),
        ("liquid_phase_seconds", serde_json::json!(liquid_clock.as_ref().map(|clock| clock.phase_seconds()))),
        ("app_construction_to_render_ready_ms", serde_json::json!(state.capture_ready_elapsed_ms)),
        ("camera", serde_json::json!(cameras.single().ok().map(|camera| serde_json::json!({"position": camera.translation.to_array(), "rotation": camera.rotation.to_array()})))),
        ("approach_waypoint_index", serde_json::json!(state.capture_route_step)),
        ("approach_complete", serde_json::json!(encounter::fort_approach_complete(state.capture_route_step))),
        ("composition_reached_frame", serde_json::json!(state.capture_composition_frame)),
        ("visible_subjects", serde_json::json!(state.capture_subjects)),
        ("approach_completed_frame", serde_json::json!(state.capture_approach_frame)),
        ("battle_setup", serde_json::json!(session.accepted_battle_setup())),
        ("human_actor_id", serde_json::json!(session.human_actor_id())),
        ("battle_summary", serde_json::json!(session.battle_summary())),
        ("observer_camera_inputs", serde_json::json!(state.capture_observer_inputs)),
        ("observer_camera", serde_json::json!({"mode":format!("{:?}",state.observer.mode),"position":state.observer.position.to_array(),"target":state.observer.target.to_array(),"distance":state.observer.distance,"actual_distance":state.observer.position.distance(state.observer.target)})),
        ("actors", serde_json::json!(actors)),
        ("barriers", serde_json::json!(barriers)),
        ("auras", serde_json::json!(auras)),
        ("parties", serde_json::json!(parties)),
        ("encounter_summary", serde_json::json!(session.encounter_summary())),
        ("encounter_stats", serde_json::json!(session.encounter_stats())),
        ("synthetic_fixture", serde_json::json!(encounter::stress_view(&state.capture_view).then_some("synthetic-party-visits-extra-life: all actors start with 100000 HP; human revisits persistent party areas for 72 ticks with current dry supported, body-clear and visible placement within 10 units of home; forward distances Dragon 2.5, Goblin 1.1, Shaman/Shadow 8 units; Area Blast requested every 240 ticks; normal brains/physics. Not movement, human balance, or ordinary gameplay evidence."))),
        ("stress_ticks", serde_json::json!(state.capture_stress_ticks)),
        ("wisp_stress", serde_json::json!(wisp::stress_view(&state.capture_view).then(|| serde_json::json!({
            "fixture": "synthetic-wisp-hp-1000", "nominal_hp": state.capture_wisp_nominal_hp,
            "loaded_from_config": state.capture_wisp_config_loaded,
            "applied_hp": wisp::STRESS_HP, "warmup_ticks": 120, "requested_ticks": wisp::STRESS_TICKS,
            "actor_hp_mutation": false, "injected_terrain_impacts": false, "rows": state.capture_wisp_ticks,
            "terminal_publication": state.capture_wisp_terminal,
            "note": "Validated Wisp HP1000 before admission; ordinary autonomous brains, movement and projectiles. Zero terrain publications are reported honestly; this is not the separate destruction workload."
        })))),
        ("capture_inputs", serde_json::json!(capture_inputs)),
        ("fixture_voxels", serde_json::json!(state.capture_fixture_voxels)),
        ("preview", serde_json::json!({"valid": predicted.valid, "wall_voxels": predicted.wall_voxels, "footprint": presentation::shield_footprint(&predicted.wall_voxels)})),
        ("bot_debug", serde_json::json!(session.bot_debug())),
        ("round_summary", serde_json::json!(session.round_summary())),
        ("notice", serde_json::json!(session.notice)),
        ("shields_raised", serde_json::json!(session.shields_raised)),
        ("terrain_outcomes", serde_json::json!(session.terrain_outcomes)),
        ("effects", serde_json::json!(effects)),
        ("engine_time_delta_ms", serde_json::json!(state.frame_times)),
        ("simulation_frame_dt_ms", serde_json::json!(state.simulation_frame_times)),
        ("app_frame_wall_intervals_ms", serde_json::json!(state.frame_wall_intervals)),
        ("frame_timing_note", serde_json::json!("Instant start-to-start of consecutive main app Update frames; includes scheduler and render-submission waits, not GPU execution or vsync timing. Simulation dt is a separate engine clock.")),
        ("frame_interval_indexing", serde_json::json!("Wall interval index 0 spans Update starts at frame 1 to 2 and includes frame 1 work; stress tick rows identify their containing app frame.")),
        ("tick_samples", serde_json::json!(tick_samples)),
        ("width", serde_json::json!(WIDTH)),
        ("height", serde_json::json!(HEIGHT)),
        ("evidence", serde_json::json!(if encounter::stress_view(&state.capture_view) || wisp::stress_view(&state.capture_view) { "SYNTHETIC_PERFORMANCE; normal gameplay, movement, human balance, GPU and vsync are not established" } else { "STATIC_CAPTURE_UNREVIEWED; logic is recorded separately; native feel pending" })),
    ].into_iter().map(|(key, value)| (key.to_owned(), value)).collect());
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
