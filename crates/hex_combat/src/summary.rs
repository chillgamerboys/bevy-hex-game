//! Deterministic, session-scoped combat reporting.

use std::collections::BTreeMap;

use bevy::prelude::*;
use hex_ai::AiDecisionTrace;
use hex_core::{GameCommand, Mode, Screen, UnitId};
use serde::{Deserialize, Serialize};

use crate::{AiDecisionTraces, CombatEvent, CombatSystems, EncounterOutcome, TurnOrder};

/// Stable command categories used by aggregate reporting.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CommandKind {
    /// One unit followed a path.
    Move,
    /// The exploring party followed coordinated routes atomically.
    MoveParty,
    /// One unit made a basic attack.
    Strike,
    /// One unit explicitly passed its turn.
    EndTurn,
    /// One unit cast a spell.
    Cast,
    /// One unit attempted to channel.
    Channel,
    /// A defender selected exact cells to disable.
    ChooseDisables,
    /// A restorer selected exact cells to renew.
    ChooseRestores,
    /// The exploring party restored all eligible cells.
    Rest,
}

impl CommandKind {
    fn of(command: &GameCommand) -> Self {
        match command {
            GameCommand::MoveAlong { .. } => Self::Move,
            GameCommand::MoveParty { .. } => Self::MoveParty,
            GameCommand::Strike { .. } => Self::Strike,
            GameCommand::EndTurn { .. } => Self::EndTurn,
            GameCommand::Cast { .. } => Self::Cast,
            GameCommand::Channel { .. } => Self::Channel,
            GameCommand::ChooseDisables { .. } => Self::ChooseDisables,
            GameCommand::ChooseRestores { .. } => Self::ChooseRestores,
            GameCommand::Rest { .. } => Self::Rest,
        }
    }
}

/// Stable summary of one gameplay session's combat and recovery stream.
#[derive(Resource, Serialize, Deserialize, Debug, Default, Clone, PartialEq, Eq)]
pub struct CombatSummary {
    /// One-based rounds reached while combat was active.
    pub rounds: u32,
    /// Successful commands by stable unit and semantic kind.
    pub commands: BTreeMap<UnitId, BTreeMap<CommandKind, u32>>,
    /// Algorithm dispatches in the order they selected commands.
    pub ai_selections: Vec<AiDecisionTrace>,
    /// Successful moves, including atomic party moves.
    pub moves: u32,
    /// Successful casts.
    pub casts: u32,
    /// Successful strikes.
    pub strikes: u32,
    /// Successful exact-cell answers.
    pub decisions: u32,
    /// Explicit end-turn commands.
    pub idle_turns: u32,
    /// Incoming disables before prevention.
    pub raw_disables: u32,
    /// Incoming disables absorbed by defences.
    pub prevented_disables: u32,
    /// Exact cells actually disabled.
    pub applied_disables: u32,
    /// Exact cells restored by spells or Rest.
    pub restored_cells: u32,
    /// Units revived by restoration.
    pub revivals: u32,
    /// Units downed.
    pub downings: u32,
    /// Final retained-world result.
    pub outcome: Option<EncounterOutcome>,
    /// Structured facts in simulation order for deterministic inspection.
    pub events: Vec<CombatEvent>,
}

impl CombatSummary {
    pub(crate) fn record_command(&mut self, command: &GameCommand) {
        let kind = CommandKind::of(command);
        *self
            .commands
            .entry(command.unit())
            .or_default()
            .entry(kind)
            .or_default() += 1;
        match kind {
            CommandKind::Move | CommandKind::MoveParty => self.moves += 1,
            CommandKind::Strike => self.strikes += 1,
            CommandKind::EndTurn => self.idle_turns += 1,
            CommandKind::Cast => self.casts += 1,
            CommandKind::ChooseDisables | CommandKind::ChooseRestores => self.decisions += 1,
            CommandKind::Channel | CommandKind::Rest => {}
        }
    }

    fn record_event(&mut self, event: &CombatEvent) {
        match event {
            CombatEvent::DecisionOpened { count, .. } => {
                self.raw_disables += u32::from(*count);
            }
            CombatEvent::DamagePrevented { amount, .. } => {
                self.raw_disables += u32::from(*amount);
                self.prevented_disables += u32::from(*amount);
            }
            CombatEvent::HexesDisabled { cells, .. } => {
                self.applied_disables += u32::try_from(cells.len()).unwrap_or(u32::MAX);
            }
            CombatEvent::HexesRestored { cells, .. } | CombatEvent::Rested { cells, .. } => {
                self.restored_cells += u32::try_from(cells.len()).unwrap_or(u32::MAX);
            }
            CombatEvent::Revived { .. } => self.revivals += 1,
            CombatEvent::Downed { .. } => self.downings += 1,
            CombatEvent::EncounterResolved { outcome } => self.outcome = Some(*outcome),
            _ => {}
        }
        self.events.push(event.clone());
    }
}

