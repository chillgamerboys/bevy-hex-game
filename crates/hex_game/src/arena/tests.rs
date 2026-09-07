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

fn menu_app() -> (App, Entity) {
    let mut builder = HeadlessAppBuilder::new()
        .with_minimal_plugins()
        .with_fixed_step(std::time::Duration::from_secs_f64(1.0 / 60.0));
    builder
        .app_mut()
        .insert_resource(ViewState {
            capture: None,
            ..default()
        })
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ButtonInput<MouseButton>>()
        .add_message::<CursorMoved>()
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
    app.world_mut().resource_mut::<ArenaInput>().human.cast = true;
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
    assert!(!app.world().resource::<ArenaInput>().human.cast);
    tap_key(&mut app, KeyCode::Escape);
    app.world_mut().resource_mut::<ArenaInput>().human.cast = true;
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
    assert!(
        app.world()
            .resource::<ArenaSession>()
            .actors
            .first()
            .is_some_and(|actor| actor
                .cooldowns
                .get(1)
                .is_some_and(|cooldown| *cooldown > 1.0)),
        "a fresh gameplay click after release must still cast"
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
        assert!(!app.world().resource::<ArenaInput>().human.cast);
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
        assert!(session
            .actors
            .iter()
            .all(|actor| (actor.hp - 100.0).abs() < 0.001
                && actor
                    .cooldowns
                    .iter()
                    .all(|cooldown| cooldown.abs() < 0.001)));
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
        cast: true,
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
    assert_eq!(forecast.wall_voxels.len(), 25);
    app.world_mut().resource_mut::<ArenaInput>().human.cast = true;
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
            cast: true,
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
        assert!(!app.world().resource::<ArenaInput>().human.cast);
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
    app.world_mut().resource_mut::<ArenaInput>().human.cast = true;
    app.update();
    assert!(app.world().resource::<ViewState>().paused);
    assert!(!app.world().resource::<ArenaInput>().human.cast);
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
                cast: true,
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

#[cfg(feature = "test-support")]
#[test]
fn start_and_pause_controls_fit_computed_layout_at_supported_window_sizes() {
    use hex_ui::test_support::{ui_tree_snapshot, HeadlessUiPlugin};

    for (width, height) in [(1600, 900), (1280, 720)] {
        let mut app = App::new();
        app.add_plugins(HeadlessUiPlugin::new(width, height))
            .insert_resource(ViewState {
                started: false,
                paused: true,
                capture: None,
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
        assert_eq!(start_actions, 3);
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
            (false, "start", start_actions * 2),
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
                if node.name.contains(" parameter ") || node.name.ends_with(" glyphs") {
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
