//! Ordered wire messages, command bounds, and typed authority outcomes.

use std::{collections::BTreeSet, fmt};

use bevy_app::App;
use bevy_ecs::{
    error::{BevyError, Result as BevyResult},
    prelude::Message,
};
use bevy_replicon::{
    bytes::Bytes,
    postcard_utils,
    prelude::{
        AppRuleExt, Channel, ClientMessageAppExt, ProtocolHash, ProtocolHasher, ServerMessageAppExt,
    },
    shared::message::ctx::{ClientReceiveCtx, ClientSendCtx, ServerReceiveCtx, ServerSendCtx},
};
use hex_core::{
    CommandRequestId, GameCommand, LatticeCoord, PartyPath, PlayerSeat, Sextant, TilePos, UnitId,
};
use serde::{Deserialize, Serialize};

use crate::{
    limits::{
        BoundError, BoundedText, BoundedVec, MAX_ABS_COMMAND_COORDINATE, MAX_ABS_COMMAND_LEVEL,
        MAX_ABS_LATTICE_COORDINATE, MAX_COMMAND_BYTES, MAX_DECISION_CELLS, MAX_IDENTITY_BYTES,
        MAX_PARTY_MEMBERS, MAX_ROUTE_STEPS,
    },
    split_bounded_snapshot, BuildIdentityV1, CampaignSaveStatusV2, ClientLobbyRequest,
    ContentFingerprint, LiveSessionSnapshotV1, LiveSnapshotHeaderV1, LobbySnapshot,
    PlayerKnowledgeSnapshotV1, PublicWorldFingerprint, ReconnectCredential, SessionControlResult,
    SessionManifestV1, SessionPeerId, SessionReplica, UnitReplica, WorldDeltaV1,
    MAX_LIVE_SNAPSHOT_BYTES,
};

/// Project-owned schema material not visible to Replicon's type/order hashing.
pub const PROTOCOL_SCHEMA_TAG: &str =
    "hex-multiplayer/v1;seatless-command-and-lobby;bounded-wire;authorized-projections;session-bound-live-world-v1;ordered-player-knowledge-v1;run-level-liquid-flow;shipped-projection-524288;visible-archetype-v1;explicit-host-map-ready;system-boundary-sequence;explicit-session-launch-kind-v1;campaign-save-status-v2;world-columns-131072";

/// Monotonic ordering assigned by the simulation authority.
#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct AuthoritySequence(pub u64);

/// One command request sent to the authority.
///
/// There is deliberately no seat field. The authority derives the effective seat and any
/// temporary delegation from the authenticated connection.
#[derive(Message, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameCommandRequest {
    /// Idempotence/correlation identity allocated by the human command source.
    pub request_id: CommandRequestId,
    /// Requested domain action.
    pub command: GameCommand,
}

impl GameCommandRequest {
    /// Applies the same structural/domain validation used by the network serializer.
    ///
    /// Local/listen-host ingress calls this too because Replicon's disconnected local
    /// delivery path does not need to serialize the message before re-emitting it.
    pub fn validate(&self) -> Result<(), CommandWireError> {
        BoundedGameCommandRequest::try_from(self).map(|_| ())
    }
}

/// Disclosure-safe reason an authenticated command was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandRefusalReason {
    /// The connection has not completed admission.
    NotAuthorized,
    /// The acting unit does not exist in the authorized view.
    UnknownUnit,
    /// The authenticated seat neither owns nor temporarily delegates the acting unit.
    WrongSeat,
    /// The command is not permitted in the current global mode.
    WrongMode,
    /// Another unit owns the one global turn.
    NotCurrentTurn,
    /// Resolution is waiting for an earlier decision.
    DecisionPending,
    /// The acting unit has work in flight.
    Busy,
    /// A movement route is structurally or legally invalid.
    InvalidPath,
    /// A movement route conflicts with occupied space.
    Occupied,
    /// The requested target is invalid without disclosing hidden target facts.
    InvalidTarget,
    /// A decision answer is malformed or illegal.
    InvalidChoice,
    /// The request exceeded its authenticated burst budget.
    RateLimited,
    /// The decoded request violated a public structural/domain limit.
    Malformed,
}

