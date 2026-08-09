//! Application adapter for Direct Connect, lobby control, and session presentation.
//!
//! Socket construction lives behind explicit host/join actions. World-owned code supplies
//! [`PreparedDirectSandboxSession`] and [`DirectWorldReady`]; this module never substitutes
//! `GenerationReport::map_fingerprint` for the complete public-world contract.

use std::collections::BTreeSet;

use bevy::prelude::*;
use bevy_replicon::prelude::{ClientState, ProtocolHash};
use hex_assets::AcceptedContentRevision;
use hex_core::{CommandRequestId, PlayerSeat, Screen, SimulationRole, UnitId};
use hex_gameplay_model::{
    MainMenuModel, MainMenuRoute, MultiplayerBackResult, MultiplayerEndReason, MultiplayerModel,
    MultiplayerRole,
};
use hex_multiplayer::{
    AdmissionAccepted, AdmissionCredential, AdmissionRefusal, AdmissionRefusalReason,
    AtomicFileReconnectCredentialStore, BuildIdentityV1, CertificateFingerprint, ClientHello,
    ClientLobbyAction, ClientLobbyRequest, ClientMapReady, ContentFingerprint,
    CredentialStorageOperation, CredentialStorageStatus, DirectConnectionCode, DirectEndpoint,
    HostSessionAction, HostSessionControlRequest, LobbyPhase, LobbySnapshot, PreparedDirectHost,
    PreparedDirectJoin, PublicWorldFingerprint, ReconnectCredentialStorage, SeatConnectionState,
    SessionAdmissionAuthority, SessionCloseReason, SessionClosed, SessionControlOutcome,
    SessionControlRefusal, SessionControlResult, SessionManifestV1, StoredReconnectCredential,
};
use hex_ui::{
    MultiplayerAssignmentView, MultiplayerIntent, MultiplayerSeatConnectionView,
    MultiplayerSeatView, MultiplayerTextField, MultiplayerView, SensitiveText, UiIntent, UiSystems,
};

use crate::storage::StoragePaths;

/// World-owned, fully validated handoff created after shipped Sandbox deployment.
///
/// L3 must compute `manifest.map.expected_public_fingerprint` from the complete current
/// public-world contract before constructing this resource. L4 deliberately has no fallback.
#[derive(Resource, Debug, Clone)]
pub(crate) struct PreparedDirectSandboxSession {
    pub(crate) manifest: SessionManifestV1,
    pub(crate) summary: String,
}

impl PreparedDirectSandboxSession {
    pub(crate) fn new(manifest: SessionManifestV1, summary: impl Into<String>) -> Option<Self> {
        manifest.validate().ok()?;
        Some(Self {
            manifest,
            summary: summary.into(),
        })
    }
}

/// Complete local-world verification supplied by L3 after generation/import.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DirectWorldReady {
    pub(crate) fingerprint: PublicWorldFingerprint,
}

/// Public endpoint retained while the existing Sandbox/deployment flow is active.
#[derive(Resource, Debug, Clone)]
pub(crate) struct PendingDirectHostSetup {
    pub(crate) endpoint: DirectEndpoint,
}

#[derive(Resource, Debug, Clone)]
struct MultiplayerDraft {
    advertised_host: String,
    advertised_port: String,
    join_code: SensitiveText,
}

impl Default for MultiplayerDraft {
    fn default() -> Self {
        Self {
            advertised_host: "127.0.0.1".to_owned(),
            advertised_port: hex_multiplayer::DEFAULT_DIRECT_PORT.to_string(),
            join_code: SensitiveText::default(),
        }
    }
}

#[derive(Resource, Debug, Default)]
struct SessionUiNotice(Option<String>);

#[derive(Resource, Debug, Default)]
struct SessionProjection {
    lobby: Option<LobbySnapshot>,
    manifest: Option<SessionManifestV1>,
}

#[derive(Resource, Debug, Default)]
struct StoredCredentialState {
    value: Option<StoredReconnectCredential>,
}

#[derive(Resource, Debug)]
struct SessionUiRequestIds(u64);

impl Default for SessionUiRequestIds {
    fn default() -> Self {
        let identity = hex_multiplayer::SessionPeerId::generate().to_bytes();
        let mut epoch = [0_u8; 8];
        epoch.copy_from_slice(&identity[..8]);
        Self(u64::from_be_bytes(epoch) & (u64::MAX >> 1))
    }
}

impl SessionUiRequestIds {
    fn allocate(&mut self) -> Option<CommandRequestId> {
        self.0 = self.0.checked_add(1)?;
        Some(CommandRequestId(self.0))
    }
}

#[derive(Resource)]
enum DirectStartQueue {
    Host {
        endpoint: DirectEndpoint,
        prepared: PreparedDirectSandboxSession,
    },
    Join {
        code: DirectConnectionCode,
        credential: AdmissionCredential,
        reconnecting: bool,
    },
}

#[derive(Debug, Clone)]
struct HostedCodeSource {
    endpoint: DirectEndpoint,
    certificate_fingerprint: CertificateFingerprint,
}

#[derive(Resource, Debug)]
struct ActiveDirectSession {
    entity: Entity,
    role: MultiplayerRole,
    hosted_code: Option<HostedCodeSource>,
}

#[derive(Resource, Debug)]
struct PendingClientHello {
    credential: AdmissionCredential,
    sent: bool,
}

#[derive(Resource, Debug, Default)]
struct MapReadyReportState {
    sent: bool,
}

#[derive(Resource, Debug, Default)]
struct HostOutcomeState {
    sent: bool,
}

#[derive(Resource, Debug)]
struct HostShutdownCountdown(u8);

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<StoragePaths>();
    let reconnect_path = app
        .world()
        .resource::<StoragePaths>()
        .preferences
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("temporary-session")
        .join("reconnect.hexrc1");
    app.insert_resource(ReconnectCredentialStorage::new(
        AtomicFileReconnectCredentialStore::new(reconnect_path),
    ));

    app.init_resource::<MultiplayerModel>()
        .init_resource::<MultiplayerDraft>()
        .init_resource::<SessionUiNotice>()
        .init_resource::<SessionProjection>()
        .init_resource::<StoredCredentialState>()
        .init_resource::<SessionUiRequestIds>()
        .init_resource::<MapReadyReportState>()
        .init_resource::<HostOutcomeState>()
        .add_systems(Startup, load_stored_credential)
        .add_systems(
            OnEnter(Screen::Multiplayer),
            queue_prepared_host_after_sandbox,
        )
        .add_systems(
            Update,
            (
                handle_intents.after(UiSystems::EmitIntents),
                start_queued_direct_session,
            )
                .chain()
                .run_if(in_state(Screen::Multiplayer)),
        )
        .add_systems(
            Update,
            (
                send_client_hello,
                capture_session_messages,
                sync_host_session,
                finish_host_shutdown,
                report_client_map_ready,
                publish_host_outcome,
                publish_view.before(UiSystems::Render),
            )
                .chain(),
        )
        .add_systems(
            Update,
            handle_intents
                .after(UiSystems::EmitIntents)
                .run_if(in_state(Screen::Gameplay)),
        )
        .add_systems(OnEnter(Screen::Gameplay), reset_gameplay_session_flags);
}

