//! Authority ordering, idempotence, rate limiting, and safe-boundary vocabulary.

use std::{collections::BTreeMap, collections::VecDeque, fmt, time::Duration};

use bevy_ecs::prelude::Resource;
use hex_core::{CommandRequestId, PlayerSeat};

use crate::{AuthoritySequence, CommandOutcome, CommandResult};

/// Maximum retained final command outcomes for one human seat.
pub const MAX_CACHED_RESULTS_PER_SEAT: usize = 4_096;
/// Maximum unfinished requests accepted concurrently for one human seat.
pub const MAX_IN_FLIGHT_REQUESTS_PER_SEAT: usize = 64;
/// Default accepted request burst in one rate-limit window.
pub const DEFAULT_REQUEST_BURST: u32 = 32;
/// Default fixed request-rate window.
pub const DEFAULT_REQUEST_WINDOW: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy)]
enum RequestState {
    InFlight,
    Final(CommandResult),
}

#[derive(Debug, Default)]
struct SeatRequests {
    requests: BTreeMap<CommandRequestId, RequestState>,
    final_order: VecDeque<CommandRequestId>,
    in_flight: usize,
}

/// Host-side command request sequencer and bounded idempotence cache.
#[derive(Resource, Debug, Default)]
pub struct CommandSequencer {
    last_sequence: AuthoritySequence,
    seats: BTreeMap<PlayerSeat, SeatRequests>,
}

impl CommandSequencer {
    /// Last sequence allocated by the authority.
    #[must_use]
    pub const fn last_sequence(&self) -> AuthoritySequence {
        self.last_sequence
    }

    /// Allocates one sequence for an authoritative boundary with no human request.
    ///
    /// AI turns, world mutation, and other host-system changes still need a unique
    /// projection/delta sequence even though they do not produce a [`CommandResult`].
    /// Callers must invoke this exactly once for one newly published boundary.
    pub fn advance_system_boundary(&mut self) -> Result<AuthoritySequence, SequencerError> {
        self.allocate_sequence()
    }

    /// Begins one request without allocating a sequence or re-enqueueing a retry.
    pub fn begin(
        &mut self,
        seat: PlayerSeat,
        request_id: CommandRequestId,
    ) -> Result<CommandBegin, SequencerError> {
        if !seat.is_human() {
            return Err(SequencerError::NonHumanSeat);
        }
        let existing = self
            .seats
            .get(&seat)
            .and_then(|requests| requests.requests.get(&request_id))
            .copied();
        match existing {
            Some(RequestState::InFlight) => return Ok(CommandBegin::AlreadyInFlight),
            Some(RequestState::Final(original)) => {
                let duplicate_sequence = self.allocate_sequence()?;
                return Ok(CommandBegin::Duplicate(CommandResult {
                    request_id,
                    authority_sequence: duplicate_sequence,
                    outcome: CommandOutcome::Duplicate {
                        original_sequence: original.authority_sequence,
                    },
                }));
            }
            None => {}
        }

        let requests = self.seats.entry(seat).or_default();
        if requests.in_flight >= MAX_IN_FLIGHT_REQUESTS_PER_SEAT {
            return Err(SequencerError::TooManyInFlight);
        }
        requests.requests.insert(request_id, RequestState::InFlight);
        requests.in_flight = requests.in_flight.saturating_add(1);
        Ok(CommandBegin::Enqueue)
    }

    /// Finalizes one previously begun request and caches its accepted/refused result.
    pub fn finish(
        &mut self,
        seat: PlayerSeat,
        request_id: CommandRequestId,
        outcome: CommandOutcome,
    ) -> Result<CommandResult, SequencerError> {
        if matches!(outcome, CommandOutcome::Duplicate { .. }) {
            return Err(SequencerError::DuplicateCannotBeFinalized);
        }
        let state = self
            .seats
            .get(&seat)
            .and_then(|requests| requests.requests.get(&request_id))
            .copied()
            .ok_or(SequencerError::UnknownRequest)?;
        if matches!(state, RequestState::Final(_)) {
            return Err(SequencerError::AlreadyFinal);
        }

        let authority_sequence = self.allocate_sequence()?;
        let result = CommandResult {
            request_id,
            authority_sequence,
            outcome,
        };
        let requests = self
            .seats
            .get_mut(&seat)
            .ok_or(SequencerError::UnknownRequest)?;
        requests
            .requests
            .insert(request_id, RequestState::Final(result));
        requests.in_flight = requests.in_flight.saturating_sub(1);
        requests.final_order.push_back(request_id);
        while requests.final_order.len() > MAX_CACHED_RESULTS_PER_SEAT {
            let Some(expired) = requests.final_order.pop_front() else {
                break;
            };
            if matches!(
                requests.requests.get(&expired),
                Some(RequestState::Final(_))
            ) {
                requests.requests.remove(&expired);
            }
        }
        Ok(result)
    }

