//! Bevy adapters for custom authorization, command ingress, and client credentials.

use std::{fmt, time::Duration};

use aeronet::io::connection::Disconnected;
use aeronet_replicon::client::AeronetRepliconClient;
use bevy_app::{PreUpdate, Update};
use bevy_ecs::{
    event::EntityEvent as _,
    lifecycle::Remove,
    message::{MessageReader, MessageWriter},
    prelude::{
        Component, IntoScheduleConfigs as _, Message, On, Query, Res, ResMut, Resource, With,
    },
    schedule::SystemSet,
    system::Commands,
};
use bevy_replicon::prelude::{
    AuthorizedClient, ClientId, ConnectedClient, DisconnectRequest, FromClient, SendTargets,
    ServerSystems, ToClients,
};
use bevy_time::{Real, Time};
use hex_core::{CommandRequestId, GameCommand, LocalGameCommandRequest, PlayerSeat};

use crate::{
    AdmissionAccepted, AdmissionRefusal, AdmissionRefusalReason, AuthorityBoundary,
    AuthoritySequence, AuthorizedSessionClient, ClientHello, ClientLobbyAction, ClientLobbyRequest,
    ClientMapReady, CommandBegin, CommandOutcome, CommandRefusalReason, CommandResult,
    CommandSequencer, GameCommandRequest, HostSessionAction, HostSessionControlRequest,
    LobbyMutationError, LobbySnapshot, MapReadyStatus, ReconnectCredentialStorage,
    ReconnectEndpointBinding, RequestRateLimiter, SessionActivationError,
    SessionAdmissionAuthority, SessionCloseReason, SessionClosed, SessionControlOutcome,
    SessionControlRefusal, SessionControlResult, SessionManifestV1, SessionPeerId,
    StoredReconnectCredential, MAX_LIVE_SNAPSHOT_BYTES,
};

const SNAPSHOT_HEADER_VERSION: u16 = 1;
const SNAPSHOT_HEADER_BYTES: usize = LiveSnapshotHeaderV1::ENCODED_BYTES;

/// Ordering sets for the transport-neutral session adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SystemSet)]
pub enum SessionRuntimeSystems {
    /// Authenticate newly connected clients before accepting other requests.
    Admission,
    /// Apply seatless client lobby requests and trusted listen-host session controls.
    LobbyControl,
    /// Convert host-derived identities and wire/local requests into authority work.
    CommandIngress,
    /// Consume reducer outcomes and emit ordered, idempotent results.
    CommandResults,
    /// Advance real-time reservations and apply safe delegation reclamation.
    Boundaries,
    /// Persist rotating client reconnect credentials.
    CredentialStorage,
}

/// One validated request ready for the gameplay authority adapter.
///
/// `source_seat` is derived from a server-side connection component or the configured
/// local source. L2 may map a host request to a temporarily delegated canonical owner, but
/// must keep this source identity for idempotence and result routing.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedCommandRequest {
    /// Canonical authenticated human source.
    pub source_seat: PlayerSeat,
    /// Stable player identity associated with the source.
    pub player_identity: SessionPeerId,
    /// Source-scoped idempotence key.
    pub request_id: CommandRequestId,
    /// Structurally validated domain intent; legality remains reducer-owned.
    pub command: GameCommand,
}

/// Final gameplay reducer outcome returned to the L1 sequencer.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthorityCommandResolution {
    /// Authenticated source seat from [`AuthenticatedCommandRequest`].
    pub source_seat: PlayerSeat,
    /// Correlated source request id.
    pub request_id: CommandRequestId,
    /// Accepted or disclosure-safe refused result. `Duplicate` is sequencer-owned.
    pub outcome: CommandOutcome,
}

/// Default offline/listen-host identity for seatless local command ingress.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalCommandSource {
    /// Local human seat; defaults to host seat zero.
    pub seat: PlayerSeat,
    /// Stable local identity for correlation and lobby adapters.
    pub player_identity: SessionPeerId,
}

impl Default for LocalCommandSource {
    fn default() -> Self {
        Self {
            seat: PlayerSeat::HOST,
            player_identity: SessionPeerId::from_bytes([0; SessionPeerId::BYTE_LENGTH]),
        }
    }
}

/// Monotonic real-time clock used by request budgets and disconnect reservations.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SessionRuntimeClock {
    elapsed: Duration,
    reservation_remainder: Duration,
}

#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
struct RemoteSessionLifecycle {
    admitted: bool,
    received_typed_close: bool,
    transport_disconnect_pending: bool,
    session_instance_id: Option<crate::SessionInstanceId>,
}

#[derive(Resource, Debug, Default)]
struct LobbyRequestRateLimiter(RequestRateLimiter);

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
struct DisconnectAfterFlush {
    remaining_updates: u8,
}

impl SessionRuntimeClock {
    /// Real time elapsed since session runtime initialization.
    #[must_use]
    pub const fn elapsed(self) -> Duration {
        self.elapsed
    }