fn load_stored_credential(
    storage: Res<ReconnectCredentialStorage>,
    mut state: ResMut<StoredCredentialState>,
    mut notice: ResMut<SessionUiNotice>,
) {
    match storage.store().load() {
        Ok(value) => state.value = value,
        Err(_) => {
            state.value = None;
            notice.0 = Some(
                "The temporary reconnect credential could not be read; ordinary Direct Join remains available."
                    .to_owned(),
            );
        }
    }
}

fn queue_prepared_host_after_sandbox(
    mut commands: Commands,
    pending: Option<Res<PendingDirectHostSetup>>,
    prepared: Option<Res<PreparedDirectSandboxSession>>,
    active: Option<Res<ActiveDirectSession>>,
    mut model: ResMut<MultiplayerModel>,
    mut notice: ResMut<SessionUiNotice>,
) {
    let Some(pending) = pending else {
        return;
    };
    model.show_host_direct();
    if active.is_some() {
        return;
    }
    let Some(prepared) = prepared else {
        notice.0 = Some(
            "The complete public-world snapshot contract is not available yet; hosting was not started."
                .to_owned(),
        );
        return;
    };
    let Some(prepared) =
        PreparedDirectSandboxSession::new(prepared.manifest.clone(), prepared.summary.clone())
    else {
        notice.0 = Some(
            "The world adapter supplied an invalid frozen session manifest; hosting was not started."
                .to_owned(),
        );
        return;
    };
    model.connecting(MultiplayerRole::Host);
    commands.insert_resource(DirectStartQueue::Host {
        endpoint: pending.endpoint.clone(),
        prepared,
    });
}

#[expect(
    clippy::too_many_arguments,
    reason = "the session screen maps one typed intent family onto its explicit local/wire effect boundaries"
)]
fn handle_intents(
    mut intents: MessageReader<UiIntent>,
    mut model: ResMut<MultiplayerModel>,
    mut draft: ResMut<MultiplayerDraft>,
    stored: Res<StoredCredentialState>,
    projection: Res<SessionProjection>,
    authority: Option<Res<SessionAdmissionAuthority>>,
    active: Option<Res<ActiveDirectSession>>,
    mut ids: ResMut<SessionUiRequestIds>,
    mut notice: ResMut<SessionUiNotice>,
    mut host_controls: MessageWriter<HostSessionControlRequest>,
    mut client_controls: MessageWriter<ClientLobbyRequest>,
    mut commands: Commands,
    mut next_screen: ResMut<NextState<Screen>>,
    mut main_menu: ResMut<MainMenuModel>,
) {
    for ui_intent in intents.read() {
        let translated = match ui_intent {
            UiIntent::Outcome(hex_ui::OutcomeIntent::Activate(action)) if model.role.is_some() => {
                match action {
                    hex_ui::OutcomeAction::RetryExact => Some(MultiplayerIntent::RetryExact),
                    hex_ui::OutcomeAction::ReturnToLobby => Some(MultiplayerIntent::ReturnToLobby),
                    hex_ui::OutcomeAction::CloseSession => Some(MultiplayerIntent::CloseSession),
                    hex_ui::OutcomeAction::LeaveSession => Some(MultiplayerIntent::LeaveSession),
                    _ => None,
                }
            }
            _ => None,
        };
        let intent = match ui_intent {
            UiIntent::Multiplayer(intent) => intent,
            _ => {
                let Some(intent) = translated.as_ref() else {
                    continue;
                };
                intent
            }
        };
        match intent {
            MultiplayerIntent::OpenHostDirect => {
                notice.0 = None;
                model.show_host_direct();
            }
            MultiplayerIntent::OpenJoinDirect => {
                notice.0 = None;
                model.show_join_direct();
            }
            MultiplayerIntent::SetText(field, value) => match field {
                MultiplayerTextField::AdvertisedHost => {
                    draft.advertised_host = value.expose().to_owned();
                }
                MultiplayerTextField::AdvertisedPort => {
                    draft.advertised_port = value.expose().to_owned();
                }
                MultiplayerTextField::JoinCode => draft.join_code = value.clone(),
            },
            MultiplayerIntent::ConfigureSandbox => {
                let endpoint = match direct_endpoint(&draft) {
                    Ok(endpoint) => endpoint,
                    Err(reason) => {
                        notice.0 = Some(reason);
                        continue;
                    }
                };
                commands.insert_resource(PendingDirectHostSetup { endpoint });
                commands.remove_resource::<PreparedDirectSandboxSession>();
                notice.0 = None;
                next_screen.set(Screen::Sandbox);
            }
            MultiplayerIntent::JoinDirect | MultiplayerIntent::ReconnectDirect => {
                if active.is_some() {
                    notice.0 = Some("A direct session is already active.".to_owned());
                    continue;
                }
                let code = match DirectConnectionCode::parse(draft.join_code.expose()) {
                    Ok(code) => code,
                    Err(error) => {
                        notice.0 = Some(format!("Direct connection code refused: {error}."));
                        continue;
                    }
                };
                let reconnecting = matches!(intent, MultiplayerIntent::ReconnectDirect);
                let credential = if reconnecting {
                    let Some(stored) = stored.value else {
                        notice.0 = Some(
                            "No private reconnect credential is available in temporary storage."
                                .to_owned(),
                        );
                        continue;
                    };
                    AdmissionCredential::Reconnect(stored.reconnect_credential)
                } else {
                    AdmissionCredential::Invite(code.invite_token)
                };
                if reconnecting {
                    model.connecting(MultiplayerRole::Client);
                    model.show_reconnecting();
                } else {
                    model.connecting(MultiplayerRole::Client);
                }
                notice.0 = None;
                commands.insert_resource(DirectStartQueue::Join {
                    code,
                    credential,
                    reconnecting,
                });
            }
            MultiplayerIntent::AssignUnit { unit, destination } => {
                write_host_control(
                    model.role,
                    &mut ids,
                    HostSessionAction::AssignUnit {
                        unit: *unit,
                        destination: *destination,
                    },
                    &mut host_controls,
                    &mut notice,
                );
            }
            MultiplayerIntent::Kick(seat) => write_host_control(
                model.role,
                &mut ids,
                HostSessionAction::Kick { seat: *seat },
                &mut host_controls,
                &mut notice,
            ),
            MultiplayerIntent::SetReady(ready) => {
                if model.role != Some(MultiplayerRole::Client) {
                    notice.0 =
                        Some("Only an admitted guest can change guest readiness.".to_owned());
                    continue;
                }
                write_client_control(
                    &mut ids,
                    ClientLobbyAction::SetReady(*ready),
                    &mut client_controls,
                    &mut notice,
                );
            }
            MultiplayerIntent::Launch => {
                let fingerprint = authority
                    .as_deref()
                    .map(SessionAdmissionAuthority::manifest)
                    .or(projection.manifest.as_ref())
                    .map(|manifest| manifest.map.expected_public_fingerprint);
                let Some(fingerprint) = fingerprint else {
                    notice.0 = Some(
                        "Launch refused: the complete frozen session manifest is unavailable."
                            .to_owned(),
                    );
                    continue;
                };
                write_host_control(
                    model.role,
                    &mut ids,
                    HostSessionAction::BeginLoading {
                        public_world_fingerprint: fingerprint,
                    },
                    &mut host_controls,
                    &mut notice,
                );
            }
            MultiplayerIntent::RetryExact => {
                let fingerprint = authority
                    .as_deref()
                    .map(SessionAdmissionAuthority::manifest)
                    .or(projection.manifest.as_ref())
                    .map(|manifest| manifest.map.expected_public_fingerprint);
                let Some(fingerprint) = fingerprint else {
                    notice.0 =
                        Some("Retry refused: the frozen manifest is unavailable.".to_owned());
                    continue;
                };
                write_host_control(
                    model.role,
                    &mut ids,
                    HostSessionAction::RetryExact {
                        public_world_fingerprint: fingerprint,
                    },
                    &mut host_controls,
                    &mut notice,
                );
            }
            MultiplayerIntent::ReturnToLobby => write_host_control(
                model.role,
                &mut ids,
                HostSessionAction::ReturnToLobby,
                &mut host_controls,
                &mut notice,
            ),
            MultiplayerIntent::CloseSession => write_host_control(
                model.role,
                &mut ids,
                HostSessionAction::CloseSession,
                &mut host_controls,
                &mut notice,
            ),
            MultiplayerIntent::LeaveSession => {
                leave_session(
                    &mut model,
                    active.as_deref(),
                    &mut ids,
                    &mut client_controls,
                    &mut commands,
                    &mut notice,
                );
            }
            MultiplayerIntent::ResumeLocal => {
                if model.role == Some(MultiplayerRole::Client) && model.local_menu_open {
                    let _changed = model.toggle_client_menu();
                }
            }
            MultiplayerIntent::Back => match model.back() {
                MultiplayerBackResult::Home => {
                    commands.remove_resource::<PendingDirectHostSetup>();
                    commands.remove_resource::<PreparedDirectSandboxSession>();
                    notice.0 = None;
                }
                MultiplayerBackResult::LeaveSession => leave_session(
                    &mut model,
                    active.as_deref(),
                    &mut ids,
                    &mut client_controls,
                    &mut commands,
                    &mut notice,
                ),
                MultiplayerBackResult::MainMenu => {
                    main_menu.show(MainMenuRoute::Root);
                    next_screen.set(Screen::Title);
                }
            },
        }
    }
}

