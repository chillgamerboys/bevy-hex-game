//! Casting: the single legality function, its plan, and the applier.

use std::collections::{BTreeMap, BTreeSet};

use hex_core::{ElementId, LatticeCoord, SpellId};

use crate::spec::{CellKind, LatticeSpec};
use crate::state::{ActiveEnchantment, LatticeState};
use crate::tables::{Casting, Requirement, Tables};

/// The exact plan for a cast: which gems supply how much mana.
///
/// Produced by [`castable`] and consumed by [`apply_cast`], so preview,
/// application, and AI forward-simulation all agree on the mana to the point. The
/// `drains` map records the leaf gems (a fusion resolves to *its* feeder gems), so
/// its values sum to the cast's total mana cost — the basis of conservation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CastPlan {
    /// The spell being cast.
    pub spell: SpellId,
    /// The spell cell it is cast from.
    pub cell: LatticeCoord,
    /// Leaf gem drains: coordinate to mana removed.
    pub drains: BTreeMap<LatticeCoord, u16>,
}

/// Why a cast is illegal — the vocabulary for saying no out loud.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CastBlocked {
    /// The chosen cell does not hold a spell.
    NotASpell,
    /// The spell's own cell is disabled.
    SpellDisabled,
    /// The spell's adjacent element and mana requirements cannot all be met — a
    /// missing element, a disabled feeder, or too little mana. Casting is binary,
    /// so there is no degraded cast.
    Unsatisfiable,
}

/// Decides whether the spell at `cell` can be cast, and if so exactly how.
///
/// Binary: it returns a complete [`CastPlan`] or a [`CastBlocked`] reason, never a
/// partial cast. A requirement is met by pooling as many distinct adjacent live
/// gems and fusions of the right element as it takes to reach its mana total — not
/// one cell alone: a gem contributes up to its own mana, and a fusion contributes
/// any amount its own recipe (scaled by that amount) can resolve from *its*
/// neighbours, recursively and cycle-safe. A fusion never funds more than one
/// requirement — that stays a single dedicated slot, and it is also the cycle guard
/// for fusion chains. A gem funding a spell's own requirement *directly* is the same
/// one-slot-one-source deal, but a gem reached only as a fusion's feeder is not: two
/// sibling fusions that share a neighbouring gem may each draw part of its mana, up
/// to the gem's own total, since the gem itself has no notion of which fusion is
/// asking. The assignment is deterministic: candidates are tried in [`LatticeCoord`]
/// order, greedily filling each requirement before moving to the next, and the first
/// complete assignment wins.
pub fn castable(
    spec: &LatticeSpec,
    state: &LatticeState,
    cell: LatticeCoord,
    tables: &impl Tables,
) -> Result<CastPlan, CastBlocked> {
    let spell = match spec.get(cell) {
        Some(CellKind::Spell { spell }) => spell,
        _ => return Err(CastBlocked::NotASpell),
    };
    if state.is_disabled(cell) {
        return Err(CastBlocked::SpellDisabled);
    }

    let requirements = tables.requirements(spell);
    let mut used = BTreeSet::new();
    let mut drains = BTreeMap::new();
    if satisfy(
        &requirements,
        cell,
        spec,
        state,
        tables,
        &mut used,
        &mut drains,
        false,
    ) {
        Ok(CastPlan {
            spell,
            cell,
            drains,
        })
    } else {
        Err(CastBlocked::Unsatisfiable)
    }
}

/// Applies a cast's plan to the lattice: drains the leaf gems, and for an
/// enchantment ties the drawn mana up and locks its funding gems. Returns whether
/// the plan was applied.
///
/// A plan should come from [`castable`] on the *current* state — the command funnel
/// validates each command immediately before applying it. As a mutation-site guard
/// against a stale plan (a funding gem drained, locked, or disabled since the plan
/// was produced), this rejects such a plan **atomically** — returning `false`
/// without mutating — rather than half-applying it or overwriting an existing lock
/// (which would orphan the earlier enchantment, stranding its mana and leaving it
/// unbreakable). Evocations simply consume; enchantments record their locked mana
/// and mark each funding gem so that disabling one later breaks the enchantment.
pub fn apply_cast(state: &mut LatticeState, plan: &CastPlan, tables: &impl Tables) -> bool {
    let applicable = plan.drains.iter().all(|(&coord, &amount)| {
        state.mana(coord) >= amount && !state.is_locked(coord) && !state.is_disabled(coord)
    });
    if !applicable {
        return false;
    }
    for (&coord, &amount) in &plan.drains {
        state.drain(coord, amount);
    }
    if let Casting::Enchantment { defense } = tables.casting(plan.spell) {
        let locked = plan
            .drains
            .values()
            .copied()
            .fold(0u16, u16::saturating_add);
        let id = state.allocate_enchant();
        state.insert_enchantment(
            id,
            ActiveEnchantment {
                spell: plan.spell,
                cell: plan.cell,
                locked_mana: locked,
                defense,
            },
        );
        for &coord in plan.drains.keys() {
            state.lock(coord, id);
        }
    }
    true
}

