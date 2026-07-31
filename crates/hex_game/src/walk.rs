//! Scripted visual walk: drive the real game through its screens and
//! photograph each step, so an agent (or a human) can look at the frames.
//!
//! Compiled only with the default-off `visual-walk` feature. Setting
//! `HEX_WALK_SCRIPT` to a RON step list and `HEX_WALK_OUT` to an output
//! directory runs the walk on launch: the runner advances one step at a time
//! (waiting for screens, settling frames, injecting clicks and keys, capturing
//! PNGs) and exits with success only if every step completed. A per-step
//! watchdog turns a stall into a diagnostic and a failing exit instead of a
//! hang. `HEX_WALK_SIZE=1280x720` optionally selects an exact review viewport;
//! the default is 1920×1080.
//!
//! # Why clicks are injected as `Interaction::Pressed`
//!
//! `bevy_ui`'s focus system only resets a node's `Interaction` when it is not
//! `Pressed` — an injected press on a button the real cursor is nowhere near
//! is deliberately left alone ("press sticks until release"). Every handler in
//! this game reads `Changed<Interaction>` + `== Pressed`, so one injected
//! insert is exactly one activation, exercised through the real button wiring
//! rather than a state-bypass. The runner clears the press to
//! `Interaction::None` on the following step for buttons that outlive their
//! click. Keys go through `ButtonInput::press` from `PreUpdate`, after the
//! input plugin's frame clear, so `just_pressed` is visible to every `Update`
//! reader in the same frame.

use std::env;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use bevy::camera::RenderTarget;
use bevy::ecs::system::SystemParam;
use bevy::input::InputSystems;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use hex_assets::ScenarioLibrary;
use hex_combat::{CombatSummary, EncounterOutcome, EncounterResolution};
use hex_core::{GameplaySetupFailure, HexTile, ResolvedMapSeed, Screen};
use serde::Deserialize;

use crate::capture::write_png;
use crate::scenarios::ScenarioToLoad;

const SCRIPT_ENV: &str = "HEX_WALK_SCRIPT";
const OUT_ENV: &str = "HEX_WALK_OUT";
const SIZE_ENV: &str = "HEX_WALK_SIZE";
const STEP_TIMEOUT: Duration = Duration::from_secs(60);
const WALK_TIME_SCALE: f32 = 12.0;
const DEFAULT_WALK_WIDTH: u32 = 1920;
const DEFAULT_WALK_HEIGHT: u32 = 1080;

/// Installs the walk runner only when its environment is present.
pub(super) fn plugin(app: &mut App) {
    let script = env::var(SCRIPT_ENV).ok();
    let out = env::var(OUT_ENV).ok();
    let (script, out) = match (script, out) {
        (None, None) => return,
        (Some(script), Some(out)) => (script, out),
        (Some(_), None) => {
            install_config_error(app, format!("{SCRIPT_ENV} requires {OUT_ENV}"));
            return;
        }
        (None, Some(_)) => {
            install_config_error(app, format!("{OUT_ENV} requires {SCRIPT_ENV}"));
            return;
        }
    };

    let steps = match load_script(&script) {
        Ok(steps) => steps,
        Err(error) => {
            install_config_error(app, error);
            return;
        }
    };
    let size = match env::var(SIZE_ENV) {
        Ok(size) => match parse_size(&size) {
            Ok(size) => size,
            Err(error) => {
                install_config_error(app, error);
                return;
            }
        },
        Err(env::VarError::NotPresent) => (DEFAULT_WALK_WIDTH, DEFAULT_WALK_HEIGHT),
        Err(error) => {
            install_config_error(app, format!("cannot read {SIZE_ENV}: {error}"));
            return;
        }
    };

    info!(
        "visual walk: {} steps from {script}, output to {out} at {}x{}",
        steps.len(),
        size.0,
        size.1
    );
    app.insert_resource(WalkState::new(steps, PathBuf::from(out), size))
        .add_systems(Startup, accelerate_walk_time)
        .add_systems(PreUpdate, run_walk.after(InputSystems));
}

fn accelerate_walk_time(mut time: ResMut<Time<Virtual>>) {
    // Walks exercise the ordinary animation/timer systems, but should not spend
    // wall-clock minutes waiting through every combatant in a 6v6 matrix.
    time.set_relative_speed(WALK_TIME_SCALE);
}

