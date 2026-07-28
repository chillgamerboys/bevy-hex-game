//! The persistent-effect runtime: the ledger, the two tick hooks, and burn.
//!
//! [`hex_core::effects`] says what one lasting effect *is*; this says when it ticks,
//! when it stops, and what happens when it comes due. Lattice payloads are applied
//! through `hex_lattice`'s existing functions, which is the split
//! `docs/systems/casting.md` prescribes and the same one the command funnel uses.
//!
//! # Two hooks, because tick point is per payload
//!
//! [`tick_turn_effects`] runs at the **start of the acting unit's turn** and is where
//! personal payloads come due. [`expire_round_effects`] runs on `RoundElapsed` and is
//! where the round-boundary work lives. Burn is personal, so it ticks in the first —
//! the design words fire's damage over time as "at the start of the target's turn", and
//! a burn that ticked on the round boundary would hit a unit that had just acted and one
//! that had not at the same moment.
//!
//! # One countdown, in the lattice
//!
//! A burn's remaining turns live in the target's [`LatticeState`], where HEX-12 put
//! them, and this ledger deliberately does **not** keep a second copy. The alternative —
//! moving `Vec<Burn>` out of `LatticeState` and into here — is a breaking change to a
//! type that is both a `Component` and the serde form of a unit's battle state, and it
//! would strand the lattice demo, which ticks burns through the engine directly.
//!
//! So the two stores hold different facts rather than the same one twice. The lattice
//! answers *how many hexes burn takes this turn*; the ledger answers *who lit it, when,
//! and under what end condition*. Neither can drift from the other, because neither
//! restates the other: [`is_live`] derives every end condition from live state.
//!
//! # Burn ignores armour, but not the defender
//!
//! A due burn does **not** go through [`hex_lattice::resolve_incoming`] — fire's
//! identity is beating defences by ignoring them rather than overpowering them, and the
//! design says so outright. It *does* go through the defender-chooses seam, exactly as a
//! spell's damage does: the count is named, [`PendingDecision::ChooseDisables`] is
//! parked, and something answers with a `ChooseDisables` command that lands in the
//! replay log. Bypassing the subtraction is not the same as bypassing the choice, and
//! conflating the two would make burn the one damage source a fight could not replay.

use std::collections::{BTreeMap, VecDeque};

use bevy::prelude::*;

use hex_core::{
    AppSystems, EffectEnd, EffectId, EffectPayload, Mode, PausableSystems, PendingDecision,
    PersistentEffect, RoundElapsed, Screen, UnitId,
};
use hex_lattice::{tick_burns, LatticeState};
use hex_units::UnitRegistry;

use crate::turns::TurnOrder;

/// A tick's worth of damage waiting for the decision seam to be free.
///
/// Everything needed to park a [`PendingDecision::ChooseDisables`], captured at the
/// moment the effect came due rather than re-derived when the seam opens — by then the
/// burn has already been taken off the lattice's counter and the count is gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DueHit {
    /// Whose hexes go down.
    target: UnitId,
    /// How many. **Already final** — burn skips defensive subtraction.
    count: u16,
    /// Who is blamed, for the log.
    source: UnitId,
}

/// Every persistent effect currently running, and the ticks they owe.
///
/// A resource rather than per-unit components: an effect names a source *and* a target,
/// so it belongs to neither entity, and the runtime has to iterate all of them on a
/// round boundary whether or not their units are still in the fight.
///
/// **Ordered by construction.** The ledger is a `BTreeMap` keyed by [`EffectId`] and the
/// due list is a queue, so iteration order is allocation order on every machine — the
/// same reason every collection in `hex_lattice` is a `BTreeMap`. Nothing here consults
/// entity order, entity bits, or query iteration order.
#[derive(Resource, Debug, Default)]
pub struct PersistentEffects {
    /// Running effects, in allocation order.
    effects: BTreeMap<EffectId, PersistentEffect>,
    /// The next handle. Monotonic, never reused within a session.
    next: u64,
    /// The turn whose personal effects have already ticked, as `(round, unit)`.
    ///
    /// A turn is identified by the round it falls in and who is acting, which is unique
    /// because the order advances between turns and the round counter moves when it
    /// wraps. Recording it is what makes the tick happen **exactly once** per turn no
    /// matter how many frames a turn lasts — a turn is many frames long, so anything
    /// keyed on "the acting unit is burning" would empty a lattice in about a second.
    ///
    /// One case slips through, deliberately in the safe direction. `TurnOrder::remove`
    /// wraps to the front **without** counting a round when the last unit in the order
    /// goes down, so whoever is at the front can get a second turn under a key that has
    /// already ticked, and their burn sits that one out. Skipping a tick is recoverable;
    /// the alternative reading — treat a repeated key as a new turn — would double every
    /// burn in the fight on every frame a turn lasts.
    last_ticked: Option<(u32, UnitId)>,
    /// Ticks that have come due but have not yet been handed to the decision seam.
    due: VecDeque<DueHit>,
}