/// Final idempotent outcome for one request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandOutcome {
    /// Authority accepted and applied the request at the result sequence.
    Accepted,
    /// This request id already reached a final outcome and was not applied again.
    Duplicate {
        /// Sequence of the original final outcome.
        original_sequence: AuthoritySequence,
    },
    /// Authority refused the request without changing simulation truth.
    Refused(CommandRefusalReason),
}

/// Ordered authority response correlated with one command request.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandResult {
    /// Request identity supplied by the command source.
    pub request_id: CommandRequestId,
    /// Authority ordering after the request reached a final outcome.
    pub authority_sequence: AuthoritySequence,
    /// Typed accepted, duplicate, or refusal outcome.
    pub outcome: CommandOutcome,
}

/// Credential presented during custom admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AdmissionCredential {
    /// One-time invitation for a new pre-launch player.
    Invite(crate::InviteToken),
    /// Rotating private credential for an already-admitted player.
    Reconnect(ReconnectCredential),
}

/// First ordered message on every physical client connection.
#[derive(Message, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientHello {
    /// Replicon's order-sensitive protocol hash.
    pub protocol_hash: ProtocolHash,
    /// Exact executable build identity.
    pub build: BuildIdentityV1,
    /// Exact accepted shipped-content revision.
    pub content_fingerprint: ContentFingerprint,
    /// New-admission or reconnect credential.
    pub credential: AdmissionCredential,
}

/// Typed successful admission response, including the next rotating reconnect secret.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdmissionAccepted {
    /// Concrete host session that issued this rotating credential.
    pub session_instance_id: crate::SessionInstanceId,
    /// Canonical human seat derived by the host.
    pub seat: PlayerSeat,
    /// Stable admitted-player identity, never a transport entity id.
    pub player_identity: SessionPeerId,
    /// Credential that replaces the one just presented.
    pub reconnect_credential: ReconnectCredential,
}

/// Typed reason custom admission failed before replication authorization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AdmissionRefusalReason {
    /// Replicon/project protocol identity differs.
    ProtocolMismatch,
    /// Exact executable build identity differs.
    BuildMismatch,
    /// Exact shipped-content fingerprint differs.
    ContentMismatch,
    /// Frozen scenario/map identity differs.
    SessionMismatch,
    /// New admission is closed because launch already began.
    LobbyClosed,
    /// All six human seats are claimed.
    LobbyFull,
    /// Invite credential is invalid, expired, or already consumed.
    InvalidInvite,
    /// Reconnect credential is invalid, expired, or already rotated.
    InvalidReconnect,
    /// The credential's seat already has an active connection.
    DuplicateActiveSeat,
    /// The hello itself violated a public structural bound.
    Malformed,
}

/// Independent pre-authorization response for a rejected connection.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdmissionRefusal {
    /// Stable refusal reason suitable for the Multiplayer screen.
    pub reason: AdmissionRefusalReason,
}

/// Client report after generating the static world from the frozen manifest.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientMapReady {
    /// Complete public fingerprint computed from the resulting `TerrainReady` world.
    pub public_world_fingerprint: PublicWorldFingerprint,
}

/// Why an active multiplayer session returned a client to the Multiplayer screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionCloseReason {
    /// The listen host process/connection ended; there is no host migration.
    HostDisconnected,
    /// The host explicitly closed the session.
    HostClosed,
    /// The host removed this player.
    Kicked,
    /// The peer violated the protocol or security bounds.
    ProtocolViolation,
    /// World fingerprint verification failed.
    MapMismatch,
    /// The encounter/session ended normally.
    SessionEnded,
}

/// Independent typed session termination notification.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionClosed {
    /// Concrete session whose stored reconnect state may be deleted.
    pub session_instance_id: crate::SessionInstanceId,
    /// Stable reason displayed after returning to Multiplayer.
    pub reason: SessionCloseReason,
}

