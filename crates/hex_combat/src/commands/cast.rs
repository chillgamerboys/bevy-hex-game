//! Casting: the first three rungs of the legality ladder, and the damage that follows.
//!
//! # What this owns, and what it refuses
//!
//! `docs/systems/casting.md` specifies five rungs, checked in order. This file does
//! **1 to 5** for permanent terrain construction: whose turn it is (the funnel's
//! existing gate), whether the lattice can pay ([`castable`]), whether the target is in
//! range and the shape resolves, whether public construction policy admits the spell,
//! and whether exact occupancy permits emitting low-level [`TerrainEdit::Set`] requests.
//! Hidden obstruction suppresses edits without changing acceptance or payment. Elemental
//! material response and enchantment-bound terrain remain refused by name rather than
//! silently skipped.
//!
//! # Casting is committing
//!
//! A cast that passes the ladder spends its mana and takes the action, whether or not
//! the effect finds anything to do. A fireball into an empty hex is a legal cast that
//! burns nothing. That payment policy is provisional and the design says so, but the
//! order matters: the lattice is drained *before* effects resolve, so an effect that
//! turns out to be a no-op has still cost what it cost.

use bevy::prelude::*;

use hex_assets::{CastingAxis, Effect, Spell, TargetShape};
use hex_core::{
    KnowledgeExpiry, KnowledgeState, LatticeCoord, PendingDecision, TerrainEdit, TilePos, UnitId,
};
use hex_lattice::{apply_cast, castable, CastBlocked, CellKind, LatticeSpec, LatticeState};
use hex_units::{
    resolve_creation_volume, targeting, validate_creation_volume, volumes, CreationBody,
};

use crate::{CastBlockReason, CombatData, CombatEvent, CommandRefusal, UnitData};

