//! Six-seat lobby projections and transport-neutral admission state.

use std::{collections::BTreeSet, fmt};

use bevy_ecs::prelude::Message;
use hex_core::{PlayerSeat, UnitId};
use serde::{Deserialize, Serialize};

use crate::{
    limits::{BoundError, BoundedText, BoundedVec, MAX_IDENTITY_BYTES, MAX_PARTY_MEMBERS},
    PublicWorldFingerprint, SessionManifestV1,
};

/// Stable session identity for one admitted human, independent of a transport entity id.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SessionPeerId([u8; Self::BYTE_LENGTH]);

impl SessionPeerId {
    /// Identity byte length.
    pub const BYTE_LENGTH: usize = 16;

    /// Constructs a session identity from exact bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; Self::BYTE_LENGTH]) -> Self {
        Self(bytes)
    }

    /// Returns the exact identity bytes.
    #[must_use]
    pub const fn to_bytes(self) -> [u8; Self::BYTE_LENGTH] {
        self.0
    }
}

impl fmt::Debug for SessionPeerId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SessionPeerId(")?;
        for byte in self.0.iter().take(4) {
            write!(formatter, "{byte:02x}")?;
        }
        formatter.write_str("…)")
    }
}

/// Connection state disclosed for one stable human seat.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SeatConnectionState {
    /// No player has claimed the seat.
    #[default]
    Vacant,
    /// Its admitted player currently has one live connection.
    Connected,
    /// Its admitted player disconnected and may reclaim it before delegation.
    Reserved {
        /// Remaining real-time reservation in milliseconds.
        remaining_millis: u32,
    },
    /// The player remains disconnected and the host may temporarily command its units.
    TemporarilyDelegated,
}

impl SeatConnectionState {
    /// Whether a stable player identity owns this seat.
    #[must_use]
    pub const fn is_claimed(self) -> bool {
        !matches!(self, Self::Vacant)
    }

    /// Whether the player is currently connected.
    #[must_use]
    pub const fn is_connected(self) -> bool {
        matches!(self, Self::Connected)
    }
}

/// One seat in a complete lobby projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LobbySeatSnapshot {
    /// Canonical human seat.
    pub seat: PlayerSeat,
    /// Current connection/reservation/delegation state.
    pub connection: SeatConnectionState,
    /// Stable admitted-player identity; absent only for a vacant seat.
    pub player: Option<SessionPeerId>,
    /// Party characters canonically assigned to this seat.
    pub assigned_units: BoundedVec<UnitId, MAX_PARTY_MEMBERS>,
    /// Ready flag, cleared whenever an affected assignment changes.
    pub ready: bool,
}

impl LobbySeatSnapshot {
    /// Constructs an empty seat with its canonical index.
    #[must_use]
    pub fn vacant(seat: PlayerSeat) -> Self {
        Self {
            seat,
            connection: SeatConnectionState::Vacant,
            player: None,
            assigned_units: BoundedVec::default(),
            ready: false,
        }
    }
}

/// Coarse lobby/session lifecycle relevant to admission and presentation.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LobbyPhase {
    /// New invite credentials may claim vacant seats.
    #[default]
    Open,
    /// Launch is frozen while every peer generates and verifies the map.
    Loading,
    /// Gameplay is active; only rotating reconnect credentials are admitted.
    Active,
    /// The encounter reached a terminal result and awaits the host's choice.
    Outcome,
    /// The host closed the session.
    Closed,
}

/// Frozen launch facts shown in the lobby/loading presentation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchSummaryV1 {
    /// Stable built-in scenario identity.
    pub scenario_identity: BoundedText<MAX_IDENTITY_BYTES>,
    /// Expected complete public world digest.
    pub public_world_fingerprint: PublicWorldFingerprint,
    /// Number of claimed human seats at launch.
    pub claimed_seats: u8,
}

/// Complete disclosure-safe six-seat lobby projection.
#[derive(Message, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LobbySnapshot {
    /// Seats in canonical `0..=5` order.
    pub seats: [LobbySeatSnapshot; PlayerSeat::HUMAN_COUNT],
    /// Stable identity of the host, which always owns seat zero.
    pub host_identity: SessionPeerId,
    /// Current admission/lifecycle phase.
    pub phase: LobbyPhase,
    /// Frozen launch summary once launch begins.
    pub launch_summary: Option<LaunchSummaryV1>,
}

