//! Observer presentation contracts: actual setup, camera-only input, and terminal/reset UI.
use super::*;
use crate::arena::spectator::{self, ObserverCamera, ObserverCameraMode};
use bevy::ecs::system::RunSystemOnce;
use hex_arena::{ArenaBattleSetup, ArenaControl, BattlePreset, BattleResult, BattleSummary};

#[test]
fn observer_launch_rejects_player_options_and_unsupported_map() {
    assert!(
        spectator::launch_setup(ArenaMap::Duel, false, Some("dragon"), None, None, None).is_err()
    );
    assert!(spectator::launch_setup(ArenaMap::SevenRegions, true, None, None, None, None).is_err());
    assert!(spectator::launch_setup(ArenaMap::Fort, true, None, None, None, Some("0")).is_err());
    assert!(spectator::launch_setup(ArenaMap::Fort, true, None, None, Some("-1"), None).is_err());
    let setup = spectator::launch_setup(
        ArenaMap::Duel,
        true,
        Some("goblins"),
        Some("shaman-party"),
        Some("42"),
        Some("720"),
    )
    .expect("valid explicit observer recipe");
    assert_eq!(setup.seed, 42);
    assert_eq!(setup.tick_limit, Some(720));
    assert_eq!(
        spectator::preset_for(&setup, 0),
        Some(BattlePreset::Goblins)
    );
    assert_eq!(
        spectator::preset_for(&setup, 1),
        Some(BattlePreset::ShamanParty)
    );
}

#[test]
fn observer_camera_switches_without_pose_jump_and_clamps_zoom_and_pitch() {
    let mut camera = ObserverCamera::default();
    camera.reset(7, Transform::from_xyz(10.0, 12.0, 10.0), Vec3::ZERO);
    let initial = camera.position;
    let direction = camera.direction();
    camera.toggle_mode();
    let pose = camera.update_pose(Vec2::ZERO, Vec3::ZERO, false, 0.0, 0.0);
    assert!(pose.translation.distance(initial) < 0.001);
    assert!(Vec3::from(pose.forward()).dot(direction) > 0.999);
    camera.toggle_mode();
    let pose = camera.update_pose(Vec2::ZERO, Vec3::ZERO, false, 0.0, 0.0);
    assert!(pose.translation.distance(initial) < 0.001);
    camera.update_pose(Vec2::splat(100_000.0), Vec3::ZERO, false, 1000.0, 0.0);
    assert!((camera.distance - 3.0).abs() < f32::EPSILON);
    assert!(camera.pitch.abs() <= 1.48);
    camera.update_pose(Vec2::ZERO, Vec3::ZERO, false, -1000.0, 0.0);
    assert!((camera.distance - 120.0).abs() < f32::EPSILON);
    assert!(camera.position.is_finite());
}

#[test]
fn observer_menu_resets_roster_and_returns_to_player_with_human_identity() {
    let (mut fixture, _) = menu_app();
    press_action(&mut fixture, hud::Action::Map(ArenaMap::SevenRegions));
    press_action(&mut fixture, hud::Action::Control(ArenaControl::Spectator));
    assert_eq!(
        fixture.world().resource::<ArenaSelection>().map,
        ArenaMap::Fort
    );
    assert_eq!(
        fixture.world().resource::<ArenaSession>().human_actor_id(),
        None
    );
    assert!(fixture
        .world()
        .resource::<ArenaSession>()
        .battle_summary()
        .is_some());
    let generation = fixture.world().resource::<ArenaReset>().generation;
    press_action(&mut fixture, hud::Action::Roster(0, 1));
    assert_eq!(
        fixture.world().resource::<ArenaReset>().generation,
        generation + 1
    );
    let accepted = fixture
        .world()
        .resource::<ArenaSession>()
        .accepted_battle_setup()
        .clone();
    assert_eq!(accepted, *fixture.world().resource::<ArenaBattleSetup>());
    press_action(&mut fixture, hud::Action::Map(ArenaMap::SevenRegions));
    assert_eq!(
        fixture.world().resource::<ArenaSelection>().map,
        ArenaMap::Fort
    );
    press_action(&mut fixture, hud::Action::Start);
    press_action(&mut fixture, hud::Action::Roster(1, 1));
    assert_eq!(accepted, *fixture.world().resource::<ArenaBattleSetup>());
    tap_key(&mut fixture, KeyCode::KeyR);
    assert!(!fixture.world().resource::<ViewState>().started);
    assert_eq!(
        accepted,
        *fixture
            .world()
            .resource::<ArenaSession>()
            .accepted_battle_setup()
    );
    press_action(&mut fixture, hud::Action::Control(ArenaControl::Player));
    assert_eq!(
        fixture.world().resource::<ArenaSession>().human_actor_id(),
        Some(0)
    );
    assert!(fixture
        .world()
        .resource::<ArenaSession>()
        .battle_summary()
        .is_none());
}

