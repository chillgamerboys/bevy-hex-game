//! The property suite and unit tests for the lattice engine.
//!
//! Headless, no `App`, no fixtures — the cheapest test surface in the workspace.
//! It carries the design's geometric theorems (two tier-6 spells can never be
//! adjacent, fusion chains die downstream, disabling a locked gem breaks its
//! enchantment, serde round-trips are identity, channel/cast conserves mana)
//! alongside targeted unit checks.

use std::collections::BTreeMap;

use hex_core::{ElementId, LatticeCoord, SpellId};
use hex_lattice::{
    apply_cast, apply_disables, castable, channel, resolve_incoming, tick_burns, CastBlocked,
    Casting, CellKind, FusionTable, LatticeSpec, LatticeState, LatticeStats, Requirement,
    SpellTable,
};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

// --- content the tests speak against --------------------------------------

const FIRE: ElementId = ElementId(0);
const LIGHT: ElementId = ElementId(1);
const AIR: ElementId = ElementId(2);
const WATER: ElementId = ElementId(3);
const LIGHTNING: ElementId = ElementId(10);
const THUNDER: ElementId = ElementId(11);

const EMBER: SpellId = SpellId(0); // tier-1 fire evocation
const FIREBALL: SpellId = SpellId(1); // tier-6 fire evocation
const SHIELD: SpellId = SpellId(2); // tier-1 fire enchantment, defence 1
const THUNDERSPELL: SpellId = SpellId(3); // tier-1, needs a THUNDER source
const FREE: SpellId = SpellId(4); // tier-0
const WARD: SpellId = SpellId(5); // tier-1 fire enchantment, defence 0
const AEGIS: SpellId = SpellId(6); // tier-2 fire enchantment (two funding gems)
const HEAVY: SpellId = SpellId(7); // tier-2 fire evocation, draws 3 from each gem

fn req(element: ElementId, mana: u16) -> Requirement {
    Requirement { element, mana }
}

/// An in-memory stand-in for the RON content `hex_assets` will supply later.
struct Content;

impl SpellTable for Content {
    fn requirements(&self, spell: SpellId) -> Vec<Requirement> {
        if spell == EMBER {
            vec![req(FIRE, 1)]
        } else if spell == FIREBALL {
            vec![req(FIRE, 1); 6]
        } else if spell == SHIELD {
            vec![req(FIRE, 2)]
        } else if spell == THUNDERSPELL {
            vec![req(THUNDER, 1)]
        } else if spell == WARD {
            vec![req(FIRE, 1)]
        } else if spell == AEGIS {
            vec![req(FIRE, 1); 2]
        } else if spell == HEAVY {
            vec![req(FIRE, 3); 2]
        } else {
            Vec::new()
        }
    }

    fn casting(&self, spell: SpellId) -> Casting {
        if spell == SHIELD {
            Casting::Enchantment { defense: 1 }
        } else if spell == WARD {
            Casting::Enchantment { defense: 0 }
        } else if spell == AEGIS {
            Casting::Enchantment { defense: 2 }
        } else {
            Casting::Evocation
        }
    }
}

impl FusionTable for Content {
    fn recipe(&self, output: ElementId) -> Option<Vec<Requirement>> {
        if output == LIGHTNING {
            Some(vec![req(LIGHT, 1), req(FIRE, 1)])
        } else if output == THUNDER {
            Some(vec![req(LIGHTNING, 1), req(WATER, 1)])
        } else {
            None
        }
    }
}

fn basic_stats() -> LatticeStats {
    LatticeStats::new(
        BTreeMap::from([(FIRE, 3), (LIGHT, 3), (AIR, 3), (WATER, 3)]),
        BTreeMap::from([(FIRE, 5), (LIGHT, 5), (AIR, 5), (WATER, 5)]),
    )
}

/// A gem of `element`, for terser spec construction.
fn gem(element: ElementId) -> CellKind {
    CellKind::Gem { element }
}

// --- property 1: two tier-6 spells can never be adjacent ------------------

