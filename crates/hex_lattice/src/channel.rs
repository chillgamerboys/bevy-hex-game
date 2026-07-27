//! Channelling: the deterministic burst refill.

use std::collections::BTreeMap;

use hex_core::{ElementId, LatticeCoord};

use crate::spec::{CellKind, LatticeSpec};
use crate::state::{LatticeState, LatticeStats};

/// Refills the lattice's gems — the channel action.
///
/// For each attuned element, in element order, its channelling rate is a budget
/// poured into that element's live gems in [`LatticeCoord`] order, each filled up
/// to its capacity, until the budget is spent. This is the *burst* refill; whether
/// mana also trickles passively is a policy question left open above the engine, so
/// there is no trickle here. Disabled gems are skipped, and so are **locked** gems:
/// an enchantment's cost is capacity — that part of the lattice is spoken for and
/// cannot be channelled — so refilling a locked gem would quietly refund the mana
/// the enchantment tied up and collapse the throughput/capacity distinction.
pub fn channel(state: &mut LatticeState, spec: &LatticeSpec, stats: &LatticeStats) {
    // Group live gems by element. `spec.cells()` yields coordinate order, so each
    // element's gem list is already sorted by `LatticeCoord`.
    let mut by_element: BTreeMap<ElementId, Vec<LatticeCoord>> = BTreeMap::new();
    for (coord, kind) in spec.cells() {
        if let CellKind::Gem { element } = kind {
            if !state.is_disabled(coord) && !state.is_locked(coord) {
                by_element.entry(element).or_default().push(coord);
            }
        }
    }

    // `by_element` iterates in element order; each gem list is in coordinate order.
    for (element, gems) in by_element {
        let capacity = stats.capacity(element);
        let mut budget = stats.channelling(element);
        for coord in gems {
            if budget == 0 {
                break;
            }
            let current = state.mana(coord);
            let room = capacity.saturating_sub(current);
            let added = room.min(budget);
            if added > 0 {
                state.set_mana(coord, current.saturating_add(added));
                budget -= added;
            }
        }
    }
}
