//! Transport-neutral lobby and host-session control requests.

use bevy_ecs::prelude::Message;
use hex_core::{CommandRequestId, PlayerSeat, UnitId};
use serde::{Deserialize, Serialize};

use crate::PublicWorldFingerprint;

/// Seatless lobby mutation a remote client may request.
///
/// The wire payload deliberately cannot name a seat. The authority derives it from the
/// connection's `AuthorizedSessionClient` component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClientLobbyAction {
    /// Change the authenticated guest's ready state while the lobby is open.
    SetReady(bool),
    /// Leave the current session. Open-lobby assignments return to the host; an active
    /// seat follows the ordinary disconnected-seat reservation path.
    Leave,
}

/// One ordered, seatless remote lobby request.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientLobbyRequest {
    /// Source-allocated correlation identity. Lobby requests have a separate result stream
    /// from gameplay commands even though they share the stable id vocabulary.
    pub request_id: CommandRequestId,
    /// The only mutations available to a non-host client.
    pub action: ClientLobbyAction,
}

/// Trusted local action available only inside the listen-host process.
///
/// This type is a Bevy message, not a registered wire message. A remote peer therefore
/// cannot serialize a host-only action or assert a target seat through the protocol.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostSessionControlRequest {
    /// Local correlation identity used by the host UI.
    pub request_id: CommandRequestId,
    /// Host-owned session transition.
    pub action: HostSessionAction,
}

/// Host-only lobby/session transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostSessionAction {
    /// Move one party member to another claimed human seat.
    AssignUnit {
        /// Stable party member identity.
        unit: UnitId,
        /// Claimed destination seat.
        destination: PlayerSeat,
    },
    /// Remove one non-host player from an open lobby.
    Kick {
        /// Non-host seat to remove.
        seat: PlayerSeat,
    },
    /// Freeze admission and begin exact map verification.
    BeginLoading {
        /// Complete host-computed preflight public world fingerprint.
        public_world_fingerprint: PublicWorldFingerprint,
    },
    /// Report the listen host's freshly generated map after entering Loading.
    ReportHostMapReady {
        /// Complete public fingerprint of the regenerated `TerrainReady` world.
        public_world_fingerprint: PublicWorldFingerprint,
    },
    /// Mark the active encounter as having reached its terminal outcome.
    EnterOutcome,
    /// Retry the frozen encounter from its exact manifest and initial deployment.
    RetryExact {
        /// Complete host-computed preflight fingerprint for the frozen world.
        public_world_fingerprint: PublicWorldFingerprint,
    },
    /// Reopen the assignment lobby after an encounter outcome and clear guest readiness.
    ReturnToLobby,
    /// Close the session and invalidate every invitation/reconnect credential.
    CloseSession,
}

/// Disclosure-safe refusal for a lobby/session control request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionControlRefusal {
    /// The physical connection has not completed custom admission.
    NotAuthorized,
    /// The request is valid only in another lobby/session phase.
    WrongPhase,
    /// Admission and assignment mutation have already closed.
    LobbyClosed,
    /// A seat value was not a non-host human seat.
    InvalidSeat,
    /// The requested seat is vacant, disconnected, or otherwise unavailable.
    SeatUnavailable,
    /// Moving the unit would leave a claimed human without a character.
    WouldEmptySeat,
    /// The lobby has no remaining assignable character/seat capacity.
    LobbyFull,
    /// The host world does not match the frozen manifest.
    MapMismatch,
    /// The resulting lobby would violate a frozen launch invariant.
    InvalidLobby,
    /// The authenticated peer exceeded the lobby-request burst budget.
    RateLimited,
}

/// Final typed result of one lobby/session control request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionControlOutcome {
    /// The authority applied the request and, where applicable, published a new lobby.
    Accepted,
    /// The authority refused the request without mutating canonical lobby state.
    Refused(SessionControlRefusal),
}

/// Ordered control result correlated with either a seatless client request or trusted
/// listen-host request.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionControlResult {
    /// Request identity supplied by the local source.
    pub request_id: CommandRequestId,
    /// Accepted or disclosure-safe refused outcome.
    pub outcome: SessionControlOutcome,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_lobby_request_has_no_seat_field() {
        let encoded = serde_json::to_value(ClientLobbyRequest {
            request_id: CommandRequestId(7),
            action: ClientLobbyAction::SetReady(true),
        })
        .expect("fixed lobby request should serialize");

        let object = encoded
            .as_object()
            .expect("request serializes as an object");
        assert_eq!(object.len(), 2);
        assert!(object.contains_key("request_id"));
        assert!(object.contains_key("action"));
        assert!(!object.contains_key("seat"));
    }
}