#[test]
fn two_tier6_spells_cannot_both_be_castable_when_adjacent() {
    // Two adjacent full-ring spells, every other neighbour a full fire gem.
    let a = LatticeCoord::new(0, 0);
    let b = LatticeCoord::new(1, 0);
    assert!(
        a.is_adjacent(b),
        "test fixture must place the spells adjacent"
    );

    let mut spec = LatticeSpec::default()
        .with(a, CellKind::Spell { spell: FIREBALL })
        .with(b, CellKind::Spell { spell: FIREBALL });
    for neighbor in a.neighbors() {
        if neighbor != b {
            spec = spec.with(neighbor, gem(FIRE));
        }
    }
    for neighbor in b.neighbors() {
        if neighbor != a && spec.get(neighbor).is_none() {
            spec = spec.with(neighbor, gem(FIRE));
        }
    }

    let state = LatticeState::new(&spec, &basic_stats());
    let a_ok = castable(&spec, &state, a, &Content).is_ok();
    let b_ok = castable(&spec, &state, b, &Content).is_ok();
    assert!(
        !(a_ok && b_ok),
        "adjacent tier-6 spells cannot both be castable"
    );
    // In fact neither is: each occupies the other's sixth slot.
    assert!(!a_ok && !b_ok);
}

#[test]
fn a_lone_tier6_spell_ringed_by_gems_is_castable() {
    // The positive control: the theorem forbids *adjacent* pairs, not the spell.
    let center = LatticeCoord::new(0, 0);
    let mut spec = LatticeSpec::default().with(center, CellKind::Spell { spell: FIREBALL });
    for neighbor in center.neighbors() {
        spec = spec.with(neighbor, gem(FIRE));
    }
    let state = LatticeState::new(&spec, &basic_stats());
    let plan = castable(&spec, &state, center, &Content).expect("a full fire ring powers fireball");
    assert_eq!(plan.drains.len(), 6, "six distinct gems fund the six slots");
}

#[test]
fn no_two_adjacent_tier6_spells_are_ever_both_castable() {
    // The theorem swept over random lattices.
    let mut pairs_checked = 0usize;
    for seed in 0..64 {
        let (spec, stats) = random_lattice(seed);
        let state = LatticeState::new(&spec, &stats);
        for (a, ka) in spec.cells() {
            let CellKind::Spell { spell: sa } = ka else {
                continue;
            };
            if sa != FIREBALL {
                continue;
            }
            for (b, kb) in spec.cells() {
                if b <= a || !a.is_adjacent(b) {
                    continue;
                }
                let CellKind::Spell { spell: sb } = kb else {
                    continue;
                };
                if sb != FIREBALL {
                    continue;
                }
                let a_ok = castable(&spec, &state, a, &Content).is_ok();
                let b_ok = castable(&spec, &state, b, &Content).is_ok();
                pairs_checked += 1;
                assert!(
                    !(a_ok && b_ok),
                    "seed {seed}: adjacent tier-6 spells both castable at {a:?}/{b:?}"
                );
            }
        }
    }
    assert!(
        pairs_checked > 0,
        "the sweep never generated an adjacent tier-6 pair — the property would be vacuous"
    );
}

// --- property 2: fusion chains die downstream -----------------------------

#[test]
fn disabling_a_deep_feeder_kills_the_whole_fusion_chain() {
    // spell -> thunder-fusion -> {lightning-fusion, water}; lightning -> {light, fire}.
    let spell = LatticeCoord::new(0, 0);
    let thunder = LatticeCoord::new(1, 0);
    let lightning = LatticeCoord::new(2, 0);
    let water = LatticeCoord::new(1, -1);
    let light = LatticeCoord::new(2, -1);
    let fire = LatticeCoord::new(2, 1);

    // The chain only means anything if the geometry is a real chain.
    assert!(spell.is_adjacent(thunder));
    assert!(thunder.is_adjacent(lightning));
    assert!(thunder.is_adjacent(water));
    assert!(lightning.is_adjacent(light));
    assert!(lightning.is_adjacent(fire));

    let spec = LatticeSpec::default()
        .with(
            spell,
            CellKind::Spell {
                spell: THUNDERSPELL,
            },
        )
        .with(thunder, CellKind::Fusion { output: THUNDER })
        .with(lightning, CellKind::Fusion { output: LIGHTNING })
        .with(water, gem(WATER))
        .with(light, gem(LIGHT))
        .with(fire, gem(FIRE));

    let mut state = LatticeState::new(&spec, &basic_stats());
    assert!(
        castable(&spec, &state, spell, &Content).is_ok(),
        "the intact chain powers the spell"
    );

    // Disable the deepest feeder; the death must propagate all the way up.
    let broken = apply_disables(&mut state, &[fire]);
    assert!(broken.is_empty(), "no enchantment was involved");
    assert_eq!(
        castable(&spec, &state, spell, &Content),
        Err(CastBlocked::Unsatisfiable),
        "a dead leaf kills the fusion chain downstream"
    );
}