fn direct_endpoint(draft: &MultiplayerDraft) -> Result<DirectEndpoint, String> {
    let port = draft
        .advertised_port
        .parse::<u16>()
        .map_err(|_| "UDP port must be a number from 1 through 65535.".to_owned())?;
    DirectEndpoint::new(draft.advertised_host.trim(), port)
        .map_err(|error| format!("Advertised endpoint refused: {error}."))
}

fn write_host_control(
    role: Option<MultiplayerRole>,
    ids: &mut SessionUiRequestIds,
    action: HostSessionAction,
    controls: &mut MessageWriter<HostSessionControlRequest>,
    notice: &mut SessionUiNotice,
) {
    if role != Some(MultiplayerRole::Host) {
        notice.0 = Some("Only the listen host may perform that session action.".to_owned());
        return;
    }
    let Some(request_id) = ids.allocate() else {
        notice.0 = Some("Session control request identity exhausted.".to_owned());
        return;
    };
    controls.write(HostSessionControlRequest { request_id, action });
}

fn write_client_control(
    ids: &mut SessionUiRequestIds,
    action: ClientLobbyAction,
    controls: &mut MessageWriter<ClientLobbyRequest>,
    notice: &mut SessionUiNotice,
) {
    let Some(request_id) = ids.allocate() else {
        notice.0 = Some("Session control request identity exhausted.".to_owned());
        return;
    };
    controls.write(ClientLobbyRequest { request_id, action });
}

fn leave_session(
    model: &mut MultiplayerModel,
    active: Option<&ActiveDirectSession>,
    ids: &mut SessionUiRequestIds,
    client_controls: &mut MessageWriter<ClientLobbyRequest>,
    commands: &mut Commands,
    notice: &mut SessionUiNotice,
) {
    match (model.role, active) {
        (Some(MultiplayerRole::Client), Some(active))
            if model.local_seat.is_some() && active.role == MultiplayerRole::Client =>
        {
            write_client_control(ids, ClientLobbyAction::Leave, client_controls, notice);
        }
        (Some(MultiplayerRole::Client), Some(active)) => {
            commands.entity(active.entity).despawn();
            commands.remove_resource::<ActiveDirectSession>();
            commands.remove_resource::<PendingClientHello>();
            commands.insert_resource(SimulationRole::Authority);
            model.enter_home();
            notice.0 = None;
        }
        (Some(MultiplayerRole::Host), _) => {
            notice.0 =
                Some("The host must use Close Session to end the session for everyone.".to_owned());
        }
        _ => {
            model.enter_home();
            notice.0 = None;
        }
    }
}

fn start_queued_direct_session(world: &mut World) {
    let Some(request) = world.remove_resource::<DirectStartQueue>() else {
        return;
    };
    let result = match request {
        DirectStartQueue::Host { endpoint, prepared } => {
            start_direct_host(world, endpoint, prepared)
        }
        DirectStartQueue::Join {
            code,
            credential,
            reconnecting,
        } => start_direct_join(world, &code, credential, reconnecting),
    };
    if let Err(reason) = result {
        if let Some(mut model) = world.get_resource_mut::<MultiplayerModel>() {
            model.end(MultiplayerEndReason::ProtocolViolation);
        }
        if let Some(mut notice) = world.get_resource_mut::<SessionUiNotice>() {
            notice.0 = Some(reason);
        }
        world.insert_resource(SimulationRole::Authority);
    }
}

