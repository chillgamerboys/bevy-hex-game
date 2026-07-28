//! Casting: the first three rungs of the legality ladder, and the damage that follows.
//!
//! # What this owns, and what it refuses
//!
//! `docs/systems/casting.md` specifies five rungs, checked in order. This file does
//! **1 to 3**: whose turn it is (the funnel's existing gate), whether the lattice can
//! pay ([`castable`]), and whether the target is in range and the shape resolves. Rungs
//! 4 and 5 — what a spell does to *terrain*, and announcing it to the world — belong to
//! terrain magic, which is blocked on a fact about voxel occupancy the world lane has
//! not published yet. Effects that need them are refused by name rather than silently
//! skipped.
//!
//! # Casting is committing
//!
//! A cast that passes the ladder spends its mana and takes the action, whether or not
//! the effect finds anything to do. A fireball into an empty hex is a legal cast that
//! burns nothing. That payment policy is provisional and the design says so, but the
//! order matters: the lattice is drained *before* effects resolve, so an effect that
//! turns out to be a no-op has still cost what it cost.

use bevy::prelude::*;

use hex_assets::{Effect, TargetShape};
use hex_core::{Busy, LatticeCoord, PendingDecision, TilePos, UnitId};
use hex_lattice::{apply_cast, castable, CastBlocked, CellKind, LatticeSpec, LatticeState};
use hex_units::{targeting, volumes};

use super::{ActorQuery, Verb};