/// Tries to satisfy `reqs` from the neighbours of `around`, extending the `used`
/// cell set and the `drains` map. Returns whether a complete assignment was found.
///
/// Each requirement is handed to [`draw`], which pools it from as many distinct
/// adjacent cells as it needs; the rest of the list is what `draw` runs once its own
/// pool is full, so a dead end anywhere downstream unwinds this requirement's
/// choices too rather than leaving a partial draw committed. `inside_fusion` is
/// threaded through unchanged — it says whether `reqs` is a spell's own requirement
/// list (`false`) or a fusion's recipe, scaled for however much of it is being drawn
/// (`true`); see [`draw`] for what that changes about gem sharing.
#[expect(
    clippy::too_many_arguments,
    reason = "the list being worked through (reqs, around), the board it draws from \
              (spec, state, tables), the in-flight assignment (used, drains), and \
              which sharing rule applies (inside_fusion) are each load-bearing; \
              bundling any of them would just move the count into a struct without \
              shrinking what a caller has to supply"
)]
fn satisfy(
    reqs: &[Requirement],
    around: LatticeCoord,
    spec: &LatticeSpec,
    state: &LatticeState,
    tables: &impl Tables,
    used: &mut BTreeSet<LatticeCoord>,
    drains: &mut BTreeMap<LatticeCoord, u16>,
    inside_fusion: bool,
) -> bool {
    let Some((req, rest)) = reqs.split_first() else {
        return true;
    };
    draw(
        req.element,
        req.mana,
        around,
        spec,
        state,
        tables,
        used,
        drains,
        inside_fusion,
        &mut |used, drains| {
            satisfy(
                rest,
                around,
                spec,
                state,
                tables,
                used,
                drains,
                inside_fusion,
            )
        },
    )
}

