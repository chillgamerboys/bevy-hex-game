//! Private transaction state for host-resolved area and terrain spell effects.

use std::collections::{BTreeMap, VecDeque};

use bevy::prelude::*;
use hex_core::{
    TerrainBatchId, TerrainImpact, TerrainImpactOutcome, TerrainImpactRejection,
    TerrainImpactResult, TilePos, UnitId,
};

/// Typed evidence retained when spell resolution cannot safely continue.
///
/// A frozen transaction never times out or releases optimistically. Tests and
/// diagnostics can inspect this value without scraping a log line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpellResolutionFailure {
    /// The session-local terrain batch counter could not reserve a complete range.
    BatchIdsExhausted {
        /// First id the cast attempted to reserve.
        next: u64,
        /// Number of ids required by the cast.
        count: usize,
    },
    /// A second cast attempted to replace an unresolved transaction.
    OverlappingCast {
        /// Caster whose work could not be installed.
        caster: UnitId,
    },
    /// A cast attempted to commit terrain ids other than its preflighted range.
    BatchReservationMismatch {
        /// First id reserved by the session allocator.
        expected: TerrainBatchId,
        /// First id carried by the malformed transaction, if it carried one.
        received: Option<TerrainBatchId>,
    },
    /// An outcome named no batch in the active transaction.
    ForeignOutcome {
        /// Unrecognized batch id.
        batch: TerrainBatchId,
    },
    /// The world answered one batch more than once.
    DuplicateOutcome {
        /// Repeated batch id.
        batch: TerrainBatchId,
    },
    /// The answer did not match the exact announcement it claimed to resolve.
    InconsistentOutcome {
        /// Exact gameplay announcement retained for correlation.
        expected: TerrainImpact,
        /// Structurally incompatible world answer.
        received: TerrainImpactOutcome,
    },
    /// The world reported that gameplay reused a session-local batch id.
    ReusedBatch {
        /// Rejected batch id.
        batch: TerrainBatchId,
    },
    /// Combat authority disappeared or rejected the transaction boundary.
    AuthorityUnavailable {
        /// Stable diagnostic from the authority seam.
        reason: String,
    },
    /// A snapshotted occupant disappeared before its queued effect could resolve.
    UnitUnavailable {
        /// Stable identity retained by the paid cast.
        unit: UnitId,
    },
    /// Published terrain facts needed for deterministic settlement were unavailable.
    SettlementUnavailable {
        /// Stable diagnostic naming the missing projection.
        reason: String,
    },
    /// Terrain reconciliation found no legal surface for an unsupported actor.
    NoLegalLanding {
        /// Actor that cannot be settled.
        unit: UnitId,
        /// Exact support surface that disappeared.
        origin: TilePos,
    },
}

/// Read-only phase summary for diagnostics and focused headless contracts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpellResolutionStatus {
    /// No cast owns the resolution gate.
    Idle,
    /// One cast still has unit or terrain obligations.
    Pending {
        /// Caster that paid for the active transaction.
        caster: UnitId,
        /// Unit-effect operations not yet synchronously applied or opened as a choice.
        queued_unit_effects: usize,
        /// Terrain batches still awaiting their first valid answer.
        pending_terrain_batches: usize,
    },
    /// Resolution is frozen behind retained typed evidence.
    Frozen(SpellResolutionFailure),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum UnitResolution {
    Disable {
        source: UnitId,
        target: UnitId,
        count: u16,
    },
    Burn {
        source: UnitId,
        target: UnitId,
        turns: u16,
    },
}

#[derive(Debug)]
struct ExpectedImpact {
    impact: TerrainImpact,
    complete: bool,
    settlement_staged: bool,
}

#[derive(Debug)]
struct ActiveResolution {
    caster: UnitId,
    unit_work: VecDeque<UnitResolution>,
    impacts: BTreeMap<TerrainBatchId, ExpectedImpact>,
    settlement_required: bool,
    settlement_adopted: bool,
}

/// Session-local completion authority for one paid spell cast.
#[derive(Resource, Debug, Default)]
pub struct SpellResolutionState {
    next_batch: u64,
    active: Option<ActiveResolution>,
    failure: Option<SpellResolutionFailure>,
}

