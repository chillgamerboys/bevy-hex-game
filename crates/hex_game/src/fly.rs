//! One-pawn walk/fly exploration for inspecting development maps.
//!
//! This module is compiled only as part of the `dev` feature. A launch may come
//! from `HEX_FLY_MAP` or from the small Grand V3 shortcut on the title screen;
//! both are resolved through the shipped Sandbox catalog and use the ordinary
//! scenario loading pipeline. Once gameplay starts, the only player piece and
//! camera move together. Grounded exploration uses public collision geometry;
//! tactical gameplay authority remains suspended.

use std::env;
use std::ffi::OsString;

use bevy::input::InputSystems;
use bevy::input_focus::{InputFocus, InputFocusSystems};
use bevy::prelude::*;
use bevy::transform::TransformSystems;
use bevy::ui::InteractionDisabled;
use bevy::window::PrimaryWindow;
use hex_assets::{
    CameraSettings, Encounter, EncounterFaction, EncounterPlacement, LoadSettings, Roster,
    RosterEntry, SandboxMapCatalog, SandboxMapDefinition, SandboxRegionCenter, Scenario,
    ScenarioLibrary,
};
use hex_core::{
    GameplayPhase, GameplaySetup, GameplaySetupFailure, InputAction, InputBindings, KeyModifiers,
    Pause, PresentationOcclusion, PresentationOcclusionReason, PresentationSystems,
    ResolvedMapSeed, Screen,
};
use hex_gameplay_model::{MainMenuModel, MainMenuRoute};
use hex_ui::{fine, label, stacked_row_button, DespawnOnExit, UiAssets, UiVisibilityRequirement};
use hex_units::Player;
use hex_world::{CameraMode, CameraSystems, PanOrbitCamera};

use crate::scenarios::ScenarioToLoad;

mod collision;
mod controller;
use collision::CollisionWorld;
use controller::{Body, Intent, Mode, Settings};

const FLY_MAP_ENV: &str = "HEX_FLY_MAP";
const GRAND_V3_MAP_ID: &str = "grand-v3-baseline";
const FLY_SPEED: f32 = 25.0;

/// A typed request shared by environment and title-button launches.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub(crate) struct FlyLaunchRequest {
    map_id: String,
    source: FlyLaunchSource,
    launched: bool,
}

impl FlyLaunchRequest {
    fn environment(map_id: String) -> Self {
        Self {
            map_id,
            source: FlyLaunchSource::Environment,
            launched: false,
        }
    }

    /// Queues the same validated launch path for an in-app development control.
    pub(crate) fn from_dev_ui(map_id: impl Into<String>) -> Self {
        Self {
            map_id: map_id.into(),
            source: FlyLaunchSource::TitleButton,
            launched: false,
        }
    }

