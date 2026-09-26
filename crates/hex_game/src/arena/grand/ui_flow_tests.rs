//! Storage-independent checks of the production HUD action routing.
use super::*;
use bevy::window::PrimaryWindow;

fn menu_fixture() -> App {
    let mut app = App::new();
    app.insert_resource(ViewState {
        started: false,
        paused: true,
        capture: None,
        ..default()
    })
    .insert_resource(ArenaSelection {
        map: ArenaMap::GrandV4,
        ..default()
    })
    .insert_resource(State {
        available: true,
        token: Some(CheckpointToken {
            generation: 17,
            fingerprint: 91,
        }),
        ..default()
    })
    .init_resource::<ArenaReset>()
    .init_resource::<ArenaTuning>()
    .init_resource::<hex_arena::ArenaBattleSetup>()
    .init_resource::<ArenaInput>()
    .init_resource::<ArenaSession>()
    .add_message::<AppExit>();
    app.world_mut().spawn((
        Window {
            focused: true,
            ..default()
        },
        PrimaryWindow,
    ));
    app
}

#[test]
fn grand_menu_buttons_queue_exact_manager_requests_without_exiting_or_resetting() {
    let mut app = menu_fixture();
    let token = app.world().resource::<State>().token;
    let generation = app.world().resource::<ArenaReset>().generation;
    // Start and Continue share the primary action; the manager chooses from the
    // durable slot's availability. Neither button may bypass that manager.
    for available in [false, true] {
        app.world_mut().resource_mut::<State>().available = available;
        press_menu_action(&mut app, hud::Action::Start);
        assert!(matches!(
            app.world().resource::<State>().request,
            Some(Request::Primary)
        ));
        assert!(!app.world().resource::<ViewState>().started);
        assert!(app.world().resource::<ViewState>().paused);
    }
    for action in [
        hud::Action::NewRun,
        hud::Action::ConfirmNew,
        hud::Action::CancelNew,
        hud::Action::Quit,
    ] {
        app.world_mut().resource_mut::<State>().request = None;
        press_menu_action(&mut app, action);
        assert!(matches!(
            (action, app.world().resource::<State>().request),
            (hud::Action::NewRun, Some(Request::NewRun))
                | (hud::Action::ConfirmNew, Some(Request::ConfirmNew))
                | (hud::Action::CancelNew, Some(Request::CancelNew))
                | (hud::Action::Quit, Some(Request::SaveQuit))
        ));
        assert_eq!(app.world().resource::<State>().token, token);
        assert!(app.world().resource::<State>().available);
        assert_eq!(app.world().resource::<ArenaReset>().generation, generation);
        assert!(app.world().resource::<ViewState>().paused);
        assert!(app.world().resource::<Messages<AppExit>>().is_empty());
    }
}

#[test]
fn grand_restart_requires_confirmation_and_resume_waits_for_cancel() {
    let mut app = menu_fixture();
    app.world_mut().resource_mut::<ViewState>().started = true;
    let token = app.world().resource::<State>().token;
    let generation = app.world().resource::<ArenaReset>().generation;
    press_menu_action(&mut app, hud::Action::Restart);
    assert!(app.world().resource::<State>().confirmation);
    assert!(app
        .world()
        .resource::<State>()
        .status
        .contains("Confirm New Run or Cancel"));
    assert_eq!(app.world().resource::<ArenaReset>().generation, generation);
    press_menu_action(&mut app, hud::Action::Resume);
    assert!(app.world().resource::<ViewState>().paused);
    assert_eq!(app.world().resource::<State>().token, token);

    press_menu_action(&mut app, hud::Action::CancelNew);
    // This resource-only fixture has no tuning or map plugins, so this manager
    // branch cannot open storage, stream terrain, or advance gameplay.
    update(app.world_mut());
    assert!(!app.world().resource::<State>().confirmation);
    assert!(app.world().resource::<State>().available);
    assert_eq!(app.world().resource::<State>().token, token);
    assert_eq!(app.world().resource::<ArenaReset>().generation, generation);
    press_menu_action(&mut app, hud::Action::Resume);
    assert!(!app.world().resource::<ViewState>().paused);
    assert!(app.world().resource::<ViewState>().started);
    assert!(app.world().resource::<Messages<AppExit>>().is_empty());
}
