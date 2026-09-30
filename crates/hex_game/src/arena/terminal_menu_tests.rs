//! Real application-driver round completion and menu re-entry, without a window.
use super::*;
use bevy::ecs::system::RunSystemOnce;
use hex_arena::{ArenaBattleSetup, ArenaOutcome, BattlePreset};
use hex_core::{HexCoord, TerrainEdit, TilePos};

fn finish_frame(app: &mut App) {
    for _ in 0..6 {
        app.update();
        if app.world().resource::<ViewState>().paused {
            return;
        }
    }
    assert!(
        app.world().resource::<ViewState>().paused,
        "terminal tick must open the menu"
    );
}

fn install_hud(app: &mut App) {
    app.world_mut()
        .run_system_once(hud::setup)
        .expect("create actual arena HUD");
    app.world_mut()
        .run_system_once(hud::update)
        .expect("update actual arena HUD");
}

#[test]
fn shadow_menu_tuning_is_live_bounded_and_survives_reset_and_map_selection() {
    let (mut app, _) = menu_app();
    app.world_mut().resource_mut::<ArenaSession>().bot_enabled = false;
    install_hud(&mut app);
    press_action(&mut app, hud::Action::Start);
    tap_key(&mut app, KeyCode::Escape);
    let paused_tick = app.world().resource::<ArenaSession>().tick;
    assert!(
        (app.world()
            .resource::<ArenaTuning>()
            .bot
            .acquisition_seconds
            - 0.15)
            .abs()
            < 0.001
    );
    for (steps, direction, expected, label) in [
        (4, -1.0, 0.0, "Off"),
        (3, 1.0, 0.15, "150 ms"),
        (12, 1.0, 0.5, "500 ms"),
    ] {
        for _ in 0..steps {
            press_action(&mut app, hud::Action::Change(10, direction));
        }
        assert!(
            (app.world()
                .resource::<ArenaTuning>()
                .bot
                .acquisition_seconds
                - expected)
                .abs()
                < 0.001
        );
        assert_eq!(app.world().resource::<ArenaSession>().tick, paused_tick);
        app.world_mut()
            .run_system_once(hud::update)
            .expect("updated parameter text");
        let mut labels = app.world_mut().query::<(&hud::Label, &Text)>();
        assert!(labels
            .iter(app.world())
            .any(
                |(kind, text)| matches!(kind, hud::Label::Parameter(10)) && text.0.contains(label)
            ));
    }
    press_action(&mut app, hud::Action::Change(11, -1.0));
    assert!(!app.world().resource::<ArenaTuning>().bot.escape.enabled);
    press_action(&mut app, hud::Action::Change(11, 1.0));
    assert!(app.world().resource::<ArenaTuning>().bot.escape.enabled);
    press_action(&mut app, hud::Action::Change(2, 1.0));
    press_action(&mut app, hud::Action::Change(7, -1.0));
    assert!((app.world().resource::<ArenaTuning>().high_jump_height - 4.5).abs() < 0.001);
    assert!((app.world().resource::<ArenaTuning>().high_jump_cooldown - 6.5).abs() < 0.001);
    press_action(&mut app, hud::Action::Resume);
    assert!(!app.world().resource::<ViewState>().paused);
    assert!(app
        .world()
        .resource::<ArenaSession>()
        .actors
        .iter()
        .all(|a| a.charge().is_none()));
    tap_key(&mut app, KeyCode::Escape);
    press_action(&mut app, hud::Action::Restart);
    press_action(&mut app, hud::Action::Map(ArenaMap::Fort));
    assert_eq!(app.world().resource::<ArenaSelection>().map, ArenaMap::Fort);
    assert!((app.world().resource::<ArenaTuning>().high_jump_height - 4.5).abs() < 0.001);
    assert!((app.world().resource::<ArenaTuning>().high_jump_cooldown - 6.5).abs() < 0.001);

    assert!(
        (app.world()
            .resource::<ArenaTuning>()
            .bot
            .acquisition_seconds
            - 0.5)
            .abs()
            < 0.001
    );
    assert!(app
        .world()
        .resource::<ArenaSession>()
        .actors
        .iter()
        .all(|a| a.charge().is_none()));
}

