//! Store-neutral online identity, lobby, reconnect, and streamed-snapshot contracts.

use std::{fmt, hash::Hash};

use bevy_ecs::prelude::Message;
use rand::{rngs::OsRng, RngCore as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::{
    AuthoritySequence, BoundError, BoundedText, BoundedVec, ReconnectEndpointBinding,
    SessionInstanceId, MAX_IDENTITY_BYTES,
};

/// Random bytes carried by one 16-character Crockford Base32 join code (80 bits).
pub const ONLINE_JOIN_CODE_RANDOM_BYTES: usize = 10;
/// Maximum sanitized UTF-8 bytes in a lobby display name.
pub const MAX_PLAYER_DISPLAY_NAME_BYTES: usize = 48;
/// Snapshot transfer protocol version.
pub const SNAPSHOT_TRANSFER_VERSION_V1: u16 = 1;
/// Payload bytes in one bounded reliable-unordered snapshot chunk.
pub const SNAPSHOT_CHUNK_BYTES: usize = 32 * 1024;
/// Maximum compressed or uncompressed snapshot transfer allocation.
pub const MAX_SNAPSHOT_TRANSFER_BYTES: usize = crate::limits::MAX_SNAPSHOT_TRANSFER_BYTES;
/// Maximum chunks in a 64 MiB transfer.
pub const MAX_SNAPSHOT_CHUNKS: usize = MAX_SNAPSHOT_TRANSFER_BYTES / SNAPSHOT_CHUNK_BYTES;
/// Maximum chunks sent without acknowledgement.
pub const SNAPSHOT_IN_FLIGHT_CHUNKS: usize = 8;

const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Transport selected for one concrete multiplayer session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SessionTransportKind {
    /// Existing encrypted, SPKI-pinned WebTransport Direct/LAN path.
    DirectWebTransport,
    /// Store-neutral Epic Online Services P2P path.
    EosP2p,
}

/// Store identity provider used to obtain one EOS Product User ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OnlineIdentityProvider {
    /// Device-local standalone guest identity.
    DeviceId,
    /// Steam authentication ticket exchanged through EOS Connect.
    Steam,
}

/// Presentation-only platform badge; never an authorization input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PlatformBadge {
    /// Authenticated from a Steam build ticket.
    Steam,
    /// Device-local standalone guest.
    Standalone,
}

/// Sanitized player-facing lobby name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PlayerDisplayName(BoundedText<MAX_PLAYER_DISPLAY_NAME_BYTES>);

impl PlayerDisplayName {
    /// Trims and validates a display name.
    pub fn new(value: impl AsRef<str>) -> Result<Self, BoundError> {
        BoundedText::new(value.as_ref().trim().to_owned()).map(Self)
    }

    /// Borrows the sanitized presentation string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// Opaque EOS Product User ID.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OnlinePrincipal(BoundedText<MAX_IDENTITY_BYTES>);

impl OnlinePrincipal {
    /// Validates the canonical string produced by the EOS SDK.
    pub fn new(value: impl Into<String>) -> Result<Self, BoundError> {
        BoundedText::new(value).map(Self)
    }

    /// Borrows the canonical value only for the online-service authorization adapter.
    #[must_use]
    pub fn expose_to_online_service(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for OnlinePrincipal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OnlinePrincipal([REDACTED])")
    }
}

/// Opaque EOS lobby identity.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OnlineLobbyId(BoundedText<MAX_IDENTITY_BYTES>);

impl OnlineLobbyId {
    /// Validates the canonical string produced by the EOS SDK.
    pub fn new(value: impl Into<String>) -> Result<Self, BoundError> {
        BoundedText::new(value).map(Self)
    }

    /// Borrows the canonical value only for the online-service lobby adapter.
    #[must_use]
    pub fn expose_to_online_service(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for OnlineLobbyId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OnlineLobbyId([REDACTED])")
    }
}

/// Private, human-shareable 80-bit online join code.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OnlineJoinCode([u8; ONLINE_JOIN_CODE_RANDOM_BYTES]);

impl OnlineJoinCode {
    /// Generates a code from the operating system cryptographic random source.
    #[must_use]
    pub fn generate() -> Self {
        let mut bytes = [0_u8; ONLINE_JOIN_CODE_RANDOM_BYTES];
        while bytes == [0; ONLINE_JOIN_CODE_RANDOM_BYTES] {
            OsRng.fill_bytes(&mut bytes);
        }
        Self(bytes)
    }