impl LobbySnapshot {
    /// Creates an open lobby with the host connected in seat zero and owning every
    /// initially unassigned shipped party member.
    pub fn new(
        host_identity: SessionPeerId,
        manifest: &SessionManifestV1,
    ) -> Result<Self, BoundError> {
        let mut seats = std::array::from_fn(|index| {
            let seat = PlayerSeat::human(u8::try_from(index).unwrap_or(u8::MAX))
                .unwrap_or(PlayerSeat::HOST);
            LobbySeatSnapshot::vacant(seat)
        });
        if let Some(host) = seats.first_mut() {
            host.connection = SeatConnectionState::Connected;
            host.player = Some(host_identity);
            host.assigned_units = BoundedVec::new(
                manifest
                    .shipped_roster
                    .as_slice()
                    .iter()
                    .map(|entry| entry.unit)
                    .collect(),
            )?;
        }
        Ok(Self {
            seats,
            host_identity,
            phase: LobbyPhase::Open,
            launch_summary: None,
        })
    }

    /// Validates canonical seat order, identity/connection coherence, unique party
    /// assignments, and launch-readiness rules against a frozen manifest.
    pub fn validate(&self, manifest: &SessionManifestV1) -> Result<(), LobbyValidationError> {
        let roster = manifest
            .shipped_roster
            .as_slice()
            .iter()
            .map(|entry| entry.unit)
            .collect::<BTreeSet<_>>();
        let mut players = BTreeSet::new();
        let mut assignments = BTreeSet::new();

        for (index, seat) in self.seats.iter().enumerate() {
            if seat.seat.human_index() != Some(index) {
                return Err(LobbyValidationError::NonCanonicalSeatOrder);
            }
            match (seat.connection.is_claimed(), seat.player) {
                (false, None) => {
                    if !seat.assigned_units.is_empty() || seat.ready {
                        return Err(LobbyValidationError::VacantSeatCarriesState(seat.seat));
                    }
                }
                (true, Some(player)) => {
                    if !players.insert(player) {
                        return Err(LobbyValidationError::DuplicatePlayerIdentity);
                    }
                }
                _ => return Err(LobbyValidationError::ConnectionIdentityMismatch(seat.seat)),
            }

            for &unit in seat.assigned_units.as_slice() {
                if !roster.contains(&unit) {
                    return Err(LobbyValidationError::UnknownAssignedUnit(unit));
                }
                if !assignments.insert(unit) {
                    return Err(LobbyValidationError::DuplicateAssignedUnit(unit));
                }
            }
        }

        let host = self
            .seats
            .first()
            .ok_or(LobbyValidationError::MissingHost)?;
        if host.player != Some(self.host_identity) || !host.connection.is_claimed() {
            return Err(LobbyValidationError::MissingHost);
        }
        if host.assigned_units.is_empty() {
            return Err(LobbyValidationError::HostHasNoCharacter);
        }

        for seat in self
            .seats
            .iter()
            .filter(|seat| seat.connection.is_claimed())
        {
            if seat.assigned_units.is_empty() {
                return Err(LobbyValidationError::ClaimedSeatHasNoCharacter(seat.seat));
            }
        }

        if !matches!(self.phase, LobbyPhase::Open) {
            for seat in self
                .seats
                .iter()
                .filter(|seat| seat.connection.is_connected() && seat.seat != PlayerSeat::HOST)
            {
                if !seat.ready {
                    return Err(LobbyValidationError::ConnectedSeatNotReady(seat.seat));
                }
            }
            if assignments != roster {
                return Err(LobbyValidationError::UnassignedRosterUnit);
            }
            if self.launch_summary.is_none() {
                return Err(LobbyValidationError::MissingLaunchSummary);
            }
        }
        Ok(())
    }
}

/// Why a lobby projection violates the six-seat/session contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LobbyValidationError {
    /// Seats are not stored once each in `0..=5` order.
    NonCanonicalSeatOrder,
    /// A seat's claimed/vacant state disagrees with its stable player identity.
    ConnectionIdentityMismatch(PlayerSeat),
    /// A vacant seat retains assignments or readiness.
    VacantSeatCarriesState(PlayerSeat),
    /// One stable player identity appears in multiple seats.
    DuplicatePlayerIdentity,
    /// Seat zero is not claimed by the recorded host identity.
    MissingHost,
    /// The host must always control at least one party member.
    HostHasNoCharacter,
    /// Any claimed non-spectator seat must control at least one party member.
    ClaimedSeatHasNoCharacter(PlayerSeat),
    /// An assignment names a unit outside the frozen shipped roster.
    UnknownAssignedUnit(UnitId),
    /// One party member is assigned to more than one seat.
    DuplicateAssignedUnit(UnitId),
    /// Launch attempted while a connected non-host player was not ready.
    ConnectedSeatNotReady(PlayerSeat),
    /// Launch attempted without assigning every shipped party member.
    UnassignedRosterUnit,
    /// A non-open phase lacks its frozen launch summary.
    MissingLaunchSummary,
}

