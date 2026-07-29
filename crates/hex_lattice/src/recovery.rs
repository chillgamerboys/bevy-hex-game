//! Between-encounter recovery for one lattice.

use hex_core::LatticeCoord;

use crate::{restore, CellKind, LatticeSpec, LatticeState, LatticeStats};

/// Restores every disabled cell and fills each live, unlocked gem to capacity.
///
/// Active enchantments and their locks are deliberately untouched. A broken
/// enchantment has already been removed by damage and is not recreated here.
/// Returns the exact restored cells and the amount of mana added.
pub fn rest(
    spec: &LatticeSpec,
    stats: &LatticeStats,
    state: &mut LatticeState,
) -> (Vec<LatticeCoord>, u16) {
    let cells: Vec<_> = spec
        .cells()
        .filter(|&(coord, _)| state.is_disabled(coord))
        .map(|(coord, _)| coord)
        .collect();
    restore(state, &cells);

    let mut refilled = 0_u16;
    for (coord, kind) in spec.cells() {
        let CellKind::Gem { element } = kind else {
            continue;
        };
        if state.is_disabled(coord) || state.is_locked(coord) {
            continue;
        }
        let capacity = stats.capacity(element);
        let before = state.mana(coord);
        state.set_mana(coord, capacity);
        refilled = refilled.saturating_add(capacity.saturating_sub(before));
    }
    (cells, refilled)
}