#[test]
fn observer_input_remains_neutral_and_focus_pause_freezes_battle() {
    let (mut fixture, window) = menu_app();
    press_action(&mut fixture, hud::Action::Control(ArenaControl::Spectator));
    fixture
        .world_mut()
        .resource_mut::<ViewState>()
        .observer
        .reset(0, Transform::from_xyz(12.0, 14.0, 12.0), Vec3::ZERO);
    press_action(&mut fixture, hud::Action::Start);
    fixture
        .world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyW);
    fixture
        .world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Space);
    fixture
        .world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Digit2);
    fixture
        .world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    fixture.update();
    let input = fixture.world().resource::<ArenaInput>().human;
    assert_eq!(input.movement, Vec2::ZERO);
    assert!(
        !input.jump
            && !input.run
            && !input.cast_pressed
            && !input.cast_held
            && !input.cast_released
    );
    assert_eq!(input.selected, None);
    fixture
        .world_mut()
        .get_mut::<Window>(window)
        .expect("window")
        .focused = false;
    fixture.update();
    let before = combat_snapshot(&fixture);
    for _ in 0..12 {
        fixture.update();
    }
    assert_eq!(before, combat_snapshot(&fixture));
    assert!(fixture.world().resource::<ViewState>().paused);
    assert!(
        fixture
            .world()
            .get::<CursorOptions>(window)
            .expect("cursor")
            .visible
    );
}

#[test]
fn observer_actor_zero_is_visible_and_player_hud_is_absent() {
    let (mut fixture, _) = menu_app();
    press_action(&mut fixture, hud::Action::Control(ArenaControl::Spectator));
    press_action(&mut fixture, hud::Action::Start);
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
        .run_system_once(presentation::actors)
        .expect("observer models");
    let visible_zero = fixture
        .world_mut()
        .query::<(&Name, &Visibility)>()
        .iter(fixture.world())
        .any(|(name, visibility)| {
            name.as_str() == "Arena actor 0" && *visibility == Visibility::Visible
        });
    assert!(
        visible_zero,
        "actor zero is an ordinary visible monster in spectator mode"
    );
    fixture
        .world_mut()
        .run_system_once(hud::setup)
        .expect("HUD setup");
    fixture
        .world_mut()
        .run_system_once(hud::update)
        .expect("observer HUD");
    for (node, combat, observer) in fixture
        .world_mut()
        .query::<(&Node, Has<hud::CombatHud>, Has<hud::ObserverHud>)>()
        .iter(fixture.world())
    {
        if combat {
            assert_eq!(node.display, Display::None);
        }
        if observer {
            assert_eq!(node.display, Display::Flex);
        }
    }
}

