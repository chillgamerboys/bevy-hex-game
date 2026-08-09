//! Shared multiplayer protocol and session vocabulary.
//!
//! This crate is shared infrastructure: it depends on stable domain types but never
//! queries private map, unit, combat, or perception implementations. The authority-owning
//! crates export projections into these contracts; remote clients apply them through
//! adapters in the composition root.
//!
//! Version 1 provides a server-authoritative listen-host foundation for Direct Connect.
//! Merely installing [`MultiplayerPlugin`] does not open a socket. Steam will later supply
//! another connection/lobby backend without changing the protocol types in this crate.

mod auth;
mod connection_code;
mod control;
#[cfg(feature = "direct")]
mod direct;
mod limits;
mod lobby;
mod manifest;
mod plugin;
mod protocol;
mod replica;
mod runtime;
mod secret;
mod sequence;
#[cfg(feature = "test-harness")]
mod testing;

pub use auth::{
    AdmissionGrant, AdmissionSetupError, AtomicFileReconnectCredentialStore,
    AuthorizedSessionClient, CredentialStoreError, MapReadyStatus, MemoryReconnectCredentialStore,
    ReconnectCredentialStorage, ReconnectCredentialStore, SessionActivationError,
    SessionAdmissionAuthority, StoredReconnectCredential,
};
pub use connection_code::{
    CertificateFingerprint, ConnectionCodeError, DirectConnectionCode, DirectEndpoint,
    EncodedConnectionCode,
};
pub use control::{
    ClientLobbyAction, ClientLobbyRequest, HostSessionAction, HostSessionControlRequest,
    SessionControlOutcome, SessionControlRefusal, SessionControlResult,
};
#[cfg(feature = "direct")]
pub use direct::{
    DirectTransportError, PreparedDirectHost, PreparedDirectJoin, SpkiPinVerifier,
    DEFAULT_DIRECT_PORT, DIRECT_SESSION_PATH,
};
pub use limits::{
    BoundError, BoundedText, BoundedVec, MAX_ABS_COMMAND_COORDINATE, MAX_ABS_COMMAND_LEVEL,
    MAX_ABS_LATTICE_COORDINATE, MAX_ADVERTISED_HOST_BYTES, MAX_BUILD_IDENTITY_BYTES,
    MAX_COMMAND_BYTES, MAX_CONNECTION_CODE_BYTES, MAX_DECISION_CELLS, MAX_IDENTITY_BYTES,
    MAX_LIVE_SNAPSHOT_BYTES, MAX_PARTY_MEMBERS, MAX_ROUTE_STEPS, MAX_SESSION_UNITS,
    MAX_UNIT_EFFECTS,
};
pub use lobby::{
    LaunchSummaryV1, LobbyAuthority, LobbyMutationError, LobbyPhase, LobbySeatSnapshot,
    LobbySnapshot, LobbyValidationError, SeatConnectionState, SessionPeerId,
};
pub use manifest::{
    BuildIdentityV1, ContentFingerprint, ManifestValidationError, MapManifestV1, ProtocolVersion,
    PublicWorldFingerprint, RosterEntryV1, RulesManifestV1, SessionManifestV1, UnitDeploymentV1,
    SESSION_PROTOCOL_VERSION,
};
pub use plugin::MultiplayerPlugin;
pub use protocol::{
    register_protocol, AdmissionAccepted, AdmissionCredential, AdmissionRefusal,
    AdmissionRefusalReason, AuthoritySequence, ClientHello, ClientMapReady, CommandOutcome,
    CommandRefusalReason, CommandResult, CommandWireError, GameCommandRequest, SessionCloseReason,
    SessionClosed, PROTOCOL_SCHEMA_TAG,
};
pub use replica::{
    MotionReplicaV1, ReplicaValidationError, SessionOutcome, SessionReplica, UnitReplica,
};
pub use runtime::{
    split_bounded_snapshot, AuthenticatedCommandRequest, AuthorityCommandResolution,
    CredentialStorageOperation, CredentialStorageStatus, LiveSnapshotHeaderV1, LocalCommandSource,
    SessionRuntimeClock, SessionRuntimeSystems, SnapshotHeaderError,
};
pub use secret::{InviteToken, ReconnectCredential};
pub use sequence::{
    AuthorityBoundary, BoundaryError, CommandBegin, CommandSequencer, RateLimitError,
    RequestRateLimiter, SequencerError, DEFAULT_REQUEST_BURST, DEFAULT_REQUEST_WINDOW,
    MAX_CACHED_RESULTS_PER_SEAT, MAX_IN_FLIGHT_REQUESTS_PER_SEAT,
};
#[cfg(feature = "test-harness")]
pub use testing::{ChannelSessionHarness, ClientProbe, HarnessError, HostProbe};