    /// Cancels an unfinished request without allocating a result sequence.
    ///
    /// This is reserved for session teardown before the reducer sees the command. Ordinary
    /// refusals are finalized and cached through [`Self::finish`].
    pub fn cancel(
        &mut self,
        seat: PlayerSeat,
        request_id: CommandRequestId,
    ) -> Result<(), SequencerError> {
        let requests = self
            .seats
            .get_mut(&seat)
            .ok_or(SequencerError::UnknownRequest)?;
        match requests.requests.remove(&request_id) {
            Some(RequestState::InFlight) => {
                requests.in_flight = requests.in_flight.saturating_sub(1);
                Ok(())
            }
            Some(RequestState::Final(result)) => {
                requests
                    .requests
                    .insert(request_id, RequestState::Final(result));
                Err(SequencerError::AlreadyFinal)
            }
            None => Err(SequencerError::UnknownRequest),
        }
    }

    /// Number of unfinished commands for one canonical seat.
    #[must_use]
    pub fn in_flight_for(&self, seat: PlayerSeat) -> usize {
        self.seats
            .get(&seat)
            .map_or(0, |requests| requests.in_flight)
    }

    fn allocate_sequence(&mut self) -> Result<AuthoritySequence, SequencerError> {
        let next = self
            .last_sequence
            .0
            .checked_add(1)
            .ok_or(SequencerError::SequenceExhausted)?;
        self.last_sequence = AuthoritySequence(next);
        Ok(self.last_sequence)
    }
}

/// Result of beginning one canonical seat/request-id pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandBegin {
    /// This is the first sighting and may enter authority work exactly once.
    Enqueue,
    /// The first copy is still in flight; this retry is not enqueued.
    AlreadyInFlight,
    /// The original is final; return this ordered duplicate response without enqueueing.
    Duplicate(CommandResult),
}

/// Why request sequencing failed closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequencerError {
    /// Remote human ingress may never use the AI/system seat.
    NonHumanSeat,
    /// The unfinished per-seat cap was reached.
    TooManyInFlight,
    /// A reducer result named no unfinished request.
    UnknownRequest,
    /// The request already has a final cached outcome.
    AlreadyFinal,
    /// A duplicate marker is produced by the sequencer, never finalized by a reducer.
    DuplicateCannotBeFinalized,
    /// The monotonic authority sequence exhausted `u64`.
    SequenceExhausted,
}

impl fmt::Display for SequencerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonHumanSeat => "command ingress used a non-human seat",
            Self::TooManyInFlight => "seat has too many unfinished command requests",
            Self::UnknownRequest => "command result names an unknown request",
            Self::AlreadyFinal => "command request is already final",
            Self::DuplicateCannotBeFinalized => "duplicate outcome cannot come from the reducer",
            Self::SequenceExhausted => "authority sequence is exhausted",
        })
    }
}

impl std::error::Error for SequencerError {}

#[derive(Debug, Clone, Copy)]
struct RateWindow {
    started_at: Duration,
    accepted: u32,
}

/// Deterministic fixed-window request limiter keyed by host-derived seat.
#[derive(Resource, Debug)]
pub struct RequestRateLimiter {
    burst: u32,
    window: Duration,
    seats: BTreeMap<PlayerSeat, RateWindow>,
}

impl Default for RequestRateLimiter {
    fn default() -> Self {
        Self::new(DEFAULT_REQUEST_BURST, DEFAULT_REQUEST_WINDOW)
    }
}

impl RequestRateLimiter {
    /// Creates a limiter with an explicit non-zero burst and time window.
    #[must_use]
    pub fn new(burst: u32, window: Duration) -> Self {
        Self {
            burst: burst.max(1),
            window: window.max(Duration::from_millis(1)),
            seats: BTreeMap::new(),
        }
    }

