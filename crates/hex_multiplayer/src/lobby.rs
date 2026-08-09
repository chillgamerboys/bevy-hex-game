//! Six-seat lobby projections and transport-neutral admission state.

use std::{collections::BTreeSet, fmt};

use bevy_ecs::prelude::Message;
use hex_core::{PlayerSeat, UnitId};
use rand::{rngs::OsRng, RngCore};
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

    /// Generates an identity from the operating system's cryptographic random source.
    #[must_use]
    pub fn generate() -> Self {
        let mut bytes = [0_u8; Self::BYTE_LENGTH];
        OsRng.fill_bytes(&mut bytes);
        Self(bytes)
    }

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
    /// The player reconnected after delegation and is waiting for a safe authority boundary.
    ReclaimPending,
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
        matches!(self, Self::Connected | Self::ReclaimPending)
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
        if host.player != Some(self.host_identity)
            || !matches!(host.connection, SeatConnectionState::Connected)
        {
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

        if matches!(self.phase, LobbyPhase::Open) && self.launch_summary.is_some() {
            return Err(LobbyValidationError::UnexpectedLaunchSummary);
        }

        if matches!(
            self.phase,
            LobbyPhase::Loading | LobbyPhase::Active | LobbyPhase::Outcome
        ) {
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
            let summary = self
                .launch_summary
                .as_ref()
                .ok_or(LobbyValidationError::MissingLaunchSummary)?;
            validate_launch_summary(self, manifest, summary)?;
        } else if let Some(summary) = &self.launch_summary {
            validate_launch_summary(self, manifest, summary)?;
        }
        Ok(())
    }
}

fn validate_launch_summary(
    lobby: &LobbySnapshot,
    manifest: &SessionManifestV1,
    summary: &LaunchSummaryV1,
) -> Result<(), LobbyValidationError> {
    if summary.scenario_identity != manifest.scenario_identity
        || summary.public_world_fingerprint != manifest.map.expected_public_fingerprint
    {
        return Err(LobbyValidationError::LaunchIdentityMismatch);
    }
    let claimed = lobby
        .seats
        .iter()
        .filter(|seat| seat.connection.is_claimed())
        .count();
    if usize::from(summary.claimed_seats) != claimed {
        return Err(LobbyValidationError::ClaimedSeatCountMismatch);
    }
    Ok(())
}

/// Mutable host-owned lobby mechanics behind [`LobbySnapshot`].
///
/// This type never stores transport entity ids or credentials. Admission binds a physical
/// connection to a seat separately, while this authority preserves canonical assignments
/// across disconnects and reconnects.
#[derive(Debug, Clone)]
pub struct LobbyAuthority {
    snapshot: LobbySnapshot,
}

impl LobbyAuthority {
    /// Real-time reservation before host delegation becomes eligible.
    pub const DISCONNECT_RESERVATION_MILLIS: u32 = 30_000;

    /// Creates a host-owned open lobby from a validated frozen manifest.
    pub fn new(
        host_identity: SessionPeerId,
        manifest: &SessionManifestV1,
    ) -> Result<Self, LobbyMutationError> {
        manifest
            .validate()
            .map_err(|_error| LobbyMutationError::InvalidManifest)?;
        let snapshot = LobbySnapshot::new(host_identity, manifest)?;
        Ok(Self { snapshot })
    }

    /// Returns the current disclosure-safe projection.
    #[must_use]
    pub const fn snapshot(&self) -> &LobbySnapshot {
        &self.snapshot
    }

    /// Returns a cloned projection suitable for an ordered server message.
    #[must_use]
    pub fn snapshot_owned(&self) -> LobbySnapshot {
        self.snapshot.clone()
    }

    /// Finds the seat occupied by one stable player identity.
    #[must_use]
    pub fn seat_for_player(&self, player: SessionPeerId) -> Option<PlayerSeat> {
        self.snapshot
            .seats
            .iter()
            .find(|seat| seat.player == Some(player))
            .map(|seat| seat.seat)
    }