/// Registers every version-1 message and replica in one deterministic order.
///
/// Call only after [`bevy_replicon::prelude::RepliconPlugins`] has initialized its
/// protocol hasher. Host, remote-client, and listen-host builds all execute this same
/// function.
pub fn register_protocol(app: &mut App) {
    app.world_mut()
        .resource_mut::<ProtocolHasher>()
        .add_custom(PROTOCOL_SCHEMA_TAG);

    app.add_client_message::<ClientHello>(Channel::Ordered)
        .add_client_message::<ClientLobbyRequest>(Channel::Ordered)
        .add_client_message_with(
            Channel::Ordered,
            serialize_command_request,
            deserialize_command_request,
        )
        .add_client_message::<ClientMapReady>(Channel::Ordered)
        .add_server_message::<AdmissionAccepted>(Channel::Ordered)
        .make_message_independent::<AdmissionAccepted>()
        .add_server_message::<AdmissionRefusal>(Channel::Ordered)
        .make_message_independent::<AdmissionRefusal>()
        .add_server_message::<CommandResult>(Channel::Ordered)
        .make_message_independent::<CommandResult>()
        .add_server_message::<SessionControlResult>(Channel::Ordered)
        .make_message_independent::<SessionControlResult>()
        .add_server_message::<CampaignSaveStatusV2>(Channel::Ordered)
        .make_message_independent::<CampaignSaveStatusV2>()
        .add_server_message::<SessionManifestV1>(Channel::Ordered)
        .make_message_independent::<SessionManifestV1>()
        .add_server_message::<LobbySnapshot>(Channel::Ordered)
        .make_message_independent::<LobbySnapshot>()
        .add_server_message::<PlayerKnowledgeSnapshotV1>(Channel::Ordered)
        .make_message_independent::<PlayerKnowledgeSnapshotV1>()
        .add_server_message_with(
            Channel::Ordered,
            serialize_live_session_snapshot,
            deserialize_live_session_snapshot,
        )
        .make_message_independent::<LiveSessionSnapshotV1>()
        .add_server_message_with(
            Channel::Ordered,
            serialize_world_delta,
            deserialize_world_delta,
        )
        .make_message_independent::<WorldDeltaV1>()
        .add_server_message::<SessionClosed>(Channel::Ordered)
        .make_message_independent::<SessionClosed>()
        .replicate::<UnitReplica>()
        .replicate::<SessionReplica>();
}

fn serialize_live_session_snapshot(
    _context: &mut ServerSendCtx,
    snapshot: &LiveSessionSnapshotV1,
    message: &mut Vec<u8>,
) -> BevyResult<()> {
    let start = message.len();
    let header_end = start
        .checked_add(LiveSnapshotHeaderV1::ENCODED_BYTES)
        .ok_or_else(|| BevyError::error(SnapshotWireError::HeaderBounds))?;
    message.resize(header_end, 0);
    postcard_utils::to_extend_mut(snapshot, message)?;
    let payload_bytes = message.len().saturating_sub(header_end);
    let header = match LiveSnapshotHeaderV1::new(snapshot.baseline_sequence, payload_bytes) {
        Ok(header) => header,
        Err(error) => {
            message.truncate(start);
            return Err(BevyError::error(error));
        }
    };
    if let Err(error) = snapshot.validate_with_header(header) {
        message.truncate(start);
        return Err(BevyError::error(error));
    }
    let header_slot = message
        .get_mut(start..header_end)
        .ok_or_else(|| BevyError::error(SnapshotWireError::HeaderBounds))?;
    header_slot.copy_from_slice(&header.encode());
    Ok(())
}

