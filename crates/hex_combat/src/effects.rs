//! The persistent-effect runtime: the ledger, the two tick hooks, and burn.
//!
//! [`hex_core::effects`] says what one lasting effect *is*; this says when it ticks,
//! when it stops, and what happens when it comes due. Lattice payloads are applied
//! through `hex_lattice`'s existing functions, which is the split
//! `docs/systems/casting.md` prescribes and the same one the command funnel uses.
//!
//! # Two hooks, because tick point is per payload
//!
//! `tick_turn_effects` runs at the **start of the acting unit's turn** and is where
//! personal payloads come due. `expire_round_effects` runs on `RoundElapsed` and is
//! where the round-boundary work lives. Burn is personal, so it ticks in the first —
//! the design words fire's damage over time as "at the start of the target's turn", and
//! a burn that ticked on the round boundary would hit a unit that had just acted and one
//! that had not at the same moment.
//!
//! # One countdown, here
//!
//! A burn is **entirely** a ledger entry: who lit it, who carries it, how long it lasts,
//! and how much of that has elapsed. The lattice holds hexes, mana and enchantments, and
//! a fire is none of those.
//!
//! It briefly lived the other way — a `Vec<Burn>` inside `LatticeState`, ticked by the
//! engine — and the seam was wrong in both directions. A burn has a *source*, and the
//! lattice has no vocabulary for one, so attribution had to live here anyway and the two
//! stores described one fact between them. Worse, the engine's counter ticked per
//! `advance` rather than per the target's turn, which is the tick point the design
//! actually specifies; a sandbox with no turn order could tick it at all. Splitting the
//! countdown from the record also meant every liveness question had to consult both.
//!
//! [`hex_core::PersistentEffect`]'s `ticks` closes it: one store, and `is_live` is a total function
//! of the record. There is nothing to drift against.
//!
//! # Burn ignores armour, but not the defender
//!
//! A due burn does **not** go through [`hex_lattice::resolve_incoming`] — fire's
//! identity is beating defences by ignoring them rather than overpowering them, and the
//! design says so outright. It *does* go through the defender-chooses seam, exactly as a
//! spell's damage does: the count is named, `PendingDecision::ChooseDisables` is
//! parked, and something answers with a `ChooseDisables` command that lands in the
//! replay log. Bypassing the subtraction is not the same as bypassing the choice, and
//! conflating the two would make burn the one damage source a fight could not replay.

use std::collections::{BTreeMap, VecDeque};

use bevy::prelude::*;

use hex_core::{
    AppSystems, EffectEnd, EffectId, EffectPayload, Mode, PausableSystems, PendingDecision,
    PersistentEffect, RoundElapsed, Screen, Turn, UnitId,
};
use hex_lattice::LatticeState;
use hex_units::UnitRegistry;

use crate::turns::TurnOrder;
use crate::CombatEvent;

