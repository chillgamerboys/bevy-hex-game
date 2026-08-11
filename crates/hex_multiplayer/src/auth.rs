//! Host-owned custom admission and reconnect credential persistence.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    fs::OpenOptions,
    io::{self, Read as _, Write as _},
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};

use atomicwrites::{AllowOverwrite, AtomicFile};
use bevy_ecs::prelude::{Component, Entity, Resource};
use bevy_replicon::prelude::ProtocolHash;
use hex_core::PlayerSeat;

use crate::{
    AdmissionAccepted, AdmissionCredential, AdmissionRefusalReason, CertificateFingerprint,
    ClientHello, DirectEndpoint, InviteToken, LobbyAuthority, LobbyMutationError, LobbyPhase,
    LobbySnapshot, ManifestValidationError, PublicWorldFingerprint, ReconnectCredential,
    SessionInstanceId, SessionManifestV1, SessionPeerId, MAX_ADVERTISED_HOST_BYTES,
};

const CREDENTIAL_FILE_MAGIC: &[u8; 6] = b"HEXRC2";
const CREDENTIAL_FILE_FIXED_BYTES: usize = CREDENTIAL_FILE_MAGIC.len()
    + SessionInstanceId::BYTE_LENGTH
    + 8
    + 2
    + 2
    + CertificateFingerprint::BYTE_LENGTH
    + 1
    + SessionPeerId::BYTE_LENGTH
    + ReconnectCredential::BYTE_LENGTH;
const CREDENTIAL_FILE_MAX_BYTES: usize = CREDENTIAL_FILE_FIXED_BYTES + MAX_ADVERTISED_HOST_BYTES;

/// Stable authorization derived by the host for one physical connection.
///
/// No serialized client payload can construct this component. Runtime admission inserts
/// it beside Replicon's `AuthorizedClient` only after all compatibility and credential
/// checks succeed.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthorizedSessionClient {
    /// Canonical human seat bound to this connection.
    pub seat: PlayerSeat,
    /// Stable admitted identity independent of transport entity ids.
    pub player_identity: SessionPeerId,
}

/// Successful host-side admission, including the private credential to return once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdmissionGrant {
    /// Connection-bound public authorization.
    pub client: AuthorizedSessionClient,
    /// Independent ordered acceptance message.
    pub accepted: AdmissionAccepted,
    /// Whether the connection reclaimed an existing seat.
    pub reconnected: bool,
}

#[derive(Debug, Clone, Copy)]
struct AdmittedPeer {
    player: SessionPeerId,
    reconnect_credential: ReconnectCredential,
    active_connection: Option<Entity>,
}

/// Pure host-owned admission/lobby authority for one frozen session.
///
/// The transport entity is only a lookup key. Seats, player identities, credentials, and
/// assignments survive transport replacement, while credentials rotate after every
/// successful reconnect.
#[derive(Resource)]
pub struct SessionAdmissionAuthority {
    protocol_hash: ProtocolHash,
    manifest: SessionManifestV1,
    invite_token: InviteToken,
    lobby: LobbyAuthority,
    peers: BTreeMap<PlayerSeat, AdmittedPeer>,
    connections: BTreeMap<Entity, PlayerSeat>,
    map_ready: BTreeSet<PlayerSeat>,
}

impl SessionAdmissionAuthority {
    /// Creates a fresh session with random host identity and invitation credential.
    pub fn new(
        protocol_hash: ProtocolHash,
        manifest: SessionManifestV1,
    ) -> Result<Self, AdmissionSetupError> {
        Self::with_session_secrets(
            protocol_hash,
            manifest,
            SessionPeerId::generate(),
            InviteToken::generate(),
        )
    }

    /// Creates a session from explicit initial secrets, useful for deterministic harnesses.
    pub fn with_session_secrets(
        protocol_hash: ProtocolHash,
        manifest: SessionManifestV1,
        host_identity: SessionPeerId,
        invite_token: InviteToken,
    ) -> Result<Self, AdmissionSetupError> {
        manifest
            .validate()
            .map_err(AdmissionSetupError::InvalidManifest)?;
        let lobby =
            LobbyAuthority::new(host_identity, &manifest).map_err(AdmissionSetupError::Lobby)?;
        Ok(Self {
            protocol_hash,
            manifest,
            invite_token,
            lobby,
            peers: BTreeMap::new(),
            connections: BTreeMap::new(),
            map_ready: BTreeSet::new(),
        })
    }

    /// Frozen session manifest used for exact compatibility and map checks.
    #[must_use]
    pub const fn manifest(&self) -> &SessionManifestV1 {
        &self.manifest
    }

    /// Current one-time invite token for the next generated connection code.
    #[must_use]
    pub const fn invite_token(&self) -> InviteToken {
        self.invite_token
    }

    /// Current six-seat disclosure-safe projection.
    #[must_use]
    pub const fn lobby(&self) -> &LobbyAuthority {
        &self.lobby
    }

    /// Mutable lobby mechanics for host-only assignment/readiness operations.
    #[must_use]
    pub const fn lobby_mut(&mut self) -> &mut LobbyAuthority {
        &mut self.lobby
    }