    fn from_value(value: Option<String>) -> Result<Option<Self>, String> {
        let Some(value) = value else {
            return Ok(None);
        };
        let map_id = value.trim();
        if map_id.is_empty() {
            return Err(format!("{FLY_MAP_ENV} must name a Sandbox map ID"));
        }
        Ok(Some(Self::environment(map_id.to_owned())))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlyLaunchSource {
    Environment,
    TitleButton,
}

/// Exists from the Loading transition until gameplay teardown.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
struct FlySession {
    map_id: String,
}

/// The disposable actor translated by the noclip controller.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
struct FlyPawn;

#[derive(Resource, Debug)]
struct FlyConfigurationError(String);

#[derive(Component)]
struct FlyTitleSurface;

#[derive(Component)]
struct FlyTitleControl;

#[derive(Component)]
struct FlyTitleLabel;

#[derive(Component)]
struct FlyTitleStatus;

/// Installs fly mode only in a development build.
pub(super) fn plugin(app: &mut App) {
    app.load_settings::<Settings>("config/exploration.ron", &["exploration.ron"])
        .init_resource::<CapturedInput>();
    let request = match fly_request_from_environment() {
        Ok(request) => request,
        Err(error) => {
            app.insert_resource(FlyConfigurationError(error))
                .add_systems(Startup, reject_invalid_configuration);
            return;
        }
    };
    if let Some(request) = request {
        app.insert_resource(request);
    }

    #[cfg(feature = "visual-walk")]
    app.configure_sets(
        PreUpdate,
        ExplorationInput.after(crate::walk::WalkSystems::InjectInput),
    );

    app.add_systems(
        PreUpdate,
        capture_input
            .in_set(ExplorationInput)
            .after(InputSystems)
            .before(InputFocusSystems::Dispatch)
            .run_if(in_state(Screen::Gameplay))
            .run_if(resource_exists::<FlySession>),
    )
    .add_systems(OnEnter(Screen::Title), spawn_title_button)
    .add_systems(
        Update,
        (
            sync_title_button,
            request_title_button_launch,
            launch_requested_map,
        )
            .chain()
            .run_if(in_state(Screen::Title)),
    )
    .add_systems(
        Update,
        return_fly_session_to_title
            .run_if(in_state(Screen::Gameplay))
            .run_if(resource_exists::<FlySession>),
    )
    .add_systems(
        OnEnter(Screen::Gameplay),
        activate_fly_session.after(GameplaySetup::Finalize),
    )
    .add_systems(
        PostUpdate,
        (collision::refresh, move_fly_pawn, update_hint)
            .chain()
            .after(CameraSystems::FollowCharacter)
            .before(CameraSystems::FollowPresentation)
            .before(TransformSystems::Propagate)
            .run_if(in_state(Screen::Gameplay))
            .run_if(resource_exists::<Explorer>),
    )
    .add_systems(
        PostUpdate,
        hide_close_pawn
            .after(PresentationSystems::ResolveCameraOcclusion)
            .before(PresentationSystems::ApplyVisibility)
            .run_if(resource_exists::<Explorer>),
    )
    .add_systems(OnExit(Screen::Gameplay), clear_fly_session);
}

fn fly_request_from_environment() -> Result<Option<FlyLaunchRequest>, String> {
    match env::var(FLY_MAP_ENV) {
        Ok(value) => FlyLaunchRequest::from_value(Some(value)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(value)) => Err(format!(
            "{FLY_MAP_ENV} is not valid Unicode: {}",
            display_os_string(value)
        )),
    }
}

fn display_os_string(value: OsString) -> String {
    value.to_string_lossy().into_owned()
}

fn reject_invalid_configuration(
    error: Res<FlyConfigurationError>,
    mut exit: MessageWriter<AppExit>,
) {
    error!("invalid fly-map configuration: {}", error.0);
    exit.write(AppExit::error());
}

fn spawn_title_button(mut commands: Commands, assets: Res<UiAssets>) {
    commands
        .spawn((
            Name::new("Fly Map Test Shortcut"),
            FlyTitleSurface,
            DespawnOnExit(Screen::Title),
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(24.0),
                bottom: Val::Px(24.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Stretch,
                ..default()
            },
            GlobalZIndex(2),
            Pickable::IGNORE,
        ))
        .with_children(|surface| {
            let mut button = surface.spawn((
                stacked_row_button("Explore Grand V3 Map", 260.0),
                FlyTitleControl,
                InteractionDisabled,
                UiVisibilityRequirement::Immediate,
            ));
            button.with_children(|button| {
                button.spawn((FlyTitleLabel, label(&assets, "Explore Grand V3 Map")));
                button.spawn((FlyTitleStatus, fine(&assets, "Loading map catalog…")));
            });
        });
}

fn sync_title_button(
    model: Option<Res<MainMenuModel>>,
    catalog: Option<Res<SandboxMapCatalog>>,
    library: Option<Res<ScenarioLibrary>>,
    pending: Option<Res<FlyLaunchRequest>>,
    mut surfaces: Query<&mut Visibility, With<FlyTitleSurface>>,
    controls: Query<(Entity, Has<InteractionDisabled>), With<FlyTitleControl>>,
    mut labels: Query<&mut Text, (With<FlyTitleLabel>, Without<FlyTitleStatus>)>,
    mut statuses: Query<&mut Text, (With<FlyTitleStatus>, Without<FlyTitleLabel>)>,
    mut commands: Commands,
) {
    let on_root = model
        .as_deref()
        .is_none_or(|model| model.route == MainMenuRoute::Root);
    for mut visibility in &mut surfaces {
        let wanted = if on_root {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *visibility != wanted {
            *visibility = wanted;
        }
    }

    let (enabled, label_text, status_text) = if let Some(request) = pending.as_deref() {
        (
            false,
            "Launching fly mode…".to_owned(),
            format!("Map ID: {}", request.map_id),
        )
    } else {
        match (catalog.as_deref(), library.as_deref()) {
            (Some(catalog), Some(library)) => {
                match resolve_map(GRAND_V3_MAP_ID, catalog, library) {
                    Ok((definition, _)) => (
                        true,
                        "Explore Grand V3 Map".to_owned(),
                        definition.display_name,
                    ),
                    Err(reason) => (false, "Grand V3 Fly Unavailable".to_owned(), reason),
                }
            }
            _ => (
                false,
                "Explore Grand V3 Map".to_owned(),
                "Loading map catalog…".to_owned(),
            ),
        }
    };

    for (entity, disabled) in &controls {
        if enabled && disabled {
            commands.entity(entity).remove::<InteractionDisabled>();
        } else if !enabled && !disabled {
            commands.entity(entity).insert(InteractionDisabled);
        }
    }
    for mut label in &mut labels {
        if label.0 != label_text {
            label.0.clone_from(&label_text);
        }
    }
    for mut status in &mut statuses {
        if status.0 != status_text {
            status.0.clone_from(&status_text);
        }
    }
}

fn request_title_button_launch(
    controls: Query<
        &Interaction,
        (
            Changed<Interaction>,
            With<FlyTitleControl>,
            Without<InteractionDisabled>,
        ),
    >,
    pending: Option<Res<FlyLaunchRequest>>,
    mut commands: Commands,
) {
    if pending.is_some() {
        return;
    }
    if controls
        .iter()
        .any(|interaction| *interaction == Interaction::Pressed)
    {
        commands.insert_resource(FlyLaunchRequest::from_dev_ui(GRAND_V3_MAP_ID));
    }
}

fn launch_requested_map(
    mut commands: Commands,
    request: Option<ResMut<FlyLaunchRequest>>,
    catalog: Option<Res<SandboxMapCatalog>>,
    library: Option<Res<ScenarioLibrary>>,
    setup_failure: Option<Res<GameplaySetupFailure>>,
    mut phase: ResMut<GameplayPhase>,
    mut next: ResMut<NextState<Screen>>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(mut request) = request else { return };
    if request.launched {
        if let Some(failure) = setup_failure {
            match request.source {
                FlyLaunchSource::Environment => {
                    error!(
                        "fly-map launch for {:?} failed: {}",
                        request.map_id, failure.reason
                    );
                    exit.write(AppExit::error());
                }
                FlyLaunchSource::TitleButton => {
                    warn!(
                        "Grand V3 fly launch for {:?} failed: {}",
                        request.map_id, failure.reason
                    );
                    *phase = GameplayPhase::Active;
                    commands.remove_resource::<FlySession>();
                    commands.remove_resource::<FlyLaunchRequest>();
                }
            }
        }
        return;
    }
    let (Some(catalog), Some(library)) = (catalog, library) else {
        return;
    };

    let launch = match resolve_launch(&request.map_id, &catalog, &library) {
        Ok(launch) => launch,
        Err(reason) => {
            match request.source {
                FlyLaunchSource::Environment => {
                    error!("invalid fly-map launch: {reason}");
                    exit.write(AppExit::error());
                }
                FlyLaunchSource::TitleButton => {
                    warn!("Grand V3 fly shortcut is unavailable: {reason}");
                    commands.remove_resource::<FlyLaunchRequest>();
                }
            }
            request.launched = true;
            return;
        }
    };

    info!(
        "launching fly-map session for {:?} ({})",
        request.map_id, launch.scenario.name
    );
    commands.insert_resource(FlySession {
        map_id: request.map_id.clone(),
    });
    commands.insert_resource(launch);
    commands.remove_resource::<GameplaySetupFailure>();
    *phase = GameplayPhase::Preparing;
    request.launched = true;
    next.set(Screen::Loading);
}

fn return_fly_session_to_title(
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<InputBindings>,
    mut next: ResMut<NextState<Screen>>,
) {
    if bindings.just_pressed(&keys, InputAction::ReturnTitle) {
        next.set(Screen::Title);
    }
}

fn resolve_launch(
    map_id: &str,
    catalog: &SandboxMapCatalog,
    library: &ScenarioLibrary,
) -> Result<ScenarioToLoad, String> {
    let (definition, scenario) = resolve_map(map_id, catalog, library)?;
    let encounter = fly_encounter(&definition);
    let resolved_seed = definition.fixed_seed.or(scenario.generation_seed);
    Ok(ScenarioToLoad {
        scenario,
        resolved_seed: resolved_seed.map(ResolvedMapSeed),
        encounter_override: Some(encounter),
    })
}

fn resolve_map(
    map_id: &str,
    catalog: &SandboxMapCatalog,
    library: &ScenarioLibrary,
) -> Result<(SandboxMapDefinition, Scenario), String> {
    let definition = catalog
        .get(map_id)
        .cloned()
        .ok_or_else(|| format!("{FLY_MAP_ENV} map ID {map_id:?} is not in sandbox_maps.ron"))?;
    let mut scenarios = library
        .scenarios
        .iter()
        .filter(|scenario| scenario.name == definition.scenario);
    let scenario = scenarios.next().cloned().ok_or_else(|| {
        format!(
            "Sandbox map {:?} names unavailable scenario {:?}",
            definition.id, definition.scenario
        )
    })?;
    if scenarios.next().is_some() {
        return Err(format!(
            "Sandbox map {:?} names duplicated scenario {:?}",
            definition.id, definition.scenario
        ));
    }
    Ok((definition, scenario))
}

fn fly_encounter(definition: &SandboxMapDefinition) -> Encounter {
    let placement = match &definition.player_region.center {
        SandboxRegionCenter::Fixed(coord) => EncounterPlacement::Fixed(*coord),
        SandboxRegionCenter::Anchor(anchor) => EncounterPlacement::Anchor(anchor.clone()),
    };
    Encounter {
        name: format!("Fly Map Test · {}", definition.display_name),
        rosters: vec![Roster {
            faction: EncounterFaction::Player,
            placement,
            units: vec![RosterEntry {
                archetype: "hedge-mage".to_owned(),
                placement: None,
                ai_profile: None,
                ai_group: None,
            }],
        }],
    }
}

fn activate_fly_session(
    mut commands: Commands,
    ui_assets: Res<UiAssets>,
    fog: Option<Res<crate::fog::FogPresentationMode>>,
    #[cfg(feature = "visual-walk")] automation: Option<Res<crate::walk::AutomatedWalk>>,
    #[cfg(feature = "visual-walk")] mut virtual_time: ResMut<Time<Virtual>>,
    session: Option<Res<FlySession>>,
    setup_failure: Option<Res<GameplaySetupFailure>>,
    players: Query<(Entity, &Transform), (With<Player>, Without<PanOrbitCamera>)>,
    mut cameras: Query<
        (&mut Transform, &mut PanOrbitCamera),
        (With<PanOrbitCamera>, Without<Player>),
    >,
    settings: Res<CameraSettings>,
    mut phase: ResMut<GameplayPhase>,
    mut mode: ResMut<CameraMode>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(session) = session else { return };
    // Sandbox's shared Finalize adapter activates ordinary sessions. Reassert
    // Preparing before every fly-specific validation so even a failed pawn or
    // camera setup cannot expose one frame of gameplay authority.
    *phase = GameplayPhase::Preparing;
    if let Some(failure) = setup_failure {
        error!(
            "fly-map session for {:?} could not start: {}",
            session.map_id, failure.reason
        );
        exit.write(AppExit::error());
        return;
    }
    let Ok((pawn_entity, pawn_transform)) = players.single() else {
        error!(
            "fly-map session for {:?} needs exactly one placed player pawn; found {}",
            session.map_id,
            players.iter().count()
        );
        exit.write(AppExit::error());
        return;
    };
    let Ok((mut camera_transform, mut camera)) = cameras.single_mut() else {
        error!(
            "fly-map session for {:?} needs exactly one gameplay camera",
            session.map_id
        );
        exit.write(AppExit::error());
        return;
    };

    let pawn_position = pawn_transform.translation;
    commands.insert_resource(CollisionWorld::default());
    let mut explorer = Explorer::new(pawn_position, settings.character_radius);
    explorer.saved_fog = fog.as_deref().copied();
    #[cfg(feature = "visual-walk")]
    if automation.is_some() {
        explorer.saved_time_speed = Some(virtual_time.relative_speed());
        virtual_time.set_relative_speed(1.0);
    }
    commands.insert_resource(explorer);
    commands.insert_resource(crate::fog::FogPresentationMode::NoTerrainShading);
    commands.spawn((
        ExplorerHint,
        DespawnOnExit(Screen::Gameplay),
        Pickable::IGNORE,
        Text::new("Fly · WASD move · F walk · Right-drag look · Scroll zoom"),
        TextFont {
            font: ui_assets.body.clone().into(),
            font_size: FontSize::Px(16.0),
            ..default()
        },
        TextColor(Color::WHITE),
        BackgroundColor(Color::srgba(0.03, 0.04, 0.06, 0.9)),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(16.0),
            bottom: Val::Px(16.0),
            padding: UiRect::all(Val::Px(10.0)),
            max_width: Val::Percent(90.0),
            ..default()
        },
        GlobalZIndex(100),
    ));
    commands.entity(pawn_entity).insert(FlyPawn);
    camera.focus = pawn_position + Vec3::Y * settings.character_focus_height;
    camera.radius = settings.character_radius;
    camera_transform.translation =
        camera.focus + camera_transform.rotation * Vec3::Z * camera.radius;
    *mode = CameraMode::Fly;
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct FlyInput {
    forward: bool,
    backward: bool,
    left: bool,
    right: bool,
}

fn fly_translation(rotation: Quat, input: FlyInput, seconds: f32) -> Vec3 {
    if !seconds.is_finite() || seconds <= 0.0 {
        return Vec3::ZERO;
    }
    let forward = rotation * Vec3::NEG_Z;
    let right = rotation * Vec3::X;
    let mut direction = Vec3::ZERO;
    if input.forward {
        direction += forward;
    }
    if input.backward {
        direction -= forward;
    }
    if input.left {
        direction -= right;
    }
    if input.right {
        direction += right;
    }
    direction.normalize_or_zero() * FLY_SPEED * seconds
}

#[derive(Resource, Debug)]
pub(crate) struct Explorer {
    body: Body,
    mode: Mode,
    accumulator: f32,
    jump_pending: bool,
    effective_radius: f32,
    clear_seconds: f32,
    notice: String,
    notice_seconds: f32,
    saved_fog: Option<crate::fog::FogPresentationMode>,
    #[cfg(feature = "visual-walk")]
    saved_time_speed: Option<f32>,
}

impl Explorer {
    #[cfg(feature = "visual-walk")]
    pub(crate) fn observation(&self) -> (&'static str, bool, Vec3) {
        (
            if self.mode == Mode::Walk {
                "walk"
            } else {
                "fly"
            },
            self.body.grounded,
            self.body.position,
        )
    }
    fn new(position: Vec3, radius: f32) -> Self {
        Self {
            body: Body::new(position),
            mode: Mode::Fly,
            accumulator: 0.0,
            jump_pending: false,
            effective_radius: radius,
            clear_seconds: 0.0,
            notice: String::new(),
            notice_seconds: 0.0,
            saved_fog: None,
            #[cfg(feature = "visual-walk")]
            saved_time_speed: None,
        }
    }
    fn notify(&mut self, message: impl Into<String>) {
        self.notice = message.into();
        self.notice_seconds = 4.0;
    }
}

#[derive(Component)]
struct ExplorerHint;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ExplorationInput;

#[derive(Resource, Default, Debug)]
struct CapturedInput {
    flight: FlyInput,
    run: bool,
    jump: bool,
    toggle: bool,
    enabled: bool,
    simulate: bool,
}

fn movement_pressed(
    keys: &ButtonInput<KeyCode>,
    bindings: &InputBindings,
    action: InputAction,
) -> bool {
    let chord = bindings.chord(action);
    let mut modifiers = KeyModifiers::from_input(keys);
    let shift_matches = !chord.modifiers.shift || modifiers.shift;
    modifiers.shift = chord.modifiers.shift;
    keys.pressed(chord.key) && shift_matches && modifiers == chord.modifiers
}

fn capture_input(
    mut keys: ResMut<ButtonInput<KeyCode>>,
    bindings: Res<InputBindings>,
    focus: Option<Res<InputFocus>>,
    inspector: Option<Res<hex_dev::DevUiInputCapture>>,
    #[cfg(feature = "visual-walk")] automation: Option<Res<crate::walk::AutomatedWalk>>,
    windows: Query<(Entity, &Window), With<PrimaryWindow>>,
    pause: Option<Res<State<Pause>>>,
    mut captured: ResMut<CapturedInput>,
) {
    *captured = CapturedInput::default();
    let window = windows.single().ok();
    #[cfg(feature = "visual-walk")]
    let automated = automation.is_some();
    #[cfg(not(feature = "visual-walk"))]
    let automated = false;
    let focused_control = focus
        .as_deref()
        .and_then(InputFocus::get)
        .is_some_and(|entity| window.is_none_or(|(window, _)| entity != window));
    if (!automated && window.is_some_and(|(_, window)| !window.focused))
        || pause.as_deref().is_some_and(|pause| pause.get().0)
    {
        return;
    }
    captured.simulate = true;
    if focused_control
        || inspector
            .as_deref()
            .is_some_and(hex_dev::DevUiInputCapture::wants_any_keyboard_input)
    {
        return;
    }
    captured.enabled = true;
    captured.flight = FlyInput {
        forward: movement_pressed(&keys, &bindings, InputAction::CameraForward),
        backward: movement_pressed(&keys, &bindings, InputAction::CameraBackward),
        left: movement_pressed(&keys, &bindings, InputAction::CameraLeft),
        right: movement_pressed(&keys, &bindings, InputAction::CameraRight),
    };
    let modifiers = KeyModifiers::from_input(&keys);
    captured.run = modifiers.shift;
    if !modifiers.control && !modifiers.alt && !modifiers.super_key {
        captured.jump = keys.just_pressed(KeyCode::Space);
        captured.toggle = keys.just_pressed(KeyCode::KeyF);
    }
    // Consume only edges: held movement keys must survive until physical key-up.
    for key in [KeyCode::KeyF, KeyCode::Space] {
        let _ = keys.clear_just_pressed(key);
    }
    for action in [
        InputAction::CameraForward,
        InputAction::CameraBackward,
        InputAction::CameraLeft,
        InputAction::CameraRight,
    ] {
        let _ = keys.clear_just_pressed(bindings.chord(action).key);
    }
}

fn move_fly_pawn(
    input: Res<CapturedInput>,
    time: Res<Time>,
    settings: Res<Settings>,
    map: Res<hex_map::MapSettings>,
    camera_settings: Res<CameraSettings>,
    world: Res<CollisionWorld>,
    mut explorer: ResMut<Explorer>,
    mut pawns: Query<&mut Transform, (With<FlyPawn>, Without<PanOrbitCamera>)>,
    mut cameras: Query<
        (&mut Transform, &mut PanOrbitCamera),
        (With<PanOrbitCamera>, Without<FlyPawn>),
    >,
) {
    let (Ok(mut pawn), Ok((mut camera_transform, mut camera))) =
        (pawns.single_mut(), cameras.single_mut())
    else {
        return;
    };
    let elapsed = time.delta_secs().min(0.25);
    explorer.notice_seconds = (explorer.notice_seconds - elapsed).max(0.0);
    if input.toggle {
        if explorer.mode == Mode::Walk {
            explorer.mode = Mode::Fly;
            explorer.body.clear_motion();
        } else if world.initialized
            && world.clear(
                explorer.body.position,
                settings.body_levels * map.level_height,
                settings.body_radius,
            )
        {
            explorer.mode = Mode::Walk;
            explorer.body.clear_motion();
        } else {
            explorer.notify("Move clear of objects to walk.");
        }
        explorer.accumulator = 0.0;
        explorer.jump_pending = false;
        explorer.clear_seconds = 0.0;
    }
    if !input.simulate {
        explorer.accumulator = 0.0;
        explorer.jump_pending = false;
    } else if explorer.mode == Mode::Fly {
        explorer.body.position += fly_translation(camera_transform.rotation, input.flight, elapsed)
            * (settings.fly_speed / FLY_SPEED);
    } else if let Some(error) = &world.error {
        explorer.notify(format!(
            "Walk collision unavailable: {error}. Press F to fly."
        ));
    } else if world.initialized {
        explorer.accumulator += elapsed;
        explorer.jump_pending |= input.jump;
        // Camera right stays horizontal even at the vertical pitch poles.
        let right = camera_transform.rotation * Vec3::X;
        let horizontal_right = Vec3::new(right.x, 0.0, right.z).normalize_or_zero();
        let forward = Vec3::Y.cross(horizontal_right);
        let direction = forward
            * (f32::from(u8::from(input.flight.forward))
                - f32::from(u8::from(input.flight.backward)))
            + horizontal_right
                * (f32::from(u8::from(input.flight.right))
                    - f32::from(u8::from(input.flight.left)));
        while explorer.accumulator + f32::EPSILON >= controller::STEP {
            let intent = Intent {
                direction,
                run: input.run,
                jump: explorer.jump_pending,
            };
            explorer.jump_pending = false;
            let notice = explorer
                .body
                .tick(intent, &settings, map.level_height, &world);
            explorer.accumulator -= controller::STEP;
            if let Some(notice) = notice {
                explorer.notify(notice);
                if notice == "Safe ground changed; fly mode enabled." {
                    explorer.mode = Mode::Fly;
                    break;
                }
            }
        }
    }
    pawn.translation = explorer.body.position;
    let height = settings.body_levels * map.level_height;
    let probe = camera_settings
        .character_probe_radius
        .min(settings.body_radius * 0.8)
        .min(height * 0.2);
    let focus_height = if explorer.mode == Mode::Walk {
        let margin = collision::SKIN.min(height * 0.05);
        camera_settings
            .character_focus_height
            .clamp(probe + margin, height - probe - margin)
    } else {
        camera_settings.character_focus_height
    };
    camera.focus = pawn.translation + Vec3::Y * focus_height;
    let delta = camera_transform.rotation * Vec3::Z * camera.radius;
    if explorer.mode == Mode::Fly {
        explorer.effective_radius = camera.radius;
    } else {
        let safe = world
            .sweep(camera.focus - Vec3::Y * probe, delta, probe * 2.0, probe)
            .map_or(camera.radius, |hit| {
                (camera.radius * hit.fraction - camera_settings.character_collision_margin).max(0.0)
            });
        if safe < explorer.effective_radius {
            explorer.effective_radius = safe;
            explorer.clear_seconds = 0.0;
        } else {
            explorer.clear_seconds += elapsed;
            if explorer.clear_seconds >= camera_settings.character_collision_release_delay {
                explorer.effective_radius = (explorer.effective_radius
                    + camera_settings.character_restoration_speed * elapsed)
                    .min(safe);
            }
        }
        explorer.effective_radius = explorer.effective_radius.min(camera.radius);
    }
    camera_transform.translation =
        camera.focus + camera_transform.rotation * Vec3::Z * explorer.effective_radius;
}

fn update_hint(explorer: Res<Explorer>, mut hints: Query<&mut Text, With<ExplorerHint>>) {
    let controls = match explorer.mode {
        Mode::Walk => "Walk · WASD move · Shift run · Space jump · F fly",
        Mode::Fly => "Fly · WASD move · F walk",
    };
    let notice = if explorer.notice_seconds > 0.0 {
        format!("\n{}", explorer.notice)
    } else {
        String::new()
    };
    let text = format!("{controls}\nRight-drag look · Scroll zoom · Backspace menu{notice}");
    for mut hint in &mut hints {
        if hint.0 != text {
            hint.0.clone_from(&text);
        }
    }
}

fn hide_close_pawn(
    explorer: Res<Explorer>,
    settings: Res<CameraSettings>,
    mut pawns: Query<(Entity, Option<&mut PresentationOcclusion>), With<FlyPawn>>,
    mut commands: Commands,
) {
    let hidden = explorer.mode == Mode::Walk
        && explorer.effective_radius <= settings.character_self_hide_radius;
    for (entity, occlusion) in &mut pawns {
        if let Some(mut occlusion) = occlusion {
            let _changed = if hidden {
                occlusion.insert(PresentationOcclusionReason::CharacterCameraProximity)
            } else {
                occlusion.remove(PresentationOcclusionReason::CharacterCameraProximity)
            };
        } else if hidden {
            commands
                .entity(entity)
                .insert(PresentationOcclusion::from_reason(
                    PresentationOcclusionReason::CharacterCameraProximity,
                ));
        }
    }
}

fn clear_fly_session(
    mut commands: Commands,
    explorer: Option<Res<Explorer>>,
    session: Option<Res<FlySession>>,
    mut phase: ResMut<GameplayPhase>,
    mut mode: ResMut<CameraMode>,
    #[cfg(feature = "visual-walk")] mut virtual_time: ResMut<Time<Virtual>>,
) {
    if session.is_none() {
        return;
    }
    *phase = GameplayPhase::Active;
    *mode = CameraMode::Map;
    if let Some(explorer) = explorer {
        commands.insert_resource(explorer.saved_fog.unwrap_or_default());
        #[cfg(feature = "visual-walk")]
        if let Some(speed) = explorer.saved_time_speed {
            virtual_time.set_relative_speed(speed);
        }
    }
    commands.remove_resource::<FlySession>();
    commands.remove_resource::<FlyLaunchRequest>();
    commands.remove_resource::<Explorer>();
    commands.remove_resource::<CollisionWorld>();
    commands.insert_resource(CapturedInput::default());
}

#[cfg(test)]
mod tests {
    use std::f32::consts::FRAC_PI_4;
    use std::time::Duration;

    use bevy::state::app::StatesPlugin;

    use super::*;

    fn shipped_catalog() -> SandboxMapCatalog {
        ron::from_str(include_str!("../../../assets/config/sandbox_maps.ron"))
            .expect("the shipped Sandbox map catalog should parse")
    }

    fn shipped_scenarios() -> ScenarioLibrary {
        ron::from_str(include_str!("../../../assets/config/scenarios.ron"))
            .expect("the shipped scenario library should parse")
    }

    fn camera_settings() -> CameraSettings {
        ron::from_str(include_str!("../../../assets/config/camera.ron"))
            .expect("the shipped camera settings should parse")
    }

    fn ui_assets() -> UiAssets {
        UiAssets {
            display: Handle::default(),
            body: Handle::default(),
            logo: Handle::default(),
            hex_cell: Handle::default(),
        }
    }

    #[test]
    fn absent_configuration_is_inert_and_blank_configuration_is_rejected() {
        assert_eq!(FlyLaunchRequest::from_value(None), Ok(None));
        let error = FlyLaunchRequest::from_value(Some("   ".to_owned()))
            .expect_err("a blank map ID must be rejected");
        assert!(error.contains(FLY_MAP_ENV));
    }

    #[test]
    fn generated_map_launch_uses_one_player_at_the_catalog_center_and_fixed_seed() {
        let launch = resolve_launch("two-rings", &shipped_catalog(), &shipped_scenarios())
            .expect("Two Rings should resolve");
        let encounter = launch
            .encounter_override
            .expect("fly mode should own an encounter override");

        assert_eq!(launch.scenario.name, "Two Rings");
        assert_eq!(launch.resolved_seed, Some(ResolvedMapSeed(1_592_598_566)));
        assert_eq!(encounter.unit_count(EncounterFaction::Player), 1);
        assert_eq!(encounter.unit_count(EncounterFaction::Hostile), 0);
        let unit = encounter.entries().next().expect("one fly pawn");
        assert_eq!(unit.archetype, "hedge-mage");
        assert_eq!(
            unit.placement,
            &EncounterPlacement::Anchor("party_start".to_owned())
        );
    }

    #[test]
    fn authored_map_launch_uses_the_fixed_player_center_without_a_seed() {
        let catalog = shipped_catalog();
        let definition = catalog
            .get("flat-arena")
            .expect("Flat Arena should remain in the catalog");
        let launch = resolve_launch("flat-arena", &catalog, &shipped_scenarios())
            .expect("Flat Arena should resolve");
        let encounter = launch
            .encounter_override
            .expect("fly mode should own an encounter override");
        let expected = match definition.player_region.center {
            SandboxRegionCenter::Fixed(coord) => Some(EncounterPlacement::Fixed(coord)),
            SandboxRegionCenter::Anchor(_) => None,
        };

        assert_eq!(launch.resolved_seed, None);
        assert!(
            expected.is_some(),
            "Flat Arena should have an authored center"
        );
        assert_eq!(
            encounter.entries().next().map(|unit| unit.placement),
            expected.as_ref()
        );
    }

    #[test]
    fn scenario_seed_is_the_deterministic_fallback_when_catalog_seed_is_absent() {
        let mut catalog = shipped_catalog();
        let map = catalog
            .maps
            .iter_mut()
            .find(|map| map.id == "two-rings")
            .expect("Two Rings should remain in the catalog");
        map.fixed_seed = None;

        let launch = resolve_launch("two-rings", &catalog, &shipped_scenarios())
            .expect("the scenario seed should keep a generated map deterministic");

        assert_eq!(launch.resolved_seed, Some(ResolvedMapSeed(1_592_598_566)));
    }

    #[test]
    fn unknown_map_and_missing_scenario_have_actionable_errors() {
        let catalog = shipped_catalog();
        let scenarios = shipped_scenarios();
        let unknown = resolve_launch("not-a-map", &catalog, &scenarios)
            .expect_err("unknown map IDs must fail");
        assert!(unknown.contains("not-a-map"));
        assert!(unknown.contains("sandbox_maps.ron"));

        let mut broken = catalog.clone();
        let broken_id = broken
            .maps
            .first_mut()
            .map(|map| {
                map.scenario = "Missing Scenario".to_owned();
                map.id.clone()
            })
            .expect("the shipped catalog should have a first map");
        let missing = resolve_launch(&broken_id, &broken, &scenarios)
            .expect_err("missing scenarios must fail");
        assert!(missing.contains("Missing Scenario"));
        assert!(missing.contains("unavailable scenario"));
    }

    #[test]
    fn fly_translation_uses_full_pitch_normalizes_diagonals_and_scales_with_time() {
        let rotation = Quat::from_rotation_x(FRAC_PI_4);
        let forward = fly_translation(
            rotation,
            FlyInput {
                forward: true,
                ..default()
            },
            1.0,
        );
        assert!(forward.y > 0.0, "pitched forward flight must gain altitude");
        assert!((forward.length() - FLY_SPEED).abs() < 1e-5);

        let diagonal = fly_translation(
            rotation,
            FlyInput {
                forward: true,
                right: true,
                ..default()
            },
            1.0,
        );
        assert!((diagonal.length() - FLY_SPEED).abs() < 1e-5);

        let yaw = Quat::from_rotation_y(FRAC_PI_4);
        let right = fly_translation(
            yaw,
            FlyInput {
                right: true,
                ..default()
            },
            1.0,
        );
        assert!((right - yaw * Vec3::X * FLY_SPEED).length() < 1e-5);
        let left = fly_translation(
            yaw,
            FlyInput {
                left: true,
                ..default()
            },
            1.0,
        );
        assert!((left + right).length() < 1e-5);

        let full = fly_translation(
            rotation,
            FlyInput {
                forward: true,
                ..default()
            },
            1.0,
        );
        let halves = fly_translation(
            rotation,
            FlyInput {
                forward: true,
                ..default()
            },
            0.5,
        ) * 2.0;
        assert!(full.distance(halves) < 1e-5);
    }

    #[test]
    fn activation_marks_only_player_and_preserves_camera_orientation() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(FlySession {
                map_id: "fixture".to_owned(),
            })
            .insert_resource(GameplayPhase::Active)
            .insert_resource(CameraMode::Map)
            .insert_resource(camera_settings())
            .insert_resource(ui_assets())
            .add_systems(Update, activate_fly_session);
        let pawn_position = Vec3::new(2.0, 4.0, -3.0);
        let pawn = app
            .world_mut()
            .spawn((Player, Transform::from_translation(pawn_position)))
            .id();
        let rotation = Quat::from_euler(EulerRot::YXZ, 0.7, -0.25, 0.0);
        let camera = app
            .world_mut()
            .spawn((
                Transform::from_rotation(rotation),
                PanOrbitCamera::default(),
            ))
            .id();

        app.update();

        assert!(app.world().get::<FlyPawn>(pawn).is_some());
        assert_eq!(*app.world().resource::<CameraMode>(), CameraMode::Fly);
        assert_eq!(
            *app.world().resource::<GameplayPhase>(),
            GameplayPhase::Preparing
        );
        let settings = app.world().resource::<CameraSettings>();
        let camera_transform = app
            .world()
            .get::<Transform>(camera)
            .expect("the camera should retain its transform");
        let pan_orbit = app
            .world()
            .get::<PanOrbitCamera>(camera)
            .expect("the camera should retain its orbit component");
        assert!(camera_transform.rotation.abs_diff_eq(rotation, 1e-6));
        assert!((pan_orbit.radius - settings.character_radius).abs() < 1e-5);
        assert!(
            pan_orbit
                .focus
                .distance(pawn_position + Vec3::Y * settings.character_focus_height)
                < 1e-5
        );
    }

    fn exploration_settings() -> Settings {
        ron::from_str(include_str!("../../../assets/config/exploration.ron"))
            .expect("exploration settings")
    }

    fn movement_app(position: Vec3) -> (App, Entity, Entity) {
        let mut app = App::new();
        let map: hex_map::MapSettings = ron::from_str(include_str!(
            "../../../assets/config/worlds/procedural-grand-v3-baseline.ron"
        ))
        .expect("Grand V3 settings");
        app.insert_resource(exploration_settings())
            .insert_resource(map)
            .insert_resource(camera_settings())
            .insert_resource(CollisionWorld::default())
            .insert_resource(Explorer::new(position, 7.0))
            .insert_resource(CapturedInput {
                enabled: true,
                simulate: true,
                ..default()
            })
            .add_systems(Update, move_fly_pawn);
        let mut time = Time::<()>::default();
        time.advance_by(Duration::from_millis(100));
        app.insert_resource(time);
        let pawn = app
            .world_mut()
            .spawn((FlyPawn, Transform::from_translation(position)))
            .id();
        let rotation = Quat::from_rotation_x(FRAC_PI_4);
        let focus = position + Vec3::Y * camera_settings().character_focus_height;
        let camera = app
            .world_mut()
            .spawn((
                Transform::from_rotation(rotation)
                    .with_translation(focus + rotation * Vec3::Z * 7.0),
                PanOrbitCamera { focus, radius: 7.0 },
            ))
            .id();
        (app, pawn, camera)
    }

    #[test]
    fn movement_translates_pawn_camera_eye_and_focus_together() {
        let start = Vec3::new(1.0, 2.0, 3.0);
        let (mut app, pawn, camera) = movement_app(start);
        app.world_mut()
            .resource_mut::<CapturedInput>()
            .flight
            .forward = true;
        app.world_mut().resource_mut::<CollisionWorld>().replace(
            Entity::from_bits(100),
            vec![collision::Span {
                coord: hex_core::HexCoord::default(),
                bottom: -100.0,
                top: 100.0,
                material: collision::Material::Solid,
            }],
        );
        let rotation = app
            .world()
            .get::<Transform>(camera)
            .expect("camera")
            .rotation;
        let before_eye = app
            .world()
            .get::<Transform>(camera)
            .expect("camera")
            .translation;
        let before_focus = app
            .world()
            .get::<PanOrbitCamera>(camera)
            .expect("camera")
            .focus;
        let expected = fly_translation(
            rotation,
            FlyInput {
                forward: true,
                ..default()
            },
            0.1,
        );
        app.update();
        assert!(
            app.world()
                .get::<Transform>(pawn)
                .expect("pawn")
                .translation
                .distance(start + expected)
                < 1e-5
        );
        assert!(
            app.world()
                .get::<Transform>(camera)
                .expect("camera")
                .translation
                .distance(before_eye + expected)
                < 1e-5
        );
        assert!(
            app.world()
                .get::<PanOrbitCamera>(camera)
                .expect("camera")
                .focus
                .distance(before_focus + expected)
                < 1e-5
        );
    }

    #[test]
    fn toggle_in_air_starts_falling_and_toggle_during_fall_stops_gravity() {
        let (mut app, _, camera) = movement_app(Vec3::Y * 5.0);
        app.world_mut().resource_mut::<CollisionWorld>().initialized = true;
        app.world_mut().resource_mut::<CollisionWorld>().floor = -20.0;
        app.world_mut().resource_mut::<CapturedInput>().toggle = true;
        let before = *app.world().get::<Transform>(camera).expect("camera");
        app.update();
        let state = app.world().resource::<Explorer>();
        assert_eq!(state.mode, Mode::Walk);
        assert!(state.body.position.y < 5.0 && state.body.vertical_velocity < 0.0);
        let fall_position = state.body.position;
        app.update();
        let state = app.world().resource::<Explorer>();
        assert_eq!(state.mode, Mode::Fly);
        assert!(state.body.position.abs_diff_eq(fall_position, 0.00001));
        assert!(state.body.vertical_velocity.abs() < f32::EPSILON);
        assert!(app
            .world()
            .get::<Transform>(camera)
            .expect("camera")
            .rotation
            .abs_diff_eq(before.rotation, 1e-6));
        assert!(
            (app.world()
                .get::<PanOrbitCamera>(camera)
                .expect("camera")
                .radius
                - 7.0)
                .abs()
                < 1e-6
        );
    }

    #[test]
    fn cannot_enable_walking_inside_solid_geometry() {
        let (mut app, _, _) = movement_app(Vec3::Y);
        let mut geometry = app.world_mut().resource_mut::<CollisionWorld>();
        geometry.initialized = true;
        geometry.replace(
            Entity::from_bits(99),
            vec![collision::Span {
                coord: hex_core::HexCoord::default(),
                bottom: 0.0,
                top: 3.0,
                material: collision::Material::Solid,
            }],
        );
        app.world_mut().resource_mut::<CapturedInput>().toggle = true;
        app.update();
        let state = app.world().resource::<Explorer>();
        assert_eq!(state.mode, Mode::Fly);
        assert_eq!(state.notice, "Move clear of objects to walk.");
    }

    #[test]
    fn camera_retracts_and_restores_toward_partial_clearance_without_changing_look_or_zoom() {
        let (mut app, _, camera) = movement_app(Vec3::ZERO);
        let wall = Entity::from_bits(99);
        let direction = hex_core::HexCoord::from_axial(0, 1)
            .to_world(0.0)
            .normalize();
        let rotation = Quat::from_rotation_arc(Vec3::Z, direction);
        app.world_mut()
            .get_mut::<Transform>(camera)
            .expect("camera")
            .rotation = rotation;
        app.world_mut().resource_mut::<Explorer>().mode = Mode::Walk;
        {
            let mut world = app.world_mut().resource_mut::<CollisionWorld>();
            world.initialized = true;
            world.floor = -20.0;
            world.replace(
                Entity::from_bits(100),
                vec![collision::Span {
                    coord: hex_core::HexCoord::default(),
                    bottom: -1.0,
                    top: 0.0,
                    material: collision::Material::Solid,
                }],
            );
            world.replace(
                wall,
                vec![collision::Span {
                    coord: hex_core::HexCoord::from_axial(0, 1),
                    bottom: 0.0,
                    top: 5.0,
                    material: collision::Material::Solid,
                }],
            );
        }
        app.update();
        let close = app.world().resource::<Explorer>().effective_radius;
        assert!(close < 1.0, "near wall must retract: {close}");
        app.world_mut().resource_mut::<CollisionWorld>().replace(
            wall,
            vec![collision::Span {
                coord: hex_core::HexCoord::from_axial(0, 3),
                bottom: 0.0,
                top: 5.0,
                material: collision::Material::Solid,
            }],
        );
        for _ in 0..60 {
            app.update();
        }
        let farther = app.world().resource::<Explorer>().effective_radius;
        assert!(
            farther > close + 2.0 && farther < 7.0,
            "partial clearance: {farther}"
        );
        let eye = app.world().get::<Transform>(camera).expect("camera");
        assert!(eye.rotation.abs_diff_eq(rotation, 1e-6));
        assert!(app.world().resource::<CollisionWorld>().clear(
            eye.translation - Vec3::Y * 0.1,
            0.2,
            0.1
        ));
        assert!(
            (app.world()
                .get::<PanOrbitCamera>(camera)
                .expect("camera")
                .radius
                - 7.0)
                .abs()
                < 1e-6
        );
        app.world_mut()
            .resource_mut::<CollisionWorld>()
            .remove(wall);
        for _ in 0..60 {
            app.update();
        }
        assert!((app.world().resource::<Explorer>().effective_radius - 7.0).abs() < 1e-6);
    }

    #[test]
    fn exploration_claims_f_space_and_shift_w_but_yields_to_focus_and_blur() {
        let mut app = App::new();
        app.init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<InputBindings>()
            .init_resource::<CapturedInput>()
            .add_systems(Update, capture_input);
        let window = app
            .world_mut()
            .spawn((
                PrimaryWindow,
                Window {
                    focused: true,
                    ..default()
                },
            ))
            .id();
        app.insert_resource(InputFocus::from_entity(window));
        for key in [
            KeyCode::ShiftLeft,
            KeyCode::KeyW,
            KeyCode::KeyF,
            KeyCode::Space,
        ] {
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(key);
        }
        app.update();
        let input = app.world().resource::<CapturedInput>();
        assert!(input.enabled && input.run && input.flight.forward && input.toggle && input.jump);
        let keys = app.world().resource::<ButtonInput<KeyCode>>();
        assert!(!keys.just_pressed(KeyCode::KeyF) && !keys.just_pressed(KeyCode::Space));
        assert!(keys.pressed(KeyCode::KeyW));
        app.update();
        assert!(
            !app.world().resource::<CapturedInput>().jump,
            "held Space must not autojump"
        );
        let button = app.world_mut().spawn_empty().id();
        app.insert_resource(InputFocus::from_entity(button));
        app.update();
        assert!(!app.world().resource::<CapturedInput>().enabled);
        app.insert_resource(InputFocus::from_entity(window));
        app.world_mut()
            .get_mut::<Window>(window)
            .expect("window")
            .focused = false;
        app.update();
        assert!(!app.world().resource::<CapturedInput>().enabled);
    }

    #[test]
    fn configured_return_action_cleans_session_and_allows_a_fresh_request() {
        let mut app = App::new();
        let mut explorer = Explorer::new(Vec3::ZERO, 7.0);
        explorer.saved_fog = Some(crate::fog::FogPresentationMode::Dimmed);
        #[cfg(feature = "visual-walk")]
        {
            explorer.saved_time_speed = Some(12.0);
        }
        app.add_plugins((MinimalPlugins, StatesPlugin))
            .insert_state(Screen::Gameplay)
            .insert_resource(ButtonInput::<KeyCode>::default())
            .init_resource::<InputBindings>()
            .insert_resource(GameplayPhase::Preparing)
            .insert_resource(CameraMode::Fly)
            .insert_resource(FlySession {
                map_id: "fixture".to_owned(),
            })
            .insert_resource(FlyLaunchRequest::from_dev_ui("fixture"))
            .insert_resource(explorer)
            .insert_resource(CollisionWorld::default())
            .insert_resource(crate::fog::FogPresentationMode::NoTerrainShading)
            .insert_resource(CapturedInput {
                jump: true,
                toggle: true,
                ..default()
            })
            .add_systems(
                Update,
                return_fly_session_to_title
                    .run_if(in_state(Screen::Gameplay))
                    .run_if(resource_exists::<FlySession>),
            )
            .add_systems(OnExit(Screen::Gameplay), clear_fly_session);
        let return_key = app
            .world()
            .resource::<InputBindings>()
            .chord(InputAction::ReturnTitle)
            .key;
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(return_key);

        app.update();
        app.update();

        assert_eq!(
            *app.world().resource::<State<Screen>>().get(),
            Screen::Title
        );
        assert!(app.world().get_resource::<FlySession>().is_none());
        assert!(app.world().get_resource::<FlyLaunchRequest>().is_none());
        assert!(app.world().get_resource::<Explorer>().is_none());
        assert!(app.world().get_resource::<CollisionWorld>().is_none());
        assert_eq!(
            *app.world().resource::<crate::fog::FogPresentationMode>(),
            crate::fog::FogPresentationMode::Dimmed
        );
        assert!(!app.world().resource::<CapturedInput>().jump);
        assert!(!app.world().resource::<CapturedInput>().toggle);
        #[cfg(feature = "visual-walk")]
        assert!((app.world().resource::<Time<Virtual>>().relative_speed() - 12.0).abs() < 1e-6);
        assert_eq!(
            *app.world().resource::<GameplayPhase>(),
            GameplayPhase::Active
        );
        assert_eq!(*app.world().resource::<CameraMode>(), CameraMode::Map);

        app.world_mut()
            .insert_resource(FlyLaunchRequest::from_dev_ui(GRAND_V3_MAP_ID));
        let fresh = app.world().resource::<FlyLaunchRequest>();
        assert_eq!(fresh.map_id, GRAND_V3_MAP_ID);
        assert!(!fresh.launched);
    }

    #[test]
    fn grand_v3_title_button_queues_the_shared_launch_path_when_available() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(ui_assets())
            .insert_resource(MainMenuModel::default())
            .insert_resource(shipped_catalog())
            .insert_resource(shipped_scenarios())
            .add_systems(Startup, spawn_title_button)
            .add_systems(
                Update,
                (sync_title_button, request_title_button_launch).chain(),
            );

        app.update();

        let control = {
            let world = app.world_mut();
            let mut controls = world.query_filtered::<Entity, With<FlyTitleControl>>();
            controls
                .single(world)
                .expect("the title should contain exactly one Grand V3 fly control")
        };
        let entity = app.world().entity(control);
        assert_eq!(
            entity.get::<Name>().map(Name::as_str),
            Some("Explore Grand V3 Map")
        );
        assert!(entity.get::<InteractionDisabled>().is_none());
        assert_eq!(
            entity.get::<UiVisibilityRequirement>(),
            Some(&UiVisibilityRequirement::Immediate)
        );

        app.world_mut()
            .entity_mut(control)
            .insert(Interaction::Pressed);
        app.update();

        let request = app
            .world()
            .get_resource::<FlyLaunchRequest>()
            .expect("pressing the shortcut should queue a fly request");
        assert_eq!(request.map_id, GRAND_V3_MAP_ID);
        assert_eq!(request.source, FlyLaunchSource::TitleButton);
        assert!(!request.launched);
    }

    #[test]
    fn grand_v3_title_button_is_safely_disabled_until_content_arrives() {
        let mut catalog = shipped_catalog();
        catalog.maps.retain(|map| map.id != GRAND_V3_MAP_ID);

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(ui_assets())
            .insert_resource(MainMenuModel::default())
            .insert_resource(catalog)
            .insert_resource(shipped_scenarios())
            .add_systems(Startup, spawn_title_button)
            .add_systems(Update, sync_title_button);

        app.update();

        let world = app.world_mut();
        let mut controls = world.query_filtered::<
            (Has<InteractionDisabled>, &UiVisibilityRequirement),
            With<FlyTitleControl>,
        >();
        let (disabled, visibility) = controls
            .single(world)
            .expect("the unavailable shortcut should remain visible and singular");
        assert!(disabled);
        assert_eq!(*visibility, UiVisibilityRequirement::Immediate);
    }
    #[test]
    fn grounded_motion_matches_across_frame_rates_and_ui_focus_still_falls() {
        let mut results = Vec::new();
        for hz in [30_u32, 60, 144] {
            let (mut app, _, _) = movement_app(Vec3::ZERO);
            {
                let mut world = app.world_mut().resource_mut::<CollisionWorld>();
                world.initialized = true;
                world.floor = -20.0;
                world.replace(
                    Entity::from_bits(100),
                    hex_core::HexCoord::default()
                        .within_radius(10)
                        .into_iter()
                        .map(|coord| collision::Span {
                            coord,
                            bottom: -1.0,
                            top: 0.0,
                            material: collision::Material::Solid,
                        })
                        .collect(),
                );
            }
            app.world_mut().resource_mut::<Explorer>().mode = Mode::Walk;
            app.world_mut().resource_mut::<CapturedInput>().flight.right = true;
            for _ in 0..hz {
                app.world_mut()
                    .resource_mut::<Time>()
                    .advance_by(Duration::from_secs_f64(1.0 / f64::from(hz)));
                app.update();
            }
            results.push(app.world().resource::<Explorer>().body.position);
        }
        for position in results {
            assert!((position.x - 3.0).abs() < 0.03, "{position:?}");
        }
        let (mut app, _, _) = movement_app(Vec3::Y * 5.0);
        app.world_mut().resource_mut::<Explorer>().mode = Mode::Walk;
        app.world_mut().resource_mut::<CollisionWorld>().initialized = true;
        app.world_mut().resource_mut::<CollisionWorld>().floor = -20.0;
        app.world_mut().resource_mut::<CapturedInput>().enabled = false;
        app.update();
        assert!(app.world().resource::<Explorer>().body.position.y < 5.0);
        let before = app.world().resource::<Explorer>().body.position;
        app.world_mut().resource_mut::<CapturedInput>().simulate = false;
        app.update();
        assert!(app
            .world()
            .resource::<Explorer>()
            .body
            .position
            .abs_diff_eq(before, 1e-6));
    }
}
