//! Stable identity for units, and the seams a deterministic sim hangs off.
//!
//! [`UnitId`] exists because `Entity` is not stable: its index is recycled and
//! its bits differ across runs and saves, so anything keyed on it — a turn
//! order, an AI tie-break, a future replay log — silently reshuffles. A `UnitId` is
//! allocated once per unit in scenario spawn order and never reused within a
//! session, which makes it the key saves store, commands name, and every sim
//! tie-break compares.
//!
//! [`PlayerSeat`] and [`ControlOwner`] are the entire future co-op ownership
//! model, one field each: today every unit belongs to seat 0 and the fields
//! are inert, but a command validated against "does this seat own this unit"
//! is the same code in single-player and co-op.
//!
//! [`SimSeeds`] holds the only seeds the sim may ever draw on. Today it has
//! **no reader**: the sim contains no randomness at all — resolution takes no
//! RNG *by signature* — and the standing rule is that a sim decision may never
//! key on entity order, entity bits, or query iteration order. The resource
//! exists so that when AI flavor or cosmetic variation wants randomness later,
//! there is exactly one seeded, save-visible place to get it from.

use bevy_ecs::prelude::*;
use bevy_reflect::prelude::*;
use serde::{Deserialize, Serialize};

/// A unit's stable identity for the whole session.
///
/// Allocated in scenario spawn order by `hex_units`' allocator, carried as a
/// component, and resolved back to an [`Entity`] through the registry. Ordered,
/// so it can break sim ties deterministically where an entity id must not.
#[derive(
    Component,
    Reflect,
    Serialize,
    Deserialize,
    Debug,
    Copy,
    Clone,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
)]
#[reflect(Component)]
pub struct UnitId(pub u64);

/// One player's chair at the table.
///
/// Human seats are exactly `0..=5`. [`Self::AI`] is reserved for host-owned AI and
/// system commands, so the host's human seat never grants authority over hostile units.
#[derive(
    Reflect,
    Serialize,
    Deserialize,
    Debug,
    Default,
    Copy,
    Clone,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
)]
pub struct PlayerSeat(pub u8);

impl PlayerSeat {
    /// Number of human seats supported by one session.
    pub const HUMAN_COUNT: usize = 6;
    /// First valid human seat and the offline/listen-host default.
    pub const HOST: Self = Self(0);
    /// Last valid human seat.
    pub const LAST_HUMAN: Self = Self(5);
    /// Host-only AI and system command authority.
    pub const AI: Self = Self(u8::MAX);

    /// Constructs one of the six human seats.
    #[must_use]
    pub const fn human(index: u8) -> Option<Self> {
        if index <= Self::LAST_HUMAN.0 {
            Some(Self(index))
        } else {
            None
        }
    }

    /// Whether this is one of the six human seats.
    #[must_use]
    pub const fn is_human(self) -> bool {
        self.0 <= Self::LAST_HUMAN.0
    }

    /// Zero-based human seat index, excluding AI/system authority.
    #[must_use]
    pub const fn human_index(self) -> Option<usize> {
        if self.is_human() {
            Some(self.0 as usize)
        } else {
            None
        }
    }
}

/// Which seat commands a unit.
///
/// Player units default to [`PlayerSeat::HOST`]. Hostile units carry
/// [`PlayerSeat::AI`], assigned by their spawn owner. Temporary disconnect delegation is
/// session authorization and never rewrites this canonical component.
#[derive(
    Component, Reflect, Serialize, Deserialize, Debug, Default, Copy, Clone, PartialEq, Eq,
)]
#[reflect(Component)]
pub struct ControlOwner(pub PlayerSeat);

/// The only randomness the sim is ever allowed to draw on, split by audience.
///
/// Inserted per scenario launch beside the resolved map seed and cleared with
/// the rest of the session state. Deliberately unread today — see the module
/// docs. Seeds are split so a cosmetic effect can never perturb an AI
/// decision by consuming from the same stream.
#[derive(Resource, Reflect, Serialize, Deserialize, Debug, Default, Copy, Clone, PartialEq, Eq)]
#[reflect(Resource)]
pub struct SimSeeds {
    /// Seed for anything that shapes the world's simulation state.
    pub world: u64,
    /// Seed for AI flavor choices that must stay replay-stable.
    pub ai_flavor: u64,
    /// Seed for presentation-only variation; never allowed to touch the sim.
    pub cosmetic: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_and_ai_seats_are_disjoint_and_bounded() {
        assert_eq!(PlayerSeat::default(), PlayerSeat::HOST);
        assert_eq!(PlayerSeat::HUMAN_COUNT, 6);
        for index in 0_u8..=5 {
            let seat = PlayerSeat::human(index).expect("0 through 5 are human seats");
            assert!(seat.is_human());
            assert_eq!(seat.human_index(), Some(usize::from(index)));
        }
        assert_eq!(PlayerSeat::human(6), None);
        assert_eq!(PlayerSeat::human(u8::MAX), None);
        assert!(!PlayerSeat::AI.is_human());
        assert_eq!(PlayerSeat::AI.human_index(), None);
    }
}