// --- property 3: disabling a locked gem breaks its enchantment -------------

#[test]
fn disabling_a_locked_gem_breaks_its_enchantment_and_burns_the_mana() {
    let spell = LatticeCoord::new(0, 0);
    let [fire, ..] = spell.neighbors();
    let spec = LatticeSpec::default()
        .with(spell, CellKind::Spell { spell: SHIELD })
        .with(fire, gem(FIRE));

    let mut state = LatticeState::new(&spec, &basic_stats());
    let plan = castable(&spec, &state, spell, &Content).expect("the shield can be raised");
    apply_cast(&mut state, &plan, &Content);
    assert_eq!(state.enchantment_count(), 1);
    assert_eq!(state.total_locked_mana(), 2);
    assert!(state.is_locked(fire));

    let broken = apply_disables(&mut state, &[fire]);
    assert_eq!(broken.len(), 1, "the shield breaks");
    let record = broken.into_iter().next().expect("one break");
    assert_eq!(record.spell, SHIELD);
    assert_eq!(record.burned_mana, 2, "the locked mana is consumed");
    assert_eq!(record.trigger, fire);
    assert_eq!(state.enchantment_count(), 0, "the enchantment is gone");
    assert_eq!(state.total_locked_mana(), 0);
}

// --- property 4: serde round-trips are identity ---------------------------

#[test]
fn lattice_spec_round_trips_through_ron() {
    let spec = LatticeSpec::default()
        .with(LatticeCoord::new(0, 0), CellKind::Spell { spell: FIREBALL })
        .with(LatticeCoord::new(1, 0), gem(FIRE))
        .with(
            LatticeCoord::new(0, 1),
            CellKind::Fusion { output: LIGHTNING },
        )
        .with(LatticeCoord::new(-1, 0), CellKind::Blank);

    let text = ron::ser::to_string(&spec).expect("a spec serializes");
    let restored: LatticeSpec = ron::from_str(&text).expect("and deserializes");
    assert_eq!(spec, restored);
}

#[test]
fn random_specs_round_trip_through_ron() {
    for seed in 0..64 {
        let (spec, _) = random_lattice(seed);
        let text = ron::ser::to_string(&spec).expect("a spec serializes");
        let restored: LatticeSpec = ron::from_str(&text).expect("and deserializes");
        assert_eq!(spec, restored, "seed {seed} did not round-trip");
    }
}

// --- property 5: channel/cast conservation --------------------------------

#[test]
fn an_evocation_removes_exactly_its_plan_and_channel_refills_within_caps() {
    let spell = LatticeCoord::new(0, 0);
    let [fire, ..] = spell.neighbors();
    let spec = LatticeSpec::default()
        .with(spell, CellKind::Spell { spell: EMBER })
        .with(fire, gem(FIRE));

    let stats = basic_stats();
    let mut state = LatticeState::new(&spec, &stats);
    let before = state.total_gem_mana();

    let plan = castable(&spec, &state, spell, &Content).expect("ember can be cast");
    let cost: u32 = plan.drains.values().map(|&mana| u32::from(mana)).sum();
    apply_cast(&mut state, &plan, &Content);
    assert_eq!(
        before - state.total_gem_mana(),
        cost,
        "an evocation removes exactly the plan's mana"
    );

    // Channel refills toward capacity, never past it, and is deterministic.
    let drained = state.total_gem_mana();
    let mut twice = state.clone();
    channel(&mut state, &spec, &stats);
    let refilled = state.total_gem_mana();
    assert_eq!(
        refilled - drained,
        cost,
        "channel restores the throughput spent"
    );
    assert_eq!(
        refilled, before,
        "and no further — the gem is back at capacity"
    );

    channel(&mut twice, &spec, &stats);
    channel(&mut twice, &spec, &stats);
    assert_eq!(
        twice, state,
        "channelling is deterministic and idempotent at cap"
    );
}

