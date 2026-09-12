//! Asynchronous, opt-in native recording. It observes the run and never drives it.

mod backend;

use std::sync::{mpsc, Mutex};
use std::time::Instant;

use bevy::prelude::*;
use bevy::window::{PrimaryWindow, WindowCloseRequested};
use hex_arena::ArenaSession;
use hex_core::arena::{ArenaReset, ArenaTerrainView};
use serde_json::{json, Value};

use super::{ArenaFrame, ViewState};

/// Local recording control and read-only HUD state; filesystem work runs elsewhere.
#[derive(Resource)]
pub(super) struct Recorder {
    commands: mpsc::SyncSender<backend::Command>,
    responses: Mutex<mpsc::Receiver<backend::Response>>,
    supported: bool,
    active: bool,
    finalizing: bool,
    started: Option<Instant>,
    status: String,
    toggle: bool,
    open_folder: bool,
    quit: bool,
    quit_sent: bool,
    last_run: Option<RunMarker>,
}

impl Recorder {
    pub(super) fn request_toggle(&mut self) {
        if self.supported && !self.quit {
            self.toggle = true;
        }
    }
    pub(super) fn request_open_folder(&mut self) {
        self.open_folder = true;
    }
    pub(super) fn request_quit(&mut self) {
        self.quit = true;
    }
    pub(super) fn status_text(&self) -> &str {
        &self.status
    }
    pub(super) fn is_recording(&self) -> bool {
        self.started.is_some()
    }
    pub(super) fn is_starting(&self) -> bool {
        self.active && self.started.is_none() && !self.finalizing
    }
    pub(super) fn is_finalizing(&self) -> bool {
        self.finalizing
    }
    pub(super) fn elapsed_seconds(&self) -> f64 {
        self.started
            .map_or(0.0, |started| started.elapsed().as_secs_f64())
    }
    pub(super) fn can_record(&self) -> bool {
        self.supported && !self.quit
    }

    fn send(&mut self, command: backend::Command) -> bool {
        match self.commands.try_send(command) {
            Ok(()) => true,
            Err(error) => {
                self.status = format!("Recorder unavailable: {error}");
                false
            }
        }
    }

