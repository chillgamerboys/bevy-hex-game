//! Disclosure-safe exact projections consumed by replica ECS/UI adapters.

use std::fmt;

use bevy_ecs::prelude::Component;
use hex_core::{
    ControlOwner, Faction, Mode, Pause, PendingDecision, PersistentEffect, TilePos, Turn, UnitId,
};
use hex_lattice::LatticeState;
use serde::{Deserialize, Serialize};

use crate::{
    limits::{
        BoundError, BoundedText, BoundedVec, MAX_IDENTITY_BYTES, MAX_ROUTE_STEPS,
        MAX_SESSION_UNITS, MAX_UNIT_EFFECTS,
    },
    AuthoritySequence,
};

/// Bounded shipped archetype identity disclosed with one visible unit.
///
/// This is enough for a replica to materialize the correct actor shell without receiving
/// an encounter's undisclosed hostile roster or any private lattice/AI state.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ArchetypeIdentityV1(BoundedText<MAX_IDENTITY_BYTES>);

impl ArchetypeIdentityV1 {
    /// Validates one stable shipped archetype identity.
    pub fn new(value: impl Into<String>) -> Result<Self, BoundError> {
        BoundedText::new(value).map(Self)
    }

    /// Borrows the validated identity.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Consumes the wrapper and returns the stable identity.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0.into_string()
    }
}

/// Exact authoritative domain route and clock used for client interpolation.
///
/// Floating-point fields are carried as IEEE bit patterns so serialization preserves the
/// authority value exactly. Clients may interpolate transforms but use the discrete
/// [`UnitReplica::position`] for correction and legality presentation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MotionReplicaV1 {
    /// Domain surfaces in route order, including the starting surface.
    pub route: BoundedVec<TilePos, MAX_ROUTE_STEPS>,
    /// `f32::to_bits` of committed domain speed.
    pub speed_bits: u32,
    /// `f64::to_bits` of elapsed authoritative route time.
    pub elapsed_bits: u64,
    /// Whether the route epoch has been established.
    pub started: bool,
    /// Last route step committed as the exact discrete position.
    pub reconciled_step: u32,
}

impl MotionReplicaV1 {
    /// Exact committed speed.
    #[must_use]
    pub const fn speed(&self) -> f32 {
        f32::from_bits(self.speed_bits)
    }

    /// Exact elapsed route time.
    #[must_use]
    pub const fn elapsed(&self) -> f64 {
        f64::from_bits(self.elapsed_bits)
    }

    /// Rejects non-finite clocks, empty routes, and impossible reconciliation indices.
    pub fn validate(&self) -> Result<(), ReplicaValidationError> {
        if self.route.is_empty() {
            return Err(ReplicaValidationError::EmptyMotionRoute);
        }
        if !self.speed().is_finite() || !self.elapsed().is_finite() || self.elapsed() < 0.0 {
            return Err(ReplicaValidationError::InvalidMotionClock);
        }
        let reconciled_step = usize::try_from(self.reconciled_step)
            .map_err(|_conversion_error| ReplicaValidationError::InvalidReconciledStep)?;
        if reconciled_step >= self.route.len() {
            return Err(ReplicaValidationError::InvalidReconciledStep);
        }
        Ok(())
    }
}

/// Exact authorized projection of one unit.
///
/// `lattice` is `None` when the shared player-faction knowledge view does not disclose
/// it. The full host-only combat state is never represented here.
#[derive(Component, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitReplica {
    /// Stable session unit identity.
    pub unit: UnitId,
    /// Shipped archetype identity, disclosed only while this unit itself is visible.
    pub archetype: ArchetypeIdentityV1,
    /// Disclosed faction.
    pub faction: Faction,
    /// Exact authoritative surface position.
    pub position: TilePos,
    /// Exact route state while domain movement is in flight.
    pub motion: Option<MotionReplicaV1>,
    /// Canonical ownership; temporary delegation never changes this field.
    pub owner: ControlOwner,
    /// Authorized battle-mutable lattice state, absent when undisclosed.
    pub lattice: Option<LatticeState>,
    /// Whether the unit is functionally downed.
    pub downed: bool,
    /// Current action/movement budget when this unit owns the global turn.
    pub turn: Option<Turn>,
    /// Authorized persistent effects in stable effect-id order.
    pub effects: BoundedVec<PersistentEffect, MAX_UNIT_EFFECTS>,
}