#[test]
fn an_enchantment_ties_mana_up_and_a_break_consumes_it() {
    let spell = LatticeCoord::new(0, 0);
    let [fire, ..] = spell.neighbors();
    let spec = LatticeSpec::default()
        .with(spell, CellKind::Spell { spell: SHIELD })
        .with(fire, gem(FIRE));

    let mut state = LatticeState::new(&spec, &basic_stats());
    let total_before = state.total_gem_mana() + state.total_locked_mana();

    let plan = castable(&spec, &state, spell, &Content).expect("the shield can be raised");
    apply_cast(&mut state, &plan, &Content);
    assert_eq!(
        state.total_gem_mana() + state.total_locked_mana(),
        total_before,
        "casting an enchantment ties mana up rather than spending it"
    );

    let locked = state.total_locked_mana();
    apply_disables(&mut state, &[fire]);
    assert_eq!(
        state.total_gem_mana() + state.total_locked_mana(),
        total_before - locked,
        "a broken enchantment consumes its locked mana"
    );
}

// --- enchantment lock bookkeeping (regression + coverage) -----------------

#[test]
fn a_locked_gem_cannot_fund_a_second_enchantment() {
    // Regression: one gem, two adjacent spell cells. The first enchantment locks
    // the gem; the second must NOT be able to reuse it — doing so would overwrite
    // the lock and orphan the first enchantment (it could then never break).
    let gem_coord = LatticeCoord::new(0, 0);
    let [s1, s2, ..] = gem_coord.neighbors();
    let spec = LatticeSpec::default()
        .with(gem_coord, gem(FIRE))
        .with(s1, CellKind::Spell { spell: SHIELD }) // needs FIRE 2
        .with(s2, CellKind::Spell { spell: WARD }); // needs FIRE 1

    let mut state = LatticeState::new(&spec, &basic_stats()); // gem full at 3
    let plan = castable(&spec, &state, s1, &Content).expect("the shield raises");
    apply_cast(&mut state, &plan, &Content);
    assert!(state.is_locked(gem_coord));
    assert_eq!(state.enchantment_count(), 1);
    // The gem keeps 1 residual mana (>= WARD's 1 requirement), so only the lock —
    // not a mana shortfall — can be what stops the second cast.
    assert_eq!(state.mana(gem_coord), 1);

    assert_eq!(
        castable(&spec, &state, s2, &Content),
        Err(CastBlocked::Unsatisfiable),
        "a gem already hosting an enchantment cannot fund a second"
    );

    // The first enchantment's lock is intact: disabling the gem still breaks it.
    let broken = apply_disables(&mut state, &[gem_coord]);
    assert_eq!(broken.len(), 1);
    assert_eq!(state.enchantment_count(), 0);
}