    /// Attempts to consume one request at monotonic real time since session start.
    pub fn allow(&mut self, seat: PlayerSeat, now: Duration) -> Result<(), RateLimitError> {
        if !seat.is_human() {
            return Err(RateLimitError::NonHumanSeat);
        }
        let current = self.seats.entry(seat).or_insert(RateWindow {
            started_at: now,
            accepted: 0,
        });
        if now < current.started_at || now.saturating_sub(current.started_at) >= self.window {
            *current = RateWindow {
                started_at: now,
                accepted: 0,
            };
        }
        if current.accepted >= self.burst {
            return Err(RateLimitError::BurstExceeded);
        }
        current.accepted = current.accepted.saturating_add(1);
        Ok(())
    }
}

/// Why an authenticated request did not enter sequencing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateLimitError {
    /// Only host-derived human seats have request budgets.
    NonHumanSeat,
    /// The seat exhausted its current fixed-window burst.
    BurstExceeded,
}

impl fmt::Display for RateLimitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonHumanSeat => "rate limiter received a non-human seat",
            Self::BurstExceeded => "seat exceeded its authenticated request burst",
        })
    }
}

impl std::error::Error for RateLimitError {}

/// Host-only work counters that define a safe ownership/delegation boundary.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AuthorityBoundary {
    commands_in_flight: u32,
    decisions_in_flight: u32,
    movements_in_flight: u32,
}

impl AuthorityBoundary {
    /// Whether no command, decision, or movement is in flight.
    #[must_use]
    pub const fn is_quiescent(self) -> bool {
        self.commands_in_flight == 0
            && self.decisions_in_flight == 0
            && self.movements_in_flight == 0
    }

    /// Marks one authority command as in flight.
    pub fn begin_command(&mut self) {
        self.commands_in_flight = self.commands_in_flight.saturating_add(1);
    }

    /// Marks one authority command complete.
    pub fn finish_command(&mut self) -> Result<(), BoundaryError> {
        decrement(&mut self.commands_in_flight)
    }

    /// Marks one defender/restoration decision as in flight.
    pub fn begin_decision(&mut self) {
        self.decisions_in_flight = self.decisions_in_flight.saturating_add(1);
    }

    /// Marks one decision complete.
    pub fn finish_decision(&mut self) -> Result<(), BoundaryError> {
        decrement(&mut self.decisions_in_flight)
    }

    /// Marks one domain movement as in flight.
    pub fn begin_movement(&mut self) {
        self.movements_in_flight = self.movements_in_flight.saturating_add(1);
    }

    /// Marks one domain movement complete.
    pub fn finish_movement(&mut self) -> Result<(), BoundaryError> {
        decrement(&mut self.movements_in_flight)
    }
}

fn decrement(counter: &mut u32) -> Result<(), BoundaryError> {
    let next = counter
        .checked_sub(1)
        .ok_or(BoundaryError::UnbalancedFinish)?;
    *counter = next;
    Ok(())
}

/// A completion signal had no matching begin signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryError {
    /// The corresponding in-flight counter was already zero.
    UnbalancedFinish,
}

impl fmt::Display for BoundaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("authority boundary finish was not paired with a begin")
    }
}

impl std::error::Error for BoundaryError {}

#[cfg(test)]
mod tests {
    use crate::CommandRefusalReason;

    use super::*;

    #[test]
    fn duplicate_retry_never_reenters_authority_work() {
        let mut sequencer = CommandSequencer::default();
        let request = CommandRequestId(7);
        assert_eq!(
            sequencer.begin(PlayerSeat(2), request),
            Ok(CommandBegin::Enqueue)
        );
        assert_eq!(
            sequencer.begin(PlayerSeat(2), request),
            Ok(CommandBegin::AlreadyInFlight)
        );
        let original = sequencer
            .finish(PlayerSeat(2), request, CommandOutcome::Accepted)
            .expect("first request should finish");
        let duplicate = sequencer
            .begin(PlayerSeat(2), request)
            .expect("finished retry should produce a duplicate");
        assert_eq!(
            duplicate,
            CommandBegin::Duplicate(CommandResult {
                request_id: request,
                authority_sequence: AuthoritySequence(2),
                outcome: CommandOutcome::Duplicate {
                    original_sequence: original.authority_sequence,
                },
            })
        );
        assert_eq!(sequencer.in_flight_for(PlayerSeat(2)), 0);
    }