/// Applies a cast, or returns the reason it was refused.
#[expect(
    clippy::too_many_arguments,
    reason = "a cast reads more of the world than any other verb: the caster's lattice, \
              the target's, the spell book, and the shape. Grouping them behind a struct \
              would mean holding several ECS borrows at once, which is exactly what the \
              Verb split exists to avoid."
)]
pub(super) fn apply(
    ctx: &mut Verb,
    commands: &mut Commands,
    actors: &mut ActorQuery,
    lattices: &mut LatticeQuery,
    unit: UnitId,
    entity: Entity,
    spell_name: &str,
    target: TilePos,
    facing: Option<hex_core::Sextant>,
) -> Result<(), &'static str> {
    if !ctx.in_combat {
        return Err("casting is combat-only until out-of-combat magic is designed");
    }
    if ctx.turn_order.current() != Some(unit) {
        return Err("not this unit's turn");
    }
    // One decision at a time. A second cast landing while a defender still owes an
    // answer would resolve its damage against a lattice that is about to change.
    if ctx.pending.is_open() {
        return Err("a decision is still open — resolution is parked");
    }

    let Some(book) = ctx.spells else {
        return Err("no spell book loaded");
    };
    let Some(tables) = ctx.tables.as_ref() else {
        return Err("no content tables to resolve requirements against");
    };
    let Some(spell) = book.id(spell_name) else {
        return Err("no such spell");
    };
    let Some(spec) = book.spell(spell) else {
        return Err("spell has no definition");
    };

    let (standing, busy) = {
        let Ok((standing, _, turn, busy, _, _)) = actors.get(entity) else {
            return Err("unit no longer exists");
        };
        // Rung 1 is "action available", and casting is an action. Without this a unit
        // casts every frame its turn lasts: a cast starts no animation, so the `Busy`
        // it sets is gone by the next frame, and `advance_turn` waits for the movement
        // budget as well — so the whole lattice could be spent in one turn.
        let Some(turn) = turn else {
            return Err("no turn to take the action from");
        };
        if turn.acted {
            return Err("unit already took its action");
        }
        let Some(standing) = standing.copied() else {
            return Err("unit has no standing to cast from");
        };
        (standing.0, busy)
    };
    if busy || ctx.committed.contains(&entity) {
        return Err("unit is still finishing its last action");
    }

    // --- rung 3: targeting -------------------------------------------------

    // Directed shapes need a facing, and a directed cast without one is a malformed
    // command rather than a cast that reaches nothing.
    if volumes::needs_facing(&spec.targeting.shape) && facing.is_none() {
        return Err("this spell points somewhere and the cast named no facing");
    }
    // `SelfCast` is the one shape whose range is not a question.
    if !matches!(spec.targeting.shape, TargetShape::SelfCast) {
        let levels = ctx.combat.map_or(DEFAULT_LEVELS_PER_BONUS, |settings| {
            settings.levels_per_bonus_range
        });
        if !targeting::in_reach(
            standing.pos,
            target,
            u32::from(spec.targeting.range),
            levels,
        ) {
            return Err("target is out of range");
        }
    }
    // Resolved but not yet consumed: rungs 4 and 5 are what read a volume, and both
    // are terrain magic's. Resolving it here anyway is the cheap half of the check —
    // a shape that cannot resolve is a cast that should not have been legal.
    if volumes::resolve(&spec.targeting.shape, standing.pos, target, facing).is_none() {
        return Err("the spell's shape did not resolve");
    }

    // Observation. Returns true because no fog exists yet — every current target
    // genuinely *is* observed — and it is written as a function rather than omitted so
    // that the day `hex_perception` lands, this is the one line that changes.
    if !anchor_is_observed(target) {
        return Err("the cast's anchor is not observed");
    }

    // --- rung 2: the lattice -----------------------------------------------

    let cell = {
        let Ok((caster_spec, caster_state)) = lattices.get(entity) else {
            return Err("caster has no lattice to cast from");
        };
        spell_cell(caster_spec, caster_state, spell)
            .ok_or("this unit's lattice does not inscribe that spell")?
    };

    let plan = {
        let Ok((caster_spec, caster_state)) = lattices.get(entity) else {
            return Err("caster has no lattice to cast from");
        };
        castable(caster_spec, caster_state, cell, tables).map_err(|blocked| match blocked {
            CastBlocked::NotASpell => "that lattice cell holds no spell",
            CastBlocked::SpellDisabled => "that spell's hex is disabled",
            CastBlocked::Unsatisfiable => "the lattice cannot pay for that spell",
        })?
    };

    // Committing. `apply_cast` returns `false` when the plan went stale between
    // planning and applying — nothing mutated, and a caller that ignored it would
    // silently eat the cast along with the player's turn.
    {
        let Ok((_, mut caster_state)) = lattices.get_mut(entity) else {
            return Err("caster has no lattice to cast from");
        };
        if !apply_cast(&mut caster_state, &plan, tables) {
            return Err("the cast plan went stale before it could be applied");
        }
    }

    // --- effects -----------------------------------------------------------

    let target_unit = unit_standing_on(ctx, actors, target);
    let round = ctx.turn_order.round;
    let mut refusals: Vec<&'static str> = Vec::new();
    for effect in &spec.effects {
        match effect {
            Effect::DisableHexes { count, targeted } => {
                if *targeted {
                    refusals.push("targeted disables need the attacker to pick hexes (not built)");
                    continue;
                }
                let Some(defender) = target_unit else {
                    // Not a refusal: a damaging spell that reaches nobody is a legal
                    // cast that hurt nothing, and it has already been paid for.
                    continue;
                };
                open_disable_decision(
                    ctx,
                    lattices,
                    defender.0,
                    defender.1,
                    unit,
                    u16::from(*count),
                );
            }
            Effect::Reveal { .. } => {
                refusals.push("Reveal waits on divination writing into the knowledge store");
            }
            Effect::Illuminate { .. } => refusals.push("Illuminate waits on the perception lane"),
            Effect::SetTerrain { .. } | Effect::ClearTerrain | Effect::SpawnWall { .. } => {
                refusals.push("terrain effects wait on RunBottom and the announce path");
            }
            Effect::Burn { amount } => {
                let Some(defender) = target_unit else {
                    // Same rule as `DisableHexes` above: a spell that reaches nobody is
                    // a legal cast that set nothing alight, and it is already paid for.
                    continue;
                };
                let Ok((_, mut state)) = lattices.get_mut(defender.1) else {
                    refusals.push("the target has no lattice to set alight");
                    continue;
                };
                // **Nothing goes down now.** Burn's whole shape is that it arrives at
                // the start of each of the target's own turns, so what a cast does is
                // start a countdown; `crate::effects` is what collects on it and routes
                // the result through the same defender-chooses seam as any other damage.
                //
                // `amount` is **how many of the target's turns burn for**, which is the
                // only reading the design supports: burn is "one additional hex disabled
                // at the start of the target's turn, for some number of turns", and
                // `LatticeState::add_burn` takes exactly that in exactly this width.
                // `hex_assets`' field doc still describes an older idea (burning locked
                // mana) that nothing implements and that would make the shipped
                // Flamethrower a no-op against any target without an enchantment.
                crate::effects::apply_burn(
                    ctx.effects,
                    &mut state,
                    round,
                    unit,
                    defender.0,
                    *amount,
                );
                info!(
                    "cast: {unit:?} sets {:?} alight for {amount} of its turns",
                    defender.0
                );
            }
            Effect::RestoreHexes { .. } => {
                refusals.push("RestoreHexes waits on choosing which hexes come back");
            }
            Effect::ModifyIncomingDisables { .. } => {
                refusals.push("one-shot wards have nowhere to live in the lattice yet");
            }
            Effect::Displace { .. } => refusals.push("Displace waits on forced movement"),
        }
    }

    // Loud, but after the fact: the cast happened and was paid for. Refusing the whole
    // cast because one of its effects is unbuilt would make a spell that mostly works
    // uncastable, and refusing silently is the failure this codebase is worst at seeing.
    for reason in refusals {
        warn!("cast {spell_name:?} by {unit:?}: effect skipped — {reason}");
    }

    if let Some(mut turn) = actors
        .get_mut(entity)
        .ok()
        .and_then(|(_, _, turn, ..)| turn)
    {
        turn.acted = true;
    }
    if ctx.settings.is_some() {
        commands.entity(entity).insert(Busy);
        ctx.committed.push(entity);
    }
    info!("cast: {unit:?} casts {spell_name:?} at {target:?}");
    Ok(())
}