/// A tick's worth of damage waiting for the decision seam to be free.
///
/// Everything needed to park a `PendingDecision::ChooseDisables`, captured at the
/// moment the effect came due rather than re-derived when the seam opens. By then the
/// tick has already been spent against the effect's countdown, so the count cannot be
/// recomputed — and the effect may have expired out of the ledger entirely.
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

    /// Advances every live personal effect on `target` and returns how many hexes they
    /// take between them.
    ///
    /// One hex per burn, which is the design's wording: "one additional hex disabled at
    /// the start of the target's turn". Two fires burn twice as fast.
    ///
    /// Ticking *then* counting matters at the boundary: a burn with one turn left still
    /// takes its hex on the turn it expires, so `AfterTurns(1)` means one hex rather
    /// than none.
    ///
    /// **Whether a payload is personal is a property of the payload, not of how the
    /// effect ends.** The match below is on `payload` for that reason: a burn bound to an
    /// enchantment rather than a turn count is still a burn, and an earlier shape that
    /// keyed the whole tick on `AfterTurns` would have skipped it silently forever. The
    /// end condition only decides whether there is anything left to spend.
    fn tick_personal(&mut self, target: UnitId) -> u16 {
        let mut due: u16 = 0;
        for effect in self.effects.values_mut() {
            if effect.target != target {
                continue;
            }
            // A turn-bounded effect spends one of its turns; anything else has no
            // per-turn budget to spend, and ticking it is free.
            if let EffectEnd::AfterTurns(turns) = effect.end {
                if effect.ticks >= turns {
                    continue;
                }
                effect.ticks = effect.ticks.saturating_add(1);
            }
            match effect.payload {
                EffectPayload::Burn => due = due.saturating_add(1),
            }
        }
        due
    }

    /// Registers a running effect and hands back its handle.
    ///
    /// Handles are dealt from a monotonic counter and **never reused within a session**,
    /// which is what lets `burn_source` blame the first fire lit deterministically
    /// rather than whichever record happened to land in a freed slot.
    fn insert(&mut self, effect: PersistentEffect) -> EffectId {
        let id = EffectId(self.next);
        self.next = self.next.saturating_add(1);
        self.effects.insert(id, effect);
        id
    }

    /// Drops every effect whose end condition has come.
    ///
    /// Takes a lookup rather than a query so both hooks can share it: the two hooks hold
    /// differently-shaped lattice queries, and neither can be spelled as the other's
    /// type.
    fn expire<'a>(
        &mut self,
        round: u32,
        mut lattice_of: impl FnMut(UnitId) -> Option<&'a LatticeState>,
    ) {
        self.effects
            .retain(|_, effect| is_live(effect, round, lattice_of(effect.target)));
    }

    /// Drops every effect whose end condition is denominated in rounds.
    ///
    /// See `clear_undelivered`, which is the only caller and carries the reasoning.
    fn drop_round_bounded(&mut self) {
        self.effects.retain(|id, effect| {
            let keep = !matches!(effect.end, EffectEnd::AfterRounds(_));
            if !keep {
                info!("effects: dropping {id:?}, measured in a finished fight's rounds");
            }
            keep
        });
    }

    /// Forgets everything. Session teardown — see `clear_session_effects`.
    fn clear(&mut self) {
        self.effects.clear();
        self.next = 0;
        self.due.clear();
    }
}

/// Whether an effect is still running.
///
/// **Every arm is a total function of facts this crate can see.** Two of them compare
/// counters that only ever go up — the round clock, and the effect's own tick count —
/// and the third asks the lattice a question only the lattice can answer: is that
/// enchantment still there.
///
/// A target with no lattice ends an enchantment-bound effect, because there is nothing
/// left to hold the enchantment. Turn- and round-bounded effects are not lattice-derived
/// and survive that, so an effect on a unit that somehow lost its lattice still expires
/// on schedule rather than never.
fn is_live(effect: &PersistentEffect, round: u32, lattice: Option<&LatticeState>) -> bool {
    match effect.end {
        EffectEnd::AfterRounds(rounds) => round < effect.start.saturating_add(rounds),
        EffectEnd::AfterTurns(turns) => effect.ticks < turns,
        EffectEnd::WithEnchantment(enchant) => {
            lattice.is_some_and(|state| state.enchantment(enchant).is_some())
        }
    }
}