    /// Claims the lowest free non-host seat and transfers one host-owned party member.
    ///
    /// Automatic transfer keeps the invariant that every admitted human owns at least one
    /// character. The host always retains one; a lobby with no transferable member is full
    /// even if an unused numeric seat remains.
    pub fn admit_guest(&mut self, player: SessionPeerId) -> Result<PlayerSeat, LobbyMutationError> {
        if self.snapshot.phase != LobbyPhase::Open {
            return Err(LobbyMutationError::LobbyClosed);
        }
        if self.seat_for_player(player).is_some() {
            return Err(LobbyMutationError::DuplicatePlayerIdentity);
        }
        let guest_index = self
            .snapshot
            .seats
            .iter()
            .enumerate()
            .skip(1)
            .find(|(_, seat)| !seat.connection.is_claimed())
            .map(|(index, _)| index)
            .ok_or(LobbyMutationError::LobbyFull)?;

        let mut host_units = self
            .snapshot
            .seats
            .first()
            .map(|seat| seat.assigned_units.as_slice().to_vec())
            .ok_or(LobbyMutationError::MissingHost)?;
        if host_units.len() <= 1 {
            return Err(LobbyMutationError::LobbyFull);
        }
        let transferred = host_units.pop().ok_or(LobbyMutationError::LobbyFull)?;
        let host = self
            .snapshot
            .seats
            .first_mut()
            .ok_or(LobbyMutationError::MissingHost)?;
        host.assigned_units = BoundedVec::new(host_units)?;
        host.ready = false;

        let guest = self
            .snapshot
            .seats
            .get_mut(guest_index)
            .ok_or(LobbyMutationError::LobbyFull)?;
        guest.connection = SeatConnectionState::Connected;
        guest.player = Some(player);
        guest.assigned_units = BoundedVec::new(vec![transferred])?;
        guest.ready = false;
        Ok(guest.seat)
    }

    /// Moves one party member between claimed seats and clears affected readiness.
    pub fn assign_unit(
        &mut self,
        unit: UnitId,
        destination: PlayerSeat,
    ) -> Result<(), LobbyMutationError> {
        if self.snapshot.phase != LobbyPhase::Open {
            return Err(LobbyMutationError::LobbyClosed);
        }
        let destination_index = destination
            .human_index()
            .ok_or(LobbyMutationError::NonHumanSeat)?;
        let destination_seat = self
            .snapshot
            .seats
            .get(destination_index)
            .ok_or(LobbyMutationError::NonHumanSeat)?;
        if !destination_seat.connection.is_claimed() {
            return Err(LobbyMutationError::VacantDestination);
        }
        let source_index = self
            .snapshot
            .seats
            .iter()
            .position(|seat| seat.assigned_units.contains(&unit))
            .ok_or(LobbyMutationError::UnknownAssignedUnit)?;
        if source_index == destination_index {
            return Ok(());
        }

        let mut source_units = self
            .snapshot
            .seats
            .get(source_index)
            .map(|seat| seat.assigned_units.as_slice().to_vec())
            .ok_or(LobbyMutationError::UnknownAssignedUnit)?;
        if source_units.len() <= 1 {
            return Err(LobbyMutationError::WouldEmptyClaimedSeat);
        }
        source_units.retain(|candidate| *candidate != unit);
        let mut destination_units = destination_seat.assigned_units.as_slice().to_vec();
        destination_units.push(unit);
        let destination_units = BoundedVec::new(destination_units)?;
        let source_units = BoundedVec::new(source_units)?;

        if let Some(source) = self.snapshot.seats.get_mut(source_index) {
            source.assigned_units = source_units;
            source.ready = false;
        }
        if let Some(target) = self.snapshot.seats.get_mut(destination_index) {
            target.assigned_units = destination_units;
            target.ready = false;
        }
        Ok(())
    }