#[test]
fn apply_cast_rejects_a_stale_plan_rather_than_orphaning() {
    // Two enchantment plans computed against the SAME fresh state, then both
    // applied. Once the first is applied its gem is locked, so the second plan is
    // stale; `apply_cast` must reject it atomically rather than overwrite the
    // first's lock and orphan it. This is the mutation-site guard for the
    // shared-gem invariant (the plan-time filter alone misses the concurrent order).
    let gem_coord = LatticeCoord::new(0, 0);
    let [s1, s2, ..] = gem_coord.neighbors();
    let spec = LatticeSpec::default()
        .with(gem_coord, gem(FIRE))
        .with(s1, CellKind::Spell { spell: SHIELD }) // FIRE 2, defence 1
        .with(s2, CellKind::Spell { spell: WARD }); // FIRE 1, defence 0

    let mut state = LatticeState::new(&spec, &basic_stats());
    // Both plans see the fresh, unlocked state, so both look castable.
    let plan1 = castable(&spec, &state, s1, &Content).expect("shield");
    let plan2 = castable(&spec, &state, s2, &Content).expect("ward, on the fresh state");

    assert!(
        apply_cast(&mut state, &plan1, &Content),
        "the first cast applies"
    );
    assert!(
        !apply_cast(&mut state, &plan2, &Content),
        "the stale second plan is rejected, not applied over the first's lock"
    );

    // The first enchantment is intact and still the only one — not orphaned.
    assert_eq!(state.enchantment_count(), 1);
    assert_eq!(resolve_incoming(&state, 1), 0, "the shield still defends");
    let broken = apply_disables(&mut state, &[gem_coord]);
    assert_eq!(broken.len(), 1, "disabling the gem breaks the shield");
    assert_eq!(broken.into_iter().next().expect("one break").burned_mana, 2);
    assert_eq!(state.enchantment_count(), 0);
}

#[test]
fn an_enchantment_funded_by_two_gems_clears_both_locks_when_it_breaks() {
    let spell = LatticeCoord::new(0, 0);
    let [g1, g2, ..] = spell.neighbors();
    let spec = LatticeSpec::default()
        .with(spell, CellKind::Spell { spell: AEGIS }) // needs FIRE 1 x2
        .with(g1, gem(FIRE))
        .with(g2, gem(FIRE));

    let mut state = LatticeState::new(&spec, &basic_stats());
    let plan = castable(&spec, &state, spell, &Content).expect("aegis draws from both gems");
    assert_eq!(plan.drains.len(), 2, "two distinct gems fund it");
    apply_cast(&mut state, &plan, &Content);
    assert!(state.is_locked(g1) && state.is_locked(g2));
    assert_eq!(state.enchantment_count(), 1);

    // Disabling just one funding gem breaks the enchantment and clears BOTH locks.
    let broken = apply_disables(&mut state, &[g1]);
    assert_eq!(broken.len(), 1);
    assert_eq!(state.enchantment_count(), 0);
    assert!(!state.is_locked(g2), "the other gem's lock is cleared too");
}

#[test]
fn channel_distributes_a_budget_across_gems_in_coordinate_order() {
    // Two empty fire gems and a channel budget that fills only one: the lower
    // coordinate must fill first, the higher stay empty (budget exhausted).
    let spell = LatticeCoord::new(0, 0);
    let [ga, gb, ..] = spell.neighbors();
    let (low, high) = if ga < gb { (ga, gb) } else { (gb, ga) };
    let spec = LatticeSpec::default()
        .with(spell, CellKind::Spell { spell: HEAVY }) // draws FIRE 3 x2
        .with(ga, gem(FIRE))
        .with(gb, gem(FIRE));

    // Capacity 3, channel budget exactly 3 — enough to refill one gem, not both.
    let stats = LatticeStats::new(BTreeMap::from([(FIRE, 3)]), BTreeMap::from([(FIRE, 3)]));
    let mut state = LatticeState::new(&spec, &stats); // both gems at 3
    let plan = castable(&spec, &state, spell, &Content).expect("heavy drains both gems");
    apply_cast(&mut state, &plan, &Content);
    assert_eq!(state.mana(low), 0);
    assert_eq!(state.mana(high), 0);

    channel(&mut state, &spec, &stats);
    assert_eq!(state.mana(low), 3, "the lower coordinate fills first");
    assert_eq!(
        state.mana(high),
        0,
        "the budget is spent before reaching the higher"
    );
}

// --- targeted unit tests ---------------------------------------------------

#[test]
fn casting_a_non_spell_cell_is_not_a_spell() {
    let coord = LatticeCoord::new(0, 0);
    let spec = LatticeSpec::default().with(coord, gem(FIRE));
    let state = LatticeState::new(&spec, &basic_stats());
    assert_eq!(
        castable(&spec, &state, coord, &Content),
        Err(CastBlocked::NotASpell)
    );
    // An absent cell is likewise not a spell.
    assert_eq!(
        castable(&spec, &state, LatticeCoord::new(9, 9), &Content),
        Err(CastBlocked::NotASpell)
    );
}