    /// Derives authorization from a connection and its first ordered hello.
    ///
    /// Compatibility checks precede secret checks so a stale build receives a stable
    /// mismatch without consuming or rotating a valid credential.
    pub fn admit(
        &mut self,
        connection: Entity,
        hello: &ClientHello,
    ) -> Result<AdmissionGrant, AdmissionRefusalReason> {
        if self.connections.contains_key(&connection) {
            return Err(AdmissionRefusalReason::DuplicateActiveSeat);
        }
        if hello.protocol_hash != self.protocol_hash {
            return Err(AdmissionRefusalReason::ProtocolMismatch);
        }
        if hello.build != self.manifest.build {
            return Err(AdmissionRefusalReason::BuildMismatch);
        }
        if hello.content_fingerprint != self.manifest.content_fingerprint {
            return Err(AdmissionRefusalReason::ContentMismatch);
        }

        match hello.credential {
            AdmissionCredential::Invite(presented) => self.admit_invited(connection, presented),
            AdmissionCredential::Reconnect(presented) => {
                self.admit_reconnecting(connection, presented)
            }
        }
    }

    /// Begins loading and closes new admission after validating the host preflight world.
    ///
    /// No peer is marked ready here. The listen host must regenerate the frozen map and
    /// report that actual result through [`Self::report_host_map_ready`], just like every
    /// guest reports through [`Self::report_map_ready`].
    pub fn begin_loading(
        &mut self,
        host_fingerprint: PublicWorldFingerprint,
    ) -> Result<LobbySnapshot, SessionActivationError> {
        if host_fingerprint != self.manifest.map.expected_public_fingerprint {
            return Err(SessionActivationError::MapMismatch);
        }
        self.lobby
            .begin_loading(&self.manifest)
            .map_err(SessionActivationError::Lobby)?;
        self.map_ready.clear();
        Ok(self.lobby.snapshot_owned())
    }

    /// Re-enters loading from an encounter outcome without carrying prior readiness.
    pub fn retry_loading(
        &mut self,
        host_fingerprint: PublicWorldFingerprint,
    ) -> Result<LobbySnapshot, SessionActivationError> {
        if host_fingerprint != self.manifest.map.expected_public_fingerprint {
            return Err(SessionActivationError::MapMismatch);
        }
        self.lobby
            .retry_loading(&self.manifest)
            .map_err(SessionActivationError::Lobby)?;
        self.map_ready.clear();
        Ok(self.lobby.snapshot_owned())
    }

    /// Marks the active encounter as terminal while retaining reconnect eligibility.
    pub fn enter_outcome(&mut self) -> Result<LobbySnapshot, LobbyMutationError> {
        self.lobby.enter_outcome()?;
        Ok(self.lobby.snapshot_owned())
    }

    /// Reopens assignment after an outcome and clears launch verification/readiness.
    pub fn return_to_lobby(&mut self) -> Result<LobbySnapshot, LobbyMutationError> {
        self.lobby.return_to_lobby()?;
        self.map_ready.clear();
        Ok(self.lobby.snapshot_owned())
    }

    /// Records the listen host's actual regenerated map fingerprint.
    pub fn report_host_map_ready(
        &mut self,
        fingerprint: PublicWorldFingerprint,
    ) -> Result<MapReadyStatus, SessionActivationError> {
        self.record_map_ready(PlayerSeat::HOST, fingerprint)
    }

    /// Records one authenticated guest's map fingerprint and activates when all claimed
    /// seats, including the listen host, have reported the exact expected public world.
    pub fn report_map_ready(
        &mut self,
        connection: Entity,
        fingerprint: PublicWorldFingerprint,
    ) -> Result<MapReadyStatus, SessionActivationError> {
        if self.lobby.snapshot().phase != LobbyPhase::Loading {
            return Err(SessionActivationError::WrongPhase);
        }
        let seat = self
            .connections
            .get(&connection)
            .copied()
            .ok_or(SessionActivationError::NotAuthorized)?;
        self.record_map_ready(seat, fingerprint)
    }

    fn record_map_ready(
        &mut self,
        seat: PlayerSeat,
        fingerprint: PublicWorldFingerprint,
    ) -> Result<MapReadyStatus, SessionActivationError> {
        if self.lobby.snapshot().phase != LobbyPhase::Loading {
            return Err(SessionActivationError::WrongPhase);
        }
        if fingerprint != self.manifest.map.expected_public_fingerprint {
            return Err(SessionActivationError::MapMismatch);
        }
        self.map_ready.insert(seat);

        if self.activate_if_every_claimed_seat_is_ready()? {
            Ok(MapReadyStatus::Activated)
        } else {
            Ok(MapReadyStatus::Waiting)
        }
    }

