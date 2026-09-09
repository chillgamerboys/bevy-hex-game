//! Combined producer/consumer and input lifecycle contracts, without a window.

use super::*;
use hex_arena::preview;
use hex_core::arena::ArenaVoxelGeometry;
use hex_test_app::HeadlessAppBuilder;

fn app(frame_hz: u32) -> App {
    let mut builder = HeadlessAppBuilder::new()
        .with_minimal_plugins()
        .with_fixed_step(std::time::Duration::from_secs_f64(
            1.0 / f64::from(frame_hz),
        ));
    builder
        .app_mut()
        .insert_resource(ViewState {
            started: true,
            paused: false,
            capture: None,
            capture_view: "first".into(),
            ..default()
        })
        .add_plugins((hex_map::arena::plugin, hex_arena::plugin))
        .add_systems(Update, drive_simulation);
    let mut app = builder.build();
    app.world_mut().resource_mut::<ArenaSession>().bot_enabled = false;
    app.update();
    app.world_mut().run_schedule(ArenaTick);
    app
}

fn tick(app: &mut App) {
    app.world_mut().run_schedule(ArenaTick);
}

#[test]
fn third_person_body_hiding_matches_the_actual_camera_beside_a_wall() {
    use bevy::ecs::system::RunSystemOnce;
    use hex_core::{HexCoord, TerrainEdit, TilePos};

    let mut fixture = app(60);
    let stone = fixture
        .world()
        .resource::<hex_core::arena::ArenaMaterials>()
        .stone;
    for level in 9..=13 {
        fixture.world_mut().write_message(TerrainEdit::Set {
            pos: TilePos::new(HexCoord::ORIGIN, level),
            substance: stone,
        });
    }
    tick(&mut fixture);
    let eye = {
        let mut session = fixture.world_mut().resource_mut::<ArenaSession>();
        let human = session.actors.first_mut().expect("human initialized");
        human.feet = Vec3::new(1.2, 3.2, 0.0);
        human.aim = Vec3::X;
        human.eye()
    };
    fixture
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();
    fixture
        .world_mut()
        .run_system_once(golem::setup)
        .expect("Golem cached visuals");
    fixture
        .world_mut()
        .run_system_once(wisp::setup)
        .expect("Wisp cached visuals");
    fixture
        .world_mut()
        .run_system_once(worm::setup)
        .expect("Worm cached visuals");
    let camera_entity = fixture
        .world_mut()
        .spawn((ArenaCamera, Transform::default()))
        .id();

    // Actor aim looks away from the wall, retracting the camera into the body.
    // The deliberately stale view aim points along the wall into open space.
    // Check first model spawn, existing models, and the initialized live policy.
    for (capture, initialized, retracted) in [
        (false, false, true),
        (true, true, true),
        (false, true, false),
    ] {
        {
            let mut state = fixture.world_mut().resource_mut::<ViewState>();
            state.third_person = true;
            state.initialized = initialized;
            state.capture = capture.then(|| PathBuf::from("unused-test-capture.png"));
            state.capture_view = "bot-combat-third".into();
            state.yaw = 0.0;
            state.pitch = 0.0;
        }
        fixture
            .world_mut()
            .run_system_once(presentation::actors)
            .expect("actor presentation");
        fixture
            .world_mut()
            .run_system_once(presentation::camera)
            .expect("camera presentation");
        let camera = fixture
            .world()
            .get::<Transform>(camera_entity)
            .expect("camera transform");
        assert_eq!(camera.translation.distance(eye) < 0.45, retracted);
        let direction = if retracted { Vec3::X } else { Vec3::NEG_Z };
        assert!(Vec3::from(camera.forward()).dot(direction) > 0.999);
        let expected = if retracted {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
        let mut models = fixture.world_mut().query::<(&Name, &Visibility)>();
        let mut seen = 0;
        for (name, visibility) in models.iter(fixture.world()) {
            match name.as_str() {
                "Arena actor 0" => {
                    assert_eq!(
                        *visibility, expected,
                        "capture={capture}, initialized={initialized}"
                    );
                    seen += 1;
                }
                "Arena actor 1" => {
                    assert_eq!(*visibility, Visibility::Visible);
                    seen += 1;
                }
                _ => {}
            }
        }
        assert_eq!(seen, 2);
    }
}

fn menu_app() -> (App, Entity) {
    menu_app_at(60)
}

fn menu_app_at(frame_hz: u32) -> (App, Entity) {
    let mut builder = HeadlessAppBuilder::new()
        .with_minimal_plugins()
        .with_fixed_step(std::time::Duration::from_secs_f64(
            1.0 / f64::from(frame_hz),
        ));
    builder
        .app_mut()
        .insert_resource(ViewState {
            capture: None,
            ..default()
        })
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ButtonInput<MouseButton>>()
        .add_message::<CursorMoved>()
        .add_message::<MouseWheel>()
        .add_message::<AppExit>()
        .add_plugins((hex_map::arena::plugin, hex_arena::plugin))
        .add_systems(PreUpdate, (input, hud::buttons, sync_cursor).chain())
        .add_systems(Update, drive_simulation);
    let mut app = builder.build();
    let window = app
        .world_mut()
        .spawn((
            Window {
                focused: true,
                ..default()
            },
            CursorOptions::default(),
            PrimaryWindow,
        ))
        .id();
    app.update();
    (app, window)
}

fn tap_key(app: &mut App, key: KeyCode) {
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(key);
    app.update();
    *app.world_mut().resource_mut::<ButtonInput<KeyCode>>() = ButtonInput::default();
}

fn press_action(app: &mut App, action: hud::Action) {
    let button = app.world_mut().spawn((Interaction::Pressed, action)).id();
    app.update();
    app.world_mut().despawn(button);
}

#[derive(Debug, PartialEq)]
struct CombatSnapshot {
    tick: u64,
    actors: Vec<(u8, Vec3, f32, [f32; 3])>,
    projectiles: Vec<(Vec3, Vec3, f32)>,
    terrain_revision: u64,
}

fn combat_snapshot(app: &App) -> CombatSnapshot {
    let session = app.world().resource::<ArenaSession>();
    CombatSnapshot {
        tick: session.tick,
        actors: session
            .actors
            .iter()
            .map(|actor| (actor.id, actor.feet, actor.hp, actor.cooldowns))
            .collect(),
        projectiles: session
            .projectiles
            .iter()
            .map(|shot| (shot.position, shot.velocity, shot.age))
            .collect(),
        terrain_revision: app.world().resource::<ArenaTerrainView>().revision,
    }
}

#[test]
fn ready_screen_freezes_combat_and_only_enter_starts_keyboard_play() {
    let (mut app, window) = menu_app();
    assert_eq!(app.world().resource::<ArenaSession>().actors.len(), 2);
    assert!(!app.world().resource::<ViewState>().started);
    assert!(app.world().resource::<ViewState>().paused);
    let frozen = combat_snapshot(&app);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyW);
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    for _ in 0..180 {
        app.update();
    }
    assert_eq!(combat_snapshot(&app), frozen);
    for key in [KeyCode::Escape, KeyCode::Tab] {
        tap_key(&mut app, key);
        assert!(!app.world().resource::<ViewState>().started);
        assert!(app.world().resource::<ViewState>().paused);
        assert_eq!(combat_snapshot(&app), frozen);
    }
    let cursor = app
        .world()
        .get::<CursorOptions>(window)
        .expect("synthetic cursor");
    assert!(cursor.visible && cursor.grab_mode == CursorGrabMode::None);
    tap_key(&mut app, KeyCode::Enter);
    assert!(app.world().resource::<ViewState>().started);
    assert!(!app.world().resource::<ViewState>().paused);
    assert!(app.world().resource::<ArenaSession>().tick > frozen.tick);
    assert!(
        app.world()
            .resource::<ArenaSession>()
            .projectiles
            .is_empty(),
        "held start click must not cast"
    );
    assert!(
        !app.world()
            .get::<CursorOptions>(window)
            .expect("synthetic cursor")
            .visible
    );
}

#[test]
fn start_and_resume_buttons_clear_stale_casts_and_hide_cursor_in_the_same_frame() {
    let (mut app, window) = menu_app();
    app.world_mut().resource_mut::<ArenaSession>().bot_enabled = false;
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    app.world_mut()
        .resource_mut::<ArenaInput>()
        .human
        .cast_pressed = true;
    app.world_mut().resource_mut::<ArenaInput>().human.jump = true;
    press_action(&mut app, hud::Action::Start);
    assert!(app.world().resource::<ViewState>().started);
    assert!(!app.world().resource::<ViewState>().paused);
    assert!(
        !app.world()
            .get::<CursorOptions>(window)
            .expect("synthetic cursor")
            .visible
    );
    assert!(app
        .world()
        .resource::<ArenaSession>()
        .projectiles
        .is_empty());
    assert!(!app.world().resource::<ArenaInput>().human.cast_pressed);
    tap_key(&mut app, KeyCode::Escape);
    app.world_mut()
        .resource_mut::<ArenaInput>()
        .human
        .cast_pressed = true;
    press_action(&mut app, hud::Action::Resume);
    assert!(!app.world().resource::<ViewState>().paused);
    assert!(
        !app.world()
            .get::<CursorOptions>(window)
            .expect("synthetic cursor")
            .visible
    );
    assert!(app
        .world()
        .resource::<ArenaSession>()
        .projectiles
        .is_empty());
    *app.world_mut().resource_mut::<ButtonInput<MouseButton>>() = ButtonInput::default();
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    app.update();
    assert!(app
        .world()
        .resource::<ArenaSession>()
        .actors
        .first()
        .is_some_and(|actor| actor.charge().is_some()));
    assert!(app
        .world()
        .resource::<ArenaSession>()
        .projectiles
        .is_empty());
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .clear();
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .release(MouseButton::Left);
    app.update();
    assert!(
        app.world()
            .resource::<ArenaSession>()
            .actors
            .first()
            .is_some_and(|actor| actor
                .cooldowns
                .get(1)
                .is_some_and(|cooldown| *cooldown > 1.0)),
        "a fresh gameplay press followed by release must cast"
    );
}