impl SpellResolutionState {
    /// Returns the current externally observable transaction phase.
    #[must_use]
    pub fn status(&self) -> SpellResolutionStatus {
        if let Some(failure) = &self.failure {
            return SpellResolutionStatus::Frozen(failure.clone());
        }
        let Some(active) = &self.active else {
            return SpellResolutionStatus::Idle;
        };
        SpellResolutionStatus::Pending {
            caster: active.caster,
            queued_unit_effects: active.unit_work.len(),
            pending_terrain_batches: active
                .impacts
                .values()
                .filter(|expected| !expected.complete)
                .count(),
        }
    }

    /// Whether ordinary commands, turn advancement, and disengagement must wait.
    #[must_use]
    pub fn is_blocking(&self) -> bool {
        self.active.is_some() || self.failure.is_some()
    }

    pub(crate) fn preview_batch_ids(
        &self,
        count: usize,
    ) -> Result<Vec<TerrainBatchId>, SpellResolutionFailure> {
        let count_u64 = u64::try_from(count).map_err(|_conversion_error| {
            SpellResolutionFailure::BatchIdsExhausted {
                next: self.next_batch,
                count,
            }
        })?;
        self.next_batch.checked_add(count_u64).ok_or(
            SpellResolutionFailure::BatchIdsExhausted {
                next: self.next_batch,
                count,
            },
        )?;
        Ok((0..count_u64)
            .map(|offset| TerrainBatchId(self.next_batch + offset))
            .collect())
    }

    pub(crate) fn begin(
        &mut self,
        caster: UnitId,
        unit_work: Vec<UnitResolution>,
        impacts: Vec<TerrainImpact>,
    ) -> Result<bool, SpellResolutionFailure> {
        if self.active.is_some() || self.failure.is_some() {
            let failure = SpellResolutionFailure::OverlappingCast { caster };
            self.freeze(failure.clone());
            return Err(failure);
        }
        let expected_ids = self.preview_batch_ids(impacts.len())?;
        if impacts
            .iter()
            .map(|impact| impact.batch)
            .ne(expected_ids.iter().copied())
        {
            let failure = SpellResolutionFailure::BatchReservationMismatch {
                expected: TerrainBatchId(self.next_batch),
                received: impacts.first().map(|impact| impact.batch),
            };
            self.freeze(failure.clone());
            return Err(failure);
        }
        let count = u64::try_from(impacts.len()).map_err(|_conversion_error| {
            SpellResolutionFailure::BatchIdsExhausted {
                next: self.next_batch,
                count: impacts.len(),
            }
        })?;
        self.next_batch = self.next_batch.checked_add(count).ok_or(
            SpellResolutionFailure::BatchIdsExhausted {
                next: self.next_batch,
                count: impacts.len(),
            },
        )?;
        if unit_work.is_empty() && impacts.is_empty() {
            return Ok(false);
        }
        self.active = Some(ActiveResolution {
            caster,
            unit_work: unit_work.into(),
            impacts: impacts
                .into_iter()
                .map(|impact| {
                    (
                        impact.batch,
                        ExpectedImpact {
                            impact,
                            complete: false,
                            settlement_staged: false,
                        },
                    )
                })
                .collect(),
            settlement_required: false,
            settlement_adopted: false,
        });
        Ok(true)
    }

    pub(crate) fn pop_unit_work(&mut self) -> Option<UnitResolution> {
        self.active.as_mut()?.unit_work.pop_front()
    }

