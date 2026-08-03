//! Deterministic landing choices after terrain withdraws an actor's support.
//!
//! This module chooses one landing from gameplay-visible surface facts. It does not
//! mutate ECS state or combat authority: the integration layer applies actors in
//! stable [`hex_core::UnitId`] order, reserves each accepted destination in
//! [`UnitOccupancy`], and commits the resulting position transactionally.

use std::fmt;

use hex_core::{TilePos, UnitId};

use crate::{Footing, Standing, UnitOccupancy};

/// No legal, unoccupied landing exists for an unsupported actor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoLanding {
    /// The exact support position withdrawn by the terrain change.
    pub from: TilePos,
}

impl fmt::Display for NoLanding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "no legal landing exists for unsupported actor at {:?}",
            self.from
        )
    }
}

impl std::error::Error for NoLanding {}

/// Chooses the deterministic landing for one unsupported actor.
///
/// [`Footing`] has already applied the actor body's traversal profile, substance
/// solidity, exact headroom, and published traversal blockers. `occupancy` supplies
/// current bodies and destinations reserved for actors processed earlier in the same
/// settlement pass. The actor's own old position and stale route reservations are
/// ignored because integration cancels that route when it commits this plan.
///
/// Selection follows the accepted settlement contract exactly:
///
/// 1. the highest unoccupied legal support strictly below `from` in its column;
/// 2. otherwise a different column ordered by hex distance, absolute level
///    difference, whether it is higher, then exact [`TilePos`].
///
/// Settlement is a landing, not an ordinary adjacent step: climb and drop limits do
/// not connect the old support to the answer, while the profile's standability rules
/// still decide whether the actor fits on each candidate.
///
/// # Errors
///
/// Returns [`NoLanding`] when neither search contains a legal unoccupied surface.
pub fn plan_unsupported_actor_landing(
    actor: UnitId,
    from: TilePos,
    footing: &Footing,
    occupancy: &UnitOccupancy,
) -> Result<Standing, NoLanding> {
    if let Some(below) = footing
        .at_coord(from.coord)
        .iter()
        .copied()
        .filter(|candidate| candidate.pos.level < from.level)
        .filter(|candidate| !occupancy.is_occupied(candidate.pos, Some(actor)))
        .max_by_key(|candidate| candidate.pos.level)
    {
        return Ok(below);
    }

    footing
        .standings()
        .into_iter()
        .filter(|candidate| candidate.pos.coord != from.coord)
        .filter(|candidate| !occupancy.is_occupied(candidate.pos, Some(actor)))
        .min_by_key(|candidate| {
            (
                from.coord.distance(candidate.pos.coord),
                from.level.abs_diff(candidate.pos.level),
                candidate.pos.level > from.level,
                candidate.pos,
            )
        })
        .ok_or(NoLanding { from })
}