    fn activate_if_every_claimed_seat_is_ready(&mut self) -> Result<bool, SessionActivationError> {
        let all_claimed_ready = self
            .lobby
            .snapshot()
            .seats
            .iter()
            .filter(|entry| entry.connection.is_claimed())
            .all(|entry| self.map_ready.contains(&entry.seat));
        if all_claimed_ready {
            self.lobby
                .activate()
                .map_err(SessionActivationError::Lobby)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Removes a physical connection while preserving its canonical seat and credential.
    pub fn disconnect(&mut self, connection: Entity) -> Option<PlayerSeat> {
        let seat = self.connections.get(&connection).copied()?;
        self.lobby.disconnect(seat).ok()?;
        self.connections.remove(&connection);
        if let Some(peer) = self.peers.get_mut(&seat) {
            peer.active_connection = None;
        }
        self.map_ready.remove(&seat);
        Some(seat)
    }

    /// Removes one guest from an open lobby and invalidates its reconnect credential.
    /// Returns the physical connection that should receive a typed kick before teardown.
    pub fn kick(&mut self, seat: PlayerSeat) -> Result<Option<Entity>, LobbyMutationError> {
        self.lobby.remove_guest(seat)?;
        let connection = self
            .peers
            .remove(&seat)
            .and_then(|peer| peer.active_connection);
        if let Some(connection) = connection {
            self.connections.remove(&connection);
        }
        self.map_ready.remove(&seat);
        Ok(connection)
    }

    /// Applies an explicit remote leave. Open-lobby seats are vacated; later phases retain
    /// their canonical assignment through the ordinary reservation/delegation path.
    pub fn leave(&mut self, connection: Entity) -> Result<PlayerSeat, LobbyMutationError> {
        let seat = self
            .connections
            .get(&connection)
            .copied()
            .ok_or(LobbyMutationError::SeatNotConnected)?;
        if self.lobby.snapshot().phase == LobbyPhase::Open {
            self.kick(seat)?;
            Ok(seat)
        } else {
            self.disconnect(connection)
                .ok_or(LobbyMutationError::SeatNotConnected)
        }
    }

    /// Invalidates all credentials and closes the host-owned session.
    pub fn close(&mut self) {
        self.connections.clear();
        self.peers.clear();
        self.map_ready.clear();
        self.invite_token = InviteToken::generate();
        self.lobby.close();
    }

    /// Returns the host-derived authorization for a physical connection.
    #[must_use]
    pub fn authorized_client(&self, connection: Entity) -> Option<AuthorizedSessionClient> {
        let seat = self.connections.get(&connection).copied()?;
        let peer = self.peers.get(&seat)?;
        Some(AuthorizedSessionClient {
            seat,
            player_identity: peer.player,
        })
    }

    /// Current physical connection for one admitted seat, if connected.
    #[must_use]
    pub fn active_connection(&self, seat: PlayerSeat) -> Option<Entity> {
        self.peers
            .get(&seat)
            .and_then(|peer| peer.active_connection)
    }

    /// Snapshot of every connected non-host peer and its canonical seat.
    #[must_use]
    pub fn connected_peers(&self) -> Vec<(PlayerSeat, Entity)> {
        self.peers
            .iter()
            .filter_map(|(&seat, peer)| peer.active_connection.map(|connection| (seat, connection)))
            .collect()
    }

    fn admit_invited(
        &mut self,
        connection: Entity,
        presented: InviteToken,
    ) -> Result<AdmissionGrant, AdmissionRefusalReason> {
        if self.lobby.snapshot().phase != LobbyPhase::Open {
            return Err(AdmissionRefusalReason::LobbyClosed);
        }
        if !self.invite_token.matches(presented) {
            return Err(AdmissionRefusalReason::InvalidInvite);
        }

        let player = self.unique_peer_identity();
        let seat = self
            .lobby
            .admit_guest(player)
            .map_err(map_lobby_admission_error)?;
        let reconnect_credential = ReconnectCredential::generate();
        self.peers.insert(
            seat,
            AdmittedPeer {
                player,
                reconnect_credential,
                active_connection: Some(connection),
            },
        );
        self.connections.insert(connection, seat);
        self.invite_token = InviteToken::generate();
        Ok(grant(
            self.manifest.session_instance_id,
            seat,
            player,
            reconnect_credential,
            false,
        ))
    }

    fn admit_reconnecting(
        &mut self,
        connection: Entity,
        presented: ReconnectCredential,
    ) -> Result<AdmissionGrant, AdmissionRefusalReason> {
        if self.lobby.snapshot().phase == LobbyPhase::Closed {
            return Err(AdmissionRefusalReason::LobbyClosed);
        }
        let matched = self
            .peers
            .iter()
            .find(|(_, peer)| peer.reconnect_credential.matches(presented))
            .map(|(&seat, peer)| (seat, *peer))
            .ok_or(AdmissionRefusalReason::InvalidReconnect)?;
        let (seat, peer) = matched;
        if peer.active_connection.is_some() {
            return Err(AdmissionRefusalReason::DuplicateActiveSeat);
        }
        self.lobby.reconnect(seat).map_err(|error| match error {
            LobbyMutationError::DuplicateActiveSeat => AdmissionRefusalReason::DuplicateActiveSeat,
            LobbyMutationError::VacantSeat => AdmissionRefusalReason::InvalidReconnect,
            _ => AdmissionRefusalReason::Malformed,
        })?;

        let rotated = ReconnectCredential::generate();
        if let Some(record) = self.peers.get_mut(&seat) {
            record.reconnect_credential = rotated;
            record.active_connection = Some(connection);
        }
        self.connections.insert(connection, seat);
        Ok(grant(
            self.manifest.session_instance_id,
            seat,
            peer.player,
            rotated,
            true,
        ))
    }

    fn unique_peer_identity(&self) -> SessionPeerId {
        loop {
            let candidate = SessionPeerId::generate();
            if candidate != self.lobby.snapshot().host_identity
                && self.peers.values().all(|peer| peer.player != candidate)
            {
                return candidate;
            }
        }
    }
}

impl fmt::Debug for SessionAdmissionAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionAdmissionAuthority")
            .field("protocol_hash", &self.protocol_hash)
            .field("manifest", &self.manifest)
            .field("invite_token", &self.invite_token)
            .field("lobby", &self.lobby)
            .field("peer_count", &self.peers.len())
            .field("connection_count", &self.connections.len())
            .finish_non_exhaustive()
    }
}