fn start_direct_host(
    world: &mut World,
    endpoint: DirectEndpoint,
    prepared: PreparedDirectSandboxSession,
) -> Result<(), String> {
    let protocol_hash = *world.resource::<ProtocolHash>();
    let authority = SessionAdmissionAuthority::new(protocol_hash, prepared.manifest.clone())
        .map_err(|error| format!("Host session refused its frozen manifest: {error}."))?;
    let direct = PreparedDirectHost::new(endpoint.clone(), authority.invite_token())
        .map_err(|error| format!("Could not prepare the encrypted direct host: {error}."))?;
    let hosted_code = HostedCodeSource {
        endpoint,
        certificate_fingerprint: direct.connection_code().certificate_fingerprint,
    };
    let server = direct.open(world);
    world.insert_resource(authority);
    world.insert_resource(ActiveDirectSession {
        entity: server,
        role: MultiplayerRole::Host,
        hosted_code: Some(hosted_code),
    });
    world.insert_resource(SessionProjection {
        lobby: None,
        manifest: Some(prepared.manifest.clone()),
    });
    world.insert_resource(prepared);
    world.insert_resource(SimulationRole::Authority);
    world.remove_resource::<PendingDirectHostSetup>();
    world.remove_resource::<DirectWorldReady>();
    world.resource_mut::<MapReadyReportState>().sent = false;
    let admitted = world
        .resource_mut::<MultiplayerModel>()
        .admitted(MultiplayerRole::Host, PlayerSeat::HOST);
    if !admitted {
        return Err("Host session produced an invalid human seat.".to_owned());
    }
    Ok(())
}

fn start_direct_join(
    world: &mut World,
    code: &DirectConnectionCode,
    credential: AdmissionCredential,
    reconnecting: bool,
) -> Result<(), String> {
    if world.get_resource::<AcceptedContentRevision>().is_none() {
        return Err("Shipped content is still loading; Direct Join was not started.".to_owned());
    }
    let direct = PreparedDirectJoin::new(code)
        .map_err(|error| format!("Could not prepare the pinned direct connection: {error}."))?;
    let entity = direct.connect(world);
    world.insert_resource(ActiveDirectSession {
        entity,
        role: MultiplayerRole::Client,
        hosted_code: None,
    });
    world.insert_resource(PendingClientHello {
        credential,
        sent: false,
    });
    world.insert_resource(SimulationRole::Replica);
    world.remove_resource::<DirectWorldReady>();
    world.resource_mut::<MapReadyReportState>().sent = false;
    if reconnecting {
        world.resource_mut::<MultiplayerModel>().show_reconnecting();
    }
    Ok(())
}

fn send_client_hello(
    state: Option<Res<State<ClientState>>>,
    protocol_hash: Res<ProtocolHash>,
    accepted: Option<Res<AcceptedContentRevision>>,
    mut pending: Option<ResMut<PendingClientHello>>,
    mut hellos: MessageWriter<ClientHello>,
    mut notice: ResMut<SessionUiNotice>,
) {
    let Some(pending) = pending.as_mut() else {
        return;
    };
    if pending.sent
        || state
            .as_deref()
            .is_none_or(|state| *state.get() != ClientState::Connected)
    {
        return;
    }
    let Some(accepted) = accepted else {
        notice.0 = Some("Shipped content became unavailable before admission.".to_owned());
        return;
    };
    let Ok(build) = local_build_identity() else {
        notice.0 = Some("The local build identity exceeds protocol bounds.".to_owned());
        return;
    };
    hellos.write(ClientHello {
        protocol_hash: *protocol_hash,
        build,
        content_fingerprint: ContentFingerprint(accepted.fingerprint()),
        credential: pending.credential,
    });
    pending.sent = true;
}

fn local_build_identity() -> Result<BuildIdentityV1, hex_multiplayer::BoundError> {
    BuildIdentityV1::new(
        env!("CARGO_PKG_VERSION"),
        option_env!("HEX_GAME_BUILD_ID").unwrap_or(env!("CARGO_PKG_VERSION")),
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "independent protocol message readers converge into one renderer-free session projection"
)]
fn capture_session_messages(
    mut accepted: MessageReader<AdmissionAccepted>,
    mut refused: MessageReader<AdmissionRefusal>,
    mut lobbies: MessageReader<LobbySnapshot>,
    mut manifests: MessageReader<SessionManifestV1>,
    mut closed: MessageReader<SessionClosed>,
    mut control_results: MessageReader<SessionControlResult>,
    mut storage_status: MessageReader<CredentialStorageStatus>,
    storage: Res<ReconnectCredentialStorage>,
    active: Option<Res<ActiveDirectSession>>,
    ready: Option<Res<DirectWorldReady>>,
    mut stored: ResMut<StoredCredentialState>,
    mut projection: ResMut<SessionProjection>,
    mut model: ResMut<MultiplayerModel>,
    mut notice: ResMut<SessionUiNotice>,
    mut commands: Commands,
    mut next_screen: ResMut<NextState<Screen>>,
) {
    for message in accepted.read() {
        if !model.admitted(MultiplayerRole::Client, message.seat) {
            notice.0 = Some("The host admitted an invalid human seat.".to_owned());
        }
    }
    if let Some(manifest) = manifests.read().last() {
        projection.manifest = Some(manifest.clone());
    }
    if let Some(lobby) = lobbies.read().last() {
        projection.lobby = Some(lobby.clone());
        project_lobby_phase(lobby.phase, ready.as_deref(), &mut model, &mut next_screen);
    }
    for refusal in refused.read() {
        notice.0 = Some(admission_refusal_copy(refusal.reason).to_owned());
        end_active_session(
            admission_end_reason(refusal.reason),
            active.as_deref(),
            &mut model,
            &mut commands,
        );
        next_screen.set(Screen::Multiplayer);
    }
    for result in control_results.read() {
        if let SessionControlOutcome::Refused(reason) = result.outcome {
            notice.0 = Some(control_refusal_copy(reason).to_owned());
        }
    }
    for status in storage_status.read() {
        match status {
            CredentialStorageStatus::Stored => match storage.store().load() {
                Ok(value) => stored.value = value,
                Err(_) => {
                    notice.0 =
                        Some("Reconnect credential storage could not be verified.".to_owned())
                }
            },
            CredentialStorageStatus::Deleted => stored.value = None,
            CredentialStorageStatus::Failed(operation) => {
                notice.0 = Some(match operation {
                    CredentialStorageOperation::Store => {
                        "The reconnect credential could not be stored; restart recovery is unavailable."
                    }
                    CredentialStorageOperation::Delete => {
                        "Temporary reconnect state could not be deleted."
                    }
                }
                .to_owned());
            }
        }
    }
    for message in closed.read() {
        notice.0 = Some(session_close_copy(message.reason).to_owned());
        end_active_session(
            session_end_reason(message.reason),
            active.as_deref(),
            &mut model,
            &mut commands,
        );
        next_screen.set(Screen::Multiplayer);
    }
}