fn install_config_error(app: &mut App, error: String) {
    app.insert_resource(WalkConfigurationError(error))
        .add_systems(Startup, reject_invalid_configuration);
}

#[derive(Resource, Debug)]
struct WalkConfigurationError(String);

fn reject_invalid_configuration(
    error: Res<WalkConfigurationError>,
    mut exit: MessageWriter<AppExit>,
) {
    error!("invalid visual-walk configuration: {}", error.0);
    exit.write(AppExit::error());
}

/// One scripted action. The RON script is a `Vec<WalkStep>`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
enum WalkStep {
    /// Wait until the app is in the named screen.
    AwaitScreen(String),
    /// Wait until validated terrain is ready (gameplay only).
    AwaitTerrain,
    /// Let this many frames pass before the next step.
    Settle(u32),
    /// Photograph the primary window into `<out>/<name>.png`.
    Capture(String),
    /// Press the `index`-th button whose `Name` starts with `name`.
    Click {
        name: String,
        #[serde(default)]
        index: usize,
    },
    /// Wait until a named button exists without activating it.
    AwaitButton(String),
    /// Open a canonical fixture report for presentation review without solving combat.
    ///
    /// This feature-only step deliberately supplies a terminal result to the ordinary
    /// report system. Logical outcome and metric evidence belongs to the deterministic
    /// simulation target; the visual walk reviews only the resulting composition.
    PresentFixtureReport,
    /// Press and release a supported gameplay or menu key.
    Key(String),
    /// Launch a scenario by exact name, bypassing the menu UI.
    StartScenario {
        name: String,
        #[serde(default)]
        seed: Option<u64>,
    },
    /// Launch an immutable Combat Lab fixture by stable machine id.
    StartFixture {
        id: String,
        #[serde(default)]
        profile: Option<String>,
    },
}

fn load_script(path: &str) -> Result<Vec<WalkStep>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read {SCRIPT_ENV} {path}: {error}"))?;
    let steps: Vec<WalkStep> =
        ron::from_str(&text).map_err(|error| format!("cannot parse {path}: {error}"))?;
    if steps.is_empty() {
        return Err(format!("{path} contains no steps"));
    }
    for (index, step) in steps.iter().enumerate() {
        validate_step(step).map_err(|error| format!("{path} step {index}: {error}"))?;
    }
    Ok(steps)
}

fn validate_step(step: &WalkStep) -> Result<(), String> {
    match step {
        WalkStep::AwaitScreen(name) => parse_screen(name).map(|_| ()),
        WalkStep::Key(name) => parse_key(name).map(|_| ()),
        WalkStep::Capture(name) if name.trim().is_empty() => {
            Err("capture name must not be empty".to_owned())
        }
        WalkStep::Click { name, .. } if name.trim().is_empty() => {
            Err("click name must not be empty".to_owned())
        }
        WalkStep::AwaitButton(name) if name.trim().is_empty() => {
            Err("awaited button name must not be empty".to_owned())
        }
        WalkStep::StartScenario { name, .. } if name.trim().is_empty() => {
            Err("scenario name must not be empty".to_owned())
        }
        WalkStep::StartFixture { id, .. } if id.trim().is_empty() => {
            Err("fixture id must not be empty".to_owned())
        }
        _ => Ok(()),
    }
}

fn parse_screen(name: &str) -> Result<Screen, String> {
    match name {
        "Splash" => Ok(Screen::Splash),
        "Title" => Ok(Screen::Title),
        "Settings" => Ok(Screen::Settings),
        "LatticeDemo" => Ok(Screen::LatticeDemo),
        "CharacterCreator" => Ok(Screen::CharacterCreator),
        "SpellCreator" => Ok(Screen::SpellCreator),
        "CombatLab" => Ok(Screen::CombatLab),
        "Loading" => Ok(Screen::Loading),
        "Gameplay" => Ok(Screen::Gameplay),
        _ => Err(format!(
            "unknown screen {name:?}; expected Splash, Title, Settings, CharacterCreator, SpellCreator, CombatLab, LatticeDemo, Loading, or Gameplay"
        )),
    }
}