fn assert_terminal_menu(app: &mut App, window: Entity, expected_title: &str) {
    let state = app.world().resource::<ViewState>();
    assert!(state.started && state.paused && state.suppress_click);
    assert_eq!(state.accumulator.to_bits(), 0.0_f64.to_bits());
    let cursor = app
        .world()
        .get::<CursorOptions>(window)
        .expect("primary cursor");
    assert!(cursor.visible);
    assert_eq!(cursor.grab_mode, CursorGrabMode::None);
    assert!(app
        .world()
        .resource::<ArenaSession>()
        .actors
        .iter()
        .all(|a| a.charge().is_none()));
    let input = &app.world().resource::<ArenaInput>().human;
    assert!(!input.cast_pressed && !input.cast_released && !input.cast_held);
    assert_eq!(input.movement, Vec2::ZERO);
    app.world_mut()
        .run_system_once(hud::update)
        .expect("terminal HUD");
    let mut panels = app
        .world_mut()
        .query_filtered::<&Node, With<hud::PausePanel>>();
    assert!(panels.single(app.world()).expect("one pause panel").display != Display::None);
    let mut labels = app.world_mut().query::<(&hud::Label, &Text)>();
    let mut title = false;
    let mut resume = false;
    for (label, text) in labels.iter(app.world()) {
        if matches!(label, hud::Label::MenuTitle) {
            assert_eq!(text.0, expected_title);
            title = true;
        }
        if matches!(label, hud::Label::Resume) {
            assert_eq!(text.0, "ROUND COMPLETE");
            resume = true;
        }
    }
    assert!(title && resume);
}

#[test]
fn win_loss_and_draw_auto_open_the_menu_and_cancel_a_prepared_human_cast() {
    for (dead_human, dead_enemy, expected, title) in [
        (
            false,
            true,
            ArenaOutcome::Winner(0),
            "YOU WIN / COMBAT MENU",
        ),
        (
            true,
            false,
            ArenaOutcome::Winner(1),
            "DEFEATED / COMBAT MENU",
        ),
        (true, true, ArenaOutcome::Draw, "ROUND OVER / COMBAT MENU"),
    ] {
        let (mut app, window) = menu_app();
        app.world_mut().resource_mut::<ArenaSession>().bot_enabled = false;
        install_hud(&mut app);
        press_action(&mut app, hud::Action::Start);
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
            .expect("human")
            .charge()
            .is_some());
        {
            let mut session = app.world_mut().resource_mut::<ArenaSession>();
            if dead_human {
                session.actors.first_mut().expect("human").hp = 0.0;
            }
            if dead_enemy {
                session.actors.get_mut(1).expect("opponent").hp = 0.0;
            }
        }
        finish_frame(&mut app);
        assert_eq!(
            app.world().resource::<ArenaSession>().outcome,
            Some(expected)
        );
        assert_terminal_menu(&mut app, window, title);
        let ended_tick = app.world().resource::<ArenaSession>().tick;
        let hp: Vec<_> = app
            .world()
            .resource::<ArenaSession>()
            .actors
            .iter()
            .map(|a| a.hp)
            .collect();
        for key in [KeyCode::Escape, KeyCode::Tab, KeyCode::Enter] {
            tap_key(&mut app, key);
        }
        press_action(&mut app, hud::Action::Resume);
        assert!(app.world().resource::<ViewState>().paused);
        assert_eq!(app.world().resource::<ArenaSession>().tick, ended_tick);
        assert_eq!(
            app.world()
                .resource::<ArenaSession>()
                .actors
                .iter()
                .map(|a| a.hp)
                .collect::<Vec<_>>(),
            hp
        );
        assert_terminal_menu(&mut app, window, title);
    }
}

#[test]
fn one_dead_group_member_keeps_combat_running_and_whole_party_death_opens_menu() {
    let (mut app, window) = menu_app();
    app.world_mut().resource_mut::<ArenaSession>().bot_enabled = false;
    press_action(&mut app, hud::Action::Encounter(ArenaEncounter::Goblins));
    install_hud(&mut app);
    press_action(&mut app, hud::Action::Start);
    {
        let mut session = app.world_mut().resource_mut::<ArenaSession>();
        assert_eq!(
            session.actors.len(),
            BattlePreset::Goblins.members().len() + 1
        );
        session.actors.get_mut(1).expect("first Goblin").hp = 0.0;
    }
    app.update();
    assert!(!app.world().resource::<ArenaSession>().is_finished());
    assert!(!app.world().resource::<ViewState>().paused);
    assert!(
        !app.world()
            .get::<CursorOptions>(window)
            .expect("cursor")
            .visible
    );
    for actor in app
        .world_mut()
        .resource_mut::<ArenaSession>()
        .actors
        .iter_mut()
        .skip(1)
    {
        actor.hp = 0.0;
    }
    finish_frame(&mut app);
    assert_terminal_menu(&mut app, window, "YOU WIN / COMBAT MENU");
}