fn deserialize_live_session_snapshot(
    _context: &mut ClientReceiveCtx,
    message: &mut Bytes,
) -> BevyResult<LiveSessionSnapshotV1> {
    let (header, _payload) = split_bounded_snapshot(message.as_ref()).map_err(BevyError::error)?;
    let mut payload = message.split_off(LiveSnapshotHeaderV1::ENCODED_BYTES);
    *message = Bytes::new();
    let snapshot: LiveSessionSnapshotV1 = postcard_utils::from_buf(&mut payload)?;
    if !payload.is_empty() {
        return Err(BevyError::error(SnapshotWireError::TrailingData));
    }
    snapshot
        .validate_with_header(header)
        .map_err(BevyError::error)?;
    Ok(snapshot)
}

fn serialize_world_delta(
    _context: &mut ServerSendCtx,
    delta: &WorldDeltaV1,
    message: &mut Vec<u8>,
) -> BevyResult<()> {
    delta.validate().map_err(BevyError::error)?;
    let start = message.len();
    postcard_utils::to_extend_mut(delta, message)?;
    if message.len().saturating_sub(start) > MAX_LIVE_SNAPSHOT_BYTES {
        message.truncate(start);
        return Err(BevyError::error(SnapshotWireError::MessageTooLarge));
    }
    Ok(())
}

fn deserialize_world_delta(
    _context: &mut ClientReceiveCtx,
    message: &mut Bytes,
) -> BevyResult<WorldDeltaV1> {
    if message.len() > MAX_LIVE_SNAPSHOT_BYTES {
        return Err(BevyError::error(SnapshotWireError::MessageTooLarge));
    }
    let delta: WorldDeltaV1 = postcard_utils::from_buf(message)?;
    if !message.is_empty() {
        return Err(BevyError::error(SnapshotWireError::TrailingData));
    }
    delta.validate().map_err(BevyError::error)?;
    Ok(delta)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SnapshotWireError {
    MessageTooLarge,
    TrailingData,
    HeaderBounds,
}

impl fmt::Display for SnapshotWireError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::MessageTooLarge => "world snapshot/delta exceeds the 64 MiB frame cap",
            Self::TrailingData => "world snapshot/delta contains trailing data",
            Self::HeaderBounds => "live snapshot header bounds are invalid",
        })
    }
}