fn parse_key(name: &str) -> Result<KeyCode, String> {
    match name {
        "Backspace" => Ok(KeyCode::Backspace),
        "Escape" => Ok(KeyCode::Escape),
        "Space" => Ok(KeyCode::Space),
        "Enter" => Ok(KeyCode::Enter),
        "C" => Ok(KeyCode::KeyC),
        // `Tab` and `Q` are casting's — step to the next target, and put the aim down.
        // `Enter` confirms an aim, and the casting walk drives that through the panel's
        // Confirm button instead: a `Click` that never finds its button stalls the walk
        // and fails it, which is a stronger assertion than a key that can be pressed
        // into a screen with no aiming UI on it at all.
        "Tab" => Ok(KeyCode::Tab),
        "KeyQ" => Ok(KeyCode::KeyQ),
        "KeyH" => Ok(KeyCode::KeyH),
        "KeyL" => Ok(KeyCode::KeyL),
        "KeyR" => Ok(KeyCode::KeyR),
        "F5" => Ok(KeyCode::F5),
        _ => Err(format!(
            "unknown key {name:?}; expected Backspace, Escape, Space, Enter, C, Tab, KeyQ, KeyH, KeyL, KeyR, or F5"
        )),
    }
}

fn parse_size(size: &str) -> Result<(u32, u32), String> {
    let invalid = || format!("{SIZE_ENV} must be WIDTHxHEIGHT with two positive integers");
    let Some((width, height)) = size.split_once('x') else {
        return Err(invalid());
    };
    if height.contains('x') {
        return Err(invalid());
    }
    let width = width
        .parse::<u32>()
        .map_err(|error| format!("{SIZE_ENV} width is invalid: {error}"))?;
    let height = height
        .parse::<u32>()
        .map_err(|error| format!("{SIZE_ENV} height is invalid: {error}"))?;
    if width == 0 || height == 0 {
        return Err(invalid());
    }
    Ok((width, height))
}

/// What the capture observer reports back to the runner.
#[derive(Debug)]
enum CaptureOutcome {
    Written { brightest: u8, coverage: bool },
    Failed(String),
}

#[derive(Resource)]
struct WalkState {
    steps: Vec<WalkStep>,
    cursor: usize,
    out_dir: PathBuf,
    settled: u32,
    step_started: Instant,
    capture_requested: bool,
    capture_outcome: Option<CaptureOutcome>,
    /// A button pressed by the previous step, to be reset to `None`.
    pressed: Option<Entity>,
    /// A key pressed by the previous step, to be released.
    held_key: Option<KeyCode>,
    /// The offscreen image the camera renders into for capture.
    ///
    /// The window surface is not readable on every backend — on macOS/Metal a
    /// `Screenshot::primary_window()` comes back black — so the walk redirects
    /// the game's single camera into a target image exactly as the map-review
    /// harness does. `bevy_ui` only follows a *window*-targeting camera by
    /// default, so the runner also tags every UI root with `UiTargetCamera`
    /// pointing at the redirected camera; frames then show everything a
    /// player would see.
    target: Option<Handle<Image>>,
    /// The camera entity the UI roots must be pointed at.
    camera: Option<Entity>,
    /// Exact offscreen viewport under review.
    size: (u32, u32),
    failed: bool,
}

#[derive(SystemParam)]
struct WalkCombat<'w> {
    resolution: Option<ResMut<'w, EncounterResolution>>,
    summary: Option<ResMut<'w, CombatSummary>>,
}

#[derive(SystemParam)]
struct WalkContent<'w> {
    failure: Option<Res<'w, GameplaySetupFailure>>,
    library: Option<Res<'w, ScenarioLibrary>>,
    presets: Option<Res<'w, hex_assets::CreationPresetCatalog>>,
    shipped_spells: Option<Res<'w, hex_assets::SpellFile>>,
    base_lattices: Option<Res<'w, hex_assets::LatticeFile>>,
    elements: Option<Res<'w, hex_assets::ElementCatalog>>,
    substances: Option<Res<'w, hex_assets::SubstanceTable>>,
    combat: Option<Res<'w, hex_assets::CombatSettings>>,
}

impl WalkState {
    fn new(steps: Vec<WalkStep>, out_dir: PathBuf, size: (u32, u32)) -> Self {
        Self {
            steps,
            cursor: 0,
            out_dir,
            settled: 0,
            step_started: Instant::now(),
            capture_requested: false,
            capture_outcome: None,
            pressed: None,
            held_key: None,
            target: None,
            camera: None,
            size,
            failed: false,
        }
    }

