//! Bounded worker protocol. No process, disk or encoder wait runs on a Bevy system.

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command as Process, Stdio};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

pub(super) enum Command {
    Start {
        pid: u32,
        title: String,
        snapshot: Value,
    },
    Stop,
    Event {
        kind: String,
        snapshot: Value,
    },
    OpenFolder,
    Quit,
}

pub(super) enum Response {
    Supported(bool, String),
    Started,
    Status(String),
    Finished(String),
    Failed(String),
    Quit(Result<(), String>),
}

pub(super) fn launch() -> (SyncSender<Command>, Receiver<Response>) {
    let (sender, requests) = mpsc::sync_channel(32);
    let (responses, receiver) = mpsc::sync_channel(32);
    let errors = responses.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("battle-recorder".into())
        .spawn(move || worker(requests, responses))
    {
        let _ = errors.send(Response::Supported(
            false,
            format!("Recorder worker unavailable: {error}"),
        ));
    }
    (sender, receiver)
}

fn folder() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or("The home folder is unavailable.")?;
    let path = PathBuf::from(home).join("Movies/Hex Game/Recordings");
    fs::create_dir_all(&path).map_err(|error| error.to_string())?;
    Ok(path)
}

fn supported() -> bool {
    cfg!(target_os = "macos")
        && Process::new("/usr/bin/sw_vers")
            .arg("-productVersion")
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| {
                String::from_utf8_lossy(&output.stdout)
                    .split('.')
                    .next()?
                    .trim()
                    .parse::<u32>()
                    .ok()
            })
            .is_some_and(|major| major >= 15)
}