#[test]
fn escape_and_tab_pause_all_combat_release_cursor_and_resume_without_a_cast() {
    for key in [KeyCode::Escape, KeyCode::Tab] {
        let (mut app, window) = menu_app();
        tap_key(&mut app, KeyCode::Enter);
        if let Some(actor) = app
            .world_mut()
            .resource_mut::<ArenaSession>()
            .actors
            .first_mut()
        {
            actor.hp = 78.0;
            actor.cooldowns = [2.0, 1.0, 4.0];
        }
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        tap_key(&mut app, key);
        assert!(app.world().resource::<ViewState>().paused);
        let frozen = combat_snapshot(&app);
        for _ in 0..90 {
            app.update();
        }
        assert_eq!(combat_snapshot(&app), frozen);
        let cursor = app
            .world()
            .get::<CursorOptions>(window)
            .expect("synthetic cursor");
        assert!(cursor.visible && cursor.grab_mode == CursorGrabMode::None);
        assert!(!app.world().resource::<ArenaInput>().human.cast_pressed);
        tap_key(&mut app, key);
        assert!(!app.world().resource::<ViewState>().paused);
        assert!(app.world().resource::<ArenaSession>().tick > frozen.tick);
        assert!(app
            .world()
            .resource::<ArenaSession>()
            .projectiles
            .is_empty());
        assert!(
            !app.world()
                .get::<CursorOptions>(window)
                .expect("synthetic cursor")
                .visible
        );
    }
}

#[test]
fn keyboard_and_menu_reset_restore_the_round_and_return_to_frozen_ready_screen() {
    for keyboard in [true, false] {
        let (mut app, window) = menu_app();
        tap_key(&mut app, KeyCode::Enter);
        if let Some(actor) = app
            .world_mut()
            .resource_mut::<ArenaSession>()
            .actors
            .first_mut()
        {
            actor.hp = 20.0;
            actor.cooldowns = [2.0, 1.0, 4.0];
        }
        let generation = app.world().resource::<ArenaReset>().generation;
        if keyboard {
            tap_key(&mut app, KeyCode::KeyR);
        } else {
            tap_key(&mut app, KeyCode::Tab);
            press_action(&mut app, hud::Action::Restart);
        }
        assert_eq!(
            app.world().resource::<ArenaReset>().generation,
            generation + 1
        );
        let state = app.world().resource::<ViewState>();
        assert!(!state.started && state.paused && state.suppress_click);
        let session = app.world().resource::<ArenaSession>();
        assert_eq!(session.actors.len(), 2);
        assert!(session.actors.iter().all(|actor| {
            (actor.hp - 100.0).abs() < 0.001
                && actor
                    .cooldowns
                    .iter()
                    .all(|cooldown| cooldown.abs() < 0.001)
        }));
        assert!(session.outcome.is_none() && session.projectiles.is_empty());
        assert!(
            app.world()
                .get::<CursorOptions>(window)
                .expect("synthetic cursor")
                .visible
        );
        let frozen = combat_snapshot(&app);
        for _ in 0..30 {
            app.update();
        }
        assert_eq!(combat_snapshot(&app), frozen);
    }
}

#[test]
fn fullscreen_toggle_preserves_pause_and_quit_emits_successful_app_exit() {
    use bevy::ecs::message::Messages;
    use bevy::window::WindowMode;

    let (mut app, window) = menu_app();
    tap_key(&mut app, KeyCode::Enter);
    tap_key(&mut app, KeyCode::Escape);
    let label = app
        .world_mut()
        .spawn((Text::new(""), hud::Label::WindowMode))
        .id();
    app.add_systems(Update, hud::update);
    app.update();
    let windowed_label = app
        .world()
        .get::<Text>(label)
        .expect("window mode label")
        .0
        .clone();
    let frozen = combat_snapshot(&app);
    press_action(&mut app, hud::Action::Fullscreen);
    assert!(!matches!(
        app.world()
            .get::<Window>(window)
            .expect("synthetic window")
            .mode,
        WindowMode::Windowed
    ));
    assert!(app.world().resource::<ViewState>().paused);
    assert!(
        app.world()
            .get::<CursorOptions>(window)
            .expect("synthetic cursor")
            .visible
    );
    assert_eq!(combat_snapshot(&app), frozen);
    assert_ne!(
        app.world().get::<Text>(label).expect("window mode label").0,
        windowed_label
    );
    press_action(&mut app, hud::Action::Fullscreen);
    assert!(matches!(
        app.world()
            .get::<Window>(window)
            .expect("synthetic window")
            .mode,
        WindowMode::Windowed
    ));
    assert_eq!(combat_snapshot(&app), frozen);
    assert_eq!(
        app.world().get::<Text>(label).expect("window mode label").0,
        windowed_label
    );
    press_action(&mut app, hud::Action::Quit);
    let exits = app
        .world_mut()
        .resource_mut::<Messages<AppExit>>()
        .drain()
        .collect::<Vec<_>>();
    assert!(matches!(exits.as_slice(), [AppExit::Success]));
    assert_eq!(combat_snapshot(&app), frozen);
}

#[test]
fn enabled_bot_damages_a_player_from_normal_arena_spawns() {
    let mut app = app(60);
    app.world_mut().resource_mut::<ArenaSession>().bot_enabled = true;
    let initial_world = app.world().resource::<ArenaTerrainView>().voxels.len();
    let mut casts = 0;
    for _ in 0..120 * 20 {
        tick(&mut app);
        let session = app.world().resource::<ArenaSession>();
        casts += session
            .projectiles
            .iter()
            .filter(|shot| shot.owner == 1 && shot.age < hex_arena::STEP)
            .count();
        if session
            .actors
            .iter()
            .any(|actor| actor.id == 0 && actor.hp < 80.0)
        {
            break;
        }
    }
    // Settle the impact through the world owner on the following physics tick.
    tick(&mut app);
    let session = app.world().resource::<ArenaSession>();
    assert!(session
        .actors
        .iter()
        .any(|actor| actor.id == 0 && actor.hp < 80.0));
    assert!(session.actors.iter().all(|actor| actor.feet.is_finite()));
    assert!(casts > 0);
    assert!(session.terrain_outcomes > 0);
    assert!(app.world().resource::<ArenaTerrainView>().voxels.len() < initial_world);
}

#[test]
fn paused_last_tick_cast_survives_message_expiry_then_refreshes_before_movement() {
    let mut app = app(60);
    let original = app.world().resource::<ArenaTerrainView>().clone();
    let initial_feet = original.spawns.first().copied().unwrap_or_default();
    app.world_mut().resource_mut::<ArenaInput>().human = ActorIntent {
        selected: Some(Spell::AreaBlast),
        cast_pressed: true,
        cast_released: true,
        ..default()
    };
    tick(&mut app);
    assert_eq!(
        app.world().resource::<ArenaTerrainView>().voxels,
        original.voxels
    );
    let cast_tick = app.world().resource::<ArenaSession>().tick;
    app.world_mut().resource_mut::<ViewState>().paused = true;
    for _ in 0..10 {
        app.update();
    }
    assert_eq!(app.world().resource::<ArenaSession>().tick, cast_tick);
    app.world_mut().resource_mut::<ViewState>().paused = false;
    app.update();
    let session = app.world().resource::<ArenaSession>();
    assert_eq!(session.terrain_outcomes, 1);
    assert!(session
        .actors
        .first()
        .is_some_and(|a| a.feet.y < initial_feet.y && a.hp > 99.9));
    assert!(app.world().resource::<ArenaTerrainView>().voxels.len() < original.voxels.len());
    app.update();
    assert_eq!(app.world().resource::<ArenaSession>().terrain_outcomes, 1);
}