    /// Advances the clock and returns whole milliseconds for lobby reservation mechanics.
    pub fn advance(&mut self, delta: Duration) -> u32 {
        self.elapsed = self.elapsed.saturating_add(delta);
        self.reservation_remainder = self.reservation_remainder.saturating_add(delta);
        let whole_millis = self.reservation_remainder.as_millis();
        let bounded = whole_millis.min(u128::from(u32::MAX));
        let whole_millis = u32::try_from(bounded).unwrap_or(u32::MAX);
        self.reservation_remainder = self
            .reservation_remainder
            .saturating_sub(Duration::from_millis(u64::from(whole_millis)));
        whole_millis
    }
}

/// Disclosure-safe client storage status for session UI.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialStorageStatus {
    /// A newly issued or rotated reconnect credential was stored atomically.
    Stored,
    /// Session teardown deleted any stored reconnect credential.
    Deleted,
    /// A closure for another session left the current credential untouched.
    Preserved,
    /// The requested storage operation failed; no path or secret is disclosed.
    Failed(CredentialStorageOperation),
}

/// Operation associated with a reconnect persistence failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialStorageOperation {
    /// Atomic create/replace after admission.
    Store,
    /// Deletion after session termination.
    Delete,
}

/// Fixed bounded header parsed before allocating or deserializing a live snapshot payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveSnapshotHeaderV1 {
    /// Authority baseline represented by the snapshot.
    pub baseline_sequence: AuthoritySequence,
    /// Exact bounded payload byte count following this header.
    pub payload_bytes: u32,
}

impl LiveSnapshotHeaderV1 {
    /// Fixed encoded header size in bytes.
    pub const ENCODED_BYTES: usize = 2 + 8 + 4;

    /// Validates a payload length against the global pre-allocation cap.
    pub fn new(
        baseline_sequence: AuthoritySequence,
        payload_bytes: usize,
    ) -> Result<Self, SnapshotHeaderError> {
        if payload_bytes > MAX_LIVE_SNAPSHOT_BYTES {
            return Err(SnapshotHeaderError::PayloadTooLarge);
        }
        let payload_bytes =
            u32::try_from(payload_bytes).map_err(|_error| SnapshotHeaderError::PayloadTooLarge)?;
        Ok(Self {
            baseline_sequence,
            payload_bytes,
        })
    }

    /// Encodes the allocation header in fixed network byte order.
    #[must_use]
    pub fn encode(self) -> [u8; SNAPSHOT_HEADER_BYTES] {
        let mut bytes = [0_u8; SNAPSHOT_HEADER_BYTES];
        let mut offset = 0;
        write_field(
            &mut bytes,
            &mut offset,
            &SNAPSHOT_HEADER_VERSION.to_be_bytes(),
        );
        write_field(
            &mut bytes,
            &mut offset,
            &self.baseline_sequence.0.to_be_bytes(),
        );
        write_field(&mut bytes, &mut offset, &self.payload_bytes.to_be_bytes());
        bytes
    }
}

/// Parses one complete bounded snapshot frame without allocating its declared payload.
pub fn split_bounded_snapshot(
    frame: &[u8],
) -> Result<(LiveSnapshotHeaderV1, &[u8]), SnapshotHeaderError> {
    if frame.len() < SNAPSHOT_HEADER_BYTES {
        return Err(SnapshotHeaderError::Truncated);
    }
    let mut remaining = frame;
    let version = u16::from_be_bytes(take_array::<2>(&mut remaining)?);
    if version != SNAPSHOT_HEADER_VERSION {
        return Err(SnapshotHeaderError::WrongVersion);
    }
    let baseline_sequence = AuthoritySequence(u64::from_be_bytes(take_array::<8>(&mut remaining)?));
    let payload_bytes = u32::from_be_bytes(take_array::<4>(&mut remaining)?);
    if usize::try_from(payload_bytes).map_or(true, |length| length > MAX_LIVE_SNAPSHOT_BYTES) {
        return Err(SnapshotHeaderError::PayloadTooLarge);
    }
    if remaining.len() != usize::try_from(payload_bytes).unwrap_or(usize::MAX) {
        return Err(SnapshotHeaderError::LengthMismatch);
    }
    Ok((
        LiveSnapshotHeaderV1 {
            baseline_sequence,
            payload_bytes,
        },
        remaining,
    ))
}

fn write_field<const FIELD: usize>(
    target: &mut [u8; SNAPSHOT_HEADER_BYTES],
    offset: &mut usize,
    field: &[u8; FIELD],
) {
    let end = offset.saturating_add(FIELD);
    if let Some(slot) = target.get_mut(*offset..end) {
        slot.copy_from_slice(field);
        *offset = end;
    }
}

fn take_array<const LENGTH: usize>(bytes: &mut &[u8]) -> Result<[u8; LENGTH], SnapshotHeaderError> {
    let (head, tail) = bytes
        .split_at_checked(LENGTH)
        .ok_or(SnapshotHeaderError::Truncated)?;
    *bytes = tail;
    head.try_into()
        .map_err(|_error| SnapshotHeaderError::Truncated)
}