#[test]
fn observer_terminal_text_distinguishes_timeout_draw_and_winner() {
    let mut summary = BattleSummary {
        seed: 91,
        ticks: 720,
        seconds: 6.0,
        teams: Vec::new(),
        result: Some(BattleResult::Timeout),
    };
    assert!(
        spectator::battle_status(&summary, ObserverCameraMode::Orbit, false)
            .contains("TIME LIMIT / NO WINNER")
    );
    summary.result = Some(BattleResult::Draw);
    assert!(
        spectator::battle_status(&summary, ObserverCameraMode::Free, false)
            .contains("BOTH TEAMS ELIMINATED")
    );
    summary.result = Some(BattleResult::TeamWinner(2));
    assert!(
        spectator::battle_status(&summary, ObserverCameraMode::Orbit, false)
            .contains("TEAM 2 WINS")
    );
}

#[test]
fn observer_capture_waits_for_actual_timeout_and_records_only_neutral_input() {
    let mut fixture = app(60);
    let mut setup = ArenaBattleSetup::spectator(BattlePreset::Shadow, BattlePreset::Shadow, 13);
    setup.tick_limit = Some(30);
    fixture.insert_resource(setup);
    fixture.world_mut().resource_mut::<ArenaReset>().generation += 1;
    {
        let mut state = fixture.world_mut().resource_mut::<ViewState>();
        state.capture = Some(PathBuf::from("unused-observer-test.png"));
        state.capture_view = "observer-result".into();
        state.frames = 0;
    }
    for _ in 0..50 {
        fixture.update();
    }
    let summary = fixture
        .world()
        .resource::<ArenaSession>()
        .battle_summary()
        .expect("observer summary");
    assert_eq!(summary.result, Some(BattleResult::Timeout));
    assert_eq!(summary.ticks, 30);
    let state = fixture.world().resource::<ViewState>();
    assert!(state.capture_event_frame.is_some());
    assert!(state
        .capture_inputs
        .iter()
        .all(|(_, input)| input.movement == Vec2::ZERO
            && !input.cast_pressed
            && !input.cast_released
            && !input.cast_held));
}

#[test]
fn invalid_observer_setup_does_not_repeat_ready_initialization_ticks() {
    let (mut fixture, _) = menu_app();
    let mut setup = ArenaBattleSetup::spectator(BattlePreset::Shadow, BattlePreset::Dragon, 1);
    setup.rosters.clear();
    fixture.insert_resource(setup);
    fixture.world_mut().resource_mut::<ArenaReset>().generation += 1;
    fixture.update();
    assert!(fixture.world().resource::<ArenaSession>().is_finished());
    assert!(fixture.world().resource::<ArenaSession>().actors.is_empty());
    let rows = fixture.world().resource::<ViewState>().tick_times.len();
    for _ in 0..8 {
        fixture.update();
    }
    assert_eq!(
        fixture.world().resource::<ViewState>().tick_times.len(),
        rows
    );
}

#[test]
fn observer_camera_remains_available_after_timeout_until_pause() {
    let (mut fixture, window) = menu_app();
    let mut setup = ArenaBattleSetup::spectator(BattlePreset::Shadow, BattlePreset::Shadow, 1);
    setup.tick_limit = Some(3);
    fixture.insert_resource(setup);
    fixture.world_mut().resource_mut::<ArenaReset>().generation += 1;
    fixture.update();
    fixture
        .world_mut()
        .resource_mut::<ViewState>()
        .observer
        .reset(1, Transform::from_xyz(12.0, 14.0, 12.0), Vec3::ZERO);
    press_action(&mut fixture, hud::Action::Start);
    for _ in 0..5 {
        fixture.update();
    }
    assert!(fixture.world().resource::<ArenaSession>().is_finished());
    assert!(
        !fixture
            .world()
            .get::<CursorOptions>(window)
            .expect("cursor")
            .visible
    );
    tap_key(&mut fixture, KeyCode::KeyC);
    assert_eq!(
        fixture.world().resource::<ViewState>().observer.mode,
        ObserverCameraMode::Free
    );
    tap_key(&mut fixture, KeyCode::Escape);
    assert!(
        fixture
            .world()
            .get::<CursorOptions>(window)
            .expect("cursor")
            .visible
    );
    assert!(fixture.world().resource::<ViewState>().paused);
}