impl UnitReplica {
    /// Validates canonical faction ownership and exact motion-position coherence.
    pub fn validate(&self) -> Result<(), ReplicaValidationError> {
        match self.faction {
            Faction::Player if !self.owner.0.is_human() => {
                return Err(ReplicaValidationError::InvalidPlayerOwner)
            }
            Faction::Hostile if self.owner.0 != hex_core::PlayerSeat::AI => {
                return Err(ReplicaValidationError::InvalidHostileOwner)
            }
            _ => {}
        }
        if let Some(motion) = &self.motion {
            motion.validate()?;
            let index = usize::try_from(motion.reconciled_step)
                .map_err(|_conversion_error| ReplicaValidationError::InvalidReconciledStep)?;
            if motion.route.get(index).copied() != Some(self.position) {
                return Err(ReplicaValidationError::MotionPositionMismatch);
            }
        }
        Ok(())
    }
}

/// Terminal authoritative encounter projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionOutcome {
    /// At least one player remains and no hostile does.
    Victory,
    /// No player remains active.
    Defeat,
}

/// Exact disclosure-safe session state shared by all co-op seats.
#[derive(Component, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionReplica {
    /// Last authority sequence reflected by this projection.
    pub authority_sequence: AuthoritySequence,
    /// One global exploration/combat tempo.
    pub mode: Mode,
    /// Host-owned global pause state.
    pub pause: Pause,
    /// Stable global initiative order.
    pub initiative: BoundedVec<UnitId, MAX_SESSION_UNITS>,
    /// Unit currently holding the one global turn.
    pub active_turn: Option<UnitId>,
    /// Number of full combat rounds elapsed.
    pub round: u32,
    /// One globally pending defender/restoration decision, if any.
    pub pending_decision: PendingDecision,
    /// Terminal encounter result, if reached.
    pub outcome: Option<SessionOutcome>,
}

impl SessionReplica {
    /// Validates the single global mode/turn/decision shape.
    pub fn validate(&self) -> Result<(), ReplicaValidationError> {
        let mut seen = std::collections::BTreeSet::new();
        for &unit in self.initiative.as_slice() {
            if !seen.insert(unit) {
                return Err(ReplicaValidationError::DuplicateInitiativeUnit(unit));
            }
        }
        if let Some(active) = self.active_turn {
            if !seen.contains(&active) {
                return Err(ReplicaValidationError::ActiveTurnOutsideInitiative(active));
            }
        }
        if self.mode == Mode::Exploring
            && (!self.initiative.is_empty()
                || self.active_turn.is_some()
                || self.pending_decision.is_open())
        {
            return Err(ReplicaValidationError::CombatStateDuringExploration);
        }
        Ok(())
    }
}

/// Why an authorized projection is internally inconsistent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplicaValidationError {
    /// An in-flight route has no starting surface.
    EmptyMotionRoute,
    /// Speed or elapsed time is NaN/infinite, or elapsed time is negative.
    InvalidMotionClock,
    /// The route reconciliation index is outside the route.
    InvalidReconciledStep,
    /// Exact position disagrees with the route's reconciled step.
    MotionPositionMismatch,
    /// A player unit has a non-human canonical owner.
    InvalidPlayerOwner,
    /// A hostile unit is not canonically owned by the AI/system seat.
    InvalidHostileOwner,
    /// The initiative projection repeats a stable unit.
    DuplicateInitiativeUnit(UnitId),
    /// The active turn names a unit outside initiative.
    ActiveTurnOutsideInitiative(UnitId),
    /// Exploration disclosed combat-only turn or decision state.
    CombatStateDuringExploration,
}

impl fmt::Display for ReplicaValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyMotionRoute => "motion route is empty",
            Self::InvalidMotionClock => "motion clock is non-finite or negative",
            Self::InvalidReconciledStep => "motion reconciliation index is invalid",
            Self::MotionPositionMismatch => "motion route disagrees with exact unit position",
            Self::InvalidPlayerOwner => "player replica has a non-human canonical owner",
            Self::InvalidHostileOwner => "hostile replica is not owned by the AI seat",
            Self::DuplicateInitiativeUnit(_) => "initiative repeats a stable unit",
            Self::ActiveTurnOutsideInitiative(_) => "active turn is absent from initiative",
            Self::CombatStateDuringExploration => {
                "exploration replica contains combat-only turn or decision state"
            }
        })
    }
}

impl std::error::Error for ReplicaValidationError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archetype_identity_is_nonempty_bounded_and_control_free() {
        let identity = ArchetypeIdentityV1::new("warrior");
        assert!(identity.is_ok());
        assert_eq!(
            identity.map(|value| value.into_string()),
            Ok("warrior".to_owned())
        );
        assert_eq!(ArchetypeIdentityV1::new(""), Err(BoundError::EmptyText));
        assert_eq!(
            ArchetypeIdentityV1::new("x".repeat(MAX_IDENTITY_BYTES + 1)),
            Err(BoundError::TextTooLong {
                maximum: MAX_IDENTITY_BYTES,
                actual: MAX_IDENTITY_BYTES + 1,
            })
        );
        assert_eq!(
            ArchetypeIdentityV1::new("warrior\nadmin"),
            Err(BoundError::ControlCharacter)
        );
    }
}