    /// Removes one non-host player from an open lobby and returns their assignments to
    /// the host. Active-session removal uses disconnect/delegation instead so canonical
    /// ownership never changes while authority work may be in flight.
    pub fn remove_guest(&mut self, seat: PlayerSeat) -> Result<(), LobbyMutationError> {
        if self.snapshot.phase != LobbyPhase::Open {
            return Err(LobbyMutationError::LobbyClosed);
        }
        if seat == PlayerSeat::HOST {
            return Err(LobbyMutationError::HostCannotBeRemoved);
        }
        let index = seat.human_index().ok_or(LobbyMutationError::NonHumanSeat)?;
        let removed = self
            .snapshot
            .seats
            .get(index)
            .ok_or(LobbyMutationError::NonHumanSeat)?;
        if !removed.connection.is_claimed() {
            return Err(LobbyMutationError::VacantSeat);
        }
        let mut host_units = self
            .snapshot
            .seats
            .first()
            .map(|host| host.assigned_units.as_slice().to_vec())
            .ok_or(LobbyMutationError::MissingHost)?;
        host_units.extend_from_slice(removed.assigned_units.as_slice());
        host_units.sort_unstable();
        let host_units = BoundedVec::new(host_units)?;

        let host = self
            .snapshot
            .seats
            .first_mut()
            .ok_or(LobbyMutationError::MissingHost)?;
        host.assigned_units = host_units;
        host.ready = false;
        let removed = self
            .snapshot
            .seats
            .get_mut(index)
            .ok_or(LobbyMutationError::NonHumanSeat)?;
        *removed = LobbySeatSnapshot::vacant(seat);
        Ok(())
    }

    /// Changes a connected guest's ready state. The host does not need a ready flag.
    pub fn set_ready(&mut self, seat: PlayerSeat, ready: bool) -> Result<(), LobbyMutationError> {
        if self.snapshot.phase != LobbyPhase::Open {
            return Err(LobbyMutationError::LobbyClosed);
        }
        if seat == PlayerSeat::HOST {
            return Err(LobbyMutationError::HostReadinessIsImplicit);
        }
        let entry = self.seat_mut(seat)?;
        if !entry.connection.is_connected() {
            return Err(LobbyMutationError::SeatNotConnected);
        }
        entry.ready = ready;
        Ok(())
    }

    /// Freezes admission and enters map loading after all launch invariants pass.
    pub fn begin_loading(
        &mut self,
        manifest: &SessionManifestV1,
    ) -> Result<(), LobbyMutationError> {
        self.transition_to_loading(manifest, LobbyPhase::Open)
    }

    /// Re-enters loading from an encounter outcome using the same frozen manifest and
    /// assignments. Readiness remains the launch readiness already accepted for this
    /// encounter; disconnected seats may continue through host delegation.
    pub fn retry_loading(
        &mut self,
        manifest: &SessionManifestV1,
    ) -> Result<(), LobbyMutationError> {
        self.transition_to_loading(manifest, LobbyPhase::Outcome)
    }

    fn transition_to_loading(
        &mut self,
        manifest: &SessionManifestV1,
        expected_phase: LobbyPhase,
    ) -> Result<(), LobbyMutationError> {
        if self.snapshot.phase != expected_phase {
            return Err(if expected_phase == LobbyPhase::Open {
                LobbyMutationError::LobbyClosed
            } else {
                LobbyMutationError::WrongPhase
            });
        }
        let previous_summary = self.snapshot.launch_summary.clone();
        let claimed_seats = self
            .snapshot
            .seats
            .iter()
            .filter(|seat| seat.connection.is_claimed())
            .count();
        let claimed_seats =
            u8::try_from(claimed_seats).map_err(|_error| LobbyMutationError::LobbyFull)?;
        self.snapshot.phase = LobbyPhase::Loading;
        self.snapshot.launch_summary = Some(LaunchSummaryV1 {
            scenario_identity: manifest.scenario_identity.clone(),
            public_world_fingerprint: manifest.map.expected_public_fingerprint,
            claimed_seats,
        });
        if let Err(error) = self.snapshot.validate(manifest) {
            self.snapshot.phase = expected_phase;
            self.snapshot.launch_summary = previous_summary;
            return Err(LobbyMutationError::InvalidLobby(error));
        }
        Ok(())
    }