    /// Constructs a deterministic code for decoding and tests.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; ONLINE_JOIN_CODE_RANDOM_BYTES]) -> Self {
        Self(bytes)
    }

    /// Parses exactly four groups of four Crockford Base32 characters.
    pub fn parse(value: &str) -> Result<Self, OnlineJoinCodeError> {
        let mut symbols = [0_u8; 16];
        let mut count = 0_usize;
        for byte in value.bytes() {
            if byte == b'-' {
                continue;
            }
            let Some(slot) = symbols.get_mut(count) else {
                return Err(OnlineJoinCodeError::WrongLength);
            };
            *slot = decode_crockford(byte).ok_or(OnlineJoinCodeError::InvalidCharacter)?;
            count = count.saturating_add(1);
        }
        if count != symbols.len() || !valid_grouping(value) {
            return Err(OnlineJoinCodeError::WrongLength);
        }

        let mut bytes = [0_u8; ONLINE_JOIN_CODE_RANDOM_BYTES];
        let mut accumulator = 0_u32;
        let mut bits = 0_u8;
        let mut output = 0_usize;
        for symbol in symbols {
            accumulator = (accumulator << 5) | u32::from(symbol);
            bits = bits.saturating_add(5);
            while bits >= 8 {
                bits -= 8;
                let value = (accumulator >> bits) & 0xff;
                let Some(slot) = bytes.get_mut(output) else {
                    return Err(OnlineJoinCodeError::WrongLength);
                };
                *slot = u8::try_from(value).map_err(|_error| OnlineJoinCodeError::WrongLength)?;
                output = output.saturating_add(1);
                accumulator &= (1_u32 << bits).saturating_sub(1);
            }
        }
        if output != bytes.len() || bits != 0 {
            return Err(OnlineJoinCodeError::WrongLength);
        }
        Ok(Self(bytes))
    }

    /// Returns the explicit player-share surface `XXXX-XXXX-XXXX-XXXX`.
    ///
    /// The returned string is secret-bearing and must never be logged.
    #[must_use]
    pub fn expose_for_sharing(self) -> String {
        let mut symbols = [0_u8; 16];
        let mut accumulator = 0_u32;
        let mut bits = 0_u8;
        let mut output = 0_usize;
        for byte in self.0 {
            accumulator = (accumulator << 8) | u32::from(byte);
            bits = bits.saturating_add(8);
            while bits >= 5 {
                bits -= 5;
                let index = usize::try_from((accumulator >> bits) & 0x1f).unwrap_or_default();
                if let (Some(slot), Some(&symbol)) = (symbols.get_mut(output), CROCKFORD.get(index))
                {
                    *slot = symbol;
                }
                output = output.saturating_add(1);
                accumulator &= (1_u32 << bits).saturating_sub(1);
            }
        }
        let mut result = String::with_capacity(19);
        for (index, symbol) in symbols.into_iter().enumerate() {
            if index > 0 && index % 4 == 0 {
                result.push('-');
            }
            result.push(char::from(symbol));
        }
        result
    }

    /// SHA-256 lobby-search digest; the raw code is verified during encrypted admission.
    #[must_use]
    pub fn digest(self) -> OnlineJoinCodeDigest {
        OnlineJoinCodeDigest(Sha256::digest(self.0).into())
    }

    /// Compares a presented code without exposing it.
    #[must_use]
    pub fn matches(self, presented: Self) -> bool {
        let mut difference = 0_u8;
        for (left, right) in self.0.into_iter().zip(presented.0) {
            difference |= left ^ right;
        }
        difference == 0
    }
}

impl fmt::Debug for OnlineJoinCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OnlineJoinCode([REDACTED])")
    }
}

/// Public lobby-search digest of an online join code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OnlineJoinCodeDigest([u8; 32]);

impl OnlineJoinCodeDigest {
    /// Exact digest bytes suitable for EOS lobby metadata.
    #[must_use]
    pub const fn to_bytes(self) -> [u8; 32] {
        self.0
    }
}