use super::{presentation, ActorQuery, Verb};

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
    terrain_edits: &mut MessageWriter<TerrainEdit>,
    commands: &mut Commands,
    actors: &mut ActorQuery,
    lattices: &mut LatticeQuery,
    unit: UnitId,
    entity: Entity,
    spell_name: &str,
    target: TilePos,
    facing: Option<hex_core::Sextant>,
) -> Result<(), CommandRefusal> {
    if !ctx.in_combat {
        return Err(CommandRefusal::CombatOnly);
    }
    if ctx.turn_order.current() != Some(unit) {
        return Err(CommandRefusal::NotCurrentTurn {
            current: ctx.turn_order.current(),
        });
    }
    // One decision at a time. A second cast landing while a defender still owes an
    // answer would resolve its damage against a lattice that is about to change.
    if ctx.pending.is_open() {
        let decider = match *ctx.pending {
            PendingDecision::ChooseDisables { decider, .. }
            | PendingDecision::ChooseRestores { decider, .. } => decider,
            PendingDecision::None => unit,
        };
        return Err(CommandRefusal::DecisionPending { decider });
    }

    let Some(book) = ctx.spells else {
        return Err(CommandRefusal::MissingCombatData {
            data: CombatData::SpellBook,
        });
    };
    let Some(tables) = ctx.tables.as_ref() else {
        return Err(CommandRefusal::MissingCombatData {
            data: CombatData::ContentTables,
        });
    };
    let Some(spell) = book.id(spell_name) else {
        return Err(CommandRefusal::UnknownSpell {
            spell: spell_name.to_owned(),
        });
    };
    let Some(spec) = book.spell(spell) else {
        return Err(CommandRefusal::MissingSpellDefinition {
            spell: spell_name.to_owned(),
        });
    };
    if spec
        .effects
        .iter()
        .any(|effect| matches!(effect, Effect::Reveal { .. }))
        && ctx.combat.is_none()
    {
        return Err(CommandRefusal::MissingCombatData {
            data: CombatData::CombatSettings,
        });
    }
    // Rung 0: does casting this *do* anything yet. Checked before the action, the range
    // and above all the payment, because a spell whose every effect is still waiting on
    // another lane would otherwise spend the caster's mana and its whole turn and produce
    // nothing but `warn!` lines — invisible in a release build, where the console is
    // hidden. See `delivers_anything`.
    if !delivers_anything(spec) {
        return Err(CommandRefusal::UndeliverableSpell {
            spell: spell_name.to_owned(),
        });
    }

    let (standing, busy, caster_faction) = {
        let Ok((standing, _, turn, busy, _, faction, _)) = actors.get(entity) else {
            return Err(CommandRefusal::MissingUnitData {
                unit,
                data: UnitData::EntityRecord,
            });
        };
        // Rung 1 is "action available", and casting is an action. Without this a unit
        // casts every frame its turn lasts. Presentation lifetime cannot be the gate:
        // `advance_turn` also waits for the movement budget, so without this explicit
        // domain flag the whole lattice could be spent in one turn.
        let Some(turn) = turn else {
            return Err(CommandRefusal::NoTurn);
        };
        if turn.acted {
            return Err(CommandRefusal::ActionAlreadySpent);
        }
        let Some(standing) = standing.copied() else {
            return Err(CommandRefusal::MissingUnitData {
                unit,
                data: UnitData::Standing,
            });
        };
        let Some(faction) = faction.copied() else {
            return Err(CommandRefusal::MissingUnitData {
                unit,
                data: UnitData::Faction,
            });
        };
        (standing.0, busy, faction)
    };
    if busy || ctx.committed.contains(&entity) {
        return Err(CommandRefusal::Busy);
    }

    // --- rung 3: targeting -------------------------------------------------

    // Directed shapes need a facing, and a directed cast without one is a malformed
    // command rather than a cast that reaches nothing.
    if volumes::needs_facing(&spec.targeting.shape) && facing.is_none() {
        return Err(CommandRefusal::MissingFacing {
            spell: spell_name.to_owned(),
        });
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
            return Err(CommandRefusal::TargetOutOfRange {
                spell: spell_name.to_owned(),
                target,
            });
        }
    }
    // Resolved but not yet consumed: rungs 4 and 5 are what read a volume, and both
    // are terrain magic's. Resolving it here anyway is the cheap half of the check —
    // a shape that cannot resolve is a cast that should not have been legal.
    if volumes::resolve(&spec.targeting.shape, standing.pos, target, facing).is_none() {
        return Err(CommandRefusal::ShapeUnresolved {
            spell: spell_name.to_owned(),
            target,
        });
    }

    let Some(spatial) = ctx.spatial else {
        return Err(CommandRefusal::MissingCombatData {
            data: CombatData::SpatialKnowledge,
        });
    };
    if spatial.faction(caster_faction).state(target) != KnowledgeState::Observed {
        return Err(CommandRefusal::TargetUnobserved {
            spell: spell_name.to_owned(),
            target,
        });
    }
    let target_unit = unit_standing_on(ctx, actors, target);
    if let Some((target, _, true)) = target_unit {
        if spec.effects.iter().any(effect_damages_unit) {
            return Err(CommandRefusal::TargetDowned { target });
        }
    }

    // Terrain creation is the only effect with a pre-payment public-policy gate. Its
    // hidden authoritative obstruction check may suppress the edit batch, but never
    // changes acceptance or payment. Retain any safe low-level edits for emission only
    // after the lattice plan succeeds.
    let creation_edits =
        plan_terrain_creation(ctx, actors, spell_name, spec, standing.pos, target, facing)?;

    // --- rung 2: the lattice -----------------------------------------------

    let cell = {
        let Ok((caster_spec, caster_state)) = lattices.get(entity) else {
            return Err(CommandRefusal::MissingUnitData {
                unit,
                data: UnitData::Lattice,
            });
        };
        spell_cell(caster_spec, caster_state, spell).ok_or_else(|| {
            CommandRefusal::SpellNotInscribed {
                spell: spell_name.to_owned(),
            }
        })?
    };

    let plan = {
        let Ok((caster_spec, caster_state)) = lattices.get(entity) else {
            return Err(CommandRefusal::MissingUnitData {
                unit,
                data: UnitData::Lattice,
            });
        };
        castable(caster_spec, caster_state, cell, tables).map_err(|blocked| {
            let reason = match blocked {
                CastBlocked::NotASpell => CastBlockReason::NotASpell,
                CastBlocked::SpellDisabled => CastBlockReason::SpellDisabled,
                CastBlocked::Unsatisfiable => CastBlockReason::Unsatisfiable,
            };
            CommandRefusal::CastBlocked {
                spell: spell_name.to_owned(),
                reason,
            }
        })?
    };

    // Committing. `apply_cast` returns `false` when the plan went stale between
    // planning and applying — nothing mutated, and a caller that ignored it would
    // silently eat the cast along with the player's turn.
    {
        let Ok((_, mut caster_state)) = lattices.get_mut(entity) else {
            return Err(CommandRefusal::MissingUnitData {
                unit,
                data: UnitData::Lattice,
            });
        };
        if !apply_cast(&mut caster_state, &plan, tables) {
            return Err(CommandRefusal::CastPlanStale {
                spell: spell_name.to_owned(),
            });
        }
    }
    ctx.events.push(CombatEvent::Cast {
        caster: unit,
        spell: spell_name.to_owned(),
        target,
    });
    terrain_edits.write_batch(creation_edits);

    // --- effects -----------------------------------------------------------

    let round = ctx.turn_order.round;
    let mut refusals: Vec<&'static str> = Vec::new();
    let mut played_direct_recoil = false;
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
                let landed = open_disable_decision(
                    ctx,
                    lattices,
                    defender.0,
                    defender.1,
                    unit,
                    u16::from(*count),
                );
                if landed && !played_direct_recoil {
                    if let (Some(settings), Ok((Some(target_standing), ..))) =
                        (ctx.settings, actors.get(defender.1))
                    {
                        presentation::recoil(
                            commands,
                            defender.1,
                            target_standing.0,
                            standing,
                            settings.speed,
                        );
                        played_direct_recoil = true;
                    }
                }
            }
            Effect::Reveal { tier } => {
                let Some(subject) = target_unit else {
                    // An empty observed anchor is still a legal cast. It reveals no
                    // lattice because there is no subject there.
                    continue;
                };
                let Ok((target_spec, target_state)) = lattices.get(subject.1) else {
                    refusals.push("the target has no lattice to reveal");
                    continue;
                };
                let rounds = ctx.combat.map_or(0, |settings| {
                    settings
                        .divination_rounds_per_tier
                        .saturating_mul(u32::from(*tier))
                });
                let Some(cells) = ctx.knowledge.reveal(
                    caster_faction,
                    subject.0,
                    target_spec,
                    target_state,
                    KnowledgeExpiry::Rounds(rounds),
                ) else {
                    refusals.push("the target has no published base visibility");
                    continue;
                };
                ctx.events.push(CombatEvent::Revealed {
                    viewer: caster_faction,
                    subject: subject.0,
                    cells,
                    rounds,
                });
            }
            Effect::Illuminate { .. } => refusals.push("Illuminate waits on the perception lane"),
            Effect::ClearTerrain => refusals.push("legacy ClearTerrain is decode-only"),
            Effect::SetTerrain { .. } | Effect::SpawnWall { .. } => {}
            Effect::Burn { turns } => {
                let Some(defender) = target_unit else {
                    // Same rule as `DisableHexes` above: a spell that reaches nobody is
                    // a legal cast that set nothing alight, and it is already paid for.
                    continue;
                };
                // **Nothing goes down now, and the lattice is not told.** Burn's whole
                // shape is that it arrives at the start of each of the target's own
                // turns, so what a cast does is open a ledger entry; `crate::effects`
                // collects on it and routes the result through the same
                // defender-chooses seam as any other damage.
                //
                // A lattice-less target cannot be set alight — `hex_units` spawns a unit
                // inert when its archetype names no lattice, and the design's answer for
                // one is "playable but cannot be damaged". Booking the fire anyway would
                // charge the caster for damage that has nowhere to land, so this refuses
                // by name. `effects::open_due_decision` guards the same case again at the
                // seam, because a ledger entry can outlive the components it was made
                // against.
                if lattices.get(defender.1).is_err() {
                    refusals.push("the target has no lattice to set alight");
                    continue;
                }
                crate::effects::apply_burn(ctx.effects, round, unit, defender.0, *turns);
                ctx.events.push(CombatEvent::BurnApplied {
                    source: unit,
                    target: defender.0,
                    turns: *turns,
                });
                info!(
                    "cast: {unit:?} sets {:?} alight for {turns} of its turns",
                    defender.0
                );
            }
            Effect::RestoreHexes { count } => {
                let Some((target_unit, target_entity, _)) = target_unit else {
                    continue;
                };
                let Ok((target_spec, target_state)) = lattices.get(target_entity) else {
                    refusals.push("the restoration target has no lattice");
                    continue;
                };
                let disabled = target_spec
                    .cells()
                    .filter(|&(coord, _)| target_state.is_disabled(coord))
                    .count();
                let owed = usize::from(*count).min(disabled);
                if owed > 0 {
                    *ctx.pending = PendingDecision::ChooseRestores {
                        decider: unit,
                        target: target_unit,
                        count: u16::try_from(owed).unwrap_or(u16::MAX),
                    };
                }
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
pub(crate) fn spell_cell(
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
) -> bool {
    let Ok((_, state)) = lattices.get(defender_entity) else {
        return false;
    };
    let count = hex_lattice::resolve_incoming(state, raw);
    let prevented = raw.saturating_sub(count);
    if prevented > 0 {
        ctx.events.push(CombatEvent::DamagePrevented {
            source,
            target: defender_id,
            amount: prevented,
        });
    }
    if count == 0 {
        info!("cast: {defender_id:?} absorbed the whole hit");
        return false;
    }
    *ctx.pending = PendingDecision::ChooseDisables {
        decider: defender_id,
        count,
        source,
    };
    ctx.events.push(CombatEvent::DecisionOpened {
        decider: defender_id,
        source,
        count,
    });
    true
}

/// The refusal a spell gets when nothing it does is built yet.
///
/// A `&'static str` shared with the interface rather than each writing its own, so the
/// panel's reason and the applier's are the same sentence — and so a reader grepping for
/// one finds both.
pub const UNDELIVERABLE: &str = "nothing this spell does is built yet";

/// Whether the applier delivers **any** of a spell's effects today.
///
/// The gate the interface and the applier share, and the reason it exists is a specific
/// failure: several shipped spells — Earthen Wall, Stone Shaper, Daylight — are legal
/// casts whose every effect is still waiting on a lane that has not landed. Offering
/// one is worse than hiding it. The cast is legal, so it is charged: the mana goes, the
/// turn goes, and the only trace is a log line the player cannot see.
///
/// **Any, not all.** A partially built spell still does something, and refusing it would
/// take away a real effect because a second one is pending; the applier already reports
/// each unbuilt effect it skips. A spell with no effects is ordinarily undeliverable,
/// except for a positive-defense enchantment: applying that enchantment is itself the
/// delivered result even when its effect list is empty.
///
/// Kept beside the match it mirrors so the two move together. Adding an effect arm above
/// without adding it here fails closed, which is the safe direction: the spell stays
/// unoffered until somebody notices.
#[must_use]
pub fn delivers_anything(spell: &Spell) -> bool {
    matches!(
        spell.casting,
        CastingAxis::Enchantment { defense } if defense > 0
    ) || spell.effects.iter().any(|effect| match effect {
        // Damage and restoration both park on an exact-cell decision; fire lands too.
        Effect::DisableHexes { targeted, .. } => !targeted,
        Effect::RestoreHexes { .. } => true,
        Effect::Burn { .. } => true,
        // Everything below is refused by name in the match above; see the reasons there.
        Effect::Reveal { .. } => true,
        Effect::SetTerrain { .. } | Effect::SpawnWall { .. } => {
            // The built edit path is permanent. Enchantment manifestations promise
            // removal when their binding breaks, and no voxel provenance/removal
            // ledger exists yet, so those remain fail-closed rather than becoming
            // immortal "bound" terrain.
            matches!(spell.casting, CastingAxis::Evocation)
        }
        Effect::Illuminate { .. }
        | Effect::ClearTerrain
        | Effect::ModifyIncomingDisables { .. }
        | Effect::Displace { .. } => false,
    })
}

/// Resolves public construction policy and plans any authoritatively safe edit batch.
///
/// Detailed obstruction data never crosses into [`CommandRefusal`]: the resolved
/// volume may extend into hidden space, so identifying its blocker would disclose
/// authoritative terrain or a hidden unit.
fn plan_terrain_creation(
    ctx: &Verb,
    actors: &ActorQuery,
    spell_name: &str,
    spell: &Spell,
    caster: TilePos,
    selected_surface: TilePos,
    facing: Option<hex_core::Sextant>,
) -> Result<Vec<TerrainEdit>, CommandRefusal> {
    if !spell
        .effects
        .iter()
        .any(|effect| matches!(effect, Effect::SetTerrain { .. } | Effect::SpawnWall { .. }))
    {
        return Ok(Vec::new());
    }

    let Some(table) = ctx.table else {
        return Err(CommandRefusal::MissingCombatData {
            data: CombatData::SubstanceTable,
        });
    };
    if !terrain_creation_is_admitted(spell, table) {
        return Err(CommandRefusal::TerrainCreationBlocked {
            spell: spell_name.to_owned(),
        });
    }
    let Some(terrain) = ctx.terrain else {
        return Err(CommandRefusal::MissingCombatData {
            data: CombatData::TerrainOccupancy,
        });
    };
    let Some(volume) =
        resolve_creation_volume(&spell.targeting.shape, caster, selected_surface, facing)
    else {
        return Err(CommandRefusal::ShapeUnresolved {
            spell: spell_name.to_owned(),
            target: selected_surface,
        });
    };
    let bodies: Vec<_> = ctx
        .registry
        .iter()
        .filter_map(|(unit, entity)| {
            let (standing, body, ..) = actors.get(entity).ok()?;
            Some(CreationBody {
                unit,
                support: standing?.0.pos,
                body: *body?,
            })
        })
        .collect();
    if validate_creation_volume(&volume, terrain, bodies).is_err() {
        // Hidden terrain and units must not become a yes/no or payment oracle. The
        // cast remains accepted and paid exactly as it would in known-clear space,
        // while authority atomically withholds the unsafe low-level edit batch.
        return Ok(Vec::new());
    }

    let mut edits = Vec::new();
    for effect in &spell.effects {
        let substance = match effect {
            Effect::SetTerrain { substance } | Effect::SpawnWall { substance } => substance,
            _ => continue,
        };
        let Some(substance) = table.id(substance).filter(|id| table.is_conjurable(*id)) else {
            // Content admission normally makes this unreachable. Fail closed anyway
            // rather than letting a stale or hand-built harness bypass world policy.
            return Err(CommandRefusal::TerrainCreationBlocked {
                spell: spell_name.to_owned(),
            });
        };
        edits.extend(
            volume
                .iter()
                .copied()
                .map(|pos| TerrainEdit::Set { pos, substance }),
        );
    }
    Ok(edits)
}

/// Whether public content policy admits a permanent construction spell.
///
/// This gate depends only on the spell definition and published substance policy,
/// never on hidden terrain or unit truth, so AI enumeration can share it safely.
pub(crate) fn terrain_creation_is_admitted(
    spell: &Spell,
    table: &hex_assets::SubstanceTable,
) -> bool {
    let mut creations = spell.effects.iter().filter_map(|effect| match effect {
        Effect::SetTerrain { substance } | Effect::SpawnWall { substance } => Some(substance),
        _ => None,
    });
    let Some(substance) = creations.next() else {
        return true;
    };
    creations.next().is_none()
        && spell.effects.len() == 1
        && matches!(spell.casting, CastingAxis::Evocation)
        && matches!(
            spell.targeting.shape,
            TargetShape::Single | TargetShape::Column { .. }
        )
        && table
            .id(substance)
            .is_some_and(|id| table.is_conjurable(id))
}

/// The unit standing on `pos`, if any.
fn unit_standing_on(
    ctx: &Verb,
    actors: &ActorQuery,
    pos: TilePos,
) -> Option<(UnitId, Entity, bool)> {
    ctx.registry.iter().find_map(|(id, entity)| {
        let (standing, _, _, _, _, _, downed) = actors.get(entity).ok()?;
        (standing?.0.pos == pos).then_some((id, entity, downed))
    })
}

/// Whether this implemented effect would further damage a unit on the anchor.
///
/// Downed lattices stay queryable because restoration needs them. Damage is a
/// narrower rule: it refuses a spent target before payment instead of opening a
/// defender choice that can only answer with zero cells.
fn effect_damages_unit(effect: &Effect) -> bool {
    matches!(
        effect,
        Effect::DisableHexes {
            targeted: false,
            ..
        } | Effect::Burn { .. }
    )
}

/// Lattices, as the applier reaches them.
///
/// Separate from [`ActorQuery`] rather than a column on it: a cast reads the caster's
/// lattice and writes the target's, and Bevy will not hand out two mutable borrows of
/// one query at once. Keeping them apart makes each access a short scope rather than a
/// lifetime puzzle.
///
/// **Deliberately unfiltered by `Downed`.** Renewal needs access to the retained lattice
/// before it can reactivate the target. Being downed stops a unit *acting*, which is the
/// turn order's job, not its lattice's.
pub(super) type LatticeQuery<'w, 's> =
    Query<'w, 's, (&'static LatticeSpec, &'static mut LatticeState)>;

#[cfg(test)]
mod tests {
    use hex_assets::{
        CastingAxis, Effect, GemRequirement, ManaAxis, Spell, TargetShape, TargetingSpec,
    };
    use hex_core::{LatticeCoord, SpellId};
    use hex_lattice::{apply_disables, CellKind, LatticeSpec, LatticeState, LatticeStats};
    use hex_test_support::fixture_assets;

    use super::{spell_cell, terrain_creation_is_admitted};

    fn construction(casting: CastingAxis, shape: TargetShape) -> Spell {
        Spell {
            requirements: vec![GemRequirement {
                element: "Earth".to_owned(),
                mana: 1,
            }],
            casting,
            mana: ManaAxis::Fixed,
            co_castable: false,
            targeting: TargetingSpec {
                range: 2,
                shape,
                needs_los: false,
            },
            effects: vec![Effect::SpawnWall {
                substance: "stone".to_owned(),
            }],
        }
    }

    #[test]
    fn redundant_spell_resolution_prefers_a_live_copy() {
        let spell = SpellId(4);
        let first = LatticeCoord::ORIGIN;
        let second = LatticeCoord::new(1, 0);
        let spec = LatticeSpec::default()
            .with(first, CellKind::Spell { spell })
            .with(second, CellKind::Spell { spell });
        let mut state = LatticeState::new(&spec, &LatticeStats::default());
        apply_disables(&mut state, &[first]);

        assert_eq!(spell_cell(&spec, &state, spell), Some(second));
    }

    #[test]
    fn ai_and_authority_share_public_construction_admission() {
        let (_, substances) = fixture_assets().expect("the test substance table resolves");
        assert!(terrain_creation_is_admitted(
            &construction(CastingAxis::Evocation, TargetShape::Column { height: 2 }),
            &substances,
        ));
        assert!(!terrain_creation_is_admitted(
            &construction(
                CastingAxis::Enchantment { defense: 1 },
                TargetShape::Column { height: 2 }
            ),
            &substances,
        ));
        assert!(!terrain_creation_is_admitted(
            &construction(CastingAxis::Evocation, TargetShape::Sphere { radius: 1 }),
            &substances,
        ));
    }
}