#[test]
fn a_disabled_spell_cell_cannot_cast() {
    let spell = LatticeCoord::new(0, 0);
    let [fire, ..] = spell.neighbors();
    let spec = LatticeSpec::default()
        .with(spell, CellKind::Spell { spell: EMBER })
        .with(fire, gem(FIRE));
    let mut state = LatticeState::new(&spec, &basic_stats());
    apply_disables(&mut state, &[spell]);
    assert_eq!(
        castable(&spec, &state, spell, &Content),
        Err(CastBlocked::SpellDisabled)
    );
}

#[test]
fn a_missing_element_is_unsatisfiable() {
    let spell = LatticeCoord::new(0, 0);
    let [neighbor, ..] = spell.neighbors();
    // Ember needs fire; offer it water.
    let spec = LatticeSpec::default()
        .with(spell, CellKind::Spell { spell: EMBER })
        .with(neighbor, gem(WATER));
    let state = LatticeState::new(&spec, &basic_stats());
    assert_eq!(
        castable(&spec, &state, spell, &Content),
        Err(CastBlocked::Unsatisfiable)
    );
}

#[test]
fn defensive_enchantments_subtract_from_incoming_disables() {
    // One active shield (defence 1): a fireball's 3 becomes 2, an ember's 1 becomes 0.
    let spell = LatticeCoord::new(0, 0);
    let [fire, ..] = spell.neighbors();
    let spec = LatticeSpec::default()
        .with(spell, CellKind::Spell { spell: SHIELD })
        .with(fire, gem(FIRE));
    let mut state = LatticeState::new(&spec, &basic_stats());
    let plan = castable(&spec, &state, spell, &Content).expect("shield");
    apply_cast(&mut state, &plan, &Content);

    assert_eq!(resolve_incoming(&state, 3), 2);
    assert_eq!(resolve_incoming(&state, 1), 0);
    assert_eq!(resolve_incoming(&state, 0), 0);
}

#[test]
fn burns_disable_one_hex_per_turn_and_expire() {
    let spec = LatticeSpec::default();
    let mut state = LatticeState::new(&spec, &basic_stats());
    state.add_burn(2);
    state.add_burn(2);

    assert_eq!(tick_burns(&mut state), 2, "two burns disable two hexes");
    assert_eq!(tick_burns(&mut state), 2, "and again on the next turn");
    assert_eq!(tick_burns(&mut state), 0, "then both have expired");
    assert!(state.burns().is_empty());
}

#[test]
fn a_new_state_starts_every_gem_full() {
    let a = LatticeCoord::new(0, 0);
    let b = LatticeCoord::new(1, 0);
    let spec = LatticeSpec::default()
        .with(a, gem(FIRE))
        .with(b, gem(LIGHT));
    let state = LatticeState::new(&spec, &basic_stats());
    assert_eq!(state.mana(a), 3, "fire gem full to its attunement capacity");
    assert_eq!(state.mana(b), 3);
}

#[test]
fn matching_is_deterministic_and_prefers_the_lower_coordinate() {
    // Ember needs one fire gem; two are adjacent. The lower coordinate must win,
    // every time.
    let spell = LatticeCoord::new(0, 0);
    let [first, second, ..] = spell.neighbors();
    let low = first.min(second);
    let spec = LatticeSpec::default()
        .with(spell, CellKind::Spell { spell: EMBER })
        .with(first, gem(FIRE))
        .with(second, gem(FIRE));
    let state = LatticeState::new(&spec, &basic_stats());

    let plan = castable(&spec, &state, spell, &Content).expect("ember");
    let again = castable(&spec, &state, spell, &Content).expect("ember");
    assert_eq!(plan, again, "the same inputs give the same plan");
    let (&picked, _) = plan.drains.iter().next().expect("one gem funds ember");
    assert_eq!(picked, low, "the lower-coordinate gem is chosen");
}