impl PersistentEffects {
    /// Every running effect, in allocation order.
    pub fn iter(&self) -> impl Iterator<Item = (EffectId, &PersistentEffect)> + '_ {
        self.effects.iter().map(|(&id, effect)| (id, effect))
    }

    /// Every running effect carried by `target`, in allocation order.
    pub fn on(&self, target: UnitId) -> impl Iterator<Item = (EffectId, &PersistentEffect)> + '_ {
        self.iter()
            .filter(move |(_, effect)| effect.target == target)
    }

    /// How many effects are running.
    #[must_use]
    pub fn len(&self) -> usize {
        self.effects.len()
    }

    /// Whether nothing is running.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.effects.is_empty()
    }

    /// Registers a running effect and hands back its handle.
    fn insert(&mut self, effect: PersistentEffect) -> EffectId {
        let id = EffectId(self.next);
        self.next = self.next.saturating_add(1);
        self.effects.insert(id, effect);
        id
    }

    /// Drops every effect whose end condition has come.
    ///
    /// Takes a lookup rather than a query so both hooks can share it: the turn hook
    /// holds a mutable lattice query and the round hook a read-only one, and neither can
    /// be spelled as the other's type.
    fn expire<'a>(
        &mut self,
        round: u32,
        mut lattice_of: impl FnMut(UnitId) -> Option<&'a LatticeState>,
    ) {
        self.effects
            .retain(|_, effect| is_live(effect, round, lattice_of(effect.target)));
    }

    /// Forgets everything. Session teardown — see [`clear_session_effects`].
    fn clear(&mut self) {
        self.effects.clear();
        self.next = 0;
        self.last_ticked = None;
        self.due.clear();
    }
}

/// Whether an effect is still running.
///
/// **Every arm asks live state; none of them decrements a counter this crate holds.**
/// That is the whole anti-drift device described in the module docs — a burn is over
/// when the lattice says its burns are out, an enchantment-bound effect is over when the
/// enchantment is gone, and a round-bounded one is over when the clock says so.
///
/// A target with no lattice at all ends every lattice-derived effect: there is nothing
/// left to burn and nothing left to hold an enchantment. A round count is not
/// lattice-derived and survives that, so a future global payload on a unit that lost its
/// lattice still expires on schedule rather than never.
fn is_live(effect: &PersistentEffect, round: u32, lattice: Option<&LatticeState>) -> bool {
    match effect.end {
        EffectEnd::AfterRounds(rounds) => round < effect.start.saturating_add(rounds),
        EffectEnd::AfterTurns(_) => match effect.payload {
            EffectPayload::Burn => lattice.is_some_and(|state| !state.burns().is_empty()),
        },
        EffectEnd::WithEnchantment(enchant) => {
            lattice.is_some_and(|state| state.enchantment(enchant).is_some())
        }
    }
}

/// Sets `target` alight for `turns` of its own turns, and records who did it.
///
/// Two writes that are one fact: the burn goes onto the target's lattice, which is the
/// countdown, and the ledger records the source and the end condition, which the lattice
/// has no room for. There is no third place a burn can be added from — the cast path
/// calls this — so the ledger can be trusted to know every fire in the fight.
///
/// A zero-turn burn is dropped on both sides rather than recorded as an effect that ends
/// immediately. `hex_assets` already rejects `Burn(amount: 0)` at load, so this is the
/// belt to that file's braces; recording it would leave a ledger entry that
/// [`is_live`] would drop on the next pass anyway, having attributed a burn that never
/// took a hex.
pub(crate) fn apply_burn(
    effects: &mut PersistentEffects,
    state: &mut LatticeState,
    round: u32,
    source: UnitId,
    target: UnitId,
    turns: u16,
) {
    if turns == 0 {
        return;
    }
    state.add_burn(turns);
    effects.insert(PersistentEffect {
        source,
        target,
        payload: EffectPayload::Burn,
        start: round,
        end: EffectEnd::AfterTurns(turns),
    });
}