#[test]
fn terminal_restart_button_and_r_return_to_ready_and_preserve_the_selected_party() {
    for button in [true, false] {
        let (mut app, window) = menu_app();
        app.world_mut().resource_mut::<ArenaSession>().bot_enabled = false;
        press_action(
            &mut app,
            hud::Action::Encounter(ArenaEncounter::ShamanParty),
        );
        install_hud(&mut app);
        press_action(&mut app, hud::Action::Start);
        app.world_mut()
            .resource_mut::<ArenaSession>()
            .actors
            .first_mut()
            .expect("human")
            .hp = 0.0;
        finish_frame(&mut app);
        let generation = app.world().resource::<ArenaReset>().generation;
        if button {
            press_action(&mut app, hud::Action::Restart);
        } else {
            tap_restart(&mut app);
        }
        assert_eq!(
            app.world().resource::<ArenaReset>().generation,
            generation + 1
        );
        assert!(!app.world().resource::<ViewState>().started);
        assert!(app.world().resource::<ViewState>().paused);
        let session = app.world().resource::<ArenaSession>();
        assert!(!session.is_finished());
        assert!(session
            .actors
            .iter()
            .all(|a| a.hp.to_bits() == a.max_hp.to_bits() && a.charge().is_none()));
        assert_eq!(
            session
                .actors
                .iter()
                .skip(1)
                .map(|a| a.species)
                .collect::<Vec<_>>(),
            BattlePreset::ShamanParty.members()
        );
        app.world_mut()
            .run_system_once(hud::update)
            .expect("reset HUD");
        let mut start = app
            .world_mut()
            .query_filtered::<&Node, With<hud::StartPanel>>();
        assert_ne!(
            start.single(app.world()).expect("start panel").display,
            Display::None
        );
        let mut pause = app
            .world_mut()
            .query_filtered::<&Node, With<hud::PausePanel>>();
        assert_eq!(
            pause.single(app.world()).expect("pause panel").display,
            Display::None
        );
        assert!(
            app.world()
                .get::<CursorOptions>(window)
                .expect("cursor")
                .visible
        );
    }
}

#[test]
fn observer_timeout_opens_terminal_menu_without_a_dummy_human_or_resumable_camera() {
    let (mut app, window) = menu_app();
    let mut setup = ArenaBattleSetup::spectator(BattlePreset::Shadow, BattlePreset::Shadow, 17);
    setup.tick_limit = Some(3);
    app.insert_resource(setup);
    app.world_mut().resource_mut::<ArenaReset>().generation += 1;
    app.update();
    install_hud(&mut app);
    press_action(&mut app, hud::Action::Start);
    finish_frame(&mut app);
    let session = app.world().resource::<ArenaSession>();
    assert!(session.human_actor_id().is_none());
    assert_eq!(
        session.battle_summary().expect("observer result").result,
        Some(hex_arena::BattleResult::Timeout)
    );
    assert_terminal_menu(&mut app, window, "BATTLE COMPLETE");
    let mode = app.world().resource::<ViewState>().observer.mode;
    tap_key(&mut app, KeyCode::KeyC);
    tap_key(&mut app, KeyCode::Escape);
    assert_eq!(app.world().resource::<ViewState>().observer.mode, mode);
    assert!(app.world().resource::<ViewState>().paused);
}

#[derive(Resource)]
struct LastTickEdit {
    pos: TilePos,
    written: bool,
}

fn enqueue_after_terminal_simulation(
    session: Res<ArenaSession>,
    materials: Res<hex_core::arena::ArenaMaterials>,
    mut edit: ResMut<LastTickEdit>,
    mut writes: MessageWriter<TerrainEdit>,
) {
    if session.is_finished() && !edit.written {
        writes.write(TerrainEdit::Set {
            pos: edit.pos,
            substance: materials.stone,
        });
        edit.written = true;
    }
}

#[test]
fn terminal_menu_flushes_the_last_tick_terrain_queue_without_an_extra_living_tick() {
    let (mut app, window) = menu_app_at(480);
    app.world_mut().resource_mut::<ArenaSession>().bot_enabled = false;
    let pos = TilePos::new(HexCoord::ORIGIN, 15);
    assert!(app
        .world()
        .resource::<ArenaTerrainView>()
        .voxels
        .get(&pos)
        .copied()
        .is_none());
    app.insert_resource(LastTickEdit {
        pos,
        written: false,
    });
    app.add_systems(
        ArenaTick,
        enqueue_after_terminal_simulation.after(hex_core::arena::ArenaSystems::Simulate),
    );
    press_action(&mut app, hud::Action::Start);
    let before = app.world().resource::<ArenaSession>().tick;
    app.world_mut()
        .resource_mut::<ArenaSession>()
        .actors
        .get_mut(1)
        .expect("opponent")
        .hp = 0.0;
    finish_frame(&mut app);
    assert!(app.world().resource::<LastTickEdit>().written);
    assert_eq!(app.world().resource::<ArenaSession>().tick, before + 1);
    let stone = app
        .world()
        .resource::<hex_core::arena::ArenaMaterials>()
        .stone;
    assert_eq!(
        app.world()
            .resource::<ArenaTerrainView>()
            .voxels
            .get(&pos)
            .copied(),
        Some(stone)
    );
    assert!(
        app.world()
            .get::<CursorOptions>(window)
            .expect("cursor")
            .visible
    );
    let frozen = app.world().resource::<ArenaSession>().tick;
    app.update();
    assert_eq!(app.world().resource::<ArenaSession>().tick, frozen);
}

