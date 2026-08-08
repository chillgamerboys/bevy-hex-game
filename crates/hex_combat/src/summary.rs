//! Deterministic, session-scoped combat reporting.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use bevy::prelude::*;
use hex_ai::{AiDecisionRecord, AiDecisionTrace};
use hex_core::{GameCommand, Mode, Screen, UnitId};
use serde::{Deserialize, Serialize};
use xxhash_rust::xxh3::xxh3_64;

use crate::{AiDecisionTraces, CombatEvent, CombatSystems, EncounterOutcome, TurnOrder};

/// Maximum compact AI records and structured events retained by [`CombatSummary`].
pub const MAX_COMBAT_SUMMARY_DETAILS: usize = 4_096;
/// Stable schema domain for deterministic summary fingerprints.
pub const COMBAT_SUMMARY_FINGERPRINT_VERSION: u16 = 1;

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

/// Stable delivered-effect categories used by aggregate reporting.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DeliveredEffectKind {
    /// Incoming disable demand opened a defender choice.
    Disable,
    /// A persistent Burn was attached.
    Burn,
    /// Divination revealed lattice facts.
    Reveal,
    /// Disabled cells were restored by a spell.
    Restore,
    /// A defensive enchantment absorbed incoming disables.
    Prevention,
}

/// Per-unit projection of the same canonical command and event stream as
/// [`CombatSummary`].
#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq, Eq)]
#[serde(default)]
pub struct UnitCombatSummary {
    /// Completed authoritative turns owned by this unit.
    pub turns: u32,
    /// Successful commands issued by this unit.
    pub successful_commands: u32,
    /// Refused commands issued by this unit.
    pub refused_commands: u32,
    /// Exact surface edges committed by this unit.
    pub movement_distance: u32,
    /// Movement budget spent by this unit.
    pub movement_budget_used: u32,
    /// Successful casts by stable spell name.
    pub casts_by_spell: BTreeMap<String, u32>,
    /// Delivered spell/effect categories attributed to this unit.
    pub delivered_effects: BTreeMap<DeliveredEffectKind, u32>,
    /// Successful Channel actions.
    pub channels: u32,
    /// Mana restored by stable element name.
    pub channelled_mana: BTreeMap<String, u32>,
    /// Successful strikes.
    pub strikes: u32,
    /// Raw disables caused before prevention.
    pub raw_disables: u32,
    /// Incoming disables prevented by this unit's defences.
    pub prevented_disables: u32,
    /// Exact cells disabled by this unit.
    pub applied_disables: u32,
    /// Exact cells restored by this unit.
    pub restored_cells: u32,
    /// Times this unit was downed.
    pub downings: u32,
    /// Times this unit was revived.
    pub revivals: u32,
    /// Explicit no-action yields.
    pub idle_turns: u32,
    /// Current consecutive completed turns without progress for this unit.
    pub no_progress_current: u32,
    /// Longest no-progress stretch for this unit.
    pub no_progress_max: u32,
    /// Canonical AI decision dispatches for this unit.
    pub ai_choices: u64,
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
    /// Completed authoritative combat turns.
    #[serde(default)]
    pub turns: u32,
    /// Per-unit projections keyed by stable session identity.
    #[serde(default)]
    pub units: BTreeMap<UnitId, UnitCombatSummary>,
    /// Successful commands by stable unit and semantic kind.
    pub commands: BTreeMap<UnitId, BTreeMap<CommandKind, u32>>,
    /// Refused commands by stable unit and semantic kind.
    #[serde(default)]
    pub refusals: BTreeMap<UnitId, BTreeMap<CommandKind, u32>>,
    /// Total successful commands.
    #[serde(default)]
    pub successful_commands: u32,
    /// Total refused commands.
    #[serde(default)]
    pub refused_commands: u32,
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
    /// Exact surface edges committed across individual and party paths.
    #[serde(default)]
    pub movement_distance: u32,
    /// Movement budget spent; currently one per committed surface edge.
    #[serde(default)]
    pub movement_budget_used: u32,
    /// Successful casts.
    pub casts: u32,
    /// Successful casts under stable spell names.
    #[serde(default)]
    pub casts_by_spell: BTreeMap<String, u32>,
    /// Successfully delivered mechanical outcomes by semantic category.
    #[serde(default)]
    pub delivered_effects: BTreeMap<DeliveredEffectKind, u32>,
    /// Successful Channel actions.
    #[serde(default)]
    pub channels: u32,
    /// Mana restored by stable element name.
    #[serde(default)]
    pub channelled_mana: BTreeMap<String, u32>,
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
    /// Current consecutive completed turns with no committed movement, action, or
    /// delivered effect.
    #[serde(default)]
    pub no_progress_current: u32,
    /// Longest consecutive no-progress stretch in this session.
    #[serde(default)]
    pub no_progress_max: u32,
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
    /// Units that made progress since their most recent turn began.
    ///
    /// This small deterministic cursor is serialized because it affects the next
    /// authoritative turn-boundary result. Omitting it would make a mid-session
    /// round trip change no-progress telemetry.
    #[serde(default)]
    progress_units: BTreeSet<UnitId>,
    /// First serialization failure encountered while building rolling evidence.
    #[serde(default)]
    pub evidence_error: Option<String>,
}