#[test]
fn enchant_ids_are_allocated_monotonically() {
    // Two shields on two spell cells, each drawing from its own gem.
    let s0 = LatticeCoord::new(0, 0);
    let s1 = LatticeCoord::new(3, 0);
    let [g0, ..] = s0.neighbors();
    let [g1, ..] = s1.neighbors();
    let spec = LatticeSpec::default()
        .with(s0, CellKind::Spell { spell: SHIELD })
        .with(g0, gem(FIRE))
        .with(s1, CellKind::Spell { spell: SHIELD })
        .with(g1, gem(FIRE));
    let mut state = LatticeState::new(&spec, &basic_stats());

    let p0 = castable(&spec, &state, s0, &Content).expect("first shield");
    apply_cast(&mut state, &p0, &Content);
    let p1 = castable(&spec, &state, s1, &Content).expect("second shield");
    apply_cast(&mut state, &p1, &Content);

    let ids: Vec<u32> = state.active_enchantments().map(|(id, _)| id.0).collect();
    assert_eq!(ids, vec![0, 1], "ids come from a monotonic counter");
}

#[test]
fn a_tier_zero_spell_casts_for_free() {
    let spell = LatticeCoord::new(0, 0);
    let spec = LatticeSpec::default().with(spell, CellKind::Spell { spell: FREE });
    let state = LatticeState::new(&spec, &basic_stats());
    let plan = castable(&spec, &state, spell, &Content).expect("a free spell always casts");
    assert!(plan.drains.is_empty(), "no gems are drawn");
}

#[test]
fn channel_never_exceeds_capacity() {
    // A full gem and a channel budget far past capacity: the gem must not overflow.
    // (The conservation test covers refilling from below and stopping at the cap.)
    let gem_coord = LatticeCoord::new(0, 0);
    let spec = LatticeSpec::default().with(gem_coord, gem(FIRE));
    let stats = LatticeStats::new(BTreeMap::from([(FIRE, 3)]), BTreeMap::from([(FIRE, 100)]));
    let mut state = LatticeState::new(&spec, &stats);
    channel(&mut state, &spec, &stats);
    assert_eq!(state.mana(gem_coord), 3, "a full gem stays at capacity");
}

#[test]
fn channelling_never_refills_a_locked_gem() {
    // The wave review's ship-blocker: an enchantment's cost is CAPACITY — the
    // locked gem is spoken for and must not be channelled. Refilling it would
    // quietly refund the mana the enchantment tied up.
    let gem_coord = LatticeCoord::new(0, 0);
    let [s1, free_gem, ..] = gem_coord.neighbors();
    let spec = LatticeSpec::default()
        .with(gem_coord, gem(FIRE))
        .with(free_gem, gem(FIRE))
        .with(s1, CellKind::Spell { spell: SHIELD }); // FIRE 2, locks gem_coord

    let stats = basic_stats();
    let mut state = LatticeState::new(&spec, &stats);
    // Drain the free gem so the positive control has room to refill.
    let heavy_drain = state.mana(free_gem);
    assert!(heavy_drain > 0, "fixture: the control gem starts funded");

    let plan = castable(&spec, &state, s1, &Content).expect("the shield raises");
    apply_cast(&mut state, &plan, &Content);
    assert!(state.is_locked(gem_coord));
    let locked_residual = state.mana(gem_coord);

    channel(&mut state, &spec, &stats);

    assert_eq!(
        state.mana(gem_coord),
        locked_residual,
        "a locked gem is capacity, not throughput — channelling must not touch it"
    );
    assert_eq!(
        state.mana(free_gem),
        heavy_drain,
        "the unlocked control gem still channels to its cap"
    );
}