    fn advance(&mut self) {
        self.cursor += 1;
        self.settled = 0;
        self.capture_requested = false;
        self.capture_outcome = None;
        self.step_started = Instant::now();
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the runner is one sequential state machine over the app's surfaces; \
              splitting it would scatter the step semantics across systems"
)]
fn run_walk(
    mut commands: Commands,
    mut state: ResMut<WalkState>,
    screen: Res<State<Screen>>,
    mut next: ResMut<NextState<Screen>>,
    mut combat: WalkCombat,
    content: WalkContent,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    buttons: Query<(Entity, &Name), With<Button>>,
    tiles: Query<Entity, With<HexTile>>,
    mut images: ResMut<Assets<Image>>,
    mut camera_targets: Query<(Entity, &mut RenderTarget), With<Camera>>,
    ui_roots: Query<Entity, (With<Node>, Without<ChildOf>, Without<UiTargetCamera>)>,
    mut exit: MessageWriter<AppExit>,
) {
    if state.failed {
        return;
    }

    // Redirect the game's single camera into a readable offscreen image before
    // anything is photographed; see `WalkState::target`.
    if state.target.is_none() {
        let Ok((camera, mut render_target)) = camera_targets.single_mut() else {
            return;
        };
        let image = Image::new_target_texture(
            state.size.0,
            state.size.1,
            TextureFormat::Rgba8UnormSrgb,
            None,
        );
        let handle = images.add(image);
        *render_target = RenderTarget::Image(handle.clone().into());
        state.target = Some(handle);
        state.camera = Some(camera);
    }

    // UI roots spawn and despawn with every screen; keep pointing new ones at
    // the redirected camera or their screens render into nothing.
    if let Some(camera) = state.camera {
        for root in &ui_roots {
            commands.entity(root).insert(UiTargetCamera(camera));
        }
    }
    if let Some(failure) = content.failure.as_deref() {
        error!(
            "visual walk aborted: gameplay setup failed: {}",
            failure.reason
        );
        state.failed = true;
        exit.write(AppExit::error());
        return;
    }

    // Cleanups owed from the previous step, regardless of what runs now.
    if let Some(entity) = state.pressed.take() {
        if buttons.contains(entity) {
            commands.entity(entity).insert(Interaction::None);
        }
    }
    if let Some(key) = state.held_key.take() {
        keys.release(key);
    }

    let Some(step) = state.steps.get(state.cursor).cloned() else {
        info!("visual walk complete: {} steps", state.steps.len());
        exit.write(AppExit::Success);
        state.failed = true;
        return;
    };

    if state.step_started.elapsed() > STEP_TIMEOUT {
        error!(
            "visual walk timed out on step {} ({step:?}) after {:.0}s on screen {:?}",
            state.cursor,
            STEP_TIMEOUT.as_secs_f32(),
            screen.get()
        );
        state.failed = true;
        exit.write(AppExit::error());
        return;
    }

    match step {
        WalkStep::AwaitScreen(ref name) => {
            let wanted = parse_screen(name).unwrap_or(Screen::Title);
            if *screen.get() == wanted {
                state.advance();
            }
        }
        WalkStep::AwaitTerrain => {
            if !tiles.is_empty() {
                state.advance();
            }
        }
        WalkStep::Settle(frames) => {
            state.settled += 1;
            if state.settled >= frames {
                state.advance();
            }
        }
        WalkStep::Capture(ref name) => {
            if !state.capture_requested {
                let Some(target) = state.target.clone() else {
                    return;
                };
                let path = state.out_dir.join(format!("{name}.png"));
                info!("visual walk capturing {}", path.display());
                commands.spawn(Screenshot::image(target)).observe(
                    move |captured: On<ScreenshotCaptured>, mut state: ResMut<WalkState>| {
                        let outcome = match write_png(&captured.image, &path) {
                            Ok(stats) => CaptureOutcome::Written {
                                brightest: stats.brightest,
                                coverage: stats.has_coverage,
                            },
                            Err(error) => CaptureOutcome::Failed(error),
                        };
                        state.capture_outcome = Some(outcome);
                    },
                );
                state.capture_requested = true;
                return;
            }
            match state.capture_outcome.take() {
                None => {}
                Some(CaptureOutcome::Written {
                    brightest,
                    coverage,
                }) if brightest > 8 => {
                    info!("visual walk captured {name} (coverage: {coverage})");
                    state.advance();
                }
                Some(CaptureOutcome::Written { .. }) => {
                    error!("visual walk capture {name} came back black; failing the walk");
                    state.failed = true;
                    exit.write(AppExit::error());
                }
                Some(CaptureOutcome::Failed(error)) => {
                    error!("visual walk capture {name} failed: {error}");
                    state.failed = true;
                    exit.write(AppExit::error());
                }
            }
        }
        WalkStep::Click { ref name, index } => {
            let mut matches: Vec<(Entity, String)> = buttons
                .iter()
                .filter(|(_, button_name)| button_name.as_str().starts_with(name.as_str()))
                .map(|(entity, button_name)| (entity, button_name.as_str().to_owned()))
                .collect();
            // Query iteration order is arbitrary; entity order tracks spawn
            // order within one screen build, which is deterministic.
            matches.sort_by_key(|&(entity, _)| entity);
            let Some(&(entity, ref matched)) = matches.get(index) else {
                return; // keep waiting; the watchdog reports if it never appears
            };
            info!(
                "visual walk clicking {matched:?} ({} of {})",
                index,
                matches.len()
            );
            commands.entity(entity).insert(Interaction::Pressed);
            state.pressed = Some(entity);
            state.advance();
        }
        WalkStep::AwaitButton(ref name) => {
            if buttons
                .iter()
                .any(|(_, button_name)| button_name.as_str().starts_with(name.as_str()))
            {
                state.advance();
            }
        }
        WalkStep::PresentFixtureReport => {
            let (Some(resolution), Some(summary)) = (
                combat.resolution.as_deref_mut(),
                combat.summary.as_deref_mut(),
            ) else {
                return;
            };
            resolution.0 = Some(EncounterOutcome::Victory);
            summary.outcome = Some(EncounterOutcome::Victory);
            state.advance();
        }
        WalkStep::Key(ref name) => {
            let key = parse_key(name).unwrap_or(KeyCode::Escape);
            info!("visual walk pressing {name}");
            keys.press(key);
            state.held_key = Some(key);
            state.advance();
        }
        WalkStep::StartScenario { ref name, seed } => {
            let Some(library) = content.library.as_deref() else {
                return;
            };
            let Some(scenario) = library
                .scenarios
                .iter()
                .find(|scenario| scenario.name == *name)
                .cloned()
            else {
                error!("visual walk: scenario {name:?} is not in scenarios.ron");
                state.failed = true;
                exit.write(AppExit::error());
                return;
            };
            let resolved_seed = seed.or(scenario.generation_seed).map(ResolvedMapSeed);
            info!("visual walk launching scenario {name:?}");
            commands.insert_resource(ScenarioToLoad {
                scenario,
                resolved_seed,
                encounter_override: None,
            });
            next.set(Screen::Loading);
            state.advance();
        }
        WalkStep::StartFixture {
            ref id,
            ref profile,
        } => {
            let Some(combat_settings) = content.combat.as_deref() else {
                return;
            };
            let Some(library) = content.library.as_deref() else {
                return;
            };
            let Some(name) = crate::screens::combat_lab::fixture_scenario_name(id) else {
                error!("visual walk: fixture {id:?} is not registered");
                state.failed = true;
                exit.write(AppExit::error());
                return;
            };
            let Some(scenario) = library
                .scenarios
                .iter()
                .find(|scenario| scenario.name == name)
                .cloned()
            else {
                error!("visual walk: fixture {id:?} scenario {name:?} is missing");
                state.failed = true;
                exit.write(AppExit::error());
                return;
            };
            let resolved_seed = scenario.generation_seed.map(ResolvedMapSeed);
            let profile = match crate::screens::combat_lab::walk_fixture_profile(
                profile.as_deref(),
                combat_settings,
            ) {
                Ok(profile) => profile,
                Err(reason) => {
                    error!("visual walk: {reason}");
                    state.failed = true;
                    exit.write(AppExit::error());
                    return;
                }
            };
            // Direct fixture-to-fixture transitions do not pass through Combat
            // Lab initialization, so explicitly retire the prior run's frozen
            // launch and completed report before installing the next session.
            commands.remove_resource::<crate::screens::combat_lab::CombatLabReportLaunch>();
            commands.remove_resource::<crate::combat_reports::CurrentCombatLabReport>();
            commands.insert_resource(crate::screens::combat_lab::CombatLabSession {
                kind: crate::screens::combat_lab::CombatLabSessionKind::FixedFixture(id.clone()),
                return_to: Screen::CombatLab,
                profile,
                shipped_combat: combat_settings.clone(),
                report_map: crate::combat_reports::CombatLabReportMap {
                    catalog_id: crate::screens::combat_lab::fixture_sandbox_map(id)
                        .unwrap_or(id)
                        .to_owned(),
                    scenario: scenario.name.clone(),
                    resolved_seed: resolved_seed.map(|seed| seed.0),
                },
                initial_state: (id == "channel-attrition").then_some(
                    crate::screens::combat_lab::FixedFixtureInitialState::ChannelAttrition,
                ),
            });
            let payload = match crate::screens::combat_lab::creator_fixture_payload(
                id,
                content.presets.as_deref(),
                content.shipped_spells.as_deref(),
                content.base_lattices.as_deref(),
                content.elements.as_deref(),
                content.substances.as_deref(),
            ) {
                Ok(payload) => payload,
                Err(reason) => {
                    error!("visual walk: fixture {id:?} is invalid: {reason}");
                    state.failed = true;
                    exit.write(AppExit::error());
                    return;
                }
            };
            let encounter_override = payload
                .map(|(overlay, encounter)| {
                    commands.insert_resource(overlay);
                    encounter
                })
                .or_else(|| crate::screens::combat_lab::walk_fixture_encounter(id));
            commands.insert_resource(ScenarioToLoad {
                scenario,
                resolved_seed,
                encounter_override,
            });
            next.set(Screen::Loading);
            state.advance();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL_SCRIPT: &str = r#"[
        AwaitScreen("Title"),
        Settle(30),
        Capture("01-title"),
        Click(name: "Combat Lab"),
        AwaitScreen("CombatLab"),
        Key("Backspace"),
        StartScenario(name: "The Crossing"),
        StartFixture(id: "ability-lab"),
        AwaitTerrain,
        Key("KeyR"),
        AwaitButton("Cast Ember"),
        PresentFixtureReport,
        Capture("02-crossing"),
    ]"#;

    #[test]
    fn a_full_script_parses_with_every_step_kind() {
        let steps: Vec<WalkStep> = ron::from_str(FULL_SCRIPT).expect("script parses");
        assert_eq!(steps.len(), 13);
        assert_eq!(steps.first(), Some(&WalkStep::AwaitScreen("Title".into())));
        assert_eq!(
            steps.get(3),
            Some(&WalkStep::Click {
                name: "Combat Lab".into(),
                index: 0
            })
        );
        assert_eq!(
            steps.get(6),
            Some(&WalkStep::StartScenario {
                name: "The Crossing".into(),
                seed: None
            })
        );
        assert_eq!(
            steps.get(7),
            Some(&WalkStep::StartFixture {
                id: "ability-lab".into(),
                profile: None,
            })
        );
        for step in &steps {
            validate_step(step).expect("every step validates");
        }
    }

    #[test]
    fn unknown_screens_and_keys_are_rejected_at_load() {
        assert_eq!(parse_key("KeyH"), Ok(KeyCode::KeyH));
        assert_eq!(parse_key("KeyL"), Ok(KeyCode::KeyL));
        assert_eq!(parse_key("KeyR"), Ok(KeyCode::KeyR));
        assert!(validate_step(&WalkStep::AwaitScreen("Menu".into())).is_err());
        assert!(validate_step(&WalkStep::Key("F13".into())).is_err());
        assert!(validate_step(&WalkStep::Capture(" ".into())).is_err());
        assert!(validate_step(&WalkStep::Click {
            name: String::new(),
            index: 0
        })
        .is_err());
        assert!(validate_step(&WalkStep::AwaitButton(" ".into())).is_err());
    }

    #[test]
    fn capture_size_is_explicit_and_positive() {
        assert_eq!(parse_size("1280x720"), Ok((1280, 720)));
        assert_eq!(parse_size("1920x1080"), Ok((1920, 1080)));
        for invalid in ["", "1280", "1280X720", "x720", "1280x", "0x720", "1280x0"] {
            assert!(parse_size(invalid).is_err(), "{invalid:?} should fail");
        }
    }

    #[test]
    fn every_screen_name_round_trips() {
        for name in [
            "Splash",
            "Title",
            "CharacterCreator",
            "CombatLab",
            "LatticeDemo",
            "Loading",
            "Gameplay",
        ] {
            parse_screen(name).expect("known screen parses");
        }
        assert!(parse_screen("Gameplay ").is_err());
    }

    #[test]
    fn the_shipped_walk_scripts_parse_and_validate() {
        for script in [
            "../../walks/gameplay_ui.ron",
            "../../walks/waterfall.ron",
            "../../walks/forest.ron",
        ] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(script);
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
            let steps: Vec<WalkStep> = ron::from_str(&text)
                .unwrap_or_else(|error| panic!("cannot parse {}: {error}", path.display()));
            assert!(!steps.is_empty());
            for step in &steps {
                validate_step(step)
                    .unwrap_or_else(|error| panic!("{} invalid: {error}", path.display()));
            }
        }
    }

    #[test]
    fn scoped_gameplay_acceptance_stays_within_the_frame_budget() {
        let steps: Vec<WalkStep> = ron::from_str(include_str!("../../../walks/gameplay_ui.ron"))
            .expect("the gameplay UI walk parses");
        let captures = steps
            .iter()
            .filter(|step| matches!(step, WalkStep::Capture(_)))
            .count();
        assert!(
            captures <= 10,
            "scoped gameplay acceptance captured {captures} frames; the contract permits 10"
        );
    }

    #[test]
    fn forest_walk_pins_the_shipped_hero_seed() {
        let library: ScenarioLibrary =
            ron::from_str(include_str!("../../../assets/config/scenarios.ron"))
                .expect("the shipped scenario library parses");
        let hero_seed = library
            .scenarios
            .iter()
            .find(|scenario| scenario.name == "Forest")
            .and_then(|scenario| scenario.generation_seed)
            .expect("the shipped Forest scenario has a hero seed");
        let steps: Vec<WalkStep> = ron::from_str(include_str!("../../../walks/forest.ron"))
            .expect("the shipped Forest walk parses");

        assert!(steps.contains(&WalkStep::StartScenario {
            name: "Forest".to_owned(),
            seed: Some(hero_seed),
        }));
    }

    /// Every scenario a shipped walk starts must still exist in `scenarios.ron`.
    ///
    /// Parsing proves a script is well-formed; it does not prove it still points at
    /// anything. `validate_step` only rejects an *empty* name, so renaming a scenario
    /// leaves the walk naming a ghost — and that sails through fmt, clippy, the whole
    /// suite and all six CI jobs, failing only when a person runs the walk by hand.
    /// The wave that added encounters rewrote `scenarios.ron` and got away with it
    /// because it happened to rename nothing.
    #[test]
    fn every_walk_scenario_name_resolves() {
        let library: ScenarioLibrary =
            ron::from_str(include_str!("../../../assets/config/scenarios.ron"))
                .expect("scenarios.ron parses");
        let known: Vec<&str> = library
            .scenarios
            .iter()
            .map(|scenario| scenario.name.as_str())
            .collect();

        let mut checked = 0;
        let mut launches_default = false;
        let mut continues_save = false;
        let mut launches_fixture = false;
        for script in ["../../walks/gameplay_ui.ron"] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(script);
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
            let steps: Vec<WalkStep> = ron::from_str(&text)
                .unwrap_or_else(|error| panic!("cannot parse {}: {error}", path.display()));
            for step in &steps {
                if let WalkStep::StartScenario { name, .. } = step {
                    assert!(
                        known.contains(&name.as_str()),
                        "{} starts {name:?}, which is not in scenarios.ron; it offers {known:?}",
                        path.display(),
                    );
                    checked += 1;
                }
                if matches!(step, WalkStep::Click { name, .. } if name == "New Game") {
                    launches_default = true;
                }
                if matches!(step, WalkStep::Click { name, .. } if name == "Continue") {
                    continues_save = true;
                }
                if let WalkStep::StartFixture { id, .. } = step {
                    assert!(
                        crate::screens::combat_lab::fixture_scenario_name(id).is_some(),
                        "{} starts unknown fixture {id:?}",
                        path.display()
                    );
                    launches_fixture = true;
                }
            }
        }
        assert!(
            checked > 0 || launches_default || continues_save || launches_fixture,
            "the UI walk must exercise at least one real application launch path"
        );
    }
}