fn grant(
    session_instance_id: crate::SessionInstanceId,
    seat: PlayerSeat,
    player_identity: SessionPeerId,
    reconnect_credential: ReconnectCredential,
    reconnected: bool,
) -> AdmissionGrant {
    AdmissionGrant {
        client: AuthorizedSessionClient {
            seat,
            player_identity,
        },
        accepted: AdmissionAccepted {
            session_instance_id,
            seat,
            player_identity,
            reconnect_credential,
        },
        reconnected,
    }
}

fn map_lobby_admission_error(error: LobbyMutationError) -> AdmissionRefusalReason {
    match error {
        LobbyMutationError::LobbyClosed => AdmissionRefusalReason::LobbyClosed,
        LobbyMutationError::LobbyFull => AdmissionRefusalReason::LobbyFull,
        LobbyMutationError::DuplicatePlayerIdentity => AdmissionRefusalReason::DuplicateActiveSeat,
        _ => AdmissionRefusalReason::Malformed,
    }
}

/// Why a session authority could not be constructed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionSetupError {
    /// The frozen session manifest is structurally invalid.
    InvalidManifest(ManifestValidationError),
    /// Host lobby initialization failed.
    Lobby(LobbyMutationError),
}

impl fmt::Display for AdmissionSetupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidManifest(_) => "session manifest is invalid",
            Self::Lobby(_) => "session lobby could not be initialized",
        })
    }
}

impl std::error::Error for AdmissionSetupError {}

/// Result of one exact map fingerprint report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapReadyStatus {
    /// Other claimed peers have not reported yet.
    Waiting,
    /// Every claimed peer matched and the lobby entered active gameplay.
    Activated,
}

/// Why static map verification or activation was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionActivationError {
    /// The reporting transport has not completed admission.
    NotAuthorized,
    /// The complete public world fingerprint differed.
    MapMismatch,
    /// Map readiness was reported outside loading.
    WrongPhase,
    /// The underlying lobby transition failed.
    Lobby(LobbyMutationError),
}

impl fmt::Display for SessionActivationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NotAuthorized => "map report came from an unauthorized connection",
            Self::MapMismatch => "generated public world fingerprint differs",
            Self::WrongPhase => "map readiness was reported outside loading",
            Self::Lobby(_) => "lobby activation transition failed",
        })
    }
}

impl std::error::Error for SessionActivationError {}

/// Direct endpoint/certificate facts bound to persisted reconnect state.
///
/// The candidate binding is derived from the pinned connection code. Runtime storage
/// persists it only after the TLS verifier accepts those exact certificate facts and
/// the host emits [`AdmissionAccepted`].
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct ReconnectEndpointBinding {
    /// Advertised endpoint used for this session.
    pub endpoint: DirectEndpoint,
    /// Exact pinned SPKI digest.
    pub certificate_fingerprint: CertificateFingerprint,
    /// Verified leaf certificate expiry as Unix seconds.
    pub certificate_expires_unix_seconds: u64,
}

impl ReconnectEndpointBinding {
    /// Validates a non-zero certificate expiry.
    pub fn new(
        endpoint: DirectEndpoint,
        certificate_fingerprint: CertificateFingerprint,
        certificate_expires_unix_seconds: u64,
    ) -> Result<Self, CredentialStoreError> {
        if certificate_expires_unix_seconds == 0 {
            return Err(CredentialStoreError::Malformed);
        }
        Ok(Self {
            endpoint,
            certificate_fingerprint,
            certificate_expires_unix_seconds,
        })
    }
}

/// Credential material persisted by a reconnecting client.
#[derive(Clone, PartialEq, Eq)]
pub struct StoredReconnectCredential {
    /// Concrete host session that issued the credential.
    pub session_instance_id: SessionInstanceId,
    /// Verified endpoint/SPKI/certificate-lifetime binding.
    pub endpoint_binding: ReconnectEndpointBinding,
    /// Canonical seat assigned by the host.
    pub seat: PlayerSeat,
    /// Stable session player identity.
    pub player_identity: SessionPeerId,
    /// Current rotating private credential.
    pub reconnect_credential: ReconnectCredential,
}

impl fmt::Debug for StoredReconnectCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoredReconnectCredential")
            .field("session_instance_id", &self.session_instance_id)
            .field("endpoint_binding", &self.endpoint_binding)
            .field("seat", &self.seat)
            .field("player_identity", &self.player_identity)
            .field("reconnect_credential", &self.reconnect_credential)
            .finish()
    }
}

impl StoredReconnectCredential {
    /// Combines one accepted rotating credential with the verified direct endpoint.
    #[must_use]
    pub fn new(accepted: AdmissionAccepted, endpoint_binding: ReconnectEndpointBinding) -> Self {
        Self {
            session_instance_id: accepted.session_instance_id,
            endpoint_binding,
            seat: accepted.seat,
            player_identity: accepted.player_identity,
            reconnect_credential: accepted.reconnect_credential,
        }
    }

    /// Whether the pinned certificate can no longer authenticate this endpoint.
    #[must_use]
    pub const fn is_expired_at(&self, unix_seconds: u64) -> bool {
        unix_seconds >= self.endpoint_binding.certificate_expires_unix_seconds
    }
}

