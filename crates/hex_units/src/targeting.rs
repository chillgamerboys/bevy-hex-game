//! Who can be reached from where, and what standing above them is worth.
//!
//! # Range is not distance
//!
//! There is no single number for how far apart two surfaces are, because **reach is
//! directional**. A caster on a clifftop reaches further down than the unit at the
//! bottom reaches back up. Asking "how far apart are they" has two different answers
//! depending on who is asking, so this module exposes a predicate rather than a
//! distance.
//!
//! That matters more than it looks. A symmetric metric combining hexes and levels —
//! `max(horizontal, vertical)`, say — reads as the obvious fix for code that ignores
//! elevation, and it encodes height as a **penalty**: further away, harder to reach.
//! The design wants the opposite. Height is an advantage, and a metric with the sign
//! backwards would have to be unpicked the moment abilities land.
//!
//! # Height buys range
//!
//! > A caster gains **one extra hex of range for every `levels_per_bonus`
//! > levels** it stands above its target — the value comes from
//! > `combat.ron`'s `levels_per_bonus_range` knob, threaded by the caller.
//!
//! Horizontal separation is measured on the hex grid and is unaffected by elevation:
//! climbing does not move you sideways. Height enters only as the bonus.
//!
//! # Reach is not the same as range
//!
//! Melee does **not** use this. An attacker five levels up should not acquire a
//! two-hex punch, so swinging requires the active
//! [`TraversalProfile`](hex_core::TraversalProfile) to admit the adjacent-surface step
//! in both directions. Two rules on purpose: a spell has *range* and gains from
//! elevation; a fist has *reach* and does not.
//!
//! # What this becomes
//!
//! Nothing here knows about spells, because lattices do not exist yet and a spell is a
//! lattice's business. This is the geometry an ability will ask about once there is
//! one, and engagement is simply its first caller.

use hex_core::TilePos;

use crate::Footing;

/// Extra hexes of range a caster at `from` gets for standing above `to`.
///
/// `levels_per_bonus` is `combat.ron`'s `levels_per_bonus_range` knob — this
/// module stays pure geometry and the caller threads the policy. Zero when
/// level with the target or below it. **There is no penalty for being lower**
/// — the low unit simply does not get the bonus, which is not the same thing
/// and keeps the rule to one direction.
#[must_use]
pub fn high_ground_bonus(from: TilePos, to: TilePos, levels_per_bonus: u32) -> u32 {
    // Validation rejects a zero knob; the guard keeps the function total.
    if levels_per_bonus == 0 {
        return 0;
    }
    // `level_step_to` is positive when `to` is *higher*, so the height advantage is
    // its negation. Clamped at zero rather than allowed to go negative: being below
    // costs nothing.
    let levels_above = u32::try_from((-from.level_step_to(to)).max(0)).unwrap_or(0);
    levels_above / levels_per_bonus
}

/// Whether a caster at `from` can reach `to` with an ability of range `base`.
#[must_use]
pub fn in_reach(from: TilePos, to: TilePos, base: u32, levels_per_bonus: u32) -> bool {
    from.coord.distance(to.coord)
        <= base.saturating_add(high_ground_bonus(from, to, levels_per_bonus))
}

/// Whether two *different* occupied surfaces are mutually step-adjacent for touch.
///
/// Touch uses the same exact, body-specific transition graph as melee, but requires
/// the edge in both directions. That makes an ordinary one-level step valid low cover
/// while rejecting cliffs, one-way drops, stacked surfaces, and guessed coordinate
/// adjacency. Self-targeting is an identity rule owned by the caller and deliberately
/// is not folded into this position-only predicate.
#[must_use]
pub fn in_touch_reach(footing: &Footing, from: TilePos, to: TilePos) -> bool {
    from != to && footing.admits_step(from, to) && footing.admits_step(to, from)
}