#[test]
fn predicted_wall_is_published_persists_and_reset_restores_complete_world() {
    let mut app = app(60);
    let baseline = app.world().resource::<ArenaTerrainView>().clone();
    app.world_mut().resource_mut::<ArenaInput>().human = ActorIntent {
        aim: Vec3::new(1.0, -0.2, 0.0).normalize(),
        selected: Some(Spell::Shield),
        ..default()
    };
    tick(&mut app);
    let forecast = preview(
        app.world().resource::<ArenaSession>(),
        app.world().resource::<ArenaTerrainView>(),
        app.world().resource::<ArenaVoxelGeometry>(),
        app.world().resource::<ArenaTuning>(),
    );
    assert!(forecast.valid);
    assert!(!forecast.wall_voxels.is_empty() && forecast.wall_voxels.len() <= 25);
    app.world_mut()
        .resource_mut::<ArenaInput>()
        .human
        .cast_pressed = true;
    app.world_mut()
        .resource_mut::<ArenaInput>()
        .human
        .cast_released = true;
    for _ in 0..120 {
        tick(&mut app);
    }
    assert_eq!(app.world().resource::<ArenaSession>().shields_raised, 1);
    for pos in &forecast.wall_voxels {
        assert!(app
            .world()
            .resource::<ArenaTerrainView>()
            .voxels
            .contains_key(pos));
    }
    let wall_world = app.world().resource::<ArenaTerrainView>().voxels.clone();
    for _ in 0..240 {
        tick(&mut app);
    }
    assert_eq!(
        app.world().resource::<ArenaTerrainView>().voxels,
        wall_world
    );
    app.world_mut().resource_mut::<ArenaReset>().generation += 1;
    tick(&mut app);
    assert_eq!(
        app.world().resource::<ArenaTerrainView>().voxels,
        baseline.voxels
    );
    let session = app.world().resource::<ArenaSession>();
    assert_eq!(session.shields_raised, 0);
    assert!(session.projectiles.is_empty());
    assert!(session
        .actors
        .iter()
        .all(|a| a.hp > 99.9 && a.cooldowns.iter().all(|v| v.abs() < 0.001)));
    assert!(session.outcome.is_none());
}

#[test]
fn render_rates_preserve_one_second_walk_and_queued_click_is_consumed_once() {
    let mut positions = Vec::new();
    for hz in [30, 60, 144] {
        let mut app = app(hz);
        app.world_mut().resource_mut::<ArenaInput>().human = ActorIntent {
            movement: Vec2::Y,
            aim: Vec3::X,
            selected: Some(Spell::Fireball),
            cast_pressed: true,
            cast_released: true,
            ..default()
        };
        for _ in 0..hz {
            app.update();
        }
        let session = app.world().resource::<ArenaSession>();
        positions.push(session.actors.first().map_or(Vec3::ZERO, |a| a.feet));
        assert!(session
            .actors
            .first()
            .is_some_and(|a| a.cooldowns.get(1).is_some_and(|v| (0.2..0.3).contains(v))));
        assert!(!app.world().resource::<ArenaInput>().human.cast_pressed);
    }
    for pair in positions.windows(2) {
        if let [a, b] = pair {
            assert!(a.distance(*b) < 0.06);
        }
    }
}

#[test]
fn focus_loss_clears_edges_and_resume_click_cannot_cast() {
    let mut app = app(60);
    app.init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ButtonInput<MouseButton>>()
        .add_message::<CursorMoved>()
        .add_message::<MouseWheel>()
        .add_systems(PreUpdate, (input, sync_cursor).chain());
    let window = app
        .world_mut()
        .spawn((
            Window {
                focused: false,
                ..default()
            },
            CursorOptions::default(),
            PrimaryWindow,
        ))
        .id();
    app.world_mut()
        .resource_mut::<ArenaInput>()
        .human
        .cast_pressed = true;
    app.update();
    assert!(app.world().resource::<ViewState>().paused);
    assert!(!app.world().resource::<ArenaInput>().human.cast_pressed);
    if let Some(mut window) = app.world_mut().get_mut::<Window>(window) {
        window.focused = true;
    }
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Escape);
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    app.update();
    assert!(!app.world().resource::<ViewState>().paused);
    assert!(app
        .world()
        .resource::<ArenaSession>()
        .actors
        .first()
        .is_some_and(|a| a.cooldowns.iter().all(|v| v.abs() < 0.001)));
}

#[test]
fn repeated_entry_and_paused_reset_create_only_two_fresh_actors() {
    for _ in 0..3 {
        let mut app = app(60);
        for _ in 0..4 {
            app.world_mut().resource_mut::<ViewState>().paused = true;
            app.world_mut().resource_mut::<ArenaReset>().generation += 1;
            app.update();
            let session = app.world().resource::<ArenaSession>();
            assert_eq!(session.actors.len(), 2);
            assert!(session.actors.iter().all(|a| a.hp > 99.9));
            assert!(session.projectiles.is_empty());
        }
    }
}

#[test]
fn every_paused_tuning_control_remains_valid_and_sizes_are_independent() {
    let mut tuning = ArenaTuning::default();
    for field in 0..12 {
        for direction in [-1.0, 1.0] {
            for _ in 0..100 {
                hud::change(&mut tuning, field, direction);
                assert!(tuning.validate().is_ok());
            }
        }
    }
    tuning = ArenaTuning::default();
    hud::change(&mut tuning, 0, 1.0);
    assert_eq!(
        (tuning.shield_size, tuning.fireball_size, tuning.blast_size),
        (2, 1, 1)
    );
}

#[test]
fn repeated_large_blasts_measure_mutation_and_collision_refresh_cost() {
    let mut app = app(60);
    app.world_mut().resource_mut::<ArenaTuning>().blast_size = 2;
    let mut costs = Vec::new();
    let mut destruction = Vec::new();
    for frame in 0..360 {
        if frame % 30 == 0 {
            app.world_mut().resource_mut::<ArenaReset>().generation += 1;
            tick(&mut app);
            app.world_mut().resource_mut::<ArenaInput>().human = ActorIntent {
                selected: Some(Spell::AreaBlast),
                cast_pressed: true,
                cast_released: true,
                ..default()
            };
            tick(&mut app);
        }
        let before = app.world().resource::<ArenaTerrainView>().voxels.len();
        let start = std::time::Instant::now();
        app.update();
        let millis = start.elapsed().as_secs_f64() * 1000.0;
        costs.push(millis);
        if app.world().resource::<ArenaTerrainView>().voxels.len() < before {
            destruction.push(millis);
        }
    }
    assert_eq!(destruction.len(), 12);
    costs.sort_by(f64::total_cmp);
    destruction.sort_by(f64::total_cmp);
    eprintln!(
        "ARENA_PERF {}",
        serde_json::json!({
            "frames":costs.len(), "destruction_samples":destruction.len(),
            "cpu_frame_median_ms":costs.get(costs.len()/2),
            "cpu_frame_p95_ms":costs.get(costs.len()*95/100),
            "cpu_frame_max_ms":costs.last(), "destruction_max_ms":destruction.last(),
            "method":"headless CPU app update, 120Hz simulation, large blasts, no GPU"
        })
    );
}

fn clear_mouse_edges(app: &mut App) {
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .clear();
}

fn charge_with_mouse(app: &mut App) {
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    app.update();
    clear_mouse_edges(app);
    assert!(app
        .world()
        .resource::<ArenaSession>()
        .actors
        .first()
        .is_some_and(|actor| actor.charge().is_some()));
}

#[test]
fn quick_tap_between_fixed_ticks_retains_both_edges_and_casts_only_on_release() {
    let (mut app, _) = menu_app_at(480);
    app.world_mut().resource_mut::<ArenaSession>().bot_enabled = false;
    {
        let mut state = app.world_mut().resource_mut::<ViewState>();
        state.begin_play();
        state.suppress_click = false;
    }
    let initial_tick = app.world().resource::<ArenaSession>().tick;
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    app.update();
    clear_mouse_edges(&mut app);
    assert_eq!(app.world().resource::<ArenaSession>().tick, initial_tick);
    assert!(app.world().resource::<ArenaInput>().human.cast_pressed);
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .release(MouseButton::Left);
    app.update();
    clear_mouse_edges(&mut app);
    assert_eq!(app.world().resource::<ArenaSession>().tick, initial_tick);
    let input = &app.world().resource::<ArenaInput>().human;
    assert!(input.cast_pressed && input.cast_released && !input.cast_held);
    for _ in 0..3 {
        app.update();
    }
    let session = app.world().resource::<ArenaSession>();
    assert!(session.actors.first().is_some_and(|actor| {
        actor.charge().is_none()
            && actor
                .cooldowns
                .get(1)
                .is_some_and(|cooldown| *cooldown > 1.2)
    }));
    let input = &app.world().resource::<ArenaInput>().human;
    assert!(!input.cast_pressed && !input.cast_released && !input.cast_held);
}

#[test]
fn release_frame_aim_survives_camera_motion_before_the_next_fixed_tick() {
    for third_person in [false, true] {
        let (mut app, _) = menu_app_at(480);
        app.world_mut().resource_mut::<ArenaSession>().bot_enabled = false;
        {
            let mut state = app.world_mut().resource_mut::<ViewState>();
            state.begin_play();
            state.suppress_click = false;
            state.initialized = true;
            state.third_person = third_person;
            state.pitch = 0.25;
        }
        let initial_tick = app.world().resource::<ArenaSession>().tick;
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.update();
        clear_mouse_edges(&mut app);
        app.world_mut().resource_mut::<ViewState>().yaw = 0.3;
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
        app.update();
        clear_mouse_edges(&mut app);
        let released_aim = app.world().resource::<ArenaInput>().human.aim;
        assert_eq!(app.world().resource::<ArenaSession>().tick, initial_tick);
        assert!(app.world().resource::<ArenaInput>().human.cast_released);
        app.world_mut().resource_mut::<ViewState>().yaw = 1.3;
        app.update();
        assert_eq!(app.world().resource::<ArenaSession>().tick, initial_tick);
        assert!(
            app.world()
                .resource::<ArenaInput>()
                .human
                .aim
                .distance(released_aim)
                < 0.001
        );
        // Duration rounds 1/480 s to nanoseconds, so four render frames can
        // still fall just short of a physics tick. Allow up to six in total.
        for _ in 0..3 {
            if app.world().resource::<ArenaSession>().tick > initial_tick {
                break;
            }
            let input = &app.world().resource::<ArenaInput>().human;
            assert!(input.cast_released && input.aim.distance(released_aim) < 0.001);
            app.update();
        }
        let session = app.world().resource::<ArenaSession>();
        assert_eq!(session.tick, initial_tick + 1);
        let shot = session
            .projectiles
            .first()
            .expect("tap must release on the first physics tick");
        assert!(shot.velocity.normalize_or_zero().dot(released_aim) > 0.999);
        assert!(!app.world().resource::<ArenaInput>().human.cast_released);
        app.update();
        assert!(
            app.world()
                .resource::<ArenaInput>()
                .human
                .aim
                .distance(released_aim)
                > 0.2,
            "current aim resumes after physics consumes the release"
        );
    }
}