impl CombatSummary {
    /// Deterministic identity of every serialized aggregate and retained detail.
    ///
    /// Runtime cursors are serde-skipped, so collection timing cannot affect the
    /// artifact. Count and rolling-fingerprint fields continue to cover details that
    /// aged out of the bounded windows.
    pub fn fingerprint(&self) -> Result<u64, String> {
        if let Some(error) = &self.evidence_error {
            return Err(format!("combat summary evidence is incomplete: {error}"));
        }
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"hex-combat-summary-v1");
        bytes.extend_from_slice(&COMBAT_SUMMARY_FINGERPRINT_VERSION.to_le_bytes());
        serde_json::to_writer(&mut bytes, self)
            .map_err(|error| format!("combat summary fingerprint serialization failed: {error}"))?;
        Ok(xxh3_64(&bytes))
    }

    pub(crate) fn record_command(&mut self, command: &GameCommand) {
        let kind = CommandKind::of(command);
        let unit = command.unit();
        self.successful_commands = self.successful_commands.saturating_add(1);
        *self
            .commands
            .entry(unit)
            .or_default()
            .entry(kind)
            .or_default() += 1;
        let movement = command_movement_distance(command);
        self.movement_distance = self.movement_distance.saturating_add(movement);
        self.movement_budget_used = self.movement_budget_used.saturating_add(movement);
        match kind {
            CommandKind::Move | CommandKind::MoveParty => self.moves += 1,
            CommandKind::Strike => self.strikes += 1,
            CommandKind::EndTurn => self.idle_turns += 1,
            CommandKind::Cast => {
                self.casts += 1;
                if let GameCommand::Cast { spell, .. } = command {
                    *self.casts_by_spell.entry(spell.clone()).or_default() += 1;
                }
            }
            CommandKind::ChooseDisables | CommandKind::ChooseRestores => self.decisions += 1,
            CommandKind::Channel => self.channels += 1,
            CommandKind::Rest => {}
        }
        let successful = &mut self.units.entry(unit).or_default().successful_commands;
        *successful = successful.saturating_add(1);
        match command {
            GameCommand::MoveAlong { .. } => {
                let per_unit = self.units.entry(unit).or_default();
                per_unit.movement_distance = per_unit.movement_distance.saturating_add(movement);
                per_unit.movement_budget_used =
                    per_unit.movement_budget_used.saturating_add(movement);
            }
            GameCommand::MoveParty { paths, .. } => {
                for path in paths {
                    let distance =
                        u32::try_from(path.path.len().saturating_sub(1)).unwrap_or(u32::MAX);
                    let member = self.units.entry(path.member).or_default();
                    member.movement_distance = member.movement_distance.saturating_add(distance);
                    member.movement_budget_used =
                        member.movement_budget_used.saturating_add(distance);
                    if distance > 0 {
                        self.progress_units.insert(path.member);
                    }
                }
            }
            GameCommand::Strike { .. } => {
                let strikes = &mut self.units.entry(unit).or_default().strikes;
                *strikes = strikes.saturating_add(1);
            }
            GameCommand::EndTurn { .. } => {
                let idle = &mut self.units.entry(unit).or_default().idle_turns;
                *idle = idle.saturating_add(1);
            }
            GameCommand::Cast { spell, .. } => {
                *self
                    .units
                    .entry(unit)
                    .or_default()
                    .casts_by_spell
                    .entry(spell.clone())
                    .or_default() += 1;
            }
            GameCommand::Channel { .. } => {
                let channels = &mut self.units.entry(unit).or_default().channels;
                *channels = channels.saturating_add(1);
            }
            GameCommand::ChooseDisables { .. }
            | GameCommand::ChooseRestores { .. }
            | GameCommand::Rest { .. } => {}
        }
        if !matches!(kind, CommandKind::EndTurn) {
            self.progress_units.insert(unit);
        }
    }

    fn record_event(&mut self, event: &CombatEvent) {
        match event {
            CombatEvent::TurnAdvanced { unit, .. } => {
                self.turns = self.turns.saturating_add(1);
                let progressed = self.progress_units.remove(unit);
                if progressed {
                    self.no_progress_current = 0;
                } else {
                    self.no_progress_current = self.no_progress_current.saturating_add(1);
                    self.no_progress_max = self.no_progress_max.max(self.no_progress_current);
                }
                let per_unit = self.units.entry(*unit).or_default();
                per_unit.turns = per_unit.turns.saturating_add(1);
                if progressed {
                    per_unit.no_progress_current = 0;
                } else {
                    per_unit.no_progress_current = per_unit.no_progress_current.saturating_add(1);
                    per_unit.no_progress_max =
                        per_unit.no_progress_max.max(per_unit.no_progress_current);
                }
            }
            CombatEvent::DecisionOpened { source, count, .. } => {
                self.raw_disables += u32::from(*count);
                *self
                    .delivered_effects
                    .entry(DeliveredEffectKind::Disable)
                    .or_default() += 1;
                let source = self.units.entry(*source).or_default();
                source.raw_disables = source.raw_disables.saturating_add(u32::from(*count));
                *source
                    .delivered_effects
                    .entry(DeliveredEffectKind::Disable)
                    .or_default() += 1;
            }
            CombatEvent::DamagePrevented {
                source,
                target,
                amount,
            } => {
                self.raw_disables += u32::from(*amount);
                self.prevented_disables += u32::from(*amount);
                let source = self.units.entry(*source).or_default();
                source.raw_disables = source.raw_disables.saturating_add(u32::from(*amount));
                let target = self.units.entry(*target).or_default();
                target.prevented_disables =
                    target.prevented_disables.saturating_add(u32::from(*amount));
                *self
                    .delivered_effects
                    .entry(DeliveredEffectKind::Prevention)
                    .or_default() += 1;
            }
            CombatEvent::HexesDisabled { source, cells, .. } => {
                let count = u32::try_from(cells.len()).unwrap_or(u32::MAX);
                self.applied_disables = self.applied_disables.saturating_add(count);
                let source = self.units.entry(*source).or_default();
                source.applied_disables = source.applied_disables.saturating_add(count);
            }
            CombatEvent::HexesRestored { caster, cells, .. } => {
                let count = u32::try_from(cells.len()).unwrap_or(u32::MAX);
                self.restored_cells = self.restored_cells.saturating_add(count);
                let caster = self.units.entry(*caster).or_default();
                caster.restored_cells = caster.restored_cells.saturating_add(count);
                *self
                    .delivered_effects
                    .entry(DeliveredEffectKind::Restore)
                    .or_default() += 1;
            }
            CombatEvent::Rested { unit, cells, .. } => {
                let count = u32::try_from(cells.len()).unwrap_or(u32::MAX);
                self.restored_cells = self.restored_cells.saturating_add(count);
                let unit = self.units.entry(*unit).or_default();
                unit.restored_cells = unit.restored_cells.saturating_add(count);
            }
            CombatEvent::BurnApplied { source, .. } => {
                let source = self.units.entry(*source).or_default();
                *source
                    .delivered_effects
                    .entry(DeliveredEffectKind::Burn)
                    .or_default() += 1;
                *self
                    .delivered_effects
                    .entry(DeliveredEffectKind::Burn)
                    .or_default() += 1;
            }
            CombatEvent::Revealed { .. } => {
                *self
                    .delivered_effects
                    .entry(DeliveredEffectKind::Reveal)
                    .or_default() += 1;
            }
            CombatEvent::Channelled { unit, restored } => {
                let per_unit = self.units.entry(*unit).or_default();
                for (element, amount) in restored {
                    let total = self.channelled_mana.entry(element.clone()).or_default();
                    *total = total.saturating_add(u32::from(*amount));
                    let total = per_unit.channelled_mana.entry(element.clone()).or_default();
                    *total = total.saturating_add(u32::from(*amount));
                }
            }
            CombatEvent::CommandRefused { command, .. } => {
                let unit = command.unit();
                self.refused_commands = self.refused_commands.saturating_add(1);
                *self
                    .refusals
                    .entry(unit)
                    .or_default()
                    .entry(CommandKind::of(command))
                    .or_default() += 1;
                let unit = self.units.entry(unit).or_default();
                unit.refused_commands = unit.refused_commands.saturating_add(1);
            }
            CombatEvent::Revived { unit, .. } => {
                self.revivals = self.revivals.saturating_add(1);
                let unit = self.units.entry(*unit).or_default();
                unit.revivals = unit.revivals.saturating_add(1);
            }
            CombatEvent::Downed { unit } => {
                self.downings = self.downings.saturating_add(1);
                let unit = self.units.entry(*unit).or_default();
                unit.downings = unit.downings.saturating_add(1);
            }
            CombatEvent::EncounterResolved { outcome } => self.outcome = Some(*outcome),
            CombatEvent::Cast { caster, .. }
            | CombatEvent::Strike {
                attacker: caster, ..
            }
            | CombatEvent::PartyMoved { anchor: caster, .. } => {
                self.progress_units.insert(*caster);
            }
            CombatEvent::BurnTicked { source, .. } => {
                self.progress_units.insert(*source);
            }
            CombatEvent::EnchantmentBroken { unit, .. } => {
                self.progress_units.insert(*unit);
            }
        }
        match rolling_fingerprint(
            b"combat-event-v1",
            self.event_fingerprint,
            self.event_count,
            event,
        ) {
            Ok(fingerprint) => self.event_fingerprint = fingerprint,
            Err(error) => {
                self.evidence_error.get_or_insert(error);
            }
        };
        self.event_count = self.event_count.saturating_add(1);
        push_bounded(&mut self.events, event.clone());
    }

    fn record_ai_selection(&mut self, trace: &AiDecisionTrace) {
        let record = AiDecisionRecord::from(trace);
        let unit = self.units.entry(record.actor).or_default();
        unit.ai_choices = unit.ai_choices.saturating_add(1);
        match rolling_fingerprint(
            b"ai-selection-v1",
            self.ai_selection_fingerprint,
            self.ai_selection_count,
            &record,
        ) {
            Ok(fingerprint) => self.ai_selection_fingerprint = fingerprint,
            Err(error) => {
                self.evidence_error.get_or_insert(error);
            }
        };
        self.ai_selection_count = self.ai_selection_count.saturating_add(1);
        self.ai_trace_cursor = trace.sequence.saturating_add(1);
        push_bounded(&mut self.ai_selections, record);
    }
}