impl std::error::Error for SnapshotWireError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct BoundedPartyPath {
    member: UnitId,
    path: BoundedVec<TilePos, MAX_ROUTE_STEPS>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum BoundedGameCommand {
    MoveAlong {
        unit: UnitId,
        path: BoundedVec<TilePos, MAX_ROUTE_STEPS>,
    },
    MoveParty {
        anchor: UnitId,
        paths: BoundedVec<BoundedPartyPath, MAX_PARTY_MEMBERS>,
    },
    Strike {
        unit: UnitId,
        target: UnitId,
    },
    EndTurn {
        unit: UnitId,
    },
    Cast {
        unit: UnitId,
        spell: BoundedText<MAX_IDENTITY_BYTES>,
        target: TilePos,
        facing: Option<Sextant>,
        mana: Option<u16>,
    },
    Channel {
        unit: UnitId,
    },
    ChooseDisables {
        unit: UnitId,
        cells: BoundedVec<LatticeCoord, MAX_DECISION_CELLS>,
    },
    ChooseRestores {
        unit: UnitId,
        target: UnitId,
        cells: BoundedVec<LatticeCoord, MAX_DECISION_CELLS>,
    },
    Rest {
        unit: UnitId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct BoundedGameCommandRequest {
    request_id: CommandRequestId,
    command: BoundedGameCommand,
}

impl TryFrom<&GameCommandRequest> for BoundedGameCommandRequest {
    type Error = CommandWireError;

    fn try_from(request: &GameCommandRequest) -> Result<Self, Self::Error> {
        let command = match &request.command {
            GameCommand::MoveAlong { unit, path } => BoundedGameCommand::MoveAlong {
                unit: *unit,
                path: bounded_nonempty_path(path)?,
            },
            GameCommand::MoveParty { anchor, paths } => {
                if paths.is_empty() {
                    return Err(CommandWireError::EmptyPartyMove);
                }
                let mut members = BTreeSet::new();
                let paths = paths
                    .iter()
                    .map(|party_path| {
                        if !members.insert(party_path.member) {
                            return Err(CommandWireError::DuplicatePartyMember);
                        }
                        Ok(BoundedPartyPath {
                            member: party_path.member,
                            path: bounded_nonempty_path(&party_path.path)?,
                        })
                    })
                    .collect::<Result<Vec<_>, CommandWireError>>()?;
                BoundedGameCommand::MoveParty {
                    anchor: *anchor,
                    paths: BoundedVec::new(paths)?,
                }
            }
            GameCommand::Strike { unit, target } => BoundedGameCommand::Strike {
                unit: *unit,
                target: *target,
            },
            GameCommand::EndTurn { unit } => BoundedGameCommand::EndTurn { unit: *unit },
            GameCommand::Cast {
                unit,
                spell,
                target,
                facing,
                mana,
            } => {
                validate_position(*target)?;
                BoundedGameCommand::Cast {
                    unit: *unit,
                    spell: BoundedText::new(spell.clone())?,
                    target: *target,
                    facing: *facing,
                    mana: *mana,
                }
            }
            GameCommand::Channel { unit } => BoundedGameCommand::Channel { unit: *unit },
            GameCommand::ChooseDisables { unit, cells } => BoundedGameCommand::ChooseDisables {
                unit: *unit,
                cells: bounded_lattice_cells(cells)?,
            },
            GameCommand::ChooseRestores {
                unit,
                target,
                cells,
            } => BoundedGameCommand::ChooseRestores {
                unit: *unit,
                target: *target,
                cells: bounded_lattice_cells(cells)?,
            },
            GameCommand::Rest { unit } => BoundedGameCommand::Rest { unit: *unit },
        };
        Ok(Self {
            request_id: request.request_id,
            command,
        })
    }
}

impl TryFrom<BoundedGameCommandRequest> for GameCommandRequest {
    type Error = CommandWireError;

    fn try_from(request: BoundedGameCommandRequest) -> Result<Self, Self::Error> {
        let command = match request.command {
            BoundedGameCommand::MoveAlong { unit, path } => {
                validate_nonempty_path(path.as_slice())?;
                GameCommand::MoveAlong {
                    unit,
                    path: path.into_vec(),
                }
            }
            BoundedGameCommand::MoveParty { anchor, paths } => {
                if paths.is_empty() {
                    return Err(CommandWireError::EmptyPartyMove);
                }
                let mut members = BTreeSet::new();
                let paths = paths
                    .into_iter()
                    .map(|party_path| {
                        if !members.insert(party_path.member) {
                            return Err(CommandWireError::DuplicatePartyMember);
                        }
                        validate_nonempty_path(party_path.path.as_slice())?;
                        Ok(PartyPath {
                            member: party_path.member,
                            path: party_path.path.into_vec(),
                        })
                    })
                    .collect::<Result<Vec<_>, CommandWireError>>()?;
                GameCommand::MoveParty { anchor, paths }
            }
            BoundedGameCommand::Strike { unit, target } => GameCommand::Strike { unit, target },
            BoundedGameCommand::EndTurn { unit } => GameCommand::EndTurn { unit },
            BoundedGameCommand::Cast {
                unit,
                spell,
                target,
                facing,
                mana,
            } => {
                validate_position(target)?;
                GameCommand::Cast {
                    unit,
                    spell: spell.into_string(),
                    target,
                    facing,
                    mana,
                }
            }
            BoundedGameCommand::Channel { unit } => GameCommand::Channel { unit },
            BoundedGameCommand::ChooseDisables { unit, cells } => GameCommand::ChooseDisables {
                unit,
                cells: validated_lattice_cells(cells)?,
            },
            BoundedGameCommand::ChooseRestores {
                unit,
                target,
                cells,
            } => GameCommand::ChooseRestores {
                unit,
                target,
                cells: validated_lattice_cells(cells)?,
            },
            BoundedGameCommand::Rest { unit } => GameCommand::Rest { unit },
        };
        Ok(Self {
            request_id: request.request_id,
            command,
        })
    }
}

fn bounded_nonempty_path(
    path: &[TilePos],
) -> Result<BoundedVec<TilePos, MAX_ROUTE_STEPS>, CommandWireError> {
    validate_nonempty_path(path)?;
    BoundedVec::new(path.to_vec()).map_err(CommandWireError::from)
}

fn validate_nonempty_path(path: &[TilePos]) -> Result<(), CommandWireError> {
    if path.is_empty() {
        return Err(CommandWireError::EmptyPath);
    }
    for &position in path {
        validate_position(position)?;
    }
    Ok(())
}

fn validate_position(position: TilePos) -> Result<(), CommandWireError> {
    if position.coord.x().unsigned_abs() > MAX_ABS_COMMAND_COORDINATE
        || position.coord.y().unsigned_abs() > MAX_ABS_COMMAND_COORDINATE
        || position.coord.z().unsigned_abs() > MAX_ABS_COMMAND_COORDINATE
        || position.level.unsigned_abs() > MAX_ABS_COMMAND_LEVEL
    {
        return Err(CommandWireError::PositionOutsideDomain);
    }
    Ok(())
}

fn bounded_lattice_cells(
    cells: &[LatticeCoord],
) -> Result<BoundedVec<LatticeCoord, MAX_DECISION_CELLS>, CommandWireError> {
    let cells = BoundedVec::new(cells.to_vec())?;
    validate_lattice_cells(cells.as_slice())?;
    Ok(cells)
}

fn validated_lattice_cells(
    cells: BoundedVec<LatticeCoord, MAX_DECISION_CELLS>,
) -> Result<Vec<LatticeCoord>, CommandWireError> {
    validate_lattice_cells(cells.as_slice())?;
    Ok(cells.into_vec())
}

fn validate_lattice_cells(cells: &[LatticeCoord]) -> Result<(), CommandWireError> {
    if cells.iter().any(|cell| {
        cell.q().unsigned_abs() > MAX_ABS_LATTICE_COORDINATE
            || cell.r().unsigned_abs() > MAX_ABS_LATTICE_COORDINATE
    }) {
        return Err(CommandWireError::LatticeCellOutsideDomain);
    }
    Ok(())
}

fn serialize_command_request(
    _context: &mut ClientSendCtx,
    request: &GameCommandRequest,
    message: &mut Vec<u8>,
) -> BevyResult<()> {
    let bounded = BoundedGameCommandRequest::try_from(request).map_err(BevyError::error)?;
    let start = message.len();
    postcard_utils::to_extend_mut(&bounded, message)?;
    if message.len().saturating_sub(start) > MAX_COMMAND_BYTES {
        message.truncate(start);
        return Err(BevyError::error(CommandWireError::MessageTooLarge));
    }
    Ok(())
}

fn deserialize_command_request(
    _context: &mut ServerReceiveCtx,
    message: &mut Bytes,
) -> BevyResult<GameCommandRequest> {
    decode_command_request(message)
}

fn decode_command_request(message: &mut Bytes) -> BevyResult<GameCommandRequest> {
    if message.len() > MAX_COMMAND_BYTES {
        return Err(BevyError::error(CommandWireError::MessageTooLarge));
    }
    let bounded: BoundedGameCommandRequest = postcard_utils::from_buf(message)?;
    if !message.is_empty() {
        return Err(BevyError::error(CommandWireError::TrailingData));
    }
    bounded.try_into().map_err(BevyError::error)
}

/// Why a command is rejected at the untrusted wire boundary before the reducer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandWireError {
    /// A bounded text/sequence field exceeded its public limit.
    Bound(BoundError),
    /// The serialized envelope exceeds 64 KiB.
    MessageTooLarge,
    /// A movement route contains no starting surface.
    EmptyPath,
    /// A party movement request contains no members.
    EmptyPartyMove,
    /// A party movement request repeats a stable member id.
    DuplicatePartyMember,
    /// A supplied voxel lies outside the defensive command domain.
    PositionOutsideDomain,
    /// A supplied lattice cell lies outside the defensive lattice domain.
    LatticeCellOutsideDomain,
    /// Bytes remained after one complete command request.
    TrailingData,
}

impl From<BoundError> for CommandWireError {
    fn from(error: BoundError) -> Self {
        Self::Bound(error)
    }
}

impl fmt::Display for CommandWireError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Bound(_) => "command field exceeds a structural bound",
            Self::MessageTooLarge => "serialized command exceeds 64 KiB",
            Self::EmptyPath => "movement path is empty",
            Self::EmptyPartyMove => "party movement contains no members",
            Self::DuplicatePartyMember => "party movement repeats a member",
            Self::PositionOutsideDomain => "command position is outside the accepted domain",
            Self::LatticeCellOutsideDomain => {
                "decision cell is outside the accepted lattice domain"
            }
            Self::TrailingData => "serialized command contains trailing data",
        })
    }
}