    /// Activates gameplay after every connected peer has verified the generated map.
    pub fn activate(&mut self) -> Result<(), LobbyMutationError> {
        if self.snapshot.phase != LobbyPhase::Loading {
            return Err(LobbyMutationError::WrongPhase);
        }
        self.snapshot.phase = LobbyPhase::Active;
        Ok(())
    }

    /// Marks an encounter outcome while retaining assignments and reconnect eligibility.
    pub fn enter_outcome(&mut self) -> Result<(), LobbyMutationError> {
        if self.snapshot.phase != LobbyPhase::Active {
            return Err(LobbyMutationError::WrongPhase);
        }
        self.snapshot.phase = LobbyPhase::Outcome;
        Ok(())
    }

    /// Reopens assignment after an encounter outcome and clears every guest readiness
    /// flag. Canonical claimed seats and assignments survive the transition.
    pub fn return_to_lobby(&mut self) -> Result<(), LobbyMutationError> {
        if self.snapshot.phase != LobbyPhase::Outcome {
            return Err(LobbyMutationError::WrongPhase);
        }
        self.snapshot.phase = LobbyPhase::Open;
        self.snapshot.launch_summary = None;
        for seat in self.snapshot.seats.iter_mut().skip(1) {
            seat.ready = false;
        }
        Ok(())
    }

    /// Closes admission and session mechanics.
    pub fn close(&mut self) {
        self.snapshot.phase = LobbyPhase::Closed;
    }

    /// Reserves a disconnected non-host seat without changing assignments.
    pub fn disconnect(&mut self, seat: PlayerSeat) -> Result<(), LobbyMutationError> {
        if seat == PlayerSeat::HOST {
            return Err(LobbyMutationError::HostCannotDisconnect);
        }
        let clear_ready = self.snapshot.phase == LobbyPhase::Open;
        let entry = self.seat_mut(seat)?;
        if clear_ready {
            entry.ready = false;
        }
        entry.connection = match entry.connection {
            SeatConnectionState::Connected => SeatConnectionState::Reserved {
                remaining_millis: Self::DISCONNECT_RESERVATION_MILLIS,
            },
            SeatConnectionState::ReclaimPending => SeatConnectionState::TemporarilyDelegated,
            _ => return Err(LobbyMutationError::SeatNotConnected),
        };
        Ok(())
    }

    /// Advances reservation clocks using real elapsed time supplied by the composition root.
    pub fn advance_reservations(&mut self, elapsed_millis: u32) {
        for seat in &mut self.snapshot.seats {
            let SeatConnectionState::Reserved { remaining_millis } = seat.connection else {
                continue;
            };
            seat.connection = if elapsed_millis >= remaining_millis {
                SeatConnectionState::TemporarilyDelegated
            } else {
                SeatConnectionState::Reserved {
                    remaining_millis: remaining_millis - elapsed_millis,
                }
            };
        }
    }

    /// Reconnects a reserved seat, deferring delegation revocation when necessary.
    pub fn reconnect(&mut self, seat: PlayerSeat) -> Result<(), LobbyMutationError> {
        let entry = self.seat_mut(seat)?;
        entry.connection = match entry.connection {
            SeatConnectionState::Reserved { .. } => SeatConnectionState::Connected,
            SeatConnectionState::TemporarilyDelegated => SeatConnectionState::ReclaimPending,
            SeatConnectionState::Connected | SeatConnectionState::ReclaimPending => {
                return Err(LobbyMutationError::DuplicateActiveSeat);
            }
            SeatConnectionState::Vacant => return Err(LobbyMutationError::VacantSeat),
        };
        Ok(())
    }

    /// Revokes all pending host delegations at a quiescent authority boundary.
    pub fn apply_safe_reclaims(&mut self, boundary_is_quiescent: bool) -> usize {
        if !boundary_is_quiescent {
            return 0;
        }
        let mut reclaimed = 0;
        for seat in &mut self.snapshot.seats {
            if seat.connection == SeatConnectionState::ReclaimPending {
                seat.connection = SeatConnectionState::Connected;
                reclaimed += 1;
            }
        }
        reclaimed
    }