fn command_movement_distance(command: &GameCommand) -> u32 {
    match command {
        GameCommand::MoveAlong { path, .. } => {
            u32::try_from(path.len().saturating_sub(1)).unwrap_or(u32::MAX)
        }
        GameCommand::MoveParty { paths, .. } => paths.iter().fold(0_u32, |total, member| {
            total.saturating_add(
                u32::try_from(member.path.len().saturating_sub(1)).unwrap_or(u32::MAX),
            )
        }),
        _ => 0,
    }
}

fn push_bounded<T>(values: &mut VecDeque<T>, value: T) {
    while values.len() >= MAX_COMBAT_SUMMARY_DETAILS {
        let _ = values.pop_front();
    }
    values.push_back(value);
}

fn rolling_fingerprint(
    domain: &[u8],
    previous: u64,
    ordinal: u64,
    value: &impl Serialize,
) -> Result<u64, String> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(domain);
    bytes.extend_from_slice(&previous.to_le_bytes());
    bytes.extend_from_slice(&ordinal.to_le_bytes());
    serde_json::to_writer(&mut bytes, value).map_err(|error| {
        format!(
            "{} rolling fingerprint serialization failed at {ordinal}: {error}",
            String::from_utf8_lossy(domain)
        )
    })?;
    Ok(xxh3_64(&bytes))
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
    fn per_unit_metrics_and_no_progress_use_canonical_turn_boundaries() {
        let unit = UnitId(1);
        let next = UnitId(2);
        let start = TilePos::new(HexCoord::ORIGIN, 1);
        let destination = TilePos::new(HexCoord::from_axial(1, 0), 1);
        let mut summary = CombatSummary::default();

        summary.record_command(&GameCommand::EndTurn { unit });
        summary.record_event(&CombatEvent::TurnAdvanced {
            unit,
            next: Some(next),
            round: 0,
        });
        assert_eq!(
            (
                summary.turns,
                summary.no_progress_current,
                summary.no_progress_max
            ),
            (1, 1, 1)
        );

        summary.record_command(&GameCommand::MoveAlong {
            unit: next,
            path: vec![start, destination],
        });
        summary.record_event(&CombatEvent::Channelled {
            unit: next,
            restored: BTreeMap::from([("Fire".to_owned(), 2)]),
        });
        summary.record_event(&CombatEvent::TurnAdvanced {
            unit: next,
            next: Some(unit),
            round: 1,
        });
        summary.record_event(&CombatEvent::CommandRefused {
            command: GameCommand::Channel { unit: next },
            refusal: crate::CommandRefusal::ActionAlreadySpent,
        });

        let projected = summary.units.get(&next).expect("unit projection");
        assert_eq!(projected.movement_distance, 1);
        assert_eq!(projected.movement_budget_used, 1);
        assert_eq!(projected.channelled_mana.get("Fire"), Some(&2));
        assert_eq!(projected.refused_commands, 1);
        assert_eq!(projected.turns, 1);
        assert_eq!(projected.no_progress_max, 0);
        assert_eq!(summary.turns, 2);
        assert_eq!(summary.no_progress_current, 0);
        assert_eq!(summary.no_progress_max, 1);
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
        assert_eq!(decoded.fingerprint(), summary.fingerprint());
    }

    #[test]
    fn summary_fingerprint_changes_with_canonical_combat_truth() {
        let mut first = CombatSummary::default();
        let mut second = CombatSummary::default();
        assert_eq!(first.fingerprint(), second.fingerprint());

        second.record_command(&GameCommand::EndTurn { unit: UnitId(1) });
        assert_ne!(first.fingerprint(), second.fingerprint());

        first.record_command(&GameCommand::EndTurn { unit: UnitId(1) });
        assert_eq!(first.fingerprint(), second.fingerprint());
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