fn sync_host_session(
    authority: Option<Res<SessionAdmissionAuthority>>,
    active: Option<Res<ActiveDirectSession>>,
    ready: Option<Res<DirectWorldReady>>,
    mut projection: ResMut<SessionProjection>,
    mut model: ResMut<MultiplayerModel>,
    mut commands: Commands,
    mut next_screen: ResMut<NextState<Screen>>,
    mut notice: ResMut<SessionUiNotice>,
) {
    let (Some(authority), Some(active)) = (authority, active) else {
        return;
    };
    if active.role != MultiplayerRole::Host {
        return;
    }
    let snapshot = authority.lobby().snapshot_owned();
    projection.lobby = Some(snapshot.clone());
    projection.manifest = Some(authority.manifest().clone());
    if snapshot.phase == LobbyPhase::Closed {
        if model.ended_reason != Some(MultiplayerEndReason::HostClosed) {
            notice.0 = Some("The host closed the session.".to_owned());
            model.end(MultiplayerEndReason::HostClosed);
            // L1 keeps each child connection alive for two updates so the typed close
            // can flush. Keep the server endpoint alive for one additional update.
            commands.insert_resource(HostShutdownCountdown(3));
            next_screen.set(Screen::Multiplayer);
        }
        return;
    }
    project_lobby_phase(
        snapshot.phase,
        ready.as_deref(),
        &mut model,
        &mut next_screen,
    );
}

fn finish_host_shutdown(
    mut countdown: Option<ResMut<HostShutdownCountdown>>,
    active: Option<Res<ActiveDirectSession>>,
    mut model: ResMut<MultiplayerModel>,
    mut commands: Commands,
) {
    let Some(countdown) = countdown.as_mut() else {
        return;
    };
    if countdown.0 > 0 {
        countdown.0 -= 1;
        return;
    }
    end_active_session(
        MultiplayerEndReason::HostClosed,
        active.as_deref(),
        &mut model,
        &mut commands,
    );
    commands.remove_resource::<HostShutdownCountdown>();
}

fn project_lobby_phase(
    phase: LobbyPhase,
    ready: Option<&DirectWorldReady>,
    model: &mut MultiplayerModel,
    next_screen: &mut NextState<Screen>,
) {
    match phase {
        LobbyPhase::Open => {
            model.show_lobby();
            next_screen.set(Screen::Multiplayer);
        }
        LobbyPhase::Loading => {
            model.show_loading();
            next_screen.set(Screen::Multiplayer);
        }
        LobbyPhase::Active if ready.is_some() => next_screen.set(Screen::Gameplay),
        LobbyPhase::Active => model.show_loading(),
        LobbyPhase::Outcome => {}
        LobbyPhase::Closed => model.end(MultiplayerEndReason::SessionEnded),
    }
}

fn report_client_map_ready(
    model: Res<MultiplayerModel>,
    projection: Res<SessionProjection>,
    ready: Option<Res<DirectWorldReady>>,
    mut report_state: ResMut<MapReadyReportState>,
    mut reports: MessageWriter<ClientMapReady>,
    mut notice: ResMut<SessionUiNotice>,
) {
    if model.role != Some(MultiplayerRole::Client)
        || projection.lobby.as_ref().map(|lobby| lobby.phase) != Some(LobbyPhase::Loading)
    {
        report_state.sent = false;
        return;
    }
    if report_state.sent {
        return;
    }
    let (Some(ready), Some(manifest)) = (ready, projection.manifest.as_ref()) else {
        return;
    };
    if ready.fingerprint != manifest.map.expected_public_fingerprint {
        notice.0 = Some("The generated world does not match the host manifest.".to_owned());
        return;
    }
    reports.write(ClientMapReady {
        public_world_fingerprint: ready.fingerprint,
    });
    report_state.sent = true;
}

fn publish_host_outcome(
    model: Res<MultiplayerModel>,
    resolution: Res<hex_combat::EncounterResolution>,
    authority: Option<Res<SessionAdmissionAuthority>>,
    mut state: ResMut<HostOutcomeState>,
    mut ids: ResMut<SessionUiRequestIds>,
    mut controls: MessageWriter<HostSessionControlRequest>,
) {
    if model.role != Some(MultiplayerRole::Host)
        || authority
            .as_deref()
            .is_none_or(|authority| authority.lobby().snapshot().phase != LobbyPhase::Active)
    {
        state.sent = false;
        return;
    }
    if resolution.outcome().is_none() || state.sent {
        return;
    }
    let Some(request_id) = ids.allocate() else {
        return;
    };
    controls.write(HostSessionControlRequest {
        request_id,
        action: HostSessionAction::EnterOutcome,
    });
    state.sent = true;
}

fn reset_gameplay_session_flags(
    mut outcome: ResMut<HostOutcomeState>,
    mut reports: ResMut<MapReadyReportState>,
) {
    outcome.sent = false;
    reports.sent = false;
}

