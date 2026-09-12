//! Preparation of the implicit expedition package before launch or map selection.

use std::{
    ffi::OsStr,
    path::PathBuf,
    process::Command,
    sync::{Mutex, mpsc},
};

use hex_core::arena::ArenaMap;

/// Preparing a candidate never changes the currently admitted map. Canceling
/// interest leaves any running preparation available as a cache for a later click.
#[derive(Default)]
pub(super) struct Preparation {
    completion: Option<Mutex<mpsc::Receiver<Result<(), String>>>>,
    requested: bool,
    ready: bool,
    error: Option<String>,
}

impl Preparation {
    /// True permits immediate selection; false leaves the old map published.
    pub fn request(&mut self) -> bool {
        if !requires_preparation(
            ArenaMap::ForestMassif,
            std::env::var_os("HEX_FOREST_WORLD").as_deref(),
        ) || self.ready
        {
            return true;
        }
        self.requested = true;
        self.error = None;
        if self.completion.is_none() {
            let (send, receive) = mpsc::channel();
            // Waiting for a compiler is blocking work. Keep it off Bevy's shared
            // pools so choosing another map can still load that map's assets.
            match std::thread::Builder::new()
                .name("forest-preparation".into())
                .spawn(move || {
                    // Closing the app can drop the receiver while the reusable
                    // package finishes; there is then no menu to notify.
                    drop(send.send(prepare_default(ArenaMap::ForestMassif)));
                }) {
                Ok(_) => self.completion = Some(Mutex::new(receive)),
                Err(error) => {
                    self.finish(Err(format!("Cannot start package worker: {error}")));
                }
            }
        }
        false
    }

    pub fn cancel_selection(&mut self) {
        self.requested = false;
        self.error = None;
    }

    /// Nonblocking completion poll; true commits a still-requested selection once.
    pub fn poll(&mut self) -> bool {
        let Some(completion) = self.completion.as_mut() else {
            return false;
        };
        // The receiver belongs only to this state; Mutex makes the Bevy resource
        // Sync, and exclusive access here never waits on the worker thread.
        let receive = completion
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let result = match receive.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return false,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("Package worker ended without a result".into())
            }
        };
        self.completion = None;
        self.finish(result)
    }

    fn finish(&mut self, result: Result<(), String>) -> bool {
        let requested = std::mem::take(&mut self.requested);
        match result {
            Ok(()) => {
                self.ready = true;
                self.error = None;
                requested
            }
            Err(error) => {
                bevy::log::error!("Forest preparation: {error}");
                if requested {
                    self.error = Some(format!(
                        "Forest preparation failed: {error}\nYour current map is available. Choose Forest to retry."
                    ));
                }
                false
            }
        }
    }

    pub fn status(&self) -> Option<&str> {
        if self.requested && self.completion.is_some() {
            Some(
                "Preparing Forest Expedition…\nYou can choose another map or start the current map while it builds.",
            )
        } else {
            self.error.as_deref()
        }
    }
}

fn requires_preparation(map: ArenaMap, package_override: Option<&OsStr>) -> bool {
    map == ArenaMap::ForestMassif && package_override.is_none()
}

fn authoring_target(root: &std::path::Path, app_target: Option<&OsStr>) -> PathBuf {
    let preferred = root.join("target/v4-authoring");
    let app_target = app_target.map_or_else(
        || root.join("target"),
        |value| {
            let path = PathBuf::from(value);
            if path.is_absolute() {
                path
            } else {
                root.join(path)
            }
        },
    );
    let canonical_app = app_target.canonicalize().unwrap_or(app_target);
    let canonical_preferred = preferred
        .canonicalize()
        .unwrap_or_else(|_| preferred.clone());
    if canonical_app == canonical_preferred {
        preferred.join("forest-bootstrap")
    } else {
        preferred
    }
}