#[test]
fn selection_and_pause_discard_queued_release_aim_before_a_fixed_tick() {
    for key in [KeyCode::Digit1, KeyCode::Escape] {
        let (mut app, _) = menu_app_at(480);
        app.world_mut().resource_mut::<ArenaSession>().bot_enabled = false;
        {
            let mut state = app.world_mut().resource_mut::<ViewState>();
            state.begin_play();
            state.suppress_click = false;
            state.initialized = true;
            state.pitch = 0.25;
        }
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.update();
        clear_mouse_edges(&mut app);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
        app.update();
        clear_mouse_edges(&mut app);
        let released_aim = app.world().resource::<ArenaInput>().human.aim;
        app.world_mut().resource_mut::<ViewState>().yaw = 1.3;
        tap_key(&mut app, key);
        let input = &app.world().resource::<ArenaInput>().human;
        assert!(!input.cast_pressed && !input.cast_released);
        if key == KeyCode::Escape {
            tap_key(&mut app, KeyCode::Escape);
        }
        assert!(
            app.world()
                .resource::<ArenaInput>()
                .human
                .aim
                .distance(released_aim)
                > 0.2
        );
        for _ in 0..5 {
            app.update();
        }
        assert!(app
            .world()
            .resource::<ArenaSession>()
            .projectiles
            .is_empty());
        assert!(app
            .world()
            .resource::<ArenaSession>()
            .actors
            .iter()
            .all(|actor| actor.charge().is_none()));
    }
}

#[test]
fn held_charge_progress_uses_physics_time_at_all_render_rates() {
    for hz in [30, 60, 144, 240] {
        let mut app = app(hz);
        app.world_mut().resource_mut::<ArenaInput>().human = ActorIntent {
            cast_pressed: true,
            cast_held: true,
            aim: Vec3::X,
            ..default()
        };
        for _ in 0..hz / 2 {
            app.update();
        }
        let session = app.world().resource::<ArenaSession>();
        let actor = session.actors.first().expect("human actor");
        let charge = actor.charge().expect("held input arms the selected spell");
        assert!(
            (charge.elapsed - 0.5).abs() <= hex_arena::STEP * 2.0,
            "{hz} Hz produced {}s charge",
            charge.elapsed
        );
        assert!(session.projectiles.is_empty());
        assert!(actor
            .cooldowns
            .iter()
            .all(|cooldown| cooldown.abs() < 0.001));
        {
            let mut input = app.world_mut().resource_mut::<ArenaInput>();
            input.human.cast_held = false;
            input.human.cast_released = true;
        }
        for _ in 0..3 {
            app.update();
        }
        let session = app.world().resource::<ArenaSession>();
        assert!(session.actors.first().is_some_and(|actor| {
            actor.charge().is_none()
                && actor
                    .cooldowns
                    .get(1)
                    .is_some_and(|cooldown| *cooldown > 1.1)
        }));
        assert!(!app.world().resource::<ArenaInput>().human.cast_released);
    }
}

#[test]
fn fresh_press_can_arm_a_spell_selected_in_the_same_render_frame() {
    let (mut app, _) = menu_app();
    app.world_mut().resource_mut::<ArenaSession>().bot_enabled = false;
    tap_key(&mut app, KeyCode::Enter);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Digit1);
    charge_with_mouse(&mut app);
    let charge = app
        .world()
        .resource::<ArenaSession>()
        .actors
        .first()
        .and_then(|actor| actor.charge())
        .expect("fresh same-frame press");
    assert_eq!(charge.spell, Spell::Shield);
    assert!(app
        .world()
        .resource::<ArenaSession>()
        .projectiles
        .is_empty());
}

#[test]
fn active_holds_cancel_for_pause_focus_reset_selection_and_knockout() {
    for cancellation in ["escape", "tab", "focus", "reset", "selection", "knockout"] {
        let (mut app, window) = menu_app();
        app.world_mut().resource_mut::<ArenaSession>().bot_enabled = false;
        tap_key(&mut app, KeyCode::Enter);
        charge_with_mouse(&mut app);
        match cancellation {
            "escape" => tap_key(&mut app, KeyCode::Escape),
            "tab" => tap_key(&mut app, KeyCode::Tab),
            "reset" => tap_key(&mut app, KeyCode::KeyR),
            "selection" => tap_key(&mut app, KeyCode::Digit1),
            "focus" => {
                app.world_mut()
                    .get_mut::<Window>(window)
                    .expect("synthetic window")
                    .focused = false;
                app.update();
            }
            "knockout" => {
                app.world_mut()
                    .resource_mut::<ArenaSession>()
                    .actors
                    .first_mut()
                    .expect("human")
                    .hp = 0.0;
                app.update();
            }
            _ => unreachable!(),
        }
        let session = app.world().resource::<ArenaSession>();
        assert!(
            session.actors.iter().all(|actor| actor.charge().is_none()),
            "{cancellation} must cancel an active charge"
        );
        assert!(
            session.projectiles.is_empty(),
            "{cancellation} must not release a spell"
        );
        assert!(session.actors.iter().all(|actor| {
            actor
                .cooldowns
                .iter()
                .all(|cooldown| cooldown.abs() < 0.001)
        }));
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
        app.update();
        clear_mouse_edges(&mut app);
        assert!(
            app.world()
                .resource::<ArenaSession>()
                .projectiles
                .is_empty(),
            "release after {cancellation} must stay cancelled"
        );
        if cancellation == "selection" {
            charge_with_mouse(&mut app);
            assert!(app
                .world()
                .resource::<ArenaSession>()
                .actors
                .first()
                .and_then(|actor| actor.charge())
                .is_some_and(|charge| charge.spell == Spell::Shield));
        }
    }
}

#[test]
fn pausing_cancels_human_and_bot_charges_without_advancing_a_tick() {
    let (mut app, _) = menu_app();
    tap_key(&mut app, KeyCode::Enter);
    charge_with_mouse(&mut app);
    for _ in 0..600 {
        if app
            .world()
            .resource::<ArenaSession>()
            .actors
            .iter()
            .any(|actor| actor.id == 1 && actor.charge().is_some())
        {
            break;
        }
        app.update();
    }
    assert!(
        app.world()
            .resource::<ArenaSession>()
            .actors
            .iter()
            .all(|actor| actor.charge().is_some()),
        "fixture must catch both actors holding a charge"
    );
    let previous_tick = app.world().resource::<ArenaSession>().tick;
    tap_key(&mut app, KeyCode::Escape);
    let session = app.world().resource::<ArenaSession>();
    assert_eq!(session.tick, previous_tick);
    assert!(session.actors.iter().all(|actor| actor.charge().is_none()));
    let input = &app.world().resource::<ArenaInput>().human;
    assert!(!input.cast_pressed && !input.cast_released && !input.cast_held);
}

#[test]
fn paused_programmatic_frames_and_menu_buttons_cancel_charges_immediately() {
    for action in [
        hud::Action::Resume,
        hud::Action::Restart,
        hud::Action::Fullscreen,
        hud::Action::Quit,
    ] {
        let (mut app, _) = menu_app();
        app.world_mut().resource_mut::<ArenaSession>().bot_enabled = false;
        tap_key(&mut app, KeyCode::Enter);
        charge_with_mouse(&mut app);
        app.world_mut().resource_mut::<ViewState>().paused = true;
        press_action(&mut app, action);
        assert!(app
            .world()
            .resource::<ArenaSession>()
            .actors
            .iter()
            .all(|actor| actor.charge().is_none()));
        assert!(app
            .world()
            .resource::<ArenaSession>()
            .projectiles
            .is_empty());
    }
    let mut app = app(240);
    app.world_mut().resource_mut::<ArenaInput>().human = ActorIntent {
        cast_pressed: true,
        cast_held: true,
        ..default()
    };
    tick(&mut app);
    assert!(app
        .world()
        .resource::<ArenaSession>()
        .actors
        .first()
        .is_some_and(|actor| actor.charge().is_some()));
    app.world_mut().resource_mut::<ViewState>().paused = true;
    let previous_tick = app.world().resource::<ArenaSession>().tick;
    app.update();
    assert_eq!(app.world().resource::<ArenaSession>().tick, previous_tick);
    assert!(app
        .world()
        .resource::<ArenaSession>()
        .actors
        .iter()
        .all(|actor| actor.charge().is_none()));
}