/// Sets `target` alight for `turns` of its own turns, and records who did it.
///
/// **One write, in one place.** The ledger holds everything a burn is: who lit it, who
/// carries it, how long it lasts, and how much of that has elapsed. The lattice is not
/// told — it holds hexes and mana, and a fire is neither.
///
/// A zero-turn burn is dropped rather than recorded as an effect that ends immediately.
/// `hex_assets` already rejects `Burn(turns: 0)` at load, so this is the belt to that
/// file's braces; recording it would leave an entry `is_live` drops on the next pass,
/// having attributed a burn that never took a hex.
pub(crate) fn apply_burn(
    effects: &mut PersistentEffects,
    round: u32,
    source: UnitId,
    target: UnitId,
    turns: u16,
) {
    if turns == 0 {
        return;
    }
    effects.insert(PersistentEffect {
        source,
        target,
        payload: EffectPayload::Burn,
        start: round,
        end: EffectEnd::AfterTurns(turns),
        ticks: 0,
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
/// parks is already open when its human or policy answerer looks for one — a tick inside
/// `Act` would be unordered against the systems that answer it.
fn tick_turn_effects(
    order: Res<TurnOrder>,
    turns: Query<&UnitId, Added<Turn>>,
    registry: Res<UnitRegistry>,
    mut effects: ResMut<PersistentEffects>,
    // Read-only, and deliberately so: the tick no longer writes to a lattice at all, and
    // the only thing still asked of one is whether an enchantment an effect is bound to
    // survives. A `&mut` here would keep this system serialized against every other
    // lattice writer for a borrow it never uses.
    lattices: Query<&LatticeState>,
    mut events: MessageWriter<CombatEvent>,
) {
    let mut started: Vec<UnitId> = turns.iter().copied().collect();
    if started.is_empty() {
        return;
    }
    // Query iteration order is not stable. There should be exactly one newly granted
    // turn in ordinary play, but sorting keeps recovery from malformed multi-turn state
    // deterministic too.
    started.sort_unstable();

    for current in started {
        // The tick is entirely a ledger operation now: every live burn on this unit
        // advances by one and contributes one hex. `Added<Turn>` is the edge, so a turn
        // lasting many frames ticks once while a real same-round handoff ticks again.
        let due = effects.tick_personal(current);

        if due > 0 {
            let source = burn_source(&effects, current).unwrap_or_else(|| {
                // The only writer is `apply_burn`, which always records a source, and the
                // tick above only counts entries it just advanced — so this is unreachable
                // short of a wiring bug. Loud rather than silent: an unattributed hit is
                // how folklore about "random" damage starts.
                warn!("{current:?} burned with no effect record; blaming the target");
                current
            });
            effects.due.push_back(DueHit {
                target: current,
                count: due,
                source,
            });
            events.write(CombatEvent::BurnTicked {
                source,
                target: current,
                count: due,
            });
        }
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
/// Separate from `tick_turn_effects` because the seam holds **one** decision at a
/// time and a tick must not be lost waiting for it. The tick always happens on
/// schedule and queues what it found; this drains that queue as fast as the seam
/// allows, which in the ordinary case is the same frame.
///
/// **No [`hex_lattice::resolve_incoming`].** Burn ignores armour, so the count parked
/// here is the count the tick produced. It still goes through the defender's choice.
///
/// # Never park a decision nobody can answer
///
/// Every answer path needs the decider's [`LatticeState`] to offer valid cells. A unit
/// spawned from an archetype
/// `lattices.ron` does not define has no lattice — `hex_units` warns and spawns it inert —
/// and it still joins the turn order. Parking its choice would deadlock the entire fight:
/// neither UI nor policy can answer, so nothing clears `pending`, no unit acts, and every
/// later cast and strike is refused until the player walks far enough away to end combat.
///
/// So the hit is **dropped, loudly**, rather than parked. A fire on something that cannot
/// burn is content that needs fixing, and the log says which unit.
fn open_due_decision(
    registry: Res<UnitRegistry>,
    lattices: Query<&LatticeState>,
    mut effects: ResMut<PersistentEffects>,
    mut pending: ResMut<PendingDecision>,
    mut events: MessageWriter<CombatEvent>,
) {
    if pending.is_open() {
        return;
    }
    let Some(hit) = effects.due.pop_front() else {
        return;
    };
    let answerable = registry
        .entity_of(hit.target)
        .is_some_and(|entity| lattices.contains(entity));
    if !answerable {
        warn!(
            "burn: {:?} has no lattice to take {} hex(es) from; the tick is dropped",
            hit.target, hit.count
        );
        return;
    }
    *pending = PendingDecision::ChooseDisables {
        decider: hit.target,
        count: hit.count,
        source: hit.source,
    };
    events.write(CombatEvent::DecisionOpened {
        decider: hit.target,
        source: hit.source,
        count: hit.count,
    });
    info!(
        "burn: {:?} takes {} hex(es) from {:?}, ignoring armour",
        hit.source, hit.count, hit.target
    );
}

/// Expires effects at the round boundary.
///
/// The round half of "tick point is per payload". No payload ticks globally yet — burn
/// is personal and ticks in `tick_turn_effects` — so what this hook does today is
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

/// Ends the fight's half of the ledger: undelivered ticks and anything
/// measured in rounds.
///
/// A due hit that never reached the seam has nobody left to answer it, exactly like the
/// open decision `commands::clear_pending_decision` drops beside it. It is a real loss —
/// the tick that produced it was already spent against its effect's countdown — but the
/// alternative is holding damage for a fight that is over.
///
/// # Turn-bounded effects survive; round-bounded ones cannot
///
/// Nothing in the design puts a fire out because the party walked away from it, so a burn
/// keeps every turn it is owed: a unit's turns are its own and mean the same thing in any
/// fight. **A round does not.** `TurnOrder::clear` resets the counter to zero, so a
/// [`PersistentEffect::start`] recorded at round 7 of a finished fight would be compared
/// against a new fight's clock and read as eight rounds of life left instead of the one
/// it had. Rounds are a unit of *this fight's* time and do not survive it. Dropping them
/// here is the only reading that cannot silently make an effect permanent.
fn clear_undelivered(mut effects: ResMut<PersistentEffects>) {
    effects.due.clear();
    effects.drop_round_bounded();
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
            ticks: 0,
        }
    }

    /// A turn-bounded effect counts its own ticks and stops at the number it was given —
    /// the countdown is the record's, and nothing else has to be asked.
    #[test]
    fn a_turn_bounded_effect_lives_for_exactly_as_many_ticks_as_it_names() {
        let mut effect = burn_for(2);

        assert!(is_live(&effect, 0, None), "the fire is lit");
        effect.ticks = 1;
        assert!(is_live(&effect, 0, None), "one turn in, one to go");
        effect.ticks = 2;
        assert!(!is_live(&effect, 0, None), "and it is spent");
        assert!(!is_live(&effect, 99, None), "rounds do not resurrect it");
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

    /// A burn on a unit with no lattice keeps its schedule rather than ending early or
    /// hanging around: the countdown never depended on the lattice, so losing one cannot
    /// change it. Only [`EffectEnd::WithEnchantment`] is lattice-derived.
    #[test]
    fn a_burn_on_a_unit_with_no_lattice_keeps_its_own_schedule() {
        let mut effect = burn_for(2);
        assert!(is_live(&effect, 0, None), "still burning");
        effect.ticks = 2;
        assert!(
            !is_live(&effect, 0, None),
            "and still expires on its own count"
        );
    }

    /// Blame is deterministic and does not consult insertion luck: the first fire lit
    /// is the one named, on every machine.
    #[test]
    fn blame_falls_on_the_first_fire_lit() {
        let mut effects = PersistentEffects::default();
        apply_burn(&mut effects, 0, UnitId(7), UnitId(2), 2);
        apply_burn(&mut effects, 0, UnitId(3), UnitId(2), 2);
        apply_burn(&mut effects, 0, UnitId(9), UnitId(5), 2);

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

    /// Applying a burn writes the whole fire to the ledger in one place — countdown
    /// included, and starting from nothing elapsed.
    #[test]
    fn applying_a_burn_writes_the_whole_fire_to_the_ledger() {
        let mut effects = PersistentEffects::default();

        apply_burn(&mut effects, 4, UnitId(1), UnitId(2), 3);

        let (_, effect) = effects.iter().next().expect("the ledger holds the record");
        assert_eq!(effect.source, UnitId(1));
        assert_eq!(effect.target, UnitId(2));
        assert_eq!(effect.start, 4, "the round it began");
        assert_eq!(effect.end, EffectEnd::AfterTurns(3));
        assert_eq!(effect.ticks, 0, "and none of it has elapsed");
    }

    /// A tick is *personal*: it advances the acting unit's fires and nobody else's. The
    /// bug this forbids is the one the lattice-held design could not even express — a
    /// burn on a unit that has not had its turn yet counting down anyway.
    #[test]
    fn a_tick_advances_only_the_acting_units_fires() {
        let mut effects = PersistentEffects::default();
        apply_burn(&mut effects, 0, UnitId(1), UnitId(2), 2);
        apply_burn(&mut effects, 0, UnitId(1), UnitId(3), 2);

        assert_eq!(effects.tick_personal(UnitId(2)), 1, "one hex, once");
        let ticked: Vec<_> = effects.iter().map(|(_, e)| (e.target, e.ticks)).collect();
        assert_eq!(
            ticked,
            vec![(UnitId(2), 1), (UnitId(3), 0)],
            "the bystander's fire is untouched"
        );
    }

    /// Two fires on one target come due as one aggregated count, and a spent one stops
    /// contributing — the count is what the defender is asked to answer for.
    #[test]
    fn several_fires_on_one_target_aggregate_and_then_run_out() {
        let mut effects = PersistentEffects::default();
        apply_burn(&mut effects, 0, UnitId(7), UnitId(2), 1);
        apply_burn(&mut effects, 0, UnitId(3), UnitId(2), 2);

        assert_eq!(effects.tick_personal(UnitId(2)), 2, "both fires bite");
        assert_eq!(
            effects.tick_personal(UnitId(2)),
            1,
            "the short one is spent"
        );
        assert_eq!(effects.tick_personal(UnitId(2)), 0, "and then both are");
    }

    /// A zero-turn burn is not a burn. Recording one would attribute a fire that never
    /// takes a hex, and `hex_assets` rejects the content that could produce it anyway.
    #[test]
    fn a_zero_turn_burn_is_not_recorded_anywhere() {
        let mut effects = PersistentEffects::default();

        apply_burn(&mut effects, 0, UnitId(1), UnitId(2), 0);

        assert!(effects.is_empty(), "no record, so nothing to come due");
        assert_eq!(effects.tick_personal(UnitId(2)), 0);
    }

    /// Handles are never reused within a session, so a record cannot inherit the
    /// identity of one that expired — which is what keeps blame stable.
    #[test]
    fn handles_are_not_reused_after_an_effect_expires() {
        let mut effects = PersistentEffects::default();
        apply_burn(&mut effects, 0, UnitId(1), UnitId(2), 1);
        let first = effects.iter().next().map(|(id, _)| id);

        // Expire it the way a turn would: its one tick fires, then the sweep runs.
        effects.tick_personal(UnitId(2));
        effects.expire(0, |_| None);
        assert!(effects.is_empty(), "precondition: the record went with it");

        apply_burn(&mut effects, 0, UnitId(1), UnitId(2), 1);
        let second = effects.iter().next().map(|(id, _)| id);

        assert_eq!(first, Some(EffectId(0)));
        assert_eq!(second, Some(EffectId(1)), "the counter does not rewind");
    }

    /// Session teardown really forgets, handle counter included — unit ids restart, so
    /// a ledger that kept counting would be the only thing in the session that did.
    #[test]
    fn clearing_forgets_the_handles_too() {
        let mut effects = PersistentEffects::default();
        apply_burn(&mut effects, 0, UnitId(1), UnitId(2), 1);

        effects.clear();
        apply_burn(&mut effects, 0, UnitId(1), UnitId(2), 1);

        assert_eq!(
            effects.iter().next().map(|(id, _)| id),
            Some(EffectId(0)),
            "a new session starts at the first handle"
        );
    }
}