    /// Whether the host currently has temporary authority for this canonical seat.
    #[must_use]
    pub fn host_can_delegate(&self, seat: PlayerSeat) -> bool {
        seat.human_index()
            .and_then(|index| self.snapshot.seats.get(index))
            .is_some_and(|entry| {
                matches!(
                    entry.connection,
                    SeatConnectionState::TemporarilyDelegated | SeatConnectionState::ReclaimPending
                )
            })
    }

    /// Whether the canonical player may submit new work from this seat.
    ///
    /// A reconnecting player remains blocked while temporary host delegation is pending
    /// revocation. This prevents host and client work from entering the same authority
    /// boundary before [`Self::apply_safe_reclaims`] observes quiescence.
    #[must_use]
    pub fn player_can_issue_commands(&self, seat: PlayerSeat) -> bool {
        seat.human_index()
            .and_then(|index| self.snapshot.seats.get(index))
            .is_some_and(|entry| entry.connection == SeatConnectionState::Connected)
    }

    fn seat_mut(&mut self, seat: PlayerSeat) -> Result<&mut LobbySeatSnapshot, LobbyMutationError> {
        let index = seat.human_index().ok_or(LobbyMutationError::NonHumanSeat)?;
        self.snapshot
            .seats
            .get_mut(index)
            .ok_or(LobbyMutationError::NonHumanSeat)
    }
}

/// Why a host-owned lobby transition was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LobbyMutationError {
    /// The frozen manifest was invalid.
    InvalidManifest,
    /// The current lobby projection was invalid.
    InvalidLobby(LobbyValidationError),
    /// A bounded assignment container rejected the transition.
    Bound(BoundError),
    /// New admission or assignment changes are closed.
    LobbyClosed,
    /// No numeric seat and transferable character remain.
    LobbyFull,
    /// The host seat is absent or malformed.
    MissingHost,
    /// The stable player identity was already admitted.
    DuplicatePlayerIdentity,
    /// A caller supplied the AI/system seat or another invalid seat.
    NonHumanSeat,
    /// An assignment target is vacant.
    VacantDestination,
    /// The requested unit is not currently assigned.
    UnknownAssignedUnit,
    /// The move would leave a claimed human without a character.
    WouldEmptyClaimedSeat,
    /// Host readiness is implicit and cannot be toggled.
    HostReadinessIsImplicit,
    /// The seat does not have a live connection for this transition.
    SeatNotConnected,
    /// The transition is valid only in another lobby phase.
    WrongPhase,
    /// Host loss terminates the session instead of reserving seat zero.
    HostCannotDisconnect,
    /// Seat zero cannot be removed from its own host-owned lobby.
    HostCannotBeRemoved,
    /// Reconnect targeted a vacant seat.
    VacantSeat,
    /// Reconnect targeted a seat that already has a live connection.
    DuplicateActiveSeat,
}

impl From<BoundError> for LobbyMutationError {
    fn from(error: BoundError) -> Self {
        Self::Bound(error)
    }
}

impl fmt::Display for LobbyMutationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidManifest => "cannot create a lobby from an invalid manifest",
            Self::InvalidLobby(_) => "lobby state does not satisfy launch invariants",
            Self::Bound(_) => "lobby transition exceeded an assignment bound",
            Self::LobbyClosed => "lobby admission and assignment are closed",
            Self::LobbyFull => "lobby has no seat with an assignable character",
            Self::MissingHost => "lobby host seat is missing",
            Self::DuplicatePlayerIdentity => "player identity is already admitted",
            Self::NonHumanSeat => "seat is outside the human range",
            Self::VacantDestination => "assignment destination is vacant",
            Self::UnknownAssignedUnit => "unit is not assigned in this lobby",
            Self::WouldEmptyClaimedSeat => "assignment would leave a player without a character",
            Self::HostReadinessIsImplicit => "the host does not use a ready flag",
            Self::SeatNotConnected => "seat is not connected",
            Self::WrongPhase => "lobby transition is not valid in this phase",
            Self::HostCannotDisconnect => "host loss closes the session",
            Self::HostCannotBeRemoved => "the host cannot be removed from its own lobby",
            Self::VacantSeat => "vacant seat cannot reconnect",
            Self::DuplicateActiveSeat => "seat already has a live connection",
        })
    }
}