#[expect(
    clippy::too_many_arguments,
    reason = "the immutable view deliberately gathers only disclosure-safe session projections"
)]
fn publish_view(
    model: Res<MultiplayerModel>,
    draft: Res<MultiplayerDraft>,
    notice: Res<SessionUiNotice>,
    stored: Res<StoredCredentialState>,
    projection: Res<SessionProjection>,
    authority: Option<Res<SessionAdmissionAuthority>>,
    active: Option<Res<ActiveDirectSession>>,
    prepared: Option<Res<PreparedDirectSandboxSession>>,
    mut view: ResMut<MultiplayerView>,
) {
    let lobby = authority
        .as_deref()
        .map(|authority| authority.lobby().snapshot_owned())
        .or_else(|| projection.lobby.clone());
    let manifest = authority
        .as_deref()
        .map(|authority| authority.manifest().clone())
        .or_else(|| projection.manifest.clone());
    let seats = lobby.as_ref().map_or_else(default_seats, |lobby| {
        lobby
            .seats
            .iter()
            .map(|seat| MultiplayerSeatView {
                seat: seat.seat,
                connection: seat_connection_view(seat.connection),
                player_label: seat.connection.is_claimed().then(|| {
                    if seat.seat == PlayerSeat::HOST {
                        "Host".to_owned()
                    } else {
                        format!("Player {}", seat.seat.0 + 1)
                    }
                }),
                assignments: seat
                    .assigned_units
                    .as_slice()
                    .iter()
                    .map(|unit| MultiplayerAssignmentView {
                        unit: *unit,
                        label: roster_label(manifest.as_ref(), *unit),
                    })
                    .collect(),
                ready: seat.ready,
                local: model.local_seat == Some(seat.seat),
            })
            .collect()
    });
    let (can_launch, launch_blocker) = launch_gate(lobby.as_ref(), manifest.as_ref());
    let share_code = active
        .as_deref()
        .filter(|active| active.role == MultiplayerRole::Host)
        .and_then(|active| active.hosted_code.as_ref())
        .zip(authority.as_deref())
        .map(|(source, authority)| {
            SensitiveText::new(
                DirectConnectionCode {
                    endpoint: source.endpoint.clone(),
                    certificate_fingerprint: source.certificate_fingerprint,
                    invite_token: authority.invite_token(),
                }
                .encode()
                .expose_for_sharing(),
            )
        });
    let launch_summary = lobby
        .as_ref()
        .and_then(|lobby| lobby.launch_summary.as_ref())
        .map(|summary| {
            format!(
                "{} · {} claimed seat{} · world {:016x}",
                summary.scenario_identity.as_str(),
                summary.claimed_seats,
                if summary.claimed_seats == 1 { "" } else { "s" },
                summary.public_world_fingerprint.0
            )
        })
        .or_else(|| prepared.as_deref().map(|prepared| prepared.summary.clone()));
    let next = MultiplayerView {
        route: model.route,
        role: model.role,
        local_seat: model.local_seat,
        advertised_host: draft.advertised_host.clone(),
        advertised_port: draft.advertised_port.clone(),
        share_code,
        join_code: draft.join_code.clone(),
        reconnect_available: stored.value.is_some(),
        seats,
        launch_summary,
        notice: notice.0.clone(),
        can_launch,
        launch_blocker,
        local_menu_open: model.local_menu_open,
    };
    if *view != next {
        *view = next;
    }
}

fn default_seats() -> Vec<MultiplayerSeatView> {
    (0_u8..PlayerSeat::HUMAN_COUNT as u8)
        .filter_map(PlayerSeat::human)
        .map(MultiplayerSeatView::vacant)
        .collect()
}

fn seat_connection_view(connection: SeatConnectionState) -> MultiplayerSeatConnectionView {
    match connection {
        SeatConnectionState::Vacant => MultiplayerSeatConnectionView::Vacant,
        SeatConnectionState::Connected => MultiplayerSeatConnectionView::Connected,
        SeatConnectionState::Reserved { remaining_millis } => {
            MultiplayerSeatConnectionView::Reserved {
                seconds: remaining_millis.saturating_add(999) / 1_000,
            }
        }
        SeatConnectionState::TemporarilyDelegated => MultiplayerSeatConnectionView::Delegated,
        SeatConnectionState::ReclaimPending => MultiplayerSeatConnectionView::ReclaimPending,
    }
}

fn roster_label(manifest: Option<&SessionManifestV1>, unit: UnitId) -> String {
    manifest
        .and_then(|manifest| {
            manifest
                .shipped_roster
                .as_slice()
                .iter()
                .find(|entry| entry.unit == unit)
        })
        .map_or_else(
            || format!("Party member #{}", unit.0),
            |entry| entry.character_identity.as_str().to_owned(),
        )
}

fn launch_gate(
    lobby: Option<&LobbySnapshot>,
    manifest: Option<&SessionManifestV1>,
) -> (bool, Option<String>) {
    let (Some(lobby), Some(manifest)) = (lobby, manifest) else {
        return (
            false,
            Some("Configure and freeze a shipped Sandbox encounter first.".to_owned()),
        );
    };
    if lobby.phase != LobbyPhase::Open {
        return (
            false,
            Some("Admission is closed for this encounter.".to_owned()),
        );
    }
    let Some(host) = lobby.seats.first() else {
        return (false, Some("The host seat is unavailable.".to_owned()));
    };
    if host.assigned_units.is_empty() {
        return (false, Some("The host must own a party member.".to_owned()));
    }
    for seat in lobby
        .seats
        .iter()
        .filter(|seat| seat.connection.is_claimed())
    {
        if seat.assigned_units.is_empty() {
            return (
                false,
                Some(format!("Seat {} needs a party member.", seat.seat.0 + 1)),
            );
        }
        if seat.seat != PlayerSeat::HOST && seat.connection.is_connected() && !seat.ready {
            return (
                false,
                Some(format!("Seat {} is not ready.", seat.seat.0 + 1)),
            );
        }
    }
    let assigned = lobby
        .seats
        .iter()
        .flat_map(|seat| seat.assigned_units.as_slice().iter().copied())
        .collect::<BTreeSet<_>>();
    let roster = manifest
        .shipped_roster
        .as_slice()
        .iter()
        .map(|entry| entry.unit)
        .collect::<BTreeSet<_>>();
    if assigned != roster {
        return (
            false,
            Some("Every shipped party member must be assigned exactly once.".to_owned()),
        );
    }
    (true, None)
}

fn end_active_session(
    reason: MultiplayerEndReason,
    active: Option<&ActiveDirectSession>,
    model: &mut MultiplayerModel,
    commands: &mut Commands,
) {
    if let Some(active) = active {
        commands.entity(active.entity).despawn();
    }
    commands.remove_resource::<ActiveDirectSession>();
    commands.remove_resource::<PendingClientHello>();
    commands.remove_resource::<SessionAdmissionAuthority>();
    commands.remove_resource::<DirectWorldReady>();
    commands.insert_resource(SimulationRole::Authority);
    model.end(reason);
}

fn admission_end_reason(reason: AdmissionRefusalReason) -> MultiplayerEndReason {
    match reason {
        AdmissionRefusalReason::ProtocolMismatch
        | AdmissionRefusalReason::BuildMismatch
        | AdmissionRefusalReason::ContentMismatch
        | AdmissionRefusalReason::SessionMismatch => MultiplayerEndReason::Incompatible,
        AdmissionRefusalReason::LobbyClosed => MultiplayerEndReason::LobbyClosed,
        AdmissionRefusalReason::LobbyFull => MultiplayerEndReason::LobbyFull,
        AdmissionRefusalReason::InvalidInvite | AdmissionRefusalReason::InvalidReconnect => {
            MultiplayerEndReason::InvalidCredential
        }
        AdmissionRefusalReason::DuplicateActiveSeat | AdmissionRefusalReason::Malformed => {
            MultiplayerEndReason::ProtocolViolation
        }
    }
}

