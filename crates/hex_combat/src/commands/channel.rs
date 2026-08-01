//! Spending one combat action to refill the acting unit's lattice.

use std::collections::BTreeMap;

use bevy::prelude::*;
use hex_assets::ElementCatalog;
use hex_core::{Turn, UnitId};
use hex_lattice::{channel, CellKind, LatticeSpec, LatticeStats};

use crate::{CombatData, CombatEvent, CommandRefusal, UnitData};

use super::{cast::LatticeQuery, ActorQuery, Verb};

/// Immutable command facts used by both Channel presentation and application.
///
/// Keeping this contract in combat prevents a UI projection from maintaining a
/// second, inevitably drifting list of Channel preconditions. Full mana is
/// intentionally absent: the current authority permits Channel at full mana and
/// spends the action even when it restores nothing.
#[derive(Debug, Clone, Copy)]
pub struct ChannelReadiness<'a> {
    /// Whether combat is currently active.
    pub in_combat: bool,
    /// Stable identity of the proposed actor.
    pub unit: UnitId,
    /// Stable identity currently holding initiative.
    pub current: Option<UnitId>,
    /// Whether the actor is downed.
    pub downed: bool,
    /// Whether an earlier domain action is still settling.
    pub busy: bool,
    /// The actor's current turn budget, when present.
    pub turn: Option<&'a Turn>,
    /// The actor's lattice shape, when present.
    pub lattice: Option<&'a LatticeSpec>,
    /// The actor's lattice statistics, when present.
    pub stats: Option<&'a LatticeStats>,
    /// Loaded element identities required to report restored mana.
    pub elements: Option<&'a ElementCatalog>,
}

/// Returns the exact refusal Channel would produce from immutable command facts.
///
/// The command applier calls this before mutation, and presentation consumers may
/// call it to publish an enabled action only when the same command gate agrees.
#[must_use]
pub fn channel_refusal(readiness: ChannelReadiness<'_>) -> Option<CommandRefusal> {
    if !readiness.in_combat {
        return Some(CommandRefusal::CombatOnly);
    }
    if readiness.downed {
        return Some(CommandRefusal::ActingUnitDowned {
            unit: readiness.unit,
        });
    }
    if readiness.current != Some(readiness.unit) {
        return Some(CommandRefusal::NotCurrentTurn {
            current: readiness.current,
        });
    }
    if readiness.busy {
        return Some(CommandRefusal::Busy);
    }
    let Some(turn) = readiness.turn else {
        return Some(CommandRefusal::NoTurn);
    };
    if turn.acted {
        return Some(CommandRefusal::ActionAlreadySpent);
    }
    let Some(lattice) = readiness.lattice else {
        return Some(CommandRefusal::MissingUnitData {
            unit: readiness.unit,
            data: UnitData::Lattice,
        });
    };
    if readiness.stats.is_none() {
        return Some(CommandRefusal::MissingUnitData {
            unit: readiness.unit,
            data: UnitData::Lattice,
        });
    }
    let Some(elements) = readiness.elements else {
        return Some(CommandRefusal::MissingCombatData {
            data: CombatData::ElementCatalog,
        });
    };
    if lattice.cells().any(
        |(_, kind)| matches!(kind, CellKind::Gem { element } if elements.name(element).is_none()),
    ) {
        return Some(CommandRefusal::MissingCombatData {
            data: CombatData::ElementCatalog,
        });
    }
    None
}