/// Pools `need` mana of `element` from the neighbours of `around`, then calls
/// `continuation` once the pool is full.
///
/// Unlike a spell's requirement *list* — one distinct cell per entry — a single
/// requirement's mana is not claimed by one cell alone: a gem contributes up to its
/// own mana, and a fusion contributes any amount from `need` down to 1 whose scaled
/// recipe its own neighbours can pay (tried highest first, since a smaller draw is
/// never harder to satisfy than a larger one, so the common case fills in one try).
/// A fusion is never claimed twice — `used` marks it spoken for the moment it
/// contributes anything, which is also the cycle guard for fusion chains. A gem is
/// tracked by remaining mana instead: `drains` accumulates what has already been
/// taken from it, so a later draw only ever sees what is left. `inside_fusion`
/// decides whether a fully-drained gem is *also* placed in `used` — at the top
/// level (`false`) it is, keeping a spell's own requirement slots each backed by a
/// distinct source exactly as before; inside a fusion's own recipe (`true`) it is
/// not, so a gem two sibling fusions both neighbour can fund each of them in turn,
/// bounded only by its own total mana. `continuation` is only invoked once the pool
/// is exactly full, and a `false` from it unwinds this draw's choices (a used/drains
/// snapshot for a fusion's recipe, a restore of the gem's prior drained amount and
/// `used` membership for a gem) before the next candidate is tried — so a
/// requirement further down the spell's list can veto a pooling choice made for an
/// earlier one.
#[expect(
    clippy::too_many_arguments,
    reason = "the pool being filled (element, need), the board it draws from (spec, \
              state, tables), the in-flight assignment (used, drains), which sharing \
              rule applies (inside_fusion), and what to do once it's full \
              (continuation) are each load-bearing; bundling any of them would just \
              move the count into a struct without shrinking what a caller has to \
              supply"
)]
fn draw(
    element: ElementId,
    need: u16,
    around: LatticeCoord,
    spec: &LatticeSpec,
    state: &LatticeState,
    tables: &impl Tables,
    used: &mut BTreeSet<LatticeCoord>,
    drains: &mut BTreeMap<LatticeCoord, u16>,
    inside_fusion: bool,
    continuation: &mut dyn FnMut(
        &mut BTreeSet<LatticeCoord>,
        &mut BTreeMap<LatticeCoord, u16>,
    ) -> bool,
) -> bool {
    if need == 0 {
        return continuation(used, drains);
    }

    // A locked gem is spoken for by the enchantment it already hosts — its capacity
    // is committed, so it can fund no further cast. Excluding it keeps `locks`
    // one-enchantment-per-gem: without this, a second cast drawing on the same gem
    // would overwrite its lock and orphan the first enchantment, which could then
    // never break (its mana stranded, the disable→break invariant violated).
    let candidates: Vec<(LatticeCoord, CellKind)> = spec
        .present_neighbors(around)
        .into_iter()
        .filter(|(coord, _)| {
            !state.is_disabled(*coord) && !used.contains(coord) && !state.is_locked(*coord)
        })
        .collect();

    for (coord, kind) in candidates {
        match kind {
            CellKind::Gem {
                element: gem_element,
            } if gem_element == element => {
                // `already` is whatever an earlier draw in this same plan has taken
                // from this gem — possible when it feeds more than one fusion, since
                // those draws leave it out of `used`. Its remaining mana is what's
                // left after that.
                let already = drains.get(&coord).copied().unwrap_or(0);
                let available = state.mana(coord).saturating_sub(already);
                if available == 0 {
                    continue;
                }
                // Taking the most this gem can usefully give is always at least as
                // good as taking less: at the top level the cell is spoken for
                // either way, and inside a fusion's recipe a smaller draw here would
                // only leave more of `need` for other cells to cover without freeing
                // anything up in return.
                let take = available.min(need);
                drains.insert(coord, already + take);
                if !inside_fusion {
                    used.insert(coord);
                }
                if draw(
                    element,
                    need - take,
                    around,
                    spec,
                    state,
                    tables,
                    used,
                    drains,
                    inside_fusion,
                    continuation,
                ) {
                    return true;
                }
                if !inside_fusion {
                    used.remove(&coord);
                }
                if already == 0 {
                    drains.remove(&coord);
                } else {
                    drains.insert(coord, already);
                }
            }
            CellKind::Fusion { output } if output == element => {
                let Some(recipe) = tables.recipe(output) else {
                    continue;
                };
                // The amount drawn from this fusion scales its recipe rather than
                // being discarded: taking `amount` units of the fused element needs
                // that many units of *each* of the recipe's own feeders, recursively.
                // `amount == 1` reproduces the base recipe exactly. Tried from `need`
                // down to 1 so the fusion first offers to close out the whole pool by
                // itself, falling back to a smaller share only if its neighbours
                // can't back that much.
                let mut amount = need;
                loop {
                    let scaled_recipe: Vec<Requirement> = recipe
                        .iter()
                        .map(|feeder| Requirement {
                            element: feeder.element,
                            mana: feeder.mana.saturating_mul(amount),
                        })
                        .collect();
                    let used_snapshot = used.clone();
                    let drains_snapshot = drains.clone();
                    // A fusion is always exclusive, and its own recipe always runs
                    // with `inside_fusion = true` — a gem it draws on stays free for
                    // any sibling fusion resolved elsewhere in this same plan.
                    used.insert(coord);
                    let filled = satisfy(
                        &scaled_recipe,
                        coord,
                        spec,
                        state,
                        tables,
                        used,
                        drains,
                        true,
                    ) && draw(
                        element,
                        need - amount,
                        around,
                        spec,
                        state,
                        tables,
                        used,
                        drains,
                        inside_fusion,
                        continuation,
                    );
                    if filled {
                        return true;
                    }
                    *used = used_snapshot;
                    *drains = drains_snapshot;
                    if amount == 1 {
                        break;
                    }
                    amount -= 1;
                }
            }
            _ => {}
        }
    }

    false
}
