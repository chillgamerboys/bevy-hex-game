//! Application adapter for Direct Connect, lobby control, and session presentation.
//!
//! Socket construction lives behind explicit host/join actions. World-owned code supplies
//! [`PreparedDirectSandboxSession`] and [`DirectWorldReady`]; this module never substitutes
//! `GenerationReport::map_fingerprint` for the complete public-world contract.

use std::{
    collections::{BTreeMap, BTreeSet},
    time::{SystemTime, UNIX_EPOCH},
};

use bevy::prelude::*;
use bevy_replicon::prelude::{ClientId, ClientState, ProtocolHash, SendTargets, ToClients};
use hex_assets::{
    AcceptedContentRevision, CombatSettings, CubeCoord, Encounter, EncounterFaction,
    EncounterPlacement, FormationCenter, Roster, RosterEntry, ScenarioLibrary, SubstanceTable,
};
use hex_core::{
    CommandRequestId, GameplayPhase, InputAction, InputBindings, LocalMapKnowledge, PlayerSeat,
    ResolvedMapSeed, Screen, SimulationRole, UnitId,
};
use hex_gameplay_model::{
    MainMenuModel, MainMenuRoute, MultiplayerBackResult, MultiplayerEndReason, MultiplayerModel,
    MultiplayerRole,
};
use hex_map::{
    diff_world_snapshots_v1, CurrentWorldSnapshotV1, WorldReplicationOutcomeV1,
    WorldReplicationRefusalV1, WorldReplicationRequestV1, WorldReplicationResultV1,
};
use hex_multiplayer::{
    AdmissionAccepted, AdmissionCredential, AdmissionRefusal, AdmissionRefusalReason,
    AtomicFileReconnectCredentialStore, AuthorityBoundary, AuthoritySequence,
    AuthorizedSessionClient, BoundedVec, BuildIdentityV1, CertificateFingerprint, ClientHello,
    ClientLobbyAction, ClientLobbyRequest, ClientMapReady, CommandSequencer, ContentFingerprint,
    CredentialStorageOperation, CredentialStorageStatus, DirectConnectionCode, DirectEndpoint,
    EncodedConnectionCode, HostSessionAction, HostSessionControlRequest, LiveSessionSnapshotV1,
    LiveSnapshotHeaderV1, LobbyPhase, LobbySnapshot, PlayerKnowledgeSnapshotV1, PreparedDirectHost,
    PreparedDirectJoin, PreparedDirectReconnect, PublicWorldFingerprint,
    ReconnectCredentialStorage, ReconnectEndpointBinding, SeatConnectionState,
    SessionAdmissionAuthority, SessionCloseReason, SessionClosed, SessionControlOutcome,
    SessionControlRefusal, SessionControlResult, SessionInstanceId, SessionManifestV1,
    SessionReplica, StoredReconnectCredential, UnitReplica, WorldDeltaV1, WorldSnapshotV1,
    LIVE_SESSION_SNAPSHOT_VERSION_V1, MAX_SESSION_UNITS,
};
use hex_perception::{
    export_player_knowledge_snapshot_v1, import_player_knowledge_snapshot_v1, FactionMapKnowledge,
};
use hex_ui::{
    MultiplayerAssignmentView, MultiplayerIntent, MultiplayerSeatConnectionView,
    MultiplayerSeatView, MultiplayerTextField, MultiplayerView, SensitiveText, UiIntent, UiSystems,
};

use crate::multiplayer_gameplay::{ApplyReplicaBaseline, MultiplayerGameplaySystems};
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
        prepared: Box<PreparedDirectSandboxSession>,
    },
    Join {
        target: DirectJoinTarget,
        credential: AdmissionCredential,
        reconnecting: bool,
    },
}

#[derive(Debug, Clone)]
enum DirectJoinTarget {
    Invite(DirectConnectionCode),
    Reconnect(ReconnectEndpointBinding),
}

#[derive(Debug, Clone)]
struct HostedCodeSource {
    endpoint: DirectEndpoint,
    certificate_fingerprint: CertificateFingerprint,
    certificate_expires_unix_seconds: u64,
}

#[derive(Resource, Debug)]
struct ActiveDirectSession {
    entity: Entity,
    role: MultiplayerRole,
    hosted_code: Option<HostedCodeSource>,
}

trait ClipboardTextWriter {
    fn write_text(&mut self, text: &str) -> Result<(), bevy::clipboard::ClipboardError>;
}

impl ClipboardTextWriter for Clipboard {
    fn write_text(&mut self, text: &str) -> Result<(), bevy::clipboard::ClipboardError> {
        Clipboard::set_text(self, text.to_owned())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectionCodeCopyError {
    NoActiveHostCode,
    ClipboardUnavailable,
    ClipboardWriteFailed,
}

impl ConnectionCodeCopyError {
    const fn message(self) -> &'static str {
        match self {
            Self::NoActiveHostCode => "No active host connection code is available to copy.",
            Self::ClipboardUnavailable => "The system clipboard is unavailable on this device.",
            Self::ClipboardWriteFailed => {
                "The system clipboard refused the connection code. Try again."
            }
        }
    }
}

fn hosted_connection_code(
    active: Option<&ActiveDirectSession>,
    authority: Option<&SessionAdmissionAuthority>,
) -> Option<EncodedConnectionCode> {
    let source = active
        .filter(|active| active.role == MultiplayerRole::Host)?
        .hosted_code
        .as_ref()?;
    let authority = authority?;
    Some(
        DirectConnectionCode {
            endpoint: source.endpoint.clone(),
            certificate_fingerprint: source.certificate_fingerprint,
            certificate_expires_unix_seconds: source.certificate_expires_unix_seconds,
            invite_token: authority.invite_token(),
        }
        .encode(),
    )
}

fn copy_hosted_connection_code<W: ClipboardTextWriter>(
    active: Option<&ActiveDirectSession>,
    authority: Option<&SessionAdmissionAuthority>,
    clipboard: Option<&mut W>,
) -> Result<(), ConnectionCodeCopyError> {
    let code = hosted_connection_code(active, authority)
        .ok_or(ConnectionCodeCopyError::NoActiveHostCode)?;
    let clipboard = clipboard.ok_or(ConnectionCodeCopyError::ClipboardUnavailable)?;
    match clipboard.write_text(code.expose_for_sharing()) {
        Ok(()) => Ok(()),
        Err(_) => Err(ConnectionCodeCopyError::ClipboardWriteFailed),
    }
}

#[derive(Resource, Debug)]
struct PendingClientHello {
    credential: AdmissionCredential,
    sent: bool,
    transport_observed: bool,
}

#[derive(Resource, Debug, Default)]
struct MapReadyReportState {
    sent: bool,
}

/// One lobby loading epoch whose local world is being generated and verified.
#[derive(Resource, Debug, Default)]
pub(crate) struct DirectMapLoadState {
    session: Option<SessionInstanceId>,
    loading: bool,
    started: bool,
    awaiting_snapshot: bool,
    restored_reconnect: bool,
}

impl DirectMapLoadState {
    pub(crate) const fn is_loading(&self) -> bool {
        self.loading
    }