/// Who gets blamed for a burn tick.
///
/// Several burns on one target come due as **one** aggregated count, and therefore as
/// one decision with one blank to fill — so the blank cannot name every arsonist. The
/// lowest [`EffectId`] fills it: the first fire lit, chosen deterministically from an
/// ordered map rather than from whatever the ledger happened to iterate first.
///
/// The imprecision is bounded to the log, because `source` is attribution and
/// [`PersistentEffect::source`] says the rules never read it. The alternative — a
/// decision per burn — would park resolution on a chain of answers for what the design
/// describes as one hit.
fn burn_source(effects: &PersistentEffects, target: UnitId) -> Option<UnitId> {
    effects
        .on(target)
        .find(|(_, effect)| matches!(effect.payload, EffectPayload::Burn))
        .map(|(_, effect)| effect.source)
}

/// Ticks the acting unit's personal effects, at the start of its turn.
///
/// Runs before [`CombatSystems::Act`](crate::CombatSystems), so the decision a due burn
/// parks is already open when the auto-policy looks for one — a tick inside `Act` would
/// be unordered against the system that answers it.
fn tick_turn_effects(
    order: Res<TurnOrder>,
    registry: Res<UnitRegistry>,
    mut effects: ResMut<PersistentEffects>,
    mut lattices: Query<&mut LatticeState>,
) {
    let Some(current) = order.current() else {
        return;
    };
    let turn = (order.round, current);
    if effects.last_ticked == Some(turn) {
        return;
    }
    // Recorded before the tick rather than after it, and unconditionally. A turn that
    // ticked twice would double every burn in the fight; a turn that recorded only on
    // success would re-tick every frame for a unit whose lattice is momentarily
    // unreachable. Once, or not at all, is the only safe pair.
    effects.last_ticked = Some(turn);

    // Scoped so the mutable lattice borrow is over before the read-only one below.
    let due = registry
        .entity_of(current)
        .and_then(|entity| lattices.get_mut(entity).ok())
        .map_or(0, |mut state| tick_burns(&mut state));

    if due > 0 {
        let source = burn_source(&effects, current).unwrap_or_else(|| {
            // Burns reach a lattice only through `apply_burn`, which always records
            // one. A burn with no record is a wiring bug, and it has to be loud —
            // silently blaming somebody is how an unattributed hit becomes folklore.
            warn!("{current:?} is burning with no effect record; blaming the target");
            current
        });
        effects.due.push_back(DueHit {
            target: current,
            count: due,
            source,
        });
    }

    let round = order.round;
    effects.expire(round, |unit| {
        registry
            .entity_of(unit)
            .and_then(|entity| lattices.get(entity).ok())
    });
}

/// Hands the next due tick to the defender-chooses seam.
///
/// Separate from [`tick_turn_effects`] because the seam holds **one** decision at a
/// time and a tick must not be lost waiting for it. The tick always happens on
/// schedule and queues what it found; this drains that queue as fast as the seam
/// allows, which in the ordinary case is the same frame.
///
/// **No [`hex_lattice::resolve_incoming`].** Burn ignores armour, so the count parked
/// here is the count the tick produced. It still goes through the defender's choice.
fn open_due_decision(mut effects: ResMut<PersistentEffects>, mut pending: ResMut<PendingDecision>) {
    if pending.is_open() {
        return;
    }
    let Some(hit) = effects.due.pop_front() else {
        return;
    };
    *pending = PendingDecision::ChooseDisables {
        decider: hit.target,
        count: hit.count,
        source: hit.source,
    };
    info!(
        "burn: {:?} takes {} hex(es) from {:?}, ignoring armour",
        hit.source, hit.count, hit.target
    );
}

/// Expires effects at the round boundary.
///
/// The round half of "tick point is per payload". No payload ticks globally yet — burn
/// is personal and ticks in [`tick_turn_effects`] — so what this hook does today is
/// evaluate end conditions, which is real work: [`EffectEnd::AfterRounds`] can only come
/// due here, and a burn whose lattice counter emptied gets dropped here as well as on
/// its target's next turn.
///
/// Reads `RoundElapsed` rather than watching [`TurnOrder::round`](crate::TurnOrder), so
/// every per-round consumer agrees on when a round ended, and is ordered **after**
/// `CombatSystems::Advance` through the shared set — that is where the message is
/// written, and a local `.chain()` cannot express ordering across that boundary.
fn expire_round_effects(
    mut rounds: MessageReader<RoundElapsed>,
    order: Res<TurnOrder>,
    registry: Res<UnitRegistry>,
    mut effects: ResMut<PersistentEffects>,
    lattices: Query<&LatticeState>,
) {
    if rounds.read().count() == 0 {
        return;
    }
    let round = order.round;
    effects.expire(round, |unit| {
        registry
            .entity_of(unit)
            .and_then(|entity| lattices.get(entity).ok())
    });
}