/// Injected persistence boundary for temporary reconnect state.
pub trait ReconnectCredentialStore: Send + Sync + 'static {
    /// Reads the current credential, or `None` when no active session is stored.
    fn load(&self) -> Result<Option<StoredReconnectCredential>, CredentialStoreError>;
    /// Atomically replaces the stored credential after admission or reconnect rotation.
    fn store_atomically(
        &self,
        credential: StoredReconnectCredential,
    ) -> Result<(), CredentialStoreError>;
    /// Deletes reconnect state only when it belongs to `session_instance_id`.
    fn delete_if_session(
        &self,
        session_instance_id: SessionInstanceId,
    ) -> Result<bool, CredentialStoreError>;
    /// Deletes reconnect state only after its verified certificate expiry.
    fn delete_if_expired(&self, unix_seconds: u64) -> Result<bool, CredentialStoreError>;
}

/// Bevy resource wrapping an injected reconnect credential store.
#[derive(Resource, Clone)]
pub struct ReconnectCredentialStorage(Arc<dyn ReconnectCredentialStore>);

impl ReconnectCredentialStorage {
    /// Wraps an application- or test-owned store.
    #[must_use]
    pub fn new(store: impl ReconnectCredentialStore) -> Self {
        Self(Arc::new(store))
    }

    /// Borrows the injected store.
    #[must_use]
    pub fn store(&self) -> &dyn ReconnectCredentialStore {
        self.0.as_ref()
    }
}

impl fmt::Debug for ReconnectCredentialStorage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ReconnectCredentialStorage([INJECTED STORE])")
    }
}

/// Thread-safe in-memory credential storage for deterministic tests.
#[derive(Debug, Default)]
pub struct MemoryReconnectCredentialStore(RwLock<Option<StoredReconnectCredential>>);

impl ReconnectCredentialStore for MemoryReconnectCredentialStore {
    fn load(&self) -> Result<Option<StoredReconnectCredential>, CredentialStoreError> {
        self.0
            .read()
            .map(|stored| stored.clone())
            .map_err(|_poisoned| CredentialStoreError::Unavailable)
    }

    fn store_atomically(
        &self,
        credential: StoredReconnectCredential,
    ) -> Result<(), CredentialStoreError> {
        let mut stored = self
            .0
            .write()
            .map_err(|_poisoned| CredentialStoreError::Unavailable)?;
        *stored = Some(credential);
        Ok(())
    }