impl fmt::Display for LobbyValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonCanonicalSeatOrder => "lobby seats are not in canonical order",
            Self::ConnectionIdentityMismatch(_) => {
                "lobby seat connection state disagrees with its player identity"
            }
            Self::VacantSeatCarriesState(_) => "vacant lobby seat carries assignment/readiness",
            Self::DuplicatePlayerIdentity => "one player identity occupies multiple seats",
            Self::MissingHost => "host identity does not own claimed seat zero",
            Self::HostHasNoCharacter => "host has no assigned party member",
            Self::ClaimedSeatHasNoCharacter(_) => "claimed seat has no assigned party member",
            Self::UnknownAssignedUnit(_) => "seat assignment names an unknown roster unit",
            Self::DuplicateAssignedUnit(_) => "party member is assigned to multiple seats",
            Self::ConnectedSeatNotReady(_) => "connected non-host seat is not ready",
            Self::UnassignedRosterUnit => "not every roster unit is assigned at launch",
            Self::MissingLaunchSummary => "launched lobby is missing a launch summary",
        })
    }
}

impl std::error::Error for LobbyValidationError {}

#[cfg(test)]
mod tests {
    use hex_core::{Faction, SimSeeds, TilePos, UnitId};

    use super::*;
    use crate::{
        BuildIdentityV1, ContentFingerprint, MapManifestV1, ProtocolVersion, RosterEntryV1,
        RulesManifestV1, UnitDeploymentV1,
    };

    fn text(value: &str) -> BoundedText<MAX_IDENTITY_BYTES> {
        BoundedText::new(value).expect("fixture identity should fit")
    }

    fn manifest() -> SessionManifestV1 {
        let roster = (0_u64..2)
            .map(|unit| RosterEntryV1 {
                unit: UnitId(unit),
                archetype_identity: text("warrior"),
                character_identity: text(if unit == 0 { "alpha" } else { "beta" }),
                faction: Faction::Player,
            })
            .collect();
        let deployment = (0_u64..2)
            .map(|unit| UnitDeploymentV1 {
                unit: UnitId(unit),
                position: TilePos::ORIGIN,
            })
            .collect();
        SessionManifestV1 {
            protocol: ProtocolVersion::default(),
            build: BuildIdentityV1::new("0.4.0", "fixture").expect("valid fixture build"),
            content_fingerprint: ContentFingerprint(1),
            scenario_identity: text("sandbox"),
            map: MapManifestV1 {
                catalog_identity: text("small"),
                seed: 1,
                generator_identity: text("v3"),
                generator_version: 3,
                expected_public_fingerprint: PublicWorldFingerprint(2),
            },
            rules: RulesManifestV1 {
                profile_identity: text("default"),
                fingerprint: 3,
            },
            shipped_roster: BoundedVec::new(roster).expect("two roster entries fit"),
            deployment: BoundedVec::new(deployment).expect("two deployment entries fit"),
            simulation_seeds: SimSeeds::default(),
        }
    }

    #[test]
    fn new_lobby_assigns_unallocated_party_to_host() {
        let manifest = manifest();
        let host = SessionPeerId::from_bytes([1; 16]);
        let lobby = LobbySnapshot::new(host, &manifest).expect("manifest roster is bounded");
        assert_eq!(lobby.validate(&manifest), Ok(()));
        assert_eq!(
            lobby
                .seats
                .first()
                .map(|seat| seat.assigned_units.as_slice()),
            Some([UnitId(0), UnitId(1)].as_slice())
        );
    }

    #[test]
    fn launch_requires_non_host_readiness_and_complete_unique_assignments() {
        let manifest = manifest();
        let host = SessionPeerId::from_bytes([1; 16]);
        let mut lobby = LobbySnapshot::new(host, &manifest).expect("manifest roster is bounded");
        let host_seat = lobby.seats.first_mut().expect("six-seat array has a host");
        host_seat.assigned_units = BoundedVec::new(vec![UnitId(0)]).expect("one assignment fits");
        let guest = lobby.seats.get_mut(1).expect("six-seat array has seat one");
        guest.connection = SeatConnectionState::Connected;
        guest.player = Some(SessionPeerId::from_bytes([2; 16]));
        guest.assigned_units = BoundedVec::new(vec![UnitId(1)]).expect("one assignment fits");
        lobby.phase = LobbyPhase::Loading;
        lobby.launch_summary = Some(LaunchSummaryV1 {
            scenario_identity: text("sandbox"),
            public_world_fingerprint: PublicWorldFingerprint(2),
            claimed_seats: 2,
        });

        assert_eq!(
            lobby.validate(&manifest),
            Err(LobbyValidationError::ConnectedSeatNotReady(PlayerSeat(1)))
        );
        if let Some(guest) = lobby.seats.get_mut(1) {
            guest.ready = true;
        }
        assert_eq!(lobby.validate(&manifest), Ok(()));
    }
}