    fn receive(&mut self) -> Vec<backend::Response> {
        match self.responses.get_mut() {
            Ok(receiver) => receiver.try_iter().take(16).collect(),
            Err(_) => {
                self.status = "Recorder status channel failed.".into();
                Vec::new()
            }
        }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        // The worker also finalizes on channel EOF, including abnormal app exit.
        let _ = self.commands.try_send(backend::Command::Quit);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct RunMarker {
    generation: u64,
    paused: bool,
    dead: bool,
    finished: bool,
}

pub(super) fn install(app: &mut App) {
    let (commands, responses) = backend::launch();
    app.insert_resource(Recorder {
        commands,
        responses: Mutex::new(responses),
        supported: false,
        active: false,
        finalizing: false,
        started: None,
        status: "Checking recording support…".into(),
        toggle: false,
        open_folder: false,
        quit: false,
        quit_sent: false,
        last_run: None,
    })
    .add_systems(
        Update,
        update.after(ArenaFrame::Tick).before(ArenaFrame::Present),
    );
}

fn snapshot(
    view: &ViewState,
    session: &ArenaSession,
    reset: &ArenaReset,
    terrain: &ArenaTerrainView,
) -> Value {
    json!({
        "generation": reset.generation, "tick": session.tick, "paused": view.paused,
        "started": view.started, "map": format!("{:?}", terrain.selection.map),
        "encounter": format!("{:?}", terrain.selection.encounter),
        "package": terrain.package_identity,
        "player": session.actors.iter().find(|actor| actor.id == 0).map(|actor|
            json!({"hp": actor.hp, "max_hp": actor.max_hp, "position": actor.feet.to_array()})),
    })
}

fn update(
    mut recorder: ResMut<Recorder>,
    view: Res<ViewState>,
    session: Res<ArenaSession>,
    reset: Res<ArenaReset>,
    terrain: Res<ArenaTerrainView>,
    keys: Res<ButtonInput<KeyCode>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut closing: MessageReader<WindowCloseRequested>,
    mut exit: MessageWriter<AppExit>,
) {
    for response in recorder.receive() {
        match response {
            backend::Response::Supported(supported, message) => {
                recorder.supported = supported;
                recorder.status = message;
            }
            backend::Response::Started => {
                recorder.started = Some(Instant::now());
                if !recorder.finalizing {
                    recorder.status = "Recording video · F9 adds a bookmark".into();
                }
            }
            backend::Response::Status(message) => recorder.status = message,
            backend::Response::Finished(message) | backend::Response::Failed(message) => {
                recorder.active = false;
                recorder.finalizing = false;
                recorder.started = None;
                recorder.status = message;
            }
            backend::Response::Quit => {
                exit.write(AppExit::Success);
            }
        }
    }
    if closing.read().next().is_some() {
        recorder.request_quit();
    }
    if recorder.quit && !recorder.quit_sent {
        recorder.finalizing = recorder.active;
        recorder.quit_sent = recorder.send(backend::Command::Quit);
        recorder.status = "Finalizing recording before quitting…".into();
        // A disconnected worker cannot finalize; report the failure, then exit.
        if !recorder.quit_sent {
            exit.write(AppExit::error());
        }
        return;
    }
    if std::mem::take(&mut recorder.open_folder) {
        recorder.send(backend::Command::OpenFolder);
    }
    if std::mem::take(&mut recorder.toggle) && !recorder.quit {
        if recorder.active {
            if recorder.send(backend::Command::Stop) {
                recorder.finalizing = true;
                recorder.status = "Finalizing recording…".into();
            }
        } else if view.capture.is_some() {
            recorder.status = "Recording is unavailable in windowless capture mode.".into();
        } else if let Ok(window) = windows.single() {
            let command = backend::Command::Start {
                pid: std::process::id(),
                title: window.title.clone(),
                snapshot: snapshot(&view, &session, &reset, &terrain),
            };
            if recorder.send(command) {
                recorder.active = true;
                recorder.finalizing = false;
                recorder.status = "Starting recorder; macOS may request screen permission…".into();
            }
        } else {
            recorder.status = "Recording needs exactly one battle window.".into();
        }
    }
    let marker = RunMarker {
        generation: reset.generation,
        paused: view.paused,
        dead: session
            .actors
            .iter()
            .find(|actor| actor.id == 0)
            .is_some_and(|actor| actor.hp <= 0.0),
        finished: session.outcome.is_some(),
    };
    if recorder.active {
        for kind in changed_events(recorder.last_run, marker) {
            recorder.send(backend::Command::Event {
                kind: kind.into(),
                snapshot: snapshot(&view, &session, &reset, &terrain),
            });
        }
        if keys.just_pressed(KeyCode::F9) && recorder.is_recording() && !recorder.finalizing {
            if recorder.send(backend::Command::Event {
                kind: "bookmark".into(),
                snapshot: snapshot(&view, &session, &reset, &terrain),
            }) {
                recorder.status = "Saving bookmark…".into();
            }
        }
    }
    recorder.last_run = Some(marker);
}

fn changed_events(previous: Option<RunMarker>, current: RunMarker) -> Vec<&'static str> {
    let Some(previous) = previous else {
        return Vec::new();
    };
    if previous.generation != current.generation {
        return vec!["restart"];
    }
    let mut events = Vec::with_capacity(3);
    if previous.dead != current.dead && current.dead {
        events.push("player_death");
    }
    if previous.paused != current.paused {
        events.push(if current.paused { "pause" } else { "resume" });
    }
    if previous.finished != current.finished && current.finished {
        events.push("run_finished");
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_events_keep_simultaneous_death_pause_and_outcome_and_restart_once() {
        let before = RunMarker {
            generation: 4,
            paused: false,
            dead: false,
            finished: false,
        };
        let terminal = RunMarker {
            generation: 4,
            paused: true,
            dead: true,
            finished: true,
        };
        assert_eq!(
            changed_events(Some(before), terminal),
            vec!["player_death", "pause", "run_finished"]
        );
        assert!(changed_events(Some(terminal), terminal).is_empty());
        assert_eq!(
            changed_events(
                Some(terminal),
                RunMarker {
                    generation: 5,
                    ..before
                }
            ),
            vec!["restart"]
        );
    }

    #[test]
    fn recording_state_waits_for_native_start_and_toggle_only_requests_work() {
        let (sender, receiver) = mpsc::sync_channel(8);
        let (_, responses) = mpsc::channel();
        let mut recorder = Recorder {
            commands: sender,
            responses: Mutex::new(responses),
            supported: true,
            active: false,
            finalizing: false,
            started: None,
            status: String::new(),
            toggle: false,
            open_folder: false,
            quit: false,
            quit_sent: false,
            last_run: None,
        };
        recorder.request_toggle();
        assert!(recorder.toggle);
        assert!(!recorder.is_recording());
        assert!(receiver.try_recv().is_err());
        recorder.request_quit();
        assert!(!recorder.can_record());
    }
}