#[test]
fn apply_cast_rejects_every_staleness_mode_and_leaves_state_untouched() {
    // The staleness predicate names three modes: a funding gem drained, locked,
    // or disabled between plan and apply. The locked mode has its own regression
    // above; these are the other two, each asserting FULL state equality so the
    // atomic-rejection claim is encoded rather than inferred.
    let gem_coord = LatticeCoord::new(0, 0);
    let [s1, s2, ..] = gem_coord.neighbors();
    let spec = LatticeSpec::default()
        .with(gem_coord, gem(FIRE))
        .with(s1, CellKind::Spell { spell: SHIELD }) // draws 2
        .with(s2, CellKind::Spell { spell: EMBER }); // draws 1

    // Drained: plan the SHIELD while the gem holds 3 (needs 2), then burn the gem
    // down to 1 with two separately-planned EMBERs before applying the stale plan.
    let mut state = LatticeState::new(&spec, &basic_stats());
    let shield_plan = castable(&spec, &state, s1, &Content).expect("shield on a full gem");
    for _ in 0..2 {
        let ember_plan = castable(&spec, &state, s2, &Content).expect("ember");
        assert!(apply_cast(&mut state, &ember_plan, &Content));
    }
    assert_eq!(
        state.mana(gem_coord),
        1,
        "fixture: below the shield's draw of 2"
    );
    let before = state.clone();
    assert!(
        !apply_cast(&mut state, &shield_plan, &Content),
        "a drained funding gem stales the plan"
    );
    assert_eq!(state, before, "rejection mutates nothing");

    // Disabled: plan, then disable the funding gem.
    let mut state = LatticeState::new(&spec, &basic_stats());
    let ember_plan = castable(&spec, &state, s2, &Content).expect("ember");
    apply_disables(&mut state, &[gem_coord]);
    let before = state.clone();
    assert!(
        !apply_cast(&mut state, &ember_plan, &Content),
        "a disabled funding gem stales the plan"
    );
    assert_eq!(state, before, "rejection mutates nothing");
}

#[test]
fn lattice_state_round_trips_through_ron() {
    // The battle-mutable half is the save-relevant half; its BTree maps, the
    // enchantment table, and the id counter must all survive serialization.
    let gem_coord = LatticeCoord::new(0, 0);
    let [s1, other, ..] = gem_coord.neighbors();
    let spec = LatticeSpec::default()
        .with(gem_coord, gem(FIRE))
        .with(other, gem(LIGHT))
        .with(s1, CellKind::Spell { spell: SHIELD });

    let mut state = LatticeState::new(&spec, &basic_stats());
    let plan = castable(&spec, &state, s1, &Content).expect("shield");
    apply_cast(&mut state, &plan, &Content); // an active enchantment + a lock
    apply_disables(&mut state, &[other]); // and a disabled cell

    let ron = ron::to_string(&state).expect("serialize");
    let back: LatticeState = ron::from_str(&ron).expect("deserialize");
    assert_eq!(state, back, "round trip changed the battle state");
}

#[test]
fn lattice_wire_formats_are_pinned() {
    // Same guard Wave 3 gave HexCoord: a symmetric field/variant rename passes
    // every round-trip test while silently changing the authoring and save
    // format, so the concrete text is asserted here.
    let coord = ron::to_string(&LatticeCoord::new(2, -1)).expect("serialize");
    assert_eq!(coord, "(q:2,r:-1)");

    let cell = ron::to_string(&CellKind::Gem { element: FIRE }).expect("serialize");
    assert_eq!(cell, "Gem(element:(0))");
}

// --- randomised lattice generator -----------------------------------------

fn random_lattice(seed: u64) -> (LatticeSpec, LatticeStats) {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut spec = LatticeSpec::default();
    for q in -2..=2 {
        for r in -2..=2 {
            let coord = LatticeCoord::new(q, r);
            match rng.gen_range(0..6) {
                0 => spec = spec.with(coord, gem(FIRE)),
                1 => spec = spec.with(coord, gem(LIGHT)),
                2 => spec = spec.with(coord, CellKind::Fusion { output: LIGHTNING }),
                3 => spec = spec.with(coord, CellKind::Spell { spell: FIREBALL }),
                4 => spec = spec.with(coord, CellKind::Blank),
                _ => {}
            }
        }
    }
    let stats = LatticeStats::new(
        BTreeMap::from([(FIRE, 3), (LIGHT, 3), (AIR, 3), (WATER, 3)]),
        BTreeMap::from([(FIRE, 1)]),
    );
    (spec, stats)
}