fn admission_refusal_copy(reason: AdmissionRefusalReason) -> &'static str {
    match reason {
        AdmissionRefusalReason::ProtocolMismatch => "Protocol mismatch: host and client use different multiplayer schemas.",
        AdmissionRefusalReason::BuildMismatch => "Build mismatch: host and client must use the exact same build.",
        AdmissionRefusalReason::ContentMismatch => "Content mismatch: only the exact shipped content revision is supported.",
        AdmissionRefusalReason::SessionMismatch => "Session mismatch: the frozen encounter identity differs.",
        AdmissionRefusalReason::LobbyClosed => "The host has launched; new admission is closed.",
        AdmissionRefusalReason::LobbyFull => "The lobby has no remaining human seat with a party assignment.",
        AdmissionRefusalReason::InvalidInvite => "The one-time invite is invalid or was already consumed. Ask the host for the current code.",
        AdmissionRefusalReason::InvalidReconnect => "The private reconnect credential is invalid or was already rotated.",
        AdmissionRefusalReason::DuplicateActiveSeat => "That reserved seat already has an active connection.",
        AdmissionRefusalReason::Malformed => "The host rejected malformed admission data.",
    }
}

fn control_refusal_copy(reason: SessionControlRefusal) -> &'static str {
    match reason {
        SessionControlRefusal::NotAuthorized => {
            "Session control refused: this connection is not authorized."
        }
        SessionControlRefusal::WrongPhase => "Session control refused in the current phase.",
        SessionControlRefusal::LobbyClosed => "The assignment lobby is closed.",
        SessionControlRefusal::InvalidSeat => "The requested seat is not a valid guest seat.",
        SessionControlRefusal::SeatUnavailable => "The requested seat is unavailable.",
        SessionControlRefusal::WouldEmptySeat => {
            "Assignment refused because every claimed seat must retain a party member."
        }
        SessionControlRefusal::LobbyFull => "The lobby has no remaining assignment capacity.",
        SessionControlRefusal::MapMismatch => {
            "The complete public-world fingerprint does not match the frozen manifest."
        }
        SessionControlRefusal::InvalidLobby => {
            "The lobby does not satisfy the frozen launch invariants."
        }
        SessionControlRefusal::RateLimited => {
            "Session controls are arriving too quickly; wait and try again."
        }
    }
}

fn session_end_reason(reason: SessionCloseReason) -> MultiplayerEndReason {
    match reason {
        SessionCloseReason::HostDisconnected => MultiplayerEndReason::HostDisconnected,
        SessionCloseReason::HostClosed => MultiplayerEndReason::HostClosed,
        SessionCloseReason::Kicked => MultiplayerEndReason::Kicked,
        SessionCloseReason::ProtocolViolation => MultiplayerEndReason::ProtocolViolation,
        SessionCloseReason::MapMismatch => MultiplayerEndReason::MapMismatch,
        SessionCloseReason::SessionEnded => MultiplayerEndReason::SessionEnded,
    }
}

