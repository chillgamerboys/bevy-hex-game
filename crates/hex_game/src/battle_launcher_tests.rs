use super::*;
use std::collections::{BTreeMap, VecDeque};
use std::ffi::OsStr;
use std::sync::{Arc, Mutex};

use bevy::state::app::StatesPlugin;

enum Reply {
    Exit(bool),
    MonitorError,
}

#[derive(Default)]
struct Probe {
    launches: usize,
    terminations: usize,
    fail_spawn: bool,
    replies: VecDeque<Reply>,
}

struct TestChild(Arc<Mutex<Probe>>);

impl ChildProcess for TestChild {
    fn poll(&mut self) -> io::Result<Option<ChildExit>> {
        match self
            .0
            .lock()
            .map_err(|_poison| io::Error::other("poisoned probe"))?
            .replies
            .pop_front()
        {
            Some(Reply::Exit(success)) => Ok(Some(ChildExit {
                success,
                description: "test exit status".into(),
            })),
            Some(Reply::MonitorError) => Err(io::Error::other("test monitor failure")),
            None => Ok(None),
        }
    }

    fn terminate(&mut self) {
        self.0.lock().expect("probe").terminations += 1;
    }
}

struct Fixture {
    app: App,
    probe: Arc<Mutex<Probe>>,
    window: Entity,
    hidden_window: Entity,
    camera: Entity,
    inactive_camera: Entity,
    settings: WinitSettings,
}

fn fixture() -> Fixture {
    fixture_with_window_policy(true)
}

fn fixture_with_window_policy(hide_while_running: bool) -> Fixture {
    let probe = Arc::new(Mutex::new(Probe::default()));
    let for_backend = Arc::clone(&probe);
    let mut app = App::new();
    let settings = WinitSettings {
        focused_mode: UpdateMode::reactive(Duration::from_secs(2)),
        unfocused_mode: UpdateMode::reactive_low_power(Duration::from_secs(4)),
    };
    app.add_plugins((MinimalPlugins, StatesPlugin))
        .insert_state(Screen::Title)
        .insert_resource(settings.clone());
    plugin(&mut app);
    app.insert_resource(ParentWindowPolicy { hide_while_running });
    app.insert_resource(LaunchBackend(Box::new(move || {
        let mut state = for_backend
            .lock()
            .map_err(|_poison| io::Error::other("poisoned probe"))?;
        state.launches += 1;
        if state.fail_spawn {
            return Err(io::Error::other("test spawn failure"));
        }
        Ok(RunningChild {
            process: Box::new(TestChild(Arc::clone(&for_backend))),
            finished: false,
        })
    })));
    let window = app
        .world_mut()
        .spawn((Window::default(), PrimaryWindow))
        .id();
    let hidden_window = app
        .world_mut()
        .spawn((
            Window {
                visible: false,
                ..default()
            },
            PrimaryWindow,
        ))
        .id();
    let camera = app.world_mut().spawn(Camera::default()).id();
    let inactive_camera = app
        .world_mut()
        .spawn(Camera {
            is_active: false,
            ..default()
        })
        .id();
    Fixture {
        app,
        probe,
        window,
        hidden_window,
        camera,
        inactive_camera,
        settings,
    }
}

fn request(f: &mut Fixture) {
    f.app.insert_resource(BattleLaunchRequest);
    f.app.update();
    assert!(!f.app.world().contains_resource::<BattleLaunchRequest>());
}

fn assert_restored(f: &Fixture) {
    let world = f.app.world();
    assert!(world.get::<Window>(f.window).expect("window").visible);
    assert!(
        !world
            .get::<Window>(f.hidden_window)
            .expect("hidden window")
            .visible
    );
    assert!(world.get::<Camera>(f.camera).expect("camera").is_active);
    assert!(
        !world
            .get::<Camera>(f.inactive_camera)
            .expect("inactive camera")
            .is_active
    );
    let restored = world.resource::<WinitSettings>();
    assert_eq!(restored.focused_mode, f.settings.focused_mode);
    assert_eq!(restored.unfocused_mode, f.settings.unfocused_mode);
    assert_eq!(*world.resource::<State<Screen>>().get(), Screen::Title);
}

#[test]
fn native_command_has_one_mode_argument_and_preserves_assets_and_runtime_environment() {
    let keys = [
        "HEX_ARENA_CAPTURE",
        "HEX_ARENA_VIEW",
        "HEX_ARENA_CONTROL",
        "HEX_ARENA_TEAM_A",
        "hex_arena_battle_seed",
        "HEX_REVIEW_SCENARIO",
        "HEX_WALK_SCRIPT",
        "BEVY_ASSET_ROOT",
        "CARGO_MANIFEST_DIR",
        "DYLD_LIBRARY_PATH",
        "LD_LIBRARY_PATH",
        "PATH",
        "HEX_GAME_DATA_DIR",
    ];
    let command = launch_command(
        Path::new("/installation/Hex Game"),
        keys.map(OsString::from).into_iter(),
    );
    assert_eq!(command.get_program(), OsStr::new("/installation/Hex Game"));
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        [OsStr::new("--arena")]
    );
    assert!(command.get_current_dir().is_none());
    let changes: BTreeMap<_, _> = command.get_envs().collect();
    for key in [
        "HEX_ARENA_CAPTURE",
        "HEX_ARENA_VIEW",
        "HEX_ARENA_TEAM_A",
        "hex_arena_battle_seed",
        "HEX_REVIEW_SCENARIO",
        "HEX_WALK_SCRIPT",
    ] {
        assert_eq!(changes.get(OsStr::new(key)), Some(&None));
    }
    for (key, expected) in [
        ("HEX_ARENA_MAP", "fort"),
        ("HEX_ARENA_ENCOUNTER", "dragon"),
        ("HEX_ARENA_CONTROL", "player"),
    ] {
        assert_eq!(
            changes.get(OsStr::new(key)),
            Some(&Some(OsStr::new(expected)))
        );
    }
    for key in [
        "BEVY_ASSET_ROOT",
        "CARGO_MANIFEST_DIR",
        "DYLD_LIBRARY_PATH",
        "LD_LIBRARY_PATH",
        "PATH",
        "HEX_GAME_DATA_DIR",
    ] {
        assert!(!changes.contains_key(OsStr::new(key)));
    }
}