fn worker(requests: Receiver<Command>, responses: SyncSender<Response>) {
    let available = supported();
    if responses
        .send(Response::Supported(
            available,
            if available {
                "Record video · MP4 · 1080p max · 30 fps · no audio"
            } else {
                "Video recording requires macOS 15 or newer."
            }
            .into(),
        ))
        .is_err()
    {
        return;
    }
    let mut active: Option<Clip> = None;
    let mut quitting = false;
    let mut last_failure = None;
    loop {
        match requests.recv_timeout(Duration::from_millis(25)) {
            Ok(Command::Start {
                pid,
                title,
                snapshot,
            }) if active.is_none() && available && !quitting => {
                match Clip::start(pid, title, snapshot) {
                    Ok(clip) => {
                        active = Some(clip);
                        last_failure = None;
                    }
                    Err(error) => {
                        last_failure = Some(error.clone());
                        let _ = responses.send(Response::Failed(error));
                    }
                }
            }
            Ok(Command::Start { .. }) => {
                let _ = responses.send(Response::Failed("Recorder is busy or unsupported.".into()));
            }
            Ok(Command::Stop) => {
                if let Some(clip) = &mut active {
                    if let Err(error) = clip.stop(false) {
                        clip.failure = Some(error);
                    }
                }
            }
            Ok(Command::Quit) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                if quitting {
                    std::thread::sleep(Duration::from_millis(25));
                }
                if !quitting {
                    quitting = true;
                    if let Some(clip) = &mut active {
                        if let Err(error) = clip.stop(true) {
                            clip.failure = Some(error);
                        }
                    }
                }
            }
            Ok(Command::OpenFolder) => {
                let result = folder().and_then(|path| {
                    if !cfg!(target_os = "macos") {
                        return Err("Recording folders are available on macOS.".into());
                    }
                    Process::new("/usr/bin/open")
                        .arg(path)
                        .status()
                        .map_err(|error| error.to_string())
                        .and_then(|status| {
                            if status.success() {
                                Ok(())
                            } else {
                                Err("The recording folder could not be opened.".into())
                            }
                        })
                });
                if let Err(error) = result {
                    let _ = responses.send(Response::Status(error));
                }
            }
            Ok(Command::Event { kind, snapshot }) => {
                if let Some(clip) = &mut active {
                    match clip.event(&kind, snapshot) {
                        Ok(()) if kind == "bookmark" => {
                            let _ = responses.send(Response::Status(format!(
                                "Bookmark saved at {:.1}s",
                                clip.started
                                    .map_or(0.0, |start| start.elapsed().as_secs_f64())
                            )));
                        }
                        Err(error) => clip.failure = Some(error),
                        Ok(()) => {}
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if let Some(clip) = &mut active {
            if let Some(outcome) = clip.poll(&responses) {
                last_failure = outcome.err();
                active = None;
            }
        }
        if quitting && active.is_none() {
            let _ = responses.send(Response::Quit(last_failure.map_or(Ok(()), Err)));
            return;
        }
    }
}

struct Clip {
    identity: SourceIdentity,
    child: Child,
    input: ChildStdin,
    output: Receiver<Result<Value, String>>,
    events: File,
    path: PathBuf,
    requested: Instant,
    started: Option<Instant>,
    stopping: Option<Instant>,
    failure: Option<String>,
    completed: bool,
}

impl Clip {
    fn start(pid: u32, title: String, snapshot: Value) -> Result<Self, String> {
        let helper = prepare_helper()?;
        let root = folder()?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_millis();
        let path = root.join(format!("battle-{timestamp}-{pid}.mp4"));
        let mut events = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path.with_extension("events.jsonl"))
            .map_err(|error| error.to_string())?;
        write_line(
            &mut events,
            &json!({"event":"requested", "unix_ms":timestamp,
            "build":option_env!("HEX_RECORDER_BUILD_ID").unwrap_or("unknown"),
            "version":env!("CARGO_PKG_VERSION"), "platform":std::env::consts::OS,
            "process_id":pid,"window_title":title,"video_path":path,
            "settings":{"codec":"h264","container":"mp4","max_width":1920,"max_height":1080,"fps":30,"audio":false,"dynamic_range":"sdr","queue_depth":3},
            "run":snapshot}),
        )?;
        let mut child = Process::new(helper)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| error.to_string())?;
        let setup = (|| {
            let mut input = child
                .stdin
                .take()
                .ok_or("Recorder control pipe is missing.")?;
            let output = child
                .stdout
                .take()
                .ok_or("Recorder output pipe is missing.")?;
            write_line(
                &mut input,
                &json!({"command":"start","pid":pid,"title":title,"path":path}),
            )?;
            let (send, receive) = mpsc::sync_channel(16);
            std::thread::Builder::new()
                .name("recorder-events".into())
                .spawn(move || {
                    let mut reader = BufReader::new(output);
                    loop {
                        // Read at most 16 KiB per protocol event. An overlong line is
                        // rejected without allocating an unbounded string.
                        let mut bytes = Vec::new();
                        let result = (&mut reader).take(16385).read_until(b'\n', &mut bytes);
                        let event = match result {
                            Ok(0) => break,
                            Ok(_) if bytes.len() <= 16384 => {
                                serde_json::from_slice(&bytes).map_err(|error| error.to_string())
                            }
                            Ok(_) => Err("Recorder sent an oversized status message.".into()),
                            Err(error) => Err(error.to_string()),
                        };
                        let failed = event.is_err();
                        if send.send(event).is_err() || failed {
                            break;
                        }
                    }
                })
                .map_err(|error| error.to_string())?;
            Ok::<_, String>((input, receive))
        })();
        let (input, output) = match setup {
            Ok(value) => value,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        Ok(Self {
            identity: SourceIdentity { pid, window: None },
            child,
            input,
            output,
            events,
            path,
            requested: Instant::now(),
            started: None,
            stopping: None,
            failure: None,
            completed: false,
        })
    }

    fn stop(&mut self, quit: bool) -> Result<(), String> {
        if self.stopping.is_none() {
            write_line(
                &mut self.input,
                &json!({"command":if quit { "quit" } else { "stop" }}),
            )?;
            self.stopping = Some(Instant::now());
            self.event(
                if quit {
                    "quit_requested"
                } else {
                    "stop_requested"
                },
                Value::Null,
            )?;
        }
        Ok(())
    }

    fn event(&mut self, kind: &str, snapshot: Value) -> Result<(), String> {
        write_line(
            &mut self.events,
            &json!({"event":kind,
            "video_seconds":self.started.map(|start| start.elapsed().as_secs_f64()),
            "since_request_seconds":self.requested.elapsed().as_secs_f64(), "run":snapshot}),
        )
    }

    fn poll(&mut self, responses: &SyncSender<Response>) -> Option<Result<(), String>> {
        let mut output_closed = false;
        for _ in 0..16 {
            match self.output.try_recv() {
                Ok(Ok(event)) => {
                    if let Err(error) = self.native_event(event, responses) {
                        self.failure = Some(error);
                    }
                }
                Ok(Err(error)) => self.failure = Some(error),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    output_closed = true;
                    break;
                }
            }
        }
        if self.completed {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return Some(Ok(()));
            }
            if self
                .stopping
                .is_some_and(|time| time.elapsed() > Duration::from_secs(5))
            {
                let _ = self.child.kill();
                let _ = self.child.wait();
                return Some(Ok(()));
            }
            return None;
        }
        if self
            .stopping
            .is_some_and(|time| time.elapsed() > Duration::from_secs(40))
        {
            self.failure =
                Some("Recording finalization timed out; any partial MP4 was retained.".into());
        } else if self.started.is_none() && self.requested.elapsed() > Duration::from_secs(100) {
            self.failure =
                Some("Recording startup timed out. Check macOS screen permission.".into());
        }
        if self.failure.is_none() {
            match self.child.try_wait() {
                Ok(Some(status)) if output_closed => {
                    self.failure = Some(format!(
                        "Recorder exited ({status}) without confirming a finalized MP4."
                    ))
                }
                Ok(_) => {}
                Err(error) => self.failure = Some(error.to_string()),
            }
        }
        if let Some(error) = self.failure.take() {
            let _ = self.event("failed", json!({"message":error}));
            let _ = self.child.kill();
            let _ = self.child.wait();
            let _ = responses.send(Response::Failed(format!("Recording failed: {error}")));
            return Some(Err(error));
        }
        None
    }

    fn native_event(
        &mut self,
        event: Value,
        responses: &SyncSender<Response>,
    ) -> Result<(), String> {
        self.event("native", event.clone())?;
        match event.get("event").and_then(Value::as_str) {
            Some("ready") => {}
            Some("configured") => {
                self.identity.configure(&event)?;
            }
            Some("started") if self.started.is_none() => {
                self.identity.confirm_started(&event)?;
                self.started = Some(Instant::now());
                responses
                    .send(Response::Started)
                    .map_err(|error| error.to_string())?;
            }
            Some("notice") => {
                responses
                    .send(Response::Status(message(&event)))
                    .map_err(|error| error.to_string())?;
            }
            Some("cancelled") if self.started.is_none() => {
                self.events.sync_all().map_err(|error| error.to_string())?;
                self.completed = true;
                self.stopping = Some(Instant::now());
                responses
                    .send(Response::Finished(message(&event)))
                    .map_err(|error| error.to_string())?;
            }
            Some("finished") if self.started.is_some() => {
                if event.get("path").and_then(Value::as_str).map(Path::new)
                    != Some(self.path.as_path())
                {
                    return Err("Recorder finished an unexpected output path.".into());
                }
                if fs::metadata(&self.path)
                    .map_err(|error| error.to_string())?
                    .len()
                    == 0
                {
                    return Err("Recorder output is empty.".into());
                }
                self.events.sync_all().map_err(|error| error.to_string())?;
                self.completed = true;
                self.stopping = Some(Instant::now());
                responses
                    .send(Response::Finished(format!("Saved {}", self.path.display())))
                    .map_err(|error| error.to_string())?;
            }
            Some("failed") => return Err(message(&event)),
            _ => return Err("Recorder sent an invalid lifecycle transition.".into()),
        }
        Ok(())
    }
}

