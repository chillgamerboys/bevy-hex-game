//! Exploration-only whole-party recovery.

use bevy::prelude::*;
use hex_core::UnitId;
use hex_lattice::{rest, LatticeStats};
use hex_units::Downed;

use crate::{CombatEvent, CommandRefusal, UnitData};

use super::{cast::LatticeQuery, Verb};

pub(super) fn apply(
    ctx: &mut Verb,
    commands: &mut Commands,
    lattices: &mut LatticeQuery,
    stats: &Query<&LatticeStats>,
    unit: UnitId,
) -> Result<(), CommandRefusal> {
    if ctx.in_combat {
        return Err(CommandRefusal::RestExploringOnly);
    }
    if !ctx.party.members.contains(&unit) {
        return Err(CommandRefusal::RestUnavailable);
    }

    // Validate the entire roster before mutating any member.
    let mut members = Vec::with_capacity(ctx.party.members.len());
    for &member in &ctx.party.members {
        let Some(entity) = ctx.registry.entity_of(member) else {
            return Err(CommandRefusal::UnknownUnit);
        };
        if lattices.get(entity).is_err() || stats.get(entity).is_err() {
            return Err(CommandRefusal::MissingUnitData {
                unit: member,
                data: UnitData::Lattice,
            });
        }
        members.push((member, entity));
    }

    for (member, entity) in members {
        let Ok((spec, mut state)) = lattices.get_mut(entity) else {
            unreachable!("whole roster was validated immediately before recovery");
        };
        let Ok(stats) = stats.get(entity) else {
            unreachable!("whole roster was validated immediately before recovery");
        };
        let (cells, refilled_mana) = rest(spec, stats, &mut state);
        commands.entity(entity).remove::<Downed>();
        ctx.effects.remove_on(member);
        ctx.events.push(CombatEvent::Rested {
            unit: member,
            cells,
            refilled_mana,
        });
    }
    info!("rest: recovered {} party members", ctx.party.members.len());
    Ok(())
}