#[test]
fn release_uses_current_camera_aim_in_first_and_third_person() {
    for third_person in [false, true] {
        let (mut app, _) = menu_app();
        app.world_mut().resource_mut::<ArenaSession>().bot_enabled = false;
        tap_key(&mut app, KeyCode::Enter);
        charge_with_mouse(&mut app);
        {
            let mut state = app.world_mut().resource_mut::<ViewState>();
            state.third_person = third_person;
            state.yaw += 0.5;
            state.pitch = 0.25;
        }
        let session = app.world().resource::<ArenaSession>();
        let state = app.world().resource::<ViewState>();
        let actor = session.actors.first().expect("human");
        let direction = aim(state);
        let camera = camera_origin(session, state, actor.eye(), direction);
        let expected = session
            .aim_from_camera(actor.id, camera, direction)
            .with_y(0.0)
            .normalize_or_zero();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
        app.update();
        let session = app.world().resource::<ArenaSession>();
        let shot = session
            .projectiles
            .first()
            .expect("released fireball must leave the open spawn");
        let actual = shot.velocity.with_y(0.0).normalize_or_zero();
        assert!(
            actual.dot(expected) > 0.999,
            "third={third_person}: {actual:?} != {expected:?}"
        );
    }
}

#[test]
fn clipped_shield_footprint_keeps_lowest_surviving_level_in_each_column() {
    use hex_core::{HexCoord, TilePos};
    let low = TilePos {
        coord: HexCoord::ORIGIN,
        level: 3,
    };
    let high = TilePos {
        coord: HexCoord::new_cubic(1, 0, -1),
        level: 7,
    };
    let clipped = [high.above(), low.above(), high, low, high.above().above()];
    let footprint = presentation::shield_footprint(&clipped);
    assert_eq!(footprint.len(), 2);
    assert!(footprint.contains(&low) && footprint.contains(&high));
}

#[test]
fn capture_charge_scenarios_are_driven_by_authoritative_input() {
    for (view, spell, expected_progress) in [
        ("shield-charge-partial-first", Spell::Shield, 0.52),
        ("shield-charge-full-third", Spell::Shield, 1.0),
        ("fireball-charge-partial-third", Spell::Fireball, 0.52),
        ("fireball-charge-full-first", Spell::Fireball, 1.0),
        ("blast-armed-first", Spell::AreaBlast, 0.87),
    ] {
        let mut app = app(60);
        for frame in 1..=capture_frame_index(view) {
            let direction = app
                .world()
                .resource::<ArenaSession>()
                .actors
                .first()
                .expect("human")
                .aim;
            let sample = capture_intent(
                frame,
                view,
                app.world().resource::<ArenaTuning>(),
                direction,
            );
            app.world_mut().resource_mut::<ArenaInput>().human = sample;
            app.update();
        }
        let session = app.world().resource::<ArenaSession>();
        let charge = session
            .actors
            .first()
            .and_then(|actor| actor.charge())
            .expect("capture must retain an authoritative charge");
        assert_eq!(charge.spell, spell);
        if spell != Spell::AreaBlast {
            assert!(
                (charge.elapsed / app.world().resource::<ArenaTuning>().charge_seconds
                    - expected_progress)
                    .abs()
                    < 0.03,
                "{view}: {}",
                charge.elapsed
            );
        }
        assert!(session.projectiles.is_empty());
    }
}

#[test]
fn partial_preview_capture_clips_cells_through_world_authority() {
    let mut app = app(60);
    {
        let mut state = app.world_mut().resource_mut::<ViewState>();
        state.capture = Some(PathBuf::from("unused-test-capture.png"));
        state.capture_view = "shield-partial-preview-first".into();
        state.frames = 0;
    }
    for _ in 0..90 {
        app.update();
    }
    let fixture = &app.world().resource::<ViewState>().capture_fixture_voxels;
    assert_eq!(
        fixture.len(),
        2,
        "fixture must place two occupied slots in one outer column"
    );
    let terrain = app.world().resource::<ArenaTerrainView>();
    assert!(fixture.iter().all(|pos| terrain.voxels.contains_key(pos)));
    let predicted = preview(
        app.world().resource::<ArenaSession>(),
        terrain,
        app.world().resource::<ArenaVoxelGeometry>(),
        app.world().resource::<ArenaTuning>(),
    );
    assert!(predicted.valid && !predicted.wall_voxels.is_empty());
    assert!(fixture
        .iter()
        .all(|pos| !predicted.wall_voxels.contains(pos)));
    let footprint = presentation::shield_footprint(&predicted.wall_voxels);
    let edge = fixture.first().expect("fixture edge");
    assert!(
        footprint
            .iter()
            .any(|pos| pos.coord == edge.coord && pos.level > edge.level + 1),
        "preview must keep the clipped column's higher surviving outline"
    );
}

#[cfg(feature = "test-support")]
#[test]
fn charge_bar_and_release_guidance_fit_below_crosshair_and_hide_when_cancelled() {
    use hex_ui::test_support::{ui_tree_snapshot, HeadlessUiPlugin};

    for (width, height) in [(1600, 900), (1280, 720)] {
        for (spell, ticks) in [
            (Spell::Shield, 60),
            (Spell::Fireball, 120),
            (Spell::AreaBlast, 60),
        ] {
            let mut fixture = app(120);
            fixture.world_mut().resource_mut::<ArenaInput>().human = ActorIntent {
                selected: Some(spell),
                cast_pressed: true,
                cast_held: true,
                ..default()
            };
            for _ in 0..ticks {
                tick(&mut fixture);
            }
            let session = fixture
                .world_mut()
                .remove_resource::<ArenaSession>()
                .expect("charged authority fixture");
            let charge = session
                .actors
                .first()
                .and_then(|actor| actor.charge())
                .expect("armed spell");
            let mut ui = App::new();
            ui.add_plugins(HeadlessUiPlugin::new(width, height))
                .insert_resource(ViewState {
                    started: true,
                    paused: false,
                    capture: None,
                    ..default()
                })
                .insert_resource(session)
                .init_resource::<ArenaTuning>()
                .add_systems(Startup, hud::setup)
                .add_systems(Update, hud::update);
            for _ in 0..8 {
                ui.update();
            }
            let snapshot = ui_tree_snapshot(ui.world_mut());
            let nodes = snapshot
                .nodes
                .iter()
                .filter(|node| node.name.starts_with("Charge "))
                .collect::<Vec<_>>();
            assert_eq!(nodes.len(), if spell == Spell::AreaBlast { 2 } else { 4 });
            let panel = nodes
                .iter()
                .find(|node| node.name == "Charge panel")
                .expect("visible charge panel");
            assert!((panel.center.x - snapshot.metrics.logical_size.x * 0.5).abs() < 1.0);
            for node in &nodes {
                let bounds = Rect::from_center_size(node.center, node.size);
                assert!(
                    node.fully_visible
                        && bounds.min.y > snapshot.metrics.logical_size.y * 0.5
                        && bounds.max.cmple(snapshot.metrics.logical_size).all(),
                    "{width}x{height}: {node:?}"
                );
            }
            let label = nodes
                .iter()
                .find(|node| node.name == "Charge guidance")
                .expect("release guidance");
            let glyphs = label
                .rendered_text_bounds
                .expect("release guidance has real glyphs");
            let panel_bounds = Rect::from_center_size(panel.center, panel.size);
            assert!(
                glyphs.min.cmpge(panel_bounds.min).all()
                    && glyphs.max.cmple(panel_bounds.max + Vec2::splat(1.0)).all()
            );
            let text = ui
                .world_mut()
                .query::<(&hud::Label, &Text)>()
                .iter(ui.world())
                .find_map(|(label, text)| {
                    matches!(label, hud::Label::Charge).then_some(text.0.clone())
                })
                .expect("charge label");
            assert!(text.contains("Release to cast"));
            if spell == Spell::AreaBlast {
                assert!(!text.contains('%'));
            } else {
                let track = nodes
                    .iter()
                    .find(|node| node.name == "Charge track")
                    .expect("charge track");
                let fill = nodes
                    .iter()
                    .find(|node| node.name == "Charge fill")
                    .expect("charge fill");
                let progress = charge.elapsed / ui.world().resource::<ArenaTuning>().charge_seconds;
                assert!((fill.size.x / track.size.x - progress).abs() < 0.01);
            }
            ui.world_mut().resource_mut::<ViewState>().pause();
            for _ in 0..2 {
                ui.update();
            }
            assert!(ui_tree_snapshot(ui.world_mut())
                .nodes
                .iter()
                .all(|node| !node.name.starts_with("Charge ")));
            ui.world_mut()
                .resource_mut::<ArenaSession>()
                .cancel_charges();
            ui.world_mut().resource_mut::<ViewState>().begin_play();
            for _ in 0..2 {
                ui.update();
            }
            assert!(ui_tree_snapshot(ui.world_mut())
                .nodes
                .iter()
                .all(|node| !node.name.starts_with("Charge ")));
        }
    }
}