fn session_close_copy(reason: SessionCloseReason) -> &'static str {
    match reason {
        SessionCloseReason::HostDisconnected => {
            "The host connection ended. Host migration is not supported."
        }
        SessionCloseReason::HostClosed => "The host closed the session.",
        SessionCloseReason::Kicked => "The host removed this player from the session.",
        SessionCloseReason::ProtocolViolation => {
            "The session ended after a protocol or security violation."
        }
        SessionCloseReason::MapMismatch => {
            "The generated public world did not match the host manifest."
        }
        SessionCloseReason::SessionEnded => "The multiplayer session ended.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::{Faction, SimSeeds, TilePos};
    use hex_multiplayer::{
        BoundedText, BoundedVec, MapManifestV1, ProtocolVersion, RosterEntryV1, RulesManifestV1,
        SessionPeerId, UnitDeploymentV1, MAX_IDENTITY_BYTES,
    };

    fn text(value: &str) -> BoundedText<MAX_IDENTITY_BYTES> {
        BoundedText::new(value).expect("test identity fits")
    }

    fn manifest() -> SessionManifestV1 {
        SessionManifestV1 {
            protocol: ProtocolVersion::default(),
            build: local_build_identity().expect("local build fits"),
            content_fingerprint: ContentFingerprint(7),
            scenario_identity: text("sandbox"),
            map: MapManifestV1 {
                catalog_identity: text("flat-arena"),
                seed: 8,
                generator_identity: text("authored"),
                generator_version: 1,
                expected_public_fingerprint: PublicWorldFingerprint(9),
            },
            rules: RulesManifestV1 {
                profile_identity: text("default"),
                fingerprint: 10,
            },
            shipped_roster: BoundedVec::new(
                (0_u64..6)
                    .map(|unit| RosterEntryV1 {
                        unit: UnitId(unit),
                        archetype_identity: text("hero"),
                        character_identity: text(&format!("Hero {}", unit + 1)),
                        faction: Faction::Player,
                    })
                    .collect(),
            )
            .expect("six roster members fit"),
            deployment: BoundedVec::new(
                (0_u64..6)
                    .map(|unit| UnitDeploymentV1 {
                        unit: UnitId(unit),
                        position: TilePos::ORIGIN,
                    })
                    .collect(),
            )
            .expect("six deployments fit"),
            simulation_seeds: SimSeeds {
                world: 11,
                ai_flavor: 12,
                cosmetic: 13,
            },
        }
    }

    #[test]
    fn endpoint_validation_is_explicit_and_rejects_zero_or_non_numeric_ports() {
        let mut draft = MultiplayerDraft::default();
        assert!(direct_endpoint(&draft).is_ok());
        draft.advertised_port = "0".to_owned();
        assert!(direct_endpoint(&draft).is_err());
        draft.advertised_port = "not-a-port".to_owned();
        assert!(direct_endpoint(&draft).is_err());
    }

    #[test]
    fn launch_gate_requires_assignments_and_guest_readiness() {
        let manifest = manifest();
        let mut lobby = LobbySnapshot::new(SessionPeerId::from_bytes([1; 16]), &manifest)
            .expect("manifest creates a lobby");
        assert_eq!(launch_gate(Some(&lobby), Some(&manifest)), (true, None));

        {
            let guest = &mut lobby.seats[1];
            guest.connection = SeatConnectionState::Connected;
            guest.player = Some(SessionPeerId::from_bytes([2; 16]));
            guest.assigned_units = BoundedVec::new(vec![UnitId(5)]).expect("one assignment fits");
        }
        lobby.seats[0].assigned_units =
            BoundedVec::new((0_u64..5).map(UnitId).collect()).expect("host assignments fit");
        assert!(launch_gate(Some(&lobby), Some(&manifest))
            .1
            .is_some_and(|reason| reason.contains("not ready")));
        lobby.seats[1].ready = true;
        assert_eq!(launch_gate(Some(&lobby), Some(&manifest)), (true, None));
    }

    #[test]
    fn host_only_actions_are_never_wire_client_lobby_actions() {
        let wire = [ClientLobbyAction::SetReady(true), ClientLobbyAction::Leave];
        assert_eq!(wire.len(), 2);
        let host = [
            HostSessionAction::AssignUnit {
                unit: UnitId(0),
                destination: PlayerSeat(1),
            },
            HostSessionAction::Kick {
                seat: PlayerSeat(1),
            },
            HostSessionAction::BeginLoading {
                public_world_fingerprint: PublicWorldFingerprint(9),
            },
            HostSessionAction::RetryExact {
                public_world_fingerprint: PublicWorldFingerprint(9),
            },
            HostSessionAction::ReturnToLobby,
            HostSessionAction::CloseSession,
        ];
        assert_eq!(host.len(), 6);
    }

    #[test]
    fn prepared_host_handoff_rejects_an_invalid_manifest() {
        let mut invalid = manifest();
        invalid.shipped_roster = BoundedVec::default();
        assert!(PreparedDirectSandboxSession::new(invalid, "invalid").is_none());
    }

    #[test]
    fn pending_host_without_world_handoff_opens_no_session() {
        let endpoint = DirectEndpoint::new("127.0.0.1", 7_777).expect("loopback endpoint is valid");
        let mut app = App::new();
        app.init_resource::<MultiplayerModel>()
            .init_resource::<SessionUiNotice>()
            .insert_resource(PendingDirectHostSetup { endpoint })
            .add_systems(Update, queue_prepared_host_after_sandbox);

        app.update();

        assert!(app.world().get_resource::<DirectStartQueue>().is_none());
        assert!(app.world().get_resource::<ActiveDirectSession>().is_none());
        assert_eq!(
            app.world().resource::<SessionUiNotice>().0.as_deref(),
            Some(
                "The complete public-world snapshot contract is not available yet; hosting was not started."
            )
        );
    }

    #[test]
    fn exact_world_handoff_only_queues_the_explicit_host_start() {
        let endpoint = DirectEndpoint::new("127.0.0.1", 7_777).expect("loopback endpoint is valid");
        let prepared = PreparedDirectSandboxSession::new(manifest(), "Frozen test encounter")
            .expect("valid manifest prepares a host handoff");
        let mut app = App::new();
        app.init_resource::<MultiplayerModel>()
            .init_resource::<SessionUiNotice>()
            .insert_resource(PendingDirectHostSetup { endpoint })
            .insert_resource(prepared)
            .add_systems(Update, queue_prepared_host_after_sandbox);

        app.update();

        assert!(matches!(
            app.world().get_resource::<DirectStartQueue>(),
            Some(DirectStartQueue::Host { .. })
        ));
        assert!(app.world().get_resource::<ActiveDirectSession>().is_none());
        assert_eq!(
            app.world().resource::<MultiplayerModel>().role,
            Some(MultiplayerRole::Host)
        );
    }

    #[test]
    fn active_phase_waits_for_the_exact_local_world_before_gameplay() {
        let mut model = MultiplayerModel::default();
        assert!(model.admitted(MultiplayerRole::Client, PlayerSeat(1)));
        let mut next_screen = NextState::Unchanged;

        project_lobby_phase(LobbyPhase::Active, None, &mut model, &mut next_screen);

        assert_eq!(model.route, hex_gameplay_model::MultiplayerRoute::Loading);
        assert!(matches!(next_screen, NextState::Unchanged));

        let ready = DirectWorldReady {
            fingerprint: PublicWorldFingerprint(9),
        };
        project_lobby_phase(
            LobbyPhase::Active,
            Some(&ready),
            &mut model,
            &mut next_screen,
        );
        assert!(matches!(next_screen, NextState::Pending(Screen::Gameplay)));
    }

    fn intent_adapter_app(role: MultiplayerRole) -> App {
        let mut model = MultiplayerModel::default();
        model.connecting(role);
        let seat = if role == MultiplayerRole::Host {
            PlayerSeat::HOST
        } else {
            PlayerSeat(1)
        };
        assert!(model.admitted(role, seat));

        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::state::app::StatesPlugin))
            .insert_state(Screen::Multiplayer)
            .insert_resource(model)
            .init_resource::<MultiplayerDraft>()
            .init_resource::<StoredCredentialState>()
            .init_resource::<SessionProjection>()
            .init_resource::<SessionUiRequestIds>()
            .init_resource::<SessionUiNotice>()
            .init_resource::<MainMenuModel>()
            .add_message::<UiIntent>()
            .add_message::<HostSessionControlRequest>()
            .add_message::<ClientLobbyRequest>()
            .add_systems(Update, handle_intents);
        app
    }

    #[test]
    fn client_host_only_action_is_a_typed_local_refusal_with_no_wire_request() {
        let mut app = intent_adapter_app(MultiplayerRole::Client);
        app.world_mut()
            .write_message(UiIntent::Multiplayer(MultiplayerIntent::AssignUnit {
                unit: UnitId(0),
                destination: PlayerSeat(1),
            }));

        app.update();

        assert_eq!(
            app.world().resource::<SessionUiNotice>().0.as_deref(),
            Some("Only the listen host may perform that session action.")
        );
        assert!(app
            .world_mut()
            .resource_mut::<Messages<HostSessionControlRequest>>()
            .drain()
            .next()
            .is_none());
        assert!(app
            .world_mut()
            .resource_mut::<Messages<ClientLobbyRequest>>()
            .drain()
            .next()
            .is_none());
    }

    #[test]
    fn role_adapter_keeps_host_controls_local_and_client_readiness_seatless() {
        let mut host = intent_adapter_app(MultiplayerRole::Host);
        host.world_mut()
            .write_message(UiIntent::Multiplayer(MultiplayerIntent::AssignUnit {
                unit: UnitId(0),
                destination: PlayerSeat(1),
            }));
        host.update();
        let host_request = host
            .world_mut()
            .resource_mut::<Messages<HostSessionControlRequest>>()
            .drain()
            .next()
            .expect("listen host receives a trusted local request");
        assert_eq!(
            host_request.action,
            HostSessionAction::AssignUnit {
                unit: UnitId(0),
                destination: PlayerSeat(1),
            }
        );
        assert!(host
            .world_mut()
            .resource_mut::<Messages<ClientLobbyRequest>>()
            .drain()
            .next()
            .is_none());

        let mut client = intent_adapter_app(MultiplayerRole::Client);
        client
            .world_mut()
            .write_message(UiIntent::Multiplayer(MultiplayerIntent::SetReady(true)));
        client.update();
        let client_request = client
            .world_mut()
            .resource_mut::<Messages<ClientLobbyRequest>>()
            .drain()
            .next()
            .expect("guest readiness produces one seatless request");
        assert_eq!(client_request.action, ClientLobbyAction::SetReady(true));
        assert!(client
            .world_mut()
            .resource_mut::<Messages<HostSessionControlRequest>>()
            .drain()
            .next()
            .is_none());
    }
}