/// Forgets every effect on the way out of a session.
///
/// Unit ids restart each session, so a ledger held across one names somebody else's
/// unit next launch — the same reason the command queue and the open decision are
/// cleared beside it (see `commands::clear_session_state`). An effect is worse than a
/// stale command, because nothing ever drains it: a burn inherited from a previous
/// session would tick on a stranger every turn, forever.
fn clear_session_effects(mut effects: ResMut<PersistentEffects>) {
    effects.clear();
}

/// Drops undelivered ticks and the turn cursor when a fight ends.
///
/// A due hit that never reached the seam has nobody left to answer it, exactly like the
/// open decision `commands::clear_pending_decision` drops beside it. The cursor goes too
/// because the next fight restarts the round counter, and a stale `(round, unit)` would
/// match the first turn of that fight and silently skip its tick.
///
/// **The effects themselves stay.** Nothing in the design puts a fire out because the
/// party walked away from it, and the lattice's own burn counter survives the mode flip
/// regardless — it is a component on the unit. Clearing the ledger here would leave
/// those burns ticking with no record of who lit them.
fn clear_undelivered(mut effects: ResMut<PersistentEffects>) {
    effects.due.clear();
    effects.last_ticked = None;
}

/// Registers the runtime.
pub(crate) fn plugin(app: &mut App) {
    app.init_resource::<PersistentEffects>()
        .register_type::<PersistentEffect>()
        .register_type::<EffectPayload>()
        .register_type::<EffectEnd>()
        .register_type::<EffectId>();
    app.add_systems(
        Update,
        (tick_turn_effects, open_due_decision)
            .chain()
            .in_set(AppSystems::Update)
            .in_set(PausableSystems)
            // A shared set, not `.before(a_system)`: the tick has to be complete
            // before anything decides what to do with the turn, and `Act` is what
            // "deciding" means. The set boundary also supplies the sync point.
            .before(crate::CombatSystems::Act)
            .run_if(in_state(Mode::Combat)),
    );
    app.add_systems(
        Update,
        expire_round_effects
            .in_set(AppSystems::Update)
            .in_set(PausableSystems)
            .after(crate::CombatSystems::Advance)
            .run_if(in_state(Mode::Combat)),
    );
    app.add_systems(OnExit(Screen::Gameplay), clear_session_effects);
    app.add_systems(OnExit(Mode::Combat), clear_undelivered);
}

#[cfg(test)]
mod tests {
    use hex_core::EnchantId;

    use super::*;

    fn burn_for(turns: u16) -> PersistentEffect {
        PersistentEffect {
            source: UnitId(1),
            target: UnitId(2),
            payload: EffectPayload::Burn,
            start: 0,
            end: EffectEnd::AfterTurns(turns),
        }
    }

    /// A burn is live while the lattice still carries one, and dead the moment it does
    /// not — the ledger never second-guesses the countdown it does not own.
    #[test]
    fn a_burn_lives_exactly_as_long_as_the_lattice_says() {
        let mut state = LatticeState::default();
        state.add_burn(1);
        let effect = burn_for(1);

        assert!(is_live(&effect, 0, Some(&state)), "the fire is lit");
        assert_eq!(tick_burns(&mut state), 1, "precondition: it comes due once");
        assert!(
            !is_live(&effect, 0, Some(&state)),
            "and the record goes with the last burn, not on a count of its own"
        );
    }

    /// A round-bounded effect expires on the clock, with no lattice consulted — so a
    /// future global payload does not silently become permanent on a unit whose lattice
    /// is gone.
    #[test]
    fn a_round_bounded_effect_expires_on_the_clock() {
        let effect = PersistentEffect {
            end: EffectEnd::AfterRounds(2),
            start: 3,
            ..burn_for(1)
        };

        assert!(is_live(&effect, 3, None), "the round it began");
        assert!(is_live(&effect, 4, None), "one round in");
        assert!(!is_live(&effect, 5, None), "two rounds in, and it is over");
        assert!(!is_live(&effect, 99, None), "and stays over");
    }

    /// An enchantment-bound effect dies with the enchantment, which is what "bound to"
    /// has to mean — the alternative is an upkeep that outlives the thing it upkeeps.
    #[test]
    fn an_enchantment_bound_effect_dies_with_its_enchantment() {
        let state = LatticeState::default();
        let effect = PersistentEffect {
            end: EffectEnd::WithEnchantment(EnchantId(0)),
            ..burn_for(1)
        };

        assert!(
            !is_live(&effect, 0, Some(&state)),
            "no such enchantment, so nothing to hold it up"
        );
        assert!(
            !is_live(&effect, 0, None),
            "and no lattice is not a reason to keep it either"
        );
    }