impl Drop for Clip {
    fn drop(&mut self) {
        // Reap only after the worker has finalized or reported a bounded failure.
        if !self.completed {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

#[derive(Default)]
struct SourceIdentity {
    pid: u32,
    window: Option<u64>,
}

impl SourceIdentity {
    fn configure(&mut self, event: &Value) -> Result<(), String> {
        let number = |key| event.get(key).and_then(Value::as_u64);
        let valid = self.window.is_none()
            && number("pid") == Some(u64::from(self.pid))
            && number("window_id").is_some_and(|id| id > 0)
            && number("width").is_some_and(|size| size > 0 && size <= 1920)
            && number("height").is_some_and(|size| size > 0 && size <= 1080)
            && number("fps") == Some(30)
            && event.get("audio").and_then(Value::as_bool) == Some(false)
            && event.get("codec").and_then(Value::as_str) == Some("h264");
        if !valid {
            return Err(
                "The recorder did not confirm the requested game process and video-only settings."
                    .into(),
            );
        }
        self.window = number("window_id");
        Ok(())
    }
    fn confirm_started(&self, event: &Value) -> Result<(), String> {
        if self.window.is_none() || event.get("window_id").and_then(Value::as_u64) != self.window {
            return Err("Recording started without the validated game-window identity.".into());
        }
        Ok(())
    }
}

fn message(value: &Value) -> String {
    value
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("Unknown native recording error.")
        .to_owned()
}

fn write_line(writer: &mut impl Write, value: &Value) -> Result<(), String> {
    serde_json::to_writer(&mut *writer, value).map_err(|error| error.to_string())?;
    writer
        .write_all(b"\n")
        .and_then(|()| writer.flush())
        .map_err(|error| error.to_string())
}

#[cfg(target_os = "macos")]
fn prepare_helper() -> Result<PathBuf, String> {
    use std::os::unix::fs::PermissionsExt;
    const BINARY: &[u8] = include_bytes!(env!("HEX_RECORDER_BINARY"));
    const PLIST: &str = include_str!("../../../native/recorder/Info.plist");
    let home = std::env::var_os("HOME").ok_or("The home folder is unavailable.")?;
    let contents = PathBuf::from(home)
        .join("Library/Application Support/Hex Game/Recorder/Hex Game Recorder.app/Contents");
    let executable = contents.join("MacOS/hex-game-recorder");
    fs::create_dir_all(contents.join("MacOS")).map_err(|error| error.to_string())?;
    if fs::read(&executable).ok().as_deref() != Some(BINARY) {
        let temporary = executable.with_extension(format!("{}.new", std::process::id()));
        fs::write(&temporary, BINARY).map_err(|error| error.to_string())?;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o755))
            .map_err(|error| error.to_string())?;
        fs::rename(temporary, &executable).map_err(|error| error.to_string())?;
    }
    fs::write(contents.join("Info.plist"), PLIST).map_err(|error| error.to_string())?;
    Ok(executable)
}

#[cfg(not(target_os = "macos"))]
fn prepare_helper() -> Result<PathBuf, String> {
    Err("Video recording requires macOS 15 or newer.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn native_failure_remains_an_error_after_child_cleanup_for_quit() {
        let path = std::env::temp_dir().join(format!(
            "hex-recorder-failure-{}-{}.events.jsonl",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("epoch")
                .as_nanos()
        ));
        let mut child = Process::new("/bin/cat")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("inert child");
        let input = child.stdin.take().expect("pipe");
        let (native, output) = mpsc::sync_channel(4);
        let (responses, received) = mpsc::sync_channel(4);
        let mut clip = Clip {
            identity: SourceIdentity {
                pid: 42,
                window: Some(7),
            },
            child,
            input,
            output,
            events: OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&path)
                .expect("metadata"),
            path: path.with_extension("mp4"),
            requested: Instant::now(),
            started: Some(Instant::now()),
            stopping: Some(Instant::now()),
            failure: None,
            completed: false,
        };
        native
            .send(Ok(
                json!({"event":"failed","message":"disk full during finalization"}),
            ))
            .expect("native event");
        let outcome = clip.poll(&responses).expect("terminal cleanup");
        assert_eq!(outcome, Err("disk full during finalization".to_owned()));
        assert!(
            matches!(received.try_recv(), Ok(Response::Failed(message)) if message.contains("disk full"))
        );
        assert!(!clip.completed, "failed cleanup is not MP4 completion");
        assert!(clip.child.try_wait().expect("child state").is_some());
        drop(clip);
        fs::remove_file(path).expect("remove owned metadata fixture");
    }