#[cfg(feature = "test-support")]
#[test]
fn start_and_pause_controls_fit_computed_layout_at_supported_window_sizes() {
    use hex_ui::test_support::{ui_tree_snapshot, HeadlessUiPlugin};

    for (width, height, control) in [
        (1600, 900, hex_arena::ArenaControl::Player),
        (1280, 720, hex_arena::ArenaControl::Player),
        (1600, 900, hex_arena::ArenaControl::Spectator),
        (1280, 720, hex_arena::ArenaControl::Spectator),
    ] {
        let mut app = App::new();
        app.add_plugins(HeadlessUiPlugin::new(width, height))
            .insert_resource(ViewState {
                started: false,
                paused: true,
                capture: None,
                ..default()
            })
            .insert_resource(hex_arena::ArenaBattleSetup {
                control,
                rosters: vec![
                    hex_arena::TeamRoster::from_preset(1, hex_arena::BattlePreset::ShamanParty),
                    hex_arena::TeamRoster::from_preset(2, hex_arena::BattlePreset::Wisps12),
                ],
                ..default()
            })
            .init_resource::<ArenaSession>()
            .init_resource::<ArenaTuning>()
            .add_systems(Startup, hud::setup)
            .add_systems(Update, hud::update);
        for _ in 0..8 {
            app.update();
        }

        // Every interactive action belongs to an overlay; live play has no Menu button.
        let overlay = |world: &World, mut entity: Entity| loop {
            if world.get::<hud::StartPanel>(entity).is_some() {
                break "start";
            }
            if world.get::<hud::PausePanel>(entity).is_some() {
                break "pause";
            }
            entity = world
                .get::<ChildOf>(entity)
                .expect("arena actions must belong to start or pause panels")
                .parent();
        };
        let parameters = app
            .world_mut()
            .query::<(Entity, &hud::Label, &ChildOf)>()
            .iter(app.world())
            .filter_map(|(entity, label, parent)| match label {
                hud::Label::Parameter(index) => Some((entity, *index, parent.parent())),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(parameters.len(), 12);
        for (entity, index, row) in parameters {
            assert_eq!(overlay(app.world(), entity), "pause");
            app.world_mut()
                .entity_mut(entity)
                .insert(Name::new(format!("Arena pause parameter {index}")));
            app.world_mut()
                .entity_mut(row)
                .insert(Name::new(format!("Arena pause row {index}")));
        }
        let team_labels = app
            .world_mut()
            .query::<(Entity, &hud::Label)>()
            .iter(app.world())
            .filter_map(|(entity, label)| match label {
                hud::Label::Team(slot) => Some((entity, *slot)),
                _ => None,
            })
            .collect::<Vec<_>>();
        for (entity, slot) in team_labels {
            app.world_mut()
                .entity_mut(entity)
                .insert(Name::new(format!("Arena start team {slot}")));
        }
        let actions = app
            .world_mut()
            .query::<(Entity, &hud::Action, &Children)>()
            .iter(app.world())
            .map(|(entity, action, children)| {
                let phase = overlay(app.world(), entity);
                let label = match action {
                    hud::Action::Start => "start".into(),
                    hud::Action::Resume => "resume".into(),
                    hud::Action::Restart => "restart".into(),
                    hud::Action::Fullscreen => "fullscreen".into(),
                    hud::Action::Quit => "quit".into(),
                    hud::Action::Change(index, amount) => format!("change {index} {amount}"),
                    hud::Action::Control(control) => format!("control {control:?}"),
                    hud::Action::Roster(slot, step) => format!("roster {slot} {step}"),
                    hud::Action::Map(map) => format!("map {map:?}"),
                    hud::Action::Encounter(encounter) => format!("encounter {encounter:?}"),
                    hud::Action::PlayerRecipe(recipe) => format!("player recipe {recipe:?}"),
                };
                if phase == "start" {
                    assert!(!matches!(
                        action,
                        hud::Action::Resume | hud::Action::Restart | hud::Action::Change(..)
                    ));
                } else {
                    assert!(!matches!(action, hud::Action::Start));
                }
                (entity, phase, label, children.iter().collect::<Vec<_>>())
            })
            .collect::<Vec<_>>();
        let start_actions = actions
            .iter()
            .filter(|(_, phase, _, _)| *phase == "start")
            .count();
        let pause_actions = actions
            .iter()
            .filter(|(_, phase, _, _)| *phase == "pause")
            .count();
        assert_eq!(start_actions, 19); // Two modes, three maps, seven recipes, four roster arrows, three actions.
        assert_eq!(
            actions
                .iter()
                .filter(|(_, phase, label, _)| *phase == "start" && label == "start")
                .count(),
            1
        );
        assert_eq!(pause_actions, 28); // Twelve +/- pairs plus the four pause-menu actions.
        for required in ["resume", "restart", "fullscreen", "quit"] {
            assert_eq!(
                actions
                    .iter()
                    .filter(|(_, phase, label, _)| *phase == "pause" && label == required)
                    .count(),
                1
            );
        }
        for (entity, phase, label, children) in actions {
            let name = format!("Arena {phase} action {label}");
            app.world_mut()
                .entity_mut(entity)
                .insert(Name::new(name.clone()));
            assert_eq!(children.len(), 1, "each button has one text label");
            for child in children {
                app.world_mut()
                    .entity_mut(child)
                    .insert(Name::new(format!("{name} glyphs")));
            }
        }

        for (started, phase, expected) in [
            (
                false,
                "start",
                (start_actions
                    - if control == hex_arena::ArenaControl::Spectator {
                        7
                    } else {
                        4
                    })
                    * 2
                    + if control == hex_arena::ArenaControl::Spectator {
                        2
                    } else {
                        0
                    },
            ),
            (true, "pause", 24 + pause_actions * 2),
        ] {
            {
                let mut state = app.world_mut().resource_mut::<ViewState>();
                state.started = started;
                state.paused = true;
            }
            for _ in 0..8 {
                app.update();
            }
            let snapshot = ui_tree_snapshot(app.world_mut());
            let viewport = Rect::from_corners(Vec2::ZERO, snapshot.metrics.logical_size);
            let contains = |outer: Rect, inner: Rect| {
                let tolerance = Vec2::splat(1.0);
                (inner.min + tolerance).cmpge(outer.min).all()
                    && inner.max.cmple(outer.max + tolerance).all()
            };
            let observed = snapshot
                .nodes
                .iter()
                .filter(|node| node.name.starts_with("Arena "))
                .collect::<Vec<_>>();
            assert_eq!(
                observed.len(),
                expected,
                "wrong visible controls for {phase}"
            );
            for node in &observed {
                assert!(node.name.starts_with(&format!("Arena {phase} ")));
                let bounds = Rect::from_center_size(node.center, node.size);
                assert!(
                    node.size.cmpgt(Vec2::ZERO).all()
                        && node.fully_visible
                        && contains(viewport, bounds),
                    "{} must fit at {width}x{height}: {node:?}",
                    node.name
                );
                if node.name.contains(" parameter ")
                    || node.name.ends_with(" glyphs")
                    || node.name.contains(" start team ")
                {
                    let glyphs = node
                        .rendered_text_bounds
                        .expect("real text layout must produce visible glyphs");
                    assert!(
                        contains(viewport, glyphs) && contains(bounds, glyphs),
                        "{} glyphs must fit their node at {width}x{height}: {node:?}",
                        node.name
                    );
                }
            }
            if !started && control == hex_arena::ArenaControl::Player {
                let mut creatures = observed
                    .iter()
                    .filter(|node| {
                        (node.name.starts_with("Arena start action encounter ")
                            || node.name.starts_with("Arena start action player recipe "))
                            && !node.name.ends_with(" glyphs")
                    })
                    .map(|node| Rect::from_center_size(node.center, node.size))
                    .collect::<Vec<_>>();
                creatures.sort_by(|left, right| {
                    left.min
                        .y
                        .total_cmp(&right.min.y)
                        .then(left.min.x.total_cmp(&right.min.x))
                });
                assert_eq!(creatures.len(), 7, "seven Fort creature choices");
                for row in creatures.chunks(4) {
                    for pair in row.windows(2) {
                        let [left, right] = pair else { unreachable!() };
                        assert!((left.min.y - right.min.y).abs() < 0.5);
                        assert!(left.max.x <= right.min.x);
                    }
                }
                let upper = creatures.first().expect("upper creature row");
                let lower = creatures.last().expect("lower creature row");
                assert!(upper.max.y < lower.min.y, "two distinct creature rows");
            }
            let mut rows = observed
                .iter()
                .filter(|node| node.name.starts_with("Arena pause row "))
                .map(|node| Rect::from_center_size(node.center, node.size))
                .collect::<Vec<_>>();
            rows.sort_by(|left, right| left.min.y.total_cmp(&right.min.y));
            for pair in rows.windows(2) {
                let [upper, lower] = pair else { unreachable!() };
                assert!(
                    upper.max.y <= lower.min.y + 0.5,
                    "parameter rows overlap at {width}x{height}: {upper:?}, {lower:?}"
                );
            }
        }
        app.world_mut().resource_mut::<ViewState>().begin_play();
        for _ in 0..2 {
            app.update();
        }
        let live = ui_tree_snapshot(app.world_mut());
        assert!(
            live.nodes
                .iter()
                .all(|node| !node.name.starts_with("Arena ")),
            "live play must hide start/pause controls, including any Menu button"
        );
    }
}

#[path = "terrain_preservation_tests.rs"]
mod terrain_preservation_tests;

#[test]
fn trajectory_toggle_uses_new_selection_before_its_physics_tick() {
    let (mut app, _) = menu_app_at(480);
    tap_key(&mut app, KeyCode::Enter);
    app.world_mut().resource_mut::<ArenaSession>().bot_enabled = false;
    {
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.press(KeyCode::Digit1);
        keys.press(KeyCode::KeyT);
    }
    app.update();
    assert_eq!(app.world().resource::<ViewState>().previews, [false, false]);
}

#[path = "bot_evaluation_tests.rs"]
mod bot_evaluation_tests;

#[test]
fn bot_combat_capture_exercises_live_decisions_casts_and_terrain_publication() {
    let mut fixture = app(60);
    {
        let mut state = fixture.world_mut().resource_mut::<ViewState>();
        state.capture = Some(PathBuf::from("unused-bot-capture.png"));
        state.capture_view = "bot-combat-first".into();
    }
    for _ in 0..360 {
        fixture.update();
    }
    let session = fixture.world().resource::<ArenaSession>();
    assert!(session.bot_debug().decisions > 0);
    assert!(session
        .round_summary()
        .actors
        .get(1)
        .expect("bot stats")
        .casts
        .iter()
        .any(|count| *count > 0));
    assert!(session.terrain_outcomes > 0);
    assert!(fixture
        .world()
        .resource::<ViewState>()
        .tick_times
        .iter()
        .any(|(_, changed, _)| *changed));
}

#[test]
fn bot_combat_is_identical_across_render_batch_rates() {
    let mut reference = None;
    for hz in [30, 60, 144, 480] {
        let mut fixture = app(hz);
        fixture.world_mut().resource_mut::<ArenaReset>().generation = 1;
        tick(&mut fixture);
        {
            let mut state = fixture.world_mut().resource_mut::<ViewState>();
            state.reset_seen = 1;
            state.accumulator = 0.0;
        }
        fixture
            .world_mut()
            .resource_mut::<ArenaSession>()
            .bot_enabled = true;
        for frame in 0..u64::from(hz) * 2 {
            // Distribute nanosecond rounding so every cadence represents exactly
            // two seconds, rather than comparing 239 ticks with 240 at an edge.
            let nanos =
                (frame + 1) * 1_000_000_000 / u64::from(hz) - frame * 1_000_000_000 / u64::from(hz);
            fixture
                .world_mut()
                .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                    std::time::Duration::from_nanos(nanos),
                ));
            fixture.update();
        }
        let session = fixture.world().resource::<ArenaSession>();
        let snapshot = (
            session.tick,
            session
                .actors
                .iter()
                .map(|actor| {
                    (
                        actor.feet,
                        actor.aim,
                        actor.hp.to_bits(),
                        actor.cooldowns.map(f32::to_bits),
                        actor.charge(),
                    )
                })
                .collect::<Vec<_>>(),
            session
                .projectiles
                .iter()
                .map(|shot| (shot.owner, shot.spell, shot.position, shot.velocity))
                .collect::<Vec<_>>(),
            serde_json::to_value(session.bot_debug()).expect("serializable bot evidence"),
            serde_json::to_value(session.round_summary()).expect("serializable round evidence"),
        );
        if let Some(expected) = &reference {
            assert_eq!(
                &snapshot, expected,
                "render rate {hz} changed fixed simulation"
            );
        } else {
            reference = Some(snapshot);
        }
    }
}

