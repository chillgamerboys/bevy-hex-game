//! Deterministic, session-scoped combat reporting.

use std::collections::{BTreeMap, VecDeque};

use bevy::prelude::*;
use hex_ai::{AiDecisionRecord, AiDecisionTrace};
use hex_core::{GameCommand, Mode, Screen, UnitId};
use serde::{Deserialize, Serialize};
use xxhash_rust::xxh3::xxh3_64;

use crate::{AiDecisionTraces, CombatEvent, CombatSystems, EncounterOutcome, TurnOrder};

/// Maximum compact AI records and structured events retained by [`CombatSummary`].
pub const MAX_COMBAT_SUMMARY_DETAILS: usize = 4_096;

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
    /// Most recent compact algorithm dispatches in selection order.
    pub ai_selections: VecDeque<AiDecisionRecord>,
    /// Total algorithm dispatches, including records outside the retained window.
    #[serde(default)]
    pub ai_selection_count: u64,
    /// Rolling deterministic fingerprint of every algorithm dispatch.
    #[serde(default)]
    pub ai_selection_fingerprint: u64,
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
    /// Most recent structured facts in simulation order.
    pub events: VecDeque<CombatEvent>,
    /// Total structured facts, including events outside the retained window.
    #[serde(default)]
    pub event_count: u64,
    /// Rolling deterministic fingerprint of every structured fact.
    #[serde(default)]
    pub event_fingerprint: u64,
    /// Next live AI trace sequence to collect. Runtime-only, never an artifact field.
    #[serde(skip)]
    ai_trace_cursor: u64,
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
        self.event_fingerprint = rolling_fingerprint(
            b"combat-event-v1",
            self.event_fingerprint,
            self.event_count,
            event,
        );
        self.event_count = self.event_count.saturating_add(1);
        push_bounded(&mut self.events, event.clone());
    }

    fn record_ai_selection(&mut self, trace: &AiDecisionTrace) {
        let record = AiDecisionRecord::from(trace);
        self.ai_selection_fingerprint = rolling_fingerprint(
            b"ai-selection-v1",
            self.ai_selection_fingerprint,
            self.ai_selection_count,
            &record,
        );
        self.ai_selection_count = self.ai_selection_count.saturating_add(1);
        self.ai_trace_cursor = trace.sequence.saturating_add(1);
        push_bounded(&mut self.ai_selections, record);
    }
}

fn push_bounded<T>(values: &mut VecDeque<T>, value: T) {
    while values.len() >= MAX_COMBAT_SUMMARY_DETAILS {
        let _ = values.pop_front();
    }
    values.push_back(value);
}

fn rolling_fingerprint(domain: &[u8], previous: u64, ordinal: u64, value: &impl Serialize) -> u64 {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(domain);
    bytes.extend_from_slice(&previous.to_le_bytes());
    bytes.extend_from_slice(&ordinal.to_le_bytes());
    if serde_json::to_writer(&mut bytes, value).is_err() {
        // Every summary input derives Serialize and Vec writes cannot fail. Keeping a
        // deterministic marker still makes the function total if that contract ever
        // changes, rather than silently reusing the previous fingerprint.
        bytes.extend_from_slice(b"<serialization-error>");
    }
    xxh3_64(&bytes)
}

/// Opt-in unbounded transcript for tests and diagnostic tooling.
///
/// Shipping sessions keep this disabled and retain only bounded summary/live windows.
/// Enabling it is an explicit request to trade memory for every exact observation,
/// legal domain, selection, and combat event in the session.
#[derive(Resource, Debug, Default)]
pub struct CombatTranscriptRecorder {
    enabled: bool,
    ai_trace_cursor: u64,
    ai_selections: Vec<AiDecisionTrace>,
    events: Vec<CombatEvent>,
}

impl CombatTranscriptRecorder {
    /// Enables full recording and clears any earlier transcript.
    pub fn enable(&mut self) {
        self.enabled = true;
        self.ai_trace_cursor = 0;
        self.ai_selections.clear();
        self.events.clear();
    }

    /// Disables and clears full recording.
    pub fn disable(&mut self) {
        self.enabled = false;
        self.ai_trace_cursor = 0;
        self.ai_selections.clear();
        self.events.clear();
    }

    /// Whether full recording is active.
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Exact AI dispatches recorded since enabling.
    #[must_use]
    pub fn ai_selections(&self) -> &[AiDecisionTrace] {
        &self.ai_selections
    }

    /// Exact structured events recorded since enabling.
    #[must_use]
    pub fn events(&self) -> &[CombatEvent] {
        &self.events
    }

    fn reset_session(&mut self) {
        self.ai_trace_cursor = 0;
        self.ai_selections.clear();
        self.events.clear();
    }
}