/// Why a live snapshot frame was rejected before payload allocation/deserialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotHeaderError {
    /// Header bytes ended early.
    Truncated,
    /// The header version is unsupported.
    WrongVersion,
    /// The declared payload exceeds the defensive cap.
    PayloadTooLarge,
    /// Actual frame bytes disagree with the declared bounded length.
    LengthMismatch,
}

impl fmt::Display for SnapshotHeaderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Truncated => "live snapshot header is truncated",
            Self::WrongVersion => "live snapshot header version is unsupported",
            Self::PayloadTooLarge => "live snapshot payload exceeds its allocation cap",
            Self::LengthMismatch => "live snapshot payload length disagrees with its header",
        })
    }
}

impl std::error::Error for SnapshotHeaderError {}

pub(crate) fn install_runtime(app: &mut bevy_app::App) {
    app.init_resource::<CommandSequencer>()
        .init_resource::<RequestRateLimiter>()
        .init_resource::<LobbyRequestRateLimiter>()
        .init_resource::<AuthorityBoundary>()
        .init_resource::<LocalCommandSource>()
        .init_resource::<SessionRuntimeClock>()
        .init_resource::<RemoteSessionLifecycle>()
        .add_message::<AuthenticatedCommandRequest>()
        .add_message::<AuthorityCommandResolution>()
        .add_message::<HostSessionControlRequest>()
        .add_message::<CredentialStorageStatus>()
        .configure_sets(
            PreUpdate,
            (
                SessionRuntimeSystems::Admission,
                SessionRuntimeSystems::LobbyControl,
                SessionRuntimeSystems::CommandIngress,
                SessionRuntimeSystems::CommandResults,
                SessionRuntimeSystems::CredentialStorage,
            )
                .chain()
                .after(ServerSystems::Receive),
        )
        .add_systems(
            PreUpdate,
            (
                handle_client_hello.in_set(SessionRuntimeSystems::Admission),
                handle_client_map_ready.in_set(SessionRuntimeSystems::Admission),
                (handle_remote_lobby_requests, handle_host_session_controls)
                    .chain()
                    .in_set(SessionRuntimeSystems::LobbyControl),
                handle_remote_commands.in_set(SessionRuntimeSystems::CommandIngress),
                handle_local_commands.in_set(SessionRuntimeSystems::CommandIngress),
                handle_authority_results.in_set(SessionRuntimeSystems::CommandResults),
                persist_accepted_credential.in_set(SessionRuntimeSystems::CredentialStorage),
                delete_closed_credential.in_set(SessionRuntimeSystems::CredentialStorage),
                track_remote_session_messages.in_set(SessionRuntimeSystems::CredentialStorage),
            ),
        )
        .add_systems(
            Update,
            (
                advance_runtime_clock,
                apply_safe_reclaims,
                emit_pending_host_disconnect,
                flush_typed_disconnects,
            )
                .chain()
                .in_set(SessionRuntimeSystems::Boundaries),
        )
        .add_observer(on_connected_client_removed)
        .add_observer(on_remote_transport_disconnected);
}

fn handle_client_hello(
    mut hellos: MessageReader<FromClient<ClientHello>>,
    mut authority: Option<ResMut<SessionAdmissionAuthority>>,
    connected: Query<(), With<ConnectedClient>>,
    mut commands: Commands,
    mut accepted: MessageWriter<ToClients<AdmissionAccepted>>,
    mut refused: MessageWriter<ToClients<AdmissionRefusal>>,
    mut manifests: MessageWriter<ToClients<SessionManifestV1>>,
    mut lobbies: MessageWriter<ToClients<LobbySnapshot>>,
) {
    let Some(authority) = authority.as_mut() else {
        return;
    };
    for hello in hellos.read() {
        let Some(connection) = hello.client_id.entity() else {
            continue;
        };
        if connected.get(connection).is_err() {
            continue;
        }
        match authority.admit(connection, hello) {
            Ok(grant) => {
                commands
                    .entity(connection)
                    .insert((AuthorizedClient, grant.client));
                accepted.write(ToClients {
                    targets: SendTargets::Single(hello.client_id),
                    message: grant.accepted,
                });
                manifests.write(ToClients {
                    targets: SendTargets::Single(hello.client_id),
                    message: authority.manifest().clone(),
                });
                lobbies.write(ToClients {
                    targets: SendTargets::All,
                    message: authority.lobby().snapshot_owned(),
                });
            }
            Err(reason) => refuse_and_disconnect(
                hello.client_id,
                connection,
                reason,
                &mut refused,
                &mut commands,
            ),
        }
    }
}

