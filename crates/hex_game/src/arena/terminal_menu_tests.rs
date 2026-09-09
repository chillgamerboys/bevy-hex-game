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

fn assert_terminal_menu(app: &mut App, window: Entity, expected_title: &str) {
    let state = app.world().resource::<ViewState>();
    assert!(state.started && state.paused && state.suppress_click);
    assert_eq!(state.accumulator, 0.0);
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
            tap_key(&mut app, KeyCode::KeyR);
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
            .all(|a| a.hp == a.max_hp && a.charge().is_none()));
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
