//! Shared multiplayer protocol and session vocabulary.
//!
//! This crate is shared infrastructure: it depends on stable domain types but never
//! queries private map, unit, combat, or perception implementations. The authority-owning
//! crates export projections into these contracts; remote clients apply them through
//! adapters in the composition root.
//!
//! Version 1 provides a server-authoritative listen-host foundation for Direct Connect.
//! Merely installing [`MultiplayerPlugin`] does not open a socket. EOS will later provide
//! the universal Internet lobby/P2P path through `hex_online`; Steam remains an optional
//! identity and native-invitation adapter. Neither changes the gameplay protocol here.

mod auth;
mod campaign;
mod connection_code;
mod control;
#[cfg(feature = "direct")]
mod direct;
mod limits;
mod lobby;
mod manifest;
mod online;
mod plugin;
mod protocol;
mod replica;
mod runtime;
mod secret;
mod sequence;
mod snapshot;
#[cfg(feature = "test-harness")]
mod testing;

pub use auth::{
    AdmissionGrant, AdmissionSetupError, AtomicFileReconnectCredentialStore,
    AuthorizedSessionClient, CredentialStoreError, MapReadyStatus, MemoryReconnectCredentialStore,
    ReconnectCredentialStorage, ReconnectCredentialStore, ReconnectEndpointBinding,
    SessionActivationError, SessionAdmissionAuthority, StoredReconnectCredential,
};
pub use campaign::{
    CampaignEffectCheckpointV2, CampaignEffectLedgerV2, CampaignSaveRefusalV2, CampaignSaveStateV2,
    CampaignSaveStatusV2, CampaignUnitCheckpointV2, CampaignValidationError,
    HostCampaignCheckpointV2, CAMPAIGN_CHECKPOINT_VERSION_V2, MAX_CAMPAIGN_EFFECTS,
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
    DirectTransportError, PreparedDirectHost, PreparedDirectJoin, PreparedDirectReconnect,
    SpkiPinVerifier, DEFAULT_DIRECT_PORT, DIRECT_SESSION_PATH,
};
pub use limits::{
    BoundError, BoundedText, BoundedVec, MAX_ABS_COMMAND_COORDINATE, MAX_ABS_COMMAND_LEVEL,
    MAX_ABS_LATTICE_COORDINATE, MAX_ADVERTISED_HOST_BYTES, MAX_BUILD_IDENTITY_BYTES,
    MAX_COMMAND_BYTES, MAX_CONNECTION_CODE_BYTES, MAX_DECISION_CELLS, MAX_IDENTITY_BYTES,
    MAX_LIVE_SNAPSHOT_BYTES, MAX_OBJECT_BLOCKER_SURFACES, MAX_PARTY_MEMBERS, MAX_ROUTE_STEPS,
    MAX_SESSION_UNITS, MAX_UNIT_EFFECTS, MAX_WORLD_COLUMNS, MAX_WORLD_DELTA_OPERATIONS,
    MAX_WORLD_PROJECTION_ENTRIES, MAX_WORLD_RUNS_PER_COLUMN,
};
pub use lobby::{
    LaunchSummaryV1, LobbyAuthority, LobbyMutationError, LobbyPhase, LobbySeatSnapshot,
    LobbySnapshot, LobbyValidationError, SeatConnectionState, SessionPeerId,
};
pub use manifest::{
    BuildIdentityV1, ContentFingerprint, ManifestValidationError, MapManifestV1, ProtocolVersion,
    PublicWorldFingerprint, PublicWorldFingerprintV1, RosterEntryV1, RulesManifestV1,
    SessionInstanceId, SessionLaunchKindV1, SessionManifestV1, UnitDeploymentV1,
    SESSION_PROTOCOL_VERSION,
};
pub use online::{
    OnlineIdentityProvider, OnlineIdentityState, OnlineJoinCode, OnlineJoinCodeDigest,
    OnlineJoinCodeError, OnlineLobbyId, OnlinePrincipal, OnlineServiceRefusal, OnlineSessionEvent,
    OnlineSessionOperation, OnlineSessionProgress, OnlineSessionRequest, PlatformBadge,
    PlayerDisplayName, ReconnectTransportBinding, SessionTransportKind, SnapshotChunkV1,
    SnapshotTransferHeaderV1, SnapshotTransferId, SnapshotTransferProgress,
    SnapshotTransferValidationError, MAX_PLAYER_DISPLAY_NAME_BYTES, MAX_SNAPSHOT_CHUNKS,
    MAX_SNAPSHOT_TRANSFER_BYTES, ONLINE_JOIN_CODE_RANDOM_BYTES, SNAPSHOT_CHUNK_BYTES,
    SNAPSHOT_IN_FLIGHT_CHUNKS, SNAPSHOT_TRANSFER_VERSION_V1,
};
pub use plugin::MultiplayerPlugin;
pub use protocol::{
    register_protocol, AdmissionAccepted, AdmissionCredential, AdmissionRefusal,
    AdmissionRefusalReason, AuthoritySequence, ClientHello, ClientMapReady, CommandOutcome,
    CommandRefusalReason, CommandResult, CommandWireError, GameCommandRequest, SessionCloseReason,
    SessionClosed, PROTOCOL_SCHEMA_TAG,
};
pub use replica::{
    ArchetypeIdentityV1, MotionReplicaV1, ReplicaValidationError, SessionOutcome, SessionReplica,
    UnitReplica,
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
pub use snapshot::{
    BiomeRegionSnapshotV1, InteriorRoofSnapshotV1, InteriorSurfaceSnapshotV1,
    LiveSessionSnapshotV1, LiveSessionSnapshotValidationError, PlayerKnowledgeSnapshotV1,
    PlayerKnowledgeStateV1, PlayerKnownSurfaceV1, PlayerLightDomainV1, SpecialRegionSnapshotV1,
    WorldAnchorSnapshotV1, WorldColumnSnapshotV1, WorldDamageSnapshotV1, WorldDeltaOperationV1,
    WorldDeltaV1, WorldIlluminationV1, WorldLightSnapshotV1, WorldLiquidFlowV1,
    WorldLiquidSnapshotV1, WorldObjectSnapshotV1, WorldRunSnapshotV1, WorldSnapshotV1,
    WorldSnapshotValidationError, WorldViewHintSnapshotV1, LIVE_SESSION_SNAPSHOT_VERSION_V1,
    PLAYER_KNOWLEDGE_SNAPSHOT_VERSION_V1, WORLD_DELTA_VERSION_V1, WORLD_SNAPSHOT_VERSION_V1,
};
#[cfg(feature = "test-harness")]
pub use testing::{ChannelSessionHarness, ClientProbe, HarnessError, HostProbe};