/// Height-per-range-bonus when `combat.ron` has not loaded.
///
/// The real value is `CombatSettings::levels_per_bonus_range`; this is only the fallback
/// for a headless harness with no settings, and it matches the shipped number so the two
/// cannot disagree in the case that matters. Casting inherits high-ground-buys-range for
/// free by going through the same [`targeting::in_reach`] engagement uses — the rule was
/// written for spells and has had exactly one consumer until now.
const DEFAULT_LEVELS_PER_BONUS: u32 = 5;

/// The caster's cell holding `spell`, lowest coordinate first.
///
/// A lattice **may** inscribe the same spell twice — nothing forbids it, and a designer
/// might do it deliberately so that losing one hex does not lose the spell. Picking the
/// lowest `LatticeCoord` makes the choice deterministic rather than dependent on
/// iteration; `spec.cells()` is already ordered, so first-match is that. The alternative
/// — refusing an ambiguous lattice — would turn a redundancy into an authoring error.
fn spell_cell(
    spec: &LatticeSpec,
    state: &LatticeState,
    spell: hex_core::SpellId,
) -> Option<LatticeCoord> {
    let mut matching = spec.cells().filter_map(|(coord, kind)| match kind {
        CellKind::Spell { spell: found } if found == spell => Some(coord),
        _ => None,
    });
    // The live one, and only then the lowest. Taking the lowest unconditionally would
    // defeat the redundancy this is supposed to preserve: a second copy exists so that
    // losing one hex does not lose the spell, and picking a disabled cell when a live
    // one is right there refuses the cast as "that spell's hex is disabled".
    let mut fallback = None;
    for coord in &mut matching {
        if !state.is_disabled(coord) {
            return Some(coord);
        }
        fallback = fallback.or(Some(coord));
    }
    fallback
}

/// Whether the cast's anchor is currently observed by the acting faction.
///
/// **True, and that is the truth rather than a stub.** The rule is absolute — a cast
/// must anchor on an observed position, including divination — but no fog exists yet, so
/// every position genuinely is observed. Written as a function so the day `hex_perception`
/// publishes what a faction can see, this is the one line that changes, in one crate.
const fn anchor_is_observed(_anchor: TilePos) -> bool {
    true
}

/// Parks the defender's choice of which hexes go down.
///
/// The count comes out of [`hex_lattice::resolve_incoming`], which subtracts the flat
/// defence of any active enchantment. **Zero opens no decision**: a hit that a shield
/// absorbed entirely is not a choice anybody has to make, and an open decision requiring
/// zero cells would park resolution on an answer with no content.
pub(super) fn open_disable_decision(
    ctx: &mut Verb,
    lattices: &mut LatticeQuery,
    defender_id: UnitId,
    defender_entity: Entity,
    source: UnitId,
    raw: u16,
) {
    let Ok((_, state)) = lattices.get(defender_entity) else {
        return;
    };
    let count = hex_lattice::resolve_incoming(state, raw);
    if count == 0 {
        info!("cast: {defender_id:?} absorbed the whole hit");
        return;
    }
    *ctx.pending = PendingDecision::ChooseDisables {
        decider: defender_id,
        count,
        source,
    };
}

/// The unit standing on `pos`, if any.
fn unit_standing_on(ctx: &Verb, actors: &ActorQuery, pos: TilePos) -> Option<(UnitId, Entity)> {
    ctx.registry.iter().find_map(|(id, entity)| {
        let (standing, ..) = actors.get(entity).ok()?;
        (standing?.0.pos == pos).then_some((id, entity))
    })
}

/// Lattices, as the applier reaches them.
///
/// Separate from [`ActorQuery`] rather than a column on it: a cast reads the caster's
/// lattice and writes the target's, and Bevy will not hand out two mutable borrows of
/// one query at once. Keeping them apart makes each access a short scope rather than a
/// lifetime puzzle.
///
/// **Deliberately unfiltered by `Downed`.** Filtering it would have been the obvious
/// thing and would have quietly broken the design: a downed unit is revivable by a
/// restoring spell, and a spell cannot restore a lattice it cannot reach. Being downed
/// stops a unit *acting*, which is the turn order's job, not its lattice's.
pub(super) type LatticeQuery<'w, 's> =
    Query<'w, 's, (&'static LatticeSpec, &'static mut LatticeState)>;