fn handle_client_map_ready(
    mut reports: MessageReader<FromClient<ClientMapReady>>,
    mut authority: Option<ResMut<SessionAdmissionAuthority>>,
    mut lobbies: MessageWriter<ToClients<LobbySnapshot>>,
    mut closed: MessageWriter<ToClients<SessionClosed>>,
    mut commands: Commands,
) {
    let Some(authority) = authority.as_mut() else {
        return;
    };
    for report in reports.read() {
        let Some(connection) = report.client_id.entity() else {
            continue;
        };
        match authority.report_map_ready(connection, report.public_world_fingerprint) {
            Ok(MapReadyStatus::Waiting) => {}
            Ok(MapReadyStatus::Activated) => {
                lobbies.write(ToClients {
                    targets: SendTargets::All,
                    message: authority.lobby().snapshot_owned(),
                });
            }
            Err(error) => {
                let reason = match error {
                    SessionActivationError::MapMismatch => SessionCloseReason::MapMismatch,
                    SessionActivationError::NotAuthorized
                    | SessionActivationError::WrongPhase
                    | SessionActivationError::Lobby(_) => SessionCloseReason::ProtocolViolation,
                };
                closed.write(ToClients {
                    targets: SendTargets::Single(report.client_id),
                    message: SessionClosed {
                        session_instance_id: authority.manifest().session_instance_id,
                        reason,
                    },
                });
                schedule_disconnect_after_flush(connection, &mut commands);
            }
        }
    }
}

fn handle_remote_lobby_requests(
    mut requests: MessageReader<FromClient<ClientLobbyRequest>>,
    clients: Query<&AuthorizedSessionClient, With<AuthorizedClient>>,
    mut authority: Option<ResMut<SessionAdmissionAuthority>>,
    clock: Res<SessionRuntimeClock>,
    mut limiter: ResMut<LobbyRequestRateLimiter>,
    mut results: MessageWriter<ToClients<SessionControlResult>>,
    mut lobbies: MessageWriter<ToClients<LobbySnapshot>>,
    mut closed: MessageWriter<ToClients<SessionClosed>>,
    mut commands: Commands,
) {
    for request in requests.read() {
        let Some(connection) = request.client_id.entity() else {
            continue;
        };
        let Ok(client) = clients.get(connection) else {
            send_control_result(
                request.client_id,
                request.request_id,
                SessionControlOutcome::Refused(SessionControlRefusal::NotAuthorized),
                &mut results,
            );
            schedule_disconnect_after_flush(connection, &mut commands);
            continue;
        };
        let Some(authority) = authority.as_mut() else {
            send_control_result(
                request.client_id,
                request.request_id,
                SessionControlOutcome::Refused(SessionControlRefusal::NotAuthorized),
                &mut results,
            );
            continue;
        };

        let outcome = match request.action {
            ClientLobbyAction::SetReady(ready) => {
                if limiter.0.allow(client.seat, clock.elapsed()).is_err() {
                    Err(SessionControlRefusal::RateLimited)
                } else {
                    authority
                        .lobby_mut()
                        .set_ready(client.seat, ready)
                        .map_err(map_lobby_control_error)
                }
            }
            ClientLobbyAction::Leave => authority
                .leave(connection)
                .map(|_seat| ())
                .map_err(map_lobby_control_error),
        };
        let outcome = match outcome {
            Ok(()) => {
                lobbies.write(ToClients {
                    targets: SendTargets::All,
                    message: authority.lobby().snapshot_owned(),
                });
                if request.action == ClientLobbyAction::Leave {
                    closed.write(ToClients {
                        targets: SendTargets::Single(request.client_id),
                        message: SessionClosed {
                            session_instance_id: authority.manifest().session_instance_id,
                            reason: SessionCloseReason::SessionEnded,
                        },
                    });
                    schedule_disconnect_after_flush(connection, &mut commands);
                }
                SessionControlOutcome::Accepted
            }
            Err(reason) => SessionControlOutcome::Refused(reason),
        };
        send_control_result(request.client_id, request.request_id, outcome, &mut results);
    }
}