    #[test]
    fn request_identity_is_scoped_by_host_derived_seat() {
        let mut sequencer = CommandSequencer::default();
        let request = CommandRequestId(1);
        assert_eq!(
            sequencer.begin(PlayerSeat(1), request),
            Ok(CommandBegin::Enqueue)
        );
        assert_eq!(
            sequencer.begin(PlayerSeat(2), request),
            Ok(CommandBegin::Enqueue)
        );
        let first = sequencer
            .finish(
                PlayerSeat(1),
                request,
                CommandOutcome::Refused(CommandRefusalReason::WrongMode),
            )
            .expect("seat one should finish");
        let second = sequencer
            .finish(PlayerSeat(2), request, CommandOutcome::Accepted)
            .expect("seat two should finish");
        assert_ne!(first.outcome, second.outcome);
        assert!(first.authority_sequence < second.authority_sequence);
    }

    #[test]
    fn system_boundaries_share_the_command_sequence_without_a_fake_seat() {
        let mut sequencer = CommandSequencer::default();
        let system = sequencer
            .advance_system_boundary()
            .expect("first system boundary should allocate");
        assert_eq!(system, AuthoritySequence(1));

        let request = CommandRequestId(4);
        assert_eq!(
            sequencer.begin(PlayerSeat::HOST, request),
            Ok(CommandBegin::Enqueue)
        );
        let result = sequencer
            .finish(PlayerSeat::HOST, request, CommandOutcome::Accepted)
            .expect("human request should share the sequence");
        assert_eq!(result.authority_sequence, AuthoritySequence(2));
        assert_eq!(sequencer.last_sequence(), AuthoritySequence(2));
    }

    #[test]
    fn cache_and_unfinished_work_are_bounded_per_seat() {
        let mut sequencer = CommandSequencer::default();
        for id in 0..MAX_IN_FLIGHT_REQUESTS_PER_SEAT {
            let id = u64::try_from(id).expect("small test id fits u64");
            assert_eq!(
                sequencer.begin(PlayerSeat::HOST, CommandRequestId(id)),
                Ok(CommandBegin::Enqueue)
            );
        }
        assert_eq!(
            sequencer.begin(PlayerSeat::HOST, CommandRequestId(u64::MAX)),
            Err(SequencerError::TooManyInFlight)
        );

        for id in 0..MAX_IN_FLIGHT_REQUESTS_PER_SEAT {
            let id = u64::try_from(id).expect("small test id fits u64");
            sequencer
                .finish(
                    PlayerSeat::HOST,
                    CommandRequestId(id),
                    CommandOutcome::Accepted,
                )
                .expect("unfinished request should finish");
        }
        for id in MAX_IN_FLIGHT_REQUESTS_PER_SEAT..=MAX_CACHED_RESULTS_PER_SEAT {
            let id = u64::try_from(id).expect("small test id fits u64");
            assert_eq!(
                sequencer.begin(PlayerSeat::HOST, CommandRequestId(id)),
                Ok(CommandBegin::Enqueue)
            );
            sequencer
                .finish(
                    PlayerSeat::HOST,
                    CommandRequestId(id),
                    CommandOutcome::Accepted,
                )
                .expect("new request should finish");
        }
        assert_eq!(
            sequencer.begin(PlayerSeat::HOST, CommandRequestId(0)),
            Ok(CommandBegin::Enqueue),
            "the oldest final result should be evicted once the cap is exceeded"
        );
    }

    #[test]
    fn rate_limit_resets_by_elapsed_real_time_and_is_seat_scoped() {
        let mut limiter = RequestRateLimiter::new(2, Duration::from_secs(1));
        assert_eq!(limiter.allow(PlayerSeat(1), Duration::ZERO), Ok(()));
        assert_eq!(limiter.allow(PlayerSeat(1), Duration::ZERO), Ok(()));
        assert_eq!(
            limiter.allow(PlayerSeat(1), Duration::ZERO),
            Err(RateLimitError::BurstExceeded)
        );
        assert_eq!(limiter.allow(PlayerSeat(2), Duration::ZERO), Ok(()));
        assert_eq!(limiter.allow(PlayerSeat(1), Duration::from_secs(1)), Ok(()));
    }

    #[test]
    fn delegation_reclaim_boundary_includes_all_three_work_domains() {
        let mut boundary = AuthorityBoundary::default();
        assert!(boundary.is_quiescent());
        boundary.begin_command();
        boundary.begin_decision();
        boundary.begin_movement();
        assert!(!boundary.is_quiescent());
        assert_eq!(boundary.finish_command(), Ok(()));
        assert_eq!(boundary.finish_decision(), Ok(()));
        assert!(!boundary.is_quiescent());
        assert_eq!(boundary.finish_movement(), Ok(()));
        assert!(boundary.is_quiescent());
        assert_eq!(
            boundary.finish_movement(),
            Err(BoundaryError::UnbalancedFinish)
        );
    }
}