#[test]
fn duplicate_requests_and_exit_frame_click_cannot_start_a_second_child() {
    let mut f = fixture();
    request(&mut f);
    assert!(f.app.world().resource::<BattleLauncher>().running());
    assert!(
        !f.app
            .world()
            .get::<Window>(f.window)
            .expect("window")
            .visible
    );
    assert!(
        !f.app
            .world()
            .get::<Camera>(f.camera)
            .expect("camera")
            .is_active
    );
    let settings = f.app.world().resource::<WinitSettings>();
    let low_power = UpdateMode::reactive_low_power(Duration::from_millis(250));
    assert_eq!(settings.focused_mode, low_power);
    assert_eq!(settings.unfocused_mode, low_power);
    request(&mut f);
    f.probe
        .lock()
        .expect("probe")
        .replies
        .push_back(Reply::Exit(true));
    request(&mut f);
    assert_eq!(f.probe.lock().expect("probe").launches, 1);
    assert_eq!(f.probe.lock().expect("probe").terminations, 0);
    assert!(!f.app.world().resource::<BattleLauncher>().running());
    assert!(f.app.world().resource::<BattleLauncher>().error().is_none());
    assert_restored(&f);
    request(&mut f);
    assert_eq!(f.probe.lock().expect("probe").launches, 2);
}

#[test]
fn non_hiding_backend_keeps_the_parent_rendered_and_throttled_until_child_exit() {
    let mut f = fixture_with_window_policy(false);
    request(&mut f);
    let world = f.app.world();
    assert!(world.resource::<BattleLauncher>().running());
    assert!(
        world
            .get::<Window>(f.window)
            .expect("visible parent")
            .visible
    );
    assert!(
        !world
            .get::<Window>(f.hidden_window)
            .expect("hidden parent")
            .visible
    );
    assert!(
        world
            .get::<Camera>(f.camera)
            .expect("active parent camera")
            .is_active
    );
    assert!(
        !world
            .get::<Camera>(f.inactive_camera)
            .expect("inactive camera")
            .is_active
    );
    let low_power = UpdateMode::reactive_low_power(Duration::from_millis(250));
    let settings = world.resource::<WinitSettings>();
    assert_eq!(settings.focused_mode, low_power);
    assert_eq!(settings.unfocused_mode, low_power);
    f.probe
        .lock()
        .expect("probe")
        .replies
        .push_back(Reply::Exit(true));
    f.app.update();
    assert!(!f.app.world().resource::<BattleLauncher>().running());
    assert_restored(&f);
}

#[test]
fn spawn_failure_keeps_menu_live_and_retry_clears_the_notice() {
    let mut f = fixture();
    f.probe.lock().expect("probe").fail_spawn = true;
    request(&mut f);
    let launcher = f.app.world().resource::<BattleLauncher>();
    assert!(!launcher.running());
    assert_eq!(
        launcher.error(),
        Some("Battle Mode could not open. Please try again.")
    );
    assert_restored(&f);
    f.probe.lock().expect("probe").fail_spawn = false;
    request(&mut f);
    let launcher = f.app.world().resource::<BattleLauncher>();
    assert!(launcher.running() && launcher.error().is_none());
}

#[test]
fn child_failure_or_monitor_failure_restores_the_exact_presentation_state() {
    for monitor_failure in [false, true] {
        let mut f = fixture();
        request(&mut f);
        f.probe
            .lock()
            .expect("probe")
            .replies
            .push_back(if monitor_failure {
                Reply::MonitorError
            } else {
                Reply::Exit(false)
            });
        f.app.update();
        let launcher = f.app.world().resource::<BattleLauncher>();
        assert!(!launcher.running());
        assert!(launcher.error().is_some());
        assert_restored(&f);
        assert_eq!(
            f.probe.lock().expect("probe").terminations,
            usize::from(monitor_failure)
        );
    }
}

#[test]
fn non_title_requests_are_consumed_and_app_shutdown_owns_the_live_child() {
    let mut f = fixture();
    f.app.insert_state(Screen::Sandbox);
    request(&mut f);
    assert_eq!(f.probe.lock().expect("probe").launches, 0);
    assert!(
        f.app
            .world()
            .get::<Window>(f.window)
            .expect("window")
            .visible
    );
    f.app.insert_state(Screen::Title);
    request(&mut f);
    let probe = Arc::clone(&f.probe);
    drop(f);
    assert_eq!(probe.lock().expect("probe").terminations, 1);
}