    fn mark_reconnect_restored(&mut self) {
        self.awaiting_snapshot = false;
        self.restored_reconnect = true;
    }
}

#[derive(Resource, Debug, Default)]
struct HostOutcomeState {
    sent: bool,
}

#[derive(Resource, Debug)]
struct HostShutdownCountdown(u8);

#[derive(Resource, Debug, Default)]
struct PendingReconnectSnapshotTargets(BTreeSet<Entity>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReplicaWorldRequestKind {
    Baseline(AuthoritySequence),
    Delta(AuthoritySequence),
}

#[derive(Resource, Debug, Default)]
struct ReplicaWorldSyncState {
    baseline: Option<Box<LiveSessionSnapshotV1>>,
    player_knowledge: Option<PlayerKnowledgeSnapshotV1>,
    deltas: BTreeMap<AuthoritySequence, WorldDeltaV1>,
    in_flight: Option<ReplicaWorldRequestKind>,
}

#[derive(Resource, Debug, Default)]
struct HostWorldDeltaState {
    snapshot: Option<WorldSnapshotV1>,
    authority_sequence: Option<AuthoritySequence>,
}

#[derive(Resource, Debug, Default)]
struct HostPlayerKnowledgeState(Option<PlayerKnowledgeSnapshotV1>);

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
        .init_resource::<InputBindings>()
        .init_resource::<MultiplayerDraft>()
        .init_resource::<SessionUiNotice>()
        .init_resource::<SessionProjection>()
        .init_resource::<StoredCredentialState>()
        .init_resource::<SessionUiRequestIds>()
        .init_resource::<MapReadyReportState>()
        .init_resource::<DirectMapLoadState>()
        .init_resource::<HostOutcomeState>()
        .init_resource::<PendingReconnectSnapshotTargets>()
        .init_resource::<ReplicaWorldSyncState>()
        .init_resource::<HostWorldDeltaState>()
        .init_resource::<HostPlayerKnowledgeState>()
        .add_systems(Startup, load_stored_credential)
        .add_systems(
            OnEnter(Screen::Multiplayer),
            queue_prepared_host_after_sandbox,
        )
        .add_systems(
            Update,
            (
                handle_back_input,
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
                detect_failed_client_connection,
                detect_failed_host_endpoint,
                sync_host_session,
                drive_direct_map_loading,
                finish_host_shutdown,
                observe_current_world_ready,
                report_local_map_ready,
                capture_reconnect_snapshot_targets,
                publish_host_world_deltas
                    .after(MultiplayerGameplaySystems::PublishAuthorityProjection),
                send_pending_reconnect_snapshots,
                finish_replica_world_request,
                capture_replica_world_messages,
                issue_replica_world_request,
                apply_replica_player_knowledge
                    .before(MultiplayerGameplaySystems::ApplyReplicaProjection),
                publish_host_player_knowledge,
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

fn handle_back_input(
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<InputBindings>,
    mut intents: MessageWriter<UiIntent>,
) {
    if bindings.just_pressed(&keys, InputAction::Cancel) {
        intents.write(UiIntent::Multiplayer(MultiplayerIntent::Back));
    }
}

fn load_stored_credential(
    storage: Res<ReconnectCredentialStorage>,
    mut state: ResMut<StoredCredentialState>,
    mut notice: ResMut<SessionUiNotice>,
) {
    let Some(now) = current_unix_seconds() else {
        state.value = None;
        notice.0 = Some(
            "The system clock cannot validate the temporary reconnect credential; ordinary Direct Join remains available."
                .to_owned(),
        );
        return;
    };
    if storage.store().delete_if_expired(now).is_err() {
        state.value = None;
        notice.0 = Some(
            "Expired reconnect state could not be checked; ordinary Direct Join remains available."
                .to_owned(),
        );
        return;
    }
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
        prepared: Box::new(prepared),
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
    mut stored: ResMut<StoredCredentialState>,
    storage: Option<Res<ReconnectCredentialStorage>>,
    projection: Res<SessionProjection>,
    authority: Option<Res<SessionAdmissionAuthority>>,
    active: Option<Res<ActiveDirectSession>>,
    mut ids: ResMut<SessionUiRequestIds>,
    mut notice: ResMut<SessionUiNotice>,
    mut clipboard: Option<ResMut<Clipboard>>,
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
            MultiplayerIntent::CopyConnectionCode => {
                notice.0 = Some(
                    match copy_hosted_connection_code(
                        active.as_deref(),
                        authority.as_deref(),
                        clipboard.as_deref_mut(),
                    ) {
                        Ok(()) => "Direct connection code copied to the clipboard.",
                        Err(error) => error.message(),
                    }
                    .to_owned(),
                );
            }
            MultiplayerIntent::JoinDirect | MultiplayerIntent::ReconnectDirect => {
                if active.is_some() {
                    notice.0 = Some("A direct session is already active.".to_owned());
                    continue;
                }
                let reconnecting = matches!(intent, MultiplayerIntent::ReconnectDirect);
                let (target, credential) = if reconnecting {
                    let Some(stored_credential) = stored.value.clone() else {
                        notice.0 = Some(
                            "No private reconnect credential is available in temporary storage."
                                .to_owned(),
                        );
                        continue;
                    };
                    let Some(now) = current_unix_seconds() else {
                        notice.0 = Some(
                            "The system clock cannot validate the private reconnect credential."
                                .to_owned(),
                        );
                        continue;
                    };
                    if stored_credential.is_expired_at(now) {
                        if let Some(storage) = storage.as_deref() {
                            let _deleted = storage.store().delete_if_expired(now);
                        }
                        stored.value = None;
                        notice.0 = Some(
                            "The saved host certificate has expired. Ask the host for a fresh session invite."
                                .to_owned(),
                        );
                        continue;
                    }
                    (
                        DirectJoinTarget::Reconnect(stored_credential.endpoint_binding),
                        AdmissionCredential::Reconnect(stored_credential.reconnect_credential),
                    )
                } else {
                    let code = match DirectConnectionCode::parse(draft.join_code.expose()) {
                        Ok(code) => code,
                        Err(error) => {
                            notice.0 = Some(format!("Direct connection code refused: {error}."));
                            continue;
                        }
                    };
                    let credential = AdmissionCredential::Invite(code.invite_token);
                    (DirectJoinTarget::Invite(code), credential)
                };
                if reconnecting {
                    model.connecting(MultiplayerRole::Client);
                    model.show_reconnecting();
                } else {
                    model.connecting(MultiplayerRole::Client);
                }
                notice.0 = None;
                commands.insert_resource(DirectStartQueue::Join {
                    target,
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
                    commands.remove_resource::<DirectStartQueue>();
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

fn current_unix_seconds() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|elapsed| elapsed.as_secs())
}

fn direct_endpoint(draft: &MultiplayerDraft) -> Result<DirectEndpoint, String> {
    let port = draft
        .advertised_port
        .parse::<u16>()
        .map_err(|_error| "UDP port must be a number from 1 through 65535.".to_owned())?;
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
        (Some(MultiplayerRole::Client), None) => {
            commands.remove_resource::<DirectStartQueue>();
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
            start_direct_host(world, endpoint, *prepared)
        }
        DirectStartQueue::Join {
            target,
            credential,
            reconnecting,
        } => start_direct_join(world, &target, credential, reconnecting),
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
        certificate_expires_unix_seconds: direct.connection_code().certificate_expires_unix_seconds,
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
    world.insert_resource(DirectMapLoadState::default());
    world.insert_resource(PendingReconnectSnapshotTargets::default());
    world.insert_resource(ReplicaWorldSyncState::default());
    world.insert_resource(HostWorldDeltaState::default());
    world.insert_resource(HostPlayerKnowledgeState::default());
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
    target: &DirectJoinTarget,
    credential: AdmissionCredential,
    reconnecting: bool,
) -> Result<(), String> {
    if world.get_resource::<AcceptedContentRevision>().is_none() {
        return Err("Shipped content is still loading; Direct Join was not started.".to_owned());
    }
    let entity = match target {
        DirectJoinTarget::Invite(code) => PreparedDirectJoin::new(code)
            .map_err(|error| format!("Could not prepare the pinned direct connection: {error}."))?
            .connect(world),
        DirectJoinTarget::Reconnect(binding) => PreparedDirectReconnect::new(binding)
            .map_err(|error| format!("Could not prepare the pinned direct reconnect: {error}."))?
            .connect(world),
    };
    world.insert_resource(ActiveDirectSession {
        entity,
        role: MultiplayerRole::Client,
        hosted_code: None,
    });
    world.insert_resource(PendingClientHello {
        credential,
        sent: false,
        transport_observed: false,
    });
    world.insert_resource(SimulationRole::Replica);
    world.remove_resource::<DirectWorldReady>();
    world.insert_resource(DirectMapLoadState::default());
    world.insert_resource(PendingReconnectSnapshotTargets::default());
    world.insert_resource(ReplicaWorldSyncState::default());
    world.insert_resource(HostWorldDeltaState::default());
    world.insert_resource(HostPlayerKnowledgeState::default());
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
    let Some(state) = state.as_deref() else {
        return;
    };
    match *state.get() {
        ClientState::Disconnected => return,
        ClientState::Connecting => {
            pending.transport_observed = true;
            return;
        }
        ClientState::Connected => pending.transport_observed = true,
    }
    if pending.sent {
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

pub(crate) fn local_build_identity() -> Result<BuildIdentityV1, hex_multiplayer::BoundError> {
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
            end_active_session(
                MultiplayerEndReason::ProtocolViolation,
                active.as_deref(),
                &mut model,
                &mut commands,
            );
            next_screen.set(Screen::Multiplayer);
        } else {
            commands.remove_resource::<PendingClientHello>();
        }
    }
    if let Some(manifest) = manifests.read().last() {
        projection.manifest = Some(manifest.clone());
    }
    if let Some(lobby) = lobbies.read().last() {
        projection.lobby = Some(lobby.clone());
        if model.role.is_some() {
            project_lobby_phase(lobby.phase, ready.as_deref(), &mut model, &mut next_screen);
        }
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
            CredentialStorageStatus::Preserved => {}
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

fn detect_failed_client_connection(
    state: Option<Res<State<ClientState>>>,
    pending: Option<Res<PendingClientHello>>,
    active: Option<Res<ActiveDirectSession>>,
    mut model: ResMut<MultiplayerModel>,
    mut notice: ResMut<SessionUiNotice>,
    mut commands: Commands,
    mut next_screen: ResMut<NextState<Screen>>,
) {
    let (Some(state), Some(pending), Some(active)) = (state, pending, active) else {
        return;
    };
    if active.role != MultiplayerRole::Client
        || model.role != Some(MultiplayerRole::Client)
        || model.local_seat.is_some()
        || !pending.transport_observed
        || *state.get() != ClientState::Disconnected
    {
        return;
    }
    notice.0 = Some(
        "The direct connection ended before admission completed. Check the host address, UDP forwarding, and firewall, then try again."
            .to_owned(),
    );
    end_active_session(
        MultiplayerEndReason::ConnectionFailed,
        Some(&active),
        &mut model,
        &mut commands,
    );
    next_screen.set(Screen::Multiplayer);
}

fn detect_failed_host_endpoint(
    active: Option<Res<ActiveDirectSession>>,
    entities: Query<()>,
    mut model: ResMut<MultiplayerModel>,
    mut notice: ResMut<SessionUiNotice>,
    mut commands: Commands,
    mut next_screen: ResMut<NextState<Screen>>,
) {
    let Some(active) = active else {
        return;
    };
    if active.role != MultiplayerRole::Host
        || model.role != Some(MultiplayerRole::Host)
        || entities.get(active.entity).is_ok()
    {
        return;
    }
    notice.0 = Some(
        "The direct host endpoint closed. Check whether the UDP port is already in use or blocked, then try again."
            .to_owned(),
    );
    end_active_session(
        MultiplayerEndReason::ConnectionFailed,
        Some(&active),
        &mut model,
        &mut commands,
    );
    next_screen.set(Screen::Multiplayer);
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

#[expect(
    clippy::too_many_arguments,
    reason = "the composition adapter joins the frozen lobby, local launch owner, and screen state without moving authority into shared protocol code"
)]
fn drive_direct_map_loading(
    screen: Res<State<Screen>>,
    active: Option<Res<ActiveDirectSession>>,
    authority: Option<Res<SessionAdmissionAuthority>>,
    projection: Res<SessionProjection>,
    scenarios: Option<Res<ScenarioLibrary>>,
    sandbox: Option<Res<super::sandbox::SandboxSession>>,
    local_rules: Option<Res<CombatSettings>>,
    ready: Option<Res<DirectWorldReady>>,
    mut state: ResMut<DirectMapLoadState>,
    mut report_state: ResMut<MapReadyReportState>,
    mut phase: ResMut<GameplayPhase>,
    mut model: ResMut<MultiplayerModel>,
    mut notice: ResMut<SessionUiNotice>,
    mut commands: Commands,
    mut next_screen: ResMut<NextState<Screen>>,
) {
    let lobby = authority
        .as_deref()
        .map(|authority| authority.lobby().snapshot_owned())
        .or_else(|| projection.lobby.clone());
    let manifest = authority
        .as_deref()
        .map(|authority| authority.manifest().clone())
        .or_else(|| projection.manifest.clone());
    let (Some(active), Some(lobby), Some(manifest)) = (active, lobby, manifest) else {
        state.loading = false;
        state.started = false;
        state.awaiting_snapshot = false;
        state.restored_reconnect = false;
        state.session = None;
        return;
    };

    let reconnect_epoch = active.role == MultiplayerRole::Client
        && matches!(lobby.phase, LobbyPhase::Active | LobbyPhase::Outcome)
        && (state.awaiting_snapshot || (ready.is_none() && !state.restored_reconnect));
    let requires_load = lobby.phase == LobbyPhase::Loading || reconnect_epoch;
    if !requires_load {
        if matches!(lobby.phase, LobbyPhase::Active | LobbyPhase::Outcome)
            && ready.as_deref().is_some_and(|ready| {
                state.restored_reconnect
                    || ready.fingerprint == manifest.map.expected_public_fingerprint
            })
        {
            *phase = GameplayPhase::Active;
            NextState::set_if_neq(&mut next_screen, Screen::Gameplay);
        }
        state.loading = false;
        state.started = false;
        state.awaiting_snapshot = false;
        state.session = Some(manifest.session_instance_id);
        return;
    }

    if state.session != Some(manifest.session_instance_id) || !state.loading {
        state.session = Some(manifest.session_instance_id);
        state.loading = true;
        state.started = false;
        state.awaiting_snapshot = reconnect_epoch;
        state.restored_reconnect = false;
        report_state.sent = false;
        *phase = GameplayPhase::Preparing;
        commands.remove_resource::<DirectWorldReady>();
    }
    model.show_loading();
    if state.started || matches!(screen.get(), Screen::Loading | Screen::Gameplay) {
        return;
    }

    let loading_input = match active.role {
        MultiplayerRole::Host => {
            let Some(sandbox) = sandbox.as_deref() else {
                notice.0 = Some(
                    "The host's frozen Sandbox launch is unavailable; exact map loading was refused."
                        .to_owned(),
                );
                end_active_session(
                    MultiplayerEndReason::ProtocolViolation,
                    Some(&active),
                    &mut model,
                    &mut commands,
                );
                next_screen.set(Screen::Multiplayer);
                return;
            };
            commands.insert_resource(sandbox.launch.rules.clone());
            sandbox.launch.loading_input()
        }
        MultiplayerRole::Client => {
            let (Some(scenarios), Some(local_rules)) =
                (scenarios.as_deref(), local_rules.as_deref())
            else {
                return;
            };
            let local_rules_fingerprint = super::sandbox::direct_rules_fingerprint(local_rules);
            if manifest.rules.profile_identity.as_str() != "sandbox-rules-v1"
                || manifest.rules.fingerprint != local_rules_fingerprint
            {
                notice.0 = Some(
                    "Rules mismatch: Direct multiplayer requires the exact shipped Sandbox rules."
                        .to_owned(),
                );
                end_active_session(
                    MultiplayerEndReason::Incompatible,
                    Some(&active),
                    &mut model,
                    &mut commands,
                );
                next_screen.set(Screen::Multiplayer);
                return;
            }
            match client_scenario_to_load(&manifest, scenarios) {
                Ok(loading_input) => loading_input,
                Err(reason) => {
                    notice.0 = Some(reason);
                    end_active_session(
                        MultiplayerEndReason::Incompatible,
                        Some(&active),
                        &mut model,
                        &mut commands,
                    );
                    next_screen.set(Screen::Multiplayer);
                    return;
                }
            }
        }
    };

    commands.insert_resource(loading_input);
    commands.insert_resource(GameplayPhase::Preparing);
    next_screen.set(Screen::Loading);
    state.started = true;
}

fn client_scenario_to_load(
    manifest: &SessionManifestV1,
    scenarios: &ScenarioLibrary,
) -> Result<crate::scenarios::ScenarioToLoad, String> {
    let scenario = scenarios
        .scenarios
        .iter()
        .find(|scenario| scenario.name == manifest.scenario_identity.as_str())
        .cloned()
        .ok_or_else(|| {
            "Scenario mismatch: the host's shipped scenario is unavailable locally.".to_owned()
        })?;
    let mut units = Vec::with_capacity(manifest.shipped_roster.len());
    for entry in manifest.shipped_roster.as_slice() {
        let placement = manifest
            .deployment
            .as_slice()
            .iter()
            .find(|placement| placement.unit == entry.unit)
            .ok_or_else(|| "The host manifest omits a party deployment surface.".to_owned())?;
        units.push(RosterEntry {
            archetype: entry.archetype_identity.as_str().to_owned(),
            placement: Some(EncounterPlacement::Surface(placement.position)),
            ai_profile: None,
            ai_group: None,
        });
    }
    let encounter = Encounter {
        name: format!("Direct replica · {}", manifest.scenario_identity.as_str()),
        rosters: vec![Roster {
            faction: EncounterFaction::Player,
            placement: EncounterPlacement::Formation {
                center: FormationCenter::Fixed(CubeCoord { x: 0, y: 0, z: 0 }),
                spread: 0,
            },
            units,
        }],
    };
    Ok(crate::scenarios::ScenarioToLoad {
        resolved_seed: scenario
            .generation_seed
            .map(|_configured| ResolvedMapSeed(manifest.map.seed)),
        scenario,
        encounter_override: Some(encounter),
    })
}

fn observe_current_world_ready(
    screen: Res<State<Screen>>,
    state: Res<DirectMapLoadState>,
    current: Option<Res<CurrentWorldSnapshotV1>>,
    existing: Option<Res<DirectWorldReady>>,
    mut commands: Commands,
) {
    if !state.loading || *screen.get() != Screen::Gameplay {
        return;
    }
    let Some(current) = current.as_deref() else {
        return;
    };
    let fingerprint = current.fingerprint();
    if existing
        .as_deref()
        .is_some_and(|ready| ready.fingerprint == fingerprint)
    {
        return;
    }
    commands.insert_resource(DirectWorldReady { fingerprint });
}

fn capture_reconnect_snapshot_targets(
    added: Query<(Entity, &AuthorizedSessionClient), Added<AuthorizedSessionClient>>,
    authority: Option<Res<SessionAdmissionAuthority>>,
    mut pending: ResMut<PendingReconnectSnapshotTargets>,
) {
    let Some(authority) = authority else {
        return;
    };
    if authority.lobby().snapshot().phase == LobbyPhase::Open {
        return;
    }
    for (connection, client) in &added {
        if authority.active_connection(client.seat) == Some(connection) {
            pending.0.insert(connection);
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "a restart baseline intentionally joins the exact world, knowledge, and disclosure-safe gameplay projections at one quiescent authority boundary"
)]
fn send_pending_reconnect_snapshots(
    boundary: Res<AuthorityBoundary>,
    authority: Option<Res<SessionAdmissionAuthority>>,
    current_world: Option<Res<CurrentWorldSnapshotV1>>,
    knowledge: Option<Res<FactionMapKnowledge>>,
    substances: Option<Res<SubstanceTable>>,
    sequencer: Res<CommandSequencer>,
    units: Query<&UnitReplica>,
    sessions: Query<&SessionReplica>,
    mut pending: ResMut<PendingReconnectSnapshotTargets>,
    mut snapshots: MessageWriter<ToClients<LiveSessionSnapshotV1>>,
) {
    if pending.0.is_empty() || !boundary.is_quiescent() {
        return;
    }
    let (Some(authority), Some(current_world), Some(knowledge), Some(substances)) =
        (authority, current_world, knowledge, substances)
    else {
        return;
    };
    if !matches!(
        authority.lobby().snapshot().phase,
        LobbyPhase::Active | LobbyPhase::Outcome
    ) {
        return;
    }
    let Some(mut session) = sessions
        .iter()
        .filter(|session| session.validate().is_ok())
        .max_by_key(|session| session.authority_sequence)
        .cloned()
    else {
        return;
    };
    let mut units = units
        .iter()
        .filter(|unit| unit.validate().is_ok())
        .cloned()
        .collect::<Vec<_>>();
    units.sort_by_key(|unit| unit.unit);
    let Ok(units) = BoundedVec::<_, MAX_SESSION_UNITS>::new(units) else {
        error!("reconnect baseline exceeded the authorized unit bound");
        return;
    };
    let Ok(player_knowledge) = export_player_knowledge_snapshot_v1(&knowledge, &substances) else {
        error!("player knowledge could not be exported for a reconnect baseline");
        return;
    };
    let baseline_sequence = sequencer.last_sequence();
    session.authority_sequence = baseline_sequence;
    let snapshot = LiveSessionSnapshotV1 {
        version: LIVE_SESSION_SNAPSHOT_VERSION_V1,
        manifest: authority.manifest().clone(),
        world: current_world.snapshot().clone(),
        player_knowledge,
        units,
        session,
        baseline_sequence,
    };

    let connected = authority
        .connected_peers()
        .into_iter()
        .map(|(_seat, connection)| connection)
        .collect::<BTreeSet<_>>();
    pending.0.retain(|connection| {
        if !connected.contains(connection) {
            return false;
        }
        snapshots.write(ToClients {
            targets: SendTargets::Single(ClientId::Client(*connection)),
            message: snapshot.clone(),
        });
        false
    });
}

fn capture_replica_world_messages(
    role: Res<SimulationRole>,
    projection: Res<SessionProjection>,
    mut knowledge: MessageReader<PlayerKnowledgeSnapshotV1>,
    mut snapshots: MessageReader<LiveSessionSnapshotV1>,
    mut deltas: MessageReader<WorldDeltaV1>,
    mut state: ResMut<ReplicaWorldSyncState>,
    mut notice: ResMut<SessionUiNotice>,
) {
    if *role != SimulationRole::Replica {
        knowledge.clear();
        snapshots.clear();
        deltas.clear();
        return;
    }
    for projection in knowledge.read() {
        if projection.validate().is_ok() {
            state.player_knowledge = Some(projection.clone());
        } else {
            notice.0 = Some("The host sent an invalid player-knowledge projection.".to_owned());
        }
    }
    for snapshot in snapshots.read() {
        let structurally_valid = LiveSnapshotHeaderV1::new(snapshot.baseline_sequence, 0)
            .is_ok_and(|header| snapshot.validate_with_header(header).is_ok());
        if !structurally_valid {
            notice.0 = Some("The host sent an invalid reconnect baseline.".to_owned());
            continue;
        }
        let Some(manifest) = projection.manifest.as_ref() else {
            continue;
        };
        if &snapshot.manifest != manifest
            || snapshot.baseline_sequence != snapshot.session.authority_sequence
        {
            notice.0 = Some(
                "The reconnect baseline did not match the admitted session manifest.".to_owned(),
            );
            continue;
        }
        if state
            .baseline
            .as_ref()
            .is_none_or(|current| snapshot.baseline_sequence >= current.baseline_sequence)
        {
            state
                .deltas
                .retain(|sequence, _delta| *sequence > snapshot.baseline_sequence);
            state.player_knowledge = None;
            state.baseline = Some(Box::new(snapshot.clone()));
            state.in_flight = None;
        }
    }
    for delta in deltas.read() {
        if delta.validate().is_err() {
            notice.0 = Some("The host sent an invalid ordered world delta.".to_owned());
            continue;
        }
        let covered_by_baseline = state
            .baseline
            .as_ref()
            .is_some_and(|snapshot| delta.authority_sequence <= snapshot.baseline_sequence);
        if !covered_by_baseline {
            state
                .deltas
                .entry(delta.authority_sequence)
                .or_insert_with(|| delta.clone());
        }
    }
}

fn issue_replica_world_request(
    role: Res<SimulationRole>,
    screen: Res<State<Screen>>,
    load: Res<DirectMapLoadState>,
    current_world: Option<Res<CurrentWorldSnapshotV1>>,
    mut state: ResMut<ReplicaWorldSyncState>,
    mut requests: MessageWriter<WorldReplicationRequestV1>,
) {
    if *role != SimulationRole::Replica
        || *screen.get() != Screen::Gameplay
        || current_world.is_none()
        || state.in_flight.is_some()
    {
        return;
    }
    if load.awaiting_snapshot {
        let Some(snapshot) = state.baseline.as_ref() else {
            return;
        };
        let sequence = snapshot.baseline_sequence;
        requests.write(WorldReplicationRequestV1::Restore {
            baseline_sequence: sequence,
            snapshot: Box::new(snapshot.world.clone()),
        });
        state.in_flight = Some(ReplicaWorldRequestKind::Baseline(sequence));
        return;
    }
    let Some((&sequence, delta)) = state.deltas.first_key_value() else {
        return;
    };
    requests.write(WorldReplicationRequestV1::ApplyDelta(delta.clone()));
    state.in_flight = Some(ReplicaWorldRequestKind::Delta(sequence));
}

fn apply_replica_player_knowledge(
    role: Res<SimulationRole>,
    screen: Res<State<Screen>>,
    load: Res<DirectMapLoadState>,
    substances: Option<Res<SubstanceTable>>,
    knowledge: Option<Res<FactionMapKnowledge>>,
    local: Option<Res<LocalMapKnowledge>>,
    mut state: ResMut<ReplicaWorldSyncState>,
    mut notice: ResMut<SessionUiNotice>,
    mut commands: Commands,
) {
    if *role != SimulationRole::Replica
        || *screen.get() != Screen::Gameplay
        || load.awaiting_snapshot
        || state.in_flight.is_some()
        || !state.deltas.is_empty()
    {
        return;
    }
    let (Some(projection), Some(substances)) =
        (state.player_knowledge.take(), substances.as_deref())
    else {
        return;
    };
    let mut restored_knowledge = knowledge.as_deref().cloned().unwrap_or_default();
    let mut restored_local = local.as_deref().cloned().unwrap_or_default();
    if let Err(error) = import_player_knowledge_snapshot_v1(
        &projection,
        substances,
        &mut restored_knowledge,
        &mut restored_local,
    ) {
        notice.0 = Some(format!(
            "The player-knowledge projection was refused: {error}."
        ));
        return;
    }
    commands.insert_resource(restored_knowledge);
    commands.insert_resource(restored_local);
}

#[expect(
    clippy::too_many_arguments,
    reason = "transactional world completion restores the matching knowledge and gameplay baseline before releasing replica presentation"
)]
fn finish_replica_world_request(
    mut results: MessageReader<WorldReplicationResultV1>,
    mut state: ResMut<ReplicaWorldSyncState>,
    substances: Option<Res<SubstanceTable>>,
    knowledge: Option<Res<FactionMapKnowledge>>,
    local: Option<Res<LocalMapKnowledge>>,
    active: Option<Res<ActiveDirectSession>>,
    mut load: ResMut<DirectMapLoadState>,
    mut model: ResMut<MultiplayerModel>,
    mut notice: ResMut<SessionUiNotice>,
    mut baselines: MessageWriter<ApplyReplicaBaseline>,
    mut commands: Commands,
    mut next_screen: ResMut<NextState<Screen>>,
) {
    let Some(in_flight) = state.in_flight else {
        results.clear();
        return;
    };
    let sequence = match in_flight {
        ReplicaWorldRequestKind::Baseline(sequence) | ReplicaWorldRequestKind::Delta(sequence) => {
            sequence
        }
    };
    let Some(result) = results
        .read()
        .filter(|result| result.authority_sequence == sequence)
        .last()
        .cloned()
    else {
        return;
    };
    let resulting_fingerprint = match result.outcome {
        WorldReplicationOutcomeV1::Applied { public_fingerprint }
        | WorldReplicationOutcomeV1::Duplicate { public_fingerprint } => public_fingerprint,
        WorldReplicationOutcomeV1::Refused(WorldReplicationRefusalV1::BoundaryBusy) => {
            state.in_flight = None;
            return;
        }
        WorldReplicationOutcomeV1::Refused(reason) => {
            notice.0 = Some(format!(
                "The ordered multiplayer world update was refused: {reason:?}."
            ));
            state.in_flight = None;
            if let Some(active) = active.as_deref() {
                end_active_session(
                    MultiplayerEndReason::ProtocolViolation,
                    Some(active),
                    &mut model,
                    &mut commands,
                );
                next_screen.set(Screen::Multiplayer);
            }
            return;
        }
    };

    match in_flight {
        ReplicaWorldRequestKind::Delta(sequence) => {
            state.deltas.remove(&sequence);
        }
        ReplicaWorldRequestKind::Baseline(sequence) => {
            let Some(snapshot) = state.baseline.take() else {
                notice.0 = Some("The reconnect baseline disappeared during import.".to_owned());
                state.in_flight = None;
                return;
            };
            if snapshot.world.public_fingerprint != resulting_fingerprint {
                notice.0 = Some(
                    "The restored reconnect world did not match its target fingerprint.".to_owned(),
                );
                state.in_flight = None;
                return;
            }
            let Some(substances) = substances.as_deref() else {
                notice.0 = Some(
                    "Shipped substances were unavailable while restoring reconnect knowledge."
                        .to_owned(),
                );
                state.in_flight = None;
                return;
            };
            let mut restored_knowledge = knowledge.as_deref().cloned().unwrap_or_default();
            let mut restored_local = local.as_deref().cloned().unwrap_or_default();
            if let Err(error) = import_player_knowledge_snapshot_v1(
                &snapshot.player_knowledge,
                substances,
                &mut restored_knowledge,
                &mut restored_local,
            ) {
                notice.0 = Some(format!(
                    "The reconnect knowledge baseline was refused: {error}."
                ));
                state.in_flight = None;
                return;
            }
            let baseline = match ApplyReplicaBaseline::new(
                snapshot.units.as_slice().iter().cloned(),
                snapshot.session.clone(),
            ) {
                Ok(baseline) => baseline,
                Err(error) => {
                    notice.0 = Some(format!(
                        "The gameplay reconnect baseline was refused: {error}."
                    ));
                    state.in_flight = None;
                    return;
                }
            };
            commands.insert_resource(restored_knowledge);
            commands.insert_resource(restored_local);
            commands.insert_resource(DirectWorldReady {
                fingerprint: resulting_fingerprint,
            });
            baselines.write(baseline);
            load.mark_reconnect_restored();
            state
                .deltas
                .retain(|delta_sequence, _delta| *delta_sequence > sequence);
        }
    }
    state.in_flight = None;
}

fn publish_host_world_deltas(
    role: Res<SimulationRole>,
    authority: Option<Res<SessionAdmissionAuthority>>,
    boundary: Res<AuthorityBoundary>,
    current: Option<Res<CurrentWorldSnapshotV1>>,
    mut sequencer: ResMut<CommandSequencer>,
    mut state: ResMut<HostWorldDeltaState>,
    mut deltas: MessageWriter<ToClients<WorldDeltaV1>>,
    mut notice: ResMut<SessionUiNotice>,
) {
    let active = *role == SimulationRole::Authority
        && authority
            .as_deref()
            .is_some_and(|authority| authority.lobby().snapshot().phase == LobbyPhase::Active);
    let Some(current) = current else {
        state.snapshot = None;
        state.authority_sequence = None;
        return;
    };
    if !active {
        state.snapshot = None;
        state.authority_sequence = None;
        return;
    }
    let Some(previous) = state.snapshot.as_ref() else {
        state.snapshot = Some(current.snapshot().clone());
        state.authority_sequence = Some(sequencer.last_sequence());
        return;
    };
    if previous.public_fingerprint == current.fingerprint() || !boundary.is_quiescent() {
        return;
    }
    let last_delta = state.authority_sequence.unwrap_or_default();
    let sequence = if sequencer.last_sequence() <= last_delta {
        match sequencer.advance_system_boundary() {
            Ok(sequence) => sequence,
            Err(error) => {
                notice.0 = Some(format!(
                    "The authority sequence could not publish a world update: {error:?}."
                ));
                return;
            }
        }
    } else {
        sequencer.last_sequence()
    };
    let delta = match diff_world_snapshots_v1(previous, current.snapshot(), sequence) {
        Ok(delta) => delta,
        Err(error) => {
            notice.0 = Some(format!(
                "The authoritative world delta could not be built: {error}."
            ));
            return;
        }
    };
    deltas.write(ToClients {
        targets: SendTargets::CLIENTS_ONLY,
        message: delta,
    });
    state.snapshot = Some(current.snapshot().clone());
    state.authority_sequence = Some(sequence);
}

fn publish_host_player_knowledge(
    role: Res<SimulationRole>,
    authority: Option<Res<SessionAdmissionAuthority>>,
    knowledge: Option<Res<FactionMapKnowledge>>,
    substances: Option<Res<SubstanceTable>>,
    mut state: ResMut<HostPlayerKnowledgeState>,
    mut projections: MessageWriter<ToClients<PlayerKnowledgeSnapshotV1>>,
) {
    let active = *role == SimulationRole::Authority
        && authority.as_deref().is_some_and(|authority| {
            matches!(
                authority.lobby().snapshot().phase,
                LobbyPhase::Loading | LobbyPhase::Active | LobbyPhase::Outcome
            )
        });
    if !active {
        state.0 = None;
        return;
    }
    let (Some(knowledge), Some(substances)) = (knowledge.as_deref(), substances.as_deref()) else {
        return;
    };
    let Ok(snapshot) = export_player_knowledge_snapshot_v1(knowledge, substances) else {
        error!("player knowledge could not be exported for ordered multiplayer projection");
        return;
    };
    if state.0.as_ref() == Some(&snapshot) {
        return;
    }
    projections.write(ToClients {
        targets: SendTargets::CLIENTS_ONLY,
        message: snapshot.clone(),
    });
    state.0 = Some(snapshot);
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
            // The host projects the open lobby every update. A reentrant transition
            // would despawn and rebuild the whole UI before its pressed controls can
            // emit intents, leaving the otherwise visible lobby pointer-inert.
            next_screen.set_if_neq(Screen::Multiplayer);
        }
        LobbyPhase::Loading => {
            model.show_loading();
        }
        LobbyPhase::Active if ready.is_some() => next_screen.set_if_neq(Screen::Gameplay),
        LobbyPhase::Active => model.show_loading(),
        LobbyPhase::Outcome => {}
        LobbyPhase::Closed => model.end(MultiplayerEndReason::SessionEnded),
    }
}

fn report_local_map_ready(
    model: Res<MultiplayerModel>,
    projection: Res<SessionProjection>,
    ready: Option<Res<DirectWorldReady>>,
    mut report_state: ResMut<MapReadyReportState>,
    mut ids: ResMut<SessionUiRequestIds>,
    mut client_reports: MessageWriter<ClientMapReady>,
    mut host_reports: MessageWriter<HostSessionControlRequest>,
    mut notice: ResMut<SessionUiNotice>,
) {
    if model.role.is_none()
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
    }
    match model.role {
        Some(MultiplayerRole::Client) => {
            client_reports.write(ClientMapReady {
                public_world_fingerprint: ready.fingerprint,
            });
        }
        Some(MultiplayerRole::Host) => {
            let Some(request_id) = ids.allocate() else {
                notice.0 = Some("Host map verification request IDs are exhausted.".to_owned());
                return;
            };
            host_reports.write(HostSessionControlRequest {
                request_id,
                action: HostSessionAction::ReportHostMapReady {
                    public_world_fingerprint: ready.fingerprint,
                },
            });
        }
        None => return,
    }
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
    let share_code = hosted_connection_code(active.as_deref(), authority.as_deref())
        .map(|code| SensitiveText::new(code.expose_for_sharing()));
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
    (0_u8..=PlayerSeat::LAST_HUMAN.0)
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
        commands.entity(active.entity).try_despawn();
    }
    commands.remove_resource::<ActiveDirectSession>();
    commands.remove_resource::<PendingClientHello>();
    commands.remove_resource::<SessionAdmissionAuthority>();
    commands.remove_resource::<DirectWorldReady>();
    commands.insert_resource(DirectMapLoadState::default());
    commands.insert_resource(PendingReconnectSnapshotTargets::default());
    commands.insert_resource(ReplicaWorldSyncState::default());
    commands.insert_resource(HostWorldDeltaState::default());
    commands.insert_resource(HostPlayerKnowledgeState::default());
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
    use hex_assets::ScenarioCategory;
    use hex_core::{Faction, HexCoord, SimSeeds, TilePos};
    use hex_multiplayer::{
        BoundedText, BoundedVec, InviteToken, MapManifestV1, ProtocolVersion, ReconnectCredential,
        RosterEntryV1, RulesManifestV1, SessionInstanceId, SessionPeerId, UnitDeploymentV1,
        MAX_IDENTITY_BYTES,
    };

    fn text(value: &str) -> BoundedText<MAX_IDENTITY_BYTES> {
        BoundedText::new(value).expect("test identity fits")
    }

    fn manifest() -> SessionManifestV1 {
        SessionManifestV1 {
            session_instance_id: SessionInstanceId::from_bytes([6; 16]),
            protocol: ProtocolVersion::default(),
            build: local_build_identity().expect("local build fits"),
            content_fingerprint: ContentFingerprint(7),
            scenario_identity: text("sandbox"),
            launch_kind: hex_multiplayer::SessionLaunchKindV1::Sandbox,
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

    #[derive(Default)]
    struct RecordingClipboard {
        writes: Vec<String>,
        fail: bool,
    }

    impl ClipboardTextWriter for RecordingClipboard {
        fn write_text(&mut self, text: &str) -> Result<(), bevy::clipboard::ClipboardError> {
            if self.fail {
                return Err(bevy::clipboard::ClipboardError::ClipboardNotSupported);
            }
            self.writes.push(text.to_owned());
            Ok(())
        }
    }

    fn hosted_copy_fixture() -> (ActiveDirectSession, SessionAdmissionAuthority) {
        let authority = SessionAdmissionAuthority::with_session_secrets(
            serde_json::from_str("99").expect("protocol hash is a serialized newtype"),
            manifest(),
            SessionPeerId::from_bytes([1; 16]),
            InviteToken::from_bytes([4; 16]),
        )
        .expect("the deterministic admission fixture is valid");
        let active = ActiveDirectSession {
            entity: Entity::PLACEHOLDER,
            role: MultiplayerRole::Host,
            hosted_code: Some(HostedCodeSource {
                endpoint: DirectEndpoint::new("127.0.0.1", 7_777)
                    .expect("loopback endpoint is valid"),
                certificate_fingerprint: CertificateFingerprint::from_bytes([3; 32]),
                certificate_expires_unix_seconds: 2_000_000_000,
            }),
        };
        (active, authority)
    }

    #[test]
    fn copy_adapter_writes_the_current_host_authority_code_without_disclosing_it() {
        let (active, authority) = hosted_copy_fixture();
        let expected = hosted_connection_code(Some(&active), Some(&authority))
            .expect("the host fixture has a current code");
        let mut clipboard = RecordingClipboard::default();

        assert_eq!(
            copy_hosted_connection_code(Some(&active), Some(&authority), Some(&mut clipboard)),
            Ok(())
        );
        assert_eq!(
            clipboard.writes.first().map(String::as_str),
            Some(expected.expose_for_sharing())
        );
        assert!(!format!("{expected:?}").contains(expected.expose_for_sharing()));
    }

    #[test]
    fn copy_adapter_fails_closed_without_a_host_code_or_working_clipboard() {
        let (active, authority) = hosted_copy_fixture();
        let mut clipboard = RecordingClipboard::default();
        assert_eq!(
            copy_hosted_connection_code(None, Some(&authority), Some(&mut clipboard)),
            Err(ConnectionCodeCopyError::NoActiveHostCode)
        );
        assert!(clipboard.writes.is_empty());
        assert_eq!(
            copy_hosted_connection_code::<RecordingClipboard>(
                Some(&active),
                Some(&authority),
                None,
            ),
            Err(ConnectionCodeCopyError::ClipboardUnavailable)
        );
        clipboard.fail = true;
        assert_eq!(
            copy_hosted_connection_code(Some(&active), Some(&authority), Some(&mut clipboard)),
            Err(ConnectionCodeCopyError::ClipboardWriteFailed)
        );
        assert!(clipboard.writes.is_empty());
    }

    #[test]
    fn canonical_cancel_binding_emits_the_same_typed_back_intent_as_the_button() {
        let mut app = App::new();
        app.init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<InputBindings>()
            .add_message::<UiIntent>()
            .add_systems(Update, handle_back_input);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Escape);

        app.update();

        let intents = app
            .world_mut()
            .resource_mut::<Messages<UiIntent>>()
            .drain()
            .collect::<Vec<_>>();
        assert!(matches!(
            intents.as_slice(),
            [UiIntent::Multiplayer(MultiplayerIntent::Back)]
        ));
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
            HostSessionAction::ReportHostMapReady {
                public_world_fingerprint: PublicWorldFingerprint(9),
            },
            HostSessionAction::RetryExact {
                public_world_fingerprint: PublicWorldFingerprint(9),
            },
            HostSessionAction::ReturnToLobby,
            HostSessionAction::CloseSession,
        ];
        assert_eq!(host.len(), 7);
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
        assert!(matches!(
            next_screen,
            NextState::PendingIfNeq(Screen::Gameplay)
        ));
    }

    #[test]
    fn recurring_lobby_projection_never_reenters_the_current_screen() {
        let mut model = MultiplayerModel::default();
        assert!(model.admitted(MultiplayerRole::Host, PlayerSeat::HOST));
        let mut next_screen = NextState::Unchanged;

        project_lobby_phase(LobbyPhase::Open, None, &mut model, &mut next_screen);

        assert!(matches!(
            next_screen,
            NextState::PendingIfNeq(Screen::Multiplayer)
        ));

        let ready = DirectWorldReady {
            fingerprint: PublicWorldFingerprint(9),
        };
        project_lobby_phase(
            LobbyPhase::Active,
            Some(&ready),
            &mut model,
            &mut next_screen,
        );
        assert!(matches!(
            next_screen,
            NextState::PendingIfNeq(Screen::Gameplay)
        ));
    }

    #[test]
    fn replica_loading_freezes_the_host_seed_party_identity_and_exact_deployment() {
        let mut manifest = manifest();
        manifest.deployment = BoundedVec::new(
            (0_u64..6)
                .map(|unit| UnitDeploymentV1 {
                    unit: UnitId(unit),
                    position: TilePos::new(
                        HexCoord::from_axial(
                            i32::try_from(unit).expect("small test coordinate"),
                            0,
                        ),
                        2,
                    ),
                })
                .collect(),
        )
        .expect("six deployments fit");
        let scenario = hex_assets::Scenario {
            name: "sandbox".to_owned(),
            category: ScenarioCategory::Map,
            blurb: "test".to_owned(),
            world: "config/worlds/test.ron".to_owned(),
            lighting: "config/lighting/test.ron".to_owned(),
            generation_seed: Some(123),
            starting_time_hours: None,
            encounter: "config/encounters/test.ron".to_owned(),
        };
        let scenarios = ScenarioLibrary {
            default_game: scenario.name.clone(),
            scenarios: vec![scenario.clone()],
        };

        let loading = client_scenario_to_load(&manifest, &scenarios)
            .expect("matching shipped scenario should adapt for replica loading");

        assert_eq!(loading.scenario, scenario);
        assert_eq!(
            loading.resolved_seed,
            Some(ResolvedMapSeed(manifest.map.seed))
        );
        let encounter = loading
            .encounter_override
            .expect("replica launch must suppress undisclosed authored hostiles");
        assert_eq!(encounter.unit_count(EncounterFaction::Player), 6);
        assert_eq!(encounter.unit_count(EncounterFaction::Hostile), 0);
        let placements = encounter
            .entries()
            .map(|unit| match unit.placement {
                EncounterPlacement::Surface(position) => Some(*position),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()
            .expect("every replica party member uses its exact host deployment");
        assert_eq!(
            placements,
            manifest
                .deployment
                .as_slice()
                .iter()
                .map(|deployment| deployment.position)
                .collect::<Vec<_>>()
        );
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

    fn stored_reconnect(certificate_expires_unix_seconds: u64) -> StoredReconnectCredential {
        StoredReconnectCredential::new(
            AdmissionAccepted {
                session_instance_id: SessionInstanceId::from_bytes([6; 16]),
                seat: PlayerSeat(1),
                player_identity: SessionPeerId::from_bytes([2; 16]),
                reconnect_credential: ReconnectCredential::from_bytes([5; 32]),
            },
            ReconnectEndpointBinding::new(
                DirectEndpoint::new("127.0.0.1", 7_777).expect("loopback endpoint is valid"),
                CertificateFingerprint::from_bytes([3; 32]),
                certificate_expires_unix_seconds,
            )
            .expect("certificate expiry is valid"),
        )
    }

    #[test]
    fn reconnect_uses_the_persisted_pinned_endpoint_without_an_invite_code() {
        let mut app = intent_adapter_app(MultiplayerRole::Client);
        app.world_mut()
            .resource_mut::<MultiplayerModel>()
            .show_join_direct();
        let expires = current_unix_seconds()
            .expect("test clock is after the Unix epoch")
            .saturating_add(3_600);
        app.world_mut()
            .resource_mut::<StoredCredentialState>()
            .value = Some(stored_reconnect(expires));
        assert!(app
            .world()
            .resource::<MultiplayerDraft>()
            .join_code
            .is_empty());

        app.world_mut()
            .write_message(UiIntent::Multiplayer(MultiplayerIntent::ReconnectDirect));
        app.update();

        let queued = app
            .world()
            .get_resource::<DirectStartQueue>()
            .expect("reconnect intent should queue a pinned connection");
        let DirectStartQueue::Join {
            target,
            credential,
            reconnecting,
        } = queued
        else {
            panic!("reconnect must not queue a host endpoint");
        };
        assert!(*reconnecting);
        assert!(
            matches!(target, DirectJoinTarget::Reconnect(binding) if binding.endpoint.host() == "127.0.0.1" && binding.endpoint.port() == 7_777)
        );
        assert!(matches!(credential, AdmissionCredential::Reconnect(_)));
    }

    #[test]
    fn expired_reconnect_state_is_refused_and_removed_before_socket_start() {
        let mut app = intent_adapter_app(MultiplayerRole::Client);
        app.world_mut()
            .resource_mut::<MultiplayerModel>()
            .show_join_direct();
        app.world_mut()
            .resource_mut::<StoredCredentialState>()
            .value = Some(stored_reconnect(1));

        app.world_mut()
            .write_message(UiIntent::Multiplayer(MultiplayerIntent::ReconnectDirect));
        app.update();

        assert!(app.world().get_resource::<DirectStartQueue>().is_none());
        assert!(app
            .world()
            .resource::<StoredCredentialState>()
            .value
            .is_none());
        assert_eq!(
            app.world().resource::<SessionUiNotice>().0.as_deref(),
            Some(
                "The saved host certificate has expired. Ask the host for a fresh session invite."
            )
        );
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
    fn cancelling_a_queued_join_before_socket_start_opens_no_connection() {
        let mut app = intent_adapter_app(MultiplayerRole::Client);
        app.world_mut()
            .resource_mut::<MultiplayerModel>()
            .show_join_direct();
        let code = DirectConnectionCode {
            endpoint: DirectEndpoint::new("127.0.0.1", 7_777).expect("loopback endpoint is valid"),
            certificate_fingerprint: CertificateFingerprint::from_bytes([3; 32]),
            certificate_expires_unix_seconds: 2_000_000_000,
            invite_token: InviteToken::from_bytes([4; 16]),
        }
        .encode();
        app.world_mut().resource_mut::<MultiplayerDraft>().join_code =
            SensitiveText::new(code.expose_for_sharing());
        app.world_mut()
            .write_message(UiIntent::Multiplayer(MultiplayerIntent::JoinDirect));
        app.world_mut()
            .write_message(UiIntent::Multiplayer(MultiplayerIntent::Back));

        app.update();

        assert!(app.world().get_resource::<DirectStartQueue>().is_none());
        let model = app.world().resource::<MultiplayerModel>();
        assert_eq!(model.route, hex_gameplay_model::MultiplayerRoute::Home);
        assert_eq!(model.role, None);
        assert_eq!(
            *app.world().resource::<SimulationRole>(),
            SimulationRole::Authority
        );
    }

    #[test]
    fn pre_admission_disconnect_is_typed_only_after_transport_was_observed() {
        let mut model = MultiplayerModel::default();
        model.connecting(MultiplayerRole::Client);
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::state::app::StatesPlugin))
            .insert_state(Screen::Multiplayer)
            .insert_state(ClientState::Disconnected)
            .insert_resource(model)
            .insert_resource(SimulationRole::Replica)
            .insert_resource(SessionUiNotice::default())
            .add_systems(Update, detect_failed_client_connection);
        let connection = app.world_mut().spawn_empty().id();
        app.insert_resource(ActiveDirectSession {
            entity: connection,
            role: MultiplayerRole::Client,
            hosted_code: None,
        })
        .insert_resource(PendingClientHello {
            credential: AdmissionCredential::Invite(InviteToken::from_bytes([4; 16])),
            sent: false,
            transport_observed: false,
        });

        app.update();
        assert!(app.world().get_resource::<ActiveDirectSession>().is_some());
        app.world_mut()
            .resource_mut::<PendingClientHello>()
            .transport_observed = true;

        app.update();

        assert!(app.world().get_resource::<ActiveDirectSession>().is_none());
        assert!(app.world().get_resource::<PendingClientHello>().is_none());
        assert_eq!(
            *app.world().resource::<SimulationRole>(),
            SimulationRole::Authority
        );
        let model = app.world().resource::<MultiplayerModel>();
        assert_eq!(model.route, hex_gameplay_model::MultiplayerRoute::Ended);
        assert_eq!(
            model.ended_reason,
            Some(MultiplayerEndReason::ConnectionFailed)
        );
        assert!(app
            .world()
            .resource::<SessionUiNotice>()
            .0
            .as_deref()
            .is_some_and(|message| message.contains("before admission completed")));
    }

    #[test]
    fn closed_listen_endpoint_returns_host_to_a_typed_failure() {
        let mut model = MultiplayerModel::default();
        model.connecting(MultiplayerRole::Host);
        assert!(model.admitted(MultiplayerRole::Host, PlayerSeat::HOST));
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::state::app::StatesPlugin))
            .insert_state(Screen::Multiplayer)
            .insert_resource(model)
            .insert_resource(SimulationRole::Authority)
            .insert_resource(SessionUiNotice::default())
            .add_systems(Update, detect_failed_host_endpoint);
        let endpoint = app.world_mut().spawn_empty().id();
        assert!(app.world_mut().despawn(endpoint));
        app.insert_resource(ActiveDirectSession {
            entity: endpoint,
            role: MultiplayerRole::Host,
            hosted_code: None,
        });

        app.update();

        assert!(app.world().get_resource::<ActiveDirectSession>().is_none());
        let model = app.world().resource::<MultiplayerModel>();
        assert_eq!(model.route, hex_gameplay_model::MultiplayerRoute::Ended);
        assert_eq!(
            model.ended_reason,
            Some(MultiplayerEndReason::ConnectionFailed)
        );
        assert!(app
            .world()
            .resource::<SessionUiNotice>()
            .0
            .as_deref()
            .is_some_and(|message| message.contains("endpoint closed")));
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

    fn map_ready_report(
        role: MultiplayerRole,
        actual: PublicWorldFingerprint,
    ) -> (
        Vec<ClientMapReady>,
        Vec<HostSessionControlRequest>,
        Option<String>,
        bool,
    ) {
        let manifest = manifest();
        let mut lobby = LobbySnapshot::new(SessionPeerId::from_bytes([1; 16]), &manifest)
            .expect("manifest creates a lobby");
        lobby.phase = LobbyPhase::Loading;
        let mut model = MultiplayerModel::default();
        model.connecting(role);
        let seat = if role == MultiplayerRole::Host {
            PlayerSeat::HOST
        } else {
            PlayerSeat(1)
        };
        assert!(model.admitted(role, seat));

        let mut app = App::new();
        app.insert_resource(model)
            .insert_resource(SessionProjection {
                lobby: Some(lobby),
                manifest: Some(manifest),
            })
            .insert_resource(DirectWorldReady {
                fingerprint: actual,
            })
            .init_resource::<MapReadyReportState>()
            .init_resource::<SessionUiRequestIds>()
            .init_resource::<SessionUiNotice>()
            .add_message::<ClientMapReady>()
            .add_message::<HostSessionControlRequest>()
            .add_systems(Update, report_local_map_ready);

        app.update();

        let reports = app
            .world_mut()
            .resource_mut::<Messages<ClientMapReady>>()
            .drain()
            .collect();
        let host_reports = app
            .world_mut()
            .resource_mut::<Messages<HostSessionControlRequest>>()
            .drain()
            .collect();
        let notice = app.world().resource::<SessionUiNotice>().0.clone();
        let sent = app.world().resource::<MapReadyReportState>().sent;
        (reports, host_reports, notice, sent)
    }

    #[test]
    fn client_reports_its_actual_world_fingerprint_once_even_when_it_mismatches() {
        let expected = PublicWorldFingerprint(9);
        let (matching, host_reports, notice, sent) =
            map_ready_report(MultiplayerRole::Client, expected);
        assert_eq!(
            matching,
            vec![ClientMapReady {
                public_world_fingerprint: expected,
            }]
        );
        assert!(host_reports.is_empty());
        assert_eq!(notice, None);
        assert!(sent);

        let actual = PublicWorldFingerprint(99);
        let (mismatching, host_reports, notice, sent) =
            map_ready_report(MultiplayerRole::Client, actual);
        assert_eq!(
            mismatching,
            vec![ClientMapReady {
                public_world_fingerprint: actual,
            }]
        );
        assert!(host_reports.is_empty());
        assert_eq!(
            notice.as_deref(),
            Some("The generated world does not match the host manifest.")
        );
        assert!(sent, "the mismatch must not be reported repeatedly");
    }

    #[test]
    fn listen_host_reports_map_readiness_only_through_trusted_local_control() {
        let expected = PublicWorldFingerprint(9);

        let (client_reports, host_reports, notice, sent) =
            map_ready_report(MultiplayerRole::Host, expected);

        assert!(client_reports.is_empty());
        assert_eq!(host_reports.len(), 1);
        assert!(matches!(
            host_reports.first().map(|request| &request.action),
            Some(HostSessionAction::ReportHostMapReady {
                public_world_fingerprint
            }) if *public_world_fingerprint == expected
        ));
        assert_eq!(notice, None);
        assert!(sent);
    }
}
