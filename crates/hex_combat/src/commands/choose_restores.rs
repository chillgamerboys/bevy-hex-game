//! The caster's exact answer to a restoration decision.

use bevy::prelude::*;
use hex_core::{LatticeCoord, PendingDecision, UnitId};
use hex_lattice::restore;
use hex_units::Downed;

use crate::{CombatEvent, CommandRefusal, RestorationRefusal, UnitData};

use super::{cast::LatticeQuery, ActorQuery, Verb};

pub(super) fn apply(
    ctx: &mut Verb,
    commands: &mut Commands,
    actors: &mut ActorQuery,
    lattices: &mut LatticeQuery,
    caster: UnitId,
    target: UnitId,
    cells: &[LatticeCoord],
) -> Result<(), CommandRefusal> {
    let PendingDecision::ChooseRestores {
        decider,
        target: expected_target,
        count,
    } = *ctx.pending
    else {
        return Err(CommandRefusal::Restoration {
            reason: RestorationRefusal::NoDecision,
        });
    };
    if decider != caster {
        return Err(CommandRefusal::WrongDecisionUnit { expected: decider });
    }
    if target != expected_target {
        return Err(CommandRefusal::Restoration {
            reason: RestorationRefusal::WrongTarget {
                expected: expected_target,
            },
        });
    }
    let Some(target_entity) = ctx.registry.entity_of(target) else {
        return Err(CommandRefusal::UnknownUnit);
    };
    let Ok((spec, mut state)) = lattices.get_mut(target_entity) else {
        return Err(CommandRefusal::MissingUnitData {
            unit: target,
            data: UnitData::Lattice,
        });
    };
    let disabled = spec
        .cells()
        .filter(|&(coord, _)| state.is_disabled(coord))
        .count();
    let owed = usize::from(count).min(disabled);
    let actual = u16::try_from(cells.len()).unwrap_or(u16::MAX);
    if cells.len() != owed {
        return Err(CommandRefusal::Restoration {
            reason: RestorationRefusal::WrongCount {
                expected: u16::try_from(owed).unwrap_or(u16::MAX),
                actual,
            },
        });
    }
    let mut seen = Vec::with_capacity(cells.len());
    for &cell in cells {
        if spec.get(cell).is_none() {
            return Err(CommandRefusal::CellOutsideLattice { cell });
        }
        if seen.contains(&cell) {
            return Err(CommandRefusal::DuplicateCell { cell });
        }
        if !state.is_disabled(cell) {
            return Err(CommandRefusal::Restoration {
                reason: RestorationRefusal::CellNotDisabled { cell },
            });
        }
        seen.push(cell);
    }

    restore(&mut state, cells);
    *ctx.pending = PendingDecision::None;
    ctx.events.push(CombatEvent::HexesRestored {
        caster,
        target,
        cells: cells.to_vec(),
    });

    if cells.is_empty() {
        return Ok(());
    }
    if let Ok((.., is_downed)) = actors.get(target_entity) {
        if is_downed {
            commands.entity(target_entity).remove::<Downed>();
            let reenters_round = ctx.turn_order.round.saturating_add(1);
            ctx.revivals.schedule(target, reenters_round);
            ctx.events.push(CombatEvent::Revived {
                unit: target,
                reenters_round,
            });
        }
    }
    Ok(())
}
