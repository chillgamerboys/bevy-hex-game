//! Persistent effects: the six facts that describe one lasting effect.
//!
//! Damage over time, enchantment upkeep and decaying divination are the same system
//! wearing three hats, so `docs/systems/casting.md` builds it once, around one shape:
//!
//! ```text
//! { source, target, payload, start, end, ticks }
//! ```
//!
//! The vocabulary is here, the runtime is `hex_combat::effects`, and lattice payloads
//! are applied through `hex_lattice`'s existing functions — the same split the command
//! funnel uses. `Burn` is a [payload](EffectPayload::Burn), not a special case: what
//! burning *means* can be redefined without touching the framework, which is the point
//! of having one.
//!
//! # The record is the whole effect
//!
//! Everything needed to decide whether an effect is still running is on the record. That
//! is worth stating because it was briefly built the other way: a burn's remaining turns
//! lived in the target's `hex_lattice::LatticeState` on the theory that a payload's own
//! store should own its countdown, and a second counter beside it would be drift waiting
//! to happen.
//!
//! It was the wrong seam. An effect names a *source*, and the payload's store has no
//! vocabulary for one, so half the effect lived here regardless — two stores describing a
//! single fact between them, which is the drift the split was meant to avoid. Worse, a
//! rules engine has no turn order, so it could not tick at the point the design
//! specifies. `hex_lattice` now holds hexes, mana and enchantments; nothing else.
//!
//! So [`PersistentEffect`]'s `ticks` counts what has happened and [`EffectEnd`] states the
//! condition, and comparing them is a total function of the record. Both `start` and
//! `ticks` only ever go up, which is what makes them safe against a repeated frame.
//! [`EffectEnd::WithEnchantment`] is the one condition that still asks the world, because
//! an enchantment genuinely is somebody else's fact.

use bevy_reflect::prelude::*;
use serde::{Deserialize, Serialize};

use crate::lattice_ids::EnchantId;
use crate::unit_ids::UnitId;

/// A per-session handle to one running persistent effect.
///
/// Allocated from a monotonic counter in the runtime's ledger and never reused within a
/// session, so it is a stable key — and, being ordered, a deterministic tie-break where
/// several effects on one target have to be told apart without consulting entity order.
///
/// **Session-local**, like [`UnitId`]: the counter restarts when a session does, so
/// anything holding one across a teardown is naming a stranger.
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
pub struct EffectId(pub u64);

/// What a persistent effect does while it runs.
///
/// A closed enum, in the same spirit as `hex_assets::Effect`: extension is one variant
/// here plus one arm where payloads are ticked, never a script.
#[derive(Reflect, Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum EffectPayload {
    /// Fire's damage over time: one more of the target's hexes goes down at the
    /// **start of each of the target's own turns**, for as long as the effect runs.
    ///
    /// The tick point is personal rather than global — `docs/systems/casting.md` says so
    /// explicitly, and it is how the design words fire's damage over time. A burn that
    /// ticked on the round boundary would hit a unit that had just acted and one that
    /// had not at the same moment, which is a different mechanic.
    ///
    /// Two properties come from the design and are load-bearing. Burn **ignores
    /// armour** — fire's identity is beating defences by ignoring them rather than
    /// overpowering them — and it is **the same currency as everything else**: it
    /// disables hexes, and the defender still chooses which. Bypassing the defensive
    /// subtraction is not the same thing as bypassing the defender's choice.
    ///
    Burn,
}

/// When a persistent effect stops.
///
/// Every variant is a **question about live state**, not a counter this type
/// decrements. `docs/systems/casting.md` fixes the wave-3 set at "after N rounds, or
/// bound to an enchantment"; area-lingering zones and dispels come later.
#[derive(Reflect, Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum EffectEnd {
    /// Ends after this many of the **target's own turns** have ticked it.
    ///
    /// Counted against [`PersistentEffect`]'s `ticks`, which the runtime increments as it
    /// ticks — so the countdown lives with the effect rather than in whatever the
    /// payload happens to touch. That matters more than it sounds: an earlier design
    /// kept a burn's remaining turns inside the target's `LatticeState` and asked the
    /// lattice whether the effect was still alive, which left the runtime that owns
    /// effects unable to answer the one question that decides their lifetime.
    AfterTurns(u16),
    /// Ends this many rounds after `start`, on the round boundary.
    ///
    /// Derived from the round counter and `start`, so nothing has to be decremented and
    /// a missed frame cannot extend it.
    AfterRounds(u32),
    /// Ends when this enchantment breaks.
    ///
    /// An enchantment breaks when one of its funding gems is disabled, which the lattice
    /// records by dropping it — so "is that enchantment still there" is the whole
    /// condition, and an effect bound to a shield dies with the shield.
    WithEnchantment(EnchantId),
}