#[test]
fn observer_default_camera_moves_close_while_map_review_keeps_whole_footprint_pose() {
    let (mut fixture, _) = menu_app();
    press_action(&mut fixture, hud::Action::Control(ArenaControl::Spectator));
    let camera = fixture
        .world_mut()
        .spawn((ArenaCamera, Transform::default()))
        .id();
    for (view, capture, expected_distance) in [
        ("first", false, Some(18.0)),
        ("observer-orbit", true, None),
        ("observer-close", true, Some(18.0)),
    ] {
        {
            let mut state = fixture.world_mut().resource_mut::<ViewState>();
            state.capture = capture.then(|| PathBuf::from("unused-camera-check.png"));
            state.capture_view = view.into();
            state.frames = 1;
            state.observer.generation = None;
        }
        fixture
            .world_mut()
            .run_system_once(spectator::camera)
            .expect("observer camera");
        let state = fixture.world().resource::<ViewState>();
        if let Some(distance) = expected_distance {
            assert!((state.observer.distance - distance).abs() < 0.01);
        } else {
            assert!(state.observer.distance > 30.0);
        }
        let position = fixture
            .world()
            .get::<Transform>(camera)
            .expect("camera pose")
            .translation;
        assert!(position.is_finite());
        let subjects = fixture
            .world()
            .resource::<ArenaSession>()
            .actors
            .iter()
            .map(|actor| actor.center())
            .collect::<Vec<_>>();
        let center = subjects.iter().copied().sum::<Vec3>()
            / f32::from(u16::try_from(subjects.len()).expect("bounded admitted roster"));
        assert!(state.observer.target.distance(center) < 0.001);
    }
}

#[test]
fn close_capture_recipes_zoom_and_rotate_using_camera_controls_only() {
    let session = ArenaSession::default();
    let mut poses = Vec::new();
    for rear in [false, true] {
        let mut camera = ObserverCamera::default();
        camera.reset(0, Transform::from_xyz(18.0, 18.0, 18.0), Vec3::ZERO);
        spectator::focus_live(&mut camera, &session);
        let initial_yaw = camera.yaw;
        for frame in 5..110 {
            let sample = spectator::close_controls(&camera, &session, frame, rear);
            camera.update_pose(
                Vec2::from_array(sample.look),
                Vec3::from_array(sample.movement),
                false,
                sample.wheel,
                1.0 / 60.0,
            );
        }
        assert!((camera.distance - 10.0).abs() < 0.01);
        assert!((camera.pitch + 0.9).abs() < 0.001);
        let rotation = camera.yaw - initial_yaw;
        assert!((rotation - if rear { std::f32::consts::PI } else { 0.0 }).abs() < 0.001);
        poses.push(camera.position);
    }
    let front = poses.first().expect("front pose");
    let rear = poses.get(1).expect("rear pose");
    assert!(front.with_y(0.0).dot(rear.with_y(0.0)) < 0.0);
    assert!(!spectator::both_teams_visible(&session, &[]));
}