/// Why a player-entered online join code is invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnlineJoinCodeError {
    /// The code is not exactly four four-character groups.
    WrongLength,
    /// A symbol is outside the unambiguous Crockford alphabet.
    InvalidCharacter,
}

impl fmt::Display for OnlineJoinCodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::WrongLength => "online join code must be XXXX-XXXX-XXXX-XXXX",
            Self::InvalidCharacter => "online join code contains an invalid character",
        })
    }
}

impl std::error::Error for OnlineJoinCodeError {}

/// Current store-neutral identity lifecycle.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum OnlineIdentityState {
    /// No online identity request is active.
    #[default]
    SignedOut,
    /// Silent authentication is in progress.
    SigningIn(OnlineIdentityProvider),
    /// EOS Connect produced an authenticated principal.
    SignedIn {
        /// Opaque authorization identity.
        principal: OnlinePrincipal,
        /// Sanitized lobby name.
        display_name: PlayerDisplayName,
        /// Presentation-only platform source.
        badge: PlatformBadge,
    },
    /// Authentication failed with a typed disclosure-safe reason.
    Failed(OnlineServiceRefusal),
}

/// Explicit application requests accepted by an online-service adapter.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub enum OnlineSessionRequest {
    /// Silently authenticate and create one private EOS lobby.
    Host,
    /// Find and join the EOS lobby matching a private code.
    JoinCode(OnlineJoinCode),
    /// Reconnect through a session-bound online credential.
    Reconnect,
    /// Leave the current lobby/session and clear joinable presence.
    Leave,
}

/// Stable operation used to correlate typed online refusals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnlineSessionOperation {
    /// Silent identity acquisition.
    Authenticate,
    /// Lobby creation.
    Host,
    /// Join-code search/admission.
    Join,
    /// Previously admitted player reconnect.
    Reconnect,
    /// Local leave/cleanup.
    Leave,
    /// Packet transport.
    Transport,
}

/// Player-facing progress stages for asynchronous online work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnlineSessionProgress {
    /// Acquiring a standalone or Steam-backed EOS identity.
    Authenticating,
    /// Creating the host lobby.
    CreatingLobby,
    /// Searching lobby metadata by join-code digest.
    SearchingLobby,
    /// Establishing EOS P2P to the lobby owner.
    ConnectingToHost,
    /// Completing the existing encrypted game admission handshake.
    Authorizing,
    /// Receiving and validating a restart snapshot.
    TransferringSnapshot(SnapshotTransferProgress),
}

/// Typed disclosure-safe failure at the EOS/service boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnlineServiceRefusal {
    /// Online support is not compiled or configured.
    Disabled,
    /// The checksum-pinned EOS runtime is absent or incompatible.
    RuntimeUnavailable,
    /// Product/sandbox/deployment credentials are absent or invalid.
    NotConfigured,
    /// Silent identity acquisition failed.
    AuthenticationFailed,
    /// No compatible open lobby matched the code digest.
    LobbyNotFound,
    /// Exact build/content/protocol compatibility failed.
    Incompatible,
    /// Admission has closed because launch began.
    LobbyClosed,
    /// Capacity or party-seat requirements cannot be met.
    LobbyFull,
    /// The raw code or reconnect binding failed encrypted verification.
    InvalidCredential,
    /// Service request budget was exceeded.
    RateLimited,
    /// EOS is temporarily unavailable.
    ServiceUnavailable,
    /// Service callback or packet data violated the local bounded contract.
    MalformedServiceData,
}

/// Typed event stream emitted by the safe online-service adapter.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub enum OnlineSessionEvent {
    /// Identity lifecycle changed.
    Identity(OnlineIdentityState),
    /// One asynchronous stage began or advanced.
    Progress(OnlineSessionProgress),
    /// Host lobby is ready and exposes its private share code explicitly.
    Hosted {
        /// Opaque EOS lobby identity.
        lobby_id: OnlineLobbyId,
        /// Secret share surface; `Debug` remains redacted.
        join_code: OnlineJoinCode,
    },
    /// Client joined the target lobby and may begin game admission.
    Joined {
        /// Opaque EOS lobby identity.
        lobby_id: OnlineLobbyId,
    },
    /// One explicit operation failed without leaking platform data.
    Refused {
        /// Operation that failed.
        operation: OnlineSessionOperation,
        /// Stable presentation-safe reason.
        reason: OnlineServiceRefusal,
    },
    /// An established online session ended.
    Disconnected(OnlineServiceRefusal),
    /// Local leave/cleanup completed.
    Left,
}