    /// A lattice-derived effect on a unit with no lattice ends rather than hanging
    /// around: there is nothing left to burn, and a record nothing can expire would
    /// tick forever.
    #[test]
    fn a_burn_on_a_unit_with_no_lattice_ends() {
        assert!(!is_live(&burn_for(2), 0, None));
    }

    /// Blame is deterministic and does not consult insertion luck: the first fire lit
    /// is the one named, on every machine.
    #[test]
    fn blame_falls_on_the_first_fire_lit() {
        let mut effects = PersistentEffects::default();
        let mut state = LatticeState::default();
        apply_burn(&mut effects, &mut state, 0, UnitId(7), UnitId(2), 2);
        apply_burn(&mut effects, &mut state, 0, UnitId(3), UnitId(2), 2);
        apply_burn(&mut effects, &mut state, 0, UnitId(9), UnitId(5), 2);

        assert_eq!(
            burn_source(&effects, UnitId(2)),
            Some(UnitId(7)),
            "the earlier effect id wins, not the lower unit id"
        );
        assert_eq!(burn_source(&effects, UnitId(5)), Some(UnitId(9)));
        assert_eq!(
            burn_source(&effects, UnitId(4)),
            None,
            "a unit nobody set alight has nobody to blame"
        );
    }

    /// Applying a burn writes the countdown to the lattice and the attribution to the
    /// ledger — the two facts the module docs say are kept apart.
    #[test]
    fn applying_a_burn_writes_the_countdown_to_the_lattice() {
        let mut effects = PersistentEffects::default();
        let mut state = LatticeState::default();

        apply_burn(&mut effects, &mut state, 4, UnitId(1), UnitId(2), 3);

        assert_eq!(state.burns().len(), 1, "the lattice carries the countdown");
        let (_, effect) = effects.iter().next().expect("and the ledger the record");
        assert_eq!(effect.source, UnitId(1));
        assert_eq!(effect.target, UnitId(2));
        assert_eq!(effect.start, 4, "the round it began");
        assert_eq!(effect.end, EffectEnd::AfterTurns(3));
    }

    /// A zero-turn burn is not a burn. Recording one would attribute a fire that never
    /// takes a hex, and `hex_assets` rejects the content that could produce it anyway.
    #[test]
    fn a_zero_turn_burn_is_not_recorded_anywhere() {
        let mut effects = PersistentEffects::default();
        let mut state = LatticeState::default();

        apply_burn(&mut effects, &mut state, 0, UnitId(1), UnitId(2), 0);

        assert!(effects.is_empty(), "no record");
        assert!(state.burns().is_empty(), "and no countdown");
    }

    /// Handles are never reused within a session, so a record cannot inherit the
    /// identity of one that expired — which is what keeps blame stable.
    #[test]
    fn handles_are_not_reused_after_an_effect_expires() {
        let mut effects = PersistentEffects::default();
        let mut state = LatticeState::default();
        apply_burn(&mut effects, &mut state, 0, UnitId(1), UnitId(2), 1);
        let first = effects.iter().next().map(|(id, _)| id);

        // Expire it the way a tick would: the lattice's counter empties.
        tick_burns(&mut state);
        effects.expire(0, |_| Some(&state));
        assert!(effects.is_empty(), "precondition: the record went with it");

        apply_burn(&mut effects, &mut state, 0, UnitId(1), UnitId(2), 1);
        let second = effects.iter().next().map(|(id, _)| id);

        assert_eq!(first, Some(EffectId(0)));
        assert_eq!(second, Some(EffectId(1)), "the counter does not rewind");
    }

    /// Session teardown really forgets, handle counter included — unit ids restart, so
    /// a ledger that kept counting would be the only thing in the session that did.
    #[test]
    fn clearing_forgets_the_handles_too() {
        let mut effects = PersistentEffects::default();
        let mut state = LatticeState::default();
        apply_burn(&mut effects, &mut state, 0, UnitId(1), UnitId(2), 1);

        effects.clear();
        apply_burn(&mut effects, &mut state, 0, UnitId(1), UnitId(2), 1);

        assert_eq!(
            effects.iter().next().map(|(id, _)| id),
            Some(EffectId(0)),
            "a new session starts at the first handle"
        );
    }
}