#[test]
fn native_selection_defaults_and_invalid_capabilities_are_explicit() {
    assert_eq!(
        launch_selection(None, None).unwrap(),
        ArenaSelection {
            map: ArenaMap::Fort,
            encounter: ArenaEncounter::Dragon
        }
    );
    assert_eq!(
        launch_selection(Some("duel"), Some("shadow")).unwrap().map,
        ArenaMap::Duel
    );
    assert!(launch_selection(Some("seven-regions"), None).is_ok());
    assert!(launch_selection(Some("grand"), None).is_err());
    assert!(launch_selection(None, Some("unknown")).is_err());
}

#[test]
fn selector_actions_reset_frozen_input_and_do_not_change_an_active_run() {
    let (mut fixture, _) = menu_app();
    let initial_generation = fixture.world().resource::<ArenaReset>().generation;
    press_action(&mut fixture, hud::Action::Map(ArenaMap::Fort));
    assert_eq!(
        fixture.world().resource::<ArenaSelection>().map,
        ArenaMap::Fort
    );
    assert_eq!(
        fixture.world().resource::<ArenaTerrainView>().selection.map,
        ArenaMap::Fort
    );
    assert_eq!(
        fixture.world().resource::<ArenaReset>().generation,
        initial_generation + 1
    );
    assert!(!fixture.world().resource::<ViewState>().started);
    press_action(
        &mut fixture,
        hud::Action::Encounter(ArenaEncounter::Goblins),
    );
    assert_eq!(
        fixture.world().resource::<ArenaSelection>().encounter,
        ArenaEncounter::Goblins
    );
    let selection = *fixture.world().resource::<ArenaSelection>();
    press_action(&mut fixture, hud::Action::Start);
    let input = &fixture.world().resource::<ArenaInput>().human;
    assert!(!input.cast_pressed && !input.cast_held && !input.cast_released);
    tap_key(&mut fixture, KeyCode::Escape);
    press_action(&mut fixture, hud::Action::Map(ArenaMap::SevenRegions));
    assert_eq!(
        *fixture.world().resource::<ArenaSelection>(),
        selection,
        "map selection belongs to the ready screen"
    );
    press_action(&mut fixture, hud::Action::Restart);
    assert!(!fixture.world().resource::<ViewState>().started);
    assert_eq!(
        *fixture.world().resource::<ArenaSelection>(),
        selection,
        "restart preserves the recipe"
    );
    press_action(&mut fixture, hud::Action::Map(ArenaMap::Duel));
    press_action(&mut fixture, hud::Action::Encounter(ArenaEncounter::Dragon));
    assert_eq!(
        fixture.world().resource::<ArenaSelection>().encounter,
        ArenaEncounter::Dragon,
        "Duel also accepts the enemy party choice"
    );
    assert_eq!(fixture.world().resource::<ArenaSession>().actors.len(), 2);
    assert_eq!(
        fixture
            .world()
            .resource::<hex_arena::ArenaBattleSetup>()
            .player_recipe,
        Some(hex_arena::BattlePreset::Dragon)
    );
}

#[test]
fn actor_models_reconcile_removed_species_and_reset_generations() {
    use bevy::ecs::system::RunSystemOnce;
    let mut fixture = app(60);
    fixture
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();
    fixture
        .world_mut()
        .run_system_once(golem::setup)
        .expect("Golem cached visuals");
    fixture
        .world_mut()
        .run_system_once(wisp::setup)
        .expect("Wisp cached visuals");
    fixture
        .world_mut()
        .run_system_once(worm::setup)
        .expect("Worm cached visuals");
    let render = |fixture: &mut App| {
        fixture
            .world_mut()
            .run_system_once(presentation::actors)
            .expect("actor presentation");
        fixture
            .world_mut()
            .query_filtered::<Entity, With<presentation::ActorModel>>()
            .iter(fixture.world())
            .collect::<Vec<_>>()
    };
    let original = render(&mut fixture);
    assert_eq!(original.len(), 2);
    fixture
        .world_mut()
        .resource_mut::<ArenaSession>()
        .actors
        .get_mut(1)
        .expect("Duel fixture must contain an enemy actor")
        .species = hex_arena::Species::Goblin;
    let changed = render(&mut fixture);
    assert_eq!(changed.len(), 2);
    assert_eq!(
        original
            .iter()
            .filter(|entity| changed.contains(entity))
            .count(),
        1,
        "changed species needs a new body"
    );
    fixture
        .world_mut()
        .resource_mut::<ArenaSession>()
        .actors
        .pop();
    assert_eq!(render(&mut fixture).len(), 1);
    fixture.world_mut().resource_mut::<ArenaReset>().generation += 1;
    tick(&mut fixture);
    let reset = render(&mut fixture);
    assert_eq!(reset.len(), 2);
    assert!(
        reset.iter().all(|entity| !changed.contains(entity)),
        "reset clears every stale model"
    );
    assert_eq!(
        render(&mut fixture).len(),
        2,
        "reconciliation does not duplicate models"
    );
}

#[test]
fn encounter_hud_reveals_only_whole_party_completion() {
    use bevy::ecs::system::RunSystemOnce;
    let (mut fixture, _) = menu_app();
    press_action(&mut fixture, hud::Action::Map(ArenaMap::Fort));
    press_action(
        &mut fixture,
        hud::Action::Encounter(ArenaEncounter::Goblins),
    );
    fixture
        .world_mut()
        .resource_mut::<ArenaSession>()
        .bot_enabled = false;
    fixture
        .world_mut()
        .run_system_once(hud::setup)
        .expect("HUD setup");
    let label = |fixture: &mut App| {
        fixture
            .world_mut()
            .run_system_once(hud::update)
            .expect("HUD update");
        fixture
            .world_mut()
            .query::<(&hud::Label, &Text)>()
            .iter(fixture.world())
            .find_map(|(label, text)| {
                matches!(label, hud::Label::Encounter).then(|| text.0.clone())
            })
            .expect("encounter label")
    };
    let initial = label(&mut fixture);
    assert!(initial.contains("0 / 1 parties cleared"));
    // An unseen member can die before the party is cleared. Its live roster size
    // and remaining HP must not become a normal-HUD observation.
    fixture
        .world_mut()
        .resource_mut::<ArenaSession>()
        .actors
        .get_mut(1)
        .expect("Fort Goblins fixture must contain an enemy actor")
        .hp = 0.0;
    tick(&mut fixture);
    assert_eq!(label(&mut fixture), initial);
    for actor in fixture
        .world_mut()
        .resource_mut::<ArenaSession>()
        .actors
        .iter_mut()
        .skip(1)
    {
        actor.hp = 0.0;
    }
    tick(&mut fixture);
    assert!(label(&mut fixture).contains("1 / 1 parties cleared"));
}