fn handle_host_session_controls(
    mut requests: MessageReader<HostSessionControlRequest>,
    mut authority: Option<ResMut<SessionAdmissionAuthority>>,
    mut local_results: MessageWriter<SessionControlResult>,
    mut lobbies: MessageWriter<ToClients<LobbySnapshot>>,
    mut closed: MessageWriter<ToClients<SessionClosed>>,
    mut commands: Commands,
) {
    for request in requests.read() {
        let Some(authority) = authority.as_mut() else {
            local_results.write(SessionControlResult {
                request_id: request.request_id,
                outcome: SessionControlOutcome::Refused(SessionControlRefusal::NotAuthorized),
            });
            continue;
        };

        let mut publish_lobby = true;
        let outcome = match request.action {
            HostSessionAction::AssignUnit { unit, destination } => authority
                .lobby_mut()
                .assign_unit(unit, destination)
                .map_err(map_lobby_control_error),
            HostSessionAction::Kick { seat } => authority
                .kick(seat)
                .map(|connection| {
                    if let Some(connection) = connection {
                        closed.write(ToClients {
                            targets: SendTargets::Single(ClientId::Client(connection)),
                            message: SessionClosed {
                                session_instance_id: authority.manifest().session_instance_id,
                                reason: SessionCloseReason::Kicked,
                            },
                        });
                        schedule_disconnect_after_flush(connection, &mut commands);
                    }
                })
                .map_err(map_lobby_control_error),
            HostSessionAction::BeginLoading {
                public_world_fingerprint,
            } => authority
                .begin_loading(public_world_fingerprint)
                .map(|_snapshot| ())
                .map_err(map_activation_control_error),
            HostSessionAction::EnterOutcome => authority
                .enter_outcome()
                .map(|_snapshot| ())
                .map_err(map_lobby_control_error),
            HostSessionAction::RetryExact {
                public_world_fingerprint,
            } => authority
                .retry_loading(public_world_fingerprint)
                .map(|_snapshot| ())
                .map_err(map_activation_control_error),
            HostSessionAction::ReturnToLobby => authority
                .return_to_lobby()
                .map(|_snapshot| ())
                .map_err(map_lobby_control_error),
            HostSessionAction::CloseSession => {
                let connections = authority.connected_peers();
                closed.write(ToClients {
                    targets: SendTargets::CLIENTS_ONLY,
                    message: SessionClosed {
                        session_instance_id: authority.manifest().session_instance_id,
                        reason: SessionCloseReason::HostClosed,
                    },
                });
                authority.close();
                for (_seat, connection) in connections {
                    schedule_disconnect_after_flush(connection, &mut commands);
                }
                publish_lobby = false;
                Ok(())
            }
        };

        let outcome = match outcome {
            Ok(()) => {
                if publish_lobby {
                    lobbies.write(ToClients {
                        targets: SendTargets::All,
                        message: authority.lobby().snapshot_owned(),
                    });
                }
                SessionControlOutcome::Accepted
            }
            Err(reason) => SessionControlOutcome::Refused(reason),
        };
        local_results.write(SessionControlResult {
            request_id: request.request_id,
            outcome,
        });
    }
}

fn map_activation_control_error(error: SessionActivationError) -> SessionControlRefusal {
    match error {
        SessionActivationError::MapMismatch => SessionControlRefusal::MapMismatch,
        SessionActivationError::WrongPhase => SessionControlRefusal::WrongPhase,
        SessionActivationError::NotAuthorized => SessionControlRefusal::NotAuthorized,
        SessionActivationError::Lobby(error) => map_lobby_control_error(error),
    }
}

fn map_lobby_control_error(error: LobbyMutationError) -> SessionControlRefusal {
    match error {
        LobbyMutationError::LobbyClosed => SessionControlRefusal::LobbyClosed,
        LobbyMutationError::WrongPhase => SessionControlRefusal::WrongPhase,
        LobbyMutationError::NonHumanSeat
        | LobbyMutationError::HostReadinessIsImplicit
        | LobbyMutationError::HostCannotDisconnect
        | LobbyMutationError::HostCannotBeRemoved => SessionControlRefusal::InvalidSeat,
        LobbyMutationError::VacantDestination
        | LobbyMutationError::SeatNotConnected
        | LobbyMutationError::VacantSeat
        | LobbyMutationError::DuplicateActiveSeat => SessionControlRefusal::SeatUnavailable,
        LobbyMutationError::WouldEmptyClaimedSeat => SessionControlRefusal::WouldEmptySeat,
        LobbyMutationError::LobbyFull => SessionControlRefusal::LobbyFull,
        LobbyMutationError::InvalidManifest
        | LobbyMutationError::InvalidLobby(_)
        | LobbyMutationError::Bound(_)
        | LobbyMutationError::MissingHost
        | LobbyMutationError::DuplicatePlayerIdentity
        | LobbyMutationError::UnknownAssignedUnit => SessionControlRefusal::InvalidLobby,
    }
}

fn handle_remote_commands(
    mut requests: MessageReader<FromClient<GameCommandRequest>>,
    clients: Query<&AuthorizedSessionClient, With<AuthorizedClient>>,
    authority: Option<Res<SessionAdmissionAuthority>>,
    clock: Res<SessionRuntimeClock>,
    mut limiter: ResMut<RequestRateLimiter>,
    mut sequencer: ResMut<CommandSequencer>,
    mut boundary: ResMut<AuthorityBoundary>,
    mut authenticated: MessageWriter<AuthenticatedCommandRequest>,
    mut results: MessageWriter<ToClients<CommandResult>>,
    mut refused: MessageWriter<ToClients<AdmissionRefusal>>,
    mut commands: Commands,
) {
    for request in requests.read() {
        let Some(connection) = request.client_id.entity() else {
            continue;
        };
        let Ok(client) = clients.get(connection) else {
            refuse_and_disconnect(
                request.client_id,
                connection,
                AdmissionRefusalReason::Malformed,
                &mut refused,
                &mut commands,
            );
            continue;
        };
        let begin = sequencer.begin(client.seat, request.request_id);
        match begin {
            Ok(CommandBegin::Duplicate(result)) => {
                send_result(request.client_id, result, &mut results);
            }
            Ok(CommandBegin::AlreadyInFlight) => {}
            Ok(CommandBegin::Enqueue) => {
                let refusal = if authority.as_ref().is_none() {
                    Some(CommandRefusalReason::NotAuthorized)
                } else if authority.as_ref().is_some_and(|authority| {
                    !authority.lobby().player_can_issue_commands(client.seat)
                }) {
                    Some(CommandRefusalReason::Busy)
                } else if limiter.allow(client.seat, clock.elapsed()).is_err() {
                    Some(CommandRefusalReason::RateLimited)
                } else if request.validate().is_err() {
                    Some(CommandRefusalReason::Malformed)
                } else {
                    None
                };
                if let Some(reason) = refusal {
                    if let Ok(result) = sequencer.finish(
                        client.seat,
                        request.request_id,
                        CommandOutcome::Refused(reason),
                    ) {
                        send_result(request.client_id, result, &mut results);
                    }
                    continue;
                }
                boundary.begin_command();
                authenticated.write(AuthenticatedCommandRequest {
                    source_seat: client.seat,
                    player_identity: client.player_identity,
                    request_id: request.request_id,
                    command: request.command.clone(),
                });
            }
            Err(_) => {}
        }
    }
}