/// Transport-specific binding retained beside a rotating reconnect credential.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReconnectTransportBinding {
    /// Existing Direct endpoint/SPKI/certificate binding.
    Direct(ReconnectEndpointBinding),
    /// EOS lobby-owner binding for a previously admitted principal.
    Online {
        /// Opaque lobby retained for reconnect discovery.
        lobby_id: OnlineLobbyId,
        /// Authenticated owner principal.
        host_principal: OnlinePrincipal,
        /// Service/session expiry as Unix seconds.
        expires_at_unix_seconds: u64,
    },
}

impl fmt::Debug for ReconnectTransportBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Direct(_) => formatter.write_str("ReconnectTransportBinding::Direct([REDACTED])"),
            Self::Online { .. } => {
                formatter.write_str("ReconnectTransportBinding::Online([REDACTED])")
            }
        }
    }
}

/// Random identity of one snapshot transfer attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SnapshotTransferId([u8; 16]);

impl SnapshotTransferId {
    /// Generates a non-zero transfer identity.
    #[must_use]
    pub fn generate() -> Self {
        let mut bytes = [0_u8; 16];
        while bytes == [0; 16] {
            OsRng.fill_bytes(&mut bytes);
        }
        Self(bytes)
    }

    /// Constructs an identity for decoding and deterministic tests.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Whether this identity was assigned.
    #[must_use]
    pub fn is_valid(self) -> bool {
        self.0 != [0; 16]
    }
}

/// Allocation-safe metadata sent before compressed snapshot chunks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotTransferHeaderV1 {
    /// Transfer schema.
    pub version: u16,
    /// Concrete host session.
    pub session_instance_id: SessionInstanceId,
    /// Unique transfer attempt.
    pub transfer_id: SnapshotTransferId,
    /// Authority baseline represented by canonical bytes.
    pub baseline_sequence: AuthoritySequence,
    /// Exact canonical byte length before compression.
    pub uncompressed_bytes: u32,
    /// Exact zstd payload length.
    pub compressed_bytes: u32,
    /// SHA-256 of uncompressed canonical bytes.
    pub uncompressed_sha256: [u8; 32],
    /// Exact 32 KiB chunk count.
    pub chunk_count: u32,
}

impl SnapshotTransferHeaderV1 {
    /// Validates all lengths before allocating a receive buffer.
    pub fn validate(self) -> Result<(), SnapshotTransferValidationError> {
        if self.version != SNAPSHOT_TRANSFER_VERSION_V1 {
            return Err(SnapshotTransferValidationError::WrongVersion);
        }
        if !self.session_instance_id.is_valid() || !self.transfer_id.is_valid() {
            return Err(SnapshotTransferValidationError::InvalidIdentity);
        }
        let uncompressed = usize::try_from(self.uncompressed_bytes)
            .map_err(|_error| SnapshotTransferValidationError::TransferTooLarge)?;
        let compressed = usize::try_from(self.compressed_bytes)
            .map_err(|_error| SnapshotTransferValidationError::TransferTooLarge)?;
        if uncompressed == 0 || compressed == 0 {
            return Err(SnapshotTransferValidationError::EmptyTransfer);
        }
        if uncompressed > MAX_SNAPSHOT_TRANSFER_BYTES || compressed > MAX_SNAPSHOT_TRANSFER_BYTES {
            return Err(SnapshotTransferValidationError::TransferTooLarge);
        }
        let expected = compressed.div_ceil(SNAPSHOT_CHUNK_BYTES);
        if usize::try_from(self.chunk_count) != Ok(expected) || expected > MAX_SNAPSHOT_CHUNKS {
            return Err(SnapshotTransferValidationError::ChunkCountMismatch);
        }
        Ok(())
    }
}