    pub(crate) fn has_unit_work(&self) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| !active.unit_work.is_empty())
    }

    pub(crate) fn needs_terrain_settlement_attempt(&self) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| active.settlement_required && !active.settlement_adopted)
    }

    pub(crate) fn terrain_settlement_required(&self) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| active.settlement_required)
    }

    pub(crate) fn mark_terrain_settlement_adopted(&mut self) {
        if let Some(active) = self.active.as_mut() {
            active.settlement_adopted = true;
        }
    }

    /// Stages only a valid first Applied answer far enough ahead to make its
    /// post-world settlement mandatory. Correlation and completion still happen in
    /// `TerrainSystems::ConsumeOutcomes` through [`Self::accept_outcome`].
    pub(crate) fn stage_outcome_for_settlement(&mut self, outcome: &TerrainImpactOutcome) {
        if self.failure.is_some() {
            return;
        }
        let Some(active) = self.active.as_mut() else {
            return;
        };
        let Some(expected) = active.impacts.get_mut(&outcome.batch) else {
            return;
        };
        if expected.complete
            || expected.settlement_staged
            || !matches!(&outcome.result, TerrainImpactResult::Applied(_))
            || !outcome.is_consistent_with(&expected.impact)
        {
            return;
        }
        expected.settlement_staged = true;
        active.settlement_required = true;
        active.settlement_adopted = false;
    }

    pub(crate) fn accept_outcome(&mut self, outcome: TerrainImpactOutcome) {
        if self.failure.is_some() {
            return;
        }
        let Some(active) = self.active.as_mut() else {
            self.freeze(SpellResolutionFailure::ForeignOutcome {
                batch: outcome.batch,
            });
            return;
        };
        let applied = matches!(&outcome.result, TerrainImpactResult::Applied(_));
        let mut accepted_applied = None;
        let failure = match active.impacts.get_mut(&outcome.batch) {
            None => Some(SpellResolutionFailure::ForeignOutcome {
                batch: outcome.batch,
            }),
            Some(expected) if expected.complete => Some(SpellResolutionFailure::DuplicateOutcome {
                batch: outcome.batch,
            }),
            Some(expected) if !outcome.is_consistent_with(&expected.impact) => {
                Some(SpellResolutionFailure::InconsistentOutcome {
                    expected: expected.impact.clone(),
                    received: outcome,
                })
            }
            Some(_)
                if matches!(
                    outcome.result,
                    TerrainImpactResult::Rejected(TerrainImpactRejection::ReusedBatch)
                ) =>
            {
                Some(SpellResolutionFailure::ReusedBatch {
                    batch: outcome.batch,
                })
            }
            Some(expected) => {
                expected.complete = true;
                if applied {
                    accepted_applied = Some(expected.settlement_staged);
                    expected.settlement_staged = false;
                }
                None
            }
        };
        if let Some(was_staged) = accepted_applied {
            active.settlement_required = true;
            if !was_staged {
                active.settlement_adopted = false;
            }
        }
        if let Some(failure) = failure {
            self.freeze(failure);
        }
    }

    pub(crate) fn obligations_complete(&self) -> bool {
        self.failure.is_none()
            && self.active.as_ref().is_some_and(|active| {
                active.unit_work.is_empty()
                    && active.impacts.values().all(|expected| expected.complete)
                    && (!active.settlement_required || active.settlement_adopted)
            })
    }

    pub(crate) fn finish(&mut self) {
        if self.obligations_complete() {
            self.active = None;
        }
    }

    pub(crate) fn freeze(&mut self, failure: SpellResolutionFailure) {
        if self.failure.is_none() {
            error!("spell resolution frozen: {failure:?}");
            self.failure = Some(failure);
        }
    }

    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use hex_core::{ElementId, TerrainImpactDisposition, TerrainVoxelOutcome};

    use super::*;

    fn impact(batch: u64) -> TerrainImpact {
        TerrainImpact {
            batch: TerrainBatchId(batch),
            volume: vec![TilePos::ORIGIN],
            element: ElementId(0),
            power: 2,
        }
    }

    fn applied(batch: u64) -> TerrainImpactOutcome {
        TerrainImpactOutcome {
            batch: TerrainBatchId(batch),
            result: TerrainImpactResult::Applied(vec![TerrainVoxelOutcome {
                pos: TilePos::ORIGIN,
                disposition: TerrainImpactDisposition::NoMaterial,
                before: None,
                after: None,
                health_before: None,
                health_after: None,
            }]),
        }
    }

    #[test]
    fn terrain_batches_are_monotonic_and_release_only_after_every_answer() {
        let mut state = SpellResolutionState::default();
        assert_eq!(
            state.preview_batch_ids(2).expect("two ids fit"),
            vec![TerrainBatchId(0), TerrainBatchId(1)]
        );
        assert!(state
            .begin(UnitId(4), Vec::new(), vec![impact(0), impact(1)])
            .expect("the first transaction starts"));

        state.accept_outcome(applied(1));
        assert!(!state.obligations_complete());
        state.accept_outcome(applied(0));
        assert!(
            !state.obligations_complete(),
            "an applied answer still owes exact settlement adoption"
        );
        state.mark_terrain_settlement_adopted();
        assert!(state.obligations_complete());
        state.finish();
        assert_eq!(state.status(), SpellResolutionStatus::Idle);
        assert_eq!(
            state.preview_batch_ids(1).expect("the allocator advances"),
            vec![TerrainBatchId(2)]
        );
    }

    #[test]
    fn valid_unavailable_rejection_completes_but_reused_and_duplicate_freeze() {
        let mut unavailable = SpellResolutionState::default();
        unavailable
            .begin(UnitId(1), Vec::new(), vec![impact(0)])
            .expect("transaction starts");
        unavailable.accept_outcome(TerrainImpactOutcome {
            batch: TerrainBatchId(0),
            result: TerrainImpactResult::Rejected(TerrainImpactRejection::TerrainUnavailable),
        });
        assert!(unavailable.obligations_complete());
        assert!(!unavailable.needs_terrain_settlement_attempt());

        let mut reused = SpellResolutionState::default();
        reused
            .begin(UnitId(1), Vec::new(), vec![impact(0)])
            .expect("transaction starts");
        reused.accept_outcome(TerrainImpactOutcome {
            batch: TerrainBatchId(0),
            result: TerrainImpactResult::Rejected(TerrainImpactRejection::ReusedBatch),
        });
        assert!(matches!(
            reused.status(),
            SpellResolutionStatus::Frozen(SpellResolutionFailure::ReusedBatch {
                batch: TerrainBatchId(0)
            })
        ));

        let mut duplicate = SpellResolutionState::default();
        duplicate
            .begin(UnitId(1), Vec::new(), vec![impact(0)])
            .expect("transaction starts");
        duplicate.accept_outcome(applied(0));
        duplicate.accept_outcome(applied(0));
        assert!(matches!(
            duplicate.status(),
            SpellResolutionStatus::Frozen(SpellResolutionFailure::DuplicateOutcome {
                batch: TerrainBatchId(0)
            })
        ));
    }

    #[test]
    fn each_late_applied_batch_invalidates_an_older_settlement_adoption() {
        let mut state = SpellResolutionState::default();
        state
            .begin(UnitId(1), Vec::new(), vec![impact(0), impact(1)])
            .expect("transaction starts");

        let first = applied(0);
        state.stage_outcome_for_settlement(&first);
        assert!(state.needs_terrain_settlement_attempt());
        state.mark_terrain_settlement_adopted();
        state.accept_outcome(first);
        assert!(!state.needs_terrain_settlement_attempt());

        let second = applied(1);
        state.stage_outcome_for_settlement(&second);
        assert!(
            state.needs_terrain_settlement_attempt(),
            "a later applied batch cannot reuse the earlier projection"
        );
        state.mark_terrain_settlement_adopted();
        state.accept_outcome(second);
        assert!(state.obligations_complete());
    }

    #[test]
    fn foreign_and_structurally_inconsistent_outcomes_retain_evidence() {
        let mut foreign = SpellResolutionState::default();
        foreign.accept_outcome(applied(9));
        assert!(matches!(
            foreign.status(),
            SpellResolutionStatus::Frozen(SpellResolutionFailure::ForeignOutcome {
                batch: TerrainBatchId(9)
            })
        ));

        let mut inconsistent = SpellResolutionState::default();
        inconsistent
            .begin(UnitId(1), Vec::new(), vec![impact(0)])
            .expect("transaction starts");
        let received = TerrainImpactOutcome {
            batch: TerrainBatchId(0),
            result: TerrainImpactResult::Applied(Vec::new()),
        };
        inconsistent.accept_outcome(received.clone());
        assert_eq!(
            inconsistent.status(),
            SpellResolutionStatus::Frozen(SpellResolutionFailure::InconsistentOutcome {
                expected: impact(0),
                received,
            })
        );
    }

    #[test]
    fn unit_work_is_fifo_and_session_reset_restarts_the_allocator() {
        let first = UnitResolution::Disable {
            source: UnitId(3),
            target: UnitId(1),
            count: 2,
        };
        let second = UnitResolution::Burn {
            source: UnitId(3),
            target: UnitId(4),
            turns: 2,
        };
        let mut state = SpellResolutionState::default();
        state
            .begin(
                UnitId(3),
                vec![first.clone(), second.clone()],
                vec![impact(0)],
            )
            .expect("mixed work starts");
        assert_eq!(
            state.status(),
            SpellResolutionStatus::Pending {
                caster: UnitId(3),
                queued_unit_effects: 2,
                pending_terrain_batches: 1,
            }
        );
        assert_eq!(state.pop_unit_work(), Some(first));
        assert_eq!(state.pop_unit_work(), Some(second));
        assert_eq!(state.pop_unit_work(), None);

        state.reset();
        assert_eq!(state.status(), SpellResolutionStatus::Idle);
        assert_eq!(
            state.preview_batch_ids(1).expect("reset allocator fits"),
            vec![TerrainBatchId(0)]
        );
    }
}