#[cfg(feature = "test-support")]
#[test]
fn observer_footer_text_is_centered_and_padded_at_supported_sizes() {
    use hex_ui::test_support::{ui_tree_snapshot, HeadlessUiPlugin};
    for (width, height) in [(1600, 900), (1280, 720)] {
        let (mut fixture, _) = menu_app();
        press_action(&mut fixture, hud::Action::Control(ArenaControl::Spectator));
        let session = fixture
            .world_mut()
            .remove_resource::<ArenaSession>()
            .expect("accepted observer session");
        let mut ui = App::new();
        ui.add_plugins(HeadlessUiPlugin::new(width, height))
            .insert_resource(session)
            .init_resource::<ArenaTuning>()
            .insert_resource(ViewState {
                started: true,
                paused: false,
                capture: None,
                ..default()
            })
            .add_systems(Startup, hud::setup)
            .add_systems(Update, hud::update);
        for _ in 0..8 {
            ui.update();
        }
        let snapshot = ui_tree_snapshot(ui.world_mut());
        let text = snapshot
            .nodes
            .iter()
            .find(|node| node.name == "Observer footer text")
            .expect("visible observer guidance");
        let glyphs = text.rendered_text_bounds.expect("real footer glyphs");
        let panel = snapshot
            .nodes
            .iter()
            .find(|node| node.name == "Observer footer panel")
            .expect("visible dark guidance panel");
        let panel_bounds = Rect::from_center_size(panel.center, panel.size);
        assert!(panel.fully_visible);
        assert!(glyphs.min.x >= panel_bounds.min.x + 10.0);
        assert!(glyphs.max.x <= panel_bounds.max.x - 10.0);
        assert!(glyphs.min.y >= panel_bounds.min.y + 4.0);
        assert!(glyphs.max.y <= panel_bounds.max.y - 4.0);
        let dark_panel = ui
            .world_mut()
            .query::<(&Name, &BackgroundColor)>()
            .iter(ui.world())
            .find(|(name, _)| name.as_str() == "Observer footer panel")
            .map(|(_, color)| color.0.to_srgba())
            .expect("panel owns a real background");
        assert!(dark_panel.red < 0.1 && dark_panel.green < 0.1 && dark_panel.blue < 0.1);
        assert!(dark_panel.alpha >= 0.75);
        assert!(text.fully_visible);
        assert!(glyphs.min.x >= 18.0 && glyphs.max.x <= snapshot.metrics.logical_size.x - 18.0);
        assert!((text.center.x - snapshot.metrics.logical_size.x * 0.5).abs() < 1.0);
        assert!(glyphs.max.y < snapshot.metrics.logical_size.y - 10.0);
    }
}

#[test]
fn close_observer_routes_reach_both_real_teams_without_actor_staging() {
    for map in [ArenaMap::Duel, ArenaMap::Fort] {
        for view in ["observer-close", "observer-close-rear"] {
            let mut fixture = app(60);
            let (left, right) = if map == ArenaMap::Fort {
                (BattlePreset::Goblins, BattlePreset::ShamanParty)
            } else {
                (BattlePreset::Shadow, BattlePreset::Dragon)
            };
            fixture.insert_resource(ArenaBattleSetup::spectator(left, right, 1));
            fixture.world_mut().resource_mut::<ArenaSelection>().map = map;
            fixture.world_mut().resource_mut::<ArenaReset>().generation += 1;
            {
                let mut state = fixture.world_mut().resource_mut::<ViewState>();
                state.capture = Some(PathBuf::from("unused-close-route-check.png"));
                state.capture_view = view.into();
                state.frames = 0;
            }
            let camera = fixture
                .world_mut()
                .spawn((ArenaCamera, Transform::default()))
                .id();
            fixture.add_systems(Update, spectator::camera.after(drive_simulation));
            let mut reached = false;
            for _ in 0..240 {
                fixture.update();
                let session = fixture.world().resource::<ArenaSession>();
                let pose = fixture
                    .world()
                    .get::<Transform>(camera)
                    .expect("real observer pose");
                let subjects = spectator::close_subjects(session, pose);
                if fixture.world().resource::<ViewState>().frames >= 110
                    && spectator::both_teams_visible(session, &subjects)
                {
                    reached = true;
                    break;
                }
                if session.is_finished() {
                    break;
                }
            }
            assert!(
                reached,
                "{map:?}/{view} must frame a useful unobstructed subject from each actual team"
            );
            assert!(fixture
                .world()
                .resource::<ViewState>()
                .capture_inputs
                .iter()
                .all(|(_, input)| input.movement == Vec2::ZERO
                    && !input.cast_pressed
                    && !input.cast_held
                    && !input.cast_released));
        }
    }
}