/// Whether **either** of two units can reach the other at `base` range.
///
/// The one a fight cares about: a unit that can be shot but cannot shoot back is still
/// in a fight. Because the bonus only ever helps whichever unit is higher, this is not
/// two tests — the higher one always has the longer reach, so it decides.
#[must_use]
pub fn either_in_reach(a: TilePos, b: TilePos, base: u32, levels_per_bonus: u32) -> bool {
    in_reach(a, b, base, levels_per_bonus) || in_reach(b, a, base, levels_per_bonus)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::{HexCoord, Level};

    fn at(x: i32, y: i32, z: i32, level: Level) -> TilePos {
        TilePos::new(HexCoord::new_cubic(x, y, z), level)
    }

    /// On the flat, reach is exactly the range and nothing else.
    ///
    /// This is what makes the rule safe to introduce: every existing threshold keeps
    /// the meaning it was tuned with, and only elevation changes anything.
    #[test]
    fn level_ground_reaches_exactly_its_range() {
        let caster = at(0, 0, 0, 4);
        assert!(in_reach(caster, at(4, -4, 0, 4), 4, 5));
        assert!(!in_reach(caster, at(5, -5, 0, 4), 4, 5));
    }

    /// Five levels of height is worth one hex.
    #[test]
    fn height_buys_range() {
        let high = at(0, 0, 0, 9);
        let low = at(5, -5, 0, 4);
        assert!(
            !in_reach(at(0, 0, 0, 4), low, 4, 5),
            "the same shot from level ground should fall short"
        );
        assert!(
            in_reach(high, low, 4, 5),
            "five levels up should buy the hex"
        );
    }

    /// And the unit underneath gains nothing from it.
    ///
    /// The asymmetry **is** the mechanic. A symmetric metric passes a test that only
    /// looks downhill, which is exactly how a metric with the sign backwards would
    /// have survived review.
    #[test]
    fn the_low_ground_gains_nothing() {
        let high = at(0, 0, 0, 9);
        let low = at(5, -5, 0, 4);
        assert!(in_reach(high, low, 4, 5), "downhill reaches");
        assert!(!in_reach(low, high, 4, 5), "uphill does not reach back");
    }

    /// Ordinary terracing is not high ground.
    #[test]
    fn a_single_step_up_is_not_an_advantage() {
        assert_eq!(high_ground_bonus(at(0, 0, 0, 5), at(1, -1, 0, 4), 5), 0);
    }

    /// The bonus accumulates in whole hexes, not fractions of one.
    #[test]
    fn height_pays_in_whole_hexes() {
        let target = at(0, 0, 0, 0);
        assert_eq!(high_ground_bonus(at(0, 0, 0, 4), target, 5), 0);
        assert_eq!(high_ground_bonus(at(0, 0, 0, 5), target, 5), 1);
        assert_eq!(high_ground_bonus(at(0, 0, 0, 9), target, 5), 1);
        assert_eq!(high_ground_bonus(at(0, 0, 0, 10), target, 5), 2);
    }

    /// A fight needs only one side able to act.
    #[test]
    fn either_side_reaching_is_a_fight() {
        let high = at(0, 0, 0, 9);
        let low = at(5, -5, 0, 4);
        assert!(
            either_in_reach(low, high, 4, 5),
            "being shot at is being in a fight"
        );
    }

    /// Two surfaces stacked at one coordinate are not far apart, however tall the
    /// column between them.
    ///
    /// Deliberate, and the opposite of what a symmetric metric would say. Horizontal
    /// separation is zero and a caster directly above its target can target it — that
    /// is the high ground working, not a stack being collapsed by accident.
    #[test]
    fn a_bridge_is_above_the_ground_not_away_from_it() {
        let deck = at(0, 0, 0, 16);
        let below = at(0, 0, 0, 1);
        assert!(in_reach(deck, below, 1, 5));
        assert!(either_in_reach(below, deck, 1, 5));
    }
}