pub(super) fn prepare_default(map: ArenaMap) -> Result<(), String> {
    let package_override = std::env::var_os("HEX_FOREST_WORLD");
    if !requires_preparation(map, package_override.as_deref()) {
        return Ok(());
    }
    let root = std::env::var_os("BEVY_ASSET_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let root = root
        .canonicalize()
        .map_err(|error| format!("Forest asset root: {error}"))?;
    // The child may build worldc; never inherit the currently running app target.
    let target = authoring_target(&root, std::env::var_os("CARGO_TARGET_DIR").as_deref());
    let status = Command::new("python3")
        .arg(root.join("tools/forest_package.py"))
        .arg("ensure")
        .arg("--target-dir")
        .arg(&target)
        .current_dir(&root)
        .env("CARGO_TARGET_DIR", &target)
        .env("CARGO_INCREMENTAL", "0")
        .env("CARGO_BUILD_JOBS", "2")
        .status()
        .map_err(|error| format!("Cannot prepare Forest Expedition: {error}. Install Python 3 and run python3 tools/forest_package.py ensure."))?;
    if !status.success() {
        return Err(format!(
            "Forest Expedition preparation failed ({status}). Run python3 tools/forest_package.py ensure."
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::{ViewState, hud};
    use super::*;
    use bevy::prelude::*;
    use hex_arena::{ArenaBattleSetup, ArenaInput, ArenaSession, ArenaTuning};
    use hex_core::arena::{ArenaReset, ArenaSelection};
    use hex_test_app::HeadlessAppBuilder;

    fn inject_preparation(app: &mut App) -> mpsc::Sender<Result<(), String>> {
        let (send, receive) = mpsc::channel();
        app.world_mut()
            .resource_mut::<ViewState>()
            .forest_preparation = Preparation {
            completion: Some(Mutex::new(receive)),
            requested: true,
            ..Default::default()
        };
        send
    }

    fn preparation_menu() -> App {
        let mut builder = HeadlessAppBuilder::new().with_minimal_plugins();
        builder
            .app_mut()
            .insert_resource(ViewState {
                started: false,
                paused: true,
                capture: None,
                ..default()
            })
            .insert_resource(ArenaSelection {
                map: ArenaMap::Duel,
                ..default()
            })
            .init_resource::<ArenaReset>()
            .init_resource::<ArenaBattleSetup>()
            .init_resource::<ArenaTuning>()
            .init_resource::<ArenaInput>()
            .init_resource::<ArenaSession>()
            .add_message::<AppExit>()
            .add_systems(Update, hud::buttons);
        builder.build()
    }

    #[test]
    fn only_implicit_forest_launches_prepare_a_package() {
        assert!(requires_preparation(ArenaMap::ForestMassif, None));
        for map in [ArenaMap::Duel, ArenaMap::Fort, ArenaMap::SevenRegions] {
            assert!(!requires_preparation(map, None));
        }
        assert!(!requires_preparation(
            ArenaMap::ForestMassif,
            Some(OsStr::new("legacy-package"))
        ));
        assert!(
            !requires_preparation(ArenaMap::ForestMassif, Some(OsStr::new(""))),
            "an invalid explicit override must fail through normal admission"
        );
    }

    #[test]
    fn authoring_uses_a_different_target_even_if_app_uses_the_preferred_cache() {
        let root = std::env::temp_dir().join("hex-bootstrap-target-test");
        let preferred = root.join("target/v4-authoring");
        assert_eq!(authoring_target(&root, None), preferred);
        assert_eq!(
            authoring_target(&root, Some(preferred.as_os_str())),
            preferred.join("forest-bootstrap")
        );
    }

    #[test]
    fn failed_preparation_cannot_commit_a_map_and_reports_retryable_error() {
        let mut preparation = Preparation {
            requested: true,
            ..Default::default()
        };
        assert!(!preparation.finish(Err("Python is unavailable".into())));
        assert!(!preparation.ready);
        assert!(
            preparation
                .status()
                .expect("visible failure")
                .contains("Python is unavailable")
        );
        preparation.cancel_selection();
        assert!(preparation.status().is_none());
    }

    #[test]
    fn canceled_switch_never_overrides_later_play_or_map_choice() {
        let mut preparation = Preparation {
            requested: true,
            ..Default::default()
        };
        preparation.cancel_selection();
        assert!(!preparation.finish(Ok(())));
        assert!(
            preparation.ready,
            "completed cache may serve a later explicit Forest click"
        );
        assert!(preparation.request());
        assert!(preparation.status().is_none());
    }

    #[test]
    fn accepted_preparation_commits_only_the_pending_request_once() {
        let mut preparation = Preparation {
            requested: true,
            ..Default::default()
        };
        assert!(preparation.finish(Ok(())));
        assert!(!preparation.poll());
        assert!(!preparation.finish(Ok(())));
    }

    #[test]
    fn pending_background_work_is_polled_without_waiting_for_its_result() {
        let (release, wait) = mpsc::channel();
        let (send, receive) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            wait.recv_timeout(std::time::Duration::from_secs(2))
                .expect("test releases the background preparation");
            send.send(Ok(())).expect("preparation still exists");
        });
        let mut preparation = Preparation {
            completion: Some(Mutex::new(receive)),
            requested: true,
            ..Default::default()
        };
        assert!(
            !preparation.poll(),
            "an unfinished process cannot block the menu"
        );
        assert!(
            preparation
                .status()
                .expect("visible progress")
                .starts_with("Preparing")
        );
        release.send(()).expect("release background worker");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !preparation.poll() {
            assert!(
                std::time::Instant::now() < deadline,
                "completed task must be delivered"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(preparation.status().is_none());
        assert!(!preparation.poll(), "completion commits only once");
        worker.join().expect("background worker succeeds");
    }

    #[test]
    fn disconnected_worker_preserves_the_map_and_allows_retry() {
        let (send, receive) = mpsc::channel::<Result<(), String>>();
        let mut preparation = Preparation {
            completion: Some(Mutex::new(receive)),
            requested: true,
            ..Default::default()
        };
        drop(send);
        assert!(!preparation.poll());
        assert!(preparation.completion.is_none());
        assert!(!preparation.ready);
        assert!(
            preparation
                .status()
                .expect("visible error")
                .contains("without a result")
        );
    }

    #[test]
    fn menu_changes_selection_and_reset_only_after_successful_preparation() {
        let mut app = preparation_menu();
        let generation = app.world().resource::<ArenaReset>().generation;
        let send = inject_preparation(&mut app);
        app.update();
        assert_eq!(app.world().resource::<ArenaSelection>().map, ArenaMap::Duel);
        assert_eq!(app.world().resource::<ArenaReset>().generation, generation);
        send.send(Err("compiler unavailable".into()))
            .expect("menu listens");
        app.update();
        assert_eq!(app.world().resource::<ArenaSelection>().map, ArenaMap::Duel);
        assert_eq!(app.world().resource::<ArenaReset>().generation, generation);
        assert!(
            app.world()
                .resource::<ViewState>()
                .forest_preparation
                .status()
                .expect("visible error")
                .contains("compiler unavailable")
        );

        inject_preparation(&mut app)
            .send(Ok(()))
            .expect("retry listens");
        app.update();
        assert_eq!(
            app.world().resource::<ArenaSelection>().map,
            ArenaMap::ForestMassif
        );
        assert_eq!(
            app.world().resource::<ArenaReset>().generation,
            generation + 1
        );
        assert!(!app.world().resource::<ViewState>().started);
        app.update();
        assert_eq!(
            app.world().resource::<ArenaReset>().generation,
            generation + 1
        );
    }

    #[test]
    fn other_map_and_start_clicks_win_over_same_frame_preparation_completion() {
        for action in [hud::Action::Map(ArenaMap::Fort), hud::Action::Start] {
            let mut app = preparation_menu();
            inject_preparation(&mut app)
                .send(Ok(()))
                .expect("menu listens");
            app.world_mut().spawn((Interaction::Pressed, action));
            app.update();
            assert_ne!(
                app.world().resource::<ArenaSelection>().map,
                ArenaMap::ForestMassif
            );
            match action {
                hud::Action::Map(map) => {
                    assert_eq!(app.world().resource::<ArenaSelection>().map, map)
                }
                hud::Action::Start => assert!(app.world().resource::<ViewState>().started),
                _ => unreachable!("test covers two competing menu choices"),
            }
        }
    }
}