pub(crate) fn plugin(app: &mut App) {
    app.init_resource::<CombatSummary>()
        .init_resource::<CombatTranscriptRecorder>()
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
    mut transcript: ResMut<CombatTranscriptRecorder>,
) {
    if *mode.get() == Mode::Combat {
        summary.rounds = summary.rounds.max(order.round.saturating_add(1));
    }
    let summary_cursor = summary.ai_trace_cursor;
    for trace in traces
        .entries
        .iter()
        .filter(|trace| trace.sequence >= summary_cursor)
    {
        summary.record_ai_selection(trace);
    }
    if transcript.enabled {
        let transcript_cursor = transcript.ai_trace_cursor;
        for trace in traces
            .entries
            .iter()
            .filter(|trace| trace.sequence >= transcript_cursor)
        {
            transcript.ai_trace_cursor = trace.sequence.saturating_add(1);
            transcript.ai_selections.push(trace.clone());
        }
    }
    for event in events.read() {
        summary.record_event(event);
        if transcript.enabled {
            transcript.events.push(event.clone());
        }
    }
}

fn reset(mut summary: ResMut<CombatSummary>, mut transcript: ResMut<CombatTranscriptRecorder>) {
    *summary = CombatSummary::default();
    transcript.reset_session();
}

#[cfg(test)]
mod tests {
    use hex_ai::{
        AiAlgorithmId, AiAlliedUnit, AiDecisionKind, AiDecisionTrace, AiLatticeObservation,
        AiObservation, AiProfileId, AiSelection, LegalActionFingerprint, LegalActionSet,
    };
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

    #[test]
    fn detail_windows_are_bounded_while_counts_and_fingerprints_cover_all_events() {
        let mut first = CombatSummary::default();
        let mut second = CombatSummary::default();
        let total = MAX_COMBAT_SUMMARY_DETAILS.saturating_add(17);
        for index in 0..total {
            let event = CombatEvent::Downed {
                unit: UnitId(u64::try_from(index).unwrap_or(u64::MAX)),
            };
            first.record_event(&event);
            second.record_event(&event);
        }

        assert_eq!(first.events.len(), MAX_COMBAT_SUMMARY_DETAILS);
        assert_eq!(first.event_count, u64::try_from(total).unwrap_or(u64::MAX));
        assert_eq!(first.event_fingerprint, second.event_fingerprint);
        assert_eq!(
            first.events.front(),
            Some(&CombatEvent::Downed { unit: UnitId(17) })
        );
    }

    #[test]
    fn legacy_summary_shape_decodes_with_new_aggregate_defaults() {
        let legacy = r#"{
            "rounds": 2,
            "commands": {},
            "ai_selections": [],
            "moves": 0,
            "casts": 0,
            "strikes": 0,
            "decisions": 0,
            "idle_turns": 0,
            "raw_disables": 0,
            "prevented_disables": 0,
            "applied_disables": 0,
            "restored_cells": 0,
            "revivals": 0,
            "downings": 0,
            "outcome": null,
            "events": []
        }"#;
        let decoded: CombatSummary =
            serde_json::from_str(legacy).expect("legacy summaries remain readable");
        assert_eq!(decoded.rounds, 2);
        assert_eq!(decoded.ai_selection_count, 0);
        assert_eq!(decoded.event_count, 0);
    }

    #[test]
    fn a_full_legacy_trace_decodes_as_one_compact_record() {
        let legal_actions = LegalActionSet::from_canonical_commands(
            LegalActionFingerprint(42),
            vec![GameCommand::EndTurn { unit: UnitId(3) }],
        );
        let key = legal_actions
            .actions()
            .first()
            .expect("fixture has one action")
            .key;
        let trace = AiDecisionTrace {
            sequence: 7,
            profile: AiProfileId("baseline".to_owned()),
            algorithm: AiAlgorithmId("baseline-v1".to_owned()),
            actor: UnitId(3),
            group: None,
            kind: AiDecisionKind::TurnAction,
            observation: AiObservation {
                actor: AiAlliedUnit {
                    unit: UnitId(3),
                    position: TilePos::new(HexCoord::ORIGIN, 1),
                    downed: false,
                    lattice: AiLatticeObservation {
                        capacity: None,
                        cells: Vec::new(),
                    },
                    spells: Vec::new(),
                },
                allies: Vec::new(),
                hostiles: Vec::new(),
                turn_order: vec![UnitId(3)],
                round: 1,
                effects: Vec::new(),
                traversal: Vec::new(),
            },
            legal_actions: legal_actions.clone(),
            fingerprint: legal_actions.fingerprint(),
            cell_fingerprint: None,
            selected: AiSelection::Action(key),
            command: Some(GameCommand::EndTurn { unit: UnitId(3) }),
            failure: None,
        };
        let encoded = serde_json::to_value(&trace).expect("full trace serializes");
        let compact: AiDecisionRecord =
            serde_json::from_value(encoded).expect("full trace shape remains readable");
        assert_eq!(compact, AiDecisionRecord::from(&trace));
    }
}