fn handle_local_commands(
    mut requests: MessageReader<LocalGameCommandRequest>,
    source: Res<LocalCommandSource>,
    mut sequencer: ResMut<CommandSequencer>,
    mut boundary: ResMut<AuthorityBoundary>,
    mut authenticated: MessageWriter<AuthenticatedCommandRequest>,
    mut results: MessageWriter<ToClients<CommandResult>>,
) {
    for request in requests.read() {
        match sequencer.begin(source.seat, request.request_id) {
            Ok(CommandBegin::Duplicate(result)) => {
                send_result(ClientId::Server, result, &mut results);
            }
            Ok(CommandBegin::AlreadyInFlight) => {}
            Ok(CommandBegin::Enqueue) => {
                let wire_request = GameCommandRequest {
                    request_id: request.request_id,
                    command: request.command.clone(),
                };
                let refusal = if wire_request.validate().is_err() {
                    Some(CommandRefusalReason::Malformed)
                } else {
                    None
                };
                if let Some(reason) = refusal {
                    if let Ok(result) = sequencer.finish(
                        source.seat,
                        request.request_id,
                        CommandOutcome::Refused(reason),
                    ) {
                        send_result(ClientId::Server, result, &mut results);
                    }
                    continue;
                }
                boundary.begin_command();
                authenticated.write(AuthenticatedCommandRequest {
                    source_seat: source.seat,
                    player_identity: source.player_identity,
                    request_id: request.request_id,
                    command: request.command.clone(),
                });
            }
            Err(_) => {}
        }
    }
}

fn handle_authority_results(
    mut resolutions: MessageReader<AuthorityCommandResolution>,
    mut sequencer: ResMut<CommandSequencer>,
    mut boundary: ResMut<AuthorityBoundary>,
    authority: Option<Res<SessionAdmissionAuthority>>,
    mut results: MessageWriter<ToClients<CommandResult>>,
) {
    for resolution in resolutions.read() {
        let result = sequencer.finish(
            resolution.source_seat,
            resolution.request_id,
            resolution.outcome,
        );
        if result.is_ok() {
            let _finish_result = boundary.finish_command();
        }
        let Ok(result) = result else {
            continue;
        };
        let target = if resolution.source_seat == PlayerSeat::HOST {
            Some(ClientId::Server)
        } else {
            authority
                .as_ref()
                .and_then(|authority| authority.active_connection(resolution.source_seat))
                .map(ClientId::Client)
        };
        if let Some(target) = target {
            send_result(target, result, &mut results);
        }
    }
}

fn advance_runtime_clock(
    time: Option<Res<Time<Real>>>,
    mut clock: ResMut<SessionRuntimeClock>,
    mut authority: Option<ResMut<SessionAdmissionAuthority>>,
    mut lobbies: MessageWriter<ToClients<LobbySnapshot>>,
) {
    let Some(time) = time else {
        return;
    };
    let elapsed_millis = clock.advance(time.delta());
    if elapsed_millis == 0 {
        return;
    }
    let Some(authority) = authority.as_mut() else {
        return;
    };
    let before = authority.lobby().snapshot_owned();
    authority.lobby_mut().advance_reservations(elapsed_millis);
    if authority.lobby().snapshot() != &before {
        lobbies.write(ToClients {
            targets: SendTargets::All,
            message: authority.lobby().snapshot_owned(),
        });
    }
}

fn apply_safe_reclaims(
    boundary: Res<AuthorityBoundary>,
    mut authority: Option<ResMut<SessionAdmissionAuthority>>,
    mut lobbies: MessageWriter<ToClients<LobbySnapshot>>,
) {
    let Some(authority) = authority.as_mut() else {
        return;
    };
    if authority
        .lobby_mut()
        .apply_safe_reclaims(boundary.is_quiescent())
        > 0
    {
        lobbies.write(ToClients {
            targets: SendTargets::All,
            message: authority.lobby().snapshot_owned(),
        });
    }
}