#[cfg(feature = "test-support")]
#[test]
#[ignore = "requires HEX_FOREST_WORLD pointing at the compiled expedition and companion"]
fn expedition_defeat_restart_pointer_does_not_activate_quit() {
    use bevy::input::{mouse::MouseButtonInput, ButtonState};
    use hex_ui::test_support::HeadlessUiPlugin;

    for (width, height) in [(1600, 900), (1280, 720), (1920, 1080)] {
        let mut app = App::new();
        app.add_plugins(HeadlessUiPlugin::new(width, height))
            .insert_resource(ViewState {
                started: false,
                paused: true,
                capture: None,
                ..default()
            })
            .insert_resource(ArenaSelection {
                map: ArenaMap::ForestMassif,
                ..default()
            })
            .add_plugins((hex_map::arena::plugin, hex_arena::plugin))
            .add_systems(Startup, hud::setup)
            .add_systems(
                Update,
                (hud::buttons, drive_simulation, hud::update).chain(),
            );
        for _ in 0..8 {
            app.update();
        }
        let window = app
            .world_mut()
            .query_filtered::<Entity, With<PrimaryWindow>>()
            .single(app.world())
            .expect("one primary window");
        app.world_mut()
            .get_mut::<Window>(window)
            .expect("window")
            .focused = true;
        app.world_mut().resource_mut::<ArenaSession>().bot_enabled = false;
        app.world_mut().resource_mut::<ViewState>().begin_play();
        app.world_mut()
            .resource_mut::<ArenaSession>()
            .actors
            .first_mut()
            .expect("player")
            .hp = 0.0;
        for _ in 0..8 {
            app.update();
        }
        assert!(app.world().resource::<ArenaSession>().is_finished());
        assert!(app.world().resource::<ViewState>().paused);
        let (restart, position, restart_size) = app
            .world_mut()
            .query::<(
                Entity,
                &hud::Action,
                &bevy::ui::UiGlobalTransform,
                &ComputedNode,
            )>()
            .iter(app.world())
            .find_map(|(entity, action, transform, node)| {
                matches!(action, hud::Action::Restart).then_some((
                    entity,
                    transform.affine().translation,
                    node.size(),
                ))
            })
            .expect("restart control");
        let generation = app.world().resource::<ArenaReset>().generation;
        app.world_mut()
            .get_mut::<Window>(window)
            .expect("window")
            .set_physical_cursor_position(Some(position.into()));
        app.world_mut().write_message(MouseButtonInput {
            button: MouseButton::Left,
            state: ButtonState::Pressed,
            window,
        });
        app.update();
        assert_eq!(
            app.world().get::<Interaction>(restart),
            Some(&Interaction::Pressed)
        );
        assert_eq!(
            app.world().resource::<ArenaReset>().generation,
            generation + 1
        );
        assert!(!app.world().resource::<ArenaSession>().is_finished());
        assert!(
            app.world().resource::<Messages<AppExit>>().is_empty(),
            "restart click must not quit at {width}x{height}"
        );
        app.world_mut().write_message(MouseButtonInput {
            button: MouseButton::Left,
            state: ButtonState::Released,
            window,
        });
        app.update();
        assert!(
            app.world().resource::<Messages<AppExit>>().is_empty(),
            "restart release must not quit"
        );
        let old_restart = Rect::from_center_size(position, restart_size);
        for (action, transform, node) in app
            .world_mut()
            .query::<(&hud::Action, &bevy::ui::UiGlobalTransform, &ComputedNode)>()
            .iter(app.world())
        {
            if matches!(action, hud::Action::Quit) && node.size().min_element() > 0.0 {
                let quit = Rect::from_center_size(transform.affine().translation, node.size());
                assert!(quit.max.x <= old_restart.min.x || old_restart.max.x <= quit.min.x || quit.max.y <= old_restart.min.y || old_restart.max.y <= quit.min.y,
                    "new Quit must not overlap any part of Restart at {width}x{height}: {old_restart:?}, {quit:?}");
            }
        }
        // A rapid second press follows the real UI focus/hitbox path after the
        // replacement menu was laid out, rather than directly invoking an action.
        app.world_mut().write_message(MouseButtonInput {
            button: MouseButton::Left,
            state: ButtonState::Pressed,
            window,
        });
        app.update();
        assert!(
            app.world().resource::<Messages<AppExit>>().is_empty(),
            "a repeated restart click must not land on Quit in the new menu"
        );
        assert!(
            app.world().resource::<ViewState>().started,
            "repeated click selects Start at {width}x{height}"
        );
    }
}