impl std::error::Error for CommandWireError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_command_json_cannot_supply_a_seat() {
        let request = GameCommandRequest {
            request_id: CommandRequestId(7),
            command: GameCommand::Rest { unit: UnitId(2) },
        };
        let value = serde_json::to_value(&request).expect("request should serialize");
        let object = value.as_object().expect("request should be an object");
        assert_eq!(object.len(), 2);
        assert!(object.contains_key("request_id"));
        assert!(object.contains_key("command"));
        assert!(!object.contains_key("seat"));

        let attempted = r#"{"request_id":7,"seat":5,"command":{"Rest":{"unit":2}}}"#;
        let decoded = serde_json::from_str::<GameCommandRequest>(attempted);
        assert!(decoded.is_err(), "unknown seat field must fail closed");
    }

    #[test]
    fn bounded_command_conversion_rejects_large_or_empty_paths() {
        let empty = GameCommandRequest {
            request_id: CommandRequestId(1),
            command: GameCommand::MoveAlong {
                unit: UnitId(0),
                path: Vec::new(),
            },
        };
        assert_eq!(
            BoundedGameCommandRequest::try_from(&empty),
            Err(CommandWireError::EmptyPath)
        );

        let oversized = GameCommandRequest {
            request_id: CommandRequestId(2),
            command: GameCommand::MoveAlong {
                unit: UnitId(0),
                path: vec![TilePos::ORIGIN; MAX_ROUTE_STEPS + 1],
            },
        };
        assert!(matches!(
            BoundedGameCommandRequest::try_from(&oversized),
            Err(CommandWireError::Bound(BoundError::TooManyItems { .. }))
        ));

        let invalid_cell = GameCommandRequest {
            request_id: CommandRequestId(3),
            command: GameCommand::ChooseDisables {
                unit: UnitId(0),
                cells: vec![LatticeCoord::new(65, 0)],
            },
        };
        assert_eq!(
            invalid_cell.validate(),
            Err(CommandWireError::LatticeCellOutsideDomain)
        );
    }

    #[test]
    fn arbitrary_auth_and_command_envelopes_fail_closed_without_panicking() {
        for length in 0_usize..512 {
            let arbitrary = (0..length)
                .map(|index| {
                    index
                        .wrapping_mul(31)
                        .wrapping_add(length.wrapping_mul(17))
                        .to_le_bytes()[0]
                })
                .collect::<Vec<_>>();

            let mut auth_bytes = Bytes::from(arbitrary.clone());
            let _auth_result = postcard_utils::from_buf::<ClientHello, _>(&mut auth_bytes);

            let mut command_bytes = Bytes::from(arbitrary);
            let _command_result = decode_command_request(&mut command_bytes);
        }

        let mut oversized = Bytes::from(vec![0_u8; MAX_COMMAND_BYTES.saturating_add(1)]);
        assert!(decode_command_request(&mut oversized).is_err());
    }
}