fn on_connected_client_removed(
    trigger: On<Remove, ConnectedClient>,
    mut authority: Option<ResMut<SessionAdmissionAuthority>>,
    mut lobbies: MessageWriter<ToClients<LobbySnapshot>>,
) {
    let Some(authority) = authority.as_mut() else {
        return;
    };
    if authority.disconnect(trigger.event_target()).is_some() {
        lobbies.write(ToClients {
            targets: SendTargets::All,
            message: authority.lobby().snapshot_owned(),
        });
    }
}

fn persist_accepted_credential(
    mut accepted: MessageReader<AdmissionAccepted>,
    storage: Option<Res<ReconnectCredentialStorage>>,
    endpoint_binding: Option<Res<ReconnectEndpointBinding>>,
    mut status: MessageWriter<CredentialStorageStatus>,
) {
    let Some(storage) = storage else {
        return;
    };
    for accepted in accepted.read() {
        let Some(endpoint_binding) = endpoint_binding.as_ref() else {
            status.write(CredentialStorageStatus::Failed(
                CredentialStorageOperation::Store,
            ));
            continue;
        };
        let outcome = storage
            .store()
            .store_atomically(StoredReconnectCredential::new(
                *accepted,
                (**endpoint_binding).clone(),
            ));
        status.write(if outcome.is_ok() {
            CredentialStorageStatus::Stored
        } else {
            CredentialStorageStatus::Failed(CredentialStorageOperation::Store)
        });
    }
}

fn delete_closed_credential(
    mut closed: MessageReader<SessionClosed>,
    storage: Option<Res<ReconnectCredentialStorage>>,
    mut status: MessageWriter<CredentialStorageStatus>,
) {
    let Some(storage) = storage else {
        return;
    };
    for closed in closed.read() {
        let outcome = storage
            .store()
            .delete_if_session(closed.session_instance_id);
        status.write(match outcome {
            Ok(true) => CredentialStorageStatus::Deleted,
            Ok(false) => CredentialStorageStatus::Preserved,
            Err(_error) => CredentialStorageStatus::Failed(CredentialStorageOperation::Delete),
        });
    }
}

fn track_remote_session_messages(
    mut accepted: MessageReader<AdmissionAccepted>,
    mut closed: MessageReader<SessionClosed>,
    mut lifecycle: ResMut<RemoteSessionLifecycle>,
) {
    if let Some(accepted) = accepted.read().last() {
        lifecycle.admitted = true;
        lifecycle.received_typed_close = false;
        lifecycle.transport_disconnect_pending = false;
        lifecycle.session_instance_id = Some(accepted.session_instance_id);
    }
    if let Some(closed) = closed.read().last() {
        lifecycle.admitted = false;
        lifecycle.received_typed_close = true;
        lifecycle.transport_disconnect_pending = false;
        if lifecycle.session_instance_id == Some(closed.session_instance_id) {
            lifecycle.session_instance_id = None;
        }
    }
}

fn on_remote_transport_disconnected(
    trigger: On<Disconnected>,
    clients: Query<(), With<AeronetRepliconClient>>,
    mut lifecycle: ResMut<RemoteSessionLifecycle>,
) {
    if clients.get(trigger.event_target()).is_ok() && lifecycle.admitted {
        lifecycle.transport_disconnect_pending = true;
    }
}

fn emit_pending_host_disconnect(
    mut lifecycle: ResMut<RemoteSessionLifecycle>,
    mut closed: MessageWriter<SessionClosed>,
) {
    if !lifecycle.transport_disconnect_pending {
        return;
    }
    lifecycle.transport_disconnect_pending = false;
    lifecycle.admitted = false;
    if !lifecycle.received_typed_close {
        let Some(session_instance_id) = lifecycle.session_instance_id.take() else {
            return;
        };
        closed.write(SessionClosed {
            session_instance_id,
            reason: SessionCloseReason::HostDisconnected,
        });
    }
}

fn flush_typed_disconnects(
    mut pending: Query<(bevy_ecs::prelude::Entity, &mut DisconnectAfterFlush)>,
    mut disconnects: MessageWriter<DisconnectRequest>,
) {
    for (connection, mut pending) in &mut pending {
        if pending.remaining_updates > 0 {
            pending.remaining_updates = pending.remaining_updates.saturating_sub(1);
            continue;
        }
        disconnects.write(DisconnectRequest { client: connection });
    }
}

fn refuse_and_disconnect(
    client_id: ClientId,
    connection: bevy_ecs::prelude::Entity,
    reason: AdmissionRefusalReason,
    refused: &mut MessageWriter<ToClients<AdmissionRefusal>>,
    commands: &mut Commands,
) {
    refused.write(ToClients {
        targets: SendTargets::Single(client_id),
        message: AdmissionRefusal { reason },
    });
    schedule_disconnect_after_flush(connection, commands);
}

fn schedule_disconnect_after_flush(connection: bevy_ecs::prelude::Entity, commands: &mut Commands) {
    commands
        .entity(connection)
        .remove::<(AuthorizedClient, AuthorizedSessionClient)>()
        .insert(DisconnectAfterFlush {
            remaining_updates: 1,
        });
}