    fn delete_if_session(
        &self,
        session_instance_id: SessionInstanceId,
    ) -> Result<bool, CredentialStoreError> {
        let mut stored = self
            .0
            .write()
            .map_err(|_poisoned| CredentialStoreError::Unavailable)?;
        if stored
            .as_ref()
            .is_some_and(|stored| stored.session_instance_id == session_instance_id)
        {
            *stored = None;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn delete_if_expired(&self, unix_seconds: u64) -> Result<bool, CredentialStoreError> {
        let mut stored = self
            .0
            .write()
            .map_err(|_poisoned| CredentialStoreError::Unavailable)?;
        if stored
            .as_ref()
            .is_some_and(|stored| stored.is_expired_at(unix_seconds))
        {
            *stored = None;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// Atomic bounded-size file store beneath an application-selected temporary-data path.
#[derive(Debug, Clone)]
pub struct AtomicFileReconnectCredentialStore {
    path: PathBuf,
}

impl AtomicFileReconnectCredentialStore {
    /// Creates a store for one exact file path.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Exact application-owned storage path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl ReconnectCredentialStore for AtomicFileReconnectCredentialStore {
    fn load(&self) -> Result<Option<StoredReconnectCredential>, CredentialStoreError> {
        let mut file = match std::fs::File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(CredentialStoreError::Io(error)),
        };
        let mut bytes = Vec::with_capacity(CREDENTIAL_FILE_MAX_BYTES.saturating_add(1));
        std::io::Read::by_ref(&mut file)
            .take(u64::try_from(CREDENTIAL_FILE_MAX_BYTES.saturating_add(1)).unwrap_or(u64::MAX))
            .read_to_end(&mut bytes)?;
        decode_stored_credential(&bytes).map(Some)
    }

    fn store_atomically(
        &self,
        credential: StoredReconnectCredential,
    ) -> Result<(), CredentialStoreError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = encode_stored_credential(&credential);
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        AtomicFile::new(&self.path, AllowOverwrite)
            .write_with_options(|file| file.write_all(&bytes), options)
            .map_err(io::Error::from)?;
        Ok(())
    }

    fn delete_if_session(
        &self,
        session_instance_id: SessionInstanceId,
    ) -> Result<bool, CredentialStoreError> {
        let Some(stored) = self.load()? else {
            return Ok(false);
        };
        if stored.session_instance_id != session_instance_id {
            return Ok(false);
        }
        self.delete_file()?;
        Ok(true)
    }

    fn delete_if_expired(&self, unix_seconds: u64) -> Result<bool, CredentialStoreError> {
        let Some(stored) = self.load()? else {
            return Ok(false);
        };
        if !stored.is_expired_at(unix_seconds) {
            return Ok(false);
        }
        self.delete_file()?;
        Ok(true)
    }
}

impl AtomicFileReconnectCredentialStore {
    fn delete_file(&self) -> Result<(), CredentialStoreError> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(CredentialStoreError::Io(error)),
        }
    }
}

fn encode_stored_credential(credential: &StoredReconnectCredential) -> Vec<u8> {
    let host = credential.endpoint_binding.endpoint.host().as_bytes();
    let mut bytes = Vec::with_capacity(CREDENTIAL_FILE_FIXED_BYTES.saturating_add(host.len()));
    bytes.extend_from_slice(CREDENTIAL_FILE_MAGIC);
    bytes.extend_from_slice(&credential.session_instance_id.to_bytes());
    bytes.extend_from_slice(
        &credential
            .endpoint_binding
            .certificate_expires_unix_seconds
            .to_be_bytes(),
    );
    bytes.extend_from_slice(&u16::try_from(host.len()).unwrap_or(u16::MAX).to_be_bytes());
    bytes.extend_from_slice(host);
    bytes.extend_from_slice(&credential.endpoint_binding.endpoint.port().to_be_bytes());
    bytes.extend_from_slice(
        &credential
            .endpoint_binding
            .certificate_fingerprint
            .to_bytes(),
    );
    bytes.push(credential.seat.0);
    bytes.extend_from_slice(&credential.player_identity.to_bytes());
    bytes.extend_from_slice(&credential.reconnect_credential.to_bytes());
    bytes
}

fn decode_stored_credential(
    bytes: &[u8],
) -> Result<StoredReconnectCredential, CredentialStoreError> {
    if bytes.len() < CREDENTIAL_FILE_FIXED_BYTES || bytes.len() > CREDENTIAL_FILE_MAX_BYTES {
        return Err(CredentialStoreError::Malformed);
    }
    let mut remaining = bytes;
    if take_array::<6>(&mut remaining)? != *CREDENTIAL_FILE_MAGIC {
        return Err(CredentialStoreError::Malformed);
    }
    let session_instance_id = SessionInstanceId::from_bytes(take_array::<16>(&mut remaining)?);
    if !session_instance_id.is_valid() {
        return Err(CredentialStoreError::Malformed);
    }
    let certificate_expires_unix_seconds = u64::from_be_bytes(take_array::<8>(&mut remaining)?);
    if certificate_expires_unix_seconds == 0 {
        return Err(CredentialStoreError::Malformed);
    }
    let host_length = usize::from(u16::from_be_bytes(take_array::<2>(&mut remaining)?));
    if host_length == 0 || host_length > MAX_ADVERTISED_HOST_BYTES {
        return Err(CredentialStoreError::Malformed);
    }
    let (host, tail) = remaining
        .split_at_checked(host_length)
        .ok_or(CredentialStoreError::Malformed)?;
    remaining = tail;
    let host = std::str::from_utf8(host).map_err(|_error| CredentialStoreError::Malformed)?;
    let port = u16::from_be_bytes(take_array::<2>(&mut remaining)?);
    let endpoint =
        DirectEndpoint::new(host, port).map_err(|_error| CredentialStoreError::Malformed)?;
    let certificate_fingerprint =
        CertificateFingerprint::from_bytes(take_array::<32>(&mut remaining)?);
    let seat_byte = take_array::<1>(&mut remaining)?;
    let seat = PlayerSeat::human(
        seat_byte
            .first()
            .copied()
            .ok_or(CredentialStoreError::Malformed)?,
    )
    .ok_or(CredentialStoreError::Malformed)?;
    let player_identity = SessionPeerId::from_bytes(take_array::<16>(&mut remaining)?);
    let reconnect_credential = ReconnectCredential::from_bytes(take_array::<32>(&mut remaining)?);
    if !remaining.is_empty() {
        return Err(CredentialStoreError::Malformed);
    }
    Ok(StoredReconnectCredential {
        session_instance_id,
        endpoint_binding: ReconnectEndpointBinding {
            endpoint,
            certificate_fingerprint,
            certificate_expires_unix_seconds,
        },
        seat,
        player_identity,
        reconnect_credential,
    })
}

fn take_array<const LENGTH: usize>(
    bytes: &mut &[u8],
) -> Result<[u8; LENGTH], CredentialStoreError> {
    let (head, tail) = bytes
        .split_at_checked(LENGTH)
        .ok_or(CredentialStoreError::Malformed)?;
    *bytes = tail;
    head.try_into()
        .map_err(|_error| CredentialStoreError::Malformed)
}

/// Failure at the temporary reconnect persistence boundary.
#[derive(Debug)]
pub enum CredentialStoreError {
    /// Filesystem access failed.
    Io(io::Error),
    /// Stored bytes did not match the fixed bounded format.
    Malformed,
    /// An in-memory synchronization primitive was poisoned.
    Unavailable,
}

impl From<io::Error> for CredentialStoreError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl fmt::Display for CredentialStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Io(_) => "reconnect credential storage I/O failed",
            Self::Malformed => "stored reconnect credential is malformed",
            Self::Unavailable => "reconnect credential storage is unavailable",
        })
    }
}