/// One bounded compressed snapshot chunk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotChunkV1 {
    /// Correlates this chunk with one header/attempt.
    pub transfer_id: SnapshotTransferId,
    /// Zero-based chunk index.
    pub chunk_index: u32,
    /// At most 32 KiB of compressed data.
    pub payload: BoundedVec<u8, SNAPSHOT_CHUNK_BYTES>,
}

impl SnapshotChunkV1 {
    /// Validates correlation, index, and exact non-final/final payload length.
    pub fn validate_against(
        &self,
        header: SnapshotTransferHeaderV1,
    ) -> Result<(), SnapshotTransferValidationError> {
        header.validate()?;
        if self.transfer_id != header.transfer_id {
            return Err(SnapshotTransferValidationError::WrongTransfer);
        }
        if self.chunk_index >= header.chunk_count || self.payload.is_empty() {
            return Err(SnapshotTransferValidationError::ChunkOutOfRange);
        }
        let index = usize::try_from(self.chunk_index)
            .map_err(|_error| SnapshotTransferValidationError::ChunkOutOfRange)?;
        let compressed = usize::try_from(header.compressed_bytes)
            .map_err(|_error| SnapshotTransferValidationError::TransferTooLarge)?;
        let remaining = compressed.saturating_sub(index.saturating_mul(SNAPSHOT_CHUNK_BYTES));
        let expected = remaining.min(SNAPSHOT_CHUNK_BYTES);
        if self.payload.len() != expected {
            return Err(SnapshotTransferValidationError::ChunkLengthMismatch);
        }
        Ok(())
    }
}

/// Immutable UI progress for one bounded snapshot transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotTransferProgress {
    /// Chunks accepted exactly once.
    pub received_chunks: u32,
    /// Header-declared chunk count.
    pub total_chunks: u32,
    /// Compressed bytes accepted exactly once.
    pub received_bytes: u32,
    /// Header-declared compressed bytes.
    pub total_bytes: u32,
}

impl SnapshotTransferProgress {
    /// Validates monotonically bounded progress.
    pub fn validate(self) -> Result<(), SnapshotTransferValidationError> {
        if self.total_chunks == 0 || self.total_bytes == 0 {
            return Err(SnapshotTransferValidationError::EmptyTransfer);
        }
        if self.received_chunks > self.total_chunks || self.received_bytes > self.total_bytes {
            return Err(SnapshotTransferValidationError::ProgressOutOfRange);
        }
        Ok(())
    }
}

/// Why snapshot transfer metadata or a chunk is invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotTransferValidationError {
    /// Unsupported transfer schema.
    WrongVersion,
    /// Session or transfer identity is zero/unassigned.
    InvalidIdentity,
    /// Compressed or uncompressed length is zero.
    EmptyTransfer,
    /// A declared length exceeds 64 MiB.
    TransferTooLarge,
    /// Chunk count disagrees with the compressed byte length.
    ChunkCountMismatch,
    /// Chunk belongs to another transfer.
    WrongTransfer,
    /// Chunk index is outside the declared range.
    ChunkOutOfRange,
    /// Chunk payload length disagrees with its exact position.
    ChunkLengthMismatch,
    /// UI progress exceeds its declared totals.
    ProgressOutOfRange,
}

impl fmt::Display for SnapshotTransferValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::WrongVersion => "snapshot transfer version is unsupported",
            Self::InvalidIdentity => "snapshot transfer identity is invalid",
            Self::EmptyTransfer => "snapshot transfer is empty",
            Self::TransferTooLarge => "snapshot transfer exceeds 64 MiB",
            Self::ChunkCountMismatch => "snapshot chunk count disagrees with payload size",
            Self::WrongTransfer => "snapshot chunk belongs to another transfer",
            Self::ChunkOutOfRange => "snapshot chunk index is outside the transfer",
            Self::ChunkLengthMismatch => "snapshot chunk length is invalid",
            Self::ProgressOutOfRange => "snapshot transfer progress exceeds its totals",
        })
    }
}

impl std::error::Error for SnapshotTransferValidationError {}

fn valid_grouping(value: &str) -> bool {
    value.len() == 19
        && value
            .bytes()
            .enumerate()
            .all(|(index, byte)| (index % 5 == 4) == (byte == b'-'))
}