/// Applies Channel, or returns the exact reason it was refused.
pub(super) fn apply(
    ctx: &mut Verb,
    actors: &mut ActorQuery,
    lattices: &mut LatticeQuery,
    stats: &Query<&LatticeStats>,
    unit: UnitId,
    entity: Entity,
) -> Result<(), CommandRefusal> {
    // Preserve the shared funnel's established refusal precedence: mode is known
    // before an entity lookup and therefore wins over incomplete unit fixtures.
    if !ctx.in_combat {
        return Err(CommandRefusal::CombatOnly);
    }
    let Ok((_, _, turn, busy, _, _, downed)) = actors.get_mut(entity) else {
        return Err(CommandRefusal::MissingUnitData {
            unit,
            data: UnitData::EntityRecord,
        });
    };
    let mut lattice = lattices.get_mut(entity).ok();
    let lattice_stats = stats.get(entity).ok();
    if let Some(refusal) = channel_refusal(ChannelReadiness {
        in_combat: true,
        unit,
        current: ctx.turn_order.current(),
        downed,
        busy: busy || ctx.committed.contains(&entity),
        turn: turn.as_deref(),
        lattice: lattice.as_ref().map(|(spec, _)| *spec),
        stats: lattice_stats,
        elements: ctx.elements,
    }) {
        return Err(refusal);
    }
    let Some(mut turn) = turn else {
        unreachable!("Channel readiness accepted a missing turn")
    };
    let Some((spec, mut state)) = lattice.take() else {
        unreachable!("Channel readiness accepted a missing lattice")
    };
    let Some(stats) = lattice_stats else {
        unreachable!("Channel readiness accepted missing lattice stats")
    };
    let Some(elements) = ctx.elements else {
        unreachable!("Channel readiness accepted a missing element catalog")
    };

    let mut restored = BTreeMap::new();
    for (element, amount) in channel(&mut state, spec, stats) {
        let Some(name) = elements.name(element) else {
            unreachable!("every gem element was validated before Channel mutated the lattice")
        };
        restored.insert(name.to_owned(), amount);
    }
    turn.acted = true;
    ctx.events.push(CombatEvent::Channelled { unit, restored });
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use bevy::platform::collections::HashMap;
    use hex_assets::ElementFile;
    use hex_core::{ElementId, LatticeCoord};
    use hex_lattice::LatticeState;

    use super::*;

    fn elements() -> ElementCatalog {
        ElementCatalog::from_file(&ElementFile {
            wheel: vec!["fire".to_owned(), "water".to_owned()],
            fusions: HashMap::default(),
        })
    }

    fn ready<'a>(
        turn: &'a Turn,
        lattice: Option<&'a LatticeSpec>,
        stats: Option<&'a LatticeStats>,
        elements: &'a ElementCatalog,
    ) -> ChannelReadiness<'a> {
        ChannelReadiness {
            in_combat: true,
            unit: UnitId(1),
            current: Some(UnitId(1)),
            downed: false,
            busy: false,
            turn: Some(turn),
            lattice,
            stats,
            elements: Some(elements),
        }
    }

    #[test]
    fn busy_and_missing_lattice_are_exact_shared_refusals() {
        let turn = Turn {
            movement_left: 2,
            acted: false,
        };
        let elements = elements();
        let mut busy = ready(&turn, None, None, &elements);
        busy.busy = true;
        assert_eq!(channel_refusal(busy), Some(CommandRefusal::Busy));
        assert_eq!(
            channel_refusal(ready(&turn, None, None, &elements)),
            Some(CommandRefusal::MissingUnitData {
                unit: UnitId(1),
                data: UnitData::Lattice,
            })
        );
    }

    #[test]
    fn full_mana_does_not_invent_a_refusal_the_authority_does_not_have() {
        let element = ElementId(0);
        let spec = LatticeSpec::default().with(LatticeCoord::ORIGIN, CellKind::Gem { element });
        let stats = LatticeStats::new(
            BTreeMap::from([(element, 3)]),
            BTreeMap::from([(element, 2)]),
        );
        let state = LatticeState::new(&spec, &stats);
        assert_eq!(state.mana(LatticeCoord::ORIGIN), 3);
        let turn = Turn {
            movement_left: 2,
            acted: false,
        };
        assert_eq!(
            channel_refusal(ready(&turn, Some(&spec), Some(&stats), &elements())),
            None
        );
    }
}