impl std::error::Error for CredentialStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Malformed | Self::Unavailable => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use hex_core::{Faction, SimSeeds, TilePos, UnitId};

    use super::*;
    use crate::{
        BoundError, BoundedText, BoundedVec, BuildIdentityV1, ContentFingerprint, MapManifestV1,
        ProtocolVersion, RosterEntryV1, RulesManifestV1, UnitDeploymentV1, MAX_IDENTITY_BYTES,
    };

    fn text(value: &str) -> BoundedText<MAX_IDENTITY_BYTES> {
        BoundedText::new(value).expect("fixture identity should fit")
    }

    fn manifest() -> SessionManifestV1 {
        let roster = (0_u64..6)
            .map(|unit| RosterEntryV1 {
                unit: UnitId(unit),
                archetype_identity: text("warrior"),
                character_identity: text(&format!("hero-{unit}")),
                faction: Faction::Player,
            })
            .collect();
        let deployment = (0_u64..6)
            .map(|unit| UnitDeploymentV1 {
                unit: UnitId(unit),
                position: TilePos::ORIGIN,
            })
            .collect();
        SessionManifestV1 {
            session_instance_id: crate::SessionInstanceId::from_bytes([1; 16]),
            protocol: ProtocolVersion::default(),
            build: BuildIdentityV1::new("0.4.0", "fixture").expect("valid build"),
            content_fingerprint: ContentFingerprint(11),
            scenario_identity: text("sandbox"),
            map: MapManifestV1 {
                catalog_identity: text("small"),
                seed: 9,
                generator_identity: text("v3"),
                generator_version: 3,
                expected_public_fingerprint: PublicWorldFingerprint(12),
            },
            rules: RulesManifestV1 {
                profile_identity: text("default"),
                fingerprint: 13,
            },
            shipped_roster: BoundedVec::new(roster).expect("six roster entries fit"),
            deployment: BoundedVec::new(deployment).expect("six deployments fit"),
            simulation_seeds: SimSeeds::default(),
        }
    }

    fn hello(
        authority: &SessionAdmissionAuthority,
        credential: AdmissionCredential,
    ) -> ClientHello {
        ClientHello {
            protocol_hash: authority.protocol_hash,
            build: authority.manifest.build.clone(),
            content_fingerprint: authority.manifest.content_fingerprint,
            credential,
        }
    }

    fn authority() -> SessionAdmissionAuthority {
        SessionAdmissionAuthority::with_session_secrets(
            protocol_hash(),
            manifest(),
            SessionPeerId::from_bytes([1; 16]),
            InviteToken::from_bytes([2; 16]),
        )
        .expect("valid authority")
    }

    fn protocol_hash() -> ProtocolHash {
        serde_json::from_str("99").expect("protocol hash is a serialized newtype")
    }

    #[test]
    fn compatibility_failures_do_not_consume_the_invite() {
        let mut authority = authority();
        let invite = authority.invite_token();
        let mut wrong_protocol = hello(&authority, AdmissionCredential::Invite(invite));
        wrong_protocol.protocol_hash =
            serde_json::from_str("100").expect("protocol hash is a serialized newtype");
        assert_eq!(
            authority.admit(Entity::from_bits(1), &wrong_protocol),
            Err(AdmissionRefusalReason::ProtocolMismatch)
        );
        assert!(authority.invite_token().matches(invite));

        let mut stale = hello(&authority, AdmissionCredential::Invite(invite));
        stale.build = BuildIdentityV1::new("0.4.0", "stale").expect("valid stale build");
        assert_eq!(
            authority.admit(Entity::from_bits(1), &stale),
            Err(AdmissionRefusalReason::BuildMismatch)
        );
        assert!(authority.invite_token().matches(invite));

        let mut wrong_content = hello(&authority, AdmissionCredential::Invite(invite));
        wrong_content.content_fingerprint = ContentFingerprint(999);
        assert_eq!(
            authority.admit(Entity::from_bits(1), &wrong_content),
            Err(AdmissionRefusalReason::ContentMismatch)
        );
        assert!(authority.invite_token().matches(invite));

        let accepted = authority
            .admit(
                Entity::from_bits(1),
                &hello(&authority, AdmissionCredential::Invite(invite)),
            )
            .expect("unchanged invite should still work");
        assert_eq!(accepted.client.seat, PlayerSeat(1));
        assert!(!authority.invite_token().matches(invite));
        assert_eq!(
            authority.admit(
                Entity::from_bits(2),
                &hello(&authority, AdmissionCredential::Invite(invite)),
            ),
            Err(AdmissionRefusalReason::InvalidInvite),
            "a consumed invitation cannot be reused"
        );
    }

    #[test]
    fn reconnect_rotates_secret_and_rejects_duplicate_active_seat() {
        let mut authority = authority();
        let first_connection = Entity::from_bits(1);
        let admitted = authority
            .admit(
                first_connection,
                &hello(
                    &authority,
                    AdmissionCredential::Invite(authority.invite_token()),
                ),
            )
            .expect("guest should be admitted");
        let original = admitted.accepted.reconnect_credential;
        assert_eq!(
            admitted.accepted.session_instance_id,
            authority.manifest().session_instance_id
        );
        assert_eq!(
            authority.admit(
                Entity::from_bits(2),
                &hello(&authority, AdmissionCredential::Reconnect(original)),
            ),
            Err(AdmissionRefusalReason::DuplicateActiveSeat)
        );

        assert_eq!(authority.disconnect(first_connection), Some(PlayerSeat(1)));
        let reconnected = authority
            .admit(
                Entity::from_bits(2),
                &hello(&authority, AdmissionCredential::Reconnect(original)),
            )
            .expect("reserved seat should reconnect");
        assert!(reconnected.reconnected);
        assert_eq!(
            reconnected.accepted.session_instance_id,
            authority.manifest().session_instance_id
        );
        assert!(!reconnected.accepted.reconnect_credential.matches(original));
        assert_eq!(
            authority.admit(
                Entity::from_bits(3),
                &hello(&authority, AdmissionCredential::Reconnect(original)),
            ),
            Err(AdmissionRefusalReason::InvalidReconnect)
        );
    }

    #[test]
    fn six_guests_are_bounded_by_party_ownership_not_numeric_seats() {
        let mut authority = authority();
        for connection_id in 1_u64..=5 {
            let invite = authority.invite_token();
            let result = authority.admit(
                Entity::from_bits(connection_id),
                &hello(&authority, AdmissionCredential::Invite(invite)),
            );
            assert!(
                result.is_ok(),
                "five guests should fill seats one through five"
            );
        }
        let final_invite = authority.invite_token();
        assert_eq!(
            authority.admit(
                Entity::from_bits(10),
                &hello(&authority, AdmissionCredential::Invite(final_invite)),
            ),
            Err(AdmissionRefusalReason::LobbyFull)
        );
    }

    #[test]
    fn map_activation_requires_exact_report_from_every_claimed_seat() {
        let mut authority = authority();
        let connection = Entity::from_bits(1);
        let invite = authority.invite_token();
        authority
            .admit(
                connection,
                &hello(&authority, AdmissionCredential::Invite(invite)),
            )
            .expect("guest should be admitted");
        authority
            .lobby_mut()
            .set_ready(PlayerSeat(1), true)
            .expect("guest can ready");
        assert!(authority.begin_loading(PublicWorldFingerprint(12)).is_ok());
        let closed_invite = authority.invite_token();
        assert_eq!(
            authority.admit(
                Entity::from_bits(2),
                &hello(&authority, AdmissionCredential::Invite(closed_invite)),
            ),
            Err(AdmissionRefusalReason::LobbyClosed)
        );
        assert_eq!(
            authority.report_map_ready(connection, PublicWorldFingerprint(999)),
            Err(SessionActivationError::MapMismatch)
        );
        assert_eq!(
            authority.report_host_map_ready(PublicWorldFingerprint(12)),
            Ok(MapReadyStatus::Waiting)
        );
        assert_eq!(
            authority.report_map_ready(connection, PublicWorldFingerprint(12)),
            Ok(MapReadyStatus::Activated)
        );
        assert_eq!(authority.lobby().snapshot().phase, LobbyPhase::Active);
    }

    #[test]
    fn host_only_lobby_waits_for_its_regenerated_host_fingerprint() {
        let mut authority = authority();

        let snapshot = authority
            .begin_loading(PublicWorldFingerprint(12))
            .expect("the exact preflight host world should enter loading");

        assert_eq!(snapshot.phase, LobbyPhase::Loading);
        assert_eq!(authority.lobby().snapshot().phase, LobbyPhase::Loading);
        assert_eq!(
            authority.report_host_map_ready(PublicWorldFingerprint(999)),
            Err(SessionActivationError::MapMismatch)
        );
        assert_eq!(authority.lobby().snapshot().phase, LobbyPhase::Loading);
        assert_eq!(
            authority.report_host_map_ready(PublicWorldFingerprint(12)),
            Ok(MapReadyStatus::Activated)
        );
        assert_eq!(authority.lobby().snapshot().phase, LobbyPhase::Active);
    }

    #[test]
    fn memory_and_atomic_file_stores_round_trip_and_delete_secrets() {
        let stored = StoredReconnectCredential {
            session_instance_id: SessionInstanceId::from_bytes([3; 16]),
            endpoint_binding: ReconnectEndpointBinding::new(
                DirectEndpoint::new("127.0.0.1", 7777).expect("endpoint should be valid"),
                CertificateFingerprint::from_bytes([9; 32]),
                2_000,
            )
            .expect("binding should be valid"),
            seat: PlayerSeat(3),
            player_identity: SessionPeerId::from_bytes([4; 16]),
            reconnect_credential: ReconnectCredential::from_bytes([5; 32]),
        };
        let memory = MemoryReconnectCredentialStore::default();
        memory
            .store_atomically(stored.clone())
            .expect("memory store works");
        assert!(matches!(memory.load(), Ok(Some(value)) if value == stored));
        assert!(!memory
            .delete_if_session(SessionInstanceId::from_bytes([8; 16]))
            .expect("unrelated closure should be harmless"));
        assert!(matches!(memory.load(), Ok(Some(value)) if value == stored));
        assert!(memory
            .delete_if_session(stored.session_instance_id)
            .expect("matching memory delete works"));
        assert!(matches!(memory.load(), Ok(None)));

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let directory = std::env::temp_dir().join(format!(
            "hex-multiplayer-credential-{}-{nonce}",
            std::process::id()
        ));
        let path = directory.join("reconnect.bin");
        let file = AtomicFileReconnectCredentialStore::new(&path);
        file.store_atomically(stored.clone())
            .expect("atomic store works");
        assert!(matches!(file.load(), Ok(Some(value)) if value == stored));
        assert_eq!(
            std::fs::read(&path)
                .expect("stored credential should be readable")
                .len(),
            encode_stored_credential(&stored).len()
        );
        assert!(!file
            .delete_if_expired(1_999)
            .expect("unexpired state should be preserved"));
        assert!(file
            .delete_if_expired(2_000)
            .expect("expired state should be deleted"));
        assert!(matches!(file.load(), Ok(None)));
        let _cleanup_result = std::fs::remove_dir(&directory);
    }

    #[test]
    fn malformed_stored_bytes_fail_closed_without_allocating_past_the_cap() {
        for length in 0..CREDENTIAL_FILE_FIXED_BYTES {
            let bytes = vec![0_u8; length];
            assert!(decode_stored_credential(&bytes).is_err());
        }
        assert!(
            decode_stored_credential(&vec![0_u8; CREDENTIAL_FILE_MAX_BYTES.saturating_add(1)])
                .is_err()
        );
        assert_eq!(
            BoundedText::<1>::new("too long"),
            Err(BoundError::TextTooLong {
                maximum: 1,
                actual: 8
            })
        );
    }
}