fn decode_crockford(value: u8) -> Option<u8> {
    let upper = value.to_ascii_uppercase();
    CROCKFORD
        .iter()
        .position(|&candidate| candidate == upper)
        .and_then(|index| u8::try_from(index).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_code_round_trip_is_grouped_and_debug_redacted() {
        let code = OnlineJoinCode::from_bytes([0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
        let shared = code.expose_for_sharing();
        assert_eq!(shared.len(), 19);
        assert_eq!(OnlineJoinCode::parse(&shared), Ok(code));
        assert_eq!(
            OnlineJoinCode::parse(&shared.to_ascii_lowercase()),
            Ok(code)
        );
        assert_eq!(format!("{code:?}"), "OnlineJoinCode([REDACTED])");
        assert!(!format!("{code:?}").contains(&shared));
    }

    #[test]
    fn join_code_rejects_ambiguous_or_misgrouped_input() {
        assert_eq!(
            OnlineJoinCode::parse("0000-0000-0000-000O"),
            Err(OnlineJoinCodeError::InvalidCharacter)
        );
        assert_eq!(
            OnlineJoinCode::parse("00000000-0000-0000"),
            Err(OnlineJoinCodeError::WrongLength)
        );
    }

    #[test]
    fn digest_is_stable_without_disclosing_the_code() {
        let first = OnlineJoinCode::from_bytes([7; ONLINE_JOIN_CODE_RANDOM_BYTES]);
        let same = OnlineJoinCode::from_bytes([7; ONLINE_JOIN_CODE_RANDOM_BYTES]);
        let other = OnlineJoinCode::from_bytes([8; ONLINE_JOIN_CODE_RANDOM_BYTES]);
        assert_eq!(first.digest(), same.digest());
        assert_ne!(first.digest(), other.digest());
        assert!(first.matches(same));
        assert!(!first.matches(other));
    }

    fn header(compressed_bytes: u32) -> SnapshotTransferHeaderV1 {
        let compressed = usize::try_from(compressed_bytes).expect("fixture fits");
        SnapshotTransferHeaderV1 {
            version: SNAPSHOT_TRANSFER_VERSION_V1,
            session_instance_id: SessionInstanceId::from_bytes([1; 16]),
            transfer_id: SnapshotTransferId::from_bytes([2; 16]),
            baseline_sequence: AuthoritySequence(9),
            uncompressed_bytes: compressed_bytes.saturating_mul(2),
            compressed_bytes,
            uncompressed_sha256: [3; 32],
            chunk_count: u32::try_from(compressed.div_ceil(SNAPSHOT_CHUNK_BYTES))
                .expect("fixture chunk count fits"),
        }
    }

    #[test]
    fn transfer_header_and_chunks_enforce_exact_bounds() {
        let header = header(u32::try_from(SNAPSHOT_CHUNK_BYTES + 7).expect("fixture fits"));
        assert_eq!(header.validate(), Ok(()));
        let first = SnapshotChunkV1 {
            transfer_id: header.transfer_id,
            chunk_index: 0,
            payload: BoundedVec::new(vec![0; SNAPSHOT_CHUNK_BYTES]).expect("chunk fits"),
        };
        let last = SnapshotChunkV1 {
            transfer_id: header.transfer_id,
            chunk_index: 1,
            payload: BoundedVec::new(vec![0; 7]).expect("chunk fits"),
        };
        assert_eq!(first.validate_against(header), Ok(()));
        assert_eq!(last.validate_against(header), Ok(()));

        let wrong = SnapshotChunkV1 {
            payload: BoundedVec::new(vec![0; 8]).expect("chunk fits"),
            ..last
        };
        assert_eq!(
            wrong.validate_against(header),
            Err(SnapshotTransferValidationError::ChunkLengthMismatch)
        );
    }

    #[test]
    fn transfer_header_refuses_oversized_allocation_before_chunks() {
        let too_large = u32::try_from(MAX_SNAPSHOT_TRANSFER_BYTES + 1).expect("limit fits u32");
        let mut header = header(1);
        header.uncompressed_bytes = too_large;
        assert_eq!(
            header.validate(),
            Err(SnapshotTransferValidationError::TransferTooLarge)
        );
    }
}