fn send_result(
    target: ClientId,
    result: CommandResult,
    results: &mut MessageWriter<ToClients<CommandResult>>,
) {
    results.write(ToClients {
        targets: SendTargets::Single(target),
        message: result,
    });
}

fn send_control_result(
    target: ClientId,
    request_id: CommandRequestId,
    outcome: SessionControlOutcome,
    results: &mut MessageWriter<ToClients<SessionControlResult>>,
) {
    results.write(ToClients {
        targets: SendTargets::Single(target),
        message: SessionControlResult {
            request_id,
            outcome,
        },
    });
}

#[cfg(test)]
mod tests {
    use bevy_app::App;
    use bevy_ecs::message::Messages;
    use bevy_replicon::prelude::ProtocolHash;
    use bevy_state::app::StatesPlugin;
    use bevy_time::TimePlugin;
    use hex_core::{GameCommand, UnitId};

    use super::*;
    use crate::MultiplayerPlugin;

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins((TimePlugin, StatesPlugin, MultiplayerPlugin));
        app.finish();
        app.cleanup();
        app
    }

    #[test]
    fn local_requests_use_the_same_sequencer_and_publish_one_authority_ingress() {
        let mut app = app();
        app.world_mut().write_message(LocalGameCommandRequest {
            request_id: CommandRequestId(8),
            command: GameCommand::Rest { unit: UnitId(1) },
        });
        app.update();
        let messages = app
            .world()
            .resource::<Messages<AuthenticatedCommandRequest>>();
        let mut cursor = messages.get_cursor();
        let ingress = cursor
            .read(messages)
            .next()
            .expect("local request should enter authority once");
        assert_eq!(ingress.source_seat, PlayerSeat::HOST);
        assert_eq!(ingress.request_id, CommandRequestId(8));

        app.world_mut().write_message(LocalGameCommandRequest {
            request_id: CommandRequestId(8),
            command: GameCommand::Rest { unit: UnitId(1) },
        });
        app.update();
        let messages = app
            .world()
            .resource::<Messages<AuthenticatedCommandRequest>>();
        let mut cursor = messages.get_cursor();
        assert_eq!(
            cursor
                .read(messages)
                .filter(|request| request.request_id == CommandRequestId(8))
                .count(),
            1,
            "retry while the first request is in flight must not re-enqueue"
        );
    }

    #[test]
    fn local_final_retry_returns_duplicate_without_reapplying() {
        let mut app = app();
        app.world_mut().write_message(LocalGameCommandRequest {
            request_id: CommandRequestId(3),
            command: GameCommand::Rest { unit: UnitId(1) },
        });
        app.update();
        app.world_mut().write_message(AuthorityCommandResolution {
            source_seat: PlayerSeat::HOST,
            request_id: CommandRequestId(3),
            outcome: CommandOutcome::Accepted,
        });
        app.update();
        app.world_mut().write_message(LocalGameCommandRequest {
            request_id: CommandRequestId(3),
            command: GameCommand::Rest { unit: UnitId(1) },
        });
        app.update();

        let received = app.world().resource::<Messages<CommandResult>>();
        let mut cursor = received.get_cursor();
        assert!(cursor.read(received).any(|result| matches!(
            result.outcome,
            CommandOutcome::Duplicate {
                original_sequence: AuthoritySequence(1)
            }
        )));
        assert_eq!(
            app.world().resource::<CommandSequencer>().last_sequence(),
            AuthoritySequence(2)
        );
    }

    #[test]
    fn snapshot_header_rejects_arbitrary_and_oversized_frames_before_payload_use() {
        for length in 0..64 {
            let frame = vec![u8::try_from(length).unwrap_or(u8::MAX); length];
            let _result = split_bounded_snapshot(&frame);
        }
        let header =
            LiveSnapshotHeaderV1::new(AuthoritySequence(9), 3).expect("small snapshot header");
        let mut frame = header.encode().to_vec();
        frame.extend_from_slice(&[1, 2, 3]);
        assert_eq!(
            split_bounded_snapshot(&frame),
            Ok((header, [1, 2, 3].as_slice()))
        );

        let mut oversized = LiveSnapshotHeaderV1 {
            baseline_sequence: AuthoritySequence(0),
            payload_bytes: u32::try_from(MAX_LIVE_SNAPSHOT_BYTES)
                .unwrap_or(u32::MAX)
                .saturating_add(1),
        }
        .encode();
        if let Some(version) = oversized.get_mut(0..2) {
            version.copy_from_slice(&SNAPSHOT_HEADER_VERSION.to_be_bytes());
        }
        assert_eq!(
            split_bounded_snapshot(&oversized),
            Err(SnapshotHeaderError::PayloadTooLarge)
        );
    }

    #[test]
    fn protocol_hash_resource_remains_available_after_runtime_installation() {
        let app = app();
        assert!(app.world().contains_resource::<ProtocolHash>());
    }
}