impl std::error::Error for LobbyMutationError {}

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
    /// An open lobby unexpectedly carries frozen launch facts.
    UnexpectedLaunchSummary,
    /// Frozen scenario or map identity disagrees with the manifest.
    LaunchIdentityMismatch,
    /// Frozen claimed-seat count disagrees with the six-seat projection.
    ClaimedSeatCountMismatch,
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
            Self::UnexpectedLaunchSummary => "open lobby unexpectedly has a launch summary",
            Self::LaunchIdentityMismatch => "launch summary disagrees with the frozen manifest",
            Self::ClaimedSeatCountMismatch => {
                "launch summary claimed-seat count disagrees with the lobby"
            }
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

    #[test]
    fn admission_uses_lowest_seat_and_preserves_one_character_per_human() {
        let manifest = manifest();
        let host = SessionPeerId::from_bytes([1; 16]);
        let guest = SessionPeerId::from_bytes([2; 16]);
        let mut lobby = LobbyAuthority::new(host, &manifest).expect("valid fixture lobby");

        assert_eq!(lobby.admit_guest(guest), Ok(PlayerSeat(1)));
        let snapshot = lobby.snapshot_owned();
        assert_eq!(
            snapshot.seats.first().map(|seat| seat.assigned_units.len()),
            Some(1)
        );
        assert_eq!(
            snapshot.seats.get(1).map(|seat| seat.assigned_units.len()),
            Some(1)
        );
        assert_eq!(
            lobby.admit_guest(SessionPeerId::from_bytes([3; 16])),
            Err(LobbyMutationError::LobbyFull),
            "host must retain its final character"
        );
        assert_eq!(lobby.snapshot().validate(&manifest), Ok(()));
    }

    #[test]
    fn disconnect_delegation_and_reclaim_wait_for_a_safe_boundary() {
        let manifest = manifest();
        let host = SessionPeerId::from_bytes([1; 16]);
        let guest = SessionPeerId::from_bytes([2; 16]);
        let mut lobby = LobbyAuthority::new(host, &manifest).expect("valid fixture lobby");
        let seat = lobby.admit_guest(guest).expect("guest should fit");

        assert_eq!(lobby.disconnect(seat), Ok(()));
        lobby.advance_reservations(29_999);
        assert!(matches!(
            lobby.snapshot().seats.get(1).map(|seat| seat.connection),
            Some(SeatConnectionState::Reserved {
                remaining_millis: 1
            })
        ));
        lobby.advance_reservations(1);
        assert!(lobby.host_can_delegate(seat));
        assert_eq!(lobby.reconnect(seat), Ok(()));
        assert_eq!(
            lobby.snapshot().seats.get(1).map(|seat| seat.connection),
            Some(SeatConnectionState::ReclaimPending)
        );
        assert!(!lobby.player_can_issue_commands(seat));
        assert_eq!(lobby.apply_safe_reclaims(false), 0);
        assert!(lobby.host_can_delegate(seat));
        assert_eq!(lobby.apply_safe_reclaims(true), 1);
        assert_eq!(
            lobby.snapshot().seats.get(1).map(|seat| seat.connection),
            Some(SeatConnectionState::Connected)
        );
        assert!(!lobby.host_can_delegate(seat));
        assert!(lobby.player_can_issue_commands(seat));
    }

    #[test]
    fn assignment_changes_clear_both_seats_readiness() {
        let mut manifest = manifest();
        let third = RosterEntryV1 {
            unit: UnitId(2),
            archetype_identity: text("warrior"),
            character_identity: text("gamma"),
            faction: Faction::Player,
        };
        let mut roster = manifest.shipped_roster.as_slice().to_vec();
        roster.push(third);
        manifest.shipped_roster = BoundedVec::new(roster).expect("three roster entries fit");
        manifest.deployment = BoundedVec::new(vec![
            UnitDeploymentV1 {
                unit: UnitId(0),
                position: TilePos::ORIGIN,
            },
            UnitDeploymentV1 {
                unit: UnitId(1),
                position: TilePos::ORIGIN,
            },
            UnitDeploymentV1 {
                unit: UnitId(2),
                position: TilePos::ORIGIN,
            },
        ])
        .expect("three deployments fit");
        let mut lobby = LobbyAuthority::new(SessionPeerId::from_bytes([1; 16]), &manifest)
            .expect("valid fixture lobby");
        let guest = lobby
            .admit_guest(SessionPeerId::from_bytes([2; 16]))
            .expect("guest should fit");
        lobby.set_ready(guest, true).expect("guest can ready");

        assert_eq!(lobby.assign_unit(UnitId(0), guest), Ok(()));
        assert_eq!(
            lobby.snapshot().seats.first().map(|seat| seat.ready),
            Some(false)
        );
        assert_eq!(
            lobby.snapshot().seats.get(1).map(|seat| seat.ready),
            Some(false)
        );
        assert_eq!(lobby.snapshot().validate(&manifest), Ok(()));
    }

    #[test]
    fn removing_an_open_lobby_guest_returns_assignments_and_vacates_the_seat() {
        let manifest = manifest();
        let mut lobby = LobbyAuthority::new(SessionPeerId::from_bytes([1; 16]), &manifest)
            .expect("valid fixture lobby");
        let guest = lobby
            .admit_guest(SessionPeerId::from_bytes([2; 16]))
            .expect("guest should fit");

        assert_eq!(lobby.remove_guest(guest), Ok(()));
        assert_eq!(
            lobby
                .snapshot()
                .seats
                .first()
                .map(|seat| seat.assigned_units.as_slice()),
            Some([UnitId(0), UnitId(1)].as_slice())
        );
        assert_eq!(
            lobby.snapshot().seats.get(1).map(|seat| seat.connection),
            Some(SeatConnectionState::Vacant)
        );
        assert_eq!(lobby.snapshot().validate(&manifest), Ok(()));
    }

    #[test]
    fn retry_preserves_launch_readiness_while_return_to_lobby_clears_it() {
        let manifest = manifest();
        let mut lobby = LobbyAuthority::new(SessionPeerId::from_bytes([1; 16]), &manifest)
            .expect("valid fixture lobby");
        let guest = lobby
            .admit_guest(SessionPeerId::from_bytes([2; 16]))
            .expect("guest should fit");
        lobby.set_ready(guest, true).expect("guest can ready");
        lobby.begin_loading(&manifest).expect("lobby can launch");
        lobby.activate().expect("loading can activate");

        lobby
            .disconnect(guest)
            .expect("active guest can disconnect");
        assert_eq!(
            lobby.snapshot().seats.get(1).map(|seat| seat.ready),
            Some(true),
            "an active disconnect must not erase accepted launch readiness"
        );
        lobby
            .reconnect(guest)
            .expect("guest can reclaim before delegation");
        lobby.enter_outcome().expect("active encounter can end");
        lobby
            .retry_loading(&manifest)
            .expect("outcome can retry exactly");
        lobby.activate().expect("retry loading can activate");
        lobby.enter_outcome().expect("retried encounter can end");
        lobby.return_to_lobby().expect("outcome can reopen lobby");

        assert_eq!(lobby.snapshot().phase, LobbyPhase::Open);
        assert_eq!(lobby.snapshot().launch_summary, None);
        assert_eq!(
            lobby.snapshot().seats.get(1).map(|seat| seat.ready),
            Some(false)
        );
        assert_eq!(lobby.snapshot().validate(&manifest), Ok(()));
    }
}