    #[test]
    fn native_start_requires_the_confirmed_process_window_and_bounded_video_settings() {
        let configured = json!({"pid":42,"window_id":7,"width":1600,"height":900,"fps":30,"audio":false,"codec":"h264"});
        let mut identity = SourceIdentity {
            pid: 42,
            window: None,
        };
        assert!(identity.confirm_started(&json!({"window_id":7})).is_err());
        assert!(SourceIdentity {
            pid: 99,
            window: None
        }
        .configure(&configured)
        .is_err());
        let mut audio = configured.clone();
        *audio.get_mut("audio").expect("audio field") = json!(true);
        assert!(identity.configure(&audio).is_err());
        identity.configure(&configured).expect("exact video source");
        assert!(identity.confirm_started(&json!({"window_id":8})).is_err());
        identity
            .confirm_started(&json!({"window_id":7}))
            .expect("same window");
        assert!(
            identity.configure(&configured).is_err(),
            "source cannot silently switch"
        );
    }

    #[test]
    fn protocol_preserves_paths_unicode_and_one_event_per_line() {
        let mut bytes = Vec::new();
        let event = json!({"path":"/tmp/Hex Game/a\"b\\c.mp4", "title":"Hex — Spell Arena"});
        write_line(&mut bytes, &event).expect("serialize");
        assert_eq!(bytes.iter().filter(|byte| **byte == b'\n').count(), 1);
        assert_eq!(
            serde_json::from_slice::<Value>(&bytes).expect("parse"),
            event
        );
    }
}