pub(crate) fn plugin(app: &mut App) {
    app.init_resource::<CombatSummary>()
        .add_systems(OnEnter(Screen::Gameplay), reset)
        .add_systems(OnExit(Screen::Gameplay), reset)
        .add_systems(
            Update,
            collect
                .after(CombatSystems::Advance)
                .run_if(in_state(Screen::Gameplay)),
        );
}

fn collect(
    mode: Res<State<Mode>>,
    order: Res<TurnOrder>,
    traces: Res<AiDecisionTraces>,
    mut events: MessageReader<CombatEvent>,
    mut summary: ResMut<CombatSummary>,
) {
    if *mode.get() == Mode::Combat {
        summary.rounds = summary.rounds.max(order.round.saturating_add(1));
    }
    let recorded_ai_selections = summary.ai_selections.len();
    if let Some(new_selections) = traces.entries.get(recorded_ai_selections..) {
        summary.ai_selections.extend_from_slice(new_selections);
    }
    for event in events.read() {
        summary.record_event(event);
    }
}

fn reset(mut summary: ResMut<CombatSummary>) {
    *summary = CombatSummary::default();
}

#[cfg(test)]
mod tests {
    use hex_core::{HexCoord, LatticeCoord, PartyPath, TilePos};

    use super::*;

    #[test]
    fn aggregate_counts_are_derived_from_successful_commands_and_facts() {
        let unit = UnitId(1);
        let target = UnitId(2);
        let cell = LatticeCoord::ORIGIN;
        let mut summary = CombatSummary::default();

        for command in [
            GameCommand::MoveParty {
                anchor: unit,
                paths: vec![PartyPath {
                    member: unit,
                    path: vec![TilePos::new(HexCoord::ORIGIN, 1)],
                }],
            },
            GameCommand::Cast {
                unit,
                spell: "Renewal".to_owned(),
                target: TilePos::new(HexCoord::ORIGIN, 1),
                facing: None,
                mana: None,
            },
            GameCommand::EndTurn { unit },
            GameCommand::ChooseRestores {
                unit,
                target,
                cells: vec![cell],
            },
        ] {
            summary.record_command(&command);
        }
        for event in [
            CombatEvent::DamagePrevented {
                source: target,
                target: unit,
                amount: 1,
            },
            CombatEvent::DecisionOpened {
                decider: unit,
                source: target,
                count: 2,
            },
            CombatEvent::HexesDisabled {
                source: target,
                target: unit,
                cells: vec![cell, LatticeCoord::new(1, 0)],
            },
            CombatEvent::HexesRestored {
                caster: unit,
                target,
                cells: vec![cell],
            },
            CombatEvent::Rested {
                unit,
                cells: vec![cell, LatticeCoord::new(1, 0)],
                refilled_mana: 3,
            },
            CombatEvent::Revived {
                unit: target,
                reenters_round: 4,
            },
            CombatEvent::Downed { unit: target },
            CombatEvent::EncounterResolved {
                outcome: EncounterOutcome::Victory,
            },
        ] {
            summary.record_event(&event);
        }

        assert_eq!(summary.moves, 1);
        assert_eq!(summary.casts, 1);
        assert_eq!(summary.decisions, 1);
        assert_eq!(summary.idle_turns, 1);
        assert_eq!(summary.raw_disables, 3);
        assert_eq!(summary.prevented_disables, 1);
        assert_eq!(summary.applied_disables, 2);
        assert_eq!(summary.restored_cells, 3);
        assert_eq!(summary.revivals, 1);
        assert_eq!(summary.downings, 1);
        assert_eq!(summary.outcome, Some(EncounterOutcome::Victory));
        assert_eq!(summary.events.len(), 8);
        assert_eq!(
            *summary
                .commands
                .get(&unit)
                .expect("the recorded unit should have command counts"),
            BTreeMap::from([
                (CommandKind::MoveParty, 1),
                (CommandKind::EndTurn, 1),
                (CommandKind::Cast, 1),
                (CommandKind::ChooseRestores, 1),
            ])
        );
    }

    #[test]
    fn a_summary_round_trips_for_replay_artifacts() {
        let mut summary = CombatSummary::default();
        summary.record_command(&GameCommand::Strike {
            unit: UnitId(3),
            target: UnitId(7),
        });
        summary.record_event(&CombatEvent::EncounterResolved {
            outcome: EncounterOutcome::Defeat,
        });

        let encoded = serde_json::to_string(&summary).expect("summary serializes");
        let decoded: CombatSummary = serde_json::from_str(&encoded).expect("summary deserializes");
        assert_eq!(decoded, summary);
    }
}