#[test]
fn frame_wall_timing_is_independent_from_manual_simulation_delta() {
    let mut state = ViewState::default();
    let start = std::time::Instant::now();
    state.record_frame_timing(start, 1.0 / 60.0);
    state.record_frame_timing(start + std::time::Duration::from_millis(40), 1.0 / 60.0);
    assert_eq!(state.frame_wall_intervals.len(), 1);
    let interval = state
        .frame_wall_intervals
        .first()
        .expect("two frame starts must produce one wall interval");
    assert!((*interval - 40.0).abs() < 0.001);
    assert!(state
        .frame_times
        .iter()
        .all(|dt| (*dt - 1000.0 / 60.0).abs() < 0.001));
    let mut fixture = app(60);
    fixture.world_mut().resource_mut::<ViewState>().pause();
    fixture.update();
    let paused = fixture.world().resource::<ViewState>();
    assert!(paused
        .simulation_frame_times
        .last()
        .is_some_and(|dt| dt.abs() < 0.001));
    assert!(paused.frame_times.last().is_some_and(|dt| *dt > 16.0));
}

#[test]
fn synthetic_stress_changes_require_explicit_capture_and_retain_tick_activity() {
    let (mut fixture, _) = menu_app();
    encounter::configure_encounter_stress_tuning(
        true,
        "encounter-stress",
        &mut fixture.world_mut().resource_mut::<ArenaTuning>(),
    )
    .expect("validated synthetic leashes before Fort admission");
    press_action(&mut fixture, hud::Action::Map(ArenaMap::Fort));
    {
        let mut state = fixture.world_mut().resource_mut::<ViewState>();
        state.capture_view = "encounter-stress".into();
        state.started = true;
        state.paused = false;
    }
    let original = fixture
        .world()
        .resource::<ArenaSession>()
        .actors
        .first()
        .expect("human")
        .feet;
    assert!(encounter::prepare_stress_tick(fixture.world_mut()).is_none());
    assert_eq!(
        fixture
            .world()
            .resource::<ArenaSession>()
            .actors
            .first()
            .expect("human")
            .feet,
        original
    );
    assert!(fixture
        .world()
        .resource::<ArenaSession>()
        .actors
        .iter()
        .all(|actor| actor.max_hp < 1000.0));
    fixture.world_mut().resource_mut::<ViewState>().capture =
        Some(PathBuf::from("unused-synthetic-stress.png"));
    for _ in 0..70 {
        fixture.update();
    }
    let state = fixture.world().resource::<ViewState>();
    assert!(state.capture_stress_initialized && state.capture_stress_steps >= 120);
    assert_eq!(
        state.capture_stress_ticks.len(),
        usize::try_from(state.capture_stress_steps).expect("bounded capture ticks")
    );
    assert!(state
        .capture_stress_ticks
        .iter()
        .all(|row| row.frame > 0 && row.tick > 0 && row.cpu_ms >= 0.0));
    let session = fixture.world().resource::<ArenaSession>();
    assert!(session.actors.iter().all(|actor| actor.max_hp > 99_999.0));
    assert!(session.outcome.is_none());
}

#[test]
fn goblin_swipe_capture_walks_the_fort_detour_and_reaches_a_real_windup() {
    let mut fixture = app(60);
    *fixture.world_mut().resource_mut::<ArenaSelection>() = ArenaSelection {
        map: ArenaMap::Fort,
        encounter: ArenaEncounter::Goblins,
    };
    fixture.world_mut().resource_mut::<ArenaReset>().generation += 1;
    {
        let mut state = fixture.world_mut().resource_mut::<ViewState>();
        state.capture = Some("capture-adapter-test-no-screenshot-system.png".into());
        state.capture_view = "encounter-swipe".into();
    }
    for _ in 0..1800 {
        fixture.update();
        if fixture
            .world()
            .resource::<ViewState>()
            .capture_event_frame
            .is_some()
        {
            break;
        }
    }
    let state = fixture.world().resource::<ViewState>();
    let session = fixture.world().resource::<ArenaSession>();
    assert!(
        state.capture_route_step > 0,
        "capture must take the gate/keep detour"
    );
    assert!(
        state.capture_event_frame.is_some(),
        "swipe capture stalled at waypoint {}: {:?}",
        state.capture_route_step,
        session
            .actors
            .iter()
            .map(|actor| (actor.id, actor.feet, actor.hp))
            .collect::<Vec<_>>()
    );
    assert!(encounter::phase_ready(session, "encounter-swipe"));
    assert!(session.actors.first().is_some_and(|actor| actor.hp > 0.0));
}

#[test]
fn fort_dragon_actor_camera_scripts_reach_a_visible_subject_after_the_keep_detour() {
    use bevy::ecs::system::RunSystemOnce;
    for (view, third_person) in [("encounter-first", false), ("encounter-third", true)] {
        let mut fixture = app(60);
        *fixture.world_mut().resource_mut::<ArenaSelection>() = ArenaSelection {
            map: ArenaMap::Fort,
            encounter: ArenaEncounter::Dragon,
        };
        fixture.world_mut().resource_mut::<ArenaReset>().generation += 1;
        {
            let mut state = fixture.world_mut().resource_mut::<ViewState>();
            state.capture = Some("camera-adapter-test-no-screenshot-system.png".into());
            state.capture_view = view.into();
            state.third_person = third_person;
        }
        let camera = fixture
            .world_mut()
            .spawn((ArenaCamera, Transform::default()))
            .id();
        let mut subjects = Vec::new();
        for _ in 0..1800 {
            fixture.update();
            fixture
                .world_mut()
                .run_system_once(presentation::camera)
                .expect("actor camera projection");
            if encounter::fort_approach_complete(
                fixture.world().resource::<ViewState>().capture_route_step,
            ) {
                subjects = encounter::visible_subjects(
                    fixture.world().resource::<ArenaSession>(),
                    fixture
                        .world()
                        .get::<Transform>(camera)
                        .expect("camera pose"),
                    view,
                );
            }
            if !subjects.is_empty() || fixture.world().resource::<ArenaSession>().outcome.is_some()
            {
                break;
            }
        }
        assert!(
            !subjects.is_empty(),
            "{view} never composed a visible Dragon after the Fort approach; actors {:?}",
            fixture
                .world()
                .resource::<ArenaSession>()
                .actors
                .iter()
                .map(|actor| (actor.id, actor.feet, actor.hp))
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn fort_capture_scripts_reach_requested_phases_after_the_keep_detour() {
    for (encounter, view) in [
        (ArenaEncounter::Dragon, "encounter-windup"),
        (ArenaEncounter::Dragon, "encounter-breath"),
        (ArenaEncounter::Dragon, "encounter-barrier"),
        (ArenaEncounter::ShamanParty, "encounter-fireball"),
        (ArenaEncounter::ShamanParty, "encounter-aura"),
    ] {
        let mut fixture = app(60);
        *fixture.world_mut().resource_mut::<ArenaSelection>() = ArenaSelection {
            map: ArenaMap::Fort,
            encounter,
        };
        fixture.world_mut().resource_mut::<ArenaReset>().generation += 1;
        {
            let mut state = fixture.world_mut().resource_mut::<ViewState>();
            state.capture = Some("phase-adapter-test-no-screenshot-system.png".into());
            state.capture_view = view.into();
        }
        for _ in 0..1800 {
            fixture.update();
            if fixture
                .world()
                .resource::<ViewState>()
                .capture_event_frame
                .is_some()
                || fixture.world().resource::<ArenaSession>().outcome.is_some()
            {
                break;
            }
        }
        let state = fixture.world().resource::<ViewState>();
        let session = fixture.world().resource::<ArenaSession>();
        assert!(
            state.capture_event_frame.is_some(),
            "{view} failed at approach waypoint{}; actors {:?}",
            state.capture_route_step,
            session
                .actors
                .iter()
                .map(|actor| (actor.id, actor.feet, actor.hp))
                .collect::<Vec<_>>()
        );
        assert!(encounter::fort_approach_complete(state.capture_route_step));
        assert!(encounter::phase_ready(session, view));
    }
}

#[path = "spectator_tests.rs"]
mod spectator_tests;

#[path = "golem_tests.rs"]
mod golem_tests;

#[path = "wisp_tests.rs"]
mod wisp_tests;
#[path = "worm_tests.rs"]
mod worm_tests;

#[path = "duel_party_tests.rs"]
mod duel_party_tests;

#[path = "terminal_menu_tests.rs"]
mod terminal_menu_tests;