/// One lasting effect: who caused it, who carries it, what it does, and when it stops.
///
/// Deliberately plain data with no behaviour. The runtime that owns the ledger decides
/// what ticking a payload means and when an end condition has come, because both
/// answers need the world and this crate cannot see it.
#[derive(Reflect, Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Eq)]
pub struct PersistentEffect {
    /// Who caused it.
    ///
    /// Attribution for the combat log and presentation. **The rules do not read it** —
    /// a burn hurts the same whoever lit it — which is what makes it safe for the
    /// runtime to pick one source when several effects come due together.
    pub source: UnitId,
    /// Who carries it. The unit whose turn ticks a personal payload.
    pub target: UnitId,
    /// What it does.
    pub payload: EffectPayload,
    /// The round it began, counted by the turn order.
    ///
    /// A fact, not a countdown: [`EffectEnd::AfterRounds`] is evaluated against it and
    /// the current round rather than by decrementing anything.
    pub start: u32,
    /// When it stops.
    pub end: EffectEnd,
    /// How many times a personal tick has fired for this effect.
    ///
    /// The countdown [`EffectEnd::AfterTurns`] is measured against, and the only mutable
    /// field here. It is a count of what has happened rather than what is left, for the
    /// same reason `start` is: a number that only goes up cannot be double-decremented
    /// by a repeated frame, and comparing it to the end condition is a total function of
    /// two facts.
    pub ticks: u16,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ledger keys on this, and several effects on one target are told apart by it
    /// rather than by entity order — so the ordering has to be the allocation order.
    #[test]
    fn effect_ids_order_by_allocation() {
        let mut ids = [EffectId(2), EffectId(0), EffectId(1)];
        ids.sort_unstable();
        assert_eq!(ids, [EffectId(0), EffectId(1), EffectId(2)]);
    }

    /// The whole shape round-trips, because a save has to carry an effect that is still
    /// running — an effect dropped at the save boundary is free healing. (serde_json is
    /// hex_core's available dev-dependency; the format is not the point.)
    #[test]
    fn a_persistent_effect_round_trips_through_serde() {
        let effect = PersistentEffect {
            source: UnitId(1),
            target: UnitId(2),
            payload: EffectPayload::Burn,
            start: 3,
            end: EffectEnd::AfterTurns(2),
            ticks: 1,
        };

        let encoded = serde_json::to_string(&effect).expect("a persistent effect should encode");
        let decoded: PersistentEffect =
            serde_json::from_str(&encoded).expect("and decode to what it was");

        assert_eq!(decoded, effect);
    }

    /// Each end condition encodes distinguishably, so a save cannot read a
    /// round-bounded effect back as a turn-bounded one. All three carry the same
    /// integer payload on purpose — that is exactly the collision to rule out.
    #[test]
    fn the_end_conditions_are_distinguishable_on_the_wire() {
        let ends = [
            EffectEnd::AfterTurns(2),
            EffectEnd::AfterRounds(2),
            EffectEnd::WithEnchantment(EnchantId(2)),
        ];
        let encoded: Vec<String> = ends
            .iter()
            .map(|end| serde_json::to_string(end).expect("an end condition should encode"))
            .collect();

        for (index, text) in encoded.iter().enumerate() {
            let decoded: EffectEnd = serde_json::from_str(text).expect("and decode");
            assert_eq!(
                Some(&decoded),
                ends.get(index),
                "every end condition must survive the round trip: {text}"
            );
        }
        assert_eq!(
            encoded.len(),
            encoded
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            "two end conditions encoding identically would be indistinguishable on load"
        );
    }
}
